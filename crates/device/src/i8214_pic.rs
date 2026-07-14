//! uPB8214 eight-level priority interrupt controller for the PC-8801.
//!
//! The PC-88 wires logical interrupt sources to the controller so that lower
//! level numbers have higher priority (level 0 = RXRDY highest, level 7 lowest).
//! The Z80 runs in IM 2, so an accepted interrupt returns the low byte of the
//! IM2 vector, which is `level * 2`.
//!
//! Two I/O ports drive it: port 0xE4 sets the priority threshold (only requests
//! at levels below the written value are eligible), and port 0xE6 sets the
//! low-three-source enable mask. Accepting an interrupt sets an internal
//! interrupt-disable latch that blocks further requests until port 0xE4 is
//! written again, matching the device's interrupt-disable flip-flop.

use std::ops::{Deref, DerefMut};

/// RXRDY (i8251 USART receive). Highest priority.
pub const LEVEL_RXRDY: u8 = 0;
/// VRTC (CRTC vertical retrace / end of display).
pub const LEVEL_VRTC: u8 = 1;
/// CLOCK (600 Hz periodic system timer).
pub const LEVEL_CLOCK: u8 = 2;
/// INT3 (expansion; unused on the bare MA).
pub const LEVEL_INT3: u8 = 3;
/// INT4 (OPN/OPNA sound timer; gated by 0x32 bit 7 SINTM).
pub const LEVEL_INT4: u8 = 4;
/// INT5 (expansion).
pub const LEVEL_INT5: u8 = 5;
/// FDCINT1 (unused on the bare MA; FDC IRQ goes to the sub CPU).
pub const LEVEL_FDCINT1: u8 = 6;
/// FDCINT2 (unused on the bare MA). Lowest priority.
pub const LEVEL_FDCINT2: u8 = 7;

/// Port 0xE6 enable bit for the CLOCK source (level 2).
const MASK_BIT_CLOCK: u8 = 0x01;
/// Port 0xE6 enable bit for the VRTC source (level 1).
const MASK_BIT_VRTC: u8 = 0x02;
/// Port 0xE6 enable bit for the RXRDY source (level 0).
const MASK_BIT_RXRDY: u8 = 0x04;

/// Levels 3-7 are not gated by port 0xE6; they are always enabled.
const ALWAYS_ENABLED_LEVELS: u8 = 0b1111_1000;

/// Maximum priority value (all eight levels eligible).
const PRIORITY_ALL: u8 = 8;

/// Snapshot of the i8214 priority interrupt controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I8214PicState {
    /// Pending request bit per level (bit `n` set = level `n` requesting).
    pub request: u8,
    /// Per-level enable mask from port 0xE6 (levels 3-7 always enabled).
    pub enable_mask: u8,
    /// Priority threshold mask from port 0xE4: bit `n` set = level `n` eligible.
    pub priority_mask: u8,
    /// Interrupt-disable latch: set on acknowledge, cleared by a 0xE4 write.
    pub interrupt_disabled: bool,
}

/// Result of an interrupt acknowledge cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct I8214Acknowledge {
    /// Accepted controller level, or none when no request was eligible.
    pub level: Option<u8>,
    /// IM2 vector low byte returned to the processor.
    pub vector: u8,
}

/// uPB8214 eight-level priority interrupt controller.
pub struct I8214Pic {
    /// Embedded state for save/restore.
    pub state: I8214PicState,
}

impl Deref for I8214Pic {
    type Target = I8214PicState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for I8214Pic {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Default for I8214Pic {
    fn default() -> Self {
        Self::new()
    }
}

impl I8214Pic {
    /// Creates a controller in its power-on reset state: no requests, all
    /// sources masked, no levels eligible, interrupts enabled.
    pub fn new() -> Self {
        Self {
            state: I8214PicState {
                request: 0,
                enable_mask: ALWAYS_ENABLED_LEVELS,
                priority_mask: 0,
                interrupt_disabled: false,
            },
        }
    }

    /// Resets the controller to its power-on state.
    pub fn reset(&mut self) {
        self.state = Self::new().state;
    }

    /// Raises the request line for `level`.
    pub fn set_request(&mut self, level: u8) {
        self.state.request |= 1 << level;
    }

    /// Clears the request line for `level`.
    pub fn clear_request(&mut self, level: u8) {
        self.state.request &= !(1 << level);
    }

    /// Writes the priority register (port 0xE4): levels below `value` become
    /// eligible. Re-enables the controller after an acknowledgment.
    pub fn write_priority(&mut self, value: u8) {
        let levels = value.min(PRIORITY_ALL);
        self.state.priority_mask = if levels >= PRIORITY_ALL {
            0xFF
        } else {
            (1u8 << levels) - 1
        };
        self.state.interrupt_disabled = false;
    }

    /// Writes the source enable mask (port 0xE6): bit 0 unmasks CLOCK, bit 1
    /// VRTC, bit 2 RXRDY. Levels 3-7 stay enabled.
    pub fn write_mask(&mut self, value: u8) {
        let mut mask = ALWAYS_ENABLED_LEVELS;
        if value & MASK_BIT_CLOCK != 0 {
            mask |= 1 << LEVEL_CLOCK;
        }
        if value & MASK_BIT_VRTC != 0 {
            mask |= 1 << LEVEL_VRTC;
        }
        if value & MASK_BIT_RXRDY != 0 {
            mask |= 1 << LEVEL_RXRDY;
        }
        self.state.enable_mask = mask;
    }

    /// Returns the set of currently eligible request bits.
    fn eligible(&self) -> u8 {
        if self.state.interrupt_disabled {
            0
        } else {
            self.state.request & self.state.priority_mask & self.state.enable_mask
        }
    }

    /// Returns `true` if an eligible interrupt request is pending.
    pub fn has_pending_irq(&self) -> bool {
        self.eligible() != 0
    }

    /// Accepts an interrupt and reports its controller level and vector.
    pub fn acknowledge(&mut self) -> I8214Acknowledge {
        let eligible = self.eligible();
        if eligible == 0 {
            return I8214Acknowledge {
                level: None,
                vector: 0,
            };
        }
        let level = eligible.trailing_zeros() as u8;
        self.state.request &= !(1 << level);
        self.state.interrupt_disabled = true;
        I8214Acknowledge {
            level: Some(level),
            vector: level * 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_all() -> I8214Pic {
        let mut pic = I8214Pic::new();
        pic.write_mask(MASK_BIT_CLOCK | MASK_BIT_VRTC | MASK_BIT_RXRDY);
        pic.write_priority(PRIORITY_ALL);
        pic
    }

    #[test]
    fn lower_level_wins_priority() {
        let mut pic = enabled_all();
        pic.set_request(LEVEL_CLOCK);
        pic.set_request(LEVEL_VRTC);
        assert!(pic.has_pending_irq());
        // VRTC (level 1) outranks CLOCK (level 2).
        assert_eq!(pic.acknowledge().vector, LEVEL_VRTC * 2);
    }

    #[test]
    fn vector_is_level_times_two() {
        let mut pic = enabled_all();
        pic.set_request(LEVEL_CLOCK);
        assert_eq!(pic.acknowledge().vector, 0x04);

        pic.write_priority(PRIORITY_ALL);
        pic.set_request(LEVEL_RXRDY);
        assert_eq!(pic.acknowledge().vector, 0x00);
    }

    #[test]
    fn mask_gates_source() {
        let mut pic = I8214Pic::new();
        pic.write_priority(PRIORITY_ALL);
        // CLOCK left masked.
        pic.write_mask(MASK_BIT_VRTC);
        pic.set_request(LEVEL_CLOCK);
        assert!(!pic.has_pending_irq());

        // Request stays latched; unmasking reveals it.
        pic.write_mask(MASK_BIT_CLOCK);
        assert!(pic.has_pending_irq());
        assert_eq!(pic.acknowledge().vector, LEVEL_CLOCK * 2);
    }

    #[test]
    fn mask_bit_to_level_remap() {
        let mut pic = I8214Pic::new();
        pic.write_priority(PRIORITY_ALL);
        pic.write_mask(MASK_BIT_RXRDY);
        pic.set_request(LEVEL_RXRDY);
        pic.set_request(LEVEL_VRTC);
        // Only RXRDY (bit 2 of the port value) is unmasked.
        assert_eq!(pic.acknowledge().vector, LEVEL_RXRDY * 2);
    }

    #[test]
    fn priority_threshold_gates_levels() {
        let mut pic = I8214Pic::new();
        pic.write_mask(MASK_BIT_CLOCK);
        // Threshold of 2 makes only levels 0 and 1 eligible; CLOCK is level 2.
        pic.write_priority(2);
        pic.set_request(LEVEL_CLOCK);
        assert!(!pic.has_pending_irq());

        pic.write_priority(3);
        assert!(pic.has_pending_irq());
    }

    #[test]
    fn acknowledge_latches_disabled_until_priority_rewrite() {
        let mut pic = enabled_all();
        pic.set_request(LEVEL_CLOCK);
        assert_eq!(pic.acknowledge().vector, LEVEL_CLOCK * 2);

        // A fresh request cannot be delivered while disabled.
        pic.set_request(LEVEL_CLOCK);
        assert!(!pic.has_pending_irq());

        // Re-arming via port 0xE4 clears the latch.
        pic.write_priority(PRIORITY_ALL);
        assert!(pic.has_pending_irq());
    }

    #[test]
    fn higher_levels_not_gated_by_mask() {
        let mut pic = I8214Pic::new();
        pic.write_priority(PRIORITY_ALL);
        pic.write_mask(0);
        pic.set_request(LEVEL_INT4);
        assert!(pic.has_pending_irq());
        assert_eq!(pic.acknowledge().vector, LEVEL_INT4 * 2);
    }
}
