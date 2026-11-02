//! INT 0Ch RS-232C receive interrupt handler and the receive buffer layout it shares with INT 19h.
//!
//! The layout of the buffer control block follows the PC-9800 BIOS databook (chapter 11, RS-232C
//! BIOS, receive buffer table). Each entry of the ring buffer is one word with the 8251A status in
//! the low byte and the received character in the high byte. INT 19h function 04h returns this
//! word in CX with the character in CH and the status in CL.

use common::Cpu;

use super::{super::Pc9801Bus, real_mode_address};
use crate::TraceSink;

/// Work area word with the offset of the channel 0 buffer control block (RS_OFST_ADR).
pub(super) const RS_BUFFER_OFFSET_ADDRESS: usize = 0x0556;
/// Work area word with the segment of the channel 0 buffer control block (RS_SEG_ADR).
pub(super) const RS_BUFFER_SEGMENT_ADDRESS: usize = 0x0558;
/// Work area byte with the SI/SO conversion state of each channel (RS_S_FLAG).
const RS_SHIFT_FLAG_ADDRESS: usize = 0x055B;
/// Work area byte with the DEL code handling of each channel (RS_D_FLAG).
const RS_DEL_FLAG_ADDRESS: usize = 0x05C1;

/// RS_S_FLAG bit: SI/SO conversion is enabled on channel 0.
const SHIFT_CONVERSION_CHANNEL0: u8 = 0x80;
/// RS_S_FLAG bit: channel 0 is in the shift out (SO) state.
const SHIFT_OUT_STATE_CHANNEL0: u8 = 0x10;
/// RS_D_FLAG bit: channel 0 handles DEL as memory switch 3 bit 7 selects.
const DEL_HANDLING_CHANNEL0: u8 = 0x01;

/// Text VRAM offset of memory switch 3.
const MEMORY_SWITCH_3_OFFSET: usize = 0x3FEA;
/// Memory switch 3 bit: a received DEL code is stored as NUL.
const MEMORY_SWITCH_3_DEL_AS_NUL: u8 = 0x80;

/// Control block offset of the interface field (INT).
pub(super) const BLOCK_INTERFACE: u32 = 0x00;
/// Control block offset of the BASIC flag (BFLG).
pub(super) const BLOCK_BASIC_FLAG: u32 = 0x01;
/// Control block offset of the internal flag field (FLAG).
pub(super) const BLOCK_FLAG: u32 = 0x02;
/// Control block offset of the saved 8251A command (CMD).
pub(super) const BLOCK_COMMAND: u32 = 0x03;
/// Control block offset of the transmit timeout in 500 ms units (STIME).
pub(super) const BLOCK_SEND_TIMEOUT: u32 = 0x04;
/// Control block offset of the receive timeout in 500 ms units (RTIME).
pub(super) const BLOCK_RECEIVE_TIMEOUT: u32 = 0x05;
/// Control block offset of the low fill level that sends CTRL-Q (XOFF word).
pub(super) const BLOCK_RESUME_LEVEL: u32 = 0x06;
/// Control block offset of the high fill level that sends CTRL-S (XON word).
pub(super) const BLOCK_STOP_LEVEL: u32 = 0x08;
/// Control block offset of the first byte of the ring buffer (HEADP).
pub(super) const BLOCK_HEAD_POINTER: u32 = 0x0A;
/// Control block offset of the address after the ring buffer (TAILP).
pub(super) const BLOCK_TAIL_POINTER: u32 = 0x0C;
/// Control block offset of the number of stored entries (CNT).
pub(super) const BLOCK_COUNT: u32 = 0x0E;
/// Control block offset of the next free entry (PUTP).
pub(super) const BLOCK_PUT_POINTER: u32 = 0x10;
/// Control block offset of the oldest stored entry (GETP).
pub(super) const BLOCK_GET_POINTER: u32 = 0x12;
/// Offset of the ring buffer from the start of the control block.
pub(super) const BLOCK_SIZE: u16 = 0x14;

/// Interface field bit: a receive interrupt occurred.
const INTERFACE_RECEIVED: u8 = 0x80;

/// FLAG bit: the 8251A was initialized by the BIOS (INIT).
pub(super) const FLAG_INITIALIZED: u8 = 0x80;
/// FLAG bit: the ring buffer is full (BFULL).
pub(super) const FLAG_BUFFER_FULL: u8 = 0x40;
/// FLAG bit: a character was lost because the ring buffer was full (BOVF).
pub(super) const FLAG_OVERFLOW: u8 = 0x20;
/// FLAG bit: receive flow control with CTRL-S and CTRL-Q is active.
pub(super) const FLAG_FLOW_CONTROL: u8 = 0x10;
/// FLAG bit: CTRL-S was sent and CTRL-Q is still due.
pub(super) const FLAG_STOP_SENT: u8 = 0x08;

/// Size in bytes of one ring buffer entry.
pub(super) const ENTRY_SIZE: u16 = 2;

/// DC3 (CTRL-S) asks the remote station to stop sending.
pub(super) const CONTROL_STOP: u8 = 0x13;
/// DC1 (CTRL-Q) asks the remote station to resume sending.
pub(super) const CONTROL_RESUME: u8 = 0x11;

/// Shift out code: the following characters belong to the upper half of the 8-bit code.
const SHIFT_OUT_CODE: u8 = 0x0E;
/// Shift in code: the following characters belong to the lower half of the 8-bit code.
const SHIFT_IN_CODE: u8 = 0x0F;
/// First character code affected by the SI/SO state.
const FIRST_PRINTABLE_CODE: u8 = 0x20;
/// DEL code without its eighth bit.
const DEL_CODE: u8 = 0x7F;

/// 8251A status bits stored with each character: DSR, BREAK, FE, OE, PE and TxEMPTY.
const USART_STATUS_STORED_BITS: u8 = 0xFC;
/// System port B bit: CS# (clear to send) of the RS-232C line.
const SYSTEM_PORT_CS: u8 = 0x40;
/// System port B bit: CD# (carrier detect) of the RS-232C line.
const SYSTEM_PORT_CD: u8 = 0x20;
/// Stored status bit that carries CS#.
const ENTRY_STATUS_CS: u8 = 0x02;
/// Stored status bit that carries CD#.
const ENTRY_STATUS_CD: u8 = 0x01;

/// IRQ line of the built-in RS-232C port.
pub(super) const RS232C_IRQ: u8 = 4;
/// OCW2 non-specific end of interrupt command.
const PIC_NONSPECIFIC_EOI: u8 = 0x20;

impl<T: TraceSink> Pc9801Bus<T> {
    /// Returns the linear address of the channel 0 buffer control block or `None` when unset.
    pub(super) fn rs232c_control_block(&self) -> Option<u32> {
        let offset = self.ram_read_u16(RS_BUFFER_OFFSET_ADDRESS);
        let segment = self.ram_read_u16(RS_BUFFER_SEGMENT_ADDRESS);
        if offset == 0 && segment == 0 {
            return None;
        }
        Some(real_mode_address(segment, offset))
    }

    /// INT 0Ch: stores the received character into the RS-232C ring buffer.
    pub(super) fn hle_int0ch(&mut self, _cpu: &mut impl Cpu) {
        let (received, clear_irq, retrigger_irq) = self.serial.read_data();
        if clear_irq {
            self.clear_pic_irq(RS232C_IRQ);
        }
        if retrigger_irq {
            self.raise_pic_irq(RS232C_IRQ);
        }
        let status = self.rs232c_entry_status();

        if let Some(block) = self.rs232c_control_block() {
            let buffer_full = self.read_mem_byte(block + BLOCK_FLAG) & FLAG_BUFFER_FULL != 0;
            let character = if buffer_full {
                Some(received)
            } else {
                self.rs232c_translate_received(received)
            };
            if let Some(character) = character {
                self.rs232c_store_entry(block, character, status);
            }
        }

        self.pic.write_port0(0, PIC_NONSPECIFIC_EOI);
    }

    /// Builds the status byte stored with a character from the 8251A and the system port.
    fn rs232c_entry_status(&self) -> u8 {
        let line = self.system_ppi.read_rs232c_status();
        let mut status = self.serial.read_status() & USART_STATUS_STORED_BITS;
        if line & SYSTEM_PORT_CS != 0 {
            status |= ENTRY_STATUS_CS;
        }
        if line & SYSTEM_PORT_CD != 0 {
            status |= ENTRY_STATUS_CD;
        }
        status
    }

    /// Applies the SI/SO and DEL rules. Returns `None` for a character that is not stored.
    fn rs232c_translate_received(&mut self, received: u8) -> Option<u8> {
        let shift_flag = self.memory.state.ram[RS_SHIFT_FLAG_ADDRESS];
        let mut character = received;
        if shift_flag & SHIFT_CONVERSION_CHANNEL0 != 0 {
            match character {
                SHIFT_OUT_CODE => {
                    self.memory.state.ram[RS_SHIFT_FLAG_ADDRESS] =
                        shift_flag | SHIFT_OUT_STATE_CHANNEL0;
                    return None;
                }
                SHIFT_IN_CODE => {
                    self.memory.state.ram[RS_SHIFT_FLAG_ADDRESS] =
                        shift_flag & !SHIFT_OUT_STATE_CHANNEL0;
                    return None;
                }
                code if code >= FIRST_PRINTABLE_CODE => {
                    character = if shift_flag & SHIFT_OUT_STATE_CHANNEL0 != 0 {
                        code | 0x80
                    } else {
                        code & 0x7F
                    };
                }
                _ => {}
            }
        }

        let del_as_nul = self.memory.state.ram[RS_DEL_FLAG_ADDRESS] & DEL_HANDLING_CHANNEL0 != 0
            && self.memory.state.text_vram[MEMORY_SWITCH_3_OFFSET] & MEMORY_SWITCH_3_DEL_AS_NUL
                != 0;
        if del_as_nul && character & 0x7F == DEL_CODE {
            character = 0x00;
        }
        Some(character)
    }

    /// Appends one entry to the ring buffer and updates the flow control state.
    fn rs232c_store_entry(&mut self, block: u32, character: u8, status: u8) {
        let mut flag = self.read_mem_byte(block + BLOCK_FLAG);

        if flag & FLAG_BUFFER_FULL != 0 {
            flag |= FLAG_OVERFLOW;
        } else {
            let segment = self.ram_read_u16(RS_BUFFER_SEGMENT_ADDRESS);
            let put_pointer = self.read_mem_word(block + BLOCK_PUT_POINTER);
            let entry = u16::from(status) | (u16::from(character) << 8);
            self.write_mem_word(real_mode_address(segment, put_pointer), entry);

            let mut next_put = put_pointer.wrapping_add(ENTRY_SIZE);
            if next_put >= self.read_mem_word(block + BLOCK_TAIL_POINTER) {
                next_put = self.read_mem_word(block + BLOCK_HEAD_POINTER);
            }
            self.write_mem_word(block + BLOCK_PUT_POINTER, next_put);

            let count = self.read_mem_word(block + BLOCK_COUNT).wrapping_add(1);
            self.write_mem_word(block + BLOCK_COUNT, count);

            if next_put == self.read_mem_word(block + BLOCK_GET_POINTER) {
                flag |= FLAG_BUFFER_FULL;
            }

            let flow_waiting = flag & (FLAG_FLOW_CONTROL | FLAG_STOP_SENT) == FLAG_FLOW_CONTROL;
            if flow_waiting && count >= self.read_mem_word(block + BLOCK_STOP_LEVEL) {
                self.serial.write_data(CONTROL_STOP);
                flag |= FLAG_STOP_SENT;
            }
        }

        self.write_mem_byte(block + BLOCK_FLAG, flag);
        let interface = self.read_mem_byte(block + BLOCK_INTERFACE);
        self.write_mem_byte(block + BLOCK_INTERFACE, interface | INTERFACE_RECEIVED);
    }
}
