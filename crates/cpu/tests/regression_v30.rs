use common::{Bus as _, Cpu as _};
use cpu::{V30, V30State};

const RAM_SIZE: usize = 1024 * 1024;
const ADDRESS_MASK: u32 = 0x000F_FFFF;

struct TestBus {
    ram: Vec<u8>,
    current_cycle: u64,
}

impl TestBus {
    fn new() -> Self {
        Self {
            ram: vec![0; RAM_SIZE],
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

fn place_code(bus: &mut TestBus, cs: u16, ip: u16, code: &[u8]) {
    let base = u32::from(cs) << 4;
    for (index, &byte) in code.iter().enumerate() {
        bus.write_byte(base + u32::from(ip) + index as u32, byte);
    }
}

#[test]
fn undefined_register_form_lea_les_lds_does_not_panic() {
    let mut cpu = V30::new();
    let mut bus = TestBus::new();

    #[rustfmt::skip]
    place_code(&mut bus, 0x1000, 0x0100, &[
        0x8D, 0xC0,       // LEA AX, AX
        0xC4, 0xC0,       // LES AX, AX
        0xC5, 0xC0,       // LDS AX, AX
        0xF4,             // HLT
    ]);

    let mut state = V30State::default();
    state.set_cs(0x1000);
    state.set_ss(0x2000);
    state.set_sp(0x1000);
    state.ip = 0x0100;
    state.initialize_cold_frontend();
    cpu.load_state(&state);

    for _ in 0..4 {
        cpu.step(&mut bus);
    }

    assert!(
        cpu.halted(),
        "V30 should execute past undefined register-form LEA/LES/LDS"
    );
}
