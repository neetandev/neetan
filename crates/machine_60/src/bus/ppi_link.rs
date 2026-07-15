//! i8255 PPI wiring for the PC-6001.
//!
//! Port A reads return the keycode latched by the sub-controller; port A
//! writes carry sub-controller commands. The control register doubles as a
//! bank-select latch: certain control words swap what the 0x6000 window
//! exposes. Port C reads return a handshake shadow the firmware polls.

use device::i8255::I8255;

use crate::memory::BankWindow;

/// Side effect of a PPI write that the bus must apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PpiEffect {
    /// Nothing for the bus to do.
    None,
    /// Switch the 0x6000 bank window.
    SetBank(BankWindow),
    /// A sub-controller command was issued through port A.
    SubCommand(u8),
}

/// Control-word handshake bits the firmware always expects to read back.
const HANDSHAKE_BITS: u8 = 0xA8;

/// Port C bit that gates the CRT display (and thus the VRAM bus-request stall):
/// set enables the display, clear blanks it.
const PORT_C_CRT_ENABLE_BIT: u8 = 1;

/// PPI plus the handshake shadow exposed on port C.
pub(crate) struct PpiLink {
    ppi: I8255,
    port_c_shadow: u8,
    crt_enabled: bool,
}

save_state::runtime_state! {
/// Authoritative state of the linked PPI handshake lines.
#[derive(Clone)]
pub(crate) struct PpiLinkState {
    ppi: device::i8255::I8255State,
    port_c_shadow: u8,
    crt_enabled: bool,
}}

impl PpiLink {
    /// Creates a PPI in the power-on state (display active).
    pub(crate) fn new() -> Self {
        Self {
            ppi: I8255::new(),
            port_c_shadow: 0,
            crt_enabled: true,
        }
    }

    pub(crate) fn capture_state(&self) -> PpiLinkState {
        PpiLinkState {
            ppi: self.ppi.state.clone(),
            port_c_shadow: self.port_c_shadow,
            crt_enabled: self.crt_enabled,
        }
    }

    pub(crate) fn restore_state(&mut self, state: PpiLinkState) {
        self.ppi.state = state.ppi;
        self.port_c_shadow = state.port_c_shadow;
        self.crt_enabled = state.crt_enabled;
    }

    /// Whether the CRT display is enabled. While it is, the video circuit steals
    /// the bus from the CPU for part of every active scanline.
    pub(crate) fn crt_enabled(&self) -> bool {
        self.crt_enabled
    }

    /// Reads a PPI register. `keycode` is the sub-controller's latched key.
    pub(crate) fn read(&self, offset: u8, keycode: u8) -> u8 {
        match offset & 0x03 {
            0 => keycode,
            2 => self.port_c_shadow,
            other => self.ppi.read(other),
        }
    }

    /// Writes a PPI register, returning the side effect for the bus to apply.
    pub(crate) fn write(&mut self, offset: u8, value: u8) -> PpiEffect {
        let effect = match offset & 0x03 {
            0 => PpiEffect::SubCommand(value),
            3 => self.control_write(value),
            _ => PpiEffect::None,
        };
        self.ppi.write(offset, value);
        effect
    }

    fn control_write(&mut self, value: u8) -> PpiEffect {
        // The control word's bit set/reset form drives the port C handshake.
        let bit = (value >> 1) & 0x07;
        if value & 0x01 != 0 {
            self.port_c_shadow |= 1 << bit;
        } else {
            self.port_c_shadow &= !(1 << bit);
        }
        if bit == PORT_C_CRT_ENABLE_BIT {
            self.crt_enabled = value & 0x01 != 0;
        }
        self.port_c_shadow |= HANDSHAKE_BITS;

        match value & 0x0F {
            0x05 => PpiEffect::SetBank(BankWindow::CartridgeUpper),
            0x04 => PpiEffect::SetBank(BankWindow::CharacterGenerator),
            _ => PpiEffect::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_a_read_returns_keycode() {
        let ppi = PpiLink::new();
        assert_eq!(ppi.read(0, 0x41), 0x41);
    }

    #[test]
    fn control_word_selects_cartridge_bank() {
        let mut ppi = PpiLink::new();
        assert_eq!(
            ppi.write(3, 0x05),
            PpiEffect::SetBank(BankWindow::CartridgeUpper)
        );
        assert_eq!(
            ppi.write(3, 0x04),
            PpiEffect::SetBank(BankWindow::CharacterGenerator)
        );
    }

    #[test]
    fn port_a_write_is_a_sub_command() {
        let mut ppi = PpiLink::new();
        assert_eq!(ppi.write(0, 0x06), PpiEffect::SubCommand(0x06));
    }
}
