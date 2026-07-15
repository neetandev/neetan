//! Machine model definitions and clock configuration for the Sharp X1 family.

/// Main CPU clock: all X1 machines run a 4 MHz Z80A.
const CLOCK_HZ_4MHZ: u32 = 4_000_000;

/// Base X1 work RAM: a single 64 KiB bank.
const WORK_RAM_64K: usize = 0x1_0000;

/// Base X1 IPL ROM size.
const IPL_ROM_4K: usize = 0x1000;
/// X1 turbo IPL ROM size.
const IPL_ROM_32K: usize = 0x8000;

/// Sharp X1 machine model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X1Model {
    /// Base Sharp X1 (CZ-800C class): 4 KiB IPL, 8-color digital video, PIO FDC.
    X1,
    /// X1 turbo (CZ-850C, model 10): 32 KiB IPL, RAM banking, DMA, SIO, kanji.
    X1Turbo,
}

impl X1Model {
    /// Main CPU clock in Hz.
    pub const fn main_clock_hz(self) -> u32 {
        match self {
            X1Model::X1 | X1Model::X1Turbo => CLOCK_HZ_4MHZ,
        }
    }

    /// Work RAM size in bytes.
    pub const fn work_ram_size(self) -> usize {
        match self {
            X1Model::X1 | X1Model::X1Turbo => WORK_RAM_64K,
        }
    }

    /// IPL ROM size in bytes.
    pub const fn ipl_rom_size(self) -> usize {
        match self {
            X1Model::X1 => IPL_ROM_4K,
            X1Model::X1Turbo => IPL_ROM_32K,
        }
    }

    /// Whether this is a turbo-generation machine (RAM banking, DMA, SIO, kanji).
    pub const fn is_turbo(self) -> bool {
        match self {
            X1Model::X1 => false,
            X1Model::X1Turbo => true,
        }
    }

    /// Whether a Z80 DMA controller is fitted (the turbo FDC is DMA-driven).
    pub const fn has_dma(self) -> bool {
        match self {
            X1Model::X1 => false,
            X1Model::X1Turbo => true,
        }
    }

    /// Whether a Z80 SIO is fitted (RS-232C on channel 0, mouse on channel 1).
    pub const fn has_sio(self) -> bool {
        match self {
            X1Model::X1 => false,
            X1Model::X1Turbo => true,
        }
    }

    /// Whether the kanji ROM and kanji text VRAM plane are present.
    pub const fn has_kanji(self) -> bool {
        match self {
            X1Model::X1 => false,
            X1Model::X1Turbo => true,
        }
    }

    /// Whether the CZ-8BS1 FM sound board (YM2151 + paired Z80 CTC) is fitted.
    ///
    /// The FM board was an optional expansion on any model, but is the standard
    /// pairing with the turbo and is modeled as a fixed part of it; the base X1
    /// has no FM.
    pub const fn has_fm(self) -> bool {
        match self {
            X1Model::X1 => false,
            X1Model::X1Turbo => true,
        }
    }

    /// Whether the 400-line (24 kHz) hi-res video mode is available.
    pub const fn has_hires(self) -> bool {
        match self {
            X1Model::X1 => false,
            X1Model::X1Turbo => true,
        }
    }
}

impl std::fmt::Display for X1Model {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            X1Model::X1 => formatter.write_str("X1"),
            X1Model::X1Turbo => formatter.write_str("X1TURBO"),
        }
    }
}

impl std::str::FromStr for X1Model {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_uppercase().as_str() {
            "X1" => Ok(X1Model::X1),
            "X1TURBO" => Ok(X1Model::X1Turbo),
            _ => Err(format!("unknown X1 model '{text}', expected X1 or X1TURBO")),
        }
    }
}

pub use device::subcontroller_x1::X1KeyboardMode;

/// Immutable clock configuration for an X1 machine variant.
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
        for (text, model) in [("X1", X1Model::X1), ("X1TURBO", X1Model::X1Turbo)] {
            assert_eq!(text.parse::<X1Model>(), Ok(model));
            assert_eq!(text.to_ascii_lowercase().parse::<X1Model>(), Ok(model));
            assert_eq!(model.to_string(), text);
        }
        assert!("PC6001".parse::<X1Model>().is_err());
    }

    #[test]
    fn turbo_descriptors_track_the_generation() {
        assert!(!X1Model::X1.is_turbo());
        assert!(X1Model::X1Turbo.is_turbo());

        assert_eq!(X1Model::X1.ipl_rom_size(), IPL_ROM_4K);
        assert_eq!(X1Model::X1Turbo.ipl_rom_size(), IPL_ROM_32K);

        // The FM board is a fixed part of the turbo only.
        assert!(!X1Model::X1.has_fm());
        assert!(X1Model::X1Turbo.has_fm());
    }
}
