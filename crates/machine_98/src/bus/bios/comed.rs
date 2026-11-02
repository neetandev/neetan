//! INT 19h RS-232C BIOS for channel 0 in single mode.
//!
//! Functions 00h to 06h follow the PC-9800 BIOS databook (chapter 11, RS-232C BIOS) and
//! undoc98 `int19bio.txt`. Received characters reach the ring buffer through INT 0Ch.

use common::Cpu;

use super::{
    super::Pc9801Bus,
    real_mode_address,
    serial_rs232c::{
        BLOCK_BASIC_FLAG, BLOCK_COMMAND, BLOCK_COUNT, BLOCK_FLAG, BLOCK_GET_POINTER,
        BLOCK_HEAD_POINTER, BLOCK_INTERFACE, BLOCK_PUT_POINTER, BLOCK_RECEIVE_TIMEOUT,
        BLOCK_RESUME_LEVEL, BLOCK_SEND_TIMEOUT, BLOCK_SIZE, BLOCK_STOP_LEVEL, BLOCK_TAIL_POINTER,
        CONTROL_RESUME, ENTRY_SIZE, FLAG_BUFFER_FULL, FLAG_FLOW_CONTROL, FLAG_INITIALIZED,
        FLAG_OVERFLOW, FLAG_STOP_SENT, RS_BUFFER_OFFSET_ADDRESS, RS_BUFFER_SEGMENT_ADDRESS,
        RS232C_IRQ,
    },
};
use crate::TraceSink;

/// Function 00h: initialize without flow control.
const FUNCTION_INITIALIZE: u8 = 0x00;
/// Function 01h: initialize with receive flow control.
const FUNCTION_INITIALIZE_FLOW: u8 = 0x01;
/// Function 02h: number of received characters.
const FUNCTION_RECEIVED_COUNT: u8 = 0x02;
/// Function 03h: send one character.
const FUNCTION_SEND: u8 = 0x03;
/// Function 04h: take one character from the receive buffer.
const FUNCTION_RECEIVE: u8 = 0x04;
/// Function 05h: write a command word to the 8251A.
const FUNCTION_COMMAND: u8 = 0x05;
/// Function 06h: read the 8251A and modem status.
const FUNCTION_STATUS: u8 = 0x06;

/// Return code: success.
const RESULT_OK: u8 = 0x00;
/// Return code: the RS-232C BIOS is not initialized.
const RESULT_NOT_INITIALIZED: u8 = 0x01;
/// Return code: the receive buffer overflowed since the last report.
const RESULT_OVERFLOW: u8 = 0x02;
/// Return code: timeout.
const RESULT_TIMEOUT: u8 = 0x03;

/// Transfer rates selected by AL in bits per second.
const TRANSFER_RATES: [u32; 8] = [75, 150, 300, 600, 1200, 2400, 4800, 9600];
/// Transfer rate index used for an unsupported AL value (1200 bps).
const DEFAULT_TRANSFER_RATE_INDEX: usize = 4;

/// 8251A mode word bits 1-0: baud rate factor.
const MODE_BAUD_FACTOR_MASK: u8 = 0x03;
/// 8251A mode word baud rate factor value for x64.
const MODE_BAUD_FACTOR_64: u8 = 0x03;
/// Clock cycles per bit with the x16 factor.
const BAUD_FACTOR_16: u32 = 16;
/// Clock cycles per bit with the x64 factor.
const BAUD_FACTOR_64: u32 = 64;

/// 8251A command bit: receive enable (RXE).
const COMMAND_RECEIVE_ENABLE: u8 = 0x04;
/// 8251A command bit: internal reset (IR).
const COMMAND_INTERNAL_RESET: u8 = 0x40;
/// Dummy byte written three times to bring the 8251A into a known state.
const USART_SYNC_BYTE: u8 = 0x00;

/// Transmit timeout used when BH is zero (1 second).
const DEFAULT_SEND_TIMEOUT: u8 = 0x02;
/// Receive timeout used when BL is zero in normal mode (15 seconds).
const DEFAULT_RECEIVE_TIMEOUT: u8 = 0x1E;

/// PIT counter that clocks the RS-232C 8251A.
const PIT_BAUD_COUNTER: usize = 2;
/// PIT control word: counter 2, low byte then high byte, mode 3, binary.
const PIT_BAUD_CONTROL: u8 = 0xB6;

/// System PPI bit reset command for port C bit 0 (RXRE, receive ready interrupt).
const PPI_RECEIVE_INTERRUPT_OFF: u8 = 0x00;
/// System PPI bit set command for port C bit 0 (RXRE, receive ready interrupt).
const PPI_RECEIVE_INTERRUPT_ON: u8 = 0x01;
/// System PPI bit reset command for port C bit 1 (TXEE, transmit empty interrupt).
const PPI_TRANSMIT_EMPTY_INTERRUPT_OFF: u8 = 0x02;
/// System PPI bit reset command for port C bit 2 (TXRE, transmit ready interrupt).
const PPI_TRANSMIT_READY_INTERRUPT_OFF: u8 = 0x04;

impl<T: TraceSink> Pc9801Bus<T> {
    /// INT 19h: RS-232C BIOS entry.
    pub(super) fn hle_int19h(&mut self, cpu: &mut impl Cpu) {
        let function = cpu.ah();
        match function {
            FUNCTION_INITIALIZE | FUNCTION_INITIALIZE_FLOW => {
                self.int19h_initialize(cpu, function == FUNCTION_INITIALIZE_FLOW);
            }
            FUNCTION_RECEIVED_COUNT..=FUNCTION_STATUS => {
                let Some(block) = self.int19h_initialized_block() else {
                    cpu.set_ah(RESULT_NOT_INITIALIZED);
                    return;
                };
                let result = match function {
                    FUNCTION_RECEIVED_COUNT => {
                        cpu.set_cx(self.read_mem_word(block + BLOCK_COUNT));
                        RESULT_OK
                    }
                    FUNCTION_SEND => {
                        self.serial.write_data(cpu.al());
                        RESULT_OK
                    }
                    FUNCTION_RECEIVE => self.int19h_receive(cpu, block),
                    FUNCTION_COMMAND => {
                        self.int19h_command(block, cpu.al());
                        RESULT_OK
                    }
                    _ => {
                        cpu.set_ch(self.serial.read_status());
                        cpu.set_cl(self.system_ppi.read_rs232c_status() | self.rtc.cdat());
                        RESULT_OK
                    }
                };
                cpu.set_ah(self.int19h_report_overflow(block, result));
            }
            _ => cpu.set_ah(RESULT_OK),
        }
    }

    /// Returns the control block when the BIOS initialized channel 0.
    fn int19h_initialized_block(&mut self) -> Option<u32> {
        let block = self.rs232c_control_block()?;
        let flag = self.read_mem_byte(block + BLOCK_FLAG);
        (flag & FLAG_INITIALIZED != 0).then_some(block)
    }

    /// Replaces the result by the overflow code once and clears the overflow flag.
    fn int19h_report_overflow(&mut self, block: u32, result: u8) -> u8 {
        let flag = self.read_mem_byte(block + BLOCK_FLAG);
        if flag & FLAG_OVERFLOW == 0 {
            return result;
        }
        self.write_mem_byte(block + BLOCK_FLAG, flag & !FLAG_OVERFLOW);
        RESULT_OVERFLOW
    }

    /// Functions 00h and 01h: programs the 8251A and the PIT and builds the control block.
    fn int19h_initialize(&mut self, cpu: &mut impl Cpu, flow_control: bool) {
        let mode = cpu.ch();
        let command = cpu.cl();
        let segment = cpu.es();
        let offset = cpu.di();
        let buffer_size = cpu.dx();
        let block = real_mode_address(segment, offset);

        self.int19h_program_baud_rate(cpu.al(), mode);
        self.int19h_program_usart(mode, command);

        self.ram_write_u16(RS_BUFFER_OFFSET_ADDRESS, offset);
        self.ram_write_u16(RS_BUFFER_SEGMENT_ADDRESS, segment);

        let mut flag = 0;
        if flow_control {
            flag |= FLAG_FLOW_CONTROL;
        }
        if command & COMMAND_INTERNAL_RESET == 0 {
            flag |= FLAG_INITIALIZED;
        }
        let send_timeout = match cpu.bh() {
            0 => DEFAULT_SEND_TIMEOUT,
            value => value,
        };
        let receive_timeout = match cpu.bl() {
            0 => DEFAULT_RECEIVE_TIMEOUT,
            value => value,
        };
        let entry_capacity = buffer_size / ENTRY_SIZE;
        let resume_level = entry_capacity / 4;
        let stop_level = entry_capacity / 4 * 3;
        let head = offset.wrapping_add(BLOCK_SIZE);
        let tail = head.wrapping_add(buffer_size);

        self.write_mem_byte(block + BLOCK_INTERFACE, 0);
        self.write_mem_byte(block + BLOCK_BASIC_FLAG, 0);
        self.write_mem_byte(block + BLOCK_FLAG, flag);
        self.write_mem_byte(block + BLOCK_COMMAND, command);
        self.write_mem_byte(block + BLOCK_SEND_TIMEOUT, send_timeout);
        self.write_mem_byte(block + BLOCK_RECEIVE_TIMEOUT, receive_timeout);
        self.write_mem_word(block + BLOCK_RESUME_LEVEL, resume_level);
        self.write_mem_word(block + BLOCK_STOP_LEVEL, stop_level);
        self.write_mem_word(block + BLOCK_HEAD_POINTER, head);
        self.write_mem_word(block + BLOCK_TAIL_POINTER, tail);
        self.write_mem_word(block + BLOCK_COUNT, 0);
        self.write_mem_word(block + BLOCK_PUT_POINTER, head);
        self.write_mem_word(block + BLOCK_GET_POINTER, head);

        self.system_ppi
            .write_control(PPI_TRANSMIT_EMPTY_INTERRUPT_OFF);
        self.system_ppi
            .write_control(PPI_TRANSMIT_READY_INTERRUPT_OFF);
        let receive_enabled =
            command & (COMMAND_RECEIVE_ENABLE | COMMAND_INTERNAL_RESET) == COMMAND_RECEIVE_ENABLE;
        self.int19h_set_receive_interrupt(receive_enabled);

        cpu.set_ah(RESULT_OK);
    }

    /// Loads PIT counter 2 with the divisor for the requested transfer rate.
    fn int19h_program_baud_rate(&mut self, rate_index: u8, mode: u8) {
        let rate = TRANSFER_RATES
            .get(usize::from(rate_index))
            .copied()
            .unwrap_or(TRANSFER_RATES[DEFAULT_TRANSFER_RATE_INDEX]);
        let factor = if mode & MODE_BAUD_FACTOR_MASK == MODE_BAUD_FACTOR_64 {
            BAUD_FACTOR_64
        } else {
            BAUD_FACTOR_16
        };
        let divisor = (self.clocks.pit_clock_hz / (rate * factor)) as u16;
        self.pit.write_control(
            PIT_BAUD_COUNTER,
            PIT_BAUD_CONTROL,
            self.current_cycle,
            self.clocks.cpu_clock_hz,
            self.clocks.pit_clock_hz,
        );
        let [low, high] = divisor.to_le_bytes();
        self.pit.write_counter(PIT_BAUD_COUNTER, low);
        self.pit.write_counter(PIT_BAUD_COUNTER, high);
    }

    /// Resets the 8251A and writes the mode word and the command word.
    fn int19h_program_usart(&mut self, mode: u8, command: u8) {
        for _ in 0..3 {
            self.serial.write_command(USART_SYNC_BYTE);
        }
        self.serial.write_command(COMMAND_INTERNAL_RESET);
        self.serial.write_command(mode);
        self.serial.write_command(command);
    }

    /// Enables or disables the receive ready interrupt in the system port and the PIC.
    fn int19h_set_receive_interrupt(&mut self, enabled: bool) {
        let irq_bit = 1u8 << RS232C_IRQ;
        let mask = self.pic.read_port2(0);
        if enabled {
            self.system_ppi.write_control(PPI_RECEIVE_INTERRUPT_ON);
            self.pic.write_port2(0, mask & !irq_bit);
        } else {
            self.system_ppi.write_control(PPI_RECEIVE_INTERRUPT_OFF);
            self.pic.write_port2(0, mask | irq_bit);
        }
    }

    /// Function 04h: takes the oldest entry from the ring buffer into CX.
    fn int19h_receive(&mut self, cpu: &mut impl Cpu, block: u32) -> u8 {
        let count = self.read_mem_word(block + BLOCK_COUNT);
        if count == 0 {
            return RESULT_TIMEOUT;
        }

        let segment = self.ram_read_u16(RS_BUFFER_SEGMENT_ADDRESS);
        let get_pointer = self.read_mem_word(block + BLOCK_GET_POINTER);
        let entry = self.read_mem_word(real_mode_address(segment, get_pointer));
        let mut next_get = get_pointer.wrapping_add(ENTRY_SIZE);
        if next_get >= self.read_mem_word(block + BLOCK_TAIL_POINTER) {
            next_get = self.read_mem_word(block + BLOCK_HEAD_POINTER);
        }
        let remaining = count - 1;
        self.write_mem_word(block + BLOCK_GET_POINTER, next_get);
        self.write_mem_word(block + BLOCK_COUNT, remaining);

        let mut flag = self.read_mem_byte(block + BLOCK_FLAG) & !FLAG_BUFFER_FULL;
        if flag & FLAG_STOP_SENT != 0 && remaining <= self.read_mem_word(block + BLOCK_RESUME_LEVEL)
        {
            self.serial.write_data(CONTROL_RESUME);
            flag &= !FLAG_STOP_SENT;
        }
        self.write_mem_byte(block + BLOCK_FLAG, flag);

        cpu.set_cx(entry);
        RESULT_OK
    }

    /// Function 05h: writes a command word and follows it with the interrupt enables.
    fn int19h_command(&mut self, block: u32, command: u8) {
        self.serial.write_command(command);
        self.write_mem_byte(block + BLOCK_COMMAND, command);
        if command & COMMAND_INTERNAL_RESET != 0 {
            let flag = self.read_mem_byte(block + BLOCK_FLAG);
            self.write_mem_byte(block + BLOCK_FLAG, flag & !FLAG_INITIALIZED);
        }
        let receive_enabled =
            command & (COMMAND_RECEIVE_ENABLE | COMMAND_INTERNAL_RESET) == COMMAND_RECEIVE_ENABLE;
        self.int19h_set_receive_interrupt(receive_enabled);
    }
}
