use super::*;

const OPCODE_SOLID_RECTANGLE: u16 = 0x6FE8;
const OPCODE_ROP_SOLID_RECTANGLE_FOREGROUND: u16 = 0x4AE8;
const OPCODE_ROP_SOLID_RECTANGLE_ALTERNATE: u16 = 0x42F8;
const OPCODE_HGA_ROP_SOLID_RECTANGLE_FOREGROUND: u16 = 0x6AE8;
const OPCODE_ROP_RECTANGLE_FOREGROUND: u16 = 0x6A28;
const OPCODE_SOLID_RECTANGLE_SOURCE: u16 = 0x4FE8;
const OPCODE_SOLID_RECTANGLE_ALTERNATE: u16 = 0x4FF8;
const OPCODE_HOST_COLOR_EXPAND: u16 = 0x0AC8;
const OPCODE_TILED_RECTANGLE: u16 = 0x50E8;
const OPCODE_IMAGE_RESTORE: u16 = 0x45E8;
const OPCODE_HGA_ROP_IMAGE_RESTORE: u16 = 0x4528;
const OPCODE_PATTERN_EXPAND_RECTANGLE: u16 = 0x4688;
const OPCODE_OPAQUE_PATTERN_EXPAND_RECTANGLE: u16 = 0x4A88;
const OPCODE_PIXEL_READ: u16 = 0x20E8;
const OPCODE_HGA_COPY_RECTANGLE_BASE: u16 = 0x6028;
const OPCODE_COPY_RECTANGLE_BASE: u16 = 0x60E8;
const OPCODE_SOLID_LINE_BASE: u16 = 0x1FE8;
const OPCODE_STYLED_LINE_BASE: u16 = 0x1348;
const OPCODE_ROP_LINE_BASE: u16 = 0x1A48;
const OPCODE_HGA_ROP_LINE_BASE: u16 = 0x1A58;

const DIRECTION_Y_MAJOR: u8 = 0x01;
const DIRECTION_DESCENDING_Y: u8 = 0x02;
const DIRECTION_DESCENDING_X: u8 = 0x04;
const NORMAL_WRITE_BIT: u16 = 0x1000;
const MIX_XOR: u8 = 0x06;
const MIX_DESTINATION: u8 = 0x0A;
const MIX_SOURCE: u8 = 0x0C;
const CLIP_CONTROL_ENABLE: u16 = 0x0001;
const CLIP_CONTROL_OUTSIDE: u16 = 0x0002;
const POP1_SCANLINE_PIXEL_READ: u16 = 0x3000;
// HGA256.DRV and HGA64K.DRV pad DIB restore rows to 32-bit boundaries.
const INDEXED_IMAGE_RESTORE_ROW_ALIGNMENT: u32 = 4;
const DIRECT_COLOR16_IMAGE_RESTORE_ROW_ALIGNMENT: u32 = 2;
const PIXEL_READ_WORD_WIDTH: u32 = 16;
const TILE_WIDTH: u32 = 8;
const TILE_HEIGHT: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PixelMix {
    Source,
    Xor,
}

impl Ga1280a {
    pub(super) fn execute_pop2(&mut self, opcode: u16) {
        match opcode {
            OPCODE_SOLID_RECTANGLE
            | OPCODE_SOLID_RECTANGLE_SOURCE
            | OPCODE_SOLID_RECTANGLE_ALTERNATE => self.execute_solid_rectangle(),
            OPCODE_ROP_SOLID_RECTANGLE_FOREGROUND
            | OPCODE_ROP_SOLID_RECTANGLE_ALTERNATE
            | OPCODE_HGA_ROP_SOLID_RECTANGLE_FOREGROUND => {
                self.execute_rop_solid_rectangle_foreground()
            }
            OPCODE_ROP_RECTANGLE_FOREGROUND => self.execute_rop_rectangle_foreground(),
            OPCODE_HOST_COLOR_EXPAND => self.execute_host_color_expand(),
            OPCODE_TILED_RECTANGLE => self.execute_tiled_rectangle(),
            OPCODE_PATTERN_EXPAND_RECTANGLE => self.execute_pattern_expand_rectangle(false),
            OPCODE_OPAQUE_PATTERN_EXPAND_RECTANGLE => self.execute_pattern_expand_rectangle(true),
            OPCODE_PIXEL_READ => self.execute_pixel_read(),
            value if value & !0x0007 == OPCODE_IMAGE_RESTORE => {
                self.execute_image_restore((value & 0x0007) as u8)
            }
            value if value & !0x0007 == OPCODE_HGA_ROP_IMAGE_RESTORE => {
                self.execute_rop_image_restore((value & 0x0007) as u8)
            }
            value if value & !0x0007 == OPCODE_HGA_COPY_RECTANGLE_BASE => {
                self.execute_copy_rectangle_with_mix((value & 0x0007) as u8)
            }
            value if value & !0x0007 == OPCODE_COPY_RECTANGLE_BASE => {
                self.execute_copy_rectangle((value & 0x0007) as u8)
            }
            value if value & !0x0007 == OPCODE_SOLID_LINE_BASE => {
                self.execute_solid_line((value & 0x0007) as u8)
            }
            value if value & !0x0007 == OPCODE_STYLED_LINE_BASE => {
                self.execute_styled_line((value & 0x0007) as u8)
            }
            value
                if value & !0x0007 == OPCODE_ROP_LINE_BASE
                    || value & !0x0007 == OPCODE_HGA_ROP_LINE_BASE =>
            {
                self.execute_rop_line((value & 0x0007) as u8)
            }
            value => self.warn_unknown_command(value),
        }
    }

    pub(super) fn write_pdt_word(&mut self, value: u16) {
        self.state.pdt = value;
        self.state.pdt_latch[0] = value;
        self.state.pdt_read_phase = 0;
        match self.state.stream {
            Ga1280aStreamState::PixelRead(_) => {
                self.state.stream = Ga1280aStreamState::Inactive;
            }
            Ga1280aStreamState::PatternExpand(_) => {
                self.consume_pattern_expand_word(value);
            }
            Ga1280aStreamState::ImageRestore(_) => {
                self.consume_image_restore_word(value);
            }
            Ga1280aStreamState::Inactive => {}
        }
    }

    pub(super) fn read_pdt_word(&mut self) -> u16 {
        if matches!(self.state.stream, Ga1280aStreamState::PixelRead(_)) {
            return self.read_pixel_read_word();
        }

        let phase = self.state.pdt_read_phase as usize % self.state.pdt_latch.len();
        let value = self.state.pdt_latch[phase];
        self.state.pdt = value;
        self.state.pdt_read_phase = self.state.pdt_read_phase.wrapping_add(1) & 0x03;
        value
    }

    pub(super) fn write_cwb(&mut self, value: u16) {
        self.state.cwb = value;
        match value & 0xF000 {
            0x0000 => self.state.clip_sy = value & 0x0FFF,
            0x1000 => self.state.clip_sx = value & 0x0FFF,
            0x2000 => self.state.clip_ey = value & 0x0FFF,
            0x3000 => self.state.clip_ex = value & 0x0FFF,
            0x4000 => {
                self.state.clip_enabled = value & CLIP_CONTROL_ENABLE != 0;
                self.state.clip_outside = value & CLIP_CONTROL_OUTSIDE != 0;
            }
            _ => {}
        }
    }

    fn execute_solid_rectangle(&mut self) {
        let mix = self.foreground_mix();
        let color = u32::from(self.state.col);
        self.execute_solid_rectangle_color(color, mix);
    }

    fn execute_rop_solid_rectangle_foreground(&mut self) {
        let width = u32::from(self.state.opd1) + 1;
        let height = u32::from(self.state.opd2) + 1;
        let start_x = u32::from(self.state.dstx);
        let start_y = u32::from(self.state.dsty);
        let color = u32::from(self.state.fcol);
        let rop = self.state.fmix;

        for y in start_y..start_y.saturating_add(height) {
            for x in start_x..start_x.saturating_add(width) {
                self.write_pixel_rop(x, y, color, rop);
            }
        }
    }

    fn execute_rop_rectangle_foreground(&mut self) {
        let width = u32::from(self.state.opd1) + 1;
        let height = u32::from(self.state.opd2) + 1;
        let start_x = u32::from(self.state.dstx);
        let start_y = u32::from(self.state.dsty);

        for row in 0..height {
            for column in 0..width {
                let source_x = u32::from(self.state.srcx) + column;
                let source_y = u32::from(self.state.srcy) + row;
                let (source, rop) = if self.rop_pattern_bit(source_x, source_y) {
                    (u32::from(self.state.fcol), self.state.fmix)
                } else {
                    (u32::from(self.state.bcol), self.state.bmix)
                };
                self.write_pixel_rop(start_x + column, start_y + row, source, rop);
            }
        }
    }

    fn execute_host_color_expand(&mut self) {
        self.state.stream = Ga1280aStreamState::Inactive;
    }

    fn execute_solid_rectangle_color(&mut self, color: u32, mix: PixelMix) {
        let width = u32::from(self.state.opd1) + 1;
        let height = u32::from(self.state.opd2) + 1;
        let start_x = u32::from(self.state.dstx);
        let start_y = u32::from(self.state.dsty);

        for y in start_y..start_y.saturating_add(height) {
            for x in start_x..start_x.saturating_add(width) {
                self.write_pixel_mixed(x, y, color, mix);
            }
        }
    }

    fn execute_copy_rectangle(&mut self, direction: u8) {
        let width = u32::from(self.state.opd1) + 1;
        let height = u32::from(self.state.opd2) + 1;

        for row in 0..height {
            for column in 0..width {
                let source_x = directed_coordinate(self.state.srcx, column, direction, true);
                let source_y = directed_coordinate(self.state.srcy, row, direction, false);
                let dest_x = directed_coordinate(self.state.dstx, column, direction, true);
                let dest_y = directed_coordinate(self.state.dsty, row, direction, false);
                let color = self.read_pixel_color_signed(source_x, source_y);
                self.write_pixel_mixed_signed(dest_x, dest_y, color, PixelMix::Source);
            }
        }
    }

    fn execute_copy_rectangle_with_mix(&mut self, direction: u8) {
        let width = u32::from(self.state.opd1) + 1;
        let height = u32::from(self.state.opd2) + 1;

        for row in 0..height {
            for column in 0..width {
                let source_x = directed_coordinate(self.state.srcx, column, direction, true);
                let source_y = directed_coordinate(self.state.srcy, row, direction, false);
                let dest_x = directed_coordinate(self.state.dstx, column, direction, true);
                let dest_y = directed_coordinate(self.state.dsty, row, direction, false);
                let color = self.read_pixel_color_signed(source_x, source_y);
                self.write_pixel_rop_signed(dest_x, dest_y, color, self.state.fmix);
            }
        }
    }

    fn execute_tiled_rectangle(&mut self) {
        let width = u32::from(self.state.opd1) + 1;
        let height = u32::from(self.state.opd2) + 1;
        let source_x = u32::from(self.state.srcx);
        let source_y = u32::from(self.state.srcy);
        let dest_x = u32::from(self.state.dstx);
        let dest_y = u32::from(self.state.dsty);
        let tile_base_x = source_x & !(TILE_WIDTH - 1);

        for row in 0..height {
            for column in 0..width {
                let tile_column = (source_x + column) % TILE_WIDTH;
                let tile_row = (dest_y + row) % TILE_HEIGHT;
                let x = tile_base_x + tile_row * TILE_WIDTH + tile_column;
                let color = self.read_pixel_color(x, source_y);
                self.write_pixel_mixed(dest_x + column, dest_y + row, color, PixelMix::Source);
            }
        }
    }

    fn execute_solid_line(&mut self, direction: u8) {
        self.compute_line_points(direction);
        let mix = self.foreground_mix();
        let color = u32::from(self.state.col);
        for index in 0..self.line_points.len() {
            let (_step, x, y) = self.line_points[index];
            self.write_pixel_mixed_signed(x, y, color, mix);
        }
    }

    fn execute_styled_line(&mut self, direction: u8) {
        self.compute_line_points(direction);
        let mix = self.foreground_mix();
        let color = u32::from(self.state.col);
        for index in 0..self.line_points.len() {
            let (step, x, y) = self.line_points[index];
            if line_style_bit(self.state.lins, step) {
                self.write_pixel_mixed_signed(x, y, color, mix);
            } else if self.state.bmix != MIX_DESTINATION {
                self.warn_unknown_mix(self.state.fmix, self.state.bmix, self.state.pop1);
            }
        }
    }

    fn execute_rop_line(&mut self, direction: u8) {
        self.compute_line_points(direction);
        for index in 0..self.line_points.len() {
            let (step, x, y) = self.line_points[index];
            let (source, rop) = if line_style_bit(self.state.lins, step) {
                (u32::from(self.state.fcol), self.state.fmix)
            } else {
                (u32::from(self.state.bcol), self.state.bmix)
            };
            self.write_pixel_rop_signed(x, y, source, rop);
        }
    }

    fn execute_pattern_expand_rectangle(&mut self, opaque: bool) {
        let width = u32::from(self.state.opd1) + 1;
        let height = u32::from(self.state.opd2) + 1;
        if width == 0 || height == 0 {
            self.state.stream = Ga1280aStreamState::Inactive;
            return;
        }

        self.state.stream = Ga1280aStreamState::PatternExpand(Ga1280aPatternExpandState {
            x: u32::from(self.state.dstx),
            y: u32::from(self.state.dsty),
            width,
            height,
            row: 0,
            column: 0,
            word_phase: 0,
            source_word: 0,
            foreground_color: u32::from(self.state.fcol),
            background_color: u32::from(self.state.bcol),
            foreground_mix: self.state.fmix,
            background_mix: self.state.bmix,
            opaque,
        });
    }

    fn execute_image_restore(&mut self, direction: u8) {
        let width = u32::from(self.state.opd1) + 1;
        let height = u32::from(self.state.opd2) + 1;
        if width == 0 || height == 0 {
            self.state.stream = Ga1280aStreamState::Inactive;
            return;
        }
        let xor_pixels = self.state.fmix == MIX_XOR;
        self.state.stream = Ga1280aStreamState::ImageRestore(Ga1280aImageRestoreState::new(
            u32::from(self.state.dstx),
            u32::from(self.state.dsty),
            width,
            height,
            xor_pixels,
            direction,
            None,
        ));
    }

    fn execute_rop_image_restore(&mut self, direction: u8) {
        let width = u32::from(self.state.opd1) + 1;
        let height = u32::from(self.state.opd2) + 1;
        if width == 0 || height == 0 {
            self.state.stream = Ga1280aStreamState::Inactive;
            return;
        }

        self.state.stream = Ga1280aStreamState::ImageRestore(Ga1280aImageRestoreState::new(
            u32::from(self.state.dstx),
            u32::from(self.state.dsty),
            width,
            height,
            false,
            direction,
            Some(self.state.fmix),
        ));
    }

    fn execute_pixel_read(&mut self) {
        if self.state.pop1 == POP1_SCANLINE_PIXEL_READ {
            self.execute_scanline_pixel_read();
            return;
        }

        self.state.stream = Ga1280aStreamState::Inactive;
        let color = self.read_pixel_color(u32::from(self.state.srcx), u32::from(self.state.srcy));
        self.state.pdt_latch = if self.state.plane_mode == Ga1280aPlaneMode::FullColor24 {
            [color as u16, ((color >> 16) & 0x00FF) as u16, 0, 0]
        } else {
            [color as u16; 4]
        };
        self.state.pdt = color as u16;
        self.state.pdt_read_phase = 0;
    }

    fn execute_scanline_pixel_read(&mut self) {
        let width = u32::from(self.state.opd1) + 1;
        let height = u32::from(self.state.opd2) + 1;
        if width == 0 || height == 0 {
            self.state.stream = Ga1280aStreamState::Inactive;
            return;
        }

        self.state.stream = Ga1280aStreamState::PixelRead(Ga1280aPixelReadState::new(
            u32::from(self.state.srcx),
            u32::from(self.state.srcy),
            width,
            height,
        ));
        self.state.pdt_read_phase = 0;
    }

    fn consume_image_restore_word(&mut self, value: u16) {
        match self.state.plane_mode {
            Ga1280aPlaneMode::Indexed8 => {
                self.consume_image_restore_indexed_word(value);
            }
            Ga1280aPlaneMode::DirectColor16 => {
                self.consume_image_restore_direct_color16_word(value);
            }
            Ga1280aPlaneMode::FullColor24 => {
                self.consume_image_restore_rgb_byte(value as u8);
                self.consume_image_restore_rgb_byte((value >> 8) as u8);
            }
        }
    }

    fn read_pixel_read_word(&mut self) -> u16 {
        let Ga1280aStreamState::PixelRead(state) = &self.state.stream else {
            return self.state.pdt;
        };
        let state = state.clone();
        if state.row >= state.height {
            self.state.stream = Ga1280aStreamState::Inactive;
            return self.state.pdt;
        }

        let mut value = 0u16;
        let y = state.y + state.row;
        for column in 0..PIXEL_READ_WORD_WIDTH {
            let source_column = state.column + column;
            if source_column >= state.width {
                break;
            }
            if self.pixel_read_selected_plane_bit(state.x + source_column, y) {
                value |= pattern_word_bit_mask(column);
            }
        }

        self.state.pdt = value;
        self.advance_pixel_read_word();
        value
    }

    fn pixel_read_selected_plane_bit(&self, x: u32, y: u32) -> bool {
        if x >= self.pixel_map_width() || y >= self.pixel_map_height() {
            return false;
        }

        let plane_mask = self.active_read_plane_mask();
        if plane_mask == 0 {
            return false;
        }

        self.read_packed_pixel(x, y) & plane_mask != 0
    }

    fn advance_pixel_read_word(&mut self) {
        let exhausted = {
            let Ga1280aStreamState::PixelRead(state) = &mut self.state.stream else {
                return;
            };
            let next_column = state.column + PIXEL_READ_WORD_WIDTH;
            if next_column >= state.width {
                state.column = 0;
                state.row += 1;
            } else {
                state.column = next_column;
            }
            state.row >= state.height
        };
        if exhausted {
            self.state.stream = Ga1280aStreamState::Inactive;
        }
    }

    fn consume_image_restore_indexed_word(&mut self, value: u16) {
        self.consume_image_restore_indexed_pixel(u32::from(value as u8));
        self.consume_image_restore_indexed_pixel(u32::from((value >> 8) as u8));
    }

    pub(super) fn consume_image_restore_indexed_pixel(&mut self, color: u32) {
        self.consume_image_restore_padded_pixel(color, INDEXED_IMAGE_RESTORE_ROW_ALIGNMENT);
    }

    fn consume_image_restore_direct_color16_word(&mut self, color: u16) {
        self.consume_image_restore_padded_pixel(
            u32::from(color),
            DIRECT_COLOR16_IMAGE_RESTORE_ROW_ALIGNMENT,
        );
    }

    fn consume_image_restore_padded_pixel(&mut self, color: u32, row_alignment: u32) {
        let (x, y, input_row, input_column, width, height, xor_pixels, direction, rop) = {
            let Ga1280aStreamState::ImageRestore(state) = &self.state.stream else {
                return;
            };
            (
                state.x,
                state.y,
                state.input_row,
                state.input_column,
                state.width,
                state.height,
                state.xor_pixels,
                state.direction,
                state.rop,
            )
        };
        if input_row >= height {
            self.state.stream = Ga1280aStreamState::Inactive;
            return;
        }

        if input_column < width {
            let Some((x, y)) = image_restore_destination(x, y, input_column, input_row, direction)
            else {
                self.advance_padded_image_restore_input(row_alignment);
                return;
            };
            if let Some(rop) = rop {
                self.write_pixel_rop(x, y, color, rop);
            } else {
                let mix = if xor_pixels {
                    PixelMix::Xor
                } else {
                    PixelMix::Source
                };
                self.write_pixel_mixed(x, y, color, mix);
            }
            if let Ga1280aStreamState::ImageRestore(state) = &mut self.state.stream {
                state.pixel_index += 1;
            }
        }

        self.advance_padded_image_restore_input(row_alignment);
    }

    fn advance_padded_image_restore_input(&mut self, row_alignment: u32) {
        let exhausted = {
            let Ga1280aStreamState::ImageRestore(state) = &mut self.state.stream else {
                return;
            };
            let input_width = align_up(state.width, row_alignment);
            state.input_column += 1;
            if state.input_column >= input_width {
                state.input_column = 0;
                state.input_row += 1;
            }
            state.input_row >= state.height
        };
        if exhausted {
            self.state.stream = Ga1280aStreamState::Inactive;
        }
    }

    fn consume_pattern_expand_word(&mut self, value: u16) {
        let (opaque, width, column, word_phase, source_word) = {
            let Ga1280aStreamState::PatternExpand(state) = &self.state.stream else {
                return;
            };
            (
                state.opaque,
                state.width,
                state.column,
                state.word_phase,
                state.source_word,
            )
        };

        if opaque {
            self.draw_pattern_expand_word(value, 0xFFFF, column);
            self.advance_pattern_expand_chunk(column, align_up(width, 32));
            return;
        }

        if word_phase == 0 {
            if let Ga1280aStreamState::PatternExpand(state) = &mut self.state.stream {
                state.source_word = value;
                state.word_phase = 1;
            }
            return;
        }

        let mask_word = value;
        self.draw_pattern_expand_row(source_word, mask_word);
        let exhausted = {
            let Ga1280aStreamState::PatternExpand(state) = &mut self.state.stream else {
                return;
            };
            state.word_phase = 0;
            state.row += 1;
            state.row >= state.height
        };
        if exhausted {
            self.state.stream = Ga1280aStreamState::Inactive;
        }
    }

    fn draw_pattern_expand_word(&mut self, source_word: u16, mask_word: u16, column_start: u32) {
        let Ga1280aStreamState::PatternExpand(state) = &self.state.stream else {
            return;
        };
        let state = state.clone();
        if state.row >= state.height {
            return;
        }

        let y = state.y + state.row;
        for column in 0..16 {
            let destination_column = column_start + column;
            if destination_column >= state.width {
                break;
            }
            if !pattern_word_bit(mask_word, column) {
                continue;
            }

            let x = state.x + destination_column;
            if pattern_word_bit(source_word, column) {
                self.write_pixel_rop(x, y, state.foreground_color, state.foreground_mix);
            } else if state.background_mix != MIX_DESTINATION {
                self.write_pixel_rop(x, y, state.background_color, state.background_mix);
            }
        }
    }

    fn draw_pattern_expand_row(&mut self, source_word: u16, mask_word: u16) {
        self.draw_pattern_expand_word(source_word, mask_word, 0);
    }

    fn advance_pattern_expand_chunk(&mut self, column: u32, input_width: u32) {
        let exhausted = {
            let Ga1280aStreamState::PatternExpand(state) = &mut self.state.stream else {
                return;
            };
            let next_column = column + 16;
            if next_column >= input_width {
                state.column = 0;
                state.row += 1;
            } else {
                state.column = next_column;
            }
            state.row >= state.height
        };
        if exhausted {
            self.state.stream = Ga1280aStreamState::Inactive;
        }
    }

    fn consume_image_restore_rgb_byte(&mut self, value: u8) {
        let completed_color = {
            let Ga1280aStreamState::ImageRestore(state) = &mut self.state.stream else {
                return;
            };
            let phase = state.byte_phase as usize;
            state.byte_accumulator[phase] = value;
            state.byte_phase = (state.byte_phase + 1) % 3;
            if state.byte_phase == 0 {
                let [red, green, blue] = state.byte_accumulator;
                Some((u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue))
            } else {
                None
            }
        };
        if let Some(color) = completed_color {
            self.consume_image_restore_pixel(color);
        }
    }

    fn consume_image_restore_pixel(&mut self, color: u32) {
        let (x, y, width, height, pixel_index, xor_pixels, direction, rop) = {
            let Ga1280aStreamState::ImageRestore(state) = &self.state.stream else {
                return;
            };
            (
                state.x,
                state.y,
                state.width,
                state.height,
                state.pixel_index,
                state.xor_pixels,
                state.direction,
                state.rop,
            )
        };
        if pixel_index >= width.saturating_mul(height) {
            self.state.stream = Ga1280aStreamState::Inactive;
            return;
        }

        let Some((x, y)) =
            image_restore_destination(x, y, pixel_index % width, pixel_index / width, direction)
        else {
            self.advance_image_restore_pixel(width, height);
            return;
        };
        if let Some(rop) = rop {
            self.write_pixel_rop(x, y, color, rop);
        } else {
            let mix = if xor_pixels {
                PixelMix::Xor
            } else {
                PixelMix::Source
            };
            self.write_pixel_mixed(x, y, color, mix);
        }
        self.advance_image_restore_pixel(width, height);
    }

    fn advance_image_restore_pixel(&mut self, width: u32, height: u32) {
        let exhausted = {
            let Ga1280aStreamState::ImageRestore(state) = &mut self.state.stream else {
                return;
            };
            state.pixel_index += 1;
            state.pixel_index >= width.saturating_mul(height)
        };
        if exhausted {
            self.state.stream = Ga1280aStreamState::Inactive;
        }
    }

    fn foreground_mix(&mut self) -> PixelMix {
        match (self.state.fmix, self.state.pop1 & NORMAL_WRITE_BIT != 0) {
            (MIX_XOR, false) => PixelMix::Xor,
            (MIX_SOURCE, true) => PixelMix::Source,
            (MIX_XOR, true) => PixelMix::Xor,
            (MIX_SOURCE, false) => PixelMix::Source,
            _ => {
                self.warn_unknown_mix(self.state.fmix, self.state.bmix, self.state.pop1);
                PixelMix::Source
            }
        }
    }

    fn compute_line_points(&mut self, direction: u8) {
        // GALIB precomputes the Bresenham terms; POP2 only consumes them.
        let major_len = usize::from(self.state.opd1);
        let y_major = direction & DIRECTION_Y_MAJOR != 0;
        let x_step = if direction & DIRECTION_DESCENDING_X != 0 {
            -1
        } else {
            1
        };
        let y_step = if direction & DIRECTION_DESCENDING_Y != 0 {
            -1
        } else {
            1
        };
        let mut x = i32::from(self.state.dstx);
        let mut y = i32::from(self.state.dsty);
        let mut error = signed_word(self.state.errs);
        let k1 = signed_word(self.state.k1);
        let k2 = signed_word(self.state.k2);

        self.line_points.clear();

        for step in 0..=major_len {
            self.line_points.push((step, x, y));
            if step == major_len {
                break;
            }
            if error >= 0 {
                if y_major {
                    x += x_step;
                } else {
                    y += y_step;
                }
                error += k2;
            } else {
                error += k1;
            }
            if y_major {
                y += y_step;
            } else {
                x += x_step;
            }
        }
    }

    fn read_pixel_color_signed(&self, x: i32, y: i32) -> u32 {
        let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
            return 0;
        };
        self.read_pixel_color(x, y)
    }

    pub(super) fn read_pixel_color(&self, x: u32, y: u32) -> u32 {
        self.read_packed_pixel(x, y) & self.active_color_mask()
    }

    fn write_pixel_mixed_signed(&mut self, x: i32, y: i32, color: u32, mix: PixelMix) {
        let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
            return;
        };
        self.write_pixel_mixed(x, y, color, mix);
    }

    pub(super) fn write_pixel_mixed(&mut self, x: u32, y: u32, color: u32, mix: PixelMix) {
        if !self.pixel_writable(x, y) {
            return;
        }
        let current = self.read_pixel_color(x, y);
        let color_mask = self.active_color_mask();
        let result = match mix {
            PixelMix::Source => color,
            PixelMix::Xor => current ^ color,
        } & color_mask;
        self.write_pixel_color(x, y, result);
    }

    fn write_pixel_rop_signed(&mut self, x: i32, y: i32, source: u32, rop: u8) {
        let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
            return;
        };
        self.write_pixel_rop(x, y, source, rop);
    }

    pub(super) fn write_pixel_rop(&mut self, x: u32, y: u32, source: u32, rop: u8) {
        if !self.pixel_writable(x, y) {
            return;
        }

        let mask = self.active_color_mask();
        let destination = self.read_pixel_color(x, y);
        let result = apply_rop(rop, source & mask, destination & mask) & mask;
        self.write_pixel_color(x, y, result);
    }

    pub(super) fn write_pixel_color(&mut self, x: u32, y: u32, color: u32) {
        let plane_mask = self.active_write_plane_mask();
        if plane_mask == 0 {
            return;
        }
        let Some(current) = self.read_packed_pixel_checked(x, y) else {
            return;
        };
        let new_color = (current & !plane_mask) | (color & plane_mask);
        self.write_packed_pixel(x, y, new_color);
    }

    fn rop_pattern_bit(&self, x: u32, y: u32) -> bool {
        let row = self.state.rop_pattern[(y as usize) & (ROP_PATTERN_ROWS - 1)];
        row & (0x80 >> (x & 7)) != 0
    }

    fn pixel_writable(&self, x: u32, y: u32) -> bool {
        if x >= self.pixel_map_width() || y >= self.pixel_map_height() {
            return false;
        }
        if !self.write_bit_mask_allows(x) {
            return false;
        }
        if !self.state.clip_enabled {
            return true;
        }

        let inside = x >= u32::from(self.state.clip_sx)
            && x <= u32::from(self.state.clip_ex)
            && y >= u32::from(self.state.clip_sy)
            && y <= u32::from(self.state.clip_ey);
        if self.state.clip_outside {
            !inside
        } else {
            inside
        }
    }

    fn write_bit_mask_allows(&self, x: u32) -> bool {
        let byte_in_line = x / 8;
        let bit = 0x80 >> (x & 7);
        let mask = if byte_in_line & 1 == 0 {
            self.state.wbm as u8
        } else {
            (self.state.wbm >> 8) as u8
        };
        mask & bit != 0
    }

    fn active_color_mask(&self) -> u32 {
        match self.state.plane_mode {
            Ga1280aPlaneMode::Indexed8 => 0x0000FF,
            Ga1280aPlaneMode::DirectColor16 => 0x00FFFF,
            Ga1280aPlaneMode::FullColor24 => 0xFFFFFF,
        }
    }

    fn warn_unknown_command(&mut self, value: u16) {
        if self.state.unknown_command_warning_count == 0 {
            common::warn!("Unhandled I-O DATA GA POP2 command {value:#06X}");
        }
        self.state.unknown_command_warning_count += 1;
    }

    fn warn_unknown_mix(&mut self, fmix: u8, bmix: u8, pop1: u16) {
        if self.state.unknown_mix_warning_count == 0 {
            common::warn!(
                "Unhandled I-O DATA GA mix combination fmix={fmix:#04X} bmix={bmix:#04X} pop1={pop1:#06X}"
            );
        }
        self.state.unknown_mix_warning_count += 1;
    }
}

pub(super) fn apply_rop(code: u8, source: u32, destination: u32) -> u32 {
    match code & 0x0F {
        0 => 0,
        1 => !source & !destination,
        2 => !source & destination,
        3 => !source,
        4 => source & !destination,
        5 => !destination,
        6 => source ^ destination,
        7 => !source | !destination,
        8 => source & destination,
        9 => !source ^ destination,
        10 => destination,
        11 => !source | destination,
        12 => source,
        13 => source | !destination,
        14 => source | destination,
        15 => u32::MAX,
        _ => unreachable!(),
    }
}

fn directed_coordinate(start: u16, offset: u32, direction: u8, x_axis: bool) -> i32 {
    let descending = if x_axis {
        direction & DIRECTION_DESCENDING_X != 0
    } else {
        direction & DIRECTION_DESCENDING_Y != 0
    };
    let start = i32::from(start);
    let offset = offset as i32;
    if descending {
        start - offset
    } else {
        start + offset
    }
}

fn image_restore_destination(
    start_x: u32,
    start_y: u32,
    column: u32,
    row: u32,
    direction: u8,
) -> Option<(u32, u32)> {
    let x = directed_coordinate_u32(start_x, column, direction, true)?;
    let y = directed_coordinate_u32(start_y, row, direction, false)?;
    Some((x, y))
}

fn directed_coordinate_u32(start: u32, offset: u32, direction: u8, x_axis: bool) -> Option<u32> {
    let descending = if x_axis {
        direction & DIRECTION_DESCENDING_X != 0
    } else {
        direction & DIRECTION_DESCENDING_Y != 0
    };
    if descending {
        start.checked_sub(offset)
    } else {
        start.checked_add(offset)
    }
}

fn signed_word(value: u16) -> i32 {
    i32::from(i16::from_ne_bytes(value.to_ne_bytes()))
}

fn align_up(value: u32, alignment: u32) -> u32 {
    value.div_ceil(alignment) * alignment
}

fn line_style_bit(style: u16, step: usize) -> bool {
    style & (0x8000 >> (step & 0x0F)) != 0
}

fn pattern_word_bit(word: u16, column: u32) -> bool {
    if column < 8 {
        word & (0x0080 >> column) != 0
    } else if column < 16 {
        word & (0x8000 >> (column - 8)) != 0
    } else {
        false
    }
}

fn pattern_word_bit_mask(column: u32) -> u16 {
    if column < 8 {
        0x0080 >> column
    } else if column < 16 {
        0x8000 >> (column - 8)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hga_rop_solid_rectangle_uses_foreground_color_and_mix() {
        let mut ga = Ga1280a::new();
        ga.state.wpm = 0x00FF;
        ga.state.wbm = 0xFFFF;
        ga.state.fcol = 0x05;
        ga.state.fmix = MIX_XOR;
        ga.state.dstx = 3;
        ga.state.dsty = 4;
        ga.state.opd1 = 1;
        ga.state.opd2 = 0;
        ga.write_pixel_color(3, 4, 0x03);
        ga.write_pixel_color(4, 4, 0x04);

        ga.execute_pop2(OPCODE_HGA_ROP_SOLID_RECTANGLE_FOREGROUND);

        assert_eq!(ga.read_pixel_color(3, 4), 0x06);
        assert_eq!(ga.read_pixel_color(4, 4), 0x01);
    }

    #[test]
    fn alternate_rop_solid_rectangle_uses_foreground_color() {
        let mut ga = Ga1280a::new();
        ga.state.wpm = 0x00FF;
        ga.state.wbm = 0xFFFF;
        ga.state.dstx = 3;
        ga.state.dsty = 4;
        ga.state.opd1 = 2;
        ga.state.opd2 = 1;
        ga.state.fcol = 7;
        ga.state.col = 2;
        ga.state.fmix = MIX_SOURCE;

        ga.execute_pop2(OPCODE_ROP_SOLID_RECTANGLE_ALTERNATE);

        assert_eq!(ga.read_pixel_color(3, 4), 7);
        assert_eq!(ga.read_pixel_color(5, 5), 7);
        assert_eq!(ga.read_pixel_color(6, 5), 0);
    }

    #[test]
    fn hga_rop_rectangle_uses_foreground_color_and_mix() {
        let mut ga = Ga1280a::new();
        ga.state.wpm = 0x00FF;
        ga.state.wbm = 0xFFFF;
        ga.state.fcol = 0x05;
        ga.state.fmix = MIX_XOR;
        ga.state.dstx = 3;
        ga.state.dsty = 4;
        ga.state.opd1 = 1;
        ga.state.opd2 = 0;
        ga.write_pixel_color(3, 4, 0x03);
        ga.write_pixel_color(4, 4, 0x04);

        ga.execute_pop2(OPCODE_ROP_RECTANGLE_FOREGROUND);

        assert_eq!(ga.read_pixel_color(3, 4), 0x06);
        assert_eq!(ga.read_pixel_color(4, 4), 0x01);
    }

    #[test]
    fn hga_copy_rectangle_uses_foreground_mix() {
        let mut ga = Ga1280a::new();
        ga.state.pmw = 1023;
        ga.state.pmh = 1023;
        ga.state.wpm = 0x00FF;
        ga.state.wbm = 0xFFFF;
        ga.state.srcx = 0;
        ga.state.srcy = 768;
        ga.state.dstx = 10;
        ga.state.dsty = 20;
        ga.state.opd1 = 1;
        ga.state.opd2 = 0;
        ga.state.fmix = 0x08;
        ga.write_pixel_color(0, 768, 0xFF);
        ga.write_pixel_color(1, 768, 0x00);
        ga.write_pixel_color(10, 20, 0x07);
        ga.write_pixel_color(11, 20, 0x07);

        ga.execute_pop2(OPCODE_HGA_COPY_RECTANGLE_BASE);

        assert_eq!(ga.read_pixel_color(10, 20), 0x07);
        assert_eq!(ga.read_pixel_color(11, 20), 0x00);
    }

    #[test]
    fn host_window_writes_feed_indexed_image_restore_stream() {
        let mut ga = Ga1280a::new();
        ga.state.pmw = 1023;
        ga.state.pmh = 1023;
        ga.state.wpm = 0x00FF;
        ga.state.wbm = 0xFFFF;
        ga.state.wba1 = 0x0002;
        ga.state.dstx = 4;
        ga.state.dsty = 3;
        ga.state.opd1 = 2;
        ga.state.opd2 = 1;
        ga.state.fmix = MIX_SOURCE;

        ga.execute_pop2(OPCODE_IMAGE_RESTORE);
        for value in 1..=8 {
            ga.host_window_write(value, value as u8);
        }

        assert!(matches!(ga.state.stream, Ga1280aStreamState::Inactive));
        assert_eq!(ga.read_pixel_color(4, 3), 1);
        assert_eq!(ga.read_pixel_color(5, 3), 2);
        assert_eq!(ga.read_pixel_color(6, 3), 3);
        assert_eq!(ga.read_pixel_color(4, 4), 5);
        assert_eq!(ga.read_pixel_color(5, 4), 6);
        assert_eq!(ga.read_pixel_color(6, 4), 7);
    }

    #[test]
    fn image_restore_direction_bits_can_stream_rows_upward() {
        let mut ga = Ga1280a::new();
        ga.state.pmw = 1023;
        ga.state.pmh = 1023;
        ga.state.wpm = 0x00FF;
        ga.state.wbm = 0xFFFF;
        ga.state.dstx = 4;
        ga.state.dsty = 7;
        ga.state.opd1 = 3;
        ga.state.opd2 = 1;
        ga.state.fmix = MIX_SOURCE;

        ga.execute_pop2(OPCODE_IMAGE_RESTORE | u16::from(DIRECTION_DESCENDING_Y));
        ga.write_pdt_word(0x0201);
        ga.write_pdt_word(0x0403);
        ga.write_pdt_word(0x0605);
        ga.write_pdt_word(0x0807);

        assert!(matches!(ga.state.stream, Ga1280aStreamState::Inactive));
        assert_eq!(ga.read_pixel_color(4, 7), 1);
        assert_eq!(ga.read_pixel_color(5, 7), 2);
        assert_eq!(ga.read_pixel_color(6, 7), 3);
        assert_eq!(ga.read_pixel_color(7, 7), 4);
        assert_eq!(ga.read_pixel_color(4, 6), 5);
        assert_eq!(ga.read_pixel_color(5, 6), 6);
        assert_eq!(ga.read_pixel_color(6, 6), 7);
        assert_eq!(ga.read_pixel_color(7, 6), 8);
    }

    #[test]
    fn hga_rop_image_restore_applies_foreground_mix_to_packed_rows() {
        let mut ga = Ga1280a::new();
        ga.state.pmw = 1023;
        ga.state.pmh = 1023;
        ga.state.wpm = 0x00FF;
        ga.state.wbm = 0xFFFF;
        ga.state.dstx = 259;
        ga.state.dsty = 41;
        ga.state.opd1 = 6;
        ga.state.opd2 = 8;
        ga.state.fmix = 0x08;

        for y in 41..=49 {
            for x in 259..=266 {
                ga.write_pixel_color(x, y, 0x07);
            }
        }

        ga.execute_pop2(OPCODE_HGA_ROP_IMAGE_RESTORE);
        for value in [
            0xFFFF, 0x0000, 0xFF00, 0x00FF, 0xFFFF, 0x0000, 0xFF00, 0x02FF, 0xFFFF, 0x0000, 0xFF00,
            0x04FF, 0x0000, 0x0000, 0x0000, 0x0500, 0x00FF, 0x0000, 0x0000, 0x07FF, 0xFFFF, 0x0000,
            0xFF00, 0x08FF, 0xFFFF, 0x00FF, 0xFFFF, 0x0AFF, 0xFFFF, 0xFFFF, 0xFFFF, 0x0CFF, 0x0000,
            0x0000, 0x0000, 0x0D00,
        ] {
            ga.write_pdt_word(value);
        }

        assert!(matches!(ga.state.stream, Ga1280aStreamState::Inactive));
        assert_eq!(ga.read_pixel_color(266, 41), 0x07);
        assert_black_pixel_pattern(
            &ga,
            259,
            41,
            &[
                "..###..", "..###..", "..###..", "#######", ".#####.", "..###..", "...#...",
                ".......", "#######",
            ],
        );
    }

    #[test]
    fn tiled_rectangle_uses_destination_y_to_select_shadow_tile_row() {
        const SAMPLE_TILE: [u8; 64] = [
            0x00, 0x00, 0x66, 0x00, 0x00, 0x42, 0x3C, 0x00, 0x00, 0x00, 0x66, 0x00, 0x00, 0x42,
            0x3C, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x66, 0x00,
            0x00, 0x42, 0x3C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let mut ga = Ga1280a::new();
        ga.state.pmw = 1023;
        ga.state.pmh = 1023;
        ga.state.wpm = 0x00FF;
        ga.state.wbm = 0xFFFF;

        for row in 0..TILE_HEIGHT {
            for column in 0..TILE_WIDTH {
                ga.write_pixel_color(
                    640 + row * TILE_WIDTH + column,
                    769,
                    sample_tile_color(&SAMPLE_TILE, row, column),
                );
            }
        }

        ga.state.dstx = 13;
        ga.state.dsty = 21;
        ga.state.srcx = 640 + (ga.state.dstx & 7);
        ga.state.srcy = 769;
        ga.state.opd1 = 15;
        ga.state.opd2 = 7;

        ga.execute_pop2(OPCODE_TILED_RECTANGLE);

        for row in 0..=ga.state.opd2 {
            for column in 0..=ga.state.opd1 {
                let x = u32::from(ga.state.dstx) + u32::from(column);
                let y = u32::from(ga.state.dsty) + u32::from(row);
                let expected = sample_tile_color(&SAMPLE_TILE, y & 7, x & 7);
                assert_eq!(ga.read_pixel_color(x, y), expected, "pixel ({x},{y})");
            }
        }
    }

    #[test]
    fn opaque_pattern_expand_consumes_one_word_per_16_pixels() {
        let mut ga = Ga1280a::new();
        ga.state.wpm = 0x00FF;
        ga.state.wbm = 0xFFFF;
        ga.state.dstx = 8;
        ga.state.dsty = 0;
        ga.state.opd1 = 19;
        ga.state.opd2 = 0;
        ga.state.fcol = 5;
        ga.state.bcol = 2;
        ga.state.fmix = MIX_SOURCE;
        ga.state.bmix = MIX_SOURCE;

        ga.execute_pop2(OPCODE_OPAQUE_PATTERN_EXPAND_RECTANGLE);
        ga.write_pdt_word(0b1010_0000);
        ga.write_pdt_word(0b0100_0000);

        assert_eq!(ga.read_pixel_color(8, 0), 5);
        assert_eq!(ga.read_pixel_color(9, 0), 2);
        assert_eq!(ga.read_pixel_color(10, 0), 5);
        assert_eq!(ga.read_pixel_color(11, 0), 2);
        assert_eq!(ga.read_pixel_color(24, 0), 2);
        assert_eq!(ga.read_pixel_color(25, 0), 5);
        assert_eq!(ga.read_pixel_color(26, 0), 2);
        assert_eq!(ga.read_pixel_color(27, 0), 2);
    }

    #[test]
    fn transparent_pattern_expand_respects_row_mask() {
        let mut ga = Ga1280a::new();
        ga.state.wpm = 0x00FF;
        ga.state.wbm = 0xFFFF;
        ga.state.dstx = 8;
        ga.state.dsty = 0;
        ga.state.opd1 = 3;
        ga.state.opd2 = 0;
        ga.state.fcol = 7;
        ga.state.bcol = 3;
        ga.state.fmix = MIX_SOURCE;
        ga.state.bmix = MIX_DESTINATION;

        ga.write_pixel_color(9, 0, 4);
        ga.write_pixel_color(11, 0, 6);
        ga.execute_pop2(OPCODE_PATTERN_EXPAND_RECTANGLE);
        ga.write_pdt_word(0b1010_0000);
        ga.write_pdt_word(0b1110_0000);

        assert_eq!(ga.read_pixel_color(8, 0), 7);
        assert_eq!(ga.read_pixel_color(9, 0), 4);
        assert_eq!(ga.read_pixel_color(10, 0), 7);
        assert_eq!(ga.read_pixel_color(11, 0), 6);
    }

    #[test]
    fn rop_table_matches_galib_documentation() {
        let source = 0b1010_1100u32;
        let destination = 0b1100_1010u32;
        let mask = 0x00FF;
        let expected = [
            0,
            !source & !destination,
            !source & destination,
            !source,
            source & !destination,
            !destination,
            source ^ destination,
            !source | !destination,
            source & destination,
            !source ^ destination,
            destination,
            !source | destination,
            source,
            source | !destination,
            source | destination,
            u32::MAX,
        ];

        for (code, expected) in expected.into_iter().enumerate() {
            assert_eq!(
                apply_rop(code as u8, source, destination) & mask,
                expected & mask
            );
        }
    }

    fn sample_tile_color(tile: &[u8; 64], row: u32, column: u32) -> u32 {
        let bit = 0x80 >> column;
        let mut color = 0;
        for plane in 0..8 {
            if tile[plane * 8 + row as usize] & bit != 0 {
                color |= 1 << plane;
            }
        }
        color
    }

    fn assert_black_pixel_pattern(ga: &Ga1280a, x: u32, y: u32, pattern: &[&str]) {
        for (row, line) in pattern.iter().enumerate() {
            for (column, expected) in line.bytes().enumerate() {
                let color = ga.read_pixel_color(x + column as u32, y + row as u32);
                match expected {
                    b'#' => assert_eq!(color, 0, "pixel ({column},{row})"),
                    b'.' => assert_ne!(color, 0, "pixel ({column},{row})"),
                    _ => panic!("unexpected pattern byte {expected}"),
                }
            }
        }
    }
}
