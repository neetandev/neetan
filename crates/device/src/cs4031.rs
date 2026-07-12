//! Chips & Technologies CS4031 chipset configuration core for the PC/AT.
//!
//! This models the parts of the CS4031 that are unique to the chipset: the
//! indexed configuration registers at ports 0x22/0x23, the shadow-RAM/ROMCS
//! memory decode for the C0000-FFFFF region, the fast Gate A20 and fast CPU
//! reset paths (port 0x92 and the register-0x1C emulation), the "port B"
//! system-control register (port 0x61), the NMI mask and RTC address latch
//! (port 0x70), and the keyboard-command interception state machine.

/// Register index: DMA wait-state control (stored, timing effects deferred).
pub const CS4031_REG_DMA_WAIT_STATE: u8 = 0x01;
/// Register index: DMA clock selection (stored, timing effects deferred).
pub const CS4031_REG_DMA_CLOCK: u8 = 0x0A;
/// Register index: internal DRAM mapping for A0000 and B0000.
pub const CS4031_REG_SHADOW_AB: u8 = 0x18;
/// Register index: per-region DRAM shadow read enable.
pub const CS4031_REG_SHADOW_READ: u8 = 0x19;
/// Register index: per-region DRAM shadow write enable.
pub const CS4031_REG_SHADOW_WRITE: u8 = 0x1A;
/// Register index: per-region ROMCS enable (bit 7 also enables ROM writes).
pub const CS4031_REG_ROMCS: u8 = 0x1B;
/// Register index: soft reset and Gate A20 emulation control.
///
/// Bit 4 enables emulated keyboard reset, bit 5 enables emulated Gate A20,
/// bit 7 blocks keyboard commands the chipset chooses to intercept.
pub const CS4031_REG_SOFT_RESET_GATEA20: u8 = 0x1C;

/// Power-on value of the ROMCS register: regions 5 (E0000) and 6 (F0000) enabled.
const ROMCS_RESET_VALUE: u8 = 0x60;

/// Number of shadow-controlled UMA regions (four 16 KiB blocks in C0000-CFFFF,
/// then the D0000, E0000 and F0000 64 KiB blocks).
pub const CS4031_UMA_REGION_COUNT: usize = 7;

/// Source a UMA region read resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionReadSource {
    /// Read from the option/system ROM behind the region.
    Rom,
    /// Read from shadow DRAM.
    Ram,
    /// Nothing decoded: open bus (ISA).
    OpenBus,
}

/// Target a UMA region write resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionWriteTarget {
    /// Write into shadow DRAM.
    Ram,
    /// Write is discarded (write-protected ROM region or undecoded).
    Blocked,
}

/// Effects a chipset access can produce for the bus to apply.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cs4031Effects {
    /// The shadow/ROMCS map changed; the bus must refresh the memory map.
    pub shadow_map_changed: bool,
    /// The Gate A20 state changed; the bus must refresh the A20 mask.
    pub a20_changed: bool,
    /// A CPU reset pulse was requested.
    pub cpu_reset_pulse: bool,
}

/// Effects of a port-B (0x61) write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortBEffects {
    /// New timer-2 gate level (port B bit 0).
    pub timer2_gate: bool,
    /// New speaker data level (port B bit 1).
    pub speaker_data: bool,
}

/// Disposition of a keyboard command after chipset interception.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeybCommandAction {
    /// Forward the command byte to the 8042 keyboard controller.
    pub forward: bool,
    /// Pulse the CPU reset line.
    pub cpu_reset_pulse: bool,
}

/// Disposition of a keyboard data byte after chipset interception.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeybDataAction {
    /// Forward the data byte to the 8042 keyboard controller.
    pub forward: bool,
    /// Pulse the CPU reset line.
    pub cpu_reset_pulse: bool,
}

/// CS4031 configuration and glue state.
pub struct Cs4031 {
    /// Indexed configuration registers 0x00-0x1F.
    pub registers: [u8; 0x20],
    /// Currently selected configuration index (port 0x22).
    pub index: u8,
    /// Whether the selected index is valid (below 0x20). Cleared after every
    /// configuration-data read or write, matching the CS4031 one-shot access.
    pub index_valid: bool,
    /// Port B (0x61) latch: only bits 0-3 are writable.
    pub port_b: u8,
    /// NMI enabled flag (port 0x70 bit 7 clear enables NMI).
    pub nmi_enabled: bool,
    /// Port 0x92 bit 0 fast-reset latch (edge detection source).
    pub sysctrl_reset_latch: bool,
    /// Fast Gate A20 level (port 0x92 bit 1).
    pub fast_gate_a20: bool,
    /// External Gate A20 level driven by the 8042 output port bit 1.
    pub ext_gate_a20: bool,
    /// Emulated Gate A20 level (register 0x1C bit 5 emulation).
    pub emu_gate_a20: bool,
    /// Last external keyboard-controller reset line level (active low).
    pub kbrst_level: bool,
    /// Whether a `0xD1` write-output-port command was just seen.
    pub keybc_d1_written: bool,
    /// Whether the last keyboard data byte was consumed by the chipset.
    pub keybc_data_blocked: bool,
}

impl Default for Cs4031 {
    fn default() -> Self {
        Self::new()
    }
}

impl Cs4031 {
    /// Creates a CS4031 in its power-on state.
    pub fn new() -> Self {
        let mut registers = [0u8; 0x20];
        registers[CS4031_REG_ROMCS as usize] = ROMCS_RESET_VALUE;
        Self {
            registers,
            index: 0,
            index_valid: false,
            port_b: 0x0F,
            nmi_enabled: true,
            sysctrl_reset_latch: false,
            fast_gate_a20: false,
            ext_gate_a20: false,
            emu_gate_a20: false,
            kbrst_level: true,
            keybc_d1_written: false,
            keybc_data_blocked: false,
        }
    }

    /// Selects a configuration register (port 0x22 write).
    pub fn write_config_address(&mut self, value: u8) {
        self.index = value;
        self.index_valid = value < 0x20;
    }

    /// Reads the selected configuration register (port 0x23 read).
    ///
    /// Returns 0xFF when no valid index is selected. The selection is cleared
    /// afterwards (CS4031 one-shot access).
    pub fn read_config_data(&mut self) -> u8 {
        let result = if self.index_valid {
            self.registers[self.index as usize]
        } else {
            0xFF
        };
        self.index_valid = false;
        result
    }

    /// Writes the selected configuration register (port 0x23 write).
    ///
    /// The selection is cleared afterwards (CS4031 one-shot access).
    pub fn write_config_data(&mut self, value: u8) -> Cs4031Effects {
        let mut effects = Cs4031Effects::default();
        if self.index_valid {
            let value = if self.index == CS4031_REG_SHADOW_AB {
                value & 0xF3
            } else {
                value
            };
            self.registers[self.index as usize] = value;
            match self.index {
                CS4031_REG_SHADOW_AB
                | CS4031_REG_SHADOW_READ
                | CS4031_REG_SHADOW_WRITE
                | CS4031_REG_ROMCS => {
                    effects.shadow_map_changed = true;
                }
                CS4031_REG_SOFT_RESET_GATEA20 => {
                    effects.a20_changed = true;
                }
                _ => {}
            }
        }
        self.index_valid = false;
        effects
    }

    /// Returns whether A0000 or B0000 is mapped to internal DRAM.
    pub fn ab_region_internal(&self, region: usize) -> bool {
        region < 2 && self.registers[CS4031_REG_SHADOW_AB as usize] & (1 << region) != 0
    }

    /// Returns the read source for a UMA region (0-6).
    pub fn region_read_source(&self, region: usize) -> RegionReadSource {
        let shadow_read = self.registers[CS4031_REG_SHADOW_READ as usize] & (1 << region) != 0;
        let romcs = self.registers[CS4031_REG_ROMCS as usize] & (1 << region) != 0;
        if shadow_read {
            RegionReadSource::Ram
        } else if romcs {
            RegionReadSource::Rom
        } else {
            RegionReadSource::OpenBus
        }
    }

    /// Returns the write target for a UMA region (0-6).
    pub fn region_write_target(&self, region: usize) -> RegionWriteTarget {
        let shadow_write = self.registers[CS4031_REG_SHADOW_WRITE as usize] & (1 << region) != 0;
        if shadow_write {
            RegionWriteTarget::Ram
        } else {
            RegionWriteTarget::Blocked
        }
    }

    /// Returns whether Gate A20 is currently enabled (A20 line passes through).
    pub fn a20_enabled(&self) -> bool {
        let emulation = self.registers[CS4031_REG_SOFT_RESET_GATEA20 as usize] & 0x20 != 0;
        let gate = if emulation {
            self.emu_gate_a20
        } else {
            self.ext_gate_a20
        };
        self.fast_gate_a20 || gate
    }

    /// Sets the external Gate A20 level driven by the 8042 output port bit 1.
    pub fn set_ext_gate_a20(&mut self, level: bool) -> Cs4031Effects {
        self.ext_gate_a20 = level;
        Cs4031Effects {
            a20_changed: true,
            ..Cs4031Effects::default()
        }
    }

    /// Reacts to the external keyboard-controller reset line (active low).
    ///
    /// A high-to-low transition pulses the CPU reset unless the chipset's
    /// emulated-reset path (register 0x1C bit 4) is enabled.
    pub fn kbc_reset_line(&mut self, active_low_level: bool) -> Cs4031Effects {
        let mut effects = Cs4031Effects::default();
        let emulation = self.registers[CS4031_REG_SOFT_RESET_GATEA20 as usize] & 0x10 != 0;
        if !emulation && self.kbrst_level && !active_low_level {
            effects.cpu_reset_pulse = true;
        }
        self.kbrst_level = active_low_level;
        effects
    }

    /// Reads the fast reset / Gate A20 control register (port 0x92).
    pub fn read_sysctrl(&self) -> u8 {
        (self.sysctrl_reset_latch as u8) | ((self.fast_gate_a20 as u8) << 1)
    }

    /// Writes the fast reset / Gate A20 control register (port 0x92).
    ///
    /// Bit 1 drives the fast Gate A20; a 0-to-1 transition on bit 0 pulses
    /// the CPU reset.
    pub fn write_sysctrl(&mut self, value: u8) -> Cs4031Effects {
        let mut effects = Cs4031Effects {
            a20_changed: true,
            ..Cs4031Effects::default()
        };
        self.fast_gate_a20 = value & 0x02 != 0;
        let reset_bit = value & 0x01 != 0;
        if !self.sysctrl_reset_latch && reset_bit {
            effects.cpu_reset_pulse = true;
        }
        self.sysctrl_reset_latch = reset_bit;
        effects
    }

    /// Writes the NMI mask and RTC address register (port 0x70).
    ///
    /// Bit 7 masks the NMI; bits 6:0 are returned as the RTC address to latch.
    pub fn write_rtc_nmi(&mut self, value: u8) -> u8 {
        self.nmi_enabled = value & 0x80 == 0;
        value & 0x7F
    }

    /// Reads port B (0x61) with the live refresh and timer-2 output bits.
    pub fn read_port_b(&self, refresh_toggle: bool, timer2_out: bool) -> u8 {
        let mut value = self.port_b & 0x0F;
        value |= (refresh_toggle as u8) << 4;
        value |= (timer2_out as u8) << 5;
        value
    }

    /// Writes port B (0x61); only bits 0-3 are writable.
    pub fn write_port_b(&mut self, value: u8) -> PortBEffects {
        self.port_b = (self.port_b & 0xF0) | (value & 0x0F);
        // Clearing the channel-check latch when the enable bit is set.
        if self.port_b & 0x08 != 0 {
            self.port_b &= 0xBF;
        }
        PortBEffects {
            timer2_gate: self.port_b & 0x01 != 0,
            speaker_data: self.port_b & 0x02 != 0,
        }
    }

    /// Intercepts a keyboard command write (port 0x64), per the CS4031.
    pub fn filter_keyboard_command(&mut self, command: u8) -> KeybCommandAction {
        let blocking = self.registers[CS4031_REG_SOFT_RESET_GATEA20 as usize] & 0x80 != 0;
        self.keybc_d1_written = false;
        let mut cpu_reset_pulse = false;
        match command {
            // Self-test: drive the emulated reset and Gate A20 high, always forward.
            0xAA => {
                self.emulated_kbreset(true);
                self.emulated_gatea20(true);
                KeybCommandAction {
                    forward: true,
                    cpu_reset_pulse,
                }
            }
            0xD1 => {
                self.keybc_d1_written = true;
                KeybCommandAction {
                    forward: !blocking,
                    cpu_reset_pulse,
                }
            }
            0xF0..=0xFE => {
                if command & 0x01 == 0 {
                    cpu_reset_pulse = self.emulated_kbreset(false);
                    self.emulated_kbreset(true);
                }
                if command & 0x02 == 0 {
                    self.emulated_gatea20(false);
                    self.emulated_gatea20(true);
                }
                KeybCommandAction {
                    forward: !blocking,
                    cpu_reset_pulse,
                }
            }
            0xFF => {
                if self.keybc_data_blocked {
                    self.keybc_data_blocked = false;
                    KeybCommandAction {
                        forward: !blocking,
                        cpu_reset_pulse,
                    }
                } else {
                    KeybCommandAction {
                        forward: true,
                        cpu_reset_pulse,
                    }
                }
            }
            _ => KeybCommandAction {
                forward: true,
                cpu_reset_pulse,
            },
        }
    }

    /// Intercepts a keyboard data byte write (port 0x60), per the CS4031.
    ///
    /// When command blocking is enabled and a `0xD1` command was just seen,
    /// the byte configures the emulated reset (bit 0) and Gate A20 (bit 1)
    /// instead of reaching the keyboard controller.
    pub fn filter_keyboard_data(&mut self, data: u8) -> KeybDataAction {
        let blocking = self.registers[CS4031_REG_SOFT_RESET_GATEA20 as usize] & 0x80 != 0;
        if blocking && self.keybc_d1_written {
            self.keybc_data_blocked = true;
            self.keybc_d1_written = false;
            let cpu_reset_pulse = self.emulated_kbreset(data & 0x01 != 0);
            self.emulated_gatea20(data & 0x02 != 0);
            KeybDataAction {
                forward: false,
                cpu_reset_pulse,
            }
        } else {
            self.keybc_data_blocked = false;
            KeybDataAction {
                forward: true,
                cpu_reset_pulse: false,
            }
        }
    }

    /// Applies an emulated keyboard-reset level (register 0x1C bit 4 gated).
    fn emulated_kbreset(&mut self, level: bool) -> bool {
        self.registers[CS4031_REG_SOFT_RESET_GATEA20 as usize] & 0x10 != 0 && !level
    }

    /// Applies an emulated Gate A20 level (register 0x1C bit 5 gated).
    fn emulated_gatea20(&mut self, level: bool) {
        if self.registers[CS4031_REG_SOFT_RESET_GATEA20 as usize] & 0x20 != 0 {
            self.emu_gate_a20 = level;
        }
    }
}
