use common::{
    TraceAccess, TraceAccessKind, TraceAddressSpace, TraceContext, TraceEvent, TraceSink,
};
use machine_msx::{MainBusView, MsxBus, MsxMachine, MsxModel};

fn instruction_cycles(model: MsxModel, program: &[u8]) -> u64 {
    let mut bus = MsxBus::new(model, 48_000);
    bus.load_synthetic_program(program).unwrap();
    let mut cpu = cpu::Z80::new(bus.cpu_clock_hz());
    let mut view = MainBusView { bus: &mut bus };
    cpu.step(&mut view);
    cpu.cycles_consumed()
}

#[test]
fn representative_m1_fetches_each_take_one_wait() {
    for model in MsxModel::ALL {
        for (program, expected_cycles) in [
            (&[0x00][..], 5),
            (&[0xCB, 0x00], 10),
            (&[0xED, 0x44], 10),
            (&[0xDD, 0x00], 10),
            (&[0xFD, 0xCB, 0x00, 0x46], 22),
        ] {
            assert_eq!(
                instruction_cycles(model, program),
                expected_cycles,
                "{model:?} {program:02X?}"
            );
        }
    }
}

#[test]
fn operands_memory_and_io_do_not_add_waits() {
    for model in MsxModel::ALL {
        for (program, expected_cycles) in [
            (&[0x3E, 0x42][..], 8),
            (&[0x3A, 0x00, 0x40], 14),
            (&[0xD3, 0x98], 12),
        ] {
            assert_eq!(
                instruction_cycles(model, program),
                expected_cycles,
                "{model:?} {program:02X?}"
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ObservedAccess {
    tick: u64,
    space: TraceAddressSpace,
    kind: TraceAccessKind,
    address: u64,
}

#[derive(Default)]
struct AccessTrace {
    accesses: Vec<ObservedAccess>,
}

impl TraceSink for AccessTrace {
    fn trace(&mut self, context: TraceContext, event: TraceEvent<'_>) {
        if let TraceEvent::Access(TraceAccess {
            space,
            kind,
            address,
            ..
        }) = event
        {
            self.accesses.push(ObservedAccess {
                tick: context.tick,
                space,
                kind,
                address,
            });
        }
    }
}

#[test]
fn instruction_boundary_access_order_is_stable() {
    let mut bus = MsxBus::new_with_trace_sink(MsxModel::Msx, 48_000, AccessTrace::default());
    bus.load_synthetic_program(&[
        0x3A, 0x00, 0x40, // LD A,(0x4000)
        0xD3, 0x98, // OUT (0x98),A
    ])
    .unwrap();
    bus.poke_byte(0x4000, 0x42);
    let mut cpu = cpu::Z80::new(bus.cpu_clock_hz());

    {
        let mut view = MainBusView { bus: &mut bus };
        cpu.step(&mut view);
        cpu.step(&mut view);
    }

    assert_eq!(
        bus.tracer().accesses,
        [
            ObservedAccess {
                tick: 0,
                space: TraceAddressSpace::MAIN_MEMORY,
                kind: TraceAccessKind::Fetch,
                address: 0,
            },
            ObservedAccess {
                tick: 0,
                space: TraceAddressSpace::MAIN_MEMORY,
                kind: TraceAccessKind::Read,
                address: 1,
            },
            ObservedAccess {
                tick: 0,
                space: TraceAddressSpace::MAIN_MEMORY,
                kind: TraceAccessKind::Read,
                address: 2,
            },
            ObservedAccess {
                tick: 0,
                space: TraceAddressSpace::MAIN_MEMORY,
                kind: TraceAccessKind::Read,
                address: 0x4000,
            },
            ObservedAccess {
                tick: 14,
                space: TraceAddressSpace::MAIN_MEMORY,
                kind: TraceAccessKind::Fetch,
                address: 3,
            },
            ObservedAccess {
                tick: 14,
                space: TraceAddressSpace::MAIN_MEMORY,
                kind: TraceAccessKind::Read,
                address: 4,
            },
            ObservedAccess {
                tick: 14,
                space: TraceAddressSpace::MAIN_IO,
                kind: TraceAccessKind::Write,
                address: 0x4298,
            },
        ]
    );
}

#[test]
fn long_run_has_no_vdp_clock_drift() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    bus.load_synthetic_program(&[0x18, 0xFE]).unwrap();
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    let mut machine = MsxMachine::new(main_cpu, bus);

    machine.run_for(5_000_001);

    let cpu_cycle = machine.bus.current_cycle();
    assert_eq!(machine.bus.vdp_tick(), cpu_cycle * 6);
    assert_eq!(machine.bus.vdp_dot(), cpu_cycle * 3 / 2);
    assert_eq!(machine.bus.vdp_dot_phase(), (cpu_cycle as u8 & 1) * 2);
    let vdp_tick = cpu_cycle * 6;
    let completed_scanlines = if vdp_tick < 1_282 {
        0
    } else {
        (vdp_tick - 1_282) / 1_368 + 1
    };
    assert_eq!(machine.bus.completed_scanlines(), completed_scanlines);
    assert_eq!(machine.bus.scanline(), (completed_scanlines % 262) as u16);
    let next_scanline_tick = completed_scanlines * 1_368 + 1_282;
    let frame_ticks = 262 * 1_368;
    let vblank_phase = 227 * 1_368 + 202;
    let mut next_vblank_tick = (vdp_tick / frame_ticks) * frame_ticks + vblank_phase;
    if next_vblank_tick <= vdp_tick {
        next_vblank_tick += frame_ticks;
    }
    let next_tick = next_scanline_tick.min(next_vblank_tick);
    assert_eq!(machine.bus.next_event_cycle(), Some(next_tick.div_ceil(6)));
}
