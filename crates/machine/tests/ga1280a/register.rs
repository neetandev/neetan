use common::{Bus, CpuMode, MachineModel};
use machine::{NoTracing, Pc9801Bus};

const GA_GAPORT: u16 = 0x00D8;

fn ga_port(selector: u8, offset: u8) -> u16 {
    (u16::from(selector) << 8) | (GA_GAPORT + u16::from(offset))
}

fn setup_bus() -> Pc9801Bus<NoTracing> {
    let mut bus = Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::High, 48000);
    bus.install_ga1280a();
    bus
}

#[test]
fn status_register_identifies_ga1280_class() {
    let mut bus = setup_bus();

    assert_eq!(bus.io_read_byte(ga_port(0x1D, 0)), 0x53);
    assert_eq!(
        bus.ga1280a_state()
            .expect("GA board installed")
            .id_stream_cursor,
        0,
        "status reads must not consume the ID stream"
    );
}

#[test]
fn ga1280_crtc3f_high_byte_tracks_vsync_for_driver_poll_loop() {
    let mut bus = setup_bus();

    bus.io_write_byte(ga_port(0x1E, 0), 0x3F);
    assert_eq!(bus.io_read_byte(ga_port(0x1F, 1)) & 0x04, 0x00);

    bus.ga1280a_present_now();
    assert_eq!(bus.io_read_byte(ga_port(0x1F, 1)) & 0x04, 0x04);
}

#[test]
fn port_decode_handles_full_four_byte_block() {
    let mut bus = setup_bus();

    assert_eq!(bus.io_read_byte(ga_port(0x1D, 1)), b'.');
    assert_eq!(
        bus.ga1280a_state()
            .expect("GA board installed")
            .id_stream_cursor,
        1
    );

    assert_eq!(bus.io_read_byte(ga_port(0x1D, 3)), 0x00);
    assert_eq!(
        bus.ga1280a_state()
            .expect("GA board installed")
            .id_stream_cursor,
        1,
        "offset 3 must not advance the GA ID stream"
    );

    let wrong_candidate_port = (0x1Du16 << 8) | 0x00D5;
    assert_eq!(bus.io_read_byte(wrong_candidate_port), 0xFF);
    assert_eq!(
        bus.ga1280a_state()
            .expect("GA board installed")
            .id_stream_cursor,
        1,
        "wrong candidate base ports must not advance the configured board"
    );

    assert_eq!(bus.io_read_byte((0x1Du16 << 8) | 0x00D1), 0xFF);
    assert_eq!(
        bus.ga1280a_state()
            .expect("GA board installed")
            .id_stream_cursor,
        1,
        "alternate-base probes should be handled as absent boards"
    );

    bus.io_write_byte(ga_port(0x0E, 3), 0xFF);
    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.mod1, 0);
    assert_eq!(state.mod2, 0);
}

#[test]
fn plus_two_register_high_byte_round_trips_through_offset_three() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x12, 2), 0x03FF);
    assert_eq!(bus.io_read_byte(ga_port(0x12, 2)), 0xFF);
    assert_eq!(bus.io_read_byte(ga_port(0x12, 3)), 0x03);

    bus.io_write_byte(ga_port(0x12, 3), 0x12);
    assert_eq!(bus.io_read_word(ga_port(0x12, 2)), 0x12FF);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.pmw, 0x12FF);
}

#[test]
fn base_control_registers_round_trip_through_bus_io() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x01, 0), 0x1111);
    bus.io_write_word(ga_port(0x02, 0), 0x2222);
    bus.io_write_word(ga_port(0x03, 0), 0x3333);
    bus.io_write_word(ga_port(0x05, 0), 0x5555);
    bus.io_write_byte(ga_port(0x06, 0), 0x66);
    bus.io_write_byte(ga_port(0x07, 0), 0x77);
    bus.io_write_word(ga_port(0x09, 0), 0x9999);
    bus.io_write_word(ga_port(0x0B, 0), 0xBBBB);
    bus.io_write_byte(ga_port(0x0D, 0), 0xDD);
    bus.io_write_byte(ga_port(0x0E, 0), 0xE0);
    bus.io_write_byte(ga_port(0x0E, 1), 0xE1);
    bus.io_write_word(ga_port(0x10, 0), 0x1010);
    bus.io_write_word(ga_port(0x12, 0), 0x1212);
    bus.io_write_byte(ga_port(0x14, 0), 0xA4);
    bus.io_write_byte(ga_port(0x14, 1), 0xB4);
    bus.io_write_word(ga_port(0x15, 0), 0x1515);
    bus.io_write_word(ga_port(0x16, 0), 0x20C1);
    bus.io_write_byte(ga_port(0x16, 0), 0xDD);
    bus.io_write_byte(ga_port(0x16, 1), 0x50);
    bus.io_write_word(ga_port(0x17, 0), 0x1717);
    bus.io_write_word(ga_port(0x1C, 0), 0xABCD);
    bus.io_write_byte(ga_port(0x1C, 0), 0x34);
    bus.io_write_byte(ga_port(0x1C, 1), 0x56);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.srw, 0x1111);
    assert_eq!(state.srr, 0x2222);
    assert_eq!(state.wpm, 0x3333);
    assert_eq!(state.wbm, 0x5555);
    assert_eq!(state.prs, 0x66);
    assert_eq!(state.rpe, 0x77);
    assert_eq!(state.col, 0x9999);
    assert_eq!(state.tile, 0xBBBB);
    assert_eq!(state.rot, 0xDD);
    assert_eq!(state.mod1, 0xE0);
    assert_eq!(state.mod2, 0xE1);
    assert_eq!(state.fcol, 0x1010);
    assert_eq!(state.bcol, 0x1212);
    assert_eq!(state.fmix, 0xA4);
    assert_eq!(state.bmix, 0xB4);
    assert_eq!(state.cwb, 0x1515);
    assert_eq!(state.wba1, 0x50DD);
    assert_eq!(state.wba2, 0x1717);
    assert_eq!(state.system_register, 0xAB34);
    assert_eq!(state.system_auxiliary_register, 0x56);

    assert_eq!(bus.io_read_word(ga_port(0x01, 0)), 0x1111);
    assert_eq!(bus.io_read_word(ga_port(0x02, 0)), 0x2222);
    assert_eq!(bus.io_read_word(ga_port(0x03, 0)), 0x3333);
    assert_eq!(bus.io_read_word(ga_port(0x05, 0)), 0x5555);
    assert_eq!(bus.io_read_byte(ga_port(0x06, 0)), 0x66);
    assert_eq!(bus.io_read_byte(ga_port(0x06, 1)), 0x00);
    assert_eq!(bus.io_read_word(ga_port(0x06, 0)), 0x0066);
    assert_eq!(bus.io_read_byte(ga_port(0x07, 0)), 0x77);
    assert_eq!(bus.io_read_byte(ga_port(0x07, 1)), 0x00);
    assert_eq!(bus.io_read_word(ga_port(0x07, 0)), 0x0077);
    assert_eq!(bus.io_read_word(ga_port(0x09, 0)), 0x9999);
    assert_eq!(bus.io_read_word(ga_port(0x0B, 0)), 0xBBBB);
    assert_eq!(bus.io_read_byte(ga_port(0x0D, 0)), 0xDD);
    assert_eq!(bus.io_read_byte(ga_port(0x0E, 0)), 0xE0);
    assert_eq!(bus.io_read_byte(ga_port(0x0E, 1)), 0xE1);
    assert_eq!(bus.io_read_word(ga_port(0x10, 0)), 0x1010);
    assert_eq!(bus.io_read_word(ga_port(0x12, 0)), 0x1212);
    assert_eq!(bus.io_read_byte(ga_port(0x14, 0)), 0xA4);
    assert_eq!(bus.io_read_byte(ga_port(0x14, 1)), 0xB4);
    assert_eq!(bus.io_read_word(ga_port(0x15, 0)), 0x1515);
    assert_eq!(bus.io_read_word(ga_port(0x16, 0)), 0x50DD);
    assert_eq!(bus.io_read_byte(ga_port(0x16, 0)), 0xDD);
    assert_eq!(bus.io_read_byte(ga_port(0x16, 1)), 0x50);
    assert_eq!(bus.io_read_word(ga_port(0x17, 0)), 0x1717);
    assert_eq!(bus.io_read_word(ga_port(0x1C, 0)), 0xAB34);
    assert_eq!(bus.io_read_byte(ga_port(0x1C, 1)), 0x56);
}

#[test]
fn byte_sized_base_register_high_bytes_round_trip_through_word_accesses() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x06, 0), 0xA566);
    assert_eq!(bus.io_read_byte(ga_port(0x06, 0)), 0x66);
    assert_eq!(bus.io_read_byte(ga_port(0x06, 1)), 0xA5);
    assert_eq!(bus.io_read_word(ga_port(0x06, 0)), 0xA566);
    bus.io_write_byte(ga_port(0x06, 0), 0x12);
    assert_eq!(bus.io_read_word(ga_port(0x06, 0)), 0xA512);
    bus.io_write_byte(ga_port(0x06, 1), 0x34);
    assert_eq!(bus.io_read_word(ga_port(0x06, 0)), 0x3412);

    bus.io_write_word(ga_port(0x07, 0), 0x5555);
    assert_eq!(bus.io_read_byte(ga_port(0x07, 0)), 0x55);
    assert_eq!(bus.io_read_byte(ga_port(0x07, 1)), 0x55);
    assert_eq!(bus.io_read_word(ga_port(0x07, 0)), 0x5555);
    bus.io_write_byte(ga_port(0x07, 0), 0xAA);
    assert_eq!(bus.io_read_word(ga_port(0x07, 0)), 0x55AA);
    bus.io_write_byte(ga_port(0x07, 1), 0xCC);
    assert_eq!(bus.io_read_word(ga_port(0x07, 0)), 0xCCAA);

    bus.io_write_word(ga_port(0x0D, 0), 0xD00D);
    assert_eq!(bus.io_read_byte(ga_port(0x0D, 0)), 0x0D);
    assert_eq!(bus.io_read_byte(ga_port(0x0D, 1)), 0xD0);
    assert_eq!(bus.io_read_word(ga_port(0x0D, 0)), 0xD00D);
    bus.io_write_byte(ga_port(0x0D, 0), 0x77);
    assert_eq!(bus.io_read_word(ga_port(0x0D, 0)), 0xD077);
    bus.io_write_byte(ga_port(0x0D, 1), 0x88);
    assert_eq!(bus.io_read_word(ga_port(0x0D, 0)), 0x8877);

    bus.io_write_word(ga_port(0x0E, 0), 0xC001);
    assert_eq!(bus.io_read_byte(ga_port(0x0E, 0)), 0x01);
    assert_eq!(bus.io_read_byte(ga_port(0x0E, 1)), 0xC0);
    assert_eq!(bus.io_read_word(ga_port(0x0E, 0)), 0xC001);
    bus.io_write_byte(ga_port(0x0E, 0), 0x02);
    assert_eq!(bus.io_read_word(ga_port(0x0E, 0)), 0xC002);
    bus.io_write_byte(ga_port(0x0E, 1), 0x40);
    assert_eq!(bus.io_read_word(ga_port(0x0E, 0)), 0x4002);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.prs, 0x12);
    assert_eq!(state.prs_high, 0x34);
    assert_eq!(state.rpe, 0xAA);
    assert_eq!(state.rpe_high, 0xCC);
    assert_eq!(state.rot, 0x77);
    assert_eq!(state.rot_high, 0x88);
    assert_eq!(state.mod1, 0x02);
    assert_eq!(state.mod2, 0x40);
}

#[test]
fn tile_pattern_register_replays_last_eight_word_writes_in_order() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x0B, 0), 0xBBBB);
    assert_eq!(bus.io_read_word(ga_port(0x0B, 0)), 0xBBBB);
    assert_eq!(bus.io_read_word(ga_port(0x0B, 0)), 0xBBBB);

    let pattern = [
        0x0800, 0x0701, 0x0602, 0x0503, 0x0404, 0x0305, 0x0206, 0x0107,
    ];
    for value in pattern {
        bus.io_write_word(ga_port(0x0B, 0), value);
    }

    for value in pattern {
        assert_eq!(bus.io_read_word(ga_port(0x0B, 0)), value);
    }
    assert_eq!(bus.io_read_word(ga_port(0x0B, 0)), pattern[0]);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.tile, 0x0107);
}

#[test]
fn command_block_registers_are_independent_from_base_registers() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x01, 0), 0x1001);
    bus.io_write_word(ga_port(0x02, 0), 0x1002);
    bus.io_write_word(ga_port(0x03, 0), 0x1003);
    bus.io_write_word(ga_port(0x05, 0), 0x1005);
    bus.io_write_word(ga_port(0x09, 0), 0x1009);
    bus.io_write_word(ga_port(0x0B, 0), 0x100B);
    bus.io_write_word(ga_port(0x12, 0), 0x1012);

    bus.io_write_word(ga_port(0x01, 2), 0x2001);
    bus.io_write_word(ga_port(0x02, 2), 0x2002);
    bus.io_write_word(ga_port(0x03, 2), 0x2003);
    bus.io_write_word(ga_port(0x04, 2), 0x2004);
    bus.io_write_word(ga_port(0x05, 2), 0x2005);
    bus.io_write_word(ga_port(0x06, 2), 0x2006);
    bus.io_write_word(ga_port(0x08, 2), 0x2008);
    bus.io_write_word(ga_port(0x09, 2), 0x2009);
    bus.io_write_word(ga_port(0x0A, 2), 0x200A);
    bus.io_write_word(ga_port(0x0B, 2), 0x200B);
    bus.io_write_word(ga_port(0x12, 2), 0x2012);
    bus.io_write_word(ga_port(0x13, 2), 0x2013);
    bus.io_write_word(ga_port(0x1C, 2), 0x201C);
    bus.io_write_word(ga_port(0x1D, 2), 0x201D);
    bus.io_write_word(ga_port(0x1E, 2), 0x201E);
    bus.io_write_word(ga_port(0x1F, 2), 0x201F);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.srw, 0x1001);
    assert_eq!(state.srr, 0x1002);
    assert_eq!(state.wpm, 0x1003);
    assert_eq!(state.wbm, 0x1005);
    assert_eq!(state.col, 0x1009);
    assert_eq!(state.tile, 0x100B);
    assert_eq!(state.bcol, 0x1012);
    assert_eq!(state.errs, 0x2001);
    assert_eq!(state.k1, 0x2002);
    assert_eq!(state.k2, 0x2003);
    assert_eq!(state.opd1, 0x2004);
    assert_eq!(state.opd2, 0x2005);
    assert_eq!(state.lins, 0x2006);
    assert_eq!(state.srcx, 0x2008);
    assert_eq!(state.srcy, 0x2009);
    assert_eq!(state.dstx, 0x200A);
    assert_eq!(state.dsty, 0x200B);
    assert_eq!(state.pmw, 0x2012);
    assert_eq!(state.pmh, 0x2013);
    assert_eq!(state.pdt, 0x201C);
    assert_eq!(state.ssv, 0x201D);
    assert_eq!(state.pop1, 0x201E);
    assert_eq!(state.pop2, 0x201F);

    assert_eq!(bus.io_read_word(ga_port(0x01, 2)), 0x2001);
    assert_eq!(bus.io_read_word(ga_port(0x02, 2)), 0x2002);
    assert_eq!(bus.io_read_word(ga_port(0x03, 2)), 0x2003);
    assert_eq!(bus.io_read_word(ga_port(0x04, 2)), 0x2004);
    assert_eq!(bus.io_read_word(ga_port(0x05, 2)), 0x2005);
    assert_eq!(bus.io_read_word(ga_port(0x06, 2)), 0x2006);
    assert_eq!(bus.io_read_word(ga_port(0x08, 2)), 0x2008);
    assert_eq!(bus.io_read_word(ga_port(0x09, 2)), 0x2009);
    assert_eq!(bus.io_read_word(ga_port(0x0A, 2)), 0x200A);
    assert_eq!(bus.io_read_word(ga_port(0x0B, 2)), 0x200B);
    assert_eq!(bus.io_read_word(ga_port(0x12, 2)), 0x2012);
    assert_eq!(bus.io_read_word(ga_port(0x13, 2)), 0x2013);
    assert_eq!(bus.io_read_word(ga_port(0x1C, 2)), 0x201C);
    assert_eq!(bus.io_read_word(ga_port(0x1D, 2)), 0x201D);
    assert_eq!(bus.io_read_word(ga_port(0x1E, 2)), 0x201E);
    assert_eq!(bus.io_read_word(ga_port(0x1F, 2)), 0x201F);
}

#[test]
fn crtc_index_masks_high_bit_for_data_register_selection() {
    let mut bus = setup_bus();

    bus.io_write_byte(ga_port(0x1E, 0), 0xB8);
    bus.io_write_word(ga_port(0x1F, 0), 0xCAFE);
    bus.io_write_byte(ga_port(0x1F, 0), 0x12);
    bus.io_write_byte(ga_port(0x1F, 1), 0x34);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.crtc_index, 0xB8);
    assert_eq!(state.crtc_registers[0x38], 0x3412);
    assert_eq!(state.pop2, 0, "CRTC data must not alias POP2");
    assert!(state.crtc_write_count >= 2);

    assert_eq!(bus.io_read_byte(ga_port(0x1E, 0)), 0xB8);
    assert_eq!(bus.io_read_word(ga_port(0x1F, 0)), 0x3412);
    assert_eq!(bus.io_read_byte(ga_port(0x1F, 0)), 0x12);
    assert_eq!(bus.io_read_byte(ga_port(0x1F, 1)), 0x34);
}

#[test]
fn ramdac_palette_streams_and_control_ports_are_independent() {
    let mut bus = setup_bus();

    bus.io_write_byte(ga_port(0x18, 0), 7);
    bus.io_write_byte(ga_port(0x1A, 0), 0x11);
    bus.io_write_byte(ga_port(0x1A, 0), 0x22);
    bus.io_write_byte(ga_port(0x1A, 0), 0x33);
    bus.io_write_byte(ga_port(0x1A, 0), 0x44);
    bus.io_write_byte(ga_port(0x1A, 0), 0x55);
    bus.io_write_byte(ga_port(0x1A, 0), 0x66);
    bus.io_write_byte(ga_port(0x1B, 0), 0xFE);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.vdac_rs, 0x00);
    assert_eq!(state.palette_index_write, 9);
    assert_eq!(state.palette[7], [0x11, 0x22, 0x33]);
    assert_eq!(state.palette[8], [0x44, 0x55, 0x66]);
    assert_eq!(state.vdac_mask, 0xFE);

    bus.io_write_byte(ga_port(0x19, 0), 7);
    assert_eq!(bus.io_read_byte(ga_port(0x1A, 0)), 0x11);
    assert_eq!(bus.io_read_byte(ga_port(0x1A, 0)), 0x22);
    assert_eq!(bus.io_read_byte(ga_port(0x1A, 0)), 0x33);
    assert_eq!(bus.io_read_byte(ga_port(0x1A, 0)), 0x44);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.palette_index_read, 8);
    assert_eq!(state.palette_rgb_phase, 1);
    assert_eq!(state.vdac_rs, 0x00);
    assert_eq!(bus.io_read_byte(ga_port(0x18, 0)), 9);
    assert_eq!(bus.io_read_byte(ga_port(0x18, 1)), 0x00);
    assert_eq!(bus.io_read_byte(ga_port(0x1B, 0)), 0xFE);
}

#[test]
fn reset_unknowns_and_timeout_recovery_ports_are_accepted() {
    let mut bus = setup_bus();

    bus.io_write_byte(ga_port(0x0D, 0), 0x77);
    bus.io_write_byte(ga_port(0x0D, 1), 0x02);
    bus.io_write_byte(ga_port(0x0D, 1), 0x00);
    bus.io_write_word(ga_port(0x0F, 0), 0x0F0F);
    bus.io_write_word(ga_port(0x14, 2), 0x1414);
    bus.io_write_byte(ga_port(0x15, 2), 0x15);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.rot, 0x77);
    assert_eq!(state.unknown_sel_0f_off0, 0x0F0F);
    assert_eq!(state.unknown_sel_14_off2, 0x1414);
    assert_eq!(state.unknown_sel_15_off2, 0x15);
    assert!(state.reset_unknown_write_count >= 3);

    assert_eq!(bus.io_read_byte(ga_port(0x0D, 1)), 0);
    assert_eq!(bus.io_read_word(ga_port(0x0F, 0)), 0x0F0F);
    assert_eq!(bus.io_read_word(ga_port(0x14, 2)), 0x1414);
}
