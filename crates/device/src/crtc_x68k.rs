//! Sharp X68000 CRTC register and beam-timing state.

/// X68000 CRTC oscillator frequencies.
pub const CRTC_X68K_OSCILLATOR_HZ: [u32; 3] = [38_863_632, 69_551_900, 50_349_800];

/// Oscillator selected by the HRL and R20 clock-selection bits.
const OSCILLATOR_INDEX: [usize; 16] = [0, 0, 0, 0, 1, 1, 1, 2, 0, 0, 0, 0, 1, 1, 1, 2];
/// Dot-clock divisor selected by the HRL and R20 clock-selection bits.
const CLOCK_DIVISOR: [u8; 16] = [8, 4, 8, 8, 6, 3, 2, 2, 8, 4, 8, 8, 8, 4, 2, 2];

/// Kind of CRTC state changed by a register write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CrtcChangeX68k {
    /// Timing or visible geometry changed.
    pub timing: bool,
    /// Selected oscillator or divider changed.
    pub clock: bool,
    /// Text scroll changed.
    pub text_scroll: bool,
    /// Display or storage mode changed.
    pub display: bool,
}

save_state::runtime_state! {
/// CRTC output signal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrtcSignalsX68k {
    /// Horizontal-sync output.
    pub horizontal_sync: bool,
    /// Vertical display-period output.
    pub vertical_display: bool,
    /// Active-low raster interrupt output.
    pub raster_interrupt: bool,
}}

/// Current CRTC beam position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrtcBeamPositionX68k {
    /// Current raster number, with zero at vertical sync.
    pub raster: u16,
    /// Current character column.
    pub column: u16,
    /// Dot position within the current character column.
    pub dot: u8,
    /// Current field parity.
    pub odd_field: bool,
}

/// Visible frame geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrtcGeometryX68k {
    /// Visible width in pixels.
    pub width: u32,
    /// Visible height in pixels.
    pub height: u32,
    /// First visible raster.
    pub first_raster: u16,
    /// First visible character column.
    pub first_column: u16,
}

/// Signals changed while advancing the CRTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CrtcTransitionsX68k {
    /// A new frame began.
    pub frame_started: bool,
    /// The vertical display period began.
    pub vertical_display_started: bool,
    /// One or more output pins changed.
    pub signals_changed: bool,
    /// Horizontal front porches passed with the raster-copy switch on.
    pub raster_copies: u16,
}

/// Vertical scan class selected by the R20 frequency and resolution bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrtcScanClassX68k {
    /// One visible raster per content line.
    Normal,
    /// 31 kHz scan of 256-line content reading each content line on two rasters.
    DoubleRead,
    /// Two half-height fields carrying alternating content lines.
    Interlace,
}

/// Graphics VRAM memory mode selected by CRTC R20.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GvramModeX68k {
    /// 512x512 16-color mode with four page windows.
    Colors16,
    /// 512x512 256-color mode with two page-pair windows.
    Colors256,
    /// Undocumented memory mode 2 with pair-packed access.
    MemoryMode2,
    /// 512x512 65536-color mode with one word page.
    Colors65536,
    /// 1024x1024 16-color mode with quadrant addressing.
    Colors16Virtual1024,
}

save_state::runtime_state! {
/// Sharp X68000 CRTC.
#[derive(Debug, Clone)]
pub struct CrtcX68k {
    registers: [u16; 24],
    operation: u16,
    high_speed_clear_armed: bool,
    high_resolution_clock: bool,
    raster: u16,
    column: u16,
    phase_ticks: u64,
    frame_count: u64,
    odd_field: bool,
    signals: CrtcSignalsX68k,
}}

impl CrtcX68k {
    /// Captures complete CRTC register and beam timing state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores complete CRTC register and beam timing state.
    pub fn restore_state(&mut self, state: Self) {
        *self = state;
    }
}

impl Default for CrtcX68k {
    fn default() -> Self {
        Self::new()
    }
}

impl CrtcX68k {
    /// Creates cleared CRTC state.
    pub fn new() -> Self {
        let mut registers = [0; 24];
        registers[9] = 0x03FF;
        Self {
            registers,
            operation: 0,
            high_speed_clear_armed: false,
            high_resolution_clock: false,
            raster: 0,
            column: 0,
            phase_ticks: 0,
            frame_count: 0,
            odd_field: false,
            signals: CrtcSignalsX68k {
                horizontal_sync: false,
                vertical_display: false,
                raster_interrupt: true,
            },
        }
    }

    /// Resets CRTC state.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Reads CRTC register R00-R23.
    pub fn read_register(&self, index: usize) -> u16 {
        self.registers[index % 24]
    }

    /// Writes CRTC register R00-R23. R00 bit 0 is hard-wired to 1.
    pub fn write_register(&mut self, index: usize, value: u16) -> CrtcChangeX68k {
        let index = index % 24;
        let old_clock = self.clock_key();
        self.registers[index] = value & REGISTER_MASKS[index];
        if index == 0 {
            self.registers[0] |= 0x0001;
        }
        self.refresh_signals();
        CrtcChangeX68k {
            timing: index <= 9,
            clock: old_clock != self.clock_key(),
            text_scroll: matches!(index, 10 | 11),
            display: matches!(index, 12..=20),
        }
    }

    /// Reads the operation port.
    pub const fn read_operation(&self) -> u16 {
        self.operation
    }

    /// Writes the operation port, holding the raster-copy switch and arming
    /// the high-speed clear. The unimplemented image input never latches.
    pub fn write_operation(&mut self, value: u16) {
        self.operation = value & 0x0008 | self.operation & 0x0002;
        if value & 0x0002 != 0 {
            self.high_speed_clear_armed = true;
        }
    }

    /// Returns whether a high-speed clear is armed and waits for display start.
    pub const fn high_speed_clear_requested(&self) -> bool {
        self.high_speed_clear_armed
    }

    /// Starts the armed high-speed clear; the operation port reads it back.
    pub fn begin_high_speed_clear(&mut self) {
        self.high_speed_clear_armed = false;
        self.operation |= 0x0002;
    }

    /// Returns whether a high-speed clear frame is in progress.
    pub const fn high_speed_clear_active(&self) -> bool {
        self.operation & 0x0002 != 0
    }

    /// Ends the high-speed clear frame and clears the read-back bit.
    pub fn complete_high_speed_clear(&mut self) {
        self.operation &= !0x0002;
    }

    /// Selects the HRL system-port input.
    pub fn set_hrl(&mut self, enabled: bool) -> CrtcChangeX68k {
        let old_clock = self.clock_key();
        self.high_resolution_clock = enabled;
        CrtcChangeX68k {
            clock: old_clock != self.clock_key(),
            ..CrtcChangeX68k::default()
        }
    }

    /// Advances by oscillator ticks in the currently selected clock domain.
    pub fn advance_oscillator_ticks(&mut self, mut ticks: u64) -> CrtcTransitionsX68k {
        let mut result = CrtcTransitionsX68k::default();
        while ticks != 0 {
            let Some(until) = self.ticks_until_transition() else {
                break;
            };
            if ticks < until {
                self.phase_ticks += ticks;
                break;
            }
            ticks -= until;
            self.phase_ticks = 0;
            let old = self.signals;
            if self.next_boundary() == u16::from(self.registers[0] as u8) + 1 {
                self.column = 0;
                self.advance_raster(&mut result);
            } else {
                self.column = self.next_boundary();
                if self.column == self.registers[3] + 5 && self.operation & 0x0008 != 0 {
                    result.raster_copies += 1;
                }
            }
            self.refresh_signals();
            result.vertical_display_started |=
                !old.vertical_display && self.signals.vertical_display;
            result.signals_changed |= old != self.signals;
        }
        result
    }

    /// Returns oscillator ticks until the next significant transition.
    pub fn ticks_until_transition(&self) -> Option<u64> {
        let column_ticks = self.column_ticks()?;
        let columns = u64::from(self.next_boundary().saturating_sub(self.column).max(1));
        Some(columns * column_ticks - self.phase_ticks.min(columns * column_ticks - 1))
    }

    /// Returns current output signals.
    pub const fn signals(&self) -> CrtcSignalsX68k {
        self.signals
    }

    /// Returns the current beam position.
    pub fn beam_position(&self) -> CrtcBeamPositionX68k {
        let divisor = u64::from(self.clock_divisor());
        CrtcBeamPositionX68k {
            raster: self.raster,
            column: self.column,
            dot: (self.phase_ticks / divisor).min(7) as u8,
            odd_field: self.odd_field,
        }
    }

    /// Returns visible geometry when the timing registers are valid.
    pub fn frame_geometry(&self) -> Option<CrtcGeometryX68k> {
        let start_column = self.registers[2] + 5;
        let end_column = self.registers[3] + 5;
        let start_raster = self.registers[6] + 1;
        let end_raster = self.registers[7] + 1;
        if self.registers[0] < end_column
            || self.registers[4] < end_raster
            || end_column <= start_column
            || end_raster <= start_raster
        {
            return None;
        }
        Some(CrtcGeometryX68k {
            width: u32::from(end_column - start_column) * 8,
            height: u32::from(end_raster - start_raster),
            first_raster: start_raster,
            first_column: start_column,
        })
    }

    /// Returns the selected oscillator frequency.
    pub fn oscillator_hz(&self) -> u32 {
        CRTC_X68K_OSCILLATOR_HZ[OSCILLATOR_INDEX[self.clock_key()]]
    }

    /// Returns the selected oscillator divider.
    pub fn clock_divisor(&self) -> u8 {
        CLOCK_DIVISOR[self.clock_key()]
    }

    /// Returns the completed frame count.
    pub const fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Returns text horizontal scroll.
    pub const fn text_scroll_x(&self) -> u16 {
        self.registers[10]
    }

    /// Returns text vertical scroll.
    pub const fn text_scroll_y(&self) -> u16 {
        self.registers[11]
    }

    /// Returns the vertical scan class selected by R20.
    pub const fn scan_class(&self) -> CrtcScanClassX68k {
        let high_frequency = self.registers[20] & 0x0010 != 0;
        let vertical_resolution = (self.registers[20] >> 2) & 3;
        if high_frequency && vertical_resolution == 0 {
            CrtcScanClassX68k::DoubleRead
        } else if (!high_frequency && vertical_resolution >= 1)
            || (high_frequency && vertical_resolution >= 2)
        {
            CrtcScanClassX68k::Interlace
        } else {
            CrtcScanClassX68k::Normal
        }
    }

    /// Returns whether text storage mode hides the text layer.
    pub const fn text_storage_enabled(&self) -> bool {
        self.registers[20] & 0x1000 != 0
    }

    /// Returns whether the sprite RAM is reachable in the current mode.
    pub const fn sprite_area_accessible(&self) -> bool {
        self.registers[20] & 0b1_0010 != 0b1_0010
    }

    /// Returns the graphics VRAM memory mode from R20.
    pub const fn graphic_memory_mode(&self) -> GvramModeX68k {
        match (self.registers[20] >> 8) & 7 {
            0 => GvramModeX68k::Colors16,
            1 => GvramModeX68k::Colors256,
            2 => GvramModeX68k::MemoryMode2,
            3 => GvramModeX68k::Colors65536,
            _ => GvramModeX68k::Colors16Virtual1024,
        }
    }

    /// Returns whether graphics VRAM is switched to storage mode.
    pub const fn graphic_storage_enabled(&self) -> bool {
        self.registers[20] & 0x0800 != 0
    }

    /// Returns graphic horizontal scroll for one page.
    pub const fn graphic_scroll_x(&self, page: usize) -> u16 {
        self.registers[12 + (page & 3) * 2]
    }

    /// Returns graphic vertical scroll for one page.
    pub const fn graphic_scroll_y(&self, page: usize) -> u16 {
        self.registers[13 + (page & 3) * 2]
    }

    fn clock_key(&self) -> usize {
        usize::from(self.high_resolution_clock) << 3
            | usize::from((self.registers[20] >> 4) & 1) << 2
            | usize::from(self.registers[20] & 3)
    }

    fn column_ticks(&self) -> Option<u64> {
        self.frame_geometry()?;
        Some(u64::from(self.clock_divisor()) * 8)
    }

    fn next_boundary(&self) -> u16 {
        let total = self.registers[0] + 1;
        [
            self.registers[1] + 1,
            self.registers[2] + 5,
            self.registers[3] + 5,
            total,
        ]
        .into_iter()
        .filter(|&boundary| boundary > self.column && boundary <= total)
        .min()
        .unwrap_or(total)
    }

    fn advance_raster(&mut self, result: &mut CrtcTransitionsX68k) {
        self.raster += 1;
        if self.raster > self.registers[4] {
            self.raster = 0;
            self.frame_count += 1;
            self.odd_field = !self.odd_field;
            result.frame_started = true;
        }
    }

    fn refresh_signals(&mut self) {
        self.signals.horizontal_sync = self.column <= self.registers[1];
        self.signals.vertical_display =
            self.raster > self.registers[6] && self.raster <= self.registers[7];
        self.signals.raster_interrupt = self.raster != self.registers[9];
    }
}

/// Writable-bit masks for CRTC registers R00-R23.
const REGISTER_MASKS: [u16; 24] = [
    0x00FF, 0x00FF, 0x00FF, 0x00FF, 0x03FF, 0x03FF, 0x03FF, 0x03FF, 0x03FF, 0x03FF, 0x03FF, 0x03FF,
    0x03FF, 0x03FF, 0x01FF, 0x01FF, 0x01FF, 0x01FF, 0x01FF, 0x01FF, 0x1F1F, 0x03FF, 0xFFFF, 0xFFFF,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn standard_crtc() -> CrtcX68k {
        let mut crtc = CrtcX68k::new();
        for (index, value) in [137, 14, 28, 124, 567, 5, 40, 552, 27, 100]
            .into_iter()
            .enumerate()
        {
            crtc.write_register(index, value);
        }
        crtc.write_register(20, 0x0016);
        crtc
    }

    #[test]
    fn standard_geometry_and_clock_match_the_book() {
        let crtc = standard_crtc();
        assert_eq!(crtc.frame_geometry().unwrap().width, 768);
        assert_eq!(crtc.frame_geometry().unwrap().height, 512);
        assert_eq!(crtc.oscillator_hz(), 69_551_900);
        assert_eq!(crtc.clock_divisor(), 2);
    }

    #[test]
    fn oscillator_selection_covers_all_three_crystals() {
        let mut crtc = CrtcX68k::new();
        assert_eq!(crtc.oscillator_hz(), 38_863_632);
        assert_eq!(crtc.clock_divisor(), 8);
        crtc.write_register(20, 0x0016);
        assert_eq!(crtc.oscillator_hz(), 69_551_900);
        assert_eq!(crtc.clock_divisor(), 2);
        crtc.write_register(20, 0x0013);
        assert_eq!(crtc.oscillator_hz(), 50_349_800);
        assert_eq!(crtc.clock_divisor(), 2);
    }

    #[test]
    fn scan_class_follows_r20_frequency_and_resolution_bits() {
        let cases = [
            (0x0000, CrtcScanClassX68k::Normal),
            (0x0014, CrtcScanClassX68k::Normal),
            (0x0010, CrtcScanClassX68k::DoubleRead),
            (0x0011, CrtcScanClassX68k::DoubleRead),
            (0x0004, CrtcScanClassX68k::Interlace),
            (0x0008, CrtcScanClassX68k::Interlace),
            (0x000C, CrtcScanClassX68k::Interlace),
            (0x0018, CrtcScanClassX68k::Interlace),
            (0x001C, CrtcScanClassX68k::Interlace),
        ];
        let mut crtc = CrtcX68k::new();
        for (value, class) in cases {
            crtc.write_register(20, value);
            assert_eq!(crtc.scan_class(), class, "R20 = {value:#06X}");
        }
    }

    #[test]
    fn hrl_changes_three_and_six_dividers() {
        let mut crtc = standard_crtc();
        crtc.write_register(20, 0x0015);
        assert_eq!(crtc.clock_divisor(), 3);
        assert!(crtc.set_hrl(true).clock);
        assert_eq!(crtc.clock_divisor(), 4);
    }

    #[test]
    fn raster_and_frame_signals_advance() {
        let mut crtc = standard_crtc();
        let line_ticks = u64::from(crtc.read_register(0) + 1) * u64::from(crtc.clock_divisor()) * 8;
        crtc.advance_oscillator_ticks(line_ticks * 100);
        assert_eq!(crtc.beam_position().raster, 100);
        assert!(!crtc.signals().raster_interrupt);
        let transitions = crtc.advance_oscillator_ticks(line_ticks * 468);
        assert!(transitions.frame_started);
        assert_eq!(crtc.frame_count(), 1);
    }

    #[test]
    fn r00_bit_zero_is_hard_wired_to_one() {
        let mut crtc = CrtcX68k::new();
        crtc.write_register(0, 68);
        assert_eq!(crtc.read_register(0), 69);
        crtc.write_register(0, 69);
        assert_eq!(crtc.read_register(0), 69);
        crtc.write_register(1, 68);
        assert_eq!(crtc.read_register(1), 68);
    }

    #[test]
    fn operation_port_holds_the_raster_copy_switch_only() {
        let mut crtc = CrtcX68k::new();
        crtc.write_operation(0x09);
        // Bit 3 is a level switch and stays readable until cleared; the
        // unimplemented image-input bit never latches.
        assert_eq!(crtc.read_operation(), 0x0008);
        crtc.write_operation(0x00);
        assert_eq!(crtc.read_operation(), 0);
    }

    #[test]
    fn raster_copy_switch_counts_every_front_porch() {
        let mut crtc = standard_crtc();
        let line_ticks = u64::from(crtc.read_register(0) + 1) * u64::from(crtc.clock_divisor()) * 8;
        crtc.write_operation(0x0008);
        let transitions = crtc.advance_oscillator_ticks(line_ticks);
        assert_eq!(transitions.raster_copies, 1);
        let transitions = crtc.advance_oscillator_ticks(line_ticks * 3);
        assert_eq!(transitions.raster_copies, 3);
        crtc.write_operation(0x0000);
        let transitions = crtc.advance_oscillator_ticks(line_ticks * 2);
        assert_eq!(transitions.raster_copies, 0);
    }

    #[test]
    fn high_speed_clear_arms_and_reads_back_while_active() {
        let mut crtc = CrtcX68k::new();
        crtc.write_operation(0x02);
        assert!(crtc.high_speed_clear_requested());
        assert_eq!(crtc.read_operation(), 0);
        crtc.begin_high_speed_clear();
        assert!(!crtc.high_speed_clear_requested());
        assert!(crtc.high_speed_clear_active());
        assert_eq!(crtc.read_operation(), 0x0002);
        crtc.complete_high_speed_clear();
        assert!(!crtc.high_speed_clear_active());
        assert_eq!(crtc.read_operation(), 0);
    }

    #[test]
    fn vertical_display_start_reports_a_transition_edge() {
        let mut crtc = standard_crtc();
        let line_ticks = u64::from(crtc.read_register(0) + 1) * u64::from(crtc.clock_divisor()) * 8;
        let transitions = crtc.advance_oscillator_ticks(line_ticks * 40);
        assert!(!transitions.vertical_display_started);
        let transitions = crtc.advance_oscillator_ticks(line_ticks);
        assert!(transitions.vertical_display_started);
        let transitions = crtc.advance_oscillator_ticks(line_ticks);
        assert!(!transitions.vertical_display_started);
    }
}
