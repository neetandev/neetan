//! Clock-domain conversion helpers.

/// Converts an absolute CPU cycle to an absolute device tick.
pub(crate) fn cycle_to_tick(cycle: u64, device_hz: u64, cpu_hz: u64) -> u64 {
    (u128::from(cycle) * u128::from(device_hz) / u128::from(cpu_hz)) as u64
}

/// Converts an absolute device deadline to the first CPU cycle at or after it.
pub(crate) fn tick_to_cycle(tick: u64, device_hz: u64, cpu_hz: u64) -> u64 {
    let numerator = u128::from(tick) * u128::from(cpu_hz);
    numerator.div_ceil(u128::from(device_hz)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadlines_round_up_without_drift() {
        assert_eq!(cycle_to_tick(10, 4, 10), 4);
        assert_eq!(tick_to_cycle(5, 4, 10), 13);
        assert!(cycle_to_tick(tick_to_cycle(123, 32_768, 10_000_000), 32_768, 10_000_000) >= 123);
    }
}
