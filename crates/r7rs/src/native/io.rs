//! Port and I/O primitives, datum read/write, file operations, and
//! system/process procedures.

use super::{collection::*, *};

pub(super) fn port_id(cx: &NativeContext<'_>, value: Value) -> Result<crate::port::PortId, Error> {
    cx.heap
        .port(value)
        .ok_or_else(|| type_error("port", value, cx.heap))
}

pub(super) fn optional_port(
    cx: &NativeContext<'_>,
    values: &[Value],
    input: bool,
) -> Result<crate::port::PortId, Error> {
    let value = values.first().copied().unwrap_or(cx.current_port(if input {
        "current-input-port"
    } else {
        "current-output-port"
    })?);
    port_id(cx, value)
}

pub(super) fn input_port_predicate(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    Ok(Value::boolean(
        cx.heap
            .port(a[0])
            .and_then(|id| cx.heap.ports_mut().properties(id).ok())
            .is_some_and(|p| p.0),
    ))
}
pub(super) fn output_port_predicate(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    Ok(Value::boolean(
        cx.heap
            .port(a[0])
            .and_then(|id| cx.heap.ports_mut().properties(id).ok())
            .is_some_and(|p| p.1),
    ))
}
pub(super) fn textual_port_predicate(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    Ok(Value::boolean(
        cx.heap
            .port(a[0])
            .and_then(|id| cx.heap.ports_mut().properties(id).ok())
            .is_some_and(|p| p.2),
    ))
}
pub(super) fn binary_port_predicate(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    Ok(Value::boolean(
        cx.heap
            .port(a[0])
            .and_then(|id| cx.heap.ports_mut().properties(id).ok())
            .is_some_and(|p| p.3),
    ))
}
pub(super) fn port_predicate(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    Ok(Value::boolean(cx.heap.port(a[0]).is_some()))
}
pub(super) fn input_port_open(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = port_id(cx, a[0])?;
    let p = cx.heap.ports_mut().properties(id)?;
    Ok(Value::boolean(p.0 && p.4))
}
pub(super) fn output_port_open(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = port_id(cx, a[0])?;
    let p = cx.heap.ports_mut().properties(id)?;
    Ok(Value::boolean(p.1 && p.4))
}

pub(super) fn open_input_string(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let value = string_argument(cx, a[0])?;
    cx.input_string_utf8(value)
}
pub(super) fn open_output_string(cx: &mut NativeContext<'_>, _: &[Value]) -> Result<Value, Error> {
    cx.output_string()
}
pub(super) fn get_output_string(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = port_id(cx, a[0])?;
    let value = cx.heap.ports_mut().text_output(id)?;
    cx.string_utf8(value)
}
pub(super) fn open_input_bytevector(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    cx.input_bytevector(bytevector_argument(cx, a[0])?)
}
pub(super) fn open_output_bytevector(
    cx: &mut NativeContext<'_>,
    _: &[Value],
) -> Result<Value, Error> {
    cx.output_bytevector()
}
pub(super) fn get_output_bytevector(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    let id = port_id(cx, a[0])?;
    let value = cx.heap.ports_mut().byte_output(id)?;
    cx.bytevector(value)
}
pub(super) fn close_port(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = port_id(cx, a[0])?;
    cx.heap.ports_mut().close(id)?;
    Ok(Value::unspecified())
}
pub(super) fn close_input_port(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = port_id(cx, a[0])?;
    if !cx.heap.ports_mut().properties(id)?.0 {
        return Err(Error::plain(ErrorKind::TypeError, "expected input port"));
    }
    cx.heap.ports_mut().close(id)?;
    Ok(Value::unspecified())
}
pub(super) fn close_output_port(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = port_id(cx, a[0])?;
    if !cx.heap.ports_mut().properties(id)?.1 {
        return Err(Error::plain(ErrorKind::TypeError, "expected output port"));
    }
    cx.heap.ports_mut().close(id)?;
    Ok(Value::unspecified())
}

pub(super) fn read_char(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = optional_port(cx, a, true)?;
    Ok(cx
        .heap
        .ports_mut()
        .read_char(id, false)?
        .map(Value::character)
        .unwrap_or(Value::eof()))
}
pub(super) fn peek_char(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = optional_port(cx, a, true)?;
    Ok(cx
        .heap
        .ports_mut()
        .read_char(id, true)?
        .map(Value::character)
        .unwrap_or(Value::eof()))
}
pub(super) fn read_u8(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = optional_port(cx, a, true)?;
    Ok(cx
        .heap
        .ports_mut()
        .read_u8(id, false)?
        .map(|v| Value::integer(v.into()))
        .unwrap_or(Value::eof()))
}
pub(super) fn peek_u8(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = optional_port(cx, a, true)?;
    Ok(cx
        .heap
        .ports_mut()
        .read_u8(id, true)?
        .map(|v| Value::integer(v.into()))
        .unwrap_or(Value::eof()))
}
pub(super) fn char_ready(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = optional_port(cx, a, true)?;
    Ok(Value::boolean(cx.heap.ports_mut().ready(id, true)?))
}
pub(super) fn u8_ready(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = optional_port(cx, a, true)?;
    Ok(Value::boolean(cx.heap.ports_mut().ready(id, false)?))
}

pub(super) fn read_line(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = optional_port(cx, a, true)?;
    let mut values = String::new();
    loop {
        match cx.heap.ports_mut().read_char(id, false)? {
            None if values.is_empty() => return Ok(Value::eof()),
            None => break,
            Some('\n') => break,
            Some('\r') => {
                if matches!(cx.heap.ports_mut().read_char(id, true)?, Some('\n')) {
                    let _ = cx.heap.ports_mut().read_char(id, false)?;
                }
                break;
            }
            Some(value) => values.push(value),
        }
    }
    cx.string_utf8(values)
}

pub(super) fn read_string(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let count = index(cx, a[0])?;
    let id = optional_port(cx, &a[1..], true)?;
    let mut output = String::new();
    for _ in 0..count {
        match cx.heap.ports_mut().read_char(id, false)? {
            Some(value) => output.push(value),
            None => break,
        }
    }
    if output.is_empty() {
        Ok(Value::eof())
    } else {
        cx.string_utf8(output)
    }
}

pub(super) fn read_bytevector(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let count = index(cx, a[0])?;
    let id = optional_port(cx, &a[1..], true)?;
    // The port may reach EOF immediately, so the requested upper bound is not
    // a sound allocation size. Grow only for bytes that were actually read.
    let mut output = Vec::new();
    for _ in 0..count {
        match cx.heap.ports_mut().read_u8(id, false)? {
            Some(value) => output.push(value),
            None => break,
        }
    }
    if output.is_empty() {
        Ok(Value::eof())
    } else {
        cx.bytevector(output)
    }
}

pub(super) fn read_bytevector_mut(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let target = a[0];
    let length = cx
        .heap
        .bytevector_len(target)
        .ok_or_else(|| type_error("bytevector", target, cx.heap))?;
    let port_start = usize::from(a.len() >= 2);
    let id = optional_port(
        cx,
        &a[port_start..port_start + (a.len() >= 2) as usize],
        true,
    )?;
    let start = if a.len() >= 3 { index(cx, a[2])? } else { 0 };
    let end = if a.len() == 4 {
        index(cx, a[3])?
    } else {
        length
    };
    if start > end || end > length {
        return Err(Error::plain(
            ErrorKind::RangeError,
            "invalid bytevector range",
        ));
    }
    let mut read = 0;
    for index in start..end {
        match cx.heap.ports_mut().read_u8(id, false)? {
            Some(value) => {
                if !cx.heap.bytevector_set(target, index, value) {
                    return Err(sequence_mutation_error(
                        cx,
                        cx.heap.bytevector_len(target),
                        index,
                        "bytevector",
                        target,
                    ));
                }
                read += 1;
            }
            None => break,
        }
    }
    if read == 0 {
        Ok(Value::eof())
    } else {
        cx.integer(i128::from(read))
    }
}

pub(super) fn write_char(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = optional_port(cx, &a[1..], false)?;
    let value = character(cx, a[0])?;
    cx.heap.ports_mut().write_char(id, value)?;
    Ok(Value::unspecified())
}
pub(super) fn newline(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = optional_port(cx, a, false)?;
    cx.heap.ports_mut().write_char(id, '\n')?;
    Ok(Value::unspecified())
}
pub(super) fn write_u8(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = optional_port(cx, &a[1..], false)?;
    let value = byte(cx, a[0])?;
    cx.heap.ports_mut().write_u8(id, value)?;
    Ok(Value::unspecified())
}
pub(super) fn write_string(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let len = cx
        .heap
        .string_len(a[0])
        .ok_or_else(|| type_error("string", a[0], cx.heap))?;
    let (id, start, end) = output_range(cx, &a[1..], len)?;
    let text = cx
        .heap
        .string_range(a[0], start, end)
        .ok_or_else(|| type_error("string", a[0], cx.heap))?
        .to_owned();
    for value in text.chars() {
        cx.heap.ports_mut().write_char(id, value)?;
    }
    Ok(Value::unspecified())
}
pub(super) fn write_bytevector(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let value = bytevector_argument(cx, a[0])?;
    let (id, start, end) = output_range(cx, &a[1..], value.len())?;
    for value in &value[start..end] {
        cx.heap.ports_mut().write_u8(id, *value)?;
    }
    Ok(Value::unspecified())
}
pub(super) fn output_range(
    cx: &NativeContext<'_>,
    args: &[Value],
    length: usize,
) -> Result<(crate::port::PortId, usize, usize), Error> {
    let port_count = usize::from(!args.is_empty());
    let id = optional_port(cx, &args[..port_count], false)?;
    let start = if args.len() >= 2 {
        index(cx, args[1])?
    } else {
        0
    };
    let end = if args.len() >= 3 {
        index(cx, args[2])?
    } else {
        length
    };
    if start > end || end > length {
        return Err(Error::plain(ErrorKind::RangeError, "invalid output range"));
    }
    Ok((id, start, end))
}
pub(super) fn flush_output_port(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = optional_port(cx, a, false)?;
    cx.heap.ports_mut().flush(id)?;
    Ok(Value::unspecified())
}
pub(super) fn eof_object(_: &mut NativeContext<'_>, _: &[Value]) -> Result<Value, Error> {
    Ok(Value::eof())
}
pub(super) fn eof_object_predicate(_: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    Ok(Value::boolean(a[0] == Value::eof()))
}
pub(super) fn read(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let id = optional_port(cx, a, true)?;
    let datum = cx
        .heap
        .ports_mut()
        .read_datum(id, &crate::EngineConfig::default())?;
    match datum {
        Some(datum) => materialize_datum(
            cx,
            &datum,
            datum.root(),
            &mut std::collections::HashMap::new(),
        ),
        None => Ok(Value::eof()),
    }
}

pub(super) fn materialize_datum(
    cx: &mut NativeContext<'_>,
    datum: &crate::Datum,
    reference: crate::DatumRef,
    values: &mut std::collections::HashMap<crate::DatumRef, Value>,
) -> Result<Value, Error> {
    let reference = datum
        .resolved_ref(reference)
        .ok_or_else(|| Error::plain(ErrorKind::InvalidDatum, "invalid datum graph"))?;
    if let Some(value) = values.get(&reference) {
        return Ok(*value);
    }
    match datum
        .kind(reference)
        .ok_or_else(|| Error::plain(ErrorKind::InvalidDatum, "invalid datum graph"))?
    {
        crate::DatumKind::Nil => Ok(Value::nil()),
        crate::DatumKind::Boolean(value) => Ok(Value::boolean(value)),
        crate::DatumKind::Character(value) => Ok(Value::character(value)),
        crate::DatumKind::String(value) => cx.string(value.chars()),
        crate::DatumKind::Symbol(value) => cx.intern_symbol(value),
        crate::DatumKind::Number(value) => match value {
            crate::Number::Real(crate::Real::ExactInteger(value)) => cx.integer(*value),
            crate::Number::Real(crate::Real::Inexact(value)) => Ok(Value::float(*value)),
            value => cx.alloc(Object::Number(Box::new(
                crate::number::RuntimeNumber::from_literal(*value),
            ))),
        },
        crate::DatumKind::Bytevector(value) => cx.bytevector(value.to_vec()),
        crate::DatumKind::Pair { car, cdr } => {
            let pair = cx.alloc(Object::Pair(Value::unspecified(), Value::unspecified()))?;
            values.insert(reference, pair);
            let car = materialize_datum(cx, datum, car, values)?;
            let cdr = materialize_datum(cx, datum, cdr, values)?;
            let _ = cx.heap.set_pair_car(pair, car);
            let _ = cx.heap.set_pair_cdr(pair, cdr);
            Ok(pair)
        }
        crate::DatumKind::Vector(value) => {
            let vector = cx.vector(vec![Value::unspecified(); value.len()])?;
            values.insert(reference, vector);
            for (index, child) in value.iter().enumerate() {
                let child = materialize_datum(cx, datum, *child, values)?;
                let _ = cx.heap.vector_set(vector, index, child);
            }
            Ok(vector)
        }
    }
}
pub(super) fn write(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    write_with_mode(cx, a, crate::printer::RuntimeWriteMode::Write)
}
pub(super) fn write_shared(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    write_with_mode(cx, a, crate::printer::RuntimeWriteMode::Shared)
}
pub(super) fn write_simple(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    write_with_mode(cx, a, crate::printer::RuntimeWriteMode::Simple)
}
pub(super) fn display(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    write_with_mode(cx, a, crate::printer::RuntimeWriteMode::Display)
}
pub(super) fn write_with_mode(
    cx: &mut NativeContext<'_>,
    a: &[Value],
    mode: crate::printer::RuntimeWriteMode,
) -> Result<Value, Error> {
    let output = crate::printer::write_value(cx.heap, a[0], mode)?;
    let id = optional_port(cx, &a[1..], false)?;
    for value in output.chars() {
        cx.heap.ports_mut().write_char(id, value)?;
    }
    Ok(Value::unspecified())
}
pub(super) fn file_path(cx: &NativeContext<'_>, value: Value) -> Result<String, Error> {
    string_argument(cx, value)
}
pub(super) fn open_input_file(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let path = file_path(cx, a[0])?;
    let id = cx.heap.open_file(&path, true, false)?;
    cx.port(id)
}
pub(super) fn open_output_file(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let path = file_path(cx, a[0])?;
    let id = cx.heap.open_file(&path, false, false)?;
    cx.port(id)
}
pub(super) fn open_binary_input_file(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    let path = file_path(cx, a[0])?;
    let id = cx.heap.open_file(&path, true, true)?;
    cx.port(id)
}
pub(super) fn open_binary_output_file(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    let path = file_path(cx, a[0])?;
    let id = cx.heap.open_file(&path, false, true)?;
    cx.port(id)
}
pub(super) fn file_exists(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let path = file_path(cx, a[0])?;
    Ok(Value::boolean(cx.heap.file_exists(&path)?))
}
pub(super) fn delete_file(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    let path = file_path(cx, a[0])?;
    cx.heap.delete_file(&path)?;
    Ok(Value::unspecified())
}
pub(super) fn string_list(
    cx: &mut NativeContext<'_>,
    values: impl IntoIterator<Item = String>,
    immutable_strings: bool,
) -> Result<Value, Error> {
    let mut output = Value::nil();
    let values: Vec<_> = values.into_iter().collect();
    for value in values.into_iter().rev() {
        let value = cx.string(value.chars())?;
        if immutable_strings {
            cx.heap.make_immutable(value);
        }
        output = cx.pair(value, output)?;
    }
    Ok(output)
}
pub(super) fn command_line(cx: &mut NativeContext<'_>, _: &[Value]) -> Result<Value, Error> {
    let values = cx.heap.process_context()?.command_line().map_err(|error| {
        Error::plain(
            ErrorKind::RuntimeError,
            format!("process capability failed: {error}"),
        )
    })?;
    string_list(cx, values, true)
}
pub(super) fn get_environment_variable(
    cx: &mut NativeContext<'_>,
    a: &[Value],
) -> Result<Value, Error> {
    let name = file_path(cx, a[0])?;
    let value = cx
        .heap
        .process_context()?
        .environment_variable(&name)
        .map_err(|error| {
            Error::plain(
                ErrorKind::RuntimeError,
                format!("process capability failed: {error}"),
            )
        })?;
    match value {
        Some(value) => {
            let value = cx.string(value.chars())?;
            cx.heap.make_immutable(value);
            Ok(value)
        }
        None => Ok(Value::boolean(false)),
    }
}
pub(super) fn get_environment_variables(
    cx: &mut NativeContext<'_>,
    _: &[Value],
) -> Result<Value, Error> {
    let values = cx
        .heap
        .process_context()?
        .environment_variables()
        .map_err(|error| {
            Error::plain(
                ErrorKind::RuntimeError,
                format!("process capability failed: {error}"),
            )
        })?;
    let mut output = Value::nil();
    for (name, value) in values.into_iter().rev() {
        let name = cx.string(name.chars())?;
        let value = cx.string(value.chars())?;
        let pair = cx.pair(name, value)?;
        cx.heap.make_immutable(name);
        cx.heap.make_immutable(value);
        cx.heap.make_immutable(pair);
        output = cx.pair(pair, output)?;
        cx.heap.make_immutable(output);
    }
    Ok(output)
}
pub(super) fn current_second(cx: &mut NativeContext<'_>, _: &[Value]) -> Result<Value, Error> {
    let value = cx.heap.clock()?.current_second().map_err(|error| {
        Error::plain(
            ErrorKind::RuntimeError,
            format!("clock capability failed: {error}"),
        )
    })?;
    Ok(Value::float(value))
}
pub(super) fn current_jiffy(cx: &mut NativeContext<'_>, _: &[Value]) -> Result<Value, Error> {
    let value = cx.heap.clock()?.current_jiffy().map_err(|error| {
        Error::plain(
            ErrorKind::RuntimeError,
            format!("clock capability failed: {error}"),
        )
    })?;
    cx.integer(i128::from(value))
}
pub(super) fn jiffies_per_second(cx: &mut NativeContext<'_>, _: &[Value]) -> Result<Value, Error> {
    let value = cx.heap.clock()?.jiffies_per_second().map_err(|error| {
        Error::plain(
            ErrorKind::RuntimeError,
            format!("clock capability failed: {error}"),
        )
    })?;
    cx.integer(i128::from(value))
}
pub(super) fn exit(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    request_exit(cx, a, false)
}
pub(super) fn emergency_exit(cx: &mut NativeContext<'_>, a: &[Value]) -> Result<Value, Error> {
    request_exit(cx, a, true)
}
pub(super) fn request_exit(
    cx: &mut NativeContext<'_>,
    a: &[Value],
    emergency: bool,
) -> Result<Value, Error> {
    let value = match a.first().copied() {
        None => Some(0),
        Some(value) if value == Value::boolean(true) => Some(0),
        Some(value) if value == Value::boolean(false) => Some(1),
        Some(value) => Some(i64::try_from(exact_integer(cx, value)?).map_err(|_| {
            Error::plain(
                ErrorKind::RangeError,
                "exit code exceeds the supported i64 host range",
            )
        })?),
    };
    // Validate the capability now, but defer notification until the VM has
    // performed the required dynamic-wind cleanup.
    let _ = cx.heap.process_context()?;
    cx.heap
        .request_exit(crate::ExitStatus::new(value, emergency));
    Ok(Value::unspecified())
}
