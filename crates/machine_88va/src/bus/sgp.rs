//! PC-88VA2 Super Graphic Processor (SGP).
//!
//! The SGP is a blitter coprocessor driven by a command list in memory. It owns
//! a micro-program counter and a small set of execution states that! step BITBLT,
//! PATBLT, CLS and LINE one word or one pixel at a time. The engine runs the whole
//! command list when triggered; the per-operation cycle costs are accumulated so
//! the bus can schedule a completion event that clears busy and raises the SGP
//! interrupt at the right cycle.
//!
//! The SGP addresses its own wide space: main RAM (`0x000000`), the kanji/font
//! ROM (`0x100000`), text VRAM (`0x180000`) and graphics VRAM (`0x200000`). All
//! memory access goes through [`Pc88VaMemory::sgp_read_word`] /
//! [`Pc88VaMemory::sgp_write_word`].

use super::Pc88VaBus;
use crate::{memory::Pc88VaMemory, scheduler::Event88Va};

pub(crate) const SGP_INTF: u8 = 0x04;
const SGP_ABORT: u8 = 0x02;
pub(crate) const SGP_BUSY: u8 = 0x01;

const BLTMODE_SF: u16 = 0x1000;
const BLTMODE_VD: u16 = 0x0800;
const BLTMODE_HD: u16 = 0x0400;
const BLTMODE_TP: u16 = 0x0300;
const BLTMODE_OP: u16 = 0x000F;
const BLTMODE_LINE_VD: u16 = 0x0400;
const BLTMODE_LINE_HD: u16 = 0x0800;

/// Pixels packed into one 16-bit word, indexed by screen mode.
const DOTCOUNTMAX: [i32; 4] = [0x10, 0x04, 0x02, 0x01];
/// Bits per pixel, indexed by screen mode.
const BPP: [i32; 4] = [1, 4, 8, 16];

/// Upper bound on micro-steps per run, guarding against a command list with no
/// END terminator. A full 256 KiB blit is well under this.
const MAX_STEPS: u64 = 16_000_000;

/// Current execution state of the engine's micro-program.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SgpFunc {
    #[default]
    FetchCommand,
    ExecBitblt,
    ExecBitbltHd,
    ExecCls,
    ExecLineX,
    ExecLineY,
}

/// A source or destination block descriptor.
#[derive(Clone, Copy, Default)]
struct SgpBlock {
    scrnmode: usize,
    dot: i32,
    width: u16,
    height: u16,
    fbw: i16,
    address: u32,
    lineaddress: u32,
    nextaddress: u32,
    dotcount: i32,
    buf: u16,
    xcount: u16,
    ycount: u16,
}

impl SgpBlock {
    fn init(&mut self) {
        self.lineaddress = self.address;
        self.nextaddress = self.address;
        self.ycount = self.height;
    }

    fn read_word(&mut self, mem: &Pc88VaMemory) {
        self.buf = mem.sgp_read_word(self.nextaddress).swap_bytes();
        self.dotcount = DOTCOUNTMAX[self.scrnmode];
    }
}

/// The SGP engine state.
#[derive(Default)]
pub(crate) struct SgpState {
    initialpc: u32,
    pc: u32,
    workmem: u32,
    pub(crate) ctrl: u8,
    pub(crate) busy: u8,
    pub(crate) intreq: bool,
    color: u16,
    func: SgpFunc,
    src: SgpBlock,
    dest: SgpBlock,
    newval: u16,
    newvalmask: u16,
    bltmode: u16,
    clsaddr: u32,
    clscount: u32,
    lineslopedenominator: u32,
    lineslopenumerator: u32,
    lineslopecount: u32,
    /// Accumulated cycle cost of the current run, used to time completion.
    cycles: u64,
}

impl SgpState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Writes one byte of the 32-bit command-list start address (ports
    /// `0x500-0x503`). Bit 0 is forced clear.
    pub(crate) fn write_address_byte(&mut self, port: u16, value: u8) {
        let bit = u32::from(port - 0x500) * 8;
        let mask = 0x0000_00FFu32 << bit;
        self.initialpc = ((self.initialpc & !mask) | (u32::from(value) << bit)) & 0xFFFF_FFFE;
    }

    /// Writes the execution-trigger port (`0x506`). Returns true when this write
    /// transitions the engine from idle to busy and a run should start.
    pub(crate) fn write_trigger(&mut self, value: u8) -> bool {
        let value = value & SGP_BUSY;
        let start = (self.busy & SGP_BUSY) == 0 && (value & SGP_BUSY) != 0;
        self.busy = value;
        start
    }

    fn spend(&mut self, cycles: u64) {
        self.cycles += cycles;
    }

    /// Runs the command list to completion, returning the accumulated cycle cost.
    pub(crate) fn execute(&mut self, mem: &mut Pc88VaMemory) -> u64 {
        self.cycles = 0;
        self.func = SgpFunc::FetchCommand;
        self.pc = self.initialpc;
        let mut steps: u64 = 0;
        loop {
            steps += 1;
            if steps > MAX_STEPS {
                break;
            }
            match self.func {
                SgpFunc::FetchCommand => {
                    if !self.fetch_command(mem) {
                        break;
                    }
                }
                SgpFunc::ExecBitblt => self.exec_bitblt(mem),
                SgpFunc::ExecBitbltHd => self.exec_bitblt_hd(mem),
                SgpFunc::ExecCls => self.exec_cls(mem),
                SgpFunc::ExecLineX => self.exec_line_x(mem),
                SgpFunc::ExecLineY => self.exec_line_y(mem),
            }
        }
        self.cycles
    }

    /// Fetches and dispatches the next command. Returns false on END.
    fn fetch_command(&mut self, mem: &mut Pc88VaMemory) -> bool {
        let cmd = mem.sgp_read_word(self.pc);
        self.pc = self.pc.wrapping_add(2);
        match cmd {
            0x01 => return false,
            0x02 => self.cmd_nop(),
            0x03 => self.cmd_set_work(mem),
            0x04 => self.cmd_set_source(mem),
            0x05 => self.cmd_set_destination(mem),
            0x06 => self.cmd_set_color(mem),
            0x07 => self.cmd_bitblt(mem),
            0x08 => self.cmd_patblt(mem),
            0x09 => self.cmd_line(mem),
            0x0A => self.cmd_cls(mem),
            0x0B => self.cmd_scan(mem, false),
            0x0C => self.cmd_scan(mem, true),
            _ => {}
        }
        true
    }

    fn cmd_nop(&mut self) {
        self.spend(5 * 2);
    }

    fn cmd_set_work(&mut self, mem: &Pc88VaMemory) {
        self.workmem = u32::from(mem.sgp_read_word(self.pc) & 0xFFFE)
            | (u32::from(mem.sgp_read_word(self.pc.wrapping_add(2))) << 16);
        self.pc = self.pc.wrapping_add(4);
        self.spend(23 * 2);
    }

    fn cmd_set_source(&mut self, mem: &Pc88VaMemory) {
        fetch_block(mem, self.pc, &mut self.src);
        self.pc = self.pc.wrapping_add(12);
        self.spend(106 * 2);
    }

    fn cmd_set_destination(&mut self, mem: &Pc88VaMemory) {
        fetch_block(mem, self.pc, &mut self.dest);
        self.pc = self.pc.wrapping_add(12);
        self.spend(106 * 2);
    }

    fn cmd_set_color(&mut self, mem: &Pc88VaMemory) {
        self.color = mem.sgp_read_word(self.pc);
        self.pc = self.pc.wrapping_add(2);
        self.spend(10 * 2);
    }

    fn cmd_bitblt(&mut self, mem: &Pc88VaMemory) {
        self.bltmode = mem.sgp_read_word(self.pc);
        self.pc = self.pc.wrapping_add(2);
        self.spend(338 * 2);

        // BITBLT ignores the destination size and copies the source dimensions.
        self.dest.width = self.src.width;
        self.dest.height = self.src.height;

        self.src.init();
        self.dest.init();
        if self.bltmode & BLTMODE_HD != 0 {
            self.init_src_line_hd(mem);
            self.init_dest_line_hd(mem);
            self.func = SgpFunc::ExecBitbltHd;
        } else {
            self.init_src_line(mem);
            self.init_dest_line();
            self.func = SgpFunc::ExecBitblt;
        }
    }

    fn cmd_patblt(&mut self, mem: &Pc88VaMemory) {
        self.bltmode = mem.sgp_read_word(self.pc);
        self.pc = self.pc.wrapping_add(2);
        self.spend(338 * 2);

        self.src.init();
        self.dest.init();
        if self.bltmode & BLTMODE_HD != 0 {
            self.init_src_line_hd(mem);
            self.init_dest_line_hd(mem);
            self.func = SgpFunc::ExecBitbltHd;
        } else {
            self.init_src_line(mem);
            self.init_dest_line();
            self.func = SgpFunc::ExecBitblt;
        }
    }

    fn cmd_line(&mut self, mem: &Pc88VaMemory) {
        self.bltmode = mem.sgp_read_word(self.pc);
        self.pc = self.pc.wrapping_add(2);
        fetch_block(mem, self.pc, &mut self.dest);
        self.pc = self.pc.wrapping_add(12);
        self.spend(109);

        if self.dest.width < self.dest.height {
            self.func = SgpFunc::ExecLineY;
            self.dest.ycount = self.dest.height;
            let denom = u32::from(self.dest.height.wrapping_sub(1));
            self.lineslopedenominator = denom;
            self.lineslopenumerator = if self.dest.width == 0 {
                0
            } else {
                u32::from(self.dest.width - 1)
            };
            self.lineslopecount = denom / 2;
            self.dest.dotcount = self.dest.dot;
            self.dest.nextaddress = self.dest.address;
        } else {
            self.func = SgpFunc::ExecLineX;
            self.dest.xcount = self.dest.width;
            let denom = u32::from(self.dest.width.wrapping_sub(1));
            self.lineslopedenominator = denom;
            self.lineslopenumerator = if self.dest.height == 0 {
                0
            } else {
                u32::from(self.dest.height - 1)
            };
            self.lineslopecount = if denom == 0 { 0 } else { (denom - 1) / 2 };
            if self.bltmode & BLTMODE_LINE_HD != 0 {
                self.dest.dotcount = self.dest.dot + 1;
            } else {
                self.dest.dotcount = DOTCOUNTMAX[self.dest.scrnmode] - self.dest.dot;
            }
            self.dest.nextaddress = self.dest.address;
            self.newval = 0;
            self.newvalmask = 0;
        }
    }

    fn cmd_cls(&mut self, mem: &Pc88VaMemory) {
        self.clsaddr = (u32::from(mem.sgp_read_word(self.pc))
            | (u32::from(mem.sgp_read_word(self.pc.wrapping_add(2))) << 16))
            & 0xFFFF_FFFE;
        self.clscount = u32::from(mem.sgp_read_word(self.pc.wrapping_add(4)))
            | (u32::from(mem.sgp_read_word(self.pc.wrapping_add(6))) << 16);
        self.pc = self.pc.wrapping_add(8);
        self.spend(26 * 2);
        self.func = SgpFunc::ExecCls;
    }

    /// SCAN RIGHT (`0x0b`) / SCAN LEFT (`0x0c`).
    ///
    /// Paint helpers: starting from the destination block's start pixel, scan one
    /// row in the given direction for the first pixel equal to the SET COLOR value,
    /// so a following PATBLT can fill the span up to the boundary. The scanned pixel
    /// count becomes the destination width (0 if the start pixel is already the
    /// color); when the color is not found, nothing is updated.
    ///
    /// SCAN RIGHT updates only the width. SCAN LEFT also repositions the destination
    /// start address and dot to the left edge of the scanned region, so the
    /// subsequent rightward PATBLT fills the interior span between the left boundary
    /// and the original start pixel.
    fn cmd_scan(&mut self, mem: &Pc88VaMemory, left: bool) {
        self.spend(10 * 2);
        let scrnmode = self.dest.scrnmode;
        let bpp = BPP[scrnmode];
        let dotcountmax = DOTCOUNTMAX[scrnmode];
        let pixel_mask: u32 = if bpp >= 16 { 0xFFFF } else { (1u32 << bpp) - 1 };
        let target = u32::from(self.color) & pixel_mask;
        let direction: i32 = if left { -1 } else { 1 };
        for x in 0..self.dest.width {
            let pixel_index = self.dest.dot + direction * i32::from(x);
            let word_index = pixel_index.div_euclid(dotcountmax);
            let within = pixel_index.rem_euclid(dotcountmax);
            let address = self.dest.address.wrapping_add_signed(word_index * 2);
            let raw = mem.sgp_read_word(address).swap_bytes();
            let shift = ((dotcountmax - 1 - within) * bpp) as u32;
            let pixel = (u32::from(raw) >> shift) & pixel_mask;
            if pixel == target {
                if left {
                    let left_edge = self.dest.dot - i32::from(x) + 1;
                    let edge_word_index = left_edge.div_euclid(dotcountmax);
                    self.dest.dot = left_edge.rem_euclid(dotcountmax);
                    self.dest.address = self.dest.address.wrapping_add_signed(edge_word_index * 2);
                }
                self.dest.width = x;
                return;
            }
        }
    }

    fn init_src_line(&mut self, mem: &Pc88VaMemory) {
        let dot = if self.bltmode & BLTMODE_SF != 0 {
            if self.dest.dot < self.src.dot {
                self.src.nextaddress = self.src.nextaddress.wrapping_add(2);
            }
            self.dest.dot
        } else {
            self.src.dot
        };
        self.src.read_word(mem);
        self.src.nextaddress = self.src.nextaddress.wrapping_add(2);
        self.src.dotcount -= dot;
        let shift =
            ((DOTCOUNTMAX[self.src.scrnmode] - self.src.dotcount) * BPP[self.src.scrnmode]) as u32;
        self.src.buf = ((u32::from(self.src.buf)) << shift) as u16;
        self.src.xcount = self.src.width;
    }

    fn init_dest_line(&mut self) {
        self.newval = 0;
        self.newvalmask = 0;
        self.dest.dotcount = DOTCOUNTMAX[self.dest.scrnmode] - self.dest.dot;
        self.dest.xcount = self.dest.width;
    }

    fn init_src_line_hd(&mut self, mem: &Pc88VaMemory) {
        let dot = if self.bltmode & BLTMODE_SF != 0 {
            if self.dest.dot > self.src.dot {
                self.src.nextaddress = self.src.nextaddress.wrapping_sub(2);
            }
            self.dest.dot
        } else {
            self.src.dot
        };
        self.src.read_word(mem);
        self.src.nextaddress = self.src.nextaddress.wrapping_sub(2);
        self.src.dotcount = dot + 1;
        let shift =
            ((DOTCOUNTMAX[self.src.scrnmode] - self.src.dotcount) * BPP[self.src.scrnmode]) as u32;
        self.src.buf = ((u32::from(self.src.buf)) >> shift) as u16;
        self.src.xcount = self.src.width;
    }

    fn init_dest_line_hd(&mut self, mem: &Pc88VaMemory) {
        self.newval = 0;
        self.newvalmask = 0;
        self.dest.read_word(mem);
        self.dest.dotcount = self.dest.dot + 1;
        let shift = ((DOTCOUNTMAX[self.dest.scrnmode] - self.dest.dotcount)
            * BPP[self.dest.scrnmode]) as u32;
        self.dest.buf = ((u32::from(self.dest.buf)) >> shift) as u16;
        self.dest.xcount = self.dest.width;
    }

    fn logicalop(&self, dat: u16, dest: u16, mask: &mut u16) -> u16 {
        match self.bltmode & BLTMODE_OP {
            0x0 => 0,
            0x1 => dat & dest,
            0x2 => !dat & dest,
            0x3 => {
                *mask = 0;
                dat
            }
            0x4 => dat & !dest,
            0x6 => dat ^ dest,
            0x7 => dat | dest,
            0x8 => !(dat | dest),
            0x9 => !(dat ^ dest),
            0xA => !dat,
            0xB => !dat | dest,
            0xC => !dest,
            0xD => dat | !dest,
            0xE => !(dat & dest),
            0xF => 0xFFFF,
            _ => dat,
        }
    }

    fn write_dest2(&self, mem: &mut Pc88VaMemory) {
        // The undocumented transparent mode 3 never transfers.
        if self.bltmode & BLTMODE_TP == 0x0300 {
            return;
        }
        let dest = mem.sgp_read_word(self.dest.nextaddress).swap_bytes();
        let mut datmask = self.newvalmask;
        if self.bltmode & BLTMODE_TP == 0x0200 {
            datmask &= !zeromask(dest, self.dest.scrnmode);
        }
        let dat = self.logicalop(self.newval, dest, &mut datmask);
        let dat = (dest & !datmask) | (dat & datmask);
        mem.sgp_write_word(self.dest.nextaddress, dat.swap_bytes());
    }

    fn exec_bitblt(&mut self, mem: &mut Pc88VaMemory) {
        let bpp = BPP[self.dest.scrnmode];
        let bpp_u = bpp as u32;
        let pixmask = (!(0xFFFFu32 << bpp_u)) as u16;
        let extpix = self.src.scrnmode == 0 && self.dest.scrnmode != 0;

        if self.src.dotcount == 0 {
            self.src.read_word(mem);
            self.src.nextaddress = self.src.nextaddress.wrapping_add(2);
        }
        let dat: u16 = if extpix {
            if self.src.buf & 0x8000 != 0 {
                0xFFFF
            } else {
                0
            }
        } else {
            (u32::from(self.src.buf) >> ((16 - bpp) as u32)) as u16
        };
        let datmask: u16 = match self.bltmode & BLTMODE_TP {
            0x0100 => {
                if dat != 0 {
                    0xFFFF
                } else {
                    0
                }
            }
            _ => 0xFFFF,
        };

        self.newval = (((u32::from(self.newval)) << bpp_u) | u32::from(dat & pixmask)) as u16;
        self.newvalmask =
            (((u32::from(self.newvalmask)) << bpp_u) | u32::from(datmask & pixmask)) as u16;
        self.dest.dotcount -= 1;

        if extpix {
            self.src.buf = ((u32::from(self.src.buf)) << 1) as u16;
        } else {
            self.src.buf = ((u32::from(self.src.buf)) << bpp_u) as u16;
        }
        self.src.dotcount -= 1;
        self.dest.xcount = self.dest.xcount.wrapping_sub(1);
        self.src.xcount = self.src.xcount.wrapping_sub(1);

        if self.dest.dotcount == 0 || self.dest.xcount == 0 {
            let shift = (self.dest.dotcount * bpp) as u32;
            self.newval = ((u32::from(self.newval)) << shift) as u16;
            self.newvalmask = ((u32::from(self.newvalmask)) << shift) as u16;
            if extpix {
                self.newval &= self.color.swap_bytes();
            }
            self.write_dest2(mem);
            self.dest.nextaddress = self.dest.nextaddress.wrapping_add(2);
            self.dest.dotcount = DOTCOUNTMAX[self.dest.scrnmode];
            if (self.bltmode & BLTMODE_TP) == 0x0100 {
                self.spend(10 * 2);
            } else {
                self.spend(8 * 2);
            }
        }

        self.advance_blit_line(mem, false);
    }

    fn exec_bitblt_hd(&mut self, mem: &mut Pc88VaMemory) {
        let bpp = BPP[self.dest.scrnmode];
        let bpp_u = bpp as u32;
        let pixmask = (!(0xFFFFu32 << bpp_u)) as u16;
        let extpix = self.src.scrnmode == 0 && self.dest.scrnmode != 0;

        if self.src.dotcount == 0 {
            self.src.read_word(mem);
            self.src.nextaddress = self.src.nextaddress.wrapping_sub(2);
        }
        let dat: u16 = if extpix {
            if self.src.buf & 0x0001 != 0 {
                0xFFFF
            } else {
                0
            }
        } else {
            self.src.buf
        };
        let datmask: u16 = match self.bltmode & BLTMODE_TP {
            0x0100 => {
                if dat != 0 {
                    0xFFFF
                } else {
                    0
                }
            }
            _ => 0xFFFF,
        };

        self.newval = (((u32::from(self.newval)) >> bpp_u)
            | (u32::from(dat & pixmask) << ((16 - bpp) as u32))) as u16;
        self.newvalmask = (((u32::from(self.newvalmask)) >> bpp_u)
            | (u32::from(datmask & pixmask) << ((16 - bpp) as u32)))
            as u16;
        self.dest.dotcount -= 1;

        if extpix {
            self.src.buf = ((u32::from(self.src.buf)) >> 1) as u16;
        } else {
            self.src.buf = ((u32::from(self.src.buf)) >> bpp_u) as u16;
        }
        self.src.dotcount -= 1;
        self.dest.xcount = self.dest.xcount.wrapping_sub(1);
        self.src.xcount = self.src.xcount.wrapping_sub(1);

        if self.dest.dotcount == 0 || self.dest.xcount == 0 {
            let shift = (self.dest.dotcount * bpp) as u32;
            self.newval = ((u32::from(self.newval)) >> shift) as u16;
            self.newvalmask = ((u32::from(self.newvalmask)) >> shift) as u16;
            if extpix {
                self.newval &= self.color.swap_bytes();
            }
            self.write_dest2(mem);
            self.dest.nextaddress = self.dest.nextaddress.wrapping_sub(2);
            self.dest.dotcount = DOTCOUNTMAX[self.dest.scrnmode];
            if (self.bltmode & BLTMODE_TP) == 0x0100 {
                self.spend(10 * 2);
            } else {
                self.spend(8 * 2);
            }
        }

        self.advance_blit_line(mem, true);
    }

    /// Shared end-of-pixel line/wrap handling for the forward and reverse blits.
    fn advance_blit_line(&mut self, mem: &Pc88VaMemory, hd: bool) {
        if self.dest.xcount == 0 {
            self.dest.ycount = self.dest.ycount.wrapping_sub(1);
            self.src.ycount = self.src.ycount.wrapping_sub(1);
            if self.dest.ycount == 0 {
                self.func = SgpFunc::FetchCommand;
            } else {
                self.spend(14 * 2);
                if self.bltmode & BLTMODE_VD != 0 {
                    self.src.lineaddress = self
                        .src
                        .lineaddress
                        .wrapping_add_signed(-i32::from(self.src.fbw));
                    self.dest.lineaddress = self
                        .dest
                        .lineaddress
                        .wrapping_add_signed(-i32::from(self.dest.fbw));
                } else {
                    self.src.lineaddress = self
                        .src
                        .lineaddress
                        .wrapping_add_signed(i32::from(self.src.fbw));
                    self.dest.lineaddress = self
                        .dest
                        .lineaddress
                        .wrapping_add_signed(i32::from(self.dest.fbw));
                }
                if self.src.ycount == 0 {
                    self.src.init();
                }
                self.src.nextaddress = self.src.lineaddress;
                self.dest.nextaddress = self.dest.lineaddress;
                if hd {
                    self.init_src_line_hd(mem);
                    self.init_dest_line_hd(mem);
                } else {
                    self.init_src_line(mem);
                    self.init_dest_line();
                }
            }
        } else if self.src.xcount == 0 {
            self.src.nextaddress = self.src.lineaddress;
            if hd {
                self.init_src_line_hd(mem);
            } else {
                self.init_src_line(mem);
            }
        }
    }

    fn exec_cls(&mut self, mem: &mut Pc88VaMemory) {
        mem.sgp_write_word(self.clsaddr, self.color);
        self.clsaddr = self.clsaddr.wrapping_add(2);
        self.clscount -= 1;
        if self.clscount == 0 {
            self.func = SgpFunc::FetchCommand;
        }
        self.spend(3 * 2);
    }

    fn exec_line_x(&mut self, mem: &mut Pc88VaMemory) {
        let xdir: i32 = if self.bltmode & BLTMODE_LINE_HD != 0 {
            -1
        } else {
            1
        };
        let ydir: i32 = if self.bltmode & BLTMODE_LINE_VD != 0 {
            -1
        } else {
            1
        };
        let dotcountmax = DOTCOUNTMAX[self.dest.scrnmode];
        let bpp = BPP[self.dest.scrnmode];
        let bpp_u = bpp as u32;

        let dat = self.color.swap_bytes();
        let datmask = match self.bltmode & BLTMODE_TP {
            0x0100 => zeromask(dat, self.dest.scrnmode),
            _ => 0xFFFF,
        };

        let shift = ((self.dest.dotcount - 1) * bpp) as u32;
        if xdir > 0 {
            let pixmask = (!(0xFFFFu32 << bpp_u)) as u16;
            self.newval = (((u32::from(self.newval)) << bpp_u)
                | ((u32::from(dat) >> shift) & u32::from(pixmask)))
                as u16;
            self.newvalmask = (((u32::from(self.newvalmask)) << bpp_u)
                | ((u32::from(datmask) >> shift) & u32::from(pixmask)))
                as u16;
        } else {
            let pixmask = (!(0xFFFFu32 >> bpp_u)) as u16;
            self.newval = (((u32::from(self.newval)) >> bpp_u)
                | ((u32::from(dat) << shift) & u32::from(pixmask)))
                as u16;
            self.newvalmask = (((u32::from(self.newvalmask)) >> bpp_u)
                | ((u32::from(datmask) << shift) & u32::from(pixmask)))
                as u16;
        }

        self.dest.dotcount -= 1;
        self.lineslopecount += self.lineslopenumerator;
        self.dest.xcount = self.dest.xcount.wrapping_sub(1);
        self.spend(11);

        if self.dest.dotcount == 0
            || self.lineslopecount >= self.lineslopedenominator
            || self.dest.xcount == 0
        {
            let shift = (self.dest.dotcount * bpp) as u32;
            if xdir > 0 {
                self.newval = ((u32::from(self.newval)) << shift) as u16;
                self.newvalmask = ((u32::from(self.newvalmask)) << shift) as u16;
            } else {
                self.newval = ((u32::from(self.newval)) >> shift) as u16;
                self.newvalmask = ((u32::from(self.newvalmask)) >> shift) as u16;
            }
            self.write_dest2(mem);
            self.spend(3);

            if self.dest.xcount > 0 {
                if self.dest.dotcount == 0 {
                    if xdir > 0 {
                        self.dest.nextaddress = self.dest.nextaddress.wrapping_add(2);
                    } else {
                        self.dest.nextaddress = self.dest.nextaddress.wrapping_sub(2);
                    }
                    self.dest.dotcount = dotcountmax;
                }
                if self.lineslopecount >= self.lineslopedenominator {
                    self.lineslopecount -= self.lineslopedenominator;
                    if ydir > 0 {
                        self.dest.nextaddress = self
                            .dest
                            .nextaddress
                            .wrapping_add_signed(i32::from(self.dest.fbw));
                    } else {
                        self.dest.nextaddress = self
                            .dest
                            .nextaddress
                            .wrapping_add_signed(-i32::from(self.dest.fbw));
                    }
                    self.spend(11);
                }
                self.newval = 0;
                self.newvalmask = 0;
            }
        }

        if self.dest.xcount == 0 {
            self.func = SgpFunc::FetchCommand;
        }
    }

    fn exec_line_y(&mut self, mem: &mut Pc88VaMemory) {
        const YSTEPWAIT: [u64; 4] = [8, 9, 9, 10];
        let ydir: i32 = if self.bltmode & BLTMODE_LINE_VD != 0 {
            -1
        } else {
            1
        };
        let xdir: i32 = if self.bltmode & BLTMODE_LINE_HD != 0 {
            -1
        } else {
            1
        };
        let dotcountmax = DOTCOUNTMAX[self.dest.scrnmode];
        let bpp = BPP[self.dest.scrnmode];
        let bpp_u = bpp as u32;

        let dat = self.color.swap_bytes();
        let mut datmask = match self.bltmode & BLTMODE_TP {
            0x0100 => zeromask(dat, self.dest.scrnmode),
            _ => 0xFFFF,
        };
        let pixmask = ((!(0xFFFFu32 >> bpp_u)) >> ((self.dest.dotcount * bpp) as u32)) as u16;
        datmask &= pixmask;

        self.newval = dat;
        self.newvalmask = datmask;
        self.write_dest2(mem);
        self.spend(3);

        if ydir > 0 {
            self.dest.nextaddress = self
                .dest
                .nextaddress
                .wrapping_add_signed(i32::from(self.dest.fbw));
        } else {
            self.dest.nextaddress = self
                .dest
                .nextaddress
                .wrapping_add_signed(-i32::from(self.dest.fbw));
        }
        self.spend(YSTEPWAIT[self.dest.scrnmode]);

        self.lineslopecount += self.lineslopenumerator;
        if self.lineslopecount >= self.lineslopedenominator {
            self.dest.dotcount += xdir;
            if self.dest.dotcount < 0 {
                self.dest.nextaddress = self.dest.nextaddress.wrapping_sub(2);
                self.dest.dotcount += dotcountmax;
            } else if self.dest.dotcount >= dotcountmax {
                self.dest.nextaddress = self.dest.nextaddress.wrapping_add(2);
                self.dest.dotcount -= dotcountmax;
            }
            self.lineslopecount -= self.lineslopedenominator;
            self.spend(11);
        }

        self.dest.ycount = self.dest.ycount.wrapping_sub(1);
        if self.dest.ycount == 0 {
            self.func = SgpFunc::FetchCommand;
        }
    }
}

/// Loads a 6-word block descriptor from memory.
fn fetch_block(mem: &Pc88VaMemory, address: u32, block: &mut SgpBlock) {
    let dat = mem.sgp_read_word(address);
    block.scrnmode = (dat & 0x03) as usize;
    block.dot = (i32::from(dat >> 4)) & (DOTCOUNTMAX[block.scrnmode] - 1);
    block.width = mem.sgp_read_word(address.wrapping_add(2)) & 0x3FFF;
    block.height = mem.sgp_read_word(address.wrapping_add(4));
    block.fbw = (mem.sgp_read_word(address.wrapping_add(6)) & 0xFFFE) as i16;
    block.address = u32::from(mem.sgp_read_word(address.wrapping_add(8)) & 0xFFFE)
        | (u32::from(mem.sgp_read_word(address.wrapping_add(10))) << 16);
    if block.fbw < 0 {
        block.fbw = block.fbw.wrapping_add(2);
    }
}

/// Returns a per-pixel mask: bits set where the pixel value is non-zero.
fn zeromask(dat: u16, scrnmode: usize) -> u16 {
    let bpp = BPP[scrnmode] as u32;
    let pixmask = (!(0xFFFFu32 >> bpp)) as u16;
    let maskelem = (!(0xFFFFu32 << bpp)) as u16;
    let mut mask: u16 = 0;
    let mut dat = dat;
    for _ in 0..DOTCOUNTMAX[scrnmode] {
        mask = ((u32::from(mask)) << bpp) as u16;
        if dat & pixmask != 0 {
            mask |= maskelem;
        }
        dat = ((u32::from(dat)) << bpp) as u16;
    }
    mask
}

/// The value the SGP status/control ports return when single-plane (GMSP) mode is off.
fn sgp_notactive(port: u16) -> u8 {
    if port & 0x0F == 0x0A { 0xFA } else { 0xFE }
}

/// The value an unhandled SGP port returns.
fn sgp_notimpl(port: u16) -> u8 {
    if port & 1 != 0 {
        if port == 0x501 || port == 0x503 {
            0xFF
        } else if port & 0x02 != 0 {
            0xFD
        } else {
            0xFF
        }
    } else if port & 0x0F == 0x0A {
        0xFA
    } else {
        0xFE
    }
}

impl Pc88VaBus {
    /// True when the SGP is active, i.e. single-plane (GMSP) mode is selected.
    fn sgp_active(&self) -> bool {
        self.memory.gmsp_bit() != 0
    }

    /// Dispatches an SGP register read (`0x500-0x508`).
    pub(crate) fn sgp_io_read(&self, port: u16) -> u8 {
        if !self.sgp_active() {
            return sgp_notactive(port);
        }
        match port {
            0x504 => self.sgp.ctrl,
            0x506 => self.sgp.busy,
            0x508 => 1,
            _ => sgp_notimpl(port),
        }
    }

    /// Dispatches an SGP register write (`0x500-0x506`).
    pub(crate) fn sgp_io_write(&mut self, port: u16, value: u8) {
        match port {
            0x500..=0x503 => self.sgp.write_address_byte(port, value),
            0x504 => self.write_sgp_ctrl(value),
            0x506 if self.sgp.write_trigger(value) => self.start_sgp(),
            _ => {}
        }
    }

    /// Handles a write to the SGP control port (`0x504`): interrupt-enable and
    /// the abort request.
    fn write_sgp_ctrl(&mut self, value: u8) {
        let value = value & (SGP_INTF | SGP_ABORT);
        self.sgp.ctrl = value;
        if value & SGP_ABORT != 0 {
            self.sgp.busy &= !SGP_BUSY;
            self.scheduler.cancel(Event88Va::SgpComplete);
            if value & SGP_INTF != 0 && !self.sgp.intreq {
                self.sgp.intreq = true;
                self.pic.set_irq(8);
            }
            self.update_next_event_cycle();
        }
        if value & SGP_INTF == 0 && self.sgp.intreq {
            self.sgp.intreq = false;
            self.pic.clear_irq(8);
        }
    }

    /// Runs the command list and schedules the completion event. The SGP only
    /// runs in single-plane (GMSP) mode.
    fn start_sgp(&mut self) {
        if !self.sgp_active() {
            return;
        }
        let cycles = self.sgp.execute(&mut self.memory);
        self.scheduler
            .schedule(Event88Va::SgpComplete, self.current_cycle + cycles.max(1));
        self.update_next_event_cycle();
    }

    /// Clears busy and raises the SGP interrupt when a run completes.
    pub(crate) fn on_sgp_complete(&mut self) {
        self.sgp.busy &= !SGP_BUSY;
        if self.sgp.ctrl & SGP_INTF != 0 && !self.sgp.intreq {
            self.sgp.intreq = true;
            self.pic.set_irq(8);
        }
    }
}
