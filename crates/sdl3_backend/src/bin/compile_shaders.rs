use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Copy, Clone)]
enum ShaderTarget {
    Spirv,
    Dxil,
    Metallib,
}

impl ShaderTarget {
    fn name(self) -> &'static str {
        match self {
            ShaderTarget::Spirv => "SPIR-V (Vulkan)",
            ShaderTarget::Dxil => "DXIL (Direct3D)",
            ShaderTarget::Metallib => "Metal",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            ShaderTarget::Spirv => "spv",
            ShaderTarget::Dxil => "dxil",
            ShaderTarget::Metallib => "metallib",
        }
    }
}

const ALL_TARGETS: [ShaderTarget; 3] = [
    ShaderTarget::Spirv,
    ShaderTarget::Dxil,
    ShaderTarget::Metallib,
];

fn discover_pass_files(passes_dir: &Path) -> Vec<PathBuf> {
    let mut pass_files = Vec::new();

    if let Ok(entries) = fs::read_dir(passes_dir) {
        entries.flatten().for_each(|entry| {
            let path = entry.path();
            if path.is_dir()
                && let Ok(pass_entries) = fs::read_dir(&path)
            {
                pass_entries.flatten().for_each(|pass_entry| {
                    let pass_file = pass_entry.path();
                    if pass_file.is_file()
                        && pass_file.extension().is_some_and(|ext| ext == "slang")
                    {
                        pass_files.push(pass_file);
                    }
                });
            }
        });
    }

    pass_files
}

fn stage_info_for_pass(shader_file: &Path) -> (&'static str, &'static str) {
    let stem = shader_file.file_stem().unwrap().to_str().unwrap();

    if stem.ends_with(".vert") {
        ("vs_main", "vs_6_0")
    } else if stem.ends_with(".frag") {
        ("fs_main", "ps_6_0")
    } else {
        panic!(
            "unrecognized shader pass file '{}': expected .vert.slang or .frag.slang",
            shader_file.display()
        );
    }
}

fn spirv_command(shader_file: &Path, output_file: &Path, modules_dir: &Path) -> Command {
    let mut cmd = Command::new("slangc");
    cmd.arg("-target")
        .arg("spirv")
        .arg("-O3")
        .arg("-I")
        .arg(modules_dir)
        .arg("-profile")
        .arg("spirv_1_0")
        // Uses the entrypoint name from the source instead of 'main' in the spirv output.
        .arg("-fvk-use-entrypoint-name")
        .arg("-DTARGET_SPIRV=1")
        .arg("-o")
        .arg(output_file)
        .arg(shader_file);
    cmd
}

fn dxil_command(shader_file: &Path, output_file: &Path, modules_dir: &Path) -> Command {
    let (entry_point, profile) = stage_info_for_pass(shader_file);

    let mut cmd = Command::new("slangc");
    cmd.arg("-target")
        .arg("dxil")
        .arg("-O3")
        .arg("-I")
        .arg(modules_dir)
        .arg("-profile")
        .arg(profile)
        .arg("-entry")
        .arg(entry_point)
        .arg("-o")
        .arg(output_file)
        .arg(shader_file);
    cmd
}

fn metallib_command(shader_file: &Path, output_file: &Path, modules_dir: &Path) -> Command {
    let mut cmd = Command::new("slangc");
    cmd.arg("-target")
        .arg("metallib")
        .arg("-O3")
        .arg("-I")
        .arg(modules_dir)
        .arg("-o")
        .arg(output_file)
        .arg(shader_file);
    cmd
}

fn command_for(
    target: ShaderTarget,
    shader_file: &Path,
    output_file: &Path,
    modules_dir: &Path,
) -> Command {
    match target {
        ShaderTarget::Spirv => spirv_command(shader_file, output_file, modules_dir),
        ShaderTarget::Dxil => dxil_command(shader_file, output_file, modules_dir),
        ShaderTarget::Metallib => metallib_command(shader_file, output_file, modules_dir),
    }
}

/// Runs slangc, returning the captured stderr on failure.
fn run_slangc(mut cmd: Command) -> Result<(), String> {
    match cmd.output() {
        Ok(result) if result.status.success() => Ok(()),
        Ok(result) => Err(String::from_utf8_lossy(&result.stderr).into_owned()),
        Err(error) => Err(error.to_string()),
    }
}

/// Probes whether the local slangc toolchain can produce the given target by
/// compiling a trivial canary shader. Targets whose downstream compiler is
/// missing (for example dxc on a non-Windows host) report failure and are skipped.
fn probe_target(target: ShaderTarget, canary_file: &Path, output_dir: &Path) -> bool {
    let output_file = output_dir.join(format!("canary.{}", target.extension()));
    let modules_dir = canary_file.parent().unwrap();
    run_slangc(command_for(target, canary_file, &output_file, modules_dir)).is_ok()
}

fn main() {
    check_slangc_availability();

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let shader_dir = manifest_dir.join("shaders");
    let modules_dir = shader_dir.join("modules");
    let passes_dir = shader_dir.join("passes");

    let temp_dir = env::temp_dir().join("neetan_compile_shaders_probe");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).expect("failed to clear probe directory");
    }
    fs::create_dir_all(&temp_dir).expect("failed to create probe directory");

    let canary_file = temp_dir.join("canary.vert.slang");
    fs::write(
        &canary_file,
        "[shader(\"vertex\")] float4 vs_main() : SV_Position { return float4(0, 0, 0, 1); }\n",
    )
    .expect("failed to write canary shader");

    let available_targets: Vec<ShaderTarget> = ALL_TARGETS
        .into_iter()
        .filter(|target| {
            let supported = probe_target(*target, &canary_file, &temp_dir);
            if supported {
                println!("Toolchain supports {}: will (re)build", target.name());
            } else {
                println!(
                    "Toolchain cannot build {}: skipping (keeping any committed artifact)",
                    target.name()
                );
            }
            supported
        })
        .collect();

    fs::remove_dir_all(&temp_dir).ok();

    if available_targets.is_empty() {
        eprintln!("Error: the local slangc toolchain cannot produce any shader format.");
        std::process::exit(1);
    }

    let pass_files = discover_pass_files(&passes_dir);
    if pass_files.is_empty() {
        eprintln!(
            "Error: no shader pass files found under {}",
            passes_dir.display()
        );
        std::process::exit(1);
    }

    let mut written = Vec::new();

    for pass_file in &pass_files {
        let base_name = pass_file.file_stem().unwrap().to_str().unwrap();

        for target in &available_targets {
            let output_file = shader_dir.join(format!("{base_name}.{}", target.extension()));
            if let Err(error) =
                run_slangc(command_for(*target, pass_file, &output_file, &modules_dir))
            {
                eprintln!(
                    "Error: failed to compile {} for {}:\n{error}",
                    pass_file.display(),
                    target.name()
                );
                std::process::exit(1);
            }
            written.push(output_file);
        }
    }

    println!("\nWrote {} compiled shader artifact(s):", written.len());
    for path in &written {
        println!("  {}", path.display());
    }

    let skipped: Vec<&str> = ALL_TARGETS
        .into_iter()
        .filter(|target| {
            !available_targets
                .iter()
                .any(|available| available.extension() == target.extension())
        })
        .map(|target| target.name())
        .collect();
    if !skipped.is_empty() {
        println!(
            "\nSkipped formats (regenerate these on a host with the matching toolchain): {}",
            skipped.join(", ")
        );
    }
}

fn check_slangc_availability() {
    const MIN_YEAR: u32 = 2026;
    const MIN_MAJOR: u32 = 4;

    let result = Command::new("slangc").arg("-version").output();

    match result {
        Ok(output) => {
            if !output.status.success() {
                eprintln!("Error: slangc is installed but failed to report version");
                std::process::exit(1);
            }

            // `slangc` currently outputs all diagnostics messages to `stderr`, including the version.
            let version_output = String::from_utf8_lossy(&output.stderr);

            match parse_slangc_version(&version_output) {
                Some((year, major)) => {
                    let is_valid = year > MIN_YEAR
                        || (year == MIN_YEAR && major > MIN_MAJOR)
                        || (year == MIN_YEAR && major == MIN_MAJOR);

                    if !is_valid {
                        eprintln!("Error: slangc version {year}.{major} is too old.");
                        eprintln!(
                            "At least version {MIN_YEAR}.{MIN_MAJOR} is required to compile shaders."
                        );
                        eprintln!(
                            "Please update slangc by downloading slang directly from https://github.com/shader-slang/slang/releases"
                        );
                        std::process::exit(1);
                    }
                }
                None => {
                    eprintln!("Warning: Failed to parse slangc version from output:");
                    eprintln!("{version_output}");
                    eprintln!(
                        "Proceeding anyway, but at least version {MIN_YEAR}.{MIN_MAJOR} is required.",
                    );
                }
            }
        }
        Err(_) => {
            eprintln!("Error: slangc is not available in PATH.");
            eprintln!(
                "slangc is required to compile shaders. You can install it by downloading slang directly from https://github.com/shader-slang/slang/releases"
            );
            eprintln!("After installation, ensure slangc is in your PATH.");
            eprintln!("At least version {MIN_YEAR}.{MIN_MAJOR} is needed to compile shaders.",);
            std::process::exit(1);
        }
    }
}

fn parse_slangc_version(version_output: &str) -> Option<(u32, u32)> {
    // On Nix, the version is formatted as `vYEAR.MAJOR.MINOR-nixpkgs`, so we
    // sanitize the input to be in the format `YEAR.MAJOR`.
    let sanitized_output: String = version_output
        .chars()
        .filter(|character| character.is_ascii_digit() || *character == '.')
        .collect();

    let parts: Vec<&str> = sanitized_output.split('.').collect();

    if parts.len() < 2 {
        return None;
    }

    let first = parts[0].parse::<u32>().ok()?;
    let second = parts[1].parse::<u32>().ok()?;

    Some((first, second))
}
