//! Bounded execution, reset, restore, and runtime save-state natives.

use std::{cell::RefCell, rc::Rc};

use r7rs::{Engine, Error, LibraryName, Value};

use super::support::{error_value, machine_id, op_error_value, run_result_value, to_count};
use crate::session::AutomationSession;

/// Registers the bounded execution and reset natives.
pub(super) fn register_run_natives(
    engine: &mut Engine,
    session: &Rc<RefCell<AutomationSession>>,
    internal: &LibraryName,
) -> Result<(), Error> {
    let run_ticks = Rc::clone(session);
    engine.register_library_fn(internal, "%run-ticks", 2..=2, move |context, args| {
        if let Err(value) = machine_id(context, &run_ticks, args[0])? {
            return Ok(value);
        }
        let count = match to_count(context, args[1])? {
            Ok(count) => count,
            Err(value) => return Ok(value),
        };
        let result = run_ticks.borrow_mut().advance_ticks(count);
        run_result_value(context, &run_ticks, result)
    })?;

    let run_frames = Rc::clone(session);
    engine.register_library_fn(internal, "%run-frames", 3..=3, move |context, args| {
        if let Err(value) = machine_id(context, &run_frames, args[0])? {
            return Ok(value);
        }
        let count = match to_count(context, args[1])? {
            Ok(count) => count,
            Err(value) => return Ok(value),
        };
        let max_ticks = match to_count(context, args[2])? {
            Ok(value) => value,
            Err(value) => return Ok(value),
        };
        let result = run_frames.borrow_mut().advance_frames(count, max_ticks);
        run_result_value(context, &run_frames, result)
    })?;

    let run_until_frame = Rc::clone(session);
    engine.register_library_fn(internal, "%run-until-frame", 3..=3, move |context, args| {
        if let Err(value) = machine_id(context, &run_until_frame, args[0])? {
            return Ok(value);
        }
        let raw_frame = context.to_i128(args[1])?;
        let frame = match u128::try_from(raw_frame) {
            Ok(frame) => frame,
            Err(_) => return error_value(context, "neetan/range", "frame is out of range"),
        };
        let max_ticks = match to_count(context, args[2])? {
            Ok(value) => value,
            Err(value) => return Ok(value),
        };
        let result = run_until_frame
            .borrow_mut()
            .advance_until_frame(frame, max_ticks);
        run_result_value(context, &run_until_frame, result)
    })?;

    let reset = Rc::clone(session);
    engine.register_library_fn(internal, "%reset", 2..=2, move |context, args| {
        if let Err(value) = machine_id(context, &reset, args[0])? {
            return Ok(value);
        }
        let kind = context.to_symbol_name(args[1])?.to_owned();
        let hard = match kind.as_str() {
            "hard" => true,
            "soft" => false,
            _ => {
                return error_value(
                    context,
                    "neetan/argument",
                    "reset kind must be 'hard or 'soft",
                );
            }
        };
        match reset.borrow_mut().reset(hard) {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let restore_startup = Rc::clone(session);
    engine.register_library_fn(internal, "%restore-startup", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &restore_startup, args[0])? {
            return Ok(value);
        }
        match restore_startup.borrow_mut().restore_startup() {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    register_state_natives(engine, session, internal)
}

/// Registers the runtime save-state natives.
fn register_state_natives(
    engine: &mut Engine,
    session: &Rc<RefCell<AutomationSession>>,
    internal: &LibraryName,
) -> Result<(), Error> {
    let save_state = Rc::clone(session);
    engine.register_library_fn(internal, "%save-state", 1..=1, move |context, args| {
        let machine_id = match machine_id(context, &save_state, args[0])? {
            Ok(id) => id,
            Err(value) => return Ok(value),
        };
        match save_state.borrow_mut().save_state(machine_id) {
            Ok(handle) => context.integer(i128::from(handle)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let restore_state = Rc::clone(session);
    engine.register_library_fn(internal, "%restore-state", 2..=2, move |context, args| {
        let machine_id = match machine_id(context, &restore_state, args[0])? {
            Ok(id) => id,
            Err(value) => return Ok(value),
        };
        let handle = match to_count(context, args[1])? {
            Ok(handle) => handle,
            Err(value) => return Ok(value),
        };
        match restore_state.borrow_mut().restore_state(machine_id, handle) {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let discard_state = Rc::clone(session);
    engine.register_library_fn(internal, "%discard-state", 2..=2, move |context, args| {
        let machine_id = match machine_id(context, &discard_state, args[0])? {
            Ok(id) => id,
            Err(value) => return Ok(value),
        };
        let handle = match to_count(context, args[1])? {
            Ok(handle) => handle,
            Err(value) => return Ok(value),
        };
        match discard_state.borrow_mut().discard_state(machine_id, handle) {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    Ok(())
}
