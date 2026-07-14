//! Graphics VRAM access packing for the CRTC memory modes.
//!
//! The 2 MiB window at `0xC00000` exposes one 512x512 array of 16-bit words
//! through mode-dependent page windows. The 16-color mode maps each of the
//! four nibbles as its own page window, the 256-color mode maps the two
//! nibble pairs as byte-wide page windows, the 65536-color mode maps the
//! full words once, and the 1024x1024 mode maps the four nibbles as screen
//! quadrants. Accesses outside the windows a mode defines raise bus errors.

use common::{M68000BusError, TraceSink};
use device::crtc_x68k::GvramModeX68k;

use super::X68kBus;

/// Base address of the graphics VRAM window.
const GVRAM_BASE: u32 = 0xC00000;
/// Word-offset mask within one 512x512 page.
const PAGE_WORD_MASK: u32 = 0x3_FFFF;

impl<T: TraceSink> X68kBus<T> {
    /// Reads one packed graphics VRAM word.
    pub(super) fn read_graphic_vram_word(&self, address: u32) -> Result<u16, M68000BusError> {
        let relative = address - GVRAM_BASE;
        let page_word = ((relative >> 1) & PAGE_WORD_MASK) as usize;
        let page_window = (relative >> 19) as usize;
        if self.crtc.graphic_storage_enabled() {
            return if page_window == 0 {
                Ok(self.graphic_vram[page_word])
            } else {
                Err(M68000BusError)
            };
        }
        match self.crtc.graphic_memory_mode() {
            GvramModeX68k::Colors16 => {
                Ok((self.graphic_vram[page_word] >> (page_window * 4)) & 0x000F)
            }
            GvramModeX68k::Colors256 => match page_window {
                0 => Ok(self.graphic_vram[page_word] & 0x00FF),
                1 => Ok(self.graphic_vram[page_word] >> 8),
                _ => Err(M68000BusError),
            },
            GvramModeX68k::MemoryMode2 => {
                if page_window & 1 == 0 {
                    Ok(self.graphic_vram[page_word] & 0x00FF)
                } else {
                    Ok(self.graphic_vram[page_word] >> 8)
                }
            }
            GvramModeX68k::Colors65536 => {
                if page_window == 0 {
                    Ok(self.graphic_vram[page_word])
                } else {
                    Err(M68000BusError)
                }
            }
            GvramModeX68k::Colors16Virtual1024 => {
                let (word, nibble) = virtual_1024_location(relative);
                Ok((self.graphic_vram[word] >> (nibble * 4)) & 0x000F)
            }
        }
    }

    /// Reads one graphics VRAM byte lane.
    pub(super) fn read_graphic_vram_byte(&self, address: u32) -> Result<u8, M68000BusError> {
        let relative = address - GVRAM_BASE;
        if self.crtc.graphic_memory_mode() == GvramModeX68k::MemoryMode2
            && !self.crtc.graphic_storage_enabled()
        {
            let page_word = ((relative >> 1) & PAGE_WORD_MASK) as usize;
            return if relative & 1 != 0 && (relative >> 19) & 1 == 0 {
                Ok(self.graphic_vram[page_word] as u8)
            } else {
                Ok(0)
            };
        }
        let word = self.read_graphic_vram_word(address & !1)?;
        if address & 1 == 0 {
            Ok((word >> 8) as u8)
        } else {
            Ok(word as u8)
        }
    }

    /// Writes one packed graphics VRAM word.
    pub(super) fn write_graphic_vram_word(
        &mut self,
        address: u32,
        value: u16,
    ) -> Result<(), M68000BusError> {
        self.catch_up_video();
        let relative = address - GVRAM_BASE;
        let page_word = ((relative >> 1) & PAGE_WORD_MASK) as usize;
        let page_window = (relative >> 19) as usize;
        if self.crtc.graphic_storage_enabled() {
            return if page_window == 0 {
                self.graphic_vram[page_word] = value;
                Ok(())
            } else {
                Err(M68000BusError)
            };
        }
        match self.crtc.graphic_memory_mode() {
            GvramModeX68k::Colors16 => {
                let shift = page_window * 4;
                let word = &mut self.graphic_vram[page_word];
                *word = *word & !(0x000F << shift) | (value & 0x000F) << shift;
            }
            GvramModeX68k::Colors256 => match page_window {
                0 => {
                    let word = &mut self.graphic_vram[page_word];
                    *word = *word & 0xFF00 | value & 0x00FF;
                }
                1 => {
                    let word = &mut self.graphic_vram[page_word];
                    *word = *word & 0x00FF | (value & 0x00FF) << 8;
                }
                _ => return Err(M68000BusError),
            },
            GvramModeX68k::MemoryMode2 => {
                let low = value & 0x00FF;
                self.graphic_vram[page_word] = low << 8 | low;
            }
            GvramModeX68k::Colors65536 => {
                if page_window != 0 {
                    return Err(M68000BusError);
                }
                self.graphic_vram[page_word] = value;
            }
            GvramModeX68k::Colors16Virtual1024 => {
                let (word_index, nibble) = virtual_1024_location(relative);
                let shift = nibble * 4;
                let word = &mut self.graphic_vram[word_index];
                *word = *word & !(0x000F << shift) | (value & 0x000F) << shift;
            }
        }
        Ok(())
    }

    /// Writes one graphics VRAM byte lane.
    pub(super) fn write_graphic_vram_byte(
        &mut self,
        address: u32,
        value: u8,
    ) -> Result<(), M68000BusError> {
        let relative = address - GVRAM_BASE;
        let even_lane = address & 1 == 0;
        if self.crtc.graphic_storage_enabled() {
            self.catch_up_video();
            let page_word = ((relative >> 1) & PAGE_WORD_MASK) as usize;
            if relative >> 19 != 0 {
                return Err(M68000BusError);
            }
            let word = &mut self.graphic_vram[page_word];
            *word = if even_lane {
                *word & 0x00FF | u16::from(value) << 8
            } else {
                *word & 0xFF00 | u16::from(value)
            };
            return Ok(());
        }
        match self.crtc.graphic_memory_mode() {
            GvramModeX68k::Colors16 | GvramModeX68k::Colors16Virtual1024 => {
                if even_lane {
                    self.check_graphic_window(relative)?;
                    return Ok(());
                }
                self.write_graphic_vram_word(address & !1, u16::from(value))
            }
            GvramModeX68k::Colors256 => {
                if even_lane {
                    self.check_graphic_window(relative)?;
                    return Ok(());
                }
                self.write_graphic_vram_word(address & !1, u16::from(value))
            }
            GvramModeX68k::MemoryMode2 => {
                self.catch_up_video();
                let page_word = ((relative >> 1) & PAGE_WORD_MASK) as usize;
                let word = &mut self.graphic_vram[page_word];
                *word = if even_lane {
                    *word & 0x00FF | u16::from(value) << 8
                } else {
                    *word & 0xFF00 | u16::from(value)
                };
                Ok(())
            }
            GvramModeX68k::Colors65536 => {
                self.check_graphic_window(relative)?;
                self.catch_up_video();
                let page_word = ((relative >> 1) & PAGE_WORD_MASK) as usize;
                let word = &mut self.graphic_vram[page_word];
                *word = if even_lane {
                    *word & 0x00FF | u16::from(value) << 8
                } else {
                    *word & 0xFF00 | u16::from(value)
                };
                Ok(())
            }
        }
    }

    /// Validates that the current mode maps the accessed page window.
    fn check_graphic_window(&self, relative: u32) -> Result<(), M68000BusError> {
        let page_window = relative >> 19;
        let mapped = match self.crtc.graphic_memory_mode() {
            GvramModeX68k::Colors16
            | GvramModeX68k::MemoryMode2
            | GvramModeX68k::Colors16Virtual1024 => true,
            GvramModeX68k::Colors256 => page_window < 2,
            GvramModeX68k::Colors65536 => page_window == 0,
        };
        if mapped { Ok(()) } else { Err(M68000BusError) }
    }
}

/// Maps a window offset to its word index and quadrant nibble in 1024 mode.
const fn virtual_1024_location(relative: u32) -> (usize, usize) {
    let word = ((relative >> 2) & 0x3_FE00 | (relative >> 1) & 0x01FF) as usize;
    let nibble = ((relative >> 19) & 2 | (relative >> 10) & 1) as usize;
    (word, nibble)
}

#[cfg(test)]
mod tests {
    use common::{Bus, M68000AccessSize, M68000FunctionCode};

    use crate::{
        X68kModel,
        bus::{X68kBus, test_support::bus},
    };

    /// CRTC R20 values for the graphics memory modes under test.
    const MODE_16: u16 = 0x0000;
    const MODE_256: u16 = 0x0100;
    const MODE_2: u16 = 0x0200;
    const MODE_65536: u16 = 0x0300;
    const MODE_1024: u16 = 0x0400;
    const STORAGE: u16 = 0x0800;

    fn gvram_bus(memory_mode: u16) -> X68kBus {
        let mut bus = bus(X68kModel::X68000);
        bus.crtc.write_register(20, memory_mode);
        bus
    }

    fn write_word(bus: &mut X68kBus, address: u32, value: u16) {
        bus.m68000_write(
            crate::bus::test_support::access(
                address,
                M68000AccessSize::Word,
                M68000FunctionCode::SupervisorData,
            ),
            value,
        )
        .unwrap();
    }

    fn read_word(bus: &mut X68kBus, address: u32) -> u16 {
        bus.m68000_read(crate::bus::test_support::access(
            address,
            M68000AccessSize::Word,
            M68000FunctionCode::SupervisorData,
        ))
        .unwrap()
    }

    fn write_byte(bus: &mut X68kBus, address: u32, value: u8) {
        bus.m68000_write(
            crate::bus::test_support::access(
                address,
                M68000AccessSize::Byte,
                M68000FunctionCode::SupervisorData,
            ),
            u16::from(value),
        )
        .unwrap();
    }

    fn read_byte(bus: &mut X68kBus, address: u32) -> u8 {
        bus.m68000_read(crate::bus::test_support::access(
            address,
            M68000AccessSize::Byte,
            M68000FunctionCode::SupervisorData,
        ))
        .unwrap() as u8
    }

    fn word_fails(bus: &mut X68kBus, address: u32) -> bool {
        bus.m68000_read(crate::bus::test_support::access(
            address,
            M68000AccessSize::Word,
            M68000FunctionCode::SupervisorData,
        ))
        .is_err()
    }

    #[test]
    fn sixteen_color_pages_are_independent_nibble_planes() {
        let mut bus = gvram_bus(MODE_16);
        write_word(&mut bus, 0xC00000, 0xFFFF);
        write_word(&mut bus, 0xC80000, 0x0003);
        write_word(&mut bus, 0xD00000, 0x0005);
        write_word(&mut bus, 0xD80000, 0x0009);
        assert_eq!(read_word(&mut bus, 0xC00000), 0x000F);
        assert_eq!(read_word(&mut bus, 0xC80000), 0x0003);
        assert_eq!(read_word(&mut bus, 0xD00000), 0x0005);
        assert_eq!(read_word(&mut bus, 0xD80000), 0x0009);
        assert_eq!(bus.graphic_vram_data()[0], 0x953F);
    }

    #[test]
    fn sixteen_color_byte_lanes_use_only_the_odd_byte() {
        let mut bus = gvram_bus(MODE_16);
        write_byte(&mut bus, 0xC00001, 0x1A);
        write_byte(&mut bus, 0xC00000, 0xFF);
        assert_eq!(read_word(&mut bus, 0xC00000), 0x000A);
        assert_eq!(read_byte(&mut bus, 0xC00000), 0x00);
        assert_eq!(read_byte(&mut bus, 0xC00001), 0x0A);
    }

    #[test]
    fn sixteen_color_byte_reads_expose_each_plane_nibble() {
        let mut bus = gvram_bus(MODE_65536);
        write_word(&mut bus, 0xC00000, 0x1234);
        bus.crtc.write_register(20, MODE_16);
        assert_eq!(read_byte(&mut bus, 0xC00001), 0x04);
        assert_eq!(read_byte(&mut bus, 0xC80001), 0x03);
        assert_eq!(read_byte(&mut bus, 0xD00001), 0x02);
        assert_eq!(read_byte(&mut bus, 0xD80001), 0x01);
        assert_eq!(read_byte(&mut bus, 0xD80000), 0x00);
    }

    #[test]
    fn two_hundred_fifty_six_color_byte_lanes_use_only_the_odd_byte() {
        let mut bus = gvram_bus(MODE_65536);
        write_word(&mut bus, 0xC00000, 0x1234);
        bus.crtc.write_register(20, MODE_256);
        assert_eq!(read_byte(&mut bus, 0xC00000), 0x00);
        assert_eq!(read_byte(&mut bus, 0xC00001), 0x34);
        assert_eq!(read_byte(&mut bus, 0xC80001), 0x12);
        write_byte(&mut bus, 0xC00000, 0x56);
        assert_eq!(read_byte(&mut bus, 0xC00001), 0x34);
        write_byte(&mut bus, 0xC00001, 0x78);
        write_byte(&mut bus, 0xC80001, 0x9A);
        assert_eq!(bus.graphic_vram_data()[0], 0x9A78);
    }

    #[test]
    fn two_hundred_fifty_six_color_pages_map_byte_pairs() {
        let mut bus = gvram_bus(MODE_256);
        write_word(&mut bus, 0xC00000, 0xA55A);
        write_word(&mut bus, 0xC80000, 0x11C3);
        assert_eq!(read_word(&mut bus, 0xC00000), 0x005A);
        assert_eq!(read_word(&mut bus, 0xC80000), 0x00C3);
        assert_eq!(bus.graphic_vram_data()[0], 0xC35A);
    }

    #[test]
    fn two_hundred_fifty_six_color_upper_windows_raise_bus_errors() {
        let mut bus = gvram_bus(MODE_256);
        assert!(word_fails(&mut bus, 0xD00000));
        assert!(word_fails(&mut bus, 0xDFFFFE));
    }

    #[test]
    fn full_color_words_round_trip_and_upper_windows_fail() {
        let mut bus = gvram_bus(MODE_65536);
        write_word(&mut bus, 0xC00000, 0x1234);
        assert_eq!(read_word(&mut bus, 0xC00000), 0x1234);
        assert!(word_fails(&mut bus, 0xC80000));
        assert!(word_fails(&mut bus, 0xD80000));
        bus.crtc.write_register(20, MODE_16);
        assert_eq!(read_word(&mut bus, 0xC00000), 0x0004);
        assert_eq!(read_word(&mut bus, 0xC80000), 0x0003);
        assert_eq!(read_word(&mut bus, 0xD00000), 0x0002);
        assert_eq!(read_word(&mut bus, 0xD80000), 0x0001);
    }

    #[test]
    fn memory_mode_two_packs_page_pairs() {
        let mut bus = gvram_bus(MODE_2);
        write_word(&mut bus, 0xC00000, 0xFF3C);
        assert_eq!(bus.graphic_vram_data()[0], 0x3C3C);
        assert_eq!(read_word(&mut bus, 0xC00000), 0x003C);
        assert_eq!(read_word(&mut bus, 0xC80000), 0x003C);
        assert_eq!(read_word(&mut bus, 0xD00000), 0x003C);
        write_byte(&mut bus, 0xC80000, 0x71);
        assert_eq!(bus.graphic_vram_data()[0], 0x713C);
        assert_eq!(read_word(&mut bus, 0xC80000), 0x0071);
        assert_eq!(read_byte(&mut bus, 0xC80001), 0x00);
        assert_eq!(read_byte(&mut bus, 0xC00001), 0x3C);
        assert_eq!(read_byte(&mut bus, 0xC00000), 0x00);
    }

    #[test]
    fn virtual_1024_mode_addresses_quadrant_nibbles() {
        let mut bus = gvram_bus(MODE_1024);
        write_word(&mut bus, 0xC00000, 0x000F);
        write_word(&mut bus, 0xC00000 + 512 * 2, 0x0001);
        write_word(&mut bus, 0xC00000 + 512 * 2048, 0x0002);
        write_word(&mut bus, 0xC00000 + 512 * 2050, 0x0003);
        assert_eq!(bus.graphic_vram_data()[0], 0x321F);
        assert_eq!(read_word(&mut bus, 0xC00000), 0x000F);
        assert_eq!(read_word(&mut bus, 0xC00000 + 512 * 2), 0x0001);
        assert_eq!(read_word(&mut bus, 0xC00000 + 512 * 2048), 0x0002);
        assert_eq!(read_word(&mut bus, 0xC00000 + 512 * 2050), 0x0003);
        bus.crtc.write_register(20, MODE_16);
        assert_eq!(read_word(&mut bus, 0xC00000), 0x000F);
        assert_eq!(read_word(&mut bus, 0xC80000), 0x0001);
        assert_eq!(read_word(&mut bus, 0xD00000), 0x0002);
        assert_eq!(read_word(&mut bus, 0xD80000), 0x0003);
    }

    #[test]
    fn undefined_upper_modes_behave_like_the_1024_mode() {
        for mode in [0x0500, 0x0600, 0x0700] {
            let mut bus = gvram_bus(mode);
            write_word(&mut bus, 0xC00000 + 512 * 2, 0x0001);
            assert_eq!(read_word(&mut bus, 0xC00000 + 512 * 2), 0x0001);
            bus.crtc.write_register(20, MODE_16);
            assert_eq!(read_word(&mut bus, 0xC80000), 0x0001);
        }
    }

    #[test]
    fn storage_mode_maps_flat_words_in_the_first_window() {
        let mut bus = gvram_bus(MODE_16 | STORAGE);
        write_word(&mut bus, 0xC00000, 0xBEEF);
        assert_eq!(read_word(&mut bus, 0xC00000), 0xBEEF);
        assert_eq!(bus.graphic_vram_data()[0], 0xBEEF);
        assert!(word_fails(&mut bus, 0xC80000));
        write_byte(&mut bus, 0xC00002, 0x12);
        write_byte(&mut bus, 0xC00003, 0x34);
        assert_eq!(read_word(&mut bus, 0xC00002), 0x1234);
    }

    #[test]
    fn cpu_packing_follows_the_crtc_not_the_video_controller() {
        let mut bus = gvram_bus(MODE_16);
        bus.video_controller.write_register(0, 0x0003);
        write_word(&mut bus, 0xC00000, 0xFFFF);
        assert_eq!(read_word(&mut bus, 0xC00000), 0x000F);
    }
}
