//! Display sub CPU memory-mapped I/O decode (`0xD400-0xD7FF`).
//!
//! The sixteen registers repeat every sixteen bytes across the region, so the bus
//! masks the offset to four bits before dispatching here. The base registers
//! provide the main/sub handshake and display controls, while FM-77AV-only
//! registers are decoded directly.

use common::TraceSink;
use device::mb61vh010_alu::{PORT_ALU_FIRST, PORT_ALU_LAST};

use crate::bus::Fm7Bus;

/// `0xD400` keyboard-high port: bit 7 carries the ninth keycode bit (read).
const PORT_KEYBOARD_HIGH: u8 = 0x00;
/// `0xD401` keyboard-low port: keycode low byte, read clears the interrupt (read).
const PORT_KEYBOARD_LOW: u8 = 0x01;
/// `0xD402` CANCEL interrupt acknowledge (read).
const PORT_CANCEL_ACK: u8 = 0x02;
/// `0xD403` main beeper one-shot request (read).
const PORT_BEEP_REQUEST: u8 = 0x03;
/// `0xD404` main ATTENTION interrupt request (read).
const PORT_ATTENTION: u8 = 0x04;
/// `0xD405` FM-77AV cycle-steal control (write).
const PORT_CYCLE_STEAL: u8 = 0x05;
/// `0xD408` CRT display enable flag.
const PORT_CRT_FLAG: u8 = 0x08;

/// `0xD405` write bit 0: cycle steal is enabled while it is clear (active low).
const CYCLE_STEAL_DISABLE_BIT: u8 = 0x01;
/// `0xD409` VRAM access flag: read sets it, write clears it.
const PORT_VRAM_ACCESS: u8 = 0x09;
/// `0xD40A` sub busy flag: read clears it, write sets it.
const PORT_SUB_BUSY: u8 = 0x0A;
/// `0xD40D` FM-77AV keyboard LED register: read lights the INSERT LED, write
/// clears it.
const PORT_KEY_LED: u8 = 0x0D;
/// `0xD40E` display offset high byte (write).
const PORT_DISPLAY_OFFSET_HIGH: u8 = 0x0E;
/// `0xD40F` display offset low byte (write).
const PORT_DISPLAY_OFFSET_LOW: u8 = 0x0F;
/// `0xD431` FM-77AV keyboard-encoder data register (read/write).
const PORT_ENCODER_DATA: u8 = 0x31;
/// `0xD432` FM-77AV keyboard-encoder status register (read).
const PORT_ENCODER_STATUS: u8 = 0x32;

/// `0xD400` read value when the ninth keycode bit is set (bit 7 high).
const KEYBOARD_HIGH_SET: u8 = 0xFF;
/// `0xD400` read value when the ninth keycode bit is clear (bit 7 low).
const KEYBOARD_HIGH_CLEAR: u8 = 0x7F;
/// Read value returned by handshake ports without a data payload.
const HANDSHAKE_READ: u8 = 0xFF;
/// Read value returned by ports that report no data.
const IDLE_READ: u8 = 0x00;
/// Open-bus value returned by ports that float high.
const OPEN_BUS: u8 = 0xFF;

/// Mask folding an address into one of the 16 base sub registers. The base
/// registers (`0xD400-0xD40F`) mirror across the region on the FM-7.
const BASE_PORT_MASK: u8 = 0x0F;
/// `0xD430` FM-77AV sub misc register: NMI mask, ALU busy, display page / CG.
const PORT_SUB_MISC: u8 = 0x30;
/// First low-byte offset of the FM-77AV AV-only sub register block (`0xD410`).
const AV_REGISTER_FIRST: u8 = 0x10;

impl<T: TraceSink> Fm7Bus<T> {
    /// Reads a sub MMIO register and reports whether it was decoded.
    pub(crate) fn sub_io_read(&mut self, port: u8) -> (u8, bool) {
        if self.model().has_mmr() && port >= AV_REGISTER_FIRST {
            let value = match port {
                PORT_ALU_FIRST..=PORT_ALU_LAST => self.alu.read_register(port),
                PORT_SUB_MISC => self.read_sub_misc_register(),
                PORT_ENCODER_DATA => self.encoder_read_data(),
                PORT_ENCODER_STATUS => self.encoder_read_status(),
                _ => return (OPEN_BUS, false),
            };
            return (value, true);
        }
        (self.sub_io_base_read(port & BASE_PORT_MASK), true)
    }

    /// Reads one of the 16 base sub registers.
    fn sub_io_base_read(&mut self, port: u8) -> u8 {
        match port {
            PORT_KEYBOARD_HIGH => {
                if self.keyboard.keycode_high() {
                    KEYBOARD_HIGH_SET
                } else {
                    KEYBOARD_HIGH_CLEAR
                }
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
            PORT_CANCEL_ACK => {
                self.acknowledge_cancel();
                HANDSHAKE_READ
            }
            PORT_BEEP_REQUEST => {
                self.request_sub_beep();
                HANDSHAKE_READ
            }
            PORT_ATTENTION => {
                self.raise_sub_attention();
                HANDSHAKE_READ
            }
            PORT_VRAM_ACCESS => {
                self.set_vram_access_flag(true);
                HANDSHAKE_READ
            }
            PORT_SUB_BUSY => {
                self.clear_sub_busy_on_read();
                HANDSHAKE_READ
            }
            PORT_CRT_FLAG => {
                self.video.set_crt_enabled(true);
                OPEN_BUS
            }
            PORT_KEY_LED if self.model().has_mmr() => {
                self.set_insert_led(true);
                IDLE_READ
            }
            _ => IDLE_READ,
        }
    }

    /// Writes a sub MMIO register and reports whether it was decoded.
    pub(crate) fn sub_io_write(&mut self, port: u8, value: u8) -> bool {
        if self.model().has_mmr() && port >= AV_REGISTER_FIRST {
            match port {
                PORT_ALU_FIRST..=PORT_ALU_LAST => self.alu_register_write(port, value),
                PORT_SUB_MISC => self.write_sub_misc_register(value),
                PORT_ENCODER_DATA => self.encoder_write_data(value),
                _ => return false,
            }
            return true;
        }
        self.sub_io_base_write(port & BASE_PORT_MASK, value);
        true
    }

    /// Writes one of the 16 base sub registers.
    fn sub_io_base_write(&mut self, port: u8, value: u8) {
        match port {
            PORT_CYCLE_STEAL if self.model().has_mmr() => {
                self.set_cycle_steal(value & CYCLE_STEAL_DISABLE_BIT == 0);
            }
            PORT_VRAM_ACCESS => self.set_vram_access_flag(false),
            PORT_SUB_BUSY => self.set_sub_busy_on_write(),
            PORT_CRT_FLAG => self.video.set_crt_enabled(false),
            PORT_KEY_LED if self.model().has_mmr() => self.set_insert_led(false),
            PORT_DISPLAY_OFFSET_HIGH => self.video.write_display_offset_high(value),
            PORT_DISPLAY_OFFSET_LOW => self.video.write_display_offset_low(value),
            _ => {}
        }
    }
}
