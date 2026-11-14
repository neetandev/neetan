//! Renderer-neutral display descriptors shared by devices and renderers.

use alloc::{format, string::String};

/// Treatment of the gap rasters between the picture lines of a 200-line
/// graphics mode shown on a 400-line monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineDoubling {
    /// Follows the guest setting of the machine.
    #[default]
    Auto,
    /// Repeats the picture line in the gap raster.
    On,
    /// Blanks the gap raster.
    Off,
}

impl core::fmt::Display for LineDoubling {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Auto => formatter.write_str("auto"),
            Self::On => formatter.write_str("on"),
            Self::Off => formatter.write_str("off"),
        }
    }
}

impl core::str::FromStr for LineDoubling {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "on" => Ok(Self::On),
            "off" => Ok(Self::Off),
            _ => Err(format!(
                "unknown line doubling '{text}', expected auto, on or off"
            )),
        }
    }
}

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

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::LineDoubling;

    #[test]
    fn line_doubling_defaults_to_auto() {
        assert_eq!(LineDoubling::default(), LineDoubling::Auto);
    }

    #[test]
    fn line_doubling_display_and_from_str_round_trip() {
        for (value, name) in [
            (LineDoubling::Auto, "auto"),
            (LineDoubling::On, "on"),
            (LineDoubling::Off, "off"),
        ] {
            assert_eq!(value.to_string(), name);
            assert_eq!(name.parse::<LineDoubling>(), Ok(value));
        }
    }

    #[test]
    fn line_doubling_from_str_ignores_case() {
        assert_eq!("AUTO".parse::<LineDoubling>(), Ok(LineDoubling::Auto));
        assert_eq!("On".parse::<LineDoubling>(), Ok(LineDoubling::On));
        assert_eq!("oFF".parse::<LineDoubling>(), Ok(LineDoubling::Off));
    }

    #[test]
    fn line_doubling_from_str_rejects_unknown_names() {
        assert!("".parse::<LineDoubling>().is_err());
        assert!("yes".parse::<LineDoubling>().is_err());
        assert!("double".parse::<LineDoubling>().is_err());
    }
}
