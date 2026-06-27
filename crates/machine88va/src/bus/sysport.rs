//! PC-88VA2 system ports, calendar strobe latches, and DIP-switch state
//! (SYSPORTVA).

/// SYSPORTVA register/latch state.
pub struct SysPortVa {
    /// Port 0x1C9 read: bit7,6,0 fixed 1, bit5 SPEED, bits4:1 DIP sw5-2.
    pub(crate) a: u8,
    /// Port 0x1CD latch (RS-232C / sysport control bits).
    pub(crate) c: u8,
    /// Port 0x010 write latch (RTC C0-C2/data, printer data).
    pub(crate) port010: u8,
    /// Port 0x032 latch (interrupt-mask register; bit7 = FM IRQ mask).
    pub(crate) port032: u8,
    /// Port 0x040 write latch (RTC STB/CLK, mouse strobe).
    pub(crate) port040: u8,
    /// Port 0x190 latch (system port 5, audio control).
    pub(crate) port190: u8,
    /// System operation mode word, read at 0x150/0x151, written via 0x1C6.
    pub(crate) modesw: u16,
    /// DIP-switch configuration. Bit0 selects the CRT mode (1 = 24 kHz,
    /// 0 = 15 kHz) until the video controller takes over.
    pub(crate) dipsw: u8,
}

impl SysPortVa {
    /// Creates the reset-state system ports.
    pub(crate) fn new() -> Self {
        let mut sysport = Self {
            a: 0,
            c: 0,
            port010: 0,
            port032: 0,
            port040: 0,
            port190: 0,
            modesw: 0,
            dipsw: 0xCD,
        };
        sysport.reset();
        sysport
    }

    fn reset(&mut self) {
        self.a |= 0xC1;
        self.c = 0xF9;
        self.port010 = 0;
        self.port040 = 0;
        self.port190 &= 0x01;
        self.port190 |= 0x18;
    }

    /// True when the configured CRT mode is 24.8 kHz (DIP bit0 set).
    pub(crate) fn crt_mode_24khz(&self) -> bool {
        self.dipsw & 0x01 != 0
    }
}

/// Returns the current host local time as the 6-byte BCD buffer the uPD4990A
/// TIME_READ command expects: `[year, month<<4|day_of_week, day, hour, minute,
/// second]`.
pub(crate) fn default_local_time() -> [u8; 6] {
    fn to_bcd(value: u8) -> u8 {
        ((value / 10) << 4) | (value % 10)
    }
    use std::time::SystemTime;
    let seconds = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let days = (seconds / 86_400) as u32;
    let time_of_day = (seconds % 86_400) as u32;
    let hour = (time_of_day / 3_600) as u8;
    let minute = ((time_of_day % 3_600) / 60) as u8;
    let second = (time_of_day % 60) as u8;
    // 1970-01-01 was a Thursday (day_of_week 4).
    let day_of_week = ((days + 4) % 7) as u8;
    let mut year = 1970u32;
    let mut remaining = days;
    loop {
        let year_days =
            if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) {
                366
            } else {
                365
            };
        if remaining < year_days {
            break;
        }
        remaining -= year_days;
        year += 1;
    }
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u8;
    for &days_in_month in &month_days {
        if remaining < days_in_month {
            break;
        }
        remaining -= days_in_month;
        month += 1;
    }
    let day = remaining as u8 + 1;
    [
        to_bcd((year % 100) as u8),
        (month << 4) | day_of_week,
        to_bcd(day),
        to_bcd(hour),
        to_bcd(minute),
        to_bcd(second),
    ]
}
