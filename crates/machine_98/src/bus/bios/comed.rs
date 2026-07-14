//! INT 19h COMED RS-232C buffered serial adapter service.

use common::Cpu;

use super::super::Pc9801Bus;
use crate::Tracing;

impl<T: Tracing> Pc9801Bus<T> {
    pub(super) fn hle_int19h(&mut self, cpu: &mut impl Cpu) {
        match cpu.ah() {
            0x00 | 0x01 => {
                if cpu.ah() == 0x01 {
                    self.int19h_init_flow(cpu);
                } else {
                    self.int19h_init(cpu);
                }
            }
            0x02..=0x06 => {
                if !self.int19h_is_initialized() {
                    cpu.set_ah(0x01);
                    return;
                }
                match cpu.ah() {
                    0x02 => self.int19h_rx_count(cpu),
                    0x03 => self.int19h_send(cpu),
                    0x04 => {
                        if self.int19h_receive(cpu) {
                            return;
                        }
                    }
                    0x05 => self.int19h_cmd_output(cpu),
                    0x06 => self.int19h_status(cpu),
                    _ => unreachable!(),
                }
                // BOVF check at function exit: if buffer overflow occurred,
                // clear the flag and return AH=2.
                let buf_base = self.int19h_get_buf_base();
                let flag = self.read_mem_byte(buf_base + 0x02);
                if flag & 0x20 != 0 {
                    self.write_mem_byte(buf_base + 0x02, flag & !0x20);
                    cpu.set_ah(0x02);
                }
            }
            _ => {}
        }
    }

    fn int19h_get_buf_base(&self) -> u32 {
        let offset = self.ram_read_u16(0x0556);
        let segment = self.ram_read_u16(0x0558);
        (u32::from(segment) << 4).wrapping_add(u32::from(offset))
    }

    fn int19h_is_initialized(&mut self) -> bool {
        let buf_base = self.int19h_get_buf_base();
        if buf_base == 0 {
            return false;
        }
        let flag = self.read_mem_byte(buf_base + 0x02);
        flag & 0x80 != 0
    }

    fn int19h_init(&mut self, cpu: &mut impl Cpu) {
        let buf_seg = cpu.es();
        let buf_off = cpu.di();
        let buf_base = (u32::from(buf_seg) << 4).wrapping_add(u32::from(buf_off));
        let buf_size = cpu.dx();

        // Program PIT channel 2 for RS-232C baud rate generation.
        #[rustfmt::skip]
        const RS_SPEED: [u16; 20] = [
            // 8MHz lineage (PIT clock ≈ 1.9968 MHz)
            0x0680, 0x0340, 0x01A0, 0x00D0,
            0x0068, 0x0034, 0x001A, 0x000D,
            // 10MHz lineage (PIT clock ≈ 2.4576 MHz)
            0x0800, 0x0400, 0x0200, 0x0100,
            0x0080, 0x0040, 0x0020, 0x0010,
            0x0008, 0x0004, 0x0002, 0x0001,
        ];
        let mut speed = cpu.al();
        if speed >= 8 {
            speed = 4; // Default to 1200 bps.
        }
        let is_8mhz = self.memory.state.ram[0x0501] & 0x80 != 0;
        let speed_idx = if is_8mhz {
            speed as usize
        } else {
            speed as usize + 8
        };
        let divisor = RS_SPEED[speed_idx];
        self.pit.write_control(
            2,
            0xB6,
            self.current_cycle,
            self.clocks.cpu_clock_hz,
            self.clocks.pit_clock_hz,
        );
        self.pit.write_counter(2, divisor as u8);
        self.pit.write_counter(2, (divisor >> 8) as u8);

        // Store buffer pointer in BDA.
        self.ram_write_u16(0x0556, buf_off);
        self.ram_write_u16(0x0558, buf_seg);

        // Initialize control block fields.
        let data_start = buf_off + 0x14; // After RSBIOS header
        let data_end = data_start + buf_size;

        // R_INT (offset 0x00) and R_BFLG (offset 0x01): clear stale state.
        self.write_mem_byte(buf_base, 0x00);
        self.write_mem_byte(buf_base + 0x01, 0x00);

        // R_FLAG (offset 0x02): set AH<<4, only add RFLAG_INIT if IR bit clear.
        let cmd = cpu.cl();
        let mut flag = cpu.ah() << 4;
        // Clear sysport RS-232C interrupt source bits (bits 0-2).
        self.system_ppi.state.port_c &= !0x07;
        if cmd & 0x40 == 0 {
            // IR bit clear: mark initialized.
            flag |= 0x80;
            if cmd & 0x04 != 0 {
                // RXE set: enable RxRDY interrupt via sysport and unmask IRQ 4.
                self.system_ppi.state.port_c |= 0x01;
                self.pic.state.chips[0].imr &= !0x10;
                self.pic.invalidate_irq_cache();
            }
        }
        self.write_mem_byte(buf_base + 0x02, flag);

        // R_CMD (offset 0x03).
        self.write_mem_byte(buf_base + 0x03, cmd);
        // R_STIME (offset 0x04) = BH, default 0x04 if zero.
        let stime = if cpu.bh() == 0 { 0x04 } else { cpu.bh() };
        self.write_mem_byte(buf_base + 0x04, stime);
        // R_RTIME (offset 0x05) = BL, default 0x40 if zero.
        let rtime = if cpu.bx() as u8 == 0 {
            0x40
        } else {
            cpu.bx() as u8
        };
        self.write_mem_byte(buf_base + 0x05, rtime);

        // R_XOFF (offset 0x06) = buf_size / 8.
        let xoff = buf_size / 8;
        self.write_mem_word(buf_base + 0x06, xoff);
        // R_XON (offset 0x08) = XOFF + buf_size / 4.
        let xon = xoff + buf_size / 4;
        self.write_mem_word(buf_base + 0x08, xon);

        // R_HEADP (offset 0x0A).
        self.write_mem_word(buf_base + 0x0A, data_start);
        // R_TAILP (offset 0x0C).
        self.write_mem_word(buf_base + 0x0C, data_end);
        // R_CNT (offset 0x0E).
        self.write_mem_word(buf_base + 0x0E, 0x0000);
        // R_PUTP (offset 0x10).
        self.write_mem_word(buf_base + 0x10, data_start);
        // R_GETP (offset 0x12).
        self.write_mem_word(buf_base + 0x12, data_start);

        // Program the serial UART command register.
        self.serial.write_command(cmd);

        cpu.set_ah(0x00);
    }

    fn int19h_init_flow(&mut self, cpu: &mut impl Cpu) {
        // Same as AH=00h but with flow control flag.
        self.int19h_init(cpu);

        let buf_base = self.int19h_get_buf_base();
        // Set RFLAG_XON bit (bit 4).
        let flag = self.read_mem_byte(buf_base + 0x02);
        self.write_mem_byte(buf_base + 0x02, flag | 0x10);
    }

    fn int19h_rx_count(&mut self, cpu: &mut impl Cpu) {
        let buf_base = self.int19h_get_buf_base();
        let count = self.read_mem_word(buf_base + 0x0E);
        cpu.set_cx(count);
        cpu.set_ah(0x00);
    }

    fn int19h_send(&mut self, cpu: &mut impl Cpu) {
        // Write data byte to serial data port (port 0x30).
        self.serial.write_data(cpu.al());
        cpu.set_ah(0x00);
    }

    fn int19h_receive(&mut self, cpu: &mut impl Cpu) -> bool {
        let buf_base = self.int19h_get_buf_base();
        let buf_seg = self.ram_read_u16(0x0558);
        let count = self.read_mem_word(buf_base + 0x0E);

        if count == 0 {
            cpu.set_ah(0x03);
            return false;
        }

        let r_getp = self.read_mem_word(buf_base + 0x12);
        let get_addr = (u32::from(buf_seg) << 4).wrapping_add(u32::from(r_getp));
        let entry = self.read_mem_word(get_addr);

        // Advance get pointer with wrap.
        let r_tailp = self.read_mem_word(buf_base + 0x0C);
        let r_headp = self.read_mem_word(buf_base + 0x0A);
        let mut new_getp = r_getp + 2;
        if new_getp >= r_tailp {
            new_getp = r_headp;
        }

        let new_count = count - 1;
        self.write_mem_word(buf_base + 0x0E, new_count);
        self.write_mem_word(buf_base + 0x12, new_getp);

        // XON flow control: send XON if XOFF was active and count dropped below threshold.
        let flag = self.read_mem_byte(buf_base + 0x02);
        if (flag & 0x08) != 0 && new_count < self.read_mem_word(buf_base + 0x06) {
            self.serial.write_data(0x11); // XON
            self.write_mem_byte(buf_base + 0x02, (flag & !0x08) & !0x20);
        } else {
            self.write_mem_byte(buf_base + 0x02, flag & !0x20);
        }

        // Return full entry word in CX (CL=status/error, CH=data).
        cpu.set_cx(entry);
        cpu.set_ah(0x00);
        true
    }

    fn int19h_cmd_output(&mut self, cpu: &mut impl Cpu) {
        let buf_base = self.int19h_get_buf_base();
        let cmd = cpu.al();

        self.serial.write_command(cmd);

        let flag = self.read_mem_byte(buf_base + 0x02);
        if cmd & 0x40 != 0 {
            // IR (internal reset): clear INIT flag, disable RxRDY, mask IRQ 4.
            self.write_mem_byte(buf_base + 0x02, flag & !0x80);
            self.system_ppi.state.port_c &= !0x01;
            self.pic.state.chips[0].imr |= 0x10;
        } else if cmd & 0x04 == 0 {
            // RXE clear: disable RxRDY, mask IRQ 4.
            self.system_ppi.state.port_c &= !0x01;
            self.pic.state.chips[0].imr |= 0x10;
        } else {
            // RXE set: enable RxRDY, unmask IRQ 4.
            self.system_ppi.state.port_c |= 0x01;
            self.pic.state.chips[0].imr &= !0x10;
        }
        self.pic.invalidate_irq_cache();

        self.write_mem_byte(buf_base + 0x03, cmd);
        cpu.set_ah(0x00);
    }

    fn int19h_status(&mut self, cpu: &mut impl Cpu) {
        cpu.set_ch(self.serial.read_status());
        cpu.set_cl(self.system_ppi.read_rs232c_status() | self.rtc.cdat());
        cpu.set_ah(0x00);
    }
}
