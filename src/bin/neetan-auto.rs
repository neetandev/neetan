//! The `neetan-auto` binary entry point.

#![forbid(unsafe_code)]

use std::{process::exit, sync::mpsc, thread};

use automation::{
    CommonConfig, cli, execute_script, orchestration::orchestrate, render::render_run,
    watchdog::CancelHandle,
};

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let command = match cli::parse(arguments) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("error: {error}");
            exit(2);
        }
    };

    match command {
        cli::Command::Help => {
            print!("{}", cli::help_text());
            exit(0);
        }
        cli::Command::Version => {
            println!("{}", cli::version_text());
            exit(0);
        }
        cli::Command::Run(run) => exit(run_command(run)),
        cli::Command::Orchestrate(orchestrate_args) => {
            exit(orchestrate_command(orchestrate_args));
        }
    }
}

fn orchestrate_command(args: cli::OrchestrateArgs) -> i32 {
    let mut config = match CommonConfig::load(args.global_config.as_deref(), args.config.as_deref())
    {
        Ok(config) => config,
        Err(error) => {
            eprintln!("error: {error}");
            return 2;
        }
    };
    if let Some(artifacts) = args.artifacts {
        config.artifact_root = Some(artifacts);
    }
    if let Some(timeout) = args.timeout {
        config.timeout_seconds = timeout;
    }
    if let Some(guest_time) = args.guest_time {
        config.guest_time = guest_time;
    }

    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    match orchestrate(&args.directory, config, args.jobs, &mut output) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("error: {error}");
            error.exit_code()
        }
    }
}

fn run_command(run: cli::RunArgs) -> i32 {
    let mut config = match CommonConfig::load(run.global_config.as_deref(), run.config.as_deref()) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("error: {error}");
            return 2;
        }
    };
    if let Some(artifacts) = run.artifacts {
        config.artifact_root = Some(artifacts);
    }
    if let Some(timeout) = run.timeout {
        config.timeout_seconds = timeout;
    }
    if let Some(guest_time) = run.guest_time {
        config.guest_time = guest_time;
    }

    let (sender, receiver) = mpsc::channel();
    let cancel = CancelHandle::new();
    let script = run.script;
    let script_args = run.script_args;
    let worker = thread::spawn(move || {
        execute_script(script, config, script_args, sender, cancel);
    });

    let exit_code = render_run(&receiver);
    let _ = worker.join();
    exit_code
}
