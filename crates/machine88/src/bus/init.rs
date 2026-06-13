//! Bus construction, ROM application, and power-on reset state.

use common::{BeeperKind, Tracing};
use device::{
    beeper::Beeper,
    cdrom_pc88::Pc88Cdrom,
    i8214_pic::I8214Pic,
    i8251_serial::I8251Serial,
    i8255::I8255,
    i8257_dma::I8257Dma,
    palette_pc88::Pc88Palette,
    soundboard_ii::SoundboardII,
    upd765a_fdc::{FloppyController, Upd765aFdc},
    upd3301_crtc::Upd3301,
    upd4990a_rtc::Upd4990aRtc,
};
use software_renderer::Pc88Renderer;

use super::sub_mem::SubMemory;
use crate::{
    bus::Pc8801Bus,
    config::{BootMode, ClockConfig, ClockSelect, MonitorTiming, Pc8801Model},
    memory::Pc8801Memory,
    rom::LoadedRoms,
    scheduler::{Event88, Pc8801Scheduler},
};

/// Power-on N88-BASIC boot mode. The application overrides it after
/// construction via `Pc8801Bus::set_boot_mode` from its configuration.
const DEFAULT_BOOT_MODE: BootMode = BootMode::V2;
/// Default monitor timing: 15 kHz (200-line).
const DEFAULT_HIRESO: bool = false;
/// Power-on memory-wait switch state (compatible). Overridden by the application
/// via `Pc8801Bus::set_memory_wait`.
const DEFAULT_MEM_WAIT_ON: bool = true;
/// Power-on 8 MHz wait mode (compatible, i.e. not high-speed). Overridden by the
/// application via `Pc8801Bus::set_eight_mhz_wait`.
const DEFAULT_EIGHT_MHZ_FAST: bool = false;
/// Fixed beeper tone frequency (the PC-88 1-bit speaker is a ~2400 Hz square wave).
const BEEP_FREQUENCY_HZ: u32 = 2400;

impl<T: Tracing + Default> Pc8801Bus<T> {
    /// Creates a bus in its power-on reset state with empty ROM arrays.
    ///
    /// Bank-control registers reset to zero, which maps the N88-BASIC ROM at
    /// 0x0000-0x7FFF (MMODE and RMODE clear), with the dictionary ROM disabled.
    pub fn new(model: Pc8801Model, clock_select: ClockSelect, sample_rate: u32) -> Self {
        let clocks = ClockConfig {
            main_clock_hz: clock_select.main_clock_hz(model),
            sub_clock_hz: model.sub_clock_hz(),
            sample_rate,
        };

        let mut memory = Pc8801Memory::new(model);
        memory.set_boot_mode(DEFAULT_BOOT_MODE);

        let clock_timer_period = u64::from(clocks.main_clock_hz) / super::CLOCK_TIMER_HZ;
        let horizontal_freq = if DEFAULT_HIRESO {
            super::HORIZONTAL_FREQ_24KHZ
        } else {
            super::HORIZONTAL_FREQ_15KHZ
        };
        let crtc_line_period = (u64::from(clocks.main_clock_hz) / horizontal_freq).max(1);

        // PIO data-rate pacing: 250 kbps MFM is 31250 bytes/s.
        let drq_byte_cycles = (u64::from(clocks.main_clock_hz) / 31_250).max(1);
        let cpu_clock_low = clocks.main_clock_hz < super::CPU_CLOCK_LOW_THRESHOLD_HZ;
        let sub_to_main_shift = if cpu_clock_low { 0 } else { 1 };
        let (gvram_access_limit_read, gvram_access_limit_write) = if cpu_clock_low {
            (
                super::GVRAM_ACCESS_LIMIT_4MHZ,
                super::GVRAM_ACCESS_LIMIT_4MHZ,
            )
        } else {
            (
                super::GVRAM_ACCESS_LIMIT_8MHZ_READ,
                super::GVRAM_ACCESS_LIMIT_8MHZ_WRITE,
            )
        };

        let mut scheduler = Pc8801Scheduler::new();
        scheduler.schedule(Event88::ClockTimer, clock_timer_period);
        scheduler.schedule(Event88::CrtcDisplayStart, crtc_line_period);
        let next_event_cycle = scheduler.next_event_cycle().unwrap_or(u64::MAX);

        Self {
            memory,
            scheduler,
            pic: I8214Pic::new(),
            crtc: Upd3301::new(DEFAULT_HIRESO),
            dma: I8257Dma::new(),
            palette: Pc88Palette::new(),
            renderer: Pc88Renderer::new(&[]),
            soundboard_ii: SoundboardII::new(clocks.main_clock_hz, clocks.sample_rate),
            cdrom: Pc88Cdrom::new(clocks.sample_rate),
            beeper: Beeper::new(
                BeeperKind::Fixed {
                    hz: BEEP_FREQUENCY_HZ,
                },
                clocks.main_clock_hz,
            ),
            rtc: Upd4990aRtc::new(),
            serial: I8251Serial::new(),
            host_local_time_fn: super::default_local_time,
            port10: 0,
            kanji1_addr: 0,
            kanji2_addr: 0,
            current_cycle: 0,
            next_event_cycle,
            vrtc_active: false,
            clock_timer_period,
            crtc_line_period,
            crtc_current_row: 0,
            hireso: DEFAULT_HIRESO,
            port30: 0,
            port40: 0,
            baud_rate: 4,
            layer_disable: 0,
            keyboard_rows: [0xFF; 16],
            mouse_x: 0,
            mouse_y: 0,
            mouse_latch_x: 0,
            mouse_latch_y: 0,
            mouse_data: 0,
            mouse_phase: 3,
            mouse_strobe_level: false,
            mouse_strobe_cycle: 0,
            mouse_buttons: 0xFF,
            mouse_timeout_cycles: (u64::from(clocks.main_clock_hz) / 1000)
                * super::MOUSE_STROBE_TIMEOUT_MS,
            mouse_strobe_seen: false,
            joystick_port_a: 0xFF,
            joystick_port_b: 0xFF,
            monitor_timing: MonitorTiming::default(),
            busreq_clocks: 0,
            busreq_until: 0,
            memory_wait_cycles: 0,
            cpu_clock_low,
            mem_wait_on: DEFAULT_MEM_WAIT_ON,
            eight_mhz_fast: DEFAULT_EIGHT_MHZ_FAST,
            gvram_access_count: 0,
            gvram_access_limit_read,
            gvram_access_limit_write,
            display_width: 640,
            display_height: 200,
            kanji1: Vec::new(),
            kanji2: Vec::new(),
            sub_mem: SubMemory::new(),
            sub_cycle: 0,
            sub_to_main_shift,
            sub_clock_credit: 0,
            fdc: Upd765aFdc::new(),
            floppy: FloppyController::new(),
            ppi_main: I8255::new(),
            ppi_sub: I8255::new(),
            drive_mode: 0,
            motor_on: 0,
            tc_active: false,
            resync_until: 0,
            drq_byte_cycles,
            clocks,
            model,
            tracer: T::default(),
        }
    }
}

impl<T: Tracing> Pc8801Bus<T> {
    /// Applies a loaded and validated ROM set to the bus.
    pub fn load_roms(&mut self, roms: &LoadedRoms) {
        self.memory.load_n88_rom(&roms.n88);
        for (bank, data) in roms.n88_ext.iter().enumerate() {
            self.memory.load_n88_ext_rom(bank, data);
        }
        self.memory.load_n_basic_rom(&roms.n_basic);
        self.memory.load_n80_mkii_rom(roms.n80_mkii.as_deref());
        self.memory.load_n80_mkiisr_rom(roms.n80_mkiisr.as_deref());
        self.memory.load_n80sr_rom(roms.n80sr.as_deref());
        self.memory.load_dictionary_rom(&roms.dictionary);
        self.memory.load_cdbios_rom(&roms.cdrom_bios);
        self.kanji1 = roms.kanji1.clone();
        self.kanji2 = roms.kanji2.clone();
        self.renderer.update_font_rom(&roms.kanji1);
        self.sub_mem.load_disk_rom(&roms.disk);
    }
}
