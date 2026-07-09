//! Motorola 68000 CPU core.
//!
//! The instruction bodies are generated from MAME's compact MC68000 decode
//! and microcode tables. This file provides the Rust shell, public state
//! surface, ALU helpers, and bus facade used by that checked-in generated code.

#[cfg(feature = "verification")]
use alloc::vec::Vec;

use common::{Bus, CpuM68000};

/// Default Sharp X68000 68000 input clock in Hz.
pub const M68000_DEFAULT_CLOCK_HZ: u32 = 10_000_000;

const ADDRESS_MASK: u32 = 0x00FF_FFFF;
const RUN_BUDGET: i32 = 1 << 28;
const MAX_STATE_STEPS: usize = 4096;

const S_RESET: u32 = 0;
const S_BUS_ERROR: u32 = 1;
const S_ADDRESS_ERROR: u32 = 2;
#[allow(dead_code)]
const S_DOUBLE_FAULT: u32 = 3;
const S_INTERRUPT: u32 = 4;
const S_TRACE: u32 = 5;
const S_ILLEGAL: u32 = 6;
const S_PRIVILEDGE: u32 = 7;
#[allow(dead_code)]
const S_LINEA: u32 = 8;
#[allow(dead_code)]
const S_LINEF: u32 = 9;
const S_FIRST_INSTRUCTION: u32 = S_ILLEGAL;

const SR_C: u32 = 0x0001;
const SR_V: u32 = 0x0002;
const SR_Z: u32 = 0x0004;
const SR_N: u32 = 0x0008;
const SR_X: u32 = 0x0010;
const SR_I: u32 = 0x0700;
const SR_S: u32 = 0x2000;
const SR_T: u32 = 0x8000;
const SR_CCR: u32 = SR_C | SR_V | SR_Z | SR_N | SR_X;
const SR_SR: u32 = SR_I | SR_S | SR_T;

const SSW_DATA: u32 = 0x01;
const SSW_PROGRAM: u32 = 0x02;
const SSW_CPU: u32 = 0x03;
const SSW_S: u32 = 0x04;
const SSW_N: u32 = 0x08;
const SSW_R: u32 = 0x10;
const SSW_CRITICAL: u32 = 0x20;

const PR_NONE: u32 = 0;
const PR_BERR: u32 = 1;

/// Packed 68000 status register flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct M68000Flags {
    bits: u16,
}

impl M68000Flags {
    /// Creates flags from a packed status register value.
    pub const fn from_bits(bits: u16) -> Self {
        Self { bits }
    }

    /// Returns the packed status register value.
    pub const fn bits(self) -> u16 {
        self.bits
    }
}

/// Motorola 68000 bus cycle direction used by verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum M68000BusDirection {
    /// Memory or CPU-space read.
    Read,
    /// Memory write.
    Write,
}

/// Motorola 68000 bus cycle transfer size used by verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum M68000BusSize {
    /// 8-bit transfer.
    Byte,
    /// 16-bit transfer.
    Word,
}

/// Motorola 68000 bus cycle observed by the MOO verification corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct M68000BusCycle {
    /// Cycle timestamp relative to the current instruction.
    pub cycle: u32,
    /// Transfer direction.
    pub direction: M68000BusDirection,
    /// Transfer size.
    pub size: M68000BusSize,
    /// 24-bit physical address.
    pub address: u32,
    /// Data value on the 16-bit data bus.
    pub data: u16,
    /// 68000 function code.
    pub function_code: u8,
    /// Four-byte status tag used by the MOO verification corpus.
    pub status: [u8; 4],
}

/// Public Motorola 68000 CPU state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct M68000State {
    /// Data registers D0-D7.
    pub data: [u32; 8],
    /// Address registers A0-A6. A7 is selected from USP/SSP by SR.S.
    pub address: [u32; 7],
    /// User stack pointer.
    pub usp: u32,
    /// Supervisor stack pointer.
    pub ssp: u32,
    /// Instruction program counter.
    pub pc: u32,
    /// Status register.
    pub sr: u16,
    /// Prefetched instruction word.
    pub ir: u16,
    /// Prefetched extension word.
    pub irc: u16,
}

impl Default for M68000State {
    fn default() -> Self {
        Self {
            data: [0; 8],
            address: [0; 7],
            usp: 0,
            ssp: 0,
            pc: 0,
            sr: 0x2700,
            ir: 0,
            irc: 0,
        }
    }
}

/// Motorola 68000 CPU core.
pub struct M68000 {
    clock_hz: u32,
    last_cycles: u64,
    stopped: bool,
    m_decode_table: [u32; 0x10000],
    m_da: [u32; 17],
    m_ipc: u32,
    m_pc: u32,
    m_au: u32,
    m_at: u32,
    m_aob: u32,
    m_dt: u32,
    m_int_vector: u32,
    m_sp: usize,
    m_icount: i32,
    #[allow(dead_code)]
    m_bcount: i32,
    m_count_before_instruction_step: i32,
    m_t: u32,
    m_movems: usize,
    m_isr: u32,
    m_sr: u32,
    m_new_sr: u32,
    m_dbin: u32,
    m_dbout: u32,
    m_edb: u32,
    m_irc: u32,
    m_ir: u32,
    m_ird: u32,
    m_ftu: u32,
    m_aluo: u32,
    m_alue: u32,
    m_alub: u32,
    m_movemr: u32,
    m_irdi: u32,
    m_base_ssw: u32,
    m_ssw: u32,
    m_dcr: u32,
    #[allow(dead_code)]
    m_virq_state: u32,
    m_nmi_pending: u32,
    m_int_level: u32,
    m_int_next_state: u32,
    m_inst_state: u32,
    m_inst_substate: u32,
    m_next_state: u32,
    m_post_run: u32,
    m_post_run_cycles: i32,
    #[cfg(feature = "verification")]
    bus_cycles: Vec<M68000BusCycle>,
}

impl Default for M68000 {
    fn default() -> Self {
        Self::new(M68000_DEFAULT_CLOCK_HZ)
    }
}

impl M68000 {
    /// Creates a new 68000 core with the given input clock.
    pub fn new(clock_hz: u32) -> Self {
        let mut cpu = Self {
            clock_hz,
            last_cycles: 0,
            stopped: false,
            m_decode_table: [S_ILLEGAL; 0x10000],
            m_da: [0; 17],
            m_ipc: 0,
            m_pc: 0,
            m_au: 0,
            m_at: 0,
            m_aob: 0,
            m_dt: 0,
            m_int_vector: 0,
            m_sp: 16,
            m_icount: 0,
            m_bcount: 0,
            m_count_before_instruction_step: 0,
            m_t: 0,
            m_movems: 0,
            m_isr: 0,
            m_sr: SR_S | SR_I,
            m_new_sr: 0,
            m_dbin: 0,
            m_dbout: 0,
            m_edb: 0,
            m_irc: 0,
            m_ir: 0,
            m_ird: 0,
            m_ftu: 0,
            m_aluo: 0,
            m_alue: 0,
            m_alub: 0,
            m_movemr: 0,
            m_irdi: 0,
            m_base_ssw: 0,
            m_ssw: 0,
            m_dcr: 0,
            m_virq_state: 0,
            m_nmi_pending: 0,
            m_int_level: 0,
            m_int_next_state: 0,
            m_inst_state: S_RESET,
            m_inst_substate: 0,
            m_next_state: 0,
            m_post_run: PR_NONE,
            m_post_run_cycles: 0,
            #[cfg(feature = "verification")]
            bus_cycles: Vec::new(),
        };
        cpu.init_decode_table();
        cpu.update_user_super();
        cpu
    }

    /// Loads public CPU state and primes the two-word prefetch queue.
    pub fn load_state(&mut self, state: M68000State) {
        self.m_da[..8].copy_from_slice(&state.data);
        self.m_da[8..15].copy_from_slice(&state.address);
        self.m_da[15] = state.usp;
        self.m_da[16] = state.ssp;
        self.m_sr = u32::from(state.sr) & (SR_SR | SR_CCR);
        self.update_user_super();
        self.prime_prefetch(state.pc, state.ir, state.irc);
        self.stopped = false;
    }

    /// Saves the public CPU state.
    pub fn save_state(&self) -> M68000State {
        let mut data = [0; 8];
        let mut address = [0; 7];
        data.copy_from_slice(&self.m_da[..8]);
        address.copy_from_slice(&self.m_da[8..15]);
        M68000State {
            data,
            address,
            usp: self.m_da[15],
            ssp: self.m_da[16],
            pc: self.m_pc.wrapping_sub(2) & ADDRESS_MASK,
            sr: self.m_sr as u16,
            ir: self.m_ir as u16,
            irc: self.m_irc as u16,
        }
    }

    /// Primes MAME's internal prefetch latches for a known instruction boundary.
    pub fn prime_prefetch(&mut self, pc: u32, ir: u16, irc: u16) {
        self.m_ipc = pc & ADDRESS_MASK;
        self.m_pc = pc.wrapping_add(2) & ADDRESS_MASK;
        self.m_au = pc.wrapping_add(4) & ADDRESS_MASK;
        self.m_ir = u32::from(ir);
        self.m_ird = u32::from(ir);
        self.m_irdi = u32::from(ir);
        self.m_irc = u32::from(irc);
        self.m_dbin = u32::from(irc);
        self.set_ftu_const();
        self.m_inst_state = self.m_decode_table[self.m_ird as usize];
        self.m_inst_substate = 0;
        self.m_next_state = 0;
    }

    /// Returns the cycle count consumed by the last `step` call.
    pub const fn cycles_consumed(&self) -> u64 {
        self.last_cycles
    }

    /// Returns bus cycles captured during the last single-instruction run.
    #[cfg(feature = "verification")]
    pub fn bus_cycles(&self) -> &[M68000BusCycle] {
        &self.bus_cycles
    }

    fn init_decode_table(&mut self) {
        self.m_decode_table.fill(S_ILLEGAL);
        for &(value, mask, state) in PACKED_DECODE_TABLE {
            let mut cvalue = 0u16;
            loop {
                let opcode = value | cvalue;
                if self.m_decode_table[opcode as usize] == S_ILLEGAL {
                    self.m_decode_table[opcode as usize] = state;
                }
                cvalue = ((cvalue | mask).wrapping_add(1)) & !mask;
                if cvalue == 0 {
                    break;
                }
            }
        }
    }

    fn run_one_instruction(&mut self, bus: &mut impl Bus) -> u64 {
        #[cfg(feature = "verification")]
        self.bus_cycles.clear();

        self.poll_interrupts(bus);
        if self.stopped {
            if self.m_int_next_state == 0 {
                self.last_cycles = 0;
                return 0;
            }
            self.stopped = false;
        }
        self.m_icount = RUN_BUDGET;
        let mut boundary_count = 0;
        for _ in 0..MAX_STATE_STEPS {
            if self.m_inst_state == S_DOUBLE_FAULT {
                self.stopped = true;
                break;
            }
            if self.m_inst_state >= S_FIRST_INSTRUCTION && self.m_inst_substate == 0 {
                boundary_count += 1;
                if boundary_count == 2 {
                    break;
                }
                self.m_ipc = self.m_pc.wrapping_sub(2) & ADDRESS_MASK;
                self.m_irdi = self.m_ird;
            }
            let state = self.m_inst_state;
            self.dispatch_full(bus, state);
            if self.m_post_run != PR_NONE {
                self.do_post_run();
            }
        }
        let consumed = (RUN_BUDGET - self.m_icount).max(0) as u64;
        self.last_cycles = consumed;
        consumed
    }

    fn poll_interrupts(&mut self, bus: &mut impl Bus) {
        let level = bus.m68000_interrupt_level().min(7) as u32;
        if self.m_int_level != level {
            if self.m_int_level != 7 && level == 7 {
                self.m_nmi_pending = 1;
            }
            self.m_int_level = level;
            self.update_interrupt();
        }
    }

    fn read_program(&mut self, bus: &mut impl Bus, address: u32, mem_mask: u32) -> u32 {
        self.read_space(bus, address, mem_mask, SSW_PROGRAM, *b"r-p-")
    }

    fn read_data(&mut self, bus: &mut impl Bus, address: u32, mem_mask: u32) -> u32 {
        self.read_space(bus, address, mem_mask, SSW_DATA, *b"r-d-")
    }

    fn read_cpu(&mut self, bus: &mut impl Bus, address: u32, mem_mask: u32) -> u32 {
        let level = (self.m_next_state >> 24).min(7) as u8;
        let vector = bus.m68000_acknowledge_interrupt(level);
        self.record_bus_cycle(M68000BusCycle {
            cycle: 0,
            direction: M68000BusDirection::Read,
            size: M68000BusSize::Byte,
            address,
            data: u16::from(vector),
            function_code: 7,
            status: *b"r-c-",
        });
        if mem_mask == 0xFF00 {
            u32::from(vector) << 8
        } else {
            u32::from(vector)
        }
    }

    fn read_space(
        &mut self,
        bus: &mut impl Bus,
        address: u32,
        mem_mask: u32,
        space: u32,
        status: [u8; 4],
    ) -> u32 {
        let address = address & ADDRESS_MASK;
        let high = bus.read_byte(address) as u32;
        let low = bus.read_byte(address.wrapping_add(1) & ADDRESS_MASK) as u32;
        let value = (high << 8) | low;
        let (size, cycle_address, cycle_data) = if mem_mask == 0xFFFF {
            (M68000BusSize::Word, address, value as u16)
        } else if mem_mask == 0xFF00 {
            (M68000BusSize::Byte, address, high as u16)
        } else {
            (
                M68000BusSize::Byte,
                address.wrapping_add(1) & ADDRESS_MASK,
                low as u16,
            )
        };
        self.record_bus_cycle(M68000BusCycle {
            cycle: 0,
            direction: M68000BusDirection::Read,
            size,
            address: cycle_address,
            data: cycle_data,
            function_code: self.function_code(space),
            status,
        });
        value & mem_mask
    }

    fn write_data(&mut self, bus: &mut impl Bus, address: u32, data: u32, mem_mask: u32) {
        self.write_space(bus, address, data, mem_mask);
    }

    fn write_tas_data(&mut self, bus: &mut impl Bus, address: u32, data: u32) {
        let address = address & ADDRESS_MASK;
        let value = if address & 1 == 0 {
            (data >> 8) as u8
        } else {
            data as u8
        };
        bus.write_byte(address, value);
        self.record_bus_cycle(M68000BusCycle {
            cycle: 0,
            direction: M68000BusDirection::Write,
            size: M68000BusSize::Byte,
            address,
            data: u16::from(value),
            function_code: self.function_code(SSW_DATA),
            status: *b"-wd-",
        });
    }

    fn write_space(&mut self, bus: &mut impl Bus, address: u32, data: u32, mem_mask: u32) {
        let address = address & ADDRESS_MASK;
        if mem_mask & 0xFF00 != 0 {
            let value = (data >> 8) as u8;
            bus.write_byte(address, value);
        }
        if mem_mask & 0x00FF != 0 {
            let value = data as u8;
            let byte_address = address.wrapping_add(1) & ADDRESS_MASK;
            bus.write_byte(byte_address, value);
        }
        let (size, cycle_address, cycle_data) = if mem_mask == 0xFFFF {
            (M68000BusSize::Word, address, data as u16)
        } else if mem_mask == 0xFF00 {
            (M68000BusSize::Byte, address, (data >> 8) as u16)
        } else {
            (
                M68000BusSize::Byte,
                address.wrapping_add(1) & ADDRESS_MASK,
                (data & 0xFF) as u16,
            )
        };
        self.record_bus_cycle(M68000BusCycle {
            cycle: 0,
            direction: M68000BusDirection::Write,
            size,
            address: cycle_address,
            data: cycle_data,
            function_code: self.function_code(SSW_DATA),
            status: *b"-wd-",
        });
    }

    #[cfg(feature = "verification")]
    fn record_bus_cycle(&mut self, mut cycle: M68000BusCycle) {
        cycle.cycle = (RUN_BUDGET - self.m_icount).max(0) as u32;
        cycle.address &= ADDRESS_MASK;
        self.bus_cycles.push(cycle);
    }

    #[cfg(not(feature = "verification"))]
    fn record_bus_cycle(&mut self, _cycle: M68000BusCycle) {}

    fn function_code(&self, space: u32) -> u8 {
        match space & 3 {
            SSW_PROGRAM => {
                if self.m_sr & SR_S != 0 {
                    6
                } else {
                    2
                }
            }
            SSW_CPU => 7,
            _ => {
                if self.m_sr & SR_S != 0 {
                    5
                } else {
                    1
                }
            }
        }
    }

    fn access_to_be_redone(&self) -> bool {
        false
    }

    #[allow(dead_code)]
    fn abort_access(&mut self, reason: u32) {
        self.m_post_run = reason;
        self.m_post_run_cycles = self.m_icount;
        self.m_icount = 0;
    }

    fn do_post_run(&mut self) {
        self.m_icount = self.m_post_run_cycles;
        self.m_post_run_cycles = 0;
        if self.m_post_run == PR_BERR {
            self.m_inst_state = S_BUS_ERROR;
            self.m_inst_substate = 0;
            self.m_icount -= 10;
        }
        self.m_post_run = PR_NONE;
    }

    fn start_interrupt_vector_lookup(&mut self) {
        let level = self.m_next_state >> 24;
        if level == 7 {
            self.m_nmi_pending = 0;
            self.update_interrupt();
        }
    }

    fn end_interrupt_vector_lookup(&mut self) {
        self.m_int_vector = (self.m_edb & 0xFF) << 2;
        self.m_int_next_state = 0;
    }

    fn update_user_super(&mut self) {
        self.m_sp = if self.m_sr & SR_S != 0 { 16 } else { 15 };
    }

    fn update_interrupt(&mut self) {
        if self.m_nmi_pending != 0 {
            self.m_int_next_state = (7 << 24) | S_INTERRUPT;
        } else if self.m_int_level > ((self.m_sr >> 8) & 7) {
            self.m_int_next_state = (self.m_int_level << 24) | S_INTERRUPT;
        } else {
            self.m_int_next_state = 0;
        }
    }

    fn step_movem(&mut self) {
        let mut register = self.m_movemr.trailing_zeros() as usize;
        if register > 15 {
            register = 0;
        }
        self.m_movems = self.map_sp(register as u32);
        self.m_movemr &= !(1u32 << register);
    }

    fn step_movem_predec(&mut self) {
        let mut register = self.m_movemr.trailing_zeros() as usize;
        if register > 15 {
            register = 0;
        }
        self.m_movems = self.map_sp((register ^ 0x0F) as u32);
        self.m_movemr &= !(1u32 << register);
    }

    fn map_sp(&self, register: u32) -> usize {
        if register == 15 {
            self.m_sp
        } else {
            register as usize
        }
    }

    fn set_ftu_const(&mut self) {
        match self.m_ird >> 12 {
            0x4 => self.m_ftu = 0x80,
            0x5 | 0xE => {
                self.m_ftu = (self.m_ird >> 9) & 7;
                if self.m_ftu == 0 {
                    self.m_ftu = 8;
                }
            }
            0x6 | 0x7 => self.m_ftu = s8(self.m_ird),
            0x8 | 0xC => self.m_ftu = 0x0F,
            _ => self.m_ftu = 0,
        }
    }

    fn debugger_exception_hook(&mut self, _vector: u32) {}

    fn debugger_wait_hook(&mut self) {
        self.stopped = true;
    }

    fn cmpild_instr_callback(&mut self, _bus: &mut impl Bus, _register: usize, _value: u32) {}

    fn rte_instr_callback(&mut self, _bus: &mut impl Bus, _asserted: bool) {}

    fn alu_add(&mut self, a: u32, b: u32) {
        let a = a & 0xFFFF;
        let b = b & 0xFFFF;
        let r = b.wrapping_add(a);
        self.m_isr = 0;
        if r & 0xFFFF == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x8000 != 0 {
            self.m_isr |= SR_N;
        }
        if r & 0x10000 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if ((b & a & !r) | ((!b) & (!a) & r)) & 0x8000 != 0 {
            self.m_isr |= SR_V;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_add8(&mut self, a: u32, b: u32) {
        let a = a & 0xFF;
        let b = b & 0xFF;
        let r = b.wrapping_add(a);
        self.m_isr = 0;
        if r & 0xFF == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x80 != 0 {
            self.m_isr |= SR_N;
        }
        if r & 0x100 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if ((b & a & !r) | ((!b) & (!a) & r)) & 0x80 != 0 {
            self.m_isr |= SR_V;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_addc(&mut self, a: u32, b: u32) {
        let a = a & 0xFFFF;
        let b = b & 0xFFFF;
        let r = b
            .wrapping_add(a)
            .wrapping_add(u32::from(self.m_isr & SR_C != 0));
        self.m_isr = 0;
        if r & 0xFFFF == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x8000 != 0 {
            self.m_isr |= SR_N;
        }
        if r & 0x10000 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if ((b & a & !r) | ((!b) & (!a) & r)) & 0x8000 != 0 {
            self.m_isr |= SR_V;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_addx(&mut self, a: u32, b: u32) {
        let a = a & 0xFFFF;
        let b = b & 0xFFFF;
        let r = b
            .wrapping_add(a)
            .wrapping_add(u32::from(self.m_sr & SR_X != 0));
        self.m_isr = 0;
        if r & 0xFFFF == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x8000 != 0 {
            self.m_isr |= SR_N;
        }
        if r & 0x10000 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if ((b & a & !r) | ((!b) & (!a) & r)) & 0x8000 != 0 {
            self.m_isr |= SR_V;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_addx8(&mut self, a: u32, b: u32) {
        let a = a & 0xFF;
        let b = b & 0xFF;
        let r = b
            .wrapping_add(a)
            .wrapping_add(u32::from(self.m_sr & SR_X != 0));
        self.m_isr = 0;
        if r & 0xFF == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x80 != 0 {
            self.m_isr |= SR_N;
        }
        if r & 0x100 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if ((b & a & !r) | ((!b) & (!a) & r)) & 0x80 != 0 {
            self.m_isr |= SR_V;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_and(&mut self, a: u32, b: u32) {
        let r = (b & a) & 0xFFFF;
        self.m_isr = self.m_sr & SR_X;
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x8000 != 0 {
            self.m_isr |= SR_N;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_andx(&mut self, a: u32, b: u32) {
        self.alu_and(a, b);
        self.m_isr = (self.m_isr & !SR_C) | if self.m_sr & SR_X != 0 { SR_C } else { 0 };
    }

    fn alu_and8(&mut self, a: u32, b: u32) {
        let r = (b & a) & 0xFFFF;
        self.m_isr = self.m_sr & SR_X;
        if r & 0xFF == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x80 != 0 {
            self.m_isr |= SR_N;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_and8x(&mut self, a: u32, b: u32) {
        self.alu_and8(a, b);
        self.m_isr = (self.m_isr & !SR_C) | if self.m_sr & SR_X != 0 { SR_C } else { 0 };
    }

    fn alu_or(&mut self, a: u32, b: u32) {
        let r = (b | a) & 0xFFFF;
        self.m_isr = self.m_sr & SR_X;
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x8000 != 0 {
            self.m_isr |= SR_N;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_or8(&mut self, a: u32, b: u32) {
        let r = (b | a) & 0xFF;
        self.m_isr = self.m_sr & SR_X;
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x80 != 0 {
            self.m_isr |= SR_N;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_eor(&mut self, a: u32, b: u32) {
        self.alu_or(a ^ b, 0);
    }

    fn alu_eor8(&mut self, a: u32, b: u32) {
        self.alu_or8(a ^ b, 0);
    }

    fn alu_ext(&mut self, a: u32) {
        let r = s8(a) & 0xFFFF;
        self.m_isr = 0;
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x8000 != 0 {
            self.m_isr |= SR_N;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_not(&mut self, a: u32) {
        self.alu_and(!a, 0xFFFF);
    }

    fn alu_not8(&mut self, a: u32) {
        self.alu_and8(!a, 0xFF);
    }

    fn alu_sub(&mut self, a: u32, b: u32) {
        let a = a & 0xFFFF;
        let b = b & 0xFFFF;
        let r = b.wrapping_sub(a);
        self.m_isr = 0;
        if r & 0xFFFF == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x8000 != 0 {
            self.m_isr |= SR_N;
        }
        if r & 0x10000 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if ((b & !a & !r) | ((!b) & a & r)) & 0x8000 != 0 {
            self.m_isr |= SR_V;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_sub8(&mut self, a: u32, b: u32) {
        let a = a & 0xFF;
        let b = b & 0xFF;
        let r = b.wrapping_sub(a);
        self.m_isr = 0;
        if r & 0xFF == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x80 != 0 {
            self.m_isr |= SR_N;
        }
        if r & 0x100 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if ((b & !a & !r) | ((!b) & a & r)) & 0x80 != 0 {
            self.m_isr |= SR_V;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_subc(&mut self, a: u32, b: u32) {
        let a = a & 0xFFFF;
        let b = b & 0xFFFF;
        let r = b
            .wrapping_sub(a)
            .wrapping_sub(u32::from(self.m_isr & SR_C != 0));
        self.m_isr = 0;
        if r & 0xFFFF == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x8000 != 0 {
            self.m_isr |= SR_N;
        }
        if r & 0x10000 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if ((b & !a & !r) | ((!b) & a & r)) & 0x8000 != 0 {
            self.m_isr |= SR_V;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_subx(&mut self, a: u32, b: u32) {
        let a = a & 0xFFFF;
        let b = b & 0xFFFF;
        let r = b
            .wrapping_sub(a)
            .wrapping_sub(u32::from(self.m_sr & SR_X != 0));
        self.m_isr = 0;
        if r & 0xFFFF == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x8000 != 0 {
            self.m_isr |= SR_N;
        }
        if r & 0x10000 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if ((b & !a & !r) | ((!b) & a & r)) & 0x8000 != 0 {
            self.m_isr |= SR_V;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_subx8(&mut self, a: u32, b: u32) {
        let a = a & 0xFF;
        let b = b & 0xFF;
        let r = b
            .wrapping_sub(a)
            .wrapping_sub(u32::from(self.m_sr & SR_X != 0));
        self.m_isr = 0;
        if r & 0xFF == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x80 != 0 {
            self.m_isr |= SR_N;
        }
        if r & 0x100 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if ((b & !a & !r) | ((!b) & a & r)) & 0x80 != 0 {
            self.m_isr |= SR_V;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_abcd8(&mut self, a: u32, b: u32) {
        let a = a & 0xFF;
        let b = b & 0xFF;
        let carry = u32::from(self.m_sr & SR_X != 0);
        let half = (b & 0x0F).wrapping_add(a & 0x0F).wrapping_add(carry);
        let low_correction = half > 9;
        let r1 = b.wrapping_add(a).wrapping_add(carry);
        let mut r = r1.wrapping_add(if low_correction { 6 } else { 0 });
        if r > 0x9F {
            r = r.wrapping_add(0x60);
        }
        self.m_isr = 0;
        if r & 0xFF == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x80 != 0 {
            self.m_isr |= SR_N;
        }
        if r & 0x300 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if r & 0x80 != 0 && r1 & 0x80 == 0 {
            self.m_isr |= SR_V;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_sbcd8(&mut self, a: u32, b: u32) {
        let a = a & 0xFF;
        let b = b & 0xFF;
        let carry = u32::from(self.m_sr & SR_X != 0);
        let half = (b & 0x0F).wrapping_sub(a & 0x0F).wrapping_sub(carry);
        let low_correction = half & 0x10 != 0;
        let r1 = b.wrapping_sub(a).wrapping_sub(carry);
        let mut r = r1.wrapping_sub(if low_correction { 6 } else { 0 });
        if r1 & 0x100 != 0 {
            r = r.wrapping_sub(0x60);
        }
        self.m_isr = 0;
        if r & 0xFF == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x80 != 0 {
            self.m_isr |= SR_N;
        }
        if r & 0x300 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if r & 0x80 == 0 && r1 & 0x80 != 0 {
            self.m_isr |= SR_V;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_sla0(&mut self, a: u32) {
        let a = a & 0xFFFF;
        let r = (a << 17) | (self.m_alue << 1);
        self.m_isr = self.m_sr & SR_X;
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x8000_0000 != 0 {
            self.m_isr |= SR_N;
        }
        if a & 0x8000 != 0 {
            self.m_isr |= SR_C;
        }
        self.m_alue = r & 0xFFFF;
        self.m_aluo = (r >> 16) & 0xFFFF;
    }

    fn alu_sla1(&mut self, a: u32) {
        let a = a & 0xFFFF;
        let r = (a << 17) | (self.m_alue << 1) | 1;
        self.m_isr = self.m_sr & SR_X;
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x8000_0000 != 0 {
            self.m_isr |= SR_N;
        }
        if a & 0x8000 != 0 {
            self.m_isr |= SR_C;
        }
        self.m_alue = r & 0xFFFF;
        self.m_aluo = (r >> 16) & 0xFFFF;
    }

    fn alu_over(&mut self, a: u32) {
        self.m_isr = SR_V | SR_N;
        self.m_aluo = s8(a) & 0xFFFF;
    }

    fn alu_asl(&mut self, a: u32) {
        let a = a & 0xFFFF;
        let r = (a << 1) & 0xFFFF;
        self.m_isr = self.m_sr & SR_V;
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x8000 != 0 {
            self.m_isr |= SR_N;
        }
        if a & 0x8000 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if (r ^ a) & 0x8000 != 0 {
            self.m_isr |= SR_V;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_asl8(&mut self, a: u32) {
        let a = a & 0xFF;
        let r = (a << 1) & 0xFF;
        self.m_isr = self.m_sr & SR_V;
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x80 != 0 {
            self.m_isr |= SR_N;
        }
        if a & 0x80 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if (r ^ a) & 0x80 != 0 {
            self.m_isr |= SR_V;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_asl32(&mut self, a: u32) {
        let old_high = self.m_alue & 0xFFFF;
        let r = (old_high << 17) | ((a & 0xFFFF) << 1);
        self.m_isr = self.m_sr & SR_V;
        self.finish_shift32(
            r,
            old_high & 0x8000 != 0,
            ((r >> 16) ^ old_high) & 0x8000 != 0,
            true,
        );
    }

    fn alu_asr(&mut self, a: u32) {
        let a = a & 0xFFFF;
        let mut r = a >> 1;
        self.m_isr = 0;
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if a & 0x8000 != 0 {
            r |= 0x8000;
            self.m_isr |= SR_N;
        }
        if a & 1 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_asr8(&mut self, a: u32) {
        let a = a & 0xFF;
        let mut r = a >> 1;
        self.m_isr = 0;
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if a & 0x80 != 0 {
            r |= 0x80;
            self.m_isr |= SR_N;
        }
        if a & 1 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_asr32(&mut self, a: u32) {
        let high = self.m_alue & 0xFFFF;
        let mut r = (high << 15) | ((a & 0xFFFF) >> 1);
        self.m_isr = 0;
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if high & 0x8000 != 0 {
            r |= 0x8000_0000;
            self.m_isr |= SR_N;
        }
        if a & 1 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        self.m_aluo = r & 0xFFFF;
        self.m_alue = (r >> 16) & 0xFFFF;
    }

    fn alu_lsl(&mut self, a: u32) {
        let a = a & 0xFFFF;
        let r = (a << 1) & 0xFFFF;
        self.m_isr = 0;
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x8000 != 0 {
            self.m_isr |= SR_N;
        }
        if a & 0x8000 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_lsl8(&mut self, a: u32) {
        let a = a & 0xFF;
        let r = (a << 1) & 0xFF;
        self.m_isr = 0;
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x80 != 0 {
            self.m_isr |= SR_N;
        }
        if a & 0x80 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_lsl32(&mut self, a: u32) {
        let old_high = self.m_alue & 0xFFFF;
        let r = (old_high << 17) | ((a & 0xFFFF) << 1);
        self.finish_shift32(r, old_high & 0x8000 != 0, false, false);
    }

    fn alu_lsr(&mut self, a: u32) {
        let a = a & 0xFFFF;
        let r = a >> 1;
        self.m_isr = 0;
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if a & 1 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_lsr8(&mut self, a: u32) {
        let a = a & 0xFF;
        let r = a >> 1;
        self.m_isr = 0;
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if a & 1 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_lsr32(&mut self, a: u32) {
        let r = ((self.m_alue & 0xFFFF) << 15) | ((a & 0xFFFF) >> 1);
        self.m_isr = 0;
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if a & 1 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        self.m_aluo = r & 0xFFFF;
        self.m_alue = (r >> 16) & 0xFFFF;
    }

    fn alu_rol(&mut self, a: u32) {
        let a = a & 0xFFFF;
        let mut r = (a << 1) & 0xFFFF;
        self.m_isr = 0;
        if a & 0x8000 != 0 {
            self.m_isr |= SR_X | SR_C;
            r |= 1;
        }
        if r & 0x8000 != 0 {
            self.m_isr |= SR_N;
        }
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_rol8(&mut self, a: u32) {
        let a = a & 0xFF;
        let mut r = (a << 1) & 0xFF;
        self.m_isr = 0;
        if a & 0x80 != 0 {
            self.m_isr |= SR_X | SR_C;
            r |= 1;
        }
        if r & 0x80 != 0 {
            self.m_isr |= SR_N;
        }
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_rol32(&mut self, a: u32) {
        let high = self.m_alue & 0xFFFF;
        let mut r = (high << 17) | ((a & 0xFFFF) << 1);
        self.m_isr = 0;
        if high & 0x8000 != 0 {
            self.m_isr |= SR_X | SR_C;
            r |= 1;
        }
        if r & 0x8000_0000 != 0 {
            self.m_isr |= SR_N;
        }
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        self.m_aluo = r & 0xFFFF;
        self.m_alue = (r >> 16) & 0xFFFF;
    }

    fn alu_ror(&mut self, a: u32) {
        let a = a & 0xFFFF;
        let mut r = a >> 1;
        self.m_isr = 0;
        if a & 1 != 0 {
            self.m_isr |= SR_X | SR_C | SR_N;
            r |= 0x8000;
        }
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_ror8(&mut self, a: u32) {
        let a = a & 0xFF;
        let mut r = a >> 1;
        self.m_isr = 0;
        if a & 1 != 0 {
            self.m_isr |= SR_X | SR_C | SR_N;
            r |= 0x80;
        }
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_ror32(&mut self, a: u32) {
        let mut r = ((self.m_alue & 0xFFFF) << 15) | ((a & 0xFFFF) >> 1);
        self.m_isr = 0;
        if a & 1 != 0 {
            self.m_isr |= SR_X | SR_C | SR_N;
            r |= 0x8000_0000;
        }
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        self.m_aluo = r & 0xFFFF;
        self.m_alue = (r >> 16) & 0xFFFF;
    }

    fn alu_roxl(&mut self, a: u32) {
        let a = a & 0xFFFF;
        let r = ((a << 1) | u32::from(self.m_sr & SR_X != 0)) & 0xFFFF;
        self.m_isr = 0;
        if a & 0x8000 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if r & 0x8000 != 0 {
            self.m_isr |= SR_N;
        }
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_roxl8(&mut self, a: u32) {
        let a = a & 0xFF;
        let r = ((a << 1) | u32::from(self.m_sr & SR_X != 0)) & 0xFF;
        self.m_isr = 0;
        if a & 0x80 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if r & 0x80 != 0 {
            self.m_isr |= SR_N;
        }
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_roxl32(&mut self, a: u32) {
        let high = self.m_alue & 0xFFFF;
        let r = (high << 17) | ((a & 0xFFFF) << 1) | u32::from(self.m_sr & SR_X != 0);
        self.m_isr = 0;
        if high & 0x8000 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if r & 0x8000_0000 != 0 {
            self.m_isr |= SR_N;
        }
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        self.m_aluo = r & 0xFFFF;
        self.m_alue = (r >> 16) & 0xFFFF;
    }

    fn alu_roxr(&mut self, a: u32) {
        let a = a & 0xFFFF;
        let r = (a >> 1) | if self.m_sr & SR_X != 0 { 0x8000 } else { 0 };
        self.m_isr = 0;
        if a & 1 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if r & 0x8000 != 0 {
            self.m_isr |= SR_N;
        }
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_roxr8(&mut self, a: u32) {
        let a = a & 0xFF;
        let r = (a >> 1) | if self.m_sr & SR_X != 0 { 0x80 } else { 0 };
        self.m_isr = 0;
        if a & 1 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if r & 0x80 != 0 {
            self.m_isr |= SR_N;
        }
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        self.m_aluo = r & 0xFFFF;
    }

    fn alu_roxr32(&mut self, a: u32) {
        let a = a & 0xFFFF;
        let r = ((self.m_alue & 0xFFFF) << 15)
            | (a >> 1)
            | if self.m_sr & SR_X != 0 {
                0x8000_0000
            } else {
                0
            };
        self.m_isr = 0;
        if a & 1 != 0 {
            self.m_isr |= SR_X | SR_C;
        }
        if r & 0x8000_0000 != 0 {
            self.m_isr |= SR_N;
        }
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        self.m_aluo = r & 0xFFFF;
        self.m_alue = (r >> 16) & 0xFFFF;
    }

    fn alu_roxr32ms(&mut self, a: u32) {
        let carry_in =
            ((self.m_isr & (SR_N | SR_V)) == SR_N) || ((self.m_isr & (SR_N | SR_V)) == SR_V);
        let r = ((a & 0xFFFF) << 15)
            | ((self.m_alue & 0xFFFF) >> 1)
            | if carry_in { 0x8000_0000 } else { 0 };
        self.m_isr = 0;
        if a & 1 != 0 {
            self.m_isr |= SR_X;
        }
        if r & 0x8000_0000 != 0 {
            self.m_isr |= SR_N;
        }
        if r & 0xFFFF_0000 == 0 {
            self.m_isr |= SR_Z;
        }
        self.m_aluo = (r >> 16) & 0xFFFF;
        self.m_alue = r & 0xFFFF;
    }

    fn alu_roxr32mu(&mut self, a: u32) {
        let r = ((a & 0xFFFF) << 15)
            | ((self.m_alue & 0xFFFF) >> 1)
            | if self.m_isr & SR_C != 0 {
                0x8000_0000
            } else {
                0
            };
        self.m_isr = 0;
        if a & 1 != 0 {
            self.m_isr |= SR_X;
        }
        if r & 0x8000_0000 != 0 {
            self.m_isr |= SR_N;
        }
        if r & 0xFFFF_0000 == 0 {
            self.m_isr |= SR_Z;
        }
        self.m_aluo = (r >> 16) & 0xFFFF;
        self.m_alue = r & 0xFFFF;
    }

    fn finish_shift32(&mut self, r: u32, carry: bool, overflow: bool, preserve_v: bool) {
        self.m_isr = if preserve_v { self.m_sr & SR_V } else { 0 };
        if r == 0 {
            self.m_isr |= SR_Z;
        }
        if r & 0x8000_0000 != 0 {
            self.m_isr |= SR_N;
        }
        if carry {
            self.m_isr |= SR_X | SR_C;
        }
        if overflow {
            self.m_isr |= SR_V;
        }
        self.m_aluo = r & 0xFFFF;
        self.m_alue = (r >> 16) & 0xFFFF;
    }

    fn sr_z(&mut self) {
        self.m_sr = (self.m_sr & !SR_Z) | (self.m_isr & SR_Z);
    }

    fn sr_nz_u(&mut self) {
        self.m_sr = (self.m_sr & !SR_N & (self.m_isr | !SR_Z)) | (self.m_isr & SR_N);
    }

    fn sr_nzvc(&mut self) {
        self.m_sr =
            (self.m_sr & !(SR_N | SR_Z | SR_V | SR_C)) | (self.m_isr & (SR_N | SR_Z | SR_V | SR_C));
    }

    fn sr_nzvc_u(&mut self) {
        self.m_sr = (self.m_sr & !(SR_N | SR_V | SR_C) & (self.m_isr | !SR_Z))
            | (self.m_isr & (SR_N | SR_V | SR_C));
    }

    fn sr_xnzvc(&mut self) {
        self.m_sr = (self.m_sr & !(SR_X | SR_N | SR_Z | SR_V | SR_C))
            | (self.m_isr & (SR_X | SR_N | SR_Z | SR_V | SR_C));
    }

    fn sr_xnzvc_u(&mut self) {
        self.m_sr = (self.m_sr & !(SR_X | SR_N | SR_V | SR_C) & (self.m_isr | !SR_Z))
            | (self.m_isr & (SR_X | SR_N | SR_V | SR_C));
    }
}

impl CpuM68000 for M68000 {
    fn run_for(&mut self, cycles_to_run: u64, bus: &mut impl Bus) -> u64 {
        let start_cycle = bus.current_cycle();
        let mut consumed = 0u64;
        while consumed < cycles_to_run {
            let cycles = self.run_one_instruction(bus);
            if cycles == 0 {
                break;
            }
            consumed = consumed.saturating_add(cycles);
            consumed = consumed.saturating_add(bus.drain_wait_cycles().max(0) as u64);
            bus.set_current_cycle(start_cycle + consumed);
            if bus.reset_pending() || bus.cpu_should_yield() {
                break;
            }
        }
        bus.set_current_cycle(start_cycle + consumed);
        consumed
    }

    fn step(&mut self, bus: &mut impl Bus) -> u64 {
        let start_cycle = bus.current_cycle();
        let cycles = self.run_one_instruction(bus);
        bus.set_current_cycle(start_cycle + cycles);
        cycles
    }

    fn reset(&mut self) {
        self.m_inst_state = S_RESET;
        self.m_inst_substate = 0;
        self.m_count_before_instruction_step = 0;
        self.m_post_run = PR_NONE;
        self.m_post_run_cycles = 0;
        self.stopped = false;
        self.update_user_super();
    }

    fn halted(&self) -> bool {
        self.stopped
    }

    fn clock_hz(&self) -> u32 {
        self.clock_hz
    }

    fn set_clock_hz(&mut self, clock_hz: u32) {
        self.clock_hz = clock_hz;
    }

    fn pc(&self) -> u32 {
        self.m_ipc & ADDRESS_MASK
    }

    fn set_pc(&mut self, value: u32) {
        self.m_ipc = value & ADDRESS_MASK;
    }

    fn d(&self, index: usize) -> u32 {
        self.m_da[index]
    }

    fn set_d(&mut self, index: usize, value: u32) {
        self.m_da[index] = value;
    }

    fn a(&self, index: usize) -> u32 {
        if index == 7 {
            self.m_da[self.m_sp]
        } else {
            self.m_da[8 + index]
        }
    }

    fn set_a(&mut self, index: usize, value: u32) {
        if index == 7 {
            self.m_da[self.m_sp] = value;
        } else {
            self.m_da[8 + index] = value;
        }
    }

    fn usp(&self) -> u32 {
        self.m_da[15]
    }

    fn set_usp(&mut self, value: u32) {
        self.m_da[15] = value;
    }

    fn ssp(&self) -> u32 {
        self.m_da[16]
    }

    fn set_ssp(&mut self, value: u32) {
        self.m_da[16] = value;
    }

    fn sr(&self) -> u16 {
        self.m_sr as u16
    }

    fn set_sr(&mut self, value: u16) {
        self.m_sr = u32::from(value) & (SR_SR | SR_CCR);
        self.update_user_super();
        self.update_interrupt();
    }
}

#[inline]
fn merge_16_32(high: u32, low: u32) -> u32 {
    ((high & 0xFFFF) << 16) | (low & 0xFFFF)
}

#[inline]
fn high16(value: u32) -> u32 {
    (value >> 16) & 0xFFFF
}

#[inline]
fn ext32(value: u32) -> u32 {
    s16(value)
}

#[inline]
fn set_16h(register: &mut u32, value: u32) {
    *register = (*register & 0x0000_FFFF) | ((value & 0xFFFF) << 16);
}

#[inline]
fn set_16l(register: &mut u32, value: u32) {
    *register = (*register & 0xFFFF_0000) | (value & 0xFFFF);
}

#[inline]
fn set_8(register: &mut u32, value: u32) {
    *register = (*register & 0xFFFF_FF00) | (value & 0xFF);
}

#[inline]
fn set_8h(register: &mut u32, value: u32) {
    *register = (*register & 0x00FF) | ((value & 0xFF) << 8);
}

#[inline]
fn set_8xl(register: &mut u32, value: u32) {
    *register = (value & 0x00FF) | ((value & 0x00FF) << 8);
}

#[inline]
fn set_8xh(register: &mut u32, value: u32) {
    *register = (value & 0xFF00) | ((value & 0xFF00) >> 8);
}

#[inline]
fn s8(value: u32) -> u32 {
    (value as u8 as i8 as i32) as u32
}

#[inline]
fn s16(value: u32) -> u32 {
    (value as u16 as i16 as i32) as u32
}

include!("m68000_generated.rs");
