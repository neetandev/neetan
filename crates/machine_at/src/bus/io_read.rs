//! PC/AT I/O port read dispatch.

use common::Tracing;

use crate::{bus::AtBus, config::PIT_CLOCK_HZ};

/// Value returned by an open-bus / write-only port read.
const OPEN_BUS: u8 = 0xFF;

impl<T: Tracing> AtBus<T> {
    /// Reads a byte from an I/O port.
    pub(crate) fn io_read(&mut self, port: u16) -> u8 {
        match port {
            0x00..=0x0F | 0x80..=0x8F | 0xC0..=0xDF => self.dma.io_read(port).unwrap_or(OPEN_BUS),
            0x20 => self.pic.read_port0(0),
            0x21 => self.pic.read_port2(0),
            0x22 => OPEN_BUS, // config-address port is write-only
            0x23 => self.chipset.read_config_data(),
            0x40..=0x42 => {
                let channel = (port - 0x40) as usize;
                self.pit.read_counter(
                    channel,
                    self.current_cycle,
                    self.clocks.cpu_clock_hz,
                    PIT_CLOCK_HZ,
                )
            }
            0x43 => OPEN_BUS, // PIT control port is write-only
            0x60 => {
                let (byte, effects) = self.kbc.read_data();
                self.apply_kbc_effects(effects);
                byte
            }
            0x61 => {
                let refresh = self.refresh_toggle();
                let timer2 = self.timer2_output();
                self.chipset.read_port_b(refresh, timer2)
            }
            0x64 => self.kbc.read_status(),
            0x70 => OPEN_BUS, // NMI/RTC-address port is write-only
            0x71 => self.rtc.read(self.current_cycle, self.clocks.cpu_clock_hz),
            0x92 => self.chipset.read_sysctrl(),
            0xA0 => self.pic.read_port0(1),
            0xA1 => self.pic.read_port2(1),
            0xF0 | 0xF1 => OPEN_BUS, // FPU busy-latch clear ports are write-only
            0x03B0..=0x03DF => {
                let retrace = self.vga_retrace_status();
                self.vga.io_read(port, retrace).unwrap_or(OPEN_BUS)
            }
            0x0200..=0x0207 => self.gameport.read(self.current_cycle),
            0x0220..=0x022F | 0x0388..=0x038B => self.sound_io_read(port),
            0x0330..=0x0331 => self.mpu_io_read(port),
            0x03F8..=0x03FF => self.serial_io_read(port),
            0x01F0..=0x01F7 | 0x03F6 => self.ide_io_read(port),
            0x0170..=0x0177 | 0x0376 => self.ide_secondary_io_read(port),
            0x03F0..=0x03F5 | 0x03F7 => self.fdc_io_read(port),
            // Absent serial UARTs: COM2 (0x2F8), COM3 (0x3E8) and COM4 (0x2E8).
            // Only COM1 is fitted.
            0x02F8..=0x02FF | 0x03E8..=0x03EF | 0x02E8..=0x02EF => OPEN_BUS,
            // IBM 8514/A and S3 accelerator sparse register file (repeats every
            // 0x400 with the low 10 bits fixed at 0x2E8/0x2E9).
            port if port & 0x03FF == 0x02E8 || port & 0x03FF == 0x02E9 => OPEN_BUS,
            // Absent parallel ports LPT1 (0x378) and LPT2 (0x278).
            0x0378..=0x037A | 0x0278..=0x027A => OPEN_BUS,
            // UART-shaped presence scans at the non-standard bases 0x2F0, 0x5F0
            // and 0x6F0 used by optional multi-I/O / DOS-V add-on cards.
            0x02F0..=0x02F7 | 0x05F0..=0x05F7 | 0x06F0..=0x06F7 => OPEN_BUS,
            // Absent IBM XGA: instance register banks at 0x2100 + (instance << 4).
            0x2100..=0x217F => OPEN_BUS,
            // DOS/V display-adapter presence probe for an unfitted PS/55 adapter.
            0x1160 => OPEN_BUS,
            _ => {
                self.log_unhandled_read(port);
                OPEN_BUS
            }
        }
    }
}
