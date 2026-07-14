//! PC/AT I/O port write dispatch.

use common::TraceSink;
use device::i8253_pit::WriteResult;

use crate::{
    bus::{AtBus, IRQ_FPU},
    config::PIT_CLOCK_HZ,
};

/// PIT read-back command selector (control word SC bits = 11).
const PIT_READ_BACK: u8 = 0xC0;

impl<T: TraceSink> AtBus<T> {
    /// Writes a byte to an I/O port.
    pub(crate) fn io_write(&mut self, port: u16, value: u8) -> bool {
        let mut handled = true;
        match port {
            0x80 => {
                // Port 0x80 is both the POST diagnostic port and a DMA page
                // scratch register: record the code, then store it.
                self.record_post_code(value);
                self.dma.io_write(port, value);
            }
            0x00..=0x0F | 0x81..=0x8F | 0xC0..=0xDF => {
                self.dma.io_write(port, value);
            }
            0x20 => self.pic.write_port0(0, value),
            0x21 => self.pic.write_port2(0, value),
            0x22 => self.chipset.write_config_address(value),
            0x23 => {
                let effects = self.chipset.write_config_data(value);
                self.apply_chipset_effects(effects);
            }
            0x40..=0x42 => {
                let channel = (port - 0x40) as usize;
                let result = self.pit.write_counter(channel, value);
                if result != WriteResult::Skip {
                    self.pit.channels[channel].last_load_cycle = self.current_cycle;
                    if channel == 0 {
                        self.reschedule_pit_channel0();
                    } else if channel == 2 {
                        let reload = self.pit.channels[2].value;
                        self.beeper.set_pit_reload(reload, self.current_cycle);
                    }
                }
            }
            0x43 => {
                if value & PIT_READ_BACK == PIT_READ_BACK {
                    self.pit.write_read_back(
                        value,
                        self.current_cycle,
                        self.clocks.cpu_clock_hz,
                        PIT_CLOCK_HZ,
                    );
                } else {
                    let channel = (value >> 6) as usize;
                    self.pit.write_control(
                        channel,
                        value,
                        self.current_cycle,
                        self.clocks.cpu_clock_hz,
                        PIT_CLOCK_HZ,
                    );
                }
            }
            0x60 => {
                let action = self.chipset.filter_keyboard_data(value);
                if action.cpu_reset_pulse {
                    self.cpu_reset_pending = true;
                }
                if action.forward {
                    let effects = self.kbc.write_data(value);
                    self.apply_kbc_effects(effects);
                }
                self.memory.set_a20(self.chipset.a20_enabled());
            }
            0x61 => {
                let effects = self.chipset.write_port_b(value);
                // A rising edge on the timer-2 gate restarts channel 2.
                if effects.timer2_gate && !self.timer2_gate {
                    self.pit.channels[2].last_load_cycle = self.current_cycle;
                }
                self.timer2_gate = effects.timer2_gate;
                self.beeper.set_buzzer_enabled(
                    effects.timer2_gate && effects.speaker_data,
                    self.current_cycle,
                );
            }
            0x64 => {
                let action = self.chipset.filter_keyboard_command(value);
                if action.cpu_reset_pulse {
                    self.cpu_reset_pending = true;
                }
                if action.forward {
                    let effects = self.kbc.write_command(value);
                    self.apply_kbc_effects(effects);
                }
                self.memory.set_a20(self.chipset.a20_enabled());
            }
            0x70 => {
                let address = self.chipset.write_rtc_nmi(value);
                self.rtc.set_address(address);
            }
            0x71 => {
                let effect = self.rtc.write(value);
                if effect.reschedule_update {
                    self.reschedule_rtc_update();
                }
                if effect.reschedule_periodic {
                    self.reschedule_rtc_periodic();
                }
            }
            0x92 => {
                let effects = self.chipset.write_sysctrl(value);
                self.apply_chipset_effects(effects);
            }
            0xA0 => self.pic.write_port0(1, value),
            0xA1 => self.pic.write_port2(1, value),
            0xF0 | 0xF1 => {
                // Clear the coprocessor busy latch (FERR#).
                self.fpu_busy_latch = false;
                self.clear_irq(IRQ_FPU);
            }
            0x03B0..=0x03DF => {
                self.vga.io_write(port, value);
            }
            0x0200..=0x0207 => self.gameport.write(self.current_cycle),
            0x0220..=0x022F | 0x0388..=0x038B => self.sound_io_write(port, value),
            0x0330..=0x0331 => self.mpu_io_write(port, value),
            0x03F8..=0x03FF => self.serial_io_write(port, value),
            0x01F0..=0x01F7 | 0x03F6 => self.ide_io_write(port, value),
            0x0170..=0x0177 | 0x0376 => self.ide_secondary_io_write(port, value),
            0x03F0..=0x03F5 | 0x03F7 => self.fdc_io_write(port, value),
            // Absent serial UARTs: COM2 (0x2F8), COM3 (0x3E8) and COM4 (0x2E8).
            // Only COM1 is fitted.
            0x02F8..=0x02FF | 0x03E8..=0x03EF | 0x02E8..=0x02EF => {}
            // IBM 8514/A and S3 accelerator sparse register file (repeats every
            // 0x400 with the low 10 bits fixed at 0x2E8/0x2E9).
            port if port & 0x03FF == 0x02E8 || port & 0x03FF == 0x02E9 => {}
            // Absent parallel ports LPT1 (0x378) and LPT2 (0x278).
            0x0378..=0x037A | 0x0278..=0x027A => {}
            // UART-shaped presence scans at the non-standard bases 0x2F0, 0x5F0
            // and 0x6F0 used by optional multi-I/O / DOS-V add-on cards.
            0x02F0..=0x02F7 | 0x05F0..=0x05F7 | 0x06F0..=0x06F7 => {}
            // Absent IBM XGA: instance register banks at 0x2100 + (instance << 4).
            0x2100..=0x217F => {}
            // DOS/V display-adapter presence probe for an unfitted PS/55 adapter.
            0x1160 => {}
            _ => {
                self.log_unhandled_write(port, value);
                handled = false;
            }
        }
        handled
    }
}
