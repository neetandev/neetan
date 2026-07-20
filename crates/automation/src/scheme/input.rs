//! Logical key, text, joystick, and mouse natives.

use std::{cell::RefCell, rc::Rc};

use common::StopReason;
use r7rs::{Engine, Error, LibraryName, Value};

use super::support::{
    error_value, is_truthy, machine_id, op_error_value, run_outcome_value, to_count,
};
use crate::{
    input::{key_from_name, mouse_button_from_name},
    session::AutomationSession,
};

/// Registers the logical key, text, joystick, and mouse natives.
pub(super) fn register_input_natives(
    engine: &mut Engine,
    session: &Rc<RefCell<AutomationSession>>,
    internal: &LibraryName,
) -> Result<(), Error> {
    let key_down = Rc::clone(session);
    engine.register_library_fn(internal, "%key-down", 2..=2, move |context, args| {
        if let Err(value) = machine_id(context, &key_down, args[0])? {
            return Ok(value);
        }
        let name = context.to_symbol_name(args[1])?.to_owned();
        let Some(key) = key_from_name(&name) else {
            return error_value(context, "neetan/argument", &format!("unknown key '{name}'"));
        };
        match key_down.borrow_mut().key_down(key) {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let key_up = Rc::clone(session);
    engine.register_library_fn(internal, "%key-up", 2..=2, move |context, args| {
        if let Err(value) = machine_id(context, &key_up, args[0])? {
            return Ok(value);
        }
        let name = context.to_symbol_name(args[1])?.to_owned();
        let Some(key) = key_from_name(&name) else {
            return error_value(context, "neetan/argument", &format!("unknown key '{name}'"));
        };
        match key_up.borrow_mut().key_up(key) {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let key_tap = Rc::clone(session);
    engine.register_library_fn(internal, "%key-tap", 4..=4, move |context, args| {
        if let Err(value) = machine_id(context, &key_tap, args[0])? {
            return Ok(value);
        }
        let name = context.to_symbol_name(args[1])?.to_owned();
        let Some(key) = key_from_name(&name) else {
            return error_value(context, "neetan/argument", &format!("unknown key '{name}'"));
        };
        let frames = match to_count(context, args[2])? {
            Ok(frames) => frames,
            Err(value) => return Ok(value),
        };
        let max_ticks = match to_count(context, args[3])? {
            Ok(value) => value,
            Err(value) => return Ok(value),
        };
        let result = key_tap.borrow_mut().key_tap(key, frames, max_ticks);
        match result {
            Err(error) => op_error_value(context, &error),
            Ok(outcome) => match outcome.stop_reason {
                StopReason::TickLimit => error_value(
                    context,
                    "neetan/timeout",
                    "tick limit reached before target",
                ),
                StopReason::GuestShutdown => {
                    error_value(context, "neetan/guest-shutdown", "guest requested shutdown")
                }
                StopReason::TargetReached
                | StopReason::Cancelled
                | StopReason::CounterExhausted
                | StopReason::MachineError => run_outcome_value(context, &key_tap, &outcome),
            },
        }
    })?;

    let type_text = Rc::clone(session);
    engine.register_library_fn(internal, "%type-text", 4..=4, move |context, args| {
        if let Err(value) = machine_id(context, &type_text, args[0])? {
            return Ok(value);
        }
        let text = context.to_str(args[1])?.to_owned();
        let spacing = match to_count(context, args[2])? {
            Ok(spacing) => spacing,
            Err(value) => return Ok(value),
        };
        let max_ticks = match to_count(context, args[3])? {
            Ok(value) => value,
            Err(value) => return Ok(value),
        };
        match type_text.borrow_mut().type_text(&text, spacing, max_ticks) {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    register_pointer_natives(engine, session, internal)
}

/// Registers the joystick and mouse natives.
fn register_pointer_natives(
    engine: &mut Engine,
    session: &Rc<RefCell<AutomationSession>>,
    internal: &LibraryName,
) -> Result<(), Error> {
    let joystick_set = Rc::clone(session);
    engine.register_library_fn(internal, "%joystick-set", 4..=4, move |context, args| {
        if let Err(value) = machine_id(context, &joystick_set, args[0])? {
            return Ok(value);
        }
        let index = match to_count(context, args[1])? {
            Ok(index) => index as usize,
            Err(value) => return Ok(value),
        };
        let control = context.to_symbol_name(args[2])?.to_owned();
        let pressed = is_truthy(args[3]);
        match joystick_set
            .borrow_mut()
            .joystick_set(index, &control, pressed)
        {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let joystick_clear = Rc::clone(session);
    engine.register_library_fn(internal, "%joystick-clear", 2..=2, move |context, args| {
        if let Err(value) = machine_id(context, &joystick_clear, args[0])? {
            return Ok(value);
        }
        let index = match to_count(context, args[1])? {
            Ok(index) => index as usize,
            Err(value) => return Ok(value),
        };
        match joystick_clear.borrow_mut().joystick_clear(index) {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let mouse_move = Rc::clone(session);
    engine.register_library_fn(internal, "%mouse-move", 3..=3, move |context, args| {
        if let Err(value) = machine_id(context, &mouse_move, args[0])? {
            return Ok(value);
        }
        let delta_x = context.to_i128(args[1])?;
        let delta_y = context.to_i128(args[2])?;
        match mouse_move.borrow_mut().mouse_move(delta_x, delta_y) {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let mouse_button = Rc::clone(session);
    engine.register_library_fn(internal, "%mouse-button", 3..=3, move |context, args| {
        if let Err(value) = machine_id(context, &mouse_button, args[0])? {
            return Ok(value);
        }
        let name = context.to_symbol_name(args[1])?.to_owned();
        let Some(button) = mouse_button_from_name(&name) else {
            return error_value(
                context,
                "neetan/argument",
                &format!("unknown mouse button '{name}'"),
            );
        };
        let pressed = is_truthy(args[2]);
        match mouse_button.borrow_mut().mouse_button(button, pressed) {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    Ok(())
}
