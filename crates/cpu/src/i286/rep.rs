use super::{
    ADDRESS_MASK, I286,
    string_ops::{rep_string_complete_cycles, rep_string_odd_di_adjustment, string_timing},
    timing::I286FinishState,
};
use crate::{SegReg16, WordReg};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum RepType {
    RepNe,
    RepE,
}

impl I286 {
    fn peek_rep_opcode(&self, bus: &mut impl common::Bus) -> u8 {
        let code_segment_base = self.seg_bases[SegReg16::CS as usize];
        bus.read_byte(code_segment_base.wrapping_add(u32::from(self.ip)) & ADDRESS_MASK)
    }

    fn handle_rep_segment_prefix(
        &mut self,
        bus: &mut impl common::Bus,
        prefix_seg: SegReg16,
    ) -> u8 {
        self.seg_prefix = true;
        self.prefix_seg = prefix_seg;
        self.state.timing.note_prefix();
        let next = self.peek_rep_opcode(bus);
        if !Self::string_opcode(next) || self.state.timing.prefix_count_is_odd() {
            self.clk_prefix(bus);
        } else {
            self.clk_visible(1);
        }
        self.fetch(bus)
    }

    fn start_rep(&mut self, rep_type: RepType, bus: &mut impl common::Bus) {
        self.rep_restart_ip = self.prev_ip;
        self.state.timing.note_rep_startup();
        let mut next = self.fetch(bus);
        let mut rep_prefix_seen = false;

        // Handle any number of prefixes between REP and opcode.
        loop {
            match next {
                0x26 => {
                    rep_prefix_seen = true;
                    next = self.handle_rep_segment_prefix(bus, SegReg16::ES);
                }
                0x2E => {
                    rep_prefix_seen = true;
                    next = self.handle_rep_segment_prefix(bus, SegReg16::CS);
                }
                0x36 => {
                    rep_prefix_seen = true;
                    next = self.handle_rep_segment_prefix(bus, SegReg16::SS);
                }
                0x3E => {
                    rep_prefix_seen = true;
                    next = self.handle_rep_segment_prefix(bus, SegReg16::DS);
                }
                0xF0 => {
                    rep_prefix_seen = true;
                    self.state.timing.note_lock_prefix(0);
                    next = self.fetch(bus);
                    self.clk_lock_prefix(bus, next, 0, true);
                }
                _ => break,
            }
        }

        if Self::string_opcode(next) {
            let timing = string_timing(next);
            let mut startup = if self.regs.word(WordReg::CX) == 0 {
                timing.rep_zero_count_startup_cycles
            } else {
                timing.rep_startup_cycles
            };
            if rep_prefix_seen || self.state.timing.prefix_count_is_odd() {
                startup = startup.saturating_sub(1);
            }
            self.clk_visible(startup);
        } else {
            self.clk_prefetch(bus, 2);
        }
        self.do_rep(rep_type, next, bus);
    }

    fn do_rep(&mut self, rep_type: RepType, next: u8, bus: &mut impl common::Bus) {
        let mut count = self.regs.word(WordReg::CX);

        if count == 0 {
            return;
        }

        let is_cmps_scas = matches!(next, 0xA6 | 0xA7 | 0xAE | 0xAF);

        loop {
            self.state.timing.note_rep_iteration();
            self.finish_state = I286FinishState::RepSteadyState;

            let di_before = self.regs.word(WordReg::DI);

            match next {
                0x6C => self.insb_body(bus),
                0x6D => self.insw_body(bus),
                0x6E => self.outsb_body(bus),
                0x6F => self.outsw_body(bus),
                0xA4 => self.movsb_body(bus),
                0xA5 => self.movsw_body(bus),
                0xA6 => self.cmpsb_body(bus),
                0xA7 => self.cmpsw_body(bus),
                0xAA => self.stosb_body(bus),
                0xAB => self.stosw_body(bus),
                0xAC => self.lodsb_body(bus),
                0xAD => self.lodsw_body(bus),
                0xAE => self.scasb_body(bus),
                0xAF => self.scasw_body(bus),
                _ => {
                    self.dispatch(next, bus);
                    return;
                }
            }

            let timing = string_timing(next);
            let odd_di_adjustment = rep_string_odd_di_adjustment(timing, di_before);
            self.clk(timing.rep_iter_base_cycles + odd_di_adjustment);

            count -= 1;
            if count == 0 {
                break;
            }

            let terminate = if is_cmps_scas {
                match rep_type {
                    RepType::RepNe => self.flags.zf(),
                    RepType::RepE => !self.flags.zf(),
                }
            } else {
                false
            };

            if terminate {
                break;
            }

            self.cycles_remaining -= bus.drain_wait_cycles();

            // Update the bus cycle so scheduler events (vsync, PIT, etc.)
            // fire at the correct time during long string operations.
            let consumed = (self.run_budget as i64 - self.cycles_remaining) as u64;
            bus.set_current_cycle(self.run_start_cycle + consumed);

            let interrupt_pending = bus.has_nmi() || (self.flags.if_flag && bus.has_irq());

            if self.cycles_remaining <= 0 || interrupt_pending {
                // Save state for resume.
                self.state.timing.note_rep_suspend();
                self.finish_state = I286FinishState::RepSuspended;
                self.rep_active = true;
                self.rep_ip = self.ip;
                self.rep_seg_prefix = self.seg_prefix;
                self.rep_prefix_seg = self.prefix_seg;
                self.rep_opcode = next;
                self.rep_type = match rep_type {
                    RepType::RepNe => 1,
                    RepType::RepE => 0,
                };
                self.regs.set_word(WordReg::CX, count);
                self.ip = self.rep_restart_ip;
                return;
            }
        }

        self.regs.set_word(WordReg::CX, count);
        self.seg_prefix = false;
        let complete_cycles =
            rep_string_complete_cycles(string_timing(next), self.regs.word(WordReg::DI));
        if complete_cycles != 0 {
            self.clk_visible(complete_cycles);
        }
        self.state.timing.note_rep_complete();
        self.finish_state = I286FinishState::RepComplete;
    }

    pub(super) fn continue_rep(&mut self, bus: &mut impl common::Bus) {
        self.ip = self.rep_ip;
        self.seg_prefix = self.rep_seg_prefix;
        self.prefix_seg = self.rep_prefix_seg;
        let next = self.rep_opcode;
        let rep_type = if self.rep_type == 1 {
            RepType::RepNe
        } else {
            RepType::RepE
        };
        self.rep_active = false;
        self.state.timing.note_rep_iteration();
        self.finish_state = I286FinishState::RepSteadyState;
        self.do_rep(rep_type, next, bus);
    }

    pub(super) fn repne(&mut self, bus: &mut impl common::Bus) {
        self.start_rep(RepType::RepNe, bus);
    }

    pub(super) fn repe(&mut self, bus: &mut impl common::Bus) {
        self.start_rep(RepType::RepE, bus);
    }
}
