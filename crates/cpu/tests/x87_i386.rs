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

/// tan(0.5) rounded at 64 bits down, the exact value of the function with 320-bit arithmetic.
const TAN_HALF_DOWN: Fp80 = Fp80::from_bits(0x3FFE, 0x8BDA_7ADF_9A3A_5218);
/// tan(0.5) rounded at 64 bits up, the exact value of the function with 320-bit arithmetic.
const TAN_HALF_UP: Fp80 = Fp80::from_bits(0x3FFE, 0x8BDA_7ADF_9A3A_5219);

/// FPTAN rounds the tangent in the mode RC selects and pushes 1.0. RC applies to FPTAN
/// (80486 PRM 15.2.3 and table 16-1) and PC does not (80486 PRM 15.2.4).
#[test]
fn fptan_follows_the_rounding_control() {
    let expectations = [
        (CW_NEAREST_EXT, TAN_HALF_UP, true),
        (CW_DOWN_EXT, TAN_HALF_DOWN, false),
        (CW_UP_EXT, TAN_HALF_UP, true),
        (CW_ZERO_EXT, TAN_HALF_DOWN, false),
        (CW_NEAREST_SGL, TAN_HALF_UP, true),
    ];
    for (control_word, tangent, rounded_up) in expectations {
        let r = run_x87_with_cw(&[0xD9, 0xF2], &[HALF], control_word);
        assert_eq!(r.st(0), Fp80::ONE, "{control_word:04X}");
        assert_eq!(r.st(1), tangent, "{control_word:04X}");
        assert_eq!(r.c1(), rounded_up, "{control_word:04X}");
        assert!(r.pe() && !r.c2(), "{control_word:04X}");
    }
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

/// The significand of pi, the dividend significand of the remainder guest loop tests.
const PI_SIGNIFICAND: u64 = 0xC90F_DAA2_2168_C234;
/// The second opcode byte of FPREM (D9 F8).
const FPREM_OPCODE: u8 = 0xF8;
/// The second opcode byte of FPREM1 (D9 F5).
const FPREM1_OPCODE: u8 = 0xF5;
/// The offset of the HLT behind the remainder guest loop.
const REMAINDER_LOOP_END: u32 = 7;
/// The most remainder executions a guest loop test allows.
const REMAINDER_LOOP_LIMIT: u32 = 1000;
/// The C0, C1 and C3 bits of the status word.
const QUOTIENT_CONDITION_BITS: u16 = 0x4300;

/// Runs the guest loop `L: FPREM or FPREM1; FNSTSW AX; SAHF; JP L; HLT` with ST(0) =
/// `dividend` and ST(1) = `divisor` until it reaches the HLT. SAHF copies C2 into PF.
/// Returns the number of remainder executions and the final x87 state.
fn run_remainder_loop(opcode: u8, dividend: Fp80, divisor: Fp80) -> (u32, X87Result) {
    let code = [0xD9, opcode, 0xDF, 0xE0, 0x9E, 0x7A, 0xF9, 0xF4];
    let mut cpu = I386::<{ CPU_MODEL_486_DX }>::new();
    let mut bus = TestBus::new();
    place_code(&mut bus, 0x1000, 0x0000, &code);
    cpu.load_state(&build_state(&[dividend, divisor], CW_NEAREST_EXT));
    let mut count = 0;
    loop {
        match cpu.state.eip() {
            0 => {
                count += 1;
                assert!(
                    count <= REMAINDER_LOOP_LIMIT,
                    "the remainder loop does not end"
                );
            }
            REMAINDER_LOOP_END => return (count, extract_result(&cpu)),
            _ => {}
        }
        cpu.step(&mut bus);
    }
}

/// Returns the exponent field of `value` with a field of 0 read as 1.
fn effective_exponent(value: Fp80) -> i32 {
    i32::from((value.exponent() & 0x7FFF).max(1))
}

/// Returns |`dividend`| modulo `modulus` in units of the lowest significand bit of
/// `divisor`. The dividend exponent must not be below the divisor exponent.
fn dividend_modulo(dividend: Fp80, divisor: Fp80, modulus: u128) -> u128 {
    let mut value = u128::from(dividend.significand()) % modulus;
    for _ in 0..effective_exponent(dividend) - effective_exponent(divisor) {
        value = (value << 1) % modulus;
    }
    value
}

/// Returns the value `sign` x `magnitude` in units of the lowest significand bit of
/// `divisor`, with a zero magnitude giving a zero of that sign.
fn pack_at_divisor_scale(sign: bool, magnitude: u64, divisor: Fp80) -> Fp80 {
    let sign_bit = if sign { 0x8000 } else { 0x0000 };
    if magnitude == 0 {
        return Fp80::from_bits(sign_bit, 0);
    }
    let divisor_exponent = effective_exponent(divisor);
    let shift = (magnitude.leading_zeros() as i32).min(divisor_exponent - 1);
    let significand = magnitude << shift;
    let exponent = if significand >> 63 == 0 {
        0
    } else {
        divisor_exponent - shift
    };
    Fp80::from_bits(sign_bit | exponent as u16, significand)
}

/// Returns the exact remainder of the positive `dividend` divided by `divisor` with the
/// quotient chopped toward zero. The dividend exponent must not be below the divisor
/// exponent.
fn exact_remainder(dividend: Fp80, divisor: Fp80) -> Fp80 {
    let modulus = u128::from(divisor.significand());
    let remainder = dividend_modulo(dividend, divisor, modulus) as u64;
    pack_at_divisor_scale(false, remainder, divisor)
}

/// Returns the exact IEEE remainder of `dividend` divided by `divisor` with the quotient
/// rounded to the nearest even integer, and the C0, C1 and C3 bits of the quotient
/// magnitude: C0 = Q2, C3 = Q1 and C1 = Q0. A zero remainder takes the dividend sign. The
/// dividend exponent must not be below the divisor exponent.
fn exact_ieee_remainder(dividend: Fp80, divisor: Fp80) -> (Fp80, u16) {
    let divisor_significand = u128::from(divisor.significand());
    let modulo_eight = dividend_modulo(dividend, divisor, divisor_significand * 8);
    let mut quotient = (modulo_eight / divisor_significand) as u16;
    let mut remainder = modulo_eight % divisor_significand;
    let mut sign = dividend.sign();
    if remainder * 2 > divisor_significand
        || (remainder * 2 == divisor_significand && quotient & 1 != 0)
    {
        remainder = divisor_significand - remainder;
        quotient += 1;
        sign = !sign;
    }
    if remainder == 0 {
        sign = dividend.sign();
    }
    let condition_bits = ((quotient & 4) << 6) | ((quotient & 2) << 13) | ((quotient & 1) << 9);
    (
        pack_at_divisor_scale(sign, remainder as u64, divisor),
        condition_bits,
    )
}

/// Returns the remainder loop test operands: pi x 2^k by 3.0 with exponent differences of
/// 63 to 16000 and the expected FPREM count of each.
fn pi_by_three_cases() -> Vec<(Fp80, Fp80, u32)> {
    [
        (63, 1),
        (64, 2),
        (100, 2),
        (127, 3),
        (128, 3),
        (200, 4),
        (1000, 16),
        (16000, 251),
    ]
    .into_iter()
    .map(|(exponent_difference, count)| {
        (
            Fp80::from_bits(0x4000 + exponent_difference, PI_SIGNIFICAND),
            THREE,
            count,
        )
    })
    .collect()
}

/// The FPREM guest loop reduces an exponent difference of 64 or more in steps of 32 to 63
/// bits and ends with the exact remainder and C2 clear (80486 PRM, FPREM).
#[test]
fn fprem_guest_loop_ends_with_the_exact_remainder() {
    let mut cases = pi_by_three_cases();
    cases.push((Fp80::from_bits(0x40C7, 0x8000_0000_0000_0000), Fp80::ONE, 2));
    cases.push((
        Fp80::ONE,
        Fp80::from_bits(0x0000, 0x8000_0000_0000_0001),
        261,
    ));
    for (dividend, divisor, expected_count) in cases {
        let (count, result) = run_remainder_loop(FPREM_OPCODE, dividend, divisor);
        let label = format!("{dividend:?} by {divisor:?}");
        assert_eq!(count, expected_count, "{label}");
        assert_eq!(result.st(0), exact_remainder(dividend, divisor), "{label}");
        assert!(!result.c2(), "{label}");
    }
}

/// The FPREM1 guest loop ends with the exact IEEE remainder, the dividend sign on a zero
/// and the quotient bits of the whole quotient in C0, C3 and C1 (80486 PRM, FPREM1).
#[test]
fn fprem1_guest_loop_ends_with_the_exact_remainder_and_quotient_bits() {
    let mut cases: Vec<(Fp80, Fp80)> = pi_by_three_cases()
        .into_iter()
        .flat_map(|(dividend, divisor, _)| {
            let negative_dividend = Fp80::from_bits(dividend.exponent() | 0x8000, PI_SIGNIFICAND);
            [(dividend, divisor), (negative_dividend, NEGATIVE_THREE)]
        })
        .collect();
    cases.push((
        Fp80::from_bits(0x61EB, 0xFA84_BABF_F680_42BC),
        Fp80::from_bits(0xAF11, 0x8000_0000_0000_0000),
    ));
    cases.push((Fp80::from_bits(0x40C7, 0x8000_0000_0000_0000), Fp80::ONE));
    for (dividend, divisor) in cases {
        let (_, result) = run_remainder_loop(FPREM1_OPCODE, dividend, divisor);
        let label = format!("{dividend:?} by {divisor:?}");
        let (expected, condition_bits) = exact_ieee_remainder(dividend, divisor);
        assert_eq!(result.st(0), expected, "{label}");
        assert_eq!(result.st(0).sign(), expected.sign(), "{label}");
        assert_eq!(
            result.status_word & QUOTIENT_CONDITION_BITS,
            condition_bits,
            "{label}"
        );
        assert!(!result.c2(), "{label}");
    }
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

/// Runs `steps` instructions of `code` at 1000:0000 with the x87 `stack` and the bytes
/// `memory` at DS:0000 (linear 20000h), after `configure` adjusted the initial state.
fn run_x87_program(
    code: &[u8],
    stack: &[Fp80],
    memory: &[u8],
    steps: usize,
    configure: impl FnOnce(&mut I386State, &mut TestBus),
) -> (I386<{ CPU_MODEL_486_DX }>, TestBus) {
    let mut cpu = I386::<{ CPU_MODEL_486_DX }>::new();
    let mut bus = TestBus::new();
    place_code(&mut bus, 0x1000, 0x0000, code);
    bus.ram[0x20000..0x20000 + memory.len()].copy_from_slice(memory);
    let mut state = build_state(stack, CW_NEAREST_EXT);
    configure(&mut state, &mut bus);
    cpu.load_state(&state);
    for _ in 0..steps {
        cpu.step(&mut bus);
    }
    (cpu, bus)
}

fn read_mem_u16(bus: &TestBus, offset: usize) -> u16 {
    read_mem_i16(bus, offset) as u16
}

/// Points the real mode interrupt vector `vector` at `segment:0000`.
fn set_interrupt_vector(bus: &mut TestBus, vector: usize, segment: u16) {
    bus.ram[vector * 4..vector * 4 + 4].copy_from_slice(&[
        0,
        0,
        segment as u8,
        (segment >> 8) as u8,
    ]);
}

const THREE_F64: [u8; 8] = 3.0f64.to_le_bytes();

#[test]
fn fdiv_records_the_data_pointer_as_offset_and_selector() {
    let mut memory = [0u8; 0x20];
    memory[0x10..0x18].copy_from_slice(&THREE_F64);
    let (cpu, _) = run_x87_program(
        &[0xDC, 0x36, 0x10, 0x00],
        &[Fp80::ONE],
        &memory,
        1,
        |_, _| {},
    );
    assert_eq!(cpu.state.fpu.fip_offset, 0x0000);
    assert_eq!(cpu.state.fpu.fip_selector, 0x1000);
    assert_eq!(cpu.state.fpu.fdp_offset, 0x0010);
    assert_eq!(cpu.state.fpu.fdp_selector, 0x2000);
    assert_eq!(cpu.state.fpu.fpu_opcode, 0x0436);
}

#[test]
fn fdiv_sets_c1_when_the_quotient_rounds_up() {
    let mut memory = [0u8; 0x20];
    memory[0x10..0x18].copy_from_slice(&THREE_F64);
    let (cpu, _) = run_x87_program(
        &[0xDC, 0x36, 0x10, 0x00],
        &[Fp80::ONE],
        &memory,
        1,
        |_, _| {},
    );
    let result = extract_result(&cpu);
    assert_eq!(result.st(0), Fp80::from_bits(0x3FFD, 0xAAAA_AAAA_AAAA_AAAB));
    assert!(result.pe());
    assert!(result.c1());
}

#[test]
fn fnstenv_real_mode_16bit_stores_linear_pointers() {
    let mut memory = [0u8; 0x60];
    memory[0x10..0x18].copy_from_slice(&THREE_F64);
    let code = [0xDC, 0x36, 0x10, 0x00, 0xD9, 0x36, 0x40, 0x00];
    let (_, bus) = run_x87_program(&code, &[Fp80::ONE], &memory, 2, |_, _| {});
    assert_eq!(read_mem_u16(&bus, 0x40), 0x037F);
    assert_eq!(read_mem_u16(&bus, 0x42), 0x3A20);
    assert_eq!(read_mem_u16(&bus, 0x44), 0x3FFF);
    assert_eq!(read_mem_u16(&bus, 0x46), 0x0000);
    assert_eq!(read_mem_u16(&bus, 0x48), 0x1436);
    assert_eq!(read_mem_u16(&bus, 0x4A), 0x0010);
    assert_eq!(read_mem_u16(&bus, 0x4C), 0x2000);
}

#[test]
fn fnstenv_real_mode_32bit_stores_linear_pointers() {
    let mut memory = [0u8; 0x60];
    memory[0x10..0x18].copy_from_slice(&THREE_F64);
    let code = [0xDC, 0x36, 0x10, 0x00, 0x66, 0xD9, 0x36, 0x40, 0x00];
    let (_, bus) = run_x87_program(&code, &[Fp80::ONE], &memory, 2, |_, _| {});
    assert_eq!(read_mem_u32(&bus, 0x40), 0xFFFF_037F);
    assert_eq!(read_mem_u32(&bus, 0x44), 0xFFFF_3A20);
    assert_eq!(read_mem_u32(&bus, 0x48), 0xFFFF_3FFF);
    assert_eq!(read_mem_u32(&bus, 0x4C), 0xFFFF_0000);
    assert_eq!(read_mem_u32(&bus, 0x50), 0x0000_1436);
    assert_eq!(read_mem_u32(&bus, 0x54), 0xFFFF_0010);
    assert_eq!(read_mem_u32(&bus, 0x58), 0x0000_2000);
}

#[test]
fn fldenv_real_mode_loads_linear_pointers() {
    let mut memory = [0u8; 0x20];
    let image: [u16; 7] = [0x027F, 0x3000, 0x0FFF, 0x2345, 0x1123, 0x6789, 0x5000];
    for (index, word) in image.iter().enumerate() {
        memory[index * 2..index * 2 + 2].copy_from_slice(&word.to_le_bytes());
    }
    let (cpu, _) = run_x87_program(&[0xD9, 0x26, 0x00, 0x00], &[], &memory, 1, |_, _| {});
    assert_eq!(cpu.state.fpu.control_word, 0x027F);
    assert_eq!(cpu.state.fpu.status_word, 0x3000);
    assert_eq!(cpu.state.fpu.tag_word, 0x0FFF);
    assert_eq!(cpu.state.fpu.fip_offset, 0x0001_2345);
    assert_eq!(cpu.state.fpu.fip_selector, 0);
    assert_eq!(cpu.state.fpu.fpu_opcode, 0x0123);
    assert_eq!(cpu.state.fpu.fdp_offset, 0x0005_6789);
    assert_eq!(cpu.state.fpu.fdp_selector, 0);
}

#[test]
fn fnsave_stores_registers_in_stack_order() {
    let (_, bus) = run_x87_program(&[0xDD, 0x36, 0x40, 0x00], &[TWO, THREE], &[], 1, |_, _| {});
    assert_eq!(read_mem_u16(&bus, 0x42), 0x3000);
    assert_eq!(read_mem_tbyte(&bus, 0x40 + 14), TWO.to_le_bytes());
    assert_eq!(read_mem_tbyte(&bus, 0x40 + 24), THREE.to_le_bytes());
}

#[test]
fn frstor_loads_registers_in_stack_order_of_the_loaded_top() {
    let mut memory = [0u8; 94];
    let environment: [u16; 7] = [0x037F, 0x3000, 0x0FFF, 0, 0, 0, 0];
    for (index, word) in environment.iter().enumerate() {
        memory[index * 2..index * 2 + 2].copy_from_slice(&word.to_le_bytes());
    }
    memory[14..24].copy_from_slice(&FIVE.to_le_bytes());
    memory[24..34].copy_from_slice(&SIX.to_le_bytes());
    let (cpu, _) = run_x87_program(&[0xDD, 0x26, 0x00, 0x00], &[], &memory, 1, |_, _| {});
    assert_eq!(cpu.state.fpu.registers[6], FIVE);
    assert_eq!(cpu.state.fpu.registers[7], SIX);
}

/// Sets a pending unmasked invalid operation exception with CR0.NE=1.
fn pend_invalid_operation(state: &mut I386State, bus: &mut TestBus) {
    state.cr0 |= 0x20;
    state.fpu.control_word = 0x037E;
    state.fpu.status_word |= 0x8081;
    set_interrupt_vector(bus, 13, 0x6000);
    set_interrupt_vector(bus, 16, 0x5000);
}

#[test]
fn fnclex_runs_with_a_pending_unmasked_exception() {
    let (cpu, _) = run_x87_program(
        &[0xDB, 0xE2],
        &[ONE_AND_HALF],
        &[],
        1,
        pend_invalid_operation,
    );
    assert_eq!(cpu.state.fpu.status_word & 0x80FF, 0);
    assert_eq!(common::Cpu::cs(&cpu), 0x1000);
    assert_eq!(common::Cpu::ip(&cpu), 0x0002);
}

#[test]
fn fnstsw_ax_runs_with_a_pending_unmasked_exception() {
    let (cpu, _) = run_x87_program(
        &[0xDF, 0xE0],
        &[ONE_AND_HALF],
        &[],
        1,
        pend_invalid_operation,
    );
    assert_eq!(cpu.state.eax() as u16, 0xB881);
    assert_eq!(common::Cpu::cs(&cpu), 0x1000);
    assert_eq!(common::Cpu::ip(&cpu), 0x0002);
}

#[test]
fn fadd_with_a_pending_unmasked_exception_raises_math_fault() {
    let (cpu, _) = run_x87_program(
        &[0xD8, 0xC1],
        &[ONE_AND_HALF, TWO],
        &[],
        1,
        pend_invalid_operation,
    );
    assert_eq!(common::Cpu::cs(&cpu), 0x5000);
}

#[test]
fn code_fetch_fault_of_the_displacement_precedes_the_pending_x87_exception() {
    let (cpu, _) = run_x87_program(&[], &[ONE_AND_HALF], &[], 1, |state, bus| {
        pend_invalid_operation(state, bus);
        state.set_eip(0xFFFD);
        place_code(bus, 0x1000, 0xFFFD, &[0xD8, 0x06, 0x10]);
    });
    assert_eq!(common::Cpu::cs(&cpu), 0x6000);
}

/// Runs `steps` instructions of `code` at linear 4000h with flat protected mode segments,
/// a 32-bit code segment when `code_32` is set, CS 0008h and data selectors 0010h.
fn run_x87_protected_program(
    code: &[u8],
    regions: &[(usize, &[u8])],
    steps: usize,
    code_32: bool,
) -> (I386<{ CPU_MODEL_486_DX }>, TestBus) {
    let mut cpu = I386::<{ CPU_MODEL_486_DX }>::new();
    let mut bus = TestBus::new();
    bus.ram[0x4000..0x4000 + code.len()].copy_from_slice(code);
    for (start, bytes) in regions {
        bus.ram[*start..*start + bytes.len()].copy_from_slice(bytes);
    }
    let mut state = I386State {
        cr0: 0x0000_0031,
        ..I386State::default()
    };
    for segment in 0..6 {
        let is_code = segment == 1;
        state.sregs[segment] = if is_code { 0x08 } else { 0x10 };
        state.seg_bases[segment] = 0;
        state.seg_limits[segment] = 0xFFFF_FFFF;
        state.seg_rights[segment] = if is_code { 0x9B } else { 0x93 };
        state.seg_granularity[segment] = if !is_code || code_32 { 0xCF } else { 0x8F };
        state.seg_valid[segment] = true;
    }
    state.set_esp(0x8000);
    state.set_eip(0x4000);
    state.set_eflags(0x0000_0002);
    cpu.load_state(&state);
    for _ in 0..steps {
        cpu.step(&mut bus);
    }
    (cpu, bus)
}

fn read_linear_u16(bus: &TestBus, address: usize) -> u16 {
    u16::from_le_bytes([bus.ram[address], bus.ram[address + 1]])
}

fn read_linear_u32(bus: &TestBus, address: usize) -> u32 {
    u32::from_le_bytes(bus.ram[address..address + 4].try_into().unwrap())
}

fn read_linear_tbyte(bus: &TestBus, address: usize) -> [u8; 10] {
    bus.ram[address..address + 10].try_into().unwrap()
}

/// FNINIT, FLD1, FDIV qword [5008h] in a 32-bit code segment.
const PROTECTED_FDIV_32: [u8; 10] = [0xDB, 0xE3, 0xD9, 0xE8, 0xDC, 0x35, 0x08, 0x50, 0x00, 0x00];

#[test]
fn fnstenv_protected_mode_32bit_image() {
    let mut code = PROTECTED_FDIV_32.to_vec();
    code.extend_from_slice(&[0xD9, 0x35, 0x00, 0x51, 0x00, 0x00]);
    let (_, bus) = run_x87_protected_program(&code, &[(0x5008, &THREE_F64)], 4, true);
    let image: Vec<u32> = (0..7)
        .map(|index| read_linear_u32(&bus, 0x5100 + index * 4))
        .collect();
    assert_eq!(
        image,
        [
            0xFFFF_037F,
            0xFFFF_3A20,
            0xFFFF_3FFF,
            0x0000_4004,
            0x0435_0008,
            0x0000_5008,
            0xFFFF_0010
        ]
    );
}

#[test]
fn fnstenv_protected_mode_16bit_image_in_a_32bit_code_segment() {
    let mut code = PROTECTED_FDIV_32.to_vec();
    code.extend_from_slice(&[0x66, 0xD9, 0x35, 0x20, 0x51, 0x00, 0x00]);
    let (_, bus) = run_x87_protected_program(&code, &[(0x5008, &THREE_F64)], 4, true);
    let image: Vec<u16> = (0..7)
        .map(|index| read_linear_u16(&bus, 0x5120 + index * 2))
        .collect();
    assert_eq!(
        image,
        [0x037F, 0x3A20, 0x3FFF, 0x4004, 0x0008, 0x5008, 0x0010]
    );
}

#[test]
fn fnstenv_protected_mode_16bit_image_in_a_16bit_code_segment() {
    let code = [
        0xDB, 0xE3, 0xD9, 0xE8, 0xDC, 0x36, 0x08, 0x50, 0xD9, 0x36, 0x20, 0x51,
    ];
    let (_, bus) = run_x87_protected_program(&code, &[(0x5008, &THREE_F64)], 4, false);
    let image: Vec<u16> = (0..7)
        .map(|index| read_linear_u16(&bus, 0x5120 + index * 2))
        .collect();
    assert_eq!(
        image,
        [0x037F, 0x3A20, 0x3FFF, 0x4004, 0x0008, 0x5008, 0x0010]
    );
}

#[test]
fn fnsave_in_a_32bit_code_segment_stores_the_108_byte_image() {
    let mut code = PROTECTED_FDIV_32.to_vec();
    code.extend_from_slice(&[0xDD, 0x35, 0x00, 0x53, 0x00, 0x00]);
    let (cpu, bus) = run_x87_protected_program(&code, &[(0x5008, &THREE_F64)], 4, true);
    assert_eq!(read_linear_u32(&bus, 0x5300 + 12), 0x0000_4004);
    assert_eq!(read_linear_u32(&bus, 0x5300 + 24), 0xFFFF_0010);
    assert_eq!(
        read_linear_tbyte(&bus, 0x5300 + 28),
        Fp80::from_bits(0x3FFD, 0xAAAA_AAAA_AAAA_AAAB).to_le_bytes()
    );
    assert_eq!(cpu.state.fpu.tag_word, 0xFFFF);
}

#[test]
fn fldenv_protected_mode_32bit_image() {
    let image: Vec<u8> = [
        0xFFFF_037Fu32,
        0xFFFF_0000,
        0xFFFF_FFFF,
        0x0040_1234,
        0x0123_0008,
        0x0040_5678,
        0xFFFF_0010,
    ]
    .iter()
    .flat_map(|dword| dword.to_le_bytes())
    .collect();
    let code = [0xD9, 0x25, 0x00, 0x52, 0x00, 0x00];
    let (cpu, _) = run_x87_protected_program(&code, &[(0x5200, &image)], 1, true);
    assert_eq!(cpu.state.fpu.fip_offset, 0x0040_1234);
    assert_eq!(cpu.state.fpu.fip_selector, 0x0008);
    assert_eq!(cpu.state.fpu.fpu_opcode, 0x0123);
    assert_eq!(cpu.state.fpu.fdp_offset, 0x0040_5678);
    assert_eq!(cpu.state.fpu.fdp_selector, 0x0010);
}

#[test]
fn fldenv_protected_mode_16bit_image() {
    let image: Vec<u8> = [0x037Fu16, 0x0000, 0xFFFF, 0x1234, 0x0008, 0x5678, 0x0010]
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect();
    let code = [0x66, 0xD9, 0x25, 0x40, 0x52, 0x00, 0x00];
    let (cpu, _) = run_x87_protected_program(&code, &[(0x5240, &image)], 1, true);
    assert_eq!(cpu.state.fpu.fip_offset, 0x1234);
    assert_eq!(cpu.state.fpu.fip_selector, 0x0008);
    assert_eq!(cpu.state.fpu.fpu_opcode, 0);
    assert_eq!(cpu.state.fpu.fdp_offset, 0x5678);
    assert_eq!(cpu.state.fpu.fdp_selector, 0x0010);
}

#[test]
fn fnstenv_in_virtual_8086_mode_stores_the_real_mode_image() {
    let mut cpu = I386::<{ CPU_MODEL_486_DX }>::new();
    let mut bus = TestBus::new();
    let code = [
        0xDB, 0xE3, 0xD9, 0xE8, 0xDC, 0x36, 0x08, 0x04, 0xD9, 0x36, 0x20, 0x04,
    ];
    bus.ram[0x0100..0x0100 + code.len()].copy_from_slice(&code);
    bus.ram[0x0608..0x0610].copy_from_slice(&THREE_F64);
    let mut state = I386State {
        cr0: 0x0000_0031,
        ..I386State::default()
    };
    let selectors = [0x0020u16, 0x0010, 0x0030, 0x0020, 0x0020, 0x0020];
    for (segment, selector) in selectors.into_iter().enumerate() {
        state.sregs[segment] = selector;
        state.seg_bases[segment] = u32::from(selector) << 4;
        state.seg_limits[segment] = 0xFFFF;
        state.seg_rights[segment] = 0xF3;
        state.seg_granularity[segment] = 0;
        state.seg_valid[segment] = true;
    }
    state.stored_cpl = 3;
    state.set_esp(0x0800);
    state.set_eip(0);
    state.set_eflags(0x0002_3002);
    cpu.load_state(&state);
    for _ in 0..4 {
        cpu.step(&mut bus);
    }
    let image: Vec<u16> = (0..7)
        .map(|index| read_linear_u16(&bus, 0x620 + index * 2))
        .collect();
    assert_eq!(
        image,
        [0x037F, 0x3A20, 0x3FFF, 0x0104, 0x0436, 0x0608, 0x0000]
    );
}

#[test]
fn fnstenv_derives_the_tags_of_nonempty_registers_from_their_contents() {
    let (_, bus) = run_x87_program(
        &[0xD9, 0x36, 0x40, 0x00],
        &[Fp80::ZERO],
        &[],
        1,
        |state, _| {
            state.fpu.tag_word = 0x3FFF;
        },
    );
    assert_eq!(read_mem_u16(&bus, 0x44), 0x7FFF);
}

#[test]
fn legacy_no_op_forms_run_with_a_pending_unmasked_exception() {
    for modrm in [0xE0, 0xE1, 0xE4] {
        let (cpu, _) = run_x87_program(
            &[0xDB, modrm],
            &[ONE_AND_HALF],
            &[],
            1,
            pend_invalid_operation,
        );
        assert_eq!(common::Cpu::cs(&cpu), 0x1000, "DB {modrm:02X}");
        assert_eq!(common::Cpu::ip(&cpu), 0x0002, "DB {modrm:02X}");
        assert_eq!(cpu.state.fpu.status_word & 0x8081, 0x8081, "DB {modrm:02X}");
    }
}

#[test]
fn control_stores_and_fninit_run_with_a_pending_unmasked_exception() {
    let programs: [&[u8]; 4] = [
        &[0xD9, 0x36, 0x40, 0x00],
        &[0xDD, 0x36, 0x40, 0x00],
        &[0xD9, 0x3E, 0x40, 0x00],
        &[0xDB, 0xE3],
    ];
    for program in programs {
        let (cpu, bus) = run_x87_program(program, &[ONE_AND_HALF], &[], 1, pend_invalid_operation);
        assert_eq!(common::Cpu::cs(&cpu), 0x1000, "{program:02X?}");
        assert_eq!(
            common::Cpu::ip(&cpu),
            program.len() as u16,
            "{program:02X?}"
        );
        if program[0] != 0xDB {
            assert_eq!(read_mem_u16(&bus, 0x40), 0x037E, "{program:02X?}");
        }
    }
}

/// FDIV qword [0010h] with 3.0 there, which rounds up and sets C1.
const FDIV_THREE: [u8; 4] = [0xDC, 0x36, 0x10, 0x00];

/// Runs FDIV with a rounded-up quotient followed by `next` and returns the final state.
fn run_after_rounded_up_fdiv(next: &[u8], extra_stack: &[Fp80]) -> X87Result {
    let mut memory = [0u8; 0x20];
    memory[0x10..0x18].copy_from_slice(&THREE_F64);
    memory[0x18..0x1A].copy_from_slice(&7u16.to_le_bytes());
    let mut code = FDIV_THREE.to_vec();
    code.extend_from_slice(next);
    let mut stack = vec![Fp80::ONE];
    stack.extend_from_slice(extra_stack);
    let (cpu, _) = run_x87_program(&code, &stack, &memory, 2, |_, _| {});
    extract_result(&cpu)
}

#[test]
fn loads_and_register_stores_clear_c1() {
    let programs: [&[u8]; 8] = [
        &[0xD9, 0xE8],
        &[0xD9, 0xEB],
        &[0xDF, 0x06, 0x18, 0x00],
        &[0xD9, 0xC0],
        &[0xDD, 0xD1],
        &[0xDB, 0x3E, 0x00, 0x01],
        &[0xDB, 0x2E, 0x00, 0x01],
        &[0xD9, 0xEE],
    ];
    for program in programs {
        let result = run_after_rounded_up_fdiv(program, &[TWO]);
        assert!(!result.c1(), "{program:02X?}");
    }
    let result = run_after_rounded_up_fdiv(&[], &[]);
    assert!(result.c1());
}

#[test]
fn exact_tiny_result_raises_unmasked_underflow() {
    let half = 0.5f64.to_le_bytes();
    let (cpu, _) = run_x87_program(
        &[0xDC, 0x0E, 0x00, 0x00],
        &[Fp80::from_bits(0x0001, 0x8000_0000_0000_0000)],
        &half,
        1,
        |state, _| state.fpu.control_word = 0x036F,
    );
    let result = extract_result(&cpu);
    assert!(result.ue());
    assert!(!result.pe());
    assert_eq!(result.status_word & 0x8080, 0x8080);
}

#[test]
fn exact_tiny_result_raises_no_masked_underflow() {
    let half = 0.5f64.to_le_bytes();
    let (cpu, _) = run_x87_program(
        &[0xDC, 0x0E, 0x00, 0x00],
        &[Fp80::from_bits(0x0001, 0x8000_0000_0000_0000)],
        &half,
        1,
        |_, _| {},
    );
    let result = extract_result(&cpu);
    assert!(!result.ue());
    assert_eq!(result.st(0), Fp80::from_bits(0x0000, 0x4000_0000_0000_0000));
}

#[test]
fn faulting_fild_keeps_c1() {
    let mut memory = [0u8; 0x20];
    memory[0x10..0x18].copy_from_slice(&THREE_F64);
    let mut code = FDIV_THREE.to_vec();
    code.extend_from_slice(&[0xDF, 0x06, 0xFF, 0xFF]);
    let (cpu, _) = run_x87_program(&code, &[Fp80::ONE], &memory, 2, |_, bus| {
        set_interrupt_vector(bus, 13, 0x6000);
    });
    let result = extract_result(&cpu);
    assert_eq!(common::Cpu::cs(&cpu), 0x6000);
    assert!(result.c1());
    assert_eq!(result.top, 7);
}

#[test]
fn faulting_fstp_m80_keeps_c1_and_the_stack() {
    let mut cpu = I386::<{ CPU_MODEL_486_DX }>::new();
    let mut bus = TestBus::new();
    let mut code = PROTECTED_FDIV_32.to_vec();
    code.extend_from_slice(&[0xDB, 0x3D, 0x00, 0x90, 0x00, 0x00]);
    bus.ram[0x4000..0x4000 + code.len()].copy_from_slice(&code);
    bus.ram[0x5008..0x5010].copy_from_slice(&THREE_F64);
    let page_directory = 0x10000;
    let page_table = 0x11000;
    bus.ram[page_directory..page_directory + 4]
        .copy_from_slice(&(page_table as u32 | 0x3).to_le_bytes());
    for page in 0..0x100usize {
        let entry = if page == 0x9 {
            0
        } else {
            (page as u32) << 12 | 0x3
        };
        bus.ram[page_table + page * 4..page_table + page * 4 + 4]
            .copy_from_slice(&entry.to_le_bytes());
    }
    let gdt = 0x6800;
    bus.ram[gdt + 8..gdt + 16].copy_from_slice(&[0xFF, 0xFF, 0, 0, 0, 0x9B, 0xCF, 0]);
    let idt = 0x6000;
    bus.ram[idt + 14 * 8..idt + 15 * 8]
        .copy_from_slice(&[0x00, 0x70, 0x08, 0x00, 0x00, 0x8E, 0x00, 0x00]);
    bus.ram[0x7000] = 0xF4;
    let mut state = I386State {
        cr0: 0x8000_0031,
        cr3: page_directory as u32,
        gdt_base: gdt as u32,
        gdt_limit: 0x0F,
        idt_base: idt as u32,
        idt_limit: 0x07FF,
        ..I386State::default()
    };
    for segment in 0..6 {
        let is_code = segment == 1;
        state.sregs[segment] = if is_code { 0x08 } else { 0x10 };
        state.seg_limits[segment] = 0xFFFF_FFFF;
        state.seg_rights[segment] = if is_code { 0x9B } else { 0x93 };
        state.seg_granularity[segment] = 0xCF;
        state.seg_valid[segment] = true;
    }
    state.set_esp(0x8000);
    state.set_eip(0x4000);
    state.set_eflags(0x0000_0002);
    cpu.load_state(&state);
    for _ in 0..4 {
        cpu.step(&mut bus);
    }
    let result = extract_result(&cpu);
    assert_eq!(cpu.state.cr2, 0x9000);
    assert_eq!(cpu.state.eip(), 0x7000);
    assert!(result.c1());
    assert_eq!(result.top, 7);
}

/// The control word with only the invalid operation exception unmasked.
const CW_INVALID_UNMASKED: u16 = 0x037E;
/// The control word with only the denormal operand exception unmasked.
const CW_DENORMAL_UNMASKED: u16 = 0x037D;
/// The control word with only the zero divide exception unmasked.
const CW_ZERO_DIVIDE_UNMASKED: u16 = 0x037B;
/// The control word with only the overflow exception unmasked.
const CW_OVERFLOW_UNMASKED: u16 = 0x0377;
/// The control word with only the underflow exception unmasked.
const CW_UNDERFLOW_UNMASKED: u16 = 0x036F;
/// The largest exponent with the significand 1.5: 1.5 * 2^16383.
const HUGE: Fp80 = Fp80::from_bits(0x7FFE, 0xC000_0000_0000_0000);
/// The smallest exponent with the significand 1.5: 1.5 * 2^-16382.
const TINY: Fp80 = Fp80::from_bits(0x0001, 0xC000_0000_0000_0000);
/// A denormal with a multi-bit significand.
const DENORMAL: Fp80 = Fp80::from_bits(0x0000, 0x0000_0000_0000_1234);
/// 1/3 rounded to nearest.
const ONE_THIRD: Fp80 = Fp80::from_bits(0x3FFD, 0xAAAA_AAAA_AAAA_AAAB);
/// The fill byte of the memory a suppressed store must leave unchanged.
const UNTOUCHED: u8 = 0x55;

/// Runs one instruction `code` with `stack`, the control word `control_word` and
/// `memory` at DS:0000h.
fn run_with_control(
    code: &[u8],
    stack: &[Fp80],
    control_word: u16,
    memory: &[u8],
) -> (I386<{ CPU_MODEL_486_DX }>, TestBus) {
    run_x87_program(code, stack, memory, 1, |state, _| {
        state.fpu.control_word = control_word;
    })
}

/// Returns 32 bytes of untouched memory with `bytes` at offset 0.
fn memory_with(bytes: &[u8]) -> [u8; 32] {
    let mut memory = [UNTOUCHED; 32];
    memory[..bytes.len()].copy_from_slice(bytes);
    memory
}

/// Asserts that the 16 bytes at DS:0010h hold the fill byte.
fn assert_store_suppressed(bus: &TestBus, program: &[u8]) {
    assert_eq!(
        bus.ram[0x20010..0x20020],
        [UNTOUCHED; 16],
        "{program:02X?} stored a result"
    );
}

#[test]
fn unmasked_stack_underflow_stores_nothing_and_keeps_top() {
    let programs: [&[u8]; 8] = [
        &[0xD9, 0x1E, 0x10, 0x00],
        &[0xDD, 0x1E, 0x10, 0x00],
        &[0xDB, 0x3E, 0x10, 0x00],
        &[0xDF, 0x1E, 0x10, 0x00],
        &[0xDB, 0x1E, 0x10, 0x00],
        &[0xDF, 0x3E, 0x10, 0x00],
        &[0xDF, 0x36, 0x10, 0x00],
        &[0xDD, 0xD9],
    ];
    for program in programs {
        let (cpu, bus) = run_with_control(program, &[], CW_INVALID_UNMASKED, &memory_with(&[]));
        let result = extract_result(&cpu);
        assert_eq!(result.top, 0, "{program:02X?}");
        assert_eq!(result.status_word & 0x82FF, 0x80C1, "{program:02X?}");
        assert_eq!(result.tag_word, 0xFFFF, "{program:02X?}");
        assert_store_suppressed(&bus, program);
    }
}

#[test]
fn unmasked_invalid_store_stores_nothing_and_keeps_the_operand() {
    let cases: [(&[u8], Fp80); 6] = [
        (&[0xD9, 0x16, 0x10, 0x00], POSITIVE_SNAN),
        (&[0xDD, 0x1E, 0x10, 0x00], POSITIVE_SNAN),
        (&[0xDB, 0x1E, 0x10, 0x00], POSITIVE_SNAN),
        (&[0xDF, 0x1E, 0x10, 0x00], TWO_POW_100),
        (&[0xDF, 0x3E, 0x10, 0x00], TWO_POW_100),
        (&[0xDF, 0x36, 0x10, 0x00], TWO_POW_100),
    ];
    for (program, operand) in cases {
        let (cpu, bus) =
            run_with_control(program, &[operand], CW_INVALID_UNMASKED, &memory_with(&[]));
        let result = extract_result(&cpu);
        assert_eq!(result.top, 7, "{program:02X?}");
        assert_eq!(result.st(0), operand, "{program:02X?}");
        assert_eq!(result.status_word & 0x82FF, 0x8081, "{program:02X?}");
        assert_store_suppressed(&bus, program);
    }
}

#[test]
fn unmasked_invalid_arithmetic_keeps_the_operands_and_top() {
    let cases: [(&[u8], &[Fp80]); 8] = [
        (&[0xD9, 0xFA], &[NEGATIVE_ONE]),
        (&[0xD8, 0x06, 0x00, 0x00], &[Fp80::ONE]),
        (&[0xDE, 0xC1], &[Fp80::ONE, POSITIVE_SNAN]),
        (&[0xDE, 0xE9], &[Fp80::INFINITY, Fp80::INFINITY]),
        (&[0xD9, 0xF8], &[Fp80::ONE, Fp80::ZERO]),
        (&[0xD9, 0xF4], &[POSITIVE_SNAN]),
        (&[0xD9, 0xFB], &[Fp80::INFINITY]),
        (&[0xD9, 0xF1], &[NEGATIVE_ONE, Fp80::ONE]),
    ];
    let signaling = 0x7F80_0001u32.to_le_bytes();
    for (program, stack) in cases {
        let (cpu, _) = run_with_control(
            program,
            stack,
            CW_INVALID_UNMASKED,
            &memory_with(&signaling),
        );
        let result = extract_result(&cpu);
        assert_eq!(result.top as usize, 8 - stack.len(), "{program:02X?}");
        for (index, value) in stack.iter().enumerate() {
            assert_eq!(result.st(index as u8), *value, "{program:02X?} ST({index})");
        }
        assert!(result.ie(), "{program:02X?}");
        assert_eq!(result.status_word & 0x8080, 0x8080, "{program:02X?}");
    }
}

#[test]
fn unmasked_invalid_compare_sets_the_condition_codes_and_keeps_top() {
    let cases: [(&[u8], &[Fp80]); 4] = [
        (&[0xD8, 0xD9], &[Fp80::ONE, POSITIVE_SNAN]),
        (&[0xDE, 0xD9], &[Fp80::ONE]),
        (&[0xDA, 0xE9], &[Fp80::ONE, POSITIVE_SNAN]),
        (&[0xD8, 0x1E, 0x00, 0x00], &[Fp80::ONE]),
    ];
    let signaling = 0x7F80_0001u32.to_le_bytes();
    for (program, stack) in cases {
        let (cpu, _) = run_with_control(
            program,
            stack,
            CW_INVALID_UNMASKED,
            &memory_with(&signaling),
        );
        let result = extract_result(&cpu);
        assert_eq!(result.top as usize, 8 - stack.len(), "{program:02X?}");
        assert!(result.c3() && result.c2() && result.c0(), "{program:02X?}");
        assert!(result.ie(), "{program:02X?}");
    }
}

#[test]
fn unmasked_stack_overflow_keeps_top_and_the_registers() {
    let stack = [Fp80::ONE, TWO, THREE, FOUR, FIVE, SIX, SEVEN, HALF];
    let programs: [&[u8]; 5] = [
        &[0xD9, 0xE8],
        &[0xD9, 0xC3],
        &[0xD9, 0xF2],
        &[0xD9, 0xF4],
        &[0xD9, 0xFB],
    ];
    for program in programs {
        let (cpu, _) = run_with_control(program, &stack, CW_INVALID_UNMASKED, &[]);
        let result = extract_result(&cpu);
        assert_eq!(result.top, 0, "{program:02X?}");
        for (index, value) in stack.iter().enumerate() {
            assert_eq!(result.st(index as u8), *value, "{program:02X?} ST({index})");
        }
        assert_eq!(result.status_word & 0x82FF, 0x82C1, "{program:02X?}");
    }
}

/// The C2 bit of the status word.
const CONDITION_C2: u16 = 0x0400;

#[test]
fn masked_stack_overflow_of_a_two_result_instruction_writes_the_indefinite_twice() {
    let programs: [(&[u8], u16); 3] = [
        (&[0xD9, 0xF2], 0),
        (&[0xD9, 0xFB], 0),
        (&[0xD9, 0xF4], CONDITION_C2),
    ];
    for (program, condition_c2) in programs {
        for operand in [Fp80::ONE, Fp80::ZERO, OUT_OF_RANGE] {
            let stack = [operand, TWO, THREE, FOUR, FIVE, SIX, SEVEN, HALF];
            let (cpu, _) = run_x87_program(program, &stack, &[], 1, |state, _| {
                state.fpu.status_word |= CONDITION_C2;
            });
            let result = extract_result(&cpu);
            assert_eq!(result.top, 7, "{program:02X?} {operand:?}");
            assert_eq!(result.st(0), Fp80::INDEFINITE, "{program:02X?} {operand:?}");
            assert_eq!(result.st(1), Fp80::INDEFINITE, "{program:02X?} {operand:?}");
            assert_eq!(result.tag(0), 0b10, "{program:02X?} {operand:?}");
            assert_eq!(result.tag(1), 0b10, "{program:02X?} {operand:?}");
            for index in 2..8 {
                assert_eq!(
                    result.st(index),
                    stack[index as usize - 1],
                    "{program:02X?} {operand:?} ST({index})"
                );
            }
            assert_eq!(
                result.status_word & 0xC7FF,
                0x0241 | condition_c2,
                "{program:02X?} {operand:?}"
            );
        }
    }
}

#[test]
fn fld_of_a_signaling_nan_with_unmasked_invalid_pushes_nothing() {
    let programs: [(&[u8], &[u8]); 2] = [
        (&[0xD9, 0x06, 0x00, 0x00], &0x7F80_0001u32.to_le_bytes()),
        (
            &[0xDD, 0x06, 0x00, 0x00],
            &0x7FF0_0000_0000_0001u64.to_le_bytes(),
        ),
    ];
    for (program, operand) in programs {
        let (cpu, _) = run_with_control(program, &[], CW_INVALID_UNMASKED, operand);
        let result = extract_result(&cpu);
        assert_eq!(result.top, 0, "{program:02X?}");
        assert_eq!(result.tag_word, 0xFFFF, "{program:02X?}");
        assert_eq!(result.status_word & 0x80FF, 0x8081, "{program:02X?}");
    }
}

#[test]
fn fld_of_a_denormal_pushes_it_normalized_and_reports_de() {
    let programs: [(&[u8], &[u8], Fp80); 2] = [
        (
            &[0xD9, 0x06, 0x00, 0x00],
            &0x0000_0123u32.to_le_bytes(),
            Fp80::from_bits(0x3F72, 0x9180_0000_0000_0000),
        ),
        (
            &[0xDD, 0x06, 0x00, 0x00],
            &0x0000_0000_0000_0123u64.to_le_bytes(),
            Fp80::from_bits(0x3BD5, 0x9180_0000_0000_0000),
        ),
    ];
    for (program, operand, expected) in programs {
        let (cpu, _) = run_with_control(program, &[], CW_DENORMAL_UNMASKED, operand);
        let result = extract_result(&cpu);
        assert_eq!(result.top, 7, "{program:02X?}");
        assert_eq!(result.st(0), expected, "{program:02X?}");
        assert_eq!(result.status_word & 0x80FF, 0x8082, "{program:02X?}");
    }
}

#[test]
fn unmasked_zero_divide_keeps_the_operands_and_top() {
    let cases: [(&[u8], &[Fp80]); 4] = [
        (&[0xDE, 0xF9], &[Fp80::ZERO, Fp80::ONE]),
        (&[0xD8, 0x36, 0x00, 0x00], &[Fp80::ONE]),
        (&[0xD9, 0xF4], &[Fp80::ZERO]),
        (&[0xD9, 0xF1], &[Fp80::ZERO, Fp80::ONE]),
    ];
    for (program, stack) in cases {
        let (cpu, _) = run_with_control(
            program,
            stack,
            CW_ZERO_DIVIDE_UNMASKED,
            &memory_with(&[0, 0, 0, 0]),
        );
        let result = extract_result(&cpu);
        assert_eq!(result.top as usize, 8 - stack.len(), "{program:02X?}");
        for (index, value) in stack.iter().enumerate() {
            assert_eq!(result.st(index as u8), *value, "{program:02X?} ST({index})");
        }
        assert_eq!(result.status_word & 0x80FF, 0x8084, "{program:02X?}");
    }
}

#[test]
fn unmasked_denormal_operand_keeps_the_operands_and_reports_only_de() {
    let cases: [(&[u8], &[Fp80]); 6] = [
        (&[0xDE, 0xC1], &[ONE_THIRD, DENORMAL]),
        (&[0xD9, 0xFA], &[DENORMAL]),
        (&[0xD9, 0xFC], &[DENORMAL]),
        (&[0xD9, 0xF4], &[DENORMAL]),
        (&[0xD9, 0xF3], &[Fp80::ONE, DENORMAL]),
        (&[0xD8, 0xD9], &[Fp80::ONE, DENORMAL]),
    ];
    for (program, stack) in cases {
        let (cpu, _) = run_with_control(program, stack, CW_DENORMAL_UNMASKED, &[]);
        let result = extract_result(&cpu);
        assert_eq!(result.top as usize, 8 - stack.len(), "{program:02X?}");
        for (index, value) in stack.iter().enumerate() {
            assert_eq!(result.st(index as u8), *value, "{program:02X?} ST({index})");
        }
        assert_eq!(result.status_word & 0x80FF, 0x8082, "{program:02X?}");
    }
}

#[test]
fn unmasked_overflow_delivers_the_biased_result() {
    let (cpu, _) = run_with_control(&[0xD8, 0xC8], &[HUGE], CW_OVERFLOW_UNMASKED, &[]);
    let result = extract_result(&cpu);
    assert_eq!(result.st(0), Fp80::from_bits(0x5FFE, 0x9000_0000_0000_0000));
    assert_eq!(result.status_word & 0x82FF, 0x8088);

    let (cpu, _) = run_with_control(&[0xDE, 0xC1], &[HUGE, HUGE], CW_OVERFLOW_UNMASKED, &[]);
    let result = extract_result(&cpu);
    assert_eq!(result.top, 7);
    assert_eq!(result.st(0), Fp80::from_bits(0x1FFF, 0xC000_0000_0000_0000));

    let one_and_half = 1.5f32.to_le_bytes();
    let (cpu, _) = run_with_control(
        &[0xD8, 0x0E, 0x00, 0x00],
        &[LARGEST_NORMAL],
        0x0B77,
        &one_and_half,
    );
    let result = extract_result(&cpu);
    assert_eq!(result.st(0), Fp80::from_bits(0x1FFF, 0xC000_0000_0000_0000));
    assert_eq!(result.status_word & 0x82FF, 0x82A8);
}

#[test]
fn unmasked_underflow_delivers_the_biased_result() {
    let (cpu, _) = run_with_control(&[0xD8, 0xC8], &[TINY], CW_UNDERFLOW_UNMASKED, &[]);
    let result = extract_result(&cpu);
    assert_eq!(result.st(0), Fp80::from_bits(0x2004, 0x9000_0000_0000_0000));
    assert_eq!(result.status_word & 0x82FF, 0x8090);
}

#[test]
fn unmasked_overflow_and_underflow_store_nothing_to_memory() {
    let large = Fp80::from_bits(0x43E1, 0x8000_0000_0000_0000);
    let small = Fp80::from_bits(0x3C1C, 0x8000_0000_0000_0000);
    let cases: [(&[u8], Fp80, u16, u16); 3] = [
        (
            &[0xD9, 0x1E, 0x10, 0x00],
            large,
            CW_OVERFLOW_UNMASKED,
            0x8088,
        ),
        (
            &[0xDD, 0x1E, 0x10, 0x00],
            HUGE,
            CW_OVERFLOW_UNMASKED,
            0x8088,
        ),
        (
            &[0xD9, 0x1E, 0x10, 0x00],
            small,
            CW_UNDERFLOW_UNMASKED,
            0x8090,
        ),
    ];
    for (program, operand, control_word, status) in cases {
        let (cpu, bus) = run_with_control(program, &[operand], control_word, &memory_with(&[]));
        let result = extract_result(&cpu);
        assert_eq!(result.top, 7, "{program:02X?}");
        assert_eq!(result.st(0), operand, "{program:02X?}");
        assert_eq!(result.status_word & 0x82FF, status, "{program:02X?}");
        assert_store_suppressed(&bus, program);
    }
}

#[test]
fn fscale_overflow_follows_the_rounding_mode() {
    let large_scale = Fp80::from_i32(100_000);
    let cases: [(Fp80, u16, Fp80, bool); 3] = [
        (
            Fp80::ONE,
            CW_ZERO_EXT,
            Fp80::from_bits(0x7FFE, u64::MAX),
            false,
        ),
        (
            Fp80::ONE,
            CW_DOWN_EXT,
            Fp80::from_bits(0x7FFE, u64::MAX),
            false,
        ),
        (NEGATIVE_ONE, CW_DOWN_EXT, Fp80::NEG_INFINITY, true),
    ];
    for (value, control_word, expected, rounded_up) in cases {
        let (cpu, _) = run_with_control(&[0xD9, 0xFD], &[value, large_scale], control_word, &[]);
        let result = extract_result(&cpu);
        assert_eq!(result.st(0), expected, "{control_word:04X}");
        assert!(result.oe() && result.pe(), "{control_word:04X}");
        assert_eq!(result.c1(), rounded_up, "{control_word:04X}");
    }
}

#[test]
fn fscale_rounds_a_denormal_result() {
    let value = Fp80::from_bits(0x3FFF, 0xC000_0000_0000_0001);
    let (cpu, _) = run_with_control(
        &[0xD9, 0xFD],
        &[value, Fp80::from_i32(-16400)],
        CW_UP_EXT,
        &[],
    );
    let result = extract_result(&cpu);
    assert_eq!(result.st(0), Fp80::from_bits(0x0000, 0x0000_3000_0000_0001));
    assert!(result.ue() && result.pe() && result.c1());
}

#[test]
fn fscale_beyond_the_unmasked_bias_returns_the_masked_round_to_nearest_result() {
    let (cpu, _) = run_with_control(
        &[0xD9, 0xFD],
        &[Fp80::ONE, Fp80::from_i32(100_000)],
        CW_OVERFLOW_UNMASKED,
        &[],
    );
    let result = extract_result(&cpu);
    assert_eq!(result.st(0), Fp80::INFINITY);
    assert_eq!(result.status_word & 0x82FF, 0x82A8);

    let (cpu, _) = run_with_control(
        &[0xD9, 0xFD],
        &[Fp80::ONE, Fp80::from_i32(-100_000)],
        CW_UNDERFLOW_UNMASKED,
        &[],
    );
    let result = extract_result(&cpu);
    assert_eq!(result.st(0), Fp80::ZERO);
    assert_eq!(result.status_word & 0x82FF, 0x80B0);
}

#[test]
fn memory_arithmetic_reads_the_operand_before_the_stack_check() {
    let programs: [&[u8]; 4] = [
        &[0xD8, 0x06, 0xFE, 0xFF],
        &[0xDC, 0x16, 0xFC, 0xFF],
        &[0xDE, 0x1E, 0xFF, 0xFF],
        &[0xDA, 0x36, 0xFE, 0xFF],
    ];
    for program in programs {
        let (cpu, _) = run_x87_program(program, &[], &[], 1, |state, bus| {
            state.fpu.control_word = CW_INVALID_UNMASKED;
            set_interrupt_vector(bus, 13, 0x6000);
        });
        let result = extract_result(&cpu);
        assert_eq!(common::Cpu::cs(&cpu), 0x6000, "{program:02X?}");
        assert_eq!(result.status_word, 0, "{program:02X?}");
    }
}

#[test]
fn faulting_operand_read_keeps_the_pointers_of_the_previous_instruction() {
    let mut memory = [0u8; 0x20];
    memory[0x10..0x18].copy_from_slice(&THREE_F64);
    let mut code = FDIV_THREE.to_vec();
    code.extend_from_slice(&[0xD8, 0x06, 0xFE, 0xFF]);
    let (cpu, _) = run_x87_program(&code, &[Fp80::ONE], &memory, 2, |_, bus| {
        set_interrupt_vector(bus, 13, 0x6000);
    });
    assert_eq!(common::Cpu::cs(&cpu), 0x6000);
    assert_eq!(cpu.state.fpu.fip_offset, 0x0000);
    assert_eq!(cpu.state.fpu.fdp_offset, 0x0010);
    assert_eq!(cpu.state.fpu.fpu_opcode, 0x0436);
}

#[test]
fn suppressed_fprem_and_trigonometric_results_clear_c2() {
    let cases: [(&[u8], &[Fp80]); 4] = [
        (&[0xD9, 0xF8], &[Fp80::ONE, Fp80::ZERO]),
        (&[0xD9, 0xF5], &[POSITIVE_SNAN, Fp80::ONE]),
        (&[0xD9, 0xFE], &[Fp80::INFINITY]),
        (&[0xD9, 0xF2], &[POSITIVE_SNAN]),
    ];
    for (program, stack) in cases {
        let (cpu, _) = run_x87_program(program, stack, &[], 1, |state, _| {
            state.fpu.control_word = CW_INVALID_UNMASKED;
            state.fpu.status_word |= 0x4500;
        });
        let result = extract_result(&cpu);
        assert!(!result.c2(), "{program:02X?}");
        assert!(result.c3() && result.c0(), "{program:02X?}");
        assert_eq!(result.st(0), stack[0], "{program:02X?}");
    }
}

#[test]
fn fyl2xp1_of_an_operand_below_minus_one_is_invalid() {
    let r = run_x87(&[0xD9, 0xF9], &[NEGATIVE_TWO, Fp80::ONE]);
    assert_eq!(r.top, 7);
    assert_eq!(r.st(0), Fp80::INDEFINITE);
    assert!(r.ie());
}

#[test]
fn invalid_and_zero_divide_take_priority_over_a_denormal_operand() {
    let (cpu, _) = run_with_control(
        &[0xD9, 0xFA],
        &[DENORMAL.negate()],
        CW_DENORMAL_UNMASKED,
        &[],
    );
    let result = extract_result(&cpu);
    assert_eq!(result.st(0), Fp80::INDEFINITE);
    assert_eq!(result.status_word & 0x80FF, 0x0001);

    let r = run_x87(&[0xDE, 0xF9], &[Fp80::ZERO, DENORMAL]);
    assert_eq!(r.st(0), Fp80::INFINITY);
    assert_eq!(r.status_word & 0x00FF, 0x0004);
}

#[test]
fn transcendental_and_remainder_instructions_report_a_denormal_operand() {
    let cases: [(&[u8], &[Fp80]); 6] = [
        (&[0xD9, 0xF4], &[DENORMAL]),
        (&[0xD9, 0xF3], &[Fp80::ONE, DENORMAL]),
        (&[0xD9, 0xF0], &[DENORMAL]),
        (&[0xD9, 0xFE], &[DENORMAL]),
        (&[0xD9, 0xF8], &[DENORMAL, Fp80::ONE]),
        (&[0xD9, 0xFD], &[Fp80::ONE, DENORMAL]),
    ];
    for (program, stack) in cases {
        let r = run_x87(program, stack);
        assert!(r.de(), "{program:02X?}");
    }
}

#[test]
fn nan_and_zero_divide_take_priority_over_a_denormal_memory_operand() {
    let denormal_single = 0x0000_0001u32.to_le_bytes();
    let (cpu, _) = run_with_control(
        &[0xD8, 0x06, 0x00, 0x00],
        &[POSITIVE_SNAN],
        CW_DENORMAL_UNMASKED,
        &denormal_single,
    );
    let result = extract_result(&cpu);
    assert_eq!(result.st(0), POSITIVE_SNAN.quieten());
    assert_eq!(result.status_word & 0x80FF, 0x0001);

    let (cpu, _) = run_with_control(
        &[0xD8, 0x16, 0x00, 0x00],
        &[POSITIVE_QNAN],
        CW_DENORMAL_UNMASKED,
        &denormal_single,
    );
    let result = extract_result(&cpu);
    assert!(result.c3() && result.c2() && result.c0());
    assert_eq!(result.status_word & 0x80FF, 0x0001);

    let (cpu, _) = run_with_control(
        &[0xD8, 0x3E, 0x00, 0x00],
        &[Fp80::ZERO],
        CW_DENORMAL_UNMASKED,
        &denormal_single,
    );
    let result = extract_result(&cpu);
    assert_eq!(result.st(0), Fp80::INFINITY);
    assert_eq!(result.status_word & 0x80FF, 0x0004);
}

#[test]
fn fst_of_a_denormal_source_reports_de() {
    let (cpu, bus) = run_with_control(
        &[0xD9, 0x16, 0x10, 0x00],
        &[DENORMAL],
        CW_NEAREST_EXT,
        &memory_with(&[]),
    );
    let result = extract_result(&cpu);
    assert_eq!(read_mem_u32(&bus, 0x10), 0);
    assert!(result.de() && result.ue() && result.pe());

    let cases: [(&[u8], Fp80); 3] = [
        (&[0xD9, 0x16, 0x10, 0x00], DENORMAL),
        (&[0xDD, 0x1E, 0x10, 0x00], DENORMAL),
        (&[0xD9, 0x1E, 0x10, 0x00], POSITIVE_PSEUDO_DENORMAL),
    ];
    for (program, operand) in cases {
        let (cpu, bus) = run_with_control(
            program,
            &[operand],
            CW_DENORMAL_UNMASKED & !0x0010,
            &memory_with(&[]),
        );
        let result = extract_result(&cpu);
        assert_eq!(result.top, 7, "{program:02X?}");
        assert_eq!(result.st(0), operand, "{program:02X?}");
        assert_eq!(result.status_word & 0x82FF, 0x8082, "{program:02X?}");
        assert_store_suppressed(&bus, program);
    }
}

#[test]
fn suppressed_store_keeps_a_masked_denormal_operand_exception() {
    let operand = Fp80::from_bits(0x0000, 0x4000_0000_0000_0001);
    let (cpu, bus) = run_with_control(
        &[0xD9, 0x1E, 0x10, 0x00],
        &[operand],
        CW_UNDERFLOW_UNMASKED,
        &memory_with(&[]),
    );
    let result = extract_result(&cpu);
    assert_eq!(result.top, 7);
    assert_eq!(result.status_word & 0x82FF, 0x8092);
    assert_store_suppressed(&bus, &[0xD9, 0x1E, 0x10, 0x00]);
}

#[test]
fn compare_stopped_by_an_unmasked_denormal_sets_the_condition_codes() {
    let denormal_single = 0x0000_0001u32.to_le_bytes();
    let (cpu, _) = run_x87_program(
        &[0xD8, 0x1E, 0x00, 0x00],
        &[Fp80::ONE],
        &denormal_single,
        1,
        |state, _| {
            state.fpu.control_word = CW_DENORMAL_UNMASKED;
            state.fpu.status_word |= 0x4500;
        },
    );
    let result = extract_result(&cpu);
    assert_eq!(result.top, 7);
    assert!(!result.c3() && !result.c2() && !result.c0());
    assert_eq!(result.status_word & 0x80FF, 0x8082);
}

/// An unnormal with a zero significand.
const UNNORMAL_ZERO_SIGNIFICAND: Fp80 = Fp80::from_bits(0x4000, 0x0000_0000_0000_0000);
/// A pseudo-NaN with the quiet bit set.
const QUIET_PSEUDO_NAN: Fp80 = Fp80::from_bits(0x7FFF, 0x4000_0000_0000_0000);
/// Every unsupported encoding class (80486 PRM 16.1.7).
const UNSUPPORTED_OPERANDS: [Fp80; 6] = [
    UNNORMAL,
    UNNORMAL_ZERO_SIGNIFICAND,
    PSEUDO_INFINITY,
    NEGATIVE_PSEUDO_INFINITY,
    PSEUDO_NAN,
    QUIET_PSEUDO_NAN,
];

#[test]
fn fdiv_by_an_unsupported_encoding_raises_invalid() {
    for divisor in [PSEUDO_INFINITY, UNNORMAL_ZERO_SIGNIFICAND] {
        let result = run_x87(&[0xD8, 0xF1], &[Fp80::ONE, divisor]);
        assert_eq!(result.st(0), Fp80::INDEFINITE, "{divisor:?}");
        assert_eq!(result.st(1), divisor, "{divisor:?}");
        assert_eq!(result.status_word & 0x80FF, 0x0001, "{divisor:?}");

        let result = run_x87_with_cw(&[0xD8, 0xF1], &[Fp80::ONE, divisor], CW_INVALID_UNMASKED);
        assert_eq!(result.st(0), Fp80::ONE, "{divisor:?}");
        assert_eq!(result.st(1), divisor, "{divisor:?}");
        assert_eq!(result.status_word & 0x80FF, 0x8081, "{divisor:?}");
    }
}

#[test]
fn arithmetic_on_an_unsupported_encoding_returns_the_indefinite() {
    let binary: [&[u8]; 11] = [
        &[0xD8, 0xC1],
        &[0xD8, 0xE1],
        &[0xD8, 0xC9],
        &[0xD8, 0xF1],
        &[0xD8, 0xF9],
        &[0xD9, 0xFD],
        &[0xD9, 0xF8],
        &[0xD9, 0xF5],
        &[0xD9, 0xF1],
        &[0xD9, 0xF3],
        &[0xD9, 0xF9],
    ];
    let unary: [&[u8]; 5] = [
        &[0xD9, 0xFA],
        &[0xD9, 0xFC],
        &[0xD9, 0xFE],
        &[0xD9, 0xFF],
        &[0xD9, 0xF0],
    ];
    for operand in UNSUPPORTED_OPERANDS {
        let cases = binary
            .iter()
            .flat_map(|program| {
                [
                    (*program, [operand, Fp80::ONE]),
                    (*program, [Fp80::ONE, operand]),
                ]
            })
            .chain(unary.iter().map(|program| (*program, [operand, Fp80::ONE])));
        for (program, stack) in cases {
            let result = run_x87(program, &stack);
            assert_eq!(result.st(0), Fp80::INDEFINITE, "{program:02X?} {stack:?}");
            assert_eq!(
                result.status_word & 0x00FF,
                0x0001,
                "{program:02X?} {stack:?}"
            );
        }
    }
}

#[test]
fn compare_with_an_unsupported_encoding_is_not_comparable() {
    let programs: [&[u8]; 3] = [&[0xD8, 0xD1], &[0xDD, 0xE1], &[0xD9, 0xE4]];
    for operand in UNSUPPORTED_OPERANDS {
        for program in programs {
            let result = run_x87(program, &[operand, Fp80::ONE]);
            assert!(result.c3() && result.c2() && result.c0(), "{program:02X?}");
            assert_eq!(result.status_word & 0x00FF, 0x0001, "{program:02X?}");
        }
    }
}

#[test]
fn pseudo_denormal_result_takes_the_exponent_field_one() {
    let value = Fp80::from_bits(0x0001, 0x8000_0000_0000_0000);
    let result = run_x87(&[0xD8, 0xC1], &[POSITIVE_PSEUDO_DENORMAL, Fp80::ZERO]);
    assert_eq!(result.st(0), value);
    assert_eq!(result.status_word & 0x00FF, 0x0002);

    let result = run_x87(&[0xD9, 0xF8], &[POSITIVE_PSEUDO_DENORMAL, Fp80::ONE]);
    assert_eq!(result.st(0), value);
    assert_eq!(result.status_word & 0x00FF, 0x0002);

    let result = run_x87(&[0xD9, 0xFE], &[POSITIVE_PSEUDO_DENORMAL]);
    assert_eq!(result.st(0), value);
    assert_eq!(result.status_word & 0x00FF, 0x0022);
}
