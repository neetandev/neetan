//! Integration tests that run hand-assembled machine code through the real i486
//! core and FM Towns bus: they prove the run loop, memory, IVT dispatch, PIC,
//! interval timer, keyboard, and scheduler all work together end to end.

#[path = "common/harness.rs"]
mod harness;

use common::{Bus, Cpu, Machine};
use harness::{load_code, machine_mx, real_mode_state};

/// Executes a small real-mode program from RAM and checks its memory result.
/// Proves the CPU fetches, executes, and writes back through the Towns bus.
#[test]
fn executes_hand_assembled_program_from_ram() {
    let mut machine = machine_mx();

    // MOV AX, 0x1234 ; MOV [0x3000], AX ; HLT
    let program = [0xB8, 0x34, 0x12, 0xA3, 0x00, 0x30, 0xF4];
    load_code(&mut machine.bus, 0x2000, &program);

    machine
        .cpu
        .load_state(&real_mode_state(0, 0x2000, 0, 0x1000));
    machine.run_for(1_000);

    assert!(machine.cpu.halted(), "the program should HLT");
    assert_eq!(machine.bus.read_byte(0x3000), 0x34);
    assert_eq!(machine.bus.read_byte(0x3001), 0x12);
}

/// Programs the master PIC and interval-timer channel 0 to raise IRQ 0, installs
/// a real-mode handler through the IVT, and confirms the CPU takes the interrupt:
/// the handler increments a memory counter and the machine settles halted. This
/// exercises CPU + PIC + PIT + scheduler + bus wiring end to end.
#[test]
fn interval_timer_interrupt_is_delivered_to_the_cpu() {
    let mut machine = machine_mx();

    // Master PIC: ICW1, ICW2 (vector base 0x08), ICW3 (slave on IR7), ICW4
    // (8086 mode), OCW1 (unmask all).
    machine.bus.io_write_byte(0x0000, 0x11);
    machine.bus.io_write_byte(0x0002, 0x08);
    machine.bus.io_write_byte(0x0002, 0x80);
    machine.bus.io_write_byte(0x0002, 0x01);
    machine.bus.io_write_byte(0x0002, 0x00);

    // IVT vector 0x08 (IRQ 0) -> 0x0000:0x1200.
    machine.bus.write_word(0x08 * 4, 0x1200);
    machine.bus.write_word(0x08 * 4 + 2, 0x0000);

    // Handler: INC [0x1500]; clear timer0 OUT (0x60 <- 0x81, keep enable); EOI
    // the master PIC (0x00 <- 0x20); IRET.
    let handler = [
        0xFE, 0x06, 0x00, 0x15, // INC byte [0x1500]
        0xB0, 0x81, 0xE6, 0x60, // MOV AL, 0x81 ; OUT 0x60, AL
        0xB0, 0x20, 0xE6, 0x00, // MOV AL, 0x20 ; OUT 0x00, AL
        0xCF, // IRET
    ];
    load_code(&mut machine.bus, 0x1200, &handler);

    // Main: STI ; HLT ; CLI ; HLT (the second HLT is the permanent resting state
    // once interrupts are disabled after the first service).
    let main = [0xFB, 0xF4, 0xFA, 0xF4];
    load_code(&mut machine.bus, 0x1000, &main);
    machine.bus.write_byte(0x1500, 0x00);

    // Enable the channel-0 timer interrupt (0x0060 bit 0), then program channel 0
    // as periodic (mode 3, low-then-high) with a short reload so it fires soon.
    machine.bus.io_write_byte(0x0060, 0x01);
    machine.bus.io_write_byte(0x0046, 0x36);
    machine.bus.io_write_byte(0x0040, 0xC8); // reload low (200)
    machine.bus.io_write_byte(0x0040, 0x00); // reload high

    machine
        .cpu
        .load_state(&real_mode_state(0, 0x1000, 0, 0x0F00));
    machine.run_for(1_000_000);

    assert!(
        machine.bus.read_byte(0x1500) >= 1,
        "the timer interrupt handler never ran"
    );
    assert!(
        machine.cpu.halted(),
        "the CPU should rest halted after IRET"
    );
}

/// Enables the keyboard interrupt, injects a key event, and confirms the CPU
/// takes IRQ 1: the handler drains the two-byte serial packet, increments a
/// counter, and EOIs the PIC. This exercises the keyboard -> PIC -> CPU path end
/// to end.
#[test]
fn keyboard_interrupt_is_delivered_to_the_cpu() {
    let mut machine = machine_mx();

    // Master PIC init (vector base 0x08 -> IRQ 1 is vector 0x09).
    machine.bus.io_write_byte(0x0000, 0x11);
    machine.bus.io_write_byte(0x0002, 0x08);
    machine.bus.io_write_byte(0x0002, 0x80);
    machine.bus.io_write_byte(0x0002, 0x01);
    machine.bus.io_write_byte(0x0002, 0x00);

    // IVT vector 0x09 (IRQ 1) -> 0x0000:0x1200.
    machine.bus.write_word(0x09 * 4, 0x1200);
    machine.bus.write_word(0x09 * 4 + 2, 0x0000);

    // Handler: drain the two serial bytes from 0x0600 (clears the keyboard IRQ),
    // INC [0x1500], EOI the master PIC, IRET.
    let handler = [
        0xBA, 0x00, 0x06, // MOV DX, 0x0600
        0xEC, // IN AL, DX  (flag byte)
        0xEC, // IN AL, DX  (scancode)
        0xFE, 0x06, 0x00, 0x15, // INC byte [0x1500]
        0xB0, 0x20, 0xE6, 0x00, // MOV AL, 0x20 ; OUT 0x00, AL
        0xCF, // IRET
    ];
    load_code(&mut machine.bus, 0x1200, &handler);

    let main = [0xFB, 0xF4, 0xFA, 0xF4]; // STI ; HLT ; CLI ; HLT
    load_code(&mut machine.bus, 0x1000, &main);
    machine.bus.write_byte(0x1500, 0x00);

    // Enable the keyboard interrupt and inject an 'A' key press.
    machine.bus.io_write_byte(0x0604, 0x01);
    machine.push_keyboard_scancode(0x1E);

    machine
        .cpu
        .load_state(&real_mode_state(0, 0x1000, 0, 0x0F00));
    machine.run_for(1_000_000);

    assert_eq!(
        machine.bus.read_byte(0x1500),
        1,
        "the keyboard interrupt handler ran exactly once"
    );
    assert!(
        machine.cpu.halted(),
        "the CPU should rest halted after IRET"
    );
}
