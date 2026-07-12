#![cfg(feature = "verification")]

//! Aggregate timing calibration for the 80386SX CPU model.
//!
//! The SingleStepTests 80386 suite was captured from a 386EX, which - like the
//! 386SX - drives a 16-bit data bus, so its per-cycle traces are a usable proxy
//! for SX instruction lengths. This test sums the trace clock counts and the
//! emulated `cycles_consumed()` counts over the whole real-mode suite and
//! asserts they agree within a small tolerance. It is a calibration aid for the
//! SX timing constants in `i386.rs`, not a per-instruction cycle check.
//!
//! Two hardware artifacts of the capture are compensated for:
//!  - Each test terminates via an injected `HALT` captured through SMI/SMM,
//!    adding a fixed capture overhead per test (subtracted as
//!    [`SMM_CAPTURE_OVERHEAD_CLOCKS`], the free parameter that centers the
//!    aggregate; re-tune it if the per-opcode model changes).
//!  - The 386EX's SMM microcode lengthened IN/INS/OUT/OUTS/HLT/MOV CR0 by 1-4
//!    clocks over a plain SX, so those opcodes are excluded from the aggregate.
//!
//! Run with `--nocapture` for two reports: the worst stems by absolute delta,
//! and an "important opcodes" table (MOV/ALU/MUL/shifts/string/...) that removes
//! the per-test overhead to expose per-instruction error for tuning. Short
//! opcodes below a clock floor are overhead-noise-limited and left at their
//! datasheet timing; only high-clock outliers are flagged.

#[path = "common/verification_common.rs"]
mod verification_common;

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use common::Cpu as _;
use cpu::{ADDRESS_WIDTH_24, CPU_MODEL_386_SX, I386, I386State};
use verification_common::{MooCycle, load_moo_tests, load_revocation_list};

const RAM_SIZE: usize = 16 * 1024 * 1024;
const ADDRESS_MASK: u32 = 0x00FF_FFFF;
const REG_ORDER_386: [&str; 20] = [
    "cr0", "cr3", "eax", "ebx", "ecx", "edx", "esi", "edi", "ebp", "esp", "cs", "ds", "es", "fs",
    "gs", "ss", "eip", "eflags", "dr6", "dr7",
];

/// Average clocks the 386EX spends per test on the post-jump prefetch queue
/// fill and the SMM state capture triggered by the terminating `HALT` - the
/// portion of each trace that a plain 386SX would not spend on the instruction
/// itself. Subtracted from each trace length. This is an average of a
/// per-test-varying process (fill length depends on instruction length and
/// alignment), so a fractional value is expected; the suite README quotes ~14
/// as a rough figure and notes it "isn't perfect". Calibration constant.
const SMM_CAPTURE_OVERHEAD_CLOCKS: f64 = 12.32;

/// Aggregate agreement required between emulated and trace clock totals.
const AGGREGATE_TOLERANCE: f64 = 0.005;

/// Maximum instructions executed per test before giving up (matches the
/// architectural harness).
const MAX_STEPS: usize = 4096;

/// Opcode stems (after stripping 66/67 prefixes) whose 386EX timings were
/// lengthened by the SMM microcode relative to a plain 386SX. Excluded from
/// the aggregate. Covers IN/INS/OUT/OUTS/HLT and MOV CR0,src.
const SMM_AFFECTED_OPCODES: &[&str] = &[
    "6C", "6D", "6E", "6F", "E4", "E5", "E6", "E7", "EC", "ED", "EE", "EF", "F4", "0F22",
];

/// Bare opcode stems (after stripping 66/67 prefixes) for the high-frequency
/// instructions whose 386SX per-opcode timing we refine: MOV, the ALU ops,
/// MUL/IMUL, shifts/rotates, string ops, INC/DEC, PUSH/POP, TEST, and LEA.
/// Representative encodings are enough because timing-identical encodings share
/// a single call site. Used to print the per-opcode tuning table below.
const IMPORTANT_OPCODES: &[&str] = &[
    // MOV
    "88", "89", "8A", "8B", "A0", "A1", "A2", "A3", "B0", "B8", "C6",
    "C7", // ALU r/m,reg and reg,r/m, plus immediate groups
    "01", "03", "09", "29", "31", "39", "3B", "80", "81", "83", // MUL / IMUL
    "F6", "F7", "0FAF", "69", "6B", // shifts / rotates (group 2)
    "C0", "C1", "D0", "D1", "D2", "D3", // INC / DEC
    "40", "48", "FE", "FF", // PUSH / POP
    "50", "58", "68", "6A", "8F", // string
    "A4", "A5", "A6", "A7", "AA", "AB", "AC", "AD", "AE", "AF", // TEST
    "84", "85", "A8", "A9", // LEA
    "8D",
];

/// Per-opcode delta above which the important-opcode table flags a stem for
/// tuning.
const IMPORTANT_OPCODE_BOUND_PERCENT: f64 = 10.0;

/// Minimum instruction clocks per test for a stem's delta to be trustworthy.
/// Below this the fixed ~18-clock capture overhead dwarfs the instruction, so
/// the overhead-removal variance swamps the signal and the percentage is noise;
/// such short opcodes keep their datasheet flat timing and are not flagged.
const MIN_MEANINGFUL_CLOCKS_PER_TEST: f64 = 10.0;

type SxCpu = I386<{ CPU_MODEL_386_SX }, { ADDRESS_WIDTH_24 }>;

struct TestBus {
    ram: Vec<u8>,
    dirty: Vec<u32>,
    dirty_marker: Vec<u8>,
}

impl TestBus {
    fn new() -> Self {
        Self {
            ram: vec![0u8; RAM_SIZE],
            dirty: Vec::new(),
            dirty_marker: vec![0u8; RAM_SIZE],
        }
    }

    fn clear(&mut self) {
        for &address in &self.dirty {
            let index = (address & ADDRESS_MASK) as usize;
            self.ram[index] = 0;
            self.dirty_marker[index] = 0;
        }
        self.dirty.clear();
    }

    fn set_memory(&mut self, address: u32, value: u8) {
        let index = (address & ADDRESS_MASK) as usize;
        if self.dirty_marker[index] == 0 {
            self.dirty_marker[index] = 1;
            self.dirty.push(address & ADDRESS_MASK);
        }
        self.ram[index] = value;
    }
}

impl common::Bus for TestBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        self.ram[(address & ADDRESS_MASK) as usize]
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.set_memory(address, value);
    }

    fn io_read_byte(&mut self, port: u16) -> u8 {
        match port {
            0x22 => 0x7F,
            0x23 => 0x42,
            _ => 0xFF,
        }
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
        0
    }

    fn set_current_cycle(&mut self, _cycle: u64) {}
}

fn test_dir() -> &'static Path {
    static DIR: LazyLock<PathBuf> = LazyLock::new(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/SingleStepTests/80386/v1_ex_real_mode")
    });
    &DIR
}

fn revocation_list() -> &'static HashSet<String> {
    static REVOKED: LazyLock<HashSet<String>> = LazyLock::new(|| {
        load_revocation_list(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/SingleStepTests/80386/revocation_list.txt"),
        )
    });
    &REVOKED
}

/// Strips 66/67 operand and address size prefixes and any `.ext` ModR/M
/// suffix, returning the bare opcode stem in uppercase.
fn bare_opcode(stem: &str) -> String {
    let mut opcode = stem;
    loop {
        if let Some(rest) = opcode.strip_prefix("66") {
            opcode = rest;
        } else if let Some(rest) = opcode.strip_prefix("67") {
            opcode = rest;
        } else {
            break;
        }
    }
    let opcode = opcode.split('.').next().unwrap_or(opcode);
    opcode.to_ascii_uppercase()
}

fn is_smm_affected(stem: &str) -> bool {
    let opcode = bare_opcode(stem);
    SMM_AFFECTED_OPCODES.contains(&opcode.as_str())
}

fn initial_reg_value(initial_regs: &std::collections::HashMap<String, u32>, name: &str) -> u32 {
    initial_regs
        .get(name)
        .copied()
        .unwrap_or_else(|| panic!("missing register in initial state: {name}"))
}

fn build_initial_state(initial_regs: &std::collections::HashMap<String, u32>) -> I386State {
    let mut s = I386State {
        cr0: initial_reg_value(initial_regs, "cr0"),
        cr3: initial_reg_value(initial_regs, "cr3"),
        dr6: initial_reg_value(initial_regs, "dr6"),
        dr7: initial_reg_value(initial_regs, "dr7"),
        ..I386State::default()
    };
    s.set_eax(initial_reg_value(initial_regs, "eax"));
    s.set_ebx(initial_reg_value(initial_regs, "ebx"));
    s.set_ecx(initial_reg_value(initial_regs, "ecx"));
    s.set_edx(initial_reg_value(initial_regs, "edx"));
    s.set_esi(initial_reg_value(initial_regs, "esi"));
    s.set_edi(initial_reg_value(initial_regs, "edi"));
    s.set_ebp(initial_reg_value(initial_regs, "ebp"));
    s.set_esp(initial_reg_value(initial_regs, "esp"));
    s.set_cs(initial_reg_value(initial_regs, "cs") as u16);
    s.set_ds(initial_reg_value(initial_regs, "ds") as u16);
    s.set_es(initial_reg_value(initial_regs, "es") as u16);
    s.set_fs(initial_reg_value(initial_regs, "fs") as u16);
    s.set_gs(initial_reg_value(initial_regs, "gs") as u16);
    s.set_ss(initial_reg_value(initial_regs, "ss") as u16);
    s.set_eip(initial_reg_value(initial_regs, "eip"));
    s.set_eflags(initial_reg_value(initial_regs, "eflags"));
    s
}

#[derive(Default, Clone)]
struct StemStats {
    tests: u64,
    expected: f64,
    actual: u64,
    /// Raw trace clocks (all cycle records), summed. The per-opcode table
    /// derives instruction clocks from this by subtracting a self-calibrated
    /// overhead (the capture/prefetch-fill clocks that are not the instruction).
    trace_total: u64,
    /// Emulated instruction clocks (every step except the terminating HALT),
    /// summed. Compared against the overhead-adjusted trace total.
    instr_actual: u64,
}

/// Accumulates the expected (trace) and actual (emulated) clock totals for one
/// opcode stem into `stats`, skipping revoked and exception tests.
fn accumulate_stem(stem: &str, bus: &mut TestBus, stats: &mut StemStats) {
    let revoked = revocation_list();
    let filename = format!("{stem}.MOO.gz");
    let path = test_dir().join(&filename);
    let test_cases = load_moo_tests(&path, &[], &REG_ORDER_386);

    for test in &test_cases {
        if let Some(hash) = &test.hash
            && revoked.contains(&hash.to_ascii_lowercase())
        {
            continue;
        }
        if test.exception.is_some() {
            continue;
        }

        let trace_clocks = test
            .cycles
            .iter()
            .filter(|cycle| matches!(cycle, MooCycle::I386(_)))
            .count() as u64;
        if trace_clocks == 0 {
            continue;
        }
        let expected = (trace_clocks as f64 - SMM_CAPTURE_OVERHEAD_CLOCKS).max(0.0);

        bus.clear();
        for &(address, value) in &test.initial.ram {
            bus.set_memory(address, value);
        }

        let mut cpu: SxCpu = I386::new();
        cpu.load_state(&build_initial_state(&test.initial.regs));

        let mut actual = 0u64;
        let mut instr_actual = 0u64;
        let mut steps = 0usize;
        while !cpu.halted() && steps < MAX_STEPS {
            cpu.step(bus);
            let consumed = cpu.cycles_consumed();
            actual += consumed;
            // Exclude the step that halted (the injected HALT): its cost is the
            // emulator's HALT, not the instruction under test.
            if !cpu.halted() {
                instr_actual += consumed;
            }
            steps += 1;
        }
        if steps >= MAX_STEPS {
            continue;
        }

        stats.tests += 1;
        stats.expected += expected;
        stats.actual += actual;
        stats.trace_total += trace_clocks;
        stats.instr_actual += instr_actual;
    }
}

#[test]
fn sx_aggregate_timing_within_tolerance() {
    let mut entries: Vec<String> = fs::read_dir(test_dir())
        .expect("test directory present")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter_map(|name| name.strip_suffix(".MOO.gz").map(str::to_string))
        .collect();
    entries.sort();
    assert!(!entries.is_empty(), "no MOO test files found");

    let mut bus = TestBus::new();
    let mut per_stem: Vec<(String, StemStats)> = Vec::new();

    for stem in &entries {
        if is_smm_affected(stem) {
            continue;
        }
        let mut stats = StemStats::default();
        accumulate_stem(stem, &mut bus, &mut stats);
        if stats.tests > 0 {
            per_stem.push((stem.clone(), stats));
        }
    }

    let total_expected: f64 = per_stem.iter().map(|(_, stats)| stats.expected).sum();
    let total_actual: u64 = per_stem.iter().map(|(_, stats)| stats.actual).sum();
    let total_tests: u64 = per_stem.iter().map(|(_, stats)| stats.tests).sum();

    let mut ranked = per_stem.clone();
    ranked.sort_by(|a, b| {
        let delta_a = (a.1.actual as f64 - a.1.expected).abs();
        let delta_b = (b.1.actual as f64 - b.1.expected).abs();
        delta_b.total_cmp(&delta_a)
    });

    println!("SX timing calibration report (worst 40 stems by absolute delta):");
    println!(
        "{:<12} {:>7} {:>12} {:>12} {:>12} {:>8} {:>10}",
        "stem", "tests", "expected", "actual", "delta", "delta%", "avg/test"
    );
    for (stem, stats) in ranked.iter().take(40) {
        let delta = stats.actual as f64 - stats.expected;
        let delta_pct = if stats.expected > 0.0 {
            delta / stats.expected * 100.0
        } else {
            0.0
        };
        let avg = delta / stats.tests as f64;
        println!(
            "{:<12} {:>7} {:>12.0} {:>12} {:>12.0} {:>7.2}% {:>10.3}",
            stem, stats.tests, stats.expected, stats.actual, delta, delta_pct, avg
        );
    }

    // Isolate instruction cost from the fixed per-test capture overhead. The
    // aggregate ties emulated clocks (including the emulator's HALT) to
    // trace_total - SMM_CAPTURE_OVERHEAD_CLOCKS, so the overhead that maps a
    // trace total onto pure instruction clocks is that constant plus the mean
    // emulated HALT-step cost. Self-calibrated from this run so the important
    // table centers at zero, matching the aggregate.
    let total_instr_actual: u64 = per_stem.iter().map(|(_, s)| s.instr_actual).sum();
    let mean_halt = (total_actual - total_instr_actual) as f64 / total_tests as f64;
    let overhead_instr = SMM_CAPTURE_OVERHEAD_CLOCKS + mean_halt;
    let expected_instr =
        |stats: &StemStats| stats.trace_total as f64 - stats.tests as f64 * overhead_instr;

    let mut important: Vec<&(String, StemStats)> = per_stem
        .iter()
        .filter(|(stem, _)| IMPORTANT_OPCODES.contains(&bare_opcode(stem).as_str()))
        .collect();
    // A stem carries a trustworthy signal only when its instruction cost clears
    // the overhead-variance floor; sort those to the top by relative error, and
    // rank the rest (overhead-noise-limited) below by absolute per-test error.
    let meaningful = |s: &StemStats| {
        s.trace_total as f64 / s.tests as f64 - overhead_instr >= MIN_MEANINGFUL_CLOCKS_PER_TEST
    };
    important.sort_by(|a, b| {
        let key = |s: &StemStats| {
            let pct =
                (s.instr_actual as f64 - expected_instr(s)) / expected_instr(s).max(1.0) * 100.0;
            (meaningful(s), pct.abs())
        };
        key(&b.1).partial_cmp(&key(&a.1)).unwrap()
    });
    println!(
        "\nImportant opcodes (instruction clocks, overhead {overhead_instr:.1}/test removed; \
         '!' = >{IMPORTANT_OPCODE_BOUND_PERCENT:.0}% and >={MIN_MEANINGFUL_CLOCKS_PER_TEST:.0} clocks/test):"
    );
    println!(
        "{:<12} {:>7} {:>12} {:>12} {:>12} {:>8} {:>10}",
        "stem", "tests", "expected", "actual", "delta", "delta%", "avg/test"
    );
    for (stem, stats) in important {
        let expected = expected_instr(stats);
        let delta = stats.instr_actual as f64 - expected;
        let delta_pct = if expected > 0.0 {
            delta / expected * 100.0
        } else {
            0.0
        };
        let avg = delta / stats.tests as f64;
        // Flag only stems whose instruction cost clears the overhead-variance
        // floor and whose relative error still exceeds the bound.
        let flag = if meaningful(stats) && delta_pct.abs() > IMPORTANT_OPCODE_BOUND_PERCENT {
            " !"
        } else {
            ""
        };
        println!(
            "{:<12} {:>7} {:>12.0} {:>12} {:>12.0} {:>7.2}% {:>10.3}{}",
            stem, stats.tests, expected, stats.instr_actual, delta, delta_pct, avg, flag
        );
    }

    let total_delta = total_actual as f64 - total_expected;
    let total_pct = total_delta / total_expected * 100.0;
    println!(
        "TOTAL: {total_tests} tests, expected {total_expected:.0}, actual {total_actual}, \
         delta {total_delta:.0} ({total_pct:.4}%)"
    );

    let relative = (total_actual as f64 - total_expected).abs() / total_expected;
    assert!(
        relative <= AGGREGATE_TOLERANCE,
        "SX aggregate timing off by {:.4}% (tolerance {:.4}%): expected {total_expected:.0}, got {total_actual}. \
         Tune SMM_CAPTURE_OVERHEAD_CLOCKS and the SX_EXTRA_CLOCKS_* constants in i386.rs.",
        relative * 100.0,
        AGGREGATE_TOLERANCE * 100.0,
    );
}
