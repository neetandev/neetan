//! Machine model definitions and clock configuration for the PC-6000/PC-6600 family.

/// Main CPU clock for the ~4 MHz generation (PC-6001, mkII, PC-6601).
const CLOCK_HZ_4MHZ: u32 = 3_993_600;
/// Main CPU clock for the SR generation (NTSC colorburst, 3.579545 MHz).
const CLOCK_HZ_SR: u32 = 3_579_545;

const WORK_RAM_16K: usize = 0x4000;
const WORK_RAM_64K: usize = 0x1_0000;
/// SR physical address space: 16 x 8 KiB pages into 1 MiB.
const PHYSICAL_SPACE_SR: usize = 0x10_0000;

/// PC-6000/PC-6600 machine model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pc6000Model {
    /// PC-6001: MC6847 base video, AY-3-8910 PSG, no voice.
    Pc6001,
    /// PC-6001mkII: extended video modes, AY-3-8910, uPD7752 voice.
    Pc6001Mk2,
    /// PC-6601: mkII video, AY-3-8910, voice, non-intelligent built-in 5.25" FDD.
    Pc6601,
    /// PC-6001mkIISR: native SR video, YM2203 (OPN), voice.
    Pc6001Mk2Sr,
    /// PC-6601SR: native SR video, YM2203 (OPN), voice, non-intelligent built-in 3.5" FDD.
    Pc6601Sr,
}

impl Pc6000Model {
    /// Main CPU clock in Hz.
    pub const fn main_clock_hz(self) -> u32 {
        match self {
            Pc6000Model::Pc6001 | Pc6000Model::Pc6001Mk2 | Pc6000Model::Pc6601 => CLOCK_HZ_4MHZ,
            Pc6000Model::Pc6001Mk2Sr | Pc6000Model::Pc6601Sr => CLOCK_HZ_SR,
        }
    }

    /// Work RAM size in bytes. The SR models expose a 1 MiB banked physical space.
    pub const fn work_ram_size(self) -> usize {
        match self {
            Pc6000Model::Pc6001 => WORK_RAM_16K,
            Pc6000Model::Pc6001Mk2 | Pc6000Model::Pc6601 => WORK_RAM_64K,
            Pc6000Model::Pc6001Mk2Sr | Pc6000Model::Pc6601Sr => PHYSICAL_SPACE_SR,
        }
    }

    /// Whether this is an SR-generation machine (YM2203, native SR video, banking).
    pub const fn is_sr(self) -> bool {
        match self {
            Pc6000Model::Pc6001 | Pc6000Model::Pc6001Mk2 | Pc6000Model::Pc6601 => false,
            Pc6000Model::Pc6001Mk2Sr | Pc6000Model::Pc6601Sr => true,
        }
    }

    /// Whether the YM2203 (OPN) replaces the discrete AY-3-8910 PSG.
    pub const fn has_fm(self) -> bool {
        match self {
            Pc6000Model::Pc6001 | Pc6000Model::Pc6001Mk2 | Pc6000Model::Pc6601 => false,
            Pc6000Model::Pc6001Mk2Sr | Pc6000Model::Pc6601Sr => true,
        }
    }

    /// Whether the uPD7752 voice synthesizer is fitted (mkII and later).
    pub const fn has_voice(self) -> bool {
        match self {
            Pc6000Model::Pc6001 => false,
            Pc6000Model::Pc6001Mk2
            | Pc6000Model::Pc6601
            | Pc6000Model::Pc6001Mk2Sr
            | Pc6000Model::Pc6601Sr => true,
        }
    }

    /// Whether the machine has a non-intelligent built-in uPD765A floppy drive.
    pub const fn has_builtin_fdd(self) -> bool {
        match self {
            Pc6000Model::Pc6001 | Pc6000Model::Pc6001Mk2 | Pc6000Model::Pc6001Mk2Sr => false,
            Pc6000Model::Pc6601 | Pc6000Model::Pc6601Sr => true,
        }
    }
}

impl std::fmt::Display for Pc6000Model {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pc6000Model::Pc6001 => formatter.write_str("PC6001"),
            Pc6000Model::Pc6001Mk2 => formatter.write_str("PC6001MK2"),
            Pc6000Model::Pc6601 => formatter.write_str("PC6601"),
            Pc6000Model::Pc6001Mk2Sr => formatter.write_str("PC6001MK2SR"),
            Pc6000Model::Pc6601Sr => formatter.write_str("PC6601SR"),
        }
    }
}

impl std::str::FromStr for Pc6000Model {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_uppercase().as_str() {
            "PC6001" => Ok(Pc6000Model::Pc6001),
            "PC6001MK2" => Ok(Pc6000Model::Pc6001Mk2),
            "PC6601" => Ok(Pc6000Model::Pc6601),
            "PC6001MK2SR" => Ok(Pc6000Model::Pc6001Mk2Sr),
            "PC6601SR" => Ok(Pc6000Model::Pc6601Sr),
            _ => Err(format!(
                "unknown PC-6000 model '{text}', expected PC6001, PC6001MK2, PC6601, PC6001MK2SR or PC6601SR"
            )),
        }
    }
}

/// Immutable clock configuration for a PC-6000 machine variant.
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
        for (text, model) in [
            ("PC6001", Pc6000Model::Pc6001),
            ("PC6001MK2", Pc6000Model::Pc6001Mk2),
            ("PC6601", Pc6000Model::Pc6601),
            ("PC6001MK2SR", Pc6000Model::Pc6001Mk2Sr),
            ("PC6601SR", Pc6000Model::Pc6601Sr),
        ] {
            assert_eq!(text.parse::<Pc6000Model>(), Ok(model));
            assert_eq!(text.to_ascii_lowercase().parse::<Pc6000Model>(), Ok(model));
            assert_eq!(model.to_string(), text);
        }
        assert!("PC8801MC".parse::<Pc6000Model>().is_err());
    }

    #[test]
    fn sr_models_use_the_colorburst_clock() {
        assert_eq!(Pc6000Model::Pc6001.main_clock_hz(), CLOCK_HZ_4MHZ);
        assert_eq!(Pc6000Model::Pc6601Sr.main_clock_hz(), CLOCK_HZ_SR);
        assert!(Pc6000Model::Pc6001Mk2Sr.is_sr());
        assert!(!Pc6000Model::Pc6001.is_sr());
    }
}
