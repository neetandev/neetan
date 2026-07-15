//! Sharp X1 mouse.
//!
//! The mouse is read through Z80 SIO channel 1. When the SIO channel 1 RTS
//! output makes a high-to-low transition the machine asks the mouse for a
//! report and feeds the three resulting bytes into the SIO channel 1 receive
//! buffer. The report is a status byte (movement-overflow flags plus the two
//! buttons) followed by the signed X and Y deltas accumulated since the last
//! report.

/// Accumulated-movement overflow flags in the status byte.
const STATUS_X_POSITIVE_OVERFLOW: u8 = 0x10;
const STATUS_X_NEGATIVE_OVERFLOW: u8 = 0x20;
const STATUS_Y_POSITIVE_OVERFLOW: u8 = 0x40;
const STATUS_Y_NEGATIVE_OVERFLOW: u8 = 0x80;

/// Button bits in the status byte and the host button mask.
const BUTTON_LEFT: u8 = 0x01;
const BUTTON_RIGHT: u8 = 0x02;

save_state::runtime_state! {
/// Sharp X1 mouse.
#[derive(Debug, Clone, Default)]
pub struct MouseX1 {
    delta_x: i32,
    delta_y: i32,
    buttons: u8,
}}

impl MouseX1 {
    /// Creates a mouse at rest.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears the accumulated movement and button state.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Accumulates relative movement since the last report.
    pub fn add_movement(&mut self, delta_x: i32, delta_y: i32) {
        self.delta_x = self.delta_x.saturating_add(delta_x);
        self.delta_y = self.delta_y.saturating_add(delta_y);
    }

    /// Sets the button state (bit 0 = left, bit 1 = right).
    pub fn set_buttons(&mut self, buttons: u8) {
        self.buttons = buttons & (BUTTON_LEFT | BUTTON_RIGHT);
    }

    /// Produces the three-byte report (status, X delta, Y delta) and resets the
    /// accumulated movement.
    pub fn report(&mut self) -> [u8; 3] {
        let mut status = 0u8;
        if self.delta_x >= 128 {
            status |= STATUS_X_POSITIVE_OVERFLOW;
        } else if self.delta_x < -128 {
            status |= STATUS_X_NEGATIVE_OVERFLOW;
        }
        if self.delta_y >= 128 {
            status |= STATUS_Y_POSITIVE_OVERFLOW;
        } else if self.delta_y < -128 {
            status |= STATUS_Y_NEGATIVE_OVERFLOW;
        }
        status |= self.buttons & (BUTTON_LEFT | BUTTON_RIGHT);

        let report = [status, self.delta_x as u8, self.delta_y as u8];
        self.delta_x = 0;
        self.delta_y = 0;
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_carries_signed_deltas_and_buttons() {
        let mut mouse = MouseX1::new();
        mouse.add_movement(5, -3);
        mouse.set_buttons(BUTTON_LEFT);
        let report = mouse.report();
        assert_eq!(report, [BUTTON_LEFT, 5, (-3i8) as u8]);
    }

    #[test]
    fn report_resets_the_accumulated_movement() {
        let mut mouse = MouseX1::new();
        mouse.add_movement(10, 20);
        let _ = mouse.report();
        let report = mouse.report();
        assert_eq!(report, [0x00, 0, 0]);
    }

    #[test]
    fn large_movement_sets_the_overflow_flags() {
        let mut mouse = MouseX1::new();
        mouse.add_movement(200, -200);
        let report = mouse.report();
        assert_eq!(
            report[0],
            STATUS_X_POSITIVE_OVERFLOW | STATUS_Y_NEGATIVE_OVERFLOW
        );
        assert_eq!(report[1], 200u8);
        assert_eq!(report[2], (-200i32) as u8);
    }
}
