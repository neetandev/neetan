use crate::harness::*;

fn run_mem() -> machine::Pc9801Ra {
    let mut machine = boot_hle();
    type_string(&mut machine.bus, b"CLS\r");
    run_until_prompt(&mut machine);
    type_string(&mut machine.bus, b"MEM\r");
    run_until_prompt(&mut machine);
    machine
}

fn k_positions(line: &str) -> Vec<usize> {
    line.char_indices()
        .filter(|&(_, ch)| ch == 'K')
        .map(|(i, _)| i)
        .collect()
}

#[test]
fn mem_header_present() {
    let machine = run_mem();
    let header = "Memory Type          Total  =     Used  +     Free";
    assert!(
        find_row_containing(&machine.bus, header).is_some(),
        "MEM header line should appear verbatim"
    );
}

#[test]
fn mem_separator_present() {
    let machine = run_mem();
    let sep = "-----------------  --------    --------    --------";
    assert!(
        find_row_containing(&machine.bus, sep).is_some(),
        "MEM separator line should appear verbatim"
    );
}

#[test]
fn mem_table_k_columns_aligned() {
    let machine = run_mem();
    let mut data_rows = Vec::new();
    for row in 0..25 {
        let line = text_vram_row_to_string(&machine.bus, row);
        if line.contains("Conventional")
            || line.contains("Upper")
            || line.contains("Extended (XMS)")
            || line.contains("Total memory")
            || line.contains("Total under 1 MB")
        {
            data_rows.push((row, line));
        }
    }
    assert!(
        !data_rows.is_empty(),
        "Should find at least one MEM data row"
    );
    for (row_idx, line) in &data_rows {
        let positions = k_positions(line);
        assert_eq!(
            positions.len(),
            3,
            "Row {} ({:?}) should have exactly 3 'K' characters, found {:?}",
            row_idx,
            line.trim(),
            positions,
        );
        assert_eq!(
            positions,
            vec![26, 38, 50],
            "Row {} ({:?}): K positions should be [26, 38, 50], got {:?}",
            row_idx,
            line.trim(),
            positions,
        );
    }
}

#[test]
fn mem_conventional_present() {
    let machine = run_mem();
    assert!(
        find_row_containing(&machine.bus, "Conventional").is_some(),
        "MEM should show Conventional memory row"
    );
}

#[test]
fn mem_ems_lines_aligned() {
    let machine = run_mem();
    let total_row = find_row_containing(&machine.bus, "Total Expanded (EMS)");
    let free_row = find_row_containing(&machine.bus, "Free Expanded (EMS)");
    assert!(total_row.is_some(), "MEM should show Total Expanded (EMS)");
    assert!(free_row.is_some(), "MEM should show Free Expanded (EMS)");
    let total_line = text_vram_row_to_string(&machine.bus, total_row.unwrap());
    let free_line = text_vram_row_to_string(&machine.bus, free_row.unwrap());
    let total_k = k_positions(&total_line);
    let free_k = k_positions(&free_line);
    assert_eq!(
        total_k.first(),
        free_k.first(),
        "EMS total and free lines should have K at the same column"
    );
}

#[test]
fn mem_largest_block_present() {
    let machine = run_mem();
    assert!(
        find_row_containing(&machine.bus, "Largest executable program size").is_some(),
        "MEM should show largest executable program size"
    );
}
