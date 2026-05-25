use crate::harness;

fn boot_and_get_first_mcb() -> (machine::Pc9801Ra, u32) {
    let mut machine = harness::boot_hle();
    let sysvars = harness::get_sysvars_address(&mut machine);
    let mcb_segment = harness::read_word(&machine.bus, sysvars - 2);
    let mcb_linear = harness::far_to_linear(mcb_segment, 0);
    (machine, mcb_linear)
}

struct McbEntry {
    address: u32,
    block_type: u8,
    owner: u16,
    size: u16,
    name: [u8; 8],
}

fn walk_mcb_chain(bus: &machine::Pc9801Bus, first_mcb: u32) -> Vec<McbEntry> {
    let mut entries = Vec::new();
    let mut addr = first_mcb;

    for _ in 0..1000 {
        let block_type = harness::read_byte(bus, addr);
        let owner = harness::read_word(bus, addr + 1);
        let size = harness::read_word(bus, addr + 3);
        let mut name = [0u8; 8];
        for (i, byte) in name.iter_mut().enumerate() {
            *byte = harness::read_byte(bus, addr + 8 + i as u32);
        }

        entries.push(McbEntry {
            address: addr,
            block_type,
            owner,
            size,
            name,
        });

        if block_type == 0x5A {
            break;
        }
        if block_type != 0x4D {
            break;
        }

        // Next MCB: current segment + size + 1 paragraph (16 bytes each).
        let current_segment = addr >> 4;
        let next_segment = current_segment + size as u32 + 1;
        addr = next_segment << 4;

        if addr >= 0xA0000 {
            break;
        }
    }

    entries
}

#[test]
fn chain_well_formed() {
    let (machine, first_mcb) = boot_and_get_first_mcb();
    let entries = walk_mcb_chain(&machine.bus, first_mcb);

    assert!(
        !entries.is_empty(),
        "MCB chain should have at least one entry"
    );

    for (i, entry) in entries.iter().enumerate() {
        let is_last = i == entries.len() - 1;
        if is_last {
            assert_eq!(
                entry.block_type, 0x5A,
                "Last MCB at {:#010X} should be type 'Z' (0x5A), got {:#04X}",
                entry.address, entry.block_type
            );
        } else {
            assert_eq!(
                entry.block_type, 0x4D,
                "MCB #{} at {:#010X} should be type 'M' (0x4D), got {:#04X}",
                i, entry.address, entry.block_type
            );
        }
    }
}

#[test]
fn chain_has_dos_owned_blocks() {
    let (machine, first_mcb) = boot_and_get_first_mcb();
    let entries = walk_mcb_chain(&machine.bus, first_mcb);

    let dos_owned = entries.iter().any(|e| e.owner == 0x0008);
    assert!(
        dos_owned,
        "MCB chain should contain at least one DOS-owned block (owner=0x0008)"
    );
}

#[test]
fn chain_has_command_com() {
    let (machine, first_mcb) = boot_and_get_first_mcb();
    let entries = walk_mcb_chain(&machine.bus, first_mcb);

    let has_command = entries.iter().any(|e| {
        let name_str = String::from_utf8_lossy(&e.name);
        name_str.contains("COMMAND")
    });
    assert!(
        has_command,
        "MCB chain should contain a block owned by COMMAND.COM"
    );
}

#[test]
fn chain_has_free_block() {
    let (machine, first_mcb) = boot_and_get_first_mcb();
    let entries = walk_mcb_chain(&machine.bus, first_mcb);

    let has_free = entries.iter().any(|e| e.owner == 0x0000);
    assert!(
        has_free,
        "MCB chain should contain at least one free block (owner=0x0000)"
    );
}

#[test]
fn chain_sizes_consistent() {
    let (machine, first_mcb) = boot_and_get_first_mcb();
    let entries = walk_mcb_chain(&machine.bus, first_mcb);

    for i in 0..entries.len() - 1 {
        let current = &entries[i];
        let next = &entries[i + 1];
        let current_segment = current.address >> 4;
        let expected_next_segment = current_segment + current.size as u32 + 1;
        let actual_next_segment = next.address >> 4;
        assert_eq!(
            expected_next_segment, actual_next_segment,
            "MCB #{} at seg {:#06X} with size {} paragraphs: next MCB should be at seg {:#06X}, but found at {:#06X}",
            i, current_segment, current.size, expected_next_segment, actual_next_segment
        );
    }
}
