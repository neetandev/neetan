//! DOS internal data structures (SYSVARS, SFT, CDS, DPB layout).
//!
//! Field offsets in this module hold for the DOS 3.10 and the DOS 6.20
//! format. The `layout` module holds the values that differ between them and
//! the places of the tables in the DOS data segment. The IO.SYS work area
//! lives at segment 0060h (linear 0x0600).

use crate::MemoryAccess;

// List of lists field offsets (relative to the list of lists).
/// Distance of the first MCB segment word below the list of lists.
pub const SYSVARS_FIRST_MCB_DISTANCE: u32 = 0x02;
pub const SYSVARS_OFF_DPB_PTR: u32 = 0x00;
pub const SYSVARS_OFF_SFT_PTR: u32 = 0x04;
pub const SYSVARS_OFF_CLOCK_PTR: u32 = 0x08;
pub const SYSVARS_OFF_CON_PTR: u32 = 0x0C;
pub const SYSVARS_OFF_MAX_SECTOR: u32 = 0x10;
pub const SYSVARS_OFF_BUFFER_PTR: u32 = 0x12;
pub const SYSVARS_OFF_CDS_PTR: u32 = 0x16;
pub const SYSVARS_OFF_FCB_SFT_PTR: u32 = 0x1A;
pub const SYSVARS_OFF_PROT_FCBS: u32 = 0x1E;
pub const SYSVARS_OFF_BLOCK_DEVS: u32 = 0x20;
pub const SYSVARS_OFF_LASTDRIVE: u32 = 0x21;
pub const SYSVARS_OFF_NUL_HEADER: u32 = 0x22;

// Device header structure (18 bytes each)
pub const DEVHDR_SIZE: usize = 18;
pub const DEVHDR_OFF_NEXT_PTR: u32 = 0x00;
pub const DEVHDR_OFF_ATTRIBUTE: u32 = 0x04;
pub const DEVHDR_OFF_STRATEGY: u32 = 0x06;
pub const DEVHDR_OFF_INTERRUPT: u32 = 0x08;
pub const DEVHDR_OFF_NAME: u32 = 0x0A;

// Device attribute flags
pub const DEVATTR_CHAR: u16 = 0x8000;
pub const DEVATTR_IOCTL: u16 = 0x4000;
/// The device supports the generic IOCTL functions of DOS 3.2.
pub const DEVATTR_GENERIC_IOCTL: u16 = 0x0040;
pub const DEVATTR_SPECIAL: u16 = 0x0010;
pub const DEVATTR_CLOCK: u16 = 0x0008;
pub const DEVATTR_NUL: u16 = 0x0004;
pub const DEVATTR_STDOUT: u16 = 0x0002;
pub const DEVATTR_STDIN: u16 = 0x0001;

// SFT (System File Table) block header
pub const SFT_HEADER_SIZE: u32 = 6; // DWORD next + WORD count
/// Offset of the far pointer to the next SFT block.
pub const SFT_BLOCK_OFF_NEXT: u32 = 0x00;
/// Offset of the entry count word of an SFT block.
pub const SFT_BLOCK_OFF_COUNT: u32 = 0x04;

// SFT entry field offsets
pub const SFT_ENT_REF_COUNT: u32 = 0x00;
pub const SFT_ENT_OPEN_MODE: u32 = 0x02;
pub const SFT_ENT_FILE_ATTR: u32 = 0x04;
pub const SFT_ENT_DEV_INFO: u32 = 0x05;
pub const SFT_ENT_DEV_PTR: u32 = 0x07;
/// Open mode bit of a handle that a child process does not inherit.
pub const OPEN_MODE_NO_INHERIT: u16 = 0x0080;
pub const SFT_ENT_START_CLUSTER: u32 = 0x0B;
pub const SFT_ENT_FILE_TIME: u32 = 0x0D;
pub const SFT_ENT_FILE_DATE: u32 = 0x0F;
pub const SFT_ENT_FILE_SIZE: u32 = 0x11;
pub const SFT_ENT_FILE_POS: u32 = 0x15;
pub const SFT_ENT_REL_CLUSTER: u32 = 0x19;
/// Offset of the byte index of the directory entry in its sector.
pub const SFT_ENT_DIR_INDEX: u32 = 0x1F;
pub const SFT_ENT_NAME: u32 = 0x20;
pub const SFT_ENT_PSP_OWNER: u32 = 0x31;

// SFT block counts
/// Number of SFT entries of the first SFT block.
pub const SFT_INITIAL_COUNT: u16 = 5;
pub const SFT_EXTENDED_COUNT: u16 = 15;

// Device info word flags (SFT entry +0x05)
pub const SFT_DEVINFO_CHAR: u16 = 0x0080;
/// Drive index of a file handle in the device information word.
pub const SFT_DEVINFO_DRIVE_MASK: u16 = 0x003F;
pub const SFT_DEVINFO_EOF: u16 = 0x0040;
pub const SFT_DEVINFO_IOCTL: u16 = 0x4000;
// Bit-15 mirror of DEVATTR_CHAR. Real DOS always copies it into the SFT
// device-info word for character-device handles.
pub const SFT_DEVINFO_DRIVER_CHAR: u16 = 0x8000;
pub const SFT_DEVINFO_STDIN: u16 = 0x0001;
pub const SFT_DEVINFO_STDOUT: u16 = 0x0002;
pub const SFT_DEVINFO_NUL: u16 = 0x0004;
pub const SFT_DEVINFO_CLOCK: u16 = 0x0008;
pub const SFT_DEVINFO_SPECIAL: u16 = 0x0010;
/// Raw (binary) mode of a character device: reads and writes skip the
/// line editing, the echo and the control character checks.
pub const SFT_DEVINFO_RAW: u16 = 0x0020;
// Base device-info bits real DOS 6.20 sets on every character-device SFT
// entry: bit 15 (mirror of DEVATTR_CHAR), bit 7 (CHAR), and bit 6 (no-EOF).
pub const SFT_DEVINFO_CHAR_DEVICE_COMMON: u16 =
    SFT_DEVINFO_DRIVER_CHAR | SFT_DEVINFO_CHAR | SFT_DEVINFO_EOF;

/// Number of entries of the CDS array that the self boot builds.
pub const CDS_ENTRIES: u32 = 26;

// CDS entry field offsets
pub const CDS_OFF_PATH: u32 = 0x00; // 67 bytes, null-terminated
pub const CDS_OFF_FLAGS: u32 = 0x43;
pub const CDS_OFF_DPB_PTR: u32 = 0x45;
/// Offset of the word that holds the length of the root part of the path.
pub const CDS_OFF_ROOT_LENGTH: u32 = 0x4F;
pub const CDS_FLAG_PHYSICAL: u16 = 0x4000;
pub const CDS_FLAG_NETWORK: u16 = 0x8000;

// DPB entry field offsets
pub const DPB_OFF_DRIVE_NUM: u32 = 0x00;
pub const DPB_OFF_UNIT_NUM: u32 = 0x01;
pub const DPB_OFF_BYTES_PER_SECTOR: u32 = 0x02;
pub const DPB_OFF_CLUSTER_MASK: u32 = 0x04;
pub const DPB_OFF_CLUSTER_SHIFT: u32 = 0x05;
pub const DPB_OFF_RESERVED_SECTORS: u32 = 0x06;
pub const DPB_OFF_NUM_FATS: u32 = 0x08;
pub const DPB_OFF_ROOT_ENTRIES: u32 = 0x09;
pub const DPB_OFF_FIRST_DATA_SECTOR: u32 = 0x0B;
pub const DPB_OFF_MAX_CLUSTER: u32 = 0x0D;

/// Offset of the far pointer to the next buffer header.
pub const BUFFER_OFF_NEXT: u32 = 0x00;
/// Offset of the drive byte of a buffer header (`FFh` = unused).
pub const BUFFER_OFF_DRIVE: u32 = 0x04;

// DOS swappable data area returned by INT 21h AX=5D06h. The critical-error
// flag is at SDA+0, the InDOS flag at SDA+1, both of which Windows DOSMGR
// expects to find via the patch table. The data area layout places the SDA
// and gives its sizes. These field offsets hold for DOS 3.10 and later.
/// Critical error flag byte.
pub const SDA_OFF_CRITICAL_ERROR: u16 = 0x00;
/// InDOS flag byte.
pub const SDA_OFF_INDOS: u16 = 0x01;
/// Drive of the last critical error (`FFh` outside of an INT 24h handler).
pub const SDA_OFF_ERROR_DRIVE: u16 = 0x02;
/// Far pointer to the current DTA.
pub const SDA_OFF_DTA: u16 = 0x0C;
/// Segment of the current PSP.
pub const SDA_OFF_CURRENT_PSP: u16 = 0x10;
/// Stack pointer saved across an INT 23h call.
pub const SDA_OFF_INT23_STACK_POINTER: u16 = 0x12;
/// Return code of the last terminated process.
pub const SDA_OFF_RETURN_CODE: u16 = 0x14;
/// Current drive byte.
pub const SDA_OFF_CURRENT_DRIVE: u16 = 0x16;
/// Extended break flag byte.
pub const SDA_OFF_BREAK_FLAG: u16 = 0x17;

// Size of the counted message buffer of the AX=122Eh retriever.
pub const ERROR_MESSAGE_BUFFER_SIZE: usize = 0x78;

// HLE tokens returned by AX=122Eh DL=00h/02h/04h/06h. Real DOS 5+ returns far
// pointers to internal tables; we return synthetic identifiers in a sentinel
// segment that the retriever uses to pick the right message table.
pub const ERROR_TABLE_SEGMENT: u16 = 0x0001;
pub const ERROR_TABLE_STANDARD_TOKEN: u16 = 0x0100;
pub const ERROR_TABLE_PARAMETER_TOKEN: u16 = 0x0200;
pub const ERROR_TABLE_CRITICAL_TOKEN: u16 = 0x0300;

// MCB structure field offsets (within the 16-byte MCB header)
pub const MCB_OFF_TYPE: u32 = 0x00;
pub const MCB_OFF_OWNER: u32 = 0x01;
pub const MCB_OFF_SIZE: u32 = 0x03;
pub const MCB_OFF_NAME: u32 = 0x08;
/// Size of an MCB header in bytes.
pub const MCB_HEADER_SIZE: u32 = 16;

// MCB owner special values
pub const MCB_OWNER_FREE: u16 = 0x0000;
pub const MCB_OWNER_DOS: u16 = 0x0008;

// MCB chain layout after boot, starting at the first MCB of the placement:
//   MCB[0]: env block (owner=DOS)
//   MCB[1]: COMMAND.COM PSP+code (owner=PSP)
//   MCB[2]: free memory (owner=0)
pub const ENV_BLOCK_PARAGRAPHS: u16 = 15;
pub const COMMAND_BLOCK_PARAGRAPHS: u16 = 36;

// PSP field offsets (within the 256-byte PSP)
pub const PSP_OFF_INT20: u32 = 0x00;
pub const PSP_OFF_MEM_TOP: u32 = 0x02;
pub const PSP_OFF_FAR_CALL: u32 = 0x05;
pub const PSP_OFF_INT22_VEC: u32 = 0x0A;
pub const PSP_OFF_INT23_VEC: u32 = 0x0E;
pub const PSP_OFF_INT24_VEC: u32 = 0x12;
pub const PSP_OFF_PARENT_PSP: u32 = 0x16;
pub const PSP_OFF_JFT: u32 = 0x18;
pub const PSP_OFF_ENV_SEG: u32 = 0x2C;
pub const PSP_OFF_HANDLE_SIZE: u32 = 0x32;
pub const PSP_OFF_HANDLE_PTR: u32 = 0x34;
pub const PSP_OFF_INT21_STUB: u32 = 0x50;
pub const PSP_OFF_FCB1: u32 = 0x5C;
pub const PSP_OFF_FCB2: u32 = 0x6C;
pub const PSP_OFF_CMD_TAIL_LEN: u32 = 0x80;
pub const PSP_OFF_CMD_TAIL: u32 = 0x81;

pub const IOSYS_SEGMENT: u16 = 0x0060;
pub const IOSYS_BASE: u32 = (IOSYS_SEGMENT as u32) << 4;
/// Size of the NEC `IO.SYS` work area at `0060:0000` in bytes.
pub const IOSYS_WORK_AREA_SIZE: u32 = 0x2D00;

pub const IOSYS_OFF_PRODUCT_NUMBER: u32 = 0x0020;
pub const IOSYS_OFF_INTERNAL_REVISION: u32 = 0x0022;

/// Identification values of a NEC `IO.SYS` release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NecIosysIdentity {
    /// Product number that `INT DCh CL=12h` returns in `AX`.
    pub intdch_product_number: u16,
    /// Product number at 0060:0020h.
    pub work_area_product_number: u16,
    /// Internal revision at 0060:0022h.
    pub internal_revision: u8,
}

/// Identification of NEC MS-DOS 3.1 (PS98-011). Its work area has no
/// product number and no revision.
pub const NEC_DOS310_IOSYS_IDENTITY: NecIosysIdentity = NecIosysIdentity {
    intdch_product_number: 0x0001,
    work_area_product_number: 0x0000,
    internal_revision: 0x00,
};

/// Identification of NEC MS-DOS 6.20.
pub const NEC_DOS620_IOSYS_IDENTITY: NecIosysIdentity = NecIosysIdentity {
    intdch_product_number: 0x0200,
    work_area_product_number: 0x0200,
    internal_revision: 0x01,
};

pub const IOSYS_OFF_EMM_BANK_FLAG: u32 = 0x0030;
pub const IOSYS_OFF_EXT_MEM_128K: u32 = 0x0031;
pub const IOSYS_OFF_FD_DUPLICATE: u32 = 0x0038;
pub const IOSYS_OFF_AUX_PROTOCOL: u32 = 0x0068;
pub const IOSYS_OFF_DAUA_TABLE: u32 = 0x006C;
pub const IOSYS_OFF_KANJI_MODE: u32 = 0x008A;
pub const IOSYS_OFF_GRAPH_CHAR: u32 = 0x008B;
pub const IOSYS_OFF_SHIFT_FN_CHAR: u32 = 0x008C;
pub const IOSYS_OFF_STOP_REENTRY: u32 = 0x00A4;
pub const IOSYS_OFF_INTDC_FLAG: u32 = 0x00B4;
pub const IOSYS_OFF_SPECIAL_INPUT: u32 = 0x0106;
pub const IOSYS_OFF_PRINTER_ECHO: u32 = 0x0107;
pub const IOSYS_OFF_SOFTKEY_FLAGS: u32 = 0x010C;
pub const IOSYS_OFF_CURSOR_Y: u32 = 0x0110;
pub const IOSYS_OFF_FNKEY_DISPLAY: u32 = 0x0111;
pub const IOSYS_OFF_SCROLL_LOWER: u32 = 0x0112;
pub const IOSYS_OFF_SCREEN_LINES: u32 = 0x0113;
pub const IOSYS_OFF_CLEAR_ATTR: u32 = 0x0114;
pub const IOSYS_OFF_KANJI_HI_FLAG: u32 = 0x0115;
pub const IOSYS_OFF_KANJI_HI_BYTE: u32 = 0x0116;
pub const IOSYS_OFF_LINE_WRAP: u32 = 0x0117;
pub const IOSYS_OFF_SCROLL_SPEED: u32 = 0x0118;
pub const IOSYS_OFF_CLEAR_CHAR: u32 = 0x0119;
pub const IOSYS_OFF_CURSOR_VISIBLE: u32 = 0x011B;
pub const IOSYS_OFF_CURSOR_X: u32 = 0x011C;
pub const IOSYS_OFF_DISPLAY_ATTR: u32 = 0x011D;
pub const IOSYS_OFF_SCROLL_UPPER: u32 = 0x011E;
pub const IOSYS_OFF_SCROLL_WAIT: u32 = 0x011F;
pub const IOSYS_OFF_SAVED_CURSOR_Y: u32 = 0x0126;
pub const IOSYS_OFF_SAVED_CURSOR_X: u32 = 0x0127;
pub const IOSYS_OFF_SAVED_CURSOR_ATTR: u32 = 0x012B;
pub const IOSYS_OFF_LAST_DRIVE_UNIT: u32 = 0x0136;
pub const IOSYS_OFF_FD_DUPLICATE2: u32 = 0x013B;
pub const IOSYS_OFF_EXT_ATTR_DISPLAY: u32 = 0x013C;
pub const IOSYS_OFF_EXT_ATTR_CLEAR: u32 = 0x013E;
pub const IOSYS_OFF_EXT_ATTR_MODE: u32 = 0x05D6;
pub const IOSYS_OFF_TEXT_MODE: u32 = 0x05D8;
/// Far pointer to the attribute and DA/UA table of the drives A: to Z:.
pub const IOSYS_OFF_DAUA_PTR: u32 = 0x2820;
/// Attribute and DA/UA table of the drives A: to Z:.
pub const IOSYS_OFF_EXT_DAUA_TABLE: u32 = 0x2C86;
pub const IOSYS_EXT_DAUA_TABLE_SIZE: u32 = 52;

// BDA (BIOS Data Area) fields read during drive discovery.
pub const BDA_BOOT_DEVICE: u32 = 0x0584;
pub const BDA_DISK_EQUIP: u32 = 0x055C;

/// Writes a far pointer (offset:segment, little-endian DWORD) at the given linear address.
pub fn write_far_ptr(mem: &mut dyn MemoryAccess, addr: u32, segment: u16, offset: u16) {
    mem.write_word(addr, offset);
    mem.write_word(addr + 2, segment);
}

/// Writes an 18-byte device header at the given linear address.
pub fn write_device_header(
    mem: &mut dyn MemoryAccess,
    addr: u32,
    next_segment: u16,
    next_offset: u16,
    attribute: u16,
    name: &[u8; 8],
) {
    write_far_ptr(mem, addr + DEVHDR_OFF_NEXT_PTR, next_segment, next_offset);
    mem.write_word(addr + DEVHDR_OFF_ATTRIBUTE, attribute);
    mem.write_word(addr + DEVHDR_OFF_STRATEGY, 0x0000);
    mem.write_word(addr + DEVHDR_OFF_INTERRUPT, 0x0000);
    mem.write_block(addr + DEVHDR_OFF_NAME, name);
}
