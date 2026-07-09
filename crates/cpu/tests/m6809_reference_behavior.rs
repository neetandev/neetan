use common::{Bus, Cpu6809};
use cpu::{M6809, M6809Flags, M6809State};

struct TestBus {
    memory: Box<[u8; 65_536]>,
    current_cycle: u64,
}

impl TestBus {
    fn new() -> Self {
        Self {
            memory: vec![0; 65_536].into_boxed_slice().try_into().unwrap(),
            current_cycle: 0,
        }
    }
}

impl Bus for TestBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        self.memory[(address & 0xFFFF) as usize]
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.memory[(address & 0xFFFF) as usize] = value;
    }

    fn io_read_byte(&mut self, port: u16) -> u8 {
        panic!("unexpected I/O read from {port:04X}");
    }

    fn io_write_byte(&mut self, port: u16, value: u8) {
        panic!("unexpected I/O write to {port:04X} = {value:02X}");
    }

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

fn processor_with_program(bus: &mut TestBus, program: &[u8], mut state: M6809State) -> M6809 {
    const PROGRAM_ADDRESS: u16 = 0x1000;
    bus.memory[usize::from(PROGRAM_ADDRESS)..][..program.len()].copy_from_slice(program);
    state.pc = PROGRAM_ADDRESS;
    let mut processor = M6809::new(2_000_000);
    processor.load_state(&state);
    processor
}

#[test]
fn mismatched_tfr_prefixes_eight_bit_values_with_ff() {
    for (register, value) in [(0x0A, 0x35), (0x0B, 0x47)] {
        let mut bus = TestBus::new();
        let state = M6809State {
            x: 0x1234,
            dp: 0x47,
            flags: M6809Flags::new(0x35),
            ..M6809State::default()
        };
        let mut processor =
            processor_with_program(&mut bus, &[0x1F, (register << 4) | 0x01], state);
        processor.step(&mut bus);
        assert_eq!(processor.x, 0xFF00 | value);
    }
}

#[test]
fn mismatched_exg_prefixes_eight_bit_values_with_ff() {
    for (register, value) in [(0x0A, 0x35), (0x0B, 0x47)] {
        let mut bus = TestBus::new();
        let state = M6809State {
            x: 0x1234,
            dp: 0x47,
            flags: M6809Flags::new(0x35),
            ..M6809State::default()
        };
        let mut processor =
            processor_with_program(&mut bus, &[0x1E, (register << 4) | 0x01], state);
        processor.step(&mut bus);
        assert_eq!(processor.x, 0xFF00 | value);
    }
}

#[test]
fn cwai_interrupt_does_not_stack_state_twice() {
    let mut bus = TestBus::new();
    bus.memory[0xFFF6] = 0x23;
    bus.memory[0xFFF7] = 0x45;
    let state = M6809State {
        s: 0x3000,
        ..M6809State::default()
    };
    let mut processor = processor_with_program(&mut bus, &[0x3C, 0xBF], state);

    processor.step(&mut bus);
    let stacked_s = processor.s;
    processor.request_firq();
    processor.step(&mut bus);

    assert_eq!(stacked_s, 0x2FF4);
    assert_eq!(processor.s, stacked_s);
    assert_eq!(processor.pc, 0x2345);
    assert!(processor.flags.entire);
    assert!(processor.flags.irq_mask);
    assert!(processor.flags.firq_mask);
}

#[test]
fn extended_clr_yields_between_read_and_write() {
    let mut bus = TestBus::new();
    bus.memory[0x2000] = 0xA5;
    let mut processor =
        processor_with_program(&mut bus, &[0x7F, 0x20, 0x00], M6809State::default());

    assert_eq!(processor.run_for(100, &mut bus), 4);
    assert_eq!(processor.pc, 0x1003);
    assert_eq!(bus.memory[0x2000], 0xA5);

    assert_eq!(processor.run_for(3, &mut bus), 3);
    assert_eq!(bus.memory[0x2000], 0);
    assert!(processor.flags.zero);
    assert!(!processor.flags.negative);
    assert!(!processor.flags.overflow);
    assert!(!processor.flags.carry);
}
