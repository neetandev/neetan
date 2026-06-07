mod common;

use common::harness::{AdpcmTester, write_reg_hi};

fn normalize(s: &str) -> String {
    s.lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn adpcm_b_mem_rw() {
    let mut rs = AdpcmTester::new();

    rs.seq_mem_limit(0xFFFF);
    rs.seq_mem_write(0x0000, 0x0000, 0x00, 32, "WRITE ADDRESS 0000-0000 (00-1F)");
    rs.seq_mem_write(0x0FFF, 0x1000, 0x40, 64, "WRITE ADDRESS 0fff-1000 (40-7F)");
    rs.seq_mem_write(
        0x0FFF,
        0x0FFF,
        0x80,
        64,
        "WRITE ADDRESS 0fff-0xfff x 2 (80-9F/A0-BF)",
    );
    rs.seq_mem_write(0x1FFF, 0x1FFF, 0xC0, 32, "WRITE ADDRESS 1fff-1fff (C0-DF)");

    rs.seq_mem_read(0x0000, 0x0000, 34, "READ ADDRESS 0000-0000");
    rs.seq_mem_read(0x0FFF, 0x1000, 66, "READ ADDRESS 0fff-1000");
    rs.seq_mem_read(0x0FFF, 0x0FFF, 68, "READ ADDRESS 0fff-0xfff x 2");

    // test_mem_read_start(rs, 0x0fff, 0x1000, 0x1000, 1, 66+34-1)
    {
        rs.msg("READ ADDRESS 0fff- CHANGE START(1)");
        rs.out(0x10, 0x00).out(0x10, 0x80);
        rs.out(0x00, 0x20).out(0x01, 0x02);
        rs.out(0x02, 0xFF).out(0x03, 0x0F);
        rs.out(0x04, 0x00).out(0x05, 0x10);
        rs.nl();
        rs.mrd(1).nl();
        rs.out(0x02, 0x00).out(0x03, 0x10);
        rs.mrd(66 + 34 - 1).nl();
        rs.out(0x00, 0x00).out(0x10, 0x80).nl();
    }

    // test_mem_read_start(rs, 0x0fff, 0x1000, 0x1000, 10, 66+34-10)
    {
        rs.msg("READ ADDRESS 0fff- CHANGE START(10)");
        rs.out(0x10, 0x00).out(0x10, 0x80);
        rs.out(0x00, 0x20).out(0x01, 0x02);
        rs.out(0x02, 0xFF).out(0x03, 0x0F);
        rs.out(0x04, 0x00).out(0x05, 0x10);
        rs.nl();
        rs.mrd(10).nl();
        rs.out(0x02, 0x00).out(0x03, 0x10);
        rs.mrd(66 + 34 - 10).nl();
        rs.out(0x00, 0x00).out(0x10, 0x80).nl();
    }

    // test_mem_read_start(rs, 0x0fff, 0x0000, 0x1000, 10, 66+34-10)
    {
        rs.msg("READ ADDRESS 0fff- CHANGE START(10/RESET)");
        rs.out(0x10, 0x00).out(0x10, 0x80);
        rs.out(0x00, 0x20).out(0x01, 0x02);
        rs.out(0x02, 0xFF).out(0x03, 0x0F);
        rs.out(0x04, 0x00).out(0x05, 0x10);
        rs.nl();
        rs.mrd(10).nl();
        rs.out(0x02, 0x00).out(0x03, 0x00);
        rs.mrd(66 + 34 - 10).nl();
        rs.out(0x00, 0x00).out(0x10, 0x80).nl();
    }

    rs.reset();

    // test_mem_read_stop(rs, 0x0fff, 0x0fff, 0x1000, 10, 66+66-10)
    {
        rs.msg("READ ADDRESS 0fff- CHANGE STOP");
        rs.out(0x10, 0x00).out(0x10, 0x80);
        rs.out(0x00, 0x20).out(0x01, 0x02);
        rs.out(0x02, 0xFF).out(0x03, 0x0F);
        rs.out(0x04, 0xFF).out(0x05, 0x0F);
        rs.nl();
        rs.mrd(10).nl();
        rs.out(0x04, 0x00).out(0x05, 0x10);
        rs.mrd(66 + 66 - 10).nl();
        rs.out(0x00, 0x00).out(0x10, 0x80).nl();
    }

    rs.seq_mem_read(0x1FFF, 0x1FFF, 34, "READ ADDRESS 1fff-1fff");

    rs.msg("READ / WRITE MIX");
    rs.reset();

    // test_mem_read_write(rs, 0x0000, 0x0000, 10, 68-10-1)
    {
        rs.msg("READ ADDRESS 0000-0000 (10 WRITE)");
        rs.out(0x10, 0x00).out(0x10, 0x80);
        rs.out(0x00, 0x20).out(0x01, 0x02);
        rs.out(0x02, 0x00).out(0x03, 0x00);
        rs.out(0x04, 0x00).out(0x05, 0x00);
        rs.nl();
        rs.mrd(10).nl();
        rs.out(0x08, 0xCC).stat();
        rs.mrd(68 - 10 - 1).nl();
        rs.out(0x00, 0x00).out(0x10, 0x80).nl();
    }

    let expected = include_str!("golden/mem_rw.txt");
    assert_eq!(normalize(rs.output()), normalize(expected));
}

#[test]
fn adpcm_b_mem_rw13() {
    let mut rs = AdpcmTester::new();

    rs.reset();

    rs.seq_mem_limit(0xFFFF);
    rs.seq_mem_write(
        0x0000,
        0x0001,
        0x00,
        64,
        "1. WRITE ADDRESS 0000-0001 (00-3F)",
    );
    rs.seq_mem_write(
        0x0FFF,
        0x0FFF,
        0xA0,
        32,
        "2. WRITE ADDRESS 0fff-0fff (A0-BF)",
    );

    let mut testno = 3;
    let ivals: &[u16] = &[1, 2, 3, 4, 31, 32, 33, 34, 35, 36, 37];
    for &i in ivals {
        rs.reset();
        let label = format!("{}. READ ADDRESS 0000-0000 ({}B)", testno, i);
        // test_seq_mem_read with control1 = 0x20
        {
            rs.msg(&label);
            rs.out(0x10, 0x00).out(0x10, 0x80);
            rs.out(0x00, 0x20).out(0x01, 0x02);
            rs.out(0x02, 0x00).out(0x03, 0x00);
            rs.out(0x04, 0x00).out(0x05, 0x00);
            rs.nl();
            rs.mrd(i).nl();
            rs.out(0x00, 0x00).out(0x10, 0x80).nl();
        }
        testno += 1;
        rs.reset();
        let label = format!("{}. READ ADDRESS 0fff-0fff (DUMMY READ TEST)", testno);
        rs.seq_mem_read(0x0FFF, 0x0FFF, 32 + 2, &label);
        testno += 1;
    }

    let expected = include_str!("golden/mem_rw13.txt");
    assert_eq!(normalize(rs.output()), normalize(expected));
}

#[test]
fn adpcm_b_mem_rw14() {
    let mut rs = AdpcmTester::new();

    rs.reset();

    rs.seq_mem_limit(0xFFFF);
    rs.seq_mem_write(
        0x0000,
        0x0001,
        0x00,
        64,
        "1. WRITE ADDRESS 0000-0001 (00-3F)",
    );
    rs.seq_mem_write(
        0x0FFF,
        0x0FFF,
        0xA0,
        32,
        "2. WRITE ADDRESS 0fff-0fff (A0-BF)",
    );

    let mut testno = 3;
    let ivals: &[u16] = &[1, 2, 3, 4, 31, 32, 33, 34, 35, 36, 37];
    for &i in ivals {
        rs.reset();
        let label = format!("{}. READ ADDRESS 0000-0000 ({}B) (REPEAT=1)", testno, i);
        // test_seq_mem_read with control1 = 0x30
        {
            rs.msg(&label);
            rs.out(0x10, 0x00).out(0x10, 0x80);
            rs.out(0x00, 0x30).out(0x01, 0x02);
            rs.out(0x02, 0x00).out(0x03, 0x00);
            rs.out(0x04, 0x00).out(0x05, 0x00);
            rs.nl();
            rs.mrd(i).nl();
            rs.out(0x00, 0x00).out(0x10, 0x80).nl();
        }
        testno += 1;
        rs.reset();
        let label = format!("{}. READ ADDRESS 0fff-0fff (DUMMY READ TEST)", testno);
        rs.seq_mem_read(0x0FFF, 0x0FFF, 32 + 2, &label);
        testno += 1;
    }

    let expected = include_str!("golden/mem_rw14.txt");
    assert_eq!(normalize(rs.output()), normalize(expected));
}

#[test]
fn adpcm_b_mem_rw15() {
    let mut rs = AdpcmTester::new();

    rs.seq_mem_limit(0xFFFF);
    rs.reset();

    // test_mem_write with control1 = 0x70
    fn test_mem_write(
        rs: &mut AdpcmTester,
        start: u16,
        stop: u16,
        data: u8,
        count: u16,
        message: &str,
    ) {
        rs.msg(message);
        rs.out(0x10, 0x00).out(0x10, 0x80);
        rs.out(0x00, 0x70).out(0x01, 0x02);
        rs.out(0x02, (start & 0xFF) as u8)
            .out(0x03, ((start >> 8) & 0xFF) as u8);
        rs.out(0x04, (stop & 0xFF) as u8)
            .out(0x05, ((stop >> 8) & 0xFF) as u8);
        rs.nl();
        rs.mwr(data, count).nl();
        rs.out(0x00, 0x00).out(0x10, 0x80).nl();
    }

    test_mem_write(
        &mut rs,
        0x0000,
        0x0000,
        0x00,
        32,
        "1. WRITE ADDRESS 0000-0000 (00-1F)",
    );

    let mut testno = 2;
    for i in 0..4u16 {
        rs.reset();
        let label = format!(
            "{}. WRITE ADDRESS 0fff-0fff (40-5F...) ({}B)",
            testno,
            32 + i
        );
        test_mem_write(&mut rs, 0x0FFF, 0x0FFF, 0x40, 32 + i, &label);
        testno += 1;
        rs.reset();
        let label = format!("{}. READ ADDRESS 0000-0000", testno);
        rs.seq_mem_read(0x0000, 0x0000, 34, &label);
        testno += 1;
    }

    let label = format!("{}. READ ADDRESS 0fff-0fff", testno);
    rs.reset();
    rs.seq_mem_read(0x0FFF, 0x0FFF, 34, &label);

    let expected = include_str!("golden/mem_rw15.txt");
    assert_eq!(normalize(rs.output()), normalize(expected));
}

#[test]
fn adpcm_b_mem_no_ram_reads_zero() {
    use ymfm_oxide::{Ym2608, YmfmOpnFidelity};

    let mut chip = Ym2608::new();
    chip.reset();
    chip.set_fidelity(YmfmOpnFidelity::Max);

    // Set up external read mode: control1=0x20 (external only), dram_8bit=1
    write_reg_hi(&mut chip, 0x10, 0x80); // Flag control: reset flags
    write_reg_hi(&mut chip, 0x00, 0x01); // Reset
    write_reg_hi(&mut chip, 0x00, 0x00); // Clear reset
    write_reg_hi(&mut chip, 0x00, 0x20); // External read mode
    write_reg_hi(&mut chip, 0x01, 0x02); // dram_8bit=1
    write_reg_hi(&mut chip, 0x02, 0x00); // Start low
    write_reg_hi(&mut chip, 0x03, 0x00); // Start high
    write_reg_hi(&mut chip, 0x04, 0x00); // End low
    write_reg_hi(&mut chip, 0x05, 0x00); // End high

    // Read via register 0x08 - should get 0x00 since memory is empty.
    chip.write_address_hi(0x08);
    let data = chip.read_data_hi();
    assert_eq!(
        data, 0x00,
        "Expected 0x00 from empty ADPCM RAM, got 0x{data:02X}"
    );
}

#[test]
#[should_panic(expected = "ADPCM-B RAM must not be empty")]
fn adpcm_b_empty_ram_panics() {
    use ymfm_oxide::Ym2608;

    let mut chip = Ym2608::new();
    chip.set_adpcm_b_ram(Vec::new());
}

#[test]
fn adpcm_b_mem_write_then_read_roundtrip() {
    use ymfm_oxide::{Ym2608, YmfmOpnFidelity};

    let mut chip = Ym2608::new();
    chip.reset();
    chip.set_adpcm_b_ram(vec![0; 256 * 1024]);
    chip.set_fidelity(YmfmOpnFidelity::Max);

    // Write 4 bytes in record mode at address 0x0000.
    write_reg_hi(&mut chip, 0x10, 0x80);
    write_reg_hi(&mut chip, 0x00, 0x01);
    write_reg_hi(&mut chip, 0x00, 0x00);
    write_reg_hi(&mut chip, 0x00, 0x60); // External + record
    write_reg_hi(&mut chip, 0x01, 0x02); // dram_8bit=1
    write_reg_hi(&mut chip, 0x02, 0x00);
    write_reg_hi(&mut chip, 0x03, 0x00);
    write_reg_hi(&mut chip, 0x04, 0xFF);
    write_reg_hi(&mut chip, 0x05, 0xFF);
    write_reg_hi(&mut chip, 0x0C, 0xFF);
    write_reg_hi(&mut chip, 0x0D, 0xFF);

    let pattern: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];
    for &byte in &pattern {
        write_reg_hi(&mut chip, 0x08, byte);
        write_reg_hi(&mut chip, 0x10, 0x80);
    }

    // Verify directly that the chip memory received the pattern.
    let mem = chip.adpcm_b_ram().unwrap();
    for (i, &expected) in pattern.iter().enumerate() {
        assert_eq!(
            mem[i], expected,
            "ADPCM RAM byte {i}: expected 0x{expected:02X}, got 0x{:02X}",
            mem[i]
        );
    }
}

#[test]
fn adpcm_b_mem_limit_wrapping() {
    use ymfm_oxide::{Ym2608, YmfmOpnFidelity};

    let mut chip = Ym2608::new();
    chip.reset();
    chip.set_adpcm_b_ram(vec![0; 256 * 1024]);
    chip.set_fidelity(YmfmOpnFidelity::Max);

    // With dram_8bit=1 (shift=5), each unit is 32 bytes.
    // Set limit=0x0001: when the unit address reaches 1, the address wraps to 0.
    // That means after writing 64 bytes (units 0 and 1), the next write wraps.
    // So write 96 bytes total: first 64 fill units 0-1, then the wrap triggers
    // and bytes 64-95 overwrite unit 0 (addresses 0-31).
    write_reg_hi(&mut chip, 0x10, 0x80);
    write_reg_hi(&mut chip, 0x00, 0x01);
    write_reg_hi(&mut chip, 0x00, 0x00);
    write_reg_hi(&mut chip, 0x00, 0x60); // External + record
    write_reg_hi(&mut chip, 0x01, 0x02); // dram_8bit=1
    write_reg_hi(&mut chip, 0x02, 0x00); // Start=0
    write_reg_hi(&mut chip, 0x03, 0x00);
    write_reg_hi(&mut chip, 0x04, 0xFF); // End=wide
    write_reg_hi(&mut chip, 0x05, 0xFF);
    write_reg_hi(&mut chip, 0x0C, 0x01); // Limit=0x0001
    write_reg_hi(&mut chip, 0x0D, 0x00);

    for i in 0..96u8 {
        write_reg_hi(&mut chip, 0x08, i);
        write_reg_hi(&mut chip, 0x10, 0x80);
    }

    // After wrapping, addresses 0-31 should contain bytes 64-95 (the third batch).
    let mem = chip.adpcm_b_ram().unwrap();
    for (i, &value) in mem.iter().take(32).enumerate() {
        assert_eq!(
            value,
            (i as u8 + 64),
            "Address {i}: expected {} (from wrap), got {}",
            i as u8 + 64,
            value
        );
    }
    // Addresses 32-63 should still have bytes 32-63 (the second batch, not overwritten).
    for (i, &value) in mem.iter().enumerate().take(64).skip(32) {
        assert_eq!(
            value, i as u8,
            "Address {i}: expected {} (original), got {}",
            i as u8, value
        );
    }
}

#[test]
fn adpcm_b_mem_start_stop_range() {
    use ymfm_oxide::{Ym2608, YmfmOpnFidelity};

    let mut chip = Ym2608::new();
    chip.reset();
    chip.set_adpcm_b_ram(vec![0; 256 * 1024]);
    chip.set_fidelity(YmfmOpnFidelity::Max);

    // Write a known pattern at two different address ranges using record mode.
    // Range 1: unit 0x0000 (bytes 0x00..0x1F) - fill with 0xAA
    write_reg_hi(&mut chip, 0x10, 0x80);
    write_reg_hi(&mut chip, 0x00, 0x01);
    write_reg_hi(&mut chip, 0x00, 0x00);
    write_reg_hi(&mut chip, 0x00, 0x60); // External + record
    write_reg_hi(&mut chip, 0x01, 0x02); // dram_8bit=1
    write_reg_hi(&mut chip, 0x02, 0x00);
    write_reg_hi(&mut chip, 0x03, 0x00);
    write_reg_hi(&mut chip, 0x04, 0x00);
    write_reg_hi(&mut chip, 0x05, 0x00);
    write_reg_hi(&mut chip, 0x0C, 0xFF);
    write_reg_hi(&mut chip, 0x0D, 0xFF);
    for _ in 0..32 {
        write_reg_hi(&mut chip, 0x08, 0xAA);
        write_reg_hi(&mut chip, 0x10, 0x80);
    }

    // Range 2: unit 0x0001 (bytes 0x20..0x3F) - fill with 0xBB
    write_reg_hi(&mut chip, 0x00, 0x01);
    write_reg_hi(&mut chip, 0x00, 0x00);
    write_reg_hi(&mut chip, 0x10, 0x80);
    write_reg_hi(&mut chip, 0x00, 0x60);
    write_reg_hi(&mut chip, 0x01, 0x02);
    write_reg_hi(&mut chip, 0x02, 0x01);
    write_reg_hi(&mut chip, 0x03, 0x00);
    write_reg_hi(&mut chip, 0x04, 0x01);
    write_reg_hi(&mut chip, 0x05, 0x00);
    for _ in 0..32 {
        write_reg_hi(&mut chip, 0x08, 0xBB);
        write_reg_hi(&mut chip, 0x10, 0x80);
    }

    // Verify directly that the chip memory has the correct pattern.
    let mem = chip.adpcm_b_ram().unwrap();
    for (i, &value) in mem.iter().take(32).enumerate() {
        assert_eq!(
            value, 0xAA,
            "Range 1, address {i}: expected 0xAA, got 0x{:02X}",
            value
        );
    }
    for (i, &value) in mem.iter().enumerate().take(64).skip(32) {
        assert_eq!(
            value, 0xBB,
            "Range 2, address {i}: expected 0xBB, got 0x{:02X}",
            value
        );
    }
}
