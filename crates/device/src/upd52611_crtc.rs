//! NEC uPD52611 CRTC - CRT text line/scroll controller.
//!
//! Write-only registers at ports 0x70/0x72/0x74/0x76/0x78/0x7A.
//! Index 0=PL (lines per character row), 1=BL (body face line count),
//! 2=CL (character line count), 3=SSL (smooth scroll offset),
//! 4=SUR (scroll upper limit), 5=SDR (scroll lower limit).

/// Default PL value: 0 lines per character row (24.8kHz / 400-line mode).
const DEFAULT_PL: u8 = 0x00;

/// Default BL value: body face line count (24.8kHz / 400-line mode).
const DEFAULT_BL: u8 = 0x0F;

/// Default CL value: character line count (24.8kHz / 400-line mode).
const DEFAULT_CL: u8 = 0x10;

/// Default SSL value: no smooth scroll offset.
const DEFAULT_SSL: u8 = 0x00;

/// Default SUR value: no scroll upper limit.
const DEFAULT_SUR: u8 = 0x00;

/// Default SDR value: no scroll lower limit.
const DEFAULT_SDR: u8 = 0x00;

save_state::runtime_state! {
/// Authoritative uPD52611 CRTC state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upd52611CrtcState {
    /// CRTC registers [PL, BL, CL, SSL, SUR, SDR] (write-only via ports 0x70-0x7A).
    pub regs: [u8; 6],
}}

/// NEC uPD52611 CRTC controller.
pub struct Upd52611Crtc {
    /// Embedded state for save/restore.
    pub state: Upd52611CrtcState,
}

impl_simple_state_accessors!(Upd52611Crtc, Upd52611CrtcState);

impl Default for Upd52611Crtc {
    fn default() -> Self {
        Self::new()
    }
}

impl Upd52611Crtc {
    /// Creates a new uPD52611 CRTC with default register values for 24.8kHz 400-line mode.
    pub fn new() -> Self {
        Self {
            state: Upd52611CrtcState {
                regs: [
                    DEFAULT_PL,
                    DEFAULT_BL,
                    DEFAULT_CL,
                    DEFAULT_SSL,
                    DEFAULT_SUR,
                    DEFAULT_SDR,
                ],
            },
        }
    }

    /// Writes a CRTC register (index 0-5). Values are masked to 5 bits.
    pub fn write_register(&mut self, index: usize, value: u8) {
        self.state.regs[index] = value & 0x1F;
    }
}
