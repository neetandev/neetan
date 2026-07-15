//! PC-88VA2 16-bit I/O write dispatch.

use device::i8253_pit::{PIT_FLAG_I, WriteResult};

use super::Pc88VaBus;

impl<T: common::TraceSink> Pc88VaBus<T> {
    /// Writes an I/O byte and reports whether the hardware decoded the port.
    pub(crate) fn io_write(&mut self, port: u16, value: u8) -> bool {
        match port {
            // 8259 PIC: master at 0x188/0x18A, slave at 0x184/0x186.
            0x184 => self.pic.write_port0(1, value),
            0x186 => self.pic.write_port2(1, value),
            0x188 => self.pic.write_port0(0, value),
            0x18A => self.pic.write_port2(0, value),

            // 8253 PIT counters / control.
            0x1A0 => self.write_pit_counter(0, value),
            0x1A2 => self.write_pit_counter(1, value),
            0x1A4 => self.write_pit_counter(2, value),
            0x1A6 => self.write_pit_control(value),

            // General-purpose timer 3 (TCU) control: periodic slave IRQ 13.
            0x1A8 => self.write_timer3_ctrl(value),

            // Sound board 2 (YM2608 / OPNA).
            0x044 => self.write_opn_address(value),
            0x045 => self.write_opn_data(value),
            0x046 => self.write_opn_address_hi(value),
            0x047 => self.write_opn_data_hi(value),
            // OPNA register-access wait control: accepted, no timing penalty.
            0x19C | 0x19E => {}

            // uPD71071 DMA controller (channel 2 serves the main-CPU FDC).
            0x160..=0x16F => self.dmac.write(port, value),

            // uPD765A FDC, direct main-CPU access. Control ports 0x1B0-0x1B6
            // select mode/density/motor/reset; 0x1BA is the data register.
            0x1B0 => self.write_fdc_main_mode(value),
            0x1B2 => self.write_fdc_main_control0(value),
            0x1B4 => self.write_fdc_main_control1(value),
            0x1B6 => self.write_fdc_main_control2(value),
            0x1BA => self.write_fdc_data(value),

            // Kanji / CGROM access window.
            0x14C => self.cgrom.write_addr_low(value),
            0x14D => self.cgrom.write_addr_high(value),
            0x14E => self.write_cgrom_data(value),
            0x14F => self.cgrom.write_row(value),

            // TSP: command and parameter ports.
            0x142 => {
                let effect = self.tsp.write_command(value);
                self.apply_tsp_mem_effect(effect);
            }
            0x146 => {
                let effect = self.tsp.write_parameter(value);
                self.apply_tsp_mem_effect(effect);
            }

            // Video controller registers, framebuffer descriptors, and palette.
            0x030 | 0x100..=0x148 | 0x200..=0x27F | 0x300..=0x33F => {
                self.video.write(port, value);
            }

            // Super Graphic Processor (SGP) registers.
            0x500..=0x506 => self.sgp_io_write(port, value),

            // Graphics access controller (GACTRLVA).
            0x510..=0x5A3 => self.gactrlva.io_write(port, value),

            // System-memory bank select (with the GMSP / graphics-reset side effect).
            0x153 => self.write_sysm_bank_io(value),

            // SYSPORTVA system ports.
            0x010 => {
                self.sysport.port010 = value;
                self.rtc_strobe();
            }
            0x032 => {
                // System port 0x032 bit 7 is the FM interrupt mask (set = masked).
                self.sysport.port032 = value & 0xBF;
                self.recompute_sound_irq();
            }
            0x040 => {
                self.sysport.port040 = value;
                self.rtc_strobe();
                // Bit 6 is the mouse strobe; an edge advances the nibble machine.
                self.mouse_strobe(value);
            }
            0x190 => self.sysport.port190 = value & 0x1D,
            0x1C6 => self.write_mode_switch(value),
            0x1CD => self.sysport.c = value,
            0x1CF => self.write_sysport_c_bit(value),

            // PPI mailbox, host side: A->sub port B, B->sub port A, C/control resync.
            0xFC => {
                self.ppi_link.write_main(0, value);
            }
            0xFD => {
                self.ppi_link.write_main(1, value);
            }
            0xFE | 0xFF => {
                if self.ppi_link.write_main((port & 0x03) as u8, value) {
                    self.arm_ppi_resync();
                }
            }

            _ => {
                if !self.memory.io_write_byte(port, value) {
                    return false;
                }
            }
        }
        true
    }

    fn write_pit_counter(&mut self, channel: usize, value: u8) {
        let result = self.pit.write_counter(channel, value);
        if result == WriteResult::Skip {
            return;
        }

        let mode = (self.pit.channels[channel].ctrl >> 1) & 7;
        let is_subsequent = result == WriteResult::SubsequentLoad;
        let is_periodic = mode == 2 || mode == 3;

        if is_subsequent && is_periodic {
            // In modes 2/3, a subsequent load is deferred until terminal count.
            self.pit.channels[channel].reload_pending = Some(self.pit.channels[channel].value);
            return;
        }

        self.pit.channels[channel].last_load_cycle = self.current_cycle;

        if channel == 0 {
            self.pic.clear_irq(0);
            self.pit.channels[0].flag |= PIT_FLAG_I;
            self.schedule_pit_timer0();
            self.update_next_event_cycle();
        }
    }

    fn write_pit_control(&mut self, value: u8) {
        let channel = ((value >> 6) & 3) as usize;
        if channel >= 3 {
            return;
        }
        let is_mode_set = value & 0x30 != 0;
        let pit_clock_hz = self.pit_clock_hz();
        self.pit.write_control(
            channel,
            value,
            self.current_cycle,
            self.clocks.main_clock_hz,
            pit_clock_hz,
        );
        if channel == 0 && is_mode_set {
            self.pic.clear_irq(0);
            self.pit.channels[0].flag |= PIT_FLAG_I;
        }
    }

    /// Recomputes the uPD4990A chip input from the 0x010 and 0x040 latches and
    /// strobes the RTC. The combined byte matches the chip's expected layout:
    /// bits 0-2 = command, bit3 = STB, bit4 = CLK, bit5 = data.
    fn rtc_strobe(&mut self) {
        let port010 = self.sysport.port010;
        let port040 = self.sysport.port040;
        let combined = (port010 & 0x07) | ((port010 & 0x08) << 2) | ((port040 & 0x06) << 2);
        let host_time = (self.host_date_time_provider)().to_bcd_bytes();
        self.rtc.write_port(combined, &host_time);
    }

    fn write_mode_switch(&mut self, value: u8) {
        self.sysport.modesw = if value & 0x01 != 0 { 0xFFFE } else { 0xFFFD };
        if value & 0x02 != 0 {
            self.sysport.a |= 0x20;
        } else {
            self.sysport.a &= !0x20;
        }
    }

    fn write_sysport_c_bit(&mut self, value: u8) {
        if value & 0xF0 != 0 {
            return;
        }
        let bit = 1u8 << (value >> 1);
        if value & 0x01 != 0 {
            self.sysport.c |= bit;
        } else {
            self.sysport.c &= !bit;
        }
    }
}
