//! Dual i8259A Programmable Interrupt Controller for the PC-98.
//!
//! PC-98 uses edge-triggered mode (ICW1 bit 3 = 0) with non-specific EOI.
//! See `undoc98/io_pic.txt` for IRQ assignments, ICW defaults,
//! and the canonical slave EOI pattern.

use std::{
    cell::Cell,
    ops::{Deref, DerefMut},
};

/// Master ICW1: edge-triggered, cascaded, ICW4 needed (0x11 = 0001_0001b).
/// Bits: D4=1 (ICW1 flag), D0=1 (IC4: ICW4 will follow).
/// Ref: undoc98 `io_pic.txt` ICW1 table.
const MASTER_ICW1: u8 = 0x11;

/// Master ICW2: interrupt vector base 0x08 (IR0 -> INT 08h).
/// Bits 7-3 = 00001b -> vector base 0x08.
/// Ref: undoc98 `io_pic.txt` ICW2 table.
const MASTER_ICW2: u8 = 0x08;

/// Master ICW3: slave connected on IR7 (bit 7 set = 0x80).
/// Ref: undoc98 `io_pic.txt` ICW3 table.
const MASTER_ICW3: u8 = 0x80;

/// Master ICW4: buffered master mode, normal EOI, 8086 mode (0x1D = 0001_1101b).
/// Bits: D4=1 (SFNM), D3=1 (BUF), D2=1 (M/S master), D0=1 (uPM 8086).
/// Ref: undoc98 `io_pic.txt` ICW4 table.
const MASTER_ICW4: u8 = 0x1D;

/// Master IMR default: mask all except IR1 (keyboard) and IR7 (slave cascade).
/// 0x7D = 0111_1101b: IR0 (timer), IR2-IR6 masked; IR1, IR7 enabled.
const MASTER_IMR_DEFAULT: u8 = 0x7D;

/// Slave ICW1: edge-triggered, cascaded, ICW4 needed (0x11).
const SLAVE_ICW1: u8 = 0x11;

/// Slave ICW2: interrupt vector base 0x10 (IR8 -> INT 10h).
/// undoc98 `io_pic.txt` ICW2 table.
const SLAVE_ICW2: u8 = 0x10;

/// Slave ICW3: slave ID = 7 (connected to master IR7).
/// Bits 2-0 = 111b = 0x07.
/// undoc98 `io_pic.txt` ICW3 table.
const SLAVE_ICW3: u8 = 0x07;

/// Slave ICW4: buffered slave mode, normal EOI, 8086 mode (0x09 = 0000_1001b).
/// Bits: D3=1 (BUF), D0=1 (uPM 8086). D2=0 (M/S slave).
/// undoc98 `io_pic.txt` ICW4 table.
const SLAVE_ICW4: u8 = 0x09;

/// Slave IMR default: mask all except IR9 (INT3), IR10 (INT41), IR11 (INT42).
/// 0x71 = 0111_0001b: IR8 (printer) masked; IR9-IR11 enabled; IR12-IR14 masked.
const SLAVE_IMR_DEFAULT: u8 = 0x71;

/// OCW2 level select mask (bits 2-0).
const OCW2_L: u8 = 0x07;
/// OCW2 end-of-interrupt flag (bit 5).
const OCW2_EOI: u8 = 0x20;
/// OCW2 specific level flag (bit 6).
const OCW2_SL: u8 = 0x40;
/// OCW2 rotation flag (bit 7).
const OCW2_R: u8 = 0x80;

/// OCW3 read ISR (bit 0): 0 = read IRR, 1 = read ISR.
const OCW3_RIS: u8 = 0x01;
/// OCW3 read register command (bit 1): must be 1 to update RIS.
const OCW3_RR: u8 = 0x02;
/// OCW3 special mask mode (bit 5).
const OCW3_SMM: u8 = 0x20;
/// OCW3 enable special mask mode (bit 6): must be 1 to update SMM.
const OCW3_ESMM: u8 = 0x40;

/// Snapshot of a single i8259A PIC chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I8259aPicChipState {
    /// Initialization command words (ICW1-ICW4).
    pub icw: [u8; 4],
    /// Interrupt mask register.
    pub imr: u8,
    /// In-service register.
    pub isr: u8,
    /// Interrupt request register.
    pub irr: u8,
    /// Operation command word 3.
    pub ocw3: u8,
    /// Priority rotation base.
    pub pry: u8,
    /// ICW write sequence index.
    pub write_icw: u8,
}

/// Snapshot of the dual i8259A PIC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I8259aPicState {
    /// Master (index 0) and slave (index 1) chip snapshots.
    pub chips: [I8259aPicChipState; 2],
}

enum PendingIrq {
    Master(u8),
    Slave(u8),
}

/// Dual i8259A PIC (master + slave) for the PC-98.
pub struct I8259aPic {
    /// Embedded state for save/restore.
    pub state: I8259aPicState,
    irq_cache: Cell<Option<bool>>,
}

impl Deref for I8259aPic {
    type Target = I8259aPicState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for I8259aPic {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Default for I8259aPic {
    fn default() -> Self {
        Self::new()
    }
}

impl I8259aPic {
    /// Creates a new PIC pair initialized to PC-98 boot defaults.
    pub fn new() -> Self {
        Self {
            state: I8259aPicState {
                chips: [
                    I8259aPicChipState {
                        icw: [MASTER_ICW1, MASTER_ICW2, MASTER_ICW3, MASTER_ICW4],
                        imr: MASTER_IMR_DEFAULT,
                        isr: 0,
                        irr: 0,
                        ocw3: 0,
                        pry: 0,
                        write_icw: 0,
                    },
                    I8259aPicChipState {
                        icw: [SLAVE_ICW1, SLAVE_ICW2, SLAVE_ICW3, SLAVE_ICW4],
                        imr: SLAVE_IMR_DEFAULT,
                        isr: 0,
                        irr: 0,
                        ocw3: 0,
                        pry: 0,
                        write_icw: 0,
                    },
                ],
            },
            irq_cache: Cell::new(None),
        }
    }

    /// Creates a new PIC pair with all registers zeroed.
    pub fn new_zeroed() -> Self {
        Self {
            state: I8259aPicState {
                chips: [
                    I8259aPicChipState {
                        icw: [0; 4],
                        imr: 0,
                        isr: 0,
                        irr: 0,
                        ocw3: 0,
                        pry: 0,
                        write_icw: 0,
                    },
                    I8259aPicChipState {
                        icw: [0; 4],
                        imr: 0,
                        isr: 0,
                        irr: 0,
                        ocw3: 0,
                        pry: 0,
                        write_icw: 0,
                    },
                ],
            },
            irq_cache: Cell::new(None),
        }
    }

    /// Invalidates the cached IRQ pending result.
    /// Must be called after any direct mutation of PIC chip state.
    pub fn invalidate_irq_cache(&self) {
        self.irq_cache.set(None);
    }

    /// Writes to port 0 (master: 0x00, slave: 0x08).
    /// Handles ICW1, OCW2, and OCW3 commands.
    pub fn write_port0(&mut self, chip_index: usize, data: u8) {
        let chip = &mut self.chips[chip_index];
        chip.write_icw = 0;

        match data & 0x18 {
            0x00 => {
                // OCW2: EOI handling
                let level = if data & OCW2_SL != 0 {
                    data & OCW2_L
                } else {
                    if chip.isr == 0 {
                        return;
                    }
                    let mut l = chip.pry;
                    while chip.isr & (1 << l) == 0 {
                        l = (l + 1) & 7;
                    }
                    l
                };
                if data & OCW2_R != 0 {
                    chip.pry = (level + 1) & 7;
                }
                if data & OCW2_EOI != 0 {
                    chip.isr &= !(1 << level);
                }
            }
            0x08 => {
                // OCW3: read mode / special mask.
                // RIS is only updated when RR=1; SMM is only updated when ESMM=1.
                // NP21W uses `dat &= PIC_OCW3_RIS` when RR=0, which clears
                // unrelated bits. We follow the Intel spec and preserve them.
                let old_ocw3 = chip.ocw3;
                let mut new = data;
                if data & OCW3_RR == 0 {
                    new = (new & !OCW3_RIS) | (old_ocw3 & OCW3_RIS);
                }
                if data & OCW3_ESMM == 0 {
                    new = (new & !OCW3_SMM) | (old_ocw3 & OCW3_SMM);
                }
                chip.ocw3 = new;
            }
            _ => {
                // ICW1: reinitialize
                chip.icw[0] = data;
                chip.imr = 0;
                chip.isr = 0;
                chip.irr = 0;
                chip.ocw3 = 0;
                chip.pry = 0;
                chip.write_icw = 1;
            }
        }
        self.invalidate_irq_cache();
    }

    /// Writes to port 2 (master: 0x02, slave: 0x0A).
    /// Handles ICW2-4 sequence or OCW1 (IMR).
    pub fn write_port2(&mut self, chip_index: usize, data: u8) {
        let chip = &mut self.chips[chip_index];
        if chip.write_icw == 0 {
            chip.imr = data;
        } else {
            chip.icw[chip.write_icw as usize] = data;
            chip.write_icw += 1;
            if chip.write_icw >= 3 + (chip.icw[0] & 1) {
                chip.write_icw = 0;
            }
        }
        self.invalidate_irq_cache();
    }

    /// Reads port 0 (master: 0x00, slave: 0x08).
    /// Returns IRR or ISR based on OCW3 RIS bit.
    pub fn read_port0(&self, chip_index: usize) -> u8 {
        let chip = &self.chips[chip_index];
        if chip.ocw3 & OCW3_RIS == 0 {
            chip.irr
        } else {
            chip.isr
        }
    }

    /// Reads port 2 (master: 0x02, slave: 0x0A).
    /// Returns IMR.
    pub fn read_port2(&self, chip_index: usize) -> u8 {
        self.chips[chip_index].imr
    }

    /// Sets an IRQ line (edge-triggered). IRQ 0-7 go to master, 8-15 to slave.
    pub fn set_irq(&mut self, irq: u8) {
        let bit = 1u8 << (irq & 7);
        if irq & 8 == 0 {
            self.chips[0].irr |= bit;
        } else {
            self.chips[1].irr |= bit;
        }
        self.invalidate_irq_cache();
    }

    /// Clears an IRQ line.
    pub fn clear_irq(&mut self, irq: u8) {
        let bit = 1u8 << (irq & 7);
        if irq & 8 == 0 {
            self.chips[0].irr &= !bit;
        } else {
            self.chips[1].irr &= !bit;
        }
        self.invalidate_irq_cache();
    }

    /// Returns true if a pending unmasked IRQ exists that is not blocked
    /// by a higher-priority in-service interrupt.
    pub fn has_pending_irq(&self) -> bool {
        if let Some(cached) = self.irq_cache.get() {
            return cached;
        }
        let result = self.find_pending_irq().is_some();
        self.irq_cache.set(Some(result));
        result
    }

    /// Acknowledges the highest-priority pending IRQ: sets ISR, clears IRR,
    /// and returns the interrupt vector number.
    pub fn acknowledge(&mut self) -> u8 {
        self.invalidate_irq_cache();
        let Some(pending) = self.find_pending_irq() else {
            // Spurious interrupt - IRQ was deasserted between INTR and INTA.
            // Real 8259A returns the lowest-priority master vector (base + 7).
            return (self.chips[0].icw[1] & 0xF8) | 7;
        };

        match pending {
            PendingIrq::Master(num) => {
                let bit = 1u8 << num;
                self.chips[0].isr |= bit;
                self.chips[0].irr &= !bit;
                (self.chips[0].icw[1] & 0xF8) | num
            }
            PendingIrq::Slave(num) => {
                let slave_bit = 1u8 << (self.chips[1].icw[2] & 7);
                self.chips[0].isr |= slave_bit;
                self.chips[0].irr &= !slave_bit;
                let bit = 1u8 << num;
                self.chips[1].isr |= bit;
                self.chips[1].irr &= !bit;
                (self.chips[1].icw[1] & 0xF8) | num
            }
        }
    }

    /// Priority resolution.
    fn find_pending_irq(&self) -> Option<PendingIrq> {
        let master = &self.chips[0];
        let slave = &self.chips[1];

        let sir = slave.irr & !slave.imr;
        let slave_bit = 1u8 << (slave.icw[2] & 7);

        let mut mir = master.irr;
        if sir != 0 {
            mir |= slave_bit;
        }
        mir &= !master.imr;

        if mir == 0 {
            return None;
        }

        let master_blocking_isr = if master.ocw3 & OCW3_SMM == 0 {
            master.isr
        } else {
            master.isr & !master.imr
        };
        mir |= master_blocking_isr;

        let mut num = master.pry;
        let mut bit = 1u8 << num;
        while mir & bit == 0 {
            num = (num + 1) & 7;
            bit = 1 << num;
        }

        if master.icw[2] & bit != 0 {
            // Cascade: this bit is the slave connection.
            if sir == 0 {
                return None;
            }

            let slave_blocking_isr = if slave.ocw3 & OCW3_SMM == 0 {
                slave.isr
            } else {
                slave.isr & !slave.imr
            };
            let sir_scan = sir | slave_blocking_isr;

            let mut snum = slave.pry;
            let mut sbit = 1u8 << snum;
            while sir_scan & sbit == 0 {
                snum = (snum + 1) & 7;
                sbit = 1 << snum;
            }

            if slave.isr & sbit != 0 {
                return None;
            }

            Some(PendingIrq::Slave(snum))
        } else if master.isr & bit != 0 {
            None
        } else {
            Some(PendingIrq::Master(num))
        }
    }
}
