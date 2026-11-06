use common::{Bus, Cpu, JisChar};
use device::{
    disk::{HddFormat, HddGeometry, HddImage},
    floppy::FloppyImage,
};
use machine_98::{BootDevice, Pc98InspectionState, Pc98Machine};

use super::{
    build_2hd_d88, create_machine_f, create_machine_pc9821as, create_machine_ra, create_machine_vm,
    create_machine_vx, make_halt_boot_disk, make_halt_boot_ide_hdd,
};

/// BDA byte DISK_BOOT with the DA/UA of the boot device.
const BDA_BOOT_DEVICE: u32 = 0x0584;
/// Segment of the IPL.
const IPL_SEGMENT: u16 = 0x1FC0;
/// Segment of the BIOS code in the HLE ROM.
const BIOS_CODE_SEGMENT: u16 = 0xFD80;
/// FLAGS interrupt enable bit.
const FLAGS_INTERRUPT: u16 = 0x0200;
/// Opcode of CALL FAR ptr16:16.
const FAR_CALL_OPCODE: u8 = 0x9A;
/// Linear address of the text VRAM character plane.
const TEXT_VRAM: u32 = 0xA0000;
/// Text cells in one row.
const TEXT_COLUMNS: u32 = 80;
/// Text rows of the screen.
const TEXT_ROWS: u32 = 25;
/// Linear address of the text VRAM attribute plane.
const TEXT_ATTRIBUTE_VRAM: u32 = 0xA2000;
/// Code plane bytes of a cell cleared to a space.
const CLEARED_CODE: [u8; 2] = [0x20, 0x00];
/// Attribute of a cleared cell: white.
const CLEARED_ATTRIBUTE: u8 = 0xE1;
/// BDA byte CRT_STS_FLAG with the CRT mode of INT 18h AH=0Ah.
const CRT_STATUS: u32 = 0x053C;
/// CRT_STS_FLAG bits with the AH=0Ah mode.
const CRT_STATUS_MODE_BITS: u8 = 0x0F;
/// AH=0Ah mode of 80x25 text with the simple graphics attribute.
const CRT_MODE_SIMPLE_GRAPHICS: u8 = 0x04;

/// Text VRAM code of the HLE DOS prompt character `>`.
const DOS_PROMPT: u16 = 0x003E;

/// Offset after the CLI and HLT of the halt IPL.
const HALT_IPL_END: usize = 0x0002;

/// Opcode of RETF.
const RETF_OPCODE: u8 = 0xCB;

/// Opcode of IRET.
const IRET_OPCODE: u8 = 0xCF;

/// Text attribute the returning IPL writes into the attribute plane.
const MARKER_ATTRIBUTE: u8 = 0x05;

/// CRT mode the returning IPL sets with INT 18h AH=0Ah: 80x20, vertical line.
const MARKER_CRT_MODE: u8 = 0x01;

/// IPL body that turns the text display off, sets the CRT mode `MARKER_CRT_MODE`
/// and writes the character `A` with `MARKER_ATTRIBUTE` into the top left cell and
/// into the right cell of the first message character. The return opcode follows.
#[rustfmt::skip]
const RETURNING_IPL_BODY: &[u8] = &[
    0xB4, 0x0D,                                     // MOV AH, 0Dh
    0xCD, 0x18,                                     // INT 18h
    0xB8, MARKER_CRT_MODE, 0x0A,                    // MOV AX, 0A01h
    0xCD, 0x18,                                     // INT 18h
    0xB8, 0x00, 0xA0,                               // MOV AX, A000h
    0x8E, 0xC0,                                     // MOV ES, AX
    0x26, 0xC7, 0x06, 0x00, 0x00, 0x41, 0x00,       // MOV WORD ES:[0000h], 0041h
    0x26, 0xC6, 0x06, 0x00, 0x20, MARKER_ATTRIBUTE, // MOV BYTE ES:[2000h], 05h
    0x26, 0xC7, 0x06, 0xAE, 0x07, 0x41, 0x00,       // MOV WORD ES:[07AEh], 0041h
    0x26, 0xC6, 0x06, 0xAE, 0x27, MARKER_ATTRIBUTE, // MOV BYTE ES:[27AEh], 05h
];

/// Builds the returning IPL that ends with `return_opcode`.
fn make_returning_ipl(return_opcode: u8) -> Vec<u8> {
    let mut ipl = RETURNING_IPL_BODY.to_vec();
    ipl.push(return_opcode);
    ipl
}

/// Address where the entry state probe IPL stores FLAGS, SP, SS and the two
/// words at SS:SP.
const ENTRY_RECORD: u32 = 0x1000;

/// IPL that records its entry state at 0000:1000h and halts.
#[rustfmt::skip]
const ENTRY_PROBE_IPL: &[u8] = &[
    0x9C,                   // PUSHF
    0x58,                   // POP AX
    0x31, 0xDB,             // XOR BX, BX
    0x8E, 0xDB,             // MOV DS, BX
    0xA3, 0x00, 0x10,       // MOV [1000h], AX
    0x89, 0x26, 0x02, 0x10, // MOV [1002h], SP
    0x8C, 0x16, 0x04, 0x10, // MOV [1004h], SS
    0x89, 0xE5,             // MOV BP, SP
    0x8B, 0x46, 0x00,       // MOV AX, [BP+0]
    0xA3, 0x06, 0x10,       // MOV [1006h], AX
    0x8B, 0x46, 0x02,       // MOV AX, [BP+2]
    0xA3, 0x08, 0x10,       // MOV [1008h], AX
    0xFA,                   // CLI
    0xF4,                   // HLT
];

/// Text row of the system disk message.
const MESSAGE_ROW: u32 = 12;

/// Text column of the system disk message.
const MESSAGE_COLUMN: u32 = 22;

/// JIS codes of the system disk message: システムディスクをセットしてください
const SYSTEM_DISK_MESSAGE: [u16; 18] = [
    0x2537, // シ
    0x2539, // ス
    0x2546, // テ
    0x2560, // ム
    0x2547, // デ
    0x2523, // ィ
    0x2539, // ス
    0x252F, // ク
    0x2472, // を
    0x253B, // セ
    0x2543, // ッ
    0x2548, // ト
    0x2437, // し
    0x2446, // て
    0x242F, // く
    0x2440, // だ
    0x2435, // さ
    0x2424, // い
];

/// Builds a 2HD floppy whose boot sector starts with `ipl`.
fn make_boot_disk(ipl: &[u8]) -> FloppyImage {
    let mut boot_sector = vec![0u8; 1024];
    boot_sector[..ipl.len()].copy_from_slice(ipl);
    let sectors: &[(u8, &[u8])] = &[(1, &boot_sector)];
    let tracks = &[(0, 0, sectors)];
    let d88 = build_2hd_d88(tracks, false);
    FloppyImage::from_d88_bytes(&d88).expect("boot disk")
}

/// Builds an IDE hard disk whose sector 0 starts with `ipl`.
fn make_ide_hdd(ipl: &[u8]) -> HddImage {
    let geometry = HddGeometry {
        cylinders: 20,
        heads: 4,
        sectors_per_track: 17,
        sector_size: 512,
    };
    let mut data = vec![0u8; geometry.total_bytes() as usize];
    data[..ipl.len()].copy_from_slice(ipl);
    HddImage::from_raw(geometry, HddFormat::Hdi, data)
}

/// Runs the machine until the CPU halts.
fn run_until_halted<C: Cpu>(machine: &mut Pc98Machine<C>, label: &str) {
    const MAX_CYCLES: u64 = 500_000_000;
    const CHECK_INTERVAL: u64 = 1_000_000;

    let mut total_cycles = 0u64;
    loop {
        total_cycles += machine.run_for(CHECK_INTERVAL);
        if machine.cpu.halted() {
            return;
        }
        assert!(
            total_cycles < MAX_CYCLES,
            "{label}: machine did not halt within {MAX_CYCLES} cycles"
        );
    }
}

/// Reads a little endian word from the bus.
fn read_word<C: Cpu>(machine: &mut Pc98Machine<C>, address: u32) -> u16 {
    u16::from(machine.bus.read_byte(address)) | (u16::from(machine.bus.read_byte(address + 1)) << 8)
}

/// Runs until the halt and checks that the IPL of `boot_device` halted at `halt_offset`.
fn assert_halted_in_ipl<C: Cpu>(
    machine: &mut Pc98Machine<C>,
    halt_offset: usize,
    boot_device: u8,
    label: &str,
) {
    run_until_halted(machine, label);
    assert_eq!(machine.cpu.cs(), IPL_SEGMENT, "{label}: CS of the halt");
    assert_eq!(
        usize::from(machine.cpu.ip()),
        halt_offset,
        "{label}: IP after the HLT of the IPL"
    );
    assert_eq!(
        machine.bus.read_byte(BDA_BOOT_DEVICE),
        boot_device,
        "{label}: BDA boot device"
    );
}

/// Runs until the halt and checks the halt with IF=0, the CRT mode, the cleared
/// screen and the system disk message.
fn assert_system_disk_message<C: Cpu>(machine: &mut Pc98Machine<C>, label: &str) {
    run_until_halted(machine, label);
    assert_eq!(
        machine.cpu.cs(),
        BIOS_CODE_SEGMENT,
        "{label}: CS of the halt"
    );
    assert_eq!(
        machine.cpu.flags() & FLAGS_INTERRUPT,
        0,
        "{label}: the halt keeps interrupts disabled"
    );
    machine.run_for(1_000_000);
    assert!(machine.cpu.halted(), "{label}: the CPU stays halted");
    assert_eq!(
        machine.cpu.cs(),
        BIOS_CODE_SEGMENT,
        "{label}: CS stays in the BIOS"
    );

    assert_eq!(
        machine.bus.read_byte(CRT_STATUS) & CRT_STATUS_MODE_BITS,
        CRT_MODE_SIMPLE_GRAPHICS,
        "{label}: CRT mode 80x25 with simple graphics"
    );

    let first_cell = MESSAGE_ROW * TEXT_COLUMNS + MESSAGE_COLUMN;
    let message_cells = first_cell..first_cell + SYSTEM_DISK_MESSAGE.len() as u32 * 2;
    for cell in 0..TEXT_COLUMNS * TEXT_ROWS {
        let code_address = TEXT_VRAM + cell * 2;
        let code = [
            machine.bus.read_byte(code_address),
            machine.bus.read_byte(code_address + 1),
        ];
        if message_cells.contains(&cell) {
            let index = ((cell - first_cell) / 2) as usize;
            if (cell - first_cell).is_multiple_of(2) {
                let (row_code, column_code) =
                    JisChar::from_u16(SYSTEM_DISK_MESSAGE[index]).to_vram_bytes();
                assert_eq!(
                    code,
                    [row_code, column_code],
                    "{label}: message character {index}"
                );
            } else {
                assert_eq!(
                    code, CLEARED_CODE,
                    "{label}: right cell of message character {index} holds a space"
                );
            }
        } else {
            assert_eq!(code, CLEARED_CODE, "{label}: cell {cell} holds a space");
        }
        assert_eq!(
            machine.bus.read_byte(TEXT_ATTRIBUTE_VRAM + cell * 2),
            CLEARED_ATTRIBUTE,
            "{label}: attribute of cell {cell}"
        );
    }
}

/// Checks that the text display is on.
fn assert_text_display_on(state: &Pc98InspectionState, label: &str) {
    assert!(
        state.gdc_master.display_enabled,
        "{label}: text display is on"
    );
}

/// Returns `true` when a text cell of the screen holds `code`.
fn text_screen_holds<C: Cpu>(machine: &mut Pc98Machine<C>, code: u16) -> bool {
    (0..TEXT_COLUMNS * TEXT_ROWS).any(|cell| read_word(machine, TEXT_VRAM + cell * 2) == code)
}

/// Runs until the HLE DOS prompt appears.
fn assert_hle_dos_prompt<C: Cpu>(machine: &mut Pc98Machine<C>, label: &str) {
    const MAX_CYCLES: u64 = 500_000_000;
    const CHECK_INTERVAL: u64 = 1_000_000;

    let mut total_cycles = 0u64;
    while !text_screen_holds(machine, DOS_PROMPT) {
        total_cycles += machine.run_for(CHECK_INTERVAL);
        assert!(
            total_cycles < MAX_CYCLES,
            "{label}: HLE DOS did not show its prompt within {MAX_CYCLES} cycles"
        );
    }
}

/// Boots the entry probe IPL and checks IF, SS:SP and the far return address at its entry.
fn assert_ipl_entry_state<C: Cpu>(mut machine: Pc98Machine<C>, label: &str) {
    machine
        .bus
        .insert_floppy(0, make_boot_disk(ENTRY_PROBE_IPL), None);
    assert_halted_in_ipl(&mut machine, ENTRY_PROBE_IPL.len(), 0x90, label);

    let flags = read_word(&mut machine, ENTRY_RECORD);
    let stack_pointer = read_word(&mut machine, ENTRY_RECORD + 2);
    let stack_segment = read_word(&mut machine, ENTRY_RECORD + 4);
    let return_offset = read_word(&mut machine, ENTRY_RECORD + 6);
    let return_segment = read_word(&mut machine, ENTRY_RECORD + 8);

    assert_ne!(flags & FLAGS_INTERRUPT, 0, "{label}: IF at IPL entry");
    assert_eq!(stack_segment, 0x0000, "{label}: SS at IPL entry");
    assert_eq!(stack_pointer, 0x7C02, "{label}: SP at IPL entry");
    assert_eq!(return_segment, BIOS_CODE_SEGMENT, "{label}: return segment");

    let call_address = (u32::from(return_segment) << 4) + u32::from(return_offset) - 5;
    assert_eq!(
        machine.bus.read_byte(call_address),
        FAR_CALL_OPCODE,
        "{label}: return address follows a far call"
    );
    assert_eq!(
        read_word(&mut machine, call_address + 1),
        0x0000,
        "{label}: far call offset"
    );
    assert_eq!(
        read_word(&mut machine, call_address + 3),
        IPL_SEGMENT,
        "{label}: far call segment"
    );
}

#[test]
fn ipl_entry_has_interrupts_enabled_and_a_far_return_address() {
    assert_ipl_entry_state(create_machine_f(), "F");
    assert_ipl_entry_state(create_machine_vm(), "VM");
    assert_ipl_entry_state(create_machine_vx(), "VX");
    assert_ipl_entry_state(create_machine_ra(), "RA");
    assert_ipl_entry_state(create_machine_pc9821as(), "9821AS");
}

#[test]
fn working_ipl_boots_and_runs_its_own_code() {
    let mut machine = create_machine_vm();
    machine.bus.insert_floppy(0, make_halt_boot_disk(), None);
    assert_halted_in_ipl(&mut machine, HALT_IPL_END, 0x90, "VM floppy");
    assert_eq!(machine.bus.read_byte(0x1FC00), 0xFA, "IPL byte 0 (CLI)");
    assert_eq!(machine.bus.read_byte(0x1FC01), 0xF4, "IPL byte 1 (HLT)");

    let mut machine = create_machine_pc9821as();
    machine.bus.insert_hdd(0, make_halt_boot_ide_hdd(), None);
    assert_halted_in_ipl(&mut machine, HALT_IPL_END, 0x80, "9821AS IDE");
}

/// Checks the message and the halt for a returning IPL on every model.
fn assert_returning_ipl_shows_the_message(return_opcode: u8, kind: &str) {
    let ipl = make_returning_ipl(return_opcode);

    let label = format!("F {kind}");
    let mut machine = create_machine_f();
    machine.bus.insert_floppy(0, make_boot_disk(&ipl), None);
    assert_system_disk_message(&mut machine, &label);
    assert_text_display_on(&machine.inspection_state(), &label);

    let label = format!("VM {kind}");
    let mut machine = create_machine_vm();
    machine.bus.insert_floppy(0, make_boot_disk(&ipl), None);
    assert_system_disk_message(&mut machine, &label);
    assert_text_display_on(&machine.inspection_state(), &label);

    let label = format!("VX {kind}");
    let mut machine = create_machine_vx();
    machine.bus.insert_floppy(0, make_boot_disk(&ipl), None);
    assert_system_disk_message(&mut machine, &label);
    assert_text_display_on(&machine.inspection_state(), &label);

    let label = format!("RA {kind}");
    let mut machine = create_machine_ra();
    machine.bus.insert_floppy(0, make_boot_disk(&ipl), None);
    assert_system_disk_message(&mut machine, &label);
    assert_text_display_on(&machine.inspection_state(), &label);

    let label = format!("9821AS IDE {kind}");
    let mut machine = create_machine_pc9821as();
    machine.bus.insert_hdd(0, make_ide_hdd(&ipl), None);
    assert_system_disk_message(&mut machine, &label);
    assert_text_display_on(&machine.inspection_state(), &label);
}

#[test]
fn ipl_retf_shows_the_system_disk_message() {
    assert_returning_ipl_shows_the_message(RETF_OPCODE, "RETF");
}

#[test]
fn ipl_iret_shows_the_system_disk_message() {
    assert_returning_ipl_shows_the_message(IRET_OPCODE, "IRET");
}

#[test]
fn ipl_retf_does_not_boot_the_next_device() {
    let mut machine = create_machine_vm();
    machine
        .bus
        .insert_floppy(0, make_boot_disk(&make_returning_ipl(RETF_OPCODE)), None);
    machine.bus.insert_floppy(1, make_halt_boot_disk(), None);
    assert_system_disk_message(&mut machine, "VM two floppies");
    assert_text_display_on(&machine.inspection_state(), "VM two floppies");
}

#[test]
fn no_ipl_boots_hle_dos() {
    let mut machine = create_machine_ra();
    assert_hle_dos_prompt(&mut machine, "RA auto");

    let mut machine = create_machine_vm();
    machine.bus.set_boot_device(BootDevice::Fdd1);
    assert_hle_dos_prompt(&mut machine, "VM fdd1");

    let mut machine = create_machine_pc9821as();
    machine.bus.set_boot_device(BootDevice::Hdd1);
    assert_hle_dos_prompt(&mut machine, "9821AS hdd1");

    let mut machine = create_machine_vx();
    machine.bus.insert_floppy(0, make_halt_boot_disk(), None);
    machine.bus.set_boot_device(BootDevice::Dos);
    assert_hle_dos_prompt(&mut machine, "VX dos");

    let mut machine = create_machine_f();
    machine.bus.insert_floppy(0, make_halt_boot_disk(), None);
    machine.bus.set_boot_device(BootDevice::Fdd2);
    assert_hle_dos_prompt(&mut machine, "F fdd2");

    let mut machine = create_machine_pc9821as();
    machine.bus.insert_hdd(0, make_halt_boot_ide_hdd(), None);
    machine.bus.set_boot_device(BootDevice::Hdd2);
    assert_hle_dos_prompt(&mut machine, "9821AS hdd2");
}
