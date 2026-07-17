use device::scc::{SccPlus, StandardScc};

#[path = "golden/scc.rs"]
mod golden;

fn collect<const VARIANT: u8>(scc: &mut device::scc::Scc<VARIANT>, count: usize) -> Vec<i16> {
    (0..count).map(|_| scc.clock()).collect()
}

fn write_standard_frequency(scc: &mut StandardScc, channel: usize, frequency: u16) {
    scc.write(0x80 + (channel * 2) as u8, frequency as u8);
    scc.write(0x81 + (channel * 2) as u8, (frequency >> 8) as u8);
}

#[test]
fn reset_silence_matches_reference() {
    let mut scc = StandardScc::new();
    assert_eq!(
        collect(&mut scc, golden::RESET_SILENCE.len()),
        golden::RESET_SILENCE
    );
}

#[test]
fn signed_waveform_matches_reference() {
    let mut scc = StandardScc::new();
    for index in 0..32 {
        scc.write(index, (index as i16 * 8 - 128) as u8);
    }
    write_standard_frequency(&mut scc, 0, 9);
    scc.write(0x8A, 15);
    scc.write(0x8F, 1);
    assert_eq!(
        collect(&mut scc, golden::SIGNED_WAVEFORM.len()),
        golden::SIGNED_WAVEFORM
    );
}

#[test]
fn five_channel_mix_matches_reference() {
    let mut scc = StandardScc::new();
    for channel in 0..5 {
        for index in 0..32 {
            let address = if channel == 4 {
                0x60 + index
            } else {
                channel * 0x20 + index
            };
            scc.write(address as u8, (16 * channel + index) as u8);
        }
        write_standard_frequency(&mut scc, channel, 9 + channel as u16);
        scc.write(0x8A + channel as u8, 15 - channel as u8);
    }
    scc.write(0x8F, 0x1F);
    assert_eq!(
        collect(&mut scc, golden::FIVE_CHANNEL_MIX.len()),
        golden::FIVE_CHANNEL_MIX
    );
}

#[test]
fn halted_period_matches_reference() {
    let mut scc = StandardScc::new();
    scc.write(0, 127);
    write_standard_frequency(&mut scc, 0, 8);
    scc.write(0x8F, 1);
    assert_eq!(
        collect(&mut scc, golden::HALTED_PERIOD.len()),
        golden::HALTED_PERIOD
    );
}

#[test]
fn plus_independent_waveforms_match_reference() {
    let mut scc = SccPlus::new();
    scc.write(0x60, 16);
    scc.write(0x80, 64);
    for channel in [3, 4] {
        scc.write(0xA0 + (channel * 2) as u8, 9);
        scc.write(0xA1 + (channel * 2) as u8, 0);
        scc.write(0xAA + channel as u8, 15);
    }
    scc.write(0xAF, 0x18);
    assert_eq!(
        collect(&mut scc, golden::PLUS_INDEPENDENT_WAVEFORMS.len()),
        golden::PLUS_INDEPENDENT_WAVEFORMS
    );
}
