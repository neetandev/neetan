use common::Bus as _;
use cpu::{CPU_MODEL_486_DX, I386, I386State};
use softfloat::{Fp80, FpClass};

const RAM_SIZE: usize = 1024 * 1024;
const ADDRESS_MASK: u32 = 0x000F_FFFF;

struct TestBus {
    ram: Vec<u8>,
    irq_pending: bool,
    irq_vector: u8,
}

impl TestBus {
    fn new() -> Self {
        Self {
            ram: vec![0u8; RAM_SIZE],
            irq_pending: false,
            irq_vector: 0,
        }
    }
}

impl common::Bus for TestBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        self.ram[(address & ADDRESS_MASK) as usize]
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.ram[(address & ADDRESS_MASK) as usize] = value;
    }

    fn io_read_byte(&mut self, _port: u16) -> u8 {
        0xFF
    }

    fn io_write_byte(&mut self, _port: u16, _value: u8) {}

    fn has_irq(&self) -> bool {
        self.irq_pending
    }

    fn acknowledge_irq(&mut self) -> u8 {
        self.irq_pending = false;
        self.irq_vector
    }

    fn has_nmi(&self) -> bool {
        false
    }

    fn acknowledge_nmi(&mut self) {}

    fn current_cycle(&self) -> u64 {
        0
    }

    fn set_current_cycle(&mut self, _cycle: u64) {}
}

struct X87Result {
    registers: [Fp80; 8],
    status_word: u16,
    tag_word: u16,
    top: u8,
}

impl X87Result {
    fn st(&self, i: u8) -> Fp80 {
        self.registers[((self.top.wrapping_add(i)) & 7) as usize]
    }

    fn c0(&self) -> bool {
        self.status_word & 0x0100 != 0
    }

    fn c1(&self) -> bool {
        self.status_word & 0x0200 != 0
    }

    fn c2(&self) -> bool {
        self.status_word & 0x0400 != 0
    }

    fn c3(&self) -> bool {
        self.status_word & 0x4000 != 0
    }

    fn ie(&self) -> bool {
        self.status_word & 0x01 != 0
    }

    fn de(&self) -> bool {
        self.status_word & 0x02 != 0
    }

    fn ze(&self) -> bool {
        self.status_word & 0x04 != 0
    }

    fn oe(&self) -> bool {
        self.status_word & 0x08 != 0
    }

    fn ue(&self) -> bool {
        self.status_word & 0x10 != 0
    }

    fn pe(&self) -> bool {
        self.status_word & 0x20 != 0
    }

    fn no_exceptions(&self) -> bool {
        self.status_word & 0x3F == 0
    }

    fn tag(&self, i: u8) -> u16 {
        let phys = ((self.top.wrapping_add(i)) & 7) as usize;
        (self.tag_word >> (phys * 2)) & 3
    }
}

fn tag_from_fp80(val: &Fp80) -> u16 {
    match val.classify() {
        FpClass::Zero => 0b01,
        FpClass::Normal => 0b00,
        _ => 0b10,
    }
}

fn place_code(bus: &mut TestBus, cs: u16, ip: u16, code: &[u8]) {
    let base = (cs as u32) << 4;
    for (i, &byte) in code.iter().enumerate() {
        bus.write_byte(base + ip as u32 + i as u32, byte);
    }
}

fn build_state(stack: &[Fp80], cw: u16) -> I386State {
    let mut state = I386State::default();
    state.set_cs(0x1000);
    state.set_eip(0x0000);
    state.set_ds(0x2000);
    state.set_ss(0x3000);
    state.set_esp(0x1000);

    let n = stack.len();
    let top = ((8 - n) & 7) as u8;
    state.fpu.control_word = cw;
    state.fpu.status_word = (top as u16) << 11;
    state.fpu.tag_word = 0xFFFF; // all empty

    for (i, val) in stack.iter().enumerate() {
        let phys = ((top.wrapping_add(i as u8)) & 7) as usize;
        state.fpu.registers[phys] = *val;
        let tag = tag_from_fp80(val);
        state.fpu.tag_word &= !(3 << (phys * 2));
        state.fpu.tag_word |= tag << (phys * 2);
    }

    state
}

fn extract_result(cpu: &I386<{ CPU_MODEL_486_DX }>) -> X87Result {
    let fpu = &cpu.state.fpu;
    X87Result {
        registers: fpu.registers,
        status_word: fpu.status_word,
        tag_word: fpu.tag_word,
        top: ((fpu.status_word >> 11) & 7) as u8,
    }
}

fn run_x87(code: &[u8], stack: &[Fp80]) -> X87Result {
    run_x87_with_cw(code, stack, CW_NEAREST_EXT)
}

fn run_x87_with_cw(code: &[u8], stack: &[Fp80], cw: u16) -> X87Result {
    let mut cpu = I386::<{ CPU_MODEL_486_DX }>::new();
    let mut bus = TestBus::new();
    place_code(&mut bus, 0x1000, 0x0000, code);
    let state = build_state(stack, cw);
    cpu.load_state(&state);
    cpu.step(&mut bus);
    extract_result(&cpu)
}

fn run_x87_mem(code: &[u8], stack: &[Fp80], cw: u16, mem: &[u8]) -> (X87Result, TestBus) {
    let mut cpu = I386::<{ CPU_MODEL_486_DX }>::new();
    let mut bus = TestBus::new();
    place_code(&mut bus, 0x1000, 0x0000, code);

    // Write memory data at DS:0x0000 = linear 0x20000
    for (i, &byte) in mem.iter().enumerate() {
        bus.ram[0x20000 + i] = byte;
    }

    let state = build_state(stack, cw);
    cpu.load_state(&state);
    cpu.step(&mut bus);
    (extract_result(&cpu), bus)
}

fn read_mem_i16(bus: &TestBus, offset: usize) -> i16 {
    let lo = bus.ram[0x20000 + offset] as u16;
    let hi = bus.ram[0x20000 + offset + 1] as u16;
    (lo | (hi << 8)) as i16
}

fn read_mem_i32(bus: &TestBus, offset: usize) -> i32 {
    let b0 = bus.ram[0x20000 + offset] as u32;
    let b1 = bus.ram[0x20000 + offset + 1] as u32;
    let b2 = bus.ram[0x20000 + offset + 2] as u32;
    let b3 = bus.ram[0x20000 + offset + 3] as u32;
    (b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)) as i32
}

fn read_mem_i64(bus: &TestBus, offset: usize) -> i64 {
    let mut val = 0u64;
    for i in 0..8 {
        val |= (bus.ram[0x20000 + offset + i] as u64) << (i * 8);
    }
    val as i64
}

fn read_mem_u32(bus: &TestBus, offset: usize) -> u32 {
    read_mem_i32(bus, offset) as u32
}

fn read_mem_u64(bus: &TestBus, offset: usize) -> u64 {
    read_mem_i64(bus, offset) as u64
}

fn read_mem_tbyte(bus: &TestBus, offset: usize) -> [u8; 10] {
    let mut bytes = [0u8; 10];
    bytes.copy_from_slice(&bus.ram[0x20000 + offset..0x20000 + offset + 10]);
    bytes
}

const CW_NEAREST_EXT: u16 = 0x037F; // RC=00, PC=11, all exceptions masked
const CW_DOWN_EXT: u16 = 0x077F; // RC=01, PC=11
const CW_UP_EXT: u16 = 0x0B7F; // RC=10, PC=11
const CW_ZERO_EXT: u16 = 0x0F7F; // RC=11, PC=11
const CW_NEAREST_SGL: u16 = 0x007F; // RC=00, PC=00
const CW_NEAREST_DBL: u16 = 0x027F; // RC=00, PC=10

const NEGATIVE_ONE: Fp80 = Fp80::from_bits(0xBFFF, 0x8000_0000_0000_0000);
const TWO: Fp80 = Fp80::from_bits(0x4000, 0x8000_0000_0000_0000);
const THREE: Fp80 = Fp80::from_bits(0x4000, 0xC000_0000_0000_0000);
const FOUR: Fp80 = Fp80::from_bits(0x4001, 0x8000_0000_0000_0000);
const FIVE: Fp80 = Fp80::from_bits(0x4001, 0xA000_0000_0000_0000);
const SIX: Fp80 = Fp80::from_bits(0x4001, 0xC000_0000_0000_0000);
const SEVEN: Fp80 = Fp80::from_bits(0x4001, 0xE000_0000_0000_0000);
const HALF: Fp80 = Fp80::from_bits(0x3FFE, 0x8000_0000_0000_0000);
const NEGATIVE_HALF: Fp80 = Fp80::from_bits(0xBFFE, 0x8000_0000_0000_0000);
const ONE_AND_HALF: Fp80 = Fp80::from_bits(0x3FFF, 0xC000_0000_0000_0000);
const NEGATIVE_ONE_AND_HALF: Fp80 = Fp80::from_bits(0xBFFF, 0xC000_0000_0000_0000);
const TWO_AND_HALF: Fp80 = Fp80::from_bits(0x4000, 0xA000_0000_0000_0000);
const NEGATIVE_TWO: Fp80 = Fp80::from_bits(0xC000, 0x8000_0000_0000_0000);
const NEGATIVE_THREE: Fp80 = Fp80::from_bits(0xC000, 0xC000_0000_0000_0000);
const NEGATIVE_FOUR: Fp80 = Fp80::from_bits(0xC001, 0x8000_0000_0000_0000);
const NEGATIVE_SEVEN: Fp80 = Fp80::from_bits(0xC001, 0xE000_0000_0000_0000);
const TWO_POW_100: Fp80 = Fp80::from_bits(0x4063, 0x8000_0000_0000_0000);

const POSITIVE_SNAN: Fp80 = Fp80::from_bits(0x7FFF, 0x8000_0000_0000_0001);
const NEGATIVE_SNAN: Fp80 = Fp80::from_bits(0xFFFF, 0x8000_0000_0000_0001);
const POSITIVE_QNAN: Fp80 = Fp80::from_bits(0x7FFF, 0xC000_0000_0000_0001);
const NEGATIVE_QNAN: Fp80 = Fp80::from_bits(0xFFFF, 0xC000_0000_0000_0001);

const POSITIVE_DENORMAL: Fp80 = Fp80::from_bits(0x0000, 0x0000_0000_0000_0001);
const NEGATIVE_DENORMAL: Fp80 = Fp80::from_bits(0x8000, 0x0000_0000_0000_0001);
const MAX_DENORMAL: Fp80 = Fp80::from_bits(0x0000, 0x3FFF_FFFF_FFFF_FFFF);
const POSITIVE_PSEUDO_DENORMAL: Fp80 = Fp80::from_bits(0x0000, 0x8000_0000_0000_0000);
const NEGATIVE_PSEUDO_DENORMAL: Fp80 = Fp80::from_bits(0x8000, 0x8000_0000_0000_0000);
const SMALLEST_NORMAL: Fp80 = Fp80::from_bits(0x0001, 0x8000_0000_0000_0000);
const LARGEST_NORMAL: Fp80 = Fp80::from_bits(0x7FFE, 0xFFFF_FFFF_FFFF_FFFF);
const UNNORMAL: Fp80 = Fp80::from_bits(0x0001, 0x0000_0000_0000_0001);
const UNNORMAL_MAX_EXP: Fp80 = Fp80::from_bits(0x7FFE, 0x0000_0000_0000_0001);
const PSEUDO_INFINITY: Fp80 = Fp80::from_bits(0x7FFF, 0x0000_0000_0000_0000);
const NEGATIVE_PSEUDO_INFINITY: Fp80 = Fp80::from_bits(0xFFFF, 0x0000_0000_0000_0000);
const PSEUDO_NAN: Fp80 = Fp80::from_bits(0x7FFF, 0x0000_0000_0000_0001);
const MAX_SNAN: Fp80 = Fp80::from_bits(0x7FFF, 0xBFFF_FFFF_FFFF_FFFF);
const TINY_NORMAL: Fp80 = Fp80::from_bits(0x0001, 0x8000_0000_0000_0000);

// |x| >= 2^63: smallest out-of-range value for trig functions
const OUT_OF_RANGE: Fp80 = Fp80::from_bits(0x403E, 0x8000_0000_0000_0000);

#[test]
fn fld1_pushes_one() {
    let r = run_x87(&[0xD9, 0xE8], &[]);
    assert_eq!(r.st(0), Fp80::ONE);
    assert!(r.no_exceptions());
}

#[test]
fn fldz_pushes_zero() {
    let r = run_x87(&[0xD9, 0xEE], &[]);
    assert_eq!(r.st(0), Fp80::ZERO);
    assert!(r.no_exceptions());
}

#[test]
fn fldl2t_pushes_log2_10() {
    // Default CW (nearest even): should push LOG2_10_DOWN per fpu_ops logic
    // (match arm: Up => UP, _ => DOWN)
    let r = run_x87(&[0xD9, 0xE9], &[]);
    assert_eq!(r.st(0), Fp80::LOG2_10_DOWN);

    // Round up: should push LOG2_10_UP
    let r = run_x87_with_cw(&[0xD9, 0xE9], &[], CW_UP_EXT);
    assert_eq!(r.st(0), Fp80::LOG2_10_UP);
}

#[test]
fn fldl2e_pushes_log2_e() {
    // Default CW (nearest even): should push LOG2_E_UP per fpu_ops logic
    // (match arm: Up | NearestEven => UP, _ => DOWN)
    let r = run_x87(&[0xD9, 0xEA], &[]);
    assert_eq!(r.st(0), Fp80::LOG2_E_UP);

    // Round down: should push LOG2_E_DOWN
    let r = run_x87_with_cw(&[0xD9, 0xEA], &[], CW_DOWN_EXT);
    assert_eq!(r.st(0), Fp80::LOG2_E_DOWN);
}

#[test]
fn fldpi_pushes_pi() {
    // Default CW (nearest even): PI_UP
    let r = run_x87(&[0xD9, 0xEB], &[]);
    assert_eq!(r.st(0), Fp80::PI_UP);

    // Round down: PI_DOWN
    let r = run_x87_with_cw(&[0xD9, 0xEB], &[], CW_DOWN_EXT);
    assert_eq!(r.st(0), Fp80::PI_DOWN);
}

#[test]
fn fldlg2_pushes_log10_2() {
    // Default CW (nearest even): LOG10_2_UP
    let r = run_x87(&[0xD9, 0xEC], &[]);
    assert_eq!(r.st(0), Fp80::LOG10_2_UP);

    // Round zero: LOG10_2_DOWN
    let r = run_x87_with_cw(&[0xD9, 0xEC], &[], CW_ZERO_EXT);
    assert_eq!(r.st(0), Fp80::LOG10_2_DOWN);
}

#[test]
fn fldln2_pushes_ln_2() {
    // Default CW (nearest even): LN_2_UP
    let r = run_x87(&[0xD9, 0xED], &[]);
    assert_eq!(r.st(0), Fp80::LN_2_UP);

    // Round down: LN_2_DOWN
    let r = run_x87_with_cw(&[0xD9, 0xED], &[], CW_DOWN_EXT);
    assert_eq!(r.st(0), Fp80::LN_2_DOWN);
}

fn run_fxam(val: Fp80) -> X87Result {
    run_x87(&[0xD9, 0xE5], &[val])
}

fn run_fxam_empty() -> X87Result {
    // No stack entries: TOP=0, all tags=empty
    run_x87(&[0xD9, 0xE5], &[])
}

#[test]
fn fxam_positive_zero() {
    let r = run_fxam(Fp80::ZERO);
    assert!(r.c3()); // Zero: C3=1
    assert!(!r.c2());
    assert!(!r.c1()); // positive
    assert!(!r.c0());
}

#[test]
fn fxam_negative_zero() {
    let r = run_fxam(Fp80::NEG_ZERO);
    assert!(r.c3());
    assert!(!r.c2());
    assert!(r.c1()); // negative
    assert!(!r.c0());
}

#[test]
fn fxam_positive_normal() {
    let r = run_fxam(Fp80::ONE);
    assert!(!r.c3()); // Normal: C3=0, C2=1, C0=0
    assert!(r.c2());
    assert!(!r.c1()); // positive
    assert!(!r.c0());
}

#[test]
fn fxam_negative_normal() {
    let r = run_fxam(NEGATIVE_ONE);
    assert!(!r.c3());
    assert!(r.c2());
    assert!(r.c1()); // negative
    assert!(!r.c0());
}

#[test]
fn fxam_positive_infinity() {
    let r = run_fxam(Fp80::INFINITY);
    assert!(!r.c3()); // Infinity: C3=0, C2=1, C0=1
    assert!(r.c2());
    assert!(!r.c1());
    assert!(r.c0());
}

#[test]
fn fxam_negative_infinity() {
    let r = run_fxam(Fp80::NEG_INFINITY);
    assert!(!r.c3());
    assert!(r.c2());
    assert!(r.c1()); // negative
    assert!(r.c0());
}

#[test]
fn fxam_positive_nan() {
    let r = run_fxam(POSITIVE_QNAN);
    assert!(!r.c3()); // NaN: C3=0, C2=0, C0=1
    assert!(!r.c2());
    assert!(!r.c1());
    assert!(r.c0());
}

#[test]
fn fxam_negative_nan() {
    let r = run_fxam(Fp80::INDEFINITE);
    assert!(!r.c3());
    assert!(!r.c2());
    assert!(r.c1()); // negative (INDEFINITE has sign=1)
    assert!(r.c0());
}

#[test]
fn fxam_positive_snan() {
    let r = run_fxam(POSITIVE_SNAN);
    assert!(!r.c3()); // NaN
    assert!(!r.c2());
    assert!(!r.c1());
    assert!(r.c0());
}

#[test]
fn fxam_positive_denormal() {
    let r = run_fxam(POSITIVE_DENORMAL);
    assert!(r.c3()); // Denormal: C3=1, C2=1, C0=0
    assert!(r.c2());
    assert!(!r.c1());
    assert!(!r.c0());
}

#[test]
fn fxam_negative_denormal() {
    let r = run_fxam(NEGATIVE_DENORMAL);
    assert!(r.c3());
    assert!(r.c2());
    assert!(r.c1()); // negative
    assert!(!r.c0());
}

#[test]
fn fxam_pseudo_denormal() {
    let r = run_fxam(POSITIVE_PSEUDO_DENORMAL);
    assert!(r.c3()); // Pseudo-denormal classified as Denormal
    assert!(r.c2());
    assert!(!r.c1());
    assert!(!r.c0());
}

#[test]
fn fxam_empty() {
    let r = run_fxam_empty();
    assert!(r.c3()); // Empty: C3=1, C2=0, C0=1
    assert!(!r.c2());
    // C1 = sign of the physical register value (which is 0, so positive)
    assert!(r.c0());
}

#[test]
fn fxam_unnormal() {
    let r = run_fxam(UNNORMAL);
    assert!(!r.c3()); // Unsupported: C3=0, C2=0, C0=0
    assert!(!r.c2());
    assert!(!r.c1());
    assert!(!r.c0());
}

#[test]
fn fxam_pseudo_infinity() {
    let r = run_fxam(PSEUDO_INFINITY);
    assert!(!r.c3()); // Unsupported
    assert!(!r.c2());
    assert!(!r.c1());
    assert!(!r.c0());
}

#[test]
fn fxam_pseudo_nan() {
    let r = run_fxam(PSEUDO_NAN);
    assert!(!r.c3()); // Unsupported (J=0 with exp=0x7FFF, frac!=0)
    assert!(!r.c2());
    assert!(!r.c1());
    assert!(!r.c0());
}

#[test]
fn fxam_max_denormal() {
    let r = run_fxam(MAX_DENORMAL);
    assert!(r.c3()); // Denormal
    assert!(r.c2());
    assert!(!r.c1()); // positive
    assert!(!r.c0());
}

#[test]
fn fxam_negative_pseudo_denormal() {
    let r = run_fxam(NEGATIVE_PSEUDO_DENORMAL);
    assert!(r.c3()); // Denormal (pseudo-denormal classified as denormal)
    assert!(r.c2());
    assert!(r.c1()); // negative
    assert!(!r.c0());
}

#[test]
fn fxam_smallest_normal() {
    let r = run_fxam(SMALLEST_NORMAL);
    assert!(!r.c3()); // Normal
    assert!(r.c2());
    assert!(!r.c1()); // positive
    assert!(!r.c0());
}

#[test]
fn fxam_largest_normal() {
    let r = run_fxam(LARGEST_NORMAL);
    assert!(!r.c3()); // Normal
    assert!(r.c2());
    assert!(!r.c1()); // positive
    assert!(!r.c0());
}

#[test]
fn fxam_unnormal_max_exp() {
    let r = run_fxam(UNNORMAL_MAX_EXP);
    assert!(!r.c3()); // Unsupported (J=0 with nonzero exponent)
    assert!(!r.c2());
    assert!(!r.c1());
    assert!(!r.c0());
}

#[test]
fn fxam_negative_pseudo_infinity() {
    let r = run_fxam(NEGATIVE_PSEUDO_INFINITY);
    assert!(!r.c3()); // Unsupported
    assert!(!r.c2());
    assert!(r.c1()); // negative
    assert!(!r.c0());
}

#[test]
fn fxam_max_snan() {
    let r = run_fxam(MAX_SNAN);
    assert!(!r.c3()); // NaN (SNaN with max fraction)
    assert!(!r.c2());
    assert!(!r.c1()); // positive
    assert!(r.c0());
}

#[test]
fn fxam_negative_snan() {
    let r = run_fxam(NEGATIVE_SNAN);
    assert!(!r.c3()); // NaN
    assert!(!r.c2());
    assert!(r.c1()); // negative
    assert!(r.c0());
}

#[test]
fn fxam_negative_qnan() {
    let r = run_fxam(NEGATIVE_QNAN);
    assert!(!r.c3()); // NaN
    assert!(!r.c2());
    assert!(r.c1()); // negative
    assert!(r.c0());
}

#[test]
fn fchs_negate_basic() {
    // Negate +1.0 -> -1.0
    let r = run_x87(&[0xD9, 0xE0], &[Fp80::ONE]);
    assert!(r.st(0).sign());
    assert_eq!(r.st(0).exponent(), Fp80::ONE.exponent());
    assert_eq!(r.st(0).significand(), Fp80::ONE.significand());

    // Double negate is identity
    let r = run_x87(&[0xD9, 0xE0], &[NEGATIVE_ONE]);
    assert_eq!(r.st(0), Fp80::ONE);

    // Negate zero
    let r = run_x87(&[0xD9, 0xE0], &[Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);
}

#[test]
fn fchs_negate_edge_cases() {
    let r = run_x87(&[0xD9, 0xE0], &[Fp80::NEG_ZERO]);
    assert_eq!(r.st(0), Fp80::ZERO);

    let r = run_x87(&[0xD9, 0xE0], &[Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::NEG_INFINITY);

    let r = run_x87(&[0xD9, 0xE0], &[Fp80::NEG_INFINITY]);
    assert_eq!(r.st(0), Fp80::INFINITY);

    // NaN negate flips sign, preserves significand
    let r = run_x87(&[0xD9, 0xE0], &[Fp80::INDEFINITE]);
    assert!(!r.st(0).sign()); // was negative, now positive
    assert_eq!(r.st(0).exponent(), 0x7FFF);
    assert_eq!(r.st(0).significand(), Fp80::INDEFINITE.significand());
}

#[test]
fn fabs_basic() {
    let r = run_x87(&[0xD9, 0xE1], &[Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::ONE);

    let r = run_x87(&[0xD9, 0xE1], &[NEGATIVE_ONE]);
    assert_eq!(r.st(0), Fp80::ONE);

    let r = run_x87(&[0xD9, 0xE1], &[Fp80::NEG_ZERO]);
    assert_eq!(r.st(0), Fp80::ZERO);
}

#[test]
fn fabs_edge_cases() {
    let r = run_x87(&[0xD9, 0xE1], &[Fp80::NEG_INFINITY]);
    assert_eq!(r.st(0), Fp80::INFINITY);

    // abs of INDEFINITE: sign cleared
    let r = run_x87(&[0xD9, 0xE1], &[Fp80::INDEFINITE]);
    assert!(!r.st(0).sign());
    assert_eq!(r.st(0).exponent(), 0x7FFF);
    assert_eq!(r.st(0).significand(), 0xC000_0000_0000_0000);

    // Already positive: no change
    let r = run_x87(&[0xD9, 0xE1], &[Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::INFINITY);

    let r = run_x87(&[0xD9, 0xE1], &[POSITIVE_SNAN]);
    assert_eq!(r.st(0), POSITIVE_SNAN);
}

#[test]
fn fadd_basic() {
    // FADD ST(0), ST(1) = D8 C1
    let r = run_x87(&[0xD8, 0xC1], &[Fp80::ONE, Fp80::ONE]);
    assert_eq!(r.st(0), TWO);
    assert!(r.no_exceptions());

    let r = run_x87(&[0xD8, 0xC1], &[Fp80::ONE, Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::ONE);

    let r = run_x87(&[0xD8, 0xC1], &[Fp80::ZERO, Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::ZERO);
}

#[test]
fn fadd_edge_cases() {
    // +inf + -inf = Indefinite + IE
    let r = run_x87(&[0xD8, 0xC1], &[Fp80::INFINITY, Fp80::NEG_INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // -inf + +inf = Indefinite + IE
    let r = run_x87(&[0xD8, 0xC1], &[Fp80::NEG_INFINITY, Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // +inf + +inf = +inf
    let r = run_x87(&[0xD8, 0xC1], &[Fp80::INFINITY, Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::INFINITY);
    assert!(r.no_exceptions());

    // -inf + -inf = -inf
    let r = run_x87(&[0xD8, 0xC1], &[Fp80::NEG_INFINITY, Fp80::NEG_INFINITY]);
    assert_eq!(r.st(0), Fp80::NEG_INFINITY);

    // inf + finite = inf
    let r = run_x87(&[0xD8, 0xC1], &[Fp80::INFINITY, Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::INFINITY);

    // +0 + -0 = +0 (default rounding)
    let r = run_x87(&[0xD8, 0xC1], &[Fp80::ZERO, Fp80::NEG_ZERO]);
    assert_eq!(r.st(0), Fp80::ZERO);
    assert!(!r.st(0).sign());

    // +0 + -0 = -0 when RC=Down
    let r = run_x87_with_cw(&[0xD8, 0xC1], &[Fp80::ZERO, Fp80::NEG_ZERO], CW_DOWN_EXT);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);

    // x + (-x) = +0
    let r = run_x87(&[0xD8, 0xC1], &[Fp80::ONE, NEGATIVE_ONE]);
    assert_eq!(r.st(0), Fp80::ZERO);
    assert!(!r.st(0).sign());

    // x + (-x) = -0 when RC=Down
    let r = run_x87_with_cw(&[0xD8, 0xC1], &[Fp80::ONE, NEGATIVE_ONE], CW_DOWN_EXT);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);

    // SNaN propagation
    let r = run_x87(&[0xD8, 0xC1], &[POSITIVE_SNAN, Fp80::ONE]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());

    // QNaN propagation, no IE
    let r = run_x87(&[0xD8, 0xC1], &[POSITIVE_QNAN, Fp80::ONE]);
    assert!(r.st(0).is_quiet_nan());
    assert!(!r.ie());
}

#[test]
fn fadd_same_sign_zeros() {
    // -0 + -0 = -0
    let r = run_x87(&[0xD8, 0xC1], &[Fp80::NEG_ZERO, Fp80::NEG_ZERO]);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);
}

#[test]
fn fadd_overflow() {
    let r = run_x87(&[0xD8, 0xC1], &[LARGEST_NORMAL, LARGEST_NORMAL]);
    assert_eq!(r.st(0), Fp80::INFINITY);
    assert!(r.oe());
}

#[test]
fn fadd_precision_control() {
    // Single precision
    let r = run_x87_with_cw(&[0xD8, 0xC1], &[Fp80::ONE, HALF], CW_NEAREST_SGL);
    assert_eq!(r.st(0), ONE_AND_HALF);

    // Double precision
    let r = run_x87_with_cw(&[0xD8, 0xC1], &[Fp80::ONE, HALF], CW_NEAREST_DBL);
    assert_eq!(r.st(0), ONE_AND_HALF);
}

#[test]
fn fadd_precision_loss() {
    // 1.0 + denormal: PE set
    let r = run_x87(&[0xD8, 0xC1], &[Fp80::ONE, POSITIVE_DENORMAL]);
    assert!(r.pe());
}

#[test]
fn fadd_denormal_sets_de() {
    let r = run_x87(&[0xD8, 0xC1], &[POSITIVE_DENORMAL, Fp80::ONE]);
    assert!(r.de());
}

#[test]
fn fsub_basic() {
    // FSUB ST(0), ST(1) = D8 E1
    let r = run_x87(&[0xD8, 0xE1], &[Fp80::ONE, Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::ZERO);
    assert!(!r.st(0).sign());

    let r = run_x87(&[0xD8, 0xE1], &[TWO, Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::ONE);
}

#[test]
fn fsub_edge_cases() {
    // +inf - +inf = Indefinite + IE
    let r = run_x87(&[0xD8, 0xE1], &[Fp80::INFINITY, Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // +inf - -inf = +inf
    let r = run_x87(&[0xD8, 0xE1], &[Fp80::INFINITY, Fp80::NEG_INFINITY]);
    assert_eq!(r.st(0), Fp80::INFINITY);

    // x - x = -0 when RC=Down
    let r = run_x87_with_cw(&[0xD8, 0xE1], &[Fp80::ONE, Fp80::ONE], CW_DOWN_EXT);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);

    // NaN propagation
    let r = run_x87(&[0xD8, 0xE1], &[POSITIVE_SNAN, Fp80::ONE]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());
}

#[test]
fn fsub_neg_inf_minus_neg_inf() {
    // -inf - (-inf) = Indefinite + IE
    let r = run_x87(&[0xD8, 0xE1], &[Fp80::NEG_INFINITY, Fp80::NEG_INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());
}

#[test]
fn fmul_basic() {
    // FMUL ST(0), ST(1) = D8 C9
    let r = run_x87(&[0xD8, 0xC9], &[TWO, THREE]);
    assert_eq!(r.st(0), SIX);
    assert!(r.no_exceptions());

    let r = run_x87(&[0xD8, 0xC9], &[Fp80::ONE, THREE]);
    assert_eq!(r.st(0), THREE);

    let r = run_x87(&[0xD8, 0xC9], &[Fp80::ZERO, THREE]);
    assert_eq!(r.st(0), Fp80::ZERO);
}

#[test]
fn fmul_edge_cases() {
    // inf * 0 = Indefinite + IE
    let r = run_x87(&[0xD8, 0xC9], &[Fp80::INFINITY, Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // 0 * inf = Indefinite + IE
    let r = run_x87(&[0xD8, 0xC9], &[Fp80::ZERO, Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // inf * inf = +inf
    let r = run_x87(&[0xD8, 0xC9], &[Fp80::INFINITY, Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::INFINITY);

    // +inf * -inf = -inf
    let r = run_x87(&[0xD8, 0xC9], &[Fp80::INFINITY, Fp80::NEG_INFINITY]);
    assert_eq!(r.st(0), Fp80::NEG_INFINITY);

    // neg * neg = pos
    let r = run_x87(&[0xD8, 0xC9], &[NEGATIVE_ONE, NEGATIVE_ONE]);
    assert_eq!(r.st(0), Fp80::ONE);

    // -1 * 0 = -0
    let r = run_x87(&[0xD8, 0xC9], &[NEGATIVE_ONE, Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);

    // NaN propagation
    let r = run_x87(&[0xD8, 0xC9], &[POSITIVE_SNAN, Fp80::ONE]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());
}

#[test]
fn fmul_neg_zero_sign_xor() {
    // -0 * -0 = +0
    let r = run_x87(&[0xD8, 0xC9], &[Fp80::NEG_ZERO, Fp80::NEG_ZERO]);
    assert_eq!(r.st(0), Fp80::ZERO);
    assert!(!r.st(0).sign());
}

#[test]
fn fmul_inf_finite_sign_xor() {
    let r = run_x87(&[0xD8, 0xC9], &[Fp80::INFINITY, THREE]);
    assert_eq!(r.st(0), Fp80::INFINITY);

    let r = run_x87(&[0xD8, 0xC9], &[Fp80::INFINITY, NEGATIVE_ONE]);
    assert_eq!(r.st(0), Fp80::NEG_INFINITY);

    let r = run_x87(&[0xD8, 0xC9], &[Fp80::NEG_INFINITY, THREE]);
    assert_eq!(r.st(0), Fp80::NEG_INFINITY);

    let r = run_x87(&[0xD8, 0xC9], &[Fp80::NEG_INFINITY, NEGATIVE_ONE]);
    assert_eq!(r.st(0), Fp80::INFINITY);
}

#[test]
fn fdiv_basic() {
    // FDIV ST(0), ST(1) = D8 F1
    let r = run_x87(&[0xD8, 0xF1], &[SIX, TWO]);
    assert_eq!(r.st(0), THREE);
    assert!(r.no_exceptions());

    let r = run_x87(&[0xD8, 0xF1], &[THREE, Fp80::ONE]);
    assert_eq!(r.st(0), THREE);
}

#[test]
fn fdiv_edge_cases() {
    // 0 / 0 = Indefinite + IE
    let r = run_x87(&[0xD8, 0xF1], &[Fp80::ZERO, Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // inf / inf = Indefinite + IE
    let r = run_x87(&[0xD8, 0xF1], &[Fp80::INFINITY, Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // nonzero / 0 = +inf + ZE
    let r = run_x87(&[0xD8, 0xF1], &[Fp80::ONE, Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::INFINITY);
    assert!(r.ze());

    // negative / 0 = -inf + ZE
    let r = run_x87(&[0xD8, 0xF1], &[NEGATIVE_ONE, Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::NEG_INFINITY);
    assert!(r.ze());

    // 0 / nonzero = 0
    let r = run_x87(&[0xD8, 0xF1], &[Fp80::ZERO, Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::ZERO);

    // -0 / positive = -0
    let r = run_x87(&[0xD8, 0xF1], &[Fp80::NEG_ZERO, Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);

    // inf / finite = inf
    let r = run_x87(&[0xD8, 0xF1], &[Fp80::INFINITY, Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::INFINITY);

    // finite / inf = 0
    let r = run_x87(&[0xD8, 0xF1], &[Fp80::ONE, Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::ZERO);

    // NaN propagation
    let r = run_x87(&[0xD8, 0xF1], &[POSITIVE_SNAN, Fp80::ONE]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());
}

#[test]
fn fdiv_positive_over_neg_zero() {
    // positive / -0 = -inf + ZE
    let r = run_x87(&[0xD8, 0xF1], &[Fp80::ONE, Fp80::NEG_ZERO]);
    assert_eq!(r.st(0), Fp80::NEG_INFINITY);
    assert!(r.ze());
}

#[test]
fn fdiv_precision_loss() {
    // 1 / 3 is not exact -> PE
    let r = run_x87(&[0xD8, 0xF1], &[Fp80::ONE, THREE]);
    assert!(r.pe());
}

#[test]
fn fsqrt_basic() {
    // FSQRT = D9 FA
    let r = run_x87(&[0xD9, 0xFA], &[FOUR]);
    assert_eq!(r.st(0), TWO);
    assert!(r.no_exceptions());

    let r = run_x87(&[0xD9, 0xFA], &[Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::ONE);
}

#[test]
fn fsqrt_edge_cases() {
    let r = run_x87(&[0xD9, 0xFA], &[Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::ZERO);

    let r = run_x87(&[0xD9, 0xFA], &[Fp80::NEG_ZERO]);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);

    let r = run_x87(&[0xD9, 0xFA], &[Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::INFINITY);

    // sqrt(-inf) = Indefinite + IE
    let r = run_x87(&[0xD9, 0xFA], &[Fp80::NEG_INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // sqrt(negative) = Indefinite + IE
    let r = run_x87(&[0xD9, 0xFA], &[NEGATIVE_ONE]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // sqrt(SNaN) = QNaN + IE
    let r = run_x87(&[0xD9, 0xFA], &[POSITIVE_SNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());

    // sqrt(QNaN) = QNaN, no IE
    let r = run_x87(&[0xD9, 0xFA], &[POSITIVE_QNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(!r.ie());
}

#[test]
fn fsqrt_denormal_sets_de() {
    let r = run_x87(&[0xD9, 0xFA], &[POSITIVE_DENORMAL]);
    assert!(r.de());
}

#[test]
fn fsqrt_precision_loss() {
    // sqrt(2) is irrational -> PE
    let r = run_x87(&[0xD9, 0xFA], &[TWO]);
    assert!(r.pe());
}

#[test]
fn frndint_basic() {
    // FRNDINT = D9 FC
    let r = run_x87(&[0xD9, 0xFC], &[Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::ONE);
    assert!(r.no_exceptions());

    // 1.5 NearestEven -> 2.0
    let r = run_x87(&[0xD9, 0xFC], &[ONE_AND_HALF]);
    assert_eq!(r.st(0), TWO);
    assert!(r.pe());

    // 2.5 NearestEven -> 2.0 (round to even)
    let r = run_x87(&[0xD9, 0xFC], &[TWO_AND_HALF]);
    assert_eq!(r.st(0), TWO);
    assert!(r.pe());
}

#[test]
fn frndint_edge_cases() {
    // All 4 rounding modes on 1.5
    let r = run_x87_with_cw(&[0xD9, 0xFC], &[ONE_AND_HALF], CW_UP_EXT);
    assert_eq!(r.st(0), TWO);
    assert!(r.pe());

    let r = run_x87_with_cw(&[0xD9, 0xFC], &[ONE_AND_HALF], CW_DOWN_EXT);
    assert_eq!(r.st(0), Fp80::ONE);
    assert!(r.pe());

    let r = run_x87_with_cw(&[0xD9, 0xFC], &[ONE_AND_HALF], CW_ZERO_EXT);
    assert_eq!(r.st(0), Fp80::ONE);
    assert!(r.pe());

    // Negative: -1.5 rounding modes
    let r = run_x87(&[0xD9, 0xFC], &[NEGATIVE_ONE_AND_HALF]);
    assert_eq!(r.st(0), NEGATIVE_TWO);
    assert!(r.pe());

    let r = run_x87_with_cw(&[0xD9, 0xFC], &[NEGATIVE_ONE_AND_HALF], CW_UP_EXT);
    assert_eq!(r.st(0), NEGATIVE_ONE);
    assert!(r.pe());

    let r = run_x87_with_cw(&[0xD9, 0xFC], &[NEGATIVE_ONE_AND_HALF], CW_DOWN_EXT);
    assert_eq!(r.st(0), NEGATIVE_TWO);
    assert!(r.pe());

    let r = run_x87_with_cw(&[0xD9, 0xFC], &[NEGATIVE_ONE_AND_HALF], CW_ZERO_EXT);
    assert_eq!(r.st(0), NEGATIVE_ONE);
    assert!(r.pe());

    // SNaN raises IE
    let r = run_x87(&[0xD9, 0xFC], &[POSITIVE_SNAN]);
    assert!(r.st(0).is_nan());
    assert!(r.ie());

    // Infinity unchanged
    let r = run_x87(&[0xD9, 0xFC], &[Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::INFINITY);
    assert!(r.no_exceptions());

    // Zero unchanged
    let r = run_x87(&[0xD9, 0xFC], &[Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::ZERO);

    // QNaN unchanged, no IE
    let r = run_x87(&[0xD9, 0xFC], &[POSITIVE_QNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(!r.ie());

    // -0 preserved
    let r = run_x87(&[0xD9, 0xFC], &[Fp80::NEG_ZERO]);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);
}

#[test]
fn fcom_basic() {
    // FCOM ST(1) = D8 D1
    // Greater: C3=0, C2=0, C0=0
    let r = run_x87(&[0xD8, 0xD1], &[Fp80::ONE, Fp80::ZERO]);
    assert!(!r.c3());
    assert!(!r.c2());
    assert!(!r.c0());
    assert!(r.no_exceptions());

    // Less: C0=1
    let r = run_x87(&[0xD8, 0xD1], &[Fp80::ZERO, Fp80::ONE]);
    assert!(!r.c3());
    assert!(!r.c2());
    assert!(r.c0());

    // Equal: C3=1
    let r = run_x87(&[0xD8, 0xD1], &[Fp80::ONE, Fp80::ONE]);
    assert!(r.c3());
    assert!(!r.c2());
    assert!(!r.c0());

    // +0 == -0
    let r = run_x87(&[0xD8, 0xD1], &[Fp80::ZERO, Fp80::NEG_ZERO]);
    assert!(r.c3());
    assert!(!r.c2());
    assert!(!r.c0());
}

#[test]
fn fcompp_pops_both_operands() {
    let r = run_x87(&[0xDE, 0xD9], &[Fp80::ONE, Fp80::ZERO]);
    assert!(!r.c3());
    assert!(!r.c2());
    assert!(!r.c0());
    assert_eq!(r.tag(0), 0b11);
    assert_eq!(r.tag(1), 0b11);
    assert!(r.no_exceptions());
}

#[test]
fn fcom_edge_cases() {
    // QNaN -> Unordered + IE (ordered comparison raises IE on ANY NaN)
    let r = run_x87(&[0xD8, 0xD1], &[POSITIVE_QNAN, Fp80::ONE]);
    assert!(r.c3());
    assert!(r.c2());
    assert!(r.c0());
    assert!(r.ie());

    let r = run_x87(&[0xD8, 0xD1], &[Fp80::ONE, POSITIVE_QNAN]);
    assert!(r.c3());
    assert!(r.c2());
    assert!(r.c0());
    assert!(r.ie());

    // SNaN -> Unordered + IE
    let r = run_x87(&[0xD8, 0xD1], &[POSITIVE_SNAN, Fp80::ONE]);
    assert!(r.c3());
    assert!(r.c2());
    assert!(r.c0());
    assert!(r.ie());

    // Infinity comparisons
    let r = run_x87(&[0xD8, 0xD1], &[Fp80::INFINITY, Fp80::ONE]);
    assert!(!r.c3());
    assert!(!r.c2());
    assert!(!r.c0()); // Greater

    let r = run_x87(&[0xD8, 0xD1], &[Fp80::NEG_INFINITY, Fp80::ONE]);
    assert!(!r.c3());
    assert!(!r.c2());
    assert!(r.c0()); // Less

    let r = run_x87(&[0xD8, 0xD1], &[Fp80::INFINITY, Fp80::INFINITY]);
    assert!(r.c3());
    assert!(!r.c2());
    assert!(!r.c0()); // Equal

    let r = run_x87(&[0xD8, 0xD1], &[Fp80::NEG_INFINITY, Fp80::INFINITY]);
    assert!(!r.c3());
    assert!(!r.c2());
    assert!(r.c0()); // Less

    // Negative values
    let r = run_x87(&[0xD8, 0xD1], &[NEGATIVE_ONE, Fp80::ONE]);
    assert!(r.c0()); // Less

    // -inf == -inf
    let r = run_x87(&[0xD8, 0xD1], &[Fp80::NEG_INFINITY, Fp80::NEG_INFINITY]);
    assert!(r.c3());
    assert!(r.no_exceptions());

    // Two QNaNs -> Unordered + IE
    let r = run_x87(&[0xD8, 0xD1], &[POSITIVE_QNAN, Fp80::INDEFINITE]);
    assert!(r.c3());
    assert!(r.c2());
    assert!(r.c0());
    assert!(r.ie());

    // Two SNaNs -> Unordered + IE
    let r = run_x87(&[0xD8, 0xD1], &[POSITIVE_SNAN, POSITIVE_SNAN]);
    assert!(r.c3());
    assert!(r.c2());
    assert!(r.c0());
    assert!(r.ie());

    // QNaN + SNaN -> Unordered + IE
    let r = run_x87(&[0xD8, 0xD1], &[POSITIVE_QNAN, POSITIVE_SNAN]);
    assert!(r.c3());
    assert!(r.c2());
    assert!(r.c0());
    assert!(r.ie());

    // Denormal comparison
    let r = run_x87(&[0xD8, 0xD1], &[POSITIVE_DENORMAL, Fp80::ZERO]);
    assert!(!r.c3());
    assert!(!r.c2());
    assert!(!r.c0()); // Greater
    assert!(r.de());

    let r = run_x87(&[0xD8, 0xD1], &[POSITIVE_DENORMAL, Fp80::ONE]);
    assert!(r.c0()); // Less
    assert!(r.de());
}

#[test]
fn fcom_denormal_flag() {
    let r = run_x87(&[0xD8, 0xD1], &[POSITIVE_DENORMAL, Fp80::ONE]);
    assert!(r.c0()); // Less
    assert!(r.de());
    assert!(!r.ie());

    let r = run_x87(&[0xD8, 0xD1], &[Fp80::ONE, POSITIVE_DENORMAL]);
    assert!(!r.c0()); // Greater
    assert!(r.de());

    // No denormals: no DE
    let r = run_x87(&[0xD8, 0xD1], &[Fp80::ONE, TWO]);
    assert!(!r.de());

    let r = run_x87(&[0xD8, 0xD1], &[POSITIVE_DENORMAL, Fp80::ZERO]);
    assert!(!r.c0()); // Greater
    assert!(r.de());
}

#[test]
fn fucom_basic() {
    // FUCOM ST(1) = DD E1
    let r = run_x87(&[0xDD, 0xE1], &[TWO, Fp80::ONE]);
    assert!(!r.c3());
    assert!(!r.c2());
    assert!(!r.c0()); // Greater
    assert!(r.no_exceptions());

    let r = run_x87(&[0xDD, 0xE1], &[Fp80::ONE, Fp80::ONE]);
    assert!(r.c3()); // Equal

    // +0 == -0
    let r = run_x87(&[0xDD, 0xE1], &[Fp80::ZERO, Fp80::NEG_ZERO]);
    assert!(r.c3());
}

#[test]
fn fucom_edge_cases() {
    // QNaN -> Unordered, NO IE (key difference from FCOM)
    let r = run_x87(&[0xDD, 0xE1], &[POSITIVE_QNAN, Fp80::ONE]);
    assert!(r.c3());
    assert!(r.c2());
    assert!(r.c0());
    assert!(!r.ie());

    let r = run_x87(&[0xDD, 0xE1], &[Fp80::ONE, POSITIVE_QNAN]);
    assert!(r.c3());
    assert!(r.c2());
    assert!(r.c0());
    assert!(!r.ie());

    // SNaN -> Unordered + IE (even in unordered comparison)
    let r = run_x87(&[0xDD, 0xE1], &[POSITIVE_SNAN, Fp80::ONE]);
    assert!(r.c3());
    assert!(r.c2());
    assert!(r.c0());
    assert!(r.ie());

    let r = run_x87(&[0xDD, 0xE1], &[Fp80::ONE, POSITIVE_SNAN]);
    assert!(r.ie());

    // Two QNaNs -> Unordered, no IE
    let r = run_x87(&[0xDD, 0xE1], &[POSITIVE_QNAN, Fp80::INDEFINITE]);
    assert!(r.c3());
    assert!(r.c2());
    assert!(r.c0());
    assert!(!r.ie());

    // Two SNaNs -> Unordered + IE
    let r = run_x87(&[0xDD, 0xE1], &[POSITIVE_SNAN, POSITIVE_SNAN]);
    assert!(r.ie());

    // QNaN + SNaN -> IE
    let r = run_x87(&[0xDD, 0xE1], &[POSITIVE_QNAN, POSITIVE_SNAN]);
    assert!(r.ie());
}

#[test]
fn fucom_denormal_flag() {
    let r = run_x87(&[0xDD, 0xE1], &[POSITIVE_DENORMAL, Fp80::ONE]);
    assert!(r.c0()); // Less
    assert!(r.de());
    assert!(!r.ie());

    let r = run_x87(&[0xDD, 0xE1], &[Fp80::ONE, POSITIVE_DENORMAL]);
    assert!(r.de());

    let r = run_x87(&[0xDD, 0xE1], &[Fp80::ONE, TWO]);
    assert!(!r.de());
}

#[test]
fn fild_m16_basic() {
    // FILD m16 = DF 06 00 00
    let (r, _) = run_x87_mem(
        &[0xDF, 0x06, 0x00, 0x00],
        &[],
        CW_NEAREST_EXT,
        &[0x00, 0x00],
    );
    assert_eq!(r.st(0), Fp80::ZERO);

    let (r, _) = run_x87_mem(
        &[0xDF, 0x06, 0x00, 0x00],
        &[],
        CW_NEAREST_EXT,
        &[0x01, 0x00],
    );
    assert_eq!(r.st(0), Fp80::ONE);

    let (r, _) = run_x87_mem(
        &[0xDF, 0x06, 0x00, 0x00],
        &[],
        CW_NEAREST_EXT,
        &[0xFF, 0xFF],
    ); // -1
    assert_eq!(r.st(0), NEGATIVE_ONE);
}

#[test]
fn fild_m16_edge_cases() {
    // i16::MAX = 32767
    let bytes = 32767i16.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDF, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::from_i16(i16::MAX));

    // i16::MIN = -32768
    let bytes = (-32768i16).to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDF, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::from_i16(i16::MIN));
}

#[test]
fn fild_m32_basic() {
    // FILD m32 = DB 06 00 00
    let (r, _) = run_x87_mem(
        &[0xDB, 0x06, 0x00, 0x00],
        &[],
        CW_NEAREST_EXT,
        &[0x00, 0x00, 0x00, 0x00],
    );
    assert_eq!(r.st(0), Fp80::ZERO);

    let (r, _) = run_x87_mem(
        &[0xDB, 0x06, 0x00, 0x00],
        &[],
        CW_NEAREST_EXT,
        &[0x01, 0x00, 0x00, 0x00],
    );
    assert_eq!(r.st(0), Fp80::ONE);

    let bytes = (-1i32).to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDB, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), NEGATIVE_ONE);

    let bytes = 256i32.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDB, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::from_i32(256));
}

#[test]
fn fild_m32_edge_cases() {
    let bytes = i32::MAX.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDB, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::from_i32(i32::MAX));

    let bytes = i32::MIN.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDB, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::from_i32(i32::MIN));
}

#[test]
fn fild_m64_basic() {
    // FILD m64 = DF 2E 00 00
    let bytes = 0i64.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDF, 0x2E, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::ZERO);

    let bytes = 1i64.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDF, 0x2E, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::ONE);

    let bytes = (-1i64).to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDF, 0x2E, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), NEGATIVE_ONE);

    let bytes = 2i64.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDF, 0x2E, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), TWO);
}

#[test]
fn fild_m64_edge_cases() {
    let bytes = i64::MAX.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDF, 0x2E, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::from_i64(i64::MAX));

    let bytes = i64::MIN.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDF, 0x2E, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::from_i64(i64::MIN));
}

#[test]
fn fld_m32_basic() {
    // FLD m32 = D9 06 00 00
    let bytes = 1.0f32.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xD9, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::ONE);

    let bytes = 0.0f32.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xD9, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::ZERO);

    let bytes = (-0.0f32).to_le_bytes();
    let (r, _) = run_x87_mem(&[0xD9, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);
}

#[test]
fn fld_m32_edge_cases() {
    let bytes = f32::INFINITY.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xD9, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::INFINITY);

    let bytes = f32::NEG_INFINITY.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xD9, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::NEG_INFINITY);

    // QNaN preserved
    let bytes = f32::NAN.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xD9, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert!(r.st(0).is_quiet_nan());

    // Negative value
    let bytes = (-2.0f32).to_le_bytes();
    let (r, _) = run_x87_mem(&[0xD9, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert!(r.st(0).sign());
    assert_eq!(r.st(0).exponent(), 0x4000);
}

#[test]
fn fld_m64_basic() {
    // FLD m64 = DD 06 00 00
    let bytes = 1.0f64.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDD, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::ONE);

    let bytes = 0.0f64.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDD, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::ZERO);

    let bytes = (-0.0f64).to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDD, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);
}

#[test]
fn fld_m64_edge_cases() {
    let bytes = f64::INFINITY.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDD, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::INFINITY);

    let bytes = f64::NEG_INFINITY.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDD, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert_eq!(r.st(0), Fp80::NEG_INFINITY);

    // QNaN preserved
    let bytes = f64::NAN.to_le_bytes();
    let (r, _) = run_x87_mem(&[0xDD, 0x06, 0x00, 0x00], &[], CW_NEAREST_EXT, &bytes);
    assert!(r.st(0).is_quiet_nan());
}

#[test]
fn fistp_m16_basic() {
    // FISTP m16 = DF 1E 00 00
    let (r, bus) = run_x87_mem(&[0xDF, 0x1E, 0x00, 0x00], &[Fp80::ONE], CW_NEAREST_EXT, &[]);
    assert_eq!(read_mem_i16(&bus, 0), 1);
    assert_eq!(r.tag(0), 0b11); // popped -> empty

    let (_, bus) = run_x87_mem(
        &[0xDF, 0x1E, 0x00, 0x00],
        &[Fp80::ZERO],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i16(&bus, 0), 0);

    let (_, bus) = run_x87_mem(
        &[0xDF, 0x1E, 0x00, 0x00],
        &[NEGATIVE_ONE],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i16(&bus, 0), -1);

    let (_, bus) = run_x87_mem(
        &[0xDF, 0x1E, 0x00, 0x00],
        &[Fp80::from_i16(i16::MAX)],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i16(&bus, 0), i16::MAX);

    let (_, bus) = run_x87_mem(
        &[0xDF, 0x1E, 0x00, 0x00],
        &[Fp80::from_i16(i16::MIN)],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i16(&bus, 0), i16::MIN);
}

#[test]
fn fistp_m16_edge_cases() {
    // Overflow -> integer indefinite + IE
    let (r, bus) = run_x87_mem(
        &[0xDF, 0x1E, 0x00, 0x00],
        &[Fp80::from_i32(40000)],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i16(&bus, 0), i16::MIN);
    assert!(r.ie());

    // NaN -> integer indefinite + IE
    let (r, bus) = run_x87_mem(
        &[0xDF, 0x1E, 0x00, 0x00],
        &[Fp80::INDEFINITE],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i16(&bus, 0), i16::MIN);
    assert!(r.ie());

    // Infinity -> integer indefinite + IE
    let (r, bus) = run_x87_mem(
        &[0xDF, 0x1E, 0x00, 0x00],
        &[Fp80::INFINITY],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i16(&bus, 0), i16::MIN);
    assert!(r.ie());

    // Rounding: 1.5 NearestEven -> 2
    let (r, bus) = run_x87_mem(
        &[0xDF, 0x1E, 0x00, 0x00],
        &[ONE_AND_HALF],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i16(&bus, 0), 2);
    assert!(r.pe());

    // 2.5 NearestEven -> 2
    let (_, bus) = run_x87_mem(
        &[0xDF, 0x1E, 0x00, 0x00],
        &[TWO_AND_HALF],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i16(&bus, 0), 2);

    // 1.5 toward zero -> 1
    let (_, bus) = run_x87_mem(&[0xDF, 0x1E, 0x00, 0x00], &[ONE_AND_HALF], CW_ZERO_EXT, &[]);
    assert_eq!(read_mem_i16(&bus, 0), 1);

    // -1.5 toward zero -> -1
    let (r, bus) = run_x87_mem(
        &[0xDF, 0x1E, 0x00, 0x00],
        &[NEGATIVE_ONE_AND_HALF],
        CW_ZERO_EXT,
        &[],
    );
    assert_eq!(read_mem_i16(&bus, 0), -1);
    assert!(r.pe());

    // 1.5 toward +inf -> 2
    let (r, bus) = run_x87_mem(&[0xDF, 0x1E, 0x00, 0x00], &[ONE_AND_HALF], CW_UP_EXT, &[]);
    assert_eq!(read_mem_i16(&bus, 0), 2);
    assert!(r.pe());

    // -1.5 toward +inf -> -1
    let (r, bus) = run_x87_mem(
        &[0xDF, 0x1E, 0x00, 0x00],
        &[NEGATIVE_ONE_AND_HALF],
        CW_UP_EXT,
        &[],
    );
    assert_eq!(read_mem_i16(&bus, 0), -1);
    assert!(r.pe());

    // 1.5 toward -inf -> 1
    let (r, bus) = run_x87_mem(&[0xDF, 0x1E, 0x00, 0x00], &[ONE_AND_HALF], CW_DOWN_EXT, &[]);
    assert_eq!(read_mem_i16(&bus, 0), 1);
    assert!(r.pe());

    // -1.5 toward -inf -> -2
    let (r, bus) = run_x87_mem(
        &[0xDF, 0x1E, 0x00, 0x00],
        &[NEGATIVE_ONE_AND_HALF],
        CW_DOWN_EXT,
        &[],
    );
    assert_eq!(read_mem_i16(&bus, 0), -2);
    assert!(r.pe());

    // Negative overflow
    let (r, bus) = run_x87_mem(
        &[0xDF, 0x1E, 0x00, 0x00],
        &[Fp80::from_i32(-40000)],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i16(&bus, 0), i16::MIN);
    assert!(r.ie());
}

#[test]
fn fistp_m16_neg_zero() {
    // -0 -> 0
    let (_, bus) = run_x87_mem(
        &[0xDF, 0x1E, 0x00, 0x00],
        &[Fp80::NEG_ZERO],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i16(&bus, 0), 0);
}

#[test]
fn fistp_m32_basic() {
    // FISTP m32 = DB 1E 00 00
    let (_, bus) = run_x87_mem(&[0xDB, 0x1E, 0x00, 0x00], &[Fp80::ONE], CW_NEAREST_EXT, &[]);
    assert_eq!(read_mem_i32(&bus, 0), 1);

    let (_, bus) = run_x87_mem(
        &[0xDB, 0x1E, 0x00, 0x00],
        &[Fp80::ZERO],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i32(&bus, 0), 0);

    let (_, bus) = run_x87_mem(
        &[0xDB, 0x1E, 0x00, 0x00],
        &[Fp80::from_i32(i32::MAX)],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i32(&bus, 0), i32::MAX);

    let (_, bus) = run_x87_mem(
        &[0xDB, 0x1E, 0x00, 0x00],
        &[Fp80::from_i32(i32::MIN)],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i32(&bus, 0), i32::MIN);
}

#[test]
fn fistp_m32_edge_cases() {
    // Overflow
    let too_large = Fp80::from_i64(i64::from(i32::MAX) + 1);
    let (r, bus) = run_x87_mem(&[0xDB, 0x1E, 0x00, 0x00], &[too_large], CW_NEAREST_EXT, &[]);
    assert_eq!(read_mem_i32(&bus, 0), i32::MIN);
    assert!(r.ie());

    // NaN
    let (r, bus) = run_x87_mem(
        &[0xDB, 0x1E, 0x00, 0x00],
        &[Fp80::INDEFINITE],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i32(&bus, 0), i32::MIN);
    assert!(r.ie());

    // 1.5 NearestEven -> 2
    let (r, bus) = run_x87_mem(
        &[0xDB, 0x1E, 0x00, 0x00],
        &[ONE_AND_HALF],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i32(&bus, 0), 2);
    assert!(r.pe());

    // -Infinity -> integer indefinite + IE
    let (r, bus) = run_x87_mem(
        &[0xDB, 0x1E, 0x00, 0x00],
        &[Fp80::NEG_INFINITY],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i32(&bus, 0), i32::MIN);
    assert!(r.ie());

    // -0 -> 0
    let (_, bus) = run_x87_mem(
        &[0xDB, 0x1E, 0x00, 0x00],
        &[Fp80::NEG_ZERO],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i32(&bus, 0), 0);

    // 1.5 toward zero -> 1
    let (r, bus) = run_x87_mem(&[0xDB, 0x1E, 0x00, 0x00], &[ONE_AND_HALF], CW_ZERO_EXT, &[]);
    assert_eq!(read_mem_i32(&bus, 0), 1);
    assert!(r.pe());

    // -1.5 toward zero -> -1
    let (r, bus) = run_x87_mem(
        &[0xDB, 0x1E, 0x00, 0x00],
        &[NEGATIVE_ONE_AND_HALF],
        CW_ZERO_EXT,
        &[],
    );
    assert_eq!(read_mem_i32(&bus, 0), -1);
    assert!(r.pe());

    // 1.5 toward +inf -> 2
    let (r, bus) = run_x87_mem(&[0xDB, 0x1E, 0x00, 0x00], &[ONE_AND_HALF], CW_UP_EXT, &[]);
    assert_eq!(read_mem_i32(&bus, 0), 2);
    assert!(r.pe());

    // -1.5 toward +inf -> -1
    let (r, bus) = run_x87_mem(
        &[0xDB, 0x1E, 0x00, 0x00],
        &[NEGATIVE_ONE_AND_HALF],
        CW_UP_EXT,
        &[],
    );
    assert_eq!(read_mem_i32(&bus, 0), -1);
    assert!(r.pe());

    // 1.5 toward -inf -> 1
    let (r, bus) = run_x87_mem(&[0xDB, 0x1E, 0x00, 0x00], &[ONE_AND_HALF], CW_DOWN_EXT, &[]);
    assert_eq!(read_mem_i32(&bus, 0), 1);
    assert!(r.pe());

    // -1.5 toward -inf -> -2
    let (r, bus) = run_x87_mem(
        &[0xDB, 0x1E, 0x00, 0x00],
        &[NEGATIVE_ONE_AND_HALF],
        CW_DOWN_EXT,
        &[],
    );
    assert_eq!(read_mem_i32(&bus, 0), -2);
    assert!(r.pe());
}

#[test]
fn fistp_m64_basic() {
    // FISTP m64 = DF 3E 00 00
    let (_, bus) = run_x87_mem(&[0xDF, 0x3E, 0x00, 0x00], &[Fp80::ONE], CW_NEAREST_EXT, &[]);
    assert_eq!(read_mem_i64(&bus, 0), 1);

    let (_, bus) = run_x87_mem(
        &[0xDF, 0x3E, 0x00, 0x00],
        &[Fp80::ZERO],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i64(&bus, 0), 0);

    let (_, bus) = run_x87_mem(
        &[0xDF, 0x3E, 0x00, 0x00],
        &[Fp80::from_i64(i64::MAX)],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i64(&bus, 0), i64::MAX);
}

#[test]
fn fistp_m64_edge_cases() {
    // i64::MIN round-trip
    let (_, bus) = run_x87_mem(
        &[0xDF, 0x3E, 0x00, 0x00],
        &[Fp80::from_i64(i64::MIN)],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i64(&bus, 0), i64::MIN);

    // Infinity -> indefinite + IE
    let (r, bus) = run_x87_mem(
        &[0xDF, 0x3E, 0x00, 0x00],
        &[Fp80::INFINITY],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i64(&bus, 0), i64::MIN);
    assert!(r.ie());

    // 0.5 NearestEven -> 0 (round to even)
    let (r, bus) = run_x87_mem(&[0xDF, 0x3E, 0x00, 0x00], &[HALF], CW_NEAREST_EXT, &[]);
    assert_eq!(read_mem_i64(&bus, 0), 0);
    assert!(r.pe());

    // 0.5 toward +inf -> 1
    let (_, bus) = run_x87_mem(&[0xDF, 0x3E, 0x00, 0x00], &[HALF], CW_UP_EXT, &[]);
    assert_eq!(read_mem_i64(&bus, 0), 1);

    // SNaN -> integer indefinite + IE
    let (r, bus) = run_x87_mem(
        &[0xDF, 0x3E, 0x00, 0x00],
        &[POSITIVE_SNAN],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i64(&bus, 0), i64::MIN);
    assert!(r.ie());

    // -0 -> 0
    let (_, bus) = run_x87_mem(
        &[0xDF, 0x3E, 0x00, 0x00],
        &[Fp80::NEG_ZERO],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_i64(&bus, 0), 0);

    // 0.5 toward zero -> 0
    let (r, bus) = run_x87_mem(&[0xDF, 0x3E, 0x00, 0x00], &[HALF], CW_ZERO_EXT, &[]);
    assert_eq!(read_mem_i64(&bus, 0), 0);
    assert!(r.pe());

    // 0.5 toward -inf -> 0
    let (r, bus) = run_x87_mem(&[0xDF, 0x3E, 0x00, 0x00], &[HALF], CW_DOWN_EXT, &[]);
    assert_eq!(read_mem_i64(&bus, 0), 0);
    assert!(r.pe());

    // -0.5 toward -inf -> -1
    let (r, bus) = run_x87_mem(
        &[0xDF, 0x3E, 0x00, 0x00],
        &[NEGATIVE_HALF],
        CW_DOWN_EXT,
        &[],
    );
    assert_eq!(read_mem_i64(&bus, 0), -1);
    assert!(r.pe());
}

#[test]
fn fst_m32_basic() {
    // FST m32 = D9 16 00 00
    let (_, bus) = run_x87_mem(&[0xD9, 0x16, 0x00, 0x00], &[Fp80::ONE], CW_NEAREST_EXT, &[]);
    let bits = read_mem_u32(&bus, 0);
    assert_eq!(f32::from_bits(bits), 1.0f32);

    let (_, bus) = run_x87_mem(
        &[0xD9, 0x16, 0x00, 0x00],
        &[Fp80::ZERO],
        CW_NEAREST_EXT,
        &[],
    );
    let bits = read_mem_u32(&bus, 0);
    assert_eq!(f32::from_bits(bits), 0.0f32);

    let (_, bus) = run_x87_mem(
        &[0xD9, 0x16, 0x00, 0x00],
        &[Fp80::NEG_ZERO],
        CW_NEAREST_EXT,
        &[],
    );
    let bits = read_mem_u32(&bus, 0);
    assert_eq!(f32::from_bits(bits), -0.0f32);
}

#[test]
fn fst_m32_edge_cases() {
    // Infinity passthrough
    let (_, bus) = run_x87_mem(
        &[0xD9, 0x16, 0x00, 0x00],
        &[Fp80::INFINITY],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(f32::from_bits(read_mem_u32(&bus, 0)), f32::INFINITY);

    // Large value overflows to infinity + OE
    let (r, bus) = run_x87_mem(
        &[0xD9, 0x16, 0x00, 0x00],
        &[LARGEST_NORMAL],
        CW_NEAREST_EXT,
        &[],
    );
    assert!(f32::from_bits(read_mem_u32(&bus, 0)).is_infinite());
    assert!(r.oe());

    // QNaN passthrough
    let (_, bus) = run_x87_mem(
        &[0xD9, 0x16, 0x00, 0x00],
        &[Fp80::INDEFINITE],
        CW_NEAREST_EXT,
        &[],
    );
    assert!(f32::from_bits(read_mem_u32(&bus, 0)).is_nan());

    // Precision loss
    let precise = Fp80::from_bits(0x3FFF, 0x8000_0000_0000_0001);
    let (r, _) = run_x87_mem(&[0xD9, 0x16, 0x00, 0x00], &[precise], CW_NEAREST_EXT, &[]);
    assert!(r.pe());
}

#[test]
fn fst_m32_underflow() {
    let (r, bus) = run_x87_mem(
        &[0xD9, 0x16, 0x00, 0x00],
        &[TINY_NORMAL],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(f32::from_bits(read_mem_u32(&bus, 0)), 0.0f32);
    assert!(r.ue());
}

#[test]
fn fst_m32_snan() {
    let (r, bus) = run_x87_mem(
        &[0xD9, 0x16, 0x00, 0x00],
        &[POSITIVE_SNAN],
        CW_NEAREST_EXT,
        &[],
    );
    assert!(f32::from_bits(read_mem_u32(&bus, 0)).is_nan());
    assert!(r.ie());
}

#[test]
fn fst_m32_rounding_modes() {
    // Value not exactly representable: 1.0 + 2^-63
    let precise = Fp80::from_bits(0x3FFF, 0x8000_0000_0000_0001);

    let (r, _) = run_x87_mem(&[0xD9, 0x16, 0x00, 0x00], &[precise], CW_UP_EXT, &[]);
    assert!(r.pe());

    let (r, _) = run_x87_mem(&[0xD9, 0x16, 0x00, 0x00], &[precise], CW_DOWN_EXT, &[]);
    assert!(r.pe());

    let (r, _) = run_x87_mem(&[0xD9, 0x16, 0x00, 0x00], &[precise], CW_ZERO_EXT, &[]);
    assert!(r.pe());
}

#[test]
fn fst_m64_basic() {
    // FST m64 = DD 16 00 00
    let (_, bus) = run_x87_mem(&[0xDD, 0x16, 0x00, 0x00], &[Fp80::ONE], CW_NEAREST_EXT, &[]);
    assert_eq!(f64::from_bits(read_mem_u64(&bus, 0)), 1.0f64);

    let (_, bus) = run_x87_mem(
        &[0xDD, 0x16, 0x00, 0x00],
        &[Fp80::ZERO],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(f64::from_bits(read_mem_u64(&bus, 0)), 0.0f64);
}

#[test]
fn fst_m64_edge_cases() {
    // -Infinity passthrough
    let (_, bus) = run_x87_mem(
        &[0xDD, 0x16, 0x00, 0x00],
        &[Fp80::NEG_INFINITY],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(f64::from_bits(read_mem_u64(&bus, 0)), f64::NEG_INFINITY);

    // Overflow
    let (r, bus) = run_x87_mem(
        &[0xDD, 0x16, 0x00, 0x00],
        &[LARGEST_NORMAL],
        CW_NEAREST_EXT,
        &[],
    );
    assert!(f64::from_bits(read_mem_u64(&bus, 0)).is_infinite());
    assert!(r.oe());

    // Precision loss
    let precise = Fp80::from_bits(0x3FFF, 0x8000_0000_0000_0001);
    let (r, _) = run_x87_mem(&[0xDD, 0x16, 0x00, 0x00], &[precise], CW_NEAREST_EXT, &[]);
    assert!(r.pe());
}

#[test]
fn fst_m64_underflow() {
    let (r, bus) = run_x87_mem(
        &[0xDD, 0x16, 0x00, 0x00],
        &[TINY_NORMAL],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(f64::from_bits(read_mem_u64(&bus, 0)), 0.0f64);
    assert!(r.ue());
}

#[test]
fn fst_m64_snan() {
    let (r, bus) = run_x87_mem(
        &[0xDD, 0x16, 0x00, 0x00],
        &[POSITIVE_SNAN],
        CW_NEAREST_EXT,
        &[],
    );
    assert!(f64::from_bits(read_mem_u64(&bus, 0)).is_nan());
    assert!(r.ie());
}

#[test]
fn fst_m64_rounding_modes() {
    // Value not exactly representable: 1.0 + 2^-63
    let precise = Fp80::from_bits(0x3FFF, 0x8000_0000_0000_0001);

    let (r, _) = run_x87_mem(&[0xDD, 0x16, 0x00, 0x00], &[precise], CW_UP_EXT, &[]);
    assert!(r.pe());

    let (r, _) = run_x87_mem(&[0xDD, 0x16, 0x00, 0x00], &[precise], CW_DOWN_EXT, &[]);
    assert!(r.pe());

    let (r, _) = run_x87_mem(&[0xDD, 0x16, 0x00, 0x00], &[precise], CW_ZERO_EXT, &[]);
    assert!(r.pe());
}

#[test]
fn fbld_basic() {
    // FBLD = DF 26 00 00
    // BCD zero
    let (r, _) = run_x87_mem(&[0xDF, 0x26, 0x00, 0x00], &[], CW_NEAREST_EXT, &[0; 10]);
    assert_eq!(r.st(0), Fp80::ZERO);

    // BCD for 1
    let mut bcd = [0u8; 10];
    bcd[0] = 0x01;
    let (r, _) = run_x87_mem(&[0xDF, 0x26, 0x00, 0x00], &[], CW_NEAREST_EXT, &bcd);
    assert_eq!(r.st(0), Fp80::ONE);
}

#[test]
fn fbld_edge_cases() {
    // Negative BCD
    let mut bcd = [0u8; 10];
    bcd[0] = 0x01;
    bcd[9] = 0x80;
    let (r, _) = run_x87_mem(&[0xDF, 0x26, 0x00, 0x00], &[], CW_NEAREST_EXT, &bcd);
    assert!(r.st(0).sign());

    // Max BCD: 999,999,999,999,999,999
    let max_bcd = [0x99, 0x99, 0x99, 0x99, 0x99, 0x99, 0x99, 0x99, 0x99, 0x00];
    let (r, _) = run_x87_mem(&[0xDF, 0x26, 0x00, 0x00], &[], CW_NEAREST_EXT, &max_bcd);
    assert!(r.st(0).is_normal());
    assert!(!r.st(0).sign());
}

#[test]
fn fbld_negative_zero() {
    let mut bcd = [0u8; 10];
    bcd[9] = 0x80;
    let (r, _) = run_x87_mem(&[0xDF, 0x26, 0x00, 0x00], &[], CW_NEAREST_EXT, &bcd);
    assert!(r.st(0).is_zero());
}

#[test]
fn fbstp_basic() {
    // FBSTP = DF 36 00 00
    // 0 -> BCD zero
    let (_, bus) = run_x87_mem(
        &[0xDF, 0x36, 0x00, 0x00],
        &[Fp80::ZERO],
        CW_NEAREST_EXT,
        &[],
    );
    assert_eq!(read_mem_tbyte(&bus, 0), [0u8; 10]);

    // 1.0 -> BCD 1
    let (_, bus) = run_x87_mem(&[0xDF, 0x36, 0x00, 0x00], &[Fp80::ONE], CW_NEAREST_EXT, &[]);
    let bcd = read_mem_tbyte(&bus, 0);
    assert_eq!(bcd[0], 0x01);
    assert_eq!(bcd[9] & 0x80, 0); // positive
}

#[test]
fn fbstp_edge_cases() {
    // NaN -> IE + indefinite BCD
    let (r, bus) = run_x87_mem(
        &[0xDF, 0x36, 0x00, 0x00],
        &[Fp80::INDEFINITE],
        CW_NEAREST_EXT,
        &[],
    );
    assert!(r.ie());
    assert_eq!(
        read_mem_tbyte(&bus, 0),
        [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0xFF, 0xFF]
    );

    // Infinity -> IE + indefinite BCD
    let (r, bus) = run_x87_mem(
        &[0xDF, 0x36, 0x00, 0x00],
        &[Fp80::INFINITY],
        CW_NEAREST_EXT,
        &[],
    );
    assert!(r.ie());
    assert_eq!(
        read_mem_tbyte(&bus, 0),
        [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0xFF, 0xFF]
    );
}

#[test]
fn fbstp_fractional_rounds() {
    // 1.5 -> rounds to 2, PE
    let (r, bus) = run_x87_mem(
        &[0xDF, 0x36, 0x00, 0x00],
        &[ONE_AND_HALF],
        CW_NEAREST_EXT,
        &[],
    );
    let bcd = read_mem_tbyte(&bus, 0);
    assert_eq!(bcd[0], 0x02);
    assert!(r.pe());
}

#[test]
fn fbstp_negative() {
    // -1.0 -> BCD with sign bit
    let (_, bus) = run_x87_mem(
        &[0xDF, 0x36, 0x00, 0x00],
        &[NEGATIVE_ONE],
        CW_NEAREST_EXT,
        &[],
    );
    let bcd = read_mem_tbyte(&bus, 0);
    assert_eq!(bcd[0], 0x01);
    assert_eq!(bcd[9] & 0x80, 0x80);
}

#[test]
fn fbstp_out_of_range() {
    let huge = Fp80::from_bits(0x7FFE, 0xFFFF_FFFF_FFFF_FFFF);
    let (r, bus) = run_x87_mem(&[0xDF, 0x36, 0x00, 0x00], &[huge], CW_NEAREST_EXT, &[]);
    assert!(r.ie());
    assert_eq!(
        read_mem_tbyte(&bus, 0),
        [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0xFF, 0xFF]
    );
}

#[test]
fn f2xm1_basic() {
    // F2XM1 = D9 F0
    // f2xm1(+0) = +0
    let r = run_x87(&[0xD9, 0xF0], &[Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::ZERO);

    // f2xm1(-0) = -0
    let r = run_x87(&[0xD9, 0xF0], &[Fp80::NEG_ZERO]);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);
}

#[test]
fn f2xm1_edge_cases() {
    // f2xm1(-1.0) = -0.5 + PE
    let r = run_x87(&[0xD9, 0xF0], &[NEGATIVE_ONE]);
    assert_eq!(r.st(0), NEGATIVE_HALF);
    assert!(r.pe());

    // f2xm1(+1.0) = +1.0 + PE
    let r = run_x87(&[0xD9, 0xF0], &[Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::ONE);
    assert!(r.pe());

    // SNaN -> QNaN + IE
    let r = run_x87(&[0xD9, 0xF0], &[POSITIVE_SNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());

    // QNaN -> QNaN, no IE
    let r = run_x87(&[0xD9, 0xF0], &[POSITIVE_QNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(!r.ie());
}

#[test]
fn f2xm1_intermediate_values() {
    // f2xm1(0.5) = sqrt(2) - 1 ≈ 0.4142
    let r = run_x87(&[0xD9, 0xF0], &[HALF]);
    assert!(!r.st(0).is_zero());
    assert!(!r.st(0).is_negative());
    assert!(r.pe());

    // f2xm1(-0.5) = 1/sqrt(2) - 1 ≈ -0.2929
    let r = run_x87(&[0xD9, 0xF0], &[NEGATIVE_HALF]);
    assert!(!r.st(0).is_zero());
    assert!(r.st(0).is_negative());
    assert!(r.pe());
}

#[test]
fn fyl2x_basic() {
    // FYL2X = D9 F1. Stack: ST(0)=x, ST(1)=y. Computes y*log2(x). Pops, result in new ST(0).
    // fyl2x(x=2, y=1) = 1 * log2(2) = 1.0
    let r = run_x87(&[0xD9, 0xF1], &[TWO, Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::ONE);

    // fyl2x(x=1, y=1) = 1 * log2(1) = 0
    let r = run_x87(&[0xD9, 0xF1], &[Fp80::ONE, Fp80::ONE]);
    assert!(r.st(0).is_zero());
}

#[test]
fn fyl2x_edge_cases() {
    // x < 0 -> Indefinite + IE
    let r = run_x87(&[0xD9, 0xF1], &[NEGATIVE_ONE, Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // x = 0, y = 0 -> Indefinite + IE
    let r = run_x87(&[0xD9, 0xF1], &[Fp80::ZERO, Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // x = 0, y = positive -> -inf + ZE
    let r = run_x87(&[0xD9, 0xF1], &[Fp80::ZERO, Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::NEG_INFINITY);
    assert!(r.ze());

    // x = 0, y = negative -> +inf + ZE
    let r = run_x87(&[0xD9, 0xF1], &[Fp80::ZERO, NEGATIVE_ONE]);
    assert_eq!(r.st(0), Fp80::INFINITY);
    assert!(r.ze());

    // x = +inf, y = 0 -> Indefinite + IE
    let r = run_x87(&[0xD9, 0xF1], &[Fp80::INFINITY, Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // x = 1, y = inf -> Indefinite + IE (0 * inf)
    let r = run_x87(&[0xD9, 0xF1], &[Fp80::ONE, Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // x = +inf, y = positive -> +inf
    let r = run_x87(&[0xD9, 0xF1], &[Fp80::INFINITY, Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::INFINITY);

    // x = +inf, y = negative -> -inf
    let r = run_x87(&[0xD9, 0xF1], &[Fp80::INFINITY, NEGATIVE_ONE]);
    assert_eq!(r.st(0), Fp80::NEG_INFINITY);

    // SNaN propagation
    let r = run_x87(&[0xD9, 0xF1], &[POSITIVE_SNAN, Fp80::ONE]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());

    let r = run_x87(&[0xD9, 0xF1], &[Fp80::ONE, POSITIVE_SNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());

    // QNaN propagation: no IE
    let r = run_x87(&[0xD9, 0xF1], &[POSITIVE_QNAN, Fp80::ONE]);
    assert!(r.st(0).is_quiet_nan());
    assert!(!r.ie());

    let r = run_x87(&[0xD9, 0xF1], &[Fp80::ONE, POSITIVE_QNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(!r.ie());
}

#[test]
fn fyl2x_powers_of_two() {
    // fyl2x(x=4, y=1) = log2(4) = 2.0
    let r = run_x87(&[0xD9, 0xF1], &[FOUR, Fp80::ONE]);
    assert_eq!(r.st(0), TWO);

    // fyl2x(x=0.5, y=1) = log2(0.5) = -1.0
    let r = run_x87(&[0xD9, 0xF1], &[HALF, Fp80::ONE]);
    assert_eq!(r.st(0), NEGATIVE_ONE);

    // fyl2x(x=8, y=1) = log2(8) = 3.0
    let eight = Fp80::from_bits(0x4002, 0x8000_0000_0000_0000);
    let r = run_x87(&[0xD9, 0xF1], &[eight, Fp80::ONE]);
    assert_eq!(r.st(0), THREE);
}

#[test]
fn fyl2x_non_power_of_two() {
    // fyl2x(x=3, y=1) = log2(3) ≈ 1.585, in [1.0, 2.0)
    let r = run_x87(&[0xD9, 0xF1], &[THREE, Fp80::ONE]);
    assert!(!r.st(0).is_zero());
    assert!(!r.st(0).is_negative());

    // fyl2x(x=1.5, y=1) = log2(1.5) ≈ 0.585, in [0.5, 1.0)
    let r = run_x87(&[0xD9, 0xF1], &[ONE_AND_HALF, Fp80::ONE]);
    assert!(!r.st(0).is_zero());
    assert!(!r.st(0).is_negative());
}

#[test]
fn fyl2xp1_basic() {
    // FYL2XP1 = D9 F9. Stack: ST(0)=x, ST(1)=y.
    // fyl2xp1(x=+0, y=ONE) = +0
    let r = run_x87(&[0xD9, 0xF9], &[Fp80::ZERO, Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::ZERO);
    assert!(!r.st(0).sign());

    // fyl2xp1(x=-0, y=ONE) = -0
    let r = run_x87(&[0xD9, 0xF9], &[Fp80::NEG_ZERO, Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);
}

#[test]
fn fyl2xp1_edge_cases() {
    // fyl2xp1(x=+0, y=negative) = -0
    let r = run_x87(&[0xD9, 0xF9], &[Fp80::ZERO, NEGATIVE_ONE]);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);

    // fyl2xp1(x=-0, y=negative) = +0
    let r = run_x87(&[0xD9, 0xF9], &[Fp80::NEG_ZERO, NEGATIVE_ONE]);
    assert_eq!(r.st(0), Fp80::ZERO);
    assert!(!r.st(0).sign());

    // SNaN propagation
    let r = run_x87(&[0xD9, 0xF9], &[POSITIVE_SNAN, Fp80::ONE]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());

    let r = run_x87(&[0xD9, 0xF9], &[Fp80::ZERO, POSITIVE_SNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());

    // QNaN propagation: no IE
    let r = run_x87(&[0xD9, 0xF9], &[POSITIVE_QNAN, Fp80::ONE]);
    assert!(r.st(0).is_quiet_nan());
    assert!(!r.ie());

    let r = run_x87(&[0xD9, 0xF9], &[Fp80::ZERO, POSITIVE_QNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(!r.ie());
}

#[test]
fn fsin_basic() {
    // FSIN = D9 FE
    // sin(+0) = +0
    let r = run_x87(&[0xD9, 0xFE], &[Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::ZERO);
    assert!(!r.c2()); // not out of range

    // sin(-0) = -0
    let r = run_x87(&[0xD9, 0xFE], &[Fp80::NEG_ZERO]);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);
    assert!(!r.c2());
}

#[test]
fn fsin_edge_cases() {
    // sin(+inf) = Indefinite + IE
    let r = run_x87(&[0xD9, 0xFE], &[Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // sin(-inf) = Indefinite + IE
    let r = run_x87(&[0xD9, 0xFE], &[Fp80::NEG_INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // |x| >= 2^63 -> C2=1, x unchanged
    let r = run_x87(&[0xD9, 0xFE], &[OUT_OF_RANGE]);
    assert_eq!(r.st(0), OUT_OF_RANGE);
    assert!(r.c2());

    // SNaN -> QNaN + IE
    let r = run_x87(&[0xD9, 0xFE], &[POSITIVE_SNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());

    // QNaN -> QNaN, no IE
    let r = run_x87(&[0xD9, 0xFE], &[POSITIVE_QNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(!r.ie());
}

#[test]
fn fcos_basic() {
    // FCOS = D9 FF
    // cos(+0) = +1.0
    let r = run_x87(&[0xD9, 0xFF], &[Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::ONE);
    assert!(!r.c2());

    // cos(-0) = +1.0
    let r = run_x87(&[0xD9, 0xFF], &[Fp80::NEG_ZERO]);
    assert_eq!(r.st(0), Fp80::ONE);
    assert!(!r.c2());
}

#[test]
fn fcos_edge_cases() {
    // cos(+inf) = Indefinite + IE
    let r = run_x87(&[0xD9, 0xFF], &[Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // cos(-inf) = Indefinite + IE
    let r = run_x87(&[0xD9, 0xFF], &[Fp80::NEG_INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // Out of range
    let r = run_x87(&[0xD9, 0xFF], &[OUT_OF_RANGE]);
    assert_eq!(r.st(0), OUT_OF_RANGE);
    assert!(r.c2());

    // SNaN -> QNaN + IE
    let r = run_x87(&[0xD9, 0xFF], &[POSITIVE_SNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());

    // QNaN -> QNaN, no IE
    let r = run_x87(&[0xD9, 0xFF], &[POSITIVE_QNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(!r.ie());
}

#[test]
fn fsincos_basic() {
    // FSINCOS = D9 FB
    // After: ST(0)=cos, ST(1)=sin (writes sin to ST(0), then pushes cos)
    // fsincos(+0): sin=+0, cos=+1.0
    let r = run_x87(&[0xD9, 0xFB], &[Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::ONE); // cos
    assert_eq!(r.st(1), Fp80::ZERO); // sin
    assert!(!r.c2());

    // fsincos(-0): sin=-0, cos=+1.0
    let r = run_x87(&[0xD9, 0xFB], &[Fp80::NEG_ZERO]);
    assert_eq!(r.st(0), Fp80::ONE);
    assert_eq!(r.st(1), Fp80::NEG_ZERO);
    assert!(!r.c2());
}

#[test]
fn fsincos_edge_cases() {
    // fsincos(+inf) -> Indefinite + IE
    let r = run_x87(&[0xD9, 0xFB], &[Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // Out of range: C2=1, no push
    let r = run_x87(&[0xD9, 0xFB], &[OUT_OF_RANGE]);
    assert_eq!(r.st(0), OUT_OF_RANGE);
    assert!(r.c2());

    // SNaN -> QNaN + IE
    let r = run_x87(&[0xD9, 0xFB], &[POSITIVE_SNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());

    // fsincos(-inf) -> Indefinite + IE
    let r = run_x87(&[0xD9, 0xFB], &[Fp80::NEG_INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // QNaN -> QNaN, no IE (both sin and cos are QNaN)
    let r = run_x87(&[0xD9, 0xFB], &[POSITIVE_QNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.st(1).is_quiet_nan());
    assert!(!r.ie());
}

#[test]
fn fptan_basic() {
    // FPTAN = D9 F2
    // After: writes tan to ST(0), pushes 1.0. So ST(0)=1.0, ST(1)=tan(x)
    // fptan(+0): tan=+0
    let r = run_x87(&[0xD9, 0xF2], &[Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::ONE);
    assert_eq!(r.st(1), Fp80::ZERO);
    assert!(!r.c2());

    // fptan(-0): tan=-0
    let r = run_x87(&[0xD9, 0xF2], &[Fp80::NEG_ZERO]);
    assert_eq!(r.st(0), Fp80::ONE);
    assert_eq!(r.st(1), Fp80::NEG_ZERO);
    assert!(!r.c2());
}

#[test]
fn fptan_edge_cases() {
    // fptan(+inf) = Indefinite + IE. FPTAN writes result to ST(0), then pushes 1.0.
    // After: ST(0)=1.0, ST(1)=Indefinite.
    let r = run_x87(&[0xD9, 0xF2], &[Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::ONE);
    assert_eq!(r.st(1), Fp80::INDEFINITE);
    assert!(r.ie());

    // fptan(-inf) = Indefinite + IE
    let r = run_x87(&[0xD9, 0xF2], &[Fp80::NEG_INFINITY]);
    assert_eq!(r.st(0), Fp80::ONE);
    assert_eq!(r.st(1), Fp80::INDEFINITE);
    assert!(r.ie());

    // Out of range: C2=1, no push, ST(0) unchanged
    let r = run_x87(&[0xD9, 0xF2], &[OUT_OF_RANGE]);
    assert_eq!(r.st(0), OUT_OF_RANGE);
    assert!(r.c2());

    // SNaN -> QNaN + IE. Result in ST(1), 1.0 in ST(0).
    let r = run_x87(&[0xD9, 0xF2], &[POSITIVE_SNAN]);
    assert_eq!(r.st(0), Fp80::ONE);
    assert!(r.st(1).is_quiet_nan());
    assert!(r.ie());

    let r = run_x87(&[0xD9, 0xF2], &[POSITIVE_QNAN]);
    assert_eq!(r.st(0), Fp80::ONE);
    assert!(r.st(1).is_quiet_nan());
    assert!(!r.ie());
}

#[test]
fn fpatan_basic() {
    // FPATAN = D9 F3. ST(0)=x, ST(1)=y. Computes atan2(y,x). Result in ST(1), pop.
    // fpatan(y=+0, x=+ONE) = +0
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::ONE, Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::ZERO);
    assert!(!r.st(0).sign());

    // fpatan(y=-0, x=+ONE) = -0
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::ONE, Fp80::NEG_ZERO]);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);
}

#[test]
fn fpatan_edge_cases() {
    // fpatan(y=+0, x=-ONE) = +pi
    let r = run_x87(&[0xD9, 0xF3], &[NEGATIVE_ONE, Fp80::ZERO]);
    assert!(r.st(0).is_normal());
    assert!(!r.st(0).sign());

    // fpatan(y=-0, x=-ONE) = -pi
    let r = run_x87(&[0xD9, 0xF3], &[NEGATIVE_ONE, Fp80::NEG_ZERO]);
    assert!(r.st(0).is_normal());
    assert!(r.st(0).sign());

    // fpatan(y=+ONE, x=+0) = +pi/2
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::ZERO, Fp80::ONE]);
    assert!(r.st(0).is_normal());
    assert!(!r.st(0).sign());

    // fpatan(y=-ONE, x=+0) = -pi/2
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::ZERO, NEGATIVE_ONE]);
    assert!(r.st(0).is_normal());
    assert!(r.st(0).sign());

    // fpatan(y=+inf, x=+inf) = +pi/4
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::INFINITY, Fp80::INFINITY]);
    assert!(r.st(0).is_normal());
    assert!(!r.st(0).sign());

    // fpatan(y=+inf, x=-inf) = +3pi/4
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::NEG_INFINITY, Fp80::INFINITY]);
    assert!(r.st(0).is_normal());
    assert!(!r.st(0).sign());

    // fpatan(y=-inf, x=+inf) = -pi/4
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::INFINITY, Fp80::NEG_INFINITY]);
    assert!(r.st(0).is_normal());
    assert!(r.st(0).sign());

    // fpatan(y=-inf, x=-inf) = -3pi/4
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::NEG_INFINITY, Fp80::NEG_INFINITY]);
    assert!(r.st(0).is_normal());
    assert!(r.st(0).sign());

    // fpatan(y=+inf, x=finite) = +pi/2
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::ONE, Fp80::INFINITY]);
    assert!(r.st(0).is_normal());
    assert!(!r.st(0).sign());

    // fpatan(y=-inf, x=finite) = -pi/2
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::ONE, Fp80::NEG_INFINITY]);
    assert!(r.st(0).is_normal());
    assert!(r.st(0).sign());

    // fpatan(y=finite+, x=+inf) = +0
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::INFINITY, Fp80::ONE]);
    assert!(r.st(0).is_zero());
    assert!(!r.st(0).sign());

    // fpatan(y=finite-, x=+inf) = -0
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::INFINITY, NEGATIVE_ONE]);
    assert!(r.st(0).is_zero());
    assert!(r.st(0).sign());

    // fpatan(y=finite+, x=-inf) = +pi
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::NEG_INFINITY, Fp80::ONE]);
    assert!(r.st(0).is_normal());
    assert!(!r.st(0).sign());

    // fpatan(y=finite-, x=-inf) = -pi
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::NEG_INFINITY, NEGATIVE_ONE]);
    assert!(r.st(0).is_normal());
    assert!(r.st(0).sign());

    // SNaN propagation
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::ONE, POSITIVE_SNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());

    let r = run_x87(&[0xD9, 0xF3], &[POSITIVE_SNAN, Fp80::ONE]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());

    // QNaN propagation, no IE
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::ONE, POSITIVE_QNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(!r.ie());

    let r = run_x87(&[0xD9, 0xF3], &[POSITIVE_QNAN, Fp80::ONE]);
    assert!(r.st(0).is_quiet_nan());
    assert!(!r.ie());

    // fpatan(y=+ONE, x=-0) = +pi/2
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::NEG_ZERO, Fp80::ONE]);
    assert!(r.st(0).is_normal());
    assert!(!r.st(0).sign());

    // fpatan(y=-ONE, x=-0) = -pi/2
    let r = run_x87(&[0xD9, 0xF3], &[Fp80::NEG_ZERO, NEGATIVE_ONE]);
    assert!(r.st(0).is_normal());
    assert!(r.st(0).sign());
}

#[test]
fn fscale_basic() {
    // FSCALE = D9 FD. ST(0)=value, ST(1)=scale. Result = ST(0) * 2^floor(ST(1)).
    // 1.0 * 2^1 = 2.0
    let r = run_x87(&[0xD9, 0xFD], &[Fp80::ONE, Fp80::ONE]);
    assert_eq!(r.st(0), TWO);
    assert!(r.no_exceptions());

    // 1.0 * 2^0 = 1.0
    let r = run_x87(&[0xD9, 0xFD], &[Fp80::ONE, Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::ONE);

    // 1.0 * 2^2 = 4.0
    let r = run_x87(&[0xD9, 0xFD], &[Fp80::ONE, TWO]);
    assert_eq!(r.st(0), FOUR);
}

#[test]
fn fscale_edge_cases() {
    // +0 * 2^(+inf) = Indefinite + IE
    let r = run_x87(&[0xD9, 0xFD], &[Fp80::ZERO, Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // +inf * 2^(-inf) = Indefinite + IE
    let r = run_x87(&[0xD9, 0xFD], &[Fp80::INFINITY, Fp80::NEG_INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // +0 * 2^(-inf) = +0
    let r = run_x87(&[0xD9, 0xFD], &[Fp80::ZERO, Fp80::NEG_INFINITY]);
    assert_eq!(r.st(0), Fp80::ZERO);

    // +inf * 2^(+inf) = +inf
    let r = run_x87(&[0xD9, 0xFD], &[Fp80::INFINITY, Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::INFINITY);

    // SNaN propagation
    let r = run_x87(&[0xD9, 0xFD], &[POSITIVE_SNAN, Fp80::ONE]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());

    // QNaN propagation, no IE
    let r = run_x87(&[0xD9, 0xFD], &[POSITIVE_QNAN, Fp80::ONE]);
    assert!(r.st(0).is_quiet_nan());
    assert!(!r.ie());

    // 0 * 2^(finite) = 0 with sign preserved
    let r = run_x87(&[0xD9, 0xFD], &[Fp80::NEG_ZERO, TWO]);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);
}

#[test]
fn fscale_non_integer_truncation() {
    // scale(1.0, 1.5) -> 1.0 * 2^floor(1.5) = 1.0 * 2^1 = 2.0
    let r = run_x87(&[0xD9, 0xFD], &[Fp80::ONE, ONE_AND_HALF]);
    assert_eq!(r.st(0), TWO);
}

#[test]
fn fscale_negative() {
    // scale(1.0, -1) = 1.0 * 2^(-1) = 0.5
    let r = run_x87(&[0xD9, 0xFD], &[Fp80::ONE, NEGATIVE_ONE]);
    assert_eq!(r.st(0), HALF);
}

#[test]
fn fscale_neg_inf_cases() {
    // -inf * 2^(-inf) = Indefinite + IE
    let r = run_x87(&[0xD9, 0xFD], &[Fp80::NEG_INFINITY, Fp80::NEG_INFINITY]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // -inf * 2^(+inf) = -inf
    let r = run_x87(&[0xD9, 0xFD], &[Fp80::NEG_INFINITY, Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::NEG_INFINITY);
}

#[test]
fn fscale_nan_in_scale_param() {
    // SNaN in scale -> QNaN + IE
    let r = run_x87(&[0xD9, 0xFD], &[Fp80::ONE, POSITIVE_SNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.ie());

    // QNaN in scale -> QNaN, no IE
    let r = run_x87(&[0xD9, 0xFD], &[Fp80::ONE, POSITIVE_QNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(!r.ie());
}

#[test]
fn fxtract_basic() {
    // FXTRACT = D9 F4
    // extract(1.0): significand=1.0, exponent=0.0
    let r = run_x87(&[0xD9, 0xF4], &[Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::ONE); // significand
    assert_eq!(r.st(1), Fp80::ZERO); // exponent

    // extract(4.0): significand=1.0, exponent=2.0
    let r = run_x87(&[0xD9, 0xF4], &[FOUR]);
    assert_eq!(r.st(0), Fp80::ONE); // significand
    assert_eq!(r.st(1), TWO); // exponent
}

#[test]
fn fxtract_edge_cases() {
    // extract(+0) = (sig=+0, exp=-inf) + ZE
    let r = run_x87(&[0xD9, 0xF4], &[Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::ZERO); // significand
    assert_eq!(r.st(1), Fp80::NEG_INFINITY); // exponent
    assert!(r.ze());

    // extract(-0) = (sig=-0, exp=-inf) + ZE
    let r = run_x87(&[0xD9, 0xF4], &[Fp80::NEG_ZERO]);
    assert_eq!(r.st(0), Fp80::NEG_ZERO);
    assert_eq!(r.st(1), Fp80::NEG_INFINITY);
    assert!(r.ze());

    // extract(+inf) = (sig=+inf, exp=+inf)
    let r = run_x87(&[0xD9, 0xF4], &[Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::INFINITY);
    assert_eq!(r.st(1), Fp80::INFINITY);

    // extract(-inf) = (sig=-inf, exp=+inf)
    let r = run_x87(&[0xD9, 0xF4], &[Fp80::NEG_INFINITY]);
    assert_eq!(r.st(0), Fp80::NEG_INFINITY);
    assert_eq!(r.st(1), Fp80::INFINITY);

    // extract(SNaN) -> NaN, IE
    let r = run_x87(&[0xD9, 0xF4], &[POSITIVE_SNAN]);
    assert!(r.st(0).is_nan());
    assert!(r.st(1).is_nan());
    assert!(r.ie());
}

#[test]
fn fxtract_negative_normal() {
    // extract(-4.0): significand=-1.0, exponent=2.0
    let r = run_x87(&[0xD9, 0xF4], &[NEGATIVE_FOUR]);
    assert!(r.st(0).sign());
    assert_eq!(r.st(0).exponent(), 0x3FFF);
    assert_eq!(r.st(1), TWO);
}

#[test]
fn fxtract_qnan() {
    // extract(QNaN) = (QNaN, QNaN), no IE
    let r = run_x87(&[0xD9, 0xF4], &[POSITIVE_QNAN]);
    assert!(r.st(0).is_quiet_nan());
    assert!(r.st(1).is_quiet_nan());
    assert!(!r.ie());
}

#[test]
fn fprem_basic() {
    // FPREM = D9 F8. ST(0)=dividend, ST(1)=divisor.
    // 5 % 3 = 2, quotient=1
    let r = run_x87(&[0xD9, 0xF8], &[FIVE, THREE]);
    assert_eq!(r.st(0), TWO);
    assert!(!r.c2()); // complete
    // quotient bits: Q2=C0, Q1=C3, Q0=C1. quotient=1 -> C0=0, C3=0, C1=1
    assert!(!r.c0());
    assert!(!r.c3());
    assert!(r.c1());

    // 7 % 4 = 3, quotient=1
    let r = run_x87(&[0xD9, 0xF8], &[SEVEN, FOUR]);
    assert_eq!(r.st(0), THREE);
    assert!(!r.c2());
    assert!(!r.c0());
    assert!(!r.c3());
    assert!(r.c1());
}

#[test]
fn fprem_edge_cases() {
    // inf % any = Indefinite + IE
    let r = run_x87(&[0xD9, 0xF8], &[Fp80::INFINITY, Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // any % 0 = Indefinite + IE
    let r = run_x87(&[0xD9, 0xF8], &[Fp80::ONE, Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // 0 % nonzero = +0
    let r = run_x87(&[0xD9, 0xF8], &[Fp80::ZERO, Fp80::ONE]);
    assert!(r.st(0).is_zero());
    assert!(!r.c2());

    // finite % inf = finite (unchanged)
    let r = run_x87(&[0xD9, 0xF8], &[Fp80::ONE, Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::ONE);
    assert!(!r.c2());

    // SNaN propagation
    let r = run_x87(&[0xD9, 0xF8], &[POSITIVE_SNAN, Fp80::ONE]);
    assert!(r.st(0).is_nan());
    assert!(r.ie());
}

#[test]
fn fprem_incomplete() {
    // Large exponent difference -> C2=1 (incomplete)
    let r = run_x87(&[0xD9, 0xF8], &[TWO_POW_100, Fp80::ONE]);
    assert!(r.c2());
}

#[test]
fn fprem_negative_operand() {
    // (-7) % 4 = -3, quotient=1
    let r = run_x87(&[0xD9, 0xF8], &[NEGATIVE_SEVEN, FOUR]);
    assert_eq!(r.st(0), NEGATIVE_THREE);
    assert!(!r.c2());
    assert!(!r.c0());
    assert!(!r.c3());
    assert!(r.c1());
}

#[test]
fn fprem_quotient_bits() {
    // 7 % 2 = 1, quotient=3. Q2=0, Q1=1, Q0=1 -> C0=0, C3=1, C1=1
    let r = run_x87(&[0xD9, 0xF8], &[SEVEN, TWO]);
    assert_eq!(r.st(0), Fp80::ONE);
    assert!(!r.c0()); // Q2=0
    assert!(r.c3()); // Q1=1
    assert!(r.c1()); // Q0=1
    assert!(!r.c2()); // complete
}

#[test]
fn fprem_qnan() {
    // QNaN % anything = QNaN, no IE
    let r = run_x87(&[0xD9, 0xF8], &[POSITIVE_QNAN, Fp80::ONE]);
    assert!(r.st(0).is_quiet_nan());
    assert!(!r.ie());
}

#[test]
fn fprem1_basic() {
    // FPREM1 = D9 F5. IEEE remainder.
    // 5 FPREM1 3: round_nearest(5/3)=2. rem = 5 - 2*3 = -1. quotient=2.
    let r = run_x87(&[0xD9, 0xF5], &[FIVE, THREE]);
    assert_eq!(r.st(0), NEGATIVE_ONE);
    assert!(!r.c2());
    // quotient=2 -> Q2=0, Q1=1, Q0=0 -> C0=0, C3=1, C1=0
    assert!(!r.c0());
    assert!(r.c3());
    assert!(!r.c1());

    // 7 FPREM1 4: round_nearest(7/4)=2. rem = 7 - 2*4 = -1. quotient=2.
    let r = run_x87(&[0xD9, 0xF5], &[SEVEN, FOUR]);
    assert_eq!(r.st(0), NEGATIVE_ONE);
    assert!(!r.c2());
    assert!(!r.c0());
    assert!(r.c3());
    assert!(!r.c1());
}

#[test]
fn fprem1_edge_cases() {
    // inf % any = Indefinite + IE
    let r = run_x87(&[0xD9, 0xF5], &[Fp80::INFINITY, Fp80::ONE]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // any % 0 = Indefinite + IE
    let r = run_x87(&[0xD9, 0xF5], &[Fp80::ONE, Fp80::ZERO]);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());

    // 0 % nonzero = +0
    let r = run_x87(&[0xD9, 0xF5], &[Fp80::ZERO, Fp80::ONE]);
    assert!(r.st(0).is_zero());
    assert!(!r.c2());

    // finite % inf = finite (unchanged)
    let r = run_x87(&[0xD9, 0xF5], &[Fp80::ONE, Fp80::INFINITY]);
    assert_eq!(r.st(0), Fp80::ONE);
    assert!(!r.c2());
}

#[test]
fn fprem1_halfway() {
    // 3 % 2: round_nearest(3/2)=2 (round to even). rem = 3 - 2*2 = -1. quotient=2.
    let r = run_x87(&[0xD9, 0xF5], &[THREE, TWO]);
    assert_eq!(r.st(0), NEGATIVE_ONE);
    assert!(!r.c2());
    assert!(!r.c0());
    assert!(r.c3());
    assert!(!r.c1());
}

#[test]
fn fprem1_qnan() {
    // QNaN % anything = QNaN, no IE
    let r = run_x87(&[0xD9, 0xF5], &[POSITIVE_QNAN, Fp80::ONE]);
    assert!(r.st(0).is_quiet_nan());
    assert!(!r.ie());
}

#[test]
fn fprem1_snan() {
    // SNaN propagation
    let r = run_x87(&[0xD9, 0xF5], &[POSITIVE_SNAN, Fp80::ONE]);
    assert!(r.st(0).is_nan());
    assert!(r.ie());
}
