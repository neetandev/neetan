//! Renders the FM Towns system icon art into the raw icon area binary that the
//! HLE system ROM stub includes with `incbin`.

use std::path::PathBuf;

use create_font::towns_icons::parse_icons;

/// Prints the command line help.
fn print_help() {
    println!(
        "\
towns_icons - Render the FM Towns system icon art into a raw icon binary

Usage: towns_icons --input <PATH> --output <PATH>

Options:
  -i, --input <PATH>    Source icon art (128 icons of 32 x 32 pixels)
  -o, --output <PATH>   Output raw icon binary (32768 bytes)
  -h, --help            Print help"
    );
}

/// Parses the command line into the input and the output path.
fn parse_args() -> (PathBuf, PathBuf) {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "-i" | "--input" => input = args.next().map(PathBuf::from),
            "-o" | "--output" => output = args.next().map(PathBuf::from),
            other => panic!("unknown argument: {other}"),
        }
    }
    let (Some(input), Some(output)) = (input, output) else {
        eprintln!("error: --input and --output are required");
        print_help();
        std::process::exit(1);
    };
    (input, output)
}

/// Parses the arguments, renders the icon art and writes the binary.
fn main() {
    let (input, output) = parse_args();
    let source = std::fs::read_to_string(&input)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", input.display()));
    let data = parse_icons(&source).unwrap_or_else(|error| panic!("{}: {error}", input.display()));
    std::fs::write(&output, &data)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", output.display()));
    eprintln!("Rendered {} ({} bytes)", output.display(), data.len());
}
