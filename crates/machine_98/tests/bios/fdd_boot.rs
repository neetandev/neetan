use common::Cpu;
use device::floppy::FloppyImage;

fn make_2hd_128_byte_sector_int1b_boot_disk() -> FloppyImage {
    let mut boot_sector = vec![0u8; 128];
    let code = [
        0xFA, // CLI
        0xB8, 0x60, 0x00, // MOV AX,0060h
        0x8E, 0xC0, // MOV ES,AX
        0xB8, 0x90, 0x16, // MOV AX,1690h
        0xBB, 0x00, 0x02, // MOV BX,0200h
        0xB9, 0x00, 0x00, // MOV CX,0000h
        0xBA, 0x01, 0x00, // MOV DX,0001h
        0x33, 0xED, // XOR BP,BP
        0xCD, 0x1B, // INT 1Bh
        0x72, 0x06, // JC fail
        0xEA, 0x00, 0x01, 0x60, 0x00, // JMP 0060:0100
        0xF4, // fail: HLT
    ];
    boot_sector[..code.len()].copy_from_slice(&code);

    let sector2 = vec![0x22u8; 128];
    let mut sector3 = vec![0u8; 128];
    sector3[0] = 0xFA; // CLI
    sector3[1] = 0xF4; // HLT
    let sector4 = vec![0x44u8; 128];

    let sectors: &[(u8, &[u8])] = &[
        (1, &boot_sector),
        (2, &sector2),
        (3, &sector3),
        (4, &sector4),
    ];
    let tracks = &[(0, 0, sectors)];
    let d88 = super::build_2hd_d88(tracks, false);
    FloppyImage::from_d88_bytes(&d88).expect("2HD 128-byte INT 1Bh boot disk")
}

fn make_2hd_128_byte_multi_sector_halt_boot_disk() -> FloppyImage {
    // Sector 1: CLI + HLT prologue so the bootstrap target halts immediately,
    // followed by a marker byte (0x11) we can locate at 0x1FC03.
    let mut boot_sector = vec![0u8; 128];
    boot_sector[0] = 0xFA; // CLI
    boot_sector[1] = 0xF4; // HLT
    boot_sector[2] = 0xEB; // JMP $-2 fallback (covers spurious resume).
    boot_sector[3] = 0xFE;
    boot_sector[4] = 0x11; // sector 1 marker

    // Sectors 2..=8 are filled with distinct per-sector marker bytes so we
    // can prove the bootstrap loaded each of them into the IPL load buffer
    // (0x1FC00..=0x1FFFF).
    let sector_markers: [u8; 7] = [0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
    let mut sector_data: Vec<Vec<u8>> = Vec::with_capacity(sector_markers.len());
    for &marker in &sector_markers {
        sector_data.push(vec![marker; 128]);
    }

    let mut sectors: Vec<(u8, &[u8])> = Vec::with_capacity(8);
    sectors.push((1, boot_sector.as_slice()));
    for (offset, data) in sector_data.iter().enumerate() {
        sectors.push(((offset + 2) as u8, data.as_slice()));
    }

    let tracks = &[(0, 0, sectors.as_slice())];
    let d88 = super::build_2hd_d88(tracks, false);
    FloppyImage::from_d88_bytes(&d88).expect("2HD multi-sector halt boot disk")
}

#[test]
fn hle_bootstrap_2hd_loads_full_ipl_block_not_only_first_sector() {
    let mut machine = super::create_machine_vx();
    machine
        .bus
        .insert_floppy(0, make_2hd_128_byte_multi_sector_halt_boot_disk(), None);

    const MAX_CYCLES: u64 = 50_000_000;
    const CHECK_INTERVAL: u64 = 200_000;

    let mut total_cycles = 0u64;
    while !machine.cpu.halted() {
        total_cycles += machine.run_for(CHECK_INTERVAL);
        assert!(
            total_cycles < MAX_CYCLES,
            "Machine did not halt within {MAX_CYCLES} cycles"
        );
    }

    let state = machine.save_state();
    assert_eq!(state.memory.ram[0x0584], 0x90, "2HD boot device");
    assert_eq!(
        state.memory.ram[0x1FC00], 0xFA,
        "sector 1 CLI loaded at IPL base"
    );
    assert_eq!(state.memory.ram[0x1FC04], 0x11, "sector 1 marker loaded");

    // The bootstrap must load up to 0x400 bytes (sectors 1..=8 for N=0)
    // even though the boot sector itself only consumes the first 128.
    let expected_markers: [(u32, u8); 7] = [
        (0x1FC80, 0x22), // sector 2
        (0x1FD00, 0x33), // sector 3
        (0x1FD80, 0x44), // sector 4
        (0x1FE00, 0x55), // sector 5
        (0x1FE80, 0x66), // sector 6
        (0x1FF00, 0x77), // sector 7
        (0x1FF80, 0x88), // sector 8
    ];
    for (address, marker) in expected_markers {
        assert_eq!(
            state.memory.ram[address as usize], marker,
            "expected sector marker {marker:#04X} at {address:#07X}"
        );
    }
}

#[test]
fn hle_bootstrap_pc9801f_2hd_128_byte_boot_sector_can_read_with_da90h() {
    let mut machine = super::create_machine_f();
    machine
        .bus
        .insert_floppy(0, make_2hd_128_byte_sector_int1b_boot_disk(), None);

    const MAX_CYCLES: u64 = 500_000_000;
    const CHECK_INTERVAL: u64 = 1_000_000;

    let mut total_cycles = 0u64;
    loop {
        total_cycles += machine.run_for(CHECK_INTERVAL);
        if machine.cpu.halted() {
            break;
        }
        assert!(
            total_cycles < MAX_CYCLES,
            "Machine did not halt within {MAX_CYCLES} cycles"
        );
    }

    let state = machine.save_state();
    assert_eq!(state.memory.ram[0x0584], 0x90, "2HD boot device");
    assert_eq!(machine.cpu.cs(), 0x0060, "stage-two CS");
    assert_eq!(state.memory.ram[0x00600], 0xFA, "sector 1 copied");
    assert_eq!(state.memory.ram[0x00680], 0x22, "sector 2 copied");
    assert_eq!(state.memory.ram[0x00700], 0xFA, "sector 3 copied");
    assert_eq!(state.memory.ram[0x00701], 0xF4, "stage-two HLT copied");
    assert_eq!(state.memory.ram[0x00780], 0x44, "sector 4 copied");
}
