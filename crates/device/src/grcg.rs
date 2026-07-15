//! GRCG (Graphic Charger) - hardware tile/ROP engine for VRAM.
//!
//! Port 0x7C W: mode register (resets tile index).
//! Port 0x7E W: tile data (cycles through 4 planes).

/// Mode register bit 7 (port 0x7C): CG mode enable.
/// 1 = GRCG active, 0 = GRCG disabled (normal VRAM access).
/// Ref: undoc98 `io_disp.txt` port 007Ch
const MODE_CG_ENABLE: u8 = 0x80;

/// Mode register bit 6 (port 0x7C): RMW mode select.
/// 1 = read-modify-write mode, 0 = TDW (tile data write) or TCR (tile compare read).
/// When both bits 7 and 6 are set, GRCG operates in RMW mode.
/// When bit 7 is set and bit 6 is clear, write = TDW mode, read = TCR mode.
/// Ref: undoc98 `io_disp.txt` port 007Ch
const MODE_RMW: u8 = 0x40;

/// Number of tile registers (one per VRAM plane).
const TILE_REGISTER_COUNT: usize = 4;

/// GRCG chip version. Determines which features are available.
/// 0=none, 1=GRCG v1 (VM), 2=GRCG v2 (VX), 3=EGC (VX+).
pub const GRCG_CHIP_NONE: u8 = 0;
/// GRCG version 1 (PC-9801VM).
pub const GRCG_CHIP_V1: u8 = 1;
/// GRCG version 2 with EGC support (PC-9801VX and later).
pub const GRCG_CHIP_EGC: u8 = 3;

save_state::runtime_state! {
/// Authoritative GRCG state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrcgState {
    /// Mode register (write-only via port 0x7C).
    /// Bit 7: CGmode (1=enabled), bit 6: RMWmode, bits 3-0: plane enable (0=enabled).
    pub mode: u8,
    /// Tile register index (0-3), cycles on each write to port 0x7E.
    pub tile_index: u8,
    /// Tile registers (4 planes, written cyclically via port 0x7E).
    pub tile: [u8; TILE_REGISTER_COUNT],
    /// Chip version (0=none, 1=v1/VM, 3=EGC/VX+).
    pub chip: u8,
    /// GDC-through-GRCG routing flags. Only set when chip >= 2.
    /// Bit 2: RMW mode for GDC drawing. Bit 3: GDC drawing routes through GRCG/EGC.
    pub gdc_with_grcg: u8,
}}

/// GRCG controller.
pub struct Grcg {
    /// Embedded state for save/restore.
    pub state: GrcgState,
}

impl_simple_state_accessors!(Grcg, GrcgState);

impl Default for Grcg {
    fn default() -> Self {
        Self::new(GRCG_CHIP_NONE)
    }
}

impl Grcg {
    /// Creates a new GRCG with the given chip version and all registers zeroed.
    pub fn new(chip: u8) -> Self {
        Self {
            state: GrcgState {
                mode: 0x00,
                tile_index: 0,
                tile: [0; TILE_REGISTER_COUNT],
                chip,
                gdc_with_grcg: 0,
            },
        }
    }

    /// Writes the mode register (port 0x7C). Resets tile index to 0.
    /// On chip >= 2, also extracts the GDC-through-GRCG routing flags from bits 6-7.
    pub fn write_mode(&mut self, value: u8) {
        self.state.mode = value;
        self.state.tile_index = 0;
        if self.state.chip >= 2 {
            self.state.gdc_with_grcg = (value >> 4) & 0x0C;
        }
    }

    /// Writes a tile register (port 0x7E). Cycles through planes 0-3.
    pub fn write_tile(&mut self, value: u8) {
        self.state.tile[self.state.tile_index as usize] = value;
        self.state.tile_index = (self.state.tile_index + 1) & 3;
    }

    /// Returns true if GRCG is active (CGmode bit set).
    pub fn is_active(&self) -> bool {
        self.state.mode & MODE_CG_ENABLE != 0
    }

    /// Returns true if in RMW mode (both CGmode and RMWmode bits set).
    pub fn is_rmw(&self) -> bool {
        self.state.mode & (MODE_CG_ENABLE | MODE_RMW) == (MODE_CG_ENABLE | MODE_RMW)
    }

    /// Returns true if plane `p` (0-3) is enabled for GRCG operation.
    /// Planes are enabled when the corresponding bit is 0 (active-low).
    pub fn plane_enabled(&self, p: usize) -> bool {
        self.state.mode & (1 << p) == 0
    }

    /// Returns true if GDC drawing should route through GRCG/EGC (chip >= 2 only).
    pub fn gdc_with_grcg_enabled(&self) -> bool {
        self.state.gdc_with_grcg & 0x08 != 0
    }

    /// Returns true if GDC-through-GRCG uses RMW mode (vs TDW).
    pub fn gdc_with_grcg_is_rmw(&self) -> bool {
        self.state.gdc_with_grcg & 0x04 != 0
    }

    /// Resets mode register, tile counter, and GDC routing flags.
    pub fn bios_reset(&mut self) {
        self.state.mode = 0;
        self.state.tile_index = 0;
        self.state.gdc_with_grcg = 0;
        self.state.tile = [0; TILE_REGISTER_COUNT];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gdc_with_grcg_bit_extraction_chip_egc() {
        let mut grcg = Grcg::new(GRCG_CHIP_EGC);

        grcg.write_mode(0x80);
        assert_eq!(grcg.state.gdc_with_grcg, 0x08);
        assert!(grcg.gdc_with_grcg_enabled());
        assert!(!grcg.gdc_with_grcg_is_rmw());

        grcg.write_mode(0xC0);
        assert_eq!(grcg.state.gdc_with_grcg, 0x0C);
        assert!(grcg.gdc_with_grcg_enabled());
        assert!(grcg.gdc_with_grcg_is_rmw());

        grcg.write_mode(0x00);
        assert_eq!(grcg.state.gdc_with_grcg, 0x00);
        assert!(!grcg.gdc_with_grcg_enabled());
        assert!(!grcg.gdc_with_grcg_is_rmw());
    }

    #[test]
    fn gdc_with_grcg_not_extracted_for_chip_v1() {
        let mut grcg = Grcg::new(GRCG_CHIP_V1);

        grcg.write_mode(0x80);
        assert_eq!(grcg.state.gdc_with_grcg, 0);

        grcg.write_mode(0xC0);
        assert_eq!(grcg.state.gdc_with_grcg, 0);
    }

    #[test]
    fn write_mode_resets_tile_index() {
        let mut grcg = Grcg::new(GRCG_CHIP_EGC);

        grcg.write_tile(0xAA);
        grcg.write_tile(0xBB);
        assert_eq!(grcg.state.tile_index, 2);

        grcg.write_mode(0x80);
        assert_eq!(grcg.state.tile_index, 0);
    }

    #[test]
    fn tile_write_cycles_through_planes() {
        let mut grcg = Grcg::new(GRCG_CHIP_EGC);

        grcg.write_tile(0x11);
        grcg.write_tile(0x22);
        grcg.write_tile(0x33);
        grcg.write_tile(0x44);

        assert_eq!(grcg.state.tile, [0x11, 0x22, 0x33, 0x44]);
        assert_eq!(grcg.state.tile_index, 0);

        grcg.write_tile(0xFF);
        assert_eq!(grcg.state.tile[0], 0xFF);
        assert_eq!(grcg.state.tile_index, 1);
    }

    #[test]
    fn is_active_and_is_rmw() {
        let mut grcg = Grcg::new(GRCG_CHIP_EGC);

        grcg.write_mode(0x80);
        assert!(grcg.is_active());
        assert!(!grcg.is_rmw());

        grcg.write_mode(0xC0);
        assert!(grcg.is_active());
        assert!(grcg.is_rmw());

        grcg.write_mode(0x40);
        assert!(!grcg.is_active());
        assert!(!grcg.is_rmw());
    }

    #[test]
    fn plane_enabled_active_low() {
        let mut grcg = Grcg::new(GRCG_CHIP_EGC);

        grcg.write_mode(0x80);
        assert!(grcg.plane_enabled(0));
        assert!(grcg.plane_enabled(1));
        assert!(grcg.plane_enabled(2));
        assert!(grcg.plane_enabled(3));

        grcg.write_mode(0x85);
        assert!(!grcg.plane_enabled(0));
        assert!(grcg.plane_enabled(1));
        assert!(!grcg.plane_enabled(2));
        assert!(grcg.plane_enabled(3));
    }

    #[test]
    fn bios_reset_clears_all() {
        let mut grcg = Grcg::new(GRCG_CHIP_EGC);

        grcg.write_mode(0xC5);
        grcg.write_tile(0xAA);
        grcg.write_tile(0xBB);
        grcg.write_tile(0xCC);
        grcg.write_tile(0xDD);

        grcg.bios_reset();

        assert_eq!(grcg.state.mode, 0);
        assert_eq!(grcg.state.tile_index, 0);
        assert_eq!(grcg.state.tile, [0, 0, 0, 0]);
        assert_eq!(grcg.state.gdc_with_grcg, 0);
    }
}
