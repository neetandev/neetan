use common::Bus as _;
use device::{
    cassette::SampledSignal,
    opn_fm::{OpnFm, Ym2413},
};
use machine_msx::{
    MainBusView, MsxBus, MsxControllerDevice, MsxJoystickState, MsxMachine, MsxModel,
};

fn write_psg_register(view: &mut MainBusView<'_>, register: u8, value: u8) {
    view.io_write_byte(0xA0, register);
    view.io_write_byte(0xA1, value);
}

fn read_psg_register(view: &mut MainBusView<'_>, register: u8) -> u8 {
    view.io_write_byte(0xA0, register);
    view.io_read_byte(0xA2)
}

#[test]
fn ppi_scans_the_active_low_keyboard_matrix() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    bus.push_keyboard_scancode(0x1D);
    let mut view = MainBusView { bus: &mut bus };
    view.io_write_byte(0xAB, 0x82);
    view.io_write_byte(0xAA, 0x02);
    assert_eq!(view.io_read_byte(0xA9), 0xBF);

    view.bus.push_keyboard_scancode(0x9D);
    assert_eq!(view.io_read_byte(0xA9), 0xFF);
}

#[test]
fn every_keyboard_row_and_modifier_is_scannable() {
    let row_keys = [
        (0x0A, 0, 0),
        (0x08, 1, 0),
        (0x27, 2, 0),
        (0x2B, 3, 0),
        (0x24, 4, 0),
        (0x1E, 5, 0),
        (0x70, 6, 0),
        (0x65, 7, 0),
        (0x34, 8, 0),
        (0x4E, 9, 3),
        (0x47, 10, 0),
    ];
    for (scancode, row, bit) in row_keys {
        let mut bus = MsxBus::new(MsxModel::Msx2, 48_000);
        bus.push_keyboard_scancode(scancode);
        let mut view = MainBusView { bus: &mut bus };
        view.io_write_byte(0xAB, 0x82);
        view.io_write_byte(0xAA, row);
        assert_eq!(view.io_read_byte(0xA9), !(1 << bit));
    }

    for (scancode, bit) in [(0x70, 0), (0x74, 1), (0x73, 2), (0x71, 3), (0x72, 4)] {
        let mut bus = MsxBus::new(MsxModel::Msx2, 48_000);
        bus.push_keyboard_scancode(scancode);
        let mut view = MainBusView { bus: &mut bus };
        view.io_write_byte(0xAB, 0x82);
        view.io_write_byte(0xAA, 6);
        assert_eq!(view.io_read_byte(0xA9), !(1 << bit));
    }
}

#[test]
fn keyboard_layout_and_numeric_keypad_follow_the_model() {
    let mut msx = MsxBus::new(MsxModel::Msx, 48_000);
    let mut msx2 = MsxBus::new(MsxModel::Msx2, 48_000);
    msx.push_keyboard_scancode(0x4E);
    msx2.push_keyboard_scancode(0x4E);

    let mut msx_view = MainBusView { bus: &mut msx };
    msx_view.io_write_byte(0xAB, 0x82);
    msx_view.io_write_byte(0xAA, 0x09);
    assert_eq!(msx_view.io_read_byte(0xA9), 0xFF);
    write_psg_register(&mut msx_view, 7, 0x80);
    assert_eq!(read_psg_register(&mut msx_view, 14) & 0x40, 0);

    let mut msx2_view = MainBusView { bus: &mut msx2 };
    msx2_view.io_write_byte(0xAB, 0x82);
    msx2_view.io_write_byte(0xAA, 0x09);
    assert_eq!(msx2_view.io_read_byte(0xA9), 0xF7);
    write_psg_register(&mut msx2_view, 7, 0x80);
    assert_eq!(read_psg_register(&mut msx2_view, 14) & 0x40, 0x40);
}

#[test]
fn psg_selects_both_controller_ports() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    bus.set_controller(
        0,
        MsxControllerDevice::Joystick(MsxJoystickState {
            up: true,
            trigger_a: true,
            ..MsxJoystickState::default()
        }),
    );
    bus.set_controller(
        1,
        MsxControllerDevice::Joystick(MsxJoystickState {
            down: true,
            trigger_b: true,
            ..MsxJoystickState::default()
        }),
    );
    let mut view = MainBusView { bus: &mut bus };
    write_psg_register(&mut view, 7, 0x80);
    write_psg_register(&mut view, 15, 0x00);
    assert_eq!(read_psg_register(&mut view, 14) & 0x3F, 0x2E);
    write_psg_register(&mut view, 15, 0x40);
    assert_eq!(read_psg_register(&mut view, 14) & 0x3F, 0x1D);
}

#[test]
fn mouse_clocks_scaled_signed_movement_from_port_a() {
    let mut bus = MsxBus::new(MsxModel::Msx2Plus, 48_000);
    bus.push_mouse_delta(0x22, -0x12);
    let mut view = MainBusView { bus: &mut bus };
    write_psg_register(&mut view, 7, 0x80);

    let mut read_mouse = |strobe_high: bool| {
        write_psg_register(&mut view, 15, u8::from(strobe_high) << 4);
        read_psg_register(&mut view, 14) & 0x3F
    };
    assert_eq!(read_mouse(true), 0x3E);
    assert_eq!(read_mouse(false), 0x3F);
    assert_eq!(read_mouse(true), 0x30);
    assert_eq!(read_mouse(false), 0x39);
}

#[test]
/// Emits active-high direction pulses before native mouse software clocks pin 8.
fn mouse_movement_emulates_a_joystick_until_the_strobe_is_used() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    bus.push_mouse_delta(8, 0);
    let mut view = MainBusView { bus: &mut bus };
    write_psg_register(&mut view, 7, 0x80);

    assert_eq!(read_psg_register(&mut view, 14) & 0x3F, 0x34);
    assert_eq!(read_psg_register(&mut view, 14) & 0x3F, 0x30);
}

#[test]
fn mouse_second_scan_is_zero_for_trackball_detection() {
    let mut bus = MsxBus::new(MsxModel::Msx2Plus, 48_000);
    bus.push_mouse_delta(0x22, -0x12);
    let mut view = MainBusView { bus: &mut bus };
    write_psg_register(&mut view, 7, 0x80);

    for strobe_high in [true, false, true, false] {
        write_psg_register(&mut view, 15, u8::from(strobe_high) << 4);
        read_psg_register(&mut view, 14);
    }
    for strobe_high in [true, false, true, false] {
        write_psg_register(&mut view, 15, u8::from(strobe_high) << 4);
        assert_eq!(read_psg_register(&mut view, 14) & 0x3F, 0x30);
    }
}

#[test]
fn mouse_buttons_are_active_low() {
    let mut bus = MsxBus::new(MsxModel::Msx2Plus, 48_000);
    bus.push_mouse_delta(0, 0);
    bus.set_mouse_buttons(true, false);
    let mut view = MainBusView { bus: &mut bus };
    write_psg_register(&mut view, 7, 0x80);
    write_psg_register(&mut view, 15, 0x10);
    assert_eq!(read_psg_register(&mut view, 14) & 0x30, 0x20);

    view.bus.set_mouse_buttons(false, true);
    assert_eq!(read_psg_register(&mut view, 14) & 0x30, 0x10);
    view.bus.set_mouse_buttons(true, true);
    assert_eq!(read_psg_register(&mut view, 14) & 0x30, 0);
    view.bus.set_mouse_buttons(false, false);
    assert_eq!(read_psg_register(&mut view, 14) & 0x30, 0x30);
}

#[test]
fn mouse_timeout_resynchronizes_to_x_high() {
    let mut bus = MsxBus::new(MsxModel::Msx2Plus, 48_000);
    bus.push_mouse_delta(0x44, 0);
    let mut view = MainBusView { bus: &mut bus };
    write_psg_register(&mut view, 7, 0x80);
    write_psg_register(&mut view, 15, 0x10);
    assert_eq!(read_psg_register(&mut view, 14) & 0x3F, 0x3D);
    write_psg_register(&mut view, 15, 0);
    assert_eq!(read_psg_register(&mut view, 14) & 0x3F, 0x3E);

    view.bus.set_current_cycle(6_000);
    view.bus.push_mouse_delta(0x20, 0);
    write_psg_register(&mut view, 15, 0x10);
    assert_eq!(read_psg_register(&mut view, 14) & 0x3F, 0x3F);
}

#[test]
fn engaged_joystick_reclaims_port_a_from_the_mouse() {
    let mut bus = MsxBus::new(MsxModel::Msx2Plus, 48_000);
    bus.push_mouse_delta(1, 0);
    bus.set_joystick(
        0,
        MsxJoystickState {
            left: true,
            ..MsxJoystickState::default()
        },
    );
    let mut view = MainBusView { bus: &mut bus };
    write_psg_register(&mut view, 7, 0x80);
    assert_eq!(read_psg_register(&mut view, 14) & 0x3F, 0x3B);
}

#[test]
fn psg_honors_parallel_port_directions() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    let mut view = MainBusView { bus: &mut bus };
    write_psg_register(&mut view, 7, 0xC0);
    write_psg_register(&mut view, 14, 0x12);
    assert_eq!(read_psg_register(&mut view, 14), 0x12);
    write_psg_register(&mut view, 7, 0x80);
    assert_eq!(read_psg_register(&mut view, 14) & 0x3F, 0x3F);
}

#[test]
fn ppi_bit_set_reset_controls_individual_outputs() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    let mut view = MainBusView { bus: &mut bus };
    view.io_write_byte(0xAB, 0x82);
    assert!(view.bus.cassette_motor_on());
    view.io_write_byte(0xAB, 0x09);
    assert!(!view.bus.cassette_motor_on());
    view.io_write_byte(0xAB, 0x08);
    assert!(view.bus.cassette_motor_on());
    view.io_write_byte(0xAB, 0x0D);
    assert!(!view.bus.caps_led_on());
}

#[test]
fn psg_reset_registers_and_inputs_are_stable() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    let mut view = MainBusView { bus: &mut bus };
    assert_eq!(read_psg_register(&mut view, 0), 0);
    assert_eq!(read_psg_register(&mut view, 7), 0);
    assert_eq!(read_psg_register(&mut view, 14), 0x3F);
    assert_eq!(view.io_read_byte(0xA3), 0xFF);
}

#[test]
fn ppi_controls_cassette_transport_and_outputs() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    bus.insert_cassette_signal(SampledSignal {
        sample_rate: bus.cpu_clock_hz(),
        samples: vec![0xA0],
        bit_count: 3,
    });
    let mut view = MainBusView { bus: &mut bus };
    view.io_write_byte(0xAB, 0x82);
    assert!(view.bus.cassette_motor_on());
    assert!(read_psg_register(&mut view, 14) & 0x80 != 0);

    view.bus.set_current_cycle(1);
    assert_eq!(read_psg_register(&mut view, 14) & 0x80, 0);
    view.io_write_byte(0xAA, 0xF0);
    assert!(!view.bus.cassette_motor_on());
    assert!(view.bus.cassette_output_high());
    assert!(!view.bus.caps_led_on());
    view.bus.set_current_cycle(10_000);
    assert_eq!(read_psg_register(&mut view, 14) & 0x80, 0);
    view.io_write_byte(0xAB, 0x08);
    view.bus.set_current_cycle(10_001);
    assert_ne!(read_psg_register(&mut view, 14) & 0x80, 0);
}

#[test]
fn ym2149_and_keyboard_click_generate_audio() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    {
        let mut view = MainBusView { bus: &mut bus };
        write_psg_register(&mut view, 0, 0x20);
        write_psg_register(&mut view, 1, 0);
        write_psg_register(&mut view, 7, 0x3E);
        write_psg_register(&mut view, 8, 0x0F);
        view.io_write_byte(0xAB, 0x82);
        view.io_write_byte(0xAA, 0x80);
    }
    bus.set_current_cycle(u64::from(bus.cpu_clock_hz()) / 100);
    let mut output = vec![0.0; 1_024];
    let written = bus.generate_audio_samples(0.5, &mut output);
    assert!(written > 0);
    assert!(output[..written].iter().any(|sample| *sample != 0.0));
}

/// Verifies the HB-F1XDJ keyboard-click mix level.
#[test]
fn keyboard_click_uses_hb_f1xdj_mix_level() {
    let mut bus = MsxBus::new(MsxModel::Msx2Plus, 48_000);
    {
        let mut view = MainBusView { bus: &mut bus };
        view.io_write_byte(0xAB, 0x82);
        view.io_write_byte(0xAA, 0x80);
    }
    bus.set_current_cycle(u64::from(bus.cpu_clock_hz()) / 100);

    let volume = 0.75;
    let mut output = vec![0.0; 1_024];
    let written = bus.generate_audio_samples(volume, &mut output);
    let expected = volume * 127.0 / 504.0 * 0.75;

    assert!(written > 0);
    assert!(
        output[..written]
            .iter()
            .all(|sample| (*sample - expected).abs() < f32::EPSILON)
    );
}

#[test]
fn repeated_ym2149_sequences_generate_identical_audio() {
    fn render() -> Vec<f32> {
        let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
        {
            let mut view = MainBusView { bus: &mut bus };
            write_psg_register(&mut view, 0, 0x40);
            write_psg_register(&mut view, 1, 0);
            write_psg_register(&mut view, 7, 0x3E);
            write_psg_register(&mut view, 8, 0x0C);
        }
        bus.set_current_cycle(u64::from(bus.cpu_clock_hz()) / 50);
        let mut output = vec![0.0; 2_048];
        let written = bus.generate_audio_samples(0.75, &mut output);
        output.truncate(written);
        output
    }

    assert_eq!(render(), render());
}

/// Verifies the HB-F1XDJ MSX-MUSIC mix level.
#[test]
fn ym2413_sequence_uses_hb_f1xdj_mix_level() {
    let model = MsxModel::Msx2Plus;
    let sample_rate = 48_000;
    let current_cycle = u64::from(model.main_clock_hz()) / 20;
    let register_writes = [(0x30, 0x10), (0x10, 0x98), (0x20, 0x15)];
    let mut bus = MsxBus::new(model, sample_rate);
    let mut unmixed =
        OpnFm::<Ym2413>::new(model.main_clock_hz(), sample_rate, model.main_clock_hz());
    {
        let mut view = MainBusView { bus: &mut bus };
        for (register, value) in register_writes {
            view.io_write_byte(0x7C, register);
            view.io_write_byte(0x7D, value);
            unmixed.write_address(register, 0);
            unmixed.write_data(value, 0);
        }
    }
    bus.set_current_cycle(current_cycle);

    let volume = 0.75;
    let mut output = vec![0.0; 4_096];
    let mut unmixed_output = vec![0.0; output.len()];
    bus.generate_audio_samples(volume, &mut output);
    unmixed.generate_samples(
        current_cycle,
        model.main_clock_hz(),
        volume,
        &mut unmixed_output,
    );

    assert!(unmixed_output.iter().any(|sample| *sample != 0.0));
    for (sample, unmixed_sample) in output.iter().zip(unmixed_output) {
        let expected = unmixed_sample * 9.0 / 28.0 * 0.75;
        assert!((*sample - expected).abs() < 1.0e-6);
    }
}

#[test]
fn synthetic_program_scans_keyboard_controllers_and_starts_a_tone() {
    let program = [
        0x3E, 0x02, 0xD3, 0xAA, 0xDB, 0xA9, 0x32, 0x00, 0xC0, 0x3E, 0x07, 0xD3, 0xA0, 0x3E, 0x80,
        0xD3, 0xA1, 0x3E, 0x0F, 0xD3, 0xA0, 0xAF, 0xD3, 0xA1, 0x3E, 0x0E, 0xD3, 0xA0, 0xDB, 0xA2,
        0x32, 0x01, 0xC0, 0x3E, 0x0F, 0xD3, 0xA0, 0x3E, 0x40, 0xD3, 0xA1, 0x3E, 0x0E, 0xD3, 0xA0,
        0xDB, 0xA2, 0x32, 0x02, 0xC0, 0x3E, 0x00, 0xD3, 0xA0, 0x3E, 0x20, 0xD3, 0xA1, 0x3E, 0x07,
        0xD3, 0xA0, 0x3E, 0x3E, 0xD3, 0xA1, 0x3E, 0x08, 0xD3, 0xA0, 0x3E, 0x0F, 0xD3, 0xA1, 0x76,
    ];
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    bus.load_synthetic_program(&program).unwrap();
    bus.push_keyboard_scancode(0x1D);
    bus.set_controller(
        0,
        MsxControllerDevice::Joystick(MsxJoystickState {
            up: true,
            ..MsxJoystickState::default()
        }),
    );
    bus.set_controller(
        1,
        MsxControllerDevice::Joystick(MsxJoystickState {
            down: true,
            ..MsxJoystickState::default()
        }),
    );
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    let mut machine = MsxMachine::new(main_cpu, bus);
    machine.run_for(2_000);
    assert_eq!(machine.bus.peek_byte(0xC000), 0xBF);
    assert_eq!(machine.bus.peek_byte(0xC001) & 0x3F, 0x3E);
    assert_eq!(machine.bus.peek_byte(0xC002) & 0x3F, 0x3D);

    let mut output = vec![0.0; 256];
    let written = machine.bus.generate_audio_samples(0.5, &mut output);
    assert!(written > 0);
    assert!(output[..written].iter().any(|sample| *sample != 0.0));
}
