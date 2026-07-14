use common::{BUILTIN_FONT_ROM, Bus, CpuMode, MachineModel};
use device::floppy::FloppyImage;
use machine_98::{Pc9801Bus, Pc9801Ra, Pc9801Vm, Pc9801Vx};

const TEST_CODE: u32 = 0x1000;
const HOOK_HANDLER: u32 = 0x2000;
const RESULT: u32 = 0x0600;
const BUDGET: u64 = 2_000_000;

fn make_halt_boot_disk() -> FloppyImage {
    let mut boot_sector = vec![0u8; 1024];
    boot_sector[0] = 0xFA; // CLI
    boot_sector[1] = 0xF4; // HLT

    const HEADER_SIZE: usize = 0x2B0;
    const SECTOR_HEADER_SIZE: usize = 16;

    let mut image = vec![0u8; HEADER_SIZE];
    image[..4].copy_from_slice(b"TEST");
    image[0x1B] = 0x20; // 2HD

    let track_offset = image.len() as u32;
    let pointer_base = 0x20;
    image[pointer_base..pointer_base + 4].copy_from_slice(&track_offset.to_le_bytes());

    let mut header = [0u8; SECTOR_HEADER_SIZE];
    header[0] = 0; // cylinder
    header[1] = 0; // head
    header[2] = 1; // record
    header[3] = 3; // n (1024 bytes)
    header[4..6].copy_from_slice(&1u16.to_le_bytes()); // 1 sector
    header[0x0E..0x10].copy_from_slice(&(boot_sector.len() as u16).to_le_bytes());
    image.extend_from_slice(&header);
    image.extend_from_slice(&boot_sector);

    let disk_size = image.len() as u32;
    image[0x1C..0x20].copy_from_slice(&disk_size.to_le_bytes());

    FloppyImage::from_d88_bytes(&image).expect("halt boot disk")
}

macro_rules! boot_to_halt {
    ($machine:expr) => {{
        use common::Cpu as _;
        let disk = $crate::make_halt_boot_disk();
        $machine.bus.insert_floppy(0, disk, None);

        const MAX_CYCLES: u64 = 500_000_000;
        const CHECK_INTERVAL: u64 = 1_000_000;

        let mut total_cycles = 0u64;
        loop {
            total_cycles += $machine.run_for(CHECK_INTERVAL);
            if $machine.cpu.halted() {
                break;
            }
            assert!(
                total_cycles < MAX_CYCLES,
                "Machine did not halt within {} cycles",
                MAX_CYCLES
            );
        }
        total_cycles
    }};
}

fn create_vm() -> Pc9801Vm {
    let mut machine = Pc9801Vm::new(
        cpu::V30::new(),
        Pc9801Bus::new(MachineModel::PC9801VM, CpuMode::High, 48000),
    );
    machine.bus.load_font_rom(BUILTIN_FONT_ROM);
    machine
}

fn create_vx() -> Pc9801Vx {
    let mut machine = Pc9801Vx::new(
        cpu::I286::new(),
        Pc9801Bus::new(MachineModel::PC9801VX, CpuMode::High, 48000),
    );
    machine.bus.load_font_rom(BUILTIN_FONT_ROM);
    machine
}

fn create_ra() -> Pc9801Ra {
    let mut machine = Pc9801Ra::new(
        cpu::I386::new(),
        Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::High, 48000),
    );
    machine.bus.load_font_rom(BUILTIN_FONT_ROM);
    machine
}

fn write_bytes(bus: &mut impl Bus, addr: u32, data: &[u8]) {
    for (i, &b) in data.iter().enumerate() {
        bus.write_byte(addr + i as u32, b);
    }
}

fn read_ivt_vector(bus: &mut impl Bus, vector: u8) -> (u16, u16) {
    let base = (vector as u32) * 4;
    let offset = bus.read_byte(base) as u16 | (bus.read_byte(base + 1) as u16) << 8;
    let segment = bus.read_byte(base + 2) as u16 | (bus.read_byte(base + 3) as u16) << 8;
    (segment, offset)
}

fn read_rom_byte(bus: &mut impl Bus, segment: u16, offset: u16, delta: u16) -> u8 {
    let addr = (segment as u32) << 4 | (offset.wrapping_add(delta)) as u32;
    bus.read_byte(addr)
}

/// Build the MS-DOS-style hook handler: STI / PUSH DS / PUSH DX / JMP FAR seg:(off+25)
#[rustfmt::skip]
fn make_dos_chain_hook(orig_seg: u16, orig_off: u16) -> Vec<u8> {
    let target_off = orig_off.wrapping_add(25);
    vec![
        0xFB,                                           // STI
        0x1E,                                           // PUSH DS
        0x52,                                           // PUSH DX
        0xEA,                                           // JMP FAR ptr16:16
        (target_off & 0xFF) as u8,                      //   offset lo
        (target_off >> 8) as u8,                        //   offset hi
        (orig_seg & 0xFF) as u8,                        //   segment lo
        (orig_seg >> 8) as u8,                          //   segment hi
    ]
}

/// Test code: MOV AH, 0x12 / INT 1Ah / MOV [RESULT], AX / HLT
#[rustfmt::skip]
fn make_printer_status_test() -> Vec<u8> {
    vec![
        0xB4, 0x12,             // MOV AH, 0x12
        0xCD, 0x1A,             // INT 0x1A
        0xA3, 0x00, 0x06,       // MOV [RESULT], AX
        0xF4,                   // HLT
    ]
}

// ============================================================================
// INT 1Ah +25 Chaining Contract - Structural
// ============================================================================
//
// Verify that the HLE BIOS ROM has valid code at INT 1Ah handler + 25,
// matching the MS-DOS chaining convention. The standard stub is padded to
// 25 bytes; the chainable entry at +25 starts with POP DX / POP DS
// (cleaning up MS-DOS's pushed registers) then does the HLE trap + IRET.

fn assert_int1ah_chain_entry_valid(bus: &mut impl Bus) {
    let (segment, offset) = read_ivt_vector(bus, 0x1A);

    assert!(
        segment >= 0xFD80,
        "INT 1Ah should point to BIOS ROM (got {segment:#06X}:{offset:#06X})"
    );

    // Standard stub at +0 starts with PUSH AX (0x50)
    assert_eq!(
        read_rom_byte(bus, segment, offset, 0),
        0x50,
        "handler+0 should be PUSH AX (0x50)"
    );

    // Standard stub IRET at +8
    assert_eq!(
        read_rom_byte(bus, segment, offset, 8),
        0xCF,
        "handler+8 should be IRET (0xCF)"
    );

    // Chained entry at +25 starts with POP DX (0x5A) to clean up MS-DOS's pushed DX
    assert_eq!(
        read_rom_byte(bus, segment, offset, 25),
        0x5A,
        "handler+25 (chained entry) should be POP DX (0x5A)"
    );

    // Followed by POP DS (0x1F) to clean up MS-DOS's pushed DS
    assert_eq!(
        read_rom_byte(bus, segment, offset, 26),
        0x1F,
        "handler+26 should be POP DS (0x1F)"
    );

    // Chained entry ends with IRET at +35
    assert_eq!(
        read_rom_byte(bus, segment, offset, 35),
        0xCF,
        "handler+35 should be IRET (0xCF)"
    );
}

#[test]
fn hle_int1ah_chain_entry_valid_vm() {
    let mut machine = create_vm();
    boot_to_halt!(machine);
    assert_int1ah_chain_entry_valid(&mut machine.bus);
}

#[test]
fn hle_int1ah_chain_entry_valid_vx() {
    let mut machine = create_vx();
    boot_to_halt!(machine);
    assert_int1ah_chain_entry_valid(&mut machine.bus);
}

#[test]
fn hle_int1ah_chain_entry_valid_ra() {
    let mut machine = create_ra();
    boot_to_halt!(machine);
    assert_int1ah_chain_entry_valid(&mut machine.bus);
}

// ============================================================================
// INT 1Ah +25 Chaining Contract - Functional (MS-DOS Hook Simulation)
// ============================================================================
//
// Simulate the MS-DOS INT 1Ah hooking pattern: install a hook that saves DS/DX
// and far-jumps to old_vector+25. Calling INT 1Ah through this hook must reach
// the HLE handler and return correct results with a clean stack.

#[test]
fn hle_int1ah_dos_chain_vm() {
    let mut machine = create_vm();
    boot_to_halt!(machine);

    let (orig_seg, orig_off) = read_ivt_vector(&mut machine.bus, 0x1A);
    let hook = make_dos_chain_hook(orig_seg, orig_off);
    write_bytes(&mut machine.bus, HOOK_HANDLER, &hook);

    // Patch IVT: INT 1Ah -> 0x0000:HOOK_HANDLER
    machine.bus.write_word(0x1A * 4, HOOK_HANDLER as u16);
    machine.bus.write_word(0x1A * 4 + 2, 0x0000);

    let code = make_printer_status_test();
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(BUDGET);

    let ax = machine.bus.read_byte(RESULT) as u16 | (machine.bus.read_byte(RESULT + 1) as u16) << 8;
    assert_eq!(
        ax >> 8,
        0x01,
        "Chained INT 1Ah AH=12h should return AH=0x01 (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn hle_int1ah_dos_chain_vx() {
    let mut machine = create_vx();
    boot_to_halt!(machine);

    let (orig_seg, orig_off) = read_ivt_vector(&mut machine.bus, 0x1A);
    let hook = make_dos_chain_hook(orig_seg, orig_off);
    write_bytes(&mut machine.bus, HOOK_HANDLER, &hook);

    machine.bus.write_word(0x1A * 4, HOOK_HANDLER as u16);
    machine.bus.write_word(0x1A * 4 + 2, 0x0000);

    let code = make_printer_status_test();
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(BUDGET);

    let ax = machine.bus.read_byte(RESULT) as u16 | (machine.bus.read_byte(RESULT + 1) as u16) << 8;
    assert_eq!(
        ax >> 8,
        0x01,
        "Chained INT 1Ah AH=12h should return AH=0x01 (got {:#04X})",
        ax >> 8
    );
}

#[test]
fn hle_int1ah_dos_chain_ra() {
    let mut machine = create_ra();
    boot_to_halt!(machine);

    let (orig_seg, orig_off) = read_ivt_vector(&mut machine.bus, 0x1A);
    let hook = make_dos_chain_hook(orig_seg, orig_off);
    write_bytes(&mut machine.bus, HOOK_HANDLER, &hook);

    machine.bus.write_word(0x1A * 4, HOOK_HANDLER as u16);
    machine.bus.write_word(0x1A * 4 + 2, 0x0000);

    let code = make_printer_status_test();
    write_bytes(&mut machine.bus, TEST_CODE, &code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(BUDGET);

    let ax = machine.bus.read_byte(RESULT) as u16 | (machine.bus.read_byte(RESULT + 1) as u16) << 8;
    assert_eq!(
        ax >> 8,
        0x01,
        "Chained INT 1Ah AH=12h should return AH=0x01 (got {:#04X})",
        ax >> 8
    );
}
