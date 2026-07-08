use super::{AddressMode, M6809};

impl M6809 {
    pub(crate) fn execute_page3(&mut self, opcode: u8, bus: &mut impl common::Bus) -> i32 {
        match opcode {
            0x3E => {
                self.x_firq(bus);
                20
            }
            0x3F => {
                self.swi3(bus);
                20
            }
            0x83 => self.cmp_register_mode(AddressMode::Immediate, self.u, bus),
            0x87 => {
                let _ = self.fetch_u8(bus);
                self.set_nzv8(self.a);
                3
            }
            0x8C => self.cmp_register_mode(AddressMode::Immediate, self.s, bus),
            0x8F => self.st_register_mode(AddressMode::Immediate, 0, bus),
            0x93 => self.cmp_register_mode(AddressMode::Direct, self.u, bus),
            0x9C => self.cmp_register_mode(AddressMode::Direct, self.s, bus),
            0xA3 => self.cmp_register_mode(AddressMode::Indexed, self.u, bus),
            0xAC => self.cmp_register_mode(AddressMode::Indexed, self.s, bus),
            0xB3 => self.cmp_register_mode(AddressMode::Extended, self.u, bus),
            0xBC => self.cmp_register_mode(AddressMode::Extended, self.s, bus),
            0xC3 => self.xadd16_mode(AddressMode::Immediate, self.u, bus),
            0xC7 => {
                let _ = self.fetch_u8(bus);
                self.set_nzv8(self.b);
                3
            }
            0xCF => self.st_register_mode(AddressMode::Immediate, 2, bus),
            0xD3 => self.xadd16_mode(AddressMode::Direct, self.u, bus),
            0xE3 => self.xadd16_mode(AddressMode::Indexed, self.u, bus),
            0xF3 => self.xadd16_mode(AddressMode::Extended, self.u, bus),
            0x00..=0x3D
            | 0x40..=0x82
            | 0x84..=0x86
            | 0x88..=0x8B
            | 0x8D..=0x8E
            | 0x90..=0x92
            | 0x94..=0x9B
            | 0x9D..=0xA2
            | 0xA4..=0xAB
            | 0xAD..=0xB2
            | 0xB4..=0xBB
            | 0xBD..=0xC2
            | 0xC4..=0xC6
            | 0xC8..=0xCE
            | 0xD0..=0xD2
            | 0xD4..=0xE2
            | 0xE4..=0xF2
            | 0xF4..=0xFF => 2,
        }
    }
}
