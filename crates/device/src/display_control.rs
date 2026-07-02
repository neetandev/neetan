//! Display control registers - system board video configuration.
//!
//! Manages video mode, GDC clock, VSYNC IRQ enable, border color,
//! display line count, and display/access page selection.
//!
//! These are system board control registers, not a single chip.

/// Mode register 1 (port 0x68) bit 0: attribute select.
/// 0 = vertical line, 1 = simple graphics (PC-8001 compat).
/// Ref: undoc98 `io_disp.txt` port 0068h
const MODE1_ATR_SEL: u8 = 0x01;

/// Mode register 1 (port 0x68) bit 1: graphics color mode.
/// 0 = color, 1 = monochrome.
/// Ref: undoc98 `io_disp.txt` port 0068h
const MODE1_GRAPHIC_MODE: u8 = 0x02;

/// Mode register 1 (port 0x68) bit 2: text column width.
/// 0 = 80 columns, 1 = 40 columns.
/// Ref: undoc98 `io_disp.txt` port 0068h
const MODE1_COLUMN_WIDTH: u8 = 0x04;

/// Mode register 1 (port 0x68) bit 3: text font select.
/// 0 = 6x8 dot (200-line), 1 = 7x13 dot (400-line).
/// Ref: undoc98 `io_disp.txt` port 0068h
const MODE1_FONT_SEL: u8 = 0x08;

/// Mode register 1 (port 0x68) bit 4: graphics raster mode.
/// 0 = show odd rasters, 1 = hide odd rasters (200-line in 400-line CRT).
/// Ref: undoc98 `io_disp.txt` port 0068h
const MODE1_HIDE_ODD_RASTERS: u8 = 0x10;

/// Mode register 1 (port 0x68) bit 5: kanji access mode.
/// 0 = code access, 1 = dot access.
/// Ref: undoc98 `io_disp.txt` port 0068h
const MODE1_KAC_MODE: u8 = 0x20;

/// Mode register 2 bit 2: EGC extended mode request.
const MODE2_EGC_EXTENDED_MODE: u16 = 0x0004;

/// Mode register 2 bit 3: EGC mode change permission.
const MODE2_EGC_MODE_CHANGE_PERMISSION: u16 = 0x0008;

/// Mode register 1 (port 0x68) bit 6: NVM write permit.
/// 0 = memory switch write-protected, 1 = write permitted.
/// Ref: undoc98 `io_disp.txt` port 0068h
const MODE1_NVMW_PERMIT: u8 = 0x40;

/// Mode register 1 (port 0x68) bit 7: display enable.
/// 0 = all screens off, 1 = display enabled.
/// Ref: undoc98 `io_disp.txt` port 0068h
const MODE1_DISP_ENABLE: u8 = 0x80;

/// Snapshot of the display control state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayControlState {
    /// Video mode register (port 0x68 R/W).
    pub video_mode: u8,
    /// Mode register 2 (port 0x6A W) - flip-flop controlled.
    ///
    /// Lower byte (base page - display modes):
    /// - Bit 0: Palette/depth mode (0=8-color digital, 1=16-color analog)
    /// - Bit 2: Graphics accelerator (0=GRCG compatible, 1=EGC extended)
    /// - Bit 3: EGC mode change permission
    ///
    /// Upper byte (extended page - GDC clocks):
    /// - Bit 9: GDC CLOCK-1 (0=2.5MHz, 1=5.0MHz)
    /// - Bit 10: GDC CLOCK-2 (0=2.5MHz, 1=5.0MHz)
    pub mode2: u16,
    /// Latched EGC active flag.
    ///
    /// Updated only when mode2 bit 2 is toggled while bit 3 (permission)
    /// is set and EGC hardware is present. Once latched on, clearing bit 3
    /// does NOT unlatch this - only an explicit bit 2 toggle with bit 3
    /// set will clear it.
    pub egc_latched: bool,
    /// Whether VSYNC IRQ (IRQ 2) is armed for the next vertical retrace.
    ///
    /// Writing any value to port 0x64 arms a one-shot trigger: the next
    /// VSync event will raise IRQ 2 and then clear this flag automatically.
    pub vsync_irq_enabled: bool,
    /// Border color register (port 0x6C W).
    pub border_color: u8,
    /// Display line count register (port 0x6E W).
    pub display_line_count: u8,
    /// Graphics display page select (port 0xA4 W, bit 0 only).
    pub display_page: u8,
    /// VRAM drawing page select (port 0xA6 R/W, bit 0 only).
    pub access_page: u8,
    /// 31.778 kHz horizontal scan / 480-line mode select (port 0x09A8 R/W).
    ///
    /// 0 = 24.823 kHz (400 lines), 1 = 31.778 kHz (480 lines). PEGC 480-line
    /// output requires this flag in addition to a GDC AL > 400 setup.
    pub crt_31khz_enabled: bool,
}

/// Display control register block.
pub struct DisplayControl {
    /// Embedded state for save/restore.
    pub state: DisplayControlState,
}

impl Default for DisplayControl {
    fn default() -> Self {
        Self::new()
    }
}

impl DisplayControl {
    /// Creates a new display control block.
    pub fn new() -> Self {
        Self {
            state: DisplayControlState {
                video_mode: 0x00,
                mode2: 0x0000,
                egc_latched: false,
                vsync_irq_enabled: false,
                border_color: 0x00,
                display_line_count: 0x00,
                display_page: 0,
                access_page: 0,
                crt_31khz_enabled: false,
            },
        }
    }

    /// Returns whether the CRT is configured for 31.778 kHz / 480-line scan.
    pub fn is_crt_31khz_enabled(&self) -> bool {
        self.state.crt_31khz_enabled
    }

    /// Sets the CRT 31.778 kHz / 480-line scan flag (port 0x09A8 write).
    pub fn set_crt_31khz_enabled(&mut self, enabled: bool) {
        self.state.crt_31khz_enabled = enabled;
    }

    /// Reads the video mode register (port 0x68).
    pub fn read_video_mode(&self) -> u8 {
        self.state.video_mode
    }

    /// Writes the video mode flip-flop register (port 0x68).
    ///
    /// Port 0x68 uses a flip-flop write protocol:
    /// - Write format: `0000 ADR2 ADR1 ADR0 DT` (upper nibble must be 0)
    /// - ADR\[2:0\] (bits 3:1) selects which bit (0-7) to modify
    /// - DT (bit 0) is the value: 0 = clear, 1 = set
    ///
    /// Ref: undoc98 `io_disp.txt`
    pub fn write_video_mode(&mut self, value: u8) {
        if value & 0xF0 == 0 {
            let bit = 1 << ((value >> 1) & 7);
            if value & 1 != 0 {
                self.state.video_mode |= bit;
            } else {
                self.state.video_mode &= !bit;
            }
        }
    }

    /// Arms the one-shot VSYNC interrupt trigger (port 0x64 write).
    ///
    /// Writing **any** value to port 0x64 acknowledges the current VSYNC
    /// interrupt and arms a one-shot trigger so that the next vertical
    /// retrace raises IRQ 2.  The bus is responsible for clearing the
    /// IRQ line and GDC VSYNC flags.
    pub fn write_vsync_control(&mut self, _value: u8) {
        self.state.vsync_irq_enabled = true;
    }

    /// Writes the mode register 2 flip-flop (port 0x6A).
    ///
    /// Port 0x6A uses a two-page flip-flop protocol:
    /// - Base page (bit 7 clear, upper nibble 0): bits [3:1] = address 0-7, bit [0] = data
    /// - Extended page (bit 7 set): bits [2:1] = address 8-11, bit [0] = data
    ///
    /// `has_egc_hardware` should be true on machines with EGC (VX and later), since it gates
    /// the EGC latch update
    ///
    /// Ref: undoc98 `io_disp.txt`
    pub fn write_mode2(&mut self, value: u8, has_egc_hardware: bool) {
        if value & 0x80 != 0 {
            let address = ((value >> 1) & 0x03) + 8;
            self.write_mode2_bit(address.into(), value & 1 != 0);
        } else if value & 0xF0 == 0 {
            let address = (value >> 1) & 0x07;
            let data = value & 1 != 0;

            // Mode2 bits are always stored unconditionally.
            self.write_mode2_bit(address.into(), data);

            // For bit 2 (EGC mode): latch the runtime EGC flag only when
            // bit 3 (permission) is set AND EGC hardware is present.
            if address == 2 && self.is_egc_mode_change_permitted() && has_egc_hardware {
                self.state.egc_latched = data;
            }
        }
    }

    fn write_mode2_bit(&mut self, address: u16, data: bool) {
        let bit = 1u16 << address;
        if data {
            self.state.mode2 |= bit;
        } else {
            self.state.mode2 &= !bit;
        }
    }

    /// Returns whether 16-color mode is active (mode2 bit 0).
    pub fn is_16_color(&self) -> bool {
        self.state.mode2 & 0x01 != 0
    }

    /// Returns whether both GDC clocks are set to 5 MHz (mode2 bits 9-10).
    pub fn is_gdc_5mhz(&self) -> bool {
        self.state.mode2 & 0x0600 == 0x0600
    }

    /// Returns whether EGC extended mode was requested (mode2 bit 2).
    pub fn is_egc_extended_mode_requested(&self) -> bool {
        self.state.mode2 & MODE2_EGC_EXTENDED_MODE != 0
    }

    /// Returns whether mode2 bit2 can be changed (mode2 bit 3).
    pub fn is_egc_mode_change_permitted(&self) -> bool {
        self.state.mode2 & MODE2_EGC_MODE_CHANGE_PERMISSION != 0
    }

    /// Returns whether EGC extended mode is effectively active.
    ///
    /// This checks the latched runtime flag, not the mode2 register bits
    /// directly. The latch persists even if bit 3 is later cleared.
    pub fn is_egc_extended_mode_effective(&self) -> bool {
        self.state.egc_latched
    }

    /// Writes the border color register (port 0x6C).
    pub fn write_border_color(&mut self, value: u8) {
        self.state.border_color = value;
    }

    /// Writes the display line count register (port 0x6E).
    pub fn write_display_line_count(&mut self, value: u8) {
        self.state.display_line_count = value;
    }

    /// Writes the display page register (port 0xA4, bit 0 only).
    pub fn write_display_page(&mut self, value: u8) {
        self.state.display_page = value & 1;
    }

    /// Reads the access page register (port 0xA6).
    pub fn read_access_page(&self) -> u8 {
        self.state.access_page
    }

    /// Writes the access page register (port 0xA6, bit 0 only).
    pub fn write_access_page(&mut self, value: u8) {
        self.state.access_page = value & 1;
    }

    /// Returns whether 16-color analog palette mode is active (mode2 bit 0).
    pub fn is_palette_analog_mode(&self) -> bool {
        self.state.mode2 & 0x01 != 0
    }

    /// Returns whether graphics monochrome mode is active (mode1 bit 1).
    pub fn is_graphics_monochrome(&self) -> bool {
        self.state.video_mode & MODE1_GRAPHIC_MODE != 0
    }

    /// Returns whether attr bit4 selects semigraphics (mode1 bit 0).
    pub fn is_attr_semigraphics_enabled(&self) -> bool {
        self.state.video_mode & MODE1_ATR_SEL != 0
    }

    /// Returns whether text mode is 40 columns (mode1 bit 2).
    pub fn is_text_40_columns(&self) -> bool {
        self.state.video_mode & MODE1_COLUMN_WIDTH != 0
    }

    /// Returns whether the 8x16-cell font is selected (mode1 bit 3 set).
    ///
    /// Per `undoc98`, mode1 bit 3 = 1 selects the 7x13-dot font for 400-line
    /// displays, which is rendered inside 8x16 cells. The rest of the
    /// text-rendering code uses cell-size terminology (`ANK_8X16_BASE`,
    /// `fontsel_8x16`); this helper matches that.
    pub fn is_font_8x16_mode(&self) -> bool {
        self.state.video_mode & MODE1_FONT_SEL != 0
    }

    /// Returns whether CG access mode is dot access (mode1 bit 5).
    pub fn is_kac_dot_access_mode(&self) -> bool {
        self.state.video_mode & MODE1_KAC_MODE != 0
    }

    /// Returns whether graphics odd rasters are hidden (mode1 bit 4).
    pub fn is_hide_odd_rasters_enabled(&self) -> bool {
        self.state.video_mode & MODE1_HIDE_ODD_RASTERS != 0
    }

    /// Returns whether memory-switch writes are permitted (mode1 bit 6).
    pub fn is_memory_switch_write_enabled(&self) -> bool {
        self.state.video_mode & MODE1_NVMW_PERMIT != 0
    }

    /// Returns whether global display output is enabled (mode1 bit 7).
    pub fn is_display_enabled_global(&self) -> bool {
        self.state.video_mode & MODE1_DISP_ENABLE != 0
    }
}

#[cfg(test)]
mod tests {
    use super::DisplayControl;

    fn mode1_write(bit: u8, set: bool) -> u8 {
        (bit << 1) | u8::from(set)
    }

    fn mode2_base_write(bit: u8, set: bool) -> u8 {
        (bit << 1) | u8::from(set)
    }

    fn mode2_extended_write(bit: u8, set: bool) -> u8 {
        debug_assert!((8..=11).contains(&bit));
        0x80 | ((bit - 8) << 1) | u8::from(set)
    }

    #[test]
    fn mode1_helpers_follow_flip_flop_writes() {
        let mut display_control = DisplayControl::new();

        assert!(!display_control.is_attr_semigraphics_enabled());
        assert!(!display_control.is_text_40_columns());
        assert!(!display_control.is_font_8x16_mode());
        assert!(!display_control.is_hide_odd_rasters_enabled());
        assert!(!display_control.is_kac_dot_access_mode());

        display_control.write_video_mode(mode1_write(0, true));
        display_control.write_video_mode(mode1_write(2, true));
        display_control.write_video_mode(mode1_write(3, true));
        display_control.write_video_mode(mode1_write(4, true));
        display_control.write_video_mode(mode1_write(5, true));

        assert!(display_control.is_attr_semigraphics_enabled());
        assert!(display_control.is_text_40_columns());
        assert!(display_control.is_font_8x16_mode());
        assert!(display_control.is_hide_odd_rasters_enabled());
        assert!(display_control.is_kac_dot_access_mode());

        display_control.write_video_mode(mode1_write(0, false));
        display_control.write_video_mode(mode1_write(2, false));
        display_control.write_video_mode(mode1_write(3, false));
        display_control.write_video_mode(mode1_write(4, false));
        display_control.write_video_mode(mode1_write(5, false));

        assert!(!display_control.is_attr_semigraphics_enabled());
        assert!(!display_control.is_text_40_columns());
        assert!(!display_control.is_font_8x16_mode());
        assert!(!display_control.is_hide_odd_rasters_enabled());
        assert!(!display_control.is_kac_dot_access_mode());
    }

    #[test]
    fn mode2_egc_latch_requires_permission_and_hardware() {
        let mut dc = DisplayControl::new();

        // Bit 2 is always stored in mode2 regardless of permission.
        dc.write_mode2(mode2_base_write(2, true), true);
        assert!(dc.is_egc_extended_mode_requested());
        // But the latch is NOT set because bit 3 (permission) is clear.
        assert!(!dc.is_egc_extended_mode_effective());

        // Clear bit 2, enable permission (bit 3), then set bit 2 again.
        dc.write_mode2(mode2_base_write(2, false), true);
        dc.write_mode2(mode2_base_write(3, true), true);
        dc.write_mode2(mode2_base_write(2, true), true);
        assert!(dc.is_egc_extended_mode_effective());

        // Clearing bit 3 does NOT clear the latch.
        dc.write_mode2(mode2_base_write(3, false), true);
        assert!(!dc.is_egc_mode_change_permitted());
        assert!(dc.is_egc_extended_mode_effective());

        // Trying to clear bit 2 without permission: mode2 updates but
        // the latch stays set (bit 3 is clear, so latch is not touched).
        dc.write_mode2(mode2_base_write(2, false), true);
        assert!(!dc.is_egc_extended_mode_requested());
        assert!(dc.is_egc_extended_mode_effective());

        // Re-enable permission, then clear bit 2: latch now clears.
        dc.write_mode2(mode2_base_write(3, true), true);
        dc.write_mode2(mode2_base_write(2, false), true);
        assert!(!dc.is_egc_extended_mode_effective());
    }

    #[test]
    fn mode2_egc_latch_ignored_without_egc_hardware() {
        let mut dc = DisplayControl::new();

        // Full enable sequence, but has_egc_hardware = false.
        dc.write_mode2(mode2_base_write(3, true), false);
        dc.write_mode2(mode2_base_write(2, true), false);
        assert!(dc.is_egc_extended_mode_requested());
        assert!(dc.is_egc_mode_change_permitted());
        // Latch is NOT set because no EGC hardware.
        assert!(!dc.is_egc_extended_mode_effective());
    }

    #[test]
    fn mode2_extended_page_controls_5mhz_detection() {
        let mut display_control = DisplayControl::new();
        assert!(!display_control.is_gdc_5mhz());

        display_control.write_mode2(mode2_extended_write(9, true), false);
        assert!(!display_control.is_gdc_5mhz());

        display_control.write_mode2(mode2_extended_write(10, true), false);
        assert!(display_control.is_gdc_5mhz());

        display_control.write_mode2(mode2_extended_write(9, false), false);
        assert!(!display_control.is_gdc_5mhz());
    }
}
