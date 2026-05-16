use super::{accelerator::PixelMix, *};

const HOST_WRITE_PIXEL_MASK_MODE: u8 = 0x01;
const HOST_WRITE_ROTATE_WORD_MODE: u8 = 0x02;
const HOST_WRITE_COLOR_EXPAND_MODE: u8 = 0x04;

impl Ga1280a {
    /// Returns the active GA visible dimensions.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.state.active_width, self.state.active_height)
    }

    /// Handles the GA VSYNC blanking edge.
    pub fn on_vsync_start(&mut self) {
        self.state.vsync_active = true;
    }

    /// Handles the GA active-display edge.
    pub fn on_display_start(&mut self) {
        self.state.vsync_active = false;
    }

    /// Returns read-only inputs for renderer-side GA framebuffer composition.
    pub fn render_snapshot(&self) -> Ga1280aRenderSnapshot<'_> {
        Ga1280aRenderSnapshot {
            plane_mode: self.state.plane_mode,
            width: self.state.active_width,
            height: self.state.active_height,
            pixel_map_width: self.pixel_map_width(),
            pixel_map_height: self.pixel_map_height(),
            stride_bytes: self.packed_stride(),
            display_offset_pixels: u64::from(self.display_start())
                * u64::from(self.display_pixels_per_crtc_unit()),
            palette: &self.state.palette,
            visible_mask: self.state.vdac_mask,
            vram: &self.state.vram,
            cursor: Ga1280aCursorRenderSnapshot {
                visible: self.state.cursor_visible,
                x: self.state.cursor_x,
                y: self.state.cursor_y,
                colors: self.state.cursor_colors,
                xor_pattern: &self.state.cursor_xor_pattern,
                and_pattern: &self.state.cursor_and_pattern,
            },
        }
    }

    pub(super) fn host_window_read(&self, offset: u32) -> u8 {
        if self.uses_packed_host_pixels() {
            return match self.state.plane_mode {
                Ga1280aPlaneMode::FullColor24 => self.host_window_read_packed_full_color(offset),
                Ga1280aPlaneMode::Indexed8 | Ga1280aPlaneMode::DirectColor16 => unreachable!(),
            };
        }

        if self.uses_packed_indexed_host_pixels() {
            return self.host_window_read_packed_indexed(offset);
        }

        if self.uses_packed_direct16_host_pixels() {
            return self.host_window_read_packed_direct16(offset);
        }

        let Some((line, byte_in_line)) = self.host_window_position(offset, self.state.srr) else {
            return 0xFF;
        };
        if self.state.mod1 == HOST_WRITE_PIXEL_MASK_MODE {
            return self.host_window_read_pixel_mask(line, byte_in_line);
        }

        let plane = self.state.prs & 0x0F;
        self.vram_read_plane_byte(plane as usize, line, byte_in_line)
            .unwrap_or(0xFF)
    }

    pub(super) fn host_window_write(&mut self, offset: u32, value: u8) {
        if self.host_window_mapped_register_write_byte(offset, value) {
            return;
        }
        self.host_window_write_data(offset, value);
    }

    pub(super) fn flat_aperture_read_byte_at_offset(&mut self, offset: u32) -> u8 {
        if let Some(size) = self.flat_window_size() {
            if offset >= size {
                return 0xFF;
            }
            if let Some(value) = self.host_window_mapped_register_read_byte(offset) {
                return value;
            }
            return self.host_window_read(offset);
        }

        match self.state.plane_mode {
            Ga1280aPlaneMode::Indexed8 => {
                let Some((x, y, _)) = self.flat_aperture_position(self.state.srr, offset, 1) else {
                    return 0xFF;
                };
                self.read_pixel_color(x, y) as u8
            }
            Ga1280aPlaneMode::DirectColor16 => {
                let Some((x, y, component)) =
                    self.flat_aperture_position(self.state.srr, offset, 2)
                else {
                    return 0xFF;
                };
                let color = self.read_pixel_color(x, y) as u16;
                if component == 0 {
                    color as u8
                } else {
                    (color >> 8) as u8
                }
            }
            Ga1280aPlaneMode::FullColor24 => self.host_window_read_packed_full_color(offset),
        }
    }

    pub(super) fn flat_aperture_write_byte_at_offset(&mut self, offset: u32, value: u8) {
        self.state.flat_aperture_write_count += 1;
        if let Some(size) = self.flat_window_size() {
            if offset < size {
                self.host_window_write(offset, value);
            }
            return;
        }

        if matches!(self.state.stream, Ga1280aStreamState::ImageRestore(_))
            && self.state.plane_mode == Ga1280aPlaneMode::Indexed8
        {
            self.consume_image_restore_indexed_pixel(u32::from(value));
            return;
        }

        match self.state.plane_mode {
            Ga1280aPlaneMode::Indexed8 => {
                let Some((x, y, _)) = self.flat_aperture_position(self.state.srw, offset, 1) else {
                    return;
                };
                self.write_pixel_mixed(x, y, u32::from(value), PixelMix::Source);
            }
            Ga1280aPlaneMode::DirectColor16 => {
                let Some((x, y, component)) =
                    self.flat_aperture_position(self.state.srw, offset, 2)
                else {
                    return;
                };
                let mut color = self.read_pixel_color(x, y) as u16;
                if component == 0 {
                    color = (color & 0xFF00) | u16::from(value);
                } else {
                    color = (color & 0x00FF) | (u16::from(value) << 8);
                }
                self.write_pixel_mixed(x, y, u32::from(color), PixelMix::Source);
            }
            Ga1280aPlaneMode::FullColor24 => {
                self.host_window_write_packed_full_color(offset, value);
            }
        }
    }

    pub(super) fn flat_aperture_read_word_at_offset(&mut self, offset: u32) -> u16 {
        if let Some(size) = self.flat_window_size() {
            if offset.checked_add(1).is_none_or(|end| end >= size) {
                return 0xFFFF;
            }
            if let Some(value) = self.host_window_mapped_register_read_word(offset) {
                return value;
            }
        }

        u16::from(self.flat_aperture_read_byte_at_offset(offset))
            | (u16::from(self.flat_aperture_read_byte_at_offset(offset + 1)) << 8)
    }

    pub(super) fn flat_aperture_write_word_at_offset(&mut self, offset: u32, value: u16) {
        if let Some(size) = self.flat_window_size() {
            self.state.flat_aperture_write_count += 2;
            if offset.checked_add(1).is_none_or(|end| end >= size) {
                return;
            }
            if self.host_window_mapped_register_write_word(offset, value) {
                return;
            }
            self.host_window_write(offset, value as u8);
            self.host_window_write(offset + 1, (value >> 8) as u8);
            return;
        }

        self.flat_aperture_write_byte_at_offset(offset, value as u8);
        self.flat_aperture_write_byte_at_offset(offset + 1, (value >> 8) as u8);
    }

    pub(super) fn flat_aperture_read_dword_at_offset(&mut self, offset: u32) -> u32 {
        u32::from(self.flat_aperture_read_word_at_offset(offset))
            | (u32::from(self.flat_aperture_read_word_at_offset(offset + 2)) << 16)
    }

    pub(super) fn flat_aperture_write_dword_at_offset(&mut self, offset: u32, value: u32) {
        self.flat_aperture_write_word_at_offset(offset, value as u16);
        self.flat_aperture_write_word_at_offset(offset + 2, (value >> 16) as u16);
    }

    fn host_window_write_data(&mut self, offset: u32, value: u8) {
        self.state.host_window_write_count += 1;

        if matches!(self.state.stream, Ga1280aStreamState::ImageRestore(_))
            && self.state.plane_mode == Ga1280aPlaneMode::Indexed8
        {
            self.consume_image_restore_indexed_pixel(u32::from(value));
            return;
        }

        if self.uses_packed_host_pixels() {
            match self.state.plane_mode {
                Ga1280aPlaneMode::FullColor24 => {
                    self.host_window_write_packed_full_color(offset, value);
                }
                Ga1280aPlaneMode::Indexed8 | Ga1280aPlaneMode::DirectColor16 => unreachable!(),
            }
            return;
        }

        if self.uses_packed_indexed_host_pixels() {
            self.host_window_write_packed_indexed(offset, value);
            return;
        }

        if self.uses_packed_direct16_host_pixels() {
            self.host_window_write_packed_direct16(offset, value);
            return;
        }

        if self.state.mod1 == HOST_WRITE_ROTATE_WORD_MODE {
            self.host_window_rotate_word(offset);
            return;
        }

        let Some((line, byte_in_line)) = self.host_window_position(offset, self.state.srw) else {
            return;
        };
        let bit_mask = if byte_in_line & 1 == 0 {
            self.state.wbm as u8
        } else {
            (self.state.wbm >> 8) as u8
        };
        if bit_mask == 0 {
            return;
        }

        if self.state.mod1 == HOST_WRITE_PIXEL_MASK_MODE {
            self.host_window_write_pixel_mask(line, byte_in_line, value & bit_mask);
        } else if self.state.mod1 == HOST_WRITE_COLOR_EXPAND_MODE {
            self.host_window_write_color_expand(line, byte_in_line, value, bit_mask);
        } else {
            self.host_window_write_raw(line, byte_in_line, value, bit_mask);
        }
    }

    pub(super) fn update_dimensions_from_crtc(&mut self) {
        if self.state.plane_mode == Ga1280aPlaneMode::FullColor24 {
            self.state.active_width = FULL_COLOR_WIDTH;
            self.state.active_height = FULL_COLOR_HEIGHT;
            return;
        }

        let width = (u32::from(self.state.crtc_registers[0x02]) + 1)
            * self.horizontal_pixels_per_crtc_unit();
        let height = u32::from(self.state.crtc_registers[0x12]) + 1;
        if self.state.crtc_registers[0x02] != 0 {
            self.state.active_width = clamp_visible_width(width);
        }
        if self.state.crtc_registers[0x12] != 0 {
            self.state.active_height = clamp_visible_height(height);
        }
    }

    pub(super) fn update_plane_mode_after_vdac_index_write(&mut self, value: u8) {
        if self.crtc_matches_full_color_mode() {
            self.enter_full_color_mode();
            return;
        }
        if self.state.vdac_rs != 2 {
            return;
        }
        match value {
            0x38 => {
                self.state.plane_mode = Ga1280aPlaneMode::DirectColor16;
                self.update_dimensions_from_crtc();
            }
            0x48 => {
                self.state.plane_mode = Ga1280aPlaneMode::Indexed8;
                self.update_dimensions_from_crtc();
            }
            _ => {}
        }
    }

    pub(super) fn update_plane_mode_after_vdac_mask_write(&mut self, _value: u8) {
        if self.crtc_matches_full_color_mode() {
            self.enter_full_color_mode();
        }
    }

    fn host_window_position(&self, offset: u32, start_line: u16) -> Option<(u32, u32)> {
        let bytes_per_line = self.host_bytes_per_line();
        if bytes_per_line == 0 {
            return None;
        }
        let line = self.raster_line(start_line, offset / bytes_per_line)?;
        let byte_in_line = offset % bytes_per_line;
        Some((line, byte_in_line))
    }

    fn raster_line(&self, start: u16, line_offset: u32) -> Option<u32> {
        let height = self.pixel_map_height();
        if height == 0 {
            return None;
        }
        let line = (u32::from(start).wrapping_add(line_offset)) % height;
        Some(line)
    }

    fn uses_packed_host_pixels(&self) -> bool {
        if self.state.mod1 != 0 || self.state.wbm != 0xFFFF {
            return false;
        }

        self.state.plane_mode == Ga1280aPlaneMode::FullColor24 && self.state.wpm == 0xFFFF
    }

    fn uses_packed_indexed_host_pixels(&self) -> bool {
        self.state.plane_mode == Ga1280aPlaneMode::Indexed8
            && self.state.mod1 == 0
            && self.state.wbm == 0xFFFF
            && self.state.wpm & 0x00FF == 0x00FF
            && Self::window_size_from(self.state.wba1).is_none()
            && Self::window_size_from(self.state.wba2).is_some()
    }

    fn uses_packed_direct16_host_pixels(&self) -> bool {
        self.state.plane_mode == Ga1280aPlaneMode::DirectColor16
            && self.state.mod1 == 0
            && self.state.wbm == 0xFFFF
            && self.state.wpm == 0xFFFF
            && Self::window_size_from(self.state.wba1).is_none()
            && Self::window_size_from(self.state.wba2).is_some()
    }

    fn packed_indexed_position(&self, offset: u32, start: u16) -> Option<(u32, u32)> {
        let width = self.pixel_map_width();
        if width == 0 {
            return None;
        }

        let x = offset % width;
        let y = self.raster_line(start, offset / width)?;
        Some((x, y))
    }

    fn packed_direct16_position(&self, offset: u32, start: u16) -> Option<(u32, u32, u32)> {
        let width = self.pixel_map_width();
        if width == 0 {
            return None;
        }

        let pixel_offset = offset / 2;
        let x = pixel_offset % width;
        let y = self.raster_line(start, pixel_offset / width)?;
        Some((x, y, offset & 1))
    }

    fn host_window_read_packed_direct16(&self, offset: u32) -> u8 {
        let Some((x, y, component)) = self.packed_direct16_position(offset, self.state.srr) else {
            return 0xFF;
        };
        let color = self.read_packed_pixel(x, y);
        if component == 0 {
            color as u8
        } else {
            (color >> 8) as u8
        }
    }

    fn host_window_write_packed_direct16(&mut self, offset: u32, value: u8) {
        let Some((x, y, component)) = self.packed_direct16_position(offset, self.state.srw) else {
            return;
        };
        let mut color = self.read_packed_pixel(x, y);
        if component == 0 {
            color = (color & 0xFF00) | u32::from(value);
        } else {
            color = (color & 0x00FF) | (u32::from(value) << 8);
        }
        self.write_packed_pixel(x, y, color);
    }

    fn host_window_read_packed_full_color(&self, offset: u32) -> u8 {
        let Some((x, y, component)) = self.packed_full_color_position(offset, self.state.srr)
        else {
            return 0xFF;
        };
        let color = self.read_full_color_pixel(x, y);
        match component {
            0 => (color >> 16) as u8,
            1 => (color >> 8) as u8,
            _ => color as u8,
        }
    }

    fn host_window_write_packed_full_color(&mut self, offset: u32, value: u8) {
        let Some((x, y, component)) = self.packed_full_color_position(offset, self.state.srw)
        else {
            return;
        };
        let mut color = self.read_full_color_pixel(x, y);
        match component {
            0 => color = (color & 0x00FFFF) | (u32::from(value) << 16),
            1 => color = (color & 0xFF00FF) | (u32::from(value) << 8),
            _ => color = (color & 0xFFFF00) | u32::from(value),
        }
        self.write_full_color_pixel(x, y, color);
    }

    fn host_window_write_packed_indexed(&mut self, offset: u32, palette_index: u8) {
        let Some((x, y)) = self.packed_indexed_position(offset, self.state.srw) else {
            return;
        };
        self.write_indexed_pixel(x, y, palette_index);
    }

    fn host_window_read_packed_indexed(&self, offset: u32) -> u8 {
        let Some((x, y)) = self.packed_indexed_position(offset, self.state.srr) else {
            return 0xFF;
        };
        self.read_packed_pixel(x, y) as u8
    }

    fn packed_full_color_position(&self, offset: u32, start: u16) -> Option<(u32, u32, u32)> {
        let width = self.pixel_map_width();
        if width == 0 {
            return None;
        }

        let pixel_offset = offset / 3;
        let x = pixel_offset % width;
        let y = self.raster_line(start, pixel_offset / width)?;
        Some((x, y, offset % 3))
    }

    fn flat_aperture_position(
        &self,
        start: u16,
        offset: u32,
        bytes_per_pixel: u32,
    ) -> Option<(u32, u32, u32)> {
        let width = self.pixel_map_width();
        if width == 0 {
            return None;
        }

        let pixel_offset = offset / bytes_per_pixel;
        let x = pixel_offset % width;
        let y = self.raster_line(start, pixel_offset / width)?;
        Some((x, y, offset % bytes_per_pixel))
    }

    fn flat_window_size(&self) -> Option<u32> {
        if (self.state.wba1 & WBA_LOW_BYTE_SEGMENT_MASK) != 0 {
            return None;
        }
        Self::window_size_from(self.state.wba1).and_then(|size| size.bytes())
    }

    fn host_window_rotate_word(&mut self, offset: u32) {
        if offset & 1 != 0 {
            return;
        }

        let Some((line, byte_in_line)) = self.host_window_position(offset, self.state.srw) else {
            return;
        };
        let byte_in_line = byte_in_line & !1;
        let low_mask = self.state.wbm as u8;
        let high_mask = (self.state.wbm >> 8) as u8;
        if low_mask == 0 && high_mask == 0 {
            return;
        }

        let plane_mask = self.active_write_plane_mask();
        let rotate_count = u32::from(self.state.rot & 0x0F);
        for plane in 0..self.active_plane_count() {
            if plane_mask & (1u32 << plane) == 0 {
                continue;
            }

            let low = self
                .vram_read_plane_byte(plane, line, byte_in_line)
                .unwrap_or(0);
            let high = self
                .vram_read_plane_byte(plane, line, byte_in_line + 1)
                .unwrap_or(0);
            let rotated = (u16::from(low) | (u16::from(high) << 8)).rotate_left(rotate_count);

            self.vram_write_plane_byte_masked(plane, line, byte_in_line, rotated as u8, low_mask);
            self.vram_write_plane_byte_masked(
                plane,
                line,
                byte_in_line + 1,
                (rotated >> 8) as u8,
                high_mask,
            );
        }
    }

    fn host_window_write_raw(&mut self, line: u32, byte_in_line: u32, value: u8, bit_mask: u8) {
        let plane_mask = self.active_write_plane_mask();
        for plane in 0..self.active_plane_count() {
            if plane_mask & (1u32 << plane) == 0 {
                continue;
            }
            self.vram_write_plane_byte_masked(plane, line, byte_in_line, value, bit_mask);
        }
    }

    fn host_window_write_color_expand(
        &mut self,
        line: u32,
        byte_in_line: u32,
        source_bits: u8,
        bit_mask: u8,
    ) {
        // HGA*.DRV uses mode 4 after POP2=0AC8h for 1 bpp text expansion.
        let x_base = byte_in_line * 8;
        for bit_index in 0..8 {
            let bit = 0x80 >> bit_index;
            if bit_mask & bit == 0 {
                continue;
            }

            let (color, mix) = if source_bits & bit != 0 {
                (u32::from(self.state.fcol), self.state.fmix)
            } else {
                (u32::from(self.state.bcol), self.state.bmix)
            };
            self.write_pixel_rop(x_base + bit_index, line, color, mix);
        }
    }

    fn host_window_read_pixel_mask(&self, line: u32, byte_in_line: u32) -> u8 {
        let plane_mask = self.active_read_plane_mask();
        if plane_mask == 0 {
            let plane = self.state.prs & 0x0F;
            return self
                .vram_read_plane_byte(plane as usize, line, byte_in_line)
                .unwrap_or(0xFF);
        }

        let mut value = 0;
        for plane in 0..self.active_plane_count() {
            if plane_mask & (1u32 << plane) == 0 {
                continue;
            }
            value |= self
                .vram_read_plane_byte(plane, line, byte_in_line)
                .unwrap_or(0);
        }
        value
    }

    fn host_window_write_pixel_mask(&mut self, line: u32, byte_in_line: u32, pixel_mask: u8) {
        if pixel_mask == 0 {
            return;
        }
        let plane_mask = self.active_write_plane_mask();
        let color = u32::from(self.state.col);
        for plane in 0..self.active_plane_count() {
            if plane_mask & (1u32 << plane) == 0 {
                continue;
            }
            let value = if color & (1u32 << plane) != 0 {
                0xFF
            } else {
                0
            };
            self.vram_write_plane_byte_masked(plane, line, byte_in_line, value, pixel_mask);
        }
    }

    fn write_indexed_pixel(&mut self, x: u32, y: u32, palette_index: u8) {
        self.write_packed_pixel(x, y, u32::from(palette_index));
    }

    pub(super) fn active_plane_count(&self) -> usize {
        match self.state.plane_mode {
            Ga1280aPlaneMode::Indexed8 => 8,
            Ga1280aPlaneMode::DirectColor16 => 16,
            Ga1280aPlaneMode::FullColor24 => 24,
        }
    }

    pub(super) fn active_read_plane_mask(&self) -> u32 {
        match self.state.plane_mode {
            Ga1280aPlaneMode::Indexed8 => u32::from(self.state.rpe),
            Ga1280aPlaneMode::DirectColor16 => {
                u32::from(self.state.rpe) | (u32::from(self.state.rpe_high) << 8)
            }
            Ga1280aPlaneMode::FullColor24 => {
                u32::from(self.state.rpe) | (u32::from(self.state.rpe_high) << 8)
            }
        }
    }

    pub(super) fn active_write_plane_mask(&self) -> u32 {
        if self.state.prs & 0x80 != 0 {
            let plane = usize::from(self.state.prs & 0x0F);
            return if plane < self.active_plane_count() {
                1u32 << plane
            } else {
                0
            };
        }

        match self.state.plane_mode {
            Ga1280aPlaneMode::Indexed8 => u32::from(self.state.wpm & 0x00FF),
            Ga1280aPlaneMode::DirectColor16 => u32::from(self.state.wpm),
            Ga1280aPlaneMode::FullColor24 => u32::from(self.state.wpm) | 0xFF0000,
        }
    }

    fn host_bytes_per_line(&self) -> u32 {
        self.pixel_map_width().div_ceil(8)
    }

    pub(super) fn bytes_per_pixel(&self) -> u32 {
        match self.state.plane_mode {
            Ga1280aPlaneMode::Indexed8 => 1,
            Ga1280aPlaneMode::DirectColor16 => 2,
            Ga1280aPlaneMode::FullColor24 => 3,
        }
    }

    fn packed_stride(&self) -> u32 {
        self.pixel_map_width() * self.bytes_per_pixel()
    }

    fn packed_pixel_offset(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.pixel_map_width() || y >= self.pixel_map_height() {
            return None;
        }
        let bytes_per_pixel = self.bytes_per_pixel() as usize;
        let stride = self.packed_stride() as usize;
        let offset = (y as usize) * stride + (x as usize) * bytes_per_pixel;
        if offset + bytes_per_pixel > self.state.vram.len() {
            return None;
        }
        Some(offset)
    }

    pub(super) fn read_packed_pixel(&self, x: u32, y: u32) -> u32 {
        let Some(offset) = self.packed_pixel_offset(x, y) else {
            return 0;
        };
        let vram = &self.state.vram;
        match self.state.plane_mode {
            Ga1280aPlaneMode::Indexed8 => u32::from(vram[offset]),
            Ga1280aPlaneMode::DirectColor16 => {
                u32::from(vram[offset]) | (u32::from(vram[offset + 1]) << 8)
            }
            Ga1280aPlaneMode::FullColor24 => {
                u32::from(vram[offset])
                    | (u32::from(vram[offset + 1]) << 8)
                    | (u32::from(vram[offset + 2]) << 16)
            }
        }
    }

    pub(super) fn read_packed_pixel_checked(&self, x: u32, y: u32) -> Option<u32> {
        let offset = self.packed_pixel_offset(x, y)?;
        let vram = &self.state.vram;
        Some(match self.state.plane_mode {
            Ga1280aPlaneMode::Indexed8 => u32::from(vram[offset]),
            Ga1280aPlaneMode::DirectColor16 => {
                u32::from(vram[offset]) | (u32::from(vram[offset + 1]) << 8)
            }
            Ga1280aPlaneMode::FullColor24 => {
                u32::from(vram[offset])
                    | (u32::from(vram[offset + 1]) << 8)
                    | (u32::from(vram[offset + 2]) << 16)
            }
        })
    }

    pub(super) fn write_packed_pixel(&mut self, x: u32, y: u32, color: u32) {
        let Some(offset) = self.packed_pixel_offset(x, y) else {
            return;
        };
        let vram = &mut self.state.vram;
        match self.state.plane_mode {
            Ga1280aPlaneMode::Indexed8 => vram[offset] = color as u8,
            Ga1280aPlaneMode::DirectColor16 => {
                vram[offset] = color as u8;
                vram[offset + 1] = (color >> 8) as u8;
            }
            Ga1280aPlaneMode::FullColor24 => {
                vram[offset] = color as u8;
                vram[offset + 1] = (color >> 8) as u8;
                vram[offset + 2] = (color >> 16) as u8;
            }
        }
    }

    pub(super) fn pixel_map_width(&self) -> u32 {
        // PMW is inclusive and defines VRAM stride; CRTC width is only visible output.
        if self.state.plane_mode == Ga1280aPlaneMode::FullColor24 {
            return self.state.active_width;
        }
        if self.state.pmw == 0 {
            self.state.active_width
        } else {
            clamp_pixel_map_width(u32::from(self.state.pmw) + 1)
        }
    }

    pub(super) fn pixel_map_height(&self) -> u32 {
        if self.state.plane_mode == Ga1280aPlaneMode::FullColor24 {
            return self.state.active_height;
        }
        if self.state.pmh == 0 {
            self.state.active_height
        } else {
            clamp_pixel_map_height(u32::from(self.state.pmh) + 1)
        }
    }

    fn display_start(&self) -> u32 {
        u32::from(self.state.crtc_registers[CRTC_INDEX_DISPLAY_START_LOW] as u8)
            | (u32::from(self.state.crtc_registers[CRTC_INDEX_DISPLAY_START_MID] as u8) << 8)
            | (u32::from(self.state.crtc_registers[CRTC_INDEX_DISPLAY_START_HIGH] as u8) << 16)
    }

    fn display_pixels_per_crtc_unit(&self) -> u32 {
        match self.state.plane_mode {
            Ga1280aPlaneMode::Indexed8 => 4,
            Ga1280aPlaneMode::DirectColor16 => 2,
            Ga1280aPlaneMode::FullColor24 => 1,
        }
    }

    fn horizontal_pixels_per_crtc_unit(&self) -> u32 {
        if self.state.plane_mode == Ga1280aPlaneMode::FullColor24 {
            return 4;
        }
        if self.state.plane_mode == Ga1280aPlaneMode::Indexed8 {
            16
        } else {
            8
        }
    }

    pub(super) fn vram_read_plane_byte(
        &self,
        plane: usize,
        line: u32,
        byte_in_line: u32,
    ) -> Option<u8> {
        if plane >= self.active_plane_count() || line >= self.pixel_map_height() {
            return None;
        }
        let x_base = byte_in_line.checked_mul(8)?;
        if x_base >= self.pixel_map_width() {
            return None;
        }
        let plane_bit = 1u32 << plane;
        let mut result = 0u8;
        for bit_index in 0..8u32 {
            let x = x_base + bit_index;
            if x >= self.pixel_map_width() {
                break;
            }
            if self.read_packed_pixel(x, line) & plane_bit != 0 {
                result |= 0x80 >> bit_index;
            }
        }
        Some(result)
    }

    pub(super) fn vram_write_plane_byte_masked(
        &mut self,
        plane: usize,
        line: u32,
        byte_in_line: u32,
        value: u8,
        bit_mask: u8,
    ) {
        if plane >= self.active_plane_count() || line >= self.pixel_map_height() {
            return;
        }
        let Some(x_base) = byte_in_line.checked_mul(8) else {
            return;
        };
        if x_base >= self.pixel_map_width() {
            return;
        }
        let plane_bit = 1u32 << plane;
        for bit_index in 0..8u32 {
            let bit = 0x80u8 >> bit_index;
            if bit_mask & bit == 0 {
                continue;
            }
            let x = x_base + bit_index;
            if x >= self.pixel_map_width() {
                break;
            }
            let mut pixel = self.read_packed_pixel(x, line);
            if value & bit != 0 {
                pixel |= plane_bit;
            } else {
                pixel &= !plane_bit;
            }
            self.write_packed_pixel(x, line, pixel);
        }
    }

    fn read_full_color_pixel(&self, x: u32, y: u32) -> u32 {
        self.read_packed_pixel(x, y)
    }

    fn write_full_color_pixel(&mut self, x: u32, y: u32, color: u32) {
        self.write_packed_pixel(x, y, color & 0x00FF_FFFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_pixel_round_trips_for_indexed8() {
        let mut ga = Ga1280a::new();
        ga.state.pmw = 1023;
        ga.state.pmh = 1023;
        ga.state.plane_mode = Ga1280aPlaneMode::Indexed8;

        ga.write_packed_pixel(5, 7, 0b1011_0001);
        assert_eq!(ga.read_packed_pixel(5, 7), 0b1011_0001);

        for plane in 0..8 {
            let byte = ga.vram_read_plane_byte(plane, 7, 0).unwrap();
            let expected = if (0b1011_0001u8 >> plane) & 1 != 0 {
                // pixel 5 -> bit 7 - 5 = bit 2 of the gather byte.
                0x80u8 >> 5
            } else {
                0
            };
            assert_eq!(byte, expected, "plane {plane}");
        }
    }

    #[test]
    fn flat_aperture_reads_use_source_start_and_writes_use_write_start() {
        let mut ga = Ga1280a::new();
        ga.state.pmw = 1023;
        ga.state.pmh = 1023;
        ga.state.plane_mode = Ga1280aPlaneMode::Indexed8;
        ga.state.wpm = 0x00FF;
        ga.state.wbm = 0xFFFF;
        ga.state.wba1 = 0x0002;
        ga.state.srr = 10;
        ga.state.srw = 20;
        ga.write_packed_pixel(7, 10, 0x34);
        ga.write_packed_pixel(7, 20, 0x56);

        assert_eq!(ga.flat_aperture_read_byte_at_offset(7), 0x34);

        ga.flat_aperture_write_byte_at_offset(7, 0x78);

        assert_eq!(ga.read_packed_pixel(7, 10), 0x34);
        assert_eq!(ga.read_packed_pixel(7, 20), 0x78);
    }

    #[test]
    fn flat_aperture_indexed_writes_store_zero_in_pixel_mask_mode() {
        let mut ga = Ga1280a::new();
        ga.state.pmw = 1023;
        ga.state.pmh = 1023;
        ga.state.plane_mode = Ga1280aPlaneMode::Indexed8;
        ga.state.mod1 = HOST_WRITE_PIXEL_MASK_MODE;
        ga.state.wpm = 0xFFFF;
        ga.state.wbm = 0xFFFF;
        ga.state.wba1 = 0;
        ga.state.wba2 = 0x3F01;
        ga.state.srw = 552;
        ga.write_packed_pixel(22, 552, 0xFF);

        ga.flat_aperture_write_byte_at_offset(22, 0);

        assert_eq!(ga.read_packed_pixel(22, 552), 0);
    }

    #[test]
    fn plane_byte_scatter_round_trips_for_indexed8() {
        let mut ga = Ga1280a::new();
        ga.state.pmw = 1023;
        ga.state.pmh = 1023;
        ga.state.plane_mode = Ga1280aPlaneMode::Indexed8;

        // Plane 3, byte 0 of line 0: bit pattern 0b1010_0011.
        ga.vram_write_plane_byte_masked(3, 0, 0, 0b1010_0011, 0xFF);

        let plane_bit = 1u32 << 3;
        for x in 0..8u32 {
            let pixel = ga.read_packed_pixel(x, 0);
            let expected_bit = (0b1010_0011u8 >> (7 - x)) & 1 != 0;
            assert_eq!(
                pixel & plane_bit != 0,
                expected_bit,
                "pixel {x} plane-3 bit"
            );
        }

        // Round-trip via the gather: should reproduce the original byte.
        assert_eq!(ga.vram_read_plane_byte(3, 0, 0), Some(0b1010_0011));
    }

    #[test]
    fn packed_pixel_round_trips_for_direct_color16() {
        let mut ga = Ga1280a::new();
        ga.state.pmw = 511;
        ga.state.pmh = 511;
        ga.state.plane_mode = Ga1280aPlaneMode::DirectColor16;

        ga.write_packed_pixel(3, 9, 0xABCD);
        assert_eq!(ga.read_packed_pixel(3, 9), 0xABCD);

        for plane in 0..16 {
            let bit_set = (0xABCDu32 >> plane) & 1 != 0;
            let byte = ga.vram_read_plane_byte(plane, 9, 0).unwrap();
            let expected = if bit_set { 0x80u8 >> 3 } else { 0 };
            assert_eq!(byte, expected, "plane {plane}");
        }
    }

    #[test]
    fn packed_direct16_host_path_round_trips_per_pixel() {
        let mut ga = Ga1280a::new();
        // Switch to 16-bpp and configure the GALIB-style packed-window: wba1 size nibble
        // clear, wba2 carries the size, wpm/wbm fully enabled, mod1 = 0.
        ga.state.plane_mode = Ga1280aPlaneMode::DirectColor16;
        ga.state.pmw = 639;
        ga.state.pmh = 479;
        ga.state.mod1 = 0;
        ga.state.wbm = 0xFFFF;
        ga.state.wpm = 0xFFFF;
        ga.state.wba1 = 0x00DC;
        ga.state.wba2 = 0x30DC;
        ga.state.srw = 0;
        ga.state.srr = 0;

        assert!(ga.uses_packed_direct16_host_pixels());

        // Write the two halves of pixel (5, 3) = 0xABCD via the host window.
        let pixel_offset = (3 * 640 + 5) * 2;
        ga.host_window_write(pixel_offset, 0xCD);
        ga.host_window_write(pixel_offset + 1, 0xAB);

        // Confirm reads through the host window see the same bytes.
        assert_eq!(ga.host_window_read(pixel_offset), 0xCD);
        assert_eq!(ga.host_window_read(pixel_offset + 1), 0xAB);

        // And the underlying packed pixel matches.
        assert_eq!(ga.read_packed_pixel(5, 3), 0xABCD);
    }

    #[test]
    fn plane_byte_scatter_masks_unaffected_bits() {
        let mut ga = Ga1280a::new();
        ga.state.pmw = 1023;
        ga.state.pmh = 1023;
        ga.state.plane_mode = Ga1280aPlaneMode::Indexed8;

        // Pre-seed plane 0 of pixels (0..8, 0) by writing palette index 1 (plane 0 bit set).
        for x in 0..8 {
            ga.write_packed_pixel(x, 0, 0x01);
        }

        // Write plane 0 byte 0xF0 with mask 0xF0: only the high 4 bits change.
        ga.vram_write_plane_byte_masked(0, 0, 0, 0xF0, 0xF0);

        // High 4 pixels (x = 0..4) keep plane-0 bit because value bit is 1 and mask is 1.
        for x in 0..4 {
            assert_eq!(ga.read_packed_pixel(x, 0) & 1, 1, "x={x} kept");
        }
        // Low 4 pixels (x = 4..8) are NOT touched by the mask, keeping their original bit.
        for x in 4..8 {
            assert_eq!(ga.read_packed_pixel(x, 0) & 1, 1, "x={x} untouched");
        }

        // Now clear the high 4 with value = 0x00, mask 0xF0.
        ga.vram_write_plane_byte_masked(0, 0, 0, 0x00, 0xF0);
        for x in 0..4 {
            assert_eq!(ga.read_packed_pixel(x, 0) & 1, 0, "x={x} cleared");
        }
        for x in 4..8 {
            assert_eq!(ga.read_packed_pixel(x, 0) & 1, 1, "x={x} masked-out");
        }
    }
}
