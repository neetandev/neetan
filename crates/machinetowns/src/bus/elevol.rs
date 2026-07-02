//! Electronic volume attenuators (I/O 0x04E0-0x04E3).
//!
//! Two chips sit between the analog sources and the line output, each with
//! four channels behind a data/command port pair. The second chip's channels 0
//! and 1 attenuate the CD-DA left/right signal; the remaining channels (line
//! in, microphone, modem) are stored and read back only.

/// One attenuator channel.
#[derive(Clone, Copy)]
struct ElectronicVolumeChannel {
    /// Attenuation setting, 0-63 (63 = no attenuation, -0.5 dB per step).
    volume: u8,
    /// Channel enable; a disabled channel is fully muted.
    enabled: bool,
    /// C0: bypass the attenuator (full level regardless of `volume`).
    direct: bool,
    /// C32: fixed -32 dB regardless of `volume`.
    attenuated: bool,
}

impl ElectronicVolumeChannel {
    fn new() -> Self {
        Self {
            volume: 31,
            enabled: true,
            direct: false,
            attenuated: false,
        }
    }

    /// The linear transmission ratio of this channel.
    fn ratio(&self) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        if self.attenuated {
            // -32 dB.
            return 0.025;
        }
        if self.direct || self.volume == 63 {
            return 1.0;
        }
        let decibel = 0.5 * (f32::from(self.volume) - 63.0);
        10.0f32.powf(decibel / 20.0)
    }
}

/// One electronic-volume chip: four channels behind a data/command port pair.
pub(crate) struct ElectronicVolume {
    channels: [ElectronicVolumeChannel; 4],
    /// Channel addressed by the data port, set through the command port.
    channel_latch: u8,
}

impl ElectronicVolume {
    pub(crate) fn new() -> Self {
        Self {
            channels: [ElectronicVolumeChannel::new(); 4],
            channel_latch: 0,
        }
    }

    /// Writes the data port: the addressed channel's attenuation setting.
    pub(crate) fn write_data(&mut self, value: u8) {
        self.channels[usize::from(self.channel_latch)].volume = value & 0x3F;
    }

    /// Reads the data port back.
    pub(crate) fn read_data(&self) -> u8 {
        self.channels[usize::from(self.channel_latch)].volume
    }

    /// Writes the command port: selects the addressed channel and sets its
    /// enable (bit 2), C0 (bit 3), and C32 (bit 4) controls.
    pub(crate) fn write_command(&mut self, value: u8) {
        self.channel_latch = value & 0x03;
        let channel = &mut self.channels[usize::from(self.channel_latch)];
        channel.enabled = value & 0x04 != 0;
        channel.direct = value & 0x08 != 0;
        channel.attenuated = value & 0x10 != 0;
    }

    /// Reads the command port back.
    pub(crate) fn read_command(&self) -> u8 {
        let channel = &self.channels[usize::from(self.channel_latch)];
        let mut value = self.channel_latch;
        if channel.enabled {
            value |= 0x04;
        }
        if channel.direct {
            value |= 0x08;
        }
        if channel.attenuated {
            value |= 0x10;
        }
        value
    }

    /// The linear transmission ratio of `channel`.
    pub(crate) fn channel_ratio(&self, channel: usize) -> f32 {
        self.channels[channel].ratio()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_on_defaults() {
        let elevol = ElectronicVolume::new();
        assert_eq!(elevol.read_data(), 31);
        assert_eq!(elevol.read_command(), 0x04);
    }

    #[test]
    fn command_selects_channel_and_controls() {
        let mut elevol = ElectronicVolume::new();
        elevol.write_command(0x04 | 0x01);
        elevol.write_data(63);
        assert_eq!(elevol.read_data(), 63);
        assert_eq!(elevol.channel_ratio(1), 1.0);
        // Channel 0 keeps its default attenuation.
        elevol.write_command(0x04);
        assert_eq!(elevol.read_data(), 31);
    }

    #[test]
    fn disabled_channel_is_muted() {
        let mut elevol = ElectronicVolume::new();
        elevol.write_command(0x00);
        assert_eq!(elevol.channel_ratio(0), 0.0);
    }

    #[test]
    fn control_bits_override_volume() {
        let mut elevol = ElectronicVolume::new();
        // C32 forces -32 dB even at full volume.
        elevol.write_command(0x04 | 0x10);
        elevol.write_data(63);
        assert_eq!(elevol.channel_ratio(0), 0.025);
        // C0 forces full level even at zero volume.
        elevol.write_command(0x04 | 0x08);
        elevol.write_data(0);
        assert_eq!(elevol.channel_ratio(0), 1.0);
    }

    #[test]
    fn volume_steps_attenuate_by_half_decibel() {
        let mut elevol = ElectronicVolume::new();
        elevol.write_command(0x04);
        elevol.write_data(62);
        let expected = 10.0f32.powf(-0.5 / 20.0);
        assert!((elevol.channel_ratio(0) - expected).abs() < 1e-6);
    }
}
