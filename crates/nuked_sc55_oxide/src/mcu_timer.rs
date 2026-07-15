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

use crate::{Sc55State, mcu::*, mcu_interrupt::*};

const REG_TCR: u32 = 0x00;
const REG_TCSR: u32 = 0x01;
const REG_FRCH: u32 = 0x02;
const REG_FRCL: u32 = 0x03;
const REG_OCRAH: u32 = 0x04;
const REG_OCRAL: u32 = 0x05;
const REG_OCRBH: u32 = 0x06;
const REG_OCRBL: u32 = 0x07;
const REG_ICRH: u32 = 0x08;
const REG_ICRL: u32 = 0x09;

#[derive(Clone, Copy, Default)]
/// Authoritative state of the SC-55 free-running timer.
pub struct FrtState {
    pub tcr: u8,
    pub tcsr: u8,
    pub frc: u16,
    pub ocra: u16,
    pub ocrb: u16,
    pub icr: u16,
    pub status_rd: u8,
}

#[derive(Clone, Copy, Default)]
/// Authoritative state of the SC-55 MCU timer block.
pub struct McuTimerState {
    pub tcr: u8,
    pub tcsr: u8,
    pub tcora: u8,
    pub tcorb: u8,
    pub tcnt: u8,
    pub status_rd: u8,
}

crate::impl_state_codec!(FrtState {
    tcr,
    tcsr,
    frc,
    ocra,
    ocrb,
    icr,
    status_rd,
});

crate::impl_state_codec!(McuTimerState {
    tcr,
    tcsr,
    tcora,
    tcorb,
    tcnt,
    status_rd,
});

pub(crate) fn timer_write(state: &mut Sc55State, address: u32, data: u8) {
    let t = (address >> 4).wrapping_sub(1);
    if t > 2 {
        return;
    }
    let address = address & 0x0F;
    match address {
        REG_TCR => {
            state.frt[t as usize].tcr = data;
        }
        REG_TCSR => {
            state.frt[t as usize].tcsr &= !0xF;
            state.frt[t as usize].tcsr |= data & 0xF;
            if (data & 0x10) == 0 && (state.frt[t as usize].status_rd & 0x10) != 0 {
                state.frt[t as usize].tcsr &= !0x10;
                state.frt[t as usize].status_rd &= !0x10;
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_FRT0_FOVI + t * 4, 0);
            }
            if (data & 0x20) == 0 && (state.frt[t as usize].status_rd & 0x20) != 0 {
                state.frt[t as usize].tcsr &= !0x20;
                state.frt[t as usize].status_rd &= !0x20;
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_FRT0_OCIA + t * 4, 0);
            }
            if (data & 0x40) == 0 && (state.frt[t as usize].status_rd & 0x40) != 0 {
                state.frt[t as usize].tcsr &= !0x40;
                state.frt[t as usize].status_rd &= !0x40;
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_FRT0_OCIB + t * 4, 0);
            }
        }
        REG_FRCH | REG_OCRAH | REG_OCRBH | REG_ICRH => {
            state.timer_tempreg = data;
        }
        REG_FRCL => {
            state.frt[t as usize].frc = ((state.timer_tempreg as u16) << 8) | data as u16;
        }
        REG_OCRAL => {
            state.frt[t as usize].ocra = ((state.timer_tempreg as u16) << 8) | data as u16;
        }
        REG_OCRBL => {
            state.frt[t as usize].ocrb = ((state.timer_tempreg as u16) << 8) | data as u16;
        }
        REG_ICRL => {
            state.frt[t as usize].icr = ((state.timer_tempreg as u16) << 8) | data as u16;
        }
        _ => {}
    }
}

pub(crate) fn timer_read(state: &mut Sc55State, address: u32) -> u8 {
    let t = (address >> 4).wrapping_sub(1);
    if t > 2 {
        return 0xFF;
    }
    let address = address & 0x0F;
    match address {
        REG_TCR => state.frt[t as usize].tcr,
        REG_TCSR => {
            let ret = state.frt[t as usize].tcsr;
            state.frt[t as usize].status_rd |= state.frt[t as usize].tcsr & 0xF0;
            //timer->status_rd |= 0xf0;
            ret
        }
        REG_FRCH => {
            state.timer_tempreg = (state.frt[t as usize].frc & 0xFF) as u8;
            (state.frt[t as usize].frc >> 8) as u8
        }
        REG_OCRAH => {
            state.timer_tempreg = (state.frt[t as usize].ocra & 0xFF) as u8;
            (state.frt[t as usize].ocra >> 8) as u8
        }
        REG_OCRBH => {
            state.timer_tempreg = (state.frt[t as usize].ocrb & 0xFF) as u8;
            (state.frt[t as usize].ocrb >> 8) as u8
        }
        REG_ICRH => {
            state.timer_tempreg = (state.frt[t as usize].icr & 0xFF) as u8;
            (state.frt[t as usize].icr >> 8) as u8
        }
        REG_FRCL | REG_OCRAL | REG_OCRBL | REG_ICRL => state.timer_tempreg,
        _ => 0xFF,
    }
}

pub(crate) fn timer2_write(state: &mut Sc55State, address: u32, data: u8) {
    match address as usize {
        DEV_TMR_TCR => {
            state.timer.tcr = data;
        }
        DEV_TMR_TCSR => {
            state.timer.tcsr &= !0xF;
            state.timer.tcsr |= data & 0xF;
            if (data & 0x20) == 0 && (state.timer.status_rd & 0x20) != 0 {
                state.timer.tcsr &= !0x20;
                state.timer.status_rd &= !0x20;
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_TIMER_OVI, 0);
            }
            if (data & 0x40) == 0 && (state.timer.status_rd & 0x40) != 0 {
                state.timer.tcsr &= !0x40;
                state.timer.status_rd &= !0x40;
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_TIMER_CMIA, 0);
            }
            if (data & 0x80) == 0 && (state.timer.status_rd & 0x80) != 0 {
                state.timer.tcsr &= !0x80;
                state.timer.status_rd &= !0x80;
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_TIMER_CMIB, 0);
            }
        }
        DEV_TMR_TCORA => {
            state.timer.tcora = data;
        }
        DEV_TMR_TCORB => {
            state.timer.tcorb = data;
        }
        DEV_TMR_TCNT => {
            state.timer.tcnt = data;
        }
        _ => {}
    }
}

pub(crate) fn timer_read2(state: &mut Sc55State, address: u32) -> u8 {
    match address as usize {
        DEV_TMR_TCR => state.timer.tcr,
        DEV_TMR_TCSR => {
            let ret = state.timer.tcsr;
            state.timer.status_rd |= state.timer.tcsr & 0xE0;
            ret
        }
        DEV_TMR_TCORA => state.timer.tcora,
        DEV_TMR_TCORB => state.timer.tcorb,
        DEV_TMR_TCNT => state.timer.tcnt,
        _ => 0xFF,
    }
}

#[allow(clippy::if_same_then_else)]
pub(crate) fn timer_clock(state: &mut Sc55State, cycles: u64) {
    while state.timer_cycles.wrapping_mul(2) < cycles {
        // FIXME
        for i in 0..3u32 {
            let offset = 0x10 * i;
            let _ = offset;

            match state.frt[i as usize].tcr & 3 {
                // o / 4
                0 if state.timer_cycles & 3 != 0 => continue,
                // o / 8
                1 if state.timer_cycles & 7 != 0 => continue,
                // o / 32
                2 if state.timer_cycles & 31 != 0 => continue,
                // ext (o / 2)
                3 if state.mcu_mk1 && state.timer_cycles & 3 != 0 => continue,
                3 if !state.mcu_mk1 && state.timer_cycles & 1 != 0 => continue,
                _ => {}
            }

            let value = state.frt[i as usize].frc as u32;
            let matcha = value == state.frt[i as usize].ocra as u32;
            let matchb = value == state.frt[i as usize].ocrb as u32;
            let value = if (state.frt[i as usize].tcsr & 1) != 0 && matcha {
                // CCLRA
                0u32
            } else {
                value.wrapping_add(1)
            };
            let of = (value >> 16) & 1;
            let value = value & 0xFFFF;
            state.frt[i as usize].frc = value as u16;

            // flags
            if of != 0 {
                state.frt[i as usize].tcsr |= 0x10;
            }
            if matcha {
                state.frt[i as usize].tcsr |= 0x20;
            }
            if matchb {
                state.frt[i as usize].tcsr |= 0x40;
            }
            if (state.frt[i as usize].tcr & 0x10) != 0 && (state.frt[i as usize].tcsr & 0x10) != 0 {
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_FRT0_FOVI + i * 4, 1);
            }
            if (state.frt[i as usize].tcr & 0x20) != 0 && (state.frt[i as usize].tcsr & 0x20) != 0 {
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_FRT0_OCIA + i * 4, 1);
            }
            if (state.frt[i as usize].tcr & 0x40) != 0 && (state.frt[i as usize].tcsr & 0x40) != 0 {
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_FRT0_OCIB + i * 4, 1);
            }
        }

        let mut timer_step: i32 = 0;

        match state.timer.tcr & 7 {
            0 | 4 => {}
            // o / 8
            1 if (state.timer_cycles & 7) == 0 => timer_step = 1,
            // o / 64
            2 if (state.timer_cycles & 63) == 0 => timer_step = 1,
            // o / 1024
            3 if (state.timer_cycles & 1023) == 0 => timer_step = 1,
            // ext (o / 2)
            5..=7 if state.mcu_mk1 && (state.timer_cycles & 3) == 0 => timer_step = 1,
            5..=7 if !state.mcu_mk1 && (state.timer_cycles & 1) == 0 => timer_step = 1,
            _ => {}
        }
        if timer_step != 0 {
            let value = state.timer.tcnt as u32;
            let matcha = value == state.timer.tcora as u32;
            let matchb = value == state.timer.tcorb as u32;
            let value = if (state.timer.tcr & 24) == 8 && matcha {
                0u32
            } else if (state.timer.tcr & 24) == 16 && matchb {
                0u32
            } else {
                value.wrapping_add(1)
            };
            let of = (value >> 8) & 1;
            let value = value & 0xFF;
            state.timer.tcnt = value as u8;

            // flags
            if of != 0 {
                state.timer.tcsr |= 0x20;
            }
            if matcha {
                state.timer.tcsr |= 0x40;
            }
            if matchb {
                state.timer.tcsr |= 0x80;
            }
            if (state.timer.tcr & 0x20) != 0 && (state.timer.tcsr & 0x20) != 0 {
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_TIMER_OVI, 1);
            }
            if (state.timer.tcr & 0x40) != 0 && (state.timer.tcsr & 0x40) != 0 {
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_TIMER_CMIA, 1);
            }
            if (state.timer.tcr & 0x80) != 0 && (state.timer.tcsr & 0x80) != 0 {
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_TIMER_CMIB, 1);
            }
        }

        state.timer_cycles = state.timer_cycles.wrapping_add(1);
    }
}
