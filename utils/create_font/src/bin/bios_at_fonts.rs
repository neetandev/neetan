//! Renders a CP437 `.bit` font into the raw glyph binary (256 glyphs in code
//! order) that the PC/AT HLE VGA BIOS stub ROM includes with `incbin`.

use std::path::PathBuf;

use create_font::bdf::BitmapFont;

struct Args {
    input: PathBuf,
    output: PathBuf,
    height: u32,
}

fn print_help() {
    println!(
        "\
bios_at_fonts - Render a CP437 .bit font into a raw glyph binary

Usage: bios_at_fonts --input <PATH> --height <8|14|16> --output <PATH>

Options:
  -i, --input <PATH>    Source .bit font (256 CP437 glyphs)
      --height <ROWS>   Glyph cell height: 8, 14 or 16
  -o, --output <PATH>   Output raw glyph binary (256 * height bytes)
  -h, --help            Print help"
    );
}

fn next_value(flag: &str, args: &mut impl Iterator<Item = String>) -> String {
    args.next()
        .unwrap_or_else(|| panic!("missing value for {flag}"))
}

fn parse_args() -> Args {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut height: Option<u32> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let (flag, inline_value) = match arg.split_once('=') {
            Some((f, v)) => (f.to_owned(), Some(v.to_owned())),
            None => (arg, None),
        };

        if flag == "--help" || flag == "-h" {
            print_help();
            std::process::exit(0);
        }

        let value = inline_value.unwrap_or_else(|| next_value(&flag, &mut args));

        match flag.as_str() {
            "-i" | "--input" => input = Some(PathBuf::from(value)),
            "-o" | "--output" => output = Some(PathBuf::from(value)),
            "--height" => height = Some(value.parse().expect("height must be a number")),
            other => panic!("unknown argument: {other}"),
        }
    }

    let (Some(input), Some(output), Some(height)) = (input, output, height) else {
        eprintln!("error: --input, --height and --output are required");
        print_help();
        std::process::exit(1);
    };

    Args {
        input,
        output,
        height,
    }
}

fn main() {
    let args = parse_args();

    let source = std::fs::read_to_string(&args.input)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", args.input.display()));
    let font = BitmapFont::parse(&source);

    let mut data = Vec::with_capacity(256 * args.height as usize);
    for code in 0u32..256 {
        match args.height {
            8 => data.extend_from_slice(
                &font
                    .get_8x8(code)
                    .unwrap_or_else(|| panic!("missing 8x8 glyph {code}")),
            ),
            14 => data.extend_from_slice(
                &font
                    .get_8x14(code)
                    .unwrap_or_else(|| panic!("missing 8x14 glyph {code}")),
            ),
            16 => data.extend_from_slice(
                &font
                    .get_8x16(code)
                    .unwrap_or_else(|| panic!("missing 8x16 glyph {code}")),
            ),
            other => panic!("unsupported glyph height {other}"),
        }
    }

    std::fs::write(&args.output, &data)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", args.output.display()));
    eprintln!("Rendered {} ({} bytes)", args.output.display(), data.len());
}
