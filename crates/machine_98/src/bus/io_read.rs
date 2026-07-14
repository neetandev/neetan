use common::{debug, warn};
use device::ga1280a::is_ga1280a_port;

use crate::{
    Pc9801Bus, Tracing,
    bus::{
        FDC_1MB_INPUT_REGISTER, FDC_640K_INPUT_REGISTER, FDC_MEDIA_READ_FIXED_BITS, IO_WAIT_CYCLES,
        MODE_DETECT_NORMAL, SYSTEM_STATUS_DEFAULT,
    },
    scheduler::Event98,
};

impl<T: Tracing> Pc9801Bus<T> {
    fn next_gdc_frame_event_cycle(&self) -> Option<u64> {
        let vsync_cycle = self.scheduler.state.fire_cycles[Event98::GdcVsync as usize];
        let display_start_cycle =
            self.scheduler.state.fire_cycles[Event98::GdcDisplayStart as usize];

        match (vsync_cycle, display_start_cycle) {
            (Some(vsync_cycle), Some(display_start_cycle)) => {
                Some(vsync_cycle.min(display_start_cycle))
            }
            (Some(vsync_cycle), None) => Some(vsync_cycle),
            (None, Some(display_start_cycle)) => Some(display_start_cycle),
            (None, None) => None,
        }
    }

    #[inline]
    pub(super) fn io_read_byte_impl(&mut self, port: u16) -> u8 {
        self.pending_wait_cycles += IO_WAIT_CYCLES;
        self.tracer.set_cycle(self.current_cycle);
        if is_ga1280a_port(port) {
            self.pending_wait_cycles += self.cbus_wait_cycles();
            let value = self
                .ga1280a
                .as_mut()
                .and_then(|ga| ga.try_handle_io_read_byte(port))
                .unwrap_or(0xFF);
            self.tracer.trace_io_read(port, value);
            return value;
        }
        let value = match port {
            // GAINIT probes the seven alternative GA SW1 base addresses
            // (xxD0/D4/DC/E0/E4/E8/EC + 1) at selector 0x1D to detect a board
            // at a non-factory port. Our GA is hard-wired to the factory base
            // (xxD8h), so these probes must read back as open bus.
            0x1DD1 | 0x1DD5 | 0x1DDD | 0x1DE1 | 0x1DE5 | 0x1DE9 | 0x1DED => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                0xFF
            }

            // PIC
            0x00 | 0x08 => self.pic.read_port0(((port >> 3) & 1) as usize),
            0x02 | 0x0A => self.pic.read_port2(((port >> 3) & 1) as usize),

            // DMA channel registers
            0x01 => self.dma.read_address(0),
            0x03 => self.dma.read_count(0),
            0x05 => self.dma.read_address(1),
            0x07 => self.dma.read_count(1),
            0x09 => self.dma.read_address(2),
            0x0B => self.dma.read_count(2),
            0x0D => self.dma.read_address(3),
            0x0F => self.dma.read_count(3),
            0x11 => self.dma.read_status(),
            // DMA write-only control registers return open bus on read.
            // Ref: undoc98 `io_dma.txt` (READ: 禁止 for these ports).
            0x13 | 0x15 | 0x17 | 0x19 | 0x1B | 0x1D | 0x1F | 0x29 => 0xFF,

            // DMA page registers are write-only; reads return open-bus.
            // Ref: undoc98 io_dma.txt: READ 禁止 for 0x21/0x23/0x25/0x27.
            0x21 | 0x23 | 0x25 | 0x27 => 0xFF,

            // µPD4990A RTC Set Register (write-only on µPD4990A; open-bus on read).
            // Ref: undoc98 `io_cal.txt` (port 0x0020)
            0x20 => 0xFF,

            // Undocumented RTC control/mode latch.
            // Used by some TSRs/boot utilities during CPU probing.
            0x22 => self.rtc_control_22,

            // RS-232C i8251 data register.
            0x30 => {
                let (data, clear_irq, retrigger_irq) = self.serial.read_data();
                if clear_irq {
                    self.pic.clear_irq(4);
                }
                if retrigger_irq {
                    self.pic.set_irq(4);
                }
                data
            }
            // RS-232C i8251 status register.
            0x32 => self.serial.read_status(),

            // DIP switch 2 via 8255 PPI Port A.
            // On PC-9821, synthesized from SDIP front bank registers:
            //   bits {7,6,5,3,2,1,0} from register 1 (0x851E)
            //   bit 4 (memsw init) from register 3 (0x871E) bit 5
            // Ref: undoc98 io_sdip.txt
            0x31 => {
                if self.machine_model.has_sdip() {
                    let reg1 = self.sdip.read_front_bank(1);
                    let reg3 = self.sdip.read_front_bank(3);
                    (reg1 & 0xEF) | ((reg3 & 0x20) >> 1)
                } else {
                    self.system_ppi.read_dip_switch_2()
                }
            }
            0x33 => self.system_ppi.read_rs232c_status() | self.rtc.cdat(),
            0x35 => self.system_ppi.read_port_c(),

            // ARTIC timestamp (307.2 kHz, 24-bit).
            // 0x005D and 0x005E both expose bits 15-8, and 0x005F bits 23-16.
            0x005C => (self.artic_counter() & 0xFF) as u8,
            0x005D | 0x005E => ((self.artic_counter() >> 8) & 0xFF) as u8,
            0x005F => ((self.artic_counter() >> 16) & 0xFF) as u8,

            // Printer i8255 PPI Port A - data latch (read).
            0x40 => self.printer.read_data(),

            // i8255 PPI Port B - system configuration status (read-only).
            // BUSY# (bit 2) is composed from the printer device's ready state.
            0x42 => {
                let mut value = self.system_ppi.read_port_b();
                if self.printer.is_ready() {
                    value |= 0x04;
                }
                value
            }

            // Printer i8255 PPI Port C - printer control (read).
            0x44 => self.printer.read_port_c(),

            // PC-9801F 320KB FDD PPI host interface.
            0x0051 if self.machine_model.has_fdd320_ppi() => self.fdd320_ppi.read_port_a(),
            0x0055 if self.machine_model.has_fdd320_ppi() => self.fdd320_ppi.read_port_c(),

            // Keyboard µPD8251A data register (port 0x41 read).
            0x41 => {
                let (data, clear_irq, retrigger_irq) = self.keyboard.read_data();
                if clear_irq {
                    self.pic.clear_irq(1);
                    self.tracer.trace_irq_clear(1);
                }
                if retrigger_irq {
                    self.pic.set_irq(1);
                    self.tracer.trace_irq_raise(1);
                }
                if clear_irq || retrigger_irq {
                    self.keyboard_chained_raw_code = Some(data);
                }
                data
            }
            // Keyboard µPD8251A status register (port 0x43 read).
            0x43 => self.keyboard.read_status(),

            // GDC master (text)
            0x60 => {
                let cycles_until_frame_event = self
                    .next_gdc_frame_event_cycle()
                    .map(|cycle| cycle.saturating_sub(self.current_cycle));
                self.gdc_master
                    .update_hblank_status(cycles_until_frame_event);
                self.gdc_master.read_status()
            }
            0x62 => {
                let value = self.gdc_master.read_data();
                self.feed_gdc_master_rdat();
                value
            }

            // Video mode
            0x68 => self.display_control.read_video_mode(),
            // Some software probes this odd-address alias as open bus.
            0x69 => 0xFF,

            // GRCG mode register read.
            // undoc98 io_disp.txt line 1118 says "READ: なし" (write-only).
            // MAME returns 0xFF. NP21W returns the actual mode register value.
            0x7C if self.machine_model.has_grcg() => self.grcg.state.mode,

            // PIT
            0x71 | 0x3FD9 => self.pit.read_counter(
                0,
                self.current_cycle,
                self.clocks.cpu_clock_hz,
                self.clocks.pit_clock_hz,
            ),
            0x73 | 0x3FDB => self.pit.read_counter(
                1,
                self.current_cycle,
                self.clocks.cpu_clock_hz,
                self.clocks.pit_clock_hz,
            ),
            0x75 | 0x3FDD => self.pit.read_counter(
                2,
                self.current_cycle,
                self.clocks.cpu_clock_hz,
                self.clocks.pit_clock_hz,
            ),

            // SASI hard disk controller
            0x80 if self.machine_model.has_sasi() => self.sasi.read_data(),
            0x82 if self.machine_model.has_sasi() => self.sasi.read_status(),

            // GDC slave (graphics)
            0xA0 => {
                let cycles_until_frame_event = self
                    .next_gdc_frame_event_cycle()
                    .map(|cycle| cycle.saturating_sub(self.current_cycle));
                self.gdc_slave
                    .update_hblank_status(cycles_until_frame_event);
                self.gdc_slave.read_status()
            }
            0xA2 => {
                if self.gdc_slave.state.dma_active && !self.gdc_slave.state.dma_is_write {
                    self.read_gdc_slave_dmar()
                } else {
                    self.gdc_slave.read_data()
                }
            }

            // Palette register reads (ports 0xA8/0xAA/0xAC/0xAE).
            0xA8 => {
                if self.pegc.is_256_color_active() {
                    self.pegc.state.palette_index
                } else if self.display_control.is_palette_analog_mode() {
                    self.palette.read_index()
                } else {
                    self.palette.read_digital(0)
                }
            }
            // CGROM glyph data read
            0xA9 => {
                if let Some(extractor) = self.text_extractor.as_deref_mut() {
                    self.cgrom.notify_read(extractor);
                }
                if let Some(addr) = self
                    .cgrom
                    .read_address(self.display_control.is_kac_dot_access_mode())
                {
                    self.memory.font_read(addr)
                } else {
                    0
                }
            }
            0xAA => {
                if self.pegc.is_256_color_active() {
                    self.pegc.read_palette_component(0)
                } else if self.display_control.is_palette_analog_mode() {
                    self.palette.read_analog(0)
                } else {
                    self.palette.read_digital(1)
                }
            }
            0xAC => {
                if self.pegc.is_256_color_active() {
                    self.pegc.read_palette_component(1)
                } else if self.display_control.is_palette_analog_mode() {
                    self.palette.read_analog(1)
                } else {
                    self.palette.read_digital(2)
                }
            }
            0xAE => {
                if self.pegc.is_256_color_active() {
                    self.pegc.read_palette_component(2)
                } else if self.display_control.is_palette_analog_mode() {
                    self.palette.read_analog(2)
                } else {
                    self.palette.read_digital(3)
                }
            }

            // FDC uPD765A - 1MB interface (active when PORT EXC = 1).
            0x90 => {
                if self.floppy.port_exc_is_1mb() {
                    self.floppy.fdc_1mb_mut().read_status()
                } else {
                    0xFF
                }
            }
            0x92 => {
                if self.floppy.port_exc_is_1mb() {
                    self.floppy.fdc_1mb_mut().read_data()
                } else {
                    0xFF
                }
            }
            0x94 => {
                if self.floppy.port_exc_is_1mb() {
                    FDC_1MB_INPUT_REGISTER
                } else {
                    0xFF
                }
            }
            // FDC uPD765A - 640KB interface (active when PORT EXC = 0).
            0xC8 => {
                if !self.floppy.port_exc_is_1mb() {
                    self.floppy.fdc_640k_mut().read_status()
                } else {
                    0xFF
                }
            }
            0xCA => {
                if !self.floppy.port_exc_is_1mb() {
                    self.floppy.fdc_640k_mut().read_data()
                } else {
                    0xFF
                }
            }
            0xCC => {
                if !self.floppy.port_exc_is_1mb() {
                    FDC_640K_INPUT_REGISTER
                } else {
                    0xFF
                }
            }

            // VRAM drawing page select.
            0xA6 => self.display_control.read_access_page(),

            // Dual-mode FDC interface control. Bits 0-1 reflect effective mode
            // after hardware density detection override.
            0xBE => (self.floppy.effective_fdc_media() & 3) | FDC_MEDIA_READ_FIXED_BITS,

            // PC-9801-14 PPI port A / dual-board 26K alternate status.
            0x0088 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref sb14) = self.soundboard_14 {
                    sb14.read_port_a()
                } else if self.soundboard_86.is_some()
                    && let Some(soundboard_26k) = self.soundboard_26k.as_mut()
                {
                    let value = soundboard_26k.read_status(self.current_cycle);
                    self.process_soundboard_actions();
                    value
                } else {
                    0xFF
                }
            }
            // PC-9801-14 PPI port B / dual-board 26K alternate data.
            0x008A => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref sb14) = self.soundboard_14 {
                    sb14.read_port_b()
                } else if let Some(sb26k) = self.soundboard_26k.as_mut()
                    && self.soundboard_86.is_some()
                {
                    if sb26k.address() == 0x0E {
                        let irq_bits = match sb26k.irq_line() {
                            3 => 0x00,
                            13 => 0x01,
                            10 => 0x02,
                            12 => 0x03,
                            _ => 0x03,
                        };
                        (irq_bits << 6) | 0x3F
                    } else {
                        let value = sb26k.read_data(self.current_cycle);
                        self.process_soundboard_actions();
                        value
                    }
                } else {
                    0xFF
                }
            }
            // PC-9801-14 PPI port C (TMS3631 key data latch).
            0x008C => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref sb14) = self.soundboard_14 {
                    sb14.read_port_c()
                } else {
                    0xFF
                }
            }
            // PC-9801-14 PPI DIP switch (I/O-address selector).
            0x008E => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref sb14) = self.soundboard_14 {
                    sb14.read_dip_switch()
                } else {
                    0xFF
                }
            }

            // FM sound board status (OPN / OPNA low bank), or 14-board
            // channel-enable mask readback.
            0x0188 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref sb14) = self.soundboard_14 {
                    sb14.read_enable_mask()
                } else if let Some(ref mut sb86) = self.soundboard_86 {
                    let value = sb86.read_status(self.current_cycle);
                    self.process_soundboard_86_actions();
                    value
                } else if let Some(ref mut sb26k) = self.soundboard_26k {
                    let value = sb26k.read_status(self.current_cycle);
                    self.process_soundboard_actions();
                    value
                } else {
                    0xFF
                }
            }
            // FM sound board data read (OPN / OPNA low bank), or 14-board
            // mirror of 0x0188 (enable mask).
            0x018A => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref sb14) = self.soundboard_14 {
                    sb14.read_enable_mask()
                } else if let Some(ref mut sb86) = self.soundboard_86 {
                    let value = sb86.read_data(self.current_cycle);
                    self.process_soundboard_86_actions();
                    value
                } else if let Some(ref mut sb26k) = self.soundboard_26k {
                    if sb26k.address() == 0x0E {
                        let irq_bits = match sb26k.irq_line() {
                            3 => 0x00,
                            13 => 0x01,
                            10 => 0x02,
                            12 => 0x03,
                            _ => 0x03,
                        };
                        (irq_bits << 6) | 0x3F
                    } else {
                        let value = sb26k.read_data(self.current_cycle);
                        self.process_soundboard_actions();
                        value
                    }
                } else {
                    0xFF
                }
            }
            // OPNA extended status (high bank), or 14-board 8253 counter #2
            // readback.
            0x018C => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref sb14) = self.soundboard_14 {
                    sb14.read_pit_counter()
                } else if let Some(ref mut sb86) = self.soundboard_86 {
                    let value = sb86.read_status_hi(self.current_cycle);
                    self.process_soundboard_86_actions();
                    value
                } else {
                    0xFF
                }
            }
            // OPNA extended data read (high bank), or 14-board strap
            // switch (INT5 = 0x80).
            0x018E => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref sb14) = self.soundboard_14 {
                    sb14.read_strap_switch()
                } else if let Some(ref mut sb86) = self.soundboard_86 {
                    sb86.read_data_hi(self.current_cycle)
                } else {
                    0xFF
                }
            }

            // System status register.
            // Bit 5: 1 = no built-in IDE HDD (PC-9821 only).
            0xF0 => {
                let mut status = SYSTEM_STATUS_DEFAULT;
                if self.machine_model.has_ide() && !self.ide.has_any_hdd() {
                    status |= 0x20;
                }
                status
            }
            // A20 gate state: bit 0 = 1 when masked (disabled), 0 when unmasked (enabled).
            0xF2 => 0xFF - (self.a20_enabled as u8),
            // A20 line control status (386+ CPU port).
            // Bit 0: A20 mask state (1=masked), bit 1: NMI enable (1=enabled).
            0xF6 => {
                if self.machine_model.has_a20_nmi_port() {
                    (!self.a20_enabled as u8) | ((self.nmi_enabled as u8) << 1)
                } else {
                    0xFF
                }
            }

            // Hi-res/normal mode detection (bit 2: 1=normal, 0=hi-res).
            // All PC-9801 targets use normal mode (640x400/640x200).
            // Hi-res (1120x750) is only on PC-H98, PC-98XA/XL/RL, and some PC-9821 models.
            0x0431 => MODE_DETECT_NORMAL,

            // DMA access control (bit 2: mask DMA above 1MB).
            0x0439 => self.dma_access_ctrl,

            // Expansion-slot socket processing latch.
            // Ref: undoc98 `io_memsys.txt`: bits 3:0 = sockets B0h/A0h/90h/80h processing active.
            0x043A => {
                let value = self.external_interrupt_43a;
                debug!("Port 0x043A (expansion socket processing latch) read: {value:#04X}");
                value
            }

            // 15M hole control readback (stub - not yet implemented).
            0x043B => {
                warn!(
                    "unhandled read from 15M hole control port 0x043B (returning {:#04X})",
                    self.hole_15m_control
                );
                self.hole_15m_control
            }

            // ROM bank select / cache hit status readback.
            // On 486+ machines bit 2 = cache hit status (1=hit, 0=miss).
            // The ITF tests cache by reading memory then checking this bit.
            // Returning 0x00 (no cache hits) is correct for 386 (no on-chip
            // cache) and satisfies the ITF XOR-based cache test, which expects
            // bit 2 = 0 on non-first reads and inverts bit 2 on the first.
            // Ref: undoc98 `io_mem.txt` lines 240-260.
            0x043D => 0x00,

            // Port 0x043E: not a documented I/O port.
            // Some BIOS revisions probe it during POST; ignore silently.
            0x043E => {
                debug!("Silently ignore read to undocumented port 0x043E");
                0xFF
            }

            // Protected memory registration readback (VX and later only).
            0x0567 => {
                if self.machine_model.has_protected_memory_register() {
                    self.protected_memory_max
                } else {
                    0xFF
                }
            }

            // SCSI controller (WD33C93) ports - no SCSI present.
            // Ref: undoc98 `io_scsi.txt`
            0x0CC0 | 0x0CC2 | 0x0CC4 => 0xFF,

            // Key-down sense probe latch.
            0x00EC => {
                let value = self.key_sense_0ec;
                debug!("Port 0x00EC (undocumented key-down sense latch) read: {value:#04X}");
                value
            }

            // PCM86 DAC ports.
            0xA460 | 0xA462 | 0xA464 | 0xA466 | 0xA468 | 0xA46A | 0xA46C | 0xA46E => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb86) = self.soundboard_86 {
                    let pcm86_pending =
                        self.scheduler.state.fire_cycles[Event98::Pcm86Irq as usize].is_some();
                    sb86.pcm86_read(
                        port,
                        self.current_cycle,
                        self.clocks.cpu_clock_hz,
                        pcm86_pending,
                    )
                } else {
                    0xFF
                }
            }

            // PCM86 mute control port.
            0xA66E => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb86) = self.soundboard_86 {
                    let pcm86_pending =
                        self.scheduler.state.fire_cycles[Event98::Pcm86Irq as usize].is_some();
                    sb86.pcm86_read(
                        port,
                        self.current_cycle,
                        self.clocks.cpu_clock_hz,
                        pcm86_pending,
                    )
                } else {
                    0xFF
                }
            }

            // Mouse interface i8255 PPI ports (read).
            0x7FD9 => self.mouse_ppi.read_port_a(self.current_cycle),
            0x7FDB => self.mouse_ppi.read_port_b(),
            0x7FDD => self.mouse_ppi.read_port_c(),
            // Hi-reso 8255A mouse port range. Not decoded on the normal-mode
            // machines we emulate.
            0x0061 | 0x0063 | 0x0065 | 0x0067 => 0xFF,
            // Mouse interrupt timer setting (write-only on PC-9801VM).
            0xBFDB => 0xFF,

            // Alternate FM sound board base addresses (0x0288, 0x0388).
            // Games probe these to detect sound hardware at non-primary bases.
            // No hardware present - return 0xFF silently.
            0x0288 | 0x028A | 0x028C | 0x028E | 0x0388 | 0x038A | 0x038C | 0x038E => 0xFF,

            0x09A0 => {
                if self.machine_model.has_pegc() {
                    let mut result = match self.video_ff2_index {
                        0x00 => 0xFF,
                        0x04 => (self.display_control.state.mode2 & 1) as u8,
                        0x07 => ((self.display_control.state.mode2 >> 2) & 1) as u8,
                        0x08 => ((self.display_control.state.mode2 >> 3) & 1) as u8,
                        0x0A => u8::from(self.pegc.is_256_color_active()),
                        0x0B => u8::from(self.pegc.is_packed_pixel_mode()),
                        0x0D => u8::from(
                            self.pegc.state.screen_mode == device::pegc::PegcScreenMode::OneScreen,
                        ),
                        _ => 0,
                    };
                    if self.display_control.state.mode2 & (1 << 10) != 0 {
                        result |= 0x02;
                    }
                    result
                } else {
                    0xFF
                }
            }

            // 31 kHz horizontal scan-frequency register (port 09A8h).
            // 0 = 24.823 kHz / 400 lines, 1 = 31.778 kHz / 480 lines.
            0x09A8 => u8::from(self.display_control.is_crt_31khz_enabled()),

            // IDE bank status and select registers, presence detection
            0x0430 if self.machine_model.has_ide() => self.ide.read_bank0_status(),
            0x0432 if self.machine_model.has_ide() => self.ide.read_bank(1),
            0x0433 if self.machine_model.has_ide() => self.ide.read_presence(),
            0x0435 if self.machine_model.has_ide() => self.ide.read_additional_status(),

            // IDE CS0 registers
            0x0642 if self.machine_model.has_ide() => self.ide.read_error(),
            0x0644 if self.machine_model.has_ide() => self.ide.read_sector_count(),
            0x0646 if self.machine_model.has_ide() => self.ide.read_sector_number(),
            0x0648 if self.machine_model.has_ide() => self.ide.read_cylinder_low(),
            0x064A if self.machine_model.has_ide() => self.ide.read_cylinder_high(),
            0x064C if self.machine_model.has_ide() => self.ide.read_device_head(),
            0x064E if self.machine_model.has_ide() => {
                let (status, clear_irq) = self.ide.read_status();
                if clear_irq {
                    self.pic.clear_irq(9);
                }
                status
            }

            // IDE CS1 registers
            0x074C if self.machine_model.has_ide() => self.ide.read_alt_status(),
            0x074E if self.machine_model.has_ide() => self.ide.read_digital_input(),

            // IDE BIOS work area mapping (DA000-DBFFF).
            0x1E8E if self.machine_model.has_ide() => self.ide.read_work_area_port(),

            // SIMM memory controller.
            // Ref: undoc98 `io_mem.txt` (ports 0x0530/0x0531)
            0x0530 if self.machine_model.is_pc9821() => self.simm_address_register,
            0x0531 if self.machine_model.is_pc9821() => {
                let index = self.simm_address_register as usize;
                let socket = index & 0x0F;
                let is_limit = index & 0x80 != 0;
                let data_index = socket * 2 + is_limit as usize;
                if data_index < self.simm_data.len() {
                    self.simm_data[data_index]
                } else {
                    0xFF
                }
            }

            // Memory bank switching register.
            // Ref: undoc98 `io_mem.txt` (port 0x063C)
            0x063C if self.machine_model.is_pc9821() => self.memory_bank_063c,

            // Flash ROM power voltage control (stub).
            // Ref: undoc98 `io_mem.txt` (port 0x063E)
            0x063E if self.machine_model.is_pc9821() => 0x00,

            // CPU/cache control register.
            // Ref: undoc98 `io_mem.txt` (port 0x063F)
            0x063F if self.machine_model.is_pc9821() => self.cache_control_063f,

            // IDE bank select register (port 0x0436).
            0x0436 if self.machine_model.is_pc9821() => 0xFF,

            // Window Accelerator Board (WAB) - built-in graphics accelerator.
            // Ref: undoc98 `io_wab.txt`
            0x0FAA if self.machine_model.is_pc9821() => self.wab_index,
            0x0FAB if self.machine_model.is_pc9821() => {
                let index = self.wab_index as usize;
                if index < self.wab_data.len() {
                    self.wab_data[index]
                } else {
                    0xFF
                }
            }
            0x0FAC if self.machine_model.is_pc9821() => self.wab_relay,

            // CPU mode / wait control register.
            // Ref: undoc98 `io_cpu.txt` (port 0x0534)
            0x0534 if self.machine_model.is_pc9821() => self.cpu_mode_534,

            // Undocumented PC-9821 system register (probed by Windows 95).
            0x0549 if self.machine_model.is_pc9821() => 0xFF,

            // Memory status register (read-only).
            // Bit 7,6 = 11 -> no 2nd cache RAM board.
            // Ref: undoc98 `io_mem.txt` (port 0x063D)
            0x063D if self.machine_model.is_pc9821() => 0xFF,

            // Hardware wait timing adjustment register.
            // Ref: undoc98 `io_tstmp.txt` (port 0x045F)
            0x045F if self.machine_model.is_pc9821() => 0x00,

            // Graphics accelerator presence detection.
            // 0xFF = no CL-GD5428/5430 accelerator present.
            // Ref: undoc98 `io_wab.txt` (port 0x0CA0)
            0x0CA0 if self.machine_model.is_pc9821() => 0xFF,

            // MATE-X PCM sound ports (stub, no MATE-X hardware).
            0xAC6C..=0xAC6F if self.machine_model.is_pc9821() => 0xFF,

            // Mystery I/O ports (banking mechanism, undocumented).
            // NP21W labels these as "謎のI/Oポート" in pcidev.c.
            0x18F0 if self.machine_model.is_pc9821() => 0x00,
            0x18F1 if self.machine_model.is_pc9821() => 0x00,
            0x18F2 if self.machine_model.is_pc9821() => 0x00,
            0x18F3 if self.machine_model.is_pc9821() => 0x00,

            // Software DIP Switch (SDIP) - ports 0x841E–0x8F1E at 0x100 stride.
            // Ref: undoc98 `io_sdip.txt`
            port if self.machine_model.has_sdip()
                && (port & 0xFF) == 0x1E
                && (0x841E..=0x8F1E).contains(&port) =>
            {
                let offset = ((port >> 8) as usize & 0x0F) - 4;
                self.sdip.read(offset)
            }

            // Extended RS-232C channel 2/3 status (board detection).
            // Ref: undoc98 `io_rs.txt` - 0xFF = no extended serial board present.
            0xB3 | 0xBB => 0xFF,

            // NOTE series notebook control register (not present on desktop models).
            // Ref: undoc98 `io_note.txt` (port 0xBE8E)
            0xBE8E => 0xFF,

            // Undocumented port (probed by Windows 3.1 during hardware detection).
            0x0879 => 0xFF,

            // Epson PC-98 compatible system control ports (machine detection).
            // 0xFF = not an Epson machine.
            0x0C00 | 0x0C01 | 0x0C02 | 0x0C05 => 0xFF,

            // Undocumented ports (probed by Windows 95 during hardware detection).
            0x0D00 | 0x0D01 | 0x0D02 | 0x0D05 | 0x0D0A => 0xFF,

            // Undocumented system register ports (probed by Windows 95).
            0x8D08 | 0x8D0E => 0xFF,

            // Video Board (PC-9801-72 / PC-98GS-02) detection ports.
            // Ref: undoc98 `io_vbrd.txt` - 0xFF = board not present.
            0xAF66 | 0xAF67 | 0xAF6A | 0xAF6B => 0xFF,

            // Sound Blaster 16 OPL3 status (base+0x2000, base+0x2800).
            0x20D2 | 0x28D2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb16) = self.sound_blaster_16 {
                    let v = sb16.read_opl3_status(self.current_cycle);
                    self.process_soundboard_sb16_actions();
                    v
                } else {
                    0xFF
                }
            }
            // Sound Blaster 16 OPL3 data ports (base+0x2100, base+0x2900) - write-only.
            0x21D2 | 0x29D2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                0xFF
            }
            // Sound Blaster 16 OPL3 bank-1 address write / status read (base+0x2200).
            // YMF262 has a single status register accessible from either bank, so
            // mirror the bank-0 status read.
            0x22D2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb16) = self.sound_blaster_16 {
                    let v = sb16.read_opl3_status(self.current_cycle);
                    self.process_soundboard_sb16_actions();
                    v
                } else {
                    0xFF
                }
            }
            // Sound Blaster 16 OPL3 bank-1 data write (base+0x2300) - write-only.
            0x23D2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                0xFF
            }
            // Misaligned IN AX, DX issued by SimCity 2000's OPL3 probe at the
            // low byte adjacent to the bank-0 / bank-1 data write ports; the
            // high byte hits 0x21D2 / 0x23D2 above. Return open bus without
            // warning.
            0x21D1 | 0x23D1 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                0xFF
            }
            // Sound Blaster 16 mixer address (base+0x2400).
            0x24D2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref sb16) = self.sound_blaster_16 {
                    sb16.read_mixer_address()
                } else {
                    0xFF
                }
            }
            // Sound Blaster 16 mixer data (base+0x2500).
            0x25D2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref sb16) = self.sound_blaster_16 {
                    sb16.read_mixer_data()
                } else {
                    0xFF
                }
            }
            // Sound Blaster 16 DSP reset (base+0x2600).
            0x26D2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref sb16) = self.sound_blaster_16 {
                    sb16.read_dsp_reset()
                } else {
                    0xFF
                }
            }
            // Sound Blaster 16 DSP read data (base+0x2A00).
            0x2AD2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb16) = self.sound_blaster_16 {
                    sb16.read_dsp_data()
                } else {
                    0xFF
                }
            }
            // Sound Blaster 16 DSP write status (base+0x2C00).
            0x2CD2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref sb16) = self.sound_blaster_16 {
                    sb16.read_dsp_write_status()
                } else {
                    0xFF
                }
            }
            // Sound Blaster 16 DSP read status 8-bit / IRQ ack (base+0x2E00).
            0x2ED2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb16) = self.sound_blaster_16 {
                    let v = sb16.read_dsp_status_8bit();
                    self.process_soundboard_sb16_actions();
                    v
                } else {
                    0xFF
                }
            }
            // Sound Blaster 16 DSP read status 16-bit / IRQ ack (base+0x2F00).
            0x2FD2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb16) = self.sound_blaster_16 {
                    let v = sb16.read_dsp_status_16bit();
                    self.process_soundboard_sb16_actions();
                    v
                } else {
                    0xFF
                }
            }

            // MPU-PC98II MIDI interface (C-Bus, default base 0xE0D0).
            0xE0D0 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                let value = self.mpu401.read_data();
                self.sync_mpu_irq_and_timer();
                value
            }
            0xE0D2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                self.mpu401.read_status()
            }

            // C-Bus expansion card probing (no extension hardware present).
            0xC0E0..=0xFCE2 => 0xFF,

            _ => {
                self.tracer.trace_io_unhandled_read(port);
                warn!("Unhandled I/O read: port={port:#06X}");
                0xFF
            }
        };
        self.tracer.trace_io_read(port, value);
        value
    }

    fn artic_counter(&self) -> u32 {
        let ticks = (u128::from(self.current_cycle) * 307_200u128
            / u128::from(self.clocks.cpu_clock_hz)) as u32;
        ticks & 0x00FF_FFFF
    }

    fn read_gdc_slave_dmar(&mut self) -> u8 {
        let vram_word = self
            .gdc_slave
            .dmar_next_address()
            .map(|address| self.read_gdc_vram_word_from_access_page(address));
        self.gdc_slave.dack_read(vram_word)
    }
}

#[cfg(test)]
mod tests {
    use common::{Bus, CpuMode, MachineModel, Tracing};

    use crate::bus::{NoTracing, Pc9801Bus};

    #[derive(Default)]
    struct UnhandledTrace {
        reads: Vec<u16>,
    }

    impl Tracing for UnhandledTrace {
        fn trace_io_unhandled_read(&mut self, port: u16) {
            self.reads.push(port);
        }
    }

    #[test]
    fn port_7c_read_returns_grcg_mode_register() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);

        bus.grcg.write_mode(0xCA);
        let value = bus.io_read_byte(0x7C);
        assert_eq!(value, 0xCA);

        bus.grcg.write_mode(0x80);
        let value = bus.io_read_byte(0x7C);
        assert_eq!(value, 0x80);
    }

    #[test]
    fn port_a6_read_returns_access_page() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);

        assert_eq!(bus.io_read_byte(0xA6), 0x00);

        bus.io_write_byte(0xA6, 0x01);
        assert_eq!(bus.io_read_byte(0xA6), 0x01);

        // Only bit 0 matters.
        bus.io_write_byte(0xA6, 0xFE);
        assert_eq!(bus.io_read_byte(0xA6), 0x00);
    }

    #[test]
    fn port_0cc4_returns_ff() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        assert_eq!(bus.io_read_byte(0x0CC4), 0xFF);
    }

    #[test]
    fn sound_ports_0288_and_0388_do_not_alias_low_bank() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.install_soundboard_86(None, true);

        // Primary base 0x0188 works: reg 0xFF returns chip ID 0x01.
        bus.io_write_byte(0x0188, 0xFF);
        assert_eq!(bus.io_read_byte(0x018A), 0x01);

        // Ports 0x0288 and 0x0388 must not reach the sound board.
        assert_eq!(bus.io_read_byte(0x028A), 0xFF);
        assert_eq!(bus.io_read_byte(0x038A), 0xFF);
    }

    #[test]
    fn pc9801f_fdd320_ppi_shim_reads_are_handled() {
        let mut bus = Pc9801Bus::<UnhandledTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);

        assert_eq!(bus.io_read_byte(0x0051), 0x00);
        assert_eq!(bus.io_read_byte(0x0055), 0x00);
        assert_eq!(bus.io_read_byte(0x0055), 0xFF);

        assert_eq!(bus.tracer().reads, Vec::<u16>::new());
    }

    #[test]
    fn fdd320_ppi_reads_are_not_exposed_on_later_models() {
        let mut bus =
            Pc9801Bus::<UnhandledTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

        assert_eq!(bus.io_read_byte(0x0055), 0xFF);

        assert_eq!(bus.tracer().reads, vec![0x0055]);
    }
}
