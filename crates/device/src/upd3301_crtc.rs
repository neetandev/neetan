//! uPD3301 character display controller for the PC-8801.
//!
//! The CRTC is programmed through ports 0x50 (parameter) and 0x51 (command on
//! write, status on read). It owns the text screen geometry, cursor, blink, and
//! the per-frame character/attribute buffer that the uPD8257 DMA channel 2 fills
//! one row at a time. At end of display the machine calls [`Upd3301::expand_buffer`]
//! to decode the captured byte stream into per-cell character and attribute
//! planes for the renderer.
//!
//! The PC-88 raises the vertical-retrace interrupt through machine wiring rather
//! than the CRTC interrupt pin; this device only tracks the end-of-frame status
//! bit and the interrupt mask the ROM programs.

use std::ops::{Deref, DerefMut};

/// Maximum displayed character columns.
pub const MAX_COLUMNS: usize = 80;
/// Maximum displayed character rows held in the expanded planes.
pub const MAX_ROWS: usize = 200;
/// Maximum attribute strip pairs per row.
pub const MAX_ATTRIBUTES: usize = 20;
/// Captured DMA buffer size (matches the reference 120 bytes x 200 rows).
const BUFFER_SIZE: usize = 120 * MAX_ROWS;
/// Power-of-two index mask for the capture buffer (kept below `BUFFER_SIZE`).
const BUFFER_MASK: usize = 0x3FFF;
/// Expanded plane size (one byte per character cell).
const EXPAND_SIZE: usize = MAX_ROWS * MAX_COLUMNS;

/// Status bit: display enabled (set by START DISPLAY, cleared by RESET).
pub const STATUS_DISPLAY_ENABLE: u8 = 0x10;
/// Status bit: DMA underrun during the frame.
pub const STATUS_UNDERRUN: u8 = 0x08;
/// Status bit: end-of-frame (vertical retrace) reached.
pub const STATUS_END_OF_FRAME: u8 = 0x02;
/// Status bit: light pen detected (unsupported; always clear).
pub const STATUS_LIGHT_PEN: u8 = 0x01;

const COMMAND_RESET: u8 = 0;
const COMMAND_START_DISPLAY: u8 = 1;
const COMMAND_SET_INTERRUPT_MASK: u8 = 2;
const COMMAND_READ_LIGHT_PEN: u8 = 3;
const COMMAND_LOAD_CURSOR: u8 = 4;
const COMMAND_RESET_INTERRUPT: u8 = 5;
const COMMAND_RESET_COUNTERS: u8 = 6;

save_state::runtime_state! {
/// Snapshot of the uPD3301 state for save/restore.
#[derive(Clone)]
pub struct Upd3301State {
    /// Displayed columns per row (2..=80).
    pub columns: u8,
    /// Displayed character rows (1..=64).
    pub rows: u8,
    /// Scanlines per character row (1..=32).
    pub char_height: u8,
    /// Lines of vertical retrace (1..=8).
    pub vretrace: u8,
    /// Pseudo-interlace skip-line flag.
    pub skip_line: bool,
    /// Attribute strip pairs per row (0..=20).
    pub attribute_count: u8,
    /// Display mode bits (bit0 no-attribute, bit1 color, bit2 non-transparent).
    pub display_mode: u8,
    /// Blink period in frames.
    pub blink_rate: u16,
    /// Global reverse-video flag (RVV) from START DISPLAY.
    pub reverse: u8,
    /// Interrupt mask (bit0 masks end-of-frame).
    pub interrupt_mask: u8,
    /// Status register.
    pub status: u8,
    /// Cursor column, or -1 when unset.
    pub cursor_x: i32,
    /// Cursor row, or -1 when unset.
    pub cursor_y: i32,
    /// Active cursor type, or -1 when the cursor is off.
    pub cursor_type: i32,
    /// Programmed cursor mode (0..=3).
    pub cursor_mode: i32,
    /// Whether end-of-display has been reached for the current frame.
    pub vblank: bool,
    /// Geometry changed since the last timing recompute.
    pub timing_changed: bool,
    /// Current command being assembled.
    pub current_command: u8,
    /// Parameter index within the current command.
    pub param_index: u8,
    /// Blink frame counter.
    pub blink_counter: u16,
    /// Whether blink-attributed cells are currently visible.
    pub blink_attrib_visible: bool,
    /// Whether the cursor is currently visible in its blink phase.
    pub blink_cursor_visible: bool,
    /// Captured DMA byte stream for the current frame.
    pub buffer: Box<[u8; BUFFER_SIZE]>,
    /// Write cursor into `buffer`.
    pub buffer_ptr: usize,
    /// Expanded per-cell character codes (row-major, `MAX_ROWS` x `MAX_COLUMNS`).
    pub text_expand: Box<[u8; EXPAND_SIZE]>,
    /// Expanded per-cell attributes (row-major, `MAX_ROWS` x `MAX_COLUMNS`).
    pub attrib_expand: Box<[u8; EXPAND_SIZE]>,
    /// Running attribute byte assembled during expansion.
    attrib_data: u8,
    /// Running attribute mask assembled during expansion.
    attrib_mask: u8,
}}

/// uPD3301 character display controller.
pub struct Upd3301 {
    /// Embedded state for save/restore.
    pub state: Upd3301State,
}

impl Deref for Upd3301 {
    type Target = Upd3301State;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for Upd3301 {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Upd3301 {
    /// Creates a controller in its power-on reset state. `hireso` selects the
    /// 24 kHz (400-line) character height and retrace defaults.
    pub fn new(hireso: bool) -> Self {
        let mut crtc = Self {
            state: Upd3301State {
                columns: 80,
                rows: 25,
                char_height: 8,
                vretrace: 7,
                skip_line: false,
                attribute_count: MAX_ATTRIBUTES as u8,
                display_mode: 0,
                blink_rate: 24,
                reverse: 0,
                interrupt_mask: 3,
                status: 0,
                cursor_x: -1,
                cursor_y: -1,
                cursor_type: -1,
                cursor_mode: -1,
                vblank: false,
                timing_changed: false,
                current_command: 0,
                param_index: 0,
                blink_counter: 0,
                blink_attrib_visible: false,
                blink_cursor_visible: false,
                buffer: vec![0u8; BUFFER_SIZE]
                    .into_boxed_slice()
                    .try_into()
                    .unwrap(),
                buffer_ptr: 0,
                text_expand: vec![0u8; EXPAND_SIZE]
                    .into_boxed_slice()
                    .try_into()
                    .unwrap(),
                attrib_expand: vec![0u8; EXPAND_SIZE]
                    .into_boxed_slice()
                    .try_into()
                    .unwrap(),
                attrib_data: 0xE0,
                attrib_mask: 0xFF,
            },
        };
        crtc.reset(hireso);
        crtc
    }

    /// Resets the geometry, cursor, attribute, and blink defaults.
    pub fn reset(&mut self, hireso: bool) {
        self.state.blink_rate = 24;
        self.state.cursor_type = -1;
        self.state.cursor_mode = -1;
        self.state.cursor_x = -1;
        self.state.cursor_y = -1;
        self.state.attrib_data = 0xE0;
        self.state.attrib_mask = 0xFF;
        self.state.attribute_count = MAX_ATTRIBUTES as u8;
        self.state.columns = 80;
        self.state.rows = 25;
        self.state.char_height = if hireso { 16 } else { 8 };
        self.state.skip_line = false;
        self.state.vretrace = if hireso { 3 } else { 7 };
        self.state.timing_changed = false;
        self.state.reverse = 0;
        self.state.interrupt_mask = 3;
    }

    /// Total scanlines per frame (display plus retrace).
    pub fn total_lines(&self) -> u32 {
        (u32::from(self.state.rows) + u32::from(self.state.vretrace))
            * u32::from(self.state.char_height)
    }

    /// Active display scanlines.
    pub fn display_lines(&self) -> u32 {
        u32::from(self.state.rows) * u32::from(self.state.char_height)
    }

    /// Bytes the DMA delivers per character row (characters plus attribute pairs).
    pub fn bytes_per_row(&self) -> u32 {
        u32::from(self.state.columns).max(MAX_COLUMNS as u32)
            + u32::from(self.state.attribute_count) * 2
    }

    /// Returns the expanded per-cell character codes (`MAX_ROWS` x `MAX_COLUMNS`).
    pub fn text_expand(&self) -> &[u8] {
        self.state.text_expand.as_slice()
    }

    /// Returns the expanded per-cell attributes (`MAX_ROWS` x `MAX_COLUMNS`).
    pub fn attrib_expand(&self) -> &[u8] {
        self.state.attrib_expand.as_slice()
    }

    /// Handles a command write (port 0x51).
    pub fn write_command(&mut self, data: u8) {
        self.state.current_command = (data >> 5) & 7;
        self.state.param_index = 0;
        match self.state.current_command {
            COMMAND_RESET => {
                self.state.status &=
                    !(STATUS_DISPLAY_ENABLE | STATUS_UNDERRUN | STATUS_END_OF_FRAME);
                self.state.cursor_x = -1;
                self.state.cursor_y = -1;
            }
            COMMAND_START_DISPLAY => {
                self.state.reverse = data & 1;
                self.state.status |= STATUS_DISPLAY_ENABLE;
                self.state.status &= !STATUS_UNDERRUN;
            }
            COMMAND_SET_INTERRUPT_MASK => {
                if data & 1 == 0 {
                    self.state.status = 0;
                }
                self.state.interrupt_mask = data & 3;
            }
            COMMAND_READ_LIGHT_PEN => {
                self.state.status &= !STATUS_LIGHT_PEN;
            }
            COMMAND_LOAD_CURSOR => {
                self.state.cursor_type = if data & 1 != 0 {
                    self.state.cursor_mode
                } else {
                    -1
                };
            }
            COMMAND_RESET_INTERRUPT | COMMAND_RESET_COUNTERS => {
                self.state.status &= !(STATUS_END_OF_FRAME | STATUS_LIGHT_PEN);
            }
            _ => {}
        }
    }

    /// Handles a parameter write (port 0x50).
    pub fn write_parameter(&mut self, data: u8) {
        match self.state.current_command {
            COMMAND_RESET => match self.state.param_index {
                0 => self.state.columns = ((data & 0x7F) + 2).min(MAX_COLUMNS as u8),
                1 => {
                    let rows = (data & 0x3F) + 1;
                    if self.state.rows != rows {
                        self.state.rows = rows;
                        self.state.timing_changed = true;
                    }
                    self.state.blink_rate = 32 * (u16::from(data >> 6) + 1);
                }
                2 => {
                    let char_height = (data & 0x1F) + 1;
                    if self.state.char_height != char_height {
                        self.state.char_height = char_height;
                        self.state.timing_changed = true;
                    }
                    self.state.cursor_mode = i32::from((data >> 5) & 3);
                    self.state.skip_line = data & 0x80 != 0;
                }
                3 => {
                    let vretrace = ((data >> 5) & 7) + 1;
                    if self.state.vretrace != vretrace {
                        self.state.vretrace = vretrace;
                        self.state.timing_changed = true;
                    }
                }
                4 => {
                    self.state.display_mode = (data >> 5) & 7;
                    self.state.attribute_count = if self.state.display_mode & 1 != 0 {
                        0
                    } else {
                        ((data & 0x1F) + 1).min(MAX_ATTRIBUTES as u8)
                    };
                }
                _ => {}
            },
            COMMAND_LOAD_CURSOR => match self.state.param_index {
                0 => self.state.cursor_x = i32::from(data),
                1 => self.state.cursor_y = i32::from(data),
                _ => {}
            },
            _ => {}
        }
        self.state.param_index = self.state.param_index.wrapping_add(1);
    }

    /// Reads a parameter byte (port 0x50). Light-pen reads return 0; other
    /// commands return the status byte.
    pub fn read_parameter(&mut self) -> u8 {
        let value = if self.state.current_command == COMMAND_READ_LIGHT_PEN {
            0
        } else {
            self.read_status()
        };
        self.state.param_index = self.state.param_index.wrapping_add(1);
        value
    }

    /// Reads the status register (port 0x51). During an underrun the display
    /// enable bit reads back clear.
    pub fn read_status(&self) -> u8 {
        if self.state.status & STATUS_UNDERRUN != 0 {
            self.state.status & !STATUS_DISPLAY_ENABLE
        } else {
            self.state.status
        }
    }

    /// Resets the capture buffer at the start of a display frame.
    pub fn start_frame(&mut self) {
        self.state.buffer.fill(0);
        self.state.buffer_ptr = 0;
        self.state.vblank = false;
    }

    /// Appends a DMA byte to the capture buffer (the DACK sink).
    pub fn push_dma_byte(&mut self, data: u8) {
        self.state.buffer[self.state.buffer_ptr & BUFFER_MASK] = data;
        self.state.buffer_ptr = self.state.buffer_ptr.wrapping_add(1);
    }

    /// Marks the end of display: sets the end-of-frame status when enabled and
    /// unmasked, and latches the vblank flag.
    pub fn finish_frame(&mut self) {
        if self.state.status & STATUS_DISPLAY_ENABLE != 0 && self.state.interrupt_mask & 1 == 0 {
            self.state.status |= STATUS_END_OF_FRAME;
        }
        self.state.vblank = true;
    }

    /// Advances the blink counter and recomputes the attribute and cursor blink
    /// visibility for the new frame.
    pub fn update_blink(&mut self) {
        self.state.blink_counter += 1;
        if self.state.blink_counter > self.state.blink_rate {
            self.state.blink_counter = 0;
        }
        let counter = self.state.blink_counter;
        let rate = self.state.blink_rate;
        self.state.blink_attrib_visible = counter < rate / 4;
        self.state.blink_cursor_visible =
            counter <= rate / 4 || (rate / 2 <= counter && counter <= 3 * rate / 4);
    }

    fn read_buffer(&mut self, offset: usize) -> u8 {
        if offset < self.state.buffer_ptr {
            self.state.buffer[offset & BUFFER_MASK]
        } else {
            self.state.status |= STATUS_UNDERRUN;
            0
        }
    }

    /// Decodes the captured byte stream into the expanded character and
    /// attribute planes for the current frame.
    pub fn expand_buffer(&mut self, hireso: bool, line_400: bool) {
        let mut char_height_step = i32::from(self.state.char_height);
        if !hireso {
            char_height_step <<= 1;
        }
        if line_400 || !self.state.skip_line {
            char_height_step >>= 1;
        }
        let char_height_step = char_height_step.max(1);

        let row_stride = MAX_COLUMNS + usize::from(self.state.attribute_count) * 2;
        let rows = usize::from(self.state.rows);
        let columns = usize::from(self.state.columns);
        let blink_attrib = if self.state.blink_attrib_visible {
            0
        } else {
            2
        };

        let mut exit_line: i32 = -1;
        if self.state.status & STATUS_DISPLAY_ENABLE == 0 {
            exit_line = 0;
        } else {
            // Character codes.
            let mut ytop = 0i32;
            let mut offset = 0usize;
            for cy in 0..rows {
                if ytop >= MAX_ROWS as i32 {
                    break;
                }
                for cx in 0..columns {
                    let value = self.read_buffer(offset + cx);
                    self.state.text_expand[cy * MAX_COLUMNS + cx] = value;
                }
                if self.state.status & STATUS_UNDERRUN != 0 && exit_line == -1 {
                    exit_line = cy as i32;
                }
                ytop += char_height_step;
                offset += row_stride;
            }

            // Attributes.
            if self.state.display_mode & 4 != 0 {
                self.expand_non_transparent(
                    rows,
                    columns,
                    row_stride,
                    char_height_step,
                    blink_attrib,
                    &mut exit_line,
                );
            } else if self.state.display_mode & 1 != 0 {
                self.state.attrib_expand.fill(0xE0);
            } else {
                self.expand_transparent(
                    rows,
                    columns,
                    row_stride,
                    char_height_step,
                    blink_attrib,
                    &mut exit_line,
                );
            }

            self.apply_cursor();
        }

        if exit_line != -1 {
            for cy in (exit_line as usize)..MAX_ROWS {
                let base = cy * MAX_COLUMNS;
                for cx in 0..MAX_COLUMNS {
                    self.state.text_expand[base + cx] = 0;
                    self.state.attrib_expand[base + cx] = 0xE0;
                }
            }
        }
    }

    fn expand_non_transparent(
        &mut self,
        rows: usize,
        columns: usize,
        row_stride: usize,
        char_height_step: i32,
        blink_attrib: u8,
        exit_line: &mut i32,
    ) {
        let mut ytop = 0i32;
        let mut offset = 0usize;
        for cy in 0..rows {
            if ytop >= MAX_ROWS as i32 {
                break;
            }
            let mut cx = 0;
            while cx < columns {
                let code = self.read_buffer(offset + cx + 1);
                self.set_attrib(code, blink_attrib);
                let value = self.state.attrib_data & self.state.attrib_mask;
                self.state.attrib_expand[cy * MAX_COLUMNS + cx] = value;
                if cx + 1 < MAX_COLUMNS {
                    self.state.attrib_expand[cy * MAX_COLUMNS + cx + 1] = value;
                }
                cx += 2;
            }
            if self.state.status & STATUS_UNDERRUN != 0 && *exit_line == -1 {
                *exit_line = cy as i32;
            }
            ytop += char_height_step;
            offset += row_stride;
        }
    }

    fn expand_transparent(
        &mut self,
        rows: usize,
        columns: usize,
        row_stride: usize,
        char_height_step: i32,
        blink_attrib: u8,
        exit_line: &mut i32,
    ) {
        let attribute_count = usize::from(self.state.attribute_count);
        let mut ytop = 0i32;
        let mut offset = 0usize;
        for cy in 0..rows {
            if ytop >= MAX_ROWS as i32 {
                break;
            }
            let mut flags = [0u8; 128];
            if attribute_count > 0 {
                let mut i = 2 * (attribute_count - 1);
                loop {
                    let column = (self.read_buffer(offset + i + MAX_COLUMNS) & 0x7F) as usize;
                    flags[column] = 1;
                    if i == 0 {
                        break;
                    }
                    i -= 2;
                }
            }
            let mut pos = 0usize;
            for (cx, &flag) in flags.iter().take(columns).enumerate() {
                if flag != 0 {
                    let code = self.read_buffer(offset + pos + MAX_COLUMNS + 1);
                    self.set_attrib(code, blink_attrib);
                    pos += 2;
                }
                self.state.attrib_expand[cy * MAX_COLUMNS + cx] =
                    self.state.attrib_data & self.state.attrib_mask;
            }
            if self.state.status & STATUS_UNDERRUN != 0 && *exit_line == -1 {
                *exit_line = cy as i32;
            }
            ytop += char_height_step;
            offset += row_stride;
        }
    }

    fn apply_cursor(&mut self) {
        if self.state.cursor_x < MAX_COLUMNS as i32
            && self.state.cursor_y < MAX_ROWS as i32
            && self.state.cursor_x >= 0
            && self.state.cursor_y >= 0
        {
            if self.state.cursor_type & 1 != 0 && self.state.blink_cursor_visible {
                return;
            }
            const CURSOR_TYPE_XOR: [u8; 5] = [0, 8, 8, 1, 1];
            let index = (self.state.cursor_type + 1).clamp(0, 4) as usize;
            let cell = (self.state.cursor_y as usize) * MAX_COLUMNS + self.state.cursor_x as usize;
            self.state.attrib_expand[cell] ^= CURSOR_TYPE_XOR[index];
        }
    }

    fn set_attrib(&mut self, code: u8, blink_attrib: u8) {
        let blink = if code & 2 != 0 && code & 1 == 0 {
            blink_attrib
        } else {
            0
        };
        if self.state.display_mode & 2 != 0 {
            if code & 8 != 0 {
                self.state.attrib_data = (self.state.attrib_data & 0x0F) | (code & 0xF0);
                self.state.attrib_mask = 0xF3;
            } else {
                self.state.attrib_data =
                    (self.state.attrib_data & 0xF0) | ((code >> 2) & 0x0D) | ((code << 1) & 2);
                self.state.attrib_data ^= self.state.reverse;
                self.state.attrib_data ^= blink;
                self.state.attrib_mask = 0xFF;
            }
        } else {
            self.state.attrib_data =
                0xE0 | ((code >> 3) & 0x10) | ((code >> 2) & 0x0D) | ((code << 1) & 2);
            self.state.attrib_data ^= self.state.reverse;
            self.state.attrib_data ^= blink;
            self.state.attrib_mask = 0xFF;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn programmed_crtc() -> Upd3301 {
        let mut crtc = Upd3301::new(false);
        // RESET command with five parameters: 80 columns, 25 rows, 8-line cells,
        // 7 retrace lines, non-transparent monochrome with 20 attributes.
        crtc.write_command(0x00);
        crtc.write_parameter(80 - 2); // columns
        crtc.write_parameter(25 - 1); // rows, blink rate 0
        crtc.write_parameter(8 - 1); // char height, cursor mode 0
        crtc.write_parameter((7 - 1) << 5); // vretrace
        crtc.write_parameter((4 << 5) | (20 - 1)); // non-transparent, 20 attrs
        crtc
    }

    #[test]
    fn reset_geometry_decode() {
        let crtc = programmed_crtc();
        assert_eq!(crtc.columns, 80);
        assert_eq!(crtc.rows, 25);
        assert_eq!(crtc.char_height, 8);
        assert_eq!(crtc.vretrace, 7);
        assert_eq!(crtc.attribute_count, 20);
        assert_eq!(crtc.display_mode, 4);
        assert_eq!(crtc.total_lines(), (25 + 7) * 8);
        assert_eq!(crtc.display_lines(), 200);
        assert_eq!(crtc.bytes_per_row(), 80 + 20 * 2);
    }

    #[test]
    fn status_transitions() {
        let mut crtc = programmed_crtc();
        assert_eq!(crtc.status & STATUS_DISPLAY_ENABLE, 0);

        crtc.write_command(0x40); // SET INTERRUPT MASK, unmask end-of-frame (bit0=0)
        crtc.write_command(0x20); // START DISPLAY
        assert_ne!(crtc.status & STATUS_DISPLAY_ENABLE, 0);

        crtc.finish_frame();
        assert_ne!(crtc.status & STATUS_END_OF_FRAME, 0);

        // RESET clears display and end-of-frame.
        crtc.write_command(0x00);
        assert_eq!(crtc.status & STATUS_DISPLAY_ENABLE, 0);
        assert_eq!(crtc.status & STATUS_END_OF_FRAME, 0);
    }

    #[test]
    fn end_of_frame_gated_by_interrupt_mask() {
        let mut crtc = programmed_crtc();
        crtc.write_command(0x20); // START DISPLAY
        crtc.write_command(0x41); // SET INTERRUPT MASK, mask end-of-frame (bit0=1)
        crtc.finish_frame();
        assert_eq!(crtc.status & STATUS_END_OF_FRAME, 0);
    }

    #[test]
    fn underrun_masks_display_enable_on_read() {
        let mut crtc = programmed_crtc();
        crtc.write_command(0x20);
        crtc.state.status |= STATUS_UNDERRUN;
        assert_eq!(crtc.read_status() & STATUS_DISPLAY_ENABLE, 0);
    }

    #[test]
    fn expand_transparent_strip_and_reverse() {
        // 4 columns, 1 row, 1-line cells, retrace 1, transparent color mode with
        // one attribute strip.
        let mut crtc = Upd3301::new(false);
        crtc.write_command(0x00); // RESET
        crtc.write_parameter(4 - 2); // columns = 4
        crtc.write_parameter(0); // rows = 1
        crtc.write_parameter(0); // char height = 1
        crtc.write_parameter(0); // vretrace = 1
        crtc.write_parameter(2 << 5); // color, transparent, 1 attribute
        crtc.write_command(0x20); // START DISPLAY

        // One row: 80 character bytes window then the attribute pairs. Characters
        // 'A','B','C','D' at columns 0..4.
        crtc.start_frame();
        let mut row = vec![0u8; 80 + 2];
        row[0] = b'A';
        row[1] = b'B';
        row[2] = b'C';
        row[3] = b'D';
        // attribute pair: column 0, value 0xE8 (color bits set, code&8 -> color).
        row[80] = 0; // column address
        row[81] = 0xE8; // color attribute (bit3 set -> color = 0xE0 high nibble)
        for byte in &row {
            crtc.push_dma_byte(*byte);
        }
        crtc.expand_buffer(false, false);

        assert_eq!(crtc.text_expand()[0], b'A');
        assert_eq!(crtc.text_expand()[3], b'D');
        // Color attribute high nibble preserved.
        assert_eq!(crtc.attrib_expand()[0] & 0xF0, 0xE0);
    }

    #[test]
    fn blink_cadence() {
        let mut crtc = Upd3301::new(false);
        crtc.state.blink_rate = 8;
        crtc.state.blink_counter = 0;
        crtc.update_blink();
        assert_eq!(crtc.blink_counter, 1);
        assert!(crtc.blink_attrib_visible);
    }
}
