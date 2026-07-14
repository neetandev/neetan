//! FM Towns I/O port write dispatch.

use common::TraceSink;

use super::{DMA_EXTENDED, DMA_MAIN, SLOW_MODE_MEMORY_WAITS, TownsBus};

impl<T: TraceSink> TownsBus<T> {
    /// Writes a byte to an I/O port.
    pub(crate) fn io_write(&mut self, port: u16, value: u8) -> bool {
        let mut handled = true;
        match port {
            // Master PIC (0x0000 ICW1/OCW2/OCW3, 0x0002 ICW2-4/mask).
            0x0000 => {
                self.pic.write_port0(0, value);
                self.pic.invalidate_irq_cache();
            }
            0x0002 => self.pic.write_port2(0, value),
            // Slave PIC.
            0x0010 => {
                self.pic.write_port0(1, value);
                self.pic.invalidate_irq_cache();
            }
            0x0012 => self.pic.write_port2(1, value),

            // Interval timer block 0 counters.
            0x0040 => self.write_timer_counter(0, value),
            0x0042 => self.write_timer_counter(1, value),
            0x0044 => self.write_timer_counter(2, value),
            0x0046 => self.timer.write_control(0, value),
            // Interval timer block 1 counters.
            0x0050 => self.write_timer_counter(3, value),
            0x0052 => self.write_timer_counter(4, value),
            0x0054 => self.write_timer_counter(5, value),
            0x0056 => self.timer.write_control(1, value),
            // Interval control (masks, beep enable, OUT clear).
            0x0060 => {
                self.timer.write_interval_control(value);
                self.refresh_timer_irq();
                self.refresh_beeper_gate();
            }
            // 1 us wait register: no timing side effect is modeled.
            0x006C => {}

            // RTC data / command.
            0x0070 => self.rtc.write_data(value),
            0x0080 => self.rtc.write_command(value),

            // DMA controllers: main bank and the second (EXDMAC) bank.
            0x00A0..=0x00AF => self.dmac[DMA_MAIN].write(port, value),
            0x00B0..=0x00BF => self.dmac[DMA_EXTENDED].write(port, value),

            // NMI mask register.
            0x0028 => self.nmi_mask = value,

            // Reset reason / power control (I/O 0x0020 and 0x0022): bit 6 powers
            // the machine off, bit 0 requests a software reset. The run loop
            // performs the reset or shutdown once the current instruction retires.
            0x0020 | 0x0022 => {
                if value & 0x40 != 0 {
                    self.power_off_requested = true;
                }
                if value & 0x01 != 0 {
                    self.reset_reason |= 0x01;
                    self.soft_reset_pending = true;
                }
            }
            // Serial machine-ID EEPROM clock/select/reset lines.
            0x0032 => self.write_serial_rom(value),

            // CMOS / backup RAM via the I/O window.
            0x3000..=0x3FFF => self.memory.write_cmos_io(port, value),

            // CRTC register file.
            0x0440 => self.video.write_crtc_address(value),
            0x0442 => self.video.write_crtc_data_low(value),
            0x0443 => self.video.write_crtc_data_high(value),
            // Video-out ("sifter") control.
            0x0448 => self.video.write_video_out_address(value),
            0x044A => self.video.write_video_out_data(value),
            // Sprite controller: index latch and data.
            0x0450 => self.sprite.write_address(value),
            0x0452 => {
                // A CONTROL1 write that disables the sprite engine mid-transfer
                // commits the sprites immediately so they survive to the next VSYNC.
                if self.sprite.write_data(value) {
                    let params = self.sprite.immediate_render_params();
                    self.memory.render_sprites(&params);
                }
            }
            // VRAM plane-mask registers.
            0x0458 => self.memory.write_vram_mask_latch(value),
            0x045A => self.memory.write_vram_mask_low(value),
            0x045B => self.memory.write_vram_mask_high(value),

            // MX high-resolution CRTC ("image out") register file. The presence /
            // VRAM-size ports are read-only; the index latch and data lanes are
            // written here. Word/dword accesses are handled by the io_write_word
            // override so the 32-bit register and palette auto-increment semantics
            // match the hardware.
            0x0470 | 0x0471 => {}
            0x0472 => self.video.write_high_res_addr_low(value),
            0x0473 => self.video.write_high_res_addr_high(value),
            0x0474..=0x0477 => self.video.write_high_res_data((port - 0x0474) as u8, value),
            // VSYNC interrupt clear.
            0x05CA => {
                self.video.clear_vsync_irq();
                self.refresh_vsync_irq();
            }
            // Analog palette: index and B/R/G data (BRG port order).
            0xFD90 => self.video.write_palette_code(value),
            0xFD92 => self.video.write_palette_blue(value),
            0xFD94 => self.video.write_palette_red(value),
            0xFD96 => self.video.write_palette_green(value),
            // FMR digital palette (8 entries).
            0xFD98..=0xFD9F => self
                .video
                .write_digital_palette(usize::from(port - 0xFD98), value),
            // CRT output / show-page control.
            0xFDA0 => self.video.write_show_page_fda0(value),

            // MB8877 FDC: command/track/sector/data, drive control, drive select.
            0x0200 | 0x0202 | 0x0204 | 0x0206 | 0x0208 | 0x020C | 0x020D | 0x020E => {
                self.fdc_io_write(port, value)
            }

            // CD-ROM controller: the 0x04C0-0x04CF register file.
            0x04C0..=0x04CF => self.cdrom_io_write(port, value),

            // MB89352-class SCSI controller host interface (data and control).
            0x0C30..=0x0C37 => self.scsi_io_write(port, value),

            // Game port output latch (COM / trigger strobes).
            0x04D6 => self.gameport.write_output(self.current_cycle, value),

            // Sound mute latch (bit 1 gates FM, bit 0 gates PCM).
            0x04D5 => self.sound_mute = value & 0x03,
            // OPN2 register access: 0x04D8 address / 0x04DA data (bank 0),
            // 0x04DC address / 0x04DE data (bank 1, channels 3-5).
            0x04D8 => {
                self.fm.write_address(value, self.current_cycle);
                self.apply_sound_timers();
            }
            0x04DA => {
                self.fm.write_data(value, self.current_cycle);
                self.apply_sound_timers();
            }
            0x04DC => {
                self.fm.write_address_hi(value, self.current_cycle);
                self.apply_sound_timers();
            }
            0x04DE => {
                self.fm.write_data_hi(value, self.current_cycle);
                self.apply_sound_timers();
            }
            // Sound sampling (ADC) stub: no real audio input is modeled, so the
            // data and flags writes are accepted and dropped.
            0x04E7 | 0x04E8 => {}
            // Electronic-volume attenuators: data/command port pairs.
            0x04E0 => self.elevol[0].write_data(value),
            0x04E1 => self.elevol[0].write_command(value),
            0x04E2 => self.elevol[1].write_data(value),
            0x04E3 => self.elevol[1].write_command(value),
            // RF5C68 interrupt-bank mask.
            0x04EA => {
                self.pcm.set_interrupt_mask(value);
                self.refresh_sound_irq();
            }
            // Audio-out latch (bit 6 = master output enable).
            0x04EC => self.sound_audio = value,
            // RF5C68 PCM registers (envelope, pan, freq, loop, start, control,
            // channel on/off).
            0x04F0..=0x04F8 => {
                self.pcm.write_register((port - 0x04F0) as u8, value);
                self.refresh_sound_irq();
            }

            // Memory wait latches (first-generation alias at 0x05E0). Stored
            // and read back only; the slow-mode clock change is not modeled.
            0x05E0 | 0x05E2 => self.main_ram_wait = value,
            0x05E6 => self.vram_wait = value,
            // FASTMODE: bit 0 set clears all memory waits, cleared restores
            // the FMR-compatible slow-mode waits.
            0x05EC => {
                if value & 0x01 != 0 {
                    self.main_ram_wait = 0;
                    self.vram_wait = 0;
                } else {
                    self.main_ram_wait = SLOW_MODE_MEMORY_WAITS;
                    self.vram_wait = SLOW_MODE_MEMORY_WAITS;
                }
            }

            // Memory banking registers.
            0x0404 => self.memory.write_fmr_vram_select(value),
            0x0480 => self.memory.write_sysrom_dic_select(value),
            0x0484 => self.memory.write_dic_rom_bank(value),

            // Memory card (I/O 0x048A/0x0490/0x0491): the status write is
            // ignored, the bank latch stores bits 4-5, and the attribute write
            // stores the register-select bit.
            0x048A => {}
            0x0490 => self.memcard_bank = (value >> 4) & 0x03,
            0x0491 => self.memcard_reg = value & 0x01 != 0,

            // FMR register block, accessed through its I/O-port alias: plane
            // mask, display mode, page select, kanji CG-ROM JIS code latch
            // (0xFF94/0xFF95), and the KVRAM/ANK-font select (0xFF99). The
            // pattern-read ports (0xFF96/0xFF97) are write-protected.
            0xFF81..=0xFF83 | 0xFF94..=0xFF99 => self.memory.write_fmr_io_register(port, value),
            // FMR HSYNC/VSYNC status port (I/O 0xFF86) ignores writes.
            0xFF86 => {}

            // Serial keyboard.
            0x0600 | 0x0602 => {
                self.keyboard.write_command(value);
                self.refresh_keyboard_irq();
            }
            0x0604 => {
                self.keyboard.write_irq(value);
                self.refresh_keyboard_irq();
            }

            // Built-in RS-232C USART: data, command, and interrupt control.
            0x0A00 => {
                self.rs232c.write_data(value);
                self.refresh_rs232c_irq();
            }
            0x0A02 => {
                self.rs232c.write_command(value);
                self.refresh_rs232c_irq();
            }
            0x0A08 => {
                self.rs232c_int_enable = value;
                self.refresh_rs232c_irq();
            }

            _ => {
                handled = false;
            }
        }
        handled
    }

    /// Writes a byte to an interval-timer counter, rescheduling its interrupt
    /// edge when the channel finishes loading, and clearing channel 1's OUT.
    fn write_timer_counter(&mut self, channel: usize, value: u8) {
        if channel == 1 {
            self.timer.clear_timer1_out();
            self.refresh_timer_irq();
        }
        let loaded = self.timer.write_counter(channel, value, self.current_cycle);
        if loaded && (channel == 0 || channel == 1) {
            self.reschedule_timer(channel);
        }
        // Channel 2 sets the buzzer tone; keep the beeper's reload in sync.
        if loaded && channel == 2 {
            self.beeper
                .set_pit_reload(self.timer.beep_reload(), self.current_cycle);
        }
    }
}
