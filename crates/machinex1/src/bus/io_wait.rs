//! Bitmap VRAM wait-state timing for the X1.
//!
//! Based on the data gathered by Mr.Sato (http://x1center.org/)
//!
//! Bitmap VRAM is shared between the Z80 and the video circuitry, so a CPU
//! access has to wait for a free memory slot. The stall depends on the screen
//! mode (40/80 columns and, on the turbo, the 24 kHz hi-res scan) and, at fine
//! grain, on the phase of the access within the video refresh. We only model
//! the mode-dependent mean stall: each mean is expressed as the exact fraction
//! `sum / VRAM_WAIT_PERIOD` and applied through a running fractional carry (see
//! [`super::X1Bus::charge_vram_access_wait`]), so the long-run average matches
//! the mean without per-access rounding bias.
//!
//! `VRAM_WAIT_SUM_*` is the total contention accumulated over one full
//! `VRAM_WAIT_PERIOD`-cycle video period; dividing by the period gives the mean
//! wait per access (40-col ~3.99, 80-col ~1.78, 40-col hi-res ~2.50, 80-col
//! hi-res ~1.01 cycles).

/// CPU clocks the per-access mean is averaged over, doubling as the fixed-point
/// denominator of the fractional-carry accumulator.
pub(crate) const VRAM_WAIT_PERIOD: i64 = 2112;

/// Total contention over one video period in 40-column normal-scan mode.
pub(crate) const VRAM_WAIT_SUM_40: i64 = 8420;

/// Total contention over one video period in 80-column normal-scan mode.
pub(crate) const VRAM_WAIT_SUM_80: i64 = 3752;

/// Total contention over one video period in 40-column 24 kHz hi-res mode.
pub(crate) const VRAM_WAIT_SUM_40_HIRES: i64 = 5277;

/// Total contention over one video period in 80-column 24 kHz hi-res mode.
pub(crate) const VRAM_WAIT_SUM_80_HIRES: i64 = 2136;
