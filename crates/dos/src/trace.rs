//! DOS-facing trace facade and transient event log.
//!
//! The HLE DOS console records semantic events into an interior-mutable log on
//! [`crate::console::Console`] while a dispatch runs. The machine bus arms the
//! log through [`DosTraceSink::wants`] and drains it at the dispatch boundary,
//! forwarding each event to its own generic trace sink. This keeps the `dos`
//! crate free of any host trace type, and a no-op sink costs a single bool
//! check per potential event.

use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use crate::console_esc::EscState;

/// Console output routing decisions.
pub mod route {
    /// Output reached the text console.
    pub const CONSOLE: &str = "console";
    /// Output was routed to the NUL device.
    pub const NUL: &str = "nul";
    /// Output was redirected to a file handle.
    pub const REDIRECTED: &str = "redirected";
    /// Output was suppressed before reaching the console.
    pub const SUPPRESSED: &str = "suppressed";
}

/// DOS API paths that produce console-bound output.
pub mod source {
    /// INT 21h AH=02h display character.
    pub const INT21_02: &str = "int21.02";
    /// INT 21h AH=06h direct console output.
    pub const INT21_06: &str = "int21.06";
    /// INT 21h AH=09h display string.
    pub const INT21_09: &str = "int21.09";
    /// INT 21h AH=40h write to handle.
    pub const INT21_40: &str = "int21.40";
    /// INT 29h fast console output.
    pub const INT29: &str = "int29";
}

/// Reasons console output was suppressed.
pub mod suppression {
    /// The active INT 29h handler begins with an `IRET`.
    pub const INT29_IRET_HOOK: &str = "int29-iret-hook";
}

/// Stable console character-mode symbols.
pub mod character_mode {
    /// Single-byte ANK mode.
    pub const ANK: &str = "ank";
    /// Shift-JIS kanji mode.
    pub const SHIFT_JIS: &str = "shift-jis";
}

/// Maps the console parser state to its stable symbol.
pub(crate) const fn parser_state_symbol(state: EscState) -> &'static str {
    match state {
        EscState::Normal => "normal",
        EscState::GotEsc => "escape",
        EscState::GotCsi => "csi",
        EscState::GotCsiQuestion => "csi-question",
        EscState::GotCsiGreater => "csi-greater",
        EscState::GotEscRightParen => "esc-right-paren",
        EscState::GotEscEqual => "esc-equal",
        EscState::GotEscEqualRow => "esc-equal-row",
    }
}

/// The interest category of a DOS trace event, one per device action.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DosTraceKind {
    /// An interrupt-vector change.
    Vector,
    /// A console-bound stdout routing decision.
    Stdout,
    /// A byte entering the console parser.
    ConsoleByte,
    /// A dispatched console escape sequence.
    ConsoleEscape,
    /// A written character cell.
    CellWrite,
    /// A console clear operation.
    Clear,
    /// A console scroll operation.
    Scroll,
}

/// The per-action arming state of the DOS trace log for one dispatch.
#[derive(Clone, Copy, Default)]
pub struct DosTraceInterest {
    /// Whether vector events are wanted.
    pub vector: bool,
    /// Whether stdout events are wanted.
    pub stdout: bool,
    /// Whether console byte events are wanted.
    pub console_byte: bool,
    /// Whether console escape events are wanted.
    pub console_escape: bool,
    /// Whether cell-write events are wanted.
    pub cell_write: bool,
    /// Whether clear events are wanted.
    pub clear: bool,
    /// Whether scroll events are wanted.
    pub scroll: bool,
}

impl DosTraceInterest {
    /// Returns interest in every DOS trace event.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            vector: true,
            stdout: true,
            console_byte: true,
            console_escape: true,
            cell_write: true,
            clear: true,
            scroll: true,
        }
    }
}

/// An interrupt-vector set operation.
#[derive(Clone)]
pub struct DosVectorEvent {
    /// Interrupt vector number.
    pub vector: u8,
    /// New handler segment.
    pub segment: u16,
    /// New handler offset.
    pub offset: u16,
    /// New handler linear target address.
    pub linear_address: u32,
}

/// A console-bound stdout routing decision.
#[derive(Clone)]
pub struct DosStdoutEvent {
    /// DOS API path that produced the output.
    pub source: &'static str,
    /// File handle, when the API used one.
    pub handle: Option<u16>,
    /// Output buffer linear address, when the API supplied one.
    pub buffer_address: Option<u32>,
    /// Requested output byte count.
    pub requested_count: u32,
    /// Output bytes.
    pub bytes: Vec<u8>,
    /// Routing decision.
    pub route: &'static str,
    /// Reason output was suppressed, when applicable.
    pub suppression_reason: Option<&'static str>,
    /// Active INT 29h handler segment.
    pub int29_segment: u16,
    /// Active INT 29h handler offset.
    pub int29_offset: u16,
}

/// One byte entering the console parser with before and after state.
#[derive(Clone)]
pub struct DosConsoleByteEvent {
    /// The byte.
    pub byte: u8,
    /// Parser state before the byte.
    pub parser_state_before: &'static str,
    /// Parser state after the byte.
    pub parser_state_after: &'static str,
    /// Character mode before the byte.
    pub character_mode_before: &'static str,
    /// Character mode after the byte.
    pub character_mode_after: &'static str,
    /// Pending Shift-JIS lead byte before the byte.
    pub pending_shift_jis_lead_before: Option<u8>,
    /// Pending Shift-JIS lead byte after the byte.
    pub pending_shift_jis_lead_after: Option<u8>,
    /// Cursor row before the byte.
    pub cursor_row_before: u8,
    /// Cursor column before the byte.
    pub cursor_column_before: u8,
    /// Cursor row after the byte.
    pub cursor_row_after: u8,
    /// Cursor column after the byte.
    pub cursor_column_after: u8,
    /// Text attribute before the byte.
    pub attribute_before: u8,
    /// Text attribute after the byte.
    pub attribute_after: u8,
}

/// A complete escape sequence dispatched by the console.
#[derive(Clone)]
pub struct DosConsoleEscapeEvent {
    /// Canonical escape sequence bytes.
    pub bytes: Vec<u8>,
    /// Dispatched command identifier.
    pub command: &'static str,
    /// Numeric command parameters, preserved without truncation.
    pub parameters: Vec<u16>,
    /// Text attribute before the sequence.
    pub attribute_before: u8,
    /// Text attribute after the sequence.
    pub attribute_after: u8,
    /// Cursor row before the sequence.
    pub cursor_row_before: u8,
    /// Cursor column before the sequence.
    pub cursor_column_before: u8,
    /// Cursor row after the sequence.
    pub cursor_row_after: u8,
    /// Cursor column after the sequence.
    pub cursor_column_after: u8,
}

/// A decoded character cell written to text VRAM.
#[derive(Clone)]
pub struct DosCellWriteEvent {
    /// Cell row.
    pub row: u8,
    /// Cell column.
    pub column: u8,
    /// Raw JIS character value.
    pub jis: u16,
    /// Cell display width in columns.
    pub display_width: u8,
    /// Cell attribute.
    pub attribute: u8,
}

/// A console clear operation.
#[derive(Clone)]
pub struct DosClearEvent {
    /// Top row of the affected region.
    pub region_top: u8,
    /// Bottom row of the affected region.
    pub region_bottom: u8,
    /// Number of cells cleared.
    pub count: u32,
}

/// A console scroll operation.
#[derive(Clone)]
pub struct DosScrollEvent {
    /// Top row of the affected region.
    pub region_top: u8,
    /// Bottom row of the affected region.
    pub region_bottom: u8,
    /// Number of rows scrolled.
    pub count: u8,
    /// Scroll direction, positive when scrolling up.
    pub direction: i8,
}

/// A semantic DOS trace event captured during a dispatch.
#[derive(Clone)]
pub enum DosTraceEvent {
    /// An interrupt-vector change.
    Vector(DosVectorEvent),
    /// A stdout routing decision.
    Stdout(DosStdoutEvent),
    /// A byte entering the console parser.
    ConsoleByte(DosConsoleByteEvent),
    /// A dispatched escape sequence.
    ConsoleEscape(DosConsoleEscapeEvent),
    /// A written character cell.
    CellWrite(DosCellWriteEvent),
    /// A clear operation.
    Clear(DosClearEvent),
    /// A scroll operation.
    Scroll(DosScrollEvent),
}

/// Receives semantic DOS trace events forwarded at the dispatch boundary.
///
/// A statically disabled sink sets [`DosTraceSink::ENABLED`] to `false`, which
/// lets the dispatch boundary skip arming and draining the log entirely.
pub trait DosTraceSink {
    /// Whether this sink can observe events in this monomorphization.
    const ENABLED: bool = true;

    /// Returns whether the sink wants events of this category.
    fn wants(&self, kind: DosTraceKind) -> bool {
        let _ = kind;
        false
    }

    /// Consumes one drained event.
    fn emit(&mut self, event: &DosTraceEvent) {
        let _ = event;
    }
}

/// A no-op DOS trace sink eliminated through static dispatch.
pub struct NoDosTrace;

impl DosTraceSink for NoDosTrace {
    const ENABLED: bool = false;
}

/// Interior-mutable event log filled by the console during a dispatch.
///
/// The per-action enable flags live in plain [`Cell`]s so each interest check
/// is a single bool load without touching the event vec's borrow state.
#[derive(Clone, Default)]
pub(crate) struct DosTraceLog {
    pub(crate) vector_enabled: Cell<bool>,
    pub(crate) stdout_enabled: Cell<bool>,
    pub(crate) console_byte_enabled: Cell<bool>,
    pub(crate) console_escape_enabled: Cell<bool>,
    pub(crate) cell_write_enabled: Cell<bool>,
    pub(crate) clear_enabled: Cell<bool>,
    pub(crate) scroll_enabled: Cell<bool>,
    pub(crate) events: RefCell<Vec<DosTraceEvent>>,
}

impl DosTraceLog {
    /// Arms the log for a dispatch and clears any residual events.
    pub(crate) fn arm(&self, interest: DosTraceInterest) {
        self.vector_enabled.set(interest.vector);
        self.stdout_enabled.set(interest.stdout);
        self.console_byte_enabled.set(interest.console_byte);
        self.console_escape_enabled.set(interest.console_escape);
        self.cell_write_enabled.set(interest.cell_write);
        self.clear_enabled.set(interest.clear);
        self.scroll_enabled.set(interest.scroll);
        self.events.borrow_mut().clear();
    }

    /// Disarms the log and returns the events captured during the dispatch.
    pub(crate) fn finish(&self) -> Vec<DosTraceEvent> {
        self.vector_enabled.set(false);
        self.stdout_enabled.set(false);
        self.console_byte_enabled.set(false);
        self.console_escape_enabled.set(false);
        self.cell_write_enabled.set(false);
        self.clear_enabled.set(false);
        self.scroll_enabled.set(false);
        core::mem::take(&mut *self.events.borrow_mut())
    }

    /// Records a vector event when armed.
    pub(crate) fn push_vector(&self, event: DosVectorEvent) {
        if self.vector_enabled.get() {
            self.events.borrow_mut().push(DosTraceEvent::Vector(event));
        }
    }

    /// Records a stdout event when armed.
    pub(crate) fn push_stdout(&self, event: DosStdoutEvent) {
        if self.stdout_enabled.get() {
            self.events.borrow_mut().push(DosTraceEvent::Stdout(event));
        }
    }
}
