mod common;

use std::fmt::Write;

use common::{harness::*, signals::*};
use ymfm_oxide::{Y8950, Ym2203, Ym2608, Ym3526, Ymf262, YmfmOpnFidelity};

const SAMPLES: usize = 256;

fn fmt4(name: &str, data: &[[i32; 4]]) -> String {
    let mut s = format!("pub const {name}: &[[i32; 4]] = &[\n");
    for d in data {
        writeln!(s, "    [{}, {}, {}, {}],", d[0], d[1], d[2], d[3]).unwrap();
    }
    s.push_str("];\n\n");
    s
}

fn fmt3(name: &str, data: &[[i32; 3]]) -> String {
    let mut s = format!("pub const {name}: &[[i32; 3]] = &[\n");
    for d in data {
        writeln!(s, "    [{}, {}, {}],", d[0], d[1], d[2]).unwrap();
    }
    s.push_str("];\n\n");
    s
}

fn header() -> String {
    "// Auto-generated golden vectors from C++ ymfm reference implementation.\n\
     // Regenerate: cargo test -p ymfm --test generate_golden -- --ignored --nocapture\n\n"
        .to_string()
}

fn gen_ym2203_fm(dir: &str) {
    let mut f = header();

    // silence_after_reset
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        f.push_str(&fmt4("SILENCE", &generate_4(&mut chip, SAMPLES)));
    }

    // single_tone_algo7
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        setup_ym2203_simple_tone(&mut chip, 0, 7, 0);
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4("SINGLE_TONE_ALGO7", &generate_4(&mut chip, SAMPLES)));
    }

    // all 8 algorithms (moderate TL on modulators)
    for algo in 0..8u8 {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        write_reg(&mut chip, 0xB0, algo);
        for (op_offset, tl) in [(0x00, 0x20), (0x04, 0x20), (0x08, 0x20), (0x0C, 0x00)] {
            write_reg(&mut chip, 0x30 + op_offset, 0x01);
            write_reg(&mut chip, 0x40 + op_offset, tl);
            write_reg(&mut chip, 0x50 + op_offset, 0x1F);
            write_reg(&mut chip, 0x60 + op_offset, 0x00);
            write_reg(&mut chip, 0x70 + op_offset, 0x00);
            write_reg(&mut chip, 0x80 + op_offset, 0x0F);
            write_reg(&mut chip, 0x90 + op_offset, 0x00);
        }
        write_reg(&mut chip, 0xA4, 0x22);
        write_reg(&mut chip, 0xA0, 0x69);
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4(
            &format!("ALGO_{algo}"),
            &generate_4(&mut chip, SAMPLES),
        ));
    }

    // feedback sweep
    for fb in 0..8u8 {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        setup_ym2203_simple_tone(&mut chip, 0, 0, fb);
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4(
            &format!("FEEDBACK_{fb}"),
            &generate_4(&mut chip, SAMPLES),
        ));
    }

    // detune sweep
    for dt in 0..8u8 {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        write_reg(&mut chip, 0xB0, 0x07);
        for op_offset in [0x00, 0x04, 0x08, 0x0C] {
            write_reg(&mut chip, 0x30 + op_offset, (dt << 4) | 0x01);
            write_reg(&mut chip, 0x40 + op_offset, 0x10);
            write_reg(&mut chip, 0x50 + op_offset, 0x1F);
            write_reg(&mut chip, 0x60 + op_offset, 0x00);
            write_reg(&mut chip, 0x70 + op_offset, 0x00);
            write_reg(&mut chip, 0x80 + op_offset, 0x0F);
            write_reg(&mut chip, 0x90 + op_offset, 0x00);
        }
        write_reg(&mut chip, 0xA4, 0x22);
        write_reg(&mut chip, 0xA0, 0x69);
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4(
            &format!("DETUNE_{dt}"),
            &generate_4(&mut chip, SAMPLES),
        ));
    }

    // multiple=0 (x0.5) and multiple=1 (x1)
    for mul in [0u8, 1] {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        setup_ym2203_simple_tone(&mut chip, 0, 7, 0);
        for op_offset in [0x00, 0x04, 0x08, 0x0C] {
            write_reg(&mut chip, 0x30 + op_offset, mul);
        }
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4(
            &format!("MULTIPLE_{mul}"),
            &generate_4(&mut chip, SAMPLES),
        ));
    }

    // ADSR: sustain phase
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        write_reg(&mut chip, 0xB0, 0x07);
        for op_offset in [0x00, 0x04, 0x08, 0x0C] {
            write_reg(&mut chip, 0x30 + op_offset, 0x01);
            write_reg(&mut chip, 0x40 + op_offset, 0x00);
            write_reg(&mut chip, 0x50 + op_offset, 0x1F);
            write_reg(&mut chip, 0x60 + op_offset, 0x00);
            write_reg(&mut chip, 0x70 + op_offset, 0x00);
            write_reg(&mut chip, 0x80 + op_offset, 0x0F);
            write_reg(&mut chip, 0x90 + op_offset, 0x00);
        }
        write_reg(&mut chip, 0xA4, 0x22);
        write_reg(&mut chip, 0xA0, 0x69);
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4("ADSR_SUSTAIN", &generate_4(&mut chip, SAMPLES)));
        key_off_2203(&mut chip, 0);
        f.push_str(&fmt4("ADSR_RELEASE", &generate_4(&mut chip, 512)));
    }

    // max attack rate (first 16 samples)
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        setup_ym2203_simple_tone(&mut chip, 0, 7, 0);
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4("MAX_ATTACK_RATE", &generate_4(&mut chip, 16)));
    }

    // zero attack rate
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        write_reg(&mut chip, 0xB0, 0x07);
        for op_offset in [0x00, 0x04, 0x08, 0x0C] {
            write_reg(&mut chip, 0x30 + op_offset, 0x01);
            write_reg(&mut chip, 0x40 + op_offset, 0x00);
            write_reg(&mut chip, 0x50 + op_offset, 0x00); // AR=0
            write_reg(&mut chip, 0x60 + op_offset, 0x00);
            write_reg(&mut chip, 0x70 + op_offset, 0x00);
            write_reg(&mut chip, 0x80 + op_offset, 0x0F);
            write_reg(&mut chip, 0x90 + op_offset, 0x00);
        }
        write_reg(&mut chip, 0xA4, 0x22);
        write_reg(&mut chip, 0xA0, 0x69);
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4("ZERO_ATTACK_RATE", &generate_4(&mut chip, SAMPLES)));
    }

    // key on: all ops vs op1 only (algo 7)
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        setup_ym2203_simple_tone(&mut chip, 0, 7, 0);
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4("KEY_ON_ALL_OPS", &generate_4(&mut chip, SAMPLES)));
    }
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        setup_ym2203_simple_tone(&mut chip, 0, 7, 0);
        write_reg(&mut chip, 0x28, 0x10); // Only op1
        f.push_str(&fmt4("KEY_ON_OP1_ONLY", &generate_4(&mut chip, SAMPLES)));
    }

    // multi channel: 1ch and 2ch
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        setup_ym2203_simple_tone(&mut chip, 0, 7, 0);
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4("ONE_CHANNEL", &generate_4(&mut chip, SAMPLES)));
    }
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        setup_ym2203_simple_tone(&mut chip, 0, 7, 0);
        setup_ym2203_simple_tone(&mut chip, 1, 7, 0);
        write_reg(&mut chip, 0xA5, 0x26);
        write_reg(&mut chip, 0xA1, 0xD5);
        key_on_2203(&mut chip, 0);
        key_on_2203(&mut chip, 1);
        f.push_str(&fmt4("TWO_CHANNELS", &generate_4(&mut chip, SAMPLES)));
    }

    // channel 2 multi-freq mode
    {
        let mut chip = Ym2203::new();
        chip.reset();
        chip.set_fidelity(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        write_reg(&mut chip, 0x27, 0x40);
        setup_ym2203_simple_tone(&mut chip, 2, 7, 0);
        write_reg(&mut chip, 0xAD, 0x22);
        write_reg(&mut chip, 0xA9, 0x69);
        write_reg(&mut chip, 0xAE, 0x26);
        write_reg(&mut chip, 0xAA, 0xD5);
        write_reg(&mut chip, 0xAC, 0x2A);
        write_reg(&mut chip, 0xA8, 0x40);
        key_on_2203(&mut chip, 2);
        f.push_str(&fmt4("CH2_MULTI_FREQ", &generate_4(&mut chip, SAMPLES)));
    }

    // CSM mode
    {
        let mut chip = Ym2203::new();
        chip.reset();
        chip.set_fidelity(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        setup_ym2203_simple_tone(&mut chip, 2, 7, 0);
        write_reg(&mut chip, 0x24, 0xFF);
        write_reg(&mut chip, 0x25, 0x03);
        write_reg(&mut chip, 0x27, 0x85);
        f.push_str(&fmt4("CSM_BEFORE_TRIGGER", &generate_4(&mut chip, 64)));
        chip.timer_expired(0);
        f.push_str(&fmt4("CSM_AFTER_TRIGGER", &generate_4(&mut chip, SAMPLES)));
    }

    // SSG-EG modes 0-7
    for mode in 0..8u8 {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        write_reg(&mut chip, 0xB0, 0x07);
        for op_offset in [0x00, 0x04, 0x08, 0x0C] {
            write_reg(&mut chip, 0x30 + op_offset, 0x01);
            write_reg(&mut chip, 0x40 + op_offset, 0x10);
            write_reg(&mut chip, 0x50 + op_offset, 0x1F);
            write_reg(&mut chip, 0x60 + op_offset, 0x1A);
            write_reg(&mut chip, 0x70 + op_offset, 0x00);
            write_reg(&mut chip, 0x80 + op_offset, 0x0F);
            write_reg(&mut chip, 0x90 + op_offset, 0x08 | mode);
        }
        write_reg(&mut chip, 0xA4, 0x22);
        write_reg(&mut chip, 0xA0, 0x69);
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4(
            &format!("SSG_EG_MODE_{mode}"),
            &generate_4(&mut chip, 1024),
        ));
    }

    // key re-on during release
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        write_reg(&mut chip, 0xB0, 0x07);
        for op_offset in [0x00, 0x04, 0x08, 0x0C] {
            write_reg(&mut chip, 0x30 + op_offset, 0x01);
            write_reg(&mut chip, 0x40 + op_offset, 0x00);
            write_reg(&mut chip, 0x50 + op_offset, 0x1F);
            write_reg(&mut chip, 0x60 + op_offset, 0x00);
            write_reg(&mut chip, 0x70 + op_offset, 0x00);
            write_reg(&mut chip, 0x80 + op_offset, 0x84);
            write_reg(&mut chip, 0x90 + op_offset, 0x00);
        }
        write_reg(&mut chip, 0xA4, 0x22);
        write_reg(&mut chip, 0xA0, 0x69);
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4("REON_SUSTAIN", &generate_4(&mut chip, 128)));
        key_off_2203(&mut chip, 0);
        f.push_str(&fmt4("REON_RELEASE", &generate_4(&mut chip, 64)));
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4("REON_AFTER", &generate_4(&mut chip, SAMPLES)));
    }

    // frequency zero
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        setup_ym2203_simple_tone(&mut chip, 0, 7, 0);
        write_reg(&mut chip, 0xA4, 0x00);
        write_reg(&mut chip, 0xA0, 0x00);
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4("FREQ_ZERO", &generate_4(&mut chip, SAMPLES)));
    }

    // freq write order: normal (high then low) and reverse
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        setup_ym2203_simple_tone(&mut chip, 0, 7, 0);
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4("FREQ_ORDER_NORMAL", &generate_4(&mut chip, SAMPLES)));
    }
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_ssg_bg_2203(&mut chip);
        write_reg(&mut chip, 0xB0, 0x07);
        for op_offset in [0x00, 0x04, 0x08, 0x0C] {
            write_reg(&mut chip, 0x30 + op_offset, 0x01);
            write_reg(&mut chip, 0x40 + op_offset, 0x00);
            write_reg(&mut chip, 0x50 + op_offset, 0x1F);
            write_reg(&mut chip, 0x60 + op_offset, 0x00);
            write_reg(&mut chip, 0x70 + op_offset, 0x00);
            write_reg(&mut chip, 0x80 + op_offset, 0x0F);
            write_reg(&mut chip, 0x90 + op_offset, 0x00);
        }
        write_reg(&mut chip, 0xA0, 0x69);
        write_reg(&mut chip, 0xA4, 0x22);
        write_reg(&mut chip, 0xA0, 0x69);
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4("FREQ_ORDER_REVERSE", &generate_4(&mut chip, SAMPLES)));
    }

    std::fs::write(format!("{dir}/ym2203_fm.rs"), f).unwrap();
    println!("  wrote ym2203_fm.rs");
}

fn gen_ym2203_ssg(dir: &str) {
    let mut f = header();

    // silence
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        f.push_str(&fmt4("SILENCE", &generate_4(&mut chip, SAMPLES)));
    }

    // tone channel A
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_fm_bg_2203(&mut chip);
        write_reg(&mut chip, 0x00, 0x10);
        write_reg(&mut chip, 0x01, 0x00);
        write_reg(&mut chip, 0x07, 0x3E);
        write_reg(&mut chip, 0x08, 0x0F);
        f.push_str(&fmt4("TONE_A", &generate_4(&mut chip, 512)));
    }

    // all three channels
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_fm_bg_2203(&mut chip);
        write_reg(&mut chip, 0x00, 0x10);
        write_reg(&mut chip, 0x01, 0x00);
        write_reg(&mut chip, 0x02, 0x20);
        write_reg(&mut chip, 0x03, 0x00);
        write_reg(&mut chip, 0x04, 0x40);
        write_reg(&mut chip, 0x05, 0x00);
        write_reg(&mut chip, 0x07, 0x38);
        write_reg(&mut chip, 0x08, 0x0F);
        write_reg(&mut chip, 0x09, 0x0F);
        write_reg(&mut chip, 0x0A, 0x0F);
        f.push_str(&fmt4("THREE_CHANNELS", &generate_4(&mut chip, 512)));
    }

    // noise
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_fm_bg_2203(&mut chip);
        write_reg(&mut chip, 0x06, 0x01);
        write_reg(&mut chip, 0x07, 0x37);
        write_reg(&mut chip, 0x08, 0x0F);
        f.push_str(&fmt4("NOISE", &generate_4(&mut chip, 1024)));
    }

    // tone+noise mixer: tone only, noise only, both
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_fm_bg_2203(&mut chip);
        write_reg(&mut chip, 0x00, 0x10);
        write_reg(&mut chip, 0x01, 0x00);
        write_reg(&mut chip, 0x06, 0x0A);
        write_reg(&mut chip, 0x07, 0x3E);
        write_reg(&mut chip, 0x08, 0x0F);
        f.push_str(&fmt4("MIXER_TONE_ONLY", &generate_4(&mut chip, 512)));
    }
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_fm_bg_2203(&mut chip);
        write_reg(&mut chip, 0x00, 0x10);
        write_reg(&mut chip, 0x01, 0x00);
        write_reg(&mut chip, 0x06, 0x0A);
        write_reg(&mut chip, 0x07, 0x37);
        write_reg(&mut chip, 0x08, 0x0F);
        f.push_str(&fmt4("MIXER_NOISE_ONLY", &generate_4(&mut chip, 512)));
    }
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_fm_bg_2203(&mut chip);
        write_reg(&mut chip, 0x00, 0x10);
        write_reg(&mut chip, 0x01, 0x00);
        write_reg(&mut chip, 0x06, 0x0A);
        write_reg(&mut chip, 0x07, 0x36);
        write_reg(&mut chip, 0x08, 0x0F);
        f.push_str(&fmt4("MIXER_TONE_AND_NOISE", &generate_4(&mut chip, 512)));
    }

    // envelope mode
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_fm_bg_2203(&mut chip);
        write_reg(&mut chip, 0x00, 0x10);
        write_reg(&mut chip, 0x01, 0x00);
        write_reg(&mut chip, 0x07, 0x3E);
        write_reg(&mut chip, 0x08, 0x10);
        write_reg(&mut chip, 0x0B, 0x20);
        write_reg(&mut chip, 0x0C, 0x00);
        write_reg(&mut chip, 0x0D, 0x08);
        f.push_str(&fmt4("ENVELOPE", &generate_4(&mut chip, 1024)));
    }

    // all 16 envelope shapes
    for shape in 0..16u8 {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_fm_bg_2203(&mut chip);
        write_reg(&mut chip, 0x00, 0x10);
        write_reg(&mut chip, 0x01, 0x00);
        write_reg(&mut chip, 0x07, 0x3E);
        write_reg(&mut chip, 0x08, 0x10);
        write_reg(&mut chip, 0x0B, 0x10);
        write_reg(&mut chip, 0x0C, 0x00);
        write_reg(&mut chip, 0x0D, shape);
        f.push_str(&fmt4(
            &format!("ENVELOPE_SHAPE_{shape}"),
            &generate_4(&mut chip, 512),
        ));
    }

    // fidelity outputs
    for (name, fidelity) in [
        ("FIDELITY_MAX", YmfmOpnFidelity::Max),
        ("FIDELITY_MED", YmfmOpnFidelity::Med),
        ("FIDELITY_MIN", YmfmOpnFidelity::Min),
    ] {
        let mut chip = setup_ym2203(fidelity);
        add_fm_bg_2203(&mut chip);
        write_reg(&mut chip, 0x00, 0x10);
        write_reg(&mut chip, 0x01, 0x00);
        write_reg(&mut chip, 0x07, 0x3E);
        write_reg(&mut chip, 0x08, 0x0F);
        f.push_str(&fmt4(name, &generate_4(&mut chip, 128)));
    }

    // amplitude levels
    for amp in [0x00u8, 0x05, 0x0A, 0x0F] {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        add_fm_bg_2203(&mut chip);
        write_reg(&mut chip, 0x00, 0x10);
        write_reg(&mut chip, 0x01, 0x00);
        write_reg(&mut chip, 0x07, 0x3E);
        write_reg(&mut chip, 0x08, amp);
        f.push_str(&fmt4(
            &format!("AMPLITUDE_{amp:02X}"),
            &generate_4(&mut chip, SAMPLES),
        ));
    }

    // register readback values
    {
        let mut chip = setup_ym2203(YmfmOpnFidelity::Max);
        let test_values: [(u8, u8, u8); 6] = [
            (0x00, 0xAB, 0xFF),
            (0x01, 0x05, 0x0F),
            (0x02, 0xCD, 0xFF),
            (0x03, 0x07, 0x0F),
            (0x06, 0x15, 0x1F),
            (0x07, 0x38, 0xFF),
        ];
        let mut readbacks = String::new();
        readbacks.push_str("pub const REGISTER_READBACK: &[(u8, u8, u8, u8)] = &[\n");
        for &(addr, value, mask) in &test_values {
            write_reg(&mut chip, addr, value);
            chip.write_address(addr);
            let readback = chip.read_data();
            writeln!(
                readbacks,
                "    (0x{addr:02X}, 0x{value:02X}, 0x{mask:02X}, 0x{readback:02X}),"
            )
            .unwrap();
        }
        readbacks.push_str("];\n\n");
        f.push_str(&readbacks);
    }

    std::fs::write(format!("{dir}/ym2203_ssg.rs"), f).unwrap();
    println!("  wrote ym2203_ssg.rs");
}

fn gen_ym2608_fm(dir: &str) {
    let mut f = header();

    // silence
    {
        let mut chip = setup_ym2608(YmfmOpnFidelity::Max);
        f.push_str(&fmt3("SILENCE", &generate_3(&mut chip, SAMPLES)));
    }

    // low bank single tone
    {
        let mut chip = setup_ym2608(YmfmOpnFidelity::Max);
        add_ssg_bg_2608(&mut chip);
        setup_ym2608_simple_tone(&mut chip, 0, 7, 0);
        key_on_2608(&mut chip, 0);
        f.push_str(&fmt3("LOW_BANK_TONE", &generate_3(&mut chip, SAMPLES)));
    }

    // high bank channel 3
    {
        let mut chip = setup_ym2608(YmfmOpnFidelity::Max);
        add_ssg_bg_2608(&mut chip);
        setup_ym2608_simple_tone(&mut chip, 3, 7, 0);
        key_on_2608(&mut chip, 3);
        f.push_str(&fmt3("HIGH_BANK_CH3", &generate_3(&mut chip, SAMPLES)));
    }

    // all 6 channels
    {
        let mut chip = setup_ym2608(YmfmOpnFidelity::Max);
        add_ssg_bg_2608(&mut chip);
        let freqs: [(u8, u8); 6] = [
            (0x22, 0x69),
            (0x24, 0x80),
            (0x26, 0xD5),
            (0x22, 0x40),
            (0x28, 0x50),
            (0x2A, 0xA0),
        ];
        for ch in 0..6u8 {
            setup_ym2608_simple_tone(&mut chip, ch, 7, 0);
            let (hi, lo) = freqs[ch as usize];
            if ch < 3 {
                write_reg_2608(&mut chip, 0xA4 + ch, hi);
                write_reg_2608(&mut chip, 0xA0 + ch, lo);
            } else {
                write_reg_hi(&mut chip, 0xA4 + (ch - 3), hi);
                write_reg_hi(&mut chip, 0xA0 + (ch - 3), lo);
            }
            key_on_2608(&mut chip, ch);
        }
        f.push_str(&fmt3("ALL_6_CHANNELS", &generate_3(&mut chip, SAMPLES)));
    }

    // all 8 algorithms (moderate TL)
    for algo in 0..8u8 {
        let mut chip = setup_ym2608(YmfmOpnFidelity::Max);
        add_ssg_bg_2608(&mut chip);
        write_reg_2608(&mut chip, 0xB0, algo);
        for (op_offset, tl) in [(0x00, 0x20), (0x04, 0x20), (0x08, 0x20), (0x0C, 0x00)] {
            write_reg_2608(&mut chip, 0x30 + op_offset, 0x01);
            write_reg_2608(&mut chip, 0x40 + op_offset, tl);
            write_reg_2608(&mut chip, 0x50 + op_offset, 0x1F);
            write_reg_2608(&mut chip, 0x60 + op_offset, 0x00);
            write_reg_2608(&mut chip, 0x70 + op_offset, 0x00);
            write_reg_2608(&mut chip, 0x80 + op_offset, 0x0F);
            write_reg_2608(&mut chip, 0x90 + op_offset, 0x00);
        }
        write_reg_2608(&mut chip, 0xA4, 0x22);
        write_reg_2608(&mut chip, 0xA0, 0x69);
        write_reg_2608(&mut chip, 0xB4, 0xC0);
        key_on_2608(&mut chip, 0);
        f.push_str(&fmt3(
            &format!("ALGO_{algo}"),
            &generate_3(&mut chip, SAMPLES),
        ));
    }

    // LFO: without and with
    {
        let mut chip = setup_ym2608(YmfmOpnFidelity::Max);
        add_ssg_bg_2608(&mut chip);
        write_reg_2608(&mut chip, 0xB0, 0x00);
        for (op_offset, tl) in [(0x00, 0x20), (0x04, 0x20), (0x08, 0x20), (0x0C, 0x00)] {
            write_reg_2608(&mut chip, 0x30 + op_offset, 0x01);
            write_reg_2608(&mut chip, 0x40 + op_offset, tl);
            write_reg_2608(&mut chip, 0x50 + op_offset, 0x1F);
            write_reg_2608(&mut chip, 0x60 + op_offset, 0x00);
            write_reg_2608(&mut chip, 0x70 + op_offset, 0x00);
            write_reg_2608(&mut chip, 0x80 + op_offset, 0x0F);
            write_reg_2608(&mut chip, 0x90 + op_offset, 0x00);
        }
        write_reg_2608(&mut chip, 0xA4, 0x22);
        write_reg_2608(&mut chip, 0xA0, 0x69);
        write_reg_2608(&mut chip, 0xB4, 0xC0);
        key_on_2608(&mut chip, 0);
        f.push_str(&fmt3("LFO_OFF", &generate_3(&mut chip, 1024)));
    }
    {
        let mut chip = setup_ym2608(YmfmOpnFidelity::Max);
        add_ssg_bg_2608(&mut chip);
        write_reg_2608(&mut chip, 0x22, 0x0F); // LFO on, rate=7
        write_reg_2608(&mut chip, 0xB0, 0x00);
        for (op_offset, tl) in [(0x00, 0x20), (0x04, 0x20), (0x08, 0x20), (0x0C, 0x00)] {
            write_reg_2608(&mut chip, 0x30 + op_offset, 0x01);
            write_reg_2608(&mut chip, 0x40 + op_offset, tl);
            write_reg_2608(&mut chip, 0x50 + op_offset, 0x1F);
            write_reg_2608(&mut chip, 0x60 + op_offset, 0x80); // AM=1
            write_reg_2608(&mut chip, 0x70 + op_offset, 0x00);
            write_reg_2608(&mut chip, 0x80 + op_offset, 0x0F);
            write_reg_2608(&mut chip, 0x90 + op_offset, 0x00);
        }
        write_reg_2608(&mut chip, 0xA4, 0x22);
        write_reg_2608(&mut chip, 0xA0, 0x69);
        write_reg_2608(&mut chip, 0xB4, 0xFF); // AMS=3, PMS=7
        key_on_2608(&mut chip, 0);
        f.push_str(&fmt3("LFO_ON", &generate_3(&mut chip, 1024)));
    }

    // LFO rate sweep
    for rate in 0..8u8 {
        let mut chip = setup_ym2608(YmfmOpnFidelity::Max);
        add_ssg_bg_2608(&mut chip);
        write_reg_2608(&mut chip, 0x22, 0x08 | rate);
        write_reg_2608(&mut chip, 0xB0, 0x00);
        for (op_offset, tl) in [(0x00, 0x20), (0x04, 0x20), (0x08, 0x20), (0x0C, 0x00)] {
            write_reg_2608(&mut chip, 0x30 + op_offset, 0x01);
            write_reg_2608(&mut chip, 0x40 + op_offset, tl);
            write_reg_2608(&mut chip, 0x50 + op_offset, 0x1F);
            write_reg_2608(&mut chip, 0x60 + op_offset, 0x80);
            write_reg_2608(&mut chip, 0x70 + op_offset, 0x00);
            write_reg_2608(&mut chip, 0x80 + op_offset, 0x0F);
            write_reg_2608(&mut chip, 0x90 + op_offset, 0x00);
        }
        write_reg_2608(&mut chip, 0xA4, 0x22);
        write_reg_2608(&mut chip, 0xA0, 0x69);
        write_reg_2608(&mut chip, 0xB4, 0xFF);
        key_on_2608(&mut chip, 0);
        f.push_str(&fmt3(
            &format!("LFO_RATE_{rate}"),
            &generate_3(&mut chip, 1024),
        ));
    }

    // SSG output via YM2608
    {
        let mut chip = setup_ym2608(YmfmOpnFidelity::Max);
        setup_ym2608_simple_tone(&mut chip, 0, 7, 0);
        key_on_2608(&mut chip, 0);
        write_reg_2608(&mut chip, 0x00, 0x10);
        write_reg_2608(&mut chip, 0x01, 0x00);
        write_reg_2608(&mut chip, 0x07, 0x3E);
        write_reg_2608(&mut chip, 0x08, 0x0F);
        f.push_str(&fmt3("SSG_OUTPUT", &generate_3(&mut chip, 512)));
    }

    std::fs::write(format!("{dir}/ym2608_fm.rs"), f).unwrap();
    println!("  wrote ym2608_fm.rs");
}

fn gen_ym2608_stereo(dir: &str) {
    let mut f = header();

    // center pan (default)
    {
        let mut chip = setup_ym2608(YmfmOpnFidelity::Max);
        add_ssg_bg_2608(&mut chip);
        setup_ym2608_simple_tone(&mut chip, 0, 7, 0);
        key_on_2608(&mut chip, 0);
        f.push_str(&fmt3("CENTER_PAN", &generate_3(&mut chip, SAMPLES)));
    }

    // left only
    {
        let mut chip = setup_ym2608(YmfmOpnFidelity::Max);
        add_ssg_bg_2608(&mut chip);
        setup_ym2608_simple_tone(&mut chip, 0, 7, 0);
        write_reg_2608(&mut chip, 0xB4, 0x80);
        key_on_2608(&mut chip, 0);
        f.push_str(&fmt3("LEFT_PAN", &generate_3(&mut chip, SAMPLES)));
    }

    // right only
    {
        let mut chip = setup_ym2608(YmfmOpnFidelity::Max);
        add_ssg_bg_2608(&mut chip);
        setup_ym2608_simple_tone(&mut chip, 0, 7, 0);
        write_reg_2608(&mut chip, 0xB4, 0x40);
        key_on_2608(&mut chip, 0);
        f.push_str(&fmt3("RIGHT_PAN", &generate_3(&mut chip, SAMPLES)));
    }

    // both (explicit)
    {
        let mut chip = setup_ym2608(YmfmOpnFidelity::Max);
        add_ssg_bg_2608(&mut chip);
        setup_ym2608_simple_tone(&mut chip, 0, 7, 0);
        write_reg_2608(&mut chip, 0xB4, 0xC0);
        key_on_2608(&mut chip, 0);
        f.push_str(&fmt3("BOTH_PAN", &generate_3(&mut chip, SAMPLES)));
    }

    // mute
    {
        let mut chip = setup_ym2608(YmfmOpnFidelity::Max);
        add_ssg_bg_2608(&mut chip);
        setup_ym2608_simple_tone(&mut chip, 0, 7, 0);
        write_reg_2608(&mut chip, 0xB4, 0x00);
        key_on_2608(&mut chip, 0);
        f.push_str(&fmt3("MUTE_PAN", &generate_3(&mut chip, SAMPLES)));
    }

    // per-channel independent: ch0 left, ch1 right
    {
        let mut chip = setup_ym2608(YmfmOpnFidelity::Max);
        add_ssg_bg_2608(&mut chip);
        setup_ym2608_simple_tone(&mut chip, 0, 7, 0);
        write_reg_2608(&mut chip, 0xB4, 0x80);
        setup_ym2608_simple_tone(&mut chip, 1, 7, 0);
        write_reg_2608(&mut chip, 0xA5, 0x26);
        write_reg_2608(&mut chip, 0xA1, 0xD5);
        write_reg_2608(&mut chip, 0xB5, 0x40);
        key_on_2608(&mut chip, 0);
        key_on_2608(&mut chip, 1);
        f.push_str(&fmt3("INDEPENDENT_PAN", &generate_3(&mut chip, SAMPLES)));
    }

    // high bank panning
    {
        let mut chip = setup_ym2608(YmfmOpnFidelity::Max);
        add_ssg_bg_2608(&mut chip);
        setup_ym2608_simple_tone(&mut chip, 3, 7, 0);
        write_reg_hi(&mut chip, 0xB4, 0x80);
        key_on_2608(&mut chip, 3);
        f.push_str(&fmt3("HIGH_BANK_LEFT_PAN", &generate_3(&mut chip, SAMPLES)));
    }

    std::fs::write(format!("{dir}/ym2608_stereo.rs"), f).unwrap();
    println!("  wrote ym2608_stereo.rs");
}

fn gen_ym2608_adpcm(dir: &str) {
    let mut f = header();

    // ADPCM-A key on channel 0 (bass drum)
    // ADPCM-A registers are in LOW bank at addresses 0x10-0x1F
    // Register N is accessed via write_reg_2608(chip, 0x10 + N, data)
    // Start/end addresses are hardcoded at reset (bass drum: 0x0000-0x01BF)
    {
        let adpcm_data = create_adpcm_rom();
        let mut chip = setup_ym2608_with_adpcm_data(adpcm_data);
        chip.reset();
        chip.set_fidelity(YmfmOpnFidelity::Max);
        write_reg_2608(&mut chip, 0x29, 0x80);
        add_ssg_bg_2608(&mut chip);
        // ADPCM-A register 8: channel 0 pan(L+R=0xC0) + level(0x1F)
        write_reg_2608(&mut chip, 0x18, 0xDF);
        // ADPCM-A register 1: total level = max volume (0x3F ^ 0x3F = 0 attenuation)
        write_reg_2608(&mut chip, 0x11, 0x3F);
        // ADPCM-A register 0: key on channel 0
        write_reg_2608(&mut chip, 0x10, 0x01);
        f.push_str(&fmt3("ADPCM_A_KEY_ON", &generate_3(&mut chip, 512)));
    }

    // ADPCM-A key on then key off
    {
        let adpcm_data = create_adpcm_rom();
        let mut chip = setup_ym2608_with_adpcm_data(adpcm_data);
        chip.reset();
        chip.set_fidelity(YmfmOpnFidelity::Max);
        write_reg_2608(&mut chip, 0x29, 0x80);
        add_ssg_bg_2608(&mut chip);
        write_reg_2608(&mut chip, 0x18, 0xDF);
        write_reg_2608(&mut chip, 0x11, 0x3F);
        write_reg_2608(&mut chip, 0x10, 0x01);
        f.push_str(&fmt3("ADPCM_A_ON", &generate_3(&mut chip, SAMPLES)));
        // Key off: bit 7 = dump, bits 0-5 = channels to dump
        write_reg_2608(&mut chip, 0x10, 0x80 | 0x01);
        f.push_str(&fmt3("ADPCM_A_OFF", &generate_3(&mut chip, SAMPLES)));
    }

    // ADPCM-A all 6 channels simultaneously
    {
        let adpcm_data = create_adpcm_rom();
        let mut chip = setup_ym2608_with_adpcm_data(adpcm_data);
        chip.reset();
        chip.set_fidelity(YmfmOpnFidelity::Max);
        write_reg_2608(&mut chip, 0x29, 0x80);
        add_ssg_bg_2608(&mut chip);
        // Set pan+level for all 6 channels (register 8+ch via bus 0x18+ch)
        for ch in 0..6u8 {
            write_reg_2608(&mut chip, 0x18 + ch, 0xDF);
        }
        // Total level = max volume
        write_reg_2608(&mut chip, 0x11, 0x3F);
        // Key on all 6 channels
        write_reg_2608(&mut chip, 0x10, 0x3F);
        f.push_str(&fmt3("ADPCM_A_ALL_6CH", &generate_3(&mut chip, 512)));
    }

    // ADPCM-B playback
    // ADPCM-B registers are in HIGH bank at addresses 0x00-0x0F
    {
        let adpcm_data = create_adpcm_rom();
        let mut chip = setup_ym2608_with_adpcm_data(adpcm_data);
        chip.reset();
        chip.set_fidelity(YmfmOpnFidelity::Max);
        write_reg_2608(&mut chip, 0x29, 0x80);
        add_ssg_bg_2608(&mut chip);
        // ADPCM-B register 1: pan L+R
        write_reg_hi(&mut chip, 0x01, 0xC0);
        // Start address = 0x0020 (data at 0x2000 in ROM, shifted by address_shift=0)
        write_reg_hi(&mut chip, 0x02, 0x20);
        write_reg_hi(&mut chip, 0x03, 0x00);
        // End address = 0x0023 (small region: 0x2000-0x23FF)
        write_reg_hi(&mut chip, 0x04, 0x23);
        write_reg_hi(&mut chip, 0x05, 0x00);
        // Delta-N = 0x0C49 (~49kHz at 8MHz clock)
        write_reg_hi(&mut chip, 0x09, 0x49);
        write_reg_hi(&mut chip, 0x0A, 0x0C);
        // Level = max
        write_reg_hi(&mut chip, 0x0B, 0xFF);
        // Control 1: start + external (bit 7=execute, bit 5=external)
        write_reg_hi(&mut chip, 0x00, 0xA0);
        f.push_str(&fmt3("ADPCM_B_PLAYBACK", &generate_3(&mut chip, 512)));
    }

    // ADPCM-B end of sample (short sample)
    {
        let adpcm_data = create_adpcm_rom();
        let mut chip = setup_ym2608_with_adpcm_data(adpcm_data);
        chip.reset();
        chip.set_fidelity(YmfmOpnFidelity::Max);
        write_reg_2608(&mut chip, 0x29, 0x80);
        add_ssg_bg_2608(&mut chip);
        write_reg_hi(&mut chip, 0x01, 0xC0);
        // Very short sample: start=0x0020, end=0x0020 (256 bytes)
        write_reg_hi(&mut chip, 0x02, 0x20);
        write_reg_hi(&mut chip, 0x03, 0x00);
        write_reg_hi(&mut chip, 0x04, 0x20);
        write_reg_hi(&mut chip, 0x05, 0x00);
        // Max playback rate
        write_reg_hi(&mut chip, 0x09, 0xFF);
        write_reg_hi(&mut chip, 0x0A, 0xFF);
        write_reg_hi(&mut chip, 0x0B, 0xFF);
        write_reg_hi(&mut chip, 0x00, 0xA0);
        f.push_str(&fmt3(
            "ADPCM_B_SHORT_SAMPLE",
            &generate_3(&mut chip, SAMPLES),
        ));

        // Capture EOS status
        let status_hi = chip.read_status_hi(false);
        writeln!(
            f,
            "pub const ADPCM_B_EOS_STATUS_HI: u8 = 0x{status_hi:02X};\n"
        )
        .unwrap();
    }

    // External write ADPCM-B memory mode (ADPCM-B memory write mode)
    {
        let mut chip = Ym2608::new();
        chip.reset();
        chip.set_fidelity(YmfmOpnFidelity::Max);
        write_reg_2608(&mut chip, 0x29, 0x80);
        add_ssg_bg_2608(&mut chip);
        chip.take_signals();
        // ADPCM-B register 0: reset first
        write_reg_hi(&mut chip, 0x00, 0x01);
        // ADPCM-B register 1: pan L+R
        write_reg_hi(&mut chip, 0x01, 0xC0);
        // Start address
        write_reg_hi(&mut chip, 0x02, 0x00);
        write_reg_hi(&mut chip, 0x03, 0x00);
        // End address
        write_reg_hi(&mut chip, 0x04, 0xFF);
        write_reg_hi(&mut chip, 0x05, 0xFF);
        // Control 1: external + record (bit 6=record, bit 5=external)
        write_reg_hi(&mut chip, 0x00, 0x60);
        // Write a data byte via register 8
        write_reg_hi(&mut chip, 0x08, 0xAB);
        f.push_str(&fmt3("EXTERNAL_WRITE", &generate_3(&mut chip, 128)));
    }

    std::fs::write(format!("{dir}/ym2608_adpcm.rs"), f).unwrap();
    println!("  wrote ym2608_adpcm.rs");
}

fn fmt1(name: &str, data: &[[i32; 1]]) -> String {
    let mut s = format!("pub const {name}: &[[i32; 1]] = &[\n");
    for d in data {
        writeln!(s, "    [{}],", d[0]).unwrap();
    }
    s.push_str("];\n\n");
    s
}

fn gen_ym3526_fm(dir: &str) {
    let mut f = header();

    // silence
    {
        let mut chip = setup_ym3526();
        f.push_str(&fmt1("SILENCE", &generate_1_opl(&mut chip, SAMPLES)));
    }

    // single_tone_algo1 (additive mode)
    {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 1, 0);
        key_on_opl(&mut chip, 0);
        f.push_str(&fmt1(
            "SINGLE_TONE_ALGO1",
            &generate_1_opl(&mut chip, SAMPLES),
        ));
    }

    // single_tone_algo0 (FM mode)
    {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 0, 0);
        key_on_opl(&mut chip, 0);
        f.push_str(&fmt1(
            "SINGLE_TONE_ALGO0",
            &generate_1_opl(&mut chip, SAMPLES),
        ));
    }

    // feedback sweep
    for fb in 0..8u8 {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 0, fb);
        key_on_opl(&mut chip, 0);
        f.push_str(&fmt1(
            &format!("FEEDBACK_{fb}"),
            &generate_1_opl(&mut chip, SAMPLES),
        ));
    }

    // multiple sweep
    for mul in [0u8, 1, 2, 4] {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 1, 0);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl(&mut chip, 0x20 + off, 0x20 | mul);
        }
        key_on_opl(&mut chip, 0);
        f.push_str(&fmt1(
            &format!("MULTIPLE_{mul}"),
            &generate_1_opl(&mut chip, SAMPLES),
        ));
    }

    // TL sweep (carrier only)
    for tl in [0x00u8, 0x10, 0x20, 0x3F] {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 1, 0);
        let carrier_off = opl_op_offset(0, 1);
        write_reg_opl(&mut chip, 0x40 + carrier_off, tl);
        key_on_opl(&mut chip, 0);
        f.push_str(&fmt1(
            &format!("TL_{tl:02X}"),
            &generate_1_opl(&mut chip, SAMPLES),
        ));
    }

    // ADSR
    {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 1, 0);
        key_on_opl(&mut chip, 0);
        f.push_str(&fmt1("ADSR_SUSTAIN", &generate_1_opl(&mut chip, SAMPLES)));
        key_off_opl(&mut chip, 0);
        f.push_str(&fmt1("ADSR_RELEASE", &generate_1_opl(&mut chip, 512)));
    }

    // max attack rate
    {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 1, 0);
        key_on_opl(&mut chip, 0);
        f.push_str(&fmt1("MAX_ATTACK_RATE", &generate_1_opl(&mut chip, 16)));
    }

    // zero attack rate
    {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 1, 0);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl(&mut chip, 0x60 + off, 0x00); // AR=0, DR=0
        }
        key_on_opl(&mut chip, 0);
        f.push_str(&fmt1(
            "ZERO_ATTACK_RATE",
            &generate_1_opl(&mut chip, SAMPLES),
        ));
    }

    // key re-on
    {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 1, 0);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl(&mut chip, 0x80 + off, 0x84); // SL=8, RR=4
        }
        key_on_opl(&mut chip, 0);
        f.push_str(&fmt1("REON_SUSTAIN", &generate_1_opl(&mut chip, 128)));
        key_off_opl(&mut chip, 0);
        f.push_str(&fmt1("REON_RELEASE", &generate_1_opl(&mut chip, 64)));
        key_on_opl(&mut chip, 0);
        f.push_str(&fmt1("REON_AFTER", &generate_1_opl(&mut chip, SAMPLES)));
    }

    // two channels
    {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 1, 0);
        setup_opl_simple_tone(&mut chip, 1, 1, 0);
        write_reg_opl(&mut chip, 0xA1, 0x81); // different freq for ch1
        key_on_opl(&mut chip, 0);
        key_on_opl(&mut chip, 1);
        f.push_str(&fmt1("TWO_CHANNELS", &generate_1_opl(&mut chip, SAMPLES)));
    }

    // all 9 channels
    {
        let mut chip = setup_ym3526();
        for ch in 0..9u8 {
            setup_opl_simple_tone(&mut chip, ch, 1, 0);
            write_reg_opl(&mut chip, 0xA0 + ch, 0x41 + ch * 8);
            key_on_opl(&mut chip, ch);
        }
        f.push_str(&fmt1("ALL_9_CHANNELS", &generate_1_opl(&mut chip, SAMPLES)));
    }

    // freq zero
    {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 1, 0);
        write_reg_opl(&mut chip, 0xA0, 0x00);
        write_reg_opl(&mut chip, 0xB0, 0x20); // key-on, block=0, fnum_hi=0
        f.push_str(&fmt1("FREQ_ZERO", &generate_1_opl(&mut chip, SAMPLES)));
    }

    // AM/VIB depth
    {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 1, 0);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl(&mut chip, 0x20 + off, 0xE1); // AM=1, VIB=1, EGT=1, KSR=0, MULT=1
        }
        write_reg_opl(&mut chip, 0xBD, 0xC0); // AM depth=1, VIB depth=1
        key_on_opl(&mut chip, 0);
        f.push_str(&fmt1("AM_VIB_DEPTH", &generate_1_opl(&mut chip, 1024)));
    }

    // KSL sweep
    for ksl in 0..4u8 {
        let mut chip = setup_ym3526();
        setup_opl_simple_tone(&mut chip, 0, 1, 0);
        let carrier_off = opl_op_offset(0, 1);
        write_reg_opl(&mut chip, 0x40 + carrier_off, ksl << 6);
        key_on_opl(&mut chip, 0);
        f.push_str(&fmt1(
            &format!("KSL_{ksl}"),
            &generate_1_opl(&mut chip, SAMPLES),
        ));
    }

    // rhythm mode
    {
        let mut chip = setup_ym3526();
        // Setup channels 6,7,8 for rhythm
        for ch in 6..9u8 {
            setup_opl_simple_tone(&mut chip, ch, 1, 0);
        }
        write_reg_opl(&mut chip, 0xBD, 0x3F); // rhythm=1, all rhythm keys on
        f.push_str(&fmt1("RHYTHM_MODE", &generate_1_opl(&mut chip, 512)));
    }

    // individual rhythm instruments
    for (name, bit) in [
        ("RHYTHM_BD", 0x10u8),
        ("RHYTHM_SD", 0x08),
        ("RHYTHM_TOM", 0x04),
        ("RHYTHM_CY", 0x02),
        ("RHYTHM_HH", 0x01),
    ] {
        let mut chip = setup_ym3526();
        for ch in 6..9u8 {
            setup_opl_simple_tone(&mut chip, ch, 1, 0);
        }
        write_reg_opl(&mut chip, 0xBD, 0x20 | bit); // rhythm=1, single instrument
        f.push_str(&fmt1(name, &generate_1_opl(&mut chip, SAMPLES)));
    }

    // CSM mode
    {
        let mut chip = Ym3526::new();
        chip.reset();
        setup_opl_simple_tone(&mut chip, 0, 1, 0);
        write_reg_opl(&mut chip, 0x02, 0xFF); // Timer A value
        write_reg_opl(&mut chip, 0x08, 0x80); // CSM mode enable
        write_reg_opl(&mut chip, 0x04, 0x01); // Start Timer A
        f.push_str(&fmt1("CSM_BEFORE_TRIGGER", &generate_1_opl(&mut chip, 64)));
        chip.timer_expired(0);
        f.push_str(&fmt1(
            "CSM_AFTER_TRIGGER",
            &generate_1_opl(&mut chip, SAMPLES),
        ));
    }

    std::fs::write(format!("{dir}/ym3526_fm.rs"), f).unwrap();
    println!("  wrote ym3526_fm.rs");
}

fn gen_y8950_fm(dir: &str) {
    let mut f = header();

    // silence
    {
        let mut chip = setup_y8950();
        f.push_str(&fmt1("SILENCE", &generate_1_y8950(&mut chip, SAMPLES)));
    }

    // single_tone_algo1 (additive)
    {
        let mut chip = setup_y8950();
        setup_y8950_simple_tone(&mut chip, 0, 1, 0);
        key_on_y8950(&mut chip, 0);
        f.push_str(&fmt1(
            "SINGLE_TONE_ALGO1",
            &generate_1_y8950(&mut chip, SAMPLES),
        ));
    }

    // single_tone_algo0 (FM)
    {
        let mut chip = setup_y8950();
        setup_y8950_simple_tone(&mut chip, 0, 0, 0);
        key_on_y8950(&mut chip, 0);
        f.push_str(&fmt1(
            "SINGLE_TONE_ALGO0",
            &generate_1_y8950(&mut chip, SAMPLES),
        ));
    }

    // feedback sweep
    for fb in 0..8u8 {
        let mut chip = setup_y8950();
        setup_y8950_simple_tone(&mut chip, 0, 0, fb);
        key_on_y8950(&mut chip, 0);
        f.push_str(&fmt1(
            &format!("FEEDBACK_{fb}"),
            &generate_1_y8950(&mut chip, SAMPLES),
        ));
    }

    // ADSR
    {
        let mut chip = setup_y8950();
        setup_y8950_simple_tone(&mut chip, 0, 1, 0);
        key_on_y8950(&mut chip, 0);
        f.push_str(&fmt1("ADSR_SUSTAIN", &generate_1_y8950(&mut chip, SAMPLES)));
        key_off_y8950(&mut chip, 0);
        f.push_str(&fmt1("ADSR_RELEASE", &generate_1_y8950(&mut chip, 512)));
    }

    // rhythm mode
    {
        let mut chip = setup_y8950();
        for ch in 6..9u8 {
            setup_y8950_simple_tone(&mut chip, ch, 1, 0);
        }
        write_reg_y8950(&mut chip, 0xBD, 0x3F);
        f.push_str(&fmt1("RHYTHM_MODE", &generate_1_y8950(&mut chip, 512)));
    }

    // individual rhythm instruments
    for (name, bit) in [
        ("RHYTHM_BD", 0x10u8),
        ("RHYTHM_SD", 0x08),
        ("RHYTHM_TOM", 0x04),
        ("RHYTHM_CY", 0x02),
        ("RHYTHM_HH", 0x01),
    ] {
        let mut chip = setup_y8950();
        for ch in 6..9u8 {
            setup_y8950_simple_tone(&mut chip, ch, 1, 0);
        }
        write_reg_y8950(&mut chip, 0xBD, 0x20 | bit);
        f.push_str(&fmt1(name, &generate_1_y8950(&mut chip, SAMPLES)));
    }

    // two channels
    {
        let mut chip = setup_y8950();
        setup_y8950_simple_tone(&mut chip, 0, 1, 0);
        setup_y8950_simple_tone(&mut chip, 1, 1, 0);
        write_reg_y8950(&mut chip, 0xA1, 0x81);
        key_on_y8950(&mut chip, 0);
        key_on_y8950(&mut chip, 1);
        f.push_str(&fmt1("TWO_CHANNELS", &generate_1_y8950(&mut chip, SAMPLES)));
    }

    // key re-on
    {
        let mut chip = setup_y8950();
        setup_y8950_simple_tone(&mut chip, 0, 1, 0);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_y8950(&mut chip, 0x80 + off, 0x84);
        }
        key_on_y8950(&mut chip, 0);
        f.push_str(&fmt1("REON_SUSTAIN", &generate_1_y8950(&mut chip, 128)));
        key_off_y8950(&mut chip, 0);
        f.push_str(&fmt1("REON_RELEASE", &generate_1_y8950(&mut chip, 64)));
        key_on_y8950(&mut chip, 0);
        f.push_str(&fmt1("REON_AFTER", &generate_1_y8950(&mut chip, SAMPLES)));
    }

    std::fs::write(format!("{dir}/y8950_fm.rs"), f).unwrap();
    println!("  wrote y8950_fm.rs");
}

fn gen_y8950_adpcm(dir: &str) {
    let mut f = header();

    // ADPCM-B playback
    {
        let adpcm_data = create_y8950_adpcm_data();
        let mut chip = setup_y8950_with_adpcm_data(adpcm_data);
        chip.reset();
        // Start/end address
        write_reg_y8950(&mut chip, 0x09, 0x20); // start low
        write_reg_y8950(&mut chip, 0x0A, 0x00); // start high
        write_reg_y8950(&mut chip, 0x0B, 0x23); // end low
        write_reg_y8950(&mut chip, 0x0C, 0x00); // end high
        // Delta-N
        write_reg_y8950(&mut chip, 0x10, 0x49); // delta-N low
        write_reg_y8950(&mut chip, 0x11, 0x0C); // delta-N high
        // Level
        write_reg_y8950(&mut chip, 0x12, 0xFF);
        // Pan (L+R)
        write_reg_y8950(&mut chip, 0x08, 0xC0);
        // Control: start playback
        write_reg_y8950(&mut chip, 0x07, 0xA0); // execute + external
        f.push_str(&fmt1("ADPCM_B_PLAYBACK", &generate_1_y8950(&mut chip, 512)));
    }

    // ADPCM-B on then off
    {
        let adpcm_data = create_y8950_adpcm_data();
        let mut chip = setup_y8950_with_adpcm_data(adpcm_data);
        chip.reset();
        write_reg_y8950(&mut chip, 0x09, 0x20);
        write_reg_y8950(&mut chip, 0x0A, 0x00);
        write_reg_y8950(&mut chip, 0x0B, 0x23);
        write_reg_y8950(&mut chip, 0x0C, 0x00);
        write_reg_y8950(&mut chip, 0x10, 0x49);
        write_reg_y8950(&mut chip, 0x11, 0x0C);
        write_reg_y8950(&mut chip, 0x12, 0xFF);
        write_reg_y8950(&mut chip, 0x08, 0xC0);
        write_reg_y8950(&mut chip, 0x07, 0xA0);
        f.push_str(&fmt1("ADPCM_B_ON", &generate_1_y8950(&mut chip, SAMPLES)));
        // Stop
        write_reg_y8950(&mut chip, 0x07, 0x01); // reset
        f.push_str(&fmt1("ADPCM_B_OFF", &generate_1_y8950(&mut chip, SAMPLES)));
    }

    // ADPCM-B rate low
    {
        let adpcm_data = create_y8950_adpcm_data();
        let mut chip = setup_y8950_with_adpcm_data(adpcm_data);
        chip.reset();
        write_reg_y8950(&mut chip, 0x09, 0x20);
        write_reg_y8950(&mut chip, 0x0A, 0x00);
        write_reg_y8950(&mut chip, 0x0B, 0x23);
        write_reg_y8950(&mut chip, 0x0C, 0x00);
        write_reg_y8950(&mut chip, 0x10, 0x00); // very low rate
        write_reg_y8950(&mut chip, 0x11, 0x01);
        write_reg_y8950(&mut chip, 0x12, 0xFF);
        write_reg_y8950(&mut chip, 0x08, 0xC0);
        write_reg_y8950(&mut chip, 0x07, 0xA0);
        f.push_str(&fmt1(
            "ADPCM_B_RATE_LOW",
            &generate_1_y8950(&mut chip, SAMPLES),
        ));
    }

    // ADPCM-B rate high
    {
        let adpcm_data = create_y8950_adpcm_data();
        let mut chip = setup_y8950_with_adpcm_data(adpcm_data);
        chip.reset();
        write_reg_y8950(&mut chip, 0x09, 0x20);
        write_reg_y8950(&mut chip, 0x0A, 0x00);
        write_reg_y8950(&mut chip, 0x0B, 0x23);
        write_reg_y8950(&mut chip, 0x0C, 0x00);
        write_reg_y8950(&mut chip, 0x10, 0xFF); // max rate
        write_reg_y8950(&mut chip, 0x11, 0xFF);
        write_reg_y8950(&mut chip, 0x12, 0xFF);
        write_reg_y8950(&mut chip, 0x08, 0xC0);
        write_reg_y8950(&mut chip, 0x07, 0xA0);
        f.push_str(&fmt1(
            "ADPCM_B_RATE_HIGH",
            &generate_1_y8950(&mut chip, SAMPLES),
        ));
    }

    // ADPCM-B short sample
    {
        let adpcm_data = create_y8950_adpcm_data();
        let mut chip = setup_y8950_with_adpcm_data(adpcm_data);
        chip.reset();
        write_reg_y8950(&mut chip, 0x09, 0x20); // start=0x20
        write_reg_y8950(&mut chip, 0x0A, 0x00);
        write_reg_y8950(&mut chip, 0x0B, 0x20); // end=0x20 (very short)
        write_reg_y8950(&mut chip, 0x0C, 0x00);
        write_reg_y8950(&mut chip, 0x10, 0xFF);
        write_reg_y8950(&mut chip, 0x11, 0xFF);
        write_reg_y8950(&mut chip, 0x12, 0xFF);
        write_reg_y8950(&mut chip, 0x08, 0xC0);
        write_reg_y8950(&mut chip, 0x07, 0xA0);
        f.push_str(&fmt1(
            "ADPCM_B_SHORT_SAMPLE",
            &generate_1_y8950(&mut chip, SAMPLES),
        ));
    }

    // External write
    {
        let mut chip = Y8950::new();
        chip.reset();
        chip.take_signals();
        write_reg_y8950(&mut chip, 0x07, 0x01); // reset
        write_reg_y8950(&mut chip, 0x08, 0xC0); // pan L+R
        write_reg_y8950(&mut chip, 0x09, 0x00); // start low
        write_reg_y8950(&mut chip, 0x0A, 0x00); // start high
        write_reg_y8950(&mut chip, 0x0B, 0xFF); // end low
        write_reg_y8950(&mut chip, 0x0C, 0xFF); // end high
        write_reg_y8950(&mut chip, 0x07, 0x60); // external + record
        write_reg_y8950(&mut chip, 0x0F, 0xAB); // write data byte
        f.push_str(&fmt1("EXTERNAL_WRITE", &generate_1_y8950(&mut chip, 128)));
    }

    std::fs::write(format!("{dir}/y8950_adpcm.rs"), f).unwrap();
    println!("  wrote y8950_adpcm.rs");
}

fn gen_ym3812_fm(dir: &str) {
    let mut f = header();

    // silence
    {
        let mut chip = setup_ym3812();
        f.push_str(&fmt1("SILENCE", &generate_1_opl2(&mut chip, SAMPLES)));
    }

    // single tone algo1 (additive)
    {
        let mut chip = setup_ym3812();
        setup_opl2_simple_tone(&mut chip, 0, 1, 0);
        key_on_opl2(&mut chip, 0);
        f.push_str(&fmt1(
            "SINGLE_TONE_ALGO1",
            &generate_1_opl2(&mut chip, SAMPLES),
        ));
    }

    // single tone algo0 (FM)
    {
        let mut chip = setup_ym3812();
        setup_opl2_simple_tone(&mut chip, 0, 0, 0);
        key_on_opl2(&mut chip, 0);
        f.push_str(&fmt1(
            "SINGLE_TONE_ALGO0",
            &generate_1_opl2(&mut chip, SAMPLES),
        ));
    }

    // feedback sweep
    for fb in 0..8u8 {
        let mut chip = setup_ym3812();
        setup_opl2_simple_tone(&mut chip, 0, 0, fb);
        key_on_opl2(&mut chip, 0);
        f.push_str(&fmt1(
            &format!("FEEDBACK_{fb}"),
            &generate_1_opl2(&mut chip, SAMPLES),
        ));
    }

    // ADSR
    {
        let mut chip = setup_ym3812();
        setup_opl2_simple_tone(&mut chip, 0, 1, 0);
        key_on_opl2(&mut chip, 0);
        f.push_str(&fmt1("ADSR_SUSTAIN", &generate_1_opl2(&mut chip, SAMPLES)));
        key_off_opl2(&mut chip, 0);
        f.push_str(&fmt1("ADSR_RELEASE", &generate_1_opl2(&mut chip, 512)));
    }

    // rhythm mode
    {
        let mut chip = setup_ym3812();
        for ch in 6..9u8 {
            setup_opl2_simple_tone(&mut chip, ch, 1, 0);
        }
        write_reg_opl2(&mut chip, 0xBD, 0x3F);
        f.push_str(&fmt1("RHYTHM_MODE", &generate_1_opl2(&mut chip, 512)));
    }

    // two channels
    {
        let mut chip = setup_ym3812();
        setup_opl2_simple_tone(&mut chip, 0, 1, 0);
        setup_opl2_simple_tone(&mut chip, 1, 1, 0);
        write_reg_opl2(&mut chip, 0xA1, 0x81);
        key_on_opl2(&mut chip, 0);
        key_on_opl2(&mut chip, 1);
        f.push_str(&fmt1("TWO_CHANNELS", &generate_1_opl2(&mut chip, SAMPLES)));
    }

    // all 9 channels
    {
        let mut chip = setup_ym3812();
        for ch in 0..9u8 {
            setup_opl2_simple_tone(&mut chip, ch, 1, 0);
            write_reg_opl2(&mut chip, 0xA0 + ch, 0x41 + ch * 8);
            key_on_opl2(&mut chip, ch);
        }
        f.push_str(&fmt1(
            "ALL_9_CHANNELS",
            &generate_1_opl2(&mut chip, SAMPLES),
        ));
    }

    // waveform 0-3
    for wf in 0..4u8 {
        let mut chip = setup_ym3812();
        write_reg_opl2(&mut chip, 0x01, 0x20); // enable waveform select
        setup_opl2_simple_tone(&mut chip, 0, 1, 0);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl2(&mut chip, 0xE0 + off, wf);
        }
        key_on_opl2(&mut chip, 0);
        f.push_str(&fmt1(
            &format!("WAVEFORM_{wf}"),
            &generate_1_opl2(&mut chip, SAMPLES),
        ));
    }

    // waveform gate disabled vs enabled
    {
        let mut chip = setup_ym3812();
        // waveform select NOT enabled - should use sine
        setup_opl2_simple_tone(&mut chip, 0, 1, 0);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl2(&mut chip, 0xE0 + off, 0x02); // try half-sine
        }
        key_on_opl2(&mut chip, 0);
        f.push_str(&fmt1(
            "WAVEFORM_DISABLED",
            &generate_1_opl2(&mut chip, SAMPLES),
        ));
    }
    {
        let mut chip = setup_ym3812();
        write_reg_opl2(&mut chip, 0x01, 0x20); // enable waveform select
        setup_opl2_simple_tone(&mut chip, 0, 1, 0);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl2(&mut chip, 0xE0 + off, 0x02); // half-sine
        }
        key_on_opl2(&mut chip, 0);
        f.push_str(&fmt1(
            "WAVEFORM_ENABLED",
            &generate_1_opl2(&mut chip, SAMPLES),
        ));
    }

    // key re-on
    {
        let mut chip = setup_ym3812();
        setup_opl2_simple_tone(&mut chip, 0, 1, 0);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl2(&mut chip, 0x80 + off, 0x84);
        }
        key_on_opl2(&mut chip, 0);
        f.push_str(&fmt1("REON_SUSTAIN", &generate_1_opl2(&mut chip, 128)));
        key_off_opl2(&mut chip, 0);
        f.push_str(&fmt1("REON_RELEASE", &generate_1_opl2(&mut chip, 64)));
        key_on_opl2(&mut chip, 0);
        f.push_str(&fmt1("REON_AFTER", &generate_1_opl2(&mut chip, SAMPLES)));
    }

    std::fs::write(format!("{dir}/ym3812_fm.rs"), f).unwrap();
    println!("  wrote ym3812_fm.rs");
}

fn gen_ymf262_fm(dir: &str) {
    let mut f = header();

    // silence
    {
        let mut chip = setup_ymf262();
        f.push_str(&fmt4("SILENCE", &generate_4_opl3(&mut chip, SAMPLES)));
    }

    // single tone 2-op (additive)
    {
        let mut chip = setup_ymf262();
        setup_opl3_simple_tone(&mut chip, 0, 1, 0);
        key_on_opl3(&mut chip, 0);
        f.push_str(&fmt4(
            "SINGLE_TONE_2OP",
            &generate_4_opl3(&mut chip, SAMPLES),
        ));
    }

    // single tone 2-op algo0 (FM)
    {
        let mut chip = setup_ymf262();
        setup_opl3_simple_tone(&mut chip, 0, 0, 0);
        key_on_opl3(&mut chip, 0);
        f.push_str(&fmt4(
            "SINGLE_TONE_ALGO0_2OP",
            &generate_4_opl3(&mut chip, SAMPLES),
        ));
    }

    // feedback sweep
    for fb in 0..8u8 {
        let mut chip = setup_ymf262();
        setup_opl3_simple_tone(&mut chip, 0, 0, fb);
        key_on_opl3(&mut chip, 0);
        f.push_str(&fmt4(
            &format!("FEEDBACK_{fb}"),
            &generate_4_opl3(&mut chip, SAMPLES),
        ));
    }

    // waveform 0-7
    for wf in 0..8u8 {
        let mut chip = setup_ymf262();
        setup_opl3_simple_tone(&mut chip, 0, 1, 0);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl3(&mut chip, 0xE0 + off, wf);
        }
        key_on_opl3(&mut chip, 0);
        f.push_str(&fmt4(
            &format!("WAVEFORM_{wf}"),
            &generate_4_opl3(&mut chip, SAMPLES),
        ));
    }

    // 4-op modes (channel pair 0+3)
    // 0x104 register controls 4-op enable: bit 0 = ch0+ch3 pair
    for algo in 0..4u8 {
        let mut chip = setup_ymf262();
        // Enable 4-op for channel 0+3 pair
        write_reg_opl3_hi(&mut chip, 0x04, 0x01);
        // Ch0 C0: feedback + connection bit
        let ch0_algo = algo & 0x01;
        let ch0_fb = 0; // no feedback for clean test
        write_reg_opl3(&mut chip, 0xC0, (ch0_fb << 1) | ch0_algo | 0x30);
        // Ch3 C0: connection bit for second half
        let ch3_algo = (algo >> 1) & 0x01;
        write_reg_opl3(&mut chip, 0xC3, (ch3_algo) | 0x30);
        // Setup all 4 operators
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl3(&mut chip, 0x20 + off, 0x21);
            write_reg_opl3(&mut chip, 0x40 + off, 0x00);
            write_reg_opl3(&mut chip, 0x60 + off, 0xF0);
            write_reg_opl3(&mut chip, 0x80 + off, 0x0F);
            write_reg_opl3(&mut chip, 0xE0 + off, 0x00);
        }
        for op in 0..2u8 {
            let off = opl_op_offset(3, op);
            write_reg_opl3(&mut chip, 0x20 + off, 0x21);
            write_reg_opl3(&mut chip, 0x40 + off, 0x00);
            write_reg_opl3(&mut chip, 0x60 + off, 0xF0);
            write_reg_opl3(&mut chip, 0x80 + off, 0x0F);
            write_reg_opl3(&mut chip, 0xE0 + off, 0x00);
        }
        // Set frequency on channel 0 (4-op uses ch0's frequency)
        write_reg_opl3(&mut chip, 0xA0, 0x41);
        write_reg_opl3(&mut chip, 0xB0, 0x31); // key-on
        f.push_str(&fmt4(
            &format!("FOUR_OP_ALGO_{algo}"),
            &generate_4_opl3(&mut chip, SAMPLES),
        ));
    }

    // 4-op all pairs enabled
    {
        let mut chip = setup_ymf262();
        // Enable all 4-op pairs: bits 0-5 of register 0x104
        write_reg_opl3_hi(&mut chip, 0x04, 0x3F);
        // Setup first pair (ch0+ch3)
        write_reg_opl3(&mut chip, 0xC0, 0x31); // fb=0, algo=1, L+R
        write_reg_opl3(&mut chip, 0xC3, 0x31);
        for ch_base in [0u8, 3] {
            for op in 0..2u8 {
                let off = opl_op_offset(ch_base, op);
                write_reg_opl3(&mut chip, 0x20 + off, 0x21);
                write_reg_opl3(&mut chip, 0x40 + off, 0x00);
                write_reg_opl3(&mut chip, 0x60 + off, 0xF0);
                write_reg_opl3(&mut chip, 0x80 + off, 0x0F);
                write_reg_opl3(&mut chip, 0xE0 + off, 0x00);
            }
        }
        write_reg_opl3(&mut chip, 0xA0, 0x41);
        write_reg_opl3(&mut chip, 0xB0, 0x31);
        f.push_str(&fmt4(
            "FOUR_OP_ALL_PAIRS",
            &generate_4_opl3(&mut chip, SAMPLES),
        ));
    }

    // low bank 3 channels
    {
        let mut chip = setup_ymf262();
        for ch in 0..3u8 {
            setup_opl3_simple_tone(&mut chip, ch, 1, 0);
            write_reg_opl3(&mut chip, 0xA0 + ch, 0x41 + ch * 8);
            key_on_opl3(&mut chip, ch);
        }
        f.push_str(&fmt4("LOW_BANK_3CH", &generate_4_opl3(&mut chip, SAMPLES)));
    }

    // high bank 3 channels
    {
        let mut chip = setup_ymf262();
        for ch in 9..12u8 {
            setup_opl3_simple_tone(&mut chip, ch, 1, 0);
            let ch_off = ch - 9;
            write_reg_opl3_hi(&mut chip, 0xA0 + ch_off, 0x41 + ch_off * 8);
            key_on_opl3(&mut chip, ch);
        }
        f.push_str(&fmt4("HIGH_BANK_3CH", &generate_4_opl3(&mut chip, SAMPLES)));
    }

    // all 18 channels
    {
        let mut chip = setup_ymf262();
        for ch in 0..18u8 {
            setup_opl3_simple_tone(&mut chip, ch, 1, 0);
            if ch < 9 {
                write_reg_opl3(&mut chip, 0xA0 + ch, 0x20 + ch * 5);
            } else {
                let ch_off = ch - 9;
                write_reg_opl3_hi(&mut chip, 0xA0 + ch_off, 0x20 + ch_off * 5);
            }
            key_on_opl3(&mut chip, ch);
        }
        f.push_str(&fmt4(
            "ALL_18_CHANNELS",
            &generate_4_opl3(&mut chip, SAMPLES),
        ));
    }

    // ADSR
    {
        let mut chip = setup_ymf262();
        setup_opl3_simple_tone(&mut chip, 0, 1, 0);
        key_on_opl3(&mut chip, 0);
        f.push_str(&fmt4("ADSR_SUSTAIN", &generate_4_opl3(&mut chip, SAMPLES)));
        key_off_opl3(&mut chip, 0);
        f.push_str(&fmt4("ADSR_RELEASE", &generate_4_opl3(&mut chip, 512)));
    }

    // rhythm mode
    {
        let mut chip = setup_ymf262();
        for ch in 6..9u8 {
            setup_opl3_simple_tone(&mut chip, ch, 1, 0);
        }
        write_reg_opl3(&mut chip, 0xBD, 0x3F);
        f.push_str(&fmt4("RHYTHM_MODE", &generate_4_opl3(&mut chip, 512)));
    }

    // freq zero
    {
        let mut chip = setup_ymf262();
        setup_opl3_simple_tone(&mut chip, 0, 1, 0);
        write_reg_opl3(&mut chip, 0xA0, 0x00);
        write_reg_opl3(&mut chip, 0xB0, 0x20); // key-on, block=0, fnum_hi=0
        f.push_str(&fmt4("FREQ_ZERO", &generate_4_opl3(&mut chip, SAMPLES)));
    }

    // NEW mode off vs on
    {
        let mut chip = Ymf262::new();
        chip.reset();
        // Do NOT enable NEW mode
        setup_opl3_simple_tone(&mut chip, 0, 1, 0);
        key_on_opl3(&mut chip, 0);
        f.push_str(&fmt4("NEW_MODE_OFF", &generate_4_opl3(&mut chip, SAMPLES)));
    }
    {
        let mut chip = setup_ymf262(); // NEW mode enabled
        setup_opl3_simple_tone(&mut chip, 0, 1, 0);
        key_on_opl3(&mut chip, 0);
        f.push_str(&fmt4("NEW_MODE_ON", &generate_4_opl3(&mut chip, SAMPLES)));
    }

    std::fs::write(format!("{dir}/ymf262_fm.rs"), f).unwrap();
    println!("  wrote ymf262_fm.rs");
}

fn gen_ymf262_stereo(dir: &str) {
    let mut f = header();

    // Output routing tests - 0xC0 bits 4-7 control output channels A-D
    // bit 4 = output A (left), bit 5 = output B (right)
    // bit 6 = output C, bit 7 = output D

    // Output A only
    {
        let mut chip = setup_ymf262();
        let fb_algo = 0x01 | 0x10; // algo=1, output A only
        write_reg_opl3(&mut chip, 0xC0, fb_algo);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl3(&mut chip, 0x20 + off, 0x21);
            write_reg_opl3(&mut chip, 0x40 + off, 0x00);
            write_reg_opl3(&mut chip, 0x60 + off, 0xF0);
            write_reg_opl3(&mut chip, 0x80 + off, 0x0F);
            write_reg_opl3(&mut chip, 0xE0 + off, 0x00);
        }
        write_reg_opl3(&mut chip, 0xA0, 0x41);
        write_reg_opl3(&mut chip, 0xB0, 0x31);
        f.push_str(&fmt4("OUTPUT_A_ONLY", &generate_4_opl3(&mut chip, SAMPLES)));
    }

    // Output B only
    {
        let mut chip = setup_ymf262();
        let fb_algo = 0x01 | 0x20; // algo=1, output B only
        write_reg_opl3(&mut chip, 0xC0, fb_algo);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl3(&mut chip, 0x20 + off, 0x21);
            write_reg_opl3(&mut chip, 0x40 + off, 0x00);
            write_reg_opl3(&mut chip, 0x60 + off, 0xF0);
            write_reg_opl3(&mut chip, 0x80 + off, 0x0F);
            write_reg_opl3(&mut chip, 0xE0 + off, 0x00);
        }
        write_reg_opl3(&mut chip, 0xA0, 0x41);
        write_reg_opl3(&mut chip, 0xB0, 0x31);
        f.push_str(&fmt4("OUTPUT_B_ONLY", &generate_4_opl3(&mut chip, SAMPLES)));
    }

    // Output C only
    {
        let mut chip = setup_ymf262();
        let fb_algo = 0x01 | 0x40;
        write_reg_opl3(&mut chip, 0xC0, fb_algo);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl3(&mut chip, 0x20 + off, 0x21);
            write_reg_opl3(&mut chip, 0x40 + off, 0x00);
            write_reg_opl3(&mut chip, 0x60 + off, 0xF0);
            write_reg_opl3(&mut chip, 0x80 + off, 0x0F);
            write_reg_opl3(&mut chip, 0xE0 + off, 0x00);
        }
        write_reg_opl3(&mut chip, 0xA0, 0x41);
        write_reg_opl3(&mut chip, 0xB0, 0x31);
        f.push_str(&fmt4("OUTPUT_C_ONLY", &generate_4_opl3(&mut chip, SAMPLES)));
    }

    // Output D only
    {
        let mut chip = setup_ymf262();
        let fb_algo = 0x01 | 0x80;
        write_reg_opl3(&mut chip, 0xC0, fb_algo);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl3(&mut chip, 0x20 + off, 0x21);
            write_reg_opl3(&mut chip, 0x40 + off, 0x00);
            write_reg_opl3(&mut chip, 0x60 + off, 0xF0);
            write_reg_opl3(&mut chip, 0x80 + off, 0x0F);
            write_reg_opl3(&mut chip, 0xE0 + off, 0x00);
        }
        write_reg_opl3(&mut chip, 0xA0, 0x41);
        write_reg_opl3(&mut chip, 0xB0, 0x31);
        f.push_str(&fmt4("OUTPUT_D_ONLY", &generate_4_opl3(&mut chip, SAMPLES)));
    }

    // Output A+B
    {
        let mut chip = setup_ymf262();
        let fb_algo = 0x01 | 0x30; // A+B
        write_reg_opl3(&mut chip, 0xC0, fb_algo);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl3(&mut chip, 0x20 + off, 0x21);
            write_reg_opl3(&mut chip, 0x40 + off, 0x00);
            write_reg_opl3(&mut chip, 0x60 + off, 0xF0);
            write_reg_opl3(&mut chip, 0x80 + off, 0x0F);
            write_reg_opl3(&mut chip, 0xE0 + off, 0x00);
        }
        write_reg_opl3(&mut chip, 0xA0, 0x41);
        write_reg_opl3(&mut chip, 0xB0, 0x31);
        f.push_str(&fmt4("OUTPUT_AB", &generate_4_opl3(&mut chip, SAMPLES)));
    }

    // Output all (A+B+C+D)
    {
        let mut chip = setup_ymf262();
        let fb_algo = 0x01 | 0xF0;
        write_reg_opl3(&mut chip, 0xC0, fb_algo);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl3(&mut chip, 0x20 + off, 0x21);
            write_reg_opl3(&mut chip, 0x40 + off, 0x00);
            write_reg_opl3(&mut chip, 0x60 + off, 0xF0);
            write_reg_opl3(&mut chip, 0x80 + off, 0x0F);
            write_reg_opl3(&mut chip, 0xE0 + off, 0x00);
        }
        write_reg_opl3(&mut chip, 0xA0, 0x41);
        write_reg_opl3(&mut chip, 0xB0, 0x31);
        f.push_str(&fmt4("OUTPUT_ALL", &generate_4_opl3(&mut chip, SAMPLES)));
    }

    // Output mute (no outputs selected)
    {
        let mut chip = setup_ymf262();
        let fb_algo = 0x01; // no output bits
        write_reg_opl3(&mut chip, 0xC0, fb_algo);
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl3(&mut chip, 0x20 + off, 0x21);
            write_reg_opl3(&mut chip, 0x40 + off, 0x00);
            write_reg_opl3(&mut chip, 0x60 + off, 0xF0);
            write_reg_opl3(&mut chip, 0x80 + off, 0x0F);
            write_reg_opl3(&mut chip, 0xE0 + off, 0x00);
        }
        write_reg_opl3(&mut chip, 0xA0, 0x41);
        write_reg_opl3(&mut chip, 0xB0, 0x31);
        f.push_str(&fmt4("OUTPUT_MUTE", &generate_4_opl3(&mut chip, SAMPLES)));
    }

    // Independent routing: ch0 -> A, ch1 -> B
    {
        let mut chip = setup_ymf262();
        // ch0 -> output A
        write_reg_opl3(&mut chip, 0xC0, 0x11); // algo=1, output A
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl3(&mut chip, 0x20 + off, 0x21);
            write_reg_opl3(&mut chip, 0x40 + off, 0x00);
            write_reg_opl3(&mut chip, 0x60 + off, 0xF0);
            write_reg_opl3(&mut chip, 0x80 + off, 0x0F);
            write_reg_opl3(&mut chip, 0xE0 + off, 0x00);
        }
        write_reg_opl3(&mut chip, 0xA0, 0x41);
        // ch1 -> output B
        write_reg_opl3(&mut chip, 0xC1, 0x21); // algo=1, output B
        for op in 0..2u8 {
            let off = opl_op_offset(1, op);
            write_reg_opl3(&mut chip, 0x20 + off, 0x21);
            write_reg_opl3(&mut chip, 0x40 + off, 0x00);
            write_reg_opl3(&mut chip, 0x60 + off, 0xF0);
            write_reg_opl3(&mut chip, 0x80 + off, 0x0F);
            write_reg_opl3(&mut chip, 0xE0 + off, 0x00);
        }
        write_reg_opl3(&mut chip, 0xA1, 0x81);
        key_on_opl3(&mut chip, 0);
        key_on_opl3(&mut chip, 1);
        f.push_str(&fmt4(
            "INDEPENDENT_ROUTING",
            &generate_4_opl3(&mut chip, SAMPLES),
        ));
    }

    // High bank routing: ch9 -> output A
    {
        let mut chip = setup_ymf262();
        write_reg_opl3_hi(&mut chip, 0xC0, 0x11); // algo=1, output A
        for op in 0..2u8 {
            let off = opl_op_offset(0, op);
            write_reg_opl3_hi(&mut chip, 0x20 + off, 0x21);
            write_reg_opl3_hi(&mut chip, 0x40 + off, 0x00);
            write_reg_opl3_hi(&mut chip, 0x60 + off, 0xF0);
            write_reg_opl3_hi(&mut chip, 0x80 + off, 0x0F);
            write_reg_opl3_hi(&mut chip, 0xE0 + off, 0x00);
        }
        write_reg_opl3_hi(&mut chip, 0xA0, 0x41);
        key_on_opl3(&mut chip, 9);
        f.push_str(&fmt4(
            "HIGH_BANK_ROUTING",
            &generate_4_opl3(&mut chip, SAMPLES),
        ));
    }

    std::fs::write(format!("{dir}/ymf262_stereo.rs"), f).unwrap();
    println!("  wrote ymf262_stereo.rs");
}

fn gen_ym2203_fidelity(dir: &str) {
    let mut f = header();

    for (name, fidelity) in [
        ("FIDELITY_MAX", YmfmOpnFidelity::Max),
        ("FIDELITY_MED", YmfmOpnFidelity::Med),
        ("FIDELITY_MIN", YmfmOpnFidelity::Min),
    ] {
        let mut chip = setup_ym2203(fidelity);
        setup_ym2203_simple_tone(&mut chip, 0, 7, 0);
        key_on_2203(&mut chip, 0);
        f.push_str(&fmt4(name, &generate_4(&mut chip, 64)));
    }

    std::fs::write(format!("{dir}/ym2203_fidelity.rs"), f).unwrap();
    println!("  wrote ym2203_fidelity.rs");
}

#[test]
#[ignore = "run once to regenerate golden vector files from C++ reference"]
fn generate_golden_vectors() {
    let dir = format!("{}/tests/golden", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&dir).unwrap();

    println!("Generating golden vectors...");
    gen_ym2203_fm(&dir);
    gen_ym2203_ssg(&dir);
    gen_ym2608_fm(&dir);
    gen_ym2608_stereo(&dir);
    gen_ym2608_adpcm(&dir);
    gen_ym2203_fidelity(&dir);
    gen_ym3526_fm(&dir);
    gen_y8950_fm(&dir);
    gen_y8950_adpcm(&dir);
    gen_ym3812_fm(&dir);
    gen_ymf262_fm(&dir);
    gen_ymf262_stereo(&dir);
    println!("Done!");
}
