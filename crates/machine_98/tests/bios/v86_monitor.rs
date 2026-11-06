//! V86 monitor harness. It enters V86 mode with an I/O permission bitmap that
//! traps the master PIC IMR port 0002h. Its #GP handler emulates and logs each
//! trapped access.

use common::{Bus, Cpu as _};

use super::{TEST_CODE, write_bytes};

/// CPU cycle budget of one monitor run.
const MONITOR_BUDGET: u64 = 2_000_000;

/// Linear address of the GDT of the V86 monitor.
const GDT_BASE: u32 = 0x0800;
/// Linear address of the 32-bit TSS of the V86 monitor.
const TSS_BASE: u32 = 0x0900;
/// TSS limit: the 68h byte TSS and an I/O permission bitmap for ports 0000h-07FFh.
const TSS_LIMIT: u32 = 0x0168;
/// Offset of the I/O permission bitmap inside the TSS.
const TSS_IOPB_OFFSET: u32 = 0x0068;
/// Linear address of the IDT of the V86 monitor.
const IDT_BASE: u32 = 0x0E00;
/// Number of IDT gates (exceptions 00h-0Dh).
const IDT_GATES: u32 = 14;
/// Linear address of the GDTR image for LGDT.
const GDTR_IMAGE: u32 = 0x0FF0;
/// Linear address of the IDTR image for LIDT.
const IDTR_IMAGE: u32 = 0x0FF8;
/// Linear address of the monitor exception handlers.
const MONITOR_HANDLERS: u32 = 0x1100;
/// Linear address of the monitor general protection handler.
const MONITOR_GP_HANDLER: u32 = 0x1104;
/// Real mode segment of the V86 code.
const V86_CODE_SEGMENT: u16 = 0x0300;
/// Number of logged port accesses.
const GP_LOG_COUNT: u32 = 0x0610;
/// Port access log: port, value and kind (1 = OUT, 2 = IN) in 4 byte entries.
const GP_LOG: u32 = 0x0700;
/// Marker the V86 code writes after the call returned.
const V86_RETURNED: u32 = 0x0650;
/// I/O port of the master PIC IMR.
const MASTER_IMR_PORT: u16 = 0x02;

/// Real mode entry: sets the master IMR, enters protected mode and IRETDs
/// into V86 mode at 0300:0000 with IOPL=3, IF=0, SS:SP=0000:5000 and the
/// data segments at 0000h.
#[rustfmt::skip]
fn make_monitor_entry_code(imr: u8) -> Vec<u8> {
    vec![
        0xFA,                                       // CLI
        0xB0, imr,                                  // MOV AL, imr
        0xE6, 0x02,                                 // OUT 0x02, AL
        0x0F, 0x01, 0x16, 0xF0, 0x0F,               // LGDT [0x0FF0]
        0x0F, 0x01, 0x1E, 0xF8, 0x0F,               // LIDT [0x0FF8]
        0x0F, 0x20, 0xC0,                           // MOV EAX, CR0
        0x0C, 0x01,                                 // OR AL, 1
        0x0F, 0x22, 0xC0,                           // MOV CR0, EAX
        0x66, 0xEA, 0x1F, 0x10, 0x00, 0x00, 0x08, 0x00, // JMP DWORD 0x08:0x101F
        0x66, 0xB8, 0x10, 0x00,                     // MOV AX, 0x10
        0x8E, 0xD8,                                 // MOV DS, AX
        0x8E, 0xC0,                                 // MOV ES, AX
        0x8E, 0xD0,                                 // MOV SS, AX
        0xBC, 0x00, 0x70, 0x00, 0x00,               // MOV ESP, 0x7000
        0x66, 0xB8, 0x18, 0x00,                     // MOV AX, 0x18
        0x0F, 0x00, 0xD8,                           // LTR AX
        0x6A, 0x00,                                 // PUSH DWORD 0 (GS)
        0x6A, 0x00,                                 // PUSH DWORD 0 (FS)
        0x6A, 0x00,                                 // PUSH DWORD 0 (DS)
        0x6A, 0x00,                                 // PUSH DWORD 0 (ES)
        0x6A, 0x00,                                 // PUSH DWORD 0 (SS)
        0x68, 0x00, 0x50, 0x00, 0x00,               // PUSH DWORD 0x5000 (ESP)
        0x68, 0x02, 0x30, 0x02, 0x00,               // PUSH DWORD 0x00023002 (EFLAGS)
        0x68, 0x00, 0x03, 0x00, 0x00,               // PUSH DWORD 0x0300 (CS)
        0x6A, 0x00,                                 // PUSH DWORD 0 (EIP)
        0xCF,                                       // IRETD
    ]
}

/// Monitor handlers. Every exception halts. The #GP handler emulates a
/// trapped `IN AL,imm8` or `OUT imm8,AL` of the V86 code, logs it and skips
/// it. Any other #GP, the HLT at the end of the V86 code included, halts.
#[rustfmt::skip]
const MONITOR_HANDLER_CODE: &[u8] = &[
    0xFA,                                           // 1100: CLI
    0xF4,                                           // 1101: HLT
    0xEB, 0xFC,                                     // 1102: JMP 0x1100
    0x50,                                           // 1104: PUSH EAX
    0x53,                                           // PUSH EBX
    0x52,                                           // PUSH EDX
    0x66, 0xB8, 0x10, 0x00,                         // MOV AX, 0x10
    0x8E, 0xD8,                                     // MOV DS, AX
    0x0F, 0xB7, 0x5C, 0x24, 0x14,                   // MOVZX EBX, WORD [ESP+0x14] (CS)
    0xC1, 0xE3, 0x04,                               // SHL EBX, 4
    0x03, 0x5C, 0x24, 0x10,                         // ADD EBX, [ESP+0x10] (EIP)
    0x0F, 0xB6, 0x53, 0x01,                         // MOVZX EDX, BYTE [EBX+1] (port)
    0x8A, 0x03,                                     // MOV AL, [EBX] (opcode)
    0x3C, 0xE4,                                     // CMP AL, 0xE4
    0x74, 0x0D,                                     // JZ 0x1130
    0x3C, 0xE6,                                     // CMP AL, 0xE6
    0x75, 0xD9,                                     // JNZ 0x1100
    0x8B, 0x44, 0x24, 0x08,                         // MOV EAX, [ESP+0x08] (V86 EAX)
    0xEE,                                           // OUT DX, AL
    0xB4, 0x01,                                     // MOV AH, 1
    0xEB, 0x07,                                     // JMP 0x1137
    0xEC,                                           // 1130: IN AL, DX
    0x88, 0x44, 0x24, 0x08,                         // MOV [ESP+0x08], AL (V86 AL)
    0xB4, 0x02,                                     // MOV AH, 2
    0x0F, 0xB6, 0x1D, 0x10, 0x06, 0x00, 0x00,       // 1137: MOVZX EBX, BYTE [0x0610]
    0x88, 0x14, 0x9D, 0x00, 0x07, 0x00, 0x00,       // MOV [EBX*4+0x0700], DL
    0x88, 0x04, 0x9D, 0x01, 0x07, 0x00, 0x00,       // MOV [EBX*4+0x0701], AL
    0x88, 0x24, 0x9D, 0x02, 0x07, 0x00, 0x00,       // MOV [EBX*4+0x0702], AH
    0xFE, 0x05, 0x10, 0x06, 0x00, 0x00,             // INC BYTE [0x0610]
    0x83, 0x44, 0x24, 0x10, 0x02,                   // ADD DWORD [ESP+0x10], 2
    0x5A,                                           // POP EDX
    0x5B,                                           // POP EBX
    0x58,                                           // POP EAX
    0x83, 0xC4, 0x04,                               // ADD ESP, 4
    0xCF,                                           // IRETD
];

/// General register that `load_register` loads.
#[derive(Debug, Clone, Copy)]
pub(super) enum Register {
    /// AX.
    Ax,
    /// BX.
    Bx,
    /// CX.
    Cx,
    /// DX.
    Dx,
    /// DI.
    Di,
}

/// Machine code for `MOV register, value`.
pub(super) fn load_register(register: Register, value: u16) -> [u8; 3] {
    let opcode = match register {
        Register::Ax => 0xB8,
        Register::Bx => 0xBB,
        Register::Cx => 0xB9,
        Register::Dx => 0xBA,
        Register::Di => 0xBF,
    };
    let [low, high] = value.to_le_bytes();
    [opcode, low, high]
}

/// Machine code for `MOV WORD [address], value` with DS=0000h.
pub(super) fn store_word(address: u16, value: u16) -> [u8; 6] {
    let [address_low, address_high] = address.to_le_bytes();
    let [low, high] = value.to_le_bytes();
    [0xC7, 0x06, address_low, address_high, low, high]
}

/// V86 code: runs `setup`, calls the handler at `segment:offset` like an
/// interrupt, writes the return marker and halts.
#[rustfmt::skip]
pub(super) fn make_v86_call_code(setup: &[u8], (segment, offset): (u16, u16)) -> Vec<u8> {
    let mut code = setup.to_vec();
    code.extend_from_slice(&[
        0x9C,                                               // PUSHF
        0x9A, offset as u8, (offset >> 8) as u8,
        segment as u8, (segment >> 8) as u8,                // CALL FAR segment:offset
        0xC6, 0x06, 0x50, 0x06, 0x77,                       // MOV BYTE [0x0650], 0x77
        0xF4,                                               // HLT
    ]);
    code
}

/// Code at `code_offset` of its segment: runs `setup`, pushes an interrupt
/// frame that returns behind this code and the registers DS, SI, DI, ES, BP,
/// DX, CX, BX and AX, then jumps to the ROM entry at `segment:offset`.
#[rustfmt::skip]
pub(super) fn make_rom_entry_call(
    setup: &[u8],
    (segment, offset): (u16, u16),
    code_offset: u16,
) -> Vec<u8> {
    const FRAME_CODE_SIZE: u16 = 19;
    let return_offset = code_offset + setup.len() as u16 + FRAME_CODE_SIZE;
    let mut code = setup.to_vec();
    code.extend_from_slice(&[
        0x9C,                                               // PUSHF
        0x0E,                                               // PUSH CS
        0x68, return_offset as u8, (return_offset >> 8) as u8, // PUSH return_offset
        0x1E,                                               // PUSH DS
        0x56,                                               // PUSH SI
        0x57,                                               // PUSH DI
        0x06,                                               // PUSH ES
        0x55,                                               // PUSH BP
        0x52,                                               // PUSH DX
        0x51,                                               // PUSH CX
        0x53,                                               // PUSH BX
        0x50,                                               // PUSH AX
        0xEA, offset as u8, (offset >> 8) as u8,
        segment as u8, (segment >> 8) as u8,                // JMP FAR segment:offset
    ]);
    assert_eq!(code_offset + code.len() as u16, return_offset);
    code
}

/// Writes the GDT, the TSS with an I/O permission bitmap that traps only
/// port 0002h, the IDT and the descriptor table register images.
fn write_monitor_tables(bus: &mut impl Bus) {
    let gdt: [u64; 4] = [
        0,
        0x00CF_9A00_0000_FFFF,
        0x00CF_9200_0000_FFFF,
        0x0000_8900_0000_0000 | (u64::from(TSS_BASE) << 16) | u64::from(TSS_LIMIT),
    ];
    for (index, entry) in gdt.iter().enumerate() {
        write_bytes(bus, GDT_BASE + index as u32 * 8, &entry.to_le_bytes());
    }

    for offset in 0..=TSS_LIMIT {
        bus.write_byte(TSS_BASE + offset, 0);
    }
    write_bytes(bus, TSS_BASE + 0x04, &0x7000u32.to_le_bytes());
    write_bytes(bus, TSS_BASE + 0x08, &0x0010u16.to_le_bytes());
    write_bytes(
        bus,
        TSS_BASE + 0x66,
        &(TSS_IOPB_OFFSET as u16).to_le_bytes(),
    );
    let iopb = TSS_BASE + TSS_IOPB_OFFSET;
    bus.write_byte(
        iopb + u32::from(MASTER_IMR_PORT / 8),
        1 << (MASTER_IMR_PORT % 8),
    );
    bus.write_byte(TSS_BASE + TSS_LIMIT, 0xFF);

    for vector in 0..IDT_GATES {
        let handler = if vector == 0x0D {
            MONITOR_GP_HANDLER
        } else {
            MONITOR_HANDLERS
        };
        let mut gate = [0u8; 8];
        gate[0..2].copy_from_slice(&(handler as u16).to_le_bytes());
        gate[2..4].copy_from_slice(&0x0008u16.to_le_bytes());
        gate[5] = 0x8E;
        gate[6..8].copy_from_slice(&((handler >> 16) as u16).to_le_bytes());
        write_bytes(bus, IDT_BASE + vector * 8, &gate);
    }

    write_bytes(bus, GDTR_IMAGE, &(gdt.len() as u16 * 8 - 1).to_le_bytes());
    write_bytes(bus, GDTR_IMAGE + 2, &GDT_BASE.to_le_bytes());
    write_bytes(bus, IDTR_IMAGE, &(IDT_GATES as u16 * 8 - 1).to_le_bytes());
    write_bytes(bus, IDTR_IMAGE + 2, &IDT_BASE.to_le_bytes());
}

/// Runs real mode code at TEST_CODE until the budget is spent.
pub(super) fn run_real_mode(machine: &mut machine_98::Pc9801Ra, code: &[u8]) {
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut state = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        state.set_esp(0x4000);
        state
    });
    machine.run_for(MONITOR_BUDGET);
}

/// Port access of the V86 code that the monitor trapped.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum TrappedAccess {
    /// `IN AL,port` that read `value`.
    In { port: u16, value: u8 },
    /// `OUT port,AL` that wrote `value`.
    Out { port: u16, value: u8 },
}

/// Runs `v86_code` under the monitor with the master IMR set to `imr` and
/// returns the trapped port accesses.
pub(super) fn run_under_v86_monitor(
    machine: &mut machine_98::Pc9801Ra,
    imr: u8,
    v86_code: &[u8],
) -> Vec<TrappedAccess> {
    for address in GP_LOG_COUNT..GP_LOG + 0x100 {
        machine.bus.write_byte(address, 0);
    }
    write_monitor_tables(&mut machine.bus);
    write_bytes(&mut machine.bus, MONITOR_HANDLERS, MONITOR_HANDLER_CODE);
    write_bytes(&mut machine.bus, u32::from(V86_CODE_SEGMENT) << 4, v86_code);
    run_real_mode(machine, &make_monitor_entry_code(imr));

    assert!(machine.cpu.halted(), "the monitor did not halt");
    assert_eq!(
        machine.bus.read_byte(V86_RETURNED),
        0x77,
        "the V86 call did not return"
    );

    let count = u32::from(machine.bus.read_byte(GP_LOG_COUNT));
    (0..count)
        .map(|index| {
            let entry = GP_LOG + index * 4;
            let port = u16::from(machine.bus.read_byte(entry));
            let value = machine.bus.read_byte(entry + 1);
            match machine.bus.read_byte(entry + 2) {
                1 => TrappedAccess::Out { port, value },
                2 => TrappedAccess::In { port, value },
                kind => panic!("unknown access kind {kind}"),
            }
        })
        .collect()
}

/// Asserts that the monitor saw the read-modify-write of the master IMR and
/// that the IMR holds `expected`.
pub(super) fn assert_monitor_saw_imr_update(
    machine: &mut machine_98::Pc9801Ra,
    accesses: &[TrappedAccess],
    before: u8,
    expected: u8,
) {
    assert_eq!(
        accesses,
        [
            TrappedAccess::In {
                port: MASTER_IMR_PORT,
                value: before,
            },
            TrappedAccess::Out {
                port: MASTER_IMR_PORT,
                value: expected,
            },
        ]
    );
    assert_eq!(machine.inspection_state().pic.chips[0].imr, expected);
}
