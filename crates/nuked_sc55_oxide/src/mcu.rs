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

use crate::{
    Sc55State,
    mcu_interrupt::*,
    mcu_timer::{timer_read, timer_read2, timer_write, timer2_write},
    pcm::{pcm_read, pcm_write},
    state::UART_BUFFER_SIZE,
    submcu::{sm_sys_read, sm_sys_write},
};

pub(crate) const DEV_P1DDR: usize = 0x00;
pub(crate) const DEV_P5DDR: usize = 0x08;
pub(crate) const DEV_P6DDR: usize = 0x09;
pub(crate) const DEV_P7DDR: usize = 0x0C;
pub(crate) const DEV_P7DR: usize = 0x0E;
pub(crate) const DEV_FRT1_TCR: usize = 0x10;
pub(crate) const DEV_FRT1_TCSR: usize = 0x11;
pub(crate) const DEV_FRT1_FRCH: usize = 0x12;
pub(crate) const DEV_FRT1_FRCL: usize = 0x13;
pub(crate) const DEV_FRT2_TCSR: usize = 0x21;
pub(crate) const DEV_FRT3_TCSR: usize = 0x31;
pub(crate) const DEV_FRT3_OCRAH: usize = 0x34;
pub(crate) const DEV_FRT3_OCRAL: usize = 0x35;
pub(crate) const DEV_PWM1_TCR: usize = 0x40;
pub(crate) const DEV_PWM1_DTR: usize = 0x41;
pub(crate) const DEV_PWM2_TCR: usize = 0x44;
pub(crate) const DEV_PWM2_DTR: usize = 0x45;
pub(crate) const DEV_PWM3_TCR: usize = 0x48;
pub(crate) const DEV_PWM3_DTR: usize = 0x49;
pub(crate) const DEV_TMR_TCR: usize = 0x50;
pub(crate) const DEV_TMR_TCSR: usize = 0x51;
pub(crate) const DEV_TMR_TCORA: usize = 0x52;
pub(crate) const DEV_TMR_TCORB: usize = 0x53;
pub(crate) const DEV_TMR_TCNT: usize = 0x54;
pub(crate) const DEV_SMR: usize = 0x58;
pub(crate) const DEV_BRR: usize = 0x59;
pub(crate) const DEV_SCR: usize = 0x5A;
pub(crate) const DEV_TDR: usize = 0x5B;
pub(crate) const DEV_SSR: usize = 0x5C;
pub(crate) const DEV_RDR: usize = 0x5D;
pub(crate) const DEV_ADDRAH: usize = 0x60;
pub(crate) const DEV_ADDRAL: usize = 0x61;
pub(crate) const DEV_ADDRBH: usize = 0x62;
pub(crate) const DEV_ADDRBL: usize = 0x63;
pub(crate) const DEV_ADDRCH: usize = 0x64;
pub(crate) const DEV_ADDRCL: usize = 0x65;
pub(crate) const DEV_ADDRDH: usize = 0x66;
pub(crate) const DEV_ADDRDL: usize = 0x67;
pub(crate) const DEV_ADCSR: usize = 0x68;
pub(crate) const DEV_IPRA: usize = 0x70;
pub(crate) const DEV_IPRB: usize = 0x71;
pub(crate) const DEV_IPRC: usize = 0x72;
pub(crate) const DEV_IPRD: usize = 0x73;
pub(crate) const DEV_DTEA: usize = 0x74;
pub(crate) const DEV_DTEB: usize = 0x75;
pub(crate) const DEV_DTEC: usize = 0x76;
pub(crate) const DEV_DTED: usize = 0x77;
pub(crate) const DEV_WCR: usize = 0x78;
pub(crate) const DEV_RAME: usize = 0x79;
pub(crate) const DEV_P1CR: usize = 0x7C;
pub(crate) const DEV_P9DDR: usize = 0x7E;
pub(crate) const DEV_P9DR: usize = 0x7F;

pub(crate) const SR_MASK: u16 = 0x870F;

pub(crate) const STATUS_T: u16 = 0x8000;
pub(crate) const STATUS_N: u16 = 0x08;
pub(crate) const STATUS_Z: u16 = 0x04;
pub(crate) const STATUS_V: u16 = 0x02;
pub(crate) const STATUS_C: u16 = 0x01;
pub(crate) const STATUS_INT_MASK: u16 = 0x700;

pub(crate) const VECTOR_RESET: u32 = 0;
pub(crate) const VECTOR_INVALID_INSTRUCTION: u32 = 2;
pub(crate) const VECTOR_ADDRESS_ERROR: u32 = 8;
pub(crate) const VECTOR_TRACE: u32 = 9;
pub(crate) const VECTOR_NMI: u32 = 11;
pub(crate) const VECTOR_TRAPA_0: u32 = 16;
pub(crate) const VECTOR_IRQ0: u32 = 32;
pub(crate) const VECTOR_IRQ1: u32 = 33;
pub(crate) const VECTOR_INTERNAL_INTERRUPT_94: u32 = 37; // FRT1 OCIA
pub(crate) const VECTOR_INTERNAL_INTERRUPT_98: u32 = 38; // FRT1 OCIB
pub(crate) const VECTOR_INTERNAL_INTERRUPT_9C: u32 = 39; // FRT1 FOVI
pub(crate) const VECTOR_INTERNAL_INTERRUPT_A4: u32 = 41; // FRT2 OCIA
pub(crate) const VECTOR_INTERNAL_INTERRUPT_A8: u32 = 42; // FRT2 OCIB
pub(crate) const VECTOR_INTERNAL_INTERRUPT_AC: u32 = 43; // FRT2 FOVI
pub(crate) const VECTOR_INTERNAL_INTERRUPT_B4: u32 = 45; // FRT3 OCIA
pub(crate) const VECTOR_INTERNAL_INTERRUPT_B8: u32 = 46; // FRT3 OCIB
pub(crate) const VECTOR_INTERNAL_INTERRUPT_BC: u32 = 47; // FRT3 FOVI
pub(crate) const VECTOR_INTERNAL_INTERRUPT_C0: u32 = 48; // CMIA
pub(crate) const VECTOR_INTERNAL_INTERRUPT_C4: u32 = 49; // CMIB
pub(crate) const VECTOR_INTERNAL_INTERRUPT_C8: u32 = 50; // OVI
pub(crate) const VECTOR_INTERNAL_INTERRUPT_D4: u32 = 53; // RXI
pub(crate) const VECTOR_INTERNAL_INTERRUPT_D8: u32 = 54; // TXI
pub(crate) const VECTOR_INTERNAL_INTERRUPT_E0: u32 = 56; // ADI

const ANALOG_LEVEL_RCU_LOW: u16 = 0;
const ANALOG_LEVEL_RCU_HIGH: u16 = 0;
const ANALOG_LEVEL_SW_0: u16 = 0;
const ANALOG_LEVEL_SW_1: u16 = 0x155;
const ANALOG_LEVEL_SW_2: u16 = 0x2AA;
const ANALOG_LEVEL_SW_3: u16 = 0x3FF;
const ANALOG_LEVEL_BATTERY: u16 = 0x2A0;

#[derive(Default, Clone)]
/// Complete authoritative state of the SC-55 main MCU.
pub struct McuState {
    pub r: [u16; 8],
    pub pc: u16,
    pub sr: u16,
    pub cp: u8,
    pub dp: u8,
    pub ep: u8,
    pub tp: u8,
    pub br: u8,
    pub sleep: u8,
    pub ex_ignore: u8,
    pub exception_pending: i32,
    pub interrupt_pending: [u8; INTERRUPT_SOURCE_MAX as usize],
    pub trapa_pending: [u8; 16],
    pub cycles: u64,
}

crate::impl_state_codec!(McuState {
    r,
    pc,
    sr,
    cp,
    dp,
    ep,
    tp,
    br,
    sleep,
    ex_ignore,
    exception_pending,
    interrupt_pending,
    trapa_pending,
    cycles,
});

pub(crate) fn mcu_get_address(page: u8, address: u16) -> u32 {
    ((page as u32) << 16) + address as u32
}

pub(crate) fn mcu_read_code(state: &mut Sc55State) -> u8 {
    mcu_read(state, mcu_get_address(state.mcu.cp, state.mcu.pc))
}

pub(crate) fn mcu_read_code_advance(state: &mut Sc55State) -> u8 {
    let ret = mcu_read_code(state);
    state.mcu.pc = state.mcu.pc.wrapping_add(1);
    ret
}

pub(crate) fn mcu_get_vector_address(state: &mut Sc55State, vector: u32) -> u32 {
    mcu_read32(state, vector.wrapping_mul(4))
}

pub(crate) fn mcu_get_page_for_register(state: &mut Sc55State, reg: u32) -> u32 {
    if reg >= 6 {
        state.mcu.tp as u32
    } else if reg >= 4 {
        state.mcu.ep as u32
    } else {
        state.mcu.dp as u32
    }
}

pub(crate) fn mcu_control_register_write(state: &mut Sc55State, reg: u32, siz: u32, data: u32) {
    if siz != 0 {
        if reg == 0 {
            state.mcu.sr = data as u16;
            state.mcu.sr &= SR_MASK;
        } else if reg == 5 {
            // FIXME: undocumented
            state.mcu.dp = (data & 0xFF) as u8;
        } else if reg == 4 {
            // FIXME: undocumented
            state.mcu.ep = (data & 0xFF) as u8;
        } else if reg == 3 {
            // FIXME: undocumented
            state.mcu.br = (data & 0xFF) as u8;
        } else {
            mcu_error_trap(state);
        }
    } else {
        if reg == 1 {
            state.mcu.sr &= !0xFF;
            state.mcu.sr |= (data as u16) & 0xFF;
            state.mcu.sr &= SR_MASK;
        } else if reg == 3 {
            state.mcu.br = data as u8;
        } else if reg == 4 {
            state.mcu.ep = data as u8;
        } else if reg == 5 {
            state.mcu.dp = data as u8;
        } else if reg == 7 {
            state.mcu.tp = data as u8;
        } else {
            mcu_error_trap(state);
        }
    }
}

pub(crate) fn mcu_control_register_read(state: &mut Sc55State, reg: u32, siz: u32) -> u32 {
    let mut ret: u32 = 0;
    if siz != 0 {
        if reg == 0 {
            ret = (state.mcu.sr & SR_MASK) as u32;
        } else if reg == 5 {
            // FIXME: undocumented
            ret = state.mcu.dp as u32 | ((state.mcu.dp as u32) << 8);
        } else if reg == 4 {
            // FIXME: undocumented
            ret = state.mcu.ep as u32 | ((state.mcu.ep as u32) << 8);
        } else if reg == 3 {
            // FIXME: undocumented
            ret = state.mcu.br as u32 | ((state.mcu.br as u32) << 8);
        } else {
            mcu_error_trap(state);
        }
        ret &= 0xFFFF;
    } else {
        if reg == 1 {
            ret = (state.mcu.sr & SR_MASK) as u32;
        } else if reg == 3 {
            ret = state.mcu.br as u32;
        } else if reg == 4 {
            ret = state.mcu.ep as u32;
        } else if reg == 5 {
            ret = state.mcu.dp as u32;
        } else if reg == 7 {
            ret = state.mcu.tp as u32;
        } else {
            mcu_error_trap(state);
        }
        ret &= 0xFF;
    }
    ret
}

pub(crate) fn mcu_set_status(state: &mut Sc55State, condition: u32, mask: u16) {
    if condition != 0 {
        state.mcu.sr |= mask;
    } else {
        state.mcu.sr &= !mask;
    }
}

pub(crate) fn mcu_push_stack(state: &mut Sc55State, data: u16) {
    if state.mcu.r[7] & 1 != 0 {
        mcu_interrupt_exception(state, EXCEPTION_SOURCE_ADDRESS_ERROR as u32);
    }
    state.mcu.r[7] = state.mcu.r[7].wrapping_sub(2);
    mcu_write16(state, state.mcu.r[7] as u32, data);
}

pub(crate) fn mcu_pop_stack(state: &mut Sc55State) -> u16 {
    if state.mcu.r[7] & 1 != 0 {
        mcu_interrupt_exception(state, EXCEPTION_SOURCE_ADDRESS_ERROR as u32);
    }
    let ret = mcu_read16(state, state.mcu.r[7] as u32);
    state.mcu.r[7] = state.mcu.r[7].wrapping_add(2);
    ret
}

pub(crate) fn mcu_error_trap(state: &mut Sc55State) {
    eprintln!("{:02x} {:04x}", state.mcu.cp, state.mcu.pc);
}

pub(crate) fn rcu_read() -> u8 {
    0
}

pub(crate) fn mcu_sc155_sliders(_index: u32) -> u16 {
    // 0 - 1/9
    // 1 - 2/10
    // 2 - 3/11
    // 3 - 4/12
    // 4 - 5/13
    // 5 - 6/14
    // 6 - 7/15
    // 7 - 8/16
    // 8 - ALL
    0x0
}

fn read_rcu(_state: &mut Sc55State, pin: u32) -> u16 {
    let rcu = rcu_read();
    if rcu & (1 << pin) != 0 {
        ANALOG_LEVEL_RCU_HIGH
    } else {
        ANALOG_LEVEL_RCU_LOW
    }
}

pub(crate) fn mcu_analog_read_pin(state: &mut Sc55State, pin: u32) -> u16 {
    if state.mcu_cm300 {
        return 0;
    }
    if state.mcu_jv880 {
        if pin == 1 {
            return ANALOG_LEVEL_BATTERY;
        }
        return 0x3FF;
    }
    if state.mcu_mk1 {
        if state.mcu_sc155 && (state.dev_register[DEV_P9DR] & 1) != 0 {
            return mcu_sc155_sliders(pin);
        }
        if pin == 7 {
            if state.mcu_sc155 && (state.dev_register[DEV_P9DR] & 2) != 0 {
                mcu_sc155_sliders(8)
            } else {
                ANALOG_LEVEL_BATTERY
            }
        } else {
            read_rcu(state, pin)
        }
    } else {
        if state.mcu_sc155 && (state.io_sd & 16) != 0 {
            return mcu_sc155_sliders(pin);
        }
        if pin == 7 {
            if state.mcu_mk1 {
                return ANALOG_LEVEL_BATTERY;
            }
            match (state.io_sd >> 2) & 3 {
                0 => {
                    // Battery voltage
                    ANALOG_LEVEL_BATTERY
                }
                1 => {
                    // NC
                    if state.mcu_sc155 {
                        return mcu_sc155_sliders(8);
                    }
                    0
                }
                2 => {
                    // SW
                    match state.sw_pos {
                        0 => ANALOG_LEVEL_SW_0,
                        1 => ANALOG_LEVEL_SW_1,
                        2 => ANALOG_LEVEL_SW_2,
                        3 => ANALOG_LEVEL_SW_3,
                        _ => ANALOG_LEVEL_SW_0,
                    }
                }
                3 => {
                    // RCU
                    read_rcu(state, pin)
                }
                _ => read_rcu(state, pin),
            }
        } else {
            read_rcu(state, pin)
        }
    }
}

pub(crate) fn mcu_analog_sample(state: &mut Sc55State, channel: i32) {
    let value = mcu_analog_read_pin(state, channel as u32) as i32;
    let dest = ((channel << 1) & 6) as usize;
    state.dev_register[DEV_ADDRAH + dest] = (value >> 2) as u8;
    state.dev_register[DEV_ADDRAL + dest] = ((value << 6) & 0xC0) as u8;
}

pub(crate) fn mcu_device_write(state: &mut Sc55State, address: u32, data: u8) {
    let address = address & 0x7F;
    if (0x10..0x40).contains(&address) {
        timer_write(state, address, data);
        return;
    }
    if (0x50..0x55).contains(&address) {
        timer2_write(state, address, data);
        return;
    }
    let address = address as usize;
    match address {
        DEV_P1DDR => {} // P1DDR
        DEV_P5DDR => {}
        DEV_P6DDR => {}
        DEV_P7DDR => {}
        DEV_SCR => {}
        DEV_WCR => {}
        DEV_P9DDR => {}
        DEV_RAME => {} // RAME
        DEV_P1CR => {} // P1CR
        DEV_DTEA => {}
        DEV_DTEB => {}
        DEV_DTEC => {}
        DEV_DTED => {}
        DEV_SMR => {}
        DEV_BRR => {}
        DEV_IPRA => {}
        DEV_IPRB => {}
        DEV_IPRC => {}
        DEV_IPRD => {}
        DEV_PWM1_DTR => {}
        DEV_PWM1_TCR => {}
        DEV_PWM2_DTR => {}
        DEV_PWM2_TCR => {}
        DEV_PWM3_DTR => {}
        DEV_PWM3_TCR => {}
        DEV_P7DR => {}
        DEV_TMR_TCNT => {}
        DEV_TMR_TCR => {}
        DEV_TMR_TCSR => {}
        DEV_TMR_TCORA => {}
        DEV_TDR => {}
        DEV_ADCSR => {
            state.dev_register[address] &= !0x7F;
            state.dev_register[address] |= data & 0x7F;
            if (data & 0x80) == 0 && state.adf_rd != 0 {
                state.dev_register[address] &= !0x80;
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_ANALOG, 0);
            }
            if (data & 0x40) == 0 {
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_ANALOG, 0);
            }
            return;
        }
        DEV_SSR => {
            if (data & 0x80) == 0 && (state.ssr_rd & 0x80) != 0 {
                state.dev_register[address] &= !0x80;
                state.uart_tx_delay = state.mcu.cycles.wrapping_add(3000);
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_UART_TX, 0);
            }
            if (data & 0x40) == 0 && (state.ssr_rd & 0x40) != 0 {
                state.uart_rx_delay = state.mcu.cycles.wrapping_add(3000);
                state.dev_register[address] &= !0x40;
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_UART_RX, 0);
            }
            if (data & 0x20) == 0 && (state.ssr_rd & 0x20) != 0 {
                state.dev_register[address] &= !0x20;
            }
            if (data & 0x10) == 0 && (state.ssr_rd & 0x10) != 0 {
                state.dev_register[address] &= !0x10;
            }
        }
        _ => {
            // address += 0;
        }
    }
    state.dev_register[address] = data;
}

pub(crate) fn mcu_device_read(state: &mut Sc55State, address: u32) -> u8 {
    let address = address & 0x7F;
    if (0x10..0x40).contains(&address) {
        return timer_read(state, address);
    }
    if (0x50..0x55).contains(&address) {
        return timer_read2(state, address);
    }
    let address = address as usize;
    match address {
        DEV_ADDRAH | DEV_ADDRAL | DEV_ADDRBH | DEV_ADDRBL | DEV_ADDRCH | DEV_ADDRCL
        | DEV_ADDRDH | DEV_ADDRDL => state.dev_register[address],
        DEV_ADCSR => {
            state.adf_rd = if (state.dev_register[address] & 0x80) != 0 {
                1
            } else {
                0
            };
            state.dev_register[address]
        }
        DEV_SSR => {
            state.ssr_rd = state.dev_register[address] as i32;
            state.dev_register[address]
        }
        DEV_RDR => state.uart_rx_byte,
        0x00 => 0xFF,
        DEV_P7DR => 0xFF,
        DEV_P9DR => {
            let cfg: i32 = if !state.mcu_mk1 {
                if state.mcu_sc155 { 0 } else { 2 } // bit 1: 0 - SC-155mk2 (???), 1 - SC-55mk2
            } else {
                0
            };

            let dir = state.dev_register[DEV_P9DDR] as i32;

            let mut val = cfg & (dir ^ 0xFF);
            val |= state.dev_register[DEV_P9DR] as i32 & dir;
            val as u8
        }
        DEV_SCR | DEV_TDR | DEV_SMR => state.dev_register[address],
        DEV_IPRC | DEV_IPRD | DEV_DTEC | DEV_DTED | DEV_FRT2_TCSR | DEV_FRT1_TCSR
        | DEV_FRT1_TCR | DEV_FRT1_FRCH | DEV_FRT1_FRCL | DEV_FRT3_TCSR | DEV_FRT3_OCRAH
        | DEV_FRT3_OCRAL => state.dev_register[address],
        _ => state.dev_register[address],
    }
}

pub(crate) fn mcu_device_reset(state: &mut Sc55State) {
    // dev_register[0x00] = 0x03;
    // dev_register[0x7c] = 0x87;
    state.dev_register[DEV_RAME] = 0x80;
    state.dev_register[DEV_SSR] = 0x80;
}

pub(crate) fn mcu_update_analog(state: &mut Sc55State, cycles: u64) {
    let ctrl = state.dev_register[DEV_ADCSR] as i32;
    let isscan = (ctrl & 16) != 0;

    if ctrl & 0x20 != 0 {
        if state.analog_end_time == 0 {
            state.analog_end_time = cycles.wrapping_add(200);
        } else if state.analog_end_time < cycles {
            if isscan {
                let base = ctrl & 4;
                for i in 0..=(ctrl & 3) {
                    mcu_analog_sample(state, base + i);
                }
                state.analog_end_time = cycles.wrapping_add(200);
            } else {
                mcu_analog_sample(state, ctrl & 7);
                state.dev_register[DEV_ADCSR] &= !0x20;
                state.analog_end_time = 0;
            }
            state.dev_register[DEV_ADCSR] |= 0x80;
            if ctrl & 0x40 != 0 {
                mcu_interrupt_set_request(state, INTERRUPT_SOURCE_ANALOG, 1);
            }
        }
    } else {
        state.analog_end_time = 0;
    }
}

pub(crate) fn mcu_read(state: &mut Sc55State, address: u32) -> u8 {
    let mut address_rom = address & 0x3FFFF;
    if address & 0x80000 != 0 && !state.mcu_jv880 {
        address_rom |= 0x40000;
    }
    let page = (address >> 16) & 0xF;
    let address = address & 0xFFFF;
    let ret: u8;
    match page {
        0 => {
            if address & 0x8000 == 0 {
                ret = state.rom1[(address & 0x7FFF) as usize];
            } else {
                if !state.mcu_mk1 {
                    let base: u16 = if state.mcu_jv880 { 0xF000 } else { 0xE000 };
                    if address >= base as u32 && address < (base | 0x400) as u32 {
                        ret = pcm_read(state, address & 0x3F);
                    } else if !state.mcu_scb55 && (0xEC00..0xF000).contains(&address) {
                        ret = sm_sys_read(state, address & 0xFF);
                    } else if address >= 0xFF80 {
                        ret = mcu_device_read(state, address & 0x7F);
                    } else if (0xFB80..0xFF80).contains(&address)
                        && (state.dev_register[DEV_RAME] & 0x80) != 0
                    {
                        ret = state.ram[(address.wrapping_sub(0xFB80) & 0x3FF) as usize];
                    } else if (0x8000..0xE000).contains(&address) {
                        ret = state.sram[(address & 0x7FFF) as usize];
                    } else if address == (base | 0x402) as u32 {
                        ret = state.ga_int_trigger as u8;
                        state.ga_int_trigger = 0;
                        mcu_interrupt_set_request(
                            state,
                            if state.mcu_jv880 {
                                INTERRUPT_SOURCE_IRQ0
                            } else {
                                INTERRUPT_SOURCE_IRQ1
                            },
                            0,
                        );
                    } else {
                        ret = 0xFF;
                    }
                    //
                    // e402:2-0 irq source
                    //
                } else {
                    if (0xE000..0xE040).contains(&address) {
                        ret = pcm_read(state, address & 0x3F);
                    } else if address >= 0xFF80 {
                        ret = mcu_device_read(state, address & 0x7F);
                    } else if (0xFB80..0xFF80).contains(&address)
                        && (state.dev_register[DEV_RAME] & 0x80) != 0
                    {
                        ret = state.ram[(address.wrapping_sub(0xFB80) & 0x3FF) as usize];
                    } else if (0x8000..0xE000).contains(&address) {
                        ret = state.sram[(address & 0x7FFF) as usize];
                    } else if (0xF000..0xF100).contains(&address) {
                        state.io_sd = (address & 0xFF) as u8;
                        return 0xFF;
                    } else if address == 0xF106 {
                        ret = state.ga_int_trigger as u8;
                        state.ga_int_trigger = 0;
                        mcu_interrupt_set_request(state, INTERRUPT_SOURCE_IRQ1, 0);
                    } else {
                        ret = 0xFF;
                    }
                    //
                    // f106:2-0 irq source
                    //
                }
            }
        }

        1 => {
            ret = state.rom2[(address_rom & state.rom2_mask as u32) as usize];
        }
        2 => {
            ret = state.rom2[(address_rom & state.rom2_mask as u32) as usize];
        }
        3 => {
            ret = state.rom2[(address_rom & state.rom2_mask as u32) as usize];
        }
        4 => {
            ret = state.rom2[(address_rom & state.rom2_mask as u32) as usize];
        }
        8 => {
            if !state.mcu_jv880 {
                ret = state.rom2[(address_rom & state.rom2_mask as u32) as usize];
            } else {
                ret = 0xFF;
            }
        }
        9 => {
            if !state.mcu_jv880 {
                ret = state.rom2[(address_rom & state.rom2_mask as u32) as usize];
            } else {
                ret = 0xFF;
            }
        }
        14 | 15 => {
            if !state.mcu_jv880 {
                ret = state.rom2[(address_rom & state.rom2_mask as u32) as usize];
            } else {
                ret = state.cardram[(address & 0x7FFF) as usize]; // FIXME
            }
        }
        10 | 11 => {
            if !state.mcu_mk1 {
                ret = state.sram[(address & 0x7FFF) as usize]; // FIXME
            } else {
                ret = 0xFF;
            }
        }
        12 | 13 => {
            if state.mcu_jv880 {
                ret = state.nvram[(address & 0x7FFF) as usize]; // FIXME
            } else {
                ret = 0xFF;
            }
        }
        5 => {
            if state.mcu_mk1 {
                ret = state.sram[(address & 0x7FFF) as usize]; // FIXME
            } else {
                ret = 0xFF;
            }
        }
        _ => {
            ret = 0x00;
        }
    }
    ret
}

pub(crate) fn mcu_read16(state: &mut Sc55State, address: u32) -> u16 {
    let address = address & !1;
    let b0 = mcu_read(state, address);
    let b1 = mcu_read(state, address.wrapping_add(1));
    ((b0 as u16) << 8).wrapping_add(b1 as u16)
}

pub(crate) fn mcu_read32(state: &mut Sc55State, address: u32) -> u32 {
    let address = address & !3;
    let b0 = mcu_read(state, address);
    let b1 = mcu_read(state, address.wrapping_add(1));
    let b2 = mcu_read(state, address.wrapping_add(2));
    let b3 = mcu_read(state, address.wrapping_add(3));
    ((b0 as u32) << 24)
        .wrapping_add((b1 as u32) << 16)
        .wrapping_add((b2 as u32) << 8)
        .wrapping_add(b3 as u32)
}

#[allow(clippy::if_same_then_else)]
pub(crate) fn mcu_write(state: &mut Sc55State, address: u32, value: u8) {
    let page = (address >> 16) & 0xF;
    let address = address & 0xFFFF;
    if page == 0 {
        if address & 0x8000 != 0 {
            if !state.mcu_mk1 {
                let base: u16 = if state.mcu_jv880 { 0xF000 } else { 0xE000 };
                if address >= (base | 0x400) as u32 && address < (base | 0x800) as u32 {
                    if address == (base | 0x401) as u32 {
                        state.io_sd = value;
                    } else if address == (base | 0x402) as u32 {
                        state.ga_int_enable = (value as i32) << 1;
                    }
                    //
                    // e400: always 4?
                    // e401: SC0-6?
                    // e402: enable/disable IRQ?
                    // e403: always 1?
                    // e404: LCD
                    // e405: LCD
                    // e406: 0 or 40
                    // e407: 0, e406 continuation?
                    //
                } else if address >= base as u32 && address < (base | 0x400) as u32 {
                    pcm_write(state, address & 0x3F, value);
                } else if !state.mcu_scb55 && (0xEC00..0xF000).contains(&address) {
                    sm_sys_write(state, address & 0xFF, value);
                } else if address >= 0xFF80 {
                    mcu_device_write(state, address & 0x7F, value);
                } else if (0xFB80..0xFF80).contains(&address)
                    && (state.dev_register[DEV_RAME] & 0x80) != 0
                {
                    state.ram[(address.wrapping_sub(0xFB80) & 0x3FF) as usize] = value;
                } else if (0x8000..0xE000).contains(&address) {
                    state.sram[(address & 0x7FFF) as usize] = value;
                }
            } else {
                if (0xE000..0xE040).contains(&address) {
                    pcm_write(state, address & 0x3F, value);
                } else if address >= 0xFF80 {
                    mcu_device_write(state, address & 0x7F, value);
                } else if (0xFB80..0xFF80).contains(&address)
                    && (state.dev_register[DEV_RAME] & 0x80) != 0
                {
                    state.ram[(address.wrapping_sub(0xFB80) & 0x3FF) as usize] = value;
                } else if (0x8000..0xE000).contains(&address) {
                    state.sram[(address & 0x7FFF) as usize] = value;
                } else if (0xF000..0xF100).contains(&address) {
                    state.io_sd = (address & 0xFF) as u8;
                } else if address == 0xF107 {
                    state.io_sd = value;
                }
            }
        } else if state.mcu_jv880 && (0x6196..=0x6199).contains(&address) {
            // nop: the jv880 rom writes into the rom at 002E77-002E7D
        }
    } else if page == 5 && state.mcu_mk1 {
        state.sram[(address & 0x7FFF) as usize] = value; // FIXME
    } else if page == 10 && !state.mcu_mk1 {
        state.sram[(address & 0x7FFF) as usize] = value; // FIXME
    } else if page == 12 && state.mcu_jv880 {
        state.nvram[(address & 0x7FFF) as usize] = value; // FIXME
    } else if page == 14 && state.mcu_jv880 {
        state.cardram[(address & 0x7FFF) as usize] = value; // FIXME
    } else {
        //printf("Unknown write %x %x\n", (page << 16) | address, value);
    }
}

pub(crate) fn mcu_write16(state: &mut Sc55State, address: u32, value: u16) {
    let address = address & !1;
    mcu_write(state, address, (value >> 8) as u8);
    mcu_write(state, address.wrapping_add(1), (value & 0xFF) as u8);
}

pub(crate) fn mcu_read_instruction(state: &mut Sc55State) {
    let operand = mcu_read_code_advance(state);

    crate::mcu_opcodes::mcu_operand_dispatch(state, operand);

    if state.mcu.sr & STATUS_T != 0 {
        mcu_interrupt_exception(state, EXCEPTION_SOURCE_TRACE as u32);
    }
}

pub(crate) fn mcu_init(state: &mut Sc55State) {
    state.mcu = McuState::default();
}

pub(crate) fn mcu_reset(state: &mut Sc55State) {
    state.mcu.r[0] = 0;
    state.mcu.r[1] = 0;
    state.mcu.r[2] = 0;
    state.mcu.r[3] = 0;
    state.mcu.r[4] = 0;
    state.mcu.r[5] = 0;
    state.mcu.r[6] = 0;
    state.mcu.r[7] = 0;

    state.mcu.pc = 0;

    state.mcu.sr = 0x700;

    state.mcu.cp = 0;
    state.mcu.dp = 0;
    state.mcu.ep = 0;
    state.mcu.tp = 0;
    state.mcu.br = 0;

    let reset_address = mcu_get_vector_address(state, VECTOR_RESET);
    state.mcu.cp = ((reset_address >> 16) & 0xFF) as u8;
    state.mcu.pc = (reset_address & 0xFFFF) as u16;

    state.mcu.exception_pending = -1;

    mcu_device_reset(state);

    if state.mcu_mk1 {
        state.ga_int_enable = 255;
    }
}

pub(crate) fn mcu_post_uart(state: &mut Sc55State, data: u8) {
    state.uart_buffer[state.uart_write_ptr as usize] = data;
    state.uart_write_ptr = (state.uart_write_ptr + 1) % UART_BUFFER_SIZE as u32;
}

pub(crate) fn mcu_update_uart_rx(state: &mut Sc55State) {
    if (state.dev_register[DEV_SCR] & 16) == 0 {
        // RX disabled
        return;
    }
    if state.uart_write_ptr == state.uart_read_ptr {
        // no byte
        return;
    }

    if state.dev_register[DEV_SSR] & 0x40 != 0 {
        return;
    }

    if state.mcu.cycles < state.uart_rx_delay {
        return;
    }

    state.uart_rx_byte = state.uart_buffer[state.uart_read_ptr as usize];
    state.uart_read_ptr = (state.uart_read_ptr + 1) % UART_BUFFER_SIZE as u32;
    state.dev_register[DEV_SSR] |= 0x40;
    mcu_interrupt_set_request(
        state,
        INTERRUPT_SOURCE_UART_RX,
        if (state.dev_register[DEV_SCR] & 0x40) != 0 {
            1
        } else {
            0
        },
    );
}

// dummy TX
pub(crate) fn mcu_update_uart_tx(state: &mut Sc55State) {
    if (state.dev_register[DEV_SCR] & 32) == 0 {
        // TX disabled
        return;
    }

    if state.dev_register[DEV_SSR] & 0x80 != 0 {
        return;
    }

    if state.mcu.cycles < state.uart_tx_delay {
        return;
    }

    state.dev_register[DEV_SSR] |= 0x80;
    mcu_interrupt_set_request(
        state,
        INTERRUPT_SOURCE_UART_TX,
        if (state.dev_register[DEV_SCR] & 0x80) != 0 {
            1
        } else {
            0
        },
    );
}

pub(crate) fn mcu_patch_rom(_state: &mut Sc55State) {
    //rom2[0x1333] = 0x11;
    //rom2[0x1334] = 0x19;
    //rom1[0x622d] = 0x19;
}

pub(crate) fn unscramble(src: &[u8], dst: &mut [u8], len: usize) {
    for (i, dst_byte) in dst.iter_mut().enumerate().take(len) {
        let mut address = i & !0xFFFFF;
        let aa: [usize; 20] = [
            2, 0, 3, 4, 1, 9, 13, 10, 18, 17, 6, 15, 11, 16, 8, 5, 12, 7, 14, 19,
        ];
        for (j, &a) in aa.iter().enumerate() {
            if i & (1 << j) != 0 {
                address |= 1 << a;
            }
        }
        let srcdata = src[address];
        let mut data: u8 = 0;
        let dd: [usize; 8] = [2, 0, 4, 5, 7, 6, 3, 1];
        for (j, &d) in dd.iter().enumerate() {
            if srcdata & (1 << d) != 0 {
                data |= 1 << j;
            }
        }
        *dst_byte = data;
    }
}

pub(crate) fn mcu_ga_set_ga_int(state: &mut Sc55State, line: i32, value: i32) {
    // guesswork
    if value != 0 && state.ga_int[line as usize] == 0 && (state.ga_int_enable & (1 << line)) != 0 {
        state.ga_int_trigger = line;
    }
    state.ga_int[line as usize] = value;

    if state.mcu_jv880 {
        mcu_interrupt_set_request(
            state,
            INTERRUPT_SOURCE_IRQ0,
            if state.ga_int_trigger != 0 { 1 } else { 0 },
        );
    } else {
        mcu_interrupt_set_request(
            state,
            INTERRUPT_SOURCE_IRQ1,
            if state.ga_int_trigger != 0 { 1 } else { 0 },
        );
    }
}
