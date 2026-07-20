//! Media insert, eject, flush, and info natives.

use std::{cell::RefCell, rc::Rc};

use r7rs::{Engine, Error, LibraryName, NativeContext, Value};

use super::support::{error_value, machine_id, make_alist, op_error_value, to_count};
use crate::{
    media::{MediaKind, MediaMount, media_kind_from_name},
    session::AutomationSession,
};

/// Builds the media alist describing one mount.
fn media_mount_alist(context: &mut NativeContext, mount: &MediaMount) -> Result<Value, Error> {
    let kind = context.intern_symbol(mount.kind.symbol())?;
    let slot = context.integer(i128::try_from(mount.slot).unwrap_or(i128::MAX))?;
    let format = context.string_utf8(mount.format.clone())?;
    let description = context.string_utf8(mount.description.clone())?;
    let source = context.string_utf8(mount.requested.clone())?;
    let private = match &mount.printer_artifact {
        Some(path) => context.string_utf8(path.display().to_string())?,
        None => Value::boolean(false),
    };
    make_alist(
        context,
        vec![
            ("type", kind),
            ("slot", slot),
            ("format", format),
            ("description", description),
            ("source", source),
            ("private", private),
            ("write-protected", Value::boolean(mount.write_protected)),
            ("dirty", Value::boolean(mount.dirty)),
        ],
    )
}

/// Reads a media type symbol argument, or returns the tagged argument error.
fn media_kind_argument(
    context: &mut NativeContext,
    value: Value,
) -> Result<Result<MediaKind, Value>, Error> {
    let name = context.to_symbol_name(value)?.to_owned();
    match media_kind_from_name(&name) {
        Some(kind) => Ok(Ok(kind)),
        None => Ok(Err(error_value(
            context,
            "neetan/argument",
            &format!("unknown media type '{name}'"),
        )?)),
    }
}

/// Registers the media insert, eject, flush, and info natives.
pub(super) fn register_media_natives(
    engine: &mut Engine,
    session: &Rc<RefCell<AutomationSession>>,
    internal: &LibraryName,
) -> Result<(), Error> {
    let media_insert = Rc::clone(session);
    engine.register_library_fn(internal, "%media-insert", 4..=4, move |context, args| {
        if let Err(value) = machine_id(context, &media_insert, args[0])? {
            return Ok(value);
        }
        let kind = match media_kind_argument(context, args[1])? {
            Ok(kind) => kind,
            Err(value) => return Ok(value),
        };
        let slot = match to_count(context, args[2])? {
            Ok(slot) => slot as usize,
            Err(value) => return Ok(value),
        };
        let path = context.to_str(args[3])?.to_owned();
        match media_insert.borrow_mut().media_insert(kind, slot, path) {
            Ok(mount) => media_mount_alist(context, &mount),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let media_eject = Rc::clone(session);
    engine.register_library_fn(internal, "%media-eject", 3..=3, move |context, args| {
        if let Err(value) = machine_id(context, &media_eject, args[0])? {
            return Ok(value);
        }
        let kind = match media_kind_argument(context, args[1])? {
            Ok(kind) => kind,
            Err(value) => return Ok(value),
        };
        let slot = match to_count(context, args[2])? {
            Ok(slot) => slot as usize,
            Err(value) => return Ok(value),
        };
        match media_eject.borrow_mut().media_eject(kind, slot) {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let media_flush = Rc::clone(session);
    engine.register_library_fn(internal, "%media-flush", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &media_flush, args[0])? {
            return Ok(value);
        }
        match media_flush.borrow_mut().media_flush() {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let media_info = Rc::clone(session);
    engine.register_library_fn(internal, "%media-info", 3..=3, move |context, args| {
        if let Err(value) = machine_id(context, &media_info, args[0])? {
            return Ok(value);
        }
        let kind = match media_kind_argument(context, args[1])? {
            Ok(kind) => kind,
            Err(value) => return Ok(value),
        };
        let slot = match to_count(context, args[2])? {
            Ok(slot) => slot as usize,
            Err(value) => return Ok(value),
        };
        let mount = media_info.borrow().media_info(kind, slot);
        match mount {
            Some(mount) => media_mount_alist(context, &mount),
            None => Ok(Value::boolean(false)),
        }
    })?;

    Ok(())
}
