use ymfm_oxide::{
    Y8950, Ym2151, Ym2203, Ym2413, Ym2608, Ym3526, Ym3812, Ymf262, Ymf276, YmfmOpnFidelity,
    YmfmOutput1, YmfmOutput2, YmfmOutput3, YmfmOutput4,
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

// --- OPN2 (YMF276) helpers ---

pub fn write_reg_ymf276(chip: &mut Ymf276, addr: u8, data: u8) {
    chip.write_address(addr);
    chip.write_data(data);
}

pub fn write_reg_ymf276_hi(chip: &mut Ymf276, addr: u8, data: u8) {
    chip.write_address_hi(addr);
    chip.write_data_hi(data);
}

pub fn generate_2(chip: &mut Ymf276, count: usize) -> Vec<[i32; 2]> {
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

pub fn setup_ymf276() -> Ymf276 {
    let mut chip = Ymf276::new();
    chip.reset();
    chip
}

/// A simple 4-operator tone on the given channel (0-5), routed to the correct
/// register bank (channels 0-2 low, 3-5 high).
pub fn setup_ymf276_simple_tone(chip: &mut Ymf276, channel: u8, algorithm: u8, feedback: u8) {
    let fb_algo = (feedback << 3) | (algorithm & 0x07);
    if channel < 3 {
        write_reg_ymf276(chip, 0xB0 + channel, fb_algo);
        for op_offset in [0x00, 0x04, 0x08, 0x0C] {
            let reg_base = channel + op_offset;
            write_reg_ymf276(chip, 0x30 + reg_base, 0x01);
            write_reg_ymf276(chip, 0x40 + reg_base, 0x00);
            write_reg_ymf276(chip, 0x50 + reg_base, 0x1F);
            write_reg_ymf276(chip, 0x60 + reg_base, 0x00);
            write_reg_ymf276(chip, 0x70 + reg_base, 0x00);
            write_reg_ymf276(chip, 0x80 + reg_base, 0x0F);
            write_reg_ymf276(chip, 0x90 + reg_base, 0x00);
        }
        write_reg_ymf276(chip, 0xA4 + channel, 0x22);
        write_reg_ymf276(chip, 0xA0 + channel, 0x69);
        write_reg_ymf276(chip, 0xB4 + channel, 0xC0);
    } else {
        let ch = channel - 3;
        write_reg_ymf276_hi(chip, 0xB0 + ch, fb_algo);
        for op_offset in [0x00, 0x04, 0x08, 0x0C] {
            let reg_base = ch + op_offset;
            write_reg_ymf276_hi(chip, 0x30 + reg_base, 0x01);
            write_reg_ymf276_hi(chip, 0x40 + reg_base, 0x00);
            write_reg_ymf276_hi(chip, 0x50 + reg_base, 0x1F);
            write_reg_ymf276_hi(chip, 0x60 + reg_base, 0x00);
            write_reg_ymf276_hi(chip, 0x70 + reg_base, 0x00);
            write_reg_ymf276_hi(chip, 0x80 + reg_base, 0x0F);
            write_reg_ymf276_hi(chip, 0x90 + reg_base, 0x00);
        }
        write_reg_ymf276_hi(chip, 0xA4 + ch, 0x22);
        write_reg_ymf276_hi(chip, 0xA0 + ch, 0x69);
        write_reg_ymf276_hi(chip, 0xB4 + ch, 0xC0);
    }
}

pub fn key_on_ymf276(chip: &mut Ymf276, channel: u8) {
    let ch_bits = if channel < 3 {
        channel
    } else {
        channel - 3 + 4
    };
    write_reg_ymf276(chip, 0x28, 0xF0 | ch_bits);
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

/// Writes one YM2413 register.
pub fn write_reg_ym2413(chip: &mut Ym2413, address: u8, value: u8) {
    chip.write_address(address);
    chip.write_data(value);
}

/// Configures and keys on one YM2413 melodic channel.
pub fn setup_ym2413_channel(
    chip: &mut Ym2413,
    channel: u8,
    instrument: u8,
    frequency_low: u8,
    control: u8,
    volume: u8,
) {
    write_reg_ym2413(chip, 0x30 + channel, (instrument << 4) | (volume & 0x0F));
    write_reg_ym2413(chip, 0x10 + channel, frequency_low);
    write_reg_ym2413(chip, 0x20 + channel, control);
}

/// Configures the three YM2413 rhythm channels.
pub fn setup_ym2413_rhythm(chip: &mut Ym2413) {
    setup_ym2413_channel(chip, 6, 0, 0x40, 0x15, 0);
    setup_ym2413_channel(chip, 7, 0, 0x60, 0x15, 0);
    setup_ym2413_channel(chip, 8, 0, 0x80, 0x15, 0);
    write_reg_ym2413(chip, 0x36, 0x00);
    write_reg_ym2413(chip, 0x37, 0x00);
    write_reg_ym2413(chip, 0x38, 0x00);
}

/// Generates native YM2413 melodic and rhythm samples.
pub fn generate_2_ym2413(chip: &mut Ym2413, count: usize) -> Vec<[i32; 2]> {
    let mut output = vec![YmfmOutput2 { data: [0; 2] }; count];
    chip.generate(&mut output);
    output.iter().map(|sample| sample.data).collect()
}

// --- OPM (YM2151) helpers ---
//
// These mirror the C++ golden generator (tests/cpp/gen_opm_golden.cpp) exactly;
// the sample sequences must stay in lockstep for the golden comparison.

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

pub fn setup_ym2151_simple_tone(chip: &mut Ym2151, channel: u8, algorithm: u8, feedback: u8) {
    let pan_fb_algo = 0xC0 | (feedback << 3) | (algorithm & 0x07);
    write_reg_ym2151(chip, 0x20 + channel, pan_fb_algo);
    for op in 0..4u8 {
        let off = opm_op_offset(channel, op);
        write_reg_ym2151(chip, 0x40 + off, 0x01); // DT1=0, MUL=1
        write_reg_ym2151(chip, 0x60 + off, 0x00); // TL=0
        write_reg_ym2151(chip, 0x80 + off, 0x1F); // KS=0, AR=31
        write_reg_ym2151(chip, 0xA0 + off, 0x00); // AMS-EN=0, D1R=0
        write_reg_ym2151(chip, 0xC0 + off, 0x00); // DT2=0, D2R=0
        write_reg_ym2151(chip, 0xE0 + off, 0x0F); // D1L=0, RR=15
    }
    write_reg_ym2151(chip, 0x28 + channel, 0x4A); // key code
    write_reg_ym2151(chip, 0x30 + channel, 0x00); // key fraction
}

pub fn key_on_ym2151(chip: &mut Ym2151, channel: u8) {
    write_reg_ym2151(chip, 0x08, 0x78 | (channel & 0x07));
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
