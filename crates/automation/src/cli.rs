//! Hand-rolled command-line parsing for `neetan-auto`.
//!
//! Options accept both `--flag value` and `--flag=value`. Everything after
//! `--` on the `run` command is passed to the script.

use std::path::PathBuf;

use crate::config::GuestDateTime;

/// The parsed top-level command.
#[derive(Clone, Debug)]
pub enum Command {
    /// Run a single script.
    Run(RunArgs),
    /// Run every Scheme test beneath a directory.
    Orchestrate(OrchestrateArgs),
    /// Print help text.
    Help,
    /// Print version text.
    Version,
}

/// Arguments for the `orchestrate` subcommand.
#[derive(Clone, Debug)]
pub struct OrchestrateArgs {
    /// The directory tree containing Scheme tests.
    pub directory: PathBuf,
    /// The `--config` common-settings file.
    pub config: Option<PathBuf>,
    /// The `--global-config` common-settings file.
    pub global_config: Option<PathBuf>,
    /// The `--artifacts` root override.
    pub artifacts: Option<PathBuf>,
    /// The per-script `--timeout` override in seconds.
    pub timeout: Option<u64>,
    /// The `--guest-time` override.
    pub guest_time: Option<GuestDateTime>,
    /// Maximum concurrently active executor threads.
    pub jobs: usize,
}

/// Arguments for the `run` subcommand.
#[derive(Clone, Debug, Default)]
pub struct RunArgs {
    /// The script to run.
    pub script: PathBuf,
    /// The `--config` common-settings file.
    pub config: Option<PathBuf>,
    /// The `--global-config` common-settings file.
    pub global_config: Option<PathBuf>,
    /// The `--artifacts` root override.
    pub artifacts: Option<PathBuf>,
    /// The `--timeout` override in seconds.
    pub timeout: Option<u64>,
    /// The `--guest-time` override.
    pub guest_time: Option<GuestDateTime>,
    /// Arguments after `--`, forwarded to the script.
    pub script_args: Vec<String>,
}

/// Parses the command line, excluding the program name.
pub fn parse(arguments: Vec<String>) -> Result<Command, String> {
    let mut iterator = arguments.into_iter().peekable();
    let Some(first) = iterator.next() else {
        return Ok(Command::Help);
    };
    match first.as_str() {
        "--help" | "-h" | "help" => Ok(Command::Help),
        "--version" | "-V" => Ok(Command::Version),
        "run" => parse_run(iterator.collect()),
        "orchestrate" => parse_orchestrate(iterator.collect()),
        other => Err(format!("unknown subcommand '{other}'")),
    }
}

fn parse_orchestrate(arguments: Vec<String>) -> Result<Command, String> {
    let mut directory: Option<PathBuf> = None;
    let mut config = None;
    let mut global_config = None;
    let mut artifacts = None;
    let mut timeout = None;
    let mut guest_time = None;
    let mut jobs = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        if argument == "--" {
            return Err("the 'orchestrate' command does not accept script arguments".to_owned());
        }
        if let Some((flag, inline)) = split_flag(argument) {
            match flag {
                "--help" | "-h" => return Ok(Command::Help),
                "--version" | "-V" => return Ok(Command::Version),
                "--config" => {
                    config = Some(PathBuf::from(take_value(
                        flag, inline, &arguments, &mut index,
                    )?));
                }
                "--global-config" => {
                    global_config = Some(PathBuf::from(take_value(
                        flag, inline, &arguments, &mut index,
                    )?));
                }
                "--artifacts" => {
                    artifacts = Some(PathBuf::from(take_value(
                        flag, inline, &arguments, &mut index,
                    )?));
                }
                "--timeout" => {
                    let value = take_value(flag, inline, &arguments, &mut index)?;
                    timeout =
                        Some(value.parse::<u64>().map_err(|_| {
                            format!("invalid --timeout (expected seconds): {value}")
                        })?);
                }
                "--guest-time" => {
                    let value = take_value(flag, inline, &arguments, &mut index)?;
                    guest_time = Some(GuestDateTime::parse(&value)?);
                }
                "--jobs" => {
                    let value = take_value(flag, inline, &arguments, &mut index)?;
                    jobs = value.parse::<usize>().map_err(|_| {
                        format!("invalid --jobs (expected a positive integer): {value}")
                    })?;
                    if jobs == 0 {
                        return Err("invalid --jobs (expected a positive integer): 0".to_owned());
                    }
                }
                other => return Err(format!("unknown option '{other}'")),
            }
        } else if directory.is_none() {
            directory = Some(PathBuf::from(argument));
        } else {
            return Err(format!("unexpected extra argument '{argument}'"));
        }
        index += 1;
    }

    Ok(Command::Orchestrate(OrchestrateArgs {
        directory: directory.ok_or_else(|| "missing test directory".to_owned())?,
        config,
        global_config,
        artifacts,
        timeout,
        guest_time,
        jobs,
    }))
}

fn parse_run(arguments: Vec<String>) -> Result<Command, String> {
    let mut run = RunArgs::default();
    let mut script: Option<PathBuf> = None;
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        if argument == "--" {
            run.script_args = arguments[index + 1..].to_vec();
            break;
        }
        if let Some((flag, inline)) = split_flag(argument) {
            match flag {
                "--help" | "-h" => return Ok(Command::Help),
                "--version" | "-V" => return Ok(Command::Version),
                "--config" => {
                    run.config = Some(PathBuf::from(take_value(
                        flag, inline, &arguments, &mut index,
                    )?));
                }
                "--global-config" => {
                    run.global_config = Some(PathBuf::from(take_value(
                        flag, inline, &arguments, &mut index,
                    )?));
                }
                "--artifacts" => {
                    run.artifacts = Some(PathBuf::from(take_value(
                        flag, inline, &arguments, &mut index,
                    )?));
                }
                "--timeout" => {
                    let value = take_value(flag, inline, &arguments, &mut index)?;
                    run.timeout =
                        Some(value.parse::<u64>().map_err(|_| {
                            format!("invalid --timeout (expected seconds): {value}")
                        })?);
                }
                "--guest-time" => {
                    let value = take_value(flag, inline, &arguments, &mut index)?;
                    run.guest_time = Some(GuestDateTime::parse(&value)?);
                }
                other => return Err(format!("unknown option '{other}'")),
            }
        } else if script.is_none() {
            script = Some(PathBuf::from(argument));
        } else {
            return Err(format!("unexpected extra argument '{argument}'"));
        }
        index += 1;
    }
    run.script = script.ok_or_else(|| "missing script path".to_owned())?;
    Ok(Command::Run(run))
}

/// Splits `--flag=value` into the flag and inline value, or returns the flag
/// with no inline value.
fn split_flag(argument: &str) -> Option<(&str, Option<&str>)> {
    if !argument.starts_with('-') {
        return None;
    }
    match argument.split_once('=') {
        Some((flag, value)) => Some((flag, Some(value))),
        None => Some((argument, None)),
    }
}

fn take_value(
    flag: &str,
    inline: Option<&str>,
    arguments: &[String],
    index: &mut usize,
) -> Result<String, String> {
    if let Some(value) = inline {
        return Ok(value.to_owned());
    }
    *index += 1;
    arguments
        .get(*index)
        .cloned()
        .ok_or_else(|| format!("option '{flag}' requires a value"))
}

/// Returns the help text.
#[must_use]
pub fn help_text() -> String {
    "\
neetan-auto - deterministic headless automation frontend

Usage:
  neetan-auto run <SCRIPT> [OPTIONS] [-- <SCRIPT-ARG>...]
  neetan-auto orchestrate <DIR> [OPTIONS]

Options:
  --config <PATH>          Common host settings (ROM locations, etc.)
  --global-config <PATH>   Explicit optional configuration layer
  --artifacts <PATH>       Artifact root (default: <script-dir>/artifacts/<stem>)
  --timeout <SECONDS>      Per-script wall-clock deadline (default: 600)
  --guest-time <DATETIME>  Fixed guest RTC value (default: 2000-01-01T00:00:00)
  --jobs <N>               Maximum parallel tests (default: CPU count, orchestrate only)
  --help                   Print this help
  --version                Print version information
"
    .to_owned()
}

/// Returns the version text.
#[must_use]
pub fn version_text() -> String {
    format!("neetan-auto {}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::{Command, parse};

    #[test]
    fn parses_orchestration_options() {
        let command = parse(
            ["orchestrate", "test tree", "--jobs=3", "--timeout", "12"]
                .map(str::to_owned)
                .into_iter()
                .collect(),
        )
        .expect("parse orchestrate command");
        let Command::Orchestrate(arguments) = command else {
            panic!("expected orchestrate command");
        };
        assert_eq!(arguments.directory.to_string_lossy(), "test tree");
        assert_eq!(arguments.jobs, 3);
        assert_eq!(arguments.timeout, Some(12));
    }

    #[test]
    fn rejects_zero_orchestration_jobs() {
        let error = parse(
            ["orchestrate", "tests", "--jobs", "0"]
                .map(str::to_owned)
                .into_iter()
                .collect(),
        )
        .expect_err("zero jobs must fail");
        assert!(error.contains("positive integer"));
    }
}
