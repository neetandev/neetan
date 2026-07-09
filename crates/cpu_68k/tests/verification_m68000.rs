#![cfg(feature = "verification")]

use std::{collections::HashMap, fmt::Write, fs, path::Path, sync::LazyLock};

use common::{Bus, CpuM68000};
use cpu_68k::{M68000, M68000BusCycle, M68000BusDirection, M68000BusSize, M68000State};
use zlib_rs::{InflateConfig, ReturnCode, decompress_slice};

const REG_ORDER: [&str; 19] = [
    "d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7", "a0", "a1", "a2", "a3", "a4", "a5", "a6",
    "usp", "ssp", "pc", "sr",
];

#[derive(Debug, Default, Clone)]
struct M68000MooState {
    regs: HashMap<String, u32>,
    ram: Vec<(u32, u8)>,
    pref: Option<(u16, u16)>,
}

#[derive(Debug, Clone)]
struct M68000MooTest {
    idx: u32,
    name: String,
    bytes: Vec<u8>,
    initial: M68000MooState,
    final_state: M68000MooState,
    cycles: Vec<M68000BusCycle>,
    total_cycles: u32,
    hash: Option<String>,
}

struct TestBus {
    ram: HashMap<u32, u8>,
    current_cycle: u64,
}

impl TestBus {
    fn new() -> Self {
        Self {
            ram: HashMap::new(),
            current_cycle: 0,
        }
    }
}

impl Bus for TestBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        self.ram.get(&(address & 0x00FF_FFFF)).copied().unwrap_or(0)
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.ram.insert(address & 0x00FF_FFFF, value);
    }

    fn io_read_byte(&mut self, port: u16) -> u8 {
        panic!("unexpected m68000 I/O read from 0x{port:04X}");
    }

    fn io_write_byte(&mut self, port: u16, value: u8) {
        panic!("unexpected m68000 I/O write to 0x{port:04X} = 0x{value:02X}");
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
        0
    }
}

fn test_dir() -> &'static Path {
    static DIR: LazyLock<std::path::PathBuf> = LazyLock::new(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/SingleStepTests/m68000/v1")
    });
    &DIR
}

fn read_u16(bytes: &[u8], offset: &mut usize) -> u16 {
    let end = *offset + 2;
    let value = u16::from_le_bytes(bytes[*offset..end].try_into().unwrap());
    *offset = end;
    value
}

fn read_u32(bytes: &[u8], offset: &mut usize) -> u32 {
    let end = *offset + 4;
    let value = u32::from_le_bytes(bytes[*offset..end].try_into().unwrap());
    *offset = end;
    value
}

fn read_tag(bytes: &[u8], offset: &mut usize) -> [u8; 4] {
    let end = *offset + 4;
    let value = bytes[*offset..end].try_into().unwrap();
    *offset = end;
    value
}

fn read_gzip_to_vec(path: &Path) -> Vec<u8> {
    let compressed = fs::read(path).unwrap();
    let isize_le: [u8; 4] = compressed[compressed.len() - 4..].try_into().unwrap();
    let output_len = u32::from_le_bytes(isize_le) as usize;
    let mut output = vec![0u8; output_len];
    let (decompressed, code) =
        decompress_slice(&mut output, &compressed, InflateConfig { window_bits: 31 });
    assert_eq!(code, ReturnCode::Ok, "{path:?}: inflate failed ({code:?})");
    assert_eq!(decompressed.len(), output_len);
    output
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn parse_regs(payload: &[u8]) -> HashMap<String, u32> {
    let mut offset = 0;
    let mask = read_u32(payload, &mut offset);
    let mut regs = HashMap::new();
    for (index, name) in REG_ORDER.iter().enumerate() {
        if mask & (1 << index) != 0 {
            regs.insert((*name).to_string(), read_u32(payload, &mut offset));
        }
    }
    regs
}

fn parse_ram(payload: &[u8]) -> Vec<(u32, u8)> {
    let mut offset = 0;
    let count = read_u32(payload, &mut offset) as usize;
    let mut ram = Vec::with_capacity(count);
    for _ in 0..count {
        let address = read_u32(payload, &mut offset) & 0x00FF_FFFF;
        let value = payload[offset];
        offset += 1;
        ram.push((address, value));
    }
    ram
}

fn parse_state(payload: &[u8]) -> M68000MooState {
    let mut offset = 0;
    let mut state = M68000MooState::default();
    while offset < payload.len() {
        let tag = read_tag(payload, &mut offset);
        let length = read_u32(payload, &mut offset) as usize;
        let end = offset + length;
        let sub_payload = &payload[offset..end];
        match &tag {
            b"REGS" => state.regs = parse_regs(sub_payload),
            b"RAM " => state.ram = parse_ram(sub_payload),
            b"PREF" => {
                let mut pref_offset = 0;
                state.pref = Some((
                    read_u16(sub_payload, &mut pref_offset),
                    read_u16(sub_payload, &mut pref_offset),
                ));
            }
            _ => {}
        }
        offset = end;
    }
    state
}

fn parse_cycles(payload: &[u8]) -> (u32, Vec<M68000BusCycle>) {
    let mut offset = 0;
    let total_cycles = read_u32(payload, &mut offset);
    let count = read_u32(payload, &mut offset) as usize;
    let mut cycles = Vec::with_capacity(count);
    for _ in 0..count {
        let cycle = read_u32(payload, &mut offset);
        let flags = payload[offset];
        offset += 1;
        let address = read_u32(payload, &mut offset) & 0x00FF_FFFF;
        let data = read_u16(payload, &mut offset);
        let function_code = payload[offset];
        offset += 1;
        let status = read_tag(payload, &mut offset);
        cycles.push(M68000BusCycle {
            cycle,
            direction: if flags & 1 != 0 {
                M68000BusDirection::Write
            } else {
                M68000BusDirection::Read
            },
            size: if flags & 2 != 0 {
                M68000BusSize::Word
            } else {
                M68000BusSize::Byte
            },
            address,
            data,
            function_code,
            status,
        });
    }
    (total_cycles, cycles)
}

fn parse_test(payload: &[u8]) -> M68000MooTest {
    let mut offset = 0;
    let idx = read_u32(payload, &mut offset);
    let mut test = M68000MooTest {
        idx,
        name: String::new(),
        bytes: Vec::new(),
        initial: M68000MooState::default(),
        final_state: M68000MooState::default(),
        cycles: Vec::new(),
        total_cycles: 0,
        hash: None,
    };
    while offset < payload.len() {
        let tag = read_tag(payload, &mut offset);
        let length = read_u32(payload, &mut offset) as usize;
        let end = offset + length;
        let sub_payload = &payload[offset..end];
        match &tag {
            b"NAME" => {
                let mut name_offset = 0;
                let name_len = read_u32(sub_payload, &mut name_offset) as usize;
                test.name =
                    String::from_utf8(sub_payload[name_offset..name_offset + name_len].to_vec())
                        .unwrap();
            }
            b"BYTS" => {
                let mut bytes_offset = 0;
                let byte_count = read_u32(sub_payload, &mut bytes_offset) as usize;
                test.bytes = sub_payload[bytes_offset..bytes_offset + byte_count].to_vec();
            }
            b"INIT" => test.initial = parse_state(sub_payload),
            b"FINA" => test.final_state = parse_state(sub_payload),
            b"CYCL" => {
                let (total_cycles, cycles) = parse_cycles(sub_payload);
                test.total_cycles = total_cycles;
                test.cycles = cycles;
            }
            b"HASH" => test.hash = Some(bytes_to_hex(sub_payload)),
            _ => {}
        }
        offset = end;
    }
    test
}

fn load_tests(path: &Path) -> Vec<M68000MooTest> {
    let data = read_gzip_to_vec(path);
    let mut offset = 0;
    assert_eq!(&data[0..4], b"MOO ");
    offset += 4;
    let header_len = read_u32(&data, &mut offset) as usize;
    offset += header_len;
    let mut tests = Vec::new();
    while offset < data.len() {
        let tag = read_tag(&data, &mut offset);
        let chunk_len = read_u32(&data, &mut offset) as usize;
        let end = offset + chunk_len;
        if &tag == b"TEST" {
            tests.push(parse_test(&data[offset..end]));
        }
        offset = end;
    }
    tests
}

fn resolve_regs(
    initial: &HashMap<String, u32>,
    final_state: &HashMap<String, u32>,
) -> HashMap<String, u32> {
    REG_ORDER
        .iter()
        .map(|name| {
            let value = final_state
                .get(*name)
                .or_else(|| initial.get(*name))
                .copied()
                .unwrap_or_else(|| panic!("missing register: {name}"));
            ((*name).to_string(), value)
        })
        .collect()
}

fn build_state(regs: &HashMap<String, u32>, pref: (u16, u16)) -> M68000State {
    let get = |name: &str| -> u32 {
        regs.get(name)
            .copied()
            .unwrap_or_else(|| panic!("missing register: {name}"))
    };
    let mut data = [0; 8];
    let mut address = [0; 7];
    for (index, value) in data.iter_mut().enumerate() {
        *value = get(&format!("d{index}"));
    }
    for (index, value) in address.iter_mut().enumerate() {
        *value = get(&format!("a{index}"));
    }
    M68000State {
        data,
        address,
        usp: get("usp"),
        ssp: get("ssp"),
        pc: get("pc") & 0x00FF_FFFF,
        sr: get("sr") as u16,
        ir: pref.0,
        irc: pref.1,
    }
}

fn push_state_diffs(
    initial: M68000State,
    actual: M68000State,
    expected: M68000State,
    diffs: &mut Vec<String>,
) {
    for index in 0..8 {
        if actual.data[index] != expected.data[index] {
            diffs.push(format!(
                "  d{index}: expected 0x{:08X}, got 0x{:08X} (was 0x{:08X})",
                expected.data[index], actual.data[index], initial.data[index]
            ));
        }
    }
    for index in 0..7 {
        if actual.address[index] != expected.address[index] {
            diffs.push(format!(
                "  a{index}: expected 0x{:08X}, got 0x{:08X} (was 0x{:08X})",
                expected.address[index], actual.address[index], initial.address[index]
            ));
        }
    }
    let checks = [
        ("usp", initial.usp, actual.usp, expected.usp),
        ("ssp", initial.ssp, actual.ssp, expected.ssp),
        ("pc", initial.pc, actual.pc, expected.pc),
        (
            "sr",
            u32::from(initial.sr),
            u32::from(actual.sr),
            u32::from(expected.sr),
        ),
        (
            "ir",
            u32::from(initial.ir),
            u32::from(actual.ir),
            u32::from(expected.ir),
        ),
        (
            "irc",
            u32::from(initial.irc),
            u32::from(actual.irc),
            u32::from(expected.irc),
        ),
    ];
    for (name, initial_value, actual_value, expected_value) in checks {
        if actual_value != expected_value {
            diffs.push(format!("  {name}: expected 0x{expected_value:08X}, got 0x{actual_value:08X} (was 0x{initial_value:08X})"));
        }
    }
}

fn run_test_file(stem: &str) {
    let path = test_dir().join(format!("{stem}.moo.gz"));
    let test_cases = load_tests(&path);
    let mut failures = Vec::new();
    for (index, test) in test_cases.iter().enumerate() {
        let mut bus = TestBus::new();
        for &(address, value) in &test.initial.ram {
            bus.ram.insert(address, value);
        }
        let initial_regs = resolve_regs(&test.initial.regs, &HashMap::new());
        let final_regs = resolve_regs(&test.initial.regs, &test.final_state.regs);
        let initial_state = build_state(&initial_regs, test.initial.pref.unwrap());
        let expected_state = build_state(&final_regs, test.final_state.pref.unwrap());
        let mut cpu = M68000::new(10_000_000);
        cpu.load_state(initial_state);
        let actual_cycles = cpu.step(&mut bus) as u32;
        let actual_state = cpu.save_state();
        let mut diffs = Vec::new();
        push_state_diffs(initial_state, actual_state, expected_state, &mut diffs);
        if actual_cycles != test.total_cycles {
            diffs.push(format!(
                "  cycles: expected {}, got {actual_cycles}",
                test.total_cycles
            ));
        }
        let actual_bus_cycles = cpu.bus_cycles();
        if actual_bus_cycles != test.cycles.as_slice() {
            diffs.push(format!(
                "  bus cycles: expected {} events, got {} events",
                test.cycles.len(),
                actual_bus_cycles.len()
            ));
            for (event_index, (expected, actual)) in
                test.cycles.iter().zip(actual_bus_cycles.iter()).enumerate()
            {
                if expected != actual {
                    diffs.push(format!(
                        "    event {event_index}: expected {expected:?}, got {actual:?}"
                    ));
                    break;
                }
            }
        }
        for &(address, expected_value) in &test.final_state.ram {
            let actual_value = bus.ram.get(&address).copied().unwrap_or(0);
            if actual_value != expected_value {
                let initial_value = test
                    .initial
                    .ram
                    .iter()
                    .find(|(candidate, _)| *candidate == address)
                    .map(|(_, value)| *value);
                match initial_value {
                    Some(before) => diffs.push(format!("  ram[0x{address:06X}]: expected 0x{expected_value:02X}, got 0x{actual_value:02X} (was 0x{before:02X})")),
                    None => diffs.push(format!("  ram[0x{address:06X}]: expected 0x{expected_value:02X}, got 0x{actual_value:02X} (not in initial RAM)")),
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
                "[{} #{index} idx={}] {} ({})\n{}",
                path.file_name().unwrap().to_string_lossy(),
                test.idx,
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

macro_rules! test_opcode {
    ($name:ident, $stem:expr) => {
        #[test]
        fn $name() {
            run_test_file($stem);
        }
    };
}

test_opcode!(op_0000, "0000");
test_opcode!(op_0008, "0008");
test_opcode!(op_0010, "0010");
test_opcode!(op_0018, "0018");
test_opcode!(op_0020, "0020");
test_opcode!(op_0028, "0028");
test_opcode!(op_0030, "0030");
test_opcode!(op_0038, "0038");
test_opcode!(op_0039, "0039");
test_opcode!(op_003c, "003c");
test_opcode!(op_0040, "0040");
test_opcode!(op_0050, "0050");
test_opcode!(op_0058, "0058");
test_opcode!(op_0060, "0060");
test_opcode!(op_0068, "0068");
test_opcode!(op_0070, "0070");
test_opcode!(op_0078, "0078");
test_opcode!(op_0079, "0079");
test_opcode!(op_007c, "007c");
test_opcode!(op_0080, "0080");
test_opcode!(op_0090, "0090");
test_opcode!(op_0098, "0098");
test_opcode!(op_00a0, "00a0");
test_opcode!(op_00a8, "00a8");
test_opcode!(op_00b0, "00b0");
test_opcode!(op_00b8, "00b8");
test_opcode!(op_00b9, "00b9");
test_opcode!(op_0100, "0100");
test_opcode!(op_0108, "0108");
test_opcode!(op_0110, "0110");
test_opcode!(op_0118, "0118");
test_opcode!(op_0120, "0120");
test_opcode!(op_0128, "0128");
test_opcode!(op_0130, "0130");
test_opcode!(op_0138, "0138");
test_opcode!(op_0139, "0139");
test_opcode!(op_013a, "013a");
test_opcode!(op_013b, "013b");
test_opcode!(op_013c, "013c");
test_opcode!(op_0140, "0140");
test_opcode!(op_0148, "0148");
test_opcode!(op_0150, "0150");
test_opcode!(op_0158, "0158");
test_opcode!(op_0160, "0160");
test_opcode!(op_0168, "0168");
test_opcode!(op_0170, "0170");
test_opcode!(op_0178, "0178");
test_opcode!(op_0179, "0179");
test_opcode!(op_0180, "0180");
test_opcode!(op_0188, "0188");
test_opcode!(op_0190, "0190");
test_opcode!(op_0198, "0198");
test_opcode!(op_01a0, "01a0");
test_opcode!(op_01a8, "01a8");
test_opcode!(op_01b0, "01b0");
test_opcode!(op_01b8, "01b8");
test_opcode!(op_01b9, "01b9");
test_opcode!(op_01c0, "01c0");
test_opcode!(op_01c8, "01c8");
test_opcode!(op_01d0, "01d0");
test_opcode!(op_01d8, "01d8");
test_opcode!(op_01e0, "01e0");
test_opcode!(op_01e8, "01e8");
test_opcode!(op_01f0, "01f0");
test_opcode!(op_01f8, "01f8");
test_opcode!(op_01f9, "01f9");
test_opcode!(op_0200, "0200");
test_opcode!(op_0210, "0210");
test_opcode!(op_0218, "0218");
test_opcode!(op_0220, "0220");
test_opcode!(op_0228, "0228");
test_opcode!(op_0230, "0230");
test_opcode!(op_0238, "0238");
test_opcode!(op_0239, "0239");
test_opcode!(op_023c, "023c");
test_opcode!(op_0240, "0240");
test_opcode!(op_0250, "0250");
test_opcode!(op_0258, "0258");
test_opcode!(op_0260, "0260");
test_opcode!(op_0268, "0268");
test_opcode!(op_0270, "0270");
test_opcode!(op_0278, "0278");
test_opcode!(op_0279, "0279");
test_opcode!(op_027c, "027c");
test_opcode!(op_0280, "0280");
test_opcode!(op_0290, "0290");
test_opcode!(op_0298, "0298");
test_opcode!(op_02a0, "02a0");
test_opcode!(op_02a8, "02a8");
test_opcode!(op_02b0, "02b0");
test_opcode!(op_02b8, "02b8");
test_opcode!(op_02b9, "02b9");
test_opcode!(op_0400, "0400");
test_opcode!(op_0410, "0410");
test_opcode!(op_0418, "0418");
test_opcode!(op_0420, "0420");
test_opcode!(op_0428, "0428");
test_opcode!(op_0430, "0430");
test_opcode!(op_0438, "0438");
test_opcode!(op_0439, "0439");
test_opcode!(op_0440, "0440");
test_opcode!(op_0450, "0450");
test_opcode!(op_0458, "0458");
test_opcode!(op_0460, "0460");
test_opcode!(op_0468, "0468");
test_opcode!(op_0470, "0470");
test_opcode!(op_0478, "0478");
test_opcode!(op_0479, "0479");
test_opcode!(op_0480, "0480");
test_opcode!(op_0490, "0490");
test_opcode!(op_0498, "0498");
test_opcode!(op_04a0, "04a0");
test_opcode!(op_04a8, "04a8");
test_opcode!(op_04b0, "04b0");
test_opcode!(op_04b8, "04b8");
test_opcode!(op_04b9, "04b9");
test_opcode!(op_0600, "0600");
test_opcode!(op_0610, "0610");
test_opcode!(op_0618, "0618");
test_opcode!(op_0620, "0620");
test_opcode!(op_0628, "0628");
test_opcode!(op_0630, "0630");
test_opcode!(op_0638, "0638");
test_opcode!(op_0639, "0639");
test_opcode!(op_0640, "0640");
test_opcode!(op_0650, "0650");
test_opcode!(op_0658, "0658");
test_opcode!(op_0660, "0660");
test_opcode!(op_0668, "0668");
test_opcode!(op_0670, "0670");
test_opcode!(op_0678, "0678");
test_opcode!(op_0679, "0679");
test_opcode!(op_0680, "0680");
test_opcode!(op_0690, "0690");
test_opcode!(op_0698, "0698");
test_opcode!(op_06a0, "06a0");
test_opcode!(op_06a8, "06a8");
test_opcode!(op_06b0, "06b0");
test_opcode!(op_06b8, "06b8");
test_opcode!(op_06b9, "06b9");
test_opcode!(op_0800, "0800");
test_opcode!(op_0810, "0810");
test_opcode!(op_0818, "0818");
test_opcode!(op_0820, "0820");
test_opcode!(op_0828, "0828");
test_opcode!(op_0830, "0830");
test_opcode!(op_0838, "0838");
test_opcode!(op_0839, "0839");
test_opcode!(op_083a, "083a");
test_opcode!(op_083b, "083b");
test_opcode!(op_0840, "0840");
test_opcode!(op_0850, "0850");
test_opcode!(op_0858, "0858");
test_opcode!(op_0860, "0860");
test_opcode!(op_0868, "0868");
test_opcode!(op_0870, "0870");
test_opcode!(op_0878, "0878");
test_opcode!(op_0879, "0879");
test_opcode!(op_0880, "0880");
test_opcode!(op_0890, "0890");
test_opcode!(op_0898, "0898");
test_opcode!(op_08a0, "08a0");
test_opcode!(op_08a8, "08a8");
test_opcode!(op_08b0, "08b0");
test_opcode!(op_08b8, "08b8");
test_opcode!(op_08b9, "08b9");
test_opcode!(op_08c0, "08c0");
test_opcode!(op_08d0, "08d0");
test_opcode!(op_08d8, "08d8");
test_opcode!(op_08e0, "08e0");
test_opcode!(op_08e8, "08e8");
test_opcode!(op_08f0, "08f0");
test_opcode!(op_08f8, "08f8");
test_opcode!(op_08f9, "08f9");
test_opcode!(op_0a00, "0a00");
test_opcode!(op_0a10, "0a10");
test_opcode!(op_0a18, "0a18");
test_opcode!(op_0a20, "0a20");
test_opcode!(op_0a28, "0a28");
test_opcode!(op_0a30, "0a30");
test_opcode!(op_0a38, "0a38");
test_opcode!(op_0a39, "0a39");
test_opcode!(op_0a3c, "0a3c");
test_opcode!(op_0a40, "0a40");
test_opcode!(op_0a50, "0a50");
test_opcode!(op_0a58, "0a58");
test_opcode!(op_0a60, "0a60");
test_opcode!(op_0a68, "0a68");
test_opcode!(op_0a70, "0a70");
test_opcode!(op_0a78, "0a78");
test_opcode!(op_0a79, "0a79");
test_opcode!(op_0a7c, "0a7c");
test_opcode!(op_0a80, "0a80");
test_opcode!(op_0a90, "0a90");
test_opcode!(op_0a98, "0a98");
test_opcode!(op_0aa0, "0aa0");
test_opcode!(op_0aa8, "0aa8");
test_opcode!(op_0ab0, "0ab0");
test_opcode!(op_0ab8, "0ab8");
test_opcode!(op_0ab9, "0ab9");
test_opcode!(op_0c00, "0c00");
test_opcode!(op_0c10, "0c10");
test_opcode!(op_0c18, "0c18");
test_opcode!(op_0c20, "0c20");
test_opcode!(op_0c28, "0c28");
test_opcode!(op_0c30, "0c30");
test_opcode!(op_0c38, "0c38");
test_opcode!(op_0c39, "0c39");
test_opcode!(op_0c40, "0c40");
test_opcode!(op_0c50, "0c50");
test_opcode!(op_0c58, "0c58");
test_opcode!(op_0c60, "0c60");
test_opcode!(op_0c68, "0c68");
test_opcode!(op_0c70, "0c70");
test_opcode!(op_0c78, "0c78");
test_opcode!(op_0c79, "0c79");
test_opcode!(op_0c80, "0c80");
test_opcode!(op_0c90, "0c90");
test_opcode!(op_0c98, "0c98");
test_opcode!(op_0ca0, "0ca0");
test_opcode!(op_0ca8, "0ca8");
test_opcode!(op_0cb0, "0cb0");
test_opcode!(op_0cb8, "0cb8");
test_opcode!(op_0cb9, "0cb9");
test_opcode!(op_1000, "1000");
test_opcode!(op_1010, "1010");
test_opcode!(op_1018, "1018");
test_opcode!(op_1020, "1020");
test_opcode!(op_1028, "1028");
test_opcode!(op_1030, "1030");
test_opcode!(op_1038, "1038");
test_opcode!(op_1039, "1039");
test_opcode!(op_103a, "103a");
test_opcode!(op_103b, "103b");
test_opcode!(op_103c, "103c");
test_opcode!(op_1080, "1080");
test_opcode!(op_1090, "1090");
test_opcode!(op_1098, "1098");
test_opcode!(op_10a0, "10a0");
test_opcode!(op_10a8, "10a8");
test_opcode!(op_10b0, "10b0");
test_opcode!(op_10b8, "10b8");
test_opcode!(op_10b9, "10b9");
test_opcode!(op_10ba, "10ba");
test_opcode!(op_10bb, "10bb");
test_opcode!(op_10bc, "10bc");
test_opcode!(op_10c0, "10c0");
test_opcode!(op_10d0, "10d0");
test_opcode!(op_10d8, "10d8");
test_opcode!(op_10e0, "10e0");
test_opcode!(op_10e8, "10e8");
test_opcode!(op_10f0, "10f0");
test_opcode!(op_10f8, "10f8");
test_opcode!(op_10f9, "10f9");
test_opcode!(op_10fa, "10fa");
test_opcode!(op_10fb, "10fb");
test_opcode!(op_10fc, "10fc");
test_opcode!(op_1100, "1100");
test_opcode!(op_1110, "1110");
test_opcode!(op_1118, "1118");
test_opcode!(op_1120, "1120");
test_opcode!(op_1128, "1128");
test_opcode!(op_1130, "1130");
test_opcode!(op_1138, "1138");
test_opcode!(op_1139, "1139");
test_opcode!(op_113a, "113a");
test_opcode!(op_113b, "113b");
test_opcode!(op_113c, "113c");
test_opcode!(op_1140, "1140");
test_opcode!(op_1150, "1150");
test_opcode!(op_1158, "1158");
test_opcode!(op_1160, "1160");
test_opcode!(op_1168, "1168");
test_opcode!(op_1170, "1170");
test_opcode!(op_1178, "1178");
test_opcode!(op_1179, "1179");
test_opcode!(op_117a, "117a");
test_opcode!(op_117b, "117b");
test_opcode!(op_117c, "117c");
test_opcode!(op_1180, "1180");
test_opcode!(op_1190, "1190");
test_opcode!(op_1198, "1198");
test_opcode!(op_11a0, "11a0");
test_opcode!(op_11a8, "11a8");
test_opcode!(op_11b0, "11b0");
test_opcode!(op_11b8, "11b8");
test_opcode!(op_11b9, "11b9");
test_opcode!(op_11ba, "11ba");
test_opcode!(op_11bb, "11bb");
test_opcode!(op_11bc, "11bc");
test_opcode!(op_11c0, "11c0");
test_opcode!(op_11d0, "11d0");
test_opcode!(op_11d8, "11d8");
test_opcode!(op_11e0, "11e0");
test_opcode!(op_11e8, "11e8");
test_opcode!(op_11f0, "11f0");
test_opcode!(op_11f8, "11f8");
test_opcode!(op_11f9, "11f9");
test_opcode!(op_11fa, "11fa");
test_opcode!(op_11fb, "11fb");
test_opcode!(op_11fc, "11fc");
test_opcode!(op_13c0, "13c0");
test_opcode!(op_13d0, "13d0");
test_opcode!(op_13d8, "13d8");
test_opcode!(op_13e0, "13e0");
test_opcode!(op_13e8, "13e8");
test_opcode!(op_13f0, "13f0");
test_opcode!(op_13f8, "13f8");
test_opcode!(op_13f9, "13f9");
test_opcode!(op_13fa, "13fa");
test_opcode!(op_13fb, "13fb");
test_opcode!(op_13fc, "13fc");
test_opcode!(op_2000, "2000");
test_opcode!(op_2008, "2008");
test_opcode!(op_2010, "2010");
test_opcode!(op_2018, "2018");
test_opcode!(op_2020, "2020");
test_opcode!(op_2028, "2028");
test_opcode!(op_2030, "2030");
test_opcode!(op_2038, "2038");
test_opcode!(op_2039, "2039");
test_opcode!(op_203a, "203a");
test_opcode!(op_203b, "203b");
test_opcode!(op_203c, "203c");
test_opcode!(op_2040, "2040");
test_opcode!(op_2048, "2048");
test_opcode!(op_2050, "2050");
test_opcode!(op_2058, "2058");
test_opcode!(op_2060, "2060");
test_opcode!(op_2068, "2068");
test_opcode!(op_2070, "2070");
test_opcode!(op_2078, "2078");
test_opcode!(op_2079, "2079");
test_opcode!(op_207a, "207a");
test_opcode!(op_207b, "207b");
test_opcode!(op_207c, "207c");
test_opcode!(op_2080, "2080");
test_opcode!(op_2088, "2088");
test_opcode!(op_2090, "2090");
test_opcode!(op_2098, "2098");
test_opcode!(op_20a0, "20a0");
test_opcode!(op_20a8, "20a8");
test_opcode!(op_20b0, "20b0");
test_opcode!(op_20b8, "20b8");
test_opcode!(op_20b9, "20b9");
test_opcode!(op_20ba, "20ba");
test_opcode!(op_20bb, "20bb");
test_opcode!(op_20bc, "20bc");
test_opcode!(op_20c0, "20c0");
test_opcode!(op_20c8, "20c8");
test_opcode!(op_20d0, "20d0");
test_opcode!(op_20d8, "20d8");
test_opcode!(op_20e0, "20e0");
test_opcode!(op_20e8, "20e8");
test_opcode!(op_20f0, "20f0");
test_opcode!(op_20f8, "20f8");
test_opcode!(op_20f9, "20f9");
test_opcode!(op_20fa, "20fa");
test_opcode!(op_20fb, "20fb");
test_opcode!(op_20fc, "20fc");
test_opcode!(op_2100, "2100");
test_opcode!(op_2108, "2108");
test_opcode!(op_2110, "2110");
test_opcode!(op_2118, "2118");
test_opcode!(op_2120, "2120");
test_opcode!(op_2128, "2128");
test_opcode!(op_2130, "2130");
test_opcode!(op_2138, "2138");
test_opcode!(op_2139, "2139");
test_opcode!(op_213a, "213a");
test_opcode!(op_213b, "213b");
test_opcode!(op_213c, "213c");
test_opcode!(op_2140, "2140");
test_opcode!(op_2148, "2148");
test_opcode!(op_2150, "2150");
test_opcode!(op_2158, "2158");
test_opcode!(op_2160, "2160");
test_opcode!(op_2168, "2168");
test_opcode!(op_2170, "2170");
test_opcode!(op_2178, "2178");
test_opcode!(op_2179, "2179");
test_opcode!(op_217a, "217a");
test_opcode!(op_217b, "217b");
test_opcode!(op_217c, "217c");
test_opcode!(op_2180, "2180");
test_opcode!(op_2188, "2188");
test_opcode!(op_2190, "2190");
test_opcode!(op_2198, "2198");
test_opcode!(op_21a0, "21a0");
test_opcode!(op_21a8, "21a8");
test_opcode!(op_21b0, "21b0");
test_opcode!(op_21b8, "21b8");
test_opcode!(op_21b9, "21b9");
test_opcode!(op_21ba, "21ba");
test_opcode!(op_21bb, "21bb");
test_opcode!(op_21bc, "21bc");
test_opcode!(op_21c0, "21c0");
test_opcode!(op_21c8, "21c8");
test_opcode!(op_21d0, "21d0");
test_opcode!(op_21d8, "21d8");
test_opcode!(op_21e0, "21e0");
test_opcode!(op_21e8, "21e8");
test_opcode!(op_21f0, "21f0");
test_opcode!(op_21f8, "21f8");
test_opcode!(op_21f9, "21f9");
test_opcode!(op_21fa, "21fa");
test_opcode!(op_21fb, "21fb");
test_opcode!(op_21fc, "21fc");
test_opcode!(op_23c0, "23c0");
test_opcode!(op_23c8, "23c8");
test_opcode!(op_23d0, "23d0");
test_opcode!(op_23d8, "23d8");
test_opcode!(op_23e0, "23e0");
test_opcode!(op_23e8, "23e8");
test_opcode!(op_23f0, "23f0");
test_opcode!(op_23f8, "23f8");
test_opcode!(op_23f9, "23f9");
test_opcode!(op_23fa, "23fa");
test_opcode!(op_23fb, "23fb");
test_opcode!(op_23fc, "23fc");
test_opcode!(op_3000, "3000");
test_opcode!(op_3008, "3008");
test_opcode!(op_3010, "3010");
test_opcode!(op_3018, "3018");
test_opcode!(op_3020, "3020");
test_opcode!(op_3028, "3028");
test_opcode!(op_3030, "3030");
test_opcode!(op_3038, "3038");
test_opcode!(op_3039, "3039");
test_opcode!(op_303a, "303a");
test_opcode!(op_303b, "303b");
test_opcode!(op_303c, "303c");
test_opcode!(op_3040, "3040");
test_opcode!(op_3048, "3048");
test_opcode!(op_3050, "3050");
test_opcode!(op_3058, "3058");
test_opcode!(op_3060, "3060");
test_opcode!(op_3068, "3068");
test_opcode!(op_3070, "3070");
test_opcode!(op_3078, "3078");
test_opcode!(op_3079, "3079");
test_opcode!(op_307a, "307a");
test_opcode!(op_307b, "307b");
test_opcode!(op_307c, "307c");
test_opcode!(op_3080, "3080");
test_opcode!(op_3088, "3088");
test_opcode!(op_3090, "3090");
test_opcode!(op_3098, "3098");
test_opcode!(op_30a0, "30a0");
test_opcode!(op_30a8, "30a8");
test_opcode!(op_30b0, "30b0");
test_opcode!(op_30b8, "30b8");
test_opcode!(op_30b9, "30b9");
test_opcode!(op_30ba, "30ba");
test_opcode!(op_30bb, "30bb");
test_opcode!(op_30bc, "30bc");
test_opcode!(op_30c0, "30c0");
test_opcode!(op_30c8, "30c8");
test_opcode!(op_30d0, "30d0");
test_opcode!(op_30d8, "30d8");
test_opcode!(op_30e0, "30e0");
test_opcode!(op_30e8, "30e8");
test_opcode!(op_30f0, "30f0");
test_opcode!(op_30f8, "30f8");
test_opcode!(op_30f9, "30f9");
test_opcode!(op_30fa, "30fa");
test_opcode!(op_30fb, "30fb");
test_opcode!(op_30fc, "30fc");
test_opcode!(op_3100, "3100");
test_opcode!(op_3108, "3108");
test_opcode!(op_3110, "3110");
test_opcode!(op_3118, "3118");
test_opcode!(op_3120, "3120");
test_opcode!(op_3128, "3128");
test_opcode!(op_3130, "3130");
test_opcode!(op_3138, "3138");
test_opcode!(op_3139, "3139");
test_opcode!(op_313a, "313a");
test_opcode!(op_313b, "313b");
test_opcode!(op_313c, "313c");
test_opcode!(op_3140, "3140");
test_opcode!(op_3148, "3148");
test_opcode!(op_3150, "3150");
test_opcode!(op_3158, "3158");
test_opcode!(op_3160, "3160");
test_opcode!(op_3168, "3168");
test_opcode!(op_3170, "3170");
test_opcode!(op_3178, "3178");
test_opcode!(op_3179, "3179");
test_opcode!(op_317a, "317a");
test_opcode!(op_317b, "317b");
test_opcode!(op_317c, "317c");
test_opcode!(op_3180, "3180");
test_opcode!(op_3188, "3188");
test_opcode!(op_3190, "3190");
test_opcode!(op_3198, "3198");
test_opcode!(op_31a0, "31a0");
test_opcode!(op_31a8, "31a8");
test_opcode!(op_31b0, "31b0");
test_opcode!(op_31b8, "31b8");
test_opcode!(op_31b9, "31b9");
test_opcode!(op_31ba, "31ba");
test_opcode!(op_31bb, "31bb");
test_opcode!(op_31bc, "31bc");
test_opcode!(op_31c0, "31c0");
test_opcode!(op_31c8, "31c8");
test_opcode!(op_31d0, "31d0");
test_opcode!(op_31d8, "31d8");
test_opcode!(op_31e0, "31e0");
test_opcode!(op_31e8, "31e8");
test_opcode!(op_31f0, "31f0");
test_opcode!(op_31f8, "31f8");
test_opcode!(op_31f9, "31f9");
test_opcode!(op_31fa, "31fa");
test_opcode!(op_31fb, "31fb");
test_opcode!(op_31fc, "31fc");
test_opcode!(op_33c0, "33c0");
test_opcode!(op_33c8, "33c8");
test_opcode!(op_33d0, "33d0");
test_opcode!(op_33d8, "33d8");
test_opcode!(op_33e0, "33e0");
test_opcode!(op_33e8, "33e8");
test_opcode!(op_33f0, "33f0");
test_opcode!(op_33f8, "33f8");
test_opcode!(op_33f9, "33f9");
test_opcode!(op_33fa, "33fa");
test_opcode!(op_33fb, "33fb");
test_opcode!(op_33fc, "33fc");
test_opcode!(op_4000, "4000");
test_opcode!(op_4010, "4010");
test_opcode!(op_4018, "4018");
test_opcode!(op_4020, "4020");
test_opcode!(op_4028, "4028");
test_opcode!(op_4030, "4030");
test_opcode!(op_4038, "4038");
test_opcode!(op_4039, "4039");
test_opcode!(op_4040, "4040");
test_opcode!(op_4050, "4050");
test_opcode!(op_4058, "4058");
test_opcode!(op_4060, "4060");
test_opcode!(op_4068, "4068");
test_opcode!(op_4070, "4070");
test_opcode!(op_4078, "4078");
test_opcode!(op_4079, "4079");
test_opcode!(op_4080, "4080");
test_opcode!(op_4090, "4090");
test_opcode!(op_4098, "4098");
test_opcode!(op_40a0, "40a0");
test_opcode!(op_40a8, "40a8");
test_opcode!(op_40b0, "40b0");
test_opcode!(op_40b8, "40b8");
test_opcode!(op_40b9, "40b9");
test_opcode!(op_40c0, "40c0");
test_opcode!(op_40d0, "40d0");
test_opcode!(op_40d8, "40d8");
test_opcode!(op_40e0, "40e0");
test_opcode!(op_40e8, "40e8");
test_opcode!(op_40f0, "40f0");
test_opcode!(op_40f8, "40f8");
test_opcode!(op_40f9, "40f9");
test_opcode!(op_4180, "4180");
test_opcode!(op_4190, "4190");
test_opcode!(op_4198, "4198");
test_opcode!(op_41a0, "41a0");
test_opcode!(op_41a8, "41a8");
test_opcode!(op_41b0, "41b0");
test_opcode!(op_41b8, "41b8");
test_opcode!(op_41b9, "41b9");
test_opcode!(op_41ba, "41ba");
test_opcode!(op_41bb, "41bb");
test_opcode!(op_41bc, "41bc");
test_opcode!(op_41d0, "41d0");
test_opcode!(op_41e8, "41e8");
test_opcode!(op_41f0, "41f0");
test_opcode!(op_41f8, "41f8");
test_opcode!(op_41f9, "41f9");
test_opcode!(op_41fa, "41fa");
test_opcode!(op_41fb, "41fb");
test_opcode!(op_4200, "4200");
test_opcode!(op_4210, "4210");
test_opcode!(op_4218, "4218");
test_opcode!(op_4220, "4220");
test_opcode!(op_4228, "4228");
test_opcode!(op_4230, "4230");
test_opcode!(op_4238, "4238");
test_opcode!(op_4239, "4239");
test_opcode!(op_4240, "4240");
test_opcode!(op_4250, "4250");
test_opcode!(op_4258, "4258");
test_opcode!(op_4260, "4260");
test_opcode!(op_4268, "4268");
test_opcode!(op_4270, "4270");
test_opcode!(op_4278, "4278");
test_opcode!(op_4279, "4279");
test_opcode!(op_4280, "4280");
test_opcode!(op_4290, "4290");
test_opcode!(op_4298, "4298");
test_opcode!(op_42a0, "42a0");
test_opcode!(op_42a8, "42a8");
test_opcode!(op_42b0, "42b0");
test_opcode!(op_42b8, "42b8");
test_opcode!(op_42b9, "42b9");
test_opcode!(op_4400, "4400");
test_opcode!(op_4410, "4410");
test_opcode!(op_4418, "4418");
test_opcode!(op_4420, "4420");
test_opcode!(op_4428, "4428");
test_opcode!(op_4430, "4430");
test_opcode!(op_4438, "4438");
test_opcode!(op_4439, "4439");
test_opcode!(op_4440, "4440");
test_opcode!(op_4450, "4450");
test_opcode!(op_4458, "4458");
test_opcode!(op_4460, "4460");
test_opcode!(op_4468, "4468");
test_opcode!(op_4470, "4470");
test_opcode!(op_4478, "4478");
test_opcode!(op_4479, "4479");
test_opcode!(op_4480, "4480");
test_opcode!(op_4490, "4490");
test_opcode!(op_4498, "4498");
test_opcode!(op_44a0, "44a0");
test_opcode!(op_44a8, "44a8");
test_opcode!(op_44b0, "44b0");
test_opcode!(op_44b8, "44b8");
test_opcode!(op_44b9, "44b9");
test_opcode!(op_44c0, "44c0");
test_opcode!(op_44d0, "44d0");
test_opcode!(op_44d8, "44d8");
test_opcode!(op_44e0, "44e0");
test_opcode!(op_44e8, "44e8");
test_opcode!(op_44f0, "44f0");
test_opcode!(op_44f8, "44f8");
test_opcode!(op_44f9, "44f9");
test_opcode!(op_44fa, "44fa");
test_opcode!(op_44fb, "44fb");
test_opcode!(op_44fc, "44fc");
test_opcode!(op_4600, "4600");
test_opcode!(op_4610, "4610");
test_opcode!(op_4618, "4618");
test_opcode!(op_4620, "4620");
test_opcode!(op_4628, "4628");
test_opcode!(op_4630, "4630");
test_opcode!(op_4638, "4638");
test_opcode!(op_4639, "4639");
test_opcode!(op_4640, "4640");
test_opcode!(op_4650, "4650");
test_opcode!(op_4658, "4658");
test_opcode!(op_4660, "4660");
test_opcode!(op_4668, "4668");
test_opcode!(op_4670, "4670");
test_opcode!(op_4678, "4678");
test_opcode!(op_4679, "4679");
test_opcode!(op_4680, "4680");
test_opcode!(op_4690, "4690");
test_opcode!(op_4698, "4698");
test_opcode!(op_46a0, "46a0");
test_opcode!(op_46a8, "46a8");
test_opcode!(op_46b0, "46b0");
test_opcode!(op_46b8, "46b8");
test_opcode!(op_46b9, "46b9");
test_opcode!(op_46c0, "46c0");
test_opcode!(op_46d0, "46d0");
test_opcode!(op_46d8, "46d8");
test_opcode!(op_46e0, "46e0");
test_opcode!(op_46e8, "46e8");
test_opcode!(op_46f0, "46f0");
test_opcode!(op_46f8, "46f8");
test_opcode!(op_46f9, "46f9");
test_opcode!(op_46fa, "46fa");
test_opcode!(op_46fb, "46fb");
test_opcode!(op_46fc, "46fc");
test_opcode!(op_4800, "4800");
test_opcode!(op_4810, "4810");
test_opcode!(op_4818, "4818");
test_opcode!(op_4820, "4820");
test_opcode!(op_4828, "4828");
test_opcode!(op_4830, "4830");
test_opcode!(op_4838, "4838");
test_opcode!(op_4839, "4839");
test_opcode!(op_4840, "4840");
test_opcode!(op_4850, "4850");
test_opcode!(op_4868, "4868");
test_opcode!(op_4870, "4870");
test_opcode!(op_4878, "4878");
test_opcode!(op_4879, "4879");
test_opcode!(op_487a, "487a");
test_opcode!(op_487b, "487b");
test_opcode!(op_4880, "4880");
test_opcode!(op_4890, "4890");
test_opcode!(op_48a0, "48a0");
test_opcode!(op_48a8, "48a8");
test_opcode!(op_48b0, "48b0");
test_opcode!(op_48b8, "48b8");
test_opcode!(op_48b9, "48b9");
test_opcode!(op_48c0, "48c0");
test_opcode!(op_48d0, "48d0");
test_opcode!(op_48e0, "48e0");
test_opcode!(op_48e8, "48e8");
test_opcode!(op_48f0, "48f0");
test_opcode!(op_48f8, "48f8");
test_opcode!(op_48f9, "48f9");
test_opcode!(op_4a00, "4a00");
test_opcode!(op_4a10, "4a10");
test_opcode!(op_4a18, "4a18");
test_opcode!(op_4a20, "4a20");
test_opcode!(op_4a28, "4a28");
test_opcode!(op_4a30, "4a30");
test_opcode!(op_4a38, "4a38");
test_opcode!(op_4a39, "4a39");
test_opcode!(op_4a40, "4a40");
test_opcode!(op_4a50, "4a50");
test_opcode!(op_4a58, "4a58");
test_opcode!(op_4a60, "4a60");
test_opcode!(op_4a68, "4a68");
test_opcode!(op_4a70, "4a70");
test_opcode!(op_4a78, "4a78");
test_opcode!(op_4a79, "4a79");
test_opcode!(op_4a80, "4a80");
test_opcode!(op_4a90, "4a90");
test_opcode!(op_4a98, "4a98");
test_opcode!(op_4aa0, "4aa0");
test_opcode!(op_4aa8, "4aa8");
test_opcode!(op_4ab0, "4ab0");
test_opcode!(op_4ab8, "4ab8");
test_opcode!(op_4ab9, "4ab9");
test_opcode!(op_4ac0, "4ac0");
test_opcode!(op_4ad0, "4ad0");
test_opcode!(op_4ad8, "4ad8");
test_opcode!(op_4ae0, "4ae0");
test_opcode!(op_4ae8, "4ae8");
test_opcode!(op_4af0, "4af0");
test_opcode!(op_4af8, "4af8");
test_opcode!(op_4af9, "4af9");
test_opcode!(op_4c90, "4c90");
test_opcode!(op_4c98, "4c98");
test_opcode!(op_4ca8, "4ca8");
test_opcode!(op_4cb0, "4cb0");
test_opcode!(op_4cb8, "4cb8");
test_opcode!(op_4cb9, "4cb9");
test_opcode!(op_4cba, "4cba");
test_opcode!(op_4cbb, "4cbb");
test_opcode!(op_4cd0, "4cd0");
test_opcode!(op_4cd8, "4cd8");
test_opcode!(op_4ce8, "4ce8");
test_opcode!(op_4cf0, "4cf0");
test_opcode!(op_4cf8, "4cf8");
test_opcode!(op_4cf9, "4cf9");
test_opcode!(op_4cfa, "4cfa");
test_opcode!(op_4cfb, "4cfb");
test_opcode!(op_4e40, "4e40");
test_opcode!(op_4e50, "4e50");
test_opcode!(op_4e58, "4e58");
test_opcode!(op_4e60, "4e60");
test_opcode!(op_4e68, "4e68");
test_opcode!(op_4e70, "4e70");
test_opcode!(op_4e71, "4e71");
test_opcode!(op_4e73, "4e73");
test_opcode!(op_4e75, "4e75");
test_opcode!(op_4e76, "4e76");
test_opcode!(op_4e77, "4e77");
test_opcode!(op_4e90, "4e90");
test_opcode!(op_4ea8, "4ea8");
test_opcode!(op_4eb0, "4eb0");
test_opcode!(op_4eb8, "4eb8");
test_opcode!(op_4eb9, "4eb9");
test_opcode!(op_4eba, "4eba");
test_opcode!(op_4ebb, "4ebb");
test_opcode!(op_4ed0, "4ed0");
test_opcode!(op_4ee8, "4ee8");
test_opcode!(op_4ef0, "4ef0");
test_opcode!(op_4ef8, "4ef8");
test_opcode!(op_4ef9, "4ef9");
test_opcode!(op_4efa, "4efa");
test_opcode!(op_4efb, "4efb");
test_opcode!(op_5000, "5000");
test_opcode!(op_5010, "5010");
test_opcode!(op_5018, "5018");
test_opcode!(op_5020, "5020");
test_opcode!(op_5028, "5028");
test_opcode!(op_5030, "5030");
test_opcode!(op_5038, "5038");
test_opcode!(op_5039, "5039");
test_opcode!(op_5040, "5040");
test_opcode!(op_5048, "5048");
test_opcode!(op_5050, "5050");
test_opcode!(op_5058, "5058");
test_opcode!(op_5060, "5060");
test_opcode!(op_5068, "5068");
test_opcode!(op_5070, "5070");
test_opcode!(op_5078, "5078");
test_opcode!(op_5079, "5079");
test_opcode!(op_5080, "5080");
test_opcode!(op_5088, "5088");
test_opcode!(op_5090, "5090");
test_opcode!(op_5098, "5098");
test_opcode!(op_50a0, "50a0");
test_opcode!(op_50a8, "50a8");
test_opcode!(op_50b0, "50b0");
test_opcode!(op_50b8, "50b8");
test_opcode!(op_50b9, "50b9");
test_opcode!(op_50c0, "50c0");
test_opcode!(op_50c8, "50c8");
test_opcode!(op_50d0, "50d0");
test_opcode!(op_50d8, "50d8");
test_opcode!(op_50e0, "50e0");
test_opcode!(op_50e8, "50e8");
test_opcode!(op_50f0, "50f0");
test_opcode!(op_50f8, "50f8");
test_opcode!(op_50f9, "50f9");
test_opcode!(op_5100, "5100");
test_opcode!(op_5110, "5110");
test_opcode!(op_5118, "5118");
test_opcode!(op_5120, "5120");
test_opcode!(op_5128, "5128");
test_opcode!(op_5130, "5130");
test_opcode!(op_5138, "5138");
test_opcode!(op_5139, "5139");
test_opcode!(op_5140, "5140");
test_opcode!(op_5148, "5148");
test_opcode!(op_5150, "5150");
test_opcode!(op_5158, "5158");
test_opcode!(op_5160, "5160");
test_opcode!(op_5168, "5168");
test_opcode!(op_5170, "5170");
test_opcode!(op_5178, "5178");
test_opcode!(op_5179, "5179");
test_opcode!(op_5180, "5180");
test_opcode!(op_5188, "5188");
test_opcode!(op_5190, "5190");
test_opcode!(op_5198, "5198");
test_opcode!(op_51a0, "51a0");
test_opcode!(op_51a8, "51a8");
test_opcode!(op_51b0, "51b0");
test_opcode!(op_51b8, "51b8");
test_opcode!(op_51b9, "51b9");
test_opcode!(op_51c0, "51c0");
test_opcode!(op_51c8, "51c8");
test_opcode!(op_51d0, "51d0");
test_opcode!(op_51d8, "51d8");
test_opcode!(op_51e0, "51e0");
test_opcode!(op_51e8, "51e8");
test_opcode!(op_51f0, "51f0");
test_opcode!(op_51f8, "51f8");
test_opcode!(op_51f9, "51f9");
test_opcode!(op_52c0, "52c0");
test_opcode!(op_52c8, "52c8");
test_opcode!(op_52d0, "52d0");
test_opcode!(op_52d8, "52d8");
test_opcode!(op_52e0, "52e0");
test_opcode!(op_52e8, "52e8");
test_opcode!(op_52f0, "52f0");
test_opcode!(op_52f8, "52f8");
test_opcode!(op_52f9, "52f9");
test_opcode!(op_53c0, "53c0");
test_opcode!(op_53c8, "53c8");
test_opcode!(op_53d0, "53d0");
test_opcode!(op_53d8, "53d8");
test_opcode!(op_53e0, "53e0");
test_opcode!(op_53e8, "53e8");
test_opcode!(op_53f0, "53f0");
test_opcode!(op_53f8, "53f8");
test_opcode!(op_53f9, "53f9");
test_opcode!(op_54c0, "54c0");
test_opcode!(op_54c8, "54c8");
test_opcode!(op_54d0, "54d0");
test_opcode!(op_54d8, "54d8");
test_opcode!(op_54e0, "54e0");
test_opcode!(op_54e8, "54e8");
test_opcode!(op_54f0, "54f0");
test_opcode!(op_54f8, "54f8");
test_opcode!(op_54f9, "54f9");
test_opcode!(op_55c0, "55c0");
test_opcode!(op_55c8, "55c8");
test_opcode!(op_55d0, "55d0");
test_opcode!(op_55d8, "55d8");
test_opcode!(op_55e0, "55e0");
test_opcode!(op_55e8, "55e8");
test_opcode!(op_55f0, "55f0");
test_opcode!(op_55f8, "55f8");
test_opcode!(op_55f9, "55f9");
test_opcode!(op_56c0, "56c0");
test_opcode!(op_56c8, "56c8");
test_opcode!(op_56d0, "56d0");
test_opcode!(op_56d8, "56d8");
test_opcode!(op_56e0, "56e0");
test_opcode!(op_56e8, "56e8");
test_opcode!(op_56f0, "56f0");
test_opcode!(op_56f8, "56f8");
test_opcode!(op_56f9, "56f9");
test_opcode!(op_57c0, "57c0");
test_opcode!(op_57c8, "57c8");
test_opcode!(op_57d0, "57d0");
test_opcode!(op_57d8, "57d8");
test_opcode!(op_57e0, "57e0");
test_opcode!(op_57e8, "57e8");
test_opcode!(op_57f0, "57f0");
test_opcode!(op_57f8, "57f8");
test_opcode!(op_57f9, "57f9");
test_opcode!(op_58c0, "58c0");
test_opcode!(op_58c8, "58c8");
test_opcode!(op_58d0, "58d0");
test_opcode!(op_58d8, "58d8");
test_opcode!(op_58e0, "58e0");
test_opcode!(op_58e8, "58e8");
test_opcode!(op_58f0, "58f0");
test_opcode!(op_58f8, "58f8");
test_opcode!(op_58f9, "58f9");
test_opcode!(op_59c0, "59c0");
test_opcode!(op_59c8, "59c8");
test_opcode!(op_59d0, "59d0");
test_opcode!(op_59d8, "59d8");
test_opcode!(op_59e0, "59e0");
test_opcode!(op_59e8, "59e8");
test_opcode!(op_59f0, "59f0");
test_opcode!(op_59f8, "59f8");
test_opcode!(op_59f9, "59f9");
test_opcode!(op_5ac0, "5ac0");
test_opcode!(op_5ac8, "5ac8");
test_opcode!(op_5ad0, "5ad0");
test_opcode!(op_5ad8, "5ad8");
test_opcode!(op_5ae0, "5ae0");
test_opcode!(op_5ae8, "5ae8");
test_opcode!(op_5af0, "5af0");
test_opcode!(op_5af8, "5af8");
test_opcode!(op_5af9, "5af9");
test_opcode!(op_5bc0, "5bc0");
test_opcode!(op_5bc8, "5bc8");
test_opcode!(op_5bd0, "5bd0");
test_opcode!(op_5bd8, "5bd8");
test_opcode!(op_5be0, "5be0");
test_opcode!(op_5be8, "5be8");
test_opcode!(op_5bf0, "5bf0");
test_opcode!(op_5bf8, "5bf8");
test_opcode!(op_5bf9, "5bf9");
test_opcode!(op_5cc0, "5cc0");
test_opcode!(op_5cc8, "5cc8");
test_opcode!(op_5cd0, "5cd0");
test_opcode!(op_5cd8, "5cd8");
test_opcode!(op_5ce0, "5ce0");
test_opcode!(op_5ce8, "5ce8");
test_opcode!(op_5cf0, "5cf0");
test_opcode!(op_5cf8, "5cf8");
test_opcode!(op_5cf9, "5cf9");
test_opcode!(op_5dc0, "5dc0");
test_opcode!(op_5dc8, "5dc8");
test_opcode!(op_5dd0, "5dd0");
test_opcode!(op_5dd8, "5dd8");
test_opcode!(op_5de0, "5de0");
test_opcode!(op_5de8, "5de8");
test_opcode!(op_5df0, "5df0");
test_opcode!(op_5df8, "5df8");
test_opcode!(op_5df9, "5df9");
test_opcode!(op_5ec0, "5ec0");
test_opcode!(op_5ec8, "5ec8");
test_opcode!(op_5ed0, "5ed0");
test_opcode!(op_5ed8, "5ed8");
test_opcode!(op_5ee0, "5ee0");
test_opcode!(op_5ee8, "5ee8");
test_opcode!(op_5ef0, "5ef0");
test_opcode!(op_5ef8, "5ef8");
test_opcode!(op_5ef9, "5ef9");
test_opcode!(op_5fc0, "5fc0");
test_opcode!(op_5fc8, "5fc8");
test_opcode!(op_5fd0, "5fd0");
test_opcode!(op_5fd8, "5fd8");
test_opcode!(op_5fe0, "5fe0");
test_opcode!(op_5fe8, "5fe8");
test_opcode!(op_5ff0, "5ff0");
test_opcode!(op_5ff8, "5ff8");
test_opcode!(op_5ff9, "5ff9");
test_opcode!(op_6000, "6000");
test_opcode!(op_6001, "6001");
test_opcode!(op_6100, "6100");
test_opcode!(op_6101, "6101");
test_opcode!(op_6200, "6200");
test_opcode!(op_6201, "6201");
test_opcode!(op_6300, "6300");
test_opcode!(op_6301, "6301");
test_opcode!(op_6400, "6400");
test_opcode!(op_6401, "6401");
test_opcode!(op_6500, "6500");
test_opcode!(op_6501, "6501");
test_opcode!(op_6600, "6600");
test_opcode!(op_6601, "6601");
test_opcode!(op_6700, "6700");
test_opcode!(op_6701, "6701");
test_opcode!(op_6800, "6800");
test_opcode!(op_6801, "6801");
test_opcode!(op_6900, "6900");
test_opcode!(op_6901, "6901");
test_opcode!(op_6a00, "6a00");
test_opcode!(op_6a01, "6a01");
test_opcode!(op_6b00, "6b00");
test_opcode!(op_6b01, "6b01");
test_opcode!(op_6c00, "6c00");
test_opcode!(op_6c01, "6c01");
test_opcode!(op_6d00, "6d00");
test_opcode!(op_6d01, "6d01");
test_opcode!(op_6e00, "6e00");
test_opcode!(op_6e01, "6e01");
test_opcode!(op_6f00, "6f00");
test_opcode!(op_6f01, "6f01");
test_opcode!(op_7000, "7000");
test_opcode!(op_8000, "8000");
test_opcode!(op_8010, "8010");
test_opcode!(op_8018, "8018");
test_opcode!(op_8020, "8020");
test_opcode!(op_8028, "8028");
test_opcode!(op_8030, "8030");
test_opcode!(op_8038, "8038");
test_opcode!(op_8039, "8039");
test_opcode!(op_803a, "803a");
test_opcode!(op_803b, "803b");
test_opcode!(op_803c, "803c");
test_opcode!(op_8040, "8040");
test_opcode!(op_8050, "8050");
test_opcode!(op_8058, "8058");
test_opcode!(op_8060, "8060");
test_opcode!(op_8068, "8068");
test_opcode!(op_8070, "8070");
test_opcode!(op_8078, "8078");
test_opcode!(op_8079, "8079");
test_opcode!(op_807a, "807a");
test_opcode!(op_807b, "807b");
test_opcode!(op_807c, "807c");
test_opcode!(op_8080, "8080");
test_opcode!(op_8090, "8090");
test_opcode!(op_8098, "8098");
test_opcode!(op_80a0, "80a0");
test_opcode!(op_80a8, "80a8");
test_opcode!(op_80b0, "80b0");
test_opcode!(op_80b8, "80b8");
test_opcode!(op_80b9, "80b9");
test_opcode!(op_80ba, "80ba");
test_opcode!(op_80bb, "80bb");
test_opcode!(op_80bc, "80bc");
test_opcode!(op_80c0, "80c0");
test_opcode!(op_80d0, "80d0");
test_opcode!(op_80d8, "80d8");
test_opcode!(op_80e0, "80e0");
test_opcode!(op_80e8, "80e8");
test_opcode!(op_80f0, "80f0");
test_opcode!(op_80f8, "80f8");
test_opcode!(op_80f9, "80f9");
test_opcode!(op_80fa, "80fa");
test_opcode!(op_80fb, "80fb");
test_opcode!(op_80fc, "80fc");
test_opcode!(op_8100, "8100");
test_opcode!(op_8108, "8108");
test_opcode!(op_8110, "8110");
test_opcode!(op_8118, "8118");
test_opcode!(op_8120, "8120");
test_opcode!(op_8128, "8128");
test_opcode!(op_8130, "8130");
test_opcode!(op_8138, "8138");
test_opcode!(op_8139, "8139");
test_opcode!(op_8150, "8150");
test_opcode!(op_8158, "8158");
test_opcode!(op_8160, "8160");
test_opcode!(op_8168, "8168");
test_opcode!(op_8170, "8170");
test_opcode!(op_8178, "8178");
test_opcode!(op_8179, "8179");
test_opcode!(op_8190, "8190");
test_opcode!(op_8198, "8198");
test_opcode!(op_81a0, "81a0");
test_opcode!(op_81a8, "81a8");
test_opcode!(op_81b0, "81b0");
test_opcode!(op_81b8, "81b8");
test_opcode!(op_81b9, "81b9");
test_opcode!(op_81c0, "81c0");
test_opcode!(op_81d0, "81d0");
test_opcode!(op_81d8, "81d8");
test_opcode!(op_81e0, "81e0");
test_opcode!(op_81e8, "81e8");
test_opcode!(op_81f0, "81f0");
test_opcode!(op_81f8, "81f8");
test_opcode!(op_81f9, "81f9");
test_opcode!(op_81fa, "81fa");
test_opcode!(op_81fb, "81fb");
test_opcode!(op_81fc, "81fc");
test_opcode!(op_9000, "9000");
test_opcode!(op_9010, "9010");
test_opcode!(op_9018, "9018");
test_opcode!(op_9020, "9020");
test_opcode!(op_9028, "9028");
test_opcode!(op_9030, "9030");
test_opcode!(op_9038, "9038");
test_opcode!(op_9039, "9039");
test_opcode!(op_903a, "903a");
test_opcode!(op_903b, "903b");
test_opcode!(op_903c, "903c");
test_opcode!(op_9040, "9040");
test_opcode!(op_9048, "9048");
test_opcode!(op_9050, "9050");
test_opcode!(op_9058, "9058");
test_opcode!(op_9060, "9060");
test_opcode!(op_9068, "9068");
test_opcode!(op_9070, "9070");
test_opcode!(op_9078, "9078");
test_opcode!(op_9079, "9079");
test_opcode!(op_907a, "907a");
test_opcode!(op_907b, "907b");
test_opcode!(op_907c, "907c");
test_opcode!(op_9080, "9080");
test_opcode!(op_9088, "9088");
test_opcode!(op_9090, "9090");
test_opcode!(op_9098, "9098");
test_opcode!(op_90a0, "90a0");
test_opcode!(op_90a8, "90a8");
test_opcode!(op_90b0, "90b0");
test_opcode!(op_90b8, "90b8");
test_opcode!(op_90b9, "90b9");
test_opcode!(op_90ba, "90ba");
test_opcode!(op_90bb, "90bb");
test_opcode!(op_90bc, "90bc");
test_opcode!(op_90c0, "90c0");
test_opcode!(op_90c8, "90c8");
test_opcode!(op_90d0, "90d0");
test_opcode!(op_90d8, "90d8");
test_opcode!(op_90e0, "90e0");
test_opcode!(op_90e8, "90e8");
test_opcode!(op_90f0, "90f0");
test_opcode!(op_90f8, "90f8");
test_opcode!(op_90f9, "90f9");
test_opcode!(op_90fa, "90fa");
test_opcode!(op_90fb, "90fb");
test_opcode!(op_90fc, "90fc");
test_opcode!(op_9100, "9100");
test_opcode!(op_9108, "9108");
test_opcode!(op_9110, "9110");
test_opcode!(op_9118, "9118");
test_opcode!(op_9120, "9120");
test_opcode!(op_9128, "9128");
test_opcode!(op_9130, "9130");
test_opcode!(op_9138, "9138");
test_opcode!(op_9139, "9139");
test_opcode!(op_9140, "9140");
test_opcode!(op_9148, "9148");
test_opcode!(op_9150, "9150");
test_opcode!(op_9158, "9158");
test_opcode!(op_9160, "9160");
test_opcode!(op_9168, "9168");
test_opcode!(op_9170, "9170");
test_opcode!(op_9178, "9178");
test_opcode!(op_9179, "9179");
test_opcode!(op_9180, "9180");
test_opcode!(op_9188, "9188");
test_opcode!(op_9190, "9190");
test_opcode!(op_9198, "9198");
test_opcode!(op_91a0, "91a0");
test_opcode!(op_91a8, "91a8");
test_opcode!(op_91b0, "91b0");
test_opcode!(op_91b8, "91b8");
test_opcode!(op_91b9, "91b9");
test_opcode!(op_91c0, "91c0");
test_opcode!(op_91c8, "91c8");
test_opcode!(op_91d0, "91d0");
test_opcode!(op_91d8, "91d8");
test_opcode!(op_91e0, "91e0");
test_opcode!(op_91e8, "91e8");
test_opcode!(op_91f0, "91f0");
test_opcode!(op_91f8, "91f8");
test_opcode!(op_91f9, "91f9");
test_opcode!(op_91fa, "91fa");
test_opcode!(op_91fb, "91fb");
test_opcode!(op_91fc, "91fc");
test_opcode!(op_a000, "a000");
test_opcode!(op_b000, "b000");
test_opcode!(op_b010, "b010");
test_opcode!(op_b018, "b018");
test_opcode!(op_b020, "b020");
test_opcode!(op_b028, "b028");
test_opcode!(op_b030, "b030");
test_opcode!(op_b038, "b038");
test_opcode!(op_b039, "b039");
test_opcode!(op_b03a, "b03a");
test_opcode!(op_b03b, "b03b");
test_opcode!(op_b03c, "b03c");
test_opcode!(op_b040, "b040");
test_opcode!(op_b048, "b048");
test_opcode!(op_b050, "b050");
test_opcode!(op_b058, "b058");
test_opcode!(op_b060, "b060");
test_opcode!(op_b068, "b068");
test_opcode!(op_b070, "b070");
test_opcode!(op_b078, "b078");
test_opcode!(op_b079, "b079");
test_opcode!(op_b07a, "b07a");
test_opcode!(op_b07b, "b07b");
test_opcode!(op_b07c, "b07c");
test_opcode!(op_b080, "b080");
test_opcode!(op_b088, "b088");
test_opcode!(op_b090, "b090");
test_opcode!(op_b098, "b098");
test_opcode!(op_b0a0, "b0a0");
test_opcode!(op_b0a8, "b0a8");
test_opcode!(op_b0b0, "b0b0");
test_opcode!(op_b0b8, "b0b8");
test_opcode!(op_b0b9, "b0b9");
test_opcode!(op_b0ba, "b0ba");
test_opcode!(op_b0bb, "b0bb");
test_opcode!(op_b0bc, "b0bc");
test_opcode!(op_b0c0, "b0c0");
test_opcode!(op_b0c8, "b0c8");
test_opcode!(op_b0d0, "b0d0");
test_opcode!(op_b0d8, "b0d8");
test_opcode!(op_b0e0, "b0e0");
test_opcode!(op_b0e8, "b0e8");
test_opcode!(op_b0f0, "b0f0");
test_opcode!(op_b0f8, "b0f8");
test_opcode!(op_b0f9, "b0f9");
test_opcode!(op_b0fa, "b0fa");
test_opcode!(op_b0fb, "b0fb");
test_opcode!(op_b0fc, "b0fc");
test_opcode!(op_b100, "b100");
test_opcode!(op_b108, "b108");
test_opcode!(op_b110, "b110");
test_opcode!(op_b118, "b118");
test_opcode!(op_b120, "b120");
test_opcode!(op_b128, "b128");
test_opcode!(op_b130, "b130");
test_opcode!(op_b138, "b138");
test_opcode!(op_b139, "b139");
test_opcode!(op_b140, "b140");
test_opcode!(op_b148, "b148");
test_opcode!(op_b150, "b150");
test_opcode!(op_b158, "b158");
test_opcode!(op_b160, "b160");
test_opcode!(op_b168, "b168");
test_opcode!(op_b170, "b170");
test_opcode!(op_b178, "b178");
test_opcode!(op_b179, "b179");
test_opcode!(op_b180, "b180");
test_opcode!(op_b188, "b188");
test_opcode!(op_b190, "b190");
test_opcode!(op_b198, "b198");
test_opcode!(op_b1a0, "b1a0");
test_opcode!(op_b1a8, "b1a8");
test_opcode!(op_b1b0, "b1b0");
test_opcode!(op_b1b8, "b1b8");
test_opcode!(op_b1b9, "b1b9");
test_opcode!(op_b1c0, "b1c0");
test_opcode!(op_b1c8, "b1c8");
test_opcode!(op_b1d0, "b1d0");
test_opcode!(op_b1d8, "b1d8");
test_opcode!(op_b1e0, "b1e0");
test_opcode!(op_b1e8, "b1e8");
test_opcode!(op_b1f0, "b1f0");
test_opcode!(op_b1f8, "b1f8");
test_opcode!(op_b1f9, "b1f9");
test_opcode!(op_b1fa, "b1fa");
test_opcode!(op_b1fb, "b1fb");
test_opcode!(op_b1fc, "b1fc");
test_opcode!(op_c000, "c000");
test_opcode!(op_c010, "c010");
test_opcode!(op_c018, "c018");
test_opcode!(op_c020, "c020");
test_opcode!(op_c028, "c028");
test_opcode!(op_c030, "c030");
test_opcode!(op_c038, "c038");
test_opcode!(op_c039, "c039");
test_opcode!(op_c03a, "c03a");
test_opcode!(op_c03b, "c03b");
test_opcode!(op_c03c, "c03c");
test_opcode!(op_c040, "c040");
test_opcode!(op_c050, "c050");
test_opcode!(op_c058, "c058");
test_opcode!(op_c060, "c060");
test_opcode!(op_c068, "c068");
test_opcode!(op_c070, "c070");
test_opcode!(op_c078, "c078");
test_opcode!(op_c079, "c079");
test_opcode!(op_c07a, "c07a");
test_opcode!(op_c07b, "c07b");
test_opcode!(op_c07c, "c07c");
test_opcode!(op_c080, "c080");
test_opcode!(op_c090, "c090");
test_opcode!(op_c098, "c098");
test_opcode!(op_c0a0, "c0a0");
test_opcode!(op_c0a8, "c0a8");
test_opcode!(op_c0b0, "c0b0");
test_opcode!(op_c0b8, "c0b8");
test_opcode!(op_c0b9, "c0b9");
test_opcode!(op_c0ba, "c0ba");
test_opcode!(op_c0bb, "c0bb");
test_opcode!(op_c0bc, "c0bc");
test_opcode!(op_c0c0, "c0c0");
test_opcode!(op_c0d0, "c0d0");
test_opcode!(op_c0d8, "c0d8");
test_opcode!(op_c0e0, "c0e0");
test_opcode!(op_c0e8, "c0e8");
test_opcode!(op_c0f0, "c0f0");
test_opcode!(op_c0f8, "c0f8");
test_opcode!(op_c0f9, "c0f9");
test_opcode!(op_c0fa, "c0fa");
test_opcode!(op_c0fb, "c0fb");
test_opcode!(op_c0fc, "c0fc");
test_opcode!(op_c100, "c100");
test_opcode!(op_c108, "c108");
test_opcode!(op_c110, "c110");
test_opcode!(op_c118, "c118");
test_opcode!(op_c120, "c120");
test_opcode!(op_c128, "c128");
test_opcode!(op_c130, "c130");
test_opcode!(op_c138, "c138");
test_opcode!(op_c139, "c139");
test_opcode!(op_c140, "c140");
test_opcode!(op_c148, "c148");
test_opcode!(op_c150, "c150");
test_opcode!(op_c158, "c158");
test_opcode!(op_c160, "c160");
test_opcode!(op_c168, "c168");
test_opcode!(op_c170, "c170");
test_opcode!(op_c178, "c178");
test_opcode!(op_c179, "c179");
test_opcode!(op_c188, "c188");
test_opcode!(op_c190, "c190");
test_opcode!(op_c198, "c198");
test_opcode!(op_c1a0, "c1a0");
test_opcode!(op_c1a8, "c1a8");
test_opcode!(op_c1b0, "c1b0");
test_opcode!(op_c1b8, "c1b8");
test_opcode!(op_c1b9, "c1b9");
test_opcode!(op_c1c0, "c1c0");
test_opcode!(op_c1d0, "c1d0");
test_opcode!(op_c1d8, "c1d8");
test_opcode!(op_c1e0, "c1e0");
test_opcode!(op_c1e8, "c1e8");
test_opcode!(op_c1f0, "c1f0");
test_opcode!(op_c1f8, "c1f8");
test_opcode!(op_c1f9, "c1f9");
test_opcode!(op_c1fa, "c1fa");
test_opcode!(op_c1fb, "c1fb");
test_opcode!(op_c1fc, "c1fc");
test_opcode!(op_d000, "d000");
test_opcode!(op_d010, "d010");
test_opcode!(op_d018, "d018");
test_opcode!(op_d020, "d020");
test_opcode!(op_d028, "d028");
test_opcode!(op_d030, "d030");
test_opcode!(op_d038, "d038");
test_opcode!(op_d039, "d039");
test_opcode!(op_d03a, "d03a");
test_opcode!(op_d03b, "d03b");
test_opcode!(op_d03c, "d03c");
test_opcode!(op_d040, "d040");
test_opcode!(op_d048, "d048");
test_opcode!(op_d050, "d050");
test_opcode!(op_d058, "d058");
test_opcode!(op_d060, "d060");
test_opcode!(op_d068, "d068");
test_opcode!(op_d070, "d070");
test_opcode!(op_d078, "d078");
test_opcode!(op_d079, "d079");
test_opcode!(op_d07a, "d07a");
test_opcode!(op_d07b, "d07b");
test_opcode!(op_d07c, "d07c");
test_opcode!(op_d080, "d080");
test_opcode!(op_d088, "d088");
test_opcode!(op_d090, "d090");
test_opcode!(op_d098, "d098");
test_opcode!(op_d0a0, "d0a0");
test_opcode!(op_d0a8, "d0a8");
test_opcode!(op_d0b0, "d0b0");
test_opcode!(op_d0b8, "d0b8");
test_opcode!(op_d0b9, "d0b9");
test_opcode!(op_d0ba, "d0ba");
test_opcode!(op_d0bb, "d0bb");
test_opcode!(op_d0bc, "d0bc");
test_opcode!(op_d0c0, "d0c0");
test_opcode!(op_d0c8, "d0c8");
test_opcode!(op_d0d0, "d0d0");
test_opcode!(op_d0d8, "d0d8");
test_opcode!(op_d0e0, "d0e0");
test_opcode!(op_d0e8, "d0e8");
test_opcode!(op_d0f0, "d0f0");
test_opcode!(op_d0f8, "d0f8");
test_opcode!(op_d0f9, "d0f9");
test_opcode!(op_d0fa, "d0fa");
test_opcode!(op_d0fb, "d0fb");
test_opcode!(op_d0fc, "d0fc");
test_opcode!(op_d100, "d100");
test_opcode!(op_d108, "d108");
test_opcode!(op_d110, "d110");
test_opcode!(op_d118, "d118");
test_opcode!(op_d120, "d120");
test_opcode!(op_d128, "d128");
test_opcode!(op_d130, "d130");
test_opcode!(op_d138, "d138");
test_opcode!(op_d139, "d139");
test_opcode!(op_d140, "d140");
test_opcode!(op_d148, "d148");
test_opcode!(op_d150, "d150");
test_opcode!(op_d158, "d158");
test_opcode!(op_d160, "d160");
test_opcode!(op_d168, "d168");
test_opcode!(op_d170, "d170");
test_opcode!(op_d178, "d178");
test_opcode!(op_d179, "d179");
test_opcode!(op_d180, "d180");
test_opcode!(op_d188, "d188");
test_opcode!(op_d190, "d190");
test_opcode!(op_d198, "d198");
test_opcode!(op_d1a0, "d1a0");
test_opcode!(op_d1a8, "d1a8");
test_opcode!(op_d1b0, "d1b0");
test_opcode!(op_d1b8, "d1b8");
test_opcode!(op_d1b9, "d1b9");
test_opcode!(op_d1c0, "d1c0");
test_opcode!(op_d1c8, "d1c8");
test_opcode!(op_d1d0, "d1d0");
test_opcode!(op_d1d8, "d1d8");
test_opcode!(op_d1e0, "d1e0");
test_opcode!(op_d1e8, "d1e8");
test_opcode!(op_d1f0, "d1f0");
test_opcode!(op_d1f8, "d1f8");
test_opcode!(op_d1f9, "d1f9");
test_opcode!(op_d1fa, "d1fa");
test_opcode!(op_d1fb, "d1fb");
test_opcode!(op_d1fc, "d1fc");
test_opcode!(op_e000, "e000");
test_opcode!(op_e008, "e008");
test_opcode!(op_e010, "e010");
test_opcode!(op_e018, "e018");
test_opcode!(op_e020, "e020");
test_opcode!(op_e028, "e028");
test_opcode!(op_e030, "e030");
test_opcode!(op_e038, "e038");
test_opcode!(op_e040, "e040");
test_opcode!(op_e048, "e048");
test_opcode!(op_e050, "e050");
test_opcode!(op_e058, "e058");
test_opcode!(op_e060, "e060");
test_opcode!(op_e068, "e068");
test_opcode!(op_e070, "e070");
test_opcode!(op_e078, "e078");
test_opcode!(op_e080, "e080");
test_opcode!(op_e088, "e088");
test_opcode!(op_e090, "e090");
test_opcode!(op_e098, "e098");
test_opcode!(op_e0a0, "e0a0");
test_opcode!(op_e0a8, "e0a8");
test_opcode!(op_e0b0, "e0b0");
test_opcode!(op_e0b8, "e0b8");
test_opcode!(op_e0d0, "e0d0");
test_opcode!(op_e0d8, "e0d8");
test_opcode!(op_e0e0, "e0e0");
test_opcode!(op_e0e8, "e0e8");
test_opcode!(op_e0f0, "e0f0");
test_opcode!(op_e0f8, "e0f8");
test_opcode!(op_e0f9, "e0f9");
test_opcode!(op_e100, "e100");
test_opcode!(op_e108, "e108");
test_opcode!(op_e110, "e110");
test_opcode!(op_e118, "e118");
test_opcode!(op_e120, "e120");
test_opcode!(op_e128, "e128");
test_opcode!(op_e130, "e130");
test_opcode!(op_e138, "e138");
test_opcode!(op_e140, "e140");
test_opcode!(op_e148, "e148");
test_opcode!(op_e150, "e150");
test_opcode!(op_e158, "e158");
test_opcode!(op_e160, "e160");
test_opcode!(op_e168, "e168");
test_opcode!(op_e170, "e170");
test_opcode!(op_e178, "e178");
test_opcode!(op_e180, "e180");
test_opcode!(op_e188, "e188");
test_opcode!(op_e190, "e190");
test_opcode!(op_e198, "e198");
test_opcode!(op_e1a0, "e1a0");
test_opcode!(op_e1a8, "e1a8");
test_opcode!(op_e1b0, "e1b0");
test_opcode!(op_e1b8, "e1b8");
test_opcode!(op_e1d0, "e1d0");
test_opcode!(op_e1d8, "e1d8");
test_opcode!(op_e1e0, "e1e0");
test_opcode!(op_e1e8, "e1e8");
test_opcode!(op_e1f0, "e1f0");
test_opcode!(op_e1f8, "e1f8");
test_opcode!(op_e1f9, "e1f9");
test_opcode!(op_e2d0, "e2d0");
test_opcode!(op_e2d8, "e2d8");
test_opcode!(op_e2e0, "e2e0");
test_opcode!(op_e2e8, "e2e8");
test_opcode!(op_e2f0, "e2f0");
test_opcode!(op_e2f8, "e2f8");
test_opcode!(op_e2f9, "e2f9");
test_opcode!(op_e3d0, "e3d0");
test_opcode!(op_e3d8, "e3d8");
test_opcode!(op_e3e0, "e3e0");
test_opcode!(op_e3e8, "e3e8");
test_opcode!(op_e3f0, "e3f0");
test_opcode!(op_e3f8, "e3f8");
test_opcode!(op_e3f9, "e3f9");
test_opcode!(op_e4d0, "e4d0");
test_opcode!(op_e4d8, "e4d8");
test_opcode!(op_e4e0, "e4e0");
test_opcode!(op_e4e8, "e4e8");
test_opcode!(op_e4f0, "e4f0");
test_opcode!(op_e4f8, "e4f8");
test_opcode!(op_e4f9, "e4f9");
test_opcode!(op_e5d0, "e5d0");
test_opcode!(op_e5d8, "e5d8");
test_opcode!(op_e5e0, "e5e0");
test_opcode!(op_e5e8, "e5e8");
test_opcode!(op_e5f0, "e5f0");
test_opcode!(op_e5f8, "e5f8");
test_opcode!(op_e5f9, "e5f9");
test_opcode!(op_e6d0, "e6d0");
test_opcode!(op_e6d8, "e6d8");
test_opcode!(op_e6e0, "e6e0");
test_opcode!(op_e6e8, "e6e8");
test_opcode!(op_e6f0, "e6f0");
test_opcode!(op_e6f8, "e6f8");
test_opcode!(op_e6f9, "e6f9");
test_opcode!(op_e7d0, "e7d0");
test_opcode!(op_e7d8, "e7d8");
test_opcode!(op_e7e0, "e7e0");
test_opcode!(op_e7e8, "e7e8");
test_opcode!(op_e7f0, "e7f0");
test_opcode!(op_e7f8, "e7f8");
test_opcode!(op_e7f9, "e7f9");
test_opcode!(op_f000, "f000");
