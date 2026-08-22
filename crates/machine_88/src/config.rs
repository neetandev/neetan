//! Machine model definitions and clock configuration for the PC-8801 family.

/// PC-8801 machine model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pc8801Model {
    /// PC-8801MC: internal OPNA, dictionary ROM, 4/8 MHz main CPU, 128 KiB
    /// extension RAM, CD-ROM support
    PC8801MC,
}

impl Pc8801Model {
    /// Main CPU clock in Hz for the slow (4 MHz) switch position.
    pub const fn main_clock_hz_4mhz(self) -> u32 {
        match self {
            Pc8801Model::PC8801MC => 3_993_600,
        }
    }

    /// Main CPU clock in Hz for the fast (8 MHz) switch position.
    pub const fn main_clock_hz_8mhz(self) -> u32 {
        match self {
            Pc8801Model::PC8801MC => 7_987_200,
        }
    }

    /// Sub (disk unit) CPU clock in Hz. Always the 4 MHz part.
    pub const fn sub_clock_hz(self) -> u32 {
        match self {
            Pc8801Model::PC8801MC => 3_993_600,
        }
    }

    /// Main RAM size in bytes.
    pub const fn main_ram_size(self) -> usize {
        match self {
            Pc8801Model::PC8801MC => 0x1_0000,
        }
    }

    /// Extension RAM size in bytes (four 32 KiB banks on the MA).
    pub const fn extension_ram_size(self) -> usize {
        match self {
            Pc8801Model::PC8801MC => 0x8000 * 4,
        }
    }

    /// Number of selectable extension RAM banks.
    pub const fn extension_ram_banks(self) -> usize {
        match self {
            Pc8801Model::PC8801MC => 4,
        }
    }

    /// Whether the internal dictionary (jisyo) ROM is fitted.
    pub const fn has_dictionary_rom(self) -> bool {
        match self {
            Pc8801Model::PC8801MC => true,
        }
    }
}

impl std::fmt::Display for Pc8801Model {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pc8801Model::PC8801MC => formatter.write_str("PC8801MC"),
        }
    }
}

impl std::str::FromStr for Pc8801Model {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_uppercase().as_str() {
            "PC8801MC" => Ok(Pc8801Model::PC8801MC),
            _ => Err(format!("unknown PC-88 model '{text}', expected PC8801MC")),
        }
    }
}

/// Main CPU clock switch position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockSelect {
    /// 4 MHz position.
    FourMhz,
    /// 8 MHz position.
    EightMhz,
}

impl ClockSelect {
    /// Resolves the switch position to a main CPU clock in Hz for `model`.
    pub const fn main_clock_hz(self, model: Pc8801Model) -> u32 {
        match self {
            ClockSelect::FourMhz => model.main_clock_hz_4mhz(),
            ClockSelect::EightMhz => model.main_clock_hz_8mhz(),
        }
    }
}

save_state::runtime_state_enum! {
/// PC-8801 BASIC boot mode (DIP setting). Affects the 0xF000-0xFFFF memory
/// decode: in V1S and N-family modes that region is always main RAM, while V1H
/// and V2 allow text VRAM to appear there under port 0x32 bit 4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootMode {
    /// N88-BASIC V1 standard speed.
    V1S = 0,
    /// N88-BASIC V1 high speed.
    V1H = 1,
    /// N88-BASIC V2.
    V2 = 2,
    /// Plain N-BASIC.
    N = 3,
    /// N80-BASIC, for PC-8001mkII compatibility.
    N80 = 4,
    /// N80SR-BASIC, for PC-8001mkIISR compatibility.
    N80SR = 5,
}}

impl BootMode {
    /// Whether the 0xF000-0xFFFF region is forced to main RAM regardless of the
    /// text mode bit. True for V1S and N-family modes.
    pub const fn forces_high_ram(self) -> bool {
        match self {
            BootMode::V1S | BootMode::N | BootMode::N80 | BootMode::N80SR => true,
            BootMode::V1H | BootMode::V2 => false,
        }
    }

    /// Whether this is one of the PC-8001-compatible N-family modes.
    pub const fn is_n_family(self) -> bool {
        matches!(self, BootMode::N | BootMode::N80 | BootMode::N80SR)
    }

    /// Whether this is one of the PC-8001mkII/SR N80 graphics modes.
    pub const fn is_n80_family(self) -> bool {
        matches!(self, BootMode::N80 | BootMode::N80SR)
    }

    /// Whether this is the PC-8001mkIISR-compatible N80SR mode.
    pub const fn is_n80sr(self) -> bool {
        matches!(self, BootMode::N80SR)
    }
}

impl std::fmt::Display for BootMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BootMode::V1S => formatter.write_str("v1s"),
            BootMode::V1H => formatter.write_str("v1h"),
            BootMode::V2 => formatter.write_str("v2"),
            BootMode::N => formatter.write_str("n"),
            BootMode::N80 => formatter.write_str("n80"),
            BootMode::N80SR => formatter.write_str("n80sr"),
        }
    }
}

impl std::str::FromStr for BootMode {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_lowercase().as_str() {
            "v1s" => Ok(BootMode::V1S),
            "v1h" => Ok(BootMode::V1H),
            "v2" => Ok(BootMode::V2),
            "n" => Ok(BootMode::N),
            "n80" | "n80v1" => Ok(BootMode::N80),
            "n80sr" | "n80v2" => Ok(BootMode::N80SR),
            _ => Err(format!(
                "unknown PC-88 boot mode '{text}', expected v1s, v1h, v2, n, n80 or n80sr"
            )),
        }
    }
}

/// Immutable clock configuration for a PC-8801 machine variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockConfig {
    /// Main CPU clock frequency in Hz.
    pub main_clock_hz: u32,
    /// Sub (disk unit) CPU clock frequency in Hz.
    pub sub_clock_hz: u32,
    /// Audio output sample rate in Hz.
    pub sample_rate: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_string_round_trips() {
        assert_eq!("PC8801MC".parse::<Pc8801Model>(), Ok(Pc8801Model::PC8801MC));
        assert_eq!("pc8801mc".parse::<Pc8801Model>(), Ok(Pc8801Model::PC8801MC));
        assert_eq!(Pc8801Model::PC8801MC.to_string(), "PC8801MC");
        assert!("PC9801RA".parse::<Pc8801Model>().is_err());
    }

    #[test]
    fn boot_mode_string_round_trips() {
        for (text, mode) in [
            ("v1s", BootMode::V1S),
            ("v1h", BootMode::V1H),
            ("v2", BootMode::V2),
            ("n", BootMode::N),
            ("n80", BootMode::N80),
            ("n80sr", BootMode::N80SR),
        ] {
            assert_eq!(text.parse::<BootMode>(), Ok(mode));
            assert_eq!(mode.to_string(), text);
        }
        assert_eq!("n80v1".parse::<BootMode>(), Ok(BootMode::N80));
        assert_eq!("n80v2".parse::<BootMode>(), Ok(BootMode::N80SR));
        assert!("v3".parse::<BootMode>().is_err());
    }
}
