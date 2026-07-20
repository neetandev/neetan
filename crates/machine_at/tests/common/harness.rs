//! Shared scaffolding for the machine_at integration tests.

#![allow(dead_code)]

use common::{Bus, Cpu, CpuMode, HostDateTime, Machine, NoTrace, TraceSink};
use device::vga::{
    VGA_PORT_ATC_WRITE, VGA_PORT_CRTC_DATA_COLOR, VGA_PORT_CRTC_DATA_MONO,
    VGA_PORT_CRTC_INDEX_COLOR, VGA_PORT_CRTC_INDEX_MONO, VGA_PORT_DAC_DATA, VGA_PORT_DAC_MASK,
    VGA_PORT_DAC_WRITE_INDEX, VGA_PORT_GC_DATA, VGA_PORT_GC_INDEX, VGA_PORT_HERCULES_COMPAT,
    VGA_PORT_MODE_CONTROL_COLOR, VGA_PORT_MODE_CONTROL_MONO, VGA_PORT_SEGMENT_SELECT,
    VGA_PORT_SEQ_DATA, VGA_PORT_SEQ_INDEX, VGA_PORT_STATUS_COLOR, VGA_PORT_STATUS_MONO,
    VGA_PORT_STATUS0_MISC_WRITE,
};
use machine_at::{AtBus, AtMachine, AtModel, LoadedRoms};

/// System BIOS image size in bytes.
const SYSTEM_BIOS_SIZE: usize = 0x1_0000;
/// VGA BIOS image size in bytes.
const VGA_BIOS_SIZE: usize = 0x8000;
/// Offset of the reset-vector entry point within the system BIOS image; the
/// 486 resets to CS base 0xFFFF0000, IP 0xFFF0, so the first fetch lands here.
const RESET_ENTRY_OFFSET: usize = 0xFFF0;
/// `hlt`: halts the CPU so `run_for` fast-forwards through scheduled events
/// (interrupts are disabled at reset), keeping the VGA frame renderer running
/// without a busy loop.
const PARK_PROGRAM: [u8; 1] = [0xF4];

/// A fixed host clock so runs are reproducible.
pub fn fixed_clock() -> HostDateTime {
    HostDateTime {
        year: 2026,
        month: 7,
        day: 12,
        day_of_week: 0,
        hour: 12,
        minute: 0,
        second: 0,
    }
}

/// Builds a 64 KiB system BIOS whose reset-vector entry point runs `program`.
pub fn reset_vector_bios(program: &[u8]) -> Vec<u8> {
    let mut bios = vec![0u8; SYSTEM_BIOS_SIZE];
    bios[RESET_ENTRY_OFFSET..RESET_ENTRY_OFFSET + program.len()].copy_from_slice(program);
    bios
}

/// Synthetic ROM set: a parked-CPU system BIOS and a zero-fill VGA BIOS.
pub fn synthetic_roms() -> LoadedRoms {
    roms_with_bios(reset_vector_bios(&PARK_PROGRAM))
}

/// Synthetic ROM set with a caller-supplied system BIOS image.
pub fn roms_with_bios(system_bios: Vec<u8>) -> LoadedRoms {
    LoadedRoms {
        system_bios,
        vga_bios: vec![0u8; VGA_BIOS_SIZE],
    }
}

/// Builds a machine of the given model over the parked-CPU synthetic ROM set.
pub fn machine_for_model(model: AtModel) -> AtMachine<NoTrace> {
    machine_with_roms(model, synthetic_roms())
}

/// Builds a machine of the given model over a caller-supplied ROM set.
pub fn machine_with_roms<T: TraceSink + Default>(model: AtModel, roms: LoadedRoms) -> AtMachine<T> {
    let mut bus = AtBus::new_with_trace_sink(
        model.cpu_clock_hz(CpuMode::High),
        model.ram_size(),
        roms,
        48_000,
        T::default(),
    );
    bus.set_host_date_time_source(std::sync::Arc::new(
        common::FixedHostDateTime(fixed_clock()),
    ));
    let mut cpu = cpu::I386::<{ cpu::CPU_MODEL_486_DX }, { cpu::ADDRESS_WIDTH_32 }>::new();
    cpu.reset();
    AtMachine::new(cpu, bus)
}

/// Runs the machine for the given number of emulated milliseconds.
pub fn run_millis<T: TraceSink>(machine: &mut AtMachine<T>, millis: u64) {
    let cycles = machine.bus.cpu_clock_hz() as u64 * millis / 1000;
    let slice = 1_000_000u64;
    let mut ran = 0u64;
    while ran < cycles {
        machine.run_for(slice);
        ran += slice;
    }
}

/// One VGA register file: misc output, sequencer, CRTC 0x00-0x18, graphics
/// controller, attribute controller, the full 256-entry DAC palette and the
/// segment select. The vectors and palette match the state the ET4000 BIOS
/// programs for each INT 10h mode (captured from the real BIOS).
pub struct ModeVector {
    pub misc: u8,
    pub seq: [u8; 8],
    pub crtc: [u8; 0x19],
    pub gc: [u8; 9],
    pub atc: [u8; 0x17],
    pub palette: &'static [u8; 768],
    pub segment_select: u8,
}

/// ET4000 extended CRTC registers the BIOS programs identically in every mode
/// (RAS/CAS 0x32, auxiliary 0x34, system configuration 0x36/0x37).
const EXTENDED_CRTC: [(u8, u8); 4] = [(0x32, 0x28), (0x34, 0x08), (0x36, 0x43), (0x37, 0x0F)];

impl ModeVector {
    /// Applies the register file to the bus VGA through the real I/O ports,
    /// KEY unlock included, leaving the adapter ready to scan out the mode.
    pub fn apply(&self, bus: &mut AtBus<NoTrace>) {
        bus.io_write_byte(VGA_PORT_STATUS0_MISC_WRITE, self.misc);
        let color = self.misc & 0x01 != 0;
        let (crtc_index_port, crtc_data_port, mode_control_port, status_port) = if color {
            (
                VGA_PORT_CRTC_INDEX_COLOR,
                VGA_PORT_CRTC_DATA_COLOR,
                VGA_PORT_MODE_CONTROL_COLOR,
                VGA_PORT_STATUS_COLOR,
            )
        } else {
            (
                VGA_PORT_CRTC_INDEX_MONO,
                VGA_PORT_CRTC_DATA_MONO,
                VGA_PORT_MODE_CONTROL_MONO,
                VGA_PORT_STATUS_MONO,
            )
        };

        bus.io_write_byte(VGA_PORT_HERCULES_COMPAT, 0x03);
        bus.io_write_byte(mode_control_port, 0xA0);

        for (index, value) in self.seq.iter().enumerate() {
            bus.io_write_byte(VGA_PORT_SEQ_INDEX, index as u8);
            bus.io_write_byte(VGA_PORT_SEQ_DATA, *value);
        }

        // Lift the CRTC write protection before loading 0x00-0x07, then restore
        // the captured vertical retrace end value last.
        bus.io_write_byte(crtc_index_port, 0x11);
        bus.io_write_byte(crtc_data_port, self.crtc[0x11] & 0x7F);
        for (index, value) in self.crtc.iter().enumerate() {
            if index == 0x11 {
                continue;
            }
            bus.io_write_byte(crtc_index_port, index as u8);
            bus.io_write_byte(crtc_data_port, *value);
        }
        for (index, value) in EXTENDED_CRTC {
            bus.io_write_byte(crtc_index_port, index);
            bus.io_write_byte(crtc_data_port, value);
        }
        bus.io_write_byte(crtc_index_port, 0x11);
        bus.io_write_byte(crtc_data_port, self.crtc[0x11]);

        for (index, value) in self.gc.iter().enumerate() {
            bus.io_write_byte(VGA_PORT_GC_INDEX, index as u8);
            bus.io_write_byte(VGA_PORT_GC_DATA, *value);
        }

        // Reset the attribute flip-flop to index phase, load the registers with
        // the palette address source clear, then re-enable the display.
        let _ = bus.io_read_byte(status_port);
        for (index, value) in self.atc.iter().enumerate() {
            bus.io_write_byte(VGA_PORT_ATC_WRITE, index as u8);
            bus.io_write_byte(VGA_PORT_ATC_WRITE, *value);
        }
        bus.io_write_byte(VGA_PORT_ATC_WRITE, 0x20);

        bus.io_write_byte(VGA_PORT_DAC_MASK, 0xFF);
        bus.io_write_byte(VGA_PORT_DAC_WRITE_INDEX, 0x00);
        for &component in self.palette.iter() {
            bus.io_write_byte(VGA_PORT_DAC_DATA, component);
        }

        bus.io_write_byte(VGA_PORT_SEGMENT_SELECT, self.segment_select);
    }
}

/// CS4031 configuration address port.
const CS4031_CONFIG_ADDRESS: u16 = 0x0022;
/// CS4031 configuration data port.
const CS4031_CONFIG_DATA: u16 = 0x0023;

/// Routes the A0000 and B0000 blocks to the VGA instead of internal DRAM, the
/// way the BIOS programs the chipset so text and graphics writes reach the
/// adapter. Clears the CS4031 A/B shadow register (index 0x18).
pub fn route_vga_window(bus: &mut AtBus<NoTrace>) {
    bus.io_write_byte(CS4031_CONFIG_ADDRESS, device::cs4031::CS4031_REG_SHADOW_AB);
    bus.io_write_byte(CS4031_CONFIG_DATA, 0x00);
}

/// Writes a run of bytes into VGA display memory through the CPU window, so the
/// current sequencer/graphics-controller write path applies exactly as it would
/// for a CPU store.
pub fn write_vram(bus: &mut AtBus<NoTrace>, physical: u32, bytes: &[u8]) {
    for (offset, byte) in bytes.iter().enumerate() {
        bus.write_byte(physical + offset as u32, *byte);
    }
}

/// Fills `count` bytes of VGA display memory with `value`.
pub fn fill_vram(bus: &mut AtBus<NoTrace>, physical: u32, value: u8, count: u32) {
    for offset in 0..count {
        bus.write_byte(physical + offset, value);
    }
}

/// Fills `count` bytes with an incrementing ramp starting at `start` (color =
/// running index, wrapping at 256), mirroring the exerciser position ramp.
pub fn fill_vram_ramp(bus: &mut AtBus<NoTrace>, physical: u32, start: u8, count: u32) {
    let mut value = start;
    for offset in 0..count {
        bus.write_byte(physical + offset, value);
        value = value.wrapping_add(1);
    }
}

/// Advances the machine long enough to render at least one settled frame.
pub fn render_frame<T: TraceSink>(machine: &mut AtMachine<T>) {
    run_millis(machine, 50);
}

/// Expands a 6-bit DAC component to 8 bits (matches the device DAC).
pub const fn expand(component: u8) -> u8 {
    (component << 2) | (component >> 4)
}

/// Packs a 6-bit RGB DAC entry into renderer RGBA.
pub const fn pen6(red: u8, green: u8, blue: u8) -> u32 {
    u32::from_le_bytes([expand(red), expand(green), expand(blue), 0xFF])
}

/// Reads one framebuffer pixel as packed RGBA.
pub fn pixel_rgba<T: TraceSink>(machine: &AtMachine<T>, x: u32, y: u32) -> u32 {
    let (width, _) = machine.display_dimensions();
    let offset = ((y * width + x) as usize) * 4;
    u32::from_le_bytes(
        machine.display_framebuffer()[offset..offset + 4]
            .try_into()
            .unwrap(),
    )
}

/// Samples a pixel in mode-logical coordinates (handles dot and scan doubling
/// by scaling to the rendered dimensions).
pub fn mode_pixel<T: TraceSink>(
    machine: &AtMachine<T>,
    logical: (u32, u32),
    x: u32,
    y: u32,
) -> u32 {
    let (width, height) = machine.display_dimensions();
    let scale_x = width / logical.0;
    let scale_y = height / logical.1;
    pixel_rgba(machine, x * scale_x, y * scale_y)
}

/// Lowercase hexadecimal BLAKE3 digest of the active framebuffer region.
pub fn framebuffer_hash<T: TraceSink>(machine: &AtMachine<T>) -> String {
    let (width, height) = machine.display_dimensions();
    let bytes = &machine.display_framebuffer()[..(width * height * 4) as usize];
    let mut hasher = blake3::Hasher::new();
    hasher.update(bytes);
    let mut digest = [0u8; 32];
    hasher.finalize(&mut digest);

    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        hex.push(HEX_DIGITS[(byte & 0x0F) as usize] as char);
    }
    hex
}
