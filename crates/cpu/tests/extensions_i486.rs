use common::{Bus as _, Cpu as _};
use cpu::{CPU_MODEL_486_DX, I386, I386State};

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

fn place_code(bus: &mut TestBus, cs: u16, ip: u16, code: &[u8]) {
    let base = (cs as u32) << 4;
    for (i, &byte) in code.iter().enumerate() {
        bus.write_byte(base + ip as u32 + i as u32, byte);
    }
}

fn setup_ivt_entry(bus: &mut TestBus, vector: u8, handler_cs: u16, handler_ip: u16) {
    let addr = (vector as usize) * 4;
    bus.ram[addr] = handler_ip as u8;
    bus.ram[addr + 1] = (handler_ip >> 8) as u8;
    bus.ram[addr + 2] = handler_cs as u8;
    bus.ram[addr + 3] = (handler_cs >> 8) as u8;
}

fn make_486dx() -> I386<{ CPU_MODEL_486_DX }> {
    I386::<{ CPU_MODEL_486_DX }>::new()
}

fn setup_state(cs: u16, ip: u16) -> I386State {
    let mut state = I386State::default();
    state.set_cs(cs);
    state.set_eip(ip as u32);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    state
}

#[test]
fn i486dx_cr0_after_reset() {
    let cpu = make_486dx();
    assert_eq!(
        cpu.cr0, 0x0000_0010,
        "486DX reset: ET=1 (hardwired, on-chip FPU), PE=0"
    );
}

#[test]
fn i486dx_cr0_writable_bits_round_trip() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    // 0x6005_0030: bit 30 (CD), 29 (NW), 18 (AM), 16 (WP), 5 (NE), 4 (ET).
    // PG and PE are intentionally clear so paging/protection are not enabled.
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0x66, 0xB8, 0x30, 0x00, 0x05, 0x60, // MOV EAX, 0x6005_0030
            0x0F, 0x22, 0xC0, // MOV CR0, EAX
            0x66, 0x31, 0xDB, // XOR EBX, EBX
            0x0F, 0x20, 0xC3, // MOV EBX, CR0
        ],
    );

    let state = setup_state(cs, ip);
    cpu.load_state(&state);

    cpu.step(&mut bus); // MOV EAX
    cpu.step(&mut bus); // MOV CR0, EAX
    assert_eq!(
        cpu.cr0, 0x6005_0030,
        "486 must keep NE+AM+NW+CD+WP+ET when written to CR0"
    );

    cpu.step(&mut bus); // XOR EBX, EBX
    cpu.step(&mut bus); // MOV EBX, CR0
    assert_eq!(
        cpu.ebx(),
        0x6005_0030,
        "MOV r32, CR0 must read back the same writable bits"
    );
}

#[test]
fn i486dx_cr0_reserved_bits_masked_off() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    // Try to set every CR0 bit except PE (bit 0) and PG (bit 31). The mask must
    // strip bits 6-15, 17, 19-28 because those are reserved on 486.
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0x66, 0xB8, 0xFE, 0xFF, 0xFF, 0x7F, // MOV EAX, 0x7FFF_FFFE
            0x0F, 0x22, 0xC0, // MOV CR0, EAX
        ],
    );

    let state = setup_state(cs, ip);
    cpu.load_state(&state);

    cpu.step(&mut bus); // MOV EAX
    cpu.step(&mut bus); // MOV CR0, EAX
    // 0x7FFF_FFFE & 0xE005_003F = 0x6005_003E (PE was clear, PG was clear).
    assert_eq!(
        cpu.cr0, 0x6005_003E,
        "CR0 reserved bits must be masked off on 486"
    );
}

#[test]
fn i486dx_bswap_eax() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    // Need operand-size override prefix for MOV EAX,imm32 in 16-bit mode.
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0x66, 0xB8, 0xEF, 0xBE, 0xAD, 0xDE, // MOV EAX, 0xDEADBEEF
            0x0F, 0xC8, // BSWAP EAX
        ],
    );

    let state = setup_state(cs, ip);
    cpu.load_state(&state);

    cpu.step(&mut bus); // MOV EAX, 0xDEADBEEF
    assert_eq!(cpu.eax(), 0xDEADBEEF);

    cpu.step(&mut bus); // BSWAP EAX
    assert_eq!(cpu.eax(), 0xEFBEADDE, "BSWAP should reverse byte order");
}

#[test]
fn i486dx_bswap_ecx() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0x66, 0xB9, 0x01, 0x02, 0x03, 0x04, // MOV ECX, 0x04030201
            0x0F, 0xC9, // BSWAP ECX
        ],
    );

    let state = setup_state(cs, ip);
    cpu.load_state(&state);

    cpu.step(&mut bus); // MOV ECX
    assert_eq!(cpu.ecx(), 0x04030201);

    cpu.step(&mut bus); // BSWAP ECX
    assert_eq!(cpu.ecx(), 0x01020304, "BSWAP ECX should reverse byte order");
}

#[test]
fn i486dx_bswap_zero() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0x0F, 0xC8, // BSWAP EAX (EAX=0 after reset)
        ],
    );

    let state = setup_state(cs, ip);
    cpu.load_state(&state);

    cpu.step(&mut bus);
    assert_eq!(cpu.eax(), 0, "BSWAP of zero should remain zero");
}

#[test]
fn i486dx_cmpxchg_byte_equal() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    // Set AL=0x42, CL=0x42 (dest), DL=0xFF (src)
    // CMPXCHG CL, DL: AL==CL -> CL=DL, ZF=1
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0xB0, 0x42, // MOV AL, 0x42
            0xB1, 0x42, // MOV CL, 0x42
            0xB2, 0xFF, // MOV DL, 0xFF
            0x0F, 0xB0, 0xD1, // CMPXCHG CL, DL (modrm=0xD1: reg=DL, rm=CL)
        ],
    );

    let state = setup_state(cs, ip);
    cpu.load_state(&state);

    cpu.step(&mut bus); // MOV AL
    cpu.step(&mut bus); // MOV CL
    cpu.step(&mut bus); // MOV DL
    cpu.step(&mut bus); // CMPXCHG

    assert_eq!(
        cpu.cl(),
        0xFF,
        "CMPXCHG equal: dest should be loaded with src (DL)"
    );
    assert_eq!(cpu.al(), 0x42, "CMPXCHG equal: AL should be unchanged");
    assert!(cpu.flags.zf(), "CMPXCHG equal: ZF should be set");
}

#[test]
fn i486dx_cmpxchg_byte_not_equal() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    // Set AL=0x10, CL=0x42 (dest), DL=0xFF (src)
    // CMPXCHG CL, DL: AL!=CL -> AL=CL, ZF=0
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0xB0, 0x10, // MOV AL, 0x10
            0xB1, 0x42, // MOV CL, 0x42
            0xB2, 0xFF, // MOV DL, 0xFF
            0x0F, 0xB0, 0xD1, // CMPXCHG CL, DL
        ],
    );

    let state = setup_state(cs, ip);
    cpu.load_state(&state);

    cpu.step(&mut bus); // MOV AL
    cpu.step(&mut bus); // MOV CL
    cpu.step(&mut bus); // MOV DL
    cpu.step(&mut bus); // CMPXCHG

    assert_eq!(
        cpu.cl(),
        0x42,
        "CMPXCHG not equal: dest should be unchanged"
    );
    assert_eq!(
        cpu.al(),
        0x42,
        "CMPXCHG not equal: AL should be loaded with dest"
    );
    assert!(!cpu.flags.zf(), "CMPXCHG not equal: ZF should be clear");
}

#[test]
fn i486dx_cmpxchg_word_equal() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    // AX=0x1234, CX=0x1234, DX=0xABCD
    // CMPXCHG CX, DX: AX==CX -> CX=DX, ZF=1
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0xB8, 0x34, 0x12, // MOV AX, 0x1234
            0xB9, 0x34, 0x12, // MOV CX, 0x1234
            0xBA, 0xCD, 0xAB, // MOV DX, 0xABCD
            0x0F, 0xB1, 0xD1, // CMPXCHG CX, DX
        ],
    );

    let state = setup_state(cs, ip);
    cpu.load_state(&state);

    cpu.step(&mut bus); // MOV AX
    cpu.step(&mut bus); // MOV CX
    cpu.step(&mut bus); // MOV DX
    cpu.step(&mut bus); // CMPXCHG

    assert_eq!(cpu.ecx() & 0xFFFF, 0xABCD, "CMPXCHG word equal: CX = DX");
    assert_eq!(
        cpu.eax() & 0xFFFF,
        0x1234,
        "CMPXCHG word equal: AX unchanged"
    );
    assert!(cpu.flags.zf(), "CMPXCHG word equal: ZF set");
}

#[test]
fn i486dx_cmpxchg_word_not_equal() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    // AX=0x0001, CX=0x0002, DX=0xABCD
    // CMPXCHG CX, DX: AX!=CX -> AX=CX, ZF=0
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0xB8, 0x01, 0x00, // MOV AX, 0x0001
            0xB9, 0x02, 0x00, // MOV CX, 0x0002
            0xBA, 0xCD, 0xAB, // MOV DX, 0xABCD
            0x0F, 0xB1, 0xD1, // CMPXCHG CX, DX
        ],
    );

    let state = setup_state(cs, ip);
    cpu.load_state(&state);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(
        cpu.ecx() & 0xFFFF,
        0x0002,
        "CMPXCHG word not equal: CX unchanged"
    );
    assert_eq!(
        cpu.eax() & 0xFFFF,
        0x0002,
        "CMPXCHG word not equal: AX = CX"
    );
    assert!(!cpu.flags.zf(), "CMPXCHG word not equal: ZF clear");
}

#[test]
fn i486dx_xadd_byte_reg() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    // CL=0x30 (dest), DL=0x12 (src)
    // XADD CL, DL: temp=CL, CL=CL+DL, DL=temp
    // Result: CL=0x42, DL=0x30
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0xB1, 0x30, // MOV CL, 0x30
            0xB2, 0x12, // MOV DL, 0x12
            0x0F, 0xC0, 0xD1, // XADD CL, DL
        ],
    );

    let state = setup_state(cs, ip);
    cpu.load_state(&state);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(cpu.cl(), 0x42, "XADD byte: dest = old_dest + src");
    assert_eq!(cpu.dl(), 0x30, "XADD byte: src = old_dest");
}

#[test]
fn i486dx_xadd_word_reg() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    // CX=0x1000, DX=0x0234
    // XADD CX, DX: CX=0x1234, DX=0x1000
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0xB9, 0x00, 0x10, // MOV CX, 0x1000
            0xBA, 0x34, 0x02, // MOV DX, 0x0234
            0x0F, 0xC1, 0xD1, // XADD CX, DX
        ],
    );

    let state = setup_state(cs, ip);
    cpu.load_state(&state);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert_eq!(
        cpu.ecx() & 0xFFFF,
        0x1234,
        "XADD word: dest = old_dest + src"
    );
    assert_eq!(cpu.edx() & 0xFFFF, 0x1000, "XADD word: src = old_dest");
}

#[test]
fn i486dx_invd_is_nop() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0x0F, 0x08, // INVD
        ],
    );

    let state = setup_state(cs, ip);
    cpu.load_state(&state);

    let start_ip = cpu.ip();
    cpu.step(&mut bus);

    assert_eq!(cpu.ip(), start_ip + 2, "INVD should advance IP by 2 bytes");
}

#[test]
fn i486dx_wbinvd_is_nop() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0x0F, 0x09, // WBINVD
        ],
    );

    let state = setup_state(cs, ip);
    cpu.load_state(&state);

    let start_ip = cpu.ip();
    cpu.step(&mut bus);

    assert_eq!(
        cpu.ip(),
        start_ip + 2,
        "WBINVD should advance IP by 2 bytes"
    );
}

#[test]
fn i486dx_invlpg_no_fault_real_mode() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    // INVLPG [BX] - modrm 0x3F: mod=00, reg=/7, rm=111 ([BX])
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0x0F, 0x01, 0x3F, // INVLPG [BX]
        ],
    );

    let state = setup_state(cs, ip);
    cpu.load_state(&state);

    let start_ip = cpu.ip();
    cpu.step(&mut bus);

    assert_eq!(
        cpu.ip(),
        start_ip + 3,
        "INVLPG should advance IP by 3 bytes"
    );
    assert_eq!(cpu.cs(), cs, "INVLPG should not change CS (no fault)");
}

#[test]
fn i386_bswap_triggers_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0050;
    place_code(&mut bus, cs, ip, &[0x0F, 0xC8]); // BSWAP EAX

    let handler_cs: u16 = 0x2000;
    let handler_ip: u16 = 0x0000;
    setup_ivt_entry(&mut bus, 6, handler_cs, handler_ip);

    let mut state = I386State::default();
    state.set_cs(cs);
    state.set_eip(ip as u32);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.cs(),
        handler_cs,
        "386: BSWAP should trigger #UD (INT 6)"
    );
    assert_eq!(cpu.ip() as u16, handler_ip);
}

#[test]
fn i386_cmpxchg_triggers_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0050;
    place_code(&mut bus, cs, ip, &[0x0F, 0xB0, 0xD1]); // CMPXCHG CL, DL

    let handler_cs: u16 = 0x2000;
    let handler_ip: u16 = 0x0000;
    setup_ivt_entry(&mut bus, 6, handler_cs, handler_ip);

    let mut state = I386State::default();
    state.set_cs(cs);
    state.set_eip(ip as u32);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.cs(),
        handler_cs,
        "386: CMPXCHG should trigger #UD (INT 6)"
    );
    assert_eq!(cpu.ip() as u16, handler_ip);
}

#[test]
fn i386_xadd_triggers_ud() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0050;
    place_code(&mut bus, cs, ip, &[0x0F, 0xC0, 0xD1]); // XADD CL, DL

    let handler_cs: u16 = 0x2000;
    let handler_ip: u16 = 0x0000;
    setup_ivt_entry(&mut bus, 6, handler_cs, handler_ip);

    let mut state = I386State::default();
    state.set_cs(cs);
    state.set_eip(ip as u32);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), handler_cs, "386: XADD should trigger #UD (INT 6)");
    assert_eq!(cpu.ip() as u16, handler_ip);
}

#[test]
fn i486dx_div_byte_toggles_af() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    // AX=100, CL=10 -> DIV CL -> AL=10, AH=0
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0xB8, 0x64, 0x00, // MOV AX, 100
            0xB1, 0x0A, // MOV CL, 10
            0xF6, 0xF1, // DIV CL
        ],
    );

    let mut state = setup_state(cs, ip);
    // Clear AF by setting aux_val=0 via eflags (AF is bit 4).
    state.set_eflags(0x0002); // bit 1 always set, AF=0
    cpu.load_state(&state);

    assert!(!cpu.flags.af(), "AF should start clear");

    cpu.step(&mut bus); // MOV AX
    cpu.step(&mut bus); // MOV CL
    cpu.step(&mut bus); // DIV CL

    assert!(cpu.flags.af(), "DIV byte: AF should be toggled from 0 to 1");

    // Run a second DIV to verify toggle back to 0.
    let ip2: u16 = 0x0100;
    place_code(
        &mut bus,
        cs,
        ip2,
        &[
            0xB8, 0x64, 0x00, // MOV AX, 100
            0xB1, 0x0A, // MOV CL, 10
            0xF6, 0xF1, // DIV CL
        ],
    );
    let mut state2 = setup_state(cs, ip2);
    state2.set_eflags(0x0012); // AF=1 (bit 4)
    cpu.load_state(&state2);

    assert!(cpu.flags.af(), "AF should start set");

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(
        !cpu.flags.af(),
        "DIV byte: AF should be toggled from 1 to 0"
    );
}

#[test]
fn i486dx_idiv_byte_toggles_af() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    // AX=100, CL=10 -> IDIV CL -> AL=10, AH=0
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0xB8, 0x64, 0x00, // MOV AX, 100
            0xB1, 0x0A, // MOV CL, 10
            0xF6, 0xF9, // IDIV CL
        ],
    );

    let mut state = setup_state(cs, ip);
    state.set_eflags(0x0002); // AF=0
    cpu.load_state(&state);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(
        cpu.flags.af(),
        "IDIV byte: AF should be toggled from 0 to 1"
    );
}

#[test]
fn i486dx_div_word_toggles_af() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    // DX:AX = 0:1000, CX=10 -> DIV CX -> AX=100, DX=0
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0xB8, 0xE8, 0x03, // MOV AX, 1000
            0xBA, 0x00, 0x00, // MOV DX, 0
            0xB9, 0x0A, 0x00, // MOV CX, 10
            0xF7, 0xF1, // DIV CX
        ],
    );

    let mut state = setup_state(cs, ip);
    state.set_eflags(0x0002); // AF=0
    cpu.load_state(&state);

    cpu.step(&mut bus); // MOV AX
    cpu.step(&mut bus); // MOV DX
    cpu.step(&mut bus); // MOV CX
    cpu.step(&mut bus); // DIV CX

    assert!(cpu.flags.af(), "DIV word: AF should be toggled from 0 to 1");
}

#[test]
fn i486dx_idiv_word_toggles_af() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0000;
    // DX:AX = 0:1000, CX=10 -> IDIV CX -> AX=100, DX=0
    place_code(
        &mut bus,
        cs,
        ip,
        &[
            0xB8, 0xE8, 0x03, // MOV AX, 1000
            0xBA, 0x00, 0x00, // MOV DX, 0
            0xB9, 0x0A, 0x00, // MOV CX, 10
            0xF7, 0xF9, // IDIV CX
        ],
    );

    let mut state = setup_state(cs, ip);
    state.set_eflags(0x0002); // AF=0
    cpu.load_state(&state);

    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);

    assert!(
        cpu.flags.af(),
        "IDIV word: AF should be toggled from 0 to 1"
    );
}

#[test]
fn i486dx_fpu_escape_no_fault() {
    let mut cpu = make_486dx();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0050;
    // FLDZ = D9 EE
    place_code(&mut bus, cs, ip, &[0xD9, 0xEE]);

    let handler_cs: u16 = 0x2000;
    let handler_ip: u16 = 0x0000;
    setup_ivt_entry(&mut bus, 7, handler_cs, handler_ip); // INT 7 = #NM

    let state = setup_state(cs, ip);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(cpu.cs(), cs, "486DX: FPU opcode should execute, not fault");
    assert_ne!(
        cpu.ip() as u16,
        ip,
        "486DX: IP should advance past the FPU opcode"
    );
}

#[test]
fn i386_fpu_escape_no_fault() {
    let mut cpu: I386 = I386::new();
    let mut bus = TestBus::new();

    let cs: u16 = 0x1000;
    let ip: u16 = 0x0050;
    // FLDZ = D9 EE
    place_code(&mut bus, cs, ip, &[0xD9, 0xEE]);

    let handler_cs: u16 = 0x2000;
    let handler_ip: u16 = 0x0000;
    setup_ivt_entry(&mut bus, 7, handler_cs, handler_ip);

    let mut state = I386State::default();
    state.set_cs(cs);
    state.set_eip(ip as u32);
    state.set_ss(0x3000);
    state.set_esp(0x1000);
    cpu.load_state(&state);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.cs(),
        cs,
        "386: FPU opcode should be silently consumed, not fault"
    );
    assert_ne!(
        cpu.ip() as u16,
        ip,
        "386: IP should advance past the FPU opcode"
    );
}
