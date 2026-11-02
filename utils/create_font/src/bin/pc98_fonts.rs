//! Builds the PC-98 font ROM from the `.bit` files of the PC-98 fonts.

use std::path::PathBuf;

use create_font::pc98_fonts::build_font_rom;

/// Parsed command line.
struct Args {
    fonts: PathBuf,
    font_rom: PathBuf,
}

/// Prints the command line help.
fn print_help() {
    println!(
        "\
pc98_fonts - Build the PC-98 font ROM from .bit files

Usage: pc98_fonts --fonts <DIR> --font-rom <PATH>

Options:
      --fonts <DIR>       Directory of the PC-98 .bit files
      --font-rom <PATH>   Output font ROM (288768 bytes)
  -h, --help              Print help"
    );
}

/// Parses the command line.
fn parse_args() -> Args {
    let mut fonts: Option<PathBuf> = None;
    let mut font_rom: Option<PathBuf> = None;
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
            other => panic!("unknown argument: {other}"),
        }
    }
    let (Some(fonts), Some(font_rom)) = (fonts, font_rom) else {
        eprintln!("error: --fonts and --font-rom are required");
        print_help();
        std::process::exit(1);
    };
    Args { fonts, font_rom }
}

/// Builds the font ROM and writes it.
fn main() {
    let args = parse_args();
    let data = build_font_rom(&args.fonts).unwrap_or_else(|error| panic!("{error}"));
    std::fs::write(&args.font_rom, &data)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", args.font_rom.display()));
    eprintln!("Built {} ({} bytes)", args.font_rom.display(), data.len());
}
