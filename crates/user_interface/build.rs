//! Generates the keys and the schema of the texts of the user interface from the reference
//! language file `language/en.ini`.

use std::{env, path::PathBuf, process};

/// The reference language file relative to the folder of the crate.
const REFERENCE_FILE: &str = "../../language/en.ini";

/// The file name of the generated code in the output folder.
const GENERATED_FILE: &str = "texts.rs";

/// Writes the generated texts module into `OUT_DIR`.
fn main() {
    let manifest_folder =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let output_folder = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let reference = manifest_folder.join(REFERENCE_FILE);
    println!("cargo:rerun-if-changed={}", reference.display());
    if let Err(report) =
        i18n::generate::generate_file(&reference, &output_folder.join(GENERATED_FILE))
    {
        eprintln!("{}", report.trim_end());
        process::exit(1);
    }
}
