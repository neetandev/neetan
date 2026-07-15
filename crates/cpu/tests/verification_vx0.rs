#![cfg(feature = "verification")]

#[path = "common/metadata_json.rs"]
mod metadata_json;
#[path = "common/verification_common.rs"]
mod verification_common;

use std::{collections::HashMap, path::Path, sync::LazyLock};

use cpu::{V30State, VX0};
use metadata_json::{Metadata, load_metadata};
use verification_common::{MooState, load_moo_tests};

const REG_ORDER_VX0: [&str; 14] = [
    "ax", "bx", "cx", "dx", "cs", "ss", "ds", "es", "sp", "bp", "si", "di", "ip", "flags",
];

struct TestBus {
    ram: Box<[u8; 1_048_576]>,
}

impl TestBus {
    fn new() -> Self {
        Self {
            ram: vec![0u8; 1_048_576].into_boxed_slice().try_into().unwrap(),
        }
    }
}

impl common::Bus for TestBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        self.ram[(address & 0xFFFFF) as usize]
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.ram[(address & 0xFFFFF) as usize] = value;
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
    static DIR: LazyLock<std::path::PathBuf> = LazyLock::new(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/SingleStepTests/v20/v1_native")
    });
    &DIR
}

fn metadata() -> &'static Metadata {
    static META: LazyLock<Metadata> =
        LazyLock::new(|| load_metadata(&test_dir().join("metadata.json")));
    &META
}

fn should_test(status: &str) -> bool {
    matches!(
        status,
        "normal" | "alias" | "undocumented" | "fpu" | "undefined"
    )
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
        (0x8000, "MD"),
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

fn initial_reg_value(regs: &HashMap<String, u32>, name: &str) -> u16 {
    regs.get(name)
        .copied()
        .unwrap_or_else(|| panic!("missing register in initial state: {name}")) as u16
}

fn build_state(regs: &HashMap<String, u16>) -> V30State {
    let get = |name: &str| -> u16 {
        regs.get(name)
            .copied()
            .unwrap_or_else(|| panic!("missing register: {name}"))
    };
    let mut s = V30State::default();
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
    s.initialize_cold_frontend();
    s
}

fn resolve_final_regs(
    initial: &HashMap<String, u32>,
    final_state: &HashMap<String, u32>,
) -> HashMap<String, u16> {
    REG_ORDER_VX0
        .iter()
        .map(|name| {
            let value = final_state
                .get(*name)
                .copied()
                .unwrap_or_else(|| initial_reg_value(initial, name) as u32);
            ((*name).to_string(), value as u16)
        })
        .collect()
}

fn resolve_initial_regs(initial: &HashMap<String, u32>) -> HashMap<String, u16> {
    REG_ORDER_VX0
        .iter()
        .map(|name| ((*name).to_string(), initial_reg_value(initial, name)))
        .collect()
}

fn is_division_exception(
    opcode: &str,
    reg_ext: Option<&str>,
    initial: &MooState,
    expected: &V30State,
) -> bool {
    let is_div = matches!(
        (opcode, reg_ext),
        ("F6", Some("6")) | ("F6", Some("7")) | ("F7", Some("6")) | ("F7", Some("7"))
    );
    if !is_div {
        return false;
    }

    let byte_at = |address: u32| -> u16 {
        initial
            .ram
            .iter()
            .find(|(candidate, _)| *candidate == address)
            .map(|(_, value)| *value as u16)
            .unwrap_or(0)
    };
    let handler_ip = byte_at(0) | (byte_at(1) << 8);
    let handler_cs = byte_at(2) | (byte_at(3) << 8);

    expected.cs() == handler_cs && expected.ip == handler_ip
}

fn parse_stem(stem: &str) -> (&str, Option<&str>) {
    if let Some(dot_pos) = stem.find('.') {
        (&stem[..dot_pos], Some(&stem[dot_pos + 1..]))
    } else {
        (stem, None)
    }
}

fn status_and_mask(stem: &str) -> Option<(&'static str, u16)> {
    let metadata = metadata();
    let (opcode, reg_ext) = parse_stem(stem);
    let entry = metadata.opcodes.get(opcode)?;
    match (reg_ext, &entry.reg) {
        (Some(reg_ext), Some(reg_map)) => {
            let info = reg_map.get(reg_ext)?;
            Some((info.status.as_str(), info.flags_mask.unwrap_or(0xFFFF)))
        }
        (Some(_), None) => None,
        (None, _) => {
            if let Some(status) = &entry.status {
                Some((status.as_str(), entry.flags_mask.unwrap_or(0xFFFF)))
            } else if let Some(reg_map) = &entry.reg {
                let any_testable = reg_map.values().any(|info| should_test(&info.status));
                if !any_testable {
                    return None;
                }
                let mask = reg_map
                    .values()
                    .map(|info| info.flags_mask.unwrap_or(0xFFFF))
                    .fold(0xFFFFu16, |acc, m| acc & m);
                Some(("normal", mask))
            } else {
                None
            }
        }
    }
}

fn run_test_file<const BUS: u8>(stem: &str) {
    let Some((status, flags_mask)) = status_and_mask(stem) else {
        return;
    };
    if !should_test(status) {
        return;
    }

    let (opcode, reg_ext) = parse_stem(stem);
    let filename = format!("{stem}.MOO.gz");
    let path = test_dir().join(&filename);
    let test_cases = load_moo_tests(&path, &REG_ORDER_VX0, &[]);

    let mut failures: Vec<String> = Vec::new();

    for (index, test) in test_cases.iter().enumerate() {
        let mut bus = TestBus::new();
        for &(address, value) in &test.initial.ram {
            bus.ram[(address & 0xFFFFF) as usize] = value;
        }

        let initial_regs16 = resolve_initial_regs(&test.initial.regs);
        let final_regs16 = resolve_final_regs(&test.initial.regs, &test.final_state.regs);
        let initial_state = build_state(&initial_regs16);
        let expected = build_state(&final_regs16);

        let mut cpu: VX0<BUS> = VX0::<BUS>::new();
        cpu.load_state(&initial_state);
        cpu.step(&mut bus);

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

        check_reg(
            "ax",
            initial_state.ax(),
            cpu.ax(),
            expected.ax(),
            &mut diffs,
        );
        check_reg(
            "bx",
            initial_state.bx(),
            cpu.bx(),
            expected.bx(),
            &mut diffs,
        );
        check_reg(
            "cx",
            initial_state.cx(),
            cpu.cx(),
            expected.cx(),
            &mut diffs,
        );
        check_reg(
            "dx",
            initial_state.dx(),
            cpu.dx(),
            expected.dx(),
            &mut diffs,
        );
        check_reg(
            "sp",
            initial_state.sp(),
            cpu.sp(),
            expected.sp(),
            &mut diffs,
        );
        check_reg(
            "bp",
            initial_state.bp(),
            cpu.bp(),
            expected.bp(),
            &mut diffs,
        );
        check_reg(
            "si",
            initial_state.si(),
            cpu.si(),
            expected.si(),
            &mut diffs,
        );
        check_reg(
            "di",
            initial_state.di(),
            cpu.di(),
            expected.di(),
            &mut diffs,
        );
        check_reg(
            "cs",
            initial_state.cs(),
            cpu.cs(),
            expected.cs(),
            &mut diffs,
        );
        check_reg(
            "ss",
            initial_state.ss(),
            cpu.ss(),
            expected.ss(),
            &mut diffs,
        );
        check_reg(
            "ds",
            initial_state.ds(),
            cpu.ds(),
            expected.ds(),
            &mut diffs,
        );
        check_reg(
            "es",
            initial_state.es(),
            cpu.es(),
            expected.es(),
            &mut diffs,
        );
        check_reg("ip", initial_state.ip, cpu.ip, expected.ip, &mut diffs);

        let cpu_flags_masked = cpu.compressed_flags() & flags_mask;
        let expected_flags_masked = expected.compressed_flags() & flags_mask;
        if cpu_flags_masked != expected_flags_masked {
            diffs.push(format!(
                "{} (was 0x{:04X})",
                format_flags_diff(
                    expected.compressed_flags(),
                    cpu.compressed_flags(),
                    flags_mask
                ),
                initial_state.compressed_flags()
            ));
        }

        let div_exception = is_division_exception(opcode, reg_ext, &test.initial, &expected);

        if !div_exception {
            for &(address, expected_value) in &test.final_state.ram {
                let actual_value = bus.ram[(address & 0xFFFFF) as usize];
                if actual_value != expected_value {
                    let initial_value = test
                        .initial
                        .ram
                        .iter()
                        .find(|(candidate, _)| *candidate == address)
                        .map(|(_, value)| *value);
                    match initial_value {
                        Some(before) => diffs.push(format!(
                            "  ram[0x{address:05X}]: expected 0x{expected_value:02X}, got 0x{actual_value:02X} (was 0x{before:02X})"
                        )),
                        None => diffs.push(format!(
                            "  ram[0x{address:05X}]: expected 0x{expected_value:02X}, got 0x{actual_value:02X} (not in initial RAM)"
                        )),
                    }
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
                "[{filename} #{index}] {} ({})\n{}",
                test.name,
                bytes_hex.join(" "),
                diffs.join("\n")
            ));
        }
    }

    if !failures.is_empty() {
        let fail_count = failures.len();
        let test_count = test_cases.len();
        let mut message = format!("{filename}: {fail_count}/{test_count} tests failed\n");
        let display_count = failures.len().min(5);
        for failure in &failures[..display_count] {
            message.push_str(failure);
            message.push('\n');
        }
        if failures.len() > 5 {
            message.push_str(&format!("  ... and {} more failures\n", failures.len() - 5));
        }
        panic!("{message}");
    }
}

macro_rules! tests_for_all_opcodes {
    ($emit:ident) => {
        $emit!(op_00, "00");
        $emit!(op_01, "01");
        $emit!(op_02, "02");
        $emit!(op_03, "03");
        $emit!(op_04, "04");
        $emit!(op_05, "05");
        $emit!(op_06, "06");
        $emit!(op_07, "07");
        $emit!(op_08, "08");
        $emit!(op_09, "09");
        $emit!(op_0a, "0A");
        $emit!(op_0b, "0B");
        $emit!(op_0c, "0C");
        $emit!(op_0d, "0D");
        $emit!(op_0e, "0E");
        $emit!(op_0f10, "0F10");
        $emit!(op_0f11, "0F11");
        $emit!(op_0f12, "0F12");
        $emit!(op_0f13, "0F13");
        $emit!(op_0f14, "0F14");
        $emit!(op_0f15, "0F15");
        $emit!(op_0f16, "0F16");
        $emit!(op_0f17, "0F17");
        $emit!(op_0f18, "0F18");
        $emit!(op_0f19, "0F19");
        $emit!(op_0f1a, "0F1A");
        $emit!(op_0f1b, "0F1B");
        $emit!(op_0f1c, "0F1C");
        $emit!(op_0f1d, "0F1D");
        $emit!(op_0f1e, "0F1E");
        $emit!(op_0f1f, "0F1F");
        $emit!(op_0f20, "0F20");
        $emit!(op_0f22, "0F22");
        $emit!(op_0f26, "0F26");
        $emit!(op_0f28, "0F28");
        $emit!(op_0f2a, "0F2A");
        $emit!(op_0f31, "0F31");
        $emit!(op_0f33, "0F33");
        $emit!(op_0f39, "0F39");
        $emit!(op_0f3b, "0F3B");
        $emit!(op_10, "10");
        $emit!(op_11, "11");
        $emit!(op_12, "12");
        $emit!(op_13, "13");
        $emit!(op_14, "14");
        $emit!(op_15, "15");
        $emit!(op_16, "16");
        $emit!(op_17, "17");
        $emit!(op_18, "18");
        $emit!(op_19, "19");
        $emit!(op_1a, "1A");
        $emit!(op_1b, "1B");
        $emit!(op_1c, "1C");
        $emit!(op_1d, "1D");
        $emit!(op_1e, "1E");
        $emit!(op_1f, "1F");
        $emit!(op_20, "20");
        $emit!(op_21, "21");
        $emit!(op_22, "22");
        $emit!(op_23, "23");
        $emit!(op_24, "24");
        $emit!(op_25, "25");
        $emit!(op_27, "27");
        $emit!(op_28, "28");
        $emit!(op_29, "29");
        $emit!(op_2a, "2A");
        $emit!(op_2b, "2B");
        $emit!(op_2c, "2C");
        $emit!(op_2d, "2D");
        $emit!(op_2f, "2F");
        $emit!(op_30, "30");
        $emit!(op_31, "31");
        $emit!(op_32, "32");
        $emit!(op_33, "33");
        $emit!(op_34, "34");
        $emit!(op_35, "35");
        $emit!(op_37, "37");
        $emit!(op_38, "38");
        $emit!(op_39, "39");
        $emit!(op_3a, "3A");
        $emit!(op_3b, "3B");
        $emit!(op_3c, "3C");
        $emit!(op_3d, "3D");
        $emit!(op_3f, "3F");
        $emit!(op_40, "40");
        $emit!(op_41, "41");
        $emit!(op_42, "42");
        $emit!(op_43, "43");
        $emit!(op_44, "44");
        $emit!(op_45, "45");
        $emit!(op_46, "46");
        $emit!(op_47, "47");
        $emit!(op_48, "48");
        $emit!(op_49, "49");
        $emit!(op_4a, "4A");
        $emit!(op_4b, "4B");
        $emit!(op_4c, "4C");
        $emit!(op_4d, "4D");
        $emit!(op_4e, "4E");
        $emit!(op_4f, "4F");
        $emit!(op_50, "50");
        $emit!(op_51, "51");
        $emit!(op_52, "52");
        $emit!(op_53, "53");
        $emit!(op_54, "54");
        $emit!(op_55, "55");
        $emit!(op_56, "56");
        $emit!(op_57, "57");
        $emit!(op_58, "58");
        $emit!(op_59, "59");
        $emit!(op_5a, "5A");
        $emit!(op_5b, "5B");
        $emit!(op_5c, "5C");
        $emit!(op_5d, "5D");
        $emit!(op_5e, "5E");
        $emit!(op_5f, "5F");
        $emit!(op_60, "60");
        $emit!(op_61, "61");
        $emit!(op_62, "62");
        $emit!(op_63, "63");
        $emit!(op_66, "66");
        $emit!(op_67, "67");
        $emit!(op_68, "68");
        $emit!(op_69, "69");
        $emit!(op_6a, "6A");
        $emit!(op_6b, "6B");
        $emit!(op_6c, "6C");
        $emit!(op_6d, "6D");
        $emit!(op_6e, "6E");
        $emit!(op_6f, "6F");
        $emit!(op_70, "70");
        $emit!(op_71, "71");
        $emit!(op_72, "72");
        $emit!(op_73, "73");
        $emit!(op_74, "74");
        $emit!(op_75, "75");
        $emit!(op_76, "76");
        $emit!(op_77, "77");
        $emit!(op_78, "78");
        $emit!(op_79, "79");
        $emit!(op_7a, "7A");
        $emit!(op_7b, "7B");
        $emit!(op_7c, "7C");
        $emit!(op_7d, "7D");
        $emit!(op_7e, "7E");
        $emit!(op_7f, "7F");
        $emit!(op_80_0, "80.0");
        $emit!(op_80_1, "80.1");
        $emit!(op_80_2, "80.2");
        $emit!(op_80_3, "80.3");
        $emit!(op_80_4, "80.4");
        $emit!(op_80_5, "80.5");
        $emit!(op_80_6, "80.6");
        $emit!(op_80_7, "80.7");
        $emit!(op_81_0, "81.0");
        $emit!(op_81_1, "81.1");
        $emit!(op_81_2, "81.2");
        $emit!(op_81_3, "81.3");
        $emit!(op_81_4, "81.4");
        $emit!(op_81_5, "81.5");
        $emit!(op_81_6, "81.6");
        $emit!(op_81_7, "81.7");
        $emit!(op_82_0, "82.0");
        $emit!(op_82_1, "82.1");
        $emit!(op_82_2, "82.2");
        $emit!(op_82_3, "82.3");
        $emit!(op_82_4, "82.4");
        $emit!(op_82_5, "82.5");
        $emit!(op_82_6, "82.6");
        $emit!(op_82_7, "82.7");
        $emit!(op_83_0, "83.0");
        $emit!(op_83_1, "83.1");
        $emit!(op_83_2, "83.2");
        $emit!(op_83_3, "83.3");
        $emit!(op_83_4, "83.4");
        $emit!(op_83_5, "83.5");
        $emit!(op_83_6, "83.6");
        $emit!(op_83_7, "83.7");
        $emit!(op_84, "84");
        $emit!(op_85, "85");
        $emit!(op_86, "86");
        $emit!(op_87, "87");
        $emit!(op_88, "88");
        $emit!(op_89, "89");
        $emit!(op_8a, "8A");
        $emit!(op_8b, "8B");
        $emit!(op_8c, "8C");
        $emit!(op_8d, "8D");
        $emit!(op_8e, "8E");
        $emit!(op_8f, "8F");
        $emit!(op_90, "90");
        $emit!(op_91, "91");
        $emit!(op_92, "92");
        $emit!(op_93, "93");
        $emit!(op_94, "94");
        $emit!(op_95, "95");
        $emit!(op_96, "96");
        $emit!(op_97, "97");
        $emit!(op_98, "98");
        $emit!(op_99, "99");
        $emit!(op_9a, "9A");
        $emit!(op_9c, "9C");
        $emit!(op_9d, "9D");
        $emit!(op_9e, "9E");
        $emit!(op_9f, "9F");
        $emit!(op_a0, "A0");
        $emit!(op_a1, "A1");
        $emit!(op_a2, "A2");
        $emit!(op_a3, "A3");
        $emit!(op_a4, "A4");
        $emit!(op_a5, "A5");
        $emit!(op_a6, "A6");
        $emit!(op_a7, "A7");
        $emit!(op_a8, "A8");
        $emit!(op_a9, "A9");
        $emit!(op_aa, "AA");
        $emit!(op_ab, "AB");
        $emit!(op_ac, "AC");
        $emit!(op_ad, "AD");
        $emit!(op_ae, "AE");
        $emit!(op_af, "AF");
        $emit!(op_b0, "B0");
        $emit!(op_b1, "B1");
        $emit!(op_b2, "B2");
        $emit!(op_b3, "B3");
        $emit!(op_b4, "B4");
        $emit!(op_b5, "B5");
        $emit!(op_b6, "B6");
        $emit!(op_b7, "B7");
        $emit!(op_b8, "B8");
        $emit!(op_b9, "B9");
        $emit!(op_ba, "BA");
        $emit!(op_bb, "BB");
        $emit!(op_bc, "BC");
        $emit!(op_bd, "BD");
        $emit!(op_be, "BE");
        $emit!(op_bf, "BF");
        $emit!(op_c0_0, "C0.0");
        $emit!(op_c0_1, "C0.1");
        $emit!(op_c0_2, "C0.2");
        $emit!(op_c0_3, "C0.3");
        $emit!(op_c0_4, "C0.4");
        $emit!(op_c0_5, "C0.5");
        $emit!(op_c0_6, "C0.6");
        $emit!(op_c0_7, "C0.7");
        $emit!(op_c1_0, "C1.0");
        $emit!(op_c1_1, "C1.1");
        $emit!(op_c1_2, "C1.2");
        $emit!(op_c1_3, "C1.3");
        $emit!(op_c1_4, "C1.4");
        $emit!(op_c1_5, "C1.5");
        $emit!(op_c1_6, "C1.6");
        $emit!(op_c1_7, "C1.7");
        $emit!(op_c2, "C2");
        $emit!(op_c3, "C3");
        $emit!(op_c4, "C4");
        $emit!(op_c5, "C5");
        $emit!(op_c6, "C6");
        $emit!(op_c7, "C7");
        $emit!(op_c8, "C8");
        $emit!(op_c9, "C9");
        $emit!(op_ca, "CA");
        $emit!(op_cb, "CB");
        $emit!(op_cc, "CC");
        $emit!(op_cd, "CD");
        $emit!(op_ce, "CE");
        $emit!(op_cf, "CF");
        $emit!(op_d0_0, "D0.0");
        $emit!(op_d0_1, "D0.1");
        $emit!(op_d0_2, "D0.2");
        $emit!(op_d0_3, "D0.3");
        $emit!(op_d0_4, "D0.4");
        $emit!(op_d0_5, "D0.5");
        $emit!(op_d0_6, "D0.6");
        $emit!(op_d0_7, "D0.7");
        $emit!(op_d1_0, "D1.0");
        $emit!(op_d1_1, "D1.1");
        $emit!(op_d1_2, "D1.2");
        $emit!(op_d1_3, "D1.3");
        $emit!(op_d1_4, "D1.4");
        $emit!(op_d1_5, "D1.5");
        $emit!(op_d1_6, "D1.6");
        $emit!(op_d1_7, "D1.7");
        $emit!(op_d2_0, "D2.0");
        $emit!(op_d2_1, "D2.1");
        $emit!(op_d2_2, "D2.2");
        $emit!(op_d2_3, "D2.3");
        $emit!(op_d2_4, "D2.4");
        $emit!(op_d2_5, "D2.5");
        $emit!(op_d2_6, "D2.6");
        $emit!(op_d2_7, "D2.7");
        $emit!(op_d3_0, "D3.0");
        $emit!(op_d3_1, "D3.1");
        $emit!(op_d3_2, "D3.2");
        $emit!(op_d3_3, "D3.3");
        $emit!(op_d3_4, "D3.4");
        $emit!(op_d3_5, "D3.5");
        $emit!(op_d3_6, "D3.6");
        $emit!(op_d3_7, "D3.7");
        $emit!(op_d4, "D4");
        $emit!(op_d5, "D5");
        $emit!(op_d6, "D6");
        $emit!(op_d7, "D7");
        $emit!(op_d8, "D8");
        $emit!(op_d9, "D9");
        $emit!(op_da, "DA");
        $emit!(op_db, "DB");
        $emit!(op_dc, "DC");
        $emit!(op_dd, "DD");
        $emit!(op_de, "DE");
        $emit!(op_df, "DF");
        $emit!(op_e0, "E0");
        $emit!(op_e1, "E1");
        $emit!(op_e2, "E2");
        $emit!(op_e3, "E3");
        $emit!(op_e4, "E4");
        $emit!(op_e5, "E5");
        $emit!(op_e6, "E6");
        $emit!(op_e7, "E7");
        $emit!(op_e8, "E8");
        $emit!(op_e9, "E9");
        $emit!(op_ea, "EA");
        $emit!(op_eb, "EB");
        $emit!(op_ec, "EC");
        $emit!(op_ed, "ED");
        $emit!(op_ee, "EE");
        $emit!(op_ef, "EF");
        $emit!(op_f5, "F5");
        $emit!(op_f6_0, "F6.0");
        $emit!(op_f6_1, "F6.1");
        $emit!(op_f6_2, "F6.2");
        $emit!(op_f6_3, "F6.3");
        $emit!(op_f6_4, "F6.4");
        $emit!(op_f6_5, "F6.5");
        $emit!(op_f6_6, "F6.6");
        $emit!(op_f6_7, "F6.7");
        $emit!(op_f7_0, "F7.0");
        $emit!(op_f7_1, "F7.1");
        $emit!(op_f7_2, "F7.2");
        $emit!(op_f7_3, "F7.3");
        $emit!(op_f7_4, "F7.4");
        $emit!(op_f7_5, "F7.5");
        $emit!(op_f7_6, "F7.6");
        $emit!(op_f7_7, "F7.7");
        $emit!(op_f8, "F8");
        $emit!(op_f9, "F9");
        $emit!(op_fa, "FA");
        $emit!(op_fb, "FB");
        $emit!(op_fc, "FC");
        $emit!(op_fd, "FD");
        $emit!(op_fe_0, "FE.0");
        $emit!(op_fe_1, "FE.1");
        $emit!(op_ff_0, "FF.0");
        $emit!(op_ff_1, "FF.1");
        $emit!(op_ff_2, "FF.2");
        $emit!(op_ff_3, "FF.3");
        $emit!(op_ff_4, "FF.4");
        $emit!(op_ff_5, "FF.5");
        $emit!(op_ff_6, "FF.6");
        $emit!(op_ff_7, "FF.7");
    };
}

mod v20 {
    use super::run_test_file;

    macro_rules! emit {
        ($name:ident, $file:expr) => {
            #[test]
            fn $name() {
                run_test_file::<{ cpu::V20_BUS }>($file);
            }
        };
    }

    tests_for_all_opcodes!(emit);
}

mod v30 {
    use super::run_test_file;

    macro_rules! emit {
        ($name:ident, $file:expr) => {
            #[test]
            fn $name() {
                run_test_file::<{ cpu::V30_BUS }>($file);
            }
        };
    }

    tests_for_all_opcodes!(emit);
}
