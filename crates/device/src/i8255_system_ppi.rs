//! i8255 System PPI - system configuration and control ports.
//!
//! Port B (0x42, read): system configuration status.
//! Port C (0x35 R/W, 0x37 W BSR): system control bits.
//! DIP switch 2 (0x31, read): boot/memory configuration.

/// Port B bit 7: SELECT# - printer select signal, active low.
/// 1 = no printer selected (active low).
const PORT_B_SELECT: u8 = 0x80;

/// Port B bit 5: MOD - system clock lineage.
/// 1 = 8 MHz series (timer 2.0 MHz), 0 = 5/10 MHz series (timer 2.5 MHz).
/// Ref: undoc98 `io_prn.txt` (I/O 0042h bit 5)
const PORT_B_CLOCK_8MHZ: u8 = 0x20;

/// Port B bit 4: LCD - mirrors DIP SW 1-3 (plasma display).
/// Ref: undoc98 `io_prn.txt` (I/O 0042h bit 4)
const _PORT_B_LCD: u8 = 0x10;

/// Port B bit 3: HGC - mirrors DIP SW 1-8 (graphics extension).
/// 1 = DIP SW 1-8 OFF = basic mode (no analog 16-color extension).
/// 0 = DIP SW 1-8 ON  = expanded mode (analog 16-color extension installed).
/// Ref: undoc98 `io_prn.txt` (I/O 0042h bit 3)
const PORT_B_GRAPHICS_EXT: u8 = 0x08;

/// Port B bit 1: CPUT - CPU type.
/// 1 = V30/V33, 0 = 8086/80286/80386 or later.
/// Ref: undoc98 `io_prn.txt` (I/O 0042h bit 1)
const PORT_B_CPU_V30: u8 = 0x02;

/// Port B base value for 5/10 MHz machines: SELECT# = 0x80.
/// No printer attached, so SELECT# (bit 7) is inactive-high.
/// BUSY# (bit 2) is composed dynamically from the printer device.
/// Ref: undoc98 `io_prn.txt` (I/O 0042h)
const PORT_B_BASE_10MHZ: u8 = PORT_B_SELECT;

/// Port B base value for 8 MHz machines: SELECT# | CLOCK_8MHZ = 0xA0.
/// BUSY# (bit 2) is composed dynamically from the printer device.
/// Ref: undoc98 `io_prn.txt` (I/O 0042h)
const PORT_B_BASE_8MHZ: u8 = PORT_B_SELECT | PORT_B_CLOCK_8MHZ;

/// Port C bit 7: SHUT0 - shutdown flag 0 (286+ machines).
/// Ref: undoc98 `io_syste.txt` (I/O 0035h bit 7)
const PORT_C_SHUT0: u8 = 0x80;

/// Port C bit 6: PSTBM - printer PSTB# signal mask.
/// Ref: undoc98 `io_syste.txt` (I/O 0035h bit 6)
const PORT_C_PSTBM: u8 = 0x40;

/// Port C bit 5: SHUT1 - shutdown flag 1 (286+ machines).
/// Ref: undoc98 `io_syste.txt` (I/O 0035h bit 5)
const PORT_C_SHUT1: u8 = 0x20;

/// Port C bit 4: MCHKEN - RAM parity check enable.
/// Ref: undoc98 `io_syste.txt` (I/O 0035h bit 4)
const PORT_C_MCHKEN: u8 = 0x10;

/// Port C bit 3: BUZ - internal beeper (1 = stop, 0 = sound).
/// Ref: undoc98 `io_syste.txt` (I/O 0035h bit 3)
const PORT_C_BUZ_STOP: u8 = 0x08;

/// Port C bit 2: TXRE - RS-232C TXRDY interrupt enable.
/// Ref: undoc98 `io_syste.txt` (I/O 0035h bit 2)
const _PORT_C_TXRE: u8 = 0x04;

/// Port C bit 1: TXEE - RS-232C TXEMPTY interrupt enable.
/// Ref: undoc98 `io_syste.txt` (I/O 0035h bit 1)
const _PORT_C_TXEE: u8 = 0x02;

/// Port C bit 0: RXRE - RS-232C RXRDY interrupt enable.
/// Ref: undoc98 `io_syste.txt` (I/O 0035h bit 0)
const PORT_C_RXRE: u8 = 0x01;

/// Port C reset value: 0xF9 = SHUT0 | PSTBM | SHUT1 | MCHKEN | BUZ_STOP | RXRE.
/// All shutdown/control flags set, beeper stopped, only RXRE interrupt enabled.
/// TXRE and TXEE are cleared (no RS-232C transmit interrupts).
const PORT_C_RESET: u8 =
    PORT_C_SHUT0 | PORT_C_PSTBM | PORT_C_SHUT1 | PORT_C_MCHKEN | PORT_C_BUZ_STOP | PORT_C_RXRE;

/// Control register bit 7: mode select.
/// 1 = mode set command (resets port C to 0), 0 = BSR (bit set/reset).
/// Ref: undoc98 `io_syste.txt` (I/O 0037h)
const CONTROL_MODE_SET_BIT: u8 = 0x80;

/// BSR bit position mask: bits 3:1 select which port C bit to set/reset.
/// Ref: undoc98 `io_syste.txt` (I/O 0037h)
const BSR_BIT_SELECT_MASK: u8 = 0x07;

/// DIP SW 2-1 (bit 0): System specification.
/// OFF (1) = normal boot path, ON (0) = alternate card boot.
const DIPSW2_SYSTEM_SPEC: u8 = 0x01;

/// DIP SW 2-2 (bit 1): Terminal mode.
/// OFF (1) = disabled, ON (0) = enabled.
const DIPSW2_TERMINAL_MODE: u8 = 0x02;

/// DIP SW 2-3 (bit 2): Text column width.
/// OFF (1) = 40 chars/line, ON (0) = 80 chars/line.
const _DIPSW2_TEXT_WIDTH_40: u8 = 0x04;

/// DIP SW 2-4 (bit 3): Text line height.
/// OFF (1) = 20 lines/screen, ON (0) = 25 lines/screen.
const _DIPSW2_TEXT_HEIGHT_20: u8 = 0x08;

/// DIP SW 2-5 (bit 4): Memory switch initialization.
/// OFF (1) = initialize memory switch with system defaults, ON (0) = keep current.
const DIPSW2_MEMSW_INIT: u8 = 0x10;

/// DIP SW 2-6 (bit 5): Unused (OFF on VM).
const DIPSW2_UNUSED_6: u8 = 0x20;

/// DIP SW 2-7 (bit 6): Unused (OFF on VM).
const DIPSW2_UNUSED_7: u8 = 0x40;

/// DIP SW 2-8 (bit 7): GDC clock (on VM and later).
/// OFF (1) = 2.5 MHz, ON (0) = 5 MHz.
const DIPSW2_GDC_CLOCK_2_5MHZ: u8 = 0x80;

/// Default for DIP switch 2: 0xF3.
///
/// Shared by all emulated models (VM, VX, RA).
///
/// Bit layout (`1111_0011b`):
///   SW2-1 (bit 0) = 1 OFF: normal boot (System Specification off)
///   SW2-2 (bit 1) = 1 OFF: terminal mode disabled
///   SW2-3 (bit 2) = 0 ON:  80 chars/line
///   SW2-4 (bit 3) = 0 ON:  25 lines/screen
///   SW2-5 (bit 4) = 1 OFF: initialize memory switch with system defaults
///   SW2-6 (bit 5) = 1 OFF: unused
///   SW2-7 (bit 6) = 1 OFF: unused
///   SW2-8 (bit 7) = 1 OFF: GDC clock 2.5 MHz
const DIPSW2_DEFAULT: u8 = DIPSW2_SYSTEM_SPEC
    | DIPSW2_TERMINAL_MODE
    | DIPSW2_MEMSW_INIT
    | DIPSW2_UNUSED_6
    | DIPSW2_UNUSED_7
    | DIPSW2_GDC_CLOCK_2_5MHZ;

/// Snapshot of the system PPI state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I8255SystemPpiState {
    /// Port B - system configuration status (read-only via port 0x42).
    ///
    /// Bit layout (reference: `undoc98/io_prn.txt`):
    ///   bit 7: SELECT# - printer select signal, active low (1 = no printer selected)
    ///   bit 6: unused (0)
    ///   bit 5: CPU clock lineage - 1 = 8MHz series, 0 = 5/10MHz series
    ///   bit 4: LCD mode - mirrors DIP SW 1-3
    ///   bit 3: GFX extension - mirrors DIP SW 1-8
    ///   bit 2: BUSY# - printer busy signal, active low (1 = printer not busy)
    ///   bit 1: CPU type - 1 = V30/V33, 0 = 8086/80286/80386 or later
    ///   bit 0: VF flag - 1 = PC-9801VF/U variant
    pub port_b: u8,
    /// Port C - system control (read/write via port 0x35, BSR via port 0x37).
    ///
    /// Bit layout (reference: `undoc98/io_syste.txt`):
    ///   bit 7: SHUT0 - shutdown flag 0
    ///   bit 6: PSTBM - printer PSTB# signal mask
    ///   bit 5: SHUT1 - shutdown flag 1
    ///   bit 4: MCHKEN - RAM parity check enable
    ///   bit 3: BUZ - internal beeper (1 = stop, 0 = sound)
    ///   bit 2: TXRE - RS-232C TXRDY interrupt enable
    ///   bit 1: TXEE - RS-232C TXEMPTY interrupt enable
    ///   bit 0: RXRE - RS-232C RXRDY interrupt enable
    pub port_c: u8,
    /// DIP switch 2 register (read-only via port 0x31).
    ///
    /// Bit mapping is `SW2:8..SW2:1` (bit7..bit0), where `1=OFF`, `0=ON`.
    /// Default `0xF3` (`1111_0011b`) for all models:
    /// - SW2-1 (bit0)=1 OFF: normal boot path
    /// - SW2-2 (bit1)=1 OFF: terminal mode disabled
    /// - SW2-3 (bit2)=0 ON: 80 chars/line
    /// - SW2-4 (bit3)=0 ON: 25 lines/screen
    /// - SW2-5 (bit4)=1 OFF: initialize memory switch
    /// - SW2-6 (bit5)=1 OFF: unused
    /// - SW2-7 (bit6)=1 OFF: unused
    /// - SW2-8 (bit7)=1 OFF: GDC clock 2.5 MHz
    ///
    /// Ref: undoc98 `io_syste.txt` (I/O 0031h)
    pub dip_switch_2: u8,
    /// RS-232C modem signals (bits 7-5 of port 0x33).
    ///
    /// Bit layout:
    ///   bit 7: CI#  - Carrier In (1=inactive/no modem)
    ///   bit 6: CS#  - Clear to Send (1=inactive/no modem)
    ///   bit 5: CD#  - Carrier Detect (1=inactive/no modem)
    ///
    /// Default `0xE0`: all lines inactive (no modem attached).
    /// Ref: undoc98 `io_syste.txt`
    pub rs232c_modem_signals: u8,
    /// CRT type flag (bit 3 of port 0x33).
    ///
    /// Derived from `(~DIP_SW_1) & 1` (DIP switch 1-1).
    /// `true` = standard-resolution display (bit 3 set).
    /// Ref: undoc98 `io_syste.txt`
    pub crtt: bool,
}

/// i8255 System PPI controller.
pub struct I8255SystemPpi {
    /// Embedded state for save/restore.
    pub state: I8255SystemPpiState,
}

impl Default for I8255SystemPpi {
    fn default() -> Self {
        Self::new(false)
    }
}

impl I8255SystemPpi {
    /// Creates a new system PPI.
    ///
    /// `is_8mhz_lineage`: true for 8MHz-series machines (sets bit 5 in port B).
    pub fn new(is_8mhz_lineage: bool) -> Self {
        let port_b = if is_8mhz_lineage {
            PORT_B_BASE_8MHZ
        } else {
            PORT_B_BASE_10MHZ
        };

        Self {
            state: I8255SystemPpiState {
                port_b,
                port_c: PORT_C_RESET,
                dip_switch_2: DIPSW2_DEFAULT,
                rs232c_modem_signals: 0xE0,
                crtt: true,
            },
        }
    }

    /// Reads port B (port 0x42).
    pub fn read_port_b(&self) -> u8 {
        self.state.port_b
    }

    /// Reads port C (port 0x35).
    pub fn read_port_c(&self) -> u8 {
        self.state.port_c
    }

    /// Writes port C (port 0x35).
    pub fn write_port_c(&mut self, value: u8) {
        self.state.port_c = value;
    }

    /// Writes the control register (port 0x37).
    ///
    /// Bit 7 = 1: mode set command - resets all port C output bits to 0.
    /// Bit 7 = 0: BSR (bit set/reset) on port C.
    /// Ref: undoc98 `io_syste.txt` (I/O 0037h)
    pub fn write_control(&mut self, value: u8) {
        if value & CONTROL_MODE_SET_BIT != 0 {
            self.state.port_c = 0x00;
        } else {
            let bit = 1u8 << ((value >> 1) & BSR_BIT_SELECT_MASK);
            if value & 1 != 0 {
                self.state.port_c |= bit;
            } else {
                self.state.port_c &= !bit;
            }
        }
    }

    /// Reads DIP switch 2 (port 0x31).
    pub fn read_dip_switch_2(&self) -> u8 {
        self.state.dip_switch_2
    }

    /// Reads RS-232C / system status (port 0x33).
    ///
    /// Composite register combining RS-232C modem signals, CRT type, and other system bits.
    ///
    /// Bit layout (reference: undoc98 `io_syste.txt`):
    ///   bit 7: CI#   - RS-232C Carrier In (1=inactive/no modem)
    ///   bit 6: CS#   - RS-232C Clear to Send (1=inactive/no modem)
    ///   bit 5: CD#   - RS-232C Carrier Detect (1=inactive/no modem)
    ///   bit 4: INT3  - Expansion bus INT3 signal (0=inactive)
    ///   bit 3: CRTT  - CRT type from DIP SW 1-1 (1=standard-res)
    ///   bit 2: IMCK  - Built-in RAM parity error (0=none)
    ///   bit 1: EMCK  - Extended slot parity error (0=none)
    ///   bit 0: CDAT  - µPD4990A RTC calendar clock serial data out
    pub fn read_rs232c_status(&self) -> u8 {
        let mut value = self.state.rs232c_modem_signals & 0xE0;
        if self.state.crtt {
            value |= 0x08;
        }
        // Bit 0 (CDAT): composed by the bus from the µPD4990A RTC.
        // Bits 4, 2, 1: INT3/IMCK/EMCK - always 0 (no expansion bus interrupt, no parity errors).
        value
    }

    /// Sets the CPU type bit in port B.
    ///
    /// V30/V33 CPUs set bit 1; 8086/286/386 clear it.
    /// Ref: undoc98 `io_prn.txt` (I/O 0042h bit 1)
    pub fn set_cpu_mode_bit(&mut self, is_v30: bool) {
        if is_v30 {
            self.state.port_b |= PORT_B_CPU_V30;
        } else {
            self.state.port_b &= !PORT_B_CPU_V30;
        }
    }

    /// Sets the graphics extension bit (HGC, bit 3) in port B.
    /// Ref: undoc98 `io_prn.txt` (I/O 0042h bit 3)
    pub fn set_graphics_extension_bit(&mut self, enabled: bool) {
        if enabled {
            self.state.port_b &= !PORT_B_GRAPHICS_EXT;
        } else {
            self.state.port_b |= PORT_B_GRAPHICS_EXT;
        }
    }
}
