//! PC-88VA2 system ports, calendar strobe latches, and DIP-switch state
//! (SYSPORTVA).

/// SYSPORTVA register/latch state.
pub struct SysPortVa {
    /// Port 0x1C9 read: bit7,6,0 fixed 1, bit5 SPEED, bits4:1 DIP sw5-2.
    pub a: u8,
    /// Port 0x1CD latch (RS-232C / sysport control bits).
    pub c: u8,
    /// Port 0x010 write latch (RTC C0-C2/data, printer data).
    pub port010: u8,
    /// Port 0x032 latch (interrupt-mask register; bit7 = FM IRQ mask).
    pub port032: u8,
    /// Port 0x040 write latch (RTC STB/CLK, mouse strobe).
    pub port040: u8,
    /// Port 0x190 latch (system port 5, audio control).
    pub port190: u8,
    /// System operation mode word, read at 0x150/0x151, written via 0x1C6.
    pub modesw: u16,
    /// DIP-switch configuration. Bit0 selects the CRT mode (1 = 24 kHz,
    /// 0 = 15 kHz) until the video controller takes over.
    pub dipsw: u8,
}

impl Default for SysPortVa {
    fn default() -> Self {
        Self::new()
    }
}

impl SysPortVa {
    /// Creates the reset-state system ports.
    pub fn new() -> Self {
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
    pub fn crt_mode_24khz(&self) -> bool {
        self.dipsw & 0x01 != 0
    }
}
