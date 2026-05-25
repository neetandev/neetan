use common::{CpuMode, MachineModel};

use crate::harness;

fn find_line<'a>(lines: &'a [String], prefix: &str) -> &'a str {
    lines
        .iter()
        .find(|line| line.starts_with(prefix))
        .map(String::as_str)
        .unwrap_or_else(|| panic!("missing line starting with {prefix:?}: {lines:?}"))
}

#[test]
fn host_memory_overview_is_unavailable_without_hle_dos() {
    let mut bus: machine::Pc9801Bus<machine::NoTracing> =
        machine::Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::High, 48_000);
    assert!(
        bus.debug_memory_overview_lines().is_none(),
        "overview should be unavailable before HLE DOS is active"
    );
}

#[test]
fn host_memory_overview_reports_all_sections() {
    let mut machine = harness::boot_hle();
    let lines = machine
        .bus
        .debug_memory_overview_lines()
        .expect("overview should be available under HLE DOS");

    assert_eq!(lines[0], "Memory overview (HLE DOS)");
    assert!(find_line(&lines, "Conventional: ").contains("free="));
    assert!(find_line(&lines, "UMB: ").contains("free="));
    assert!(find_line(&lines, "HMA: ").contains("state=free"));
    assert!(find_line(&lines, "EMS: ").contains("free="));
    assert!(find_line(&lines, "XMS: ").contains("free="));
    assert!(find_line(&lines, "Extended backing pool (EMS+XMS): ").contains("free="));
    assert_eq!(
        lines.last().unwrap(),
        "Note: EMS and XMS share the same backing pool."
    );
}

#[test]
fn host_memory_overview_reports_hma_allocation_state() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0xFF, 0xFF,                   // MOV DX, FFFFh
        0xB4, 0x01,                         // MOV AH, 01h (request HMA)
        0xCD, 0xFE,                         // INT FEh
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let lines = machine
        .bus
        .debug_memory_overview_lines()
        .expect("overview should be available under HLE DOS");
    let hma_line = find_line(&lines, "HMA: ");
    assert!(
        hma_line.contains("used=65,520 bytes")
            && hma_line.contains("free=0 bytes")
            && hma_line.contains("state=allocated"),
        "unexpected HMA line: {hma_line}"
    );
}

#[test]
fn host_memory_overview_reports_umb_usage() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xBA, 0x10, 0x00,                   // MOV DX, 16
        0xB4, 0x10,                         // MOV AH, 10h (UMB allocate)
        0xCD, 0xFE,                         // INT FEh
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let lines = machine
        .bus
        .debug_memory_overview_lines()
        .expect("overview should be available under HLE DOS");
    let umb_line = find_line(&lines, "UMB: ");
    assert!(
        umb_line.contains("total=65,504 bytes")
            && umb_line.contains("used=256 bytes")
            && umb_line.contains("free=65,248 bytes"),
        "unexpected UMB line: {umb_line}"
    );
}

#[test]
fn host_memory_overview_reports_shared_ems_xms_pool_usage() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        // Allocate EMS: 1 page
        0xBB, 0x01, 0x00,                   // MOV BX, 1
        0xB4, 0x43,                         // MOV AH, 43h
        0xCD, 0x67,                         // INT 67h
        // Allocate XMS: 64 KB
        0xBA, 0x40, 0x00,                   // MOV DX, 64
        0xB4, 0x09,                         // MOV AH, 09h
        0xCD, 0xFE,                         // INT FEh
        0xFA,                               // CLI
        0xF4,                               // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let lines = machine
        .bus
        .debug_memory_overview_lines()
        .expect("overview should be available under HLE DOS");
    let ems_line = find_line(&lines, "EMS: ");
    let xms_line = find_line(&lines, "XMS: ");
    let pool_line = find_line(&lines, "Extended backing pool (EMS+XMS): ");

    assert!(
        ems_line.contains("used=16K (16,384 bytes)"),
        "unexpected EMS line: {ems_line}"
    );
    assert!(
        xms_line.contains("total=12,224K (12,517,376 bytes)")
            && xms_line.contains("used=80K (81,920 bytes)")
            && xms_line.contains("free=12,144K (12,435,456 bytes)"),
        "unexpected XMS line: {xms_line}"
    );
    assert!(
        pool_line.contains("total=12,224K (12,517,376 bytes)")
            && pool_line.contains("used=80K (81,920 bytes)")
            && pool_line.contains("free=12,144K (12,435,456 bytes)"),
        "unexpected shared pool line: {pool_line}"
    );
}
