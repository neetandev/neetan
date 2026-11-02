//! The kernel entry under a synthetic `IO.SYS`: the drivers of
//! `kernel_drivers.asm` with the HLE OS ROM of `utils/dos_towns` as the
//! OS ROM drive, its `MSDOS.SYS` loaded like `IO.SYS` loads it, a SYSINIT
//! program that far-calls it and the CDS array that `IO.SYS` builds after
//! DOSINIT.

use common::Cpu as _;
use dos::{DosPlacement, DosPlatform};

use crate::harness::{GuestMachine, TestDrives};

/// Image of the synthetic `IO.SYS` drivers.
const KERNEL_DRIVERS: &[u8] = include_bytes!("kernel_drivers.bin");

/// Segment of the drivers.
pub const IOSYS_SEGMENT: u16 = 0x0A00;
/// Kernel segment (the DOS data segment).
pub const KERNEL_SEGMENT: u16 = 0x1000;
/// Top of conventional memory that SYSINIT passes in `DX`.
pub const MEMORY_TOP: u16 = 0x8000;
/// Segment of the ISO 9660 image of MENU_DRV.
pub const MENU_ISO_SEGMENT: u16 = 0x9000;
/// Segment of the RAM disk.
pub const RAMDISK_SEGMENT: u16 = 0xA000;
/// Segment of the SYSINIT program and of the test programs.
pub const SYSINIT_SEGMENT: u16 = 0xB000;
/// Segment of the CDS array of the test.
pub const CDS_SEGMENT: u16 = 0xC000;

/// Offset of the CON header.
pub const CON_HEADER: u16 = 0x0000;
/// Offset of the AUX header.
pub const AUX_HEADER: u16 = 0x0012;
/// Offset of the TOWNS_CD header.
pub const TOWNS_CD_HEADER: u16 = 0x0024;
/// Offset of the MENU_DRV header.
pub const MENU_HEADER: u16 = 0x003A;
/// Offset of the CLOCK header.
pub const CLOCK_HEADER: u16 = 0x0050;
/// Offset of the IO.SYS block driver header.
pub const IOSYS_BLOCK_HEADER: u16 = 0x0062;
/// Offset of the RAM disk header.
pub const RAMDISK_HEADER: u16 = 0x0074;
/// Offset of the output count word.
pub const OUTPUT_COUNT: u16 = 0x0104;
/// Offset of the input count word.
pub const INPUT_COUNT: u16 = 0x0106;
/// Offset of the input position word.
pub const INPUT_POSITION: u16 = 0x0108;
/// Offset of the media check result byte of the RAM disk.
pub const MEDIA_CHANGE: u16 = 0x010E;
/// Offset of the write fault switch of the RAM disk.
pub const WRITE_FAULT: u16 = 0x010F;
/// Offset of the command log count word.
pub const COMMAND_LOG_COUNT: u16 = 0x0110;
/// Offset of the CLOCK record.
pub const CLOCK_RECORD: u16 = 0x0112;
/// Offset of the RAM disk write count word.
pub const WRITE_COUNT: u16 = 0x0118;
/// Offset of the answer of MENU_DRV to the media changed query.
pub const MENU_MEDIA_CHANGE: u16 = 0x011A;
/// Offset of the count of MENU_DRV read longs that fail with not ready.
pub const MENU_NOT_READY: u16 = 0x011B;
/// Offset of the command code that the RAM disk fails, 0 for none.
const FAIL_COMMAND: u16 = 0x07F0;
/// Offset of the count of matching RAM disk requests that pass first.
const FAIL_SKIP: u16 = 0x07F1;
/// Offset of the count of matching RAM disk requests that fail.
const FAIL_COUNT: u16 = 0x07F2;
/// Offset of the driver error of a failing RAM disk request.
const FAIL_ERROR: u16 = 0x07F3;
/// Offset of the sector that a failing read or write covers.
const FAIL_SECTOR: u16 = 0x07F4;
/// Value of `FAIL_SECTOR` that matches every request.
pub const ANY_SECTOR: u16 = 0xFFFF;
/// Offset of the command log.
const COMMAND_LOG: u16 = 0x0120;
/// Offset of the device log.
const DEVICE_LOG: u16 = 0x01A0;
/// Size of the command and device log rings.
const LOG_SIZE: usize = 0x80;
/// Offset of the input buffer.
const INPUT_BUFFER: u16 = 0x0220;
/// Offset of the output log.
const OUTPUT_LOG: u16 = 0x0240;
/// Size of the output log ring.
const OUTPUT_LOG_SIZE: usize = 0x200;
/// Offset of the INT 29h handler.
const INT29_ENTRY: u16 = 0x0440;

/// Device tag of CON in the device log.
pub const TAG_CON: u8 = 1;
/// Device tag of AUX.
pub const TAG_AUX: u8 = 2;
/// Device tag of TOWNS_CD.
pub const TAG_TOWNS_CD: u8 = 3;
/// Device tag of MENU_DRV.
pub const TAG_MENU_DRV: u8 = 4;
/// Device tag of CLOCK.
pub const TAG_CLOCK: u8 = 5;
/// Device tag of the IO.SYS block driver.
pub const TAG_IOSYS_BLOCK: u8 = 6;
/// Device tag of the RAM disk.
pub const TAG_RAMDISK: u8 = 7;

/// Offset of the list of lists in the DOS 3.10 layout.
pub const LIST_OF_LISTS_OFFSET: u16 = 0x0026;
/// The HLE OS ROM image of `utils/dos_towns`.
pub const OS_ROM_IMAGE: &[u8] = include_bytes!("../../../../utils/dos_towns/osrom.rom");
/// Offset of the kernel in the OS ROM (cluster `4Eh`).
const OS_ROM_KERNEL_OFFSET: usize = 0x14800;
/// Bytes that `IO.SYS` reads from the OS ROM into its buffer.
const KERNEL_READ_SIZE: usize = 0x7800;
/// Bytes that SYSINIT copies from the buffer to the kernel segment.
const KERNEL_COPY_SIZE: usize = 0xA000;
/// Content of the SYSINIT buffer behind the bytes read from the OS ROM.
const BUFFER_FILL: u8 = 0xCC;
/// Size of a 3.10 CDS entry.
pub const CDS_ENTRY_SIZE: u32 = 0x51;
/// Offset of the far pointer to the next DPB in a 3.10 DPB.
pub const DPB_NEXT: u32 = 0x18;

/// Registers at the return of DOSINIT, stored by the SYSINIT program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DosInitReturn {
    /// ES.
    pub es: u16,
    /// DI.
    pub di: u16,
    /// DS.
    pub ds: u16,
    /// DX.
    pub dx: u16,
    /// SP.
    pub sp: u16,
    /// SS.
    pub ss: u16,
    /// FLAGS.
    pub flags: u16,
    /// AX.
    pub ax: u16,
    /// BX.
    pub bx: u16,
    /// CX.
    pub cx: u16,
    /// SI.
    pub si: u16,
    /// BP.
    pub bp: u16,
}

/// Returns the FM Towns platform before the kernel entry supplies the
/// placement.
pub fn towns_platform() -> DosPlatform {
    DosPlatform::fm_towns(DosPlacement {
        dos_data_segment: 0,
        first_mcb_segment: 0,
        memory_top_segment: 0,
    })
}

/// Returns the linear address of `segment:offset`.
pub fn linear(segment: u16, offset: u16) -> u32 {
    (u32::from(segment) << 4) + u32::from(offset)
}

/// Returns the linear address of the list of lists.
pub fn list_of_lists() -> u32 {
    linear(KERNEL_SEGMENT, LIST_OF_LISTS_OFFSET)
}

/// Returns the linear address of a driver variable.
pub fn driver_address(offset: u16) -> u32 {
    linear(IOSYS_SEGMENT, offset)
}

/// Returns a machine with the synthetic `IO.SYS`, the kernel image, the
/// MENU_DRV image and the RAM disk in memory, before DOSINIT.
pub fn kernel_machine(cdrom: Option<Vec<u8>>, menu_iso: &[u8], ramdisk: &[u8]) -> GuestMachine {
    let mut machine = GuestMachine::bare(
        towns_platform(),
        TestDrives {
            cdrom,
            os_rom: Some(OS_ROM_IMAGE.to_vec()),
            floppy: None,
        },
    );
    machine.write_bytes(linear(IOSYS_SEGMENT, 0), KERNEL_DRIVERS);
    machine.write_bytes(linear(MENU_ISO_SEGMENT, 0), menu_iso);
    machine.write_bytes(linear(RAMDISK_SEGMENT, 0), ramdisk);

    machine.write_bytes(linear(KERNEL_SEGMENT, 0), &kernel_copy());
    machine.set_vector(0x29, IOSYS_SEGMENT, INT29_ENTRY);
    machine
}

/// Returns the bytes that SYSINIT copies to the kernel segment: the
/// `7800h` bytes that `IO.SYS` reads from the OS ROM and the rest of its
/// buffer.
pub fn kernel_copy() -> Vec<u8> {
    let mut buffer = vec![BUFFER_FILL; KERNEL_COPY_SIZE];
    buffer[..KERNEL_READ_SIZE].copy_from_slice(
        &OS_ROM_IMAGE[OS_ROM_KERNEL_OFFSET..OS_ROM_KERNEL_OFFSET + KERNEL_READ_SIZE],
    );
    buffer
}

/// Starts the SYSINIT program that far-calls the kernel, without running it.
pub fn start_dosinit(machine: &mut GuestMachine) {
    let [top_low, top_high] = MEMORY_TOP.to_le_bytes();
    let [iosys_low, iosys_high] = IOSYS_SEGMENT.to_le_bytes();
    let [kernel_low, kernel_high] = KERNEL_SEGMENT.to_le_bytes();
    let code = [
        0xB8,
        iosys_low,
        iosys_high, // MOV AX, IOSYS_SEGMENT
        0x8E,
        0xD8, // MOV DS, AX
        0xBE,
        0x00,
        0x00, // MOV SI, 0
        0xBA,
        top_low,
        top_high, // MOV DX, MEMORY_TOP
        0xB8,
        kernel_low,
        kernel_high, // MOV AX, KERNEL_SEGMENT
        0x8E,
        0xC0, // MOV ES, AX
        0x9A,
        0x00,
        0x00,
        kernel_low,
        kernel_high, // CALL FAR KERNEL:0000
        0x2E,
        0xA3,
        0x0E,
        0x00, // MOV CS:[000E], AX
        0x2E,
        0x89,
        0x1E,
        0x10,
        0x00, // MOV CS:[0010], BX
        0x2E,
        0x89,
        0x0E,
        0x12,
        0x00, // MOV CS:[0012], CX
        0x2E,
        0x89,
        0x36,
        0x14,
        0x00, // MOV CS:[0014], SI
        0x2E,
        0x89,
        0x2E,
        0x16,
        0x00, // MOV CS:[0016], BP
        0x2E,
        0x8C,
        0x06,
        0x00,
        0x00, // MOV CS:[0000], ES
        0x2E,
        0x89,
        0x3E,
        0x02,
        0x00, // MOV CS:[0002], DI
        0x2E,
        0x8C,
        0x1E,
        0x04,
        0x00, // MOV CS:[0004], DS
        0x2E,
        0x89,
        0x16,
        0x06,
        0x00, // MOV CS:[0006], DX
        0x2E,
        0x89,
        0x26,
        0x08,
        0x00, // MOV CS:[0008], SP
        0x2E,
        0x8C,
        0x16,
        0x0A,
        0x00, // MOV CS:[000A], SS
        0x9C, // PUSHF
        0x58, // POP AX
        0x2E,
        0xA3,
        0x0C,
        0x00, // MOV CS:[000C], AX
        0xF4, // HLT
    ];
    machine.start_program_at(SYSINIT_SEGMENT, &code);
}

/// Runs DOSINIT and returns the registers of its return.
pub fn run_dosinit(machine: &mut GuestMachine) -> DosInitReturn {
    start_dosinit(machine);
    assert!(machine.run_steps(5_000_000), "DOSINIT did not return");
    let word = |machine: &mut GuestMachine, offset: u16| {
        machine.read_word(linear(SYSINIT_SEGMENT, offset))
    };
    DosInitReturn {
        es: word(machine, 0x00),
        di: word(machine, 0x02),
        ds: word(machine, 0x04),
        dx: word(machine, 0x06),
        sp: word(machine, 0x08),
        ss: word(machine, 0x0A),
        flags: word(machine, 0x0C),
        ax: word(machine, 0x0E),
        bx: word(machine, 0x10),
        cx: word(machine, 0x12),
        si: word(machine, 0x14),
        bp: word(machine, 0x16),
    }
}

/// Returns the DPB of `drive_index` from the DPB chain of the list of lists.
pub fn parameter_block(machine: &mut GuestMachine, drive_index: u8) -> Option<(u16, u16)> {
    let mut offset = machine.read_word(list_of_lists());
    let mut segment = machine.read_word(list_of_lists() + 2);
    for _ in 0..26 {
        if (segment, offset) == (0xFFFF, 0xFFFF) {
            return None;
        }
        let address = linear(segment, offset);
        if machine.read_byte(address) == drive_index {
            return Some((segment, offset));
        }
        offset = machine.read_word(address + DPB_NEXT);
        segment = machine.read_word(address + DPB_NEXT + 2);
    }
    None
}

/// Builds the CDS array like `IO.SYS` after DOSINIT: one entry per drive up
/// to `last_drive`, `X:\` with the physical flag and the DPB for drives with
/// a DPB, zero otherwise.
pub fn install_cds(machine: &mut GuestMachine, last_drive: u8) {
    let base = linear(CDS_SEGMENT, 0);
    machine.write_bytes(
        base,
        &vec![0u8; usize::from(last_drive) * CDS_ENTRY_SIZE as usize],
    );
    for drive_index in 0..last_drive {
        let Some((segment, offset)) = parameter_block(machine, drive_index) else {
            continue;
        };
        let entry = base + u32::from(drive_index) * CDS_ENTRY_SIZE;
        machine.write_bytes(entry, &[b'A' + drive_index, b':', b'\\']);
        machine.write_word(entry + 0x43, 0x4000);
        machine.write_word(entry + 0x45, offset);
        machine.write_word(entry + 0x47, segment);
        machine.write_word(entry + 0x49, 0xFFFF);
        machine.write_word(entry + 0x4F, 2);
    }
    machine.write_word(list_of_lists() + 0x16, 0);
    machine.write_word(list_of_lists() + 0x18, CDS_SEGMENT);
    machine.write_bytes(list_of_lists() + 0x21, &[last_drive]);
}

/// Returns a machine after DOSINIT with the CDS array of LASTDRIVE R:.
pub fn booted_kernel(cdrom: Option<Vec<u8>>, menu_iso: &[u8], ramdisk: &[u8]) -> GuestMachine {
    let mut machine = kernel_machine(cdrom, menu_iso, ramdisk);
    run_dosinit(&mut machine);
    install_cds(&mut machine, 18);
    machine
}

/// Runs `code` at `SYSINIT_SEGMENT:0100h` to its `HLT`.
pub fn run(machine: &mut GuestMachine, code: &[u8]) {
    machine.run_program_at(SYSINIT_SEGMENT, code);
}

/// Returns the bytes that reached CON or INT 29h.
pub fn console_output(machine: &mut GuestMachine) -> Vec<u8> {
    let count = usize::from(machine.read_word(driver_address(OUTPUT_COUNT))).min(OUTPUT_LOG_SIZE);
    let start = driver_address(OUTPUT_LOG) as usize;
    machine.bus.ram[start..start + count].to_vec()
}

/// Returns the requests that the drivers received as (device tag,
/// command) pairs in call order.
pub fn request_log(machine: &mut GuestMachine) -> Vec<(u8, u8)> {
    let count = usize::from(machine.read_word(driver_address(COMMAND_LOG_COUNT)));
    assert!(count <= LOG_SIZE, "the request log wrapped");
    let commands = driver_address(COMMAND_LOG) as usize;
    let devices = driver_address(DEVICE_LOG) as usize;
    (0..count)
        .map(|index| {
            (
                machine.bus.ram[devices + index],
                machine.bus.ram[commands + index],
            )
        })
        .collect()
}

/// Offset of the `INT 24h` handler of the critical error tests.
pub const CRITICAL_ERROR_HANDLER: u16 = 0x0600;
/// Offset of the record of the `INT 24h` handler: AX, DI, BP, SI, the call
/// count byte and the extended error word.
pub const CRITICAL_ERROR_RECORD: u16 = 0x0640;
/// Offset of the answers of the `INT 24h` handler, one per call.
const CRITICAL_ERROR_ANSWERS: u16 = 0x0650;
/// Number of answer slots of the `INT 24h` handler.
const CRITICAL_ERROR_ANSWER_SLOTS: usize = 8;

/// Registers and calls that the `INT 24h` handler recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CriticalErrorRecord {
    /// AX of the last call.
    pub ax: u16,
    /// DI of the last call.
    pub di: u16,
    /// BP of the last call.
    pub bp: u16,
    /// SI of the last call.
    pub si: u16,
    /// Number of calls.
    pub calls: u8,
    /// Extended error (`AH=59h`) inside the last call.
    pub extended_error: u16,
}

/// Installs an `INT 24h` handler at `SYSINIT_SEGMENT:CRITICAL_ERROR_HANDLER`
/// that records its registers and the extended error and answers
/// `answers[n]` on call `n` (the last answer for every later call).
pub fn install_critical_error_handler(machine: &mut GuestMachine, answers: &[u8]) {
    assert!((1..=CRITICAL_ERROR_ANSWER_SLOTS).contains(&answers.len()));
    let last = (answers.len() - 1) as u8;
    #[rustfmt::skip]
    let code = [
        0x2E, 0xA3, 0x40, 0x06,             // MOV [CS:0640h],AX
        0x2E, 0x89, 0x3E, 0x42, 0x06,       // MOV [CS:0642h],DI
        0x2E, 0x89, 0x2E, 0x44, 0x06,       // MOV [CS:0644h],BP
        0x2E, 0x89, 0x36, 0x46, 0x06,       // MOV [CS:0646h],SI
        0x53,                               // PUSH BX
        0xB4, 0x59,                         // MOV AH,59h
        0x31, 0xDB,                         // XOR BX,BX
        0xCD, 0x21,                         // INT 21h
        0x2E, 0xA3, 0x4A, 0x06,             // MOV [CS:064Ah],AX
        0x2E, 0x8A, 0x1E, 0x48, 0x06,       // MOV BL,[CS:0648h]
        0x2E, 0xFE, 0x06, 0x48, 0x06,       // INC BYTE [CS:0648h]
        0x80, 0xFB, last,                   // CMP BL,last
        0x76, 0x02,                         // JBE answer
        0xB3, last,                         // MOV BL,last
        0x30, 0xFF,                         // answer: XOR BH,BH
        0x2E, 0x8A, 0x87, 0x50, 0x06,       // MOV AL,[CS:BX+0650h]
        0x5B,                               // POP BX
        0xCF,                               // IRET
    ];
    machine.write_bytes(linear(SYSINIT_SEGMENT, CRITICAL_ERROR_HANDLER), &code);
    machine.write_bytes(linear(SYSINIT_SEGMENT, CRITICAL_ERROR_RECORD), &[0; 12]);
    machine.write_bytes(linear(SYSINIT_SEGMENT, CRITICAL_ERROR_ANSWERS), answers);
    machine.set_vector(0x24, SYSINIT_SEGMENT, CRITICAL_ERROR_HANDLER);
}

/// Returns the record of the `INT 24h` handler.
pub fn critical_error_record(machine: &mut GuestMachine) -> CriticalErrorRecord {
    let base = linear(SYSINIT_SEGMENT, CRITICAL_ERROR_RECORD);
    CriticalErrorRecord {
        ax: machine.read_word(base),
        di: machine.read_word(base + 2),
        bp: machine.read_word(base + 4),
        si: machine.read_word(base + 6),
        calls: machine.read_byte(base + 8),
        extended_error: machine.read_word(base + 10),
    }
}

/// Runs `INT 21h AX=3D00h` for `path` and returns the carry flag and AX.
pub fn try_open(machine: &mut GuestMachine, path: &[u8]) -> (u16, u16) {
    let mut name = path.to_vec();
    name.push(0);
    machine.write_bytes(linear(SYSINIT_SEGMENT, 0x0200), &name);
    run(
        machine,
        &[0xB8, 0x00, 0x3D, 0xBA, 0x00, 0x02, 0xCD, 0x21, 0xF4],
    );
    (machine.cpu.flags() & 1, machine.cpu.ax())
}

/// Runs `INT 21h AH=59h` and returns the extended error.
pub fn extended_error(machine: &mut GuestMachine) -> u16 {
    run(machine, &[0xB4, 0x59, 0x31, 0xDB, 0xCD, 0x21, 0xF4]);
    machine.cpu.ax()
}

/// Returns a .COM program that installs an `INT 24h` handler with the
/// answer abort, opens `path` and ends with exit code `55h` when the open
/// returns.
pub fn abort_child_program(path: &[u8]) -> Vec<u8> {
    #[rustfmt::skip]
    let mut code = vec![
        0xB8, 0x24, 0x25,                   // 100h: MOV AX,2524h
        0xBA, 0x17, 0x01,                   // 103h: MOV DX,0117h
        0xCD, 0x21,                         // 106h: INT 21h
        0xB8, 0x00, 0x3D,                   // 108h: MOV AX,3D00h
        0xBA, 0x1A, 0x01,                   // 10Bh: MOV DX,011Ah
        0xCD, 0x21,                         // 10Eh: INT 21h
        0xB8, 0x55, 0x4C,                   // 110h: MOV AX,4C55h
        0xCD, 0x21,                         // 113h: INT 21h
        0x90, 0x90,                         // 115h: NOP, NOP
        0xB0, 0x02,                         // 117h: MOV AL,02h
        0xCF,                               // 119h: IRET
    ];
    code.extend_from_slice(path);
    code.push(0);
    code
}

/// Runs `path` through `AX=4B00h` from a program with its own PSP and
/// returns the carry of the EXEC and `AX` of `AH=4Dh` behind it.
pub fn exec_and_return_code(machine: &mut GuestMachine, path: &[u8]) -> (u16, u16) {
    write_exec_arguments(machine, b"");
    let mut name = path.to_vec();
    name.push(0);
    machine.write_bytes(linear(SYSINIT_SEGMENT, 0x0280), &name);
    let mut code = own_psp();
    code.extend_from_slice(&[
        0xBA, 0x80, 0x02, // MOV DX, 0280h (path)
        0xBB, 0x00, 0x02, // MOV BX, 0200h (parameter block)
        0xB8, 0x00, 0x4B, // MOV AX, 4B00h
        0xCD, 0x21, // INT 21h
        0x9C, // PUSHF
        0x5E, // POP SI
        0xB4, 0x4D, // MOV AH, 4Dh
        0xCD, 0x21, // INT 21h
        0xF4, // HLT
    ]);
    run(machine, &code);
    (machine.cpu.si() & 1, machine.cpu.ax())
}

/// Makes `count` RAM disk requests with `command` fail with the driver
/// error `error` after `skip` of them passed. A read or write matches when
/// it covers `sector` (`ANY_SECTOR` for every request).
pub fn fail_ramdisk_requests(
    machine: &mut GuestMachine,
    command: u8,
    skip: u8,
    count: u8,
    error: u8,
    sector: u16,
) {
    machine.write_bytes(driver_address(FAIL_COMMAND), &[command]);
    machine.write_bytes(driver_address(FAIL_SKIP), &[skip]);
    machine.write_bytes(driver_address(FAIL_COUNT), &[count]);
    machine.write_bytes(driver_address(FAIL_ERROR), &[error]);
    machine.write_word(driver_address(FAIL_SECTOR), sector);
}

/// Clears the request log of the drivers.
pub fn clear_request_log(machine: &mut GuestMachine) {
    machine.write_word(driver_address(COMMAND_LOG_COUNT), 0);
}

/// Puts bytes into the input buffer of CON.
pub fn type_input(machine: &mut GuestMachine, input: &[u8]) {
    machine.write_bytes(driver_address(INPUT_BUFFER), input);
    machine.write_word(driver_address(INPUT_COUNT), input.len() as u16);
    machine.write_word(driver_address(INPUT_POSITION), 0);
}

/// Bytes per sector of an ISO 9660 test volume.
const SECTOR: usize = 2048;
/// Sector of the root directory of an ISO 9660 test volume.
const ROOT_SECTOR: usize = 18;

/// Builds an ISO 9660 volume with `files` in its root directory.
pub fn iso_image(volume_label: &[u8], files: &[(&[u8], &[u8])]) -> Vec<u8> {
    let mut file_sectors = Vec::new();
    let mut next_sector = ROOT_SECTOR + 1;
    for (_, content) in files {
        file_sectors.push(next_sector);
        next_sector += content.len().div_ceil(SECTOR).max(1);
    }
    let total_sectors = next_sector;
    let mut image = vec![0u8; total_sectors * SECTOR];

    let both_endian_dword = |value: u32| {
        let mut bytes = [0u8; 8];
        bytes[..4].copy_from_slice(&value.to_le_bytes());
        bytes[4..].copy_from_slice(&value.to_be_bytes());
        bytes
    };
    let record = |name: &[u8], sector: usize, size: usize, flags: u8| {
        let length = 33 + name.len() + (name.len() + 1) % 2;
        let mut bytes = vec![0u8; length];
        bytes[0] = length as u8;
        bytes[2..10].copy_from_slice(&both_endian_dword(sector as u32));
        bytes[10..18].copy_from_slice(&both_endian_dword(size as u32));
        bytes[18..25].copy_from_slice(&[95, 7, 17, 12, 0, 0, 0]);
        bytes[25] = flags;
        bytes[28] = 1;
        bytes[31] = 1;
        bytes[32] = name.len() as u8;
        bytes[33..33 + name.len()].copy_from_slice(name);
        bytes
    };

    let descriptor = 16 * SECTOR;
    image[descriptor] = 1;
    image[descriptor + 1..descriptor + 6].copy_from_slice(b"CD001");
    image[descriptor + 6] = 1;
    image[descriptor + 40..descriptor + 72].fill(b' ');
    image[descriptor + 40..descriptor + 40 + volume_label.len()].copy_from_slice(volume_label);
    image[descriptor + 80..descriptor + 88]
        .copy_from_slice(&both_endian_dword(total_sectors as u32));
    image[descriptor + 128..descriptor + 130].copy_from_slice(&(SECTOR as u16).to_le_bytes());
    image[descriptor + 130..descriptor + 132].copy_from_slice(&(SECTOR as u16).to_be_bytes());
    let root = record(&[0], ROOT_SECTOR, SECTOR, 2);
    image[descriptor + 156..descriptor + 156 + root.len()].copy_from_slice(&root);
    let terminator = 17 * SECTOR;
    image[terminator] = 0xFF;
    image[terminator + 1..terminator + 6].copy_from_slice(b"CD001");
    image[terminator + 6] = 1;

    let mut directory = Vec::new();
    directory.extend(record(&[0], ROOT_SECTOR, SECTOR, 2));
    directory.extend(record(&[1], ROOT_SECTOR, SECTOR, 2));
    for ((name, content), sector) in files.iter().zip(&file_sectors) {
        let mut identifier = name.to_vec();
        identifier.extend_from_slice(b";1");
        directory.extend(record(&identifier, *sector, content.len(), 0));
        image[sector * SECTOR..sector * SECTOR + content.len()].copy_from_slice(content);
    }
    image[ROOT_SECTOR * SECTOR..ROOT_SECTOR * SECTOR + directory.len()].copy_from_slice(&directory);
    image
}

/// Sector size of the RAM disk.
pub const RAMDISK_SECTOR_SIZE: usize = 512;
/// Sectors of the RAM disk.
const RAMDISK_SECTORS: usize = 128;
/// First sector of the root directory of the RAM disk.
const RAMDISK_ROOT_SECTOR: usize = 3;
/// First data sector (cluster 2) of the RAM disk.
pub const RAMDISK_DATA_SECTOR: usize = 4;

/// Returns a FAT12 volume with `files` in the root directory, each in one
/// cluster from cluster 2 on. The BPB matches the INIT answer of the RAM
/// disk driver.
pub fn ramdisk_image(files: &[(&[u8; 11], &[u8])]) -> Vec<u8> {
    let mut image = vec![0u8; RAMDISK_SECTORS * RAMDISK_SECTOR_SIZE];
    image[0..3].copy_from_slice(&[0xEB, 0x3C, 0x90]);
    image[0x0B..0x0D].copy_from_slice(&(RAMDISK_SECTOR_SIZE as u16).to_le_bytes());
    image[0x0D] = 1;
    image[0x0E..0x10].copy_from_slice(&1u16.to_le_bytes());
    image[0x10] = 2;
    image[0x11..0x13].copy_from_slice(&16u16.to_le_bytes());
    image[0x13..0x15].copy_from_slice(&(RAMDISK_SECTORS as u16).to_le_bytes());
    image[0x15] = 0xF8;
    image[0x16..0x18].copy_from_slice(&1u16.to_le_bytes());
    image[0x1FE] = 0x55;
    image[0x1FF] = 0xAA;
    let mut fat = vec![0xF8u8, 0xFF, 0xFF];
    fat.resize(RAMDISK_SECTOR_SIZE, 0);
    for index in 0..files.len() {
        let cluster = 2 + index;
        let offset = cluster * 3 / 2;
        let value: u16 = 0x0FFF;
        if cluster % 2 == 0 {
            fat[offset] = value as u8;
            fat[offset + 1] = (fat[offset + 1] & 0xF0) | (value >> 8) as u8;
        } else {
            fat[offset] = (fat[offset] & 0x0F) | ((value << 4) as u8);
            fat[offset + 1] = (value >> 4) as u8;
        }
    }
    for copy in 0..2 {
        let base = (1 + copy) * RAMDISK_SECTOR_SIZE;
        image[base..base + RAMDISK_SECTOR_SIZE].copy_from_slice(&fat);
    }
    for (index, (name, content)) in files.iter().enumerate() {
        let entry = RAMDISK_ROOT_SECTOR * RAMDISK_SECTOR_SIZE + index * 32;
        image[entry..entry + 11].copy_from_slice(*name);
        image[entry + 0x1A..entry + 0x1C].copy_from_slice(&(2 + index as u16).to_le_bytes());
        image[entry + 0x1C..entry + 0x20].copy_from_slice(&(content.len() as u32).to_le_bytes());
        let data = (RAMDISK_DATA_SECTOR + index) * RAMDISK_SECTOR_SIZE;
        image[data..data + content.len()].copy_from_slice(content);
    }
    image
}

/// Code that makes `SYSINIT_SEGMENT` the current PSP like SYSINIT does:
/// `AH=55h` with `DX` = the segment, then `AH=50h`.
pub fn own_psp() -> Vec<u8> {
    let [segment_low, segment_high] = SYSINIT_SEGMENT.to_le_bytes();
    let [top_low, top_high] = MEMORY_TOP.to_le_bytes();
    vec![
        0xBA,
        segment_low,
        segment_high, // MOV DX, SYSINIT_SEGMENT
        0xBE,
        top_low,
        top_high, // MOV SI, MEMORY_TOP
        0xB4,
        0x55, // MOV AH, 55h
        0xCD,
        0x21, // INT 21h
        0xBB,
        segment_low,
        segment_high, // MOV BX, SYSINIT_SEGMENT
        0xB4,
        0x50, // MOV AH, 50h
        0xCD,
        0x21, // INT 21h
    ]
}

/// Code that runs `C:\COMMAND.COM` through `AX=4B00h` with the arguments of
/// `write_exec_arguments` and halts.
pub fn exec_command_com() -> Vec<u8> {
    let mut code = own_psp();
    code.extend_from_slice(&[
        0xBA, 0x80, 0x02, // MOV DX, 0280h (path)
        0xBB, 0x00, 0x02, // MOV BX, 0200h (parameter block)
        0xB8, 0x00, 0x4B, // MOV AX, 4B00h
        0xCD, 0x21, // INT 21h
        0xF4, // HLT
    ]);
    code
}

/// Writes the EXEC parameter block, the path and the command tail.
pub fn write_exec_arguments(machine: &mut GuestMachine, tail: &[u8]) {
    let base = linear(SYSINIT_SEGMENT, 0);
    let mut block = vec![0u8; 14];
    block[2..4].copy_from_slice(&0x0300u16.to_le_bytes());
    block[4..6].copy_from_slice(&SYSINIT_SEGMENT.to_le_bytes());
    block[6..8].copy_from_slice(&0x005Cu16.to_le_bytes());
    block[8..10].copy_from_slice(&SYSINIT_SEGMENT.to_le_bytes());
    block[10..12].copy_from_slice(&0x006Cu16.to_le_bytes());
    block[12..14].copy_from_slice(&SYSINIT_SEGMENT.to_le_bytes());
    machine.write_bytes(base + 0x0200, &block);
    machine.write_bytes(base + 0x0280, b"C:\\COMMAND.COM\0");
    let mut command_tail = vec![tail.len() as u8];
    command_tail.extend_from_slice(tail);
    command_tail.push(0x0D);
    machine.write_bytes(base + 0x0300, &command_tail);
}
