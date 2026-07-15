//! µPD7220 GDC (Graphics Display Controller) emulator.

mod commands;
mod drawing;
mod fifo;

use std::ops::{Deref, DerefMut};

pub use fifo::FifoState;

/// Status register bit 0: data ready for CPU to read from FIFO.
pub(crate) const STATUS_DATA_READY: u8 = 0x01;

/// Status register bit 1: command FIFO is full, CPU must wait.
pub(crate) const STATUS_FIFO_FULL: u8 = 0x02;

/// Status register bit 2: command FIFO is empty, GDC is idle.
const STATUS_FIFO_EMPTY: u8 = 0x04;

/// Status register bit 3: GDC drawing operation in progress.
pub const STATUS_DRAWING: u8 = 0x08;

/// Status register bit 4: DMA transfer in progress.
pub const STATUS_DMA_EXECUTE: u8 = 0x10;

/// Status register bit 5: vertical sync is active.
const STATUS_VSYNC: u8 = 0x20;

/// Status register bit 6: horizontal blanking period.
const STATUS_HBLANK: u8 = 0x40;

/// Status register bit 7: light pen detect (always 1 on PC-98 slave GDC).
const STATUS_LIGHT_PEN: u8 = 0x80;

/// SYNC P1 display mode bits 1 and 5: mixed display.
pub const DISPLAY_MODE_MIXED: u8 = 0x00;

/// SYNC P1 display mode bits 1 and 5: graphics display.
pub const DISPLAY_MODE_GRAPHICS: u8 = 0x02;

/// SYNC P1 display mode bits 1 and 5: character display.
pub const DISPLAY_MODE_CHARACTER: u8 = 0x20;

const DISPLAY_MODE_MASK: u8 = DISPLAY_MODE_GRAPHICS | DISPLAY_MODE_CHARACTER;

/// Dot clock for 400-line mode (24.83 kHz horizontal): 21.0526 MHz.
pub const DOT_CLOCK_400LINE: u32 = 21_052_600;

/// Dot clock for 200-line mode (15.98 kHz horizontal): 14.31818 MHz.
pub const DOT_CLOCK_200LINE: u32 = 14_318_180;

/// Default pitch for the master (text) GDC: 80 characters per row.
const MASTER_DEFAULT_PITCH: u16 = 80;

/// Default pitch for the slave (graphics) GDC: 40 words per row.
const SLAVE_DEFAULT_PITCH: u16 = 40;

save_state::runtime_state! {
/// Authoritative GDC scroll display partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GdcScrollPartition {
    /// SAD: display start address (18-bit for graphics, 13-bit for text).
    pub start_address: u32,
    /// LEN: scanline count for this area (10-bit).
    pub line_count: u16,
    /// IM: image mode (false=character, true=bitmap).
    pub im: bool,
    /// WD: wide display mode.
    pub wd: bool,
}}

/// VRAM write operation produced by drawing commands.
#[derive(Debug, Clone, Copy)]
pub struct VramOp {
    /// 18-bit word address in VRAM.
    pub address: u32,
    /// 16-bit data to write.
    pub data: u16,
    /// 16-bit mask (which bits are affected).
    pub mask: u16,
    /// Transfer mode (0=replace, 1=complement, 2=clear, 3=set).
    pub mode: u8,
}

/// Result of a drawing operation.
#[derive(Debug, Clone)]
pub struct DrawResult {
    /// Number of dots drawn (for timing calculation).
    pub dot_count: u32,
}

/// Request for VRAM read data (RDAT command).
#[derive(Debug, Clone)]
pub struct RdatRequest {
    /// Transfer type (0=word, 2=low byte, 3=high byte).
    pub transfer_type: u8,
}

/// Action returned by GDC write methods that requires bus-level handling.
pub enum GdcAction {
    /// No action needed.
    None,
    /// Execute VRAM writes from a drawing command.
    Draw(DrawResult),
    /// Single VRAM write from a DMA (DACK) transfer byte.
    DackWrite(VramOp),
    /// Read VRAM data for RDAT command.
    ReadVram(RdatRequest),
    /// Master GDC timing recomputed, bus should reschedule VSYNC events.
    TimingChanged,
}

save_state::runtime_state! {
/// Authoritative uPD7220 GDC state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GdcState {
    /// Status register.
    pub status: u8,
    /// Display enabled (DE flag).
    pub display_enabled: bool,
    /// Master mode (M flag from VSYNC cmd).
    pub master_mode: bool,
    /// Whether this is a slave GDC (light pen bit always set on read).
    pub is_slave: bool,

    /// Read FIFO state.
    pub fifo: FifoState,

    /// Command being parameterized.
    pub current_command: u8,
    /// Next parameter byte index.
    pub param_index: u8,
    /// Parameter accumulation buffer.
    pub param_buffer: [u8; 16],
    /// Parameter bytes remaining for the current command.
    pub params_remaining: u8,

    /// 18-bit execute word address.
    pub ead: u32,
    /// 4-bit dot address (bit position within mask).
    pub dad: u8,
    /// 18-bit light pen address.
    pub lad: u32,
    /// 16-bit mask register.
    pub mask: u16,
    /// 16-bit drawing pattern (from RA[8..9]).
    pub pattern: u16,
    /// Parameter RAM (partitions + font data).
    pub ra: [u8; 16],
    /// Words per line (9-bit on µPD7220A).
    pub pitch: u16,

    /// 3-bit direction (0-7).
    pub drawing_dir: u8,
    /// Figure type from FIGS P1[7:3].
    pub figure_type: u8,
    /// 14-bit DC parameter.
    pub drawing_dc: u16,
    /// 14-bit D parameter.
    pub drawing_d: u16,
    /// 14-bit D2 parameter.
    pub drawing_d2: u16,
    /// 14-bit D1 parameter.
    pub drawing_d1: u16,
    /// 14-bit DM parameter.
    pub drawing_dm: u16,
    /// GD flag (graphics drawing in mixed mode).
    pub drawing_gd: bool,
    /// Transfer mode (0=replace, 1=complement, 2=clear, 3=set).
    pub bitmap_mod: u8,

    /// Display mode from SYNC P1 (bits 1, 5).
    pub display_mode: u8,
    /// Interlace mode from SYNC P1 (bits 0, 3).
    pub interlace_mode: u8,
    /// Draw on retrace only (from SYNC P1 bit 4).
    pub draw_on_retrace: bool,

    /// Active words per line.
    pub aw: u16,
    /// Horizontal sync width.
    pub hs: u16,
    /// Vertical sync width.
    pub vs: u16,
    /// Horizontal front porch.
    pub hfp: u16,
    /// Horizontal back porch.
    pub hbp: u16,
    /// Vertical front porch.
    pub vfp: u16,
    /// Vertical back porch.
    pub vbp: u16,
    /// Active lines.
    pub al: u16,

    /// DC flag (display cursor).
    pub cursor_display: bool,
    /// SC flag (false=blinking, true=steady).
    pub cursor_blink: bool,
    /// Cursor top line within character row.
    pub cursor_top: u8,
    /// Cursor bottom line within character row.
    pub cursor_bottom: u8,
    /// Blink rate (5-bit).
    pub cursor_blink_rate: u8,
    /// Lines per character row (LR+1).
    pub lines_per_row: u8,

    /// Graphics character zoom (0-15).
    pub zoom_gchr: u8,
    /// Display zoom (0-15).
    pub zoom_display: u8,

    /// CPU clock frequency in Hz (needed for timing computation).
    pub cpu_clock_hz: u32,
    /// Crystal-derived dot clock in Hz (21052600 for 400-line, 14318180 for 200-line).
    pub dot_clock_hz: u32,
    /// Odd/even field counter for interlace (0 or 1).
    pub current_field: u8,

    /// VSYNC period in CPU cycles.
    pub vsync_period: u64,
    /// CPU cycles for the active display period.
    pub display_period: u64,
    /// CPU cycles for the VSYNC blanking period.
    pub vsync_blanking_period: u64,

    /// 4 scroll display partitions.
    pub scroll: [GdcScrollPartition; 4],

    /// VSYNC-driven blink counter.
    pub blink_counter: u16,

    /// RDAT needs more data from VRAM.
    pub rdat_pending: bool,
    /// Words remaining for RDAT.
    pub rdat_remaining: u16,
    /// Transfer type for RDAT.
    pub rdat_type: u8,

    /// DMA transfer is active.
    pub dma_active: bool,
    /// True for DMAW (write to VRAM), false for DMAR (read from VRAM).
    pub dma_is_write: bool,
    /// Transfer type (0=word, 2=low byte, 3=high byte).
    pub dma_type: u8,
    /// Transfer mode (0=replace, 1=complement, 2=clear, 3=set).
    pub dma_mod: u8,
    /// Bytes remaining in DMA transfer (u32 because max exceeds u16::MAX).
    pub dma_transfer_length: u32,
    /// Cached data word for multi-byte DMA transfers.
    pub dma_data: u16,
}}

/// µPD7220 Graphics Display Controller.
pub struct Gdc {
    /// Embedded state for save/restore.
    pub state: GdcState,
    /// Reusable buffer for drawing VramOps (avoids per-draw heap allocation).
    pub draw_buffer: Vec<VramOp>,
}

impl Deref for Gdc {
    type Target = GdcState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for Gdc {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Gdc {
    /// Creates a new master (text) GDC that owns the VSYNC timing.
    ///
    /// Computes initial timing from the default 400-line SYNC parameters
    /// (matching the VM boot state from µPD7220 spec section 5.4).
    pub fn new_master(cpu_clock_hz: u32) -> Self {
        let mut gdc = Self {
            state: GdcState {
                status: STATUS_FIFO_EMPTY,
                display_enabled: false,
                master_mode: true,
                is_slave: false,
                fifo: FifoState::new(),
                current_command: 0,
                param_index: 0,
                param_buffer: [0; 16],
                params_remaining: 0,
                ead: 0,
                dad: 0,
                lad: 0,
                mask: 0xFFFF,
                pattern: 0xFFFF,
                ra: [0; 16],
                pitch: MASTER_DEFAULT_PITCH,
                drawing_dir: 0,
                figure_type: 0,
                drawing_dc: 0,
                drawing_d: 8,
                drawing_d2: 8,
                drawing_d1: 0xFFFF,
                drawing_dm: 0,
                drawing_gd: false,
                bitmap_mod: 0,
                display_mode: DISPLAY_MODE_MIXED,
                interlace_mode: 0,
                draw_on_retrace: true,
                aw: 80,
                hs: 8,
                vs: 8,
                hfp: 10,
                hbp: 8,
                vfp: 7,
                vbp: 25,
                al: 400,
                cursor_display: false,
                cursor_blink: false,
                cursor_top: 0,
                cursor_bottom: 0,
                cursor_blink_rate: 0,
                lines_per_row: 1,
                zoom_gchr: 0,
                zoom_display: 0,
                cpu_clock_hz,
                dot_clock_hz: DOT_CLOCK_400LINE,
                current_field: 0,
                vsync_period: 0,
                display_period: 0,
                vsync_blanking_period: 0,
                scroll: [GdcScrollPartition::default(); 4],
                blink_counter: 0,
                rdat_pending: false,
                rdat_remaining: 0,
                rdat_type: 0,
                dma_active: false,
                dma_is_write: false,
                dma_type: 0,
                dma_mod: 0,
                dma_transfer_length: 0,
                dma_data: 0,
            },
            draw_buffer: Vec::new(),
        };
        gdc.recompute_timing();
        gdc
    }

    /// Creates a new slave (graphics) GDC with FIFO-empty status.
    pub fn new_slave(cpu_clock_hz: u32) -> Self {
        Self {
            state: GdcState {
                status: STATUS_FIFO_EMPTY,
                display_enabled: false,
                master_mode: false,
                is_slave: true,
                fifo: FifoState::new(),
                current_command: 0,
                param_index: 0,
                param_buffer: [0; 16],
                params_remaining: 0,
                ead: 0,
                dad: 0,
                lad: 0,
                mask: 0xFFFF,
                pattern: 0xFFFF,
                ra: [0; 16],
                pitch: SLAVE_DEFAULT_PITCH,
                drawing_dir: 0,
                figure_type: 0,
                drawing_dc: 0,
                drawing_d: 8,
                drawing_d2: 8,
                drawing_d1: 0xFFFF,
                drawing_dm: 0,
                drawing_gd: false,
                bitmap_mod: 0,
                display_mode: DISPLAY_MODE_MIXED,
                interlace_mode: 0,
                draw_on_retrace: false,
                aw: 0,
                hs: 0,
                vs: 0,
                hfp: 0,
                hbp: 0,
                vfp: 0,
                vbp: 0,
                al: 0,
                cursor_display: false,
                cursor_blink: false,
                cursor_top: 0,
                cursor_bottom: 0,
                cursor_blink_rate: 0,
                lines_per_row: 1,
                zoom_gchr: 0,
                zoom_display: 0,
                cpu_clock_hz,
                dot_clock_hz: 0,
                current_field: 0,
                vsync_period: 0,
                display_period: 0,
                vsync_blanking_period: 0,
                scroll: [GdcScrollPartition::default(); 4],
                blink_counter: 0,
                rdat_pending: false,
                rdat_remaining: 0,
                rdat_type: 0,
                dma_active: false,
                dma_is_write: false,
                dma_type: 0,
                dma_mod: 0,
                dma_transfer_length: 0,
                dma_data: 0,
            },
            draw_buffer: Vec::new(),
        }
    }

    /// Captures command, FIFO, drawing, DMA, and raster state.
    pub fn capture_state(&self) -> GdcState {
        self.state.clone()
    }

    /// Restores complete GDC state and clears drawing scratch storage.
    pub fn restore_state(
        &mut self,
        state: GdcState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.is_slave != self.state.is_slave
            || state.fifo.count > state.fifo.buffer.len() as u8
            || state.fifo.read_pos >= state.fifo.buffer.len() as u8
            || state.param_index as usize > state.param_buffer.len()
            || state.params_remaining as usize > state.param_buffer.len()
            || state.dad > 15
            || state.drawing_dir > 7
            || state.bitmap_mod > 3
            || state.dma_type > 3
            || state.dma_mod > 3
        {
            return Err(save_state::StateValidationError::new(
                "uPD7220 command or FIFO state is invalid",
            ));
        }
        self.state = state;
        self.draw_buffer.clear();
        Ok(())
    }

    /// Updates the dot clock and recomputes all timing parameters.
    pub fn set_dot_clock(&mut self, dot_clock_hz: u32) {
        self.state.dot_clock_hz = dot_clock_hz;
        self.recompute_timing();
    }

    /// Writes a command byte to the GDC. Returns an action that may require bus handling.
    pub fn write_command(&mut self, value: u8) -> GdcAction {
        self.params_remaining = self.command_param_count(value);
        self.current_command = value;
        self.param_index = 0;

        // Clear read FIFO on new command (direction switches to write).
        self.fifo.clear();
        self.status =
            (self.status & !(STATUS_DATA_READY | STATUS_FIFO_EMPTY)) | self.fifo.status_bits();
        self.rdat_pending = false;

        // Commands with 0 params execute immediately.
        if self.params_remaining == 0 {
            return self.apply_command();
        }

        // RESET: immediate effects on command byte, then optionally accepts 8 SYNC params.
        if value == 0x00 || value == 0x01 || value == 0x09 {
            return self.apply_reset_immediate(value);
        }

        // SYNC: DE flag set immediately from command byte bit 0.
        if value & 0xFE == 0x0E {
            self.display_enabled = value & 1 != 0;
        }

        // WDAT latches the transfer mode from the command byte immediately.
        // Real BIOS vector drawing changes clear/set mode this way, then
        // issues FIGD without sending new WDAT pattern bytes.
        if value & 0xE4 == 0x20 {
            self.bitmap_mod = value & 3;
        }

        GdcAction::None
    }

    /// Writes a parameter/data byte to the GDC. Returns an action that may require bus handling.
    pub fn write_data(&mut self, value: u8) -> GdcAction {
        if self.dma_active && self.dma_is_write {
            return self.dack_write(value);
        }

        let idx = self.param_index as usize;
        if idx < self.param_buffer.len() {
            self.param_buffer[idx] = value;
        }
        self.param_index += 1;
        self.params_remaining = self.params_remaining.saturating_sub(1);

        self.apply_incremental()
    }

    /// Reads and returns the status register.
    pub fn read_status(&mut self) -> u8 {
        let mut value = self.status;
        // Slave GDC always has light pen detect bit set.
        if self.is_slave {
            value |= STATUS_LIGHT_PEN;
        }
        value
    }

    /// Reads the next byte from the read FIFO.
    pub fn read_data(&mut self) -> u8 {
        let value = self.fifo.dequeue_byte();
        self.status =
            (self.status & !(STATUS_DATA_READY | STATUS_FIFO_EMPTY)) | self.fifo.status_bits();

        // If RDAT is pending and FIFO has space, continue filling.
        if self.rdat_pending && self.fifo.is_empty() {
            // Bus needs to provide more data. The bus checks rdat_pending.
        }

        value
    }

    /// Sets or clears the VSYNC status flag.
    pub fn set_vsync(&mut self, active: bool) {
        if active {
            self.status |= STATUS_VSYNC;
        } else {
            self.status &= !STATUS_VSYNC;
        }
    }

    /// Sets or clears the HBLANK status flag.
    pub fn set_hblank(&mut self, active: bool) {
        if active {
            self.status |= STATUS_HBLANK;
        } else {
            self.status &= !STATUS_HBLANK;
        }
    }

    /// Recomputes the HBLANK status flag from the time remaining until the
    /// next frame transition event.
    ///
    /// Real PC-98 software polls GDC status bit 6 directly while waiting for
    /// horizontal retrace. We model this from the remaining cycles to the
    /// next frame event, which preserves the correct phase relationship across
    /// both display and vertical blank intervals.
    pub fn update_hblank_status(&mut self, cycles_until_frame_event: Option<u64>) {
        if self.aw == 0 || self.al == 0 || self.dot_clock_hz == 0 || self.cpu_clock_hz == 0 {
            self.set_hblank(false);
            return;
        }

        let total_chars = u64::from(self.hs + self.hbp + self.aw + self.hfp);
        let horiz_mult: u64 = if self.display_mode == DISPLAY_MODE_GRAPHICS {
            16
        } else {
            8
        };
        let cpu = u64::from(self.cpu_clock_hz);
        let dot = u64::from(self.dot_clock_hz);
        let line_period = cpu * total_chars * horiz_mult / dot;
        let mut hblank_period = cpu * u64::from(self.hs) * horiz_mult / dot;

        if line_period == 0 {
            self.set_hblank(false);
            return;
        }

        if hblank_period == 0 && self.hs != 0 {
            hblank_period = 1;
        }

        let Some(remaining) = cycles_until_frame_event else {
            self.set_hblank(false);
            return;
        };

        self.set_hblank(remaining % line_period < hblank_period);
    }

    /// Handles the VSYNC event: sets the status flag, increments the blink counter,
    /// and toggles the field counter for interlace modes.
    pub fn on_vsync_event(&mut self) {
        self.set_vsync(true);
        self.state.blink_counter = self.state.blink_counter.wrapping_add(1);
        if self.state.interlace_mode == 0x08 || self.state.interlace_mode == 0x09 {
            self.state.current_field ^= 1;
        } else {
            self.state.current_field = 0;
        }
    }

    /// Clears the DRAWING status flag (called when drawing timing event fires).
    pub fn on_drawing_complete(&mut self) {
        self.status &= !STATUS_DRAWING;
    }

    /// Provides a VRAM word for RDAT continuation. Returns true if more words needed.
    pub fn provide_rdat_word(&mut self, word: u16) -> bool {
        if self.rdat_remaining == 0 {
            self.rdat_pending = false;
            return false;
        }

        match self.rdat_type {
            0 => {
                self.fifo.queue_byte(word as u8);
                self.fifo.queue_byte((word >> 8) as u8);
            }
            2 => {
                self.fifo.queue_byte(word as u8);
            }
            3 => {
                self.fifo.queue_byte((word >> 8) as u8);
            }
            _ => {}
        }

        self.rdat_remaining -= 1;
        self.next_pixel_for_rdat();

        if self.rdat_remaining == 0 {
            self.rdat_pending = false;
            self.reset_figs_params();
        }

        self.status =
            (self.status & !(STATUS_DATA_READY | STATUS_FIFO_EMPTY)) | self.fifo.status_bits();
        self.rdat_remaining > 0
    }

    /// Returns the next VRAM address needed for RDAT, if any.
    pub fn rdat_next_address(&self) -> Option<u32> {
        if self.rdat_pending && self.rdat_remaining > 0 {
            Some(self.ead & 0x3FFFF)
        } else {
            None
        }
    }

    /// Resets FIGS drawing parameters to defaults and reloads pattern from RA.
    fn reset_figs_params(&mut self) {
        self.drawing_dc = 0;
        self.drawing_d = 8;
        self.drawing_d1 = 0xFFFF;
        self.drawing_d2 = 8;
        self.drawing_dm = 0;
        self.drawing_gd = false;
        self.figure_type = 0;
        self.pattern = u16::from(self.ra[8]) | (u16::from(self.ra[9]) << 8);
    }

    /// Advances EAD by one word in the current direction (for RDAT).
    fn next_pixel_for_rdat(&mut self) {
        let pitch = self.get_effective_pitch();
        let dir = self.drawing_dir;
        drawing::advance_ead_dma(&mut self.ead, dir, pitch);
    }

    /// Returns effective pitch, halved if mixed mode with GD set.
    fn get_effective_pitch(&self) -> u16 {
        if self.display_mode == DISPLAY_MODE_MIXED && self.drawing_gd {
            self.pitch >> 1
        } else {
            self.pitch
        }
    }

    /// Returns the pattern data for a given drawing cycle.
    fn get_pattern(&self, cycle: u16) -> u16 {
        if self.display_mode == DISPLAY_MODE_MIXED && !self.drawing_gd {
            // Text/mixed mode without GD: return full 16-bit pattern.
            self.pattern
        } else {
            // Graphics mode or mixed with GD: per-bit extraction.
            if (self.pattern >> (cycle & 0xF)) & 1 != 0 {
                0xFFFF
            } else {
                0x0000
            }
        }
    }

    /// Starts a DMA transfer. Sets `dma_active` and the STATUS_DMA_EXECUTE bit.
    pub(crate) fn start_dma(&mut self) {
        if self.dma_active {
            return;
        }
        self.dma_active = true;
        self.status |= STATUS_DMA_EXECUTE;
    }

    /// Stops an active DMA transfer. Clears state and resets FIGS params.
    pub(crate) fn stop_dma(&mut self) {
        if !self.dma_active {
            return;
        }
        self.dma_active = false;
        self.status &= !STATUS_DMA_EXECUTE;
        self.reset_figs_params();
    }

    /// Handles a DACK write byte during DMAW. Returns a `GdcAction::Draw` when
    /// a complete VRAM word is assembled and ready to write.
    pub fn dack_write(&mut self, byte: u8) -> GdcAction {
        if !self.dma_active || !self.dma_is_write {
            return GdcAction::None;
        }

        let pitch = self.get_effective_pitch();
        let dir = self.drawing_dir;
        let action = match self.dma_type {
            0 => {
                // Word transfer: accumulate low byte, then write on high byte.
                if self.dma_transfer_length.is_multiple_of(2) {
                    // Even = first byte (low). Just accumulate.
                    self.dma_data = u16::from(byte);
                    GdcAction::None
                } else {
                    // Odd = second byte (high). Assemble word and write.
                    let word = self.dma_data | (u16::from(byte) << 8);
                    let op = VramOp {
                        address: self.ead & 0x3FFFF,
                        data: word & self.mask,
                        mask: self.mask,
                        mode: self.dma_mod,
                    };
                    drawing::advance_ead_dma(&mut self.ead, dir, pitch);
                    GdcAction::DackWrite(op)
                }
            }
            2 => {
                // Low byte only.
                let effective_mask = self.mask & 0x00FF;
                let op = VramOp {
                    address: self.ead & 0x3FFFF,
                    data: u16::from(byte) & effective_mask,
                    mask: effective_mask,
                    mode: self.dma_mod,
                };
                drawing::advance_ead_dma(&mut self.ead, dir, pitch);
                GdcAction::DackWrite(op)
            }
            3 => {
                // High byte only.
                let effective_mask = self.mask & 0xFF00;
                let op = VramOp {
                    address: self.ead & 0x3FFFF,
                    data: (u16::from(byte) << 8) & effective_mask,
                    mask: effective_mask,
                    mode: self.dma_mod,
                };
                drawing::advance_ead_dma(&mut self.ead, dir, pitch);
                GdcAction::DackWrite(op)
            }
            _ => GdcAction::None,
        };

        self.dma_transfer_length -= 1;
        if self.dma_transfer_length == 0 {
            self.stop_dma();
        }

        action
    }

    /// Returns the VRAM address needed for the next DMAR read byte, if any.
    /// Returns `None` when the cached high byte can be returned without a new read.
    pub fn dmar_next_address(&self) -> Option<u32> {
        if !self.dma_active || self.dma_is_write || self.dma_transfer_length == 0 {
            return None;
        }

        match self.dma_type {
            0 => {
                // Word: need a new VRAM word on even (first byte of pair).
                if self.dma_transfer_length.is_multiple_of(2) {
                    Some(self.ead & 0x3FFFF)
                } else {
                    None
                }
            }
            2 | 3 => Some(self.ead & 0x3FFFF),
            _ => None,
        }
    }

    /// Handles a DACK read during DMAR. `vram_word` should be `Some` when
    /// `dmar_next_address()` returned `Some`, otherwise `None`.
    pub fn dack_read(&mut self, vram_word: Option<u16>) -> u8 {
        if !self.dma_active || self.dma_is_write || self.dma_transfer_length == 0 {
            return 0;
        }

        let pitch = self.get_effective_pitch();
        let dir = self.drawing_dir;
        let byte = match self.dma_type {
            0 => {
                if self.dma_transfer_length.is_multiple_of(2) {
                    // Even = first byte (low). Store word, advance EAD.
                    let word = vram_word.unwrap_or(0);
                    self.dma_data = word;
                    drawing::advance_ead_dma(&mut self.ead, dir, pitch);
                    word as u8
                } else {
                    // Odd = second byte (high). Return cached high byte.
                    (self.dma_data >> 8) as u8
                }
            }
            2 => {
                let word = vram_word.unwrap_or(0);
                drawing::advance_ead_dma(&mut self.ead, dir, pitch);
                word as u8
            }
            3 => {
                let word = vram_word.unwrap_or(0);
                drawing::advance_ead_dma(&mut self.ead, dir, pitch);
                (word >> 8) as u8
            }
            _ => 0,
        };

        self.dma_transfer_length -= 1;
        if self.dma_transfer_length == 0 {
            self.stop_dma();
        }

        byte
    }

    /// Loads 8 SYNC parameter bytes into the parameter buffer and parses them.
    pub fn load_sync_params(&mut self, params: &[u8; 8]) {
        self.param_buffer[..8].copy_from_slice(params);
        self.param_index = 8;
        self.parse_sync_params();
    }

    /// Writes only the P1 (mode) byte of SYNC parameters, leaving P2-P8 unchanged.
    pub fn set_sync_mode_byte(&mut self, mode: u8) {
        self.param_buffer[0] = mode;
        self.param_index = self.param_index.max(8);
        self.display_mode = mode & DISPLAY_MODE_MASK;
        self.interlace_mode = (mode & 0x01) | (mode & 0x08);
        self.draw_on_retrace = mode & 0x10 != 0;
        self.recompute_timing();
    }

    /// Recomputes display timing from current SYNC parameters and dot clock.
    ///
    /// Derives vsync_period, display_period, and vsync_blanking_period from
    /// the character clock (dot_clock / 8), horizontal/vertical totals, and
    /// the CPU clock frequency.
    pub fn recompute_timing(&mut self) {
        if self.aw == 0 || self.al == 0 || self.dot_clock_hz == 0 || self.cpu_clock_hz == 0 {
            return;
        }
        let vert_mult: u64 = if self.interlace_mode == 0x09 { 2 } else { 1 };
        let total_chars = u64::from(self.hs + self.hbp + self.aw + self.hfp);
        let total_lines =
            (u64::from(self.vs + self.vbp + self.vfp) + u64::from(self.al)) * vert_mult;
        let active_lines = u64::from(self.al) * vert_mult;

        // dot_clock / 8 = character clock (master GDC text mode: 8 dots per char)
        // frame_chars = total_chars * total_lines
        // frame_time_s = frame_chars / char_clock = frame_chars * 8 / dot_clock
        // vsync_period = frame_time_s * cpu_clock
        let cpu = u64::from(self.cpu_clock_hz);
        let dot = u64::from(self.dot_clock_hz);
        let horiz_mult: u64 = if self.display_mode == DISPLAY_MODE_GRAPHICS {
            16
        } else {
            8
        };
        self.vsync_period = cpu * total_chars * total_lines * horiz_mult / dot;

        if total_lines == 0 || self.vsync_period == 0 {
            return;
        }
        self.display_period = self.vsync_period * active_lines / total_lines;
        self.vsync_blanking_period = self.vsync_period - self.display_period;
    }

    /// Updates scroll partition data from current Parameter RAM contents.
    fn update_scroll_partitions(&mut self, is_graphics: bool) {
        for i in 0..4 {
            let base = i * 4;
            if base + 3 < 16 {
                let p0 = self.ra[base];
                let p1 = self.ra[base + 1];
                let p2 = self.ra[base + 2];
                let p3 = self.ra[base + 3];

                if is_graphics {
                    // 18-bit address for graphics.
                    self.scroll[i].start_address =
                        u32::from(p0) | (u32::from(p1) << 8) | (u32::from(p2 & 0x03) << 16);
                } else {
                    // 13-bit address for text.
                    self.scroll[i].start_address = u32::from(p0) | (u32::from(p1 & 0x1F) << 8);
                }
                self.scroll[i].line_count = (u16::from(p3 & 0x3F) << 4) | (u16::from(p2) >> 4);
                if self.scroll[i].line_count == 0 {
                    self.scroll[i].line_count = 0x400;
                }
                self.scroll[i].im = p3 & 0x40 != 0;
                self.scroll[i].wd = p3 & 0x80 != 0;
            }
        }
    }
}
