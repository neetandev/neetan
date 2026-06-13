use device::upd7220_gdc::{DISPLAY_MODE_GRAPHICS, DISPLAY_MODE_MIXED};

use super::read_ram_u16;

macro_rules! check {
    ($f:ident, $left:expr, $right:expr, $label:expr) => {
        let left = $left;
        let right = $right;
        if left != right {
            $f.push(format!("{}: left={:?}, right={:?}", $label, left, right));
        }
    };
}

macro_rules! check_true {
    ($f:ident, $val:expr, $label:expr) => {
        if !$val {
            $f.push(format!("{}: expected true, got false", $label));
        }
    };
}

macro_rules! check_false {
    ($f:ident, $val:expr, $label:expr) => {
        if $val {
            $f.push(format!("{}: expected false, got true", $label));
        }
    };
}

fn report_failures(failures: &[String], machine: &str) {
    if !failures.is_empty() {
        let msg = failures.join("\n  ");
        panic!(
            "{machine}: {n} assertion(s) failed:\n  {msg}",
            n = failures.len()
        );
    }
}

#[test]
fn post_bios_state_vm() {
    let mut machine = super::create_machine_vm();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();
    let mut f: Vec<String> = Vec::new();

    // === PIC ===
    check!(
        f,
        state.pic.chips[0].icw,
        [0x11, 0x08, 0x80, 0x1D],
        "Master PIC ICW"
    );
    check!(f, state.pic.chips[0].imr, 0x3D, "Master PIC IMR");
    check!(f, state.pic.chips[0].isr, 0x00, "Master PIC ISR");
    check!(f, state.pic.chips[0].ocw3, 0x00, "Master PIC OCW3");
    check!(f, state.pic.chips[0].pry, 0x00, "Master PIC priority");
    check!(
        f,
        state.pic.chips[0].write_icw,
        0x00,
        "Master PIC write_icw"
    );
    check!(
        f,
        state.pic.chips[1].icw,
        [0x11, 0x10, 0x07, 0x09],
        "Slave PIC ICW"
    );
    check!(f, state.pic.chips[1].imr, 0xF7, "Slave PIC IMR");
    check!(f, state.pic.chips[1].isr, 0x00, "Slave PIC ISR");
    check!(f, state.pic.chips[1].ocw3, 0x0B, "Slave PIC OCW3");
    check!(f, state.pic.chips[1].pry, 0x00, "Slave PIC priority");
    check!(f, state.pic.chips[1].write_icw, 0x00, "Slave PIC write_icw");

    // === PIT ===
    check!(f, state.pit.channels[0].ctrl, 0x30, "PIT ch0 ctrl");
    check!(f, state.pit.channels[0].flag, 0x00, "PIT ch0 flag");
    check!(f, state.pit.channels[0].value, 0x0000, "PIT ch0 value");
    check!(f, state.pit.channels[1].ctrl, 0x36, "PIT ch1 ctrl");
    check!(f, state.pit.channels[1].flag, 0x00, "PIT ch1 flag");
    check!(f, state.pit.channels[1].value, 0x04CD, "PIT ch1 value");
    check!(f, state.pit.channels[2].ctrl, 0x76, "PIT ch2 ctrl");
    check!(f, state.pit.channels[2].flag, 0x00, "PIT ch2 flag");
    check!(f, state.pit.channels[2].value, 0x0000, "PIT ch2 value");

    // === Clocks ===
    check!(f, state.clocks.cpu_clock_hz, 10_000_000, "CPU clock");
    check!(f, state.clocks.pit_clock_hz, 2_457_600, "PIT clock");

    // === GDC Master (text) ===
    check!(f, state.gdc_master.status, 0x04, "Master GDC status");
    check_false!(
        f,
        state.gdc_master.display_enabled,
        "VM Master GDC display disabled"
    );
    check_true!(f, state.gdc_master.master_mode, "Master GDC master mode");
    check_false!(f, state.gdc_master.is_slave, "Master GDC is not slave");
    check!(f, state.gdc_master.pitch, 80, "Master GDC pitch");
    check!(f, state.gdc_master.mask, 0, "Master GDC mask");
    check!(f, state.gdc_master.ead, 0, "Master GDC EAD");
    check!(f, state.gdc_master.dad, 0, "Master GDC DAD");
    check!(f, state.gdc_master.pattern, 0, "Master GDC pattern");
    check!(
        f,
        state.gdc_master.display_mode,
        DISPLAY_MODE_MIXED,
        "Master GDC display mode"
    );
    check!(
        f,
        state.gdc_master.interlace_mode,
        0,
        "Master GDC interlace"
    );
    check_true!(
        f,
        state.gdc_master.draw_on_retrace,
        "Master GDC draw on retrace"
    );
    check!(f, state.gdc_master.aw, 80, "Master GDC AW");
    check!(f, state.gdc_master.hs, 8, "Master GDC HS");
    check!(f, state.gdc_master.vs, 8, "Master GDC VS");
    check!(f, state.gdc_master.hfp, 10, "Master GDC HFP");
    check!(f, state.gdc_master.hbp, 8, "Master GDC HBP");
    check!(f, state.gdc_master.vfp, 7, "Master GDC VFP");
    check!(f, state.gdc_master.vbp, 25, "Master GDC VBP");
    check!(f, state.gdc_master.al, 400, "Master GDC AL");
    check_false!(
        f,
        state.gdc_master.cursor_display,
        "Master GDC cursor display"
    );
    check_false!(f, state.gdc_master.cursor_blink, "Master GDC cursor blink");
    check!(f, state.gdc_master.cursor_top, 0, "Master GDC cursor top");
    check!(
        f,
        state.gdc_master.cursor_bottom,
        15,
        "Master GDC cursor bottom"
    );
    check!(
        f,
        state.gdc_master.cursor_blink_rate,
        12,
        "Master GDC blink rate"
    );
    check!(
        f,
        state.gdc_master.lines_per_row,
        16,
        "Master GDC lines per row"
    );
    check!(f, state.gdc_master.zoom_gchr, 0, "Master GDC zoom gchr");
    check!(
        f,
        state.gdc_master.zoom_display,
        0,
        "Master GDC zoom display"
    );
    check!(
        f,
        state.gdc_master.scroll[0].start_address,
        0,
        "Master GDC scroll[0] start"
    );
    check!(
        f,
        state.gdc_master.scroll[0].line_count,
        0x1FF,
        "Master GDC scroll[0] lines"
    );
    check_false!(
        f,
        state.gdc_master.scroll[0].im,
        "Master GDC scroll[0] character mode"
    );
    check_false!(
        f,
        state.gdc_master.scroll[0].wd,
        "Master GDC scroll[0] no wide"
    );
    check_false!(
        f,
        state.gdc_master.rdat_pending,
        "Master GDC no rdat pending"
    );
    check_false!(f, state.gdc_master.dma_active, "Master GDC no DMA active");
    check!(f, state.gdc_master.fifo.count, 0, "Master GDC FIFO empty");

    // === GDC Slave (graphics) ===
    check!(f, state.gdc_slave.status, 0x04, "Slave GDC status");
    check_false!(
        f,
        state.gdc_slave.display_enabled,
        "Slave GDC display disabled"
    );
    check_false!(f, state.gdc_slave.master_mode, "Slave GDC not master mode");
    check_true!(f, state.gdc_slave.is_slave, "Slave GDC is slave");
    check!(f, state.gdc_slave.pitch, 40, "Slave GDC pitch");
    check!(f, state.gdc_slave.mask, 0, "Slave GDC mask");
    check!(
        f,
        state.gdc_slave.display_mode,
        DISPLAY_MODE_GRAPHICS,
        "Slave GDC display mode"
    );
    check_true!(
        f,
        state.gdc_slave.draw_on_retrace,
        "Slave GDC draw on retrace"
    );
    check!(f, state.gdc_slave.aw, 40, "Slave GDC AW");
    check!(f, state.gdc_slave.hs, 4, "Slave GDC HS");
    check!(f, state.gdc_slave.vs, 8, "Slave GDC VS");
    check!(f, state.gdc_slave.hfp, 5, "Slave GDC HFP");
    check!(f, state.gdc_slave.hbp, 4, "Slave GDC HBP");
    check!(f, state.gdc_slave.vfp, 7, "Slave GDC VFP");
    check!(f, state.gdc_slave.vbp, 25, "Slave GDC VBP");
    check!(f, state.gdc_slave.al, 400, "Slave GDC AL");
    check!(
        f,
        state.gdc_slave.lines_per_row,
        2,
        "Slave GDC lines per row"
    );
    check!(
        f,
        state.gdc_slave.scroll[0].start_address,
        0,
        "Slave GDC scroll[0] start"
    );
    check!(
        f,
        state.gdc_slave.scroll[0].line_count,
        0x1FF,
        "Slave GDC scroll[0] lines"
    );
    check_false!(
        f,
        state.gdc_slave.scroll[0].im,
        "Slave GDC scroll[0] bitmap mode"
    );
    check!(f, state.gdc_slave.fifo.count, 0, "Slave GDC FIFO empty");

    // === NMI / A20 / misc ===
    check_true!(f, state.nmi_enabled, "NMI enabled");
    check_false!(f, state.a20_enabled, "A20 disabled");
    check!(f, state.fdc_media, 3, "FDC media");
    check!(f, state.vram_ems_bank, 0, "VRAM EMS bank");
    check!(f, state.ram_window, 8, "RAM window");
    check!(f, state.mouse_timer_setting, 0, "Mouse timer setting");
    check!(f, state.hole_15m_control, 0, "15M hole control");
    check!(f, state.protected_memory_max, 0, "Protected memory max");
    check_false!(f, state.b_bank_ems, "B-bank EMS disabled");
    check!(f, state.tram_wait, 1, "TRAM wait");
    check!(f, state.vram_wait, 6, "VRAM wait");
    check!(f, state.grcg_wait, 8, "GRCG wait");

    // === Keyboard ===
    check!(f, state.keyboard.mode, 0x5E, "KB mode");
    check!(f, state.keyboard.command, 0x16, "KB command");
    check!(f, state.keyboard.status, 0x00, "KB status");
    check!(f, state.keyboard.data, 0xFF, "KB data");
    check_false!(f, state.keyboard.rx_ready, "KB rx not ready");
    check_true!(f, state.keyboard.rx_fifo.is_empty(), "KB FIFO empty");
    check_false!(f, state.keyboard.expect_mode, "KB not expecting mode");

    // === Serial ===
    check!(f, state.serial.mode, 0x02, "Serial mode");
    check!(f, state.serial.command, 0x40, "Serial command");
    check!(f, state.serial.status, 0x00, "Serial status");
    check!(f, state.serial.data, 0xFF, "Serial data");
    check_true!(f, state.serial.expect_mode, "Serial expecting mode");

    // === FDC 1MB ===
    check!(f, state.fdc_1mb.status, 0x80, "FDC 1MB MSR");
    check!(f, state.fdc_1mb.control, 0x18, "FDC 1MB control");
    check_false!(f, state.fdc_1mb.interrupt_pending, "FDC 1MB no interrupt");
    check!(
        f,
        state.fdc_1mb.drive_cylinder,
        [0, 0, 0, 0],
        "FDC 1MB cylinders"
    );
    check!(f, state.fdc_1mb.srt, 11, "FDC 1MB SRT");
    check!(f, state.fdc_1mb.hut, 10, "FDC 1MB HUT");
    check!(f, state.fdc_1mb.hlt, 25, "FDC 1MB HLT");
    check_true!(f, state.fdc_1mb.tc, "FDC 1MB TC");
    check!(f, state.fdc_1mb.drive_equipped, 3, "FDC 1MB equipped");

    // === FDC 640K ===
    check!(f, state.fdc_640k.status, 0x80, "FDC 640K MSR");
    check!(f, state.fdc_640k.control, 0x48, "FDC 640K control");
    check_false!(f, state.fdc_640k.interrupt_pending, "FDC 640K no interrupt");
    check!(
        f,
        state.fdc_640k.drive_cylinder,
        [0, 0, 0, 0],
        "FDC 640K cylinders"
    );
    check!(f, state.fdc_640k.drive_equipped, 3, "FDC 640K equipped");

    // === System PPI ===
    check!(f, state.system_ppi.port_b, 0x82, "System PPI port B");
    check!(f, state.system_ppi.port_c, 0x18, "System PPI port C");
    check!(f, state.system_ppi.dip_switch_2, 0xF3, "DIP switch 2");
    check!(
        f,
        state.system_ppi.rs232c_modem_signals,
        0xE0,
        "RS-232C modem signals"
    );
    check_true!(f, state.system_ppi.crtt, "CRTT flag");

    // === Printer ===
    check!(f, state.printer.data, 0x00, "Printer data");
    check!(f, state.printer.port_c, 0x80, "Printer port C");
    check_false!(f, state.printer.attached, "Printer not attached");

    // === CGROM ===
    check!(f, state.cgrom.code, 0x7F57, "CGROM code");
    check!(f, state.cgrom.line, 15, "CGROM line");
    check!(f, state.cgrom.lr, 0, "CGROM lr");
    check_false!(f, state.cgrom.cg_ram, "CGROM CG ROM mode");

    // === GRCG ===
    check!(f, state.grcg.mode, 0x0F, "GRCG mode");
    check!(f, state.grcg.tile_index, 0, "GRCG tile index");
    check!(f, state.grcg.tile, [0, 0, 0, 0], "GRCG tile");
    check!(f, state.grcg.chip, 1, "GRCG chip (v1)");
    check!(f, state.grcg.gdc_with_grcg, 0, "GRCG GDC routing");

    // === EGC ===
    check!(f, state.egc.access, 0xFFF0, "EGC access");
    check!(f, state.egc.fgbg, 0x00FF, "EGC fgbg");
    check!(f, state.egc.ope, 0, "EGC ope");
    check!(f, state.egc.fg, 0, "EGC fg");
    check!(f, state.egc.mask, 0xFFFF, "EGC mask");
    check!(f, state.egc.bg, 0, "EGC bg");
    check!(f, state.egc.sft, 0, "EGC sft");
    check!(f, state.egc.leng, 0x000F, "EGC leng");

    // === Display control ===
    check!(f, state.display_control.video_mode, 0x99, "Video mode");
    check!(f, state.display_control.mode2, 0x0100, "Mode2");
    check_false!(
        f,
        state.display_control.vsync_irq_enabled,
        "VSYNC IRQ disabled"
    );
    check!(f, state.display_control.border_color, 0, "Border color");
    check!(
        f,
        state.display_control.display_line_count,
        1,
        "Display line count"
    );
    check!(f, state.display_control.display_page, 0, "Display page");
    check!(f, state.display_control.access_page, 0, "Access page");

    // === CRTC ===
    check!(f, state.crtc.regs, [0, 15, 16, 0, 0, 0], "CRTC regs");

    // === Palette ===
    check!(f, state.palette.index, 8, "Palette index");
    check!(
        f,
        state.palette.digital,
        [0x37, 0x15, 0x26, 0x04],
        "Digital palette"
    );
    check!(f, state.palette.analog[0], [0, 0, 0], "Analog palette 0");
    check!(f, state.palette.analog[1], [0, 0, 10], "Analog palette 1");
    check!(f, state.palette.analog[7], [10, 10, 10], "Analog palette 7");
    check!(f, state.palette.analog[8], [7, 7, 7], "Analog palette 8");
    check!(
        f,
        state.palette.analog[15],
        [15, 15, 15],
        "Analog palette 15"
    );

    // === Soundboards ===
    check_true!(f, state.soundboard_26k.is_none(), "No 26K soundboard");
    check_true!(f, state.soundboard_86.is_none(), "No 86 soundboard");

    // === Scheduler: PIT Timer0 must be scheduled ===
    check_true!(
        f,
        state.scheduler.fire_cycles[common::EventKind::PitTimer0 as usize].is_some(),
        "PIT Timer0 event scheduled"
    );

    // === Beeper ===
    check_false!(f, state.beeper.buzzer_enabled, "Beeper disabled");
    check!(f, state.beeper.pit_reload, 1229, "Beeper PIT reload");

    // === Mouse PPI ===
    check!(f, state.mouse_ppi.mode, 0x93, "Mouse PPI mode");
    check!(f, state.mouse_ppi.port_a, 0x00, "Mouse PPI port A");
    check!(f, state.mouse_ppi.port_b, 0x00, "Mouse PPI port B");
    check!(f, state.mouse_ppi.port_c, 0xF0, "Mouse PPI port C");
    check!(f, state.mouse_ppi.buttons, 0xE0, "Mouse buttons");
    check_true!(f, state.mouse_ppi.mouse_connected, "Mouse connected");

    // === Memory: BDA fields ===
    check!(f, state.memory.ram[0x0493], 0xFF, "BDA F2HD_MODE");
    check!(f, state.memory.ram[0x053B], 0x0F, "BDA CRT_RASTER");
    check!(f, state.memory.ram[0x053C], 0x84, "BDA CRT_STS_FLAG");
    check!(f, state.memory.ram[0x054C], 0x4E, "BDA PRXCRT");
    check!(f, state.memory.ram[0x054D], 0x00, "BDA PRXDUPD");
    check!(f, state.memory.ram[0x055C], 0x03, "BDA DISK_EQUIP");
    check!(f, state.memory.ram[0x0584], 0x90, "BDA BOOT_DEVICE");
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x0522),
        0x0B28,
        "KB shift table pointer"
    );
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x0524),
        0x0502,
        "KB head pointer"
    );
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x0526),
        0x0502,
        "KB tail pointer"
    );
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x0528),
        0x0000,
        "KB count"
    );
    check!(f, state.memory.ram[0x05CA], 0xFF, "BDA F2DD_MODE");
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x05CC),
        0x1ADC,
        "F2DD_POINTER offset"
    );
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x05CE),
        0xFD80,
        "F2DD_POINTER segment"
    );
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x05F8),
        0x1AB4,
        "F2HD_POINTER offset"
    );
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x05FA),
        0xFD80,
        "F2HD_POINTER segment"
    );

    // === Memory: Conventional RAM zeroed ===
    check_true!(
        f,
        state.memory.ram[0x1000..0x7C00].iter().all(|&b| b == 0)
            && state.memory.ram[0x7C06..0x1FC00].iter().all(|&b| b == 0),
        "Conventional RAM zeroed (excluding IRET frame at 0x7C00)"
    );
    check!(
        f,
        state.memory.ram[0x1FC00],
        0xFA,
        "Boot sector byte 0 (CLI)"
    );
    check!(
        f,
        state.memory.ram[0x1FC01],
        0xF4,
        "Boot sector byte 1 (HLT)"
    );
    check_true!(
        f,
        state.memory.ram[0x1FC02..0xA0000].iter().all(|&b| b == 0),
        "RAM above boot sector zeroed"
    );

    // === Graphics VRAM zeroed ===
    check_true!(
        f,
        state.memory.graphics_vram.iter().all(|&b| b == 0),
        "Graphics VRAM zeroed"
    );
    check_true!(
        f,
        state.memory.e_plane_vram.iter().all(|&b| b == 0),
        "E-plane VRAM zeroed"
    );

    // === Text VRAM attributes ===
    for i in (0x2000..0x3FC0).step_by(2) {
        let attr = state.memory.text_vram[i];
        if attr != 0xE1 && attr != 0xE5 {
            f.push(format!(
                "Text VRAM attr at {i:#06X}: expected 0xE1 or 0xE5, got {attr:#04X}"
            ));
            break;
        }
    }

    report_failures(&f, "VM");
}

#[test]
fn post_bios_state_f() {
    let mut machine = super::create_machine_f();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();
    let mut f: Vec<String> = Vec::new();

    // === PIC ===
    check!(
        f,
        state.pic.chips[0].icw,
        [0x11, 0x08, 0x80, 0x1D],
        "Master PIC ICW"
    );
    check!(f, state.pic.chips[0].imr, 0x3D, "Master PIC IMR");
    check!(f, state.pic.chips[0].isr, 0x00, "Master PIC ISR");
    check!(f, state.pic.chips[0].ocw3, 0x00, "Master PIC OCW3");
    check!(f, state.pic.chips[0].pry, 0x00, "Master PIC priority");
    check!(
        f,
        state.pic.chips[0].write_icw,
        0x00,
        "Master PIC write_icw"
    );
    check!(
        f,
        state.pic.chips[1].icw,
        [0x11, 0x10, 0x07, 0x09],
        "Slave PIC ICW"
    );
    check!(f, state.pic.chips[1].imr, 0xFD, "Slave PIC IMR");
    check!(f, state.pic.chips[1].isr, 0x00, "Slave PIC ISR");
    check!(f, state.pic.chips[1].ocw3, 0x0B, "Slave PIC OCW3");
    check!(f, state.pic.chips[1].pry, 0x00, "Slave PIC priority");
    check!(f, state.pic.chips[1].write_icw, 0x00, "Slave PIC write_icw");

    // === PIT ===
    check!(f, state.pit.channels[0].ctrl, 0x30, "PIT ch0 ctrl");
    check!(f, state.pit.channels[0].flag, 0x00, "PIT ch0 flag");
    check!(f, state.pit.channels[0].value, 0x0000, "PIT ch0 value");
    check!(f, state.pit.channels[1].ctrl, 0x14, "PIT ch1 ctrl");
    check!(f, state.pit.channels[1].flag, 0x00, "PIT ch1 flag");
    check!(f, state.pit.channels[1].value, 0x0039, "PIT ch1 value");
    check!(f, state.pit.channels[2].ctrl, 0x76, "PIT ch2 ctrl");
    check!(f, state.pit.channels[2].flag, 0x00, "PIT ch2 flag");
    check!(f, state.pit.channels[2].value, 0x0000, "PIT ch2 value");

    // === Clocks ===
    check!(f, state.clocks.cpu_clock_hz, 8_000_000, "CPU clock");
    check!(f, state.clocks.pit_clock_hz, 1_996_800, "PIT clock");

    // === GDC Master (text) ===
    check!(f, state.gdc_master.status, 0x04, "Master GDC status");
    check_false!(
        f,
        state.gdc_master.display_enabled,
        "F Master GDC display disabled"
    );
    check_true!(f, state.gdc_master.master_mode, "Master GDC master mode");
    check_false!(f, state.gdc_master.is_slave, "Master GDC is not slave");
    check!(f, state.gdc_master.pitch, 80, "Master GDC pitch");
    check!(f, state.gdc_master.al, 400, "Master GDC AL");
    check!(
        f,
        state.gdc_master.lines_per_row,
        16,
        "Master GDC lines per row"
    );

    // === GDC Slave (graphics) ===
    check!(f, state.gdc_slave.status, 0x44, "Slave GDC status");
    check_false!(
        f,
        state.gdc_slave.display_enabled,
        "Slave GDC display disabled"
    );
    check_true!(f, state.gdc_slave.is_slave, "Slave GDC is slave");
    check!(f, state.gdc_slave.pitch, 40, "Slave GDC pitch");

    // === NMI / A20 / misc ===
    check_true!(f, state.nmi_enabled, "NMI enabled");
    check_false!(f, state.a20_enabled, "A20 disabled");
    check!(f, state.fdc_media, 3, "FDC media");

    // === Keyboard ===
    check!(f, state.keyboard.mode, 0x5E, "KB mode");
    check!(f, state.keyboard.command, 0x16, "KB command");
    check!(f, state.keyboard.data, 0xFF, "KB data");
    check_false!(f, state.keyboard.rx_ready, "KB rx not ready");
    check_true!(f, state.keyboard.rx_fifo.is_empty(), "KB FIFO empty");
    check_false!(f, state.keyboard.expect_mode, "KB not expecting mode");

    // === Serial ===
    check!(f, state.serial.mode, 0x02, "Serial mode");
    check!(f, state.serial.command, 0x40, "Serial command");
    check!(f, state.serial.status, 0x00, "Serial status");
    check!(f, state.serial.data, 0xFF, "Serial data");
    check_true!(f, state.serial.expect_mode, "Serial expecting mode");

    // === FDC 1MB ===
    check!(f, state.fdc_1mb.status, 0x80, "FDC 1MB MSR");
    check!(f, state.fdc_1mb.control, 0x18, "FDC 1MB control");
    check_false!(f, state.fdc_1mb.interrupt_pending, "FDC 1MB no interrupt");
    check!(f, state.fdc_1mb.srt, 12, "FDC 1MB SRT");
    check!(f, state.fdc_1mb.hut, 15, "FDC 1MB HUT");
    check!(f, state.fdc_1mb.hlt, 18, "FDC 1MB HLT");
    check_true!(f, state.fdc_1mb.tc, "FDC 1MB TC");
    check!(f, state.fdc_1mb.drive_equipped, 3, "FDC 1MB equipped");

    // === FDC 640K ===
    check!(f, state.fdc_640k.status, 0x80, "FDC 640K MSR");
    check!(f, state.fdc_640k.control, 0x48, "FDC 640K control");
    check_false!(f, state.fdc_640k.interrupt_pending, "FDC 640K no interrupt");
    check!(f, state.fdc_640k.drive_equipped, 3, "FDC 640K equipped");

    // === System PPI ===
    check!(f, state.system_ppi.port_b, 0xA0, "System PPI port B");
    check!(f, state.system_ppi.port_c, 0x18, "System PPI port C");

    // === CGROM ===
    check!(f, state.cgrom.code, 0x5F56, "CGROM code");
    check_false!(f, state.cgrom.cg_ram, "CGROM CG ROM mode");

    // === Display control ===
    check!(f, state.display_control.video_mode, 0x99, "Video mode");
    check!(f, state.display_control.mode2, 0x0100, "Mode2");
    check_false!(
        f,
        state.display_control.vsync_irq_enabled,
        "VSYNC IRQ disabled"
    );

    // === Soundboards ===
    check_true!(f, state.soundboard_26k.is_none(), "No 26K soundboard");
    check_true!(f, state.soundboard_86.is_none(), "No 86 soundboard");

    // === Scheduler: PIT Timer0 must be scheduled ===
    check_true!(
        f,
        state.scheduler.fire_cycles[common::EventKind::PitTimer0 as usize].is_some(),
        "PIT Timer0 event scheduled"
    );

    // === Beeper ===
    // PC-9801F has a fixed-frequency 2400 Hz hardware beeper (BeeperKind::Fixed).
    // PIT ch1 is the memory-refresh generator and writes to it must not affect
    // the audible tone, so the beeper's `pit_reload` is pinned at
    // pit_clock_hz / 2400 = 1996800 / 2400 = 832 regardless of what the BIOS
    // programs into PIT ch1 for memory refresh.
    check_false!(f, state.beeper.buzzer_enabled, "Beeper disabled");
    check!(f, state.beeper.pit_reload, 832, "Beeper PIT reload");

    // === Mouse PPI ===
    check!(f, state.mouse_ppi.mode, 0x93, "Mouse PPI mode");
    check!(f, state.mouse_ppi.port_a, 0x00, "Mouse PPI port A");
    check!(f, state.mouse_ppi.port_b, 0x00, "Mouse PPI port B");
    check!(f, state.mouse_ppi.port_c, 0xF0, "Mouse PPI port C");
    check!(f, state.mouse_ppi.buttons, 0xE0, "Mouse buttons");

    // === Memory: BDA fields ===
    check!(f, state.memory.ram[0x0493], 0x00, "BDA F2HD_MODE");
    check!(f, state.memory.ram[0x053C], 0x84, "BDA CRT_STS_FLAG");
    check!(f, state.memory.ram[0x055C], 0x00, "BDA DISK_EQUIP");
    check!(f, state.memory.ram[0x0584], 0x90, "BDA BOOT_DEVICE");
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x0522),
        0x0A58,
        "KB shift table pointer"
    );
    check!(f, state.memory.ram[0x05CA], 0x00, "BDA F2DD_MODE");

    // === Memory: HLE booted the inserted 2DD halt disk ===
    check!(f, state.memory.ram[0x1FC00], 0xFA, "Boot sector byte 0");
    check!(f, state.memory.ram[0x1FC01], 0xF4, "Boot sector byte 1");

    // === Graphics VRAM zeroed ===
    check_true!(
        f,
        state.memory.graphics_vram.iter().all(|&b| b == 0),
        "Graphics VRAM zeroed"
    );

    report_failures(&f, "F");
}

#[test]
fn post_bios_state_vx() {
    let mut machine = super::create_machine_vx();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();
    let mut f: Vec<String> = Vec::new();

    // === PIC ===
    check!(
        f,
        state.pic.chips[0].icw,
        [0x11, 0x08, 0x80, 0x1D],
        "Master PIC ICW"
    );
    check!(f, state.pic.chips[0].imr, 0x3D, "Master PIC IMR");
    check!(f, state.pic.chips[0].isr, 0x00, "Master PIC ISR");
    check!(
        f,
        state.pic.chips[1].icw,
        [0x11, 0x10, 0x07, 0x09],
        "Slave PIC ICW"
    );
    check!(f, state.pic.chips[1].imr, 0xF7, "Slave PIC IMR");
    check!(f, state.pic.chips[1].isr, 0x00, "Slave PIC ISR");

    // === PIT ===
    check!(f, state.pit.channels[0].ctrl, 0x30, "PIT ch0 ctrl");
    check!(f, state.pit.channels[0].value, 0x0000, "PIT ch0 value");
    check!(f, state.pit.channels[1].ctrl, 0x36, "PIT ch1 ctrl");
    check!(f, state.pit.channels[1].value, 0x04CD, "PIT ch1 value");
    check!(f, state.pit.channels[2].ctrl, 0x76, "PIT ch2 ctrl");
    check!(f, state.pit.channels[2].value, 0x0000, "PIT ch2 value");

    // === Clocks ===
    check!(f, state.clocks.cpu_clock_hz, 10_000_000, "CPU clock");
    check!(f, state.clocks.pit_clock_hz, 2_457_600, "PIT clock");

    // === GDC Master ===
    check_true!(
        f,
        state.gdc_master.display_enabled,
        "VX Master GDC display enabled"
    );
    check!(f, state.gdc_master.pitch, 80, "Master GDC pitch");
    check!(f, state.gdc_master.al, 400, "Master GDC AL");
    check!(
        f,
        state.gdc_master.lines_per_row,
        16,
        "Master GDC lines per row"
    );
    check!(
        f,
        state.gdc_master.scroll[0].start_address,
        0,
        "Master GDC scroll[0] start"
    );
    check!(
        f,
        state.gdc_master.scroll[0].line_count,
        0x1FF,
        "Master GDC scroll[0] lines"
    );
    check_true!(
        f,
        state.gdc_master.draw_on_retrace,
        "Master GDC draw on retrace"
    );
    check!(f, state.gdc_master.aw, 80, "Master GDC AW");
    check!(f, state.gdc_master.hs, 8, "Master GDC HS");
    check!(f, state.gdc_master.vs, 8, "Master GDC VS");
    check!(f, state.gdc_master.hfp, 10, "Master GDC HFP");
    check!(f, state.gdc_master.hbp, 8, "Master GDC HBP");
    check!(f, state.gdc_master.vfp, 7, "Master GDC VFP");
    check!(f, state.gdc_master.vbp, 25, "Master GDC VBP");
    check!(f, state.gdc_master.cursor_top, 0, "Master GDC cursor top");
    check!(
        f,
        state.gdc_master.cursor_bottom,
        15,
        "Master GDC cursor bottom"
    );
    check!(
        f,
        state.gdc_master.cursor_blink_rate,
        12,
        "Master GDC blink rate"
    );
    check!(f, state.gdc_master.fifo.count, 0, "Master GDC FIFO empty");

    // === GDC Slave ===
    check_false!(
        f,
        state.gdc_slave.display_enabled,
        "Slave GDC display disabled"
    );
    check!(f, state.gdc_slave.pitch, 40, "Slave GDC pitch");
    check!(f, state.gdc_slave.mask, 1, "Slave GDC mask");
    check!(f, state.gdc_slave.al, 400, "Slave GDC AL");
    check!(
        f,
        state.gdc_slave.lines_per_row,
        2,
        "Slave GDC lines per row"
    );
    check_true!(
        f,
        state.gdc_slave.draw_on_retrace,
        "Slave GDC draw on retrace"
    );
    check!(f, state.gdc_slave.aw, 40, "Slave GDC AW");
    check!(f, state.gdc_slave.hs, 4, "Slave GDC HS");
    check!(f, state.gdc_slave.vs, 8, "Slave GDC VS");
    check!(f, state.gdc_slave.hfp, 5, "Slave GDC HFP");
    check!(f, state.gdc_slave.hbp, 4, "Slave GDC HBP");
    check!(f, state.gdc_slave.vfp, 7, "Slave GDC VFP");
    check!(f, state.gdc_slave.vbp, 25, "Slave GDC VBP");
    check_false!(
        f,
        state.gdc_slave.scroll[0].im,
        "Slave GDC scroll[0] bitmap mode"
    );
    check!(f, state.gdc_slave.fifo.count, 0, "Slave GDC FIFO empty");

    // === NMI / A20 / misc ===
    check_true!(f, state.nmi_enabled, "NMI enabled");
    check_false!(f, state.a20_enabled, "A20 disabled");
    check!(f, state.fdc_media, 3, "FDC media");
    check!(f, state.vram_ems_bank, 0, "VRAM EMS bank");
    check!(f, state.ram_window, 8, "RAM window");
    check_false!(f, state.b_bank_ems, "B-bank EMS disabled");
    check!(f, state.tram_wait, 1, "TRAM wait");
    check!(f, state.vram_wait, 6, "VRAM wait");
    check!(f, state.grcg_wait, 8, "GRCG wait");

    // === Keyboard ===
    check!(f, state.keyboard.mode, 0x5E, "KB mode");
    check!(f, state.keyboard.command, 0x16, "KB command");
    check!(f, state.keyboard.data, 0xFF, "KB data");
    check_false!(f, state.keyboard.rx_ready, "KB rx not ready");
    check_false!(f, state.keyboard.expect_mode, "KB not expecting mode");

    // === Serial ===
    check!(f, state.serial.mode, 0x02, "Serial mode");
    check!(f, state.serial.command, 0x40, "Serial command");
    check_true!(f, state.serial.expect_mode, "Serial expecting mode");

    // === FDC 1MB ===
    check!(f, state.fdc_1mb.status, 0x80, "FDC 1MB MSR");
    check!(f, state.fdc_1mb.control, 0x18, "FDC 1MB control");
    check!(f, state.fdc_1mb.srt, 11, "FDC 1MB SRT");
    check!(f, state.fdc_1mb.hut, 10, "FDC 1MB HUT");
    check!(f, state.fdc_1mb.hlt, 25, "FDC 1MB HLT");

    // === FDC 640K ===
    check!(f, state.fdc_640k.status, 0x80, "FDC 640K MSR");
    check!(f, state.fdc_640k.control, 0x48, "FDC 640K control");

    // === System PPI ===
    check!(f, state.system_ppi.port_b, 0x80, "System PPI port B");
    check!(f, state.system_ppi.port_c, 0xB8, "System PPI port C");
    check!(f, state.system_ppi.dip_switch_2, 0xF3, "DIP switch 2");

    // === CGROM ===
    check_true!(f, state.cgrom.cg_ram, "VX has CG RAM");

    // === GRCG ===
    check!(f, state.grcg.mode, 0x00, "GRCG mode");
    check!(f, state.grcg.tile, [51, 85, 0, 0], "GRCG tile");
    check!(f, state.grcg.chip, 3, "GRCG chip (EGC)");

    // === EGC ===
    check!(f, state.egc.access, 0xFFF0, "EGC access");
    check!(f, state.egc.fgbg, 0x00FF, "EGC fgbg");
    check!(f, state.egc.mask, 0xFFFF, "EGC mask");
    check!(f, state.egc.leng, 0x000F, "EGC leng");

    // === Display control ===
    check!(f, state.display_control.video_mode, 0x99, "Video mode");
    check!(f, state.display_control.mode2, 0x0100, "Mode2");
    check!(
        f,
        state.display_control.display_line_count,
        23,
        "Display line count"
    );

    // === Palette ===
    check!(f, state.palette.index, 15, "Palette index");
    check!(f, state.palette.analog[0], [0, 0, 0], "Analog palette 0");
    check!(f, state.palette.analog[1], [0, 0, 7], "Analog palette 1");
    check!(f, state.palette.analog[8], [4, 4, 4], "Analog palette 8");
    check!(
        f,
        state.palette.analog[15],
        [15, 15, 15],
        "Analog palette 15"
    );

    // === Scheduler: PIT Timer0 must be scheduled ===
    check_true!(
        f,
        state.scheduler.fire_cycles[common::EventKind::PitTimer0 as usize].is_some(),
        "PIT Timer0 event scheduled"
    );

    // === Beeper ===
    check_false!(f, state.beeper.buzzer_enabled, "Beeper disabled");
    check!(f, state.beeper.pit_reload, 1229, "Beeper PIT reload");

    // === Mouse PPI ===
    check!(f, state.mouse_ppi.mode, 0x93, "Mouse PPI mode");
    check!(f, state.mouse_ppi.port_c, 0x00, "Mouse PPI port C");
    check!(f, state.mouse_ppi.buttons, 0xE0, "Mouse buttons");

    // === Memory: BDA fields ===
    check!(f, state.memory.ram[0x0401], 0x20, "BDA EXPMMSZ");
    check!(f, state.memory.ram[0x0493], 0xFF, "BDA F2HD_MODE");
    check!(f, state.memory.ram[0x054C], 0x4E, "BDA PRXCRT");
    check!(f, state.memory.ram[0x054D], 0x50, "BDA PRXDUPD");
    check!(f, state.memory.ram[0x055C], 0x03, "BDA DISK_EQUIP");
    check!(f, state.memory.ram[0x0584], 0x90, "BDA BOOT_DEVICE");
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x0522),
        0x0B28,
        "KB shift table pointer"
    );
    check!(f, state.memory.ram[0x05CA], 0xFF, "BDA F2DD_MODE");
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x05CC),
        0x1ADC,
        "F2DD_POINTER offset"
    );
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x05CE),
        0xFD80,
        "F2DD_POINTER segment"
    );
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x05F8),
        0x1AB4,
        "F2HD_POINTER offset"
    );
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x05FA),
        0xFD80,
        "F2HD_POINTER segment"
    );

    // === Memory: Conventional RAM zeroed ===
    check_true!(
        f,
        state.memory.ram[0x1000..0x7C00].iter().all(|&b| b == 0)
            && state.memory.ram[0x7C06..0x1FC00].iter().all(|&b| b == 0),
        "Conventional RAM zeroed (excluding IRET frame at 0x7C00)"
    );
    check!(
        f,
        state.memory.ram[0x1FC00],
        0xFA,
        "Boot sector byte 0 (CLI)"
    );
    check!(
        f,
        state.memory.ram[0x1FC01],
        0xF4,
        "Boot sector byte 1 (HLT)"
    );

    // === Text VRAM attributes ===
    for i in (0x2000..0x3FC0).step_by(2) {
        let attr = state.memory.text_vram[i];
        if attr != 0xE1 && attr != 0xE5 {
            f.push(format!(
                "Text VRAM attr at {i:#06X}: expected 0xE1 or 0xE5, got {attr:#04X}"
            ));
            break;
        }
    }

    report_failures(&f, "VX");
}

#[test]
fn post_bios_state_ra() {
    let mut machine = super::create_machine_ra();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();
    let mut f: Vec<String> = Vec::new();

    // === Clocks ===
    check!(f, state.clocks.cpu_clock_hz, 20_000_000, "CPU clock");
    check!(f, state.clocks.pit_clock_hz, 1_996_800, "PIT clock");

    // === PIC ===
    check!(
        f,
        state.pic.chips[0].icw,
        [0x11, 0x08, 0x80, 0x1D],
        "Master PIC ICW"
    );
    check!(f, state.pic.chips[0].imr, 0x3D, "Master PIC IMR");
    check!(f, state.pic.chips[0].isr, 0x00, "Master PIC ISR");
    check!(f, state.pic.chips[0].ocw3, 0x0B, "Master PIC OCW3");
    check!(
        f,
        state.pic.chips[1].icw,
        [0x11, 0x10, 0x07, 0x09],
        "Slave PIC ICW"
    );
    check!(f, state.pic.chips[1].imr, 0xF7, "Slave PIC IMR");
    check!(f, state.pic.chips[1].isr, 0x00, "Slave PIC ISR");

    // === PIT ===
    check!(f, state.pit.channels[0].ctrl, 0x30, "PIT ch0 ctrl");
    check!(f, state.pit.channels[0].value, 0x0000, "PIT ch0 value");
    check!(f, state.pit.channels[1].ctrl, 0x36, "PIT ch1 ctrl");
    check!(f, state.pit.channels[1].value, 0x03E6, "PIT ch1 value");
    check!(f, state.pit.channels[2].ctrl, 0x76, "PIT ch2 ctrl");
    check!(f, state.pit.channels[2].value, 0x0000, "PIT ch2 value");

    // === GDC Master ===
    check_true!(
        f,
        state.gdc_master.display_enabled,
        "RA Master GDC display enabled"
    );
    check!(f, state.gdc_master.pitch, 80, "Master GDC pitch");
    check!(f, state.gdc_master.al, 400, "Master GDC AL");
    check!(
        f,
        state.gdc_master.lines_per_row,
        16,
        "Master GDC lines per row"
    );
    check!(
        f,
        state.gdc_master.scroll[0].start_address,
        0,
        "Master GDC scroll[0] start"
    );
    check!(
        f,
        state.gdc_master.scroll[0].line_count,
        0x1FF,
        "Master GDC scroll[0] lines"
    );
    check_true!(
        f,
        state.gdc_master.draw_on_retrace,
        "Master GDC draw on retrace"
    );
    check!(f, state.gdc_master.aw, 80, "Master GDC AW");
    check!(f, state.gdc_master.hs, 8, "Master GDC HS");
    check!(f, state.gdc_master.vs, 8, "Master GDC VS");
    check!(f, state.gdc_master.hfp, 10, "Master GDC HFP");
    check!(f, state.gdc_master.hbp, 8, "Master GDC HBP");
    check!(f, state.gdc_master.vfp, 7, "Master GDC VFP");
    check!(f, state.gdc_master.vbp, 25, "Master GDC VBP");
    check!(f, state.gdc_master.cursor_top, 0, "Master GDC cursor top");
    check!(
        f,
        state.gdc_master.cursor_bottom,
        15,
        "Master GDC cursor bottom"
    );
    check!(
        f,
        state.gdc_master.cursor_blink_rate,
        12,
        "Master GDC blink rate"
    );
    check!(f, state.gdc_master.fifo.count, 0, "Master GDC FIFO empty");

    // === GDC Slave ===
    check_false!(
        f,
        state.gdc_slave.display_enabled,
        "Slave GDC display disabled"
    );
    check!(f, state.gdc_slave.pitch, 40, "Slave GDC pitch");
    check!(f, state.gdc_slave.mask, 1, "Slave GDC mask");
    check!(f, state.gdc_slave.al, 400, "Slave GDC AL");
    check!(
        f,
        state.gdc_slave.lines_per_row,
        2,
        "Slave GDC lines per row"
    );
    check_true!(
        f,
        state.gdc_slave.draw_on_retrace,
        "Slave GDC draw on retrace"
    );
    check!(f, state.gdc_slave.aw, 40, "Slave GDC AW");
    check!(f, state.gdc_slave.hs, 4, "Slave GDC HS");
    check!(f, state.gdc_slave.vs, 8, "Slave GDC VS");
    check!(f, state.gdc_slave.hfp, 5, "Slave GDC HFP");
    check!(f, state.gdc_slave.hbp, 4, "Slave GDC HBP");
    check!(f, state.gdc_slave.vfp, 7, "Slave GDC VFP");
    check!(f, state.gdc_slave.vbp, 25, "Slave GDC VBP");
    check_false!(
        f,
        state.gdc_slave.scroll[0].im,
        "Slave GDC scroll[0] bitmap mode"
    );
    check!(f, state.gdc_slave.fifo.count, 0, "Slave GDC FIFO empty");

    // === NMI / A20 / misc ===
    check_true!(f, state.nmi_enabled, "NMI enabled");
    check_false!(f, state.a20_enabled, "A20 disabled");
    check!(f, state.fdc_media, 3, "FDC media");
    check!(f, state.vram_ems_bank, 0x20, "VRAM EMS bank");
    check!(f, state.ram_window, 8, "RAM window");
    check_true!(f, state.b_bank_ems, "RA has B-bank EMS");
    check!(f, state.protected_memory_max, 0xE0, "Protected memory max");
    check!(f, state.tram_wait, 1, "TRAM wait");
    check!(f, state.vram_wait, 6, "VRAM wait");
    check!(f, state.grcg_wait, 8, "GRCG wait");

    // === Keyboard ===
    check!(f, state.keyboard.mode, 0x5E, "KB mode");
    check!(f, state.keyboard.command, 0x16, "KB command");
    check!(f, state.keyboard.data, 0xFF, "KB data");
    check_false!(f, state.keyboard.rx_ready, "KB rx not ready");
    check_false!(f, state.keyboard.expect_mode, "KB not expecting mode");

    // === Serial ===
    check!(f, state.serial.mode, 0x02, "Serial mode");
    check!(f, state.serial.command, 0x40, "Serial command");
    check_true!(f, state.serial.expect_mode, "Serial expecting mode");

    // === System PPI ===
    check!(f, state.system_ppi.port_b, 0xA0, "System PPI port B");
    check!(f, state.system_ppi.port_c, 0xB8, "System PPI port C");

    // === FDC 1MB ===
    check!(f, state.fdc_1mb.status, 0x80, "FDC 1MB MSR");
    check!(f, state.fdc_1mb.srt, 12, "FDC 1MB SRT");
    check!(f, state.fdc_1mb.hut, 15, "FDC 1MB HUT");
    check!(f, state.fdc_1mb.hlt, 18, "FDC 1MB HLT");

    // === FDC 640K ===
    check!(f, state.fdc_640k.status, 0x80, "FDC 640K MSR");
    check!(f, state.fdc_640k.control, 0x48, "FDC 640K control");

    // === GRCG ===
    check!(f, state.grcg.mode, 0x00, "GRCG mode");
    check!(f, state.grcg.tile, [51, 85, 0, 0], "GRCG tile");
    check!(f, state.grcg.chip, 3, "GRCG chip (EGC)");

    // === Display control ===
    check!(f, state.display_control.video_mode, 0x99, "Video mode");
    check!(
        f,
        state.display_control.display_line_count,
        1,
        "Display line count"
    );

    // === Palette ===
    check!(f, state.palette.index, 15, "Palette index");
    check!(f, state.palette.analog[0], [0, 0, 0], "Analog palette 0");
    check!(f, state.palette.analog[1], [0, 0, 7], "Analog palette 1");
    check!(f, state.palette.analog[8], [4, 4, 4], "Analog palette 8");

    // === Scheduler: PIT Timer0 must be scheduled ===
    check_true!(
        f,
        state.scheduler.fire_cycles[common::EventKind::PitTimer0 as usize].is_some(),
        "PIT Timer0 event scheduled"
    );

    // === Beeper ===
    check_false!(f, state.beeper.buzzer_enabled, "Beeper disabled");
    check!(f, state.beeper.pit_reload, 998, "Beeper PIT reload");

    // === Mouse PPI ===
    check!(f, state.mouse_ppi.mode, 0x93, "Mouse PPI mode");
    check!(f, state.mouse_ppi.port_c, 0x00, "Mouse PPI port C");

    // === Memory: BDA fields ===
    check!(f, state.memory.ram[0x0400], 0x00, "BDA byte 0x0400");
    check!(f, state.memory.ram[0x0401], 0x60, "BDA EXPMMSZ");
    check!(f, state.memory.ram[0x0493], 0xFF, "BDA F2HD_MODE");
    check!(f, state.memory.ram[0x054C], 0x4E, "BDA PRXCRT");
    check!(f, state.memory.ram[0x054D], 0x50, "BDA PRXDUPD");
    check!(f, state.memory.ram[0x055C], 0x03, "BDA DISK_EQUIP");
    check!(f, state.memory.ram[0x0584], 0x90, "BDA BOOT_DEVICE");
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x0522),
        0x0B28,
        "KB shift table pointer"
    );
    check!(f, state.memory.ram[0x05CA], 0xFF, "BDA F2DD_MODE");
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x05CC),
        0x1AD7,
        "F2DD_POINTER offset"
    );
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x05CE),
        0xFD80,
        "F2DD_POINTER segment"
    );
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x05F8),
        0x1AAF,
        "F2HD_POINTER offset"
    );
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x05FA),
        0xFD80,
        "F2HD_POINTER segment"
    );

    // === Memory: Conventional RAM zeroed ===
    check_true!(
        f,
        state.memory.ram[0x1000..0x7C00].iter().all(|&b| b == 0)
            && state.memory.ram[0x7C06..0x1FC00].iter().all(|&b| b == 0),
        "Conventional RAM zeroed (excluding IRET frame at 0x7C00)"
    );
    check!(
        f,
        state.memory.ram[0x1FC00],
        0xFA,
        "Boot sector byte 0 (CLI)"
    );
    check!(
        f,
        state.memory.ram[0x1FC01],
        0xF4,
        "Boot sector byte 1 (HLT)"
    );

    // === Text VRAM attributes ===
    for i in (0x2000..0x3FC0).step_by(2) {
        let attr = state.memory.text_vram[i];
        if attr != 0xE1 && attr != 0xE5 {
            f.push(format!(
                "Text VRAM attr at {i:#06X}: expected 0xE1 or 0xE5, got {attr:#04X}"
            ));
            break;
        }
    }

    // === Memory state ===
    check_false!(f, state.memory.e_plane_enabled, "E-plane disabled");
    check!(f, state.memory.shadow_control, 0x46, "Shadow control");

    report_failures(&f, "RA");
}

#[test]
fn post_bios_state_pc9821as_ide() {
    let mut machine = super::create_machine_pc9821as_hdd();
    let _cycles = boot_to_halt_hdd!(machine);
    let state = machine.save_state();
    let mut f: Vec<String> = Vec::new();

    // === Clocks ===
    check!(f, state.clocks.cpu_clock_hz, 33_000_000, "CPU clock");
    check!(f, state.clocks.pit_clock_hz, 1_996_800, "PIT clock");

    // === PIC ===
    check!(
        f,
        state.pic.chips[0].icw,
        [0x11, 0x08, 0x80, 0x1D],
        "Master PIC ICW"
    );
    check!(f, state.pic.chips[0].imr, 0x3D, "Master PIC IMR");
    check!(f, state.pic.chips[0].isr, 0x00, "Master PIC ISR");
    check!(f, state.pic.chips[0].ocw3, 0x0B, "Master PIC OCW3");
    check!(
        f,
        state.pic.chips[1].icw,
        [0x11, 0x10, 0x07, 0x09],
        "Slave PIC ICW"
    );
    // IDE expansion ROM init unmasks IRQ 9 (slave IR1): 0xF7 & !0x02 = 0xF5.
    check!(f, state.pic.chips[1].imr, 0xF5, "Slave PIC IMR");
    check!(f, state.pic.chips[1].isr, 0x00, "Slave PIC ISR");

    // === PIT ===
    check!(f, state.pit.channels[0].ctrl, 0x30, "PIT ch0 ctrl");
    check!(f, state.pit.channels[0].value, 0x0000, "PIT ch0 value");
    check!(f, state.pit.channels[1].ctrl, 0x36, "PIT ch1 ctrl");
    check!(f, state.pit.channels[1].value, 0x03E6, "PIT ch1 value");
    check!(f, state.pit.channels[2].ctrl, 0x76, "PIT ch2 ctrl");
    check!(f, state.pit.channels[2].value, 0x0000, "PIT ch2 value");

    // === GDC Master ===
    check_true!(
        f,
        state.gdc_master.display_enabled,
        "Master GDC display enabled"
    );
    check!(f, state.gdc_master.pitch, 80, "Master GDC pitch");
    check!(f, state.gdc_master.al, 400, "Master GDC AL");
    check!(
        f,
        state.gdc_master.lines_per_row,
        16,
        "Master GDC lines per row"
    );
    check!(
        f,
        state.gdc_master.scroll[0].start_address,
        0,
        "Master GDC scroll[0] start"
    );
    check!(
        f,
        state.gdc_master.scroll[0].line_count,
        0x1FF,
        "Master GDC scroll[0] lines"
    );
    check_true!(
        f,
        state.gdc_master.draw_on_retrace,
        "Master GDC draw on retrace"
    );
    check!(f, state.gdc_master.aw, 80, "Master GDC AW");
    check!(f, state.gdc_master.hs, 8, "Master GDC HS");
    check!(f, state.gdc_master.vs, 8, "Master GDC VS");
    check!(f, state.gdc_master.hfp, 10, "Master GDC HFP");
    check!(f, state.gdc_master.hbp, 8, "Master GDC HBP");
    check!(f, state.gdc_master.vfp, 7, "Master GDC VFP");
    check!(f, state.gdc_master.vbp, 25, "Master GDC VBP");
    check!(f, state.gdc_master.cursor_top, 0, "Master GDC cursor top");
    check!(
        f,
        state.gdc_master.cursor_bottom,
        15,
        "Master GDC cursor bottom"
    );
    check!(
        f,
        state.gdc_master.cursor_blink_rate,
        12,
        "Master GDC blink rate"
    );
    check!(f, state.gdc_master.fifo.count, 0, "Master GDC FIFO empty");

    // === GDC Slave ===
    check_false!(
        f,
        state.gdc_slave.display_enabled,
        "Slave GDC display disabled"
    );
    check!(f, state.gdc_slave.pitch, 40, "Slave GDC pitch");
    check!(f, state.gdc_slave.mask, 1, "Slave GDC mask");
    check!(f, state.gdc_slave.al, 400, "Slave GDC AL");
    check!(
        f,
        state.gdc_slave.lines_per_row,
        2,
        "Slave GDC lines per row"
    );
    check_true!(
        f,
        state.gdc_slave.draw_on_retrace,
        "Slave GDC draw on retrace"
    );
    check!(f, state.gdc_slave.aw, 40, "Slave GDC AW");
    check!(f, state.gdc_slave.hs, 4, "Slave GDC HS");
    check!(f, state.gdc_slave.vs, 8, "Slave GDC VS");
    check!(f, state.gdc_slave.hfp, 5, "Slave GDC HFP");
    check!(f, state.gdc_slave.hbp, 4, "Slave GDC HBP");
    check!(f, state.gdc_slave.vfp, 7, "Slave GDC VFP");
    check!(f, state.gdc_slave.vbp, 25, "Slave GDC VBP");
    check_false!(
        f,
        state.gdc_slave.scroll[0].im,
        "Slave GDC scroll[0] bitmap mode"
    );
    check!(f, state.gdc_slave.fifo.count, 0, "Slave GDC FIFO empty");

    // === NMI / A20 / misc ===
    check_true!(f, state.nmi_enabled, "NMI enabled");
    check_false!(f, state.a20_enabled, "A20 disabled");
    check!(f, state.fdc_media, 3, "FDC media");
    check!(f, state.vram_ems_bank, 0x20, "VRAM EMS bank");
    check!(f, state.ram_window, 8, "RAM window");
    check_true!(f, state.b_bank_ems, "B-bank EMS");
    check!(f, state.protected_memory_max, 0xE0, "Protected memory max");
    check!(f, state.tram_wait, 1, "TRAM wait");
    check!(f, state.vram_wait, 6, "VRAM wait");
    check!(f, state.grcg_wait, 8, "GRCG wait");

    // === Keyboard ===
    check!(f, state.keyboard.mode, 0x5E, "KB mode");
    check!(f, state.keyboard.command, 0x16, "KB command");
    check!(f, state.keyboard.data, 0xFF, "KB data");
    check_false!(f, state.keyboard.rx_ready, "KB rx not ready");
    check_false!(f, state.keyboard.expect_mode, "KB not expecting mode");

    // === Serial ===
    check!(f, state.serial.mode, 0x02, "Serial mode");
    check!(f, state.serial.command, 0x40, "Serial command");
    check_true!(f, state.serial.expect_mode, "Serial expecting mode");

    // === System PPI ===
    check!(f, state.system_ppi.port_b, 0xA0, "System PPI port B");
    check!(f, state.system_ppi.port_c, 0xB8, "System PPI port C");

    // === FDC 1MB ===
    check!(f, state.fdc_1mb.status, 0x80, "FDC 1MB MSR");
    check!(f, state.fdc_1mb.srt, 12, "FDC 1MB SRT");
    check!(f, state.fdc_1mb.hut, 15, "FDC 1MB HUT");
    check!(f, state.fdc_1mb.hlt, 18, "FDC 1MB HLT");

    // === FDC 640K ===
    check!(f, state.fdc_640k.status, 0x80, "FDC 640K MSR");
    check!(f, state.fdc_640k.control, 0x48, "FDC 640K control");

    // === GRCG ===
    check!(f, state.grcg.mode, 0x00, "GRCG mode");
    check!(f, state.grcg.tile, [51, 85, 0, 0], "GRCG tile");
    check!(f, state.grcg.chip, 3, "GRCG chip (EGC)");

    // === Display control ===
    check!(f, state.display_control.video_mode, 0x99, "Video mode");
    check!(
        f,
        state.display_control.display_line_count,
        1,
        "Display line count"
    );

    // === Palette ===
    check!(f, state.palette.index, 15, "Palette index");
    check!(f, state.palette.analog[0], [0, 0, 0], "Analog palette 0");
    check!(f, state.palette.analog[1], [0, 0, 7], "Analog palette 1");
    check!(f, state.palette.analog[8], [4, 4, 4], "Analog palette 8");

    // === Scheduler: PIT Timer0 must be scheduled ===
    check_true!(
        f,
        state.scheduler.fire_cycles[common::EventKind::PitTimer0 as usize].is_some(),
        "PIT Timer0 event scheduled"
    );

    // === Beeper ===
    check_false!(f, state.beeper.buzzer_enabled, "Beeper disabled");
    check!(f, state.beeper.pit_reload, 998, "Beeper PIT reload");

    // === Mouse PPI ===
    check!(f, state.mouse_ppi.mode, 0x93, "Mouse PPI mode");
    check!(f, state.mouse_ppi.port_c, 0x00, "Mouse PPI port C");

    // === IDE ===
    // IVT[0x11] should point to the IDE ROM's IRQ 9 handler at D800:008D.
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x0044),
        0x008D,
        "IVT[0x11] offset (IDE IRQ handler)"
    );
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x0046),
        0xD800,
        "IVT[0x11] segment (IDE ROM)"
    );
    // IDE ROM presence markers.
    check!(
        f,
        state.memory.ram[0x04B0],
        0xD8,
        "IDE ROM presence marker 0x04B0"
    );
    check!(
        f,
        state.memory.ram[0x04B8],
        0xD8,
        "IDE ROM presence marker 0x04B8"
    );

    // === Memory: BDA fields ===
    check!(f, state.memory.ram[0x0400], 0x00, "BDA byte 0x0400");
    check!(f, state.memory.ram[0x0401], 0x70, "BDA EXPMMSZ");
    check!(f, state.memory.ram[0x0493], 0xFF, "BDA F2HD_MODE");
    check!(f, state.memory.ram[0x054C], 0x4E, "BDA PRXCRT");
    check!(f, state.memory.ram[0x054D], 0x50, "BDA PRXDUPD");
    check!(f, state.memory.ram[0x0584], 0x80, "BDA BOOT_DEVICE");
    check!(
        f,
        read_ram_u16(&state.memory.ram, 0x0522),
        0x0B28,
        "KB shift table pointer"
    );
    check!(f, state.memory.ram[0x05CA], 0xFF, "BDA F2DD_MODE");

    // === Memory: Boot sector ===
    check!(
        f,
        state.memory.ram[0x1FC00],
        0xFA,
        "Boot sector byte 0 (CLI)"
    );
    check!(
        f,
        state.memory.ram[0x1FC01],
        0xF4,
        "Boot sector byte 1 (HLT)"
    );

    // === Text VRAM attributes ===
    for i in (0x2000..0x3FC0).step_by(2) {
        let attr = state.memory.text_vram[i];
        if attr != 0xE1 && attr != 0xE5 {
            f.push(format!(
                "Text VRAM attr at {i:#06X}: expected 0xE1 or 0xE5, got {attr:#04X}"
            ));
            break;
        }
    }

    // === Memory state ===
    check_false!(f, state.memory.e_plane_enabled, "E-plane disabled");
    check!(f, state.memory.shadow_control, 0x46, "Shadow control");

    report_failures(&f, "PC9821AS+IDE");
}

macro_rules! boot_2dd_to_halt {
    ($machine:expr) => {{
        use common::Cpu as _;
        let disk = $crate::make_halt_boot_disk_2dd();
        $machine.bus.insert_floppy(0, disk, None);

        const MAX_CYCLES: u64 = 500_000_000;
        const CHECK_INTERVAL: u64 = 1_000_000;

        let mut total_cycles = 0u64;
        loop {
            total_cycles += $machine.run_for(CHECK_INTERVAL);
            if $machine.cpu.halted() {
                break;
            }
            assert!(
                total_cycles < MAX_CYCLES,
                "Machine did not halt within {} cycles",
                MAX_CYCLES
            );
        }
        total_cycles
    }};
}

#[test]
fn hle_bootstrap_2dd_sets_640kb_boot_device_vm() {
    let mut machine = super::create_machine_vm();
    boot_2dd_to_halt!(machine);
    let state = machine.save_state();
    // Boot device should be 0x70 (640KB FDC) for a 2DD disk.
    assert_eq!(state.memory.ram[0x0584], 0x70, "2DD boot device");
    // DISK_EQUIP: drive 0 should be in 640KB FDD (055Dh bit 4), not 1MB (055Ch bit 0).
    assert_eq!(
        state.memory.ram[0x055C] & 0x01,
        0x00,
        "Drive 0 should not be in 1MB DISK_EQUIP"
    );
    assert_ne!(
        state.memory.ram[0x055D] & 0x10,
        0x00,
        "Drive 0 should be in 640KB DISK_EQUIP"
    );
}

#[test]
fn hle_bootstrap_2dd_sets_640kb_boot_device_vx() {
    let mut machine = super::create_machine_vx();
    boot_2dd_to_halt!(machine);
    let state = machine.save_state();
    assert_eq!(state.memory.ram[0x0584], 0x70, "2DD boot device");
    assert_eq!(
        state.memory.ram[0x055C] & 0x01,
        0x00,
        "Drive 0 should not be in 1MB DISK_EQUIP"
    );
    assert_ne!(
        state.memory.ram[0x055D] & 0x10,
        0x00,
        "Drive 0 should be in 640KB DISK_EQUIP"
    );
}

#[test]
fn hle_bootstrap_2hd_keeps_1mb_boot_device_vx() {
    let mut machine = super::create_machine_vx();
    boot_to_halt!(machine);
    let state = machine.save_state();
    // 2HD disk (the default halt boot disk) should use DA=0x90 (1MB FDC).
    assert_eq!(state.memory.ram[0x0584], 0x90, "2HD boot device");
    // DISK_EQUIP: drive 0 should remain in 1MB FDD (055Ch bit 0).
    assert_ne!(
        state.memory.ram[0x055C] & 0x01,
        0x00,
        "Drive 0 should be in 1MB DISK_EQUIP"
    );
}
