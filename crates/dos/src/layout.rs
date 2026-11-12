//! Structure formats and data area layouts of the DOS internal tables.
//!
//! A structure format belongs to one DOS version. It gives the entry sizes
//! and the version specific field offsets of the list of lists, the SFT, the
//! CDS, the DPB and the disk buffer. Field offsets that both formats share
//! are constants in `tables`.
//!
//! A data area layout belongs to one platform. It places the list of lists,
//! the first SFT block, the swappable data area, the private stubs and
//! tables of NeetanDos and the regions of the PC-98 self boot in the DOS data
//! segment.
//!
//! NeetanDos reaches the SFT blocks, the CDS array, the DPB chain and the
//! buffer chain through the pointers of the list of lists. A foreign `IO.SYS`
//! may allocate and relink these tables at any time.
//!
//! The DOS 6.20 format and the PC-98 6.20 data area belong to the self boot
//! of NeetanDos on the PC-98. The DOS 3.10 format and the FM Towns data area
//! belong to the kernel under the FM Towns `IO.SYS`.

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
    /// DOS version whose structure format NeetanDos writes and reads.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum DosFormat {
        /// MS-DOS 3.10 as loaded by the FM Towns `IO.SYS`.
        Dos310 = 0,
        /// MS-DOS 6.20 as NeetanDos builds it on the PC-98.
        Dos620 = 1,
    }
}

impl DosFormat {
    /// Returns the structure format of this DOS version.
    pub const fn structures(self) -> &'static StructureFormat {
        match self {
            Self::Dos310 => &DOS310_FORMAT,
            Self::Dos620 => &DOS620_FORMAT,
        }
    }
}

save_state::runtime_state_enum! {
    /// Platform whose data area layout NeetanDos writes and reads.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum DataArea {
        /// Kernel under the FM Towns `IO.SYS`.
        FmTowns = 0,
        /// MS-DOS 6.20 self boot of NeetanDos on the PC-98.
        Pc98Dos620 = 1,
    }
}

impl DataArea {
    /// Every data area.
    pub const ALL: [Self; 2] = [Self::FmTowns, Self::Pc98Dos620];

    /// Returns the structure format of the DOS version of this platform.
    pub const fn format(self) -> DosFormat {
        match self {
            Self::FmTowns => DosFormat::Dos310,
            Self::Pc98Dos620 => DosFormat::Dos620,
        }
    }

    /// Returns the data area layout of this platform.
    pub const fn layout(self) -> &'static DataAreaLayout {
        match self {
            Self::FmTowns => &FM_TOWNS_DATA_AREA,
            Self::Pc98Dos620 => &PC98_DOS620_DATA_AREA,
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

/// Entry sizes and version specific field offsets of the DOS internal
/// tables of one DOS version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StructureFormat {
    /// Fields of DOS 4.0 and later behind the NUL device header of the list
    /// of lists (`INT 21h AH=52h`).
    pub list_of_lists_extension: Option<ListOfListsExtension>,
    /// System file table entries.
    pub system_file_table: SystemFileTableFormat,
    /// Current directory structure entries.
    pub current_directory: CurrentDirectoryFormat,
    /// Drive parameter blocks.
    pub drive_parameter_block: DriveParameterBlockFormat,
    /// Disk buffer headers.
    pub disk_buffer: DiskBufferFormat,
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

/// Entry size and version specific entry fields of the system file table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemFileTableFormat {
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
pub struct CurrentDirectoryFormat {
    /// Size of one CDS entry in bytes.
    pub entry_size: u32,
}

/// Size and version specific fields of a drive parameter block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriveParameterBlockFormat {
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
pub struct DiskBufferFormat {
    /// Size of the buffer header in front of the sector data.
    pub header_size: u32,
}

/// Places of the DOS tables of one platform in the DOS data segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataAreaLayout {
    /// List of lists (`INT 21h AH=52h`).
    pub list_of_lists: ListOfListsLayout,
    /// Offset of the first SFT block header.
    pub first_sft_block_offset: u16,
    /// Swappable data area (`INT 21h AX=5D06h`).
    pub swappable_data_area: SwappableDataAreaLayout,
    /// Private stubs and tables of NeetanDos.
    pub private_area: PrivateAreaLayout,
    /// Regions that the PC-98 self boot builds. `None` for a platform whose
    /// `IO.SYS` builds the device chain and the drive tables.
    pub self_boot: Option<SelfBootLayout>,
}

/// Place of the list of lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListOfListsLayout {
    /// Offset of the list of lists in the DOS data segment.
    pub data_offset: u16,
    /// Paragraphs that `INT 21h AH=52h` subtracts from the DOS data segment.
    /// The returned offset grows by the same amount. A bias keeps the first
    /// MCB word below the list of lists addressable through `ES:[BX-2]`.
    pub pointer_paragraph_bias: u16,
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
    /// Interrupt stubs of the kernel entry. `None` for a data area without one.
    pub kernel_stubs: Option<u16>,
    /// Transfer buffer of one CD sector for the guest CD-ROM drives. `None`
    /// for a data area without one.
    pub transfer_buffer: Option<u16>,
    /// First disk buffer and the DPBs behind it. The first MCB follows them.
    pub system_data: u16,
    /// Free range in front of the system data that holds the DPBs when all
    /// of them fit. `None` for a data area without one.
    pub parameter_block_window: Option<DataRange>,
}

/// Places of the device headers, the drive tables and the stubs that the
/// PC-98 self boot writes into the DOS data segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelfBootLayout {
    /// `CON` device header.
    pub con_device: u16,
    /// `CLOCK` device header outside the device chain. The list of lists
    /// points at it.
    pub clock_device: u16,
    /// `$AID#NEC` device header.
    pub aid_nec_device: u16,
    /// Device header of the CD-ROM driver.
    pub cdrom_device: u16,
    /// `XMSXXXX0` device header.
    pub xms_device: u16,
    /// `EMMXXXX0` device header.
    pub ems_device: u16,
    /// CDS array with one entry per drive letter.
    pub current_directory_array: u16,
    /// Header of the FCB SFT block without entries.
    pub fcb_system_file_table: u16,
    /// XMS driver entry: the HIMEM.SYS prologue in front of a far trap stub.
    pub xms_entry_stub: u16,
    /// Strategy and interrupt routine of the `XMSXXXX0` and `EMMXXXX0`
    /// device headers.
    pub xms_device_stub: u16,
    /// `INT 67h` trap stub followed by the device name `EMMXXXX0`.
    pub ems_interrupt_stub: u16,
    /// Trap stub that the target of the EMS alter page map and call
    /// function returns through.
    pub ems_page_map_return_stub: u16,
    /// Default NetBIOS handler (`IRET`) of `INT 5Ch`.
    pub netbios_stub: u16,
    /// Tables of the Windows enhanced mode DOSMGR. `None` for a data area
    /// whose kernel offers no DOSMGR interface.
    pub windows_dosmgr: Option<WindowsDosmgrLayout>,
    /// Stubs and the mirror header of the CD-ROM device.
    pub cdrom_stubs: CdromStubLayout,
}

/// Places of the tables of the Windows enhanced mode DOSMGR. DOSMGR scans
/// the DOS data segment for the signature blocks. The patch table tells it
/// where the kernel keeps the InDOS flag, the save-DS and save-BX scratch
/// words, the critical section table and the UMB MCB chain head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowsDosmgrLayout {
    /// Signature blocks of the MS-DOS 6.20 kernel.
    pub signature_bank: DataRange,
    /// Patch table of `INT 2Fh AX=1607h BX=0015h CX=0000h`.
    pub patch_table: u16,
    /// Word segment of the UMB MCB chain head.
    pub umb_head: u16,
    /// Scratch word of the saved DS.
    pub saved_data_segment: u16,
    /// Scratch word of the saved BX.
    pub saved_bx: u16,
    /// Critical section table.
    pub critical_section_table: u16,
}

/// Places of the strategy and interrupt bridge of the CD-ROM device. The
/// mirror header is paragraph aligned. Callers that ignore the device header
/// offset still reach the strategy and interrupt entries through
/// `segment:0000`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CdromStubLayout {
    /// Far pointer to the active request header that the strategy stub sets.
    pub request_pointer: u16,
    /// Mirror of the CD-ROM device header.
    pub mirror_header: u16,
    /// Strategy routine.
    pub strategy_stub: u16,
    /// Interrupt routine.
    pub interrupt_stub: u16,
}

impl CdromStubLayout {
    /// Returns the strategy routine offset relative to the mirror header.
    pub const fn mirror_strategy_offset(&self) -> u16 {
        self.strategy_stub - self.mirror_header
    }

    /// Returns the interrupt routine offset relative to the mirror header.
    pub const fn mirror_interrupt_offset(&self) -> u16 {
        self.interrupt_stub - self.mirror_header
    }

    /// Returns the paragraphs from the DOS data segment to the mirror header.
    pub const fn mirror_header_paragraphs(&self) -> u16 {
        self.mirror_header / PARAGRAPH_BYTES
    }
}

/// Range of bytes in the DOS data segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataRange {
    /// Offset of the first byte.
    pub offset: u16,
    /// Size in bytes.
    pub size: u16,
}

/// DOS 6.20 structure format of NeetanDos on the PC-98.
pub const DOS620_FORMAT: StructureFormat = StructureFormat {
    list_of_lists_extension: Some(ListOfListsExtension {
        join_drive_count: 0x34,
        setver_list: 0x37,
        buffer_count: 0x3F,
        lookahead_buffer_count: 0x41,
        boot_drive: 0x43,
        dword_move_flag: 0x44,
        extended_memory_kb: 0x45,
    }),
    system_file_table: SystemFileTableFormat {
        entry_size: 0x3B,
        last_cluster: 0x35,
        directory_sector: 0x1B,
        directory_sector_width: FieldWidth::DoubleWord,
    },
    current_directory: CurrentDirectoryFormat { entry_size: 0x58 },
    drive_parameter_block: DriveParameterBlockFormat {
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
    disk_buffer: DiskBufferFormat { header_size: 0x14 },
};

/// DOS 3.10 structure format of the kernel under the FM Towns `IO.SYS`.
///
/// The list of lists has no fields behind the NUL header. A CDS entry holds
/// the root length at `4Fh`.
pub const DOS310_FORMAT: StructureFormat = StructureFormat {
    list_of_lists_extension: None,
    system_file_table: SystemFileTableFormat {
        entry_size: 0x35,
        last_cluster: 0x1B,
        directory_sector: 0x1D,
        directory_sector_width: FieldWidth::Word,
    },
    current_directory: CurrentDirectoryFormat { entry_size: 0x51 },
    drive_parameter_block: DriveParameterBlockFormat {
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
    disk_buffer: DiskBufferFormat { header_size: 0x10 },
};

/// Data area of the MS-DOS 6.20 self boot of NeetanDos on the PC-98.
///
/// The list of lists starts the segment. The device headers, the first SFT
/// block and the CDS array follow it. The DPB window, the FCB SFT, the stubs
/// and the DOSMGR tables lie in front of the SDA. The CD-ROM stubs follow
/// the SDA and end at the system data.
pub const PC98_DOS620_DATA_AREA: DataAreaLayout = DataAreaLayout {
    list_of_lists: ListOfListsLayout {
        data_offset: 0x0000,
        pointer_paragraph_bias: 1,
    },
    first_sft_block_offset: 0x0090,
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
        system_data: 0x1700,
        // Between the CDS array (`01C0h`, 26 entries of `58h`) and the FCB
        // SFT at `0D38h`. It holds 19 DPBs.
        parameter_block_window: Some(DataRange {
            offset: 0x0AB0,
            size: 0x0288,
        }),
    },
    self_boot: Some(SelfBootLayout {
        con_device: 0x0048,
        clock_device: 0x005A,
        aid_nec_device: 0x006C,
        cdrom_device: 0x007E,
        xms_device: 0x0D52,
        ems_device: 0x0D64,
        current_directory_array: 0x01C0,
        fcb_system_file_table: 0x0D38,
        xms_entry_stub: 0x0D44,
        xms_device_stub: 0x0D9F,
        ems_interrupt_stub: 0x0D76,
        ems_page_map_return_stub: 0x0D88,
        netbios_stub: 0x0D95,
        windows_dosmgr: Some(WindowsDosmgrLayout {
            signature_bank: DataRange {
                offset: 0x0E28,
                size: 0x0100,
            },
            patch_table: 0x0F28,
            umb_head: 0x0F36,
            saved_data_segment: 0x0F38,
            saved_bx: 0x0F3A,
            critical_section_table: 0x0F3C,
        }),
        cdrom_stubs: CdromStubLayout {
            request_pointer: 0x16CC,
            mirror_header: 0x16D0,
            strategy_stub: 0x16E4,
            interrupt_stub: 0x16F7,
        },
    }),
};

/// Data area of the kernel under the FM Towns `IO.SYS`.
///
/// The list of lists, the first SFT block and the SDA lie where the real
/// MS-DOS 3.10 kernel keeps them. `IO.SYS` builds the device chain and the
/// drive tables. The private window `0A10h`-`0B7Fh` lies behind the SDA and
/// in front of the CTL.SYS pattern at `11D2h`. The transfer buffer lies
/// behind the word `425Eh` that MENU_DRV changes. The real kernel keeps its
/// first disk buffer at `6AA0h`.
pub const FM_TOWNS_DATA_AREA: DataAreaLayout = DataAreaLayout {
    list_of_lists: ListOfListsLayout {
        data_offset: 0x0026,
        pointer_paragraph_bias: 0,
    },
    first_sft_block_offset: 0x0098,
    swappable_data_area: SwappableDataAreaLayout {
        data_offset: 0x02CE,
        size: 0x0738,
        always_swapped_size: 0x0018,
        // CTL.SYS takes the addresses of the words at `051Eh`-`0585h`.
        private_tail_offset: 0x0586,
    },
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
    self_boot: None,
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

impl SystemFileTableFormat {
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
    pub fn write(&self, memory: &mut dyn MemoryAccess, address: u32, format: &StructureFormat) {
        use crate::tables::*;
        let dpb = &format.drive_parameter_block;
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
    pub fn read(memory: &dyn MemoryAccess, address: u32, format: &StructureFormat) -> Self {
        use crate::tables::*;
        let dpb = &format.drive_parameter_block;
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
    format: &StructureFormat,
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
                    + u32::from(remaining) * format.system_file_table.entry_size,
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
    format: &StructureFormat,
    drive_index: u8,
) -> u32 {
    let array = read_far_pointer(
        memory,
        list_of_lists_address + crate::tables::SYSVARS_OFF_CDS_PTR,
    );
    far_pointer_address(array) + u32::from(drive_index) * format.current_directory.entry_size
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{tables::*, test_support::MockMemory};

    /// Highest tag that the completeness test of `DataArea::ALL` decodes.
    const HIGHEST_TESTED_TAG: u32 = 0xFF;

    /// Size of a word field in bytes.
    const WORD_SIZE: u32 = size_of::<u16>() as u32;

    /// Size of a far pointer in bytes.
    const FAR_POINTER_SIZE: u32 = 2 * WORD_SIZE;

    /// Returns the structure formats of every data area.
    fn formats() -> [DosFormat; DataArea::ALL.len()] {
        DataArea::ALL.map(DataArea::format)
    }

    /// Returns the size of `bytes` as a region size.
    fn byte_count(bytes: &[u8]) -> u32 {
        bytes.len() as u32
    }

    #[test]
    fn all_holds_every_data_area_once() {
        let mut decoded_count = 0;
        for tag in 0..=HIGHEST_TESTED_TAG {
            let mut encoded = Vec::new();
            save_state::StateEncode::encode_state(&tag, &mut encoded);
            let mut decoder = save_state::StateDecoder::new(&encoded, encoded.len());
            if let Ok(data_area) = <DataArea as save_state::StateDecode>::decode_state(&mut decoder)
            {
                assert!(DataArea::ALL.contains(&data_area), "{data_area:?}");
                decoded_count += 1;
            }
        }
        assert_eq!(decoded_count, DataArea::ALL.len());
    }

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
    fn dpb_geometry_round_trips_in_both_formats() {
        for format in formats() {
            let mut memory = MockMemory::with_extended_memory(TEST_MEMORY_SIZE, 0);
            let geometry = sample_geometry();
            geometry.write(&mut memory, 0x1000, format.structures());
            assert_eq!(
                DpbGeometry::read(&memory, 0x1000, format.structures()),
                geometry
            );
        }
    }

    #[test]
    fn dpb_geometry_stays_inside_the_block() {
        for format in formats() {
            let structures = format.structures();
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
                    "{format:?} {offset:#x}"
                );
            }
            assert_eq!(memory.read_byte(0x1000 + dpb.size), 0);
        }
    }

    #[test]
    fn dos310_dpb_stores_a_byte_sectors_per_fat() {
        let structures = DosFormat::Dos310.structures();
        let mut memory = MockMemory::with_extended_memory(TEST_MEMORY_SIZE, 0);
        sample_geometry().write(&mut memory, 0x1000, structures);
        assert_eq!(memory.read_byte(0x1000 + 0x0F), 0x02);
        assert_eq!(memory.read_word(0x1000 + 0x10), 0x0005);
        assert_eq!(memory.read_byte(0x1000 + 0x16), 0xFE);
    }

    #[test]
    fn dos620_dpb_stores_a_word_sectors_per_fat() {
        let structures = DosFormat::Dos620.structures();
        let mut memory = MockMemory::with_extended_memory(TEST_MEMORY_SIZE, 0);
        sample_geometry().write(&mut memory, 0x1000, structures);
        assert_eq!(memory.read_word(0x1000 + 0x0F), 0x0002);
        assert_eq!(memory.read_word(0x1000 + 0x11), 0x0005);
        assert_eq!(memory.read_byte(0x1000 + 0x17), 0xFE);
    }

    #[test]
    fn version_specific_fields_lie_inside_their_structures() {
        for format in formats() {
            let structures = format.structures();
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
                assert!(offset < dpb.size, "{format:?} DPB {offset:#x}");
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
        }
        for data_area in DataArea::ALL {
            let sda = &data_area.layout().swappable_data_area;
            assert!(u32::from(SDA_OFF_BREAK_FLAG) < u32::from(sda.always_swapped_size));
            assert!(sda.always_swapped_size < sda.size);
            assert!(sda.private_tail_offset > sda.data_offset + sda.always_swapped_size);
            assert!(sda.private_tail_offset < sda.data_offset + sda.size);
        }
    }

    #[test]
    fn fm_towns_data_area_places_the_tables_of_the_contract() {
        let structures = DataArea::FmTowns.format().structures();
        let data_area = DataArea::FmTowns.layout();
        assert_eq!(data_area.list_of_lists.data_offset, 0x0026);
        assert_eq!(
            data_area.list_of_lists.returned_pointer(0x1076),
            (0x1076, 0x0026)
        );
        assert_eq!(data_area.first_sft_block_offset, 0x0098);
        assert_eq!(structures.system_file_table.entry_size, 0x35);
        assert_eq!(structures.current_directory.entry_size, 0x51);
        assert_eq!(structures.drive_parameter_block.size, 0x20);
        assert_eq!(structures.drive_parameter_block.next, 0x18);
        assert_eq!(structures.disk_buffer.header_size, 0x10);
        assert_eq!(data_area.swappable_data_area.data_offset, 0x02CE);
        assert_eq!(data_area.swappable_data_area.indos_offset(), 0x02CF);
        assert_eq!(data_area.swappable_data_area.size, 0x0738);
        assert_eq!(data_area.swappable_data_area.always_swapped_size, 0x0018);
        let nul_offset = data_area.list_of_lists.data_offset as u32 + SYSVARS_OFF_NUL_HEADER;
        assert_eq!(nul_offset, 0x0048);
    }

    #[test]
    fn dos310_sft_entry_keeps_the_dos_3_fields() {
        let sft = &DosFormat::Dos310.structures().system_file_table;
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
        let sft = &DosFormat::Dos620.structures().system_file_table;
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
    fn pc98_dos620_data_area_returns_the_list_of_lists_one_paragraph_lower() {
        let data_area = DataArea::Pc98Dos620.layout();
        assert_eq!(
            data_area.list_of_lists.returned_pointer(0x0200),
            (0x01FF, 0x0010)
        );
    }

    #[test]
    fn sft_walk_follows_the_block_chain() {
        for format in formats() {
            let structures = format.structures();
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
        for format in formats() {
            let structures = format.structures();
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

    /// Named byte range of the DOS data segment.
    struct Region {
        /// Name of the table or stub.
        name: &'static str,
        /// Offset of the first byte.
        start: u32,
        /// Offset of the first byte behind the region.
        end: u32,
    }

    /// Returns the region `name` of `size` bytes at `offset`.
    fn region(name: &'static str, offset: u16, size: u32) -> Region {
        Region {
            name,
            start: u32::from(offset),
            end: u32::from(offset) + size,
        }
    }

    /// Returns every region that NeetanDos writes into the DOS data segment
    /// of `data_area` with the tables of `format` in front of the system data.
    fn data_area_regions(format: &StructureFormat, data_area: &DataAreaLayout) -> Vec<Region> {
        let list_of_lists = u32::from(data_area.list_of_lists.data_offset);
        let nul_header_end = SYSVARS_OFF_NUL_HEADER + DEVHDR_SIZE as u32;
        let list_end = format
            .list_of_lists_extension
            .map_or(nul_header_end, |extension| {
                nul_header_end.max(extension.extended_memory_kb + WORD_SIZE)
            });
        let sda = &data_area.swappable_data_area;
        let private_area = &data_area.private_area;
        let mut regions = vec![
            Region {
                name: "list of lists with the NUL header",
                start: list_of_lists.saturating_sub(SYSVARS_FIRST_MCB_DISTANCE),
                end: list_of_lists + list_end,
            },
            region(
                "first SFT block",
                data_area.first_sft_block_offset,
                SFT_HEADER_SIZE
                    + u32::from(SFT_INITIAL_COUNT) * format.system_file_table.entry_size,
            ),
            region("SDA", sda.data_offset, u32::from(sda.size)),
            region(
                "DBCS table",
                private_area.dbcs_table,
                crate::country::DBCS_LEAD_BYTES.len() as u32,
            ),
            region(
                "critical error stub",
                private_area.critical_error_stub,
                crate::CRITICAL_ERROR_FAIL_STUB.len() as u32,
            ),
            region(
                "error retriever stub",
                private_area.error_retriever_stub,
                u32::from(crate::trap::FAR_TRAP_STUB_LENGTH),
            ),
            region(
                "error message buffer",
                private_area.error_message_buffer,
                ERROR_MESSAGE_BUFFER_SIZE as u32,
            ),
        ];
        if let Some(offset) = private_area.kernel_stubs {
            regions.push(region(
                "kernel stubs",
                offset,
                u32::from(crate::kernel::KERNEL_STUBS_SIZE),
            ));
        }
        if let Some(offset) = private_area.transfer_buffer {
            regions.push(region(
                "transfer buffer",
                offset,
                crate::guest_cdrom::CDROM_SECTOR_SIZE as u32,
            ));
        }
        if let Some(window) = private_area.parameter_block_window {
            regions.push(region("DPB window", window.offset, u32::from(window.size)));
        }
        if let Some(self_boot) = &data_area.self_boot {
            let device_header_size = DEVHDR_SIZE as u32;
            let cdrom_stubs = &self_boot.cdrom_stubs;
            regions.extend([
                region("CON device", self_boot.con_device, device_header_size),
                region("CLOCK device", self_boot.clock_device, device_header_size),
                region(
                    "$AID#NEC device",
                    self_boot.aid_nec_device,
                    device_header_size,
                ),
                region("CD-ROM device", self_boot.cdrom_device, device_header_size),
                region("XMS device", self_boot.xms_device, device_header_size),
                region("EMS device", self_boot.ems_device, device_header_size),
                region(
                    "CDS array",
                    self_boot.current_directory_array,
                    CDS_ENTRIES * format.current_directory.entry_size,
                ),
                region("FCB SFT", self_boot.fcb_system_file_table, SFT_HEADER_SIZE),
                region(
                    "XMS entry stub",
                    self_boot.xms_entry_stub,
                    crate::XMS_ENTRY_PROLOGUE.len() as u32
                        + u32::from(crate::trap::FAR_TRAP_STUB_LENGTH),
                ),
                region(
                    "XMS device stub",
                    self_boot.xms_device_stub,
                    byte_count(&crate::XMS_DEVICE_STUB),
                ),
                region(
                    "EMS interrupt stub",
                    self_boot.ems_interrupt_stub,
                    byte_count(&crate::EMS_INTERRUPT_STUB),
                ),
                region(
                    "EMS page map return stub",
                    self_boot.ems_page_map_return_stub,
                    byte_count(&crate::EMS_PAGE_MAP_RETURN_STUB),
                ),
                region(
                    "NetBIOS stub",
                    self_boot.netbios_stub,
                    byte_count(&crate::NETBIOS_STUB),
                ),
                region(
                    "CD-ROM request pointer",
                    cdrom_stubs.request_pointer,
                    FAR_POINTER_SIZE,
                ),
                region(
                    "CD-ROM mirror header",
                    cdrom_stubs.mirror_header,
                    device_header_size,
                ),
                region(
                    "CD-ROM strategy stub",
                    cdrom_stubs.strategy_stub,
                    byte_count(&crate::CDROM_STRATEGY_STUB),
                ),
                region(
                    "CD-ROM interrupt stub",
                    cdrom_stubs.interrupt_stub,
                    u32::from(crate::trap::FAR_TRAP_STUB_LENGTH),
                ),
            ]);
            if let Some(dosmgr) = &self_boot.windows_dosmgr {
                regions.extend([
                    region(
                        "DOSMGR signature bank",
                        dosmgr.signature_bank.offset,
                        u32::from(dosmgr.signature_bank.size),
                    ),
                    region(
                        "DOSMGR patch table",
                        dosmgr.patch_table,
                        (crate::WINDOWS_DOSMGR_PATCH_TABLE_WORDS * size_of::<u16>()) as u32,
                    ),
                    region("DOSMGR UMB head", dosmgr.umb_head, WORD_SIZE),
                    region("DOSMGR saved DS", dosmgr.saved_data_segment, WORD_SIZE),
                    region("DOSMGR saved BX", dosmgr.saved_bx, WORD_SIZE),
                    region(
                        "DOSMGR critical section table",
                        dosmgr.critical_section_table,
                        size_of_val(&crate::WINDOWS_DOSMGR_CRITICAL_SECTION_TABLE) as u32,
                    ),
                ]);
            }
        }
        regions
    }

    #[test]
    fn data_area_regions_do_not_overlap() {
        for data_area in DataArea::ALL {
            let layout = data_area.layout();
            let regions = data_area_regions(data_area.format().structures(), layout);
            for (index, first) in regions.iter().enumerate() {
                assert!(first.start < first.end, "{data_area:?} {}", first.name);
                for second in &regions[index + 1..] {
                    assert!(
                        first.end <= second.start || second.end <= first.start,
                        "{data_area:?}: {} {:04X}h-{:04X}h overlaps {} {:04X}h-{:04X}h",
                        first.name,
                        first.start,
                        first.end,
                        second.name,
                        second.start,
                        second.end
                    );
                }
            }
        }
    }

    #[test]
    fn data_area_regions_lie_in_front_of_the_system_data() {
        for data_area in DataArea::ALL {
            let layout = data_area.layout();
            let system_data = u32::from(layout.private_area.system_data);
            for region in data_area_regions(data_area.format().structures(), layout) {
                assert!(
                    region.end <= system_data,
                    "{data_area:?}: {} ends at {:04X}h behind the system data",
                    region.name,
                    region.end
                );
            }
            if let Some(window) = layout.private_area.parameter_block_window {
                assert!(u32::from(window.offset) + u32::from(window.size) <= system_data);
            }
        }
    }

    #[test]
    fn cdrom_mirror_header_starts_a_paragraph() {
        for data_area in DataArea::ALL {
            let Some(self_boot) = data_area.layout().self_boot else {
                continue;
            };
            let cdrom_stubs = self_boot.cdrom_stubs;
            assert_eq!(
                cdrom_stubs.mirror_header % PARAGRAPH_BYTES,
                0,
                "{data_area:?}"
            );
            assert_eq!(
                cdrom_stubs.mirror_header_paragraphs() * PARAGRAPH_BYTES,
                cdrom_stubs.mirror_header
            );
            assert_eq!(
                cdrom_stubs.mirror_header + cdrom_stubs.mirror_strategy_offset(),
                cdrom_stubs.strategy_stub
            );
            assert_eq!(
                cdrom_stubs.mirror_header + cdrom_stubs.mirror_interrupt_offset(),
                cdrom_stubs.interrupt_stub
            );
        }
    }
}
