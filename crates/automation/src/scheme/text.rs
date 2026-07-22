//! Decoded text-surface inspection, waiting, and dump natives.

use std::{cell::RefCell, rc::Rc};

use common::TextCell;
use r7rs::{Engine, Error, LibraryName, NativeContext, Value, ValueKind};

use super::support::{
    artifact_alist, machine_id, make_alist, make_list, op_error_value, to_count, written_len,
};
use crate::session::{AutomationSession, text::TextMatch};

/// Marshals one decoded text cell into a normalized alist.
fn cell_value(context: &mut NativeContext, cell: &TextCell) -> Result<Value, Error> {
    let row = context.integer(i128::from(cell.row))?;
    let column = context.integer(i128::from(cell.column))?;
    let raw_jis = context.integer(i128::from(cell.raw_jis))?;
    let unicode = match cell.unicode {
        Some(character) => Value::character(character),
        None => Value::boolean(false),
    };
    let attribute = context.integer(i128::from(cell.attribute))?;
    let display_width = context.integer(i128::from(cell.display_width))?;
    make_alist(
        context,
        vec![
            ("row", row),
            ("column", column),
            ("raw-jis", raw_jis),
            ("unicode", unicode),
            ("attribute", attribute),
            ("display-width", display_width),
        ],
    )
}

/// Marshals a text-surface descriptor into a normalized alist.
fn surface_info_value(
    context: &mut NativeContext,
    info: &common::TextSurfaceInfo,
) -> Result<Value, Error> {
    let id = context.intern_symbol(info.id)?;
    let rows = context.integer(i128::from(info.rows))?;
    let columns = context.integer(i128::from(info.columns))?;
    make_alist(
        context,
        vec![("id", id), ("rows", rows), ("columns", columns)],
    )
}

/// Parses a bounded row argument, accepting a non-negative integer or `#f`.
fn parse_optional_row(
    context: &mut NativeContext,
    value: Value,
) -> Result<Result<Option<u16>, Value>, Error> {
    if context.kind(value) == ValueKind::Boolean && value == Value::boolean(false) {
        return Ok(Ok(None));
    }
    match to_count(context, value)? {
        Ok(row) => Ok(Ok(Some(u16::try_from(row).unwrap_or(u16::MAX)))),
        Err(value) => Ok(Err(value)),
    }
}

/// Registers the decoded text-surface natives.
pub(super) fn register_text_natives(
    engine: &mut Engine,
    session: &Rc<RefCell<AutomationSession>>,
    internal: &LibraryName,
) -> Result<(), Error> {
    let surfaces = Rc::clone(session);
    engine.register_library_fn(internal, "%text-surfaces", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &surfaces, args[0])? {
            return Ok(value);
        }
        match surfaces.borrow().text_surfaces() {
            Ok(ids) => {
                let mut values = Vec::with_capacity(ids.len());
                for id in ids {
                    values.push(context.intern_symbol(id)?);
                }
                make_list(context, values)
            }
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let surface_info = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%text-surface-info",
        2..=2,
        move |context, args| {
            if let Err(value) = machine_id(context, &surface_info, args[0])? {
                return Ok(value);
            }
            let surface = context.to_symbol_name(args[1])?.to_owned();
            match surface_info.borrow().text_surface_info(&surface) {
                Ok(info) => surface_info_value(context, &info),
                Err(error) => op_error_value(context, &error),
            }
        },
    )?;

    let text_cell = Rc::clone(session);
    engine.register_library_fn(internal, "%text-cell", 4..=4, move |context, args| {
        if let Err(value) = machine_id(context, &text_cell, args[0])? {
            return Ok(value);
        }
        let surface = context.to_symbol_name(args[1])?.to_owned();
        let row = match to_count(context, args[2])? {
            Ok(row) => u16::try_from(row).unwrap_or(u16::MAX),
            Err(value) => return Ok(value),
        };
        let column = match to_count(context, args[3])? {
            Ok(column) => u16::try_from(column).unwrap_or(u16::MAX),
            Err(value) => return Ok(value),
        };
        match text_cell.borrow().text_cell(&surface, row, column) {
            Ok(cell) => cell_value(context, &cell),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let text_screen = Rc::clone(session);
    engine.register_library_fn(internal, "%text-screen", 2..=2, move |context, args| {
        if let Err(value) = machine_id(context, &text_screen, args[0])? {
            return Ok(value);
        }
        let surface = context.to_symbol_name(args[1])?.to_owned();
        match text_screen.borrow().text_screen_lines(&surface) {
            Ok(lines) => {
                let mut values = Vec::with_capacity(lines.len());
                for line in lines {
                    values.push(context.string_utf8(line)?);
                }
                make_list(context, values)
            }
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let save_text = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%save-text-screen",
        3..=3,
        move |context, args| {
            if let Err(value) = machine_id(context, &save_text, args[0])? {
                return Ok(value);
            }
            let surface = context.to_symbol_name(args[1])?.to_owned();
            let path = context.to_str(args[2])?.to_owned();
            match save_text.borrow_mut().save_text_screen(&surface, &path) {
                Ok(written) => artifact_alist(context, &path, Some(written_len(&written))),
                Err(error) => op_error_value(context, &error),
            }
        },
    )?;

    let wait_for_text = Rc::clone(session);
    engine.register_library_fn(internal, "%wait-for-text", 6..=6, move |context, args| {
        if let Err(value) = machine_id(context, &wait_for_text, args[0])? {
            return Ok(value);
        }
        let surface = context.to_symbol_name(args[1])?.to_owned();
        let contains = context.to_str(args[2])?.to_owned();
        let row = match parse_optional_row(context, args[3])? {
            Ok(row) => row,
            Err(value) => return Ok(value),
        };
        let max_frames = match to_count(context, args[4])? {
            Ok(value) => value,
            Err(value) => return Ok(value),
        };
        let max_ticks = match to_count(context, args[5])? {
            Ok(value) => value,
            Err(value) => return Ok(value),
        };
        let predicate = TextMatch {
            surface,
            row,
            contains,
        };
        match wait_for_text
            .borrow_mut()
            .wait_for_text(predicate, max_frames, max_ticks)
        {
            Ok(Some(text)) => context.string_utf8(text),
            Ok(None) => Ok(Value::boolean(false)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    Ok(())
}
