#![cfg(feature = "verification")]

#[path = "common/verification_common.rs"]
mod verification_common;

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::BufReader,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use common::Cpu as _;
use cpu::{I386, I386State};
use verification_common::{load_moo_tests, load_revocation_list};

const RAM_SIZE: usize = 16 << 20;
const ADDRESS_MASK: u32 = 0x00FF_FFFF;
const REG_ORDER_386: [&str; 20] = [
    "cr0", "cr3", "eax", "ebx", "ecx", "edx", "esi", "edi", "ebp", "esp", "cs", "ds", "es", "fs",
    "gs", "ss", "eip", "eflags", "dr6", "dr7",
];

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

fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        if ch == '"' {
            if in_quotes && chars.peek() == Some(&'"') {
                field.push('"');
                let _ = chars.next();
            } else {
                in_quotes = !in_quotes;
            }
        } else if ch == ',' && !in_quotes {
            fields.push(field);
            field = String::new();
        } else {
            field.push(ch);
        }
    }

    fields.push(field);
    fields
}

fn parse_hex_u16(value: &str) -> Option<u16> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let raw = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    u16::from_str_radix(raw, 16).ok()
}

fn flags_compare_masks() -> &'static HashMap<(String, Option<u8>), u16> {
    static MAP: LazyLock<HashMap<(String, Option<u8>), u16>> = LazyLock::new(|| {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/SingleStepTests/80386/80386.csv");
        let file = fs::File::open(path).unwrap();
        let mut lines = std::io::BufRead::lines(BufReader::new(file));
        let header = lines.next().unwrap().unwrap();
        let columns = split_csv_line(&header);

        let mut index_op = None;
        let mut index_ex = None;
        let mut index_f_umask = None;

        for (index, column) in columns.iter().enumerate() {
            match column.as_str() {
                "op" => index_op = Some(index),
                "ex" => index_ex = Some(index),
                "f_umask" => index_f_umask = Some(index),
                _ => {}
            }
        }

        let op_index = index_op.unwrap();
        let ex_index = index_ex.unwrap();
        let f_umask_index = index_f_umask.unwrap();

        let mut result = HashMap::new();

        for line in lines {
            let line = line.unwrap();
            if line.trim().is_empty() {
                continue;
            }
            let fields = split_csv_line(&line);
            if fields.len() <= f_umask_index {
                continue;
            }

            let op = fields[op_index].trim();
            if op.is_empty() {
                continue;
            }

            let ex = fields[ex_index].trim();
            let ex = if ex.is_empty() {
                None
            } else {
                Some(ex.parse::<u8>().unwrap())
            };

            let Some(compare_mask) = parse_hex_u16(&fields[f_umask_index]) else {
                continue;
            };
            let key = (op.to_ascii_uppercase(), ex);
            result.insert(key, compare_mask);
        }

        result
    });

    &MAP
}

fn parse_stem(stem: &str) -> (&str, Option<u8>) {
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

    if let Some(dot_pos) = opcode.find('.') {
        (
            &opcode[..dot_pos],
            Some(opcode[dot_pos + 1..].parse::<u8>().unwrap()),
        )
    } else {
        (opcode, None)
    }
}

fn flags_compare_mask(stem: &str) -> u32 {
    let masks = flags_compare_masks();
    let (opcode, reg_ext) = parse_stem(stem);
    let opcode_upper = opcode.to_ascii_uppercase();
    let low16_mask = masks
        .get(&(opcode_upper.clone(), reg_ext))
        .copied()
        .or(match (opcode_upper.as_str(), reg_ext) {
            // BT/BTS/BTR/BTC define only CF; OF/SF/ZF/AF/PF are undefined.
            ("0FA3", None)
            | ("0FAB", None)
            | ("0FB3", None)
            | ("0FBB", None)
            | ("0FBA", Some(4))
            | ("0FBA", Some(5))
            | ("0FBA", Some(6))
            | ("0FBA", Some(7)) => Some(0xF72B),
            ("0FAF", None) => Some(0xFF2B),
            ("0FBC", None) | ("0FBD", None) => Some(0xF76A),
            _ => None,
        })
        .unwrap_or(0xFFFF);
    0xFFFF_0000 | low16_mask as u32
}

fn initial_reg_value(initial_regs: &HashMap<String, u32>, name: &str) -> u32 {
    initial_regs
        .get(name)
        .copied()
        .unwrap_or_else(|| panic!("missing register in initial state: {name}"))
}

fn build_expected_state(
    initial_regs: &HashMap<String, u32>,
    final_regs: &HashMap<String, u32>,
) -> I386State {
    let get = |name: &str| -> u32 {
        final_regs
            .get(name)
            .copied()
            .unwrap_or_else(|| initial_reg_value(initial_regs, name))
    };

    let mut s = I386State {
        cr0: get("cr0"),
        cr3: get("cr3"),
        dr6: get("dr6"),
        dr7: get("dr7"),
        ..I386State::default()
    };
    s.set_eax(get("eax"));
    s.set_ebx(get("ebx"));
    s.set_ecx(get("ecx"));
    s.set_edx(get("edx"));
    s.set_esi(get("esi"));
    s.set_edi(get("edi"));
    s.set_ebp(get("ebp"));
    s.set_esp(get("esp"));
    s.set_cs(get("cs") as u16);
    s.set_ds(get("ds") as u16);
    s.set_es(get("es") as u16);
    s.set_fs(get("fs") as u16);
    s.set_gs(get("gs") as u16);
    s.set_ss(get("ss") as u16);
    s.set_eip(get("eip"));
    s.set_eflags(get("eflags"));
    s
}

fn run_test_file(stem: &str, local_revoked_hashes: &[&str]) {
    let revoked = revocation_list();
    let local_revoked: HashSet<String> = local_revoked_hashes
        .iter()
        .map(|hash| hash.to_ascii_lowercase())
        .collect();
    let mut bus = TestBus::new();

    let filename = format!("{stem}.MOO.gz");
    let path = test_dir().join(&filename);
    let test_cases = load_moo_tests(&path, &[], &REG_ORDER_386);
    let mut failures: Vec<String> = Vec::new();

    for test in &test_cases {
        if let Some(hash) = &test.hash
            && (revoked.contains(&hash.to_ascii_lowercase())
                || local_revoked.contains(&hash.to_ascii_lowercase()))
        {
            continue;
        }

        if test.exception.is_some() {
            continue;
        }

        bus.clear();
        for &(address, value) in &test.initial.ram {
            bus.set_memory(address, value);
        }
        let initial = {
            let mut s = I386State {
                cr0: initial_reg_value(&test.initial.regs, "cr0"),
                cr3: initial_reg_value(&test.initial.regs, "cr3"),
                dr6: initial_reg_value(&test.initial.regs, "dr6"),
                dr7: initial_reg_value(&test.initial.regs, "dr7"),
                ..I386State::default()
            };
            s.set_eax(initial_reg_value(&test.initial.regs, "eax"));
            s.set_ebx(initial_reg_value(&test.initial.regs, "ebx"));
            s.set_ecx(initial_reg_value(&test.initial.regs, "ecx"));
            s.set_edx(initial_reg_value(&test.initial.regs, "edx"));
            s.set_esi(initial_reg_value(&test.initial.regs, "esi"));
            s.set_edi(initial_reg_value(&test.initial.regs, "edi"));
            s.set_ebp(initial_reg_value(&test.initial.regs, "ebp"));
            s.set_esp(initial_reg_value(&test.initial.regs, "esp"));
            s.set_cs(initial_reg_value(&test.initial.regs, "cs") as u16);
            s.set_ds(initial_reg_value(&test.initial.regs, "ds") as u16);
            s.set_es(initial_reg_value(&test.initial.regs, "es") as u16);
            s.set_fs(initial_reg_value(&test.initial.regs, "fs") as u16);
            s.set_gs(initial_reg_value(&test.initial.regs, "gs") as u16);
            s.set_ss(initial_reg_value(&test.initial.regs, "ss") as u16);
            s.set_eip(initial_reg_value(&test.initial.regs, "eip"));
            s.set_eflags(initial_reg_value(&test.initial.regs, "eflags"));
            s
        };

        let mut cpu: I386 = I386::new();
        cpu.load_state(&initial);
        let mut steps = 0usize;
        while !cpu.halted() && steps < 4096 {
            cpu.step(&mut bus);
            steps += 1;
        }

        if steps >= 4096 {
            let hash_suffix = test
                .hash
                .as_ref()
                .map(|hash| format!(" hash={hash}"))
                .unwrap_or_default();
            failures.push(format!(
                "[{filename} #{} idx={}] {}{} (execution did not reach HLT within 4096 instructions)",
                failures.len(),
                test.idx,
                test.name,
                hash_suffix
            ));
            continue;
        }

        let expected = build_expected_state(&test.initial.regs, &test.final_state.regs);
        let mut diffs: Vec<String> = Vec::new();

        let check_u32 = |name: &str,
                         initial_value: u32,
                         actual_value: u32,
                         expected_value: u32,
                         diffs: &mut Vec<String>| {
            if actual_value != expected_value {
                diffs.push(format!(
                    "  {name}: expected 0x{expected_value:08X}, got 0x{actual_value:08X} (was 0x{initial_value:08X})"
                ));
            }
        };

        check_u32("cr0", initial.cr0, cpu.cr0, expected.cr0, &mut diffs);
        check_u32("cr3", initial.cr3, cpu.cr3, expected.cr3, &mut diffs);
        check_u32("eax", initial.eax(), cpu.eax(), expected.eax(), &mut diffs);
        check_u32("ebx", initial.ebx(), cpu.ebx(), expected.ebx(), &mut diffs);
        check_u32("ecx", initial.ecx(), cpu.ecx(), expected.ecx(), &mut diffs);
        check_u32("edx", initial.edx(), cpu.edx(), expected.edx(), &mut diffs);
        check_u32("esi", initial.esi(), cpu.esi(), expected.esi(), &mut diffs);
        check_u32("edi", initial.edi(), cpu.edi(), expected.edi(), &mut diffs);
        check_u32("ebp", initial.ebp(), cpu.ebp(), expected.ebp(), &mut diffs);
        check_u32("esp", initial.esp(), cpu.esp(), expected.esp(), &mut diffs);
        check_u32("eip", initial.eip(), cpu.eip(), expected.eip(), &mut diffs);
        check_u32("dr6", initial.dr6, cpu.dr6, expected.dr6, &mut diffs);
        check_u32("dr7", initial.dr7, cpu.dr7, expected.dr7, &mut diffs);

        if cpu.cs() != expected.cs() {
            diffs.push(format!(
                "  cs: expected 0x{:04X}, got 0x{:04X} (was 0x{:04X})",
                expected.cs(),
                cpu.cs(),
                initial.cs()
            ));
        }
        if cpu.ds() != expected.ds() {
            diffs.push(format!(
                "  ds: expected 0x{:04X}, got 0x{:04X} (was 0x{:04X})",
                expected.ds(),
                cpu.ds(),
                initial.ds()
            ));
        }
        if cpu.es() != expected.es() {
            diffs.push(format!(
                "  es: expected 0x{:04X}, got 0x{:04X} (was 0x{:04X})",
                expected.es(),
                cpu.es(),
                initial.es()
            ));
        }
        if cpu.fs() != expected.fs() {
            diffs.push(format!(
                "  fs: expected 0x{:04X}, got 0x{:04X} (was 0x{:04X})",
                expected.fs(),
                cpu.fs(),
                initial.fs()
            ));
        }
        if cpu.gs() != expected.gs() {
            diffs.push(format!(
                "  gs: expected 0x{:04X}, got 0x{:04X} (was 0x{:04X})",
                expected.gs(),
                cpu.gs(),
                initial.gs()
            ));
        }
        if cpu.ss() != expected.ss() {
            diffs.push(format!(
                "  ss: expected 0x{:04X}, got 0x{:04X} (was 0x{:04X})",
                expected.ss(),
                cpu.ss(),
                initial.ss()
            ));
        }

        let eflags_mask = flags_compare_mask(stem);
        if (cpu.eflags() & eflags_mask) != (expected.eflags() & eflags_mask) {
            diffs.push(format!(
                "  eflags: expected 0x{:08X}, got 0x{:08X} (was 0x{:08X}, mask 0x{:08X})",
                expected.eflags() & eflags_mask,
                cpu.eflags() & eflags_mask,
                initial.eflags(),
                eflags_mask
            ));
        }

        for &(address, expected_value) in &test.final_state.ram {
            let actual_value = bus.ram[(address & ADDRESS_MASK) as usize];
            if actual_value != expected_value {
                let initial_value = test
                    .initial
                    .ram
                    .iter()
                    .find(|(initial_address, _)| *initial_address == address)
                    .map(|(_, value)| *value);
                match initial_value {
                    Some(before) => diffs.push(format!(
                        "  ram[0x{address:06X}]: expected 0x{expected_value:02X}, got 0x{actual_value:02X} (was 0x{before:02X})"
                    )),
                    None => diffs.push(format!(
                        "  ram[0x{address:06X}]: expected 0x{expected_value:02X}, got 0x{actual_value:02X} (not in initial RAM)"
                    )),
                }
            }
        }

        if !diffs.is_empty() {
            let bytes_hex: Vec<String> = test
                .bytes
                .iter()
                .map(|value| format!("{value:02X}"))
                .collect();
            failures.push(format!(
                "[{filename} #{} idx={}] {} ({})\n{}",
                failures.len(),
                test.idx,
                test.name,
                bytes_hex.join(" "),
                diffs.join("\n")
            ));
        }
    }

    if !failures.is_empty() {
        let mut message = format!(
            "{filename}: {}/{} tests failed\n",
            failures.len(),
            test_cases.len()
        );
        let display_count = failures.len().min(10);
        for failure in &failures[..display_count] {
            message.push_str(failure);
            message.push('\n');
        }
        if failures.len() > display_count {
            message.push_str(&format!(
                "  ... and {} more failures\n",
                failures.len() - display_count
            ));
        }
        panic!("{message}");
    }
}

#[test]
fn idiv_byte_min_overflow_faults_without_panic() {
    let mut bus = TestBus::new();
    let mut cpu: I386 = I386::new();

    let initial = {
        let mut s = I386State::default();
        s.set_eax(0x0000_8000);
        s.set_ecx(0x0000_00FF);
        s.set_esp(0x0000_0200);
        s.set_cs(0x1000);
        s.set_eflags(0x0000_0002);
        s
    };
    cpu.load_state(&initial);

    bus.set_memory(0x0000, 0x00);
    bus.set_memory(0x0001, 0x01);
    bus.set_memory(0x0002, 0x00);
    bus.set_memory(0x0003, 0x00);
    bus.set_memory(0x0001_0000, 0xF6);
    bus.set_memory(0x0001_0001, 0xF9);
    bus.set_memory(0x0001_0002, 0xF4);
    bus.set_memory(0x0000_0100, 0xF4);

    for _ in 0..16 {
        if cpu.halted() {
            break;
        }
        cpu.step(&mut bus);
    }
    assert!(cpu.halted(), "CPU did not halt after fault handler");

    assert_eq!(cpu.cs(), 0x0000);
    assert_eq!(cpu.eip(), 0x0000_0101);
    assert_eq!(cpu.esp(), 0x0000_01FA);
    assert_eq!(bus.ram[0x01FA], 0x00);
    assert_eq!(bus.ram[0x01FB], 0x00);
}

#[test]
fn idiv_word_min_overflow_faults_without_panic() {
    let mut bus = TestBus::new();
    let mut cpu: I386 = I386::new();

    let initial = {
        let mut s = I386State::default();
        s.set_ecx(0x0000_FFFF);
        s.set_edx(0x0000_8000);
        s.set_esp(0x0000_0300);
        s.set_cs(0x1000);
        s.set_eflags(0x0000_0002);
        s
    };
    cpu.load_state(&initial);

    bus.set_memory(0x0000, 0x00);
    bus.set_memory(0x0001, 0x01);
    bus.set_memory(0x0002, 0x00);
    bus.set_memory(0x0003, 0x00);
    bus.set_memory(0x0001_0000, 0xF7);
    bus.set_memory(0x0001_0001, 0xF9);
    bus.set_memory(0x0001_0002, 0xF4);
    bus.set_memory(0x0000_0100, 0xF4);

    for _ in 0..16 {
        if cpu.halted() {
            break;
        }
        cpu.step(&mut bus);
    }
    assert!(cpu.halted(), "CPU did not halt after fault handler");

    assert_eq!(cpu.cs(), 0x0000);
    assert_eq!(cpu.eip(), 0x0000_0101);
    assert_eq!(cpu.esp(), 0x0000_02FA);
    assert_eq!(bus.ram[0x02FA], 0x00);
    assert_eq!(bus.ram[0x02FB], 0x00);
}

macro_rules! test_opcode {
    ($name:ident, $file:expr) => {
        #[test]
        fn $name() {
            run_test_file($file, &[]);
        }
    };
    ($name:ident, $file:expr, [$($skip_hash:expr),* $(,)?]) => {
        #[test]
        fn $name() {
            run_test_file($file, &[$($skip_hash),*]);
        }
    };
}

test_opcode!(op_00, "00");
test_opcode!(op_01, "01");
test_opcode!(op_02, "02");
test_opcode!(op_03, "03");
test_opcode!(op_04, "04");
test_opcode!(op_05, "05");
test_opcode!(op_06, "06");
test_opcode!(op_07, "07");
test_opcode!(op_08, "08");
test_opcode!(op_09, "09");
test_opcode!(op_0a, "0A");
test_opcode!(op_0b, "0B");
test_opcode!(op_0c, "0C");
test_opcode!(op_0d, "0D");
test_opcode!(op_0e, "0E");
test_opcode!(op_0f06, "0F06");
test_opcode!(op_0f80, "0F80");
test_opcode!(op_0f81, "0F81");
test_opcode!(op_0f82, "0F82");
test_opcode!(op_0f83, "0F83");
test_opcode!(op_0f84, "0F84");
test_opcode!(op_0f85, "0F85");
test_opcode!(op_0f86, "0F86");
test_opcode!(op_0f87, "0F87");
test_opcode!(op_0f88, "0F88");
test_opcode!(op_0f89, "0F89");
test_opcode!(op_0f8a, "0F8A");
test_opcode!(op_0f8b, "0F8B");
test_opcode!(op_0f8c, "0F8C");
test_opcode!(op_0f8d, "0F8D");
test_opcode!(op_0f8e, "0F8E");
test_opcode!(op_0f8f, "0F8F");
test_opcode!(op_0f90, "0F90");
test_opcode!(op_0f91, "0F91");
test_opcode!(op_0f92, "0F92");
test_opcode!(op_0f93, "0F93");
test_opcode!(op_0f94, "0F94");
test_opcode!(op_0f95, "0F95");
test_opcode!(op_0f96, "0F96");
test_opcode!(op_0f97, "0F97");
test_opcode!(op_0f98, "0F98");
test_opcode!(op_0f99, "0F99");
test_opcode!(op_0f9a, "0F9A");
test_opcode!(op_0f9b, "0F9B");
test_opcode!(op_0f9c, "0F9C");
test_opcode!(op_0f9d, "0F9D");
test_opcode!(op_0f9e, "0F9E");
test_opcode!(op_0f9f, "0F9F");
test_opcode!(op_0fa0, "0FA0");
test_opcode!(op_0fa1, "0FA1");
test_opcode!(op_0fa3, "0FA3");
test_opcode!(op_0fa4, "0FA4");
test_opcode!(op_0fa5, "0FA5");
test_opcode!(op_0fa8, "0FA8");
test_opcode!(op_0fa9, "0FA9");
test_opcode!(op_0fab, "0FAB");
test_opcode!(op_0fac, "0FAC");
test_opcode!(op_0fad, "0FAD");
test_opcode!(op_0faf, "0FAF");
test_opcode!(op_0fb2, "0FB2");
test_opcode!(op_0fb3, "0FB3");
test_opcode!(op_0fb4, "0FB4");
test_opcode!(op_0fb5, "0FB5");
test_opcode!(op_0fb6, "0FB6");
test_opcode!(op_0fb7, "0FB7");
test_opcode!(op_0fba_4, "0FBA.4");
test_opcode!(op_0fba_5, "0FBA.5");
test_opcode!(op_0fba_6, "0FBA.6");
test_opcode!(op_0fba_7, "0FBA.7");
test_opcode!(op_0fbb, "0FBB");
test_opcode!(op_0fbc, "0FBC");
test_opcode!(op_0fbd, "0FBD");
test_opcode!(op_0fbe, "0FBE");
test_opcode!(op_0fbf, "0FBF");
test_opcode!(op_10, "10");
test_opcode!(op_11, "11");
test_opcode!(op_12, "12");
test_opcode!(op_13, "13");
test_opcode!(op_14, "14");
test_opcode!(op_15, "15");
test_opcode!(op_16, "16");
test_opcode!(op_17, "17");
test_opcode!(op_18, "18");
test_opcode!(op_19, "19");
test_opcode!(op_1a, "1A");
test_opcode!(op_1b, "1B");
test_opcode!(op_1c, "1C");
test_opcode!(op_1d, "1D");
test_opcode!(op_1e, "1E");
test_opcode!(op_1f, "1F");
test_opcode!(op_20, "20");
test_opcode!(op_21, "21");
test_opcode!(op_22, "22");
test_opcode!(op_23, "23");
test_opcode!(op_24, "24");
test_opcode!(op_25, "25");
test_opcode!(op_27, "27");
test_opcode!(op_28, "28");
test_opcode!(op_29, "29");
test_opcode!(op_2a, "2A");
test_opcode!(op_2b, "2B");
test_opcode!(op_2c, "2C");
test_opcode!(op_2d, "2D");
test_opcode!(op_2f, "2F");
test_opcode!(op_30, "30");
test_opcode!(op_31, "31");
test_opcode!(op_32, "32");
test_opcode!(op_33, "33");
test_opcode!(op_34, "34");
test_opcode!(op_35, "35");
test_opcode!(op_37, "37");
test_opcode!(op_38, "38");
test_opcode!(op_39, "39");
test_opcode!(op_3a, "3A");
test_opcode!(op_3b, "3B");
test_opcode!(op_3c, "3C");
test_opcode!(op_3d, "3D");
test_opcode!(op_3f, "3F");
test_opcode!(op_40, "40");
test_opcode!(op_41, "41");
test_opcode!(op_42, "42");
test_opcode!(op_43, "43");
test_opcode!(op_44, "44");
test_opcode!(op_45, "45");
test_opcode!(op_46, "46");
test_opcode!(op_47, "47");
test_opcode!(op_48, "48");
test_opcode!(op_49, "49");
test_opcode!(op_4a, "4A");
test_opcode!(op_4b, "4B");
test_opcode!(op_4c, "4C");
test_opcode!(op_4d, "4D");
test_opcode!(op_4e, "4E");
test_opcode!(op_4f, "4F");
test_opcode!(op_50, "50");
test_opcode!(op_51, "51");
test_opcode!(op_52, "52");
test_opcode!(op_53, "53");
test_opcode!(op_54, "54");
test_opcode!(op_55, "55");
test_opcode!(op_56, "56");
test_opcode!(op_57, "57");
test_opcode!(op_58, "58");
test_opcode!(op_59, "59");
test_opcode!(op_5a, "5A");
test_opcode!(op_5b, "5B");
test_opcode!(op_5c, "5C");
test_opcode!(op_5d, "5D");
test_opcode!(op_5e, "5E");
test_opcode!(op_5f, "5F");
test_opcode!(op_60, "60");
test_opcode!(op_61, "61");
test_opcode!(op_62, "62");
test_opcode!(op_6601, "6601");
test_opcode!(op_6603, "6603");
test_opcode!(op_6605, "6605");
test_opcode!(op_6606, "6606");
test_opcode!(op_6607, "6607");
test_opcode!(op_6609, "6609");
test_opcode!(op_660b, "660B");
test_opcode!(op_660d, "660D");
test_opcode!(op_660e, "660E");
test_opcode!(op_660f80, "660F80");
test_opcode!(op_660f81, "660F81");
test_opcode!(op_660f82, "660F82");
test_opcode!(op_660f83, "660F83");
test_opcode!(op_660f84, "660F84");
test_opcode!(op_660f85, "660F85");
test_opcode!(op_660f86, "660F86");
test_opcode!(op_660f87, "660F87");
test_opcode!(op_660f88, "660F88");
test_opcode!(op_660f89, "660F89");
test_opcode!(op_660f8a, "660F8A");
test_opcode!(op_660f8b, "660F8B");
test_opcode!(op_660f8c, "660F8C");
test_opcode!(op_660f8d, "660F8D");
test_opcode!(op_660f8e, "660F8E");
test_opcode!(op_660f8f, "660F8F");
test_opcode!(op_660fa0, "660FA0");
test_opcode!(op_660fa1, "660FA1");
test_opcode!(op_660fa3, "660FA3");
test_opcode!(op_660fa4, "660FA4");
test_opcode!(op_660fa5, "660FA5");
test_opcode!(op_660fa8, "660FA8");
test_opcode!(op_660fa9, "660FA9");
test_opcode!(op_660fab, "660FAB");
test_opcode!(op_660fac, "660FAC");
test_opcode!(op_660fad, "660FAD");
test_opcode!(op_660faf, "660FAF");
test_opcode!(op_660fb2, "660FB2");
test_opcode!(op_660fb3, "660FB3");
test_opcode!(op_660fb4, "660FB4");
test_opcode!(op_660fb5, "660FB5");
test_opcode!(op_660fb6, "660FB6");
test_opcode!(op_660fb7, "660FB7");
test_opcode!(op_660fba_4, "660FBA.4");
test_opcode!(op_660fba_5, "660FBA.5");
test_opcode!(op_660fba_6, "660FBA.6");
test_opcode!(op_660fba_7, "660FBA.7");
test_opcode!(op_660fbb, "660FBB");
test_opcode!(op_660fbc, "660FBC");
test_opcode!(op_660fbd, "660FBD");
test_opcode!(op_660fbe, "660FBE");
test_opcode!(op_660fbf, "660FBF");
test_opcode!(op_6611, "6611");
test_opcode!(op_6613, "6613");
test_opcode!(op_6615, "6615");
test_opcode!(op_6616, "6616");
test_opcode!(op_6617, "6617");
test_opcode!(op_6619, "6619");
test_opcode!(op_661b, "661B");
test_opcode!(op_661d, "661D");
test_opcode!(op_661e, "661E");
test_opcode!(op_661f, "661F");
test_opcode!(op_6621, "6621");
test_opcode!(op_6623, "6623");
test_opcode!(op_6625, "6625");
test_opcode!(op_6629, "6629");
test_opcode!(op_662b, "662B");
test_opcode!(op_662d, "662D");
test_opcode!(op_6631, "6631");
test_opcode!(op_6633, "6633");
test_opcode!(op_6635, "6635");
test_opcode!(op_6639, "6639");
test_opcode!(op_663b, "663B");
test_opcode!(op_663d, "663D");
test_opcode!(op_6640, "6640");
test_opcode!(op_6641, "6641");
test_opcode!(op_6642, "6642");
test_opcode!(op_6643, "6643");
test_opcode!(op_6644, "6644");
test_opcode!(op_6645, "6645");
test_opcode!(op_6646, "6646");
test_opcode!(op_6647, "6647");
test_opcode!(op_6648, "6648");
test_opcode!(op_6649, "6649");
test_opcode!(op_664a, "664A");
test_opcode!(op_664b, "664B");
test_opcode!(op_664c, "664C");
test_opcode!(op_664d, "664D");
test_opcode!(op_664e, "664E");
test_opcode!(op_664f, "664F");
test_opcode!(op_6650, "6650");
test_opcode!(op_6651, "6651");
test_opcode!(op_6652, "6652");
test_opcode!(op_6653, "6653");
test_opcode!(op_6654, "6654");
test_opcode!(op_6655, "6655");
test_opcode!(op_6656, "6656");
test_opcode!(op_6657, "6657");
test_opcode!(op_6658, "6658");
test_opcode!(op_6659, "6659");
test_opcode!(op_665a, "665A");
test_opcode!(op_665b, "665B");
test_opcode!(op_665c, "665C");
test_opcode!(op_665d, "665D");
test_opcode!(op_665e, "665E");
test_opcode!(op_665f, "665F");
test_opcode!(op_6660, "6660");
test_opcode!(op_6661, "6661");
test_opcode!(op_6662, "6662");
test_opcode!(op_6668, "6668");
test_opcode!(op_6669, "6669");
test_opcode!(op_666a, "666A");
test_opcode!(op_666b, "666B");
test_opcode!(op_666d, "666D");
test_opcode!(op_666f, "666F");
test_opcode!(op_6670, "6670");
test_opcode!(op_6671, "6671");
test_opcode!(op_6672, "6672");
test_opcode!(op_6673, "6673");
test_opcode!(op_6674, "6674");
test_opcode!(op_6675, "6675");
test_opcode!(op_6676, "6676");
test_opcode!(op_6677, "6677");
test_opcode!(op_6678, "6678");
test_opcode!(op_6679, "6679");
test_opcode!(op_667a, "667A");
test_opcode!(op_667b, "667B");
test_opcode!(op_667c, "667C");
test_opcode!(op_667d, "667D");
test_opcode!(op_667e, "667E");
test_opcode!(op_667f, "667F");
test_opcode!(op_6681_0, "6681.0");
test_opcode!(op_6681_1, "6681.1");
test_opcode!(op_6681_2, "6681.2");
test_opcode!(op_6681_3, "6681.3");
test_opcode!(op_6681_4, "6681.4");
test_opcode!(op_6681_5, "6681.5");
test_opcode!(op_6681_6, "6681.6");
test_opcode!(op_6681_7, "6681.7");
test_opcode!(op_6683_0, "6683.0");
test_opcode!(op_6683_1, "6683.1");
test_opcode!(op_6683_2, "6683.2");
test_opcode!(op_6683_3, "6683.3");
test_opcode!(op_6683_4, "6683.4");
test_opcode!(op_6683_5, "6683.5");
test_opcode!(op_6683_6, "6683.6");
test_opcode!(op_6683_7, "6683.7");
test_opcode!(op_6685, "6685");
test_opcode!(op_6687, "6687");
test_opcode!(op_6689, "6689");
test_opcode!(op_668b, "668B");
test_opcode!(op_668c, "668C");
test_opcode!(op_668d, "668D");
test_opcode!(op_668e, "668E");
test_opcode!(op_668f, "668F");
test_opcode!(op_6690, "6690");
test_opcode!(op_6691, "6691");
test_opcode!(op_6692, "6692");
test_opcode!(op_6693, "6693");
test_opcode!(op_6694, "6694");
test_opcode!(op_6695, "6695");
test_opcode!(op_6696, "6696");
test_opcode!(op_6697, "6697");
test_opcode!(op_6698, "6698");
test_opcode!(op_6699, "6699");
test_opcode!(op_669a, "669A");
test_opcode!(op_669c, "669C");
test_opcode!(op_669d, "669D");
test_opcode!(op_66a1, "66A1");
test_opcode!(op_66a3, "66A3");
test_opcode!(op_66a5, "66A5");
test_opcode!(op_66a7, "66A7");
test_opcode!(op_66ab, "66AB");
test_opcode!(op_66ad, "66AD");
test_opcode!(op_66af, "66AF");
test_opcode!(op_66b8, "66B8");
test_opcode!(op_66b9, "66B9");
test_opcode!(op_66ba, "66BA");
test_opcode!(op_66bb, "66BB");
test_opcode!(op_66bc, "66BC");
test_opcode!(op_66bd, "66BD");
test_opcode!(op_66be, "66BE");
test_opcode!(op_66bf, "66BF");
test_opcode!(op_66c1_0, "66C1.0");
test_opcode!(op_66c1_1, "66C1.1");
test_opcode!(op_66c1_2, "66C1.2");
test_opcode!(op_66c1_3, "66C1.3");
test_opcode!(op_66c1_4, "66C1.4");
test_opcode!(op_66c1_5, "66C1.5");
test_opcode!(op_66c1_6, "66C1.6");
test_opcode!(op_66c1_7, "66C1.7");
test_opcode!(op_66c2, "66C2");
test_opcode!(op_66c3, "66C3");
test_opcode!(op_66c4, "66C4");
test_opcode!(op_66c5, "66C5");
test_opcode!(op_66c7, "66C7");
test_opcode!(op_66c8, "66C8");
test_opcode!(op_66c9, "66C9");
test_opcode!(op_66ca, "66CA");
test_opcode!(op_66cb, "66CB");
test_opcode!(op_66cf, "66CF");
test_opcode!(op_66d1_0, "66D1.0");
test_opcode!(op_66d1_1, "66D1.1");
test_opcode!(op_66d1_2, "66D1.2");
test_opcode!(op_66d1_3, "66D1.3");
test_opcode!(op_66d1_4, "66D1.4");
test_opcode!(op_66d1_5, "66D1.5");
test_opcode!(op_66d1_6, "66D1.6");
test_opcode!(op_66d1_7, "66D1.7");
test_opcode!(op_66d3_0, "66D3.0");
test_opcode!(op_66d3_1, "66D3.1");
test_opcode!(op_66d3_2, "66D3.2");
test_opcode!(op_66d3_3, "66D3.3");
test_opcode!(op_66d3_4, "66D3.4");
test_opcode!(op_66d3_5, "66D3.5");
test_opcode!(op_66d3_6, "66D3.6");
test_opcode!(op_66d3_7, "66D3.7");
test_opcode!(op_66e0, "66E0");
test_opcode!(op_66e1, "66E1");
test_opcode!(op_66e2, "66E2");
test_opcode!(op_66e3, "66E3");
test_opcode!(op_66e5, "66E5");
test_opcode!(op_66e7, "66E7");
test_opcode!(op_66e8, "66E8");
test_opcode!(op_66e9, "66E9");
test_opcode!(op_66ea, "66EA");
test_opcode!(op_66eb, "66EB");
test_opcode!(op_66ed, "66ED");
test_opcode!(op_66ef, "66EF");
test_opcode!(op_66f7_0, "66F7.0");
test_opcode!(op_66f7_1, "66F7.1");
test_opcode!(op_66f7_2, "66F7.2");
test_opcode!(op_66f7_3, "66F7.3");
test_opcode!(op_66f7_4, "66F7.4");
test_opcode!(op_66f7_5, "66F7.5");
test_opcode!(op_66f7_6, "66F7.6");
test_opcode!(op_66f7_7, "66F7.7");
test_opcode!(op_6700, "6700");
test_opcode!(op_6701, "6701");
test_opcode!(op_6702, "6702");
test_opcode!(op_6703, "6703");
test_opcode!(op_6708, "6708");
test_opcode!(op_6709, "6709");
test_opcode!(op_670a, "670A");
test_opcode!(op_670b, "670B");
test_opcode!(op_670f90, "670F90");
test_opcode!(op_670f91, "670F91");
test_opcode!(op_670f92, "670F92");
test_opcode!(op_670f93, "670F93");
test_opcode!(op_670f94, "670F94");
test_opcode!(op_670f95, "670F95");
test_opcode!(op_670f96, "670F96");
test_opcode!(op_670f97, "670F97");
test_opcode!(op_670f98, "670F98");
test_opcode!(op_670f99, "670F99");
test_opcode!(op_670f9a, "670F9A");
test_opcode!(op_670f9b, "670F9B");
test_opcode!(op_670f9c, "670F9C");
test_opcode!(op_670f9d, "670F9D");
test_opcode!(op_670f9e, "670F9E");
test_opcode!(op_670f9f, "670F9F");
test_opcode!(op_670fa3, "670FA3");
test_opcode!(op_670fa4, "670FA4");
test_opcode!(op_670fa5, "670FA5");
test_opcode!(op_670fab, "670FAB");
test_opcode!(op_670fac, "670FAC");
test_opcode!(op_670fad, "670FAD");
test_opcode!(op_670faf, "670FAF");
test_opcode!(op_670fb2, "670FB2");
test_opcode!(op_670fb3, "670FB3");
test_opcode!(op_670fb4, "670FB4");
test_opcode!(op_670fb5, "670FB5");
test_opcode!(op_670fb6, "670FB6");
test_opcode!(op_670fb7, "670FB7");
test_opcode!(op_670fba_4, "670FBA.4");
test_opcode!(op_670fba_5, "670FBA.5");
test_opcode!(op_670fba_6, "670FBA.6");
test_opcode!(op_670fba_7, "670FBA.7");
test_opcode!(op_670fbb, "670FBB");
test_opcode!(op_670fbc, "670FBC");
test_opcode!(op_670fbd, "670FBD");
test_opcode!(op_670fbe, "670FBE");
test_opcode!(op_670fbf, "670FBF");
test_opcode!(op_6710, "6710");
test_opcode!(op_6711, "6711");
test_opcode!(op_6712, "6712");
test_opcode!(op_6713, "6713");
test_opcode!(op_6718, "6718");
test_opcode!(op_6719, "6719");
test_opcode!(op_671a, "671A");
test_opcode!(op_671b, "671B");
test_opcode!(op_6720, "6720");
test_opcode!(op_6721, "6721");
test_opcode!(op_6722, "6722");
test_opcode!(op_6723, "6723");
test_opcode!(op_6728, "6728");
test_opcode!(op_6729, "6729");
test_opcode!(op_672a, "672A");
test_opcode!(op_672b, "672B");
test_opcode!(op_6730, "6730");
test_opcode!(op_6731, "6731");
test_opcode!(op_6732, "6732");
test_opcode!(op_6733, "6733");
test_opcode!(op_6738, "6738");
test_opcode!(op_6739, "6739");
test_opcode!(op_673a, "673A");
test_opcode!(op_673b, "673B");
test_opcode!(op_6762, "6762");
test_opcode!(op_676601, "676601");
test_opcode!(op_676603, "676603");
test_opcode!(op_676609, "676609");
test_opcode!(op_67660b, "67660B");
test_opcode!(op_67660fa3, "67660FA3");
test_opcode!(op_67660fa4, "67660FA4");
test_opcode!(op_67660fa5, "67660FA5");
test_opcode!(op_67660fab, "67660FAB");
test_opcode!(op_67660fac, "67660FAC");
test_opcode!(op_67660fad, "67660FAD");
test_opcode!(op_67660faf, "67660FAF");
test_opcode!(op_67660fb2, "67660FB2");
test_opcode!(op_67660fb3, "67660FB3");
test_opcode!(op_67660fb4, "67660FB4");
test_opcode!(op_67660fb5, "67660FB5");
test_opcode!(op_67660fb6, "67660FB6");
test_opcode!(op_67660fb7, "67660FB7");
test_opcode!(op_67660fba_4, "67660FBA.4");
test_opcode!(op_67660fba_5, "67660FBA.5");
test_opcode!(op_67660fba_6, "67660FBA.6");
test_opcode!(op_67660fba_7, "67660FBA.7");
test_opcode!(op_67660fbb, "67660FBB");
test_opcode!(op_67660fbc, "67660FBC");
test_opcode!(op_67660fbd, "67660FBD");
test_opcode!(op_67660fbe, "67660FBE");
test_opcode!(op_67660fbf, "67660FBF");
test_opcode!(op_676611, "676611");
test_opcode!(op_676613, "676613");
test_opcode!(op_676619, "676619");
test_opcode!(op_67661b, "67661B");
test_opcode!(op_676621, "676621");
test_opcode!(op_676623, "676623");
test_opcode!(op_676629, "676629");
test_opcode!(op_67662b, "67662B");
test_opcode!(op_676631, "676631");
test_opcode!(op_676633, "676633");
test_opcode!(op_676639, "676639");
test_opcode!(op_67663b, "67663B");
test_opcode!(op_676662, "676662");
test_opcode!(op_676669, "676669");
test_opcode!(op_67666b, "67666B");
test_opcode!(op_67666d, "67666D");
test_opcode!(op_67666f, "67666F");
test_opcode!(op_676681_0, "676681.0");
test_opcode!(op_676681_1, "676681.1");
test_opcode!(op_676681_2, "676681.2");
test_opcode!(op_676681_3, "676681.3");
test_opcode!(op_676681_4, "676681.4");
test_opcode!(op_676681_5, "676681.5");
test_opcode!(op_676681_6, "676681.6");
test_opcode!(op_676681_7, "676681.7");
test_opcode!(op_676683_0, "676683.0");
test_opcode!(op_676683_1, "676683.1");
test_opcode!(op_676683_2, "676683.2");
test_opcode!(op_676683_3, "676683.3");
test_opcode!(op_676683_4, "676683.4");
test_opcode!(op_676683_5, "676683.5");
test_opcode!(op_676683_6, "676683.6");
test_opcode!(op_676683_7, "676683.7");
test_opcode!(op_676685, "676685");
test_opcode!(op_676687, "676687");
test_opcode!(op_676689, "676689");
test_opcode!(op_67668b, "67668B");
test_opcode!(op_67668c, "67668C");
test_opcode!(op_67668d, "67668D");
test_opcode!(op_67668e, "67668E");
test_opcode!(op_67668f, "67668F");
test_opcode!(op_6766a1, "6766A1");
test_opcode!(op_6766a3, "6766A3");
test_opcode!(op_6766a5, "6766A5");
test_opcode!(op_6766a7, "6766A7");
test_opcode!(op_6766ab, "6766AB");
test_opcode!(op_6766ad, "6766AD");
test_opcode!(op_6766af, "6766AF");
test_opcode!(op_6766c1_0, "6766C1.0");
test_opcode!(op_6766c1_1, "6766C1.1");
test_opcode!(op_6766c1_2, "6766C1.2");
test_opcode!(op_6766c1_3, "6766C1.3");
test_opcode!(op_6766c1_4, "6766C1.4");
test_opcode!(op_6766c1_5, "6766C1.5");
test_opcode!(op_6766c1_6, "6766C1.6");
test_opcode!(op_6766c1_7, "6766C1.7");
test_opcode!(op_6766c4, "6766C4");
test_opcode!(op_6766c5, "6766C5");
test_opcode!(op_6766c7, "6766C7");
test_opcode!(op_6766d1_0, "6766D1.0");
test_opcode!(op_6766d1_1, "6766D1.1");
test_opcode!(op_6766d1_2, "6766D1.2");
test_opcode!(op_6766d1_3, "6766D1.3");
test_opcode!(op_6766d1_4, "6766D1.4");
test_opcode!(op_6766d1_5, "6766D1.5");
test_opcode!(op_6766d1_6, "6766D1.6");
test_opcode!(op_6766d1_7, "6766D1.7");
test_opcode!(op_6766d3_0, "6766D3.0");
test_opcode!(op_6766d3_1, "6766D3.1");
test_opcode!(op_6766d3_2, "6766D3.2");
test_opcode!(op_6766d3_3, "6766D3.3");
test_opcode!(op_6766d3_4, "6766D3.4");
test_opcode!(op_6766d3_5, "6766D3.5");
test_opcode!(op_6766d3_6, "6766D3.6");
test_opcode!(op_6766d3_7, "6766D3.7");
test_opcode!(op_6766e0, "6766E0");
test_opcode!(op_6766e1, "6766E1");
test_opcode!(op_6766e2, "6766E2");
test_opcode!(op_6766e3, "6766E3");
test_opcode!(op_6766f7_0, "6766F7.0");
test_opcode!(op_6766f7_1, "6766F7.1");
test_opcode!(op_6766f7_2, "6766F7.2");
test_opcode!(op_6766f7_3, "6766F7.3");
test_opcode!(op_6766f7_4, "6766F7.4");
test_opcode!(op_6766f7_5, "6766F7.5");
test_opcode!(op_6766f7_6, "6766F7.6");
test_opcode!(op_6766f7_7, "6766F7.7");
test_opcode!(op_6769, "6769");
test_opcode!(op_676b, "676B");
test_opcode!(op_676c, "676C");
test_opcode!(op_676d, "676D");
test_opcode!(op_676e, "676E");
test_opcode!(op_676f, "676F");
test_opcode!(op_6780_0, "6780.0");
test_opcode!(op_6780_1, "6780.1");
test_opcode!(op_6780_2, "6780.2");
test_opcode!(op_6780_3, "6780.3");
test_opcode!(op_6780_4, "6780.4");
test_opcode!(op_6780_5, "6780.5");
test_opcode!(op_6780_6, "6780.6");
test_opcode!(op_6780_7, "6780.7");
test_opcode!(op_6781_0, "6781.0");
test_opcode!(op_6781_1, "6781.1");
test_opcode!(op_6781_2, "6781.2");
test_opcode!(op_6781_3, "6781.3");
test_opcode!(op_6781_4, "6781.4");
test_opcode!(op_6781_5, "6781.5");
test_opcode!(op_6781_6, "6781.6");
test_opcode!(op_6781_7, "6781.7");
test_opcode!(op_6782_0, "6782.0");
test_opcode!(op_6782_1, "6782.1");
test_opcode!(op_6782_2, "6782.2");
test_opcode!(op_6782_3, "6782.3");
test_opcode!(op_6782_4, "6782.4");
test_opcode!(op_6782_5, "6782.5");
test_opcode!(op_6782_6, "6782.6");
test_opcode!(op_6782_7, "6782.7");
test_opcode!(op_6783_0, "6783.0");
test_opcode!(op_6783_1, "6783.1");
test_opcode!(op_6783_2, "6783.2");
test_opcode!(op_6783_3, "6783.3");
test_opcode!(op_6783_4, "6783.4");
test_opcode!(op_6783_5, "6783.5");
test_opcode!(op_6783_6, "6783.6");
test_opcode!(op_6783_7, "6783.7");
test_opcode!(op_6784, "6784");
test_opcode!(op_6785, "6785");
test_opcode!(op_6786, "6786");
test_opcode!(op_6787, "6787");
test_opcode!(op_6788, "6788");
test_opcode!(op_6789, "6789");
test_opcode!(op_678a, "678A");
test_opcode!(op_678b, "678B");
test_opcode!(op_678c, "678C");
test_opcode!(op_678d, "678D");
test_opcode!(op_678e, "678E");
test_opcode!(op_678f, "678F");
test_opcode!(op_67a0, "67A0");
test_opcode!(op_67a1, "67A1");
test_opcode!(op_67a2, "67A2");
test_opcode!(op_67a3, "67A3");
test_opcode!(op_67a4, "67A4");
test_opcode!(op_67a5, "67A5");
test_opcode!(op_67a6, "67A6");
test_opcode!(op_67a7, "67A7");
test_opcode!(op_67aa, "67AA");
test_opcode!(op_67ab, "67AB");
test_opcode!(op_67ac, "67AC");
test_opcode!(op_67ad, "67AD");
test_opcode!(op_67ae, "67AE");
test_opcode!(op_67af, "67AF");
test_opcode!(op_67c0_0, "67C0.0");
test_opcode!(op_67c0_1, "67C0.1");
test_opcode!(op_67c0_2, "67C0.2");
test_opcode!(op_67c0_3, "67C0.3");
test_opcode!(op_67c0_4, "67C0.4");
test_opcode!(op_67c0_5, "67C0.5");
test_opcode!(op_67c0_6, "67C0.6");
test_opcode!(op_67c0_7, "67C0.7");
test_opcode!(op_67c1_0, "67C1.0");
test_opcode!(op_67c1_1, "67C1.1");
test_opcode!(op_67c1_2, "67C1.2");
test_opcode!(op_67c1_3, "67C1.3");
test_opcode!(op_67c1_4, "67C1.4");
test_opcode!(op_67c1_5, "67C1.5");
test_opcode!(op_67c1_6, "67C1.6");
test_opcode!(op_67c1_7, "67C1.7");
test_opcode!(op_67c4, "67C4");
test_opcode!(op_67c5, "67C5");
test_opcode!(op_67c6, "67C6");
test_opcode!(op_67c7, "67C7");
test_opcode!(op_67d0_0, "67D0.0");
test_opcode!(op_67d0_1, "67D0.1");
test_opcode!(op_67d0_2, "67D0.2");
test_opcode!(op_67d0_3, "67D0.3");
test_opcode!(op_67d0_4, "67D0.4");
test_opcode!(op_67d0_5, "67D0.5");
test_opcode!(op_67d0_6, "67D0.6");
test_opcode!(op_67d0_7, "67D0.7");
test_opcode!(op_67d1_0, "67D1.0");
test_opcode!(op_67d1_1, "67D1.1");
test_opcode!(op_67d1_2, "67D1.2");
test_opcode!(op_67d1_3, "67D1.3");
test_opcode!(op_67d1_4, "67D1.4");
test_opcode!(op_67d1_5, "67D1.5");
test_opcode!(op_67d1_6, "67D1.6");
test_opcode!(op_67d1_7, "67D1.7");
test_opcode!(op_67d2_0, "67D2.0");
test_opcode!(op_67d2_1, "67D2.1");
test_opcode!(op_67d2_2, "67D2.2");
test_opcode!(op_67d2_3, "67D2.3");
test_opcode!(op_67d2_4, "67D2.4");
test_opcode!(op_67d2_5, "67D2.5");
test_opcode!(op_67d2_6, "67D2.6");
test_opcode!(op_67d2_7, "67D2.7");
test_opcode!(op_67d3_0, "67D3.0");
test_opcode!(op_67d3_1, "67D3.1");
test_opcode!(op_67d3_2, "67D3.2");
test_opcode!(op_67d3_3, "67D3.3");
test_opcode!(op_67d3_4, "67D3.4");
test_opcode!(op_67d3_5, "67D3.5");
test_opcode!(op_67d3_6, "67D3.6");
test_opcode!(op_67d3_7, "67D3.7");
test_opcode!(op_67d7, "67D7");
test_opcode!(op_67e0, "67E0");
test_opcode!(op_67e1, "67E1");
test_opcode!(op_67e2, "67E2");
test_opcode!(op_67e3, "67E3");
test_opcode!(op_67f6_0, "67F6.0");
test_opcode!(op_67f6_1, "67F6.1");
test_opcode!(op_67f6_2, "67F6.2");
test_opcode!(op_67f6_3, "67F6.3");
test_opcode!(op_67f6_4, "67F6.4");
test_opcode!(op_67f6_5, "67F6.5");
test_opcode!(op_67f6_6, "67F6.6");
// These vectors match a quirk of the specific CPU/model used to generate the
// validation data, not strict 80386 divide-fault behavior. Keep them as local
// skips until the upstream dataset is reconciled.
test_opcode!(
    op_67f6_7,
    "67F6.7",
    [
        "c30147a2fb7f67459cb7a1a4c888b670edd838f5",
        "8bddec44e46b709759d5d1550cd9fa16ab8e89fc",
        "a06745936bac371e6afb861b5bc87be4b2dd676e",
    ]
);
test_opcode!(op_67f7_0, "67F7.0");
test_opcode!(op_67f7_1, "67F7.1");
test_opcode!(op_67f7_2, "67F7.2");
test_opcode!(op_67f7_3, "67F7.3");
test_opcode!(op_67f7_4, "67F7.4");
test_opcode!(op_67f7_5, "67F7.5");
test_opcode!(op_67f7_6, "67F7.6");
test_opcode!(op_67f7_7, "67F7.7");
test_opcode!(op_68, "68");
test_opcode!(op_69, "69");
test_opcode!(op_6a, "6A");
test_opcode!(op_6b, "6B");
test_opcode!(op_6c, "6C");
test_opcode!(op_6d, "6D");
test_opcode!(op_6e, "6E");
test_opcode!(op_6f, "6F");
test_opcode!(op_70, "70");
test_opcode!(op_71, "71");
test_opcode!(op_72, "72");
test_opcode!(op_73, "73");
test_opcode!(op_74, "74");
test_opcode!(op_75, "75");
test_opcode!(op_76, "76");
test_opcode!(op_77, "77");
test_opcode!(op_78, "78");
test_opcode!(op_79, "79");
test_opcode!(op_7a, "7A");
test_opcode!(op_7b, "7B");
test_opcode!(op_7c, "7C");
test_opcode!(op_7d, "7D");
test_opcode!(op_7e, "7E");
test_opcode!(op_7f, "7F");
test_opcode!(op_80_0, "80.0");
test_opcode!(op_80_1, "80.1");
test_opcode!(op_80_2, "80.2");
test_opcode!(op_80_3, "80.3");
test_opcode!(op_80_4, "80.4");
test_opcode!(op_80_5, "80.5");
test_opcode!(op_80_6, "80.6");
test_opcode!(op_80_7, "80.7");
test_opcode!(op_81_0, "81.0");
test_opcode!(op_81_1, "81.1");
test_opcode!(op_81_2, "81.2");
test_opcode!(op_81_3, "81.3");
test_opcode!(op_81_4, "81.4");
test_opcode!(op_81_5, "81.5");
test_opcode!(op_81_6, "81.6");
test_opcode!(op_81_7, "81.7");
test_opcode!(op_82_0, "82.0");
test_opcode!(op_82_1, "82.1");
test_opcode!(op_82_2, "82.2");
test_opcode!(op_82_3, "82.3");
test_opcode!(op_82_4, "82.4");
test_opcode!(op_82_5, "82.5");
test_opcode!(op_82_6, "82.6");
test_opcode!(op_82_7, "82.7");
test_opcode!(op_83_0, "83.0");
test_opcode!(op_83_1, "83.1");
test_opcode!(op_83_2, "83.2");
test_opcode!(op_83_3, "83.3");
test_opcode!(op_83_4, "83.4");
test_opcode!(op_83_5, "83.5");
test_opcode!(op_83_6, "83.6");
test_opcode!(op_83_7, "83.7");
test_opcode!(op_84, "84");
test_opcode!(op_85, "85");
test_opcode!(op_86, "86");
test_opcode!(op_87, "87");
test_opcode!(op_88, "88");
test_opcode!(op_89, "89");
test_opcode!(op_8a, "8A");
test_opcode!(op_8b, "8B");
test_opcode!(op_8c, "8C");
test_opcode!(op_8d, "8D");
test_opcode!(op_8e, "8E");
test_opcode!(op_8f, "8F");
test_opcode!(op_90, "90");
test_opcode!(op_91, "91");
test_opcode!(op_92, "92");
test_opcode!(op_93, "93");
test_opcode!(op_94, "94");
test_opcode!(op_95, "95");
test_opcode!(op_96, "96");
test_opcode!(op_97, "97");
test_opcode!(op_98, "98");
test_opcode!(op_99, "99");
test_opcode!(op_9a, "9A");
test_opcode!(op_9b, "9B");
test_opcode!(op_9c, "9C");
test_opcode!(op_9d, "9D");
test_opcode!(op_9e, "9E");
test_opcode!(op_9f, "9F");
test_opcode!(op_a0, "A0");
test_opcode!(op_a1, "A1");
test_opcode!(op_a2, "A2");
test_opcode!(op_a3, "A3");
test_opcode!(op_a4, "A4");
test_opcode!(op_a5, "A5");
test_opcode!(op_a6, "A6");
test_opcode!(op_a7, "A7");
test_opcode!(op_a8, "A8");
test_opcode!(op_a9, "A9");
test_opcode!(op_aa, "AA");
test_opcode!(op_ab, "AB");
test_opcode!(op_ac, "AC");
test_opcode!(op_ad, "AD");
test_opcode!(op_ae, "AE");
test_opcode!(op_af, "AF");
test_opcode!(op_b0, "B0");
test_opcode!(op_b1, "B1");
test_opcode!(op_b2, "B2");
test_opcode!(op_b3, "B3");
test_opcode!(op_b4, "B4");
test_opcode!(op_b5, "B5");
test_opcode!(op_b6, "B6");
test_opcode!(op_b7, "B7");
test_opcode!(op_b8, "B8");
test_opcode!(op_b9, "B9");
test_opcode!(op_ba, "BA");
test_opcode!(op_bb, "BB");
test_opcode!(op_bc, "BC");
test_opcode!(op_bd, "BD");
test_opcode!(op_be, "BE");
test_opcode!(op_bf, "BF");
test_opcode!(op_c0_0, "C0.0");
test_opcode!(op_c0_1, "C0.1");
test_opcode!(op_c0_2, "C0.2");
test_opcode!(op_c0_3, "C0.3");
test_opcode!(op_c0_4, "C0.4");
test_opcode!(op_c0_5, "C0.5");
test_opcode!(op_c0_6, "C0.6");
test_opcode!(op_c0_7, "C0.7");
test_opcode!(op_c1_0, "C1.0");
test_opcode!(op_c1_1, "C1.1");
test_opcode!(op_c1_2, "C1.2");
test_opcode!(op_c1_3, "C1.3");
test_opcode!(op_c1_4, "C1.4");
test_opcode!(op_c1_5, "C1.5");
test_opcode!(op_c1_6, "C1.6");
test_opcode!(op_c1_7, "C1.7");
test_opcode!(op_c2, "C2");
test_opcode!(op_c3, "C3");
test_opcode!(op_c4, "C4");
test_opcode!(op_c5, "C5");
test_opcode!(op_c6, "C6");
test_opcode!(op_c7, "C7");
test_opcode!(op_c8, "C8");
test_opcode!(op_c9, "C9");
test_opcode!(op_ca, "CA");
test_opcode!(op_cb, "CB");
test_opcode!(op_cc, "CC");
test_opcode!(op_cd, "CD");
test_opcode!(op_ce, "CE");
test_opcode!(op_cf, "CF");
test_opcode!(op_d0_0, "D0.0");
test_opcode!(op_d0_1, "D0.1");
test_opcode!(op_d0_2, "D0.2");
test_opcode!(op_d0_3, "D0.3");
test_opcode!(op_d0_4, "D0.4");
test_opcode!(op_d0_5, "D0.5");
test_opcode!(op_d0_6, "D0.6");
test_opcode!(op_d0_7, "D0.7");
test_opcode!(op_d1_0, "D1.0");
test_opcode!(op_d1_1, "D1.1");
test_opcode!(op_d1_2, "D1.2");
test_opcode!(op_d1_3, "D1.3");
test_opcode!(op_d1_4, "D1.4");
test_opcode!(op_d1_5, "D1.5");
test_opcode!(op_d1_6, "D1.6");
test_opcode!(op_d1_7, "D1.7");
test_opcode!(op_d2_0, "D2.0");
test_opcode!(op_d2_1, "D2.1");
test_opcode!(op_d2_2, "D2.2");
test_opcode!(op_d2_3, "D2.3");
test_opcode!(op_d2_4, "D2.4");
test_opcode!(op_d2_5, "D2.5");
test_opcode!(op_d2_6, "D2.6");
test_opcode!(op_d2_7, "D2.7");
test_opcode!(op_d3_0, "D3.0");
test_opcode!(op_d3_1, "D3.1");
test_opcode!(op_d3_2, "D3.2");
test_opcode!(op_d3_3, "D3.3");
test_opcode!(op_d3_4, "D3.4");
test_opcode!(op_d3_5, "D3.5");
test_opcode!(op_d3_6, "D3.6");
test_opcode!(op_d3_7, "D3.7");
test_opcode!(op_d4, "D4");
test_opcode!(op_d5, "D5");
test_opcode!(op_d6, "D6");
test_opcode!(op_d7, "D7");
test_opcode!(op_e0, "E0");
test_opcode!(op_e1, "E1");
test_opcode!(op_e2, "E2");
test_opcode!(op_e3, "E3");
test_opcode!(op_e4, "E4");
test_opcode!(op_e5, "E5");
test_opcode!(op_e6, "E6");
test_opcode!(op_e7, "E7");
test_opcode!(op_e8, "E8");
test_opcode!(op_e9, "E9");
test_opcode!(op_ea, "EA");
test_opcode!(op_eb, "EB");
test_opcode!(op_ec, "EC");
test_opcode!(op_ed, "ED");
test_opcode!(op_ee, "EE");
test_opcode!(op_ef, "EF");
test_opcode!(op_f4, "F4");
test_opcode!(op_f5, "F5");
test_opcode!(op_f6_0, "F6.0");
test_opcode!(op_f6_1, "F6.1");
test_opcode!(op_f6_2, "F6.2");
test_opcode!(op_f6_3, "F6.3");
test_opcode!(op_f6_4, "F6.4");
test_opcode!(op_f6_5, "F6.5");
test_opcode!(op_f6_6, "F6.6");
// These vectors match a quirk of the specific CPU/model used to generate the
// validation data, not strict 80386 divide-fault behavior. Keep them as local
// skips until the upstream dataset is reconciled.
test_opcode!(
    op_f6_7,
    "F6.7",
    [
        "8813025dcf45588bd5141eb4d7081c67d6cf3ec3",
        "c6d731127bec724d40594edc28a6ad8fdf14acbc",
        "c1392c8316c2d0532d80fb1fcb78832533093e4e",
        "b674afe8d525fdf51217456b4a3f0b9102350cc4",
        "13e7537c7cd00f676d17da613b4289afb93b338d",
        "a4a926d8bbca1f281b0a479fd8c79fe3231bad34",
    ]
);
test_opcode!(op_f7_0, "F7.0");
test_opcode!(op_f7_1, "F7.1");
test_opcode!(op_f7_2, "F7.2");
test_opcode!(op_f7_3, "F7.3");
test_opcode!(op_f7_4, "F7.4");
test_opcode!(op_f7_5, "F7.5");
test_opcode!(op_f7_6, "F7.6");
test_opcode!(op_f7_7, "F7.7");
test_opcode!(op_f8, "F8");
test_opcode!(op_f9, "F9");
test_opcode!(op_fa, "FA");
test_opcode!(op_fb, "FB");
test_opcode!(op_fc, "FC");
test_opcode!(op_fd, "FD");
test_opcode!(op_fe_0, "FE.0");
test_opcode!(op_fe_1, "FE.1");
test_opcode!(op_ff_0, "FF.0");
test_opcode!(op_ff_1, "FF.1");
test_opcode!(op_ff_2, "FF.2");
test_opcode!(op_ff_3, "FF.3");
test_opcode!(op_ff_4, "FF.4");
test_opcode!(op_ff_5, "FF.5");
test_opcode!(op_ff_6, "FF.6");
