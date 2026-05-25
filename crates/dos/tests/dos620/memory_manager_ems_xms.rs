use crate::harness;

#[test]
fn test_ems_status() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x40,                         // MOV AH, 40h (get status)
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ah = harness::result_byte(&machine.bus, 1);
    assert_eq!(ah, 0x00, "EMS status should be 0x00 (OK), got {:#04X}", ah);
}

#[test]
fn test_ems_page_frame_segment() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x41,                         // MOV AH, 41h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ah = harness::result_byte(&machine.bus, 1);
    let bx = harness::result_word(&machine.bus, 2);
    assert_eq!(ah, 0x00, "AH should be 0 (success), got {:#04X}", ah);
    assert_eq!(
        bx, 0xC000,
        "Page frame segment should be 0xC000, got {:#06X}",
        bx
    );
}

#[test]
fn test_ems_unallocated_page_count() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x42,                         // MOV AH, 42h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX (free pages)
        0x89, 0x16, 0x04, 0x01,             // MOV [0x0104], DX (total pages)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ah = harness::result_byte(&machine.bus, 1);
    let free = harness::result_word(&machine.bus, 2);
    let total = harness::result_word(&machine.bus, 4);
    assert_eq!(ah, 0x00);
    assert!(free > 0, "Free pages should be > 0, got {}", free);
    assert!(total > 0, "Total pages should be > 0, got {}", total);
    assert!(
        free <= total,
        "Free ({}) should be <= total ({})",
        free,
        total
    );
}

#[test]
fn test_ems_version() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x46,                         // MOV AH, 46h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ax = harness::result_word(&machine.bus, 0);
    assert_eq!(
        ax, 0x0040,
        "EMS version should be 0x0040 (4.0), got {:#06X}",
        ax
    );
}

#[test]
fn test_ems_allocate_and_deallocate() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate 1 page
        0xBB, 0x01, 0x00,                   // MOV BX, 1
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (status)
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX (handle)
        // Deallocate
        0x8B, 0x16, 0x02, 0x01,             // MOV DX, [0x0102]
        0xB4, 0x45,                         // MOV AH, 45h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x04, 0x01,                   // MOV [0x0104], AX (dealloc status)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let alloc_ah = harness::result_byte(&machine.bus, 1);
    let dealloc_ah = harness::result_byte(&machine.bus, 5);
    assert_eq!(
        alloc_ah, 0x00,
        "Allocate should succeed, got AH={:#04X}",
        alloc_ah
    );
    assert_eq!(
        dealloc_ah, 0x00,
        "Deallocate should succeed, got AH={:#04X}",
        dealloc_ah
    );
}

#[test]
fn test_ems_map_write_read() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate 1 page
        0xBB, 0x01, 0x00,                   // MOV BX, 1
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX (handle)
        // Map logical 0 to physical 0: AH=44h, AL=physical, BX=logical, DX=handle
        0xB8, 0x00, 0x44,                   // MOV AX, 4400h (AH=44, AL=0=phys page)
        0xBB, 0x00, 0x00,                   // MOV BX, 0 (logical page)
        0x8B, 0x16, 0x02, 0x01,             // MOV DX, [0x0102]
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x04, 0x01,                   // MOV [0x0104], AX (map status)
        // Write 0xAB to C000:0000
        0xB8, 0x00, 0xC0,                   // MOV AX, C000h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xC6, 0x06, 0x00, 0x00, 0xAB, // MOV BYTE ES:[0000], ABh
        // Read it back
        0x26, 0xA0, 0x00, 0x00,             // MOV AL, ES:[0000]
        0xA2, 0x06, 0x01,                   // MOV [0x0106], AL
        // Restore DS segment
        0xB8, 0x00, 0x20,                   // MOV AX, 2000h
        0x8E, 0xD8,                         // MOV DS, AX
        // Deallocate
        0x8B, 0x16, 0x02, 0x01,             // MOV DX, [0x0102]
        0xB4, 0x45,                         // MOV AH, 45h
        0xCD, 0x67,                         // INT 67h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let alloc_ah = harness::result_byte(&machine.bus, 1);
    let map_ah = harness::result_byte(&machine.bus, 5);
    let read_val = harness::result_byte(&machine.bus, 6);
    assert_eq!(alloc_ah, 0x00, "Allocate AH={:#04X}", alloc_ah);
    assert_eq!(map_ah, 0x00, "Map AH={:#04X}", map_ah);
    assert_eq!(
        read_val, 0xAB,
        "Read from page frame should be 0xAB, got {:#04X}",
        read_val
    );
}

#[test]
fn test_ems_same_logical_page_aliases_across_page_frame_slots() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate 1 page and save handle.
        0xBB, 0x01, 0x00,                   // MOV BX, 1
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        0x89, 0x16, 0x00, 0x01,             // MOV [0x0100], DX
        // Map logical 0 into physical slot 0.
        0xB8, 0x00, 0x44,                   // MOV AX, 4400h
        0xBB, 0x00, 0x00,                   // MOV BX, 0
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [0x0100]
        0xCD, 0x67,                         // INT 67h
        // Map the same logical 0 into physical slot 1.
        0xB8, 0x01, 0x44,                   // MOV AX, 4401h
        0xBB, 0x00, 0x00,                   // MOV BX, 0
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [0x0100]
        0xCD, 0x67,                         // INT 67h
        // Write through C000:0000.
        0xB8, 0x00, 0xC0,                   // MOV AX, C000h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xC6, 0x06, 0x00, 0x00, 0x6A, // MOV BYTE ES:[0000], 6Ah
        // Read through C400:0000 and store result.
        0xB8, 0x00, 0xC4,                   // MOV AX, C400h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xA0, 0x00, 0x00,             // MOV AL, ES:[0000]
        0xA2, 0x02, 0x01,                   // MOV [0x0102], AL
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let aliased_value = harness::result_byte(&machine.bus, 2);
    assert_eq!(
        aliased_value, 0x6A,
        "Both physical slots should alias the same EMS page, got {aliased_value:#04X}"
    );
}

#[test]
fn test_ems_map_all_four_pages() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        // Allocate 4 pages
        0xBB, 0x04, 0x00,                   // MOV BX, 4
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        0x89, 0x16, 0x00, 0x01,             // MOV [0x0100], DX (handle)
        // Map logical 0 to physical 0
        0xB8, 0x00, 0x44,                   // MOV AX, 4400h
        0xBB, 0x00, 0x00,                   // MOV BX, 0
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [handle]
        0xCD, 0x67,                         // INT 67h
        // Map logical 1 to physical 1
        0xB8, 0x01, 0x44,                   // MOV AX, 4401h
        0xBB, 0x01, 0x00,                   // MOV BX, 1
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [handle]
        0xCD, 0x67,                         // INT 67h
        // Map logical 2 to physical 2
        0xB8, 0x02, 0x44,                   // MOV AX, 4402h
        0xBB, 0x02, 0x00,                   // MOV BX, 2
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [handle]
        0xCD, 0x67,                         // INT 67h
        // Map logical 3 to physical 3
        0xB8, 0x03, 0x44,                   // MOV AX, 4403h
        0xBB, 0x03, 0x00,                   // MOV BX, 3
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [handle]
        0xCD, 0x67,                         // INT 67h
        // Write unique bytes to each page frame slot via ES
        0xB8, 0x00, 0xC0,                   // MOV AX, C000h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xC6, 0x06, 0x00, 0x00, 0x11, // ES:[0000] = 11h (slot 0)
        0xB8, 0x00, 0xC4,                   // MOV AX, C400h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xC6, 0x06, 0x00, 0x00, 0x22, // ES:[0000] = 22h (slot 1)
        0xB8, 0x00, 0xC8,                   // MOV AX, C800h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xC6, 0x06, 0x00, 0x00, 0x33, // ES:[0000] = 33h (slot 2)
        0xB8, 0x00, 0xCC,                   // MOV AX, CC00h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xC6, 0x06, 0x00, 0x00, 0x44, // ES:[0000] = 44h (slot 3)
        // Read them all back; store results in DS segment
        0xB8, 0x00, 0x20,                   // MOV AX, 2000h
        0x8E, 0xD8,                         // MOV DS, AX
        0xB8, 0x00, 0xC0,                   // MOV AX, C000h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xA0, 0x00, 0x00,             // MOV AL, ES:[0000]
        0xA2, 0x02, 0x01,                   // MOV [0x0102], AL
        0xB8, 0x00, 0xC4,                   // MOV AX, C400h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xA0, 0x00, 0x00,             // MOV AL, ES:[0000]
        0xA2, 0x03, 0x01,                   // MOV [0x0103], AL
        0xB8, 0x00, 0xC8,                   // MOV AX, C800h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xA0, 0x00, 0x00,             // MOV AL, ES:[0000]
        0xA2, 0x04, 0x01,                   // MOV [0x0104], AL
        0xB8, 0x00, 0xCC,                   // MOV AX, CC00h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xA0, 0x00, 0x00,             // MOV AL, ES:[0000]
        0xA2, 0x05, 0x01,                   // MOV [0x0105], AL
        // Deallocate
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [0x0100]
        0xB4, 0x45,                         // MOV AH, 45h
        0xCD, 0x67,                         // INT 67h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, &code);
    assert_eq!(harness::result_byte(&machine.bus, 2), 0x11, "Slot 0");
    assert_eq!(harness::result_byte(&machine.bus, 3), 0x22, "Slot 1");
    assert_eq!(harness::result_byte(&machine.bus, 4), 0x33, "Slot 2");
    assert_eq!(harness::result_byte(&machine.bus, 5), 0x44, "Slot 3");
}

#[test]
fn test_ems_get_handle_count() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate handle 1
        0xBB, 0x01, 0x00,                   // MOV BX, 1
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        0x89, 0x16, 0x06, 0x01,             // MOV [0x0106], DX (handle 1)
        // Allocate handle 2
        0xBB, 0x01, 0x00,                   // MOV BX, 1
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        0x89, 0x16, 0x08, 0x01,             // MOV [0x0108], DX (handle 2)
        // Get handle count
        0xB4, 0x4B,                         // MOV AH, 4Bh
        0xCD, 0x67,                         // INT 67h
        0x89, 0x1E, 0x00, 0x01,             // MOV [0x0100], BX (count)
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (status)
        // Deallocate both
        0x8B, 0x16, 0x06, 0x01,             // MOV DX, [0x0106]
        0xB4, 0x45,                         // MOV AH, 45h
        0xCD, 0x67,                         // INT 67h
        0x8B, 0x16, 0x08, 0x01,             // MOV DX, [0x0108]
        0xB4, 0x45,                         // MOV AH, 45h
        0xCD, 0x67,                         // INT 67h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let count = harness::result_word(&machine.bus, 0);
    let ah = harness::result_byte(&machine.bus, 3);
    assert_eq!(ah, 0x00);
    assert!(count >= 2, "Handle count should be >= 2, got {}", count);
}

#[test]
fn test_ems_get_handle_pages() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate 3 pages
        0xBB, 0x03, 0x00,                   // MOV BX, 3
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        0x89, 0x16, 0x04, 0x01,             // MOV [0x0104], DX (handle)
        // Get handle pages
        0x8B, 0x16, 0x04, 0x01,             // MOV DX, [handle]
        0xB4, 0x4C,                         // MOV AH, 4Ch
        0xCD, 0x67,                         // INT 67h
        0x89, 0x1E, 0x00, 0x01,             // MOV [0x0100], BX (pages)
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (status)
        // Deallocate
        0x8B, 0x16, 0x04, 0x01,             // MOV DX, [handle]
        0xB4, 0x45,                         // MOV AH, 45h
        0xCD, 0x67,                         // INT 67h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let pages = harness::result_word(&machine.bus, 0);
    let ah = harness::result_byte(&machine.bus, 3);
    assert_eq!(ah, 0x00);
    assert_eq!(pages, 3, "Handle should have 3 pages, got {}", pages);
}

#[test]
fn test_ems_save_and_restore() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        // Allocate 2 pages
        0xBB, 0x02, 0x00,                   // MOV BX, 2
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        0x89, 0x16, 0x00, 0x01,             // MOV [0x0100], DX (handle)
        // Map logical 0 to physical 0
        0xB8, 0x00, 0x44,                   // MOV AX, 4400h
        0xBB, 0x00, 0x00,                   // MOV BX, 0
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [handle]
        0xCD, 0x67,                         // INT 67h
        // Write 0xAA to page frame
        0xB8, 0x00, 0xC0,                   // MOV AX, C000h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xC6, 0x06, 0x00, 0x00, 0xAA, // ES:[0000] = AAh
        // Restore DS
        0xB8, 0x00, 0x20,                   // MOV AX, 2000h
        0x8E, 0xD8,                         // MOV DS, AX
        // Save page map (AH=47h)
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [handle]
        0xB4, 0x47,                         // MOV AH, 47h
        0xCD, 0x67,                         // INT 67h
        // Map logical 1 to physical 0 (replaces page 0)
        0xB8, 0x00, 0x44,                   // MOV AX, 4400h
        0xBB, 0x01, 0x00,                   // MOV BX, 1
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [handle]
        0xCD, 0x67,                         // INT 67h
        // Write 0xBB to page frame
        0xB8, 0x00, 0xC0,                   // MOV AX, C000h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xC6, 0x06, 0x00, 0x00, 0xBB, // ES:[0000] = BBh
        // Restore DS
        0xB8, 0x00, 0x20,                   // MOV AX, 2000h
        0x8E, 0xD8,                         // MOV DS, AX
        // Restore page map (AH=48h)
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [handle]
        0xB4, 0x48,                         // MOV AH, 48h
        0xCD, 0x67,                         // INT 67h
        // Read page frame byte
        0xB8, 0x00, 0xC0,                   // MOV AX, C000h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xA0, 0x00, 0x00,             // MOV AL, ES:[0000]
        // Store result before restoring DS (MOV AX clobbers AL)
        0xA2, 0x02, 0x01,                   // MOV [0x0102], AL
        0xB8, 0x00, 0x20,                   // MOV AX, 2000h
        0x8E, 0xD8,                         // MOV DS, AX
        // Deallocate
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [handle]
        0xB4, 0x45,                         // MOV AH, 45h
        0xCD, 0x67,                         // INT 67h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, &code);
    let val = harness::result_byte(&machine.bus, 2);
    assert_eq!(
        val, 0xAA,
        "After restore, page frame should have 0xAA, got {:#04X}",
        val
    );
}

#[test]
fn test_ems_reallocate() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate 2 pages
        0xBB, 0x02, 0x00,                   // MOV BX, 2
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        0x89, 0x16, 0x06, 0x01,             // MOV [0x0106], DX (handle)
        // Reallocate to 4 pages (AH=51h, BX=new_count, DX=handle)
        0xBB, 0x04, 0x00,                   // MOV BX, 4
        0x8B, 0x16, 0x06, 0x01,             // MOV DX, [handle]
        0xB4, 0x51,                         // MOV AH, 51h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (realloc status)
        // Get page count
        0x8B, 0x16, 0x06, 0x01,             // MOV DX, [handle]
        0xB4, 0x4C,                         // MOV AH, 4Ch
        0xCD, 0x67,                         // INT 67h
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX (page count)
        // Deallocate
        0x8B, 0x16, 0x06, 0x01,             // MOV DX, [handle]
        0xB4, 0x45,                         // MOV AH, 45h
        0xCD, 0x67,                         // INT 67h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let realloc_ah = harness::result_byte(&machine.bus, 1);
    let pages = harness::result_word(&machine.bus, 2);
    assert_eq!(realloc_ah, 0x00, "Reallocate AH={:#04X}", realloc_ah);
    assert_eq!(
        pages, 4,
        "After reallocate, pages should be 4, got {}",
        pages
    );
}

#[test]
fn test_ems_handle_name() {
    let mut machine = harness::boot_hle();
    let data_offset = 0x0200u16;
    let name_bytes = b"TESTEMS\0";
    harness::write_bytes(
        &mut machine.bus,
        harness::INJECT_CODE_BASE + data_offset as u32,
        name_bytes,
    );

    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        // Allocate 1 page
        0xBB, 0x01, 0x00,                   // MOV BX, 1
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        0x89, 0x16, 0x00, 0x01,             // MOV [0x0100], DX (handle)
        // Set handle name: AH=53h, AL=01h, DX=handle, DS:SI=name
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [handle]
        0xBE, data_offset as u8, (data_offset >> 8) as u8, // MOV SI, data_offset
        0xB8, 0x01, 0x53,                   // MOV AX, 5301h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (set status)
        // Get handle name: AH=53h, AL=00h, DX=handle, ES:DI=buffer
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [handle]
        0xBF, 0x08, 0x01,                   // MOV DI, 0x0108 (result+8)
        0xB8, 0x00, 0x53,                   // MOV AX, 5300h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x04, 0x01,                   // MOV [0x0104], AX (get status)
        // Deallocate
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [handle]
        0xB4, 0x45,                         // MOV AH, 45h
        0xCD, 0x67,                         // INT 67h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, &code);

    let set_ah = harness::result_byte(&machine.bus, 3);
    let get_ah = harness::result_byte(&machine.bus, 5);
    assert_eq!(set_ah, 0x00, "Set name AH={:#04X}", set_ah);
    assert_eq!(get_ah, 0x00, "Get name AH={:#04X}", get_ah);
    let got_name = harness::read_bytes(&machine.bus, harness::INJECT_RESULT_BASE + 8, 8);
    assert_eq!(&got_name, name_bytes, "Name mismatch");
}

#[test]
fn test_ems_allocate_raw_pages() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // AH=5Ah, AL=00h (allocate raw), BX=0 pages
        0xBB, 0x00, 0x00,                   // MOV BX, 0
        0xB8, 0x00, 0x5A,                   // MOV AX, 5A00h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX (handle)
        // Deallocate
        0x8B, 0x16, 0x02, 0x01,             // MOV DX, [handle]
        0xB4, 0x45,                         // MOV AH, 45h
        0xCD, 0x67,                         // INT 67h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ah = harness::result_byte(&machine.bus, 1);
    assert_eq!(ah, 0x00, "Allocate raw pages AH={:#04X}", ah);
}

#[test]
fn test_xms_version() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x00,                         // MOV AH, 00h (get version)
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (version)
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX (revision)
        0x89, 0x16, 0x04, 0x01,             // MOV [0x0104], DX (hma_exists)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let version = harness::result_word(&machine.bus, 0);
    let revision = harness::result_word(&machine.bus, 2);
    let hma = harness::result_word(&machine.bus, 4);
    assert_eq!(
        version, 0x0300,
        "XMS version should be 3.00, got {:#06X}",
        version
    );
    assert_eq!(
        revision, 0x0001,
        "XMS revision should be 0001, got {:#06X}",
        revision
    );
    assert_eq!(hma, 1, "HMA should exist (DX=1), got {}", hma);
}

#[test]
fn test_xms_entry_point_via_int2f() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x10, 0x43,                   // MOV AX, 4310h
        0xCD, 0x2F,                         // INT 2Fh
        0x8C, 0x06, 0x00, 0x01,             // MOV [0x0100], ES (entry segment)
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX (entry offset)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let seg = harness::result_word(&machine.bus, 0);
    let off = harness::result_word(&machine.bus, 2);
    assert_eq!(
        seg, 0x0200,
        "Entry segment should be 0x0200, got {:#06X}",
        seg
    );
    assert_eq!(
        off, 0x0D44,
        "Entry offset should be 0x0D44, got {:#06X}",
        off
    );
}

#[test]
fn test_xms_query_free() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x08,                         // MOV AH, 08h (query free)
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (largest)
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX (total)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let largest = harness::result_word(&machine.bus, 0);
    let total = harness::result_word(&machine.bus, 2);
    assert!(
        largest > 0,
        "Largest free block should be > 0, got {}",
        largest
    );
    assert!(total > 0, "Total free should be > 0, got {}", total);
    assert!(
        largest <= total,
        "Largest ({}) should be <= total ({})",
        largest,
        total
    );
}

#[test]
fn test_xms_allocate_and_free() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate 64 KB
        0xBA, 0x40, 0x00,                   // MOV DX, 64
        0xB4, 0x09,                         // MOV AH, 09h
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (1=success)
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX (handle)
        // Free
        0x8B, 0x16, 0x02, 0x01,             // MOV DX, [handle]
        0xB4, 0x0A,                         // MOV AH, 0Ah
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x04, 0x01,                   // MOV [0x0104], AX (1=success)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let alloc_ax = harness::result_word(&machine.bus, 0);
    let free_ax = harness::result_word(&machine.bus, 4);
    assert_eq!(alloc_ax, 1, "Allocate AX={}", alloc_ax);
    assert_eq!(free_ax, 1, "Free AX={}", free_ax);
}

#[test]
fn test_xms_allocate_decreases_free() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Query free before
        0xB4, 0x08,                         // MOV AH, 08h
        0xCD, 0xFE,                         // INT FEh
        0x89, 0x16, 0x00, 0x01,             // MOV [0x0100], DX (total before)
        // Allocate 100 KB
        0xBA, 0x64, 0x00,                   // MOV DX, 100
        0xB4, 0x09,                         // MOV AH, 09h
        0xCD, 0xFE,                         // INT FEh
        0x89, 0x16, 0x04, 0x01,             // MOV [0x0104], DX (handle)
        // Query free after
        0xB4, 0x08,                         // MOV AH, 08h
        0xCD, 0xFE,                         // INT FEh
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX (total after)
        // Free
        0x8B, 0x16, 0x04, 0x01,             // MOV DX, [handle]
        0xB4, 0x0A,                         // MOV AH, 0Ah
        0xCD, 0xFE,                         // INT FEh
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let before = harness::result_word(&machine.bus, 0);
    let after = harness::result_word(&machine.bus, 2);
    assert!(
        before > after,
        "Free should decrease: before={}, after={}",
        before,
        after
    );
    assert!(
        before - after >= 100,
        "Should decrease by at least 100 KB: before={}, after={}",
        before,
        after
    );
}

#[test]
fn test_xms_lock_and_unlock() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate 64 KB
        0xBA, 0x40, 0x00,                   // MOV DX, 64
        0xB4, 0x09,                         // MOV AH, 09h
        0xCD, 0xFE,                         // INT FEh
        0x89, 0x16, 0x08, 0x01,             // MOV [0x0108], DX (handle)
        // Lock: AH=0Ch, DX=handle
        0x8B, 0x16, 0x08, 0x01,             // MOV DX, [handle]
        0xB4, 0x0C,                         // MOV AH, 0Ch
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (1=success)
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX (addr high)
        0x89, 0x1E, 0x04, 0x01,             // MOV [0x0104], BX (addr low)
        // Unlock: AH=0Dh, DX=handle
        0x8B, 0x16, 0x08, 0x01,             // MOV DX, [handle]
        0xB4, 0x0D,                         // MOV AH, 0Dh
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x06, 0x01,                   // MOV [0x0106], AX (1=success)
        // Free
        0x8B, 0x16, 0x08, 0x01,             // MOV DX, [handle]
        0xB4, 0x0A,                         // MOV AH, 0Ah
        0xCD, 0xFE,                         // INT FEh
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let lock_ax = harness::result_word(&machine.bus, 0);
    let addr_high = harness::result_word(&machine.bus, 2);
    let addr_low = harness::result_word(&machine.bus, 4);
    let unlock_ax = harness::result_word(&machine.bus, 6);
    let addr = ((addr_high as u32) << 16) | addr_low as u32;
    assert_eq!(lock_ax, 1, "Lock AX={}", lock_ax);
    assert!(
        addr >= 0x100000,
        "Lock address should be >= 1MB, got {:#010X}",
        addr
    );
    assert_eq!(unlock_ax, 1, "Unlock AX={}", unlock_ax);
}

#[test]
fn test_xms_handle_info() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate 64 KB
        0xBA, 0x40, 0x00,                   // MOV DX, 64
        0xB4, 0x09,                         // MOV AH, 09h
        0xCD, 0xFE,                         // INT FEh
        0x89, 0x16, 0x06, 0x01,             // MOV [0x0106], DX (handle)
        // Handle info: AH=0Eh, DX=handle
        0x8B, 0x16, 0x06, 0x01,             // MOV DX, [handle]
        0xB4, 0x0E,                         // MOV AH, 0Eh
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (1=success)
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX (BH=lock, BL=free)
        0x89, 0x16, 0x04, 0x01,             // MOV [0x0104], DX (size_kb)
        // Free
        0x8B, 0x16, 0x06, 0x01,             // MOV DX, [handle]
        0xB4, 0x0A,                         // MOV AH, 0Ah
        0xCD, 0xFE,                         // INT FEh
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let info_ax = harness::result_word(&machine.bus, 0);
    let bx_val = harness::result_word(&machine.bus, 2);
    let size_kb = harness::result_word(&machine.bus, 4);
    let lock_count = (bx_val >> 8) as u8;
    assert_eq!(info_ax, 1, "Handle info AX={}", info_ax);
    assert_eq!(lock_count, 0, "Lock count should be 0, got {}", lock_count);
    assert_eq!(size_kb, 64, "Size should be 64 KB, got {}", size_kb);
}

#[test]
fn test_xms_reallocate() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate 64 KB
        0xBA, 0x40, 0x00,                   // MOV DX, 64
        0xB4, 0x09,                         // MOV AH, 09h
        0xCD, 0xFE,                         // INT FEh
        0x89, 0x16, 0x08, 0x01,             // MOV [0x0108], DX (handle)
        // Reallocate to 128 KB: AH=0Fh, BX=new_size, DX=handle
        0xBB, 0x80, 0x00,                   // MOV BX, 128
        0x8B, 0x16, 0x08, 0x01,             // MOV DX, [handle]
        0xB4, 0x0F,                         // MOV AH, 0Fh
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (1=success)
        // Verify with handle info
        0x8B, 0x16, 0x08, 0x01,             // MOV DX, [handle]
        0xB4, 0x0E,                         // MOV AH, 0Eh
        0xCD, 0xFE,                         // INT FEh
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX (size_kb)
        // Free
        0x8B, 0x16, 0x08, 0x01,             // MOV DX, [handle]
        0xB4, 0x0A,                         // MOV AH, 0Ah
        0xCD, 0xFE,                         // INT FEh
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let realloc_ax = harness::result_word(&machine.bus, 0);
    let size_kb = harness::result_word(&machine.bus, 2);
    assert_eq!(realloc_ax, 1, "Reallocate AX={}", realloc_ax);
    assert_eq!(
        size_kb, 128,
        "Size should be 128 KB after realloc, got {}",
        size_kb
    );
}

#[test]
fn test_xms_request_hma() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0xFF, 0xFF,                   // MOV DX, FFFFh
        0xB4, 0x01,                         // MOV AH, 01h (request HMA)
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        // Release HMA so other tests are not affected
        0xB4, 0x02,                         // MOV AH, 02h
        0xCD, 0xFE,                         // INT FEh
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ax = harness::result_word(&machine.bus, 0);
    assert_eq!(ax, 1, "Request HMA should succeed, AX={}", ax);
}

#[test]
fn test_xms_request_hma_zero_size_allowed_when_hmamin_is_zero() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x00, 0x00,                   // MOV DX, 0000h
        0xB4, 0x01,                         // MOV AH, 01h
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX
        0xB4, 0x02,                         // MOV AH, 02h
        0xCD, 0xFE,                         // INT FEh
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ax = harness::result_word(&machine.bus, 0);
    let bl = harness::result_byte(&machine.bus, 2);
    assert_eq!(ax, 1, "DX=0 Request HMA should succeed when HMAMIN is zero");
    assert_eq!(bl, 0, "BL should stay clear on successful DX=0 Request HMA");
}

#[test]
fn test_xms_request_hma_twice_fails() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // First request
        0xBA, 0xFF, 0xFF,                   // MOV DX, FFFFh
        0xB4, 0x01,                         // MOV AH, 01h
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        // Second request (should fail)
        0xBA, 0xFF, 0xFF,                   // MOV DX, FFFFh
        0xB4, 0x01,                         // MOV AH, 01h
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX
        0x89, 0x1E, 0x04, 0x01,             // MOV [0x0104], BX (BL=error)
        // Release so we clean up
        0xB4, 0x02,                         // MOV AH, 02h
        0xCD, 0xFE,                         // INT FEh
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let first_ax = harness::result_word(&machine.bus, 0);
    let second_ax = harness::result_word(&machine.bus, 2);
    let bl = harness::result_byte(&machine.bus, 4);
    assert_eq!(first_ax, 1, "First request should succeed, AX={}", first_ax);
    assert_eq!(second_ax, 0, "Second request should fail, AX={}", second_ax);
    assert_eq!(bl, 0x91, "Error code should be 0x91, got {:#04X}", bl);
}

#[test]
fn test_xms_release_hma() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Request HMA
        0xBA, 0xFF, 0xFF,                   // MOV DX, FFFFh
        0xB4, 0x01,                         // MOV AH, 01h
        0xCD, 0xFE,                         // INT FEh
        // Release HMA
        0xB4, 0x02,                         // MOV AH, 02h
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ax = harness::result_word(&machine.bus, 0);
    assert_eq!(ax, 1, "Release HMA should succeed, AX={}", ax);
}

#[test]
fn test_hma_query_reflects_allocation() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Query HMA before allocation
        0xB8, 0x01, 0x4A,                   // MOV AX, 4A01h
        0xCD, 0x2F,                         // INT 2Fh
        0x89, 0x1E, 0x00, 0x01,             // MOV [0x0100], BX (free before)
        // Request HMA
        0xBA, 0xFF, 0xFF,                   // MOV DX, FFFFh
        0xB4, 0x01,                         // MOV AH, 01h
        0xCD, 0xFE,                         // INT FEh
        // Query HMA after allocation
        0xB8, 0x01, 0x4A,                   // MOV AX, 4A01h
        0xCD, 0x2F,                         // INT 2Fh
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX (free after)
        // Release HMA
        0xB4, 0x02,                         // MOV AH, 02h
        0xCD, 0xFE,                         // INT FEh
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let before = harness::result_word(&machine.bus, 0);
    let after = harness::result_word(&machine.bus, 2);
    assert_eq!(
        before, 0xFFFF,
        "HMA should be free before, got {:#06X}",
        before
    );
    assert_eq!(
        after, 0x0000,
        "HMA should be allocated after, got {:#06X}",
        after
    );
}

#[test]
fn test_umb_allocate() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // UMB Allocate: AH=10h, DX=paragraphs
        0xBA, 0x10, 0x00,                   // MOV DX, 16
        0xB4, 0x10,                         // MOV AH, 10h
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (1=success)
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX (segment)
        0x89, 0x16, 0x04, 0x01,             // MOV [0x0104], DX (actual size)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ax = harness::result_word(&machine.bus, 0);
    let segment = harness::result_word(&machine.bus, 2);
    let size = harness::result_word(&machine.bus, 4);
    assert_eq!(ax, 1, "UMB allocate AX={}", ax);
    assert!(
        segment >= 0xD000,
        "UMB segment should be >= D000, got {:#06X}",
        segment
    );
    assert!(size >= 16, "UMB size should be >= 16, got {}", size);
}

#[test]
fn test_umb_allocate_and_free() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // UMB Allocate
        0xBA, 0x10, 0x00,                   // MOV DX, 16
        0xB4, 0x10,                         // MOV AH, 10h
        0xCD, 0xFE,                         // INT FEh
        0x89, 0x1E, 0x00, 0x01,             // MOV [0x0100], BX (segment)
        // UMB Free: AH=11h, DX=segment
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [segment]
        0xB4, 0x11,                         // MOV AH, 11h
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (1=success)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let free_ax = harness::result_word(&machine.bus, 2);
    assert_eq!(free_ax, 1, "UMB free AX={}", free_ax);
}

#[test]
fn test_umb_allocate_too_large() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0xFF, 0xFF,                   // MOV DX, FFFFh
        0xB4, 0x10,                         // MOV AH, 10h
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (0=fail)
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX (BL=error)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ax = harness::result_word(&machine.bus, 0);
    let bl = harness::result_byte(&machine.bus, 2);
    assert_eq!(ax, 0, "Should fail, AX={}", ax);
    assert_eq!(bl, 0xB0, "Error code should be 0xB0, got {:#04X}", bl);
}

#[test]
fn test_umb_reallocate() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate 32 paragraphs
        0xBA, 0x20, 0x00,                   // MOV DX, 32
        0xB4, 0x10,                         // MOV AH, 10h
        0xCD, 0xFE,                         // INT FEh
        0x89, 0x1E, 0x00, 0x01,             // MOV [0x0100], BX (segment)
        // Reallocate to 16: AH=12h, BX=new_size, DX=segment
        0xBB, 0x10, 0x00,                   // MOV BX, 16
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [segment]
        0xB4, 0x12,                         // MOV AH, 12h
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (1=success)
        // Free
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [segment]
        0xB4, 0x11,                         // MOV AH, 11h
        0xCD, 0xFE,                         // INT FEh
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let realloc_ax = harness::result_word(&machine.bus, 2);
    assert_eq!(realloc_ax, 1, "UMB reallocate AX={}", realloc_ax);
}

#[test]
fn test_ems_and_xms_coexist() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate EMS: 1 page
        0xBB, 0x01, 0x00,                   // MOV BX, 1
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (EMS status)
        0x89, 0x16, 0x06, 0x01,             // MOV [0x0106], DX (EMS handle)
        // Allocate XMS: 64 KB
        0xBA, 0x40, 0x00,                   // MOV DX, 64
        0xB4, 0x09,                         // MOV AH, 09h
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (XMS status)
        0x89, 0x16, 0x08, 0x01,             // MOV [0x0108], DX (XMS handle)
        // Free XMS
        0x8B, 0x16, 0x08, 0x01,             // MOV DX, [XMS handle]
        0xB4, 0x0A,                         // MOV AH, 0Ah
        0xCD, 0xFE,                         // INT FEh
        // Free EMS
        0x8B, 0x16, 0x06, 0x01,             // MOV DX, [EMS handle]
        0xB4, 0x45,                         // MOV AH, 45h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x04, 0x01,                   // MOV [0x0104], AX (dealloc status)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ems_ah = harness::result_byte(&machine.bus, 1);
    let xms_ax = harness::result_word(&machine.bus, 2);
    let dealloc_ah = harness::result_byte(&machine.bus, 5);
    assert_eq!(
        ems_ah, 0x00,
        "EMS allocate should succeed, AH={:#04X}",
        ems_ah
    );
    assert_eq!(xms_ax, 1, "XMS allocate should succeed, AX={}", xms_ax);
    assert_eq!(
        dealloc_ah, 0x00,
        "EMS deallocate should succeed, AH={:#04X}",
        dealloc_ah
    );
}

#[test]
fn test_ems_and_xms_share_pool() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate 700 EMS pages (11200 KB) to nearly exhaust the 12 MB pool
        0xBB, 0xBC, 0x02,                   // MOV BX, 700
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (EMS status)
        0x89, 0x16, 0x06, 0x01,             // MOV [0x0106], DX (EMS handle)
        // Try to allocate 4000 KB via XMS (more than the remaining ~1088 KB)
        0xBA, 0xA0, 0x0F,                   // MOV DX, 4000
        0xB4, 0x09,                         // MOV AH, 09h
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX (XMS result: 0=fail)
        // Free EMS
        0x8B, 0x16, 0x06, 0x01,             // MOV DX, [EMS handle]
        0xB4, 0x45,                         // MOV AH, 45h
        0xCD, 0x67,                         // INT 67h
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ems_ah = harness::result_byte(&machine.bus, 1);
    let xms_ax = harness::result_word(&machine.bus, 2);
    assert_eq!(
        ems_ah, 0x00,
        "EMS allocate 700 pages should succeed, AH={:#04X}",
        ems_ah
    );
    assert_eq!(
        xms_ax, 0,
        "XMS allocate 4000 KB should fail (pool nearly exhausted by EMS), AX={}",
        xms_ax
    );
}

#[test]
fn test_ems_5b00_get_alternate_map_register_set() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0x5B,                   // MOV AX, 5B00h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ah = harness::result_byte(&machine.bus, 1);
    let bl = harness::result_byte(&machine.bus, 2);
    assert_eq!(ah, 0x00, "AH should be 0 (success), got {:#04X}", ah);
    assert_eq!(bl, 0x00, "BL should be 0 (software mode), got {:#04X}", bl);
}

#[test]
fn test_ems_5b01_software_fallback_restores_page_map_from_pointer() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate 2 EMS pages.
        0xBB, 0x02, 0x00,                   // MOV BX, 2
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        0x89, 0x16, 0x00, 0x01,             // MOV [0x0100], DX
        // Map logical page 0 to physical page 0.
        0xB8, 0x00, 0x44,                   // MOV AX, 4400h
        0xBB, 0x00, 0x00,                   // MOV BX, 0
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [0x0100]
        0xCD, 0x67,                         // INT 67h
        // ES = DS for page-map buffers.
        0x8C, 0xD8,                         // MOV AX, DS
        0x8E, 0xC0,                         // MOV ES, AX
        // Save current page map to 0x0200 using Function 15.
        0xBF, 0x00, 0x02,                   // MOV DI, 0x0200
        0xB8, 0x00, 0x4E,                   // MOV AX, 4E00h
        0xCD, 0x67,                         // INT 67h
        // Change current map to logical page 1.
        0xB8, 0x00, 0x44,                   // MOV AX, 4400h
        0xBB, 0x01, 0x00,                   // MOV BX, 1
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [0x0100]
        0xCD, 0x67,                         // INT 67h
        // Restore via software alternate-map fallback from ES:DI=0x0200.
        0x8C, 0xD8,                         // MOV AX, DS
        0x8E, 0xC0,                         // MOV ES, AX
        0xBF, 0x00, 0x02,                   // MOV DI, 0x0200
        0xBB, 0x00, 0x00,                   // MOV BX, 0
        0xB8, 0x01, 0x5B,                   // MOV AX, 5B01h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX
        // Fetch current page map into 0x0210 for verification.
        0x8C, 0xD8,                         // MOV AX, DS
        0x8E, 0xC0,                         // MOV ES, AX
        0xBF, 0x10, 0x02,                   // MOV DI, 0x0210
        0xB8, 0x00, 0x4E,                   // MOV AX, 4E00h
        0xCD, 0x67,                         // INT 67h
        // Query alternate-map register set; should return saved pointer.
        0xB8, 0x00, 0x5B,                   // MOV AX, 5B00h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x04, 0x01,                   // MOV [0x0104], AX
        0x89, 0x1E, 0x06, 0x01,             // MOV [0x0106], BX
        0x8C, 0x06, 0x08, 0x01,             // MOV [0x0108], ES
        0x89, 0x3E, 0x0A, 0x01,             // MOV [0x010A], DI
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let handle = harness::result_word(&machine.bus, 0);
    let set_ah = harness::result_byte(&machine.bus, 3);
    let get_ah = harness::result_byte(&machine.bus, 5);
    let bl = harness::result_byte(&machine.bus, 6);
    let returned_seg = harness::result_word(&machine.bus, 8);
    let returned_off = harness::result_word(&machine.bus, 10);
    let current_handle = harness::read_word(&machine.bus, harness::INJECT_CODE_BASE + 0x0210);
    let current_page = harness::read_word(&machine.bus, harness::INJECT_CODE_BASE + 0x0212);

    assert_eq!(set_ah, 0x00, "5B01h should succeed, AH={:#04X}", set_ah);
    assert_eq!(get_ah, 0x00, "5B00h should succeed, AH={:#04X}", get_ah);
    assert_eq!(bl, 0x00, "BL should remain 0 in software fallback mode");
    assert_eq!(
        current_handle, handle,
        "restored map should target the original EMS handle"
    );
    assert_eq!(
        current_page, 0,
        "restored map should switch back to logical page 0"
    );
    assert_eq!(returned_seg, harness::INJECT_CODE_SEGMENT);
    assert_eq!(returned_off, 0x0200);
}

#[test]
fn test_ems_5b02_get_alternate_map_save_array_size() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x02, 0x5B,                   // MOV AX, 5B02h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ah = harness::result_byte(&machine.bus, 1);
    let size = harness::result_word(&machine.bus, 2);
    assert_eq!(ah, 0x00, "AH should be 0 (success), got {:#04X}", ah);
    assert_eq!(size, 16, "Save array size should match Function 15 size");
}

#[test]
fn test_ems_5b03_allocate_alternate_map_register_set() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x03, 0x5B,                   // MOV AX, 5B03h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ah = harness::result_byte(&machine.bus, 1);
    let bl = harness::result_byte(&machine.bus, 2);
    assert_eq!(ah, 0x00, "AH should be 0 (success), got {:#04X}", ah);
    assert_eq!(bl, 0x00, "BL should be 0 (software mode), got {:#04X}", bl);
}

#[test]
fn test_ems_5b01_set_alternate_map_register_set_invalid() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Try to set alternate register set 1 (not supported)
        0xBB, 0x01, 0x00,                   // MOV BX, 0001h (set 1)
        0xB8, 0x01, 0x5B,                   // MOV AX, 5B01h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ah = harness::result_byte(&machine.bus, 1);
    assert_eq!(
        ah, 0x9C,
        "AH should be 0x9C (not supported), got {:#04X}",
        ah
    );
}

#[test]
fn test_ems_5c_prepare_warm_boot() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x5C,                         // MOV AH, 5Ch
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ah = harness::result_byte(&machine.bus, 1);
    assert_eq!(ah, 0x00, "AH should be 0 (success), got {:#04X}", ah);
}

#[test]
fn test_ems_5d00_enable_os_function_set() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // First call: BX:CX = 0, should return access key
        0xBB, 0x00, 0x00,                   // MOV BX, 0
        0xB9, 0x00, 0x00,                   // MOV CX, 0
        0xB8, 0x00, 0x5D,                   // MOV AX, 5D00h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX (key high)
        0x89, 0x0E, 0x04, 0x01,             // MOV [0x0104], CX (key low)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ah = harness::result_byte(&machine.bus, 1);
    let key_hi = harness::result_word(&machine.bus, 2);
    let key_lo = harness::result_word(&machine.bus, 4);
    assert_eq!(ah, 0x00, "AH should be 0 (success), got {:#04X}", ah);
    let key = ((key_hi as u32) << 16) | key_lo as u32;
    assert_ne!(key, 0, "Access key should be non-zero");
}

#[test]
fn test_ems_5d01_disable_os_function_set() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Enable first to get key
        0xBB, 0x00, 0x00,                   // MOV BX, 0
        0xB9, 0x00, 0x00,                   // MOV CX, 0
        0xB8, 0x00, 0x5D,                   // MOV AX, 5D00h
        0xCD, 0x67,                         // INT 67h
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX (key high)
        0x89, 0x0E, 0x04, 0x01,             // MOV [0x0104], CX (key low)
        // Disable using returned key
        0x8B, 0x1E, 0x02, 0x01,             // MOV BX, [key high]
        0x8B, 0x0E, 0x04, 0x01,             // MOV CX, [key low]
        0xB8, 0x01, 0x5D,                   // MOV AX, 5D01h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ah = harness::result_byte(&machine.bus, 1);
    assert_eq!(ah, 0x00, "AH should be 0 (success), got {:#04X}", ah);
}

#[test]
fn test_ems_5d02_return_os_access_key() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Enable first to get key
        0xBB, 0x00, 0x00,                   // MOV BX, 0
        0xB9, 0x00, 0x00,                   // MOV CX, 0
        0xB8, 0x00, 0x5D,                   // MOV AX, 5D00h
        0xCD, 0x67,                         // INT 67h
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX (key high)
        0x89, 0x0E, 0x04, 0x01,             // MOV [0x0104], CX (key low)
        // Return key
        0x8B, 0x1E, 0x02, 0x01,             // MOV BX, [key high]
        0x8B, 0x0E, 0x04, 0x01,             // MOV CX, [key low]
        0xB8, 0x02, 0x5D,                   // MOV AX, 5D02h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ah = harness::result_byte(&machine.bus, 1);
    assert_eq!(ah, 0x00, "AH should be 0 (success), got {:#04X}", ah);
}

#[test]
fn test_ems_5900_reports_page_map_context_size() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x8C, 0xD8,                         // MOV AX, DS
        0x8E, 0xC0,                         // MOV ES, AX
        0xBF, 0x20, 0x01,                   // MOV DI, 0x0120
        0xB8, 0x00, 0x59,                   // MOV AX, 5900h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let ah = harness::result_byte(&machine.bus, 1);
    let raw_page_size = harness::read_word(&machine.bus, harness::INJECT_CODE_BASE + 0x0120);
    let context_size = harness::read_word(&machine.bus, harness::INJECT_CODE_BASE + 0x0124);

    assert_eq!(ah, 0x00, "5900h should succeed, AH={:#04X}", ah);
    assert_eq!(
        raw_page_size, 0x0400,
        "raw page size should be 16K in paragraphs"
    );
    assert_eq!(
        context_size, 16,
        "context save area size should match Function 15"
    );
}

#[test]
fn test_ems_5900_is_blocked_when_os_functions_disabled() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Get key.
        0xBB, 0x00, 0x00,
        0xB9, 0x00, 0x00,
        0xB8, 0x00, 0x5D,
        0xCD, 0x67,
        // Disable OS/E functions with returned key.
        0xB8, 0x01, 0x5D,
        0xCD, 0x67,
        // Attempt Function 26 while disabled.
        0x8C, 0xD8,                         // MOV AX, DS
        0x8E, 0xC0,                         // MOV ES, AX
        0xBF, 0x20, 0x01,                   // MOV DI, 0x0120
        0xB8, 0x00, 0x59,                   // MOV AX, 5900h
        0xCD, 0x67,
        0xA3, 0x00, 0x01,
        0xFA,
        0xF4,
    ];
    harness::inject_and_run(&mut machine, code);
    let ah = harness::result_byte(&machine.bus, 1);
    assert_eq!(
        ah, 0xA4,
        "5900h should be denied while OS/E functions are disabled"
    );
}

#[test]
fn test_ems_4e01_invalid_page_map_returns_a3() {
    let mut machine = harness::boot_hle();
    let map_offset = 0x0200u16;
    harness::write_bytes(
        &mut machine.bus,
        harness::INJECT_CODE_BASE + map_offset as u32,
        &0x1234u16.to_le_bytes(),
    );
    harness::write_bytes(
        &mut machine.bus,
        harness::INJECT_CODE_BASE + map_offset as u32 + 2,
        &0x0000u16.to_le_bytes(),
    );
    for slot in 1..4u32 {
        let addr = harness::INJECT_CODE_BASE + map_offset as u32 + slot * 4;
        harness::write_bytes(&mut machine.bus, addr, &0xFFFFu16.to_le_bytes());
        harness::write_bytes(&mut machine.bus, addr + 2, &0xFFFFu16.to_le_bytes());
    }

    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        0xBE, map_offset as u8, (map_offset >> 8) as u8, // MOV SI, map_offset
        0xB8, 0x01, 0x4E,                   // MOV AX, 4E01h
        0xCD, 0x67,
        0xA3, 0x00, 0x01,
        0xFA,
        0xF4,
    ];
    harness::inject_and_run(&mut machine, &code);

    let ah = harness::result_byte(&machine.bus, 1);
    assert_eq!(
        ah, 0xA3,
        "4E01h should reject invalid page-map entries with A3h"
    );
}

#[test]
fn test_xms_88_query_any_free_extended_memory() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x88,                         // MOV AH, 88h
        0xCD, 0xFE,                         // INT FEh
        // Store EAX (largest free KB) as dword at [0x0100]
        0x66, 0xA3, 0x00, 0x01,             // MOV [0x0100], EAX
        // Store EDX (total free KB) as dword at [0x0104]
        0x66, 0x89, 0x16, 0x04, 0x01,       // MOV [0x0104], EDX
        // Store ECX (highest address) as dword at [0x0108]
        0x66, 0x89, 0x0E, 0x08, 0x01,       // MOV [0x0108], ECX
        // Store BL (error code) at [0x010C]
        0x88, 0x1E, 0x0C, 0x01,             // MOV [0x010C], BL
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let largest = harness::result_dword(&machine.bus, 0);
    let total = harness::result_dword(&machine.bus, 4);
    let highest = harness::result_dword(&machine.bus, 8);
    let bl = harness::result_byte(&machine.bus, 0x0C);
    assert_eq!(bl, 0x00, "BL should be 0 (no error), got {:#04X}", bl);
    assert!(
        largest > 0,
        "Largest free block should be > 0, got {}",
        largest
    );
    assert!(total > 0, "Total free should be > 0, got {}", total);
    assert!(
        highest >= 0x100000,
        "Highest address should be >= 1MB, got {:#X}",
        highest
    );
}

#[test]
fn test_xms_89_allocate_any_extended_memory() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Set EDX = 64 (allocate 64 KB)
        0x66, 0xBA, 0x40, 0x00, 0x00, 0x00, // MOV EDX, 64
        0xB4, 0x89,                         // MOV AH, 89h
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (1=success)
        0x89, 0x16, 0x02, 0x01,             // MOV [0x0102], DX (handle)
        // Free the handle
        0x8B, 0x16, 0x02, 0x01,             // MOV DX, [handle]
        0xB4, 0x0A,                         // MOV AH, 0Ah
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x04, 0x01,                   // MOV [0x0104], AX (1=success)
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let alloc_ax = harness::result_word(&machine.bus, 0);
    let handle = harness::result_word(&machine.bus, 2);
    let free_ax = harness::result_word(&machine.bus, 4);
    assert_eq!(alloc_ax, 1, "Allocate should succeed, AX={}", alloc_ax);
    assert!(handle >= 1, "Handle should be >= 1, got {}", handle);
    assert_eq!(free_ax, 1, "Free should succeed, AX={}", free_ax);
}

#[test]
fn test_xms_8e_get_extended_handle_info() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate 64 KB via 32-bit function
        0x66, 0xBA, 0x40, 0x00, 0x00, 0x00, // MOV EDX, 64
        0xB4, 0x89,                         // MOV AH, 89h
        0xCD, 0xFE,                         // INT FEh
        0x89, 0x16, 0x06, 0x01,             // MOV [0x0106], DX (handle)
        // Query handle info via 0x8E
        0x8B, 0x16, 0x06, 0x01,             // MOV DX, [handle]
        0xB4, 0x8E,                         // MOV AH, 8Eh
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (1=success)
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX (BH=lock count)
        0x89, 0x0E, 0x04, 0x01,             // MOV [0x0104], CX (free handles)
        0x66, 0x89, 0x16, 0x08, 0x01,       // MOV [0x0108], EDX (size KB, 32-bit)
        // Clean up
        0x8B, 0x16, 0x06, 0x01,             // MOV DX, [handle]
        0xB4, 0x0A,                         // MOV AH, 0Ah
        0xCD, 0xFE,                         // INT FEh
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ax = harness::result_word(&machine.bus, 0);
    let bh = harness::result_byte(&machine.bus, 3);
    let free_handles = harness::result_word(&machine.bus, 4);
    let size_kb = harness::result_dword(&machine.bus, 8);
    assert_eq!(ax, 1, "Query should succeed, AX={}", ax);
    assert_eq!(bh, 0, "Lock count should be 0, got {}", bh);
    assert!(
        free_handles > 0,
        "Free handles should be > 0, got {}",
        free_handles
    );
    assert_eq!(size_kb, 64, "Size should be 64 KB, got {}", size_kb);
}

#[test]
fn test_xms_8f_reallocate_any_extended_memory() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate 64 KB via 32-bit function
        0x66, 0xBA, 0x40, 0x00, 0x00, 0x00, // MOV EDX, 64
        0xB4, 0x89,                         // MOV AH, 89h
        0xCD, 0xFE,                         // INT FEh
        0x89, 0x16, 0x06, 0x01,             // MOV [0x0106], DX (handle)
        // Reallocate to 128 KB via 0x8F (EBX = new size)
        0x66, 0xBB, 0x80, 0x00, 0x00, 0x00, // MOV EBX, 128
        0x8B, 0x16, 0x06, 0x01,             // MOV DX, [handle]
        0xB4, 0x8F,                         // MOV AH, 8Fh
        0xCD, 0xFE,                         // INT FEh
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX (1=success)
        // Verify new size via 0x8E
        0x8B, 0x16, 0x06, 0x01,             // MOV DX, [handle]
        0xB4, 0x8E,                         // MOV AH, 8Eh
        0xCD, 0xFE,                         // INT FEh
        0x66, 0x89, 0x16, 0x02, 0x01,       // MOV [0x0102], EDX (size KB)
        // Clean up
        0x8B, 0x16, 0x06, 0x01,             // MOV DX, [handle]
        0xB4, 0x0A,                         // MOV AH, 0Ah
        0xCD, 0xFE,                         // INT FEh
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let realloc_ax = harness::result_word(&machine.bus, 0);
    let new_size = harness::result_dword(&machine.bus, 2);
    assert_eq!(
        realloc_ax, 1,
        "Reallocate should succeed, AX={}",
        realloc_ax
    );
    assert_eq!(new_size, 128, "New size should be 128 KB, got {}", new_size);
}

/// Walks the DOS device driver chain starting at the NUL header and
/// returns each 8-byte device name found until the end-of-chain sentinel.
fn walk_device_chain_names(bus: &machine::Pc9801Bus) -> Vec<[u8; 8]> {
    const DEV_NUL_OFFSET: u16 = 0x0022;
    const DEVHDR_OFF_NEXT_PTR: u32 = 0x00;
    const DEVHDR_OFF_NAME: u32 = 0x0A;

    let mut names = Vec::new();
    let mut segment = 0x0200u16;
    let mut offset = DEV_NUL_OFFSET;
    for _ in 0..16 {
        let header_addr = ((segment as u32) << 4) + offset as u32;
        let mut name = [0u8; 8];
        for (i, b) in name.iter_mut().enumerate() {
            *b = bus.read_byte_direct(header_addr + DEVHDR_OFF_NAME + i as u32);
        }
        names.push(name);
        let (next_seg, next_off) = harness::read_far_ptr(bus, header_addr + DEVHDR_OFF_NEXT_PTR);
        if next_seg == 0xFFFF && next_off == 0xFFFF {
            return names;
        }
        segment = next_seg;
        offset = next_off;
        // Stay within the DOS data area to avoid runaway walks.
        assert_eq!(
            segment, 0x0200,
            "Device chain left DOS_DATA_SEGMENT; got {segment:#06X}"
        );
    }
    panic!("Device chain did not terminate within 16 hops");
}

#[test]
fn test_xmsxxxx0_device_in_chain_when_enabled() {
    let machine = harness::boot_hle();
    let names = walk_device_chain_names(&machine.bus);
    let found = names.iter().any(|n| n == b"XMSXXXX0");
    assert!(
        found,
        "XMSXXXX0 device missing from chain. Names seen: {:?}",
        names
            .iter()
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .collect::<Vec<_>>()
    );

    // Attribute must be DEVATTR_CHAR (0x8000).
    const DOS_DATA_BASE: u32 = 0x2000;
    const DEV_XMS_OFFSET: u16 = 0x0D4E;
    const DEVHDR_OFF_ATTRIBUTE: u32 = 0x04;
    let attr = harness::read_word(
        &machine.bus,
        DOS_DATA_BASE + DEV_XMS_OFFSET as u32 + DEVHDR_OFF_ATTRIBUTE,
    );
    assert_eq!(
        attr, 0x8000,
        "XMSXXXX0 attribute should be DEVATTR_CHAR (0x8000), got {attr:#06X}"
    );
}

#[test]
fn test_xmsxxxx0_absent_when_xms_disabled() {
    let machine = harness::boot_hle_without_xms();
    let names = walk_device_chain_names(&machine.bus);
    let found = names.iter().any(|n| n == b"XMSXXXX0");
    assert!(
        !found,
        "XMSXXXX0 should not be in the chain when XMS is disabled"
    );

    // With XMS disabled but EMS enabled (the default), CD-ROM's next_ptr
    // should link directly to EMMXXXX0 (skipping the absent XMS entry).
    const DOS_DATA_BASE: u32 = 0x2000;
    const DEV_CDROM_OFFSET: u16 = 0x007E;
    const DEV_EMS_OFFSET: u16 = 0x0D60;
    const DOS_DATA_SEGMENT: u16 = 0x0200;
    let (seg, off) = harness::read_far_ptr(&machine.bus, DOS_DATA_BASE + DEV_CDROM_OFFSET as u32);
    assert_eq!(
        (seg, off),
        (DOS_DATA_SEGMENT, DEV_EMS_OFFSET),
        "CD-ROM next_ptr should point to EMMXXXX0 when XMS disabled but EMS enabled, got {seg:#06X}:{off:#06X}"
    );
}

#[test]
fn test_cdrom_next_ptr_is_end_when_both_xms_and_ems_disabled() {
    let machine = harness::boot_hle_without_xms_and_ems();

    const DOS_DATA_BASE: u32 = 0x2000;
    const DEV_CDROM_OFFSET: u16 = 0x007E;
    let (seg, off) = harness::read_far_ptr(&machine.bus, DOS_DATA_BASE + DEV_CDROM_OFFSET as u32);
    assert_eq!(
        (seg, off),
        (0xFFFF, 0xFFFF),
        "CD-ROM next_ptr should be FFFF:FFFF when both XMS and EMS are disabled, got {seg:#06X}:{off:#06X}"
    );
}

#[test]
fn test_emmxxxx0_device_in_chain_when_enabled() {
    let machine = harness::boot_hle();
    let names = walk_device_chain_names(&machine.bus);
    let found = names.iter().any(|n| n == b"EMMXXXX0");
    assert!(
        found,
        "EMMXXXX0 device missing from chain. Names seen: {:?}",
        names
            .iter()
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .collect::<Vec<_>>()
    );

    const DOS_DATA_BASE: u32 = 0x2000;
    const DEV_EMS_OFFSET: u16 = 0x0D60;
    const DEVHDR_OFF_ATTRIBUTE: u32 = 0x04;
    let attr = harness::read_word(
        &machine.bus,
        DOS_DATA_BASE + DEV_EMS_OFFSET as u32 + DEVHDR_OFF_ATTRIBUTE,
    );
    assert_eq!(
        attr, 0x8000,
        "EMMXXXX0 attribute should be DEVATTR_CHAR (0x8000), got {attr:#06X}"
    );
}

#[test]
fn test_int67h_vector_points_at_emmxxxx0_header() {
    // Per EMS 4.0 Installation Check: reading the INT 67h vector with
    // MOV AX, 3567h; INT 21h returns ES:BX, and [ES:BX+000Ah] must
    // contain "EMMXXXX0" for applications to detect the EMS driver.
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x67, 0x35,                   // MOV AX, 3567h
        0xCD, 0x21,                         // INT 21h -> ES:BX = INT 67h vector
        0x06,                               // PUSH ES
        0x1F,                               // POP DS          (DS = ES)
        0x89, 0xDE,                         // MOV SI, BX
        0x81, 0xC6, 0x0A, 0x00,             // ADD SI, 000Ah   (DS:SI = name field)
        0x0E,                               // PUSH CS
        0x07,                               // POP ES          (ES = CS)
        0xBF, 0x00, 0x01,                   // MOV DI, 0100h   (ES:DI = result slot)
        0xB9, 0x08, 0x00,                   // MOV CX, 0008h
        0xFC,                               // CLD
        0xF3, 0xA4,                         // REP MOVSB
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let mut name = [0u8; 8];
    for (i, b) in name.iter_mut().enumerate() {
        *b = harness::result_byte(&machine.bus, i as u32);
    }
    assert_eq!(
        &name,
        b"EMMXXXX0",
        "INT 67h vector +0Ah should contain EMMXXXX0, got {:?}",
        String::from_utf8_lossy(&name)
    );
}

#[test]
fn test_int67h_stub_fires_hle_trap() {
    // Functional check: after setting up IVT[67h] to point at our stub,
    // invoking INT 67h AH=41h must reach the HLE handler and return
    // BX = 0xC000 (EMS page frame segment) with AH = 0 (success).
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x41,                         // MOV AH, 41h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ah = harness::result_byte(&machine.bus, 1);
    let bx = harness::result_word(&machine.bus, 2);
    assert_eq!(ah, 0x00, "INT 67h AH=41h should succeed, got AH={ah:#04X}");
    assert_eq!(
        bx, 0xC000,
        "INT 67h AH=41h should return BX=0xC000, got {bx:#06X}"
    );
}

#[test]
fn test_emmxxxx0_absent_when_ems_disabled() {
    let machine = harness::boot_hle_without_ems();
    let names = walk_device_chain_names(&machine.bus);
    let found = names.iter().any(|n| n == b"EMMXXXX0");
    assert!(
        !found,
        "EMMXXXX0 should not be in the chain when EMS is disabled"
    );
}

#[test]
fn test_ems_4f02_partial_page_map_size() {
    // AX=4F02h returns AL = bytes needed for a partial save array.
    // Spec: EMS 4.0 Function 16 sub 02. For N pages: size = 1 + 6*N.
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x02, 0x4F,                   // MOV AX, 4F02h
        0xBB, 0x02, 0x00,                   // MOV BX, 2
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ah = harness::result_byte(&machine.bus, 1);
    let al = harness::result_byte(&machine.bus, 0);
    assert_eq!(ah, 0x00, "AH should be 0 on success, got {ah:#04X}");
    assert_eq!(
        al,
        1 + 6 * 2,
        "Save size for 2 pages should be 13, got {al}"
    );
}

#[test]
fn test_ems_4f_partial_page_map_round_trip() {
    // Allocate 2 pages, map page 0 -> physical 0 (C000), then Get Partial
    // Page Map for segment C000. Remap to physical 1 (phys 1 = C400 segment,
    // but we use physical index 1 so we pass segment C400). Actually simpler:
    // save C000 mapping, remap C000 to a different logical page, then Set
    // the saved buffer to restore C000 to its original mapping.
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        // Allocate 2 pages
        0xBB, 0x02, 0x00,                   // MOV BX, 2
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        0x89, 0x16, 0x00, 0x01,             // MOV [0x0100], DX (handle)
        // Map logical 0 to physical 0
        0xB8, 0x00, 0x44,                   // MOV AX, 4400h
        0xBB, 0x00, 0x00,                   // MOV BX, 0
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [handle]
        0xCD, 0x67,                         // INT 67h
        // Write 0xAA to page frame 0
        0xB8, 0x00, 0xC0,                   // MOV AX, C000h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xC6, 0x06, 0x00, 0x00, 0xAA, // ES:[0000] = AAh
        0xB8, 0x00, 0x20,                   // MOV AX, 2000h
        0x8E, 0xD8,                         // MOV DS, AX
        // Build partial_page_map struct at DS:0x0200 { count=1, segs={C000} }
        0xC7, 0x06, 0x00, 0x02, 0x01, 0x00, // MOV [0x0200], 0001 (count)
        0xC7, 0x06, 0x02, 0x02, 0x00, 0xC0, // MOV [0x0202], C000 (segment)
        // Call Get Partial Page Map (AX=4F00, DS:SI=0x2000:0x0200, ES:DI=0x2000:0x0300)
        0xBE, 0x00, 0x02,                   // MOV SI, 0x0200
        0x8C, 0xD8,                         // MOV AX, DS           (AX = 0x2000)
        0x8E, 0xC0,                         // MOV ES, AX           (ES = 0x2000)
        0xBF, 0x00, 0x03,                   // MOV DI, 0x0300
        0xB8, 0x00, 0x4F,                   // MOV AX, 4F00h
        0xCD, 0x67,                         // INT 67h
        0x88, 0x26, 0x04, 0x01,             // MOV [0x0104], AH (status byte)
        // Map logical 1 to physical 0 (overwrites the mapping)
        0xB8, 0x00, 0x44,                   // MOV AX, 4400h
        0xBB, 0x01, 0x00,                   // MOV BX, 1
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [handle]
        0xCD, 0x67,                         // INT 67h
        // Write 0xBB to page frame (now mapped to page 1)
        0xB8, 0x00, 0xC0,                   // MOV AX, C000h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xC6, 0x06, 0x00, 0x00, 0xBB, // ES:[0000] = BBh
        0xB8, 0x00, 0x20,                   // MOV AX, 2000h
        0x8E, 0xD8,                         // MOV DS, AX
        // Call Set Partial Page Map (AX=4F01, DS:SI=0x2000:0x0300)
        0xBE, 0x00, 0x03,                   // MOV SI, 0x0300
        0xB8, 0x01, 0x4F,                   // MOV AX, 4F01h
        0xCD, 0x67,                         // INT 67h
        0x88, 0x26, 0x05, 0x01,             // MOV [0x0105], AH (status byte)
        // Read page frame byte 0 -- should be 0xAA again (logical 0 restored)
        0xB8, 0x00, 0xC0,                   // MOV AX, C000h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xA0, 0x00, 0x00,             // MOV AL, ES:[0000]
        0xA2, 0x03, 0x01,                   // MOV [0x0103], AL
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, &code);
    let status_get = harness::result_byte(&machine.bus, 4);
    let status_set = harness::result_byte(&machine.bus, 5);
    let val = harness::result_byte(&machine.bus, 3);
    assert_eq!(
        status_get, 0x00,
        "Get Partial status should be 0, got {status_get:#04X}"
    );
    assert_eq!(
        status_set, 0x00,
        "Set Partial status should be 0, got {status_set:#04X}"
    );
    assert_eq!(
        val, 0xAA,
        "After Set Partial Page Map restore, page frame byte should be 0xAA, got {val:#04X}"
    );
}

#[test]
fn test_ems_55_alter_page_map_and_jump() {
    // Allocate 2 pages, then INT 67h AH=55h with target pointing at our
    // "after-jump" code. Verify execution reaches the target.
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        // Allocate 2 pages
        0xBB, 0x02, 0x00,                   // MOV BX, 2
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        0x89, 0x16, 0x00, 0x01,             // MOV [0x0100], DX (handle)
        // Build map_and_jump struct at DS:0x0200:
        //   target_address = 0x2000:0x0080 (offset of 'reached' marker block)
        //   log_phys_map_len = 1
        //   log_phys_map_ptr = 0x2000:0x0210
        0xC7, 0x06, 0x00, 0x02, 0x80, 0x00, // [0x0200] = 0x0080 (target IP)
        0xC7, 0x06, 0x02, 0x02, 0x00, 0x20, // [0x0202] = 0x2000 (target CS)
        0xC6, 0x06, 0x04, 0x02, 0x01,       // [0x0204] = 1 (map_len)
        0xC7, 0x06, 0x05, 0x02, 0x10, 0x02, // [0x0205] = 0x0210 (map_ptr off)
        0xC7, 0x06, 0x07, 0x02, 0x00, 0x20, // [0x0207] = 0x2000 (map_ptr seg)
        // log_phys_map[0]: log_page=0, phys_page=0
        0xC7, 0x06, 0x10, 0x02, 0x00, 0x00, // [0x0210] = 0 (logical)
        0xC7, 0x06, 0x12, 0x02, 0x00, 0x00, // [0x0212] = 0 (physical)
        // INT 67h AH=55h AL=00h DX=handle DS:SI=0x2000:0x0200
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [handle]
        0xBE, 0x00, 0x02,                   // MOV SI, 0x0200
        0xB8, 0x00, 0x55,                   // MOV AX, 5500h
        0xCD, 0x67,                         // INT 67h
        // If we return here (fallthrough), write FF to marker; this should
        // NEVER execute because JMP transferred control to offset 0x0080.
        0xC6, 0x06, 0x20, 0x01, 0xFF,       // [0x0120] = FFh
        0xFA, 0xF4,                         // CLI / HLT (safety)
    ];
    // Pad to offset 0x80, then write "reached" marker and HLT.
    let mut code = code;
    while code.len() < 0x80 {
        code.push(0x90); // NOP
    }
    code.extend_from_slice(&[
        // Write 0xAB at [0x0120] to indicate we reached the target.
        0xC6, 0x06, 0x20, 0x01, 0xAB, // [0x0120] = ABh
        0xFA, 0xF4, // CLI / HLT
    ]);
    harness::inject_and_run(&mut machine, &code);
    let marker = harness::result_byte(&machine.bus, 0x20);
    assert_eq!(
        marker, 0xAB,
        "JMP target should have written 0xAB at marker, got {marker:#04X}"
    );
}

#[test]
fn test_ems_5602_get_page_map_stack_space_size() {
    // AX=5602h returns BX = number of bytes pushed on stack by 0x5600 (CALL).
    // EMS 4.0-compatible managers report a non-zero stack requirement.
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x02, 0x56,                   // MOV AX, 5602h
        0xCD, 0x67,                         // INT 67h
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let ah = harness::result_byte(&machine.bus, 1);
    let bx = harness::result_word(&machine.bus, 2);
    assert_eq!(ah, 0x00, "AH should be 0, got {ah:#04X}");
    assert_eq!(bx, 14, "Stack space size should be 14 bytes, got {bx}");
}

#[test]
fn test_ems_5600_alter_page_map_and_call_restores_old_map_on_retf() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        // Allocate two EMS pages and save handle.
        0xBB, 0x02, 0x00,                   // MOV BX, 2
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        0x89, 0x16, 0x00, 0x01,             // MOV [0x0100], DX
        // Map logical page 0 to physical page 0 and write 0xAA.
        0xB8, 0x00, 0x44,                   // MOV AX, 4400h
        0xBB, 0x00, 0x00,                   // MOV BX, 0
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [0x0100]
        0xCD, 0x67,                         // INT 67h
        0xB8, 0x00, 0xC0,                   // MOV AX, C000h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xC6, 0x06, 0x00, 0x00, 0xAA, // MOV byte ptr ES:[0000], 0xAA
        // Map logical page 1 to physical page 0 and write 0xBB.
        0xB8, 0x00, 0x44,                   // MOV AX, 4400h
        0xBB, 0x01, 0x00,                   // MOV BX, 1
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [0x0100]
        0xCD, 0x67,                         // INT 67h
        0xB8, 0x00, 0xC0,                   // MOV AX, C000h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xC6, 0x06, 0x00, 0x00, 0xBB, // MOV byte ptr ES:[0000], 0xBB
        // Restore logical page 0 before calling 5600h.
        0xB8, 0x00, 0x44,                   // MOV AX, 4400h
        0xBB, 0x00, 0x00,                   // MOV BX, 0
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [0x0100]
        0xCD, 0x67,                         // INT 67h
        // Build map_and_call_struct at 0x0200.
        0xC7, 0x06, 0x00, 0x02, 0x80, 0x01, // target IP = 0x0180
        0xC7, 0x06, 0x02, 0x02, 0x00, 0x20, // target CS = 0x2000
        0xC6, 0x06, 0x04, 0x02, 0x01,       // new len = 1
        0xC7, 0x06, 0x05, 0x02, 0x20, 0x02, // new ptr off = 0x0220
        0xC7, 0x06, 0x07, 0x02, 0x00, 0x20, // new ptr seg = 0x2000
        0xC6, 0x06, 0x09, 0x02, 0x01,       // old len = 1
        0xC7, 0x06, 0x0A, 0x02, 0x30, 0x02, // old ptr off = 0x0230
        0xC7, 0x06, 0x0C, 0x02, 0x00, 0x20, // old ptr seg = 0x2000
        // New mapping: logical page 1 -> physical page 0.
        0xC7, 0x06, 0x20, 0x02, 0x01, 0x00, // [0x0220] = 1
        0xC7, 0x06, 0x22, 0x02, 0x00, 0x00, // [0x0222] = 0
        // Old mapping: logical page 0 -> physical page 0.
        0xC7, 0x06, 0x30, 0x02, 0x00, 0x00, // [0x0230] = 0
        0xC7, 0x06, 0x32, 0x02, 0x00, 0x00, // [0x0232] = 0
        // Call 5600h.
        0x8B, 0x16, 0x00, 0x01,             // MOV DX, [0x0100]
        0xBE, 0x00, 0x02,                   // MOV SI, 0x0200
        0xB8, 0x00, 0x56,                   // MOV AX, 5600h
        0xCD, 0x67,                         // INT 67h
        // After RETF back from target, capture status and restored byte.
        0x88, 0x26, 0x21, 0x01,             // MOV [0x0121], AH
        0xB8, 0x00, 0xC0,                   // MOV AX, C000h
        0x8E, 0xC0,                         // MOV ES, AX
        0x26, 0xA0, 0x00, 0x00,             // MOV AL, ES:[0000]
        0xA2, 0x22, 0x01,                   // MOV [0x0122], AL
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    let mut code = code;
    while code.len() < 0x180 {
        code.push(0x90);
    }
    code.extend_from_slice(&[
        // Target: record the remapped page-frame byte, then RETF.
        0xB8, 0x00, 0xC0, // MOV AX, C000h
        0x8E, 0xC0, // MOV ES, AX
        0x26, 0xA0, 0x00, 0x00, // MOV AL, ES:[0000]
        0xA2, 0x20, 0x01, // MOV [0x0120], AL
        0xCB, // RETF
    ]);

    harness::inject_and_run(&mut machine, &code);

    let remapped_value = harness::result_byte(&machine.bus, 0x20);
    let return_status = harness::result_byte(&machine.bus, 0x21);
    let restored_value = harness::result_byte(&machine.bus, 0x22);

    assert_eq!(remapped_value, 0xBB, "target should see the new mapping");
    assert_eq!(return_status, 0x00, "5600h return restore should succeed");
    assert_eq!(
        restored_value, 0xAA,
        "caller should see the restored old mapping"
    );
}

#[test]
fn test_xms_a20_query_tracks_xms_visible_state() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Query before any enable: AX should be 0.
        0xB4, 0x07,
        0x9A, 0x44, 0x0D, 0x00, 0x02,       // CALL FAR 0200:0D44 (XMS entry stub)
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        // Local enable then query again: AX should be 1.
        0xB4, 0x05,
        0x9A, 0x44, 0x0D, 0x00, 0x02,
        0xB4, 0x07,
        0x9A, 0x44, 0x0D, 0x00, 0x02,
        0xA3, 0x02, 0x01,                   // MOV [0x0102], AX
        // Local disable then query again: AX should be 0.
        0xB4, 0x06,
        0x9A, 0x44, 0x0D, 0x00, 0x02,
        0xB4, 0x07,
        0x9A, 0x44, 0x0D, 0x00, 0x02,
        0xA3, 0x04, 0x01,                   // MOV [0x0104], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let initial = harness::result_word(&machine.bus, 0);
    let enabled = harness::result_word(&machine.bus, 2);
    let disabled = harness::result_word(&machine.bus, 4);
    assert_eq!(
        initial, 0,
        "Query A20 should report disabled before any XMS A20 enable, got {initial:#06X}"
    );
    assert_eq!(
        enabled, 1,
        "Query A20 should report enabled after Local Enable"
    );
    assert_eq!(
        disabled, 0,
        "Query A20 should report disabled after Local Disable, got {disabled:#06X}"
    );
}

#[test]
fn test_xms_a20_local_enable_nesting() {
    // Local Enable (05h) twice, Local Disable (06h) twice: both disables
    // should succeed. A third Local Disable must fail with BL=0x94.
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Local Enable x2
        0xB4, 0x05, 0x9A, 0x44, 0x0D, 0x00, 0x02,
        0xB4, 0x05, 0x9A, 0x44, 0x0D, 0x00, 0x02,
        // Local Disable x2 (both should succeed)
        0xB4, 0x06, 0x9A, 0x44, 0x0D, 0x00, 0x02,
        0xA3, 0x00, 0x01,                   // store first AX at [0x0100]
        0xB4, 0x06, 0x9A, 0x44, 0x0D, 0x00, 0x02,
        0xA3, 0x02, 0x01,                   // store second AX at [0x0102]
        // Third Local Disable (should fail AX=0, BL=0x94)
        0xB4, 0x06, 0x9A, 0x44, 0x0D, 0x00, 0x02,
        0xA3, 0x04, 0x01,                   // store AX at [0x0104]
        0x89, 0x1E, 0x06, 0x01,             // store BX at [0x0106]
        0xFA, 0xF4,
    ];
    harness::inject_and_run(&mut machine, code);
    let ax1 = harness::result_word(&machine.bus, 0);
    let ax2 = harness::result_word(&machine.bus, 2);
    let ax3 = harness::result_word(&machine.bus, 4);
    let bx3 = harness::result_word(&machine.bus, 6);
    assert_eq!(
        ax1, 1,
        "First Local Disable should succeed, got AX={ax1:#06X}"
    );
    assert_eq!(
        ax2, 1,
        "Second Local Disable should succeed, got AX={ax2:#06X}"
    );
    assert_eq!(ax3, 0, "Third Local Disable should fail, got AX={ax3:#06X}");
    assert_eq!(
        bx3 & 0x00FF,
        0x94,
        "Third Local Disable BL should be 0x94, got {:#04X}",
        bx3 & 0xFF
    );
}

#[test]
fn test_xms_a20_global_disable_blocked_by_local_enable() {
    // After a Local Enable, Global Disable must fail with BL=0x94 because
    // a local enable is still outstanding (XMS 3.0 A20 Management).
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Local Enable (05h)
        0xB4, 0x05, 0x9A, 0x44, 0x0D, 0x00, 0x02,
        // Global Disable (04h) - should fail AX=0, BL=0x94
        0xB4, 0x04, 0x9A, 0x44, 0x0D, 0x00, 0x02,
        0xA3, 0x00, 0x01,                   // AX -> [0x0100]
        0x89, 0x1E, 0x02, 0x01,             // BX -> [0x0102]
        0xFA, 0xF4,
    ];
    harness::inject_and_run(&mut machine, code);
    let ax = harness::result_word(&machine.bus, 0);
    let bx = harness::result_word(&machine.bus, 2);
    assert_eq!(
        ax, 0,
        "Global Disable should fail when local count > 0, got AX={ax:#06X}"
    );
    assert_eq!(
        bx & 0x00FF,
        0x94,
        "BL should be 0x94 (A20 still enabled), got {:#04X}",
        bx & 0xFF
    );
}

#[test]
fn test_xms_move_conventional_to_conventional_decodes_segoff() {
    // XMS 3.0 Move EMB: when SourceHandle/DestHandle is 0, the offset
    // DWORD is seg:offset (hi16 = segment, lo16 = offset), not a linear
    // address. Verify by copying "HELLOXMS" from DS:0200 to DS:0300.
    let mut machine = harness::boot_hle();
    // Move struct at DS:0x0100: length=8, src_handle=0, src_off=(2000:0200),
    // dst_handle=0, dst_off=(2000:0300). Total size 16 bytes.
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        // "HELLOXMS" -> DS:0200
        0xC7, 0x06, 0x00, 0x02, b'H', b'E',
        0xC7, 0x06, 0x02, 0x02, b'L', b'L',
        0xC7, 0x06, 0x04, 0x02, b'O', b'X',
        0xC7, 0x06, 0x06, 0x02, b'M', b'S',
        // Build move-params at DS:0x0110 (length=8, src_hdl=0,
        // src_off=0x02000200, dst_hdl=0, dst_off=0x02000300).
        0xC7, 0x06, 0x10, 0x01, 0x08, 0x00, // length_lo = 0x0008
        0xC7, 0x06, 0x12, 0x01, 0x00, 0x00, // length_hi = 0x0000
        0xC7, 0x06, 0x14, 0x01, 0x00, 0x00, // src_handle = 0
        0xC7, 0x06, 0x16, 0x01, 0x00, 0x02, // src_offset lo = 0x0200
        0xC7, 0x06, 0x18, 0x01, 0x00, 0x20, // src_offset hi = 0x2000 (segment)
        0xC7, 0x06, 0x1A, 0x01, 0x00, 0x00, // dst_handle = 0
        0xC7, 0x06, 0x1C, 0x01, 0x00, 0x03, // dst_offset lo = 0x0300
        0xC7, 0x06, 0x1E, 0x01, 0x00, 0x20, // dst_offset hi = 0x2000
        // Call XMS Move EMB (AH=0Bh, DS:SI -> params)
        0xBE, 0x10, 0x01,                   // MOV SI, 0x0110
        0xB4, 0x0B,                         // MOV AH, 0Bh
        0x9A, 0x44, 0x0D, 0x00, 0x02,       // CALL FAR 0200:0D44 (XMS entry stub)
        0xA3, 0x00, 0x04,                   // MOV [0x0400], AX (move status)
        0xFA, 0xF4,
    ];
    harness::inject_and_run(&mut machine, &code);
    let ax = harness::result_word(&machine.bus, 0x0300);
    assert_eq!(ax, 1, "Move should succeed, got AX={ax:#06X}");

    // Read destination bytes at linear 0x20000 + 0x0300.
    let mut bytes = [0u8; 8];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = harness::result_byte(&machine.bus, 0x0200 + i as u32);
    }
    assert_eq!(
        &bytes,
        b"HELLOXMS",
        "Destination should contain HELLOXMS, got {:?}",
        String::from_utf8_lossy(&bytes)
    );
}

#[test]
fn test_xms_move_rejects_odd_length() {
    // XMS 3.0: odd length returns BL=0xA7 (invalid length). Our move is a
    // no-op on length=0; length=3 should trip the check.
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: Vec<u8> = vec![
        // Build move-params at DS:0x0110 with odd length=3.
        0xC7, 0x06, 0x10, 0x01, 0x03, 0x00, // length_lo = 3
        0xC7, 0x06, 0x12, 0x01, 0x00, 0x00, // length_hi = 0
        0xC7, 0x06, 0x14, 0x01, 0x00, 0x00, // src_handle = 0
        0xC7, 0x06, 0x16, 0x01, 0x00, 0x02, // src_offset lo
        0xC7, 0x06, 0x18, 0x01, 0x00, 0x20, // src_offset hi
        0xC7, 0x06, 0x1A, 0x01, 0x00, 0x00, // dst_handle = 0
        0xC7, 0x06, 0x1C, 0x01, 0x00, 0x03, // dst_offset lo
        0xC7, 0x06, 0x1E, 0x01, 0x00, 0x20, // dst_offset hi
        0xBE, 0x10, 0x01,                   // MOV SI, 0x0110
        0xB4, 0x0B,                         // MOV AH, 0Bh
        0x9A, 0x44, 0x0D, 0x00, 0x02,       // XMS entry
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX
        0xFA, 0xF4,
    ];
    harness::inject_and_run(&mut machine, &code);
    let ax = harness::result_word(&machine.bus, 0);
    let bx = harness::result_word(&machine.bus, 2);
    assert_eq!(ax, 0, "Odd-length move should fail, got AX={ax:#06X}");
    assert_eq!(
        bx & 0x00FF,
        0xA7,
        "BL should be 0xA7 (invalid length), got {:#04X}",
        bx & 0xFF
    );
}

#[test]
fn test_xms_request_hma_respects_hmamin() {
    // /HMAMIN=32 means Request HMA with DX < 32*1024 is rejected with
    // BL=0x92. A subsequent request with DX >= 32KB succeeds.
    let mut machine = harness::boot_hle_with_hmamin_kb(32);
    #[rustfmt::skip]
    let code: &[u8] = &[
        // First try: DX = 16 (16 bytes, well below 32KB).
        0xB4, 0x01,                         // MOV AH, 01h
        0xBA, 0x10, 0x00,                   // MOV DX, 16
        0x9A, 0x44, 0x0D, 0x00, 0x02,       // CALL FAR XMS entry
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0x89, 0x1E, 0x02, 0x01,             // MOV [0x0102], BX
        // Second try: DX = 0xFFFF (applications ask for the full HMA).
        0xB4, 0x01,                         // MOV AH, 01h
        0xBA, 0xFF, 0xFF,                   // MOV DX, 0xFFFF
        0x9A, 0x44, 0x0D, 0x00, 0x02,       // CALL FAR XMS entry
        0xA3, 0x04, 0x01,                   // MOV [0x0104], AX
        0xFA, 0xF4,
    ];
    harness::inject_and_run(&mut machine, code);
    let ax_small = harness::result_word(&machine.bus, 0);
    let bx_small = harness::result_word(&machine.bus, 2);
    let ax_big = harness::result_word(&machine.bus, 4);
    assert_eq!(
        ax_small, 0,
        "Request HMA with DX<HMAMIN should fail, got AX={ax_small:#06X}"
    );
    assert_eq!(
        bx_small & 0x00FF,
        0x92,
        "BL should be 0x92, got {:#04X}",
        bx_small & 0xFF
    );
    assert_eq!(
        ax_big, 1,
        "Request HMA with DX>=HMAMIN should succeed, got AX={ax_big:#06X}"
    );
}

#[test]
fn test_xmsxxxx0_strategy_stub_sets_done_bit() {
    let mut machine = harness::boot_hle();
    // Fake request header at CS:0x0200 (ES = CS = INJECT_CODE_SEGMENT by default).
    // Call far ptr 0x0200:0x0D47 (XMS_DEV_STUB). Verify status word at +3 has 0x0100 set.
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBB, 0x00, 0x02,                   // MOV BX, 0x0200 (request header offset)
        0x31, 0xC0,                         // XOR AX, AX
        0x89, 0x47, 0x03,                   // MOV [BX+3], AX (zero status)
        0x9A, 0x47, 0x0D, 0x00, 0x02,       // CALL FAR 0x0200:0x0D47 (stub)
        0x8B, 0x47, 0x03,                   // MOV AX, [BX+3]
        0xA3, 0x00, 0x01,                   // MOV [0x0100], AX
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);
    let status = harness::result_word(&machine.bus, 0);
    assert_eq!(
        status & 0x0100,
        0x0100,
        "XMSXXXX0 stub should set DONE bit (0x0100) in status word, got {status:#06X}"
    );
    assert_eq!(
        status & 0x8000,
        0x0000,
        "XMSXXXX0 stub should not set ERROR bit (0x8000), got {status:#06X}"
    );
}
