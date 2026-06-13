use common::{Bus as _, CpuZ80 as _};
use cpu::Z80;

struct TestBus {
    ram: Box<[u8; 65_536]>,
    irq: bool,
    acknowledge_opcode: u8,
    acknowledge_count: usize,
    current_cycle: u64,
}

impl TestBus {
    fn new(acknowledge_opcode: u8) -> Self {
        Self {
            ram: vec![0; 65_536].into_boxed_slice().try_into().unwrap(),
            irq: true,
            acknowledge_opcode,
            acknowledge_count: 0,
            current_cycle: 0,
        }
    }
}

impl common::Bus for TestBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        self.ram[(address & 0xFFFF) as usize]
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.ram[(address & 0xFFFF) as usize] = value;
    }

    fn io_read_byte(&mut self, _port: u16) -> u8 {
        0xFF
    }

    fn io_write_byte(&mut self, _port: u16, _value: u8) {}

    fn has_irq(&self) -> bool {
        self.irq
    }

    fn acknowledge_irq(&mut self) -> u8 {
        self.irq = false;
        self.acknowledge_count += 1;
        self.acknowledge_opcode
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

fn cpu_in_interrupt_mode_0() -> Z80 {
    let mut cpu = Z80::new(4_000_000);
    cpu.state.iff1 = true;
    cpu.state.iff2 = true;
    cpu.state.im = 0;
    cpu.state.sp = 0x8000;
    cpu
}

#[test]
fn im0_nop_acknowledge_does_not_push_pc() {
    let mut cpu = cpu_in_interrupt_mode_0();
    let mut bus = TestBus::new(0x00);

    cpu.run_for(32, &mut bus);

    assert_eq!(bus.acknowledge_count, 1);
    assert_eq!(cpu.state.sp, 0x8000, "IM0 NOP has no stack effect");
}

#[test]
fn im0_rst_acknowledge_pushes_via_rst_instruction() {
    let mut cpu = cpu_in_interrupt_mode_0();
    let mut bus = TestBus::new(0xFF);

    cpu.run_for(32, &mut bus);

    assert_eq!(bus.acknowledge_count, 1);
    assert_eq!(cpu.state.sp, 0x7FFE);
    assert_eq!(bus.read_word(0x7FFE), 0x0001);
}
