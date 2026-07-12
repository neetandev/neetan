//! Shared helpers for the i486-manual-derived test corpus.
//!
//! Identifiers are spelled out in full and constants are named after the
//! manual sections they encode so tests do not embed magic hex.

use cpu::{CPU_MODEL_386_DX, CPU_MODEL_486_DX, I386, I386State};

// Memory layout used by setup_protected_mode and friends. Bases are chosen so
// that no two regions overlap inside a 1 MiB test bus.

pub(crate) const TEST_BUS_RAM_BYTES: usize = 1024 * 1024;
pub(crate) const TEST_BUS_ADDRESS_MASK: u32 = 0x000F_FFFF;

pub(crate) const GLOBAL_DESCRIPTOR_TABLE_BASE: u32 = 0x0008_0000;
pub(crate) const INTERRUPT_DESCRIPTOR_TABLE_BASE: u32 = 0x0009_0000;
pub(crate) const TASK_STATE_SEGMENT_BASE: u32 = 0x0007_0000;
pub(crate) const TASK_STATE_SEGMENT_SECONDARY_BASE: u32 = 0x0007_1000;
pub(crate) const RING0_CODE_BASE: u32 = 0x0005_0000;
pub(crate) const RING3_CODE_BASE: u32 = 0x0006_0000;
pub(crate) const SHARED_DATA_BASE: u32 = 0x0001_0000;
pub(crate) const RING0_STACK_BASE: u32 = 0x0000_0000;
pub(crate) const RING3_STACK_BASE: u32 = 0x0002_0000;
pub(crate) const PAGE_DIRECTORY_BASE: u32 = 0x000A_0000;
pub(crate) const PAGE_TABLE_BASE: u32 = 0x000A_1000;

// Selectors. Slot index is selector >> 3; the low two bits hold the RPL.

pub(crate) const SELECTOR_RING0_CODE: u16 = 0x0008;
pub(crate) const SELECTOR_RING0_DATA: u16 = 0x0010;
pub(crate) const SELECTOR_RING0_STACK: u16 = 0x0018;
pub(crate) const SELECTOR_RING3_CODE: u16 = 0x0023;
pub(crate) const SELECTOR_RING3_DATA: u16 = 0x002B;
pub(crate) const SELECTOR_RING3_STACK: u16 = 0x0033;
pub(crate) const SELECTOR_PRIMARY_TSS: u16 = 0x0038;
pub(crate) const SELECTOR_SECONDARY_TSS: u16 = 0x0048;

// Default exception-handler entry points (offsets within the ring-0 code
// segment). Each handler is installed by setup_protected_mode_with_handlers
// as a single HLT byte: a test that triggers a fault checks `cpu.halted()`
// and asserts `cpu.ip()` against (handler IP + 1).

pub(crate) const HANDLER_DIVIDE_ERROR_IP: u16 = 0x7000;
pub(crate) const HANDLER_DEBUG_IP: u16 = 0x7100;
pub(crate) const HANDLER_BREAKPOINT_IP: u16 = 0x7200;
pub(crate) const HANDLER_OVERFLOW_IP: u16 = 0x7300;
pub(crate) const HANDLER_BOUND_RANGE_IP: u16 = 0x7400;
pub(crate) const HANDLER_INVALID_OPCODE_IP: u16 = 0x7500;
pub(crate) const HANDLER_DEVICE_NOT_AVAILABLE_IP: u16 = 0x7600;
pub(crate) const HANDLER_DOUBLE_FAULT_IP: u16 = 0x7700;
pub(crate) const HANDLER_INVALID_TSS_IP: u16 = 0x7800;
pub(crate) const HANDLER_SEGMENT_NOT_PRESENT_IP: u16 = 0x7900;
pub(crate) const HANDLER_STACK_FAULT_IP: u16 = 0x7A00;
pub(crate) const HANDLER_GENERAL_PROTECTION_IP: u16 = 0x8000;
pub(crate) const HANDLER_PAGE_FAULT_IP: u16 = 0x8100;
pub(crate) const HANDLER_ALIGNMENT_CHECK_IP: u16 = 0x8200;

// Access-rights byte components. Intel 80486 PRM Chapter 6 ("Protection")
// for code/data; Table 6-1 for system descriptor types.

pub(crate) const ACCESS_PRESENT: u8 = 0x80;
pub(crate) const ACCESS_DPL_RING0: u8 = 0 << 5;
pub(crate) const ACCESS_DPL_RING3: u8 = 3 << 5;
pub(crate) const ACCESS_DESCRIPTOR_CODE_OR_DATA: u8 = 0x10;
pub(crate) const ACCESS_DESCRIPTOR_SYSTEM: u8 = 0x00;
pub(crate) const ACCESS_TYPE_CODE: u8 = 0x08;
pub(crate) const ACCESS_TYPE_CODE_CONFORMING: u8 = 0x04;
pub(crate) const ACCESS_TYPE_CODE_READABLE: u8 = 0x02;
pub(crate) const ACCESS_TYPE_DATA_WRITABLE: u8 = 0x02;
pub(crate) const ACCESS_TYPE_ACCESSED: u8 = 0x01;

// System-descriptor type codes (the low 4 bits of the access byte when
// ACCESS_DESCRIPTOR_SYSTEM is set). Intel 80486 PRM Table 6-1.

pub(crate) const SYSTEM_TYPE_TSS_286_AVAILABLE: u8 = 1;
pub(crate) const SYSTEM_TYPE_LDT: u8 = 2;
pub(crate) const SYSTEM_TYPE_TSS_286_BUSY: u8 = 3;
pub(crate) const SYSTEM_TYPE_CALL_GATE_286: u8 = 4;
pub(crate) const SYSTEM_TYPE_TASK_GATE: u8 = 5;
pub(crate) const SYSTEM_TYPE_INTERRUPT_GATE_286: u8 = 6;
pub(crate) const SYSTEM_TYPE_TRAP_GATE_286: u8 = 7;
pub(crate) const SYSTEM_TYPE_TSS_386_AVAILABLE: u8 = 9;
pub(crate) const SYSTEM_TYPE_TSS_386_BUSY: u8 = 11;
pub(crate) const SYSTEM_TYPE_CALL_GATE_386: u8 = 12;
pub(crate) const SYSTEM_TYPE_INTERRUPT_GATE_386: u8 = 14;
pub(crate) const SYSTEM_TYPE_TRAP_GATE_386: u8 = 15;

// Common precomputed access-rights bytes for code and data segments.

pub(crate) const RIGHTS_RING0_CODE_READABLE_ACCESSED: u8 = ACCESS_PRESENT
    | ACCESS_DPL_RING0
    | ACCESS_DESCRIPTOR_CODE_OR_DATA
    | ACCESS_TYPE_CODE
    | ACCESS_TYPE_CODE_READABLE
    | ACCESS_TYPE_ACCESSED;
pub(crate) const RIGHTS_RING0_CODE_CONFORMING_READABLE_ACCESSED: u8 =
    RIGHTS_RING0_CODE_READABLE_ACCESSED | ACCESS_TYPE_CODE_CONFORMING;
pub(crate) const RIGHTS_RING0_DATA_WRITABLE_ACCESSED: u8 = ACCESS_PRESENT
    | ACCESS_DPL_RING0
    | ACCESS_DESCRIPTOR_CODE_OR_DATA
    | ACCESS_TYPE_DATA_WRITABLE
    | ACCESS_TYPE_ACCESSED;
pub(crate) const RIGHTS_RING3_CODE_READABLE_ACCESSED: u8 = ACCESS_PRESENT
    | ACCESS_DPL_RING3
    | ACCESS_DESCRIPTOR_CODE_OR_DATA
    | ACCESS_TYPE_CODE
    | ACCESS_TYPE_CODE_READABLE
    | ACCESS_TYPE_ACCESSED;
pub(crate) const RIGHTS_RING3_DATA_WRITABLE_ACCESSED: u8 = ACCESS_PRESENT
    | ACCESS_DPL_RING3
    | ACCESS_DESCRIPTOR_CODE_OR_DATA
    | ACCESS_TYPE_DATA_WRITABLE
    | ACCESS_TYPE_ACCESSED;
pub(crate) const RIGHTS_TSS_386_AVAILABLE: u8 =
    ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_SYSTEM | SYSTEM_TYPE_TSS_386_AVAILABLE;
pub(crate) const RIGHTS_TSS_386_BUSY: u8 =
    ACCESS_PRESENT | ACCESS_DPL_RING0 | ACCESS_DESCRIPTOR_SYSTEM | SYSTEM_TYPE_TSS_386_BUSY;

// Granularity byte (high byte of the 8-byte descriptor) flags.

pub(crate) const GRANULARITY_PAGE: u8 = 0x80;
pub(crate) const GRANULARITY_BIG_OR_DEFAULT32: u8 = 0x40;

// Page-table entry bits per 80486 PRM Chapter 5.

pub(crate) const PAGE_PRESENT: u32 = 0x001;
pub(crate) const PAGE_WRITABLE: u32 = 0x002;
pub(crate) const PAGE_PRESENT_WRITABLE: u32 = PAGE_PRESENT | PAGE_WRITABLE;

// TSS layout offsets (Intel 80486 PRM Figure 7-1, 32-bit TSS).

pub(crate) const TSS_OFFSET_LINK: u32 = 0x00;
pub(crate) const TSS_OFFSET_ESP0: u32 = 0x04;
pub(crate) const TSS_OFFSET_SS0: u32 = 0x08;
pub(crate) const TSS_OFFSET_ESP1: u32 = 0x0C;
pub(crate) const TSS_OFFSET_SS1: u32 = 0x10;
pub(crate) const TSS_OFFSET_ESP2: u32 = 0x14;
pub(crate) const TSS_OFFSET_SS2: u32 = 0x18;
pub(crate) const TSS_OFFSET_CR3: u32 = 0x1C;
pub(crate) const TSS_OFFSET_EIP: u32 = 0x20;
pub(crate) const TSS_OFFSET_EFLAGS: u32 = 0x24;
pub(crate) const TSS_OFFSET_EAX: u32 = 0x28;
pub(crate) const TSS_OFFSET_ECX: u32 = 0x2C;
pub(crate) const TSS_OFFSET_EDX: u32 = 0x30;
pub(crate) const TSS_OFFSET_EBX: u32 = 0x34;
pub(crate) const TSS_OFFSET_ESP: u32 = 0x38;
pub(crate) const TSS_OFFSET_EBP: u32 = 0x3C;
pub(crate) const TSS_OFFSET_ESI: u32 = 0x40;
pub(crate) const TSS_OFFSET_EDI: u32 = 0x44;
pub(crate) const TSS_OFFSET_ES: u32 = 0x48;
pub(crate) const TSS_OFFSET_CS: u32 = 0x4C;
pub(crate) const TSS_OFFSET_SS: u32 = 0x50;
pub(crate) const TSS_OFFSET_DS: u32 = 0x54;
pub(crate) const TSS_OFFSET_FS: u32 = 0x58;
pub(crate) const TSS_OFFSET_GS: u32 = 0x5C;
pub(crate) const TSS_OFFSET_LDT: u32 = 0x60;
pub(crate) const TSS_OFFSET_IO_MAP_BASE_FIELD: u32 = 0x66;
pub(crate) const TSS_MINIMUM_LIMIT: u32 = 0x67;

pub(crate) const IO_PERMISSION_BITMAP_SENTINEL: u8 = 0xFF;
pub(crate) const IO_PERMISSION_BITMAP_DENY_ALL: u16 = 0xFFFF;

pub(crate) fn make_cpu_386() -> I386<{ CPU_MODEL_386_DX }> {
    I386::<{ CPU_MODEL_386_DX }>::new()
}

pub(crate) fn make_cpu_486() -> I386<{ CPU_MODEL_486_DX }> {
    I386::<{ CPU_MODEL_486_DX }>::new()
}

/// Test bus: a flat 1 MiB memory with simple IRQ injection.
pub(crate) struct TestBus {
    pub(crate) ram: Vec<u8>,
    pub(crate) irq_pending: bool,
    pub(crate) irq_vector: u8,
    pub(crate) io_read_default: u8,
    pub(crate) io_write_log: Vec<(u16, u8)>,
}

impl TestBus {
    pub(crate) fn new() -> Self {
        Self {
            ram: vec![0u8; TEST_BUS_RAM_BYTES],
            irq_pending: false,
            irq_vector: 0,
            io_read_default: 0xFF,
            io_write_log: Vec::new(),
        }
    }
}

impl common::Bus for TestBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        self.ram[(address & TEST_BUS_ADDRESS_MASK) as usize]
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.ram[(address & TEST_BUS_ADDRESS_MASK) as usize] = value;
    }

    fn io_read_byte(&mut self, _port: u16) -> u8 {
        self.io_read_default
    }

    fn io_write_byte(&mut self, port: u16, value: u8) {
        self.io_write_log.push((port, value));
    }

    fn is_io_port_unrestricted(&self, _port: u16) -> bool {
        false
    }

    fn has_irq(&self) -> bool {
        self.irq_pending
    }

    fn acknowledge_irq(&mut self) -> u8 {
        self.irq_pending = false;
        self.irq_vector
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

// Memory placement helpers.

pub(crate) fn place_code(bus: &mut TestBus, segment_selector: u16, offset: u16, code: &[u8]) {
    let segment_base = (segment_selector as u32) << 4;
    place_at(bus, segment_base + offset as u32, code);
}

pub(crate) fn place_at(bus: &mut TestBus, linear_address: u32, code: &[u8]) {
    for (index, &byte) in code.iter().enumerate() {
        bus.ram[linear_address as usize + index] = byte;
    }
}

pub(crate) fn read_byte_at(bus: &TestBus, linear_address: u32) -> u8 {
    bus.ram[linear_address as usize]
}

pub(crate) fn read_word_at(bus: &TestBus, linear_address: u32) -> u16 {
    let low = bus.ram[linear_address as usize] as u16;
    let high = bus.ram[linear_address as usize + 1] as u16;
    low | (high << 8)
}

pub(crate) fn read_dword_at(bus: &TestBus, linear_address: u32) -> u32 {
    let mut value = 0u32;
    for index in 0..4 {
        value |= (bus.ram[linear_address as usize + index] as u32) << (index * 8);
    }
    value
}

pub(crate) fn write_word_at(bus: &mut TestBus, linear_address: u32, value: u16) {
    bus.ram[linear_address as usize] = value as u8;
    bus.ram[linear_address as usize + 1] = (value >> 8) as u8;
}

pub(crate) fn write_dword_at(bus: &mut TestBus, linear_address: u32, value: u32) {
    for index in 0..4 {
        bus.ram[linear_address as usize + index] = (value >> (index * 8)) as u8;
    }
}

/// Descriptor builders. Intel 80486 PRM Figure 5-8 (segment descriptor).
pub(crate) fn write_segment_descriptor(
    bus: &mut TestBus,
    descriptor_table_base: u32,
    slot_index: u16,
    base_address: u32,
    limit_value: u32,
    access_rights: u8,
    granularity_byte: u8,
) {
    let descriptor_address = (descriptor_table_base + (slot_index as u32) * 8) as usize;
    bus.ram[descriptor_address] = limit_value as u8;
    bus.ram[descriptor_address + 1] = (limit_value >> 8) as u8;
    bus.ram[descriptor_address + 2] = base_address as u8;
    bus.ram[descriptor_address + 3] = (base_address >> 8) as u8;
    bus.ram[descriptor_address + 4] = (base_address >> 16) as u8;
    bus.ram[descriptor_address + 5] = access_rights;
    bus.ram[descriptor_address + 6] = granularity_byte | ((limit_value >> 16) as u8 & 0x0F);
    bus.ram[descriptor_address + 7] = (base_address >> 24) as u8;
}

pub(crate) fn write_segment_descriptor_16bit(
    bus: &mut TestBus,
    descriptor_table_base: u32,
    slot_index: u16,
    base_address: u32,
    limit_value: u16,
    access_rights: u8,
) {
    write_segment_descriptor(
        bus,
        descriptor_table_base,
        slot_index,
        base_address,
        limit_value as u32,
        access_rights,
        0,
    );
}

/// Gate descriptor: used for call gates, interrupt gates, trap gates,
/// and task gates. Intel 80486 PRM Figure 6-5 (call gate), Figure 7-4
/// (task gate), and Figure 9-2 (interrupt and trap gates). The
/// parameter-count field only applies to call gates.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_gate_descriptor(
    bus: &mut TestBus,
    descriptor_table_base: u32,
    slot_index: u16,
    target_offset: u32,
    target_selector: u16,
    parameter_count: u8,
    gate_type: u8,
    gate_dpl: u8,
) {
    let descriptor_address = (descriptor_table_base + (slot_index as u32) * 8) as usize;
    bus.ram[descriptor_address] = target_offset as u8;
    bus.ram[descriptor_address + 1] = (target_offset >> 8) as u8;
    bus.ram[descriptor_address + 2] = target_selector as u8;
    bus.ram[descriptor_address + 3] = (target_selector >> 8) as u8;
    bus.ram[descriptor_address + 4] = parameter_count & 0x1F;
    bus.ram[descriptor_address + 5] =
        ACCESS_PRESENT | ((gate_dpl & 3) << 5) | ACCESS_DESCRIPTOR_SYSTEM | (gate_type & 0x0F);
    bus.ram[descriptor_address + 6] = (target_offset >> 16) as u8;
    bus.ram[descriptor_address + 7] = (target_offset >> 24) as u8;
}

pub(crate) fn write_interrupt_gate_386(
    bus: &mut TestBus,
    interrupt_descriptor_table_base: u32,
    vector: u8,
    handler_offset: u32,
    handler_selector: u16,
    gate_dpl: u8,
) {
    write_gate_descriptor(
        bus,
        interrupt_descriptor_table_base,
        vector as u16,
        handler_offset,
        handler_selector,
        0,
        SYSTEM_TYPE_INTERRUPT_GATE_386,
        gate_dpl,
    );
}

pub(crate) fn write_trap_gate_386(
    bus: &mut TestBus,
    interrupt_descriptor_table_base: u32,
    vector: u8,
    handler_offset: u32,
    handler_selector: u16,
    gate_dpl: u8,
) {
    write_gate_descriptor(
        bus,
        interrupt_descriptor_table_base,
        vector as u16,
        handler_offset,
        handler_selector,
        0,
        SYSTEM_TYPE_TRAP_GATE_386,
        gate_dpl,
    );
}

pub(crate) fn write_interrupt_gate_286(
    bus: &mut TestBus,
    interrupt_descriptor_table_base: u32,
    vector: u8,
    handler_offset: u32,
    handler_selector: u16,
    gate_dpl: u8,
) {
    write_gate_descriptor(
        bus,
        interrupt_descriptor_table_base,
        vector as u16,
        handler_offset,
        handler_selector,
        0,
        SYSTEM_TYPE_INTERRUPT_GATE_286,
        gate_dpl,
    );
}

pub(crate) fn write_trap_gate_286(
    bus: &mut TestBus,
    interrupt_descriptor_table_base: u32,
    vector: u8,
    handler_offset: u32,
    handler_selector: u16,
    gate_dpl: u8,
) {
    write_gate_descriptor(
        bus,
        interrupt_descriptor_table_base,
        vector as u16,
        handler_offset,
        handler_selector,
        0,
        SYSTEM_TYPE_TRAP_GATE_286,
        gate_dpl,
    );
}

pub(crate) fn write_call_gate_386(
    bus: &mut TestBus,
    descriptor_table_base: u32,
    slot_index: u16,
    target_offset: u32,
    target_selector: u16,
    parameter_count: u8,
    gate_dpl: u8,
) {
    write_gate_descriptor(
        bus,
        descriptor_table_base,
        slot_index,
        target_offset,
        target_selector,
        parameter_count,
        SYSTEM_TYPE_CALL_GATE_386,
        gate_dpl,
    );
}

pub(crate) fn write_call_gate_286(
    bus: &mut TestBus,
    descriptor_table_base: u32,
    slot_index: u16,
    target_offset: u32,
    target_selector: u16,
    parameter_count: u8,
    gate_dpl: u8,
) {
    write_gate_descriptor(
        bus,
        descriptor_table_base,
        slot_index,
        target_offset,
        target_selector,
        parameter_count,
        SYSTEM_TYPE_CALL_GATE_286,
        gate_dpl,
    );
}

/// 386 TSS image. Pass `None` for any field that should remain zero.
/// Intel 80486 PRM Figure 7-1.
#[derive(Default, Clone, Copy)]
pub(crate) struct Tss386Image {
    pub(crate) backlink: u16,
    pub(crate) esp0: u32,
    pub(crate) ss0: u16,
    pub(crate) esp1: u32,
    pub(crate) ss1: u16,
    pub(crate) esp2: u32,
    pub(crate) ss2: u16,
    pub(crate) cr3: u32,
    pub(crate) eip: u32,
    pub(crate) eflags: u32,
    pub(crate) eax: u32,
    pub(crate) ecx: u32,
    pub(crate) edx: u32,
    pub(crate) ebx: u32,
    pub(crate) esp: u32,
    pub(crate) ebp: u32,
    pub(crate) esi: u32,
    pub(crate) edi: u32,
    pub(crate) es: u16,
    pub(crate) cs: u16,
    pub(crate) ss: u16,
    pub(crate) ds: u16,
    pub(crate) fs: u16,
    pub(crate) gs: u16,
    pub(crate) ldt: u16,
    pub(crate) io_map_base_offset: u16,
}

pub(crate) fn write_tss_386(bus: &mut TestBus, tss_base: u32, image: &Tss386Image) {
    write_word_at(bus, tss_base + TSS_OFFSET_LINK, image.backlink);
    write_dword_at(bus, tss_base + TSS_OFFSET_ESP0, image.esp0);
    write_word_at(bus, tss_base + TSS_OFFSET_SS0, image.ss0);
    write_dword_at(bus, tss_base + TSS_OFFSET_ESP1, image.esp1);
    write_word_at(bus, tss_base + TSS_OFFSET_SS1, image.ss1);
    write_dword_at(bus, tss_base + TSS_OFFSET_ESP2, image.esp2);
    write_word_at(bus, tss_base + TSS_OFFSET_SS2, image.ss2);
    write_dword_at(bus, tss_base + TSS_OFFSET_CR3, image.cr3);
    write_dword_at(bus, tss_base + TSS_OFFSET_EIP, image.eip);
    write_dword_at(bus, tss_base + TSS_OFFSET_EFLAGS, image.eflags);
    write_dword_at(bus, tss_base + TSS_OFFSET_EAX, image.eax);
    write_dword_at(bus, tss_base + TSS_OFFSET_ECX, image.ecx);
    write_dword_at(bus, tss_base + TSS_OFFSET_EDX, image.edx);
    write_dword_at(bus, tss_base + TSS_OFFSET_EBX, image.ebx);
    write_dword_at(bus, tss_base + TSS_OFFSET_ESP, image.esp);
    write_dword_at(bus, tss_base + TSS_OFFSET_EBP, image.ebp);
    write_dword_at(bus, tss_base + TSS_OFFSET_ESI, image.esi);
    write_dword_at(bus, tss_base + TSS_OFFSET_EDI, image.edi);
    write_word_at(bus, tss_base + TSS_OFFSET_ES, image.es);
    write_word_at(bus, tss_base + TSS_OFFSET_CS, image.cs);
    write_word_at(bus, tss_base + TSS_OFFSET_SS, image.ss);
    write_word_at(bus, tss_base + TSS_OFFSET_DS, image.ds);
    write_word_at(bus, tss_base + TSS_OFFSET_FS, image.fs);
    write_word_at(bus, tss_base + TSS_OFFSET_GS, image.gs);
    write_word_at(bus, tss_base + TSS_OFFSET_LDT, image.ldt);
    write_word_at(
        bus,
        tss_base + TSS_OFFSET_IO_MAP_BASE_FIELD,
        image.io_map_base_offset,
    );
}

pub(crate) const TSS_286_LIMIT: u32 = 43;

/// Build an I/O permission bitmap whose `allowed_ports` are clear (1 means
/// "deny" per Intel 80486 PRM 8.3.2). The bitmap occupies `bitmap_byte_count`
/// bytes followed by the mandatory 0xFF sentinel. Returns the total byte span
/// (bitmap + sentinel) so the caller can size the TSS limit correctly.
pub(crate) fn build_io_permission_bitmap(
    bus: &mut TestBus,
    tss_base: u32,
    io_map_base_offset: u16,
    bitmap_byte_count: u32,
    allowed_ports: &[u16],
) -> u32 {
    let bitmap_start = tss_base + io_map_base_offset as u32;
    for byte_index in 0..bitmap_byte_count {
        bus.ram[(bitmap_start + byte_index) as usize] = 0xFF;
    }
    for &port in allowed_ports {
        let byte_index = (port as u32) / 8;
        let bit_index = (port as u32) % 8;
        if byte_index < bitmap_byte_count {
            let address = (bitmap_start + byte_index) as usize;
            bus.ram[address] &= !(1u8 << bit_index);
        }
    }
    bus.ram[(bitmap_start + bitmap_byte_count) as usize] = IO_PERMISSION_BITMAP_SENTINEL;
    bitmap_byte_count + 1
}

/// Default offset for the I/O permission bitmap inside the 386 TSS image:
/// placed immediately after the fixed TSS fields ending at 0x67.
pub(crate) const DEFAULT_IO_MAP_BASE_OFFSET: u16 = 0x68;

/// Install an I/O permission bitmap into the TSS pointed at by `state.tr`.
/// Updates the IO_MAP_BASE field, writes the bitmap (deny-all baseline with
/// `allowed_ports` clearing their bits), writes the mandatory sentinel byte,
/// and grows both the cached `tr_limit` and the GDT TSS-slot limit so the
/// IOPB-fault path can read the bitmap and sentinel.
pub(crate) fn install_io_permission_bitmap(
    bus: &mut TestBus,
    state: &mut I386State,
    io_map_base_offset: u16,
    bitmap_byte_count: u32,
    allowed_ports: &[u16],
) {
    write_word_at(
        bus,
        state.tr_base + TSS_OFFSET_IO_MAP_BASE_FIELD,
        io_map_base_offset,
    );
    let span = build_io_permission_bitmap(
        bus,
        state.tr_base,
        io_map_base_offset,
        bitmap_byte_count,
        allowed_ports,
    );
    let new_limit = (io_map_base_offset as u32) + span - 1;
    state.tr_limit = new_limit;
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        state.tr >> 3,
        state.tr_base,
        new_limit as u16,
        state.tr_rights,
    );
}

/// Convenience wrapper for protected-mode IOPB tests: full handler set plus
/// an IOPB at DEFAULT_IO_MAP_BASE_OFFSET with the requested allow list.
pub(crate) fn setup_protected_mode_with_iopb(
    bus: &mut TestBus,
    bitmap_byte_count: u32,
    allowed_ports: &[u16],
) -> I386State {
    let mut state = setup_protected_mode_with_handlers(bus);
    install_io_permission_bitmap(
        bus,
        &mut state,
        DEFAULT_IO_MAP_BASE_OFFSET,
        bitmap_byte_count,
        allowed_ports,
    );
    state
}

/// Convenience wrapper for VM86 IOPB tests: enables VM86 at the requested
/// IOPL and installs an IOPB at DEFAULT_IO_MAP_BASE_OFFSET. The VM86 setup
/// does not include a #GP handler; tests that expect #GP must install one
/// via `install_protected_mode_general_protection_handler` first.
pub(crate) fn setup_vm86_with_iopl_and_iopb(
    bus: &mut TestBus,
    iopl: u8,
    bitmap_byte_count: u32,
    allowed_ports: &[u16],
) -> I386State {
    let mut state = setup_vm86_with_iopl(bus, iopl);
    install_io_permission_bitmap(
        bus,
        &mut state,
        DEFAULT_IO_MAP_BASE_OFFSET,
        bitmap_byte_count,
        allowed_ports,
    );
    state
}

/// Install a #GP handler at vector 13 pointing to HANDLER_GENERAL_PROTECTION_IP
/// in the ring-0 code segment. The handler is a single HLT byte, allowing
/// tests to detect the fault by checking `cpu.halted()` and `cpu.ip()`.
pub(crate) fn install_protected_mode_general_protection_handler(bus: &mut TestBus) {
    write_interrupt_gate_386(
        bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        13,
        HANDLER_GENERAL_PROTECTION_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + HANDLER_GENERAL_PROTECTION_IP as u32) as usize] = 0xF4;
}

/// Real-mode bare-bones state. Useful for tests that exercise instructions
/// before any descriptor table is set up.
pub(crate) fn make_real_mode_state() -> I386State {
    let mut state = I386State {
        ip: 0,
        ..I386State::default()
    };
    state.set_cs(0xF000);
    state.set_ds(0x0000);
    state.set_es(0x0000);
    state.set_ss(0x0000);
    state.set_esp(0xFFF0);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x000F_0000;
    state.seg_bases[cpu::SegReg32::DS as usize] = 0;
    state.seg_bases[cpu::SegReg32::ES as usize] = 0;
    state.seg_bases[cpu::SegReg32::SS as usize] = 0;
    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = RIGHTS_RING0_CODE_READABLE_ACCESSED;
    state.seg_rights[cpu::SegReg32::DS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_rights[cpu::SegReg32::ES as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_rights[cpu::SegReg32::SS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_valid = [true, true, true, true, false, false];
    state
}

/// Standard protected-mode environment with a four-entry GDT (null, ring-0
/// code, ring-0 data, ring-0 stack), an IDT with a #GP handler that halts,
/// and CR0.PE=1. The data-segment limit is parameterised so tests that
/// exercise limit-enforcement edge cases can shrink it.
pub(crate) fn setup_protected_mode(bus: &mut TestBus, data_segment_limit: u16) -> I386State {
    write_segment_descriptor_16bit(bus, GLOBAL_DESCRIPTOR_TABLE_BASE, 0, 0, 0, 0);
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        1,
        RING0_CODE_BASE,
        0xFFFF,
        RIGHTS_RING0_CODE_READABLE_ACCESSED,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        2,
        SHARED_DATA_BASE,
        data_segment_limit,
        RIGHTS_RING0_DATA_WRITABLE_ACCESSED,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        3,
        RING0_STACK_BASE,
        0xFFFF,
        RIGHTS_RING0_DATA_WRITABLE_ACCESSED,
    );

    write_interrupt_gate_386(
        bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        13,
        HANDLER_GENERAL_PROTECTION_IP as u32,
        SELECTOR_RING0_CODE,
        0,
    );
    bus.ram[(RING0_CODE_BASE + HANDLER_GENERAL_PROTECTION_IP as u32) as usize] = 0xF4;

    let mut state = I386State {
        cr0: 0x0001,
        ip: 0,
        ..I386State::default()
    };
    state.set_esp(0xFFF0);
    state.set_cs(SELECTOR_RING0_CODE);
    state.set_ds(SELECTOR_RING0_DATA);
    state.set_ss(SELECTOR_RING0_STACK);
    state.set_es(SELECTOR_RING0_DATA);

    state.seg_bases[cpu::SegReg32::CS as usize] = RING0_CODE_BASE;
    state.seg_bases[cpu::SegReg32::DS as usize] = SHARED_DATA_BASE;
    state.seg_bases[cpu::SegReg32::SS as usize] = RING0_STACK_BASE;
    state.seg_bases[cpu::SegReg32::ES as usize] = SHARED_DATA_BASE;

    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF;
    state.seg_limits[cpu::SegReg32::DS as usize] = data_segment_limit as u32;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF;
    state.seg_limits[cpu::SegReg32::ES as usize] = data_segment_limit as u32;

    state.seg_rights[cpu::SegReg32::CS as usize] = RIGHTS_RING0_CODE_READABLE_ACCESSED;
    state.seg_rights[cpu::SegReg32::DS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_rights[cpu::SegReg32::SS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_rights[cpu::SegReg32::ES as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;

    state.seg_valid = [true, true, true, true, false, false];

    state.gdt_base = GLOBAL_DESCRIPTOR_TABLE_BASE;
    state.gdt_limit = 4 * 8 - 1;
    state.idt_base = INTERRUPT_DESCRIPTOR_TABLE_BASE;
    state.idt_limit = 256 * 8 - 1;

    state
}

/// Extended protected-mode environment with ring-3 segments, a primary 386
/// TSS for inter-privilege transitions, and exception handlers for #DE,
/// #UD, #BP, #OF, #BR, #DB, #DF, #TS, #NP, #SS, #GP, #PF, #AC. Each handler
/// is a single HLT byte so tests can identify the raised exception by IP.
pub(crate) fn setup_protected_mode_with_handlers(bus: &mut TestBus) -> I386State {
    write_segment_descriptor_16bit(bus, GLOBAL_DESCRIPTOR_TABLE_BASE, 0, 0, 0, 0);
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        1,
        RING0_CODE_BASE,
        0xFFFF,
        RIGHTS_RING0_CODE_READABLE_ACCESSED,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        2,
        SHARED_DATA_BASE,
        0xFFFF,
        RIGHTS_RING0_DATA_WRITABLE_ACCESSED,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        3,
        RING0_STACK_BASE,
        0xFFFF,
        RIGHTS_RING0_DATA_WRITABLE_ACCESSED,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        4,
        RING3_CODE_BASE,
        0xFFFF,
        RIGHTS_RING3_CODE_READABLE_ACCESSED,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        5,
        SHARED_DATA_BASE,
        0xFFFF,
        RIGHTS_RING3_DATA_WRITABLE_ACCESSED,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        6,
        RING3_STACK_BASE,
        0xFFFF,
        RIGHTS_RING3_DATA_WRITABLE_ACCESSED,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        7,
        TASK_STATE_SEGMENT_BASE,
        TSS_MINIMUM_LIMIT as u16,
        RIGHTS_TSS_386_BUSY,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        9,
        TASK_STATE_SEGMENT_SECONDARY_BASE,
        TSS_MINIMUM_LIMIT as u16,
        RIGHTS_TSS_386_AVAILABLE,
    );

    let exception_handlers: &[(u8, u16)] = &[
        (0, HANDLER_DIVIDE_ERROR_IP),
        (1, HANDLER_DEBUG_IP),
        (3, HANDLER_BREAKPOINT_IP),
        (4, HANDLER_OVERFLOW_IP),
        (5, HANDLER_BOUND_RANGE_IP),
        (6, HANDLER_INVALID_OPCODE_IP),
        (7, HANDLER_DEVICE_NOT_AVAILABLE_IP),
        (8, HANDLER_DOUBLE_FAULT_IP),
        (10, HANDLER_INVALID_TSS_IP),
        (11, HANDLER_SEGMENT_NOT_PRESENT_IP),
        (12, HANDLER_STACK_FAULT_IP),
        (13, HANDLER_GENERAL_PROTECTION_IP),
        (14, HANDLER_PAGE_FAULT_IP),
        (17, HANDLER_ALIGNMENT_CHECK_IP),
    ];
    for &(vector, handler_ip) in exception_handlers {
        write_interrupt_gate_386(
            bus,
            INTERRUPT_DESCRIPTOR_TABLE_BASE,
            vector,
            handler_ip as u32,
            SELECTOR_RING0_CODE,
            0,
        );
        bus.ram[(RING0_CODE_BASE + handler_ip as u32) as usize] = 0xF4;
    }

    write_word_at(bus, TASK_STATE_SEGMENT_BASE + TSS_OFFSET_LINK, 0);
    write_dword_at(bus, TASK_STATE_SEGMENT_BASE + TSS_OFFSET_ESP0, 0xFFF0);
    write_word_at(
        bus,
        TASK_STATE_SEGMENT_BASE + TSS_OFFSET_SS0,
        SELECTOR_RING0_STACK,
    );
    write_word_at(
        bus,
        TASK_STATE_SEGMENT_BASE + TSS_OFFSET_IO_MAP_BASE_FIELD,
        IO_PERMISSION_BITMAP_DENY_ALL,
    );

    let mut state = I386State {
        cr0: 0x0001,
        ip: 0,
        ..I386State::default()
    };
    state.set_esp(0xFFF0);
    state.set_cs(SELECTOR_RING0_CODE);
    state.set_ds(SELECTOR_RING0_DATA);
    state.set_ss(SELECTOR_RING0_STACK);
    state.set_es(SELECTOR_RING0_DATA);

    state.seg_bases[cpu::SegReg32::CS as usize] = RING0_CODE_BASE;
    state.seg_bases[cpu::SegReg32::DS as usize] = SHARED_DATA_BASE;
    state.seg_bases[cpu::SegReg32::SS as usize] = RING0_STACK_BASE;
    state.seg_bases[cpu::SegReg32::ES as usize] = SHARED_DATA_BASE;

    state.seg_limits = [0xFFFF; 6];
    state.seg_rights[cpu::SegReg32::CS as usize] = RIGHTS_RING0_CODE_READABLE_ACCESSED;
    state.seg_rights[cpu::SegReg32::DS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_rights[cpu::SegReg32::SS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_rights[cpu::SegReg32::ES as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;
    state.seg_valid = [true, true, true, true, false, false];

    state.gdt_base = GLOBAL_DESCRIPTOR_TABLE_BASE;
    state.gdt_limit = 10 * 8 - 1;
    state.idt_base = INTERRUPT_DESCRIPTOR_TABLE_BASE;
    state.idt_limit = 256 * 8 - 1;

    state.tr = SELECTOR_PRIMARY_TSS;
    state.tr_base = TASK_STATE_SEGMENT_BASE;
    state.tr_limit = TSS_MINIMUM_LIMIT;
    state.tr_rights = RIGHTS_TSS_386_BUSY;

    state
}

/// Promote the supplied protected-mode state into ring-3. The caller must
/// already have the ring-3 selectors present in the GDT (every helper above
/// arranges that). CS/SS/DS/ES are repointed at their ring-3 selectors with
/// matching descriptor caches.
pub(crate) fn promote_to_ring3(state: &mut I386State) {
    state.set_cs(SELECTOR_RING3_CODE);
    state.seg_bases[cpu::SegReg32::CS as usize] = RING3_CODE_BASE;
    state.seg_rights[cpu::SegReg32::CS as usize] = RIGHTS_RING3_CODE_READABLE_ACCESSED;

    state.set_ss(SELECTOR_RING3_STACK);
    state.seg_bases[cpu::SegReg32::SS as usize] = RING3_STACK_BASE;
    state.seg_rights[cpu::SegReg32::SS as usize] = RIGHTS_RING3_DATA_WRITABLE_ACCESSED;

    state.set_ds(SELECTOR_RING3_DATA);
    state.seg_bases[cpu::SegReg32::DS as usize] = SHARED_DATA_BASE;
    state.seg_rights[cpu::SegReg32::DS as usize] = RIGHTS_RING3_DATA_WRITABLE_ACCESSED;

    state.set_es(SELECTOR_RING3_DATA);
    state.seg_bases[cpu::SegReg32::ES as usize] = SHARED_DATA_BASE;
    state.seg_rights[cpu::SegReg32::ES as usize] = RIGHTS_RING3_DATA_WRITABLE_ACCESSED;
}

// VM86 environment: CR0.PE=1, EFLAGS.VM=1, IOPL=3 by default. Provides a
// 32-bit ring-0 stack reachable via TSS ESP0/SS0 and a default INT 0x42
// gate at DPL=3 so software interrupts go through the IDT.

pub(crate) const VM86_HANDLER_IP: u16 = 0x0100;
pub(crate) const VM86_TSS_LIMIT: u32 = 0x0A;

pub(crate) fn setup_vm86(bus: &mut TestBus) -> I386State {
    setup_vm86_with_iopl(bus, 3)
}

pub(crate) fn setup_vm86_with_iopl(bus: &mut TestBus, iopl: u8) -> I386State {
    write_segment_descriptor_16bit(bus, GLOBAL_DESCRIPTOR_TABLE_BASE, 0, 0, 0, 0);
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        1,
        RING0_CODE_BASE,
        0xFFFF,
        RIGHTS_RING0_CODE_READABLE_ACCESSED,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        2,
        SHARED_DATA_BASE,
        0xFFFF,
        RIGHTS_RING0_DATA_WRITABLE_ACCESSED,
    );
    write_segment_descriptor(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        3,
        RING0_STACK_BASE,
        0xFFFF,
        RIGHTS_RING0_DATA_WRITABLE_ACCESSED,
        GRANULARITY_BIG_OR_DEFAULT32,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        4,
        TASK_STATE_SEGMENT_BASE,
        VM86_TSS_LIMIT as u16,
        RIGHTS_TSS_386_AVAILABLE,
    );

    write_interrupt_gate_386(
        bus,
        INTERRUPT_DESCRIPTOR_TABLE_BASE,
        0x42,
        VM86_HANDLER_IP as u32,
        SELECTOR_RING0_CODE,
        3,
    );
    bus.ram[(RING0_CODE_BASE + VM86_HANDLER_IP as u32) as usize] = 0xF4;

    write_dword_at(bus, TASK_STATE_SEGMENT_BASE + TSS_OFFSET_ESP0, 0x0000_1000);
    write_word_at(
        bus,
        TASK_STATE_SEGMENT_BASE + TSS_OFFSET_SS0,
        SELECTOR_RING0_STACK,
    );

    let mut state = I386State {
        cr0: 0x0001,
        eflags_upper: 0x0002_0000,
        seg_valid: [true; 6],
        gdt_base: GLOBAL_DESCRIPTOR_TABLE_BASE,
        gdt_limit: 5 * 8 - 1,
        idt_base: INTERRUPT_DESCRIPTOR_TABLE_BASE,
        idt_limit: 256 * 8 - 1,
        tr: 0x0020,
        tr_base: TASK_STATE_SEGMENT_BASE,
        tr_limit: VM86_TSS_LIMIT,
        tr_rights: RIGHTS_TSS_386_AVAILABLE,
        ..I386State::default()
    };
    state.flags.iopl = iopl & 3;

    state.set_cs(0x1000);
    state.seg_bases[cpu::SegReg32::CS as usize] = 0x0001_0000;
    state.seg_limits[cpu::SegReg32::CS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::CS as usize] = RIGHTS_RING0_CODE_READABLE_ACCESSED;

    state.set_ss(0x2000);
    state.set_esp(0xF000);
    state.seg_bases[cpu::SegReg32::SS as usize] = 0x0002_0000;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::SS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;

    state.set_es(0x3000);
    state.seg_bases[cpu::SegReg32::ES as usize] = 0x0003_0000;
    state.seg_limits[cpu::SegReg32::ES as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::ES as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;

    state.set_ds(0x4000);
    state.seg_bases[cpu::SegReg32::DS as usize] = 0x0004_0000;
    state.seg_limits[cpu::SegReg32::DS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::DS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;

    state.set_fs(0x5000);
    state.seg_bases[cpu::SegReg32::FS as usize] = 0x0005_0000;
    state.seg_limits[cpu::SegReg32::FS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::FS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;

    state.set_gs(0x6000);
    state.seg_bases[cpu::SegReg32::GS as usize] = 0x0006_0000;
    state.seg_limits[cpu::SegReg32::GS as usize] = 0xFFFF;
    state.seg_rights[cpu::SegReg32::GS as usize] = RIGHTS_RING0_DATA_WRITABLE_ACCESSED;

    state
}

/// Identity paging across the entire 4 MiB covered by a single page table.
/// The single PDE points to PAGE_TABLE_BASE; PTEs map linear==physical.
pub(crate) fn enable_identity_paging(bus: &mut TestBus, state: &mut I386State) {
    write_dword_at(
        bus,
        PAGE_DIRECTORY_BASE,
        PAGE_TABLE_BASE | PAGE_PRESENT_WRITABLE,
    );
    for page_index in 0..1024u32 {
        write_dword_at(
            bus,
            PAGE_TABLE_BASE + page_index * 4,
            (page_index << 12) | PAGE_PRESENT_WRITABLE,
        );
    }
    state.cr0 |= 0x8000_0000;
    state.cr3 = PAGE_DIRECTORY_BASE;
}

pub(crate) fn set_identity_page_flags(bus: &mut TestBus, linear_address: u32, flags: u32) {
    let entry_index = (linear_address >> 12) & 0x3FF;
    write_dword_at(
        bus,
        PAGE_TABLE_BASE + entry_index * 4,
        (entry_index << 12) | flags,
    );
}

/// Real-mode environment with single-HLT IVT handlers installed for the
/// exceptions most commonly raised by Chapter 22 edge-case tests: #DE (0),
/// #UD (6), #DF (8), #SS (12), and #GP (13). Each handler lives at a
/// distinct linear address inside the same code segment (CS=0xF000) used
/// by the test's instruction stream, but in a non-overlapping offset window
/// so tests can identify the raised vector by IP. The handler vector is
/// returned only implicitly via the constants below.
pub(crate) const REAL_MODE_HANDLER_SEGMENT: u16 = 0x2000;
pub(crate) const REAL_MODE_HANDLER_BASE: u32 = 0x0002_0000;
pub(crate) const REAL_MODE_HANDLER_DIVIDE_ERROR_OFFSET: u16 = 0x0100;
pub(crate) const REAL_MODE_HANDLER_INVALID_OPCODE_OFFSET: u16 = 0x0200;
pub(crate) const REAL_MODE_HANDLER_DOUBLE_FAULT_OFFSET: u16 = 0x0300;
pub(crate) const REAL_MODE_HANDLER_STACK_FAULT_OFFSET: u16 = 0x0400;
pub(crate) const REAL_MODE_HANDLER_GENERAL_PROTECTION_OFFSET: u16 = 0x0500;
pub(crate) const REAL_MODE_HANDLER_BOUND_RANGE_OFFSET: u16 = 0x0600;

/// Install a single IVT entry pointing at (segment:offset). Used by
/// `setup_real_mode_with_ivt_handlers` and tests that need extra handlers.
pub(crate) fn install_real_mode_ivt_entry(
    bus: &mut TestBus,
    vector: u8,
    handler_segment: u16,
    handler_offset: u16,
) {
    let entry_address = (vector as usize) * 4;
    bus.ram[entry_address] = handler_offset as u8;
    bus.ram[entry_address + 1] = (handler_offset >> 8) as u8;
    bus.ram[entry_address + 2] = handler_segment as u8;
    bus.ram[entry_address + 3] = (handler_segment >> 8) as u8;
}

/// Real-mode setup with IVT handlers wired for the common exceptions. The
/// handlers each occupy a single HLT byte at REAL_MODE_HANDLER_BASE plus
/// the per-exception offset so tests can identify which fault fired by
/// reading `cpu.ip()` against `(handler_offset + 1)`.
pub(crate) fn setup_real_mode_with_ivt_handlers(bus: &mut TestBus) -> I386State {
    let handler_entries: &[(u8, u16)] = &[
        (0, REAL_MODE_HANDLER_DIVIDE_ERROR_OFFSET),
        (5, REAL_MODE_HANDLER_BOUND_RANGE_OFFSET),
        (6, REAL_MODE_HANDLER_INVALID_OPCODE_OFFSET),
        (8, REAL_MODE_HANDLER_DOUBLE_FAULT_OFFSET),
        (12, REAL_MODE_HANDLER_STACK_FAULT_OFFSET),
        (13, REAL_MODE_HANDLER_GENERAL_PROTECTION_OFFSET),
    ];
    for &(vector, handler_offset) in handler_entries {
        install_real_mode_ivt_entry(bus, vector, REAL_MODE_HANDLER_SEGMENT, handler_offset);
        bus.ram[(REAL_MODE_HANDLER_BASE + handler_offset as u32) as usize] = 0xF4;
    }

    make_real_mode_state()
}

// Mixed 16/32-bit protected-mode setup. Adds a 16-bit code segment alongside
// the 32-bit one used by `setup_protected_mode_with_handlers` so tests can
// transition between them via call gates and prefix overrides.

pub(crate) const SELECTOR_RING0_CODE_16BIT: u16 = 0x0050;
pub(crate) const SELECTOR_RING0_DATA_16BIT_STACK: u16 = 0x0058;
pub(crate) const RING0_CODE_16BIT_BASE: u32 = 0x0004_0000;
pub(crate) const RING0_STACK_16BIT_BASE: u32 = 0x0003_0000;

/// Protected-mode environment with both a 16-bit and a 32-bit ring-0 code
/// segment. The 32-bit code segment lives at SELECTOR_RING0_CODE (slot 1,
/// base RING0_CODE_BASE) and has the D-bit set; the 32-bit stack segment
/// at SELECTOR_RING0_STACK (slot 3) has the B-bit set. The 16-bit code
/// segment lives at SELECTOR_RING0_CODE_16BIT (slot 10, base
/// RING0_CODE_16BIT_BASE) with the D-bit clear. A separate 16-bit stack
/// segment at SELECTOR_RING0_DATA_16BIT_STACK (slot 11, base
/// RING0_STACK_16BIT_BASE) lets tests exercise the SS B-bit.
pub(crate) fn setup_protected_mode_mixed_widths(bus: &mut TestBus) -> I386State {
    let mut state = setup_protected_mode_with_handlers(bus);

    // Promote the default ring-0 code segment (slot 1) to 32-bit by setting
    // the D-bit, and the ring-0 stack segment (slot 3) to 32-bit by setting
    // the B-bit. The remaining segments (data, ring-3 mirrors) stay 16-bit
    // so tests that need 16-bit data accesses still work.
    write_segment_descriptor(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        1,
        RING0_CODE_BASE,
        0xFFFF,
        RIGHTS_RING0_CODE_READABLE_ACCESSED,
        GRANULARITY_BIG_OR_DEFAULT32,
    );
    write_segment_descriptor(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        3,
        RING0_STACK_BASE,
        0x000F_FFFF,
        RIGHTS_RING0_DATA_WRITABLE_ACCESSED,
        GRANULARITY_PAGE | GRANULARITY_BIG_OR_DEFAULT32,
    );
    state.seg_granularity[cpu::SegReg32::CS as usize] = GRANULARITY_BIG_OR_DEFAULT32;
    state.seg_granularity[cpu::SegReg32::SS as usize] =
        GRANULARITY_PAGE | GRANULARITY_BIG_OR_DEFAULT32;
    state.seg_limits[cpu::SegReg32::SS as usize] = 0xFFFF_FFFF;

    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        SELECTOR_RING0_CODE_16BIT >> 3,
        RING0_CODE_16BIT_BASE,
        0xFFFF,
        RIGHTS_RING0_CODE_READABLE_ACCESSED,
    );
    write_segment_descriptor_16bit(
        bus,
        GLOBAL_DESCRIPTOR_TABLE_BASE,
        SELECTOR_RING0_DATA_16BIT_STACK >> 3,
        RING0_STACK_16BIT_BASE,
        0xFFFF,
        RIGHTS_RING0_DATA_WRITABLE_ACCESSED,
    );

    let last_slot_index = SELECTOR_RING0_DATA_16BIT_STACK >> 3;
    state.gdt_limit = (last_slot_index + 1) * 8 - 1;

    state
}
