//! Shared helpers for the FM Towns machine integration tests.

#![allow(dead_code)]

use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

use common::{Bus, Cpu, CpuMode, HostDateTime, Tracing};
use cpu::{CPU_MODEL_386_DX, CPU_MODEL_386_SX, CPU_MODEL_486_DX, I386State, SegReg32};
use machinetowns::{LoadedRoms, TownsBus, TownsMachine, TownsModel};

/// SYSROM / FONT slot size (256 KiB).
const ROM_SYSTEM_LEN: usize = 0x4_0000;
/// DOS / dictionary / F20 slot size (512 KiB).
const ROM_LARGE_LEN: usize = 0x8_0000;
/// Serial machine-ID EEPROM size.
const ROM_SERIAL_LEN: usize = 0x20;

/// MX main CPU clock (66 MHz in High mode); the reference frequency for the
/// timing-derived tests.
pub const MX_CPU_CLOCK_HZ: u32 = TownsModel::FmTownsIIMx.cpu_clock_hz(CpuMode::High);

/// Base of the linear VRAM window in the 32-bit memory map.
pub const VRAM_BASE: u32 = 0x8000_0000;

/// A zero-filled ROM set sized so the reset-vector fetch and the CPU run loop
/// index into valid backing storage.
pub fn synthetic_roms() -> LoadedRoms {
    LoadedRoms {
        dos: vec![0; ROM_LARGE_LEN],
        font: vec![0; ROM_SYSTEM_LEN],
        system: vec![0; ROM_SYSTEM_LEN],
        f20: vec![0; ROM_LARGE_LEN],
        dictionary: vec![0; ROM_LARGE_LEN],
        serial: vec![0; ROM_SERIAL_LEN],
    }
}

/// A fully empty ROM set, for tests that never run the CPU (they drive the bus
/// I/O surface directly, so no ROM content is indexed).
pub fn empty_roms() -> LoadedRoms {
    LoadedRoms {
        dos: Vec::new(),
        font: Vec::new(),
        system: Vec::new(),
        f20: Vec::new(),
        dictionary: Vec::new(),
        serial: Vec::new(),
    }
}

/// A ROM set with custom FONT (kanji CG-ROM) and serial-ID images.
pub fn font_serial_roms(font: Vec<u8>, serial: Vec<u8>) -> LoadedRoms {
    LoadedRoms {
        dos: vec![0; ROM_LARGE_LEN],
        font,
        system: vec![0; ROM_SYSTEM_LEN],
        f20: vec![0; ROM_LARGE_LEN],
        dictionary: vec![0; ROM_LARGE_LEN],
        serial,
    }
}

/// An MX machine (i486, High mode) over synthetic ROMs.
pub fn machine_mx() -> TownsMachine<{ CPU_MODEL_486_DX }> {
    build_machine(TownsModel::FmTownsIIMx, CpuMode::High, synthetic_roms())
}

/// A CX machine (i386, Low mode) over synthetic ROMs.
pub fn machine_cx() -> TownsMachine<{ CPU_MODEL_386_DX }> {
    build_machine(TownsModel::FmTownsIICx, CpuMode::Low, synthetic_roms())
}

/// A base FM Towns machine (i386SX, Low mode) over synthetic ROMs.
pub fn machine_base() -> TownsMachine<{ CPU_MODEL_386_SX }> {
    build_machine(TownsModel::FmTowns, CpuMode::Low, synthetic_roms())
}

/// An MX machine over synthetic ROMs whose bus activity is recorded.
pub fn machine_mx_traced() -> TownsMachine<{ CPU_MODEL_486_DX }, RecordingTracer> {
    build_machine(TownsModel::FmTownsIIMx, CpuMode::High, synthetic_roms())
}

/// An MX machine with custom FONT and serial-ID ROM images.
pub fn machine_with_font_serial(
    font: Vec<u8>,
    serial: Vec<u8>,
) -> TownsMachine<{ CPU_MODEL_486_DX }> {
    build_machine(
        TownsModel::FmTownsIIMx,
        CpuMode::High,
        font_serial_roms(font, serial),
    )
}

/// Builds a reset CPU around a configured FM Towns bus.
fn build_machine<const CPU_MODEL: u8, T: Tracing + Default>(
    model: TownsModel,
    cpu_mode: CpuMode,
    roms: LoadedRoms,
) -> TownsMachine<CPU_MODEL, T> {
    let bus = TownsBus::new(model, cpu_mode, roms, 48_000);
    let mut cpu = cpu::I386::<CPU_MODEL, { cpu::ADDRESS_WIDTH_32 }>::new();
    cpu.reset();
    TownsMachine::new(cpu, bus)
}

/// Writes a 16-bit CRTC register through its index/data ports.
pub fn write_crtc(bus: &mut impl Bus, index: u8, value: u16) {
    bus.io_write_byte(0x0440, index);
    bus.io_write_byte(0x0442, value as u8);
    bus.io_write_byte(0x0443, (value >> 8) as u8);
}

/// Writes a sprite-controller register through its index/data ports.
pub fn write_sprite_reg(bus: &mut impl Bus, index: u8, value: u8) {
    bus.io_write_byte(0x0450, index);
    bus.io_write_byte(0x0452, value);
}

/// Reads a little-endian 16-bit word from the linear VRAM window.
pub fn read_vram_word(bus: &mut impl Bus, offset: u32) -> u16 {
    let low = bus.read_byte(VRAM_BASE + offset);
    let high = bus.read_byte(VRAM_BASE + offset + 1);
    u16::from(low) | (u16::from(high) << 8)
}

/// Advances the bus clock to `cycle`, firing any events due by then.
pub fn advance_to(bus: &mut impl Bus, cycle: u64) {
    bus.set_current_cycle(cycle);
}

/// Advances the bus clock by `delta` cycles.
pub fn advance_by(bus: &mut impl Bus, delta: u64) {
    let target = bus.current_cycle().saturating_add(delta);
    bus.set_current_cycle(target);
}

// uPD71071 main-DMA register offsets (added to the 0x00A0 base port). The chip
// selects the register by `port & 0x0F`, so these hit the same registers the
// SYSROM programs.
const DMA_BASE_PORT: u16 = 0x00A0;
const DMA_CHANNEL_SELECT: u16 = 0x01;
const DMA_COUNT_LOW: u16 = 0x02;
const DMA_COUNT_HIGH: u16 = 0x03;
const DMA_ADDRESS_BYTE0: u16 = 0x04;
const DMA_ADDRESS_BYTE1: u16 = 0x05;
const DMA_ADDRESS_BYTE2: u16 = 0x06;
const DMA_ADDRESS_BYTE3: u16 = 0x07;
const DMA_MASK: u16 = 0x0F;

/// Programs a main-DMA channel for a `byte_count`-byte transfer at `address`
/// through the uPD71071's I/O ports, then unmasks all channels.
pub fn program_dma_channel(bus: &mut impl Bus, channel: u8, address: u32, byte_count: u16) {
    bus.io_write_byte(DMA_BASE_PORT + DMA_CHANNEL_SELECT, channel);
    let count = byte_count - 1; // the count register holds transfers minus one
    bus.io_write_byte(DMA_BASE_PORT + DMA_COUNT_LOW, count as u8);
    bus.io_write_byte(DMA_BASE_PORT + DMA_COUNT_HIGH, (count >> 8) as u8);
    bus.io_write_byte(DMA_BASE_PORT + DMA_ADDRESS_BYTE0, address as u8);
    bus.io_write_byte(DMA_BASE_PORT + DMA_ADDRESS_BYTE1, (address >> 8) as u8);
    bus.io_write_byte(DMA_BASE_PORT + DMA_ADDRESS_BYTE2, (address >> 16) as u8);
    bus.io_write_byte(DMA_BASE_PORT + DMA_ADDRESS_BYTE3, (address >> 24) as u8);
    bus.io_write_byte(DMA_BASE_PORT + DMA_MASK, 0x00);
}

// Access-rights bytes for real-mode segments (Intel 80486 PRM Chapter 6):
// present, ring 0, code/data descriptor, readable code / writable data,
// accessed.
const RIGHTS_CODE_READABLE: u8 = 0x9B;
const RIGHTS_DATA_WRITABLE: u8 = 0x93;

/// Builds a real-mode CPU state entering at `cs_base:ip` with a stack at
/// `ss_base:sp`. Segment bases are flat (selectors are cosmetic in real mode),
/// so the entry linear address is `cs_base + ip`.
pub fn real_mode_state(cs_base: u32, ip: u16, ss_base: u32, sp: u16) -> I386State {
    let mut state = I386State {
        ip,
        ..I386State::default()
    };
    state.set_cs((cs_base >> 4) as u16);
    state.set_ds(0);
    state.set_es(0);
    state.set_ss((ss_base >> 4) as u16);
    state.set_esp(u32::from(sp));
    state.seg_bases[SegReg32::CS as usize] = cs_base;
    state.seg_bases[SegReg32::DS as usize] = 0;
    state.seg_bases[SegReg32::ES as usize] = 0;
    state.seg_bases[SegReg32::SS as usize] = ss_base;
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[SegReg32::CS as usize] = RIGHTS_CODE_READABLE;
    state.seg_rights[SegReg32::DS as usize] = RIGHTS_DATA_WRITABLE;
    state.seg_rights[SegReg32::ES as usize] = RIGHTS_DATA_WRITABLE;
    state.seg_rights[SegReg32::SS as usize] = RIGHTS_DATA_WRITABLE;
    state.seg_valid = [true, true, true, true, false, false];
    state
}

/// Writes a byte sequence into memory at a linear address.
pub fn load_code(bus: &mut impl Bus, linear: u32, bytes: &[u8]) {
    for (offset, &byte) in bytes.iter().enumerate() {
        bus.write_byte(linear + offset as u32, byte);
    }
}

/// The CPU's current linear instruction pointer (CS base + EIP).
pub fn linear_pc<const CPU_MODEL: u8, T: Tracing + Default>(
    machine: &TownsMachine<CPU_MODEL, T>,
) -> u32 {
    machine.cpu.state.seg_bases[SegReg32::CS as usize].wrapping_add(machine.cpu.state.eip())
}

/// A fixed RTC time source (BCD): 2000-01-01 (Saturday) 12:34:56. Deterministic
/// so RTC readback tests do not depend on the wall clock. The layout matches the
/// bus default: `[year, month<<4 | weekday, day, hour, minute, second]`.
pub fn fixed_time() -> HostDateTime {
    HostDateTime {
        year: 2000,
        month: 1,
        day: 1,
        day_of_week: 6,
        hour: 12,
        minute: 34,
        second: 56,
    }
}

/// Records the distinct I/O ports touched and the IRQ lines raised, for
/// behavioral assertions without a framebuffer.
#[derive(Clone, Default)]
pub struct RecordingTracer {
    pub ports_read: Rc<RefCell<BTreeSet<u16>>>,
    pub ports_written: Rc<RefCell<BTreeSet<u16>>>,
    pub irqs_raised: Rc<RefCell<BTreeSet<u8>>>,
}

impl Tracing for RecordingTracer {
    fn trace_io_read(&mut self, port: u16, _value: u8) {
        self.ports_read.borrow_mut().insert(port);
    }

    fn trace_io_write(&mut self, port: u16, _value: u8) {
        self.ports_written.borrow_mut().insert(port);
    }

    fn trace_irq_raise(&mut self, irq: u8) {
        self.irqs_raised.borrow_mut().insert(irq);
    }
}
