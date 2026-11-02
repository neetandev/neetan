//! First disk buffer and drive parameter blocks of the DOS data segment.
//!
//! The DOS data segment ends with the first disk buffer and the DPBs of the
//! drives. The buffer header comes first, then the buffer data of the
//! largest sector of all drives (`LoL+10h`), then one DPB per drive with a
//! DPB in drive order. The first MCB follows the last DPB. The layout names
//! the offset of the buffer header. The real DOS 3.10 kernel keeps the same
//! order with the buffer at `6AA0h`.
//!
//! A layout with a DPB window keeps the DPBs in that window when all of them
//! fit. The first MCB then follows the buffer.

use crate::{
    MemoryAccess,
    layout::StructureLayout,
    tables::{
        BUFFER_OFF_DRIVE, BUFFER_OFF_NEXT, SYSVARS_OFF_BUFFER_PTR, SYSVARS_OFF_DPB_PTR,
        SYSVARS_OFF_MAX_SECTOR, write_far_ptr,
    },
};

/// Smallest buffer data size in bytes.
pub(crate) const MINIMUM_SECTOR_SIZE: u16 = 512;

/// Drive byte of a buffer header that holds no sector.
const BUFFER_DRIVE_NONE: u8 = 0xFF;

/// Bytes per paragraph.
const PARAGRAPH_BYTES: u32 = 16;

/// Places of the first disk buffer and the DPBs in the DOS data segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SystemDataArea {
    /// Offset of the buffer header.
    buffer_offset: u16,
    /// Size of the buffer header.
    buffer_header_size: u16,
    /// Size of the buffer data, the largest sector of all drives.
    sector_size: u16,
    /// Number of DPBs.
    parameter_block_count: u16,
    /// Size of one DPB.
    parameter_block_size: u16,
    /// Offset of the first DPB.
    parameter_block_base: u16,
}

impl SystemDataArea {
    /// Places the buffer at the system data offset of `layout` and
    /// `parameter_block_count` DPBs in the DPB window of `layout` or behind
    /// the buffer. The buffer holds `largest_sector` bytes, at least
    /// `MINIMUM_SECTOR_SIZE`.
    pub(crate) fn new(
        layout: &StructureLayout,
        parameter_block_count: usize,
        largest_sector: u16,
    ) -> Self {
        let buffer_offset = layout.private_area.system_data;
        let buffer_header_size = layout.disk_buffer.header_size as u16;
        let sector_size = largest_sector.max(MINIMUM_SECTOR_SIZE);
        let parameter_block_size = layout.drive_parameter_block.size as u16;
        let parameter_blocks_size = parameter_block_count * usize::from(parameter_block_size);
        let parameter_block_base = match layout.private_area.parameter_block_window {
            Some(window) if parameter_blocks_size <= usize::from(window.size) => window.offset,
            Some(_) | None => buffer_offset + buffer_header_size + sector_size,
        };
        Self {
            buffer_offset,
            buffer_header_size,
            sector_size,
            parameter_block_count: parameter_block_count as u16,
            parameter_block_size,
            parameter_block_base,
        }
    }

    /// Returns the number of DPBs.
    pub(crate) fn parameter_block_count(&self) -> usize {
        usize::from(self.parameter_block_count)
    }

    /// Returns the offset of DPB `index`.
    pub(crate) fn parameter_block_offset(&self, index: usize) -> u16 {
        self.parameter_block_base + index as u16 * self.parameter_block_size
    }

    /// Returns the offset of the first byte behind the buffer and the DPBs.
    pub(crate) fn end_offset(&self) -> u32 {
        let buffer_end = u32::from(self.buffer_offset)
            + u32::from(self.buffer_header_size)
            + u32::from(self.sector_size);
        let parameter_blocks_end = u32::from(self.parameter_block_base)
            + u32::from(self.parameter_block_count) * u32::from(self.parameter_block_size);
        buffer_end.max(parameter_blocks_end)
    }

    /// Returns the segment of the first MCB behind the area in the DOS data
    /// segment `dos_data_segment`.
    pub(crate) fn first_mcb_segment(&self, dos_data_segment: u16) -> u16 {
        dos_data_segment + self.end_offset().div_ceil(PARAGRAPH_BYTES) as u16
    }

    /// Writes the empty buffer header and the list of lists fields of the
    /// buffer chain, the largest sector and the first DPB. The DPBs
    /// themselves come from the drive map.
    pub(crate) fn write_buffer_and_pointers(
        &self,
        memory: &mut dyn MemoryAccess,
        dos_data_segment: u16,
        list_of_lists_address: u32,
    ) {
        let buffer = (u32::from(dos_data_segment) << 4) + u32::from(self.buffer_offset);
        let zeros = vec![0u8; usize::from(self.buffer_header_size + self.sector_size)];
        memory.write_block(buffer, &zeros);
        write_far_ptr(memory, buffer + BUFFER_OFF_NEXT, 0xFFFF, 0xFFFF);
        memory.write_byte(buffer + BUFFER_OFF_DRIVE, BUFFER_DRIVE_NONE);

        write_far_ptr(
            memory,
            list_of_lists_address + SYSVARS_OFF_BUFFER_PTR,
            dos_data_segment,
            self.buffer_offset,
        );
        memory.write_word(
            list_of_lists_address + SYSVARS_OFF_MAX_SECTOR,
            self.sector_size,
        );
        let (segment, offset) = if self.parameter_block_count == 0 {
            (0xFFFF, 0xFFFF)
        } else {
            (dos_data_segment, self.parameter_block_offset(0))
        };
        write_far_ptr(
            memory,
            list_of_lists_address + SYSVARS_OFF_DPB_PTR,
            segment,
            offset,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        layout::{DOS310_LAYOUT, DOS620_LAYOUT},
        test_support::MockMemory,
    };

    #[test]
    fn dos310_area_matches_the_real_kernel() {
        let floppy_boot = SystemDataArea::new(&DOS310_LAYOUT, 3, 0x0400);
        assert_eq!(floppy_boot.buffer_offset, 0x6AA0);
        assert_eq!(floppy_boot.parameter_block_offset(0), 0x6EB0);
        let hard_disk_boot = SystemDataArea::new(&DOS310_LAYOUT, 4, 0x0800);
        assert_eq!(hard_disk_boot.parameter_block_offset(0), 0x72B0);
        assert_eq!(hard_disk_boot.end_offset(), 0x7330);
        assert_eq!(hard_disk_boot.first_mcb_segment(0x1076), 0x1076 + 0x0733);
    }

    #[test]
    fn dos620_parameter_blocks_fill_the_window_and_the_mcb_follows_the_buffer() {
        let area = SystemDataArea::new(&DOS620_LAYOUT, 19, 0x0400);
        let buffer_end = u32::from(area.buffer_offset) + 0x14 + 0x0400;
        assert_eq!(area.parameter_block_offset(0), 0x0AB0);
        assert!(u32::from(area.parameter_block_offset(19)) <= 0x0D38);
        assert_eq!(area.end_offset(), buffer_end);
        assert_eq!(area.first_mcb_segment(0x0200), 0x0200 + 0x01B2);
    }

    #[test]
    fn dos620_parameter_blocks_beyond_the_window_follow_the_buffer() {
        let area = SystemDataArea::new(&DOS620_LAYOUT, 20, 0x0400);
        let buffer_end = u32::from(area.buffer_offset) + 0x14 + 0x0400;
        assert_eq!(u32::from(area.parameter_block_offset(0)), buffer_end);
        assert_eq!(area.end_offset(), buffer_end + 20 * 0x21);
        assert!(u32::from(area.first_mcb_segment(0x0200) - 0x0200) * 16 >= area.end_offset());
    }

    #[test]
    fn buffer_holds_at_least_a_512_byte_sector() {
        let area = SystemDataArea::new(&DOS620_LAYOUT, 1, 0);
        assert_eq!(area.sector_size, MINIMUM_SECTOR_SIZE);
    }

    #[test]
    fn pointers_name_the_buffer_and_the_first_parameter_block() {
        let mut memory = MockMemory::with_extended_memory(0x10_0000, 0);
        let area = SystemDataArea::new(&DOS310_LAYOUT, 2, 0x0400);
        let list_of_lists = 0x1_0000 + 0x26;
        area.write_buffer_and_pointers(&mut memory, 0x1000, list_of_lists);
        let buffer = 0x1_0000 + 0x6AA0;
        assert_eq!(memory.read_word(buffer), 0xFFFF);
        assert_eq!(
            memory.read_byte(buffer + BUFFER_OFF_DRIVE),
            BUFFER_DRIVE_NONE
        );
        assert_eq!(
            memory.read_word(list_of_lists + SYSVARS_OFF_BUFFER_PTR),
            0x6AA0
        );
        assert_eq!(
            memory.read_word(list_of_lists + SYSVARS_OFF_MAX_SECTOR),
            0x0400
        );
        assert_eq!(
            memory.read_word(list_of_lists + SYSVARS_OFF_DPB_PTR),
            0x6EB0
        );
        assert_eq!(
            memory.read_word(list_of_lists + SYSVARS_OFF_DPB_PTR + 2),
            0x1000
        );
    }
}
