//! Main CPU IRQ and FIRQ bookkeeping.

use common::{TraceContext, TraceEvent, TraceInterruptAction, TraceSink, trace_id};

/// Trace source id for the main IRQ line.
const IRQ_TRACE_MAIN: u8 = 0;

/// `0xFD02` mask bit enabling keyboard IRQ.
const IRQ_MASK_KEYBOARD: u8 = 0x01;
/// `0xFD02` mask bit enabling printer IRQ.
const IRQ_MASK_PRINTER: u8 = 0x02;
/// `0xFD02` mask bit enabling timer IRQ.
const IRQ_MASK_TIMER: u8 = 0x04;
/// `0xFD02` mask bit enabling floppy IRQ.
const IRQ_MASK_FDC: u8 = 0x10;

/// `0xFD03` active-low keyboard pending bit.
const IRQ_STATUS_KEYBOARD: u8 = 0x01;
/// `0xFD03` active-low printer pending bit.
const IRQ_STATUS_PRINTER: u8 = 0x02;
/// `0xFD03` active-low timer pending bit.
const IRQ_STATUS_TIMER: u8 = 0x04;
/// `0xFD03` active-low external pending bit.
const IRQ_STATUS_EXTERNAL: u8 = 0x08;

/// `0xFD04` active-low sub-attention pending bit.
const FIRQ_STATUS_SUB_ATTENTION: u8 = 0x01;
/// `0xFD04` active-low BREAK pending bit.
const FIRQ_STATUS_BREAK: u8 = 0x02;

/// Interrupt sources and masks for the main MC6809.
#[derive(Debug, Clone)]
pub(crate) struct MainInterrupts {
    mask: u8,
    timer_pending: bool,
    keyboard_pending: bool,
    printer_pending: bool,
    fdc_pending: bool,
    opn_pending: bool,
    break_pending: bool,
    sub_attention: bool,
    irq_line: bool,
    firq_line: bool,
}

impl MainInterrupts {
    /// Creates the main interrupt controller with all lines inactive.
    pub(crate) fn new() -> Self {
        Self {
            mask: 0,
            timer_pending: false,
            keyboard_pending: false,
            printer_pending: false,
            fdc_pending: false,
            opn_pending: false,
            break_pending: false,
            sub_attention: false,
            irq_line: false,
            firq_line: false,
        }
    }

    /// Writes the `0xFD02` IRQ mask register.
    pub(crate) fn write_mask<T: TraceSink>(
        &mut self,
        value: u8,
        context: TraceContext,
        tracer: &mut T,
    ) {
        self.mask = value;
        self.sync_irq_line(context, tracer);
    }

    /// Sets or clears the timer IRQ pending latch.
    pub(crate) fn set_timer_pending<T: TraceSink>(
        &mut self,
        pending: bool,
        context: TraceContext,
        tracer: &mut T,
    ) {
        self.timer_pending = pending;
        self.sync_irq_line(context, tracer);
    }

    /// Sets or clears the floppy controller IRQ pending latch, mirroring the
    /// MB8877 interrupt line. Gated to the main CPU by `0xFD02` bit 4.
    pub(crate) fn set_fdc_pending<T: TraceSink>(
        &mut self,
        pending: bool,
        context: TraceContext,
        tracer: &mut T,
    ) {
        self.fdc_pending = pending;
        self.sync_irq_line(context, tracer);
    }

    /// Sets or clears the keyboard IRQ pending latch. Cleared when the main CPU
    /// reads the keycode low byte from `0xFD01`.
    pub(crate) fn set_keyboard_pending<T: TraceSink>(
        &mut self,
        pending: bool,
        context: TraceContext,
        tracer: &mut T,
    ) {
        self.keyboard_pending = pending;
        self.sync_irq_line(context, tracer);
    }

    /// Sets or clears the YM2203 (OPN) IRQ pending latch (FM-77AV). The chip
    /// gates its own timer interrupt, so this source drives the main IRQ line
    /// directly and reports on `0xFD03` bit 3 and `0xFD17` bit 3.
    pub(crate) fn set_opn_pending<T: TraceSink>(
        &mut self,
        pending: bool,
        context: TraceContext,
        tracer: &mut T,
    ) {
        self.opn_pending = pending;
        self.sync_irq_line(context, tracer);
    }

    /// Whether the OPN IRQ is currently pending.
    pub(crate) fn opn_pending(&self) -> bool {
        self.opn_pending
    }

    /// Whether the keyboard IRQ (and the mirrored sub FIRQ) is enabled by the
    /// `0xFD02` mask.
    pub(crate) fn keyboard_irq_enabled(&self) -> bool {
        self.enabled(IRQ_MASK_KEYBOARD)
    }

    /// Sets or clears the BREAK-key FIRQ source, reported active-low on `0xFD04`.
    pub(crate) fn set_break<T: TraceSink>(
        &mut self,
        pressed: bool,
        context: TraceContext,
        tracer: &mut T,
    ) {
        self.break_pending = pressed;
        self.sync_firq_line(context, tracer);
    }

    /// Clears the timer IRQ pending latch.
    pub(crate) fn ack_timer<T: TraceSink>(&mut self, context: TraceContext, tracer: &mut T) {
        self.timer_pending = false;
        self.sync_irq_line(context, tracer);
    }

    /// Reads the `0xFD03` IRQ status register and clears timer/printer status.
    pub(crate) fn read_status<T: TraceSink>(
        &mut self,
        context: TraceContext,
        tracer: &mut T,
    ) -> u8 {
        let mut value = 0xFF;
        if self.keyboard_pending {
            value &= !IRQ_STATUS_KEYBOARD;
        }
        if self.printer_pending {
            value &= !IRQ_STATUS_PRINTER;
        }
        if self.timer_pending {
            value &= !IRQ_STATUS_TIMER;
        }
        if self.fdc_pending || self.opn_pending {
            value &= !IRQ_STATUS_EXTERNAL;
        }

        self.timer_pending = false;
        self.printer_pending = false;
        self.sync_irq_line(context, tracer);
        value
    }

    /// Whether the IRQ line is currently asserted.
    pub(crate) fn irq_line(&self) -> bool {
        (self.keyboard_pending && self.enabled(IRQ_MASK_KEYBOARD))
            || (self.printer_pending && self.enabled(IRQ_MASK_PRINTER))
            || (self.timer_pending && self.enabled(IRQ_MASK_TIMER))
            || (self.fdc_pending && self.enabled(IRQ_MASK_FDC))
            || self.opn_pending
    }

    /// Whether the FIRQ line is currently asserted.
    pub(crate) fn firq_line(&self) -> bool {
        self.break_pending || self.sub_attention
    }

    /// Raises the sub-attention FIRQ source on behalf of the sub CPU.
    pub(crate) fn raise_sub_attention<T: TraceSink>(
        &mut self,
        context: TraceContext,
        tracer: &mut T,
    ) {
        self.sub_attention = true;
        self.sync_firq_line(context, tracer);
    }

    /// Reads the `0xFD04` FIRQ status register and clears sub attention.
    pub(crate) fn read_firq_status<T: TraceSink>(
        &mut self,
        context: TraceContext,
        tracer: &mut T,
    ) -> u8 {
        let mut value = 0xFF;
        if self.sub_attention {
            value &= !FIRQ_STATUS_SUB_ATTENTION;
        }
        if self.break_pending {
            value &= !FIRQ_STATUS_BREAK;
        }

        self.sub_attention = false;
        self.sync_firq_line(context, tracer);
        value
    }

    /// Whether an IRQ mask bit enables its source.
    fn enabled(&self, bit: u8) -> bool {
        self.mask & bit != 0
    }

    /// Updates the traced IRQ line state.
    fn sync_irq_line<T: TraceSink>(&mut self, context: TraceContext, tracer: &mut T) {
        let new_line = self.irq_line();
        if new_line != self.irq_line {
            self.irq_line = new_line;
            if !T::ENABLED {
                return;
            }
            if new_line {
                tracer.trace(
                    context,
                    TraceEvent::maskable_interrupt(
                        trace_id::controller::FM7_MAIN_IRQ,
                        u16::from(IRQ_TRACE_MAIN),
                        TraceInterruptAction::Assert,
                        None,
                    ),
                );
            } else {
                tracer.trace(
                    context,
                    TraceEvent::maskable_interrupt(
                        trace_id::controller::FM7_MAIN_IRQ,
                        u16::from(IRQ_TRACE_MAIN),
                        TraceInterruptAction::Clear,
                        None,
                    ),
                );
            }
        }
    }

    /// Updates the traced FIRQ line state.
    fn sync_firq_line<T: TraceSink>(&mut self, context: TraceContext, tracer: &mut T) {
        let new_line = self.firq_line();
        if new_line != self.firq_line {
            self.firq_line = new_line;
            if !T::ENABLED {
                return;
            }
            if new_line {
                tracer.trace(
                    context,
                    TraceEvent::maskable_interrupt(
                        trace_id::controller::FM7_MAIN_FIRQ,
                        0,
                        TraceInterruptAction::Assert,
                        None,
                    ),
                );
            } else {
                tracer.trace(
                    context,
                    TraceEvent::maskable_interrupt(
                        trace_id::controller::FM7_MAIN_FIRQ,
                        0,
                        TraceInterruptAction::Clear,
                        None,
                    ),
                );
            }
        }
    }
}

impl Default for MainInterrupts {
    fn default() -> Self {
        Self::new()
    }
}
