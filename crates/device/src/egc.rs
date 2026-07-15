//! EGC (Enhanced Graphic Charger) - raster-operation engine for VRAM.
//!
//! 8 I/O registers at ports 0x04A0–0x04AE (word-accessible).
//! Provides 256 ROP codes with 3-input truth table (Source/Dest/Pattern),
//! bit-level shifting with 6 modes, pattern registers, and compare-read.

/// Lookup table: expands a 4-bit color index to per-plane u16 masks.
/// maskword[color][plane] - if bit `plane` of `color` is set, the mask is 0xFFFF, else 0x0000.
const MASKWORD: [[u16; 4]; 16] = [
    [0x0000, 0x0000, 0x0000, 0x0000],
    [0xFFFF, 0x0000, 0x0000, 0x0000],
    [0x0000, 0xFFFF, 0x0000, 0x0000],
    [0xFFFF, 0xFFFF, 0x0000, 0x0000],
    [0x0000, 0x0000, 0xFFFF, 0x0000],
    [0xFFFF, 0x0000, 0xFFFF, 0x0000],
    [0x0000, 0xFFFF, 0xFFFF, 0x0000],
    [0xFFFF, 0xFFFF, 0xFFFF, 0x0000],
    [0x0000, 0x0000, 0x0000, 0xFFFF],
    [0xFFFF, 0x0000, 0x0000, 0xFFFF],
    [0x0000, 0xFFFF, 0x0000, 0xFFFF],
    [0xFFFF, 0xFFFF, 0x0000, 0xFFFF],
    [0x0000, 0x0000, 0xFFFF, 0xFFFF],
    [0xFFFF, 0x0000, 0xFFFF, 0xFFFF],
    [0x0000, 0xFFFF, 0xFFFF, 0xFFFF],
    [0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF],
];

/// Byte mask tables for shift source masking.
/// dir:right by startbit + (len-1)*8
#[rustfmt::skip]
const BYTEMASK_U0: [u8; 64] = [
    0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01,
    0xC0, 0x60, 0x30, 0x18, 0x0C, 0x06, 0x03, 0x01,
    0xE0, 0x70, 0x38, 0x1C, 0x0E, 0x07, 0x03, 0x01,
    0xF0, 0x78, 0x3C, 0x1E, 0x0F, 0x07, 0x03, 0x01,
    0xF8, 0x7C, 0x3E, 0x1F, 0x0F, 0x07, 0x03, 0x01,
    0xFC, 0x7E, 0x3F, 0x1F, 0x0F, 0x07, 0x03, 0x01,
    0xFE, 0x7F, 0x3F, 0x1F, 0x0F, 0x07, 0x03, 0x01,
    0xFF, 0x7F, 0x3F, 0x1F, 0x0F, 0x07, 0x03, 0x01,
];

/// dir:right by length
const BYTEMASK_U1: [u8; 8] = [0x80, 0xC0, 0xE0, 0xF0, 0xF8, 0xFC, 0xFE, 0xFF];

/// dir:left by startbit + (len-1)*8
#[rustfmt::skip]
const BYTEMASK_D0: [u8; 64] = [
    0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80,
    0x03, 0x06, 0x0C, 0x18, 0x30, 0x60, 0xC0, 0x80,
    0x07, 0x0E, 0x1C, 0x38, 0x70, 0xE0, 0xC0, 0x80,
    0x0F, 0x1E, 0x3C, 0x78, 0xF0, 0xE0, 0xC0, 0x80,
    0x1F, 0x3E, 0x7C, 0xF8, 0xF0, 0xE0, 0xC0, 0x80,
    0x3F, 0x7E, 0xFC, 0xF8, 0xF0, 0xE0, 0xC0, 0x80,
    0x7F, 0xFE, 0xFC, 0xF8, 0xF0, 0xE0, 0xC0, 0x80,
    0xFF, 0xFE, 0xFC, 0xF8, 0xF0, 0xE0, 0xC0, 0x80,
];

/// dir:left by length
const BYTEMASK_D1: [u8; 8] = [0x01, 0x03, 0x07, 0x0F, 0x1F, 0x3F, 0x7F, 0xFF];

/// Buffer size: 4096/8 + 4*4 = 528 bytes.
const BUF_SIZE: usize = 4096 / 8 + 4 * 4;

/// Ascending direction buffer start offset (func 0, 2, 4).
const BUF_START_INC: usize = 0;

/// Descending direction buffer start offset (func 1, 3, 5).
const BUF_START_DEC: usize = 4096 / 8 + 3;

save_state::runtime_state! {
/// Authoritative EGC state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgcState {
    /// Reg 0 (0x04A0): plane write enable (active-low, bits 0-3).
    pub access: u16,
    /// Reg 1 (0x04A2): read plane select + FGC/BGC source.
    pub fgbg: u16,
    /// Reg 2 (0x04A4): ROP code (bits 7-0), write/read source, compare.
    pub ope: u16,
    /// Reg 3 (0x04A6): foreground color (4-bit).
    pub fg: u16,
    /// Reg 4 (0x04A8): mask register.
    pub mask: u16,
    /// Reg 5 (0x04AA): background color (4-bit).
    pub bg: u16,
    /// Reg 6 (0x04AC): shift control (direction, dst/src bit addresses).
    pub sft: u16,
    /// Reg 7 (0x04AE): bit length.
    pub leng: u16,
    /// Last VRAM data read (4 planes).
    pub lastvram: [u16; 4],
    /// Pattern register (4 planes), loadable from VRAM.
    pub patreg: [u16; 4],
    /// Expanded foreground color (4 planes).
    pub fgc: [u16; 4],
    /// Expanded background color (4 planes).
    pub bgc: [u16; 4],
    /// Shift function index (0-5).
    pub func: u8,
    /// Remaining bits in current operation.
    pub remain: u32,
    /// Buffered bit count.
    pub stack: u32,
    /// Source bit address.
    pub srcbit: u8,
    /// Destination bit address.
    pub dstbit: u8,
    /// Sub-byte left shift amount.
    pub sft8bitl: u8,
    /// Sub-byte right shift amount.
    pub sft8bitr: u8,
    /// Final effective mask (user mask & srcmask).
    pub mask2: u16,
    /// Source validity mask from shift output.
    pub srcmask: u16,
    /// Buffer input pointer offset.
    pub inptr: usize,
    /// Buffer output pointer offset.
    pub outptr: usize,
    /// Shift buffer (4 planes interleaved at stride 4).
    pub buf: Box<[u8]>,
}}

impl Default for EgcState {
    fn default() -> Self {
        Self {
            access: 0xFFF0,
            fgbg: 0x00FF,
            ope: 0,
            fg: 0,
            mask: 0xFFFF,
            bg: 0,
            sft: 0,
            leng: 0x000F,
            lastvram: [0; 4],
            patreg: [0; 4],
            fgc: [0; 4],
            bgc: [0; 4],
            func: 0,
            remain: 16,
            stack: 0,
            srcbit: 0,
            dstbit: 0,
            sft8bitl: 0,
            sft8bitr: 0,
            mask2: 0,
            srcmask: 0xFFFF,
            inptr: BUF_START_INC,
            outptr: BUF_START_INC,
            buf: vec![0; BUF_SIZE].into_boxed_slice(),
        }
    }
}

/// EGC controller.
pub struct Egc {
    /// Embedded state for save/restore.
    pub state: EgcState,
    // Transient shift output (not saved, but instead recomputed from buf on each operation).
    src: [u16; 4],
}

impl_simple_state_accessors!(Egc, EgcState);

impl Default for Egc {
    fn default() -> Self {
        Self::new()
    }
}

impl Egc {
    /// Creates a new EGC in its reset state.
    pub fn new() -> Self {
        let mut egc = Self {
            state: EgcState::default(),
            src: [0; 4],
        };
        egc.recalculate_shift();
        egc
    }

    /// Resets the EGC to power-on defaults.
    pub fn reset(&mut self) {
        self.state = EgcState::default();
        self.src = [0; 4];
        self.recalculate_shift();
        self.state.srcmask = 0xFFFF;
    }

    /// Writes an EGC register (byte access). `port` is the low nibble (0x00..=0x0F).
    pub fn write_register_byte(&mut self, port: u8, value: u8) {
        match port {
            0x00 => {
                self.state.access = (self.state.access & 0xFF00) | u16::from(value);
            }
            0x01 => {
                self.state.access = (self.state.access & 0x00FF) | (u16::from(value) << 8);
            }
            0x02 => {
                self.state.fgbg = (self.state.fgbg & 0xFF00) | u16::from(value);
            }
            0x03 => {
                self.state.fgbg = (self.state.fgbg & 0x00FF) | (u16::from(value) << 8);
            }
            0x04 => {
                self.state.ope = (self.state.ope & 0xFF00) | u16::from(value);
            }
            0x05 => {
                self.state.ope = (self.state.ope & 0x00FF) | (u16::from(value) << 8);
            }
            0x06 => {
                self.state.fg = (self.state.fg & 0xFF00) | u16::from(value);
                let color = (value & 0x0F) as usize;
                self.state.fgc = MASKWORD[color];
            }
            0x07 => {
                self.state.fg = (self.state.fg & 0x00FF) | (u16::from(value) << 8);
            }
            0x08 if self.state.fgbg & 0x6000 == 0 => {
                self.state.mask = (self.state.mask & 0xFF00) | u16::from(value);
            }
            0x09 if self.state.fgbg & 0x6000 == 0 => {
                self.state.mask = (self.state.mask & 0x00FF) | (u16::from(value) << 8);
            }
            0x0A => {
                self.state.bg = (self.state.bg & 0xFF00) | u16::from(value);
                let color = (value & 0x0F) as usize;
                self.state.bgc = MASKWORD[color];
            }
            0x0B => {
                self.state.bg = (self.state.bg & 0x00FF) | (u16::from(value) << 8);
            }
            0x0C => {
                self.state.sft = (self.state.sft & 0xFF00) | u16::from(value);
                self.recalculate_shift();
                self.state.srcmask = 0xFFFF;
            }
            0x0D => {
                self.state.sft = (self.state.sft & 0x00FF) | (u16::from(value) << 8);
                self.recalculate_shift();
                self.state.srcmask = 0xFFFF;
            }
            0x0E => {
                self.state.leng = (self.state.leng & 0xFF00) | u16::from(value);
                self.recalculate_shift();
                self.state.srcmask = 0xFFFF;
            }
            0x0F => {
                self.state.leng = (self.state.leng & 0x00FF) | (u16::from(value) << 8);
                self.recalculate_shift();
                self.state.srcmask = 0xFFFF;
            }
            _ => {}
        }
    }

    /// Writes an EGC register (word access). `port` is the low nibble (even: 0x00..=0x0E).
    pub fn write_register_word(&mut self, port: u8, value: u16) {
        match port {
            0x00 => self.state.access = value,
            0x02 => self.state.fgbg = value,
            0x04 => self.state.ope = value,
            0x06 => {
                self.state.fg = value;
                let color = (value & 0x0F) as usize;
                self.state.fgc = MASKWORD[color];
            }
            0x08 if self.state.fgbg & 0x6000 == 0 => {
                self.state.mask = value;
            }
            0x0A => {
                self.state.bg = value;
                let color = (value & 0x0F) as usize;
                self.state.bgc = MASKWORD[color];
            }
            0x0C => {
                self.state.sft = value;
                self.recalculate_shift();
                self.state.srcmask = 0xFFFF;
            }
            0x0E => {
                self.state.leng = value;
                self.recalculate_shift();
                self.state.srcmask = 0xFFFF;
            }
            _ => {}
        }
    }

    /// Recalculates shift parameters from sft/leng registers.
    fn recalculate_shift(&mut self) {
        self.state.remain = (self.state.leng & 0x0FFF) as u32 + 1;
        self.state.func = ((self.state.sft >> 12) & 1) as u8;

        if self.state.func == 0 {
            self.state.inptr = BUF_START_INC;
            self.state.outptr = BUF_START_INC;
        } else {
            self.state.inptr = BUF_START_DEC;
            self.state.outptr = BUF_START_DEC;
        }

        self.state.srcbit = (self.state.sft & 0x0F) as u8;
        self.state.dstbit = ((self.state.sft >> 4) & 0x0F) as u8;

        let src8 = self.state.srcbit & 0x07;
        let dst8 = self.state.dstbit & 0x07;

        if src8 < dst8 {
            self.state.func += 2;
            self.state.sft8bitr = dst8 - src8;
            self.state.sft8bitl = 8 - self.state.sft8bitr;
        } else if src8 > dst8 {
            self.state.func += 4;
            self.state.sft8bitl = src8 - dst8;
            self.state.sft8bitr = 8 - self.state.sft8bitl;
        }

        self.state.stack = 0;
    }

    fn sftb_upn_sub(&mut self, ext: usize) {
        if self.state.dstbit >= 8 {
            self.state.dstbit -= 8;
            set_srcmask_byte(&mut self.state.srcmask, ext, 0);
            return;
        }
        if self.state.dstbit > 0 {
            if (u32::from(self.state.dstbit) + self.state.remain) >= 8 {
                set_srcmask_byte(
                    &mut self.state.srcmask,
                    ext,
                    BYTEMASK_U0[self.state.dstbit as usize + 7 * 8],
                );
                self.state.remain -= 8 - u32::from(self.state.dstbit);
                self.state.dstbit = 0;
            } else {
                set_srcmask_byte(
                    &mut self.state.srcmask,
                    ext,
                    BYTEMASK_U0[self.state.dstbit as usize + (self.state.remain as usize - 1) * 8],
                );
                self.state.remain = 0;
                self.state.dstbit = 0;
            }
        } else if self.state.remain >= 8 {
            self.state.remain -= 8;
        } else {
            set_srcmask_byte(
                &mut self.state.srcmask,
                ext,
                BYTEMASK_U1[self.state.remain as usize - 1],
            );
            self.state.remain = 0;
        }
        let o = self.state.outptr;
        self.src_set_byte(0, ext, self.state.buf[o]);
        self.src_set_byte(1, ext, self.state.buf[o + 4]);
        self.src_set_byte(2, ext, self.state.buf[o + 8]);
        self.src_set_byte(3, ext, self.state.buf[o + 12]);
        self.state.outptr += 1;
    }

    fn sftb_dnn_sub(&mut self, ext: usize) {
        if self.state.dstbit >= 8 {
            self.state.dstbit -= 8;
            set_srcmask_byte(&mut self.state.srcmask, ext, 0);
            return;
        }
        if self.state.dstbit > 0 {
            if (u32::from(self.state.dstbit) + self.state.remain) >= 8 {
                set_srcmask_byte(
                    &mut self.state.srcmask,
                    ext,
                    BYTEMASK_D0[self.state.dstbit as usize + 7 * 8],
                );
                self.state.remain -= 8 - u32::from(self.state.dstbit);
                self.state.dstbit = 0;
            } else {
                set_srcmask_byte(
                    &mut self.state.srcmask,
                    ext,
                    BYTEMASK_D0[self.state.dstbit as usize + (self.state.remain as usize - 1) * 8],
                );
                self.state.remain = 0;
                self.state.dstbit = 0;
            }
        } else if self.state.remain >= 8 {
            self.state.remain -= 8;
        } else {
            set_srcmask_byte(
                &mut self.state.srcmask,
                ext,
                BYTEMASK_D1[self.state.remain as usize - 1],
            );
            self.state.remain = 0;
        }
        let o = self.state.outptr;
        self.src_set_byte(0, ext, self.state.buf[o]);
        self.src_set_byte(1, ext, self.state.buf[o + 4]);
        self.src_set_byte(2, ext, self.state.buf[o + 8]);
        self.src_set_byte(3, ext, self.state.buf[o + 12]);
        self.state.outptr = self.state.outptr.wrapping_sub(1);
    }

    fn sftb_upr_sub(&mut self, ext: usize) {
        if self.state.dstbit >= 8 {
            self.state.dstbit -= 8;
            set_srcmask_byte(&mut self.state.srcmask, ext, 0);
            return;
        }
        let r = self.state.sft8bitr;
        let l = self.state.sft8bitl;
        if self.state.dstbit > 0 {
            if (u32::from(self.state.dstbit) + self.state.remain) >= 8 {
                set_srcmask_byte(
                    &mut self.state.srcmask,
                    ext,
                    BYTEMASK_U0[self.state.dstbit as usize + 7 * 8],
                );
                self.state.remain -= 8 - u32::from(self.state.dstbit);
            } else {
                set_srcmask_byte(
                    &mut self.state.srcmask,
                    ext,
                    BYTEMASK_U0[self.state.dstbit as usize + (self.state.remain as usize - 1) * 8],
                );
                self.state.remain = 0;
            }
            self.state.dstbit = 0;
            let o = self.state.outptr;
            self.src_set_byte(0, ext, self.state.buf[o] >> r);
            self.src_set_byte(1, ext, self.state.buf[o + 4] >> r);
            self.src_set_byte(2, ext, self.state.buf[o + 8] >> r);
            self.src_set_byte(3, ext, self.state.buf[o + 12] >> r);
        } else {
            if self.state.remain >= 8 {
                self.state.remain -= 8;
            } else {
                set_srcmask_byte(
                    &mut self.state.srcmask,
                    ext,
                    BYTEMASK_U1[self.state.remain as usize - 1],
                );
                self.state.remain = 0;
            }
            let o = self.state.outptr;
            self.src_set_byte(
                0,
                ext,
                (self.state.buf[o] << l) | (self.state.buf[o + 1] >> r),
            );
            self.src_set_byte(
                1,
                ext,
                (self.state.buf[o + 4] << l) | (self.state.buf[o + 5] >> r),
            );
            self.src_set_byte(
                2,
                ext,
                (self.state.buf[o + 8] << l) | (self.state.buf[o + 9] >> r),
            );
            self.src_set_byte(
                3,
                ext,
                (self.state.buf[o + 12] << l) | (self.state.buf[o + 13] >> r),
            );
            self.state.outptr += 1;
        }
    }

    fn sftb_dnr_sub(&mut self, ext: usize) {
        if self.state.dstbit >= 8 {
            self.state.dstbit -= 8;
            set_srcmask_byte(&mut self.state.srcmask, ext, 0);
            return;
        }
        let r = self.state.sft8bitr;
        let l = self.state.sft8bitl;
        if self.state.dstbit > 0 {
            if (u32::from(self.state.dstbit) + self.state.remain) >= 8 {
                set_srcmask_byte(
                    &mut self.state.srcmask,
                    ext,
                    BYTEMASK_D0[self.state.dstbit as usize + 7 * 8],
                );
                self.state.remain -= 8 - u32::from(self.state.dstbit);
            } else {
                set_srcmask_byte(
                    &mut self.state.srcmask,
                    ext,
                    BYTEMASK_D0[self.state.dstbit as usize + (self.state.remain as usize - 1) * 8],
                );
                self.state.remain = 0;
            }
            self.state.dstbit = 0;
            let o = self.state.outptr;
            self.src_set_byte(0, ext, self.state.buf[o] << r);
            self.src_set_byte(1, ext, self.state.buf[o + 4] << r);
            self.src_set_byte(2, ext, self.state.buf[o + 8] << r);
            self.src_set_byte(3, ext, self.state.buf[o + 12] << r);
        } else {
            if self.state.remain >= 8 {
                self.state.remain -= 8;
            } else {
                set_srcmask_byte(
                    &mut self.state.srcmask,
                    ext,
                    BYTEMASK_D1[self.state.remain as usize - 1],
                );
                self.state.remain = 0;
            }
            self.state.outptr = self.state.outptr.wrapping_sub(1);
            let o = self.state.outptr;
            self.src_set_byte(
                0,
                ext,
                (self.state.buf[o + 1] >> l) | (self.state.buf[o] << r),
            );
            self.src_set_byte(
                1,
                ext,
                (self.state.buf[o + 5] >> l) | (self.state.buf[o + 4] << r),
            );
            self.src_set_byte(
                2,
                ext,
                (self.state.buf[o + 9] >> l) | (self.state.buf[o + 8] << r),
            );
            self.src_set_byte(
                3,
                ext,
                (self.state.buf[o + 13] >> l) | (self.state.buf[o + 12] << r),
            );
        }
    }

    fn sftb_upl_sub(&mut self, ext: usize) {
        if self.state.dstbit >= 8 {
            self.state.dstbit -= 8;
            set_srcmask_byte(&mut self.state.srcmask, ext, 0);
            return;
        }
        let l = self.state.sft8bitl;
        let r = self.state.sft8bitr;
        if self.state.dstbit > 0 {
            if (u32::from(self.state.dstbit) + self.state.remain) >= 8 {
                set_srcmask_byte(
                    &mut self.state.srcmask,
                    ext,
                    BYTEMASK_U0[self.state.dstbit as usize + 7 * 8],
                );
                self.state.remain -= 8 - u32::from(self.state.dstbit);
                self.state.dstbit = 0;
            } else {
                set_srcmask_byte(
                    &mut self.state.srcmask,
                    ext,
                    BYTEMASK_U0[self.state.dstbit as usize + (self.state.remain as usize - 1) * 8],
                );
                self.state.remain = 0;
                self.state.dstbit = 0;
            }
        } else if self.state.remain >= 8 {
            self.state.remain -= 8;
        } else {
            set_srcmask_byte(
                &mut self.state.srcmask,
                ext,
                BYTEMASK_U1[self.state.remain as usize - 1],
            );
            self.state.remain = 0;
        }
        let o = self.state.outptr;
        self.src_set_byte(
            0,
            ext,
            (self.state.buf[o] << l) | (self.state.buf[o + 1] >> r),
        );
        self.src_set_byte(
            1,
            ext,
            (self.state.buf[o + 4] << l) | (self.state.buf[o + 5] >> r),
        );
        self.src_set_byte(
            2,
            ext,
            (self.state.buf[o + 8] << l) | (self.state.buf[o + 9] >> r),
        );
        self.src_set_byte(
            3,
            ext,
            (self.state.buf[o + 12] << l) | (self.state.buf[o + 13] >> r),
        );
        self.state.outptr += 1;
    }

    fn sftb_dnl_sub(&mut self, ext: usize) {
        if self.state.dstbit >= 8 {
            self.state.dstbit -= 8;
            set_srcmask_byte(&mut self.state.srcmask, ext, 0);
            return;
        }
        let l = self.state.sft8bitl;
        let r = self.state.sft8bitr;
        if self.state.dstbit > 0 {
            if (u32::from(self.state.dstbit) + self.state.remain) >= 8 {
                set_srcmask_byte(
                    &mut self.state.srcmask,
                    ext,
                    BYTEMASK_D0[self.state.dstbit as usize + 7 * 8],
                );
                self.state.remain -= 8 - u32::from(self.state.dstbit);
                self.state.dstbit = 0;
            } else {
                set_srcmask_byte(
                    &mut self.state.srcmask,
                    ext,
                    BYTEMASK_D0[self.state.dstbit as usize + (self.state.remain as usize - 1) * 8],
                );
                self.state.remain = 0;
                self.state.dstbit = 0;
            }
        } else if self.state.remain >= 8 {
            self.state.remain -= 8;
        } else {
            set_srcmask_byte(
                &mut self.state.srcmask,
                ext,
                BYTEMASK_D1[self.state.remain as usize - 1],
            );
            self.state.remain = 0;
        }
        self.state.outptr = self.state.outptr.wrapping_sub(1);
        let o = self.state.outptr;
        self.src_set_byte(
            0,
            ext,
            (self.state.buf[o + 1] >> l) | (self.state.buf[o] << r),
        );
        self.src_set_byte(
            1,
            ext,
            (self.state.buf[o + 5] >> l) | (self.state.buf[o + 4] << r),
        );
        self.src_set_byte(
            2,
            ext,
            (self.state.buf[o + 9] >> l) | (self.state.buf[o + 8] << r),
        );
        self.src_set_byte(
            3,
            ext,
            (self.state.buf[o + 13] >> l) | (self.state.buf[o + 12] << r),
        );
    }

    fn sftb_dispatch(&mut self, ext: usize) {
        let needed = 8u32.wrapping_sub(u32::from(self.state.dstbit));
        if self.state.stack < needed {
            set_srcmask_byte(&mut self.state.srcmask, ext, 0);
            return;
        }
        self.state.stack -= needed;
        match self.state.func {
            0 => self.sftb_upn_sub(ext),
            1 => self.sftb_dnn_sub(ext),
            2 => self.sftb_upr_sub(ext),
            3 => self.sftb_dnr_sub(ext),
            4 => self.sftb_upl_sub(ext),
            5 => self.sftb_dnl_sub(ext),
            _ => {}
        }
        if self.state.remain == 0 {
            self.recalculate_shift();
        }
    }

    fn sftw_dispatch(&mut self) {
        let needed = 16 - u32::from(self.state.dstbit);
        if self.state.stack < needed {
            self.state.srcmask = 0;
            return;
        }
        self.state.stack -= needed;

        let is_ascending = self.state.func & 1 == 0;
        let (first, second) = if is_ascending { (0, 1) } else { (1, 0) };

        match self.state.func {
            0 => {
                self.sftb_upn_sub(first);
                if self.state.remain > 0 {
                    self.sftb_upn_sub(second);
                    if self.state.remain > 0 {
                        return;
                    }
                } else {
                    set_srcmask_byte(&mut self.state.srcmask, second, 0);
                }
            }
            1 => {
                self.sftb_dnn_sub(first);
                if self.state.remain > 0 {
                    self.sftb_dnn_sub(second);
                    if self.state.remain > 0 {
                        return;
                    }
                } else {
                    set_srcmask_byte(&mut self.state.srcmask, second, 0);
                }
            }
            2 => {
                self.sftb_upr_sub(first);
                if self.state.remain > 0 {
                    self.sftb_upr_sub(second);
                    if self.state.remain > 0 {
                        return;
                    }
                } else {
                    set_srcmask_byte(&mut self.state.srcmask, second, 0);
                }
            }
            3 => {
                self.sftb_dnr_sub(first);
                if self.state.remain > 0 {
                    self.sftb_dnr_sub(second);
                    if self.state.remain > 0 {
                        return;
                    }
                } else {
                    set_srcmask_byte(&mut self.state.srcmask, second, 0);
                }
            }
            4 => {
                self.sftb_upl_sub(first);
                if self.state.remain > 0 {
                    self.sftb_upl_sub(second);
                    if self.state.remain > 0 {
                        return;
                    }
                } else {
                    set_srcmask_byte(&mut self.state.srcmask, second, 0);
                }
            }
            5 => {
                self.sftb_dnl_sub(first);
                if self.state.remain > 0 {
                    self.sftb_dnl_sub(second);
                    if self.state.remain > 0 {
                        return;
                    }
                } else {
                    set_srcmask_byte(&mut self.state.srcmask, second, 0);
                }
            }
            _ => {}
        }
        self.recalculate_shift();
    }

    fn shiftinput_byte(&mut self, ext: usize) {
        if self.state.stack <= 16 {
            if self.state.srcbit >= 8 {
                self.state.srcbit -= 8;
            } else {
                self.state.stack += 8 - u32::from(self.state.srcbit);
                self.state.srcbit = 0;
            }
            if self.state.sft & 0x1000 == 0 {
                self.state.inptr += 1;
            } else {
                self.state.inptr = self.state.inptr.wrapping_sub(1);
            }
        }
        self.state.srcmask = set_srcmask_byte_ret(self.state.srcmask, ext, 0xFF);
        self.sftb_dispatch(ext);
    }

    fn shiftinput_incw(&mut self) {
        if self.state.stack <= 16 {
            self.state.inptr += 2;
            if self.state.srcbit >= 8 {
                self.state.outptr += 1;
            }
            self.state.stack += 16 - u32::from(self.state.srcbit);
            self.state.srcbit = 0;
        }
        self.state.srcmask = 0xFFFF;
        self.sftw_dispatch();
    }

    fn shiftinput_decw(&mut self) {
        if self.state.stack <= 16 {
            self.state.inptr = self.state.inptr.wrapping_sub(2);
            if self.state.srcbit >= 8 {
                self.state.outptr = self.state.outptr.wrapping_sub(1);
            }
            self.state.stack += 16 - u32::from(self.state.srcbit);
            self.state.srcbit = 0;
        }
        self.state.srcmask = 0xFFFF;
        self.sftw_dispatch();
    }

    fn get_pattern(&self) -> [u16; 4] {
        match self.state.fgbg & 0x6000 {
            0x2000 => self.state.bgc,
            0x4000 => self.state.fgc,
            0x6000 => [
                self.state.fgc[0],
                self.state.fgc[1],
                self.state.bgc[2],
                self.state.bgc[3],
            ],
            _ => {
                if (self.state.ope & 0x0300) == 0x0100 {
                    self.src
                } else {
                    self.state.patreg
                }
            }
        }
    }

    fn compute_rop(&self, ope: u8, dst: [u16; 4]) -> [u16; 4] {
        match ope {
            0x00 => [0; 4],
            0x0F => [!self.src[0], !self.src[1], !self.src[2], !self.src[3]],
            0xC0 => [
                self.src[0] & dst[0],
                self.src[1] & dst[1],
                self.src[2] & dst[2],
                self.src[3] & dst[3],
            ],
            0xF0 => self.src,
            0xFC => [
                self.src[0] | (!self.src[0] & dst[0]),
                self.src[1] | (!self.src[1] & dst[1]),
                self.src[2] | (!self.src[2] & dst[2]),
                self.src[3] | (!self.src[3] & dst[3]),
            ],
            0xFF => [0xFFFF; 4],
            _ => {
                // Check if pattern-independent (no-dest or no-pattern).
                let uses_dest = (ope & 0x80 != ope & 0x20)
                    || (ope & 0x40 != ope & 0x10)
                    || (ope & 0x08 != ope & 0x02)
                    || (ope & 0x04 != ope & 0x01);
                let uses_pat = (ope & 0x80 != ope & 0x40)
                    || (ope & 0x20 != ope & 0x10)
                    || (ope & 0x08 != ope & 0x04)
                    || (ope & 0x02 != ope & 0x01);

                if !uses_dest && uses_pat {
                    // ope_nd: no destination dependency
                    let pat = self.get_pattern();
                    let mut result = [0u16; 4];
                    for p in 0..4 {
                        if ope & 0x80 != 0 {
                            result[p] |= pat[p] & self.src[p];
                        }
                        if ope & 0x40 != 0 {
                            result[p] |= !pat[p] & self.src[p];
                        }
                        if ope & 0x08 != 0 {
                            result[p] |= pat[p] & !self.src[p];
                        }
                        if ope & 0x04 != 0 {
                            result[p] |= !pat[p] & !self.src[p];
                        }
                    }
                    result
                } else if uses_dest && !uses_pat {
                    // ope_np: no pattern dependency
                    let mut result = [0u16; 4];
                    for p in 0..4 {
                        if ope & 0x80 != 0 {
                            result[p] |= self.src[p] & dst[p];
                        }
                        if ope & 0x20 != 0 {
                            result[p] |= self.src[p] & !dst[p];
                        }
                        if ope & 0x08 != 0 {
                            result[p] |= !self.src[p] & dst[p];
                        }
                        if ope & 0x02 != 0 {
                            result[p] |= !self.src[p] & !dst[p];
                        }
                    }
                    result
                } else {
                    // ope_xx: full 3-input truth table
                    let pat = self.get_pattern();
                    let mut result = [0u16; 4];
                    for p in 0..4 {
                        if ope & 0x80 != 0 {
                            result[p] |= pat[p] & self.src[p] & dst[p];
                        }
                        if ope & 0x40 != 0 {
                            result[p] |= !pat[p] & self.src[p] & dst[p];
                        }
                        if ope & 0x20 != 0 {
                            result[p] |= pat[p] & self.src[p] & !dst[p];
                        }
                        if ope & 0x10 != 0 {
                            result[p] |= !pat[p] & self.src[p] & !dst[p];
                        }
                        if ope & 0x08 != 0 {
                            result[p] |= pat[p] & !self.src[p] & dst[p];
                        }
                        if ope & 0x04 != 0 {
                            result[p] |= !pat[p] & !self.src[p] & dst[p];
                        }
                        if ope & 0x02 != 0 {
                            result[p] |= pat[p] & !self.src[p] & !dst[p];
                        }
                        if ope & 0x01 != 0 {
                            result[p] |= !pat[p] & !self.src[p] & !dst[p];
                        }
                    }
                    result
                }
            }
        }
    }

    fn ope_byte(&mut self, addr: u32, value: u8, dst: [u16; 4]) -> [u16; 4] {
        self.state.mask2 = self.state.mask;
        match self.state.ope & 0x1800 {
            0x0800 => {
                // Shift + ROP
                if self.state.ope & 0x400 != 0 {
                    let ext = (addr & 1) as usize;
                    let inp = self.state.inptr;
                    self.state.buf[inp] = value;
                    self.state.buf[inp + 4] = value;
                    self.state.buf[inp + 8] = value;
                    self.state.buf[inp + 12] = value;
                    self.shiftinput_byte(ext);
                }
                self.state.mask2 &= self.state.srcmask;
                let rop = (self.state.ope & 0xFF) as u8;
                self.compute_rop(rop, dst)
            }
            0x1000 => {
                // Pattern/color source.
                match self.state.fgbg & 0x6000 {
                    0x2000 => self.state.bgc,
                    0x4000 => self.state.fgc,
                    _ => {
                        if self.state.ope & 0x400 != 0 {
                            let ext = (addr & 1) as usize;
                            let inp = self.state.inptr;
                            self.state.buf[inp] = value;
                            self.state.buf[inp + 4] = value;
                            self.state.buf[inp + 8] = value;
                            self.state.buf[inp + 12] = value;
                            self.shiftinput_byte(ext);
                        }
                        self.state.mask2 &= self.state.srcmask;
                        self.src
                    }
                }
            }
            _ => {
                // CPU data broadcast.
                let w = u16::from(value) | (u16::from(value) << 8);
                [w, w, w, w]
            }
        }
    }

    fn ope_word(&mut self, _addr: u32, value: u16, dst: [u16; 4]) -> [u16; 4] {
        self.state.mask2 = self.state.mask;
        match self.state.ope & 0x1800 {
            0x0800 => {
                // Shift + ROP
                if self.state.ope & 0x400 != 0 {
                    self.write_shift_input_word(value);
                }
                self.state.mask2 &= self.state.srcmask;
                let rop = (self.state.ope & 0xFF) as u8;
                self.compute_rop(rop, dst)
            }
            0x1000 => {
                // Pattern/color with shift.
                self.write_shift_input_word_always(value);
                self.state.mask2 &= self.state.srcmask;
                match self.state.fgbg & 0x6000 {
                    0x2000 => self.state.bgc,
                    0x4000 => self.state.fgc,
                    _ => self.state.patreg,
                }
            }
            _ => {
                // CPU data broadcast with shift.
                let data = [value, value, value, value];
                self.write_shift_input_word_always(value);
                self.state.mask2 &= self.state.srcmask;
                data
            }
        }
    }

    fn write_shift_input_word(&mut self, value: u16) {
        let lo = value as u8;
        let hi = (value >> 8) as u8;
        if self.state.sft & 0x1000 == 0 {
            let inp = self.state.inptr;
            self.state.buf[inp] = lo;
            self.state.buf[inp + 1] = hi;
            self.state.buf[inp + 4] = lo;
            self.state.buf[inp + 5] = hi;
            self.state.buf[inp + 8] = lo;
            self.state.buf[inp + 9] = hi;
            self.state.buf[inp + 12] = lo;
            self.state.buf[inp + 13] = hi;
            self.shiftinput_incw();
        } else {
            let inp = self.state.inptr;
            self.state.buf[inp.wrapping_sub(1)] = lo;
            self.state.buf[inp] = hi;
            self.state.buf[inp + 3] = lo;
            self.state.buf[inp + 4] = hi;
            self.state.buf[inp + 7] = lo;
            self.state.buf[inp + 8] = hi;
            self.state.buf[inp + 11] = lo;
            self.state.buf[inp + 12] = hi;
            self.shiftinput_decw();
        }
    }

    fn write_shift_input_word_always(&mut self, value: u16) {
        let lo = value as u8;
        let hi = (value >> 8) as u8;
        if self.state.sft & 0x1000 == 0 {
            let inp = self.state.inptr;
            self.state.buf[inp] = lo;
            self.state.buf[inp + 1] = hi;
            self.state.buf[inp + 4] = lo;
            self.state.buf[inp + 5] = hi;
            self.state.buf[inp + 8] = lo;
            self.state.buf[inp + 9] = hi;
            self.state.buf[inp + 12] = lo;
            self.state.buf[inp + 13] = hi;
            self.shiftinput_incw();
        } else {
            let inp = self.state.inptr;
            self.state.buf[inp.wrapping_sub(1)] = lo;
            self.state.buf[inp] = hi;
            self.state.buf[inp + 3] = lo;
            self.state.buf[inp + 4] = hi;
            self.state.buf[inp + 7] = lo;
            self.state.buf[inp + 8] = hi;
            self.state.buf[inp + 11] = lo;
            self.state.buf[inp + 12] = hi;
            self.shiftinput_decw();
        }
    }

    fn compare_color_pat(&self) -> [u16; 4] {
        match (self.state.fgbg >> 13) & 3 {
            1 => self.state.bgc,
            2 => self.state.fgc,
            _ => self.state.patreg,
        }
    }

    /// EGC byte read. Returns the byte to give to the CPU.
    /// `vram` = [B, R, G, E] plane bytes at the accessed offset.
    pub fn read_byte(&mut self, addr: u32, vram: [u8; 4]) -> u8 {
        let ext = (addr & 1) as usize;

        // Store last VRAM read
        set_lastvram_byte(&mut self.state.lastvram, 0, ext, vram[0]);
        set_lastvram_byte(&mut self.state.lastvram, 1, ext, vram[1]);
        set_lastvram_byte(&mut self.state.lastvram, 2, ext, vram[2]);
        set_lastvram_byte(&mut self.state.lastvram, 3, ext, vram[3]);

        // Shift input from VRAM (when read-source = VRAM, bit 10 clear).
        if self.state.ope & 0x400 == 0 {
            let inp = self.state.inptr;
            self.state.buf[inp] = vram[0];
            self.state.buf[inp + 4] = vram[1];
            self.state.buf[inp + 8] = vram[2];
            self.state.buf[inp + 12] = vram[3];
            self.shiftinput_byte(ext);
        }

        // Load pattern register from VRAM (reg load mode = 01).
        if (self.state.ope & 0x0300) == 0x0100 {
            set_patreg_byte(&mut self.state.patreg, 0, ext, vram[0]);
            set_patreg_byte(&mut self.state.patreg, 1, ext, vram[1]);
            set_patreg_byte(&mut self.state.patreg, 2, ext, vram[2]);
            set_patreg_byte(&mut self.state.patreg, 3, ext, vram[3]);
        }

        // Return value depends on compare-read mode.
        if self.state.ope & 0x2000 == 0 {
            let pl = ((self.state.fgbg >> 8) & 3) as usize;
            if self.state.ope & 0x400 == 0 {
                get_src_byte(&self.src, pl, ext)
            } else {
                vram[pl]
            }
        } else {
            // Compare-read: XOR each plane with the comparison color, AND together,
            // return complement. Gives per-pixel match results.
            let color = self.compare_color_pat();
            let mut result = 0xFFu8;
            for (v, c) in vram.iter().zip(&color) {
                result &= !(v ^ (*c as u8));
            }
            result
        }
    }

    /// EGC word read. Returns the word to give to the CPU.
    /// `vram` = [B, R, G, E] plane words at the accessed word-aligned offset.
    pub fn read_word(&mut self, _addr: u32, vram: [u16; 4]) -> u16 {
        self.state.lastvram = vram;

        // Shift input from VRAM
        if self.state.ope & 0x400 == 0 {
            if self.state.sft & 0x1000 == 0 {
                let inp = self.state.inptr;
                let [b, r, g, e] = vram;
                self.state.buf[inp] = b as u8;
                self.state.buf[inp + 1] = (b >> 8) as u8;
                self.state.buf[inp + 4] = r as u8;
                self.state.buf[inp + 5] = (r >> 8) as u8;
                self.state.buf[inp + 8] = g as u8;
                self.state.buf[inp + 9] = (g >> 8) as u8;
                self.state.buf[inp + 12] = e as u8;
                self.state.buf[inp + 13] = (e >> 8) as u8;
                self.shiftinput_incw();
            } else {
                let inp = self.state.inptr;
                let [b, r, g, e] = vram;
                self.state.buf[inp.wrapping_sub(1)] = b as u8;
                self.state.buf[inp] = (b >> 8) as u8;
                self.state.buf[inp + 3] = r as u8;
                self.state.buf[inp + 4] = (r >> 8) as u8;
                self.state.buf[inp + 7] = g as u8;
                self.state.buf[inp + 8] = (g >> 8) as u8;
                self.state.buf[inp + 11] = e as u8;
                self.state.buf[inp + 12] = (e >> 8) as u8;
                self.shiftinput_decw();
            }
        }

        // Load pattern register from VRAM (reg load mode = 01).
        if (self.state.ope & 0x0300) == 0x0100 {
            self.state.patreg = vram;
        }

        // Return value
        if self.state.ope & 0x2000 == 0 {
            let pl = ((self.state.fgbg >> 8) & 3) as usize;
            if self.state.ope & 0x400 == 0 {
                self.src[pl]
            } else {
                vram[pl]
            }
        } else {
            // Compare-read: XOR each plane with the comparison color, AND together,
            // return complement. Gives per-pixel match results.
            let color = self.compare_color_pat();
            let mut result = 0xFFFFu16;
            for (v, c) in vram.iter().zip(&color) {
                result &= !(v ^ c);
            }
            result
        }
    }

    /// EGC byte write. Returns `(data_per_plane, mask)` for the bus to apply.
    /// `vram` = [B, R, G, E] plane bytes at the accessed offset (current VRAM content).
    pub fn write_byte(&mut self, addr: u32, value: u8, vram: [u8; 4]) -> ([u8; 4], u8) {
        let ext = (addr & 1) as usize;

        // Load pattern register from VRAM (reg load mode = 10) BEFORE computing.
        if (self.state.ope & 0x0300) == 0x0200 {
            set_patreg_byte(&mut self.state.patreg, 0, ext, vram[0]);
            set_patreg_byte(&mut self.state.patreg, 1, ext, vram[1]);
            set_patreg_byte(&mut self.state.patreg, 2, ext, vram[2]);
            set_patreg_byte(&mut self.state.patreg, 3, ext, vram[3]);
        }

        // Build word-sized dst from current VRAM (ope functions work on words).
        let dst = [
            u16::from(vram[0]) | (u16::from(vram[0]) << 8),
            u16::from(vram[1]) | (u16::from(vram[1]) << 8),
            u16::from(vram[2]) | (u16::from(vram[2]) << 8),
            u16::from(vram[3]) | (u16::from(vram[3]) << 8),
        ];

        let data = self.ope_byte(addr, value, dst);
        let mask_byte = get_mask2_byte(self.state.mask2, ext);

        (
            [
                get_word_byte(&data, 0, ext),
                get_word_byte(&data, 1, ext),
                get_word_byte(&data, 2, ext),
                get_word_byte(&data, 3, ext),
            ],
            mask_byte,
        )
    }

    /// EGC word write. Returns `(data_per_plane, mask)` for the bus to apply.
    /// `vram` = [B, R, G, E] plane words at the accessed word-aligned offset.
    pub fn write_word(&mut self, addr: u32, value: u16, vram: [u16; 4]) -> ([u16; 4], u16) {
        // Load pattern register from VRAM (reg load mode = 10) BEFORE computing.
        if (self.state.ope & 0x0300) == 0x0200 {
            self.state.patreg = vram;
        }

        let _ = addr;
        let data = self.ope_word(addr, value, vram);
        (data, self.state.mask2)
    }

    /// Returns true if the shift direction is descending (bit 12 of sft register).
    pub fn is_descending(&self) -> bool {
        self.state.sft & 0x1000 != 0
    }

    /// Returns the plane write enable mask (active-low, bits 0-3 of access register).
    pub fn plane_write_enabled(&self, plane: usize) -> bool {
        self.state.access & (1 << plane) == 0
    }

    fn src_set_byte(&mut self, plane: usize, ext: usize, val: u8) {
        if ext == 0 {
            self.src[plane] = (self.src[plane] & 0xFF00) | u16::from(val);
        } else {
            self.src[plane] = (self.src[plane] & 0x00FF) | (u16::from(val) << 8);
        }
    }
}

fn set_srcmask_byte(srcmask: &mut u16, ext: usize, val: u8) {
    if ext == 0 {
        *srcmask = (*srcmask & 0xFF00) | u16::from(val);
    } else {
        *srcmask = (*srcmask & 0x00FF) | (u16::from(val) << 8);
    }
}

fn set_srcmask_byte_ret(srcmask: u16, ext: usize, val: u8) -> u16 {
    if ext == 0 {
        (srcmask & 0xFF00) | u16::from(val)
    } else {
        (srcmask & 0x00FF) | (u16::from(val) << 8)
    }
}

fn set_lastvram_byte(lastvram: &mut [u16; 4], plane: usize, ext: usize, val: u8) {
    if ext == 0 {
        lastvram[plane] = (lastvram[plane] & 0xFF00) | u16::from(val);
    } else {
        lastvram[plane] = (lastvram[plane] & 0x00FF) | (u16::from(val) << 8);
    }
}

fn set_patreg_byte(patreg: &mut [u16; 4], plane: usize, ext: usize, val: u8) {
    if ext == 0 {
        patreg[plane] = (patreg[plane] & 0xFF00) | u16::from(val);
    } else {
        patreg[plane] = (patreg[plane] & 0x00FF) | (u16::from(val) << 8);
    }
}

fn get_src_byte(src: &[u16; 4], plane: usize, ext: usize) -> u8 {
    if ext == 0 {
        src[plane] as u8
    } else {
        (src[plane] >> 8) as u8
    }
}

fn get_mask2_byte(mask2: u16, ext: usize) -> u8 {
    if ext == 0 {
        mask2 as u8
    } else {
        (mask2 >> 8) as u8
    }
}

fn get_word_byte(data: &[u16; 4], plane: usize, ext: usize) -> u8 {
    if ext == 0 {
        data[plane] as u8
    } else {
        (data[plane] >> 8) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_defaults() {
        let egc = Egc::new();
        assert_eq!(egc.state.access, 0xFFF0);
        assert_eq!(egc.state.fgbg, 0x00FF);
        assert_eq!(egc.state.ope, 0);
        assert_eq!(egc.state.fg, 0);
        assert_eq!(egc.state.mask, 0xFFFF);
        assert_eq!(egc.state.bg, 0);
        assert_eq!(egc.state.sft, 0);
        assert_eq!(egc.state.leng, 0x000F);
        assert_eq!(egc.state.remain, 16);
        assert_eq!(egc.state.srcmask, 0xFFFF);
        assert_eq!(egc.state.fgc, [0; 4]);
        assert_eq!(egc.state.bgc, [0; 4]);
        assert_eq!(egc.state.func, 0);
    }

    #[test]
    fn reset_restores_defaults() {
        let mut egc = Egc::new();
        egc.write_register_word(0x00, 0x1234);
        egc.write_register_word(0x04, 0x5678);
        egc.write_register_word(0x06, 0x0005);
        egc.write_register_word(0x08, 0xAAAA);

        egc.reset();

        assert_eq!(egc.state.access, 0xFFF0);
        assert_eq!(egc.state.ope, 0);
        assert_eq!(egc.state.fg, 0);
        assert_eq!(egc.state.mask, 0xFFFF);
        assert_eq!(egc.state.fgc, [0; 4]);
        assert_eq!(egc.state.srcmask, 0xFFFF);
    }

    #[test]
    fn write_register_word() {
        let mut egc = Egc::new();
        egc.write_register_word(0x00, 0x1234);
        assert_eq!(egc.state.access, 0x1234);
    }

    #[test]
    fn write_register_byte() {
        let mut egc = Egc::new();
        egc.write_register_byte(0x00, 0x34);
        assert_eq!(egc.state.access & 0x00FF, 0x0034);

        egc.write_register_byte(0x01, 0x12);
        assert_eq!(egc.state.access, 0x1234);
    }

    #[test]
    fn fg_color_expansion() {
        let mut egc = Egc::new();
        egc.write_register_word(0x06, 5);
        assert_eq!(egc.state.fgc, [0xFFFF, 0x0000, 0xFFFF, 0x0000]);
    }

    #[test]
    fn bg_color_expansion() {
        let mut egc = Egc::new();
        egc.write_register_word(0x0A, 0x0A);
        assert_eq!(egc.state.bgc, [0x0000, 0xFFFF, 0x0000, 0xFFFF]);
    }

    #[test]
    fn fg_color_expansion_byte_write() {
        let mut egc = Egc::new();
        egc.write_register_byte(0x06, 5);
        assert_eq!(egc.state.fgc, [0xFFFF, 0x0000, 0xFFFF, 0x0000]);
    }

    #[test]
    fn mask_register_gating_allowed() {
        let mut egc = Egc::new();
        egc.write_register_word(0x08, 0xAAAA);
        assert_eq!(egc.state.mask, 0xAAAA);
    }

    #[test]
    fn mask_register_gating_blocked() {
        let mut egc = Egc::new();

        egc.write_register_word(0x02, 0x2000);
        egc.write_register_word(0x08, 0xAAAA);
        assert_eq!(egc.state.mask, 0xFFFF);

        egc.write_register_word(0x02, 0x4000);
        egc.write_register_word(0x08, 0x5555);
        assert_eq!(egc.state.mask, 0xFFFF);

        egc.write_register_word(0x02, 0x6000);
        egc.write_register_word(0x08, 0x1234);
        assert_eq!(egc.state.mask, 0xFFFF);
    }

    #[test]
    fn plane_write_enabled() {
        let mut egc = Egc::new();

        assert!(egc.plane_write_enabled(0));
        assert!(egc.plane_write_enabled(1));
        assert!(egc.plane_write_enabled(2));
        assert!(egc.plane_write_enabled(3));

        egc.write_register_word(0x00, 0xFFF5);
        assert!(!egc.plane_write_enabled(0));
        assert!(egc.plane_write_enabled(1));
        assert!(!egc.plane_write_enabled(2));
        assert!(egc.plane_write_enabled(3));

        egc.write_register_word(0x00, 0xFFFF);
        assert!(!egc.plane_write_enabled(0));
        assert!(!egc.plane_write_enabled(1));
        assert!(!egc.plane_write_enabled(2));
        assert!(!egc.plane_write_enabled(3));
    }

    #[test]
    fn shift_direction() {
        let mut egc = Egc::new();

        egc.write_register_word(0x0C, 0x0000);
        assert!(!egc.is_descending());

        egc.write_register_word(0x0C, 0x1000);
        assert!(egc.is_descending());
    }

    #[test]
    fn sft_write_resets_srcmask() {
        let mut egc = Egc::new();
        egc.state.srcmask = 0x0000;
        egc.write_register_word(0x0C, 0x0000);
        assert_eq!(egc.state.srcmask, 0xFFFF);
    }

    #[test]
    fn leng_write_sets_remain() {
        let mut egc = Egc::new();
        egc.write_register_word(0x0E, 0x001F);
        assert_eq!(egc.state.remain, 32);
    }

    #[test]
    fn recalculate_shift_func_selection() {
        let mut egc = Egc::new();

        // Ascending, no shift (src8 == dst8): func=0
        egc.write_register_word(0x0C, 0x0033);
        assert_eq!(egc.state.func, 0);

        // Descending, no shift: func=1
        egc.write_register_word(0x0C, 0x1033);
        assert_eq!(egc.state.func, 1);

        // Ascending, src8(2) < dst8(5): func=2 (right shift)
        egc.write_register_word(0x0C, 0x0052);
        assert_eq!(egc.state.func, 2);
        assert_eq!(egc.state.sft8bitr, 3);
        assert_eq!(egc.state.sft8bitl, 5);

        // Ascending, src8(5) > dst8(2): func=4 (left shift)
        egc.write_register_word(0x0C, 0x0025);
        assert_eq!(egc.state.func, 4);
        assert_eq!(egc.state.sft8bitl, 3);
        assert_eq!(egc.state.sft8bitr, 5);

        // Descending, src8(2) < dst8(5): func=3
        egc.write_register_word(0x0C, 0x1052);
        assert_eq!(egc.state.func, 3);

        // Descending, src8(5) > dst8(2): func=5
        egc.write_register_word(0x0C, 0x1025);
        assert_eq!(egc.state.func, 5);
    }

    #[test]
    fn cpu_broadcast_write_word() {
        let mut egc = Egc::new();
        egc.write_register_word(0x04, 0x0000); // ope=0 -> CPU broadcast
        let vram = [0u16; 4];
        let (data, mask) = egc.write_word(0xA8000, 0xBEEF, vram);
        assert_eq!(data, [0xBEEF, 0xBEEF, 0xBEEF, 0xBEEF]);
        assert_eq!(mask, 0xFFFF);
    }

    #[test]
    fn fgc_color_fill_write_word() {
        let mut egc = Egc::new();
        egc.write_register_word(0x06, 5); // fg=5
        egc.write_register_word(0x02, 0x4000); // fgbg: FGC source
        egc.write_register_word(0x04, 0x1000); // ope: pattern source
        let vram = [0u16; 4];
        let (data, _mask) = egc.write_word(0xA8000, 0x0000, vram);
        assert_eq!(data, [0xFFFF, 0x0000, 0xFFFF, 0x0000]);
    }

    #[test]
    fn bgc_color_fill_write_word() {
        let mut egc = Egc::new();
        egc.write_register_word(0x0A, 0x0A); // bg=0xA
        egc.write_register_word(0x02, 0x2000); // fgbg: BGC source
        egc.write_register_word(0x04, 0x1000); // ope: pattern source
        let vram = [0u16; 4];
        let (data, _mask) = egc.write_word(0xA8000, 0x0000, vram);
        assert_eq!(data, [0x0000, 0xFFFF, 0x0000, 0xFFFF]);
    }

    #[test]
    fn rop_clear_all() {
        let mut egc = Egc::new();
        // ope: shift+ROP, CPU source (bit10=1), ROP=0x00
        egc.write_register_word(0x04, 0x0C00);
        let vram = [0x5555u16; 4];
        let (data, _mask) = egc.write_word(0xA8000, 0xAAAA, vram);
        assert_eq!(data, [0; 4]);
    }

    #[test]
    fn rop_fill_all() {
        let mut egc = Egc::new();
        // ope: shift+ROP, CPU source, ROP=0xFF
        egc.write_register_word(0x04, 0x0CFF);
        let vram = [0u16; 4];
        let (data, _mask) = egc.write_word(0xA8000, 0x0000, vram);
        assert_eq!(data, [0xFFFF; 4]);
    }

    #[test]
    fn rop_source_copy() {
        let mut egc = Egc::new();
        // ope: shift+ROP, CPU source (bit10=1), ROP=0xF0 (Source)
        egc.write_register_word(0x04, 0x0CF0);
        let vram = [0x5555u16; 4];
        let (data, _mask) = egc.write_word(0xA8000, 0xAAAA, vram);
        assert_eq!(data, [0xAAAA, 0xAAAA, 0xAAAA, 0xAAAA]);
    }

    #[test]
    fn rop_invert_source() {
        let mut egc = Egc::new();
        // ope: shift+ROP, CPU source, ROP=0x0F (~Source)
        egc.write_register_word(0x04, 0x0C0F);
        let vram = [0u16; 4];
        let (data, _mask) = egc.write_word(0xA8000, 0xAAAA, vram);
        assert_eq!(data, [0x5555, 0x5555, 0x5555, 0x5555]);
    }

    #[test]
    fn rop_source_and_dest() {
        let mut egc = Egc::new();
        // ope: shift+ROP, CPU source, ROP=0xC0 (S & D)
        egc.write_register_word(0x04, 0x0CC0);
        let vram = [0xFF00, 0x00FF, 0xF0F0, 0x0F0F];
        let (data, _mask) = egc.write_word(0xA8000, 0xAAAA, vram);
        assert_eq!(data, [0xAA00, 0x00AA, 0xA0A0, 0x0A0A]);
    }

    #[test]
    fn write_byte_cpu_broadcast() {
        let mut egc = Egc::new();
        egc.write_register_word(0x04, 0x0000); // ope=0 -> CPU broadcast
        let vram = [0u8; 4];
        let (data, mask) = egc.write_byte(0xA8000, 0xAB, vram);
        assert_eq!(data, [0xAB, 0xAB, 0xAB, 0xAB]);
        assert_eq!(mask, 0xFF);
    }

    #[test]
    fn sftb_dispatch_no_panic_when_dstbit_exceeds_8() {
        let mut egc = Egc::new();
        // Set up a valid state with dstbit > 8 (range is 0-15).
        egc.state.dstbit = 12;
        egc.state.stack = 0;
        // sftb_dispatch should return early without panicking.
        egc.sftb_dispatch(0);
    }

    #[test]
    fn compare_read_byte_matches_foreground_color() {
        let mut egc = Egc::new();
        // Set foreground color to 5 (planes 0 and 2 set).
        egc.write_register_word(0x06, 5);
        // Select FGC as compare source (fgbg bits 14-13 = 2).
        egc.write_register_word(0x02, 0x4000);
        // Enable compare-read mode (bit 13 of ope).
        egc.write_register_word(0x04, 0x2000);

        // VRAM where pixel matches fg color 5: B=0xFF, R=0x00, G=0xFF, E=0x00
        let vram_match: [u8; 4] = [0xFF, 0x00, 0xFF, 0x00];
        let result = egc.read_byte(0xA8000, vram_match);
        assert_eq!(result, 0xFF, "all pixels match fg color 5");

        // VRAM where no pixel matches: B=0x00, R=0xFF, G=0x00, E=0xFF
        let vram_no_match: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF];
        let result = egc.read_byte(0xA8000, vram_no_match);
        assert_eq!(result, 0x00, "no pixels match fg color 5");

        // Partial match: only some bits match across all planes.
        // fg=5 -> fgc=[0xFF,0x00,0xFF,0x00]
        // B=0xF0, R=0x00, G=0xFF, E=0x00
        // XOR: B=0x0F, R=0x00, G=0x00, E=0x00
        // NOT XOR: B=0xF0, R=0xFF, G=0xFF, E=0xFF
        // AND all: 0xF0
        let vram_partial: [u8; 4] = [0xF0, 0x00, 0xFF, 0x00];
        let result = egc.read_byte(0xA8000, vram_partial);
        assert_eq!(result, 0xF0, "upper nibble matches fg color");
    }

    #[test]
    fn compare_read_word_matches_foreground_color() {
        let mut egc = Egc::new();
        // Set foreground color to 3 (planes 0 and 1 set).
        egc.write_register_word(0x06, 3);
        // Select FGC as compare source (fgbg bits 14-13 = 2).
        egc.write_register_word(0x02, 0x4000);
        // Enable compare-read mode (bit 13 of ope).
        egc.write_register_word(0x04, 0x2000);

        // VRAM matching color 3: B=0xFFFF, R=0xFFFF, G=0x0000, E=0x0000
        let vram_match: [u16; 4] = [0xFFFF, 0xFFFF, 0x0000, 0x0000];
        let result = egc.read_word(0xA8000, vram_match);
        assert_eq!(result, 0xFFFF, "all pixels match fg color 3");

        // No match.
        let vram_no_match: [u16; 4] = [0x0000, 0x0000, 0xFFFF, 0xFFFF];
        let result = egc.read_word(0xA8000, vram_no_match);
        assert_eq!(result, 0x0000, "no pixels match fg color 3");
    }
}
