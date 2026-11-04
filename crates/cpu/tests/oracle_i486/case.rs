//! The oracle case format, the architectural view, the parser, the strict bus and the
//! one-step runner.
//!
//! A case file is a list of `key = value` lines. Empty lines and lines that start with `#`
//! are ignored. All numbers are hexadecimal with an optional `0x` prefix.
//!
//! - `name`, `mode`, `bytes`, `note` and `rules` describe the case. `mode` is `real`,
//!   `PM16`, `PM32` or `VM86` and must match the initial state.
//! - The initial state uses the field names of the architectural view (`field_name`),
//!   except the derived fields `fpu.tag_valid` and `activity`. `fpu.tag` sets the full tag
//!   word, `internal.sti_shadow` and `internal.mov_ss_shadow` set the interrupt shadows,
//!   and `internal.halted = 1` marks a step that starts in HLT and wakes on its `irq`. An
//!   omitted key keeps the value of `I386State::default()`.
//! - `memory = <address> <byte> ...` lists consecutive bytes the step may read. `in =
//!   <port> <size> <value>` lists the IN values in order. `irq = <vector>` is the vector
//!   the interrupt acknowledge returns.
//! - `expect.<field>` and `mask.<field>` give the expected value and the compared bits of
//!   a field of the architectural view. An omitted field keeps its initial value and an
//!   omitted mask compares every bit.
//! - `expect.write = <address> <byte> ...` lists the written bytes with their final
//!   values, and `expect.write_mask` the compared bits of written bytes. `expect.read =
//!   <address> <length>` lists data bytes the step must read. `expect.out = <port> <size>
//!   <value>` lists the OUT accesses in order, and `expect.interrupt = <vector>` the
//!   acknowledged interrupts.

use std::{
    cell::Cell,
    collections::{BTreeMap, VecDeque},
    fmt::Write as _,
};

use common::{Bus, Cpu};
use cpu::{CPU_MODEL_486_DX, I386, I386State};

/// The 486 core the cases run on.
type Core = I386<{ CPU_MODEL_486_DX }>;

/// The names of the six segment registers in the order of `I386State::sregs`.
pub const SEGMENT_NAMES: [&str; 6] = ["es", "cs", "ss", "ds", "fs", "gs"];
/// The names of the fields of a segment register.
const SEGMENT_PARTS: [&str; SEGMENT_FIELDS] = [
    "selector",
    "base",
    "limit",
    "rights",
    "granularity",
    "valid",
];
/// The values per segment register in the architectural view.
pub const SEGMENT_FIELDS: usize = 6;
/// The offset of the base inside the fields of a segment register.
pub const SEGMENT_BASE: usize = 1;
/// The offset of the limit inside the fields of a segment register.
pub const SEGMENT_LIMIT: usize = 2;
/// The offset of the access rights byte inside the fields of a segment register.
pub const SEGMENT_RIGHTS: usize = 3;
/// The offset of the descriptor byte 6 (G, D/B, AVL and limit bits 16 to 19).
pub const SEGMENT_GRANULARITY: usize = 4;
/// The offset of the valid flag inside the fields of a segment register.
pub const SEGMENT_VALID: usize = 5;

/// The index of EAX. ECX to EDI follow in encoding order.
pub const FIELD_EAX: usize = 0;
/// The index of ESP.
pub const FIELD_ESP: usize = 4;
/// The index of EFLAGS.
pub const FIELD_EFLAGS: usize = 9;
/// The index of the first segment register field.
pub const FIELD_SEGMENTS: usize = 10;
/// The index of the LDTR selector. The LDTR base and limit follow.
pub const FIELD_LDTR: usize = FIELD_SEGMENTS + 6 * SEGMENT_FIELDS;
/// The index of the TR selector. The TR base, limit and access byte follow.
pub const FIELD_TR: usize = FIELD_LDTR + 3;
/// The index of the GDTR base. The GDTR limit, IDTR base and IDTR limit follow.
pub const FIELD_GDTR: usize = FIELD_TR + 4;
/// The index of CR0. CR2, CR3, DR0 to DR3, DR6 and DR7 follow.
pub const FIELD_CR0: usize = FIELD_GDTR + 4;
/// The index of DR7.
pub const FIELD_DR7: usize = FIELD_CR0 + 8;
/// The index of the stored CPL.
pub const FIELD_CPL: usize = FIELD_CR0 + 9;
/// The index of the x87 control word. The status word, the abridged tag word, FIP, FCS,
/// FDP, FDS and the opcode follow.
pub const FIELD_FPU_CONTROL: usize = FIELD_CPL + 1;
/// The index of the abridged x87 tag word: one bit per physical register that is not
/// empty.
pub const FIELD_FPU_TAG: usize = FIELD_FPU_CONTROL + 2;
/// The index of the significand of the physical x87 register R0. Each register holds a
/// significand and a sign and exponent field.
pub const FIELD_FPU_REGISTERS: usize = FIELD_FPU_CONTROL + 8;
/// The index of the activity: 0 active, 1 halted, 2 shutdown, 3 waiting for x87.
pub const FIELD_ACTIVITY: usize = FIELD_FPU_REGISTERS + 16;
/// The number of fields of the architectural view.
pub const FIELD_COUNT: usize = FIELD_ACTIVITY + 1;

/// The names of the general registers.
const GENERAL_NAMES: [&str; 10] = [
    "eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi", "eip", "eflags",
];
/// The names of the descriptor table register fields.
const TABLE_NAMES: [&str; 11] = [
    "ldtr.selector",
    "ldtr.base",
    "ldtr.limit",
    "tr.selector",
    "tr.base",
    "tr.limit",
    "tr.rights",
    "gdtr.base",
    "gdtr.limit",
    "idtr.base",
    "idtr.limit",
];
/// The names of the control, debug and x87 control fields.
const CONTROL_NAMES: [&str; 18] = [
    "cr0",
    "cr2",
    "cr3",
    "dr0",
    "dr1",
    "dr2",
    "dr3",
    "dr6",
    "dr7",
    "cpl",
    "fpu.control",
    "fpu.status",
    "fpu.tag_valid",
    "fpu.fip_offset",
    "fpu.fip_selector",
    "fpu.fdp_offset",
    "fpu.fdp_selector",
    "fpu.opcode",
];

/// The tag value of an empty x87 register.
const TAG_EMPTY: u16 = 3;
/// The CR0.PE bit.
const CR0_PROTECTION_ENABLE: u32 = 1;
/// The EFLAGS.VM bit.
const EFLAGS_VIRTUAL_8086: u32 = 1 << 17;
/// The D bit of the descriptor byte 6 of a code segment.
const GRANULARITY_DEFAULT_SIZE: u8 = 0x40;
/// The activity value of a running CPU.
const ACTIVITY_ACTIVE: u64 = 0;
/// The activity value of a CPU in HLT.
const ACTIVITY_HALTED: u64 = 1;
/// The activity value of a CPU after a triple fault.
const ACTIVITY_SHUTDOWN: u64 = 2;

/// Returns the name of the field `index` of the architectural view.
pub fn field_name(index: usize) -> String {
    match index {
        _ if index < FIELD_SEGMENTS => GENERAL_NAMES[index].to_owned(),
        _ if index < FIELD_LDTR => {
            let offset = index - FIELD_SEGMENTS;
            format!(
                "{}.{}",
                SEGMENT_NAMES[offset / SEGMENT_FIELDS],
                SEGMENT_PARTS[offset % SEGMENT_FIELDS]
            )
        }
        _ if index < FIELD_CR0 => TABLE_NAMES[index - FIELD_LDTR].to_owned(),
        _ if index < FIELD_FPU_REGISTERS => CONTROL_NAMES[index - FIELD_CR0].to_owned(),
        _ if index < FIELD_ACTIVITY => {
            let offset = index - FIELD_FPU_REGISTERS;
            let part = if offset.is_multiple_of(2) {
                "significand"
            } else {
                "exponent"
            };
            format!("fpu.r{}.{part}", offset / 2)
        }
        FIELD_ACTIVITY => "activity".to_owned(),
        _ => panic!("no field {index}"),
    }
}

/// Returns the index of the field `name` of the architectural view.
pub fn field_index(name: &str) -> Option<usize> {
    (0..FIELD_COUNT).find(|index| field_name(*index) == name)
}

/// Returns the tag word with one bit per physical register that is not empty.
pub fn abridged_tag_word(tag_word: u16) -> u64 {
    (0..8)
        .filter(|register| (tag_word >> (register * 2)) & 3 != TAG_EMPTY)
        .fold(0, |abridged, register| abridged | 1 << register)
}

/// Returns the architectural view of `state` in the order of the `FIELD_*` indices.
pub fn architectural_view(state: &I386State) -> [u64; FIELD_COUNT] {
    let mut values = [0u64; FIELD_COUNT];
    let general = [
        state.eax(),
        state.ecx(),
        state.edx(),
        state.ebx(),
        state.esp(),
        state.ebp(),
        state.esi(),
        state.edi(),
        state.eip(),
        state.eflags(),
    ];
    for (index, value) in general.iter().enumerate() {
        values[FIELD_EAX + index] = u64::from(*value);
    }
    for segment in 0..6 {
        let start = FIELD_SEGMENTS + segment * SEGMENT_FIELDS;
        values[start] = u64::from(state.sregs[segment]);
        values[start + SEGMENT_BASE] = u64::from(state.seg_bases[segment]);
        values[start + SEGMENT_LIMIT] = u64::from(state.seg_limits[segment]);
        values[start + SEGMENT_RIGHTS] = u64::from(state.seg_rights[segment]);
        values[start + SEGMENT_GRANULARITY] = u64::from(state.seg_granularity[segment]);
        values[start + SEGMENT_VALID] = u64::from(state.seg_valid[segment]);
    }
    let tables = [
        u64::from(state.ldtr),
        u64::from(state.ldtr_base),
        u64::from(state.ldtr_limit),
        u64::from(state.tr),
        u64::from(state.tr_base),
        u64::from(state.tr_limit),
        u64::from(state.tr_rights),
        u64::from(state.gdt_base),
        u64::from(state.gdt_limit),
        u64::from(state.idt_base),
        u64::from(state.idt_limit),
    ];
    values[FIELD_LDTR..FIELD_CR0].copy_from_slice(&tables);
    let control = [
        u64::from(state.cr0),
        u64::from(state.cr2),
        u64::from(state.cr3),
        u64::from(state.dr0),
        u64::from(state.dr1),
        u64::from(state.dr2),
        u64::from(state.dr3),
        u64::from(state.dr6),
        u64::from(state.dr7),
        u64::from(state.stored_cpl),
        u64::from(state.fpu.control_word),
        u64::from(state.fpu.status_word),
        abridged_tag_word(state.fpu.tag_word),
        u64::from(state.fpu.fip_offset),
        u64::from(state.fpu.fip_selector),
        u64::from(state.fpu.fdp_offset),
        u64::from(state.fpu.fdp_selector),
        u64::from(state.fpu.fpu_opcode),
    ];
    values[FIELD_CR0..FIELD_FPU_REGISTERS].copy_from_slice(&control);
    for (register, value) in state.fpu.registers.iter().enumerate() {
        values[FIELD_FPU_REGISTERS + 2 * register] = value.significand();
        values[FIELD_FPU_REGISTERS + 2 * register + 1] =
            u64::from(value.exponent() | u16::from(value.sign()) << 15);
    }
    values[FIELD_ACTIVITY] = if state.shutdown() {
        ACTIVITY_SHUTDOWN
    } else if state.halted() {
        ACTIVITY_HALTED
    } else {
        ACTIVITY_ACTIVE
    };
    values
}

/// Returns the width in bits of the field `index`.
pub fn field_bits(index: usize) -> u32 {
    let segment_part = (FIELD_SEGMENTS..FIELD_LDTR)
        .contains(&index)
        .then(|| (index - FIELD_SEGMENTS) % SEGMENT_FIELDS);
    match index {
        _ if segment_part == Some(0) => 16,
        _ if segment_part == Some(SEGMENT_RIGHTS) || segment_part == Some(SEGMENT_GRANULARITY) => 8,
        _ if segment_part == Some(SEGMENT_VALID) => 1,
        _ if segment_part.is_some() => 32,
        _ if index == FIELD_LDTR || index == FIELD_TR => 16,
        _ if index == FIELD_TR + 3 => 8,
        _ if index == FIELD_GDTR + 1 || index == FIELD_GDTR + 3 => 16,
        _ if index == FIELD_CPL => 2,
        _ if index == FIELD_FPU_CONTROL || index == FIELD_FPU_CONTROL + 1 => 16,
        _ if index == FIELD_FPU_CONTROL + 4 || index == FIELD_FPU_CONTROL + 6 => 16,
        _ if index == FIELD_FPU_CONTROL + 7 => 16,
        _ if (FIELD_FPU_REGISTERS..FIELD_ACTIVITY).contains(&index) => {
            if (index - FIELD_FPU_REGISTERS).is_multiple_of(2) {
                64
            } else {
                16
            }
        }
        _ => 32,
    }
}

/// Sets the field `index` of `state` to `value`. Returns `false` for a derived field that
/// the view computes: the abridged tag word and the activity.
pub fn set_field(state: &mut I386State, index: usize, value: u64) -> bool {
    let dword = value as u32;
    let word = value as u16;
    if (FIELD_SEGMENTS..FIELD_LDTR).contains(&index) {
        let segment = (index - FIELD_SEGMENTS) / SEGMENT_FIELDS;
        match (index - FIELD_SEGMENTS) % SEGMENT_FIELDS {
            0 => state.sregs[segment] = word,
            SEGMENT_BASE => state.seg_bases[segment] = dword,
            SEGMENT_LIMIT => state.seg_limits[segment] = dword,
            SEGMENT_RIGHTS => state.seg_rights[segment] = value as u8,
            SEGMENT_GRANULARITY => state.seg_granularity[segment] = value as u8,
            _ => state.seg_valid[segment] = value != 0,
        }
        return true;
    }
    if (FIELD_FPU_REGISTERS..FIELD_ACTIVITY).contains(&index) {
        let offset = index - FIELD_FPU_REGISTERS;
        let register = &mut state.fpu.registers[offset / 2];
        let mut sign_exponent = register.exponent() | u16::from(register.sign()) << 15;
        let mut significand = register.significand();
        if offset.is_multiple_of(2) {
            significand = value;
        } else {
            sign_exponent = word;
        }
        *register = softfloat::Fp80::from_bits(sign_exponent, significand);
        return true;
    }
    match index {
        0 => state.set_eax(dword),
        1 => state.set_ecx(dword),
        2 => state.set_edx(dword),
        3 => state.set_ebx(dword),
        4 => state.set_esp(dword),
        5 => state.set_ebp(dword),
        6 => state.set_esi(dword),
        7 => state.set_edi(dword),
        8 => state.set_eip(dword),
        9 => state.set_eflags(dword),
        _ if index == FIELD_LDTR => state.ldtr = word,
        _ if index == FIELD_LDTR + 1 => state.ldtr_base = dword,
        _ if index == FIELD_LDTR + 2 => state.ldtr_limit = dword,
        _ if index == FIELD_TR => state.tr = word,
        _ if index == FIELD_TR + 1 => state.tr_base = dword,
        _ if index == FIELD_TR + 2 => state.tr_limit = dword,
        _ if index == FIELD_TR + 3 => state.tr_rights = value as u8,
        _ if index == FIELD_GDTR => state.gdt_base = dword,
        _ if index == FIELD_GDTR + 1 => state.gdt_limit = word,
        _ if index == FIELD_GDTR + 2 => state.idt_base = dword,
        _ if index == FIELD_GDTR + 3 => state.idt_limit = word,
        _ if index == FIELD_CR0 => state.cr0 = dword,
        _ if index == FIELD_CR0 + 1 => state.cr2 = dword,
        _ if index == FIELD_CR0 + 2 => state.cr3 = dword,
        _ if index == FIELD_CR0 + 3 => state.dr0 = dword,
        _ if index == FIELD_CR0 + 4 => state.dr1 = dword,
        _ if index == FIELD_CR0 + 5 => state.dr2 = dword,
        _ if index == FIELD_CR0 + 6 => state.dr3 = dword,
        _ if index == FIELD_CR0 + 7 => state.dr6 = dword,
        _ if index == FIELD_DR7 => state.dr7 = dword,
        _ if index == FIELD_CPL => state.stored_cpl = word,
        _ if index == FIELD_FPU_CONTROL => state.fpu.control_word = word,
        _ if index == FIELD_FPU_CONTROL + 1 => state.fpu.status_word = word,
        _ if index == FIELD_FPU_CONTROL + 3 => state.fpu.fip_offset = dword,
        _ if index == FIELD_FPU_CONTROL + 4 => state.fpu.fip_selector = word,
        _ if index == FIELD_FPU_CONTROL + 5 => state.fpu.fdp_offset = dword,
        _ if index == FIELD_FPU_CONTROL + 6 => state.fpu.fdp_selector = word,
        _ if index == FIELD_FPU_CONTROL + 7 => state.fpu.fpu_opcode = word,
        _ => return false,
    }
    true
}

/// Returns the mode name of `state`: real, PM16, PM32 or VM86.
pub fn mode_name(state: &I386State) -> &'static str {
    if state.cr0 & CR0_PROTECTION_ENABLE == 0 {
        "real"
    } else if state.eflags() & EFLAGS_VIRTUAL_8086 != 0 {
        "VM86"
    } else if state.seg_granularity[1] & GRANULARITY_DEFAULT_SIZE != 0 {
        "PM32"
    } else {
        "PM16"
    }
}

/// One port access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortAccess {
    /// The I/O port.
    pub port: u16,
    /// The access size in bytes: 1 or 2.
    pub size: u8,
    /// The value, zero extended.
    pub value: u32,
}

/// A parsed case file.
#[derive(Debug, Clone)]
pub struct Case {
    /// The file name, for the failure messages.
    pub file: String,
    /// The case name.
    pub name: String,
    /// The execution mode of the initial state.
    pub mode: String,
    /// The instruction bytes.
    pub bytes: Vec<u8>,
    /// The triage note.
    pub note: String,
    /// The initial CPU state.
    pub initial: I386State,
    /// The step starts in HLT and wakes on its interrupt.
    pub halted: bool,
    /// The bytes the step may read.
    pub memory: BTreeMap<u32, u8>,
    /// The IN values in order.
    pub inputs: Vec<PortAccess>,
    /// The vector of the acknowledged interrupt.
    pub irq: Option<u8>,
    /// The expected values of the architectural view by field index.
    pub expected: BTreeMap<usize, u64>,
    /// The compared bits of the architectural view by field index.
    pub masks: BTreeMap<usize, u64>,
    /// The written bytes with their final values.
    pub expected_writes: BTreeMap<u32, u8>,
    /// The compared bits of written bytes.
    pub write_masks: BTreeMap<u32, u8>,
    /// The data bytes the step must read.
    pub expected_reads: Vec<u32>,
    /// The OUT accesses in order.
    pub expected_outputs: Vec<PortAccess>,
    /// The acknowledged interrupt vectors.
    pub expected_interrupts: Vec<u8>,
}

/// The parser of one case file.
struct Parser<'a> {
    file: &'a str,
    line_number: usize,
}

impl Parser<'_> {
    /// Returns an error with the file, the line and `message`.
    fn error(&self, message: &str) -> String {
        format!("{}:{}: {message}", self.file, self.line_number)
    }

    /// Parses one hexadecimal number.
    fn number(&self, text: &str) -> Result<u64, String> {
        let digits = text
            .strip_prefix("0x")
            .or_else(|| text.strip_prefix("0X"))
            .unwrap_or(text);
        u64::from_str_radix(digits, 16)
            .map_err(|_| self.error(&format!("`{text}` is no hexadecimal number")))
    }

    /// Parses one number that fits into `bits` bits.
    fn sized(&self, text: &str, bits: u32) -> Result<u64, String> {
        let value = self.number(text)?;
        if bits < 64 && value >> bits != 0 {
            return Err(self.error(&format!("`{text}` does not fit into {bits} bits")));
        }
        Ok(value)
    }

    /// Parses the whitespace separated numbers of `value`.
    fn numbers(&self, value: &str) -> Result<Vec<u64>, String> {
        value
            .split_whitespace()
            .map(|word| self.number(word))
            .collect()
    }

    /// Parses `<address> <byte> ...` into `bytes`.
    fn byte_run(&self, value: &str, bytes: &mut BTreeMap<u32, u8>) -> Result<(), String> {
        let mut words = value.split_whitespace();
        let Some(start) = words.next() else {
            return Err(self.error("a byte run needs an address"));
        };
        let start = self.sized(start, 32)? as u32;
        for (index, word) in words.enumerate() {
            let address = start.wrapping_add(index as u32);
            let byte = self.sized(word, 8)? as u8;
            if bytes.insert(address, byte).is_some() {
                return Err(self.error(&format!("the address {address:06X} appears twice")));
            }
        }
        Ok(())
    }

    /// Parses `<port> <size> <value>`.
    fn port_access(&self, value: &str) -> Result<PortAccess, String> {
        let numbers = self.numbers(value)?;
        let [port, size, data] = numbers[..] else {
            return Err(self.error("a port access needs a port, a size and a value"));
        };
        if !matches!(size, 1 | 2) {
            return Err(self.error(&format!("the port access size {size} is not 1 or 2")));
        }
        Ok(PortAccess {
            port: u16::try_from(port).map_err(|_| self.error("the port exceeds 16 bits"))?,
            size: size as u8,
            value: u32::try_from(data).map_err(|_| self.error("the value exceeds 32 bits"))?,
        })
    }

    /// Parses a boolean written as 0 or 1.
    fn flag(&self, value: &str) -> Result<bool, String> {
        match self.number(value)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(self.error(&format!("`{value}` is not 0 or 1"))),
        }
    }

    /// Returns the view field index of `name`.
    fn field(&self, name: &str) -> Result<usize, String> {
        field_index(name)
            .ok_or_else(|| self.error(&format!("`{name}` is no field of the architectural view")))
    }
}

/// Parses the case file `file` with the contents `text`.
pub fn parse(file: &str, text: &str) -> Result<Case, String> {
    let mut case = Case {
        file: file.to_owned(),
        name: String::new(),
        mode: String::new(),
        bytes: Vec::new(),
        note: String::new(),
        initial: I386State::default(),
        halted: false,
        memory: BTreeMap::new(),
        inputs: Vec::new(),
        irq: None,
        expected: BTreeMap::new(),
        masks: BTreeMap::new(),
        expected_writes: BTreeMap::new(),
        write_masks: BTreeMap::new(),
        expected_reads: Vec::new(),
        expected_outputs: Vec::new(),
        expected_interrupts: Vec::new(),
    };
    let mut parser = Parser {
        file,
        line_number: 0,
    };
    for (index, line) in text.lines().enumerate() {
        parser.line_number = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(parser.error("a line needs the form `key = value`"));
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "name" => case.name = value.to_owned(),
            "mode" => case.mode = value.to_owned(),
            "bytes" => {
                case.bytes = value
                    .split_whitespace()
                    .map(|word| parser.sized(word, 8).map(|byte| byte as u8))
                    .collect::<Result<Vec<u8>, String>>()?;
            }
            "note" => case.note = value.to_owned(),
            "rules" => {}
            "fpu.tag" => case.initial.fpu.tag_word = parser.sized(value, 16)? as u16,
            "internal.sti_shadow" => {
                let active = parser.flag(value)?;
                case.initial.set_sti_shadow(active);
            }
            "internal.mov_ss_shadow" => {
                let active = parser.flag(value)?;
                case.initial.set_mov_ss_shadow(active);
            }
            "internal.halted" => case.halted = parser.flag(value)?,
            "memory" => parser.byte_run(value, &mut case.memory)?,
            "in" => case.inputs.push(parser.port_access(value)?),
            "irq" => {
                if case.irq.is_some() {
                    return Err(parser.error("a case has at most one irq"));
                }
                case.irq = Some(parser.sized(value, 8)? as u8);
            }
            "expect.write" => parser.byte_run(value, &mut case.expected_writes)?,
            "expect.write_mask" => parser.byte_run(value, &mut case.write_masks)?,
            "expect.read" => {
                let numbers = parser.numbers(value)?;
                let [start, length] = numbers[..] else {
                    return Err(parser.error("an expected read needs an address and a length"));
                };
                for offset in 0..length {
                    case.expected_reads
                        .push((start as u32).wrapping_add(offset as u32));
                }
            }
            "expect.out" => case.expected_outputs.push(parser.port_access(value)?),
            "expect.interrupt" => case.expected_interrupts.push(parser.sized(value, 8)? as u8),
            _ => {
                if let Some(field) = key.strip_prefix("expect.") {
                    let index = parser.field(field)?;
                    case.expected
                        .insert(index, parser.sized(value, field_bits(index))?);
                } else if let Some(field) = key.strip_prefix("mask.") {
                    let index = parser.field(field)?;
                    case.masks
                        .insert(index, parser.sized(value, field_bits(index))?);
                } else {
                    let index = field_index(key)
                        .ok_or_else(|| parser.error(&format!("unknown key `{key}`")))?;
                    let number = parser.sized(value, field_bits(index))?;
                    if !set_field(&mut case.initial, index, number) {
                        return Err(parser.error(&format!(
                            "`{key}` is derived and cannot be set in the initial state"
                        )));
                    }
                }
            }
        }
    }
    for address in case.write_masks.keys() {
        if !case.expected_writes.contains_key(address) {
            return Err(format!(
                "{file}: the write mask at {address:06X} has no expected write"
            ));
        }
    }
    if case.halted && case.irq.is_none() {
        return Err(format!(
            "{file}: a step that starts in HLT needs an irq to wake the CPU"
        ));
    }
    Ok(case)
}

/// A bus that serves only the bytes, IN values and interrupt of a case and records the
/// traffic of the step.
pub struct StrictBus<'a> {
    case: &'a Case,
    memory: BTreeMap<u32, u8>,
    /// The bytes written, in order.
    pub writes: Vec<(u32, u8)>,
    /// The addresses read, in order.
    pub reads: Vec<u32>,
    inputs: VecDeque<PortAccess>,
    /// The accesses outside the case: unlisted reads, IN mismatches and acknowledges
    /// without an irq.
    pub violations: Vec<String>,
    /// The IN accesses that did not match the next IN value.
    pub input_mismatches: Vec<String>,
    /// The OUT accesses in order.
    pub outputs: Vec<PortAccess>,
    irq_pending: bool,
    /// The vectors the interrupt acknowledge returned.
    pub acknowledged_interrupts: Vec<u8>,
    /// The non-maskable interrupt acknowledges.
    pub acknowledged_nmis: u32,
    cycle: u64,
    /// The `reset_pending` calls. The core asks once after each `execute_one`.
    pub executed_instructions: Cell<u32>,
}

impl<'a> StrictBus<'a> {
    /// Creates the bus of `case`.
    pub fn new(case: &'a Case) -> Self {
        Self {
            case,
            memory: case.memory.clone(),
            writes: Vec::new(),
            reads: Vec::new(),
            inputs: case.inputs.iter().copied().collect(),
            violations: Vec::new(),
            input_mismatches: Vec::new(),
            outputs: Vec::new(),
            irq_pending: case.irq.is_some(),
            acknowledged_interrupts: Vec::new(),
            acknowledged_nmis: 0,
            cycle: 0,
            executed_instructions: Cell::new(0),
        }
    }

    /// Returns the IN values the step did not read.
    pub fn remaining_inputs(&self) -> Vec<PortAccess> {
        self.inputs.iter().copied().collect()
    }

    /// Serves an IN of `size` bytes from `port` with the next IN value.
    fn input(&mut self, port: u16, size: u8) -> u32 {
        let Some(access) = self.inputs.pop_front() else {
            self.violations
                .push(format!("IN {port:04X}/{size} without a listed IN value"));
            return 0xFFFF;
        };
        if access.port != port || access.size != size {
            self.input_mismatches.push(format!(
                "IN {port:04X}/{size}, listed {:04X}/{}",
                access.port, access.size
            ));
        }
        access.value
    }
}

impl Bus for StrictBus<'_> {
    fn read_byte(&mut self, address: u32) -> u8 {
        let Some(value) = self.memory.get(&address).copied() else {
            self.violations
                .push(format!("read of the unlisted address {address:06X}"));
            return 0xFF;
        };
        self.reads.push(address);
        value
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.writes.push((address, value));
        self.memory.insert(address, value);
    }

    fn io_read_byte(&mut self, port: u16) -> u8 {
        self.input(port, 1) as u8
    }

    fn io_write_byte(&mut self, port: u16, value: u8) {
        self.outputs.push(PortAccess {
            port,
            size: 1,
            value: u32::from(value),
        });
    }

    fn io_read_word(&mut self, port: u16) -> u16 {
        self.input(port, 2) as u16
    }

    fn io_write_word(&mut self, port: u16, value: u16) {
        self.outputs.push(PortAccess {
            port,
            size: 2,
            value: u32::from(value),
        });
    }

    fn has_irq(&self) -> bool {
        self.irq_pending
    }

    fn acknowledge_irq(&mut self) -> u8 {
        let Some(vector) = self.case.irq.filter(|_| self.irq_pending) else {
            self.violations
                .push("interrupt acknowledge without a listed irq".to_owned());
            return 0;
        };
        self.irq_pending = false;
        self.acknowledged_interrupts.push(vector);
        vector
    }

    fn has_nmi(&self) -> bool {
        false
    }

    fn acknowledge_nmi(&mut self) {
        self.acknowledged_nmis += 1;
    }

    fn current_cycle(&self) -> u64 {
        self.cycle
    }

    fn set_current_cycle(&mut self, cycle: u64) {
        self.cycle = cycle;
    }

    fn reset_pending(&self) -> bool {
        self.executed_instructions
            .set(self.executed_instructions.get() + 1);
        false
    }
}

/// One field in which the step and the case disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    /// The field name.
    pub name: String,
    /// The value of the step.
    pub ours: String,
    /// The value of the case.
    pub expected: String,
}

/// Formats `(address, value)` pairs for a difference.
fn format_bytes(bytes: &[(u32, u8)]) -> String {
    bytes
        .iter()
        .map(|(address, value)| format!("{address:06X}={value:02X}"))
        .collect::<Vec<String>>()
        .join(" ")
}

/// Formats port accesses for a difference.
fn format_ports(accesses: &[PortAccess]) -> String {
    accesses
        .iter()
        .map(|access| format!("{:04X}/{}={:X}", access.port, access.size, access.value))
        .collect::<Vec<String>>()
        .join(" ")
}

/// Runs the step of `case` and returns the fields in which the result differs from the
/// expectation. A step that leaves the case, such as a read of an unlisted address, is an
/// error.
pub fn run(case: &Case) -> Result<Vec<Difference>, String> {
    let file = &case.file;
    let initial_mode = mode_name(&case.initial);
    if !initial_mode.eq_ignore_ascii_case(&case.mode) {
        return Err(format!(
            "{file}: the mode is `{}`, the initial state is in {initial_mode}",
            case.mode
        ));
    }
    let mut core = Core::new();
    core.restore_state(case.initial.clone())
        .map_err(|error| format!("{file}: the initial state is invalid: {error:?}"))?;
    let mut bus = StrictBus::new(case);
    let cycles = core.run_for(1, &mut bus);
    if !bus.violations.is_empty() {
        return Err(format!("{file}: {}", bus.violations.join(", ")));
    }
    let executed = bus.executed_instructions.get();
    if cycles == 0 || executed != 1 {
        return Err(format!(
            "{file}: the step ran {executed} instructions in {cycles} cycles, expected one"
        ));
    }
    if bus.acknowledged_nmis != 0 {
        return Err(format!("{file}: the step took an NMI"));
    }
    let post_state = core.capture_state();

    let mut differences = Vec::new();
    let initial_view = architectural_view(&case.initial);
    let post_view = architectural_view(&post_state);
    for index in 0..FIELD_COUNT {
        let ours = post_view[index];
        let expected = case
            .expected
            .get(&index)
            .copied()
            .unwrap_or(initial_view[index]);
        let mask = case.masks.get(&index).copied().unwrap_or(u64::MAX);
        if (ours ^ expected) & mask != 0 {
            differences.push(Difference {
                name: field_name(index),
                ours: format!("{ours:#X}"),
                expected: format!("{expected:#X}"),
            });
        }
    }

    let mut written: BTreeMap<u32, u8> = BTreeMap::new();
    for (address, value) in &bus.writes {
        written.insert(*address, *value);
    }
    let byte_mask = |address: &u32| case.write_masks.get(address).copied().unwrap_or(0xFF);
    let only_ours: Vec<(u32, u8)> = written
        .iter()
        .filter(|(address, value)| {
            case.expected_writes
                .get(address)
                .is_none_or(|expected| (*expected ^ **value) & byte_mask(address) != 0)
        })
        .map(|(address, value)| (*address, *value))
        .collect();
    let only_expected: Vec<(u32, u8)> = case
        .expected_writes
        .iter()
        .filter(|(address, expected)| {
            written
                .get(address)
                .is_none_or(|value| (**expected ^ *value) & byte_mask(address) != 0)
        })
        .map(|(address, value)| (*address, *value))
        .collect();
    if !only_ours.is_empty() || !only_expected.is_empty() {
        differences.push(Difference {
            name: "memory.write".to_owned(),
            ours: format_bytes(&only_ours),
            expected: format_bytes(&only_expected),
        });
    }

    let missing_reads: Vec<(u32, u8)> = case
        .expected_reads
        .iter()
        .filter(|address| !bus.reads.contains(address))
        .map(|address| (*address, case.memory.get(address).copied().unwrap_or(0)))
        .collect();
    if !missing_reads.is_empty() {
        differences.push(Difference {
            name: "memory.read".to_owned(),
            ours: String::new(),
            expected: format_bytes(&missing_reads),
        });
    }

    if bus.outputs != case.expected_outputs {
        differences.push(Difference {
            name: "io.out".to_owned(),
            ours: format_ports(&bus.outputs),
            expected: format_ports(&case.expected_outputs),
        });
    }
    let remaining = bus.remaining_inputs();
    if !bus.input_mismatches.is_empty() || !remaining.is_empty() {
        differences.push(Difference {
            name: "io.in".to_owned(),
            ours: bus.input_mismatches.join(", "),
            expected: format!("left over: {}", format_ports(&remaining)),
        });
    }
    if bus.acknowledged_interrupts != case.expected_interrupts {
        differences.push(Difference {
            name: "interrupt".to_owned(),
            ours: format!("{:02X?}", bus.acknowledged_interrupts),
            expected: format!("{:02X?}", case.expected_interrupts),
        });
    }
    Ok(differences)
}

/// Parses the case file `file` with the contents `text`, runs it and panics with the file
/// name and the first differing field when the result differs from the expectation.
pub fn check(file: &str, text: &str) {
    let case = parse(file, text).unwrap_or_else(|error| panic!("{error}"));
    let differences = run(&case).unwrap_or_else(|error| panic!("{error}"));
    let Some(first) = differences.first() else {
        return;
    };
    let mut message = format!(
        "{file}: first differing field `{}`: ours {} expected {}\n",
        first.name, first.ours, first.expected
    );
    for difference in &differences {
        let _ = writeln!(
            message,
            "  {:<16} ours {:<40} expected {}",
            difference.name, difference.ours, difference.expected
        );
    }
    let _ = write!(message, "note: {}", case.note);
    panic!("{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `inc ax; nop` at 1000:0100 in real mode.
    const INC_AX: &str = "
        name = inc_ax
        mode = real
        eax = 0x7FFF
        eip = 0x100
        eflags = 0x2
        cs.selector = 0x1000
        cs.base = 0x10000
        cs.limit = 0xFFFF
        cs.rights = 0x9B
        cs.valid = 1
        memory = 010100 40 90
        expect.eax = 0x8000
        expect.eip = 0x101
        expect.eflags = 0x896
    ";

    /// Verifies that a parsed case runs and matches its expectation.
    #[test]
    fn a_matching_case_has_no_differences() {
        let case = parse("inc_ax", INC_AX).unwrap();
        assert_eq!(case.memory.len(), 2);
        assert_eq!(run(&case), Ok(Vec::new()));
    }

    /// Verifies that a wrong expectation names the field.
    #[test]
    fn a_wrong_expectation_names_the_field() {
        let text = INC_AX.replace("expect.eax = 0x8000", "expect.eax = 0x8001");
        let differences = run(&parse("inc_ax", &text).unwrap()).unwrap();
        assert_eq!(differences.len(), 1);
        assert_eq!(differences[0].name, "eax");
    }

    /// Verifies that a read of an unlisted address fails with the address.
    #[test]
    fn an_unlisted_read_fails() {
        let text = INC_AX.replace("memory = 010100 40 90", "memory = 010100 40");
        let error = run(&parse("inc_ax", &text).unwrap()).unwrap_err();
        assert!(
            error.contains("read of the unlisted address 010101"),
            "{error}"
        );
    }

    /// Verifies that an unknown key fails with the line.
    #[test]
    fn an_unknown_key_fails_with_the_line() {
        let error = parse("inc_ax", "name = inc_ax\n\neaz = 1\n").unwrap_err();
        assert_eq!(error, "inc_ax:3: unknown key `eaz`");
    }

    /// Verifies that every field name maps back to its index and that the field setter
    /// reaches the view.
    #[test]
    fn field_names_and_setters_follow_the_view() {
        let mut state = I386State::default();
        for index in 0..FIELD_COUNT {
            assert_eq!(field_index(&field_name(index)), Some(index));
            let value = 1u64 << (field_bits(index) - 1);
            if set_field(&mut state, index, value) && index != FIELD_EFLAGS {
                assert_eq!(
                    architectural_view(&state)[index],
                    value,
                    "{}",
                    field_name(index)
                );
            }
        }
        assert_eq!(field_name(FIELD_SEGMENTS + 11), "cs.valid");
        assert_eq!(field_name(FIELD_TR + 3), "tr.rights");
        assert_eq!(field_name(FIELD_FPU_TAG), "fpu.tag_valid");
        assert_eq!(field_name(FIELD_FPU_REGISTERS + 15), "fpu.r7.exponent");
        assert_eq!(field_name(FIELD_ACTIVITY), "activity");
    }
}
