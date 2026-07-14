//! CRTC, palette, video-controller, and text-VRAM access paths.
//!
//! Operation-port bit 3 is a level switch: while it stays set, the raster
//! copy selected by R21 and R22 executes at every horizontal front porch.

use common::{M68000BusError, TraceContext, TraceEvent, TracePresentation, TraceSink, trace_id};
use device::{
    crtc_x68k::{CrtcBeamPositionX68k, CrtcGeometryX68k, CrtcScanClassX68k, GvramModeX68k},
    video_controller_x68k::PaletteX68k,
};
use software_renderer::x68k::{RenderInputsX68k, ScanModeX68k};

use super::{X68kBus, X68kRegion};

/// Index of the CRTC horizontal back-porch end register R2.
const CRTC_HORIZONTAL_BACK_END_REGISTER: usize = 2;
/// Index of the CRTC vertical back-porch end register R6.
const CRTC_VERTICAL_BACK_END_REGISTER: usize = 6;
/// Index of the CRTC memory-mode register R20.
const CRTC_MEMORY_MODE_REGISTER: usize = 20;
/// Index of the CRTC access-control register R21.
const CRTC_ACCESS_CONTROL_REGISTER: usize = 21;
/// Index of the video-controller memory-mode register R0.
const VIDEO_MEMORY_MODE_REGISTER: usize = 0;
/// Index of the video-controller priority register R1.
const VIDEO_PRIORITY_REGISTER: usize = 1;
/// Index of the video-controller mixing register R2.
const VIDEO_MIXING_REGISTER: usize = 2;
/// Index of the CRTC horizontal total register R0.
const CRTC_HORIZONTAL_TOTAL_REGISTER: usize = 0;
/// Index of the CRTC vertical total register R4.
const CRTC_VERTICAL_TOTAL_REGISTER: usize = 4;

/// Standard CRTMOD horizontal display for a horizontal total (R00 + 1).
///
/// Games shrink and shift their display window by moving the back-porch end
/// (R02) and display end (R03) while keeping the horizontal total at a standard
/// CRTMOD preset. Matching the total recovers the full display width the monitor
/// shows plus the display-start column (R02 + 5) of the standard centered
/// window, so a smaller window is placed at its true position instead of being
/// anchored to a corner. Non-preset totals return `None` and leave the window
/// unframed.
const fn standard_display_width(horizontal_total: u16) -> Option<(u32, u16)> {
    match horizontal_total {
        38 => Some((256, 5)),
        46 => Some((256, 11)),
        76 => Some((512, 10)),
        92 => Some((512, 22)),
        100 => Some((640, 18)),
        138 => Some((768, 33)),
        176 => Some((1024, 36)),
        _ => None,
    }
}

/// Standard CRTMOD vertical display for a vertical total (R04 + 1).
///
/// See [`standard_display_width`]; returns the standard display height in raster
/// units plus the first visible raster (R06 + 1) of the standard window.
const fn standard_display_height(vertical_total: u16) -> Option<(u32, u16)> {
    match vertical_total {
        260 => Some((240, 17)),
        465 => Some((424, 33)),
        525 => Some((480, 34)),
        568 => Some((512, 41)),
        _ => None,
    }
}

impl<T: TraceSink> X68kBus<T> {
    /// Reads a native 16-bit video or CRTC register.
    pub(super) fn read_device_word(
        &mut self,
        address: u32,
        region: X68kRegion,
    ) -> Result<u16, M68000BusError> {
        self.synchronize_devices();
        match region {
            X68kRegion::Crtc => {
                let offset = (address - 0xE80000) & 0x07FF;
                if offset < 0x30 {
                    Ok(self.crtc.read_register((offset >> 1) as usize))
                } else if offset >= 0x480 {
                    Ok(self.crtc.read_operation())
                } else {
                    // CRTC hole
                    Ok(0xFFFF)
                }
            }
            X68kRegion::Palette => {
                let palette = if address & 0x0200 == 0 {
                    PaletteX68k::Graphics
                } else {
                    PaletteX68k::Text
                };
                Ok(self
                    .video_controller
                    .read_palette(palette, ((address & 0x01FF) >> 1) as usize))
            }
            X68kRegion::VideoController => {
                let register = ((address - 0xE82400) >> 8) as usize;
                if register < 3 {
                    Ok(self.video_controller.read_register(register))
                } else {
                    Ok(0xFFFF)
                }
            }
            X68kRegion::Sprite => Ok(self.sprite.read_word(address)),
            _ => unreachable!(),
        }
    }

    /// Writes a native 16-bit video or CRTC register.
    pub(super) fn write_device_word(
        &mut self,
        address: u32,
        region: X68kRegion,
        value: u16,
    ) -> Result<(), M68000BusError> {
        self.synchronize_devices();
        self.catch_up_video();
        match region {
            X68kRegion::Crtc => {
                let offset = (address - 0xE80000) & 0x07FF;
                if offset < 0x30 {
                    let change = self.crtc.write_register((offset >> 1) as usize, value);
                    if change.clock {
                        self.crtc_remainder = 0;
                    }
                } else if offset >= 0x480 {
                    self.crtc.write_operation(value);
                }
            }
            X68kRegion::Palette => {
                let palette = if address & 0x0200 == 0 {
                    PaletteX68k::Graphics
                } else {
                    PaletteX68k::Text
                };
                self.video_controller.write_palette(
                    palette,
                    ((address & 0x01FF) >> 1) as usize,
                    value,
                );
            }
            X68kRegion::VideoController => {
                let register = ((address - 0xE82400) >> 8) as usize;
                if register < 3 {
                    self.video_controller.write_register(register, value);
                }
            }
            X68kRegion::Sprite => self.sprite.write_word(address, value),
            _ => unreachable!(),
        }
        self.update_device_pins();
        self.schedule_events();
        Ok(())
    }

    /// Copies the R22 source raster block onto its destination block for
    /// every text plane selected by R21.
    pub(super) fn execute_raster_copy(&mut self) {
        let access = self.crtc.read_register(21) as u8;
        let blocks = self.crtc.read_register(22);
        let source = usize::from(blocks >> 8) * 512;
        let destination = usize::from(blocks as u8) * 512;
        for plane in 0..4 {
            if access & (1 << plane) == 0 {
                continue;
            }
            let base = plane * 0x20_000;
            self.text_vram
                .copy_within(base + source..base + source + 512, base + destination);
        }
    }

    /// Clears the visible graphics area from the page-0 scroll origin.
    ///
    /// R21 bits 3-0 select the cleared 4-bit pages; the 1024x1024 mode
    /// ignores the selection and clears the two pages backing the row's
    /// vertical half. Hardware clears raster by raster over the display
    /// frame; this model clears the whole area at display start.
    pub(super) fn execute_high_speed_clear(&mut self) {
        let Some(geometry) = self.crtc.frame_geometry() else {
            return;
        };
        let mut start_x = usize::from(self.crtc.graphic_scroll_x(0)) & 510;
        let mut width = geometry.width as usize;
        if width >= 512 {
            start_x = 0;
            width = 512;
        }
        let scroll_y = usize::from(self.crtc.graphic_scroll_y(0));
        let page_mask = self.crtc.read_register(CRTC_ACCESS_CONTROL_REGISTER) & 0x000F;
        let mut page_keep_mask: u16 = 0;
        for page in 0..4 {
            if page_mask & (1 << page) == 0 {
                page_keep_mask |= 0xF << (page * 4);
            }
        }
        let virtual_1024 = match self.crtc.graphic_memory_mode() {
            GvramModeX68k::Colors16
            | GvramModeX68k::Colors256
            | GvramModeX68k::MemoryMode2
            | GvramModeX68k::Colors65536 => false,
            GvramModeX68k::Colors16Virtual1024 => true,
        };
        let content_height = match self.crtc.scan_class() {
            CrtcScanClassX68k::Normal => geometry.height as usize,
            CrtcScanClassX68k::DoubleRead => geometry.height as usize / 2,
            CrtcScanClassX68k::Interlace => geometry.height as usize * 2,
        };
        for row in 0..content_height {
            let vertical = scroll_y + row;
            let keep_mask = if !virtual_1024 {
                page_keep_mask
            } else if vertical & 512 == 0 {
                0xFF00
            } else {
                0x00FF
            };
            let row_offset = (vertical & 511) * 512;
            for column in 0..width {
                self.graphic_vram[row_offset + ((start_x + column) & 511)] &= keep_mask;
            }
        }
    }

    /// Applies simultaneous-plane and bit-mask rules to one TVRAM byte.
    pub(super) fn write_text_vram_byte(&mut self, address: u32, value: u8) {
        let relative = (address - 0xE00000) as usize;
        let access = self.crtc.read_register(21);
        let bit_mask_enabled = access & 0x0200 != 0;
        let simultaneous_access = access & 0x0100 != 0;
        let mask = if relative & 1 == 0 {
            (self.crtc.read_register(23) >> 8) as u8
        } else {
            self.crtc.read_register(23) as u8
        };
        let write = |storage: &mut [u8], offset: usize| {
            storage[offset] = if bit_mask_enabled {
                storage[offset] & mask | value & !mask
            } else {
                value
            };
        };
        if simultaneous_access {
            let offset = relative & 0x1_FFFF;
            for plane in 0..4 {
                if access & (0x10 << plane) != 0 {
                    write(&mut self.text_vram, plane * 0x20_000 + offset);
                }
            }
        } else {
            write(&mut self.text_vram, relative);
        }
    }

    /// Maps the CRTC scan class to the renderer scan mode.
    fn renderer_scan_mode(&self) -> ScanModeX68k {
        match self.crtc.scan_class() {
            CrtcScanClassX68k::Normal => ScanModeX68k::Progressive,
            CrtcScanClassX68k::DoubleRead => ScanModeX68k::DoubleRead,
            CrtcScanClassX68k::Interlace => ScanModeX68k::Interlace,
        }
    }

    /// Positions the display window inside the standard reference frame.
    ///
    /// When the horizontal or vertical total matches a CRTMOD preset, the window
    /// is placed at the offset its back porch selects: the difference between the
    /// current display-start column (raster) and the standard preset's, scaled to
    /// pixels. A window centered by equal back and front porches then renders
    /// centered, and a full-width window renders at the origin. Non-preset totals
    /// leave the window unframed at the origin.
    fn configure_reference_frame(&mut self, geometry: CrtcGeometryX68k) {
        let horizontal_total = self.crtc.read_register(CRTC_HORIZONTAL_TOTAL_REGISTER) + 1;
        let vertical_total = self.crtc.read_register(CRTC_VERTICAL_TOTAL_REGISTER) + 1;
        let (frame_width, offset_x) = match standard_display_width(horizontal_total) {
            Some((width, reference_first_column)) => {
                let offset =
                    u32::from(geometry.first_column.saturating_sub(reference_first_column)) * 8;
                (width.max(offset + geometry.width), offset)
            }
            None => (geometry.width, 0),
        };
        let (frame_height, offset_y) = match standard_display_height(vertical_total) {
            Some((height, reference_first_raster)) => {
                let offset =
                    u32::from(geometry.first_raster.saturating_sub(reference_first_raster));
                (height.max(offset + geometry.height), offset)
            }
            None => (geometry.height, 0),
        };
        self.renderer
            .configure_frame(offset_x, offset_y, frame_width, frame_height);
    }

    /// Brings scanout up to the current CRTC beam.
    pub(super) fn catch_up_video(&mut self) {
        let Some(geometry) = self.crtc.frame_geometry() else {
            return;
        };
        self.renderer
            .ensure_geometry(geometry.width, geometry.height, self.renderer_scan_mode());
        self.configure_reference_frame(geometry);
        let target = beam_pixel(geometry, self.crtc.beam_position());
        let inputs = RenderInputsX68k {
            text_vram: &self.text_vram,
            graphic_vram: &self.graphic_vram,
            text_palette: self.video_controller.text_palette(),
            graphics_palette: self.video_controller.graphics_palette(),
            text_scroll_x: self.crtc.text_scroll_x(),
            text_scroll_y: self.crtc.text_scroll_y(),
            graphic_scroll_x: [
                self.crtc.graphic_scroll_x(0),
                self.crtc.graphic_scroll_x(1),
                self.crtc.graphic_scroll_x(2),
                self.crtc.graphic_scroll_x(3),
            ],
            graphic_scroll_y: [
                self.crtc.graphic_scroll_y(0),
                self.crtc.graphic_scroll_y(1),
                self.crtc.graphic_scroll_y(2),
                self.crtc.graphic_scroll_y(3),
            ],
            crtc_memory_mode: self.crtc.read_register(CRTC_MEMORY_MODE_REGISTER),
            memory_mode: self
                .video_controller
                .read_register(VIDEO_MEMORY_MODE_REGISTER),
            priority: self.video_controller.read_register(VIDEO_PRIORITY_REGISTER),
            mixing: self.video_controller.read_register(VIDEO_MIXING_REGISTER),
            sprite_scroll: self.sprite.scroll_data(),
            sprite_pattern: self.sprite.pattern_data(),
            background_scroll: [
                self.sprite.background_scroll_x(0),
                self.sprite.background_scroll_y(0),
                self.sprite.background_scroll_x(1),
                self.sprite.background_scroll_y(1),
            ],
            background_control: self.sprite.background_control(),
            sprite_resolution: self.sprite.resolution(),
            sprite_horizontal_back_end: self.sprite.horizontal_back_end(),
            sprite_vertical_back_end: self.sprite.vertical_back_end(),
            crtc_horizontal_back_end: self.crtc.read_register(CRTC_HORIZONTAL_BACK_END_REGISTER),
            crtc_vertical_back_end: self.crtc.read_register(CRTC_VERTICAL_BACK_END_REGISTER),
            sprite_area_accessible: self.crtc.sprite_area_accessible(),
            contrast: self.contrast,
            width: geometry.width,
            height: geometry.height,
            odd_field: self.crtc.beam_position().odd_field,
        };
        self.renderer.catch_up(&inputs, target);
    }

    /// Completes and publishes the frame the beam just finished.
    pub(super) fn publish_video_frame(&mut self) {
        let Some(geometry) = self.crtc.frame_geometry() else {
            return;
        };
        self.renderer
            .ensure_geometry(geometry.width, geometry.height, self.renderer_scan_mode());
        self.configure_reference_frame(geometry);
        let inputs = RenderInputsX68k {
            text_vram: &self.text_vram,
            graphic_vram: &self.graphic_vram,
            text_palette: self.video_controller.text_palette(),
            graphics_palette: self.video_controller.graphics_palette(),
            text_scroll_x: self.crtc.text_scroll_x(),
            text_scroll_y: self.crtc.text_scroll_y(),
            graphic_scroll_x: [
                self.crtc.graphic_scroll_x(0),
                self.crtc.graphic_scroll_x(1),
                self.crtc.graphic_scroll_x(2),
                self.crtc.graphic_scroll_x(3),
            ],
            graphic_scroll_y: [
                self.crtc.graphic_scroll_y(0),
                self.crtc.graphic_scroll_y(1),
                self.crtc.graphic_scroll_y(2),
                self.crtc.graphic_scroll_y(3),
            ],
            crtc_memory_mode: self.crtc.read_register(CRTC_MEMORY_MODE_REGISTER),
            memory_mode: self
                .video_controller
                .read_register(VIDEO_MEMORY_MODE_REGISTER),
            priority: self.video_controller.read_register(VIDEO_PRIORITY_REGISTER),
            mixing: self.video_controller.read_register(VIDEO_MIXING_REGISTER),
            sprite_scroll: self.sprite.scroll_data(),
            sprite_pattern: self.sprite.pattern_data(),
            background_scroll: [
                self.sprite.background_scroll_x(0),
                self.sprite.background_scroll_y(0),
                self.sprite.background_scroll_x(1),
                self.sprite.background_scroll_y(1),
            ],
            background_control: self.sprite.background_control(),
            sprite_resolution: self.sprite.resolution(),
            sprite_horizontal_back_end: self.sprite.horizontal_back_end(),
            sprite_vertical_back_end: self.sprite.vertical_back_end(),
            crtc_horizontal_back_end: self.crtc.read_register(CRTC_HORIZONTAL_BACK_END_REGISTER),
            crtc_vertical_back_end: self.crtc.read_register(CRTC_VERTICAL_BACK_END_REGISTER),
            sprite_area_accessible: self.crtc.sprite_area_accessible(),
            contrast: self.contrast,
            width: geometry.width,
            height: geometry.height,
            odd_field: self.crtc.beam_position().odd_field,
        };
        self.renderer.publish_frame(&inputs);
        if T::ENABLED {
            let (width, height) = self.renderer.dimensions();
            self.tracer.trace(
                TraceContext::presentation_main(self.current_cycle, Some(self.cpu_clock_hz)),
                TraceEvent::Presentation(TracePresentation {
                    display: trace_id::display::MAIN,
                    frame: self.crtc.frame_count(),
                    width,
                    height,
                }),
            );
        }
    }
}

/// Converts a CRTC beam position to the number of completed visible pixels.
fn beam_pixel(geometry: CrtcGeometryX68k, beam: CrtcBeamPositionX68k) -> usize {
    if beam.raster < geometry.first_raster {
        return 0;
    }
    let screen_y = u32::from(beam.raster - geometry.first_raster);
    if screen_y >= geometry.height {
        return geometry.width as usize * geometry.height as usize;
    }
    let screen_x = if beam.column < geometry.first_column {
        0
    } else {
        (u32::from(beam.column - geometry.first_column) * 8 + u32::from(beam.dot))
            .min(geometry.width)
    };
    (screen_y * geometry.width + screen_x) as usize
}

#[cfg(test)]
mod tests {
    use common::{Bus, M68000AccessSize, M68000FunctionCode};
    use device::{
        crtc_x68k::{CrtcBeamPositionX68k, CrtcGeometryX68k},
        video_controller_x68k::PaletteX68k,
    };

    use super::{CRTC_HORIZONTAL_TOTAL_REGISTER, CRTC_VERTICAL_TOTAL_REGISTER, beam_pixel};
    use crate::{
        X68kModel,
        bus::{
            X68kBus,
            test_support::{
                access, advance_to_raster, bus, complete_frame, read_word, tiny_display, write_word,
            },
        },
    };

    /// Returns one published framebuffer pixel of a 16-pixel-wide display.
    fn pixel(bus: &X68kBus, x: usize, y: usize) -> [u8; 4] {
        let offset = (y * 16 + x) * 4;
        bus.display_framebuffer()[offset..offset + 4]
            .try_into()
            .unwrap()
    }

    /// Opaque white at full contrast.
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    /// Opaque black.
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    #[test]
    fn standard_presets_map_totals_to_the_full_display_size() {
        // Horizontal total + 1 selects the CRTMOD display width and start column.
        assert_eq!(super::standard_display_width(138), Some((768, 33)));
        assert_eq!(super::standard_display_width(92), Some((512, 22)));
        assert_eq!(super::standard_display_width(100), Some((640, 18)));
        assert_eq!(super::standard_display_width(46), Some((256, 11)));
        assert_eq!(super::standard_display_width(139), None);
        // Vertical total + 1 selects the CRTMOD display height and first raster.
        assert_eq!(super::standard_display_height(568), Some((512, 41)));
        assert_eq!(super::standard_display_height(525), Some((480, 34)));
        assert_eq!(super::standard_display_height(465), Some((424, 33)));
        assert_eq!(super::standard_display_height(569), None);
    }

    /// A 15 kHz window (CRTMOD $05) by enlarging the back porch renders
    /// centered in the 512x240 reference frame. Its display start
    /// (R02 = 0x0D, R06 = 0x1C) sits 8 columns and12 rasters past the
    /// standard $05 start (R02 = 5, R06 = 16).
    ///
    /// This was needed to render Daimakaimura correctly.
    #[test]
    fn centered_window_is_positioned_by_its_back_porch() {
        let geometry = CrtcGeometryX68k {
            width: 384,
            height: 224,
            first_raster: 0x1C + 1,
            first_column: 0x0D + 5,
        };
        let mut bus = bus(X68kModel::X68000);
        // CRTMOD $05: horizontal total 76, vertical total 260.
        bus.crtc.write_register(CRTC_HORIZONTAL_TOTAL_REGISTER, 75);
        bus.crtc.write_register(CRTC_VERTICAL_TOTAL_REGISTER, 259);
        bus.configure_reference_frame(geometry);
        assert_eq!(bus.renderer.frame_offset(), (64, 12));
        assert_eq!(bus.display_dimensions(), (512, 240));
    }

    #[test]
    fn text_vram_planes_render_through_the_text_palette() {
        let mut bus = bus(X68kModel::X68000);
        for (index, value) in [10, 0, 0, 2, 3, 0, 0, 1, 0, 3].into_iter().enumerate() {
            bus.crtc.write_register(index, value);
        }
        bus.video_controller
            .write_palette(PaletteX68k::Text, 1, 0xFFFF);
        bus.video_controller.write_register(2, 0x20);
        bus.contrast = 15;
        bus.text_vram[0] = 0x80;
        bus.set_current_cycle(2_000);
        bus.synchronize_devices();
        assert_eq!(bus.display_dimensions(), (16, 1));
        assert_eq!(&bus.display_framebuffer()[..4], &[255, 255, 255, 255]);
        assert_eq!(&bus.display_framebuffer()[4..8], &[0, 0, 0, 255]);
    }

    #[test]
    fn graphic_vram_renders_through_the_graphics_palette() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        for (index, value) in [10, 0, 0, 2, 3, 0, 0, 1, 0, 3].into_iter().enumerate() {
            bus.crtc.write_register(index, value);
        }
        bus.video_controller
            .write_palette(PaletteX68k::Graphics, 1, 0xFFFF);
        bus.video_controller.write_register(1, 0x00E4);
        bus.video_controller.write_register(2, 0x0001);
        bus.contrast = 15;
        bus.m68000_write(access(0xC00000, M68000AccessSize::Word, supervisor), 0x0001)
            .unwrap();
        bus.set_current_cycle(2_000);
        bus.synchronize_devices();
        assert_eq!(bus.display_dimensions(), (16, 1));
        assert_eq!(&bus.display_framebuffer()[..4], &[255, 255, 255, 255]);
        assert_eq!(&bus.display_framebuffer()[4..8], &[0, 0, 0, 255]);
    }

    #[test]
    fn graphics_show_behind_transparent_text_pixels() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        for (index, value) in [10, 0, 0, 2, 3, 0, 0, 1, 0, 3].into_iter().enumerate() {
            bus.crtc.write_register(index, value);
        }
        bus.video_controller
            .write_palette(PaletteX68k::Graphics, 1, 0x07C0);
        bus.video_controller
            .write_palette(PaletteX68k::Text, 1, 0xF800);
        bus.video_controller.write_register(1, 0x02E4);
        bus.video_controller.write_register(2, 0x0021);
        bus.contrast = 15;
        bus.text_vram[0] = 0x80;
        bus.m68000_write(access(0xC00000, M68000AccessSize::Word, supervisor), 0x0001)
            .unwrap();
        bus.m68000_write(access(0xC00002, M68000AccessSize::Word, supervisor), 0x0001)
            .unwrap();
        bus.set_current_cycle(2_000);
        bus.synchronize_devices();
        let framebuffer = bus.display_framebuffer();
        assert_eq!(&framebuffer[..2], &[0, 251]);
        assert_eq!(&framebuffer[4..6], &[251, 0]);
    }

    #[test]
    fn sprite_screen_renders_through_the_bus() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        for (index, value) in [10, 0, 0, 2, 3, 0, 0, 1, 0, 3].into_iter().enumerate() {
            bus.crtc.write_register(index, value);
        }
        bus.video_controller.write_register(1, 0x06E4);
        bus.video_controller.write_register(2, 0x0040);
        bus.video_controller
            .write_palette(PaletteX68k::Text, 0x31, 0xFFFF);
        bus.contrast = 15;
        let words = [
            (0xEB080C, 0x0004),
            (0xEB0808, 0x0200),
            (0xEB8080, 0x1000),
            (0xEB0000, 0x0010),
            (0xEB0002, 0x0010),
            (0xEB0004, 0x0301),
            (0xEB0006, 0x0003),
        ];
        for (address, value) in words {
            bus.m68000_write(access(address, M68000AccessSize::Word, supervisor), value)
                .unwrap();
        }
        bus.set_current_cycle(2_000);
        bus.synchronize_devices();
        assert_eq!(bus.display_dimensions(), (16, 1));
        assert_eq!(&bus.display_framebuffer()[..4], &[255, 255, 255, 255]);
        assert_eq!(&bus.display_framebuffer()[4..8], &[0, 0, 0, 255]);
    }

    #[test]
    fn palette_byte_lanes_update_half_an_entry() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        bus.m68000_write(access(0xE82200, M68000AccessSize::Word, supervisor), 0x1234)
            .unwrap();
        bus.m68000_write(access(0xE82200, M68000AccessSize::Byte, supervisor), 0x00AB)
            .unwrap();
        bus.m68000_write(access(0xE82201, M68000AccessSize::Byte, supervisor), 0x00CD)
            .unwrap();
        assert_eq!(
            bus.m68000_read(access(0xE82200, M68000AccessSize::Word, supervisor))
                .unwrap(),
            0xABCD
        );
        assert_eq!(
            bus.video_controller.read_palette(PaletteX68k::Text, 0),
            0xABCD
        );
    }

    /// Programs a minimal timing so front porches occur while advancing.
    fn program_copy_display(bus: &mut X68kBus) {
        for (index, value) in [10, 0, 0, 2, 3, 0, 0, 1, 0, 3].into_iter().enumerate() {
            bus.crtc.write_register(index, value);
        }
    }

    #[test]
    fn raster_copy_obeys_the_selected_text_planes() {
        let mut bus = bus(X68kModel::X68000);
        program_copy_display(&mut bus);
        bus.text_vram[512..1024].fill(0x5A);
        bus.text_vram[0x20_000 + 512..0x20_000 + 1024].fill(0xA5);
        bus.crtc.write_register(21, 0x0001);
        bus.crtc.write_register(22, 0x0102);
        bus.crtc.write_operation(0x0008);
        advance_to_raster(&mut bus, 1);
        assert!(bus.text_vram[1024..1536].iter().all(|&value| value == 0x5A));
        assert!(
            bus.text_vram[0x20_000 + 1024..0x20_000 + 1536]
                .iter()
                .all(|&value| value == 0)
        );
    }

    #[test]
    fn raster_copy_covers_all_planes_and_none() {
        let mut bus = bus(X68kModel::X68000);
        program_copy_display(&mut bus);
        for plane in 0..4 {
            bus.text_vram[plane * 0x20_000 + 512..plane * 0x20_000 + 1024]
                .fill(0x11 * (plane as u8 + 1));
        }
        bus.crtc.write_register(21, 0x000F);
        bus.crtc.write_register(22, 0x0102);
        bus.crtc.write_operation(0x0008);
        advance_to_raster(&mut bus, 1);
        for plane in 0..4 {
            assert!(
                bus.text_vram[plane * 0x20_000 + 1024..plane * 0x20_000 + 1536]
                    .iter()
                    .all(|&value| value == 0x11 * (plane as u8 + 1))
            );
        }
        bus.crtc.write_register(21, 0x0000);
        bus.crtc.write_register(22, 0x0103);
        advance_to_raster(&mut bus, 2);
        for plane in 0..4 {
            assert!(
                bus.text_vram[plane * 0x20_000 + 1536..plane * 0x20_000 + 2048]
                    .iter()
                    .all(|&value| value == 0)
            );
        }
    }

    #[test]
    fn raster_copy_repeats_each_front_porch_until_cleared() {
        let mut bus = bus(X68kModel::X68000);
        program_copy_display(&mut bus);
        bus.text_vram[512..1024].fill(0x5A);
        bus.crtc.write_register(21, 0x0001);
        bus.crtc.write_register(22, 0x0102);
        bus.crtc.write_operation(0x0008);

        // The switch reads back while held and the copy repeats each raster
        // with the current R22, following the classic scroll pattern.
        assert_eq!(bus.crtc.read_operation() & 0x0008, 0x0008);
        advance_to_raster(&mut bus, 1);
        assert!(bus.text_vram[1024..1536].iter().all(|&value| value == 0x5A));
        bus.crtc.write_register(22, 0x0203);
        advance_to_raster(&mut bus, 2);
        assert!(bus.text_vram[1536..2048].iter().all(|&value| value == 0x5A));

        // Clearing bit 3 stops the repetition.
        bus.crtc.write_operation(0x0000);
        bus.crtc.write_register(22, 0x0304);
        advance_to_raster(&mut bus, 3);
        assert!(bus.text_vram[2048..2560].iter().all(|&value| value == 0));
    }

    #[test]
    fn word_text_vram_writes_mask_both_byte_lanes() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        bus.crtc.write_register(21, 0x0200);
        bus.crtc.write_register(23, 0xF00F);
        bus.text_vram[0] = 0xAA;
        bus.text_vram[1] = 0x55;
        bus.m68000_write(access(0xE00000, M68000AccessSize::Word, supervisor), 0x3C3C)
            .unwrap();
        assert_eq!(bus.text_vram[0], 0xAC);
        assert_eq!(bus.text_vram[1], 0x35);
    }

    #[test]
    fn word_simultaneous_writes_reach_the_selected_planes() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        bus.crtc.write_register(21, 0x0150);
        bus.m68000_write(access(0xE20000, M68000AccessSize::Word, supervisor), 0x1234)
            .unwrap();
        assert_eq!(bus.text_vram[0], 0x12);
        assert_eq!(bus.text_vram[1], 0x34);
        assert_eq!(bus.text_vram[0x20_000], 0);
        assert_eq!(bus.text_vram[0x40_000], 0x12);
        assert_eq!(bus.text_vram[0x40_001], 0x34);
        assert_eq!(bus.text_vram[0x60_000], 0);
    }

    #[test]
    fn text_storage_mode_blanks_the_text_scanout() {
        let mut bus = bus(X68kModel::X68000);
        for (index, value) in [10, 0, 0, 2, 3, 0, 0, 1, 0, 3].into_iter().enumerate() {
            bus.crtc.write_register(index, value);
        }
        bus.video_controller
            .write_palette(PaletteX68k::Text, 1, 0xFFFF);
        bus.video_controller.write_register(2, 0x20);
        bus.contrast = 15;
        bus.text_vram[0] = 0x80;
        bus.crtc.write_register(20, 0x1000);
        bus.set_current_cycle(2_000);
        bus.synchronize_devices();
        assert_eq!(&bus.display_framebuffer()[..4], &[0, 0, 0, 255]);
    }

    #[test]
    fn mid_frame_raster_copy_splits_the_frame_at_the_beam() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        for (index, value) in [10, 0, 0, 2, 4, 0, 0, 3, 0, 5].into_iter().enumerate() {
            bus.crtc.write_register(index, value);
        }
        bus.video_controller
            .write_palette(PaletteX68k::Text, 1, 0xFFFF);
        bus.video_controller.write_register(2, 0x20);
        bus.contrast = 15;
        bus.text_vram[512..1024].fill(0xFF);
        bus.crtc.write_register(21, 0x0001);
        bus.crtc.write_register(22, 0x0100);
        advance_to_raster(&mut bus, 2);
        bus.m68000_write(access(0xE80480, M68000AccessSize::Word, supervisor), 0x0008)
            .unwrap();
        advance_to_raster(&mut bus, 0);
        // The copy landed at raster 2's front porch: rows drawn up to and
        // including raster 2 keep the old text VRAM, the next row is copied.
        let framebuffer = bus.display_framebuffer();
        assert_eq!(&framebuffer[..4], &[0, 0, 0, 255]);
        assert_eq!(&framebuffer[16 * 4..16 * 4 + 4], &[0, 0, 0, 255]);
        assert_eq!(
            &framebuffer[2 * 16 * 4..2 * 16 * 4 + 4],
            &[255, 255, 255, 255]
        );
    }

    #[test]
    fn text_vram_simultaneous_access_and_mask_apply_per_plane() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        bus.crtc.write_register(21, 0x0350);
        bus.crtc.write_register(23, 0xF00F);
        bus.text_vram[0] = 0xAA;
        bus.text_vram[0x40_000] = 0x55;
        bus.m68000_write(access(0xE00000, M68000AccessSize::Byte, supervisor), 0x3C)
            .unwrap();
        assert_eq!(bus.text_vram[0], 0xAC);
        assert_eq!(bus.text_vram[0x20_000], 0);
        assert_eq!(bus.text_vram[0x40_000], 0x5C);
        assert_eq!(bus.text_vram[0x60_000], 0);
    }

    #[test]
    fn high_speed_clear_waits_for_display_start() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        for (index, value) in [10, 0, 0, 2, 3, 0, 0, 1, 0, 3].into_iter().enumerate() {
            bus.crtc.write_register(index, value);
        }
        bus.crtc.write_register(21, 0x000F);
        bus.graphic_vram[0] = 0xFFFF;
        bus.graphic_vram[16] = 0xFFFF;
        bus.graphic_vram[512] = 0xFFFF;
        advance_to_raster(&mut bus, 2);
        bus.m68000_write(access(0xE80480, M68000AccessSize::Word, supervisor), 0x0002)
            .unwrap();
        assert_eq!(
            bus.m68000_read(access(0xE80480, M68000AccessSize::Word, supervisor))
                .unwrap(),
            0
        );
        assert_eq!(bus.graphic_vram[0], 0xFFFF);
        advance_to_raster(&mut bus, 1);
        assert_eq!(bus.graphic_vram[0], 0);
        assert_eq!(bus.graphic_vram[16], 0xFFFF);
        assert_eq!(bus.graphic_vram[512], 0xFFFF);
        assert_eq!(
            bus.m68000_read(access(0xE80480, M68000AccessSize::Word, supervisor))
                .unwrap(),
            0x0002
        );
        bus.graphic_vram[0] = 0x1234;
        advance_to_raster(&mut bus, 2);
        advance_to_raster(&mut bus, 1);
        assert_eq!(
            bus.m68000_read(access(0xE80480, M68000AccessSize::Word, supervisor))
                .unwrap(),
            0
        );
        assert_eq!(bus.graphic_vram[0], 0x1234);
    }

    #[test]
    fn high_speed_clear_masks_pages_and_follows_the_page_zero_scroll() {
        let mut bus = bus(X68kModel::X68000);
        for (index, value) in [10, 0, 0, 2, 3, 0, 0, 2, 0, 3].into_iter().enumerate() {
            bus.crtc.write_register(index, value);
        }
        bus.crtc.write_register(21, 0x0006);
        bus.crtc.write_register(12, 501);
        bus.crtc.write_register(13, 511);
        for word in [511 * 512 + 500, 511 * 512 + 3, 500] {
            bus.graphic_vram[word] = 0xFFFF;
        }
        for word in [511 * 512 + 499, 511 * 512 + 4, 4] {
            bus.graphic_vram[word] = 0xFFFF;
        }
        bus.crtc.write_operation(0x0002);
        advance_to_raster(&mut bus, 1);
        assert_eq!(bus.graphic_vram[511 * 512 + 500], 0xF00F);
        assert_eq!(bus.graphic_vram[511 * 512 + 3], 0xF00F);
        assert_eq!(bus.graphic_vram[500], 0xF00F);
        assert_eq!(bus.graphic_vram[511 * 512 + 499], 0xFFFF);
        assert_eq!(bus.graphic_vram[511 * 512 + 4], 0xFFFF);
        assert_eq!(bus.graphic_vram[4], 0xFFFF);
    }

    #[test]
    fn high_speed_clear_wraps_horizontally_and_evens_the_scroll_start() {
        let mut bus = bus(X68kModel::X68000);
        for (index, value) in [10, 0, 0, 2, 3, 0, 0, 2, 0, 3].into_iter().enumerate() {
            bus.crtc.write_register(index, value);
        }
        bus.crtc.write_register(21, 0x000F);
        // Page-0 scroll x = 511: the clear starts at the even dot 510 and
        // its 16-dot width wraps to the row's left edge.
        bus.crtc.write_register(12, 511);
        for word in [509, 510, 511, 0, 13, 14] {
            bus.graphic_vram[word] = 0xFFFF;
        }
        bus.crtc.write_operation(0x0002);
        advance_to_raster(&mut bus, 1);
        assert_eq!(bus.graphic_vram[509], 0xFFFF);
        assert_eq!(bus.graphic_vram[510], 0);
        assert_eq!(bus.graphic_vram[511], 0);
        assert_eq!(bus.graphic_vram[0], 0);
        assert_eq!(bus.graphic_vram[13], 0);
        assert_eq!(bus.graphic_vram[14], 0xFFFF);
    }

    #[test]
    fn high_speed_clear_saturates_width_and_ignores_the_mask_at_1024() {
        let mut bus = bus(X68kModel::X68000);
        for (index, value) in [110, 0, 0, 66, 3, 0, 0, 1, 0, 3].into_iter().enumerate() {
            bus.crtc.write_register(index, value);
        }
        bus.crtc.write_register(20, 0x0400);
        bus.crtc.write_register(12, 100);
        bus.graphic_vram[0] = 0xFFFF;
        bus.graphic_vram[511] = 0xFFFF;
        bus.crtc.write_operation(0x0002);
        advance_to_raster(&mut bus, 1);
        assert_eq!(bus.graphic_vram[0], 0xFF00);
        assert_eq!(bus.graphic_vram[511], 0xFF00);
        bus.crtc.write_register(13, 512);
        bus.graphic_vram[0] = 0xFFFF;
        bus.crtc.write_operation(0x0002);
        advance_to_raster(&mut bus, 2);
        advance_to_raster(&mut bus, 1);
        assert_eq!(bus.graphic_vram[0], 0x00FF);
    }

    #[test]
    fn high_speed_clear_does_not_split_the_requesting_frame() {
        let mut bus = bus(X68kModel::X68000);
        for (index, value) in [10, 0, 0, 2, 3, 0, 0, 2, 0, 3].into_iter().enumerate() {
            bus.crtc.write_register(index, value);
        }
        bus.video_controller
            .write_palette(PaletteX68k::Graphics, 1, 0xFFFF);
        bus.video_controller.write_register(1, 0x00E4);
        bus.video_controller.write_register(2, 0x0001);
        bus.crtc.write_register(21, 0x000F);
        bus.contrast = 15;
        for row in 0..2 {
            for column in 0..16 {
                bus.graphic_vram[row * 512 + column] = 1;
            }
        }
        advance_to_raster(&mut bus, 2);
        bus.crtc.write_operation(0x0002);
        advance_to_raster(&mut bus, 0);
        assert_eq!(&bus.display_framebuffer()[..4], &[255, 255, 255, 255]);
        assert_eq!(
            &bus.display_framebuffer()[16 * 4..16 * 4 + 4],
            &[255, 255, 255, 255]
        );
        advance_to_raster(&mut bus, 1);
        advance_to_raster(&mut bus, 0);
        assert_eq!(&bus.display_framebuffer()[..4], &[0, 0, 0, 255]);
        assert_eq!(
            &bus.display_framebuffer()[16 * 4..16 * 4 + 4],
            &[0, 0, 0, 255]
        );
    }

    /// Builds a 16x4 pixel display bus for the mid-frame split tests.
    fn split_bus() -> X68kBus {
        let mut bus = bus(X68kModel::X68000);
        tiny_display(&mut bus, 2, 4);
        bus
    }

    /// Enables graphics page 0 with palette entry 1 on the whole screen.
    fn enable_graphics(bus: &mut X68kBus) {
        bus.video_controller.write_register(1, 0x00E4);
        bus.video_controller.write_register(2, 0x0001);
        bus.video_controller
            .write_palette(PaletteX68k::Graphics, 1, 0xFFFF);
        for row in 0..4 {
            for column in 0..16 {
                bus.graphic_vram[row * 512 + column] = 1;
            }
        }
    }

    /// Enables the text screen with palette entry 1 on the whole screen.
    fn enable_text(bus: &mut X68kBus) {
        bus.video_controller.write_register(2, 0x0020);
        bus.video_controller
            .write_palette(PaletteX68k::Text, 1, 0xFFFF);
        for row in 0..4 {
            bus.text_vram[row * 128] = 0xFF;
            bus.text_vram[row * 128 + 1] = 0xFF;
        }
    }

    /// Enables the sprite screen and programs its porch registers.
    fn enable_sprites(bus: &mut X68kBus) {
        bus.video_controller.write_register(2, 0x0040);
        write_word(bus, 0xEB080C, 0x0004);
        write_word(bus, 0xEB0808, 0x0200);
    }

    #[test]
    fn mid_frame_graphics_palette_change_splits_the_frame() {
        let mut bus = split_bus();
        enable_graphics(&mut bus);
        advance_to_raster(&mut bus, 2);
        write_word(&mut bus, 0xE82002, 0);
        complete_frame(&mut bus);
        assert_eq!(pixel(&bus, 0, 0), WHITE);
        assert_eq!(pixel(&bus, 0, 1), BLACK);
        assert_eq!(pixel(&bus, 0, 3), BLACK);
    }

    #[test]
    fn mid_frame_text_palette_change_splits_the_frame() {
        let mut bus = split_bus();
        enable_text(&mut bus);
        advance_to_raster(&mut bus, 2);
        write_word(&mut bus, 0xE82202, 0);
        complete_frame(&mut bus);
        assert_eq!(pixel(&bus, 0, 0), WHITE);
        assert_eq!(pixel(&bus, 0, 1), BLACK);
    }

    #[test]
    fn mid_frame_text_scroll_change_splits_the_frame() {
        let mut bus = split_bus();
        bus.video_controller.write_register(2, 0x0020);
        bus.video_controller
            .write_palette(PaletteX68k::Text, 1, 0xFFFF);
        bus.text_vram[2 * 128] = 0xFF;
        advance_to_raster(&mut bus, 2);
        write_word(&mut bus, 0xE80016, 1);
        complete_frame(&mut bus);
        assert_eq!(pixel(&bus, 0, 0), BLACK);
        assert_eq!(pixel(&bus, 0, 1), WHITE);
        assert_eq!(pixel(&bus, 0, 2), BLACK);
    }

    #[test]
    fn mid_frame_graphic_scroll_change_splits_the_frame() {
        let mut bus = split_bus();
        bus.video_controller.write_register(1, 0x00E4);
        bus.video_controller.write_register(2, 0x0001);
        bus.video_controller
            .write_palette(PaletteX68k::Graphics, 1, 0xFFFF);
        bus.graphic_vram[2 * 512] = 1;
        advance_to_raster(&mut bus, 2);
        write_word(&mut bus, 0xE8001A, 1);
        complete_frame(&mut bus);
        assert_eq!(pixel(&bus, 0, 0), BLACK);
        assert_eq!(pixel(&bus, 0, 1), WHITE);
        assert_eq!(pixel(&bus, 0, 2), BLACK);
    }

    #[test]
    fn mid_frame_background_scroll_change_splits_the_frame() {
        let mut bus = split_bus();
        enable_sprites(&mut bus);
        bus.video_controller
            .write_palette(PaletteX68k::Text, 5, 0xFFFF);
        write_word(&mut bus, 0xEB0808, 0x0201);
        for row in 0..8 {
            write_word(&mut bus, 0xEB8000 + (16 + row * 2) * 2, 0x5000);
        }
        write_word(&mut bus, 0xEBC000, 0x0001);
        write_word(&mut bus, 0xEBC002, 0x0001);
        advance_to_raster(&mut bus, 2);
        write_word(&mut bus, 0xEB0800, 4);
        complete_frame(&mut bus);
        assert_eq!(pixel(&bus, 0, 0), WHITE);
        assert_eq!(pixel(&bus, 4, 0), BLACK);
        assert_eq!(pixel(&bus, 0, 1), BLACK);
        assert_eq!(pixel(&bus, 4, 1), WHITE);
    }

    #[test]
    fn mid_frame_priority_swap_splits_the_frame() {
        let mut bus = split_bus();
        enable_text(&mut bus);
        bus.video_controller.write_register(1, 0x10E4);
        bus.video_controller.write_register(2, 0x0021);
        bus.video_controller
            .write_palette(PaletteX68k::Graphics, 1, 0x003E);
        for column in 0..16 {
            for row in 0..4 {
                bus.graphic_vram[row * 512 + column] = 1;
            }
        }
        advance_to_raster(&mut bus, 2);
        write_word(&mut bus, 0xE82500, 0x12E4);
        complete_frame(&mut bus);
        assert_eq!(pixel(&bus, 0, 0), [0, 0, 251, 255]);
        assert_eq!(pixel(&bus, 0, 1), WHITE);
    }

    #[test]
    fn mid_frame_layer_enable_toggle_splits_the_frame() {
        let mut bus = split_bus();
        enable_text(&mut bus);
        advance_to_raster(&mut bus, 2);
        write_word(&mut bus, 0xE82600, 0);
        complete_frame(&mut bus);
        assert_eq!(pixel(&bus, 0, 0), WHITE);
        assert_eq!(pixel(&bus, 0, 1), BLACK);
    }

    #[test]
    fn mid_frame_sprite_move_splits_the_frame() {
        let mut bus = split_bus();
        enable_sprites(&mut bus);
        bus.video_controller
            .write_palette(PaletteX68k::Text, 0x15, 0xFFFF);
        for word in 0..64_u32 {
            write_word(&mut bus, 0xEB8000 + (64 + word) * 2, 0x5555);
        }
        write_word(&mut bus, 0xEB0000, 16);
        write_word(&mut bus, 0xEB0002, 16);
        write_word(&mut bus, 0xEB0004, 0x0101);
        write_word(&mut bus, 0xEB0006, 3);
        advance_to_raster(&mut bus, 2);
        write_word(&mut bus, 0xEB0000, 24);
        complete_frame(&mut bus);
        assert_eq!(pixel(&bus, 0, 0), WHITE);
        assert_eq!(pixel(&bus, 0, 1), BLACK);
        assert_eq!(pixel(&bus, 8, 1), WHITE);
    }

    #[test]
    fn mid_frame_gvram_write_behind_the_beam_shows_next_frame() {
        let mut bus = split_bus();
        bus.video_controller.write_register(1, 0x00E4);
        bus.video_controller.write_register(2, 0x0001);
        bus.video_controller
            .write_palette(PaletteX68k::Graphics, 1, 0xFFFF);
        advance_to_raster(&mut bus, 2);
        write_word(&mut bus, 0xC00000, 1);
        assert_eq!(read_word(&mut bus, 0xC00000), 1);
        complete_frame(&mut bus);
        assert_eq!(pixel(&bus, 0, 0), BLACK);
        complete_frame(&mut bus);
        assert_eq!(pixel(&bus, 0, 0), WHITE);
    }

    #[test]
    fn mid_frame_mixing_mode_change_splits_the_frame() {
        let mut bus = split_bus();
        enable_graphics(&mut bus);
        advance_to_raster(&mut bus, 2);
        write_word(&mut bus, 0xE82600, 0x4001);
        complete_frame(&mut bus);
        assert_eq!(pixel(&bus, 0, 0), WHITE);
        assert_eq!(pixel(&bus, 0, 1), [121, 121, 121, 255]);
    }

    #[test]
    fn mid_frame_contrast_change_splits_the_frame() {
        let mut bus = split_bus();
        let supervisor = M68000FunctionCode::SupervisorData;
        enable_text(&mut bus);
        advance_to_raster(&mut bus, 2);
        bus.m68000_write(access(0xE8E001, M68000AccessSize::Byte, supervisor), 0x07)
            .unwrap();
        complete_frame(&mut bus);
        assert_eq!(pixel(&bus, 0, 0), WHITE);
        assert_eq!(pixel(&bus, 0, 1), [119, 119, 119, 255]);
    }

    #[test]
    fn frames_publish_only_at_frame_completion() {
        let mut bus = split_bus();
        enable_text(&mut bus);
        advance_to_raster(&mut bus, 2);
        assert_eq!(bus.display_dimensions(), (16, 4));
        assert_eq!(pixel(&bus, 0, 0), BLACK);
        complete_frame(&mut bus);
        assert_eq!(pixel(&bus, 0, 0), WHITE);
    }

    #[test]
    fn double_read_mode_duplicates_content_rows() {
        let mut bus = bus(X68kModel::X68000);
        tiny_display(&mut bus, 2, 4);
        bus.crtc.write_register(20, 0x0010);
        bus.video_controller.write_register(2, 0x0020);
        bus.video_controller
            .write_palette(PaletteX68k::Text, 1, 0xFFFF);
        bus.text_vram[0] = 0xFF;
        complete_frame(&mut bus);
        assert_eq!(bus.display_dimensions(), (16, 4));
        assert_eq!(pixel(&bus, 0, 0), WHITE);
        assert_eq!(pixel(&bus, 0, 1), WHITE);
        assert_eq!(pixel(&bus, 0, 2), BLACK);
        assert_eq!(pixel(&bus, 0, 3), BLACK);
    }

    #[test]
    fn interlaced_mode_weaves_fields_into_a_double_height_frame() {
        let mut bus = bus(X68kModel::X68000);
        tiny_display(&mut bus, 2, 2);
        bus.crtc.write_register(20, 0x0004);
        bus.video_controller.write_register(2, 0x0020);
        bus.video_controller
            .write_palette(PaletteX68k::Text, 1, 0xFFFF);

        // The even field carries content rows 0 and 2.
        bus.text_vram[0] = 0xFF;
        complete_frame(&mut bus);
        assert_eq!(bus.display_dimensions(), (16, 4));
        assert_eq!(pixel(&bus, 0, 0), WHITE);
        assert_eq!(pixel(&bus, 0, 1), BLACK);
        assert_eq!(pixel(&bus, 0, 2), BLACK);

        // The odd field carries content rows 1 and 3; even rows are retained.
        bus.text_vram[128] = 0xFF;
        complete_frame(&mut bus);
        assert_eq!(pixel(&bus, 0, 0), WHITE);
        assert_eq!(pixel(&bus, 0, 1), WHITE);
        assert_eq!(pixel(&bus, 0, 2), BLACK);
        assert_eq!(pixel(&bus, 0, 3), BLACK);
    }

    #[test]
    fn mid_frame_split_lands_on_the_published_raster_in_double_read_mode() {
        let mut bus = bus(X68kModel::X68000);
        tiny_display(&mut bus, 2, 4);
        bus.crtc.write_register(20, 0x0010);
        bus.video_controller.write_register(2, 0x0020);
        bus.video_controller
            .write_palette(PaletteX68k::Text, 1, 0xFFFF);
        bus.text_vram[0] = 0xFF;
        bus.text_vram[128] = 0xFF;
        advance_to_raster(&mut bus, 3);
        write_word(&mut bus, 0xE82202, 0);
        complete_frame(&mut bus);
        assert_eq!(pixel(&bus, 0, 0), WHITE);
        assert_eq!(pixel(&bus, 0, 1), WHITE);
        assert_eq!(pixel(&bus, 0, 2), BLACK);
        assert_eq!(pixel(&bus, 0, 3), BLACK);
    }

    #[test]
    fn high_speed_clear_covers_the_content_height_in_double_read_mode() {
        let mut bus = bus(X68kModel::X68000);
        tiny_display(&mut bus, 2, 4);
        bus.crtc.write_register(20, 0x0010);
        bus.crtc.write_register(21, 0x000F);
        bus.graphic_vram[0] = 0xFFFF;
        bus.graphic_vram[512] = 0xFFFF;
        bus.graphic_vram[2 * 512] = 0xFFFF;
        bus.crtc.write_operation(0x0002);
        advance_to_raster(&mut bus, 2);
        advance_to_raster(&mut bus, 1);
        assert_eq!(bus.graphic_vram[0], 0);
        assert_eq!(bus.graphic_vram[512], 0);
        assert_eq!(bus.graphic_vram[2 * 512], 0xFFFF);
    }

    #[test]
    fn beam_target_covers_before_during_and_after_display() {
        let geometry = CrtcGeometryX68k {
            width: 16,
            height: 2,
            first_raster: 3,
            first_column: 4,
        };
        let position = |raster, column, dot| CrtcBeamPositionX68k {
            raster,
            column,
            dot,
            odd_field: false,
        };
        assert_eq!(beam_pixel(geometry, position(2, 8, 0)), 0);
        assert_eq!(beam_pixel(geometry, position(3, 5, 2)), 10);
        assert_eq!(beam_pixel(geometry, position(5, 0, 0)), 32);
    }
}
