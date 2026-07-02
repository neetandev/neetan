//! FM Towns I/O port read dispatch.

use common::Tracing;

use super::{DMA_EXTENDED, DMA_MAIN, FAST_MODE_LAMP_VRAM_WAIT_LIMIT, TownsBus};
use crate::config::TownsModel;

/// Horizontal scan-line period in nanoseconds (~31 kHz).
const HSYNC_LINE_NANOS: u64 = 32_000;
/// Active (non-retrace) portion of a scan line in nanoseconds; HSYNC is high for
/// the remainder of the line.
const HSYNC_ACTIVE_NANOS: u64 = 30_000;
/// Nanoseconds in one second.
const NANOS_PER_SECOND: u64 = 1_000_000_000;

impl<T: Tracing> TownsBus<T> {
    /// Reads a byte from an I/O port.
    pub(crate) fn io_read(&mut self, port: u16) -> u8 {
        match port {
            // Master PIC (0x0000 command/status, 0x0002 mask).
            0x0000 => self.pic.read_port0(0),
            0x0002 => self.pic.read_port2(0),
            // Slave PIC (0x0010 command/status, 0x0012 mask).
            0x0010 => self.pic.read_port0(1),
            0x0012 => self.pic.read_port2(1),

            // Reset reason (I/O 0x0020): bit 0 marks a prior software reset and
            // self-clears on read. Power control (0x0022) reads back as 0.
            0x0020 => {
                let reason = self.reset_reason;
                self.reset_reason &= !0x03;
                reason
            }
            0x0022 => 0x00,
            // CPU misc register: the 2_UG and later models report 0x07 here; the
            // CX and MX predate it and read back 0xFF.
            0x0024 => match self.model {
                TownsModel::FmTownsIICx | TownsModel::FmTownsIIMx => 0xFF,
            },
            // Free-running microsecond counter.
            0x0026 => self.free_run_counter() as u8,
            0x0027 => (self.free_run_counter() >> 8) as u8,
            // NMI mask register.
            0x0028 => self.nmi_mask,

            // Machine identity.
            0x0030 => self.machine_id.0,
            0x0031 => self.machine_id.1,
            // Serial machine-ID EEPROM (bit-serial read).
            0x0032 => self.read_serial_rom(),

            // Interval timer block 0 counters.
            0x0040 => self.read_timer_counter(0),
            0x0042 => self.read_timer_counter(1),
            0x0044 => self.read_timer_counter(2),
            // Interval timer block 1 counters.
            0x0050 => self.read_timer_counter(3),
            0x0052 => self.read_timer_counter(4),
            0x0054 => self.read_timer_counter(5),
            // Interval control / status.
            0x0060 => self.timer.read_interval_status(),
            // 1 us wait register: reads back as ready.
            0x006C => 0x00,

            // RTC data / command.
            0x0070 => {
                let time = (self.host_local_time_fn)();
                self.rtc.read_data(&time, self.subsecond_micros())
            }
            0x0080 => 0x00,

            // DMA controllers: main bank and the second (EXDMAC) bank.
            0x00A0..=0x00AF => self.dmac[DMA_MAIN].read(port),
            0x00B0..=0x00BF => self.dmac[DMA_EXTENDED].read(port),

            // CRTC register file.
            0x0440 => self.video.read_crtc_address(),
            0x0442 => self.video.read_crtc_data_low(),
            0x0443 => {
                let hsync = self.hsync_active();
                let vertical_display = self.vertical_display_active();
                self.video.read_crtc_data_high(hsync, vertical_display)
            }
            // Video-out control and DPMD / sprite status.
            0x0448 => self.video.read_video_out_address(),
            0x044A => self.video.read_video_out_data(),
            0x044C => self.read_dpmd_sprite_status(),
            // Sprite controller: index latch and data.
            0x0450 => self.sprite.read_address(),
            0x0452 => self.sprite.read_data(),
            // VRAM plane-mask registers.
            0x0458 => self.memory.read_vram_mask_latch(),
            0x045A => self.memory.read_vram_mask_low(),
            0x045B => self.memory.read_vram_mask_high(),

            // MX high-resolution CRTC ("image out") register file: presence /
            // VRAM-size detect, index latch, and the four data lanes.
            0x0470 => self.video.read_high_res_id(),
            0x0471 => self.video.read_vram_size(),
            0x0472 => self.video.read_high_res_addr_low(),
            0x0473 => self.video.read_high_res_addr_high(),
            0x0474..=0x0477 => self.video.read_high_res_data((port - 0x0474) as u8),

            // FMR resolution / video-detect register (read-only). Returns 0x00 on
            // real hardware; the SYSROM boot checks it and halts on 0xFF.
            0x0400 => 0x00,

            // MB8877 FDC: status/track/sector/data, drive status, drive select,
            // FDDV extension, and drive switch.
            0x0200 | 0x0202 | 0x0204 | 0x0206 | 0x0208 | 0x020C | 0x020D | 0x020E => {
                self.fdc_io_read(port)
            }

            // CD-ROM controller: capability port and the 0x04C0-0x04CF register file.
            0x04B0 | 0x04C0..=0x04CF => self.cdrom_io_read(port),

            // MB89352-class SCSI controller host interface (data, status, and
            // the word-transfer capability probe).
            0x0C30..=0x0C37 => self.scsi_io_read(port),

            // Game port: pad (0x04D0), mouse (0x04D2), output latch (0x04D6).
            0x04D0 => self.gameport.read_port_a(),
            0x04D2 => self.gameport.read_port_b(self.current_cycle),
            0x04D6 => self.gameport.read_output(),

            // Sound: mute latch, OPN2 status, sound-IRQ reason, RF5C68 IRQ.
            0x04D5 => self.sound_mute,
            // OPN2 status: timer flags in bits 0-1, busy in bit 7; the other
            // bits float high (0x7C). 0x04DC mirrors 0x04D8 on read.
            0x04D8 | 0x04DC => (self.fm.read_status(self.current_cycle) & 0x83) | 0x7C,
            // Sound-IRQ reason: bit 0 = FM timer, bit 3 = RF5C68 PCM.
            0x04E9 => {
                let mut reason = 0;
                if self.fm.irq_asserted() {
                    reason |= 0x01;
                }
                if self.pcm.interrupt_asserted() {
                    reason |= 0x08;
                }
                reason
            }
            // Sound sampling (ADC) stub: no real audio input is modeled, so the
            // data port reports the unsigned silence midpoint and the flags port
            // always reports a sample ready.
            0x04E7 => 0x80,
            0x04E8 => 0x01,
            // Electronic-volume attenuators: data/command port pairs.
            0x04E0 => self.elevol[0].read_data(),
            0x04E1 => self.elevol[0].read_command(),
            0x04E2 => self.elevol[1].read_data(),
            0x04E3 => self.elevol[1].read_command(),
            // RF5C68 interrupt-bank mask (0x04EA) and pending banks (0x04EB,
            // read-clears).
            0x04EA => self.pcm.interrupt_mask(),
            0x04EB => {
                let pending = self.pcm.take_interrupt_pending();
                self.refresh_sound_irq();
                pending
            }
            // Electronic-volume / audio-out latch.
            0x04EC => self.sound_audio,

            // FMR-compatible display sync status (bit 0 = VSYNC, bit 1 = HSYNC).
            // Games poll bit 0 to synchronize to the vertical retrace edge.
            0xFDA0 => self.read_hsync_vsync_status(),

            // Memory banking registers.
            0x0404 => self.memory.read_fmr_vram_select(),
            0x0480 => self.memory.read_sysrom_dic_select(),
            0x0484 => self.memory.read_dic_rom_bank(),

            // Memory card (I/O 0x048A/0x0490/0x0491): with no card inserted the
            // status reports "no card present" (bits 1 and 2), the bank reads
            // back its selected value, and the attribute reports the OLD-type /
            // absent-card bit 7 plus the register-select latch.
            0x048A => 0x06,
            0x0490 => (self.memcard_bank & 0x03) << 4,
            0x0491 => 0x80 | u8::from(self.memcard_reg),

            // FMR register block, accessed through its I/O-port alias: plane
            // mask, display mode, page select, kanji CG-ROM font read
            // (0xFF94-0xFF97), and the KVRAM/ANK-font select (0xFF99).
            0xFF81..=0xFF83 | 0xFF94..=0xFF99 => self.memory.read_fmr_io_register(port),

            // FMR-compatible HSYNC/VSYNC status (I/O 0xFF86): VSYNC in bit 2,
            // HSYNC in bit 7, bit 4 always set.
            0xFF86 => self.read_fmr_hsync_vsync_status(),

            // CMOS / backup RAM via the I/O window.
            0x3000..=0x3FFF => self.memory.read_cmos_io(port),

            // TVRAM dirty/enable status: bit 7 set (0x80) if the text VRAM was
            // written since the last read, then self-clearing.
            0x05C8 => self.memory.take_tvram_written(),

            // Analog palette read-back: index latch and B/R/G components (4-bit
            // precision, high nibble). The firmware saves and restores the
            // palette through these, so they must return the stored value.
            0xFD90 => self.video.read_palette_code(),
            0xFD92 => self.video.read_palette_blue(),
            0xFD94 => self.video.read_palette_red(),
            0xFD96 => self.video.read_palette_green(),

            // FMR digital palette: eight 4-bit registers. The FMR-compatible
            // palette-setup routine reads these to decide which analog color
            // each of the eight FMR palette entries maps to, so they must
            // return the stored value.
            0xFD98..=0xFD9F => self.video.read_digital_palette(usize::from(port - 0xFD98)),

            // Memory wait latches (first-generation alias at 0x05E0).
            0x05E0 | 0x05E2 => self.main_ram_wait,
            0x05E6 => self.vram_wait,

            // Installed RAM size in megabytes.
            0x05E8 => self.memory.total_ram_megabytes(),

            // FASTMODE lamp: lit while the machine runs without memory waits.
            0x05EC => {
                u8::from(self.main_ram_wait == 0 && self.vram_wait < FAST_MODE_LAMP_VRAM_WAIT_LIMIT)
            }

            // Serial keyboard.
            0x0600 => {
                let value = self.keyboard.read_data();
                self.refresh_keyboard_irq();
                value
            }
            0x0602 => self.keyboard.read_status(),
            0x0604 => self.keyboard.read_irq(),

            // Built-in RS-232C USART: data, status, and interrupt reason.
            0x0A00 => {
                let (data, _clear, _retrigger) = self.rs232c.read_data();
                self.refresh_rs232c_irq();
                data
            }
            0x0A02 => self.rs232c.read_status(),
            0x0A06 => self.rs232c_int_reason(),

            _ => {
                self.tracer.trace_io_unhandled_read(port);
                0xFF
            }
        }
    }

    /// Derives the raster sync state at the current cycle: `(in_vsync, in_hsync)`.
    /// VSYNC pulses at the start of each frame (matching the scheduled
    /// VsyncStart/VsyncEnd edges); HSYNC pulses once per scan line during the
    /// active field.
    fn sync_state(&self) -> (bool, bool) {
        let cpu_clock_hz = u64::from(self.clocks.cpu_clock_hz);
        let frame_cycles = self.video.frame_cycles(self.clocks.cpu_clock_hz).max(1);
        let into_frame = self.current_cycle % frame_cycles;

        let in_vsync = into_frame < self.vsync_duration_cycles();

        let line_cycles = (HSYNC_LINE_NANOS * cpu_clock_hz / NANOS_PER_SECOND).max(1);
        let line_active_cycles = (HSYNC_ACTIVE_NANOS * cpu_clock_hz / NANOS_PER_SECOND).max(1);
        let in_hsync = !in_vsync && (into_frame % line_cycles) >= line_active_cycles;

        (in_vsync, in_hsync)
    }

    /// FMR-compatible display sync status (I/O 0xFDA0 read): bit 0 = VSYNC,
    /// bit 1 = HSYNC.
    fn read_hsync_vsync_status(&self) -> u8 {
        let (in_vsync, in_hsync) = self.sync_state();
        let mut data = 0;
        if in_vsync {
            data |= 0x01;
        }
        if in_hsync {
            data |= 0x02;
        }
        data
    }

    /// FMR HSYNC/VSYNC status in the alternate FMR bit layout (I/O 0xFF86):
    /// VSYNC in bit 2, HSYNC in bit 7, bit 4 always set.
    fn read_fmr_hsync_vsync_status(&self) -> u8 {
        let (in_vsync, in_hsync) = self.sync_state();
        let mut data = 0x10;
        if in_vsync {
            data |= 0x04;
        }
        if in_hsync {
            data |= 0x80;
        }
        data
    }

    /// Assembles the DPMD / sprite-status byte (I/O 0x044C): DPMD in bit 7
    /// (self-clearing), sprite busy in bit 1, sprite render page in bit 0.
    fn read_dpmd_sprite_status(&mut self) -> u8 {
        let mut data = self.video.read_dpmd();
        if self.sprite.busy() {
            data |= 0x02;
        }
        if self.sprite.internal_page() {
            data |= 0x01;
        }
        data
    }

    /// Reads a byte from an interval-timer counter, handling channel 1's OUT
    /// clear on access.
    fn read_timer_counter(&mut self, channel: usize) -> u8 {
        if channel == 1 {
            self.timer.clear_timer1_out();
            self.refresh_timer_irq();
        }
        self.timer
            .read_counter(channel, self.current_cycle, self.clocks.cpu_clock_hz)
    }
}
