//! PC-88VA2 16-bit I/O read dispatch.

use super::{OPEN_BUS, Pc88VaBus};

impl<T: common::TraceSink> Pc88VaBus<T> {
    /// Reads an I/O byte and reports whether the hardware decoded the port.
    pub(crate) fn io_read(&mut self, port: u16) -> (u8, bool) {
        let value = match port {
            // 8259 PIC: master at 0x188/0x18A, slave at 0x184/0x186.
            0x184 => self.pic.read_port0(1),
            0x186 => self.pic.read_port2(1),
            0x188 => self.pic.read_port0(0),
            0x18A => self.pic.read_port2(0),

            // 8253 PIT counters.
            0x1A0 => self.read_pit_counter(0),
            0x1A2 => self.read_pit_counter(1),
            0x1A4 => self.read_pit_counter(2),

            // Sound board 2 (YM2608 / OPNA).
            0x044 => self.read_opn_status(),
            0x045 => self.read_opn_data(),
            0x046 => self.read_opn_status_hi(),
            0x047 => self.read_opn_data_hi(),

            // TSP: status read, and an open parameter port.
            0x142 => self.tsp.read_status(),
            0x143 => 0xFF,

            // Kanji / CGROM access window.
            0x14E => self.read_cgrom_data(),

            // uPD71071 DMA controller (channel 2 serves the main-CPU FDC).
            0x160..=0x16F => self.dmac.read(port),

            // uPD765A FDC, direct main-CPU access. Control ports 0x1B0-0x1B6 read
            // back 0; 0x1B8 is the main status register, 0x1BA the data register.
            0x1B0 | 0x1B2 | 0x1B4 | 0x1B6 => 0x00,
            0x1B8 => self.fdc.read_status(),
            0x1BA => self.read_fdc_data(),

            // VA91 ROM bank status: the optional version-up board is absent.
            0x156 => 0xFF,

            // HLE keyboard data port.
            0x1C1 => self.read_keyboard_data(),

            // Video controller: a few registers and the framebuffer descriptors.
            0x100..=0x10D | 0x200..=0x27F => match self.video.read(port) {
                Some(value) => value,
                None => return (OPEN_BUS, false),
            },

            // Super Graphic Processor (SGP) registers.
            0x500..=0x508 => self.sgp_io_read(port),

            // Graphics access controller (GACTRLVA).
            0x510..=0x5A3 => self.gactrlva.io_read(port),

            // 88-compatible keyboard scan matrix, rows read at 0x00-0x0E. The boot
            // ROM reads row 0x0D bit 2 (F8) here to enter the setup menu.
            0x000..=0x00E => self.keyboard.read_row(usize::from(port)),

            // SYSPORTVA system ports.
            0x032 => self.sysport.port032,
            0x040 => self.read_system_port_4(),
            0x150 => (self.sysport.modesw & 0x00FF) as u8,
            0x151 => (self.sysport.modesw >> 8) as u8,
            0x190 => self.sysport.port190,
            0x1C9 => self.sysport.a,
            0x1CB => self.read_crt_mode_port(),
            0x1CD => self.sysport.c,

            // PPI mailbox, host side: 0xFC=A, 0xFD=B, 0xFE=C, 0xFF=control.
            0xFC..=0xFF => self.ppi_main.read((port & 0x03) as u8),

            _ => match self.memory.io_read_byte(port) {
                Some(value) => value,
                None => return (OPEN_BUS, false),
            },
        };
        (value, true)
    }

    fn read_pit_counter(&mut self, channel: usize) -> u8 {
        let pit_clock_hz = self.pit_clock_hz();
        self.pit.read_counter(
            channel,
            self.current_cycle,
            self.clocks.main_clock_hz,
            pit_clock_hz,
        )
    }

    /// System port 4 (read at 0x040): VSYNC (bit 5), RTC data out (CDI, bit 4),
    /// CRT mode (bit 1), and printer-busy bits.
    fn read_system_port_4(&self) -> u8 {
        let crt_bit = if self.sysport.crt_mode_24khz() {
            0x00
        } else {
            0x02
        };
        0xC0 | (self.tsp.sysp4vsync & 0x20) | (self.rtc.cdat() << 4) | crt_bit | 0x01
    }

    /// CRT-mode / RS-232C status (read at 0x1CB). Bit3 reflects the CRT mode.
    fn read_crt_mode_port(&self) -> u8 {
        if self.sysport.crt_mode_24khz() {
            0x08
        } else {
            0x00
        }
    }
}
