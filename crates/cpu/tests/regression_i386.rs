use cpu::{I386, I386State};

const RAM_SIZE: usize = 1024 * 1024;
const ADDRESS_MASK: u32 = 0x000F_FFFF;
const CODE_BASE: u32 = 0x100;

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

fn place_code(bus: &mut TestBus, bytes: &[u8]) {
    for (offset, &byte) in bytes.iter().enumerate() {
        bus.ram[CODE_BASE as usize + offset] = byte;
    }
}

fn real_mode_cpu() -> I386 {
    let mut state = I386State::default();
    state.set_cs(0);
    state.set_eip(CODE_BASE);
    state.set_eflags(0x0000_0202);
    let mut cpu = I386::new();
    cpu.load_state(&state);
    cpu
}

// F3 66 0F BC C2 -> `rep bsf eax, edx` (32-bit operand). The F3 prefix must be
// ignored: bsf runs once even though ECX is zero, and ECX stays untouched.
#[test]
fn rep_bsf_executes_once_and_ignores_ecx() {
    let mut bus = TestBus::new();
    place_code(&mut bus, &[0xF3, 0x66, 0x0F, 0xBC, 0xC2]);

    let mut cpu = real_mode_cpu();
    cpu.state.set_edx(0x0000_0F00);
    cpu.state.set_ecx(0);
    cpu.state.set_eax(0xDEAD_BEEF);

    cpu.step(&mut bus);

    assert_eq!(
        cpu.state.eax(),
        8,
        "bsf should find the lowest set bit (bit 8)"
    );
    assert!(
        !cpu.state.flags.zf(),
        "ZF must be clear for a nonzero source"
    );
    assert_eq!(
        cpu.state.ecx(),
        0,
        "ECX must not be consumed as a repeat count"
    );
    assert_eq!(
        cpu.state.eip(),
        CODE_BASE + 5,
        "IP must advance past the single bsf instruction"
    );
}

// Same encoding with a zero source: ZF set, instruction still runs once.
#[test]
fn rep_bsf_zero_source_sets_zf() {
    let mut bus = TestBus::new();
    place_code(&mut bus, &[0xF3, 0x66, 0x0F, 0xBC, 0xC2]);

    let mut cpu = real_mode_cpu();
    cpu.state.set_edx(0);
    cpu.state.set_ecx(0x1234);

    cpu.step(&mut bus);

    assert!(cpu.state.flags.zf(), "ZF must be set for a zero source");
    assert_eq!(cpu.state.ecx(), 0x1234, "ECX must be left unchanged");
    assert_eq!(cpu.state.eip(), CODE_BASE + 5);
}
