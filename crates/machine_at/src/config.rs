//! PC/AT machine models and clock configuration.

use std::{fmt, str::FromStr};

use common::CpuMode;

/// PIT input clock in hertz (14.31818 MHz / 12).
pub const PIT_CLOCK_HZ: u32 = 1_193_182;

/// Standard AT ISA bus clock in hertz (about 8.33 MHz).
///
/// The CS4031 divides the local bus down to this rate (25 MHz / 3, 33 MHz / 4),
/// so every off-chip I/O and VGA VRAM access is paced at roughly 8.33 MHz on
/// both machine variants regardless of the CPU clock. This is the rate that
/// governs I/O-bound timing loops; cached CPU work runs at the full core clock.
pub const ISA_BUS_CLOCK_HZ: u32 = 8_333_333;

/// ISA command cycles for an 8-bit I/O access (standard-length ISA cycle).
const ISA_8BIT_CYCLES: u32 = 6;
/// ISA command cycles for a 16-bit I/O access.
const ISA_16BIT_CYCLES: u32 = 3;
/// ISA command cycles for a VGA VRAM window access.
const ISA_VRAM_CYCLES: u32 = 2;

/// PC/AT machine model. Both variants are i486DX2 parts on the CS4031 board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AtModel {
    /// i486DX2-50 (25 MHz bus, 50 MHz core).
    At486Dx50,
    /// i486DX2-66 (33 MHz bus, 66 MHz core).
    #[default]
    At486Dx66,
}

impl AtModel {
    /// Returns the doubled i486DX2 core clock in hertz (the high cpu-mode).
    pub const fn core_clock_hz(self) -> u32 {
        match self {
            AtModel::At486Dx50 => 50_000_000,
            AtModel::At486Dx66 => 66_000_000,
        }
    }

    /// Returns the ISA/DRAM bus clock in hertz (the low cpu-mode: clock
    /// doubler off).
    pub const fn bus_clock_hz(self) -> u32 {
        match self {
            AtModel::At486Dx50 => 25_000_000,
            AtModel::At486Dx66 => 33_000_000,
        }
    }

    /// Returns the running CPU clock for the selected cpu-mode: the bus clock in
    /// low mode (the clock doubler disabled) or the doubled core clock in high
    /// mode. AT486DX50 runs 25 / 50 MHz and AT486DX66 runs 33 / 66 MHz.
    pub const fn cpu_clock_hz(self, cpu_mode: CpuMode) -> u32 {
        match cpu_mode {
            CpuMode::Low => self.bus_clock_hz(),
            CpuMode::High => self.core_clock_hz(),
        }
    }

    /// Returns the installed RAM size in bytes.
    pub const fn ram_size(self) -> u32 {
        match self {
            AtModel::At486Dx50 | AtModel::At486Dx66 => 16 << 20,
        }
    }
}

impl fmt::Display for AtModel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            AtModel::At486Dx50 => "AT486DX50",
            AtModel::At486Dx66 => "AT486DX66",
        };
        formatter.write_str(name)
    }
}

impl FromStr for AtModel {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "at486dx50" | "486dx50" | "dx50" => Ok(AtModel::At486Dx50),
            "at486dx66" | "486dx66" | "dx66" => Ok(AtModel::At486Dx66),
            _ => Err(()),
        }
    }
}

/// BIOS boot device order, stored in the AMI CMOS flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AtBootDevice {
    /// Boot from A: first, then C:.
    #[default]
    FloppyFirst,
    /// Boot from C: first, then A:.
    HddFirst,
}

/// Derived clock configuration for a running machine.
#[derive(Debug, Clone, Copy)]
pub struct ClockConfig {
    /// i486DX2 core clock in hertz.
    pub cpu_clock_hz: u32,
    /// Audio sample rate in hertz.
    pub sample_rate: u32,
    /// Core-cycle penalty for an 8-bit ISA I/O access.
    pub io_8bit_wait_cycles: i64,
    /// Core-cycle penalty for a 16-bit ISA I/O access.
    pub io_16bit_wait_cycles: i64,
    /// Core-cycle penalty for a VGA VRAM window access.
    pub vga_memory_wait_cycles: i64,
}

impl ClockConfig {
    /// Builds the clock configuration and derives the ISA wait-state penalties
    /// for the given core clock. Off-chip accesses are paced at the fixed ISA
    /// bus clock, so their core-cycle cost scales with `cpu_clock_hz`.
    pub fn new(cpu_clock_hz: u32, sample_rate: u32) -> Self {
        let isa_wait = |cycles: u32| -> i64 {
            (u64::from(cpu_clock_hz) * u64::from(cycles) / u64::from(ISA_BUS_CLOCK_HZ)) as i64
        };
        Self {
            cpu_clock_hz,
            sample_rate,
            io_8bit_wait_cycles: isa_wait(ISA_8BIT_CYCLES),
            io_16bit_wait_cycles: isa_wait(ISA_16BIT_CYCLES),
            vga_memory_wait_cycles: isa_wait(ISA_VRAM_CYCLES),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_round_trips_through_string() {
        for model in [AtModel::At486Dx50, AtModel::At486Dx66] {
            let text = model.to_string();
            assert_eq!(AtModel::from_str(&text), Ok(model));
        }
        assert_eq!(AtModel::from_str("dx66"), Ok(AtModel::At486Dx66));
        assert_eq!(AtModel::from_str("nonsense"), Err(()));
    }

    #[test]
    fn clock_rates_match_model() {
        assert_eq!(AtModel::At486Dx50.cpu_clock_hz(CpuMode::High), 50_000_000);
        assert_eq!(AtModel::At486Dx50.cpu_clock_hz(CpuMode::Low), 25_000_000);
        assert_eq!(AtModel::At486Dx66.cpu_clock_hz(CpuMode::High), 66_000_000);
        assert_eq!(AtModel::At486Dx66.cpu_clock_hz(CpuMode::Low), 33_000_000);
        assert_eq!(AtModel::At486Dx66.bus_clock_hz(), 33_000_000);
    }

    #[test]
    fn both_models_install_sixteen_mebibytes() {
        assert_eq!(AtModel::At486Dx50.ram_size(), 16 << 20);
        assert_eq!(AtModel::At486Dx66.ram_size(), 16 << 20);
    }
}
