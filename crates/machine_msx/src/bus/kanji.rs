//! MSX Kanji character ROM interface.

/// Low five bits advanced by each data read.
const GLYPH_BYTE_MASK: u32 = 0x1F;
/// Writable six-bit address field.
const ADDRESS_FIELD_MASK: u8 = 0x3F;

/// MSX Kanji ROM address latch and character data.
pub(super) struct MsxKanjiRom {
    address: u32,
    rom: Vec<u8>,
}

save_state::runtime_state! {
/// Mutable MSX Kanji ROM address state.
#[derive(Clone)]
pub(super) struct MsxKanjiState {
    address: u32,
}}

impl MsxKanjiRom {
    /// Creates an interface with an erased ROM image.
    pub(super) fn new(size: usize) -> Self {
        Self {
            address: 0,
            rom: vec![0xFF; size],
        }
    }

    /// Installs the model's Kanji ROM bytes.
    pub(super) fn load(&mut self, rom: &[u8]) {
        self.rom.fill(0xFF);
        let count = self.rom.len().min(rom.len());
        self.rom[..count].copy_from_slice(&rom[..count]);
        self.address = 0;
    }

    /// Writes one address byte through D8, D9, DA, or DB.
    pub(super) fn write_address(&mut self, port: u8, value: u8) {
        if port & 1 == 0 {
            self.address = (self.address & 0x3F800) | (u32::from(value & ADDRESS_FIELD_MASK) << 5);
        } else {
            self.address = (self.address & 0x007E0) | (u32::from(value & ADDRESS_FIELD_MASK) << 11);
        }
    }

    /// Reads one byte from Kanji level one or two.
    pub(super) fn read_data(&mut self, level: u8) -> u8 {
        let index = self.address | (u32::from(level & 1) << 17);
        let Some(value) = self.rom.get(index as usize).copied() else {
            return 0xFF;
        };
        self.address =
            (self.address & !GLYPH_BYTE_MASK) | (self.address.wrapping_add(1) & GLYPH_BYTE_MASK);
        value
    }

    /// Captures the current glyph address.
    pub(super) const fn capture_state(&self) -> MsxKanjiState {
        MsxKanjiState {
            address: self.address,
        }
    }

    /// Restores the current glyph address.
    pub(super) fn restore_state(
        &mut self,
        state: MsxKanjiState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.address & !0x3_FFFF != 0 {
            return Err(save_state::StateValidationError::new(
                "MSX Kanji address is invalid",
            ));
        }
        self.address = state.address;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Address writes select both planes and data reads wrap within one glyph.
    fn address_planes_and_glyph_wrapping_are_independent() {
        let mut image = vec![0xFF; 0x40000];
        image[0x12340..0x12360].copy_from_slice(&(0..32u8).collect::<Vec<_>>());
        image[0x32340..0x32360].copy_from_slice(&(0x80..0xA0u8).collect::<Vec<_>>());
        let mut kanji = MsxKanjiRom::new(image.len());
        kanji.load(&image);

        kanji.write_address(0xD8, 0x1A);
        kanji.write_address(0xD9, 0x24);
        assert_eq!(kanji.read_data(0), 0);
        for expected in 1..32 {
            assert_eq!(kanji.read_data(0), expected);
        }
        assert_eq!(kanji.read_data(0), 0);

        kanji.write_address(0xDA, 0x1A);
        kanji.write_address(0xDB, 0x24);
        assert_eq!(kanji.read_data(1), 0x80);
    }

    #[test]
    /// An invalid plane read returns open bus without advancing the latch.
    fn invalid_plane_does_not_advance_the_address() {
        let mut kanji = MsxKanjiRom::new(0x20000);
        kanji.write_address(0xDA, 1);
        assert_eq!(kanji.read_data(1), 0xFF);
        kanji.rom[0x20] = 0xA5;
        assert_eq!(kanji.read_data(0), 0xA5);
    }
}
