/// Single-channel output sample from OPL/OPL2/Y8950 chips.
///
/// Contains one mono FM output sample.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct YmfmOutput1 {
    /// Per-channel sample data: `[FM]`.
    pub data: [i32; 1],
}

/// Four-channel output sample from the YM2203.
///
/// Contains one sample per output channel: `data[0]` is the FM output,
/// `data[1..4]` are the three SSG (PSG) channels. Values are signed
/// 32-bit integers that should be clamped to 16-bit range for playback.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct YmfmOutput4 {
    /// Per-channel sample data: `[FM, SSG-A, SSG-B, SSG-C]`.
    pub data: [i32; 4],
}

/// Output fidelity level controlling the internal sample rate.
///
/// Higher fidelity produces more samples per second, increasing accuracy
/// of the SSG resampling at the cost of more CPU. At a 4 MHz input clock
/// the effective output rates are:
///
/// | Fidelity | Output rate  |
/// |----------|-------------|
/// | `Max`    | clock / 4   |
/// | `Med`    | clock / 12  |
/// | `Min`    | clock / 24  |
#[repr(u8)]
#[derive(Clone, Copy)]
pub enum YmfmOpnFidelity {
    /// Highest fidelity (default). Matches the fastest SSG rate.
    Max = 0,
    /// Lowest fidelity. Matches the fastest FM rate.
    Min = 1,
    /// Medium fidelity. FM is never smeared across output samples.
    Med = 2,
}

/// Two-channel output sample from the OPN2 family (YM2612/YM3438/YMF276).
///
/// Contains one stereo FM sample: `data[0]` is the left output and `data[1]`
/// is the right output. Values are signed 32-bit integers.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct YmfmOutput2 {
    /// Per-channel sample data: `[FM_L, FM_R]`.
    pub data: [i32; 2],
}

/// Three-channel output sample from the YM2608.
///
/// Contains one sample per output group: `data[0]` is the left FM output,
/// `data[1]` is the right FM output, `data[2]` is the SSG output.
/// Values are signed 32-bit integers.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct YmfmOutput3 {
    /// Per-channel sample data: `[FM_L, FM_R, SSG]`.
    pub data: [i32; 3],
}

/// Timer change requested by a YMFM chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YmfmTimerUpdate {
    /// Cancel the timer.
    Cancel,
    /// Schedule the timer after the given number of chip input clocks.
    Schedule(u32),
}
