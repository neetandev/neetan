//! Builds the FM Towns font ROM and the 12-dot font area of the HLE system ROM
//! from the `.bit` files of the FM Towns fonts.

use std::path::PathBuf;

use create_font::towns_fonts::{build_font_rom, build_font12};

/// Parsed command line.
struct Args {
    fonts: PathBuf,
    font_rom: Option<PathBuf>,
    font12: Option<PathBuf>,
}

/// Prints the command line help.
fn print_help() {
    println!(
        "\
towns_fonts - Build the FM Towns font ROM and the 12-dot font area from .bit files

Usage: towns_fonts --fonts <DIR> [--font-rom <PATH>] [--font12 <PATH>]

Options:
      --fonts <DIR>       Directory of the FM Towns .bit files
      --font-rom <PATH>   Output font ROM (262144 bytes)
      --font12 <PATH>     Output 12-dot font area (110592 bytes)
  -h, --help              Print help"
    );
}

/// Parses the command line.
fn parse_args() -> Args {
    let mut fonts: Option<PathBuf> = None;
    let mut font_rom: Option<PathBuf> = None;
    let mut font12: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .map(PathBuf::from)
                .unwrap_or_else(|| panic!("missing value for {arg}"))
        };
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "--fonts" => fonts = Some(value()),
            "--font-rom" => font_rom = Some(value()),
            "--font12" => font12 = Some(value()),
            other => panic!("unknown argument: {other}"),
        }
    }
    let Some(fonts) = fonts.filter(|_| font_rom.is_some() || font12.is_some()) else {
        eprintln!("error: --fonts and at least one output are required");
        print_help();
        std::process::exit(1);
    };
    Args {
        fonts,
        font_rom,
        font12,
    }
}

/// Builds the requested images and writes them.
fn main() {
    let args = parse_args();
    let outputs = [
        (
            args.font_rom,
            build_font_rom as fn(&std::path::Path) -> Result<Vec<u8>, String>,
        ),
        (args.font12, build_font12),
    ];
    for (path, build) in outputs {
        let Some(path) = path else {
            continue;
        };
        let data = build(&args.fonts).unwrap_or_else(|error| panic!("{error}"));
        std::fs::write(&path, &data)
            .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
        eprintln!("Built {} ({} bytes)", path.display(), data.len());
    }
}
