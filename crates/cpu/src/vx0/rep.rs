use super::{VX0, biu::queue_size_for};
use crate::{SegReg16, WordReg};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum RepType {
    RepNc,
    RepC,
    RepNe,
    RepE,
}

save_state::runtime_state! {
    /// Saved state of an in-progress REP-prefixed string operation.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) struct RepState {
        pub(super) ip: u16,
        pub(super) restart_ip: u16,
        pub(super) seg_prefix: bool,
        pub(super) prefix_seg: SegReg16,
        pub(super) prefix: bool,
        pub(super) opcode: u8,
        pub(super) type_: u8,
        pub(super) active: bool,
    }
}

impl RepState {
    pub(super) const fn new() -> Self {
        Self {
            ip: 0,
            restart_ip: 0,
            seg_prefix: false,
            prefix_seg: SegReg16::DS,
            prefix: false,
            opcode: 0,
            type_: 0,
            active: false,
        }
    }
}

impl Default for RepState {
    fn default() -> Self {
        Self::new()
    }
}

impl<const MODEL: u8> VX0<MODEL> {
    fn rep_io_string_startup_cycles(&self, count: u16, entry_queue_len: usize) -> i32 {
        if count == 0 {
            5
        } else if entry_queue_len == queue_size_for(MODEL) {
            if self.seg_prefix { 3 } else { 5 }
        } else {
            0
        }
    }

    fn finish_rep_io_string_startup_prefetch(&mut self, bus: &mut impl common::Bus) {
        if self.biu_latch_is_code_fetch() {
            self.biu_bus_wait_finish(bus);
            self.biu_complete_code_fetch_for_eu();
        } else {
            self.biu_complete_current_bus_for_eu();
        }

        self.biu_start_code_fetch_for_eu();
        self.biu_bus_wait_finish(bus);
        self.biu_complete_code_fetch_for_eu();
    }

    fn ready_rep_io_string_first_bus(&mut self, next: u8) {
        match next {
            0x6C | 0x6D => self.biu_ready_io_read(),
            0x6E | 0x6F => self.biu_ready_memory_read(),
            _ => {}
        }
    }

    fn finish_rep_io_string_count_zero(&mut self, bus: &mut impl common::Bus) {
        if self.biu_latch_is_code_fetch() {
            self.biu_bus_wait_finish(bus);
            self.biu_complete_code_fetch_for_eu();
        }

        if self.queue_len() == queue_size_for(MODEL) {
            self.clk(bus, 4);
            return;
        }

        self.biu_start_code_fetch_for_eu();
        self.biu_bus_wait_finish(bus);
        self.biu_complete_code_fetch_for_eu();
        if self.queue_len() == queue_size_for(MODEL) && self.seg_prefix {
            self.clk(bus, 2);
            return;
        }
        self.biu_start_code_fetch_for_eu();
        self.cycle(bus);
    }

    fn finish_rep_io_string_nonzero(&mut self, bus: &mut impl common::Bus, opcode: u8) {
        self.biu_complete_current_bus_for_eu();
        if matches!(opcode, 0x6D | 0x6F) {
            return;
        }
        if self.queue_len() < queue_size_for(MODEL) {
            self.biu_start_code_fetch_for_eu();
        }
        self.cycle(bus);
    }

    fn prepare_rep_stos_first_bus(&mut self, bus: &mut impl common::Bus, entry_queue_len: usize) {
        if entry_queue_len != queue_size_for(MODEL) {
            return;
        }

        if self.biu_latch_is_code_fetch() {
            self.biu_bus_wait_finish(bus);
            self.biu_complete_code_fetch_for_eu();
        }

        if self.seg_prefix {
            self.clk(bus, 2);
        } else if self.queue_has_room_for_fetch() {
            self.biu_start_code_fetch_for_eu();
            self.biu_bus_wait_finish(bus);
            self.biu_complete_code_fetch_for_eu();
        }
        self.biu_ready_memory_write();
    }

    fn prepare_rep_memory_read_string_first_bus(
        &mut self,
        bus: &mut impl common::Bus,
        entry_queue_len: usize,
    ) {
        if self.biu_latch_is_code_fetch() {
            self.biu_bus_wait_finish(bus);
            self.biu_complete_code_fetch_for_eu();
        }
        if self.seg_prefix && entry_queue_len == queue_size_for(MODEL) {
            self.clk(bus, 2);
        }
        self.biu_ready_memory_read();
    }

    fn ready_rep_iteration_memory_read_string_first_bus(&mut self, bus: &mut impl common::Bus) {
        if self.biu_latch_is_code_fetch() {
            self.biu_bus_wait_finish(bus);
            self.biu_complete_code_fetch_for_eu();
        }
        self.biu_ready_memory_read();
    }

    fn complete_iteration_prefetch_then_ready_memory_read(
        &mut self,
        bus: &mut impl common::Bus,
        passive_cycles: i32,
    ) {
        let mut guard = 0u32;
        while !self.biu_latch_is_code_fetch() && guard < 8 {
            self.cycle(bus);
            guard += 1;
        }
        if self.biu_latch_is_code_fetch() {
            self.biu_bus_wait_finish(bus);
            self.biu_complete_code_fetch_for_eu();
        }
        self.clk(bus, passive_cycles);
        self.biu_ready_memory_read();
    }

    fn rep_lodsw_iteration_prefetch_gap(
        &mut self,
        bus: &mut impl common::Bus,
        entry_queue_len: usize,
        count: u16,
    ) {
        if count <= 1
            || !(entry_queue_len == 0
                || (self.seg_prefix && entry_queue_len == queue_size_for(MODEL)))
        {
            return;
        }

        if self.biu_latch_is_code_fetch() {
            self.biu_bus_wait_finish(bus);
            self.biu_complete_code_fetch_for_eu();
            self.clk(bus, 2);
            self.biu_ready_memory_read();
        }
    }

    fn start_rep(&mut self, rep_type: RepType, bus: &mut impl common::Bus) {
        self.rep_state.restart_ip = self.prev_ip;
        self.rep_state.prefix = true;
        let count = self.regs.word(WordReg::CX);
        let mut next = self.fetch(bus);

        // Handle segment prefix after REP.
        match next {
            0x26 => {
                self.seg_prefix = true;
                self.prefix_seg = SegReg16::ES;
                next = self.fetch(bus);
                self.clk(bus, 0);
            }
            0x2E => {
                self.seg_prefix = true;
                self.prefix_seg = SegReg16::CS;
                next = self.fetch(bus);
                self.clk(bus, 0);
            }
            0x36 => {
                self.seg_prefix = true;
                self.prefix_seg = SegReg16::SS;
                next = self.fetch(bus);
                self.clk(bus, 0);
            }
            0x3E => {
                self.seg_prefix = true;
                self.prefix_seg = SegReg16::DS;
                next = self.fetch(bus);
                self.clk(bus, 0);
            }
            _ => {}
        }

        let entry_queue_len = self.biu_instruction_entry_queue_len_for_timing();
        let startup = match next {
            0xA4 | 0xA5 => 5, // REP MOVSB/W
            0xA6 | 0xA7 => 5, // REP CMPSB/W
            0xAA | 0xAB => 4, // REP STOSB/W
            0xAC => {
                // REP LODSB
                if count == 0 {
                    5
                } else if count == 1 && entry_queue_len == 0 {
                    4
                } else if entry_queue_len == 0
                    || (self.seg_prefix && entry_queue_len == queue_size_for(MODEL))
                {
                    3
                } else {
                    5
                }
            }
            0xAD => {
                // REP LODSW
                if count != 0 && self.seg_prefix && entry_queue_len == queue_size_for(MODEL) {
                    4
                } else {
                    5
                }
            }
            0xAE | 0xAF => 5, // REP SCASB/W
            0x6C..=0x6F => self.rep_io_string_startup_cycles(count, entry_queue_len),
            _ => 0,
        };
        self.clk(bus, startup);
        if matches!(next, 0x6C..=0x6F) && count != 0 {
            if entry_queue_len == queue_size_for(MODEL) && self.seg_prefix {
                if self.biu_latch_is_code_fetch() {
                    self.biu_bus_wait_finish(bus);
                    self.biu_complete_code_fetch_for_eu();
                }
                self.clk(bus, 2);
                self.ready_rep_io_string_first_bus(next);
            } else if entry_queue_len != queue_size_for(MODEL) {
                self.finish_rep_io_string_startup_prefetch(bus);
                self.ready_rep_io_string_first_bus(next);
            }
        }
        if next == 0xAD
            && count != 0
            && (entry_queue_len == 0
                || (self.seg_prefix && entry_queue_len == queue_size_for(MODEL)))
        {
            if self.biu_latch_is_code_fetch() {
                self.biu_bus_wait_finish(bus);
                self.biu_complete_code_fetch_for_eu();
            }
            if self.seg_prefix && entry_queue_len == queue_size_for(MODEL) {
                self.clk(bus, 2);
            }
            self.biu_ready_memory_read();
        }
        self.do_rep(rep_type, next, bus);
    }

    fn do_rep(&mut self, rep_type: RepType, next: u8, bus: &mut impl common::Bus) {
        let mut count = self.regs.word(WordReg::CX);
        let entry_queue_len = self.biu_instruction_entry_queue_len_for_timing();
        let mut iteration_index = 0u16;
        let is_io_string = matches!(next, 0x6C..=0x6F);

        if count == 0 {
            if is_io_string {
                self.finish_rep_io_string_count_zero(bus);
            }
            if matches!(next, 0xAA | 0xAB) {
                let terminal_cycles =
                    if entry_queue_len == queue_size_for(MODEL) && !self.seg_prefix {
                        8
                    } else {
                        7
                    };
                self.clk(bus, terminal_cycles);
            }
            if matches!(next, 0xA4 | 0xA5) {
                let terminal_cycles =
                    if entry_queue_len == queue_size_for(MODEL) && !self.seg_prefix {
                        7
                    } else {
                        6
                    };
                self.clk(bus, terminal_cycles);
            }
            if matches!(next, 0xA6 | 0xA7) {
                let terminal_cycles =
                    if entry_queue_len == queue_size_for(MODEL) && !self.seg_prefix {
                        7
                    } else {
                        6
                    };
                self.clk(bus, terminal_cycles);
            }
            if matches!(next, 0xAE | 0xAF) {
                let terminal_cycles =
                    if entry_queue_len == queue_size_for(MODEL) && !self.seg_prefix {
                        7
                    } else {
                        6
                    };
                self.clk(bus, terminal_cycles);
            }
            if matches!(next, 0xAC | 0xAD) {
                let terminal_cycles = if self.biu_instruction_entry_queue_len_for_timing()
                    == queue_size_for(MODEL)
                    && !self.seg_prefix
                {
                    7
                } else {
                    6
                };
                self.clk(bus, terminal_cycles);
            }
            return;
        }

        let is_cmps = matches!(next, 0xA6 | 0xA7);
        let is_scas = matches!(next, 0xAE | 0xAF);
        let is_cmps_scas = matches!(next, 0xA6 | 0xA7 | 0xAE | 0xAF);
        let ends_with_write = matches!(next, 0x6C | 0x6D | 0x6E | 0x6F | 0xA4 | 0xA5 | 0xAA | 0xAB);

        if matches!(next, 0xAA | 0xAB) {
            self.prepare_rep_stos_first_bus(bus, entry_queue_len);
        } else if matches!(next, 0xA4 | 0xA5 | 0xA6 | 0xA7 | 0xAE | 0xAF) {
            self.prepare_rep_memory_read_string_first_bus(bus, entry_queue_len);
        }

        loop {
            match next {
                0x6C => self.insb_body(bus),
                0x6D => self.insw_body(bus),
                0x6E => self.outsb_body(bus),
                0x6F => self.outsw_body(bus),
                0xA4 => self.movsb_body(bus),
                0xA5 => self.movsw_body(bus),
                0xA6 => self.rep_cmpsb_body(bus),
                0xA7 => self.rep_cmpsw_body(bus),
                0xAA => self.stosb_body(bus),
                0xAB => self.stosw_body(bus),
                0xAC => {
                    let prefixed_full_queue_first_lodsb = self.seg_prefix
                        && entry_queue_len == queue_size_for(MODEL)
                        && iteration_index == 0;
                    if prefixed_full_queue_first_lodsb {
                        if self.biu_latch_is_code_fetch() {
                            self.biu_bus_wait_finish(bus);
                            self.biu_complete_code_fetch_for_eu();
                        }
                        self.biu_ready_memory_read();
                        self.clk(bus, 2);
                    }
                    self.lodsb_body(bus);
                    let early_lodsb_transient =
                        entry_queue_len == 0 && iteration_index < 2 && count > 1;
                    let prefixed_full_queue_lodsb_transient = self.seg_prefix
                        && entry_queue_len == queue_size_for(MODEL)
                        && iteration_index == 0
                        && count > 1;
                    if early_lodsb_transient || prefixed_full_queue_lodsb_transient {
                        self.clk(bus, 4);
                        if self.biu_latch_is_code_fetch() {
                            self.biu_complete_code_fetch_for_eu();
                        }
                        self.biu_ready_memory_read();
                        self.clk(bus, 2);
                    } else {
                        self.clk(bus, 2);
                    }
                }
                0xAD => {
                    self.lodsw(bus);
                    self.rep_lodsw_iteration_prefetch_gap(bus, entry_queue_len, count);
                }
                0xAE => self.scasb_body(bus),
                0xAF => self.scasw_body(bus),
                _ => {
                    self.dispatch(next, bus);
                    return;
                }
            }

            count -= 1;
            iteration_index = iteration_index.wrapping_add(1);
            if count == 0 {
                if is_cmps {
                    self.clk(bus, 9);
                } else if is_scas {
                    self.clk(bus, 11);
                }
                break;
            }

            let terminate = if is_cmps_scas {
                match rep_type {
                    RepType::RepNc => self.flags.cf(),
                    RepType::RepC => !self.flags.cf(),
                    RepType::RepNe => self.flags.zf(),
                    RepType::RepE => !self.flags.zf(),
                }
            } else {
                false
            };

            if terminate {
                if is_cmps {
                    self.clk(bus, 9);
                } else if is_scas {
                    self.clk(bus, 11);
                }
                break;
            }

            if is_cmps {
                let cycles = if self.queue_len() >= queue_size_for(MODEL) - 1 {
                    6
                } else {
                    8
                };
                self.clk(bus, cycles);
                self.ready_rep_iteration_memory_read_string_first_bus(bus);
            } else if is_scas {
                if self.queue_len() <= queue_size_for(MODEL) - 2 {
                    self.clk(bus, 8);
                    self.ready_rep_iteration_memory_read_string_first_bus(bus);
                } else if self.queue_len() == queue_size_for(MODEL) - 1 {
                    self.complete_iteration_prefetch_then_ready_memory_read(bus, 2);
                } else {
                    self.clk(bus, 3);
                }
            }

            self.cycles_remaining -= bus.drain_wait_cycles();

            // Update the bus cycle so scheduler events (vsync, PIT, etc.)
            // fire at the correct time during long string operations.
            let consumed = (self.run_budget as i64 - self.cycles_remaining) as u64;
            bus.set_current_cycle(self.run_start_cycle + consumed);

            let interrupt_pending = bus.has_nmi() || (self.flags.if_flag && bus.has_irq());

            if self.cycles_remaining <= 0 || interrupt_pending {
                if ends_with_write {
                    self.biu_bus_wait_finish(bus);
                }

                // Save state for resume.
                self.rep_state.active = true;
                self.rep_state.ip = self.ip;
                self.rep_state.seg_prefix = self.seg_prefix;
                self.rep_state.prefix_seg = self.prefix_seg;
                self.rep_state.opcode = next;
                self.rep_state.type_ = match rep_type {
                    RepType::RepNe => 1,
                    RepType::RepNc => 2,
                    RepType::RepC => 3,
                    RepType::RepE => 0,
                };
                self.regs.set_word(WordReg::CX, count);
                self.ip = self.rep_state.restart_ip;
                return;
            }

            if ends_with_write {
                self.biu_chain_eu_transfer();
            }
        }

        if ends_with_write {
            self.biu_bus_wait_finish(bus);
        }

        if is_io_string {
            self.finish_rep_io_string_nonzero(bus, next);
        }

        if next == 0xAC {
            self.clk(bus, 6);
        }
        if next == 0xAD {
            self.clk(bus, 6);
        }
        if next == 0xA4 {
            self.clk(bus, 1);
        }
        if next == 0xAA {
            let terminal_cycles = if entry_queue_len == 0 && iteration_index == 1 {
                2
            } else {
                1
            };
            self.clk(bus, terminal_cycles);
        }

        self.regs.set_word(WordReg::CX, count);
        self.seg_prefix = false;
    }

    pub(super) fn continue_rep(&mut self, bus: &mut impl common::Bus) {
        self.ip = self.rep_state.ip;
        self.seg_prefix = self.rep_state.seg_prefix;
        self.prefix_seg = self.rep_state.prefix_seg;
        let next = self.rep_state.opcode;
        self.rep_state.active = false;
        let rep_type = match self.rep_state.type_ {
            1 => RepType::RepNe,
            2 => RepType::RepNc,
            3 => RepType::RepC,
            _ => RepType::RepE,
        };
        self.do_rep(rep_type, next, bus);
    }

    pub(super) fn repne(&mut self, bus: &mut impl common::Bus) {
        self.start_rep(RepType::RepNe, bus);
    }

    pub(super) fn repe(&mut self, bus: &mut impl common::Bus) {
        self.start_rep(RepType::RepE, bus);
    }

    pub(super) fn repnc(&mut self, bus: &mut impl common::Bus) {
        self.start_rep(RepType::RepNc, bus);
    }

    pub(super) fn repc(&mut self, bus: &mut impl common::Bus) {
        self.start_rep(RepType::RepC, bus);
    }
}
