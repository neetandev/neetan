/*
 * Copyright (C) 2021, 2024 nukeykt
 *
 *  Redistribution and use of this code or any derivative works are permitted
 *  provided that the following conditions are met:
 *
 *   - Redistributions may not be sold, nor may they be used in a commercial
 *     product or activity.
 *
 *   - Redistributions that are modified from the original source must include the
 *     complete source code, including the source code for all components used by a
 *     binary built from the modified sources. However, as a special exception, the
 *     source code distributed need not include anything that is normally distributed
 *     (in either source or binary form) with the major components (compiler, kernel,
 *     and so on) of the operating system on which the executable runs, unless that
 *     component itself accompanies the executable.
 *
 *   - Redistributions must reproduce the above copyright notice, this list of
 *     conditions and the following disclaimer in the documentation and/or other
 *     materials provided with the distribution.
 *
 *  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
 *  AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 *  IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
 *  ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
 *  LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
 *  CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
 *  SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
 *  INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
 *  CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
 *  ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
 *  POSSIBILITY OF SUCH DAMAGE.
 */

use crate::{Sc55State, mcu_interrupt::INTERRUPT_SOURCE_IRQ0};

#[derive(Clone)]
/// Complete authoritative state of the SC-55 PCM engine.
pub(crate) struct PcmState {
    pub ram1: [[u32; 8]; 32],
    pub ram2: [[u16; 16]; 32],
    pub select_channel: u32,
    pub voice_mask: u32,
    pub voice_mask_pending: u32,
    pub voice_mask_updating: u32,
    pub write_latch: u32,
    pub wave_read_address: u32,
    pub wave_byte_latch: u8,
    pub read_latch: u32,
    pub config_reg_3c: u8, // SC55:c3 JV880:c0
    pub config_reg_3d: u8,
    pub irq_channel: u32,
    pub irq_assert: u32,
    pub nfs: u32,
    pub tv_counter: u32,
    pub cycles: u64,
    pub eram: Box<[u16; 0x4000]>,
    pub accum_l: i32,
    pub accum_r: i32,
    pub rcsum: [i32; 2],
}

impl Default for PcmState {
    fn default() -> Self {
        PcmState {
            ram1: [[0u32; 8]; 32],
            ram2: [[0u16; 16]; 32],
            select_channel: 0,
            voice_mask: 0,
            voice_mask_pending: 0,
            voice_mask_updating: 0,
            write_latch: 0,
            wave_read_address: 0,
            wave_byte_latch: 0,
            read_latch: 0,
            config_reg_3c: 0,
            config_reg_3d: 0,
            irq_channel: 0,
            irq_assert: 0,
            nfs: 0,
            tv_counter: 0,
            cycles: 0,
            eram: Box::new([0u16; 0x4000]),
            accum_l: 0,
            accum_r: 0,
            rcsum: [0i32; 2],
        }
    }
}

crate::impl_state_codec!(PcmState {
    ram1,
    ram2,
    select_channel,
    voice_mask,
    voice_mask_pending,
    voice_mask_updating,
    write_latch,
    wave_read_address,
    wave_byte_latch,
    read_latch,
    config_reg_3c,
    config_reg_3d,
    irq_channel,
    irq_assert,
    nfs,
    tv_counter,
    cycles,
    eram,
    accum_l,
    accum_r,
    rcsum,
});

fn waverom_read(rom: &[u8], index: usize) -> u8 {
    if index < rom.len() { rom[index] } else { 0 }
}

fn pcm_read_rom(state: &Sc55State) -> u8 {
    let address = state.pcm.wave_read_address;
    let bank = if state.pcm.config_reg_3d & 0x20 != 0 {
        ((address >> 21) & 7) as i32
    } else {
        ((address >> 19) & 7) as i32
    };
    match bank {
        0 => {
            if state.mcu_mk1 {
                waverom_read(&state.waverom1, (address & 0xFFFFF) as usize)
            } else {
                waverom_read(&state.waverom1, (address & 0x1FFFFF) as usize)
            }
        }
        1 => {
            if !state.mcu_jv880 {
                waverom_read(&state.waverom2, (address & 0xFFFFF) as usize)
            } else {
                waverom_read(&state.waverom2, (address & 0x1FFFFF) as usize)
            }
        }
        2 => {
            if state.mcu_jv880 {
                waverom_read(&state.waverom_card, (address & 0x1FFFFF) as usize)
            } else {
                waverom_read(&state.waverom3, (address & 0xFFFFF) as usize)
            }
        }
        3..=6 if state.mcu_jv880 => waverom_read(
            &state.waverom_exp,
            ((address & 0x1FFFFF) + (bank as u32 - 3) * 0x200000) as usize,
        ),
        _ => 0,
    }
}

#[allow(clippy::identity_op)]
fn pcm_read_rom_addr(state: &Sc55State, address: u32) -> u8 {
    let bank = if state.pcm.config_reg_3d & 0x20 != 0 {
        ((address >> 21) & 7) as i32
    } else {
        ((address >> 19) & 7) as i32
    };
    match bank {
        0 => {
            if state.mcu_mk1 {
                waverom_read(&state.waverom1, (address & 0xFFFFF) as usize)
            } else {
                waverom_read(&state.waverom1, (address & 0x1FFFFF) as usize)
            }
        }
        1 => {
            if !state.mcu_jv880 {
                waverom_read(&state.waverom2, (address & 0xFFFFF) as usize)
            } else {
                waverom_read(&state.waverom2, (address & 0x1FFFFF) as usize)
            }
        }
        2 => {
            if state.mcu_jv880 {
                waverom_read(&state.waverom_card, (address & 0x1FFFFF) as usize)
            } else {
                waverom_read(&state.waverom3, (address & 0xFFFFF) as usize)
            }
        }
        3..=6 if state.mcu_jv880 => waverom_read(
            &state.waverom_exp,
            ((address & 0x1FFFFF) + (bank as u32 - 3) * 0x200000) as usize,
        ),
        _ => 0,
    }
}

#[allow(clippy::identity_op, clippy::manual_range_contains)]
pub(crate) fn pcm_write(state: &mut Sc55State, address: u32, data: u8) {
    let address = address & 0x3F;
    if address < 0x4 {
        // voice enable
        match address & 3 {
            0 => {
                state.pcm.voice_mask_pending &= !0xF000000;
                state.pcm.voice_mask_pending |= ((data & 0xF) as u32) << 24;
            }
            1 => {
                state.pcm.voice_mask_pending &= !0xFF0000;
                state.pcm.voice_mask_pending |= ((data & 0xFF) as u32) << 16;
            }
            2 => {
                state.pcm.voice_mask_pending &= !0xFF00;
                state.pcm.voice_mask_pending |= ((data & 0xFF) as u32) << 8;
            }
            3 => {
                state.pcm.voice_mask_pending &= !0xFF;
                state.pcm.voice_mask_pending |= ((data & 0xFF) as u32) << 0;
            }
            _ => {}
        }
        state.pcm.voice_mask_updating = 1;
    } else if address >= 0x20 && address < 0x24 {
        // wave rom
        match address & 3 {
            1 => {
                state.pcm.wave_read_address &= !0xFF0000;
                state.pcm.wave_read_address |= ((data & 0xFF) as u32) << 16;
            }
            2 => {
                state.pcm.wave_read_address &= !0xFF00;
                state.pcm.wave_read_address |= ((data & 0xFF) as u32) << 8;
            }
            3 => {
                state.pcm.wave_read_address &= !0xFF;
                state.pcm.wave_read_address |= ((data & 0xFF) as u32) << 0;
                state.pcm.wave_byte_latch = pcm_read_rom(state);
            }
            _ => {}
        }
    } else if address == 0x3C {
        state.pcm.config_reg_3c = data;
    } else if address == 0x3D {
        state.pcm.config_reg_3d = data;
    } else if address == 0x3E {
        state.pcm.select_channel = (data & 0x1F) as u32;
    } else if (address >= 0x4 && address < 0x10) || (address >= 0x24 && address < 0x30) {
        match address & 3 {
            1 => {
                state.pcm.write_latch &= !0xF0000;
                state.pcm.write_latch |= ((data & 0xF) as u32) << 16;
            }
            2 => {
                state.pcm.write_latch &= !0xFF00;
                state.pcm.write_latch |= ((data & 0xFF) as u32) << 8;
            }
            3 => {
                state.pcm.write_latch &= !0xFF;
                state.pcm.write_latch |= ((data & 0xFF) as u32) << 0;
            }
            _ => {}
        }
        if (address & 3) == 3 {
            let mut ix: usize = 0;
            if address & 32 != 0 {
                ix |= 1;
            }
            if address & 8 == 0 {
                ix |= 4;
            }
            if address & 4 == 0 {
                ix |= 2;
            }

            let ch = state.pcm.select_channel as usize;
            state.pcm.ram1[ch][ix] = state.pcm.write_latch;
        }
    } else if (address >= 0x10 && address < 0x20) || (address >= 0x30 && address < 0x38) {
        match address & 1 {
            0 => {
                state.pcm.write_latch &= !0xFF00;
                state.pcm.write_latch |= ((data & 0xFF) as u32) << 8;
            }
            1 => {
                state.pcm.write_latch &= !0xFF;
                state.pcm.write_latch |= ((data & 0xFF) as u32) << 0;
            }
            _ => {}
        }
        if (address & 1) == 1 {
            let mut ix: usize = ((address >> 1) & 7) as usize;
            if address & 32 != 0 {
                ix |= 8;
            }

            let ch = state.pcm.select_channel as usize;
            state.pcm.ram2[ch][ix] = state.pcm.write_latch as u16;
        }
    }
}

// rv: [30][2], [30][3]
// ch: [31][2], [31][5]

#[allow(clippy::identity_op, clippy::manual_range_contains)]
pub(crate) fn pcm_read(state: &mut Sc55State, address: u32) -> u8 {
    let address = address & 0x3F;
    //printf("PCM Read: %.2x\n", address);

    if address < 0x4 {
        if state.pcm.voice_mask_updating != 0 {
            state.pcm.voice_mask = state.pcm.voice_mask_pending;
        }
        state.pcm.voice_mask_updating = 0;
    } else if address == 0x3C || address == 0x3E {
        // status
        let mut status: u8 = 0;
        if address == 0x3E && state.pcm.irq_assert != 0 {
            state.pcm.irq_assert = 0;
            if state.mcu_jv880 {
                crate::mcu::mcu_ga_set_ga_int(state, 5, 0);
            } else {
                crate::mcu_interrupt::mcu_interrupt_set_request(state, INTERRUPT_SOURCE_IRQ0, 0);
            }
        }

        status |= state.pcm.irq_channel as u8;
        if state.pcm.voice_mask_updating != 0 {
            status |= 32;
        }

        return status;
    } else if address == 0x3F {
        return state.pcm.wave_byte_latch;
    } else if (address >= 0x4 && address < 0x10) || (address >= 0x24 && address < 0x30) {
        if (address & 3) == 1 {
            let mut ix: usize = 0;
            if address & 32 != 0 {
                ix |= 1;
            }
            if address & 8 == 0 {
                ix |= 4;
            }
            if address & 4 == 0 {
                ix |= 2;
            }

            let ch = state.pcm.select_channel as usize;
            state.pcm.read_latch = state.pcm.ram1[ch][ix];
        }
    } else if (address >= 0x10 && address < 0x20) || (address >= 0x30 && address < 0x38) {
        if (address & 1) == 0 {
            let mut ix: usize = ((address >> 1) & 7) as usize;
            if address & 32 != 0 {
                ix |= 8;
            }

            let ch = state.pcm.select_channel as usize;
            state.pcm.read_latch = state.pcm.ram2[ch][ix] as u32;
        }
    } else if address >= 0x39 && address <= 0x3B {
        match address & 3 {
            1 => {
                return ((state.pcm.read_latch >> 16) & 0xF) as u8;
            }
            2 => {
                return ((state.pcm.read_latch >> 8) & 0xFF) as u8;
            }
            3 => {
                return ((state.pcm.read_latch >> 0) & 0xFF) as u8;
            }
            _ => {}
        }
    }

    0
}

pub(crate) fn pcm_reset(state: &mut Sc55State) {
    state.pcm = PcmState::default();
}

#[inline]
fn addclip20(add1: u32, add2: u32, cin: u32) -> u32 {
    let mut sum = (add1.wrapping_add(add2).wrapping_add(cin)) & 0xFFFFF;
    if (add1 & 0x80000) != 0 && (add2 & 0x80000) != 0 && (sum & 0x80000) == 0 {
        sum = 0x80000;
    } else if (add1 & 0x80000) == 0 && (add2 & 0x80000) == 0 && (sum & 0x80000) != 0 {
        sum = 0x7FFFF;
    }
    sum
}

#[inline]
fn multi(val1: i32, val2: i8) -> i32 {
    let mut val1 = val1;
    if val1 & 0x80000 != 0 {
        val1 |= !0xFFFFF;
    } else {
        val1 &= 0x7FFFF;
    }

    val1 = val1.wrapping_mul(val2 as i32);
    if val1 & 0x8000000 != 0 {
        val1 |= !0x1FFFFFF;
    } else {
        val1 &= 0x1FFFFFF;
    }
    val1
}

static INTERP_LUT: [[i32; 128]; 3] = [
    [
        3385, 3401, 3417, 3432, 3448, 3463, 3478, 3492, 3506, 3521, 3534, 3548, 3562, 3575, 3588,
        3601, 3614, 3626, 3638, 3650, 3662, 3673, 3685, 3696, 3707, 3718, 3728, 3739, 3749, 3759,
        3768, 3778, 3787, 3796, 3805, 3814, 3823, 3831, 3839, 3847, 3855, 3863, 3870, 3878, 3885,
        3892, 3899, 3905, 3912, 3918, 3924, 3930, 3936, 3942, 3948, 3953, 3958, 3963, 3968, 3973,
        3978, 3983, 3987, 3991, 3995, 4000, 4004, 4007, 4011, 4015, 4018, 4022, 4025, 4028, 4031,
        4034, 4037, 4040, 4042, 4045, 4047, 4050, 4052, 4054, 4057, 4059, 4061, 4063, 4064, 4066,
        4068, 4070, 4071, 4073, 4074, 4076, 4077, 4078, 4079, 4081, 4082, 4083, 4084, 4085, 4086,
        4086, 4087, 4088, 4089, 4089, 4090, 4091, 4091, 4092, 4092, 4093, 4093, 4094, 4094, 4094,
        4094, 4095, 4095, 4095, 4095, 4095, 4095, 4095,
    ],
    [
        710, 726, 742, 758, 775, 792, 809, 826, 844, 861, 879, 897, 915, 933, 952, 971, 990, 1009,
        1028, 1047, 1067, 1087, 1106, 1126, 1147, 1167, 1188, 1208, 1229, 1250, 1271, 1292, 1314,
        1335, 1357, 1379, 1400, 1423, 1445, 1467, 1489, 1512, 1534, 1557, 1580, 1602, 1625, 1648,
        1671, 1695, 1718, 1741, 1764, 1788, 1811, 1835, 1858, 1882, 1906, 1929, 1953, 1977, 2000,
        2024, 2048, 2069, 2095, 2119, 2143, 2166, 2190, 2214, 2237, 2261, 2284, 2308, 2331, 2355,
        2378, 2401, 2425, 2448, 2471, 2494, 2517, 2539, 2562, 2585, 2607, 2630, 2652, 2674, 2696,
        2718, 2740, 2762, 2783, 2805, 2826, 2847, 2868, 2889, 2910, 2931, 2951, 2971, 2991, 3011,
        3031, 3051, 3070, 3089, 3108, 3127, 3146, 3164, 3182, 3200, 3218, 3236, 3253, 3271, 3288,
        3304, 3321, 3338, 3354, 3370,
    ],
    [
        0, 0, 0, 1, 1, 1, 2, 2, 3, 3, 3, 4, 4, 5, 5, 6, 6, 7, 8, 8, 9, 10, 10, 11, 12, 13, 14, 15,
        16, 17, 18, 19, 20, 22, 23, 24, 26, 27, 29, 30, 32, 34, 36, 38, 40, 42, 44, 46, 49, 51, 53,
        56, 59, 62, 65, 68, 71, 74, 77, 81, 84, 88, 92, 96, 100, 104, 109, 113, 118, 122, 127, 132,
        137, 143, 148, 154, 160, 165, 171, 178, 184, 191, 197, 204, 211, 219, 226, 234, 241, 249,
        257, 266, 274, 283, 292, 301, 310, 319, 329, 339, 349, 359, 369, 380, 391, 402, 413, 424,
        436, 448, 460, 472, 484, 497, 510, 523, 536, 549, 563, 577, 591, 605, 619, 634, 648, 663,
        679, 694,
    ],
];

#[inline]
#[allow(clippy::identity_op, clippy::comparison_chain)]
fn calc_tv(
    pcm_nfs: u32,
    pcm_tv_counter: u32,
    e: i32,
    adjust: i32,
    levelcur: &mut u16,
    active: i32,
    volmul: Option<&mut i32>,
) {
    // int adjust = ram2[3+e];
    // int levelcur = ram2[9+e] & 0x7fff;
    *levelcur &= 0x7FFF;
    let speed = adjust & 0xFF;
    let target = (adjust >> 8) & 0xFF;

    let w1 = ((speed & 0xF0) == 0) as i32;
    let w2 = (w1 != 0 || (speed & 0x10) != 0) as i32;
    let w3 = (pcm_nfs != 0
        && ((speed & 0x80) == 0 || ((speed & 0x40) == 0 && (w2 == 0 || (speed & 0x20) == 0))))
        as i32;

    let mut typ = w2 | (w3 << 3);
    if speed & 0x20 != 0 {
        typ |= 2;
    }
    if (speed & 0x80) == 0 || (speed & 0x40) == 0 {
        typ |= 4;
    }

    let mut write = (active == 0) as i32;
    let mut addlow: i32 = 0;
    if typ & 4 != 0 {
        if pcm_tv_counter & 8 != 0 {
            addlow |= 1;
        }
        if pcm_tv_counter & 4 != 0 {
            addlow |= 2;
        }
        if pcm_tv_counter & 2 != 0 {
            addlow |= 4;
        }
        if pcm_tv_counter & 1 != 0 {
            addlow |= 8;
        }
        write |= 1;
    } else {
        match typ & 3 {
            0 => {
                if pcm_tv_counter & 0x20 != 0 {
                    addlow |= 1;
                }
                if pcm_tv_counter & 0x10 != 0 {
                    addlow |= 2;
                }
                if pcm_tv_counter & 8 != 0 {
                    addlow |= 4;
                }
                if pcm_tv_counter & 4 != 0 {
                    addlow |= 8;
                }
                write |= ((pcm_tv_counter & 3) == 0) as i32;
            }
            1 => {
                if pcm_tv_counter & 0x80 != 0 {
                    addlow |= 1;
                }
                if pcm_tv_counter & 0x40 != 0 {
                    addlow |= 2;
                }
                if pcm_tv_counter & 0x20 != 0 {
                    addlow |= 4;
                }
                if pcm_tv_counter & 0x10 != 0 {
                    addlow |= 8;
                }
                write |= ((pcm_tv_counter & 15) == 0) as i32;
            }
            2 => {
                if pcm_tv_counter & 0x200 != 0 {
                    addlow |= 1;
                }
                if pcm_tv_counter & 0x100 != 0 {
                    addlow |= 2;
                }
                if pcm_tv_counter & 0x80 != 0 {
                    addlow |= 4;
                }
                if pcm_tv_counter & 0x40 != 0 {
                    addlow |= 8;
                }
                write |= ((pcm_tv_counter & 63) == 0) as i32;
            }
            3 => {
                if pcm_tv_counter & 0x800 != 0 {
                    addlow |= 1;
                }
                if pcm_tv_counter & 0x400 != 0 {
                    addlow |= 2;
                }
                if pcm_tv_counter & 0x200 != 0 {
                    addlow |= 4;
                }
                if pcm_tv_counter & 0x100 != 0 {
                    addlow |= 8;
                }
                write |= ((pcm_tv_counter & 127) == 0) as i32;
            }
            _ => {}
        }
    }

    if (typ & 8) == 0 {
        let shift = speed & 15;
        let shift = (10i32.wrapping_sub(shift)) & 15;

        let mut sum1 = target << 11; // 5
        if e != 2 || active != 0 {
            sum1 -= (*levelcur as i32) << 4; // 6
        }
        let _neg = (sum1 & 0x80000) != 0;

        let preshift = sum1;

        let shifted = (preshift >> shift).wrapping_sub(sum1);

        let sum2 = (target << 11).wrapping_add(addlow).wrapping_add(shifted);
        if write != 0 && pcm_nfs != 0 {
            *levelcur = ((sum2 >> 4) & 0x7FFF) as u16;
        }

        if e == 0 {
            if let Some(vm) = volmul {
                *vm = (sum2 >> 4) & 0x7FFE;
            }
        } else if e == 1
            && let Some(vm) = volmul
        {
            *vm = (sum2 >> 4) & 0x7FFE;
        }
    } else {
        let mut shift = (speed >> 4) & 14;
        shift |= w2;
        shift = (10i32.wrapping_sub(shift)) & 15;

        let mut sum1 = target << 11; // 5
        if e != 2 || active != 0 {
            sum1 -= (*levelcur as i32) << 4; // 6
        }
        let neg = (sum1 & 0x80000) != 0;
        let mut preshift = (speed & 15) << 9;
        if w1 == 0 {
            preshift |= 0x2000;
        }
        if neg {
            preshift ^= !0x3F;
        }

        let shifted = preshift >> shift;
        let mut sum2 = shifted;
        if e != 2 || active != 0 {
            sum2 = sum2
                .wrapping_add((*levelcur as i32) << 4)
                .wrapping_add(addlow);
        }

        let sum2_l = sum2 >> 4;

        let sum3 = (target << 11).wrapping_sub(sum2_l << 4);

        let neg2 = (sum3 & 0x80000) != 0;
        let xnor = !(neg2 ^ neg);

        if write != 0 && pcm_nfs != 0 {
            if xnor {
                *levelcur = (sum2_l & 0x7FFF) as u16;
            } else {
                *levelcur = (target << 7) as u16;
            }
        }

        if e == 0 {
            if let Some(vm) = volmul {
                *vm = sum2_l & 0x7FFE;
            }
        } else if e == 1
            && let Some(vm) = volmul
        {
            if xnor {
                *vm = sum2_l & 0x7FFE;
            } else {
                *vm = target << 7;
            }
        }
    }
}

#[inline]
fn eram_unpack(eram: &[u16; 0x4000], addr: i32, typ: i32) -> i32 {
    let addr = (addr as usize) & 0x3FFF;
    let data = eram[addr] as i32;
    let val = data & 0x3FFF;
    let sh = (data >> 14) & 3;

    let val = val << 18;
    val >> (18 - sh * 2 + typ)
}

#[inline]
fn eram_pack(eram: &mut [u16; 0x4000], addr: i32, val: i32) {
    let addr = (addr as usize) & 0x3FFF;
    let mut top = (val >> 13) & 0x7F;
    if top & 0x40 != 0 {
        top ^= 0x7F;
    }
    let sh: i32;
    if top >= 16 {
        sh = 3;
    } else if top >= 4 {
        sh = 2;
    } else if top >= 1 {
        sh = 1;
    } else {
        sh = 0;
    }

    let mut data = (val >> (sh * 2)) & 0x3FFF;
    data |= sh << 14;
    eram[addr] = data as u16;
}

#[inline]
fn mcu_post_sample(state: &mut Sc55State, tt: &mut [i32; 2]) {
    // 2^29: the PCM accumulator is i32 fixed-point with 29 fractional bits.
    const DIV_REC: f32 = 1.0 / (1u32 << 29) as f32;

    if state.render_frames_written < state.render_frames_requested {
        let offset = (state.render_frames_written as usize) * 2;
        state.render_output[offset] = tt[0] as f32 * DIV_REC;
        state.render_output[offset + 1] = tt[1] as f32 * DIV_REC;
        state.render_frames_written += 1;
    }
}

#[allow(
    clippy::identity_op,
    clippy::collapsible_if,
    clippy::collapsible_else_if,
    clippy::manual_range_contains,
    clippy::comparison_chain
)]
pub(crate) fn pcm_update(state: &mut Sc55State, cycles: u64) {
    let reg_slots = ((state.pcm.config_reg_3d & 31) + 1) as i32;
    let voice_active = state.pcm.voice_mask & state.pcm.voice_mask_pending;
    while state.pcm.cycles < cycles {
        let mut tt: [i32; 2] = [0; 2];

        {
            // final mixing
            let mut noise_mask: i32 = 0;
            let mut orval: i32 = 0;
            let mut write_mask: i32;
            let dac_mask: i32;
            if (state.pcm.config_reg_3c & 0x30) != 0 {
                match (state.pcm.config_reg_3c >> 2) & 3 {
                    1 => {
                        noise_mask = 3;
                    }
                    2 => {
                        noise_mask = 7;
                    }
                    3 => {
                        noise_mask = 15;
                    }
                    _ => {}
                }
                match state.pcm.config_reg_3c & 3 {
                    1 => {
                        orval |= 1 << 8;
                    }
                    2 => {
                        orval |= 1 << 10;
                    }
                    _ => {}
                }
                write_mask = 15;
                dac_mask = !15;
            } else {
                match (state.pcm.config_reg_3c >> 2) & 3 {
                    2 => {
                        noise_mask = 1;
                    }
                    3 => {
                        noise_mask = 3;
                    }
                    _ => {}
                }
                match state.pcm.config_reg_3c & 3 {
                    1 => {
                        orval |= 1 << 6;
                    }
                    2 => {
                        orval |= 1 << 8;
                    }
                    _ => {}
                }
                write_mask = 3;
                dac_mask = !3;
            }
            if (state.pcm.config_reg_3c & 0x80) == 0 {
                write_mask = 0;
            }
            if (state.pcm.config_reg_3c & 0x30) == 0x30 {
                orval |= 1 << 12;
            }
            let _ = dac_mask;

            let mut shifter = state.pcm.ram2[30][10] as i32;
            let mut xr = ((shifter >> 0) ^ (shifter >> 1) ^ (shifter >> 7) ^ (shifter >> 12)) & 1;
            shifter = (shifter >> 1) | (xr << 15);
            state.pcm.ram2[30][10] = shifter as u16;

            state.pcm.accum_l =
                addclip20(state.pcm.accum_l as u32, state.pcm.ram1[30][0], 0) as i32;
            state.pcm.accum_r =
                addclip20(state.pcm.accum_r as u32, state.pcm.ram1[30][1], 0) as i32;

            state.pcm.ram1[30][2] = addclip20(
                state.pcm.accum_l as u32,
                (orval | (shifter & noise_mask)) as u32,
                0,
            );

            state.pcm.ram1[30][4] = addclip20(
                state.pcm.accum_r as u32,
                (orval | (shifter & noise_mask)) as u32,
                0,
            );

            state.pcm.ram1[30][0] = state.pcm.accum_l as u32 & write_mask as u32;
            state.pcm.ram1[30][1] = state.pcm.accum_r as u32 & write_mask as u32;

            tt[0] = ((state.pcm.ram1[30][2] & !(write_mask as u32)) << 12) as i32;
            tt[1] = ((state.pcm.ram1[30][4] & !(write_mask as u32)) << 12) as i32;

            mcu_post_sample(state, &mut tt);

            xr = ((shifter >> 0) ^ (shifter >> 1) ^ (shifter >> 7) ^ (shifter >> 12)) & 1;
            shifter = (shifter >> 1) | (xr << 15);

            state.pcm.accum_l =
                addclip20(state.pcm.accum_l as u32, state.pcm.ram1[30][0], 0) as i32;
            state.pcm.accum_r =
                addclip20(state.pcm.accum_r as u32, state.pcm.ram1[30][1], 0) as i32;

            state.pcm.ram1[30][3] = addclip20(
                state.pcm.accum_l as u32,
                (orval | (shifter & noise_mask)) as u32,
                0,
            );

            state.pcm.ram1[30][5] = addclip20(
                state.pcm.accum_r as u32,
                (orval | (shifter & noise_mask)) as u32,
                0,
            );

            if state.pcm.config_reg_3c & 0x40 != 0 {
                // oversampling
                state.pcm.ram2[30][10] = shifter as u16;

                state.pcm.ram1[30][0] = state.pcm.accum_l as u32 & write_mask as u32;
                state.pcm.ram1[30][1] = state.pcm.accum_r as u32 & write_mask as u32;

                tt[0] = ((state.pcm.ram1[30][3] & !(write_mask as u32)) << 12) as i32;
                tt[1] = ((state.pcm.ram1[30][5] & !(write_mask as u32)) << 12) as i32;

                mcu_post_sample(state, &mut tt);
            }
        }

        {
            // global counter for envelopes
            if state.pcm.nfs == 0 {
                state.pcm.tv_counter = state.pcm.ram2[31][8] as u32; // fixme
            }

            state.pcm.tv_counter = state.pcm.tv_counter.wrapping_sub(1);

            state.pcm.tv_counter &= 0x3FFF;
        }

        // chorus/reverb

        {
            // fixme
            if state.pcm.ram2[31][8] & 0x8000 != 0 {
                state.pcm.ram2[31][9] = state.pcm.ram2[31][8] & 0x7FFF;
            } else {
                state.pcm.ram2[31][10] = state.pcm.ram2[31][8] & 0x7FFF;
            }

            if (0x4000u16.wrapping_sub(state.pcm.ram2[31][8])) & 0x8000 != 0 {
                state.pcm.ram2[31][10] = (0x4000u16.wrapping_sub(state.pcm.ram2[31][8])) & 0x7FFF;
            } else {
                state.pcm.ram2[31][9] = (0x4000u16.wrapping_sub(state.pcm.ram2[31][8])) & 0x7FFF;
            }
        }

        {
            let v1 = state.pcm.ram2[31][1] as i32;

            let m1 = multi(state.pcm.ram1[29][1] as i32, (v1 >> 8) as i8) >> 5; // 14
            let m2 = multi(state.pcm.rcsum[1], (v1 & 255) as i8) >> 5; // 15

            state.pcm.ram1[29][1] =
                addclip20((m1 >> 1) as u32, (m2 >> 1) as u32, ((m1 | m2) & 1) as u32);
            // 16
        }

        {
            let okey = ((state.pcm.ram2[31][7] & 0x20) != 0) as i32;
            let key = 1i32;
            let active = (okey != 0 && key != 0) as i32;
            let mut _u: i32 = 0;
            let nfs = state.pcm.nfs;
            let tv_counter = state.pcm.tv_counter;
            calc_tv(
                nfs,
                tv_counter,
                1,
                state.pcm.ram2[30][0] as i32,
                &mut state.pcm.ram2[30][9],
                active,
                Some(&mut _u),
            );
        }

        {
            let v1 = state.pcm.ram2[30][1] as i32;
            let m1 = multi(state.pcm.ram1[29][0] as i32, (v1 >> 8) as i8) >> 5; // 17
            let m2 = multi(state.pcm.rcsum[0], (v1 & 255) as i8) >> 5; // 18

            state.pcm.ram1[29][0] =
                addclip20((m1 >> 1) as u32, (m2 >> 1) as u32, ((m1 | m2) & 1) as u32);
            // 19
        }

        let mut rcadd: [i32; 6] = [0; 6];
        let mut rcadd2: [i32; 6] = [0; 6];

        {
            {
                // 1
                let v1 = state.pcm.ram2[30][4] as i32;
                let m1 = multi(state.pcm.ram1[29][0] as i32, (v1 >> 8) as i8) >> 6;
                let mut v2 = 0i32;
                let tv_counter = state.pcm.tv_counter as i32;
                let s1 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[28][1] as i32).wrapping_add(tv_counter),
                    1,
                );
                let s2 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[28][1] as i32).wrapping_add(tv_counter),
                    0,
                );
                if (v1 & 0x30) != 0 {
                    v2 = s1;
                }
                let v3 = addclip20(m1 as u32, (v2 ^ 0xFFFFF) as u32, 1);
                state.pcm.ram1[29][4] = v3;
                let m2 = multi(v3 as i32, (v1 & 255) as i8) >> 5;
                state.pcm.ram1[29][5] = addclip20((m2 >> 1) as u32, s2 as u32, (m2 & 1) as u32);
            }
            {
                // 2
                let v1 = state.pcm.ram2[30][4] as i32;
                let mut v2 = 0i32;
                let tv_counter = state.pcm.tv_counter as i32;
                let s1 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[28][2] as i32).wrapping_add(tv_counter),
                    1,
                );
                let s2 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[28][2] as i32).wrapping_add(tv_counter),
                    0,
                );
                if (v1 & 0x30) != 0 {
                    v2 = s1;
                }
                let v3 = addclip20(state.pcm.ram1[29][5], (v2 ^ 0xFFFFF) as u32, 1);
                state.pcm.ram1[29][5] = v3;
                let m2 = multi(v3 as i32, (v1 & 255) as i8) >> 5;
                state.pcm.ram1[28][0] = addclip20((m2 >> 1) as u32, s2 as u32, (m2 & 1) as u32);
            }
            {
                // 3
                let v1 = state.pcm.ram2[30][4] as i32;
                let mut v2 = 0i32;
                let tv_counter = state.pcm.tv_counter as i32;
                let s1 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[28][3] as i32).wrapping_add(tv_counter),
                    1,
                );
                let s2 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[28][3] as i32).wrapping_add(tv_counter),
                    0,
                );
                if (v1 & 0x30) != 0 {
                    v2 = s1;
                }
                let v3 = addclip20(state.pcm.ram1[28][0], (v2 ^ 0xFFFFF) as u32, 1);
                state.pcm.ram1[28][0] = v3;
                let m2 = multi(v3 as i32, (v1 & 255) as i8) >> 5;
                state.pcm.ram1[28][1] = addclip20((m2 >> 1) as u32, s2 as u32, (m2 & 1) as u32);

                state.pcm.ram1[28][2] = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[28][5] as i32).wrapping_add(tv_counter),
                    0,
                ) as u32;
            }
            {
                // 4
                let v1 = state.pcm.ram2[30][5] as i32;
                let mut v2 = 0i32;
                let tv_counter = state.pcm.tv_counter as i32;
                let s1 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[28][4] as i32).wrapping_add(tv_counter),
                    1,
                );
                let s2 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[28][4] as i32).wrapping_add(tv_counter),
                    0,
                );
                if (v1 & 0x30) != 0 {
                    v2 = s1;
                }
                let v3 = addclip20(state.pcm.ram1[28][1], (v2 ^ 0xFFFFF) as u32, 1);
                state.pcm.ram1[28][1] = v3;
                let m2 = multi(v3 as i32, (v1 & 255) as i8) >> 5;
                state.pcm.ram1[28][3] = addclip20((m2 >> 1) as u32, s2 as u32, (m2 & 1) as u32);

                state.pcm.ram1[28][4] = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[29][1] as i32).wrapping_add(tv_counter),
                    0,
                ) as u32;
            }
            {
                // 5

                let v1 = state.pcm.ram2[30][7] as i32;
                let m1 = multi(state.pcm.ram1[29][2] as i32, (v1 >> 8) as i8) >> 5;
                let tv_counter = state.pcm.tv_counter as i32;
                let s1 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[29][0] as i32).wrapping_add(tv_counter),
                    0,
                );
                let m2 = multi(s1, (v1 & 255) as i8) >> 5;
                state.pcm.ram1[29][2] =
                    addclip20((m1 >> 1) as u32, (m2 >> 1) as u32, ((m1 | m2) & 1) as u32);

                eram_pack(
                    &mut state.pcm.eram,
                    (state.pcm.ram2[28][0] as i32).wrapping_add(tv_counter),
                    state.pcm.ram1[29][4] as i32,
                );
            }
            {
                // 6

                let v1 = state.pcm.ram2[30][8] as i32;
                let m1 = multi(state.pcm.ram1[29][3] as i32, (v1 >> 8) as i8) >> 5;
                let tv_counter = state.pcm.tv_counter as i32;
                let s1 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[29][8] as i32).wrapping_add(tv_counter),
                    0,
                );
                let m2 = multi(s1, (v1 & 255) as i8) >> 5;
                state.pcm.ram1[29][3] =
                    addclip20((m1 >> 1) as u32, (m2 >> 1) as u32, ((m1 | m2) & 1) as u32);

                eram_pack(
                    &mut state.pcm.eram,
                    (state.pcm.ram2[28][1] as i32).wrapping_add(tv_counter),
                    state.pcm.ram1[29][5] as i32,
                );

                eram_pack(
                    &mut state.pcm.eram,
                    (state.pcm.ram2[28][2] as i32).wrapping_add(tv_counter),
                    state.pcm.ram1[28][0] as i32,
                );
            }
            {
                // 7

                let v1 = state.pcm.ram2[30][9] as i32;
                let v2 = state.pcm.ram1[28][3];
                let m1 = multi(state.pcm.ram1[29][2] as i32, (v1 >> 8) as i8) >> 5;
                let m2 = multi(state.pcm.ram1[29][3] as i32, (v1 >> 8) as i8) >> 5;
                state.pcm.ram1[28][3] = addclip20(v2, (m1 >> 1) as u32, (m1 & 1) as u32);
                state.pcm.ram1[28][5] = addclip20(v2, (m2 >> 1) as u32, (m2 & 1) as u32);

                let tv_counter = state.pcm.tv_counter as i32;
                eram_pack(
                    &mut state.pcm.eram,
                    (state.pcm.ram2[28][3] as i32).wrapping_add(tv_counter),
                    state.pcm.ram1[28][1] as i32,
                );
            }
            {
                // 8

                let v1 = state.pcm.ram2[30][6] as i32;
                let m1 = multi(state.pcm.ram1[28][2] as i32, (v1 >> 8) as i8) >> 5;

                let v2 = addclip20(state.pcm.ram1[28][3], (m1 >> 1) as u32, (m1 & 1) as u32);
                state.pcm.ram1[28][3] = v2;
                let m2 = multi(v2 as i32, (v1 & 255) as i8) >> 5;
                state.pcm.ram1[28][2] =
                    addclip20(state.pcm.ram1[28][2], (m2 >> 1) as u32, (m2 & 1) as u32);

                let tv_counter = state.pcm.tv_counter as i32;
                state.pcm.ram1[28][1] = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[28][9] as i32).wrapping_add(tv_counter),
                    0,
                ) as u32;
            }
            {
                // 9

                let v1 = state.pcm.ram2[30][6] as i32;
                let m1 = multi(state.pcm.ram1[28][4] as i32, (v1 >> 8) as i8) >> 5;

                let v2 = addclip20(state.pcm.ram1[28][5], (m1 >> 1) as u32, (m1 & 1) as u32);
                state.pcm.ram1[28][5] = v2;
                let m2 = multi(v2 as i32, (v1 & 255) as i8) >> 5;
                state.pcm.ram1[28][4] =
                    addclip20(state.pcm.ram1[28][4], (m2 >> 1) as u32, (m2 & 1) as u32);

                let tv_counter = state.pcm.tv_counter as i32;
                state.pcm.ram1[29][4] = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[29][5] as i32).wrapping_add(tv_counter),
                    0,
                ) as u32;
            }
            {
                // 10

                let v1 = state.pcm.ram2[30][6] as i32;
                let v2 = state.pcm.ram1[28][1] as i32;
                let m1 = multi(v2, (v1 >> 8) as i8) >> 5;
                let tv_counter = state.pcm.tv_counter as i32;
                let s1 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[28][8] as i32).wrapping_add(tv_counter),
                    0,
                );
                let v3 = addclip20((m1 >> 1) as u32, s1 as u32, (m1 & 1) as u32);
                state.pcm.ram1[28][1] = v3;
                let m2 = multi(v3 as i32, (v1 & 255) as i8) >> 5;
                state.pcm.ram1[29][5] = addclip20((m2 >> 1) as u32, v2 as u32, (m2 & 1) as u32);

                eram_pack(
                    &mut state.pcm.eram,
                    (state.pcm.ram2[28][4] as i32).wrapping_add(tv_counter),
                    state.pcm.ram1[28][3] as i32,
                );
            }
            {
                // 11

                let v1 = state.pcm.ram2[30][6] as i32;
                let v2 = state.pcm.ram1[29][4] as i32;
                let m1 = multi(v2, (v1 >> 8) as i8) >> 5;
                let tv_counter = state.pcm.tv_counter as i32;
                let s1 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[29][4] as i32).wrapping_add(tv_counter),
                    0,
                );
                let v3 = addclip20((m1 >> 1) as u32, s1 as u32, (m1 & 1) as u32);
                state.pcm.ram1[29][4] = v3;
                let m2 = multi(v3 as i32, (v1 & 255) as i8) >> 5;
                state.pcm.ram1[28][0] = addclip20((m2 >> 1) as u32, v2 as u32, (m2 & 1) as u32);

                eram_pack(
                    &mut state.pcm.eram,
                    (state.pcm.ram2[28][5] as i32).wrapping_add(tv_counter),
                    state.pcm.ram1[28][2] as i32,
                );

                eram_pack(
                    &mut state.pcm.eram,
                    (state.pcm.ram2[29][0] as i32).wrapping_add(tv_counter),
                    state.pcm.ram1[28][5] as i32,
                );
            }
            {
                // 12

                let tv_counter = state.pcm.tv_counter as i32;
                state.pcm.ram1[28][5] = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[28][6] as i32).wrapping_add(tv_counter),
                    0,
                ) as u32;
            }

            {
                // 13

                let tv_counter = state.pcm.tv_counter as i32;
                let s1 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[28][10] as i32).wrapping_add(tv_counter),
                    0,
                );
                state.pcm.ram1[28][5] = addclip20(state.pcm.ram1[28][5], s1 as u32, 0);

                state.pcm.ram1[28][2] = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[29][2] as i32).wrapping_add(tv_counter),
                    0,
                ) as u32;
            }

            {
                // 14

                let tv_counter = state.pcm.tv_counter as i32;
                let s1 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[29][6] as i32).wrapping_add(tv_counter),
                    0,
                );
                let t1 = addclip20(s1 as u32, state.pcm.ram1[28][2], 0); // 6

                state.pcm.ram1[28][5] = addclip20(t1, state.pcm.ram1[28][5], 0);

                state.pcm.ram1[28][2] = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[28][7] as i32).wrapping_add(tv_counter),
                    0,
                ) as u32;
            }

            {
                // 15

                let tv_counter = state.pcm.tv_counter as i32;
                let s1 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[28][11] as i32).wrapping_add(tv_counter),
                    0,
                );
                state.pcm.ram1[28][2] = addclip20(state.pcm.ram1[28][2], s1 as u32, 0);

                state.pcm.ram1[28][3] = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[29][3] as i32).wrapping_add(tv_counter),
                    0,
                ) as u32;
            }

            {
                // 16

                let tv_counter = state.pcm.tv_counter as i32;
                let s1 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[29][7] as i32).wrapping_add(tv_counter),
                    0,
                );
                let t1 = addclip20(s1 as u32, state.pcm.ram1[28][2], 0);
                state.pcm.ram1[28][2] = addclip20(t1, state.pcm.ram1[28][3], 0);

                eram_pack(
                    &mut state.pcm.eram,
                    (state.pcm.ram2[29][1] as i32).wrapping_add(tv_counter),
                    state.pcm.ram1[28][4] as i32,
                );

                eram_pack(
                    &mut state.pcm.eram,
                    (state.pcm.ram2[28][8] as i32).wrapping_add(tv_counter),
                    state.pcm.ram1[28][1] as i32,
                );
            }

            {
                // 17
                let v1 = state.pcm.ram2[30][2] as i32;
                let v2 = state.pcm.ram1[28][5] as i32;

                let m1 = multi(v2, (v1 >> 8) as i8) >> 5;

                rcadd[0] = m1;

                rcadd2[0] = multi(v2, (v1 & 255) as i8) >> 5;

                let tv_counter = state.pcm.tv_counter as i32;
                let t1 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[29][10] as i32)
                        .wrapping_add(tv_counter)
                        .wrapping_add(1),
                    0,
                ); //? 3a6e
                eram_pack(
                    &mut state.pcm.eram,
                    (state.pcm.ram2[28][9] as i32).wrapping_add(tv_counter),
                    state.pcm.ram1[29][5] as i32,
                );
                state.pcm.ram1[29][5] = t1 as u32;
            }

            {
                // 18
                let v1 = state.pcm.ram2[30][3] as i32;
                let v2 = state.pcm.ram1[28][2] as i32;

                let m1 = multi(v2, (v1 >> 8) as i8) >> 5;

                rcadd[1] = m1;

                rcadd2[1] = multi(v2, (v1 & 255) as i8) >> 5;

                let tv_counter = state.pcm.tv_counter as i32;
                state.pcm.ram1[28][1] = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[29][11] as i32)
                        .wrapping_add(tv_counter)
                        .wrapping_add(1),
                    0,
                ) as u32; //? 3a1e
            }
            {
                // 19

                let v1 = state.pcm.ram2[31][9] as i32;

                let tv_counter = state.pcm.tv_counter as i32;
                let s1 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[29][10] as i32).wrapping_add(tv_counter),
                    0,
                ); //? 3a6d

                eram_pack(
                    &mut state.pcm.eram,
                    (state.pcm.ram2[29][4] as i32).wrapping_add(tv_counter),
                    state.pcm.ram1[29][4] as i32,
                );

                let m1 = multi(s1, (v1 >> 8) as i8) >> 5;
                let m2 = multi(state.pcm.ram1[29][5] as i32, (v1 >> 8) as i8) >> 5;

                let t2 = addclip20(s1 as u32, ((m1 >> 1) ^ 0xFFFFF) as u32, 1);

                state.pcm.ram1[29][5] = addclip20(t2, (m2 >> 1) as u32, (m2 & 1) as u32);
            }
            {
                // 20

                let v1 = state.pcm.ram2[31][10] as i32;

                let tv_counter = state.pcm.tv_counter as i32;
                let s1 = eram_unpack(
                    &state.pcm.eram,
                    (state.pcm.ram2[29][11] as i32).wrapping_add(tv_counter),
                    0,
                ); //? 3a1d

                eram_pack(
                    &mut state.pcm.eram,
                    (state.pcm.ram2[29][5] as i32).wrapping_add(tv_counter),
                    state.pcm.ram1[28][0] as i32,
                );

                let m1 = multi(s1, (v1 >> 8) as i8) >> 5;
                let m2 = multi(state.pcm.ram1[28][1] as i32, (v1 >> 8) as i8) >> 5;

                let t2 = addclip20(s1 as u32, ((m1 >> 1) ^ 0xFFFFF) as u32, 1);

                state.pcm.ram1[28][1] = addclip20(t2, (m2 >> 1) as u32, (m2 & 1) as u32);

                eram_pack(
                    &mut state.pcm.eram,
                    (state.pcm.ram2[29][9] as i32).wrapping_add(tv_counter),
                    state.pcm.ram1[29][1] as i32,
                );
            }
            {
                // 21

                let v1 = state.pcm.ram2[31][2] as i32;
                let v2 = state.pcm.ram1[29][5] as i32;

                let m1 = multi(v2, (v1 >> 8) as i8) >> 5;
                let m2 = multi(v2, (v1 & 255) as i8) >> 5;

                rcadd[2] = m1;
                rcadd2[2] = m2;
            }
            {
                // 22

                let v1 = state.pcm.ram2[31][3] as i32;
                let v2 = state.pcm.ram1[29][5] as i32;

                let m1 = multi(v2, (v1 >> 8) as i8) >> 5;
                let m2 = multi(v2, (v1 & 255) as i8) >> 5;

                rcadd[3] = m1;
                rcadd2[3] = m2;
            }
            {
                // 23

                let v1 = state.pcm.ram2[31][4] as i32;
                let v2 = state.pcm.ram1[28][1] as i32;

                let m1 = multi(v2, (v1 >> 8) as i8) >> 5;
                let m2 = multi(v2, (v1 & 255) as i8) >> 5;

                rcadd[4] = m1;
                rcadd2[4] = m2;
            }
            {
                // 31

                let v1 = state.pcm.ram2[31][5] as i32;
                let v2 = state.pcm.ram1[28][1] as i32;

                let m1 = multi(v2, (v1 >> 8) as i8) >> 5;
                let m2 = multi(v2, (v1 & 255) as i8) >> 5;

                rcadd[5] = m1;
                rcadd2[5] = m2;

                {
                    // address generator

                    let key = 1i32;
                    let okey = ((state.pcm.ram2[31][7] & 0x20) != 0) as i32;
                    let active = (key != 0 && okey != 0) as i32;
                    let kon = (key != 0 && okey == 0) as i32;

                    let mut b15 = ((state.pcm.ram2[31][8] & 0x8000) != 0) as i32; // 0
                    let b6 = ((state.pcm.ram2[31][7] & 0x40) != 0) as i32; // 1
                    let b7 = ((state.pcm.ram2[31][7] & 0x80) != 0) as i32; // 1
                    let _old_nibble = ((state.pcm.ram2[31][7] >> 12) & 15) as i32; // 1

                    let address = state.pcm.ram1[31][4] as i32; // 0
                    let address_end = state.pcm.ram1[31][0] as i32; // 1 or 2
                    let address_loop = state.pcm.ram1[31][2] as i32; // 2 or 1

                    let mut sub_phase = (state.pcm.ram2[31][8] & 0x3FFF) as i32; // 1
                    let _interp_ratio = (sub_phase >> 7) & 127;
                    sub_phase = sub_phase.wrapping_add(
                        state.pcm.ram2[(state.pcm.ram2[31][7] & 31) as usize][0] as i32,
                    ); // 5
                    let sub_phase_of = (sub_phase >> 14) & 7;
                    if state.pcm.nfs != 0 {
                        state.pcm.ram2[31][8] &= !0x3FFF;
                        state.pcm.ram2[31][8] |= (sub_phase & 0x3FFF) as u16;
                    }

                    // address 0
                    let mut address_cnt = address;

                    let cmp1 = if b15 != 0 { address_loop } else { address_end };
                    let cmp2 = address_cnt;
                    let mut address_cmp = ((cmp1 & 0xFFFFF) == (cmp2 & 0xFFFFF)) as i32; // 9
                    let mut next_b15 = b15;

                    let mut next_address = address_cnt; // 11

                    let cmp1 = if b6 == 0 && address_cmp != 0 {
                        address_loop
                    } else {
                        address_cnt
                    };
                    let cmp2 = address_cnt;
                    let mut address_cnt2 = if kon != 0 || (b6 == 0 && address_cmp != 0) {
                        cmp1
                    } else {
                        cmp2
                    };

                    let address_add = !(address_cmp != 0 || b6 != 0 && b15 != 0) as i32;
                    let address_sub = (address_cmp == 0 && b6 != 0 && b15 != 0) as i32;
                    if b7 != 0 {
                        address_cnt2 -= address_add - address_sub;
                    } else {
                        address_cnt2 += address_add - address_sub;
                    }
                    address_cnt = address_cnt2 & 0xFFFFF; // 11
                    b15 = (b6 != 0 && (b15 ^ address_cmp) != 0) as i32; // 11

                    let cmp1 = if b15 != 0 { address_loop } else { address_end };
                    let cmp2 = address_cnt;
                    address_cmp = ((cmp1 & 0xFFFFF) == (cmp2 & 0xFFFFF)) as i32; // 13
                    let _ = address_cmp;

                    if sub_phase_of >= 1 {
                        next_address = address_cnt; // 13
                        next_b15 = b15;
                    }

                    if active != 0 && state.pcm.nfs != 0 {
                        state.pcm.ram1[31][4] = next_address as u32;
                    }

                    if state.pcm.nfs != 0 {
                        state.pcm.ram2[31][8] &= !0x8000;
                        state.pcm.ram2[31][8] |= (next_b15 << 15) as u16;
                    }

                    let t1 = address_loop; // 18
                    let t2 = (state.pcm.ram1[31][4] as i32).wrapping_sub(t1); // 19
                    let t3 = address_end.wrapping_sub(t2); // 20
                    let t4 = state.pcm.ram1[31][4] as i32; // 23

                    state.pcm.ram2[29][10] = t3 as u16;
                    state.pcm.ram2[29][11] = t4 as u16;
                }
            }
        }

        state.pcm.ram1[31][1] = 0;
        state.pcm.ram1[31][3] = 0;
        state.pcm.rcsum[0] = 0;
        state.pcm.rcsum[1] = 0;

        for slot in 0..reg_slots {
            let slot = slot as usize;
            let okey = ((state.pcm.ram2[slot][7] & 0x20) != 0) as i32;
            let key = ((voice_active >> slot) & 1) as i32;

            let active = (okey != 0 && key != 0) as i32;
            let kon = (key != 0 && okey == 0) as i32;

            // address generator

            let mut b15 = ((state.pcm.ram2[slot][8] & 0x8000) != 0) as i32; // 0
            let b6 = ((state.pcm.ram2[slot][7] & 0x40) != 0) as i32; // 1
            let b7 = ((state.pcm.ram2[slot][7] & 0x80) != 0) as i32; // 1
            let hiaddr = ((state.pcm.ram2[slot][7] >> 8) & 15) as i32; // 1
            let old_nibble = ((state.pcm.ram2[slot][7] >> 12) & 15) as i32; // 1

            let address = state.pcm.ram1[slot][4] as i32; // 0
            let address_end = state.pcm.ram1[slot][0] as i32; // 1 or 2
            let address_loop = state.pcm.ram1[slot][2] as i32; // 2 or 1

            let cmp1 = if b15 != 0 { address_loop } else { address_end };
            let cmp2 = address;
            let nibble_cmp1 = ((cmp1 & 0xFFFF0) == (cmp2 & 0xFFFF0)) as i32; // 2

            // fixme:
            let irq_flag = if kon != 0 {
                (((cmp1.wrapping_add(address_loop) as u32) & 0x100000) != 0) as i32
            } else {
                ((address.wrapping_add(address_loop.wrapping_neg() & 0xFFFFF) as u32) & 0x100000
                    != 0) as i32
            };
            let irq_flag = irq_flag ^ b7;

            let nibble_address = if b6 == 0 && nibble_cmp1 != 0 {
                address_loop
            } else {
                address
            }; // 3
            let address_b4 = ((nibble_address & 0x10) != 0) as i32;
            let mut wave_address = nibble_address >> 5;
            let xor2 = address_b4 ^ b7;
            let check1 = (xor2 != 0 && active != 0) as i32;
            let xor1 = b15 ^ (nibble_cmp1 == 0) as i32;
            let nibble_add = if b6 != 0 {
                (check1 != 0 && xor1 != 0) as i32
            } else {
                (nibble_cmp1 == 0 && check1 != 0) as i32
            };
            let nibble_subtract = (b6 != 0 && xor1 == 0 && active != 0 && xor2 == 0) as i32;
            if b7 != 0 {
                wave_address -= nibble_add - nibble_subtract;
            } else {
                wave_address += nibble_add - nibble_subtract;
            }
            wave_address &= 0xFFFFF;

            let mut newnibble =
                pcm_read_rom_addr(state, ((hiaddr << 20) | wave_address) as u32) as i32;
            let newnibble_sel = address_b4 ^ (((b6 != 0 || nibble_cmp1 == 0) && okey != 0) as i32);
            if newnibble_sel != 0 {
                newnibble = (newnibble >> 4) & 15;
            } else {
                newnibble &= 15;
            }

            let mut sub_phase = (state.pcm.ram2[slot][8] & 0x3FFF) as i32; // 1
            let interp_ratio = ((sub_phase >> 7) & 127) as usize;
            sub_phase = sub_phase
                .wrapping_add(state.pcm.ram2[(state.pcm.ram2[slot][7] & 31) as usize][0] as i32); // 5
            let sub_phase_of = (sub_phase >> 14) & 7;
            if state.pcm.nfs != 0 {
                state.pcm.ram2[slot][8] &= !0x3FFF;
                state.pcm.ram2[slot][8] |= (sub_phase & 0x3FFF) as u16;
            }

            // address 0
            let mut address_cnt = address;
            let samp0 =
                pcm_read_rom_addr(state, ((hiaddr << 20) | address_cnt) as u32) as i8 as i32; // 18

            let cmp1 = address;
            let cmp2 = address_cnt;
            let nibble_cmp2 = ((cmp1 & 0xFFFF0) == (cmp2 & 0xFFFF0)) as i32; // 8
            let cmp1 = if b15 != 0 { address_loop } else { address_end };
            let cmp2 = address_cnt;
            let mut address_cmp = ((cmp1 & 0xFFFFF) == (cmp2 & 0xFFFFF)) as i32; // 9

            let mut next_address = address_cnt; // 11
            let mut usenew = (nibble_cmp2 == 0) as i32;
            let mut next_b15 = b15;

            let cmp1 = if b6 == 0 && address_cmp != 0 {
                address_loop
            } else {
                address_cnt
            };
            let cmp2 = address_cnt;
            let mut address_cnt2 = if kon != 0 || (b6 == 0 && address_cmp != 0) {
                cmp1
            } else {
                cmp2
            };

            let mut address_add = !(address_cmp != 0 || b6 != 0 && b15 != 0) as i32;
            let mut address_sub = (address_cmp == 0 && b6 != 0 && b15 != 0) as i32;
            if b7 != 0 {
                address_cnt2 -= address_add - address_sub;
            } else {
                address_cnt2 += address_add - address_sub;
            }
            address_cnt = address_cnt2 & 0xFFFFF; // 11
            b15 = (b6 != 0 && (b15 ^ address_cmp) != 0) as i32; // 11

            let samp1 =
                pcm_read_rom_addr(state, ((hiaddr << 20) | address_cnt) as u32) as i8 as i32; // 20

            let cmp1 = address;
            let cmp2 = address_cnt;
            let nibble_cmp3 = ((cmp1 & 0xFFFF0) == (cmp2 & 0xFFFF0)) as i32; // 12
            let cmp1 = if b15 != 0 { address_loop } else { address_end };
            let cmp2 = address_cnt;
            address_cmp = ((cmp1 & 0xFFFFF) == (cmp2 & 0xFFFFF)) as i32; // 13

            if sub_phase_of >= 1 {
                next_address = address_cnt; // 13
                usenew = (nibble_cmp3 == 0) as i32;
                next_b15 = b15;
            }

            let cmp1 = if b6 == 0 && address_cmp != 0 {
                address_loop
            } else {
                address_cnt
            };
            let cmp2 = address_cnt;
            address_cnt2 = if kon != 0 || (b6 == 0 && address_cmp != 0) {
                cmp1
            } else {
                cmp2
            };

            address_add = !(address_cmp != 0 || b6 != 0 && b15 != 0) as i32;
            address_sub = (address_cmp == 0 && b6 != 0 && b15 != 0) as i32;
            if b7 != 0 {
                address_cnt2 -= address_add - address_sub;
            } else {
                address_cnt2 += address_add - address_sub;
            }
            address_cnt = address_cnt2 & 0xFFFFF; // 15
            b15 = (b6 != 0 && (b15 ^ address_cmp) != 0) as i32; // 15

            let samp2 =
                pcm_read_rom_addr(state, ((hiaddr << 20) | address_cnt) as u32) as i8 as i32; // 1

            let cmp1 = address;
            let cmp2 = address_cnt;
            let nibble_cmp4 = ((cmp1 & 0xFFFF0) == (cmp2 & 0xFFFF0)) as i32; // 16
            let cmp1 = if b15 != 0 { address_loop } else { address_end };
            let cmp2 = address_cnt;
            address_cmp = ((cmp1 & 0xFFFFF) == (cmp2 & 0xFFFFF)) as i32; // 17

            if sub_phase_of >= 2 {
                next_address = address_cnt; // 17
                usenew = (nibble_cmp4 == 0) as i32;
                next_b15 = b15;
            }

            let cmp1 = if b6 == 0 && address_cmp != 0 {
                address_loop
            } else {
                address_cnt
            };
            let cmp2 = address_cnt;
            address_cnt2 = if kon != 0 || (b6 == 0 && address_cmp != 0) {
                cmp1
            } else {
                cmp2
            };

            address_add = !(address_cmp != 0 || b6 != 0 && b15 != 0) as i32;
            address_sub = (address_cmp == 0 && b6 != 0 && b15 != 0) as i32;
            if b7 != 0 {
                address_cnt2 -= address_add - address_sub;
            } else {
                address_cnt2 += address_add - address_sub;
            }
            address_cnt = address_cnt2 & 0xFFFFF; // 19
            b15 = (b6 != 0 && (b15 ^ address_cmp) != 0) as i32; // 19

            let samp3 =
                pcm_read_rom_addr(state, ((hiaddr << 20) | address_cnt) as u32) as i8 as i32; // 5

            let cmp1 = address;
            let cmp2 = address_cnt;
            let nibble_cmp5 = ((cmp1 & 0xFFFF0) == (cmp2 & 0xFFFF0)) as i32; // 20
            let cmp1 = if b15 != 0 { address_loop } else { address_end };
            let cmp2 = address_cnt;
            address_cmp = ((cmp1 & 0xFFFFF) == (cmp2 & 0xFFFFF)) as i32; // 21

            if sub_phase_of >= 3 {
                next_address = address_cnt; // 21
                usenew = (nibble_cmp5 == 0) as i32;
                next_b15 = b15;
            }

            let cmp1 = if b6 == 0 && address_cmp != 0 {
                address_loop
            } else {
                address_cnt
            };
            let cmp2 = address_cnt;
            address_cnt2 = if kon != 0 || (b6 == 0 && address_cmp != 0) {
                cmp1
            } else {
                cmp2
            };

            address_add = !(address_cmp != 0 || b6 != 0 && b15 != 0) as i32;
            address_sub = (address_cmp == 0 && b6 != 0 && b15 != 0) as i32;
            if b7 != 0 {
                address_cnt2 -= address_add - address_sub;
            } else {
                address_cnt2 += address_add - address_sub;
            }
            address_cnt = address_cnt2 & 0xFFFFF; // 23
            // b15 = b6 && (b15 ^ address_cmp); // 23

            let cmp1 = address;
            let cmp2 = address_cnt;
            let nibble_cmp6 = ((cmp1 & 0xFFFF0) == (cmp2 & 0xFFFF0)) as i32; // 24

            if sub_phase_of >= 4 {
                next_address = address_cnt; // 1
                usenew = (nibble_cmp6 == 0) as i32;
                // b15 is not updated?
            }

            if active != 0 && state.pcm.nfs != 0 {
                state.pcm.ram1[slot][4] = next_address as u32;
            }

            if state.pcm.nfs != 0 {
                state.pcm.ram2[slot][8] &= !0x8000;
                state.pcm.ram2[slot][8] |= (next_b15 << 15) as u16;
            }

            // dpcm

            // 18
            let mut reference = state.pcm.ram1[slot][5] as i32;

            // 19
            let mut preshift = samp0 << 10;
            let mut select_nibble = if nibble_cmp2 != 0 {
                old_nibble
            } else {
                newnibble
            };
            let mut shift = (10i32.wrapping_sub(select_nibble)) & 15;

            let mut shifted = (preshift << 1) >> shift;

            if sub_phase_of >= 1 {
                reference = addclip20(
                    reference as u32,
                    (shifted >> 1) as u32,
                    (shifted & 1) as u32,
                ) as i32;
            }

            preshift = samp1 << 10;
            select_nibble = if nibble_cmp3 != 0 {
                old_nibble
            } else {
                newnibble
            };
            shift = (10i32.wrapping_sub(select_nibble)) & 15;

            shifted = (preshift << 1) >> shift;

            if sub_phase_of >= 2 {
                reference = addclip20(
                    reference as u32,
                    (shifted >> 1) as u32,
                    (shifted & 1) as u32,
                ) as i32;
            }

            preshift = samp2 << 10;
            select_nibble = if nibble_cmp4 != 0 {
                old_nibble
            } else {
                newnibble
            };
            shift = (10i32.wrapping_sub(select_nibble)) & 15;

            shifted = (preshift << 1) >> shift;

            if sub_phase_of >= 3 {
                reference = addclip20(
                    reference as u32,
                    (shifted >> 1) as u32,
                    (shifted & 1) as u32,
                ) as i32;
            }

            preshift = samp3 << 10;
            select_nibble = if nibble_cmp5 != 0 {
                old_nibble
            } else {
                newnibble
            };
            shift = (10i32.wrapping_sub(select_nibble)) & 15;

            shifted = (preshift << 1) >> shift;

            if sub_phase_of >= 4 {
                reference = addclip20(
                    reference as u32,
                    (shifted >> 1) as u32,
                    (shifted & 1) as u32,
                ) as i32;
            }

            // interpolation

            let mut test = state.pcm.ram1[slot][5] as i32;

            let step0 = multi(INTERP_LUT[0][interp_ratio] << 6, samp0 as i8) >> 8;
            select_nibble = if nibble_cmp2 != 0 {
                old_nibble
            } else {
                newnibble
            };
            shift = (10i32.wrapping_sub(select_nibble)) & 15;
            let step0 = (step0 << 1) >> shift;

            test = addclip20(test as u32, (step0 >> 1) as u32, (step0 & 1) as u32) as i32;

            let step1 = multi(INTERP_LUT[1][interp_ratio] << 6, samp1 as i8) >> 8;
            select_nibble = if nibble_cmp3 != 0 {
                old_nibble
            } else {
                newnibble
            };
            shift = (10i32.wrapping_sub(select_nibble)) & 15;
            let step1 = (step1 << 1) >> shift;

            test = addclip20(test as u32, (step1 >> 1) as u32, (step1 & 1) as u32) as i32;

            let step2 = multi(INTERP_LUT[2][interp_ratio] << 6, samp2 as i8) >> 8;
            select_nibble = if nibble_cmp4 != 0 {
                old_nibble
            } else {
                newnibble
            };
            shift = (10i32.wrapping_sub(select_nibble)) & 15;
            let step2 = (step2 << 1) >> shift;

            let reg1 = state.pcm.ram1[slot][1] as i32;
            let reg3 = state.pcm.ram1[slot][3] as i32;
            let reg2_6 = ((state.pcm.ram2[slot][6] >> 8) & 127) as i32;

            test = addclip20(test as u32, (step2 >> 1) as u32, (step2 & 1) as u32) as i32;

            let filter = state.pcm.ram2[slot][11] as i32;
            let v3: i32;

            if state.mcu_mk1 {
                let mult1 = multi(reg1, (filter >> 8) as i8); // 8
                let mult2 = multi(reg1, ((filter >> 1) & 127) as i8); // 9
                let mult3 = multi(reg1, reg2_6 as i8); // 10

                let v2 =
                    addclip20(reg3 as u32, (mult1 >> 6) as u32, ((mult1 >> 5) & 1) as u32) as i32; // 9
                let v1 =
                    addclip20(v2 as u32, (mult2 >> 13) as u32, ((mult2 >> 12) & 1) as u32) as i32; // 10
                let subvar =
                    addclip20(v1 as u32, (mult3 >> 6) as u32, ((mult3 >> 5) & 1) as u32) as i32; // 11

                state.pcm.ram1[slot][3] = v1 as u32;

                v3 = addclip20(test as u32, (subvar ^ 0xFFFFF) as u32, 1) as i32; // 12

                let mult4 = multi(v3, (filter >> 8) as i8);
                let mult5 = multi(v3, ((filter >> 1) & 127) as i8);
                let v4 =
                    addclip20(reg1 as u32, (mult4 >> 6) as u32, ((mult4 >> 5) & 1) as u32) as i32; // 14
                let v5 =
                    addclip20(v4 as u32, (mult5 >> 13) as u32, ((mult5 >> 12) & 1) as u32) as i32; // 15

                state.pcm.ram1[slot][1] = v5 as u32;
            } else {
                // hack: use 32-bit math to avoid overflow
                let mult1 = reg1.wrapping_mul((filter >> 8) as i8 as i32); // 8
                let mult2 = reg1.wrapping_mul(((filter >> 1) & 127) as i8 as i32); // 9
                let mult3 = reg1.wrapping_mul(reg2_6 as i8 as i32); // 10

                let v2 = reg3.wrapping_add(mult1 >> 6).wrapping_add((mult1 >> 5) & 1); // 9
                let v1 = v2.wrapping_add(mult2 >> 13).wrapping_add((mult2 >> 12) & 1); // 10
                let subvar = v1.wrapping_add(mult3 >> 6).wrapping_add((mult3 >> 5) & 1); // 11

                state.pcm.ram1[slot][3] = v1 as u32;

                let mut tests = test;
                tests <<= 12;
                tests >>= 12;

                v3 = tests.wrapping_sub(subvar); // 12

                let mult4 = v3.wrapping_mul((filter >> 8) as i8 as i32);
                let mult5 = v3.wrapping_mul(((filter >> 1) & 127) as i8 as i32);
                let v4 = reg1.wrapping_add(mult4 >> 6).wrapping_add((mult4 >> 5) & 1); // 14
                let v5 = v4.wrapping_add(mult5 >> 13).wrapping_add((mult5 >> 12) & 1); // 15

                state.pcm.ram1[slot][1] = v5 as u32;
            }

            state.pcm.ram1[slot][5] = reference as u32;

            if active != 0
                && (state.pcm.ram2[slot][6] & 1) != 0
                && (state.pcm.ram2[slot][8] & 0x4000) == 0
                && state.pcm.irq_assert == 0
                && irq_flag != 0
            {
                //printf("irq voice %i\n", slot);
                if state.pcm.nfs != 0 {
                    state.pcm.ram2[slot][8] |= 0x4000;
                }
                state.pcm.irq_assert = 1;
                state.pcm.irq_channel = slot as u32;
                if state.mcu_jv880 {
                    crate::mcu::mcu_ga_set_ga_int(state, 5, 1);
                } else {
                    crate::mcu_interrupt::mcu_interrupt_set_request(
                        state,
                        INTERRUPT_SOURCE_IRQ0,
                        1,
                    );
                }
            }

            let mut volmul1: i32 = 0;
            let mut volmul2: i32 = 0;

            {
                let nfs = state.pcm.nfs;
                let tv_counter = state.pcm.tv_counter;
                let adjust = state.pcm.ram2[slot][3] as i32;
                calc_tv(
                    nfs,
                    tv_counter,
                    0,
                    adjust,
                    &mut state.pcm.ram2[slot][9],
                    active,
                    Some(&mut volmul1),
                );
            }
            {
                let nfs = state.pcm.nfs;
                let tv_counter = state.pcm.tv_counter;
                let adjust = state.pcm.ram2[slot][4] as i32;
                calc_tv(
                    nfs,
                    tv_counter,
                    1,
                    adjust,
                    &mut state.pcm.ram2[slot][10],
                    active,
                    Some(&mut volmul2),
                );
            }
            {
                let nfs = state.pcm.nfs;
                let tv_counter = state.pcm.tv_counter;
                let adjust = state.pcm.ram2[slot][5] as i32;
                calc_tv(
                    nfs,
                    tv_counter,
                    2,
                    adjust,
                    &mut state.pcm.ram2[slot][11],
                    active,
                    None,
                );
            }

            // if (volmul1 && volmul2)
            //     volmul1 += 0;

            let sample = if (state.pcm.ram2[slot][6] & 2) == 0 {
                state.pcm.ram1[slot][3] as i32
            } else {
                v3
            };
            //sample = test;

            let multiv1 = multi(sample, (volmul1 >> 8) as i8);
            let multiv2 = multi(sample, ((volmul1 >> 1) & 127) as i8);

            let sample2 = addclip20(
                (multiv1 >> 6) as u32,
                (multiv2 >> 13) as u32,
                (((multiv2 >> 12) | (multiv1 >> 5)) & 1) as u32,
            ) as i32;

            let multiv3 = multi(sample2, (volmul2 >> 8) as i8);
            let multiv4 = multi(sample2, ((volmul2 >> 1) & 127) as i8);

            let sample3 = addclip20(
                (multiv3 >> 6) as u32,
                (multiv4 >> 13) as u32,
                (((multiv4 >> 12) | (multiv3 >> 5)) & 1) as u32,
            ) as i32;

            let pan = if active != 0 {
                state.pcm.ram2[slot][1] as i32
            } else {
                0
            };
            let rc = if active != 0 {
                state.pcm.ram2[slot][2] as i32
            } else {
                0
            };

            let sampl = multi(sample3, ((pan >> 8) & 255) as i8);
            let sampr = multi(sample3, ((pan >> 0) & 255) as i8);

            let rc0 = multi(sample3, ((rc >> 8) & 255) as i8) >> 5; // reverb
            let rc1 = multi(sample3, ((rc >> 0) & 255) as i8) >> 5; // chorus

            // mix reverb/chorus?
            let slot2 = if slot as i32 == reg_slots - 1 {
                31
            } else {
                slot as i32 + 1
            };
            match slot2 {
                // 17, 18 - reverb
                17 => {
                    state.pcm.ram1[31][1] = addclip20(
                        state.pcm.ram1[31][1],
                        (rcadd[0] >> 1) as u32,
                        (rcadd[0] & 1) as u32,
                    );
                }
                18 => {
                    state.pcm.ram1[31][3] = addclip20(
                        state.pcm.ram1[31][3],
                        (rcadd[1] >> 1) as u32,
                        (rcadd[1] & 1) as u32,
                    );
                }
                21 => {
                    state.pcm.ram1[31][1] = addclip20(
                        state.pcm.ram1[31][1],
                        (rcadd[2] >> 1) as u32,
                        (rcadd[2] & 1) as u32,
                    );
                }
                22 => {
                    state.pcm.ram1[31][3] = addclip20(
                        state.pcm.ram1[31][3],
                        (rcadd[3] >> 1) as u32,
                        (rcadd[3] & 1) as u32,
                    );
                }
                23 => {
                    state.pcm.ram1[31][1] = addclip20(
                        state.pcm.ram1[31][1],
                        (rcadd[4] >> 1) as u32,
                        (rcadd[4] & 1) as u32,
                    );
                }
                31 => {
                    state.pcm.ram1[31][3] = addclip20(
                        state.pcm.ram1[31][3],
                        (rcadd[5] >> 1) as u32,
                        (rcadd[5] & 1) as u32,
                    );
                }
                _ => {}
            }

            let suml = addclip20(
                state.pcm.ram1[31][1],
                (sampl >> 6) as u32,
                ((sampl >> 5) & 1) as u32,
            );
            let sumr = addclip20(
                state.pcm.ram1[31][3],
                (sampr >> 6) as u32,
                ((sampr >> 5) & 1) as u32,
            );

            match slot2 {
                17 => {
                    state.pcm.rcsum[1] = addclip20(
                        state.pcm.rcsum[1] as u32,
                        (rcadd2[0] >> 1) as u32,
                        (rcadd2[0] & 1) as u32,
                    ) as i32;
                }
                18 => {
                    state.pcm.rcsum[1] = addclip20(
                        state.pcm.rcsum[1] as u32,
                        (rcadd2[1] >> 1) as u32,
                        (rcadd2[1] & 1) as u32,
                    ) as i32;
                }
                21 => {
                    state.pcm.rcsum[0] = addclip20(
                        state.pcm.rcsum[0] as u32,
                        (rcadd2[2] >> 1) as u32,
                        (rcadd2[2] & 1) as u32,
                    ) as i32;
                }
                22 => {
                    state.pcm.rcsum[1] = addclip20(
                        state.pcm.rcsum[1] as u32,
                        (rcadd2[3] >> 1) as u32,
                        (rcadd2[3] & 1) as u32,
                    ) as i32;
                }
                23 => {
                    state.pcm.rcsum[0] = addclip20(
                        state.pcm.rcsum[0] as u32,
                        (rcadd2[4] >> 1) as u32,
                        (rcadd2[4] & 1) as u32,
                    ) as i32;
                }
                31 => {
                    state.pcm.rcsum[1] = addclip20(
                        state.pcm.rcsum[1] as u32,
                        (rcadd2[5] >> 1) as u32,
                        (rcadd2[5] & 1) as u32,
                    ) as i32;
                }
                _ => {}
            }

            state.pcm.rcsum[0] = addclip20(
                state.pcm.rcsum[0] as u32,
                (rc0 >> 1) as u32,
                (rc0 & 1) as u32,
            ) as i32;
            state.pcm.rcsum[1] = addclip20(
                state.pcm.rcsum[1] as u32,
                (rc1 >> 1) as u32,
                (rc1 & 1) as u32,
            ) as i32;

            if slot as i32 != reg_slots - 1 {
                state.pcm.ram1[31][1] = suml;
                state.pcm.ram1[31][3] = sumr;
            } else {
                state.pcm.accum_l = suml as i32;
                state.pcm.accum_r = sumr as i32;
            }

            if key != 0 && state.pcm.nfs != 0 {
                state.pcm.ram2[slot][7] &= !0xF020;
                state.pcm.ram2[slot][7] |= ((if usenew != 0 || kon != 0 {
                    newnibble
                } else {
                    old_nibble
                }) << 12) as u16;

                // update key
                state.pcm.ram2[slot][7] |= (key << 5) as u16;
            }

            if active == 0 {
                if state.pcm.nfs != 0 {
                    state.pcm.ram1[slot][1] = 0;
                    state.pcm.ram1[slot][3] = 0;
                    state.pcm.ram1[slot][5] = 0;
                }

                state.pcm.ram2[slot][8] = 0;
                state.pcm.ram2[slot][9] = 0;
                state.pcm.ram2[slot][10] = 0;
            }
        }

        if state.pcm.nfs != 0 {
            state.pcm.ram2[31][7] |= 0x20;
        }

        state.pcm.nfs = 1;

        let cycles = ((reg_slots + 1) * 25) as u64;

        state.pcm.cycles += if state.mcu_jv880 {
            (cycles * 25) / 29
        } else {
            cycles
        };
    }
}
