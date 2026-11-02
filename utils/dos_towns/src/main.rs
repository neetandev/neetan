//! Writes the FM Towns HLE OS ROM image from the `MSDOS.SYS`, `COMMAND.COM`
//! and `MSCDEX.EXE` stubs.

use std::path::{Path, PathBuf};

use dos_towns::{build_os_rom, standard_files};

/// Paths of the command line.
struct Args {
    /// `MSDOS.SYS` stub.
    msdos_sys: PathBuf,
    /// `COMMAND.COM` stub.
    command_com: PathBuf,
    /// `MSCDEX.EXE` stub.
    mscdex_exe: PathBuf,
    /// Output image.
    output: PathBuf,
}

/// Prints the command line help.
fn print_help() {
    println!(
        "\
create_osrom - Write the FM Towns HLE OS ROM image

Usage: create_osrom --msdos <PATH> --command <PATH> --mscdex <PATH> --output <PATH>

Options:
      --msdos <PATH>    MSDOS.SYS stub (at most 7800h bytes)
      --command <PATH>  COMMAND.COM stub
      --mscdex <PATH>   MSCDEX.EXE stub
  -o, --output <PATH>   Output image (512 KiB)
  -h, --help            Print help"
    );
}

/// Parses the command line.
fn parse_args() -> Args {
    let mut msdos_sys = None;
    let mut command_com = None;
    let mut mscdex_exe = None;
    let mut output = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "--msdos" => msdos_sys = args.next().map(PathBuf::from),
            "--command" => command_com = args.next().map(PathBuf::from),
            "--mscdex" => mscdex_exe = args.next().map(PathBuf::from),
            "-o" | "--output" => output = args.next().map(PathBuf::from),
            other => panic!("unknown argument: {other}"),
        }
    }
    let (Some(msdos_sys), Some(command_com), Some(mscdex_exe), Some(output)) =
        (msdos_sys, command_com, mscdex_exe, output)
    else {
        eprintln!("error: --msdos, --command, --mscdex and --output are required");
        print_help();
        std::process::exit(1);
    };
    Args {
        msdos_sys,
        command_com,
        mscdex_exe,
        output,
    }
}

/// Reads a file or ends the program with its error.
fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

/// Parses the arguments, builds the image and writes it.
fn main() {
    let args = parse_args();
    let msdos_sys = read(&args.msdos_sys);
    let command_com = read(&args.command_com);
    let mscdex_exe = read(&args.mscdex_exe);
    let image = build_os_rom(&standard_files(&msdos_sys, &command_com, &mscdex_exe))
        .unwrap_or_else(|error| panic!("{error}"));
    std::fs::write(&args.output, &image)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", args.output.display()));
    eprintln!("Wrote {} ({} bytes)", args.output.display(), image.len());
}
