//! Trace schema definition, declarative-filter compilation, and continuous or
//! one-shot trace collection over the current machine's trace handle.
//!
//! The static schema-v1 tables here describe the normalized event shape and drive
//! both `trace-schema` discovery and up-front filter validation. A declarative
//! filter is compiled to a [`CompiledFilter`] that matches borrowed events without
//! re-entering Scheme.

use common::{
    RunRequest, RunTarget, StopReason, TraceCatalog, TraceContext, TraceEvent, TraceEventClass,
    TraceInterest,
    tracing::{ApplicationTraceEnvelope, TraceDecision, TraceFailure, TraceHandle, TraceMatcher},
};

use super::{AutomationSession, INPUT_DRAIN_INTERVAL_TICKS, OpError};

/// Value type of a normalized event field, used by the schema and filter checks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchemaType {
    /// A stable identifier, compared as a symbol.
    Symbol,
    /// An exact integer.
    Integer,
    /// A boolean.
    Boolean,
    /// An exact integer or `#f` when unavailable.
    IntegerOrFalse,
    /// A nested association list.
    Alist,
}

impl SchemaType {
    /// Returns the stable schema type name.
    pub(crate) fn name(self) -> &'static str {
        match self {
            SchemaType::Symbol => "symbol",
            SchemaType::Integer => "integer",
            SchemaType::Boolean => "boolean",
            SchemaType::IntegerOrFalse => "integer-or-false",
            SchemaType::Alist => "alist",
        }
    }

    /// Returns whether a scalar constraint can be placed on this field.
    fn filterable(self) -> bool {
        matches!(
            self,
            SchemaType::Symbol
                | SchemaType::Integer
                | SchemaType::Boolean
                | SchemaType::IntegerOrFalse
        )
    }

    /// Returns whether this field carries an integer value for comparison.
    fn integral(self) -> bool {
        matches!(self, SchemaType::Integer | SchemaType::IntegerOrFalse)
    }
}

/// A normalized event field descriptor.
pub(crate) struct FieldSchema {
    /// Stable field name.
    pub(crate) name: &'static str,
    /// Field value type.
    pub(crate) ty: SchemaType,
    /// Whether an inclusive `(range min max)` constraint is accepted here.
    pub(crate) range: bool,
}

/// A per-class data-field schema.
pub(crate) struct ClassSchema {
    /// The event class this schema describes.
    pub(crate) class: TraceEventClass,
    /// The class-specific `data` fields.
    pub(crate) fields: &'static [FieldSchema],
}

/// The normalized envelope fields shared by every event.
pub(crate) const ENVELOPE_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        name: "schema-version",
        ty: SchemaType::Integer,
        range: false,
    },
    FieldSchema {
        name: "sequence",
        ty: SchemaType::Integer,
        range: false,
    },
    FieldSchema {
        name: "epoch",
        ty: SchemaType::Integer,
        range: false,
    },
    FieldSchema {
        name: "tick",
        ty: SchemaType::Integer,
        range: true,
    },
    FieldSchema {
        name: "source",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "clock-domain",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "clock-cycle",
        ty: SchemaType::Integer,
        range: true,
    },
    FieldSchema {
        name: "clock-rate",
        ty: SchemaType::Alist,
        range: false,
    },
    FieldSchema {
        name: "class",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "data",
        ty: SchemaType::Alist,
        range: false,
    },
];

const ACCESS_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        name: "space",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "space-class",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "operation",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "address",
        ty: SchemaType::Integer,
        range: true,
    },
    FieldSchema {
        name: "width",
        ty: SchemaType::Integer,
        range: true,
    },
    FieldSchema {
        name: "value",
        ty: SchemaType::IntegerOrFalse,
        range: true,
    },
    FieldSchema {
        name: "handled?",
        ty: SchemaType::Boolean,
        range: false,
    },
];

const INTERRUPT_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        name: "controller",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "interrupt-kind",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "line",
        ty: SchemaType::IntegerOrFalse,
        range: true,
    },
    FieldSchema {
        name: "action",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "vector",
        ty: SchemaType::IntegerOrFalse,
        range: true,
    },
];

const SCHEDULED_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        name: "event",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "fire-tick",
        ty: SchemaType::Integer,
        range: true,
    },
];

const PRESENTATION_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        name: "display",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "frame",
        ty: SchemaType::Integer,
        range: true,
    },
    FieldSchema {
        name: "width",
        ty: SchemaType::Integer,
        range: true,
    },
    FieldSchema {
        name: "height",
        ty: SchemaType::Integer,
        range: true,
    },
];

const DEVICE_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        name: "device",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "action",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "fields",
        ty: SchemaType::Alist,
        range: false,
    },
];

const CALL_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        name: "provider",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "interface",
        ty: SchemaType::Alist,
        range: false,
    },
    FieldSchema {
        name: "phase",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "fields",
        ty: SchemaType::Alist,
        range: false,
    },
];

/// The per-class data-field schemas in schema order.
pub(crate) const CLASS_SCHEMAS: &[ClassSchema] = &[
    ClassSchema {
        class: TraceEventClass::Access,
        fields: ACCESS_FIELDS,
    },
    ClassSchema {
        class: TraceEventClass::Interrupt,
        fields: INTERRUPT_FIELDS,
    },
    ClassSchema {
        class: TraceEventClass::Scheduled,
        fields: SCHEDULED_FIELDS,
    },
    ClassSchema {
        class: TraceEventClass::Presentation,
        fields: PRESENTATION_FIELDS,
    },
    ClassSchema {
        class: TraceEventClass::Device,
        fields: DEVICE_FIELDS,
    },
    ClassSchema {
        class: TraceEventClass::Call,
        fields: CALL_FIELDS,
    },
];

/// Filterable envelope fields, resolved by name during compilation.
const FILTERABLE_ENVELOPE: &[FieldSchema] = &[
    FieldSchema {
        name: "class",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "source",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "clock-domain",
        ty: SchemaType::Symbol,
        range: false,
    },
    FieldSchema {
        name: "tick",
        ty: SchemaType::Integer,
        range: true,
    },
    FieldSchema {
        name: "clock-cycle",
        ty: SchemaType::Integer,
        range: true,
    },
];

/// A single parsed scalar constraint from a declarative filter.
pub(crate) enum FilterScalar {
    /// A symbol match.
    Symbol(String),
    /// An exact integer match.
    Integer(i128),
    /// A boolean match.
    Boolean(bool),
    /// An inclusive numeric range.
    Range(i128, i128),
}

/// A parsed but unvalidated declarative filter, produced by the native layer.
pub(crate) struct FilterSpec {
    /// Envelope-level key and value pairs, preserving order and duplicates.
    pub(crate) top: Vec<(String, FilterScalar)>,
    /// The nested `data` alist, when present.
    pub(crate) data: Option<Vec<(String, FilterScalar)>>,
}

/// A compiled constraint bound to a static field name.
enum Constraint {
    Symbol(&'static str, String),
    Integer(&'static str, i128),
    Boolean(&'static str, bool),
    Range(&'static str, i128, i128),
}

impl Constraint {
    /// Returns the static field name this constraint applies to.
    fn field(&self) -> &'static str {
        match self {
            Constraint::Symbol(name, _)
            | Constraint::Integer(name, _)
            | Constraint::Boolean(name, _)
            | Constraint::Range(name, ..) => name,
        }
    }

    /// Returns whether `value` satisfies this constraint.
    fn matches(&self, value: FieldValue<'_>) -> bool {
        match (self, value) {
            (Constraint::Symbol(_, expected), FieldValue::Symbol(actual)) => actual == expected,
            (Constraint::Integer(_, expected), FieldValue::Integer(actual)) => actual == *expected,
            (Constraint::Boolean(_, expected), FieldValue::Boolean(actual)) => actual == *expected,
            (Constraint::Range(_, low, high), FieldValue::Integer(actual)) => {
                actual >= *low && actual <= *high
            }
            _ => false,
        }
    }
}

/// A resolved event field value used for constraint comparison.
#[derive(Clone, Copy)]
enum FieldValue<'a> {
    Symbol(&'a str),
    Integer(i128),
    Boolean(bool),
    /// The field exists for this class but has no value (`#f`).
    Absent,
}

/// A compiled declarative filter that matches borrowed events.
pub(crate) struct CompiledFilter {
    class: Option<TraceEventClass>,
    top: Vec<Constraint>,
    data: Vec<Constraint>,
    one_shot: bool,
    matched: bool,
}

impl CompiledFilter {
    /// Returns the event-class interest this filter requires.
    ///
    /// A class-constrained filter narrows interest to that class; an unconstrained
    /// filter collects every class the machine emits.
    fn interest(&self, emitted: TraceInterest) -> TraceInterest {
        match self.class {
            Some(class) => TraceInterest::only(class),
            None => emitted,
        }
    }

    /// Returns whether an event satisfies every constraint.
    fn matches(&self, context: TraceContext, event: TraceEvent<'_>) -> bool {
        if let Some(class) = self.class
            && event.key().class() != class
        {
            return false;
        }
        for constraint in &self.top {
            match envelope_field(context, event, constraint.field()) {
                Some(value) if constraint.matches(value) => {}
                _ => return false,
            }
        }
        for constraint in &self.data {
            match data_field(event, constraint.field()) {
                Some(value) if constraint.matches(value) => {}
                _ => return false,
            }
        }
        true
    }
}

impl TraceMatcher for CompiledFilter {
    fn decide(&mut self, context: TraceContext, event: TraceEvent<'_>) -> TraceDecision {
        if self.matched {
            return TraceDecision::Ignore;
        }
        if self.matches(context, event) {
            if self.one_shot {
                self.matched = true;
                TraceDecision::RecordAndYield
            } else {
                TraceDecision::Record
            }
        } else {
            TraceDecision::Ignore
        }
    }
}

/// Returns the stable name of an access operation.
pub(crate) fn access_operation(kind: common::TraceAccessKind) -> &'static str {
    match kind {
        common::TraceAccessKind::Fetch => "fetch",
        common::TraceAccessKind::Read => "read",
        common::TraceAccessKind::Write => "write",
        _ => "unknown",
    }
}

/// Returns the width of an access in bits.
pub(crate) fn access_width_bits(width: common::TraceAccessWidth) -> i128 {
    match width {
        common::TraceAccessWidth::Byte => 8,
        common::TraceAccessWidth::Word => 16,
        common::TraceAccessWidth::Dword => 32,
        common::TraceAccessWidth::Qword => 64,
        _ => 0,
    }
}

/// Returns the stable name of an address-space class.
pub(crate) fn space_class_name(class: common::TraceAddressSpaceClass) -> &'static str {
    match class {
        common::TraceAddressSpaceClass::Memory => "memory",
        common::TraceAddressSpaceClass::Io => "io",
        _ => "unknown",
    }
}

/// Returns the stable name of an interrupt kind.
pub(crate) fn interrupt_kind_name(kind: common::TraceInterruptKind) -> &'static str {
    match kind {
        common::TraceInterruptKind::Maskable => "maskable",
        common::TraceInterruptKind::NonMaskable => "non-maskable",
        _ => "unknown",
    }
}

/// Returns the stable name of an interrupt action.
pub(crate) fn interrupt_action_name(action: common::TraceInterruptAction) -> &'static str {
    match action {
        common::TraceInterruptAction::Assert => "assert",
        common::TraceInterruptAction::Clear => "clear",
        common::TraceInterruptAction::Acknowledge => "acknowledge",
        _ => "unknown",
    }
}

/// Returns the stable name of a call phase.
pub(crate) fn call_phase_name(phase: common::TraceCallPhase) -> &'static str {
    match phase {
        common::TraceCallPhase::Enter => "enter",
        common::TraceCallPhase::Exit => "exit",
        _ => "unknown",
    }
}

/// Resolves a filterable envelope field to its runtime value.
fn envelope_field<'a>(
    context: TraceContext,
    event: TraceEvent<'a>,
    key: &str,
) -> Option<FieldValue<'a>> {
    match key {
        "class" => Some(FieldValue::Symbol(event.key().class().as_str())),
        "source" => Some(FieldValue::Symbol(context.source)),
        "clock-domain" => Some(FieldValue::Symbol(context.clock_domain)),
        "tick" => Some(FieldValue::Integer(i128::from(context.tick))),
        "clock-cycle" => Some(FieldValue::Integer(i128::from(context.clock_cycle))),
        _ => None,
    }
}

/// Resolves a class-specific data field to its runtime value.
fn data_field<'a>(event: TraceEvent<'a>, key: &str) -> Option<FieldValue<'a>> {
    match event {
        TraceEvent::Access(access) => match key {
            "space" => Some(FieldValue::Symbol(access.space.id)),
            "space-class" => Some(FieldValue::Symbol(space_class_name(access.space.class))),
            "operation" => Some(FieldValue::Symbol(access_operation(access.kind))),
            "address" => Some(FieldValue::Integer(i128::from(access.address))),
            "width" => Some(FieldValue::Integer(access_width_bits(access.width))),
            "value" => Some(match access.value {
                Some(value) => FieldValue::Integer(i128::from(value)),
                None => FieldValue::Absent,
            }),
            "handled?" => Some(FieldValue::Boolean(access.handled)),
            _ => None,
        },
        TraceEvent::Interrupt(interrupt) => match key {
            "controller" => Some(FieldValue::Symbol(interrupt.controller)),
            "interrupt-kind" => Some(FieldValue::Symbol(interrupt_kind_name(interrupt.kind))),
            "line" => Some(match interrupt.line {
                Some(line) => FieldValue::Integer(i128::from(line)),
                None => FieldValue::Absent,
            }),
            "action" => Some(FieldValue::Symbol(interrupt_action_name(interrupt.action))),
            "vector" => Some(match interrupt.vector {
                Some(vector) => FieldValue::Integer(i128::from(vector)),
                None => FieldValue::Absent,
            }),
            _ => None,
        },
        TraceEvent::Scheduled { event, fire_tick } => match key {
            "event" => Some(FieldValue::Symbol(event)),
            "fire-tick" => Some(FieldValue::Integer(i128::from(fire_tick))),
            _ => None,
        },
        TraceEvent::Presentation(presentation) => match key {
            "display" => Some(FieldValue::Symbol(presentation.display)),
            "frame" => Some(FieldValue::Integer(i128::from(presentation.frame))),
            "width" => Some(FieldValue::Integer(i128::from(presentation.width))),
            "height" => Some(FieldValue::Integer(i128::from(presentation.height))),
            _ => None,
        },
        TraceEvent::Device(device) => match key {
            "device" => Some(FieldValue::Symbol(device.device)),
            "action" => Some(FieldValue::Symbol(device.action)),
            _ => None,
        },
        TraceEvent::Call(call) => match key {
            "provider" => Some(FieldValue::Symbol(call.provider)),
            "phase" => Some(FieldValue::Symbol(call_phase_name(call.phase))),
            _ => None,
        },
        _ => None,
    }
}

/// Looks up a field descriptor in a table by name.
fn find_field<'a>(fields: &'a [FieldSchema], name: &str) -> Option<&'a FieldSchema> {
    fields.iter().find(|field| field.name == name)
}

/// Parses a class symbol name into an event class.
fn parse_class(name: &str) -> Option<TraceEventClass> {
    TraceEventClass::ALL
        .into_iter()
        .find(|class| class.as_str() == name)
}

/// Builds one compiled constraint from a scalar against a field descriptor.
fn build_constraint(field: &FieldSchema, scalar: FilterScalar) -> Result<Constraint, OpError> {
    if !field.ty.filterable() {
        return Err(OpError::Argument(format!(
            "trace filter field '{}' is not filterable",
            field.name
        )));
    }
    match scalar {
        FilterScalar::Symbol(value) => {
            if field.ty == SchemaType::Symbol {
                Ok(Constraint::Symbol(field.name, value))
            } else {
                Err(type_mismatch(field, "symbol"))
            }
        }
        FilterScalar::Boolean(value) => {
            if field.ty == SchemaType::Boolean {
                Ok(Constraint::Boolean(field.name, value))
            } else {
                Err(type_mismatch(field, "boolean"))
            }
        }
        FilterScalar::Integer(value) => {
            if field.ty.integral() {
                Ok(Constraint::Integer(field.name, value))
            } else {
                Err(type_mismatch(field, "integer"))
            }
        }
        FilterScalar::Range(low, high) => {
            if !field.ty.integral() || !field.range {
                return Err(OpError::Argument(format!(
                    "trace filter field '{}' does not accept a range",
                    field.name
                )));
            }
            if low > high {
                return Err(OpError::Argument(format!(
                    "trace filter range on '{}' is reversed",
                    field.name
                )));
            }
            Ok(Constraint::Range(field.name, low, high))
        }
    }
}

/// Builds a type-mismatch error for a filter field.
fn type_mismatch(field: &FieldSchema, got: &str) -> OpError {
    OpError::Argument(format!(
        "trace filter field '{}' expects {}, got {}",
        field.name,
        field.ty.name(),
        got
    ))
}

/// Rejects a duplicate key in a filter section.
fn reject_duplicate(seen: &mut Vec<&'static str>, name: &'static str) -> Result<(), OpError> {
    if seen.contains(&name) {
        return Err(OpError::Argument(format!(
            "trace filter key '{name}' appears more than once"
        )));
    }
    seen.push(name);
    Ok(())
}

/// Compiles a declarative filter, validating it against the schema up front.
fn compile_filter(spec: FilterSpec, one_shot: bool) -> Result<CompiledFilter, OpError> {
    let mut class = None;
    let mut top = Vec::new();
    let mut data = Vec::new();
    let mut seen_top: Vec<&'static str> = Vec::new();

    for (key, scalar) in spec.top {
        if key == "class" {
            reject_duplicate(&mut seen_top, "class")?;
            let FilterScalar::Symbol(name) = scalar else {
                return Err(OpError::Argument(
                    "trace filter 'class' must be a symbol".to_owned(),
                ));
            };
            class =
                Some(parse_class(&name).ok_or_else(|| {
                    OpError::Argument(format!("unknown trace event class '{name}'"))
                })?);
            continue;
        }
        let field = find_field(FILTERABLE_ENVELOPE, &key).ok_or_else(|| {
            if find_field(ENVELOPE_FIELDS, &key).is_some() {
                OpError::Argument(format!("trace filter field '{key}' is not filterable"))
            } else {
                OpError::Argument(format!("unknown trace filter key '{key}'"))
            }
        })?;
        reject_duplicate(&mut seen_top, field.name)?;
        top.push(build_constraint(field, scalar)?);
    }

    if let Some(entries) = spec.data {
        let mut seen_data: Vec<&'static str> = Vec::new();
        for (key, scalar) in entries {
            let field = resolve_data_field(class, &key)?;
            reject_duplicate(&mut seen_data, field.name)?;
            data.push(build_constraint(field, scalar)?);
        }
    }

    Ok(CompiledFilter {
        class,
        top,
        data,
        one_shot,
        matched: false,
    })
}

/// Resolves a data-field name against the constrained class, or the union of all
/// classes when no class is fixed.
fn resolve_data_field(
    class: Option<TraceEventClass>,
    key: &str,
) -> Result<&'static FieldSchema, OpError> {
    match class {
        Some(class) => {
            let schema = CLASS_SCHEMAS
                .iter()
                .find(|schema| schema.class == class)
                .expect("every class has a schema");
            find_field(schema.fields, key).ok_or_else(|| {
                OpError::Argument(format!(
                    "trace filter data field '{key}' is not defined for class '{}'",
                    class.as_str()
                ))
            })
        }
        None => CLASS_SCHEMAS
            .iter()
            .find_map(|schema| find_field(schema.fields, key))
            .ok_or_else(|| OpError::Argument(format!("unknown trace filter data field '{key}'"))),
    }
}

/// Owned parts required to marshal `trace-schema`.
pub(crate) struct TraceSchemaParts {
    /// The event schema version.
    pub(crate) schema_version: u16,
    /// Maximum retained events.
    pub(crate) event_capacity: usize,
    /// Maximum retained variable payload bytes.
    pub(crate) byte_capacity: usize,
    /// Maximum variable payload bytes accepted for one event.
    pub(crate) event_payload_capacity: usize,
    /// Address-space identifiers used in access events.
    pub(crate) address_spaces: Vec<&'static str>,
    /// The machine's emitted identifier catalog.
    pub(crate) catalog: TraceCatalog,
}

impl AutomationSession {
    /// Returns the emitted-class interest set of the current machine.
    fn machine_trace_classes(&self) -> Result<TraceInterest, OpError> {
        self.active
            .as_ref()
            .map(|active| active.machine.trace_catalog().classes())
            .ok_or(OpError::NoMachine)
    }

    /// Assembles the parts required to build the `trace-schema` descriptor.
    pub(crate) fn trace_schema_parts(&mut self) -> Result<TraceSchemaParts, OpError> {
        let catalog = self
            .active
            .as_ref()
            .map(|active| active.machine.trace_catalog())
            .ok_or(OpError::NoMachine)?;
        let address_spaces = self.address_spaces().unwrap_or_default();
        let limits = common::tracing::TraceLimits::default();
        Ok(TraceSchemaParts {
            schema_version: common::TRACE_SCHEMA_VERSION,
            event_capacity: limits.event_capacity.get(),
            byte_capacity: limits.byte_capacity.get(),
            event_payload_capacity: limits.event_payload_capacity.get(),
            address_spaces,
            catalog,
        })
    }

    /// If an active continuous collector has failed, mirror the failure out of
    /// the sink, disable collection, clear the sink wedge, and report that the
    /// active run must raise `neetan/trace-overflow`. Buffered events are kept.
    pub(super) fn consume_trace_overflow(&mut self) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        let handle = active.trace.clone();
        if !handle.is_active() {
            return false;
        }
        if let Some(failure) = handle.failure() {
            active.trace_failure = Some(failure);
            handle.stop();
            handle.take_failure();
            return true;
        }
        false
    }

    /// Begins continuous collection with a compiled declarative filter.
    pub(crate) fn trace_start(&mut self, spec: FilterSpec) -> Result<(), OpError> {
        let classes = self.machine_trace_classes()?;
        let filter = compile_filter(spec, false)?;
        let interest = filter.interest(classes);
        let active = self.active.as_mut().ok_or(OpError::NoMachine)?;
        let handle = active.trace.clone();
        active.trace_failure = None;
        handle.start(filter, interest);
        Ok(())
    }

    /// Returns whether continuous collection is active.
    #[must_use]
    pub fn trace_active(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.trace.is_active())
    }

    /// Disables collection without discarding buffered events or the failure.
    pub fn trace_stop(&self) -> Result<(), OpError> {
        let handle = &self.active.as_ref().ok_or(OpError::NoMachine)?.trace;
        handle.stop();
        Ok(())
    }

    /// Drains and returns all buffered events in sequence order.
    pub fn trace_drain(&self) -> Result<Vec<ApplicationTraceEnvelope>, OpError> {
        let handle = &self.active.as_ref().ok_or(OpError::NoMachine)?.trace;
        Ok(handle.drain())
    }

    /// Returns the sticky trace collector failure, if any.
    pub fn trace_failure(&self) -> Result<Option<TraceFailure>, OpError> {
        let active = self.active.as_ref().ok_or(OpError::NoMachine)?;
        Ok(active.trace_failure.or_else(|| active.trace.failure()))
    }

    /// Runs a one-shot exclusive wait for the first matching event.
    ///
    /// It is invalid while continuous collection is active. It returns the matched
    /// event, or `None` when either explicit bound is exhausted, and always stops
    /// its private collector before returning.
    pub(crate) fn wait_for_event(
        &mut self,
        spec: FilterSpec,
        max_frames: u64,
        max_ticks: u64,
    ) -> Result<Option<ApplicationTraceEnvelope>, OpError> {
        let handle = self
            .active
            .as_ref()
            .ok_or(OpError::NoMachine)?
            .trace
            .clone();
        if handle.is_active() {
            return Err(OpError::TraceState(
                "wait-for-event is invalid while a continuous trace is active".to_owned(),
            ));
        }
        let classes = self.machine_trace_classes()?;
        let filter = compile_filter(spec, true)?;
        let interest = filter.interest(classes);
        self.active.as_mut().expect("machine present").trace_failure = None;
        handle.start(filter, interest);
        let result = self.drive_wait(&handle, max_frames, max_ticks);
        handle.stop();
        result
    }

    /// Drives the machine in single-frame chunks until the private collector
    /// yields on its match or the bounds are exhausted.
    fn drive_wait(
        &mut self,
        handle: &TraceHandle,
        max_frames: u64,
        max_ticks: u64,
    ) -> Result<Option<ApplicationTraceEnvelope>, OpError> {
        let mut presented = 0u64;
        let mut remaining_ticks = max_ticks;
        loop {
            if handle.yield_requested() {
                if let Some(failure) = handle.failure() {
                    self.active.as_mut().expect("machine present").trace_failure = Some(failure);
                    handle.take_failure();
                    return Err(OpError::TraceOverflow(
                        "trace collector exhausted its bounded queue".to_owned(),
                    ));
                }
                return Ok(handle.drain().into_iter().next());
            }
            if self.is_stopped() {
                return Ok(None);
            }
            if presented >= max_frames
                || remaining_ticks == 0
                || self.tick_budget_exhausted()
                || self.frame_budget_exhausted()
            {
                return Ok(None);
            }
            let request = RunRequest {
                target: RunTarget::Frames(1),
                max_ticks: remaining_ticks,
                audio_drain_interval_ticks: INPUT_DRAIN_INTERVAL_TICKS,
            };
            let outcome = self
                .active
                .as_mut()
                .expect("machine present")
                .machine
                .run_automation(request);
            self.consume_budget(&outcome);
            remaining_ticks = remaining_ticks.saturating_sub(outcome.ticks);
            presented = presented.saturating_add(outcome.frames);
            if outcome.stop_reason == StopReason::GuestShutdown {
                return Err(OpError::GuestShutdown);
            }
        }
    }
}
