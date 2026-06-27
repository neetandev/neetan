//! Machine model definitions and clock configuration for the PC-88VA2 family.

/// PC-88VA2 machine model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pc88VaModel {
    /// PC-88VA2: V30 at ~8 MHz, 512 KiB main RAM, OPNA sound.
    PC88VA2,
}

impl Pc88VaModel {
    /// Main CPU (V30) clock in Hz.
    pub const fn main_clock_hz(self) -> u32 {
        7_987_200
    }

    /// Sub (floppy unit) CPU clock in Hz. Always the 4 MHz Z80 part.
    pub const fn sub_clock_hz(self) -> u32 {
        3_993_600
    }

    /// Main RAM size in bytes.
    pub const fn main_ram_size(self) -> usize {
        0x8_0000
    }
}

impl std::fmt::Display for Pc88VaModel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("pc88va2")
    }
}

impl std::str::FromStr for Pc88VaModel {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_lowercase().as_str() {
            "pc88va2" => Ok(Pc88VaModel::PC88VA2),
            _ => Err(format!("unknown PC-88VA2 model '{text}', expected pc88va2")),
        }
    }
}

/// Immutable clock configuration for a PC-88VA2 machine variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockConfig {
    /// Main CPU (V30) clock frequency in Hz.
    pub main_clock_hz: u32,
    /// Sub (floppy unit) CPU clock frequency in Hz.
    pub sub_clock_hz: u32,
    /// Audio output sample rate in Hz.
    pub sample_rate: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_string_round_trips() {
        assert_eq!("pc88va2".parse::<Pc88VaModel>(), Ok(Pc88VaModel::PC88VA2));
        assert_eq!(Pc88VaModel::PC88VA2.to_string(), "pc88va2");
        assert_eq!("PC88VA2".parse::<Pc88VaModel>(), Ok(Pc88VaModel::PC88VA2));
        assert!("pc88va".parse::<Pc88VaModel>().is_err());
        assert!("va3".parse::<Pc88VaModel>().is_err());
    }
}
