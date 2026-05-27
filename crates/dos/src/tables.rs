//! DOS internal data structures (SYSVARS, SFT, CDS, DPB layout).
//!
//! Constants define the memory layout for NEETAN DOS boot data structures.
//! The IO.SYS work area lives at segment 0060h (linear 0x0600).
//! SYSVARS and DOS structures live at segment 0x0200 (linear 0x2000)
//! to avoid field offset conflicts with the IO.SYS work area.

use crate::MemoryAccess;

pub const DOS_DATA_SEGMENT: u16 = 0x0200;
pub const DOS_DATA_BASE: u32 = (DOS_DATA_SEGMENT as u32) << 4;

// SYSVARS / List of Lists
pub const SYSVARS_SEGMENT: u16 = DOS_DATA_SEGMENT;
pub const SYSVARS_OFFSET: u16 = 0x0000;
pub const SYSVARS_BASE: u32 = DOS_DATA_BASE;
// Pointer returned by INT 21h AH=52h. It resolves to the same linear address
// as SYSVARS_BASE but carries a non-zero offset so callers can read the
// negative-offset fields (e.g. the first-MCB pointer at SYSVARS-2).
pub const SYSVARS_LIST_SEGMENT: u16 = DOS_DATA_SEGMENT - 1;
pub const SYSVARS_LIST_OFFSET: u16 = 0x0010;

// SYSVARS field offsets (relative to SYSVARS_BASE)
pub const SYSVARS_OFF_FIRST_MCB: u32 = 0x02; // at SYSVARS - 2 (written to SYSVARS_BASE - 2)
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
pub const SYSVARS_OFF_JOIN_DRIVES: u32 = 0x34;
pub const SYSVARS_OFF_SETVER_PTR: u32 = 0x37;
pub const SYSVARS_OFF_BUFFERS: u32 = 0x3F;
pub const SYSVARS_OFF_LOOKAHEAD: u32 = 0x41;
pub const SYSVARS_OFF_BOOT_DRIVE: u32 = 0x43;
pub const SYSVARS_OFF_386_FLAG: u32 = 0x44;
pub const SYSVARS_OFF_EXT_MEM: u32 = 0x45;

// Device headers - offsets from DOS_DATA_BASE
pub const DEV_NUL_OFFSET: u16 = 0x0022; // Embedded in SYSVARS
pub const DEV_CON_OFFSET: u16 = 0x0048;
pub const DEV_CLOCK_OFFSET: u16 = 0x005A;
pub const DEV_AID_NEC_OFFSET: u16 = 0x006C;
pub const DEV_CDROM_OFFSET: u16 = 0x007E;
// XMSXXXX0 device header (conditionally linked when XMS is enabled)
pub const DEV_XMS_OFFSET: u16 = 0x0D4E;
// EMMXXXX0 device header (conditionally linked when EMS is enabled)
pub const DEV_EMS_OFFSET: u16 = 0x0D60;

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
pub const DEVATTR_SPECIAL: u16 = 0x0010;
pub const DEVATTR_CLOCK: u16 = 0x0008;
pub const DEVATTR_NUL: u16 = 0x0004;
pub const DEVATTR_STDOUT: u16 = 0x0002;
pub const DEVATTR_STDIN: u16 = 0x0001;

// SFT (System File Table)
pub const SFT_OFFSET: u16 = 0x0090;
pub const SFT_BASE: u32 = DOS_DATA_BASE + SFT_OFFSET as u32;
pub const SFT_HEADER_SIZE: u32 = 6; // DWORD next + WORD count
pub const SFT_ENTRY_SIZE: u32 = 59;

// SFT entry field offsets (within each 59-byte entry)
pub const SFT_ENT_REF_COUNT: u32 = 0x00;
pub const SFT_ENT_OPEN_MODE: u32 = 0x02;
pub const SFT_ENT_FILE_ATTR: u32 = 0x04;
pub const SFT_ENT_DEV_INFO: u32 = 0x05;
pub const SFT_ENT_DEV_PTR: u32 = 0x07;
pub const SFT_ENT_START_CLUSTER: u32 = 0x0B;
pub const SFT_ENT_FILE_TIME: u32 = 0x0D;
pub const SFT_ENT_FILE_DATE: u32 = 0x0F;
pub const SFT_ENT_FILE_SIZE: u32 = 0x11;
pub const SFT_ENT_FILE_POS: u32 = 0x15;
pub const SFT_ENT_REL_CLUSTER: u32 = 0x19;
pub const SFT_ENT_CUR_CLUSTER: u32 = 0x1B;
pub const SFT_ENT_DIR_SECTOR: u32 = 0x1D;
pub const SFT_ENT_DIR_INDEX: u32 = 0x1F;
pub const SFT_ENT_NAME: u32 = 0x20;
pub const SFT_ENT_PSP_OWNER: u32 = 0x31;

// SFT block counts
pub const SFT_INITIAL_COUNT: u16 = 5;
pub const SFT_EXTENDED_COUNT: u16 = 15;
pub const SFT_TOTAL_COUNT: u16 = SFT_INITIAL_COUNT + SFT_EXTENDED_COUNT;

// Device info word flags (SFT entry +0x05)
pub const SFT_DEVINFO_CHAR: u16 = 0x0080;
pub const SFT_DEVINFO_EOF: u16 = 0x0040;
pub const SFT_DEVINFO_STDIN: u16 = 0x0001;
pub const SFT_DEVINFO_STDOUT: u16 = 0x0002;
pub const SFT_DEVINFO_NUL: u16 = 0x0004;
pub const SFT_DEVINFO_CLOCK: u16 = 0x0008;
pub const SFT_DEVINFO_SPECIAL: u16 = 0x0010;

// CDS (Current Directory Structure)
pub const CDS_OFFSET: u16 = 0x01C0;
pub const CDS_BASE: u32 = DOS_DATA_BASE + CDS_OFFSET as u32;
pub const CDS_ENTRY_SIZE: u32 = 0x58; // 88 bytes per entry
pub const CDS_ENTRIES: u32 = 26;

// CDS entry field offsets
pub const CDS_OFF_PATH: u32 = 0x00; // 67 bytes, null-terminated
pub const CDS_OFF_FLAGS: u32 = 0x43;
pub const CDS_OFF_DPB_PTR: u32 = 0x45;
pub const CDS_OFF_BACKSLASH_OFFSET: u32 = 0x49;
pub const CDS_FLAG_PHYSICAL: u16 = 0x4000;
pub const CDS_FLAG_NETWORK: u16 = 0x8000;

// DPB (Disk Parameter Block)
pub const DPB_OFFSET: u16 = 0x0AB0;
pub const DPB_BASE: u32 = DOS_DATA_BASE + DPB_OFFSET as u32;
pub const DPB_ENTRY_SIZE: u32 = 0x21; // 33 bytes per entry (DOS 4.0+)

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
pub const DPB_OFF_SECTORS_PER_FAT: u32 = 0x0F;
pub const DPB_OFF_FIRST_ROOT_SECTOR: u32 = 0x11;
pub const DPB_OFF_DEVICE_PTR: u32 = 0x13;
pub const DPB_OFF_MEDIA_DESC: u32 = 0x17;
pub const DPB_OFF_ACCESS_FLAG: u32 = 0x18;
pub const DPB_OFF_NEXT_DPB: u32 = 0x19;

// Disk buffer
pub const DISK_BUFFER_OFFSET: u16 = 0x0B20;
pub const DISK_BUFFER_BASE: u32 = DOS_DATA_BASE + DISK_BUFFER_OFFSET as u32;

// InDOS and critical error flags
pub const INDOS_FLAG_OFFSET: u16 = 0x0D30;
pub const INDOS_FLAG_ADDR: u32 = DOS_DATA_BASE + INDOS_FLAG_OFFSET as u32;
pub const CRITICAL_ERROR_FLAG_ADDR: u32 = INDOS_FLAG_ADDR + 1;

// DBCS lead byte table (6 bytes: 81,9F, E0,FC, 00,00)
pub const DBCS_TABLE_OFFSET: u16 = 0x0D3E;
pub const DBCS_TABLE_ADDR: u32 = DOS_DATA_BASE + DBCS_TABLE_OFFSET as u32;

// FCB-SFT
pub const FCB_SFT_OFFSET: u16 = 0x0D38;
pub const FCB_SFT_BASE: u32 = DOS_DATA_BASE + FCB_SFT_OFFSET as u32;

// XMS driver entry stub (3 bytes: INT FEh / RETF)
pub const XMS_ENTRY_STUB_OFFSET: u16 = 0x0D44;
pub const XMS_ENTRY_STUB_ADDR: u32 = DOS_DATA_BASE + XMS_ENTRY_STUB_OFFSET as u32;
pub const XMS_ENTRY_STUB_SEGMENT: u16 = DOS_DATA_SEGMENT;

// XMSXXXX0 strategy/interrupt stub (6 bytes: set DONE bit + RETF)
pub const XMS_DEV_STUB_OFFSET: u16 = 0x0D47;
pub const XMS_DEV_STUB_ADDR: u32 = DOS_DATA_BASE + XMS_DEV_STUB_OFFSET as u32;

// EMS INT 67h trap stub + device name (18 bytes: 10 code + 8 "EMMXXXX0").
// Applications using the EMS spec's "get interrupt vector" installation
// check read the INT 67h vector and compare [ES:BX+000Ah] with "EMMXXXX0".
// IVT[67h] points here; the first 10 bytes are executable code that fires
// the BIOS HLE trap via OUT 07F0h, AL.
pub const EMS_INT67_STUB_OFFSET: u16 = 0x0D72;
pub const EMS_INT67_STUB_ADDR: u32 = DOS_DATA_BASE + EMS_INT67_STUB_OFFSET as u32;

pub const EMS_PGMAPRET_VECTOR: u8 = 0xE7;
pub const EMS_PGMAPRET_STUB_OFFSET: u16 = 0x0D84;
pub const EMS_PGMAPRET_STUB_ADDR: u32 = DOS_DATA_BASE + EMS_PGMAPRET_STUB_OFFSET as u32;
pub const EMS_PGMAPRET_STUB_SEGMENT: u16 = DOS_DATA_SEGMENT;

// INT 24h default critical-error handler (3 bytes: MOV AL,3 / IRET).
pub const INT24_STUB_OFFSET: u16 = 0x0D8E;
pub const INT24_STUB_ADDR: u32 = DOS_DATA_BASE + INT24_STUB_OFFSET as u32;

// UMB region with EMS: MCB chain at segment D000h (64 KB at D0000-DFFFF).
// UMB region without EMS starts at C000h and reuses the would-be EMS page frame.
pub const UMB_NOEMS_FIRST_MCB_SEGMENT: u16 = 0xC000;
pub const UMB_NOEMS_BLOCK_PARAGRAPHS: u16 = 0x1700;
pub const UMB_FIRST_MCB_SEGMENT: u16 = 0xD000;
pub const UMB_BLOCK_PARAGRAPHS: u16 = 0x0700;
pub const UMB_RESERVED_MCB_SEGMENT: u16 = UMB_FIRST_MCB_SEGMENT + UMB_BLOCK_PARAGRAPHS + 1;
pub const UMB_RESERVED_PARAGRAPHS: u16 = 0x00FD;
pub const UMB_SECOND_MCB_SEGMENT: u16 = UMB_RESERVED_MCB_SEGMENT + UMB_RESERVED_PARAGRAPHS + 1;
pub const UMB_TRAILING_RESERVED_MCB_SEGMENT: u16 =
    UMB_SECOND_MCB_SEGMENT + UMB_BLOCK_PARAGRAPHS + 1;
pub const UMB_TRAILING_RESERVED_PARAGRAPHS: u16 = 0x00FF;

// EMS page frame at C0000h (4 x 16 KB physical pages)
pub const EMS_PAGE_FRAME_SEGMENT: u16 = 0xC000;
pub const EMS_PAGE_SIZE: u32 = 0x4000;

// First MCB (sentinel)
pub const FIRST_MCB_OFFSET: u16 = 0x0DA0;
pub const FIRST_MCB_ADDR: u32 = DOS_DATA_BASE + FIRST_MCB_OFFSET as u32;
pub const FIRST_MCB_SEGMENT: u16 = (FIRST_MCB_ADDR >> 4) as u16;

// MCB structure field offsets (within the 16-byte MCB header)
pub const MCB_OFF_TYPE: u32 = 0x00;
pub const MCB_OFF_OWNER: u32 = 0x01;
pub const MCB_OFF_SIZE: u32 = 0x03;
pub const MCB_OFF_NAME: u32 = 0x08;

// MCB owner special values
pub const MCB_OWNER_FREE: u16 = 0x0000;
pub const MCB_OWNER_DOS: u16 = 0x0008;

// MCB chain layout after boot:
//   MCB[0] at FIRST_MCB_SEGMENT: env block (owner=DOS)
//   MCB[1] at COMMAND_MCB_SEGMENT: COMMAND.COM PSP+code (owner=PSP_SEGMENT)
//   MCB[2] at FREE_MCB_SEGMENT: free memory (owner=0)
pub const ENV_BLOCK_PARAGRAPHS: u16 = 15;
pub const ENV_SEGMENT: u16 = FIRST_MCB_SEGMENT + 1;
pub const COMMAND_MCB_SEGMENT: u16 = ENV_SEGMENT + ENV_BLOCK_PARAGRAPHS;
pub const COMMAND_BLOCK_PARAGRAPHS: u16 = 36;
pub const PSP_SEGMENT: u16 = COMMAND_MCB_SEGMENT + 1;
pub const FREE_MCB_SEGMENT: u16 = PSP_SEGMENT + COMMAND_BLOCK_PARAGRAPHS;

// Top of conventional memory (640 KB boundary)
pub const MEMORY_TOP_SEGMENT: u16 = 0xA000;

// PC-98 keyboard buffer in BDA (circular buffer, 16 entries of 2 bytes each)
pub const KB_BUF_START: u32 = 0x0502;
pub const KB_BUF_END: u32 = 0x0522;
pub const KB_BUF_HEAD: u32 = 0x0524;
pub const KB_BUF_TAIL: u32 = 0x0526;
pub const KB_BUF_COUNT: u32 = 0x0528;

pub(crate) fn key_available(memory: &dyn MemoryAccess) -> bool {
    memory.read_byte(KB_BUF_COUNT) > 0
}

pub(crate) fn read_key(memory: &mut dyn MemoryAccess) -> (u8, u8) {
    let head = memory.read_word(KB_BUF_HEAD) as u32;
    let ch = memory.read_byte(head);
    let scan = memory.read_byte(head + 1);

    let mut new_head = head + 2;
    if new_head >= KB_BUF_END {
        new_head = KB_BUF_START;
    }
    memory.write_word(KB_BUF_HEAD, new_head as u16);

    let count = memory.read_byte(KB_BUF_COUNT);
    if count > 0 {
        memory.write_byte(KB_BUF_COUNT, count - 1);
    }

    (scan, ch)
}

pub(crate) fn flush_keyboard_buffer(memory: &mut dyn MemoryAccess) {
    memory.write_byte(KB_BUF_COUNT, 0);
    let head = memory.read_word(KB_BUF_HEAD);
    memory.write_word(KB_BUF_TAIL, head);
}

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

pub const IOSYS_OFF_PRODUCT_NUMBER: u32 = 0x0020;
pub const IOSYS_OFF_INTERNAL_REVISION: u32 = 0x0022;
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
pub const IOSYS_OFF_DAUA_PTR: u32 = 0x2820;
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

/// Converts a DOS_DATA_BASE-relative offset to a far pointer in DOS_DATA_SEGMENT.
pub fn dos_data_far(offset: u16) -> (u16, u16) {
    (DOS_DATA_SEGMENT, offset)
}
