use device::scc::StandardScc;
use machine_msx::{CartridgeError, CartridgeMapper, MsxBus, MsxModel};

/// Size of a plain phase-2 cartridge.
const PLAIN_CARTRIDGE_SIZE: usize = 0x8000;

#[test]
fn both_plain_cartridge_connectors_are_independent() {
    let first: Vec<u8> = (0..PLAIN_CARTRIDGE_SIZE)
        .map(|offset| offset as u8)
        .collect();
    let second: Vec<u8> = (0..PLAIN_CARTRIDGE_SIZE)
        .map(|offset| !(offset as u8))
        .collect();
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    bus.load_synthetic_program(&[0x76]).unwrap();
    bus.insert_cartridge(0, &first).unwrap();
    bus.insert_cartridge(1, &second).unwrap();

    {
        use common::Bus as _;
        let mut view = machine_msx::MainBusView { bus: &mut bus };
        view.io_write_byte(0xA8, 0x55);
    }
    assert_eq!(bus.peek_byte(0x4000), first[0]);
    assert_eq!(bus.peek_byte(0xBFFF), first[0x7FFF]);

    {
        use common::Bus as _;
        let mut view = machine_msx::MainBusView { bus: &mut bus };
        view.io_write_byte(0xA8, 0xAA);
    }
    assert_eq!(bus.peek_byte(0x4000), second[0]);
    assert_eq!(bus.peek_byte(0xBFFF), second[0x7FFF]);

    bus.eject_cartridge(1).unwrap();
    assert_eq!(bus.peek_byte(0x4000), 0xFF);
}

#[test]
fn plain_cartridge_validation_is_explicit() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    assert_eq!(
        bus.insert_cartridge(2, &vec![0; PLAIN_CARTRIDGE_SIZE]),
        Err(CartridgeError::InvalidSlot { slot: 2 })
    );
    assert_eq!(
        bus.insert_cartridge(0, &[0; 0x1000]),
        Err(CartridgeError::UnsupportedSize { size: 0x1000 })
    );
    assert_eq!(
        bus.eject_cartridge(2),
        Err(CartridgeError::InvalidSlot { slot: 2 })
    );
}

/// Verifies the standard cartridge SCC mix level.
#[test]
fn synthetic_konami_scc_cartridge_uses_standard_mix_level() {
    let mut image = vec![0; 0x1_0000];
    image[..6].copy_from_slice(&[0x32, 0x00, 0x50, 0x32, 0x00, 0x90]);
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    let info = bus.insert_cartridge(0, &image).unwrap();
    assert_eq!(info.mapper, CartridgeMapper::KonamiScc);

    {
        use common::Bus as _;
        let mut view = machine_msx::MainBusView { bus: &mut bus };
        view.io_write_byte(0xAB, 0x82);
        view.io_write_byte(0xA8, 0x55);
    }
    bus.poke_byte(0x9000, 0x3F);
    bus.poke_byte(0x9800, 0x80);
    bus.poke_byte(0x9801, 0x7F);
    bus.poke_byte(0x9880, 9);
    bus.poke_byte(0x9881, 0);
    bus.poke_byte(0x988A, 15);
    bus.poke_byte(0x988F, 1);
    let current_cycle = u64::from(bus.cpu_clock_hz()) / 60;
    bus.set_current_cycle(current_cycle);

    let mut unmixed = StandardScc::new();
    for (address, value) in [
        (0x00, 0x80),
        (0x01, 0x7F),
        (0x80, 9),
        (0x81, 0),
        (0x8A, 15),
        (0x8F, 1),
    ] {
        assert!(unmixed.write(address, value));
    }

    let mut output = vec![0.0; 2_000];
    let mut unmixed_output = vec![0.0; output.len()];
    let written = bus.generate_audio_samples(1.0, &mut output);
    unmixed.mix_samples(
        current_cycle,
        bus.cpu_clock_hz(),
        48_000,
        1.0,
        &mut unmixed_output,
    );

    assert!(written > 0);
    assert!(
        unmixed_output[..written]
            .iter()
            .any(|sample| *sample != 0.0)
    );
    for (sample, unmixed_sample) in output[..written].iter().zip(&unmixed_output[..written]) {
        let expected = unmixed_sample * 8.0 / 7.0 * 0.75;
        assert!((*sample - expected).abs() < 1.0e-6);
    }
}
