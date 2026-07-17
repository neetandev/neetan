//! Panasonic FS-CA1 MSX-AUDIO expansion.

use crate::opn_fm::{FmTimerAction, OpnFm, Y8950};

/// Panasonic FS-CA1 firmware size.
const FIRMWARE_SIZE: usize = 0x20_000;
/// Panasonic FS-CA1 mapped RAM size.
const MAPPED_RAM_SIZE: usize = 0x1000;
/// Panasonic FS-CA1 Y8950 ADPCM RAM size.
const ADPCM_RAM_SIZE: usize = 0x8000;
/// Panasonic FS-CA1 Y8950 input clock.
const Y8950_CLOCK_HZ: u32 = 3_579_545;

/// Panasonic FS-CA1 MSX-AUDIO expansion.
pub struct FsCa1 {
    firmware: Box<[u8]>,
    mapped_ram: Box<[u8; MAPPED_RAM_SIZE]>,
    bank: u8,
    io_control: u8,
    y8950: OpnFm<Y8950>,
}

save_state::runtime_state! {
/// Mutable Panasonic FS-CA1 expansion state.
#[derive(Clone)]
pub struct FsCa1State {
    mapped_ram: Vec<u8>,
    bank: u8,
    io_control: u8,
    y8950: crate::opn_fm::OpnFmState<ymfm_oxide::Y8950, ymfm_oxide::YmfmOutput1>,
}}

impl FsCa1 {
    /// Creates the expansion from validated firmware.
    pub fn new(firmware: &[u8], cpu_clock_hz: u32, sample_rate: u32) -> Option<Self> {
        if firmware.len() != FIRMWARE_SIZE {
            return None;
        }
        let mut y8950 = OpnFm::<Y8950>::new(cpu_clock_hz, sample_rate, Y8950_CLOCK_HZ);
        y8950.chip_mut().set_adpcm_memory(vec![0; ADPCM_RAM_SIZE]);
        y8950.chip_mut().set_io_input(0, 0);
        Some(Self {
            firmware: firmware.into(),
            mapped_ram: Box::new([0; MAPPED_RAM_SIZE]),
            bank: 0,
            io_control: 0,
            y8950,
        })
    }

    /// Captures mapped RAM, bank controls, and Y8950 state.
    pub fn capture_state(&self) -> FsCa1State {
        FsCa1State {
            mapped_ram: self.mapped_ram.to_vec(),
            bank: self.bank,
            io_control: self.io_control,
            y8950: self.y8950.capture_state(),
        }
    }

    /// Restores mapped RAM, bank controls, and Y8950 state.
    pub fn restore_state(
        &mut self,
        state: FsCa1State,
    ) -> Result<(), save_state::StateValidationError> {
        if state.mapped_ram.len() != MAPPED_RAM_SIZE || state.bank > 3 {
            return Err(save_state::StateValidationError::new(
                "FS-CA1 state is invalid",
            ));
        }
        self.y8950.restore_state(state.y8950)?;
        self.mapped_ram.copy_from_slice(&state.mapped_ram);
        self.bank = state.bank;
        self.io_control = state.io_control;
        Ok(())
    }

    /// Returns the immutable FS-CA1 firmware identity.
    pub fn resource_identity(&self) -> save_state::ResourceIdentity {
        save_state::ResourceIdentity::from_bytes(&self.firmware)
    }

    /// Reads one selected memory address.
    pub fn read_memory(&self, address: u16) -> u8 {
        if self.bank == 0 {
            match address {
                0x3000..=0x3FFF => return self.mapped_ram[usize::from(address - 0x3000)],
                0x7000..=0x7FFD => return self.mapped_ram[usize::from(address - 0x7000)],
                _ => {}
            }
        }
        if matches!(address, 0x7FFE | 0x7FFF) {
            return 0xFF;
        }
        let half = usize::from(address & 0x4000);
        let offset = usize::from(self.bank) * 0x8000 + half + usize::from(address & 0x3FFF);
        self.firmware[offset]
    }

    /// Writes one selected memory address.
    pub fn write_memory(&mut self, address: u16, value: u8) {
        match address {
            0x7FFE => self.bank = value & 3,
            0x7FFF => self.io_control = value,
            0x3000..=0x3FFF if self.bank == 0 => {
                self.mapped_ram[usize::from(address - 0x3000)] = value;
            }
            0x7000..=0x7FFD if self.bank == 0 => {
                self.mapped_ram[usize::from(address - 0x7000)] = value;
            }
            _ => {}
        }
    }

    /// Reads one Y8950 I/O port.
    pub fn read_io(&mut self, port: u8, current_cycle: u64) -> Option<u8> {
        let pair = (port - 0xC0) >> 1;
        if self.io_control & (1 << pair) == 0 {
            return None;
        }
        Some(if port & 1 == 0 {
            self.y8950.read_status(current_cycle)
        } else {
            self.y8950.read_data(current_cycle)
        })
    }

    /// Writes one Y8950 I/O port.
    pub fn write_io(&mut self, port: u8, value: u8, current_cycle: u64) -> bool {
        let pair = (port - 0xC0) >> 1;
        if self.io_control & (1 << pair) == 0 {
            return false;
        }
        if port & 1 == 0 {
            self.y8950.write_address(value, current_cycle);
        } else {
            self.y8950.write_data(value, current_cycle);
        }
        true
    }

    /// Drains pending Y8950 timer schedule and cancel requests.
    pub fn drain_timers(&mut self) -> &[FmTimerAction] {
        self.y8950.drain_timers()
    }

    /// Notifies the Y8950 that one timer expired.
    pub fn timer_expired(&mut self, timer_id: u32, current_cycle: u64) {
        self.y8950.timer_expired(timer_id, current_cycle);
    }

    /// Returns and clears the coalesced Y8950 interrupt edge.
    pub fn take_irq_change(&mut self) -> Option<bool> {
        self.y8950.take_irq_change()
    }

    /// Whether the Y8950 interrupt output is asserted.
    pub fn irq_asserted(&self) -> bool {
        self.y8950.irq_asserted()
    }

    /// Mixes Y8950 output into the machine stream.
    pub fn generate_samples(
        &mut self,
        current_cycle: u64,
        cpu_clock_hz: u32,
        volume: f32,
        output: &mut [f32],
    ) {
        self.y8950
            .generate_samples(current_cycle, cpu_clock_hz, volume, output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn firmware() -> Vec<u8> {
        (0..4).flat_map(|bank| vec![bank as u8; 0x8000]).collect()
    }

    #[test]
    fn banking_and_ram_overlays_follow_fs_ca1_decode() {
        let mut audio = FsCa1::new(&firmware(), 3_579_545, 48_000).unwrap();
        assert_eq!(audio.read_memory(0x0000), 0);
        assert_eq!(audio.read_memory(0x8000), 0);
        audio.write_memory(0x3000, 0x55);
        assert_eq!(audio.read_memory(0x3000), 0x55);
        assert_eq!(audio.read_memory(0x7000), 0x55);

        audio.write_memory(0x7FFE, 2);
        assert_eq!(audio.read_memory(0x0000), 2);
        assert_eq!(audio.read_memory(0x8000), 2);
        assert_eq!(audio.read_memory(0x3000), 2);
        assert_eq!(audio.read_memory(0x7FFE), 0xFF);
    }

    #[test]
    fn port_pairs_are_enabled_independently() {
        let mut audio = FsCa1::new(&firmware(), 3_579_545, 48_000).unwrap();
        assert_eq!(audio.read_io(0xC0, 0), None);
        audio.write_memory(0x7FFF, 1);
        assert!(audio.write_io(0xC0, 0x19, 0));
        assert_eq!(audio.read_io(0xC1, 0), Some(0));
        assert_eq!(audio.read_io(0xC2, 0), None);
        audio.write_memory(0x7FFF, 2);
        assert!(audio.write_io(0xC2, 0x19, 0));
        assert_eq!(audio.read_io(0xC3, 0), Some(0));
    }

    #[test]
    fn y8950_timer_requests_are_exposed_to_the_machine() {
        let mut audio = FsCa1::new(&firmware(), 3_579_545, 48_000).unwrap();
        audio.write_memory(0x7FFF, 1);
        audio.write_io(0xC0, 0x02, 0);
        audio.write_io(0xC1, 0xF0, 0);
        audio.write_io(0xC0, 0x04, 0);
        audio.write_io(0xC1, 0x01, 0);
        assert!(
            audio
                .drain_timers()
                .iter()
                .any(|action| matches!(action, FmTimerAction::Schedule { timer_id: 0, .. }))
        );
    }
}
