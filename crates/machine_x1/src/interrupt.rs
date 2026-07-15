//! Z80 mode-2 interrupt daisy chain.
//!
//! The daisy chain runs, highest priority first: the sound-board CTC, the SIO,
//! the DMA, the main CTC, and finally the keyboard/sub-CPU as the last link.
//! The base X1 only populates the CTC and keyboard links (plus the sound-board
//! CTC when the FM board is present); the turbo adds the SIO and DMA. The
//! sound-board CTC (`ctc_ym`) belongs to the CZ-8BS1 FM board and provides the
//! FM interrupt. The controller tracks which sources are asserting; on
//! acknowledge the bus resolves the highest-priority source and fetches that
//! device's own mode-2 vector, which the CPU turns into the handler pointer
//! `(I << 8) | vector`.

/// Interrupt sources in daisy-chain priority order (lower value = higher
/// priority).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IrqSource {
    /// Sound-board Z80 CTC (`ctc_ym`) zero-count interrupt (FM board).
    SoundCtc = 0,
    /// Z80 SIO receive/transmit/status interrupt (turbo only).
    Sio = 1,
    /// Z80 DMA end-of-block interrupt (turbo only).
    Dma = 2,
    /// Z80 CTC zero-count interrupt.
    Ctc = 3,
    /// Keyboard interrupt from the sub-CPU (the last link in the chain).
    Keyboard = 4,
}

impl IrqSource {
    const fn bit(self) -> u8 {
        1 << (self as u8)
    }

    /// Whether the source arbitrates multiple interrupt channels internally.
    /// Such a device may assert a new request while one of its channels is
    /// under service (a higher-priority channel nesting over a lower one);
    /// the device only asserts when its internal daisy chain allows it.
    const fn arbitrates_internally(self) -> bool {
        match self {
            IrqSource::SoundCtc | IrqSource::Ctc => true,
            IrqSource::Keyboard | IrqSource::Dma | IrqSource::Sio => false,
        }
    }

    /// The sources in descending priority order.
    const PRIORITY_ORDER: [IrqSource; 5] = [
        IrqSource::SoundCtc,
        IrqSource::Sio,
        IrqSource::Dma,
        IrqSource::Ctc,
        IrqSource::Keyboard,
    ];
}

save_state::runtime_state! {
/// Z80 mode-2 daisy-chain interrupt controller.
///
/// Beyond tracking which sources assert, the controller models the Z80
/// daisy-chain "interrupt under service" behaviour: once a source's interrupt
/// is acknowledged it holds the chain, blocking itself and every lower-priority
/// source from interrupting the CPU until the handler executes `RETI`. Only a
/// strictly higher-priority source may nest. Without this, a handler that
/// re-enables interrupts (`EI`) before returning would let its own recurring
/// source re-fire immediately and starve the main program.
///
/// Sources that arbitrate multiple interrupt channels internally (the CTCs)
/// are the exception: their device may assert again while under service when
/// a higher-priority internal channel nests over a lower one, so their
/// requests pass the chain even at their own priority level.
#[derive(Debug, Clone, Default)]
pub struct InterruptController {
    /// Bitmask of asserting sources, indexed by [`IrqSource`].
    pending: u8,
    /// Bitmask of sources whose interrupt has been acknowledged but not yet
    /// dismissed by `RETI`.
    in_service: u8,
}}

impl InterruptController {
    /// Creates a controller with no interrupt pending.
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn validate_runtime_state(&self) -> Result<(), save_state::StateValidationError> {
        const VALID_SOURCES: u8 = (1u8 << IrqSource::PRIORITY_ORDER.len()) - 1;
        if self.pending & !VALID_SOURCES != 0 || self.in_service & !VALID_SOURCES != 0 {
            return Err(save_state::StateValidationError::new(
                "X1 daisy-chain state is invalid",
            ));
        }
        Ok(())
    }

    /// Asserts `source`.
    pub fn raise(&mut self, source: IrqSource) {
        self.pending |= source.bit();
    }

    /// Deasserts `source`.
    pub fn clear(&mut self, source: IrqSource) {
        self.pending &= !source.bit();
    }

    /// Sets `source` to `asserted`.
    pub fn set(&mut self, source: IrqSource, asserted: bool) {
        if asserted {
            self.raise(source);
        } else {
            self.clear(source);
        }
    }

    /// The priority level (0 = highest) of the highest-priority source that is
    /// currently under service, or `None` if the chain is idle.
    fn service_threshold(&self) -> Option<u8> {
        (0..IrqSource::PRIORITY_ORDER.len() as u8)
            .find(|level| (self.in_service & (1 << level)) != 0)
    }

    /// Whether any interrupt can currently reach the CPU: a source is asserting
    /// that outranks every source under service.
    pub fn has_pending(&self) -> bool {
        self.highest_pending().is_some()
    }

    /// The highest-priority asserting source the daisy chain lets through, if
    /// any. Walking in priority order, a source under service holds the chain
    /// against itself and everything below it; a source that arbitrates
    /// multiple channels internally may re-request while under service, as its
    /// device only asserts when a higher-priority internal channel nests.
    pub fn highest_pending(&self) -> Option<IrqSource> {
        for source in IrqSource::PRIORITY_ORDER {
            let asserting = (self.pending & source.bit()) != 0;
            let under_service = (self.in_service & source.bit()) != 0;
            if asserting && (!under_service || source.arbitrates_internally()) {
                return Some(source);
            }
            if under_service {
                return None;
            }
        }
        None
    }

    /// Whether any source of strictly higher priority than `source` is under
    /// service; this is the state of `source`'s IEI input in the daisy chain.
    pub fn higher_in_service(&self, source: IrqSource) -> bool {
        (self.in_service & (source.bit() - 1)) != 0
    }

    /// Marks `source` as under service after its interrupt is acknowledged.
    pub fn begin_service(&mut self, source: IrqSource) {
        self.in_service |= source.bit();
    }

    /// Dismisses the highest-priority source under service, as an executed
    /// `RETI` clears the interrupt that is currently being handled. Returns
    /// the dismissed source so the bus can forward the `RETI` to its device.
    pub fn end_service(&mut self) -> Option<IrqSource> {
        let level = self.service_threshold()?;
        self.in_service &= !(1 << level);
        Some(IrqSource::PRIORITY_ORDER[level as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_interrupt_pending_by_default() {
        let controller = InterruptController::new();
        assert!(!controller.has_pending());
        assert_eq!(controller.highest_pending(), None);
    }

    #[test]
    fn ctc_outranks_keyboard() {
        let mut controller = InterruptController::new();
        controller.raise(IrqSource::Keyboard);
        controller.raise(IrqSource::Ctc);
        assert_eq!(controller.highest_pending(), Some(IrqSource::Ctc));
        controller.clear(IrqSource::Ctc);
        assert_eq!(controller.highest_pending(), Some(IrqSource::Keyboard));
    }

    #[test]
    fn turbo_sources_follow_the_daisy_chain_order() {
        let mut controller = InterruptController::new();
        controller.raise(IrqSource::Sio);
        controller.raise(IrqSource::Dma);
        controller.raise(IrqSource::Ctc);
        controller.raise(IrqSource::SoundCtc);
        controller.raise(IrqSource::Keyboard);
        assert_eq!(controller.highest_pending(), Some(IrqSource::SoundCtc));
        controller.clear(IrqSource::SoundCtc);
        assert_eq!(controller.highest_pending(), Some(IrqSource::Sio));
        controller.clear(IrqSource::Sio);
        assert_eq!(controller.highest_pending(), Some(IrqSource::Dma));
        controller.clear(IrqSource::Dma);
        assert_eq!(controller.highest_pending(), Some(IrqSource::Ctc));
        controller.clear(IrqSource::Ctc);
        assert_eq!(controller.highest_pending(), Some(IrqSource::Keyboard));
    }

    #[test]
    fn set_toggles_a_source() {
        let mut controller = InterruptController::new();
        controller.set(IrqSource::Ctc, true);
        assert!(controller.has_pending());
        controller.set(IrqSource::Ctc, false);
        assert!(!controller.has_pending());
    }

    #[test]
    fn a_source_under_service_blocks_itself_until_reti() {
        // A single-channel source (here the DMA) must not re-fire while its
        // own handler is under service, even when its line keeps asserting.
        let mut controller = InterruptController::new();
        controller.raise(IrqSource::Dma);
        assert_eq!(controller.highest_pending(), Some(IrqSource::Dma));

        controller.begin_service(IrqSource::Dma);
        // The line is still asserting, but the source now holds the chain.
        assert!(!controller.has_pending());
        assert_eq!(controller.highest_pending(), None);

        assert_eq!(controller.end_service(), Some(IrqSource::Dma));
        assert_eq!(controller.highest_pending(), Some(IrqSource::Dma));
    }

    #[test]
    fn an_internally_arbitrating_source_may_nest_over_itself() {
        // The CTC arbitrates its four channels internally: when its device
        // asserts while under service, a higher-priority channel is nesting
        // and the request must pass the chain.
        let mut controller = InterruptController::new();
        controller.begin_service(IrqSource::Ctc);
        assert_eq!(controller.highest_pending(), None);

        controller.raise(IrqSource::Ctc);
        assert_eq!(controller.highest_pending(), Some(IrqSource::Ctc));

        // A source under service above the CTC still holds the chain shut.
        controller.begin_service(IrqSource::Dma);
        assert_eq!(controller.highest_pending(), None);
    }

    #[test]
    fn a_higher_priority_source_still_nests() {
        let mut controller = InterruptController::new();
        controller.begin_service(IrqSource::Ctc);
        // The DMA outranks the CTC, so it may interrupt the CTC handler.
        controller.raise(IrqSource::Dma);
        assert_eq!(controller.highest_pending(), Some(IrqSource::Dma));
        // A lower-priority source stays blocked while the CTC is under service.
        controller.raise(IrqSource::Keyboard);
        controller.begin_service(IrqSource::Dma);
        assert_eq!(controller.highest_pending(), None);
    }

    #[test]
    fn reti_dismisses_the_highest_source_first() {
        let mut controller = InterruptController::new();
        controller.begin_service(IrqSource::Ctc);
        controller.begin_service(IrqSource::Dma);
        controller.raise(IrqSource::Keyboard);

        // Returning from the DMA handler still leaves the CTC in service, so
        // the lower-priority keyboard remains blocked.
        assert_eq!(controller.end_service(), Some(IrqSource::Dma));
        assert_eq!(controller.highest_pending(), None);

        // Returning from the CTC handler frees the chain.
        assert_eq!(controller.end_service(), Some(IrqSource::Ctc));
        assert_eq!(controller.highest_pending(), Some(IrqSource::Keyboard));
    }

    #[test]
    fn reti_on_an_idle_chain_dismisses_nothing() {
        let mut controller = InterruptController::new();
        assert_eq!(controller.end_service(), None);
    }
}
