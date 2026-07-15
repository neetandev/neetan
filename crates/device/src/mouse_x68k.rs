//! Sharp X68000 mouse packet protocol.

/// Host mouse input accumulated between controller polls.
#[derive(Debug, Default)]
pub struct MouseX68k {
    delta_x: i32,
    delta_y: i32,
    left: bool,
    right: bool,
}

impl MouseX68k {
    /// Creates an idle mouse.
    pub fn new() -> Self {
        Self::default()
    }

    /// Accumulates relative host movement.
    pub fn push_delta(&mut self, delta_x: i16, delta_y: i16) {
        self.delta_x = self.delta_x.saturating_add(i32::from(delta_x));
        self.delta_y = self.delta_y.saturating_add(i32::from(delta_y));
    }

    /// Sets the two mouse buttons.
    pub fn set_buttons(&mut self, left: bool, right: bool) {
        self.left = left;
        self.right = right;
    }

    /// Takes one status and movement packet.
    pub fn take_packet(&mut self) -> [u8; 3] {
        let (delta_x, x_status) = clamp_delta(self.delta_x, 0x10, 0x20);
        let (delta_y, y_status) = clamp_delta(self.delta_y, 0x40, 0x80);
        let mut status = x_status | y_status;
        if self.left {
            status |= 0x01;
        }
        if self.right {
            status |= 0x02;
        }
        self.delta_x = 0;
        self.delta_y = 0;
        [status, delta_x, delta_y]
    }
}

fn clamp_delta(delta: i32, overflow_bit: u8, underflow_bit: u8) -> (u8, u8) {
    if delta > 127 {
        (127, overflow_bit)
    } else if delta < -128 {
        (0x80, underflow_bit)
    } else {
        (delta as u8, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_contains_buttons_and_signed_movement() {
        let mut mouse = MouseX68k::new();
        mouse.push_delta(7, -5);
        mouse.set_buttons(true, false);
        assert_eq!(mouse.take_packet(), [0x01, 7, 0xFB]);
        assert_eq!(mouse.take_packet(), [0x01, 0, 0]);
    }

    #[test]
    fn packet_clamps_overflow() {
        let mut mouse = MouseX68k::new();
        mouse.push_delta(300, -300);
        assert_eq!(mouse.take_packet(), [0x90, 127, 0x80]);
    }
}
