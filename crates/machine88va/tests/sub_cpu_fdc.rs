//! Sub-CPU floppy integration tests: the floppy sub-CPU boots and executes its
//! ROM through the main/sub interleave.

use std::path::{Path, PathBuf};

use machine88va::{LoadedRoms, load_rom_set};

#[path = "common/harness.rs"]
mod harness;
use harness::*;

/// A synthetic ROM set whose `subsys` image is a crafted Z80 program. The main
/// ROMs are filler (the V30 just churns and paces the clock); the sub-CPU ROM is
/// what the test observes.
fn roms_with_subsys(subsys: Vec<u8>) -> LoadedRoms {
    LoadedRoms {
        rom00: fill(0x10, 0x8_0000),
        rom08: fill(0x20, 0x2_0000),
        rom1: fill(0x30, 0x2_0000),
        font: fill(0x40, 0x5_0000),
        dictionary: fill(0x50, 0x8_0000),
        subsys,
    }
}

/// Path to the real VA1 dump directory, used only when present (it is untracked).
fn real_rom_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../roms_va/pc88va")
}

#[test]
fn sub_cpu_executes_its_rom_via_the_interleave() {
    // A tiny PC80S31K program: latch two sentinels into A and B, then spin.
    //   0x0000: LD A, 0x5A   (3E 5A)
    //   0x0002: LD B, 0xC3   (06 C3)
    //   0x0004: JR 0x0004    (18 FE)
    let mut subsys = vec![0u8; 0x2000];
    subsys[0x0000] = 0x3E;
    subsys[0x0001] = 0x5A;
    subsys[0x0002] = 0x06;
    subsys[0x0003] = 0xC3;
    subsys[0x0004] = 0x18;
    subsys[0x0005] = 0xFE;

    let mut machine = machine_from_roms(roms_with_subsys(subsys));

    // The sub CPU starts at its reset vector.
    assert_eq!(machine.sub_cpu.state.pc, 0x0000);

    machine.run_for(50_000);

    assert_eq!(
        machine.sub_cpu.state.a, 0x5A,
        "sub CPU ran LD A from its ROM"
    );
    assert_eq!(
        machine.sub_cpu.state.b, 0xC3,
        "sub CPU ran LD B from its ROM"
    );
    assert_eq!(
        machine.sub_cpu.state.pc, 0x0004,
        "sub CPU settled into the JR self-loop"
    );
}

#[test]
fn real_rom_set_loads_with_subsys_and_sub_cpu_boots() {
    let dir = real_rom_dir();
    if !dir.is_dir() {
        eprintln!(
            "skipping: real VA ROM dump not present at {}",
            dir.display()
        );
        return;
    }

    // The subsys (disk.rom) slot is now required: a full real load must succeed.
    let roms = load_rom_set(&dir).expect("load real PC88VA ROM set");
    assert_eq!(
        roms.subsys.len(),
        0x2000,
        "disk.rom is the 8 KiB sub-CPU ROM"
    );

    let mut machine = machine_from_roms(roms);
    assert_eq!(machine.sub_cpu.state.pc, 0x0000);

    machine.run_for(200_000);

    assert_ne!(
        machine.sub_cpu.state.pc, 0x0000,
        "the sub CPU advanced out of the reset vector executing vasubsys.rom"
    );
}
