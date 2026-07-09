//! MB61VH010 - FM-77AV graphics ALU.
//!
//! The ALU sits between the sub CPU and VRAM. While enabled it intercepts every
//! sub CPU VRAM access and replaces the plain store with a per-plane logical
//! operation (set, blank, OR, AND, XOR, NOT, tile paint, compare) gated by a
//! write mask, a per-plane bank-disable, and an optional sub-byte compare unit.
//! It also provides a hardware line generator that rasterizes a Bresenham line
//! through the same operation pipeline.

/// First low-byte offset of the ALU register block (`0xD410`).
pub const PORT_ALU_FIRST: u8 = 0x10;
/// Last low-byte offset of the ALU register block (`0xD42B`).
pub const PORT_ALU_LAST: u8 = 0x2B;

/// `0xD410` command register (read/write).
const PORT_COMMAND: u8 = 0x10;
/// `0xD411` logical colour register, one bit per plane (read/write).
const PORT_LOGICAL_COLOR: u8 = 0x11;
/// `0xD412` write mask, one bit per pixel within the addressed byte (read/write).
const PORT_WRITE_MASK: u8 = 0x12;
/// `0xD413` compare status (read) / first compare colour bank (write).
const PORT_COMPARE_FIRST: u8 = 0x13;
/// `0xD41A` last compare colour bank (write).
const PORT_COMPARE_LAST: u8 = 0x1A;
/// `0xD41B` bank-disable register, one bit per plane (read/write).
const PORT_BANK_DISABLE: u8 = 0x1B;
/// `0xD41C` tile-paint pattern for the blue plane (write).
const PORT_TILE_BLUE: u8 = 0x1C;
/// `0xD41D` tile-paint pattern for the red plane (write).
const PORT_TILE_RED: u8 = 0x1D;
/// `0xD41E` tile-paint pattern for the green plane (write).
const PORT_TILE_GREEN: u8 = 0x1E;
/// `0xD41F` tile-paint pattern for the luminance plane (write, unused on base AV).
const PORT_TILE_LUMINANCE: u8 = 0x1F;
/// `0xD420` line address offset, high byte (write).
const PORT_LINE_OFFSET_HIGH: u8 = 0x20;
/// `0xD421` line address offset, low byte (write).
const PORT_LINE_OFFSET_LOW: u8 = 0x21;
/// `0xD422` line pattern, high byte (write).
const PORT_LINE_PATTERN_HIGH: u8 = 0x22;
/// `0xD423` line pattern, low byte (write).
const PORT_LINE_PATTERN_LOW: u8 = 0x23;
/// `0xD424` line start X, high bits (write).
const PORT_LINE_X_BEGIN_HIGH: u8 = 0x24;
/// `0xD425` line start X, low byte (write).
const PORT_LINE_X_BEGIN_LOW: u8 = 0x25;
/// `0xD426` line start Y, high bit (write).
const PORT_LINE_Y_BEGIN_HIGH: u8 = 0x26;
/// `0xD427` line start Y, low byte (write).
const PORT_LINE_Y_BEGIN_LOW: u8 = 0x27;
/// `0xD428` line end X, high bits (write).
const PORT_LINE_X_END_HIGH: u8 = 0x28;
/// `0xD429` line end X, low byte (write).
const PORT_LINE_X_END_LOW: u8 = 0x29;
/// `0xD42A` line end Y, high bit (write).
const PORT_LINE_Y_END_HIGH: u8 = 0x2A;
/// `0xD42B` line end Y, low byte; writing it triggers the hardware line draw.
const PORT_LINE_Y_END_LOW: u8 = 0x2B;

/// Command bit 7: the ALU is active and intercepts VRAM accesses.
const COMMAND_ENABLE: u8 = 0x80;
/// Command bit 6: read-modify-write through the compare unit before storing.
const COMMAND_CALC_BEFORE_WRITE: u8 = 0x40;
/// Command bit 5: selects which side of the compare mask the status protects.
const COMMAND_MASK_SELECT: u8 = 0x20;
/// Command bits 2-0: the logical operation selector.
const COMMAND_OP_MASK: u8 = 0x07;

/// Operation `0`: set each plane to the logical colour.
const OP_PSET: u8 = 0;
/// Operation `1`: blank the unmasked pixels.
const OP_BLANK: u8 = 1;
/// Operation `2`: OR the logical colour into the planes.
const OP_OR: u8 = 2;
/// Operation `3`: AND the logical colour into the planes.
const OP_AND: u8 = 3;
/// Operation `4`: XOR the logical colour into the planes.
const OP_XOR: u8 = 4;
/// Operation `5`: invert the planes.
const OP_NOT: u8 = 5;
/// Operation `6`: stamp the per-plane tile pattern.
const OP_TILEPAINT: u8 = 6;

/// Number of fitted VRAM planes on the base FM-77AV (blue, red, green).
const PLANE_COUNT: u8 = 3;
/// In-plane byte-address mask (16 KiB per plane in the 200-line modes).
const IN_PLANE_ADDRESS_MASK: u16 = 0x3FFF;
/// Mask selecting the three plane bits of a logical colour.
const PLANE_COLOR_MASK: u8 = 0x07;

/// Bank-disable bits forced high for a three-plane machine (planes 3-7 absent).
const BANK_DISABLE_FORCED: u8 = 0xF8;

/// Compare-bank bit 7: the bank is unused and excluded from the match.
const COMPARE_BANK_DISABLED: u8 = 0x80;

/// Value returned by ALU registers that are write-only or unmapped.
const OPEN_BUS: u8 = 0xFF;

/// Sentinel meaning "no byte accumulated yet" during a line draw.
const LINE_ADDRESS_NONE: u32 = 0xFFFF_FFFF;
/// Highest line coordinate the generator walks before it gives up (9-bit Y).
const LINE_COORD_LIMIT: i32 = 512;
/// Fixed-point half step used by the line DDA error term.
const LINE_DDA_HALF: i32 = 16384;
/// Fixed-point full step used by the line DDA error term.
const LINE_DDA_FULL: i32 = 32768;
/// Bytes the ALU processes per microsecond, setting the busy duration.
const LINE_BYTES_PER_MICROSECOND: u32 = 16;

/// Per-pixel bit within a byte, most significant pixel first.
const PIXEL_BIT: [u8; 8] = [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01];

/// VRAM access the ALU performs against the host's planes. The host resolves the
/// in-plane byte offset and plane index to physical storage and enforces the
/// per-plane access mask (a blocked plane reads `0xFF` and drops writes).
pub trait AluMemory {
    /// Reads the byte of `plane` at the in-plane byte `offset`.
    fn read_plane(&self, offset: u16, plane: u8) -> u8;
    /// Writes the byte of `plane` at the in-plane byte `offset`.
    fn write_plane(&mut self, offset: u16, plane: u8, value: u8);
    /// Visible pixels per scanline for the current display mode.
    fn pixel_width(&self) -> u32;
}

/// The three logical operations that combine the logical colour with VRAM.
enum LogicalOp {
    Or,
    And,
    Xor,
}

/// FM-77AV MB61VH010 graphics ALU.
#[derive(Debug, Clone)]
pub struct Mb61vh010Alu {
    /// Command register (`0xD410`): operation, mask mode, calc-before-write, enable.
    command: u8,
    /// Logical colour (`0xD411`): one bit per plane supplying the drawn colour.
    logical_color: u8,
    /// Write mask (`0xD412`): a set bit preserves the existing pixel.
    write_mask: u8,
    /// Compare status (`0xD413` read): one bit per matched pixel in the byte.
    compare_status: u8,
    /// Compare colour banks (`0xD413-0xD41A`): bit 7 disables the bank.
    compare_colors: [u8; 8],
    /// Bank-disable register (`0xD41B`): a set bit excludes that plane.
    bank_disable: u8,
    /// Cached per-plane disable flags derived from `bank_disable`.
    plane_disabled: [bool; 4],
    /// Tile-paint patterns (`0xD41C-0xD41F`) for the blue/red/green/luminance planes.
    tile: [u8; 4],
    /// Line address offset (`0xD420`/`0xD421`) added to every line byte address.
    line_address_offset: u16,
    /// Line pattern (`0xD422`/`0xD423`) selecting which line pixels are drawn.
    line_pattern: u16,
    /// Line start X coordinate (`0xD424`/`0xD425`).
    line_x_begin: u16,
    /// Line start Y coordinate (`0xD426`/`0xD427`).
    line_y_begin: u16,
    /// Line end X coordinate (`0xD428`/`0xD429`).
    line_x_end: u16,
    /// Line end Y coordinate (`0xD42A`/`0xD42B`).
    line_y_end: u16,
    /// Whether a hardware line draw is still occupying the ALU.
    busy: bool,
    /// Byte address currently being accumulated by the line generator.
    line_current_address: u32,
    /// Most recent byte address the line generator computed.
    line_last_address: u32,
    /// Rolling copy of `line_pattern` consumed as the line advances.
    line_pattern_shift: u16,
}

impl Default for Mb61vh010Alu {
    fn default() -> Self {
        Self::new()
    }
}

impl Mb61vh010Alu {
    /// Creates the ALU in its power-on state: disabled, three planes fitted, all
    /// compare banks off, and a fully transparent write mask.
    pub fn new() -> Self {
        let mut alu = Self {
            command: 0,
            logical_color: 0,
            write_mask: 0,
            compare_status: 0,
            compare_colors: [COMPARE_BANK_DISABLED; 8],
            bank_disable: BANK_DISABLE_FORCED,
            plane_disabled: [false; 4],
            tile: [0; 4],
            line_address_offset: 0,
            line_pattern: 0,
            line_x_begin: 0,
            line_y_begin: 0,
            line_x_end: 0,
            line_y_end: 0,
            busy: false,
            line_current_address: LINE_ADDRESS_NONE,
            line_last_address: LINE_ADDRESS_NONE,
            line_pattern_shift: 0,
        };
        alu.refresh_plane_disable();
        alu
    }

    /// Whether the ALU is enabled and intercepting VRAM accesses.
    pub fn is_enabled(&self) -> bool {
        self.command & COMMAND_ENABLE != 0
    }

    /// Whether the ALU is busy with a hardware line draw.
    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// Clears the busy flag when the hardware line draw completes.
    pub fn clear_busy(&mut self) {
        self.busy = false;
    }

    /// Reads an ALU register by its low-byte port offset. Only the command,
    /// colour, mask, compare-status and bank-disable registers read back; the
    /// write-only registers return open bus.
    pub fn read_register(&self, port: u8) -> u8 {
        match port {
            PORT_COMMAND => self.command,
            PORT_LOGICAL_COLOR => self.logical_color,
            PORT_WRITE_MASK => self.write_mask,
            PORT_COMPARE_FIRST => self.compare_status,
            PORT_BANK_DISABLE => self.bank_disable,
            _ => OPEN_BUS,
        }
    }

    /// Writes an ALU register by its low-byte port offset. Writing the line
    /// end-Y register runs the hardware line draw and returns the busy duration
    /// in microseconds (zero when the line is too short to model a busy period);
    /// every other write returns zero.
    pub fn write_register<M: AluMemory>(&mut self, memory: &mut M, port: u8, value: u8) -> u64 {
        match port {
            PORT_COMMAND => self.command = value,
            PORT_LOGICAL_COLOR => self.logical_color = value,
            PORT_WRITE_MASK => self.write_mask = value,
            PORT_COMPARE_FIRST..=PORT_COMPARE_LAST => {
                self.compare_colors[usize::from(port - PORT_COMPARE_FIRST)] = value;
            }
            PORT_BANK_DISABLE => {
                self.bank_disable = value | BANK_DISABLE_FORCED;
                self.refresh_plane_disable();
            }
            PORT_TILE_BLUE => self.tile[0] = value,
            PORT_TILE_RED => self.tile[1] = value,
            PORT_TILE_GREEN => self.tile[2] = value,
            PORT_TILE_LUMINANCE => self.tile[3] = value,
            PORT_LINE_OFFSET_HIGH => self.write_offset_high(value),
            PORT_LINE_OFFSET_LOW => self.write_offset_low(value),
            PORT_LINE_PATTERN_HIGH => {
                self.line_pattern = (self.line_pattern & 0x00FF) | (u16::from(value) << 8);
            }
            PORT_LINE_PATTERN_LOW => {
                self.line_pattern = (self.line_pattern & 0xFF00) | u16::from(value);
            }
            PORT_LINE_X_BEGIN_HIGH => {
                self.line_x_begin = (self.line_x_begin & 0x00FF) | (u16::from(value & 0x03) << 8);
            }
            PORT_LINE_X_BEGIN_LOW => {
                self.line_x_begin = (self.line_x_begin & 0xFF00) | u16::from(value);
            }
            PORT_LINE_Y_BEGIN_HIGH => {
                self.line_y_begin = (self.line_y_begin & 0x00FF) | (u16::from(value & 0x01) << 8);
            }
            PORT_LINE_Y_BEGIN_LOW => {
                self.line_y_begin = (self.line_y_begin & 0xFF00) | u16::from(value);
            }
            PORT_LINE_X_END_HIGH => {
                self.line_x_end = (self.line_x_end & 0x00FF) | (u16::from(value & 0x03) << 8);
            }
            PORT_LINE_X_END_LOW => {
                self.line_x_end = (self.line_x_end & 0xFF00) | u16::from(value);
            }
            PORT_LINE_Y_END_HIGH => {
                self.line_y_end = (self.line_y_end & 0x00FF) | (u16::from(value & 0x01) << 8);
            }
            PORT_LINE_Y_END_LOW => {
                self.line_y_end = (self.line_y_end & 0xFF00) | u16::from(value);
                return self.draw_line(memory);
            }
            _ => {}
        }
        0
    }

    /// Runs the ALU operation for the VRAM byte at `address`. The host calls this
    /// for both reads and writes while the ALU is enabled; the CPU data value is
    /// ignored because the operation is driven entirely by the ALU registers.
    pub fn access_vram<M: AluMemory>(&mut self, memory: &mut M, address: u16) {
        self.apply_operation(memory, address & IN_PLANE_ADDRESS_MASK);
    }

    /// Recomputes the cached per-plane disable flags from `bank_disable`.
    fn refresh_plane_disable(&mut self) {
        for (plane, disabled) in self.plane_disabled.iter_mut().enumerate() {
            *disabled = self.bank_disable & (1 << plane) != 0;
        }
    }

    /// Mask of the planes currently enabled for output (blue/red/green).
    fn enabled_planes(&self) -> u8 {
        !self.bank_disable & PLANE_COLOR_MASK
    }

    /// Writes the high byte of the line address offset. The value is doubled and
    /// packed so the offset counts in whole VRAM bytes.
    fn write_offset_high(&mut self, value: u8) {
        let high = (self.line_address_offset >> 8) as u8;
        let packed = (high & 0x01) | ((value << 1) & 0x3E);
        self.line_address_offset = (u16::from(packed) << 8) | (self.line_address_offset & 0x00FF);
    }

    /// Writes the low byte of the line address offset, carrying its top bit into
    /// the high byte to keep the doubled offset contiguous.
    fn write_offset_low(&mut self, value: u8) {
        let low = value << 1;
        let high = (self.line_address_offset >> 8) as u8;
        let packed = (high & 0xFE) | ((value >> 7) & 0x01);
        self.line_address_offset = (u16::from(packed) << 8) | u16::from(low);
    }

    /// Applies the selected operation to one byte position across the planes,
    /// running the compare unit first when calc-before-write is active.
    fn apply_operation<M: AluMemory>(&mut self, memory: &mut M, offset: u16) {
        if self.command & COMMAND_CALC_BEFORE_WRITE != 0 {
            self.update_compare_status(memory, offset);
        }
        match self.command & COMMAND_OP_MASK {
            OP_PSET => self.op_pset(memory, offset),
            OP_BLANK => self.op_blank(memory, offset),
            OP_OR => self.op_logical(memory, offset, LogicalOp::Or),
            OP_AND => self.op_logical(memory, offset, LogicalOp::And),
            OP_XOR => self.op_logical(memory, offset, LogicalOp::Xor),
            OP_NOT => self.op_not(memory, offset),
            OP_TILEPAINT => self.op_tilepaint(memory, offset),
            // The operation selector masks to three bits, so the only remaining
            // value is the compare operation, which just refreshes the status.
            _ => self.update_compare_status(memory, offset),
        }
    }

    /// Writes one plane's byte at `offset`, applying the compare-unit read-modify
    /// combine when calc-before-write is active. Disabled planes are left
    /// untouched; the host drops writes to access-masked planes.
    fn write_plane<M: AluMemory>(&self, memory: &mut M, offset: u16, plane: u8, value: u8) {
        if self.plane_disabled[usize::from(plane)] {
            return;
        }
        let stored = if self.command & COMMAND_CALC_BEFORE_WRITE != 0 {
            let existing = memory.read_plane(offset, plane);
            let status = self.compare_status;
            let (kept, incoming) = if self.command & COMMAND_MASK_SELECT != 0 {
                (existing & status, value & !status)
            } else {
                (existing & !status, value & status)
            };
            kept | incoming
        } else {
            value
        };
        memory.write_plane(offset, plane, stored);
    }

    /// Merges a freshly computed plane value with the existing pixels through the
    /// write mask (`1` preserves the existing pixel).
    fn merge(&self, source: u8, new_value: u8) -> u8 {
        (source & self.write_mask) | (new_value & !self.write_mask)
    }

    /// Sets each enabled plane to the solid logical colour.
    fn op_pset<M: AluMemory>(&mut self, memory: &mut M, offset: u16) {
        for plane in 0..PLANE_COUNT {
            if self.plane_disabled[usize::from(plane)] {
                continue;
            }
            let source = memory.read_plane(offset, plane);
            let solid = if self.logical_color & (1 << plane) != 0 {
                0xFF
            } else {
                0x00
            };
            let merged = self.merge(source, solid);
            self.write_plane(memory, offset, plane, merged);
        }
    }

    /// Clears the unmasked pixels of each enabled plane.
    fn op_blank<M: AluMemory>(&mut self, memory: &mut M, offset: u16) {
        for plane in 0..PLANE_COUNT {
            if self.plane_disabled[usize::from(plane)] {
                continue;
            }
            let source = memory.read_plane(offset, plane);
            self.write_plane(memory, offset, plane, source & self.write_mask);
        }
    }

    /// Combines the logical colour with each enabled plane via OR, AND or XOR.
    fn op_logical<M: AluMemory>(&mut self, memory: &mut M, offset: u16, op: LogicalOp) {
        for plane in 0..PLANE_COUNT {
            if self.plane_disabled[usize::from(plane)] {
                continue;
            }
            let source = memory.read_plane(offset, plane);
            let color_set = self.logical_color & (1 << plane) != 0;
            let new_value = match op {
                LogicalOp::Or => {
                    if color_set {
                        0xFF
                    } else {
                        source
                    }
                }
                LogicalOp::And => {
                    if color_set {
                        source
                    } else {
                        0x00
                    }
                }
                LogicalOp::Xor => {
                    let color = if color_set { 0xFF } else { 0x00 };
                    color ^ source
                }
            };
            let merged = self.merge(source, new_value);
            self.write_plane(memory, offset, plane, merged);
        }
    }

    /// Inverts each enabled plane.
    fn op_not<M: AluMemory>(&mut self, memory: &mut M, offset: u16) {
        for plane in 0..PLANE_COUNT {
            if self.plane_disabled[usize::from(plane)] {
                continue;
            }
            let source = memory.read_plane(offset, plane);
            let merged = self.merge(source, !source);
            self.write_plane(memory, offset, plane, merged);
        }
    }

    /// Stamps the per-plane tile pattern into each enabled plane.
    fn op_tilepaint<M: AluMemory>(&mut self, memory: &mut M, offset: u16) {
        for plane in 0..PLANE_COUNT {
            if self.plane_disabled[usize::from(plane)] {
                continue;
            }
            let source = memory.read_plane(offset, plane);
            let merged = self.merge(source, self.tile[usize::from(plane)]);
            self.write_plane(memory, offset, plane, merged);
        }
    }

    /// Rebuilds the compare status: each bit marks a pixel whose blue/red/green
    /// colour matches one of the enabled compare colour banks.
    fn update_compare_status<M: AluMemory>(&mut self, memory: &M, offset: u16) {
        let enabled = self.enabled_planes();
        let mut banks = [0u8; 8];
        let mut bank_count = 0;
        for &color in self.compare_colors.iter() {
            if color & COMPARE_BANK_DISABLED == 0 {
                banks[bank_count] = color & enabled;
                bank_count += 1;
            }
        }
        self.compare_status = 0;
        if bank_count == 0 {
            return;
        }
        let mut blue = memory.read_plane(offset, 0);
        let mut red = memory.read_plane(offset, 1);
        let mut green = memory.read_plane(offset, 2);
        let mut status = 0u8;
        for _ in 0..8 {
            status <<= 1;
            let pixel_color =
                (((blue >> 7) & 1) | (((red >> 7) & 1) << 1) | (((green >> 7) & 1) << 2)) & enabled;
            if banks[..bank_count].contains(&pixel_color) {
                status |= 1;
            }
            blue <<= 1;
            red <<= 1;
            green <<= 1;
        }
        self.compare_status = status;
    }

    /// Rasterizes a hardware line from the start to the end coordinate, drawing
    /// each pixel through the operation pipeline and gating them by the line
    /// pattern. Returns the busy duration in microseconds (zero for lines too
    /// short to model), setting the busy flag while it is nonzero.
    fn draw_line<M: AluMemory>(&mut self, memory: &mut M) -> u64 {
        let width = memory.pixel_width() as i32;
        let x_begin = i32::from(self.line_x_begin);
        let y_begin = i32::from(self.line_y_begin);
        let x_end = i32::from(self.line_x_end);
        let y_end = i32::from(self.line_y_end);
        let delta_x = x_end - x_begin;
        let delta_y = y_end - y_begin;
        let x_count = delta_x.abs();
        let y_count = delta_y.abs();

        self.line_current_address = LINE_ADDRESS_NONE;
        self.line_last_address = LINE_ADDRESS_NONE;
        self.line_pattern_shift = self.line_pattern;
        self.busy = true;
        self.write_mask = 0xFF;
        if self.line_pattern_shift & 0x8000 != 0 {
            self.write_mask &= !PIXEL_BIT[(x_begin & 7) as usize];
        }

        let mut total_bytes = 0u32;
        let flush_final = self.trace_line(
            memory,
            x_begin,
            y_begin,
            x_end,
            y_end,
            delta_x,
            delta_y,
            x_count,
            y_count,
            width,
            &mut total_bytes,
        );

        if flush_final {
            let address = self.line_last_address;
            self.line_flush(memory, address);
            total_bytes += 1;
        }
        self.write_mask = 0xFF;

        let busy_micros = u64::from(total_bytes / LINE_BYTES_PER_MICROSECOND);
        if busy_micros == 0 {
            self.busy = false;
        }
        busy_micros
    }

    /// Walks the line coordinates, plotting pixels and counting touched bytes.
    /// Returns whether the trailing accumulated byte still needs flushing (false
    /// for the single-pixel and off-screen cases that finish inside the walk).
    #[allow(clippy::too_many_arguments)]
    fn trace_line<M: AluMemory>(
        &mut self,
        memory: &mut M,
        x_begin: i32,
        y_begin: i32,
        x_end: i32,
        y_end: i32,
        delta_x: i32,
        delta_y: i32,
        x_count: i32,
        y_count: i32,
        width: i32,
        total_bytes: &mut u32,
    ) -> bool {
        let mut cursor_x = x_begin;
        let mut cursor_y = y_begin;

        if y_count == 0 {
            if !(0..LINE_COORD_LIMIT).contains(&cursor_y) {
                return false;
            }
            if x_count == 0 {
                self.line_plot(memory, cursor_x, cursor_y, width);
                return false;
            }
            if delta_x > 0 {
                let mut remaining = x_end - cursor_x;
                if cursor_x & 0x07 != 7 {
                    *total_bytes = 1;
                }
                while cursor_x <= x_end {
                    if cursor_x & 7 == 0 && remaining >= 8 {
                        self.line_plot_byte(memory, cursor_x, cursor_y, width);
                        cursor_x += 8;
                        remaining -= 8;
                        *total_bytes += 1;
                        continue;
                    }
                    self.line_plot(memory, cursor_x, cursor_y, width);
                    cursor_x += 1;
                    remaining -= 1;
                    if cursor_x & 0x07 == 7 {
                        *total_bytes += 1;
                    }
                }
            } else {
                let mut remaining = cursor_x - x_end;
                if cursor_x & 0x07 != 0 {
                    *total_bytes = 1;
                }
                while cursor_x >= x_end {
                    if cursor_x < 0 {
                        break;
                    }
                    if cursor_x & 7 == 7 && remaining >= 8 {
                        self.line_plot_byte(memory, cursor_x, cursor_y, width);
                        cursor_x -= 8;
                        remaining -= 8;
                        *total_bytes += 1;
                        continue;
                    }
                    if cursor_x & 7 == 0 {
                        *total_bytes += 1;
                    }
                    self.line_plot(memory, cursor_x, cursor_y, width);
                    remaining -= 1;
                    cursor_x -= 1;
                }
            }
        } else if x_count == 0 {
            if cursor_x < 0 {
                return false;
            }
            if delta_y > 0 {
                while cursor_y <= y_end {
                    if cursor_y >= LINE_COORD_LIMIT {
                        break;
                    }
                    self.line_plot(memory, cursor_x, cursor_y, width);
                    *total_bytes += 1;
                    cursor_y += 1;
                }
            } else {
                while cursor_y >= y_end {
                    if cursor_y < 0 {
                        break;
                    }
                    self.line_plot(memory, cursor_x, cursor_y, width);
                    *total_bytes += 1;
                    cursor_y -= 1;
                }
            }
        } else if x_count > y_count {
            self.trace_line_x_major(
                memory,
                &mut cursor_x,
                &mut cursor_y,
                delta_x,
                delta_y,
                x_count,
                y_count,
                width,
                total_bytes,
            );
        } else if x_count == y_count {
            self.trace_line_diagonal(
                memory,
                &mut cursor_x,
                &mut cursor_y,
                delta_x,
                delta_y,
                x_count,
                width,
                total_bytes,
            );
        } else {
            self.trace_line_y_major(
                memory,
                &mut cursor_x,
                &mut cursor_y,
                delta_x,
                delta_y,
                x_count,
                y_count,
                width,
                total_bytes,
            );
        }
        true
    }

    /// Walks an X-major line, stepping Y as the error term accumulates.
    #[allow(clippy::too_many_arguments)]
    fn trace_line_x_major<M: AluMemory>(
        &mut self,
        memory: &mut M,
        cursor_x: &mut i32,
        cursor_y: &mut i32,
        delta_x: i32,
        delta_y: i32,
        x_count: i32,
        y_count: i32,
        width: i32,
        total_bytes: &mut u32,
    ) {
        let step = (y_count * LINE_DDA_FULL) / x_count;
        let step_byte = step << 3;
        if delta_x < 0 {
            if *cursor_x & 0x07 != 0 {
                *total_bytes = 1;
            }
        } else if *cursor_x & 0x07 == 0 {
            *total_bytes = 1;
        }
        let mut remaining = x_count;
        let mut error = 0;
        while remaining >= 0 {
            if step_byte + error <= LINE_DDA_HALF {
                if delta_x > 0 {
                    if *cursor_x & 0x07 == 0 && remaining >= 8 {
                        self.line_plot_byte(memory, *cursor_x, *cursor_y, width);
                        *total_bytes += 1;
                        error += step_byte;
                        remaining -= 8;
                        *cursor_x += 8;
                        continue;
                    }
                } else if *cursor_x & 0x07 == 7 && remaining >= 8 {
                    self.line_plot_byte(memory, *cursor_x, *cursor_y, width);
                    *total_bytes += 1;
                    error += step_byte;
                    remaining -= 8;
                    *cursor_x -= 8;
                    if *cursor_x < 0 {
                        break;
                    }
                    continue;
                }
            }
            self.line_plot(memory, *cursor_x, *cursor_y, width);
            error += step;
            if error > LINE_DDA_HALF {
                if delta_y < 0 {
                    *cursor_y -= 1;
                    if *cursor_y < 0 {
                        break;
                    }
                } else {
                    *cursor_y += 1;
                    if *cursor_y >= LINE_COORD_LIMIT {
                        break;
                    }
                }
                *total_bytes += 1;
                error -= LINE_DDA_FULL;
            }
            if delta_x > 0 {
                *cursor_x += 1;
                if *cursor_x & 0x07 == 0 {
                    *total_bytes += 1;
                }
            } else if delta_x < 0 {
                if *cursor_x & 0x07 == 0 {
                    *total_bytes += 1;
                }
                *cursor_x -= 1;
                if *cursor_x < 0 {
                    break;
                }
            }
            remaining -= 1;
        }
    }

    /// Walks a 45-degree line, stepping both axes each pixel.
    #[allow(clippy::too_many_arguments)]
    fn trace_line_diagonal<M: AluMemory>(
        &mut self,
        memory: &mut M,
        cursor_x: &mut i32,
        cursor_y: &mut i32,
        delta_x: i32,
        delta_y: i32,
        x_count: i32,
        width: i32,
        total_bytes: &mut u32,
    ) {
        if delta_x < 0 {
            if *cursor_x & 0x07 != 0 {
                *total_bytes = 1;
            }
        } else if *cursor_x & 0x07 == 0 {
            *total_bytes = 1;
        }
        let mut remaining = x_count;
        while remaining >= 0 {
            self.line_plot(memory, *cursor_x, *cursor_y, width);
            if delta_y < 0 {
                *cursor_y -= 1;
                if *cursor_y < 0 {
                    break;
                }
            } else {
                *cursor_y += 1;
                if *cursor_y >= LINE_COORD_LIMIT {
                    break;
                }
            }
            *total_bytes += 1;
            if delta_x > 0 {
                *cursor_x += 1;
            } else if delta_x < 0 {
                *cursor_x -= 1;
                if *cursor_x < 0 {
                    break;
                }
            }
            remaining -= 1;
        }
    }

    /// Walks a Y-major line, stepping X as the error term accumulates.
    #[allow(clippy::too_many_arguments)]
    fn trace_line_y_major<M: AluMemory>(
        &mut self,
        memory: &mut M,
        cursor_x: &mut i32,
        cursor_y: &mut i32,
        delta_x: i32,
        delta_y: i32,
        x_count: i32,
        y_count: i32,
        width: i32,
        total_bytes: &mut u32,
    ) {
        let step = (x_count * LINE_DDA_FULL) / y_count;
        let mut remaining = y_count;
        let mut error = 0;
        while remaining >= 0 {
            self.line_plot(memory, *cursor_x, *cursor_y, width);
            *total_bytes += 1;
            error += step;
            if error > LINE_DDA_HALF {
                if delta_x < 0 {
                    *cursor_x -= 1;
                    if *cursor_x < 0 {
                        break;
                    }
                } else if delta_x > 0 {
                    *cursor_x += 1;
                }
                error -= LINE_DDA_FULL;
            }
            if delta_y > 0 {
                *cursor_y += 1;
                if *cursor_y >= LINE_COORD_LIMIT {
                    break;
                }
            } else {
                *cursor_y -= 1;
                if *cursor_y < 0 {
                    break;
                }
            }
            remaining -= 1;
        }
    }

    /// Computes the byte address of a line pixel within its plane.
    fn line_address(&self, x: i32, y: i32, width: i32) -> u32 {
        let byte = ((y * width + x) >> 3) as u32;
        byte.wrapping_add(u32::from(self.line_address_offset)) & u32::from(IN_PLANE_ADDRESS_MASK)
    }

    /// Accumulates one line pixel into the pending byte mask, flushing the
    /// previous byte through the operation pipeline when the address changes.
    fn line_plot<M: AluMemory>(&mut self, memory: &mut M, x: i32, y: i32, width: i32) {
        if !self.is_enabled() {
            return;
        }
        if x < 0 || y < 0 {
            return;
        }
        let address = self.line_address(x, y, width);
        self.line_last_address = address;
        let pixel_mask = !PIXEL_BIT[(x & 7) as usize];
        let pattern_on = self.line_pattern_shift & 0x8000 != 0;
        if self.line_current_address != address {
            if self.line_current_address == LINE_ADDRESS_NONE {
                if pattern_on {
                    self.write_mask &= pixel_mask;
                }
                self.line_current_address = address;
            }
            let pending = self.line_current_address;
            self.line_flush(memory, pending);
            self.write_mask = 0xFF;
            self.line_current_address = address;
        }
        self.line_pattern_shift <<= 1;
        if pattern_on {
            self.write_mask &= pixel_mask;
            self.line_pattern_shift |= 1;
        }
    }

    /// Accumulates a whole aligned byte of a line, consuming eight pattern bits.
    fn line_plot_byte<M: AluMemory>(&mut self, memory: &mut M, x: i32, y: i32, width: i32) {
        if !self.is_enabled() {
            return;
        }
        if x < 0 || y < 0 {
            return;
        }
        let address = self.line_address(x, y, width);
        self.line_last_address = address;
        if self.line_current_address != address {
            if self.line_current_address == LINE_ADDRESS_NONE {
                if self.line_pattern_shift & 0x8000 != 0 {
                    self.write_mask &= !PIXEL_BIT[(x & 7) as usize];
                }
                self.line_current_address = address;
            }
            let pending = self.line_current_address;
            self.line_flush(memory, pending);
            self.write_mask = 0xFF;
            self.line_current_address = address;
        }
        let high = (self.line_pattern_shift >> 8) as u8;
        self.write_mask &= !high;
        let low = (self.line_pattern_shift & 0xFF) as u8;
        self.line_pattern_shift = (u16::from(low) << 8) | u16::from(high);
    }

    /// Flushes an accumulated line byte through the operation pipeline.
    fn line_flush<M: AluMemory>(&mut self, memory: &mut M, address: u32) {
        if address == LINE_ADDRESS_NONE {
            return;
        }
        self.apply_operation(memory, (address as u16) & IN_PLANE_ADDRESS_MASK);
    }
}
