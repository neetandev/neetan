//! HLE CMOS BIOS at `FFFB:0019`.
//!
//! The CMOS starts with a header of 16 block descriptors. Each descriptor
//! holds a flag byte (bit 0 present, bit 7 checksummed), an ID byte, a
//! 4-character name, the CMOS address and the size of the block. The header
//! sum, the signature, a checksum table and a block number table complete it.
//! Addresses are CMOS byte indices; CMOS byte `n` is I/O port `3000h + 2n`.

use common::{Cpu, SegmentRegister, TraceSink, warn};

use crate::bus::TownsBus;

/// CMOS port where the CMOS BIOS saves the AX of the caller.
const CMOS_SAVED_AX: u16 = 0x319C;
/// CMOS port where the CMOS BIOS saves the caller SS.
const CMOS_SAVED_SS: u16 = 0x31A0;
/// CMOS port where the CMOS BIOS saves the caller SP.
const CMOS_SAVED_SP: u16 = 0x31A4;

/// Number of block descriptors in the header.
const BLOCK_COUNT: usize = 16;
/// Size of one block descriptor.
const DESCRIPTOR_SIZE: usize = 10;
/// Offset of the address word in a block descriptor.
const DESCRIPTOR_ADDRESS: usize = 6;
/// Offset of the size word in a block descriptor.
const DESCRIPTOR_SIZE_WORD: usize = 8;
/// Descriptor flag bit: the block is present.
const BLOCK_PRESENT: u8 = 0x01;
/// Descriptor flag bit: the block has a checksum.
const BLOCK_CHECKSUMMED: u8 = 0x80;
/// CMOS index of the header word that holds the end of the used area.
const HEADER_END_OF_BLOCKS: usize = 0x00A0;
/// CMOS index of the header word returned by function 05h.
const HEADER_WORD_A2: usize = 0x00A2;
/// CMOS index of the header sum word.
const HEADER_SUM: usize = 0x00A4;
/// CMOS index of the header signature word.
const HEADER_SIGNATURE: usize = 0x00A6;
/// Size of the header area cleared by function 00h.
const HEADER_SIZE: usize = 0x00A8;
/// Header signature value.
const SIGNATURE: u16 = 0x4179;
/// Last CMOS index cleared by function 00h, plus one.
const CMOS_DATA_END: usize = 0x07C1;
/// CMOS index of the checksum table (I/O 3F82h).
const CHECKSUM_TABLE: usize = 0x07C1;
/// CMOS index of the block number table (I/O 3FA2h).
const BLOCK_NUMBER_TABLE: usize = 0x07D1;

/// Default block descriptors written by function 00h: flag, ID, name, CMOS
/// address and size.
const DEFAULT_BLOCKS: [(u8, u8, &[u8; 4], u16, u16); 6] = [
    (BLOCK_PRESENT, 0xFF, b"BOOT", 0x00A8, 0x0040),
    (BLOCK_PRESENT, 0xFE, b"SETU", 0x00E8, 0x0100),
    (BLOCK_PRESENT, 0xFD, b"LOG ", 0x01E8, 0x0310),
    (BLOCK_PRESENT, 0xFC, b"OASY", 0x04F8, 0x0020),
    (BLOCK_PRESENT, 0xFB, b"XENI", 0x0518, 0x0010),
    (BLOCK_PRESENT, 0xFA, b"TOWN", 0x0528, 0x0100),
];
/// Default header words at 00A0h-00A7h: end of the used area, word A2h, header
/// sum placeholder and signature.
const DEFAULT_HEADER_WORDS: [u16; 4] = [0x0628, 0x0199, 0x0000, SIGNATURE];
/// Default block number table written by function 00h.
const DEFAULT_BLOCK_NUMBERS: [u8; BLOCK_COUNT] = [
    0, 1, 2, 3, 4, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
];

/// Status: undefined function.
const STATUS_UNDEFINED: u8 = 0x01;
/// Status: block number out of range.
const STATUS_BLOCK_NUMBER: u8 = 0x02;
/// Status: header signature wrong.
const STATUS_SIGNATURE: u8 = 0x03;
/// Status: hard error with the detail in CX.
const STATUS_HARD_ERROR: u8 = 0x80;
/// Detail: the transfer range exceeds the block.
const DETAIL_RANGE: u16 = 0x0004;
/// Detail: the header sum is wrong.
const DETAIL_HEADER_SUM: u16 = 0x0020;
/// Detail: the block is not present or its ID does not match.
const DETAIL_BLOCK: u16 = 0x0040;

/// Outcome of a CMOS BIOS function.
pub(super) enum CmosStatus {
    /// Success.
    Ok,
    /// An error status without detail.
    Error(u8),
    /// A hard error with the detail returned in CX.
    HardError(u16),
}

impl<T: TraceSink> TownsBus<T> {
    /// Runs the CMOS BIOS (vector F4h). `AH` selects the function in the
    /// signed range FDh-20h. The functions 00h, 05h, 10h, 11h and 20h are
    /// implemented. Every other function returns `AH = 01h` (undefined
    /// function) with CF set and logs a warning on its first call.
    pub(super) fn hle_cmos_bios(&mut self, cpu: &mut impl Cpu) {
        self.set_cmos_port_word(CMOS_SAVED_AX, cpu.ax());
        self.set_cmos_port_word(CMOS_SAVED_SS, cpu.ss());
        self.set_cmos_port_word(CMOS_SAVED_SP, cpu.sp().wrapping_sub(2));

        let status = match cpu.ah() {
            0x00 => {
                self.cmos_initialize();
                CmosStatus::Ok
            }
            0x05 => {
                cpu.set_bx(self.cmos_word(HEADER_WORD_A2));
                CmosStatus::Ok
            }
            0x10 => self.cmos_write_block(cpu),
            0x11 => self.cmos_read_block(cpu),
            0x20 => match self.cmos_header_status() {
                Ok(bad_blocks) => {
                    cpu.set_bx(bad_blocks);
                    CmosStatus::Ok
                }
                Err(status) => status,
            },
            function => {
                if !self.bios_state.undefined_cmos_function_reported {
                    self.bios_state.undefined_cmos_function_reported = true;
                    warn!("machine_towns: CMOS BIOS function {function:02X}h is not implemented");
                }
                CmosStatus::Error(STATUS_UNDEFINED)
            }
        };

        match status {
            CmosStatus::Ok => Self::set_bios_status(cpu, 0),
            CmosStatus::Error(code) => Self::set_bios_status(cpu, code),
            CmosStatus::HardError(detail) => {
                cpu.set_cx(detail);
                Self::set_bios_status(cpu, STATUS_HARD_ERROR);
            }
        }
    }

    /// Reads the little-endian CMOS word at index `index`.
    fn cmos_word(&self, index: usize) -> u16 {
        u16::from_le_bytes([
            self.memory.cmos_byte(index),
            self.memory.cmos_byte(index + 1),
        ])
    }

    /// Writes the little-endian CMOS word at index `index`.
    fn set_cmos_word(&mut self, index: usize, value: u16) {
        let [low, high] = value.to_le_bytes();
        self.memory.set_cmos_byte(index, low);
        self.memory.set_cmos_byte(index + 1, high);
    }

    /// Returns the sum of the header words 0000h-00A3h.
    fn cmos_header_sum(&self) -> u16 {
        (0..HEADER_SUM)
            .step_by(2)
            .fold(0u16, |sum, index| sum.wrapping_add(self.cmos_word(index)))
    }

    /// Returns the CMOS address and size of block `block`.
    fn cmos_block_extent(&self, block: usize) -> (usize, usize) {
        let descriptor = block * DESCRIPTOR_SIZE;
        (
            usize::from(self.cmos_word(descriptor + DESCRIPTOR_ADDRESS)),
            usize::from(self.cmos_word(descriptor + DESCRIPTOR_SIZE_WORD)),
        )
    }

    /// Returns the checksum byte of block `block`: the negated byte sum.
    fn cmos_block_checksum(&self, block: usize) -> u8 {
        let (address, size) = self.cmos_block_extent(block);
        let sum = (address..address + size).fold(0u8, |sum, index| {
            sum.wrapping_add(self.memory.cmos_byte(index))
        });
        0u8.wrapping_sub(sum)
    }

    /// Function 00h: writes the default header, clears the blocks and the
    /// checksum table and writes the block number table.
    fn cmos_initialize(&mut self) {
        for index in 0..HEADER_SIZE {
            self.memory.set_cmos_byte(index, 0);
        }
        for (block, (flag, identifier, name, address, size)) in
            DEFAULT_BLOCKS.into_iter().enumerate()
        {
            let descriptor = block * DESCRIPTOR_SIZE;
            self.memory.set_cmos_byte(descriptor, flag);
            self.memory.set_cmos_byte(descriptor + 1, identifier);
            for (offset, &character) in name.iter().enumerate() {
                self.memory
                    .set_cmos_byte(descriptor + 2 + offset, character);
            }
            self.set_cmos_word(descriptor + DESCRIPTOR_ADDRESS, address);
            self.set_cmos_word(descriptor + DESCRIPTOR_SIZE_WORD, size);
        }
        for (index, word) in DEFAULT_HEADER_WORDS.into_iter().enumerate() {
            self.set_cmos_word(HEADER_END_OF_BLOCKS + index * 2, word);
        }
        let end_of_blocks = usize::from(self.cmos_word(HEADER_END_OF_BLOCKS));
        for index in end_of_blocks..CMOS_DATA_END {
            self.memory.set_cmos_byte(index, 0);
        }
        for (block, number) in DEFAULT_BLOCK_NUMBERS.into_iter().enumerate() {
            self.memory
                .set_cmos_byte(BLOCK_NUMBER_TABLE + block, number);
        }
        let sum = self.cmos_header_sum();
        self.set_cmos_word(HEADER_SUM, sum);
        for block in 0..BLOCK_COUNT {
            self.memory.set_cmos_byte(CHECKSUM_TABLE + block, 0);
        }
    }

    /// Function 20h check: returns the bit mask of blocks with a bad checksum,
    /// or the error status of a bad signature or header sum.
    pub(super) fn cmos_header_status(&self) -> Result<u16, CmosStatus> {
        if self.cmos_word(HEADER_SIGNATURE) != SIGNATURE {
            return Err(CmosStatus::Error(STATUS_SIGNATURE));
        }
        if self.cmos_header_sum() != self.cmos_word(HEADER_SUM) {
            return Err(CmosStatus::HardError(DETAIL_HEADER_SUM));
        }
        let mut bad_blocks = 0u16;
        for block in 0..BLOCK_COUNT {
            let flags = self.memory.cmos_byte(block * DESCRIPTOR_SIZE);
            if flags & BLOCK_CHECKSUMMED == 0 {
                continue;
            }
            if self.memory.cmos_byte(CHECKSUM_TABLE + block) != self.cmos_block_checksum(block) {
                bad_blocks |= 1 << block;
            }
        }
        Ok(bad_blocks)
    }

    /// Validates the block transfer registers of functions 10h and 11h.
    /// Returns the first CMOS index and the length, or the error status.
    fn cmos_transfer_range(&self, cpu: &impl Cpu) -> Result<(usize, usize), CmosStatus> {
        let block = usize::from(cpu.al());
        if block >= BLOCK_COUNT {
            return Err(CmosStatus::Error(STATUS_BLOCK_NUMBER));
        }
        let descriptor = block * DESCRIPTOR_SIZE;
        let flags = self.memory.cmos_byte(descriptor);
        let identifier = self.memory.cmos_byte(descriptor + 1);
        if flags & BLOCK_PRESENT == 0 || identifier != cpu.cl() {
            return Err(CmosStatus::HardError(DETAIL_BLOCK));
        }
        let (address, size) = self.cmos_block_extent(block);
        let offset = cpu.dx();
        let length = cpu.bx();
        match offset.checked_add(length) {
            Some(end) if usize::from(end) <= size => {
                Ok((address + usize::from(offset), usize::from(length)))
            }
            _ => Err(CmosStatus::HardError(DETAIL_RANGE)),
        }
    }

    /// Function 10h: copies `BX` bytes from `DS:DI` into block `AL` at offset
    /// `DX` and updates the checksum of a checksummed block.
    fn cmos_write_block(&mut self, cpu: &impl Cpu) -> CmosStatus {
        let (start, length) = match self.cmos_transfer_range(cpu) {
            Ok(range) => range,
            Err(status) => return status,
        };
        if length == 0 {
            return CmosStatus::Ok;
        }
        let source = cpu
            .segment_base(SegmentRegister::DS)
            .wrapping_add(u32::from(cpu.di()));
        for index in 0..length {
            let value = self.hle_read_byte(source.wrapping_add(index as u32));
            self.memory.set_cmos_byte(start + index, value);
        }
        let block = usize::from(cpu.al());
        if self.memory.cmos_byte(block * DESCRIPTOR_SIZE) & BLOCK_CHECKSUMMED != 0 {
            let checksum = self.cmos_block_checksum(block);
            self.memory.set_cmos_byte(CHECKSUM_TABLE + block, checksum);
        }
        CmosStatus::Ok
    }

    /// Function 11h: copies `BX` bytes of block `AL` at offset `DX` to `DS:DI`.
    fn cmos_read_block(&mut self, cpu: &impl Cpu) -> CmosStatus {
        let (start, length) = match self.cmos_transfer_range(cpu) {
            Ok(range) => range,
            Err(status) => return status,
        };
        let destination = cpu
            .segment_base(SegmentRegister::DS)
            .wrapping_add(u32::from(cpu.di()));
        for index in 0..length {
            let value = self.memory.cmos_byte(start + index);
            self.hle_write_byte(destination.wrapping_add(index as u32), value);
        }
        CmosStatus::Ok
    }
}

#[cfg(test)]
mod tests {
    use common::{Cpu, SegmentRegister};

    use super::*;
    use crate::bus::bios::{
        VECTOR_CMOS_BIOS, cmos_index,
        tests::{hle_bus, real_mode_cpu, trap},
    };

    /// Physical address of the transfer buffer in the tests.
    const BUFFER: u32 = 0x2000;

    /// Calls the CMOS BIOS with the given registers and buffer at DS:DI = 0200:0000.
    fn call(
        bus: &mut TownsBus,
        ax: u16,
        bx: u16,
        cx: u16,
        dx: u16,
    ) -> super::super::tests::TestCpu {
        let mut cpu = real_mode_cpu();
        cpu.load_segment_real_mode(SegmentRegister::DS, (BUFFER >> 4) as u16);
        cpu.set_di(0);
        cpu.set_ax(ax);
        cpu.set_bx(bx);
        cpu.set_cx(cx);
        cpu.set_dx(dx);
        trap(bus, &mut cpu, VECTOR_CMOS_BIOS);
        cpu
    }

    #[test]
    fn default_cmos_passes_the_header_check() {
        let mut bus = hle_bus();
        let cpu = call(&mut bus, 0x2000, 0, 0, 0);
        assert_eq!(cpu.ah(), 0);
        assert_eq!(cpu.flags() & 1, 0);
    }

    #[test]
    fn read_block_copies_the_boot_block() {
        let mut bus = hle_bus();
        let cpu = call(&mut bus, 0x1100, 0x0016, 0x00FF, 0x0000);
        assert_eq!(cpu.ah(), 0);
        assert_eq!(cpu.al(), 0);
        for offset in 0..0x16u32 {
            assert_eq!(
                bus.peek_byte(BUFFER + offset),
                bus.memory.cmos_byte(0x00A8 + offset as usize)
            );
        }
    }

    #[test]
    fn read_block_rejects_wrong_identifier_and_range() {
        let mut bus = hle_bus();
        let cpu = call(&mut bus, 0x1100, 1, 0x00FE, 0);
        assert_eq!(cpu.ah(), 0x80);
        assert_eq!(cpu.cx(), DETAIL_BLOCK);
        assert_eq!(cpu.flags() & 1, 1);

        let cpu = call(&mut bus, 0x1100, 0x0041, 0x00FF, 0);
        assert_eq!(cpu.ah(), 0x80);
        assert_eq!(cpu.cx(), DETAIL_RANGE);

        let cpu = call(&mut bus, 0x1110, 1, 0x00FF, 0);
        assert_eq!(cpu.ah(), STATUS_BLOCK_NUMBER);
    }

    #[test]
    fn write_then_read_round_trips() {
        let mut bus = hle_bus();
        for offset in 0..4 {
            bus.poke_byte(BUFFER + offset, 0xA0 + offset as u8);
        }
        let cpu = call(&mut bus, 0x1001, 4, 0x00FE, 0x00B0);
        assert_eq!(cpu.ah(), 0);
        for offset in 0..4 {
            bus.poke_byte(BUFFER + offset, 0);
        }
        let cpu = call(&mut bus, 0x1101, 4, 0x00FE, 0x00B0);
        assert_eq!(cpu.ah(), 0);
        for offset in 0..4 {
            assert_eq!(bus.peek_byte(BUFFER + offset), 0xA0 + offset as u8);
        }
    }

    #[test]
    fn function_05_returns_header_word() {
        let mut bus = hle_bus();
        let cpu = call(&mut bus, 0x0500, 0, 0, 0);
        assert_eq!(cpu.ah(), 0);
        assert_eq!(cpu.bx(), bus.cmos_word(HEADER_WORD_A2));
    }

    #[test]
    fn initialize_writes_a_valid_header_and_check_detects_corruption() {
        let mut bus = hle_bus();
        bus.memory.set_cmos_byte(0x0003, 0x00);
        let cpu = call(&mut bus, 0x2000, 0, 0, 0);
        assert_eq!(cpu.ah(), 0x80);
        assert_eq!(cpu.cx(), DETAIL_HEADER_SUM);

        bus.memory.set_cmos_byte(HEADER_SIGNATURE, 0);
        let cpu = call(&mut bus, 0x2000, 0, 0, 0);
        assert_eq!(cpu.ah(), STATUS_SIGNATURE);

        let cpu = call(&mut bus, 0x0000, 0, 0, 0);
        assert_eq!(cpu.ah(), 0);
        let cpu = call(&mut bus, 0x2000, 0, 0, 0);
        assert_eq!(cpu.ah(), 0);
        assert_eq!(bus.cmos_word(HEADER_SUM), bus.cmos_header_sum());
        assert_eq!(bus.memory.cmos_byte(cmos_index(0x3140)), 0x28);
    }

    #[test]
    fn write_to_a_checksummed_block_keeps_the_check_clean_until_corruption() {
        let mut bus = hle_bus();
        let setup_descriptor = DESCRIPTOR_SIZE;
        bus.memory
            .set_cmos_byte(setup_descriptor, BLOCK_PRESENT | BLOCK_CHECKSUMMED);
        let sum = bus.cmos_header_sum();
        bus.set_cmos_word(HEADER_SUM, sum);
        for offset in 0..4 {
            bus.poke_byte(BUFFER + offset, 0x11 * (offset as u8 + 1));
        }

        let cpu = call(&mut bus, 0x1001, 4, 0x00FE, 0x0000);
        assert_eq!(cpu.ah(), 0);
        let cpu = call(&mut bus, 0x2000, 0, 0, 0);
        assert_eq!(cpu.ah(), 0);
        assert_eq!(cpu.bx(), 0);

        bus.memory.set_cmos_byte(0x00E8, 0x5A);
        let cpu = call(&mut bus, 0x2000, 0, 0, 0);
        assert_eq!(cpu.ah(), 0);
        assert_eq!(cpu.bx(), 1 << 1);
    }

    #[test]
    fn caller_ax_ss_and_sp_are_saved_to_the_cmos() {
        let port_word = |bus: &TownsBus, port: u16| {
            u16::from_le_bytes([
                bus.memory.cmos_byte(cmos_index(port)),
                bus.memory.cmos_byte(cmos_index(port + 2)),
            ])
        };
        let mut bus = hle_bus();
        let mut cpu = real_mode_cpu();
        cpu.load_segment_real_mode(SegmentRegister::SS, 0x0123);
        cpu.set_ax(0x0500);
        trap(&mut bus, &mut cpu, VECTOR_CMOS_BIOS);
        assert_eq!(port_word(&bus, CMOS_SAVED_AX), 0x0500);
        assert_eq!(port_word(&bus, CMOS_SAVED_SS), 0x0123);
        assert_eq!(port_word(&bus, CMOS_SAVED_SP), 0x7FFE);
    }

    #[test]
    fn undefined_function_sets_carry() {
        let mut bus = hle_bus();
        let cpu = call(&mut bus, 0x0700, 0, 0, 0);
        assert_eq!(cpu.ah(), STATUS_UNDEFINED);
        assert_eq!(cpu.flags() & 1, 1);
    }
}
