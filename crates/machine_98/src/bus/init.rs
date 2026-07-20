use common::{BeeperKind, CpuMode, CpuType, MachineModel};
use device::{
    beeper::Beeper,
    cgrom::Cgrom,
    display_control::DisplayControl,
    egc::Egc,
    grcg::Grcg,
    i8237_dma::I8237Dma,
    i8251_keyboard::I8251Keyboard,
    i8251_serial::I8251Serial,
    i8253_pit::I8253Pit,
    i8255_mouse_ppi::I8255MousePpi,
    i8255_system_ppi::I8255SystemPpi,
    i8259a_pic::I8259aPic,
    mpu401::Mpu401,
    palette::Palette,
    pegc::Pegc,
    printer::Printer,
    sasi::SasiController,
    sdip::Sdip,
    upd765a_fdc::FloppyController,
    upd4990a_rtc::Upd4990aRtc,
    upd7220_gdc::{DISPLAY_MODE_GRAPHICS, Gdc, GdcScrollPartition},
    upd52611_crtc::Upd52611Crtc,
};
use software_renderer::SoftwareRenderer;

use crate::{
    ClockConfig, Pc9801Bus, TraceSink,
    bus::{
        BootDevice, DMA_ACCESS_CTRL_20BIT, GRCG_WAIT_CYCLES, KEYBOARD_ROM_OFFSET_F,
        KEYBOARD_ROM_OFFSET_VM, MOUSE_TIMER_DEFAULT_SETTING, MOUSE_TIMER_IRQ_LINE,
        TRAM_WAIT_CYCLES, VRAM_WAIT_CYCLES,
    },
    memory::Pc9801Memory,
    scheduler::{Event98, Pc98Scheduler},
};

#[rustfmt::skip]
const KEYBOARD_TABLES: [[u8; 0x60]; 8] = [
    // Table 0: normal
    [
        0x1b, b'1', b'2', b'3', b'4', b'5', b'6', b'7',
        b'8', b'9', b'0', b'-', b'^', b'\\', 0x08, 0x09,
        b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i',
        b'o', b'p', b'@', b'[', 0x0d, b'a', b's', b'd',
        b'f', b'g', b'h', b'j', b'k', b'l', b';', b':',
        b']', b'z', b'x', b'c', b'v', b'b', b'n', b'm',
        b',', b'.', b'/', 0xff, b' ', 0x35, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3e, 0x00,
        b'-', b'/', b'7', b'8', b'9', b'*', b'4', b'5',
        b'6', b'+', b'1', b'2', b'3', b'=', b'0', b',',
        b'.', 0x51, 0xff, 0xff, 0xff, 0xff, 0x62, 0x63,
        0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b,
    ],
    // Table 1: shift
    [
        0x1b, b'!', b'"', b'#', b'$', b'%', b'&', b'\'',
        b'(', b')', b'0', b'=', b'^', b'|', 0x08, 0x09,
        b'Q', b'W', b'E', b'R', b'T', b'Y', b'U', b'I',
        b'O', b'P', b'~', b'{', 0x0d, b'A', b'S', b'D',
        b'F', b'G', b'H', b'J', b'K', b'L', b'+', b'*',
        b'}', b'Z', b'X', b'C', b'V', b'B', b'N', b'M',
        b'<', b'>', b'?', b'_', b' ', 0xa5, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xae, 0x00,
        b'-', b'/', b'7', b'8', b'9', b'*', b'4', b'5',
        b'6', b'+', b'1', b'2', b'3', b'=', b'0', b',',
        b'.', 0xa1, 0xff, 0xff, 0xff, 0xff, 0x82, 0x83,
        0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b,
    ],
    // Table 2: CAPS
    [
        0x1b, b'1', b'2', b'3', b'4', b'5', b'6', b'7',
        b'8', b'9', b'0', b'-', b'^', b'\\', 0x08, 0x09,
        b'Q', b'W', b'E', b'R', b'T', b'Y', b'U', b'I',
        b'O', b'P', b'@', b'[', 0x0d, b'A', b'S', b'D',
        b'F', b'G', b'H', b'J', b'K', b'L', b';', b':',
        b']', b'Z', b'X', b'C', b'V', b'B', b'N', b'M',
        b',', b'.', b'/', 0xff, b' ', 0x35, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3e, 0x00,
        b'-', b'/', b'7', b'8', b'9', b'*', b'4', b'5',
        b'6', b'+', b'1', b'2', b'3', b'=', b'0', b',',
        b'.', 0xa1, 0xff, 0xff, 0xff, 0xff, 0x62, 0x63,
        0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b,
    ],
    // Table 3: shift + CAPS
    [
        0x1b, b'!', b'"', b'#', b'$', b'%', b'&', b'\'',
        b'(', b')', b'0', b'=', b'`', b'|', 0x08, 0x09,
        b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i',
        b'o', b'p', b'~', b'{', 0x0d, b'a', b's', b'd',
        b'f', b'g', b'h', b'j', b'k', b'l', b'+', b'*',
        b'}', b'z', b'x', b'c', b'v', b'b', b'n', b'm',
        b'<', b'>', b'?', b'_', b' ', 0xa5, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xae, 0x00,
        b'-', b'/', b'7', b'8', b'9', b'*', b'4', b'5',
        b'6', b'+', b'1', b'2', b'3', b'=', b'0', b',',
        b'.', 0xa1, 0xff, 0xff, 0xff, 0xff, 0x82, 0x83,
        0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b,
    ],
    // Table 4: kana
    [
        0x1b, 0xc7, 0xcc, 0xb1, 0xb3, 0xb4, 0xb5, 0xd4,
        0xd5, 0xd6, 0xdc, 0xce, 0xcd, 0xb0, 0x08, 0x09,
        0xc0, 0xc3, 0xb2, 0xbd, 0xb6, 0xdd, 0xc5, 0xc6,
        0xd7, 0xbe, 0xde, 0xdf, 0x0d, 0xc1, 0xc4, 0xbc,
        0xca, 0xb7, 0xb8, 0xcf, 0xc9, 0xd8, 0xda, 0xb9,
        0xd1, 0xc2, 0xbb, 0xbf, 0xcb, 0xba, 0xd0, 0xd3,
        0xc8, 0xd9, 0xd2, 0xdb, b' ', 0x35, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3e, 0x00,
        b'-', b'/', b'7', b'8', b'9', b'*', b'4', b'5',
        b'6', b'+', b'1', b'2', b'3', b'=', b'0', b',',
        b'.', 0x51, 0xff, 0xff, 0xff, 0xff, 0x62, 0x63,
        0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b,
    ],
    // Table 5: kana + shift
    [
        0x1b, 0xc7, 0xcc, 0xa7, 0xa9, 0xaa, 0xab, 0xac,
        0xad, 0xae, 0xa6, 0xce, 0xcd, 0xb0, 0x08, 0x09,
        0xc0, 0xc3, 0xa8, 0xbd, 0xb6, 0xdd, 0xc5, 0xc6,
        0xd7, 0xbe, 0xde, 0xa2, 0x0d, 0xc1, 0xc4, 0xbc,
        0xca, 0xb7, 0xb8, 0xcf, 0xc9, 0xd8, 0xda, 0xb9,
        0xa3, 0xaf, 0xbb, 0xbf, 0xcb, 0xba, 0xd0, 0xd3,
        0xa4, 0xa1, 0xa5, 0xdb, b' ', 0xa5, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xae, 0x00,
        b'-', b'/', b'7', b'8', b'9', b'*', b'4', b'5',
        b'6', b'+', b'1', b'2', b'3', b'=', b'0', b',',
        b'.', 0xa1, 0xff, 0xff, 0xff, 0xff, 0x82, 0x83,
        0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b,
    ],
    // Table 6: grph
    [
        0x1b, 0xff, 0xff, 0xff, 0xff, 0xf2, 0xf3, 0xf4,
        0xf5, 0xf6, 0xf7, 0x8c, 0x8b, 0xf1, 0x08, 0x09,
        0x9c, 0x9d, 0xe4, 0xe5, 0xee, 0xef, 0xf0, 0xe8,
        0xe9, 0x8d, 0x8a, 0xff, 0x0d, 0x9e, 0x9f, 0xe6,
        0xe7, 0xec, 0xed, 0xea, 0xeb, 0x8e, 0x89, 0x94,
        0xff, 0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86,
        0x87, 0x88, 0x97, 0xff, 0x20, 0x35, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0x00,
        b'-', b'/', 0x98, 0x91, 0x99, 0x95, 0xe1, 0xe2,
        0xe3, 0xe0, 0x93, 0x8f, 0x92, 0x96, 0x9a, 0x90,
        0x9b, 0x51, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    ],
    // Table 7: ctrl
    [
        0x1b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0x1e, 0x1c, 0x08, 0x09,
        0x11, 0x17, 0x05, 0x12, 0x14, 0x19, 0x15, 0x09,
        0x0f, 0x10, 0x00, 0x1b, 0x0d, 0x01, 0x13, 0x04,
        0x06, 0x07, 0x08, 0x0a, 0x0b, 0x0c, 0xff, 0xff,
        0x1d, 0x1a, 0x18, 0x03, 0x16, 0x02, 0x0e, 0x0d,
        0xff, 0xff, 0xff, 0x1f, 0x20, 0xb5, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0x00,
        b'-', b'/', b'7', b'8', b'9', b'*', b'4', b'5',
        b'6', b'+', b'1', b'2', b'3', b'=', b'0', b',',
        b'.', 0xb1, 0xff, 0xff, 0xff, 0xff, 0x92, 0x93,
        0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b,
    ],
];

impl<T: TraceSink> Pc9801Bus<T> {
    /// Creates a traced bus configured for the given machine model and CPU mode.
    ///
    /// `cpu_mode` selects between Low and High CPU clock for PC-9801 models.
    /// PC-9821 models ignore this value.
    pub fn new_with_trace_sink(
        machine_model: MachineModel,
        cpu_mode: CpuMode,
        sample_rate: u32,
        tracer: T,
    ) -> Self {
        let clocks = ClockConfig {
            cpu_clock_hz: machine_model.cpu_clock_hz(cpu_mode),
            pit_clock_hz: machine_model.pit_clock_hz(),
            sample_rate,
        };
        let is_8mhz_lineage = machine_model.is_8mhz_pit_lineage();

        let mut bus = Self {
            current_cycle: 0,
            next_event_cycle: u64::MAX,
            nmi_enabled: false,
            clocks,
            memory: Pc9801Memory::new(machine_model, machine_model.extended_ram_default_size()),
            pic: I8259aPic::new(),
            scheduler: Pc98Scheduler::new(),
            pit: I8253Pit::new(is_8mhz_lineage),
            dma: I8237Dma::new(),
            keyboard: I8251Keyboard::new(),
            keyboard_chained_raw_code: None,
            serial: I8251Serial::new(),
            gdc_master: Gdc::new_master(clocks.cpu_clock_hz),
            gdc_slave: Gdc::new_slave(clocks.cpu_clock_hz),
            floppy: FloppyController::new(),
            fdd640k_hle: device::fdd640k_hle::Fdd640kHle::new(),
            fdd320_ppi: device::fdd320_ppi::Fdd320Ppi::new(),
            system_ppi: I8255SystemPpi::new(is_8mhz_lineage),
            printer: Printer::new(),
            display_control: DisplayControl::new(),
            cgrom: Cgrom::new(),
            crtc: Upd52611Crtc::new(),
            grcg: Grcg::new(machine_model.grcg_chip_version()),
            egc: Egc::new(),
            pegc: Pegc::new(),
            palette: Palette::new(),
            soundboard_14: None,
            soundboard_26k: None,
            soundboard_86: None,
            sound_blaster_16: None,
            ga1280a: None,
            beeper: Beeper::new(machine_model.beeper_kind(), clocks.pit_clock_hz),
            rtc: Upd4990aRtc::new(),
            host_date_time_source: common::default_host_date_time_source(),
            automation_audio_remainder: 0,
            mpu401: Mpu401::new(),
            #[cfg(feature = "mt32")]
            mt32: None,
            #[cfg(feature = "sc55")]
            sc55: None,
            mouse_ppi: I8255MousePpi::new(),
            mouse_timer_setting: MOUSE_TIMER_DEFAULT_SETTING,
            sasi: SasiController::new(),
            ide: device::ide::IdeController::new(sample_rate),
            sdip: Sdip::new(),
            bios: device::bios::BiosController::new(),
            bios_interval_timer_active: false,
            current_cpu_protected_mode: false,
            a20_enabled: false,
            machine_model,
            reset_pending: false,
            shutdown_requested: false,
            needs_full_reinit: false,
            warm_reset_context: None,
            // Initialize the renderer with empty font ROM bytes; the bus's
            // `load_font_rom` (or first VSYNC after CG-RAM dirties) will
            // refresh the renderer's copy.
            software_renderer: Box::new(SoftwareRenderer::new(&[])),
            display_width: SoftwareRenderer::WIDTH as u32,
            display_height: 400,
            presented_frames: 0,
            dma_access_ctrl: match machine_model {
                MachineModel::PC9801F | MachineModel::PC9801VM | MachineModel::PC9801VX => {
                    DMA_ACCESS_CTRL_20BIT
                }
                MachineModel::PC9801RS
                | MachineModel::PC9801RA
                | MachineModel::PC9821AS
                | MachineModel::PC9821AP => 0x00,
            },
            vram_ems_bank: 0x00,
            ram_window: 0x08,
            hole_15m_control: 0x00,
            protected_memory_max: 0x00,
            b_bank_ems: machine_model.has_b_bank_ems(),
            graphics_extension_enabled: false,
            pending_wait_cycles: 0,
            rtc_control_22: 0x00,
            key_sense_0ec: 0xFF,
            external_interrupt_43a: 0xFF,
            video_ff2_index: 0x00,
            wab_index: 0x00,
            wab_data: [0u8; 8],
            wab_relay: 0xFC,
            cpu_mode_534: 0x00,
            simm_address_register: 0x00,
            simm_data: [0u8; 32],
            memory_bank_063c: 0x00,
            cache_control_063f: 0x00,
            tram_wait: TRAM_WAIT_CYCLES,
            vram_wait: VRAM_WAIT_CYCLES,
            grcg_wait: GRCG_WAIT_CYCLES,
            tracer,
            fdd_seek_cylinder: [0; 4],
            fdd_read_id_index: [0; 4],
            hle_cr0: 0,
            hle_cr3: 0,
            boot_device: BootDevice::default(),
            dos: None,
            ems_enabled: true,
            xms_enabled: true,
            xms_32_enabled: false,
            xms_hmamin_kb: 0,
            text_extractor: None,
        };

        match machine_model {
            MachineModel::PC9801F | MachineModel::PC9801VM => {}
            MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => bus.set_cg_ram(true),
        }

        // Emulate a machine with the analog 16-color graphics extension installed.
        bus.set_graphics_extension_enabled(true);

        // Set the mouse interpolation clock from the CPU clock.
        bus.mouse_ppi.set_cpu_clock(clocks.cpu_clock_hz);

        // Schedule the first VSYNC event after one display period.
        bus.scheduler
            .schedule(Event98::GdcVsync, bus.gdc_master.state.display_period);
        bus.system_ppi.set_cpu_mode_bit(match machine_model {
            MachineModel::PC9801F => false,
            MachineModel::PC9801VM => true,
            MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => false,
        });
        bus.update_next_event_cycle();
        bus.initialize_post_boot_state();
        bus
    }

    /// Populates the IVT at 0x0000–0x03FF from the stub BIOS ROM's vector table.
    ///
    /// The ROM contains a vector initialization table at a known offset within
    /// the BIOS code segment (0xFD80). Each entry is a (vector_number, handler_offset)
    /// pair of 16-bit words, terminated by 0xFFFF. The handler offsets are relative
    /// to segment 0xFD80.
    pub(super) fn populate_ivt_from_stub_bios(&mut self) {
        const BIOS_CODE_SEG: u16 = 0xFD80;
        const BIOS_CODE_OFFSET: usize = 0x15800;
        // Read the vector table offset from the metadata header at the start of
        // the BIOS code region (+0 = vector table segment offset).
        let vector_table_seg_offset = u16::from_le_bytes([
            self.memory.rom_byte(BIOS_CODE_OFFSET),
            self.memory.rom_byte(BIOS_CODE_OFFSET + 1),
        ]) as usize;

        let mut rom_pos = BIOS_CODE_OFFSET + vector_table_seg_offset;

        loop {
            let vector_num = u16::from_le_bytes([
                self.memory.rom_byte(rom_pos),
                self.memory.rom_byte(rom_pos + 1),
            ]);
            if vector_num == 0xFFFF {
                break;
            }
            let handler_offset = u16::from_le_bytes([
                self.memory.rom_byte(rom_pos + 2),
                self.memory.rom_byte(rom_pos + 3),
            ]);
            let ivt_addr = (vector_num as usize) * 4;
            self.memory.state.ram[ivt_addr] = handler_offset as u8;
            self.memory.state.ram[ivt_addr + 1] = (handler_offset >> 8) as u8;
            self.memory.state.ram[ivt_addr + 2] = BIOS_CODE_SEG as u8;
            self.memory.state.ram[ivt_addr + 3] = (BIOS_CODE_SEG >> 8) as u8;
            rom_pos += 4;
        }
    }

    /// Populates BDA fields based on machine model and clock lineage.
    fn populate_bda(&mut self) {
        let cpu_type = self.machine_model.cpu_type();
        // MSW3 from text VRAM memory switch area (stride 4, at offset 0x3FEA).
        let msw3 = self.memory.state.text_vram[0x3FEA];

        // BIOS_FLAG0 (0x0500): per undoc98/memsys.txt:
        //   bit 0: machine identification (0 = PC-9801 F/E/M lineup, 1 = others)
        //   bit 1: FDD generation (0 = 640KB, 1 = 1MB)
        //   bit 5: keyboard buffer overflow beep (0 = beep, 1 = silent)
        self.memory.state.ram[0x0500] = match self.machine_model {
            MachineModel::PC9801F => 0x20,
            MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => 0x03,
        };

        // BIOS_FLAG1 (0x0501):
        //   bit 7 = 1 if 8MHz lineage
        //   bit 6 = 1 if V30 CPU
        //   bit 5 = 1 (always set)
        //   bits 0-2 = real-mode memory size from MSW3 (128KB units above base 128KB)
        let bios_flag1 = match self.machine_model {
            MachineModel::PC9801F => 0xA0 | (msw3 & 0x07),
            MachineModel::PC9801VM => 0x60 | (msw3 & 0x07),
            MachineModel::PC9801VX => 0x20 | (msw3 & 0x07),
            MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => 0xA0 | (msw3 & 0x07),
        };
        self.memory.state.ram[0x0501] = bios_flag1;

        // BIOS_FLAG2 (0x0400): per undoc98/memsys.txt:
        //   bit 7: RAM drive capability
        //   bit 5: RESUME feature ON/OFF
        //   bit 4: resume/suspend notification
        //   bit 3: V33A identification
        //   bit 2: CPU speed middle mode
        //   bit 0: BRANCH BIOS 4670 I/F
        self.memory.state.ram[0x0400] = 0x00;

        // SYS_TYPE (0x0480): CPU type + hardware detection flags.
        //   bits 0-1: CPU type (8086/V30=0x00, I286=0x01, I386=0x03)
        //   bit 3: dual-use FDD present
        //   bit 6: EGC / protected mode test passed
        //   bit 7: IDE hard disk controller present
        let sys_type = match cpu_type {
            CpuType::I8086 | CpuType::V30 => 0x00,
            CpuType::I286 => 0x01,
            CpuType::I386 | CpuType::I486DX => 0x4B,
        };
        let sys_type = sys_type
            | match self.machine_model {
                MachineModel::PC9801F
                | MachineModel::PC9801VM
                | MachineModel::PC9801VX
                | MachineModel::PC9801RS
                | MachineModel::PC9801RA => 0x00,
                MachineModel::PC9821AS | MachineModel::PC9821AP => 0x80,
            };
        self.memory.state.ram[0x0480] = sys_type;

        // USER_SP / USER_SS (0x0404 / 0x0406): saved stack from the ITF
        // protected mode test on 386+ CPUs.
        if cpu_type >= CpuType::I386 {
            self.memory.state.ram[0x0404] = 0xF8;
            self.memory.state.ram[0x0405] = 0x00;
            self.memory.state.ram[0x0406] = 0x30;
            self.memory.state.ram[0x0407] = 0x00;
        }

        // EXPMMSZ (0x0401): extended memory size in 128KB units.
        let expmmsz = (self.memory.state.extended_ram.len() / 0x20000) as u8;
        self.memory.state.ram[0x0401] = expmmsz;

        // GRPH_ARCH (0x045C): extended graph architecture identification.
        //   bit 6: 1 = PC-9821 with PEGC (Pure Enhanced Graphics Controller).
        // Ref: undoc98 `memsys.txt` (port 0x045C bit 6).
        self.memory.state.ram[0x045C] = match self.machine_model.has_pegc() {
            false => 0x00,
            true => 0x40,
        };

        // BIOS_FLAG3 (0x0481): 386 machines have bit 5 set.
        self.memory.state.ram[0x0481] = match cpu_type {
            CpuType::I8086 | CpuType::I286 | CpuType::V30 => 0x00,
            CpuType::I386 | CpuType::I486DX => 0x20,
        };

        // F2HD_MODE (0x0493): all drives 2HD on later machines.
        self.memory.state.ram[0x0493] = match self.machine_model {
            MachineModel::PC9801F => 0x00,
            MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => 0xFF,
        };

        // F2DD_MODE (0x05CA): all drives normal density on later machines.
        self.memory.state.ram[0x05CA] = match self.machine_model {
            MachineModel::PC9801F => 0x00,
            MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => 0xFF,
        };

        // F2DD_POINTER (0x05CC) / F2HD_POINTER (0x05F8): far pointers to format
        // tables in ROM. The offsets differ between BIOS generations.
        match self.machine_model {
            MachineModel::PC9801F => {
                self.memory.state.ram[0x05CC..0x05D0].fill(0);
                self.memory.state.ram[0x05F8..0x05FC].fill(0);
            }
            MachineModel::PC9801VM | MachineModel::PC9801VX => {
                let (f2hd_off, f2dd_off): (u16, u16) = (0x1AB4, 0x1ADC);
                self.memory.state.ram[0x05CC..0x05D0].copy_from_slice(&[
                    f2dd_off as u8,
                    (f2dd_off >> 8) as u8,
                    0x80,
                    0xFD,
                ]);
                self.memory.state.ram[0x05F8..0x05FC].copy_from_slice(&[
                    f2hd_off as u8,
                    (f2hd_off >> 8) as u8,
                    0x80,
                    0xFD,
                ]);
            }
            MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => {
                let (f2hd_off, f2dd_off): (u16, u16) = (0x1AAF, 0x1AD7);
                self.memory.state.ram[0x05CC..0x05D0].copy_from_slice(&[
                    f2dd_off as u8,
                    (f2dd_off >> 8) as u8,
                    0x80,
                    0xFD,
                ]);
                self.memory.state.ram[0x05F8..0x05FC].copy_from_slice(&[
                    f2hd_off as u8,
                    (f2hd_off >> 8) as u8,
                    0x80,
                    0xFD,
                ]);
            }
        }

        // CRT_RASTER (0x053B): raster count.
        self.memory.state.ram[0x053B] = 0x0F;

        // CRT_STS_FLAG (0x053C): display status.
        self.memory.state.ram[0x053C] = 0x84;

        // PRXCRT (0x054C): display config (color, GRCG present, 8MHz).
        // Bit 6: GDC 2.5 MHz mode (set when DIP SW 2-8 is OFF = 2.5 MHz).
        let gdc_2_5mhz = self.system_ppi.state.dip_switch_2 & 0x80 != 0;
        self.memory.state.ram[0x054C] = if gdc_2_5mhz { 0x4E } else { 0x0E };

        // PRXDUPD (0x054D): graphics mode / GRCG version.
        // Bit 4: EGC present. Bit 5: GDC 5 MHz capable (from DIP SW 2-8).
        let mut prxdupd: u8 = 0x00;
        match self.machine_model {
            MachineModel::PC9801F | MachineModel::PC9801VM => {}
            MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => {
                prxdupd |= 0x50;
            }
        }
        if !gdc_2_5mhz {
            prxdupd |= 0x20;
        }
        self.memory.state.ram[0x054D] = prxdupd;

        // DISK_EQUIP (0x055C): 2 built-in 1MB FDD drives on later machines.
        self.memory.state.ram[0x055C] = match self.machine_model {
            MachineModel::PC9801F => 0x00,
            MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => 0x03,
        };

        // Keyboard shift table pointer -> ROM shift table at FD80:0B28.
        let keyboard_rom_offset = match self.machine_model {
            MachineModel::PC9801F => KEYBOARD_ROM_OFFSET_F,
            MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => KEYBOARD_ROM_OFFSET_VM,
        };
        self.memory.state.ram[0x0522] = keyboard_rom_offset as u8; // KB_SHIFT_TBL low
        self.memory.state.ram[0x0523] = (keyboard_rom_offset >> 8) as u8; // KB_SHIFT_TBL high

        // Keyboard buffer pointers.
        self.memory.state.ram[0x0524] = 0x02; // KB_BUF_HEAD low
        self.memory.state.ram[0x0525] = 0x05; // KB_BUF_HEAD high
        self.memory.state.ram[0x0526] = 0x02; // KB_BUF_TAIL low
        self.memory.state.ram[0x0527] = 0x05; // KB_BUF_TAIL high
        self.memory.state.ram[0x0528] = 0x00; // KB_COUNT low
        self.memory.state.ram[0x0529] = 0x00; // KB_COUNT high

        // Keyboard code table pointer -> ROM code table at FD80:0B28.
        self.memory.state.ram[0x05C6] = keyboard_rom_offset as u8; // KB_CODE_OFF low
        self.memory.state.ram[0x05C7] = (keyboard_rom_offset >> 8) as u8; // KB_CODE_OFF high
        self.memory.state.ram[0x05C8] = 0x80; // KB_CODE_SEG low
        self.memory.state.ram[0x05C9] = 0xFD; // KB_CODE_SEG high

        // IVT: INT 1Eh at 0x0078 -> E800:0000.
        self.memory.state.ram[0x0078] = 0x00; // offset low
        self.memory.state.ram[0x0079] = 0x00; // offset high
        self.memory.state.ram[0x007A] = 0x00; // segment low
        self.memory.state.ram[0x007B] = 0xE8; // segment high
    }

    /// Sets all device state to match the PC-98 post-BIOS state.
    pub(super) fn initialize_post_boot_state(&mut self) {
        // Load stub BIOS ROM, keyboard tables, and populate IVT.
        self.memory.load_stub_bios_rom();
        self.memory.install_nec_copyright(self.machine_model);
        let keyboard_rom_offset = match self.machine_model {
            MachineModel::PC9801F => KEYBOARD_ROM_OFFSET_F,
            MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => KEYBOARD_ROM_OFFSET_VM,
        };
        self.memory.install_keyboard_tables(
            &KEYBOARD_TABLES,
            keyboard_rom_offset,
            self.machine_model,
        );
        self.memory.install_disk_format_tables(self.machine_model);
        self.populate_ivt_from_stub_bios();

        // PIC: fully initialized with PC-98 vectors and cascade.
        self.pic.state.chips[0].icw = [0x11, 0x08, 0x80, 0x1D];
        self.pic.state.chips[0].imr = 0x3D;
        self.pic.state.chips[0].isr = 0;
        self.pic.state.chips[0].irr = 0;
        self.pic.state.chips[0].write_icw = 0;
        self.pic.state.chips[1].icw = [0x11, 0x10, 0x07, 0x09];
        self.pic.state.chips[1].imr = match self.machine_model {
            MachineModel::PC9801F => 0xFD,
            MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => 0xF7,
        };
        self.pic.state.chips[1].isr = 0;
        self.pic.state.chips[1].irr = 0;
        self.pic.state.chips[1].write_icw = 0;
        match self.machine_model {
            MachineModel::PC9801F | MachineModel::PC9801VM => {
                self.pic.state.chips[1].ocw3 = 0x0B;
            }
            MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => {
                self.pic.state.chips[0].ocw3 = 0x0B;
            }
            MachineModel::PC9801VX => {}
        }
        self.pic.invalidate_irq_cache();

        // PIT: ch0 mode 0 (interrupt on terminal count), ch1 mode 3 (beep),
        // ch2 mode 3 (baud rate). ch1 value is already set correctly by I8253Pit::new().
        // After POST, the real BIOS leaves ch0 in mode 0 (one-shot, already expired)
        // with IRQ 0 masked. The event must be scheduled so on_timer0_event keeps
        // the counter running (needed for correct get_count behavior), matching
        // the real BIOS where the PIT free-runs even without raising IRQ.
        self.pit.state.channels[0].ctrl = 0x30;
        self.pit.state.channels[0].value = 0;
        self.pit.state.channels[0].flag = 0;
        self.pit.state.channels[0].last_load_cycle = self.current_cycle;
        self.pit.state.channels[0].output = true;
        self.pit.state.channels[0].reload_pending = None;
        let cpu_cycles = self
            .pit
            .timer0_period_cycles(self.clocks.cpu_clock_hz, self.clocks.pit_clock_hz);
        self.scheduler
            .schedule(Event98::PitTimer0, self.current_cycle + cpu_cycles);
        match self.machine_model {
            MachineModel::PC9801F => {
                self.pit.state.channels[1].ctrl = 0x14;
                self.pit.state.channels[1].value = 0x0039;
            }
            MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => {
                self.pit.state.channels[1].ctrl = 0x36;
            }
        }
        self.pit.state.channels[2].ctrl = 0x76;
        self.pit.state.channels[2].value = 0;

        // Keyboard: async 8-bit even parity, RxE/TxE/DTR enabled.
        self.keyboard.state.mode = 0x5E;
        self.keyboard.state.command = 0x16;
        self.keyboard.state.expect_mode = false;

        // Serial 8251: mode set, then internal reset command, leaving expect_mode.
        self.serial.state.mode = 0x02;
        self.serial.state.command = 0x40;
        self.serial.state.expect_mode = true;

        // System PPI: machine-specific port_c values.
        self.system_ppi.state.port_c = match self.machine_model {
            MachineModel::PC9801F | MachineModel::PC9801VM => 0x18,
            MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => 0xB8,
        };

        // NMI gate enabled.
        self.nmi_enabled = true;

        // GDC master (text): scroll areas, pitch, mask, display enable.
        self.gdc_master.state.scroll[0] = GdcScrollPartition {
            start_address: 0x0000,
            line_count: 0x1FF,
            im: false,
            wd: false,
        };
        for i in 1..4 {
            self.gdc_master.state.scroll[i] = GdcScrollPartition {
                start_address: 0x0000,
                line_count: 0x010,
                im: false,
                wd: false,
            };
        }
        self.gdc_master.state.pitch = 80;
        self.gdc_master.state.mask = 0;
        self.gdc_master.state.cursor_bottom = 15;
        self.gdc_master.state.cursor_blink_rate = 12;
        self.gdc_master.state.lines_per_row = 16;
        self.gdc_master.state.draw_on_retrace = true;
        match self.machine_model {
            MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => {
                self.gdc_master.state.display_enabled = true;
            }
            MachineModel::PC9801F | MachineModel::PC9801VM => {
                self.gdc_master.state.pattern = 0;
            }
        }

        // GDC slave (graphics): SYNC params, scroll areas, and mask.
        self.gdc_slave.state.aw = 40;
        self.gdc_slave.state.hs = 4;
        self.gdc_slave.state.vs = 8;
        self.gdc_slave.state.hfp = 5;
        self.gdc_slave.state.hbp = 4;
        self.gdc_slave.state.vfp = 7;
        self.gdc_slave.state.vbp = 25;
        self.gdc_slave.state.al = 400;
        let gdc_is_2_5mhz = self.system_ppi.state.dip_switch_2 & 0x80 != 0;
        self.gdc_slave.state.lines_per_row = if gdc_is_2_5mhz { 2 } else { 1 };
        self.gdc_slave.state.scroll[0] = GdcScrollPartition {
            start_address: 0x0000,
            line_count: 0x1FF,
            im: false,
            wd: false,
        };
        for i in 1..4 {
            self.gdc_slave.state.scroll[i] = GdcScrollPartition {
                start_address: 0x0000,
                line_count: 0x010,
                im: false,
                wd: false,
            };
        }
        self.gdc_slave.state.mask = match self.machine_model {
            MachineModel::PC9801F | MachineModel::PC9801VM => 0,
            MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => 1,
        };
        self.gdc_slave.state.draw_on_retrace = true;
        match self.machine_model {
            MachineModel::PC9801F => {
                self.gdc_slave.state.status = 0x44;
            }
            MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => {}
        }
        match self.machine_model {
            MachineModel::PC9801F | MachineModel::PC9801VM => {
                self.gdc_slave.state.display_mode = DISPLAY_MODE_GRAPHICS;
            }
            MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => {}
        }

        // Display control.
        self.display_control.state.video_mode = 0x99;
        self.display_control.state.mode2 = 0x0100;
        self.display_control.state.display_line_count = match self.machine_model {
            MachineModel::PC9801VX => 23,
            MachineModel::PC9801F
            | MachineModel::PC9801VM
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => 1,
        };
        self.display_control.state.vsync_irq_enabled = true;

        // Analog palette: indices 0-7 dim, 8 half-bright, 9-15 bright (0x0F).
        let (dim, half_bright) = match self.machine_model {
            MachineModel::PC9801F | MachineModel::PC9801VM => (0x0Au8, [7u8, 7, 7]),
            MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => (0x07u8, [4u8, 4, 4]),
        };
        self.palette.state.analog[0] = [0, 0, 0];
        for i in 1u8..8 {
            let blue = if i & 1 != 0 { dim } else { 0 };
            let red = if i & 2 != 0 { dim } else { 0 };
            let green = if i & 4 != 0 { dim } else { 0 };
            self.palette.state.analog[i as usize] = [green, red, blue];
        }
        self.palette.state.analog[8] = half_bright;
        for i in 9u8..16 {
            let j = i - 8;
            let blue = if j & 1 != 0 { 0x0F } else { 0 };
            let red = if j & 2 != 0 { 0x0F } else { 0 };
            let green = if j & 4 != 0 { 0x0F } else { 0 };
            self.palette.state.analog[i as usize] = [green, red, blue];
        }
        self.palette.state.index = match self.machine_model {
            MachineModel::PC9801F | MachineModel::PC9801VM => 0x08,
            MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => 0x0F,
        };
        self.palette.state.digital = [0x37, 0x15, 0x26, 0x04];

        // GRCG: mode and tile registers. PC-9801F has no GRCG hardware.
        match self.machine_model {
            MachineModel::PC9801F => {}
            MachineModel::PC9801VM => {
                self.grcg.state.mode = 0x0F;
            }
            MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => {
                self.grcg.state.mode = 0x00;
                self.grcg.state.tile = [51, 85, 0, 0];
            }
        }

        // CGROM: last accessed character state.
        self.cgrom.state.code = match self.machine_model {
            MachineModel::PC9801F => 0x5F56,
            MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => 0x7F57,
        };
        self.cgrom.state.line = 0x0F;
        self.cgrom.state.lr = 0x0000;

        // CRTC line counter registers.
        self.crtc.state.regs = [0x00, 0x0F, 0x10, 0x00, 0x00, 0x00];

        // Printer port C.
        match self.machine_model {
            MachineModel::PC9801F | MachineModel::PC9801VM => {
                self.printer.state.port_c = 0x80;
            }
            MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => {}
        }

        // Sound ROM: install stub if no full ROM was loaded.
        if !self.memory.has_sound_rom() {
            self.memory.load_sound_rom(None);
        }

        // Beeper: muted at boot (port C bit 3 = 1 means buzzer off).
        self.beeper.state.buzzer_enabled = (self.system_ppi.state.port_c & 0x08) == 0;
        // For PIT-driven beepers (VM and later) the audible tone follows PIT ch1.
        // For Fixed-frequency beepers (PC-9801F) PIT ch1 is the memory-refresh
        // generator and must not affect the tone, so we keep the boot reload
        // computed by Beeper::new from BeeperKind::Fixed { hz }.
        match self.machine_model.beeper_kind() {
            BeeperKind::PitDriven => {
                self.beeper.state.pit_reload = self.pit.state.channels[1].value;
                self.beeper.state.pit_last_load_cycle = self.pit.state.channels[1].last_load_cycle;
            }
            BeeperKind::Fixed { hz } if hz > 0 => {
                self.beeper.state.pit_reload = (self.clocks.pit_clock_hz / hz) as u16;
                self.beeper.state.pit_last_load_cycle = 0;
            }
            BeeperKind::Fixed { .. } => {
                self.beeper.state.pit_reload = 0;
                self.beeper.state.pit_last_load_cycle = 0;
            }
        }

        // Mouse PPI: reset to defaults.
        self.mouse_ppi.state.mode = 0x93;
        self.mouse_ppi.state.port_a = 0x00;
        self.mouse_ppi.state.port_b = 0x00;
        self.mouse_ppi.state.port_c = match self.machine_model {
            MachineModel::PC9801F | MachineModel::PC9801VM => 0xF0,
            MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => 0x00,
        };
        self.mouse_ppi.state.accumulator_x = 0;
        self.mouse_ppi.state.accumulator_y = 0;
        self.mouse_ppi.state.remaining_x = 0;
        self.mouse_ppi.state.remaining_y = 0;
        self.mouse_ppi.state.sample_x = 0;
        self.mouse_ppi.state.sample_y = 0;
        self.mouse_ppi.state.latch_x = 0;
        self.mouse_ppi.state.latch_y = 0;
        self.mouse_ppi.state.buttons = 0xE0;
        self.mouse_ppi.state.last_interpolation_cycle = 0;
        self.mouse_ppi.state.mouse_connected = true;
        self.mouse_timer_setting = MOUSE_TIMER_DEFAULT_SETTING;
        self.scheduler.cancel(Event98::MouseTimer);
        self.clear_pic_irq(MOUSE_TIMER_IRQ_LINE);

        // FDC 1MB + 640K: post-Recalibrate state.
        self.floppy.initialize_boot_state(self.clocks.pit_clock_hz);

        // Text VRAM: character area filled with space words (0x0020).
        for i in (0..0x2000).step_by(2) {
            self.memory.state.text_vram[i] = 0x20;
            self.memory.state.text_vram[i + 1] = 0x00;
        }
        // Text VRAM: attribute area filled with 0xE1 at even bytes.
        for i in (0x2000..0x3FC0).step_by(2) {
            self.memory.state.text_vram[i] = 0xE1;
        }

        // Memory switches at text VRAM (stride 4).
        // MSW3 bits 0-2 encode conventional memory in 128 KB units above
        // the base 128 KB: 0x04 = 4 * 128 KB + 128 KB = 640 KB.
        let year_bcd = self.host_date_time_source.now().to_bcd_bytes()[0];
        let msw_values: [u8; 8] = [0x48, 0x05, 0x04, 0x00, 0x01, 0x00, 0x00, year_bcd];
        let msw_offsets: [usize; 8] = [
            0x3FE2, 0x3FE6, 0x3FEA, 0x3FEE, 0x3FF2, 0x3FF6, 0x3FFA, 0x3FFE,
        ];
        for (&offset, &value) in msw_offsets.iter().zip(msw_values.iter()) {
            self.memory.state.text_vram[offset] = value;
        }

        // RAM trampoline at 0x04F8.
        self.memory.state.ram[0x04F8..0x04FE]
            .copy_from_slice(&[0xEE, 0xEA, 0x00, 0x00, 0xFF, 0xFF]);

        // BDA fields derived from machine model and memory switch configuration.
        // Must run after memory switch initialization above.
        self.populate_bda();

        // Apply display line/mode2 clock selection and refresh VSYNC timing.
        self.apply_gdc_dot_clock();
        self.reschedule_gdc_events();

        // Registers of the 386/486 machines (RS and later).
        match self.machine_model {
            MachineModel::PC9801F | MachineModel::PC9801VM | MachineModel::PC9801VX => {}
            MachineModel::PC9801RS
            | MachineModel::PC9801RA
            | MachineModel::PC9821AS
            | MachineModel::PC9821AP => {
                self.vram_ems_bank = 0x20;
                self.protected_memory_max = 0xE0;
                self.memory.copy_rom_to_shadow_ram();
                self.memory.set_shadow_control(0x46);
            }
        }

        // Ensure E-plane mapping is consistent with display mode.
        self.update_plane_e_mapping();
    }
}

impl Pc9801Bus<common::NoTrace> {
    /// Creates an untraced bus configured for a machine model and CPU mode.
    pub fn new(machine_model: MachineModel, cpu_mode: CpuMode, sample_rate: u32) -> Self {
        Self::new_with_trace_sink(machine_model, cpu_mode, sample_rate, common::NoTrace)
    }
}
