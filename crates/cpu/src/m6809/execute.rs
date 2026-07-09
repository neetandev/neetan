use super::{AddressMode, EffectiveAddress, M6809, M6809Flags};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnaryOp {
    Neg,
    Xnc,
    Com,
    Lsr,
    Ror,
    Asr,
    Asl,
    Rol,
    Dec,
    Xdec,
    Inc,
    Tst,
    Clr,
    Xclr,
}

impl M6809 {
    pub(crate) fn execute_base(&mut self, opcode: u8, bus: &mut impl common::Bus) -> i32 {
        match opcode {
            0x00 | 0x01 => self.execute_memory_unary(AddressMode::Direct, UnaryOp::Neg, 6, bus),
            0x02 => self.execute_memory_unary(AddressMode::Direct, UnaryOp::Xnc, 6, bus),
            0x03 => self.execute_memory_unary(AddressMode::Direct, UnaryOp::Com, 6, bus),
            0x04 | 0x05 => self.execute_memory_unary(AddressMode::Direct, UnaryOp::Lsr, 6, bus),
            0x06 => self.execute_memory_unary(AddressMode::Direct, UnaryOp::Ror, 6, bus),
            0x07 => self.execute_memory_unary(AddressMode::Direct, UnaryOp::Asr, 6, bus),
            0x08 => self.execute_memory_unary(AddressMode::Direct, UnaryOp::Asl, 6, bus),
            0x09 => self.execute_memory_unary(AddressMode::Direct, UnaryOp::Rol, 6, bus),
            0x0A => self.execute_memory_unary(AddressMode::Direct, UnaryOp::Dec, 6, bus),
            0x0B => self.execute_memory_unary(AddressMode::Direct, UnaryOp::Xdec, 6, bus),
            0x0C => self.execute_memory_unary(AddressMode::Direct, UnaryOp::Inc, 6, bus),
            0x0D => self.execute_memory_unary(AddressMode::Direct, UnaryOp::Tst, 6, bus),
            0x0E => {
                self.pc = self.direct_address(bus);
                3
            }
            0x0F => self.execute_memory_unary(AddressMode::Direct, UnaryOp::Clr, 6, bus),
            0x12 | 0x1B => 2,
            0x13 => {
                self.sync();
                2
            }
            0x14 | 0x15 | 0xCD => {
                self.halted = true;
                1
            }
            0x16 => self.long_branch(true, bus, 5),
            0x17 => self.long_bsr(bus),
            0x18 => self.x18(bus),
            0x19 => {
                self.daa();
                2
            }
            0x1A => {
                let value = self.fetch_u8(bus);
                let condition_code = self.flags.compress();
                self.flags.expand(condition_code | value);
                3
            }
            0x1C => {
                let value = self.fetch_u8(bus);
                let condition_code = self.flags.compress();
                self.flags.expand(condition_code & value);
                3
            }
            0x1D => {
                self.sex();
                2
            }
            0x1E => self.exg(bus),
            0x1F => self.tfr(bus),
            0x20 => self.short_branch(true, bus),
            0x21 => self.short_branch(false, bus),
            0x22 => self.short_branch(self.condition_hi(), bus),
            0x23 => self.short_branch(!self.condition_hi(), bus),
            0x24 => self.short_branch(!self.flags.carry, bus),
            0x25 => self.short_branch(self.flags.carry, bus),
            0x26 => self.short_branch(!self.flags.zero, bus),
            0x27 => self.short_branch(self.flags.zero, bus),
            0x28 => self.short_branch(!self.flags.overflow, bus),
            0x29 => self.short_branch(self.flags.overflow, bus),
            0x2A => self.short_branch(!self.flags.negative, bus),
            0x2B => self.short_branch(self.flags.negative, bus),
            0x2C => self.short_branch(self.condition_ge(), bus),
            0x2D => self.short_branch(!self.condition_ge(), bus),
            0x2E => self.short_branch(self.condition_gt(), bus),
            0x2F => self.short_branch(!self.condition_gt(), bus),
            0x30 => self.lea(AddressMode::Indexed, 0, bus),
            0x31 => self.lea(AddressMode::Indexed, 1, bus),
            0x32 => self.lea(AddressMode::Indexed, 2, bus),
            0x33 => self.lea(AddressMode::Indexed, 3, bus),
            0x34 => self.push_or_pull(true, true, bus),
            0x35 => self.push_or_pull(true, false, bus),
            0x36 => self.push_or_pull(false, true, bus),
            0x37 => self.push_or_pull(false, false, bus),
            0x38 => {
                self.clk(1);
                let value = self.fetch_u8(bus);
                let condition_code = self.flags.compress();
                self.flags.expand(condition_code & value);
                4
            }
            0x39 => {
                self.pc = self.pull_s_word(bus);
                5
            }
            0x3A => {
                self.x = self.x.wrapping_add(u16::from(self.b));
                3
            }
            0x3B => self.rti(bus),
            0x3C => {
                self.cwai(bus);
                20
            }
            0x3D => {
                self.mul();
                11
            }
            0x3E => {
                self.x_reset(bus);
                19
            }
            0x3F => {
                self.swi(bus);
                19
            }
            0x40 | 0x41 => self.execute_register_unary(true, UnaryOp::Neg),
            0x42 => self.execute_register_unary(true, UnaryOp::Xnc),
            0x43 => self.execute_register_unary(true, UnaryOp::Com),
            0x44 | 0x45 => self.execute_register_unary(true, UnaryOp::Lsr),
            0x46 => self.execute_register_unary(true, UnaryOp::Ror),
            0x47 => self.execute_register_unary(true, UnaryOp::Asr),
            0x48 => self.execute_register_unary(true, UnaryOp::Asl),
            0x49 => self.execute_register_unary(true, UnaryOp::Rol),
            0x4A => self.execute_register_unary(true, UnaryOp::Dec),
            0x4B => self.execute_register_unary(true, UnaryOp::Xdec),
            0x4C => self.execute_register_unary(true, UnaryOp::Inc),
            0x4D => self.execute_register_unary(true, UnaryOp::Tst),
            0x4E => self.execute_register_unary(true, UnaryOp::Xclr),
            0x4F => self.execute_register_unary(true, UnaryOp::Clr),
            0x50 | 0x51 => self.execute_register_unary(false, UnaryOp::Neg),
            0x52 => self.execute_register_unary(false, UnaryOp::Xnc),
            0x53 => self.execute_register_unary(false, UnaryOp::Com),
            0x54 | 0x55 => self.execute_register_unary(false, UnaryOp::Lsr),
            0x56 => self.execute_register_unary(false, UnaryOp::Ror),
            0x57 => self.execute_register_unary(false, UnaryOp::Asr),
            0x58 => self.execute_register_unary(false, UnaryOp::Asl),
            0x59 => self.execute_register_unary(false, UnaryOp::Rol),
            0x5A => self.execute_register_unary(false, UnaryOp::Dec),
            0x5B => self.execute_register_unary(false, UnaryOp::Xdec),
            0x5C => self.execute_register_unary(false, UnaryOp::Inc),
            0x5D => self.execute_register_unary(false, UnaryOp::Tst),
            0x5E => self.execute_register_unary(false, UnaryOp::Xclr),
            0x5F => self.execute_register_unary(false, UnaryOp::Clr),
            0x60 | 0x61 => self.execute_memory_unary(AddressMode::Indexed, UnaryOp::Neg, 6, bus),
            0x62 => self.execute_memory_unary(AddressMode::Indexed, UnaryOp::Xnc, 6, bus),
            0x63 => self.execute_memory_unary(AddressMode::Indexed, UnaryOp::Com, 6, bus),
            0x64 | 0x65 => self.execute_memory_unary(AddressMode::Indexed, UnaryOp::Lsr, 6, bus),
            0x66 => self.execute_memory_unary(AddressMode::Indexed, UnaryOp::Ror, 6, bus),
            0x67 => self.execute_memory_unary(AddressMode::Indexed, UnaryOp::Asr, 6, bus),
            0x68 => self.execute_memory_unary(AddressMode::Indexed, UnaryOp::Asl, 6, bus),
            0x69 => self.execute_memory_unary(AddressMode::Indexed, UnaryOp::Rol, 6, bus),
            0x6A => self.execute_memory_unary(AddressMode::Indexed, UnaryOp::Dec, 6, bus),
            0x6B => self.execute_memory_unary(AddressMode::Indexed, UnaryOp::Xdec, 6, bus),
            0x6C => self.execute_memory_unary(AddressMode::Indexed, UnaryOp::Inc, 6, bus),
            0x6D => self.execute_memory_unary(AddressMode::Indexed, UnaryOp::Tst, 6, bus),
            0x6E => {
                let ea = self.address_for_mode(AddressMode::Indexed, bus);
                self.pc = ea.address;
                3 + ea.extra_cycles
            }
            0x6F => self.execute_memory_unary(AddressMode::Indexed, UnaryOp::Clr, 6, bus),
            0x70 | 0x71 => self.execute_memory_unary(AddressMode::Extended, UnaryOp::Neg, 7, bus),
            0x72 => self.execute_memory_unary(AddressMode::Extended, UnaryOp::Xnc, 7, bus),
            0x73 => self.execute_memory_unary(AddressMode::Extended, UnaryOp::Com, 7, bus),
            0x74 | 0x75 => self.execute_memory_unary(AddressMode::Extended, UnaryOp::Lsr, 7, bus),
            0x76 => self.execute_memory_unary(AddressMode::Extended, UnaryOp::Ror, 7, bus),
            0x77 => self.execute_memory_unary(AddressMode::Extended, UnaryOp::Asr, 7, bus),
            0x78 => self.execute_memory_unary(AddressMode::Extended, UnaryOp::Asl, 7, bus),
            0x79 => self.execute_memory_unary(AddressMode::Extended, UnaryOp::Rol, 7, bus),
            0x7A => self.execute_memory_unary(AddressMode::Extended, UnaryOp::Dec, 7, bus),
            0x7B => self.execute_memory_unary(AddressMode::Extended, UnaryOp::Xdec, 7, bus),
            0x7C => self.execute_memory_unary(AddressMode::Extended, UnaryOp::Inc, 7, bus),
            0x7D => self.execute_memory_unary(AddressMode::Extended, UnaryOp::Tst, 7, bus),
            0x7E => {
                self.pc = self.fetch_u16(bus);
                4
            }
            0x7F => self.begin_extended_clear(bus),
            0x80..=0xFF => {
                if opcode == 0x8D {
                    self.short_bsr(bus)
                } else {
                    self.execute_load_store_alu(opcode, bus)
                }
            }
            0x10 | 0x11 => 1,
        }
    }

    pub(crate) fn address_for_mode(
        &mut self,
        mode: AddressMode,
        bus: &mut impl common::Bus,
    ) -> EffectiveAddress {
        match mode {
            AddressMode::Immediate => EffectiveAddress {
                address: self.pc,
                extra_cycles: 0,
            },
            AddressMode::Direct => EffectiveAddress {
                address: self.direct_address(bus),
                extra_cycles: 0,
            },
            AddressMode::Indexed => self.indexed_address(bus),
            AddressMode::Extended => EffectiveAddress {
                address: self.fetch_u16(bus),
                extra_cycles: 0,
            },
        }
    }

    pub(crate) fn read_mode_u8(
        &mut self,
        mode: AddressMode,
        bus: &mut impl common::Bus,
    ) -> (u8, i32) {
        match mode {
            AddressMode::Immediate => (self.fetch_u8(bus), 0),
            AddressMode::Direct | AddressMode::Indexed | AddressMode::Extended => {
                let ea = self.address_for_mode(mode, bus);
                (self.read_byte(bus, ea.address), ea.extra_cycles)
            }
        }
    }

    pub(crate) fn read_mode_u16(
        &mut self,
        mode: AddressMode,
        bus: &mut impl common::Bus,
    ) -> (u16, i32) {
        match mode {
            AddressMode::Immediate => (self.fetch_u16(bus), 0),
            AddressMode::Direct | AddressMode::Indexed | AddressMode::Extended => {
                let ea = self.address_for_mode(mode, bus);
                (self.read_word(bus, ea.address), ea.extra_cycles)
            }
        }
    }

    pub(crate) fn write_mode_u8(
        &mut self,
        mode: AddressMode,
        value: u8,
        bus: &mut impl common::Bus,
    ) -> i32 {
        let ea = self.address_for_mode(mode, bus);
        self.write_byte(bus, ea.address, value);
        ea.extra_cycles
    }

    pub(crate) fn write_mode_u16(
        &mut self,
        mode: AddressMode,
        value: u16,
        bus: &mut impl common::Bus,
    ) -> i32 {
        let ea = self.address_for_mode(mode, bus);
        self.write_word(bus, ea.address, value);
        ea.extra_cycles
    }

    pub(crate) fn condition_hi(&self) -> bool {
        !(self.flags.carry || self.flags.zero)
    }

    pub(crate) fn condition_ge(&self) -> bool {
        self.flags.negative == self.flags.overflow
    }

    pub(crate) fn condition_gt(&self) -> bool {
        self.condition_ge() && !self.flags.zero
    }

    pub(crate) fn short_branch(&mut self, condition: bool, bus: &mut impl common::Bus) -> i32 {
        let offset = self.fetch_u8(bus) as i8;
        if condition {
            self.pc = self.pc.wrapping_add_signed(i16::from(offset));
        }
        3
    }

    pub(crate) fn long_branch(
        &mut self,
        condition: bool,
        bus: &mut impl common::Bus,
        taken_cycles: i32,
    ) -> i32 {
        let offset = self.fetch_u16(bus);
        if condition {
            self.pc = self.pc.wrapping_add(offset);
            taken_cycles
        } else {
            taken_cycles - 1
        }
    }

    fn short_bsr(&mut self, bus: &mut impl common::Bus) -> i32 {
        let offset = self.fetch_u8(bus) as i8;
        let target = self.pc.wrapping_add_signed(i16::from(offset));
        self.push_s_word(bus, self.pc);
        self.pc = target;
        7
    }

    fn long_bsr(&mut self, bus: &mut impl common::Bus) -> i32 {
        let offset = self.fetch_u16(bus);
        let target = self.pc.wrapping_add(offset);
        self.push_s_word(bus, self.pc);
        self.pc = target;
        9
    }

    fn execute_memory_unary(
        &mut self,
        mode: AddressMode,
        operation: UnaryOp,
        base_cycles: i32,
        bus: &mut impl common::Bus,
    ) -> i32 {
        let ea = self.address_for_mode(mode, bus);
        let value = self.read_byte(bus, ea.address);
        match operation {
            UnaryOp::Tst => self.tst8(value),
            UnaryOp::Clr => {
                let result = self.clr8();
                self.write_byte(bus, ea.address, result);
            }
            UnaryOp::Xclr => {
                let result = self.xclr8();
                self.write_byte(bus, ea.address, result);
            }
            UnaryOp::Neg
            | UnaryOp::Xnc
            | UnaryOp::Com
            | UnaryOp::Lsr
            | UnaryOp::Ror
            | UnaryOp::Asr
            | UnaryOp::Asl
            | UnaryOp::Rol
            | UnaryOp::Dec
            | UnaryOp::Xdec
            | UnaryOp::Inc => {
                let result = self.apply_unary(operation, value);
                self.write_byte(bus, ea.address, result);
            }
        }
        base_cycles + ea.extra_cycles
    }

    fn execute_register_unary(&mut self, accumulator_a: bool, operation: UnaryOp) -> i32 {
        let value = if accumulator_a { self.a } else { self.b };
        match operation {
            UnaryOp::Tst => self.tst8(value),
            UnaryOp::Clr => {
                let result = self.clr8();
                if accumulator_a {
                    self.a = result;
                } else {
                    self.b = result;
                }
            }
            UnaryOp::Xclr => {
                let result = self.xclr8();
                if accumulator_a {
                    self.a = result;
                } else {
                    self.b = result;
                }
            }
            UnaryOp::Neg
            | UnaryOp::Xnc
            | UnaryOp::Com
            | UnaryOp::Lsr
            | UnaryOp::Ror
            | UnaryOp::Asr
            | UnaryOp::Asl
            | UnaryOp::Rol
            | UnaryOp::Dec
            | UnaryOp::Xdec
            | UnaryOp::Inc => {
                let result = self.apply_unary(operation, value);
                if accumulator_a {
                    self.a = result;
                } else {
                    self.b = result;
                }
            }
        }
        2
    }

    fn begin_extended_clear(&mut self, bus: &mut impl common::Bus) -> i32 {
        let address = self.fetch_u16(bus);
        let _ = self.read_byte(bus, address);
        self.pending_extended_clear = Some(address);
        4
    }

    fn apply_unary(&mut self, operation: UnaryOp, value: u8) -> u8 {
        match operation {
            UnaryOp::Neg => self.neg8(value),
            UnaryOp::Xnc => {
                if self.flags.carry {
                    self.com8(value)
                } else {
                    self.neg8(value)
                }
            }
            UnaryOp::Com => self.com8(value),
            UnaryOp::Lsr => self.lsr8(value),
            UnaryOp::Ror => self.ror8(value),
            UnaryOp::Asr => self.asr8(value),
            UnaryOp::Asl => self.asl8(value),
            UnaryOp::Rol => self.rol8(value),
            UnaryOp::Dec => self.dec8(value),
            UnaryOp::Xdec => self.xdec8(value),
            UnaryOp::Inc => self.inc8(value),
            UnaryOp::Tst => {
                self.tst8(value);
                value
            }
            UnaryOp::Clr => self.clr8(),
            UnaryOp::Xclr => self.xclr8(),
        }
    }

    fn push_or_pull(
        &mut self,
        use_hardware_stack: bool,
        push: bool,
        bus: &mut impl common::Bus,
    ) -> i32 {
        let mask = self.fetch_u8(bus);
        if push {
            // A real FM-77AV seems to observe the initial PSH stack bus cycle.
            let stack_address = if use_hardware_stack { self.s } else { self.u };
            self.read_byte(bus, stack_address);
            self.push_registers(bus, use_hardware_stack, mask);
        } else {
            self.pull_registers(bus, use_hardware_stack, mask);
            // sic! A real FM-77AV seems to observe the final PUL stack bus cycle (ref. 77AVEMU).
            // If not read, the game "Metal -X-" will not draw correctly.
            let stack_address = if use_hardware_stack { self.s } else { self.u };
            self.read_byte(bus, stack_address);
        }
        5 + push_pull_bytes(mask)
    }

    fn lea(&mut self, mode: AddressMode, register: u8, bus: &mut impl common::Bus) -> i32 {
        let ea = self.address_for_mode(mode, bus);
        match register {
            0 => self.x = self.set_z16(ea.address),
            1 => self.y = self.set_z16(ea.address),
            2 => {
                self.s = ea.address;
                self.mark_s_loaded();
            }
            3 => self.u = ea.address,
            _ => {}
        }
        4 + ea.extra_cycles
    }

    fn execute_load_store_alu(&mut self, opcode: u8, bus: &mut impl common::Bus) -> i32 {
        let mode = match opcode & 0x30 {
            0x00 => AddressMode::Immediate,
            0x10 => AddressMode::Direct,
            0x20 => AddressMode::Indexed,
            0x30 => AddressMode::Extended,
            _ => AddressMode::Immediate,
        };
        let low = opcode & 0x0F;
        let accumulator_b_group = opcode >= 0xC0;

        match low {
            0x00 => self.alu8(mode, accumulator_b_group, bus, |cpu, left, right| {
                cpu.sub8(left, right)
            }),
            0x01 => self.cmp8_mode(mode, accumulator_b_group, bus),
            0x02 => self.alu8(mode, accumulator_b_group, bus, |cpu, left, right| {
                cpu.sbc8(left, right)
            }),
            0x03 => {
                if accumulator_b_group {
                    self.addd_mode(mode, bus)
                } else {
                    self.subd_mode(mode, bus)
                }
            }
            0x04 => self.alu8(mode, accumulator_b_group, bus, |cpu, left, right| {
                cpu.and8(left, right)
            }),
            0x05 => self.bit8_mode(mode, accumulator_b_group, bus),
            0x06 => self.ld8_mode(mode, accumulator_b_group, bus),
            0x07 => self.st8_mode(mode, accumulator_b_group, bus),
            0x08 => self.alu8(mode, accumulator_b_group, bus, |cpu, left, right| {
                cpu.eor8(left, right)
            }),
            0x09 => self.alu8(mode, accumulator_b_group, bus, |cpu, left, right| {
                cpu.adc8(left, right)
            }),
            0x0A => self.alu8(mode, accumulator_b_group, bus, |cpu, left, right| {
                cpu.or8(left, right)
            }),
            0x0B => self.alu8(mode, accumulator_b_group, bus, |cpu, left, right| {
                cpu.add8(left, right)
            }),
            0x0C => {
                if accumulator_b_group {
                    self.ldd_mode(mode, bus)
                } else {
                    self.cmpx_mode(mode, bus)
                }
            }
            0x0D => {
                if accumulator_b_group {
                    self.std_mode(mode, bus)
                } else {
                    self.jsr_mode(mode, bus)
                }
            }
            0x0E => {
                if accumulator_b_group {
                    self.ldu_mode(mode, bus)
                } else {
                    self.ldx_mode(mode, bus)
                }
            }
            0x0F => {
                if accumulator_b_group {
                    self.stu_mode(mode, bus)
                } else {
                    self.stx_mode(mode, bus)
                }
            }
            _ => 1,
        }
    }

    fn alu8(
        &mut self,
        mode: AddressMode,
        accumulator_b: bool,
        bus: &mut impl common::Bus,
        operation: fn(&mut M6809, u8, u8) -> u8,
    ) -> i32 {
        let (operand, extra_cycles) = self.read_mode_u8(mode, bus);
        let value = if accumulator_b { self.b } else { self.a };
        let result = operation(self, value, operand);
        if accumulator_b {
            self.b = result;
        } else {
            self.a = result;
        }
        mode_cycles(mode, 2, 4, 4, 5) + extra_cycles
    }

    fn cmp8_mode(
        &mut self,
        mode: AddressMode,
        accumulator_b: bool,
        bus: &mut impl common::Bus,
    ) -> i32 {
        let (operand, extra_cycles) = self.read_mode_u8(mode, bus);
        let value = if accumulator_b { self.b } else { self.a };
        self.cmp8(value, operand);
        mode_cycles(mode, 2, 4, 4, 5) + extra_cycles
    }

    fn bit8_mode(
        &mut self,
        mode: AddressMode,
        accumulator_b: bool,
        bus: &mut impl common::Bus,
    ) -> i32 {
        let (operand, extra_cycles) = self.read_mode_u8(mode, bus);
        let value = if accumulator_b { self.b } else { self.a };
        self.bit8(value, operand);
        mode_cycles(mode, 2, 4, 4, 5) + extra_cycles
    }

    fn ld8_mode(
        &mut self,
        mode: AddressMode,
        accumulator_b: bool,
        bus: &mut impl common::Bus,
    ) -> i32 {
        let (operand, extra_cycles) = self.read_mode_u8(mode, bus);
        let value = self.set_nzv8(operand);
        if accumulator_b {
            self.b = value;
        } else {
            self.a = value;
        }
        mode_cycles(mode, 2, 4, 4, 5) + extra_cycles
    }

    fn st8_mode(
        &mut self,
        mode: AddressMode,
        accumulator_b: bool,
        bus: &mut impl common::Bus,
    ) -> i32 {
        let value = if accumulator_b { self.b } else { self.a };
        self.set_nzv8(value);
        if mode == AddressMode::Immediate {
            let _ = self.fetch_u8(bus);
            2
        } else {
            let extra_cycles = self.write_mode_u8(mode, value, bus);
            mode_cycles(mode, 2, 4, 4, 5) + extra_cycles
        }
    }

    fn subd_mode(&mut self, mode: AddressMode, bus: &mut impl common::Bus) -> i32 {
        let (operand, extra_cycles) = self.read_mode_u16(mode, bus);
        let result = self.sub16(self.d(), operand);
        self.set_d(result);
        mode_cycles(mode, 4, 6, 6, 7) + extra_cycles
    }

    fn addd_mode(&mut self, mode: AddressMode, bus: &mut impl common::Bus) -> i32 {
        let (operand, extra_cycles) = self.read_mode_u16(mode, bus);
        let result = self.add16(self.d(), operand);
        self.set_d(result);
        mode_cycles(mode, 4, 6, 6, 7) + extra_cycles
    }

    pub(crate) fn cmpd_mode(&mut self, mode: AddressMode, bus: &mut impl common::Bus) -> i32 {
        let (operand, extra_cycles) = self.read_mode_u16(mode, bus);
        self.cmp16(self.d(), operand);
        mode_cycles(mode, 5, 7, 7, 8) + extra_cycles
    }

    fn cmpx_mode(&mut self, mode: AddressMode, bus: &mut impl common::Bus) -> i32 {
        let (operand, extra_cycles) = self.read_mode_u16(mode, bus);
        self.cmp16(self.x, operand);
        mode_cycles(mode, 4, 6, 6, 7) + extra_cycles
    }

    fn ldd_mode(&mut self, mode: AddressMode, bus: &mut impl common::Bus) -> i32 {
        let (operand, extra_cycles) = self.read_mode_u16(mode, bus);
        let value = self.set_nzv16(operand);
        self.set_d(value);
        mode_cycles(mode, 3, 5, 5, 6) + extra_cycles
    }

    fn std_mode(&mut self, mode: AddressMode, bus: &mut impl common::Bus) -> i32 {
        let value = self.d();
        self.set_nzv16(value);
        if mode == AddressMode::Immediate {
            self.xst16_immediate(value, bus);
            3
        } else {
            let extra_cycles = self.write_mode_u16(mode, value, bus);
            mode_cycles(mode, 3, 5, 5, 6) + extra_cycles
        }
    }

    fn ldx_mode(&mut self, mode: AddressMode, bus: &mut impl common::Bus) -> i32 {
        let (operand, extra_cycles) = self.read_mode_u16(mode, bus);
        self.x = self.set_nzv16(operand);
        mode_cycles(mode, 3, 5, 5, 6) + extra_cycles
    }

    fn stx_mode(&mut self, mode: AddressMode, bus: &mut impl common::Bus) -> i32 {
        if mode == AddressMode::Immediate {
            let value = self.x;
            self.set_nzv16(value);
            self.xst16_immediate(value, bus);
            3
        } else {
            let ea = self.address_for_mode(mode, bus);
            let value = self.x;
            self.set_nzv16(value);
            self.write_word(bus, ea.address, value);
            let extra_cycles = ea.extra_cycles;
            mode_cycles(mode, 3, 5, 5, 6) + extra_cycles
        }
    }

    fn ldu_mode(&mut self, mode: AddressMode, bus: &mut impl common::Bus) -> i32 {
        let (operand, extra_cycles) = self.read_mode_u16(mode, bus);
        self.u = self.set_nzv16(operand);
        mode_cycles(mode, 3, 5, 5, 6) + extra_cycles
    }

    fn stu_mode(&mut self, mode: AddressMode, bus: &mut impl common::Bus) -> i32 {
        if mode == AddressMode::Immediate {
            let value = self.u;
            self.set_nzv16(value);
            self.xst16_immediate(value, bus);
            3
        } else {
            let ea = self.address_for_mode(mode, bus);
            let value = self.u;
            self.set_nzv16(value);
            self.write_word(bus, ea.address, value);
            let extra_cycles = ea.extra_cycles;
            mode_cycles(mode, 3, 5, 5, 6) + extra_cycles
        }
    }

    fn jsr_mode(&mut self, mode: AddressMode, bus: &mut impl common::Bus) -> i32 {
        let ea = self.address_for_mode(mode, bus);
        self.push_s_word(bus, self.pc);
        self.pc = ea.address;
        mode_cycles(mode, 7, 7, 7, 8) + ea.extra_cycles
    }

    pub(crate) fn cmp_register_mode(
        &mut self,
        mode: AddressMode,
        register_value: u16,
        bus: &mut impl common::Bus,
    ) -> i32 {
        let (operand, extra_cycles) = self.read_mode_u16(mode, bus);
        self.cmp16(register_value, operand);
        mode_cycles(mode, 5, 7, 7, 8) + extra_cycles
    }

    pub(crate) fn ld_register_mode(
        &mut self,
        mode: AddressMode,
        bus: &mut impl common::Bus,
    ) -> (u16, i32) {
        let (operand, extra_cycles) = self.read_mode_u16(mode, bus);
        (
            self.set_nzv16(operand),
            mode_cycles(mode, 4, 6, 6, 7) + extra_cycles,
        )
    }

    pub(crate) fn st_register_mode(
        &mut self,
        mode: AddressMode,
        register: u8,
        bus: &mut impl common::Bus,
    ) -> i32 {
        if mode == AddressMode::Immediate {
            let value = self.register16_value(register);
            self.set_nzv16(value);
            self.xst16_immediate(value, bus);
            4
        } else {
            let ea = self.address_for_mode(mode, bus);
            let value = self.register16_value(register);
            self.set_nzv16(value);
            self.write_word(bus, ea.address, value);
            let extra_cycles = ea.extra_cycles;
            mode_cycles(mode, 4, 6, 6, 7) + extra_cycles
        }
    }

    pub(crate) fn xadd16_mode(
        &mut self,
        mode: AddressMode,
        register_value: u16,
        bus: &mut impl common::Bus,
    ) -> i32 {
        let (operand, extra_cycles) = self.read_mode_u16(mode, bus);
        let _ = self.add16(register_value, operand);
        mode_cycles(mode, 5, 7, 7, 8) + extra_cycles
    }

    fn xst16_immediate(&mut self, value: u16, bus: &mut impl common::Bus) {
        let _ = self.fetch_u8(bus);
        self.write_byte(bus, self.pc, value as u8);
        self.pc = self.pc.wrapping_add(1);
    }

    fn x18(&mut self, bus: &mut impl common::Bus) -> i32 {
        let operand = self.read_byte(bus, self.pc);
        let condition_code = self.flags.compress();
        let value = (condition_code & operand) << 1;
        self.flags
            .expand(value | ((condition_code & M6809Flags::ZERO) >> 1));
        2
    }

    fn exg(&mut self, bus: &mut impl common::Bus) -> i32 {
        let parameter = self.fetch_u8(bus);
        let (left, right) = if parameter & 0x80 != 0 {
            (
                self.read_tfr_exg_816_register(parameter >> 4),
                self.read_tfr_exg_816_register(parameter),
            )
        } else {
            (
                self.read_exg_168_register(parameter >> 4),
                self.read_exg_168_register(parameter),
            )
        };
        self.write_exgtfr_register(parameter, left, false);
        self.write_exgtfr_register(parameter >> 4, right, false);
        8
    }

    fn tfr(&mut self, bus: &mut impl common::Bus) -> i32 {
        let parameter = self.fetch_u8(bus);
        let value = self.read_tfr_exg_816_register(parameter >> 4);
        self.write_exgtfr_register(parameter, value, true);
        6
    }

    fn read_tfr_exg_816_register(&self, register: u8) -> u16 {
        match register & 0x0F {
            0x00 => self.d(),
            0x01 => self.x,
            0x02 => self.y,
            0x03 => self.u,
            0x04 => self.s,
            0x05 => self.pc,
            0x08 => 0xFF00 | u16::from(self.a),
            0x09 => 0xFF00 | u16::from(self.b),
            0x0A => 0xFF00 | u16::from(self.flags.compress()),
            0x0B => 0xFF00 | u16::from(self.dp),
            0x06 | 0x07 | 0x0C | 0x0D | 0x0E | 0x0F => 0xFFFF,
            _ => 0xFFFF,
        }
    }

    fn read_exg_168_register(&self, register: u8) -> u16 {
        match register & 0x0F {
            0x00 => self.d(),
            0x01 => self.x,
            0x02 => self.y,
            0x03 => self.u,
            0x04 => self.s,
            0x05 => self.pc,
            0x08 => 0xFF00 | u16::from(self.a),
            0x09 => 0xFF00 | u16::from(self.b),
            0x0A => 0xFF00 | u16::from(self.flags.compress()),
            0x0B => 0xFF00 | u16::from(self.dp),
            0x06 | 0x07 | 0x0C | 0x0D | 0x0E | 0x0F => 0xFFFF,
            _ => 0xFFFF,
        }
    }

    fn write_exgtfr_register(&mut self, register: u8, value: u16, mark_s_loaded: bool) {
        match register & 0x0F {
            0x00 => self.set_d(value),
            0x01 => self.x = value,
            0x02 => self.y = value,
            0x03 => self.u = value,
            0x04 => {
                self.s = value;
                if mark_s_loaded {
                    self.mark_s_loaded();
                }
            }
            0x05 => self.pc = value,
            0x08 => self.a = value as u8,
            0x09 => self.b = value as u8,
            0x0A => self.flags.expand(value as u8),
            0x0B => self.dp = value as u8,
            0x06 | 0x07 | 0x0C | 0x0D | 0x0E | 0x0F => {}
            _ => {}
        }
    }

    fn register16_value(&self, register: u8) -> u16 {
        match register {
            0 => self.x,
            1 => self.y,
            2 => self.u,
            3 => self.s,
            _ => 0xFFFF,
        }
    }
}

pub(crate) fn mode_cycles(
    mode: AddressMode,
    immediate: i32,
    direct: i32,
    indexed: i32,
    extended: i32,
) -> i32 {
    match mode {
        AddressMode::Immediate => immediate,
        AddressMode::Direct => direct,
        AddressMode::Indexed => indexed,
        AddressMode::Extended => extended,
    }
}

fn push_pull_bytes(mask: u8) -> i32 {
    i32::from(mask & 0x01 != 0)
        + i32::from(mask & 0x02 != 0)
        + i32::from(mask & 0x04 != 0)
        + i32::from(mask & 0x08 != 0)
        + if mask & 0x10 != 0 { 2 } else { 0 }
        + if mask & 0x20 != 0 { 2 } else { 0 }
        + if mask & 0x40 != 0 { 2 } else { 0 }
        + if mask & 0x80 != 0 { 2 } else { 0 }
}
