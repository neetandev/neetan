//! INT 18h AH=30h and AH=31h: the extended screen mode of the PC-9821 normal mode.
//!
//! The functions follow undoc98 `int18crt.txt` (Function 30h and 31h) with its table of the
//! valid mode combinations, and the work areas of undoc98 `memsys.txt` (0459h, 053Ch, 054Dh
//! and 0597h). The display timings follow the scan frequencies of that table: 31.47 kHz lines
//! of 800 dots, 449 lines per frame at 70.10 Hz and 525 lines per frame at 59.94 Hz.

use common::Cpu;

use super::{
    super::Pc9801Bus,
    crt::{CELL_RASTERS_400_LINE, CRT_STATUS_ADDRESS},
    graphics::{
        FULL_SCREEN_LINE_COUNT, GRAPHICS_RESOLUTION_ADDRESS, PRXCRT_31KHZ, PRXCRT_ADDRESS,
        PRXCRT_DIP_SWITCH_1_8, PRXDUPD_5MHZ_ACTIVE, PRXDUPD_5MHZ_ALLOWED, PRXDUPD_ADDRESS,
        PRXDUPD_EXTENDED_GRAPHICS, PRXDUPD_FLASH_DRAW, RESOLUTION_480_LINE, RESOLUTION_ALL,
        RESOLUTION_LOWER, RESOLUTION_MASK, RESOLUTION_UPPER, SETTING_AREA_ALL, SETTING_AREA_LOWER,
        SETTING_AREA_SHIFT, SETTING_AREA_UPPER, SETTING_DISPLAY_BANK, SETTING_MONOCHROME,
        SYNC_MODE_GRAPHICS, SYNC_TIMING_2_5MHZ, SYNC_TIMING_5MHZ, SyncTiming,
    },
};
use crate::TraceSink;

/// Work area byte 0597h bit 7: INT 18h AH=30h and AH=31h are supported.
const EXTENDED_SCREEN_SUPPORTED: u8 = 0x80;

/// Work area byte with the extended CRT status (CRT_EXT_STS).
const CRT_EXTENDED_STATUS_ADDRESS: usize = 0x0459;
/// CRT_EXT_STS bit 3: non-interlaced raster scan, the inverse of AL bit 0 of AH=30h and AH=31h.
const EXTENDED_STATUS_NON_INTERLACED: u8 = 0x08;
/// CRT_EXT_STS bit 0: the CRT shows 640x480.
const EXTENDED_STATUS_480_LINE: u8 = 0x01;

/// CRT_STS_FLAG bit 0: 20 or 30 text rows.
const STATUS_ROWS_20_OR_30: u8 = 0x01;
/// CRT_STS_FLAG bit 4: 30 text rows.
const STATUS_ROWS_30: u8 = 0x10;

/// AL bits that must be zero on a PC-9821: bits 7-4 and the external sync bit 1.
const RATE_RESERVED_BITS: u8 = 0xF2;
/// AL bit 0: interlaced raster scan.
const RATE_INTERLACED: u8 = 0x01;
/// AL bits 3-2 for a 24.83 kHz horizontal scan frequency.
const RATE_24KHZ: u8 = 0x08;
/// AL bits 3-2 for a 31.47 kHz horizontal scan frequency.
const RATE_31KHZ: u8 = 0x0C;
/// AL bits 3-2: the horizontal scan frequency field.
const RATE_FREQUENCY_MASK: u8 = 0x0C;

/// BH bits that must be zero: bits 7-6 and bits 3-2.
const SCREEN_RESERVED_BITS: u8 = 0xCC;
/// Shift of the graphics resolution field (BH bits 5-4).
const SCREEN_RESOLUTION_SHIFT: u8 = 4;
/// BH bits 1-0: the text row field.
const SCREEN_ROWS_MASK: u8 = 0x03;
/// Text row field value for 20 rows.
pub(super) const ROWS_20: u8 = 0b00;
/// Text row field value for 25 rows.
pub(super) const ROWS_25: u8 = 0b01;
/// Text row field value for 30 rows.
pub(super) const ROWS_30: u8 = 0b10;

/// AL and BH of a mode the hardware hangs on: 31.47 kHz interlaced with 640x480 and 25 rows.
const FORBIDDEN_MODE: (u8, u8) = (0x0D, 0x31);

/// AL and BH result value of a successful call.
const RESULT_SUCCESS: u8 = 0x00;
/// AL and BH result value of a call with an invalid parameter.
const RESULT_ERROR: u8 = 0x01;

/// SYNC mode byte of the text GDC: mixed display mode, drawing during the retrace.
const SYNC_MODE_TEXT: u8 = 0x10;

/// Display lines of the 640x400 screen.
pub(super) const LINES_400: u16 = 400;
/// Display lines of the 640x480 screen.
pub(super) const LINES_480: u16 = 480;
/// Text GDC words per line.
const TEXT_PITCH: u16 = 80;

/// Text GDC timing at 24.83 kHz with 400 lines.
const SYNC_TIMING_TEXT_24KHZ: SyncTiming = SyncTiming {
    words_per_line: 0x4E,
    horizontal_sync: 0x07,
    horizontal_front_porch: 0x09,
    horizontal_back_porch: 0x07,
    vertical_sync: 0x08,
    vertical_front_porch: 0x07,
    vertical_back_porch: 0x19,
    active_lines: 400,
};

/// GDC timing at 31.47 kHz with 400 lines: 100 characters of 8 dots and 449 lines per frame.
const SYNC_TIMING_31KHZ_400_LINE: SyncTiming = SyncTiming {
    words_per_line: 0x4E,
    horizontal_sync: 0x07,
    horizontal_front_porch: 0x03,
    horizontal_back_porch: 0x07,
    vertical_sync: 0x02,
    vertical_front_porch: 0x0D,
    vertical_back_porch: 0x22,
    active_lines: 400,
};

/// Graphics GDC timing at 2.5 MHz and 31.47 kHz with 400 lines: 50 words of 16 dots per line.
const SYNC_TIMING_31KHZ_400_LINE_2_5MHZ: SyncTiming = SyncTiming {
    words_per_line: 0x26,
    horizontal_sync: 0x01,
    horizontal_front_porch: 0x03,
    horizontal_back_porch: 0x03,
    vertical_sync: 0x02,
    vertical_front_porch: 0x0D,
    vertical_back_porch: 0x22,
    active_lines: 400,
};

/// GDC timing at 31.47 kHz with 480 lines: 100 characters of 8 dots and 525 lines per frame.
const SYNC_TIMING_31KHZ_480_LINE: SyncTiming = SyncTiming {
    words_per_line: 0x4E,
    horizontal_sync: 0x0B,
    horizontal_front_porch: 0x03,
    horizontal_back_porch: 0x03,
    vertical_sync: 0x02,
    vertical_front_porch: 0x06,
    vertical_back_porch: 0x25,
    active_lines: 480,
};

/// Returns `timing` with `active_lines` shown and the remaining lines of the screen added to the
/// vertical front porch, so the frame keeps its line count.
const fn with_active_lines(timing: SyncTiming, active_lines: u16) -> SyncTiming {
    SyncTiming {
        vertical_front_porch: timing.vertical_front_porch
            + (timing.active_lines - active_lines) as u8,
        active_lines,
        ..timing
    }
}

/// Returns the AH=42h setting with the VRAM area of the graphics `resolution`.
const fn display_area_setting(resolution: u8) -> u8 {
    let area = match resolution {
        RESOLUTION_UPPER => SETTING_AREA_UPPER,
        RESOLUTION_LOWER => SETTING_AREA_LOWER,
        _ => SETTING_AREA_ALL,
    };
    area << SETTING_AREA_SHIFT
}

/// Returns whether the PC-9821 supports the mode of AL `rate` and BH `screen`.
const fn extended_screen_mode_valid(rate: u8, screen: u8) -> bool {
    if rate & RATE_RESERVED_BITS != 0 || screen & SCREEN_RESERVED_BITS != 0 {
        return false;
    }
    if rate == FORBIDDEN_MODE.0 && screen == FORBIDDEN_MODE.1 {
        return false;
    }
    let is_31khz = match rate & RATE_FREQUENCY_MASK {
        RATE_31KHZ => true,
        RATE_24KHZ => false,
        _ => return false,
    };
    let is_480_line = screen >> SCREEN_RESOLUTION_SHIFT == RESOLUTION_480_LINE;
    match screen & SCREEN_ROWS_MASK {
        ROWS_20 | ROWS_25 => !is_480_line || is_31khz,
        ROWS_30 => is_480_line && is_31khz,
        _ => false,
    }
}

impl<T: TraceSink> Pc9801Bus<T> {
    /// AH=30h: sets the horizontal scan frequency, the graphics resolution and the text rows.
    pub(super) fn int18h_extended_screen_mode_set(&mut self, cpu: &mut impl Cpu) {
        if !self.extended_screen_supported() {
            return;
        }
        let rate = cpu.al();
        let screen = cpu.bh();
        let result = if extended_screen_mode_valid(rate, screen) {
            let flags = self.extended_screen_current_area_flags();
            self.extended_screen_mode_apply(rate, screen, flags);
            RESULT_SUCCESS
        } else {
            RESULT_ERROR
        };
        cpu.set_al(result);
        cpu.set_bh(result);
    }

    /// AH=31h: returns the scan frequency in AL and the graphics resolution and text rows in BH.
    pub(super) fn int18h_extended_screen_mode_get(&mut self, cpu: &mut impl Cpu) {
        if !self.extended_screen_supported() {
            return;
        }
        let ram = &self.memory.state.ram;
        let resolution = ram[GRAPHICS_RESOLUTION_ADDRESS] & RESOLUTION_MASK;
        cpu.set_al(self.extended_screen_current_rate());
        cpu.set_bh((resolution << SCREEN_RESOLUTION_SHIFT) | self.extended_screen_current_rows());
    }

    /// Returns whether the work area reports INT 18h AH=30h and AH=31h as supported.
    fn extended_screen_supported(&self) -> bool {
        self.memory.state.ram[GRAPHICS_RESOLUTION_ADDRESS] & EXTENDED_SCREEN_SUPPORTED != 0
    }

    /// Returns whether the CRT shows 640x480 (CRT_EXT_STS bit 0).
    pub(super) fn crt_480_line_mode(&self) -> bool {
        self.memory.state.ram[CRT_EXTENDED_STATUS_ADDRESS] & EXTENDED_STATUS_480_LINE != 0
    }

    /// Returns the AL value of AH=31h: the scan frequency and the raster scan mode.
    fn extended_screen_current_rate(&self) -> u8 {
        let frequency = if self.display_control.is_crt_31khz_enabled() {
            RATE_31KHZ
        } else {
            RATE_24KHZ
        };
        let extended_status = self.memory.state.ram[CRT_EXTENDED_STATUS_ADDRESS];
        if extended_status & EXTENDED_STATUS_NON_INTERLACED != 0 {
            frequency
        } else {
            frequency | RATE_INTERLACED
        }
    }

    /// Returns the text row field of CRT_STS_FLAG.
    fn extended_screen_current_rows(&self) -> u8 {
        let status = self.memory.state.ram[CRT_STATUS_ADDRESS];
        if status & STATUS_ROWS_30 != 0 {
            ROWS_30
        } else if status & STATUS_ROWS_20_OR_30 != 0 {
            ROWS_20
        } else {
            ROWS_25
        }
    }

    /// Returns the AH=42h monochrome and display bank bits of the current graphics screen.
    fn extended_screen_current_area_flags(&self) -> u8 {
        let mut flags = 0;
        if self.display_control.is_graphics_monochrome() {
            flags |= SETTING_MONOCHROME;
        }
        if self.display_page_index() != 0 {
            flags |= SETTING_DISPLAY_BANK;
        }
        flags
    }

    /// AH=42h in 640x480: selects the 640x400 or 640x200 area of `setting` at the current scan
    /// frequency. 30 text rows become 20 rows. The text display keeps its state.
    pub(super) fn extended_screen_leave_480_line(&mut self, setting: u8) {
        let resolution = match setting >> SETTING_AREA_SHIFT {
            SETTING_AREA_UPPER => RESOLUTION_UPPER,
            SETTING_AREA_LOWER => RESOLUTION_LOWER,
            SETTING_AREA_ALL => RESOLUTION_ALL,
            _ => return,
        };
        let rows = match self.extended_screen_current_rows() {
            ROWS_30 => ROWS_20,
            rows => rows,
        };
        let rate = self.extended_screen_current_rate();
        let text_enabled = self.gdc_master.state.display_enabled;
        let flags = setting & (SETTING_MONOCHROME | SETTING_DISPLAY_BANK);
        self.extended_screen_mode_apply(
            rate,
            (resolution << SCREEN_RESOLUTION_SHIFT) | rows,
            flags,
        );
        self.gdc_master.state.display_enabled = text_enabled;
    }

    /// Programs the display for a valid mode of AL `rate` and BH `screen`. `area_flags` holds
    /// the AH=42h monochrome and display bank bits for the 640x400 and 640x200 resolutions.
    fn extended_screen_mode_apply(&mut self, rate: u8, screen: u8, area_flags: u8) {
        let is_31khz = rate & RATE_FREQUENCY_MASK == RATE_31KHZ;
        let resolution = screen >> SCREEN_RESOLUTION_SHIFT;
        let is_480_line = resolution == RESOLUTION_480_LINE;
        let rows = screen & SCREEN_ROWS_MASK;

        self.display_control.set_crt_31khz_enabled(is_31khz);
        let ram = &mut self.memory.state.ram;
        ram[PRXCRT_ADDRESS] &= !PRXCRT_31KHZ;
        if is_31khz {
            ram[PRXCRT_ADDRESS] |= PRXCRT_31KHZ;
        }
        let extended_status = &mut ram[CRT_EXTENDED_STATUS_ADDRESS];
        *extended_status &= !(EXTENDED_STATUS_NON_INTERLACED | EXTENDED_STATUS_480_LINE);
        if rate & RATE_INTERLACED == 0 {
            *extended_status |= EXTENDED_STATUS_NON_INTERLACED;
        }
        if is_480_line {
            *extended_status |= EXTENDED_STATUS_480_LINE;
        }

        self.extended_screen_set_text(is_31khz, is_480_line, rows);
        self.gdc_master.state.pitch = TEXT_PITCH;
        self.crt_set_single_area(0);
        self.extended_screen_set_graphics(is_31khz, resolution, area_flags);

        self.apply_gdc_dot_clock();
        self.reschedule_gdc_events();
    }

    /// Sets the text GDC timing, the text rows and the line counter, and hides the text screen
    /// and the cursor.
    pub(super) fn extended_screen_set_text(&mut self, is_31khz: bool, is_480_line: bool, rows: u8) {
        let (screen_lines, row_count) = match (is_480_line, rows) {
            (true, ROWS_20) => (LINES_480, 20),
            (true, ROWS_25) => (LINES_480, 25),
            (true, _) => (LINES_480, 30),
            (false, ROWS_20) => (LINES_400, 20),
            (false, _) => (LINES_400, 25),
        };
        let row_rasters = (screen_lines / row_count) as u8;
        let base_timing = match (is_31khz, is_480_line) {
            (true, true) => SYNC_TIMING_31KHZ_480_LINE,
            (true, false) => SYNC_TIMING_31KHZ_400_LINE,
            (false, _) => SYNC_TIMING_TEXT_24KHZ,
        };
        let timing = with_active_lines(base_timing, row_count * u16::from(row_rasters));
        self.gdc_master
            .load_sync_params(&timing.sync_parameters(SYNC_MODE_TEXT));

        let status = &mut self.memory.state.ram[CRT_STATUS_ADDRESS];
        *status &= !(STATUS_ROWS_20_OR_30 | STATUS_ROWS_30);
        match rows {
            ROWS_20 => *status |= STATUS_ROWS_20_OR_30,
            ROWS_30 => *status |= STATUS_ROWS_20_OR_30 | STATUS_ROWS_30,
            _ => {}
        }
        self.crt_set_row_rasters(row_rasters, CELL_RASTERS_400_LINE);
        self.crt_set_cursor_form(false);
        self.gdc_master.state.display_enabled = false;
    }

    /// Selects the graphics resolution with its GDC timing, GDC clock and PEGC screen layout. The
    /// graphics GDC runs at 5 MHz in 640x480 and in 640x400 when DIP SW 2-8 allows it, and at
    /// 2.5 MHz in 640x200 (undoc98 `memsys.txt` 0000:054Dh bit 2).
    fn extended_screen_set_graphics(&mut self, is_31khz: bool, resolution: u8, area_flags: u8) {
        if resolution == RESOLUTION_480_LINE {
            self.pegc.set_256_color_enabled(true);
            self.pegc.set_screen_mode(true);
            let ram = &mut self.memory.state.ram;
            ram[PRXDUPD_ADDRESS] |= PRXDUPD_EXTENDED_GRAPHICS;
            if ram[PRXCRT_ADDRESS] & PRXCRT_DIP_SWITCH_1_8 != 0 {
                ram[PRXDUPD_ADDRESS] |= PRXDUPD_FLASH_DRAW;
            }
            let saved = &mut ram[GRAPHICS_RESOLUTION_ADDRESS];
            *saved = (*saved & !RESOLUTION_MASK) | RESOLUTION_480_LINE;
            self.update_plane_e_mapping();
            self.graphics_set_gdc_clock(true);

            self.gdc_slave
                .load_sync_params(&SYNC_TIMING_31KHZ_480_LINE.sync_parameters(SYNC_MODE_GRAPHICS));
            self.gdc_slave.state.lines_per_row = 1;
            let scroll = &mut self.gdc_slave.state.scroll[0];
            scroll.start_address = 0;
            scroll.line_count = FULL_SCREEN_LINE_COUNT;
            scroll.im = false;
            scroll.wd = false;
            return;
        }

        self.pegc.set_screen_mode(false);
        if resolution == RESOLUTION_UPPER || resolution == RESOLUTION_LOWER {
            self.pegc.set_256_color_enabled(false);
            self.memory.state.ram[PRXDUPD_ADDRESS] &= !PRXDUPD_EXTENDED_GRAPHICS;
        }
        self.update_plane_e_mapping();
        let five_mhz_allowed = self.memory.state.ram[PRXDUPD_ADDRESS] & PRXDUPD_5MHZ_ALLOWED != 0;
        self.graphics_set_gdc_clock(resolution == RESOLUTION_ALL && five_mhz_allowed);
        self.graphics_set_display_area(display_area_setting(resolution) | area_flags);

        let is_5mhz = self.memory.state.ram[PRXDUPD_ADDRESS] & PRXDUPD_5MHZ_ACTIVE != 0;
        let timing = match (is_31khz, is_5mhz) {
            (true, true) => SYNC_TIMING_31KHZ_400_LINE,
            (true, false) => SYNC_TIMING_31KHZ_400_LINE_2_5MHZ,
            (false, true) => SYNC_TIMING_5MHZ,
            (false, false) => SYNC_TIMING_2_5MHZ,
        };
        self.gdc_slave
            .load_sync_params(&timing.sync_parameters(SYNC_MODE_GRAPHICS));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_screen_mode_valid_follows_the_pc9821_table() {
        let cases: &[(u8, u8, bool)] = &[
            (0x08, 0x00, true),
            (0x08, 0x01, true),
            (0x08, 0x10, true),
            (0x08, 0x11, true),
            (0x08, 0x20, true),
            (0x08, 0x21, true),
            (0x08, 0x30, false),
            (0x08, 0x31, false),
            (0x08, 0x22, false),
            (0x09, 0x21, true),
            (0x0C, 0x21, true),
            (0x0C, 0x30, true),
            (0x0C, 0x31, true),
            (0x0C, 0x32, true),
            (0x0C, 0x22, false),
            (0x0C, 0x33, false),
            (0x0D, 0x30, true),
            (0x0D, 0x31, false),
            (0x0D, 0x32, true),
            (0x00, 0x21, false),
            (0x04, 0x21, false),
            (0x01, 0x00, false),
            (0x0A, 0x21, false),
            (0x18, 0x21, false),
            (0x0C, 0x41, false),
            (0x0C, 0x35, false),
            (0x0C, 0x39, false),
        ];
        for &(rate, screen, valid) in cases {
            assert_eq!(
                extended_screen_mode_valid(rate, screen),
                valid,
                "AL={rate:02X}h BH={screen:02X}h"
            );
        }
    }
}
