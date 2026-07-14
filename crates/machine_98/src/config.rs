/// Immutable clock configuration for a PC-98 machine variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockConfig {
    /// CPU clock frequency in Hz.
    pub cpu_clock_hz: u32,
    /// PIT clock frequency in Hz.
    pub pit_clock_hz: u32,
    /// Audio output sample rate in Hz.
    pub sample_rate: u32,
}
