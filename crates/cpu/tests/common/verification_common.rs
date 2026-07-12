#![cfg(feature = "verification")]
#![allow(dead_code)]

use std::{collections::HashMap, fmt::Write, fs, path::Path};

use zlib_rs::{InflateConfig, ReturnCode, decompress_slice};

#[derive(Debug, Clone)]
pub struct MooState {
    pub regs: HashMap<String, u32>,
    pub ram: Vec<(u32, u8)>,
    pub queue: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MooLegacyCycle {
    pub address: Option<u16>,
    pub data: Option<u8>,
    pub status: [u8; 4],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MooI286Cycle {
    pub pin_bitfield: u8,
    pub address: u32,
    pub memory_status: u8,
    pub io_status: u8,
    pub bhe_status: u8,
    pub data_bus: u16,
    pub bus_status: String,
    pub raw_bus_status: u8,
    pub t_state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MooI386Cycle {
    pub pin_bitfield: u8,
    pub address: u32,
    pub memory_status: u8,
    pub io_status: u8,
    pub bhe_status: u8,
    pub data_bus: u16,
    pub bus_status: String,
    pub raw_bus_status: u8,
    pub t_state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MooV20Cycle {
    pub pin_bitfield: u8,
    pub bus_value: u32,
    pub segment_status: u8,
    pub memory_status: u8,
    pub io_status: u8,
    pub data: u8,
    pub raw_bus_status: u8,
    pub bus_status: String,
    pub raw_t_state: u8,
    pub t_state: String,
    pub raw_queue_op: u8,
    pub queue_op: String,
    pub queue_byte: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MooCycle {
    Legacy(MooLegacyCycle),
    I286(MooI286Cycle),
    I386(MooI386Cycle),
    V20(MooV20Cycle),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MooPort {
    pub address: u16,
    pub value: u8,
    pub direction: u8,
}

#[cfg(feature = "verification")]
#[derive(Debug, Clone)]
pub struct MooException {
    pub number: u8,
    pub flag_address: u32,
}

#[derive(Debug, Clone)]
pub struct MooTest {
    pub idx: u32,
    pub name: String,
    pub bytes: Vec<u8>,
    pub initial: MooState,
    pub final_state: MooState,
    pub cycles: Vec<MooCycle>,
    pub ports: Vec<MooPort>,
    pub exception: Option<MooException>,
    pub hash: Option<String>,
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

fn parse_regs16(payload: &[u8], reg_order: &[&str]) -> HashMap<String, u32> {
    let mut offset = 0;
    let mask = read_u16(payload, &mut offset);
    let mut regs = HashMap::new();

    for (index, name) in reg_order.iter().enumerate() {
        if mask & (1 << index) != 0 {
            let value = read_u16(payload, &mut offset) as u32;
            regs.insert((*name).to_string(), value);
        }
    }

    regs
}

fn parse_regs32(payload: &[u8], reg_order: &[&str]) -> HashMap<String, u32> {
    let mut offset = 0;
    let mask = read_u32(payload, &mut offset);
    let mut regs = HashMap::new();

    for (index, name) in reg_order.iter().enumerate() {
        if mask & (1 << index) != 0 {
            let value = read_u32(payload, &mut offset);
            regs.insert((*name).to_string(), value);
        }
    }

    regs
}

fn parse_ram(payload: &[u8]) -> Vec<(u32, u8)> {
    let mut offset = 0;
    let count = read_u32(payload, &mut offset) as usize;
    let mut entries = Vec::with_capacity(count);

    for _ in 0..count {
        let address = read_u32(payload, &mut offset);
        let value = payload[offset];
        offset += 1;
        entries.push((address, value));
    }

    entries
}

fn parse_queue(payload: &[u8]) -> Vec<u8> {
    let mut offset = 0;
    let count = read_u32(payload, &mut offset) as usize;
    payload[offset..offset + count].to_vec()
}

fn decode_i286_bus_status(raw_bus_status: u8) -> String {
    match raw_bus_status & 0x0F {
        0x0 => "IRQA",
        0x4 => "HALT",
        0x5 => "MEMR",
        0x6 => "MEMW",
        0x9 => "IOR",
        0xA => "IOW",
        0xD => "CODE",
        _ => "PASV",
    }
    .to_string()
}

fn decode_i286_t_state(raw_t_state: u8) -> String {
    match raw_t_state & 0x07 {
        1 => "Ts",
        2 => "Tc",
        _ => "Ti",
    }
    .to_string()
}

fn decode_i386_bus_status(raw_bus_status: u8) -> String {
    match raw_bus_status & 0x07 {
        0 => "INTA",
        2 => "IOR",
        3 => "IOW",
        4 => "CODE",
        5 => "HALT",
        6 => "MEMR",
        7 => "MEMW",
        _ => "PASV",
    }
    .to_string()
}

fn decode_i386_t_state(raw_t_state: u8) -> String {
    match raw_t_state & 0x07 {
        1 => "T1",
        2 => "T2",
        _ => "Ti",
    }
    .to_string()
}

fn decode_v20_bus_status(raw_bus_status: u8) -> String {
    match raw_bus_status & 0x07 {
        0 => "INTA",
        1 => "IOR",
        2 => "IOW",
        3 => "MEMR",
        4 => "MEMW",
        5 => "HALT",
        6 => "CODE",
        _ => "PASV",
    }
    .to_string()
}

fn decode_v20_t_state(raw_t_state: u8) -> String {
    match raw_t_state & 0x07 {
        1 => "T1",
        2 => "T2",
        3 => "T3",
        4 => "T4",
        5 => "Tw",
        _ => "Ti",
    }
    .to_string()
}

fn decode_v20_queue_op(raw_queue_op: u8) -> String {
    match raw_queue_op & 0x03 {
        1 => "F",
        2 => "E",
        3 => "S",
        _ => "-",
    }
    .to_string()
}

fn parse_cycles(payload: &[u8], cpu_name: &str) -> Vec<MooCycle> {
    let mut offset = 0;
    let count = read_u32(payload, &mut offset) as usize;
    let mut cycles = Vec::with_capacity(count);

    if cpu_name.contains("V20") {
        for _ in 0..count {
            let pin_bitfield = payload[offset];
            offset += 1;
            let bus_value = read_u32(payload, &mut offset);
            let segment_status = payload[offset];
            offset += 1;
            let memory_status = payload[offset];
            offset += 1;
            let io_status = payload[offset];
            offset += 1;
            offset += 1;
            let data = payload[offset];
            offset += 1;
            offset += 1;
            let raw_bus_status = payload[offset];
            offset += 1;
            let raw_t_state = payload[offset];
            offset += 1;
            let raw_queue_op = payload[offset];
            offset += 1;
            let queue_byte = payload[offset];
            offset += 1;

            cycles.push(MooCycle::V20(MooV20Cycle {
                pin_bitfield,
                bus_value,
                segment_status,
                memory_status,
                io_status,
                data,
                raw_bus_status,
                bus_status: decode_v20_bus_status(raw_bus_status),
                raw_t_state,
                t_state: decode_v20_t_state(raw_t_state),
                raw_queue_op,
                queue_op: decode_v20_queue_op(raw_queue_op),
                queue_byte,
            }));
        }

        return cycles;
    }

    if cpu_name.contains("386") {
        for _ in 0..count {
            let pin_bitfield = payload[offset];
            offset += 1;
            let address = read_u32(payload, &mut offset);
            offset += 1;
            let memory_status = payload[offset];
            offset += 1;
            let io_status = payload[offset];
            offset += 1;
            let bhe_status = payload[offset];
            offset += 1;
            let data_bus = read_u16(payload, &mut offset);
            let raw_bus_status = payload[offset];
            offset += 1;
            let raw_t_state = payload[offset];
            offset += 1;
            offset += 2;

            cycles.push(MooCycle::I386(MooI386Cycle {
                pin_bitfield,
                address,
                memory_status,
                io_status,
                bhe_status,
                data_bus,
                bus_status: decode_i386_bus_status(raw_bus_status),
                raw_bus_status,
                t_state: decode_i386_t_state(raw_t_state),
            }));
        }

        return cycles;
    }

    if cpu_name.contains("286") {
        for _ in 0..count {
            let pin_bitfield = payload[offset];
            offset += 1;
            let address = read_u32(payload, &mut offset);
            offset += 1;
            let memory_status = payload[offset];
            offset += 1;
            let io_status = payload[offset];
            offset += 1;
            let bhe_status = payload[offset];
            offset += 1;
            let data_bus = read_u16(payload, &mut offset);
            let raw_bus_status = payload[offset];
            offset += 1;
            let raw_t_state = payload[offset];
            offset += 1;
            offset += 2;

            cycles.push(MooCycle::I286(MooI286Cycle {
                pin_bitfield,
                address,
                memory_status,
                io_status,
                bhe_status,
                data_bus,
                bus_status: decode_i286_bus_status(raw_bus_status),
                raw_bus_status,
                t_state: decode_i286_t_state(raw_t_state),
            }));
        }

        return cycles;
    }

    for _ in 0..count {
        let pin_bitfield = payload[offset];
        offset += 1;
        let address = read_u16(payload, &mut offset);
        let data = payload[offset];
        offset += 1;
        let status = read_tag(payload, &mut offset);

        cycles.push(MooCycle::Legacy(MooLegacyCycle {
            address: if pin_bitfield & 0x01 != 0 {
                Some(address)
            } else {
                None
            },
            data: if pin_bitfield & 0x02 != 0 {
                Some(data)
            } else {
                None
            },
            status,
        }));
    }

    cycles
}

fn parse_ports(payload: &[u8]) -> Vec<MooPort> {
    let mut offset = 0;
    let count = read_u32(payload, &mut offset) as usize;
    let mut ports = Vec::with_capacity(count);

    for _ in 0..count {
        let address = read_u16(payload, &mut offset);
        let value = payload[offset];
        offset += 1;
        let direction = payload[offset];
        offset += 1;
        ports.push(MooPort {
            address,
            value,
            direction,
        });
    }

    ports
}

fn parse_cpu_state(payload: &[u8], reg_order16: &[&str], reg_order32: &[&str]) -> MooState {
    let mut offset = 0;
    let mut regs = HashMap::new();
    let mut ram = Vec::new();
    let mut queue = Vec::new();

    while offset < payload.len() {
        let tag = read_tag(payload, &mut offset);
        let length = read_u32(payload, &mut offset) as usize;
        let end = offset + length;
        let sub_payload = &payload[offset..end];

        match &tag {
            b"REGS" => regs = parse_regs16(sub_payload, reg_order16),
            b"RG32" => regs = parse_regs32(sub_payload, reg_order32),
            b"RAM " => ram = parse_ram(sub_payload),
            b"QUEU" => queue = parse_queue(sub_payload),
            b"EA32" => {}
            _ => {}
        }

        offset = end;
    }

    MooState { regs, ram, queue }
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn parse_test_chunk(
    payload: &[u8],
    reg_order16: &[&str],
    reg_order32: &[&str],
    cpu_name: &str,
) -> MooTest {
    let mut offset = 0;
    let idx = read_u32(payload, &mut offset);
    let mut name = String::new();
    let mut bytes = Vec::new();
    let mut initial = MooState {
        regs: HashMap::new(),
        ram: Vec::new(),
        queue: Vec::new(),
    };
    let mut final_state = MooState {
        regs: HashMap::new(),
        ram: Vec::new(),
        queue: Vec::new(),
    };
    let mut cycles = Vec::new();
    let mut ports = Vec::new();
    let mut exception = None;
    let mut hash = None;

    while offset < payload.len() {
        let tag = read_tag(payload, &mut offset);
        let length = read_u32(payload, &mut offset) as usize;
        let end = offset + length;
        let sub_payload = &payload[offset..end];

        match &tag {
            b"NAME" => {
                let mut name_offset = 0;
                let name_len = read_u32(sub_payload, &mut name_offset) as usize;
                name = String::from_utf8(sub_payload[name_offset..name_offset + name_len].to_vec())
                    .unwrap();
            }
            b"BYTS" => {
                let mut bytes_offset = 0;
                let byte_count = read_u32(sub_payload, &mut bytes_offset) as usize;
                bytes = sub_payload[bytes_offset..bytes_offset + byte_count].to_vec();
            }
            b"INIT" => initial = parse_cpu_state(sub_payload, reg_order16, reg_order32),
            b"FINA" => final_state = parse_cpu_state(sub_payload, reg_order16, reg_order32),
            b"CYCL" => cycles = parse_cycles(sub_payload, cpu_name),
            b"PORT" => ports = parse_ports(sub_payload),
            b"EXCP" if !sub_payload.is_empty() => {
                let mut exception_offset = 0;
                let number = sub_payload[exception_offset];
                exception_offset += 1;
                let flag_address = read_u32(sub_payload, &mut exception_offset);
                exception = Some(MooException {
                    number,
                    flag_address,
                });
            }
            b"EXCP" => {}
            b"HASH" => {
                hash = Some(bytes_to_hex(sub_payload));
            }
            b"GMET" => {}
            _ => {}
        }

        offset = end;
    }

    MooTest {
        idx,
        name,
        bytes,
        initial,
        final_state,
        cycles,
        ports,
        exception,
        hash,
    }
}

fn read_gzip_to_vec(path: &Path) -> Vec<u8> {
    let compressed = fs::read(path).unwrap();
    assert!(
        compressed.len() >= 18,
        "{path:?}: file too short to be a valid gzip stream"
    );
    let isize_le: [u8; 4] = compressed[compressed.len() - 4..].try_into().unwrap();
    let output_len = u32::from_le_bytes(isize_le) as usize;

    let mut output = vec![0u8; output_len];
    let (decompressed, code) =
        decompress_slice(&mut output, &compressed, InflateConfig { window_bits: 31 });
    assert_eq!(code, ReturnCode::Ok, "{path:?}: inflate failed ({code:?})");
    assert_eq!(
        decompressed.len(),
        output_len,
        "{path:?}: decoded size does not match gzip ISIZE"
    );
    output
}

pub fn load_moo_tests(path: &Path, reg_order16: &[&str], reg_order32: &[&str]) -> Vec<MooTest> {
    let data = read_gzip_to_vec(path);

    let mut offset = 0;
    assert_eq!(&data[0..4], b"MOO ");
    offset += 4;

    let header_len = read_u32(&data, &mut offset) as usize;
    let header = &data[offset..offset + header_len];
    let cpu_name = std::str::from_utf8(&header[8..12]).unwrap().trim_end();
    offset += header_len;

    let mut tests = Vec::new();
    while offset < data.len() {
        let tag = read_tag(&data, &mut offset);
        let chunk_len = read_u32(&data, &mut offset) as usize;
        let end = offset + chunk_len;

        if &tag == b"TEST" {
            tests.push(parse_test_chunk(
                &data[offset..end],
                reg_order16,
                reg_order32,
                cpu_name,
            ));
        }

        offset = end;
    }

    tests
}

pub fn load_revocation_list(path: &Path) -> std::collections::HashSet<String> {
    let text = fs::read_to_string(path).unwrap();
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                None
            } else {
                Some(trimmed.to_ascii_lowercase())
            }
        })
        .collect()
}
