// Initial vectors were established from the BSD-licensed MAME K051649/K052539 core.
// Regenerate: cargo test -p device --test generate_scc_golden -- --ignored

/// Raw SCC clocks after reset.
pub const RESET_SILENCE: &[i16] = &[
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// Raw SCC clocks for a signed waveform.
pub const SIGNED_WAVEFORM: &[i16] = &[
    -120, -120, -120, -120, -120, -120, -120, -120, -120, -120, -113, -113, -113, -113, -113, -113,
    -113, -113, -113, -113, -105, -105, -105, -105, -105, -105, -105, -105, -105, -105, -98, -98,
    -98, -98, -98, -98, -98, -98, -98, -98, -90, -90, -90, -90, -90, -90, -90, -90, -90, -90, -83,
    -83, -83, -83, -83, -83, -83, -83, -83, -83, -75, -75, -75, -75,
];

/// Raw SCC clocks for all five channels.
pub const FIVE_CHANNEL_MIX: &[i16] = &[
    132, 132, 132, 132, 132, 132, 132, 132, 132, 132, 132, 132, 132, 132, 132, 132, 132, 132, 132,
    132, 133, 133, 134, 134, 135, 135, 136, 136, 137, 137, 138, 138, 138, 139, 139, 139, 140, 140,
    140, 141, 142, 142, 143, 143, 144, 144, 144, 144, 145, 145, 146, 146, 147, 147, 147, 148, 148,
    148, 148, 148, 150, 150, 150, 150,
];

/// Raw SCC clocks for the halted period boundary.
pub const HALTED_PERIOD: &[i16] = &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// Raw SCC+ clocks for independent channel four and five waveforms.
pub const PLUS_INDEPENDENT_WAVEFORMS: &[i16] = &[
    75, 75, 75, 75, 75, 75, 75, 75, 75, 75, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0,
];
