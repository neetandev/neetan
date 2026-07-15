//! FM Towns CRTC, video-out control, and palette register file.
//!
//! Holds the 32 CRTC registers, the four-byte video-out "sifter" (single-page
//! select, per-page show bits, priority, palette select), the analog 16/256-color
//! palettes and the FMR digital palette, and derives the display geometry that
//! the software renderer consumes.

use common::{HighResCursor, TownsLayer};

/// CRTC register indices (I/O 0x0440 selects one, 0x0442/0x0443 read/write it).
const REG_HST: usize = 0x04;
const REG_VST: usize = 0x08;
const REG_HDS0: usize = 0x09;
const REG_HDE0: usize = 0x0A;
const REG_HDS1: usize = 0x0B;
const REG_VDS0: usize = 0x0D;
const REG_VDE0: usize = 0x0E;
const REG_VDS1: usize = 0x0F;
const REG_VDE1: usize = 0x10;
const REG_FA0: usize = 0x11;
const REG_HAJ0: usize = 0x12;
const REG_FO0: usize = 0x13;
const REG_LO0: usize = 0x14;
const REG_ZOOM: usize = 0x1B;
const REG_CR0: usize = 0x1C;
const REG_CR1: usize = 0x1D;
const REG_FR: usize = 0x1E;

/// The four CRTC dot-clock frequencies selected by CR1 bits 0-1, in Hz.
const DOT_CLOCK_HZ: [u32; 4] = [28_636_300, 24_545_400, 25_175_000, 21_052_500];

/// HST values that pair with CLKSEL=3 in the VING 31 kHz zoom quirk.
const VING_HST_A: u16 = 0x029D;
const VING_HST_B: u16 = 0x02BD;

/// Power-on CRTC register defaults (640x480-ish two-page 16-color).
const DEFAULT_CRTC: [u16; 32] = [
    0x0040, 0x0320, 0x0000, 0x0000, 0x035F, 0x0000, 0x0010, 0x0000, 0x036F, 0x009C, 0x031C, 0x009C,
    0x031C, 0x0040, 0x0360, 0x0040, 0x0360, 0x0000, 0x009C, 0x0000, 0x0050, 0x0000, 0x009C, 0x0000,
    0x0050, 0x004A, 0x0001, 0x0000, 0x003F, 0x0003, 0x0000, 0x0150,
];

/// Power-on video-out ("sifter") defaults.
const DEFAULT_SIFTER: [u8; 4] = [0x15, 0x08, 0, 0];

/// Size of the MX high-resolution CRTC ("image out") register file. Each entry
/// is a 32-bit register accessed indirectly through the 0x0472 index latch and
/// the 0x0474-0x0477 data lanes. Mouse-control sub-registers (0x200-0x209) sit
/// above this range and are handled as side effects, not stored here.
const HIGH_RES_REG_COUNT: usize = 512;

/// High-res CRTC global sub-register indices.
const HR_CTRL0: usize = 0x000;
const HR_PGCTRL: usize = 0x001;
const HR_CTRL1: usize = 0x004;
const HR_DISPPAGE: usize = 0x005;
const HR_VSYNC1: usize = 0x006;
const HR_XSTART: usize = 0x104;
const HR_XEND: usize = 0x105;
const HR_YSTART: usize = 0x106;
const HR_YEND: usize = 0x107;
const HR_PALSEL: usize = 0x130;
const HR_PALINDEX: usize = 0x132;
const HR_PALCOL: usize = 0x133;

/// High-res per-page sub-register indices, keyed by page (0 or 1).
const HR_PAGE_VRAM_WIDTH: [usize; 2] = [0x114, 0x124];
const HR_PAGE_VRAM_OFFSET_X: [usize; 2] = [0x116, 0x126];
const HR_PAGE_VRAM_OFFSET_Y: [usize; 2] = [0x117, 0x127];
const HR_PAGE_ZOOM: [usize; 2] = [0x119, 0x129];
const HR_PAGE_RGB_SWAP: [usize; 2] = [0x11A, 0x12A];
const HR_PAGE_PALETTE: [usize; 2] = [0x11B, 0x12B];

/// High-res hardware mouse-cursor sub-register indices.
const HR_MOUSE_X: usize = 0x200;
const HR_MOUSE_Y: usize = 0x201;
const HR_MOUSE_ORIGIN_X: usize = 0x202;
const HR_MOUSE_ORIGIN_Y: usize = 0x203;
const HR_MOUSE_DEFINE: usize = 0x206;
const HR_MOUSE_PATTERN: usize = 0x209;

/// One VRAM page is 512 KiB in the high-res layout (pages sit 0x80000 apart,
/// unlike the 0x40000-spaced low-res pages).
const HR_PAGE_STRIDE: usize = 0x0008_0000;
/// V-scroll wrap mask covering all 1 MiB of VRAM (single-page high-res).
const HR_V_SCROLL_MASK_FULL: usize = 0x000F_FFFF;
/// V-scroll wrap mask covering one 512 KiB page (two-page high-res).
const HR_V_SCROLL_MASK_PAGE: usize = 0x0007_FFFF;

/// Resolved CRTC geometry and palettes for a frame, everything the renderer
/// needs except the VRAM borrow (supplied by the bus).
pub struct ResolvedVideo {
    /// Whether one layer spans all VRAM.
    pub single_page: bool,
    /// Front layer in two-page mode.
    pub priority_page: usize,
    /// Resolved display layers.
    pub layers: [TownsLayer; 2],
    /// Converted 16-color palettes.
    pub palette_16: [[u32; 16]; 2],
    /// Converted 256-color palette.
    pub palette_256: [u32; 256],
    /// Display width.
    pub width: u32,
    /// Display height.
    pub height: u32,
    /// Whether the high-resolution CRTC is driving this frame (selects the
    /// high-res VRAM interleave and enables the hardware mouse-cursor overlay).
    pub high_res: bool,
    /// The hardware mouse-cursor overlay, present only when defined in high-res.
    pub mouse_cursor: Option<HighResCursor>,
}

/// FM Towns MX high-resolution hardware mouse cursor: a 64x64 two-plane sprite.
#[derive(Clone, Copy)]
struct TownsMouseCursor {
    x: u32,
    y: u32,
    origin_x: u32,
    origin_y: u32,
    defining: bool,
    defined: bool,
    pattern_count: u32,
    and_pattern: [u8; 512],
    or_pattern: [u8; 512],
}

impl Default for TownsMouseCursor {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            origin_x: 0,
            origin_y: 0,
            defining: false,
            defined: false,
            pattern_count: 0,
            and_pattern: [0; 512],
            or_pattern: [0; 512],
        }
    }
}

/// An analog palette color (8-bit components; 16-color entries carry 4-bit
/// precision replicated into the high nibble).
#[derive(Clone, Copy, Default)]
struct TownsColor {
    red: u8,
    green: u8,
    blue: u8,
}

impl TownsColor {
    fn to_rgba(self) -> u32 {
        towns_color_to_rgba(self.red, self.green, self.blue)
    }
}

fn towns_color_to_rgba(red: u8, green: u8, blue: u8) -> u32 {
    u32::from(red) | (u32::from(green) << 8) | (u32::from(blue) << 16) | 0xFF00_0000
}

/// FM Towns CRTC, video-out, and palette register file.
pub struct TownsVideo {
    crtc_registers: [u16; 32],
    crtc_addr_latch: usize,
    sifter: [u8; 4],
    sifter_addr_latch: usize,
    show_page_0448: [bool; 2],
    show_page_fda0: [bool; 2],
    palette_code_latch: u8,
    palette_16: [[TownsColor; 16]; 2],
    palette_256: [TownsColor; 256],
    fmr_digital_palette: [u8; 8],
    dpmd: bool,
    vsync_irq: bool,
    vsync_active: bool,
    /// Whether this machine carries the MX high-resolution CRTC.
    high_res_available: bool,
    /// Whether the high-resolution CRTC is currently enabled (HR_CTRL0 bit 0).
    high_res_enabled: bool,
    /// The high-resolution "image out" register file (0x0472 indexes it).
    high_res_reg: Box<[u32; HIGH_RES_REG_COUNT]>,
    high_res_addr_latch: u16,
    high_res_reg4_bit0: bool,
    high_res_reg4_bit1: bool,
    /// The high-res CRTC's own palette banks (separate from the low-res ones).
    high_res_palette_16: [[TownsColor; 16]; 2],
    high_res_palette_256: [TownsColor; 256],
    high_res_palette_code_latch: u8,
    high_res_mouse: TownsMouseCursor,
}

impl TownsVideo {
    /// Creates the video controller for a machine with the selected capability.
    pub fn new(high_res_available: bool) -> Self {
        Self {
            crtc_registers: DEFAULT_CRTC,
            crtc_addr_latch: 0,
            sifter: DEFAULT_SIFTER,
            sifter_addr_latch: 0,
            show_page_0448: [true, false],
            show_page_fda0: [true, true],
            palette_code_latch: 0,
            palette_16: [[TownsColor::default(); 16]; 2],
            palette_256: [TownsColor {
                red: 0xFF,
                green: 0xFF,
                blue: 0xFF,
            }; 256],
            fmr_digital_palette: [0, 1, 2, 3, 4, 5, 6, 7],
            dpmd: false,
            vsync_irq: false,
            vsync_active: false,
            high_res_available,
            high_res_enabled: false,
            high_res_reg: Box::new([0; HIGH_RES_REG_COUNT]),
            high_res_addr_latch: 0,
            high_res_reg4_bit0: false,
            high_res_reg4_bit1: true,
            high_res_palette_16: [[TownsColor::default(); 16]; 2],
            high_res_palette_256: [TownsColor::default(); 256],
            high_res_palette_code_latch: 0,
            high_res_mouse: TownsMouseCursor::default(),
        }
    }

    /// Writes the standard CRTC address latch.
    pub fn write_crtc_address(&mut self, value: u8) {
        self.crtc_addr_latch = usize::from(value & 0x1F);
    }

    /// Reads the standard CRTC address latch.
    pub fn read_crtc_address(&self) -> u8 {
        self.crtc_addr_latch as u8
    }

    /// Writes the low byte of the selected standard CRTC register.
    pub fn write_crtc_data_low(&mut self, value: u8) {
        let register = &mut self.crtc_registers[self.crtc_addr_latch];
        *register = (*register & 0xFF00) | u16::from(value);
        if self.crtc_addr_latch == REG_CR0 {
            self.on_cr0_write();
        }
    }

    /// Writes the high byte of the selected standard CRTC register.
    pub fn write_crtc_data_high(&mut self, value: u8) {
        let register = &mut self.crtc_registers[self.crtc_addr_latch];
        *register = (*register & 0x00FF) | (u16::from(value) << 8);
        if self.crtc_addr_latch == REG_CR0 {
            self.on_cr0_write();
        }
    }

    /// A write to the standard CRTC CR0 register updates the high-res status
    /// bits (the actual high-res enable is driven only by HR_CTRL0). The START
    /// bit (CR0 bit 15) cleared selects the high-res CRTC.
    fn on_cr0_write(&mut self) {
        if self.high_res_available {
            self.high_res_reg4_bit1 = true;
            self.high_res_reg4_bit0 = self.crtc_registers[REG_CR0] & 0x8000 == 0;
        }
    }

    /// Reads the low byte of the selected standard CRTC register.
    pub fn read_crtc_data_low(&self) -> u8 {
        self.crtc_registers[self.crtc_addr_latch] as u8
    }

    /// Reads the high byte or live field status of the selected CRTC register.
    pub fn read_crtc_data_high(&self, hsync: bool, vertical_display: (bool, bool)) -> u8 {
        if self.crtc_addr_latch == REG_FR {
            self.field_status(hsync, vertical_display)
        } else {
            (self.crtc_registers[self.crtc_addr_latch] >> 8) as u8
        }
    }

    /// The CRTC field/status register (read at 0x0443 when the FR index is
    /// latched): sync flags and display-timing bits. Raster-effect code polls
    /// the DSPTH bits to synchronize with the per-scanline blanking edge, so
    /// `hsync` carries the caller's current horizontal-sync phase.
    /// `vertical_display` carries the per-layer vertical display state, which
    /// spans the VDS..VDE raster window and is low for the whole vertical
    /// blanking region, not just the short vertical-sync pulse.
    fn field_status(&self, hsync: bool, vertical_display: (bool, bool)) -> u8 {
        let vsync = self.vsync_active;
        let hsync = hsync && !vsync;
        let display_horizontal = !hsync;
        let mut data = 0u8;
        if hsync {
            data |= 0x02;
        }
        if vsync {
            data |= 0x04;
        }
        if display_horizontal {
            data |= 0x10 | 0x20;
        }
        if vertical_display.0 {
            data |= 0x40;
        }
        if vertical_display.1 {
            data |= 0x80;
        }
        data
    }

    /// The per-layer vertical display state at `into_frame` cycles past the
    /// vertical-sync start. The frame spans VST half-rasters; each layer
    /// displays while the raster position lies inside its VDS..VDE window.
    pub fn vertical_display_active(&self, into_frame: u64, frame_cycles: u64) -> (bool, bool) {
        let vertical_total = u64::from(self.crtc_registers[REG_VST].max(1));
        let position = (into_frame.min(frame_cycles) * vertical_total / frame_cycles.max(1)) as u16;
        let in_window = |start: u16, end: u16| start < end && (start..end).contains(&position);
        (
            in_window(self.crtc_registers[REG_VDS0], self.crtc_registers[REG_VDE0]),
            in_window(self.crtc_registers[REG_VDS1], self.crtc_registers[REG_VDE1]),
        )
    }

    /// Writes the video-out address latch.
    pub fn write_video_out_address(&mut self, value: u8) {
        self.sifter_addr_latch = usize::from(value & 3);
    }

    /// Reads the video-out address latch.
    pub fn read_video_out_address(&self) -> u8 {
        self.sifter_addr_latch as u8
    }

    /// Writes the selected video-out register.
    pub fn write_video_out_data(&mut self, value: u8) {
        self.sifter[self.sifter_addr_latch] = value;
        if self.sifter_addr_latch == 0 {
            if self.single_page() {
                self.show_page_0448[0] = value & 0x08 != 0;
                self.show_page_0448[1] = false;
            } else {
                self.show_page_0448[0] = value & 0x01 != 0;
                self.show_page_0448[1] = value & 0x04 != 0;
            }
        }
    }

    /// Reads the selected video-out register.
    pub fn read_video_out_data(&self) -> u8 {
        self.sifter[self.sifter_addr_latch]
    }

    /// Reads the DPMD / sprite-status register (0x044C). The DPMD flag
    /// self-clears on read; sprite status is added in the sprite phase.
    pub fn read_dpmd(&mut self) -> u8 {
        let data = if self.dpmd { 0x80 } else { 0x00 };
        self.dpmd = false;
        data
    }

    /// Writes the analog palette entry latch.
    pub fn write_palette_code(&mut self, value: u8) {
        self.palette_code_latch = value;
    }

    /// Reads the analog palette entry latch.
    pub fn read_palette_code(&self) -> u8 {
        self.palette_code_latch
    }

    fn read_color16_component(&self, component: impl Fn(&TownsColor) -> u8) -> u8 {
        let index = usize::from(self.palette_code_latch & 0x0F);
        match self.palette_select() {
            0 => component(&self.palette_16[0][index]) & 0xF0,
            2 => component(&self.palette_16[1][index]) & 0xF0,
            _ => component(&self.palette_256[usize::from(self.palette_code_latch)]),
        }
    }

    /// Reads the blue component of the selected analog palette entry.
    pub fn read_palette_blue(&self) -> u8 {
        self.read_color16_component(|color| color.blue)
    }

    /// Reads the red component of the selected analog palette entry.
    pub fn read_palette_red(&self) -> u8 {
        self.read_color16_component(|color| color.red)
    }

    /// Reads the green component of the selected analog palette entry.
    pub fn read_palette_green(&self) -> u8 {
        self.read_color16_component(|color| color.green)
    }

    /// Writes the blue component of the selected analog palette entry.
    pub fn write_palette_blue(&mut self, value: u8) {
        match self.palette_select() {
            0 => self.set_color16(0, |color| color.blue = quantize4(value)),
            2 => self.set_color16(1, |color| color.blue = quantize4(value)),
            _ => self.palette_256[usize::from(self.palette_code_latch)].blue = value,
        }
    }

    /// Writes the red component of the selected analog palette entry.
    pub fn write_palette_red(&mut self, value: u8) {
        match self.palette_select() {
            0 => self.set_color16(0, |color| color.red = quantize4(value)),
            2 => self.set_color16(1, |color| color.red = quantize4(value)),
            _ => self.palette_256[usize::from(self.palette_code_latch)].red = value,
        }
    }

    /// Writes the green component of the selected analog palette entry.
    pub fn write_palette_green(&mut self, value: u8) {
        match self.palette_select() {
            0 => self.set_color16(0, |color| color.green = quantize4(value)),
            2 => self.set_color16(1, |color| color.green = quantize4(value)),
            _ => self.palette_256[usize::from(self.palette_code_latch)].green = value,
        }
    }

    /// Writes an FMR digital palette entry.
    pub fn write_digital_palette(&mut self, index: usize, value: u8) {
        self.fmr_digital_palette[index & 7] = value & 0x0F;
        self.dpmd = true;
    }

    /// Reads an FMR digital palette entry.
    pub fn read_digital_palette(&self, index: usize) -> u8 {
        self.fmr_digital_palette[index & 7]
    }

    /// Writes the FMR page visibility register.
    pub fn write_show_page_fda0(&mut self, value: u8) {
        if self.single_page() {
            self.show_page_fda0[0] = (value >> 2) & 3 != 0;
            self.show_page_fda0[1] = self.show_page_fda0[0];
        } else {
            self.show_page_fda0[0] = (value >> 2) & 3 != 0;
            self.show_page_fda0[1] = value & 3 != 0;
        }
    }

    /// Clears the pending VSYNC interrupt (write to 0x05CA).
    pub fn clear_vsync_irq(&mut self) {
        self.vsync_irq = false;
    }

    /// Whether the VSYNC interrupt latch is currently asserted.
    pub fn vsync_irq_pending(&self) -> bool {
        self.vsync_irq
    }

    /// Raises the VSYNC interrupt latch and marks the vertical-sync interval.
    pub fn enter_vsync(&mut self) {
        self.vsync_irq = true;
        self.vsync_active = true;
    }

    /// Leaves the vertical-sync interval.
    pub fn leave_vsync(&mut self) {
        self.vsync_active = false;
    }

    /// The number of CPU cycles in one display frame for VSYNC scheduling.
    pub fn frame_cycles(&self, cpu_clock_hz: u32) -> u64 {
        let refresh_hz = self.refresh_rate_hz().max(1.0);
        (f64::from(cpu_clock_hz) / refresh_hz) as u64
    }

    /// Approximate vertical refresh rate from the CRTC totals.
    fn refresh_rate_hz(&self) -> f64 {
        let clock = f64::from(DOT_CLOCK_HZ[self.clksel()]);
        let horizontal_total = f64::from(self.crtc_registers[REG_HST].max(1));
        let vertical_total = f64::from(self.crtc_registers[REG_VST].max(1));
        // VST counts half-lines, so the frame spans VST/2 scanlines.
        let denominator = horizontal_total * vertical_total / 2.0;
        if denominator > 0.0 {
            clock / denominator
        } else {
            60.0
        }
    }

    fn palette_select(&self) -> u8 {
        (self.sifter[1] >> 4) & 3
    }

    fn set_color16(&mut self, bank: usize, apply: impl FnOnce(&mut TownsColor)) {
        apply(&mut self.palette_16[bank][usize::from(self.palette_code_latch & 0x0F)]);
    }

    fn clksel(&self) -> usize {
        usize::from(self.crtc_registers[REG_CR1] & 3)
    }

    fn horizontal_frequency_khz(&self) -> u32 {
        let clock = DOT_CLOCK_HZ[self.clksel()];
        let horizontal_total = self.crtc_registers[REG_HST];
        if horizontal_total > 0 {
            (clock / u32::from(horizontal_total)) / 1000
        } else {
            31
        }
    }

    fn single_page(&self) -> bool {
        self.sifter[0] & 0x10 == 0
    }

    fn priority_page(&self) -> usize {
        usize::from(self.sifter[1] & 1)
    }

    fn ving_quirk(&self) -> bool {
        self.clksel() == 3
            && (self.crtc_registers[REG_HST] == VING_HST_A
                || self.crtc_registers[REG_HST] == VING_HST_B)
    }

    fn page_shown(&self, page: usize) -> bool {
        self.show_page_fda0[page] && self.show_page_0448[page]
    }

    // TODO: MX 1024x768 high-res CRTC (I/O 0x0470-0x0477 register file) and its
    //       24bpp mode are not modeled; only the low-res CRTC path is implemented.
    fn page_bits_per_pixel(&self, page: usize) -> u8 {
        let color = (self.crtc_registers[REG_CR0] >> (page * 2)) & 3;
        if self.single_page() {
            match color {
                2 => 16,
                3 => 8,
                _ => 4,
            }
        } else {
            match color {
                1 => 16,
                _ => 4,
            }
        }
    }

    fn page_zoom2x(&self, page: usize) -> (u8, u8) {
        let page_zoom = self.crtc_registers[REG_ZOOM] >> (8 * page);
        let mut zoom_x = u32::from(page_zoom & 15) + 1;
        let mut zoom_y = u32::from((page_zoom >> 4) & 15) + 1;

        if self.horizontal_frequency_khz() == 15 {
            if self.single_page() {
                let field_offset = self.crtc_registers[REG_FO0 + 4 * page];
                let line_offset = self.crtc_registers[REG_LO0 + 4 * page];
                if field_offset == 0 || field_offset == line_offset {
                    zoom_y *= 4;
                } else {
                    zoom_y *= 2;
                }
            } else {
                zoom_y *= 4;
            }
        } else if self.ving_quirk() {
            zoom_x = 2 + 3 * u32::from(page_zoom & 15);
            zoom_y *= 2;
        } else {
            zoom_x *= 2;
            zoom_y *= 2;
        }

        (zoom_x.min(255) as u8, zoom_y.min(255) as u8)
    }

    fn page_size(&self, page: usize) -> (usize, usize) {
        let horizontal_end = self.crtc_registers[REG_HDE0 + page * 2];
        let horizontal_start = self.crtc_registers[REG_HDS0 + page * 2];
        let vertical_end = self.crtc_registers[REG_VDE0 + page * 2];
        let vertical_start = self.crtc_registers[REG_VDS0 + page * 2];
        let mut width = u32::from(horizontal_end.saturating_sub(horizontal_start));
        let mut height = u32::from(vertical_end.saturating_sub(vertical_start));

        if self.horizontal_frequency_khz() == 15 {
            width /= 2;
            height *= 2;
        } else if self.ving_quirk() {
            let (zoom_x, _) = self.page_zoom2x(page);
            if zoom_x >= 5 {
                width = width * u32::from(zoom_x) / 4;
            }
        }

        let field_offset = self.crtc_registers[REG_FO0 + 4 * page];
        let line_offset = self.crtc_registers[REG_LO0 + 4 * page];
        if field_offset == 0 || field_offset == line_offset {
            height /= 2;
        }

        (width.min(800) as usize, height as usize)
    }

    fn page_origin(&self, page: usize) -> (usize, usize) {
        let horizontal_start =
            self.crtc_registers[REG_HDS0 + page * 2].max(self.crtc_registers[REG_HAJ0 + page * 4]);
        let vertical_start = self.crtc_registers[REG_VDS0 + page * 2];

        let mut std_horizontal = match self.crtc_registers[REG_HST] {
            779 => 127,
            863 => 156,
            895 => 160,
            1559 => 231,
            1819 => 297,
            _ => 138,
        };
        let horizontal_start_min = self.crtc_registers[REG_HDS0].min(self.crtc_registers[REG_HDS1]);
        std_horizontal = std_horizontal.min(horizontal_start_min);

        let mut std_vertical = match self.crtc_registers[REG_VST] {
            523 => 40,
            524 => 42,
            879 => 64,
            _ => 70,
        };
        let vertical_start_min = self.crtc_registers[REG_VDS0].min(self.crtc_registers[REG_VDS1]);
        std_vertical = std_vertical.min(vertical_start_min);

        let mut origin_x = i32::from(horizontal_start) - i32::from(std_horizontal);
        let mut origin_y = (i32::from(vertical_start) - i32::from(std_vertical)) >> 1;

        if self.horizontal_frequency_khz() == 15 {
            origin_x >>= 1;
            origin_y <<= 1;
        }

        (origin_x.max(0) as usize, origin_y.max(0) as usize)
    }

    /// The visible scan height in monitor lines, derived from the vertical
    /// total the same way `page_origin` derives the top border. Content that
    /// ends above this line gets its bottom letterbox bar.
    fn scan_height(&self) -> usize {
        let mut std_vertical = match self.crtc_registers[REG_VST] {
            523 => 40,
            524 => 42,
            879 => 64,
            _ => 70,
        };
        let vertical_start_min = self.crtc_registers[REG_VDS0].min(self.crtc_registers[REG_VDS1]);
        std_vertical = std_vertical.min(vertical_start_min);

        let mut height = i32::from(self.crtc_registers[REG_VST].saturating_sub(std_vertical)) >> 1;
        if self.horizontal_frequency_khz() == 15 {
            height <<= 1;
        }
        height.max(0) as usize
    }

    fn page_bytes_per_line(&self, page: usize) -> usize {
        let mut bytes = usize::from(self.crtc_registers[REG_LO0 + page * 4]) * 4;
        if self.single_page() {
            bytes *= 2;
        }
        bytes
    }

    fn page_scroll_offset(&self, page: usize) -> usize {
        let frame_address = usize::from(self.crtc_registers[REG_FA0 + page * 4]);
        match self.page_bits_per_pixel(page) {
            4 => frame_address * 4,
            8 => frame_address * 8,
            16 => {
                if self.single_page() {
                    frame_address * 8
                } else {
                    frame_address * 4
                }
            }
            _ => 0,
        }
    }

    fn page_h_scroll_mask(&self, page: usize) -> usize {
        let bytes_per_line = self.page_bytes_per_line(page);
        if bytes_per_line.is_power_of_two() {
            bytes_per_line - 1
        } else {
            usize::MAX
        }
    }

    fn page_v_scroll_mask(&self, page: usize) -> usize {
        if self.single_page() && page == 0 {
            0x0007_FFFF
        } else {
            0x0003_FFFF
        }
    }

    fn page_vram_h_skip_bytes(&self, page: usize) -> usize {
        let horizontal_adjust = self.crtc_registers[REG_HAJ0 + page * 4];
        let horizontal_start = self.crtc_registers[REG_HDS0 + page * 2];
        let skip = usize::from(horizontal_start.saturating_sub(horizontal_adjust));
        let raw_zoom = usize::from((self.crtc_registers[REG_ZOOM] >> (8 * page)) & 15) + 1;
        ((skip / raw_zoom) * usize::from(self.page_bits_per_pixel(page))) >> 3
    }

    /// Whether the current screen mode accepts sprites: two-page mode with page
    /// 1 in 16 bpp direct color at 512 bytes per line.
    pub fn screen_mode_accepts_sprite(&self) -> bool {
        !self.single_page()
            && self.page_bits_per_pixel(1) == 16
            && self.page_bytes_per_line(1) == 512
    }

    fn build_layer(
        &self,
        page: usize,
        fmr_display_planes: u8,
        fmr_display_page_offset: usize,
        sprite_display_offset: usize,
    ) -> TownsLayer {
        let (width, height) = self.page_size(page);
        let (origin_x, origin_y) = self.page_origin(page);
        let (zoom_x, zoom_y) = self.page_zoom2x(page);
        // Layer 0 carries the FMR display page offset; layer 1 carries the
        // sprite engine's displayed double-buffer half.
        let page_scroll = self.page_scroll_offset(page)
            + if page == 0 {
                fmr_display_page_offset
            } else {
                sprite_display_offset
            };
        TownsLayer {
            shown: self.page_shown(page),
            bits_per_pixel: self.page_bits_per_pixel(page),
            vram_addr: 0x0004_0000 * page,
            bytes_per_line: self.page_bytes_per_line(page),
            scroll_offset: page_scroll,
            h_scroll_mask: self.page_h_scroll_mask(page),
            v_scroll_mask: self.page_v_scroll_mask(page),
            vram_h_skip_bytes: self.page_vram_h_skip_bytes(page),
            width,
            height,
            origin_x,
            origin_y,
            zoom_x,
            zoom_y,
            plane_mask: if page == 0 {
                fmr_display_planes & 0x0F
            } else {
                0x0F
            },
            palette_bank: page as u8,
            high_res_rgb_swap: 0,
        }
    }

    fn palette_16_rgba(&self) -> [[u32; 16]; 2] {
        let mut output = [[0u32; 16]; 2];
        for (bank, colors) in self.palette_16.iter().enumerate() {
            for (index, color) in colors.iter().enumerate() {
                output[bank][index] = color.to_rgba();
            }
        }
        output
    }

    fn palette_256_rgba(&self) -> [u32; 256] {
        let mut output = [0u32; 256];
        for (index, color) in self.palette_256.iter().enumerate() {
            output[index] = color.to_rgba();
        }
        output
    }

    /// Reads the high-res presence port (0x0470): 0x7F when the high-res CRTC is
    /// available (MX), 0x80 otherwise.
    pub fn read_high_res_id(&self) -> u8 {
        if self.high_res_available { 0x7F } else { 0x80 }
    }

    /// Reads the VRAM-size port (0x0471): 0x01 on the high-res-capable MX.
    pub fn read_vram_size(&self) -> u8 {
        if self.high_res_available { 0x01 } else { 0x00 }
    }

    /// Writes the low byte of the high-resolution register latch.
    pub fn write_high_res_addr_low(&mut self, value: u8) {
        if self.high_res_available {
            self.high_res_addr_latch = (self.high_res_addr_latch & 0xFF00) | u16::from(value);
        }
    }

    /// Writes the high byte of the high-resolution register latch.
    pub fn write_high_res_addr_high(&mut self, value: u8) {
        if self.high_res_available {
            self.high_res_addr_latch =
                (self.high_res_addr_latch & 0x00FF) | (u16::from(value) << 8);
        }
    }

    /// Latches the full 16-bit high-res register index (0x0472 word access).
    pub fn write_high_res_addr_word(&mut self, value: u16) {
        if self.high_res_available {
            self.high_res_addr_latch = value;
        }
    }

    /// Reads the low byte of the high-resolution register latch.
    pub fn read_high_res_addr_low(&self) -> u8 {
        if self.high_res_available {
            self.high_res_addr_latch as u8
        } else {
            0xFF
        }
    }

    /// Reads the high byte of the high-resolution register latch.
    pub fn read_high_res_addr_high(&self) -> u8 {
        if self.high_res_available {
            (self.high_res_addr_latch >> 8) as u8
        } else {
            0xFF
        }
    }

    /// Writes one byte lane (0-3) of the selected high-res register (data ports
    /// 0x0474-0x0477) and applies the per-lane side effects.
    pub fn write_high_res_data(&mut self, lane: u8, value: u8) {
        if !self.high_res_available {
            return;
        }
        let index = usize::from(self.high_res_addr_latch);
        if index < HIGH_RES_REG_COUNT {
            let shift = u32::from(lane) * 8;
            let register = &mut self.high_res_reg[index];
            *register = (*register & !(0xFFu32 << shift)) | (u32::from(value) << shift);
        }
        match lane {
            0 => self.write_high_res_lane0(index, value),
            1 => self.write_high_res_palette_byte(index, 0, value),
            2 => self.write_high_res_palette_byte(index, 1, value),
            _ => {}
        }
    }

    fn write_high_res_lane0(&mut self, index: usize, value: u8) {
        match index {
            HR_CTRL0 => self.high_res_enabled = value & 1 != 0,
            HR_CTRL1 => {
                if value & 2 != 0 {
                    self.high_res_reg4_bit0 = false;
                    self.high_res_reg4_bit1 = false;
                }
            }
            HR_PALINDEX => self.high_res_palette_code_latch = value,
            HR_PALCOL => self.write_high_res_palette_byte(HR_PALCOL, 2, value),
            HR_MOUSE_PATTERN => self.push_mouse_pattern(value),
            _ => {}
        }
    }

    /// Low 16 bits of the selected register (0x0474 word access, lanes 0-1).
    pub fn write_high_res_data_low_word(&mut self, value: u16) {
        if !self.high_res_available {
            return;
        }
        let index = usize::from(self.high_res_addr_latch);
        if index < HIGH_RES_REG_COUNT {
            let register = &mut self.high_res_reg[index];
            *register = (*register & 0xFFFF_0000) | u32::from(value);
        }
        match index {
            HR_CTRL0 => self.high_res_enabled = value & 1 != 0,
            HR_CTRL1 => {
                if value & 2 != 0 {
                    self.high_res_reg4_bit0 = false;
                    self.high_res_reg4_bit1 = false;
                }
            }
            HR_PALINDEX => self.high_res_palette_code_latch = value as u8,
            HR_PALCOL => {
                self.write_high_res_palette_byte(HR_PALCOL, 2, value as u8);
                self.write_high_res_palette_byte(HR_PALCOL, 0, (value >> 8) as u8);
            }
            HR_MOUSE_X => self.high_res_mouse.x = u32::from(value),
            HR_MOUSE_Y => self.high_res_mouse.y = u32::from(value),
            HR_MOUSE_ORIGIN_X => self.high_res_mouse.origin_x = u32::from(value),
            HR_MOUSE_ORIGIN_Y => self.high_res_mouse.origin_y = u32::from(value),
            HR_MOUSE_DEFINE => self.set_mouse_define(value),
            _ => {}
        }
    }

    /// High 16 bits of the selected register (0x0476 word access, lanes 2-3).
    /// A PALCOL write here advances the palette index, matching the hardware's
    /// auto-increment on the completing 32-bit access.
    pub fn write_high_res_data_high_word(&mut self, value: u16) {
        if !self.high_res_available {
            return;
        }
        let index = usize::from(self.high_res_addr_latch);
        if index < HIGH_RES_REG_COUNT {
            let register = &mut self.high_res_reg[index];
            *register = (*register & 0x0000_FFFF) | (u32::from(value) << 16);
        }
        if index == HR_PALCOL {
            self.write_high_res_palette_byte(HR_PALCOL, 1, value as u8);
            self.high_res_palette_code_latch = self.high_res_palette_code_latch.wrapping_add(1);
        }
    }

    fn write_high_res_palette_byte(&mut self, index: usize, component: usize, value: u8) {
        if index != HR_PALCOL {
            return;
        }
        match self.high_res_reg[HR_PALSEL] {
            0 => self.set_high_res_16(0, component, value),
            1 => self.set_high_res_16(1, component, value),
            _ => self.set_high_res_256(component, value),
        }
    }

    fn push_mouse_pattern(&mut self, value: u8) {
        if !self.high_res_mouse.defining {
            return;
        }
        let count = self.high_res_mouse.pattern_count;
        if count < 512 {
            self.high_res_mouse.and_pattern[count as usize] = value;
            self.high_res_mouse.pattern_count += 1;
        } else if count < 1024 {
            self.high_res_mouse.or_pattern[(count - 512) as usize] = value;
            self.high_res_mouse.pattern_count += 1;
        }
    }

    fn set_mouse_define(&mut self, value: u16) {
        if value == 0 {
            self.high_res_mouse.defining = true;
            self.high_res_mouse.defined = false;
            self.high_res_mouse.pattern_count = 0;
        } else if self.high_res_mouse.defining {
            self.high_res_mouse.defined = true;
            self.high_res_mouse.defining = false;
        }
    }

    fn set_high_res_16(&mut self, page: usize, component: usize, value: u8) {
        let expanded = quantize4(value);
        let color = &mut self.high_res_palette_16[page]
            [usize::from(self.high_res_palette_code_latch & 0x0F)];
        match component {
            0 => color.red = expanded,
            1 => color.green = expanded,
            _ => color.blue = expanded,
        }
    }

    fn set_high_res_256(&mut self, component: usize, value: u8) {
        let color = &mut self.high_res_palette_256[usize::from(self.high_res_palette_code_latch)];
        match component {
            0 => color.red = value,
            1 => color.green = value,
            _ => color.blue = value,
        }
    }

    fn get_high_res_16(&self, page: usize, component: usize) -> u8 {
        let color =
            self.high_res_palette_16[page][usize::from(self.high_res_palette_code_latch & 0x0F)];
        let raw = match component {
            0 => color.red,
            1 => color.green,
            _ => color.blue,
        };
        raw & 0xF0
    }

    fn get_high_res_256(&self, component: usize) -> u8 {
        let color = self.high_res_palette_256[usize::from(self.high_res_palette_code_latch)];
        match component {
            0 => color.red,
            1 => color.green,
            _ => color.blue,
        }
    }

    /// Reads one byte lane (0-3) of the selected high-res register (0x0474-0x0477).
    pub fn read_high_res_data(&self, lane: u8) -> u8 {
        if !self.high_res_available {
            return 0xFF;
        }
        let index = usize::from(self.high_res_addr_latch);
        match lane {
            0 => self.read_high_res_lane0(index),
            1 => self.read_high_res_lane1(index),
            2 => self.read_high_res_lane2(index),
            _ => self.high_res_reg.get(index).map_or(0, |r| (r >> 24) as u8),
        }
    }

    fn read_high_res_lane0(&self, index: usize) -> u8 {
        match index {
            HR_CTRL1 => {
                let mut data = 0;
                if self.high_res_reg4_bit1 {
                    data |= 2;
                }
                if self.high_res_reg4_bit0 {
                    data |= 1;
                }
                data
            }
            HR_VSYNC1 => {
                if self.vsync_active {
                    2
                } else {
                    0
                }
            }
            HR_PALCOL => self.read_high_res_palette(2),
            _ => self.high_res_reg.get(index).map_or(0, |r| *r as u8),
        }
    }

    fn read_high_res_lane1(&self, index: usize) -> u8 {
        match index {
            HR_CTRL1 => 0,
            HR_PALCOL => self.read_high_res_palette(0),
            _ => self.high_res_reg.get(index).map_or(0, |r| (r >> 8) as u8),
        }
    }

    fn read_high_res_lane2(&self, index: usize) -> u8 {
        match index {
            HR_PALCOL => self.read_high_res_palette(1),
            _ => self.high_res_reg.get(index).map_or(0, |r| (r >> 16) as u8),
        }
    }

    fn read_high_res_palette(&self, component: usize) -> u8 {
        match self.high_res_reg[HR_PALSEL] {
            0 => self.get_high_res_16(0, component),
            1 => self.get_high_res_16(1, component),
            _ => self.get_high_res_256(component),
        }
    }

    fn high_res_single_page(&self) -> bool {
        self.high_res_reg[HR_PGCTRL] & 2 == 0
    }

    fn high_res_display_size(&self) -> (usize, usize) {
        let width = self.high_res_reg[HR_XEND].wrapping_sub(self.high_res_reg[HR_XSTART]) as usize;
        let height = self.high_res_reg[HR_YEND].wrapping_sub(self.high_res_reg[HR_YSTART]) as usize;
        (width, height)
    }

    fn high_res_page_zoom2x(&self, page: usize) -> (u8, u8) {
        let zoom = self.high_res_reg[HR_PAGE_ZOOM[page]];
        let zoom_x = (1 + (zoom & 0xFF)) * 2;
        let zoom_y = (1 + ((zoom >> 8) & 0xFF)) * 2;
        (zoom_x.min(255) as u8, zoom_y.min(255) as u8)
    }

    fn high_res_page_bits_per_pixel(&self, page: usize) -> u8 {
        match self.high_res_reg[HR_PAGE_PALETTE[page]] & 0xFFFF {
            0xFF => 8,
            0x8000 => 16,
            0xFFFF => 24,
            _ => 4,
        }
    }

    fn high_res_page_shown(&self, page: usize) -> bool {
        let show_bit = if page == 0 { 0x100 } else { 0x200 };
        self.high_res_reg[HR_DISPPAGE] & show_bit != 0
    }

    fn build_high_res_layer(&self, page: usize) -> TownsLayer {
        let bits_per_pixel = self.high_res_page_bits_per_pixel(page);
        let vram_width = self.high_res_reg[HR_PAGE_VRAM_WIDTH[page]] as usize;
        let offset_x = self.high_res_reg[HR_PAGE_VRAM_OFFSET_X[page]] as usize;
        let offset_y = self.high_res_reg[HR_PAGE_VRAM_OFFSET_Y[page]] as usize;
        let bytes_per_line = vram_width * usize::from(bits_per_pixel) / 8;
        let scroll_offset = (offset_y * vram_width + offset_x) * usize::from(bits_per_pixel) / 8;
        let (width, height) = self.high_res_display_size();
        let (zoom_x, zoom_y) = self.high_res_page_zoom2x(page);
        let h_scroll_mask = if bytes_per_line.is_power_of_two() {
            bytes_per_line - 1
        } else {
            usize::MAX
        };
        let v_scroll_mask = if self.high_res_single_page() {
            HR_V_SCROLL_MASK_FULL
        } else {
            HR_V_SCROLL_MASK_PAGE
        };
        TownsLayer {
            shown: self.high_res_page_shown(page),
            bits_per_pixel,
            vram_addr: HR_PAGE_STRIDE * page,
            bytes_per_line,
            scroll_offset,
            h_scroll_mask,
            v_scroll_mask,
            vram_h_skip_bytes: 0,
            width,
            height,
            origin_x: 0,
            origin_y: 0,
            zoom_x,
            zoom_y,
            plane_mask: 0x0F,
            palette_bank: page as u8,
            high_res_rgb_swap: (self.high_res_reg[HR_PAGE_RGB_SWAP[page]] & 0x3F) as u8,
        }
    }

    fn high_res_palette_16_rgba(&self) -> [[u32; 16]; 2] {
        let mut output = [[0u32; 16]; 2];
        for (bank, colors) in self.high_res_palette_16.iter().enumerate() {
            for (index, color) in colors.iter().enumerate() {
                output[bank][index] = color.to_rgba();
            }
        }
        output
    }

    fn high_res_palette_256_rgba(&self) -> [u32; 256] {
        let mut output = [0u32; 256];
        for (index, color) in self.high_res_palette_256.iter().enumerate() {
            output[index] = color.to_rgba();
        }
        output
    }

    fn high_res_cursor_view(&self) -> Option<HighResCursor> {
        if !self.high_res_mouse.defined {
            return None;
        }
        Some(HighResCursor {
            x: self.high_res_mouse.x,
            y: self.high_res_mouse.y,
            origin_x: self.high_res_mouse.origin_x,
            origin_y: self.high_res_mouse.origin_y,
            and_pattern: self.high_res_mouse.and_pattern,
            or_pattern: self.high_res_mouse.or_pattern,
        })
    }

    /// Resolves the current CRTC state into renderer geometry: the two layers,
    /// single-page flag, priority page, and the overall display dimensions.
    pub fn resolve(
        &self,
        fmr_display_planes: u8,
        fmr_display_page_offset: usize,
        sprite_display_offset: usize,
    ) -> ResolvedVideo {
        if self.high_res_enabled {
            let layers = [self.build_high_res_layer(0), self.build_high_res_layer(1)];
            let (width, height) = self.high_res_display_size();
            return ResolvedVideo {
                single_page: self.high_res_single_page(),
                priority_page: (self.high_res_reg[HR_DISPPAGE] & 1) as usize,
                layers,
                palette_16: self.high_res_palette_16_rgba(),
                palette_256: self.high_res_palette_256_rgba(),
                width: (width.max(1)) as u32,
                height: (height.max(1)) as u32,
                high_res: true,
                mouse_cursor: self.high_res_cursor_view(),
            };
        }

        let layers = [
            self.build_layer(
                0,
                fmr_display_planes,
                fmr_display_page_offset,
                sprite_display_offset,
            ),
            self.build_layer(
                1,
                fmr_display_planes,
                fmr_display_page_offset,
                sprite_display_offset,
            ),
        ];

        let mut width = 0usize;
        let mut height = 0usize;
        for layer in &layers {
            if layer.shown {
                width = width.max(layer.origin_x + layer.width);
                height = height.max(layer.origin_y + layer.height);
            }
        }
        height = height.max(self.scan_height());
        if width == 0 {
            width = 640;
        }
        if height == 0 {
            height = 480;
        }

        ResolvedVideo {
            single_page: self.single_page(),
            priority_page: self.priority_page(),
            layers,
            palette_16: self.palette_16_rgba(),
            palette_256: self.palette_256_rgba(),
            width: width as u32,
            height: height as u32,
            high_res: false,
            mouse_cursor: None,
        }
    }
}

/// Expands a 4-bit-precision palette component write (keeps the high nibble and
/// replicates it into the low nibble), matching the analog palette hardware.
fn quantize4(value: u8) -> u8 {
    (value & 0xF0) | (value >> 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_register(video: &mut TownsVideo, index: u8, value: u16) {
        video.write_crtc_address(index);
        video.write_crtc_data_low(value as u8);
        video.write_crtc_data_high((value >> 8) as u8);
    }

    /// Selects which palette the analog color ports address, matching the
    /// FMR-compatible setup: sifter[1] bits 5-4 pick the bank (0 = layer 0,
    /// 2 = layer 1, 1/3 = the 256-color palette).
    fn select_palette(video: &mut TownsVideo, select: u8) {
        video.write_video_out_address(1);
        video.write_video_out_data(select << 4);
    }

    #[test]
    fn digital_palette_defaults_to_identity() {
        let video = TownsVideo::new(false);
        for index in 0..8 {
            assert_eq!(video.read_digital_palette(index), index as u8);
        }
    }

    #[test]
    fn digital_palette_round_trips_written_codes() {
        // The FMR-compatible palette-setup routine reads the eight digital
        // palette registers back to decide which analog color each FMR entry
        // maps to. Reads must return the stored 4-bit code, not a floating
        // value; returning 0xFF collapsed every entry onto color 15.
        let mut video = TownsVideo::new(false);
        let codes = [0x0F, 0x0A, 0x03, 0x00, 0x07, 0x0C, 0x01, 0x09];
        for (index, &code) in codes.iter().enumerate() {
            video.write_digital_palette(index, code);
        }
        for (index, &code) in codes.iter().enumerate() {
            assert_eq!(video.read_digital_palette(index), code);
        }
    }

    #[test]
    fn digital_palette_keeps_only_the_low_nibble() {
        let mut video = TownsVideo::new(false);
        video.write_digital_palette(0, 0xF5);
        assert_eq!(video.read_digital_palette(0), 0x05);
    }

    #[test]
    fn analog_palette_16_round_trips_at_four_bit_precision() {
        let mut video = TownsVideo::new(false);
        select_palette(&mut video, 0);
        video.write_palette_code(3);
        video.write_palette_red(0xC0);
        video.write_palette_green(0x50);
        video.write_palette_blue(0xA0);
        assert_eq!(video.read_palette_red(), 0xC0);
        assert_eq!(video.read_palette_green(), 0x50);
        assert_eq!(video.read_palette_blue(), 0xA0);
    }

    #[test]
    fn analog_palette_16_banks_are_independent() {
        let mut video = TownsVideo::new(false);
        select_palette(&mut video, 0);
        video.write_palette_code(5);
        video.write_palette_red(0x10);
        video.write_palette_green(0x20);
        video.write_palette_blue(0x30);

        select_palette(&mut video, 2);
        video.write_palette_code(5);
        video.write_palette_red(0x90);
        video.write_palette_green(0xA0);
        video.write_palette_blue(0xB0);

        select_palette(&mut video, 0);
        video.write_palette_code(5);
        assert_eq!(video.read_palette_red(), 0x10);
        assert_eq!(video.read_palette_green(), 0x20);
        assert_eq!(video.read_palette_blue(), 0x30);

        select_palette(&mut video, 2);
        video.write_palette_code(5);
        assert_eq!(video.read_palette_red(), 0x90);
        assert_eq!(video.read_palette_green(), 0xA0);
        assert_eq!(video.read_palette_blue(), 0xB0);
    }

    #[test]
    fn crtc_register_round_trips() {
        let mut video = TownsVideo::new(false);
        set_register(&mut video, REG_HST as u8, 0x1234);
        video.write_crtc_address(REG_HST as u8);
        assert_eq!(video.read_crtc_data_low(), 0x34);
        assert_eq!(video.read_crtc_data_high(false, (true, true)), 0x12);
    }

    #[test]
    fn default_geometry_is_640x400_two_page_16_color() {
        let video = TownsVideo::new(false);
        let resolved = video.resolve(0x0F, 0, 0);
        assert!(!resolved.single_page);
        assert_eq!(resolved.layers[0].bits_per_pixel, 4);
        assert_eq!(resolved.layers[0].width, 640);
        assert_eq!(resolved.layers[0].height, 400);
    }

    #[test]
    fn fmr_display_page_offset_applies_to_layer_0_only() {
        let video = TownsVideo::new(false);
        let base = video.resolve(0x0F, 0, 0);
        let fmr = video.resolve(0x0F, 0x0002_0000, 0);
        assert_eq!(
            fmr.layers[0].scroll_offset,
            base.layers[0].scroll_offset + 0x0002_0000
        );
        assert_eq!(fmr.layers[1].scroll_offset, base.layers[1].scroll_offset);
        let sprite = video.resolve(0x0F, 0, 0x0002_0000);
        assert_eq!(sprite.layers[0].scroll_offset, base.layers[0].scroll_offset);
        assert_eq!(
            sprite.layers[1].scroll_offset,
            base.layers[1].scroll_offset + 0x0002_0000
        );
    }

    #[test]
    fn vram_h_skip_bytes_from_haj_less_than_hds() {
        let mut video = TownsVideo::new(false);
        // Default HDS0 is 0x009C with a raw zoom nibble of 0 and 4 bpp pages.
        set_register(&mut video, REG_HAJ0 as u8, 0x008C);
        let resolved = video.resolve(0x0F, 0, 0);
        assert_eq!(resolved.layers[0].vram_h_skip_bytes, 8);
        set_register(&mut video, REG_HAJ0 as u8, 0x009C);
        let resolved = video.resolve(0x0F, 0, 0);
        assert_eq!(resolved.layers[0].vram_h_skip_bytes, 0);
    }

    #[test]
    fn letterbox_height_extends_to_the_scan_bottom() {
        let mut video = TownsVideo::new(false);
        // 15 kHz two-page 32768-color mode with 320 content lines starting
        // below the border, as programmed by a letterboxed intro.
        set_register(&mut video, REG_HST as u8, 0x0617);
        set_register(&mut video, REG_VST as u8, 0x020B);
        set_register(&mut video, REG_HDS0 as u8, 0x00E7);
        set_register(&mut video, REG_HDE0 as u8, 0x05E7);
        set_register(&mut video, REG_HDS1 as u8, 0x00E7);
        set_register(&mut video, REG_HAJ0 as u8, 0x00E7);
        set_register(&mut video, REG_VDS0 as u8, 0x007A);
        set_register(&mut video, REG_VDE0 as u8, 0x01BA);
        set_register(&mut video, REG_VDS1 as u8, 0x007A);
        set_register(&mut video, REG_VDE1 as u8, 0x01BA);
        set_register(&mut video, REG_ZOOM as u8, 0x0303);
        set_register(&mut video, REG_CR0 as u8, 0x8005);
        set_register(&mut video, REG_CR1 as u8, 0x0001);
        assert_eq!(video.horizontal_frequency_khz(), 15);
        let resolved = video.resolve(0x0F, 0, 0);
        assert_eq!(resolved.layers[0].origin_y, 82);
        assert_eq!(resolved.layers[0].height, 320);
        // The frame reaches the scan bottom, leaving a lower letterbox bar
        // instead of ending at the content.
        assert_eq!(resolved.height, 482);
    }

    #[test]
    fn scroll_masks_follow_page_mode_and_stride() {
        let mut video = TownsVideo::new(false);
        let resolved = video.resolve(0x0F, 0, 0);
        assert_eq!(resolved.layers[0].h_scroll_mask, usize::MAX);
        assert_eq!(resolved.layers[0].v_scroll_mask, 0x0003_FFFF);
        // 512 bytes per line on layer 1 enables the horizontal wrap mask.
        set_register(&mut video, (REG_LO0 + 4) as u8, 0x0080);
        let resolved = video.resolve(0x0F, 0, 0);
        assert_eq!(resolved.layers[1].h_scroll_mask, 511);
        // Single-page mode widens layer 0's vertical wrap to the full VRAM.
        video.write_video_out_address(0);
        video.write_video_out_data(0x05);
        let resolved = video.resolve(0x0F, 0, 0);
        assert_eq!(resolved.layers[0].v_scroll_mask, 0x0007_FFFF);
    }

    #[test]
    fn ving_quirk_produces_640_wide_at_31khz() {
        let mut video = TownsVideo::new(false);
        // CLKSEL=3 is the default; program the VING HST and page geometry.
        set_register(&mut video, REG_HST as u8, VING_HST_A);
        set_register(&mut video, REG_HDS0 as u8, 0x0082);
        set_register(&mut video, REG_HDE0 as u8, 0x0282);
        set_register(&mut video, REG_ZOOM as u8, 0x1111);
        assert_eq!(video.horizontal_frequency_khz(), 31);
        assert_eq!(video.page_zoom2x(0), (5, 4));
        assert_eq!(video.page_size(0).0, 640);
    }

    #[test]
    fn vsync_irq_latches_and_clears() {
        let mut video = TownsVideo::new(false);
        assert!(!video.vsync_irq_pending());
        video.enter_vsync();
        assert!(video.vsync_irq_pending());
        video.clear_vsync_irq();
        assert!(!video.vsync_irq_pending());
    }

    #[test]
    fn analog_palette_write_uses_brg_order_and_select() {
        let mut video = TownsVideo::new(false);
        // Palette select 0 targets 16-color bank 0; write index 1 = pure red.
        video.write_palette_code(1);
        video.write_palette_red(0xF0);
        let palettes = video.palette_16_rgba();
        assert_eq!(palettes[0][1], towns_color_to_rgba(0xFF, 0x00, 0x00));
    }

    fn set_high_res_index(video: &mut TownsVideo, index: u16) {
        video.write_high_res_addr_low(index as u8);
        video.write_high_res_addr_high((index >> 8) as u8);
    }

    fn set_high_res_reg(video: &mut TownsVideo, index: u16, value: u32) {
        set_high_res_index(video, index);
        for lane in 0..4u8 {
            video.write_high_res_data(lane, (value >> (lane * 8)) as u8);
        }
    }

    #[test]
    fn high_res_detect_ports_report_model() {
        let mx = TownsVideo::new(true);
        assert_eq!(mx.read_high_res_id(), 0x7F);
        assert_eq!(mx.read_vram_size(), 0x01);
        let cx = TownsVideo::new(false);
        assert_eq!(cx.read_high_res_id(), 0x80);
        assert_eq!(cx.read_vram_size(), 0x00);
    }

    #[test]
    fn high_res_register_file_round_trips() {
        let mut video = TownsVideo::new(true);
        set_high_res_reg(&mut video, HR_PAGE_VRAM_WIDTH[0] as u16, 0x1234_5678);
        set_high_res_index(&mut video, HR_PAGE_VRAM_WIDTH[0] as u16);
        assert_eq!(video.read_high_res_data(0), 0x78);
        assert_eq!(video.read_high_res_data(1), 0x56);
        assert_eq!(video.read_high_res_data(2), 0x34);
        assert_eq!(video.read_high_res_data(3), 0x12);
    }

    #[test]
    fn high_res_register_file_ignored_without_high_res() {
        let mut video = TownsVideo::new(false);
        set_high_res_reg(&mut video, HR_PAGE_VRAM_WIDTH[0] as u16, 0x1234_5678);
        set_high_res_index(&mut video, HR_PAGE_VRAM_WIDTH[0] as u16);
        assert_eq!(video.read_high_res_data(0), 0xFF);
    }

    #[test]
    fn ctrl0_enables_high_res() {
        let mut video = TownsVideo::new(true);
        assert!(!video.high_res_enabled);
        set_high_res_index(&mut video, HR_CTRL0 as u16);
        video.write_high_res_data(0, 0x01);
        assert!(video.high_res_enabled);
        set_high_res_index(&mut video, HR_CTRL0 as u16);
        video.write_high_res_data(0, 0x00);
        assert!(!video.high_res_enabled);
    }

    #[test]
    fn ctrl1_status_bits_set_by_cr0_and_cleared_by_bit1() {
        let mut video = TownsVideo::new(true);
        // A CR0 write with the START bit clear sets both status bits.
        set_register(&mut video, REG_CR0 as u8, 0x0003);
        set_high_res_index(&mut video, HR_CTRL1 as u16);
        assert_eq!(video.read_high_res_data(0), 0x03);
        // Writing bit 1 clears both.
        video.write_high_res_data(0, 0x02);
        assert_eq!(video.read_high_res_data(0), 0x00);
    }

    #[test]
    fn high_res_palette_lane_maps_to_components() {
        let mut video = TownsVideo::new(true);
        // Select 16-color bank 0, index 1.
        set_high_res_reg(&mut video, HR_PALSEL as u16, 0);
        set_high_res_index(&mut video, HR_PALINDEX as u16);
        video.write_high_res_data(0, 1);
        // PALCOL lanes: D0 -> blue, D1 -> red, D2 -> green.
        set_high_res_index(&mut video, HR_PALCOL as u16);
        video.write_high_res_data(0, 0xF0);
        video.write_high_res_data(1, 0xE0);
        video.write_high_res_data(2, 0xD0);
        assert_eq!(video.read_high_res_data(0), 0xF0);
        assert_eq!(video.read_high_res_data(1), 0xE0);
        assert_eq!(video.read_high_res_data(2), 0xD0);
        // 16-color components carry 4-bit precision replicated into the low nibble.
        let rgba = video.high_res_palette_16_rgba();
        assert_eq!(rgba[0][1], towns_color_to_rgba(0xEE, 0xDD, 0xFF));
    }

    #[test]
    fn depth_select_from_page_palette_register() {
        let mut video = TownsVideo::new(true);
        set_high_res_reg(&mut video, HR_CTRL0 as u16, 1);
        set_high_res_reg(&mut video, HR_XSTART as u16, 0);
        set_high_res_reg(&mut video, HR_XEND as u16, 640);
        set_high_res_reg(&mut video, HR_YSTART as u16, 0);
        set_high_res_reg(&mut video, HR_YEND as u16, 480);
        set_high_res_reg(&mut video, HR_DISPPAGE as u16, 0x100);
        for (palette_value, expected) in [(0x0Fu32, 4u8), (0xFF, 8), (0x8000, 16), (0xFFFF, 24)] {
            set_high_res_reg(&mut video, HR_PAGE_PALETTE[0] as u16, palette_value);
            let resolved = video.resolve(0x0F, 0, 0);
            assert_eq!(resolved.layers[0].bits_per_pixel, expected);
        }
    }

    #[test]
    fn high_res_geometry_resolves_1024x768_two_page() {
        let mut video = TownsVideo::new(true);
        set_high_res_reg(&mut video, HR_CTRL0 as u16, 1);
        set_high_res_reg(&mut video, HR_PGCTRL as u16, 2);
        set_high_res_reg(&mut video, HR_XSTART as u16, 0);
        set_high_res_reg(&mut video, HR_XEND as u16, 1024);
        set_high_res_reg(&mut video, HR_YSTART as u16, 0);
        set_high_res_reg(&mut video, HR_YEND as u16, 768);
        set_high_res_reg(&mut video, HR_PAGE_PALETTE[0] as u16, 0xFF);
        set_high_res_reg(&mut video, HR_PAGE_VRAM_WIDTH[0] as u16, 1024);
        set_high_res_reg(&mut video, HR_DISPPAGE as u16, 0x0100);
        let resolved = video.resolve(0x0F, 0, 0);
        assert!(resolved.high_res);
        assert!(!resolved.single_page);
        assert_eq!(resolved.width, 1024);
        assert_eq!(resolved.height, 768);
        assert_eq!(resolved.layers[0].bits_per_pixel, 8);
        assert_eq!(resolved.layers[0].vram_addr, 0);
        assert_eq!(resolved.layers[1].vram_addr, 0x0008_0000);
        assert_eq!(resolved.layers[0].bytes_per_line, 1024);
        assert!(resolved.layers[0].shown);
    }

    #[test]
    fn word_palette_write_auto_increments_index() {
        let mut video = TownsVideo::new(true);
        set_high_res_reg(&mut video, HR_PALSEL as u16, 2);
        set_high_res_index(&mut video, HR_PALINDEX as u16);
        video.write_high_res_data_low_word(0);
        // Two 32-bit color writes to consecutive indices without re-latching.
        set_high_res_index(&mut video, HR_PALCOL as u16);
        // Low word packs blue (byte 0) and red (byte 1); high word sets green.
        video.write_high_res_data_low_word(0x11 << 8);
        video.write_high_res_data_high_word(0x22);
        video.write_high_res_data_low_word(0x33 | (0x44 << 8));
        video.write_high_res_data_high_word(0x55);
        let palette = video.high_res_palette_256_rgba();
        assert_eq!(palette[0], towns_color_to_rgba(0x11, 0x22, 0x00));
        assert_eq!(palette[1], towns_color_to_rgba(0x44, 0x55, 0x33));
    }
}
