//! INT 09h keyboard interrupt + INT 18h AH=00..05 keyboard service handlers.

use common::{Cpu, MachineModel};

use super::{
    super::{KEYBOARD_ROM_OFFSET_F, KEYBOARD_ROM_OFFSET_VM, Pc9801Bus},
    iret_stack_base,
};
use crate::TraceSink;

/// Key code of the STOP key.
const STOP_KEY_CODE: u8 = 0x60;

/// Key code of the COPY key.
const COPY_KEY_CODE: u8 = 0x61;

/// Vector the INT 09h stub raises for a STOP make code.
const STOP_KEY_INTERRUPT_VECTOR: u8 = 0x06;

/// Vector the INT 09h stub raises for a COPY make code.
const COPY_KEY_INTERRUPT_VECTOR: u8 = 0x05;

/// INT 09h stub return value that raises no key interrupt.
const NO_KEY_INTERRUPT: u8 = 0x00;

impl<T: TraceSink> Pc9801Bus<T> {
    /// INT 09h keyboard interrupt. Buffers the scan code the ROM stub read
    /// from port 41h into AL and returns in AL the vector the stub raises
    /// after the EOI: 06h for a STOP make code, 05h for a COPY make code and
    /// 00h for every other code.
    pub(super) fn hle_int09h(&mut self, cpu: &mut impl Cpu) {
        let key_interrupt_vector = match self.buffer_keyboard_scan_code(cpu.al()) {
            (STOP_KEY_CODE, false) => STOP_KEY_INTERRUPT_VECTOR,
            (COPY_KEY_CODE, false) => COPY_KEY_INTERRUPT_VECTOR,
            _ => NO_KEY_INTERRUPT,
        };
        cpu.set_al(key_interrupt_vector);
    }

    fn read_pending_keyboard_scan_code(&mut self) -> Option<u8> {
        if !self.keyboard.has_rx_ready() {
            return None;
        }

        let (raw_code, clear_irq, retrigger_irq) = self.keyboard.read_data();
        if clear_irq {
            self.clear_pic_irq(1);
        }
        if retrigger_irq {
            self.raise_pic_irq(1);
        }
        Some(raw_code)
    }

    fn poll_keyboard_scan_codes(&mut self) {
        while let Some(raw_code) = self.read_pending_keyboard_scan_code() {
            self.buffer_keyboard_scan_code(raw_code);
        }
    }

    fn buffer_keyboard_scan_code(&mut self, raw_code: u8) -> (u8, bool) {
        let key_code = raw_code & 0x7F;
        let is_release = raw_code & 0x80 != 0;
        let group = (key_code >> 3) as usize;
        let bit = 1u8 << (key_code & 0x07);
        let key_state_addr = 0x052A + group;

        if is_release {
            // Key release: clear key status bit.
            self.memory.state.ram[key_state_addr] &= !bit;

            // Update shift state for modifier key releases.
            if (0x70..0x75).contains(&key_code) || key_code == 0x7D {
                if key_code == 0x70 || key_code == 0x7D {
                    self.memory.state.ram[0x053A] &= !0x01;
                } else {
                    self.memory.state.ram[0x053A] &= !bit;
                }
                self.update_shift_key();
            }
        } else {
            // Key press: set key status bit.
            self.memory.state.ram[key_state_addr] |= bit;

            // Update shift state for modifier key presses.
            if key_code == 0x70 || key_code == 0x7D {
                self.memory.state.ram[0x053A] |= 0x01;
                self.update_shift_key();
            } else if (0x71..0x75).contains(&key_code) {
                self.memory.state.ram[0x053A] |= bit;
                self.update_shift_key();
            } else {
                // Non-modifier key press: translate and buffer.
                let count = self.memory.state.ram[0x0528];
                if count < 0x10 {
                    let code = self.translate_key(key_code);
                    if code != 0xFFFF {
                        self.memory.state.ram[0x0528] = count + 1;
                        let tail = self.ram_read_u16(0x0526) as usize;
                        self.memory.state.ram[tail] = code as u8;
                        self.memory.state.ram[tail + 1] = (code >> 8) as u8;
                        let mut new_tail = tail + 2;
                        if new_tail >= 0x0522 {
                            new_tail = 0x0502;
                        }
                        self.ram_write_u16(0x0526, new_tail as u16);
                    }
                }
            }
        }

        (key_code, is_release)
    }

    fn update_shift_key(&mut self) {
        let shift_sts = self.memory.state.ram[0x053A];

        let base = if shift_sts & 0x10 != 0 {
            7u16
        } else if shift_sts & 0x08 != 0 {
            6u16
        } else {
            let mut b = (shift_sts & 0x07) as u16;
            if b >= 6 {
                b -= 2;
            }
            b
        };
        let table_base = match self.machine_model {
            MachineModel::PC9801F => KEYBOARD_ROM_OFFSET_F as u16,
            MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => KEYBOARD_ROM_OFFSET_VM as u16,
        };
        let table_offset = table_base + base * 0x60;
        self.ram_write_u16(0x0522, table_offset);
    }

    fn translate_key(&self, key_code: u8) -> u16 {
        let table_offset = self.ram_read_u16(0x0522) as u32;
        let table_base = 0xFD800 + table_offset;

        if key_code <= 0x51 {
            if key_code == 0x51 || key_code == 0x35 || key_code == 0x3E {
                let val = self.read_byte_direct(table_base + key_code as u32);
                if val == 0xFF {
                    0xFFFF
                } else {
                    (val as u16) << 8
                }
            } else {
                let val = self.read_byte_direct(table_base + key_code as u32);
                if val == 0xFF {
                    0xFFFF
                } else {
                    val as u16 + ((key_code as u16) << 8)
                }
            }
        } else if key_code < 0x60 {
            if key_code == 0x5E { 0xAE00 } else { 0xFFFF }
        } else if (0x62..0x70).contains(&key_code) {
            let val = self.read_byte_direct(table_base + (key_code - 0x0C) as u32);
            if val == 0xFF {
                0xFFFF
            } else {
                (val as u16) << 8
            }
        } else {
            0xFFFF
        }
    }

    pub(super) fn int18h_key_read(&mut self, cpu: &mut impl Cpu) {
        self.poll_keyboard_scan_codes();
        let count = self.memory.state.ram[0x0528];
        if count == 0 {
            // Block until a key is available by rewinding the caller's return IP
            // in the IRET frame to re-execute the INT 18H instruction (2 bytes: CD 18).
            let base = iret_stack_base(cpu);
            let caller_ip = self.read_mem_word(base);
            self.write_mem_word(base, caller_ip.wrapping_sub(2));

            // Ensure IF is set in the IRET flags so hardware interrupts (especially
            // IRQ 1 keyboard) can fire during the wait loop.
            let flags = self.read_mem_word(base + 4);
            self.write_mem_word(base + 4, flags | 0x0200);

            // Burn enough cycles so the timeslice ends and the timer interrupt can fire.
            self.pending_wait_cycles += 2000;
            return;
        }

        let head = self.ram_read_u16(0x0524) as usize;
        let entry = self.ram_read_u16(head);

        // Advance head with wrap.
        let mut new_head = head + 2;
        if new_head >= 0x0522 {
            new_head = 0x0502;
        }
        self.ram_write_u16(0x0524, new_head as u16);
        self.memory.state.ram[0x0528] = count - 1;

        cpu.set_ax(entry);
    }

    pub(super) fn int18h_buffer_sense(&mut self, cpu: &mut impl Cpu) {
        self.poll_keyboard_scan_codes();
        let count = self.memory.state.ram[0x0528];
        if count == 0 {
            cpu.set_bh(0x00);
            return;
        }

        // Peek at head entry without removing.
        let head = self.ram_read_u16(0x0524) as usize;
        let entry = self.ram_read_u16(head);

        cpu.set_ax(entry);
        cpu.set_bh(0x01);
    }

    pub(super) fn int18h_shift_status(&mut self, cpu: &mut impl Cpu) {
        let shift = self.memory.state.ram[0x053A];
        cpu.set_al(shift);
    }

    pub(super) fn int18h_kb_init(&mut self, _cpu: &mut impl Cpu) {
        // Clear keyboard buffer and key status area.
        self.memory.state.ram[0x0502..0x0522].fill(0);
        self.memory.state.ram[0x0528..0x053B].fill(0);
        let keyboard_table = match self.machine_model {
            MachineModel::PC9801F => KEYBOARD_ROM_OFFSET_F as u16,
            MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => KEYBOARD_ROM_OFFSET_VM as u16,
        };
        self.ram_write_u16(0x0522, keyboard_table); // KB_SHIFT_TBL
        self.ram_write_u16(0x0524, 0x0502); // KB_BUF_HEAD
        self.ram_write_u16(0x0526, 0x0502); // KB_BUF_TAIL
        self.ram_write_u16(0x05C6, keyboard_table); // KB_CODE_OFF
        self.ram_write_u16(0x05C8, 0xFD80); // KB_CODE_SEG
    }

    pub(super) fn int18h_key_state_sense(&mut self, cpu: &mut impl Cpu) {
        let group = cpu.al() as usize;
        let value = if group < 16 {
            self.memory.state.ram[0x052A + group]
        } else {
            0
        };
        cpu.set_ah(value);
    }

    pub(super) fn int18h_key_code_read(&mut self, cpu: &mut impl Cpu) {
        self.poll_keyboard_scan_codes();
        let count = self.memory.state.ram[0x0528];
        if count == 0 {
            cpu.set_bh(0x00);
            return;
        }

        let head = self.ram_read_u16(0x0524) as usize;
        let entry = self.ram_read_u16(head);

        // Consume from buffer (unlike AH=01h which peeks).
        let mut new_head = head + 2;
        if new_head >= 0x0522 {
            new_head = 0x0502;
        }
        self.ram_write_u16(0x0524, new_head as u16);
        self.memory.state.ram[0x0528] = count - 1;

        cpu.set_ax(entry);
        cpu.set_bh(0x01);
    }
}
