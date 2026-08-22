//! Shared helpers for machine_x68k integration tests: synthetic and scripted
//! ROM sets, supervisor bus accessors, a DMAC channel-1 programmer, in-memory
//! X68000 .hdf image builders, and framebuffer probing.

#![allow(dead_code)]

use common::{
    Bus, CpuM68000, CpuMode, M68000AccessSize, M68000BusAccess, M68000CycleKind, M68000FunctionCode,
};
use device::disk::{HddImage, X68K_SASI_HDF_10MB_BYTES};
use machine_x68k::{LoadedRoms, X68kBus, X68kMachine, X68kModel};

/// Size of the mapped character-generator ROM.
const CGROM_SIZE: usize = 0x0C_0000;
/// Size of the mapped IPL ROM.
const IPL_SIZE: usize = 0x02_0000;

/// Base address of the DMAC channel-1 register block.
pub const DMAC_CHANNEL1_BASE: u32 = 0xE84040;

/// Builds a synthetic ROM set whose reset vector points into the IPL.
pub fn test_roms(model: X68kModel) -> LoadedRoms {
    let mut ipl = vec![0; IPL_SIZE];
    ipl[0x10000..0x10008].copy_from_slice(&[0, 0xC0, 0, 0, 0, 0xFE, 0, 8]);
    LoadedRoms {
        model,
        cgrom: vec![0xCC; CGROM_SIZE],
        ipl,
        internal_scsi: model.has_internal_scsi().then(|| vec![0x5A; 0x2000]),
        uses_compatibility_scsi: model == X68kModel::X68000Xvi,
    }
}

/// Builds a machine from the synthetic ROM set.
pub fn machine(model: X68kModel) -> X68kMachine {
    machine_from_roms(model, CpuMode::High, test_roms(model))
}

/// Builds a machine with an explicit CPU speed mode.
pub fn machine_with_mode(model: X68kModel, cpu_mode: CpuMode) -> X68kMachine {
    machine_from_roms(model, cpu_mode, test_roms(model))
}

/// Builds a reset machine from an explicit ROM set.
pub fn machine_from_roms(model: X68kModel, cpu_mode: CpuMode, roms: LoadedRoms) -> X68kMachine {
    let bus = X68kBus::new(model, cpu_mode, roms, 48_000).unwrap();
    X68kMachine::from_bus(model, cpu_mode, bus)
}

/// Reset supervisor stack pointer used by scripted synthetic IPLs.
pub const SCRIPT_STACK_POINTER: u32 = 0x0010_0000;

/// Builds a synthetic ROM set whose IPL executes the given 68000 words from
/// the reset vector.
pub fn scripted_roms(model: X68kModel, program: &[u16]) -> LoadedRoms {
    let mut roms = test_roms(model);
    roms.ipl[0x10000..0x10004].copy_from_slice(&SCRIPT_STACK_POINTER.to_be_bytes());
    for (index, word) in program.iter().enumerate() {
        let offset = 8 + index * 2;
        roms.ipl[offset..offset + 2].copy_from_slice(&word.to_be_bytes());
    }
    roms
}

/// Builds a machine that executes the given 68000 words from reset.
pub fn scripted_machine(model: X68kModel, program: &[u16]) -> X68kMachine {
    machine_from_roms(model, CpuMode::High, scripted_roms(model, program))
}

/// Assembles one `move.b #value, (address).l` per entry.
pub fn byte_write_script(writes: &[(u32, u8)]) -> Vec<u16> {
    let mut program = Vec::new();
    for &(address, value) in writes {
        program.extend([
            0x13FC,
            u16::from(value),
            (address >> 16) as u16,
            address as u16,
        ]);
    }
    program
}

/// Assembles one `move.w #value, (address).l` per entry.
pub fn word_write_script(writes: &[(u32, u16)]) -> Vec<u16> {
    let mut program = Vec::new();
    for &(address, value) in writes {
        program.extend([0x33FC, value, (address >> 16) as u16, address as u16]);
    }
    program
}

/// The `stop #0x2700` epilogue halting a scripted program.
pub const STOP_MASKED: [u16; 2] = [0x4E72, 0x2700];

/// Steps the CPU until STOP, returning the consumed cycles.
pub fn run_until_stop(machine: &mut X68kMachine, limit: usize) -> u64 {
    let mut total = 0;
    for _ in 0..limit {
        let cycles = machine.cpu.step(&mut machine.bus);
        if cycles == 0 {
            return total;
        }
        total += cycles;
    }
    panic!("the scripted program never reached STOP");
}

/// Returns the packed RGBA pixel at the framebuffer position.
pub fn pixel(machine: &X68kMachine, x: u32, y: u32) -> [u8; 4] {
    let (width, _) = machine.bus.display_dimensions();
    let offset = ((y * width + x) * 4) as usize;
    machine.bus.display_framebuffer()[offset..offset + 4]
        .try_into()
        .unwrap()
}

/// Builds a normal supervisor-data bus access.
fn access(address: u32, size: M68000AccessSize) -> M68000BusAccess {
    M68000BusAccess {
        address,
        size,
        function_code: M68000FunctionCode::SupervisorData,
        cycle_kind: M68000CycleKind::Normal,
    }
}

/// Reads one byte register through the supervisor bus.
pub fn read_byte(machine: &mut X68kMachine, address: u32) -> u8 {
    machine
        .bus
        .m68000_read(access(address, M68000AccessSize::Byte))
        .expect("register read must not raise a CPU bus error") as u8
}

/// Writes one byte register through the supervisor bus.
pub fn write_byte(machine: &mut X68kMachine, address: u32, value: u8) {
    machine
        .bus
        .m68000_write(access(address, M68000AccessSize::Byte), u16::from(value))
        .expect("register write must not raise a CPU bus error");
}

/// Reads one word register through the supervisor bus.
pub fn read_word(machine: &mut X68kMachine, address: u32) -> u16 {
    machine
        .bus
        .m68000_read(access(address, M68000AccessSize::Word))
        .expect("register read must not raise a CPU bus error")
}

/// Writes one word register through the supervisor bus.
pub fn write_word(machine: &mut X68kMachine, address: u32, value: u16) {
    machine
        .bus
        .m68000_write(access(address, M68000AccessSize::Word), value)
        .expect("register write must not raise a CPU bus error");
}

/// Advances the machine through every pending device event.
pub fn run_pending_events(machine: &mut X68kMachine, limit: usize) {
    for _ in 0..limit {
        let Some(deadline) = machine.bus.next_event_cycle() else {
            return;
        };
        machine.bus.set_current_cycle(deadline);
        machine.bus.process_due_events();
    }
}

/// Programs DMAC channel 1 for a dual-address byte transfer between memory
/// and the storage-controller data register, then starts the operation.
/// `to_memory` selects the device-to-memory direction.
pub fn program_storage_dma(
    machine: &mut X68kMachine,
    memory_address: u32,
    device_address: u32,
    count: u16,
    to_memory: bool,
) {
    let base = DMAC_CHANNEL1_BASE;
    // Clear completion status, then DCR: dual address, 8-bit device port.
    write_byte(machine, base, 0xFF);
    write_byte(machine, base + 0x04, 0x00);
    // OCR: byte operand, external request; bit 7 selects device-to-memory.
    write_byte(machine, base + 0x05, if to_memory { 0x82 } else { 0x02 });
    // SCR: memory address counts up, device address static.
    write_byte(machine, base + 0x06, 0x04);
    write_byte(machine, base + 0x0A, (count >> 8) as u8);
    write_byte(machine, base + 0x0B, count as u8);
    for (index, byte) in memory_address.to_be_bytes().into_iter().enumerate() {
        write_byte(machine, base + 0x0C + index as u32, byte);
    }
    for (index, byte) in device_address.to_be_bytes().into_iter().enumerate() {
        write_byte(machine, base + 0x14 + index as u32, byte);
    }
    // CCR: start the operation.
    write_byte(machine, base + 0x07, 0x80);
}

/// Builds a 10 MB SASI .hdf image where every sector starts with its LBA
/// (little-endian) followed by a repeating pattern byte.
pub fn patterned_sasi_hdf() -> HddImage {
    let mut data = vec![0u8; X68K_SASI_HDF_10MB_BYTES];
    for (lba, sector) in data.as_chunks_mut::<256>().0.iter_mut().enumerate() {
        sector[..4].copy_from_slice(&(lba as u32).to_le_bytes());
        sector[4..].fill((lba as u8) ^ 0x5A);
    }
    HddImage::from_x68k_sasi(data).unwrap()
}

/// Builds a flat SCSI .hdf image of `megabytes` MiB with the same
/// LBA-stamped sector pattern over 512-byte sectors.
pub fn patterned_scsi_hdf(megabytes: usize) -> HddImage {
    let mut data = vec![0u8; megabytes << 20];
    for (lba, sector) in data.as_chunks_mut::<512>().0.iter_mut().enumerate() {
        sector[..4].copy_from_slice(&(lba as u32).to_le_bytes());
        sector[4..].fill((lba as u8) ^ 0xA5);
    }
    HddImage::from_raw_flat(data).unwrap()
}
