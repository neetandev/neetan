use super::{ADDRESS_MASK, I286};
use crate::{PENDING_IRQ, PENDING_NMI, SegReg16};

const INTGATE: u8 = 6;
const TRAPGATE: u8 = 7;

enum DoubleFaultResult {
    Normal,
    DoubleFault,
    Shutdown,
}

impl I286 {
    pub(super) fn check_interrupts(&mut self, bus: &mut impl common::Bus) {
        if self.pending_irq & PENDING_NMI != 0 && self.inhibit_all == 0 {
            self.pending_irq &= !PENDING_NMI;
            bus.acknowledge_nmi();
            self.raise_interrupt(2, bus);
        } else if self.flags.if_flag
            && self.pending_irq & PENDING_IRQ != 0
            && self.no_interrupt == 0
            && self.inhibit_all == 0
        {
            self.pending_irq &= !PENDING_IRQ;
            let vector = bus.acknowledge_irq();
            self.raise_interrupt(vector, bus);
        }
    }

    fn interrupt_with_return_ip(
        &mut self,
        vector: u8,
        return_ip: u16,
        error_code: Option<u16>,
        is_software_int: bool,
        is_external: bool,
        bus: &mut impl common::Bus,
    ) {
        self.finish_state = super::timing::I286FinishState::FaultRestart;
        self.rep_active = false;
        if self.msw & 1 == 0 {
            let flags_val = self.flags.compress();
            self.push(bus, flags_val);
            self.flags.tf = false;
            self.flags.if_flag = false;

            let cs = self.sregs[SegReg16::CS as usize];
            self.push(bus, cs);
            self.push(bus, return_ip);

            let addr = (vector as u32) * 4;
            let dest_ip = bus.read_word(addr);
            let dest_cs = bus.read_word(addr + 2);
            if !self.load_segment(SegReg16::CS, dest_cs, bus) {
                return;
            }
            self.ip = dest_ip;
        } else {
            self.interrupt_protected(
                vector,
                return_ip,
                error_code,
                is_software_int,
                is_external,
                bus,
            );
        }
    }

    fn interrupt_protected(
        &mut self,
        vector: u8,
        return_ip: u16,
        error_code: Option<u16>,
        is_software_int: bool,
        is_external: bool,
        bus: &mut impl common::Bus,
    ) {
        let ext = is_external as u16;
        let gate_offset = (vector as u32) * 8;
        if gate_offset + 7 > self.idt_limit as u32 {
            self.raise_fault_with_code(13, gate_offset as u16 + 2 + ext, bus);
            return;
        }

        let gate_addr = self.idt_base.wrapping_add(gate_offset);
        let w0 = bus.read_byte(gate_addr & ADDRESS_MASK) as u16
            | ((bus.read_byte(gate_addr.wrapping_add(1) & ADDRESS_MASK) as u16) << 8);
        let w1 = bus.read_byte(gate_addr.wrapping_add(2) & ADDRESS_MASK) as u16
            | ((bus.read_byte(gate_addr.wrapping_add(3) & ADDRESS_MASK) as u16) << 8);
        let w2 = bus.read_byte(gate_addr.wrapping_add(4) & ADDRESS_MASK) as u16
            | ((bus.read_byte(gate_addr.wrapping_add(5) & ADDRESS_MASK) as u16) << 8);

        let gate_ip = w0;
        let gate_selector = w1;
        let rights_byte = (w2 >> 8) as u8;
        let gate_type = rights_byte & 0x1F;
        let gate_dpl = ((rights_byte >> 5) & 0x03) as u16;
        let gate_present = rights_byte & 0x80 != 0;

        let cpl = self.cpl();

        if is_software_int && gate_dpl < cpl {
            self.raise_fault_with_code(13, gate_offset as u16 + 2 + ext, bus);
            return;
        }

        if !gate_present {
            self.raise_fault_with_code(11, gate_offset as u16 + 2 + ext, bus);
            return;
        }

        match gate_type {
            INTGATE | TRAPGATE => {
                self.dispatch_int_trap_gate(
                    gate_ip,
                    gate_selector,
                    gate_type,
                    return_ip,
                    error_code,
                    ext,
                    bus,
                );
            }
            _ => {
                self.raise_fault_with_code(13, gate_offset as u16 + 2 + ext, bus);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_int_trap_gate(
        &mut self,
        gate_ip: u16,
        gate_selector: u16,
        gate_type: u8,
        return_ip: u16,
        error_code: Option<u16>,
        ext: u16,
        bus: &mut impl common::Bus,
    ) {
        let Some(descriptor) = self.decode_descriptor(gate_selector, bus) else {
            self.raise_fault_with_code(13, Self::segment_error_code(gate_selector) + ext, bus);
            return;
        };

        let rights = descriptor.rights;
        if !Self::descriptor_is_code(rights) || !Self::descriptor_is_segment(rights) {
            self.raise_fault_with_code(13, Self::segment_error_code(gate_selector) + ext, bus);
            return;
        }

        let target_dpl = Self::descriptor_dpl(rights);
        let cpl = self.cpl();

        if target_dpl > cpl {
            self.raise_fault_with_code(13, Self::segment_error_code(gate_selector) + ext, bus);
            return;
        }

        if !Self::descriptor_present(rights) {
            self.raise_fault_with_code(11, Self::segment_error_code(gate_selector) + ext, bus);
            return;
        }

        if gate_ip > descriptor.limit {
            self.raise_fault_with_code(13, ext, bus);
            return;
        }

        if Self::descriptor_is_conforming_code(rights) {
            // Conforming code: DPL <= CPL is sufficient, treat as same-privilege.
        } else if target_dpl < cpl {
            // Inter-privilege interrupt: switch stacks from TSS.
            let new_dpl = target_dpl;

            // Read inner SS:SP from TSS.
            let tss_sp_offset = 2 + new_dpl * 4;
            let tss_ss_offset = 4 + new_dpl * 4;

            if tss_sp_offset as u32 + 3 > self.tr_limit as u32 {
                self.raise_fault_with_code(10, Self::segment_error_code(self.tr), bus);
                return;
            }

            let new_sp = bus
                .read_byte(self.tr_base.wrapping_add(tss_sp_offset as u32) & ADDRESS_MASK)
                as u16
                | ((bus.read_byte(
                    self.tr_base
                        .wrapping_add(tss_sp_offset.wrapping_add(1) as u32)
                        & ADDRESS_MASK,
                ) as u16)
                    << 8);
            let new_ss = bus
                .read_byte(self.tr_base.wrapping_add(tss_ss_offset as u32) & ADDRESS_MASK)
                as u16
                | ((bus.read_byte(
                    self.tr_base
                        .wrapping_add(tss_ss_offset.wrapping_add(1) as u32)
                        & ADDRESS_MASK,
                ) as u16)
                    << 8);

            let old_ss = self.sregs[SegReg16::SS as usize];
            let old_sp = self.regs.word(crate::WordReg::SP);
            let old_flags = self.flags.compress();
            let old_cs = self.sregs[SegReg16::CS as usize];

            // Validate new SS inline using target_dpl (not old CPL). Faults use #TS (vector 10).
            let ss_error_code = Self::segment_error_code(new_ss) + ext;
            if new_ss & 0xFFFC == 0 {
                self.raise_fault_with_code(10, ss_error_code, bus);
                return;
            }
            let Some(ss_descriptor) = self.decode_descriptor(new_ss, bus) else {
                self.raise_fault_with_code(10, ss_error_code, bus);
                return;
            };
            let ss_rights = ss_descriptor.rights;
            let ss_dpl = Self::descriptor_dpl(ss_rights);
            let ss_rpl = new_ss & 0x0003;
            if !Self::descriptor_is_segment(ss_rights) || !Self::descriptor_is_writable(ss_rights) {
                self.raise_fault_with_code(10, ss_error_code, bus);
                return;
            }
            if ss_dpl != new_dpl || ss_rpl != new_dpl {
                self.raise_fault_with_code(10, ss_error_code, bus);
                return;
            }
            if !Self::descriptor_present(ss_rights) {
                self.raise_fault_with_code(10, ss_error_code, bus);
                return;
            }
            self.set_accessed_bit(new_ss, bus);
            self.set_loaded_segment_cache(SegReg16::SS, new_ss, ss_descriptor);
            self.regs.set_word(crate::WordReg::SP, new_sp);

            // Push old SS, old SP, FLAGS, CS, IP on new stack.
            self.push(bus, old_ss);
            self.push(bus, old_sp);
            self.push(bus, old_flags);
            self.push(bus, old_cs);
            self.push(bus, return_ip);
            if let Some(code) = error_code {
                self.push(bus, code);
            }

            self.set_accessed_bit(gate_selector, bus);
            let adjusted_selector = (gate_selector & !3) | new_dpl;
            self.set_loaded_segment_cache(SegReg16::CS, adjusted_selector, descriptor);
            self.ip = gate_ip;

            self.flags.tf = false;
            self.flags.nt = false;
            if gate_type == INTGATE {
                self.flags.if_flag = false;
            }
            return;
        }

        // Same-privilege interrupt.
        let flags_val = self.flags.compress();
        let cs = self.sregs[SegReg16::CS as usize];
        self.push(bus, flags_val);
        self.push(bus, cs);
        self.push(bus, return_ip);
        if let Some(code) = error_code {
            self.push(bus, code);
        }

        self.set_accessed_bit(gate_selector, bus);
        let adjusted_selector = (gate_selector & !3) | target_dpl;
        self.set_loaded_segment_cache(SegReg16::CS, adjusted_selector, descriptor);
        self.ip = gate_ip;

        self.flags.tf = false;
        self.flags.nt = false;
        if gate_type == INTGATE {
            self.flags.if_flag = false;
        }
    }

    pub(super) fn raise_interrupt(&mut self, vector: u8, bus: &mut impl common::Bus) {
        let return_ip = if self.rep_active {
            self.rep_restart_ip
        } else {
            self.ip
        };
        self.interrupt_with_return_ip(vector, return_ip, None, false, true, bus);
    }

    pub(super) fn raise_software_interrupt(&mut self, vector: u8, bus: &mut impl common::Bus) {
        let return_ip = if self.rep_active {
            self.rep_restart_ip
        } else {
            self.ip
        };
        self.interrupt_with_return_ip(vector, return_ip, None, true, false, bus);
    }

    pub(super) fn raise_fault(&mut self, vector: u8, bus: &mut impl common::Bus) {
        if self.shutdown {
            return;
        }
        self.finish_state = super::timing::I286FinishState::FaultRestart;
        self.state.timing.note_exception_entry();
        if self.is_protected_mode() {
            match self.check_double_fault(vector) {
                DoubleFaultResult::Shutdown => return,
                DoubleFaultResult::DoubleFault => {
                    self.interrupt_with_return_ip(8, self.prev_ip, Some(0), false, false, bus);
                    return;
                }
                DoubleFaultResult::Normal => {}
            }
        }
        self.interrupt_with_return_ip(vector, self.prev_ip, None, false, false, bus);
        self.trap_level = 0;
    }

    pub(super) fn raise_fault_with_code(
        &mut self,
        vector: u8,
        error_code: u16,
        bus: &mut impl common::Bus,
    ) {
        if self.shutdown {
            return;
        }
        self.finish_state = super::timing::I286FinishState::FaultRestart;
        self.state.timing.note_exception_entry();
        if self.is_protected_mode() {
            match self.check_double_fault(vector) {
                DoubleFaultResult::Shutdown => return,
                DoubleFaultResult::DoubleFault => {
                    self.interrupt_with_return_ip(8, self.prev_ip, Some(0), false, false, bus);
                    return;
                }
                DoubleFaultResult::Normal => {}
            }
        }
        self.interrupt_with_return_ip(vector, self.prev_ip, Some(error_code), false, false, bus);
        self.trap_level = 0;
    }

    fn is_contributory_exception(vector: u8) -> bool {
        matches!(vector, 0 | 10 | 11 | 12 | 13)
    }

    fn check_double_fault(&mut self, vector: u8) -> DoubleFaultResult {
        if Self::is_contributory_exception(vector) || vector == 8 {
            self.trap_level += 1;
            if self.trap_level >= 3 {
                self.shutdown = true;
                self.halted = true;
                return DoubleFaultResult::Shutdown;
            }
            if self.trap_level >= 2 {
                return DoubleFaultResult::DoubleFault;
            }
        }
        DoubleFaultResult::Normal
    }
}
