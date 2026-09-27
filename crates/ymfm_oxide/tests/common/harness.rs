use ymfm_oxide::{
    OpllFamily, Opn2Family, Y8950, Ym2149, Ym2151, Ym2164, Ym2203, Ym2413, Ym2414, Ym2608,
    Ym2610Family, Ym3526, Ym3806, Ym3812, Ymf262, Ymf278b, Ymf288, Ymf289b, YmfmOpnFidelity,
    YmfmOutput1, YmfmOutput2, YmfmOutput3, YmfmOutput4, YmfmOutput6,
};

/// Redistributable YM2413 instrument table adapted from emu2413.
///
/// Copyright (C) Mitsutaka Okazaki 2004
///
/// This software is provided 'as-is', without any express or implied warranty.
/// In no event will the authors be held liable for any damages arising from
/// the use of this software.
///
/// Permission is granted to anyone to use this software for any purpose,
/// including commercial applications, and to alter it and redistribute it
/// freely, subject to the following restrictions:
///
/// 1. The origin of this software must not be misrepresented; you must not
///    claim that you wrote the original software. If you use this software
///    in a product, an acknowledgment in the product documentation would be
///    appreciated but is not required.
/// 2. Altered source versions must be plainly marked as such, and must not
///    be misrepresented as being the original software.
/// 3. This notice may not be removed or altered from any source distribution.
pub const EMU2413_YM2413_INSTRUMENTS: [u8; 144] = [
    0x61, 0x61, 0x1E, 0x17, 0xF0, 0x7F, 0x00, 0x17, 0x13, 0x41, 0x16, 0x0E, 0xFD, 0xF4, 0x23, 0x23,
    0x03, 0x01, 0x9A, 0x04, 0xF3, 0xF3, 0x13, 0xF3, 0x11, 0x61, 0x0E, 0x07, 0xFA, 0x64, 0x70, 0x17,
    0x22, 0x21, 0x1E, 0x06, 0xF0, 0x76, 0x00, 0x28, 0x21, 0x22, 0x16, 0x05, 0xF0, 0x71, 0x00, 0x18,
    0x21, 0x61, 0x1D, 0x07, 0x82, 0x80, 0x17, 0x17, 0x23, 0x21, 0x2D, 0x16, 0x90, 0x90, 0x00, 0x07,
    0x21, 0x21, 0x1B, 0x06, 0x64, 0x65, 0x10, 0x17, 0x21, 0x21, 0x0B, 0x1A, 0x85, 0xA0, 0x70, 0x07,
    0x23, 0x01, 0x83, 0x10, 0xFF, 0xB4, 0x10, 0xF4, 0x97, 0xC1, 0x20, 0x07, 0xFF, 0xF4, 0x22, 0x22,
    0x61, 0x00, 0x0C, 0x05, 0xC2, 0xF6, 0x40, 0x44, 0x01, 0x01, 0x56, 0x03, 0x94, 0xC2, 0x03, 0x12,
    0x21, 0x01, 0x89, 0x03, 0xF1, 0xE4, 0xF0, 0x23, 0x07, 0x21, 0x14, 0x00, 0xEE, 0xF8, 0xFF, 0xF8,
    0x21, 0x31, 0x00, 0x00, 0xA7, 0xF7, 0xF7, 0xF7, 0x25, 0x11, 0x00, 0x00, 0xF8, 0xFB, 0xF8, 0x55,
];

/// First audible C++ YMFM sample for each emu2413 preset capture.
pub const EMU2413_YM2413_PRESET_CAPTURE_STARTS: [usize; 15] =
    [379, 1, 2, 251, 175, 187, 166, 67, 195, 37, 18, 1, 2, 11, 3];

pub fn write_reg(chip: &mut Ym2203, addr: u8, data: u8) {
    chip.write_address(addr);
    chip.write_data(data);
}

pub fn write_reg_2608(chip: &mut Ym2608, addr: u8, data: u8) {
    chip.write_address(addr);
    chip.write_data(data);
}

pub fn write_reg_hi(chip: &mut Ym2608, addr: u8, data: u8) {
    chip.write_address_hi(addr);
    chip.write_data_hi(data);
}

pub fn generate_4(chip: &mut Ym2203, count: usize) -> Vec<[i32; 4]> {
    let mut output = vec![YmfmOutput4 { data: [0; 4] }; count];
    chip.generate(&mut output);
    output.iter().map(|s| s.data).collect()
}

pub fn generate_3(chip: &mut Ym2608, count: usize) -> Vec<[i32; 3]> {
    let mut output = vec![YmfmOutput3 { data: [0; 3] }; count];
    chip.generate(&mut output);
    output.iter().map(|s| s.data).collect()
}

pub fn assert_samples_4(actual: &[[i32; 4]], expected: &[[i32; 4]]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "sample count mismatch: got {}, expected {}",
        actual.len(),
        expected.len()
    );
    for (i, (got, exp)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            got, exp,
            "sample {i} mismatch: got {got:?}, expected {exp:?}"
        );
    }
}

pub fn assert_samples_3(actual: &[[i32; 3]], expected: &[[i32; 3]]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "sample count mismatch: got {}, expected {}",
        actual.len(),
        expected.len()
    );
    for (i, (got, exp)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            got, exp,
            "sample {i} mismatch: got {got:?}, expected {exp:?}"
        );
    }
}

pub fn print_golden_4(samples: &[[i32; 4]]) {
    println!("&[");
    for s in samples {
        println!("    [{}, {}, {}, {}],", s[0], s[1], s[2], s[3]);
    }
    println!("]");
}

pub fn print_golden_3(samples: &[[i32; 3]]) {
    println!("&[");
    for s in samples {
        println!("    [{}, {}, {}],", s[0], s[1], s[2]);
    }
    println!("]");
}

pub fn setup_ym2203_simple_tone(chip: &mut Ym2203, channel: u8, algorithm: u8, feedback: u8) {
    let fb_algo = (feedback << 3) | (algorithm & 0x07);
    write_reg(chip, 0xB0 + channel, fb_algo);

    for op_offset in [0x00, 0x04, 0x08, 0x0C] {
        let reg_base = channel + op_offset;
        write_reg(chip, 0x30 + reg_base, 0x01); // DT=0, MUL=1
        write_reg(chip, 0x40 + reg_base, 0x00); // TL=0 (max volume)
        write_reg(chip, 0x50 + reg_base, 0x1F); // KS=0, AR=31 (max attack)
        write_reg(chip, 0x60 + reg_base, 0x00); // AM=0, DR=0
        write_reg(chip, 0x70 + reg_base, 0x00); // SR=0
        write_reg(chip, 0x80 + reg_base, 0x0F); // SL=0, RR=15
        write_reg(chip, 0x90 + reg_base, 0x00); // SSG-EG=0
    }

    // F-number = 0x269, Block = 4 -> ~440 Hz equivalent
    write_reg(chip, 0xA4 + channel, 0x22); // Block=4, F-num high=0x02
    write_reg(chip, 0xA0 + channel, 0x69); // F-num low=0x69
}

pub fn setup_ym2608_simple_tone(chip: &mut Ym2608, channel: u8, algorithm: u8, feedback: u8) {
    let fb_algo = (feedback << 3) | (algorithm & 0x07);

    if channel < 3 {
        write_reg_2608(chip, 0xB0 + channel, fb_algo);
        for op_offset in [0x00, 0x04, 0x08, 0x0C] {
            let reg_base = channel + op_offset;
            write_reg_2608(chip, 0x30 + reg_base, 0x01);
            write_reg_2608(chip, 0x40 + reg_base, 0x00);
            write_reg_2608(chip, 0x50 + reg_base, 0x1F);
            write_reg_2608(chip, 0x60 + reg_base, 0x00);
            write_reg_2608(chip, 0x70 + reg_base, 0x00);
            write_reg_2608(chip, 0x80 + reg_base, 0x0F);
            write_reg_2608(chip, 0x90 + reg_base, 0x00);
        }
        write_reg_2608(chip, 0xA4 + channel, 0x22);
        write_reg_2608(chip, 0xA0 + channel, 0x69);
        // Enable both L+R output
        write_reg_2608(chip, 0xB4 + channel, 0xC0);
    } else {
        let ch = channel - 3;
        write_reg_hi(chip, 0xB0 + ch, fb_algo);
        for op_offset in [0x00, 0x04, 0x08, 0x0C] {
            let reg_base = ch + op_offset;
            write_reg_hi(chip, 0x30 + reg_base, 0x01);
            write_reg_hi(chip, 0x40 + reg_base, 0x00);
            write_reg_hi(chip, 0x50 + reg_base, 0x1F);
            write_reg_hi(chip, 0x60 + reg_base, 0x00);
            write_reg_hi(chip, 0x70 + reg_base, 0x00);
            write_reg_hi(chip, 0x80 + reg_base, 0x0F);
            write_reg_hi(chip, 0x90 + reg_base, 0x00);
        }
        write_reg_hi(chip, 0xA4 + ch, 0x22);
        write_reg_hi(chip, 0xA0 + ch, 0x69);
        write_reg_hi(chip, 0xB4 + ch, 0xC0);
    }
}

pub fn key_on_2203(chip: &mut Ym2203, channel: u8) {
    write_reg(chip, 0x28, 0xF0 | (channel & 0x03));
}

pub fn key_off_2203(chip: &mut Ym2203, channel: u8) {
    write_reg(chip, 0x28, channel & 0x03);
}

pub fn key_on_2608(chip: &mut Ym2608, channel: u8) {
    // For YM2608, channel 0-2 = low bank, 3-5 = high bank (encoded as 4-6 in reg 0x28)
    let ch_bits = if channel < 3 {
        channel
    } else {
        channel - 3 + 4
    };
    write_reg_2608(chip, 0x28, 0xF0 | ch_bits);
}

pub fn key_off_2608(chip: &mut Ym2608, channel: u8) {
    let ch_bits = if channel < 3 {
        channel
    } else {
        channel - 3 + 4
    };
    write_reg_2608(chip, 0x28, ch_bits);
}

pub fn setup_ym2203(fidelity: YmfmOpnFidelity) -> Ym2203 {
    let mut chip = Ym2203::new();
    chip.reset();
    chip.set_fidelity(fidelity);
    chip
}

pub fn setup_ym2608(fidelity: YmfmOpnFidelity) -> Ym2608 {
    let mut chip = Ym2608::new();
    chip.reset();
    chip.set_fidelity(fidelity);
    // Enable extended 6-channel FM mode (bit 7 of reg 0x29)
    write_reg_2608(&mut chip, 0x29, 0x80);
    chip
}

pub fn setup_ym2608_with_adpcm_data(data: Vec<u8>) -> Ym2608 {
    let mut chip = Ym2608::new();
    chip.set_adpcm_a_rom(&data);
    chip.set_adpcm_b_ram(data);
    chip
}

pub fn add_ssg_bg_2203(chip: &mut Ym2203) {
    write_reg(chip, 0x00, 0x10);
    write_reg(chip, 0x01, 0x00);
    write_reg(chip, 0x02, 0x20);
    write_reg(chip, 0x03, 0x00);
    write_reg(chip, 0x04, 0x40);
    write_reg(chip, 0x05, 0x00);
    write_reg(chip, 0x07, 0x38);
    write_reg(chip, 0x08, 0x08);
    write_reg(chip, 0x09, 0x08);
    write_reg(chip, 0x0A, 0x08);
}

pub fn add_fm_bg_2203(chip: &mut Ym2203) {
    setup_ym2203_simple_tone(chip, 2, 7, 0);
    key_on_2203(chip, 2);
}

// --- OPN2 (YM2612 / YM3438 / YMF276) helpers ---

pub fn write_reg_opn2<const VARIANT: u8>(chip: &mut Opn2Family<VARIANT>, addr: u8, data: u8) {
    chip.write_address(addr);
    chip.write_data(data);
}

pub fn write_reg_opn2_hi<const VARIANT: u8>(chip: &mut Opn2Family<VARIANT>, addr: u8, data: u8) {
    chip.write_address_hi(addr);
    chip.write_data_hi(data);
}

pub fn generate_2_opn2<const VARIANT: u8>(
    chip: &mut Opn2Family<VARIANT>,
    count: usize,
) -> Vec<[i32; 2]> {
    let mut output = vec![YmfmOutput2 { data: [0; 2] }; count];
    chip.generate(&mut output);
    output.iter().map(|s| s.data).collect()
}

pub fn assert_samples_2(actual: &[[i32; 2]], expected: &[[i32; 2]]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "sample count mismatch: got {}, expected {}",
        actual.len(),
        expected.len()
    );
    for (i, (got, exp)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            got, exp,
            "sample {i} mismatch: got {got:?}, expected {exp:?}"
        );
    }
}

pub fn setup_opn2<const VARIANT: u8>() -> Opn2Family<VARIANT> {
    let mut chip = Opn2Family::<VARIANT>::new();
    chip.reset();
    chip
}

/// A simple 4-operator tone on the given channel (0-5), routed to the correct
/// register bank (channels 0-2 low, 3-5 high).
pub fn setup_opn2_simple_tone<const VARIANT: u8>(
    chip: &mut Opn2Family<VARIANT>,
    channel: u8,
    algorithm: u8,
    feedback: u8,
) {
    let fb_algo = (feedback << 3) | (algorithm & 0x07);
    let high = channel >= 3;
    let ch = if high { channel - 3 } else { channel };
    let mut write = |addr: u8, data: u8| {
        if high {
            write_reg_opn2_hi(chip, addr, data);
        } else {
            write_reg_opn2(chip, addr, data);
        }
    };
    write(0xB0 + ch, fb_algo);
    for op_offset in [0x00, 0x04, 0x08, 0x0C] {
        let reg_base = ch + op_offset;
        write(0x30 + reg_base, 0x01);
        write(0x40 + reg_base, 0x00);
        write(0x50 + reg_base, 0x1F);
        write(0x60 + reg_base, 0x00);
        write(0x70 + reg_base, 0x00);
        write(0x80 + reg_base, 0x0F);
        write(0x90 + reg_base, 0x00);
    }
    write(0xA4 + ch, 0x22);
    write(0xA0 + ch, 0x69);
    write(0xB4 + ch, 0xC0);
}

pub fn key_on_opn2<const VARIANT: u8>(chip: &mut Opn2Family<VARIANT>, channel: u8) {
    let ch_bits = if channel < 3 {
        channel
    } else {
        channel - 3 + 4
    };
    write_reg_opn2(chip, 0x28, 0xF0 | ch_bits);
}

/// Writes one 4-operator voice on channel 0 with moderate modulator levels.
fn setup_opn2_moderate_voice<const VARIANT: u8>(chip: &mut Opn2Family<VARIANT>, fb_algo: u8) {
    write_reg_opn2(chip, 0xB0, fb_algo);
    for (op_offset, tl) in [(0x00, 0x20), (0x04, 0x20), (0x08, 0x20), (0x0C, 0x00)] {
        write_reg_opn2(chip, 0x30 + op_offset, 0x01);
        write_reg_opn2(chip, 0x40 + op_offset, tl);
        write_reg_opn2(chip, 0x50 + op_offset, 0x1F);
        write_reg_opn2(chip, 0x60 + op_offset, 0x00);
        write_reg_opn2(chip, 0x70 + op_offset, 0x00);
        write_reg_opn2(chip, 0x80 + op_offset, 0x0F);
        write_reg_opn2(chip, 0x90 + op_offset, 0x00);
    }
    write_reg_opn2(chip, 0xA4, 0x22);
    write_reg_opn2(chip, 0xA0, 0x69);
}

/// Names of the OPN2 golden scenarios shared by all three variants.
pub const OPN2_SCENARIOS: &[&str] = &[
    "SILENCE",
    "SINGLE_TONE",
    "ALGO_0",
    "ALGO_1",
    "ALGO_2",
    "ALGO_3",
    "ALGO_4",
    "ALGO_5",
    "ALGO_6",
    "ALGO_7",
    "ALL_6_CHANNELS",
    "LFO_OFF",
    "LFO_ON",
    "PAN_LEFT",
    "PAN_RIGHT",
    "DAC_MODE",
    "DAC_NEGATIVE",
    "DAC_LOW_BIT",
    "DAC_LEFT_ONLY",
    "DAC_WITH_FM",
    "DAC_KEPT_BY_RESET",
    "CSM_KEY_ON",
];

/// Builds the named OPN2 scenario and returns the chip ready to generate.
pub fn opn2_scenario<const VARIANT: u8>(name: &str) -> Opn2Family<VARIANT> {
    let mut chip = setup_opn2::<VARIANT>();
    match name {
        "SILENCE" => {}
        "SINGLE_TONE" => {
            setup_opn2_simple_tone(&mut chip, 0, 7, 0);
            key_on_opn2(&mut chip, 0);
        }
        "ALGO_0" | "ALGO_1" | "ALGO_2" | "ALGO_3" | "ALGO_4" | "ALGO_5" | "ALGO_6" | "ALGO_7" => {
            let algorithm = name.as_bytes()[5] - b'0';
            setup_opn2_moderate_voice(&mut chip, algorithm);
            write_reg_opn2(&mut chip, 0xB4, 0xC0);
            key_on_opn2(&mut chip, 0);
        }
        "ALL_6_CHANNELS" => {
            let freqs: [(u8, u8); 6] = [
                (0x22, 0x69),
                (0x24, 0x80),
                (0x26, 0xD5),
                (0x22, 0x40),
                (0x28, 0x50),
                (0x2A, 0xA0),
            ];
            for ch in 0..6u8 {
                setup_opn2_simple_tone(&mut chip, ch, 7, 0);
                let (hi, lo) = freqs[ch as usize];
                if ch < 3 {
                    write_reg_opn2(&mut chip, 0xA4 + ch, hi);
                    write_reg_opn2(&mut chip, 0xA0 + ch, lo);
                } else {
                    write_reg_opn2_hi(&mut chip, 0xA4 + (ch - 3), hi);
                    write_reg_opn2_hi(&mut chip, 0xA0 + (ch - 3), lo);
                }
                key_on_opn2(&mut chip, ch);
            }
        }
        "LFO_OFF" => {
            setup_opn2_moderate_voice(&mut chip, 0x00);
            write_reg_opn2(&mut chip, 0xB4, 0xC0);
            key_on_opn2(&mut chip, 0);
        }
        "LFO_ON" => {
            setup_opn2_moderate_voice(&mut chip, 0x00);
            write_reg_opn2(&mut chip, 0x22, 0x08); // LFO enable, rate 0
            write_reg_opn2(&mut chip, 0xB4, 0xC0 | 0x27); // AMS=2, PMS=7, L+R
            write_reg_opn2(&mut chip, 0x60, 0x80); // AM enable on operator 1
            key_on_opn2(&mut chip, 0);
        }
        "PAN_LEFT" | "PAN_RIGHT" => {
            setup_opn2_simple_tone(&mut chip, 0, 7, 0);
            write_reg_opn2(
                &mut chip,
                0xB4,
                if name == "PAN_LEFT" { 0x80 } else { 0x40 },
            );
            key_on_opn2(&mut chip, 0);
        }
        "DAC_MODE" => {
            write_reg_opn2(&mut chip, 0x2B, 0x80); // DAC enable
            write_reg_opn2(&mut chip, 0x2A, 0xC0); // DAC data (positive)
            write_reg_opn2_hi(&mut chip, 0xB6, 0xC0); // channel 6 pan L+R
        }
        "DAC_NEGATIVE" => {
            write_reg_opn2(&mut chip, 0x2B, 0x80);
            write_reg_opn2(&mut chip, 0x2A, 0x20);
            write_reg_opn2_hi(&mut chip, 0xB6, 0xC0);
        }
        "DAC_LOW_BIT" => {
            write_reg_opn2(&mut chip, 0x2B, 0x80);
            write_reg_opn2(&mut chip, 0x2A, 0x83);
            write_reg_opn2(&mut chip, 0x2C, 0x08); // low DAC bit
            write_reg_opn2_hi(&mut chip, 0xB6, 0xC0);
        }
        "DAC_LEFT_ONLY" => {
            write_reg_opn2(&mut chip, 0x2B, 0x80);
            write_reg_opn2(&mut chip, 0x2A, 0xE0);
            write_reg_opn2_hi(&mut chip, 0xB6, 0x80);
        }
        "DAC_WITH_FM" => {
            for ch in 0..6u8 {
                setup_opn2_simple_tone(&mut chip, ch, 7, 0);
                key_on_opn2(&mut chip, ch);
            }
            write_reg_opn2(&mut chip, 0x2B, 0x80);
            write_reg_opn2(&mut chip, 0x2A, 0x10);
        }
        "DAC_KEPT_BY_RESET" => {
            write_reg_opn2(&mut chip, 0x2A, 0xF0);
            write_reg_opn2(&mut chip, 0x2B, 0x80);
            chip.reset();
            write_reg_opn2_hi(&mut chip, 0xB6, 0xC0);
        }
        "CSM_KEY_ON" => {
            setup_opn2_simple_tone(&mut chip, 2, 4, 5);
            write_reg_opn2(&mut chip, 0x24, 0xFF);
            write_reg_opn2(&mut chip, 0x25, 0x03);
            write_reg_opn2(&mut chip, 0x27, 0x85); // CSM mode, enable and load timer A
            chip.timer_expired(0);
        }
        _ => panic!("unknown OPN2 scenario {name}"),
    }
    chip
}

// --- OPN3L (YMF288) helpers ---

pub fn write_reg_ymf288(chip: &mut Ymf288, addr: u8, data: u8) {
    chip.write_address(addr);
    chip.write_data(data);
}

pub fn write_reg_ymf288_hi(chip: &mut Ymf288, addr: u8, data: u8) {
    chip.write_address_hi(addr);
    chip.write_data_hi(data);
}

pub fn generate_3_ymf288(chip: &mut Ymf288, count: usize) -> Vec<[i32; 3]> {
    let mut output = vec![YmfmOutput3 { data: [0; 3] }; count];
    chip.generate(&mut output);
    output.iter().map(|s| s.data).collect()
}

pub fn setup_ymf288(fidelity: YmfmOpnFidelity) -> Ymf288 {
    let mut chip = Ymf288::new();
    chip.reset();
    chip.set_fidelity(fidelity);
    chip
}

/// A simple 4-operator tone on the given channel (0-5).
pub fn setup_ymf288_simple_tone(chip: &mut Ymf288, channel: u8, algorithm: u8, feedback: u8) {
    let fb_algo = (feedback << 3) | (algorithm & 0x07);
    let high = channel >= 3;
    let ch = if high { channel - 3 } else { channel };
    let mut write = |addr: u8, data: u8| {
        if high {
            write_reg_ymf288_hi(chip, addr, data);
        } else {
            write_reg_ymf288(chip, addr, data);
        }
    };
    write(0xB0 + ch, fb_algo);
    for op_offset in [0x00, 0x04, 0x08, 0x0C] {
        let reg_base = ch + op_offset;
        write(0x30 + reg_base, 0x01);
        write(0x40 + reg_base, 0x00);
        write(0x50 + reg_base, 0x1F);
        write(0x60 + reg_base, 0x00);
        write(0x70 + reg_base, 0x00);
        write(0x80 + reg_base, 0x0F);
        write(0x90 + reg_base, 0x00);
    }
    write(0xA4 + ch, 0x22);
    write(0xA0 + ch, 0x69);
    write(0xB4 + ch, 0xC0);
}

pub fn key_on_ymf288(chip: &mut Ymf288, channel: u8) {
    let ch_bits = if channel < 3 { channel } else { channel + 1 };
    write_reg_ymf288(chip, 0x28, 0xF0 | ch_bits);
}

/// Names of the YMF288 FM and SSG golden scenarios.
pub const YMF288_FM_SCENARIOS: &[&str] = &[
    "SILENCE",
    "TONE_CH0",
    "THREE_CHANNEL_MODE",
    "SIX_CHANNEL_MODE",
    "ALGO_0",
    "ALGO_1",
    "ALGO_2",
    "ALGO_3",
    "ALGO_4",
    "ALGO_5",
    "ALGO_6",
    "ALGO_7",
    "PAN_LEFT",
    "PAN_RIGHT",
    "LFO_ON",
    "SSG_TONE",
    "SSG_ENVELOPE",
    "FIDELITY_MIN",
    "FIDELITY_MED",
    "YMF288_MODE_TONE",
    "CSM_IGNORED",
    "PRESCALER_IGNORED",
];

/// Builds the named YMF288 FM or SSG scenario.
pub fn ymf288_fm_scenario(name: &str) -> Ymf288 {
    let fidelity = match name {
        "FIDELITY_MIN" => YmfmOpnFidelity::Min,
        "FIDELITY_MED" => YmfmOpnFidelity::Med,
        _ => YmfmOpnFidelity::Max,
    };
    let mut chip = setup_ymf288(fidelity);
    match name {
        "SILENCE" => {}
        "TONE_CH0" | "FIDELITY_MIN" | "FIDELITY_MED" => {
            setup_ymf288_simple_tone(&mut chip, 0, 7, 0);
            key_on_ymf288(&mut chip, 0);
        }
        "THREE_CHANNEL_MODE" | "SIX_CHANNEL_MODE" => {
            if name == "SIX_CHANNEL_MODE" {
                write_reg_ymf288(&mut chip, 0x29, 0x83);
            }
            for ch in 0..6u8 {
                setup_ymf288_simple_tone(&mut chip, ch, 7, 0);
                key_on_ymf288(&mut chip, ch);
            }
        }
        "ALGO_0" | "ALGO_1" | "ALGO_2" | "ALGO_3" | "ALGO_4" | "ALGO_5" | "ALGO_6" | "ALGO_7" => {
            let algorithm = name.as_bytes()[5] - b'0';
            setup_ymf288_simple_tone(&mut chip, 0, algorithm, 3);
            for op_offset in [0x00, 0x04, 0x08] {
                write_reg_ymf288(&mut chip, 0x40 + op_offset, 0x20);
            }
            key_on_ymf288(&mut chip, 0);
        }
        "PAN_LEFT" | "PAN_RIGHT" => {
            setup_ymf288_simple_tone(&mut chip, 1, 7, 0);
            write_reg_ymf288(
                &mut chip,
                0xB5,
                if name == "PAN_LEFT" { 0x80 } else { 0x40 },
            );
            key_on_ymf288(&mut chip, 1);
        }
        "LFO_ON" => {
            setup_ymf288_simple_tone(&mut chip, 0, 4, 2);
            write_reg_ymf288(&mut chip, 0x22, 0x0B);
            write_reg_ymf288(&mut chip, 0xB4, 0xC0 | 0x37);
            write_reg_ymf288(&mut chip, 0x60, 0x80);
            key_on_ymf288(&mut chip, 0);
        }
        "SSG_TONE" => {
            write_reg_ymf288(&mut chip, 0x00, 0x10);
            write_reg_ymf288(&mut chip, 0x01, 0x00);
            write_reg_ymf288(&mut chip, 0x02, 0x03);
            write_reg_ymf288(&mut chip, 0x03, 0x00);
            write_reg_ymf288(&mut chip, 0x07, 0x3C);
            write_reg_ymf288(&mut chip, 0x08, 0x0F);
            write_reg_ymf288(&mut chip, 0x09, 0x0A);
        }
        "SSG_ENVELOPE" => {
            write_reg_ymf288(&mut chip, 0x00, 0x08);
            write_reg_ymf288(&mut chip, 0x07, 0x3E);
            write_reg_ymf288(&mut chip, 0x08, 0x10);
            write_reg_ymf288(&mut chip, 0x0B, 0x01);
            write_reg_ymf288(&mut chip, 0x0C, 0x00);
            write_reg_ymf288(&mut chip, 0x0D, 0x0E);
        }
        "YMF288_MODE_TONE" => {
            write_reg_ymf288(&mut chip, 0x20, 0x02);
            setup_ymf288_simple_tone(&mut chip, 2, 5, 6);
            key_on_ymf288(&mut chip, 2);
        }
        "CSM_IGNORED" => {
            setup_ymf288_simple_tone(&mut chip, 2, 7, 0);
            write_reg_ymf288(&mut chip, 0x24, 0xFF);
            write_reg_ymf288(&mut chip, 0x25, 0x03);
            write_reg_ymf288(&mut chip, 0x27, 0x85);
            chip.timer_expired(0);
        }
        "PRESCALER_IGNORED" => {
            chip.write_address(0x2F);
            setup_ymf288_simple_tone(&mut chip, 0, 7, 0);
            key_on_ymf288(&mut chip, 0);
        }
        _ => panic!("unknown YMF288 scenario {name}"),
    }
    chip
}

/// Names of the YMF288 rhythm golden scenarios.
pub const YMF288_RHYTHM_SCENARIOS: &[&str] = &[
    "RHYTHM_BASS_DRUM",
    "RHYTHM_ALL",
    "RHYTHM_PANNED",
    "RHYTHM_WITH_FM",
    "RHYTHM_MISSING_ROM",
];

/// Builds the named YMF288 rhythm scenario.
pub fn ymf288_rhythm_scenario(name: &str) -> Ymf288 {
    let mut chip = Ymf288::new();
    if name != "RHYTHM_MISSING_ROM" {
        chip.set_adpcm_a_rom(&create_adpcm_rom());
    }
    chip.reset();
    chip.set_fidelity(YmfmOpnFidelity::Max);
    write_reg_ymf288(&mut chip, 0x11, 0x3F);
    match name {
        "RHYTHM_BASS_DRUM" | "RHYTHM_MISSING_ROM" => {
            write_reg_ymf288(&mut chip, 0x18, 0xDF);
            write_reg_ymf288(&mut chip, 0x10, 0x01);
        }
        "RHYTHM_ALL" => {
            for register in 0x18..0x1E {
                write_reg_ymf288(&mut chip, register, 0xDF);
            }
            write_reg_ymf288(&mut chip, 0x10, 0x3F);
        }
        "RHYTHM_PANNED" => {
            write_reg_ymf288(&mut chip, 0x18, 0x9F);
            write_reg_ymf288(&mut chip, 0x19, 0x5F);
            write_reg_ymf288(&mut chip, 0x10, 0x03);
        }
        "RHYTHM_WITH_FM" => {
            setup_ymf288_simple_tone(&mut chip, 0, 7, 0);
            key_on_ymf288(&mut chip, 0);
            write_reg_ymf288(&mut chip, 0x1A, 0xDF);
            write_reg_ymf288(&mut chip, 0x10, 0x04);
        }
        _ => panic!("unknown YMF288 rhythm scenario {name}"),
    }
    chip
}

pub fn add_ssg_bg_2608(chip: &mut Ym2608) {
    write_reg_2608(chip, 0x00, 0x10);
    write_reg_2608(chip, 0x01, 0x00);
    write_reg_2608(chip, 0x07, 0x3E);
    write_reg_2608(chip, 0x08, 0x0F);
}

pub fn create_adpcm_rom() -> Vec<u8> {
    let mut data = vec![0u8; 256 * 1024];
    for (i, byte) in data.iter_mut().take(0x2000).enumerate() {
        *byte = if i % 2 == 0 { 0x77 } else { 0x17 };
    }
    for i in 0..1024 {
        data[0x2000 + i] = ((i * 3) & 0xFF) as u8;
    }
    data
}

/// Creates a reset YM2413 using the emu2413 instrument table.
pub fn setup_ym2413() -> Ym2413 {
    let mut chip = Ym2413::new_with_instruments(EMU2413_YM2413_INSTRUMENTS);
    chip.reset();
    chip
}

/// Writes one OPLL register.
pub fn write_reg_opll<const VARIANT: u8>(chip: &mut OpllFamily<VARIANT>, address: u8, value: u8) {
    chip.write_address(address);
    chip.write_data(value);
}

/// Configures and keys on one OPLL melodic channel.
pub fn setup_opll_channel<const VARIANT: u8>(
    chip: &mut OpllFamily<VARIANT>,
    channel: u8,
    instrument: u8,
    frequency_low: u8,
    control: u8,
    volume: u8,
) {
    write_reg_opll(chip, 0x30 + channel, (instrument << 4) | (volume & 0x0F));
    write_reg_opll(chip, 0x10 + channel, frequency_low);
    write_reg_opll(chip, 0x20 + channel, control);
}

/// Configures the three OPLL rhythm channels.
pub fn setup_opll_rhythm<const VARIANT: u8>(chip: &mut OpllFamily<VARIANT>) {
    setup_opll_channel(chip, 6, 0, 0x40, 0x15, 0);
    setup_opll_channel(chip, 7, 0, 0x60, 0x15, 0);
    setup_opll_channel(chip, 8, 0, 0x80, 0x15, 0);
    write_reg_opll(chip, 0x36, 0x00);
    write_reg_opll(chip, 0x37, 0x00);
    write_reg_opll(chip, 0x38, 0x00);
}

/// Generates native OPLL melodic and rhythm samples.
pub fn generate_2_opll<const VARIANT: u8>(
    chip: &mut OpllFamily<VARIANT>,
    count: usize,
) -> Vec<[i32; 2]> {
    let mut output = vec![YmfmOutput2 { data: [0; 2] }; count];
    chip.generate(&mut output);
    output.iter().map(|sample| sample.data).collect()
}

/// Names of the OPLL instrument ROM golden scenarios shared by all variants.
pub const OPLL_ROM_SCENARIOS: &[&str] = &[
    "SILENCE",
    "INSTRUMENT_1",
    "INSTRUMENT_2",
    "INSTRUMENT_3",
    "INSTRUMENT_4",
    "INSTRUMENT_5",
    "INSTRUMENT_6",
    "INSTRUMENT_7",
    "INSTRUMENT_8",
    "INSTRUMENT_9",
    "INSTRUMENT_10",
    "INSTRUMENT_11",
    "INSTRUMENT_12",
    "INSTRUMENT_13",
    "INSTRUMENT_14",
    "INSTRUMENT_15",
    "USER_INSTRUMENT",
    "RHYTHM_BASS_DRUM",
    "RHYTHM_ALL",
];

/// Number of samples an OPLL ROM scenario runs before its golden window.
pub const OPLL_ROM_WARM_UP: usize = 2048;

/// Builds the named OPLL ROM scenario with the default instruments of the variant.
pub fn opll_rom_scenario<const VARIANT: u8>(name: &str) -> OpllFamily<VARIANT> {
    let mut chip = OpllFamily::<VARIANT>::new();
    chip.reset();
    if let Some(number) = name.strip_prefix("INSTRUMENT_") {
        let instrument: u8 = number.parse().unwrap();
        setup_opll_channel(&mut chip, 0, instrument, 0x58 + instrument * 7, 0x19, 0);
        return chip;
    }
    match name {
        "SILENCE" => {}
        "USER_INSTRUMENT" => {
            for (address, value) in [0xF1, 0xF1, 0x1E, 0x17, 0xF0, 0xF0, 0x00, 0x07]
                .into_iter()
                .enumerate()
            {
                write_reg_opll(&mut chip, address as u8, value);
            }
            setup_opll_channel(&mut chip, 1, 0, 0x80, 0x15, 2);
        }
        "RHYTHM_BASS_DRUM" => {
            setup_opll_rhythm(&mut chip);
            write_reg_opll(&mut chip, 0x0E, 0x30);
        }
        "RHYTHM_ALL" => {
            setup_opll_rhythm(&mut chip);
            write_reg_opll(&mut chip, 0x0E, 0x3F);
        }
        _ => panic!("unknown OPLL scenario {name}"),
    }
    chip
}

// --- OPL3L (YMF289B) helpers ---

pub fn write_reg_ymf289b(chip: &mut Ymf289b, addr: u8, data: u8) {
    chip.write_address(addr);
    chip.write_data(data);
}

pub fn write_reg_ymf289b_hi(chip: &mut Ymf289b, addr: u8, data: u8) {
    chip.write_address_hi(addr);
    chip.write_data(data);
}

pub fn generate_2_ymf289b(chip: &mut Ymf289b, count: usize) -> Vec<[i32; 2]> {
    let mut output = vec![YmfmOutput2 { data: [0; 2] }; count];
    chip.generate(&mut output);
    output.iter().map(|s| s.data).collect()
}

/// Creates a reset YMF289B in OPL3 mode.
pub fn setup_ymf289b() -> Ymf289b {
    let mut chip = Ymf289b::new();
    chip.reset();
    write_reg_ymf289b_hi(&mut chip, 0x05, 0x01);
    chip
}

/// A simple 2-operator tone on the given channel (0-17) with the given output bits.
pub fn setup_ymf289b_simple_tone(chip: &mut Ymf289b, channel: u8, algorithm: u8, outputs: u8) {
    let fb_algo = (algorithm & 0x01) | (outputs << 4);
    let high = channel >= 9;
    let ch = if high { channel - 9 } else { channel };
    let mut write = |addr: u8, data: u8| {
        if high {
            write_reg_ymf289b_hi(chip, addr, data);
        } else {
            write_reg_ymf289b(chip, addr, data);
        }
    };
    write(0xC0 + ch, fb_algo);
    for op in 0..2u8 {
        let off = opl_op_offset(ch, op);
        write(0x20 + off, 0x21);
        write(0x40 + off, 0x00);
        write(0x60 + off, 0xF0);
        write(0x80 + off, 0x0F);
        write(0xE0 + off, 0x00);
    }
    write(0xA0 + ch, 0x41);
    write(0xB0 + ch, 0x31);
}

/// Names of the YMF289B golden scenarios.
pub const YMF289B_SCENARIOS: &[&str] = &[
    "SILENCE",
    "TONE_OUTPUT_A",
    "TONE_OUTPUT_B",
    "TONE_OUTPUTS_C_AND_D",
    "HIGH_BANK_TONE",
    "FOUR_OPERATOR",
    "WAVEFORMS",
    "LOUD_CLAMPED",
    "REGISTER_CLEAR",
    "YMF289B_MODE_TONE",
];

/// Builds the named YMF289B scenario.
pub fn ymf289b_scenario(name: &str) -> Ymf289b {
    let mut chip = setup_ymf289b();
    match name {
        "SILENCE" => {}
        "TONE_OUTPUT_A" => setup_ymf289b_simple_tone(&mut chip, 0, 0, 0x1),
        "TONE_OUTPUT_B" => setup_ymf289b_simple_tone(&mut chip, 1, 1, 0x2),
        "TONE_OUTPUTS_C_AND_D" => setup_ymf289b_simple_tone(&mut chip, 2, 0, 0xC),
        "HIGH_BANK_TONE" => setup_ymf289b_simple_tone(&mut chip, 13, 0, 0x3),
        "FOUR_OPERATOR" => {
            write_reg_ymf289b_hi(&mut chip, 0x04, 0x01);
            setup_ymf289b_simple_tone(&mut chip, 3, 1, 0x3);
            setup_ymf289b_simple_tone(&mut chip, 0, 0, 0x3);
        }
        "WAVEFORMS" => {
            for channel in 0..8u8 {
                setup_ymf289b_simple_tone(&mut chip, channel, 1, 0x3);
                for op in 0..2u8 {
                    write_reg_ymf289b(&mut chip, 0xE0 + opl_op_offset(channel, op), channel);
                }
            }
        }
        "LOUD_CLAMPED" => {
            for channel in 0..18u8 {
                setup_ymf289b_simple_tone(&mut chip, channel, 1, 0x3);
            }
        }
        "REGISTER_CLEAR" => {
            setup_ymf289b_simple_tone(&mut chip, 0, 0, 0x3);
            write_reg_ymf289b_hi(&mut chip, 0x05, 0x05);
            write_reg_ymf289b_hi(&mut chip, 0x08, 0x04);
        }
        "YMF289B_MODE_TONE" => {
            write_reg_ymf289b_hi(&mut chip, 0x05, 0x05);
            setup_ymf289b_simple_tone(&mut chip, 4, 0, 0x3);
        }
        _ => panic!("unknown YMF289B scenario {name}"),
    }
    chip
}

// --- OPM (YM2151) helpers ---

pub fn write_reg_ym2151(chip: &mut Ym2151, addr: u8, data: u8) {
    chip.write_address(addr);
    chip.write_data(data);
}

pub fn generate_2_opm(chip: &mut Ym2151, count: usize) -> Vec<[i32; 2]> {
    let mut output = vec![YmfmOutput2 { data: [0; 2] }; count];
    chip.generate(&mut output);
    output.iter().map(|s| s.data).collect()
}

pub fn setup_ym2151() -> Ym2151 {
    let mut chip = Ym2151::new();
    chip.reset();
    chip
}

// Operator register offset for OPM: channel in bits 0-2, operator in bits 3-4.
pub fn opm_op_offset(channel: u8, op: u8) -> u8 {
    channel + (op << 3)
}

/// Issues the register writes of a simple 4-operator OPM tone through `write`.
pub fn opm_simple_tone(write: &mut impl FnMut(u8, u8), channel: u8, algorithm: u8, feedback: u8) {
    let pan_fb_algo = 0xC0 | (feedback << 3) | (algorithm & 0x07);
    write(0x20 + channel, pan_fb_algo);
    for op in 0..4u8 {
        let off = opm_op_offset(channel, op);
        write(0x40 + off, 0x01); // DT1=0, MUL=1
        write(0x60 + off, 0x00); // TL=0
        write(0x80 + off, 0x1F); // KS=0, AR=31
        write(0xA0 + off, 0x00); // AMS-EN=0, D1R=0
        write(0xC0 + off, 0x00); // DT2=0, D2R=0
        write(0xE0 + off, 0x0F); // D1L=0, RR=15
    }
    write(0x28 + channel, 0x4A); // key code
    write(0x30 + channel, 0x00); // key fraction
}

pub fn setup_ym2151_simple_tone(chip: &mut Ym2151, channel: u8, algorithm: u8, feedback: u8) {
    opm_simple_tone(
        &mut |address, data| write_reg_ym2151(chip, address, data),
        channel,
        algorithm,
        feedback,
    );
}

pub fn key_on_ym2151(chip: &mut Ym2151, channel: u8) {
    write_reg_ym2151(chip, 0x08, 0x78 | (channel & 0x07));
}

// --- OPP (YM2164) helpers ---

pub fn write_reg_ym2164(chip: &mut Ym2164, addr: u8, data: u8) {
    chip.write_address(addr);
    chip.write_data(data);
}

pub fn generate_2_ym2164(chip: &mut Ym2164, count: usize) -> Vec<[i32; 2]> {
    let mut output = vec![YmfmOutput2 { data: [0; 2] }; count];
    chip.generate(&mut output);
    output.iter().map(|s| s.data).collect()
}

pub fn setup_ym2164() -> Ym2164 {
    let mut chip = Ym2164::new();
    chip.reset();
    chip
}

/// Names of the YM2164 golden scenarios.
pub const YM2164_SCENARIOS: &[&str] = &[
    "SILENCE",
    "TONE_ALGO7",
    "ALL_ALGORITHMS",
    "LOW_REGISTERS",
    "LFO_AM_PM",
    "NOISE",
    "DETUNE",
];

/// Builds the named YM2164 scenario with all channels keyed on.
pub fn ym2164_scenario(name: &str) -> Ym2164 {
    let mut chip = setup_ym2164();
    let mut write = |address: u8, data: u8| write_reg_ym2164(&mut chip, address, data);
    match name {
        "SILENCE" => {}
        "TONE_ALGO7" => {
            opm_simple_tone(&mut write, 0, 7, 0);
            write(0x08, 0x78);
        }
        "ALL_ALGORITHMS" => {
            for channel in 0..8u8 {
                opm_simple_tone(&mut write, channel, channel, 3);
                write(0x28 + channel, 0x3A + channel * 4);
                write(0x08, 0x78 | channel);
            }
        }
        "LOW_REGISTERS" => {
            for address in 0x00..0x08u8 {
                write(address, 0xFF);
            }
            opm_simple_tone(&mut write, 2, 5, 2);
            write(0x08, 0x78 | 2);
        }
        "LFO_AM_PM" => {
            write(0x18, 0xF0);
            write(0x19, 0x40);
            write(0x19, 0xFF);
            write(0x1B, 0x02);
            opm_simple_tone(&mut write, 1, 7, 0);
            write(0x38 + 1, 0x71);
            for op in [0u8, 2] {
                write(0xA0 + opm_op_offset(1, op), 0x80);
            }
            write(0x08, 0x78 | 1);
        }
        "NOISE" => {
            write(0x0F, 0x88);
            opm_simple_tone(&mut write, 7, 7, 0);
            write(0x08, 0x78 | 7);
        }
        "DETUNE" => {
            opm_simple_tone(&mut write, 3, 7, 0);
            for (op, dt1_mul) in [(0u8, 0x31u8), (1, 0x72), (2, 0x13), (3, 0x54)] {
                write(0x40 + opm_op_offset(3, op), dt1_mul);
                write(0xC0 + opm_op_offset(3, op), op << 6);
            }
            write(0x08, 0x78 | 3);
        }
        _ => panic!("unknown YM2164 scenario {name}"),
    }
    chip
}

// --- Checksum and fuzz helpers ---

/// Deterministic xorshift32 stream shared by the fuzz scenarios.
pub struct XorShift32(u32);

impl XorShift32 {
    /// Creates a stream from a non-zero seed.
    pub fn new(seed: u32) -> Self {
        Self(seed)
    }

    /// Returns the next value of the stream.
    pub fn next_u32(&mut self) -> u32 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 17;
        value ^= value << 5;
        self.0 = value;
        value
    }

    /// Returns the next value of the stream, reduced to `0..limit`.
    pub fn below(&mut self, limit: u32) -> u32 {
        self.next_u32() % limit
    }
}

/// FNV-1a-64 over the little-endian bytes of every sample value.
pub fn fnv1a_samples<const N: usize>(samples: &[[i32; N]]) -> u64 {
    let mut hash: u64 = 0xCBF2_9CE4_8422_2325;
    for value in samples.iter().flatten() {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    hash
}

/// Splits `samples` into blocks of `block` samples and hashes each block.
pub fn block_checksums<const N: usize>(samples: &[[i32; N]], block: usize) -> Vec<u64> {
    samples.chunks(block).map(fnv1a_samples).collect()
}

// --- OPQ (YM3806) helpers ---

pub fn write_reg_ym3806(chip: &mut Ym3806, addr: u8, data: u8) {
    chip.write(addr, data);
}

pub fn generate_2_ym3806(chip: &mut Ym3806, count: usize) -> Vec<[i32; 2]> {
    let mut output = vec![YmfmOutput2 { data: [0; 2] }; count];
    chip.generate(&mut output);
    output.iter().map(|s| s.data).collect()
}

pub fn setup_ym3806() -> Ym3806 {
    let mut chip = Ym3806::new();
    chip.reset();
    chip
}

// Operator register offset for OPQ: channel in bits 0-2, operator in bits 3-4.
pub fn opq_op_offset(channel: u8, op: u8) -> u8 {
    channel + (op << 3)
}

/// A 4-operator OPQ tone with zero detune, multiple 1 and the same
/// frequency in both frequency register pairs.
pub fn setup_ym3806_tone(chip: &mut Ym3806, channel: u8, algorithm: u8, feedback: u8) {
    write_reg_ym3806(chip, 0x10 + channel, 0xC0 | (feedback << 3) | algorithm);
    for op in 0..4u8 {
        let offset = opq_op_offset(channel, op);
        write_reg_ym3806(chip, 0x40 + offset, 0x20); // detune 0
        write_reg_ym3806(chip, 0x40 + offset, 0x81); // multiple 1
        write_reg_ym3806(chip, 0x60 + offset, 0x00); // TL 0
        write_reg_ym3806(chip, 0x80 + offset, 0x1F); // KSR 0, AR 31
        write_reg_ym3806(chip, 0xA0 + offset, 0x00); // DR 0, sine
        write_reg_ym3806(chip, 0xC0 + offset, 0x00); // SR 0
        write_reg_ym3806(chip, 0xE0 + offset, 0x0F); // SL 0, RR 15
    }
    set_ym3806_frequency(chip, channel, 0x28, 4, 0x480);
    set_ym3806_frequency(chip, channel, 0x20, 4, 0x480);
}

/// Writes a block and 12-bit FNUM to the register pair at `high` (0x20 or 0x28).
pub fn set_ym3806_frequency(chip: &mut Ym3806, channel: u8, high: u8, block: u8, fnum: u16) {
    write_reg_ym3806(chip, high + channel, (block << 4) | (fnum >> 8) as u8);
    write_reg_ym3806(chip, high + 0x10 + channel, fnum as u8);
}

/// Keys on the given operators (bit 0 = operator 1) of a channel.
pub fn key_on_ym3806(chip: &mut Ym3806, channel: u8, operators: u8) {
    write_reg_ym3806(chip, 0x05, (operators << 3) | channel);
}

/// Number of samples in each YM3806 golden scenario.
pub const YM3806_SCENARIO_SAMPLES: usize = 256;

/// Names of the YM3806 golden scenarios with full sample vectors.
pub const YM3806_SCENARIOS: &[&str] = &[
    "SILENCE",
    "ALGORITHM_0",
    "ALGORITHM_1",
    "ALGORITHM_2",
    "ALGORITHM_3",
    "ALGORITHM_4",
    "ALGORITHM_5",
    "ALGORITHM_6",
    "ALGORITHM_7",
    "FEEDBACK",
    "HALF_SINE",
    "DETUNE_MIN",
    "DETUNE_ZERO",
    "DETUNE_MAX",
    "MULTIPLES",
    "FREQUENCY_REGISTERS",
    "BLOCKS",
    "PAN",
    "PARTIAL_KEY_ON",
    "KEY_SCALE_RATE",
    "ENVELOPE_RATES",
];

/// Names of the long YM3806 scenarios stored as block checksums.
pub const YM3806_LONG_SCENARIOS: &[&str] = &[
    "RELEASE",
    "REVERB",
    "LFO_RATE_0",
    "LFO_RATE_1",
    "LFO_RATE_2",
    "LFO_RATE_3",
    "LFO_RATE_4",
    "LFO_RATE_5",
    "LFO_RATE_6",
    "LFO_RATE_7",
    "LFO_PM_SENSITIVITY_1",
    "LFO_PM_SENSITIVITY_2",
    "LFO_PM_SENSITIVITY_3",
    "LFO_PM_SENSITIVITY_4",
    "LFO_PM_SENSITIVITY_5",
    "LFO_PM_SENSITIVITY_6",
    "LFO_PM_SENSITIVITY_7",
    "LFO_AM_SENSITIVITY_1",
    "LFO_AM_SENSITIVITY_2",
    "LFO_AM_SENSITIVITY_3",
    "LFO_DISABLED",
    "LFO_RESTART",
];

/// Number of samples in each long YM3806 scenario.
pub const YM3806_LONG_SCENARIO_SAMPLES: usize = 4096;

/// Number of samples hashed into each long scenario checksum.
pub const YM3806_CHECKSUM_BLOCK: usize = 256;

/// Runs the named YM3806 scenario and returns its samples.
pub fn ym3806_scenario(name: &str) -> Vec<[i32; 2]> {
    let mut chip = setup_ym3806();
    let chip = &mut chip;
    if let Some(algorithm) = name.strip_prefix("ALGORITHM_") {
        setup_ym3806_tone(chip, 0, algorithm.parse().unwrap(), 0);
        key_on_ym3806(chip, 0, 0x0F);
        return generate_2_ym3806(chip, YM3806_SCENARIO_SAMPLES);
    }
    match name {
        "SILENCE" => {}
        "FEEDBACK" => {
            setup_ym3806_tone(chip, 1, 0, 6);
            key_on_ym3806(chip, 1, 0x0F);
        }
        "HALF_SINE" => {
            setup_ym3806_tone(chip, 2, 4, 3);
            for op in 0..4u8 {
                write_reg_ym3806(chip, 0xA0 + opq_op_offset(2, op), 0x40);
            }
            key_on_ym3806(chip, 2, 0x0F);
        }
        "DETUNE_MIN" | "DETUNE_ZERO" | "DETUNE_MAX" => {
            let detune = match name {
                "DETUNE_MIN" => 0x00,
                "DETUNE_ZERO" => 0x20,
                _ => 0x3F,
            };
            for (channel, block) in [(0u8, 1u8), (1, 4), (2, 7)] {
                setup_ym3806_tone(chip, channel, 7, 0);
                set_ym3806_frequency(chip, channel, 0x28, block, 0xF00);
                set_ym3806_frequency(chip, channel, 0x20, block, 0xF00);
                for op in 0..4u8 {
                    let offset = opq_op_offset(channel, op);
                    write_reg_ym3806(chip, 0x40 + offset, detune);
                    write_reg_ym3806(chip, 0x40 + offset, 0x80 | (op * 4 + 3));
                    write_reg_ym3806(chip, 0x60 + offset, 0x10);
                }
                key_on_ym3806(chip, channel, 0x0F);
            }
        }
        "MULTIPLES" => {
            for channel in 0..4u8 {
                setup_ym3806_tone(chip, channel, 7, 0);
                set_ym3806_frequency(chip, channel, 0x28, 3, 0x300);
                set_ym3806_frequency(chip, channel, 0x20, 3, 0x300);
                for op in 0..4u8 {
                    let offset = opq_op_offset(channel, op);
                    write_reg_ym3806(chip, 0x40 + offset, 0x80 | (channel * 4 + op));
                    write_reg_ym3806(chip, 0x60 + offset, 0x18);
                }
                key_on_ym3806(chip, channel, 0x0F);
            }
        }
        "FREQUENCY_REGISTERS" => {
            setup_ym3806_tone(chip, 3, 7, 0);
            set_ym3806_frequency(chip, 3, 0x28, 3, 0x2AB);
            set_ym3806_frequency(chip, 3, 0x20, 5, 0x9CD);
            key_on_ym3806(chip, 3, 0x0F);
        }
        "BLOCKS" => {
            for channel in 0..8u8 {
                setup_ym3806_tone(chip, channel, 7, 0);
                set_ym3806_frequency(chip, channel, 0x28, channel, 0xFFF);
                set_ym3806_frequency(chip, channel, 0x20, 7 - channel, 0x801);
                for op in 0..4u8 {
                    write_reg_ym3806(chip, 0x60 + opq_op_offset(channel, op), 0x20);
                }
                key_on_ym3806(chip, channel, 0x0F);
            }
        }
        "PAN" => {
            for (channel, pan) in [(0u8, 0x40u8), (1, 0x80), (2, 0x00), (3, 0xC0)] {
                setup_ym3806_tone(chip, channel, 7, 0);
                set_ym3806_frequency(chip, channel, 0x28, 3 + channel, 0x500);
                set_ym3806_frequency(chip, channel, 0x20, 3 + channel, 0x500);
                write_reg_ym3806(chip, 0x10 + channel, pan | 0x07);
                key_on_ym3806(chip, channel, 0x0F);
            }
        }
        "PARTIAL_KEY_ON" => {
            setup_ym3806_tone(chip, 4, 7, 0);
            set_ym3806_frequency(chip, 4, 0x20, 5, 0x600);
            key_on_ym3806(chip, 4, 0x06);
            let mut samples = generate_2_ym3806(chip, YM3806_SCENARIO_SAMPLES / 2);
            key_on_ym3806(chip, 4, 0x09);
            samples.extend(generate_2_ym3806(chip, YM3806_SCENARIO_SAMPLES / 2));
            return samples;
        }
        "KEY_SCALE_RATE" => {
            for channel in 0..4u8 {
                setup_ym3806_tone(chip, channel, 7, 0);
                set_ym3806_frequency(chip, channel, 0x28, 7, 0xE00);
                set_ym3806_frequency(chip, channel, 0x20, 7, 0xE00);
                for op in 0..4u8 {
                    let offset = opq_op_offset(channel, op);
                    write_reg_ym3806(chip, 0x80 + offset, (channel << 6) | 0x08);
                    write_reg_ym3806(chip, 0xA0 + offset, 0x0C);
                    write_reg_ym3806(chip, 0xE0 + offset, 0x3F);
                }
                key_on_ym3806(chip, channel, 0x0F);
            }
        }
        "ENVELOPE_RATES" => {
            setup_ym3806_tone(chip, 5, 7, 0);
            for (op, (attack, decay, sustain, level)) in [
                (0x14u8, 0x10u8, 0x08u8, 0x2u8),
                (0x1A, 0x14, 0x0C, 0x5),
                (0x10, 0x1F, 0x1F, 0x9),
                (0x1F, 0x08, 0x10, 0xF),
            ]
            .into_iter()
            .enumerate()
            {
                let offset = opq_op_offset(5, op as u8);
                write_reg_ym3806(chip, 0x80 + offset, attack);
                write_reg_ym3806(chip, 0xA0 + offset, decay);
                write_reg_ym3806(chip, 0xC0 + offset, sustain);
                write_reg_ym3806(chip, 0xE0 + offset, (level << 4) | 0x0F);
            }
            key_on_ym3806(chip, 5, 0x0F);
        }
        _ => panic!("unknown YM3806 scenario {name}"),
    }
    generate_2_ym3806(chip, YM3806_SCENARIO_SAMPLES)
}

/// Sets up an LFO test tone on channel 0 with the given sensitivities.
fn setup_ym3806_lfo_tone(chip: &mut Ym3806, rate: u8, pm_sensitivity: u8, am_sensitivity: u8) {
    write_reg_ym3806(chip, 0x04, rate);
    setup_ym3806_tone(chip, 0, 7, 0);
    set_ym3806_frequency(chip, 0, 0x28, 5, 0xC00);
    set_ym3806_frequency(chip, 0, 0x20, 4, 0x700);
    write_reg_ym3806(chip, 0x18, (pm_sensitivity << 4) | am_sensitivity);
    for op in [0u8, 2] {
        write_reg_ym3806(chip, 0xA0 + opq_op_offset(0, op), 0x80);
    }
    for op in 0..4u8 {
        write_reg_ym3806(chip, 0x60 + opq_op_offset(0, op), 0x08);
    }
    key_on_ym3806(chip, 0, 0x0F);
}

/// Runs the named long YM3806 scenario and returns its samples.
pub fn ym3806_long_scenario(name: &str) -> Vec<[i32; 2]> {
    let mut chip = setup_ym3806();
    let chip = &mut chip;
    let samples = YM3806_LONG_SCENARIO_SAMPLES;
    if let Some(rate) = name.strip_prefix("LFO_RATE_") {
        setup_ym3806_lfo_tone(chip, rate.parse().unwrap(), 7, 3);
        return generate_2_ym3806(chip, samples);
    }
    if let Some(sensitivity) = name.strip_prefix("LFO_PM_SENSITIVITY_") {
        setup_ym3806_lfo_tone(chip, 6, sensitivity.parse().unwrap(), 0);
        return generate_2_ym3806(chip, samples);
    }
    if let Some(sensitivity) = name.strip_prefix("LFO_AM_SENSITIVITY_") {
        setup_ym3806_lfo_tone(chip, 7, 0, sensitivity.parse().unwrap());
        return generate_2_ym3806(chip, samples);
    }
    match name {
        "RELEASE" | "REVERB" => {
            for channel in 0..4u8 {
                setup_ym3806_tone(chip, channel, 7, 0);
                set_ym3806_frequency(chip, channel, 0x28, 3 + channel, 0x480);
                set_ym3806_frequency(chip, channel, 0x20, 3 + channel, 0x480);
                let reverb = if name == "REVERB" { 0x80 } else { 0x00 };
                write_reg_ym3806(chip, 0x18 + channel, reverb);
                for op in 0..4u8 {
                    let offset = opq_op_offset(channel, op);
                    write_reg_ym3806(chip, 0x80 + offset, (channel << 6) | 0x1F);
                    write_reg_ym3806(chip, 0xE0 + offset, 0x06 + channel * 3);
                }
                key_on_ym3806(chip, channel, 0x0F);
            }
            let mut output = generate_2_ym3806(chip, 256);
            for channel in 0..4u8 {
                key_on_ym3806(chip, channel, 0x00);
            }
            output.extend(generate_2_ym3806(chip, samples - 256));
            return output;
        }
        "LFO_DISABLED" => {
            setup_ym3806_lfo_tone(chip, 0x0F, 7, 3);
        }
        "LFO_RESTART" => {
            setup_ym3806_lfo_tone(chip, 5, 5, 2);
            let mut output = generate_2_ym3806(chip, samples / 4);
            write_reg_ym3806(chip, 0x04, 0x0D);
            output.extend(generate_2_ym3806(chip, samples / 4));
            write_reg_ym3806(chip, 0x04, 0x03);
            output.extend(generate_2_ym3806(chip, samples / 2));
            return output;
        }
        _ => panic!("unknown long YM3806 scenario {name}"),
    }
    generate_2_ym3806(chip, samples)
}

/// Number of fuzz seeds for the YM3806.
pub const YM3806_FUZZ_SEEDS: u32 = 8;

/// Number of blocks in each YM3806 fuzz run.
pub const YM3806_FUZZ_BLOCKS: usize = 16;

/// Number of samples in each YM3806 fuzz block.
pub const YM3806_FUZZ_BLOCK_SAMPLES: usize = 256;

/// Runs a YM3806 fuzz stream and returns one checksum per block.
///
/// The stream starts with a keyed tone on every channel. Before each block
/// it writes a random set of registers with random sample gaps in between.
/// Timer control writes are left out.
pub fn ym3806_fuzz(seed: u32) -> Vec<u64> {
    let mut random = XorShift32::new(seed.wrapping_mul(0x9E37_79B9) | 1);
    let mut chip = setup_ym3806();
    for channel in 0..8u8 {
        setup_ym3806_tone(&mut chip, channel, channel, channel % 4);
        set_ym3806_frequency(
            &mut chip,
            channel,
            0x28,
            2 + channel % 5,
            0x300 + 0x123 * channel as u16,
        );
        for op in 0..4u8 {
            write_reg_ym3806(&mut chip, 0x60 + opq_op_offset(channel, op), 0x10);
        }
        key_on_ym3806(&mut chip, channel, 0x0F);
    }
    let mut checksums = Vec::new();
    for _ in 0..YM3806_FUZZ_BLOCKS {
        let mut samples = Vec::new();
        let writes = 1 + random.below(12);
        for _ in 0..writes {
            let address = match random.below(8) {
                0 => 0x04,
                1 => 0x05,
                2 => 0x10 + random.below(0x10) as u8,
                3 => 0x20 + random.below(0x20) as u8,
                4 => 0x60 + random.below(0x20) as u8,
                _ => 0x40 + random.below(0xC0) as u8,
            };
            let mut data = random.next_u32() as u8;
            if (0x60..0x80).contains(&address) {
                data &= 0x3F;
            }
            write_reg_ym3806(&mut chip, address, data);
            if random.below(4) == 0 {
                let gap =
                    (random.below(24) as usize).min(YM3806_FUZZ_BLOCK_SAMPLES - samples.len());
                samples.extend(generate_2_ym3806(&mut chip, gap));
            }
        }
        let remaining = YM3806_FUZZ_BLOCK_SAMPLES - samples.len();
        samples.extend(generate_2_ym3806(&mut chip, remaining));
        checksums.push(fnv1a_samples(&samples));
    }
    checksums
}

// --- OPZ (YM2414) helpers ---

pub fn write_reg_ym2414(chip: &mut Ym2414, addr: u8, data: u8) {
    chip.write_address(addr);
    chip.write_data(data);
}

pub fn generate_2_ym2414(chip: &mut Ym2414, count: usize) -> Vec<[i32; 2]> {
    let mut output = vec![YmfmOutput2 { data: [0; 2] }; count];
    chip.generate(&mut output);
    output.iter().map(|s| s.data).collect()
}

pub fn setup_ym2414() -> Ym2414 {
    let mut chip = Ym2414::new();
    chip.reset();
    chip
}

// Operator register offset for OPZ: channel in bits 0-2, operator in bits 3-4.
pub fn opz_op_offset(channel: u8, op: u8) -> u8 {
    channel + (op << 3)
}

/// Selects `channel` for key-on writes. The write also loads the preset of
/// the channel, so it comes before the tone setup.
pub fn select_ym2414_channel(chip: &mut Ym2414, channel: u8) {
    write_reg_ym2414(chip, 0x08, channel);
}

/// A 4-operator OPZ tone on the selected channel with multiple 1, zero
/// detune and output on both sides, left keyed off.
pub fn setup_ym2414_tone(chip: &mut Ym2414, channel: u8, algorithm: u8, feedback: u8) {
    for op in 0..4u8 {
        let offset = opz_op_offset(channel, op);
        write_reg_ym2414(chip, 0x40 + offset, 0x01); // DT1 0, MUL 1
        write_reg_ym2414(chip, 0x60 + offset, 0x00); // TL 0
        write_reg_ym2414(chip, 0x80 + offset, 0x1F); // KSR 0, AR 31
        write_reg_ym2414(chip, 0xA0 + offset, 0x00); // D1R 0
        write_reg_ym2414(chip, 0xC0 + offset, 0x00); // DT2 0, D2R 0
        write_reg_ym2414(chip, 0xE0 + offset, 0x0F); // D1L 0, RR 15
    }
    write_reg_ym2414(chip, 0x28 + channel, 0x4A); // key code
    write_reg_ym2414(chip, 0x30 + channel, 0x01); // key fraction 0, output on
    write_reg_ym2414(chip, 0x20 + channel, 0x80 | (feedback << 3) | algorithm);
}

/// Keys the selected channel on or off through its 20-27 register.
pub fn key_ym2414(chip: &mut Ym2414, channel: u8, algorithm: u8, feedback: u8, on: bool) {
    let key = if on { 0x40 } else { 0x00 };
    write_reg_ym2414(
        chip,
        0x20 + channel,
        0x80 | key | (feedback << 3) | algorithm,
    );
}

/// Selects, sets up and keys on a tone on `channel`.
pub fn play_ym2414_tone(chip: &mut Ym2414, channel: u8, algorithm: u8, feedback: u8) {
    select_ym2414_channel(chip, channel);
    setup_ym2414_tone(chip, channel, algorithm, feedback);
    key_ym2414(chip, channel, algorithm, feedback, true);
}

/// Number of samples in each YM2414 golden scenario.
pub const YM2414_SCENARIO_SAMPLES: usize = 256;

/// Names of the YM2414 golden scenarios with full sample vectors.
pub const YM2414_SCENARIOS: &[&str] = &[
    "SILENCE",
    "ALGORITHM_0",
    "ALGORITHM_1",
    "ALGORITHM_2",
    "ALGORITHM_3",
    "ALGORITHM_4",
    "ALGORITHM_5",
    "ALGORITHM_6",
    "ALGORITHM_7",
    "FEEDBACK",
    "WAVEFORM_0",
    "WAVEFORM_1",
    "WAVEFORM_2",
    "WAVEFORM_3",
    "WAVEFORM_4",
    "WAVEFORM_5",
    "WAVEFORM_6",
    "WAVEFORM_7",
    "FINE_MULTIPLE",
    "HALF_MULTIPLE",
    "DETUNE",
    "DETUNE2",
    "FIXED_FREQUENCY_LOW",
    "FIXED_FREQUENCY_MID",
    "FIXED_FREQUENCY_HIGH",
    "FIXED_FREQUENCY_ZERO",
    "FIXED_AND_KEYED",
    "EG_SHIFT",
    "PAN",
    "NOISE",
    "KEY_SCALE_RATE",
    "KEY_ON_CHANNEL_MATCH",
    "PRESET_LOAD",
];

/// Runs the named YM2414 scenario and returns its samples.
pub fn ym2414_scenario(name: &str) -> Vec<[i32; 2]> {
    let mut chip = setup_ym2414();
    let chip = &mut chip;
    let samples = YM2414_SCENARIO_SAMPLES;
    if let Some(algorithm) = name.strip_prefix("ALGORITHM_") {
        play_ym2414_tone(chip, 0, algorithm.parse().unwrap(), 0);
        return generate_2_ym2414(chip, samples);
    }
    if let Some(waveform) = name.strip_prefix("WAVEFORM_") {
        let waveform: u8 = waveform.parse().unwrap();
        select_ym2414_channel(chip, 1);
        setup_ym2414_tone(chip, 1, 4, 0);
        for op in 0..4u8 {
            write_reg_ym2414(chip, 0x40 + opz_op_offset(1, op), 0x80 | (waveform << 4));
        }
        key_ym2414(chip, 1, 4, 0, true);
        return generate_2_ym2414(chip, samples);
    }
    if let Some(range) = name.strip_prefix("FIXED_FREQUENCY_") {
        let (range, frequency, fine) = match range {
            "LOW" => (4u8, 0x1u8, 0x5u8),
            "MID" => (5, 0xA, 0x3),
            "HIGH" => (7, 0xF, 0xF),
            _ => (6, 0x0, 0x0),
        };
        select_ym2414_channel(chip, 2);
        setup_ym2414_tone(chip, 2, 7, 0);
        for op in 0..4u8 {
            let offset = opz_op_offset(2, op);
            write_reg_ym2414(chip, 0x40 + offset, ((range + op) & 7) << 4 | frequency);
            write_reg_ym2414(chip, 0x40 + offset, 0x80 | fine);
            write_reg_ym2414(chip, 0x80 + offset, 0x3F);
            write_reg_ym2414(chip, 0x60 + offset, 0x10);
        }
        key_ym2414(chip, 2, 7, 0, true);
        return generate_2_ym2414(chip, samples);
    }
    match name {
        "SILENCE" => {}
        "FEEDBACK" => play_ym2414_tone(chip, 3, 0, 6),
        "FINE_MULTIPLE" | "HALF_MULTIPLE" => {
            select_ym2414_channel(chip, 4);
            setup_ym2414_tone(chip, 4, 7, 0);
            for op in 0..4u8 {
                let offset = opz_op_offset(4, op);
                let multiple = if name == "HALF_MULTIPLE" {
                    0
                } else {
                    1 + op * 3
                };
                write_reg_ym2414(chip, 0x40 + offset, multiple);
                write_reg_ym2414(chip, 0x40 + offset, 0x80 | (op * 5));
                write_reg_ym2414(chip, 0x60 + offset, 0x0C);
            }
            key_ym2414(chip, 4, 7, 0, true);
        }
        "DETUNE" | "DETUNE2" => {
            select_ym2414_channel(chip, 5);
            setup_ym2414_tone(chip, 5, 7, 0);
            write_reg_ym2414(chip, 0x28 + 5, 0x6E);
            for op in 0..4u8 {
                let offset = opz_op_offset(5, op);
                if name == "DETUNE" {
                    write_reg_ym2414(chip, 0x40 + offset, (op * 2 + 1) << 4 | 0x03);
                } else {
                    write_reg_ym2414(chip, 0xC0 + offset, op << 6);
                }
                write_reg_ym2414(chip, 0x60 + offset, 0x0C);
            }
            key_ym2414(chip, 5, 7, 0, true);
        }
        "FIXED_AND_KEYED" => {
            select_ym2414_channel(chip, 6);
            setup_ym2414_tone(chip, 6, 4, 2);
            for op in [1u8, 3] {
                let offset = opz_op_offset(6, op);
                write_reg_ym2414(chip, 0x40 + offset, 0x45);
                write_reg_ym2414(chip, 0x80 + offset, 0x3F);
            }
            key_ym2414(chip, 6, 4, 2, true);
        }
        "EG_SHIFT" => {
            select_ym2414_channel(chip, 0);
            setup_ym2414_tone(chip, 0, 7, 0);
            for op in 0..4u8 {
                let offset = opz_op_offset(0, op);
                write_reg_ym2414(chip, 0x80 + offset, 0x14);
                write_reg_ym2414(chip, 0xA0 + offset, 0x0C);
                write_reg_ym2414(chip, 0xE0 + offset, 0xCF);
                write_reg_ym2414(chip, 0xC0 + offset, 0x20 | (op << 6));
            }
            key_ym2414(chip, 0, 7, 0, true);
        }
        "PAN" => {
            for (channel, right, mono) in [
                (0u8, 0x00u8, 0x00u8),
                (1, 0x80, 0x00),
                (2, 0x00, 0x01),
                (3, 0x80, 0x01),
            ] {
                select_ym2414_channel(chip, channel);
                setup_ym2414_tone(chip, channel, 7, 0);
                write_reg_ym2414(chip, 0x28 + channel, 0x3A + channel * 8);
                write_reg_ym2414(chip, 0x30 + channel, 0x40 | mono);
                write_reg_ym2414(chip, 0x20 + channel, right | 0x40 | 0x07);
            }
        }
        "NOISE" => {
            write_reg_ym2414(chip, 0x0F, 0x8A);
            play_ym2414_tone(chip, 7, 7, 0);
        }
        "KEY_SCALE_RATE" => {
            for channel in 0..4u8 {
                select_ym2414_channel(chip, channel);
                setup_ym2414_tone(chip, channel, 7, 0);
                write_reg_ym2414(chip, 0x28 + channel, 0x7C);
                for op in 0..4u8 {
                    let offset = opz_op_offset(channel, op);
                    write_reg_ym2414(chip, 0x80 + offset, (channel << 6) | 0x08);
                    write_reg_ym2414(chip, 0xA0 + offset, 0x0C);
                    write_reg_ym2414(chip, 0xE0 + offset, 0x3F);
                }
                key_ym2414(chip, channel, 7, 0, true);
            }
        }
        "KEY_ON_CHANNEL_MATCH" => {
            select_ym2414_channel(chip, 0);
            setup_ym2414_tone(chip, 0, 7, 0);
            select_ym2414_channel(chip, 3);
            key_ym2414(chip, 0, 7, 0, true);
            let mut output = generate_2_ym2414(chip, samples / 2);
            write_reg_ym2414(chip, 0x08, 0xF8);
            key_ym2414(chip, 0, 7, 0, true);
            output.extend(generate_2_ym2414(chip, samples / 2));
            return output;
        }
        "PRESET_LOAD" => {
            select_ym2414_channel(chip, 1);
            setup_ym2414_tone(chip, 1, 7, 0);
            for op in 0..4u8 {
                let offset = opz_op_offset(1, op);
                write_reg_ym2414(chip, 0x40 + offset, 0x80 | (op << 4));
                write_reg_ym2414(chip, 0xC0 + offset, 0x20 | 0x03);
                write_reg_ym2414(chip, 0xE0 + offset, 0x24 + op);
            }
            key_ym2414(chip, 1, 7, 0, true);
            let mut output = generate_2_ym2414(chip, samples / 4);
            for op in 0..4u8 {
                write_reg_ym2414(chip, 0xE0 + opz_op_offset(1, op), 0xFF);
                write_reg_ym2414(chip, 0xC0 + opz_op_offset(1, op), 0x20 | 0x07);
            }
            key_ym2414(chip, 1, 7, 0, false);
            output.extend(generate_2_ym2414(chip, samples / 4));
            select_ym2414_channel(chip, 1);
            key_ym2414(chip, 1, 7, 0, true);
            output.extend(generate_2_ym2414(chip, samples / 4));
            key_ym2414(chip, 1, 7, 0, false);
            output.extend(generate_2_ym2414(chip, samples / 4));
            return output;
        }
        _ => panic!("unknown YM2414 scenario {name}"),
    }
    generate_2_ym2414(chip, samples)
}

/// Names of the long YM2414 scenarios stored as block checksums.
pub const YM2414_LONG_SCENARIOS: &[&str] = &[
    "RELEASE",
    "REVERB_RATES",
    "LFO_WAVEFORM_0",
    "LFO_WAVEFORM_1",
    "LFO_WAVEFORM_2",
    "LFO_WAVEFORM_3",
    "LFO2_WAVEFORM_0",
    "LFO2_WAVEFORM_1",
    "LFO2_WAVEFORM_2",
    "LFO2_WAVEFORM_3",
    "LFO_PM_SENSITIVITIES",
    "LFO2_PM_SENSITIVITIES",
    "BOTH_LFOS",
    "LFO_SYNC",
    "LFO2_SYNC",
    "FIXED_FREQUENCY_SWEEP",
    "FIXED_FREQUENCY_WITH_LFO",
    "CSM",
];

/// Number of samples in each long YM2414 scenario.
pub const YM2414_LONG_SCENARIO_SAMPLES: usize = 4096;

/// Number of samples hashed into each long scenario checksum.
pub const YM2414_CHECKSUM_BLOCK: usize = 256;

/// Plays an LFO test tone on channel `channel` with the given sensitivity
/// register (0x38 with bit 7 clear for LFO 1, set for LFO 2).
fn play_ym2414_lfo_tone(chip: &mut Ym2414, channel: u8, sensitivity: u8) {
    select_ym2414_channel(chip, channel);
    setup_ym2414_tone(chip, channel, 7, 0);
    write_reg_ym2414(chip, 0x28 + channel, 0x5C);
    write_reg_ym2414(chip, 0x38 + channel, sensitivity);
    for op in 0..4u8 {
        let offset = opz_op_offset(channel, op);
        write_reg_ym2414(chip, 0x60 + offset, 0x08);
        if op & 1 == 0 {
            write_reg_ym2414(chip, 0xA0 + offset, 0x80);
        }
    }
    key_ym2414(chip, channel, 7, 0, true);
}

/// Runs the named long YM2414 scenario and returns its samples.
pub fn ym2414_long_scenario(name: &str) -> Vec<[i32; 2]> {
    let mut chip = setup_ym2414();
    let chip = &mut chip;
    let samples = YM2414_LONG_SCENARIO_SAMPLES;
    if let Some(waveform) = name.strip_prefix("LFO_WAVEFORM_") {
        write_reg_ym2414(chip, 0x18, 0xC4);
        write_reg_ym2414(chip, 0x19, 0x60);
        write_reg_ym2414(chip, 0x19, 0xFF);
        write_reg_ym2414(chip, 0x1B, waveform.parse().unwrap());
        play_ym2414_lfo_tone(chip, 0, 0x62);
        return generate_2_ym2414(chip, samples);
    }
    if let Some(waveform) = name.strip_prefix("LFO2_WAVEFORM_") {
        let waveform: u8 = waveform.parse().unwrap();
        write_reg_ym2414(chip, 0x16, 0xB9);
        write_reg_ym2414(chip, 0x17, 0x50);
        write_reg_ym2414(chip, 0x17, 0xE0);
        write_reg_ym2414(chip, 0x1B, waveform << 2);
        play_ym2414_lfo_tone(chip, 1, 0x80 | 0x63);
        return generate_2_ym2414(chip, samples);
    }
    match name {
        "RELEASE" | "REVERB_RATES" => {
            for channel in 0..8u8 {
                select_ym2414_channel(chip, channel);
                setup_ym2414_tone(chip, channel, 7, 0);
                write_reg_ym2414(chip, 0x28 + channel, 0x2A + channel * 10);
                for op in 0..4u8 {
                    let offset = opz_op_offset(channel, op);
                    write_reg_ym2414(chip, 0x60 + offset, 0x10);
                    write_reg_ym2414(chip, 0x80 + offset, ((channel & 3) << 6) | 0x1F);
                    write_reg_ym2414(chip, 0xE0 + offset, 0x0A + (channel & 1) * 3);
                    if name == "REVERB_RATES" {
                        write_reg_ym2414(chip, 0xC0 + offset, 0x20 | channel);
                    }
                }
                key_ym2414(chip, channel, 7, 0, true);
            }
            let mut output = generate_2_ym2414(chip, 256);
            for channel in 0..8u8 {
                // the select loads the empty preset; restore the release
                // and reverb rates before the key off
                select_ym2414_channel(chip, channel);
                for op in 0..4u8 {
                    let offset = opz_op_offset(channel, op);
                    write_reg_ym2414(chip, 0xE0 + offset, 0x0A + (channel & 1) * 3);
                    if name == "REVERB_RATES" {
                        write_reg_ym2414(chip, 0xC0 + offset, 0x20 | channel);
                    }
                }
                key_ym2414(chip, channel, 7, 0, false);
            }
            output.extend(generate_2_ym2414(chip, samples - 256));
            return output;
        }
        "LFO_PM_SENSITIVITIES" | "LFO2_PM_SENSITIVITIES" => {
            let lfo2 = name == "LFO2_PM_SENSITIVITIES";
            write_reg_ym2414(chip, 0x18, 0xD2);
            write_reg_ym2414(chip, 0x19, 0xFF);
            write_reg_ym2414(chip, 0x16, 0xD8);
            write_reg_ym2414(chip, 0x17, 0xC0);
            write_reg_ym2414(chip, 0x1B, 0x06);
            for channel in 0..8u8 {
                let sensitivity = (channel << 4) | if lfo2 { 0x80 } else { 0x00 };
                play_ym2414_lfo_tone(chip, channel, sensitivity);
                write_reg_ym2414(chip, 0x28 + channel, 0x1C + channel * 12);
            }
        }
        "BOTH_LFOS" => {
            write_reg_ym2414(chip, 0x18, 0xB7);
            write_reg_ym2414(chip, 0x19, 0x7F);
            write_reg_ym2414(chip, 0x19, 0xC0);
            write_reg_ym2414(chip, 0x16, 0xC9);
            write_reg_ym2414(chip, 0x17, 0x40);
            write_reg_ym2414(chip, 0x17, 0xA0);
            write_reg_ym2414(chip, 0x1B, 0x09);
            play_ym2414_lfo_tone(chip, 2, 0x53);
            write_reg_ym2414(chip, 0x38 + 2, 0x80 | 0x32);
        }
        "LFO_SYNC" | "LFO2_SYNC" => {
            let sync = if name == "LFO_SYNC" { 0x10 } else { 0x20 };
            write_reg_ym2414(chip, 0x18, 0xA5);
            write_reg_ym2414(chip, 0x19, 0xFF);
            write_reg_ym2414(chip, 0x16, 0xA5);
            write_reg_ym2414(chip, 0x17, 0xFF);
            write_reg_ym2414(chip, 0x1B, sync | 0x0A);
            play_ym2414_lfo_tone(chip, 3, 0x70);
            write_reg_ym2414(chip, 0x38 + 3, 0x80 | 0x70);
            let mut output = Vec::new();
            for step in 0..8 {
                output.extend(generate_2_ym2414(chip, samples / 8));
                key_ym2414(chip, 3, 7, 0, step & 1 != 0);
            }
            return output;
        }
        "FIXED_FREQUENCY_SWEEP" => {
            select_ym2414_channel(chip, 4);
            setup_ym2414_tone(chip, 4, 7, 0);
            for op in 0..4u8 {
                let offset = opz_op_offset(4, op);
                write_reg_ym2414(chip, 0x80 + offset, 0x3F);
                write_reg_ym2414(chip, 0x60 + offset, 0x10);
            }
            key_ym2414(chip, 4, 7, 0, true);
            let mut output = Vec::new();
            for step in 0..16u8 {
                for op in 0..4u8 {
                    let offset = opz_op_offset(4, op);
                    write_reg_ym2414(chip, 0x40 + offset, ((step + op) & 7) << 4 | step);
                    write_reg_ym2414(chip, 0x40 + offset, 0x80 | (15 - step));
                }
                output.extend(generate_2_ym2414(chip, samples / 16));
            }
            return output;
        }
        "FIXED_FREQUENCY_WITH_LFO" => {
            write_reg_ym2414(chip, 0x18, 0xC8);
            write_reg_ym2414(chip, 0x19, 0x7F);
            write_reg_ym2414(chip, 0x19, 0xFF);
            play_ym2414_lfo_tone(chip, 5, 0x72);
            for op in 0..4u8 {
                let offset = opz_op_offset(5, op);
                write_reg_ym2414(chip, 0x40 + offset, 0x5A);
                write_reg_ym2414(chip, 0x80 + offset, 0x3F);
            }
        }
        "CSM" => {
            select_ym2414_channel(chip, 6);
            setup_ym2414_tone(chip, 6, 7, 0);
            for op in 0..4u8 {
                write_reg_ym2414(chip, 0xE0 + opz_op_offset(6, op), 0x08);
            }
            write_reg_ym2414(chip, 0x10, 0xF0);
            write_reg_ym2414(chip, 0x14, 0x85);
            let mut output = Vec::new();
            for _ in 0..8 {
                output.extend(generate_2_ym2414(chip, samples / 8));
                chip.timer_expired(0);
            }
            return output;
        }
        _ => panic!("unknown long YM2414 scenario {name}"),
    }
    generate_2_ym2414(chip, samples)
}

/// Number of fuzz seeds for the YM2414.
pub const YM2414_FUZZ_SEEDS: u32 = 8;

/// Number of blocks in each YM2414 fuzz run.
pub const YM2414_FUZZ_BLOCKS: usize = 16;

/// Number of samples in each YM2414 fuzz block.
pub const YM2414_FUZZ_BLOCK_SAMPLES: usize = 256;

/// Runs a YM2414 fuzz stream and returns one checksum per block.
///
/// The stream starts with a keyed tone on every channel. Before each block
/// it writes a random set of registers with random sample gaps in between.
/// Timer control and CT writes are left out.
pub fn ym2414_fuzz(seed: u32) -> Vec<u64> {
    let mut random = XorShift32::new(seed.wrapping_mul(0x85EB_CA6B) | 1);
    let mut chip = setup_ym2414();
    for channel in 0..8u8 {
        select_ym2414_channel(&mut chip, channel);
        setup_ym2414_tone(&mut chip, channel, channel, channel % 4);
        write_reg_ym2414(&mut chip, 0x28 + channel, 0x20 + channel * 11);
        for op in 0..4u8 {
            write_reg_ym2414(&mut chip, 0x60 + opz_op_offset(channel, op), 0x10);
        }
        key_ym2414(&mut chip, channel, channel, channel % 4, true);
    }
    let mut checksums = Vec::new();
    for _ in 0..YM2414_FUZZ_BLOCKS {
        let mut samples = Vec::new();
        let writes = 1 + random.below(12);
        for _ in 0..writes {
            let address = match random.below(10) {
                0 => 0x08,
                1 => [0x0F, 0x16, 0x17, 0x18, 0x19][random.below(5) as usize],
                2 => 0x1B,
                3 => 0x20 + random.below(0x08) as u8,
                4 => 0x28 + random.below(0x18) as u8,
                5 => 0x60 + random.below(0x20) as u8,
                _ => 0x40 + random.below(0xC0) as u8,
            };
            let mut data = random.next_u32() as u8;
            if (0x60..0x80).contains(&address) {
                data &= 0x3F;
            }
            if address == 0x1B {
                data &= 0x3F;
            }
            write_reg_ym2414(&mut chip, address, data);
            if random.below(4) == 0 {
                let gap =
                    (random.below(24) as usize).min(YM2414_FUZZ_BLOCK_SAMPLES - samples.len());
                samples.extend(generate_2_ym2414(&mut chip, gap));
            }
        }
        let remaining = YM2414_FUZZ_BLOCK_SAMPLES - samples.len();
        samples.extend(generate_2_ym2414(&mut chip, remaining));
        checksums.push(fnv1a_samples(&samples));
    }
    checksums
}

// --- SSG (YM2149) helpers ---

pub fn write_reg_ym2149(chip: &mut Ym2149, addr: u8, data: u8) {
    chip.write_address(addr);
    chip.write_data(data);
}

pub fn generate_3_ym2149(chip: &mut Ym2149, count: usize) -> Vec<[i32; 3]> {
    let mut output = vec![YmfmOutput3 { data: [0; 3] }; count];
    chip.generate(&mut output);
    output.iter().map(|s| s.data).collect()
}

pub fn setup_ym2149() -> Ym2149 {
    let mut chip = Ym2149::new();
    chip.reset();
    chip
}

/// Names of the YM2149 golden scenarios.
pub const YM2149_SCENARIOS: &[&str] = &[
    "SILENCE",
    "TONE_A",
    "THREE_CHANNELS",
    "TONE_PERIOD_ZERO",
    "NOISE",
    "NOISE_PERIOD_ZERO",
    "TONE_AND_NOISE",
    "AMPLITUDES",
    "ENVELOPE_SHAPE_0",
    "ENVELOPE_SHAPE_1",
    "ENVELOPE_SHAPE_2",
    "ENVELOPE_SHAPE_3",
    "ENVELOPE_SHAPE_4",
    "ENVELOPE_SHAPE_5",
    "ENVELOPE_SHAPE_6",
    "ENVELOPE_SHAPE_7",
    "ENVELOPE_SHAPE_8",
    "ENVELOPE_SHAPE_9",
    "ENVELOPE_SHAPE_A",
    "ENVELOPE_SHAPE_B",
    "ENVELOPE_SHAPE_C",
    "ENVELOPE_SHAPE_D",
    "ENVELOPE_SHAPE_E",
    "ENVELOPE_SHAPE_F",
    "ENVELOPE_PERIOD_ZERO",
    "ENVELOPE_TONE",
    "ENVELOPE_RESTART",
    "BUS_INTERFACE",
];

/// Number of samples in each YM2149 golden scenario.
pub const YM2149_SCENARIO_SAMPLES: usize = 512;

/// Runs the named YM2149 scenario and returns its samples.
pub fn ym2149_scenario(name: &str) -> Vec<[i32; 3]> {
    let mut chip = setup_ym2149();
    if let Some(shape) = name.strip_prefix("ENVELOPE_SHAPE_") {
        let shape = u8::from_str_radix(shape, 16).unwrap();
        write_reg_ym2149(&mut chip, 0x07, 0x3F);
        write_reg_ym2149(&mut chip, 0x08, 0x10);
        write_reg_ym2149(&mut chip, 0x0B, 0x02);
        write_reg_ym2149(&mut chip, 0x0C, 0x00);
        write_reg_ym2149(&mut chip, 0x0D, shape);
        return generate_3_ym2149(&mut chip, YM2149_SCENARIO_SAMPLES);
    }
    match name {
        "SILENCE" => {}
        "TONE_A" => {
            write_reg_ym2149(&mut chip, 0x00, 0x10);
            write_reg_ym2149(&mut chip, 0x01, 0x00);
            write_reg_ym2149(&mut chip, 0x07, 0x3E);
            write_reg_ym2149(&mut chip, 0x08, 0x0F);
        }
        "THREE_CHANNELS" => {
            write_reg_ym2149(&mut chip, 0x00, 0x10);
            write_reg_ym2149(&mut chip, 0x02, 0x23);
            write_reg_ym2149(&mut chip, 0x03, 0x00);
            write_reg_ym2149(&mut chip, 0x04, 0x07);
            write_reg_ym2149(&mut chip, 0x05, 0xF1);
            write_reg_ym2149(&mut chip, 0x07, 0xF8);
            write_reg_ym2149(&mut chip, 0x08, 0x0F);
            write_reg_ym2149(&mut chip, 0x09, 0x0A);
            write_reg_ym2149(&mut chip, 0x0A, 0x05);
        }
        "TONE_PERIOD_ZERO" => {
            write_reg_ym2149(&mut chip, 0x07, 0x3C);
            write_reg_ym2149(&mut chip, 0x08, 0x0F);
            write_reg_ym2149(&mut chip, 0x02, 0x01);
            write_reg_ym2149(&mut chip, 0x09, 0x0F);
        }
        "NOISE" => {
            write_reg_ym2149(&mut chip, 0x06, 0x05);
            write_reg_ym2149(&mut chip, 0x07, 0x07);
            write_reg_ym2149(&mut chip, 0x08, 0x0F);
            write_reg_ym2149(&mut chip, 0x09, 0x0C);
            write_reg_ym2149(&mut chip, 0x0A, 0x08);
        }
        "NOISE_PERIOD_ZERO" => {
            write_reg_ym2149(&mut chip, 0x06, 0x00);
            write_reg_ym2149(&mut chip, 0x07, 0x37);
            write_reg_ym2149(&mut chip, 0x08, 0x0F);
        }
        "TONE_AND_NOISE" => {
            write_reg_ym2149(&mut chip, 0x00, 0x0C);
            write_reg_ym2149(&mut chip, 0x06, 0x1F);
            write_reg_ym2149(&mut chip, 0x07, 0x36);
            write_reg_ym2149(&mut chip, 0x08, 0x0F);
        }
        "AMPLITUDES" => {
            write_reg_ym2149(&mut chip, 0x07, 0x3F);
            let mut samples = Vec::new();
            for amplitude in 0..16u8 {
                write_reg_ym2149(&mut chip, 0x08, amplitude);
                write_reg_ym2149(&mut chip, 0x09, 15 - amplitude);
                write_reg_ym2149(&mut chip, 0x0A, amplitude | 0xE0);
                samples.extend(generate_3_ym2149(&mut chip, YM2149_SCENARIO_SAMPLES / 16));
            }
            return samples;
        }
        "ENVELOPE_PERIOD_ZERO" => {
            write_reg_ym2149(&mut chip, 0x07, 0x3F);
            write_reg_ym2149(&mut chip, 0x09, 0x10);
            write_reg_ym2149(&mut chip, 0x0D, 0x0E);
        }
        "ENVELOPE_TONE" => {
            write_reg_ym2149(&mut chip, 0x04, 0x05);
            write_reg_ym2149(&mut chip, 0x07, 0x3B);
            write_reg_ym2149(&mut chip, 0x0A, 0x10);
            write_reg_ym2149(&mut chip, 0x0B, 0x04);
            write_reg_ym2149(&mut chip, 0x0D, 0x0A);
        }
        "ENVELOPE_RESTART" => {
            write_reg_ym2149(&mut chip, 0x07, 0x3F);
            write_reg_ym2149(&mut chip, 0x08, 0x10);
            write_reg_ym2149(&mut chip, 0x0B, 0x03);
            write_reg_ym2149(&mut chip, 0x0D, 0x0D);
            let mut samples = generate_3_ym2149(&mut chip, 50);
            write_reg_ym2149(&mut chip, 0x0D, 0x0D);
            samples.extend(generate_3_ym2149(&mut chip, 70));
            write_reg_ym2149(&mut chip, 0x0D, 0x04);
            samples.extend(generate_3_ym2149(&mut chip, YM2149_SCENARIO_SAMPLES - 120));
            return samples;
        }
        "BUS_INTERFACE" => {
            for (address, data) in [(0x00, 0x0E), (0x07, 0x3E), (0x08, 0x0F)] {
                chip.write(0, address);
                chip.write(2, data);
            }
            chip.write(3, 0x08);
            chip.write(1, 0x09);
            chip.write(2, 0x0B);
            chip.write(1, 0x00);
            let mut samples = generate_3_ym2149(&mut chip, YM2149_SCENARIO_SAMPLES / 2);
            chip.write(0, 0x18);
            chip.write(2, 0x06);
            samples.extend(generate_3_ym2149(&mut chip, YM2149_SCENARIO_SAMPLES / 2));
            return samples;
        }
        _ => panic!("unknown YM2149 scenario {name}"),
    }
    generate_3_ym2149(&mut chip, YM2149_SCENARIO_SAMPLES)
}

// --- OPL helpers ---

pub fn write_reg_opl(chip: &mut Ym3526, addr: u8, data: u8) {
    chip.write_address(addr);
    chip.write_data(data);
}

pub fn write_reg_y8950(chip: &mut Y8950, addr: u8, data: u8) {
    chip.write_address(addr);
    chip.write_data(data);
}

pub fn write_reg_opl2(chip: &mut Ym3812, addr: u8, data: u8) {
    chip.write_address(addr);
    chip.write_data(data);
}

pub fn write_reg_opl3(chip: &mut Ymf262, addr: u8, data: u8) {
    chip.write_address(addr);
    chip.write_data(data);
}

pub fn write_reg_opl3_hi(chip: &mut Ymf262, addr: u8, data: u8) {
    chip.write_address_hi(addr);
    chip.write_data(data);
}

pub fn generate_1_opl(chip: &mut Ym3526, count: usize) -> Vec<[i32; 1]> {
    let mut output = vec![YmfmOutput1 { data: [0] }; count];
    chip.generate(&mut output);
    output.iter().map(|s| s.data).collect()
}

pub fn generate_1_y8950(chip: &mut Y8950, count: usize) -> Vec<[i32; 1]> {
    let mut output = vec![YmfmOutput1 { data: [0] }; count];
    chip.generate(&mut output);
    output.iter().map(|s| s.data).collect()
}

pub fn generate_1_opl2(chip: &mut Ym3812, count: usize) -> Vec<[i32; 1]> {
    let mut output = vec![YmfmOutput1 { data: [0] }; count];
    chip.generate(&mut output);
    output.iter().map(|s| s.data).collect()
}

pub fn generate_4_opl3(chip: &mut Ymf262, count: usize) -> Vec<[i32; 4]> {
    let mut output = vec![YmfmOutput4 { data: [0; 4] }; count];
    chip.generate(&mut output);
    output.iter().map(|s| s.data).collect()
}

pub fn assert_samples_1(actual: &[[i32; 1]], expected: &[[i32; 1]]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "sample count mismatch: got {}, expected {}",
        actual.len(),
        expected.len()
    );
    for (i, (got, exp)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            got, exp,
            "sample {i} mismatch: got {got:?}, expected {exp:?}"
        );
    }
}

#[allow(dead_code)]
pub fn print_golden_1(samples: &[[i32; 1]]) {
    println!("&[");
    for s in samples {
        println!("    [{}],", s[0]);
    }
    println!("]");
}

pub fn opl_op_offset(channel: u8, op: u8) -> u8 {
    (channel % 3) + 8 * (channel / 3) + 3 * op
}

pub fn setup_opl_simple_tone(chip: &mut Ym3526, channel: u8, algorithm: u8, feedback: u8) {
    let fb_algo = (feedback << 1) | (algorithm & 0x01);
    write_reg_opl(chip, 0xC0 + channel, fb_algo);

    for op in 0..2u8 {
        let off = opl_op_offset(channel, op);
        write_reg_opl(chip, 0x20 + off, 0x21);
        write_reg_opl(chip, 0x40 + off, 0x00);
        write_reg_opl(chip, 0x60 + off, 0xF0);
        write_reg_opl(chip, 0x80 + off, 0x0F);
        write_reg_opl(chip, 0xE0 + off, 0x00);
    }

    write_reg_opl(chip, 0xA0 + channel, 0x41);
    write_reg_opl(chip, 0xB0 + channel, 0x11);
}

pub fn setup_y8950_simple_tone(chip: &mut Y8950, channel: u8, algorithm: u8, feedback: u8) {
    let fb_algo = (feedback << 1) | (algorithm & 0x01);
    write_reg_y8950(chip, 0xC0 + channel, fb_algo);

    for op in 0..2u8 {
        let off = opl_op_offset(channel, op);
        write_reg_y8950(chip, 0x20 + off, 0x21);
        write_reg_y8950(chip, 0x40 + off, 0x00);
        write_reg_y8950(chip, 0x60 + off, 0xF0);
        write_reg_y8950(chip, 0x80 + off, 0x0F);
        write_reg_y8950(chip, 0xE0 + off, 0x00);
    }

    write_reg_y8950(chip, 0xA0 + channel, 0x41);
    write_reg_y8950(chip, 0xB0 + channel, 0x11);
}

pub fn setup_opl2_simple_tone(chip: &mut Ym3812, channel: u8, algorithm: u8, feedback: u8) {
    let fb_algo = (feedback << 1) | (algorithm & 0x01);
    write_reg_opl2(chip, 0xC0 + channel, fb_algo);

    for op in 0..2u8 {
        let off = opl_op_offset(channel, op);
        write_reg_opl2(chip, 0x20 + off, 0x21);
        write_reg_opl2(chip, 0x40 + off, 0x00);
        write_reg_opl2(chip, 0x60 + off, 0xF0);
        write_reg_opl2(chip, 0x80 + off, 0x0F);
        write_reg_opl2(chip, 0xE0 + off, 0x00);
    }

    write_reg_opl2(chip, 0xA0 + channel, 0x41);
    write_reg_opl2(chip, 0xB0 + channel, 0x11);
}

pub fn setup_opl3_simple_tone(chip: &mut Ymf262, channel: u8, algorithm: u8, feedback: u8) {
    let fb_algo = (feedback << 1) | (algorithm & 0x01) | 0x30; // L+R output

    if channel < 9 {
        write_reg_opl3(chip, 0xC0 + channel, fb_algo);
        for op in 0..2u8 {
            let off = opl_op_offset(channel, op);
            write_reg_opl3(chip, 0x20 + off, 0x21);
            write_reg_opl3(chip, 0x40 + off, 0x00);
            write_reg_opl3(chip, 0x60 + off, 0xF0);
            write_reg_opl3(chip, 0x80 + off, 0x0F);
            write_reg_opl3(chip, 0xE0 + off, 0x00);
        }
        write_reg_opl3(chip, 0xA0 + channel, 0x41);
        write_reg_opl3(chip, 0xB0 + channel, 0x11);
    } else {
        let ch = channel - 9;
        write_reg_opl3_hi(chip, 0xC0 + ch, fb_algo);
        for op in 0..2u8 {
            let off = opl_op_offset(ch, op);
            write_reg_opl3_hi(chip, 0x20 + off, 0x21);
            write_reg_opl3_hi(chip, 0x40 + off, 0x00);
            write_reg_opl3_hi(chip, 0x60 + off, 0xF0);
            write_reg_opl3_hi(chip, 0x80 + off, 0x0F);
            write_reg_opl3_hi(chip, 0xE0 + off, 0x00);
        }
        write_reg_opl3_hi(chip, 0xA0 + ch, 0x41);
        write_reg_opl3_hi(chip, 0xB0 + ch, 0x11);
    }
}

pub fn key_on_opl(chip: &mut Ym3526, channel: u8) {
    write_reg_opl(chip, 0xB0 + channel, 0x31);
}

pub fn key_off_opl(chip: &mut Ym3526, channel: u8) {
    write_reg_opl(chip, 0xB0 + channel, 0x11);
}

pub fn key_on_y8950(chip: &mut Y8950, channel: u8) {
    write_reg_y8950(chip, 0xB0 + channel, 0x31);
}

pub fn key_off_y8950(chip: &mut Y8950, channel: u8) {
    write_reg_y8950(chip, 0xB0 + channel, 0x11);
}

pub fn key_on_opl2(chip: &mut Ym3812, channel: u8) {
    write_reg_opl2(chip, 0xB0 + channel, 0x31);
}

pub fn key_off_opl2(chip: &mut Ym3812, channel: u8) {
    write_reg_opl2(chip, 0xB0 + channel, 0x11);
}

pub fn key_on_opl3(chip: &mut Ymf262, channel: u8) {
    if channel < 9 {
        write_reg_opl3(chip, 0xB0 + channel, 0x31);
    } else {
        write_reg_opl3_hi(chip, 0xB0 + (channel - 9), 0x31);
    }
}

pub fn key_off_opl3(chip: &mut Ymf262, channel: u8) {
    if channel < 9 {
        write_reg_opl3(chip, 0xB0 + channel, 0x11);
    } else {
        write_reg_opl3_hi(chip, 0xB0 + (channel - 9), 0x11);
    }
}

pub fn setup_ym3526() -> Ym3526 {
    let mut chip = Ym3526::new();
    chip.reset();
    chip
}

pub fn setup_y8950() -> Y8950 {
    let mut chip = Y8950::new();
    chip.reset();
    chip
}

pub fn setup_y8950_with_adpcm_data(data: Vec<u8>) -> Y8950 {
    let mut chip = Y8950::new();
    chip.set_adpcm_memory(data);
    chip
}

pub fn setup_ym3812() -> Ym3812 {
    let mut chip = Ym3812::new();
    chip.reset();
    chip
}

pub fn setup_ymf262() -> Ymf262 {
    let mut chip = Ymf262::new();
    chip.reset();
    write_reg_opl3_hi(&mut chip, 0x05, 0x01); // Enable OPL3 NEW mode
    chip
}

pub struct AdpcmTester {
    chip: Ym2608,
    output: String,
}

impl AdpcmTester {
    pub fn new() -> Self {
        let mut chip = Ym2608::new();
        chip.reset();
        chip.set_adpcm_b_ram(vec![0x80; 0x40000]);
        let mut tester = Self {
            chip,
            output: String::new(),
        };
        tester.out(0x00, 0x01).out(0x00, 0x00).nl();
        tester
    }

    pub fn output(&self) -> &str {
        &self.output
    }

    pub fn out(&mut self, reg: u16, data: u8) -> &mut Self {
        use std::fmt::Write;
        write!(self.output, "O{:02X}:{:02X} ", reg, data).unwrap();
        self.chip.write_address_hi(reg as u8);
        self.chip.write_data_hi(data);
        self
    }

    pub fn inp(&mut self, reg: u16) -> &mut Self {
        use std::fmt::Write;
        self.chip.write_address_hi(reg as u8);
        let result = self.chip.read_data_hi();
        write!(self.output, "I{:02X}:{:02X} ", reg, result).unwrap();
        self
    }

    pub fn stat(&mut self) -> &mut Self {
        use std::fmt::Write;
        let status = self.chip.read_status_hi(false);
        write!(self.output, "S{:02X}    ", status).unwrap();
        self
    }

    pub fn out0(&mut self, reg: u16, data: u8) -> &mut Self {
        use std::fmt::Write;
        write!(self.output, "O{:02X}:{:02X} ", reg, data).unwrap();
        self.chip.write_address(reg as u8);
        self.chip.write_data(data);
        self
    }

    pub fn nl(&mut self) -> &mut Self {
        self.output.push('\n');
        self
    }

    pub fn msg(&mut self, s: &str) -> &mut Self {
        use std::fmt::Write;
        write!(self.output, "\n{}\n", s).unwrap();
        self
    }

    pub fn mwr(&mut self, mut data: u8, count: u16) -> &mut Self {
        use std::fmt::Write;
        for _ in 0..count {
            self.chip.write_address_hi(8);
            self.chip.write_data_hi(data);
            let stat = self.chip.read_status_hi(false);
            self.chip.write_address_hi(0x10);
            self.chip.write_data_hi(0x80);
            write!(self.output, "W{:02X}:{:02X} ", data, stat).unwrap();
            data = data.wrapping_add(1);
        }
        self
    }

    pub fn mrd(&mut self, count: u16) -> &mut Self {
        use std::fmt::Write;
        for _ in 0..count {
            self.chip.write_address_hi(8);
            let data = self.chip.read_data_hi();
            let stat = self.chip.read_status_hi(false);
            self.chip.write_address_hi(0x10);
            self.chip.write_data_hi(0x80);
            write!(self.output, "R{:02X}:{:02X} ", data, stat).unwrap();
        }
        self
    }

    pub fn reset(&mut self) -> &mut Self {
        self.out(0x00, 0x01).out(0x00, 0x00).nl();
        self
    }

    pub fn seq_mem_limit(&mut self, adr: u16) -> &mut Self {
        self.out(0x0C, (adr & 0xFF) as u8)
            .out(0x0D, ((adr >> 8) & 0xFF) as u8);
        self
    }

    pub fn seq_mem_write(
        &mut self,
        start: u16,
        stop: u16,
        data: u8,
        count: u16,
        message: &str,
    ) -> &mut Self {
        self.msg(message);
        self.out(0x10, 0x00).out(0x10, 0x80);
        self.out(0x00, 0x60).out(0x01, 0x02);
        self.out(0x02, (start & 0xFF) as u8)
            .out(0x03, ((start >> 8) & 0xFF) as u8);
        self.out(0x04, (stop & 0xFF) as u8)
            .out(0x05, ((stop >> 8) & 0xFF) as u8);
        self.nl();
        self.mwr(data, count).nl();
        self.out(0x00, 0x00).out(0x10, 0x80).nl();
        self
    }

    pub fn seq_mem_read(&mut self, start: u16, stop: u16, count: u16, message: &str) -> &mut Self {
        self.msg(message);
        self.out(0x10, 0x00).out(0x10, 0x80);
        self.out(0x00, 0x20).out(0x01, 0x02);
        self.out(0x02, (start & 0xFF) as u8)
            .out(0x03, ((start >> 8) & 0xFF) as u8);
        self.out(0x04, (stop & 0xFF) as u8)
            .out(0x05, ((stop >> 8) & 0xFF) as u8);
        self.nl();
        self.mrd(count).nl();
        self.out(0x00, 0x00).out(0x10, 0x80).nl();
        self
    }
}

pub fn create_y8950_adpcm_data() -> Vec<u8> {
    let mut data = vec![0u8; 256 * 1024];
    for (i, byte) in data.iter_mut().take(0x2000).enumerate() {
        *byte = if i % 2 == 0 { 0x77 } else { 0x17 };
    }
    for i in 0..1024 {
        data[0x2000 + i] = ((i * 3) & 0xFF) as u8;
    }
    data
}

pub fn write_reg_2610<const FM_CHANNEL_MASK: u32>(
    chip: &mut Ym2610Family<FM_CHANNEL_MASK>,
    addr: u8,
    data: u8,
) {
    chip.write_address(addr);
    chip.write_data(data);
}

pub fn write_reg_2610_hi<const FM_CHANNEL_MASK: u32>(
    chip: &mut Ym2610Family<FM_CHANNEL_MASK>,
    addr: u8,
    data: u8,
) {
    chip.write_address_hi(addr);
    chip.write_data_hi(data);
}

pub fn generate_3_2610<const FM_CHANNEL_MASK: u32>(
    chip: &mut Ym2610Family<FM_CHANNEL_MASK>,
    count: usize,
) -> Vec<[i32; 3]> {
    let mut output = vec![YmfmOutput3 { data: [0; 3] }; count];
    chip.generate(&mut output);
    output.iter().map(|s| s.data).collect()
}

/// Creates a reset YM2610 family chip. `with_roms` attaches the synthetic ADPCM ROMs.
pub fn setup_ym2610<const FM_CHANNEL_MASK: u32>(
    fidelity: YmfmOpnFidelity,
    with_roms: bool,
) -> Ym2610Family<FM_CHANNEL_MASK> {
    let mut chip = Ym2610Family::<FM_CHANNEL_MASK>::new();
    if with_roms {
        chip.set_adpcm_a_rom(create_ym2610_adpcm_a_rom());
        chip.set_adpcm_b_rom(create_ym2610_adpcm_b_rom());
    }
    chip.reset();
    chip.set_fidelity(fidelity);
    chip
}

/// Synthetic 192 KiB ADPCM-A ROM. Samples above 64 KiB need the 8-bit address shift.
pub fn create_ym2610_adpcm_a_rom() -> Vec<u8> {
    (0..0x30000usize)
        .map(|i| (((i * 7) ^ (i >> 3)) & 0xFF) as u8)
        .collect()
}

/// Synthetic 128 KiB ADPCM-B ROM.
pub fn create_ym2610_adpcm_b_rom() -> Vec<u8> {
    (0..0x20000usize)
        .map(|i| (((i * 13) + (i >> 4)) & 0xFF) as u8)
        .collect()
}

pub fn setup_ym2610_simple_tone<const FM_CHANNEL_MASK: u32>(
    chip: &mut Ym2610Family<FM_CHANNEL_MASK>,
    channel: u8,
    algorithm: u8,
    feedback: u8,
) {
    let fb_algo = (feedback << 3) | (algorithm & 0x07);
    let high = channel >= 3;
    let ch = if high { channel - 3 } else { channel };
    let mut write = |addr: u8, data: u8| {
        if high {
            write_reg_2610_hi(chip, addr, data);
        } else {
            write_reg_2610(chip, addr, data);
        }
    };
    write(0xB0 + ch, fb_algo);
    for op_offset in [0x00, 0x04, 0x08, 0x0C] {
        let reg_base = ch + op_offset;
        write(0x30 + reg_base, 0x01);
        write(0x40 + reg_base, 0x00);
        write(0x50 + reg_base, 0x1F);
        write(0x60 + reg_base, 0x00);
        write(0x70 + reg_base, 0x00);
        write(0x80 + reg_base, 0x0F);
        write(0x90 + reg_base, 0x00);
    }
    write(0xA4 + ch, 0x22);
    write(0xA0 + ch, 0x69);
    write(0xB4 + ch, 0xC0);
}

pub fn key_on_2610<const FM_CHANNEL_MASK: u32>(
    chip: &mut Ym2610Family<FM_CHANNEL_MASK>,
    channel: u8,
) {
    let ch_bits = if channel < 3 { channel } else { channel + 1 };
    write_reg_2610(chip, 0x28, 0xF0 | ch_bits);
}

pub fn add_ssg_tone_2610<const FM_CHANNEL_MASK: u32>(chip: &mut Ym2610Family<FM_CHANNEL_MASK>) {
    write_reg_2610(chip, 0x00, 0x10);
    write_reg_2610(chip, 0x01, 0x00);
    write_reg_2610(chip, 0x07, 0x3E);
    write_reg_2610(chip, 0x08, 0x0F);
}

/// Keys on ADPCM-A channel 0 (start/end 0x0123, both sides) and channel 5 (start/end 0x0200, left).
pub fn start_ym2610_adpcm_a<const FM_CHANNEL_MASK: u32>(chip: &mut Ym2610Family<FM_CHANNEL_MASK>) {
    for (addr, data) in [
        (0x10, 0x23),
        (0x18, 0x01),
        (0x20, 0x23),
        (0x28, 0x01),
        (0x15, 0x00),
        (0x1D, 0x02),
        (0x25, 0x00),
        (0x2D, 0x02),
        (0x08, 0xDF),
        (0x0D, 0x9F),
        (0x01, 0x3F),
        (0x00, 0x21),
    ] {
        write_reg_2610_hi(chip, addr, data);
    }
}

/// Starts ADPCM-B playback of 0x0100-0x0100. Control 1 also sets the record bit, which the chip ignores.
pub fn start_ym2610_adpcm_b<const FM_CHANNEL_MASK: u32>(chip: &mut Ym2610Family<FM_CHANNEL_MASK>) {
    for (addr, data) in [
        (0x11, 0xC0),
        (0x12, 0x00),
        (0x13, 0x01),
        (0x14, 0x00),
        (0x15, 0x01),
        (0x19, 0x55),
        (0x1A, 0x55),
        (0x1B, 0xFF),
        (0x10, 0xC0),
    ] {
        write_reg_2610(chip, addr, data);
    }
}

// --- OPL4 (YMF278B) helpers ---

/// Base address of the YMF278B test wave RAM.
pub const YMF278B_RAM_BASE: u32 = 0x20_0000;

/// Size of the YMF278B test wave ROM.
pub const YMF278B_ROM_SIZE: usize = 0x4000;

/// Size of the YMF278B test wave RAM.
pub const YMF278B_RAM_SIZE: usize = 0x1000;

/// Offset of the free test RAM area that scenarios upload data into.
pub const YMF278B_RAM_UPLOAD_OFFSET: u32 = 0xC00;

/// Writes a 12-byte wave table header at `offset`. `end` is the sample count.
fn write_ymf278b_header(
    memory: &mut [u8],
    offset: usize,
    format: u8,
    base: u32,
    loop_start: u16,
    end: u16,
    registers: [u8; 5],
) {
    let negated_end = end.wrapping_neg();
    memory[offset] = (format << 6) | ((base >> 16) & 0x3F) as u8;
    memory[offset + 1] = (base >> 8) as u8;
    memory[offset + 2] = base as u8;
    memory[offset + 3] = (loop_start >> 8) as u8;
    memory[offset + 4] = loop_start as u8;
    memory[offset + 5] = (negated_end >> 8) as u8;
    memory[offset + 6] = negated_end as u8;
    memory[offset + 7..offset + 12].copy_from_slice(&registers);
}

/// Returns a triangle wave value in `-(1 << (bits - 1))..(1 << (bits - 1))`.
fn triangle(index: usize, period: usize, bits: u32) -> i32 {
    let phase = index % period;
    let half = period / 2;
    let full_scale = 1i32 << bits;
    let rising = if phase < half { phase } else { period - phase };
    (rising as i32 * full_scale / half as i32).min(full_scale - 1) - (full_scale >> 1)
}

/// Stores 12-bit samples packed two into three bytes.
fn write_ymf278b_12bit(memory: &mut [u8], offset: usize, samples: &[i32]) {
    for (pair, chunk) in samples.chunks(2).enumerate() {
        let first = chunk[0] as u32 & 0xFFF;
        let second = chunk.get(1).copied().unwrap_or(0) as u32 & 0xFFF;
        let address = offset + pair * 3;
        memory[address] = (first >> 4) as u8;
        memory[address + 1] = ((first & 0x0F) | ((second & 0x0F) << 4)) as u8;
        memory[address + 2] = (second >> 4) as u8;
    }
}

/// Stores 16-bit samples high byte first.
fn write_ymf278b_16bit(memory: &mut [u8], offset: usize, samples: &[i32]) {
    for (index, sample) in samples.iter().enumerate() {
        memory[offset + index * 2] = (*sample >> 8) as u8;
        memory[offset + index * 2 + 1] = *sample as u8;
    }
}

/// Builds the YMF278B test wave ROM.
///
/// Waves 0 to 6 cover the 8-bit, 12-bit and 16-bit formats with different
/// loops and envelope defaults. Wave 7 plays 16-bit data from the test RAM.
/// Wave 384 has its bank 0 header at 0x1200.
pub fn create_ymf278b_rom() -> Vec<u8> {
    let mut rom = vec![0u8; YMF278B_ROM_SIZE];
    let plain = [0x00, 0xF0, 0x00, 0xF7, 0x00];

    write_ymf278b_header(&mut rom, 0, 0, 0x2000, 0, 256, plain);
    for index in 0..256 {
        rom[0x2000 + index] = triangle(index, 256, 8) as u8;
    }

    write_ymf278b_header(&mut rom, 12, 1, 0x2200, 0, 256, plain);
    let samples: Vec<i32> = (0..256).map(|index| triangle(index, 128, 12)).collect();
    write_ymf278b_12bit(&mut rom, 0x2200, &samples);

    write_ymf278b_header(&mut rom, 24, 2, 0x2400, 0, 256, plain);
    let samples: Vec<i32> = (0..256).map(|index| (index * 251) as i16 as i32).collect();
    write_ymf278b_16bit(&mut rom, 0x2400, &samples);

    write_ymf278b_header(
        &mut rom,
        36,
        0,
        0x2600,
        192,
        256,
        [0x00, 0xE4, 0x32, 0xF6, 0x00],
    );
    let mut random = XorShift32::new(0x2780_1234);
    for index in 0..256 {
        rom[0x2600 + index] = random.next_u32() as u8;
    }

    write_ymf278b_header(
        &mut rom,
        48,
        2,
        0x2800,
        32,
        64,
        [0x3A, 0xF0, 0x00, 0xF7, 0x03],
    );
    let samples: Vec<i32> = (0..64).map(|index| triangle(index, 32, 16)).collect();
    write_ymf278b_16bit(&mut rom, 0x2800, &samples);

    write_ymf278b_header(
        &mut rom,
        60,
        1,
        0x2900,
        101,
        255,
        [0x09, 0xD3, 0x21, 0xE8, 0x01],
    );
    let samples: Vec<i32> = (0..255).map(|index| triangle(index, 85, 12)).collect();
    write_ymf278b_12bit(&mut rom, 0x2900, &samples);

    write_ymf278b_header(&mut rom, 72, 3, 0x2B00, 0, 128, plain);
    let samples: Vec<i32> = (0..128).map(|index| triangle(index, 64, 12)).collect();
    write_ymf278b_12bit(&mut rom, 0x2B00, &samples);

    write_ymf278b_header(&mut rom, 84, 2, YMF278B_RAM_BASE + 0x400, 64, 128, plain);

    write_ymf278b_header(&mut rom, 12 * 384, 0, 0x2C00, 0, 64, plain);
    for index in 0..64 {
        rom[0x2C00 + index] = if index < 32 { 0x60 } else { 0xA0 };
    }
    rom
}

/// Builds the YMF278B test wave RAM, mapped at [`YMF278B_RAM_BASE`].
///
/// The RAM starts with the bank 4 headers of waves 384 and 385 and holds the
/// data of wave 7 at 0x400 and of wave 384 at 0x800.
pub fn create_ymf278b_ram() -> Vec<u8> {
    let mut ram = vec![0u8; YMF278B_RAM_SIZE];
    let plain = [0x00, 0xF0, 0x00, 0xF7, 0x00];

    write_ymf278b_header(&mut ram, 0, 2, YMF278B_RAM_BASE + 0x800, 0, 64, plain);
    write_ymf278b_header(
        &mut ram,
        12,
        1,
        0x2000,
        0,
        128,
        [0x12, 0xF2, 0x44, 0xF9, 0x02],
    );

    let samples: Vec<i32> = (0..128).map(|index| triangle(index, 128, 16)).collect();
    write_ymf278b_16bit(&mut ram, 0x400, &samples);

    let samples: Vec<i32> = (0..64)
        .map(|index| (index * 1024 - 32768) as i16 as i32)
        .collect();
    write_ymf278b_16bit(&mut ram, 0x800, &samples);
    ram
}

pub fn write_reg_ymf278b(chip: &mut Ymf278b, addr: u16, data: u8) {
    if addr >= 0x100 {
        chip.write_address_hi(addr as u8);
    } else {
        chip.write_address(addr as u8);
    }
    chip.write_data(data);
}

pub fn write_pcm_ymf278b(chip: &mut Ymf278b, reg: u8, data: u8) {
    chip.write_address_pcm(reg);
    chip.write_data_pcm(data);
}

pub fn generate_6_ymf278b(chip: &mut Ymf278b, count: usize) -> Vec<[i32; 6]> {
    let mut output = vec![YmfmOutput6 { data: [0; 6] }; count];
    chip.generate(&mut output);
    output.iter().map(|s| s.data).collect()
}

pub fn assert_samples_6(actual: &[[i32; 6]], expected: &[[i32; 6]]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "sample count mismatch: got {}, expected {}",
        actual.len(),
        expected.len()
    );
    for (i, (got, exp)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            got, exp,
            "sample {i} mismatch: got {got:?}, expected {exp:?}"
        );
    }
}

/// Creates a YMF278B with the test wave memory in OPL4 mode (NEW and NEW2 set).
pub fn setup_ymf278b() -> Ymf278b {
    let mut chip = setup_ymf278b_compatible();
    write_reg_ymf278b(&mut chip, 0x105, 0x03);
    chip
}

/// Creates a YMF278B with the test wave memory in OPL2 compatibility mode.
pub fn setup_ymf278b_compatible() -> Ymf278b {
    let mut chip = Ymf278b::new();
    chip.set_pcm_rom(create_ymf278b_rom());
    chip.set_pcm_ram(YMF278B_RAM_BASE, create_ymf278b_ram());
    chip.reset();
    chip
}

/// Sets up a 2-operator FM tone. `outputs` holds the output enable bits 4 to 7
/// of register 0xC0.
pub fn setup_ymf278b_fm_tone(
    chip: &mut Ymf278b,
    channel: u8,
    algorithm: u8,
    feedback: u8,
    outputs: u8,
) {
    let bank = if channel < 9 { 0x000 } else { 0x100 };
    let local = channel % 9;
    write_reg_ymf278b(
        chip,
        bank + 0xC0 + local as u16,
        outputs | (feedback << 1) | (algorithm & 1),
    );
    for op in 0..2u8 {
        let offset = bank + opl_op_offset(local, op) as u16;
        write_reg_ymf278b(chip, 0x20 + offset, 0x21);
        write_reg_ymf278b(chip, 0x40 + offset, 0x00);
        write_reg_ymf278b(chip, 0x60 + offset, 0xF0);
        write_reg_ymf278b(chip, 0x80 + offset, 0x0F);
        write_reg_ymf278b(chip, 0xE0 + offset, 0x00);
    }
    write_reg_ymf278b(chip, bank + 0xA0 + local as u16, 0x41);
    write_reg_ymf278b(chip, bank + 0xB0 + local as u16, 0x11);
}

/// Keys an FM channel on or off.
pub fn key_ymf278b_fm(chip: &mut Ymf278b, channel: u8, on: bool) {
    let bank = if channel < 9 { 0x000 } else { 0x100 };
    let data = if on { 0x31 } else { 0x11 };
    write_reg_ymf278b(chip, bank + 0xB0 + (channel % 9) as u16, data);
}

/// Sets the pitch of a PCM channel and the high bit of its wave number.
pub fn set_ymf278b_pcm_pitch(chip: &mut Ymf278b, channel: u8, wave: u16, octave: i8, fnum: u16) {
    write_pcm_ymf278b(
        chip,
        0x20 + channel,
        (((fnum & 0x7F) as u8) << 1) | (wave >> 8) as u8,
    );
    write_pcm_ymf278b(
        chip,
        0x38 + channel,
        ((octave as u8 & 0x0F) << 4) | (fnum >> 7) as u8,
    );
}

/// Selects wave `wave` on a PCM channel at full level without keying it on.
pub fn load_ymf278b_pcm(chip: &mut Ymf278b, channel: u8, wave: u16, octave: i8, fnum: u16) {
    set_ymf278b_pcm_pitch(chip, channel, wave, octave, fnum);
    write_pcm_ymf278b(chip, 0x50 + channel, 0x01);
    write_pcm_ymf278b(chip, 0x08 + channel, wave as u8);
}

/// Writes the key control register (key on, damp, LFO reset, output channel, pan).
pub fn key_ymf278b_pcm(chip: &mut Ymf278b, channel: u8, control: u8) {
    write_pcm_ymf278b(chip, 0x68 + channel, control);
}

/// Selects a wave at full level and keys it on with centered pan.
pub fn play_ymf278b_pcm(chip: &mut Ymf278b, channel: u8, wave: u16, octave: i8, fnum: u16) {
    load_ymf278b_pcm(chip, channel, wave, octave, fnum);
    key_ymf278b_pcm(chip, channel, 0x80);
}

/// Writes `data` into wave memory at `address` through the memory port.
pub fn upload_ymf278b_memory(chip: &mut Ymf278b, address: u32, data: &[u8]) {
    write_pcm_ymf278b(chip, 0x02, 0x03);
    write_pcm_ymf278b(chip, 0x03, (address >> 16) as u8);
    write_pcm_ymf278b(chip, 0x04, (address >> 8) as u8);
    write_pcm_ymf278b(chip, 0x05, address as u8);
    for byte in data {
        write_pcm_ymf278b(chip, 0x06, *byte);
    }
    write_pcm_ymf278b(chip, 0x02, 0x02);
}

/// Number of samples in each YMF278B golden scenario.
pub const YMF278B_SCENARIO_SAMPLES: usize = 256;

/// Names of the YMF278B FM golden scenarios with full sample vectors.
pub const YMF278B_FM_SCENARIOS: &[&str] = &[
    "SILENCE",
    "FM_TONE",
    "FM_FEEDBACK",
    "FM_OUTPUTS_2_3",
    "FM_ALL_OUTPUTS",
    "FM_HIGH_BANK",
    "FM_FOUR_OP",
    "FM_WAVEFORM_5",
    "FM_COMPATIBILITY_MODE",
    "FM_RELEASE",
];

/// Runs the named YMF278B FM scenario and returns its samples.
pub fn ymf278b_fm_scenario(name: &str) -> Vec<[i32; 6]> {
    let mut chip = if name == "FM_COMPATIBILITY_MODE" {
        setup_ymf278b_compatible()
    } else {
        setup_ymf278b()
    };
    let chip = &mut chip;
    let samples = YMF278B_SCENARIO_SAMPLES;
    match name {
        "SILENCE" => {}
        "FM_TONE" => {
            setup_ymf278b_fm_tone(chip, 0, 0, 0, 0x30);
            key_ymf278b_fm(chip, 0, true);
        }
        "FM_COMPATIBILITY_MODE" => {
            setup_ymf278b_fm_tone(chip, 0, 0, 0, 0xC0);
            write_reg_ymf278b(chip, 0x01, 0x20);
            for op in 0..2u8 {
                write_reg_ymf278b(chip, 0xE0 + opl_op_offset(0, op) as u16, 0x05);
            }
            key_ymf278b_fm(chip, 0, true);
        }
        "FM_FEEDBACK" => {
            setup_ymf278b_fm_tone(chip, 1, 0, 5, 0x30);
            key_ymf278b_fm(chip, 1, true);
        }
        "FM_OUTPUTS_2_3" => {
            setup_ymf278b_fm_tone(chip, 2, 1, 0, 0xC0);
            key_ymf278b_fm(chip, 2, true);
        }
        "FM_ALL_OUTPUTS" => {
            setup_ymf278b_fm_tone(chip, 3, 0, 3, 0xF0);
            key_ymf278b_fm(chip, 3, true);
        }
        "FM_HIGH_BANK" => {
            setup_ymf278b_fm_tone(chip, 13, 0, 2, 0x50);
            key_ymf278b_fm(chip, 13, true);
        }
        "FM_FOUR_OP" => {
            write_reg_ymf278b(chip, 0x104, 0x01);
            setup_ymf278b_fm_tone(chip, 0, 1, 0, 0x30);
            setup_ymf278b_fm_tone(chip, 3, 0, 0, 0x30);
            key_ymf278b_fm(chip, 0, true);
        }
        "FM_WAVEFORM_5" => {
            setup_ymf278b_fm_tone(chip, 4, 0, 0, 0x30);
            for op in 0..2u8 {
                write_reg_ymf278b(chip, 0xE0 + opl_op_offset(4, op) as u16, 0x05);
            }
            key_ymf278b_fm(chip, 4, true);
        }
        "FM_RELEASE" => {
            setup_ymf278b_fm_tone(chip, 5, 0, 0, 0x30);
            for op in 0..2u8 {
                write_reg_ymf278b(chip, 0x80 + opl_op_offset(5, op) as u16, 0x07);
            }
            key_ymf278b_fm(chip, 5, true);
            let mut result = generate_6_ymf278b(chip, 64);
            key_ymf278b_fm(chip, 5, false);
            result.extend(generate_6_ymf278b(chip, samples - 64));
            return result;
        }
        _ => panic!("unknown YMF278B FM scenario {name}"),
    }
    generate_6_ymf278b(chip, samples)
}

/// Names of the YMF278B PCM golden scenarios with full sample vectors.
pub const YMF278B_PCM_SCENARIOS: &[&str] = &[
    "PCM_8BIT",
    "PCM_12BIT",
    "PCM_16BIT",
    "PCM_FORMAT_3",
    "PCM_SHORT_LOOP",
    "PCM_ODD_LOOP",
    "PCM_OCTAVE_MINUS_8",
    "PCM_OCTAVE_MINUS_3",
    "PCM_OCTAVE_PLUS_2",
    "PCM_OCTAVE_PLUS_7",
    "PCM_FNUMBER",
    "PCM_PAN_0",
    "PCM_PAN_1",
    "PCM_PAN_2",
    "PCM_PAN_3",
    "PCM_PAN_4",
    "PCM_PAN_5",
    "PCM_PAN_6",
    "PCM_PAN_7",
    "PCM_PAN_8",
    "PCM_PAN_9",
    "PCM_PAN_10",
    "PCM_PAN_11",
    "PCM_PAN_12",
    "PCM_PAN_13",
    "PCM_PAN_14",
    "PCM_PAN_15",
    "PCM_OUTPUT_CHANNEL",
    "MIX_0",
    "MIX_1",
    "MIX_2",
    "MIX_3",
    "MIX_4",
    "MIX_5",
    "MIX_6",
    "MIX_7",
    "PCM_ATTACK_8",
    "PCM_ATTACK_12",
    "PCM_ATTACK_14",
    "PCM_DECAY_SUSTAIN",
    "PCM_RELEASE",
    "PCM_DAMP",
    "PCM_REVERB",
    "PCM_RATE_CORRECTION",
    "PCM_LEVEL_DIRECT",
    "PCM_LEVEL_INTERPOLATION",
    "PCM_VIBRATO",
    "PCM_TREMOLO",
    "PCM_LFO_RESET",
    "PCM_HEADER_DEFAULTS",
    "PCM_BANK_ROM_HEADER",
    "PCM_BANK_RAM_HEADER",
    "PCM_BANK_RAM_REFORMAT",
    "PCM_RAM_SAMPLE",
    "PCM_MEMORY_UPLOAD",
    "PCM_KEY_ON_OFF_PENDING",
    "PCM_RETRIGGER",
    "PCM_WAVE_CHANGE",
    "PCM_ALL_CHANNELS",
    "PCM_AND_FM",
    "PCM_NEW2_OFF",
];

/// Overrides the envelope registers of a PCM channel.
pub fn set_ymf278b_pcm_envelope(
    chip: &mut Ymf278b,
    channel: u8,
    attack_decay: u8,
    sustain: u8,
    correction_release: u8,
) {
    write_pcm_ymf278b(chip, 0x98 + channel, attack_decay);
    write_pcm_ymf278b(chip, 0xB0 + channel, sustain);
    write_pcm_ymf278b(chip, 0xC8 + channel, correction_release);
}

/// Runs the named YMF278B PCM scenario and returns its samples.
pub fn ymf278b_pcm_scenario(name: &str) -> Vec<[i32; 6]> {
    let mut chip = setup_ymf278b();
    let chip = &mut chip;
    let samples = YMF278B_SCENARIO_SAMPLES;
    if let Some(pan) = name.strip_prefix("PCM_PAN_") {
        load_ymf278b_pcm(chip, 0, 4, 0, 0x100);
        key_ymf278b_pcm(chip, 0, 0x80 | pan.parse::<u8>().unwrap());
        return generate_6_ymf278b(chip, samples);
    }
    if let Some(mix) = name.strip_prefix("MIX_") {
        let mix: u8 = mix.parse().unwrap();
        setup_ymf278b_fm_tone(chip, 0, 0, 0, 0x30);
        key_ymf278b_fm(chip, 0, true);
        play_ymf278b_pcm(chip, 0, 0, 0, 0x200);
        write_pcm_ymf278b(chip, 0xF8, mix | ((7 - mix) << 3));
        write_pcm_ymf278b(chip, 0xF9, (7 - mix) | (mix << 3));
        return generate_6_ymf278b(chip, samples);
    }
    match name {
        "PCM_8BIT" => play_ymf278b_pcm(chip, 0, 0, 0, 0),
        "PCM_12BIT" => play_ymf278b_pcm(chip, 1, 1, 0, 0x080),
        "PCM_16BIT" => play_ymf278b_pcm(chip, 2, 2, 0, 0x100),
        "PCM_FORMAT_3" => play_ymf278b_pcm(chip, 3, 6, 1, 0),
        "PCM_SHORT_LOOP" => play_ymf278b_pcm(chip, 4, 4, 1, 0x155),
        "PCM_ODD_LOOP" => play_ymf278b_pcm(chip, 5, 5, 2, 0x2AA),
        "PCM_OCTAVE_MINUS_8" => play_ymf278b_pcm(chip, 6, 4, -8, 0x3FF),
        "PCM_OCTAVE_MINUS_3" => play_ymf278b_pcm(chip, 7, 4, -3, 0x123),
        "PCM_OCTAVE_PLUS_2" => play_ymf278b_pcm(chip, 8, 4, 2, 0x321),
        "PCM_OCTAVE_PLUS_7" => play_ymf278b_pcm(chip, 9, 4, 7, 0x3FF),
        "PCM_FNUMBER" => play_ymf278b_pcm(chip, 10, 0, 0, 0x155),
        "PCM_OUTPUT_CHANNEL" => {
            load_ymf278b_pcm(chip, 11, 2, 0, 0x100);
            key_ymf278b_pcm(chip, 11, 0x93);
        }
        "PCM_ATTACK_8" | "PCM_ATTACK_12" | "PCM_ATTACK_14" => {
            let rate: u8 = name.strip_prefix("PCM_ATTACK_").unwrap().parse().unwrap();
            load_ymf278b_pcm(chip, 12, 0, 0, 0x200);
            set_ymf278b_pcm_envelope(chip, 12, rate << 4, 0x00, 0xF7);
            key_ymf278b_pcm(chip, 12, 0x80);
        }
        "PCM_DECAY_SUSTAIN" => {
            load_ymf278b_pcm(chip, 13, 2, 0, 0x100);
            set_ymf278b_pcm_envelope(chip, 13, 0xFC, 0x3B, 0xF7);
            key_ymf278b_pcm(chip, 13, 0x80);
        }
        "PCM_RELEASE" => {
            load_ymf278b_pcm(chip, 14, 2, 0, 0x100);
            set_ymf278b_pcm_envelope(chip, 14, 0xF0, 0x00, 0xFC);
            key_ymf278b_pcm(chip, 14, 0x80);
            let mut result = generate_6_ymf278b(chip, 64);
            key_ymf278b_pcm(chip, 14, 0x00);
            result.extend(generate_6_ymf278b(chip, samples - 64));
            return result;
        }
        "PCM_DAMP" => {
            load_ymf278b_pcm(chip, 15, 2, 0, 0x100);
            key_ymf278b_pcm(chip, 15, 0x80);
            let mut result = generate_6_ymf278b(chip, 48);
            key_ymf278b_pcm(chip, 15, 0xC0);
            result.extend(generate_6_ymf278b(chip, samples - 48));
            return result;
        }
        "PCM_REVERB" => {
            load_ymf278b_pcm(chip, 16, 2, 0, 0x100);
            write_pcm_ymf278b(chip, 0x38 + 16, 0x08 | 0x02);
            set_ymf278b_pcm_envelope(chip, 16, 0xFD, 0xF0, 0xFD);
            key_ymf278b_pcm(chip, 16, 0x80);
        }
        "PCM_RATE_CORRECTION" => {
            load_ymf278b_pcm(chip, 17, 4, 3, 0x280);
            set_ymf278b_pcm_envelope(chip, 17, 0xF9, 0xF6, 0x55);
            key_ymf278b_pcm(chip, 17, 0x80);
        }
        "PCM_LEVEL_DIRECT" => {
            play_ymf278b_pcm(chip, 18, 2, 0, 0x100);
            let mut result = generate_6_ymf278b(chip, 64);
            write_pcm_ymf278b(chip, 0x50 + 18, 0x41);
            result.extend(generate_6_ymf278b(chip, samples - 64));
            return result;
        }
        "PCM_LEVEL_INTERPOLATION" => {
            play_ymf278b_pcm(chip, 19, 2, 0, 0x100);
            let mut result = generate_6_ymf278b(chip, 32);
            write_pcm_ymf278b(chip, 0x50 + 19, 0x10);
            result.extend(generate_6_ymf278b(chip, 96));
            write_pcm_ymf278b(chip, 0x50 + 19, 0x00);
            result.extend(generate_6_ymf278b(chip, samples - 128));
            return result;
        }
        "PCM_VIBRATO" => {
            load_ymf278b_pcm(chip, 20, 0, 1, 0x100);
            write_pcm_ymf278b(chip, 0x80 + 20, 0x3F);
            key_ymf278b_pcm(chip, 20, 0x80);
        }
        "PCM_TREMOLO" => {
            load_ymf278b_pcm(chip, 21, 0, 1, 0x100);
            write_pcm_ymf278b(chip, 0x80 + 21, 0x38);
            write_pcm_ymf278b(chip, 0xE0 + 21, 0x07);
            key_ymf278b_pcm(chip, 21, 0x80);
        }
        "PCM_LFO_RESET" => {
            load_ymf278b_pcm(chip, 22, 0, 1, 0x100);
            write_pcm_ymf278b(chip, 0x80 + 22, 0x3D);
            write_pcm_ymf278b(chip, 0xE0 + 22, 0x05);
            key_ymf278b_pcm(chip, 22, 0x80);
            let mut result = generate_6_ymf278b(chip, 100);
            key_ymf278b_pcm(chip, 22, 0x00);
            result.extend(generate_6_ymf278b(chip, 4));
            key_ymf278b_pcm(chip, 22, 0xA0);
            result.extend(generate_6_ymf278b(chip, samples - 104));
            return result;
        }
        "PCM_HEADER_DEFAULTS" => {
            set_ymf278b_pcm_pitch(chip, 23, 3, 0, 0x200);
            write_pcm_ymf278b(chip, 0x50 + 23, 0x01);
            write_pcm_ymf278b(chip, 0x08 + 23, 3);
            key_ymf278b_pcm(chip, 23, 0x80);
            let mut result = generate_6_ymf278b(chip, 128);
            key_ymf278b_pcm(chip, 23, 0x00);
            result.extend(generate_6_ymf278b(chip, samples - 128));
            return result;
        }
        "PCM_BANK_ROM_HEADER" => play_ymf278b_pcm(chip, 0, 384, 0, 0x200),
        "PCM_BANK_RAM_HEADER" => {
            write_pcm_ymf278b(chip, 0x02, 0x12);
            play_ymf278b_pcm(chip, 1, 384, 0, 0x200);
        }
        "PCM_BANK_RAM_REFORMAT" => {
            write_pcm_ymf278b(chip, 0x02, 0x12);
            play_ymf278b_pcm(chip, 2, 385, 0, 0x100);
        }
        "PCM_RAM_SAMPLE" => play_ymf278b_pcm(chip, 3, 7, 0, 0x100),
        "PCM_MEMORY_UPLOAD" => {
            let data_address = YMF278B_RAM_BASE + YMF278B_RAM_UPLOAD_OFFSET + 0x100;
            let mut data = Vec::new();
            for index in 0..96 {
                data.push((triangle(index, 48, 8) as u8) ^ (index as u8 & 0x0F));
            }
            upload_ymf278b_memory(chip, data_address, &data);
            let mut header = [0u8; 12];
            write_ymf278b_header(
                &mut header,
                0,
                0,
                data_address,
                16,
                96,
                [0x00, 0xF0, 0x00, 0xF7, 0x00],
            );
            // wave 386 reads its header from bank 4 at offset 24
            upload_ymf278b_memory(chip, YMF278B_RAM_BASE + 24, &header);
            write_pcm_ymf278b(chip, 0x02, 0x12);
            play_ymf278b_pcm(chip, 4, 386, 0, 0x200);
        }
        "PCM_KEY_ON_OFF_PENDING" => {
            load_ymf278b_pcm(chip, 5, 3, 0, 0x100);
            key_ymf278b_pcm(chip, 5, 0x80);
            key_ymf278b_pcm(chip, 5, 0x00);
            let mut result = generate_6_ymf278b(chip, 96);
            key_ymf278b_pcm(chip, 5, 0x00);
            result.extend(generate_6_ymf278b(chip, 64));
            key_ymf278b_pcm(chip, 5, 0x80);
            key_ymf278b_pcm(chip, 5, 0x00);
            result.extend(generate_6_ymf278b(chip, samples - 160));
            return result;
        }
        "PCM_RETRIGGER" => {
            load_ymf278b_pcm(chip, 6, 2, 0, 0x100);
            set_ymf278b_pcm_envelope(chip, 6, 0xF0, 0x00, 0xFA);
            key_ymf278b_pcm(chip, 6, 0x80);
            let mut result = generate_6_ymf278b(chip, 80);
            key_ymf278b_pcm(chip, 6, 0x00);
            result.extend(generate_6_ymf278b(chip, 40));
            key_ymf278b_pcm(chip, 6, 0x80);
            result.extend(generate_6_ymf278b(chip, samples - 120));
            return result;
        }
        "PCM_WAVE_CHANGE" => {
            play_ymf278b_pcm(chip, 7, 0, 0, 0x100);
            let mut result = generate_6_ymf278b(chip, 100);
            write_pcm_ymf278b(chip, 0x08 + 7, 2);
            result.extend(generate_6_ymf278b(chip, samples - 100));
            return result;
        }
        "PCM_ALL_CHANNELS" => {
            for channel in 0..24u8 {
                let wave = [0u16, 1, 2, 3, 4, 5, 6, 7][channel as usize % 8];
                let octave = (channel % 5) as i8 - 2;
                load_ymf278b_pcm(chip, channel, wave, octave, u16::from(channel) * 37);
                write_pcm_ymf278b(chip, 0x50 + channel, 0x41);
                key_ymf278b_pcm(chip, channel, 0x80 | (channel % 16) | ((channel & 1) << 4));
            }
        }
        "PCM_AND_FM" => {
            setup_ymf278b_fm_tone(chip, 0, 0, 1, 0xF0);
            key_ymf278b_fm(chip, 0, true);
            play_ymf278b_pcm(chip, 0, 1, 0, 0x100);
            load_ymf278b_pcm(chip, 1, 2, 1, 0x040);
            key_ymf278b_pcm(chip, 1, 0x90);
        }
        "PCM_NEW2_OFF" => {
            write_reg_ymf278b(chip, 0x105, 0x01);
            setup_ymf278b_fm_tone(chip, 0, 0, 2, 0x30);
            key_ymf278b_fm(chip, 0, true);
            play_ymf278b_pcm(chip, 0, 2, 0, 0x100);
        }
        _ => panic!("unknown YMF278B PCM scenario {name}"),
    }
    generate_6_ymf278b(chip, samples)
}

/// Names of the long YMF278B scenarios stored as block checksums.
pub const YMF278B_LONG_SCENARIOS: &[&str] = &[
    "PCM_FULL_ENVELOPE",
    "PCM_REVERB_LONG",
    "PCM_DAMP_LONG",
    "PCM_LEVEL_SWEEP",
    "PCM_LFO_SPEEDS",
    "PCM_RATE_CORRECTIONS",
    "PCM_PREPARE_SWEEP",
    "FM_RESAMPLING",
];

/// Number of samples in each long YMF278B scenario.
pub const YMF278B_LONG_SCENARIO_SAMPLES: usize = 8192;

/// Number of samples hashed into each long scenario checksum.
pub const YMF278B_CHECKSUM_BLOCK: usize = 256;

/// Runs the named long YMF278B scenario and returns its samples.
pub fn ymf278b_long_scenario(name: &str) -> Vec<[i32; 6]> {
    let mut chip = setup_ymf278b();
    let chip = &mut chip;
    let samples = YMF278B_LONG_SCENARIO_SAMPLES;
    match name {
        "PCM_FULL_ENVELOPE" => {
            load_ymf278b_pcm(chip, 0, 4, 0, 0x100);
            set_ymf278b_pcm_envelope(chip, 0, 0x64, 0x83, 0xF5);
            key_ymf278b_pcm(chip, 0, 0x80);
            let mut result = generate_6_ymf278b(chip, samples / 2);
            key_ymf278b_pcm(chip, 0, 0x00);
            result.extend(generate_6_ymf278b(chip, samples / 2));
            return result;
        }
        "PCM_REVERB_LONG" => {
            load_ymf278b_pcm(chip, 1, 4, 0, 0x100);
            write_pcm_ymf278b(chip, 0x38 + 1, 0x08);
            set_ymf278b_pcm_envelope(chip, 1, 0xF4, 0x42, 0xF6);
            key_ymf278b_pcm(chip, 1, 0x80);
            let mut result = generate_6_ymf278b(chip, 2048);
            key_ymf278b_pcm(chip, 1, 0x00);
            result.extend(generate_6_ymf278b(chip, samples - 2048));
            return result;
        }
        "PCM_DAMP_LONG" => {
            load_ymf278b_pcm(chip, 2, 4, 0, 0x100);
            set_ymf278b_pcm_envelope(chip, 2, 0xF0, 0x00, 0xF2);
            key_ymf278b_pcm(chip, 2, 0x80);
            let mut result = generate_6_ymf278b(chip, 512);
            key_ymf278b_pcm(chip, 2, 0xC0);
            result.extend(generate_6_ymf278b(chip, 1024));
            key_ymf278b_pcm(chip, 2, 0x40);
            result.extend(generate_6_ymf278b(chip, 64));
            key_ymf278b_pcm(chip, 2, 0x80);
            result.extend(generate_6_ymf278b(chip, 2496));
            key_ymf278b_pcm(chip, 2, 0x00);
            result.extend(generate_6_ymf278b(chip, samples - 4096));
            return result;
        }
        "PCM_LEVEL_SWEEP" => {
            load_ymf278b_pcm(chip, 3, 4, 0, 0x100);
            write_pcm_ymf278b(chip, 0x50 + 3, 0x00);
            key_ymf278b_pcm(chip, 3, 0x80);
            let mut result = generate_6_ymf278b(chip, 4096);
            write_pcm_ymf278b(chip, 0x50 + 3, 0xFE);
            result.extend(generate_6_ymf278b(chip, samples - 4096));
            return result;
        }
        "PCM_LFO_SPEEDS" => {
            for channel in 0..8u8 {
                load_ymf278b_pcm(chip, channel, 0, 0, 0x080 + u16::from(channel) * 0x40);
                write_pcm_ymf278b(chip, 0x50 + channel, 0x21);
                write_pcm_ymf278b(chip, 0x80 + channel, (channel << 3) | (7 - channel));
                write_pcm_ymf278b(chip, 0xE0 + channel, channel);
                key_ymf278b_pcm(
                    chip,
                    channel,
                    0x80 | if channel < 4 { channel } else { 16 - channel },
                );
            }
        }
        "PCM_RATE_CORRECTIONS" => {
            for channel in 0..16u8 {
                let octave = (channel % 8) as i8 - 4;
                load_ymf278b_pcm(chip, channel, 4, octave, 0x3FF - u16::from(channel) * 0x40);
                write_pcm_ymf278b(chip, 0x50 + channel, 0x31);
                set_ymf278b_pcm_envelope(chip, channel, 0x86, 0x74, (channel << 4) | 0x07);
                key_ymf278b_pcm(chip, channel, 0x80 | ((channel & 1) << 4));
            }
            let mut result = generate_6_ymf278b(chip, 4096);
            for channel in 0..16u8 {
                key_ymf278b_pcm(chip, channel, (channel & 1) << 4);
            }
            result.extend(generate_6_ymf278b(chip, samples - 4096));
            return result;
        }
        "PCM_PREPARE_SWEEP" => {
            load_ymf278b_pcm(chip, 4, 2, 0, 0x100);
            set_ymf278b_pcm_envelope(chip, 4, 0xF3, 0x43, 0xF4);
            key_ymf278b_pcm(chip, 4, 0x80);
            key_ymf278b_pcm(chip, 4, 0x00);
            let mut result = Vec::new();
            for _ in 0..samples / 128 {
                result.extend(generate_6_ymf278b(chip, 128));
            }
            return result;
        }
        "FM_RESAMPLING" => {
            setup_ymf278b_fm_tone(chip, 0, 0, 4, 0xF0);
            write_reg_ymf278b(chip, 0xBD, 0xC0);
            for op in 0..2u8 {
                write_reg_ymf278b(chip, 0x20 + opl_op_offset(0, op) as u16, 0xE1);
            }
            key_ymf278b_fm(chip, 0, true);
            play_ymf278b_pcm(chip, 0, 4, 0, 0x100);
        }
        _ => panic!("unknown long YMF278B scenario {name}"),
    }
    generate_6_ymf278b(chip, samples)
}

/// Number of fuzz seeds for the YMF278B.
pub const YMF278B_FUZZ_SEEDS: u32 = 8;

/// Number of blocks in each YMF278B fuzz run.
pub const YMF278B_FUZZ_BLOCKS: usize = 16;

/// Number of samples in each YMF278B fuzz block.
pub const YMF278B_FUZZ_BLOCK_SAMPLES: usize = 256;

/// Runs a YMF278B fuzz stream and returns one checksum per block.
///
/// The stream starts with FM tones on four channels and keyed PCM waves on
/// eight channels. Before each block it writes a random set of FM and PCM
/// registers with random sample gaps in between. Timer control and the
/// memory access registers are left out.
pub fn ymf278b_fuzz(seed: u32) -> Vec<u64> {
    let mut random = XorShift32::new(seed.wrapping_mul(0x9E37_79B9) | 1);
    let mut chip = setup_ymf278b();
    for channel in 0..4u8 {
        setup_ymf278b_fm_tone(
            &mut chip,
            channel * 4,
            channel & 1,
            channel,
            0x30 << (channel & 1),
        );
        key_ymf278b_fm(&mut chip, channel * 4, true);
    }
    for channel in 0..8u8 {
        load_ymf278b_pcm(&mut chip, channel * 3, u16::from(channel), 0, 0x100);
        write_pcm_ymf278b(&mut chip, 0x50 + channel * 3, 0x21);
        key_ymf278b_pcm(&mut chip, channel * 3, 0x80 | channel);
    }
    let mut checksums = Vec::new();
    for _ in 0..YMF278B_FUZZ_BLOCKS {
        let mut samples = Vec::new();
        let writes = 1 + random.below(12);
        for _ in 0..writes {
            match random.below(8) {
                0 => {
                    let address = 0x20 + random.below(0xE0) as u16;
                    write_reg_ymf278b(&mut chip, address, random.next_u32() as u8);
                }
                1 => {
                    let address = 0x120 + random.below(0xE0) as u16;
                    write_reg_ymf278b(&mut chip, address, random.next_u32() as u8);
                }
                2 => {
                    let channel = random.below(24) as u8;
                    let wave =
                        [0u16, 1, 2, 3, 4, 5, 6, 7, 384, 385, 100][random.below(11) as usize];
                    set_ymf278b_pcm_pitch(&mut chip, channel, wave, 0, random.below(0x400) as u16);
                    write_pcm_ymf278b(&mut chip, 0x08 + channel, wave as u8);
                }
                3 => {
                    let channel = random.below(24) as u8;
                    key_ymf278b_pcm(&mut chip, channel, random.next_u32() as u8);
                }
                4 => {
                    let data = (random.next_u32() as u8) & 0x1E;
                    write_pcm_ymf278b(&mut chip, 0x02, data);
                }
                5 => {
                    let register = [0xF8, 0xF9][random.below(2) as usize];
                    write_pcm_ymf278b(&mut chip, register, random.next_u32() as u8);
                }
                _ => {
                    let register = 0x20 + random.below(0xD8) as u8;
                    write_pcm_ymf278b(&mut chip, register, random.next_u32() as u8);
                }
            }
            if random.below(4) == 0 {
                let gap =
                    (random.below(24) as usize).min(YMF278B_FUZZ_BLOCK_SAMPLES - samples.len());
                samples.extend(generate_6_ymf278b(&mut chip, gap));
            }
        }
        let remaining = YMF278B_FUZZ_BLOCK_SAMPLES - samples.len();
        samples.extend(generate_6_ymf278b(&mut chip, remaining));
        checksums.push(fnv1a_samples(&samples));
    }
    checksums
}
