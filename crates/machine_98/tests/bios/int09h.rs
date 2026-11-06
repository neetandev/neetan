use common::{Bus, Cpu};
use machine_98::Pc9801F;

use super::{
    KB_STATUS_START, TEST_CALLBACK, TEST_CODE, create_machine_f, read_ivt_vector, write_bytes,
};

const RESULT_AX: u32 = 0x0600;
const RESULT_BH: u32 = 0x0602;
const RESULT_FLAGS: u32 = 0x0604;
const N_KEY_MAKE: u8 = 0x2E;
const N_KEY_BUFFER_ENTRY: u16 = 0x2E6E;

#[test]
fn int18h_key_code_read_polls_pending_scancode_when_interrupts_disabled() {
    let mut machine = create_machine_f();

    let program = [
        0xB4, 0x05, // MOV AH,05h
        0xCD, 0x18, // INT 18h
        0xA3, 0x00, 0x06, // MOV [0600h],AX
        0x88, 0x3E, 0x02, 0x06, // MOV [0602h],BH
        0x9C, // PUSHF
        0x58, // POP AX
        0xA3, 0x04, 0x06, // MOV [0604h],AX
        0xF4, // HLT
    ];
    for (offset, &byte) in program.iter().enumerate() {
        machine.bus.write_byte(0x0100 + offset as u32, byte);
    }

    machine.bus.push_keyboard_scancode(N_KEY_MAKE);
    machine.cpu.load_state(&{
        let mut state = cpu::I8086State {
            ip: 0x0100,
            ..Default::default()
        };
        state.initialize_cold_frontend();
        state.set_sp(0x4000);
        state.set_compressed_flags(0x0002);
        state
    });

    machine.run_for(100_000);

    assert!(machine.cpu.halted(), "guest program should halt");
    assert_eq!(
        machine.bus.read_word(RESULT_AX),
        N_KEY_BUFFER_ENTRY,
        "INT 18h AH=05h should return the pending n key even though IF was clear"
    );
    assert_eq!(
        machine.bus.read_byte(RESULT_BH),
        0x01,
        "INT 18h AH=05h should report a key was available"
    );
    assert_eq!(
        machine.bus.read_word(RESULT_FLAGS) & 0x0200,
        0,
        "the test must keep IF clear across the BIOS call"
    );

    let state = machine.inspection_state();
    assert_eq!(
        state.memory.ram[0x0528], 0,
        "the BIOS keyboard buffer entry should have been consumed"
    );
    assert_eq!(
        state.pic.chips[0].isr & 0x02,
        0,
        "IRQ 1 should not have been acknowledged while IF was clear"
    );
    assert!(
        !state.keyboard.rx_ready && state.keyboard.rx_fifo.is_empty(),
        "the pending controller scan code should have been drained by INT 18h polling"
    );
}

/// Make code of the STOP key.
const STOP_MAKE: u8 = 0x60;
/// Release code of the STOP key.
const STOP_RELEASE: u8 = 0xE0;
/// Make code of the COPY key.
const COPY_MAKE: u8 = 0x61;
/// Release code of the COPY key.
const COPY_RELEASE: u8 = 0xE1;
/// Bit of the STOP key in the key status group 0Ch.
const STOP_STATUS_BIT: u8 = 0x01;
/// Bit of the COPY key in the key status group 0Ch.
const COPY_STATUS_BIT: u8 = 0x02;
/// Address of the key status group 0Ch that holds STOP and COPY.
const KEY_STATUS_GROUP_0C: u16 = (KB_STATUS_START + 0x0C) as u16;
/// Address of the recording INT 06h handler.
const STOP_CALLBACK: u32 = TEST_CALLBACK;
/// Address of the recording INT 05h handler.
const COPY_CALLBACK: u32 = TEST_CALLBACK + 0x0100;
/// SP of the program before it pushes its three words.
const INTERRUPTED_STACK_TOP: u16 = 0x4000;
/// SP of the program after it pushes its three words.
const INTERRUPTED_SP: u16 = 0x3FFA;
/// AX of the program while it waits for the key interrupt.
const INTERRUPTED_AX: u16 = 0xA5A5;
/// Address where the program stores its SP after the key interrupt.
const RESULT_INTERRUPTED_SP: u32 = 0x0600;
/// Address where the handler stores its return IP.
const RESULT_RETURN_IP: u32 = 0x0602;
/// Address where the handler stores its return CS.
const RESULT_RETURN_CS: u32 = 0x0604;
/// Address where the handler stores its live flags.
const RESULT_CALLBACK_FLAGS: u32 = 0x0606;
/// Address where the handler stores the AX it sees.
const RESULT_CALLBACK_AX: u32 = 0x0608;
/// Address where the handler stores its vector.
const RESULT_VECTOR: u32 = 0x060A;
/// Address of the handler call count.
const RESULT_CALL_COUNT: u32 = 0x060B;
/// Address where the program stores its AX after the key interrupt.
const RESULT_INTERRUPTED_AX: u32 = 0x060C;
/// Cycle budget of each run phase.
const KEY_INTERRUPT_BUDGET: u64 = 2_000_000;

/// Returns a key interrupt handler that records its return address, flags,
/// AX and vector, and counts its calls.
fn key_interrupt_callback(vector: u8) -> Vec<u8> {
    vec![
        0x55, // PUSH BP
        0x89, 0xE5, // MOV BP,SP
        0xA3, 0x08, 0x06, // MOV [0608h],AX
        0x53, // PUSH BX
        0x8B, 0x5E, 0x02, // MOV BX,[BP+2]
        0x89, 0x1E, 0x02, 0x06, // MOV [0602h],BX
        0x8B, 0x5E, 0x04, // MOV BX,[BP+4]
        0x89, 0x1E, 0x04, 0x06, // MOV [0604h],BX
        0x9C, // PUSHF
        0x5B, // POP BX
        0x89, 0x1E, 0x06, 0x06, // MOV [0606h],BX
        0xC6, 0x06, 0x0A, 0x06, vector, // MOV BYTE [060Ah],vector
        0xFE, 0x06, 0x0B, 0x06, // INC BYTE [060Bh]
        0x5B, // POP BX
        0x5D, // POP BP
        0xCF, // IRET
    ]
}

/// Returns a program that pushes 1111h, 2222h and 3333h, enables interrupts
/// and waits until the status bit of the key group 0Ch is set. With
/// `wait_for_release` it then waits until the bit is clear again. It stores
/// SP and AX and halts.
fn key_interrupt_program(status_bit: u8, wait_for_release: bool) -> Vec<u8> {
    let [status_low, status_high] = KEY_STATUS_GROUP_0C.to_le_bytes();
    let [ax_low, ax_high] = INTERRUPTED_AX.to_le_bytes();
    #[rustfmt::skip]
    let mut program = vec![
        0xB8, 0x11, 0x11, // MOV AX,1111h
        0x50, // PUSH AX
        0xB8, 0x22, 0x22, // MOV AX,2222h
        0x50, // PUSH AX
        0xB8, 0x33, 0x33, // MOV AX,3333h
        0x50, // PUSH AX
        0xB8, ax_low, ax_high, // MOV AX,A5A5h
        0xFB, // STI
        0xF6, 0x06, status_low, status_high, status_bit, // TEST BYTE [0536h],bit
        0x74, 0xF9, // JZ -7
    ];
    if wait_for_release {
        #[rustfmt::skip]
        program.extend_from_slice(&[
            0xF6, 0x06, status_low, status_high, status_bit, // TEST BYTE [0536h],bit
            0x75, 0xF9, // JNZ -7
        ]);
    }
    program.extend_from_slice(&[
        0xFA, // CLI
        0x89, 0x26, 0x00, 0x06, // MOV [0600h],SP
        0xA3, 0x0C, 0x06, // MOV [060Ch],AX
        0xF4, // HLT
    ]);
    program
}

/// Boots a PC-9801F and points INT 05h and INT 06h at recording handlers.
/// It feeds the make code and runs the program. With a release code it
/// feeds the release code after the program has seen the make code and
/// runs on. The program has to halt.
fn run_key_interrupt(make_code: u8, release_code: Option<u8>, status_bit: u8) -> Pc9801F {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);

    write_bytes(
        &mut machine.bus,
        STOP_CALLBACK,
        &key_interrupt_callback(0x06),
    );
    write_bytes(
        &mut machine.bus,
        COPY_CALLBACK,
        &key_interrupt_callback(0x05),
    );
    for (vector, callback) in [(0x06u32, STOP_CALLBACK), (0x05, COPY_CALLBACK)] {
        let [offset_low, offset_high] = (callback as u16).to_le_bytes();
        write_bytes(
            &mut machine.bus,
            vector * 4,
            &[offset_low, offset_high, 0x00, 0x00],
        );
    }
    write_bytes(
        &mut machine.bus,
        TEST_CODE,
        &key_interrupt_program(status_bit, release_code.is_some()),
    );

    machine.cpu.load_state(&{
        let mut state = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        state.initialize_cold_frontend();
        state.set_sp(INTERRUPTED_STACK_TOP);
        state.set_compressed_flags(0x0002);
        state
    });
    machine.bus.push_keyboard_scancode(make_code);
    machine.run_for(KEY_INTERRUPT_BUDGET);
    if let Some(release_code) = release_code {
        machine.bus.push_keyboard_scancode(release_code);
        machine.run_for(KEY_INTERRUPT_BUDGET);
    }
    assert!(machine.cpu.halted(), "guest program should halt");
    machine
}

/// Asserts that the interrupted program got its SP, AX and stack back.
fn assert_interrupted_program_intact(machine: &mut Pc9801F) {
    assert_eq!(
        machine.bus.read_word(RESULT_INTERRUPTED_SP),
        INTERRUPTED_SP,
        "SP of the interrupted program should keep its value"
    );
    assert_eq!(
        machine.bus.read_word(RESULT_INTERRUPTED_AX),
        INTERRUPTED_AX,
        "AX of the interrupted program should keep its value"
    );
    assert_eq!(machine.bus.read_word(0x3FFA), 0x3333);
    assert_eq!(machine.bus.read_word(0x3FFC), 0x2222);
    assert_eq!(machine.bus.read_word(0x3FFE), 0x1111);
}

/// Asserts that the key interrupt handler of `vector` ran once from an INT
/// instruction of the ROM with IF clear and the interrupted AX.
fn assert_key_interrupt_raised(machine: &mut Pc9801F, vector: u8) {
    assert_eq!(machine.bus.read_byte(RESULT_CALL_COUNT), 1);
    assert_eq!(machine.bus.read_byte(RESULT_VECTOR), vector);

    let return_ip = machine.bus.read_word(RESULT_RETURN_IP);
    let return_cs = machine.bus.read_word(RESULT_RETURN_CS);
    let (int09h_segment, _) = read_ivt_vector(&machine.inspection_state().memory.ram, 0x09);
    assert_eq!(
        return_cs, int09h_segment,
        "the handler should return into the INT 09h handler of the ROM"
    );
    let return_address = (u32::from(return_cs) << 4) + u32::from(return_ip);
    assert_eq!(
        [
            machine.bus.read_byte(return_address - 2),
            machine.bus.read_byte(return_address - 1),
        ],
        [0xCD, vector],
        "the bytes before the return address should be INT {vector:02X}h"
    );
    assert_eq!(
        machine.bus.read_word(RESULT_CALLBACK_FLAGS) & 0x0200,
        0,
        "the handler should run with IF clear"
    );
    assert_eq!(
        machine.bus.read_word(RESULT_CALLBACK_AX),
        INTERRUPTED_AX,
        "the handler should see the AX of the interrupted program"
    );
}

#[test]
fn int09h_stop_key_raises_int06h() {
    let mut machine = run_key_interrupt(STOP_MAKE, None, STOP_STATUS_BIT);

    assert_key_interrupt_raised(&mut machine, 0x06);
    assert_interrupted_program_intact(&mut machine);
}

#[test]
fn int09h_copy_key_raises_int05h() {
    let mut machine = run_key_interrupt(COPY_MAKE, None, COPY_STATUS_BIT);

    assert_key_interrupt_raised(&mut machine, 0x05);
    assert_interrupted_program_intact(&mut machine);
}

#[test]
fn int09h_stop_key_release_raises_nothing() {
    let mut machine = run_key_interrupt(STOP_MAKE, Some(STOP_RELEASE), STOP_STATUS_BIT);

    assert_key_interrupt_raised(&mut machine, 0x06);
    assert_interrupted_program_intact(&mut machine);
}

#[test]
fn int09h_copy_key_release_raises_nothing() {
    let mut machine = run_key_interrupt(COPY_MAKE, Some(COPY_RELEASE), COPY_STATUS_BIT);

    assert_key_interrupt_raised(&mut machine, 0x05);
    assert_interrupted_program_intact(&mut machine);
}
