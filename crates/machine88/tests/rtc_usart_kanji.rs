//! RTC, USART, kanji ROM windows, and dictionary ROM.

mod harness;

use harness::{build_machine_with_rom, build_machine_with_synthetic_roms};
use machine88::Pc8801Machine;

/// Fixed BCD time injected into the RTC: `[year, month<<4|dow, day, hour,
/// minute, second]` = 2026-03-03 (Monday) 14:30:45.
const RTC_TIME: [u8; 6] = [0x26, 0x31, 0x03, 0x14, 0x30, 0x45];

fn fixed_rtc_time() -> [u8; 6] {
    RTC_TIME
}

// Port 0x10 RTC command line bits and port 0x40 strobe bits, matching the bus
// wiring (C0/C1/C2 + DIN on 0x10; STB/CLK on 0x40; DOUT on 0x40 read bit 4).
const RTC_STB: u8 = 0x02;
const RTC_CLK: u8 = 0x04;
const RTC_DOUT: u8 = 0x10;

const RTC_CMD_REGISTER_SHIFT: u8 = 0x01;
const RTC_CMD_TIME_READ: u8 = 0x03;

/// Latches a parallel command and pulses STB to execute it (DATA phase -> STB
/// rising -> STB release).
fn rtc_command(machine: &mut Pc8801Machine, command: u8) {
    machine.bus.io_write(0x10, command);
    machine.bus.io_write(0x40, RTC_STB);
    machine.bus.io_write(0x40, 0x00);
}

/// Reads the current RTC serial-out bit from port 0x40.
fn rtc_dout(machine: &mut Pc8801Machine) -> u8 {
    u8::from(machine.bus.io_read(0x40) & RTC_DOUT != 0)
}

#[test]
fn rtc_time_read_shifts_out_host_time() {
    let mut machine = build_machine_with_rom(&[0u8; 0x8000]);
    machine.bus.set_host_local_time_fn(fixed_rtc_time);

    // Load the host time into the shift register, then enter shift mode.
    rtc_command(&mut machine, RTC_CMD_TIME_READ);
    rtc_command(&mut machine, RTC_CMD_REGISTER_SHIFT);

    // Bit 0 is available immediately; the remaining 47 come out on each CLK
    // rising edge. Every bit is captured before the following CLK-low DATA phase
    // overwrites its (already-read) position.
    let mut bits = [0u8; 48];
    bits[0] = rtc_dout(&mut machine);
    for bit in bits.iter_mut().skip(1) {
        machine.bus.io_write(0x40, RTC_CLK);
        *bit = rtc_dout(&mut machine);
        machine.bus.io_write(0x40, 0x00);
    }

    // The register shifts reg[7] (seconds) first down to reg[2] (year), LSB
    // first within each byte, so the 6 bytes reconstruct in reverse order.
    let mut reconstructed = [0u8; 6];
    for (index, bit) in bits.iter().enumerate() {
        reconstructed[5 - (index / 8)] |= bit << (index % 8);
    }
    assert_eq!(reconstructed, RTC_TIME);
}

/// Deterministic per-index byte for the synthetic kanji1 ROM.
fn kanji1_byte(index: usize) -> u8 {
    index as u8
}

/// Deterministic per-index byte for the synthetic kanji2 ROM (distinct from
/// kanji1 so a window confusing the two would be caught).
fn kanji2_byte(index: usize) -> u8 {
    !(index as u8)
}

/// Deterministic per-index byte for the synthetic dictionary ROM. Always
/// non-zero so it stays distinct from the zeroed ALU read path.
fn dictionary_byte(index: usize) -> u8 {
    (index as u8) | 0x01
}

#[test]
fn kanji_windows_read_rom_words() {
    let mut machine = build_machine_with_synthetic_roms(|roms| {
        for (index, byte) in roms.kanji1.iter_mut().enumerate() {
            *byte = kanji1_byte(index);
        }
        for (index, byte) in roms.kanji2.iter_mut().enumerate() {
            *byte = kanji2_byte(index);
        }
    });

    let code: u16 = 0x4000;
    // Level-1 kanji ROM (ports 0xE8/0xE9).
    machine.bus.io_write(0xE8, (code & 0xFF) as u8);
    machine.bus.io_write(0xE9, (code >> 8) as u8);
    assert_eq!(
        machine.bus.io_read(0xE8),
        kanji1_byte(code as usize * 2 + 1)
    );
    assert_eq!(machine.bus.io_read(0xE9), kanji1_byte(code as usize * 2));

    // Level-2 kanji ROM (ports 0xEC/0xED).
    machine.bus.io_write(0xEC, (code & 0xFF) as u8);
    machine.bus.io_write(0xED, (code >> 8) as u8);
    assert_eq!(
        machine.bus.io_read(0xEC),
        kanji2_byte(code as usize * 2 + 1)
    );
    assert_eq!(machine.bus.io_read(0xED), kanji2_byte(code as usize * 2));
}

#[test]
fn dictionary_window_reads_selected_bank() {
    let mut machine = build_machine_with_synthetic_roms(|roms| {
        for (index, byte) in roms.dictionary.iter_mut().enumerate() {
            *byte = dictionary_byte(index);
        }
    });

    // Enable the ALU-gated window (GVAM + GAM) and the dictionary (0xF1 bit 0
    // clear); the dictionary read must take priority over the ALU read.
    machine.bus.io_write(0x32, 0x40); // GVAM
    machine.bus.io_write(0x35, 0x80); // GAM

    let bank: u8 = 1;
    machine.bus.io_write(0xF0, bank);
    machine.bus.io_write(0xF1, 0x00); // dictionary enabled

    for &offset in &[0usize, 1, 0x100, 0x3FFF] {
        let value = machine.bus.peek_byte(0xC000 + offset as u16);
        let expected = dictionary_byte(bank as usize * 0x4000 + offset);
        assert_eq!(value, expected, "dictionary byte at offset {offset:#x}");
    }

    // Disabling the dictionary drops back to the ALU read path (not ROM).
    machine.bus.io_write(0xF1, 0x01);
    let alu_value = machine.bus.peek_byte(0xC000);
    assert_ne!(alu_value, dictionary_byte(bank as usize * 0x4000));
}

/// Builds a ROM that installs an IM 2 vector for the RXRDY source (level 0),
/// enables it via ports 0xE4/0xE6, and runs `EI; HALT` until the ISR has read
/// one received byte. The ISR reads port 0x20 (clearing RXRDY), stores the byte
/// at 0x9101, and increments the counter at 0x9100.
fn build_usart_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];

    let init: &[u8] = &[
        0xF3, // DI
        0x31, 0x00, 0xB0, // LD SP, 0xB000
        0x3E, 0x90, // LD A, 0x90 (vector page)
        0xED, 0x47, // LD I, A
        0xED, 0x5E, // IM 2
        0x21, 0x50, 0x00, // LD HL, 0x0050 (ISR)
        0x22, 0x00, 0x90, // LD (0x9000), HL  ; RXRDY (level 0) vector
        0xAF, // XOR A
        0x32, 0x00, 0x91, // LD (0x9100), A  ; counter = 0
        0x32, 0x01, 0x91, // LD (0x9101), A  ; received = 0
        0x3E, 0x08, // LD A, 0x08
        0xD3, 0xE4, // OUT (0xE4), A   ; all priority levels
        0x3E, 0x04, // LD A, 0x04
        0xD3, 0xE6, // OUT (0xE6), A   ; unmask RXRDY
        // loop (0x001F):
        0xFB, // EI
        0x76, // HALT
        0x3A, 0x00, 0x91, // LD A, (0x9100)
        0xFE, 0x01, // CP 1
        0x38, 0xF7, // JR C, loop (-9)
        0x18, 0xFE, // JR $ (-2)
    ];
    rom[..init.len()].copy_from_slice(init);

    let isr: &[u8] = &[
        0xDB, 0x20, // IN A, (0x20)   ; read serial data, clears RXRDY
        0x32, 0x01, 0x91, // LD (0x9101), A
        0x3A, 0x00, 0x91, // LD A, (0x9100)
        0x3C, // INC A
        0x32, 0x00, 0x91, // LD (0x9100), A
        0x3E, 0x08, // LD A, 0x08
        0xD3, 0xE4, // OUT (0xE4), A  ; re-arm priority
        0xFB, // EI
        0xED, 0x4D, // RETI
    ];
    rom[0x0050..0x0050 + isr.len()].copy_from_slice(isr);

    rom
}

#[test]
fn usart_rxrdy_interrupt_delivers_byte() {
    let mut machine = build_machine_with_rom(&build_usart_rom());

    // Run far enough to install the vector and settle into EI; HALT.
    machine.run_for(50_000);

    // No data yet: RXRDY clear.
    assert_eq!(machine.bus.io_read(0x21) & 0x02, 0);

    machine.bus.inject_serial_byte(0x5A);
    // RXRDY asserted after injection (status read does not consume it).
    assert_ne!(machine.bus.io_read(0x21) & 0x02, 0);

    const STEP: u64 = 50_000;
    const CAP: u64 = 4_000_000;
    let mut total = 0u64;
    while total < CAP {
        machine.run_for(STEP);
        total += STEP;
        if machine.bus.peek_byte(0x9100) >= 1 {
            break;
        }
    }

    assert!(machine.bus.peek_byte(0x9100) >= 1, "RXRDY ISR never ran");
    assert_eq!(machine.bus.peek_byte(0x9101), 0x5A, "wrong received byte");
    // The ISR's data read drained the FIFO, clearing RXRDY.
    assert_eq!(machine.bus.io_read(0x21) & 0x02, 0);
}
