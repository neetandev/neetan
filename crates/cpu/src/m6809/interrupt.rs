use super::{
    M6809, PENDING_FIRQ, VECTOR_FIRQ, VECTOR_IRQ, VECTOR_NMI, VECTOR_SWI, VECTOR_SWI2, VECTOR_SWI3,
};

const PARTIAL_STATE_MASK: u8 = 0x81;
const ENTIRE_STATE_MASK: u8 = 0xFF;

impl M6809 {
    pub(crate) fn check_interrupts(&mut self, bus: &mut impl common::Bus) -> Option<i32> {
        if self.pending_irq & crate::PENDING_NMI != 0 {
            self.service_nmi(bus);
            Some(19)
        } else if self.pending_irq & PENDING_FIRQ != 0 && !self.flags.firq_mask {
            self.service_firq(bus);
            Some(10)
        } else if self.pending_irq & crate::PENDING_IRQ != 0 && !self.flags.irq_mask {
            self.service_irq(bus);
            Some(19)
        } else {
            None
        }
    }

    fn service_nmi(&mut self, bus: &mut impl common::Bus) {
        self.halted = false;
        self.flags.entire = true;
        self.push_registers(bus, true, ENTIRE_STATE_MASK);
        self.flags.irq_mask = true;
        self.flags.firq_mask = true;
        self.pc = self.read_word(bus, VECTOR_NMI);
        self.pending_irq &= !crate::PENDING_NMI;
        bus.acknowledge_nmi();
    }

    fn service_firq(&mut self, bus: &mut impl common::Bus) {
        self.halted = false;
        self.flags.entire = false;
        self.push_registers(bus, true, PARTIAL_STATE_MASK);
        self.flags.irq_mask = true;
        self.flags.firq_mask = true;
        self.pc = self.read_word(bus, VECTOR_FIRQ);
        self.pending_irq &= !PENDING_FIRQ;
    }

    fn service_irq(&mut self, bus: &mut impl common::Bus) {
        self.halted = false;
        self.flags.entire = true;
        self.push_registers(bus, true, ENTIRE_STATE_MASK);
        self.flags.irq_mask = true;
        self.pc = self.read_word(bus, VECTOR_IRQ);
        self.pending_irq &= !crate::PENDING_IRQ;
        let _ = bus.acknowledge_irq();
    }

    pub(crate) fn swi(&mut self, bus: &mut impl common::Bus) {
        self.software_interrupt(bus, VECTOR_SWI, true, true, true);
    }

    pub(crate) fn swi2(&mut self, bus: &mut impl common::Bus) {
        self.software_interrupt(bus, VECTOR_SWI2, false, false, true);
    }

    pub(crate) fn swi3(&mut self, bus: &mut impl common::Bus) {
        self.software_interrupt(bus, VECTOR_SWI3, false, false, true);
    }

    pub(crate) fn x_swi2(&mut self, bus: &mut impl common::Bus) {
        self.software_interrupt(bus, VECTOR_SWI2, false, false, false);
    }

    pub(crate) fn x_firq(&mut self, bus: &mut impl common::Bus) {
        self.software_interrupt(bus, VECTOR_FIRQ, false, false, false);
    }

    pub(crate) fn x_reset(&mut self, bus: &mut impl common::Bus) {
        self.software_interrupt(bus, super::VECTOR_RESET, false, false, false);
    }

    fn software_interrupt(
        &mut self,
        bus: &mut impl common::Bus,
        vector: u16,
        set_irq_mask: bool,
        set_firq_mask: bool,
        set_entire: bool,
    ) {
        if set_entire {
            self.flags.entire = true;
        }
        self.push_registers(bus, true, ENTIRE_STATE_MASK);
        if set_irq_mask {
            self.flags.irq_mask = true;
        }
        if set_firq_mask {
            self.flags.firq_mask = true;
        }
        self.pc = self.read_word(bus, vector);
    }

    pub(crate) fn rti(&mut self, bus: &mut impl common::Bus) -> i32 {
        let condition_code = self.pull_s_byte(bus);
        self.flags.expand(condition_code);
        if self.flags.entire {
            self.a = self.pull_s_byte(bus);
            self.b = self.pull_s_byte(bus);
            self.dp = self.pull_s_byte(bus);
            self.x = self.pull_s_word(bus);
            self.y = self.pull_s_word(bus);
            self.u = self.pull_s_word(bus);
            self.pc = self.pull_s_word(bus);
            15
        } else {
            self.pc = self.pull_s_word(bus);
            6
        }
    }

    pub(crate) fn push_registers(
        &mut self,
        bus: &mut impl common::Bus,
        use_hardware_stack: bool,
        mask: u8,
    ) {
        let mut stack = if use_hardware_stack { self.s } else { self.u };
        if mask & 0x80 != 0 {
            self.push_word(bus, &mut stack, self.pc);
        }
        if mask & 0x40 != 0 {
            let value = if use_hardware_stack { self.u } else { self.s };
            self.push_word(bus, &mut stack, value);
        }
        if mask & 0x20 != 0 {
            self.push_word(bus, &mut stack, self.y);
        }
        if mask & 0x10 != 0 {
            self.push_word(bus, &mut stack, self.x);
        }
        if mask & 0x08 != 0 {
            self.push_byte(bus, &mut stack, self.dp);
        }
        if mask & 0x04 != 0 {
            self.push_byte(bus, &mut stack, self.b);
        }
        if mask & 0x02 != 0 {
            self.push_byte(bus, &mut stack, self.a);
        }
        if mask & 0x01 != 0 {
            self.push_byte(bus, &mut stack, self.flags.compress());
        }
        if use_hardware_stack {
            self.s = stack;
        } else {
            self.u = stack;
        }
    }

    pub(crate) fn pull_registers(
        &mut self,
        bus: &mut impl common::Bus,
        use_hardware_stack: bool,
        mask: u8,
    ) {
        let mut stack = if use_hardware_stack { self.s } else { self.u };
        if mask & 0x01 != 0 {
            let value = self.pull_byte(bus, &mut stack);
            self.flags.expand(value);
        }
        if mask & 0x02 != 0 {
            self.a = self.pull_byte(bus, &mut stack);
        }
        if mask & 0x04 != 0 {
            self.b = self.pull_byte(bus, &mut stack);
        }
        if mask & 0x08 != 0 {
            self.dp = self.pull_byte(bus, &mut stack);
        }
        if mask & 0x10 != 0 {
            self.x = self.pull_word(bus, &mut stack);
        }
        if mask & 0x20 != 0 {
            self.y = self.pull_word(bus, &mut stack);
        }
        if mask & 0x40 != 0 {
            let value = self.pull_word(bus, &mut stack);
            if use_hardware_stack {
                self.u = value;
            } else {
                self.s = value;
                self.mark_s_loaded();
            }
        }
        if mask & 0x80 != 0 {
            self.pc = self.pull_word(bus, &mut stack);
        }
        if use_hardware_stack {
            self.s = stack;
        } else {
            self.u = stack;
        }
    }

    pub(crate) fn cwai(&mut self, bus: &mut impl common::Bus) {
        let mask = self.fetch_u8(bus);
        let condition_code = self.flags.compress();
        self.flags.expand(condition_code & mask);
        self.flags.entire = true;
        self.push_registers(bus, true, ENTIRE_STATE_MASK);
        self.halted = true;
    }

    pub(crate) fn sync(&mut self) {
        self.halted = true;
    }
}
