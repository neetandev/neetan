//! Vectored interrupt controller.
//!
//! The Z80 runs in interrupt mode 2 with several prioritized sources. On
//! acknowledge the controller returns the vector of the highest-priority
//! pending source (lowest index wins) and clears it; the CPU forms the table
//! address from its I register and that vector.

/// Interrupt sources in priority order (lower index = higher priority). The
/// earlier machines deliver no sound/voice or vertical-retrace interrupt; those
/// sources are only raised on the SR generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IrqSource {
    /// Keyboard / cassette events delivered by the sub-CPU.
    SubCpu = 0,
    /// Joystick trigger.
    Joystick = 1,
    /// Periodic timer tick.
    Timer = 2,
    /// Sound/voice interrupt slot.
    Voice = 3,
    /// Vertical retrace (SR frame interrupt).
    Vrtc = 4,
}

impl IrqSource {
    /// Interrupt sources in priority order.
    const ALL: [Self; 5] = [
        Self::SubCpu,
        Self::Joystick,
        Self::Timer,
        Self::Voice,
        Self::Vrtc,
    ];
}

/// Result of an interrupt acknowledge cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptAcknowledge {
    /// Accepted interrupt source, or none when no source was pending.
    pub source: Option<IrqSource>,
    /// IM2 vector low byte returned to the processor.
    pub vector: u8,
}

/// Number of prioritized sources and SR programmable vector slots.
const SOURCE_COUNT: usize = 8;
/// Fixed acknowledge vector for the joystick source.
const JOYSTICK_VECTOR: u8 = 0x16;
/// Power-on timer acknowledge vector. The mkII reprograms it through port 0xF7.
const TIMER_VECTOR_DEFAULT: u8 = 0x06;
/// Fixed acknowledge vector for the uPD7752 voice source on the pre-SR machines.
const VOICE_VECTOR_DEFAULT: u8 = 0x20;

save_state::runtime_state! {
/// Prioritized interrupt controller. On the SR generation every source draws
/// its acknowledge vector from a programmable table (ports 0xB8-0xBF); the
/// earlier machines use fixed and dynamically latched vectors.
#[derive(Debug, Clone)]
pub struct InterruptController {
    pending: u8,
    sub_vector: u8,
    timer_vector: u8,
    programmable: bool,
    sr_vectors: [u8; SOURCE_COUNT],
}}

impl InterruptController {
    /// Creates a controller with no interrupts pending. `programmable` selects
    /// the SR vector-table behavior.
    pub fn new(programmable: bool) -> Self {
        Self {
            pending: 0,
            sub_vector: 0,
            timer_vector: TIMER_VECTOR_DEFAULT,
            programmable,
            sr_vectors: [0; SOURCE_COUNT],
        }
    }

    pub(crate) fn validate_runtime_state(&self) -> Result<(), save_state::StateValidationError> {
        const VALID_SOURCES: u8 = (1u8 << IrqSource::ALL.len()) - 1;
        if self.pending & !VALID_SOURCES != 0 {
            return Err(save_state::StateValidationError::new(
                "PC-6000 interrupt state is invalid",
            ));
        }
        Ok(())
    }

    /// Raises an interrupt source.
    pub fn raise(&mut self, source: IrqSource) {
        self.pending |= 1 << source as u8;
    }

    /// Clears an interrupt source without acknowledging it.
    pub fn clear(&mut self, source: IrqSource) {
        self.pending &= !(1 << source as u8);
    }

    /// Latches the sub-CPU acknowledge vector and raises the sub-CPU source.
    pub fn set_sub_vector(&mut self, vector: u8) {
        self.sub_vector = vector;
        self.raise(IrqSource::SubCpu);
    }

    /// Sets the timer acknowledge vector (mkII port 0xF7).
    pub fn set_timer_vector(&mut self, vector: u8) {
        self.timer_vector = vector;
    }

    /// Sets one SR programmable acknowledge vector (ports 0xB8-0xBF).
    pub fn set_sr_vector(&mut self, index: usize, vector: u8) {
        self.sr_vectors[index & (SOURCE_COUNT - 1)] = vector;
    }

    /// Reads back one SR programmable acknowledge vector.
    pub fn sr_vector(&self, index: usize) -> u8 {
        self.sr_vectors[index & (SOURCE_COUNT - 1)]
    }

    /// Whether any interrupt is pending.
    pub fn has_pending(&self) -> bool {
        self.pending != 0
    }

    /// Acknowledges an interrupt and reports its source and vector.
    pub fn acknowledge(&mut self) -> InterruptAcknowledge {
        for source in IrqSource::ALL {
            let index = source as usize;
            let mask = 1 << index;
            if self.pending & mask != 0 {
                self.pending &= !mask;
                let vector = match source {
                    IrqSource::SubCpu => self.sub_vector,
                    IrqSource::Joystick => JOYSTICK_VECTOR,
                    IrqSource::Timer => self.timer_vector,
                    _ if self.programmable => self.sr_vectors[index],
                    IrqSource::Voice => VOICE_VECTOR_DEFAULT,
                    IrqSource::Vrtc => self.timer_vector,
                };
                return InterruptAcknowledge {
                    source: Some(source),
                    vector,
                };
            }
        }
        InterruptAcknowledge {
            source: None,
            vector: self.timer_vector,
        }
    }
}

impl Default for InterruptController {
    fn default() -> Self {
        Self::new(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acknowledges_in_priority_order() {
        let mut controller = InterruptController::new(false);
        controller.raise(IrqSource::Timer);
        controller.set_sub_vector(0x02);

        // Sub-CPU outranks the timer.
        assert_eq!(controller.acknowledge().vector, 0x02);
        assert_eq!(controller.acknowledge().vector, 0x06);
        assert!(!controller.has_pending());
    }

    #[test]
    fn joystick_uses_its_fixed_vector() {
        let mut controller = InterruptController::new(false);
        controller.raise(IrqSource::Joystick);
        assert_eq!(controller.acknowledge().vector, JOYSTICK_VECTOR);
    }

    #[test]
    fn timer_vector_is_reprogrammable() {
        let mut controller = InterruptController::new(false);
        controller.raise(IrqSource::Timer);
        assert_eq!(controller.acknowledge().vector, TIMER_VECTOR_DEFAULT);

        controller.set_timer_vector(0x22);
        controller.raise(IrqSource::Timer);
        assert_eq!(controller.acknowledge().vector, 0x22);
    }

    #[test]
    fn function_key_vector_round_trips() {
        let mut controller = InterruptController::new(false);
        controller.set_sub_vector(0x14);
        assert!(controller.has_pending());
        assert_eq!(controller.acknowledge().vector, 0x14);
    }

    #[test]
    fn sr_acknowledge_uses_the_programmable_table() {
        let mut controller = InterruptController::new(true);
        controller.set_sr_vector(IrqSource::Vrtc as usize, 0x86);

        controller.set_sub_vector(0x02);
        controller.raise(IrqSource::Vrtc);
        // The sub-CPU outranks VRTC, but it keeps its latched vector.
        assert_eq!(controller.acknowledge().vector, 0x02);
        assert_eq!(controller.acknowledge().vector, 0x86);
        assert!(!controller.has_pending());
    }

    #[test]
    fn sr_vector_round_trips() {
        let mut controller = InterruptController::new(true);
        controller.set_sr_vector(IrqSource::Voice as usize, 0x8A);
        assert_eq!(controller.sr_vector(IrqSource::Voice as usize), 0x8A);
    }

    #[test]
    fn clear_drops_a_pending_source() {
        let mut controller = InterruptController::new(false);
        controller.raise(IrqSource::Timer);
        controller.clear(IrqSource::Timer);
        assert!(!controller.has_pending());
    }
}
