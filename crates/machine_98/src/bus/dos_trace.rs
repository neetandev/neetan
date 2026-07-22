//! Bridge from the DOS trace facade to the generic machine trace sink.
//!
//! The bridge implements `dos::trace::DosTraceSink` on top of any `TraceSink`,
//! translating each semantic DOS event into a `neetan.dos.*` device event. It
//! is generic over the sink type, so an untraced build monomorphizes `wants` to
//! a constant `false` and emits nothing.

use common::{
    StackVec, TraceContext, TraceDeviceEvent, TraceEvent, TraceEventKey, TraceField, TraceSink,
    TraceValue, trace_id,
};
use dos::trace::{
    DosCellWriteEvent, DosClearEvent, DosConsoleByteEvent, DosConsoleEscapeEvent, DosScrollEvent,
    DosStdoutEvent, DosTraceEvent, DosTraceKind, DosTraceSink, DosVectorEvent,
};

/// The maximum number of fields any single DOS device event carries.
const MAX_FIELDS: usize = 16;

/// Forwards DOS trace events to a generic trace sink as device events.
pub(super) struct DosTraceBridge<'a, T: TraceSink> {
    pub(super) sink: &'a mut T,
    pub(super) cycle: u64,
    pub(super) clock_hz: u32,
}

impl<T: TraceSink> DosTraceBridge<'_, T> {
    fn emit(&mut self, device: &'static str, action: &'static str, fields: &[TraceField<'_>]) {
        self.sink.trace(
            TraceContext::main_cpu(self.cycle, Some(u64::from(self.clock_hz))),
            TraceEvent::Device(TraceDeviceEvent {
                device,
                action,
                fields,
            }),
        );
    }
}

/// Represents an optional unsigned field as a value or Boolean false.
fn optional_unsigned(value: Option<u64>) -> TraceValue<'static> {
    match value {
        Some(value) => TraceValue::Unsigned(value),
        None => TraceValue::Bool(false),
    }
}

/// Represents an optional symbol field as a symbol or Boolean false.
fn optional_symbol(value: Option<&'static str>) -> TraceValue<'static> {
    match value {
        Some(value) => TraceValue::Symbol(value),
        None => TraceValue::Bool(false),
    }
}

fn field<'a>(name: &'static str, value: TraceValue<'a>) -> TraceField<'a> {
    TraceField { name, value }
}

fn unsigned(name: &'static str, value: u64) -> TraceField<'static> {
    field(name, TraceValue::Unsigned(value))
}

impl<T: TraceSink> DosTraceSink for DosTraceBridge<'_, T> {
    const ENABLED: bool = T::ENABLED;

    fn wants(&self, kind: DosTraceKind) -> bool {
        if !T::ENABLED {
            return false;
        }
        let (device, action) = match kind {
            DosTraceKind::Vector => (trace_id::device::NEETAN_DOS_VECTOR, trace_id::action::SET),
            DosTraceKind::Stdout => (trace_id::device::NEETAN_DOS_STDOUT, trace_id::action::WRITE),
            DosTraceKind::ConsoleByte => {
                (trace_id::device::NEETAN_DOS_CONSOLE, trace_id::action::BYTE)
            }
            DosTraceKind::ConsoleEscape => (
                trace_id::device::NEETAN_DOS_CONSOLE,
                trace_id::action::ESCAPE,
            ),
            DosTraceKind::CellWrite => (
                trace_id::device::NEETAN_DOS_CONSOLE,
                trace_id::action::CELL_WRITE,
            ),
            DosTraceKind::Clear => (
                trace_id::device::NEETAN_DOS_CONSOLE,
                trace_id::action::CLEAR,
            ),
            DosTraceKind::Scroll => (
                trace_id::device::NEETAN_DOS_CONSOLE,
                trace_id::action::SCROLL,
            ),
        };
        self.sink
            .interested(TraceEventKey::Device { device, action })
    }

    fn emit(&mut self, event: &DosTraceEvent) {
        match event {
            DosTraceEvent::Vector(event) => self.emit_vector(event),
            DosTraceEvent::Stdout(event) => self.emit_stdout(event),
            DosTraceEvent::ConsoleByte(event) => self.emit_console_byte(event),
            DosTraceEvent::ConsoleEscape(event) => self.emit_console_escape(event),
            DosTraceEvent::CellWrite(event) => self.emit_cell_write(event),
            DosTraceEvent::Clear(event) => self.emit_clear(event),
            DosTraceEvent::Scroll(event) => self.emit_scroll(event),
        }
    }
}

impl<T: TraceSink> DosTraceBridge<'_, T> {
    fn emit_vector(&mut self, event: &DosVectorEvent) {
        let fields = [
            unsigned(trace_id::field::VECTOR, u64::from(event.vector)),
            unsigned(trace_id::field::SEGMENT, u64::from(event.segment)),
            unsigned(trace_id::field::OFFSET, u64::from(event.offset)),
            unsigned(
                trace_id::field::LINEAR_ADDRESS,
                u64::from(event.linear_address),
            ),
        ];
        self.emit(
            trace_id::device::NEETAN_DOS_VECTOR,
            trace_id::action::SET,
            &fields,
        );
    }

    fn emit_stdout(&mut self, event: &DosStdoutEvent) {
        let fields = [
            field(trace_id::field::SOURCE, TraceValue::Symbol(event.source)),
            field(
                trace_id::field::HANDLE,
                optional_unsigned(event.handle.map(u64::from)),
            ),
            field(
                trace_id::field::BUFFER_ADDRESS,
                optional_unsigned(event.buffer_address.map(u64::from)),
            ),
            unsigned(
                trace_id::field::REQUESTED_COUNT,
                u64::from(event.requested_count),
            ),
            field(trace_id::field::BYTES, TraceValue::Bytes(&event.bytes)),
            field(trace_id::field::ROUTE, TraceValue::Symbol(event.route)),
            field(
                trace_id::field::SUPPRESSION_REASON,
                optional_symbol(event.suppression_reason),
            ),
            unsigned(
                trace_id::field::INT29_SEGMENT,
                u64::from(event.int29_segment),
            ),
            unsigned(trace_id::field::INT29_OFFSET, u64::from(event.int29_offset)),
        ];
        self.emit(
            trace_id::device::NEETAN_DOS_STDOUT,
            trace_id::action::WRITE,
            &fields,
        );
    }

    fn emit_console_byte(&mut self, event: &DosConsoleByteEvent) {
        let mut fields = StackVec::<TraceField<'_>, MAX_FIELDS>::new();
        fields.push(unsigned(trace_id::field::BYTE, u64::from(event.byte)));
        fields.push(field(
            trace_id::field::PARSER_STATE_BEFORE,
            TraceValue::Symbol(event.parser_state_before),
        ));
        fields.push(field(
            trace_id::field::PARSER_STATE_AFTER,
            TraceValue::Symbol(event.parser_state_after),
        ));
        fields.push(field(
            trace_id::field::CHARACTER_MODE_BEFORE,
            TraceValue::Symbol(event.character_mode_before),
        ));
        fields.push(field(
            trace_id::field::CHARACTER_MODE_AFTER,
            TraceValue::Symbol(event.character_mode_after),
        ));
        fields.push(field(
            trace_id::field::PENDING_SHIFT_JIS_LEAD_BEFORE,
            optional_unsigned(event.pending_shift_jis_lead_before.map(u64::from)),
        ));
        fields.push(field(
            trace_id::field::PENDING_SHIFT_JIS_LEAD_AFTER,
            optional_unsigned(event.pending_shift_jis_lead_after.map(u64::from)),
        ));
        fields.push(unsigned(
            trace_id::field::CURSOR_ROW_BEFORE,
            u64::from(event.cursor_row_before),
        ));
        fields.push(unsigned(
            trace_id::field::CURSOR_COLUMN_BEFORE,
            u64::from(event.cursor_column_before),
        ));
        fields.push(unsigned(
            trace_id::field::CURSOR_ROW_AFTER,
            u64::from(event.cursor_row_after),
        ));
        fields.push(unsigned(
            trace_id::field::CURSOR_COLUMN_AFTER,
            u64::from(event.cursor_column_after),
        ));
        fields.push(unsigned(
            trace_id::field::ATTRIBUTE_BEFORE,
            u64::from(event.attribute_before),
        ));
        fields.push(unsigned(
            trace_id::field::ATTRIBUTE_AFTER,
            u64::from(event.attribute_after),
        ));
        self.emit(
            trace_id::device::NEETAN_DOS_CONSOLE,
            trace_id::action::BYTE,
            &fields,
        );
    }

    fn emit_console_escape(&mut self, event: &DosConsoleEscapeEvent) {
        let fields = [
            field(trace_id::field::BYTES, TraceValue::Bytes(&event.bytes)),
            field(trace_id::field::COMMAND, TraceValue::Symbol(event.command)),
            field(
                trace_id::field::PARAMETERS,
                TraceValue::U16List(&event.parameters),
            ),
            unsigned(
                trace_id::field::ATTRIBUTE_BEFORE,
                u64::from(event.attribute_before),
            ),
            unsigned(
                trace_id::field::ATTRIBUTE_AFTER,
                u64::from(event.attribute_after),
            ),
            unsigned(
                trace_id::field::CURSOR_ROW_BEFORE,
                u64::from(event.cursor_row_before),
            ),
            unsigned(
                trace_id::field::CURSOR_COLUMN_BEFORE,
                u64::from(event.cursor_column_before),
            ),
            unsigned(
                trace_id::field::CURSOR_ROW_AFTER,
                u64::from(event.cursor_row_after),
            ),
            unsigned(
                trace_id::field::CURSOR_COLUMN_AFTER,
                u64::from(event.cursor_column_after),
            ),
        ];
        self.emit(
            trace_id::device::NEETAN_DOS_CONSOLE,
            trace_id::action::ESCAPE,
            &fields,
        );
    }

    fn emit_cell_write(&mut self, event: &DosCellWriteEvent) {
        let fields = [
            unsigned(trace_id::field::ROW, u64::from(event.row)),
            unsigned(trace_id::field::COLUMN, u64::from(event.column)),
            unsigned(trace_id::field::JIS, u64::from(event.jis)),
            unsigned(
                trace_id::field::DISPLAY_WIDTH,
                u64::from(event.display_width),
            ),
            unsigned(trace_id::field::ATTRIBUTE, u64::from(event.attribute)),
        ];
        self.emit(
            trace_id::device::NEETAN_DOS_CONSOLE,
            trace_id::action::CELL_WRITE,
            &fields,
        );
    }

    fn emit_clear(&mut self, event: &DosClearEvent) {
        let fields = [
            unsigned(trace_id::field::REGION_TOP, u64::from(event.region_top)),
            unsigned(
                trace_id::field::REGION_BOTTOM,
                u64::from(event.region_bottom),
            ),
            unsigned(trace_id::field::COUNT, u64::from(event.count)),
        ];
        self.emit(
            trace_id::device::NEETAN_DOS_CONSOLE,
            trace_id::action::CLEAR,
            &fields,
        );
    }

    fn emit_scroll(&mut self, event: &DosScrollEvent) {
        let fields = [
            unsigned(trace_id::field::REGION_TOP, u64::from(event.region_top)),
            unsigned(
                trace_id::field::REGION_BOTTOM,
                u64::from(event.region_bottom),
            ),
            unsigned(trace_id::field::COUNT, u64::from(event.count)),
            field(
                trace_id::field::DIRECTION,
                TraceValue::Signed(i64::from(event.direction)),
            ),
        ];
        self.emit(
            trace_id::device::NEETAN_DOS_CONSOLE,
            trace_id::action::SCROLL,
            &fields,
        );
    }
}
