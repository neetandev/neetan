//! INT 0Ch RS-232C serial receive interrupt handler.

use common::Cpu;

use super::super::Pc9801Bus;
use crate::Tracing;

impl<T: Tracing> Pc9801Bus<T> {
    pub(super) fn hle_int0ch(&mut self, _cpu: &mut impl Cpu) {
        let (data, clear_irq, retrigger_irq) = self.serial.read_data();
        if clear_irq {
            self.pic.clear_irq(4);
        }
        if retrigger_irq {
            self.pic.set_irq(4);
        }
        // Include RS-232C signal line status (CI, CS) from sysport port B.
        let status =
            (self.serial.read_status() & 0xFC) | (self.system_ppi.read_rs232c_status() & 0x03);

        // Read the RS-232C buffer control block pointer from BDA.
        let buf_offset = self.ram_read_u16(0x0556);
        let buf_segment = self.ram_read_u16(0x0558);
        let buf_base = (u32::from(buf_segment) << 4).wrapping_add(u32::from(buf_offset));

        if buf_base != 0 {
            let mut flag = self.read_mem_byte(buf_base + 0x02);
            let mut data = data;

            if flag & 0x40 == 0 {
                // SI/SO character set conversion (JIS 7-bit encoding).
                let rs_s_flag = self.memory.state.ram[0x055B];
                if rs_s_flag & 0x80 != 0 {
                    if data >= 0x20 {
                        if rs_s_flag & 0x10 != 0 {
                            data |= 0x80;
                        } else {
                            data &= 0x7F;
                        }
                    } else if data == 0x0E {
                        // SO: set shift-out flag, send EOI, return without buffering.
                        self.memory.state.ram[0x055B] |= 0x10;
                        self.pic.write_port0(0, 0x20);
                        return;
                    } else if data == 0x0F {
                        // SI: clear shift-out flag, send EOI, return without buffering.
                        self.memory.state.ram[0x055B] &= !0x10;
                        self.pic.write_port0(0, 0x20);
                        return;
                    }
                }

                // DEL code handling: convert DEL to NUL if configured.
                if self.memory.state.ram[0x05C1] & 0x01 != 0
                    && (data & 0x7F) == 0x7F
                    && self.memory.state.text_vram[0x3FEA] & 0x80 != 0
                {
                    data = 0;
                }
                // Buffer not full: store data.
                let r_putp = self.read_mem_word(buf_base + 0x10);
                let r_tailp = self.read_mem_word(buf_base + 0x0C);

                // Store (data << 8) | status at put pointer.
                let entry = u16::from(status) | (u16::from(data) << 8);
                let put_addr = (u32::from(buf_segment) << 4).wrapping_add(u32::from(r_putp));
                self.write_mem_word(put_addr, entry);

                // Advance put pointer with wrap.
                let mut new_putp = r_putp + 2;
                if new_putp >= r_tailp {
                    let r_headp = self.read_mem_word(buf_base + 0x0A);
                    new_putp = r_headp;
                }
                self.write_mem_word(buf_base + 0x10, new_putp);

                // Increment counter.
                let r_cnt = self.read_mem_word(buf_base + 0x0E);
                let new_cnt = r_cnt + 1;
                self.write_mem_word(buf_base + 0x0E, new_cnt);

                // Check for buffer full (put pointer caught up to get pointer).
                let r_getp = self.read_mem_word(buf_base + 0x12);
                if new_putp == r_getp {
                    flag |= 0x40; // RFLAG_BFULL
                }

                // XON/XOFF flow control: send XOFF if threshold reached.
                // RFLAG_XON=0x10, RFLAG_XOFF=0x08.
                if (flag & 0x18) == 0x10 {
                    let r_xon = self.read_mem_word(buf_base + 0x08);
                    if new_cnt >= r_xon {
                        self.serial.write_data(0x13); // XOFF
                        flag |= 0x08; // RFLAG_XOFF
                    }
                }
            } else {
                // Buffer full: set overflow flag (RFLAG_BOVF=0x20) in R_CMD (offset 0x03).
                let r_cmd = self.read_mem_byte(buf_base + 0x03);
                self.write_mem_byte(buf_base + 0x03, r_cmd | 0x20);
            }

            // Set interrupt flag (RINT_INT=0x80) in R_INT.
            let r_int = self.read_mem_byte(buf_base);
            self.write_mem_byte(buf_base, r_int | 0x80);

            // Write back updated flag.
            self.write_mem_byte(buf_base + 0x02, flag);
        }

        // Send EOI to master PIC.
        self.pic.write_port0(0, 0x20);
    }
}
