//! TSP timing integration tests: the command/parameter protocol, the display
//! timing it derives, the VSYNC status bits, and the once-per-frame VSYNC IRQ,
//! all driven through the public bus surface.

use common::{Bus, Cpu};
use machine88va::Pc88VaMachine;

#[path = "common/harness.rs"]
mod harness;
use harness::*;

// TSP command codes.
const CMD_SYNC: u8 = 0x10;

// Status bits on port 0x142.
const STATUS_BUSY: u8 = 0x04;
const STATUS_VB: u8 = 0x40;

// Default 24.8 kHz / 400-line frame period, derived exactly the way the TSP
// does it (see tsp_updateclock): with no SYNC programmed the reset default
// parameters apply.
//   w = (16+1 + 0 + 159+1 + 0 + 16+1 + 15+1) * 4 = 840
//   h = 25 + 0 + 400 + 0 + 7 + 8 = 440
//   hclock = 20_854_022 / 840 = 24826
//   cnt = 7_987_200 * 440 / 24826 = 141_559
const DEFAULT_FRAME_PERIOD: u64 = 141_559;

/// Advances the bus to the next scheduled event and returns that cycle.
fn step_to_next_event(machine: &mut Pc88VaMachine) -> u64 {
    let next = machine
        .bus
        .next_event_cycle()
        .expect("an event is always scheduled");
    machine.bus.set_current_cycle(next);
    next
}

/// Issues a 14-byte SYNC command through ports 0x142 / 0x146.
fn issue_sync(machine: &mut Pc88VaMachine, params: [u8; 14]) {
    machine.bus.io_write_byte(0x142, CMD_SYNC);
    for byte in params {
        machine.bus.io_write_byte(0x146, byte);
    }
}

#[test]
fn status_reports_busy_during_command_then_ready() {
    let mut machine = machine();

    // At reset the VSYNC phase is inactive, so VB is clear.
    assert_eq!(machine.bus.io_read_byte(0x142) & STATUS_VB, 0);

    machine.bus.io_write_byte(0x142, CMD_SYNC);
    assert_eq!(machine.bus.io_read_byte(0x142) & STATUS_BUSY, STATUS_BUSY);

    // SYNC needs 14 parameters; BUSY stays set until the last one arrives.
    for byte in 0u8..13 {
        machine.bus.io_write_byte(0x146, byte);
        assert_eq!(machine.bus.io_read_byte(0x142) & STATUS_BUSY, STATUS_BUSY);
    }
    machine.bus.io_write_byte(0x146, 13);
    assert_eq!(machine.bus.io_read_byte(0x142) & STATUS_BUSY, 0);
}

#[test]
fn unknown_command_clears_busy() {
    let mut machine = machine();
    // EXIT (0x88) takes no parameters and must leave the TSP ready.
    machine.bus.io_write_byte(0x142, 0x88);
    assert_eq!(machine.bus.io_read_byte(0x142) & STATUS_BUSY, 0);
}

#[test]
fn parameter_port_143_reads_open() {
    let mut machine = machine();
    assert_eq!(machine.bus.io_read_byte(0x143), 0xFF);
}

#[test]
fn vsync_status_bit_toggles_across_the_frame() {
    let mut machine = machine();

    // The TSP VB bit (port 0x142 bit 6) is set during the VSYNC phase and clear
    // during the display phase. Step through events and confirm it changes.
    let mut saw_set = false;
    let mut saw_clear = false;
    for _ in 0..64 {
        step_to_next_event(&mut machine);
        if machine.bus.io_read_byte(0x142) & STATUS_VB != 0 {
            saw_set = true;
        } else {
            saw_clear = true;
        }
        if saw_set && saw_clear {
            break;
        }
    }
    assert!(saw_set, "VB bit never set");
    assert!(saw_clear, "VB bit never cleared");
}

#[test]
fn system_port_4_vsync_bit_toggles() {
    let mut machine = machine();

    // Port 0x040 bit 5 reflects the system-port-4 VSYNC window.
    let mut saw_set = false;
    let mut saw_clear = false;
    for _ in 0..64 {
        step_to_next_event(&mut machine);
        if machine.bus.io_read_byte(0x040) & 0x20 != 0 {
            saw_set = true;
        } else {
            saw_clear = true;
        }
        if saw_set && saw_clear {
            break;
        }
    }
    assert!(saw_set, "system-port-4 VSYNC bit never set");
    assert!(saw_clear, "system-port-4 VSYNC bit never cleared");
}

#[test]
fn tsp_vb_bit_period_matches_default_timing() {
    let mut machine = machine();

    // Collect the cycles at which the VB bit rises (once per frame).
    let mut edges = Vec::new();
    let mut previous = false;
    for _ in 0..4000 {
        let cycle = step_to_next_event(&mut machine);
        let active = machine.bus.io_read_byte(0x142) & STATUS_VB != 0;
        if active && !previous {
            edges.push(cycle);
        }
        previous = active;
        if edges.len() >= 4 {
            break;
        }
    }
    assert!(edges.len() >= 3, "too few VSYNC edges observed: {edges:?}");
    for window in edges.windows(2) {
        assert_eq!(window[1] - window[0], DEFAULT_FRAME_PERIOD);
    }
}

#[test]
fn vsync_irq_fires_once_per_frame() {
    let mut machine = machine();
    // Unmask master IRQ2 (VSYNC); keep the rest masked.
    machine.bus.io_write_byte(0x18A, 0xFB);

    let mut edges = Vec::new();
    let mut previous = false;
    for _ in 0..4000 {
        let cycle = step_to_next_event(&mut machine);
        let pending = machine.bus.has_irq();
        if pending && !previous {
            edges.push(cycle);
        }
        previous = pending;
        if edges.len() >= 4 {
            break;
        }
    }
    assert!(edges.len() >= 3, "too few VSYNC IRQs observed: {edges:?}");
    for window in edges.windows(2) {
        assert_eq!(window[1] - window[0], DEFAULT_FRAME_PERIOD);
    }
}

#[test]
fn vsync_irq_acknowledges_as_vector_0x0a() {
    let mut machine = machine();
    machine.bus.io_write_byte(0x18A, 0xFB);

    for _ in 0..256 {
        step_to_next_event(&mut machine);
        if machine.bus.has_irq() {
            assert_eq!(machine.bus.acknowledge_irq(), 0x0A);
            return;
        }
    }
    panic!("VSYNC IRQ never fired");
}

#[test]
fn changing_sync_line_count_changes_the_frame_period() {
    let mut machine = machine();

    // Program a SYNC with half the vertical active lines (200 instead of 400),
    // keeping the default horizontal timing. The frame period must shrink.
    let mut params = [0u8; 14];
    params[0x02] = 0x10; // lbl
    params[0x03] = 0x00; // lbr
    params[0x04] = 0x9F; // had
    params[0x05] = 0x00; // rbr
    params[0x06] = 0x10; // rbl
    params[0x07] = 0x0F; // hs
    params[0x08] = 0x19; // tbl
    params[0x09] = 0x00; // tbr
    params[0x0A] = 0xC8; // vad low = 200
    params[0x0B] = 0x00; // vad high / bbr
    params[0x0C] = 0x07; // bbl
    params[0x0D] = 0x08; // vs
    issue_sync(&mut machine, params);

    // Measure two VB rising edges after the new timing takes effect.
    let mut edges = Vec::new();
    let mut previous = false;
    for _ in 0..4000 {
        let cycle = step_to_next_event(&mut machine);
        let active = machine.bus.io_read_byte(0x142) & STATUS_VB != 0;
        if active && !previous {
            edges.push(cycle);
        }
        previous = active;
        if edges.len() >= 3 {
            break;
        }
    }
    assert!(edges.len() >= 2, "too few VSYNC edges observed: {edges:?}");
    let period = edges[edges.len() - 1] - edges[edges.len() - 2];
    assert!(
        period < DEFAULT_FRAME_PERIOD,
        "200-line period {period} not shorter than 400-line {DEFAULT_FRAME_PERIOD}"
    );
}

#[test]
fn halted_v30_wakes_on_vsync_irq() {
    let mut machine = machine();

    // INT 0x0A vector (IRQ2) -> 0x0000:0x0100.
    machine.bus.write_byte(0x28, 0x00);
    machine.bus.write_byte(0x29, 0x01);
    machine.bus.write_byte(0x2A, 0x00);
    machine.bus.write_byte(0x2B, 0x00);

    // Handler at 0x0100: MOV AL, 0x99 ; HLT.
    machine.bus.write_byte(0x0100, 0xB0);
    machine.bus.write_byte(0x0101, 0x99);
    machine.bus.write_byte(0x0102, 0xF4);

    // Main program at 0x0400: STI ; HLT.
    machine.bus.write_byte(0x0400, 0xFB);
    machine.bus.write_byte(0x0401, 0xF4);

    machine.bus.io_write_byte(0x18A, 0xFB);

    machine.cpu.set_cs(0x0000);
    machine.cpu.set_ip(0x0400);

    machine.run_for(400_000);

    assert_eq!(machine.cpu.ax() & 0xFF, 0x99);
}
