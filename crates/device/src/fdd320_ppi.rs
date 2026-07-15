//! PC-9801 320KB FDD interface PPI.
//!
//! Early PC-9801 models expose the 320KB floppy subsystem through host
//! interface ports starting at 0x51. We do not support reading from those
//! and instead want them to use the 640KB FDC. BIOS writes to this PPI are
//! accepted as no-ops for compatibility with the real PC-9801F boot probe.

save_state::runtime_state! {
/// Authoritative state of the 320KB FDD PPI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fdd320PpiState {
    /// Alternating status byte returned by port C reads.
    pub status: u8,
}}

/// PC-9801 320KB FDD PPI compatibility shim.
pub struct Fdd320Ppi {
    /// Public state for save-state support.
    pub state: Fdd320PpiState,
}

impl_simple_state_accessors!(Fdd320Ppi, Fdd320PpiState);

impl Default for Fdd320Ppi {
    fn default() -> Self {
        Self::new()
    }
}

impl Fdd320Ppi {
    /// Creates a new 320KB FDD PPI in reset state.
    pub fn new() -> Self {
        Self {
            state: Fdd320PpiState { status: 0xFF },
        }
    }

    /// Reads port 0x51.
    ///
    /// Port A is the subsystem-to-host data/status byte. The shim returns
    /// `0x00` rather than open bus `0xFF`: the PPI exists on PC-9801F, but
    /// there is no emulated 320KB FDD subsystem behind it. BIOS probes get no
    /// usable command response here, so they do not find a bootable 320KB FDD.
    pub fn read_port_a(&mut self) -> u8 {
        0x00
    }

    /// Reads port 0x55 and advances the handshake shim.
    ///
    /// Port C carries the 320KB subsystem handshake lines. Alternating
    /// `0x00` and `0xFF` lets BIOS polling loops observe activity and finish
    /// their probe, while port A still supplies no real protocol data. This is
    /// a compatibility shim, not a ready-drive report.
    pub fn read_port_c(&mut self) -> u8 {
        self.state.status ^= 0xFF;
        self.state.status
    }

    /// Writes port 0x53.
    ///
    /// Port B sends command/data bytes to the intelligent 320KB FDD
    /// subsystem. The shim deliberately has no such subsystem behind it, so
    /// writes are accepted and ignored. This lets the BIOS finish probing the
    /// 320KB path and continue with the 640KB FDC.
    pub fn write_port_b(&mut self, _value: u8) {}

    /// Writes port 0x57.
    ///
    /// This is the 8255 control register for the 320KB FDD PPI. The BIOS uses
    /// it to configure Port A/B/C direction and pulse Port C handshake bits.
    /// We stub those controls with a no-op because no 320KB FDD subsystem is
    /// emulated; boot media should be handled by the 640KB FDC instead.
    pub fn write_control(&mut self, _value: u8) {}
}

#[cfg(test)]
mod tests {
    use super::Fdd320Ppi;

    #[test]
    fn port_c_read_alternates_from_reset() {
        let mut ppi = Fdd320Ppi::new();

        assert_eq!(ppi.read_port_c(), 0x00);
        assert_eq!(ppi.read_port_c(), 0xFF);
        assert_eq!(ppi.read_port_c(), 0x00);
    }

    #[test]
    fn port_a_matches_probe_value() {
        let mut ppi = Fdd320Ppi::new();

        assert_eq!(ppi.read_port_a(), 0x00);
    }

    #[test]
    fn writes_are_no_ops() {
        let mut ppi = Fdd320Ppi::new();

        ppi.write_control(0x91);
        ppi.write_port_b(0x00);
        ppi.write_control(0x0F);

        assert_eq!(ppi.read_port_c(), 0x00);
    }
}
