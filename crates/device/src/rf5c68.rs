//! Ricoh RF5C68 8-channel PCM sound generator.
//!
//! Used by the FM TOWNS for sampled sound. Eight independent channels each play
//! 8-bit sign-magnitude samples out of a shared 64 KB wave RAM, with per-channel
//! envelope, stereo pan, frequency step, and a loop point. A `0xFF` byte in the
//! wave RAM is the loop terminator: it rewinds the channel to its loop start and
//! (when it sits at the end of a 4 KB bank) latches a bank interrupt.

use resampler::{Attenuation, Latency, ResamplerFir};

/// Native output rate: the 8 MHz master clock divided by 384.
const NATIVE_SAMPLE_RATE: u32 = 20833;
/// Number of PCM channels.
const CHANNEL_COUNT: usize = 8;
/// Wave RAM size in bytes.
const WAVE_RAM_SIZE: usize = 65536;
/// Wave RAM address mask.
const WAVE_RAM_MASK: usize = WAVE_RAM_SIZE - 1;
/// Number of address bits below the wave-RAM byte address (the fractional
/// position accumulated from the frequency step).
const ADDRESS_FRACTION_BITS: u32 = 11;
/// Bytes per interrupt bank (`2^12`).
const BANK_SHIFT: u32 = 12;
/// Byte value that terminates a sample / marks a loop point.
const LOOP_TERMINATOR: u8 = 0xFF;

const RESAMPLER_LATENCY: Latency = Latency::Sample64;
const RESAMPLER_ATTENUATION: Attenuation = Attenuation::Db60;

/// Register offsets within the 04F0-04F8 window.
const REG_ENVELOPE: u8 = 0;
const REG_PAN: u8 = 1;
const REG_FREQUENCY_LOW: u8 = 2;
const REG_FREQUENCY_HIGH: u8 = 3;
const REG_LOOP_START_LOW: u8 = 4;
const REG_LOOP_START_HIGH: u8 = 5;
const REG_START_ADDRESS: u8 = 6;
const REG_CONTROL: u8 = 7;
const REG_CHANNEL_ON_OFF: u8 = 8;

/// A single RF5C68 PCM channel.
#[derive(Clone, Copy)]
struct PcmChannel {
    /// Envelope (overall amplitude), 0-255.
    envelope: u8,
    /// Stereo pan: low nibble is the left gain, high nibble the right gain.
    pan: u8,
    /// Start address high byte (the start address is `start_page << 8`).
    start_page: u8,
    /// Frequency step added to `position` each native sample (8.11 fixed point).
    frequency_delta: u16,
    /// Loop-start wave-RAM address.
    loop_start: u16,
    /// Fixed-point play position; the wave-RAM byte address is
    /// `position >> ADDRESS_FRACTION_BITS`.
    position: u32,
}

impl PcmChannel {
    fn new() -> Self {
        Self {
            envelope: 0xFF,
            pan: 0,
            start_page: 0,
            frequency_delta: 0,
            loop_start: 0,
            position: 0,
        }
    }

    /// Current wave-RAM byte address.
    fn wave_address(&self) -> usize {
        ((self.position >> ADDRESS_FRACTION_BITS) as usize) & WAVE_RAM_MASK
    }

    /// Seeds the play position from the start-address register.
    fn rewind_to_start(&mut self) {
        self.position = (self.start_page as u32) << (8 + ADDRESS_FRACTION_BITS);
    }
}

/// Ricoh RF5C68 PCM sound generator.
pub struct Rf5c68 {
    channels: [PcmChannel; CHANNEL_COUNT],
    wave_ram: Box<[u8; WAVE_RAM_SIZE]>,
    /// Channel selected for register writes (control bit 6 path).
    channel_select: u8,
    /// Byte offset of the wave-RAM window into the full 64 KB (control bit 6
    /// clear path).
    wave_bank: usize,
    /// Chip playback enable (control bit 7).
    enabled: bool,
    /// Per-channel on/off, active low: a set bit means the channel is off.
    channel_on_off: u8,
    /// Pending bank interrupts (read/cleared through 04EB).
    interrupt_pending: u8,
    /// Bank-interrupt mask (04EA).
    interrupt_mask: u8,

    sample_rate: u32,
    resampler: ResamplerFir,
    input_buffer: Vec<f32>,
    resample_output: Vec<f32>,
    sample_remainder: f64,
    last_generate_cycle: u64,
}

impl Rf5c68 {
    /// Creates a new RF5C68 driving output at `sample_rate` Hz.
    pub fn new(sample_rate: u32) -> Self {
        let resampler = ResamplerFir::new_from_hz(
            2,
            NATIVE_SAMPLE_RATE,
            sample_rate,
            RESAMPLER_LATENCY,
            RESAMPLER_ATTENUATION,
        );
        let output_size = resampler.buffer_size_output();
        Self {
            channels: [PcmChannel::new(); CHANNEL_COUNT],
            // Wave RAM powers up filled with the loop terminator so silent
            // channels stay silent until a program uploads samples.
            wave_ram: Box::new([LOOP_TERMINATOR; WAVE_RAM_SIZE]),
            channel_select: 0,
            wave_bank: 0,
            enabled: false,
            channel_on_off: 0xFF,
            interrupt_pending: 0,
            interrupt_mask: 0,
            sample_rate,
            resampler,
            input_buffer: Vec::new(),
            resample_output: vec![0.0; output_size],
            sample_remainder: 0.0,
            last_generate_cycle: 0,
        }
    }

    /// Resets the chip to its power-on state, preserving wave RAM contents.
    pub fn reset(&mut self) {
        self.channels = [PcmChannel::new(); CHANNEL_COUNT];
        self.channel_select = 0;
        self.wave_bank = 0;
        self.enabled = false;
        self.channel_on_off = 0xFF;
        self.interrupt_pending = 0;
        self.interrupt_mask = 0;
        self.sample_remainder = 0.0;
        self.input_buffer.clear();
    }

    /// Changes the device output sample rate, rebuilding the resampler.
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        if sample_rate == self.sample_rate {
            return;
        }
        self.sample_rate = sample_rate;
        self.resampler = ResamplerFir::new_from_hz(
            2,
            NATIVE_SAMPLE_RATE,
            sample_rate,
            RESAMPLER_LATENCY,
            RESAMPLER_ATTENUATION,
        );
        self.resample_output = vec![0.0; self.resampler.buffer_size_output()];
        self.input_buffer.clear();
    }

    /// Writes one of the nine control registers (04F0-04F8, offset 0-8).
    pub fn write_register(&mut self, offset: u8, value: u8) {
        let channel = &mut self.channels[self.channel_select as usize];
        match offset {
            REG_ENVELOPE => channel.envelope = value,
            REG_PAN => channel.pan = value,
            REG_FREQUENCY_LOW => {
                channel.frequency_delta = (channel.frequency_delta & 0xFF00) | value as u16;
            }
            REG_FREQUENCY_HIGH => {
                channel.frequency_delta =
                    (channel.frequency_delta & 0x00FF) | ((value as u16) << 8);
            }
            REG_LOOP_START_LOW => {
                channel.loop_start = (channel.loop_start & 0xFF00) | value as u16;
            }
            REG_LOOP_START_HIGH => {
                channel.loop_start = (channel.loop_start & 0x00FF) | ((value as u16) << 8);
            }
            REG_START_ADDRESS => {
                channel.start_page = value;
                channel.rewind_to_start();
            }
            REG_CONTROL => self.write_control(value),
            REG_CHANNEL_ON_OFF => self.write_channel_on_off(value),
            _ => {}
        }
    }

    fn write_control(&mut self, value: u8) {
        if value & 0x40 != 0 {
            // Bit 6 set: select the channel that register writes address.
            self.channel_select = value & 0x07;
        } else {
            // Bit 6 clear: select the 4 KB wave-RAM window bank.
            self.wave_bank = ((value & 0x0F) as usize) << BANK_SHIFT;
        }
        // Bit 7 is the overall playback enable.
        self.enabled = value & 0x80 != 0;
    }

    fn write_channel_on_off(&mut self, value: u8) {
        // Active low: a channel that transitions between on and off while the
        // chip is enabled restarts from its start address.
        if self.enabled {
            let toggled = self.channel_on_off ^ value;
            for channel_index in 0..CHANNEL_COUNT {
                if toggled & (1 << channel_index) != 0 {
                    self.channels[channel_index].rewind_to_start();
                }
            }
        }
        self.channel_on_off = value;
    }

    /// Reads a byte from the currently banked wave-RAM window (offset 0-0xFFF).
    pub fn read_wave_ram(&self, offset: u16) -> u8 {
        self.wave_ram[(self.wave_bank + offset as usize) & WAVE_RAM_MASK]
    }

    /// Writes a byte into the currently banked wave-RAM window (offset 0-0xFFF).
    pub fn write_wave_ram(&mut self, offset: u16, value: u8) {
        self.wave_ram[(self.wave_bank + offset as usize) & WAVE_RAM_MASK] = value;
    }

    /// Sets the bank-interrupt mask register (04EA).
    pub fn set_interrupt_mask(&mut self, value: u8) {
        self.interrupt_mask = value;
    }

    /// Returns the bank-interrupt mask register (04EA).
    pub fn interrupt_mask(&self) -> u8 {
        self.interrupt_mask
    }

    /// Returns the pending bank interrupts and clears them (04EB read).
    pub fn take_interrupt_pending(&mut self) -> u8 {
        let pending = self.interrupt_pending;
        self.interrupt_pending = 0;
        pending
    }

    /// Whether the chip is currently asserting its interrupt line.
    pub fn interrupt_asserted(&self) -> bool {
        self.interrupt_pending != 0
    }

    /// Latches a bank interrupt if it is not masked off.
    fn latch_bank_interrupt(&mut self, wave_address: usize) {
        // Sixteen 4 KB banks map onto the eight mask/pending bits (bank >> 1).
        let bank = (wave_address >> BANK_SHIFT) & 0x0F;
        let bit = 1u8 << (bank >> 1);
        if self.interrupt_mask & bit != 0 {
            self.interrupt_pending |= bit;
        }
    }

    /// Produces one native (20833 Hz) stereo sample, stepping every active
    /// channel. Returns clamped 16-bit `(left, right)`.
    fn step_native_sample(&mut self) -> (i32, i32) {
        if !self.enabled {
            return (0, 0);
        }

        let mut left = 0i32;
        let mut right = 0i32;

        for channel_index in 0..CHANNEL_COUNT {
            // Active low: a set bit disables the channel.
            if self.channel_on_off & (1 << channel_index) != 0 {
                continue;
            }
            let frequency_delta = self.channels[channel_index].frequency_delta;
            if frequency_delta == 0 {
                continue;
            }

            let mut wave_address = self.channels[channel_index].wave_address();
            let mut sample = self.wave_ram[wave_address];

            // A loop terminator rewinds to the loop start; if it sits at the end
            // of a bank it latches an interrupt. A second terminator right at the
            // loop start means the channel is effectively dead this sample.
            if sample == LOOP_TERMINATOR {
                if wave_address & 0xFFF == 0xFFF {
                    self.latch_bank_interrupt(wave_address);
                }
                let loop_position =
                    (self.channels[channel_index].loop_start as u32) << ADDRESS_FRACTION_BITS;
                self.channels[channel_index].position = loop_position;
                wave_address = self.channels[channel_index].wave_address();
                sample = self.wave_ram[wave_address];
                if sample == LOOP_TERMINATOR {
                    continue;
                }
            }

            let pan = self.channels[channel_index].pan;
            let envelope = self.channels[channel_index].envelope as i32;
            let magnitude = (sample & 0x7F) as i32;
            let left_gain = (pan & 0x0F) as i32 * envelope;
            let right_gain = ((pan >> 4) & 0x0F) as i32 * envelope;
            let mut channel_left = (magnitude * left_gain) >> 5;
            let mut channel_right = (magnitude * right_gain) >> 5;
            // The MSB is the sign bit: a set bit is a positive sample.
            if sample & 0x80 == 0 {
                channel_left = -channel_left;
                channel_right = -channel_right;
            }
            left += channel_left;
            right += channel_right;

            // Advance the play position. Crossing into a new bank latches an
            // interrupt for the bank we left, and wrapping past the top of wave
            // RAM restarts at zero.
            let previous_bank = (wave_address >> BANK_SHIFT) & 0x0F;
            let next_position = self.channels[channel_index]
                .position
                .wrapping_add(frequency_delta as u32);
            self.channels[channel_index].position = next_position;
            let next_bank = (self.channels[channel_index].wave_address() >> BANK_SHIFT) & 0x0F;
            if previous_bank != next_bank {
                self.latch_bank_interrupt(wave_address);
                if next_bank == 0 {
                    self.channels[channel_index].position = 0;
                }
            }
        }

        (left.clamp(-32768, 32767), right.clamp(-32768, 32767))
    }

    /// Advances the chip by the cycles elapsed since the last call, resamples
    /// the produced audio, and additively mixes it into `output` (interleaved
    /// stereo) at `volume`.
    pub fn generate_samples(
        &mut self,
        current_cycle: u64,
        cpu_clock_hz: u32,
        volume: f32,
        output: &mut [f32],
    ) {
        let frame_cycles = current_cycle.saturating_sub(self.last_generate_cycle);
        if frame_cycles > 0 && cpu_clock_hz > 0 {
            let exact = frame_cycles as f64 * f64::from(NATIVE_SAMPLE_RATE)
                / f64::from(cpu_clock_hz)
                + self.sample_remainder;
            let native_count = exact as usize;
            self.sample_remainder = exact - native_count as f64;

            self.input_buffer.reserve(native_count * 2);
            for _ in 0..native_count {
                let (left, right) = self.step_native_sample();
                self.input_buffer.push(left as f32 / 32768.0);
                self.input_buffer.push(right as f32 / 32768.0);
            }
        }

        if !self.input_buffer.is_empty() {
            let total_interleaved = self.input_buffer.len();
            let mut input_offset = 0;
            let mut output_offset = 0;
            let sample_count = output.len();
            while input_offset < total_interleaved && output_offset < sample_count {
                let remaining_output =
                    (sample_count - output_offset).min(self.resample_output.len());
                let out_buffer = &mut self.resample_output[..remaining_output];
                let Ok((consumed, produced)) = self.resampler.resample(
                    &self.input_buffer[input_offset..total_interleaved],
                    out_buffer,
                ) else {
                    break;
                };
                for i in 0..produced {
                    output[output_offset + i] += out_buffer[i] * volume;
                }
                input_offset += consumed;
                output_offset += produced;
                if consumed == 0 {
                    break;
                }
            }
            self.input_buffer.drain(..input_offset);
        }

        self.last_generate_cycle = current_cycle;
    }

    /// Realigns the generation clock without producing samples (used when audio
    /// is discarded, e.g. during fast-forward).
    pub fn advance_generate_cycle(&mut self, current_cycle: u64) {
        self.last_generate_cycle = current_cycle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPU_CLOCK: u32 = 66_000_000;

    fn upload_sample(chip: &mut Rf5c68, address: u16, bytes: &[u8]) {
        for (index, &byte) in bytes.iter().enumerate() {
            chip.wave_ram[address as usize + index] = byte;
        }
    }

    /// Selects a channel for register access via the control register.
    fn select_channel(chip: &mut Rf5c68, channel: u8) {
        chip.write_register(REG_CONTROL, 0x40 | (channel & 0x07));
    }

    #[test]
    fn control_register_selects_channel_and_bank() {
        let mut chip = Rf5c68::new(48_000);
        chip.write_register(REG_CONTROL, 0x40 | 5);
        assert_eq!(chip.channel_select, 5);
        // Bit 6 clear selects the wave-RAM window bank instead.
        chip.write_register(REG_CONTROL, 0x03);
        assert_eq!(chip.wave_bank, 0x3000);
        assert!(!chip.enabled);
        chip.write_register(REG_CONTROL, 0x80);
        assert!(chip.enabled);
    }

    #[test]
    fn wave_ram_window_is_banked() {
        let mut chip = Rf5c68::new(48_000);
        chip.write_register(REG_CONTROL, 0x01); // bank 1 -> offset 0x1000
        chip.write_wave_ram(0x010, 0x42);
        assert_eq!(chip.wave_ram[0x1010], 0x42);
        assert_eq!(chip.read_wave_ram(0x010), 0x42);
    }

    #[test]
    fn start_address_seeds_play_position() {
        let mut chip = Rf5c68::new(48_000);
        select_channel(&mut chip, 0);
        chip.write_register(REG_START_ADDRESS, 0x20);
        // Start page 0x20 -> wave address 0x2000.
        assert_eq!(chip.channels[0].wave_address(), 0x2000);
    }

    #[test]
    fn steps_positive_and_negative_samples_with_pan() {
        let mut chip = Rf5c68::new(48_000);
        // A positive sample (bit 7 set), magnitude 0x40, at address 0x0000.
        // 0xFF is the loop terminator, so avoid it by using 0xC0 = 0x80 | 0x40.
        upload_sample(&mut chip, 0x0000, &[0xC0]);
        select_channel(&mut chip, 0);
        chip.write_register(REG_ENVELOPE, 0xFF);
        chip.write_register(REG_PAN, 0x0F); // full left, zero right
        chip.write_register(REG_FREQUENCY_LOW, 0x00);
        chip.write_register(REG_FREQUENCY_HIGH, 0x08); // step 0x0800 = 1 byte/sample
        chip.write_register(REG_START_ADDRESS, 0x00);
        chip.write_register(REG_CONTROL, 0x80); // enable
        chip.write_register(REG_CHANNEL_ON_OFF, 0xFE); // channel 0 on (active low)

        let (left, right) = chip.step_native_sample();
        // magnitude 0x40 * left gain (0x0F * 0xFF) >> 5, positive.
        let expected = (0x40 * (0x0F * 0xFF)) >> 5;
        assert_eq!(left, expected);
        assert_eq!(right, 0);
    }

    #[test]
    fn negative_sample_has_inverted_sign() {
        let mut chip = Rf5c68::new(48_000);
        chip.wave_ram[0x0000] = 0x40; // bit 7 clear -> negative
        select_channel(&mut chip, 0);
        chip.write_register(REG_ENVELOPE, 0xFF);
        chip.write_register(REG_PAN, 0xF0); // full right
        chip.write_register(REG_FREQUENCY_HIGH, 0x08);
        chip.write_register(REG_START_ADDRESS, 0x00);
        chip.write_register(REG_CONTROL, 0x80);
        chip.write_register(REG_CHANNEL_ON_OFF, 0xFE);

        let (left, right) = chip.step_native_sample();
        let magnitude = (0x40 * (0x0F * 0xFF)) >> 5;
        assert_eq!(left, 0);
        assert_eq!(right, -magnitude);
    }

    #[test]
    fn disabled_channel_and_chip_are_silent() {
        let mut chip = Rf5c68::new(48_000);
        chip.wave_ram[0x0000] = 0xC0;
        select_channel(&mut chip, 0);
        chip.write_register(REG_ENVELOPE, 0xFF);
        chip.write_register(REG_PAN, 0xFF);
        chip.write_register(REG_FREQUENCY_HIGH, 0x08);
        chip.write_register(REG_START_ADDRESS, 0x00);
        // Chip not enabled yet.
        assert_eq!(chip.step_native_sample(), (0, 0));
        // Enable chip but leave all channels off (0xFF active low).
        chip.write_register(REG_CONTROL, 0x80);
        chip.write_register(REG_CHANNEL_ON_OFF, 0xFF);
        assert_eq!(chip.step_native_sample(), (0, 0));
    }

    #[test]
    fn loop_terminator_rewinds_to_loop_start() {
        let mut chip = Rf5c68::new(48_000);
        // Sample at 0x0000 is a positive value, 0x0001 is the terminator, and the
        // loop start at 0x0000 replays it.
        chip.wave_ram[0x0000] = 0xC0;
        chip.wave_ram[0x0001] = LOOP_TERMINATOR;
        select_channel(&mut chip, 0);
        chip.write_register(REG_ENVELOPE, 0xFF);
        chip.write_register(REG_PAN, 0x0F);
        chip.write_register(REG_FREQUENCY_HIGH, 0x08); // 1 byte/sample
        chip.write_register(REG_LOOP_START_LOW, 0x00);
        chip.write_register(REG_LOOP_START_HIGH, 0x00);
        chip.write_register(REG_START_ADDRESS, 0x00);
        chip.write_register(REG_CONTROL, 0x80);
        chip.write_register(REG_CHANNEL_ON_OFF, 0xFE);

        let expected = (0x40 * (0x0F * 0xFF)) >> 5;
        // First sample plays byte 0, advances to byte 1.
        assert_eq!(chip.step_native_sample().0, expected);
        // Second sample hits the terminator at byte 1, rewinds to loop start 0,
        // and plays byte 0 again.
        assert_eq!(chip.step_native_sample().0, expected);
    }

    #[test]
    fn bank_terminator_latches_masked_interrupt() {
        let mut chip = Rf5c68::new(48_000);
        // Playable samples from 0x0F00 up to the last byte of bank 0 (0x0FFF),
        // where a terminator latches interrupt bit 0. The loop start (0x0000)
        // is left as a terminator so the channel goes silent afterwards.
        for address in 0x0F00..0x0FFF {
            chip.wave_ram[address] = 0xC0;
        }
        chip.wave_ram[0x0FFF] = LOOP_TERMINATOR;
        chip.wave_ram[0x0000] = LOOP_TERMINATOR;
        select_channel(&mut chip, 0);
        chip.set_interrupt_mask(0xFF);
        chip.write_register(REG_ENVELOPE, 0xFF);
        chip.write_register(REG_PAN, 0xFF);
        chip.write_register(REG_FREQUENCY_HIGH, 0x08);
        chip.write_register(REG_START_ADDRESS, 0x0F); // start at 0x0F00
        chip.write_register(REG_CONTROL, 0x80);
        chip.write_register(REG_CHANNEL_ON_OFF, 0xFE);

        // Step until the play pointer reaches 0x0FFF.
        for _ in 0..0x100 {
            chip.step_native_sample();
            if chip.interrupt_asserted() {
                break;
            }
        }
        assert!(chip.interrupt_asserted());
        assert_eq!(chip.interrupt_mask(), 0xFF);
        assert_eq!(chip.take_interrupt_pending() & 0x01, 0x01);
        assert!(!chip.interrupt_asserted());
    }

    #[test]
    fn masked_out_bank_does_not_latch_interrupt() {
        let mut chip = Rf5c68::new(48_000);
        for address in 0x0F00..0x0FFF {
            chip.wave_ram[address] = 0xC0;
        }
        chip.wave_ram[0x0FFF] = LOOP_TERMINATOR;
        chip.wave_ram[0x0000] = LOOP_TERMINATOR;
        select_channel(&mut chip, 0);
        chip.set_interrupt_mask(0x00); // all banks masked off
        chip.write_register(REG_ENVELOPE, 0xFF);
        chip.write_register(REG_FREQUENCY_HIGH, 0x08);
        chip.write_register(REG_START_ADDRESS, 0x0F);
        chip.write_register(REG_CONTROL, 0x80);
        chip.write_register(REG_CHANNEL_ON_OFF, 0xFE);

        for _ in 0..0x100 {
            chip.step_native_sample();
        }
        assert!(!chip.interrupt_asserted());
    }

    #[test]
    fn generate_samples_mixes_into_output() {
        let mut chip = Rf5c68::new(48_000);
        // A repeating positive tone.
        for address in 0..0x40u16 {
            chip.wave_ram[address as usize] = 0xC0;
        }
        chip.wave_ram[0x40] = LOOP_TERMINATOR;
        select_channel(&mut chip, 0);
        chip.write_register(REG_ENVELOPE, 0xFF);
        chip.write_register(REG_PAN, 0xFF);
        chip.write_register(REG_LOOP_START_LOW, 0x00);
        chip.write_register(REG_FREQUENCY_HIGH, 0x08);
        chip.write_register(REG_START_ADDRESS, 0x00);
        chip.write_register(REG_CONTROL, 0x80);
        chip.write_register(REG_CHANNEL_ON_OFF, 0xFE);

        let mut output = vec![0.0f32; 512];
        // One frame worth of cycles at the CPU clock.
        let cycles = CPU_CLOCK as u64 / 100;
        chip.generate_samples(cycles, CPU_CLOCK, 1.0, &mut output);
        assert!(output.iter().any(|&sample| sample != 0.0));
    }
}
