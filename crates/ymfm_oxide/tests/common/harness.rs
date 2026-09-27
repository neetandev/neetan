use ymfm_oxide::{
    OpllFamily, Opn2Family, Y8950, Ym2149, Ym2151, Ym2164, Ym2203, Ym2413, Ym2608, Ym2610Family,
    Ym3526, Ym3812, Ymf262, Ymf288, Ymf289b, YmfmOpnFidelity, YmfmOutput1, YmfmOutput2,
    YmfmOutput3, YmfmOutput4,
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
