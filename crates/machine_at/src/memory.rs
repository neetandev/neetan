//! PC/AT physical memory map with shadow-RAM decode and A20 masking.
//!
//! The map is conventional RAM below 640 KiB, the VGA display window (routed
//! to the VGA device by the bus), the shadow-controlled upper memory area
//! C0000-FFFFF with the VGA BIOS ROM behind C0000-C7FFF, extended RAM above
//! 1 MiB, and the system BIOS aliased at the top of the 4 GiB space so the
//! 486 reset fetch at 0xFFFFFFF0 lands in it. The shadow decode is
//! precomputed into per-region lookup tables that the bus refreshes whenever
//! the CS4031 shadow/ROMCS registers change.

use device::cs4031::{Cs4031, RegionReadSource, RegionWriteTarget};

/// Size of the system BIOS ROM in bytes.
const BIOS_SIZE: u32 = 0x1_0000;
/// Start of the conventional/UMA boundary (VGA window base).
pub const VGA_WINDOW_BASE: u32 = 0x000A_0000;
/// Start of the upper memory area (shadow-controlled region).
pub const UMA_BASE: u32 = 0x000C_0000;
/// End (exclusive) of the upper memory area / start of extended memory.
const EXTENDED_BASE: u32 = 0x0010_0000;
/// Base of the VGA BIOS ROM in real-mode space (C0000).
const VGA_BIOS_REAL_BASE: u32 = 0x000C_0000;
/// End (exclusive) of the VGA BIOS ROM window.
const VGA_BIOS_END: u32 = 0x000C_8000;
/// Base of the system BIOS region in real-mode space (F0000).
const BIOS_REAL_BASE: u32 = 0x000F_0000;
/// Base of the system BIOS alias at the top of the 4 GiB space.
const BIOS_ALIAS_BASE: u32 = 0xFFFF_0000;
/// Number of shadow-controlled UMA regions.
const UMA_REGION_COUNT: usize = 7;
/// Value returned by an open-bus read.
const OPEN_BUS: u8 = 0xFF;

save_state::runtime_state! {
/// Authoritative PC/AT RAM and memory-controller state.
#[derive(Clone)]
pub(crate) struct AtMemoryState {
    ram: Vec<u8>,
}}

/// Read resolution for a UMA region, precomputed from the chipset registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UmaRead {
    /// Read from shadow DRAM.
    Ram,
    /// Read from the system BIOS ROM (only region 6 has ROM content).
    Rom,
    /// Nothing decoded: open bus.
    OpenBus,
}

/// Write resolution for a UMA region, precomputed from the chipset registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UmaWrite {
    /// Write into shadow DRAM.
    Ram,
    /// Write discarded.
    Blocked,
}

/// PC/AT physical memory.
pub struct AtMemory {
    /// Full linear RAM (`ram_size` bytes); the 0xC0000-0xFFFFF slice is the
    /// shadow RAM.
    ram: Vec<u8>,
    /// System BIOS ROM (64 KiB).
    bios: Vec<u8>,
    /// VGA BIOS ROM (32 KiB) on the display adapter, decoded at C0000.
    vga_bios: Vec<u8>,
    /// Precomputed read source per UMA region.
    uma_read: [UmaRead; UMA_REGION_COUNT],
    /// Precomputed write target per UMA region.
    uma_write: [UmaWrite; UMA_REGION_COUNT],
    /// Internal DRAM decode for the A0000 and B0000 64 KiB regions.
    ab_internal: [bool; 2],
    /// A20 address mask (`!0` when enabled, bit 20 cleared when disabled).
    a20_mask: u32,
    /// Total RAM size in bytes.
    ram_size: u32,
}

impl AtMemory {
    /// Creates the memory with `ram_size` bytes of RAM and the ROM images.
    ///
    /// A20 starts enabled: the 486 reset fetch at 0xFFFFFFF0 has bit 20 set.
    pub fn new(ram_size: u32, bios: Vec<u8>, vga_bios: Vec<u8>) -> Self {
        Self {
            ram: vec![0; ram_size as usize],
            bios,
            vga_bios,
            uma_read: [UmaRead::OpenBus; UMA_REGION_COUNT],
            uma_write: [UmaWrite::Blocked; UMA_REGION_COUNT],
            ab_internal: [false; 2],
            a20_mask: !0,
            ram_size,
        }
    }

    /// Captures writable physical memory without ROM or derived decode tables.
    pub(crate) fn capture_state(&self) -> AtMemoryState {
        AtMemoryState {
            ram: self.ram.clone(),
        }
    }

    /// Restores writable physical memory after validating its configured size.
    pub(crate) fn restore_state(
        &mut self,
        state: AtMemoryState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.ram.len() != self.ram.len() {
            return Err(save_state::StateValidationError::new(
                "PC/AT RAM size differs",
            ));
        }
        self.ram = state.ram;
        Ok(())
    }

    /// Returns stable identities for the system and display ROMs.
    pub(crate) fn resource_bindings(
        &self,
    ) -> Result<Vec<save_state::ResourceBinding>, save_state::StateValidationError> {
        Ok(vec![
            save_state::ResourceBinding {
                identifier: save_state::ResourceBindingId::new("rom:system-bios")?,
                identity: save_state::ResourceIdentity::from_bytes(&self.bios),
            },
            save_state::ResourceBinding {
                identifier: save_state::ResourceBindingId::new("rom:vga-bios")?,
                identity: save_state::ResourceIdentity::from_bytes(&self.vga_bios),
            },
        ])
    }

    /// Recomputes the UMA read/write tables from the chipset registers.
    pub fn refresh_uma(&mut self, chipset: &Cs4031) {
        for region in 0..2 {
            self.ab_internal[region] = chipset.ab_region_internal(region);
        }
        for region in 0..UMA_REGION_COUNT {
            self.uma_read[region] = match chipset.region_read_source(region) {
                RegionReadSource::Ram => UmaRead::Ram,
                RegionReadSource::Rom => UmaRead::Rom,
                RegionReadSource::OpenBus => UmaRead::OpenBus,
            };
            self.uma_write[region] = match chipset.region_write_target(region) {
                RegionWriteTarget::Ram => UmaWrite::Ram,
                RegionWriteTarget::Blocked => UmaWrite::Blocked,
            };
        }
    }

    /// Returns whether internal DRAM owns an address in A0000-BFFFF.
    pub fn ab_internal(&self, address: u32) -> bool {
        self.ab_internal[((address - VGA_WINDOW_BASE) >> 16) as usize]
    }

    /// Sets the A20 gate state.
    pub fn set_a20(&mut self, enabled: bool) {
        self.a20_mask = if enabled { !0 } else { !(1 << 20) };
    }

    /// Applies the A20 mask to a CPU address.
    pub fn apply_a20(&self, address: u32) -> u32 {
        address & self.a20_mask
    }

    /// Reads a byte from the CPU's view of memory (test helper; the bus
    /// applies A20 itself to route the VGA window first).
    #[cfg(test)]
    fn read_byte(&self, address: u32) -> u8 {
        self.read_physical(self.apply_a20(address))
    }

    /// Writes a byte to the CPU's view of memory (test helper).
    #[cfg(test)]
    fn write_byte(&mut self, address: u32, value: u8) {
        self.write_physical(self.apply_a20(address), value);
    }

    /// Reads a byte from physical memory (no A20 mask; used by DMA).
    ///
    /// The VGA display window reads as open bus unless the chipset maps its
    /// A0000 or B0000 region to internal DRAM. The bus routes external CPU
    /// accesses to the VGA device before consulting this map.
    pub fn read_physical(&self, address: u32) -> u8 {
        if address < VGA_WINDOW_BASE {
            return self.ram[address as usize];
        }
        if address < UMA_BASE {
            return if self.ab_internal(address) {
                self.ram[address as usize]
            } else {
                OPEN_BUS
            };
        }
        if address < EXTENDED_BASE {
            let region = uma_region(address);
            return match self.uma_read[region] {
                UmaRead::Ram => self.ram[address as usize],
                UmaRead::Rom | UmaRead::OpenBus if address < VGA_BIOS_END => {
                    // The ISA display adapter claims every C0000-C7FFF cycle
                    // the chipset does not satisfy from shadow DRAM; the
                    // chipset ROMCS has no content behind this range.
                    self.vga_bios[(address - VGA_BIOS_REAL_BASE) as usize]
                }
                UmaRead::Rom if address >= BIOS_REAL_BASE => {
                    self.bios[(address - BIOS_REAL_BASE) as usize]
                }
                UmaRead::Rom | UmaRead::OpenBus => OPEN_BUS,
            };
        }
        if address >= BIOS_ALIAS_BASE {
            return self.bios[(address & (BIOS_SIZE - 1)) as usize];
        }
        if address < self.ram_size {
            return self.ram[address as usize];
        }
        OPEN_BUS
    }

    /// Writes a byte to physical memory (no A20 mask; used by DMA).
    pub fn write_physical(&mut self, address: u32, value: u8) {
        if address < VGA_WINDOW_BASE {
            self.ram[address as usize] = value;
            return;
        }
        if address < UMA_BASE {
            if self.ab_internal(address) {
                self.ram[address as usize] = value;
            }
            return;
        }
        if address < EXTENDED_BASE {
            let region = uma_region(address);
            if self.uma_write[region] == UmaWrite::Ram {
                self.ram[address as usize] = value;
            }
            return;
        }
        if address >= BIOS_ALIAS_BASE {
            return; // BIOS alias is read-only
        }
        if address < self.ram_size {
            self.ram[address as usize] = value;
        }
    }

    /// Returns whether a `len`-byte access at `physical` lies wholly inside
    /// contiguous read/write RAM (conventional below the VGA window, or
    /// extended above 1 MiB). The VGA window, the UMA/shadow area, the BIOS
    /// alias, and anything past `ram_size` are excluded so those keep their
    /// per-byte device/decode routing.
    fn linear_ram_range(&self, physical: u32, len: u32) -> bool {
        let end = physical.wrapping_add(len - 1);
        end < VGA_WINDOW_BASE || (physical >= EXTENDED_BASE && end < self.ram_size)
    }

    /// Reads a contiguous little-endian word from linear RAM, or `None` when
    /// the range is not pure RAM. `physical` must already be A20-masked.
    pub fn read_ram_word(&self, physical: u32) -> Option<u16> {
        if !self.linear_ram_range(physical, 2) {
            return None;
        }
        let base = physical as usize;
        Some(self.ram[base] as u16 | ((self.ram[base + 1] as u16) << 8))
    }

    /// Reads a contiguous little-endian dword from linear RAM, or `None` when
    /// the range is not pure RAM. `physical` must already be A20-masked.
    pub fn read_ram_dword(&self, physical: u32) -> Option<u32> {
        if !self.linear_ram_range(physical, 4) {
            return None;
        }
        let base = physical as usize;
        Some(
            self.ram[base] as u32
                | ((self.ram[base + 1] as u32) << 8)
                | ((self.ram[base + 2] as u32) << 16)
                | ((self.ram[base + 3] as u32) << 24),
        )
    }

    /// Writes a contiguous little-endian word to linear RAM. Returns `false`
    /// when the range is not pure RAM. `physical` must already be A20-masked.
    pub fn write_ram_word(&mut self, physical: u32, value: u16) -> bool {
        if !self.linear_ram_range(physical, 2) {
            return false;
        }
        let base = physical as usize;
        self.ram[base] = value as u8;
        self.ram[base + 1] = (value >> 8) as u8;
        true
    }

    /// Writes a contiguous little-endian dword to linear RAM. Returns `false`
    /// when the range is not pure RAM. `physical` must already be A20-masked.
    pub fn write_ram_dword(&mut self, physical: u32, value: u32) -> bool {
        if !self.linear_ram_range(physical, 4) {
            return false;
        }
        let base = physical as usize;
        self.ram[base] = value as u8;
        self.ram[base + 1] = (value >> 8) as u8;
        self.ram[base + 2] = (value >> 16) as u8;
        self.ram[base + 3] = (value >> 24) as u8;
        true
    }
}

/// Returns the shadow region index (0-6) for an address in C0000-FFFFF.
fn uma_region(address: u32) -> usize {
    if address < 0x000D_0000 {
        ((address - UMA_BASE) >> 14) as usize
    } else if address < 0x000E_0000 {
        4
    } else if address < BIOS_REAL_BASE {
        5
    } else {
        6
    }
}

#[cfg(test)]
mod tests {
    use device::cs4031::{
        CS4031_REG_SHADOW_AB, CS4031_REG_SHADOW_READ, CS4031_REG_SHADOW_WRITE, Cs4031,
    };

    use super::*;

    fn memory() -> AtMemory {
        let mut bios = vec![0u8; BIOS_SIZE as usize];
        bios[0] = 0xAA; // marker at F0000
        bios[0xFFF0] = 0x55; // marker at the reset-vector offset
        let mut vga_bios = vec![0u8; 0x8000];
        vga_bios[0] = 0x55; // option ROM signature bytes
        vga_bios[1] = 0xAA;
        vga_bios[0x7FFF] = 0x99; // marker at the last ROM byte
        let mut memory = AtMemory::new(8 << 20, bios, vga_bios);
        memory.refresh_uma(&Cs4031::new());
        memory
    }

    #[test]
    fn bios_alias_serves_reset_vector() {
        let memory = memory();
        assert_eq!(memory.read_byte(0xFFFF_FFF0), 0x55);
        assert_eq!(memory.read_byte(0xFFFF_0000), 0xAA);
    }

    #[test]
    fn bios_visible_at_f_segment_by_default() {
        // ROMCS reset default enables region 6 (F0000) as ROM.
        let memory = memory();
        assert_eq!(memory.read_byte(0x000F_0000), 0xAA);
    }

    #[test]
    fn vga_window_reads_open_bus() {
        let memory = memory();
        assert_eq!(memory.read_byte(0x000A_0000), OPEN_BUS);
    }

    #[test]
    fn chipset_can_map_b0000_region_to_internal_dram() {
        let mut memory = memory();
        let mut chipset = Cs4031::new();
        chipset.write_config_address(CS4031_REG_SHADOW_AB);
        chipset.write_config_data(0x02);
        memory.refresh_uma(&chipset);

        memory.write_byte(0x000B_8000, 0x5A);
        assert_eq!(memory.read_byte(0x000B_8000), 0x5A);
        assert!(!memory.ab_internal(0x000A_0000));
        assert!(memory.ab_internal(0x000B_8000));
    }

    #[test]
    fn conventional_and_extended_ram_round_trip() {
        let mut memory = memory();
        memory.write_byte(0x0000_1234, 0x42);
        assert_eq!(memory.read_byte(0x0000_1234), 0x42);
        memory.write_byte(0x0020_0000, 0x99);
        assert_eq!(memory.read_byte(0x0020_0000), 0x99);
    }

    #[test]
    fn a20_masking_wraps_the_first_megabyte() {
        let mut memory = memory();
        memory.write_byte(0x0000_0000, 0x11);
        memory.set_a20(false);
        // With A20 masked, 0x100000 aliases to 0x000000.
        assert_eq!(memory.read_byte(0x0010_0000), 0x11);
        memory.set_a20(true);
        assert_eq!(memory.read_byte(0x0010_0000), 0x00);
    }

    #[test]
    fn vga_bios_visible_at_c_segment() {
        let memory = memory();
        assert_eq!(memory.read_byte(0x000C_0000), 0x55);
        assert_eq!(memory.read_byte(0x000C_0001), 0xAA);
        assert_eq!(memory.read_byte(0x000C_7FFF), 0x99);
        // Past the ROM the UMA stays open bus.
        assert_eq!(memory.read_byte(0x000C_8000), OPEN_BUS);
    }

    #[test]
    fn vga_bios_shadow_copy_round_trip() {
        let mut memory = memory();
        let mut chipset = Cs4031::new();
        // Enable shadow writes for region 0 (C0000-C3FFF), keep reads on ROM.
        chipset.write_config_address(CS4031_REG_SHADOW_WRITE);
        chipset.write_config_data(1 << 0);
        memory.refresh_uma(&chipset);
        let byte = memory.read_byte(0x000C_0000);
        memory.write_byte(0x000C_0000, byte);
        memory.write_byte(0x000C_0001, 0x77);
        // Reads still come from the ROM until shadow read is enabled.
        assert_eq!(memory.read_byte(0x000C_0001), 0xAA);
        chipset.write_config_address(CS4031_REG_SHADOW_READ);
        chipset.write_config_data(1 << 0);
        memory.refresh_uma(&chipset);
        assert_eq!(memory.read_byte(0x000C_0000), 0x55);
        assert_eq!(memory.read_byte(0x000C_0001), 0x77);
    }

    #[test]
    fn shadow_write_then_read_ram() {
        let mut memory = memory();
        let mut chipset = Cs4031::new();
        // Enable shadow write and shadow read for region 6 (F0000).
        chipset.write_config_address(CS4031_REG_SHADOW_WRITE);
        chipset.write_config_data(1 << 6);
        chipset.write_config_address(CS4031_REG_SHADOW_READ);
        chipset.write_config_data(1 << 6);
        memory.refresh_uma(&chipset);

        memory.write_byte(0x000F_1000, 0x7E);
        assert_eq!(memory.read_byte(0x000F_1000), 0x7E);
    }

    #[test]
    fn ram_fast_path_only_covers_pure_ram_regions() {
        let memory = memory(); // 8 MiB RAM
        // Conventional RAM and extended RAM take the fast path.
        assert!(memory.read_ram_word(0x0000_1234).is_some());
        assert!(memory.read_ram_dword(0x0000_1234).is_some());
        assert!(memory.read_ram_word(0x0009_FFFE).is_some()); // ends at 0x9FFFF
        assert!(memory.read_ram_dword(0x0020_0000).is_some());
        assert!(memory.read_ram_dword(0x007F_FFFC).is_some()); // ends at ram_size - 1
        // The VGA window, the UMA/shadow area, and past ram_size fall back.
        assert!(memory.read_ram_word(0x0009_FFFF).is_none()); // dword would enter VGA window
        assert!(memory.read_ram_dword(0x0009_FFFE).is_none()); // dword would enter VGA window
        assert!(memory.read_ram_word(0x000A_0000).is_none()); // VGA window
        assert!(memory.read_ram_dword(0x000C_0000).is_none()); // UMA/shadow
        assert!(memory.read_ram_dword(0x000F_FFFE).is_none()); // straddles 0xFFFFF/0x100000
        assert!(memory.read_ram_dword(0x007F_FFFE).is_none()); // straddles ram_size
        assert!(memory.read_ram_word(0x0080_0000).is_none()); // at ram_size
    }

    #[test]
    fn ram_fast_path_matches_byte_composition() {
        let mut memory = memory();
        // Write via the multi-byte helpers, read back both ways.
        assert!(memory.write_ram_dword(0x0000_2000, 0x1122_3344));
        assert_eq!(memory.read_ram_dword(0x0000_2000), Some(0x1122_3344));
        assert_eq!(memory.read_physical(0x0000_2000), 0x44);
        assert_eq!(memory.read_physical(0x0000_2001), 0x33);
        assert_eq!(memory.read_physical(0x0000_2002), 0x22);
        assert_eq!(memory.read_physical(0x0000_2003), 0x11);
        assert_eq!(memory.read_ram_word(0x0000_2002), Some(0x1122));

        assert!(memory.write_ram_word(0x0030_0000, 0xBEEF));
        assert_eq!(memory.read_ram_word(0x0030_0000), Some(0xBEEF));
        assert_eq!(memory.read_physical(0x0030_0000), 0xEF);
        assert_eq!(memory.read_physical(0x0030_0001), 0xBE);

        // A non-RAM range leaves memory untouched and reports no hit.
        assert!(!memory.write_ram_word(0x000A_0000, 0x5555));
    }
}
