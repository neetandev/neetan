//! PC-88VA2 Text and Sprite Processor (TSP): the command/parameter protocol and
//! the display-timing derivation that drives the VSYNC loop.

/// Horizontal sync mode, selected by the CRT type (the interlace variant is
/// added with the video controller).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HsyncMode {
    /// 24.8 kHz horizontal sync.
    Khz24_8,
    /// 15.98 kHz horizontal sync.
    Khz15_98,
}

/// Which half of the frame loop the next TSP frame event advances.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePhase {
    /// Start the display interval.
    DisplayStart,
    /// Start the vertical-sync interval.
    Vsync,
}

/// Step of the system-port-4 VSYNC / IRQ chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sysp4Phase {
    /// End the system-port sync interval.
    End,
    /// Start the system-port sync interval.
    Start,
    /// Assert the system-port interrupt.
    Int,
}

/// The command currently collecting parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    None,
    Sync,
    Dspon,
    Dspdef,
    Curdef,
    Spron,
    /// SPRDEF, awaiting its first parameter (the write offset).
    SprdefBegin,
    /// SPRDEF, streaming bytes into the sprite table until the next command.
    SprdefStream,
}

const CMD_SYNC: u8 = 0x10;
const CMD_DSPON: u8 = 0x12;
const CMD_DSPDEF: u8 = 0x14;
const CMD_CURDEF: u8 = 0x15;
const CMD_SPRON: u8 = 0x82;
const CMD_SPROFF: u8 = 0x83;
const CMD_SPRDEF: u8 = 0x84;

const STATUS_BUSY: u8 = 0x04;
const STATUS_VB: u8 = 0x40;

/// A text-VRAM write a TSP command produces, applied by the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TspMemEffect {
    /// Set or clear a sprite's enable bit (word 0, bit 9) at `offset`.
    SpriteEnable {
        /// Byte offset of the sprite descriptor.
        offset: u16,
        /// New sprite enable state.
        enable: bool,
    },
    /// Write a byte into the sprite table at `offset` (SPRDEF stream).
    WriteByte {
        /// Byte offset within the sprite table.
        offset: u16,
        /// Value to store.
        value: u8,
    },
}

/// TSP timing and command state.
pub struct TspState {
    status: u8,
    command: Command,
    recvdatacnt: u8,
    paramindex: usize,
    parambuf: [u8; 16],
    syncparam: [u8; 14],

    /// Display phase duration in CPU cycles (frame start to VSYNC).
    pub dispclock: u64,
    /// VSYNC phase duration in CPU cycles.
    pub vsyncclock: u64,
    /// System-port-4 VSYNC window offset from frame start, in CPU cycles.
    pub sysp4vsyncextension: u64,
    /// System-port-4 display duration in CPU cycles.
    pub sysp4dispclock: u64,

    /// TSP VSYNC status (port 0x142 bit 6 when non-zero).
    pub vsync: u8,
    /// System-port-4 VSYNC status (port 0x040 bit 5).
    pub sysp4vsync: u8,

    /// Next frame timing phase.
    pub frame_phase: FramePhase,
    /// Next system-port-4 timing phase.
    pub sysp4_phase: Sysp4Phase,

    /// Text-table base offset within text VRAM (DSPON).
    pub texttable: u16,
    /// Attribute byte offset relative to a character code (DSPDEF).
    pub attroffset: u16,
    /// Scanlines per text row (DSPDEF, stored as the programmed value + 1).
    pub lineheight: u8,
    /// Horizontal-line raster position (DSPDEF).
    pub hlinepos: u8,
    /// Blink-rate reload value (DSPDEF, bits 5-7 of the rate byte).
    pub blink: u8,
    /// Text display enabled (DSPON).
    pub dspon: bool,
    /// Blink countdown, reloaded from `blink` each cycle.
    pub blinkcnt: u8,
    /// Blink phase counter, incremented when `blinkcnt` reaches zero.
    pub blinkcnt2: u8,
    /// Cursor sprite number (CURDEF).
    pub curn: u8,
    /// Cursor blink enable (CURDEF).
    pub be: bool,
    /// Sprite-table base offset within text VRAM (SPRON).
    pub sprtable: u16,
    /// Maximum sprites per raster minus one (SPRON `hspn`).
    pub hspn: u8,
    /// Sprite 2x vertical magnification (SPRON `mg`).
    pub mg: bool,
    /// Sprite grouping mode (SPRON `gr`).
    pub gr: bool,
    /// Sprite display enabled (SPRON / SPROFF).
    pub spron: bool,
    /// Next write offset for a SPRDEF stream, relative to `sprtable`.
    sprdef_offset: u16,
    /// Text 2x vertical magnification (SYNC parameter 0).
    pub textmg: bool,
    /// Programmed screen line count (SYNC parameters 0x0a/0x0b).
    pub screenlines: u16,
}

impl Default for TspState {
    fn default() -> Self {
        Self::new()
    }
}

impl TspState {
    /// Creates reset TSP timing and command state.
    pub fn new() -> Self {
        Self {
            status: 0,
            command: Command::None,
            recvdatacnt: 0,
            paramindex: 0,
            parambuf: [0; 16],
            syncparam: [0; 14],
            dispclock: 0,
            vsyncclock: 0,
            sysp4vsyncextension: 0,
            sysp4dispclock: 0,
            vsync: 0,
            sysp4vsync: 0,
            frame_phase: FramePhase::DisplayStart,
            sysp4_phase: Sysp4Phase::End,
            texttable: 0,
            attroffset: 0,
            lineheight: 0,
            hlinepos: 0,
            blink: 0,
            dspon: false,
            blinkcnt: 0,
            blinkcnt2: 0,
            curn: 0,
            be: false,
            sprtable: 0,
            hspn: 0,
            mg: false,
            gr: false,
            spron: false,
            sprdef_offset: 0,
            textmg: false,
            screenlines: 0,
        }
    }

    /// SYNC parameter 0, selecting the 200-line text doubling case.
    pub fn sync_param0(&self) -> u8 {
        self.syncparam[0]
    }

    /// Advances the text blink counters once per frame (`screenvsyncva`).
    pub fn tick_blink(&mut self) {
        self.blinkcnt = self.blinkcnt.wrapping_sub(1);
        if self.blinkcnt == 0 {
            self.blinkcnt = self.blink;
            self.blinkcnt2 = self.blinkcnt2.wrapping_add(1);
        }
    }

    /// Status read (port 0x142): the busy flag plus the VSYNC (VB) bit.
    pub fn read_status(&self) -> u8 {
        let vb = if self.vsync != 0 { STATUS_VB } else { 0 };
        self.status | vb
    }

    /// Command write (port 0x142): latches the command and arms its parameter
    /// count. Commands without parameters complete immediately.
    pub fn write_command(&mut self, command: u8) -> Option<TspMemEffect> {
        self.paramindex = 0;
        self.status |= STATUS_BUSY;
        match command {
            CMD_SYNC => {
                self.command = Command::Sync;
                self.recvdatacnt = 14;
            }
            CMD_DSPON => {
                self.command = Command::Dspon;
                self.recvdatacnt = 3;
            }
            CMD_SPRON => {
                self.command = Command::Spron;
                self.recvdatacnt = 3;
            }
            CMD_DSPDEF => {
                self.command = Command::Dspdef;
                self.recvdatacnt = 6;
            }
            CMD_CURDEF => {
                self.command = Command::Curdef;
                self.recvdatacnt = 1;
            }
            CMD_SPRDEF => {
                // SPRDEF streams bytes into the sprite table; the first parameter
                // is the offset and busy stays set until the next command.
                self.command = Command::SprdefBegin;
                self.recvdatacnt = 0;
            }
            CMD_SPROFF => {
                self.spron = false;
                self.command = Command::None;
                self.recvdatacnt = 0;
                self.status &= !STATUS_BUSY;
            }
            _ => {
                // DSPOFF, EXIT, and unknown commands take no parameters.
                self.command = Command::None;
                self.recvdatacnt = 0;
                self.status &= !STATUS_BUSY;
            }
        }
        None
    }

    /// Parameter write (port 0x146): fills the parameter buffer and executes the
    /// command once the last byte arrives, or streams a SPRDEF byte.
    pub fn write_parameter(&mut self, value: u8) -> Option<TspMemEffect> {
        match self.command {
            Command::SprdefBegin => {
                self.sprdef_offset = u16::from(value);
                self.command = Command::SprdefStream;
                return None;
            }
            Command::SprdefStream => {
                let offset = self.sprtable.wrapping_add(self.sprdef_offset);
                self.sprdef_offset = self.sprdef_offset.wrapping_add(1);
                return Some(TspMemEffect::WriteByte { offset, value });
            }
            _ => {}
        }

        if self.recvdatacnt == 0 {
            return None;
        }
        if self.paramindex < self.parambuf.len() {
            self.parambuf[self.paramindex] = value;
        }
        self.paramindex += 1;
        self.recvdatacnt -= 1;
        if self.recvdatacnt == 0 {
            let effect = self.execute_command();
            self.command = Command::None;
            self.status &= !STATUS_BUSY;
            return effect;
        }
        None
    }

    /// Applies a fully received command's parameters to the TSP state.
    fn execute_command(&mut self) -> Option<TspMemEffect> {
        let parameters = self.parambuf;
        match self.command {
            Command::Sync => {
                self.syncparam.copy_from_slice(&parameters[..14]);
                self.textmg = self.syncparam[0] & 0xC0 == 0x80;
                self.screenlines =
                    u16::from(self.syncparam[0x0A]) | (u16::from(self.syncparam[0x0B] & 0x40) << 2);
                None
            }
            Command::Dspon => {
                self.texttable = u16::from(parameters[0]) << 8;
                self.dspon = true;
                None
            }
            Command::Dspdef => {
                self.attroffset = u16::from(parameters[0]) | (u16::from(parameters[1]) << 8);
                self.lineheight = parameters[3].wrapping_add(1);
                self.hlinepos = parameters[4];
                self.blink = parameters[5] >> 3;
                self.blinkcnt = self.blink;
                None
            }
            Command::Curdef => {
                self.curn = parameters[0] >> 3;
                self.be = parameters[0] & 0x01 != 0;
                let offset = self.sprtable.wrapping_add(u16::from(self.curn) * 8);
                Some(TspMemEffect::SpriteEnable {
                    offset,
                    enable: parameters[0] & 0x02 != 0,
                })
            }
            Command::Spron => {
                self.sprtable = u16::from(parameters[0]) << 8;
                self.hspn = parameters[2] >> 3;
                self.mg = parameters[2] & 0x02 != 0;
                self.gr = parameters[2] & 0x01 != 0;
                self.spron = true;
                None
            }
            Command::None | Command::SprdefBegin | Command::SprdefStream => None,
        }
    }

    /// Recomputes the frame timing from the SYNC parameters and the CPU clock.
    /// With no SYNC programmed (`vad == 0 && had == 0`) the reset default of
    /// 24.8 kHz / 400 lines applies.
    pub fn update_clock(&mut self, main_clock_hz: u32, hsyncmode: HsyncMode) {
        let parameters = &self.syncparam;
        let mut lbl = u64::from(parameters[2] & 0x3F);
        let mut lbr = u64::from(parameters[3] & 0x3F);
        let mut had = u64::from(parameters[4]);
        let mut rbr = u64::from(parameters[5] & 0x3F);
        let mut rbl = u64::from(parameters[6] & 0x3F);
        let mut hs = u64::from(parameters[7] & 0x3F);
        let mut tbl = u64::from(parameters[8] & 0x3F);
        let mut tbr = u64::from(parameters[9] & 0x3F);
        let mut vad = u64::from(parameters[10]) + (u64::from(parameters[11] & 0x40) << 2);
        let mut bbr = u64::from(parameters[11] & 0x3F);
        let mut bbl = u64::from(parameters[12] & 0x3F);
        let mut vs = u64::from(parameters[13] & 0x3F);

        let mode = if vad == 0 && had == 0 {
            lbl = 0x10;
            lbr = 0;
            had = 0x9F;
            rbl = 0x10;
            rbr = 0;
            hs = 0x0F;
            tbl = 0x19;
            tbr = 0;
            vad = 0x190;
            bbl = 0x07;
            bbr = 0;
            vs = 8;
            HsyncMode::Khz24_8
        } else {
            hsyncmode
        };

        let (dot_clock, sysp4displines_base, sysp4vsyncexlines) = match mode {
            HsyncMode::Khz24_8 => (20_854_022u64, 402u64, 25u64),
            HsyncMode::Khz15_98 => (14_189_837, 202, 37),
        };

        if vs < 4 {
            vs = 4;
        }
        if vad < 4 {
            vad = 4;
        }
        had |= 1;
        if hs < 4 {
            hs = 4;
        }
        if lbl < 3 {
            lbl = 3;
        }

        let width = (lbl + 1 + lbr + had + 1 + rbr + rbl + 1 + hs + 1) * 4;
        let height = tbl + tbr + vad + bbr + bbl + vs;

        let mut sysp4displines = sysp4displines_base;
        if sysp4displines + sysp4vsyncexlines >= height {
            sysp4displines = height - sysp4vsyncexlines - 4;
        }

        let hclock = dot_clock / width;
        let frame = u64::from(main_clock_hz) * height / hclock;
        self.vsyncclock = frame * (bbr + bbl + vs) / height;
        self.dispclock = frame - self.vsyncclock;
        self.sysp4vsyncextension = frame * sysp4vsyncexlines / height;
        self.sysp4dispclock = frame * sysp4displines / height;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN_CLOCK_HZ: u32 = 7_987_200;

    #[test]
    fn default_timing_is_24khz_400_line() {
        let mut tsp = TspState::new();
        tsp.update_clock(MAIN_CLOCK_HZ, HsyncMode::Khz24_8);

        // w = 840, h = 440, hclock = 20_854_022 / 840 = 24826,
        // frame = 7_987_200 * 440 / 24826 = 141_559.
        const FRAME: u64 = 141_559;
        assert_eq!(tsp.dispclock + tsp.vsyncclock, FRAME);
        assert_eq!(tsp.vsyncclock, FRAME * 15 / 440);
        assert_eq!(tsp.dispclock, FRAME - FRAME * 15 / 440);
        assert_eq!(tsp.sysp4vsyncextension, FRAME * 25 / 440);
        assert_eq!(tsp.sysp4dispclock, FRAME * 402 / 440);
    }

    #[test]
    fn sync_command_drives_the_timing() {
        let mut tsp = TspState::new();
        tsp.write_command(CMD_SYNC);
        assert_eq!(tsp.read_status() & STATUS_BUSY, STATUS_BUSY);

        let mut params = [0u8; 14];
        params[2] = 0x10; // lbl
        params[4] = 0x9F; // had
        params[6] = 0x10; // rbl
        params[7] = 0x0F; // hs
        params[8] = 0x19; // tbl
        params[10] = 0xC8; // vad = 200
        params[12] = 0x07; // bbl
        params[13] = 0x08; // vs
        for (index, byte) in params.iter().enumerate() {
            assert_eq!(tsp.read_status() & STATUS_BUSY, STATUS_BUSY);
            tsp.write_parameter(*byte);
            // The buffer reached the TSP intact.
            assert_eq!(tsp.parambuf[index], *byte);
        }
        assert_eq!(tsp.read_status() & STATUS_BUSY, 0);

        // A 200-line frame must be shorter than the 400-line default.
        tsp.update_clock(MAIN_CLOCK_HZ, HsyncMode::Khz24_8);
        let shorter = tsp.dispclock + tsp.vsyncclock;
        let mut default_tsp = TspState::new();
        default_tsp.update_clock(MAIN_CLOCK_HZ, HsyncMode::Khz24_8);
        assert!(shorter < default_tsp.dispclock + default_tsp.vsyncclock);
    }

    #[test]
    fn command_without_parameters_clears_busy_immediately() {
        let mut tsp = TspState::new();
        tsp.write_command(0x88); // EXIT
        assert_eq!(tsp.read_status() & STATUS_BUSY, 0);
    }

    #[test]
    fn spron_decodes_table_limit_magnify_and_grouping() {
        let mut tsp = TspState::new();
        tsp.write_command(CMD_SPRON);
        tsp.write_parameter(0x7E); // sprtable high byte
        tsp.write_parameter(0x00);
        // hspn = 0x1F (>>3 of 0xFB), mg set (bit 1), gr set (bit 0).
        tsp.write_parameter(0xFB);
        assert_eq!(tsp.sprtable, 0x7E00);
        assert_eq!(tsp.hspn, 0x1F);
        assert!(tsp.mg);
        assert!(tsp.gr);
        assert!(tsp.spron);
        assert_eq!(tsp.read_status() & STATUS_BUSY, 0);
    }

    #[test]
    fn sproff_clears_sprite_enable() {
        let mut tsp = TspState::new();
        tsp.write_command(CMD_SPRON);
        tsp.write_parameter(0x7E);
        tsp.write_parameter(0x00);
        tsp.write_parameter(0x00);
        assert!(tsp.spron);
        tsp.write_command(CMD_SPROFF);
        assert!(!tsp.spron);
        assert_eq!(tsp.read_status() & STATUS_BUSY, 0);
    }

    #[test]
    fn curdef_emits_sprite_enable_effect() {
        let mut tsp = TspState::new();
        tsp.write_command(CMD_SPRON);
        tsp.write_parameter(0x7E);
        tsp.write_parameter(0x00);
        tsp.write_parameter(0x00);

        tsp.write_command(CMD_CURDEF);
        // curn = 3 (>>3 of 0x1B), be set (bit 0), show cursor set (bit 1).
        let effect = tsp.write_parameter(0x1B);
        assert_eq!(tsp.curn, 3);
        assert!(tsp.be);
        assert_eq!(
            effect,
            Some(TspMemEffect::SpriteEnable {
                offset: 0x7E00 + 3 * 8,
                enable: true,
            })
        );
    }

    #[test]
    fn sprdef_streams_bytes_into_the_table() {
        let mut tsp = TspState::new();
        tsp.write_command(CMD_SPRON);
        tsp.write_parameter(0x7E);
        tsp.write_parameter(0x00);
        tsp.write_parameter(0x00);

        tsp.write_command(CMD_SPRDEF);
        // First parameter is the offset; no write effect yet.
        assert_eq!(tsp.write_parameter(0x10), None);
        // Subsequent parameters stream into the table at increasing offsets.
        assert_eq!(
            tsp.write_parameter(0xAB),
            Some(TspMemEffect::WriteByte {
                offset: 0x7E00 + 0x10,
                value: 0xAB,
            })
        );
        assert_eq!(
            tsp.write_parameter(0xCD),
            Some(TspMemEffect::WriteByte {
                offset: 0x7E00 + 0x11,
                value: 0xCD,
            })
        );
        // EXIT ends the stream and clears busy.
        tsp.write_command(0x88);
        assert_eq!(tsp.read_status() & STATUS_BUSY, 0);
        assert_eq!(tsp.write_parameter(0xFF), None);
    }
}
