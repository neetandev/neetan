use common::{Bus, Cpu, CpuMode, MachineModel};
use machine::{NoTracing, Pc9801Bus, Pc9801Ra};

const GA_GAPORT: u16 = 0x00D8;
const GA_ID_STREAM: &[u8; 16] = b".O DATA DEVICE I";
const GAINIT_WINDOW_BLOCK_COUNT: usize = 32;

fn ga_port(selector: u8, offset: u8) -> u16 {
    (u16::from(selector) << 8) | (GA_GAPORT + u16::from(offset))
}

fn setup_bus() -> Pc9801Bus<NoTracing> {
    let mut bus = Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::High, 48000);
    bus.install_ga1280a();
    bus
}

fn write_bytes(bus: &mut impl Bus, address: u32, bytes: &[u8]) {
    for (offset, &byte) in bytes.iter().enumerate() {
        bus.write_byte(address + offset as u32, byte);
    }
}

fn read_word_direct(bus: &Pc9801Bus<NoTracing>, address: u32) -> u16 {
    u16::from(bus.read_byte_direct(address)) | (u16::from(bus.read_byte_direct(address + 1)) << 8)
}

fn apply_gainit_reset_entry(bus: &mut Pc9801Bus<NoTracing>, encoded: u16, port_offset: u8) {
    let selector = ((encoded >> 8) as u8) & 0x7F;
    let offset = (encoded as u8).wrapping_add(port_offset);
    if encoded & 0x8000 != 0 {
        bus.io_write_byte(ga_port(selector, offset), 0);
    } else {
        bus.io_write_word(ga_port(selector, offset), 0);
    }
}

fn apply_gainit_reset_lists(bus: &mut Pc9801Bus<NoTracing>) {
    const ZERO_PORTS_BASE: &[u16] = &[
        0x0100, 0x0200, 0x0300, 0x0500, 0x8600, 0x0700, 0x0900, 0x0B00, 0x8D00, 0x8E00, 0x0F00,
        0x1000, 0x1200, 0x1400,
    ];
    const ZERO_PORTS_PLUS2: &[u16] = &[
        0x0100, 0x0200, 0x0300, 0x0400, 0x0500, 0x0600, 0x0800, 0x0900, 0x0A00, 0x0B00, 0x1400,
        0x9500, 0x1D00,
    ];

    for &encoded in ZERO_PORTS_BASE {
        apply_gainit_reset_entry(bus, encoded, 0);
    }
    for &encoded in ZERO_PORTS_PLUS2 {
        apply_gainit_reset_entry(bus, encoded, 2);
    }
}

fn gainit_choose_window_segment(
    occupied_blocks: &[bool; GAINIT_WINDOW_BLOCK_COUNT],
    required_blocks: usize,
) -> Option<u16> {
    assert!((1..=GAINIT_WINDOW_BLOCK_COUNT).contains(&required_blocks));
    let last_candidate = GAINIT_WINDOW_BLOCK_COUNT - required_blocks;
    let mut candidate_block = 0;

    while candidate_block <= last_candidate {
        if occupied_blocks[candidate_block..candidate_block + required_blocks]
            .iter()
            .all(|occupied| !occupied)
        {
            return Some(0xC000 + candidate_block as u16 * 0x0100);
        }
        candidate_block += 4;
    }

    None
}

#[test]
fn gainit_probe_contract_selects_configured_board() {
    let mut bus = setup_bus();

    let probed: Vec<u8> = (0..GA_ID_STREAM.len())
        .map(|_| bus.io_read_byte(ga_port(0x1D, 1)))
        .collect();
    assert_eq!(probed, GA_ID_STREAM);

    for base in [0xD0u16, 0xD4, 0xDC, 0xE0, 0xE4, 0xE8, 0xEC] {
        let port = (0x1Du16 << 8) | (base + 1);
        assert_eq!(bus.io_read_byte(port), 0xFF);
    }
}

#[test]
fn gainit_auto_window_contract_matches_real_exe_candidates() {
    // GAINIT does not need to load before an EMS driver, but auto-window
    // detection needs one free 16 KB slot in C0000h-DFFFFh. A real setup can
    // load GABIOS.SYS early in CONFIG.SYS, before EMS claims the range.
    let mut occupied_blocks = [false; GAINIT_WINDOW_BLOCK_COUNT];

    assert_eq!(
        gainit_choose_window_segment(&occupied_blocks, 4),
        Some(0xC000)
    );

    occupied_blocks[..4].fill(true);
    assert_eq!(
        gainit_choose_window_segment(&occupied_blocks, 4),
        Some(0xC400)
    );

    occupied_blocks.fill(false);
    occupied_blocks[..16].fill(true);
    occupied_blocks[16..].fill(true);
    assert_eq!(
        gainit_choose_window_segment(&occupied_blocks, 4),
        None,
        "GAINIT should fail when EMS and UMB occupy every C000h-DFFFh candidate"
    );
}

#[test]
fn gainit_phase1_initialization_contract() {
    let mut bus = setup_bus();

    bus.io_write_word(ga_port(0x0F, 0), 0xAAAA);
    bus.io_write_word(ga_port(0x14, 2), 0xBBBB);
    bus.io_write_byte(ga_port(0x15, 2), 0xCC);
    apply_gainit_reset_lists(&mut bus);

    bus.io_write_word(ga_port(0x05, 0), 0xFFFF);
    bus.io_write_word(ga_port(0x03, 0), 0xFFFF);
    bus.io_write_byte(ga_port(0x07, 0), 0xFF);
    bus.io_write_word(ga_port(0x15, 0), 0x4000);
    bus.io_write_word(ga_port(0x16, 0), 0x20C1);
    bus.io_write_word(ga_port(0x1C, 0), 0x0003);
    bus.io_write_byte(ga_port(0x1C, 1), 0x00);
    bus.io_write_word(ga_port(0x12, 2), 0x03FF);
    bus.io_write_word(ga_port(0x13, 2), 0x07FF);
    bus.io_write_byte(ga_port(0x18, 0), 7);
    bus.io_write_byte(ga_port(0x1A, 0), 0x11);
    bus.io_write_byte(ga_port(0x1A, 0), 0x22);
    bus.io_write_byte(ga_port(0x1A, 0), 0x33);
    bus.io_write_byte(ga_port(0x1B, 0), 0xFF);
    bus.io_write_byte(ga_port(0x1E, 0), 0x38);
    bus.io_write_word(ga_port(0x1F, 0), 0x0003);

    let state = bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.gaport, GA_GAPORT);
    assert_eq!(state.unknown_sel_0f_off0, 0);
    assert_eq!(state.unknown_sel_14_off2, 0);
    assert_eq!(state.unknown_sel_15_off2, 0);
    assert_eq!(state.wbm, 0xFFFF);
    assert_eq!(state.wpm, 0xFFFF);
    assert_eq!(state.rpe, 0xFF);
    assert_eq!(state.cwb, 0x4000);
    assert_eq!(state.wba1, 0x20C1);
    assert_eq!(state.mod1, 0x00);
    assert_eq!(state.mod2, 0x00);
    assert_eq!(state.system_register, 0x0003);
    assert_eq!(state.system_auxiliary_register, 0x00);
    assert_eq!(state.pmw, 0x03FF);
    assert_eq!(state.pmh, 0x07FF);
    assert_eq!(state.palette[7], [0x11, 0x22, 0x33]);
    assert_eq!(state.vdac_mask, 0xFF);
    assert_eq!(state.crtc_registers[0x38], 0x0003);
    assert_eq!(state.pop1, 0x0000);
    assert_eq!(state.pop2, 0x0000);
    assert!(state.reset_unknown_write_count >= 3);
    assert!(state.wba1_write_count > 0);
    assert!(state.crtc_write_count > 0);
    assert!(state.ramdac_write_count > 0);
}

#[test]
fn gainit_word_io_reaches_ga_atomically() {
    let mut bus = setup_bus();
    let code = [
        0xBA, 0xD8, 0x05, // mov dx,05D8h
        0xB8, 0xEF, 0xBE, // mov ax,BEEFh
        0xEF, // out dx,ax
        0xB8, 0x00, 0x00, // mov ax,0
        0xED, // in ax,dx
        0xA3, 0x00, 0x05, // mov [0500h],ax
        0xF4, // hlt
    ];
    write_bytes(&mut bus, 0x0100, &code);

    let mut machine = Pc9801Ra::new(cpu::I386::new(), bus);
    machine.cpu.load_state(&{
        let mut state = cpu::I386State::default();
        state.set_cs(0);
        state.set_ds(0);
        state.set_ss(0);
        state.set_esp(0x8000);
        state.set_eip(0x0100);
        state
    });
    machine.run_for(10_000);

    assert!(machine.cpu.halted());
    assert_eq!(read_word_direct(&machine.bus, 0x0500), 0xBEEF);
    let state = machine.bus.ga1280a_state().expect("GA board installed");
    assert_eq!(state.wbm, 0xBEEF);
}
