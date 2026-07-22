//! Trace schema discovery, continuous collection, and one-shot event waiting.

use std::{cell::RefCell, rc::Rc};

use common::{
    OwnedTraceCall, OwnedTraceDeviceEvent, OwnedTraceEvent, OwnedTraceField, OwnedTraceValue,
    ProcessorSnapshot, TraceCallInterface, TraceContext, TraceEventClass, TraceFieldDescriptor,
    tracing::{ApplicationTraceEnvelope, TraceFailure},
};
use r7rs::{Engine, Error, LibraryName, NativeContext, Value, ValueKind};

use super::support::{
    artifact_alist, error_value, inspected_integer, machine_id, make_alist, make_list,
    op_error_value, to_count, written_len,
};
use crate::session::{
    AutomationSession, OpError,
    trace::{
        CLASS_SCHEMAS, ENVELOPE_FIELDS, FilterScalar, FilterSpec, SchemaType, access_operation,
        access_width_bits, call_phase_name, interrupt_action_name, interrupt_kind_name,
        space_class_name,
    },
};

/// Parses a scalar filter value, or returns a tagged argument error value.
fn parse_scalar(
    context: &mut NativeContext,
    value: Value,
) -> Result<Result<FilterScalar, Value>, Error> {
    match context.kind(value) {
        ValueKind::Symbol => {
            let name = context.to_symbol_name(value)?.to_owned();
            Ok(Ok(FilterScalar::Symbol(name)))
        }
        ValueKind::Boolean => Ok(Ok(FilterScalar::Boolean(value == Value::boolean(true)))),
        ValueKind::Fixnum | ValueKind::Number => {
            let integer = context.to_i128(value)?;
            Ok(Ok(FilterScalar::Integer(integer)))
        }
        ValueKind::Pair => parse_range(context, value),
        ValueKind::Bytevector => {
            let bytes = context.to_bytes(value)?.to_vec();
            Ok(Ok(FilterScalar::Bytes(bytes)))
        }
        _ => Ok(Err(error_value(
            context,
            "neetan/argument",
            "trace filter value must be a symbol, integer, boolean, bytevector, or range",
        )?)),
    }
}

/// Parses an `(range minimum maximum)` list into a range scalar.
fn parse_range(
    context: &mut NativeContext,
    value: Value,
) -> Result<Result<FilterScalar, Value>, Error> {
    let malformed = |context: &mut NativeContext| {
        error_value(
            context,
            "neetan/argument",
            "trace filter range must be (range minimum maximum) with two integers",
        )
    };
    let Ok(items) = context.to_list(value) else {
        return Ok(Err(malformed(context)?));
    };
    if items.len() != 3 || context.kind(items[0]) != ValueKind::Symbol {
        return Ok(Err(malformed(context)?));
    }
    if context.to_symbol_name(items[0])? != "range" {
        return Ok(Err(malformed(context)?));
    }
    match (context.to_i128(items[1]), context.to_i128(items[2])) {
        (Ok(low), Ok(high)) => Ok(Ok(FilterScalar::Range(low, high))),
        _ => Ok(Err(malformed(context)?)),
    }
}

/// Parses a `(key . value)` alist into filter key and scalar pairs.
fn parse_alist(
    context: &mut NativeContext,
    value: Value,
) -> Result<Result<Vec<(String, FilterScalar)>, Value>, Error> {
    match context.kind(value) {
        ValueKind::Nil => Ok(Ok(Vec::new())),
        ValueKind::Pair => {
            let Ok(items) = context.to_list(value) else {
                return Ok(Err(error_value(
                    context,
                    "neetan/argument",
                    "trace filter must be a proper association list",
                )?));
            };
            let mut entries = Vec::with_capacity(items.len());
            for item in items {
                if context.kind(item) != ValueKind::Pair {
                    return Ok(Err(error_value(
                        context,
                        "neetan/argument",
                        "trace filter entry must be a (key . value) pair",
                    )?));
                }
                let (key, tail) = context.to_pair(item)?;
                if context.kind(key) != ValueKind::Symbol {
                    return Ok(Err(error_value(
                        context,
                        "neetan/argument",
                        "trace filter key must be a symbol",
                    )?));
                }
                let name = context.to_symbol_name(key)?.to_owned();
                let scalar = match parse_scalar(context, tail)? {
                    Ok(scalar) => scalar,
                    Err(error) => return Ok(Err(error)),
                };
                entries.push((name, scalar));
            }
            Ok(Ok(entries))
        }
        _ => Ok(Err(error_value(
            context,
            "neetan/argument",
            "trace filter must be an association list",
        )?)),
    }
}

/// The two sections of a parsed `data` block: the direct data constraints and an
/// optional nested `fields` sub-alist of provider-specific field constraints.
type DataSections = (
    Vec<(String, FilterScalar)>,
    Option<Vec<(String, FilterScalar)>>,
);

/// Parses the nested `data` alist, splitting out an optional `fields` sub-alist.
fn parse_data(
    context: &mut NativeContext,
    value: Value,
) -> Result<Result<DataSections, Value>, Error> {
    let items = match context.kind(value) {
        ValueKind::Nil => Vec::new(),
        ValueKind::Pair => {
            let Ok(items) = context.to_list(value) else {
                return Ok(Err(error_value(
                    context,
                    "neetan/argument",
                    "trace filter data must be a proper association list",
                )?));
            };
            items
        }
        _ => {
            return Ok(Err(error_value(
                context,
                "neetan/argument",
                "trace filter data must be an association list",
            )?));
        }
    };
    let mut data = Vec::new();
    let mut fields = None;
    for item in items {
        if context.kind(item) != ValueKind::Pair {
            return Ok(Err(error_value(
                context,
                "neetan/argument",
                "trace filter data entry must be a (key . value) pair",
            )?));
        }
        let (key, tail) = context.to_pair(item)?;
        if context.kind(key) != ValueKind::Symbol {
            return Ok(Err(error_value(
                context,
                "neetan/argument",
                "trace filter data key must be a symbol",
            )?));
        }
        let name = context.to_symbol_name(key)?.to_owned();
        if name == "fields" {
            match parse_alist(context, tail)? {
                Ok(sub) => fields = Some(sub),
                Err(error) => return Ok(Err(error)),
            }
        } else {
            let scalar = match parse_scalar(context, tail)? {
                Ok(scalar) => scalar,
                Err(error) => return Ok(Err(error)),
            };
            data.push((name, scalar));
        }
    }
    Ok(Ok((data, fields)))
}

/// Parses a full declarative filter value into a filter specification.
fn parse_filter(
    context: &mut NativeContext,
    value: Value,
) -> Result<Result<FilterSpec, Value>, Error> {
    let mut top = Vec::new();
    let mut data = None;
    let mut fields = None;
    let items = match context.kind(value) {
        ValueKind::Nil => Vec::new(),
        ValueKind::Pair => match context.to_list(value) {
            Ok(items) => items,
            Err(_) => {
                return Ok(Err(error_value(
                    context,
                    "neetan/argument",
                    "trace filter must be a proper association list",
                )?));
            }
        },
        _ => {
            return Ok(Err(error_value(
                context,
                "neetan/argument",
                "trace filter must be an association list",
            )?));
        }
    };
    for item in items {
        if context.kind(item) != ValueKind::Pair {
            return Ok(Err(error_value(
                context,
                "neetan/argument",
                "trace filter entry must be a (key . value) pair",
            )?));
        }
        let (key, tail) = context.to_pair(item)?;
        if context.kind(key) != ValueKind::Symbol {
            return Ok(Err(error_value(
                context,
                "neetan/argument",
                "trace filter key must be a symbol",
            )?));
        }
        let name = context.to_symbol_name(key)?.to_owned();
        if name == "data" {
            match parse_data(context, tail)? {
                Ok((sub_data, sub_fields)) => {
                    data = Some(sub_data);
                    fields = sub_fields;
                }
                Err(error) => return Ok(Err(error)),
            }
        } else {
            let scalar = match parse_scalar(context, tail)? {
                Ok(scalar) => scalar,
                Err(error) => return Ok(Err(error)),
            };
            top.push((name, scalar));
        }
    }
    Ok(Ok(FilterSpec { top, data, fields }))
}

/// Interns a stable identifier symbol.
fn symbol(context: &mut NativeContext, name: &str) -> Result<Value, Error> {
    context.intern_symbol(name)
}

/// Returns the stable class name of an owned event.
fn owned_class_name(event: &OwnedTraceEvent) -> &'static str {
    match event {
        OwnedTraceEvent::Access(_) => TraceEventClass::Access.as_str(),
        OwnedTraceEvent::Interrupt(_) => TraceEventClass::Interrupt.as_str(),
        OwnedTraceEvent::Scheduled { .. } => TraceEventClass::Scheduled.as_str(),
        OwnedTraceEvent::Presentation(_) => TraceEventClass::Presentation.as_str(),
        OwnedTraceEvent::Device(_) => TraceEventClass::Device.as_str(),
        OwnedTraceEvent::Call(_) => TraceEventClass::Call.as_str(),
        _ => "unknown",
    }
}

/// Marshals an owned trace field value.
fn field_value(context: &mut NativeContext, value: &OwnedTraceValue) -> Result<Value, Error> {
    match value {
        OwnedTraceValue::Unsigned(number) => context.integer(i128::from(*number)),
        OwnedTraceValue::Signed(number) => context.integer(i128::from(*number)),
        OwnedTraceValue::Bool(flag) => Ok(Value::boolean(*flag)),
        OwnedTraceValue::Bytes(bytes) => context.bytevector(bytes.clone()),
        OwnedTraceValue::Text(text) => context.string_utf8(text.clone()),
        OwnedTraceValue::Symbol(name) => symbol(context, name),
        OwnedTraceValue::U16List(elements) => {
            let mut values = Vec::with_capacity(elements.len());
            for element in elements {
                values.push(context.integer(i128::from(*element))?);
            }
            make_list(context, values)
        }
        _ => Ok(Value::boolean(false)),
    }
}

/// Marshals a list of owned trace fields into an association list.
fn fields_value(context: &mut NativeContext, fields: &[OwnedTraceField]) -> Result<Value, Error> {
    let mut entries = Vec::with_capacity(fields.len());
    for field in fields {
        let value = field_value(context, &field.value)?;
        entries.push((field.name, value));
    }
    make_alist(context, entries)
}

/// Marshals a call interface into a `((kind . ...) (value . ...))` alist.
fn interface_value(
    context: &mut NativeContext,
    interface: TraceCallInterface,
) -> Result<Value, Error> {
    let (kind, value) = match interface {
        TraceCallInterface::Interrupt(vector) => {
            ("interrupt", context.integer(i128::from(vector))?)
        }
        TraceCallInterface::EntryPoint(address) => {
            ("entry-point", context.integer(i128::from(address))?)
        }
        TraceCallInterface::Named(name) => ("named", symbol(context, name)?),
        _ => ("unknown", Value::boolean(false)),
    };
    let kind_symbol = symbol(context, kind)?;
    make_alist(context, vec![("kind", kind_symbol), ("value", value)])
}

/// Marshals the class-specific `data` alist for an owned event.
fn data_value(context: &mut NativeContext, event: &OwnedTraceEvent) -> Result<Value, Error> {
    match event {
        OwnedTraceEvent::Access(access) => {
            let space = symbol(context, access.space.id)?;
            let space_class = symbol(context, space_class_name(access.space.class))?;
            let operation = symbol(context, access_operation(access.kind))?;
            let address = context.integer(i128::from(access.address))?;
            let width = context.integer(access_width_bits(access.width))?;
            let value = match access.value {
                Some(value) => context.integer(i128::from(value))?,
                None => Value::boolean(false),
            };
            make_alist(
                context,
                vec![
                    ("space", space),
                    ("space-class", space_class),
                    ("operation", operation),
                    ("address", address),
                    ("width", width),
                    ("value", value),
                    ("handled?", Value::boolean(access.handled)),
                ],
            )
        }
        OwnedTraceEvent::Interrupt(interrupt) => {
            let controller = symbol(context, interrupt.controller)?;
            let kind = symbol(context, interrupt_kind_name(interrupt.kind))?;
            let line = match interrupt.line {
                Some(line) => context.integer(i128::from(line))?,
                None => Value::boolean(false),
            };
            let action = symbol(context, interrupt_action_name(interrupt.action))?;
            let vector = match interrupt.vector {
                Some(vector) => context.integer(i128::from(vector))?,
                None => Value::boolean(false),
            };
            make_alist(
                context,
                vec![
                    ("controller", controller),
                    ("interrupt-kind", kind),
                    ("line", line),
                    ("action", action),
                    ("vector", vector),
                ],
            )
        }
        OwnedTraceEvent::Scheduled { event, fire_tick } => {
            let event_symbol = symbol(context, event)?;
            let fire_tick = context.integer(i128::from(*fire_tick))?;
            make_alist(
                context,
                vec![("event", event_symbol), ("fire-tick", fire_tick)],
            )
        }
        OwnedTraceEvent::Presentation(presentation) => {
            let display = symbol(context, presentation.display)?;
            let frame = context.integer(i128::from(presentation.frame))?;
            let width = context.integer(i128::from(presentation.width))?;
            let height = context.integer(i128::from(presentation.height))?;
            make_alist(
                context,
                vec![
                    ("display", display),
                    ("frame", frame),
                    ("width", width),
                    ("height", height),
                ],
            )
        }
        OwnedTraceEvent::Device(OwnedTraceDeviceEvent {
            device,
            action,
            fields,
        }) => {
            let device_symbol = symbol(context, device)?;
            let action_symbol = symbol(context, action)?;
            let fields_alist = fields_value(context, fields)?;
            make_alist(
                context,
                vec![
                    ("device", device_symbol),
                    ("action", action_symbol),
                    ("fields", fields_alist),
                ],
            )
        }
        OwnedTraceEvent::Call(OwnedTraceCall {
            provider,
            interface,
            phase,
            fields,
        }) => {
            let provider_symbol = symbol(context, provider)?;
            let interface_alist = interface_value(context, *interface)?;
            let phase_symbol = symbol(context, call_phase_name(*phase))?;
            let fields_alist = fields_value(context, fields)?;
            make_alist(
                context,
                vec![
                    ("provider", provider_symbol),
                    ("interface", interface_alist),
                    ("phase", phase_symbol),
                    ("fields", fields_alist),
                ],
            )
        }
        _ => make_alist(context, Vec::new()),
    }
}

/// Marshals a trace context clock rate into `#f` or a rate alist.
fn clock_rate_value(
    context: &mut NativeContext,
    trace_context: &TraceContext,
) -> Result<Value, Error> {
    match trace_context.clock_rate {
        Some(rate) => {
            let numerator = context.integer(i128::from(rate.numerator))?;
            let denominator = context.integer(i128::from(rate.denominator.get()))?;
            make_alist(
                context,
                vec![("numerator", numerator), ("denominator", denominator)],
            )
        }
        None => Ok(Value::boolean(false)),
    }
}

/// Marshals an entry snapshot into `#f` or a `((processor . registers))` alist.
///
/// The registers use the same names and values as `register-ref`, captured at
/// HLE dispatch entry. Events without an armed snapshot marshal to `#f`.
fn snapshot_value(
    context: &mut NativeContext,
    snapshot: &Option<ProcessorSnapshot>,
) -> Result<Value, Error> {
    let Some(snapshot) = snapshot else {
        return Ok(Value::boolean(false));
    };
    let mut register_entries: Vec<(&str, Value)> = Vec::with_capacity(snapshot.registers.len());
    for register in &snapshot.registers {
        let value = inspected_integer(context, register.value)?;
        register_entries.push((register.name, value));
    }
    let registers = make_alist(context, register_entries)?;
    make_alist(context, vec![(snapshot.processor, registers)])
}

/// Marshals one owned trace envelope into a normalized event alist.
fn envelope_value(
    context: &mut NativeContext,
    envelope: &ApplicationTraceEnvelope,
) -> Result<Value, Error> {
    let schema_version = context.integer(i128::from(envelope.schema_version))?;
    let sequence = context.integer(i128::from(envelope.sequence))?;
    let epoch = context.integer(i128::from(envelope.epoch))?;
    let tick = context.integer(i128::from(envelope.context.tick))?;
    let source = symbol(context, envelope.context.source)?;
    let clock_domain = symbol(context, envelope.context.clock_domain)?;
    let clock_cycle = context.integer(i128::from(envelope.context.clock_cycle))?;
    let clock_rate = clock_rate_value(context, &envelope.context)?;
    let class = symbol(context, owned_class_name(&envelope.event))?;
    let data = data_value(context, &envelope.event)?;
    let snapshot = snapshot_value(context, &envelope.snapshot)?;
    make_alist(
        context,
        vec![
            ("schema-version", schema_version),
            ("sequence", sequence),
            ("epoch", epoch),
            ("tick", tick),
            ("source", source),
            ("clock-domain", clock_domain),
            ("clock-cycle", clock_cycle),
            ("clock-rate", clock_rate),
            ("class", class),
            ("data", data),
            ("snapshot", snapshot),
        ],
    )
}

/// Marshals a sticky trace failure into `#f` or a failure alist.
fn failure_value(
    context: &mut NativeContext,
    failure: Option<TraceFailure>,
) -> Result<Value, Error> {
    let Some(failure) = failure else {
        return Ok(Value::boolean(false));
    };
    match failure {
        TraceFailure::QueueOverflow {
            event_capacity,
            byte_capacity,
        } => {
            let reason = symbol(context, "queue-overflow")?;
            let events = context.integer(event_capacity.get() as i128)?;
            let bytes = context.integer(byte_capacity.get() as i128)?;
            make_alist(
                context,
                vec![
                    ("reason", reason),
                    ("event-capacity", events),
                    ("byte-capacity", bytes),
                ],
            )
        }
        TraceFailure::EventPayloadTooLarge { capacity } => {
            let reason = symbol(context, "event-payload-too-large")?;
            let bytes = context.integer(capacity.get() as i128)?;
            make_alist(
                context,
                vec![("reason", reason), ("event-payload-capacity", bytes)],
            )
        }
        TraceFailure::SequenceExhausted => {
            let reason = symbol(context, "sequence-exhausted")?;
            make_alist(context, vec![("reason", reason)])
        }
    }
}

/// Marshals a list of provider-specific field descriptors into a list of
/// `((name . sym) (type . sym) (range . bool))` alists.
fn descriptor_list(
    context: &mut NativeContext,
    descriptors: &[TraceFieldDescriptor],
) -> Result<Value, Error> {
    let mut entries = Vec::with_capacity(descriptors.len());
    for descriptor in descriptors {
        let name = symbol(context, descriptor.name)?;
        let type_symbol = symbol(
            context,
            SchemaType::from_field_type(descriptor.value_type).name(),
        )?;
        let entry = make_alist(
            context,
            vec![
                ("name", name),
                ("type", type_symbol),
                ("range", Value::boolean(descriptor.range)),
            ],
        )?;
        entries.push(entry);
    }
    make_list(context, entries)
}

/// Builds the `trace-schema` descriptor alist.
fn schema_value(
    context: &mut NativeContext,
    session: &Rc<RefCell<AutomationSession>>,
) -> Result<Value, Error> {
    let parts = match session.borrow_mut().trace_schema_parts() {
        Ok(parts) => parts,
        Err(error) => return op_error_value(context, &error),
    };

    let schema_version = context.integer(i128::from(parts.schema_version))?;
    let event_capacity = context.integer(parts.event_capacity as i128)?;
    let byte_capacity = context.integer(parts.byte_capacity as i128)?;
    let event_payload_capacity = context.integer(parts.event_payload_capacity as i128)?;
    let queue_limits = make_alist(
        context,
        vec![
            ("event-capacity", event_capacity),
            ("byte-capacity", byte_capacity),
            ("event-payload-capacity", event_payload_capacity),
        ],
    )?;

    let mut envelope_entries = Vec::with_capacity(ENVELOPE_FIELDS.len());
    for field in ENVELOPE_FIELDS {
        let name = symbol(context, field.name)?;
        let ty = symbol(context, field.ty.name())?;
        let entry = make_alist(
            context,
            vec![
                ("name", name),
                ("type", ty),
                ("range", Value::boolean(field.range)),
            ],
        )?;
        envelope_entries.push(entry);
    }
    let envelope_fields = make_list(context, envelope_entries)?;

    let mut class_entries = Vec::with_capacity(CLASS_SCHEMAS.len());
    for schema in CLASS_SCHEMAS {
        let class = symbol(context, schema.class.as_str())?;
        let mut field_entries = Vec::with_capacity(schema.fields.len());
        for field in schema.fields {
            let name = symbol(context, field.name)?;
            let ty = symbol(context, field.ty.name())?;
            let entry = make_alist(
                context,
                vec![
                    ("name", name),
                    ("type", ty),
                    ("range", Value::boolean(field.range)),
                ],
            )?;
            field_entries.push(entry);
        }
        let fields = make_list(context, field_entries)?;
        let entry = make_alist(context, vec![("class", class), ("fields", fields)])?;
        class_entries.push(entry);
    }
    let classes = make_list(context, class_entries)?;

    let emitted = parts.catalog.classes();
    let mut supported = Vec::new();
    for class in TraceEventClass::ALL {
        if emitted.contains(class) {
            supported.push(symbol(context, class.as_str())?);
        }
    }
    let supported_classes = make_list(context, supported)?;

    let mut space_symbols = Vec::with_capacity(parts.address_spaces.len());
    for space in &parts.address_spaces {
        space_symbols.push(symbol(context, space)?);
    }
    let address_spaces = make_list(context, space_symbols)?;

    let mut controller_symbols = Vec::with_capacity(parts.catalog.controllers.len());
    for controller in parts.catalog.controllers {
        controller_symbols.push(symbol(context, controller)?);
    }
    let controllers = make_list(context, controller_symbols)?;

    let mut scheduled_symbols = Vec::with_capacity(parts.catalog.scheduled.len());
    for scheduled in parts.catalog.scheduled {
        scheduled_symbols.push(symbol(context, scheduled)?);
    }
    let scheduled = make_list(context, scheduled_symbols)?;

    let mut device_entries = Vec::with_capacity(parts.catalog.devices.len());
    for device in parts.catalog.devices {
        let device_symbol = symbol(context, device.device)?;
        let mut action_entries = Vec::with_capacity(device.actions.len());
        for action in device.actions {
            let action_symbol = symbol(context, action.action)?;
            let fields = descriptor_list(context, action.fields)?;
            let entry = make_alist(context, vec![("action", action_symbol), ("fields", fields)])?;
            action_entries.push(entry);
        }
        let actions = make_list(context, action_entries)?;
        let entry = make_alist(
            context,
            vec![("device", device_symbol), ("actions", actions)],
        )?;
        device_entries.push(entry);
    }
    let devices = make_list(context, device_entries)?;

    let mut provider_entries = Vec::with_capacity(parts.catalog.providers.len());
    for provider in parts.catalog.providers {
        let provider_symbol = symbol(context, provider.provider)?;
        let mut interface_symbols = Vec::with_capacity(provider.named_interfaces.len());
        for interface in provider.named_interfaces {
            interface_symbols.push(symbol(context, interface)?);
        }
        let interfaces = make_list(context, interface_symbols)?;
        let call_fields = descriptor_list(context, provider.call_fields)?;
        let entry = make_alist(
            context,
            vec![
                ("provider", provider_symbol),
                ("named-interfaces", interfaces),
                ("call-fields", call_fields),
            ],
        )?;
        provider_entries.push(entry);
    }
    let providers = make_list(context, provider_entries)?;

    make_alist(
        context,
        vec![
            ("schema-version", schema_version),
            ("queue-limits", queue_limits),
            ("envelope-fields", envelope_fields),
            ("classes", classes),
            ("supported-classes", supported_classes),
            ("address-spaces", address_spaces),
            ("controllers", controllers),
            ("scheduled", scheduled),
            ("devices", devices),
            ("providers", providers),
        ],
    )
}

/// Parses a snapshot-processor argument into a list of processor identifiers.
///
/// The argument is a list of symbols, or `#f` for no snapshot request.
fn parse_snapshot_processors(
    context: &mut NativeContext,
    value: Value,
) -> Result<Result<Vec<String>, Value>, Error> {
    match context.kind(value) {
        ValueKind::Boolean if value == Value::boolean(false) => Ok(Ok(Vec::new())),
        ValueKind::Nil => Ok(Ok(Vec::new())),
        ValueKind::Pair => {
            let Ok(items) = context.to_list(value) else {
                return Ok(Err(error_value(
                    context,
                    "neetan/argument",
                    "snapshot must be a proper list of processor symbols",
                )?));
            };
            let mut processors = Vec::with_capacity(items.len());
            for item in items {
                if context.kind(item) != ValueKind::Symbol {
                    return Ok(Err(error_value(
                        context,
                        "neetan/argument",
                        "snapshot processor must be a symbol",
                    )?));
                }
                processors.push(context.to_symbol_name(item)?.to_owned());
            }
            Ok(Ok(processors))
        }
        _ => Ok(Err(error_value(
            context,
            "neetan/argument",
            "snapshot must be a list of processor symbols",
        )?)),
    }
}

/// Registers the tracing natives.
pub(super) fn register_trace_natives(
    engine: &mut Engine,
    session: &Rc<RefCell<AutomationSession>>,
    internal: &LibraryName,
) -> Result<(), Error> {
    let schema = Rc::clone(session);
    engine.register_library_fn(internal, "%trace-schema", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &schema, args[0])? {
            return Ok(value);
        }
        schema_value(context, &schema)
    })?;

    let start = Rc::clone(session);
    engine.register_library_fn(internal, "%trace-start", 2..=2, move |context, args| {
        if let Err(value) = machine_id(context, &start, args[0])? {
            return Ok(value);
        }
        let spec = match parse_filter(context, args[1])? {
            Ok(spec) => spec,
            Err(value) => return Ok(value),
        };
        match start.borrow_mut().trace_start(spec) {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let active = Rc::clone(session);
    engine.register_library_fn(internal, "%trace-active?", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &active, args[0])? {
            return Ok(value);
        }
        Ok(Value::boolean(active.borrow().trace_active()))
    })?;

    let stop = Rc::clone(session);
    engine.register_library_fn(internal, "%trace-stop", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &stop, args[0])? {
            return Ok(value);
        }
        match stop.borrow().trace_stop() {
            Ok(()) => Ok(Value::boolean(true)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let drain = Rc::clone(session);
    engine.register_library_fn(internal, "%trace-drain", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &drain, args[0])? {
            return Ok(value);
        }
        let events = match drain.borrow().trace_drain() {
            Ok(events) => events,
            Err(error) => return op_error_value(context, &error),
        };
        let mut values = Vec::with_capacity(events.len());
        for envelope in &events {
            values.push(envelope_value(context, envelope)?);
        }
        make_list(context, values)
    })?;

    let failure = Rc::clone(session);
    engine.register_library_fn(internal, "%trace-failure", 1..=1, move |context, args| {
        if let Err(value) = machine_id(context, &failure, args[0])? {
            return Ok(value);
        }
        match failure.borrow().trace_failure() {
            Ok(failure) => failure_value(context, failure),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let save_trace = Rc::clone(session);
    engine.register_library_fn(internal, "%save-trace", 2..=2, move |context, args| {
        if let Err(value) = machine_id(context, &save_trace, args[0])? {
            return Ok(value);
        }
        let path = context.to_str(args[1])?.to_owned();
        let events = match save_trace.borrow().trace_snapshot() {
            Ok(events) => events,
            Err(error) => return op_error_value(context, &error),
        };
        // Render each event with the same alist marshalling as trace-drain!, then
        // to its `write` external form, so the artifact reads back with `read`.
        let mut text = String::new();
        for envelope in &events {
            let value = envelope_value(context, envelope)?;
            text.push_str(&context.write_to_string(value)?);
            text.push('\n');
        }
        match save_trace
            .borrow_mut()
            .write_artifact(&path, text.as_bytes())
        {
            Ok(written) => artifact_alist(context, &path, Some(written_len(&written))),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    let arm = Rc::clone(session);
    engine.register_library_fn(internal, "%trace-arm", 8..=8, move |context, args| {
        if let Err(value) = machine_id(context, &arm, args[0])? {
            return Ok(value);
        }
        let capture = match parse_filter(context, args[1])? {
            Ok(spec) => spec,
            Err(value) => return Ok(value),
        };
        let trigger = match parse_filter(context, args[2])? {
            Ok(spec) => spec,
            Err(value) => return Ok(value),
        };
        let before = match to_count(context, args[3])? {
            Ok(value) => value,
            Err(value) => return Ok(value),
        };
        let after = match to_count(context, args[4])? {
            Ok(value) => value,
            Err(value) => return Ok(value),
        };
        let path = context.to_str(args[5])?.to_owned();
        let max_frames = match to_count(context, args[6])? {
            Ok(value) => value,
            Err(value) => return Ok(value),
        };
        let max_ticks = match to_count(context, args[7])? {
            Ok(value) => value,
            Err(value) => return Ok(value),
        };
        if let Err(error) = arm.borrow().validate_artifact_path(&path) {
            return op_error_value(context, &error);
        }
        let outcome = match arm
            .borrow_mut()
            .trace_capture(capture, trigger, before, after, max_frames, max_ticks)
        {
            Ok(outcome) => outcome,
            Err(error) => return op_error_value(context, &error),
        };
        // The artifact uses the same one-datum-per-line rendering as
        // save-trace!, and is only written once the trigger has fired. A
        // capture ended by an overflow still writes the partially retained
        // window before the failure is raised.
        let mut written_path = None;
        if outcome.triggered {
            let mut text = String::new();
            for envelope in &outcome.events {
                let value = envelope_value(context, envelope)?;
                text.push_str(&context.write_to_string(value)?);
                text.push('\n');
            }
            match arm.borrow_mut().write_artifact(&path, text.as_bytes()) {
                Ok(written) => written_path = Some(written),
                Err(error) => return op_error_value(context, &error),
            }
        }
        if outcome.failure.is_some() {
            return op_error_value(
                context,
                &OpError::TraceOverflow(
                    "trace capture exceeded its bounded payload limits".to_owned(),
                ),
            );
        }
        let bytes_value = match &written_path {
            Some(written) => context.integer(written_len(written) as i128)?,
            None => Value::boolean(false),
        };
        let triggered = Value::boolean(outcome.triggered);
        let complete = Value::boolean(outcome.complete);
        let events = context.integer(outcome.events.len() as i128)?;
        let trigger_index = match outcome.trigger_index {
            Some(index) => context.integer(index as i128)?,
            None => Value::boolean(false),
        };
        make_alist(
            context,
            vec![
                ("triggered", triggered),
                ("complete", complete),
                ("events", events),
                ("trigger-index", trigger_index),
                ("bytes", bytes_value),
            ],
        )
    })?;

    let wait = Rc::clone(session);
    engine.register_library_fn(internal, "%wait-for-event", 5..=5, move |context, args| {
        if let Err(value) = machine_id(context, &wait, args[0])? {
            return Ok(value);
        }
        let spec = match parse_filter(context, args[1])? {
            Ok(spec) => spec,
            Err(value) => return Ok(value),
        };
        let max_frames = match to_count(context, args[2])? {
            Ok(value) => value,
            Err(value) => return Ok(value),
        };
        let max_ticks = match to_count(context, args[3])? {
            Ok(value) => value,
            Err(value) => return Ok(value),
        };
        let snapshot_processors = match parse_snapshot_processors(context, args[4])? {
            Ok(processors) => processors,
            Err(value) => return Ok(value),
        };
        match wait
            .borrow_mut()
            .wait_for_event(spec, max_frames, max_ticks, snapshot_processors)
        {
            Ok(Some(envelope)) => envelope_value(context, &envelope),
            Ok(None) => Ok(Value::boolean(false)),
            Err(error) => op_error_value(context, &error),
        }
    })?;

    Ok(())
}
