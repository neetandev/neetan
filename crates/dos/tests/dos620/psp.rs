use crate::harness;

fn boot_and_get_psp() -> (machine::Pc9801Ra, u32) {
    let mut machine = harness::boot_hle();
    let psp_segment = harness::get_psp_segment(&mut machine);
    let psp_linear = harness::far_to_linear(psp_segment, 0);
    (machine, psp_linear)
}

#[test]
fn int20h_instruction_at_offset_0() {
    let (machine, psp) = boot_and_get_psp();
    let byte0 = harness::read_byte(&machine.bus, psp);
    let byte1 = harness::read_byte(&machine.bus, psp + 1);
    assert_eq!(
        byte0, 0xCD,
        "PSP+0x00 should be 0xCD (INT opcode), got {:#04X}",
        byte0
    );
    assert_eq!(
        byte1, 0x20,
        "PSP+0x01 should be 0x20 (INT 20h vector), got {:#04X}",
        byte1
    );
}

#[test]
fn handle_table_populated() {
    let (machine, psp) = boot_and_get_psp();
    // Job File Table at PSP+0x18, 20 bytes. Handles 0-4 should be open (not 0xFF).
    for handle in 0..5u32 {
        let entry = harness::read_byte(&machine.bus, psp + 0x18 + handle);
        assert_ne!(
            entry, 0xFF,
            "PSP handle {} should be open (not 0xFF), got {:#04X}",
            handle, entry
        );
    }
}

#[test]
fn environment_segment_valid() {
    let (machine, psp) = boot_and_get_psp();
    let env_segment = harness::read_word(&machine.bus, psp + 0x2C);
    assert_ne!(
        env_segment, 0x0000,
        "PSP environment segment should be non-zero"
    );
    let env_linear = harness::far_to_linear(env_segment, 0);
    assert!(
        env_linear < 0xA0000,
        "Environment segment should be in conventional memory, got {:#010X}",
        env_linear
    );
}

#[test]
fn handle_table_size_default_20() {
    let (machine, psp) = boot_and_get_psp();
    let table_size = harness::read_word(&machine.bus, psp + 0x32);
    assert_eq!(
        table_size, 20,
        "Default handle table size should be 20, got {}",
        table_size
    );
}

#[test]
fn far_call_stub_at_05h() {
    let (machine, psp) = boot_and_get_psp();
    let opcode = harness::read_byte(&machine.bus, psp + 0x05);
    assert_eq!(
        opcode, 0x9A,
        "PSP+05h should be 0x9A (CALL FAR opcode), got {:#04X}",
        opcode
    );
}

#[test]
fn saved_int22h_vector() {
    let (machine, psp) = boot_and_get_psp();
    let (segment, offset) = harness::read_far_ptr(&machine.bus, psp + 0x0A);
    let linear = harness::far_to_linear(segment, offset);
    assert_ne!(linear, 0, "PSP+0Ah saved INT 22h vector should be non-zero");
    assert!(
        linear < 0x100000,
        "PSP+0Ah saved INT 22h vector should be in addressable memory, got {:#010X}",
        linear
    );
}

#[test]
fn saved_int23h_vector() {
    let (machine, psp) = boot_and_get_psp();
    let (segment, offset) = harness::read_far_ptr(&machine.bus, psp + 0x0E);
    let linear = harness::far_to_linear(segment, offset);
    assert_ne!(linear, 0, "PSP+0Eh saved INT 23h vector should be non-zero");
    assert!(
        linear < 0x100000,
        "PSP+0Eh saved INT 23h vector should be in addressable memory, got {:#010X}",
        linear
    );
}

#[test]
fn saved_int24h_vector() {
    let (machine, psp) = boot_and_get_psp();
    let (segment, offset) = harness::read_far_ptr(&machine.bus, psp + 0x12);
    let linear = harness::far_to_linear(segment, offset);
    assert_ne!(linear, 0, "PSP+12h saved INT 24h vector should be non-zero");
    assert!(
        linear < 0x100000,
        "PSP+12h saved INT 24h vector should be in addressable memory, got {:#010X}",
        linear
    );
}

#[test]
fn parent_psp_segment() {
    let (machine, psp) = boot_and_get_psp();
    let parent_seg = harness::read_word(&machine.bus, psp + 0x16);
    assert_ne!(
        parent_seg, 0x0000,
        "PSP+16h parent PSP segment should be non-zero"
    );
    let parent_linear = harness::far_to_linear(parent_seg, 0);
    assert!(
        parent_linear < 0xA0000,
        "Parent PSP should be in conventional memory, got {:#010X}",
        parent_linear
    );

    // For COMMAND.COM, parent PSP should equal its own PSP.
    let own_seg = (psp >> 4) as u16;
    assert_eq!(
        parent_seg, own_seg,
        "COMMAND.COM's parent PSP ({:#06X}) should equal own PSP ({:#06X})",
        parent_seg, own_seg
    );
}

#[test]
fn handle_table_pointer() {
    let (machine, psp) = boot_and_get_psp();
    let (segment, offset) = harness::read_far_ptr(&machine.bus, psp + 0x34);
    let psp_segment = (psp >> 4) as u16;
    // Default handle table pointer should point to PSP+0x18.
    assert_eq!(
        segment, psp_segment,
        "Handle table pointer segment ({:#06X}) should equal PSP segment ({:#06X})",
        segment, psp_segment
    );
    assert_eq!(
        offset, 0x0018,
        "Handle table pointer offset should be 0x0018, got {:#06X}",
        offset
    );
}

#[test]
fn int21h_retf_stub_at_50h() {
    let (machine, psp) = boot_and_get_psp();
    let byte0 = harness::read_byte(&machine.bus, psp + 0x50);
    let byte1 = harness::read_byte(&machine.bus, psp + 0x51);
    let byte2 = harness::read_byte(&machine.bus, psp + 0x52);
    assert_eq!(
        byte0, 0xCD,
        "PSP+50h should be 0xCD (INT opcode), got {:#04X}",
        byte0
    );
    assert_eq!(
        byte1, 0x21,
        "PSP+51h should be 0x21 (INT 21h), got {:#04X}",
        byte1
    );
    assert_eq!(
        byte2, 0xCB,
        "PSP+52h should be 0xCB (RETF), got {:#04X}",
        byte2
    );
}

#[test]
fn command_tail_at_80h() {
    let (machine, psp) = boot_and_get_psp();
    let length = harness::read_byte(&machine.bus, psp + 0x80);
    assert!(
        length <= 127,
        "Command tail length should be <= 127, got {}",
        length
    );

    if length > 0 {
        let terminator = harness::read_byte(&machine.bus, psp + 0x81 + length as u32);
        assert_eq!(
            terminator, 0x0D,
            "Command tail should be terminated by 0x0D (CR), got {:#04X}",
            terminator
        );
    }
}
