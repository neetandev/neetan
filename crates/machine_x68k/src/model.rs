//! X68000 model selection and fixed hardware properties.

use common::CpuMode;

/// X68000 machine model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X68kModel {
    /// Original 10 MHz X68000 with VINAS video and SASI storage.
    X68000,
    /// X68000 SUPER with VICON video and internal SCSI.
    X68000Super,
    /// X68000 XVI with selectable 10/16.67 MHz operation.
    X68000Xvi,
}

/// X68000 video-controller generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X68kVideoController {
    /// Original VINAS controller.
    Vinas,
    /// VICON controller used by the SUPER and XVI.
    Vicon,
}

/// X68000 built-in storage-controller generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X68kStorageController {
    /// Original SASI interface.
    Sasi,
    /// SUPER/XVI internal SCSI interface.
    InternalScsi,
}

impl X68kStorageController {
    /// Returns the native hard-disk sector size in bytes.
    pub const fn sector_size(self) -> u16 {
        match self {
            Self::Sasi => 256,
            Self::InternalScsi => 512,
        }
    }
}

impl X68kModel {
    /// Returns the selected CPU input clock in Hz.
    pub const fn cpu_clock_hz(self, cpu_mode: CpuMode) -> u32 {
        match (self, cpu_mode) {
            (Self::X68000Xvi, CpuMode::High) => 16_666_667,
            _ => 10_000_000,
        }
    }

    /// Returns the installed video-controller generation.
    pub const fn video_controller(self) -> X68kVideoController {
        match self {
            Self::X68000 => X68kVideoController::Vinas,
            Self::X68000Super | Self::X68000Xvi => X68kVideoController::Vicon,
        }
    }

    /// Returns the built-in storage-controller generation.
    pub const fn storage_controller(self) -> X68kStorageController {
        match self {
            Self::X68000 => X68kStorageController::Sasi,
            Self::X68000Super | Self::X68000Xvi => X68kStorageController::InternalScsi,
        }
    }

    /// Returns whether an internal SCSI boot-ROM window is present.
    pub const fn has_internal_scsi(self) -> bool {
        matches!(self, Self::X68000Super | Self::X68000Xvi)
    }
}

impl std::fmt::Display for X68kModel {
    /// Formats the canonical model name.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::X68000 => "X68000",
            Self::X68000Super => "X68000SUPER",
            Self::X68000Xvi => "X68000XVI",
        })
    }
}

impl std::str::FromStr for X68kModel {
    type Err = String;

    /// Parses an X68000 model name.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_uppercase().as_str() {
            "X68000" => Ok(Self::X68000),
            "X68000SUPER" => Ok(Self::X68000Super),
            "X68000XVI" => Ok(Self::X68000Xvi),
            _ => Err(format!(
                "unknown X68000 model '{text}', expected X68000, X68000SUPER or X68000XVI"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_parse_and_format_canonically() {
        for (text, model) in [
            ("x68000", X68kModel::X68000),
            ("X68000super", X68kModel::X68000Super),
            ("x68000xvi", X68kModel::X68000Xvi),
        ] {
            assert_eq!(text.parse(), Ok(model));
            assert_eq!(model.to_string(), text.to_ascii_uppercase());
        }
    }

    #[test]
    fn storage_controller_selects_the_sector_size() {
        assert_eq!(X68kModel::X68000.storage_controller().sector_size(), 256);
        assert_eq!(
            X68kModel::X68000Super.storage_controller().sector_size(),
            512
        );
        assert_eq!(X68kModel::X68000Xvi.storage_controller().sector_size(), 512);
    }

    #[test]
    fn only_xvi_high_mode_changes_the_clock() {
        assert_eq!(X68kModel::X68000.cpu_clock_hz(CpuMode::High), 10_000_000);
        assert_eq!(
            X68kModel::X68000Super.cpu_clock_hz(CpuMode::Low),
            10_000_000
        );
        assert_eq!(X68kModel::X68000Xvi.cpu_clock_hz(CpuMode::Low), 10_000_000);
        assert_eq!(X68kModel::X68000Xvi.cpu_clock_hz(CpuMode::High), 16_666_667);
    }
}
