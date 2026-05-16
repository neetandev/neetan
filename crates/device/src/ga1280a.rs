//! I-O DATA GA-1280A C-Bus graphics accelerator.
//!
//! Supports 16, 256 and 65536 color modes and resolutions up to 1600x1024.
//! It was the high quality version of the GA-1024A, having double the VRAM (2 MiB) and hardware
//! RAMDAC mouse cursor support. All software that supported the GA-1024A support the GA-1280A too.
//!
//! This implementation was created by analyzing the DOS and Windows 3.1 drivers, and additionally
//! the official SDK / sample programs (version 3.60). It's an "ideal" implementation, that does
//! not capture real hardware quirks or realistic timings. As such it's acceleration operations are
//! most likely too perfect, compared to the real hardware.
//!
//! It's mainly useful for games that do not support the PEGC graphics mode of the PC-9821 (between
//! 1993 and 1994) or for high resolution support for Windows 3.1 and Windows 95.

mod accelerator;
mod framebuffer;

const DEFAULT_GAPORT: u16 = 0x00D8;
const ID_STREAM: &[u8; 16] = b".O DATA DEVICE I";
const GA1280_MAX_VISIBLE_WIDTH: u32 = 1600;
const GA1280_MAX_VISIBLE_HEIGHT: u32 = 1024;
const GA1280_MAX_PIXEL_MAP_WIDTH: u32 = 2048;
const GA1280_MAX_PIXEL_MAP_HEIGHT: u32 = 2048;
const DEFAULT_WIDTH: u32 = 640;
const DEFAULT_HEIGHT: u32 = 480;
const FULL_COLOR_WIDTH: u32 = 512;
const FULL_COLOR_HEIGHT: u32 = 480;
const CURSOR_MASK_BYTES: usize = 128;
const CURSOR_PATTERN_BYTES: usize = CURSOR_MASK_BYTES * 2;
const CURSOR_PATTERN_BYTES_U16: u16 = CURSOR_PATTERN_BYTES as u16;
const TILE_PATTERN_WORDS: usize = 8;
const ROP_PATTERN_ROWS: usize = 8;

const SELECTOR_INDEX: u8 = 0x00;
const SELECTOR_SRW: u8 = 0x01;
const SELECTOR_SRR: u8 = 0x02;
const SELECTOR_WPM: u8 = 0x03;
const SELECTOR_WBM: u8 = 0x05;
const SELECTOR_PRS: u8 = 0x06;
const SELECTOR_RPE: u8 = 0x07;
const SELECTOR_COL: u8 = 0x09;
const SELECTOR_TILE: u8 = 0x0B;
const SELECTOR_ROT: u8 = 0x0D;
const SELECTOR_MOD: u8 = 0x0E;
const SELECTOR_UNKNOWN_0F: u8 = 0x0F;
const SELECTOR_FCOL: u8 = 0x10;
const SELECTOR_BCOL_PMW: u8 = 0x12;
const SELECTOR_PMH: u8 = 0x13;
const SELECTOR_MIX: u8 = 0x14;
const SELECTOR_CWB_UNKNOWN: u8 = 0x15;
const SELECTOR_WBA1: u8 = 0x16;
const SELECTOR_WBA2: u8 = 0x17;
const SELECTOR_VDAC_ARW_RS: u8 = 0x18;
const SELECTOR_VDAC_ARR: u8 = 0x19;
const SELECTOR_VDAC_CPR: u8 = 0x1A;
const SELECTOR_VDAC_MSK: u8 = 0x1B;
const SELECTOR_SYSTEM_PDT: u8 = 0x1C;
const SELECTOR_STATUS_SSV: u8 = 0x1D;
const SELECTOR_CRTC_POP1: u8 = 0x1E;
const SELECTOR_CRTC_POP2: u8 = 0x1F;

const OFFSET_BASE: u8 = 0;
const OFFSET_BASE_PLUS_ONE: u8 = 1;
const OFFSET_PLUS_TWO: u8 = 2;
const OFFSET_PLUS_THREE: u8 = 3;
const FIXED_WINDOW_PORT: u16 = 0x1600;
const COMPATIBILITY_MAPPED_REGISTER_BASE_OFFSET: u32 = 0x1F00;
const COMPATIBILITY_MAPPED_REGISTER_PLUS_TWO_OFFSET: u32 = 0x1F40;
const MAPPED_REGISTER_APERTURE_BYTES: u32 = 0x40;
const WBA_LOW_BYTE_SEGMENT_MASK: u16 = 0x00FE;

const CONVENTIONAL_WINDOW_BASE: u32 = 0x000C_0000;
const CONVENTIONAL_WINDOW_BYTES: u32 = 0x3_0000;
const FLAT_APERTURE_BASE: u32 = 0x00F0_0000;
const FLAT_APERTURE_BYTES: u32 = 0x1_0000;

const GA1280_VRAM_BYTES: usize = 2 * 1024 * 1024;

const DEFAULT_REFRESH_HZ: u32 = 60;
const DEFAULT_ACTIVE_LINES: u32 = 480;
const DEFAULT_TOTAL_LINES: u32 = 524;

const CRTC_INDEX_HORIZONTAL_TOTAL: usize = 0x00;
const CRTC_INDEX_VERTICAL_TOTAL: usize = 0x10;
const CRTC_INDEX_VERTICAL_DISPLAY_END: usize = 0x12;
const CRTC_INDEX_VSYNC_STATUS: usize = 0x1F;
const CRTC_INDEX_GA1280_VSYNC_STATUS: usize = 0x3F;
const CRTC_INDEX_DISPLAY_START_LOW: usize = 0x30;
const CRTC_INDEX_DISPLAY_START_MID: usize = 0x31;
const CRTC_INDEX_DISPLAY_START_HIGH: usize = 0x32;
const CRTC_BIT_VSYNC_ACTIVE: u8 = 0x02;
const CRTC_BIT_GA1280_VSYNC_ACTIVE: u16 = 0x0400;

/// Refresh rate per GAINIT screen mode, keyed by `(CRTC[0x00], CRTC[0x10])`.
const MODE_REFRESH_TABLE: &[(u16, u16, u32)] = &[
    (0x009D, 0x032E, 86), // mode 01: 1024x768 86Hz interlaced
    (0x0063, 0x020B, 60), // mode 02: 640x480 60Hz
    (0x0069, 0x01B6, 56), // mode 05: 640x400 56Hz
    (0x007F, 0x026F, 56), // mode 06: 800x600 56Hz
    (0x00A7, 0x0324, 60), // mode 07: 1024x768 60Hz
    (0x00A5, 0x0324, 70), // mode 08: 1024x768 70Hz
    (0x00A6, 0x0332, 80), // mode 09: 1024x768 80Hz NEC interlaced
    (0x0082, 0x0298, 72), // mode 10: 800x600 72Hz
    (0x0067, 0x0206, 72), // mode 11: 640x480 72Hz
    (0x00CA, 0x03F6, 60), // mode 12: 1280x960 60Hz
    (0x00CC, 0x03DC, 66), // mode 13: 1280x960 66Hz
    (0x0067, 0x01EE, 51), // mode 14: 640x480 51Hz NDHD-2
    (0x0087, 0x01EE, 52), // mode 15: 800x480 52Hz NDHD-2
    (0x0079, 0x01EE, 52), // mode 16: 640x480 52Hz NDHD-2
    (0x00D1, 0x042B, 60), // mode 17: 1280x1024 60Hz
    (0x008C, 0x0236, 56), // mode 18: 768x512 56Hz
    (0x0106, 0x046C, 90), // mode 19: 1600x1024 90Hz interlaced
    (0x00A6, 0x020B, 60), // modes 20 / 21: 512x480 full color
    (0x0100, 0x041D, 60), // mode 22: 1600x1024 60Hz
];

/// GAINIT timing mode number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Ga1280aScreenMode {
    /// `/m=1`.
    Mode01 = 1,
    /// `/m=2`.
    Mode02 = 2,
    /// `/m=3`, reserved by the driver.
    Mode03Reserved = 3,
    /// `/m=4`, reserved by the driver.
    Mode04Reserved = 4,
    /// `/m=5`.
    Mode05 = 5,
    /// `/m=6`.
    Mode06 = 6,
    /// `/m=7`.
    Mode07 = 7,
    /// `/m=8`.
    Mode08 = 8,
    /// `/m=9`.
    Mode09 = 9,
    /// `/m=10`.
    Mode10 = 10,
    /// `/m=11`.
    Mode11 = 11,
    /// `/m=12`.
    Mode12 = 12,
    /// `/m=13`.
    Mode13 = 13,
    /// `/m=14`.
    Mode14 = 14,
    /// `/m=15`.
    Mode15 = 15,
    /// `/m=16`.
    Mode16 = 16,
    /// `/m=17`.
    Mode17 = 17,
    /// `/m=18`.
    Mode18 = 18,
    /// `/m=19`.
    Mode19 = 19,
    /// `/m=20`.
    Mode20 = 20,
    /// `/m=21`.
    Mode21 = 21,
    /// `/m=22`.
    Mode22 = 22,
}

/// Decoded host-window size from WBA1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ga1280aWindowSize {
    /// Host window is disabled or the size code is unknown.
    Disabled,
    /// 16 KB window.
    K16,
    /// 32 KB window.
    K32,
    /// 64 KB window.
    K64,
    /// 128 KB window.
    K128,
}

impl Ga1280aWindowSize {
    /// Returns the window byte length.
    pub fn bytes(self) -> Option<u32> {
        match self {
            Self::Disabled => None,
            Self::K16 => Some(16 * 1024),
            Self::K32 => Some(32 * 1024),
            Self::K64 => Some(64 * 1024),
            Self::K128 => Some(128 * 1024),
        }
    }
}

/// Active framebuffer interpretation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ga1280aPlaneMode {
    /// 8 one-bit planes, interpreted through the RAMDAC palette.
    Indexed8,
    /// 16 one-bit planes, interpreted as fixed RGB565.
    DirectColor16,
    /// 24 one-bit planes, interpreted as fixed RGB888.
    FullColor24,
}

/// RAMDAC hardware cursor inputs for a GA-1280A frame.
pub struct Ga1280aCursorRenderSnapshot<'a> {
    /// Whether the hardware cursor is visible.
    pub visible: bool,
    /// Raw RAMDAC cursor X position.
    pub x: u16,
    /// Raw RAMDAC cursor Y position.
    pub y: u16,
    /// Cursor background and foreground colors as RGB triples.
    pub colors: [[u8; 3]; 2],
    /// Cursor XOR pattern bytes.
    pub xor_pattern: &'a [u8],
    /// Cursor AND pattern bytes.
    pub and_pattern: &'a [u8],
}

/// Read-only GA-1280A render inputs captured from the device state.
pub struct Ga1280aRenderSnapshot<'a> {
    /// Active framebuffer interpretation.
    pub plane_mode: Ga1280aPlaneMode,
    /// Visible output width in pixels.
    pub width: u32,
    /// Visible output height in pixels.
    pub height: u32,
    /// Backing pixel-map width in pixels.
    pub pixel_map_width: u32,
    /// Backing pixel-map height in pixels.
    pub pixel_map_height: u32,
    /// Backing VRAM row stride in bytes.
    pub stride_bytes: u32,
    /// Display-start offset in pixels after mode-specific CRTC unit expansion.
    pub display_offset_pixels: u64,
    /// RAMDAC palette as RGB triples.
    pub palette: &'a [[u8; 3]; 256],
    /// RAMDAC visible palette mask.
    pub visible_mask: u8,
    /// Raw packed-pixel GA VRAM.
    pub vram: &'a [u8],
    /// Hardware cursor state for this frame.
    pub cursor: Ga1280aCursorRenderSnapshot<'a>,
}

/// Stateful pixel stream consumed after the `45E8h` image-restore command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ga1280aImageRestoreState {
    /// Destination left X.
    pub x: u32,
    /// Destination top Y.
    pub y: u32,
    /// Restore width in pixels.
    pub width: u32,
    /// Restore height in pixels.
    pub height: u32,
    /// Current destination pixel offset.
    pub pixel_index: u32,
    /// Current input column, including row padding.
    pub input_column: u32,
    /// Current input row.
    pub input_row: u32,
    /// Byte phase for RGB888 packed streams.
    pub byte_phase: u8,
    /// Partial RGB888 packed pixel.
    pub byte_accumulator: [u8; 3],
    /// Whether streamed pixels are XORed with the destination.
    pub xor_pixels: bool,
    /// Direction bits encoded in the POP2 opcode.
    pub direction: u8,
    /// Optional foreground ROP used by HGA image-transfer commands.
    pub rop: Option<u8>,
}

impl Ga1280aImageRestoreState {
    fn new(
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        xor_pixels: bool,
        direction: u8,
        rop: Option<u8>,
    ) -> Self {
        Self {
            x,
            y,
            width,
            height,
            pixel_index: 0,
            input_column: 0,
            input_row: 0,
            byte_phase: 0,
            byte_accumulator: [0; 3],
            xor_pixels,
            direction,
            rop,
        }
    }
}

/// Stateful pixel stream produced by the `20E8h` pixel-read command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ga1280aPixelReadState {
    /// Source left X.
    pub x: u32,
    /// Source top Y.
    pub y: u32,
    /// Read width in pixels.
    pub width: u32,
    /// Read height in rows.
    pub height: u32,
    /// Current row within the read rectangle.
    pub row: u32,
    /// Current 16-pixel chunk within the row.
    pub column: u32,
}

impl Ga1280aPixelReadState {
    fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            row: 0,
            column: 0,
        }
    }
}

/// Stateful monochrome pattern stream consumed after text POP2 commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ga1280aPatternExpandState {
    /// Destination left X.
    pub x: u32,
    /// Destination top Y.
    pub y: u32,
    /// Pattern width in pixels.
    pub width: u32,
    /// Pattern height in rows.
    pub height: u32,
    /// Current row within the pattern.
    pub row: u32,
    /// Current 16-pixel chunk within the row.
    pub column: u32,
    /// Current word phase: source bits first, then mask bits.
    pub word_phase: u8,
    /// Source bits captured for the current row.
    pub source_word: u16,
    /// Foreground color captured when the command was issued.
    pub foreground_color: u32,
    /// Background color captured when the command was issued.
    pub background_color: u32,
    /// Foreground ROP captured when the command was issued.
    pub foreground_mix: u8,
    /// Background ROP captured when the command was issued.
    pub background_mix: u8,
    /// Whether every source bit draws foreground or background.
    pub opaque: bool,
}

/// Active POP2 streaming operation. At most one of the three streams can be
/// active at a time on real hardware, so they share one slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ga1280aStreamState {
    /// No POP2 stream is in progress.
    Inactive,
    /// Active POP2=45E8h image-restore stream.
    ImageRestore(Ga1280aImageRestoreState),
    /// Active POP2=20E8h scanline-read stream.
    PixelRead(Ga1280aPixelReadState),
    /// Active POP2 text-pattern stream.
    PatternExpand(Ga1280aPatternExpandState),
}

/// Serializable GA board state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ga1280aState {
    /// Low base port, normally `00D8h`.
    pub gaport: u16,
    /// Current cyclic ID stream cursor.
    pub id_stream_cursor: u8,
    /// Base/index register.
    pub index: u16,
    /// Start raster for writes.
    pub srw: u16,
    /// Start raster for reads.
    pub srr: u16,
    /// Write plane mask.
    pub wpm: u16,
    /// Write bit mask.
    pub wbm: u16,
    /// Read plane select.
    pub prs: u8,
    /// Stored high byte for word accesses to the read plane select register.
    pub prs_high: u8,
    /// Read plane enable.
    pub rpe: u8,
    /// Stored high byte for word accesses to the read plane enable register.
    pub rpe_high: u8,
    /// Color register.
    pub col: u16,
    /// Tile pattern register.
    pub tile: u16,
    /// Tile pattern stream written through the tile register.
    pub tile_pattern: [u16; TILE_PATTERN_WORDS],
    /// Number of valid words in the tile pattern stream.
    pub tile_pattern_count: u8,
    /// Next tile pattern stream write index.
    pub tile_write_index: u8,
    /// Next tile pattern stream read index.
    pub tile_read_index: u8,
    /// 8-row brush pattern used by POP2=6A28h ROP rectangles.
    pub rop_pattern: [u8; ROP_PATTERN_ROWS],
    /// Next brush pattern row written through selector 14h offset 2.
    pub rop_pattern_index: u8,
    /// Rotation register.
    pub rot: u8,
    /// Stored high byte for word accesses to the rotation register.
    pub rot_high: u8,
    /// Controller mode register.
    pub mod1: u8,
    /// CRT/output selection register.
    pub mod2: u8,
    /// ROP foreground color.
    pub fcol: u16,
    /// ROP background color.
    pub bcol: u16,
    /// Foreground mix register.
    pub fmix: u8,
    /// Background mix register.
    pub bmix: u8,
    /// Clipping-window boundary/control register.
    pub cwb: u16,
    /// Clipping low X boundary.
    pub clip_sx: u16,
    /// Clipping low Y boundary.
    pub clip_sy: u16,
    /// Clipping high X boundary.
    pub clip_ex: u16,
    /// Clipping high Y boundary.
    pub clip_ey: u16,
    /// Whether the clipping window is active.
    pub clip_enabled: bool,
    /// Whether clipping accepts pixels outside, instead of inside, the window.
    pub clip_outside: bool,
    /// Raw memory-window register.
    pub wba1: u16,
    /// Last valid WBA1 window size, retained for mapped-register decode after closing WBA1.
    pub last_wba1_window_size: Ga1280aWindowSize,
    /// Secondary memory-window register.
    pub wba2: u16,
    /// RAMDAC palette write index.
    pub palette_index_write: u8,
    /// RAMDAC palette read index.
    pub palette_index_read: u8,
    /// RAMDAC RGB stream phase.
    pub palette_rgb_phase: u8,
    /// RAMDAC palette, stored as RGB triples.
    pub palette: Box<[[u8; 3]; 256]>,
    /// RAMDAC mask register.
    pub vdac_mask: u8,
    /// RAMDAC register select/bank register.
    pub vdac_rs: u8,
    /// RAMDAC cursor-color write index.
    pub cursor_color_index: u8,
    /// RAMDAC cursor-color RGB stream phase.
    pub cursor_color_rgb_phase: u8,
    /// RAMDAC cursor colors, stored as RGB triples.
    pub cursor_colors: [[u8; 3]; 2],
    /// RAMDAC cursor pattern stream index.
    pub cursor_pattern_index: u16,
    /// RAMDAC cursor XOR mask.
    pub cursor_xor_pattern: [u8; CURSOR_MASK_BYTES],
    /// RAMDAC cursor AND mask.
    pub cursor_and_pattern: [u8; CURSOR_MASK_BYTES],
    /// Raw RAMDAC cursor X position.
    pub cursor_x: u16,
    /// Raw RAMDAC cursor Y position.
    pub cursor_y: u16,
    /// Whether the RAMDAC cursor position has been programmed.
    pub cursor_visible: bool,
    /// System register SYS1.
    pub system_register: u16,
    /// System auxiliary register SYS2.
    pub system_auxiliary_register: u8,
    /// Current CRTC index.
    pub crtc_index: u8,
    /// CRTC register storage.
    pub crtc_registers: Box<[u16; 128]>,
    /// Active visible width in pixels.
    pub active_width: u32,
    /// Active visible height in pixels.
    pub active_height: u32,
    /// Active VRAM interpretation.
    pub plane_mode: Ga1280aPlaneMode,
    /// Packed-pixel board VRAM. Layout depends on `plane_mode`: 1, 2, or 3 bytes per pixel
    /// for `Indexed8`, `DirectColor16`, and `FullColor24` respectively. Stride is
    /// `pixel_map_width * bytes_per_pixel`. The total byte budget matches the board VRAM size
    /// so high-bpp modes lose addressable area, mirroring the hardware's fixed VRAM size.
    pub vram: Box<[u8]>,
    /// Accelerator line error term.
    pub errs: u16,
    /// Accelerator line slope term 1.
    pub k1: u16,
    /// Accelerator line slope term 2.
    pub k2: u16,
    /// Accelerator operand 1.
    pub opd1: u16,
    /// Accelerator operand 2.
    pub opd2: u16,
    /// Accelerator line style.
    pub lins: u16,
    /// Accelerator source X.
    pub srcx: u16,
    /// Accelerator source Y.
    pub srcy: u16,
    /// Accelerator destination X.
    pub dstx: u16,
    /// Accelerator destination Y.
    pub dsty: u16,
    /// Pixel-map width.
    pub pmw: u16,
    /// Pixel-map height.
    pub pmh: u16,
    /// Pixel data readback register.
    pub pdt: u16,
    /// Pixel data readback shift-register contents.
    pub pdt_latch: [u16; 4],
    /// Current pixel data readback phase.
    pub pdt_read_phase: u8,
    /// Save/restore state register.
    pub ssv: u16,
    /// Status/control byte written by the Windows drivers.
    pub status_control: u8,
    /// Command parameter register.
    pub pop1: u16,
    /// Command trigger/opcode register.
    pub pop2: u16,
    /// Unknown selector 0Fh register from the GAINIT reset list.
    pub unknown_sel_0f_off0: u16,
    /// Unknown selector 14h offset 2 register from the GAINIT reset list.
    pub unknown_sel_14_off2: u16,
    /// Unknown selector 15h offset 2 byte from the GAINIT reset list.
    pub unknown_sel_15_off2: u8,
    /// Number of writes accepted by the device.
    pub register_write_count: u64,
    /// Number of WBA1 writes accepted by the device.
    pub wba1_write_count: u64,
    /// Number of CRTC data writes accepted by the device.
    pub crtc_write_count: u64,
    /// Number of RAMDAC writes accepted by the device.
    pub ramdac_write_count: u64,
    /// Number of byte writes through a GA VRAM host window.
    pub host_window_write_count: u64,
    /// Number of byte writes through the F00000h flat aperture.
    pub flat_aperture_write_count: u64,
    /// Number of reset-list unknown writes accepted by the device.
    pub reset_unknown_write_count: u64,
    /// Number of unknown accelerator opcodes reported.
    pub unknown_command_warning_count: u64,
    /// Number of unknown accelerator mix combinations reported.
    pub unknown_mix_warning_count: u64,
    /// Live vertical-blanking state.
    pub vsync_active: bool,
    /// Progress through the inferred full-color helper sequence.
    pub full_color_helper_step: u8,
    /// Active POP2 streaming operation.
    pub stream: Ga1280aStreamState,
}

impl Ga1280aState {
    fn new() -> Self {
        Self {
            gaport: DEFAULT_GAPORT,
            id_stream_cursor: 0,
            index: 0,
            srw: 0,
            srr: 0,
            wpm: 0,
            wbm: 0,
            prs: 0,
            prs_high: 0,
            rpe: 0,
            rpe_high: 0,
            col: 0,
            tile: 0,
            tile_pattern: [0; TILE_PATTERN_WORDS],
            tile_pattern_count: 0,
            tile_write_index: 0,
            tile_read_index: 0,
            rop_pattern: [0xFF; ROP_PATTERN_ROWS],
            rop_pattern_index: 0,
            rot: 0,
            rot_high: 0,
            mod1: 0,
            mod2: 0,
            fcol: 0,
            bcol: 0,
            fmix: 0,
            bmix: 0,
            cwb: 0,
            clip_sx: 0,
            clip_sy: 0,
            clip_ex: (DEFAULT_WIDTH - 1) as u16,
            clip_ey: (DEFAULT_HEIGHT - 1) as u16,
            clip_enabled: false,
            clip_outside: false,
            wba1: 0,
            last_wba1_window_size: Ga1280aWindowSize::Disabled,
            wba2: 0,
            palette_index_write: 0,
            palette_index_read: 0,
            palette_rgb_phase: 0,
            palette: Box::new([[0; 3]; 256]),
            vdac_mask: 0,
            vdac_rs: 0,
            cursor_color_index: 0,
            cursor_color_rgb_phase: 0,
            cursor_colors: [[0; 3]; 2],
            cursor_pattern_index: 0,
            cursor_xor_pattern: [0; CURSOR_MASK_BYTES],
            cursor_and_pattern: [0xFF; CURSOR_MASK_BYTES],
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: false,
            system_register: 0,
            system_auxiliary_register: 0,
            crtc_index: 0,
            crtc_registers: Box::new([0; 128]),
            active_width: DEFAULT_WIDTH,
            active_height: DEFAULT_HEIGHT,
            plane_mode: Ga1280aPlaneMode::Indexed8,
            vram: vec![0; GA1280_VRAM_BYTES].into_boxed_slice(),
            errs: 0,
            k1: 0,
            k2: 0,
            opd1: 0,
            opd2: 0,
            lins: 0,
            srcx: 0,
            srcy: 0,
            dstx: 0,
            dsty: 0,
            pmw: 0,
            pmh: 0,
            pdt: 0,
            pdt_latch: [0; 4],
            pdt_read_phase: 0,
            ssv: 0,
            status_control: 0,
            pop1: 0,
            pop2: 0,
            unknown_sel_0f_off0: 0,
            unknown_sel_14_off2: 0,
            unknown_sel_15_off2: 0,
            register_write_count: 0,
            wba1_write_count: 0,
            crtc_write_count: 0,
            ramdac_write_count: 0,
            host_window_write_count: 0,
            flat_aperture_write_count: 0,
            reset_unknown_write_count: 0,
            unknown_command_warning_count: 0,
            unknown_mix_warning_count: 0,
            vsync_active: false,
            full_color_helper_step: 0,
            stream: Ga1280aStreamState::Inactive,
        }
    }
}

/// Documented GA-1280A I/O port window: selector 0x01..=0x1F, low byte 0xD8..=0xDB
/// (SW1 1-2-3 factory setting), plus the WBA1 byte aperture at [`FIXED_WINDOW_PORT`].
pub const fn is_ga1280a_port(port: u16) -> bool {
    let high = (port >> 8) as u8;
    let low = port as u8;
    (matches!(high, 0x01..=0x1F) && matches!(low, 0xD8..=0xDB))
        || port == FIXED_WINDOW_PORT
        || port == FIXED_WINDOW_PORT + 1
}

fn clamp_visible_width(width: u32) -> u32 {
    width.clamp(1, GA1280_MAX_VISIBLE_WIDTH)
}

fn clamp_visible_height(height: u32) -> u32 {
    height.clamp(1, GA1280_MAX_VISIBLE_HEIGHT)
}

fn clamp_pixel_map_width(width: u32) -> u32 {
    width.clamp(1, GA1280_MAX_PIXEL_MAP_WIDTH)
}

fn clamp_pixel_map_height(height: u32) -> u32 {
    height.clamp(1, GA1280_MAX_PIXEL_MAP_HEIGHT)
}

/// I-O DATA GA-1280A accelerator board.
pub struct Ga1280a {
    /// Embedded state for save/restore.
    pub state: Ga1280aState,
    /// Reusable Bresenham scratch buffer for POP2 line commands.
    pub(crate) line_points: Vec<(usize, i32, i32)>,
}

impl Default for Ga1280a {
    fn default() -> Self {
        Self::new()
    }
}

impl Ga1280a {
    /// Creates a new GA-1280A graphic board.
    pub fn new() -> Self {
        Self {
            state: Ga1280aState::new(),
            line_points: Vec::new(),
        }
    }

    /// Creates a board from a previously saved state.
    pub fn from_state(mut state: Ga1280aState) -> Self {
        state.gaport = DEFAULT_GAPORT;
        state.active_width = clamp_visible_width(state.active_width);
        state.active_height = clamp_visible_height(state.active_height);
        Self {
            state,
            line_points: Vec::new(),
        }
    }

    /// Returns the active GA mode refresh rate in Hz.
    ///
    /// Identifies the mode by looking up `(CRTC[0x00], CRTC[0x10])` in
    /// [`MODE_REFRESH_TABLE`]. Falls back to [`DEFAULT_REFRESH_HZ`] when
    /// the CRTC has not been programmed or no entry matches.
    pub fn detect_refresh_hz(&self) -> u32 {
        let horizontal_total = self.state.crtc_registers[CRTC_INDEX_HORIZONTAL_TOTAL];
        let vertical_total = self.state.crtc_registers[CRTC_INDEX_VERTICAL_TOTAL];
        for &(horiz, vert, hz) in MODE_REFRESH_TABLE {
            if horiz == horizontal_total && vert == vertical_total {
                return hz;
            }
        }
        DEFAULT_REFRESH_HZ
    }

    /// Returns the CPU-cycle length of the active GA display period.
    pub fn display_period_cycles(&self, cpu_clock_hz: u32) -> u64 {
        let (display, _blanking) = self.period_split_cycles(cpu_clock_hz);
        display
    }

    /// Returns the CPU-cycle length of the active GA vertical blanking period.
    pub fn blanking_period_cycles(&self, cpu_clock_hz: u32) -> u64 {
        let (_display, blanking) = self.period_split_cycles(cpu_clock_hz);
        blanking
    }

    fn period_split_cycles(&self, cpu_clock_hz: u32) -> (u64, u64) {
        let refresh_hz = self.detect_refresh_hz().max(1);
        let frame_cycles = u64::from(cpu_clock_hz) / u64::from(refresh_hz);
        let vertical_total = u32::from(self.state.crtc_registers[CRTC_INDEX_VERTICAL_TOTAL]) + 1;
        let vertical_display_end =
            u32::from(self.state.crtc_registers[CRTC_INDEX_VERTICAL_DISPLAY_END]) + 1;
        let (active_lines, total_lines) = if vertical_total <= 1 || vertical_display_end <= 1 {
            (DEFAULT_ACTIVE_LINES, DEFAULT_TOTAL_LINES)
        } else if vertical_display_end >= vertical_total {
            (vertical_total, vertical_total)
        } else {
            (vertical_display_end, vertical_total)
        };
        let display = frame_cycles * u64::from(active_lines) / u64::from(total_lines);
        let blanking = frame_cycles.saturating_sub(display);
        (display.max(1), blanking.max(1))
    }

    /// Returns the status register value observed by GAINIT.
    pub fn status_register(&self) -> u8 {
        let ready = 0x10;
        let direct_crtc_38 = 0x40;
        ready | direct_crtc_38 | 0x03
    }

    fn crtc_matches_full_color_mode(&self) -> bool {
        self.state.crtc_registers[0x00] == 0x00A6
            && self.state.crtc_registers[0x02] == 0x007F
            && self.state.crtc_registers[0x10] == 0x020B
            && self.state.crtc_registers[0x12] == 0x01DF
            && self.state.crtc_registers[0x36] == 0x5084
    }

    fn enter_full_color_mode(&mut self) {
        self.state.plane_mode = Ga1280aPlaneMode::FullColor24;
        self.state.active_width = FULL_COLOR_WIDTH;
        self.state.active_height = FULL_COLOR_HEIGHT;
    }

    fn update_full_color_mode_from_crtc(&mut self) {
        if self.crtc_matches_full_color_mode() {
            self.enter_full_color_mode();
        }
    }

    fn observe_full_color_helper_write(&mut self, selector: u8, offset: u8, value: u8) {
        if !self.crtc_matches_full_color_mode() {
            self.state.full_color_helper_step = 0;
            return;
        }

        self.observe_ga1280_full_color_helper_write(selector, offset, value);
    }

    fn observe_ga1280_full_color_helper_write(&mut self, selector: u8, offset: u8, value: u8) {
        let expected = match self.state.full_color_helper_step {
            0 => (SELECTOR_VDAC_ARW_RS, OFFSET_BASE_PLUS_ONE, 0x02),
            1 => (SELECTOR_VDAC_ARW_RS, OFFSET_BASE, 0x18),
            2 => (SELECTOR_VDAC_ARW_RS, OFFSET_BASE_PLUS_ONE, 0x01),
            3 => (SELECTOR_VDAC_MSK, OFFSET_BASE, 0x22),
            4 => (SELECTOR_VDAC_ARW_RS, OFFSET_BASE_PLUS_ONE, 0x00),
            5 => (SELECTOR_SYSTEM_PDT, OFFSET_BASE, 0x03),
            _ => {
                self.state.full_color_helper_step = 0;
                return;
            }
        };

        if (selector, offset, value) == expected {
            self.state.full_color_helper_step += 1;
            if self.state.full_color_helper_step == 6 {
                self.enter_full_color_mode();
                self.state.full_color_helper_step = 0;
            }
        } else {
            self.state.full_color_helper_step = 0;
        }
    }

    /// Reads a byte through the host VRAM window using a physical bus address.
    pub fn window_read_byte(&self, address: u32) -> Option<u8> {
        let offset = self.window_offset(address)?;
        Some(self.host_window_read(offset))
    }

    /// Writes a byte through the host VRAM window using a physical bus address.
    pub fn window_write_byte(&mut self, address: u32, value: u8) -> bool {
        let Some(offset) = self.window_offset(address) else {
            return false;
        };
        self.host_window_write(offset, value);
        true
    }

    /// Reads a byte through the mapped register aperture using a physical bus address.
    pub fn mapped_register_read_byte(&mut self, address: u32) -> Option<u8> {
        let offset = self.mapped_register_offset(address)?;
        self.host_window_mapped_register_read_byte(offset)
    }

    /// Writes a byte through the mapped register aperture using a physical bus address.
    pub fn mapped_register_write_byte(&mut self, address: u32, value: u8) -> bool {
        let Some(offset) = self.mapped_register_offset(address) else {
            return false;
        };
        self.host_window_mapped_register_write_byte(offset, value)
    }

    /// Reads a word through the mapped register aperture using a physical bus address.
    pub fn mapped_register_read_word(&mut self, address: u32) -> Option<u16> {
        let offset = self.mapped_register_offset(address)?;
        self.host_window_mapped_register_read_word(offset)
    }

    /// Writes a word through the mapped register aperture using a physical bus address.
    pub fn mapped_register_write_word(&mut self, address: u32, value: u16) -> bool {
        let Some(offset) = self.mapped_register_offset(address) else {
            return false;
        };
        self.host_window_mapped_register_write_word(offset, value)
    }

    /// Reads a byte through the protected-mode flat aperture using a physical bus address.
    pub fn flat_aperture_read_byte(&mut self, address: u32) -> Option<u8> {
        let offset = self.flat_aperture_offset(address, 1)?;
        Some(self.flat_aperture_read_byte_at_offset(offset))
    }

    /// Writes a byte through the protected-mode flat aperture using a physical bus address.
    pub fn flat_aperture_write_byte(&mut self, address: u32, value: u8) -> bool {
        let Some(offset) = self.flat_aperture_offset(address, 1) else {
            return false;
        };
        self.flat_aperture_write_byte_at_offset(offset, value);
        true
    }

    /// Reads a word through the protected-mode flat aperture using a physical bus address.
    pub fn flat_aperture_read_word(&mut self, address: u32) -> Option<u16> {
        let offset = self.flat_aperture_offset(address, 2)?;
        Some(self.flat_aperture_read_word_at_offset(offset))
    }

    /// Writes a word through the protected-mode flat aperture using a physical bus address.
    pub fn flat_aperture_write_word(&mut self, address: u32, value: u16) -> bool {
        let Some(offset) = self.flat_aperture_offset(address, 2) else {
            return false;
        };
        self.flat_aperture_write_word_at_offset(offset, value);
        true
    }

    /// Reads a doubleword through the protected-mode flat aperture using a physical bus address.
    pub fn flat_aperture_read_dword(&mut self, address: u32) -> Option<u32> {
        let offset = self.flat_aperture_offset(address, 4)?;
        Some(self.flat_aperture_read_dword_at_offset(offset))
    }

    /// Writes a doubleword through the protected-mode flat aperture using a physical bus address.
    pub fn flat_aperture_write_dword(&mut self, address: u32, value: u32) -> bool {
        let Some(offset) = self.flat_aperture_offset(address, 4) else {
            return false;
        };
        self.flat_aperture_write_dword_at_offset(offset, value);
        true
    }

    fn window_offset(&self, address: u32) -> Option<u32> {
        let size = match self.window_size() {
            Ga1280aWindowSize::Disabled => return None,
            size => size.bytes()?,
        };
        let base = u32::from(self.window_segment()) << 4;
        address.checked_sub(base).filter(|offset| *offset < size)
    }

    fn mapped_register_offset(&self, address: u32) -> Option<u32> {
        if let Some(offset) = self.window_offset(address) {
            return Some(offset);
        }

        if self.window_size() != Ga1280aWindowSize::Disabled {
            return None;
        }

        let size = self.closed_wba1_mapped_window_size()?.bytes()?;
        address
            .checked_sub(CONVENTIONAL_WINDOW_BASE)
            .filter(|offset| *offset < CONVENTIONAL_WINDOW_BYTES)
            .map(|offset| offset % size)
    }

    fn flat_aperture_offset(&self, address: u32, bytes: u32) -> Option<u32> {
        if bytes == 0 {
            return None;
        }
        let end = address.checked_add(bytes - 1)?;
        let aperture_end = FLAT_APERTURE_BASE + FLAT_APERTURE_BYTES;
        if address >= FLAT_APERTURE_BASE && end < aperture_end {
            Some(address - FLAT_APERTURE_BASE)
        } else {
            None
        }
    }

    fn window_segment(&self) -> u16 {
        let source = if Self::window_size_from(self.state.wba1).is_some() {
            self.state.wba1
        } else if Self::window_size_from(self.state.wba2).is_some() {
            self.state.wba2
        } else {
            self.state.wba1
        };
        Self::window_segment_from(source)
    }

    fn window_size(&self) -> Ga1280aWindowSize {
        // GALIB-style direct paths can leave WBA1's size nibble clear and
        // carry the active aperture segment and size in WBA2.
        Self::window_size_from(self.state.wba1)
            .or_else(|| Self::window_size_from(self.state.wba2))
            .unwrap_or(Ga1280aWindowSize::Disabled)
    }

    fn window_size_from(value: u16) -> Option<Ga1280aWindowSize> {
        match (value >> 8) & 0x00F0 {
            0x20 => Some(Ga1280aWindowSize::K16),
            0x30 => Some(Ga1280aWindowSize::K32),
            0x40 => Some(Ga1280aWindowSize::K64),
            0x50 => Some(Ga1280aWindowSize::K128),
            _ => None,
        }
    }

    fn window_segment_from(value: u16) -> u16 {
        if (value & WBA_LOW_BYTE_SEGMENT_MASK) == 0 {
            ((value >> 8) & 0x000F) << 12
        } else {
            (value & WBA_LOW_BYTE_SEGMENT_MASK) << 8
        }
    }

    fn closed_wba1_mapped_window_size(&self) -> Option<Ga1280aWindowSize> {
        if self.state.wba1 != 0 {
            return None;
        }
        match self.state.last_wba1_window_size {
            Ga1280aWindowSize::Disabled => None,
            size => Some(size),
        }
    }

    fn mapped_register_window_size(&self) -> Option<Ga1280aWindowSize> {
        if let Some(size) = Self::window_size_from(self.state.wba1) {
            return Some(size);
        }
        if self.state.wba1 == 0 {
            return match self.state.last_wba1_window_size {
                Ga1280aWindowSize::Disabled => None,
                size => Some(size),
            };
        }
        None
    }

    fn host_window_mapped_register_read_byte(&mut self, offset: u32) -> Option<u8> {
        if !self.mapped_register_aperture_enabled() {
            return None;
        }
        let (selector, register_offset, byte_offset) = self.mapped_register_address(offset)?;
        if byte_offset == 0 || register_offset == OFFSET_BASE {
            self.read_byte(selector, register_offset + byte_offset)
        } else {
            self.read_word(selector, register_offset)
                .map(|value| (value >> 8) as u8)
        }
    }

    fn host_window_mapped_register_write_byte(&mut self, offset: u32, value: u8) -> bool {
        if !self.mapped_register_aperture_enabled() {
            return false;
        }
        let Some((selector, register_offset, byte_offset)) = self.mapped_register_address(offset)
        else {
            return false;
        };
        if byte_offset == 0 || register_offset == OFFSET_BASE {
            return self.write_byte(selector, register_offset + byte_offset, value);
        }
        if let Some(previous_value) = self.read_word(selector, register_offset) {
            return self.write_word(
                selector,
                register_offset,
                (previous_value & 0x00FF) | (u16::from(value) << 8),
            );
        }
        false
    }

    fn host_window_mapped_register_read_word(&mut self, offset: u32) -> Option<u16> {
        if !self.mapped_register_aperture_enabled() {
            return None;
        }
        let (selector, register_offset, byte_offset) = self.mapped_register_address(offset)?;
        if byte_offset != 0 {
            return None;
        }
        self.read_word(selector, register_offset)
    }

    fn host_window_mapped_register_write_word(&mut self, offset: u32, value: u16) -> bool {
        if !self.mapped_register_aperture_enabled() {
            return false;
        }
        let Some((selector, register_offset, byte_offset)) = self.mapped_register_address(offset)
        else {
            return false;
        };
        byte_offset == 0 && self.write_word(selector, register_offset, value)
    }

    fn mapped_register_aperture_enabled(&self) -> bool {
        self.state.mod1 == 2 || self.mapped_register_window_size().is_some()
    }

    fn mapped_register_address(&self, offset: u32) -> Option<(u8, u8, u8)> {
        if self.mapped_register_window_size().is_some()
            && let Some(address) = Self::mapped_register_address_at(
                offset,
                COMPATIBILITY_MAPPED_REGISTER_BASE_OFFSET,
                COMPATIBILITY_MAPPED_REGISTER_PLUS_TWO_OFFSET,
            )
        {
            return Some(address);
        }

        let size = self
            .mapped_register_window_size()
            .and_then(|size| size.bytes())?;
        if size < 0x100 {
            return None;
        }

        // HGA*.DRV uses the register aperture at the end of the WBA1 window.
        let base = size - 0x100;
        Self::mapped_register_address_at(offset, base, base + 0x40)
    }

    fn write_wba1(&mut self, value: u16) {
        self.state.wba1 = value;
        if let Some(size) = Self::window_size_from(value)
            && Self::window_segment_from(value) != 0
        {
            self.state.last_wba1_window_size = size;
        }
        self.state.wba1_write_count += 1;
    }

    fn mapped_register_address_at(
        offset: u32,
        base_offset: u32,
        plus_two_offset: u32,
    ) -> Option<(u8, u8, u8)> {
        let (aperture_base, register_offset) =
            if (base_offset..base_offset + MAPPED_REGISTER_APERTURE_BYTES).contains(&offset) {
                (base_offset, OFFSET_BASE)
            } else if (plus_two_offset..plus_two_offset + MAPPED_REGISTER_APERTURE_BYTES)
                .contains(&offset)
            {
                (plus_two_offset, OFFSET_PLUS_TWO)
            } else {
                return None;
            };
        let relative_offset = offset - aperture_base;
        Some((
            (relative_offset / 2) as u8,
            register_offset,
            (relative_offset & 1) as u8,
        ))
    }

    /// Returns whether MOD2 selects the GA output.
    pub fn is_driving_monitor(&self) -> bool {
        self.state.mod2 & 0x80 != 0
    }

    /// Attempts to handle a byte-sized I/O read.
    pub fn try_handle_io_read_byte(&mut self, port: u16) -> Option<u8> {
        let (selector, offset) = Self::decode_port(port)?;
        self.read_byte(selector, offset)
    }

    /// Attempts to handle a byte-sized I/O write.
    pub fn try_handle_io_write_byte(&mut self, port: u16, value: u8) -> bool {
        let Some((selector, offset)) = Self::decode_port(port) else {
            return false;
        };
        self.write_byte(selector, offset, value)
    }

    /// Attempts to handle a word-sized I/O read.
    pub fn try_handle_io_read_word(&mut self, port: u16) -> Option<u16> {
        let (selector, offset) = Self::decode_port(port)?;
        self.read_word(selector, offset)
    }

    /// Attempts to handle a word-sized I/O write.
    pub fn try_handle_io_write_word(&mut self, port: u16, value: u16) -> bool {
        let Some((selector, offset)) = Self::decode_port(port) else {
            return false;
        };
        self.write_word(selector, offset, value)
    }

    /// Attempts to handle a byte-sized register read.
    pub fn try_read_byte(&mut self, selector: u8, offset: u8) -> Option<u8> {
        self.read_byte(selector, offset)
    }

    /// Attempts to handle a byte-sized register write.
    pub fn try_write_byte(&mut self, selector: u8, offset: u8, value: u8) -> bool {
        self.write_byte(selector, offset, value)
    }

    /// Attempts to handle a word-sized register read.
    pub fn try_read_word(&mut self, selector: u8, offset: u8) -> Option<u16> {
        self.read_word(selector, offset)
    }

    /// Attempts to handle a word-sized register write.
    pub fn try_write_word(&mut self, selector: u8, offset: u8, value: u16) -> bool {
        self.write_word(selector, offset, value)
    }

    fn decode_port(port: u16) -> Option<(u8, u8)> {
        if (FIXED_WINDOW_PORT..=FIXED_WINDOW_PORT + 1).contains(&port) {
            return Some((SELECTOR_WBA1, (port - FIXED_WINDOW_PORT) as u8));
        }
        let selector = (port >> 8) as u8;
        if !matches!(selector, 0x01..=0x1F) {
            return None;
        }
        let offset = match port & 0x00FF {
            DEFAULT_GAPORT => OFFSET_BASE,
            low if low == DEFAULT_GAPORT + 1 => OFFSET_BASE_PLUS_ONE,
            low if low == DEFAULT_GAPORT + 2 => OFFSET_PLUS_TWO,
            low if low == DEFAULT_GAPORT + 3 => OFFSET_PLUS_THREE,
            _ => return None,
        };
        Some((selector, offset))
    }

    fn read_byte(&mut self, selector: u8, offset: u8) -> Option<u8> {
        match (selector, offset) {
            (SELECTOR_PRS, OFFSET_BASE) => Some(self.state.prs),
            (SELECTOR_PRS, OFFSET_BASE_PLUS_ONE) => Some(self.state.prs_high),
            (SELECTOR_RPE, OFFSET_BASE) => Some(self.state.rpe),
            (SELECTOR_RPE, OFFSET_BASE_PLUS_ONE) => Some(self.state.rpe_high),
            (SELECTOR_ROT, OFFSET_BASE) => Some(self.state.rot),
            (SELECTOR_ROT, OFFSET_BASE_PLUS_ONE) => Some(self.state.rot_high),
            (SELECTOR_MOD, OFFSET_BASE) => Some(self.state.mod1),
            (SELECTOR_MOD, OFFSET_BASE_PLUS_ONE) => Some(self.state.mod2),
            (SELECTOR_MIX, OFFSET_BASE) => Some(self.state.fmix),
            (SELECTOR_MIX, OFFSET_BASE_PLUS_ONE) => Some(self.state.bmix),
            (SELECTOR_WBA1, OFFSET_BASE) => Some(self.state.wba1 as u8),
            (SELECTOR_WBA1, OFFSET_BASE_PLUS_ONE) => Some((self.state.wba1 >> 8) as u8),
            (SELECTOR_VDAC_ARW_RS, OFFSET_BASE) => Some(self.read_vdac_arw()),
            (SELECTOR_VDAC_ARW_RS, OFFSET_BASE_PLUS_ONE) => Some(self.state.vdac_rs),
            (SELECTOR_VDAC_ARR, OFFSET_BASE) => Some(self.read_vdac_arr()),
            (SELECTOR_VDAC_CPR, OFFSET_BASE) => Some(self.read_vdac_cpr()),
            (SELECTOR_VDAC_MSK, OFFSET_BASE) => Some(self.read_vdac_msk()),
            (SELECTOR_SYSTEM_PDT, OFFSET_BASE) => Some(self.state.system_register as u8),
            (SELECTOR_SYSTEM_PDT, OFFSET_BASE_PLUS_ONE) => {
                Some(self.state.system_auxiliary_register)
            }
            (SELECTOR_STATUS_SSV, OFFSET_BASE) => Some(self.status_register()),
            (SELECTOR_STATUS_SSV, OFFSET_BASE_PLUS_ONE) => Some(self.read_id_stream()),
            (SELECTOR_CRTC_POP1, OFFSET_BASE) => Some(self.state.crtc_index),
            (SELECTOR_CRTC_POP2, OFFSET_BASE) => {
                let index = (self.state.crtc_index & 0x7F) as usize;
                Some(self.crtc_data_word(index) as u8)
            }
            _ => self
                .read_word(selector, Self::word_base_offset(offset)?)
                .map(|value| {
                    if Self::is_high_byte_offset(offset) {
                        (value >> 8) as u8
                    } else {
                        value as u8
                    }
                }),
        }
    }

    fn crtc_data_word(&self, index: usize) -> u16 {
        let stored = self.state.crtc_registers[index];
        if index == CRTC_INDEX_VSYNC_STATUS {
            let masked = stored & !u16::from(CRTC_BIT_VSYNC_ACTIVE);
            return if self.state.vsync_active {
                masked | u16::from(CRTC_BIT_VSYNC_ACTIVE)
            } else {
                masked
            };
        }
        if index == CRTC_INDEX_GA1280_VSYNC_STATUS {
            let masked = stored & !CRTC_BIT_GA1280_VSYNC_ACTIVE;
            return if self.state.vsync_active {
                masked | CRTC_BIT_GA1280_VSYNC_ACTIVE
            } else {
                masked
            };
        }
        stored
    }

    fn write_crtc_data_low_byte(&mut self, index: usize, value: u8) {
        self.state.crtc_registers[index] =
            (self.state.crtc_registers[index] & 0xFF00) | u16::from(value);
        self.update_after_crtc_write();
        self.state.crtc_write_count += 1;
    }

    fn write_crtc_data_word(&mut self, index: usize, value: u16) {
        self.state.crtc_registers[index] = value;
        self.update_after_crtc_write();
        self.state.crtc_write_count += 1;
    }

    fn update_after_crtc_write(&mut self) {
        self.update_full_color_mode_from_crtc();
        self.update_dimensions_from_crtc();
    }

    fn write_byte(&mut self, selector: u8, offset: u8, value: u8) -> bool {
        match (selector, offset) {
            (SELECTOR_PRS, OFFSET_BASE) => self.state.prs = value,
            (SELECTOR_PRS, OFFSET_BASE_PLUS_ONE) => self.state.prs_high = value,
            (SELECTOR_RPE, OFFSET_BASE) => self.state.rpe = value,
            (SELECTOR_RPE, OFFSET_BASE_PLUS_ONE) => self.state.rpe_high = value,
            (SELECTOR_ROT, OFFSET_BASE) => self.state.rot = value,
            (SELECTOR_ROT, OFFSET_BASE_PLUS_ONE) => self.state.rot_high = value,
            (SELECTOR_MOD, OFFSET_BASE) => self.state.mod1 = value,
            (SELECTOR_MOD, OFFSET_BASE_PLUS_ONE) => self.state.mod2 = value,
            (SELECTOR_MIX, OFFSET_BASE) => self.state.fmix = value,
            (SELECTOR_MIX, OFFSET_BASE_PLUS_ONE) => self.state.bmix = value,
            (SELECTOR_WBA1, OFFSET_BASE) => {
                self.write_wba1((self.state.wba1 & 0xFF00) | u16::from(value));
            }
            (SELECTOR_WBA1, OFFSET_BASE_PLUS_ONE) => {
                self.write_wba1((self.state.wba1 & 0x00FF) | (u16::from(value) << 8));
            }
            (SELECTOR_VDAC_ARW_RS, OFFSET_BASE) => {
                self.write_vdac_arw(value);
                self.state.ramdac_write_count += 1;
            }
            (SELECTOR_VDAC_ARW_RS, OFFSET_BASE_PLUS_ONE) => {
                self.write_vdac_rs(value);
                self.state.ramdac_write_count += 1;
            }
            (SELECTOR_VDAC_ARR, OFFSET_BASE) => {
                self.write_vdac_arr(value);
                self.state.ramdac_write_count += 1;
            }
            (SELECTOR_VDAC_CPR, OFFSET_BASE) => {
                self.write_vdac_cpr(value);
                self.state.ramdac_write_count += 1;
            }
            (SELECTOR_VDAC_MSK, OFFSET_BASE) => {
                self.write_vdac_msk(value);
                self.state.ramdac_write_count += 1;
            }
            (SELECTOR_SYSTEM_PDT, OFFSET_BASE) => {
                self.state.system_register =
                    (self.state.system_register & 0xFF00) | u16::from(value);
                self.observe_full_color_helper_write(selector, offset, value);
            }
            (SELECTOR_SYSTEM_PDT, OFFSET_BASE_PLUS_ONE) => {
                self.state.system_auxiliary_register = value;
            }
            (SELECTOR_STATUS_SSV, OFFSET_BASE) => return false,
            (SELECTOR_CRTC_POP1, OFFSET_BASE) => self.state.crtc_index = value,
            (SELECTOR_CRTC_POP2, OFFSET_BASE) => {
                let index = (self.state.crtc_index & 0x7F) as usize;
                self.write_crtc_data_low_byte(index, value);
            }
            (SELECTOR_MIX, OFFSET_PLUS_TWO) => self.write_rop_pattern_byte(value),
            (SELECTOR_CWB_UNKNOWN, OFFSET_PLUS_TWO) => {
                self.reset_rop_pattern_stream(value);
            }
            _ => {
                let Some(word_offset) = Self::word_base_offset(offset) else {
                    return false;
                };
                let Some(previous_value) = self.read_word(selector, word_offset) else {
                    return false;
                };
                let new_value = if Self::is_high_byte_offset(offset) {
                    (previous_value & 0x00FF) | (u16::from(value) << 8)
                } else {
                    (previous_value & 0xFF00) | u16::from(value)
                };
                if !self.write_word(selector, word_offset, new_value) {
                    return false;
                }
                return true;
            }
        }
        self.state.register_write_count += 1;
        true
    }

    fn word_base_offset(offset: u8) -> Option<u8> {
        match offset {
            OFFSET_BASE | OFFSET_BASE_PLUS_ONE => Some(OFFSET_BASE),
            OFFSET_PLUS_TWO | OFFSET_PLUS_THREE => Some(OFFSET_PLUS_TWO),
            _ => None,
        }
    }

    fn is_high_byte_offset(offset: u8) -> bool {
        offset & 1 != 0
    }

    fn read_word(&mut self, selector: u8, offset: u8) -> Option<u16> {
        match (selector, offset) {
            (SELECTOR_INDEX, OFFSET_BASE) => Some(self.state.index),
            (SELECTOR_SRW, OFFSET_BASE) => Some(self.state.srw),
            (SELECTOR_SRR, OFFSET_BASE) => Some(self.state.srr),
            (SELECTOR_WPM, OFFSET_BASE) => Some(self.state.wpm),
            (SELECTOR_WBM, OFFSET_BASE) => Some(self.state.wbm),
            (SELECTOR_PRS, OFFSET_BASE) => {
                Some(u16::from(self.state.prs) | (u16::from(self.state.prs_high) << 8))
            }
            (SELECTOR_RPE, OFFSET_BASE) => {
                Some(u16::from(self.state.rpe) | (u16::from(self.state.rpe_high) << 8))
            }
            (SELECTOR_COL, OFFSET_BASE) => Some(self.state.col),
            (SELECTOR_TILE, OFFSET_BASE) => Some(self.read_tile_word()),
            (SELECTOR_UNKNOWN_0F, OFFSET_BASE) => Some(self.state.unknown_sel_0f_off0),
            (SELECTOR_FCOL, OFFSET_BASE) => Some(self.state.fcol),
            (SELECTOR_BCOL_PMW, OFFSET_BASE) => Some(self.state.bcol),
            (SELECTOR_MIX, OFFSET_BASE) => {
                Some(u16::from(self.state.fmix) | (u16::from(self.state.bmix) << 8))
            }
            (SELECTOR_CWB_UNKNOWN, OFFSET_BASE) => Some(self.state.cwb),
            (SELECTOR_WBA1, OFFSET_BASE) => Some(self.state.wba1),
            (SELECTOR_WBA2, OFFSET_BASE) => Some(self.state.wba2),
            (SELECTOR_SYSTEM_PDT, OFFSET_BASE) => Some(self.state.system_register),
            (SELECTOR_CRTC_POP2, OFFSET_BASE) => {
                let index = (self.state.crtc_index & 0x7F) as usize;
                Some(self.crtc_data_word(index))
            }
            (SELECTOR_STATUS_SSV, OFFSET_PLUS_TWO) => Some(self.state.ssv),
            (SELECTOR_CRTC_POP1, OFFSET_PLUS_TWO) => Some(self.state.pop1),
            (SELECTOR_CRTC_POP2, OFFSET_PLUS_TWO) => Some(self.state.pop2),
            (SELECTOR_SYSTEM_PDT, OFFSET_PLUS_TWO) => Some(self.read_pdt_word()),
            (SELECTOR_SRW, OFFSET_PLUS_TWO) => Some(self.state.errs),
            (SELECTOR_SRR, OFFSET_PLUS_TWO) => Some(self.state.k1),
            (SELECTOR_WPM, OFFSET_PLUS_TWO) => Some(self.state.k2),
            (0x04, OFFSET_PLUS_TWO) => Some(self.state.opd1),
            (SELECTOR_WBM, OFFSET_PLUS_TWO) => Some(self.state.opd2),
            (SELECTOR_PRS, OFFSET_PLUS_TWO) => Some(self.state.lins),
            (0x08, OFFSET_PLUS_TWO) => Some(self.state.srcx),
            (SELECTOR_COL, OFFSET_PLUS_TWO) => Some(self.state.srcy),
            (0x0A, OFFSET_PLUS_TWO) => Some(self.state.dstx),
            (SELECTOR_TILE, OFFSET_PLUS_TWO) => Some(self.state.dsty),
            (SELECTOR_BCOL_PMW, OFFSET_PLUS_TWO) => Some(self.state.pmw),
            (SELECTOR_PMH, OFFSET_PLUS_TWO) => Some(self.state.pmh),
            (SELECTOR_MIX, OFFSET_PLUS_TWO) => Some(self.state.unknown_sel_14_off2),
            _ => None,
        }
    }

    fn read_tile_word(&mut self) -> u16 {
        let count = usize::from(self.state.tile_pattern_count).min(TILE_PATTERN_WORDS);
        if count == 0 {
            return self.state.tile;
        }

        let index = usize::from(self.state.tile_read_index) % TILE_PATTERN_WORDS;
        let value = self.state.tile_pattern[index];
        let next_index = if count == TILE_PATTERN_WORDS {
            (index + 1) % TILE_PATTERN_WORDS
        } else {
            (index + 1) % count
        };
        self.state.tile_read_index = next_index as u8;
        value
    }

    fn write_tile_word(&mut self, value: u16) {
        self.state.tile = value;
        let index = usize::from(self.state.tile_write_index) % TILE_PATTERN_WORDS;
        self.state.tile_pattern[index] = value;
        self.state.tile_pattern_count =
            (usize::from(self.state.tile_pattern_count) + 1).min(TILE_PATTERN_WORDS) as u8;
        self.state.tile_write_index =
            ((usize::from(self.state.tile_write_index) + 1) % TILE_PATTERN_WORDS) as u8;
        self.state.tile_read_index =
            if usize::from(self.state.tile_pattern_count) == TILE_PATTERN_WORDS {
                self.state.tile_write_index
            } else {
                0
            };
    }

    fn reset_rop_pattern_stream(&mut self, value: u8) {
        self.state.unknown_sel_15_off2 = value;
        self.state.rop_pattern_index = value & 0x07;
        self.state.reset_unknown_write_count += 1;
    }

    fn write_rop_pattern_byte(&mut self, value: u8) {
        let index = usize::from(self.state.rop_pattern_index) % ROP_PATTERN_ROWS;
        self.state.rop_pattern[index] = value;
        self.state.rop_pattern_index =
            ((usize::from(self.state.rop_pattern_index) + 1) % ROP_PATTERN_ROWS) as u8;
    }

    fn write_rop_pattern_word(&mut self, value: u16) {
        self.state.unknown_sel_14_off2 = value;
        self.write_rop_pattern_byte(value as u8);
        self.write_rop_pattern_byte((value >> 8) as u8);
    }

    fn write_word(&mut self, selector: u8, offset: u8, value: u16) -> bool {
        match (selector, offset) {
            (SELECTOR_INDEX, OFFSET_BASE) => self.state.index = value,
            (SELECTOR_SRW, OFFSET_BASE) => self.state.srw = value,
            (SELECTOR_SRR, OFFSET_BASE) => self.state.srr = value,
            (SELECTOR_WPM, OFFSET_BASE) => self.state.wpm = value,
            (SELECTOR_WBM, OFFSET_BASE) => self.state.wbm = value,
            (SELECTOR_PRS, OFFSET_BASE) => {
                self.state.prs = value as u8;
                self.state.prs_high = (value >> 8) as u8;
            }
            (SELECTOR_RPE, OFFSET_BASE) => {
                self.state.rpe = value as u8;
                self.state.rpe_high = (value >> 8) as u8;
            }
            (SELECTOR_COL, OFFSET_BASE) => self.state.col = value,
            (SELECTOR_TILE, OFFSET_BASE) => self.write_tile_word(value),
            (SELECTOR_ROT, OFFSET_BASE) => {
                self.state.rot = value as u8;
                self.state.rot_high = (value >> 8) as u8;
            }
            (SELECTOR_MOD, OFFSET_BASE) => {
                self.state.mod1 = value as u8;
                self.state.mod2 = (value >> 8) as u8;
            }
            (SELECTOR_UNKNOWN_0F, OFFSET_BASE) => {
                self.state.unknown_sel_0f_off0 = value;
                self.state.reset_unknown_write_count += 1;
            }
            (SELECTOR_FCOL, OFFSET_BASE) => self.state.fcol = value,
            (SELECTOR_BCOL_PMW, OFFSET_BASE) => self.state.bcol = value,
            (SELECTOR_MIX, OFFSET_BASE) => {
                self.state.fmix = value as u8;
                self.state.bmix = (value >> 8) as u8;
            }
            (SELECTOR_CWB_UNKNOWN, OFFSET_BASE) => self.write_cwb(value),
            (SELECTOR_WBA1, OFFSET_BASE) => self.write_wba1(value),
            (SELECTOR_WBA2, OFFSET_BASE) => self.state.wba2 = value,
            (SELECTOR_SYSTEM_PDT, OFFSET_BASE) => self.state.system_register = value,
            (SELECTOR_CRTC_POP1, OFFSET_BASE) => {
                self.state.crtc_index = value as u8;
                let index = (value as u8 & 0x7F) as usize;
                let data = (value >> 8) as u8;
                self.write_crtc_data_low_byte(index, data);
            }
            (SELECTOR_CRTC_POP2, OFFSET_BASE) => {
                let index = (self.state.crtc_index & 0x7F) as usize;
                self.write_crtc_data_word(index, value);
            }
            (SELECTOR_SRW, OFFSET_PLUS_TWO) => self.state.errs = value,
            (SELECTOR_SRR, OFFSET_PLUS_TWO) => self.state.k1 = value,
            (SELECTOR_WPM, OFFSET_PLUS_TWO) => self.state.k2 = value,
            (0x04, OFFSET_PLUS_TWO) => self.state.opd1 = value,
            (SELECTOR_WBM, OFFSET_PLUS_TWO) => self.state.opd2 = value,
            (SELECTOR_PRS, OFFSET_PLUS_TWO) => self.state.lins = value,
            (0x08, OFFSET_PLUS_TWO) => self.state.srcx = value,
            (SELECTOR_COL, OFFSET_PLUS_TWO) => self.state.srcy = value,
            (0x0A, OFFSET_PLUS_TWO) => self.state.dstx = value,
            (SELECTOR_TILE, OFFSET_PLUS_TWO) => self.state.dsty = value,
            (SELECTOR_MIX, OFFSET_PLUS_TWO) => {
                self.write_rop_pattern_word(value);
                self.state.reset_unknown_write_count += 1;
            }
            (SELECTOR_BCOL_PMW, OFFSET_PLUS_TWO) => self.state.pmw = value,
            (SELECTOR_PMH, OFFSET_PLUS_TWO) => self.state.pmh = value,
            (SELECTOR_SYSTEM_PDT, OFFSET_PLUS_TWO) => {
                self.write_pdt_word(value);
            }
            (SELECTOR_STATUS_SSV, OFFSET_PLUS_TWO) => self.state.ssv = value,
            (SELECTOR_CRTC_POP1, OFFSET_PLUS_TWO) => self.state.pop1 = value,
            (SELECTOR_CRTC_POP2, OFFSET_PLUS_TWO) => {
                self.state.pop2 = value;
                self.execute_pop2(value);
            }
            _ => return false,
        }
        self.state.register_write_count += 1;
        true
    }

    fn read_id_stream(&mut self) -> u8 {
        let value = ID_STREAM[self.state.id_stream_cursor as usize % ID_STREAM.len()];
        self.state.id_stream_cursor = self.state.id_stream_cursor.wrapping_add(1) & 0x0F;
        value
    }

    fn cursor_bank(&self, bank: u8) -> bool {
        self.state.vdac_rs == bank
    }

    fn read_vdac_arw(&self) -> u8 {
        if self.cursor_bank(1) {
            self.state.cursor_color_index
        } else if self.cursor_bank(3) {
            self.state.cursor_x as u8
        } else {
            self.state.palette_index_write
        }
    }

    fn read_vdac_arr(&self) -> u8 {
        if self.cursor_bank(3) {
            (self.state.cursor_y >> 8) as u8
        } else {
            self.state.palette_index_read
        }
    }

    fn read_vdac_cpr(&mut self) -> u8 {
        if self.cursor_bank(1) {
            self.read_cursor_color_component()
        } else if self.cursor_bank(3) {
            (self.state.cursor_x >> 8) as u8
        } else {
            self.read_palette_component()
        }
    }

    fn read_vdac_msk(&self) -> u8 {
        if self.cursor_bank(3) {
            self.state.cursor_y as u8
        } else {
            self.state.vdac_mask
        }
    }

    fn write_vdac_rs(&mut self, value: u8) {
        self.state.vdac_rs = value;
        match value {
            1 => self.state.cursor_color_rgb_phase = 0,
            2 => self.state.cursor_pattern_index = 0,
            _ => {}
        }
        self.observe_full_color_helper_write(SELECTOR_VDAC_ARW_RS, OFFSET_BASE_PLUS_ONE, value);
    }

    fn write_vdac_arw(&mut self, value: u8) {
        if self.cursor_bank(1) {
            self.state.cursor_color_index = value & 1;
            self.state.cursor_color_rgb_phase = 0;
            return;
        }
        if self.cursor_bank(3) {
            self.state.cursor_x = (self.state.cursor_x & 0xFF00) | u16::from(value);
            return;
        }

        self.state.palette_index_write = value;
        self.state.palette_rgb_phase = 0;
        self.update_plane_mode_after_vdac_index_write(value);
        self.observe_full_color_helper_write(SELECTOR_VDAC_ARW_RS, OFFSET_BASE, value);
    }

    fn write_vdac_arr(&mut self, value: u8) {
        if self.cursor_bank(2) {
            self.write_cursor_pattern_byte(value);
            return;
        }
        if self.cursor_bank(3) {
            self.state.cursor_y = (self.state.cursor_y & 0x00FF) | (u16::from(value) << 8);
            self.state.cursor_visible = true;
            return;
        }

        self.state.palette_index_read = value;
        self.state.palette_rgb_phase = 0;
    }

    fn write_vdac_cpr(&mut self, value: u8) {
        if self.cursor_bank(1) {
            self.write_cursor_color_component(value);
            return;
        }
        if self.cursor_bank(3) {
            self.state.cursor_x = (self.state.cursor_x & 0x00FF) | (u16::from(value) << 8);
            return;
        }

        self.write_palette_component(value);
    }

    fn write_vdac_msk(&mut self, value: u8) {
        if self.cursor_bank(3) {
            self.state.cursor_y = (self.state.cursor_y & 0xFF00) | u16::from(value);
            return;
        }

        self.state.vdac_mask = value;
        self.update_plane_mode_after_vdac_mask_write(value);
        self.observe_full_color_helper_write(SELECTOR_VDAC_MSK, OFFSET_BASE, value);
    }

    fn write_cursor_pattern_byte(&mut self, value: u8) {
        let index = usize::from(self.state.cursor_pattern_index) % CURSOR_PATTERN_BYTES;
        if index < CURSOR_MASK_BYTES {
            self.state.cursor_xor_pattern[index] = value;
        } else {
            self.state.cursor_and_pattern[index - CURSOR_MASK_BYTES] = value;
        }
        self.state.cursor_pattern_index =
            (self.state.cursor_pattern_index + 1) % CURSOR_PATTERN_BYTES_U16;
    }

    fn write_cursor_color_component(&mut self, value: u8) {
        let index = usize::from(self.state.cursor_color_index & 1);
        let phase = usize::from(self.state.cursor_color_rgb_phase % 3);
        self.state.cursor_colors[index][phase] = value;
        self.state.cursor_color_rgb_phase = (self.state.cursor_color_rgb_phase + 1) % 3;
        if self.state.cursor_color_rgb_phase == 0 {
            self.state.cursor_color_index = self.state.cursor_color_index.wrapping_add(1) & 1;
        }
    }

    fn read_cursor_color_component(&mut self) -> u8 {
        let index = usize::from(self.state.cursor_color_index & 1);
        let phase = usize::from(self.state.cursor_color_rgb_phase % 3);
        let value = self.state.cursor_colors[index][phase];
        self.state.cursor_color_rgb_phase = (self.state.cursor_color_rgb_phase + 1) % 3;
        if self.state.cursor_color_rgb_phase == 0 {
            self.state.cursor_color_index = self.state.cursor_color_index.wrapping_add(1) & 1;
        }
        value
    }

    fn write_palette_component(&mut self, value: u8) {
        let index = self.state.palette_index_write as usize;
        let phase = self.state.palette_rgb_phase as usize % 3;
        self.state.palette[index][phase] = value;
        self.state.palette_rgb_phase = (self.state.palette_rgb_phase + 1) % 3;
        if self.state.palette_rgb_phase == 0 {
            self.state.palette_index_write = self.state.palette_index_write.wrapping_add(1);
        }
    }

    fn read_palette_component(&mut self) -> u8 {
        let index = self.state.palette_index_read as usize;
        let phase = self.state.palette_rgb_phase as usize % 3;
        let value = self.state.palette[index][phase];
        self.state.palette_rgb_phase = (self.state.palette_rgb_phase + 1) % 3;
        if self.state.palette_rgb_phase == 0 {
            self.state.palette_index_read = self.state.palette_index_read.wrapping_add(1);
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_display_start(ga: &mut Ga1280a, value: u32) {
        ga.state.crtc_registers[CRTC_INDEX_DISPLAY_START_LOW] = (value & 0xFF) as u16;
        ga.state.crtc_registers[CRTC_INDEX_DISPLAY_START_MID] = ((value >> 8) & 0xFF) as u16;
        ga.state.crtc_registers[CRTC_INDEX_DISPLAY_START_HIGH] = ((value >> 16) & 0xFF) as u16;
    }

    #[test]
    fn id_stream_cycles() {
        let mut ga = Ga1280a::new();
        let mut bytes = Vec::new();
        for _ in 0..32 {
            bytes.push(ga.try_read_byte(SELECTOR_STATUS_SSV, 1).unwrap());
        }
        assert_eq!(&bytes[..16], ID_STREAM);
        assert_eq!(&bytes[16..], ID_STREAM);
    }

    #[test]
    fn status_value_identifies_ga1280_class() {
        assert_eq!(Ga1280a::new().status_register(), 0x53);
    }

    #[test]
    fn wba1_byte_and_word_access_round_trips() {
        let mut ga = Ga1280a::new();
        assert!(ga.try_write_byte(SELECTOR_WBA1, 0, 0xC1));
        assert!(ga.try_write_byte(SELECTOR_WBA1, 1, 0x02));
        assert_eq!(ga.try_read_word(SELECTOR_WBA1, 0), Some(0x02C1));

        assert!(ga.try_write_word(SELECTOR_WBA1, 0, 0x50DD));
        assert_eq!(ga.try_read_byte(SELECTOR_WBA1, 0), Some(0xDD));
        assert_eq!(ga.try_read_byte(SELECTOR_WBA1, 1), Some(0x50));
        assert_eq!(ga.window_segment(), 0xDC00);
        assert_eq!(ga.window_size(), Ga1280aWindowSize::K128);
    }

    #[test]
    fn wba2_high_nibble_can_supply_window_size() {
        let mut ga = Ga1280a::new();
        assert!(ga.try_write_word(SELECTOR_WBA1, 0, 0x00E9));
        assert!(ga.try_write_word(SELECTOR_WBA2, 0, 0x30E1));
        assert_eq!(ga.window_segment(), 0xE000);
        assert_eq!(ga.window_size(), Ga1280aWindowSize::K32);
    }

    #[test]
    fn ga1280_wba1_decodes_segment_from_high_byte_low_nibble() {
        let mut ga = Ga1280a::new();
        assert!(ga.try_write_word(SELECTOR_WBA1, 0, 0x3F01));

        assert_eq!(ga.window_segment(), 0xF000);
        assert_eq!(ga.window_size(), Ga1280aWindowSize::K32);
    }

    #[test]
    fn ga1280_wba1_decodes_segment_from_low_byte_when_present() {
        let mut ga = Ga1280a::new();
        assert!(ga.try_write_word(SELECTOR_WBA1, 0, 0x30F1));

        assert_eq!(ga.window_segment(), 0xF000);
        assert_eq!(ga.window_size(), Ga1280aWindowSize::K32);
    }

    #[test]
    fn ga1280_wba1_window_exposes_compatibility_and_hga_mapped_register_apertures() {
        let mut ga = Ga1280a::new();
        assert!(!ga.host_window_mapped_register_write_word(0x1F00 + 0x03 * 2, 0x0000));
        assert_eq!(ga.state.wpm, 0x0000);

        assert!(ga.try_write_word(SELECTOR_WBA1, 0, 0x30F1));
        assert!(ga.try_write_word(SELECTOR_WPM, 0, 0x00FF));

        assert!(ga.host_window_mapped_register_write_word(0x1F00 + 0x03 * 2, 0x0000));
        assert_eq!(ga.state.wpm, 0x0000);

        assert!(ga.try_write_word(SELECTOR_WPM, 0, 0x00FF));
        assert!(ga.host_window_mapped_register_write_word(0x7F00 + 0x03 * 2, 0x0000));
        assert_eq!(ga.state.wpm, 0x0000);

        assert!(ga.try_write_word(SELECTOR_WBA1, 0, 0x0000));
        assert!(ga.try_write_word(SELECTOR_WPM, 0, 0x00FF));

        assert!(ga.host_window_mapped_register_write_word(0x1F00 + 0x03 * 2, 0x0000));
        assert_eq!(ga.state.wpm, 0x0000);

        assert!(ga.try_write_word(SELECTOR_WPM, 0, 0x00FF));
        assert!(ga.host_window_mapped_register_write_word(0x7F00 + 0x03 * 2, 0x0000));
        assert_eq!(ga.state.wpm, 0x0000);

        assert!(ga.try_write_word(SELECTOR_WBA1, 0, 0x3F01));
        assert!(ga.try_write_word(SELECTOR_WPM, 0, 0x00FF));
        assert!(ga.host_window_mapped_register_write_word(0x7F00 + 0x03 * 2, 0x0000));
        assert_eq!(ga.state.wpm, 0x0000);
    }

    #[test]
    fn sys_registers_store_direct_guest_writes() {
        let mut ga = Ga1280a::new();
        assert!(ga.try_write_word(SELECTOR_SYSTEM_PDT, 0, 0x1234));
        assert!(ga.try_write_byte(SELECTOR_SYSTEM_PDT, 1, 0x56));
        assert_eq!(ga.state.system_register, 0x1234);
        assert_eq!(ga.state.system_auxiliary_register, 0x56);
    }

    #[test]
    fn crtc_index_and_data_are_stored_atomically() {
        let mut ga = Ga1280a::new();
        assert!(ga.try_write_byte(SELECTOR_CRTC_POP1, 0, 0x38));
        assert!(ga.try_write_word(SELECTOR_CRTC_POP2, 0, 0xCAFE));
        assert_eq!(ga.state.crtc_registers[0x38], 0xCAFE);
        assert_eq!(ga.state.pop2, 0);
    }

    #[test]
    fn crtc_address_word_write_stores_index_and_low_data_byte() {
        let mut ga = Ga1280a::new();
        assert!(ga.try_write_word(SELECTOR_CRTC_POP1, 0, 0xA531));
        assert_eq!(ga.state.crtc_index, 0x31);
        assert_eq!(ga.state.crtc_registers[0x31] & 0x00FF, 0x00A5);
        assert_eq!(ga.state.crtc_write_count, 1);
    }

    #[test]
    fn render_snapshot_uses_crtc30_32_display_start() {
        let mut ga = Ga1280a::new();
        ga.state.pmw = 0x03FF;
        ga.state.pmh = 0x03FF;

        // HGA16.DRV and HGA256.DRV write y * ((virtual_width + 1) / 4) + x / 4
        // as low, middle, and high bytes through CRTC indexes 30h, 31h, and 32h.
        set_display_start(&mut ga, (2 * 1024 + 8) / 4);
        assert_eq!(ga.render_snapshot().display_offset_pixels, 2 * 1024 + 8);

        set_display_start(&mut ga, 0x020000);
        assert_eq!(ga.render_snapshot().display_offset_pixels, 0x020000 * 4);

        // HGA64K.DRV uses y * 0200h + x / 2 in 65536-color mode.
        ga.state.plane_mode = Ga1280aPlaneMode::DirectColor16;
        assert_eq!(ga.render_snapshot().display_offset_pixels, 0x020000 * 2);
    }

    #[test]
    fn ramdac_palette_stream_stores_rgb_triplets() {
        let mut ga = Ga1280a::new();
        assert!(ga.try_write_byte(SELECTOR_VDAC_ARW_RS, 0, 7));
        assert!(ga.try_write_byte(SELECTOR_VDAC_CPR, 0, 0x11));
        assert!(ga.try_write_byte(SELECTOR_VDAC_CPR, 0, 0x22));
        assert!(ga.try_write_byte(SELECTOR_VDAC_CPR, 0, 0x33));
        assert_eq!(ga.state.palette[7], [0x11, 0x22, 0x33]);
        assert_eq!(ga.state.palette_index_write, 8);
    }

    #[test]
    fn command_block_word_registers_are_separate_from_base_registers() {
        let mut ga = Ga1280a::new();
        assert!(ga.try_write_word(SELECTOR_WBM, 0, 0x1111));
        assert!(ga.try_write_word(SELECTOR_WBM, 2, 0x2222));
        assert_eq!(ga.state.wbm, 0x1111);
        assert_eq!(ga.state.opd2, 0x2222);
    }

    #[test]
    fn reset_list_unknowns_are_accepted() {
        let mut ga = Ga1280a::new();
        assert!(ga.try_write_word(SELECTOR_UNKNOWN_0F, 0, 0x1111));
        assert!(ga.try_write_word(SELECTOR_MIX, 2, 0x2222));
        assert!(ga.try_write_byte(SELECTOR_CWB_UNKNOWN, 2, 0x33));
        assert_eq!(ga.state.unknown_sel_0f_off0, 0x1111);
        assert_eq!(ga.state.unknown_sel_14_off2, 0x2222);
        assert_eq!(ga.state.unknown_sel_15_off2, 0x33);
    }

    #[test]
    fn offset_three_reads_high_byte_without_advancing_id_stream() {
        let mut ga = Ga1280a::new();
        assert_eq!(ga.try_read_byte(SELECTOR_STATUS_SSV, 1), Some(b'.'));
        assert_eq!(ga.try_read_byte(SELECTOR_STATUS_SSV, 3), Some(0));
        assert_eq!(ga.state.id_stream_cursor, 1);
    }

    // The SDK sample rgbload.c wait() writes decimal 31, so it selects CRTC
    // register 1Fh and polls CRTC data mask 02h to count the GA CRTC VSYNC.
    #[test]
    fn crtc1f_bit2_reflects_vsync_active_state() {
        let mut ga = Ga1280a::new();
        ga.state.crtc_registers[CRTC_INDEX_VSYNC_STATUS] = 0x00A5;
        ga.state.crtc_index = CRTC_INDEX_VSYNC_STATUS as u8;

        assert!(!ga.state.vsync_active);
        let inactive = ga.try_read_byte(SELECTOR_CRTC_POP2, 0).unwrap();
        assert_eq!(inactive & 0x02, 0x00);
        assert_eq!(inactive & !0x02, 0xA5 & !0x02);

        ga.on_vsync_start();
        let active = ga.try_read_byte(SELECTOR_CRTC_POP2, 0).unwrap();
        assert_eq!(active & 0x02, 0x02);

        ga.on_display_start();
        let again_inactive = ga.try_read_byte(SELECTOR_CRTC_POP2, 0).unwrap();
        assert_eq!(again_inactive & 0x02, 0x00);
    }

    #[test]
    fn ga1280_crtc3f_high_byte_bit4_reflects_vsync_active_state() {
        let mut ga = Ga1280a::new();
        ga.state.crtc_registers[CRTC_INDEX_GA1280_VSYNC_STATUS] = 0x55AA;
        ga.state.crtc_index = CRTC_INDEX_GA1280_VSYNC_STATUS as u8;

        assert!(!ga.state.vsync_active);
        let inactive = ga.try_read_byte(SELECTOR_CRTC_POP2, 1).unwrap();
        assert_eq!(inactive & 0x04, 0x00);
        assert_eq!(inactive & !0x04, 0x55 & !0x04);

        ga.on_vsync_start();
        let active = ga.try_read_byte(SELECTOR_CRTC_POP2, 1).unwrap();
        assert_eq!(active & 0x04, 0x04);

        ga.on_display_start();
        let again_inactive = ga.try_read_word(SELECTOR_CRTC_POP2, 0).unwrap();
        assert_eq!(again_inactive & CRTC_BIT_GA1280_VSYNC_ACTIVE, 0x0000);
        assert_eq!(
            again_inactive & !CRTC_BIT_GA1280_VSYNC_ACTIVE,
            0x55AA & !CRTC_BIT_GA1280_VSYNC_ACTIVE
        );
    }

    #[test]
    fn crtc31_is_display_start_middle_byte_not_vsync_status() {
        let mut ga = Ga1280a::new();
        ga.state.crtc_registers[CRTC_INDEX_DISPLAY_START_MID] = 0x0000;
        ga.state.crtc_index = CRTC_INDEX_DISPLAY_START_MID as u8;

        ga.on_vsync_start();

        assert_eq!(ga.try_read_byte(SELECTOR_CRTC_POP2, 0), Some(0x00));
    }

    #[test]
    fn other_crtc_indices_return_stored_value_verbatim() {
        let mut ga = Ga1280a::new();
        ga.state.crtc_registers[0x10] = 0x0203;
        ga.state.crtc_index = 0x10;
        assert_eq!(ga.try_read_byte(SELECTOR_CRTC_POP2, 0), Some(0x03));
        assert_eq!(ga.try_read_word(SELECTOR_CRTC_POP2, 0), Some(0x0203));
    }

    #[test]
    fn refresh_rate_detection_matches_known_modes() {
        let mut ga = Ga1280a::new();
        assert_eq!(ga.detect_refresh_hz(), 60);

        ga.state.crtc_registers[CRTC_INDEX_HORIZONTAL_TOTAL] = 0x0063;
        ga.state.crtc_registers[CRTC_INDEX_VERTICAL_TOTAL] = 0x020B;
        assert_eq!(ga.detect_refresh_hz(), 60);

        ga.state.crtc_registers[CRTC_INDEX_HORIZONTAL_TOTAL] = 0x007F;
        ga.state.crtc_registers[CRTC_INDEX_VERTICAL_TOTAL] = 0x026F;
        assert_eq!(ga.detect_refresh_hz(), 56);

        ga.state.crtc_registers[CRTC_INDEX_HORIZONTAL_TOTAL] = 0x00A7;
        ga.state.crtc_registers[CRTC_INDEX_VERTICAL_TOTAL] = 0x0324;
        assert_eq!(ga.detect_refresh_hz(), 60);

        ga.state.crtc_registers[CRTC_INDEX_HORIZONTAL_TOTAL] = 0xFFFF;
        ga.state.crtc_registers[CRTC_INDEX_VERTICAL_TOTAL] = 0xFFFF;
        assert_eq!(ga.detect_refresh_hz(), 60);
    }

    #[test]
    fn period_split_sums_to_frame_cycles() {
        let mut ga = Ga1280a::new();
        ga.state.crtc_registers[CRTC_INDEX_HORIZONTAL_TOTAL] = 0x0063;
        ga.state.crtc_registers[CRTC_INDEX_VERTICAL_TOTAL] = 0x020B;
        ga.state.crtc_registers[CRTC_INDEX_VERTICAL_DISPLAY_END] = 0x01DF;

        let cpu_clock_hz: u32 = 10_000_000;
        let display = ga.display_period_cycles(cpu_clock_hz);
        let blanking = ga.blanking_period_cycles(cpu_clock_hz);
        let frame_cycles = u64::from(cpu_clock_hz) / 60;
        assert_eq!(display + blanking, frame_cycles);

        // 480 active / 524 total lines for mode 02.
        assert_eq!(display, frame_cycles * 480 / 524);
    }

    #[test]
    fn render_snapshot_reflects_active_dimensions() {
        let mut ga = Ga1280a::new();
        assert_eq!(ga.dimensions(), (DEFAULT_WIDTH, DEFAULT_HEIGHT));

        ga.state.active_width = 1024;
        ga.state.active_height = 768;

        let snapshot = ga.render_snapshot();
        assert_eq!(snapshot.width, 1024);
        assert_eq!(snapshot.height, 768);
        assert_eq!((ga.state.active_width, ga.state.active_height), (1024, 768));
    }
}
