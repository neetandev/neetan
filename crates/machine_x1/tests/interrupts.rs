//! Z80 mode-2 daisy-chain interrupt tests.

mod harness;

use harness::{build_machine, fire_next_event, run_bus_cycles};
use machine_x1::X1Model;

/// CTC control word: control-word select, interrupt enable, timer mode,
/// prescaler 16, time-constant follows.
const CTC_TIMER_INT: u8 = 0x85;

#[test]
fn ctc_zero_count_delivers_its_daisy_chain_vector() {
    let mut machine = build_machine(X1Model::X1);
    let bus = &mut machine.bus;

    // Channel 0 vector base 0xE0, then a short interrupting timer. The base X1
    // wires the CTC at 0x1FA8-0x1FAB.
    bus.io_write(0x1FA8, 0xE0);
    bus.io_write(0x1FA8, CTC_TIMER_INT);
    bus.io_write(0x1FA8, 0x01); // time constant 1 -> 16 cycles

    let mut vector = None;
    for _ in 0..8 {
        if let Some(v) = fire_next_event(bus) {
            vector = Some(v);
            break;
        }
    }
    assert_eq!(vector, Some(0xE0));
}

#[test]
fn turbo_daisy_chain_prioritises_sio_over_ctc() {
    let mut machine = build_machine(X1Model::X1Turbo);
    let bus = &mut machine.bus;

    // Arm a mouse (SIO channel 1) receive interrupt: WR2 vector 0x30, WR1 rx int
    // on all chars + status affects vector, then an RTS edge to load a report.
    bus.io_write(0x1F93, 0x02);
    bus.io_write(0x1F93, 0x30);
    bus.io_write(0x1F93, 0x01);
    bus.io_write(0x1F93, 0x1C);
    bus.set_mouse_input(1, 1, 0x00);
    bus.io_write(0x1F93, 0x05);
    bus.io_write(0x1F93, 0x02);
    assert!(bus.has_irq());

    // Arm a short CTC channel 0 timer with vector base 0xE0.
    bus.io_write(0x1FA0, 0xE0);
    bus.io_write(0x1FA0, CTC_TIMER_INT);
    bus.io_write(0x1FA0, 0x01); // time constant 1 -> 16 cycles

    // Advance just past the CTC zero count so both the CTC and SIO are pending.
    run_bus_cycles(bus, 32);
    assert!(bus.has_irq());

    // The SIO outranks the CTC in the daisy chain, so its receive interrupt
    // acknowledges first, vectored through channel B.
    let mut vector = bus.acknowledge_irq();
    assert_eq!(vector, 0x34);
    // While the SIO is under service it holds the chain, so the lower-priority
    // CTC stays blocked until the SIO handler returns.
    assert!(!bus.has_irq());
    // Each remaining mouse-report byte re-interrupts ahead of the CTC; drain
    // the receive FIFO the way a handler would.
    for _ in 0..8 {
        if vector != 0x34 {
            break;
        }
        let _ = bus.io_read(0x1F92);
        bus.notify_reti();
        vector = bus.acknowledge_irq();
    }
    // The CTC zero count follows once the receive FIFO is drained.
    assert_eq!(vector, 0xE0);
}

#[test]
fn sound_ctc_zero_count_delivers_its_daisy_chain_vector() {
    let mut machine = build_machine(X1Model::X1Turbo);
    let bus = &mut machine.bus;

    // Program the sound-board CTC channel 0 (port 0x0704): vector base 0x40,
    // then an interrupting timer with a short time constant.
    bus.io_write(0x0704, 0x40);
    bus.io_write(0x0704, CTC_TIMER_INT);
    bus.io_write(0x0704, 0x01); // time constant 1 -> 16 cycles

    let mut vector = None;
    for _ in 0..8 {
        if let Some(v) = fire_next_event(bus) {
            vector = Some(v);
            break;
        }
    }
    assert_eq!(vector, Some(0x40));
}

#[test]
fn sound_ctc_outranks_main_ctc() {
    let mut machine = build_machine(X1Model::X1Turbo);
    let bus = &mut machine.bus;

    // Arm the main CTC channel 0 (vector 0xE0) and the sound-board CTC channel 0
    // (vector 0x40) with identical short timers.
    bus.io_write(0x1FA0, 0xE0);
    bus.io_write(0x1FA0, CTC_TIMER_INT);
    bus.io_write(0x1FA0, 0x01);
    bus.io_write(0x0704, 0x40);
    bus.io_write(0x0704, CTC_TIMER_INT);
    bus.io_write(0x0704, 0x01);

    run_bus_cycles(bus, 32);
    assert!(bus.has_irq());

    // The sound-board CTC heads the daisy chain, so it acknowledges first and
    // then holds the chain until its handler returns.
    assert_eq!(bus.acknowledge_irq(), 0x40);
    assert!(!bus.has_irq());
    bus.notify_reti();
    assert_eq!(bus.acknowledge_irq(), 0xE0);
}

#[test]
fn reti_discards_a_ctc_zero_count_latched_during_service() {
    // A CTC handler that runs longer than its own timer period must not be
    // re-entered back-to-back: the zero count that occurred while the channel
    // was under service is discarded on RETI, giving the main program the rest
    // of the period. Arcus's fast screen wipe (a 976 Hz timer with a handler
    // several periods long) hangs without this.
    let mut machine = build_machine(X1Model::X1);
    let bus = &mut machine.bus;

    bus.io_write(0x1FA8, 0xE0);
    bus.io_write(0x1FA8, CTC_TIMER_INT);
    bus.io_write(0x1FA8, 0x01); // time constant 1 -> 16 cycles

    run_bus_cycles(bus, 32);
    assert!(bus.has_irq());
    assert_eq!(bus.acknowledge_irq(), 0xE0);
    assert!(!bus.has_irq());

    // The timer keeps running and hits zero again while under service.
    run_bus_cycles(bus, 32);
    assert!(!bus.has_irq(), "the source under service holds the chain");

    // RETI drops the request latched during service instead of re-firing.
    bus.notify_reti();
    assert!(!bus.has_irq(), "the stale zero count is discarded on RETI");

    // The next fresh zero count interrupts normally again.
    run_bus_cycles(bus, 32);
    assert!(bus.has_irq());
    assert_eq!(bus.acknowledge_irq(), 0xE0);
}

#[test]
fn keyboard_interrupt_delivers_the_programmed_vector() {
    let mut machine = build_machine(X1Model::X1);
    let bus = &mut machine.bus;

    // Enable the key interrupt with vector 0x20 (command 0xE4, one parameter).
    bus.io_write(0x1900, 0xE4);
    run_bus_cycles(bus, 4_000);
    bus.io_write(0x1900, 0x20);
    run_bus_cycles(bus, 4_000);

    // Press 'A'; the sub-CPU raises the keyboard interrupt on its next poll.
    bus.push_keyboard_scancode(0x41);
    run_bus_cycles(bus, 8_000);

    assert!(bus.has_irq());
    assert_eq!(bus.acknowledge_irq(), 0x20);
}
