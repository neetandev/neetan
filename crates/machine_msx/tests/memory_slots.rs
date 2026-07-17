use common::{
    Bus as _, TraceContext, TraceDeviceEvent, TraceEvent, TraceSink, TraceValue, trace_id,
};
use machine_msx::{MainBusView, MsxBus, MsxMachine, MsxModel};

/// Mode 0 with PPI port A configured as an output.
const PPI_MODE_MSX: u8 = 0x82;
/// PPI port A, which selects the four primary slots.
const PPI_PRIMARY_SLOT_PORT: u16 = 0xA8;
/// PPI control port.
const PPI_CONTROL_PORT: u16 = 0xAB;
/// Size of a plain phase-2 cartridge.
const PLAIN_CARTRIDGE_SIZE: usize = 0x8000;

fn io_write(bus: &mut MsxBus, port: u16, value: u8) {
    let mut view = MainBusView { bus };
    view.io_write_byte(port, value);
}

fn io_read(bus: &mut MsxBus, port: u16) -> u8 {
    let mut view = MainBusView { bus };
    view.io_read_byte(port)
}

fn memory_write(bus: &mut MsxBus, address: u16, value: u8) {
    let mut view = MainBusView { bus };
    view.write_byte(u32::from(address), value);
}

fn configure_primary_slots(bus: &mut MsxBus) {
    io_write(bus, PPI_CONTROL_PORT, PPI_MODE_MSX);
}

fn primary_register_with_page(page: usize, primary: u8) -> u8 {
    primary << (page * 2)
}

#[test]
fn every_primary_slot_can_be_selected_for_every_page() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    bus.load_synthetic_program(&[0x76]).unwrap();
    bus.insert_cartridge(0, &vec![0x11; PLAIN_CARTRIDGE_SIZE])
        .unwrap();
    bus.insert_cartridge(1, &vec![0x22; PLAIN_CARTRIDGE_SIZE])
        .unwrap();

    let addresses = [0x0000, 0x4000, 0x8000, 0xC000];
    for (page, address) in addresses.into_iter().enumerate() {
        bus.poke_byte(address, 0x30 + page as u8);
    }

    for (page, address) in addresses.into_iter().enumerate() {
        for primary in 0..4 {
            io_write(
                &mut bus,
                PPI_PRIMARY_SLOT_PORT,
                primary_register_with_page(page, primary),
            );
            let expected = match (primary, page) {
                (1, 1 | 2) => 0x11,
                (2, 1 | 2) => 0x22,
                (3, _) => 0x30 + page as u8,
                _ => 0xFF,
            };
            assert_eq!(
                bus.peek_byte(address),
                expected,
                "page {page}, primary slot {primary}"
            );
        }
    }
}

#[test]
fn every_secondary_slot_can_be_selected_for_every_page() {
    let mut bus = MsxBus::new(MsxModel::Msx2, 48_000);
    bus.load_synthetic_program(&[0x76]).unwrap();
    let addresses = [0x0000, 0x4000, 0x8000, 0xC000];
    for (page, address) in addresses.into_iter().enumerate() {
        bus.poke_byte(address, 0x60 + page as u8);
    }

    io_write(&mut bus, PPI_PRIMARY_SLOT_PORT, 0xFF);
    for (page, address) in addresses.into_iter().enumerate() {
        for secondary in 0..4 {
            memory_write(&mut bus, u16::MAX, secondary << (page * 2));
            let expected = if secondary == 3 {
                0x60 + page as u8
            } else {
                0xFF
            };
            assert_eq!(
                bus.peek_byte(address),
                expected,
                "page {page}, secondary slot {secondary}"
            );
        }
    }
}

#[test]
fn secondary_register_follows_the_expanded_primary_visible_in_page_three() {
    let mut bus = MsxBus::new(MsxModel::Msx2Plus, 48_000);
    configure_primary_slots(&mut bus);

    io_write(&mut bus, PPI_PRIMARY_SLOT_PORT, 0x00);
    memory_write(&mut bus, u16::MAX, 0x1B);
    assert_eq!(bus.peek_byte(u16::MAX), 0xE4);

    io_write(&mut bus, PPI_PRIMARY_SLOT_PORT, 0xC0);
    memory_write(&mut bus, u16::MAX, 0xE4);
    assert_eq!(bus.peek_byte(u16::MAX), 0x1B);

    io_write(&mut bus, PPI_PRIMARY_SLOT_PORT, 0x40);
    memory_write(&mut bus, u16::MAX, 0x55);
    assert_eq!(bus.peek_byte(u16::MAX), 0xFF);

    io_write(&mut bus, PPI_PRIMARY_SLOT_PORT, 0x00);
    assert_eq!(bus.peek_byte(u16::MAX), 0xE4);
    io_write(&mut bus, PPI_PRIMARY_SLOT_PORT, 0xC0);
    assert_eq!(bus.peek_byte(u16::MAX), 0x1B);
}

#[test]
fn a_write_to_a_selected_cartridge_does_not_reach_hidden_ram() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    bus.load_synthetic_program(&[0x76]).unwrap();
    bus.insert_cartridge(0, &vec![0x44; PLAIN_CARTRIDGE_SIZE])
        .unwrap();
    bus.poke_byte(0x4000, 0xA5);

    io_write(&mut bus, PPI_PRIMARY_SLOT_PORT, 0x04);
    memory_write(&mut bus, 0x4000, 0x5A);
    assert_eq!(bus.peek_byte(0x4000), 0x44);

    io_write(&mut bus, PPI_PRIMARY_SLOT_PORT, 0x0C);
    assert_eq!(bus.peek_byte(0x4000), 0xA5);
}

#[test]
fn mapper_ports_wrap_physical_segments_but_preserve_readback_bits() {
    let mut bus = MsxBus::new(MsxModel::Msx2Plus, 48_000);
    bus.load_synthetic_program(&[0x76]).unwrap();

    for segment in 0..32 {
        io_write(&mut bus, 0xFC, segment);
        bus.poke_byte(0x0000, 0x10 + segment);
    }
    for segment in 0..32 {
        io_write(&mut bus, 0xFC, segment);
        assert_eq!(bus.peek_byte(0x0000), 0x10 + segment);
    }

    io_write(&mut bus, 0xFC, 2);
    io_write(&mut bus, 0xFD, 2);
    bus.poke_byte(0x0000, 0xA5);
    assert_eq!(bus.peek_byte(0x4000), 0xA5);

    io_write(&mut bus, 0xFC, 0x20);
    assert_eq!(bus.peek_byte(0x0000), 0x10);
    assert_eq!(io_read(&mut bus, 0xFC), 0x80);
}

#[test]
fn synthetic_program_reports_all_four_primary_slots() {
    let program = [
        0x3E, 0xCF, 0xD3, 0xA8, 0x3A, 0x00, 0x80, 0x32, 0x00, 0xC1, 0x3E, 0xDF, 0xD3, 0xA8, 0x3A,
        0x00, 0x80, 0x32, 0x01, 0xC1, 0x3E, 0xEF, 0xD3, 0xA8, 0x3A, 0x00, 0x80, 0x32, 0x02, 0xC1,
        0x3E, 0xFF, 0xD3, 0xA8, 0x3A, 0x00, 0x80, 0x32, 0x03, 0xC1, 0x76,
    ];
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    bus.load_synthetic_program(&program).unwrap();
    bus.insert_cartridge(0, &vec![0x11; PLAIN_CARTRIDGE_SIZE])
        .unwrap();
    bus.insert_cartridge(1, &vec![0x22; PLAIN_CARTRIDGE_SIZE])
        .unwrap();
    bus.poke_byte(0x8000, 0x33);
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    let mut machine = MsxMachine::new(main_cpu, bus);

    machine.run_for(1_000);

    assert_eq!(
        [
            machine.bus.peek_byte(0xC100),
            machine.bus.peek_byte(0xC101),
            machine.bus.peek_byte(0xC102),
            machine.bus.peek_byte(0xC103),
        ],
        [0xFF, 0x11, 0x22, 0x33]
    );
}

#[derive(Default)]
struct DeviceTrace {
    slot_selects: usize,
    mapper_banks: Vec<(u64, u64, u64)>,
}

impl TraceSink for DeviceTrace {
    fn trace(&mut self, _context: TraceContext, event: TraceEvent<'_>) {
        let TraceEvent::Device(TraceDeviceEvent {
            device,
            action,
            fields,
        }) = event
        else {
            return;
        };
        if device == trace_id::device::MSX_SLOT && action == trace_id::action::SELECT {
            self.slot_selects += 1;
        }
        if device == trace_id::device::MSX_MAPPER && action == trace_id::action::BANK {
            let value = |name| {
                fields
                    .iter()
                    .find(|field| field.name == name)
                    .and_then(|field| match field.value {
                        TraceValue::Unsigned(value) => Some(value),
                        _ => None,
                    })
                    .unwrap()
            };
            self.mapper_banks.push((
                value(trace_id::field::PAGE),
                value(trace_id::field::VALUE),
                value(trace_id::field::SEGMENT),
            ));
        }
    }
}

#[test]
fn slot_and_mapper_changes_have_stable_device_traces() {
    let mut bus = MsxBus::new_with_trace_sink(MsxModel::Msx2Plus, 48_000, DeviceTrace::default());
    bus.load_synthetic_program(&[0x76]).unwrap();
    {
        let mut view = MainBusView { bus: &mut bus };
        view.io_write_byte(0xA8, 0xFF);
        view.io_write_byte(0xFC, 0x1F);
        view.write_byte(0xFFFF, 0xE4);
    }

    assert_eq!(bus.tracer().slot_selects, 8);
    assert_eq!(bus.tracer().mapper_banks, [(0, 0x1F, 0x1F)]);
}
