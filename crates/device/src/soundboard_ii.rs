//! PC-8801 Sound Board II: the internal YM2608 (OPNA).

use ymfm_oxide::Ym2608;

use crate::opn_fm::{EVOLVED_RHYTHM_ROM, FmTimerAction, OpnFm};

/// 256 KiB ADPCM-B sample RAM.
const ADPCM_B_RAM_SIZE: usize = 256 * 1024;

/// YM2608 input clock on the Sound Board II (7.9872 MHz).
const YM2608_CLOCK_HZ: u32 = 7_987_200;

/// Idle joystick/mouse readback (active-low: nothing pressed).
const JOYSTICK_IDLE: u8 = 0xFF;

/// PC-8801 Sound Board II: YM2608 (OPNA) FM + SSG + ADPCM with resampling.
pub struct SoundboardII {
    core: OpnFm<Ym2608>,
    address_low: u8,
    joyport_a: u8,
    joyport_b: u8,
}

impl SoundboardII {
    /// Creates a Sound Board II with the embedded rhythm ROM and ADPCM-B RAM.
    pub fn new(cpu_clock_hz: u32, sample_rate: u32) -> Self {
        let mut core = OpnFm::<Ym2608>::new(cpu_clock_hz, sample_rate, YM2608_CLOCK_HZ);
        core.chip_mut()
            .set_adpcm_a_rom(EVOLVED_RHYTHM_ROM.as_slice());
        core.chip_mut().set_adpcm_b_ram(vec![0; ADPCM_B_RAM_SIZE]);
        Self {
            core,
            address_low: 0,
            joyport_a: JOYSTICK_IDLE,
            joyport_b: JOYSTICK_IDLE,
        }
    }

    /// Sets the SSG I/O port readback presented at registers 0x0E/0x0F, which on
    /// the PC-88 carry the joystick/mouse lines. `port_a` (register 0x0E) is the
    /// data nibble window; `port_b` (register 0x0F) is the button lines.
    pub fn set_joyport(&mut self, port_a: u8, port_b: u8) {
        self.joyport_a = port_a;
        self.joyport_b = port_b;
    }

    /// Reads the low-bank status register (port 0x44).
    pub fn read_status(&mut self, current_cycle: u64) -> u8 {
        self.core.read_status(current_cycle)
    }

    /// Reads the addressed low-bank register (port 0x45).
    ///
    /// The SSG I/O ports A/B (registers 0x0E/0x0F) carry the joystick/mouse lines
    /// and return the values set via `set_joyport` (defaulting to idle).
    pub fn read_data(&mut self, current_cycle: u64) -> u8 {
        match self.address_low {
            0x0E => self.joyport_a,
            0x0F => self.joyport_b,
            _ => self.core.read_data(current_cycle),
        }
    }

    /// Reads the high-bank status register (port 0x46).
    pub fn read_status_hi(&mut self, current_cycle: u64) -> u8 {
        self.core.read_status_hi(current_cycle)
    }

    /// Reads the addressed high-bank register (port 0x47).
    pub fn read_data_hi(&mut self, current_cycle: u64) -> u8 {
        self.core.read_data_hi(current_cycle)
    }

    /// Latches the low-bank register address (port 0x44 write).
    pub fn write_address(&mut self, value: u8, current_cycle: u64) {
        self.address_low = value;
        self.core.write_address(value, current_cycle);
    }

    /// Writes the addressed low-bank register (port 0x45 write).
    pub fn write_data(&mut self, value: u8, current_cycle: u64) {
        self.core.write_data(value, current_cycle);
    }

    /// Latches the high-bank register address (port 0x46 write).
    pub fn write_address_hi(&mut self, value: u8, current_cycle: u64) {
        self.core.write_address_hi(value, current_cycle);
    }

    /// Writes the addressed high-bank register (port 0x47 write).
    pub fn write_data_hi(&mut self, value: u8, current_cycle: u64) {
        self.core.write_data_hi(value, current_cycle);
    }

    /// Notifies the chip that timer `timer_id` has expired.
    pub fn timer_expired(&mut self, timer_id: u32, current_cycle: u64) {
        self.core.timer_expired(timer_id, current_cycle);
    }

    /// Drains pending FM timer schedule/cancel requests (keyed by `timer_id`).
    pub fn drain_timers(&mut self) -> &[FmTimerAction] {
        self.core.drain_timers()
    }

    /// Returns and clears the coalesced chip IRQ-output edge.
    pub fn take_irq_change(&mut self) -> Option<bool> {
        self.core.take_irq_change()
    }

    /// Returns whether the chip IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.core.irq_asserted()
    }

    /// Generates resampled stereo OPNA audio and mixes it into `output`.
    pub fn generate_samples(
        &mut self,
        current_cycle: u64,
        cpu_clock_hz: u32,
        volume: f32,
        output: &mut [f32],
    ) {
        self.core
            .generate_samples(current_cycle, cpu_clock_hz, volume, output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_timer_a_schedules_it() {
        let mut board = SoundboardII::new(7_987_200, 48_000);
        board.write_address(0x24, 0);
        board.write_data(0xFF, 0);
        board.write_address(0x25, 0);
        board.write_data(0x03, 0);
        board.write_address(0x27, 0);
        board.write_data(0x01, 0);
        let scheduled = board
            .drain_timers()
            .iter()
            .any(|a| matches!(a, FmTimerAction::Schedule { timer_id: 0, .. }));
        assert!(scheduled, "loading timer A schedules it");
    }

    #[test]
    fn ssg_io_ports_read_back_idle_joystick() {
        let mut board = SoundboardII::new(7_987_200, 48_000);
        board.write_address(0x0E, 0);
        assert_eq!(board.read_data(0), JOYSTICK_IDLE);
        board.write_address(0x0F, 0);
        assert_eq!(board.read_data(0), JOYSTICK_IDLE);
    }
}
