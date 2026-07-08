use super::{AddressMode, M6809};

impl M6809 {
    pub(crate) fn execute_page2(&mut self, opcode: u8, bus: &mut impl common::Bus) -> i32 {
        match opcode {
            0x20 => self.long_branch(true, bus, 6),
            0x21 => self.long_branch(false, bus, 6),
            0x22 => self.long_branch(self.condition_hi(), bus, 6),
            0x23 => self.long_branch(!self.condition_hi(), bus, 6),
            0x24 => self.long_branch(!self.flags.carry, bus, 6),
            0x25 => self.long_branch(self.flags.carry, bus, 6),
            0x26 => self.long_branch(!self.flags.zero, bus, 6),
            0x27 => self.long_branch(self.flags.zero, bus, 6),
            0x28 => self.long_branch(!self.flags.overflow, bus, 6),
            0x29 => self.long_branch(self.flags.overflow, bus, 6),
            0x2A => self.long_branch(!self.flags.negative, bus, 6),
            0x2B => self.long_branch(self.flags.negative, bus, 6),
            0x2C => self.long_branch(self.condition_ge(), bus, 6),
            0x2D => self.long_branch(!self.condition_ge(), bus, 6),
            0x2E => self.long_branch(self.condition_gt(), bus, 6),
            0x2F => self.long_branch(!self.condition_gt(), bus, 6),
            0x3E => {
                self.x_swi2(bus);
                20
            }
            0x3F => {
                self.swi2(bus);
                20
            }
            0x83 => self.cmpd_mode(AddressMode::Immediate, bus),
            0x87 => {
                let _ = self.fetch_u8(bus);
                self.set_nzv8(self.a);
                3
            }
            0x8C => self.cmp_register_mode(AddressMode::Immediate, self.y, bus),
            0x8E => {
                let (value, cycles) = self.ld_register_mode(AddressMode::Immediate, bus);
                self.y = value;
                cycles
            }
            0x8F => self.st_register_mode(AddressMode::Immediate, 1, bus),
            0x93 => self.cmpd_mode(AddressMode::Direct, bus),
            0x9C => self.cmp_register_mode(AddressMode::Direct, self.y, bus),
            0x9E => {
                let (value, cycles) = self.ld_register_mode(AddressMode::Direct, bus);
                self.y = value;
                cycles
            }
            0x9F => self.st_register_mode(AddressMode::Direct, 1, bus),
            0xA3 => self.cmpd_mode(AddressMode::Indexed, bus),
            0xAC => self.cmp_register_mode(AddressMode::Indexed, self.y, bus),
            0xAE => {
                let (value, cycles) = self.ld_register_mode(AddressMode::Indexed, bus);
                self.y = value;
                cycles
            }
            0xAF => self.st_register_mode(AddressMode::Indexed, 1, bus),
            0xB3 => self.cmpd_mode(AddressMode::Extended, bus),
            0xBC => self.cmp_register_mode(AddressMode::Extended, self.y, bus),
            0xBE => {
                let (value, cycles) = self.ld_register_mode(AddressMode::Extended, bus);
                self.y = value;
                cycles
            }
            0xBF => self.st_register_mode(AddressMode::Extended, 1, bus),
            0xC3 => self.xadd16_mode(AddressMode::Immediate, self.d(), bus),
            0xC7 => {
                let _ = self.fetch_u8(bus);
                self.set_nzv8(self.b);
                3
            }
            0xCE => {
                let (value, cycles) = self.ld_register_mode(AddressMode::Immediate, bus);
                self.s = value;
                self.mark_s_loaded();
                cycles
            }
            0xCF => self.st_register_mode(AddressMode::Immediate, 3, bus),
            0xD3 => self.xadd16_mode(AddressMode::Direct, self.d(), bus),
            0xDE => {
                let (value, cycles) = self.ld_register_mode(AddressMode::Direct, bus);
                self.s = value;
                self.mark_s_loaded();
                cycles
            }
            0xDF => self.st_register_mode(AddressMode::Direct, 3, bus),
            0xE3 => self.xadd16_mode(AddressMode::Indexed, self.d(), bus),
            0xEE => {
                let (value, cycles) = self.ld_register_mode(AddressMode::Indexed, bus);
                self.s = value;
                self.mark_s_loaded();
                cycles
            }
            0xEF => self.st_register_mode(AddressMode::Indexed, 3, bus),
            0xF3 => self.xadd16_mode(AddressMode::Extended, self.d(), bus),
            0xFE => {
                let (value, cycles) = self.ld_register_mode(AddressMode::Extended, bus);
                self.s = value;
                self.mark_s_loaded();
                cycles
            }
            0xFF => self.st_register_mode(AddressMode::Extended, 3, bus),
            0x00..=0x1F
            | 0x30..=0x3D
            | 0x40..=0x82
            | 0x84..=0x86
            | 0x88..=0x8B
            | 0x8D
            | 0x90..=0x92
            | 0x94..=0x9B
            | 0x9D
            | 0xA0..=0xA2
            | 0xA4..=0xAB
            | 0xAD
            | 0xB0..=0xB2
            | 0xB4..=0xBB
            | 0xBD
            | 0xC0..=0xC2
            | 0xC4..=0xC6
            | 0xC8..=0xCD
            | 0xD0..=0xD2
            | 0xD4..=0xDD
            | 0xE0..=0xE2
            | 0xE4..=0xED
            | 0xF0..=0xF2
            | 0xF4..=0xFD => 2,
        }
    }
}
