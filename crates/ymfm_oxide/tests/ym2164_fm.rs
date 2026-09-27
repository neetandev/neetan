mod common;

#[allow(dead_code)]
mod golden {
    include!("golden/ym2164_fm.rs");
}

use common::harness::*;
use ymfm_oxide::YmfmTimerUpdate;

const CLOCK: u32 = 3_579_545;

fn check(name: &str, expected: &[[i32; 2]]) {
    let mut chip = ym2164_scenario(name);
    let samples = generate_2_ym2164(&mut chip, expected.len());
    assert_samples_2(&samples, expected);
}

#[test]
fn sample_rate() {
    let chip = setup_ym2164();
    assert_eq!(chip.sample_rate(CLOCK), CLOCK / 64);
}

#[test]
fn silence() {
    check("SILENCE", golden::SILENCE);
}

#[test]
fn tone_algo7() {
    check("TONE_ALGO7", golden::TONE_ALGO7);
}

#[test]
fn all_algorithms() {
    check("ALL_ALGORITHMS", golden::ALL_ALGORITHMS);
}

#[test]
fn low_registers() {
    check("LOW_REGISTERS", golden::LOW_REGISTERS);
}

#[test]
fn lfo_am_pm() {
    check("LFO_AM_PM", golden::LFO_AM_PM);
}

#[test]
fn noise() {
    check("NOISE", golden::NOISE);
}

#[test]
fn detune() {
    check("DETUNE", golden::DETUNE);
}

#[test]
fn timer_b_expiry_raises_irq() {
    let mut chip = setup_ym2164();
    write_reg_ym2164(&mut chip, 0x12, 0xFE);
    write_reg_ym2164(&mut chip, 0x14, 0x0A);
    assert_eq!(
        chip.take_timer_update(1),
        Some(YmfmTimerUpdate::Schedule(16 * 2 * 32 * 2))
    );

    chip.timer_expired(1);
    assert_eq!(chip.take_irq_update(), Some(true));
    assert_eq!(chip.read_status(false), 0x02);
}
