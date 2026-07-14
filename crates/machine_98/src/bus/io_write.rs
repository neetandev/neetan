use common::{MachineModel, debug, warn};
use device::{
    ga1280a::is_ga1280a_port,
    grcg,
    i8253_pit::{PIT_FLAG_I, WriteResult},
    sasi::{SasiAction, SasiPhase},
    upd765a_fdc::FdcAction,
    upd7220_gdc,
};
use upd7220_gdc::{DOT_CLOCK_200LINE, DOT_CLOCK_400LINE, GdcAction, STATUS_DRAWING, VramOp};

use crate::{
    Pc9801Bus, Tracing,
    bus::{INTERRUPT_DELAY_CYCLES, IO_WAIT_CYCLES, MOUSE_TIMER_IRQ_LINE},
    scheduler::Event98,
};

impl<T: Tracing> Pc9801Bus<T> {
    #[inline]
    pub(super) fn io_write_byte_impl(&mut self, port: u16, value: u8) {
        self.pending_wait_cycles += IO_WAIT_CYCLES;
        self.tracer.set_cycle(self.current_cycle);
        self.tracer.trace_io_write(port, value);
        if is_ga1280a_port(port) {
            self.pending_wait_cycles += self.cbus_wait_cycles();
            if let Some(ga) = self.ga1280a.as_mut() {
                ga.try_handle_io_write_byte(port, value);
            }
            return;
        }
        match port {
            // PIC
            0x00 | 0x08 => self.pic.write_port0(((port >> 3) & 1) as usize, value),
            0x02 | 0x0A => self.pic.write_port2(((port >> 3) & 1) as usize, value),

            // DMA channel registers
            0x01 => self.dma.write_address(0, value),
            0x03 => self.dma.write_count(0, value),
            0x05 => self.dma.write_address(1, value),
            0x07 => self.dma.write_count(1, value),
            0x09 => self.dma.write_address(2, value),
            0x0B => self.dma.write_count(2, value),
            0x0D => self.dma.write_address(3, value),
            0x0F => self.dma.write_count(3, value),
            0x11 => self.dma.write_command(value),
            // DMA request register (software-triggered DMA, not used).
            0x13 => {}
            0x15 => self.dma.write_single_mask(value),
            0x17 => self.dma.write_mode(value),
            0x19 => self.dma.clear_flip_flop(),
            0x1B => self.dma.master_clear(),
            0x1D => self.dma.state.mask = 0,
            0x1F => self.dma.write_all_mask(value),
            0x21 => self.dma.write_page(1, value & 0x0F),
            0x23 => self.dma.write_page(2, value & 0x0F),
            0x25 => self.dma.write_page(3, value & 0x0F),
            0x27 => self.dma.write_page(0, value & 0x0F),
            // DMA auto-increment boundary mode register.
            0x29 => self.dma.write_bound(value),

            // Undocumented RTC control/mode latch.
            0x22 => {
                self.rtc_control_22 = value;
            }

            // System port C
            0x35 => {
                self.system_ppi.write_port_c(value);
                self.beeper
                    .set_buzzer_enabled(value & 0x08 == 0, self.current_cycle);
            }
            // i8255 PPI control port
            0x37 => {
                self.system_ppi.write_control(value);
                let enabled = self.system_ppi.state.port_c & 0x08 == 0;
                self.beeper.set_buzzer_enabled(enabled, self.current_cycle);
            }

            // RS-232C i8251 data register.
            0x30 => self.serial.write_data(value),
            // RS-232C i8251 mode/command register.
            0x32 => self.serial.write_command(value),

            // Printer i8255 PPI Port A - data latch (write).
            0x40 => self.printer.write_data(value),

            // Keyboard µPD8251A data register (port 0x41 write).
            0x41 => self.keyboard.write_data(value),
            // Keyboard µPD8251A control register (port 0x43 write).
            0x43 => self.keyboard.write_command(value),

            // Printer i8255 PPI Port C - printer control (write).
            0x44 => self.printer.write_port_c(value),
            // Printer i8255 PPI control register (write-only).
            0x46 => self.printer.write_control(value),

            // PC-9801F 320KB FDD PPI host interface.
            0x0053 if self.machine_model.has_fdd320_ppi() => self.fdd320_ppi.write_port_b(value),
            0x0057 if self.machine_model.has_fdd320_ppi() => self.fdd320_ppi.write_control(value),

            // NMI control
            0x50 => self.nmi_enabled = false,
            0x52 => self.nmi_enabled = true,

            // GDC master (text)
            0x60 => {
                let action = self.gdc_master.write_data(value);
                self.handle_gdc_master_action(action);
            }
            0x62 => {
                let action = self.gdc_master.write_command(value);
                self.handle_gdc_master_action(action);
            }

            // VSYNC IRQ control and acknowledge
            0x64 => {
                self.display_control.write_vsync_control(value);
            }

            // Video mode
            0x68 => {
                self.display_control.write_video_mode(value);
                self.update_plane_e_mapping();
            }
            // Mode F/F odd-address alias: ignored.
            0x69 => {}
            // Mode register 2 (GDC clock + color depth + accelerator mode).
            0x6A => {
                let has_egc = self.grcg.state.chip == grcg::GRCG_CHIP_EGC;
                self.display_control.write_mode2(value, has_egc);
                if self.machine_model.has_pegc() {
                    match value {
                        0x20 if self.display_control.is_egc_mode_change_permitted() => {
                            self.pegc.set_256_color_enabled(false);
                            self.update_pegc_mapping();
                        }
                        0x21 if self.display_control.is_egc_mode_change_permitted() => {
                            self.pegc.set_256_color_enabled(true);
                            self.update_pegc_mapping();
                        }
                        0x68 => self.pegc.set_screen_mode(false),
                        0x69 => self.pegc.set_screen_mode(true),
                        _ => {}
                    }
                }
                self.update_plane_e_mapping();
                self.apply_gdc_dot_clock();
                self.reschedule_gdc_events();
            }
            // Border color.
            0x6C => self.display_control.write_border_color(value),
            // Display line count (CRT frequency select).
            0x6E => {
                self.display_control.write_display_line_count(value);
                self.apply_gdc_dot_clock();
                self.reschedule_gdc_events();
            }

            // CRTC uPD52611 text line counter registers.
            0x70 | 0x72 | 0x74 | 0x76 | 0x78 | 0x7A => {
                self.crtc
                    .write_register(((port - 0x70) >> 1) as usize, value);
            }

            // ARTIC hardware wait: `OUT 0x005F,AL` inserts >=0.6 us delay.
            0x005F => {
                self.pending_wait_cycles += self.artic_wait_cycles();
            }

            // PIT
            0x71 | 0x3FD9 => self.write_pit_counter(0, value),
            0x73 | 0x3FDB => self.write_pit_counter(1, value),
            0x75 | 0x3FDD => self.write_pit_counter(2, value),
            0x77 | 0x3FDF => self.write_pit_control(value),

            // GRCG mode register - resets tile counter.
            0x7C if self.machine_model.has_grcg() => self.grcg.write_mode(value),
            // GRCG tile register - cycles through planes 0-3.
            0x7E if self.machine_model.has_grcg() => self.grcg.write_tile(value),

            // SASI hard disk controller
            0x80 if self.machine_model.has_sasi() => {
                let action = self.sasi.write_data(value);
                self.process_sasi_action(action);
            }
            0x82 if self.machine_model.has_sasi() => {
                let action = self.sasi.write_control(value);
                self.process_sasi_action(action);
            }

            // FDC uPD765A - 1MB interface (active when PORT EXC = 1).
            0x92 => {
                if self.floppy.port_exc_is_1mb() {
                    let action = self.floppy.fdc_1mb_mut().write_data(value);
                    self.handle_fdc_action(action, 0);
                }
            }
            0x94 => {
                if self.floppy.port_exc_is_1mb() {
                    self.floppy.fdc_1mb_mut().write_control(value);
                }
            }
            // FDC uPD765A - 640KB interface (active when PORT EXC = 0).
            0xCA => {
                if !self.floppy.port_exc_is_1mb() {
                    let action = self.floppy.fdc_640k_mut().write_data(value);
                    self.handle_fdc_action(action, 1);
                }
            }
            0xCC => {
                if !self.floppy.port_exc_is_1mb() {
                    self.floppy.fdc_640k_mut().write_control(value);
                }
            }

            // GDC slave (graphics)
            0xA0 => {
                let action = self.gdc_slave.write_data(value);
                self.handle_gdc_slave_action(action);
            }
            0xA2 => {
                let action = self.gdc_slave.write_command(value);
                self.handle_gdc_slave_action(action);
            }
            // Graphics display page select.
            0xA4 => {
                if !self.machine_model.has_pegc()
                    || self.pegc.state.screen_mode != device::pegc::PegcScreenMode::OneScreen
                {
                    self.display_control.write_display_page(value);
                }
            }
            // VRAM drawing page select.
            0xA6 => self.display_control.write_access_page(value),
            // Palette registers (mode-dependent via mode2 bit 0).
            // 16-color analog: 0xA8=index select, 0xAA=green, 0xAC=red, 0xAE=blue.
            // 8-color digital: all 4 ports store packed nibble pairs directly.
            0xA8 => {
                if self.pegc.is_256_color_active() {
                    self.pegc.write_palette_index(value);
                } else if self.display_control.is_palette_analog_mode() {
                    self.palette.write_index(value);
                } else {
                    self.palette.write_digital(0, value);
                }
            }
            0xAA => {
                if self.pegc.is_256_color_active() {
                    self.pegc.write_palette_component(0, value);
                } else if self.display_control.is_palette_analog_mode() {
                    self.palette.write_analog(0, value);
                } else {
                    self.palette.write_digital(1, value);
                }
            }
            0xAC => {
                if self.pegc.is_256_color_active() {
                    self.pegc.write_palette_component(1, value);
                } else if self.display_control.is_palette_analog_mode() {
                    self.palette.write_analog(1, value);
                } else {
                    self.palette.write_digital(2, value);
                }
            }
            0xAE => {
                if self.pegc.is_256_color_active() {
                    self.pegc.write_palette_component(2, value);
                } else if self.display_control.is_palette_analog_mode() {
                    self.palette.write_analog(2, value);
                } else {
                    self.palette.write_digital(3, value);
                }
            }
            // CGROM character code high byte
            0xA1 => self.cgrom.write_code_high(value),
            // CGROM character code low byte
            0xA3 => self.cgrom.write_code_low(value),
            // CGROM line selector + left/right half
            0xA5 => self.cgrom.write_line_selector(value),
            // CGROM glyph data write (user-definable range only)
            0xA9 => {
                if let Some(addr) = self
                    .cgrom
                    .write_address(self.display_control.is_kac_dot_access_mode())
                {
                    self.memory.font_write(addr, value);
                }
            }

            // A20 gate disable + V30 CPU mode switch + CPU reset/shutdown.
            0xF0 => {
                self.a20_enabled = false;
                if self.machine_model == MachineModel::PC9801VM {
                    self.system_ppi.set_cpu_mode_bit(false);
                }

                let port_c = self.system_ppi.state.port_c;
                let shut0 = port_c & 0x80 != 0;
                let shut1 = port_c & 0x20 != 0;

                match (shut0, shut1) {
                    (true, false) => {
                        debug!("System shutdown (SHUT0=1, SHUT1=0)");
                        self.shutdown_requested = true;
                        self.reset_pending = true;
                    }
                    (false, _) => {
                        debug!("Warm reset (SHUT0=0)");
                        self.warm_reset_context = Some(self.read_warm_reset_context());
                        self.needs_full_reinit = true;
                        self.reset_pending = true;
                    }
                    (true, true) => {
                        debug!("Cold reset (SHUT0=1, SHUT1=1)");
                        self.warm_reset_context = None;
                        self.needs_full_reinit = true;
                        self.reset_pending = true;
                    }
                }
            }
            // Dual-mode FDC interface control.
            0xBE => {
                self.floppy.set_fdc_media(value);
            }

            // A20 gate enable + restore V30 native mode (no reset).
            0xF2 => {
                self.a20_enabled = true;
                if self.machine_model == MachineModel::PC9801VM {
                    self.system_ppi.set_cpu_mode_bit(true);
                }
            }
            // A20 line control + SDIP bank select.
            // On PC-9821 first-gen and Ce, writes of 0xA0/0xE0 select the SDIP
            // bank (bit 6: 0 = front, 1 = back). Other values control A20/NMI.
            // Ref: undoc98 `io_sdip.txt`
            0xF6 => {
                if self.machine_model.has_sdip() && (value == 0xA0 || value == 0xE0) {
                    self.sdip.select_bank_from_bit6(value);
                } else if self.machine_model.has_a20_nmi_port() {
                    match value {
                        0x02 => self.a20_enabled = true,
                        0x03 => self.a20_enabled = false,
                        _ => {}
                    }
                }
            }

            // PC-9801-14 PPI port A write / dual-board 26K alternate address.
            0x0088 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb14) = self.soundboard_14 {
                    sb14.write_port_a(value);
                } else if let Some(sb26k) = self.soundboard_26k.as_mut()
                    && self.soundboard_86.is_some()
                {
                    sb26k.write_address(value, self.current_cycle);
                    self.process_soundboard_actions();
                }
            }
            // PC-9801-14 PPI port B write / dual-board 26K alternate data.
            0x008A => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb14) = self.soundboard_14 {
                    sb14.write_port_b(value);
                } else if let Some(sb26k) = self.soundboard_26k.as_mut()
                    && self.soundboard_86.is_some()
                {
                    sb26k.write_data(value, self.current_cycle);
                    self.process_soundboard_actions();
                }
            }
            // PC-9801-14 PPI port C write (TMS3631 key-data handshake).
            0x008C => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb14) = self.soundboard_14 {
                    sb14.write_port_c(value);
                }
            }
            // PC-9801-14 PPI mode register write.
            0x008E => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb14) = self.soundboard_14 {
                    sb14.write_ppi_mode(value);
                }
            }

            // FM sound board register select (OPN / OPNA low bank) or
            // PC-9801-14 channel-enable mask.
            0x0188 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb14) = self.soundboard_14 {
                    sb14.write_enable_mask(value);
                } else if let Some(ref mut sb86) = self.soundboard_86 {
                    sb86.write_address(value, self.current_cycle);
                    self.process_soundboard_86_actions();
                } else if let Some(ref mut sb26k) = self.soundboard_26k {
                    sb26k.write_address(value, self.current_cycle);
                    self.process_soundboard_actions();
                }
            }
            // FM sound board data write (OPN / OPNA low bank) or mirror
            // of 0x0188 on PC-9801-14.
            0x018A => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb14) = self.soundboard_14 {
                    sb14.write_enable_mask(value);
                } else if let Some(ref mut sb86) = self.soundboard_86 {
                    sb86.write_data(value, self.current_cycle);
                    self.process_soundboard_86_actions();
                } else if let Some(ref mut sb26k) = self.soundboard_26k {
                    sb26k.write_data(value, self.current_cycle);
                    self.process_soundboard_actions();
                }
            }
            // OPNA extended register select (high bank) or PC-9801-14
            // 8253 counter #2 reload.
            0x018C => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb14) = self.soundboard_14 {
                    sb14.write_pit_counter(value, self.current_cycle);
                    self.process_soundboard_14_actions();
                } else if let Some(ref mut sb86) = self.soundboard_86 {
                    sb86.write_address_hi(value, self.current_cycle);
                    self.process_soundboard_86_actions();
                }
            }
            // OPNA extended data write (high bank) or PC-9801-14 8253
            // control-word register.
            0x018E => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb14) = self.soundboard_14 {
                    sb14.write_pit_control(value, self.current_cycle);
                    self.process_soundboard_14_actions();
                } else if let Some(ref mut sb86) = self.soundboard_86 {
                    sb86.write_data_hi(value, self.current_cycle);
                    self.process_soundboard_86_actions();
                }
            }

            // DMA access control (bit 2: mask DMA above 1MB).
            0x0439 => {
                self.dma_access_ctrl = value;
            }
            // Expansion-slot socket processing latch.
            // Ref: undoc98 `io_memsys.txt`: bits 3:0 = sockets B0h/A0h/90h/80h processing active.
            0x043A => {
                debug!("Port 0x043A (expansion socket processing latch) write: {value:#04X}");
                self.external_interrupt_43a = value;
            }
            // 15M hole control (port 0x043B, stub - not yet implemented).
            0x043B => {
                warn!("unhandled write to 15M hole control port 0x043B: {value:#04X}");
                self.hole_15m_control = value;
            }
            // ROM bank select (port 0x043D).
            // 0x10/0x00/0x18 = ITF bank, 0x12 = BIOS bank.
            0x043D => {
                if self.machine_model.is_dual_bank_bios() {
                    match value {
                        0x00 | 0x10 | 0x18 => self.memory.select_banked_rom_window(false),
                        0x12 => self.memory.select_banked_rom_window(true),
                        _ => {}
                    }
                }
            }
            // Port 0x043E: not a documented I/O port. Ignore writes silently.
            0x043E => {}
            // VRAM/EMS banking.
            0x043F => {
                self.vram_ems_bank = value;
            }
            // RAM window.
            0x0461 => {
                self.ram_window = value;
            }
            // Shadow RAM control.
            0x053D => {
                if self.machine_model.has_shadow_ram() {
                    self.memory.set_shadow_control(value);
                }
            }
            // Protected memory registration.
            0x0567 => {
                if self.machine_model.has_protected_memory_register() {
                    self.protected_memory_max = value;
                }
            }

            // 640KB FDD HLE trap port.
            0x07ED if self.fdd640k_hle.rom_installed() => {
                self.fdd640k_hle.write_trap_port(value);
            }

            // SASI HLE trap port.
            0x07EF if self.machine_model.has_sasi() => {
                self.sasi.write_trap_port(value);
            }

            // IDE HLE trap port.
            0x07EE if self.machine_model.has_ide() => {
                self.ide.write_trap_port(value);
            }

            // BIOS HLE trap port.
            0x07F0 => {
                self.bios.write_trap_port(value);
                // TODO: Calibrate these by comparing against the real VM target BIOS calls,
                //       once we have a verified V30 cycle accurate core.
                let cost = match self.bios.pending_vector() {
                    0x09 | 0x0C | 0x12 | 0x13 => 50,
                    0x18 => 20,
                    0x19 | 0x1A | 0x1B | 0x1C | 0x1F => 20,
                    0x08 => 20,
                    0xF2 => 1000,
                    _ => 0,
                };
                self.pending_wait_cycles += cost;
            }

            // Hi-res mode notification to expansion boards (write-only).
            // Bit 1: 1 = hi-res mode, 0 = normal mode. PC-9801VM is always normal mode.
            0x0467 => {}
            // Mouse interface i8255 PPI ports (write).
            0x7FD9 => self.mouse_ppi.write_port_a(value),
            0x7FDB => self.mouse_ppi.write_port_b(value),
            0x7FDD => {
                let was_enabled = self.mouse_timer_irq_enabled();
                let hc_rising = self.mouse_ppi.write_port_c(value);
                if hc_rising {
                    self.mouse_ppi.latch(self.current_cycle);
                }
                self.handle_mouse_timer_control_change(was_enabled);
            }
            0x7FDF => {
                let was_enabled = self.mouse_timer_irq_enabled();
                let hc_rising = self.mouse_ppi.write_ctrl(value);
                if hc_rising {
                    self.mouse_ppi.latch(self.current_cycle);
                }
                self.handle_mouse_timer_control_change(was_enabled);
            }
            // Hi-reso 8255A mouse port range. Not decoded on the normal-mode
            // machines we emulate.
            0x0061 | 0x0063 | 0x0065 | 0x0067 => {}
            // Mouse interrupt timer setting.
            // Only bits 1:0 select frequency. Ignore writes with upper bits set
            // (games like jastrike write non-frequency values to this port).
            0xBFDB => {
                if value & 0xFC != 0 {
                    return;
                }
                self.mouse_timer_setting = value;
                if self.mouse_timer_irq_enabled() {
                    self.schedule_mouse_timer();
                    self.update_next_event_cycle();
                }
            }

            // Key-down sense probe latch.
            0x00EC => {
                debug!("Port 0x00EC (undocumented key-down sense latch) write: {value:#04X}");
                self.key_sense_0ec = value;
            }

            // µPD4990A RTC strobe/command (port 0x20 write).
            0x20 => {
                let host_time = (self.host_date_time_provider)().to_bcd_bytes();
                self.rtc.write_port(value, &host_time);
            }

            // PCM86 DAC ports.
            0xA460 | 0xA462 | 0xA464 | 0xA466 | 0xA468 | 0xA46A | 0xA46C | 0xA46E => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb86) = self.soundboard_86 {
                    sb86.pcm86_write(port, value, self.current_cycle, self.clocks.cpu_clock_hz);
                }
                self.process_soundboard_86_actions();
            }

            // PCM86 mute control port.
            0xA66E => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb86) = self.soundboard_86 {
                    sb86.pcm86_write(port, value, self.current_cycle, self.clocks.cpu_clock_hz);
                }
            }

            // EGC registers (byte access).
            0x04A0..=0x04AF => {
                if self.machine_model.has_egc()
                    && self.display_control.is_egc_extended_mode_effective()
                    && self.grcg.is_active()
                {
                    self.egc.write_register_byte((port & 0x0F) as u8, value);
                }
            }

            // DMA extended bank registers (A24-A31, 386+ only).
            0x0E05 | 0x0E07 | 0x0E09 | 0x0E0B => {
                if self.machine_model.has_extended_dma() {
                    let channel = ((port - 0x0E05) / 2) as usize;
                    self.dma.write_extended_page(channel, value);
                }
            }

            // Alternate FM sound board base addresses - silently ignore writes.
            0x0288 | 0x028A | 0x028C | 0x028E | 0x0388 | 0x038A | 0x038C | 0x038E => {}

            0x09A0 => {
                if self.machine_model.has_pegc() {
                    self.video_ff2_index = value;
                }
            }

            // 31 kHz horizontal scan-frequency register (port 09A8h).
            // 0 = 24.823 kHz / 400 lines, 1 = 31.778 kHz / 480 lines.
            0x09A8 => {
                self.display_control.set_crt_31khz_enabled(value & 1 != 0);
            }

            // IDE bank select
            0x0430 if self.machine_model.has_ide() => self.ide.write_bank(0, value),
            0x0432 if self.machine_model.has_ide() => self.ide.write_bank(1, value),
            // IDE presence detection register (no-op on write).
            0x0433 if self.machine_model.has_ide() => {}

            // IDE CS0 registers
            0x0642 if self.machine_model.has_ide() => self.ide.write_features(value),
            0x0644 if self.machine_model.has_ide() => self.ide.write_sector_count(value),
            0x0646 if self.machine_model.has_ide() => self.ide.write_sector_number(value),
            0x0648 if self.machine_model.has_ide() => self.ide.write_cylinder_low(value),
            0x064A if self.machine_model.has_ide() => self.ide.write_cylinder_high(value),
            0x064C if self.machine_model.has_ide() => self.ide.write_device_head(value),
            0x064E if self.machine_model.has_ide() => {
                let action = self.ide.write_command(value);
                self.process_ide_action(action);
            }

            // IDE CS1 registers
            0x074C if self.machine_model.has_ide() => self.ide.write_device_control(value),
            0x074E if self.machine_model.has_ide() => {} // Digital input - write is a no-op

            // IDE BIOS work area mapping (DA000-DBFFF).
            0x1E8E if self.machine_model.has_ide() => self.ide.write_work_area_port(value),

            // SIMM memory controller.
            // Ref: undoc98 `io_mem.txt` (ports 0x0530/0x0531)
            0x0530 if self.machine_model.is_pc9821() => {
                self.simm_address_register = value;
            }
            0x0531 if self.machine_model.is_pc9821() => {
                let index = self.simm_address_register as usize;
                let socket = index & 0x0F;
                let is_limit = index & 0x80 != 0;
                let data_index = socket * 2 + is_limit as usize;
                if data_index < self.simm_data.len() {
                    self.simm_data[data_index] = value;
                }
            }

            // Memory bank switching register.
            // Ref: undoc98 `io_mem.txt` (port 0x063C)
            0x063C if self.machine_model.is_pc9821() => {
                self.memory_bank_063c = value;
            }

            // Flash ROM power voltage control (no-op).
            // Ref: undoc98 `io_mem.txt` (port 0x063E)
            0x063E if self.machine_model.is_pc9821() => {}

            // CPU/cache control register.
            // Ref: undoc98 `io_mem.txt` (port 0x063F)
            0x063F if self.machine_model.is_pc9821() => {
                self.cache_control_063f = value;
            }

            // IDE bank select register (port 0x0436).
            0x0436 if self.machine_model.is_pc9821() => {}

            // Window Accelerator Board (WAB) - built-in graphics accelerator.
            // Ref: undoc98 `io_wab.txt`
            0x0FAA if self.machine_model.is_pc9821() => {
                self.wab_index = value;
            }
            0x0FAB if self.machine_model.is_pc9821() => {
                let index = self.wab_index as usize;
                if index < self.wab_data.len() {
                    self.wab_data[index] = value;
                }
            }
            0x0FAC if self.machine_model.is_pc9821() => {
                self.wab_relay = value;
            }

            // CPU mode / wait control register.
            // Ref: undoc98 `io_cpu.txt` (port 0x0534)
            0x0534 if self.machine_model.is_pc9821() => {
                self.cpu_mode_534 = value;
            }

            // Text control register (PC-H98).
            // Ref: undoc98 `io_disp.txt` (port 0x00A7)
            0x00A7 => {}

            // Memory status register (read-only, writes ignored).
            // Ref: undoc98 `io_mem.txt` (port 0x063D)
            0x063D if self.machine_model.is_pc9821() => {}

            // Hardware wait timing adjustment register.
            // Ref: undoc98 `io_tstmp.txt` (port 0x045F)
            0x045F if self.machine_model.is_pc9821() => {}

            // Graphics accelerator attribute controller (no accelerator present).
            // Ref: undoc98 `io_wab.txt` (port 0x0CA0)
            0x0CA0 if self.machine_model.is_pc9821() => {}

            // SCSI controller (WD33C93) - no SCSI present.
            // Ref: undoc98 `io_scsi.txt`
            0x0CC0 | 0x0CC2 => {}

            // Mouse interrupt vector setting.
            // Ref: undoc98 `io_mouse.txt` (port 0x98D7)
            0x98D7 if self.machine_model.is_pc9821() => {}

            // Unknown display register (PC-9821).
            0x98DB if self.machine_model.is_pc9821() => {}

            // Serial port FIFO control register.
            // Ref: undoc98 `io_rs.txt` (port 0x0138)
            0x0138 if self.machine_model.is_pc9821() => {}

            // Printer interface control register.
            0x0149 if self.machine_model.is_pc9821() => {}

            // Extended RS-232C control register.
            // Ref: undoc98 `io_rs.txt` (port 0x0434)
            0x0434 if self.machine_model.is_pc9821() => {}

            // CPU/system control register (port 0x00F4).
            0x00F4 if self.machine_model.is_pc9821() => {}

            // Memory/expansion control registers.
            0x0448 if self.machine_model.is_pc9821() => {}
            0x047B if self.machine_model.is_pc9821() => {}
            0x0549 if self.machine_model.is_pc9821() => {}
            0x0555 if self.machine_model.is_pc9821() => {}

            // Extended DMA control registers (stub).
            0x0E00 | 0x0E01 | 0x0E02 | 0x0E03 | 0x0E0F if self.machine_model.is_pc9821() => {}

            // 32-bit DMA controller (ORBIT) index/data.
            // Ref: undoc98 `io_dma.txt` (ports 0x002B/0x002D)
            0x002B | 0x002D if self.machine_model.is_pc9821() => {}

            // Pixel mask register (PC-H98 only, ignore on PC-9821).
            // Ref: undoc98 `io_disp.txt` (port 0x09AE)
            0x09AE if self.machine_model.is_pc9821() => {}

            // Unknown keyboard/display mapping register.
            0x0535 if self.machine_model.is_pc9821() => {}

            // MATE-X PCM sound ports (stub, no MATE-X hardware).
            0xAC6C..=0xAC6F if self.machine_model.is_pc9821() => {}

            // Mystery I/O ports (banking mechanism, undocumented).
            // NP21W labels these as "謎のI/Oポート" in pcidev.c.
            0x18F0 if self.machine_model.is_pc9821() => {}
            0x18F1 if self.machine_model.is_pc9821() => {}
            0x18F2 if self.machine_model.is_pc9821() => {}
            0x18F3 if self.machine_model.is_pc9821() => {}

            // Software DIP Switch (SDIP) - ports 0x841E–0x8F1E at 0x100 stride.
            // Ref: undoc98 `io_sdip.txt`
            port if self.machine_model.has_sdip()
                && (port & 0xFF) == 0x1E
                && (0x841E..=0x8F1E).contains(&port) =>
            {
                let offset = ((port >> 8) as usize & 0x0F) - 4;
                self.sdip.write(offset, value);
            }

            // SDIP bank select (later PC-9821 models).
            // Bit 6: 0 = front bank, 1 = back bank.
            // Ref: undoc98 `io_sdip.txt`
            0x8F1F if self.machine_model.has_sdip() => {
                self.sdip.select_bank_from_bit6(value);
            }

            // Epson PC-98 compatible system control ports (ignored, not an Epson machine).
            0x0C05 => {}

            // Undocumented ports (probed by Windows 95 during hardware detection).
            0x0D00..=0x0D03 => {}

            // Undocumented system register ports (probed by Windows 95).
            0x8D08 | 0x8D0E => {}

            // Video Board (PC-9801-72 / PC-98GS-02) control ports (ignored, no video board).
            // Ref: undoc98 `io_vbrd.txt`
            0xAF67 | 0xAF6A | 0xAF6B => {}

            // Sound Blaster 16 OPL3 address lo (base+0x2000, base+0x2800).
            0x20D2 | 0x28D2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb16) = self.sound_blaster_16 {
                    sb16.write_opl3_address_lo(value, self.current_cycle);
                    self.process_soundboard_sb16_actions();
                }
            }
            // Sound Blaster 16 OPL3 data lo (base+0x2100, base+0x2900).
            0x21D2 | 0x29D2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb16) = self.sound_blaster_16 {
                    sb16.write_opl3_data(value, self.current_cycle);
                    self.process_soundboard_sb16_actions();
                }
            }
            // Sound Blaster 16 OPL3 address hi (base+0x2200).
            0x22D2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb16) = self.sound_blaster_16 {
                    sb16.write_opl3_address_hi(value, self.current_cycle);
                    self.process_soundboard_sb16_actions();
                }
            }
            // Sound Blaster 16 OPL3 data hi (base+0x2300).
            0x23D2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb16) = self.sound_blaster_16 {
                    sb16.write_opl3_data(value, self.current_cycle);
                    self.process_soundboard_sb16_actions();
                }
            }
            // Sound Blaster 16 mixer address (base+0x2400).
            0x24D2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb16) = self.sound_blaster_16 {
                    sb16.write_mixer_address(value);
                }
            }
            // Sound Blaster 16 mixer data (base+0x2500).
            0x25D2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb16) = self.sound_blaster_16 {
                    sb16.write_mixer_data(value);
                    self.process_soundboard_sb16_actions();
                }
            }
            // Sound Blaster 16 DSP reset (base+0x2600).
            0x26D2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb16) = self.sound_blaster_16 {
                    sb16.write_dsp_reset(value);
                    self.process_soundboard_sb16_actions();
                }
            }
            // Sound Blaster 16 DSP read data port (base+0x2A00) - writes ignored.
            0x2AD2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
            }
            // Sound Blaster 16 DSP write command/data (base+0x2C00).
            0x2CD2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                if let Some(ref mut sb16) = self.sound_blaster_16 {
                    sb16.write_dsp_command(value);
                    self.process_soundboard_sb16_actions();
                }
            }

            // MPU-PC98II MIDI interface (C-Bus, default base 0xE0D0).
            0xE0D0 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                self.mpu401.write_data(value);
                self.sync_mpu_irq_and_timer();
            }
            0xE0D2 => {
                self.pending_wait_cycles += self.cbus_wait_cycles();
                self.mpu401.write_command(value);
                self.sync_mpu_irq_and_timer();
            }

            // C-Bus expansion card probing (no extension hardware present).
            0xC0E0..=0xFCE2 => {}

            _ => {
                self.tracer.trace_io_unhandled_write(port, value);
                warn!("Unhandled I/O write: port={port:#06X} value={value:#04X}");
            }
        }
    }

    /// Returns the hardware wait penalty for `OUT 0x005F,AL` (>= 0.6 us).
    fn artic_wait_cycles(&self) -> i64 {
        let cycles = (u64::from(self.clocks.cpu_clock_hz) * 6).div_ceil(10_000_000);
        cycles.max(1) as i64
    }

    /// Returns the C-bus expansion board I/O wait penalty in CPU cycles.
    ///
    /// The PC-98 C-bus runs at a fixed speed independent of the CPU clock.
    /// Each I/O access to a C-bus device takes at least 0.6 µs (same as the
    /// ARTIC wait), regardless of CPU frequency.
    pub(super) fn cbus_wait_cycles(&self) -> i64 {
        let cycles = (u64::from(self.clocks.cpu_clock_hz) * 6).div_ceil(10_000_000);
        cycles.max(1) as i64
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
        if channel == 1 {
            self.beeper
                .set_pit_reload(self.pit.channels[1].value, self.current_cycle);
        }
        if channel == 0 {
            self.pic.clear_irq(0);
            self.tracer.trace_irq_clear(0);
            self.pit.channels[0].flag |= PIT_FLAG_I;
            let cpu_cycles = self
                .pit
                .timer0_period_cycles(self.clocks.cpu_clock_hz, self.clocks.pit_clock_hz);
            self.scheduler
                .schedule(Event98::PitTimer0, self.current_cycle + cpu_cycles);
            self.update_next_event_cycle();
        }
    }

    fn write_pit_control(&mut self, value: u8) {
        let channel = ((value >> 6) & 3) as usize;
        if channel >= 3 {
            return;
        }
        let is_mode_set = value & 0x30 != 0;
        let control_cycle = if !is_mode_set && self.machine_model.is_pc9821() {
            let delay = self.pc9821_pit_latch_delay_cycles();
            self.pending_wait_cycles += delay as i64;
            self.current_cycle + delay
        } else {
            self.current_cycle
        };
        self.pit.write_control(
            channel,
            value,
            control_cycle,
            self.clocks.cpu_clock_hz,
            self.clocks.pit_clock_hz,
        );
        if channel == 0 && is_mode_set {
            self.pic.clear_irq(0);
            self.tracer.trace_irq_clear(0);
            self.pit.channels[0].flag |= PIT_FLAG_I;
        }
    }

    fn pc9821_pit_latch_delay_cycles(&self) -> u64 {
        // The PC-9821 defers a counter-latch command so the count is not sampled
        // mid-cycle. The delay is one period of half the 1.9968 MHz PIT input
        // clock (998.4 kHz, i.e. two PIT clocks), converted to CPU cycles.
        const PIT_LATCH_SETTLE_HZ: u32 = 998_400;
        u64::from(self.clocks.cpu_clock_hz)
            .div_ceil(u64::from(PIT_LATCH_SETTLE_HZ))
            .max(1)
    }

    /// Seek delay in CPU cycles (~500µs at 10 MHz = 5000 cycles).
    const SEEK_DELAY_CYCLES: u64 = 5000;
    /// Execution delay in CPU cycles (data ready after command).
    const EXECUTION_DELAY_CYCLES: u64 = 512;

    fn handle_fdc_action(&mut self, action: FdcAction, interface: u8) {
        match action {
            FdcAction::None => {}
            FdcAction::ScheduleSeekInterrupt => {
                {
                    let fdc = if interface == 0 {
                        self.floppy.fdc_1mb()
                    } else {
                        self.floppy.fdc_640k()
                    };
                    let drive = fdc.current_drive();
                    let cyl = fdc.state.drive_cylinder[drive];
                    let hd_us = fdc.state.hd_us;
                    self.tracer.trace_fdc_seek(drive, cyl, hd_us);
                }
                self.floppy.set_active_interface(interface);
                self.scheduler.schedule(
                    Event98::FdcInterrupt,
                    self.current_cycle + Self::SEEK_DELAY_CYCLES,
                );
                self.update_next_event_cycle();
            }
            FdcAction::StartReadData
            | FdcAction::StartReadId
            | FdcAction::StartWriteData
            | FdcAction::StartFormatTrack => {
                self.floppy.set_active_interface(interface);
                self.scheduler.schedule(
                    Event98::FdcExecution,
                    self.current_cycle + Self::EXECUTION_DELAY_CYCLES,
                );
                self.update_next_event_cycle();
            }
            FdcAction::StartScan => unreachable!("SCAN commands are disabled on this FDC"),
        }
    }

    fn read_warm_reset_context(&self) -> (u16, u16, u16, u16) {
        let sp = self.read_word_direct(0x0404);
        let ss = self.read_word_direct(0x0406);
        let stack_base = (ss as u32) * 16 + sp as u32;
        let ret_ip = self.read_word_direct(stack_base);
        let ret_cs = self.read_word_direct(stack_base + 2);
        (ss, sp.wrapping_add(4), ret_cs, ret_ip)
    }

    fn update_pegc_mapping(&mut self) {
        if self.pegc.is_256_color_active() {
            self.memory.set_e_plane_enabled(false);
        } else {
            self.update_plane_e_mapping();
        }
    }

    fn handle_mouse_timer_control_change(&mut self, was_enabled: bool) {
        let enabled = self.mouse_timer_irq_enabled();
        if enabled && !was_enabled {
            self.schedule_mouse_timer();
            self.update_next_event_cycle();
        } else if !enabled && was_enabled {
            self.scheduler.cancel(Event98::MouseTimer);
            self.pic.clear_irq(MOUSE_TIMER_IRQ_LINE);
            self.tracer.trace_irq_clear(MOUSE_TIMER_IRQ_LINE);
            self.update_next_event_cycle();
        }
    }

    fn handle_gdc_slave_action(&mut self, action: GdcAction) {
        match action {
            GdcAction::None => {}
            GdcAction::Draw(result) => {
                for i in 0..self.gdc_slave.draw_buffer.len() {
                    let op = self.gdc_slave.draw_buffer[i];
                    self.apply_gdc_vram_op(&op);
                }
                self.gdc_slave.draw_buffer.clear();
                if result.dot_count > 0 {
                    self.schedule_drawing_timing(result.dot_count);
                } else {
                    self.gdc_slave.state.status &= !STATUS_DRAWING;
                }
            }
            GdcAction::DackWrite(op) => {
                self.apply_gdc_vram_op(&op);
            }
            GdcAction::ReadVram(_request) => {
                // Feed VRAM words to the GDC for RDAT.
                while let Some(address) = self.gdc_slave.rdat_next_address() {
                    let word = self.read_gdc_vram_word_from_access_page(address);
                    let needs_more = self.gdc_slave.provide_rdat_word(word);
                    if !needs_more {
                        break;
                    }
                    // Stop if FIFO is getting full (leave room for CPU reads).
                    if self.gdc_slave.state.fifo.count >= 14 {
                        break;
                    }
                }
            }
            GdcAction::TimingChanged => {}
        }
    }

    fn handle_gdc_master_action(&mut self, action: GdcAction) {
        match action {
            GdcAction::None => {}
            GdcAction::Draw(result) => {
                for i in 0..self.gdc_master.draw_buffer.len() {
                    let op = self.gdc_master.draw_buffer[i];
                    self.apply_gdc_text_vram_op(&op);
                }
                self.gdc_master.draw_buffer.clear();
                if result.dot_count == 0 {
                    self.gdc_master.state.status &= !STATUS_DRAWING;
                }
            }
            GdcAction::DackWrite(op) => {
                self.apply_gdc_text_vram_op(&op);
            }
            GdcAction::ReadVram(_request) => {
                self.feed_gdc_master_rdat();
            }
            GdcAction::TimingChanged => self.reschedule_gdc_events(),
        }
    }

    pub(super) fn feed_gdc_master_rdat(&mut self) {
        while let Some(address) = self.gdc_master.rdat_next_address() {
            let word = self.read_gdc_text_vram_word(address);
            let needs_more = self.gdc_master.provide_rdat_word(word);
            if !needs_more {
                break;
            }
            if self.gdc_master.state.fifo.count >= 14 {
                break;
            }
        }
    }

    fn schedule_drawing_timing(&mut self, dot_count: u32) {
        let dots = dot_count as u64;
        let is_8mhz_lineage = self.clocks.pit_clock_hz == 1_996_800;
        let factor = if is_8mhz_lineage { 22464u64 } else { 27648u64 };
        let clk = dots * factor / 15625 + 30;
        self.gdc_slave.state.status |= STATUS_DRAWING;
        self.scheduler
            .schedule(Event98::GdcDrawingComplete, self.current_cycle + clk);
        self.update_next_event_cycle();
    }

    fn process_sasi_action(&mut self, action: SasiAction) {
        match action {
            SasiAction::None => {}
            SasiAction::ScheduleCompletion | SasiAction::FormatTrack => {
                self.scheduler.schedule(
                    Event98::SasiExecution,
                    self.current_cycle + INTERRUPT_DELAY_CYCLES,
                );
                self.update_next_event_cycle();
            }
            SasiAction::DmaReady => {
                self.handle_sasi_dma();
            }
        }
    }

    fn handle_sasi_dma(&mut self) {
        let mask_20bit = self.dma_access_ctrl & 0x04 != 0;

        match self.sasi.phase() {
            SasiPhase::Read => loop {
                let (byte, action) = self.sasi.dma_read_byte();
                let dma_result = self.dma.transfer_write_to_memory(0, &[byte]);
                for (addr, b) in &dma_result.writes {
                    let addr = if mask_20bit { *addr & 0xF_FFFF } else { *addr };
                    self.memory.write_byte(addr, *b);
                }
                if matches!(action, SasiAction::ScheduleCompletion) || dma_result.terminal_count {
                    self.scheduler.schedule(
                        Event98::SasiExecution,
                        self.current_cycle + INTERRUPT_DELAY_CYCLES,
                    );
                    self.update_next_event_cycle();
                    break;
                }
                if !self.sasi.dma_ready() {
                    break;
                }
            },
            SasiPhase::Write => loop {
                let dma_result = self.dma.transfer_read_from_memory(0, 1);
                if dma_result.addresses.is_empty() {
                    break;
                }
                let addr = dma_result.addresses[0];
                let addr = if mask_20bit { addr & 0xF_FFFF } else { addr };
                let byte = self.memory.read_byte(addr);
                let action = self.sasi.dma_write_byte(byte);
                if matches!(action, SasiAction::ScheduleCompletion) {
                    self.scheduler.schedule(
                        Event98::SasiExecution,
                        self.current_cycle + INTERRUPT_DELAY_CYCLES,
                    );
                    self.update_next_event_cycle();
                    break;
                }
                if dma_result.terminal_count || !self.sasi.dma_ready() {
                    break;
                }
            },
            _ => {}
        }
    }

    fn current_dot_clock_hz(&self) -> u32 {
        let base = if self.display_control.state.display_line_count & 1 != 0 {
            DOT_CLOCK_400LINE
        } else {
            DOT_CLOCK_200LINE
        };
        if self.display_control.is_gdc_5mhz() {
            base.saturating_mul(2)
        } else {
            base
        }
    }

    pub(super) fn apply_gdc_dot_clock(&mut self) {
        let dot_clock = self.current_dot_clock_hz();
        self.gdc_master.set_dot_clock(dot_clock);
        self.gdc_slave.set_dot_clock(dot_clock);
    }

    pub(super) fn reschedule_gdc_events(&mut self) {
        self.scheduler.schedule(
            Event98::GdcVsync,
            self.current_cycle + self.gdc_master.state.display_period,
        );
        self.update_next_event_cycle();
    }

    fn write_gdc_vram_word_to_access_page(&mut self, address: u32, value: u16) {
        let (plane, byte_offset) = Self::gdc_address_to_plane_and_byte_offset(address);
        if plane == 3 && !self.graphics_extension_enabled {
            return;
        }
        let page = self.access_page_index();
        self.graphics_plane_write_byte_to_page(page, plane, byte_offset, value as u8);
        self.graphics_plane_write_byte_to_page(page, plane, byte_offset + 1, (value >> 8) as u8);
    }

    /// Converts between GDC bit ordering and CPU/VRAM bit ordering.
    /// The GDC places the leftmost pixel at bit 0 (LSB) within each byte,
    /// while the CPU/VRAM layout places the leftmost pixel at bit 7 (MSB).
    fn reverse_bits_in_bytes(word: u16) -> u16 {
        let lo = (word as u8).reverse_bits();
        let hi = ((word >> 8) as u8).reverse_bits();
        u16::from(lo) | (u16::from(hi) << 8)
    }

    pub(super) fn apply_gdc_vram_op(&mut self, op: &VramOp) {
        if self.grcg.gdc_with_grcg_enabled() {
            if self.display_control.is_egc_extended_mode_effective() {
                self.apply_gdc_vram_op_egc(op);
                return;
            }
            self.apply_gdc_vram_op_grcg(op);
            return;
        }

        let current = self.read_gdc_vram_word_from_access_page(op.address);

        let mask = Self::reverse_bits_in_bytes(op.mask);
        let data = Self::reverse_bits_in_bytes(op.data);
        let result = match op.mode {
            0 => (current & !mask) | (data & mask),
            1 => current ^ (data & mask),
            2 => current & !(data & mask),
            3 => current | (data & mask),
            _ => current,
        };

        self.write_gdc_vram_word_to_access_page(op.address, result);
    }

    fn read_gdc_text_vram_word(&self, address: u32) -> u16 {
        let byte_offset = ((address as usize) & 0x1FFF) * 2;
        let low = self.memory.state.text_vram[byte_offset];
        let high = self.memory.state.text_vram[byte_offset + 1];
        u16::from(low) | (u16::from(high) << 8)
    }

    fn write_gdc_text_vram_word(&mut self, address: u32, value: u16) {
        let byte_offset = ((address as usize) & 0x1FFF) * 2;
        self.memory.state.text_vram[byte_offset] = value as u8;
        self.memory.state.text_vram[byte_offset + 1] = (value >> 8) as u8;
    }

    fn apply_gdc_text_vram_op(&mut self, op: &VramOp) {
        let current = self.read_gdc_text_vram_word(op.address);
        let mask = op.mask;
        let data = op.data;
        let result = match op.mode {
            0 => (current & !mask) | (data & mask),
            1 => current ^ (data & mask),
            2 => current & !(data & mask),
            3 => current | (data & mask),
            _ => current,
        };
        self.write_gdc_text_vram_word(op.address, result);
    }

    fn apply_gdc_vram_op_grcg(&mut self, op: &VramOp) {
        let byte_offset = (op.address as usize & 0x3FFF) * 2;
        let raw_mask = op.data & op.mask;
        if raw_mask == 0 {
            return;
        }
        let active_mask = Self::reverse_bits_in_bytes(raw_mask);

        for p in 0..4 {
            if p == 3 && !self.graphics_extension_enabled {
                continue;
            }
            let tile_word =
                u16::from(self.grcg.state.tile[p]) | (u16::from(self.grcg.state.tile[p]) << 8);

            if !self.grcg.is_rmw() {
                // TDW: write tile word directly, ignoring GDC mask/data/mode.
                self.graphics_plane_write_byte(p, byte_offset, tile_word as u8);
                self.graphics_plane_write_byte(p, byte_offset + 1, (tile_word >> 8) as u8);
            } else {
                // RMW: use active draw bits as selector, tile as color source.
                let current_low = self.graphics_plane_read_byte(p, byte_offset);
                let current_high = self.graphics_plane_read_byte(p, byte_offset + 1);
                let current = u16::from(current_low) | (u16::from(current_high) << 8);
                let result = (current & !active_mask) | (tile_word & active_mask);
                self.graphics_plane_write_byte(p, byte_offset, result as u8);
                self.graphics_plane_write_byte(p, byte_offset + 1, (result >> 8) as u8);
            }
        }
    }

    /// Routes a GDC VRAM operation through the EGC.
    /// Each per-dot VramOp is converted to a word-aligned EGC write with the pixel
    /// bit set in the corresponding position within the word.
    ///
    /// Uses the inner variant to avoid charging CPU wait cycles
    /// (GDC drawing has its own timing model.
    fn apply_gdc_vram_op_egc(&mut self, op: &VramOp) {
        let raw_mask = op.data & op.mask;
        if raw_mask == 0 {
            return;
        }
        let active_mask = Self::reverse_bits_in_bytes(raw_mask);
        // GDC word address -> byte offset. Each GDC word is 2 bytes.
        let byte_offset = (op.address as usize & 0x3FFF) * 2;
        // Convert the 16-bit active mask to an EGC word write at the word-aligned address.
        // The address is already word-aligned (GDC addresses are word addresses).
        let addr = 0xA8000 + byte_offset as u32;
        self.egc_write_word_inner(addr, active_mask);
    }
}

#[cfg(test)]
mod tests {
    use common::{Bus, CpuMode, MachineModel, Tracing};
    use device::upd7220_gdc::{DOT_CLOCK_200LINE, DOT_CLOCK_400LINE, VramOp};

    use crate::bus::{NoTracing, Pc9801Bus};

    #[derive(Default)]
    struct UnhandledWriteTrace {
        writes: Vec<(u16, u8)>,
    }

    impl Tracing for UnhandledWriteTrace {
        fn trace_io_unhandled_write(&mut self, port: u16, value: u8) {
            self.writes.push((port, value));
        }
    }

    fn enable_egc_mode(bus: &mut Pc9801Bus<NoTracing>) {
        bus.io_write_byte(0x6A, 0x07);
        bus.io_write_byte(0x6A, 0x05);
    }

    #[test]
    fn pc9801f_fdd320_ppi_shim_writes_are_handled() {
        let mut bus =
            Pc9801Bus::<UnhandledWriteTrace>::new(MachineModel::PC9801F, CpuMode::High, 48000);

        bus.io_write_byte(0x0057, 0x91);
        bus.io_write_byte(0x0053, 0x00);
        bus.io_write_byte(0x0057, 0x0F);

        assert_eq!(bus.tracer().writes, Vec::<(u16, u8)>::new());
    }

    #[test]
    fn fdd320_ppi_writes_are_not_exposed_on_later_models() {
        let mut bus =
            Pc9801Bus::<UnhandledWriteTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

        bus.io_write_byte(0x0053, 0x22);
        bus.io_write_byte(0x0057, 0x91);

        assert_eq!(bus.tracer().writes, vec![(0x0053, 0x22), (0x0057, 0x91)]);
    }

    #[test]
    fn mode2_and_line_count_update_both_gdc_dot_clocks() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);

        // Boot state has display_line_count=0x01 (400-line dot clock).
        assert_eq!(bus.gdc_master.state.dot_clock_hz, DOT_CLOCK_400LINE);
        assert_eq!(bus.gdc_slave.state.dot_clock_hz, DOT_CLOCK_400LINE);

        // Switch to 200-line base clock.
        bus.io_write_byte(0x6E, 0x00);
        assert_eq!(bus.gdc_master.state.dot_clock_hz, DOT_CLOCK_200LINE);
        assert_eq!(bus.gdc_slave.state.dot_clock_hz, DOT_CLOCK_200LINE);

        // Back to 400-line base clock.
        bus.io_write_byte(0x6E, 0x01);
        assert_eq!(bus.gdc_master.state.dot_clock_hz, DOT_CLOCK_400LINE);
        assert_eq!(bus.gdc_slave.state.dot_clock_hz, DOT_CLOCK_400LINE);

        // Only one 5MHz bit set: still base clock.
        bus.io_write_byte(0x6A, 0x83);
        assert_eq!(bus.gdc_master.state.dot_clock_hz, DOT_CLOCK_400LINE);
        assert_eq!(bus.gdc_slave.state.dot_clock_hz, DOT_CLOCK_400LINE);

        // Both 5MHz bits set: doubled clock.
        bus.io_write_byte(0x6A, 0x85);
        assert_eq!(bus.gdc_master.state.dot_clock_hz, DOT_CLOCK_400LINE * 2);
        assert_eq!(bus.gdc_slave.state.dot_clock_hz, DOT_CLOCK_400LINE * 2);
    }

    #[test]
    fn gdc_direct_draw_uses_address_plane_bits() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);

        for (address, offset) in [(0x4000, 0), (0x8000, 0x8000), (0xC000, 0x10000)] {
            bus.apply_gdc_vram_op(&VramOp {
                address,
                data: 0x0080,
                mask: 0x0080,
                mode: 0,
            });

            assert_eq!(bus.memory.state.graphics_vram[offset], 0x01);
        }
        assert_eq!(bus.memory.state.e_plane_vram[0], 0x00);
    }

    #[test]
    fn gdc_direct_read_uses_address_plane_bits() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);

        bus.memory.state.graphics_vram[0] = 0x12;
        bus.memory.state.graphics_vram[1] = 0x34;
        bus.memory.state.graphics_vram[0x8000] = 0x56;
        bus.memory.state.graphics_vram[0x8001] = 0x78;
        bus.memory.state.graphics_vram[0x10000] = 0x9A;
        bus.memory.state.graphics_vram[0x10001] = 0xBC;

        assert_eq!(bus.read_gdc_vram_word_from_access_page(0x4000), 0x3412);
        assert_eq!(bus.read_gdc_vram_word_from_access_page(0x8000), 0x7856);
        assert_eq!(bus.read_gdc_vram_word_from_access_page(0xC000), 0xBC9A);
    }

    #[test]
    fn gdc_direct_e_plane_requires_graphics_extension() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
        bus.set_graphics_extension_enabled(false);
        let operation = VramOp {
            address: 0x0000,
            data: 0x0080,
            mask: 0x0080,
            mode: 0,
        };

        bus.apply_gdc_vram_op(&operation);
        assert_eq!(bus.memory.state.e_plane_vram[0], 0x00);

        bus.set_graphics_extension_enabled(true);
        bus.apply_gdc_vram_op(&operation);
        assert_eq!(bus.memory.state.e_plane_vram[0], 0x01);
    }

    #[test]
    fn gdc_grcg_tdw_ignores_pattern_off_writes() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);

        bus.grcg.write_mode(0x80);
        bus.grcg.state.tile = [0x5A, 0xA5, 0x3C, 0xC3];

        bus.memory.state.graphics_vram[0] = 0x11;
        bus.memory.state.graphics_vram[1] = 0x22;

        bus.apply_gdc_vram_op(&VramOp {
            address: 0,
            data: 0x0000,
            mask: 0x0080,
            mode: 0,
        });

        assert_eq!(bus.memory.state.graphics_vram[0], 0x11);
        assert_eq!(bus.memory.state.graphics_vram[1], 0x22);

        bus.apply_gdc_vram_op(&VramOp {
            address: 0,
            data: 0x0080,
            mask: 0x0080,
            mode: 0,
        });

        assert_eq!(bus.memory.state.graphics_vram[0], 0x5A);
        assert_eq!(bus.memory.state.graphics_vram[1], 0x5A);
    }

    #[test]
    fn gdc_grcg_rmw_uses_active_mask_bits_only() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);

        bus.grcg.write_mode(0xC0);
        bus.grcg.state.tile = [0xFF, 0x00, 0x00, 0x00];

        bus.memory.state.graphics_vram[0] = 0xAA;
        bus.memory.state.graphics_vram[1] = 0xAA;

        bus.apply_gdc_vram_op(&VramOp {
            address: 0,
            data: 0x0000,
            mask: 0x00FF,
            mode: 0,
        });

        assert_eq!(bus.memory.state.graphics_vram[0], 0xAA);
        assert_eq!(bus.memory.state.graphics_vram[1], 0xAA);

        bus.apply_gdc_vram_op(&VramOp {
            address: 0,
            data: 0x00F0,
            mask: 0x00FF,
            mode: 0,
        });

        // raw_mask = 0x00F0, after bit reversal within bytes = 0x000F
        // Plane 0: (0xAA & !0x0F) | (0xFF & 0x0F) = 0xA0 | 0x0F = 0xAF
        assert_eq!(bus.memory.state.graphics_vram[0], 0xAF);
        assert_eq!(bus.memory.state.graphics_vram[1], 0xAA);
    }

    #[test]
    fn gdc_grcg_tdw_writes_all_planes_regardless_of_plane_enable() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
        // TDW mode with only planes 0,2 enabled (bits 1,3 set = planes 1,3 disabled).
        bus.grcg.write_mode(0x8A);
        bus.grcg.state.tile = [0x11, 0x22, 0x33, 0x44];

        bus.apply_gdc_vram_op(&VramOp {
            address: 0,
            data: 0x0080,
            mask: 0x0080,
            mode: 0,
        });

        // All planes should be written despite plane-enable bits.
        assert_eq!(bus.memory.state.graphics_vram[0], 0x11); // B (plane 0)
        assert_eq!(bus.memory.state.graphics_vram[0x8000], 0x22); // R (plane 1, "disabled")
        assert_eq!(bus.memory.state.graphics_vram[0x10000], 0x33); // G (plane 2)
    }

    #[test]
    fn gdc_egc_write_does_not_charge_cpu_wait() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
        enable_egc_mode(&mut bus);
        bus.grcg.write_mode(0x80);

        bus.pending_wait_cycles = 0;
        bus.apply_gdc_vram_op(&VramOp {
            address: 0,
            data: 0x0080,
            mask: 0x0080,
            mode: 0,
        });
        assert_eq!(
            bus.pending_wait_cycles, 0,
            "GDC-EGC should not charge CPU wait"
        );
    }

    #[test]
    fn egc_io_write_word_sets_register_atomically() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
        enable_egc_mode(&mut bus);
        bus.grcg.write_mode(0x80); // activate GRCG

        bus.io_write_word(0x04AC, 0x1033);
        assert_eq!(bus.egc.state.sft, 0x1033);

        bus.io_write_word(0x04A8, 0xAAAA);
        assert_eq!(bus.egc.state.mask, 0xAAAA);
    }

    #[test]
    fn egc_io_write_word_noop_when_egc_inactive() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);
        let old_sft = bus.egc.state.sft;
        bus.io_write_word(0x04AC, 0x1033);
        assert_eq!(bus.egc.state.sft, old_sft);
    }

    #[test]
    fn port_053d_shadow_ram_control() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.memory.set_shadow_control(0x00);
        assert_eq!(bus.memory.state.shadow_control, 0x00);
        bus.io_write_byte(0x053D, 0x82);
        assert_eq!(bus.memory.state.shadow_control, 0x82);
    }

    #[test]
    fn port_053d_shadow_ram_boot_sequence() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.memory.set_shadow_control(0x00);
        let mut rom = vec![0xFFu8; 0x18000];
        rom[0] = 0xAA;
        rom[0x17FFF] = 0xBB;
        bus.memory.load_rom(&rom);

        // ROM mode (shadow_control=0x00): reads come from ROM.
        assert_eq!(bus.memory.read_byte(0xE8000), 0xAA);
        assert_eq!(bus.memory.read_byte(0xFFFFF), 0xBB);

        // Read ROM contents and write back while still in ROM-read mode.
        // Writes go to shadow RAM independently of the read selector.
        let val_e8 = bus.memory.read_byte(0xE8000);
        let val_ff = bus.memory.read_byte(0xFFFFF);
        bus.memory.write_byte(0xE8000, val_e8);
        bus.memory.write_byte(0xFFFFF, val_ff);

        // Shadow RAM now has the copied values, but reads still return ROM.
        assert_eq!(bus.memory.read_byte(0xE8000), 0xAA);

        // Switch to shadow RAM read mode (bit 1 set).
        bus.io_write_byte(0x053D, 0x02);

        // Reads now come from shadow RAM and should return the copied values.
        assert_eq!(bus.memory.read_byte(0xE8000), 0xAA);
        assert_eq!(bus.memory.read_byte(0xFFFFF), 0xBB);
    }

    #[test]
    fn port_043b_readback() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        assert_eq!(bus.hole_15m_control, 0x00);
        bus.io_write_byte(0x043B, 0x55);
        assert_eq!(bus.hole_15m_control, 0x55);
        assert_eq!(bus.io_read_byte(0x043B), 0x55);
    }

    #[test]
    fn ram_window_blocked_when_bios_access_disabled() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        // Shadow RAM read mode (bit 1) so we can read back writes to the BIOS range.
        bus.memory.set_shadow_control(0x02);
        // Map RAM window to E0000-FFFFF range (window value 0x0E -> physical base 0xE0000).
        // Use offset 0x88000 to reach physical 0xE8000 (BIOS ROM / shadow RAM area).
        bus.ram_window = 0x0E;
        bus.write_byte_with_access_page(0x88000, 0xAB);
        assert_eq!(bus.read_byte_with_access_page(0x88000), 0xAB);

        // Set bit 2 of shadow_control: disable BIOS RAM access via window.
        bus.io_write_byte(0x053D, 0x06);

        // Reads to the BIOS range via window should now return 0xFF.
        assert_eq!(bus.read_byte_with_access_page(0x88000), 0xFF);

        // Writes should be dropped.
        bus.write_byte_with_access_page(0x88000, 0xCD);
        // Clear bit 2, re-read to verify write was dropped.
        bus.io_write_byte(0x053D, 0x02);
        assert_eq!(bus.read_byte_with_access_page(0x88000), 0xAB);
    }

    #[test]
    fn port_00f6_a20_control() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.a20_enabled = false;
        // 0x02 = release A20 mask.
        bus.io_write_byte(0xF6, 0x02);
        assert!(bus.a20_enabled);
        // 0x03 = set A20 mask.
        bus.io_write_byte(0xF6, 0x03);
        assert!(!bus.a20_enabled);
        // Read back: bit 0 = mask state (1=masked).
        assert_eq!(bus.io_read_byte(0xF6) & 1, 1);
        bus.io_write_byte(0xF6, 0x02);
        assert_eq!(bus.io_read_byte(0xF6) & 1, 0);
    }

    #[test]
    fn port_00f2_a20_status_readback() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        // After reset A20 is masked: bit 0 = 1.
        assert_eq!(bus.io_read_byte(0xF2) & 1, 1, "A20 masked: bit 0 must be 1");
        // Write anything to 0xF2 to unmask A20.
        bus.io_write_byte(0xF2, 0x00);
        assert!(bus.a20_enabled);
        // Bit 0 must now be 0 (unmasked).
        assert_eq!(
            bus.io_read_byte(0xF2) & 1,
            0,
            "A20 unmasked: bit 0 must be 0"
        );
        // 0xF0 write re-masks A20.
        bus.io_write_byte(0xF0, 0x00);
        assert!(!bus.a20_enabled);
        assert_eq!(
            bus.io_read_byte(0xF2) & 1,
            1,
            "A20 masked again: bit 0 must be 1"
        );
    }

    #[test]
    fn port_0567_readback() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        assert_eq!(bus.io_read_byte(0x0567), 0xE0);
        bus.io_write_byte(0x0567, 0x42);
        assert_eq!(bus.io_read_byte(0x0567), 0x42);
    }

    #[test]
    fn port_a460_reports_86_id_and_mask_controls_opna() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.install_soundboard_86(None, true);

        assert_eq!(bus.io_read_byte(0xA460), 0x40);

        bus.io_write_byte(0xA460, 0x03);
        assert_eq!(bus.io_read_byte(0xA460), 0x43);

        bus.io_write_byte(0xA460, 0x02);
        bus.io_write_byte(0x0188, 0xFF);
        assert_eq!(bus.io_read_byte(0x018A), 0xFF);

        bus.io_write_byte(0xA460, 0x00);
        bus.io_write_byte(0x0188, 0xFF);
        assert_eq!(bus.io_read_byte(0x018A), 0x01);
    }

    #[test]
    fn soundboard_86_adpcm_ram_disabled() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.install_soundboard_86(None, false);

        // Enable extended mode.
        bus.io_write_byte(0xA460, 0x01);

        // Set up external + record mode for ADPCM-B to write data.
        bus.io_write_byte(0x018C, 0x00);
        bus.io_write_byte(0x018E, 0x01); // Reset
        bus.io_write_byte(0x018C, 0x00);
        bus.io_write_byte(0x018E, 0x00); // Clear reset
        bus.io_write_byte(0x018C, 0x01);
        bus.io_write_byte(0x018E, 0x02); // dram_8bit=1
        bus.io_write_byte(0x018C, 0x02);
        bus.io_write_byte(0x018E, 0x00); // Start=0
        bus.io_write_byte(0x018C, 0x03);
        bus.io_write_byte(0x018E, 0x00);
        bus.io_write_byte(0x018C, 0x04);
        bus.io_write_byte(0x018E, 0xFF); // End=wide
        bus.io_write_byte(0x018C, 0x05);
        bus.io_write_byte(0x018E, 0xFF);
        bus.io_write_byte(0x018C, 0x0C);
        bus.io_write_byte(0x018E, 0xFF); // Limit=max
        bus.io_write_byte(0x018C, 0x0D);
        bus.io_write_byte(0x018E, 0xFF);
        bus.io_write_byte(0x018C, 0x00);
        bus.io_write_byte(0x018E, 0x60); // External + record

        // Write several bytes - these go nowhere since no RAM.
        for byte in [0xAB, 0xCD, 0xEF] {
            bus.io_write_byte(0x018C, 0x08);
            bus.io_write_byte(0x018E, byte);
            bus.io_write_byte(0x018C, 0x10);
            bus.io_write_byte(0x018E, 0x80);
        }

        // Switch to external read mode.
        bus.io_write_byte(0x018C, 0x00);
        bus.io_write_byte(0x018E, 0x01); // Reset
        bus.io_write_byte(0x018C, 0x00);
        bus.io_write_byte(0x018E, 0x00); // Clear reset
        bus.io_write_byte(0x018C, 0x10);
        bus.io_write_byte(0x018E, 0x80);
        bus.io_write_byte(0x018C, 0x00);
        bus.io_write_byte(0x018E, 0x20); // External read
        bus.io_write_byte(0x018C, 0x01);
        bus.io_write_byte(0x018E, 0x02); // dram_8bit=1
        bus.io_write_byte(0x018C, 0x02);
        bus.io_write_byte(0x018E, 0x00);
        bus.io_write_byte(0x018C, 0x03);
        bus.io_write_byte(0x018E, 0x00);
        bus.io_write_byte(0x018C, 0x04);
        bus.io_write_byte(0x018E, 0xFF);
        bus.io_write_byte(0x018C, 0x05);
        bus.io_write_byte(0x018E, 0xFF);

        // The first two reads return stale data from the CPU data register
        // (chip-internal buffer pre-fill behavior). Discard them.
        for _ in 0..2 {
            bus.io_write_byte(0x018C, 0x08);
            let _ = bus.io_read_byte(0x018E);
            bus.io_write_byte(0x018C, 0x10);
            bus.io_write_byte(0x018E, 0x80);
        }

        // Third read fetches from external memory - should be 0x00 with no RAM.
        bus.io_write_byte(0x018C, 0x08);
        let data = bus.io_read_byte(0x018E);
        assert_eq!(
            data, 0x00,
            "ADPCM-B read with no RAM should return 0x00, got 0x{data:02X}"
        );
    }

    #[test]
    fn port_f0_cold_reset_preserves_shut_bits() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
        // Set SHUT0=1 (bit 7) and SHUT1=1 (bit 5) via PPI control port.
        bus.io_write_byte(0x37, 0x0F); // set SHUT0
        bus.io_write_byte(0x37, 0x0B); // set SHUT1
        assert_ne!(bus.system_ppi.state.port_c & 0x80, 0, "SHUT0 should be set");
        assert_ne!(bus.system_ppi.state.port_c & 0x20, 0, "SHUT1 should be set");

        bus.io_write_byte(0xF0, 0x00);

        // Cold reset must leave both SHUT bits intact so the ITF
        // can read SHUT0=1, SHUT1=1 and perform a normal reset.
        assert!(bus.reset_pending);
        assert!(!bus.shutdown_requested);
        assert!(bus.warm_reset_context.is_none());
        assert_ne!(
            bus.system_ppi.state.port_c & 0x80,
            0,
            "SHUT0 must survive cold reset"
        );
        assert_ne!(
            bus.system_ppi.state.port_c & 0x20,
            0,
            "SHUT1 must survive cold reset"
        );
    }

    #[test]
    fn port_f0_warm_reset_captures_context() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
        // Clear SHUT0 (bit 7) - leaves warm-reset mode.
        bus.io_write_byte(0x37, 0x0E); // clear SHUT0
        assert_eq!(
            bus.system_ppi.state.port_c & 0x80,
            0,
            "SHUT0 should be clear"
        );

        // Plant a warm-reset context at 0000:0404 (SP) and 0000:0406 (SS).
        // The ITF reads SS:SP from there, then pops CS:IP via RETF.
        // SS=0x0000, SP=0x0600: stack at 0000:0600 contains IP=0x1234, CS=0x5678.
        bus.memory.state.ram[0x0404] = 0x00; // SP low
        bus.memory.state.ram[0x0405] = 0x06; // SP high (0x0600)
        bus.memory.state.ram[0x0406] = 0x00; // SS low
        bus.memory.state.ram[0x0407] = 0x00; // SS high (0x0000)
        bus.memory.state.ram[0x0600] = 0x34; // IP low
        bus.memory.state.ram[0x0601] = 0x12; // IP high
        bus.memory.state.ram[0x0602] = 0x78; // CS low
        bus.memory.state.ram[0x0603] = 0x56; // CS high

        bus.io_write_byte(0xF0, 0x00);

        assert!(bus.reset_pending);
        assert!(!bus.shutdown_requested);
        let (ss, sp, cs, ip) = bus.warm_reset_context.unwrap();
        assert_eq!(ss, 0x0000);
        assert_eq!(sp, 0x0604); // SP advanced past the popped RETF frame
        assert_eq!(cs, 0x5678);
        assert_eq!(ip, 0x1234);
    }

    #[test]
    fn port_f0_shutdown_sets_flag() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
        // Set SHUT0=1 (bit 7), clear SHUT1 (bit 5) -> shutdown.
        bus.io_write_byte(0x37, 0x0F); // set SHUT0
        bus.io_write_byte(0x37, 0x0A); // clear SHUT1
        assert_ne!(bus.system_ppi.state.port_c & 0x80, 0, "SHUT0 should be set");
        assert_eq!(
            bus.system_ppi.state.port_c & 0x20,
            0,
            "SHUT1 should be clear"
        );

        bus.io_write_byte(0xF0, 0x00);

        assert!(bus.reset_pending);
        assert!(bus.shutdown_requested);
        assert!(bus.warm_reset_context.is_none());
    }

    #[test]
    fn pcm86_mute_port_a66e_read_write() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.install_soundboard_86(None, true);

        // Board starts unmuted.
        assert_eq!(bus.io_read_byte(0xA66E), 0x00);

        // Mute.
        bus.io_write_byte(0xA66E, 0x01);
        assert_eq!(bus.io_read_byte(0xA66E), 0x01);

        // Unmute.
        bus.io_write_byte(0xA66E, 0x00);
        assert_eq!(bus.io_read_byte(0xA66E), 0x00);

        // Full byte is stored; only bit 0 controls mute.
        bus.io_write_byte(0xA66E, 0xFE);
        assert_eq!(bus.io_read_byte(0xA66E), 0xFE);
    }

    #[test]
    fn pcm86_fifo_status_full_and_empty() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.install_soundboard_86(None, true);

        // Initially FIFO is empty - bit 6 should be set.
        let status = bus.io_read_byte(0xA466);
        assert_ne!(status & 0x40, 0, "FIFO empty flag should be set initially");
        assert_eq!(status & 0x80, 0, "FIFO full flag should be clear initially");

        // Fill FIFO to capacity (32 KB logical max).
        for _ in 0..0x8000 {
            bus.io_write_byte(0xA46C, 0x55);
        }
        let status = bus.io_read_byte(0xA466);
        assert_ne!(
            status & 0x80,
            0,
            "FIFO full flag should be set after filling"
        );
        assert_eq!(
            status & 0x40,
            0,
            "FIFO empty flag should be clear when full"
        );
    }

    #[test]
    fn pcm86_fifo_reset_clears_buffer() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.install_soundboard_86(None, true);

        // Write some data.
        for _ in 0..100 {
            bus.io_write_byte(0xA46C, 0xAA);
        }
        // Verify not empty.
        let status = bus.io_read_byte(0xA466);
        assert_eq!(status & 0x40, 0, "FIFO should not be empty after writes");

        // Set bit 3 to reset FIFO.
        bus.io_write_byte(0xA468, 0x08);
        // Clear bit 3.
        bus.io_write_byte(0xA468, 0x00);

        // Should be empty again.
        let status = bus.io_read_byte(0xA466);
        assert_ne!(status & 0x40, 0, "FIFO should be empty after reset");
    }

    #[test]
    fn pcm86_volume_register_stores_all_lines() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.install_soundboard_86(None, true);

        // Write volume to line 5 (PCM direct, bits 7-5 = 101).
        bus.io_write_byte(0xA466, 0xA5); // 101_00101 -> line 5, value 5

        // Write volume to line 0 (FM direct, bits 7-5 = 000).
        bus.io_write_byte(0xA466, 0x0A); // 000_01010 -> line 0, value 10

        // Snapshot the live state and verify both were stored.
        let state = bus.soundboard_86.as_ref().unwrap().save_state();
        assert_eq!(state.pcm86.vol[5], 5);
        assert_eq!(state.pcm86.vol[0], 10);
    }

    #[test]
    fn pcm86_irq_flag_set_when_buffer_below_threshold() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.install_soundboard_86(None, true);

        // Advance past data_write_irq_wait so IRQ reporting is not delayed.
        bus.current_cycle = 200_000;

        // Enable IRQ (bit 5) and FIFO (bit 7), set sample rate.
        // This also sets last_clock_for_wait = current_cycle (IRQ clear + empty buffer).
        bus.io_write_byte(0xA468, 0xA0); // bit 7=1 (play), bit 5=1 (IRQ en)

        // Set IRQ threshold to 128 bytes (value 0 -> (0+1)*128 = 128).
        bus.io_write_byte(0xA46A, 0x00);

        // Advance cycle past data_write_irq_wait before reading status.
        bus.current_cycle = 400_000;

        // FIFO is empty and below threshold -> IRQ should be flagged.
        let ctrl = bus.io_read_byte(0xA468);
        assert_ne!(
            ctrl & 0x10,
            0,
            "IRQ flag should be set when FIFO below threshold"
        );

        // Clear IRQ by writing bit 4 = 0 (resets last_clock_for_wait since buffer empty).
        bus.io_write_byte(0xA468, 0xA0);

        // Advance past the new wait period.
        bus.current_cycle = 600_000;
        let ctrl = bus.io_read_byte(0xA468);
        // IRQ re-asserts because buffer is still below threshold.
        assert_ne!(
            ctrl & 0x10,
            0,
            "IRQ should re-assert when buffer still below threshold"
        );
    }

    #[test]
    fn pcm86_buffer_overflow_discards_oldest() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.install_soundboard_86(None, true);

        // Write exactly 64 KB (the physical buffer size).
        for i in 0..65536u32 {
            bus.io_write_byte(0xA46C, (i & 0xFF) as u8);
        }

        // Writing one more should not crash (overflow protection kicks in).
        bus.io_write_byte(0xA46C, 0xFF);

        // The real_buf should be capped below the buffer size.
        let state = bus.soundboard_86.as_ref().unwrap().save_state();
        assert!(
            state.pcm86.real_buf < 65536,
            "real_buf should be capped after overflow"
        );
    }

    #[test]
    fn pcm86_dactrl_modes_and_fifo_threshold_dual_purpose() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.install_soundboard_86(None, true);

        // Write dactrl when IRQ is disabled (fifo bit 5 = 0).
        bus.io_write_byte(0xA468, 0x00); // IRQ disabled
        bus.io_write_byte(0xA46A, 0x70); // 8-bit stereo
        assert_eq!(bus.io_read_byte(0xA46A), 0x70);

        // Enable IRQ (fifo bit 5 = 1). Writing 0xA46A now sets FIFO threshold.
        bus.io_write_byte(0xA468, 0x20); // IRQ enabled
        bus.io_write_byte(0xA46A, 0x03); // threshold = (3+1)*128 = 512

        // dactrl read should still return the previously set value (0x70).
        assert_eq!(bus.io_read_byte(0xA46A), 0x70);

        // Verify threshold was set correctly.
        let state = bus.soundboard_86.as_ref().unwrap().save_state();
        assert_eq!(state.pcm86.fifo_size, 512);
    }

    #[test]
    fn sb16_dsp_reset_and_version_via_ports() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.install_sound_blaster_16();

        // DSP reset: write 1 then 0 to port base+0x2600
        bus.io_write_byte(0x26D2, 0x01);
        bus.io_write_byte(0x26D2, 0x00);

        // Read ready byte from port base+0x2A00
        assert_eq!(bus.io_read_byte(0x2AD2), 0xAA);

        // Send version command (0xE1) via port base+0x2C00
        bus.io_write_byte(0x2CD2, 0xE1);

        // Read version: 4, 12
        assert_eq!(bus.io_read_byte(0x2AD2), 4);
        assert_eq!(bus.io_read_byte(0x2AD2), 12);
    }

    #[test]
    fn sb16_mixer_write_read_via_ports() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.install_sound_blaster_16();

        // Write mixer address (base+0x2400), then data (base+0x2500)
        bus.io_write_byte(0x24D2, 0x32); // Select voice volume left
        bus.io_write_byte(0x25D2, 0xA0); // Write value

        // Read it back
        bus.io_write_byte(0x24D2, 0x32);
        assert_eq!(bus.io_read_byte(0x25D2), 0xA0);
    }

    #[test]
    fn sb16_opl3_status_read() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.install_sound_blaster_16();

        // Read OPL3 status (base+0x2000) - should not panic and return a value
        let status = bus.io_read_byte(0x20D2);
        // After reset, timer flags should be clear
        assert_eq!(status & 0xE0, 0);
    }

    #[test]
    fn sb16_ports_do_not_conflict_with_86_board() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.install_soundboard_86(None, true);
        bus.install_sound_blaster_16();

        // 86 board still responds at 0x0188/0x018A
        bus.io_write_byte(0x0188, 0xFF);
        assert_eq!(bus.io_read_byte(0x018A), 0x01); // YM2608 chip ID

        // SB16 responds at its own ports
        bus.io_write_byte(0x26D2, 0x01);
        bus.io_write_byte(0x26D2, 0x00);
        assert_eq!(bus.io_read_byte(0x2AD2), 0xAA);
    }
}
