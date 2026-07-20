//! Read-only inspection and mutation natives.

use std::{cell::RefCell, rc::Rc};

use common::{
    AddressSpaceClass, AddressSpaceDescriptor, ByteOrder, ProcessorDescriptor, ProtectedModeState,
    RegisterReading,
};
use r7rs::{Engine, Error, LibraryName, NativeContext, Value};

use super::support::{
    error_value, inspected_integer, machine_id, make_alist, make_list, op_error_value, to_count,
    to_u32, to_u128,
};
use crate::session::AutomationSession;

/// Returns the stable symbol name of a byte order.
fn byte_order_name(order: ByteOrder) -> &'static str {
    match order {
        ByteOrder::Little => "little",
        ByteOrder::Big => "big",
    }
}

/// Parses a byte-order symbol into an optional order, where `native` is `None`.
fn parse_byte_order(
    context: &mut NativeContext,
    value: Value,
) -> Result<Result<Option<ByteOrder>, Value>, Error> {
    let name = context.to_symbol_name(value)?.to_owned();
    match name.as_str() {
        "little" => Ok(Ok(Some(ByteOrder::Little))),
        "big" => Ok(Ok(Some(ByteOrder::Big))),
        "native" => Ok(Ok(None)),
        other => Ok(Err(error_value(
            context,
            "neetan/argument",
            &format!("unknown byte order '{other}'"),
        )?)),
    }
}

/// Builds the processor descriptor alist required by `processor-info`.
fn processor_info_value(
    context: &mut NativeContext,
    descriptor: &ProcessorDescriptor,
) -> Result<Value, Error> {
    let id = context.intern_symbol(descriptor.id)?;
    let architecture = context.intern_symbol(descriptor.architecture)?;
    let mut register_entries = Vec::with_capacity(descriptor.registers.len());
    for register in descriptor.registers {
        let name = context.intern_symbol(register.name)?;
        let bits = context.integer(i128::from(register.bits))?;
        let entry = make_alist(
            context,
            vec![
                ("name", name),
                ("bits", bits),
                ("writable", Value::boolean(register.writable)),
            ],
        )?;
        register_entries.push(entry);
    }
    let registers = make_list(context, register_entries)?;
    make_alist(
        context,
        vec![
            ("id", id),
            ("architecture", architecture),
            ("protected-mode", Value::boolean(descriptor.protected_mode)),
            ("registers", registers),
        ],
    )
}

/// Builds the address-space descriptor alist required by `address-space-info`.
fn address_space_info_value(
    context: &mut NativeContext,
    descriptor: &AddressSpaceDescriptor,
) -> Result<Value, Error> {
    let id = context.intern_symbol(descriptor.id)?;
    let class = context.intern_symbol(match descriptor.class {
        AddressSpaceClass::Memory => "memory",
        AddressSpaceClass::Io => "io",
    })?;
    let address_bits = context.integer(i128::from(descriptor.address_bits))?;
    let byte_order = context.intern_symbol(byte_order_name(descriptor.byte_order))?;
    make_alist(
        context,
        vec![
            ("id", id),
            ("class", class),
            ("address-bits", address_bits),
            ("byte-order", byte_order),
            ("peekable", Value::boolean(descriptor.peekable)),
            ("writable", Value::boolean(descriptor.writable)),
        ],
    )
}

/// Builds a register-name to value alist from a list of readings.
fn register_readings_alist(
    context: &mut NativeContext,
    readings: &[RegisterReading],
) -> Result<Value, Error> {
    let mut entries: Vec<(&str, Value)> = Vec::with_capacity(readings.len());
    for reading in readings {
        let value = inspected_integer(context, reading.value)?;
        entries.push((reading.name, value));
    }
    make_alist(context, entries)
}

/// Builds the protected-mode state alist required by `protected-mode-state`.
fn protected_mode_value(
    context: &mut NativeContext,
    state: &ProtectedModeState,
) -> Result<Value, Error> {
    let general = register_readings_alist(context, &state.general)?;
    let mut segment_entries = Vec::with_capacity(state.segments.len());
    for segment in &state.segments {
        let name = context.intern_symbol(segment.name)?;
        let selector = inspected_integer(context, u128::from(segment.selector))?;
        let base = inspected_integer(context, u128::from(segment.base))?;
        let limit = inspected_integer(context, u128::from(segment.limit))?;
        let rights = inspected_integer(context, u128::from(segment.rights))?;
        let entry = make_alist(
            context,
            vec![
                ("name", name),
                ("selector", selector),
                ("base", base),
                ("limit", limit),
                ("rights", rights),
            ],
        )?;
        segment_entries.push(entry);
    }
    let segments = make_list(context, segment_entries)?;
    let control = register_readings_alist(context, &state.control)?;
    let debug = register_readings_alist(context, &state.debug)?;
    let mut table_entries = Vec::with_capacity(state.descriptor_tables.len());
    for table in &state.descriptor_tables {
        let name = context.intern_symbol(table.name)?;
        let selector = match table.selector {
            Some(value) => inspected_integer(context, u128::from(value))?,
            None => Value::boolean(false),
        };
        let base = inspected_integer(context, u128::from(table.base))?;
        let limit = inspected_integer(context, u128::from(table.limit))?;
        let entry = make_alist(
            context,
            vec![
                ("name", name),
                ("selector", selector),
                ("base", base),
                ("limit", limit),
            ],
        )?;
        table_entries.push(entry);
    }
    let descriptor_tables = make_list(context, table_entries)?;
    let eip = inspected_integer(context, u128::from(state.eip))?;
    let eflags = inspected_integer(context, u128::from(state.eflags))?;
    make_alist(
        context,
        vec![
            ("general", general),
            ("segments", segments),
            ("control", control),
            ("debug", debug),
            ("descriptor-tables", descriptor_tables),
            ("eip", eip),
            ("eflags", eflags),
        ],
    )
}

/// Registers the read-only inspection natives.
pub(super) fn register_inspect_natives(
    engine: &mut Engine,
    session: &Rc<RefCell<AutomationSession>>,
    internal: &LibraryName,
) -> Result<(), Error> {
    let processors = Rc::clone(session);
    engine.register_library_fn(internal, "%processors", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &processors, args[0])? {
            return Ok(value);
        }
        match processors.borrow_mut().processors() {
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

    let processor_info = Rc::clone(session);
    engine.register_library_fn(internal, "%processor-info", 2..=2, move |context, args| {
        if let Err(value) = machine_id(context, &processor_info, args[0])? {
            return Ok(value);
        }
        let processor = context.to_symbol_name(args[1])?.to_owned();
        match processor_info.borrow_mut().processor_info(&processor) {
            Ok(descriptor) => processor_info_value(context, &descriptor),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let registers = Rc::clone(session);
    engine.register_library_fn(internal, "%registers", 2..=2, move |context, args| {
        if let Err(value) = machine_id(context, &registers, args[0])? {
            return Ok(value);
        }
        let processor = context.to_symbol_name(args[1])?.to_owned();
        match registers.borrow_mut().processor_registers(&processor) {
            Ok(readings) => register_readings_alist(context, &readings),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let register_ref = Rc::clone(session);
    engine.register_library_fn(internal, "%register-ref", 3..=3, move |context, args| {
        if let Err(value) = machine_id(context, &register_ref, args[0])? {
            return Ok(value);
        }
        let processor = context.to_symbol_name(args[1])?.to_owned();
        let register = context.to_symbol_name(args[2])?.to_owned();
        match register_ref
            .borrow_mut()
            .read_register(&processor, &register)
        {
            Ok(value) => inspected_integer(context, value),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let protected_mode_state = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%protected-mode-state",
        2..=2,
        move |context, args| {
            if let Err(value) = machine_id(context, &protected_mode_state, args[0])? {
                return Ok(value);
            }
            let processor = context.to_symbol_name(args[1])?.to_owned();
            match protected_mode_state
                .borrow_mut()
                .protected_mode_state(&processor)
            {
                Ok(state) => protected_mode_value(context, &state),
                Err(error) => op_error_value(context, &error),
            }
        },
    )?;

    let address_spaces = Rc::clone(session);
    engine.register_library_fn(internal, "%address-spaces", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &address_spaces, args[0])? {
            return Ok(value);
        }
        match address_spaces.borrow_mut().address_spaces() {
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

    let address_space_info = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%address-space-info",
        2..=2,
        move |context, args| {
            if let Err(value) = machine_id(context, &address_space_info, args[0])? {
                return Ok(value);
            }
            let space = context.to_symbol_name(args[1])?.to_owned();
            match address_space_info.borrow_mut().address_space_info(&space) {
                Ok(descriptor) => address_space_info_value(context, &descriptor),
                Err(error) => op_error_value(context, &error),
            }
        },
    )?;

    let memory_read = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%memory-read-bytevector",
        4..=4,
        move |context, args| {
            if let Err(value) = machine_id(context, &memory_read, args[0])? {
                return Ok(value);
            }
            let space = context.to_symbol_name(args[1])?.to_owned();
            let address = match to_count(context, args[2])? {
                Ok(address) => address,
                Err(value) => return Ok(value),
            };
            let length = match to_count(context, args[3])? {
                Ok(length) => length,
                Err(value) => return Ok(value),
            };
            match memory_read
                .borrow_mut()
                .peek_memory(&space, address, length)
            {
                Ok(bytes) => context.bytevector(bytes),
                Err(error) => op_error_value(context, &error),
            }
        },
    )?;

    let memory_peek = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%memory-peek-unsigned",
        5..=5,
        move |context, args| {
            if let Err(value) = machine_id(context, &memory_peek, args[0])? {
                return Ok(value);
            }
            let space = context.to_symbol_name(args[1])?.to_owned();
            let address = match to_count(context, args[2])? {
                Ok(address) => address,
                Err(value) => return Ok(value),
            };
            let width = match to_u32(context, args[3])? {
                Ok(width) => width,
                Err(value) => return Ok(value),
            };
            let order = match parse_byte_order(context, args[4])? {
                Ok(order) => order,
                Err(value) => return Ok(value),
            };
            match memory_peek
                .borrow_mut()
                .peek_unsigned(&space, address, width, order)
            {
                Ok(value) => inspected_integer(context, value),
                Err(error) => op_error_value(context, &error),
            }
        },
    )?;

    Ok(())
}

/// Registers the mutation natives.
pub(super) fn register_mutate_natives(
    engine: &mut Engine,
    session: &Rc<RefCell<AutomationSession>>,
    internal: &LibraryName,
) -> Result<(), Error> {
    let register_set = Rc::clone(session);
    engine.register_library_fn(internal, "%register-set", 4..=4, move |context, args| {
        if let Err(value) = machine_id(context, &register_set, args[0])? {
            return Ok(value);
        }
        let processor = context.to_symbol_name(args[1])?.to_owned();
        let register = context.to_symbol_name(args[2])?.to_owned();
        let value = match to_u128(context, args[3])? {
            Ok(value) => value,
            Err(value) => return Ok(value),
        };
        match register_set
            .borrow_mut()
            .write_register(&processor, &register, value)
        {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let memory_write = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%memory-write-bytevector",
        4..=4,
        move |context, args| {
            if let Err(value) = machine_id(context, &memory_write, args[0])? {
                return Ok(value);
            }
            let space = context.to_symbol_name(args[1])?.to_owned();
            let address = match to_count(context, args[2])? {
                Ok(address) => address,
                Err(value) => return Ok(value),
            };
            let bytes = context.to_bytes(args[3])?.to_vec();
            match memory_write
                .borrow_mut()
                .poke_memory(&space, address, &bytes)
            {
                Ok(()) => Ok(Value::boolean(true)),
                Err(error) => op_error_value(context, &error),
            }
        },
    )?;

    let memory_poke = Rc::clone(session);
    engine.register_library_fn(
        internal,
        "%memory-poke-unsigned",
        6..=6,
        move |context, args| {
            if let Err(value) = machine_id(context, &memory_poke, args[0])? {
                return Ok(value);
            }
            let space = context.to_symbol_name(args[1])?.to_owned();
            let address = match to_count(context, args[2])? {
                Ok(address) => address,
                Err(value) => return Ok(value),
            };
            let width = match to_u32(context, args[3])? {
                Ok(width) => width,
                Err(value) => return Ok(value),
            };
            let order = match parse_byte_order(context, args[4])? {
                Ok(order) => order,
                Err(value) => return Ok(value),
            };
            let value = match to_u128(context, args[5])? {
                Ok(value) => value,
                Err(value) => return Ok(value),
            };
            match memory_poke
                .borrow_mut()
                .poke_unsigned(&space, address, width, order, value)
            {
                Ok(()) => Ok(Value::boolean(true)),
                Err(error) => op_error_value(context, &error),
            }
        },
    )?;

    Ok(())
}
