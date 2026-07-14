//! Main CPU `0xFDxx` memory-mapped I/O decode.

use common::TraceSink;

use crate::bus::Fm7Bus;

/// `0xFD00` system control and keyboard-high port.
const PORT_SYSTEM: u8 = 0x00;
/// `0xFD01` keyboard-low and printer data port.
const PORT_KEYBOARD_LOW: u8 = 0x01;
/// `0xFD02` cassette/printer status and IRQ mask port.
const PORT_CASSETTE_PRINTER_IRQ_MASK: u8 = 0x02;
/// `0xFD03` IRQ status and beeper control port.
const PORT_IRQ_STATUS_BEEPER: u8 = 0x03;
/// `0xFD04` FIRQ status port.
const PORT_FIRQ_STATUS: u8 = 0x04;
/// `0xFD05` display sub-CPU control/status port.
const PORT_SUB_CONTROL: u8 = 0x05;
/// `0xFD0D` PSG command latch port (write).
const PORT_PSG_COMMAND: u8 = 0x0D;
/// `0xFD0E` PSG data port (read/write).
const PORT_PSG_DATA: u8 = 0x0E;
/// `0xFD0F` F-BASIC ROM bank latch port.
const PORT_BASIC_ROM_BANK: u8 = 0x0F;
/// `0xFD18` first floppy disk controller port.
const PORT_FDC_FIRST: u8 = 0x18;
/// `0xFD1F` last floppy disk controller port.
const PORT_FDC_LAST: u8 = 0x1F;
/// `0xFD20` kanji ROM address high byte (write).
const PORT_KANJI_ADDRESS_HIGH: u8 = 0x20;
/// `0xFD21` kanji ROM address low byte (write).
const PORT_KANJI_ADDRESS_LOW: u8 = 0x21;
/// `0xFD22` kanji ROM left data byte (read).
const PORT_KANJI_DATA_LEFT: u8 = 0x22;
/// `0xFD23` kanji ROM right data byte (read).
const PORT_KANJI_DATA_RIGHT: u8 = 0x23;
/// `0xFD0B` FM-77AV boot-mode readback port (read).
const PORT_BOOT_MODE: u8 = 0x0B;
/// `0xFD10` FM-77AV initiator ROM enable port (write).
const PORT_INITIATOR: u8 = 0x10;
/// `0xFD12` FM-77AV sub status / 320-mode select port (read/write).
const PORT_SUB_STATUS: u8 = 0x12;
/// `0xFD13` FM-77AV sub-monitor bank select port (write).
const PORT_SUB_MONITOR_BANK: u8 = 0x13;
/// `0xFD15` FM-77AV native OPN command latch port (write).
const PORT_OPN_COMMAND: u8 = 0x15;
/// `0xFD16` FM-77AV native OPN data port (read/write).
const PORT_OPN_DATA: u8 = 0x16;
/// `0xFD17` FM-77AV OPN/mouse external status/control port (read/write).
const PORT_OPN_EXT: u8 = 0x17;
/// `0xFD80` first FM-77AV MMR page register of the active segment.
const PORT_MMR_PAGE_FIRST: u8 = 0x80;
/// `0xFD8F` last FM-77AV MMR page register of the active segment.
const PORT_MMR_PAGE_LAST: u8 = 0x8F;
/// `0xFD90` FM-77AV MMR segment select port (write).
const PORT_MMR_SEGMENT: u8 = 0x90;
/// `0xFD92` FM-77AV MMR window offset port (write).
const PORT_MMR_WINDOW_OFFSET: u8 = 0x92;
/// `0xFD93` FM-77AV MMR control port (read/write).
const PORT_MMR_CONTROL: u8 = 0x93;

/// `0xFD0B` read value while booting into F-BASIC (bit 0 clear).
const BOOT_MODE_BASIC: u8 = 0xFE;
/// `0xFD0B` read value while booting from DOS-style media (bit 0 set).
const BOOT_MODE_OTHER: u8 = 0xFF;
/// `0xFD10` write bit 1: the initiator ROM overlay is enabled while it is clear.
const INITIATOR_DISABLE_BIT: u8 = 0x02;
/// `0xFD12` bit 6 selecting FM-77AV 320x200 (4096-color) mode.
const SUB_STATUS_MODE320_BIT: u8 = 0x40;
/// `0xFD12` read bit 1 reporting the active display period.
const SUB_STATUS_DISPLAY_BIT: u8 = 0x02;
/// `0xFD12` read bit 0 reporting the vertical sync pulse.
const SUB_STATUS_VSYNC_BIT: u8 = 0x01;
/// `0xFD12` read base value with the unused bits set.
const SUB_STATUS_READ_BASE: u8 = 0xBC;

/// `0xFD30` FM-77AV analog palette index high nibble (write).
const PORT_ANALOG_INDEX_HIGH: u8 = 0x30;
/// `0xFD31` FM-77AV analog palette index low byte (write).
const PORT_ANALOG_INDEX_LOW: u8 = 0x31;
/// `0xFD32` FM-77AV analog palette blue component (write).
const PORT_ANALOG_BLUE: u8 = 0x32;
/// `0xFD33` FM-77AV analog palette red component (write).
const PORT_ANALOG_RED: u8 = 0x33;
/// `0xFD34` FM-77AV analog palette green component (write).
const PORT_ANALOG_GREEN: u8 = 0x34;

/// `0xFD37` display multipage register (access mask + display mask).
const PORT_MULTIPAGE: u8 = 0x37;
/// `0xFD38` first digital palette register.
const PORT_DIGITAL_PALETTE_FIRST: u8 = 0x38;
/// `0xFD3F` last digital palette register.
const PORT_DIGITAL_PALETTE_LAST: u8 = 0x3F;

/// Base `0xFD00` read value with keyboard-high and speed bits clear.
const SYSTEM_BASE: u8 = 0x7E;
/// `0xFD00` read bit reporting fast CPU clock.
const SYSTEM_CLOCK_FAST: u8 = 0x01;
/// `0xFD00` read bit 7 carrying the ninth keycode bit.
const KEYCODE_HIGH_BIT: u8 = 0x80;
/// `0xFD05` read base value. Bit 7 (sub busy) is overlaid separately; bit 0 is the
/// external-detect line reporting the floppy/expansion unit present (active low,
/// `0` = present), so it reads `0` on this machine, which always fits the FDC.
const SUB_STATUS_BASE: u8 = 0x7E;
/// Bit 7 of `0xFD04`/`0xFD05` reporting the sub CPU busy / halted state.
const SUB_BUSY_BIT: u8 = 0x80;
/// `0xFD05` write bit requesting the sub CPU to halt.
const SUB_HALT_REQUEST_BIT: u8 = 0x80;
/// `0xFD05` write bit raising the sub CANCEL interrupt.
const SUB_CANCEL_BIT: u8 = 0x40;
/// Open-bus value for unhandled reads.
const OPEN_BUS: u8 = 0xFF;

impl<T: TraceSink> Fm7Bus<T> {
    /// Reads a byte from the main CPU `0xFDxx` I/O page.
    pub(crate) fn main_io_read(&mut self, port: u8) -> (u8, bool) {
        let mut handled = true;
        let value = match port {
            PORT_SYSTEM => {
                let mut value = SYSTEM_BASE;
                if self.clock_fast {
                    value |= SYSTEM_CLOCK_FAST;
                }
                if self.keyboard.keycode_high() {
                    value |= KEYCODE_HIGH_BIT;
                }
                value
            }
            PORT_KEYBOARD_LOW => {
                let value = self.keyboard.read_low();
                self.interrupts.set_keyboard_pending(
                    false,
                    common::TraceContext::main_cpu(
                        self.current_cycle,
                        Some(u64::from(self.cpu_clock_hz())),
                    ),
                    &mut self.tracer,
                );
                value
            }
            PORT_CASSETTE_PRINTER_IRQ_MASK => self.read_cassette_printer_status(),
            PORT_IRQ_STATUS_BEEPER => self.interrupts.read_status(
                common::TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.cpu_clock_hz())),
                ),
                &mut self.tracer,
            ),
            PORT_FIRQ_STATUS => {
                let status = self.interrupts.read_firq_status(
                    common::TraceContext::main_cpu(
                        self.current_cycle,
                        Some(u64::from(self.cpu_clock_hz())),
                    ),
                    &mut self.tracer,
                );
                (status & !SUB_BUSY_BIT) | self.sub_busy_bit()
            }
            PORT_SUB_CONTROL => SUB_STATUS_BASE | self.sub_busy_bit(),
            PORT_PSG_DATA => self.read_psg_data(),
            PORT_BASIC_ROM_BANK => {
                self.memory.map_rom();
                OPEN_BUS
            }
            PORT_FDC_FIRST..=PORT_FDC_LAST => self.fdc_read(port),
            PORT_KANJI_DATA_LEFT => self.kanji.read_left(),
            PORT_KANJI_DATA_RIGHT => self.kanji.read_right(),
            PORT_DIGITAL_PALETTE_FIRST..=PORT_DIGITAL_PALETTE_LAST => self
                .video
                .read_digital_palette(port - PORT_DIGITAL_PALETTE_FIRST),
            _ => match self.av_io_read(port) {
                Some(value) => value,
                None => {
                    handled = false;
                    OPEN_BUS
                }
            },
        };
        (value, handled)
    }

    /// Writes a byte to the main CPU `0xFDxx` I/O page.
    pub(crate) fn main_io_write(&mut self, port: u8, value: u8) -> bool {
        let mut handled = true;
        match port {
            PORT_SYSTEM => self.write_system_port(value),
            PORT_KEYBOARD_LOW => {}
            PORT_CASSETTE_PRINTER_IRQ_MASK => self.interrupts.write_mask(
                value,
                common::TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.cpu_clock_hz())),
                ),
                &mut self.tracer,
            ),
            PORT_IRQ_STATUS_BEEPER => self.write_beeper_control(value),
            PORT_FIRQ_STATUS => {}
            PORT_SUB_CONTROL => {
                self.sub_halt_requested = value & SUB_HALT_REQUEST_BIT != 0;
                if value & SUB_CANCEL_BIT != 0 {
                    self.raise_cancel_request();
                }
            }
            PORT_PSG_COMMAND => self.write_psg_command(value),
            PORT_PSG_DATA => self.write_psg_data(value),
            PORT_BASIC_ROM_BANK => self.memory.map_ram(),
            PORT_FDC_FIRST..=PORT_FDC_LAST => self.fdc_write(port, value),
            PORT_KANJI_ADDRESS_HIGH => self.kanji.write_address_high(value),
            PORT_KANJI_ADDRESS_LOW => self.kanji.write_address_low(value),
            PORT_MULTIPAGE => self.video.write_multipage(value),
            PORT_DIGITAL_PALETTE_FIRST..=PORT_DIGITAL_PALETTE_LAST => {
                self.video
                    .write_digital_palette(port - PORT_DIGITAL_PALETTE_FIRST, value);
            }
            _ => {
                if !self.av_io_write(port, value) {
                    handled = false;
                }
            }
        }
        handled
    }

    /// Bit 7 reporting the sub CPU busy / halted state to `0xFD04`/`0xFD05`.
    fn sub_busy_bit(&self) -> u8 {
        if self.sub_busy() { SUB_BUSY_BIT } else { 0 }
    }

    /// Reads an FM-77AV-only `0xFDxx` port, returning `None` on the FM-7 (where
    /// these ports are unmapped) so the caller falls back to open bus.
    fn av_io_read(&mut self, port: u8) -> Option<u8> {
        if !self.model().has_mmr() {
            return None;
        }
        let value = match port {
            PORT_BOOT_MODE => self.read_av_boot_mode(),
            PORT_SUB_STATUS => self.read_av_sub_status(),
            PORT_OPN_DATA => self.read_opn_data(),
            PORT_OPN_EXT => self.read_opn_ext_status(),
            PORT_MMR_PAGE_FIRST..=PORT_MMR_PAGE_LAST => self
                .memory
                .read_mmr_page_register(port - PORT_MMR_PAGE_FIRST),
            PORT_MMR_CONTROL => self.memory.read_mmr_control(),
            _ => return None,
        };
        Some(value)
    }

    /// Writes an FM-77AV-only `0xFDxx` port, returning `false` on the FM-7 or for
    /// an unhandled port so the caller emits the unhandled-write trace.
    fn av_io_write(&mut self, port: u8, value: u8) -> bool {
        if !self.model().has_mmr() {
            return false;
        }
        match port {
            PORT_INITIATOR => self
                .memory
                .set_initiator_enabled(value & INITIATOR_DISABLE_BIT == 0),
            PORT_SUB_STATUS => self.video.set_mode320(value & SUB_STATUS_MODE320_BIT != 0),
            PORT_ANALOG_INDEX_HIGH if self.model().has_analog_palette() => {
                self.video.write_analog_index_high(value);
            }
            PORT_ANALOG_INDEX_LOW if self.model().has_analog_palette() => {
                self.video.write_analog_index_low(value);
            }
            PORT_ANALOG_BLUE if self.model().has_analog_palette() => {
                self.video.write_analog_blue(value);
            }
            PORT_ANALOG_RED if self.model().has_analog_palette() => {
                self.video.write_analog_red(value);
            }
            PORT_ANALOG_GREEN if self.model().has_analog_palette() => {
                self.video.write_analog_green(value);
            }
            PORT_SUB_MONITOR_BANK => self.set_sub_monitor_bank(value),
            PORT_OPN_COMMAND => self.write_opn_native_command(value),
            PORT_OPN_DATA => self.write_opn_data(value),
            PORT_OPN_EXT => self.write_opn_ext_control(value),
            PORT_MMR_PAGE_FIRST..=PORT_MMR_PAGE_LAST => self
                .memory
                .write_mmr_page_register(port - PORT_MMR_PAGE_FIRST, value),
            PORT_MMR_SEGMENT => self.memory.set_mmr_segment(value),
            PORT_MMR_WINDOW_OFFSET => self.memory.set_mmr_window_offset(value),
            PORT_MMR_CONTROL => self.write_mmr_control(value),
            _ => return false,
        }
        true
    }

    /// Reads the FM-77AV boot-mode port `0xFD0B`: bit 0 clear while booting into
    /// F-BASIC, set otherwise.
    fn read_av_boot_mode(&self) -> u8 {
        match self.boot_mode {
            crate::config::BootMode::Basic => BOOT_MODE_BASIC,
            crate::config::BootMode::Dos => BOOT_MODE_OTHER,
        }
    }

    /// Reads the FM-77AV sub status port `0xFD12`: the 320-mode latch in bit 6,
    /// the active display period in bit 1 and the vertical sync pulse in bit 0.
    fn read_av_sub_status(&self) -> u8 {
        let mut value = SUB_STATUS_READ_BASE;
        if self.video.mode320() {
            value |= SUB_STATUS_MODE320_BIT;
        }
        if self.display_active() {
            value |= SUB_STATUS_DISPLAY_BIT;
        }
        if self.vsync_active() {
            value |= SUB_STATUS_VSYNC_BIT;
        }
        value
    }
}
