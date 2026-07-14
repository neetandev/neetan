use common::{Bus, Cpu};

use super::create_machine_f;

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

    let state = machine.save_state();
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
