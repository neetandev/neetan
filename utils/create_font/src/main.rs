// TODO Rework this file to also use the src/bin/ and /lib.rs pattern so we align the PC-908 and PC/AT paths.
use std::path::{Path, PathBuf};

use create_font::bdf::BitmapFont;

const V98_SIZE: usize = 0x46800;

const JIS7883: [(u16, u16); 4] = [
    (0x3646, 0x7421),
    (0x4B6A, 0x7422),
    (0x4D5A, 0x7423),
    (0x596A, 0x7424),
];

const JIS8390: [(u16, u16); 22] = [
    (0x724D, 0x3033),
    (0x7274, 0x3229),
    (0x695A, 0x3342),
    (0x5978, 0x3349),
    (0x635E, 0x3376),
    (0x5E75, 0x3443),
    (0x6B5D, 0x3452),
    (0x7074, 0x375B),
    (0x6268, 0x395C),
    (0x6922, 0x3C49),
    (0x7057, 0x3F59),
    (0x6C4D, 0x4128),
    (0x5464, 0x445B),
    (0x626A, 0x4557),
    (0x5B6D, 0x456E),
    (0x5E39, 0x4573),
    (0x6D6E, 0x4676),
    (0x6A24, 0x4768),
    (0x5B58, 0x4930),
    (0x5056, 0x4B79),
    (0x692E, 0x4C79),
    (0x6446, 0x4F36),
];

const DELTABLE_22_TO_2D: [&[(u8, u8)]; 12] = [
    &[(0x0F, 0x5F)],
    &[(0x01, 0x10), (0x1A, 0x21), (0x3B, 0x41), (0x5B, 0x5F)],
    &[(0x54, 0x5F)],
    &[(0x57, 0x5F)],
    &[(0x19, 0x21), (0x39, 0x5F)],
    &[(0x22, 0x31), (0x52, 0x5F)],
    &[(0x01, 0x5F)],
    &[(0x01, 0x5F)],
    &[(0x01, 0x5F)],
    &[(0x01, 0x5F)],
    &[(0x01, 0x5F)],
    &[(0x1F, 0x20), (0x37, 0x3F), (0x5D, 0x5F)],
];

fn main() {
    let args = parse_args();

    eprintln!("Loading BDF fonts...");
    let kanji_source = std::fs::read_to_string(&args.kanji_bdf).expect("Failed to read kanji BDF");
    let kanji_font = BitmapFont::parse(&kanji_source);

    let ank_source = std::fs::read_to_string(&args.ank_bdf).expect("Failed to read ANK BDF");
    let mut ank_font = BitmapFont::parse(&ank_source);

    eprintln!("Loading patch fonts...");
    let ank_8x8_font = load_patch_font(&args.patch_dir, "ank_8x8.bit");
    let ank_ctrl_font = load_patch_font(&args.patch_dir, "ank_8x16_ctrl.bit");
    ank_font.merge(&ank_ctrl_font);

    let kanji_29_font = load_patch_font(&args.patch_dir, "kanji_hankaku_29.bit");
    let kanji_2a_font = load_patch_font(&args.patch_dir, "kanji_hankaku_2a.bit");
    let kanji_2b_font = load_patch_font(&args.patch_dir, "kanji_hankaku_2b.bit");
    let kanji_2c_font = load_patch_font(&args.patch_dir, "kanji_fullwidth_2c.bit");

    let mut v98 = vec![0u8; V98_SIZE];

    eprintln!("Generating 8x8 ANK...");
    generate_ank_8x8(&mut v98, &ank_8x8_font);

    eprintln!("Generating 8x16 ANK...");
    generate_ank_8x16(&mut v98, &ank_font);

    eprintln!("Generating 16x16 kanji...");
    generate_kanji_16x16(
        &mut v98,
        &kanji_font,
        &kanji_29_font,
        &kanji_2a_font,
        &kanji_2b_font,
        &kanji_2c_font,
    );

    std::fs::write(&args.output, &v98).expect("Failed to write output file");
    eprintln!(
        "Generated V98 font ROM: {} ({} bytes)",
        args.output.display(),
        V98_SIZE
    );
}

fn load_patch_font(patch_dir: &Path, filename: &str) -> BitmapFont {
    let path = patch_dir.join(filename);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
    BitmapFont::parse(&source)
}

fn generate_ank_8x8(v98: &mut [u8], ank_8x8_font: &BitmapFont) {
    for code in 0u32..256 {
        let offset = code as usize * 8;
        if let Some(glyph) = ank_8x8_font.get_8x8(code) {
            v98[offset..offset + 8].copy_from_slice(&glyph);
        }
    }
}

fn generate_ank_8x16(v98: &mut [u8], ank_font: &BitmapFont) {
    for code in 0u16..256 {
        let offset = if code < 0x80 {
            0x0800 + code as usize * 16
        } else {
            0x1000 + (code as usize - 0x80) * 16
        };

        if let Some(glyph) = ank_font.get_8x16(code as u32) {
            v98[offset..offset + 16].copy_from_slice(&glyph);
        }
    }
}

fn kanji_v98_offset(code_row: u8, col: u8) -> usize {
    0x1800 + 0x60 * 32 * (code_row as usize - 1) + (col as usize - 0x20) * 32
}

fn write_kanji_v98(v98: &mut [u8], code_row: u8, col: u8, bitmap: &[u8]) {
    let offset = kanji_v98_offset(code_row, col);
    if offset + 32 > v98.len() {
        return;
    }
    if bitmap.len() == 32 {
        v98[offset..offset + 32].copy_from_slice(bitmap);
    } else if bitmap.len() == 16 {
        v98[offset..offset + 16].copy_from_slice(bitmap);
    }
}

fn convert_jis_with_np21_tables(jis: u16) -> u16 {
    fn convert_jis_once(jis: u16, table: &[(u16, u16)]) -> u16 {
        for &(lhs, rhs) in table {
            if jis == lhs {
                return rhs;
            }
            if jis == rhs {
                return lhs;
            }
        }
        jis
    }

    let jis = convert_jis_once(jis, &JIS7883);
    convert_jis_once(jis, &JIS8390)
}

fn is_pc98_jis(jis: u16) -> bool {
    let row = (jis >> 8) as u8;
    let col = jis as u8;

    match row {
        0x22..=0x2D => {
            let ranges = DELTABLE_22_TO_2D[(row - 0x22) as usize];
            let column_offset = col.wrapping_sub(0x20);
            for &(start, end) in ranges {
                if column_offset >= start && column_offset < end {
                    return false;
                }
            }
            true
        }
        0x4F => col < 0x54,
        0x7C => col != 0x6F && col != 0x70,
        0x2E | 0x2F | 0x74 | 0x75 | 0x76 | 0x77 | 0x78 | 0x7D | 0x7E | 0x7F => false,
        _ => true,
    }
}

fn patch_hankaku_row(v98: &mut [u8], code_row: u8, jis_row: u8, font: &BitmapFont) {
    for col in 0x21u8..=0x7E {
        let encoding = ((jis_row as u32) << 8) | col as u32;
        if let Some(glyph) = font.get_8x16(encoding) {
            write_kanji_v98(v98, code_row, col, &glyph);
        }
    }
}

fn patch_fullwidth_row(v98: &mut [u8], code_row: u8, jis_row: u8, font: &BitmapFont) {
    for col in 0x21u8..=0x7E {
        let encoding = ((jis_row as u32) << 8) | col as u32;
        if let Some(mut glyph) = font.get_16x16(encoding) {
            swap_fullwidth_halves(&mut glyph);
            write_kanji_v98(v98, code_row, col, &glyph);
        }
    }
}

/// Swaps the two 8-pixel halves used by the NP21W fullwidth patch data.
fn swap_fullwidth_halves(glyph: &mut [u8; 32]) {
    for row in 0..16 {
        glyph.swap(row, 16 + row);
    }
}

fn generate_kanji_16x16(
    v98: &mut [u8],
    kanji_font: &BitmapFont,
    kanji_29_font: &BitmapFont,
    kanji_2a_font: &BitmapFont,
    kanji_2b_font: &BitmapFont,
    kanji_2c_font: &BitmapFont,
) {
    for jis_row in 0x21u8..=0x7E {
        let code_row = jis_row - 0x20;
        for col in 0x21u8..=0x7E {
            let jis = ((jis_row as u16) << 8) | col as u16;
            if !is_pc98_jis(jis) {
                continue;
            }
            let converted_jis = convert_jis_with_np21_tables(jis);
            if let Some(glyph) = kanji_font.get_16x16(converted_jis as u32) {
                write_kanji_v98(v98, code_row, col, &glyph);
            }
        }
    }

    patch_hankaku_row(v98, 0x0B, 0x2B, kanji_2b_font);
    patch_fullwidth_row(v98, 0x0C, 0x2C, kanji_2c_font);
    patch_hankaku_row(v98, 0x09, 0x29, kanji_29_font);
    patch_hankaku_row(v98, 0x0A, 0x2A, kanji_2a_font);
}

struct Args {
    output: PathBuf,
    kanji_bdf: PathBuf,
    ank_bdf: PathBuf,
    patch_dir: PathBuf,
}

fn next_value(flag: &str, args: &mut impl Iterator<Item = String>) -> String {
    args.next()
        .unwrap_or_else(|| panic!("missing value for {flag}"))
}

fn print_help() {
    println!(
        "\
create_font - Generate a V98-format font ROM from Shinonome bitmap fonts

Usage: create_font [OPTIONS] -o <PATH>

Options:
  -o, --output <PATH>       Output V98 font ROM path (required)
      --kanji-bdf <PATH>    Path to 16x16 kanji BDF font (.bit) [default: utils/bios/fonts/shinonome/kanjic16.bit]
      --ank-bdf <PATH>      Path to 8x16 ANK BDF font (.bit) [default: utils/bios/fonts/shinonome/latin1_8x16.bit]
      --patch-dir <PATH>    Directory containing BDF patch files [default: utils/bios/fonts/patches]
  -h, --help                Print help"
    );
}

fn parse_args() -> Args {
    let mut output: Option<PathBuf> = None;
    let mut kanji_bdf = PathBuf::from("utils/bios/fonts/shinonome/kanjic16.bit");
    let mut ank_bdf = PathBuf::from("utils/bios/fonts/shinonome/latin1_8x16.bit");
    let mut patch_dir = PathBuf::from("utils/bios/fonts/patches");

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let (flag, inline_value) = match arg.split_once('=') {
            Some((f, v)) => (f.to_owned(), Some(v.to_owned())),
            None => (arg, None),
        };

        let value = inline_value.unwrap_or_else(|| next_value(&flag, &mut args));

        match flag.as_str() {
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "-o" | "--output" => output = Some(PathBuf::from(value)),
            "--kanji-bdf" => kanji_bdf = PathBuf::from(value),
            "--ank-bdf" => ank_bdf = PathBuf::from(value),
            "--patch-dir" => patch_dir = PathBuf::from(value),
            other => panic!("unknown argument: {other}"),
        }
    }

    let output = output.unwrap_or_else(|| {
        eprintln!("error: --output/-o is required");
        print_help();
        std::process::exit(1);
    });

    Args {
        output,
        kanji_bdf,
        ank_bdf,
        patch_dir,
    }
}
