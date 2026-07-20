//! The `run` command console renderer.
//!
//! It prints each captured output chunk live, shows a single-line wall-clock
//! heartbeat from progress messages, prints the final status, and returns the
//! process exit code derived from the terminal `RunTermination`.

use std::{io::Write, sync::mpsc::Receiver};

use crate::protocol::{ExecutionResult, MessageProtocol, RunTermination};

/// Renders the message stream and returns the process exit code.
pub fn render_run(events: &Receiver<MessageProtocol>) -> i32 {
    let mut heartbeat_shown = false;
    for message in events {
        match message {
            MessageProtocol::Started { script } => {
                println!("running {}", script.display());
            }
            MessageProtocol::MachineReady { identity } => {
                println!("machine ready: {} / {}", identity.target, identity.model);
            }
            MessageProtocol::Output(text) => {
                clear_heartbeat(&mut heartbeat_shown);
                print!("{text}");
                let _ = std::io::stdout().flush();
            }
            MessageProtocol::Progress(progress) => {
                let seconds = progress.wall_elapsed_ms as f64 / 1000.0;
                eprint!("\r[ {seconds:6.1}s ]");
                let _ = std::io::stderr().flush();
                heartbeat_shown = true;
            }
            MessageProtocol::Result(result) => {
                clear_heartbeat(&mut heartbeat_shown);
                match result {
                    ExecutionResult::Ok => println!("result: OK"),
                    ExecutionResult::Error { message } => {
                        println!("result: ERROR {message}");
                    }
                }
            }
            MessageProtocol::TestCaseFinished { .. } => {}
            MessageProtocol::Finished(termination) => {
                clear_heartbeat(&mut heartbeat_shown);
                print_final(&termination);
                return termination.exit_code();
            }
        }
    }
    // The sender dropped without a Finished message.
    eprintln!("internal error: executor ended without a result");
    4
}

fn clear_heartbeat(heartbeat_shown: &mut bool) {
    if *heartbeat_shown {
        eprint!("\r            \r");
        let _ = std::io::stderr().flush();
        *heartbeat_shown = false;
    }
}

fn print_final(termination: &RunTermination) {
    match termination {
        RunTermination::Completed(ExecutionResult::Ok) => println!("finished: passed"),
        RunTermination::Completed(ExecutionResult::Error { message }) => {
            println!("finished: failed: {message}");
        }
        RunTermination::NoResult => {
            eprintln!("finished: script set no result");
        }
        RunTermination::Timeout => eprintln!("finished: timed out"),
        RunTermination::Cancelled => eprintln!("finished: cancelled"),
        RunTermination::ConfigError(message) => {
            eprintln!("finished: configuration error: {message}")
        }
        RunTermination::CompileError(diagnostic) => {
            eprintln!("finished: compile error: {}", diagnostic.message());
        }
        RunTermination::RuntimeError(diagnostic) => {
            eprintln!("finished: runtime error: {}", diagnostic.message());
        }
        RunTermination::Internal(message) => eprintln!("finished: internal error: {message}"),
    }
}
