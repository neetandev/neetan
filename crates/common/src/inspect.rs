//! Machine-neutral inspection and mutation boundary.
//!
//! Defines the [`MachineInspector`] surface and its descriptor types, plus
//! shared per-architecture register helpers so each machine family exposes a
//! uniform register and memory view without leaking concrete CPU or device
//! types. Register access is split across the four architecture traits
//! ([`Cpu`], [`CpuZ80`], [`CpuM68000`], [`Cpu6809`]),
//! none of which is object-safe, so the helpers are generic free functions.

use alloc::vec::Vec;

use crate::{Cpu, Cpu6809, CpuM68000, CpuZ80, stack_vec::StackVec};

/// Maximum inspectable processors a machine exposes (dual-CPU families).
pub const MAX_PROCESSORS: usize = 2;

/// Maximum inspectable text surfaces a machine exposes.
pub const MAX_TEXT_SURFACES: usize = 2;

/// Maximum inspectable address spaces a machine exposes (main and sub, each
/// with a memory and an I/O space).
pub const MAX_ADDRESS_SPACES: usize = 4;

/// A bounded, stack-allocated list of processor descriptors.
pub type ProcessorList = StackVec<ProcessorDescriptor, MAX_PROCESSORS>;

/// A bounded, stack-allocated list of address-space descriptors.
pub type AddressSpaceList = StackVec<AddressSpaceDescriptor, MAX_ADDRESS_SPACES>;

/// Byte order of a multi-byte value in an address space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    /// Least significant byte first.
    Little,
    /// Most significant byte first.
    Big,
}

/// Broad class of an inspectable address space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSpaceClass {
    /// A processor memory space.
    Memory,
    /// A processor I/O space.
    Io,
}

/// Static shape of one inspectable register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterDescriptor {
    /// Stable register name, for example `"pc"` or `"ax"`.
    pub name: &'static str,
    /// Register width in bits.
    pub bits: u32,
    /// Whether mutation supports writing this register.
    pub writable: bool,
}

/// Descriptor for one inspectable processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessorDescriptor {
    /// Stable processor identifier, for example `"cpu.main"` or `"cpu.sub"`.
    pub id: &'static str,
    /// Stable architecture name, for example `"z80"`, `"x86"`, `"m68000"`, or
    /// `"m6809"`.
    pub architecture: &'static str,
    /// The registers this processor exposes. This is a borrow of a fixed
    /// architecture table, so it never allocates.
    pub registers: &'static [RegisterDescriptor],
    /// Whether this processor provides a protected-mode state snapshot.
    pub protected_mode: bool,
}

/// Descriptor for one inspectable address space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressSpaceDescriptor {
    /// Stable space identifier, for example `"cpu.main.memory"`.
    pub id: &'static str,
    /// Broad class of the space.
    pub class: AddressSpaceClass,
    /// Address width in bits.
    pub address_bits: u32,
    /// Byte order used to assemble multi-byte values.
    pub byte_order: ByteOrder,
    /// Whether a side-effect-free peek is supported.
    pub peekable: bool,
    /// Whether mutation supports writing this space.
    pub writable: bool,
}

/// One named register value read from a processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterReading {
    /// Stable register name.
    pub name: &'static str,
    /// Current value, zero-extended into an unsigned 128-bit integer.
    pub value: u128,
}

/// One segment register with its cached descriptor fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentReading {
    /// Stable segment name, for example `"cs"`.
    pub name: &'static str,
    /// Segment selector value.
    pub selector: u32,
    /// Cached linear base address.
    pub base: u32,
    /// Cached effective limit.
    pub limit: u32,
    /// Cached access-rights byte.
    pub rights: u32,
}

/// One descriptor-table or system-segment register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescriptorTableReading {
    /// Stable name, for example `"gdtr"`, `"idtr"`, `"ldtr"`, or `"tr"`.
    pub name: &'static str,
    /// Selector value, when the register has one.
    pub selector: Option<u32>,
    /// Cached base address.
    pub base: u32,
    /// Cached limit.
    pub limit: u32,
}

/// A full protected-mode state snapshot for an i386 or later processor.
///
/// Every field is a fixed-size array because the protected-mode register set
/// is fixed by the architecture, so the snapshot needs no heap allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtectedModeState {
    /// 32-bit general-purpose registers (`eax` through `edi`).
    pub general: [RegisterReading; 8],
    /// Segment registers with cached descriptor fields (`es`, `cs`, `ss`, `ds`,
    /// `fs`, `gs`).
    pub segments: [SegmentReading; 6],
    /// Control registers (`cr0`, `cr2`, `cr3`).
    pub control: [RegisterReading; 3],
    /// Debug registers (`dr0` through `dr3`, `dr6`, `dr7`).
    pub debug: [RegisterReading; 6],
    /// Descriptor-table and system-segment registers (`gdtr`, `idtr`, `ldtr`,
    /// `tr`).
    pub descriptor_tables: [DescriptorTableReading; 4],
    /// Full 32-bit instruction pointer.
    pub eip: u64,
    /// Full 32-bit flags register.
    pub eflags: u64,
}

/// Why an inspection or mutation operation failed.
///
/// The automation session maps each variant to a stable `neetan/*` error
/// symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectError {
    /// The processor identifier is not known to this machine.
    UnknownProcessor,
    /// The register name is not known to the processor.
    UnknownRegister,
    /// The address-space identifier is not known to this machine.
    UnknownSpace,
    /// The register or space is read-only.
    NotWritable,
    /// The space does not support a side-effect-free peek.
    NotPeekable,
    /// The operation is not supported by this machine or processor.
    Unsupported,
    /// An address, length, or value was out of the accepted range.
    OutOfRange,
}

/// Read-only inspection and mutation surface for a machine.
///
/// A machine implements this on itself and returns `Some(self)` from
/// [`crate::AutomatedMachine::inspector`], so register reads and memory peeks
/// can borrow the CPU and bus together. Peeks are side-effect-free: they use
/// the memory decode only and never perform a device read.
pub trait MachineInspector {
    /// Returns the processors this machine exposes.
    fn processors(&self) -> ProcessorList;

    /// Returns the address spaces this machine exposes.
    fn address_spaces(&self) -> AddressSpaceList;

    /// Reads one register, zero-extended into an unsigned 128-bit integer.
    fn read_register(&self, processor: &str, register: &str) -> Result<u128, InspectError>;

    /// Writes one register, validating the value against its width first.
    fn write_register(
        &mut self,
        processor: &str,
        register: &str,
        value: u128,
    ) -> Result<(), InspectError>;

    /// Returns the protected-mode state of an i386 or later processor.
    fn protected_mode_state(&self, processor: &str) -> Result<ProtectedModeState, InspectError>;

    /// Fills `buffer` with a side-effect-free read of `space` from `address`.
    fn peek_memory(
        &mut self,
        space: &str,
        address: u64,
        buffer: &mut [u8],
    ) -> Result<(), InspectError>;

    /// Writes `bytes` to `space` at `address` through the memory decode.
    fn poke_memory(&mut self, space: &str, address: u64, bytes: &[u8]) -> Result<(), InspectError>;
}

/// A bounded, stack-allocated list of text-surface descriptors.
pub type TextSurfaceList = StackVec<TextSurfaceInfo, MAX_TEXT_SURFACES>;

/// Geometry of one inspectable text surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSurfaceInfo {
    /// Stable surface identifier, for example `"display.main"`.
    pub id: &'static str,
    /// Number of text rows.
    pub rows: u16,
    /// Number of text columns.
    pub columns: u16,
}

/// One decoded text-mode cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextCell {
    /// Zero-based row.
    pub row: u16,
    /// Zero-based column.
    pub column: u16,
    /// Raw code as stored in the character plane.
    pub raw_jis: u16,
    /// Decoded Unicode character, when the code maps to one.
    pub unicode: Option<char>,
    /// Raw attribute byte from the attribute plane.
    pub attribute: u8,
    /// Display width in cells, 1 for half-width and 2 for full-width.
    pub display_width: u8,
}

/// Read-only decoded text-surface inspection surface.
///
/// A machine implements this on itself and returns `Some(self)` from
/// [`crate::AutomatedMachine::text_inspector`]. Every read is side-effect-free
/// and decodes the current text VRAM without touching device state.
pub trait TextSurfaceInspector {
    /// Returns the text surfaces this machine exposes.
    fn text_surfaces(&self) -> TextSurfaceList;

    /// Returns the geometry of one surface, or an error when it is unknown.
    fn text_surface_info(&self, surface: &str) -> Result<TextSurfaceInfo, InspectError>;

    /// Decodes one cell of a surface.
    fn text_cell(&self, surface: &str, row: u16, column: u16) -> Result<TextCell, InspectError>;

    /// Decodes one full row of a surface, left to right.
    fn text_row(&self, surface: &str, row: u16) -> Result<Vec<TextCell>, InspectError>;

    /// Decodes every row of a surface, top to bottom.
    fn text_screen(&self, surface: &str) -> Result<Vec<Vec<TextCell>>, InspectError>;
}

/// Adds a byte offset to a base address for a 16-bit space, reporting overflow
/// as an out-of-range failure.
pub fn offset_u16(base: u64, index: usize) -> Result<u16, InspectError> {
    let address = base
        .checked_add(index as u64)
        .ok_or(InspectError::OutOfRange)?;
    u16::try_from(address).map_err(|_| InspectError::OutOfRange)
}

/// Adds a byte offset to a base address for a 32-bit space, reporting overflow
/// as an out-of-range failure.
pub fn offset_u32(base: u64, index: usize) -> Result<u32, InspectError> {
    let address = base
        .checked_add(index as u64)
        .ok_or(InspectError::OutOfRange)?;
    u32::try_from(address).map_err(|_| InspectError::OutOfRange)
}

/// Builds a Z80 processor descriptor with the given identifier.
pub fn z80_processor(id: &'static str) -> ProcessorDescriptor {
    ProcessorDescriptor {
        id,
        architecture: "z80",
        registers: z80_registers(),
        protected_mode: false,
    }
}

/// Builds an x86 processor descriptor with the given identifier.
pub fn x86_processor(id: &'static str, protected_mode: bool) -> ProcessorDescriptor {
    ProcessorDescriptor {
        id,
        architecture: "x86",
        registers: x86_registers(),
        protected_mode,
    }
}

/// Builds an MC68000 processor descriptor with the given identifier.
pub fn m68000_processor(id: &'static str) -> ProcessorDescriptor {
    ProcessorDescriptor {
        id,
        architecture: "m68000",
        registers: m68000_registers(),
        protected_mode: false,
    }
}

/// Builds an MC6809 processor descriptor with the given identifier.
pub fn m6809_processor(id: &'static str) -> ProcessorDescriptor {
    ProcessorDescriptor {
        id,
        architecture: "m6809",
        registers: m6809_registers(),
        protected_mode: false,
    }
}

/// Builds a peekable, writable memory address-space descriptor.
pub fn memory_space(
    id: &'static str,
    address_bits: u32,
    byte_order: ByteOrder,
) -> AddressSpaceDescriptor {
    AddressSpaceDescriptor {
        id,
        class: AddressSpaceClass::Memory,
        address_bits,
        byte_order,
        peekable: true,
        writable: true,
    }
}

/// Builds an I/O address-space descriptor. I/O spaces are descriptors only: no
/// side-effect-free peek exists, so they are neither peekable nor writable.
pub fn io_space(
    id: &'static str,
    address_bits: u32,
    byte_order: ByteOrder,
) -> AddressSpaceDescriptor {
    AddressSpaceDescriptor {
        id,
        class: AddressSpaceClass::Io,
        address_bits,
        byte_order,
        peekable: false,
        writable: false,
    }
}

/// Builds a writable register descriptor.
const fn writable(name: &'static str, bits: u32) -> RegisterDescriptor {
    RegisterDescriptor {
        name,
        bits,
        writable: true,
    }
}

/// Narrows a 128-bit value to a `u8`, or reports an out-of-range failure.
fn fit8(value: u128) -> Result<u8, InspectError> {
    u8::try_from(value).map_err(|_| InspectError::OutOfRange)
}

/// Narrows a 128-bit value to a `u16`, or reports an out-of-range failure.
fn fit16(value: u128) -> Result<u16, InspectError> {
    u16::try_from(value).map_err(|_| InspectError::OutOfRange)
}

/// Narrows a 128-bit value to a `u32`, or reports an out-of-range failure.
fn fit32(value: u128) -> Result<u32, InspectError> {
    u32::try_from(value).map_err(|_| InspectError::OutOfRange)
}

/// The fixed base Z80 register table.
static Z80_REGISTERS: [RegisterDescriptor; 13] = [
    writable("pc", 16),
    writable("sp", 16),
    writable("af", 16),
    writable("bc", 16),
    writable("de", 16),
    writable("hl", 16),
    writable("ix", 16),
    writable("iy", 16),
    writable("i", 8),
    writable("r", 8),
    writable("iff1", 1),
    writable("iff2", 1),
    writable("im", 8),
];

/// Returns the base Z80 register descriptors.
pub fn z80_registers() -> &'static [RegisterDescriptor] {
    &Z80_REGISTERS
}

/// Reads one Z80 register by name.
pub fn z80_read<C: CpuZ80>(cpu: &C, register: &str) -> Result<u128, InspectError> {
    let value = match register {
        "pc" => u128::from(cpu.pc()),
        "sp" => u128::from(cpu.sp()),
        "af" => u128::from(cpu.af()),
        "bc" => u128::from(cpu.bc()),
        "de" => u128::from(cpu.de()),
        "hl" => u128::from(cpu.hl()),
        "ix" => u128::from(cpu.ix()),
        "iy" => u128::from(cpu.iy()),
        "i" => u128::from(cpu.i()),
        "r" => u128::from(cpu.r()),
        "iff1" => u128::from(cpu.iff1()),
        "iff2" => u128::from(cpu.iff2()),
        "im" => u128::from(cpu.im()),
        _ => return Err(InspectError::UnknownRegister),
    };
    Ok(value)
}

/// Writes one Z80 register by name.
pub fn z80_write<C: CpuZ80>(cpu: &mut C, register: &str, value: u128) -> Result<(), InspectError> {
    match register {
        "pc" => cpu.set_pc(fit16(value)?),
        "sp" => cpu.set_sp(fit16(value)?),
        "af" => cpu.set_af(fit16(value)?),
        "bc" => cpu.set_bc(fit16(value)?),
        "de" => cpu.set_de(fit16(value)?),
        "hl" => cpu.set_hl(fit16(value)?),
        "ix" => cpu.set_ix(fit16(value)?),
        "iy" => cpu.set_iy(fit16(value)?),
        "i" => cpu.set_i(fit8(value)?),
        "r" => cpu.set_r(fit8(value)?),
        "iff1" => cpu.set_iff1(value != 0),
        "iff2" => cpu.set_iff2(value != 0),
        "im" => cpu.set_im(fit8(value)?),
        _ => return Err(InspectError::UnknownRegister),
    }
    Ok(())
}

/// The fixed base x86 register table (16-bit view shared by every x86 model).
static X86_REGISTERS: [RegisterDescriptor; 14] = [
    writable("ax", 16),
    writable("bx", 16),
    writable("cx", 16),
    writable("dx", 16),
    writable("sp", 16),
    writable("bp", 16),
    writable("si", 16),
    writable("di", 16),
    writable("es", 16),
    writable("cs", 16),
    writable("ss", 16),
    writable("ds", 16),
    writable("ip", 16),
    writable("flags", 16),
];

/// Returns the base x86 register descriptors (16-bit view shared by every
/// x86 model).
pub fn x86_registers() -> &'static [RegisterDescriptor] {
    &X86_REGISTERS
}

/// Reads one base x86 register by name.
pub fn x86_read<C: Cpu>(cpu: &C, register: &str) -> Result<u128, InspectError> {
    let value = match register {
        "ax" => u128::from(cpu.ax()),
        "bx" => u128::from(cpu.bx()),
        "cx" => u128::from(cpu.cx()),
        "dx" => u128::from(cpu.dx()),
        "sp" => u128::from(cpu.sp()),
        "bp" => u128::from(cpu.bp()),
        "si" => u128::from(cpu.si()),
        "di" => u128::from(cpu.di()),
        "es" => u128::from(cpu.es()),
        "cs" => u128::from(cpu.cs()),
        "ss" => u128::from(cpu.ss()),
        "ds" => u128::from(cpu.ds()),
        "ip" => u128::from(cpu.ip()),
        "flags" => u128::from(cpu.flags()),
        _ => return Err(InspectError::UnknownRegister),
    };
    Ok(value)
}

/// Writes one base x86 register by name.
pub fn x86_write<C: Cpu>(cpu: &mut C, register: &str, value: u128) -> Result<(), InspectError> {
    match register {
        "ax" => cpu.set_ax(fit16(value)?),
        "bx" => cpu.set_bx(fit16(value)?),
        "cx" => cpu.set_cx(fit16(value)?),
        "dx" => cpu.set_dx(fit16(value)?),
        "sp" => cpu.set_sp(fit16(value)?),
        "bp" => cpu.set_bp(fit16(value)?),
        "si" => cpu.set_si(fit16(value)?),
        "di" => cpu.set_di(fit16(value)?),
        "es" => cpu.set_es(fit16(value)?),
        "cs" => cpu.set_cs(fit16(value)?),
        "ss" => cpu.set_ss(fit16(value)?),
        "ds" => cpu.set_ds(fit16(value)?),
        "ip" => cpu.set_ip(fit16(value)?),
        "flags" => cpu.set_flags(fit16(value)?),
        _ => return Err(InspectError::UnknownRegister),
    }
    Ok(())
}

/// The fixed base MC68000 register table.
static M68000_REGISTERS: [RegisterDescriptor; 19] = [
    writable("pc", 32),
    writable("d0", 32),
    writable("d1", 32),
    writable("d2", 32),
    writable("d3", 32),
    writable("d4", 32),
    writable("d5", 32),
    writable("d6", 32),
    writable("d7", 32),
    writable("a0", 32),
    writable("a1", 32),
    writable("a2", 32),
    writable("a3", 32),
    writable("a4", 32),
    writable("a5", 32),
    writable("a6", 32),
    writable("usp", 32),
    writable("ssp", 32),
    writable("sr", 16),
];

/// Returns the base MC68000 register descriptors.
pub fn m68000_registers() -> &'static [RegisterDescriptor] {
    &M68000_REGISTERS
}

/// Maps a `d0`..`d7` name to its index.
fn data_register_index(register: &str) -> Option<usize> {
    match register {
        "d0" => Some(0),
        "d1" => Some(1),
        "d2" => Some(2),
        "d3" => Some(3),
        "d4" => Some(4),
        "d5" => Some(5),
        "d6" => Some(6),
        "d7" => Some(7),
        _ => None,
    }
}

/// Maps an `a0`..`a6` name to its index.
fn address_register_index(register: &str) -> Option<usize> {
    match register {
        "a0" => Some(0),
        "a1" => Some(1),
        "a2" => Some(2),
        "a3" => Some(3),
        "a4" => Some(4),
        "a5" => Some(5),
        "a6" => Some(6),
        _ => None,
    }
}

/// Reads one MC68000 register by name.
pub fn m68000_read<C: CpuM68000>(cpu: &C, register: &str) -> Result<u128, InspectError> {
    if let Some(index) = data_register_index(register) {
        return Ok(u128::from(cpu.d(index)));
    }
    if let Some(index) = address_register_index(register) {
        return Ok(u128::from(cpu.a(index)));
    }
    let value = match register {
        "pc" => u128::from(cpu.pc()),
        "usp" => u128::from(cpu.usp()),
        "ssp" => u128::from(cpu.ssp()),
        "sr" => u128::from(cpu.sr()),
        _ => return Err(InspectError::UnknownRegister),
    };
    Ok(value)
}

/// Writes one MC68000 register by name.
pub fn m68000_write<C: CpuM68000>(
    cpu: &mut C,
    register: &str,
    value: u128,
) -> Result<(), InspectError> {
    if let Some(index) = data_register_index(register) {
        cpu.set_d(index, fit32(value)?);
        return Ok(());
    }
    if let Some(index) = address_register_index(register) {
        cpu.set_a(index, fit32(value)?);
        return Ok(());
    }
    match register {
        "pc" => cpu.set_pc(fit32(value)?),
        "usp" => cpu.set_usp(fit32(value)?),
        "ssp" => cpu.set_ssp(fit32(value)?),
        "sr" => cpu.set_sr(fit16(value)?),
        _ => return Err(InspectError::UnknownRegister),
    }
    Ok(())
}

/// The fixed base MC6809 register table.
static M6809_REGISTERS: [RegisterDescriptor; 10] = [
    writable("pc", 16),
    writable("s", 16),
    writable("u", 16),
    writable("x", 16),
    writable("y", 16),
    writable("a", 8),
    writable("b", 8),
    writable("d", 16),
    writable("dp", 8),
    writable("cc", 8),
];

/// Returns the base MC6809 register descriptors.
pub fn m6809_registers() -> &'static [RegisterDescriptor] {
    &M6809_REGISTERS
}

/// Reads one MC6809 register by name.
pub fn m6809_read<C: Cpu6809>(cpu: &C, register: &str) -> Result<u128, InspectError> {
    let value = match register {
        "pc" => u128::from(cpu.pc()),
        "s" => u128::from(cpu.s()),
        "u" => u128::from(cpu.u()),
        "x" => u128::from(cpu.x()),
        "y" => u128::from(cpu.y()),
        "a" => u128::from(cpu.a()),
        "b" => u128::from(cpu.b()),
        "d" => u128::from(cpu.d()),
        "dp" => u128::from(cpu.dp()),
        "cc" => u128::from(cpu.cc()),
        _ => return Err(InspectError::UnknownRegister),
    };
    Ok(value)
}

/// Writes one MC6809 register by name.
pub fn m6809_write<C: Cpu6809>(
    cpu: &mut C,
    register: &str,
    value: u128,
) -> Result<(), InspectError> {
    match register {
        "pc" => cpu.set_pc(fit16(value)?),
        "s" => cpu.set_s(fit16(value)?),
        "u" => cpu.set_u(fit16(value)?),
        "x" => cpu.set_x(fit16(value)?),
        "y" => cpu.set_y(fit16(value)?),
        "a" => cpu.set_a(fit8(value)?),
        "b" => cpu.set_b(fit8(value)?),
        "d" => cpu.set_d(fit16(value)?),
        "dp" => cpu.set_dp(fit8(value)?),
        "cc" => cpu.set_cc(fit8(value)?),
        _ => return Err(InspectError::UnknownRegister),
    }
    Ok(())
}
