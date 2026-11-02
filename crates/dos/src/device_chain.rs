//! The device driver chain behind the NUL header.

use crate::{END_OF_CHAIN, MemoryAccess, guest_call::FarPointer, tables};

/// Largest number of devices that a chain walk visits.
const DEVICE_CHAIN_LIMIT: usize = 256;

/// Returns the first device of the chain that starts at the NUL header
/// `nul` for which `matches` returns true. `matches` gets the header and its
/// attribute word.
fn find_device(
    memory: &dyn MemoryAccess,
    nul: FarPointer,
    mut matches: impl FnMut(FarPointer, u16) -> bool,
) -> Option<FarPointer> {
    let mut device = nul;
    for _ in 0..DEVICE_CHAIN_LIMIT {
        let header = device.linear();
        let attribute = memory.read_word(header + tables::DEVHDR_OFF_ATTRIBUTE);
        if matches(device, attribute) {
            return Some(device);
        }
        let next = FarPointer::read(memory, header + tables::DEVHDR_OFF_NEXT_PTR);
        if (next.segment, next.offset) == END_OF_CHAIN {
            return None;
        }
        device = next;
    }
    None
}

/// Returns the character device named `name` in the device chain that
/// starts at the NUL header `nul`.
pub(crate) fn find_character_device(
    memory: &dyn MemoryAccess,
    nul: FarPointer,
    name: &[u8; 8],
) -> Option<FarPointer> {
    find_device(memory, nul, |device, attribute| {
        let mut device_name = [0u8; 8];
        memory.read_block(device.linear() + tables::DEVHDR_OFF_NAME, &mut device_name);
        device_name.make_ascii_uppercase();
        attribute & tables::DEVATTR_CHAR != 0 && device_name == *name
    })
}

/// Returns true when `header` is a block device of the device chain that
/// starts at the NUL header `nul`.
pub(crate) fn is_chain_block_device(
    memory: &dyn MemoryAccess,
    nul: FarPointer,
    header: FarPointer,
) -> bool {
    find_device(memory, nul, |device, attribute| {
        attribute & tables::DEVATTR_CHAR == 0 && device.linear() == header.linear()
    })
    .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::MockMemory;

    #[test]
    fn walk_finds_a_device_behind_nul_and_stops_at_the_end() {
        let mut memory = MockMemory::with_extended_memory(0x10_0000, 0);
        let nul = FarPointer::new(0x1000, 0x0048);
        tables::write_device_header(
            &mut memory,
            nul.linear(),
            0x2000,
            0x0000,
            0x8004,
            b"NUL     ",
        );
        tables::write_device_header(&mut memory, 0x2_0000, 0x2000, 0x0012, 0x8013, b"CON     ");
        tables::write_device_header(
            &mut memory,
            0x2_0012,
            0xFFFF,
            0xFFFF,
            0x0800,
            b"\x03\0\0\0\0\0\0\0",
        );
        assert_eq!(
            find_character_device(&memory, nul, b"CON     "),
            Some(FarPointer::new(0x2000, 0x0000))
        );
        assert_eq!(find_character_device(&memory, nul, b"AUX     "), None);
        assert!(is_chain_block_device(
            &memory,
            nul,
            FarPointer::new(0x2000, 0x0012)
        ));
        assert!(!is_chain_block_device(
            &memory,
            nul,
            FarPointer::new(0x2000, 0x0000)
        ));
        assert!(!is_chain_block_device(
            &memory,
            nul,
            FarPointer::new(0x0000, 0x0000)
        ));
    }
}
