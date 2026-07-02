use super::{Fault, I386, Step};
use crate::{PENDING_IRQ, PENDING_NMI, SegReg32};

const INTGATE_286: u8 = 6;
const TRAPGATE_286: u8 = 7;
const INTGATE_386: u8 = 14;
const TRAPGATE_386: u8 = 15;
const TASKGATE: u8 = 5;

enum DoubleFaultResult {
    Normal,
    DoubleFault,
    Shutdown,
}

/// Exception classification per Intel 386 Programmer's Reference Manual,
/// Table 9-3.
///
///   0 = benign (INT 1/2/3/4/5/6/7/16)
///   1 = contributory (#DE=0, #CSO=9, #TS=10, #NP=11, #SS=12, #GP=13)
///   2 = page fault (#PF=14)
///   3 = double fault (#DF=8)
const fn exception_class(vector: u8) -> u8 {
    match vector {
        0 | 9 | 10 | 11 | 12 | 13 => 1,
        14 => 2,
        8 => 3,
        _ => 0,
    }
}

/// Escalation table: `ESCALATION[prev_class][current_class]` is `true` when
/// the combination should escalate to a double fault (or shutdown if prev was DF).
///
/// Matches the Intel 386 manual Table 9-4.
const ESCALATION: [[bool; 4]; 4] = [
    //             benign  contrib  PF     DF
    /* benign  */ [false, false, false, true],
    /* contrib */ [false, true, false, true],
    /* PF      */ [false, true, true, true],
    /* DF      */ [true, true, true, true],
];

impl<const CPU_MODEL: u8, const ADDRESS_WIDTH: u8> I386<CPU_MODEL, ADDRESS_WIDTH> {
    pub(super) fn check_interrupts(&mut self, bus: &mut impl common::Bus) {
        if self.pending_irq & PENDING_NMI != 0 && self.inhibit_all == 0 {
            self.pending_irq &= !PENDING_NMI;
            bus.acknowledge_nmi();
            let _ = self.raise_interrupt(2, bus);
        } else if self.flags.if_flag
            && self.pending_irq & PENDING_IRQ != 0
            && self.no_interrupt == 0
            && self.inhibit_all == 0
        {
            self.pending_irq &= !PENDING_IRQ;
            let vector = bus.acknowledge_irq();
            let _ = self.raise_interrupt(vector, bus);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn interrupt_with_return_eip(
        &mut self,
        vector: u8,
        return_eip: u32,
        error_code: Option<u16>,
        is_software_int: bool,
        is_external: bool,
        is_fault: bool,
        bus: &mut impl common::Bus,
    ) -> Step {
        self.rep_active = false;
        if !self.is_protected_mode() {
            // 80486 PRM 22.5 item 21 / Table 22-2: a vector beyond the IDTR
            // limit raises #DF; if #DF itself would also be out of range,
            // the processor shuts down.
            let addr = (vector as u32) * 4;
            let idt_limit = self.idt_limit as u32;
            if addr + 3 > idt_limit {
                let df_addr = 8u32 * 4;
                if df_addr + 3 > idt_limit {
                    self.shutdown = true;
                    return Err(Fault);
                }
                return self.interrupt_with_return_eip(
                    8,
                    return_eip,
                    Some(0),
                    false,
                    false,
                    true,
                    bus,
                );
            }

            let flags_val = self.flags.compress();
            self.push(bus, flags_val)?;
            self.flags.tf = false;
            self.flags.if_flag = false;

            let cs = self.sregs[SegReg32::CS as usize];
            self.push(bus, cs)?;
            self.push(bus, return_eip as u16)?;

            let dest_ip = bus.read_word(addr);
            let dest_cs = bus.read_word(addr + 2);
            self.load_segment(SegReg32::CS, dest_cs, bus)?;
            self.ip = dest_ip;
            self.ip_upper = 0;
            Ok(())
        } else {
            self.supervisor_override = true;
            let result = self.interrupt_protected(
                vector,
                return_eip,
                error_code,
                is_software_int,
                is_external,
                is_fault,
                bus,
            );
            self.supervisor_override = false;
            result
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn interrupt_protected(
        &mut self,
        vector: u8,
        return_eip: u32,
        error_code: Option<u16>,
        is_software_int: bool,
        is_external: bool,
        is_fault: bool,
        bus: &mut impl common::Bus,
    ) -> Step {
        let ext = is_external as u16;
        let gate_offset = (vector as u32) * 8;
        if gate_offset + 7 > self.idt_limit as u32 {
            return self.raise_fault_with_code(13, gate_offset as u16 + 2 + ext, bus);
        }

        let gate_addr = self.idt_base.wrapping_add(gate_offset);
        let w0 = self.read_word_linear(bus, gate_addr)?;
        let w1 = self.read_word_linear(bus, gate_addr.wrapping_add(2))?;
        let w2 = self.read_word_linear(bus, gate_addr.wrapping_add(4))?;
        let w3 = self.read_word_linear(bus, gate_addr.wrapping_add(6))?;

        let gate_selector = w1;
        let rights_byte = (w2 >> 8) as u8;
        let gate_type = rights_byte & 0x1F;
        let gate_dpl = ((rights_byte >> 5) & 0x03) as u16;
        let gate_present = rights_byte & 0x80 != 0;

        let cpl = self.cpl();

        if is_software_int && gate_dpl < cpl {
            return self.raise_fault_with_code(13, gate_offset as u16 + 2 + ext, bus);
        }

        if !gate_present {
            return self.raise_fault_with_code(11, gate_offset as u16 + 2 + ext, bus);
        }

        let (gate_ip, is_386_gate) = match gate_type {
            INTGATE_386 | TRAPGATE_386 => ((w3 as u32) << 16 | w0 as u32, true),
            INTGATE_286 | TRAPGATE_286 => (w0 as u32, false),
            TASKGATE => {
                let task_selector = gate_selector;
                self.switch_task(task_selector, super::TaskType::Call, bus)?;
                let flags_val = self.flags.compress();
                let new_cpl = self.cpl();
                self.flags.load_flags(flags_val, new_cpl, true);
                if let Some(code) = error_code {
                    let is_386_tss = (self.tr_rights & 0x0F) >= 0x09;
                    if is_386_tss {
                        self.push_dword(bus, code as u32)?;
                    } else {
                        self.push(bus, code)?;
                    }
                }
                return Ok(());
            }
            _ => {
                return self.raise_fault_with_code(13, gate_offset as u16 + 2 + ext, bus);
            }
        };

        self.dispatch_int_trap_gate(
            gate_ip,
            gate_selector,
            gate_type,
            is_386_gate,
            return_eip,
            error_code,
            ext,
            is_fault,
            bus,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_int_trap_gate(
        &mut self,
        gate_ip: u32,
        gate_selector: u16,
        gate_type: u8,
        is_386_gate: bool,
        return_eip: u32,
        error_code: Option<u16>,
        ext: u16,
        is_fault: bool,
        bus: &mut impl common::Bus,
    ) -> Step {
        let Some(descriptor) = self.decode_descriptor(gate_selector, bus)? else {
            return self.raise_fault_with_code(
                13,
                Self::segment_error_code(gate_selector) + ext,
                bus,
            );
        };

        let rights = descriptor.rights;
        if !Self::descriptor_is_code(rights) || !Self::descriptor_is_segment(rights) {
            return self.raise_fault_with_code(
                13,
                Self::segment_error_code(gate_selector) + ext,
                bus,
            );
        }

        let target_dpl = Self::descriptor_dpl(rights);
        let from_vm86 = self.is_virtual_mode();
        let cpl = self.cpl();

        if target_dpl > cpl {
            return self.raise_fault_with_code(
                13,
                Self::segment_error_code(gate_selector) + ext,
                bus,
            );
        }

        if from_vm86 && (Self::descriptor_is_conforming_code(rights) || target_dpl != 0) {
            return self.raise_fault_with_code(
                13,
                Self::segment_error_code(gate_selector) + ext,
                bus,
            );
        }

        if !Self::descriptor_present(rights) {
            return self.raise_fault_with_code(
                11,
                Self::segment_error_code(gate_selector) + ext,
                bus,
            );
        }

        if gate_ip > descriptor.limit {
            return self.raise_fault_with_code(13, ext, bus);
        }

        if Self::descriptor_is_conforming_code(rights) {
            // Conforming code: DPL <= CPL is sufficient, treat as same-privilege.
        } else if target_dpl < cpl {
            // Inter-privilege interrupt: switch stacks from TSS.
            let new_dpl = target_dpl;
            let tss_type = self.tr_rights & 0x0F;
            let is_386_tss = tss_type == 9 || tss_type == 11;

            let (new_esp, new_ss) = if is_386_tss {
                let tss_esp_offset = 4 + new_dpl as u32 * 8;
                let tss_ss_offset = 8 + new_dpl as u32 * 8;
                if tss_ss_offset + 1 > self.tr_limit {
                    return self.raise_fault_with_code(10, Self::segment_error_code(self.tr), bus);
                }
                let esp = self.read_dword_linear(bus, self.tr_base.wrapping_add(tss_esp_offset))?;
                let ss = self.read_word_linear(bus, self.tr_base.wrapping_add(tss_ss_offset))?;
                (esp, ss)
            } else {
                let tss_sp_offset = 2 + new_dpl as u32 * 4;
                let tss_ss_offset = 4 + new_dpl as u32 * 4;
                if tss_ss_offset + 1 > self.tr_limit {
                    return self.raise_fault_with_code(10, Self::segment_error_code(self.tr), bus);
                }
                let sp = self.read_word_linear(bus, self.tr_base.wrapping_add(tss_sp_offset))?;
                let ss = self.read_word_linear(bus, self.tr_base.wrapping_add(tss_ss_offset))?;
                (sp as u32, ss)
            };

            let old_ss = self.sregs[SegReg32::SS as usize];
            let old_sp = if from_vm86 || self.use_esp() {
                self.regs.dword(crate::DwordReg::ESP)
            } else {
                self.regs.word(crate::WordReg::SP) as u32
            };
            let old_es = self.sregs[SegReg32::ES as usize];
            let old_ds = self.sregs[SegReg32::DS as usize];
            let old_fs = self.sregs[SegReg32::FS as usize];
            let old_gs = self.sregs[SegReg32::GS as usize];
            let mut old_eflags = self.eflags_upper | self.flags.compress() as u32;
            if is_fault {
                // 386 PRM 9.7: faults push EFLAGS with RF=1 so that IRET back
                // restarts the faulting instruction without immediately
                // re-triggering an instruction breakpoint.
                old_eflags |= 0x0001_0000;
            }
            let old_cs = self.sregs[SegReg32::CS as usize];

            let ss_error_code = Self::segment_error_code(new_ss) + ext;
            if new_ss & 0xFFFC == 0 {
                return self.raise_fault_with_code(10, ss_error_code, bus);
            }
            let Some(ss_descriptor) = self.decode_descriptor(new_ss, bus)? else {
                return self.raise_fault_with_code(10, ss_error_code, bus);
            };
            let ss_rights = ss_descriptor.rights;
            let ss_dpl = Self::descriptor_dpl(ss_rights);
            let ss_rpl = new_ss & 0x0003;
            if !Self::descriptor_is_segment(ss_rights) || !Self::descriptor_is_writable(ss_rights) {
                return self.raise_fault_with_code(10, ss_error_code, bus);
            }
            if ss_dpl != new_dpl || ss_rpl != new_dpl {
                return self.raise_fault_with_code(10, ss_error_code, bus);
            }
            if !Self::descriptor_present(ss_rights) {
                return self.raise_fault_with_code(12, ss_error_code, bus);
            }

            // Probe-then-commit: translate every byte the dispatch will
            // push onto the new kernel stack BEFORE committing SS/ESP/VM.
            let new_ss_base = ss_descriptor.base;
            let new_b_bit = (ss_descriptor.granularity & 0x40) != 0;
            let push_size: u32 = if is_386_gate { 4 } else { 2 };
            let push_count: u32 =
                if from_vm86 { 4 } else { 0 } + 5 + if error_code.is_some() { 1 } else { 0 };

            // Probe each push slot at its starting linear address (and at
            // its last byte, for the rare cross-page case).
            for i in 1..=push_count {
                let raw_off = new_esp.wrapping_sub(i * push_size);
                let off_lo = if new_b_bit {
                    raw_off
                } else {
                    raw_off as u16 as u32
                };
                let linear_lo = new_ss_base.wrapping_add(off_lo);
                self.translate_linear(linear_lo, true, bus)?;
                if push_size > 1 {
                    let raw_hi = raw_off.wrapping_add(push_size - 1);
                    let off_hi = if new_b_bit {
                        raw_hi
                    } else {
                        raw_hi as u16 as u32
                    };
                    let linear_hi = new_ss_base.wrapping_add(off_hi);
                    if (linear_hi & !0xFFF) != (linear_lo & !0xFFF) {
                        self.translate_linear(linear_hi, true, bus)?;
                    }
                }
            }

            self.set_accessed_bit(new_ss, bus)?;
            self.set_loaded_segment_cache(SegReg32::SS, new_ss, ss_descriptor);
            self.eflags_upper &= !0x0003_0000; // Clear RF and VM before setting ESP.
            if self.use_esp() {
                self.regs.set_dword(crate::DwordReg::ESP, new_esp);
            } else {
                self.regs.set_word(crate::WordReg::SP, new_esp as u16);
            }

            if is_386_gate {
                if from_vm86 {
                    self.push_dword(bus, old_gs as u32)?;
                    self.push_dword(bus, old_fs as u32)?;
                    self.push_dword(bus, old_ds as u32)?;
                    self.push_dword(bus, old_es as u32)?;
                }
                self.push_dword(bus, old_ss as u32)?;
                self.push_dword(bus, old_sp)?;
                self.push_dword(bus, old_eflags)?;
                self.push_dword(bus, old_cs as u32)?;
                self.push_dword(bus, return_eip)?;
                if let Some(code) = error_code {
                    self.push_dword(bus, code as u32)?;
                }
            } else {
                if from_vm86 {
                    self.push(bus, old_gs)?;
                    self.push(bus, old_fs)?;
                    self.push(bus, old_ds)?;
                    self.push(bus, old_es)?;
                }
                self.push(bus, old_ss)?;
                self.push(bus, old_sp as u16)?;
                self.push(bus, old_eflags as u16)?;
                self.push(bus, old_cs)?;
                self.push(bus, return_eip as u16)?;
                if let Some(code) = error_code {
                    self.push(bus, code)?;
                }
            }

            self.set_accessed_bit(gate_selector, bus)?;
            let adjusted_selector = (gate_selector & !3) | new_dpl;
            self.set_loaded_segment_cache(SegReg32::CS, adjusted_selector, descriptor);
            self.ip = gate_ip as u16;
            self.ip_upper = gate_ip & 0xFFFF_0000;

            self.flags.tf = false;
            self.flags.nt = false;
            if gate_type == INTGATE_286 || gate_type == INTGATE_386 {
                self.flags.if_flag = false;
            }

            if from_vm86 {
                self.set_null_segment(SegReg32::ES, 0);
                self.set_null_segment(SegReg32::DS, 0);
                self.set_null_segment(SegReg32::FS, 0);
                self.set_null_segment(SegReg32::GS, 0);
            }
            return Ok(());
        }

        // Same-privilege interrupt.
        if is_386_gate {
            let mut eflags = self.eflags_upper | self.flags.compress() as u32;
            if is_fault {
                eflags |= 0x0001_0000;
            }
            let cs = self.sregs[SegReg32::CS as usize];
            self.push_dword(bus, eflags)?;
            self.push_dword(bus, cs as u32)?;
            self.push_dword(bus, return_eip)?;
            if let Some(code) = error_code {
                self.push_dword(bus, code as u32)?;
            }
        } else {
            let flags_val = self.flags.compress();
            let cs = self.sregs[SegReg32::CS as usize];
            self.push(bus, flags_val)?;
            self.push(bus, cs)?;
            self.push(bus, return_eip as u16)?;
            if let Some(code) = error_code {
                self.push(bus, code)?;
            }
        }

        self.set_accessed_bit(gate_selector, bus)?;
        let adjusted_selector = (gate_selector & !3) | cpl;
        self.set_loaded_segment_cache(SegReg32::CS, adjusted_selector, descriptor);
        self.ip = gate_ip as u16;
        self.ip_upper = gate_ip & 0xFFFF_0000;

        self.flags.tf = false;
        self.flags.nt = false;
        self.eflags_upper &= !0x0003_0000; // Clear RF and VM.
        if gate_type == INTGATE_286 || gate_type == INTGATE_386 {
            self.flags.if_flag = false;
        }
        Ok(())
    }

    pub(super) fn raise_interrupt(&mut self, vector: u8, bus: &mut impl common::Bus) -> Step {
        let return_eip = if self.rep_active {
            self.rep_restart_ip_upper | self.rep_restart_ip as u32
        } else {
            self.ip_upper | self.ip as u32
        };
        self.interrupt_with_return_eip(vector, return_eip, None, false, true, false, bus)
    }

    pub(super) fn raise_software_interrupt(
        &mut self,
        vector: u8,
        is_int_n: bool,
        bus: &mut impl common::Bus,
    ) -> Step {
        let return_eip = if self.rep_active {
            self.rep_restart_ip_upper | self.rep_restart_ip as u32
        } else {
            self.ip_upper | self.ip as u32
        };
        // In VM86, only INT n (opcode 0xCD) is IOPL-sensitive.
        // INT 3 and INTO always go through the IDT without an IOPL check.
        if self.is_virtual_mode() && is_int_n && self.flags.iopl < 3 {
            return self.raise_fault_with_code(13, 0, bus);
        }
        self.interrupt_with_return_eip(vector, return_eip, None, true, false, false, bus)
    }

    pub(super) fn raise_trap(&mut self, vector: u8, bus: &mut impl common::Bus) -> Step {
        let return_eip = if self.rep_active {
            self.rep_restart_ip_upper | self.rep_restart_ip as u32
        } else {
            self.ip_upper | self.ip as u32
        };
        self.interrupt_with_return_eip(vector, return_eip, None, false, false, false, bus)
    }

    pub(super) fn raise_fault<T>(&mut self, vector: u8, bus: &mut impl common::Bus) -> Step<T> {
        if self.shutdown {
            return Err(Fault);
        }
        let saved_fault_pending = self.fault_pending;
        self.fault_pending = false;
        let return_eip = self.prev_ip_upper | self.prev_ip as u32;
        if self.is_protected_mode() {
            match self.check_double_fault(vector) {
                DoubleFaultResult::Shutdown => return Err(Fault),
                DoubleFaultResult::DoubleFault => {
                    self.interrupt_with_return_eip(
                        8,
                        return_eip,
                        Some(0),
                        false,
                        false,
                        true,
                        bus,
                    )?;
                    self.trap_level = 0;
                    self.fault_pending = true;
                    return Err(Fault);
                }
                DoubleFaultResult::Normal => {}
            }
        }
        let _ = self.interrupt_with_return_eip(vector, return_eip, None, false, false, true, bus);
        self.trap_level = 0;
        self.fault_pending = saved_fault_pending || self.fault_pending;
        Err(Fault)
    }

    pub(super) fn raise_fault_with_code<T>(
        &mut self,
        vector: u8,
        error_code: u16,
        bus: &mut impl common::Bus,
    ) -> Step<T> {
        if self.shutdown {
            return Err(Fault);
        }
        let saved_fault_pending = self.fault_pending;
        self.fault_pending = false;
        let return_eip = self.prev_ip_upper | self.prev_ip as u32;
        if self.is_protected_mode() {
            match self.check_double_fault(vector) {
                DoubleFaultResult::Shutdown => return Err(Fault),
                DoubleFaultResult::DoubleFault => {
                    self.interrupt_with_return_eip(
                        8,
                        return_eip,
                        Some(0),
                        false,
                        false,
                        true,
                        bus,
                    )?;
                    self.trap_level = 0;
                    self.fault_pending = true;
                    return Err(Fault);
                }
                DoubleFaultResult::Normal => {}
            }
        }
        let _ = self.interrupt_with_return_eip(
            vector,
            return_eip,
            Some(error_code),
            false,
            false,
            true,
            bus,
        );
        self.trap_level = 0;
        self.fault_pending = saved_fault_pending || self.fault_pending;
        Err(Fault)
    }

    fn check_double_fault(&mut self, vector: u8) -> DoubleFaultResult {
        let current_class = exception_class(vector);
        self.trap_level += 1;

        // Triple fault: 3+ consecutive exceptions, or any exception during DF delivery.
        if self.trap_level >= 3 || (self.trap_level >= 2 && self.prev_exception_class == 3) {
            self.shutdown = true;
            self.halted = true;
            return DoubleFaultResult::Shutdown;
        }

        // Double fault: check the escalation table for the prev/current combination.
        if self.trap_level >= 2 {
            let prev = self.prev_exception_class as usize;
            let curr = current_class as usize;
            if ESCALATION[prev][curr] {
                self.prev_exception_class = 3;
                return DoubleFaultResult::DoubleFault;
            }
        }

        self.prev_exception_class = current_class;
        DoubleFaultResult::Normal
    }
}
