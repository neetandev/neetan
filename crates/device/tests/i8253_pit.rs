use device::i8253_pit::{I8253Pit, PIT_FLAG_I, WriteResult};

const CPU_HZ: u32 = 8_000_000;
const PIT_HZ: u32 = 1_996_800;

#[test]
fn control_word_programming() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: word access, mode 3, binary
    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ);
    assert_eq!(pit.channels[0].ctrl & 0x3F, 0x36);

    // Channel 1: low byte, mode 2, binary
    pit.write_control(1, 0x54, 0, CPU_HZ, PIT_HZ);
    assert_eq!(pit.channels[1].ctrl & 0x3F, 0x14);

    // Channel 2: word access, mode 3, binary
    pit.write_control(2, 0xB6, 0, CPU_HZ, PIT_HZ);
    assert_eq!(pit.channels[2].ctrl & 0x3F, 0x36);
}

#[test]
fn counter_latch_command() {
    let mut pit = I8253Pit::new_zeroed();

    // Program channel 0: word access, mode 3
    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8); // LSB
    pit.write_counter(0, 0x03); // MSB -> value = 0x03E8 = 1000
    pit.channels[0].last_load_cycle = 0;

    // Latch channel 0 (SC=00, RL=00)
    pit.write_control(0, 0x00, 0, CPU_HZ, PIT_HZ);

    // Read latched value (word mode: low then high)
    let low = pit.read_counter(0, 0, CPU_HZ, PIT_HZ);
    let high = pit.read_counter(0, 0, CPU_HZ, PIT_HZ);
    let latched = (high as u16) << 8 | low as u16;
    assert_eq!(latched, 1000);
}

#[test]
fn word_mode_write_toggle() {
    let mut pit = I8253Pit::new_zeroed();

    // Program channel 0: word access, mode 3
    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ);

    // First byte (LSB): should return Skip (incomplete).
    assert_eq!(pit.write_counter(0, 0xE8), WriteResult::Skip);
    // Second byte (MSB): should return InitialLoad (complete).
    assert_eq!(pit.write_counter(0, 0x03), WriteResult::InitialLoad);

    assert_eq!(pit.channels[0].value, 0x03E8);
}

#[test]
fn word_mode_read_toggle() {
    let mut pit = I8253Pit::new_zeroed();

    // Program channel 0: word access, mode 3
    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x34); // LSB
    pit.write_counter(0, 0x02); // MSB -> value = 0x0234
    pit.channels[0].last_load_cycle = 0;

    // Latch to get a stable value
    pit.write_control(0, 0x00, 0, CPU_HZ, PIT_HZ);

    // Read low then high
    let low = pit.read_counter(0, 0, CPU_HZ, PIT_HZ);
    assert_eq!(low, 0x34);
    let high = pit.read_counter(0, 0, CPU_HZ, PIT_HZ);
    assert_eq!(high, 0x02);
}

#[test]
fn low_byte_access_mode() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: low byte only, mode 2
    pit.write_control(0, 0x14, 0, CPU_HZ, PIT_HZ);
    assert_eq!(pit.write_counter(0, 0x64), WriteResult::InitialLoad); // value = 100
    assert_eq!(pit.channels[0].value, 100);
}

#[test]
fn high_byte_access_mode() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: high byte only, mode 2
    pit.write_control(0, 0x24, 0, CPU_HZ, PIT_HZ);
    assert_eq!(pit.write_counter(0, 0x04), WriteResult::InitialLoad); // value = 0x0400 = 1024
    assert_eq!(pit.channels[0].value, 0x0400);
}

#[test]
fn get_count_mode2_periodic() {
    let mut pit = I8253Pit::new_zeroed();

    // Program channel 0: word access, mode 2 (rate generator)
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8); // LSB
    pit.write_counter(0, 0x03); // MSB -> reload = 1000
    pit.channels[0].last_load_cycle = 0;

    // At cycle 0: count should be 1000 (just loaded)
    let count = pit.get_count(0, 0, CPU_HZ, PIT_HZ);
    assert_eq!(count, 1000);

    // After some PIT ticks: 1 CPU cycle ≈ 0.2496 PIT ticks
    // 4 CPU cycles ≈ 0.998 PIT ticks -> ~1 PIT tick elapsed.
    let count = pit.get_count(0, 4, CPU_HZ, PIT_HZ);
    // Should be close to 999 (1000 - 1)
    assert!(count <= 1000);
    assert!(count >= 998);
}

#[test]
fn get_count_mode0_oneshot() {
    let mut pit = I8253Pit::new_zeroed();

    // Mode 0 (interrupt on terminal count)
    pit.write_control(0, 0x30, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x0A); // LSB
    pit.write_counter(0, 0x00); // MSB -> reload = 10
    pit.channels[0].last_load_cycle = 0;

    let count = pit.get_count(0, 0, CPU_HZ, PIT_HZ);
    assert_eq!(count, 10);

    // After many cycles, should be 0 (one-shot expired).
    let count = pit.get_count(0, 100_000, CPU_HZ, PIT_HZ);
    assert_eq!(count, 0);
}

#[test]
fn pc98_boot_defaults() {
    let pit_8mhz = I8253Pit::new(true);

    // Channel 0: ctrl=0x16, flag=PIT_FLAG_I
    assert_eq!(pit_8mhz.channels[0].ctrl & 0x3F, 0x16);
    assert_ne!(pit_8mhz.channels[0].flag & 0x20, 0); // PIT_FLAG_I

    // Channel 1: value=998 for 8MHz lineage
    assert_eq!(pit_8mhz.channels[1].value, 998);

    // Channel 2: ctrl=0x36
    assert_eq!(pit_8mhz.channels[2].ctrl & 0x3F, 0x36);

    // 5/10MHz lineage: beep counter = 1229
    let pit_10mhz = I8253Pit::new(false);
    assert_eq!(pit_10mhz.channels[1].value, 1229);
}

#[test]
fn pc98_boot_sequence_init() {
    let mut pit = I8253Pit::new_zeroed();

    // BIOS PIT init sequence.
    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ); // ch0: word, mode 3
    pit.write_counter(0, 0xE8); // LSB
    pit.write_counter(0, 0x03); // MSB -> 1000

    assert_eq!(pit.channels[0].value, 1000);
    assert_eq!(pit.channels[0].ctrl & 0x3F, 0x36);
}

#[test]
fn double_latch() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: word access, mode 2
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8);
    pit.write_counter(0, 0x03); // reload = 1000
    pit.channels[0].last_load_cycle = 0;

    // First latch at cycle 0 -> snapshots 1000
    pit.write_control(0, 0x00, 0, CPU_HZ, PIT_HZ);

    // Second latch at cycle 100 before reading first.
    // elapsed_pit = 100 * 1_996_800 / 8_000_000 = 24
    // mode 2: pos = 24 % 1000 = 24, count = 1000 - 24 = 976
    pit.write_control(0, 0x00, 100, CPU_HZ, PIT_HZ);

    // Read returns second latch (976), not first (1000)
    let low = pit.read_counter(0, 100, CPU_HZ, PIT_HZ);
    let high = pit.read_counter(0, 100, CPU_HZ, PIT_HZ);
    let latched = (high as u16) << 8 | low as u16;
    assert_eq!(latched, 976);
}

#[test]
fn mode2_periodic_wrap() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: word access, mode 2, reload = 1000
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8);
    pit.write_counter(0, 0x03);
    pit.channels[0].last_load_cycle = 0;

    // At cpu_cycles=5000: elapsed_pit = 5000 * 1_996_800 / 8_000_000 = 1248
    // pos = 1248 % 1000 = 248, count = 1000 - 248 = 752
    assert_eq!(pit.get_count(0, 5000, CPU_HZ, PIT_HZ), 752);

    // At cpu_cycles=20000: elapsed_pit = 20000 * 1_996_800 / 8_000_000 = 4992
    // pos = 4992 % 1000 = 992, count = 1000 - 992 = 8
    assert_eq!(pit.get_count(0, 20000, CPU_HZ, PIT_HZ), 8);
}

#[test]
fn mode3_periodic() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: word access, mode 3
    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8);
    pit.write_counter(0, 0x03); // reload = 1000
    pit.channels[0].last_load_cycle = 0;

    // Mode 3 decrements by 2 per PIT tick.
    // At cpu_cycles=5000: elapsed_pit = 1248, period = 1000, half = 500
    // pos = 1248 % 1000 = 248, pos_in_half = 248 % 500 = 248
    // count = 1000 - 248 * 2 = 504
    assert_eq!(pit.get_count(0, 5000, CPU_HZ, PIT_HZ), 504);
}

#[test]
fn counter_value_zero_is_65536() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: word access, mode 2
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x00);
    pit.write_counter(0, 0x00);
    pit.channels[0].last_load_cycle = 0;

    assert_eq!(pit.channels[0].value, 0);

    // At cycle 0: returns ch.value = 0
    assert_eq!(pit.get_count(0, 0, CPU_HZ, PIT_HZ), 0);

    // At cpu_cycles=401: elapsed_pit = 401 * 1_996_800 / 8_000_000 = 100
    // period = 0x10000 = 65536, pos = 100 % 65536 = 100
    // count = (65536 - 100) = 65436
    assert_eq!(pit.get_count(0, 401, CPU_HZ, PIT_HZ), 65436);
}

#[test]
fn mode0_countdown_reads() {
    let mut pit = I8253Pit::new_zeroed();

    // Mode 0 (interrupt on terminal count).
    pit.write_control(0, 0x30, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x64); // LSB = 100
    pit.write_counter(0, 0x00); // MSB -> value = 100
    pit.channels[0].last_load_cycle = 0;

    assert_eq!(pit.get_count(0, 0, CPU_HZ, PIT_HZ), 100);

    // At cpu_cycles=201: elapsed_pit = 201 * 1_996_800 / 8_000_000 = 50
    // count = 100 - 50 = 50
    assert_eq!(pit.get_count(0, 201, CPU_HZ, PIT_HZ), 50);

    // At cpu_cycles=401: elapsed_pit = 100
    // elapsed_pit >= count_period -> returns 0
    assert_eq!(pit.get_count(0, 401, CPU_HZ, PIT_HZ), 0);

    // Well past expiry: still 0
    assert_eq!(pit.get_count(0, 100_000, CPU_HZ, PIT_HZ), 0);
}

#[test]
fn control_word_sets_stat_cmd() {
    let mut pit = I8253Pit::new_zeroed();

    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);

    // PIT_STAT_CMD (0x40) should be set.
    assert_ne!(pit.channels[0].ctrl & 0x40, 0);
    // Mode/RL portion.
    assert_eq!(pit.channels[0].ctrl & 0x3F, 0x34);
    // Full ctrl = 0x34 | 0x40 = 0x74
    assert_eq!(pit.channels[0].ctrl, 0x74);
}

#[test]
fn write_counter_clears_stat_cmd() {
    let mut pit = I8253Pit::new_zeroed();

    // Word access, mode 2
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    assert_ne!(pit.channels[0].ctrl & 0x40, 0);

    // Write LSB - PIT_STAT_CMD still set (incomplete word write).
    pit.write_counter(0, 0x64);
    assert_ne!(pit.channels[0].ctrl & 0x40, 0);

    // Write MSB - PIT_STAT_CMD cleared.
    pit.write_counter(0, 0x00);
    assert_eq!(pit.channels[0].ctrl & 0x40, 0);

    // Low-byte-only mode
    pit.write_control(0, 0x14, 0, CPU_HZ, PIT_HZ);
    assert_ne!(pit.channels[0].ctrl & 0x40, 0);
    pit.write_counter(0, 0x50);
    assert_eq!(pit.channels[0].ctrl & 0x40, 0);

    // High-byte-only mode
    pit.write_control(0, 0x24, 0, CPU_HZ, PIT_HZ);
    assert_ne!(pit.channels[0].ctrl & 0x40, 0);
    pit.write_counter(0, 0x01);
    assert_eq!(pit.channels[0].ctrl & 0x40, 0);
}

#[test]
fn multiple_channels_independent() {
    let mut pit = I8253Pit::new_zeroed();

    // Program channel 0: mode 2, reload 500
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xF4);
    pit.write_counter(0, 0x01); // 500
    pit.channels[0].last_load_cycle = 0;

    // Program channel 1: mode 3, reload 200
    pit.write_control(1, 0x76, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(1, 0xC8);
    pit.write_counter(1, 0x00); // 200
    pit.channels[1].last_load_cycle = 0;

    // Program channel 2: low byte, mode 0, reload 50
    pit.write_control(2, 0x90, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(2, 0x32); // 50
    pit.channels[2].last_load_cycle = 0;

    assert_eq!(pit.channels[0].value, 500);
    assert_eq!(pit.channels[1].value, 200);
    assert_eq!(pit.channels[2].value, 50);

    assert_eq!(pit.get_count(0, 0, CPU_HZ, PIT_HZ), 500);
    assert_eq!(pit.get_count(1, 0, CPU_HZ, PIT_HZ), 200);
    assert_eq!(pit.get_count(2, 0, CPU_HZ, PIT_HZ), 50);

    // Modify channel 1 without affecting others.
    pit.write_control(1, 0x76, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(1, 0x64);
    pit.write_counter(1, 0x00); // 100
    assert_eq!(pit.channels[1].value, 100);
    assert_eq!(pit.channels[0].value, 500);
    assert_eq!(pit.channels[2].value, 50);
}

#[test]
fn live_count_without_latch() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: word access, mode 2, reload = 1000
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8);
    pit.write_counter(0, 0x03);
    pit.channels[0].last_load_cycle = 0;

    // PIT_FLAG_C should be clear (no latch command).
    assert_eq!(pit.channels[0].flag & 0x10, 0);

    // Read low byte (live count at cycle 0 = 1000 = 0x03E8).
    let low = pit.read_counter(0, 0, CPU_HZ, PIT_HZ);
    assert_eq!(low, 0xE8);

    // Read high byte (toggle flipped).
    let high = pit.read_counter(0, 0, CPU_HZ, PIT_HZ);
    assert_eq!(high, 0x03);
}

#[test]
fn control_word_resets_write_toggle() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: word access, mode 2
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);

    // Write LSB only (partial word write).
    pit.write_counter(0, 0xAA);
    assert_ne!(pit.channels[0].flag & 0x02, 0); // PIT_FLAG_W toggled

    // New control word resets toggles
    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ);
    assert_eq!(pit.channels[0].flag & 0x02, 0); // PIT_FLAG_W cleared

    // Full word write works from scratch.
    pit.write_counter(0, 0x64);
    pit.write_counter(0, 0x00);
    assert_eq!(pit.channels[0].value, 0x0064);
}

#[test]
fn control_word_resets_read_toggle() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: word access, mode 2, reload = 1000
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8);
    pit.write_counter(0, 0x03);
    pit.channels[0].last_load_cycle = 0;

    // Latch to get a stable value.
    pit.write_control(0, 0x00, 0, CPU_HZ, PIT_HZ);

    // Read low byte (toggle flips to "read high next")
    let low = pit.read_counter(0, 0, CPU_HZ, PIT_HZ);
    assert_eq!(low, 0xE8);

    // Do NOT read high byte. Write new control word instead.
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);

    // Read toggle should be reset: next read starts from low byte.
    assert_eq!(pit.channels[0].flag & 0x01, 0); // PIT_FLAG_R cleared

    // Reload and latch again.
    pit.write_counter(0, 0x34);
    pit.write_counter(0, 0x02); // value = 0x0234
    pit.channels[0].last_load_cycle = 0;
    pit.write_control(0, 0x00, 0, CPU_HZ, PIT_HZ);

    // Should read low byte first.
    let low = pit.read_counter(0, 0, CPU_HZ, PIT_HZ);
    assert_eq!(low, 0x34);
    let high = pit.read_counter(0, 0, CPU_HZ, PIT_HZ);
    assert_eq!(high, 0x02);
}

#[test]
fn latch_overwrites_previous() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: word access, mode 2, reload = 1000
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8);
    pit.write_counter(0, 0x03);
    pit.channels[0].last_load_cycle = 0;

    // First latch at cycle 0 -> should snapshot 1000
    pit.write_control(0, 0x00, 0, CPU_HZ, PIT_HZ);
    assert_eq!(pit.channels[0].latch, 1000);

    // Second latch at cycle 100 without reading first.
    // elapsed_pit = 100 * 1_996_800 / 8_000_000 = 24
    // mode 2: pos = 24 % 1000 = 24, count = 1000 - 24 = 976
    pit.write_control(0, 0x00, 100, CPU_HZ, PIT_HZ);
    assert_eq!(pit.channels[0].latch, 976);

    // Read returns the second (overwritten) latch value.
    let low = pit.read_counter(0, 100, CPU_HZ, PIT_HZ);
    let high = pit.read_counter(0, 100, CPU_HZ, PIT_HZ);
    let latched = (high as u16) << 8 | low as u16;
    assert_eq!(latched, 976);
}

#[test]
fn read_after_latch_consumed_returns_live() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: word access, mode 2, reload = 1000
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8);
    pit.write_counter(0, 0x03);
    pit.channels[0].last_load_cycle = 0;

    // Latch at cycle 0
    pit.write_control(0, 0x00, 0, CPU_HZ, PIT_HZ);
    assert_ne!(pit.channels[0].flag & 0x10, 0); // PIT_FLAG_C set

    // Read both bytes (consumes latch).
    pit.read_counter(0, 0, CPU_HZ, PIT_HZ); // low
    pit.read_counter(0, 0, CPU_HZ, PIT_HZ); // high
    assert_eq!(pit.channels[0].flag & 0x10, 0); // PIT_FLAG_C cleared

    // Next read at cycle 200 returns LIVE count (no latch).
    // elapsed_pit = 200 * 1_996_800 / 8_000_000 = 49
    // mode 2: pos = 49 % 1000 = 49, count = 1000 - 49 = 951 = 0x03B7
    let low = pit.read_counter(0, 200, CPU_HZ, PIT_HZ);
    assert_eq!(low, 0xB7);
    let high = pit.read_counter(0, 200, CPU_HZ, PIT_HZ);
    assert_eq!(high, 0x03);
}

#[test]
fn mode0_stays_at_zero() {
    let mut pit = I8253Pit::new_zeroed();

    // Mode 0 (interrupt on terminal count).
    pit.write_control(0, 0x30, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x0A); // LSB = 10
    pit.write_counter(0, 0x00); // MSB -> value = 10
    pit.channels[0].last_load_cycle = 0;

    assert_eq!(pit.get_count(0, 0, CPU_HZ, PIT_HZ), 10);

    // At elapsed_pit = 10: counter reaches 0
    // cpu_cycle for 10 pit ticks: ceil(10 * 8000000 / 1996800) = 41
    assert_eq!(pit.get_count(0, 41, CPU_HZ, PIT_HZ), 0);

    // Stays at 0 (does not wrap).
    assert_eq!(pit.get_count(0, 100, CPU_HZ, PIT_HZ), 0);
    assert_eq!(pit.get_count(0, 1_000, CPU_HZ, PIT_HZ), 0);
    assert_eq!(pit.get_count(0, 1_000_000, CPU_HZ, PIT_HZ), 0);
}

#[test]
fn mode2_exact_period_boundary() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: word access, mode 2, reload = 100
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x64);
    pit.write_counter(0, 0x00);
    pit.channels[0].last_load_cycle = 0;

    // At elapsed_pit = 100: pos = 100 % 100 = 0 -> returns ch.value (100)
    // cpu_cycle for 100 pit ticks: need int(X * 1996800 / 8000000) = 100
    // X = ceil(100 * 8000000 / 1996800) = 401
    assert_eq!(pit.get_count(0, 401, CPU_HZ, PIT_HZ), 100);

    // At elapsed_pit = 200: pos = 0 -> returns 100
    // X = ceil(200 * 8000000 / 1996800) = 802
    assert_eq!(pit.get_count(0, 802, CPU_HZ, PIT_HZ), 100);

    // At elapsed_pit = 50: pos = 50 % 100 = 50 -> count = 50
    // X such that int(X * 1996800 / 8000000) = 50 -> X = ceil(50 * 8000000 / 1996800) = 201
    assert_eq!(pit.get_count(0, 201, CPU_HZ, PIT_HZ), 50);
}

#[test]
fn value_zero_65536_low_byte_mode() {
    let mut pit = I8253Pit::new_zeroed();

    // Low-byte-only mode, mode 2
    pit.write_control(0, 0x14, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x00); // value = 0 -> period = 65536
    pit.channels[0].last_load_cycle = 0;

    assert_eq!(pit.channels[0].value, 0);

    // At cycle 0: returns ch.value = 0
    assert_eq!(pit.get_count(0, 0, CPU_HZ, PIT_HZ), 0);

    // After some time, uses period 65536.
    // At cpu_cycle=401: elapsed_pit = 401*1996800/8000000 = 100
    // pos = 100 % 65536 = 100, count = 65536 - 100 = 65436
    assert_eq!(pit.get_count(0, 401, CPU_HZ, PIT_HZ), 65436);

    // High-byte-only mode, mode 2
    pit.write_control(0, 0x24, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x00); // value = 0 -> period = 65536
    pit.channels[0].last_load_cycle = 0;

    assert_eq!(pit.channels[0].value, 0);
    assert_eq!(pit.get_count(0, 401, CPU_HZ, PIT_HZ), 65436);
}

#[test]
fn mode1_inhibit_behavior() {
    let mut pit = I8253Pit::new_zeroed();

    // Mode 1 (hardware retriggerable one-shot): ctrl bits 3:1 = 001
    // Word access: RL = 0x30, mode 1 = 0x02 -> control word = 0x32
    pit.write_control(0, 0x32, 0, CPU_HZ, PIT_HZ);

    // Set PIT_FLAG_I to test inhibit.
    pit.channels[0].flag |= 0x20; // PIT_FLAG_I

    // Write LSB (first byte of word).
    assert_eq!(pit.write_counter(0, 0x64), WriteResult::Skip);

    // Write MSB (second byte). Mode 1 + PIT_FLAG_I -> should return Skip (inhibit).
    let result = pit.write_counter(0, 0x00);
    assert_eq!(
        result,
        WriteResult::Skip,
        "mode 1 with PIT_FLAG_I should inhibit"
    );
    assert_eq!(pit.channels[0].value, 100);

    // Without PIT_FLAG_I, same mode should return InitialLoad.
    pit.channels[0].flag &= !0x20; // clear PIT_FLAG_I
    // Need to set PIT_STAT_CMD again so write_counter works.
    pit.write_control(0, 0x32, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x64);
    let result = pit.write_counter(0, 0x00);
    assert_eq!(
        result,
        WriteResult::InitialLoad,
        "mode 1 without PIT_FLAG_I should not inhibit"
    );
}

#[test]
fn live_count_toggle_alternation() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: word access, mode 2, reload = 0x1234
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x34);
    pit.write_counter(0, 0x12);
    pit.channels[0].last_load_cycle = 0;

    // No latch - reads toggle between low and high bytes.
    assert_eq!(pit.channels[0].flag & 0x10, 0); // PIT_FLAG_C clear

    let low1 = pit.read_counter(0, 0, CPU_HZ, PIT_HZ);
    assert_eq!(low1, 0x34);
    let high1 = pit.read_counter(0, 0, CPU_HZ, PIT_HZ);
    assert_eq!(high1, 0x12);

    let low2 = pit.read_counter(0, 0, CPU_HZ, PIT_HZ);
    assert_eq!(low2, 0x34);
    let high2 = pit.read_counter(0, 0, CPU_HZ, PIT_HZ);
    assert_eq!(high2, 0x12);
}

#[test]
fn reprogram_counter_mid_count() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: word access, mode 2, reload = 1000
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8);
    pit.write_counter(0, 0x03);
    pit.channels[0].last_load_cycle = 0;

    // At cycle 200: count = some positive value (timer running).
    let count_mid = pit.get_count(0, 200, CPU_HZ, PIT_HZ);
    assert!(count_mid > 0 && count_mid < 1000, "count was {count_mid}");

    // Reprogram with new control word and value 500.
    pit.write_control(0, 0x34, 200, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xF4);
    pit.write_counter(0, 0x01); // value = 500
    pit.channels[0].last_load_cycle = 200;

    // At cycle 200: count = 500 (just reloaded, 0 elapsed from new load)
    assert_eq!(pit.get_count(0, 200, CPU_HZ, PIT_HZ), 500);

    // At cycle 300: elapsed = 100 cpu cycles from load
    // elapsed_pit = 100 * 1996800 / 8000000 = 24
    // pos = 24 % 500 = 24, count = 500 - 24 = 476
    assert_eq!(pit.get_count(0, 300, CPU_HZ, PIT_HZ), 476);
}

#[test]
fn counter_read_immediately_after_write() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: word access, mode 2
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8);
    pit.write_counter(0, 0x03);
    pit.channels[0].last_load_cycle = 0;

    // Latch and read at same cycle as load.
    pit.write_control(0, 0x00, 0, CPU_HZ, PIT_HZ);
    let low = pit.read_counter(0, 0, CPU_HZ, PIT_HZ);
    let high = pit.read_counter(0, 0, CPU_HZ, PIT_HZ);
    let count = (high as u16) << 8 | low as u16;
    assert_eq!(count, 1000);

    // At cycle 1: 1 cpu cycle = 0.2496 PIT ticks -> truncated to 0 elapsed.
    pit.write_control(0, 0x00, 1, CPU_HZ, PIT_HZ);
    let low = pit.read_counter(0, 1, CPU_HZ, PIT_HZ);
    let high = pit.read_counter(0, 1, CPU_HZ, PIT_HZ);
    let count = (high as u16) << 8 | low as u16;
    assert_eq!(count, 1000);
}

#[test]
fn on_timer0_event_mode3_rearms_interrupt() {
    let mut pit = I8253Pit::new_zeroed();

    // Ch0: mode 3 (square wave, periodic), PIT_FLAG_I armed, reload = 1000.
    pit.channels[0].ctrl = 0x36;
    pit.channels[0].flag = PIT_FLAG_I;
    pit.channels[0].value = 1000;

    // First event: should raise IRQ and re-arm for next period.
    let raised = pit.advance_timer0(0);
    assert!(raised, "first event should raise IRQ");
    assert_ne!(
        pit.channels[0].flag & PIT_FLAG_I,
        0,
        "mode 3 should re-arm PIT_FLAG_I"
    );

    // Second event: should raise IRQ again (periodic behavior).
    let raised = pit.advance_timer0(1000);
    assert!(raised, "second event should also raise IRQ");
    assert_ne!(
        pit.channels[0].flag & PIT_FLAG_I,
        0,
        "mode 3 should re-arm PIT_FLAG_I again"
    );
}

#[test]
fn on_timer0_event_mode0_does_not_rearm() {
    let mut pit = I8253Pit::new_zeroed();

    // Ch0: mode 0 (one-shot), PIT_FLAG_I armed, reload = 1000.
    pit.channels[0].ctrl = 0x30;
    pit.channels[0].flag = PIT_FLAG_I;
    pit.channels[0].value = 1000;

    // First event: should raise IRQ but NOT re-arm.
    let raised = pit.advance_timer0(0);
    assert!(raised, "first event should raise IRQ");
    assert_eq!(
        pit.channels[0].flag & PIT_FLAG_I,
        0,
        "mode 0 should NOT re-arm PIT_FLAG_I"
    );

    // Second event: should NOT raise IRQ (one-shot expired).
    let raised = pit.advance_timer0(1000);
    assert!(!raised, "second event should NOT raise IRQ in mode 0");
}

#[test]
fn mode3_decrement_by_2_half_period_boundary() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: word access, mode 3, reload = 1000
    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8);
    pit.write_counter(0, 0x03);
    pit.channels[0].last_load_cycle = 0;

    // cpu_cycles = 2004 -> elapsed_pit = 500 (half-period boundary, counter reloads).
    let count = pit.get_count(0, 2004, CPU_HZ, PIT_HZ);
    assert_eq!(
        count, 1000,
        "at half-period boundary, counter should reload"
    );
}

#[test]
fn mode3_decrement_by_2_second_half() {
    let mut pit = I8253Pit::new_zeroed();

    // Channel 0: word access, mode 3, reload = 1000
    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8);
    pit.write_counter(0, 0x03);
    pit.channels[0].last_load_cycle = 0;

    // In the second half (pos=600): pos_in_low = 600 - 500 = 100
    // count = 1000 - 100 * 2 = 800
    // cpu_cycles = 2404 -> elapsed_pit = 2404 * 1996800 / 8000000 = 600
    let count = pit.get_count(0, 2404, CPU_HZ, PIT_HZ);
    assert_eq!(count, 800);
}

#[test]
fn mode3_zero_reload() {
    let mut pit = I8253Pit::new_zeroed();

    // Mode 3, reload = 0 -> period = 65536
    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x00);
    pit.write_counter(0, 0x00);
    pit.channels[0].last_load_cycle = 0;

    // At cycle 0: returns 0 (the programmed value)
    assert_eq!(pit.get_count(0, 0, CPU_HZ, PIT_HZ), 0);

    // At cycle 401: elapsed_pit = 100, period = 65536, half = 32768
    // pos_in_half = 100, count = 65536 - 100 * 2 = 65336
    assert_eq!(pit.get_count(0, 401, CPU_HZ, PIT_HZ), 65336);
}

#[test]
fn mode2_vs_mode3_different_counts() {
    let mut pit = I8253Pit::new_zeroed();

    // Mode 2, reload = 100
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x64);
    pit.write_counter(0, 0x00);
    pit.channels[0].last_load_cycle = 0;

    // cpu_cycles = 41 -> elapsed_pit = 41 * 1996800 / 8000000 = 10
    // mode 2 count = 100 - 10 = 90
    let mode2_count = pit.get_count(0, 41, CPU_HZ, PIT_HZ);
    assert_eq!(mode2_count, 90);

    // Same reload in mode 3
    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x64);
    pit.write_counter(0, 0x00);
    pit.channels[0].last_load_cycle = 0;

    // elapsed_pit = 10: mode 3 pos_in_half = 10 % 50 = 10, count = 100 - 10 * 2 = 80
    let mode3_count = pit.get_count(0, 41, CPU_HZ, PIT_HZ);
    assert_eq!(mode3_count, 80);
}

#[test]
fn output_state_mode0_initial() {
    let mut pit = I8253Pit::new_zeroed();

    // Mode 0: output starts LOW after mode set
    pit.write_control(0, 0x30, 0, CPU_HZ, PIT_HZ);
    assert!(!pit.channels[0].output, "mode 0 output should start LOW");
}

#[test]
fn output_state_mode2_initial() {
    let mut pit = I8253Pit::new_zeroed();

    // Mode 2: output starts HIGH after mode set
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    assert!(pit.channels[0].output, "mode 2 output should start HIGH");
}

#[test]
fn output_state_mode3_initial() {
    let mut pit = I8253Pit::new_zeroed();

    // Mode 3: output starts HIGH after mode set
    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ);
    assert!(pit.channels[0].output, "mode 3 output should start HIGH");
}

#[test]
fn get_output_mode0() {
    let mut pit = I8253Pit::new_zeroed();

    pit.write_control(0, 0x30, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x0A);
    pit.write_counter(0, 0x00); // reload = 10
    pit.channels[0].last_load_cycle = 0;

    // Before terminal count: output LOW
    assert!(!pit.get_output(0, 0, CPU_HZ, PIT_HZ));

    // After terminal count: output HIGH
    // 10 PIT ticks = 41 cpu cycles
    assert!(pit.get_output(0, 100, CPU_HZ, PIT_HZ));
}

#[test]
fn get_output_mode3_halves() {
    let mut pit = I8253Pit::new_zeroed();

    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x64);
    pit.write_counter(0, 0x00); // reload = 100
    pit.channels[0].last_load_cycle = 0;

    // First half (pos < 50): output HIGH
    // elapsed_pit = 25: cpu_cycles = 25 * 8000000 / 1996800 ≈ 100
    assert!(pit.get_output(0, 100, CPU_HZ, PIT_HZ));

    // Second half (pos >= 50): output LOW
    // elapsed_pit = 75: cpu_cycles = 75 * 8000000 / 1996800 ≈ 300
    assert!(!pit.get_output(0, 301, CPU_HZ, PIT_HZ));
}

#[test]
fn mode0_lsb_write_drops_output() {
    let mut pit = I8253Pit::new_zeroed();

    // Mode 0, word access
    pit.write_control(0, 0x30, 0, CPU_HZ, PIT_HZ);
    assert!(!pit.channels[0].output);

    // Set output HIGH to simulate post-terminal-count state
    pit.channels[0].output = true;

    // Writing the LSB of a word write should drop output LOW
    pit.write_counter(0, 0x64);
    assert!(
        !pit.channels[0].output,
        "mode 0 LSB write should drop output LOW"
    );
}

#[test]
fn on_timer0_mode3_toggles_output() {
    let mut pit = I8253Pit::new_zeroed();

    pit.channels[0].ctrl = 0x36; // mode 3
    pit.channels[0].flag = PIT_FLAG_I;
    pit.channels[0].value = 1000;
    pit.channels[0].output = true;

    // First event: output toggles to false
    pit.advance_timer0(0);
    assert!(
        !pit.channels[0].output,
        "mode 3 first event should toggle output"
    );

    // Second event: output toggles back to true
    pit.advance_timer0(4000);
    assert!(
        pit.channels[0].output,
        "mode 3 second event should toggle output back"
    );
}

#[test]
fn on_timer0_mode0_sets_output_high() {
    let mut pit = I8253Pit::new_zeroed();

    pit.channels[0].ctrl = 0x30; // mode 0
    pit.channels[0].flag = PIT_FLAG_I;
    pit.channels[0].value = 1000;
    pit.channels[0].output = false;

    pit.advance_timer0(0);
    assert!(
        pit.channels[0].output,
        "mode 0 terminal count should set output HIGH"
    );
}

#[test]
fn deferred_reload_mode2() {
    let mut pit = I8253Pit::new_zeroed();

    // Initial load: mode 2, reload = 1000
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8);
    pit.write_counter(0, 0x03);
    pit.channels[0].last_load_cycle = 0;
    pit.channels[0].flag |= PIT_FLAG_I;

    // Subsequent load: write new value 500. Since mode 2 is periodic,
    // write_counter returns SubsequentLoad.
    let result = pit.write_counter(0, 0xF4);
    assert_eq!(result, WriteResult::Skip); // first byte (LSB)
    let result = pit.write_counter(0, 0x01);
    assert_eq!(result, WriteResult::SubsequentLoad);

    // The value field holds 500 but reload_pending is not set here
    // (that's the bus layer's job). Simulate what the bus does:
    pit.channels[0].reload_pending = Some(500);

    // Timer event with old period: apply pending reload.
    pit.advance_timer0(4000);
    assert_eq!(
        pit.channels[0].value, 500,
        "pending reload should be applied"
    );
    assert!(
        pit.channels[0].reload_pending.is_none(),
        "reload_pending should be cleared"
    );
}

#[test]
fn deferred_reload_mode3() {
    let mut pit = I8253Pit::new_zeroed();

    // Initial load: mode 3, reload = 1000
    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8);
    pit.write_counter(0, 0x03);
    pit.channels[0].last_load_cycle = 0;
    pit.channels[0].flag |= PIT_FLAG_I;

    // Simulate bus setting reload_pending
    pit.channels[0].reload_pending = Some(500);

    // Timer event: apply pending reload and toggle output.
    let initial_output = pit.channels[0].output;
    pit.advance_timer0(4000);
    assert_eq!(pit.channels[0].value, 500);
    assert_eq!(pit.channels[0].output, !initial_output);
}

#[test]
fn bcd_to_binary_conversion() {
    let mut pit = I8253Pit::new_zeroed();

    // Mode 2, BCD mode (ctrl bit 0 = 1): reload = 0x1000 BCD -> 1000 decimal ticks.
    // Control word: RL=11 (0x30), mode 2 (0x04), BCD (0x01) -> 0x35
    pit.write_control(0, 0x35, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x00); // LSB = 0x00
    pit.write_counter(0, 0x10); // MSB = 0x10 -> value = 0x1000
    pit.channels[0].last_load_cycle = 0;

    assert_eq!(pit.channels[0].value, 0x1000);

    // At cycle 0: count should be 0x1000 (the BCD reload value)
    assert_eq!(pit.get_count(0, 0, CPU_HZ, PIT_HZ), 0x1000);

    // cpu_cycles = 401 -> elapsed_pit = 100. Count = 1000 - 100 = 900 -> 0x0900 BCD
    assert_eq!(pit.get_count(0, 401, CPU_HZ, PIT_HZ), 0x0900);
}

#[test]
fn bcd_zero_reload_is_10000() {
    let mut pit = I8253Pit::new_zeroed();

    // BCD mode, reload = 0 -> period = 10000 decimal ticks
    pit.write_control(0, 0x35, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x00);
    pit.write_counter(0, 0x00);
    pit.channels[0].last_load_cycle = 0;

    // At cycle 0: returns 0 (the programmed value)
    assert_eq!(pit.get_count(0, 0, CPU_HZ, PIT_HZ), 0);

    // After 100 PIT ticks: count = 10000 - 100 = 9900 -> 0x9900 BCD
    assert_eq!(pit.get_count(0, 401, CPU_HZ, PIT_HZ), 0x9900);
}

#[test]
fn bcd_mode3_decrement_by_2() {
    let mut pit = I8253Pit::new_zeroed();

    // Mode 3, BCD, reload = 0x0100 BCD (100 decimal)
    // Control word: RL=11 (0x30), mode 3 (0x06), BCD (0x01) -> 0x37
    pit.write_control(0, 0x37, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0x00); // LSB
    pit.write_counter(0, 0x01); // MSB -> 0x0100
    pit.channels[0].last_load_cycle = 0;

    // Period = 100 decimal ticks, half = 50.
    // After 10 PIT ticks: pos_in_half = 10, count = 100 - 10*2 = 80 -> 0x0080 BCD
    // cpu_cycles for 10 PIT ticks: ~41
    assert_eq!(pit.get_count(0, 41, CPU_HZ, PIT_HZ), 0x0080);
}

#[test]
fn bcd_timer0_period_cycles() {
    let mut pit = I8253Pit::new_zeroed();

    // BCD mode, reload = 0x1000 (1000 decimal) -> period = 1000 PIT ticks
    pit.channels[0].ctrl = 0x35; // mode 2, BCD
    pit.channels[0].value = 0x1000;

    // Expected: 1000 * 8000000 / 1996800 = 4006 cpu cycles (integer division)
    let next = pit.timer0_period_cycles(CPU_HZ, PIT_HZ);
    assert_eq!(next, 4006);
}

#[test]
fn write_result_initial_vs_subsequent() {
    let mut pit = I8253Pit::new_zeroed();

    // Mode 2, word access - first load
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8); // LSB -> Skip
    let result = pit.write_counter(0, 0x03); // MSB -> InitialLoad
    assert_eq!(result, WriteResult::InitialLoad);

    // Second load without new control word -> SubsequentLoad
    pit.write_counter(0, 0xF4); // LSB -> Skip
    let result = pit.write_counter(0, 0x01); // MSB -> SubsequentLoad
    assert_eq!(result, WriteResult::SubsequentLoad);
}

#[test]
fn latch_not_cleared_by_mode_set() {
    let mut pit = I8253Pit::new_zeroed();

    // Mode 2, reload = 1000
    pit.write_control(0, 0x34, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8);
    pit.write_counter(0, 0x03);
    pit.channels[0].last_load_cycle = 0;

    // Latch the counter
    pit.write_control(0, 0x00, 0, CPU_HZ, PIT_HZ);

    // New mode set should clear the latch flag
    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ);
    assert_eq!(
        pit.channels[0].flag & 0x10,
        0,
        "mode set should clear latch flag"
    );
}

#[test]
fn reload_pending_cleared_on_mode_set() {
    let mut pit = I8253Pit::new_zeroed();

    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 0xE8);
    pit.write_counter(0, 0x03);
    pit.channels[0].reload_pending = Some(500);

    // Mode set clears reload_pending
    pit.write_control(0, 0x36, 0, CPU_HZ, PIT_HZ);
    assert!(pit.channels[0].reload_pending.is_none());
}

#[test]
fn mode3_odd_reload_period() {
    let mut pit = I8253Pit::new_zeroed();

    // Mode 3, low-byte only (RL=01), reload = 101 (odd).
    // Control word: RL=01 (0x10), mode 3 (0x06) -> 0x16
    pit.write_control(0, 0x16, 0, CPU_HZ, PIT_HZ);
    pit.write_counter(0, 101);
    pit.channels[0].last_load_cycle = 0;

    // The 8253 mode 3 odd-reload behavior:
    //   HIGH half: (101+1)/2 = 51 ticks
    //     tick 0: CE = 101
    //     tick 1: CE = 100 (decremented by 1 to make even)
    //     tick 2: CE = 98  (then by 2)
    //     tick k (k>=1): CE = 101 + 1 - k*2 = 102 - 2k
    //   LOW half: (101-1)/2 = 50 ticks
    //     tick 0: CE = 101 (reloaded)
    //     tick 1: CE = 98  (decremented by 3 to make even)
    //     tick k (k>=1): CE = 101 - 1 - k*2 = 100 - 2k
    //   Total period: 51 + 50 = 101 PIT ticks (correct, not 100).

    // Use a direct PIT-tick ratio for easy reasoning: set CPU_HZ = PIT_HZ
    // so 1 CPU cycle = 1 PIT tick.
    let hz = 1_000_000;

    // At tick 0: count = 101
    assert_eq!(pit.get_count(0, 0, hz, hz), 101);

    // At tick 1 (HIGH half): CE = 102 - 2*1 = 100
    assert_eq!(pit.get_count(0, 1, hz, hz), 100);

    // At tick 25 (HIGH half): CE = 102 - 2*25 = 52
    assert_eq!(pit.get_count(0, 25, hz, hz), 52);

    // At tick 50 (HIGH half, last tick): CE = 102 - 2*50 = 2
    assert_eq!(pit.get_count(0, 50, hz, hz), 2);

    // At tick 51 (LOW half, tick 0): CE = 101 (reloaded)
    assert_eq!(pit.get_count(0, 51, hz, hz), 101);

    // At tick 52 (LOW half, tick 1): CE = 100 - 2*1 = 98
    assert_eq!(pit.get_count(0, 52, hz, hz), 98);

    // At tick 100 (LOW half, last tick): pos_in_low = 100-51 = 49
    // CE = 100 - 2*49 = 2
    assert_eq!(pit.get_count(0, 100, hz, hz), 2);

    // At tick 101: new period starts, CE = 101 (reloaded)
    assert_eq!(pit.get_count(0, 101, hz, hz), 101);

    // Verify output state: HIGH for first 51 ticks, LOW for next 50.
    assert!(pit.get_output(0, 0, hz, hz)); // tick 0: HIGH
    assert!(pit.get_output(0, 50, hz, hz)); // tick 50: still HIGH
    assert!(!pit.get_output(0, 51, hz, hz)); // tick 51: LOW
    assert!(!pit.get_output(0, 100, hz, hz)); // tick 100: still LOW
    assert!(pit.get_output(0, 101, hz, hz)); // tick 101: HIGH again
}
