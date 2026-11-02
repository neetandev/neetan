//! The HLE OS ROM of `utils/dos_towns` under a synthetic `IO.SYS`: the
//! `MSDOS.SYS` entry, the OS ROM drive, `COMMAND.COM` and the resident
//! installation of `MSCDEX.EXE` through EXEC.

use common::Cpu as _;

use crate::{
    harness::GuestMachine,
    kernel_block_device::read_file,
    kernel_harness::{
        KERNEL_SEGMENT, LIST_OF_LISTS_OFFSET, OS_ROM_IMAGE, SYSINIT_SEGMENT, booted_kernel,
        exec_command_com, iso_image, kernel_copy, kernel_machine, linear, ramdisk_image, run,
        run_dosinit, write_exec_arguments,
    },
};

/// Command tail of the MSCDEX call of `IO.SYS`.
const IO_SYS_TAIL: &[u8] = b" /L:Q /D:TOWNS_CD /D:MENU_DRV /M:8";

/// Offset of the EXEC path in the program segment.
const EXEC_PATH: u16 = 0x0280;

/// Offset of the `INT 21h` entry fragment that CTL.SYS searches.
const CTL_PATTERN_OFFSET: usize = 0x11D2;

/// Bytes of the CTL.SYS fragment up to the last operand.
const CTL_FRAGMENT_SIZE: usize = 0x1F;

/// Distances of the four SDA operands from the CTL.SYS pattern.
const CTL_OPERAND_DISTANCES: [u16; 4] = [0x14, 0x17, 0x1A, 0x1D];

/// SDA offsets that CTL.SYS reads from the operands: user SP, NSP, user SS
/// and NSS.
const CTL_OPERANDS: [u16; 4] = [0x051E, 0x0584, 0x0520, 0x0582];

/// Offset of the DOSINIT entry code of the stub.
const ENTRY_OFFSET: usize = 0x0B80;

/// Offset of the OEM handler pointer of the image header.
const OEM_HANDLER_OFFSET: usize = 0x0014;

/// Offset of the word that MENU_DRV of `IO.SYS` 1989-12-16 changes.
const MENU_DRV_WORD: u16 = 0x425E;

/// First byte of the `MSCDEX.EXE` stub (`PUSHF`).
const PUSHF_OPCODE: u8 = 0x9C;

/// Paragraphs of a PSP.
const PSP_PARAGRAPHS: u16 = 0x10;

/// Drive index of Q:.
const DRIVE_Q: u16 = 16;

/// Returns a machine after DOSINIT with a game CD and the MENU_DRV volume.
fn machine() -> GuestMachine {
    booted_kernel(
        Some(iso_image(b"GAME", &[(b"README.TXT", b"game disc")])),
        &iso_image(b"MENU", &[(b"RUN386.EXE", b"menu volume")]),
        &ramdisk_image(&[]),
    )
}

/// Returns the MCB chain from the first MCB as (segment, owner, size).
fn memory_blocks(machine: &mut GuestMachine) -> Vec<(u16, u16, u16)> {
    let mut segment = machine.read_word(linear(KERNEL_SEGMENT, LIST_OF_LISTS_OFFSET - 2));
    let mut blocks = Vec::new();
    loop {
        let mcb = linear(segment, 0);
        let signature = machine.read_byte(mcb);
        assert!(signature == b'M' || signature == b'Z', "MCB chain broken");
        let owner = machine.read_word(mcb + 1);
        let size = machine.read_word(mcb + 3);
        blocks.push((segment, owner, size));
        if signature == b'Z' {
            return blocks;
        }
        segment += size + 1;
    }
}

/// Runs `C:\MSCDEX.EXE` with `tail` through `AX=4B00h` and returns the
/// carry flag of EXEC and the exit word of `AH=4Dh`.
fn exec_mscdex(machine: &mut GuestMachine, tail: &[u8]) -> (bool, u16) {
    write_exec_arguments(machine, tail);
    machine.write_bytes(linear(SYSINIT_SEGMENT, EXEC_PATH), b"C:\\MSCDEX.EXE\0");
    let mut code = exec_command_com();
    code.pop();
    code.extend_from_slice(&[
        0x9C, // PUSHF
        0x5D, // POP BP
        0xB4, 0x4D, // MOV AH, 4Dh
        0xCD, 0x21, // INT 21h
        0xF4, // HLT
    ]);
    run(machine, &code);
    (machine.cpu.bp() & 1 != 0, machine.cpu.ax())
}

/// Runs `INT 2Fh AX=1500h` and returns `BX` and `CX`.
fn drive_count_and_first_drive(machine: &mut GuestMachine) -> (u16, u16) {
    // MOV AX,1500h; XOR BX,BX; INT 2Fh; HLT
    run(machine, &[0xB8, 0x00, 0x15, 0x31, 0xDB, 0xCD, 0x2F, 0xF4]);
    (machine.cpu.bx(), machine.cpu.cx())
}

#[test]
fn kernel_of_the_os_rom_returns_to_sysinit_with_the_contract_registers() {
    let mut machine = kernel_machine(None, &iso_image(b"MENU", &[]), &ramdisk_image(&[]));
    let result = run_dosinit(&mut machine);
    assert_eq!(
        (result.es, result.di),
        (KERNEL_SEGMENT, LIST_OF_LISTS_OFFSET)
    );
    assert_eq!(result.ds, result.dx);
    let first_mcb = machine.read_word(linear(KERNEL_SEGMENT, LIST_OF_LISTS_OFFSET - 2));
    assert_eq!(result.ds, first_mcb + 1);
    assert_eq!((result.ss, result.sp), (SYSINIT_SEGMENT, 0xFFFE));
    assert_ne!(result.flags & 0x0200, 0, "interrupts stay enabled");
}

/// Asserts that the kernel segment still holds the header, the entry code
/// and the CTL.SYS fragment of the image, and that the MENU_DRV word is zero.
fn assert_image_code_intact(machine: &mut GuestMachine) {
    let image = kernel_copy();
    let kernel = linear(KERNEL_SEGMENT, 0) as usize;
    let fragment_end = CTL_PATTERN_OFFSET + CTL_FRAGMENT_SIZE;
    for range in [
        0..3,
        OEM_HANDLER_OFFSET..OEM_HANDLER_OFFSET + 4,
        ENTRY_OFFSET..fragment_end,
    ] {
        assert_eq!(
            machine.bus.ram[kernel + range.start..kernel + range.end],
            image[range.clone()],
            "kernel bytes {range:X?}"
        );
    }
    assert_eq!(
        machine.bus.ram[kernel + CTL_PATTERN_OFFSET..kernel + CTL_PATTERN_OFFSET + 8],
        [0x8C, 0xCB, 0x8E, 0xDB, 0xFE, 0x06, 0xCF, 0x02]
    );
    let operands = CTL_OPERAND_DISTANCES.map(|distance| {
        machine.read_word(linear(KERNEL_SEGMENT, CTL_PATTERN_OFFSET as u16 + distance))
    });
    assert_eq!(operands, CTL_OPERANDS);
    assert_eq!(machine.read_word(linear(KERNEL_SEGMENT, MENU_DRV_WORD)), 0);
}

#[test]
fn kernel_entry_and_ctl_sys_pattern_survive_dosinit() {
    let mut machine = kernel_machine(None, &iso_image(b"MENU", &[]), &ramdisk_image(&[]));
    run_dosinit(&mut machine);
    assert_image_code_intact(&mut machine);
}

#[test]
fn kernel_entry_and_ctl_sys_pattern_survive_the_mscdex_installation() {
    let mut machine = machine();
    assert_eq!(exec_mscdex(&mut machine, IO_SYS_TAIL), (false, 0x0300));
    assert_eq!(read_file(&mut machine, b"R:\\RUN386.EXE"), b"menu volume");
    assert_image_code_intact(&mut machine);
}

#[test]
fn os_rom_drive_serves_the_files_of_the_image() {
    let mut machine = machine();
    assert_eq!(
        read_file(&mut machine, b"C:\\COMMAND.COM"),
        include_bytes!("../../../../utils/dos/dos.rom")
    );
    let mscdex = read_file(&mut machine, b"C:\\MSCDEX.EXE");
    assert_eq!(mscdex.first(), Some(&PUSHF_OPCODE));
    assert_eq!(OS_ROM_IMAGE.len(), 0x80000);
}

#[test]
fn mscdex_exe_stays_resident_with_its_psp_only() {
    let mut machine = machine();
    let blocks_before = memory_blocks(&mut machine);
    let (carry, exit) = exec_mscdex(&mut machine, IO_SYS_TAIL);
    assert!(!carry, "EXEC of C:\\MSCDEX.EXE succeeds");
    assert_eq!(exit, 0x0300, "terminate and stay resident with code 00h");
    assert_eq!(drive_count_and_first_drive(&mut machine), (2, DRIVE_Q));

    let blocks = memory_blocks(&mut machine);
    let first_mcb = blocks_before[0].0;
    assert_eq!(blocks.len(), 2, "{blocks:X?}");
    let (psp_mcb, psp, size) = blocks[0];
    assert_eq!(psp_mcb, first_mcb, "a child of SYSINIT gets no environment");
    assert_eq!(psp, psp_mcb + 1);
    assert_eq!(size, PSP_PARAGRAPHS, "only the PSP stays resident");
    assert_eq!(machine.read_word(linear(psp, 0x2C)), 0);
    assert_eq!(blocks[1].1, 0);
}

#[test]
fn second_mscdex_exe_ends_with_exit_code_5() {
    let mut machine = machine();
    assert_eq!(exec_mscdex(&mut machine, IO_SYS_TAIL), (false, 0x0300));
    let blocks = memory_blocks(&mut machine);
    assert_eq!(
        exec_mscdex(&mut machine, b" /D:TOWNS_CD /M:8 /L:Q"),
        (false, 0x0005)
    );
    assert_eq!(memory_blocks(&mut machine), blocks);
    assert_eq!(drive_count_and_first_drive(&mut machine), (2, DRIVE_Q));
}

#[test]
fn mscdex_exe_without_a_found_device_ends_with_exit_code_1() {
    let mut machine = machine();
    let blocks = memory_blocks(&mut machine);
    assert_eq!(
        exec_mscdex(&mut machine, b" /L:Q /D:NOCDROM"),
        (false, 0x0001)
    );
    assert_eq!(memory_blocks(&mut machine), blocks);
}
