//! Shared harness for the PC/AT HLE BIOS test suite.
//!
//! Mirrors `machine_98/tests/bios.rs`: machine factories for both AT models,
//! raw boot media builders whose boot sector is CLI;HLT, a `boot_to_halt!`
//! macro, and BDA/IVT read helpers. The per-area sub-tests live under
//! `tests/bios/` and are included with `#[path = ...]`.

use common::NoTrace;
use device::{disk::HddImage, floppy::FloppyImage};
use machine_at::{AtMachine, AtModel, LoadedRoms};

#[path = "common/harness.rs"]
mod harness;

#[path = "common/mode_vectors.rs"]
mod mode_vectors;

/// Boot sector program: CLI; HLT.
pub const HALT_BOOT_CODE: [u8; 2] = [0xFA, 0xF4];
/// Raw 360K floppy image size in bytes.
pub const FLOPPY_360K_SIZE: usize = 368_640;
/// Raw 720K floppy image size in bytes.
pub const FLOPPY_720K_SIZE: usize = 737_280;
/// Raw 1.2 MB floppy image size in bytes.
pub const FLOPPY_1200K_SIZE: usize = 1_228_800;
/// Raw 1.23 MB 3-mode floppy image size in bytes.
pub const FLOPPY_1232K_SIZE: usize = 1_261_568;
/// Raw 1.44 MB floppy image size in bytes.
pub const FLOPPY_1440K_SIZE: usize = 1_474_560;
/// One AT flat hard disk cylinder (16 heads * 63 sectors * 512 bytes).
pub const AT_FLAT_CYLINDER_SIZE: usize = 16 * 63 * 512;
/// Physical load address of the boot sector.
pub const BOOT_SECTOR_ADDRESS: u32 = 0x7C00;

/// Runs the machine in 1M-cycle slices until the CPU halts, panicking after
/// 500M cycles. Returns the cycles consumed.
macro_rules! boot_to_halt {
    ($machine:expr) => {{
        use common::Cpu;
        let mut total_cycles: u64 = 0;
        while !$machine.cpu.halted() {
            $machine.run_for(1_000_000);
            total_cycles += 1_000_000;
            assert!(
                total_cycles < 500_000_000,
                "machine did not reach HLT within the cycle budget"
            );
        }
        total_cycles
    }};
}

/// Like `boot_to_halt!`, with a caller-chosen cycle budget (the real AMI POST
/// takes far longer than the HLE POST).
macro_rules! boot_to_halt_with_budget {
    ($machine:expr, $budget:expr) => {{
        use common::Cpu;
        let mut total_cycles: u64 = 0;
        while !$machine.cpu.halted() {
            $machine.run_for(1_000_000);
            total_cycles += 1_000_000;
            assert!(
                total_cycles < $budget,
                "machine did not reach HLT within the cycle budget"
            );
        }
        total_cycles
    }};
}

/// Builds an i486DX2-50 machine over the embedded HLE stub ROM set.
pub fn create_machine_dx50() -> AtMachine<NoTrace> {
    harness::machine_with_roms(AtModel::At486Dx50, LoadedRoms::hle_stub_set())
}

/// Builds an i486DX2-66 machine over the embedded HLE stub ROM set.
pub fn create_machine_dx66() -> AtMachine<NoTrace> {
    harness::machine_with_roms(AtModel::At486Dx66, LoadedRoms::hle_stub_set())
}

/// Builds a raw 1.44 MB floppy whose boot sector is CLI;HLT.
pub fn make_halt_boot_floppy() -> FloppyImage {
    let mut data = vec![0u8; FLOPPY_1440K_SIZE];
    data[..HALT_BOOT_CODE.len()].copy_from_slice(&HALT_BOOT_CODE);
    FloppyImage::from_img_bytes(&data).expect("build 1.44 MB boot floppy")
}

/// Builds the raw bytes of a pattern floppy: every 512-byte unit is stamped
/// with its index (low byte, high byte, 0xA5 marker, index-derived fill) and
/// the boot sector starts with CLI;HLT.
pub fn make_pattern_floppy_data(size: usize) -> Vec<u8> {
    let mut data = vec![0u8; size];
    for unit in 0..size / 512 {
        let base = unit * 512;
        data[base] = unit as u8;
        data[base + 1] = (unit >> 8) as u8;
        data[base + 2] = 0xA5;
        for byte in data[base + 3..base + 512].iter_mut() {
            *byte = (unit as u8) ^ 0x5A;
        }
    }
    data[..HALT_BOOT_CODE.len()].copy_from_slice(&HALT_BOOT_CODE);
    data
}

/// Builds a raw pattern floppy image of the given size.
pub fn make_pattern_floppy(size: usize) -> FloppyImage {
    FloppyImage::from_img_bytes(&make_pattern_floppy_data(size)).expect("build pattern floppy")
}

/// Builds a one-cylinder AT flat hard disk whose MBR is CLI;HLT with the
/// 0x55AA signature.
pub fn make_halt_boot_hdd() -> HddImage {
    let mut data = vec![0u8; AT_FLAT_CYLINDER_SIZE];
    data[..HALT_BOOT_CODE.len()].copy_from_slice(&HALT_BOOT_CODE);
    data[510] = 0x55;
    data[511] = 0xAA;
    HddImage::from_at_flat(data).expect("build AT boot hard disk")
}

/// Builds a one-cylinder AT flat hard disk whose MBR lacks the 0x55AA
/// signature.
pub fn make_unsigned_boot_hdd() -> HddImage {
    let mut data = vec![0u8; AT_FLAT_CYLINDER_SIZE];
    data[..HALT_BOOT_CODE.len()].copy_from_slice(&HALT_BOOT_CODE);
    HddImage::from_at_flat(data).expect("build unsigned AT hard disk")
}

/// Builds the raw bytes of a pattern AT hard disk: every 512-byte sector is
/// stamped with its 24-bit index (three low bytes, 0xA5 marker,
/// index-derived fill) and the MBR is CLI;HLT with the boot signature.
pub fn make_pattern_hdd_data(cylinders: usize) -> Vec<u8> {
    let size = cylinders * AT_FLAT_CYLINDER_SIZE;
    let mut data = vec![0u8; size];
    for unit in 0..size / 512 {
        let base = unit * 512;
        data[base] = unit as u8;
        data[base + 1] = (unit >> 8) as u8;
        data[base + 2] = (unit >> 16) as u8;
        data[base + 3] = 0xA5;
        for byte in data[base + 4..base + 512].iter_mut() {
            *byte = (unit as u8) ^ 0x5A;
        }
    }
    data[..HALT_BOOT_CODE.len()].copy_from_slice(&HALT_BOOT_CODE);
    data[510] = 0x55;
    data[511] = 0xAA;
    data
}

/// Builds a pattern AT hard disk of the given cylinder count.
pub fn make_pattern_hdd(cylinders: usize) -> HddImage {
    HddImage::from_at_flat(make_pattern_hdd_data(cylinders)).expect("build pattern hard disk")
}

/// Physical address where injected test programs are placed.
pub const TEST_CODE: u32 = 0x1000;
/// Physical address of the injected interrupt callback.
pub const TEST_CALLBACK: u32 = 0x2000;
/// Result scratch area written by injected test programs.
pub const RESULT: u32 = 0x0600;

/// Writes bytes into guest RAM through the bus.
pub fn write_bytes(machine: &mut AtMachine<NoTrace>, address: u32, bytes: &[u8]) {
    use common::Bus;
    for (index, &byte) in bytes.iter().enumerate() {
        machine.bus.write_byte(address + index as u32, byte);
    }
}

/// Places test code at `TEST_CODE` (and an optional callback at
/// `TEST_CALLBACK`), loads a fresh real-mode CPU state with IP at
/// `TEST_CODE`, and runs the machine under the cycle budget.
pub fn inject_and_run(
    machine: &mut AtMachine<NoTrace>,
    main_code: &[u8],
    callback: &[u8],
    budget: u64,
) -> u64 {
    write_bytes(machine, TEST_CODE, main_code);
    if !callback.is_empty() {
        write_bytes(machine, TEST_CALLBACK, callback);
    }
    machine.cpu.load_state(&{
        let mut state = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        state.set_esp(0x4000);
        state
    });
    machine.run_for(budget)
}

/// Boots to the idle halt, then injects and runs test code.
pub fn boot_and_run(
    machine: &mut AtMachine<NoTrace>,
    main_code: &[u8],
    callback: &[u8],
    budget: u64,
) -> u64 {
    boot_to_halt!(machine);
    inject_and_run(machine, main_code, callback, budget)
}

/// Reads one byte of guest RAM without device side effects.
pub fn read_ram_u8(machine: &AtMachine<NoTrace>, address: u32) -> u8 {
    machine.bus.peek_byte(address)
}

/// Reads one little-endian word of guest RAM without device side effects.
pub fn read_ram_u16(machine: &AtMachine<NoTrace>, address: u32) -> u16 {
    u16::from(machine.bus.peek_byte(address)) | (u16::from(machine.bus.peek_byte(address + 1)) << 8)
}

/// Reads one little-endian doubleword of guest RAM without device side effects.
pub fn read_ram_u32(machine: &AtMachine<NoTrace>, address: u32) -> u32 {
    u32::from(read_ram_u16(machine, address))
        | (u32::from(read_ram_u16(machine, address + 2)) << 16)
}

/// Reads one IVT entry as a (segment, offset) pair.
pub fn read_ivt_vector(machine: &AtMachine<NoTrace>, vector: u8) -> (u16, u16) {
    let address = u32::from(vector) * 4;
    let offset = read_ram_u16(machine, address);
    let segment = read_ram_u16(machine, address + 2);
    (segment, offset)
}

/// BIOS data area: keyboard shift flags byte 1.
pub const BDA_KEYBOARD_FLAGS_1: u32 = 0x417;
/// BIOS data area: keyboard shift flags byte 2.
pub const BDA_KEYBOARD_FLAGS_2: u32 = 0x418;
/// BIOS data area: Alt-numpad decimal entry accumulator.
pub const BDA_ALT_NUMPAD_ACCUMULATOR: u32 = 0x419;
/// BIOS data area: keyboard buffer head pointer (word).
pub const BDA_KEYBOARD_HEAD: u32 = 0x41A;
/// BIOS data area: keyboard buffer tail pointer (word).
pub const BDA_KEYBOARD_TAIL: u32 = 0x41C;
/// BIOS data area: break flag.
pub const BDA_BREAK_FLAG: u32 = 0x471;
/// BIOS data area: keyboard mode/type flags.
pub const BDA_KEYBOARD_MODE: u32 = 0x496;
/// BIOS data area: keyboard LED flags.
pub const BDA_KEYBOARD_LEDS: u32 = 0x497;
/// Linear address of the first keyboard buffer entry.
pub const KEYBOARD_BUFFER: u32 = 0x41E;
/// Keyboard buffer start offset within segment 0x40.
pub const KEYBOARD_BUFFER_START: u16 = 0x001E;

/// Guest idle loop: STI, then HLT re-entered forever, so pending hardware
/// interrupts are serviced through the stub ROM handlers.
#[rustfmt::skip]
pub const IDLE_LOOP_CODE: &[u8] = &[
    0xFB,                   // STI
    0xF4,                   // HLT
    0xEB, 0xFD,             // JMP to the HLT
];

/// Boots to the idle halt, queues host key events (set-1 ids, bit 7 =
/// release), then services the resulting IRQ 1 stream in an idle loop.
pub fn boot_push_keys_and_run(machine: &mut AtMachine<NoTrace>, keys: &[u8], budget: u64) {
    use common::Machine;
    boot_to_halt!(machine);
    for &key in keys {
        machine.push_keyboard_scancode(key);
    }
    inject_and_run(machine, IDLE_LOOP_CODE, &[], budget);
}

/// Seeds the BDA keyboard ring buffer with the given entries.
pub fn seed_keyboard_buffer(machine: &mut AtMachine<NoTrace>, entries: &[u16]) {
    let tail = KEYBOARD_BUFFER_START + 2 * entries.len() as u16;
    let mut bytes = Vec::new();
    for &entry in entries {
        bytes.extend_from_slice(&entry.to_le_bytes());
    }
    write_bytes(machine, KEYBOARD_BUFFER, &bytes);
    write_bytes(
        machine,
        BDA_KEYBOARD_HEAD,
        &KEYBOARD_BUFFER_START.to_le_bytes(),
    );
    write_bytes(machine, BDA_KEYBOARD_TAIL, &tail.to_le_bytes());
}

#[path = "bios/boot_order.rs"]
mod boot_order;
#[path = "bios/fdc_interrupt.rs"]
mod fdc_interrupt;
#[path = "bios/fdd_boot.rs"]
mod fdd_boot;
#[path = "bios/hdd_boot.rs"]
mod hdd_boot;
#[path = "bios/hdd_interrupt.rs"]
mod hdd_interrupt;
#[path = "bios/int10h_graphics.rs"]
mod int10h_graphics;
#[path = "bios/int10h_modes.rs"]
mod int10h_modes;
#[path = "bios/int10h_palette.rs"]
mod int10h_palette;
#[path = "bios/int10h_save_pointer.rs"]
mod int10h_save_pointer;
#[path = "bios/int10h_text.rs"]
mod int10h_text;
#[path = "bios/int11h_12h.rs"]
mod int11h_12h;
#[path = "bios/int13h_floppy.rs"]
mod int13h_floppy;
#[path = "bios/int13h_hdd.rs"]
mod int13h_hdd;
#[path = "bios/int14h.rs"]
mod int14h;
#[path = "bios/int15h.rs"]
mod int15h;
#[path = "bios/int15h_pmode.rs"]
mod int15h_pmode;
#[path = "bios/int16h.rs"]
mod int16h;
#[path = "bios/int17h.rs"]
mod int17h;
#[path = "bios/int1ah.rs"]
mod int1ah;
#[path = "bios/irq_stubs.rs"]
mod irq_stubs;
#[path = "bios/keyboard.rs"]
mod keyboard;
#[path = "bios/post_bios_state.rs"]
mod post_bios_state;
#[path = "bios/timer_tick.rs"]
mod timer_tick;
#[path = "bios/warm_boot.rs"]
mod warm_boot;
