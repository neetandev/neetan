//! End-to-end CPU verification using the test386.asm self-test ROM.
//!
//! The ROM is built from vendored upstream sources under
//! `test386/upstream/` and assembled into `test386/test386.bin` via the
//! Makefile next to it. The ROM runs ~25 numbered test phases across real
//! mode, protected mode, ring-3, VM86, and paging, then HLTs with POST code
//! 0xFF on success. See `test386/upstream/README.md` for the POST code
//! table.

use common::{Bus, Cpu};
use cpu::{CPU_MODEL_386_DX, I386};

const ROM_BYTES: &[u8] = include_bytes!("test386/test386.bin");
const EE_REFERENCE: &str = include_str!("test386/upstream/test386-EE-reference.txt");

const RAM_SIZE: usize = 16 * 1024 * 1024;
const ADDRESS_MASK: u32 = 0x00FF_FFFF;
const ROM_BASE: usize = 0x000F_0000;

// Must match the EQUs in tests/test386/configuration.asm.
const POST_PORT: u16 = 0x0190;
const OUT_PORT: u16 = 0x00E9;

const MAX_STEPS: u64 = 500_000_000;

struct TestBus {
    ram: Vec<u8>,
    post_history: Vec<u8>,
    last_post: Option<u8>,
    ascii_output: Vec<u8>,
}

impl TestBus {
    fn new() -> Self {
        let mut ram = vec![0u8; RAM_SIZE];
        ram[ROM_BASE..ROM_BASE + ROM_BYTES.len()].copy_from_slice(ROM_BYTES);
        Self {
            ram,
            post_history: Vec::new(),
            last_post: None,
            ascii_output: Vec::new(),
        }
    }
}

impl Bus for TestBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        self.ram[(address & ADDRESS_MASK) as usize]
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.ram[(address & ADDRESS_MASK) as usize] = value;
    }

    fn io_read_byte(&mut self, _port: u16) -> u8 {
        0xFF
    }

    fn io_write_byte(&mut self, port: u16, value: u8) {
        match port {
            POST_PORT => {
                self.post_history.push(value);
                self.last_post = Some(value);
            }
            OUT_PORT => {
                self.ascii_output.push(value);
            }
            _ => {}
        }
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
        0
    }

    fn set_current_cycle(&mut self, _cycle: u64) {}
}

fn run_until_halt(cpu: &mut I386<CPU_MODEL_386_DX>, bus: &mut TestBus) -> u64 {
    let mut steps = 0u64;
    while !cpu.halted() && steps < MAX_STEPS {
        cpu.step(bus);
        steps += 1;
    }
    steps
}

fn format_post_history(history: &[u8]) -> String {
    history
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn test386_reaches_post_ff() {
    let mut cpu: I386<CPU_MODEL_386_DX> = I386::new();
    let mut bus = TestBus::new();

    let steps = run_until_halt(&mut cpu, &mut bus);

    assert!(
        cpu.halted(),
        "CPU did not HLT within {MAX_STEPS} steps (ran {steps}); \
         last POST: {:?}, history: [{}]",
        bus.last_post,
        format_post_history(&bus.post_history),
    );

    let last = bus.last_post.unwrap_or(0);
    assert_eq!(
        last,
        0xFF,
        "test386.asm failed at POST 0x{last:02X} after {steps} steps; \
         history: [{}]; consult crates/cpu/tests/test386/upstream/README.md \
         POST table and crates/cpu/tests/test386/test386.lst",
        format_post_history(&bus.post_history),
    );
}

#[test]
fn test386_ee_output_matches_reference() {
    let mut cpu: I386<CPU_MODEL_386_DX> = I386::new();
    let mut bus = TestBus::new();
    run_until_halt(&mut cpu, &mut bus);

    assert_eq!(
        bus.last_post,
        Some(0xFF),
        "precondition: ROM must reach POST 0xFF; last POST: {:?}",
        bus.last_post,
    );

    let produced = std::str::from_utf8(&bus.ascii_output)
        .expect("EE output must be valid UTF-8 (ROM emits ASCII only)");

    // Compare line-by-line so CRLF-on-checkout on Windows (where git can
    // convert the committed LF-terminated reference to CRLF) does not cause
    // a byte-level mismatch here. `str::lines` strips both `\n` and `\r\n`.
    if produced.lines().eq(EE_REFERENCE.lines()) {
        return;
    }

    let mismatch = produced
        .lines()
        .zip(EE_REFERENCE.lines())
        .enumerate()
        .find(|(_, (got, want))| got != want);

    match mismatch {
        Some((lineno, (got, want))) => panic!(
            "EE output diverges at line {}:\n  got:  {got}\n  want: {want}",
            lineno + 1,
        ),
        None => {
            let produced_lines = produced.lines().count();
            let expected_lines = EE_REFERENCE.lines().count();
            panic!(
                "EE output length differs: produced {produced_lines} lines, \
                 expected {expected_lines} lines",
            );
        }
    }
}
