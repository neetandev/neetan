//! Renderer-neutral display descriptors shared by devices and renderers.

/// A resolved FM Towns display layer.
#[derive(Clone, Copy, Default)]
pub struct TownsLayer {
    /// Whether the layer is shown this frame.
    pub shown: bool,
    /// Color depth in bits per pixel.
    pub bits_per_pixel: u8,
    /// Base byte address within VRAM.
    pub vram_addr: usize,
    /// Source stride in bytes per scanline.
    pub bytes_per_line: usize,
    /// Scroll byte offset from the page base.
    pub scroll_offset: usize,
    /// Horizontal wrap mask.
    pub h_scroll_mask: usize,
    /// Vertical wrap mask.
    pub v_scroll_mask: usize,
    /// Horizontal source skip in bytes.
    pub vram_h_skip_bytes: usize,
    /// Displayed width in pixels.
    pub width: usize,
    /// Displayed height in pixels.
    pub height: usize,
    /// Horizontal display origin.
    pub origin_x: usize,
    /// Vertical display origin.
    pub origin_y: usize,
    /// Horizontal zoom stored at twice its value.
    pub zoom_x: u8,
    /// Vertical zoom stored at twice its value.
    pub zoom_y: u8,
    /// Four-bit plane display mask.
    pub plane_mask: u8,
    /// Analog palette bank.
    pub palette_bank: u8,
    /// High-resolution component reorder value.
    pub high_res_rgb_swap: u8,
}

/// FM Towns MX high-resolution hardware mouse cursor.
#[derive(Clone, Copy)]
pub struct HighResCursor {
    /// Horizontal cursor position.
    pub x: u32,
    /// Vertical cursor position.
    pub y: u32,
    /// Horizontal hot-spot offset.
    pub origin_x: u32,
    /// Vertical hot-spot offset.
    pub origin_y: u32,
    /// Cursor AND plane.
    pub and_pattern: [u8; 512],
    /// Cursor OR plane.
    pub or_pattern: [u8; 512],
}

save_state::runtime_state! {
/// PC-88VA graphics framebuffer descriptor.
#[derive(Clone, Copy, Default)]
pub struct FramebufferVa {
    /// Frame start address in graphics VRAM.
    pub frame_start: u32,
    /// Frame buffer width in bytes.
    pub frame_width: u16,
    /// Frame buffer line count.
    pub frame_lines: u16,
    /// Dot address within the first word.
    pub dot: u16,
    /// Horizontal offset.
    pub offset_x: u16,
    /// Vertical offset.
    pub offset_y: u16,
    /// Display start address in graphics VRAM.
    pub display_start: u32,
    /// Sub-screen height in scanlines.
    pub display_height: u16,
    /// Sub-screen first scanline.
    pub display_position: u16,
}}

impl FramebufferVa {
    /// Returns the reset state for the no-wrap screen 1 descriptor.
    pub fn reset_screen1() -> Self {
        Self {
            frame_start: 0xFFFF_FFFF,
            frame_lines: 0xFFFF,
            offset_x: 0xFFFF,
            offset_y: 0xFFFF,
            ..Self::default()
        }
    }
}
