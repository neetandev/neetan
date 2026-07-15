#![cfg(feature = "verification")]

//! Differential V20 vs V30 timing tests.
//!
//! `cpu.cycles_consumed()` reports full T-state count between two
//! instruction-fetch boundaries. The NEC V20/V30 datasheet's Table 2-8 is EU
//! cost only, which is not directly observable. So instead of comparing to
//! an absolute datasheet number, these tests compare V20 against V30 for
//! the same instruction, asserting the **cycle saving** matches the V30's
//! 16-bit-bus advantages.
//!
//! V30 saves cycles in two independent places:
//!
//! 1. Instruction stream fetch: V20 fetches 1 byte per m-cycle (4
//!    clocks). V30 fetches 1 word per m-cycle (also 4 clocks) when PC is
//!    even-aligned, falling back to 1 byte after a branch to an odd target.
//!    For an N-byte instruction starting at an even PC, V30 needs
//!    `ceil(N/2)` m-cycles vs V20's `N` m-cycles - saving
//!    `floor(N/2) * 4` clocks.
//!
//! 2. Word memory operand: A word access at an even EA costs 1 m-cycle
//!    on V30 (saving 4 clocks vs V20) but still costs 2 m-cycles at odd EA.
//!    Byte accesses cost 1 m-cycle on both - no operand saving.
//!
//! The expected V30 saving for an instruction is the sum of these two
//! components. When the V30 BIU is correct, these deltas are exact
//! integers; when it drifts the deltas diverge and the tests pinpoint the
//! failing instruction class.

use cpu::{V20, V30, V30State};

const RAM_SIZE: usize = 1024 * 1024;
const ADDRESS_MASK: u32 = 0x000F_FFFF;

struct TestBus {
    ram: Vec<u8>,
    current_cycle: u64,
}

impl TestBus {
    fn new() -> Self {
        Self {
            ram: vec![0u8; RAM_SIZE],
            current_cycle: 0,
        }
    }

    fn write_u8(&mut self, address: u32, value: u8) {
        self.ram[(address & ADDRESS_MASK) as usize] = value;
    }

    fn write_bytes(&mut self, address: u32, bytes: &[u8]) {
        for (i, &b) in bytes.iter().enumerate() {
            self.write_u8(address.wrapping_add(i as u32), b);
        }
    }
}

impl common::Bus for TestBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        self.ram[(address & ADDRESS_MASK) as usize]
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.write_u8(address, value);
    }

    fn io_read_byte(&mut self, _port: u16) -> u8 {
        0xFF
    }

    fn io_write_byte(&mut self, _port: u16, _value: u8) {}

    fn has_irq(&self) -> bool {
        false
    }

    fn acknowledge_irq(&mut self) -> u8 {
        0
    }

    fn has_nmi(&self) -> bool {
        false
    }

    fn acknowledge_nmi(&mut self) {}

    fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;
    }
}

#[derive(Clone)]
struct Scenario {
    cs: u16,
    ip: u16,
    ds: u16,
    es: u16,
    ss: u16,
    sp: u16,
    ax: u16,
    bx: u16,
    cx: u16,
    dx: u16,
    si: u16,
    di: u16,
    bp: u16,
    flags: u16,
    code: Vec<u8>,
    /// Pre-loaded RAM (physical-address byte writes performed before run).
    ram: Vec<(u32, u8)>,
}

impl Scenario {
    fn new() -> Self {
        Self {
            cs: 0x1000,
            ip: 0x0000,
            ds: 0x2000,
            es: 0x3000,
            ss: 0x4000,
            sp: 0x0100,
            ax: 0,
            bx: 0,
            cx: 0,
            dx: 0,
            si: 0,
            di: 0,
            bp: 0,
            flags: 0x8202, // IF=1, MD=1
            code: Vec::new(),
            ram: Vec::new(),
        }
    }

    fn ip(mut self, ip: u16) -> Self {
        self.ip = ip;
        self
    }
    fn sp(mut self, sp: u16) -> Self {
        self.sp = sp;
        self
    }
    fn code(mut self, bytes: &[u8]) -> Self {
        self.code = bytes.to_vec();
        self
    }
    fn ram_byte(mut self, address: u32, value: u8) -> Self {
        self.ram.push((address, value));
        self
    }
    fn ram_word(mut self, address: u32, value: u16) -> Self {
        self.ram.push((address, value as u8));
        self.ram.push((address.wrapping_add(1), (value >> 8) as u8));
        self
    }
    fn ax(mut self, ax: u16) -> Self {
        self.ax = ax;
        self
    }
    fn bx(mut self, bx: u16) -> Self {
        self.bx = bx;
        self
    }
    fn cx(mut self, cx: u16) -> Self {
        self.cx = cx;
        self
    }
    fn si(mut self, si: u16) -> Self {
        self.si = si;
        self
    }
    fn di(mut self, di: u16) -> Self {
        self.di = di;
        self
    }
}

fn build_bus(scenario: &Scenario) -> TestBus {
    let mut bus = TestBus::new();
    let cs_base = (scenario.cs as u32) << 4;
    bus.write_bytes(cs_base.wrapping_add(scenario.ip as u32), &scenario.code);
    for &(address, value) in &scenario.ram {
        bus.write_u8(address, value);
    }
    bus
}

fn build_state(scenario: &Scenario) -> V30State {
    let mut state = V30State::default();
    state.set_ax(scenario.ax);
    state.set_bx(scenario.bx);
    state.set_cx(scenario.cx);
    state.set_dx(scenario.dx);
    state.set_si(scenario.si);
    state.set_di(scenario.di);
    state.set_bp(scenario.bp);
    state.set_sp(scenario.sp);
    state.set_cs(scenario.cs);
    state.set_ds(scenario.ds);
    state.set_es(scenario.es);
    state.set_ss(scenario.ss);
    state.ip = scenario.ip;
    state.set_compressed_flags(scenario.flags);
    state.initialize_cold_frontend();
    state
}

/// Run `cpu.step()` once on a V20 and on a V30, both built from the same
/// scenario. Returns `(v20_cycles, v30_cycles)`.
fn run_both(scenario: &Scenario) -> (u64, u64) {
    let state = build_state(scenario);

    let mut v20_bus = build_bus(scenario);
    let mut v20: V20 = V20::new();
    v20.load_state(&state);
    v20.step(&mut v20_bus);
    let v20_cycles = v20.cycles_consumed();

    let mut v30_bus = build_bus(scenario);
    let mut v30: V30 = V30::new();
    v30.load_state(&state);
    v30.step(&mut v30_bus);
    let v30_cycles = v30.cycles_consumed();

    (v20_cycles, v30_cycles)
}

/// Asserts that the V30 cycle count is V20 - `expected_saving`. Negative
/// values mean V30 is *slower* than V20 (a bug for word-mem instructions).
fn assert_delta(scenario: &Scenario, expected_saving: i64, label: &str) {
    let (v20_cycles, v30_cycles) = run_both(scenario);
    let actual_saving = v20_cycles as i64 - v30_cycles as i64;
    assert_eq!(
        actual_saving, expected_saving,
        "{label}: V20={v20_cycles}, V30={v30_cycles}, expected V30 saving = {expected_saving}, got {actual_saving}"
    );
}

mod data_transfer {
    use super::*;

    /// `MOV AL, [imm16]` is a 4-byte instruction at even PC with byte mem.
    /// Saving = floor(4/2)*4 (instr fetch) + 0 (byte operand) = 8.
    #[test]
    fn mov_reg_mem_byte_saves_8() {
        // MOV AL, [DS:0x0010]   8A 06 10 00
        let scenario = Scenario::new()
            .code(&[0x8A, 0x06, 0x10, 0x00])
            .ram_byte(0x20010, 0x42);
        assert_delta(&scenario, 8, "MOV AL, [0x0010] byte");
    }

    /// `MOV [imm16], AL` is also 4 bytes at even PC.
    /// Saving = 8 (instr) + 0 (byte operand) = 8.
    #[test]
    fn mov_mem_reg_byte_saves_8() {
        // MOV [DS:0x0010], AL   88 06 10 00
        let scenario = Scenario::new().ax(0x4242).code(&[0x88, 0x06, 0x10, 0x00]);
        assert_delta(&scenario, 8, "MOV [0x0010], AL byte");
    }

    /// `MOV AX, [imm16]` 4 bytes, word-even operand.
    /// Saving = 8 (instr) + 4 (word-even operand) = 12.
    #[test]
    fn mov_reg_mem_word_even_saves_12() {
        // MOV AX, [DS:0x0010]   8B 06 10 00 - EA=0x20010 (even)
        let scenario = Scenario::new()
            .code(&[0x8B, 0x06, 0x10, 0x00])
            .ram_word(0x20010, 0xCAFE);
        assert_delta(&scenario, 12, "MOV AX, [0x0010] (word even)");
    }

    /// `MOV [imm16], AX` 4 bytes, word-even operand.
    /// Saving = 8 (instr) + 4 (word-even operand) = 12.
    #[test]
    fn mov_mem_reg_word_even_saves_12() {
        // MOV [DS:0x0010], AX   89 06 10 00
        let scenario = Scenario::new().ax(0xCAFE).code(&[0x89, 0x06, 0x10, 0x00]);
        assert_delta(&scenario, 12, "MOV [0x0010], AX (word even)");
    }

    /// `MOV AX, [imm16]` 4 bytes, word-odd operand.
    /// Saving = 8 (instr) + 0 (odd operand still 2 m-cycles) = 8.
    #[test]
    fn mov_reg_mem_word_odd_saves_8() {
        // MOV AX, [DS:0x0011]   8B 06 11 00 - EA=0x20011 (odd)
        let scenario = Scenario::new()
            .code(&[0x8B, 0x06, 0x11, 0x00])
            .ram_word(0x20011, 0xCAFE);
        assert_delta(&scenario, 8, "MOV AX, [0x0011] (word odd)");
    }

    /// `MOV [imm16], AX` 4 bytes, word-odd operand.
    /// Saving = 8 (instr) + 0 = 8.
    #[test]
    fn mov_mem_reg_word_odd_saves_8() {
        // MOV [DS:0x0011], AX   89 06 11 00
        let scenario = Scenario::new().ax(0xCAFE).code(&[0x89, 0x06, 0x11, 0x00]);
        assert_delta(&scenario, 8, "MOV [0x0011], AX (word odd)");
    }
}

mod stack {
    use super::*;

    /// PUSH AX is a 1-byte instruction. Instruction-fetch saving = 0.
    /// Even SP-2 word write saves 4. Total = 4.
    #[test]
    fn push_reg_even_sp_saves_4() {
        // PUSH AX with SP=0x0100, SP-2 = 0x00FE (even).
        let scenario = Scenario::new().sp(0x0100).ax(0xBEEF).code(&[0x50]);
        assert_delta(&scenario, 4, "PUSH AX (even SP-2)");
    }

    /// PUSH AX with odd SP-2: word write at odd - no operand saving.
    /// Instruction is 1 byte = no instr saving. Total = 0.
    #[test]
    fn push_reg_odd_sp_no_delta() {
        // PUSH AX with SP=0x0101, SP-2 = 0x00FF (odd).
        let scenario = Scenario::new().sp(0x0101).ax(0xBEEF).code(&[0x50]);
        assert_delta(&scenario, 0, "PUSH AX (odd SP-2)");
    }

    /// POP AX 1-byte instruction; even SP word read saves 4.
    #[test]
    fn pop_reg_even_sp_saves_4() {
        let scenario = Scenario::new()
            .sp(0x00FE)
            .code(&[0x58])
            .ram_word(0x400FE, 0xBEEF);
        assert_delta(&scenario, 4, "POP AX (even SP)");
    }

    /// POP AX 1-byte instruction; odd SP word read pays the V20 penalty.
    #[test]
    fn pop_reg_odd_sp_no_delta() {
        let scenario = Scenario::new()
            .sp(0x00FF)
            .code(&[0x58])
            .ram_word(0x400FF, 0xBEEF);
        assert_delta(&scenario, 0, "POP AX (odd SP)");
    }
}

mod control_flow {
    use super::*;

    /// CALL near is 3 bytes at even PC. Instr saving = floor(3/2)*4 = 4.
    /// Even SP-2 word push saves 4. Total = 8.
    #[test]
    fn call_near_even_sp_saves_8() {
        // CALL +0   E8 00 00; SP=0x0100, SP-2 = 0x00FE (even).
        let scenario = Scenario::new().sp(0x0100).code(&[0xE8, 0x00, 0x00]);
        assert_delta(&scenario, 8, "CALL near (even SP-2)");
    }

    /// CALL near at even PC, odd SP-2: instr saving = 4, operand 0. Total = 4.
    #[test]
    fn call_near_odd_sp_saves_4() {
        let scenario = Scenario::new().sp(0x0101).code(&[0xE8, 0x00, 0x00]);
        assert_delta(&scenario, 4, "CALL near (odd SP-2)");
    }

    /// RET near is 1 byte. Instr saving = 0. Even SP word read = 4.
    #[test]
    fn ret_near_even_sp_saves_4() {
        let scenario = Scenario::new()
            .sp(0x00FE)
            .code(&[0xC3])
            .ram_word(0x400FE, 0x0010);
        assert_delta(&scenario, 4, "RET near (even SP)");
    }

    /// RET near 1 byte, odd SP: no saving on either side.
    #[test]
    fn ret_near_odd_sp_no_delta() {
        let scenario = Scenario::new()
            .sp(0x00FF)
            .code(&[0xC3])
            .ram_word(0x400FF, 0x0010);
        assert_delta(&scenario, 0, "RET near (odd SP)");
    }
}

mod alignment_stress {
    use super::*;

    /// MOVSW src and dst both even: 1 word read + 1 word write saved = 8.
    #[test]
    fn movsw_even_even_saves_8() {
        let scenario = Scenario::new()
            .si(0x0010)
            .di(0x0010)
            .code(&[0xA5])
            .ram_word(0x20010, 0xCAFE);
        assert_delta(&scenario, 8, "MOVSW even/even");
    }

    /// MOVSW src odd, dst even: only the dst write saves 4.
    #[test]
    fn movsw_odd_even_saves_4() {
        let scenario = Scenario::new()
            .si(0x0011)
            .di(0x0010)
            .code(&[0xA5])
            .ram_word(0x20011, 0xCAFE);
        assert_delta(&scenario, 4, "MOVSW odd/even");
    }

    /// MOVSW src even, dst odd: only the src read saves 4.
    #[test]
    fn movsw_even_odd_saves_4() {
        let scenario = Scenario::new()
            .si(0x0010)
            .di(0x0011)
            .code(&[0xA5])
            .ram_word(0x20010, 0xCAFE);
        assert_delta(&scenario, 4, "MOVSW even/odd");
    }

    /// MOVSW both odd: no savings.
    #[test]
    fn movsw_odd_odd_no_delta() {
        let scenario = Scenario::new()
            .si(0x0011)
            .di(0x0011)
            .code(&[0xA5])
            .ram_word(0x20011, 0xCAFE);
        assert_delta(&scenario, 0, "MOVSW odd/odd");
    }

    /// Instruction at odd PC: V30 BIU should issue a single-byte first fetch
    /// at the odd target then realign. Smoke test - both modes must complete.
    #[test]
    fn instruction_at_odd_pc_runs() {
        // MOV AX, BX   8B C3   placed at IP=0x0001 (odd).
        let scenario = Scenario::new().ip(0x0001).bx(0x1234).code(&[0x8B, 0xC3]);
        let _ = run_both(&scenario);
    }

    /// Conditional loop branching to an odd target. This is the LOOPNZ/LOOPZ
    /// case that exposed the Phase 2 BIU debug_assert.
    #[test]
    fn loopne_taken_odd_target_runs() {
        // LOOPNE -2  E0 FE  at IP=0x0001 (odd). CX=3, ZF=0.
        let scenario = Scenario::new().ip(0x0001).cx(3).code(&[0xE0, 0xFE]);
        let _ = run_both(&scenario);
    }
}
