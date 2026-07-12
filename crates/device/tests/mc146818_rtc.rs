use common::HostDateTime;
use device::mc146818_rtc::{CMOS_SIZE, Mc146818Rtc};

const CPU_HZ: u32 = 33_000_000;

/// A CMOS seed with register B configured for 24-hour BCD operation.
fn bcd_seed() -> [u8; CMOS_SIZE] {
    let mut seed = [0u8; CMOS_SIZE];
    seed[0x0A] = 0x26; // divider 010, rate 1024 Hz
    seed[0x0B] = 0x02; // 24-hour, BCD
    seed
}

/// A CMOS seed with register B configured for 24-hour binary operation.
fn binary_seed() -> [u8; CMOS_SIZE] {
    let mut seed = bcd_seed();
    seed[0x0B] = 0x06; // 24-hour, binary
    seed
}

fn datetime(hour: u8, minute: u8, second: u8) -> HostDateTime {
    HostDateTime {
        year: 2026,
        month: 7,
        day: 12,
        day_of_week: 0, // Sunday
        hour,
        minute,
        second,
    }
}

#[test]
fn seeds_bcd_time_and_century() {
    let mut rtc = Mc146818Rtc::new(datetime(13, 45, 30), &bcd_seed());
    rtc.set_address(0x00);
    assert_eq!(rtc.read(0, CPU_HZ), 0x30); // seconds 30 BCD
    rtc.set_address(0x04);
    assert_eq!(rtc.read(0, CPU_HZ), 0x13); // hours 13 BCD
    rtc.set_address(0x09);
    assert_eq!(rtc.read(0, CPU_HZ), 0x26); // year 26 BCD
    rtc.set_address(0x32);
    assert_eq!(rtc.read(0, CPU_HZ), 0x20); // century 20 BCD
}

#[test]
fn seeds_binary_time() {
    let mut rtc = Mc146818Rtc::new(datetime(13, 45, 30), &binary_seed());
    rtc.set_address(0x00);
    assert_eq!(rtc.read(0, CPU_HZ), 30);
    rtc.set_address(0x04);
    assert_eq!(rtc.read(0, CPU_HZ), 13);
}

#[test]
fn one_second_rollover_bcd() {
    let mut rtc = Mc146818Rtc::new(datetime(23, 59, 59), &bcd_seed());
    let irq = rtc.advance_one_second();
    assert!(!irq); // no update-ended interrupt enabled

    rtc.set_address(0x00);
    assert_eq!(rtc.read(0, CPU_HZ), 0x00); // seconds
    rtc.set_address(0x02);
    assert_eq!(rtc.read(0, CPU_HZ), 0x00); // minutes
    rtc.set_address(0x04);
    assert_eq!(rtc.read(0, CPU_HZ), 0x00); // hours
    rtc.set_address(0x07);
    assert_eq!(rtc.read(0, CPU_HZ), 0x13); // day rolled 12 -> 13
    rtc.set_address(0x06);
    assert_eq!(rtc.read(0, CPU_HZ), 0x02); // day of week Sunday(1) -> Monday(2)
}

#[test]
fn leap_day_rollover_binary() {
    let mut seed = binary_seed();
    // 2028 is a leap year; day-of-week value is irrelevant here.
    let dt = HostDateTime {
        year: 2028,
        month: 2,
        day: 28,
        day_of_week: 2,
        hour: 23,
        minute: 59,
        second: 59,
    };
    seed[0x0B] = 0x06;
    let mut rtc = Mc146818Rtc::new(dt, &seed);
    rtc.advance_one_second();
    rtc.set_address(0x07);
    assert_eq!(rtc.read(0, CPU_HZ), 29); // Feb 29 exists in 2028
    rtc.set_address(0x08);
    assert_eq!(rtc.read(0, CPU_HZ), 2); // still February
}

#[test]
fn twelve_hour_pm_flag() {
    let mut seed = bcd_seed();
    seed[0x0B] = 0x00; // 12-hour, BCD
    let mut rtc = Mc146818Rtc::new(datetime(13, 0, 0), &seed);
    rtc.set_address(0x04);
    let hours = rtc.read(0, CPU_HZ);
    // 13:00 -> 1 PM: BCD 01 with the PM flag (0x80) set.
    assert_eq!(hours, 0x81);
}

#[test]
fn periodic_rate_table() {
    let mut rtc = Mc146818Rtc::new(datetime(0, 0, 0), &bcd_seed());

    rtc.set_address(0x0A);
    rtc.write(0x20); // rate 0 -> disabled
    assert_eq!(rtc.periodic_period_cycles(CPU_HZ), None);

    rtc.write(0x26); // rate 6 -> 1024 Hz
    assert_eq!(
        rtc.periodic_period_cycles(CPU_HZ),
        Some(CPU_HZ as u64 / 1024)
    );

    rtc.write(0x21); // rate 1 -> 256 Hz (special)
    assert_eq!(
        rtc.periodic_period_cycles(CPU_HZ),
        Some(CPU_HZ as u64 / 256)
    );

    rtc.write(0x2F); // rate 15 -> 2 Hz
    assert_eq!(rtc.periodic_period_cycles(CPU_HZ), Some(CPU_HZ as u64 / 2));
}

#[test]
fn periodic_interrupt_gated_by_pie() {
    let mut rtc = Mc146818Rtc::new(datetime(0, 0, 0), &bcd_seed());

    // With PIE clear, a tick sets PF but raises no interrupt.
    assert!(!rtc.periodic_tick());
    rtc.set_address(0x0C);
    assert_eq!(rtc.read(0, CPU_HZ) & 0x40, 0x40); // PF set
    // Reading register C cleared the flags.
    assert_eq!(rtc.read(0, CPU_HZ), 0x00);

    // Enable PIE; now a tick raises the interrupt.
    rtc.set_address(0x0B);
    rtc.write(0x42); // 24h off but PIE on; only PIE matters here
    assert!(rtc.periodic_tick());
}

#[test]
fn alarm_match_with_dont_care() {
    let mut rtc = Mc146818Rtc::new(datetime(10, 20, 29), &binary_seed());

    // Alarm at any second (0xFF don't care), minute 20, hour 10; enable AIE.
    rtc.set_address(0x01);
    rtc.write(0xFF); // seconds alarm: don't care
    rtc.set_address(0x03);
    rtc.write(20); // minutes alarm
    rtc.set_address(0x05);
    rtc.write(10); // hours alarm
    rtc.set_address(0x0B);
    rtc.write(0x26); // 24h, binary, AIE

    // Advancing to 10:20:30 must fire the alarm.
    let irq = rtc.advance_one_second();
    assert!(irq);
    rtc.set_address(0x0C);
    assert_eq!(rtc.read(0, CPU_HZ) & 0x20, 0x20); // AF set
}

#[test]
fn register_c_reads_clear_flags() {
    let mut rtc = Mc146818Rtc::new(datetime(0, 0, 0), &bcd_seed());
    rtc.set_address(0x0B);
    rtc.write(0x12); // 24h, UIE
    rtc.advance_one_second();

    rtc.set_address(0x0C);
    let first = rtc.read(0, CPU_HZ);
    assert_eq!(first & 0x10, 0x10); // UF set
    assert_eq!(first & 0x80, 0x80); // IRQF set
    // Second read is cleared.
    assert_eq!(rtc.read(0, CPU_HZ), 0x00);
}

#[test]
fn uip_window_analytic() {
    let mut rtc = Mc146818Rtc::new(datetime(0, 0, 0), &bcd_seed());
    rtc.set_next_update_cycle(1_000_000);
    rtc.set_address(0x0A);

    // Well before the update: UIP clear.
    assert_eq!(rtc.read(0, CPU_HZ) & 0x80, 0x00);

    // Within the ~2228 us window before the update: UIP set.
    let window = 2228 * CPU_HZ as u64 / 1_000_000;
    assert_eq!(rtc.read(1_000_000 - window + 10, CPU_HZ) & 0x80, 0x80);
}

#[test]
fn set_bit_freezes_updates_and_uip() {
    let mut rtc = Mc146818Rtc::new(datetime(12, 0, 0), &bcd_seed());
    rtc.set_address(0x0B);
    rtc.write(0x82); // 24h + SET
    rtc.set_next_update_cycle(1000);

    // UIP reads clear while SET is asserted.
    rtc.set_address(0x0A);
    assert_eq!(rtc.read(999, CPU_HZ) & 0x80, 0x00);

    // advance_one_second is a no-op while SET is asserted.
    assert!(!rtc.advance_one_second());
    rtc.set_address(0x00);
    assert_eq!(rtc.read(0, CPU_HZ), 0x00); // seconds unchanged
}

#[test]
fn register_d_reads_battery_good() {
    let mut rtc = Mc146818Rtc::new(datetime(0, 0, 0), &bcd_seed());
    rtc.set_address(0x0D);
    assert_eq!(rtc.read(0, CPU_HZ), 0x80);
}
