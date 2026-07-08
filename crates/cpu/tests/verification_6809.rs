#![cfg(feature = "verification")]

#[path = "common/verification_common.rs"]
mod verification_common;

use std::{collections::HashMap, fs, path::Path, sync::LazyLock};

use cpu::{M6809, M6809State};
use verification_common::{load_moo_tests, load_revocation_list};

const REG_ORDER_6809: [&str; 9] = ["pc", "s", "u", "x", "y", "dp", "a", "b", "cc"];

struct TestBus {
    ram: Box<[u8; 65_536]>,
    current_cycle: u64,
    wait_cycles: i64,
}

impl TestBus {
    fn new() -> Self {
        Self {
            ram: vec![0u8; 65_536].into_boxed_slice().try_into().unwrap(),
            current_cycle: 0,
            wait_cycles: 0,
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

    fn io_read_byte(&mut self, port: u16) -> u8 {
        panic!("unexpected 6809 I/O read from 0x{port:04X}");
    }

    fn io_write_byte(&mut self, port: u16, value: u8) {
        panic!("unexpected 6809 I/O write to 0x{port:04X} = 0x{value:02X}");
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

    fn drain_wait_cycles(&mut self) -> i64 {
        let wait_cycles = self.wait_cycles;
        self.wait_cycles = 0;
        wait_cycles
    }
}

fn test_dir() -> &'static Path {
    static DIR: LazyLock<std::path::PathBuf> = LazyLock::new(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/SingleStepTests/6809/v1")
    });
    &DIR
}

fn manifest_stems() -> Vec<String> {
    let manifest = fs::read_to_string(test_dir().join("manifest.txt")).unwrap();
    manifest
        .lines()
        .filter_map(|line| line.rsplit_once(' ').map(|(stem, _)| stem.to_string()))
        .collect()
}

fn initial_reg_value(regs: &HashMap<String, u32>, name: &str) -> u32 {
    regs.get(name)
        .copied()
        .unwrap_or_else(|| panic!("missing register in initial state: {name}"))
}

fn resolve_regs(
    initial: &HashMap<String, u32>,
    final_state: &HashMap<String, u32>,
) -> HashMap<String, u32> {
    REG_ORDER_6809
        .iter()
        .map(|name| {
            let value = final_state
                .get(*name)
                .copied()
                .unwrap_or_else(|| initial_reg_value(initial, name));
            ((*name).to_string(), value)
        })
        .collect()
}

fn build_state(regs: &HashMap<String, u32>) -> M6809State {
    let get = |name: &str| -> u32 {
        regs.get(name)
            .copied()
            .unwrap_or_else(|| panic!("missing register: {name}"))
    };

    let mut state = M6809State {
        pc: get("pc") as u16,
        s: get("s") as u16,
        u: get("u") as u16,
        x: get("x") as u16,
        y: get("y") as u16,
        dp: get("dp") as u8,
        a: get("a") as u8,
        b: get("b") as u8,
        ..M6809State::default()
    };
    state.flags.expand(get("cc") as u8);
    state
}

fn format_flags_diff(expected: u8, actual: u8) -> String {
    const FLAG_BITS: &[(u8, &str)] = &[
        (0x80, "E"),
        (0x40, "F"),
        (0x20, "H"),
        (0x10, "I"),
        (0x08, "N"),
        (0x04, "Z"),
        (0x02, "V"),
        (0x01, "C"),
    ];

    let diff_bits = expected ^ actual;
    let mut changed = Vec::new();
    for &(bit, name) in FLAG_BITS {
        if diff_bits & bit != 0 {
            changed.push(format!(
                "{name}:{}->{}",
                u8::from(expected & bit != 0),
                u8::from(actual & bit != 0)
            ));
        }
    }
    format!(
        "  cc: expected 0x{expected:02X}, got 0x{actual:02X} [{}]",
        changed.join(", ")
    )
}

fn run_test_file(stem: &str, revoked_hashes: &std::collections::HashSet<String>) {
    let path = test_dir().join(format!("{stem}.moo.gz"));
    let test_cases = load_moo_tests(&path, &REG_ORDER_6809, &[]);

    let mut failures = Vec::new();

    for (index, test) in test_cases.iter().enumerate() {
        if test
            .hash
            .as_ref()
            .is_some_and(|hash| revoked_hashes.contains(hash))
        {
            continue;
        }

        let mut bus = TestBus::new();
        for &(address, value) in &test.initial.ram {
            bus.ram[(address & 0xFFFF) as usize] = value;
        }

        let initial_regs = resolve_regs(&test.initial.regs, &HashMap::new());
        let final_regs = resolve_regs(&test.initial.regs, &test.final_state.regs);
        let initial_state = build_state(&initial_regs);
        let expected = build_state(&final_regs);

        let mut cpu = M6809::new(1_000_000);
        cpu.load_state(&initial_state);
        cpu.step(&mut bus);

        let mut diffs = Vec::new();

        let check_u8 = |name: &str,
                        initial_value: u8,
                        actual_value: u8,
                        expected_value: u8,
                        diffs: &mut Vec<String>| {
            if actual_value != expected_value {
                diffs.push(format!(
                    "  {name}: expected 0x{expected_value:02X}, got 0x{actual_value:02X} (was 0x{initial_value:02X})"
                ));
            }
        };
        let check_u16 = |name: &str,
                         initial_value: u16,
                         actual_value: u16,
                         expected_value: u16,
                         diffs: &mut Vec<String>| {
            if actual_value != expected_value {
                diffs.push(format!(
                    "  {name}: expected 0x{expected_value:04X}, got 0x{actual_value:04X} (was 0x{initial_value:04X})"
                ));
            }
        };

        check_u16("pc", initial_state.pc, cpu.pc, expected.pc, &mut diffs);
        check_u16("s", initial_state.s, cpu.s, expected.s, &mut diffs);
        check_u16("u", initial_state.u, cpu.u, expected.u, &mut diffs);
        check_u16("x", initial_state.x, cpu.x, expected.x, &mut diffs);
        check_u16("y", initial_state.y, cpu.y, expected.y, &mut diffs);
        check_u8("dp", initial_state.dp, cpu.dp, expected.dp, &mut diffs);
        check_u8("a", initial_state.a, cpu.a, expected.a, &mut diffs);
        check_u8("b", initial_state.b, cpu.b, expected.b, &mut diffs);
        if cpu.flags.compress() != expected.flags.compress() {
            diffs.push(format!(
                "{} (was 0x{:02X})",
                format_flags_diff(expected.flags.compress(), cpu.flags.compress()),
                initial_state.flags.compress()
            ));
        }

        let actual_cycles = cpu.cycles_consumed();
        let expected_cycles = test.cycles.len() as u64;
        if actual_cycles != expected_cycles {
            diffs.push(format!(
                "  cycles: expected {expected_cycles}, got {actual_cycles}"
            ));
        }

        for &(address, expected_value) in &test.final_state.ram {
            let actual_value = bus.ram[(address & 0xFFFF) as usize];
            if actual_value != expected_value {
                let initial_value = test
                    .initial
                    .ram
                    .iter()
                    .find(|(candidate, _)| *candidate == address)
                    .map(|(_, value)| *value);
                match initial_value {
                    Some(before) => diffs.push(format!(
                        "  ram[0x{address:04X}]: expected 0x{expected_value:02X}, got 0x{actual_value:02X} (was 0x{before:02X})"
                    )),
                    None => diffs.push(format!(
                        "  ram[0x{address:04X}]: expected 0x{expected_value:02X}, got 0x{actual_value:02X} (not in initial RAM)"
                    )),
                }
            }
        }

        if !diffs.is_empty() {
            let bytes_hex: Vec<String> = test
                .bytes
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect();
            failures.push(format!(
                "[{} #{index}] {} ({})\n{}",
                path.file_name().unwrap().to_string_lossy(),
                test.name,
                bytes_hex.join(" "),
                diffs.join("\n")
            ));
        }
    }

    if !failures.is_empty() {
        let fail_count = failures.len();
        let test_count = test_cases.len();
        let mut message = format!("{stem}.moo.gz: {fail_count}/{test_count} tests failed\n");
        for failure in failures.iter().take(5) {
            message.push_str(failure);
            message.push('\n');
        }
        if failures.len() > 5 {
            message.push_str(&format!("  ... and {} more failures\n", failures.len() - 5));
        }
        panic!("{message}");
    }
}

#[test]
fn all_6809_vectors() {
    let revocation_path = test_dir().join("revocation_list.txt");
    let revoked_hashes = if revocation_path.exists() {
        load_revocation_list(&revocation_path)
    } else {
        std::collections::HashSet::new()
    };

    for stem in manifest_stems() {
        run_test_file(&stem, &revoked_hashes);
    }
}
