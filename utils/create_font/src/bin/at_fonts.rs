//! Builds the raw font tables of the PC/AT HLE VGA BIOS and the HLE system
//! BIOS from the `.bit` files of the PC/AT fonts.

use std::path::PathBuf;

use create_font::at_fonts::{FontTable, build_table};

/// Command line flags of the output tables.
const TABLE_FLAGS: [(&str, FontTable); 6] = [
    ("--vga-8x8", FontTable::Vga8x8),
    ("--vga-8x14", FontTable::Vga8x14),
    ("--vga-8x16", FontTable::Vga8x16),
    ("--vga-9x14-alt", FontTable::Vga9x14Alternate),
    ("--vga-9x16-alt", FontTable::Vga9x16Alternate),
    ("--system-8x8", FontTable::System8x8),
];

/// Parsed command line.
struct Args {
    fonts: PathBuf,
    outputs: Vec<(FontTable, PathBuf)>,
}

/// Prints the command line help.
fn print_help() {
    println!(
        "\
at_fonts - Build the PC/AT HLE BIOS font tables from .bit files

Usage: at_fonts --fonts <DIR> [--vga-8x8 <PATH>] [--vga-8x14 <PATH>] ...

Options:
      --fonts <DIR>           Directory of the PC/AT .bit files
      --vga-8x8 <PATH>        Output 8x8 font of the VGA BIOS (2048 bytes)
      --vga-8x14 <PATH>       Output 8x14 font of the VGA BIOS (3584 bytes)
      --vga-8x16 <PATH>       Output 8x16 font of the VGA BIOS (4096 bytes)
      --vga-9x14-alt <PATH>   Output 9x14 alternate table of the VGA BIOS
      --vga-9x16-alt <PATH>   Output 9x16 alternate table of the VGA BIOS
      --system-8x8 <PATH>     Output 8x8 font of the system BIOS (1024 bytes)
  -h, --help                  Print help"
    );
}

/// Parses the command line.
fn parse_args() -> Args {
    let mut fonts: Option<PathBuf> = None;
    let mut outputs = Vec::new();
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
            flag => {
                let Some(&(_, table)) = TABLE_FLAGS.iter().find(|(name, _)| *name == flag) else {
                    panic!("unknown argument: {flag}");
                };
                outputs.push((table, value()));
            }
        }
    }
    let Some(fonts) = fonts.filter(|_| !outputs.is_empty()) else {
        eprintln!("error: --fonts and at least one output are required");
        print_help();
        std::process::exit(1);
    };
    Args { fonts, outputs }
}

/// Builds the requested tables and writes them.
fn main() {
    let args = parse_args();
    for (table, path) in args.outputs {
        let data = build_table(&args.fonts, table).unwrap_or_else(|error| panic!("{error}"));
        std::fs::write(&path, &data)
            .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
        eprintln!("Built {} ({} bytes)", path.display(), data.len());
    }
}
