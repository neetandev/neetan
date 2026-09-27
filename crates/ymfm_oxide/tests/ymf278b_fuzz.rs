mod common;

#[allow(dead_code)]
mod golden {
    include!("golden/ymf278b_fuzz.rs");
}

use common::harness::*;

fn check(seed: u32, expected: &[u64]) {
    assert_eq!(
        ymf278b_fuzz(seed),
        expected,
        "seed {seed} block checksums differ"
    );
}

#[test]
fn seed_1() {
    check(1, golden::SEED_1);
}

#[test]
fn seed_2() {
    check(2, golden::SEED_2);
}

#[test]
fn seed_3() {
    check(3, golden::SEED_3);
}

#[test]
fn seed_4() {
    check(4, golden::SEED_4);
}

#[test]
fn seed_5() {
    check(5, golden::SEED_5);
}

#[test]
fn seed_6() {
    check(6, golden::SEED_6);
}

#[test]
fn seed_7() {
    check(7, golden::SEED_7);
}

#[test]
fn seed_8() {
    check(8, golden::SEED_8);
}
