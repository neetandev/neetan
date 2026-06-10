//! CPU-side software renderer for the PC-98 display.

#![warn(missing_docs)]
#![deny(unsafe_code)]
#![cfg_attr(not(any(test, feature = "std")), no_std)]

extern crate alloc;

use alloc::{boxed::Box, vec};
#[cfg(feature = "std")]
use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
};

mod compose;
mod ga1280a;
mod text_normalizer;

pub use ga1280a::{
    Ga1280aCursorRenderInputs, Ga1280aRenderInputs, Ga1280aRenderMode, compose as compose_ga1280a,
};

/// Total byte size of the PC-98 text VRAM image (16 KiB).
///
/// Bytes 0x0000-0x1FFF hold character codes (low and high byte interleaved
/// per cell); bytes 0x2000-0x3FFF hold attribute bytes with the same cell
/// layout.
pub const TEXT_VRAM_BYTES: usize = 0x4000;

/// Number of normalized text cells (covers a maximum 80x52 logical text plane).
const TEXT_CELL_COUNT: usize = 4096;

const FONT_ROM_BUFFER_SIZE: usize = 0x83000;

/// Maximum framebuffer width supported by the renderer, in pixels. Covers the
/// GA-1280A maximum visible width.
pub const MAX_FRAMEBUFFER_WIDTH: usize = 1600;
/// Maximum framebuffer height supported by the renderer, in pixels. Covers the
/// GA-1280A maximum visible height.
pub const MAX_FRAMEBUFFER_HEIGHT: usize = 1024;
/// Maximum framebuffer pixel count supported by the renderer.
pub const MAX_FRAMEBUFFER_PIXELS: usize = MAX_FRAMEBUFFER_WIDTH * MAX_FRAMEBUFFER_HEIGHT;
/// Maximum framebuffer byte size supported by the renderer.
pub const MAX_FRAMEBUFFER_BYTES: usize = MAX_FRAMEBUFFER_PIXELS * 4;

/// CPU-side renderer for the PC-98 display compose pass.
pub struct SoftwareRenderer {
    /// Embedded state for save/restore.
    pub state: SoftwareRendererState,
    /// Per-row scratch reused every frame; not part of save/restore.
    scratch: Box<compose::ComposeScratch>,
    /// Cached SIMD availability for the compose pass (AVX2 on x86_64, NEON
    /// on aarch64). Always false on other architectures.
    has_simd: bool,
}

/// Persistent buffers owned by the renderer.
pub struct SoftwareRendererState {
    /// Internal copy of the CGROM/font ROM used for text rasterization.
    pub font_rom: Box<[u8]>,
    /// 640x480 framebuffer, 4 bytes per pixel as `R, G, B, A` little-endian.
    pub framebuffer: Box<[u8]>,
    /// Per-cell normalized text descriptors (scratch reused across frames).
    pub text_cells: Box<[u32; TEXT_CELL_COUNT]>,
}

/// Per-frame inputs to the software renderer.
pub struct RenderInputs<'a> {
    /// Full TVRAM image: 0x0000-0x1FFF character bytes, 0x2000-0x3FFF attribute bytes.
    pub text_vram: &'a [u8; TEXT_VRAM_BYTES],
    /// GDC text pitch in cells per row (typically 80).
    pub gdc_text_pitch: u32,
    /// Four packed text scroll descriptors: low 16 bits = start address, high 16 bits = line count.
    pub gdc_scroll_start_line: [u32; 4],
    /// Video mode register (port 0x68 value).
    pub video_mode: u32,
    /// CRTC PL (low 16) and BL (high 16).
    pub crtc_pl_bl: u32,
    /// CRTC CL (low 16) and SSL (high 16).
    pub crtc_cl_ssl: u32,
    /// CRTC SUR (low 16) and SDR (high 16).
    pub crtc_sur_sdr: u32,
    /// Mask applied to `char_high` to detect kanji. 0xFF for code-access mode,
    /// 0x00 when KAC dot-access mode is selected (which disables kanji decoding).
    pub kanji_high_mask: u8,
    /// True when port 0x68 bit 0 selects semigraphics for attr bit 4.
    pub attr_semigraphics_mode: bool,
    /// True when port 0x68 bit 3 selects 7x13/8x16 mode.
    pub fontsel_8x16: bool,
    /// True when the current frame's blink phase makes blink-attributed cells visible.
    pub blink_visible: bool,
    /// True when the cursor is currently visible.
    pub cursor_visible: bool,
    /// EAD address of the cursor cell.
    pub cursor_addr: u32,
    /// First scanline of the cursor (CSRFORM cursor_top, 0-31).
    pub cursor_top: u32,
    /// Last scanline of the cursor (CSRFORM cursor_bottom, 0-31).
    pub cursor_bottom: u32,

    /// Graphics GDC byte pitch.
    pub gdc_graphics_pitch: u32,
    /// Four packed graphics scroll descriptors.
    pub gdc_graphics_scroll: [u32; 4],
    /// Whether the graphics GDC SYNC P1 display mode is graphics.
    pub gdc_graphics_display_mode_is_graphics: bool,
    /// Graphics GDC active display lines (AL from SYNC command).
    pub gdc_graphics_al: u32,
    /// CRT horizontal-scan-frequency flag (port 09A8h).
    /// True = 31.778 kHz / 480-line capable, false = 24.823 kHz / 400 lines.
    /// Required in combination with `gdc_graphics_al > 400` for 480-line PEGC output.
    pub crt_31khz_enabled: bool,

    /// 16-entry palette packed as `0xAA_BB_GG_RR`.
    pub palette_rgba: [u32; 16],
    /// Master display enable: when false, the compose pass outputs an all-black frame.
    pub global_enabled: bool,
    /// Text plane display enable (master GDC DE).
    pub text_enabled: bool,
    /// Graphics plane display enable (slave GDC DE).
    pub graphics_enabled: bool,

    /// Graphics source: either the GDC bitplane stack or PEGC 256-color VRAM.
    /// The two modes are mutually exclusive on real hardware.
    pub graphics: GraphicsInput<'a>,
}

/// Graphics rendering source. PC-98 hardware exposes either the four GDC
/// bitplanes (B/R/G/E) or the linear PEGC 256-color framebuffer at any one
/// time, never both simultaneously.
pub enum GraphicsInput<'a> {
    /// Traditional GDC/GRCG/EGC bitplane mode (8-color or 16-color analog).
    Gdc(GdcGraphicsInput<'a>),
    /// PEGC 256-color packed-pixel mode.
    Pegc(Box<PegcRenderInputs<'a>>),
}

/// Inputs consumed by the GDC bitplane compose path.
pub struct GdcGraphicsInput<'a> {
    /// Graphics VRAM B-plane (32 KB) for the active display page.
    pub b_plane: &'a [u8],
    /// Graphics VRAM R-plane (32 KB) for the active display page.
    pub r_plane: &'a [u8],
    /// Graphics VRAM G-plane (32 KB) for the active display page.
    pub g_plane: &'a [u8],
    /// Graphics VRAM E-plane (32 KB) for the active display page.
    pub e_plane: &'a [u8],
    /// Graphics GDC line repeat factor (CSRFORM).
    pub lines_per_row: u32,
    /// Graphics GDC display zoom factor (0-15, rendered as zoom+1).
    pub zoom_display: u32,
    /// Bitmask of graphics color indices that are "on" in monochrome mode.
    pub monochrome_mask: u32,
    /// 16-color analog graphics mode (vs 8-color/monochrome digital).
    pub is_16_color: bool,
}

/// Extra inputs required for PEGC 256-color rendering.
pub struct PegcRenderInputs<'a> {
    /// 256-entry palette packed as `0xAA_BB_GG_RR`.
    pub palette_rgba_256: [u32; 256],
    /// Flags: bit 0 = packed pixel mode, bit 1 = 1-screen (480-line) mode, bit 2 = display page 1.
    pub pegc_flags: u32,
    /// Full 512 KB PEGC VRAM as raw bytes.
    pub vram: &'a [u8],
}

impl SoftwareRenderer {
    /// Native compose-pass output width in pixels.
    pub const WIDTH: usize = 640;
    /// Native compose-pass output height in pixels.
    pub const HEIGHT: usize = 480;
    /// Number of pixels covered by the active PC-98 GDC/PEGC display area.
    pub const PIXEL_COUNT: usize = Self::WIDTH * Self::HEIGHT;
    /// Bytes per pixel (`R, G, B, A`).
    pub const PIXEL_BYTES: usize = 4;
    /// Byte size covered by the active PC-98 GDC/PEGC display area.
    pub const FRAMEBUFFER_BYTES: usize = Self::PIXEL_COUNT * Self::PIXEL_BYTES;

    /// Creates a new renderer with a copy of the supplied font ROM data.
    pub fn new(font_rom_data: &[u8]) -> Self {
        let mut font_rom = vec![0u8; FONT_ROM_BUFFER_SIZE].into_boxed_slice();
        copy_font_rom(&mut font_rom, font_rom_data);
        let framebuffer = vec![0u8; MAX_FRAMEBUFFER_BYTES].into_boxed_slice();
        let text_cells: Box<[u32; TEXT_CELL_COUNT]> = vec![0u32; TEXT_CELL_COUNT]
            .into_boxed_slice()
            .try_into()
            .expect("TEXT_CELL_COUNT u32 values");
        Self {
            state: SoftwareRendererState {
                font_rom,
                framebuffer,
                text_cells,
            },
            scratch: compose::ComposeScratch::new(),
            has_simd: detect_simd(),
        }
    }

    /// Replaces the font ROM data used by future renders.
    pub fn update_font_rom(&mut self, font_rom_data: &[u8]) {
        copy_font_rom(&mut self.state.font_rom, font_rom_data);
    }

    /// Enables or disables the SIMD compose dispatch. Intended for parity
    /// testing of the scalar fallback against the SIMD path; production
    /// callers should leave the renderer at its default (SIMD if available).
    pub fn set_simd_enabled(&mut self, enabled: bool) {
        self.has_simd = enabled && detect_simd();
    }

    /// Renders one frame into the internal framebuffer.
    pub fn render(&mut self, inputs: &RenderInputs<'_>) {
        text_normalizer::normalize_text_plane(
            &text_normalizer::TextNormalizerInputs {
                text_vram: inputs.text_vram,
                pitch: inputs.gdc_text_pitch,
                kanji_high_mask: inputs.kanji_high_mask,
                attr_semigraphics_mode: inputs.attr_semigraphics_mode,
                fontsel_8x16: inputs.fontsel_8x16,
                blink_visible: inputs.blink_visible,
                cursor_visible: inputs.cursor_visible,
                cursor_addr: inputs.cursor_addr,
            },
            &mut self.state.text_cells,
        );
        compose::compose(
            &self.state.font_rom,
            &self.state.text_cells,
            inputs,
            &mut self.state.framebuffer,
            &mut self.scratch,
            self.has_simd,
        );
    }

    /// Returns the internal framebuffer as packed `R, G, B, A` bytes (little-endian per pixel).
    ///
    /// The buffer is sized to `MAX_FRAMEBUFFER_BYTES`. Callers that have just
    /// rendered into a subregion should slice it to `width * height * 4`.
    pub fn framebuffer(&self) -> &[u8] {
        &self.state.framebuffer
    }

    /// Mutable access to the internal framebuffer, for compositors that write
    /// into it directly (such as [`compose_ga1280a`]).
    pub fn framebuffer_mut(&mut self) -> &mut [u8] {
        &mut self.state.framebuffer
    }

    /// Computes the active display height (400, or up to 480 for PEGC 480-line mode).
    ///
    /// PEGC 480-line output requires both `gdc_graphics_al > 400` AND the CRT
    /// 31.778 kHz scan frequency flag (port 09A8h).
    pub fn native_height(inputs: &RenderInputs<'_>) -> u32 {
        let is_pegc_256_color = matches!(inputs.graphics, GraphicsInput::Pegc(_));
        if is_pegc_256_color && inputs.gdc_graphics_al > 400 && inputs.crt_31khz_enabled {
            inputs.gdc_graphics_al.min(Self::HEIGHT as u32)
        } else {
            400
        }
    }

    /// Writes the top-left `width * height` region of a packed RGBA framebuffer
    /// to a binary PPM (P6) file.
    #[cfg(feature = "std")]
    pub fn write_ppm(
        path: impl AsRef<Path>,
        framebuffer: &[u8],
        width: u32,
        height: u32,
    ) -> io::Result<()> {
        let required = (width as usize) * (height as usize) * Self::PIXEL_BYTES;
        if framebuffer.len() < required {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "software renderer PPM output framebuffer is smaller than the requested region",
            ));
        }

        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        writeln!(writer, "P6\n{} {}\n255", width, height)?;

        for chunk in framebuffer[..required].chunks_exact(Self::PIXEL_BYTES) {
            writer.write_all(&chunk[0..3])?;
        }

        writer.flush()
    }
}

pub(crate) fn detect_simd() -> bool {
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    {
        is_x86_feature_detected!("avx2")
    }
    #[cfg(all(target_arch = "x86_64", not(feature = "std")))]
    {
        cfg!(target_feature = "avx2")
    }
    #[cfg(target_arch = "aarch64")]
    {
        true
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        false
    }
}

fn copy_font_rom(font_rom: &mut [u8], font_rom_data: &[u8]) {
    font_rom.fill(0);
    let copy_len = font_rom.len().min(font_rom_data.len());
    font_rom[..copy_len].copy_from_slice(&font_rom_data[..copy_len]);
}
