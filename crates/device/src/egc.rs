//! EGC (Enhanced Graphic Charger), the raster operation engine between the CPU
//! and the four graphics planes.
//!
//! Eight word registers sit at ports 0x04A0-0x04AE. A CPU access to graphics
//! VRAM in EGC mode reaches all planes at once. A write combines the shifter
//! output (S), the destination VRAM (D) and the pattern (P) with an 8-bit
//! raster operation, or writes the CPU data or the pattern directly.
//!
//! The shifter is a bit stream per plane. Every input (a VRAM read, or a CPU
//! write when the CPU is the source) appends its pixels in the direction of
//! the shift register and then takes one output unit from the stream into the
//! source latch S. The first input of an operation skips the source bit
//! address. The first output unit and the first write skip the destination
//! bit address. An output unit waits in the stream until it holds enough
//! pixels. A source bit address above the destination bit address leaves the
//! first input without a complete unit. The first write of such an operation
//! writes nothing unless a later input completed a unit. An operation covers
//! the bit length plus one pixel, counted by the writes.

/// Register 1 bits 3-0: P3EN#-P0EN#, 1 = the plane is not written.
const ACCESS_PLANE_DISABLE: u16 = 0x0001;

/// Register 2 bits 14-13: source of the pattern input P.
const FGBG_PATTERN_SOURCE_MASK: u16 = 0x6000;

/// Register 2 bits 14-13 = 01b: P is the background color.
const FGBG_BACKGROUND: u16 = 0x2000;

/// Register 2 bits 14-13 = 10b: P is the foreground color.
const FGBG_FOREGROUND: u16 = 0x4000;

/// Register 2 bits 14-13 = 11b: P is the foreground color in planes 0 and 1
/// and the background color in planes 2 and 3.
const FGBG_MIXED: u16 = 0x6000;

/// First plane that takes the background color in the mixed pattern source.
const MIXED_FIRST_BACKGROUND_PLANE: usize = 2;

/// Register 2 bits 9-8 of the read plane field (bits 11-8): plane returned by
/// a read.
const FGBG_READ_PLANE_MASK: u16 = 0x0300;

/// Shift of the read plane field in register 2.
const FGBG_READ_PLANE_SHIFT: u32 = 8;

/// Register 3 bit 13: compare read.
const OPE_COMPARE_READ: u16 = 0x2000;

/// Register 3 bits 12-11: write source.
const OPE_WRITE_SOURCE_MASK: u16 = 0x1800;

/// Register 3 bits 12-11 = 00b: the CPU data is written.
const OPE_WRITE_CPU: u16 = 0x0000;

/// Register 3 bits 12-11 = 01b: the raster operation result is written.
const OPE_WRITE_ROP: u16 = 0x0800;

/// Register 3 bits 12-11 = 10b: the pattern is written.
const OPE_WRITE_PATTERN: u16 = 0x1000;

/// Register 3 bit 10: the shifter takes the CPU write data. Clear means VRAM
/// read data.
const OPE_SOURCE_CPU: u16 = 0x0400;

/// Register 3 bits 9-8: pattern register load.
const OPE_PATTERN_LOAD_MASK: u16 = 0x0300;

/// Register 3 bits 9-8 = 01b: a VRAM read loads the pattern register.
const OPE_PATTERN_LOAD_ON_READ: u16 = 0x0100;

/// Register 3 bits 9-8 = 10b: a VRAM write loads the pattern register with the
/// VRAM data before the write.
const OPE_PATTERN_LOAD_ON_WRITE: u16 = 0x0200;

/// Register 3 bits 7-0: raster operation code.
const OPE_ROP_MASK: u16 = 0x00FF;

/// Mask of a 4-bit color in the foreground and background registers.
const COLOR_MASK: u16 = 0x000F;

/// Register 7 bit 12: 1 = descending bit order (right to left).
const SFT_DESCENDING: u16 = 0x1000;

/// Register 7 bits 7-4: destination bit address.
const SFT_DESTINATION_MASK: u16 = 0x00F0;

/// Shift of the destination bit address in register 7.
const SFT_DESTINATION_SHIFT: u32 = 4;

/// Register 7 bits 3-0: source bit address.
const SFT_SOURCE_MASK: u16 = 0x000F;

/// Register 8 bits 11-0: bit length minus one.
const LENG_MASK: u16 = 0x0FFF;

/// Number of planes.
const PLANE_COUNT: usize = 4;

/// Low bits of a bit address that address a pixel within a byte.
const BYTE_BIT_ADDRESS_MASK: u16 = 0x0007;

/// Shifter class of equal bit addresses within a byte.
const SHIFT_CLASS_ALIGNED: u8 = 0;

/// Shifter class of a destination bit address above the source bit address.
const SHIFT_CLASS_RIGHT: u8 = 2;

/// Shifter class of a source bit address above the destination bit address.
const SHIFT_CLASS_LEFT: u8 = 4;

/// Pixel width of a byte access.
const BYTE_WIDTH: u32 = 8;

/// Pixel width of a word access.
const WORD_WIDTH: u32 = 16;

/// Stream capacity per plane in pixels.
const STREAM_CAPACITY_BITS: u32 = 8 * WORD_WIDTH;

/// Stream read position from which the unread pixels shift back to the start.
const STREAM_COMPACT_BITS: u32 = 4 * WORD_WIDTH;

/// Stream bytes per plane, with room for a 3-byte access window at the end.
const STREAM_PLANE_BYTES: usize = (STREAM_CAPACITY_BITS / 8) as usize + 3;

/// Register 1 after reset: all planes written.
const ACCESS_RESET: u16 = 0xFFF0;

/// Register 2 after reset.
const FGBG_RESET: u16 = 0x00FF;

/// Register 5 after reset: all bits written.
const MASK_RESET: u16 = 0xFFFF;

/// Register 8 after reset: 16 pixels.
const LENG_RESET: u16 = 0x000F;

save_state::runtime_state! {
/// EGC state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgcState {
    /// Register 1 (0x04A0): plane write disable bits.
    pub access: u16,
    /// Register 2 (0x04A2): pattern source and read plane.
    pub fgbg: u16,
    /// Register 3 (0x04A4): compare read, write source, shifter source, pattern
    /// load and raster operation.
    pub ope: u16,
    /// Register 4 (0x04A6): foreground color.
    pub fg: u16,
    /// Register 5 (0x04A8): bit write mask, 1 = written.
    pub mask: u16,
    /// Register 6 (0x04AA): background color.
    pub bg: u16,
    /// Register 7 (0x04AC): direction, destination and source bit address.
    pub sft: u16,
    /// Register 8 (0x04AE): bit length minus one.
    pub leng: u16,
    /// Pattern register per plane in memory byte order.
    pub pattern: [u16; 4],
    /// Pixels left in the current operation.
    pub remaining_bits: u16,
    /// Source pixels still to skip at the start of the operation.
    pub source_skip: u8,
    /// Destination pixels still to skip by the writes of the operation.
    pub destination_skip: u8,
    /// Destination pixels still to skip by the output units of the operation.
    pub output_skip: u8,
    /// Whether an input of the operation completed an output unit.
    pub source_ready: bool,
    /// Shifter output per plane in memory byte order.
    pub source_latch: [u16; 4],
    /// Stream position of the next pixel taken by an output unit.
    pub stream_read: u32,
    /// Stream position of the next pixel appended by an input.
    pub stream_write: u32,
    /// Shifter stream per plane, most significant bit first.
    pub stream: Box<[u8]>,
}}

impl Default for EgcState {
    fn default() -> Self {
        let mut state = Self {
            access: ACCESS_RESET,
            fgbg: FGBG_RESET,
            ope: 0,
            fg: 0,
            mask: MASK_RESET,
            bg: 0,
            sft: 0,
            leng: LENG_RESET,
            pattern: [0; PLANE_COUNT],
            remaining_bits: 0,
            source_skip: 0,
            destination_skip: 0,
            output_skip: 0,
            source_ready: false,
            source_latch: [0; PLANE_COUNT],
            stream_read: 0,
            stream_write: 0,
            stream: vec![0; PLANE_COUNT * STREAM_PLANE_BYTES].into_boxed_slice(),
        };
        state.restart_operation();
        state
    }
}

/// Shift of the source pixels within a byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ByteShift {
    /// Source and destination share the bit address.
    Aligned,
    /// The pixels shift right by the given distance.
    Right(u8),
    /// The pixels shift left by the given distance.
    Left(u8),
}

impl EgcState {
    /// Starts a new operation from the shift and length registers.
    fn restart_operation(&mut self) {
        self.remaining_bits = (self.leng & LENG_MASK) + 1;
        self.source_skip = (self.sft & SFT_SOURCE_MASK) as u8;
        self.destination_skip = ((self.sft & SFT_DESTINATION_MASK) >> SFT_DESTINATION_SHIFT) as u8;
        self.output_skip = self.destination_skip;
        self.source_ready = false;
        self.stream_read = 0;
        self.stream_write = 0;
    }

    /// Returns whether the shifter runs right to left.
    fn descending(&self) -> bool {
        self.sft & SFT_DESCENDING != 0
    }

    /// Returns whether the source bit address lies above the destination bit
    /// address.
    fn shifts_left(&self) -> bool {
        (self.sft & SFT_SOURCE_MASK) > ((self.sft & SFT_DESTINATION_MASK) >> SFT_DESTINATION_SHIFT)
    }

    /// Returns the number of stream pixels not yet taken.
    fn stream_length(&self) -> u32 {
        self.stream_write - self.stream_read
    }

    /// Returns the foreground color expanded to a word per plane.
    pub fn foreground_planes(&self) -> [u16; 4] {
        std::array::from_fn(|plane| expand_color(self.fg, plane))
    }

    /// Returns the background color expanded to a word per plane.
    pub fn background_planes(&self) -> [u16; 4] {
        std::array::from_fn(|plane| expand_color(self.bg, plane))
    }

    /// Returns the shifter class of register 7 from the low three bits of the
    /// source and destination bit addresses. Bit 0 is the descending
    /// direction. 2 marks a right shift and 4 a left shift.
    pub fn shift_function(&self) -> u8 {
        let class = match self.byte_shift() {
            ByteShift::Aligned => SHIFT_CLASS_ALIGNED,
            ByteShift::Right(_) => SHIFT_CLASS_RIGHT,
            ByteShift::Left(_) => SHIFT_CLASS_LEFT,
        };
        class | u8::from(self.descending())
    }

    /// Returns the left and right shift distances within a byte. They add up
    /// to 8 for a shift and are 0 without a shift.
    pub fn byte_shift_bits(&self) -> (u8, u8) {
        let byte_width = BYTE_WIDTH as u8;
        match self.byte_shift() {
            ByteShift::Aligned => (0, 0),
            ByteShift::Right(distance) => (byte_width - distance, distance),
            ByteShift::Left(distance) => (distance, byte_width - distance),
        }
    }

    /// Returns the shift within a byte from the source to the destination bit
    /// address.
    fn byte_shift(&self) -> ByteShift {
        let source = self.sft & SFT_SOURCE_MASK;
        let destination = (self.sft & SFT_DESTINATION_MASK) >> SFT_DESTINATION_SHIFT;
        let offset = |address: u16| (address & BYTE_BIT_ADDRESS_MASK) as u8;
        let (source, destination) = (offset(source), offset(destination));
        if destination > source {
            ByteShift::Right(destination - source)
        } else if source > destination {
            ByteShift::Left(source - destination)
        } else {
            ByteShift::Aligned
        }
    }

    /// Returns the destination pixels of the next word write in memory form.
    pub fn source_mask(&self) -> u16 {
        stream_to_memory(
            self.next_write_bits(WORD_WIDTH),
            WORD_WIDTH,
            self.descending(),
        )
    }

    /// Returns the destination pixels of the next write of `width` pixels in
    /// stream form.
    fn next_write_bits(&self, width: u32) -> u16 {
        let offset = u32::from(self.destination_skip).min(width);
        let count = (width - offset).min(u32::from(self.remaining_bits));
        leading_bits(count) >> offset
    }

    /// Returns the pattern input P of a plane as a memory word.
    fn pattern_input(&self, plane: usize) -> u16 {
        match self.fgbg & FGBG_PATTERN_SOURCE_MASK {
            FGBG_BACKGROUND => expand_color(self.bg, plane),
            FGBG_FOREGROUND => expand_color(self.fg, plane),
            FGBG_MIXED if plane < MIXED_FIRST_BACKGROUND_PLANE => expand_color(self.fg, plane),
            FGBG_MIXED => expand_color(self.bg, plane),
            _ => self.pattern[plane],
        }
    }

    /// Returns the stream slice of a plane.
    fn plane_stream(&self, plane: usize) -> &[u8] {
        &self.stream[plane * STREAM_PLANE_BYTES..(plane + 1) * STREAM_PLANE_BYTES]
    }

    /// Appends one input of `width` pixels to the stream of every plane and
    /// takes one output unit into the source latch. `inputs` holds the memory
    /// form of each plane. `byte_odd` selects the latch byte of a byte access.
    fn shift_input(&mut self, inputs: [u16; 4], width: u32, byte_odd: Option<bool>) {
        let skip = u32::from(self.source_skip).min(width);
        self.source_skip -= skip as u8;
        let count = width - skip;
        let descending = self.descending();
        if count > 0 && self.stream_write + count <= STREAM_CAPACITY_BITS {
            let position = self.stream_write as usize;
            for (plane, &input) in inputs.iter().enumerate() {
                let bits = memory_to_stream(input, width, descending) << skip;
                let plane_stream =
                    &mut self.stream[plane * STREAM_PLANE_BYTES..(plane + 1) * STREAM_PLANE_BYTES];
                store_stream_bits(plane_stream, position, bits, count);
            }
            self.stream_write += count;
        }

        let offset = u32::from(self.output_skip).min(width);
        let unit = width - offset;
        let complete = self.stream_length() >= unit;
        let taken = unit.min(self.stream_length());
        let position = self.stream_read as usize;
        for plane in 0..PLANE_COUNT {
            let bits = load_stream_bits(self.plane_stream(plane), position, taken) >> offset;
            let output = stream_to_memory(bits, width, descending);
            let latch = &mut self.source_latch[plane];
            *latch = match byte_odd {
                Some(odd) => replace_register_byte(*latch, odd, output as u8),
                None => output,
            };
        }
        if complete {
            self.output_skip -= offset as u8;
            self.source_ready = true;
            self.stream_read += unit;
            self.compact_stream();
        }
    }

    /// Shifts the unread stream pixels back to the start of the stream.
    fn compact_stream(&mut self) {
        if self.stream_read == self.stream_write {
            self.stream_read = 0;
            self.stream_write = 0;
            return;
        }
        if self.stream_read < STREAM_COMPACT_BITS {
            return;
        }
        let length = self.stream_length();
        for plane in 0..PLANE_COUNT {
            let plane_stream =
                &mut self.stream[plane * STREAM_PLANE_BYTES..(plane + 1) * STREAM_PLANE_BYTES];
            let mut moved = 0;
            while moved < length {
                let count = (length - moved).min(WORD_WIDTH);
                let bits =
                    load_stream_bits(plane_stream, (self.stream_read + moved) as usize, count);
                store_stream_bits(plane_stream, moved as usize, bits, count);
                moved += count;
            }
        }
        self.stream_read = 0;
        self.stream_write = length;
    }

    /// Advances the operation by one write of `width` pixels and returns the
    /// destination pixel mask in stream form. Returns `None` for the delayed
    /// first write of a left shift.
    fn advance_write(&mut self, width: u32) -> Option<u16> {
        if self.shifts_left() && !self.source_ready {
            self.source_ready = true;
            return None;
        }
        let bits = self.next_write_bits(width);
        let offset = u32::from(self.destination_skip).min(width);
        self.destination_skip -= offset as u8;
        self.remaining_bits -= bits.count_ones() as u16;
        if self.remaining_bits == 0 {
            self.restart_operation();
        }
        Some(bits)
    }
}

/// Returns the plane word of a 4-bit color: all ones when the plane bit is set.
fn expand_color(color: u16, plane: usize) -> u16 {
    if (color & COLOR_MASK) & (1 << plane) != 0 {
        0xFFFF
    } else {
        0x0000
    }
}

/// Returns a 16-bit value with the top `count` bits set.
fn leading_bits(count: u32) -> u16 {
    ((0xFFFF_u32 << (WORD_WIDTH - count)) & 0xFFFF) as u16
}

/// Converts a memory value of `width` pixels into stream order, first pixel in
/// bit 15. In memory the even byte holds the left pixels and bit 7 of a byte
/// is its left pixel.
fn memory_to_stream(value: u16, width: u32, descending: bool) -> u16 {
    let screen = if width == WORD_WIDTH {
        value.swap_bytes()
    } else {
        value << 8
    };
    if descending {
        if width == WORD_WIDTH {
            screen.reverse_bits()
        } else {
            u16::from((screen >> 8) as u8).reverse_bits()
        }
    } else {
        screen
    }
}

/// Converts a stream value of `width` pixels back into memory order.
fn stream_to_memory(bits: u16, width: u32, descending: bool) -> u16 {
    if width == WORD_WIDTH {
        let screen = if descending {
            bits.reverse_bits()
        } else {
            bits
        };
        screen.swap_bytes()
    } else {
        let byte = (bits >> 8) as u8;
        u16::from(if descending {
            byte.reverse_bits()
        } else {
            byte
        })
    }
}

/// Writes the top `count` bits of `bits` into a stream at bit `position`.
fn store_stream_bits(stream: &mut [u8], position: usize, bits: u16, count: u32) {
    let first_byte = position / 8;
    let bit_offset = position % 8;
    let window = (u32::from(bits) << 8) >> bit_offset;
    let window_mask = (u32::from(leading_bits(count)) << 8) >> bit_offset;
    for index in 0..3 {
        let shift = 16 - 8 * index;
        let byte_mask = (window_mask >> shift) as u8;
        let byte_bits = (window >> shift) as u8;
        let target = &mut stream[first_byte + index];
        *target = (*target & !byte_mask) | (byte_bits & byte_mask);
    }
}

/// Reads `count` bits from a stream at bit `position`, aligned to bit 15.
fn load_stream_bits(stream: &[u8], position: usize, count: u32) -> u16 {
    let first_byte = position / 8;
    let bit_offset = position % 8;
    let window = (u32::from(stream[first_byte]) << 16)
        | (u32::from(stream[first_byte + 1]) << 8)
        | u32::from(stream[first_byte + 2]);
    let aligned = ((window << bit_offset) >> 8) as u16;
    aligned & leading_bits(count)
}

/// Applies an 8-bit raster operation code to source, destination and pattern.
///
/// Bit `4 * S + 2 * D + P` of the code is the result for the input
/// combination (S, D, P).
fn raster_operation(code: u8, source: u16, destination: u16, pattern: u16) -> u16 {
    (0..8).fold(0u16, |result, index| {
        if code & (1 << index) == 0 {
            return result;
        }
        let source_term = if index & 4 != 0 { source } else { !source };
        let destination_term = if index & 2 != 0 {
            destination
        } else {
            !destination
        };
        let pattern_term = if index & 1 != 0 { pattern } else { !pattern };
        result | (source_term & destination_term & pattern_term)
    })
}

/// Returns the byte of a 16-bit register that belongs to a byte access.
fn register_byte(value: u16, odd: bool) -> u16 {
    if odd { value >> 8 } else { value & 0x00FF }
}

/// Replaces the byte of a 16-bit register that belongs to a byte access.
fn replace_register_byte(register: u16, odd: bool, byte: u8) -> u16 {
    if odd {
        (register & 0x00FF) | (u16::from(byte) << 8)
    } else {
        (register & 0xFF00) | u16::from(byte)
    }
}

/// EGC controller.
pub struct Egc {
    /// Embedded state for save/restore.
    pub state: EgcState,
}

impl_simple_state_accessors!(Egc, EgcState);

impl Default for Egc {
    fn default() -> Self {
        Self::new()
    }
}

impl Egc {
    /// Creates a new EGC in its reset state.
    pub fn new() -> Self {
        Self {
            state: EgcState::default(),
        }
    }

    /// Resets the EGC to its power-on state.
    pub fn reset(&mut self) {
        self.state = EgcState::default();
    }

    /// Writes one byte of an EGC register. `port` is the low nibble of the port
    /// address.
    pub fn write_register_byte(&mut self, port: u8, value: u8) {
        let register_port = port & !1;
        let odd = port & 1 != 0;
        let current = match register_port {
            0x00 => self.state.access,
            0x02 => self.state.fgbg,
            0x04 => self.state.ope,
            0x06 => self.state.fg,
            0x08 => self.state.mask,
            0x0A => self.state.bg,
            0x0C => self.state.sft,
            0x0E => self.state.leng,
            _ => return,
        };
        self.write_register_word(register_port, replace_register_byte(current, odd, value));
    }

    /// Writes an EGC register. `port` is the even low nibble of the port address.
    ///
    /// The mask register ignores writes while register 2 selects a color as
    /// the pattern source. A write to the shift or length register
    /// starts a new operation.
    pub fn write_register_word(&mut self, port: u8, value: u16) {
        match port {
            0x00 => self.state.access = value,
            0x02 => self.state.fgbg = value,
            0x04 => self.state.ope = value,
            0x06 => self.state.fg = value,
            0x08 => {
                if self.state.fgbg & FGBG_PATTERN_SOURCE_MASK == 0 {
                    self.state.mask = value;
                }
            }
            0x0A => self.state.bg = value,
            0x0C => {
                self.state.sft = value;
                self.state.restart_operation();
            }
            0x0E => {
                self.state.leng = value;
                self.state.restart_operation();
            }
            _ => {}
        }
    }

    /// Returns whether the shifter runs right to left (register 7 bit 12).
    pub fn is_descending(&self) -> bool {
        self.state.descending()
    }

    /// Returns whether register 1 enables writes to `plane`.
    pub fn plane_write_enabled(&self, plane: usize) -> bool {
        self.state.access & (ACCESS_PLANE_DISABLE << plane) == 0
    }

    /// Reads one byte through the EGC. `vram` holds the byte of each plane at
    /// the accessed address.
    pub fn read_byte(&mut self, address: u32, vram: [u8; 4]) -> u8 {
        let odd = address & 1 != 0;
        let planes = vram.map(u16::from);
        if self.state.ope & OPE_PATTERN_LOAD_MASK == OPE_PATTERN_LOAD_ON_READ {
            for (pattern, &byte) in self.state.pattern.iter_mut().zip(&vram) {
                *pattern = replace_register_byte(*pattern, odd, byte);
            }
        }
        let patterns = self.patterns_for_access(Some(odd));
        self.read_unit(planes, patterns, BYTE_WIDTH, Some(odd)) as u8
    }

    /// Reads one aligned word through the EGC. `vram` holds the word of each
    /// plane at the accessed address.
    pub fn read_word(&mut self, vram: [u16; 4]) -> u16 {
        if self.state.ope & OPE_PATTERN_LOAD_MASK == OPE_PATTERN_LOAD_ON_READ {
            self.state.pattern = vram;
        }
        let patterns = self.patterns_for_access(None);
        self.read_unit(vram, patterns, WORD_WIDTH, None)
    }

    /// Writes one byte through the EGC. `vram` holds the byte of each plane at
    /// the accessed address. Returns the new data of each plane and the bits
    /// to replace.
    pub fn write_byte(&mut self, address: u32, value: u8, vram: [u8; 4]) -> ([u8; 4], u8) {
        let odd = address & 1 != 0;
        if self.state.ope & OPE_PATTERN_LOAD_MASK == OPE_PATTERN_LOAD_ON_WRITE {
            for (pattern, &byte) in self.state.pattern.iter_mut().zip(&vram) {
                *pattern = replace_register_byte(*pattern, odd, byte);
            }
        }
        let patterns = self.patterns_for_access(Some(odd));
        let mask = register_byte(self.state.mask, odd);
        let (data, write_mask) = self.write_unit(
            u16::from(value),
            vram.map(u16::from),
            patterns,
            mask,
            BYTE_WIDTH,
            Some(odd),
        );
        (data.map(|plane| plane as u8), write_mask as u8)
    }

    /// Writes one aligned word through the EGC. `vram` holds the word of each
    /// plane at the accessed address. Returns the new data of each plane and
    /// the bits to replace.
    pub fn write_word(&mut self, value: u16, vram: [u16; 4]) -> ([u16; 4], u16) {
        if self.state.ope & OPE_PATTERN_LOAD_MASK == OPE_PATTERN_LOAD_ON_WRITE {
            self.state.pattern = vram;
        }
        let patterns = self.patterns_for_access(None);
        self.write_unit(value, vram, patterns, self.state.mask, WORD_WIDTH, None)
    }

    /// Returns the pattern input of every plane for a word access (`None`) or
    /// for the byte of a byte access.
    fn patterns_for_access(&self, byte_odd: Option<bool>) -> [u16; 4] {
        std::array::from_fn(|plane| {
            let pattern = self.state.pattern_input(plane);
            match byte_odd {
                Some(odd) => register_byte(pattern, odd),
                None => pattern,
            }
        })
    }

    /// Performs a read of `width` pixels and returns the CPU data.
    ///
    /// A compare read returns 1 for each pixel whose four planes equal the
    /// pattern input. Otherwise the read plane comes from the source latch when
    /// VRAM feeds the shifter and from VRAM when the CPU feeds it.
    fn read_unit(
        &mut self,
        vram: [u16; 4],
        patterns: [u16; 4],
        width: u32,
        byte_odd: Option<bool>,
    ) -> u16 {
        let source_is_vram = self.state.ope & OPE_SOURCE_CPU == 0;
        if source_is_vram {
            self.state.shift_input(vram, width, byte_odd);
        }

        if self.state.ope & OPE_COMPARE_READ != 0 {
            return vram
                .iter()
                .zip(&patterns)
                .fold(0xFFFF, |result, (&plane, &pattern)| {
                    result & !(plane ^ pattern)
                });
        }

        let plane = usize::from((self.state.fgbg & FGBG_READ_PLANE_MASK) >> FGBG_READ_PLANE_SHIFT);
        if source_is_vram {
            latch_unit(self.state.source_latch[plane], byte_odd)
        } else {
            vram[plane]
        }
    }

    /// Performs a write of `width` pixels. Returns the data of each plane and
    /// the bits to replace, both in memory form.
    fn write_unit(
        &mut self,
        value: u16,
        vram: [u16; 4],
        patterns: [u16; 4],
        mask: u16,
        width: u32,
        byte_odd: Option<bool>,
    ) -> ([u16; 4], u16) {
        if self.state.ope & OPE_SOURCE_CPU != 0 {
            self.state
                .shift_input([value; PLANE_COUNT], width, byte_odd);
        }

        let Some(stream_mask) = self.state.advance_write(width) else {
            return (vram, 0);
        };

        let pixel_mask = stream_to_memory(stream_mask, width, self.state.descending()) & mask;
        let data = match self.state.ope & OPE_WRITE_SOURCE_MASK {
            OPE_WRITE_CPU => [value; PLANE_COUNT],
            OPE_WRITE_ROP => {
                let code = (self.state.ope & OPE_ROP_MASK) as u8;
                std::array::from_fn(|plane| {
                    let source = latch_unit(self.state.source_latch[plane], byte_odd);
                    raster_operation(code, source, vram[plane], patterns[plane])
                })
            }
            OPE_WRITE_PATTERN => patterns,
            _ => return (vram, 0),
        };
        (data, pixel_mask)
    }
}

/// Returns the part of a source latch that belongs to a word access (`None`)
/// or to the byte of a byte access.
fn latch_unit(latch: u16, byte_odd: Option<bool>) -> u16 {
    match byte_odd {
        Some(odd) => register_byte(latch, odd),
        None => latch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_defaults() {
        let egc = Egc::new();
        assert_eq!(egc.state.access, 0xFFF0);
        assert_eq!(egc.state.fgbg, 0x00FF);
        assert_eq!(egc.state.ope, 0);
        assert_eq!(egc.state.fg, 0);
        assert_eq!(egc.state.mask, 0xFFFF);
        assert_eq!(egc.state.bg, 0);
        assert_eq!(egc.state.sft, 0);
        assert_eq!(egc.state.leng, 0x000F);
        assert_eq!(egc.state.remaining_bits, 16);
        assert_eq!(egc.state.source_mask(), 0xFFFF);
        assert_eq!(egc.state.foreground_planes(), [0; 4]);
        assert_eq!(egc.state.background_planes(), [0; 4]);
        assert_eq!(egc.state.shift_function(), 0);
    }

    #[test]
    fn reset_restores_defaults() {
        let mut egc = Egc::new();
        egc.write_register_word(0x00, 0x1234);
        egc.write_register_word(0x04, 0x5678);
        egc.write_register_word(0x06, 0x0005);
        egc.write_register_word(0x08, 0xAAAA);

        egc.reset();

        assert_eq!(egc.state.access, 0xFFF0);
        assert_eq!(egc.state.ope, 0);
        assert_eq!(egc.state.fg, 0);
        assert_eq!(egc.state.mask, 0xFFFF);
        assert_eq!(egc.state.foreground_planes(), [0; 4]);
        assert_eq!(egc.state.source_mask(), 0xFFFF);
    }

    #[test]
    fn write_register_word() {
        let mut egc = Egc::new();
        egc.write_register_word(0x00, 0x1234);
        assert_eq!(egc.state.access, 0x1234);
    }

    #[test]
    fn write_register_byte() {
        let mut egc = Egc::new();
        egc.write_register_byte(0x00, 0x34);
        assert_eq!(egc.state.access & 0x00FF, 0x0034);

        egc.write_register_byte(0x01, 0x12);
        assert_eq!(egc.state.access, 0x1234);
    }

    #[test]
    fn fg_color_expansion() {
        let mut egc = Egc::new();
        egc.write_register_word(0x06, 5);
        assert_eq!(
            egc.state.foreground_planes(),
            [0xFFFF, 0x0000, 0xFFFF, 0x0000]
        );
    }

    #[test]
    fn bg_color_expansion() {
        let mut egc = Egc::new();
        egc.write_register_word(0x0A, 0x0A);
        assert_eq!(
            egc.state.background_planes(),
            [0x0000, 0xFFFF, 0x0000, 0xFFFF]
        );
    }

    #[test]
    fn fg_color_expansion_byte_write() {
        let mut egc = Egc::new();
        egc.write_register_byte(0x06, 5);
        assert_eq!(
            egc.state.foreground_planes(),
            [0xFFFF, 0x0000, 0xFFFF, 0x0000]
        );
    }

    #[test]
    fn mask_register_gating_allowed() {
        let mut egc = Egc::new();
        egc.write_register_word(0x08, 0xAAAA);
        assert_eq!(egc.state.mask, 0xAAAA);
    }

    #[test]
    fn mask_register_gating_blocked() {
        let mut egc = Egc::new();

        egc.write_register_word(0x02, 0x2000);
        egc.write_register_word(0x08, 0xAAAA);
        assert_eq!(egc.state.mask, 0xFFFF);

        egc.write_register_word(0x02, 0x4000);
        egc.write_register_word(0x08, 0x5555);
        assert_eq!(egc.state.mask, 0xFFFF);

        egc.write_register_word(0x02, 0x6000);
        egc.write_register_word(0x08, 0x1234);
        assert_eq!(egc.state.mask, 0xFFFF);
    }

    #[test]
    fn plane_write_enabled() {
        let mut egc = Egc::new();

        assert!(egc.plane_write_enabled(0));
        assert!(egc.plane_write_enabled(1));
        assert!(egc.plane_write_enabled(2));
        assert!(egc.plane_write_enabled(3));

        egc.write_register_word(0x00, 0xFFF5);
        assert!(!egc.plane_write_enabled(0));
        assert!(egc.plane_write_enabled(1));
        assert!(!egc.plane_write_enabled(2));
        assert!(egc.plane_write_enabled(3));

        egc.write_register_word(0x00, 0xFFFF);
        assert!(!egc.plane_write_enabled(0));
        assert!(!egc.plane_write_enabled(1));
        assert!(!egc.plane_write_enabled(2));
        assert!(!egc.plane_write_enabled(3));
    }

    #[test]
    fn shift_direction() {
        let mut egc = Egc::new();

        egc.write_register_word(0x0C, 0x0000);
        assert!(!egc.is_descending());

        egc.write_register_word(0x0C, 0x1000);
        assert!(egc.is_descending());
    }

    #[test]
    fn sft_write_resets_srcmask() {
        let mut egc = Egc::new();
        egc.state.destination_skip = 15;
        egc.write_register_word(0x0C, 0x0000);
        assert_eq!(egc.state.source_mask(), 0xFFFF);
    }

    #[test]
    fn leng_write_sets_remain() {
        let mut egc = Egc::new();
        egc.write_register_word(0x0E, 0x001F);
        assert_eq!(egc.state.remaining_bits, 32);
    }

    #[test]
    fn recalculate_shift_func_selection() {
        let mut egc = Egc::new();

        // Ascending, no shift (src8 == dst8): func=0
        egc.write_register_word(0x0C, 0x0033);
        assert_eq!(egc.state.shift_function(), 0);

        // Descending, no shift: func=1
        egc.write_register_word(0x0C, 0x1033);
        assert_eq!(egc.state.shift_function(), 1);

        // Ascending, src8(2) < dst8(5): func=2 (right shift)
        egc.write_register_word(0x0C, 0x0052);
        assert_eq!(egc.state.shift_function(), 2);
        assert_eq!(egc.state.byte_shift_bits().1, 3);
        assert_eq!(egc.state.byte_shift_bits().0, 5);

        // Ascending, src8(5) > dst8(2): func=4 (left shift)
        egc.write_register_word(0x0C, 0x0025);
        assert_eq!(egc.state.shift_function(), 4);
        assert_eq!(egc.state.byte_shift_bits().0, 3);
        assert_eq!(egc.state.byte_shift_bits().1, 5);

        // Descending, src8(2) < dst8(5): func=3
        egc.write_register_word(0x0C, 0x1052);
        assert_eq!(egc.state.shift_function(), 3);

        // Descending, src8(5) > dst8(2): func=5
        egc.write_register_word(0x0C, 0x1025);
        assert_eq!(egc.state.shift_function(), 5);
    }

    #[test]
    fn cpu_broadcast_write_word() {
        let mut egc = Egc::new();
        egc.write_register_word(0x04, 0x0000); // ope=0 -> CPU broadcast
        let vram = [0u16; 4];
        let (data, mask) = egc.write_word(0xBEEF, vram);
        assert_eq!(data, [0xBEEF, 0xBEEF, 0xBEEF, 0xBEEF]);
        assert_eq!(mask, 0xFFFF);
    }

    #[test]
    fn fgc_color_fill_write_word() {
        let mut egc = Egc::new();
        egc.write_register_word(0x06, 5); // fg=5
        egc.write_register_word(0x02, 0x4000); // fgbg: FGC source
        egc.write_register_word(0x04, 0x1000); // ope: pattern source
        let vram = [0u16; 4];
        let (data, _mask) = egc.write_word(0x0000, vram);
        assert_eq!(data, [0xFFFF, 0x0000, 0xFFFF, 0x0000]);
    }

    #[test]
    fn bgc_color_fill_write_word() {
        let mut egc = Egc::new();
        egc.write_register_word(0x0A, 0x0A); // bg=0xA
        egc.write_register_word(0x02, 0x2000); // fgbg: BGC source
        egc.write_register_word(0x04, 0x1000); // ope: pattern source
        let vram = [0u16; 4];
        let (data, _mask) = egc.write_word(0x0000, vram);
        assert_eq!(data, [0x0000, 0xFFFF, 0x0000, 0xFFFF]);
    }

    #[test]
    fn rop_clear_all() {
        let mut egc = Egc::new();
        // ope: shift+ROP, CPU source (bit10=1), ROP=0x00
        egc.write_register_word(0x04, 0x0C00);
        let vram = [0x5555u16; 4];
        let (data, _mask) = egc.write_word(0xAAAA, vram);
        assert_eq!(data, [0; 4]);
    }

    #[test]
    fn rop_fill_all() {
        let mut egc = Egc::new();
        // ope: shift+ROP, CPU source, ROP=0xFF
        egc.write_register_word(0x04, 0x0CFF);
        let vram = [0u16; 4];
        let (data, _mask) = egc.write_word(0x0000, vram);
        assert_eq!(data, [0xFFFF; 4]);
    }

    #[test]
    fn rop_source_copy() {
        let mut egc = Egc::new();
        // ope: shift+ROP, CPU source (bit10=1), ROP=0xF0 (Source)
        egc.write_register_word(0x04, 0x0CF0);
        let vram = [0x5555u16; 4];
        let (data, _mask) = egc.write_word(0xAAAA, vram);
        assert_eq!(data, [0xAAAA, 0xAAAA, 0xAAAA, 0xAAAA]);
    }

    #[test]
    fn rop_invert_source() {
        let mut egc = Egc::new();
        // ope: shift+ROP, CPU source, ROP=0x0F (~Source)
        egc.write_register_word(0x04, 0x0C0F);
        let vram = [0u16; 4];
        let (data, _mask) = egc.write_word(0xAAAA, vram);
        assert_eq!(data, [0x5555, 0x5555, 0x5555, 0x5555]);
    }

    #[test]
    fn rop_source_and_dest() {
        let mut egc = Egc::new();
        // ope: shift+ROP, CPU source, ROP=0xC0 (S & D)
        egc.write_register_word(0x04, 0x0CC0);
        let vram = [0xFF00, 0x00FF, 0xF0F0, 0x0F0F];
        let (data, _mask) = egc.write_word(0xAAAA, vram);
        assert_eq!(data, [0xAA00, 0x00AA, 0xA0A0, 0x0A0A]);
    }

    #[test]
    fn write_byte_cpu_broadcast() {
        let mut egc = Egc::new();
        egc.write_register_word(0x04, 0x0000); // ope=0 -> CPU broadcast
        let vram = [0u8; 4];
        let (data, mask) = egc.write_byte(0xA8000, 0xAB, vram);
        assert_eq!(data, [0xAB, 0xAB, 0xAB, 0xAB]);
        assert_eq!(mask, 0xFF);
    }

    #[test]
    fn sftb_dispatch_no_panic_when_dstbit_exceeds_8() {
        let mut egc = Egc::new();
        // Set up a valid state with dstbit > 8 (range is 0-15).
        egc.state.destination_skip = 12;
        egc.state.output_skip = 12;
        egc.state.stream_write = 0;
        // A byte input must return without panicking.
        egc.state.shift_input([0; 4], BYTE_WIDTH, Some(false));
    }

    #[test]
    fn compare_read_byte_matches_foreground_color() {
        let mut egc = Egc::new();
        // Set foreground color to 5 (planes 0 and 2 set).
        egc.write_register_word(0x06, 5);
        // Select FGC as compare source (fgbg bits 14-13 = 2).
        egc.write_register_word(0x02, 0x4000);
        // Enable compare-read mode (bit 13 of ope).
        egc.write_register_word(0x04, 0x2000);

        // VRAM where pixel matches fg color 5: B=0xFF, R=0x00, G=0xFF, E=0x00
        let vram_match: [u8; 4] = [0xFF, 0x00, 0xFF, 0x00];
        let result = egc.read_byte(0xA8000, vram_match);
        assert_eq!(result, 0xFF, "all pixels match fg color 5");

        // VRAM where no pixel matches: B=0x00, R=0xFF, G=0x00, E=0xFF
        let vram_no_match: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF];
        let result = egc.read_byte(0xA8000, vram_no_match);
        assert_eq!(result, 0x00, "no pixels match fg color 5");

        // Partial match: only some bits match across all planes.
        // fg=5 -> fgc=[0xFF,0x00,0xFF,0x00]
        // B=0xF0, R=0x00, G=0xFF, E=0x00
        // XOR: B=0x0F, R=0x00, G=0x00, E=0x00
        // NOT XOR: B=0xF0, R=0xFF, G=0xFF, E=0xFF
        // AND all: 0xF0
        let vram_partial: [u8; 4] = [0xF0, 0x00, 0xFF, 0x00];
        let result = egc.read_byte(0xA8000, vram_partial);
        assert_eq!(result, 0xF0, "upper nibble matches fg color");
    }

    #[test]
    fn compare_read_word_matches_foreground_color() {
        let mut egc = Egc::new();
        // Set foreground color to 3 (planes 0 and 1 set).
        egc.write_register_word(0x06, 3);
        // Select FGC as compare source (fgbg bits 14-13 = 2).
        egc.write_register_word(0x02, 0x4000);
        // Enable compare-read mode (bit 13 of ope).
        egc.write_register_word(0x04, 0x2000);

        // VRAM matching color 3: B=0xFFFF, R=0xFFFF, G=0x0000, E=0x0000
        let vram_match: [u16; 4] = [0xFFFF, 0xFFFF, 0x0000, 0x0000];
        let result = egc.read_word(vram_match);
        assert_eq!(result, 0xFFFF, "all pixels match fg color 3");

        // No match.
        let vram_no_match: [u16; 4] = [0x0000, 0x0000, 0xFFFF, 0xFFFF];
        let result = egc.read_word(vram_no_match);
        assert_eq!(result, 0x0000, "no pixels match fg color 3");
    }
}
