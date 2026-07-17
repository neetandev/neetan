//! Exact clock conversion for the MSX family.

use crate::MsxClockProfile;

/// Scanlines in one Japanese NTSC frame.
pub(crate) const NTSC_TOTAL_SCANLINES: u16 = 262;
/// VDP master-clock ticks in one scanline.
pub(crate) const VDP_TICKS_PER_SCANLINE: u64 = 1_368;
/// VDP master-clock ticks in one base dot.
const VDP_TICKS_PER_DOT: u64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VdpDotTime {
    pub(crate) dot: u64,
    pub(crate) phase: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MsxClock {
    cpu_master_divisor: u64,
}

impl MsxClock {
    /// Creates a clock converter for one model profile.
    pub(crate) const fn new(profile: MsxClockProfile) -> Self {
        Self {
            cpu_master_divisor: profile.normal_cpu_divisor as u64,
        }
    }

    /// Converts a CPU cycle to an absolute VDP master tick.
    pub(crate) fn vdp_tick_at(self, cpu_cycle: u64) -> u64 {
        narrow_u128(u128::from(cpu_cycle) * u128::from(self.cpu_master_divisor))
    }

    /// Converts a CPU cycle to a base dot and master-tick phase.
    pub(crate) fn vdp_dot_at(self, cpu_cycle: u64) -> VdpDotTime {
        let ticks = u128::from(cpu_cycle) * u128::from(self.cpu_master_divisor);
        VdpDotTime {
            dot: narrow_u128(ticks / u128::from(VDP_TICKS_PER_DOT)),
            phase: (ticks % u128::from(VDP_TICKS_PER_DOT)) as u8,
        }
    }

    /// Returns the first CPU cycle at or after an absolute VDP tick.
    pub(crate) fn fire_cycle_at_vdp_tick(self, vdp_tick: u64) -> u64 {
        let divisor = u128::from(self.cpu_master_divisor);
        narrow_u128(u128::from(vdp_tick).div_ceil(divisor))
    }
}

/// Narrows an exact clock calculation with saturation.
fn narrow_u128(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MsxModel;

    #[test]
    /// Every selected model keeps the standard six-to-one master divisor.
    fn every_model_has_the_standard_clock_relationship() {
        for model in MsxModel::ALL {
            let profile = model.clock_profile();
            let clock = MsxClock::new(profile);
            assert_eq!(profile.master_clock_hz, 21_477_270);
            assert_eq!(profile.normal_cpu_divisor, 6);
            assert_eq!(clock.vdp_tick_at(1), 6);
            assert_eq!(clock.vdp_dot_at(1), VdpDotTime { dot: 1, phase: 2 });
            assert_eq!(clock.vdp_dot_at(2), VdpDotTime { dot: 3, phase: 0 });
            assert_eq!(clock.fire_cycle_at_vdp_tick(VDP_TICKS_PER_SCANLINE), 228);
            assert_eq!(
                clock.fire_cycle_at_vdp_tick(262 * VDP_TICKS_PER_SCANLINE),
                59_736
            );
        }
    }

    #[test]
    /// Long conversions retain the half-dot CPU phase without drift.
    fn long_conversion_retains_the_half_dot_phase() {
        let clock = MsxClock::new(MsxModel::Msx.clock_profile());
        let cpu_cycle = 10_000_001;
        assert_eq!(clock.vdp_tick_at(cpu_cycle), cpu_cycle * 6);
        assert_eq!(
            clock.vdp_dot_at(cpu_cycle),
            VdpDotTime {
                dot: cpu_cycle * 3 / 2,
                phase: 2,
            }
        );
    }
}
