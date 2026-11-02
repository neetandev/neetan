//! HLE system ROM services.
//!
//! The stub system ROM (`utils/bios_towns/sysrom.asm`) only traps. Each trap
//! saves AX and DX on the stack, writes a vector number to port 07F0h and
//! continues. The machine loop then calls [`TownsBus::execute_bios_hle`], which
//! restores AX and DX from the stack and runs the handler of the vector with
//! direct register access. All entries run in real mode.

mod bootstrap;
mod cmos;
mod disk;
mod post;

use common::{
    Cpu, SegmentRegister, TraceCall, TraceCallInterface, TraceCallPhase, TraceContext, TraceEvent,
    TraceEventKey, TraceField, TraceSink, TraceValue, trace_id, warn,
};
use stack_vec::StackVec;

use super::{TownsBus, hle_dos};

/// Real-mode code segment of the stub system ROM (physical FC000h).
pub(crate) const CODE_SEGMENT: u16 = 0xFC00;
/// Offset of the code segment inside the 256 KiB system ROM image.
const CODE_SEGMENT_IMAGE_OFFSET: usize = 0x3_C000;
/// Stub ROM metadata word: boot device search loop offset.
const METADATA_BOOT_SEARCH: usize = 2;
/// Stub ROM metadata word: boot retry wait offset.
const METADATA_BOOT_RETRY: usize = 4;
/// Stub ROM metadata word: default interrupt handler offset.
const METADATA_UNEXPECTED_INTERRUPT: usize = 6;
/// Stub ROM metadata word: NMI handler offset.
const METADATA_NMI_HANDLER: usize = 8;
/// Stub ROM metadata word: LGDT pseudo descriptor offset.
const METADATA_GDT_REGISTER: usize = 10;

/// Pseudo vector of the power on self test.
pub(crate) const VECTOR_POST: u8 = 0xF0;
/// Pseudo vector of the disk BIOS `FFFB:0014`.
pub(crate) const VECTOR_DISK_BIOS: u8 = 0xF1;
/// Pseudo vector that loads the IPL of the next boot device.
pub(crate) const VECTOR_BOOTSTRAP: u8 = 0xF2;
/// Pseudo vector of the boot device search entry `FFFB:000A`.
pub(crate) const VECTOR_BOOT_SEARCH: u8 = 0xF3;
/// Pseudo vector of the CMOS BIOS `FFFB:0019`.
pub(crate) const VECTOR_CMOS_BIOS: u8 = 0xF4;
/// Pseudo vector of the POST tail entry `FFFB:0000`.
pub(crate) const VECTOR_RESTART_RESET: u8 = 0xF5;
/// Pseudo vector of the POST tail entry `FFFB:0005`.
pub(crate) const VECTOR_RESTART: u8 = 0xF6;
/// Pseudo vector of the display initialization entry `FFFB:000F`.
pub(crate) const VECTOR_DISPLAY_INIT: u8 = 0xF7;
/// Pseudo vector of the string print entry `FFFB:001E`.
pub(crate) const VECTOR_PRINT: u8 = 0xF8;
/// Pseudo vector of the default interrupt handler.
pub(crate) const VECTOR_UNEXPECTED_INTERRUPT: u8 = 0xF9;

/// Carry flag bit of FLAGS.
const FLAG_CARRY: u16 = 0x0001;

/// CR0 numeric error bit, set by the firmware on the i486.
const CR0_NUMERIC_ERROR: u32 = 0x0000_0020;

/// First CMOS I/O port. CMOS byte `n` is I/O port `3000h + 2n`.
const CMOS_IO_BASE: u16 = 0x3000;

/// Returns the CMOS byte index of the CMOS I/O port `port`.
pub(crate) const fn cmos_index(port: u16) -> usize {
    ((port - CMOS_IO_BASE) / 2) as usize
}

/// CPU access the HLE system ROM needs beyond the [`Cpu`] trait: the full
/// 32-bit index registers and the firmware CR0 state.
pub(crate) trait TownsBiosCpu: Cpu {
    /// Sets the full 32-bit ESI register.
    fn set_esi_full(&mut self, value: u32);

    /// Sets the full 32-bit EDI register.
    fn set_edi_full(&mut self, value: u32);

    /// Sets the full 32-bit EBP register.
    fn set_ebp_full(&mut self, value: u32);

    /// Sets the CR0 bits the real system ROM leaves at the IPL handoff.
    fn apply_firmware_cr0(&mut self);

    /// Loads GDTR with `base` and `limit`.
    fn load_gdtr(&mut self, base: u32, limit: u16);
}

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> TownsBiosCpu
    for cpu::I386<CPU_MODEL, ADDRESS_WIDTH>
{
    fn set_esi_full(&mut self, value: u32) {
        self.state.set_esi(value);
    }

    fn set_edi_full(&mut self, value: u32) {
        self.state.set_edi(value);
    }

    fn set_ebp_full(&mut self, value: u32) {
        self.state.set_ebp(value);
    }

    fn apply_firmware_cr0(&mut self) {
        if CPU_MODEL == cpu::CPU_MODEL_486_DX {
            self.state.cr0 |= CR0_NUMERIC_ERROR;
        }
    }

    fn load_gdtr(&mut self, base: u32, limit: u16) {
        self.state.gdt_base = base;
        self.state.gdt_limit = limit;
    }
}

save_state::runtime_state! {
/// Runtime state of the HLE system ROM.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct TownsBiosState {
    boot_queue: Vec<u8>,
    boot_position: u8,
    boot_failure_reported: bool,
    unexpected_interrupt_reported: bool,
    undefined_cmos_function_reported: bool,
    floppy_modes: [u8; 4],
    floppy_geometries: [u16; 4],
}}

impl TownsBiosState {
    /// Builds the power-on state: an empty boot queue and the 2HD default mode
    /// on every floppy drive.
    pub(crate) fn new() -> Self {
        Self {
            floppy_modes: [disk::FLOPPY_DEFAULT_MODE; 4],
            floppy_geometries: [disk::FLOPPY_DEFAULT_GEOMETRY; 4],
            ..Self::default()
        }
    }
}

impl<T: TraceSink> TownsBus<T> {
    /// Whether the HLE system ROM runs. The trap port 07F0h decodes only then.
    pub(crate) fn hle_bios(&self) -> bool {
        self.memory.hle_system_rom()
    }

    /// Returns whether a BIOS HLE trap is pending.
    pub(crate) fn bios_hle_pending(&self) -> bool {
        self.bios.hle_pending()
    }

    /// Executes the pending BIOS HLE trap with direct CPU register access.
    pub(crate) fn execute_bios_hle(&mut self, cpu: &mut impl TownsBiosCpu) {
        let vector = self.bios.pending_vector();
        self.bios.clear_hle_pending();

        device::bios::restore_trap_registers(cpu, |address| self.hle_read_word(address));

        let function = cpu.ah();
        let subfunction = cpu.al();
        self.trace_bios_call(
            vector,
            Some(u64::from(function)),
            Some(u64::from(subfunction)),
            TraceCallPhase::Enter,
            None,
        );

        match vector {
            VECTOR_POST => self.hle_post(cpu),
            VECTOR_DISK_BIOS => self.hle_disk_bios(cpu),
            VECTOR_BOOTSTRAP => self.hle_bootstrap(cpu),
            VECTOR_BOOT_SEARCH => self.start_boot_search(),
            VECTOR_CMOS_BIOS => self.hle_cmos_bios(cpu),
            VECTOR_RESTART_RESET => self.hle_restart(cpu, true),
            VECTOR_RESTART => self.hle_restart(cpu, false),
            VECTOR_DISPLAY_INIT => self.reinitialize_display(),
            VECTOR_PRINT => self.hle_print(cpu),
            VECTOR_UNEXPECTED_INTERRUPT => self.hle_unexpected_interrupt(),
            byte if self.dos.is_some() && hle_dos::is_dos_entry(byte) => {
                self.execute_dos_entry(cpu, byte)
            }
            _ => warn!("machine_towns: unknown BIOS HLE vector {vector:02X}h"),
        }

        self.trace_bios_call(
            vector,
            Some(u64::from(function)),
            Some(u64::from(subfunction)),
            TraceCallPhase::Exit,
            Some(u64::from(cpu.ax())),
        );
    }

    /// Emits a call trace event for a BIOS HLE dispatch boundary.
    fn trace_bios_call(
        &mut self,
        vector: u8,
        function: Option<u64>,
        subfunction: Option<u64>,
        phase: TraceCallPhase,
        result: Option<u64>,
    ) {
        self.trace_call(
            trace_id::provider::TOWNS_BIOS,
            TraceCallInterface::Interrupt(u32::from(vector)),
            function,
            subfunction,
            phase,
            result,
        );
    }

    /// Emits a call trace event of `provider` for the entry `interface`.
    pub(super) fn trace_call(
        &mut self,
        provider: &'static str,
        interface: TraceCallInterface,
        function: Option<u64>,
        subfunction: Option<u64>,
        phase: TraceCallPhase,
        result: Option<u64>,
    ) {
        if !T::ENABLED
            || !self
                .tracer
                .interested(TraceEventKey::Call { provider, phase })
        {
            return;
        }
        let mut fields = StackVec::<TraceField<'_>, 3>::new();
        if let Some(function) = function {
            fields.push(TraceField {
                name: trace_id::field::FUNCTION,
                value: TraceValue::Unsigned(function),
            });
        }
        if let Some(subfunction) = subfunction {
            fields.push(TraceField {
                name: trace_id::field::SUBFUNCTION,
                value: TraceValue::Unsigned(subfunction),
            });
        }
        if let Some(result) = result {
            fields.push(TraceField {
                name: trace_id::field::RESULT,
                value: TraceValue::Unsigned(result),
            });
        }
        self.tracer.trace(
            TraceContext::main_cpu(
                self.current_cycle,
                Some(u64::from(self.clocks.cpu_clock_hz)),
            ),
            TraceEvent::Call(TraceCall {
                provider,
                interface,
                phase,
                fields: &fields,
            }),
        );
    }

    /// Captures the HLE system ROM state.
    pub(crate) fn capture_bios_state(&self) -> TownsBiosState {
        self.bios_state.clone()
    }

    /// Restores the HLE system ROM state.
    pub(crate) fn restore_bios_state(
        &mut self,
        state: TownsBiosState,
    ) -> Result<(), save_state::StateValidationError> {
        if usize::from(state.boot_position) > state.boot_queue.len() {
            return Err(save_state::StateValidationError::new(
                "FM Towns BIOS boot queue position is invalid",
            ));
        }
        self.bios_state = state;
        Ok(())
    }

    /// Reads a byte of physical memory for an HLE handler.
    fn hle_read_byte(&mut self, address: u32) -> u8 {
        self.read_byte_data(address)
    }

    /// Writes a byte of physical memory for an HLE handler.
    fn hle_write_byte(&mut self, address: u32, value: u8) {
        self.write_byte_data(address, value);
    }

    /// Reads a little-endian word of physical memory for an HLE handler.
    fn hle_read_word(&mut self, address: u32) -> u16 {
        u16::from(self.hle_read_byte(address))
            | (u16::from(self.hle_read_byte(address.wrapping_add(1))) << 8)
    }

    /// Writes a little-endian word of physical memory for an HLE handler.
    fn hle_write_word(&mut self, address: u32, value: u16) {
        self.hle_write_byte(address, value as u8);
        self.hle_write_byte(address.wrapping_add(1), (value >> 8) as u8);
    }

    /// Reads a CMOS byte by its I/O port.
    fn cmos_port(&self, port: u16) -> u8 {
        self.memory.cmos_byte(cmos_index(port))
    }

    /// Writes a CMOS byte by its I/O port.
    fn set_cmos_port(&mut self, port: u16, value: u8) {
        self.memory.set_cmos_byte(cmos_index(port), value);
    }

    /// Writes a word into two CMOS bytes, the low byte at `port` and the high
    /// byte at `port + 2`.
    fn set_cmos_port_word(&mut self, port: u16, value: u16) {
        self.set_cmos_port(port, value as u8);
        self.set_cmos_port(port + 2, (value >> 8) as u8);
    }

    /// Returns a word of the stub ROM metadata header.
    fn stub_metadata(&self, offset: usize) -> u16 {
        self.memory
            .system_rom_word(CODE_SEGMENT_IMAGE_OFFSET + offset)
    }

    /// Returns the base and the limit of the stub ROM GDT, read from its LGDT
    /// pseudo descriptor.
    fn stub_gdt_register(&self) -> (u32, u16) {
        let descriptor =
            CODE_SEGMENT_IMAGE_OFFSET + usize::from(self.stub_metadata(METADATA_GDT_REGISTER));
        let limit = self.memory.system_rom_word(descriptor);
        let base = u32::from(self.memory.system_rom_word(descriptor + 2))
            | (u32::from(self.memory.system_rom_word(descriptor + 4)) << 16);
        (base, limit)
    }

    /// Writes the IRET frame the stub reserved at SS:SP.
    fn write_iret_frame(&mut self, cpu: &impl Cpu, offset: u16, segment: u16, flags: u16) {
        let base = cpu
            .segment_base(SegmentRegister::SS)
            .wrapping_add(u32::from(cpu.sp()));
        self.hle_write_word(base, offset);
        self.hle_write_word(base.wrapping_add(2), segment);
        self.hle_write_word(base.wrapping_add(4), flags);
    }

    /// Sets AH to `status` and the carry flag to `status != 0`.
    fn set_bios_status(cpu: &mut impl Cpu, status: u8) {
        cpu.set_ah(status);
        let flags = cpu.flags() & !FLAG_CARRY;
        cpu.set_flags(flags | u16::from(status != 0));
    }

    /// Runs the POST tail and the boot device search for the entries
    /// `FFFB:0000` and `FFFB:0005`. `AH` holds the reset reason.
    fn hle_restart(&mut self, cpu: &mut impl TownsBiosCpu, clear_work_bytes: bool) {
        self.set_cmos_port(post::CMOS_RESET_REASON, cpu.ah());
        if clear_work_bytes {
            self.set_cmos_port(post::CMOS_WORK_31B2, 0);
            self.set_cmos_port(post::CMOS_WORK_31CC, 0);
        }
        self.post_tail(cpu);
    }

    /// Logs the string of the print entry `FFFB:001E`. `DS:SI` points at the
    /// row, the column and the length, followed by the text.
    fn hle_print(&mut self, cpu: &mut impl Cpu) {
        let base = cpu
            .segment_base(SegmentRegister::DS)
            .wrapping_add(u32::from(cpu.si()));
        let length = self.hle_read_byte(base.wrapping_add(2));
        let text: Vec<u8> = (0..u32::from(length))
            .map(|index| self.hle_read_byte(base.wrapping_add(3 + index)))
            .collect();
        common::debug!(
            "machine_towns: BIOS print row {} column {}: {}",
            self.hle_read_byte(base),
            self.hle_read_byte(base.wrapping_add(1)),
            String::from_utf8_lossy(&text)
        );
    }

    /// Logs the first call of the default interrupt handler.
    fn hle_unexpected_interrupt(&mut self) {
        if !self.bios_state.unexpected_interrupt_reported {
            self.bios_state.unexpected_interrupt_reported = true;
            warn!("machine_towns: unexpected interrupt through the default BIOS handler");
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use common::{Cpu, CpuMode, SegmentRegister};

    use crate::{
        bus::TownsBus,
        config::TownsModel,
        rom::{LoadedRoms, TownsFirmware},
    };

    /// The CPU used by the HLE tests.
    pub(crate) type TestCpu = cpu::I386<{ cpu::CPU_MODEL_486_DX }, { cpu::ADDRESS_WIDTH_32 }>;

    /// Builds an MX bus that runs the HLE system ROM with zero-filled ROMs.
    pub(crate) fn hle_bus() -> TownsBus {
        let roms = LoadedRoms {
            firmware: TownsFirmware::Hle,
            font: vec![0; 0x4_0000],
            f20: vec![0; 0x8_0000],
            dictionary: vec![0; 0x8_0000],
            serial: vec![0; 0x20],
        };
        TownsBus::new(TownsModel::FmTownsIIMx, CpuMode::High, roms, 48_000)
    }

    /// Builds a real-mode CPU with its stack at 0000:8000.
    pub(crate) fn real_mode_cpu() -> TestCpu {
        let mut cpu = TestCpu::new();
        cpu.reset();
        for register in [
            SegmentRegister::CS,
            SegmentRegister::DS,
            SegmentRegister::ES,
            SegmentRegister::SS,
        ] {
            cpu.load_segment_real_mode(register, 0);
        }
        cpu.set_sp(0x8000);
        cpu
    }

    /// Runs one trap of `vector` the way the stub issues it: AX and DX on the
    /// stack, then the dispatch.
    pub(crate) fn trap(bus: &mut TownsBus, cpu: &mut TestCpu, vector: u8) {
        let stack_pointer = cpu.sp().wrapping_sub(4);
        let base = cpu.segment_base(SegmentRegister::SS);
        bus.hle_write_word(base + u32::from(stack_pointer), cpu.dx());
        bus.hle_write_word(base + u32::from(stack_pointer) + 2, cpu.ax());
        cpu.set_sp(stack_pointer);
        cpu.set_ax(0xDEAD);
        cpu.set_dx(0xBEEF);
        bus.bios.write_trap_port(vector);
        bus.execute_bios_hle(cpu);
    }

    #[test]
    fn trap_restores_ax_dx_and_pops_them() {
        let mut bus = hle_bus();
        let mut cpu = real_mode_cpu();
        cpu.set_ax(0x1234);
        cpu.set_dx(0x5678);
        trap(&mut bus, &mut cpu, super::VECTOR_PRINT);
        assert_eq!(cpu.ax(), 0x1234);
        assert_eq!(cpu.dx(), 0x5678);
        assert_eq!(cpu.sp(), 0x8000);
        assert!(!bus.bios_hle_pending());
    }

    #[test]
    fn system_icons_are_mapped_at_fffe8000() {
        const ICON_AREA: u32 = 0xFFFE_8000;
        const ICON_BYTES: u32 = 256;
        const PLANE_BYTES: u32 = ICON_BYTES / 2;
        const ROW_BYTES: u32 = 4;
        const ICON_COUNT: u32 = 128;
        const CURSOR_ARROW: u32 = 80;
        const LAST_ROW: u32 = 31;
        const COLUMN_1_BIT: u8 = 0x40;
        let mut bus = hle_bus();
        for icon in 0..ICON_COUNT {
            let mask = ICON_AREA + icon * ICON_BYTES + PLANE_BYTES;
            let opaque = (0..PLANE_BYTES).any(|offset| bus.hle_read_byte(mask + offset) != 0xFF);
            assert!(opaque, "icon {icon} has no opaque pixel");
        }
        let arrow = ICON_AREA + CURSOR_ARROW * ICON_BYTES;
        assert_ne!(
            bus.hle_read_byte(arrow + ROW_BYTES) & COLUMN_1_BIT,
            0,
            "arrow tip is black"
        );
        assert_ne!(
            bus.hle_read_byte(arrow + PLANE_BYTES + LAST_ROW * ROW_BYTES),
            0,
            "arrow corner is transparent"
        );
    }

    #[test]
    fn stub_metadata_points_at_the_real_rom_handler_offsets() {
        let bus = hle_bus();
        assert_eq!(
            bus.stub_metadata(super::METADATA_UNEXPECTED_INTERRUPT),
            0x0536
        );
        assert_eq!(bus.stub_metadata(super::METADATA_NMI_HANDLER), 0x04C8);
        assert_eq!(bus.stub_gdt_register(), (0xFFFF_C080, 0x0040));
    }
}
