use super::{IndexMode, Z80};

impl Z80 {
    pub(crate) fn check_interrupts(&mut self, bus: &mut impl common::Bus) {
        if self.pending_irq & crate::PENDING_NMI != 0 {
            self.service_nmi(bus);
        } else if self.pending_irq & crate::PENDING_IRQ != 0 && self.iff1 && self.ei == 0 {
            self.service_irq(bus);
        }
    }

    fn service_nmi(&mut self, bus: &mut impl common::Bus) {
        self.halted = false;
        self.iff1 = false;
        self.increment_r();
        self.clk(5);
        let return_pc = self.pc;
        self.push(bus, return_pc);
        self.pc = 0x0066;
        self.wz = self.pc;
        self.pending_irq &= !crate::PENDING_NMI;
        bus.acknowledge_nmi();
    }

    fn service_irq(&mut self, bus: &mut impl common::Bus) {
        self.halted = false;
        self.iff1 = false;
        self.iff2 = false;
        self.increment_r();
        let vector = bus.acknowledge_irq();
        self.clk(7);
        match self.im & 3 {
            2 => {
                let table = (u16::from(self.i) << 8) | u16::from(vector);
                let low = self.read_byte(bus, table);
                let high = self.read_byte(bus, table.wrapping_add(1));
                let return_pc = self.pc;
                self.push(bus, return_pc);
                self.pc = u16::from(low) | (u16::from(high) << 8);
            }
            1 => {
                let return_pc = self.pc;
                self.push(bus, return_pc);
                self.pc = 0x0038;
            }
            _ => self.execute_im0_opcode(vector, bus),
        }
        self.wz = self.pc;
        self.pending_irq &= !crate::PENDING_IRQ;
    }

    fn execute_im0_opcode(&mut self, opcode: u8, bus: &mut impl common::Bus) {
        self.execute_base(opcode, IndexMode::HL, false, bus);
    }
}
