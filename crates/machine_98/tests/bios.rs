use common::{CpuMode, MachineModel};

macro_rules! boot_to_halt {
    ($machine:expr) => {{
        use common::Cpu as _;
        let disk = $crate::make_halt_boot_disk();
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

macro_rules! boot_to_halt_hdd {
    ($machine:expr) => {{
        use common::Cpu as _;

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

use common::{BUILTIN_FONT_ROM, Bus, Cpu};
use device::{
    cdrom::CdImage,
    disk::{HddFormat, HddGeometry, HddImage},
    floppy::FloppyImage,
};
use machine_98::{Pc9801F, Pc9801Ra, Pc9801Vm, Pc9801Vx, Pc9821As};

type TrackList<'a> = [(u8, u8, &'a [(u8, &'a [u8])])];

fn build_2hd_d88(tracks: &TrackList<'_>, write_protected: bool) -> Vec<u8> {
    const HEADER_SIZE: usize = 0x2B0;
    const SECTOR_HEADER_SIZE: usize = 16;

    let mut image = vec![0u8; HEADER_SIZE];
    image[..4].copy_from_slice(b"TEST");
    if write_protected {
        image[0x1A] = 0x10;
    }
    image[0x1B] = 0x20; // 2HD

    for &(cylinder, head, sectors) in tracks {
        let track_index = (cylinder as usize) * 2 + head as usize;
        let track_offset = image.len() as u32;
        let pointer_base = 0x20 + track_index * 4;
        image[pointer_base..pointer_base + 4].copy_from_slice(&track_offset.to_le_bytes());

        let num_sectors = sectors.len() as u16;
        for &(record, data) in sectors {
            let n: u8 = match data.len() {
                128 => 0,
                256 => 1,
                512 => 2,
                _ => 3,
            };
            let mut header = [0u8; SECTOR_HEADER_SIZE];
            header[0] = cylinder;
            header[1] = head;
            header[2] = record;
            header[3] = n;
            header[4..6].copy_from_slice(&num_sectors.to_le_bytes());
            let data_size = data.len() as u16;
            header[0x0E..0x10].copy_from_slice(&data_size.to_le_bytes());
            image.extend_from_slice(&header);
            image.extend_from_slice(data);
        }
    }

    let disk_size = image.len() as u32;
    image[0x1C..0x20].copy_from_slice(&disk_size.to_le_bytes());
    image
}

fn build_2dd_d88(tracks: &TrackList<'_>, write_protected: bool) -> Vec<u8> {
    const HEADER_SIZE: usize = 0x2B0;
    const SECTOR_HEADER_SIZE: usize = 16;

    let mut image = vec![0u8; HEADER_SIZE];
    image[..4].copy_from_slice(b"TEST");
    if write_protected {
        image[0x1A] = 0x10;
    }
    image[0x1B] = 0x10; // 2DD

    for &(cylinder, head, sectors) in tracks {
        let track_index = (cylinder as usize) * 2 + head as usize;
        let track_offset = image.len() as u32;
        let pointer_base = 0x20 + track_index * 4;
        image[pointer_base..pointer_base + 4].copy_from_slice(&track_offset.to_le_bytes());

        let num_sectors = sectors.len() as u16;
        for &(record, data) in sectors {
            let n: u8 = match data.len() {
                128 => 0,
                256 => 1,
                512 => 2,
                _ => 3,
            };
            let mut header = [0u8; SECTOR_HEADER_SIZE];
            header[0] = cylinder;
            header[1] = head;
            header[2] = record;
            header[3] = n;
            header[4..6].copy_from_slice(&num_sectors.to_le_bytes());
            let data_size = data.len() as u16;
            header[0x0E..0x10].copy_from_slice(&data_size.to_le_bytes());
            image.extend_from_slice(&header);
            image.extend_from_slice(data);
        }
    }

    let disk_size = image.len() as u32;
    image[0x1C..0x20].copy_from_slice(&disk_size.to_le_bytes());
    image
}

fn make_halt_boot_disk() -> FloppyImage {
    let mut boot_sector = vec![0u8; 1024];
    boot_sector[0] = 0xFA; // CLI
    boot_sector[1] = 0xF4; // HLT
    let sectors: &[(u8, &[u8])] = &[(1, &boot_sector)];
    let tracks = &[(0, 0, sectors)];
    let d88 = build_2hd_d88(tracks, false);
    FloppyImage::from_d88_bytes(&d88).expect("halt boot disk")
}

fn make_halt_boot_disk_2dd() -> FloppyImage {
    let mut boot_sector = vec![0u8; 512];
    boot_sector[0] = 0xFA; // CLI
    boot_sector[1] = 0xF4; // HLT
    let sectors: &[(u8, &[u8])] = &[(1, &boot_sector)];
    let tracks = &[(0, 0, sectors)];
    let d88 = build_2dd_d88(tracks, false);
    FloppyImage::from_d88_bytes(&d88).expect("halt boot disk 2dd")
}

fn make_halt_boot_hdd() -> HddImage {
    let geometry = HddGeometry {
        cylinders: 153,
        heads: 4,
        sectors_per_track: 33,
        sector_size: 256,
    };
    let total = geometry.total_bytes() as usize;
    let mut data = vec![0u8; total];
    data[0] = 0xFA; // CLI
    data[1] = 0xF4; // HLT
    HddImage::from_raw(geometry, HddFormat::Thd, data)
}

fn make_halt_boot_ide_hdd() -> HddImage {
    let geometry = HddGeometry {
        cylinders: 20,
        heads: 4,
        sectors_per_track: 17,
        sector_size: 512,
    };
    let total = geometry.total_bytes() as usize;
    let mut data = vec![0u8; total];
    data[0] = 0xFA; // CLI
    data[1] = 0xF4; // HLT
    HddImage::from_raw(geometry, HddFormat::Hdi, data)
}

fn make_empty_boot_hdd() -> HddImage {
    let geometry = HddGeometry {
        cylinders: 153,
        heads: 4,
        sectors_per_track: 33,
        sector_size: 256,
    };
    let total = geometry.total_bytes() as usize;
    let data = vec![0u8; total];
    HddImage::from_raw(geometry, HddFormat::Thd, data)
}

fn make_empty_boot_ide_hdd() -> HddImage {
    let geometry = HddGeometry {
        cylinders: 20,
        heads: 4,
        sectors_per_track: 17,
        sector_size: 512,
    };
    let total = geometry.total_bytes() as usize;
    let data = vec![0u8; total];
    HddImage::from_raw(geometry, HddFormat::Hdi, data)
}

fn make_halt_boot_cdimage() -> CdImage {
    let cue = "FILE \"test.bin\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n";
    let mut sector = vec![0u8; 2048];
    sector[0] = 0xFA; // CLI
    sector[1] = 0xF4; // HLT
    sector[0x3FE] = 0x55; // Boot signature
    sector[0x3FF] = 0xAA;
    CdImage::from_cue(cue, sector).expect("boot CD image")
}

fn make_non_bootable_cdimage() -> CdImage {
    let cue = "FILE \"test.bin\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n";
    let mut sector = vec![0u8; 2048];
    sector[0] = 0xFA; // CLI
    sector[1] = 0xF4; // HLT
    CdImage::from_cue(cue, sector).expect("non-bootable CD image")
}

fn create_machine_pc9821as_cdrom() -> Pc9821As {
    let mut machine = create_machine_pc9821as();
    machine.bus.insert_cdrom(make_halt_boot_cdimage());
    machine
}

fn create_machine_pc9821as_non_bootable_cdrom() -> Pc9821As {
    let mut machine = create_machine_pc9821as();
    machine.bus.insert_cdrom(make_non_bootable_cdimage());
    machine
}

#[path = "bios/data.rs"]
mod data;

#[path = "bios/fdc_interrupt.rs"]
mod fdc_interrupt;

#[path = "bios/int1bh.rs"]
mod int1bh;

#[path = "bios/sasi_boot.rs"]
mod sasi_boot;

#[path = "bios/ide_boot.rs"]
mod ide_boot;

#[path = "bios/fdd_boot.rs"]
mod fdd_boot;

#[path = "bios/cdrom_boot.rs"]
mod cdrom_boot;

#[path = "bios/int18h.rs"]
mod int18h;

#[path = "bios/int09h.rs"]
mod int09h;

#[path = "bios/int19h.rs"]
mod int19h;

#[path = "bios/int1ah.rs"]
mod int1ah;

#[path = "bios/keyboard.rs"]
mod keyboard;

#[path = "bios/serial_receive.rs"]
mod serial_receive;

#[path = "bios/int1ch.rs"]
mod int1ch;

#[path = "bios/int1fh.rs"]
mod int1fh;

#[path = "bios/hw_vectors.rs"]
mod hw_vectors;

#[path = "bios/timer_tick.rs"]
mod timer_tick;

#[path = "bios/post_bios_state.rs"]
mod post_bios_state;

fn create_machine_f() -> Pc9801F {
    let mut machine = Pc9801F::new(
        cpu::I8086::new(),
        machine_98::Pc9801Bus::new(MachineModel::PC9801F, CpuMode::High, 48000),
    );
    machine.bus.load_font_rom(BUILTIN_FONT_ROM);
    machine
}

fn create_machine_vm() -> Pc9801Vm {
    let mut machine = Pc9801Vm::new(
        cpu::V30::new(),
        machine_98::Pc9801Bus::new(MachineModel::PC9801VM, CpuMode::High, 48000),
    );
    machine.bus.load_font_rom(BUILTIN_FONT_ROM);
    machine
}

fn create_machine_vx() -> Pc9801Vx {
    let mut machine = Pc9801Vx::new(
        cpu::I286::new(),
        machine_98::Pc9801Bus::new(MachineModel::PC9801VX, CpuMode::High, 48000),
    );
    machine.bus.load_font_rom(BUILTIN_FONT_ROM);
    machine
}

fn create_machine_ra() -> Pc9801Ra {
    let mut machine = Pc9801Ra::new(
        cpu::I386::new(),
        machine_98::Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::High, 48000),
    );
    machine.bus.load_font_rom(BUILTIN_FONT_ROM);
    machine
}

fn create_machine_pc9821as() -> Pc9821As {
    let mut machine = Pc9821As::new(
        cpu::I386::<{ cpu::CPU_MODEL_486_DX }>::new(),
        machine_98::Pc9801Bus::new(MachineModel::PC9821AS, CpuMode::High, 48000),
    );
    // TODO: We haven't verified our implementation yet against a real 9821 BIOS.
    machine.bus.load_font_rom(BUILTIN_FONT_ROM);
    machine
}

fn create_machine_vm_hdd() -> Pc9801Vm {
    let mut machine = create_machine_vm();
    machine.bus.insert_hdd(0, make_halt_boot_hdd(), None);
    machine
}

fn create_machine_f_hdd() -> Pc9801F {
    let mut machine = create_machine_f();
    machine.bus.insert_hdd(0, make_halt_boot_hdd(), None);
    machine
}

fn create_machine_vx_hdd() -> Pc9801Vx {
    let mut machine = create_machine_vx();
    machine.bus.insert_hdd(0, make_halt_boot_hdd(), None);
    machine
}

fn create_machine_ra_hdd() -> Pc9801Ra {
    let mut machine = create_machine_ra();
    machine.bus.insert_hdd(0, make_halt_boot_hdd(), None);
    machine
}

fn create_machine_pc9821as_hdd() -> Pc9821As {
    let mut machine = create_machine_pc9821as();
    machine.bus.insert_hdd(0, make_halt_boot_ide_hdd(), None);
    machine
}

fn create_machine_vm_empty_hdd() -> Pc9801Vm {
    let mut machine = create_machine_vm();
    machine.bus.insert_hdd(0, make_empty_boot_hdd(), None);
    machine
}

fn create_machine_pc9821as_empty_hdd() -> Pc9821As {
    let mut machine = create_machine_pc9821as();
    machine.bus.insert_hdd(0, make_empty_boot_ide_hdd(), None);
    machine
}

fn write_bytes(bus: &mut impl Bus, addr: u32, data: &[u8]) {
    for (i, &b) in data.iter().enumerate() {
        bus.write_byte(addr + i as u32, b);
    }
}

const CALLBACK_IRET: &[u8] = &[
    0xCF, // IRET
];

const TEST_CODE: u32 = 0x1000;
const TEST_CALLBACK: u32 = 0x2000;

const KB_BUFFER_START: usize = 0x0502;
const KB_HEAD: usize = 0x0524;
const KB_TAIL: usize = 0x0526;
const KB_COUNT: usize = 0x0528;
const KB_STATUS_START: usize = 0x052A;
const KB_SHIFT_STATE: usize = 0x053A;

fn read_ivt_vector(ram: &[u8; 0xA0000], vector: u8) -> (u16, u16) {
    let base = (vector as usize) * 4;
    let offset = u16::from_le_bytes([ram[base], ram[base + 1]]);
    let segment = u16::from_le_bytes([ram[base + 2], ram[base + 3]]);
    (segment, offset)
}

fn read_ram_u16(ram: &[u8; 0xA0000], addr: usize) -> u16 {
    u16::from_le_bytes([ram[addr], ram[addr + 1]])
}

fn make_sti_hlt_code(num_interrupts: usize) -> Vec<u8> {
    let mut code = vec![0xFB]; // STI
    code.extend(std::iter::repeat_n(0xF4_u8, num_interrupts + 1)); // HLTs
    code
}

fn run_vm(machine: &mut Pc9801Vm, code: &[u8], budget: u64) -> u64 {
    write_bytes(&mut machine.bus, 0x100, code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State {
            ip: 0x0100,
            ..Default::default()
        };
        s.set_sp(0x1000);
        s
    });
    machine.cpu.run_for(budget, &mut machine.bus)
}

fn run_f(machine: &mut Pc9801F, code: &[u8], budget: u64) -> u64 {
    write_bytes(&mut machine.bus, 0x100, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: 0x0100,
            ..Default::default()
        };
        s.set_sp(0x1000);
        s
    });
    machine.cpu.run_for(budget, &mut machine.bus)
}

fn boot_and_run_vm(main_code: &[u8], callback: &[u8], budget: u64) -> (Pc9801Vm, u64) {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, TEST_CODE, main_code);
    if !callback.is_empty() {
        write_bytes(&mut machine.bus, TEST_CALLBACK, callback);
    }
    machine.cpu.load_state(&{
        let mut s = cpu::V30State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    let cycles = machine.run_for(budget);
    (machine, cycles)
}

fn boot_and_run_f(main_code: &[u8], callback: &[u8], budget: u64) -> (Pc9801F, u64) {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, TEST_CODE, main_code);
    if !callback.is_empty() {
        write_bytes(&mut machine.bus, TEST_CALLBACK, callback);
    }
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    let cycles = machine.run_for(budget);
    (machine, cycles)
}

fn boot_and_run_vx(main_code: &[u8], callback: &[u8], budget: u64) -> (Pc9801Vx, u64) {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, TEST_CODE, main_code);
    if !callback.is_empty() {
        write_bytes(&mut machine.bus, TEST_CALLBACK, callback);
    }
    machine.cpu.load_state(&{
        let mut s = cpu::I286State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    let cycles = machine.run_for(budget);
    (machine, cycles)
}

fn boot_and_run_ra(main_code: &[u8], callback: &[u8], budget: u64) -> (Pc9801Ra, u64) {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, TEST_CODE, main_code);
    if !callback.is_empty() {
        write_bytes(&mut machine.bus, TEST_CALLBACK, callback);
    }
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    let cycles = machine.run_for(budget);
    (machine, cycles)
}

fn boot_inject_run_vm(scancodes: &[u8], code: &[u8], budget: u64) -> Pc9801Vm {
    let mut machine = create_machine_vm();
    boot_to_halt!(machine);
    for &sc in scancodes {
        machine.bus.push_keyboard_scancode(sc);
    }
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::V30State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(budget);
    machine
}

fn boot_inject_run_f(scancodes: &[u8], code: &[u8], budget: u64) -> Pc9801F {
    let mut machine = create_machine_f();
    boot_to_halt!(machine);
    for &sc in scancodes {
        machine.bus.push_keyboard_scancode(sc);
    }
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I8086State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(budget);
    machine
}

fn boot_inject_run_vx(scancodes: &[u8], code: &[u8], budget: u64) -> Pc9801Vx {
    let mut machine = create_machine_vx();
    boot_to_halt!(machine);
    for &sc in scancodes {
        machine.bus.push_keyboard_scancode(sc);
    }
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I286State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_sp(0x4000);
        s
    });
    machine.run_for(budget);
    machine
}

fn boot_inject_run_ra(scancodes: &[u8], code: &[u8], budget: u64) -> Pc9801Ra {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    for &sc in scancodes {
        machine.bus.push_keyboard_scancode(sc);
    }
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(budget);
    machine
}

fn boot_inject_run_pc9821as(scancodes: &[u8], code: &[u8], budget: u64) -> Pc9821As {
    let mut machine = create_machine_pc9821as_hdd();
    boot_to_halt_hdd!(machine);
    for &sc in scancodes {
        machine.bus.push_keyboard_scancode(sc);
    }
    write_bytes(&mut machine.bus, TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut s = cpu::I386State {
            ip: TEST_CODE as u16,
            ..Default::default()
        };
        s.set_esp(0x4000);
        s
    });
    machine.run_for(budget);
    machine
}
