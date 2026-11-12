use crate::harness::*;

/// Number of MSCDEX drives of the built-in CD-ROM device.
const MSCDEX_DRIVE_COUNT: u16 = 1;
/// Drive index of the CD-ROM drive Q:.
const CDROM_DRIVE_INDEX: u16 = 16;
/// List of lists offset of the far pointer to the CDS array.
const LIST_OF_LISTS_CDS_POINTER: u32 = 0x16;
/// List of lists offset of the NUL device header.
const LIST_OF_LISTS_NUL_HEADER: u32 = 0x22;
/// Size of a DOS 3.10 CDS entry.
const CDS_ENTRY_SIZE: u32 = 0x51;
/// CDS entry offset of the flags word.
const CDS_FLAGS: u32 = 0x43;
/// CDS flag of a network drive.
const CDS_FLAG_NETWORK: u16 = 0x8000;
/// Device header offset of the strategy routine.
const DEVICE_HEADER_STRATEGY: u32 = 0x06;
/// Device header offset of the interrupt routine.
const DEVICE_HEADER_INTERRUPT: u32 = 0x08;
/// Offset of a far pointer that ends a chain.
const END_OF_CHAIN_OFFSET: u16 = 0xFFFF;
/// Upper bound of devices in the chain.
const MAXIMUM_DEVICE_COUNT: usize = 32;

#[test]
fn pc9821_boots_dos_310_with_the_cdrom_device() {
    let mut machine = boot_cdrom_machine();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0x15,       // MOV AX, 1500h
        0x31, 0xDB,             // XOR BX, BX
        0xCD, 0x2F,             // INT 2Fh
        0x89, 0x1E, 0x00, 0x01, // MOV [0100h], BX
        0x89, 0x0E, 0x02, 0x01, // MOV [0102h], CX
        0xFA,                   // CLI
        0xF4,                   // HLT
    ];
    inject_and_run(&mut machine, code);
    assert_eq!(result_word(&machine.bus, 0), MSCDEX_DRIVE_COUNT);
    assert_eq!(result_word(&machine.bus, 2), CDROM_DRIVE_INDEX);

    let list_of_lists = far_to_linear(list_of_lists_pointer(&mut machine));
    let bus = &machine.bus;
    let cds = far_to_linear(read_far_pointer(
        bus,
        list_of_lists + LIST_OF_LISTS_CDS_POINTER,
    )) + u32::from(CDROM_DRIVE_INDEX) * CDS_ENTRY_SIZE;
    assert_eq!(read_bytes(bus, cds, 3), b"Q:\\");
    assert_eq!(read_word(bus, cds + CDS_FLAGS), CDS_FLAG_NETWORK);

    let stubs = dos::DataArea::Pc98Dos310
        .layout()
        .self_boot
        .expect("the PC-98 3.10 data area has self boot regions");
    let mut device = read_far_pointer(bus, list_of_lists + LIST_OF_LISTS_NUL_HEADER);
    let mut cdrom_device = None;
    for _ in 0..MAXIMUM_DEVICE_COUNT {
        if device.1 == END_OF_CHAIN_OFFSET {
            break;
        }
        if device == (DOS_DATA_SEGMENT, stubs.cdrom_device) {
            cdrom_device = Some(device);
            break;
        }
        device = read_far_pointer(bus, far_to_linear(device));
    }
    let cdrom_device = far_to_linear(cdrom_device.expect("the chain holds the CD-ROM device"));
    assert_eq!(
        read_word(bus, cdrom_device + DEVICE_HEADER_STRATEGY),
        stubs.cdrom_stubs.strategy_stub
    );
    assert_eq!(
        read_word(bus, cdrom_device + DEVICE_HEADER_INTERRUPT),
        stubs.cdrom_stubs.interrupt_stub
    );
}
