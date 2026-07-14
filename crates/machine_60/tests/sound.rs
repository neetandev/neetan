//! Sound tests: the AY-3-8910 on the mkII, the YM2203 (OPN) on the SR, the SR
//! FM timer interrupt path, and the joystick read through the PSG port A.

use common::JoystickState;
use machine_60::{Pc6000Bus, Pc6000Model};

mod harness;
use harness::{build_machine, fire_next_event};

/// Programs an SSG square-wave tone on channel A through the address/data pair
/// at `0xA0`/`0xA1` (shared by the AY-3-8910 and the YM2203 SSG block).
fn program_ssg_tone(bus: &mut Pc6000Bus) {
    let tone: [(u8, u8); 4] = [
        (0x00, 0xFE), // channel A period, fine
        (0x01, 0x00), // channel A period, coarse
        (0x07, 0x3E), // mixer: tone A enabled (bit 0 clear), rest off
        (0x08, 0x0F), // channel A amplitude, fixed maximum
    ];
    for (register, value) in tone {
        bus.io_write(0xA0, register);
        bus.io_write(0xA1, value);
    }
}

fn has_audio(bus: &mut Pc6000Bus) -> bool {
    bus.set_current_cycle(400_000);
    let mut output = vec![0.0f32; 1024 * 2];
    bus.generate_audio_samples(1.0, &mut output);
    output.iter().any(|&sample| sample != 0.0)
}

/// Uploads one voiced parameter frame through the uPD7752 ports (0xE0-0xE3).
fn upload_voice_frame(bus: &mut Pc6000Bus) {
    bus.io_write(0xE2, 0x00); // mode: 10 ms/frame, normal speed
    bus.io_write(0xE3, 0xFE); // external-message command
    for byte in [0x08u8, 0x55, 0x55, 0x55, 0x55, 0x55, 0xF4] {
        bus.io_write(0xE0, byte);
    }
}

#[test]
fn ay_ssg_tone_produces_audio() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2);
    program_ssg_tone(&mut machine.bus);
    assert!(has_audio(&mut machine.bus), "AY SSG tone produced silence");
}

#[test]
fn ay_is_silent_when_all_channels_disabled() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2);
    let bus = &mut machine.bus;
    // Disable every tone and noise channel and zero the amplitudes.
    for (register, value) in [(0x07u8, 0xFFu8), (0x08, 0x00), (0x09, 0x00), (0x0A, 0x00)] {
        bus.io_write(0xA0, register);
        bus.io_write(0xA1, value);
    }
    assert!(!has_audio(bus), "a fully disabled AY produced output");
}

#[test]
fn ay_joystick_reads_through_port_a() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2);
    let bus = &mut machine.bus;

    bus.set_joystick(JoystickState {
        left: true,
        ..Default::default()
    });

    // Address SSG register 0x0E (port A), then read it back at the data port.
    bus.io_write(0xA0, 0x0E);
    let value = bus.io_read(0xA2).0;
    assert_eq!(value & 0x04, 0, "left direction reads active-low");
    assert_ne!(value & 0x01, 0, "an unpressed direction stays high");
}

#[test]
fn mk2_voice_synthesizer_produces_audio() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2);
    upload_voice_frame(&mut machine.bus);
    assert!(
        has_audio(&mut machine.bus),
        "the uPD7752 produced no voice audio"
    );
}

#[test]
fn sr_voice_synthesizer_produces_audio() {
    let mut machine = build_machine(Pc6000Model::Pc6601Sr);
    upload_voice_frame(&mut machine.bus);
    assert!(
        has_audio(&mut machine.bus),
        "the uPD7752 produced no voice audio on the SR"
    );
}

#[test]
fn pc6001_has_no_voice_synthesizer() {
    let mut machine = build_machine(Pc6000Model::Pc6001);
    // The base machine has no uPD7752; writing the voice ports does nothing.
    upload_voice_frame(&mut machine.bus);
    assert!(
        !has_audio(&mut machine.bus),
        "the base PC-6001 unexpectedly produced voice audio"
    );
}

#[test]
fn sr_ym2203_ssg_tone_produces_audio() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2Sr);
    program_ssg_tone(&mut machine.bus);
    assert!(
        has_audio(&mut machine.bus),
        "YM2203 SSG tone produced silence"
    );
}

#[test]
fn sr_ym2203_status_read_responds() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2Sr);
    let bus = &mut machine.bus;
    bus.set_current_cycle(400_000);
    // The status port reports the chip is idle (busy bit clear), not open bus.
    assert_eq!(bus.io_read(0xA0).0 & 0x80, 0);
}

#[test]
fn sr_ym2203_joystick_reads_through_the_ssg_port() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2Sr);
    let bus = &mut machine.bus;

    bus.set_joystick(JoystickState {
        trigger1: true,
        ..Default::default()
    });

    bus.io_write(0xA0, 0x0E);
    let value = bus.io_read(0xA2).0;
    assert_eq!(value & 0x10, 0, "trigger 1 reads active-low");
    assert_ne!(value & 0x01, 0, "an unpressed direction stays high");
}

#[test]
fn sr_ym2203_timer_a_overflow_is_status_only_not_an_interrupt() {
    let mut machine = build_machine(Pc6000Model::Pc6001Mk2Sr);
    let bus = &mut machine.bus;

    // Program the sound interrupt vector slot (source index 3 -> port 0xBB) so a
    // stray FM interrupt would surface as this vector if one were ever routed.
    bus.io_write(0xBB, 0x8C);

    // Load YM2203 timer A with a short period and enable its overflow IRQ.
    for (register, value) in [(0x24u8, 0xFFu8), (0x25, 0x03), (0x27, 0x05)] {
        bus.io_write(0xA0, register);
        bus.io_write(0xA1, value);
    }

    // Pump events. The YM2203 /IRQ pin is not wired to the CPU on the PC-6001,
    // so the timer A overflow must never deliver an interrupt.
    for _ in 0..2000 {
        assert_ne!(
            fire_next_event(bus),
            Some(0x8C),
            "the YM2203 timer A overflow was wrongly routed as an interrupt"
        );
    }

    // The overflow is observable only through the status register (bit 0).
    assert_ne!(
        bus.io_read(0xA0).0 & 0x01,
        0,
        "the YM2203 timer A overflow flag was not set in the status register"
    );
}
