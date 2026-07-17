//! MSX video processor register, VRAM, status, and command state.

mod command;

use alloc::{vec, vec::Vec};

use self::command::V9938CommandEngine;

/// TMS-compatible CPU-visible VRAM address mask.
const TMS_VRAM_ADDRESS_MASK: u16 = 0x3FFF;
/// V9938 CPU-visible VRAM address mask.
const V9938_VRAM_ADDRESS_MASK: u32 = 0x1FFFF;
/// Number of TMS9118 control registers.
const TMS_REGISTER_COUNT: usize = 8;
/// Number of V9938 control register slots through R#46.
const V9938_REGISTER_COUNT: usize = 47;
/// Number of V9938 status registers.
const V9938_STATUS_COUNT: usize = 10;
/// Vertical-blank flag in status register zero.
const STATUS_VERTICAL_BLANK: u8 = 0x80;
/// Sprite-overflow flag in status register zero.
const STATUS_SPRITE_OVERFLOW: u8 = 0x40;
/// Sprite-collision flag in status register zero.
const STATUS_SPRITE_COLLISION: u8 = 0x20;
/// Sprite-number field in status register zero.
const STATUS_SPRITE_NUMBER_MASK: u8 = 0x1F;
/// Line-interrupt flag in status register one.
const STATUS_LINE_INTERRUPT: u8 = 0x01;
/// V9958 identification bit in status register one.
const STATUS_V9958_IDENTIFICATION: u8 = 0x04;
/// Command transfer-ready flag in status register two.
const STATUS_TRANSFER_READY: u8 = 0x80;
/// Command-execute flag in status register two.
const STATUS_COMMAND_EXECUTE: u8 = 0x01;
/// Horizontal-retrace flag in status register two.
const STATUS_HORIZONTAL_RETRACE: u8 = 0x20;
/// Vertical-retrace flag in status register two.
const STATUS_VERTICAL_RETRACE: u8 = 0x40;
/// VDP master-clock ticks in one scanline.
const V99X8_TICKS_PER_LINE: u64 = 1_368;
/// Minimum TMS9118 delay before a CPU VRAM request can be serviced.
const TMS_CPU_VRAM_DELAY_TICKS: u64 = 28;
/// Minimum V99x8 delay before a CPU VRAM request can be serviced.
const V99X8_CPU_VRAM_DELAY_TICKS: u64 = 16;
/// NTSC scanlines in one frame.
const V99X8_NTSC_LINES_PER_FRAME: u64 = 262;
/// Horizontal blanking length in graphics modes.
const V99X8_GRAPHICS_BLANK_TICKS: i64 = 312;
/// Horizontal blanking length in text modes.
const V99X8_TEXT_BLANK_TICKS: i64 = 404;
/// Horizontal position of the unadjusted graphics display start.
const V99X8_GRAPHICS_LEFT_TICK: i64 = 258;
/// Horizontal position of the unadjusted text display start.
const V99X8_TEXT_LEFT_TICK: i64 = 294;
/// Horizontal width of graphics modes in master ticks.
const V99X8_GRAPHICS_WIDTH_TICKS: i64 = 1_024;
/// Horizontal width of text modes in master ticks.
const V99X8_TEXT_WIDTH_TICKS: i64 = 960;
/// Horizontal phase where vertical retrace changes.
const V99X8_VERTICAL_PHASE_TICK: u64 = 202;
/// Vertical-interrupt enable bit in control register one.
const REGISTER_ONE_INTERRUPT_ENABLE: u8 = 0x20;
/// Line-interrupt enable bit in control register zero.
const REGISTER_ZERO_LINE_INTERRUPT_ENABLE: u8 = 0x10;
/// Writable masks for the TMS9118 control registers.
const TMS_REGISTER_MASKS: [u8; TMS_REGISTER_COUNT] =
    [0x03, 0xFB, 0x0F, 0xFF, 0x07, 0x7F, 0x07, 0xFF];
/// Writable masks for V9938 registers R#0 through R#23.
const V9938_REGISTER_MASKS: [u8; 24] = [
    0x7E, 0x7F, 0x7F, 0xFF, 0x3F, 0xFF, 0x3F, 0xFF, 0xFB, 0xBF, 0x07, 0x03, 0xFF, 0xFF, 0x07, 0x0F,
    0x0F, 0xBF, 0xFF, 0xFF, 0x3F, 0x3F, 0x3F, 0xFF,
];
/// Writable masks for V9958 registers R#25 through R#27.
const V9958_REGISTER_MASKS: [u8; 3] = [0x7F, 0x3F, 0x07];
/// V9938 reset palette in GRB bit order.
const V9938_RESET_PALETTE: [u16; 16] = [
    0x000, 0x000, 0x611, 0x733, 0x117, 0x327, 0x151, 0x627, 0x171, 0x373, 0x661, 0x664, 0x411,
    0x265, 0x555, 0x777,
];

/// MSX video processor version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MsxVdpVersion {
    /// TMS9118 used by the first-generation Sony target.
    Tms9118,
    /// Yamaha V9938.
    V9938,
    /// Yamaha V9958.
    V9958,
}

impl MsxVdpVersion {
    /// Whether this processor exposes the V9938-compatible interface.
    pub const fn is_v99x8(self) -> bool {
        !matches!(self, Self::Tms9118)
    }
}

/// Decoded MSX display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MsxVdpDisplayMode {
    /// Graphics 1 or SCREEN 1.
    Graphics1,
    /// Text 1 or SCREEN 0 width 40.
    Text1,
    /// Multicolor or SCREEN 3.
    Multicolor,
    /// Graphics 2 or SCREEN 2.
    Graphics2,
    /// Graphics 3 or SCREEN 4.
    Graphics3,
    /// Text 2 or SCREEN 0 width 80.
    Text2,
    /// Graphics 4 or SCREEN 5.
    Graphics4,
    /// Graphics 5 or SCREEN 6.
    Graphics5,
    /// Graphics 6 or SCREEN 7.
    Graphics6,
    /// Graphics 7 or SCREEN 8.
    Graphics7,
    /// An undocumented or unsupported mode bit pattern.
    Unsupported,
}

impl MsxVdpDisplayMode {
    /// Decodes the display mode bits shared by V9938 and V9958.
    pub const fn decode(register_zero: u8, register_one: u8) -> Self {
        let mode = ((register_zero & 0x0E) << 1)
            | ((register_one & 0x08) >> 2)
            | ((register_one & 0x10) >> 4);
        match mode {
            0x00 => Self::Graphics1,
            0x01 => Self::Text1,
            0x02 => Self::Multicolor,
            0x04 => Self::Graphics2,
            0x08 => Self::Graphics3,
            0x09 => Self::Text2,
            0x0C => Self::Graphics4,
            0x10 => Self::Graphics5,
            0x14 => Self::Graphics6,
            0x1C => Self::Graphics7,
            _ => Self::Unsupported,
        }
    }

    /// Whether this is one of the four V9938 bitmap modes.
    pub const fn is_bitmap(self) -> bool {
        matches!(
            self,
            Self::Graphics4 | Self::Graphics5 | Self::Graphics6 | Self::Graphics7
        )
    }

    /// Whether CPU VRAM accesses use the planar address transformation.
    pub const fn is_planar(self) -> bool {
        matches!(self, Self::Graphics6 | Self::Graphics7)
    }

    /// Whether the mode was introduced with the V9938.
    pub const fn is_v9938_mode(self) -> bool {
        matches!(
            self,
            Self::Graphics3
                | Self::Text2
                | Self::Graphics4
                | Self::Graphics5
                | Self::Graphics6
                | Self::Graphics7
        )
    }

    /// Whether the mode has 512 horizontal source pixels.
    pub const fn is_high_resolution(self) -> bool {
        matches!(self, Self::Text2 | Self::Graphics5 | Self::Graphics6)
    }

    /// Sprite mode selected by this display mode.
    pub const fn sprite_mode(self) -> u8 {
        match self {
            Self::Graphics1 | Self::Multicolor | Self::Graphics2 => 1,
            Self::Graphics3
            | Self::Graphics4
            | Self::Graphics5
            | Self::Graphics6
            | Self::Graphics7 => 2,
            Self::Text1 | Self::Text2 | Self::Unsupported => 0,
        }
    }
}

/// Sprite status produced while evaluating one visible scanline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsxSpriteLineStatus {
    /// First collision coordinate in active-display coordinates.
    pub collision: Option<(u16, u16)>,
    /// First sprite exceeding the per-line limit.
    pub overflow_sprite: Option<u8>,
    /// Last sprite number processed when no limit was exceeded.
    pub last_sprite: u8,
}

impl Default for MsxSpriteLineStatus {
    /// Creates status with no flags and sprite number 31.
    fn default() -> Self {
        Self {
            collision: None,
            overflow_sprite: None,
            last_sprite: STATUS_SPRITE_NUMBER_MASK,
        }
    }
}

/// Render-visible VDP state captured for one scanline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsxVdpRenderState {
    version: MsxVdpVersion,
    registers: [u8; V9938_REGISTER_COUNT],
    palette: [u16; 16],
    field: bool,
    blink: bool,
}

impl MsxVdpRenderState {
    /// Returns one masked control register.
    pub const fn register(self, index: usize) -> u8 {
        self.registers[index]
    }

    /// Returns the TMS-compatible control registers.
    pub const fn registers(self) -> [u8; TMS_REGISTER_COUNT] {
        let mut registers = [0; TMS_REGISTER_COUNT];
        let mut index = 0;
        while index < TMS_REGISTER_COUNT {
            registers[index] = self.registers[index];
            index += 1;
        }
        registers
    }

    /// Video processor version.
    pub const fn version(self) -> MsxVdpVersion {
        self.version
    }

    /// Decoded display mode.
    pub const fn display_mode(self) -> MsxVdpDisplayMode {
        MsxVdpDisplayMode::decode(self.registers[0], self.registers[1])
    }

    /// Current programmable palette in GRB bit order.
    pub const fn palette(self) -> [u16; 16] {
        self.palette
    }

    /// Whether the odd interlace field is active.
    pub const fn field(self) -> bool {
        self.field
    }

    /// Current Text 2 blink phase.
    pub const fn blink(self) -> bool {
        self.blink
    }

    /// Number of active display lines.
    pub const fn active_lines(self) -> u16 {
        if self.version.is_v99x8()
            && self.registers[9] & 0x80 != 0
            && matches!(
                self.display_mode(),
                MsxVdpDisplayMode::Text2
                    | MsxVdpDisplayMode::Graphics4
                    | MsxVdpDisplayMode::Graphics5
                    | MsxVdpDisplayMode::Graphics6
                    | MsxVdpDisplayMode::Graphics7
            )
        {
            212
        } else {
            192
        }
    }

    /// Whether V9958 YJK color decoding is enabled.
    pub const fn yjk_enabled(self) -> bool {
        matches!(self.version, MsxVdpVersion::V9958) && self.registers[25] & 0x08 != 0
    }

    /// Whether palette attributes are enabled in YJK mode.
    pub const fn yae_enabled(self) -> bool {
        self.yjk_enabled() && self.registers[25] & 0x10 != 0
    }

    /// Whether the left eight display pixels are masked.
    pub const fn horizontal_mask_enabled(self) -> bool {
        matches!(self.version, MsxVdpVersion::V9958) && self.registers[25] & 0x02 != 0
    }

    /// Whether horizontal scrolling may cross bitmap pages.
    pub const fn horizontal_multipage_enabled(self) -> bool {
        matches!(self.version, MsxVdpVersion::V9958)
            && self.registers[25] & 0x01 != 0
            && self.registers[2] & 0x20 != 0
    }

    /// Coarse horizontal scroll in eight-pixel units.
    pub const fn horizontal_scroll(self) -> u8 {
        if matches!(self.version, MsxVdpVersion::V9958) {
            self.registers[26]
        } else {
            0
        }
    }

    /// Fine horizontal scroll in pixels.
    pub const fn horizontal_adjust(self) -> u8 {
        if matches!(self.version, MsxVdpVersion::V9958) {
            self.registers[27]
        } else {
            0
        }
    }

    /// Returns the render state for the other bitmap page.
    pub const fn with_toggled_display_page(mut self) -> Self {
        self.registers[2] ^= 0x20;
        self
    }
}

/// Effects of a CPU-visible VDP access on machine scheduling.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MsxVdpEffects {
    /// Display or interrupt timing registers changed.
    pub timing_changed: bool,
    /// Command scheduling state changed.
    pub command_changed: bool,
}

#[derive(Debug, Clone, Copy)]
enum CpuVramAccess {
    Read,
    Write(u8),
}

/// Versioned MSX VDP CPU interface and authoritative VRAM state.
pub struct MsxVdp {
    version: MsxVdpVersion,
    vram: Vec<u8>,
    registers: [u8; V9938_REGISTER_COUNT],
    statuses: [u8; V9938_STATUS_COUNT],
    palette: [u16; 16],
    address: u32,
    read_ahead: u8,
    control_first_byte: u8,
    control_first_stored: bool,
    palette_first_byte: u8,
    palette_first_stored: bool,
    field: bool,
    blink: bool,
    blink_frames_remaining: u16,
    last_tick: u64,
    last_display_active: bool,
    pending_cpu_access: Option<CpuVramAccess>,
    pending_cpu_access_tick: u64,
    command: V9938CommandEngine,
}

save_state::runtime_state! {
/// Complete MSX VDP interface, VRAM, and command state.
#[derive(Clone)]
pub struct MsxVdpState {
    version: u8,
    vram: Vec<u8>,
    registers: [u8; V9938_REGISTER_COUNT],
    statuses: [u8; V9938_STATUS_COUNT],
    palette: [u16; 16],
    address: u32,
    read_ahead: u8,
    control_first_byte: u8,
    control_first_stored: bool,
    palette_first_byte: u8,
    palette_first_stored: bool,
    field: bool,
    blink: bool,
    blink_frames_remaining: u16,
    last_tick: u64,
    last_display_active: bool,
    pending_cpu_access: Option<u16>,
    pending_cpu_access_tick: u64,
    command: crate::video_msx::command::V9938CommandEngineState,
}}

impl MsxVdp {
    /// Creates a reset VDP with the requested physical VRAM size.
    pub fn new(version: MsxVdpVersion, vram_size: usize) -> Self {
        assert!(vram_size > usize::from(TMS_VRAM_ADDRESS_MASK));
        let mut statuses = [0; V9938_STATUS_COUNT];
        if version.is_v99x8() {
            statuses[2] = 0x0C;
        }
        if matches!(version, MsxVdpVersion::V9958) {
            statuses[1] = STATUS_V9958_IDENTIFICATION;
        }
        Self {
            version,
            vram: vec![0; vram_size],
            registers: [0; V9938_REGISTER_COUNT],
            statuses,
            palette: if version.is_v99x8() {
                V9938_RESET_PALETTE
            } else {
                [0; 16]
            },
            address: 0,
            read_ahead: 0,
            control_first_byte: 0,
            control_first_stored: false,
            palette_first_byte: 0,
            palette_first_stored: false,
            field: false,
            blink: false,
            blink_frames_remaining: 0,
            last_tick: 0,
            last_display_active: false,
            pending_cpu_access: None,
            pending_cpu_access_tick: 0,
            command: V9938CommandEngine::new(),
        }
    }

    /// Captures the complete VDP runtime state.
    pub fn capture_state(&self) -> MsxVdpState {
        MsxVdpState {
            version: match self.version {
                MsxVdpVersion::Tms9118 => 0,
                MsxVdpVersion::V9938 => 1,
                MsxVdpVersion::V9958 => 2,
            },
            vram: self.vram.clone(),
            registers: self.registers,
            statuses: self.statuses,
            palette: self.palette,
            address: self.address,
            read_ahead: self.read_ahead,
            control_first_byte: self.control_first_byte,
            control_first_stored: self.control_first_stored,
            palette_first_byte: self.palette_first_byte,
            palette_first_stored: self.palette_first_stored,
            field: self.field,
            blink: self.blink,
            blink_frames_remaining: self.blink_frames_remaining,
            last_tick: self.last_tick,
            last_display_active: self.last_display_active,
            pending_cpu_access: self.pending_cpu_access.map(|access| match access {
                CpuVramAccess::Read => 0,
                CpuVramAccess::Write(value) => 0x100 | u16::from(value),
            }),
            pending_cpu_access_tick: self.pending_cpu_access_tick,
            command: self.command.capture_state(),
        }
    }

    /// Restores the complete VDP runtime state.
    pub fn restore_state(
        &mut self,
        state: MsxVdpState,
    ) -> Result<(), save_state::StateValidationError> {
        let version = match state.version {
            0 => MsxVdpVersion::Tms9118,
            1 => MsxVdpVersion::V9938,
            2 => MsxVdpVersion::V9958,
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX VDP version is invalid",
                ));
            }
        };
        if version != self.version
            || state.vram.len() != self.vram.len()
            || state.address > V9938_VRAM_ADDRESS_MASK
            || state.palette.iter().any(|color| color & !0x0777 != 0)
        {
            return Err(save_state::StateValidationError::new(
                "MSX VDP configuration or state is invalid",
            ));
        }
        let pending_cpu_access = match state.pending_cpu_access {
            None => None,
            Some(0) => Some(CpuVramAccess::Read),
            Some(value) if value & 0xFF00 == 0x0100 => Some(CpuVramAccess::Write(value as u8)),
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX VDP pending access is invalid",
                ));
            }
        };
        self.command.restore_state(state.command)?;
        self.vram = state.vram;
        self.registers = state.registers;
        self.statuses = state.statuses;
        self.palette = state.palette;
        self.address = state.address;
        self.read_ahead = state.read_ahead;
        self.control_first_byte = state.control_first_byte;
        self.control_first_stored = state.control_first_stored;
        self.palette_first_byte = state.palette_first_byte;
        self.palette_first_stored = state.palette_first_stored;
        self.field = state.field;
        self.blink = state.blink;
        self.blink_frames_remaining = state.blink_frames_remaining;
        self.last_tick = state.last_tick;
        self.last_display_active = state.last_display_active;
        self.pending_cpu_access = pending_cpu_access;
        self.pending_cpu_access_tick = state.pending_cpu_access_tick;
        Ok(())
    }

    /// Video processor version.
    pub const fn version(&self) -> MsxVdpVersion {
        self.version
    }

    /// Physical video RAM.
    pub fn vram(&self) -> &[u8] {
        &self.vram
    }

    /// Current render-visible state.
    pub const fn render_state(&self) -> MsxVdpRenderState {
        MsxVdpRenderState {
            version: self.version,
            registers: self.registers,
            palette: self.palette,
            field: self.field,
            blink: self.blink,
        }
    }

    /// Advances asynchronous VDP work to an absolute master tick.
    pub fn advance_to(&mut self, tick: u64, display_active: bool) {
        if tick <= self.last_tick {
            self.last_display_active = display_active;
            return;
        }
        let mode = MsxVdpDisplayMode::decode(self.registers[0], self.registers[1]);
        if self.pending_cpu_access.is_some() && self.pending_cpu_access_tick <= tick {
            self.command.advance_to(
                self.pending_cpu_access_tick,
                display_active,
                self.registers[8] & 0x02 == 0,
                mode,
                &mut self.vram,
                &mut self.statuses,
            );
            self.execute_pending_cpu_access();
        }
        self.command.advance_to(
            tick,
            display_active,
            self.registers[8] & 0x02 == 0,
            mode,
            &mut self.vram,
            &mut self.statuses,
        );
        self.last_tick = tick;
        self.last_display_active = display_active;
    }

    /// Reads the buffered VRAM data port and advances the address.
    pub fn data_read(&mut self) -> u8 {
        let value = self.read_ahead;
        self.read_ahead = self.read_cpu_vram(self.address);
        self.increment_address();
        self.control_first_stored = false;
        value
    }

    /// Writes the VRAM data port and advances the address.
    pub fn data_write(&mut self, value: u8) {
        self.write_cpu_vram(self.address, value);
        self.read_ahead = value;
        self.increment_address();
        self.control_first_stored = false;
    }

    /// Reads the data port through the timed CPU-to-VRAM interface.
    pub fn timed_data_read(&mut self) -> u8 {
        let value = self.read_ahead;
        self.queue_cpu_vram_access(CpuVramAccess::Read);
        self.control_first_stored = false;
        value
    }

    /// Writes the data port through the timed CPU-to-VRAM interface.
    pub fn timed_data_write(&mut self, value: u8) {
        self.read_ahead = value;
        self.queue_cpu_vram_access(CpuVramAccess::Write(value));
        self.control_first_stored = false;
    }

    /// Reads the selected status register and applies its side effects.
    pub fn status_read(&mut self) -> u8 {
        self.control_first_stored = false;
        let selected = if self.version.is_v99x8() {
            usize::from(self.registers[15] & 0x0F)
        } else {
            0
        };
        if selected >= V9938_STATUS_COUNT {
            return 0xFF;
        }
        self.command.prepare_status(selected, &mut self.statuses);
        let value = if selected == 2 {
            self.statuses[selected] | self.beam_status()
        } else {
            self.statuses[selected]
        };
        match selected {
            0 => {
                self.statuses[0] &= STATUS_SPRITE_NUMBER_MASK;
            }
            1 => {
                if self.registers[0] & REGISTER_ZERO_LINE_INTERRUPT_ENABLE != 0 {
                    self.statuses[1] &= !STATUS_LINE_INTERRUPT;
                }
            }
            5 => {
                self.statuses[3] = 0;
                self.statuses[4] = 0xFE;
                self.statuses[5] = 0;
                self.statuses[6] = 0xFC;
            }
            7 => self.command.color_read(&mut self.statuses),
            9 => self.statuses[2] &= !0x10,
            _ => {}
        }
        value
    }

    /// Writes one byte of the two-byte control-port protocol.
    pub fn control_write(&mut self, value: u8) -> MsxVdpEffects {
        self.control_write_inner(value, false)
    }

    /// Writes the control port through the timed CPU-to-VRAM interface.
    pub fn timed_control_write(&mut self, value: u8) -> MsxVdpEffects {
        self.control_write_inner(value, true)
    }

    fn control_write_inner(&mut self, value: u8, timed: bool) -> MsxVdpEffects {
        if !self.control_first_stored {
            self.control_first_byte = value;
            self.control_first_stored = true;
            return MsxVdpEffects::default();
        }

        self.control_first_stored = false;
        if value & 0x80 != 0 {
            if self.version.is_v99x8() && value & 0x40 != 0 {
                return MsxVdpEffects::default();
            }
            let register_mask = if self.version.is_v99x8() { 0x3F } else { 0x07 };
            let register = usize::from(value & register_mask);
            return self.write_register(register, self.control_first_byte);
        }

        let low_address = (u32::from(value & 0x3F) << 8) | u32::from(self.control_first_byte);
        self.address = if self.version.is_v99x8() {
            (u32::from(self.registers[14] & 0x07) << 14) | low_address
        } else {
            low_address & u32::from(TMS_VRAM_ADDRESS_MASK)
        };
        if value & 0x40 == 0 {
            if timed {
                self.queue_cpu_vram_access(CpuVramAccess::Read);
            } else {
                self.read_ahead = self.read_cpu_vram(self.address);
                self.increment_address();
            }
        }
        MsxVdpEffects::default()
    }

    /// Writes one byte to the V9938 palette port.
    pub fn palette_write(&mut self, value: u8) {
        if !self.version.is_v99x8() {
            return;
        }
        self.control_first_stored = false;
        if !self.palette_first_stored {
            self.palette_first_byte = value;
            self.palette_first_stored = true;
            return;
        }
        self.palette_first_stored = false;
        let index = usize::from(self.registers[16] & 0x0F);
        self.palette[index] =
            u16::from(value & 0x07) << 8 | u16::from(self.palette_first_byte & 0x77);
        self.registers[16] = (self.registers[16] + 1) & 0x0F;
    }

    /// Writes one byte through the V9938 indirect-register port.
    pub fn indirect_write(&mut self, value: u8) -> MsxVdpEffects {
        if !self.version.is_v99x8() {
            return MsxVdpEffects::default();
        }
        self.control_first_stored = false;
        let pointer = self.registers[17];
        let effects = self.write_register(usize::from(pointer & 0x3F), value);
        if pointer & 0x80 == 0 {
            self.registers[17] = (pointer & 0xC0) | ((pointer + 1) & 0x3F);
        }
        effects
    }

    /// Sets the vertical-blank flag.
    pub fn enter_vertical_blank(&mut self) {
        self.statuses[0] |= STATUS_VERTICAL_BLANK;
    }

    /// Sets the programmable line-interrupt flag.
    pub fn enter_line_interrupt(&mut self) {
        if self.line_interrupt_enabled() {
            self.statuses[1] |= STATUS_LINE_INTERRUPT;
        }
    }

    /// Advances field and blink state at the start of a frame.
    pub fn start_frame(&mut self) {
        if !self.version.is_v99x8() {
            return;
        }
        self.field = !self.field;
        if self.blink_frames_remaining > 0 {
            self.blink_frames_remaining -= 1;
        }
        if self.blink_frames_remaining == 0 {
            let next = if self.blink {
                self.registers[13] & 0x0F
            } else {
                self.registers[13] >> 4
            };
            if next != 0 {
                self.blink = !self.blink;
                self.blink_frames_remaining = u16::from(next) * 10;
            }
        }
    }

    /// Merges sprite evaluation from one scanline into status registers.
    pub fn merge_sprite_status(&mut self, line: MsxSpriteLineStatus) {
        if let Some((x, y)) = line.collision
            && self.statuses[0] & STATUS_SPRITE_COLLISION == 0
        {
            self.statuses[0] |= STATUS_SPRITE_COLLISION;
            if self.version.is_v99x8() {
                let encoded_x = x.wrapping_add(12);
                let encoded_y = y.wrapping_add(8);
                self.statuses[3] = encoded_x as u8;
                self.statuses[4] = 0xFE | ((encoded_x >> 8) as u8 & 1);
                self.statuses[5] = encoded_y as u8;
                self.statuses[6] = 0xFC | ((encoded_y >> 8) as u8 & 3);
            }
        }
        if self.statuses[0] & STATUS_SPRITE_OVERFLOW == 0 {
            if let Some(sprite) = line.overflow_sprite {
                if self.statuses[0] & STATUS_VERTICAL_BLANK == 0 {
                    self.statuses[0] = (self.statuses[0] & STATUS_SPRITE_COLLISION)
                        | STATUS_SPRITE_OVERFLOW
                        | (sprite & STATUS_SPRITE_NUMBER_MASK);
                }
            } else {
                self.statuses[0] = (self.statuses[0] & !STATUS_SPRITE_NUMBER_MASK)
                    | (line.last_sprite & STATUS_SPRITE_NUMBER_MASK);
            }
        }
    }

    /// Whether the VDP is asserting its maskable interrupt output.
    pub const fn irq_pending(&self) -> bool {
        let vertical = self.statuses[0] & STATUS_VERTICAL_BLANK != 0
            && self.registers[1] & REGISTER_ONE_INTERRUPT_ENABLE != 0;
        let horizontal = self.version.is_v99x8()
            && self.statuses[1] & STATUS_LINE_INTERRUPT != 0
            && self.registers[0] & REGISTER_ZERO_LINE_INTERRUPT_ENABLE != 0;
        vertical || horizontal
    }

    /// Current status register zero without read side effects.
    pub const fn status(&self) -> u8 {
        self.statuses[0]
    }

    /// Current status register two without beam-position bits.
    pub const fn command_status(&self) -> u8 {
        self.statuses[2]
    }

    /// Returns the beam-position flags for status register two.
    fn beam_status(&self) -> u8 {
        if !self.version.is_v99x8() {
            return 0;
        }

        let frame_ticks = V99X8_NTSC_LINES_PER_FRAME * V99X8_TICKS_PER_LINE;
        let frame_tick = self.last_tick % frame_ticks;
        let mode = MsxVdpDisplayMode::decode(self.registers[0], self.registers[1]);
        let text_mode = matches!(mode, MsxVdpDisplayMode::Text1 | MsxVdpDisplayMode::Text2);
        let horizontal_adjust = i64::from((self.registers[18] & 0x0F) ^ 0x07);
        let left_tick = if text_mode {
            V99X8_TEXT_LEFT_TICK
        } else {
            V99X8_GRAPHICS_LEFT_TICK
        } + (horizontal_adjust - 7) * 4;
        let right_tick = left_tick
            + if text_mode {
                V99X8_TEXT_WIDTH_TICKS
            } else {
                V99X8_GRAPHICS_WIDTH_TICKS
            };
        let blank_ticks = if text_mode {
            V99X8_TEXT_BLANK_TICKS
        } else {
            V99X8_GRAPHICS_BLANK_TICKS
        };
        let line_tick = (frame_tick % V99X8_TICKS_PER_LINE) as i64;
        let horizontal_retrace =
            (line_tick - right_tick).rem_euclid(V99X8_TICKS_PER_LINE as i64) < blank_ticks;

        let active_lines = if self.registers[9] & 0x80 != 0 {
            212
        } else {
            192
        };
        let vertical_adjust = u64::from((self.registers[18] >> 4) ^ 0x07);
        let display_start =
            (3 + 13 + 9 + if active_lines == 192 { 10 } else { 0 } + vertical_adjust)
                * V99X8_TICKS_PER_LINE
                + V99X8_VERTICAL_PHASE_TICK;
        let display_end = display_start + active_lines * V99X8_TICKS_PER_LINE;
        let vertical_retrace =
            frame_tick < display_start - V99X8_TICKS_PER_LINE || frame_tick >= display_end;

        (u8::from(horizontal_retrace) * STATUS_HORIZONTAL_RETRACE)
            | (u8::from(vertical_retrace) * STATUS_VERTICAL_RETRACE)
    }

    /// Current CPU VRAM address.
    pub const fn address(&self) -> u32 {
        self.address
    }

    /// Current programmable interrupt line.
    pub const fn interrupt_line(&self) -> u8 {
        self.registers[19]
    }

    /// Current vertical scroll value.
    pub const fn vertical_scroll(&self) -> u8 {
        self.registers[23]
    }

    /// Whether line interrupts are enabled.
    pub const fn line_interrupt_enabled(&self) -> bool {
        self.version.is_v99x8() && self.registers[0] & REGISTER_ZERO_LINE_INTERRUPT_ENABLE != 0
    }

    /// Current active-line count.
    pub const fn active_lines(&self) -> u16 {
        self.render_state().active_lines()
    }

    /// Signed vertical adjustment in scanlines.
    pub const fn vertical_adjust(&self) -> i8 {
        signed_adjust(self.registers[18] >> 4)
    }

    /// Whether the display is enabled.
    pub const fn display_enabled(&self) -> bool {
        self.registers[1] & 0x40 != 0
    }

    /// Writes one masked control or command register.
    fn write_register(&mut self, register: usize, value: u8) -> MsxVdpEffects {
        if !self.version.is_v99x8() {
            if register < TMS_REGISTER_COUNT {
                self.registers[register] = value & TMS_REGISTER_MASKS[register];
            }
            return MsxVdpEffects::default();
        }
        if register >= V9938_REGISTER_COUNT
            || register == 24
            || (28..32).contains(&register)
            || matches!(self.version, MsxVdpVersion::V9938) && (25..28).contains(&register)
        {
            return MsxVdpEffects::default();
        }
        if register >= 32 {
            self.registers[register] = value;
            self.command.write_register(
                register - 32,
                value,
                self.last_tick,
                MsxVdpDisplayMode::decode(self.registers[0], self.registers[1]),
                &mut self.statuses,
            );
            return MsxVdpEffects {
                command_changed: true,
                ..MsxVdpEffects::default()
            };
        }
        let masked = if register < V9938_REGISTER_MASKS.len() {
            value & V9938_REGISTER_MASKS[register]
        } else {
            value & V9958_REGISTER_MASKS[register - 25]
        };
        let changed = self.registers[register] != masked;
        self.registers[register] = masked;
        if register == 13 {
            self.blink = masked & 0xF0 != 0;
            self.blink_frames_remaining = if masked & 0xF0 != 0 && masked & 0x0F != 0 {
                u16::from(masked >> 4) * 10
            } else {
                0
            };
        }
        if register == 16 {
            self.palette_first_stored = false;
        }
        MsxVdpEffects {
            timing_changed: changed && matches!(register, 0 | 1 | 9 | 18 | 19 | 23),
            command_changed: false,
        }
    }

    /// Queues one CPU-visible VRAM transaction.
    fn queue_cpu_vram_access(&mut self, access: CpuVramAccess) {
        let delay = if self.version.is_v99x8() {
            V99X8_CPU_VRAM_DELAY_TICKS
        } else {
            TMS_CPU_VRAM_DELAY_TICKS
        };
        let earliest = self.last_tick.saturating_add(delay);
        let interval = self.cpu_vram_service_interval();
        let line_start = earliest / V99X8_TICKS_PER_LINE * V99X8_TICKS_PER_LINE;
        let line_tick = earliest - line_start;
        self.pending_cpu_access_tick =
            line_start.saturating_add(line_tick.div_ceil(interval).saturating_mul(interval));
        self.pending_cpu_access = Some(access);
    }

    /// Executes the pending CPU transaction at its granted VRAM service slot.
    fn execute_pending_cpu_access(&mut self) {
        let Some(access) = self.pending_cpu_access.take() else {
            return;
        };
        match access {
            CpuVramAccess::Read => {
                self.read_ahead = self.read_cpu_vram(self.address);
            }
            CpuVramAccess::Write(value) => self.write_cpu_vram(self.address, value),
        }
        self.increment_address();
        let interval = self.cpu_vram_service_interval();
        self.command.defer_for_cpu_vram_access(interval);
    }

    /// Returns the aggregate interval between CPU-usable VRAM slots.
    fn cpu_vram_service_interval(&self) -> u64 {
        let mode = MsxVdpDisplayMode::decode(self.registers[0], self.registers[1]);
        if !self.last_display_active {
            return if self.version.is_v99x8() { 9 } else { 13 };
        }
        if self.version.is_v99x8() {
            if matches!(mode, MsxVdpDisplayMode::Text1 | MsxVdpDisplayMode::Text2) {
                29
            } else if mode.is_bitmap() && self.registers[8] & 0x02 != 0 {
                16
            } else {
                44
            }
        } else {
            match mode {
                MsxVdpDisplayMode::Text1 => 15,
                MsxVdpDisplayMode::Multicolor => 27,
                MsxVdpDisplayMode::Graphics1
                | MsxVdpDisplayMode::Graphics2
                | MsxVdpDisplayMode::Unsupported => 72,
                _ => 72,
            }
        }
    }

    /// Advances and wraps the CPU VRAM address.
    fn increment_address(&mut self) {
        if self.version.is_v99x8() {
            let low_address = self.address.wrapping_add(1) & u32::from(TMS_VRAM_ADDRESS_MASK);
            if low_address == 0
                && MsxVdpDisplayMode::decode(self.registers[0], self.registers[1]).is_v9938_mode()
            {
                self.registers[14] = self.registers[14].wrapping_add(1) & 0x07;
            }
            self.address = u32::from(self.registers[14]) << 14 | low_address;
        } else {
            self.address = self.address.wrapping_add(1) & u32::from(TMS_VRAM_ADDRESS_MASK);
        }
    }

    /// Transforms one logical CPU address to the installed physical VRAM.
    fn cpu_vram_address(&self, address: u32) -> u32 {
        if MsxVdpDisplayMode::decode(self.registers[0], self.registers[1]).is_planar() {
            ((address << 16) | (address >> 1)) & V9938_VRAM_ADDRESS_MASK
        } else {
            address
        }
    }

    /// Reads VRAM through the mode-dependent CPU address domain.
    fn read_cpu_vram(&self, address: u32) -> u8 {
        self.read_vram(self.cpu_vram_address(address))
    }

    /// Writes VRAM through the mode-dependent CPU address domain.
    fn write_cpu_vram(&mut self, address: u32, value: u8) {
        self.write_vram(self.cpu_vram_address(address), value);
    }

    /// Reads physical VRAM with absent addresses returning all ones.
    fn read_vram(&self, address: u32) -> u8 {
        self.vram.get(address as usize).copied().unwrap_or(0xFF)
    }

    /// Writes physical VRAM when the address is installed.
    fn write_vram(&mut self, address: u32, value: u8) {
        if let Some(destination) = self.vram.get_mut(address as usize) {
            *destination = value;
        }
    }
}

/// Decodes one four-bit signed adjustment value.
pub const fn signed_adjust(value: u8) -> i8 {
    if value & 0x08 != 0 {
        (value | 0xF0) as i8
    } else {
        value as i8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Selects a VRAM read or write address.
    fn set_address(vdp: &mut MsxVdp, address: u32, write: bool) {
        if vdp.version().is_v99x8() {
            write_register(vdp, 14, (address >> 14) as u8);
        }
        vdp.control_write(address as u8);
        vdp.control_write(((address >> 8) as u8 & 0x3F) | if write { 0x40 } else { 0 });
    }

    /// Writes one control register.
    fn write_register(vdp: &mut MsxVdp, register: u8, value: u8) {
        vdp.control_write(value);
        vdp.control_write(0x80 | register);
    }

    /// Configures V9938 Graphics 4 mode.
    fn configure_graphics_four(vdp: &mut MsxVdp) {
        write_register(vdp, 0, 0x06);
        write_register(vdp, 1, 0x40);
    }

    /// Writes one sixteen-bit command parameter.
    fn write_command_word(vdp: &mut MsxVdp, register: u8, value: u16) {
        write_register(vdp, register, value as u8);
        write_register(vdp, register + 1, (value >> 8) as u8);
    }

    /// Reads one selected V9938 status register.
    fn read_status(vdp: &mut MsxVdp, status: u8) -> u8 {
        write_register(vdp, 15, status);
        vdp.status_read()
    }

    #[test]
    /// TMS register writes use the two-byte latch and hardware masks.
    fn tms_register_writes_are_latched_and_masked() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::Tms9118, 0x4000);
        for register in 0u8..8 {
            write_register(&mut vdp, register, 0xFF);
        }
        assert_eq!(
            vdp.render_state().registers(),
            [0x03, 0xFB, 0x0F, 0xFF, 0x07, 0x7F, 0x07, 0xFF]
        );
    }

    #[test]
    /// VRAM reads return the prefetched byte before loading the next byte.
    fn read_address_prefetches_and_data_reads_stay_one_byte_ahead() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::Tms9118, 0x4000);
        set_address(&mut vdp, 0x1234, true);
        vdp.data_write(0x11);
        vdp.data_write(0x22);

        set_address(&mut vdp, 0x1234, false);
        assert_eq!(vdp.address(), 0x1235);
        assert_eq!(vdp.data_read(), 0x11);
        assert_eq!(vdp.data_read(), 0x22);
        assert_eq!(vdp.address(), 0x1237);
    }

    #[test]
    /// TMS VRAM accesses wrap at the end of the fourteen-bit address space.
    fn tms_address_auto_increment_wraps_at_fourteen_bits() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::Tms9118, 0x4000);
        set_address(&mut vdp, 0x3FFF, true);
        vdp.data_write(0xAA);
        vdp.data_write(0x55);
        assert_eq!(vdp.address(), 1);
    }

    #[test]
    /// V9938 VRAM accesses carry through the complete 128 KiB address.
    fn v9938_address_auto_increment_wraps_at_seventeen_bits() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut vdp);
        set_address(&mut vdp, 0x1FFFF, true);
        vdp.data_write(0xAA);
        vdp.data_write(0x55);
        assert_eq!(vdp.address(), 1);
        set_address(&mut vdp, 0x1FFFF, false);
        assert_eq!(vdp.data_read(), 0xAA);
        assert_eq!(vdp.data_read(), 0x55);
    }

    #[test]
    /// Legacy modes wrap the low pointer without carrying into R#14.
    fn v9938_legacy_mode_address_wrap_preserves_the_high_page() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        set_address(&mut vdp, 0x7FFF, true);
        vdp.data_write(0xAA);
        vdp.data_write(0x55);
        assert_eq!(vdp.address(), 0x4001);
        assert_eq!(vdp.vram[0x7FFF], 0xAA);
        assert_eq!(vdp.vram[0x4000], 0x55);
    }

    #[test]
    /// Planar bitmap modes interleave logical CPU bytes between both VRAM banks.
    fn v9938_planar_cpu_accesses_transform_the_vram_address() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        write_register(&mut vdp, 0, 0x0A);
        set_address(&mut vdp, 0, true);
        for value in [0x11, 0x22, 0x33, 0x44] {
            vdp.data_write(value);
        }
        assert_eq!(vdp.vram[0], 0x11);
        assert_eq!(vdp.vram[0x10000], 0x22);
        assert_eq!(vdp.vram[1], 0x33);
        assert_eq!(vdp.vram[0x10001], 0x44);

        set_address(&mut vdp, 0, false);
        assert_eq!(vdp.data_read(), 0x11);
        assert_eq!(vdp.data_read(), 0x22);
        assert_eq!(vdp.data_read(), 0x33);
        assert_eq!(vdp.data_read(), 0x44);
    }

    #[test]
    /// Palette writes latch two bytes and advance the palette pointer.
    fn v9938_palette_writes_are_latched_and_incremented() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        write_register(&mut vdp, 16, 3);
        vdp.palette_write(0x75);
        vdp.palette_write(0x06);
        assert_eq!(vdp.render_state().palette()[3], 0x675);
        assert_eq!(vdp.render_state().register(16), 4);
    }

    #[test]
    /// Indirect access honors increment and fixed-pointer modes.
    fn v9938_indirect_register_access_honors_pointer_mode() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        write_register(&mut vdp, 17, 18);
        vdp.indirect_write(0x21);
        vdp.indirect_write(0x55);
        assert_eq!(vdp.render_state().register(18), 0x21);
        assert_eq!(vdp.render_state().register(19), 0x55);

        write_register(&mut vdp, 17, 0x80 | 18);
        vdp.indirect_write(0x43);
        vdp.indirect_write(0x65);
        assert_eq!(vdp.render_state().register(18), 0x65);
    }

    #[test]
    /// V9938 control writes with both command bits set do not write a register.
    fn v9938_ignores_the_reserved_control_port_command() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        vdp.control_write(0x7F);
        vdp.control_write(0xC1);
        assert_eq!(vdp.render_state().register(1), 0);
    }

    #[test]
    /// Status selection separates vertical and line interrupt clearing.
    fn v9938_status_reads_clear_only_the_selected_interrupt() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        write_register(&mut vdp, 0, REGISTER_ZERO_LINE_INTERRUPT_ENABLE);
        write_register(&mut vdp, 1, REGISTER_ONE_INTERRUPT_ENABLE);
        vdp.enter_vertical_blank();
        vdp.enter_line_interrupt();
        assert!(vdp.irq_pending());
        write_register(&mut vdp, 15, 1);
        assert_eq!(vdp.status_read() & 1, 1);
        assert!(vdp.irq_pending());
        write_register(&mut vdp, 15, 0);
        assert_eq!(vdp.status_read() & 0x80, 0x80);
        assert!(!vdp.irq_pending());
    }

    #[test]
    /// A disabled line interrupt cannot leave a latched request.
    fn disabled_line_interrupt_does_not_latch() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        vdp.enter_line_interrupt();
        write_register(&mut vdp, 15, 1);
        assert_eq!(vdp.status_read() & STATUS_LINE_INTERRUPT, 0);
        write_register(&mut vdp, 0, REGISTER_ZERO_LINE_INTERRUPT_ENABLE);
        assert!(!vdp.irq_pending());
        vdp.enter_line_interrupt();
        assert!(vdp.irq_pending());
        assert_eq!(
            vdp.status_read() & STATUS_LINE_INTERRUPT,
            STATUS_LINE_INTERRUPT
        );
        assert!(!vdp.irq_pending());
    }

    #[test]
    /// V9938 register masks reject missing slots and reserved bits.
    fn v9938_register_masks_cover_the_control_file() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        for register in 0..24u8 {
            write_register(&mut vdp, register, 0xFF);
            assert_eq!(
                vdp.render_state().register(usize::from(register)),
                V9938_REGISTER_MASKS[usize::from(register)]
            );
        }
        write_register(&mut vdp, 24, 0xFF);
        assert_eq!(vdp.render_state().register(24), 0);
    }

    #[test]
    /// V9958 registers expose only their documented writable bits.
    fn v9958_register_masks_cover_the_extended_control_file() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9958, 0x20000);
        for register in 25..28u8 {
            write_register(&mut vdp, register, 0xFF);
            assert_eq!(
                vdp.render_state().register(usize::from(register)),
                V9958_REGISTER_MASKS[usize::from(register - 25)]
            );
        }
        write_register(&mut vdp, 24, 0xFF);
        write_register(&mut vdp, 28, 0xFF);
        assert_eq!(vdp.render_state().register(24), 0);
        assert_eq!(vdp.render_state().register(28), 0);
    }

    #[test]
    /// Status register one identifies the V9958 without disturbing line flags.
    fn v9958_status_one_reports_the_identification_bit() {
        let mut v9938 = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        let mut v9958 = MsxVdp::new(MsxVdpVersion::V9958, 0x20000);
        write_register(&mut v9938, 15, 1);
        write_register(&mut v9958, 15, 1);
        assert_eq!(v9938.status_read() & STATUS_V9958_IDENTIFICATION, 0);
        assert_eq!(
            v9958.status_read() & STATUS_V9958_IDENTIFICATION,
            STATUS_V9958_IDENTIFICATION
        );
        write_register(&mut v9958, 0, REGISTER_ZERO_LINE_INTERRUPT_ENABLE);
        v9958.enter_line_interrupt();
        assert_eq!(
            v9958.status_read(),
            STATUS_V9958_IDENTIFICATION | STATUS_LINE_INTERRUPT
        );
        assert_eq!(v9958.status_read(), STATUS_V9958_IDENTIFICATION);
    }

    #[test]
    /// Enabling vertical interrupts after VBlank asserts the output.
    fn enabling_interrupt_after_vertical_blank_asserts_the_line() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::Tms9118, 0x4000);
        vdp.enter_vertical_blank();
        assert!(!vdp.irq_pending());
        write_register(&mut vdp, 1, REGISTER_ONE_INTERRUPT_ENABLE);
        assert!(vdp.irq_pending());
    }

    #[test]
    /// HMMV changes VRAM over time and exposes CE until completion.
    fn command_fill_is_asynchronous() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut vdp);
        write_command_word(&mut vdp, 36, 0);
        write_command_word(&mut vdp, 38, 0);
        write_command_word(&mut vdp, 40, 4);
        write_command_word(&mut vdp, 42, 1);
        write_register(&mut vdp, 44, 0x0A);
        write_register(&mut vdp, 46, 0xC0);
        assert_eq!(vdp.vram[0], 0);
        assert_ne!(vdp.command_status() & STATUS_COMMAND_EXECUTE, 0);

        vdp.advance_to(10_000, false);
        assert_eq!(&vdp.vram[..2], &[0xAA, 0xAA]);
        assert_eq!(vdp.command_status() & STATUS_COMMAND_EXECUTE, 0);
    }

    #[test]
    /// V9958 scroll registers do not disturb an active bitmap command.
    fn horizontal_scroll_registers_are_independent_of_commands() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9958, 0x20000);
        configure_graphics_four(&mut vdp);
        write_command_word(&mut vdp, 36, 0);
        write_command_word(&mut vdp, 38, 0);
        write_command_word(&mut vdp, 40, 4);
        write_command_word(&mut vdp, 42, 1);
        write_register(&mut vdp, 44, 0x0A);
        write_register(&mut vdp, 46, 0xC0);
        write_register(&mut vdp, 25, 0x03);
        write_register(&mut vdp, 26, 0x3F);
        write_register(&mut vdp, 27, 7);
        assert_ne!(vdp.command_status() & STATUS_COMMAND_EXECUTE, 0);

        vdp.advance_to(10_000, false);
        assert_eq!(&vdp.vram[..2], &[0xAA, 0xAA]);
        assert_eq!(vdp.render_state().register(25), 0x03);
        assert_eq!(vdp.render_state().register(26), 0x3F);
        assert_eq!(vdp.render_state().register(27), 7);
    }

    #[test]
    /// PSET logical operations and POINT use packed bitmap pixels.
    fn point_and_pset_share_mode_specific_pixel_addressing() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut vdp);
        write_command_word(&mut vdp, 36, 1);
        write_command_word(&mut vdp, 38, 0);
        write_register(&mut vdp, 44, 3);
        write_register(&mut vdp, 46, 0x50);
        vdp.advance_to(1_000, false);
        assert_eq!(vdp.vram[0], 3);

        write_command_word(&mut vdp, 32, 1);
        write_command_word(&mut vdp, 34, 0);
        write_register(&mut vdp, 46, 0x40);
        vdp.advance_to(2_000, false);
        assert_eq!(read_status(&mut vdp, 7), 3);
    }

    #[test]
    /// LMMC waits for each CPU color and raises TR between transfers.
    fn cpu_to_vram_command_uses_transfer_handshake() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut vdp);
        write_command_word(&mut vdp, 36, 0);
        write_command_word(&mut vdp, 38, 0);
        write_command_word(&mut vdp, 40, 2);
        write_command_word(&mut vdp, 42, 1);
        write_register(&mut vdp, 46, 0xB0);
        assert_ne!(vdp.command_status() & STATUS_TRANSFER_READY, 0);

        write_register(&mut vdp, 44, 5);
        vdp.advance_to(1_000, false);
        assert_eq!(vdp.vram[0] >> 4, 5);
        assert_ne!(vdp.command_status() & STATUS_TRANSFER_READY, 0);

        write_register(&mut vdp, 44, 6);
        vdp.advance_to(2_000, false);
        assert_eq!(vdp.vram[0], 0x56);
        assert_eq!(vdp.command_status() & STATUS_COMMAND_EXECUTE, 0);
    }

    #[test]
    /// A color written before LMMC starts is the first transferred pixel.
    fn cpu_to_vram_command_consumes_the_preloaded_color() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut vdp);
        write_command_word(&mut vdp, 36, 0);
        write_command_word(&mut vdp, 38, 0);
        write_command_word(&mut vdp, 40, 2);
        write_command_word(&mut vdp, 42, 1);
        write_register(&mut vdp, 44, 5);
        write_register(&mut vdp, 46, 0xB0);
        vdp.advance_to(1_000, false);
        assert_eq!(vdp.vram[0] >> 4, 5);
        assert_ne!(vdp.command_status() & STATUS_TRANSFER_READY, 0);

        write_register(&mut vdp, 44, 6);
        vdp.advance_to(2_000, false);
        assert_eq!(vdp.vram[0], 0x56);
        assert_eq!(vdp.command_status() & STATUS_COMMAND_EXECUTE, 0);
    }

    #[test]
    /// LMCM completes while leaving the final color and TR pending.
    fn vram_to_cpu_command_uses_transfer_handshake() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut vdp);
        vdp.vram[0] = 0xA5;
        write_command_word(&mut vdp, 32, 0);
        write_command_word(&mut vdp, 34, 0);
        write_command_word(&mut vdp, 40, 2);
        write_command_word(&mut vdp, 42, 1);
        write_register(&mut vdp, 46, 0xA0);
        vdp.advance_to(1_000, false);
        assert_ne!(vdp.command_status() & STATUS_TRANSFER_READY, 0);
        assert_eq!(read_status(&mut vdp, 7), 0x0A);
        vdp.advance_to(2_000, false);
        assert_eq!(vdp.command_status() & STATUS_COMMAND_EXECUTE, 0);
        assert_ne!(vdp.command_status() & STATUS_TRANSFER_READY, 0);
        assert_eq!(read_status(&mut vdp, 7), 0x05);
        assert_eq!(vdp.command_status() & STATUS_TRANSFER_READY, 0);
    }

    #[test]
    /// STOP preserves partial command results and clears CE.
    fn stop_aborts_an_in_progress_command() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut vdp);
        write_command_word(&mut vdp, 36, 0);
        write_command_word(&mut vdp, 38, 0);
        write_command_word(&mut vdp, 40, 64);
        write_command_word(&mut vdp, 42, 1);
        write_register(&mut vdp, 44, 7);
        write_register(&mut vdp, 46, 0x80);
        vdp.advance_to(500, false);
        assert_ne!(vdp.command_status() & STATUS_COMMAND_EXECUTE, 0);
        write_register(&mut vdp, 46, 0);
        let partial = vdp.vram.clone();
        vdp.advance_to(100_000, false);
        assert_eq!(vdp.vram, partial);
        assert_eq!(vdp.command_status() & STATUS_COMMAND_EXECUTE, 0);
    }

    #[test]
    /// Search, line, logical copy, and high-speed copy commands update results.
    fn remaining_autonomous_commands_update_pixels_and_status() {
        let mut search = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut search);
        search.vram[1] = 0xA0;
        write_command_word(&mut search, 32, 0);
        write_command_word(&mut search, 34, 0);
        write_register(&mut search, 44, 0x0A);
        write_register(&mut search, 45, 0);
        write_register(&mut search, 46, 0x60);
        search.advance_to(10_000, false);
        assert_ne!(search.command_status() & 0x10, 0);
        assert_eq!(read_status(&mut search, 8), 2);

        write_command_word(&mut search, 32, 0);
        write_register(&mut search, 45, 0x02);
        write_register(&mut search, 46, 0x60);
        search.advance_to(20_000, false);
        assert_ne!(search.command_status() & 0x10, 0);
        assert_eq!(read_status(&mut search, 8), 0);

        let mut line = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut line);
        write_command_word(&mut line, 36, 0);
        write_command_word(&mut line, 38, 0);
        write_command_word(&mut line, 40, 4);
        write_command_word(&mut line, 42, 2);
        write_register(&mut line, 44, 7);
        write_register(&mut line, 46, 0x70);
        line.advance_to(10_000, false);
        assert!(line.vram[..256].iter().any(|value| *value != 0));

        let mut logical = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut logical);
        logical.vram[0] = 0xC3;
        write_command_word(&mut logical, 32, 0);
        write_command_word(&mut logical, 34, 0);
        write_command_word(&mut logical, 36, 4);
        write_command_word(&mut logical, 38, 0);
        write_command_word(&mut logical, 40, 2);
        write_command_word(&mut logical, 42, 1);
        write_register(&mut logical, 46, 0x90);
        logical.advance_to(10_000, false);
        assert_eq!(logical.vram[2], 0xC3);

        let mut high = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut high);
        high.vram[0] = 0x5A;
        write_command_word(&mut high, 32, 0);
        write_command_word(&mut high, 34, 0);
        write_command_word(&mut high, 36, 4);
        write_command_word(&mut high, 38, 0);
        write_command_word(&mut high, 40, 2);
        write_command_word(&mut high, 42, 1);
        write_register(&mut high, 46, 0xD0);
        high.advance_to(10_000, false);
        assert_eq!(high.vram[2], 0x5A);

        high.vram[131] = 0xA5;
        write_command_word(&mut high, 34, 1);
        write_command_word(&mut high, 36, 6);
        write_command_word(&mut high, 38, 2);
        write_command_word(&mut high, 42, 1);
        write_register(&mut high, 46, 0xE0);
        high.advance_to(20_000, false);
        assert_eq!(high.vram[259], 0xA5);
    }

    #[test]
    /// LINE treats NY zero as a zero-length minor axis.
    fn horizontal_line_with_zero_minor_length_stays_on_one_row() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut vdp);
        write_command_word(&mut vdp, 36, 3);
        write_command_word(&mut vdp, 38, 4);
        write_command_word(&mut vdp, 40, 5);
        write_command_word(&mut vdp, 42, 0);
        write_register(&mut vdp, 44, 7);
        write_register(&mut vdp, 46, 0x70);
        vdp.advance_to(10_000, false);

        for x in 0..256 {
            let expected = if (3..=8).contains(&x) { 7 } else { 0 };
            assert_eq!(
                command::read_pixel(&vdp.vram, MsxVdpDisplayMode::Graphics4, x, 4, false),
                expected,
            );
            assert_eq!(
                command::read_pixel(&vdp.vram, MsxVdpDisplayMode::Graphics4, x, 5, false),
                0,
            );
        }
    }

    #[test]
    /// A 256-pixel mode wraps an out-of-range command X for its first pixel.
    fn command_x_outside_a_256_pixel_mode_processes_one_wrapped_pixel() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut vdp);
        write_command_word(&mut vdp, 36, 300);
        write_command_word(&mut vdp, 38, 0);
        write_register(&mut vdp, 44, 9);
        write_register(&mut vdp, 46, 0x50);
        vdp.advance_to(1_000, false);
        assert_eq!(
            command::read_pixel(&vdp.vram, MsxVdpDisplayMode::Graphics4, 44, 0, false),
            9,
        );
    }

    #[test]
    /// YMMM ignores MXS and uses MXD for both sides of the transfer.
    fn vertical_copy_uses_the_destination_memory_selection() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut vdp);
        vdp.vram[0] = 0xA5;
        write_command_word(&mut vdp, 34, 0);
        write_command_word(&mut vdp, 36, 0);
        write_command_word(&mut vdp, 38, 1);
        write_command_word(&mut vdp, 42, 1);
        write_register(&mut vdp, 45, 0x10);
        write_register(&mut vdp, 46, 0xE0);
        vdp.advance_to(20_000, false);
        assert_eq!(vdp.vram[128], 0xA5);
    }

    #[test]
    /// HMMC transfers one packed byte for each CPU handshake.
    fn high_speed_cpu_transfer_writes_complete_bytes() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut vdp);
        write_command_word(&mut vdp, 36, 0);
        write_command_word(&mut vdp, 38, 0);
        write_command_word(&mut vdp, 40, 4);
        write_command_word(&mut vdp, 42, 1);
        write_register(&mut vdp, 46, 0xF0);
        write_register(&mut vdp, 44, 0x12);
        vdp.advance_to(1_000, false);
        write_register(&mut vdp, 44, 0x34);
        vdp.advance_to(2_000, false);
        assert_eq!(&vdp.vram[..2], &[0x12, 0x34]);
        assert_eq!(vdp.command_status() & STATUS_COMMAND_EXECUTE, 0);
    }

    #[test]
    /// HMMV follows the calibrated blanked and active-display timing matrix.
    fn command_throughput_depends_on_display_state() {
        fn fill_vdp() -> MsxVdp {
            let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
            configure_graphics_four(&mut vdp);
            write_command_word(&mut vdp, 36, 0);
            write_command_word(&mut vdp, 38, 0);
            write_command_word(&mut vdp, 40, 64);
            write_command_word(&mut vdp, 42, 1);
            write_register(&mut vdp, 44, 9);
            write_register(&mut vdp, 46, 0xC0);
            vdp
        }

        let mut blanked = fill_vdp();
        let mut sprites_disabled = fill_vdp();
        let mut sprites_enabled = fill_vdp();
        write_register(&mut sprites_disabled, 8, 0x02);
        blanked.advance_to(530, false);
        sprites_disabled.advance_to(530, true);
        sprites_enabled.advance_to(530, true);
        let blanked_bytes = blanked.vram.iter().filter(|value| **value != 0).count();
        let sprite_free_bytes = sprites_disabled
            .vram
            .iter()
            .filter(|value| **value != 0)
            .count();
        let sprite_active_bytes = sprites_enabled
            .vram
            .iter()
            .filter(|value| **value != 0)
            .count();
        assert_eq!(blanked_bytes, 12);
        assert_eq!(sprite_free_bytes, 10);
        assert_eq!(sprite_active_bytes, 9);
    }

    #[test]
    fn active_command_state_replays_exactly() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut vdp);
        write_command_word(&mut vdp, 36, 0);
        write_command_word(&mut vdp, 38, 0);
        write_command_word(&mut vdp, 40, 128);
        write_command_word(&mut vdp, 42, 4);
        write_register(&mut vdp, 44, 9);
        write_register(&mut vdp, 46, 0xC0);
        vdp.advance_to(500, false);
        let snapshot = vdp.capture_state();

        vdp.advance_to(5_000, false);
        let expected = save_state::encode_runtime_state(&vdp.capture_state());
        vdp.restore_state(snapshot).unwrap();
        vdp.advance_to(5_000, false);
        let replayed = save_state::encode_runtime_state(&vdp.capture_state());

        assert_eq!(replayed, expected);
    }

    #[test]
    /// Logical operations and transparent variants preserve packed neighbors.
    fn command_logical_operations_match_the_selected_nibble() {
        for (operation, expected) in [(0, 3), (1, 1), (2, 7), (3, 6), (4, 12)] {
            let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
            configure_graphics_four(&mut vdp);
            vdp.vram[0] = 0x5A;
            write_command_word(&mut vdp, 36, 0);
            write_command_word(&mut vdp, 38, 0);
            write_register(&mut vdp, 44, 3);
            write_register(&mut vdp, 46, 0x50 | operation);
            vdp.advance_to(1_000, false);
            assert_eq!(vdp.vram[0], expected << 4 | 0x0A);
        }

        let mut transparent = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut transparent);
        transparent.vram[0] = 0x5A;
        write_command_word(&mut transparent, 36, 0);
        write_command_word(&mut transparent, 38, 0);
        write_register(&mut transparent, 44, 0);
        write_register(&mut transparent, 46, 0x58);
        transparent.advance_to(1_000, false);
        assert_eq!(transparent.vram[0], 0x5A);
    }

    #[test]
    /// Rectangle directions clip at the left edge and wrap downward at 1024.
    fn command_direction_clipping_and_vertical_wrap_are_applied() {
        let mut decrement = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut decrement);
        write_command_word(&mut decrement, 36, 1);
        write_command_word(&mut decrement, 38, 1);
        write_command_word(&mut decrement, 40, 4);
        write_command_word(&mut decrement, 42, 2);
        write_register(&mut decrement, 44, 0x0A);
        write_register(&mut decrement, 45, 0x0C);
        write_register(&mut decrement, 46, 0x80);
        decrement.advance_to(10_000, false);
        assert_eq!(decrement.vram[0], 0xAA);
        assert_eq!(decrement.vram[128], 0xAA);
        assert_eq!(decrement.command_status() & STATUS_COMMAND_EXECUTE, 0);

        let mut wrapped = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        configure_graphics_four(&mut wrapped);
        write_command_word(&mut wrapped, 36, 0);
        write_command_word(&mut wrapped, 38, 1023);
        write_command_word(&mut wrapped, 40, 1);
        write_command_word(&mut wrapped, 42, 2);
        write_register(&mut wrapped, 44, 7);
        write_register(&mut wrapped, 46, 0x80);
        wrapped.advance_to(10_000, false);
        assert_eq!(wrapped.vram[0] >> 4, 7);
        assert_eq!(wrapped.vram[0x1FF80] >> 4, 7);
    }

    #[test]
    /// Sprite collision coordinates and overflow clear through status reads.
    fn sprite_status_coordinates_and_flags_have_read_side_effects() {
        let mut vdp = MsxVdp::new(MsxVdpVersion::V9938, 0x20000);
        vdp.merge_sprite_status(MsxSpriteLineStatus {
            collision: Some((250, 210)),
            overflow_sprite: Some(8),
            last_sprite: 7,
        });
        assert_eq!(read_status(&mut vdp, 3), 6);
        assert_eq!(read_status(&mut vdp, 4), 0xFF);
        assert_eq!(read_status(&mut vdp, 6), 0xFC);
        assert_eq!(read_status(&mut vdp, 5), 218);
        assert_eq!(read_status(&mut vdp, 3), 0);
        assert_eq!(read_status(&mut vdp, 4), 0xFE);
        assert_eq!(read_status(&mut vdp, 6), 0xFC);
        assert_eq!(read_status(&mut vdp, 0), 0x68);
        assert_eq!(vdp.status(), 8);
    }
}
