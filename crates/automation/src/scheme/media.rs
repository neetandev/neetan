//! Media insert, eject, save, info, creation, and formatting natives.

use std::{cell::RefCell, rc::Rc};

use device::{
    disk::{HddSizeType, format::PartitionTableType},
    floppy::FddType,
};
use machine_factory::config::settings::blank_images::BlankFormat;
use r7rs::{Engine, Error, LibraryName, NativeContext, Value, ValueKind};

use super::support::{
    artifact_alist, error_value, machine_id, make_alist, op_error_value, to_count, written_len,
};
use crate::{
    media::{MediaKind, MediaMount, media_kind_from_name},
    session::AutomationSession,
};

/// Parses a hard-disk size symbol into a size. Dashes are optional, so both
/// `sasi40` and `sasi-40` resolve to the same size.
fn parse_hdd_size(name: &str) -> Result<HddSizeType, String> {
    name.replace('-', "").parse()
}

/// Parses a partition-table-type symbol (`pc98` or `at`).
fn parse_partition_table(name: &str) -> Result<PartitionTableType, String> {
    match name {
        "pc98" => Ok(PartitionTableType::Pc98),
        "at" => Ok(PartitionTableType::At),
        other => Err(format!(
            "unknown partition table type '{other}', expected pc98 or at"
        )),
    }
}

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

/// Parses the format symbol of `media-create!` for `kind`: a hard-disk size
/// for `hdd` media, a floppy type for `floppy` media.
fn parse_blank_format(kind: MediaKind, name: &str) -> Result<BlankFormat, String> {
    match kind {
        MediaKind::Hdd => parse_hdd_size(name).map(BlankFormat::HardDisk),
        MediaKind::Floppy => name.parse::<FddType>().map(BlankFormat::Floppy),
        MediaKind::Cdrom | MediaKind::Cartridge | MediaKind::Cassette | MediaKind::Printer => {
            Err(format!("{} media cannot be created", kind.symbol()))
        }
    }
}

/// Registers the media insert, eject, save, info, creation, and formatting
/// natives.
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

    let media_save = Rc::clone(session);
    engine.register_library_fn(internal, "%media-save", 4..=4, move |context, args| {
        if let Err(value) = machine_id(context, &media_save, args[0])? {
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
        match media_save.borrow_mut().media_save(kind, slot, &path) {
            Ok(written) => artifact_alist(context, &path, Some(written_len(&written))),
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

    let media_create = Rc::clone(session);
    engine.register_library_fn(internal, "%media-create", 4..=4, move |context, args| {
        if let Err(value) = machine_id(context, &media_create, args[0])? {
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
        let format_name = context.to_symbol_name(args[3])?.to_owned();
        let format = match parse_blank_format(kind, &format_name) {
            Ok(format) => format,
            Err(message) => return error_value(context, "neetan/argument", &message),
        };
        match media_create.borrow_mut().media_create(kind, slot, format) {
            Ok(mount) => media_mount_alist(context, &mount),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let media_format = Rc::clone(session);
    engine.register_library_fn(internal, "%media-format", 4..=4, move |context, args| {
        if let Err(value) = machine_id(context, &media_format, args[0])? {
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
        let table =
            if context.kind(args[3]) == ValueKind::Boolean && args[3] == Value::boolean(false) {
                None
            } else {
                let table_name = context.to_symbol_name(args[3])?.to_owned();
                match parse_partition_table(&table_name) {
                    Ok(table) => Some(table),
                    Err(message) => return error_value(context, "neetan/argument", &message),
                }
            };
        match media_format.borrow_mut().media_format(kind, slot, table) {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    Ok(())
}
