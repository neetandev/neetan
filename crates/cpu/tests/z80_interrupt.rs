use common::{Bus as _, CpuZ80 as _};
use cpu::Z80;

struct TestBus {
    ram: Box<[u8; 65_536]>,
    irq: bool,
    acknowledge_opcode: u8,
    acknowledge_count: usize,
    reti_count: usize,
    current_cycle: u64,
}

impl TestBus {
    fn new(acknowledge_opcode: u8) -> Self {
        Self {
            ram: vec![0; 65_536].into_boxed_slice().try_into().unwrap(),
            irq: true,
            acknowledge_opcode,
            acknowledge_count: 0,
            reti_count: 0,
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

    fn notify_reti(&mut self) {
        self.reti_count += 1;
    }

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

/// Runs one instruction placed at 0x0000 with a return address staged on the
/// stack, returning the bus so the test can inspect the daisy-chain callback.
fn execute_return_opcode(opcode: [u8; 2]) -> TestBus {
    let mut cpu = Z80::new(4_000_000);
    cpu.state.iff1 = true;
    cpu.state.iff2 = true;
    cpu.state.sp = 0x8000;
    cpu.state.pc = 0x0000;

    let mut bus = TestBus::new(0x00);
    bus.irq = false;
    bus.ram[0x0000] = opcode[0];
    bus.ram[0x0001] = opcode[1];
    bus.ram[0x8000] = 0x34;
    bus.ram[0x8001] = 0x12;

    cpu.run_for(14, &mut bus);
    assert_eq!(cpu.pc(), 0x1234, "the return address is popped");
    assert_eq!(cpu.state.sp, 0x8002);
    bus
}

#[test]
fn reti_notifies_the_daisy_chain() {
    // ED 4D = RETI: the CPU signals the interrupt daisy chain.
    let bus = execute_return_opcode([0xED, 0x4D]);
    assert_eq!(bus.reti_count, 1);
}

#[test]
fn retn_does_not_notify_the_daisy_chain() {
    // ED 45 = RETN: shares the return path but must not signal RETI.
    let bus = execute_return_opcode([0xED, 0x45]);
    assert_eq!(bus.reti_count, 0);
}

/// IM0 adds two clocks to the supplied instruction. IM1 and IM2 take 13 and 19.
#[test]
fn acknowledge_clocks() {
    for (mode, opcode, expected) in [(0, 0x00, 6), (0, 0xFF, 13), (1, 0xFF, 13), (2, 0xFF, 19)] {
        let mut cpu = cpu_in_interrupt_mode_0();
        cpu.state.im = mode;
        cpu.state.pc = 0x1233;
        cpu.state.i = 0x20;
        let mut bus = TestBus::new(opcode);
        bus.ram[0x20FF..0x2101].copy_from_slice(&[0x00, 0x40]);

        assert_eq!(cpu.run_for(4, &mut bus), 4, "sample IRQ after a NOP");

        // This core also executes the first instruction after acknowledging.
        assert_eq!(
            cpu.run_for(1, &mut bus),
            expected + 4,
            "mode {mode}, opcode {opcode:02X}"
        );
        assert_eq!(bus.current_cycle(), expected + 8);
        assert_eq!(bus.acknowledge_count, 1);
        assert!(!cpu.state.iff1);
        assert!(!cpu.state.iff2);
        assert_eq!(cpu.state.r(), 3);
        if opcode == 0x00 {
            assert_eq!(cpu.state.sp, 0x8000);
            assert_eq!(cpu.pc(), 0x1235);
        } else {
            assert_eq!(cpu.state.sp, 0x7FFE);
            assert_eq!(bus.read_word(0x7FFE), 0x1234);
            assert_eq!(cpu.pc(), if mode == 2 { 0x4001 } else { 0x0039 });
        }
    }
}

/// HALT repeats four-clock M1 cycles without executing the following instruction.
#[test]
fn halt_runs_m1_cycles() {
    let mut cpu = cpu_in_interrupt_mode_0();
    cpu.state.iff1 = false;
    let mut bus = TestBus::new(0xFF);
    bus.ram[..2].copy_from_slice(&[0x76, 0x3C]);

    assert_eq!(cpu.run_for(4, &mut bus), 4);
    for refresh in 2..=4 {
        assert_eq!(cpu.run_for(4, &mut bus), 4);
        assert_eq!(cpu.state.r(), refresh);
    }
    assert!(cpu.halted());
    assert_eq!(cpu.pc(), 1);
    assert_eq!(cpu.state.a, 0);
    assert_eq!(bus.acknowledge_count, 0, "a masked IRQ must not end HALT");
    assert_eq!(bus.current_cycle(), 16);
}

/// The test stepping entry point also advances HALT and preserves refresh bit 7.
#[test]
fn halted_step_wraps_refresh_low_bits() {
    for high_bit in [0x00, 0x80] {
        let mut cpu = cpu_in_interrupt_mode_0();
        cpu.state.halted = true;
        cpu.state.pc = 1;
        cpu.state.set_r(high_bit | 0x7F);
        let mut bus = TestBus::new(0xFF);
        bus.irq = false;

        cpu.step(&mut bus);

        assert_eq!(cpu.cycles_consumed(), 4);
        assert_eq!(cpu.state.r(), high_bit);
        assert_eq!(cpu.pc(), 1);
        assert!(cpu.halted());
    }
}

/// An accepted IRQ ends HALT and saves the address following HALT.
#[test]
fn interrupt_ends_the_halt() {
    let mut cpu = cpu_in_interrupt_mode_0();
    cpu.state.im = 1;
    let mut bus = TestBus::new(0xFF);
    bus.irq = false;
    bus.ram[0] = 0x76;

    assert_eq!(cpu.run_for(8, &mut bus), 8);
    bus.irq = true;
    assert_eq!(cpu.run_for(1, &mut bus), 13 + 4);

    assert!(!cpu.halted());
    assert_eq!(cpu.pc(), 0x0039, "the handler's first NOP also executes");
    assert_eq!(bus.read_word(0x7FFE), 1);
    assert_eq!(cpu.state.r(), 4);
}

/// DI masks a raised IRQ until EI and its following HALT have executed.
#[test]
fn di_masks_interrupt_until_after_ei_and_halt() {
    let mut cpu = cpu_in_interrupt_mode_0();
    cpu.state.im = 1;
    let mut bus = TestBus::new(0xFF);
    bus.ram[..4].copy_from_slice(&[0xF3, 0x00, 0xFB, 0x76]);

    for _ in 0..4 {
        assert_eq!(cpu.run_for(4, &mut bus), 4);
        assert_eq!(bus.acknowledge_count, 0);
    }
    assert!(cpu.halted());
    assert_eq!(cpu.pc(), 4);
    assert_eq!(cpu.run_for(1, &mut bus), 13 + 4);
    assert_eq!(bus.acknowledge_count, 1);
    assert!(!cpu.halted());
    assert_eq!(bus.read_word(0x7FFE), 4);
}
