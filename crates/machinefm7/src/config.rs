//! Machine model definitions and clock configuration for the Fujitsu FM-7 family.

/// Main CPU clock of the base FM-7 in fast mode.
const MAIN_CLOCK_HZ: u32 = 1_798_000;
/// Main CPU clock of the base FM-7 in slow mode.
const MAIN_CLOCK_SLOW_HZ: u32 = 1_095_000;
/// Main CPU clock of the FM-77AV in fast mode while MMR or the relocatable
/// window is enabled. The address-translation overhead drops the peak clock from
/// 1.798 MHz to 1.565 MHz; the 2.016 MHz fast-MMR clock is an AV40 feature and is
/// out of scope.
const MAIN_CLOCK_MMR_HZ: u32 = 1_565_000;
/// Sub CPU clock in fast mode.
const SUB_CLOCK_HZ: u32 = 2_000_000;
/// Sub CPU clock in slow mode.
const SUB_CLOCK_SLOW_HZ: u32 = 999_000;

/// AY-3-8910 PSG input clock in Hz (the 4.9152 MHz master clock divided by 4).
pub(crate) const PSG_CLOCK_HZ: u32 = 1_228_800;
/// Reference tick clock for the fixed-frequency buzzer, in Hz. Any stable base
/// works for a fixed-frequency beeper; the main CPU fast clock is reused so the
/// reload divisor is derived from a documented constant.
pub(crate) const BEEPER_TICK_CLOCK_HZ: u32 = MAIN_CLOCK_HZ;
/// FM-7 buzzer output frequency in Hz.
pub(crate) const BEEPER_FREQUENCY_HZ: u32 = 1_200;
/// Duration the beeper one-shot stays gated, in milliseconds.
pub(crate) const BEEP_ONE_SHOT_MILLIS: u64 = 205;

/// Fujitsu FM-7 machine model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fm7Model {
    /// Base FM-7 (1982): F-BASIC ROM, 8-color digital video, AY-3-8910 sound.
    Fm7,
    /// Base FM-77AV (1985): initiator boot, MMR paging, 4096-color video,
    /// MB61VH010 ALU, YM2203 sound, serial keyboard encoder.
    Fm77Av,
}

impl Fm7Model {
    /// Main CPU clock in Hz (fast mode).
    pub const fn main_clock_hz(self) -> u32 {
        match self {
            Fm7Model::Fm7 | Fm7Model::Fm77Av => MAIN_CLOCK_HZ,
        }
    }

    /// Main CPU clock in Hz (slow mode).
    pub const fn main_clock_slow_hz(self) -> u32 {
        match self {
            Fm7Model::Fm7 | Fm7Model::Fm77Av => MAIN_CLOCK_SLOW_HZ,
        }
    }

    /// Sub CPU clock in Hz (fast mode).
    pub const fn sub_clock_hz(self) -> u32 {
        match self {
            Fm7Model::Fm7 | Fm7Model::Fm77Av => SUB_CLOCK_HZ,
        }
    }

    /// Sub CPU clock in Hz (slow mode).
    pub const fn sub_clock_slow_hz(self) -> u32 {
        match self {
            Fm7Model::Fm7 | Fm7Model::Fm77Av => SUB_CLOCK_SLOW_HZ,
        }
    }

    /// Main CPU clock in Hz while fast and MMR or the relocatable window is
    /// enabled. The FM-7 has no MMR, so it keeps its normal fast clock.
    pub const fn main_clock_mmr_hz(self) -> u32 {
        match self {
            Fm7Model::Fm7 => MAIN_CLOCK_HZ,
            Fm7Model::Fm77Av => MAIN_CLOCK_MMR_HZ,
        }
    }

    /// Whether the MMR paging unit is present.
    pub const fn has_mmr(self) -> bool {
        match self {
            Fm7Model::Fm7 => false,
            Fm7Model::Fm77Av => true,
        }
    }

    /// Whether the MB61VH010 graphics ALU is present.
    pub const fn has_alu(self) -> bool {
        match self {
            Fm7Model::Fm7 => false,
            Fm7Model::Fm77Av => true,
        }
    }

    /// Whether the YM2203 (OPN) sound source replaces the AY-3-8910.
    pub const fn has_opn(self) -> bool {
        match self {
            Fm7Model::Fm7 => false,
            Fm7Model::Fm77Av => true,
        }
    }

    /// Whether the analog palette (4096-color) hardware is present.
    pub const fn has_analog_palette(self) -> bool {
        match self {
            Fm7Model::Fm7 => false,
            Fm7Model::Fm77Av => true,
        }
    }

    /// Whether the boot region is RAM seeded from the initiator ROM.
    pub const fn has_boot_ram(self) -> bool {
        match self {
            Fm7Model::Fm7 => false,
            Fm7Model::Fm77Av => true,
        }
    }

    /// Number of floppy drive selects the machine reports.
    pub const fn drive_count(self) -> u8 {
        match self {
            Fm7Model::Fm7 => 4,
            Fm7Model::Fm77Av => 2,
        }
    }

    /// Whether the display sub CPU steals bus cycles from the main CPU instead of
    /// being throttled to a third of its clock while it touches VRAM.
    ///
    /// The FM-7 has cycle steal disabled by default, so VRAM access divides the
    /// sub clock by three; the FM-77AV enables it, removing that contention.
    pub const fn cycle_steal_default(self) -> bool {
        match self {
            Fm7Model::Fm7 => false,
            Fm7Model::Fm77Av => true,
        }
    }
}

impl std::fmt::Display for Fm7Model {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fm7Model::Fm7 => formatter.write_str("FM7"),
            Fm7Model::Fm77Av => formatter.write_str("FM77AV"),
        }
    }
}

impl std::str::FromStr for Fm7Model {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_uppercase().as_str() {
            "FM7" => Ok(Fm7Model::Fm7),
            "FM77AV" => Ok(Fm7Model::Fm77Av),
            _ => Err(format!(
                "unknown FM-7 model '{text}', expected FM7 or FM77AV"
            )),
        }
    }
}

/// Boot ROM selection for the FM-7 family.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum BootMode {
    #[default]
    /// Use the BASIC boot ROM path.
    Basic,
    /// Use the DOS boot ROM path for DOS-style IPL media.
    Dos,
}

impl std::fmt::Display for BootMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BootMode::Basic => formatter.write_str("basic"),
            BootMode::Dos => formatter.write_str("dos"),
        }
    }
}

impl std::str::FromStr for BootMode {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_lowercase().as_str() {
            "basic" => Ok(BootMode::Basic),
            "dos" => Ok(BootMode::Dos),
            _ => Err(format!(
                "unknown FM-7 boot mode '{text}', expected basic or dos"
            )),
        }
    }
}

/// Immutable clock configuration for an FM-7 machine variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockConfig {
    /// Main CPU clock frequency in Hz.
    pub main_clock_hz: u32,
    /// Audio output sample rate in Hz.
    pub sample_rate: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_string_round_trips() {
        for (text, model) in [("FM7", Fm7Model::Fm7), ("FM77AV", Fm7Model::Fm77Av)] {
            assert_eq!(text.parse::<Fm7Model>(), Ok(model));
            assert_eq!(text.to_ascii_lowercase().parse::<Fm7Model>(), Ok(model));
            assert_eq!(model.to_string(), text);
        }
        assert!("PC6001".parse::<Fm7Model>().is_err());
    }

    #[test]
    fn boot_mode_string_round_trips() {
        for (text, mode) in [("basic", BootMode::Basic), ("dos", BootMode::Dos)] {
            assert_eq!(text.parse::<BootMode>(), Ok(mode));
            assert_eq!(text.to_ascii_uppercase().parse::<BootMode>(), Ok(mode));
            assert_eq!(mode.to_string(), text);
        }
        assert!("auto".parse::<BootMode>().is_err());
        assert!("tape".parse::<BootMode>().is_err());
        assert_eq!(BootMode::default(), BootMode::Basic);
    }

    #[test]
    fn av_descriptors_track_the_model() {
        assert!(!Fm7Model::Fm7.has_mmr());
        assert!(Fm7Model::Fm77Av.has_mmr());

        assert!(!Fm7Model::Fm7.has_opn());
        assert!(Fm7Model::Fm77Av.has_opn());

        assert!(!Fm7Model::Fm7.has_alu());
        assert!(Fm7Model::Fm77Av.has_alu());

        assert_eq!(Fm7Model::Fm7.drive_count(), 4);
        assert_eq!(Fm7Model::Fm77Av.drive_count(), 2);
    }
}
