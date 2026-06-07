mod common;

use common::harness::*;
use ymfm_oxide::{Ym2203, YmfmOpnFidelity};

const YM2203_CLOCK: u32 = 3_993_600;

#[test]
fn sample_rate_max() {
    let mut chip = Ym2203::new();
    chip.reset();
    chip.set_fidelity(YmfmOpnFidelity::Max);
    assert_eq!(chip.sample_rate(YM2203_CLOCK), 998_400);
}

#[test]
fn sample_rate_med() {
    let mut chip = Ym2203::new();
    chip.reset();
    chip.set_fidelity(YmfmOpnFidelity::Med);
    assert_eq!(chip.sample_rate(YM2203_CLOCK), 332_800);
}

#[test]
fn sample_rate_min() {
    let mut chip = Ym2203::new();
    chip.reset();
    chip.set_fidelity(YmfmOpnFidelity::Min);
    assert_eq!(chip.sample_rate(YM2203_CLOCK), 166_400);
}

#[test]
fn sample_rate_default_is_max() {
    let mut chip = Ym2203::new();
    chip.reset();
    // Default fidelity should be Max
    assert_eq!(chip.sample_rate(YM2203_CLOCK), 998_400);
}

#[test]
fn fidelity_output_differs() {
    let fidelities = [
        YmfmOpnFidelity::Max,
        YmfmOpnFidelity::Med,
        YmfmOpnFidelity::Min,
    ];

    let mut outputs = Vec::new();
    for &fidelity in &fidelities {
        let mut chip = setup_ym2203(fidelity);
        setup_ym2203_simple_tone(&mut chip, 0, 7, 0);
        key_on_2203(&mut chip, 0);
        let samples = generate_4(&mut chip, 64);
        outputs.push(samples);
    }

    // Each fidelity level should produce different output
    assert_ne!(outputs[0], outputs[1], "Max and Med should differ");
    assert_ne!(outputs[0], outputs[2], "Max and Min should differ");
    assert_ne!(outputs[1], outputs[2], "Med and Min should differ");
}

#[test]
fn sample_rate_various_clocks() {
    let mut chip = Ym2203::new();
    chip.reset();
    chip.set_fidelity(YmfmOpnFidelity::Max);

    // At Max fidelity, rate = clock / 4
    assert_eq!(chip.sample_rate(4_000_000), 1_000_000);
    assert_eq!(chip.sample_rate(8_000_000), 2_000_000);
    assert_eq!(chip.sample_rate(1_000_000), 250_000);
}
