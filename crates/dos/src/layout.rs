//! Structure layouts of the DOS internal tables.
//!
//! A layout places the list of lists, the first SFT block and the swappable
//! data area in the DOS data segment and gives the entry sizes and the field
//! offsets of the list of lists, the SFT, the CDS, the DPB, the disk buffer
//! chain and the swappable data area. Field offsets that both layouts share
//! are constants in `tables`.
//!
//! NeetanDos reaches the SFT blocks, the CDS array, the DPB chain and the
//! buffer chain through the pointers of the list of lists. A foreign `IO.SYS`
//! may allocate and relink these tables at any time.
//!
//! The DOS 6.20 layout is the layout of NeetanDos on the PC-98. The DOS 3.10
//! layout is the layout of the kernel under the FM Towns `IO.SYS`.

use crate::{MemoryAccess, filesystem::fat_bpb::Bpb};

/// Size of one paragraph in bytes.
const PARAGRAPH_BYTES: u16 = 16;

/// Offset of a far pointer that ends a chain.
const END_OF_CHAIN_OFFSET: u16 = 0xFFFF;

/// Upper bound of blocks walked in one SFT chain.
const MAX_SFT_BLOCKS: usize = 64;

/// Highest SFT index that a job file table byte can name.
const MAX_SFT_INDEX: u16 = 0xFE;

/// Number of SFT indices that a job file table byte can name.
pub(crate) const SFT_INDEX_COUNT: usize = MAX_SFT_INDEX as usize + 1;

save_state::runtime_state_enum! {
    /// DOS version whose internal structure layout NeetanDos writes and reads.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum DosLayout {
        /// MS-DOS 3.10 as loaded by the FM Towns `IO.SYS`.
        Dos310 = 0,
        /// MS-DOS 6.20 as NeetanDos builds it on the PC-98.
        Dos620 = 1,
    }
}

impl DosLayout {
    /// Returns the structure layout of this DOS version.
    pub const fn structures(self) -> &'static StructureLayout {
        match self {
            Self::Dos310 => &DOS310_LAYOUT,
            Self::Dos620 => &DOS620_LAYOUT,
        }
    }
}

/// Width of a DOS table field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldWidth {
    /// One byte.
    Byte,
    /// One little-endian word.
    Word,
    /// One little-endian double word.
    DoubleWord,
}

impl FieldWidth {
    /// Returns the largest value that a field of this width holds.
    pub const fn max_value(self) -> u32 {
        match self {
            Self::Byte => u8::MAX as u32,
            Self::Word => u16::MAX as u32,
            Self::DoubleWord => u32::MAX,
        }
    }

    /// Reads a field of this width at `address`.
    pub fn read(self, memory: &dyn MemoryAccess, address: u32) -> u32 {
        match self {
            Self::Byte => u32::from(memory.read_byte(address)),
            Self::Word => u32::from(memory.read_word(address)),
            Self::DoubleWord => {
                u32::from(memory.read_word(address))
                    | (u32::from(memory.read_word(address + 2)) << 16)
            }
        }
    }

    /// Writes `value` saturated to the largest value of this width at `address`.
    pub fn write(self, memory: &mut dyn MemoryAccess, address: u32, value: u32) {
        let value = value.min(self.max_value());
        match self {
            Self::Byte => memory.write_byte(address, value as u8),
            Self::Word => memory.write_word(address, value as u16),
            Self::DoubleWord => {
                memory.write_word(address, value as u16);
                memory.write_word(address + 2, (value >> 16) as u16);
            }
        }
    }
}

/// Place and offsets of every DOS internal table of one DOS version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StructureLayout {
    /// List of lists (`INT 21h AH=52h`).
    pub list_of_lists: ListOfListsLayout,
    /// System file table blocks.
    pub system_file_table: SystemFileTableLayout,
    /// Current directory structure array.
    pub current_directory: CurrentDirectoryLayout,
    /// Drive parameter blocks.
    pub drive_parameter_block: DriveParameterBlockLayout,
    /// Disk buffer chain.
    pub disk_buffer: DiskBufferLayout,
    /// Swappable data area (`INT 21h AX=5D06h`).
    pub swappable_data_area: SwappableDataAreaLayout,
    /// Private stubs and tables of NeetanDos in the DOS data segment.
    pub private_area: PrivateAreaLayout,
}

/// Place of the list of lists and its version specific fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListOfListsLayout {
    /// Offset of the list of lists in the DOS data segment.
    pub data_offset: u16,
    /// Paragraphs that `INT 21h AH=52h` subtracts from the DOS data segment.
    /// The returned offset grows by the same amount. A bias keeps the first
    /// MCB word below the list of lists addressable through `ES:[BX-2]`.
    pub pointer_paragraph_bias: u16,
    /// Fields of DOS 4.0 and later behind the NUL device header.
    pub extension: Option<ListOfListsExtension>,
}

/// Fields of the list of lists that DOS 4.0 and later add behind the NUL
/// device header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListOfListsExtension {
    /// Offset of the byte count of JOINed drives.
    pub join_drive_count: u32,
    /// Offset of the far pointer to the SETVER program list.
    pub setver_list: u32,
    /// Offset of the word count of `BUFFERS=`.
    pub buffer_count: u32,
    /// Offset of the word count of the lookahead buffers.
    pub lookahead_buffer_count: u32,
    /// Offset of the byte boot drive (1 = A:).
    pub boot_drive: u32,
    /// Offset of the byte flag for 32-bit moves (386 or later).
    pub dword_move_flag: u32,
    /// Offset of the word extended memory size in KiB.
    pub extended_memory_kb: u32,
}

/// Place, entry size and version specific entry fields of the system file
/// table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemFileTableLayout {
    /// Offset of the first SFT block header in the DOS data segment.
    pub first_block_offset: u16,
    /// Size of one SFT entry in bytes.
    pub entry_size: u32,
    /// Offset of the word absolute cluster of the last access.
    pub last_cluster: u32,
    /// Offset of the sector number of the directory entry.
    pub directory_sector: u32,
    /// Width of the directory sector field.
    pub directory_sector_width: FieldWidth,
}

/// Entry size of the current directory structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurrentDirectoryLayout {
    /// Size of one CDS entry in bytes.
    pub entry_size: u32,
}

/// Size and version specific fields of a drive parameter block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriveParameterBlockLayout {
    /// Size of one DPB in bytes.
    pub size: u32,
    /// Offset of the sectors per FAT field.
    pub sectors_per_fat: u32,
    /// Width of the sectors per FAT field.
    pub sectors_per_fat_width: FieldWidth,
    /// Offset of the word first root directory sector.
    pub first_root_sector: u32,
    /// Offset of the far pointer to the device driver header.
    pub device: u32,
    /// Offset of the byte media descriptor.
    pub media_descriptor: u32,
    /// Offset of the byte access flag (`FFh` = not accessed yet).
    pub access_flag: u32,
    /// Offset of the far pointer to the next DPB.
    pub next: u32,
    /// Offset of the word cluster where the free cluster search starts.
    pub free_search_cluster: u32,
    /// Offset of the word free cluster count (`FFFFh` = unknown).
    pub free_cluster_count: u32,
}

/// Header size of a disk buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskBufferLayout {
    /// Size of the buffer header in front of the sector data.
    pub header_size: u32,
}

/// Place and sizes of the swappable data area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwappableDataAreaLayout {
    /// Offset of the SDA in the DOS data segment.
    pub data_offset: u16,
    /// Size of the area that is swapped while DOS is busy (`CX` of `AX=5D06h`).
    pub size: u16,
    /// Size of the area that is always swapped (`DX` of `AX=5D06h`).
    pub always_swapped_size: u16,
    /// Offset of the first byte behind the SDA fields that guest code uses.
    /// NeetanDos keeps its guest call area from there to the end of the SDA.
    pub private_tail_offset: u16,
}

/// Places of the private stubs and tables of NeetanDos in the DOS data
/// segment. Guest code reaches them only through the pointers that NeetanDos
/// hands out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrivateAreaLayout {
    /// DBCS lead byte table of `INT 21h AX=6300h`.
    pub dbcs_table: u16,
    /// Default critical error handler (`MOV AL, 3`, `IRET`).
    pub critical_error_stub: u16,
    /// Far trap stub of the `INT 2Fh AX=122Eh` error message retriever.
    pub error_retriever_stub: u16,
    /// Counted message buffer of the error message retriever.
    pub error_message_buffer: u16,
    /// Interrupt stubs of the kernel entry. `None` for a layout without one.
    pub kernel_stubs: Option<u16>,
    /// Transfer buffer of one CD sector for the guest CD-ROM drives. `None`
    /// for a layout without one.
    pub transfer_buffer: Option<u16>,
    /// First disk buffer and the DPBs behind it. The first MCB follows them.
    pub system_data: u16,
    /// Free range in front of the system data that holds the DPBs when all
    /// of them fit. `None` for a layout without one.
    pub parameter_block_window: Option<DataRange>,
}

/// Range of bytes in the DOS data segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataRange {
    /// Offset of the first byte.
    pub offset: u16,
    /// Size in bytes.
    pub size: u16,
}

/// DOS 6.20 layout of NeetanDos on the PC-98.
pub const DOS620_LAYOUT: StructureLayout = StructureLayout {
    list_of_lists: ListOfListsLayout {
        data_offset: 0x0000,
        pointer_paragraph_bias: 1,
        extension: Some(ListOfListsExtension {
            join_drive_count: 0x34,
            setver_list: 0x37,
            buffer_count: 0x3F,
            lookahead_buffer_count: 0x41,
            boot_drive: 0x43,
            dword_move_flag: 0x44,
            extended_memory_kb: 0x45,
        }),
    },
    system_file_table: SystemFileTableLayout {
        first_block_offset: 0x0090,
        entry_size: 0x3B,
        last_cluster: 0x35,
        directory_sector: 0x1B,
        directory_sector_width: FieldWidth::DoubleWord,
    },
    current_directory: CurrentDirectoryLayout { entry_size: 0x58 },
    drive_parameter_block: DriveParameterBlockLayout {
        size: 0x21,
        sectors_per_fat: 0x0F,
        sectors_per_fat_width: FieldWidth::Word,
        first_root_sector: 0x11,
        device: 0x13,
        media_descriptor: 0x17,
        access_flag: 0x18,
        next: 0x19,
        free_search_cluster: 0x1D,
        free_cluster_count: 0x1F,
    },
    // DOS 5.0 and later buffer header size. No NeetanDos code reads it on
    // the PC-98.
    disk_buffer: DiskBufferLayout { header_size: 0x14 },
    swappable_data_area: SwappableDataAreaLayout {
        data_offset: 0x0F40,
        size: 0x078C,
        always_swapped_size: 0x001A,
        private_tail_offset: 0x10CC,
    },
    private_area: PrivateAreaLayout {
        dbcs_table: 0x0D3E,
        critical_error_stub: 0x0D92,
        error_retriever_stub: 0x0D96,
        error_message_buffer: 0x0DA8,
        kernel_stubs: None,
        transfer_buffer: None,
        // End of the self boot area, the CD-ROM device stubs end at `1700h`.
        system_data: 0x1700,
        // Between the CDS array (`01C0h`, 26 entries of `58h`) and the FCB
        // SFT at `0D38h`. It holds 19 DPBs.
        parameter_block_window: Some(DataRange {
            offset: 0x0AB0,
            size: 0x0288,
        }),
    },
};

/// DOS 3.10 layout of the kernel under the FM Towns `IO.SYS`.
///
/// The list of lists has no fields behind the NUL header. A CDS entry holds
/// the root length at `4Fh`.
pub const DOS310_LAYOUT: StructureLayout = StructureLayout {
    list_of_lists: ListOfListsLayout {
        data_offset: 0x0026,
        pointer_paragraph_bias: 0,
        extension: None,
    },
    system_file_table: SystemFileTableLayout {
        first_block_offset: 0x0098,
        entry_size: 0x35,
        last_cluster: 0x1B,
        directory_sector: 0x1D,
        directory_sector_width: FieldWidth::Word,
    },
    current_directory: CurrentDirectoryLayout { entry_size: 0x51 },
    drive_parameter_block: DriveParameterBlockLayout {
        size: 0x20,
        sectors_per_fat: 0x0F,
        sectors_per_fat_width: FieldWidth::Byte,
        first_root_sector: 0x10,
        device: 0x12,
        media_descriptor: 0x16,
        access_flag: 0x17,
        next: 0x18,
        free_search_cluster: 0x1C,
        free_cluster_count: 0x1E,
    },
    disk_buffer: DiskBufferLayout { header_size: 0x10 },
    swappable_data_area: SwappableDataAreaLayout {
        data_offset: 0x02CE,
        size: 0x0738,
        always_swapped_size: 0x0018,
        // CTL.SYS takes the addresses of the words at `051Eh`-`0585h`.
        private_tail_offset: 0x0586,
    },
    // The window `0A10h`-`0B7Fh` lies behind the SDA and in front of the
    // CTL.SYS pattern at `11D2h`. The transfer buffer lies behind the word
    // `425Eh` that MENU_DRV changes. The real kernel keeps its first disk
    // buffer at `6AA0h`.
    private_area: PrivateAreaLayout {
        dbcs_table: 0x0A10,
        critical_error_stub: 0x0A18,
        error_retriever_stub: 0x0A1C,
        error_message_buffer: 0x0A28,
        kernel_stubs: Some(0x0AA0),
        transfer_buffer: Some(0x4300),
        system_data: 0x6AA0,
        parameter_block_window: None,
    },
};

impl ListOfListsLayout {
    /// Returns the far pointer `(segment, offset)` that `INT 21h AH=52h`
    /// returns for a DOS data segment.
    pub const fn returned_pointer(&self, dos_data_segment: u16) -> (u16, u16) {
        (
            dos_data_segment - self.pointer_paragraph_bias,
            self.data_offset + self.pointer_paragraph_bias * PARAGRAPH_BYTES,
        )
    }
}

impl SystemFileTableLayout {
    /// Returns the directory sector and the entry index in that sector of the
    /// SFT entry at `entry_address`.
    pub fn read_directory_location(
        &self,
        memory: &dyn MemoryAccess,
        entry_address: u32,
    ) -> (u32, u8) {
        (
            self.directory_sector_width
                .read(memory, entry_address + self.directory_sector),
            memory.read_byte(entry_address + crate::tables::SFT_ENT_DIR_INDEX),
        )
    }

    /// Writes the directory sector and the entry index in that sector of the
    /// SFT entry at `entry_address`.
    pub fn write_directory_location(
        &self,
        memory: &mut dyn MemoryAccess,
        entry_address: u32,
        sector: u32,
        index: u8,
    ) {
        self.directory_sector_width
            .write(memory, entry_address + self.directory_sector, sector);
        memory.write_byte(entry_address + crate::tables::SFT_ENT_DIR_INDEX, index);
    }

    /// Writes the cluster position of the SFT entry at `entry_address`: the
    /// relative cluster and the absolute cluster of the last access.
    pub fn write_cluster_position(
        &self,
        memory: &mut dyn MemoryAccess,
        entry_address: u32,
        relative_cluster: u16,
        last_cluster: u16,
    ) {
        memory.write_word(
            entry_address + crate::tables::SFT_ENT_REL_CLUSTER,
            relative_cluster,
        );
        memory.write_word(entry_address + self.last_cluster, last_cluster);
    }
}

impl SwappableDataAreaLayout {
    /// Returns the offset of the InDOS flag in the DOS data segment.
    pub const fn indos_offset(&self) -> u16 {
        self.data_offset + crate::tables::SDA_OFF_INDOS
    }
}

/// Geometry fields of a drive parameter block that `INT 21h AH=53h` derives
/// from a BIOS parameter block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DpbGeometry {
    /// Bytes per sector.
    pub bytes_per_sector: u16,
    /// Sectors per cluster minus one.
    pub cluster_mask: u8,
    /// Shift count from sectors to clusters.
    pub cluster_shift: u8,
    /// Sector of the first FAT (reserved sectors).
    pub first_fat_sector: u16,
    /// Number of FAT copies.
    pub fat_count: u8,
    /// Number of root directory entries.
    pub root_entries: u16,
    /// First sector of the data area.
    pub first_data_sector: u16,
    /// Highest cluster number plus one.
    pub max_cluster: u16,
    /// Sectors per FAT.
    pub sectors_per_fat: u16,
    /// First sector of the root directory.
    pub first_root_sector: u16,
    /// Media descriptor byte.
    pub media_descriptor: u8,
}

impl DpbGeometry {
    /// Returns the BIOS parameter block that this geometry derives from. The
    /// total sector count follows from the data area and the clusters.
    pub fn to_bpb(&self) -> Bpb {
        let sectors_per_cluster = self.cluster_mask.wrapping_add(1).max(1);
        let total_sectors = u32::from(self.first_data_sector)
            + u32::from(self.max_cluster.saturating_sub(1)) * u32::from(sectors_per_cluster);
        let (total_sectors_16, total_sectors_32) = match u16::try_from(total_sectors) {
            Ok(total) => (total, 0),
            Err(_) => (0, total_sectors),
        };
        Bpb {
            bytes_per_sector: self.bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors: self.first_fat_sector,
            num_fats: self.fat_count,
            root_entry_count: self.root_entries,
            total_sectors_16,
            media_descriptor: self.media_descriptor,
            sectors_per_fat: self.sectors_per_fat,
            total_sectors_32,
        }
    }

    /// Derives the DPB geometry of a volume from its BIOS parameter block.
    pub fn from_bpb(bpb: &Bpb) -> Self {
        let sectors_per_cluster = bpb.sectors_per_cluster.max(1);
        let max_cluster = bpb.total_sectors().saturating_sub(bpb.first_data_sector())
            / u32::from(sectors_per_cluster)
            + 1;
        Self {
            bytes_per_sector: bpb.bytes_per_sector,
            cluster_mask: sectors_per_cluster - 1,
            cluster_shift: sectors_per_cluster.trailing_zeros() as u8,
            first_fat_sector: bpb.reserved_sectors,
            fat_count: bpb.num_fats,
            root_entries: bpb.root_entry_count,
            first_data_sector: saturate_to_word(bpb.first_data_sector()),
            max_cluster: saturate_to_word(max_cluster),
            sectors_per_fat: bpb.sectors_per_fat,
            first_root_sector: saturate_to_word(bpb.first_root_sector()),
            media_descriptor: bpb.media_descriptor,
        }
    }

    /// Writes the geometry fields of a DPB at `address`.
    pub fn write(&self, memory: &mut dyn MemoryAccess, address: u32, layout: &StructureLayout) {
        use crate::tables::*;
        let dpb = &layout.drive_parameter_block;
        memory.write_word(address + DPB_OFF_BYTES_PER_SECTOR, self.bytes_per_sector);
        memory.write_byte(address + DPB_OFF_CLUSTER_MASK, self.cluster_mask);
        memory.write_byte(address + DPB_OFF_CLUSTER_SHIFT, self.cluster_shift);
        memory.write_word(address + DPB_OFF_RESERVED_SECTORS, self.first_fat_sector);
        memory.write_byte(address + DPB_OFF_NUM_FATS, self.fat_count);
        memory.write_word(address + DPB_OFF_ROOT_ENTRIES, self.root_entries);
        memory.write_word(address + DPB_OFF_FIRST_DATA_SECTOR, self.first_data_sector);
        memory.write_word(address + DPB_OFF_MAX_CLUSTER, self.max_cluster);
        dpb.sectors_per_fat_width.write(
            memory,
            address + dpb.sectors_per_fat,
            u32::from(self.sectors_per_fat),
        );
        memory.write_word(address + dpb.first_root_sector, self.first_root_sector);
        memory.write_byte(address + dpb.media_descriptor, self.media_descriptor);
    }

    /// Reads the geometry fields of the DPB at `address`.
    pub fn read(memory: &dyn MemoryAccess, address: u32, layout: &StructureLayout) -> Self {
        use crate::tables::*;
        let dpb = &layout.drive_parameter_block;
        Self {
            bytes_per_sector: memory.read_word(address + DPB_OFF_BYTES_PER_SECTOR),
            cluster_mask: memory.read_byte(address + DPB_OFF_CLUSTER_MASK),
            cluster_shift: memory.read_byte(address + DPB_OFF_CLUSTER_SHIFT),
            first_fat_sector: memory.read_word(address + DPB_OFF_RESERVED_SECTORS),
            fat_count: memory.read_byte(address + DPB_OFF_NUM_FATS),
            root_entries: memory.read_word(address + DPB_OFF_ROOT_ENTRIES),
            first_data_sector: memory.read_word(address + DPB_OFF_FIRST_DATA_SECTOR),
            max_cluster: memory.read_word(address + DPB_OFF_MAX_CLUSTER),
            sectors_per_fat: dpb
                .sectors_per_fat_width
                .read(memory, address + dpb.sectors_per_fat) as u16,
            first_root_sector: memory.read_word(address + dpb.first_root_sector),
            media_descriptor: memory.read_byte(address + dpb.media_descriptor),
        }
    }
}

/// Converts a sector or cluster number to a DPB word, saturating at `FFFFh`.
fn saturate_to_word(value: u32) -> u16 {
    value.min(u32::from(u16::MAX)) as u16
}

/// Reads a far pointer at `address` and returns `(segment, offset)`.
pub(crate) fn read_far_pointer(memory: &dyn MemoryAccess, address: u32) -> (u16, u16) {
    (memory.read_word(address + 2), memory.read_word(address))
}

/// Returns the linear address of a far pointer `(segment, offset)`.
pub(crate) fn far_pointer_address((segment, offset): (u16, u16)) -> u32 {
    (u32::from(segment) << 4) + u32::from(offset)
}

/// Returns the linear address of SFT entry `index`. The walk starts at the
/// first SFT block of the list of lists at `list_of_lists_address` and
/// follows the block chain.
pub(crate) fn system_file_table_entry_address(
    memory: &dyn MemoryAccess,
    list_of_lists_address: u32,
    layout: &StructureLayout,
    index: u8,
) -> Option<u32> {
    use crate::tables::{
        SFT_BLOCK_OFF_COUNT, SFT_BLOCK_OFF_NEXT, SFT_HEADER_SIZE, SYSVARS_OFF_SFT_PTR,
    };
    let mut block = read_far_pointer(memory, list_of_lists_address + SYSVARS_OFF_SFT_PTR);
    let mut remaining = u16::from(index);
    for _ in 0..MAX_SFT_BLOCKS {
        if block.1 == END_OF_CHAIN_OFFSET {
            return None;
        }
        let block_address = far_pointer_address(block);
        let count = memory.read_word(block_address + SFT_BLOCK_OFF_COUNT);
        if remaining < count {
            return Some(
                block_address
                    + SFT_HEADER_SIZE
                    + u32::from(remaining) * layout.system_file_table.entry_size,
            );
        }
        remaining -= count;
        block = read_far_pointer(memory, block_address + SFT_BLOCK_OFF_NEXT);
    }
    None
}

/// Returns the number of SFT entries in the chain of the list of lists at
/// `list_of_lists_address`, capped at the highest index a handle can name
/// plus one.
pub(crate) fn system_file_table_entry_count(
    memory: &dyn MemoryAccess,
    list_of_lists_address: u32,
) -> u16 {
    use crate::tables::{SFT_BLOCK_OFF_COUNT, SFT_BLOCK_OFF_NEXT, SYSVARS_OFF_SFT_PTR};
    let mut block = read_far_pointer(memory, list_of_lists_address + SYSVARS_OFF_SFT_PTR);
    let mut total: u16 = 0;
    for _ in 0..MAX_SFT_BLOCKS {
        if block.1 == END_OF_CHAIN_OFFSET {
            break;
        }
        let block_address = far_pointer_address(block);
        total = total.saturating_add(memory.read_word(block_address + SFT_BLOCK_OFF_COUNT));
        block = read_far_pointer(memory, block_address + SFT_BLOCK_OFF_NEXT);
    }
    total.min(MAX_SFT_INDEX + 1)
}

/// Returns the linear address of the CDS entry of `drive_index` (0 = A:) in
/// the CDS array of the list of lists at `list_of_lists_address`.
pub(crate) fn current_directory_entry_address(
    memory: &dyn MemoryAccess,
    list_of_lists_address: u32,
    layout: &StructureLayout,
    drive_index: u8,
) -> u32 {
    let array = read_far_pointer(
        memory,
        list_of_lists_address + crate::tables::SYSVARS_OFF_CDS_PTR,
    );
    far_pointer_address(array) + u32::from(drive_index) * layout.current_directory.entry_size
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{tables::*, test_support::MockMemory};

    const LAYOUTS: [DosLayout; 2] = [DosLayout::Dos310, DosLayout::Dos620];

    /// Size of the conventional memory of the mock.
    const TEST_MEMORY_SIZE: usize = 0x10_0000;

    /// Returns a DPB geometry with distinct values in every field.
    fn sample_geometry() -> DpbGeometry {
        DpbGeometry {
            bytes_per_sector: 0x0400,
            cluster_mask: 0x01,
            cluster_shift: 0x01,
            first_fat_sector: 0x0001,
            fat_count: 0x02,
            root_entries: 0x00C0,
            first_data_sector: 0x000B,
            max_cluster: 0x04CF,
            sectors_per_fat: 0x0002,
            first_root_sector: 0x0005,
            media_descriptor: 0xFE,
        }
    }

    #[test]
    fn dpb_geometry_round_trips_in_both_layouts() {
        for layout in LAYOUTS {
            let mut memory = MockMemory::with_extended_memory(TEST_MEMORY_SIZE, 0);
            let geometry = sample_geometry();
            geometry.write(&mut memory, 0x1000, layout.structures());
            assert_eq!(
                DpbGeometry::read(&memory, 0x1000, layout.structures()),
                geometry
            );
        }
    }

    #[test]
    fn dpb_geometry_stays_inside_the_block() {
        for layout in LAYOUTS {
            let structures = layout.structures();
            let mut memory = MockMemory::with_extended_memory(TEST_MEMORY_SIZE, 0);
            let geometry = DpbGeometry {
                bytes_per_sector: 0xFFFF,
                cluster_mask: 0xFF,
                cluster_shift: 0xFF,
                first_fat_sector: 0xFFFF,
                fat_count: 0xFF,
                root_entries: 0xFFFF,
                first_data_sector: 0xFFFF,
                max_cluster: 0xFFFF,
                sectors_per_fat: 0x00FF,
                first_root_sector: 0xFFFF,
                media_descriptor: 0xFF,
            };
            geometry.write(&mut memory, 0x1000, structures);
            let dpb = &structures.drive_parameter_block;
            assert_eq!(memory.read_byte(0x1000 + DPB_OFF_DRIVE_NUM), 0);
            assert_eq!(memory.read_byte(0x1000 + DPB_OFF_UNIT_NUM), 0);
            for offset in [dpb.device, dpb.access_flag, dpb.next] {
                assert_eq!(
                    memory.read_byte(0x1000 + offset),
                    0,
                    "{layout:?} {offset:#x}"
                );
            }
            assert_eq!(memory.read_byte(0x1000 + dpb.size), 0);
        }
    }

    #[test]
    fn dos310_dpb_stores_a_byte_sectors_per_fat() {
        let structures = DosLayout::Dos310.structures();
        let mut memory = MockMemory::with_extended_memory(TEST_MEMORY_SIZE, 0);
        sample_geometry().write(&mut memory, 0x1000, structures);
        assert_eq!(memory.read_byte(0x1000 + 0x0F), 0x02);
        assert_eq!(memory.read_word(0x1000 + 0x10), 0x0005);
        assert_eq!(memory.read_byte(0x1000 + 0x16), 0xFE);
    }

    #[test]
    fn dos620_dpb_stores_a_word_sectors_per_fat() {
        let structures = DosLayout::Dos620.structures();
        let mut memory = MockMemory::with_extended_memory(TEST_MEMORY_SIZE, 0);
        sample_geometry().write(&mut memory, 0x1000, structures);
        assert_eq!(memory.read_word(0x1000 + 0x0F), 0x0002);
        assert_eq!(memory.read_word(0x1000 + 0x11), 0x0005);
        assert_eq!(memory.read_byte(0x1000 + 0x17), 0xFE);
    }

    #[test]
    fn version_specific_fields_lie_inside_their_structures() {
        for layout in LAYOUTS {
            let structures = layout.structures();
            let dpb = &structures.drive_parameter_block;
            for offset in [
                dpb.sectors_per_fat,
                dpb.first_root_sector + 1,
                dpb.device + 3,
                dpb.media_descriptor,
                dpb.access_flag,
                dpb.next + 3,
                dpb.free_search_cluster + 1,
                dpb.free_cluster_count + 1,
            ] {
                assert!(offset < dpb.size, "{layout:?} DPB {offset:#x}");
            }
            let cds = &structures.current_directory;
            assert!(CDS_OFF_ROOT_LENGTH + 1 < cds.entry_size);
            assert!(CDS_OFF_DPB_PTR + 3 < cds.entry_size);
            let sft = &structures.system_file_table;
            assert!(SFT_ENT_PSP_OWNER + 1 < sft.entry_size);
            assert!(sft.last_cluster + 1 < sft.entry_size);
            let directory_sector_bytes = match sft.directory_sector_width {
                FieldWidth::Byte => 1,
                FieldWidth::Word => 2,
                FieldWidth::DoubleWord => 4,
            };
            assert!(sft.directory_sector + directory_sector_bytes <= SFT_ENT_DIR_INDEX);
            let buffer = &structures.disk_buffer;
            assert!(BUFFER_OFF_DRIVE < buffer.header_size);
            let sda = &structures.swappable_data_area;
            assert!(u32::from(SDA_OFF_BREAK_FLAG) < u32::from(sda.always_swapped_size));
            assert!(sda.always_swapped_size < sda.size);
            assert!(sda.private_tail_offset > sda.data_offset + sda.always_swapped_size);
            assert!(sda.private_tail_offset < sda.data_offset + sda.size);
        }
    }

    #[test]
    fn dos310_places_the_tables_of_the_contract() {
        let structures = DosLayout::Dos310.structures();
        assert_eq!(structures.list_of_lists.data_offset, 0x0026);
        assert_eq!(
            structures.list_of_lists.returned_pointer(0x1076),
            (0x1076, 0x0026)
        );
        assert_eq!(structures.system_file_table.first_block_offset, 0x0098);
        assert_eq!(structures.system_file_table.entry_size, 0x35);
        assert_eq!(structures.current_directory.entry_size, 0x51);
        assert_eq!(structures.drive_parameter_block.size, 0x20);
        assert_eq!(structures.drive_parameter_block.next, 0x18);
        assert_eq!(structures.disk_buffer.header_size, 0x10);
        assert_eq!(structures.swappable_data_area.data_offset, 0x02CE);
        assert_eq!(structures.swappable_data_area.indos_offset(), 0x02CF);
        assert_eq!(structures.swappable_data_area.size, 0x0738);
        assert_eq!(structures.swappable_data_area.always_swapped_size, 0x0018);
        let nul_offset = structures.list_of_lists.data_offset as u32 + SYSVARS_OFF_NUL_HEADER;
        assert_eq!(nul_offset, 0x0048);
    }

    #[test]
    fn dos310_sft_entry_keeps_the_dos_3_fields() {
        let sft = &DosLayout::Dos310.structures().system_file_table;
        let mut memory = MockMemory::with_extended_memory(TEST_MEMORY_SIZE, 0);
        sft.write_cluster_position(&mut memory, 0x1000, 0x0003, 0x0123);
        sft.write_directory_location(&mut memory, 0x1000, 0x0001_2345, 0x07);
        assert_eq!(memory.read_word(0x1000 + 0x19), 0x0003);
        assert_eq!(memory.read_word(0x1000 + 0x1B), 0x0123);
        assert_eq!(memory.read_word(0x1000 + 0x1D), 0xFFFF);
        assert_eq!(memory.read_byte(0x1000 + 0x1F), 0x07);
        assert_eq!(memory.read_byte(0x1000 + 0x20), 0x00);
        assert_eq!(sft.read_directory_location(&memory, 0x1000), (0xFFFF, 0x07));
    }

    #[test]
    fn dos620_sft_entry_uses_the_dos_4_fields() {
        let sft = &DosLayout::Dos620.structures().system_file_table;
        let mut memory = MockMemory::with_extended_memory(TEST_MEMORY_SIZE, 0);
        sft.write_cluster_position(&mut memory, 0x1000, 0x0003, 0x0123);
        sft.write_directory_location(&mut memory, 0x1000, 0x0001_2345, 0x07);
        assert_eq!(memory.read_word(0x1000 + 0x19), 0x0003);
        assert_eq!(memory.read_word(0x1000 + 0x1B), 0x2345);
        assert_eq!(memory.read_word(0x1000 + 0x1D), 0x0001);
        assert_eq!(memory.read_byte(0x1000 + 0x1F), 0x07);
        assert_eq!(memory.read_byte(0x1000 + 0x20), 0x00);
        assert_eq!(memory.read_word(0x1000 + 0x35), 0x0123);
        assert_eq!(
            sft.read_directory_location(&memory, 0x1000),
            (0x0001_2345, 0x07)
        );
    }

    #[test]
    fn field_width_saturates_on_write() {
        let mut memory = MockMemory::with_extended_memory(TEST_MEMORY_SIZE, 0);
        FieldWidth::Byte.write(&mut memory, 0x1000, 0x0123);
        assert_eq!(FieldWidth::Byte.read(&memory, 0x1000), 0xFF);
        FieldWidth::Word.write(&mut memory, 0x1000, 0x0001_2345);
        assert_eq!(FieldWidth::Word.read(&memory, 0x1000), 0xFFFF);
        FieldWidth::DoubleWord.write(&mut memory, 0x1000, 0x8765_4321);
        assert_eq!(FieldWidth::DoubleWord.read(&memory, 0x1000), 0x8765_4321);
    }

    #[test]
    fn dos620_returns_the_list_of_lists_one_paragraph_lower() {
        let structures = DosLayout::Dos620.structures();
        assert_eq!(
            structures.list_of_lists.returned_pointer(0x0200),
            (0x01FF, 0x0010)
        );
    }

    #[test]
    fn sft_walk_follows_the_block_chain() {
        for layout in LAYOUTS {
            let structures = layout.structures();
            let mut memory = MockMemory::with_extended_memory(TEST_MEMORY_SIZE, 0);
            let list_of_lists = 0x2000;
            write_far_ptr(
                &mut memory,
                list_of_lists + SYSVARS_OFF_SFT_PTR,
                0x0300,
                0x0004,
            );
            let first_block = 0x3004;
            write_far_ptr(
                &mut memory,
                first_block + SFT_BLOCK_OFF_NEXT,
                0x0400,
                0x0000,
            );
            memory.write_word(first_block + SFT_BLOCK_OFF_COUNT, 5);
            let second_block = 0x4000;
            write_far_ptr(
                &mut memory,
                second_block + SFT_BLOCK_OFF_NEXT,
                0xFFFF,
                0xFFFF,
            );
            memory.write_word(second_block + SFT_BLOCK_OFF_COUNT, 3);

            let entry_size = structures.system_file_table.entry_size;
            assert_eq!(
                system_file_table_entry_address(&memory, list_of_lists, structures, 4),
                Some(first_block + SFT_HEADER_SIZE + 4 * entry_size)
            );
            assert_eq!(
                system_file_table_entry_address(&memory, list_of_lists, structures, 5),
                Some(second_block + SFT_HEADER_SIZE)
            );
            assert_eq!(
                system_file_table_entry_address(&memory, list_of_lists, structures, 7),
                Some(second_block + SFT_HEADER_SIZE + 2 * entry_size)
            );
            assert_eq!(
                system_file_table_entry_address(&memory, list_of_lists, structures, 8),
                None
            );
            assert_eq!(system_file_table_entry_count(&memory, list_of_lists), 8);
        }
    }

    #[test]
    fn cds_entry_follows_the_array_pointer() {
        for layout in LAYOUTS {
            let structures = layout.structures();
            let mut memory = MockMemory::with_extended_memory(TEST_MEMORY_SIZE, 0);
            let list_of_lists = 0x2000;
            write_far_ptr(
                &mut memory,
                list_of_lists + SYSVARS_OFF_CDS_PTR,
                0xBF00,
                0x0010,
            );
            assert_eq!(
                current_directory_entry_address(&memory, list_of_lists, structures, 3),
                0xBF010 + 3 * structures.current_directory.entry_size
            );
        }
    }

    /// Builds a BPB from the fields that the DPB geometry depends on.
    fn bpb(
        bytes_per_sector: u16,
        sectors_per_cluster: u8,
        root_entry_count: u16,
        total_sectors: u32,
        media_descriptor: u8,
        sectors_per_fat: u16,
    ) -> Bpb {
        Bpb {
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors: 1,
            num_fats: 2,
            root_entry_count,
            total_sectors_16: u16::try_from(total_sectors).unwrap_or(0),
            media_descriptor,
            sectors_per_fat,
            total_sectors_32: if total_sectors > u32::from(u16::MAX) {
                total_sectors
            } else {
                0
            },
        }
    }

    #[test]
    fn fat12_with_512_byte_sectors() {
        let geometry = DpbGeometry::from_bpb(&bpb(512, 2, 112, 1280, 0xFB, 2));
        assert_eq!(geometry.bytes_per_sector, 512);
        assert_eq!(geometry.cluster_mask, 1);
        assert_eq!(geometry.cluster_shift, 1);
        assert_eq!(geometry.first_fat_sector, 1);
        assert_eq!(geometry.fat_count, 2);
        assert_eq!(geometry.root_entries, 112);
        assert_eq!(geometry.first_root_sector, 5);
        assert_eq!(geometry.first_data_sector, 12);
        assert_eq!(geometry.max_cluster, 635);
        assert_eq!(geometry.sectors_per_fat, 2);
        assert_eq!(geometry.media_descriptor, 0xFB);
    }

    #[test]
    fn fat12_with_1024_byte_sectors() {
        let geometry = DpbGeometry::from_bpb(&bpb(1024, 1, 192, 1232, 0xFE, 2));
        assert_eq!(geometry.bytes_per_sector, 1024);
        assert_eq!(geometry.cluster_mask, 0);
        assert_eq!(geometry.cluster_shift, 0);
        assert_eq!(geometry.first_root_sector, 5);
        assert_eq!(geometry.first_data_sector, 11);
        assert_eq!(geometry.max_cluster, 1222);
        assert_eq!(geometry.media_descriptor, 0xFE);
    }

    #[test]
    fn fat12_with_2048_byte_sectors() {
        let geometry = DpbGeometry::from_bpb(&bpb(2048, 4, 512, 0x1E00, 0xF9, 3));
        assert_eq!(geometry.bytes_per_sector, 2048);
        assert_eq!(geometry.cluster_mask, 3);
        assert_eq!(geometry.cluster_shift, 2);
        assert_eq!(geometry.first_root_sector, 7);
        assert_eq!(geometry.first_data_sector, 15);
        assert_eq!(geometry.max_cluster, 1917);
        assert_eq!(geometry.sectors_per_fat, 3);
    }

    #[test]
    fn fat16_with_512_byte_sectors() {
        let geometry = DpbGeometry::from_bpb(&bpb(512, 4, 512, 81_920, 0xF8, 80));
        assert_eq!(geometry.first_root_sector, 161);
        assert_eq!(geometry.first_data_sector, 193);
        assert_eq!(geometry.max_cluster, 20_432);
        assert_eq!(geometry.sectors_per_fat, 80);
        assert_eq!(geometry.media_descriptor, 0xF8);
    }

    #[test]
    fn fat16_with_1024_byte_sectors() {
        let geometry = DpbGeometry::from_bpb(&bpb(1024, 2, 512, 40_000, 0xF8, 40));
        assert_eq!(geometry.first_root_sector, 81);
        assert_eq!(geometry.first_data_sector, 97);
        assert_eq!(geometry.max_cluster, 19_952);
    }

    #[test]
    fn fat16_with_2048_byte_sectors() {
        let geometry = DpbGeometry::from_bpb(&bpb(2048, 2, 1024, 20_000, 0xFA, 32));
        assert_eq!(geometry.first_root_sector, 65);
        assert_eq!(geometry.first_data_sector, 81);
        assert_eq!(geometry.max_cluster, 9_960);
        assert_eq!(geometry.sectors_per_fat, 32);
    }

    #[test]
    fn large_volume_saturates_the_cluster_count() {
        let geometry = DpbGeometry::from_bpb(&bpb(512, 1, 512, 1_000_000, 0xF8, 255));
        assert_eq!(geometry.max_cluster, u16::MAX);
    }

    #[test]
    fn bpb_of_a_geometry_round_trips_a_volume_with_whole_clusters() {
        let original = bpb(1024, 1, 192, 1232, 0xFE, 2);
        let restored = DpbGeometry::from_bpb(&original).to_bpb();
        assert_eq!(restored, original);
        let original = bpb(2048, 4, 512, 0x1E00 - 0x0D, 0xF9, 3);
        let geometry = DpbGeometry::from_bpb(&original);
        assert_eq!(geometry.to_bpb().total_sectors(), original.total_sectors());
    }
}
