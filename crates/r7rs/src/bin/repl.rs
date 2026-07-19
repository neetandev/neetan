use std::io::{self, Write};

use r7rs::{Engine, EngineConfig, ErrorKind, EvalOutcome, Extension};

fn main() {
    if let Err(error) = run() {
        eprintln!("repl: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = Engine::new(EngineConfig::standalone())?;
    // The standalone REPL offers everything this build has, so every optional
    // extension library is importable out of the box.
    for extension in Extension::ALL {
        engine.install_extension(*extension)?;
    }
    let mut source = String::new();
    let mut input_number = 1_u64;

    loop {
        print!("{}", if source.is_empty() { "> " } else { "| " });
        io::stdout().flush()?;

        // Read one line without holding the stdin lock across evaluation.
        // The engine's standard input port shares this stream, so a Scheme
        // read procedure must be able to take the lock mid-eval.
        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            if !source.trim().is_empty() {
                match engine.compile_interactive(format!("repl-{input_number}"), &source) {
                    Ok(_) => {}
                    Err(error) => eprint!("{}", engine.render_error(&error)),
                }
            }
            println!();
            break;
        }
        source.push_str(&line);
        if !line.ends_with('\n') {
            source.push('\n');
        }

        let module = match engine.compile_interactive(format!("repl-{input_number}"), &source) {
            Ok(module) => module,
            Err(error) if error.kind() == ErrorKind::UnexpectedEof => continue,
            Err(error) => {
                eprint!("{}", engine.render_error(&error));
                source.clear();
                input_number += 1;
                continue;
            }
        };

        match engine.eval(&module) {
            Ok(EvalOutcome::Values(values)) => {
                for value in values.as_slice() {
                    match engine.write_root(value) {
                        Ok(rendered) => println!("{rendered}"),
                        Err(error) => eprint!("{}", engine.render_error(&error)),
                    }
                }
            }
            Ok(EvalOutcome::Exited(_)) => break,
            Err(error) => eprint!("{}", engine.render_error(&error)),
        }

        source.clear();
        input_number += 1;
    }

    Ok(())
}
