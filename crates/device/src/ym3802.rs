//! Yamaha YM3802 MCS MIDI communication and service controller.
//!
//! Implements the transmit path used by the CZ-6BM1 MIDI board: the fixed
//! and group-banked register file, the 16-byte transmit FIFO with serial
//! frame pacing, the general and MIDI-clock timers with the click counter,
//! and vectored interrupt reporting. Receive, FSK, and sequencer functions
//! accept register access but stay inert.

use std::collections::VecDeque;

/// CLKM master-clock frequency supplied to the chip on the CZ-6BM1, in Hz.
pub const YM3802_CLKM_HZ: u64 = 1_000_000;

/// CLKF sync-clock frequency supplied to the chip on the CZ-6BM1, in Hz.
const CLKF_HZ: u64 = 614_400;
/// Transmit FIFO depth in bytes.
const TRANSMIT_FIFO_DEPTH: usize = 16;

/// Interrupt-status bit for a completed general-timer count.
const INTERRUPT_GENERAL_TIMER: u8 = 0x80;
/// Interrupt-status bit for the transmit FIFO becoming empty.
const INTERRUPT_TRANSMIT_EMPTY: u8 = 0x40;
/// Interrupt-status bit shared by the click counter and MIDI-clock detect.
const INTERRUPT_CLICK_OR_MIDI_CLOCK: u8 = 0x02;
/// Interrupt cause code reported when no interrupt is pending.
const INTERRUPT_CAUSE_NONE: u8 = 8;

/// Register-group selection bits of the system-control register.
const CONTROL_GROUP_MASK: u8 = 0x0F;
/// Initial-clear request bit of the system-control register.
const CONTROL_INITIAL_CLEAR: u8 = 0x80;
/// Interrupt-mode bit selecting the MIDI-clock detect cause over the click
/// counter for the shared interrupt-status bit.
const MODE_MIDI_CLOCK_SELECT: u8 = 0x08;

/// Transmit-status bit reporting an empty FIFO.
const TRANSMIT_STATUS_EMPTY: u8 = 0x80;
/// Transmit-status bit reporting room for another FIFO byte.
const TRANSMIT_STATUS_READY: u8 = 0x40;
/// Transmit-status bit reporting a fully idle transmitter.
const TRANSMIT_STATUS_IDLE: u8 = 0x04;
/// Transmit-status bit reporting an active serial shifter.
const TRANSMIT_STATUS_BUSY: u8 = 0x01;
/// Transmit-control bit clearing the FIFO contents.
const TRANSMIT_CONTROL_CLEAR: u8 = 0x80;
/// Transmit-control bit enabling the transmitter.
const TRANSMIT_CONTROL_ENABLE: u8 = 0x01;

/// Click-counter control bit declaring a 1.0 MHz CLKM input.
const CLICK_CONTROL_CLKM_1MHZ: u8 = 0x02;
/// Immediate-load request bit shared by the timer high bytes and the click
/// counter value register.
const TIMER_LOAD_REQUEST: u8 = 0x80;

/// Yamaha YM3802 MIDI controller with a transmit-only serial connection.
#[derive(Debug, Default)]
pub struct Ym3802 {
    current_tick: u64,
    group: u8,
    last_written: u8,
    interrupt_vector_offset: u8,
    interrupt_mode: u8,
    interrupt_enable: u8,
    interrupt_status: u8,
    transmit_rate: u8,
    transmit_mode: u8,
    transmit_enabled: bool,
    transmit_fifo: VecDeque<u8>,
    transmit_in_flight: Option<InFlightByte>,
    general_timer_reload: u16,
    general_timer_deadline: Option<u64>,
    midi_clock_reload: u16,
    midi_clock_deadline: Option<u64>,
    click_counter_reload: u8,
    click_counter: u8,
    click_control: u8,
    external_io_direction: u8,
    external_io_output: u8,
    capture_enabled: bool,
    captured: Vec<u8>,
}

/// A byte inside the serial shifter and its completion deadline.
#[derive(Debug, Clone, Copy)]
struct InFlightByte {
    value: u8,
    completion_tick: u64,
}

impl Ym3802 {
    /// Creates a chip in its power-on state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies a hardware reset from the RESET pin or the initial-clear bit.
    ///
    /// Bytes that already finished transmitting stay captured; only the
    /// untransmitted FIFO contents are discarded.
    pub fn reset(&mut self) {
        self.group = 0;
        self.interrupt_vector_offset = 0;
        self.interrupt_mode = 0;
        self.interrupt_enable = 0;
        self.interrupt_status = 0;
        self.transmit_rate = 0;
        self.transmit_mode = 0;
        self.transmit_enabled = false;
        self.transmit_fifo.clear();
        self.transmit_in_flight = None;
        self.general_timer_reload = 0;
        self.general_timer_deadline = None;
        self.midi_clock_reload = 0;
        self.midi_clock_deadline = None;
        self.click_counter_reload = 0;
        self.click_counter = 0;
        self.click_control = 0;
        self.external_io_direction = 0;
        self.external_io_output = 0;
    }

    /// Reads the register selected by address bits 3:1 of the odd byte lane.
    pub fn read_register(&mut self, offset: u8, tick: u64) -> u8 {
        self.advance_to(tick);
        match offset & 7 {
            0 => self.interrupt_vector(),
            2 => self.interrupt_status,
            1 | 3 => self.last_written,
            offset => self.read_banked_register(offset),
        }
    }

    /// Writes the register selected by address bits 3:1 of the odd byte lane.
    pub fn write_register(&mut self, offset: u8, value: u8, tick: u64) {
        self.advance_to(tick);
        self.last_written = value;
        match offset & 7 {
            0 | 2 => {}
            1 => {
                self.group = value & CONTROL_GROUP_MASK;
                if value & CONTROL_INITIAL_CLEAR != 0 {
                    self.reset();
                }
            }
            3 => self.interrupt_status &= !value,
            offset => self.write_banked_register(offset, value),
        }
    }

    /// Advances transmit and timer state to the given CLKM tick.
    pub fn advance_to(&mut self, tick: u64) {
        while let Some(deadline) = self.next_event_tick() {
            if deadline > tick {
                break;
            }
            self.current_tick = deadline;
            if let Some(in_flight) = self.transmit_in_flight
                && in_flight.completion_tick == deadline
            {
                self.transmit_in_flight = None;
                if self.capture_enabled {
                    self.captured.push(in_flight.value);
                }
                self.load_transmit_byte();
            }
            if self.general_timer_deadline == Some(deadline) {
                self.general_timer_deadline = self.timer_deadline(self.general_timer_reload);
                self.raise_interrupt(INTERRUPT_GENERAL_TIMER);
            }
            if self.midi_clock_deadline == Some(deadline) {
                self.midi_clock_deadline = self.timer_deadline(self.midi_clock_reload);
                self.on_midi_clock_expiry();
            }
        }
        self.current_tick = self.current_tick.max(tick);
    }

    /// Returns the earliest transmit or timer deadline in CLKM ticks.
    pub fn next_event_tick(&self) -> Option<u64> {
        [
            self.transmit_in_flight
                .map(|in_flight| in_flight.completion_tick),
            self.general_timer_deadline,
            self.midi_clock_deadline,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Reports whether the interrupt request line is asserted.
    pub fn irq_asserted(&self) -> bool {
        self.interrupt_status != 0
    }

    /// Returns the current interrupt vector during an acknowledge cycle.
    ///
    /// The pending causes stay set; the guest clears them through the
    /// interrupt-clear register.
    pub fn acknowledge_interrupt(&mut self) -> Option<u8> {
        self.irq_asserted().then(|| self.interrupt_vector())
    }

    /// Enables capture of transmitted MIDI bytes.
    pub fn enable_midi_capture(&mut self) {
        self.capture_enabled = true;
    }

    /// Drains captured transmit bytes into `out`.
    pub fn flush_midi_into(&mut self, out: &mut Vec<u8>) {
        out.append(&mut self.captured);
    }

    /// Reads a group-banked register at offsets 4 through 7.
    fn read_banked_register(&mut self, offset: u8) -> u8 {
        if !banked_register_exists(self.group, offset) {
            return self.last_written;
        }
        match (self.group, offset) {
            (0, 4) => self.interrupt_vector_offset,
            (0, 5) => self.interrupt_mode,
            (0, 6) => self.interrupt_enable,
            (4, 4) => self.transmit_rate,
            (4, 5) => self.transmit_mode,
            (5, 4) => self.transmit_status(),
            (6, 6) => self.click_control,
            (6, 7) => self.click_counter_reload,
            (8, 4) => (self.general_timer_reload & 0xFF) as u8,
            (8, 5) => (self.general_timer_reload >> 8) as u8,
            (8, 6) => (self.midi_clock_reload & 0xFF) as u8,
            (8, 7) => (self.midi_clock_reload >> 8) as u8,
            (9, 4) => self.external_io_direction,
            (9, 5) => self.external_io_output,
            _ => 0,
        }
    }

    /// Writes a group-banked register at offsets 4 through 7.
    fn write_banked_register(&mut self, offset: u8, value: u8) {
        if !banked_register_exists(self.group, offset) {
            return;
        }
        match (self.group, offset) {
            (0, 4) => self.interrupt_vector_offset = value,
            (0, 5) => self.interrupt_mode = value,
            (0, 6) => self.interrupt_enable = value,
            (4, 4) => self.transmit_rate = value,
            (4, 5) => self.transmit_mode = value,
            (5, 5) => self.write_transmit_control(value),
            (5, 6) => self.write_transmit_data(value),
            (6, 6) => self.click_control = value,
            (6, 7) => {
                self.click_counter_reload = value & 0x7F;
                if value & TIMER_LOAD_REQUEST != 0 {
                    self.click_counter = self.click_counter_reload;
                }
            }
            (8, 4) => {
                self.general_timer_reload = (self.general_timer_reload & 0x3F00) | u16::from(value);
            }
            (8, 5) => {
                self.general_timer_reload =
                    (self.general_timer_reload & 0x00FF) | (u16::from(value & 0x3F) << 8);
                if value & TIMER_LOAD_REQUEST != 0 {
                    self.general_timer_deadline = self.timer_deadline(self.general_timer_reload);
                }
            }
            (8, 6) => {
                self.midi_clock_reload = (self.midi_clock_reload & 0x3F00) | u16::from(value);
            }
            (8, 7) => {
                self.midi_clock_reload =
                    (self.midi_clock_reload & 0x00FF) | (u16::from(value & 0x3F) << 8);
                if value & TIMER_LOAD_REQUEST != 0 {
                    self.midi_clock_deadline = self.timer_deadline(self.midi_clock_reload);
                }
            }
            (9, 4) => self.external_io_direction = value,
            (9, 5) => self.external_io_output = value,
            _ => {}
        }
    }

    /// Applies a transmit-control write: FIFO clear and transmitter enable.
    fn write_transmit_control(&mut self, value: u8) {
        if value & TRANSMIT_CONTROL_CLEAR != 0 {
            self.transmit_fifo.clear();
        }
        self.transmit_enabled = value & TRANSMIT_CONTROL_ENABLE != 0;
        if self.transmit_enabled {
            self.load_transmit_byte();
        }
    }

    /// Queues a byte for transmission, dropping it when the FIFO is full.
    fn write_transmit_data(&mut self, value: u8) {
        if self.transmit_fifo.len() < TRANSMIT_FIFO_DEPTH {
            self.transmit_fifo.push_back(value);
        }
        self.interrupt_status &= !INTERRUPT_TRANSMIT_EMPTY;
        if self.transmit_enabled {
            self.load_transmit_byte();
        }
    }

    /// Moves the next FIFO byte into the serial shifter when it is free.
    fn load_transmit_byte(&mut self) {
        if !self.transmit_enabled || self.transmit_in_flight.is_some() {
            return;
        }
        let Some(value) = self.transmit_fifo.pop_front() else {
            return;
        };
        let duration = self.frame_bits() * self.ticks_per_bit();
        self.transmit_in_flight = Some(InFlightByte {
            value,
            completion_tick: self.current_tick + duration,
        });
        if self.transmit_fifo.is_empty() {
            self.raise_interrupt(INTERRUPT_TRANSMIT_EMPTY);
        }
    }

    /// Handles a MIDI-clock timer expiry: MIDI-clock detect or click count.
    fn on_midi_clock_expiry(&mut self) {
        if self.interrupt_mode & MODE_MIDI_CLOCK_SELECT != 0 {
            self.raise_interrupt(INTERRUPT_CLICK_OR_MIDI_CLOCK);
            return;
        }
        if self.click_counter_reload == 0 {
            return;
        }
        if self.click_counter == 0 {
            self.click_counter = self.click_counter_reload;
        }
        self.click_counter -= 1;
        if self.click_counter == 0 {
            self.click_counter = self.click_counter_reload;
            self.raise_interrupt(INTERRUPT_CLICK_OR_MIDI_CLOCK);
        }
    }

    /// Latches an interrupt cause gated by the interrupt-enable register.
    fn raise_interrupt(&mut self, cause: u8) {
        self.interrupt_status |= cause & self.interrupt_enable;
    }

    /// Builds the interrupt vector from the programmed offset and the
    /// highest-priority pending cause.
    fn interrupt_vector(&self) -> u8 {
        let cause = if self.interrupt_status == 0 {
            INTERRUPT_CAUSE_NONE
        } else {
            self.interrupt_status.trailing_zeros() as u8
        };
        (self.interrupt_vector_offset & 0xE0) | (cause << 1)
    }

    /// Computes the transmit FIFO status byte.
    fn transmit_status(&self) -> u8 {
        let mut status = 0;
        if self.transmit_fifo.is_empty() {
            status |= TRANSMIT_STATUS_EMPTY;
        }
        if self.transmit_fifo.len() < TRANSMIT_FIFO_DEPTH {
            status |= TRANSMIT_STATUS_READY;
        }
        if self.transmit_fifo.is_empty() && self.transmit_in_flight.is_none() {
            status |= TRANSMIT_STATUS_IDLE;
        }
        if self.transmit_in_flight.is_some() {
            status |= TRANSMIT_STATUS_BUSY;
        }
        status
    }

    /// Returns the serial bit period in CLKM ticks for the selected rate.
    fn ticks_per_bit(&self) -> u64 {
        let selection = self.transmit_rate & 0x1F;
        match selection >> 3 {
            0b00 => 16,
            0b01 => 32,
            _ => {
                let clkf_divisor: u64 = match selection {
                    0x10..=0x13 => 32,
                    0x14 => 64,
                    0x15 => 128,
                    0x16 => 256,
                    0x17 => 512,
                    0x18 => 1024,
                    0x19 => 2048,
                    0x1A => 4096,
                    _ => 8192,
                };
                (YM3802_CLKM_HZ * clkf_divisor + CLKF_HZ / 2) / CLKF_HZ
            }
        }
    }

    /// Returns the serial frame length in bits for the selected mode.
    fn frame_bits(&self) -> u64 {
        let data_bits = if self.transmit_mode & 0x20 != 0 { 7 } else { 8 };
        let parity_bits = if self.transmit_mode & 0x10 != 0 {
            if self.transmit_mode & 0x08 != 0 { 4 } else { 1 }
        } else {
            0
        };
        let stop_bits = if self.transmit_mode & 0x02 != 0 { 2 } else { 1 };
        1 + data_bits + parity_bits + stop_bits
    }

    /// Returns the next expiry for a timer reload, or `None` when disabled.
    ///
    /// Both timers count at 125 kHz when the click-counter control declares
    /// the 1.0 MHz CLKM input; reload values below two leave the timer idle.
    fn timer_deadline(&self, reload: u16) -> Option<u64> {
        if reload < 2 {
            return None;
        }
        let prescaler: u64 = if self.click_control & CLICK_CONTROL_CLKM_1MHZ != 0 {
            8
        } else {
            4
        };
        Some(self.current_tick + u64::from(reload) * prescaler)
    }
}

/// Reports whether a banked register exists at the given group and offset.
const fn banked_register_exists(group: u8, offset: u8) -> bool {
    match offset {
        4 | 5 => group <= 9,
        6 => group <= 9 && group != 4,
        7 => matches!(group, 1 | 2 | 6 | 7 | 8),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The CLKM/32 rate selection producing the 31.25 kbit/s MIDI rate.
    const MIDI_RATE_SELECTION: u8 = 0x08;

    fn chip() -> Ym3802 {
        let mut chip = Ym3802::new();
        chip.enable_midi_capture();
        chip
    }

    fn select_group(chip: &mut Ym3802, group: u8, tick: u64) {
        chip.write_register(1, group, tick);
    }

    fn enable_transmitter_at_midi_rate(chip: &mut Ym3802, tick: u64) {
        select_group(chip, 4, tick);
        chip.write_register(4, MIDI_RATE_SELECTION, tick);
        select_group(chip, 5, tick);
        chip.write_register(5, TRANSMIT_CONTROL_ENABLE, tick);
    }

    fn queue_byte(chip: &mut Ym3802, value: u8, tick: u64) {
        select_group(chip, 5, tick);
        chip.write_register(6, value, tick);
    }

    fn captured(chip: &mut Ym3802) -> Vec<u8> {
        let mut bytes = Vec::new();
        chip.flush_midi_into(&mut bytes);
        bytes
    }

    #[test]
    fn power_on_status_reads_empty_and_ready() {
        let mut chip = chip();
        select_group(&mut chip, 5, 0);
        assert_eq!(
            chip.read_register(4, 0),
            TRANSMIT_STATUS_EMPTY | TRANSMIT_STATUS_READY | TRANSMIT_STATUS_IDLE
        );
        assert_eq!(chip.read_register(2, 0), 0);
        assert_eq!(chip.read_register(0, 0), 0x10);
    }

    #[test]
    fn group_select_banks_offset_registers() {
        let mut chip = chip();
        select_group(&mut chip, 0, 0);
        chip.write_register(4, 0x40, 0);
        assert_eq!(chip.read_register(0, 0), 0x50);
        select_group(&mut chip, 5, 0);
        chip.write_register(6, 0x90, 0);
        select_group(&mut chip, 4, 0);
        assert_eq!(chip.read_register(4, 0), 0);
    }

    #[test]
    fn group_above_nine_ignores_banked_access() {
        let mut chip = chip();
        select_group(&mut chip, 0x0A, 0);
        chip.write_register(4, 0x40, 0);
        assert_eq!(chip.read_register(4, 0), 0x40);
        select_group(&mut chip, 0, 0);
        assert_eq!(chip.read_register(4, 0), 0);
    }

    #[test]
    fn ic_reset_bit_clears_fifo_timers_and_interrupts() {
        let mut chip = chip();
        enable_transmitter_at_midi_rate(&mut chip, 0);
        queue_byte(&mut chip, 0x90, 0);
        queue_byte(&mut chip, 0x40, 0);
        select_group(&mut chip, 8, 0);
        chip.write_register(4, 100, 0);
        chip.write_register(5, TIMER_LOAD_REQUEST, 0);
        chip.write_register(1, CONTROL_INITIAL_CLEAR, 0);
        assert_eq!(chip.next_event_tick(), None);
        assert_eq!(chip.read_register(2, 0), 0);
        select_group(&mut chip, 5, 0);
        assert_eq!(
            chip.read_register(4, 0),
            TRANSMIT_STATUS_EMPTY | TRANSMIT_STATUS_READY | TRANSMIT_STATUS_IDLE
        );
    }

    #[test]
    fn fifo_holds_sixteen_bytes_and_drops_overflow() {
        let mut chip = chip();
        select_group(&mut chip, 4, 0);
        chip.write_register(4, MIDI_RATE_SELECTION, 0);
        select_group(&mut chip, 5, 0);
        for value in 0..16 {
            assert_ne!(chip.read_register(4, 0) & TRANSMIT_STATUS_READY, 0);
            chip.write_register(6, value, 0);
        }
        assert_eq!(chip.read_register(4, 0) & TRANSMIT_STATUS_READY, 0);
        chip.write_register(6, 0xFF, 0);
        chip.write_register(5, TRANSMIT_CONTROL_ENABLE, 0);
        chip.advance_to(320 * 17);
        let expected: Vec<u8> = (0..16).collect();
        assert_eq!(captured(&mut chip), expected);
    }

    #[test]
    fn transmit_disabled_holds_queued_bytes() {
        let mut chip = chip();
        queue_byte(&mut chip, 0x90, 0);
        chip.advance_to(1_000_000);
        assert!(captured(&mut chip).is_empty());
        select_group(&mut chip, 5, 1_000_000);
        assert_eq!(chip.read_register(4, 1_000_000) & TRANSMIT_STATUS_EMPTY, 0);
    }

    #[test]
    fn programmed_rate_paces_ten_bit_frames_at_midi_speed() {
        let mut chip = chip();
        enable_transmitter_at_midi_rate(&mut chip, 0);
        for value in [0x90, 0x40, 0x7F] {
            queue_byte(&mut chip, value, 0);
        }
        chip.advance_to(319);
        assert!(captured(&mut chip).is_empty());
        chip.advance_to(320);
        assert_eq!(captured(&mut chip), vec![0x90]);
        chip.advance_to(960);
        assert_eq!(captured(&mut chip), vec![0x40, 0x7F]);
    }

    #[test]
    fn default_rate_transmits_at_double_speed() {
        let mut chip = chip();
        select_group(&mut chip, 5, 0);
        chip.write_register(5, TRANSMIT_CONTROL_ENABLE, 0);
        queue_byte(&mut chip, 0xFE, 0);
        chip.advance_to(159);
        assert!(captured(&mut chip).is_empty());
        chip.advance_to(160);
        assert_eq!(captured(&mut chip), vec![0xFE]);
    }

    #[test]
    fn two_stop_bits_lengthen_the_frame() {
        let mut chip = chip();
        select_group(&mut chip, 4, 0);
        chip.write_register(4, MIDI_RATE_SELECTION, 0);
        chip.write_register(5, 0x02, 0);
        select_group(&mut chip, 5, 0);
        chip.write_register(5, TRANSMIT_CONTROL_ENABLE, 0);
        queue_byte(&mut chip, 0xFE, 0);
        chip.advance_to(351);
        assert!(captured(&mut chip).is_empty());
        chip.advance_to(352);
        assert_eq!(captured(&mut chip), vec![0xFE]);
    }

    #[test]
    fn status_bits_track_fill_shift_and_drain() {
        let mut chip = chip();
        enable_transmitter_at_midi_rate(&mut chip, 0);
        queue_byte(&mut chip, 0x90, 0);
        queue_byte(&mut chip, 0x40, 0);
        select_group(&mut chip, 5, 0);
        let shifting = chip.read_register(4, 0);
        assert_eq!(shifting & TRANSMIT_STATUS_EMPTY, 0);
        assert_ne!(shifting & TRANSMIT_STATUS_BUSY, 0);
        assert_eq!(shifting & TRANSMIT_STATUS_IDLE, 0);
        let last_in_shifter = chip.read_register(4, 320);
        assert_ne!(last_in_shifter & TRANSMIT_STATUS_EMPTY, 0);
        assert_ne!(last_in_shifter & TRANSMIT_STATUS_BUSY, 0);
        let drained = chip.read_register(4, 640);
        assert_ne!(drained & TRANSMIT_STATUS_EMPTY, 0);
        assert_ne!(drained & TRANSMIT_STATUS_IDLE, 0);
        assert_eq!(drained & TRANSMIT_STATUS_BUSY, 0);
    }

    #[test]
    fn fifo_empty_interrupt_raises_and_icr_clears() {
        let mut chip = chip();
        select_group(&mut chip, 0, 0);
        chip.write_register(6, INTERRUPT_TRANSMIT_EMPTY, 0);
        enable_transmitter_at_midi_rate(&mut chip, 0);
        queue_byte(&mut chip, 0x90, 0);
        assert!(chip.irq_asserted());
        assert_eq!(chip.read_register(2, 0), INTERRUPT_TRANSMIT_EMPTY);
        chip.write_register(3, INTERRUPT_TRANSMIT_EMPTY, 0);
        assert!(!chip.irq_asserted());
    }

    #[test]
    fn interrupt_enable_gates_status_bits() {
        let mut chip = chip();
        enable_transmitter_at_midi_rate(&mut chip, 0);
        queue_byte(&mut chip, 0x90, 0);
        chip.advance_to(320);
        assert!(!chip.irq_asserted());
        assert_eq!(chip.read_register(2, 320), 0);
    }

    #[test]
    fn vector_reports_lowest_cause_with_programmed_offset() {
        let mut chip = chip();
        select_group(&mut chip, 0, 0);
        chip.write_register(4, 0x40, 0);
        chip.write_register(6, INTERRUPT_GENERAL_TIMER | INTERRUPT_TRANSMIT_EMPTY, 0);
        select_group(&mut chip, 8, 0);
        chip.write_register(4, 4, 0);
        chip.write_register(5, TIMER_LOAD_REQUEST, 0);
        enable_transmitter_at_midi_rate(&mut chip, 0);
        queue_byte(&mut chip, 0x90, 0);
        chip.advance_to(16);
        assert_eq!(
            chip.read_register(2, 16),
            INTERRUPT_GENERAL_TIMER | INTERRUPT_TRANSMIT_EMPTY
        );
        assert_eq!(chip.read_register(0, 16), 0x4C);
        chip.write_register(3, INTERRUPT_TRANSMIT_EMPTY, 16);
        assert_eq!(chip.read_register(0, 16), 0x4E);
        chip.write_register(3, INTERRUPT_GENERAL_TIMER, 16);
        assert_eq!(chip.read_register(0, 16), 0x50);
    }

    #[test]
    fn tdr_write_clears_the_empty_cause() {
        let mut chip = chip();
        select_group(&mut chip, 0, 0);
        chip.write_register(6, INTERRUPT_TRANSMIT_EMPTY, 0);
        enable_transmitter_at_midi_rate(&mut chip, 0);
        queue_byte(&mut chip, 0x90, 0);
        assert!(chip.irq_asserted());
        queue_byte(&mut chip, 0x40, 0);
        assert!(!chip.irq_asserted());
    }

    #[test]
    fn general_timer_counts_down_reloads_and_interrupts() {
        let mut chip = chip();
        select_group(&mut chip, 0, 0);
        chip.write_register(6, INTERRUPT_GENERAL_TIMER, 0);
        select_group(&mut chip, 6, 0);
        chip.write_register(6, CLICK_CONTROL_CLKM_1MHZ, 0);
        select_group(&mut chip, 8, 0);
        chip.write_register(4, 100, 0);
        chip.write_register(5, TIMER_LOAD_REQUEST, 0);
        assert_eq!(chip.next_event_tick(), Some(800));
        chip.advance_to(799);
        assert!(!chip.irq_asserted());
        chip.advance_to(800);
        assert!(chip.irq_asserted());
        chip.write_register(3, INTERRUPT_GENERAL_TIMER, 800);
        chip.advance_to(1600);
        assert!(chip.irq_asserted());
    }

    #[test]
    fn general_timer_reload_of_zero_stays_idle() {
        let mut chip = chip();
        select_group(&mut chip, 8, 0);
        chip.write_register(4, 0, 0);
        chip.write_register(5, TIMER_LOAD_REQUEST, 0);
        assert_eq!(chip.next_event_tick(), None);
        chip.write_register(4, 1, 0);
        chip.write_register(5, TIMER_LOAD_REQUEST, 0);
        assert_eq!(chip.next_event_tick(), None);
    }

    #[test]
    fn midi_clock_timer_fires_when_imr_selects_it() {
        let mut chip = chip();
        select_group(&mut chip, 0, 0);
        chip.write_register(5, MODE_MIDI_CLOCK_SELECT, 0);
        chip.write_register(6, INTERRUPT_CLICK_OR_MIDI_CLOCK, 0);
        select_group(&mut chip, 8, 0);
        chip.write_register(6, 10, 0);
        chip.write_register(7, TIMER_LOAD_REQUEST, 0);
        chip.advance_to(40);
        assert!(chip.irq_asserted());
        chip.write_register(3, INTERRUPT_CLICK_OR_MIDI_CLOCK, 40);
        chip.advance_to(80);
        assert!(chip.irq_asserted());
    }

    #[test]
    fn click_counter_divides_the_midi_clock_timer() {
        let mut chip = chip();
        select_group(&mut chip, 0, 0);
        chip.write_register(6, INTERRUPT_CLICK_OR_MIDI_CLOCK, 0);
        select_group(&mut chip, 6, 0);
        chip.write_register(7, TIMER_LOAD_REQUEST | 3, 0);
        select_group(&mut chip, 8, 0);
        chip.write_register(6, 10, 0);
        chip.write_register(7, TIMER_LOAD_REQUEST, 0);
        chip.advance_to(80);
        assert!(!chip.irq_asserted());
        chip.advance_to(120);
        assert!(chip.irq_asserted());
        chip.write_register(3, INTERRUPT_CLICK_OR_MIDI_CLOCK, 120);
        chip.advance_to(240);
        assert!(chip.irq_asserted());
    }

    #[test]
    fn next_event_tick_returns_the_earliest_deadline() {
        let mut chip = chip();
        assert_eq!(chip.next_event_tick(), None);
        enable_transmitter_at_midi_rate(&mut chip, 0);
        queue_byte(&mut chip, 0x90, 0);
        select_group(&mut chip, 8, 0);
        chip.write_register(4, 200, 0);
        chip.write_register(5, TIMER_LOAD_REQUEST, 0);
        assert_eq!(chip.next_event_tick(), Some(320));
        chip.advance_to(320);
        assert_eq!(chip.next_event_tick(), Some(800));
    }

    #[test]
    fn capture_disabled_discards_transmitted_bytes() {
        let mut chip = Ym3802::new();
        enable_transmitter_at_midi_rate(&mut chip, 0);
        queue_byte(&mut chip, 0x90, 0);
        chip.advance_to(320);
        let mut bytes = Vec::new();
        chip.flush_midi_into(&mut bytes);
        assert!(bytes.is_empty());
    }
}
