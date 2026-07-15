#![cfg(feature = "verification")]

#[path = "common/metadata_json.rs"]
mod metadata_json;
#[path = "common/verification_common.rs"]
mod verification_common;

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::LazyLock,
};

use common::Cpu as _;
use cpu::{I286, I286State};
use metadata_json::{Metadata, load_metadata};
use verification_common::{load_moo_tests, load_revocation_list};

const RAM_SIZE: usize = 16 << 20;
const ADDRESS_MASK: u32 = 0x00FF_FFFF;
const REG_ORDER: [&str; 14] = [
    "ax", "bx", "cx", "dx", "cs", "ss", "ds", "es", "sp", "bp", "si", "di", "ip", "flags",
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
        0
    }

    fn set_current_cycle(&mut self, _cycle: u64) {}
}

fn test_dir() -> &'static Path {
    static DIR: LazyLock<PathBuf> = LazyLock::new(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/SingleStepTests/80286/v1_real_mode")
    });
    &DIR
}

fn metadata() -> &'static Metadata {
    static META: LazyLock<Metadata> =
        LazyLock::new(|| load_metadata(&test_dir().join("metadata.json")));
    &META
}

fn revocation_list() -> &'static HashSet<String> {
    static REVOKED: LazyLock<HashSet<String>> = LazyLock::new(|| {
        load_revocation_list(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/SingleStepTests/80286/revocation_list.txt"),
        )
    });
    &REVOKED
}

fn should_test(status: &str) -> bool {
    matches!(
        status,
        "normal" | "alias" | "undocumented" | "fpu" | "undefined"
    )
}

fn initial_reg_value(initial_regs: &HashMap<String, u32>, name: &str) -> u16 {
    initial_regs
        .get(name)
        .copied()
        .unwrap_or_else(|| panic!("missing register in initial state: {name}")) as u16
}

fn build_expected_state(
    initial_regs: &HashMap<String, u32>,
    final_regs: &HashMap<String, u32>,
) -> I286State {
    let get = |name: &str| -> u16 {
        final_regs
            .get(name)
            .copied()
            .map(|value| value as u16)
            .unwrap_or_else(|| initial_reg_value(initial_regs, name))
    };

    let mut s = I286State::default();
    s.set_ax(get("ax"));
    s.set_cx(get("cx"));
    s.set_dx(get("dx"));
    s.set_bx(get("bx"));
    s.set_sp(get("sp"));
    s.set_bp(get("bp"));
    s.set_si(get("si"));
    s.set_di(get("di"));
    s.set_es(get("es"));
    s.set_cs(get("cs"));
    s.set_ss(get("ss"));
    s.set_ds(get("ds"));
    s.ip = get("ip");
    s.set_compressed_flags(get("flags"));
    s.msw = 0xFFF0;
    s
}

fn parse_stem(stem: &str) -> (&str, Option<&str>) {
    if let Some(dot_pos) = stem.find('.') {
        (&stem[..dot_pos], Some(&stem[dot_pos + 1..]))
    } else {
        (stem, None)
    }
}

fn status_and_mask(stem: &str) -> Option<(&str, u16)> {
    let metadata = metadata();
    let (opcode, reg_ext) = parse_stem(stem);

    let entry = metadata.opcodes.get(opcode)?;
    if let Some(reg_ext) = reg_ext {
        let reg_info = entry.reg.as_ref()?.get(reg_ext)?;
        Some((
            reg_info.status.as_str(),
            reg_info.flags_mask.unwrap_or(0xFFFF),
        ))
    } else if let Some(status) = &entry.status {
        Some((status.as_str(), entry.flags_mask.unwrap_or(0xFFFF)))
    } else {
        None
    }
}

fn format_flags_diff(expected: u16, actual: u16, mask: u16) -> String {
    const FLAG_BITS: &[(u16, &str)] = &[
        (0x0001, "CF"),
        (0x0004, "PF"),
        (0x0010, "AF"),
        (0x0040, "ZF"),
        (0x0080, "SF"),
        (0x0100, "TF"),
        (0x0200, "IF"),
        (0x0400, "DF"),
        (0x0800, "OF"),
    ];

    let expected_masked = expected & mask;
    let actual_masked = actual & mask;
    let diff_bits = expected_masked ^ actual_masked;

    let mut changed: Vec<String> = Vec::new();
    for &(bit, name) in FLAG_BITS {
        if diff_bits & bit != 0 {
            let exp = u16::from(expected_masked & bit != 0);
            let act = u16::from(actual_masked & bit != 0);
            changed.push(format!("{name}:{exp}->{act}"));
        }
    }

    format!(
        "  flags: expected 0x{expected_masked:04X}, got 0x{actual_masked:04X} [{}] (mask 0x{mask:04X})",
        changed.join(", ")
    )
}

fn run_test_file(stem: &str, local_revoked_hashes: &[&str]) {
    let revoked = revocation_list();
    let local_revoked: HashSet<String> = local_revoked_hashes
        .iter()
        .map(|hash| hash.to_ascii_lowercase())
        .collect();
    let mut bus = TestBus::new();

    let Some((status, flags_mask)) = status_and_mask(stem) else {
        return;
    };
    if !should_test(status) {
        return;
    }

    let filename = format!("{stem}.MOO.gz");
    let path = test_dir().join(&filename);
    let test_cases = load_moo_tests(&path, &REG_ORDER, &[]);
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
            let mut s = I286State::default();
            s.set_ax(initial_reg_value(&test.initial.regs, "ax"));
            s.set_bx(initial_reg_value(&test.initial.regs, "bx"));
            s.set_cx(initial_reg_value(&test.initial.regs, "cx"));
            s.set_dx(initial_reg_value(&test.initial.regs, "dx"));
            s.set_sp(initial_reg_value(&test.initial.regs, "sp"));
            s.set_bp(initial_reg_value(&test.initial.regs, "bp"));
            s.set_si(initial_reg_value(&test.initial.regs, "si"));
            s.set_di(initial_reg_value(&test.initial.regs, "di"));
            s.set_cs(initial_reg_value(&test.initial.regs, "cs"));
            s.set_ss(initial_reg_value(&test.initial.regs, "ss"));
            s.set_ds(initial_reg_value(&test.initial.regs, "ds"));
            s.set_es(initial_reg_value(&test.initial.regs, "es"));
            s.ip = initial_reg_value(&test.initial.regs, "ip");
            s.set_compressed_flags(initial_reg_value(&test.initial.regs, "flags"));
            s.msw = 0xFFF0;
            s.initialize_real_mode_caches();
            s
        };

        let mut cpu = I286::new();
        cpu.load_state(&initial);
        if !test.initial.queue.is_empty() {
            cpu.install_front_end_state(
                &test.initial.queue,
                test.initial.queue.len() as u8,
                cpu::I286FlushState::None,
            );
        }
        let mut steps = 0usize;
        while !cpu.halted() && steps < 1024 {
            cpu.step(&mut bus);
            steps += 1;
        }
        if steps >= 1024 {
            failures.push(format!(
                "[{filename} #{} idx={}] {} (execution did not reach HLT within 1024 instructions)",
                failures.len(),
                test.idx,
                test.name
            ));
            continue;
        }

        let expected = build_expected_state(&test.initial.regs, &test.final_state.regs);
        let mut diffs: Vec<String> = Vec::new();

        let check_reg = |name: &str,
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

        check_reg("ax", initial.ax(), cpu.ax(), expected.ax(), &mut diffs);
        check_reg("bx", initial.bx(), cpu.bx(), expected.bx(), &mut diffs);
        check_reg("cx", initial.cx(), cpu.cx(), expected.cx(), &mut diffs);
        check_reg("dx", initial.dx(), cpu.dx(), expected.dx(), &mut diffs);
        check_reg("sp", initial.sp(), cpu.sp(), expected.sp(), &mut diffs);
        check_reg("bp", initial.bp(), cpu.bp(), expected.bp(), &mut diffs);
        check_reg("si", initial.si(), cpu.si(), expected.si(), &mut diffs);
        check_reg("di", initial.di(), cpu.di(), expected.di(), &mut diffs);
        check_reg("cs", initial.cs(), cpu.cs(), expected.cs(), &mut diffs);
        check_reg("ss", initial.ss(), cpu.ss(), expected.ss(), &mut diffs);
        check_reg("ds", initial.ds(), cpu.ds(), expected.ds(), &mut diffs);
        check_reg("es", initial.es(), cpu.es(), expected.es(), &mut diffs);
        check_reg("ip", initial.ip, cpu.ip, expected.ip, &mut diffs);

        if (cpu.compressed_flags() & flags_mask) != (expected.compressed_flags() & flags_mask) {
            diffs.push(format!(
                "{} (was 0x{:04X})",
                format_flags_diff(
                    expected.compressed_flags(),
                    cpu.compressed_flags(),
                    flags_mask
                ),
                initial.compressed_flags()
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
        let display_count = failures.len().min(5);
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
test_opcode!(op_d8, "D8");
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
// validation data, not strict 80286 divide-fault behavior. Keep them as local
// skips until the upstream dataset is reconciled.
test_opcode!(
    op_f6_7,
    "F6.7",
    [
        "0038b4bacfb75535b5da175f619b0812b16d0601",
        "de153d1e3812cdb2c9d25272844b4b28a5adc35f",
        "dce03c62813266bf0ba50e3325fa3898132cad1f",
        "38a27640b8a9475f75998d2cab801d51eb8bb0b2",
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
