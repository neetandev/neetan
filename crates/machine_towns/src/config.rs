//! Machine model definitions and clock configuration for the FM Towns family.

/// FM Towns machine model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TownsModel {
    /// Base FM Towns (model 1/2 class): 16 MHz, 1x CD-ROM. Wired to the 386SX
    /// CPU model so titles tuned for the base Towns / Marty run at their
    /// intended speed.
    FmTowns,
    /// FM Towns II CX: i386DX at 16 MHz (20 MHz in high mode), 1x CD-ROM.
    FmTownsIICx,
    /// FM Towns II MX: i486DX2 at 66 MHz (33 MHz in low mode), 2x CD-ROM.
    FmTownsIIMx,
}

/// Boot-device selection driving the SYSROM IPL via the CMOS boot-device byte.
///
/// The IPL is LLE: there is no HLE DOS entry as on the PC-98, so `--boot-device
/// dos` is translated to [`TownsBootDevice::Auto`] by the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TownsBootDevice {
    /// CD-ROM when a disc is inserted, floppy otherwise.
    #[default]
    Auto,
    /// Boot from floppy.
    Floppy,
    /// Boot from the SCSI hard disk.
    Hdd,
    /// Boot from CD-ROM.
    Cd,
}

impl TownsModel {
    /// Main CPU clock in Hz for the selected CPU mode. The base model runs at
    /// a fixed 16 MHz in both modes. The CX pair matches the real 386DX lineup
    /// (CX at 16 MHz, HG at 20 MHz); the MX pair matches the 486 lineup
    /// (MA-class 33 MHz, MX 66 MHz).
    pub const fn cpu_clock_hz(self, mode: common::CpuMode) -> u32 {
        match self {
            // The base machines have no clock switch; both modes run at 16 MHz.
            TownsModel::FmTowns => match mode {
                common::CpuMode::Low => 16_000_000,
                common::CpuMode::High => 16_000_000,
            },
            TownsModel::FmTownsIICx => match mode {
                common::CpuMode::Low => 16_000_000,
                common::CpuMode::High => 20_000_000,
            },
            TownsModel::FmTownsIIMx => match mode {
                common::CpuMode::Low => 33_000_000,
                common::CpuMode::High => 66_000_000,
            },
        }
    }

    /// Extended RAM size in bytes (main RAM below 1 MiB is separate).
    pub const fn extended_ram_size(self) -> usize {
        // Defaults every model to 8 MiB total; 1 MiB is the low map, the
        // rest is the extended region from 0x00100000. The real base model
        // 1/2 shipped 1-2 MiB, but the larger map is compatibility-safe.
        0x0070_0000
    }

    /// Machine identity bytes returned by I/O ports 0x0030 (low) and 0x0031
    /// (high). The low byte encodes the CPU class, the high byte the model.
    pub const fn machine_id(self) -> (u8, u8) {
        match self {
            // i386 class (0x01), base model 1/2 (0x01).
            TownsModel::FmTowns => (0x01, 0x01),
            // i386DX class (0x01), model CX (0x05).
            TownsModel::FmTownsIICx => (0x01, 0x05),
            // i486DX class (0x02), model MX (0x0C).
            TownsModel::FmTownsIIMx => (0x02, 0x0C),
        }
    }

    /// CD-ROM drive speed rating, used by the drive's compatibility timing
    /// mode (1 for the CX's 1x drive, 2 for the MX's 2x drive).
    pub const fn cd_drive_speed(self) -> u32 {
        match self {
            TownsModel::FmTowns => 1,
            TownsModel::FmTownsIICx => 1,
            TownsModel::FmTownsIIMx => 2,
        }
    }

    /// Whether the high-resolution CRTC (the 1024x768 / 16M-color "image out"
    /// register file at I/O 0x0470-0x0477 and the high-res VRAM windows) is
    /// present. Only the MX-class machines carry it.
    pub const fn high_res_available(self) -> bool {
        match self {
            TownsModel::FmTowns => false,
            TownsModel::FmTownsIICx => false,
            TownsModel::FmTownsIIMx => true,
        }
    }
}

impl std::fmt::Display for TownsModel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TownsModel::FmTowns => formatter.write_str("fmtowns"),
            TownsModel::FmTownsIICx => formatter.write_str("fmtownsiicx"),
            TownsModel::FmTownsIIMx => formatter.write_str("fmtownsiimx"),
        }
    }
}

impl std::str::FromStr for TownsModel {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_lowercase().as_str() {
            "fmtowns" | "towns" | "townsbase" | "base" => Ok(TownsModel::FmTowns),
            "fmtownsiicx" | "townscx" | "townsiicx" | "cx" => Ok(TownsModel::FmTownsIICx),
            "fmtownsiimx" | "townsmx" | "townsiimx" | "mx" => Ok(TownsModel::FmTownsIIMx),
            _ => Err(format!(
                "unknown FM Towns model '{text}', expected fmtowns, fmtownsiicx or fmtownsiimx"
            )),
        }
    }
}

/// Digital pad type plugged into game port 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TownsPadType {
    /// 2-button pad.
    TwoButton,
    /// 6-button pad (extra buttons multiplexed on the COM line).
    #[default]
    SixButton,
}

impl std::str::FromStr for TownsPadType {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_lowercase().as_str() {
            "2" | "2button" | "two" => Ok(TownsPadType::TwoButton),
            "6" | "6button" | "six" => Ok(TownsPadType::SixButton),
            _ => Err(format!(
                "unknown FM Towns pad type '{text}', expected 2 or 6"
            )),
        }
    }
}

/// Immutable clock configuration for an FM Towns machine variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockConfig {
    /// Main CPU clock frequency in Hz.
    pub cpu_clock_hz: u32,
    /// Audio output sample rate in Hz.
    pub sample_rate: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_string_round_trips() {
        assert_eq!(
            "fmtownsiimx".parse::<TownsModel>(),
            Ok(TownsModel::FmTownsIIMx)
        );
        assert_eq!("MX".parse::<TownsModel>(), Ok(TownsModel::FmTownsIIMx));
        assert_eq!(TownsModel::FmTownsIIMx.to_string(), "fmtownsiimx");
        assert!("pc98".parse::<TownsModel>().is_err());
    }

    #[test]
    fn cx_model_string_round_trips() {
        assert_eq!(
            "fmtownsiicx".parse::<TownsModel>(),
            Ok(TownsModel::FmTownsIICx)
        );
        assert_eq!("townscx".parse::<TownsModel>(), Ok(TownsModel::FmTownsIICx));
        assert_eq!(
            "townsiicx".parse::<TownsModel>(),
            Ok(TownsModel::FmTownsIICx)
        );
        assert_eq!("CX".parse::<TownsModel>(), Ok(TownsModel::FmTownsIICx));
        assert_eq!(TownsModel::FmTownsIICx.to_string(), "fmtownsiicx");
    }

    #[test]
    fn base_model_string_round_trips() {
        assert_eq!("fmtowns".parse::<TownsModel>(), Ok(TownsModel::FmTowns));
        assert_eq!("towns".parse::<TownsModel>(), Ok(TownsModel::FmTowns));
        assert_eq!("base".parse::<TownsModel>(), Ok(TownsModel::FmTowns));
        assert_eq!(TownsModel::FmTowns.to_string(), "fmtowns");
    }

    #[test]
    fn base_machine_id_matches_hardware() {
        assert_eq!(TownsModel::FmTowns.machine_id(), (0x01, 0x01));
    }

    #[test]
    fn base_cpu_clock_is_16mhz_in_both_modes() {
        assert_eq!(
            TownsModel::FmTowns.cpu_clock_hz(common::CpuMode::Low),
            16_000_000
        );
        assert_eq!(
            TownsModel::FmTowns.cpu_clock_hz(common::CpuMode::High),
            16_000_000
        );
    }

    #[test]
    fn mx_machine_id_matches_hardware() {
        assert_eq!(TownsModel::FmTownsIIMx.machine_id(), (0x02, 0x0C));
    }

    #[test]
    fn cx_machine_id_matches_hardware() {
        assert_eq!(TownsModel::FmTownsIICx.machine_id(), (0x01, 0x05));
    }

    #[test]
    fn cpu_clocks_per_mode() {
        assert_eq!(
            TownsModel::FmTownsIICx.cpu_clock_hz(common::CpuMode::Low),
            16_000_000
        );
        assert_eq!(
            TownsModel::FmTownsIICx.cpu_clock_hz(common::CpuMode::High),
            20_000_000
        );
        assert_eq!(
            TownsModel::FmTownsIIMx.cpu_clock_hz(common::CpuMode::Low),
            33_000_000
        );
        assert_eq!(
            TownsModel::FmTownsIIMx.cpu_clock_hz(common::CpuMode::High),
            66_000_000
        );
    }
}
