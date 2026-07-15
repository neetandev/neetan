//! Microsoft three-byte serial mouse protocol.

/// Identification byte sent after a serial mouse reset.
pub const SERIAL_MOUSE_IDENTIFICATION: u8 = b'M';

/// Microsoft serial mouse state accumulated from the host.
#[derive(Debug, Default)]
pub struct SerialMouse {
    delta_x: i32,
    delta_y: i32,
    left: bool,
    right: bool,
    dirty: bool,
}

impl SerialMouse {
    /// Builds a mouse with no pending movement.
    pub fn new() -> Self {
        Self::default()
    }

    /// Accumulates relative host movement.
    pub fn push_delta(&mut self, delta_x: i16, delta_y: i16) {
        if delta_x != 0 || delta_y != 0 {
            self.delta_x = self.delta_x.saturating_add(i32::from(delta_x));
            self.delta_y = self.delta_y.saturating_add(i32::from(delta_y));
            self.dirty = true;
        }
    }

    /// Updates the two mouse buttons.
    pub fn set_buttons(&mut self, left: bool, right: bool) {
        if left != self.left || right != self.right {
            self.left = left;
            self.right = right;
            self.dirty = true;
        }
    }

    /// Takes the next pending Microsoft protocol packet.
    pub fn take_packet(&mut self) -> Option<[u8; 3]> {
        if !self.dirty {
            return None;
        }
        let packet = encode_packet(self.delta_x, self.delta_y, self.left, self.right);
        self.delta_x = 0;
        self.delta_y = 0;
        self.dirty = false;
        Some(packet)
    }
}

/// Encodes a Microsoft three-byte mouse packet.
fn encode_packet(delta_x: i32, delta_y: i32, left: bool, right: bool) -> [u8; 3] {
    let delta_x = delta_x.clamp(-128, 127);
    let delta_y = delta_y.clamp(-128, 127);
    let byte0 = 0x40
        | if left { 0x20 } else { 0 }
        | if right { 0x10 } else { 0 }
        | (((delta_y >> 6) & 0x03) as u8) << 2
        | ((delta_x >> 6) & 0x03) as u8;
    [byte0, (delta_x & 0x3F) as u8, (delta_y & 0x3F) as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_movement_and_buttons() {
        assert_eq!(encode_packet(5, -3, true, false), [0x6C, 0x05, 0x3D]);
    }

    #[test]
    fn take_packet_clears_accumulated_movement() {
        let mut mouse = SerialMouse::new();
        mouse.push_delta(5, -3);
        assert_eq!(mouse.take_packet(), Some([0x4C, 0x05, 0x3D]));
        assert_eq!(mouse.take_packet(), None);
    }

    #[test]
    fn clamps_large_deltas_and_packs_high_bits() {
        let [byte0, byte1, byte2] = encode_packet(300, -300, false, false);
        assert_eq!(byte0 & 0x03, 0x01);
        assert_eq!((byte0 >> 2) & 0x03, 0x02);
        assert_eq!(byte1, 127 & 0x3F);
        assert_eq!(byte2, (-128i32 & 0x3F) as u8);
    }
}
