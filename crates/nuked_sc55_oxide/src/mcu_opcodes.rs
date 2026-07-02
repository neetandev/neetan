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
    mcu::{
        STATUS_C, STATUS_N, STATUS_V, STATUS_Z, mcu_control_register_read,
        mcu_control_register_write, mcu_error_trap, mcu_get_address, mcu_get_page_for_register,
        mcu_pop_stack, mcu_push_stack, mcu_read, mcu_read_code_advance, mcu_read16, mcu_set_status,
        mcu_write, mcu_write16,
    },
    mcu_interrupt::{
        EXCEPTION_SOURCE_ADDRESS_ERROR, EXCEPTION_SOURCE_INVALID_INSTRUCTION,
        mcu_interrupt_exception, mcu_interrupt_trapa,
    },
};

const GENERAL_DIRECT: u32 = 0;
const GENERAL_INDIRECT: u32 = 1;
const GENERAL_ABSOLUTE: u32 = 2;
const GENERAL_IMMEDIATE: u32 = 3;

const OPERAND_BYTE: u8 = 0;
const OPERAND_WORD: u8 = 1;

const INCREASE_NONE: u32 = 0;
const INCREASE_DECREASE: u32 = 1;
const INCREASE_INCREASE: u32 = 2;

pub(crate) fn mcu_sub_common(
    state: &mut Sc55State,
    mut t1: i32,
    t2: i32,
    c_bit: i32,
    siz: u32,
) -> i32 {
    let st1: i32;
    let st2: i32;
    let n: i32;
    let z: i32;
    let c: i32;
    let mut v: i32 = 0;
    if siz != 0 {
        st1 = t1 as i16 as i32;
        st2 = t2 as i16 as i32;
        t1 = t1 as u16 as i32;
        let t2 = t2 as u16 as i32;
        t1 -= t2;
        t1 -= c_bit;
        c = (t1 >> 16) & 1;

        t1 &= 0xFFFF;
        n = (t1 & 0x8000 != 0) as i32;
        z = (t1 == 0) as i32;

        let st_result = st1 - st2 - c_bit;
        if st_result < i16::MIN as i32 || st_result > i16::MAX as i32 {
            v = 1;
        }
    } else {
        st1 = t1 as i8 as i32;
        st2 = t2 as i8 as i32;
        t1 = t1 as u8 as i32;
        let t2 = t2 as u8 as i32;
        t1 -= t2;
        t1 -= c_bit;
        c = (t1 >> 8) & 1;

        t1 &= 0xFF;
        n = (t1 & 0x80 != 0) as i32;
        z = (t1 == 0) as i32;

        let st_result = st1 - st2 - c_bit;
        if st_result < i8::MIN as i32 || st_result > i8::MAX as i32 {
            v = 1;
        }
    }
    mcu_set_status(state, n as u32, STATUS_N);
    mcu_set_status(state, z as u32, STATUS_Z);
    mcu_set_status(state, c as u32, STATUS_C);
    mcu_set_status(state, v as u32, STATUS_V);

    t1
}

pub(crate) fn mcu_add_common(
    state: &mut Sc55State,
    mut t1: i32,
    t2: i32,
    c_bit: i32,
    siz: u32,
) -> i32 {
    let st1: i32;
    let st2: i32;
    let n: i32;
    let z: i32;
    let c: i32;
    let mut v: i32 = 0;
    if siz != 0 {
        st1 = t1 as i16 as i32;
        st2 = t2 as i16 as i32;
        t1 = t1 as u16 as i32;
        let t2 = t2 as u16 as i32;
        t1 += t2;
        t1 += c_bit;
        c = (t1 >> 16) & 1;

        t1 &= 0xFFFF;
        n = (t1 & 0x8000 != 0) as i32;
        z = (t1 == 0) as i32;

        let st_result = st1 + st2 + c_bit;
        if st_result < i16::MIN as i32 || st_result > i16::MAX as i32 {
            v = 1;
        }
    } else {
        st1 = t1 as i8 as i32;
        st2 = t2 as i8 as i32;
        t1 = t1 as u8 as i32;
        let t2 = t2 as u8 as i32;
        t1 += t2;
        t1 += c_bit;
        c = (t1 >> 8) & 1;

        t1 &= 0xFF;
        n = (t1 & 0x80 != 0) as i32;
        z = (t1 == 0) as i32;

        let st_result = st1 + st2 + c_bit;
        if st_result < i8::MIN as i32 || st_result > i8::MAX as i32 {
            v = 1;
        }
    }
    mcu_set_status(state, n as u32, STATUS_N);
    mcu_set_status(state, z as u32, STATUS_Z);
    mcu_set_status(state, c as u32, STATUS_C);
    mcu_set_status(state, v as u32, STATUS_V);

    t1
}

pub(crate) fn mcu_operand_nop(_state: &mut Sc55State, _operand: u8) {}

pub(crate) fn mcu_operand_sleep(state: &mut Sc55State, _operand: u8) {
    state.mcu.sleep = 1;
}

pub(crate) fn mcu_operand_not_implemented(state: &mut Sc55State, _operand: u8) {
    mcu_error_trap(state);
}

pub(crate) fn mcu_ldm(state: &mut Sc55State, _operand: u8) {
    let rlist = mcu_read_code_advance(state);
    for i in 0..8 {
        if rlist & (1 << i) != 0 {
            let data = mcu_pop_stack(state);
            if i != 7 {
                state.mcu.r[i] = data;
            }
        }
    }
}

pub(crate) fn mcu_stm(state: &mut Sc55State, _operand: u8) {
    let rlist = mcu_read_code_advance(state);
    for i in (0..8).rev() {
        if rlist & (1 << i) != 0 {
            let mut data = state.mcu.r[i];
            if i == 7 {
                data = data.wrapping_sub(2);
            }
            mcu_push_stack(state, data);
        }
    }
}

pub(crate) fn mcu_trapa(state: &mut Sc55State, _operand: u8) {
    let opcode = mcu_read_code_advance(state) as u32;
    if (opcode & 0xF0) == 0x10 {
        mcu_interrupt_trapa(state, opcode & 0x0F);
    } else {
        mcu_error_trap(state);
    }
}

pub(crate) fn mcu_jump_pjsr(state: &mut Sc55State, _operand: u8) {
    let page = mcu_read_code_advance(state);
    let mut address: u16;
    address = (mcu_read_code_advance(state) as u16) << 8;
    address |= mcu_read_code_advance(state) as u16;
    mcu_push_stack(state, state.mcu.pc);
    mcu_push_stack(state, state.mcu.cp as u16);
    state.mcu.cp = page;
    if state.mcu.cp == 0x27 {
        state.mcu.cp += 0;
    }
    state.mcu.pc = address;
}

pub(crate) fn mcu_jump_jsr(state: &mut Sc55State, _operand: u8) {
    let mut address: u16;
    address = (mcu_read_code_advance(state) as u16) << 8;
    address |= mcu_read_code_advance(state) as u16;
    mcu_push_stack(state, state.mcu.pc);
    state.mcu.pc = address;
}

pub(crate) fn mcu_jump_rte(state: &mut Sc55State, _operand: u8) {
    state.mcu.sr = mcu_pop_stack(state);
    state.mcu.cp = mcu_pop_stack(state) as u8;
    state.mcu.pc = mcu_pop_stack(state);
    state.mcu.ex_ignore = 1;
}

pub(crate) fn mcu_jump_bcc(state: &mut Sc55State, operand: u8) {
    let disp = if operand & 0x10 != 0 {
        let hi = mcu_read_code_advance(state) as u16;
        let lo = mcu_read_code_advance(state) as u16;
        (hi << 8) | lo
    } else {
        mcu_read_code_advance(state) as i8 as u16
    };
    let cond = operand & 0x0F;

    let n = (state.mcu.sr & STATUS_N) != 0;
    let c = (state.mcu.sr & STATUS_C) != 0;
    let z = (state.mcu.sr & STATUS_Z) != 0;
    let v = (state.mcu.sr & STATUS_V) != 0;

    let branch = match cond {
        0x0 => 1,                                                      // BRA/BT
        0x1 => 0,                                                      // BRN/BF
        0x2 => ((c as u32) | (z as u32) == 0) as u32,                  // BHI
        0x3 => ((c as u32) | (z as u32) == 1) as u32,                  // BLS
        0x4 => (!c) as u32,                                            // BCC/BHS
        0x5 => c as u32,                                               // BCS/BLO
        0x6 => (!z) as u32,                                            // BNE
        0x7 => z as u32,                                               // BEQ
        0x8 => (!v) as u32,                                            // BVC
        0x9 => v as u32,                                               // BVS
        0xA => (!n) as u32,                                            // BPL
        0xB => n as u32,                                               // BMI
        0xC => ((n as u32) ^ (v as u32) == 0) as u32,                  // BGE
        0xD => ((n as u32) ^ (v as u32) == 1) as u32,                  // BLT
        0xE => (((z as u32) | ((n as u32) ^ (v as u32))) == 0) as u32, // BGT
        0xF => (((z as u32) | ((n as u32) ^ (v as u32))) == 1) as u32, // BLE
        _ => 0,
    };

    if branch != 0 {
        state.mcu.pc = state.mcu.pc.wrapping_add(disp);
    }
}

pub(crate) fn mcu_jump_rts(state: &mut Sc55State, _operand: u8) {
    state.mcu.pc = mcu_pop_stack(state);
}

pub(crate) fn mcu_jump_rtd(state: &mut Sc55State, operand: u8) {
    let imm = mcu_read_code_advance(state) as i8 as i16;
    state.mcu.pc = mcu_pop_stack(state);

    if operand == 0x14 {
        state.mcu.r[7] = state.mcu.r[7].wrapping_add(imm as u16);
        if state.mcu.r[7] & 1 != 0 {
            mcu_error_trap(state);
        }
    } else {
        mcu_error_trap(state);
    }
}

pub(crate) fn mcu_jump_jmp(state: &mut Sc55State, operand: u8) {
    if operand == 0x11 {
        let opcode = mcu_read_code_advance(state);
        let opcode_h = opcode >> 3;
        let opcode_l = opcode & 0x07;
        if opcode == 0x19 {
            state.mcu.cp = mcu_pop_stack(state) as u8;
            state.mcu.pc = mcu_pop_stack(state);
        } else if opcode_h == 0x19 {
            mcu_push_stack(state, state.mcu.pc);
            mcu_push_stack(state, state.mcu.cp as u16);
            let opcode_l = opcode_l & !1;
            state.mcu.cp = (state.mcu.r[opcode_l as usize] & 0xFF) as u8;
            state.mcu.pc = state.mcu.r[(opcode_l + 1) as usize];
        } else if opcode_h == 0x1A {
            state.mcu.pc = state.mcu.r[opcode_l as usize];
        } else if opcode_h == 0x1B {
            mcu_push_stack(state, state.mcu.pc);
            state.mcu.pc = state.mcu.r[opcode_l as usize];
        } else {
            mcu_error_trap(state);
        }
    } else if operand == 0x01 {
        let opcode = mcu_read_code_advance(state);
        let reg = (opcode & 0x07) as usize;
        let opcode_shifted = opcode >> 3;
        if opcode_shifted == 0x17 {
            let disp = mcu_read_code_advance(state) as i8 as u16;
            state.mcu.r[reg] = state.mcu.r[reg].wrapping_sub(1);
            if state.mcu.r[reg] != 0xFFFF {
                state.mcu.pc = state.mcu.pc.wrapping_add(disp);
            }
        } else {
            mcu_error_trap(state);
        }
    } else if operand == 0x10 {
        let mut addr: u32;
        addr = (mcu_read_code_advance(state) as u32) << 8;
        addr |= mcu_read_code_advance(state) as u32;
        state.mcu.pc = addr as u16;
    } else if operand == 0x06 {
        let opcode = mcu_read_code_advance(state);
        let reg = (opcode & 0x07) as usize;
        let opcode_shifted = opcode >> 3;
        if opcode_shifted == 0x17 {
            let disp = mcu_read_code_advance(state) as i8 as u16;
            let z = (state.mcu.sr & STATUS_Z) != 0;
            if z {
                state.mcu.r[reg] = state.mcu.r[reg].wrapping_sub(1);
                if state.mcu.r[reg] != 0xFFFF {
                    state.mcu.pc = state.mcu.pc.wrapping_add(disp);
                }
            }
        } else {
            mcu_error_trap(state);
        }
    } else if operand == 0x07 {
        let opcode = mcu_read_code_advance(state);
        let reg = (opcode & 0x07) as usize;
        let opcode_shifted = opcode >> 3;
        if opcode_shifted == 0x17 {
            let disp = mcu_read_code_advance(state) as i8 as u16;
            let z = (state.mcu.sr & STATUS_Z) != 0;
            if !z {
                state.mcu.r[reg] = state.mcu.r[reg].wrapping_sub(1);
                if state.mcu.r[reg] != 0xFFFF {
                    state.mcu.pc = state.mcu.pc.wrapping_add(disp);
                }
            }
        } else {
            mcu_error_trap(state);
        }
    } else {
        mcu_error_trap(state);
    }
}

pub(crate) fn mcu_jump_bsr(state: &mut Sc55State, operand: u8) {
    let disp = if operand == 0x0E {
        mcu_read_code_advance(state) as i8 as u16
    } else {
        let hi = mcu_read_code_advance(state) as u16;
        let lo = mcu_read_code_advance(state) as u16;
        (hi << 8) | lo
    };
    mcu_push_stack(state, state.mcu.pc);
    state.mcu.pc = state.mcu.pc.wrapping_add(disp);
}

pub(crate) fn mcu_jump_pjmp(state: &mut Sc55State, _operand: u8) {
    let mut address: u16;
    let page = mcu_read_code_advance(state);
    address = (mcu_read_code_advance(state) as u16) << 8;
    address |= mcu_read_code_advance(state) as u16;
    state.mcu.cp = page;
    state.mcu.pc = address;
}

pub(crate) fn mcu_operand_read(state: &mut Sc55State) -> u32 {
    match state.operand_type {
        GENERAL_DIRECT => {
            if state.operand_size != 0 {
                return state.mcu.r[state.operand_reg as usize] as u32;
            }
            return (state.mcu.r[state.operand_reg as usize] & 0xFF) as u32;
        }
        GENERAL_INDIRECT | GENERAL_ABSOLUTE => {
            if state.operand_size != 0 {
                if state.operand_ea & 1 != 0 {
                    mcu_interrupt_exception(state, EXCEPTION_SOURCE_ADDRESS_ERROR as u32);
                }
                return mcu_read16(state, mcu_get_address(state.operand_ep, state.operand_ea))
                    as u32;
            }
            return mcu_read(state, mcu_get_address(state.operand_ep, state.operand_ea)) as u32;
        }
        GENERAL_IMMEDIATE => {
            return state.operand_data as u32;
        }
        _ => {}
    }
    0
}

pub(crate) fn mcu_operand_write(state: &mut Sc55State, data: u32) {
    match state.operand_type {
        GENERAL_DIRECT => {
            if state.operand_size != 0 {
                state.mcu.r[state.operand_reg as usize] = data as u16;
            } else {
                state.mcu.r[state.operand_reg as usize] &= !0xFF;
                state.mcu.r[state.operand_reg as usize] |= (data & 0xFF) as u16;
            }
        }
        GENERAL_INDIRECT | GENERAL_ABSOLUTE => {
            if state.operand_size != 0 {
                if state.operand_ea & 1 != 0 {
                    mcu_interrupt_exception(state, EXCEPTION_SOURCE_ADDRESS_ERROR as u32);
                }
                mcu_write16(
                    state,
                    mcu_get_address(state.operand_ep, state.operand_ea),
                    data as u16,
                );
            } else {
                mcu_write(
                    state,
                    mcu_get_address(state.operand_ep, state.operand_ea),
                    data as u8,
                );
            }
        }
        GENERAL_IMMEDIATE => {
            mcu_interrupt_exception(state, EXCEPTION_SOURCE_INVALID_INSTRUCTION as u32);
        }
        _ => {}
    }
}

pub(crate) fn mcu_operand_general(state: &mut Sc55State, operand: u8) {
    let mut type_ = GENERAL_DIRECT;
    let mut disp: u32 = 0;
    let mut increase = INCREASE_NONE;
    let mut data: u32 = 0;
    let mut addr: u32 = 0;
    let mut addrpage: u32 = 0;
    let mut ea: u32 = 0;
    let mut ep: u32 = 0;
    let siz = if operand & 0x08 != 0 {
        OPERAND_WORD as u32
    } else {
        OPERAND_BYTE as u32
    };
    let reg = (operand & 0x07) as u32;
    match operand & 0xF0 {
        0xA0 => {
            type_ = GENERAL_DIRECT;
        }
        0xD0 => {
            type_ = GENERAL_INDIRECT;
        }
        0xE0 => {
            type_ = GENERAL_INDIRECT;
            disp = mcu_read_code_advance(state) as i8 as u32;
        }
        0xF0 => {
            type_ = GENERAL_INDIRECT;
            disp = (mcu_read_code_advance(state) as u32) << 8;
            disp |= mcu_read_code_advance(state) as u32;
        }
        0xB0 => {
            type_ = GENERAL_INDIRECT;
            increase = INCREASE_DECREASE;
        }
        0xC0 => {
            type_ = GENERAL_INDIRECT;
            increase = INCREASE_INCREASE;
        }
        0x00 => {
            if reg == 5 {
                type_ = GENERAL_ABSOLUTE;
                addr = (state.mcu.br as u32) << 8;
                addr |= mcu_read_code_advance(state) as u32;
                addrpage = 0;
            } else if reg == 4 {
                type_ = GENERAL_IMMEDIATE;
                data = mcu_read_code_advance(state) as u32;
                if siz != 0 {
                    data <<= 8;
                    data |= mcu_read_code_advance(state) as u32;
                }
            }
        }
        0x10 if reg == 5 => {
            type_ = GENERAL_ABSOLUTE;
            addr = (mcu_read_code_advance(state) as u32) << 8;
            addr |= mcu_read_code_advance(state) as u32;
            addrpage = state.mcu.dp as u32;
        }
        _ => {}
    }
    if type_ == GENERAL_INDIRECT {
        if increase == INCREASE_DECREASE {
            if siz != 0 || reg == 7 {
                state.mcu.r[reg as usize] = state.mcu.r[reg as usize].wrapping_sub(2);
            } else {
                state.mcu.r[reg as usize] = state.mcu.r[reg as usize].wrapping_sub(1);
            }
        }
        ea = (state.mcu.r[reg as usize] as u32).wrapping_add(disp);
        if increase == INCREASE_INCREASE {
            if siz != 0 || reg == 7 {
                state.mcu.r[reg as usize] = state.mcu.r[reg as usize].wrapping_add(2);
            } else {
                state.mcu.r[reg as usize] = state.mcu.r[reg as usize].wrapping_add(1);
            }
        }

        ea &= 0xFFFF;

        ep = mcu_get_page_for_register(state, reg) & 0xFF;
    } else if type_ == GENERAL_ABSOLUTE {
        ea = addr & 0xFFFF;

        ep = addrpage & 0xFF;
    }

    let opcode = mcu_read_code_advance(state);
    state.opcode_extended = (opcode == 0x00) as u8;
    let opcode = if state.opcode_extended != 0 {
        mcu_read_code_advance(state)
    } else {
        opcode
    };
    let opcode_reg = opcode & 0x07;
    let opcode = opcode >> 3;

    state.operand_type = type_;
    state.operand_ea = ea as u16;
    state.operand_ep = ep as u8;
    state.operand_size = siz as u8;
    state.operand_reg = reg as u8;
    state.operand_data = data as u16;
    state.operand_status = 0;

    mcu_opcode_dispatch(state, opcode, opcode_reg);
}

pub(crate) fn mcu_set_status_common(state: &mut Sc55State, mut val: u32, siz: u32) {
    if siz != 0 {
        val &= 0xFFFF;
    } else {
        val &= 0xFF;
    }
    if siz != 0 {
        mcu_set_status(state, val & 0x8000, STATUS_N);
    } else {
        mcu_set_status(state, val & 0x80, STATUS_N);
    }
    mcu_set_status(state, (val == 0) as u32, STATUS_Z);
    mcu_set_status(state, 0, STATUS_V);
}

pub(crate) fn mcu_opcode_short_move(state: &mut Sc55State, opcode: u8) {
    let reg = (opcode & 0x07) as usize;
    let data = mcu_read_code_advance(state);
    state.mcu.r[reg] &= !0xFF;
    state.mcu.r[reg] |= data as u16;
    mcu_set_status_common(state, data as u32, 0);
}

pub(crate) fn mcu_opcode_short_movi(state: &mut Sc55State, opcode: u8) {
    let reg = (opcode & 0x07) as usize;
    let mut data: u16;
    data = (mcu_read_code_advance(state) as u16) << 8;
    data |= mcu_read_code_advance(state) as u16;
    state.mcu.r[reg] = data;
    mcu_set_status_common(state, data as u32, 1);
}

pub(crate) fn mcu_opcode_short_movf(state: &mut Sc55State, opcode: u8) {
    let reg = (opcode & 0x07) as usize;
    let siz = (opcode & 0x08) != 0;
    let disp = mcu_read_code_advance(state) as i8;
    let addr = state.mcu.r[6].wrapping_add(disp as u16);
    let addr = (state.mcu.tp as u32) << 16 | addr as u32;
    if (opcode & 0x10) == 0 {
        let data: u16;
        if siz {
            data = mcu_read16(state, addr);
            state.mcu.r[reg] &= !0xFF;
            state.mcu.r[reg] |= data;
            mcu_set_status_common(state, data as u32, 0);
        } else {
            data = mcu_read(state, addr) as u16;
            state.mcu.r[reg] = data;
            mcu_set_status_common(state, data as u32, 1);
        }
    } else {
        let data: u16;
        if siz {
            data = state.mcu.r[reg] & 0xFF;
            mcu_write(state, addr, data as u8);
            mcu_set_status_common(state, data as u32, 0);
        } else {
            data = state.mcu.r[reg];
            mcu_write16(state, addr, data);
            mcu_set_status_common(state, data as u32, 1);
        }
    }
}

pub(crate) fn mcu_opcode_short_movl(state: &mut Sc55State, opcode: u8) {
    let reg = (opcode & 0x07) as usize;
    let siz = (opcode & 0x08) != 0;
    let mut addr: u16 = (state.mcu.br as u16) << 8;
    addr |= mcu_read_code_advance(state) as u16;
    if siz {
        if addr & 1 != 0 {
            mcu_interrupt_exception(state, EXCEPTION_SOURCE_ADDRESS_ERROR as u32);
        }
        let data = mcu_read16(state, addr as u32) as u32;
        state.mcu.r[reg] = data as u16;
        mcu_set_status_common(state, data, 1);
    } else {
        let data = mcu_read(state, addr as u32) as u32;
        state.mcu.r[reg] &= !0xFF;
        state.mcu.r[reg] |= data as u16;
        mcu_set_status_common(state, data, 0);
    }
}

pub(crate) fn mcu_opcode_short_movs(state: &mut Sc55State, opcode: u8) {
    let reg = (opcode & 0x07) as usize;
    let siz = (opcode & 0x08) != 0;
    let mut addr: u16 = (state.mcu.br as u16) << 8;
    addr |= mcu_read_code_advance(state) as u16;
    if siz {
        if addr & 1 != 0 {
            mcu_interrupt_exception(state, EXCEPTION_SOURCE_ADDRESS_ERROR as u32);
        }
        let data = state.mcu.r[reg] as u32;
        mcu_write16(state, addr as u32, data as u16);
        mcu_set_status_common(state, data, 1);
    } else {
        let data = (state.mcu.r[reg] & 0xFF) as u32;
        mcu_write(state, addr as u32, data as u8);
        mcu_set_status_common(state, data, 0);
    }
}

pub(crate) fn mcu_opcode_short_cmp(state: &mut Sc55State, opcode: u8) {
    let reg = (opcode & 0x07) as usize;
    let siz = ((opcode & 0x08) != 0) as u32;
    let t2 = if siz != 0 {
        ((mcu_read_code_advance(state) as i32) << 8) | mcu_read_code_advance(state) as i32
    } else {
        mcu_read_code_advance(state) as i32
    };
    let t1 = state.mcu.r[reg] as i32;
    mcu_sub_common(state, t1, t2, 0, siz);
}

pub(crate) fn mcu_opcode_not_implemented(state: &mut Sc55State, _opcode: u8, _opcode_reg: u8) {
    mcu_error_trap(state);
}

pub(crate) fn mcu_opcode_movg_immediate(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    if opcode_reg == 6
        && (state.operand_type == GENERAL_INDIRECT || state.operand_type == GENERAL_ABSOLUTE)
    {
        let data = mcu_read_code_advance(state) as i8 as u32;
        mcu_operand_write(state, data);
        mcu_set_status_common(state, data, state.operand_size as u32);
    } else if opcode_reg == 7
        && (state.operand_type == GENERAL_INDIRECT || state.operand_type == GENERAL_ABSOLUTE)
    {
        let mut data: u32;
        data = (mcu_read_code_advance(state) as u32) << 8;
        data |= mcu_read_code_advance(state) as u32;
        mcu_operand_write(state, data);
        mcu_set_status_common(state, data, state.operand_size as u32);
    } else if opcode_reg == 4
        && (state.operand_type == GENERAL_INDIRECT || state.operand_type == GENERAL_ABSOLUTE)
        && state.operand_size == OPERAND_BYTE
    {
        let t1 = mcu_operand_read(state);
        let t2 = mcu_read_code_advance(state) as u32;
        mcu_sub_common(state, t1 as i32, t2 as i32, 0, OPERAND_BYTE as u32);
    } else if opcode_reg == 4
        && (state.operand_type == GENERAL_INDIRECT || state.operand_type == GENERAL_ABSOLUTE)
        && state.operand_size == OPERAND_WORD
    // FIXME
    {
        let t1 = mcu_operand_read(state);
        let t2 = (mcu_read_code_advance(state) as i8 as i16 as u16) as u32;
        mcu_sub_common(state, t1 as i32, t2 as i32, 0, OPERAND_WORD as u32);
    } else if opcode_reg == 5
        && (state.operand_type == GENERAL_INDIRECT || state.operand_type == GENERAL_ABSOLUTE)
        && state.operand_size == OPERAND_WORD
    {
        let t1 = mcu_operand_read(state);
        let mut t2: u32;
        t2 = (mcu_read_code_advance(state) as u32) << 8;
        t2 |= mcu_read_code_advance(state) as u32;
        mcu_sub_common(state, t1 as i32, t2 as i32, 0, OPERAND_WORD as u32);
    } else if opcode_reg == 5
        && (state.operand_type == GENERAL_INDIRECT || state.operand_type == GENERAL_ABSOLUTE)
        && state.operand_size == OPERAND_BYTE
    // FIXME
    {
        let t1 = mcu_operand_read(state);
        let mut t2: u32;
        t2 = (mcu_read_code_advance(state) as u32) << 8;
        t2 |= mcu_read_code_advance(state) as u32;
        mcu_sub_common(state, t1 as i32, t2 as i32, 0, OPERAND_BYTE as u32);
    } else {
        mcu_error_trap(state);
    }
}

pub(crate) fn mcu_opcode_bset_orc(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    if state.operand_type == GENERAL_IMMEDIATE {
        // ORC
        let data = mcu_operand_read(state);
        let mut val =
            mcu_control_register_read(state, opcode_reg as u32, state.operand_size as u32);
        val |= data;
        mcu_control_register_write(state, opcode_reg as u32, state.operand_size as u32, val);
        if opcode_reg >= 2 {
            mcu_set_status_common(state, val, state.operand_size as u32);
        }
        state.mcu.ex_ignore = 1;
    } else {
        // BSET
        let mut data = mcu_operand_read(state);
        let bit = state.mcu.r[opcode_reg as usize] & 0x0F;
        mcu_set_status(state, (data & (1 << bit) == 0) as u32, STATUS_Z);
        data |= 1 << bit;
        mcu_operand_write(state, data);
    }
}

pub(crate) fn mcu_opcode_bclr_andc(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    if state.operand_type == GENERAL_IMMEDIATE {
        // ANDC
        let data = mcu_operand_read(state);
        let mut val =
            mcu_control_register_read(state, opcode_reg as u32, state.operand_size as u32);
        val &= data;
        mcu_control_register_write(state, opcode_reg as u32, state.operand_size as u32, val);
        if opcode_reg >= 2 {
            mcu_set_status_common(state, val, state.operand_size as u32);
        }
        state.mcu.ex_ignore = 1;
    } else {
        // BCLR
        let mut data = mcu_operand_read(state);
        let bit = state.mcu.r[opcode_reg as usize] & 0x0F;
        mcu_set_status(state, (data & (1 << bit) == 0) as u32, STATUS_Z);
        data &= !(1 << bit);
        mcu_operand_write(state, data);
    }
}

pub(crate) fn mcu_opcode_btst(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    if state.operand_type != GENERAL_IMMEDIATE {
        let data = mcu_operand_read(state);
        let bit = state.mcu.r[opcode_reg as usize] & 0x0F;
        mcu_set_status(state, (data & (1 << bit) == 0) as u32, STATUS_Z);
    } else {
        mcu_error_trap(state);
    }
}

pub(crate) fn mcu_opcode_clr(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    if opcode_reg == 3 && state.operand_type != GENERAL_IMMEDIATE {
        // CLR
        mcu_operand_write(state, 0);
        mcu_set_status(state, 0, STATUS_N);
        mcu_set_status(state, 1, STATUS_Z);
        mcu_set_status(state, 0, STATUS_V);
        mcu_set_status(state, 0, STATUS_C);
    } else if opcode_reg == 6 && state.operand_type != GENERAL_IMMEDIATE {
        // TST
        let data = mcu_operand_read(state);
        mcu_set_status_common(state, data, state.operand_size as u32);
        mcu_set_status(state, 0, STATUS_C);
    } else if opcode_reg == 2 && state.operand_type == GENERAL_DIRECT && state.operand_size == 0 {
        // EXTU
        let data = state.mcu.r[state.operand_reg as usize] as u8 as u32;
        state.mcu.r[state.operand_reg as usize] = data as u16;
        mcu_set_status(state, 0, STATUS_N);
        mcu_set_status(state, (data == 0) as u32, STATUS_Z);
        mcu_set_status(state, 0, STATUS_V);
        mcu_set_status(state, 0, STATUS_C);
    } else if opcode_reg == 0 && state.operand_type == GENERAL_DIRECT && state.operand_size == 0 {
        // SWAP
        let data = state.mcu.r[state.operand_reg as usize] as u32;
        let data_h = data >> 8;
        let data_l = data & 0xFF;
        let data = (data_l << 8) | data_h;
        state.mcu.r[state.operand_reg as usize] = data as u16;
        mcu_set_status_common(state, data, OPERAND_WORD as u32);
    } else if opcode_reg == 5 && state.operand_type != GENERAL_IMMEDIATE {
        // NOT
        let mut data = mcu_operand_read(state);
        data = !data;
        mcu_operand_write(state, data);
        mcu_set_status_common(state, data, state.operand_size as u32);
    } else if opcode_reg == 4 && state.operand_type != GENERAL_IMMEDIATE {
        // NEG
        let data = mcu_operand_read(state);
        let data = mcu_sub_common(state, 0, data as i32, 0, state.operand_size as u32);
        mcu_operand_write(state, data as u32);
    } else if opcode_reg == 1 && state.operand_type == GENERAL_DIRECT && state.operand_size == 0 {
        // EXTS
        let data = state.mcu.r[state.operand_reg as usize];
        state.mcu.r[state.operand_reg as usize] = data as u8 as i8 as u16;
        mcu_set_status_common(state, data as u32, OPERAND_WORD as u32);
    } else {
        mcu_error_trap(state);
    }
}

pub(crate) fn mcu_opcode_ldc(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    let data = mcu_operand_read(state);
    mcu_control_register_write(state, opcode_reg as u32, state.operand_size as u32, data);
    state.mcu.ex_ignore = 1;
}

pub(crate) fn mcu_opcode_stc(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    let data = mcu_control_register_read(state, opcode_reg as u32, state.operand_size as u32);
    mcu_operand_write(state, data);
}

pub(crate) fn mcu_opcode_bset(state: &mut Sc55State, opcode: u8, opcode_reg: u8) {
    if state.operand_type != GENERAL_IMMEDIATE {
        let mut data = mcu_operand_read(state);
        let bit = (opcode_reg as u32) | (((opcode & 1) as u32) << 3);
        mcu_set_status(state, (data & (1 << bit) == 0) as u32, STATUS_Z);
        data |= 1 << bit;
        mcu_operand_write(state, data);
    } else {
        mcu_error_trap(state);
    }
}

pub(crate) fn mcu_opcode_bclr(state: &mut Sc55State, opcode: u8, opcode_reg: u8) {
    if state.operand_type != GENERAL_IMMEDIATE {
        let mut data = mcu_operand_read(state);
        let bit = (opcode_reg as u32) | (((opcode & 1) as u32) << 3);
        mcu_set_status(state, (data & (1 << bit) == 0) as u32, STATUS_Z);
        data &= !(1 << bit);
        mcu_operand_write(state, data);
    } else {
        mcu_error_trap(state);
    }
}

pub(crate) fn mcu_opcode_movg(state: &mut Sc55State, opcode: u8, opcode_reg: u8) {
    if state.opcode_extended != 0 {
        if opcode == 0x12 {
            // FIXME
            mcu_error_trap(state);
        } else {
            mcu_error_trap(state);
        }
    } else {
        let d = (opcode & 2) != 0;
        if d {
            if state.operand_type == GENERAL_DIRECT {
                // XCH
                if state.operand_size != 0 {
                    let r1 = state.mcu.r[opcode_reg as usize];
                    let r2 = state.mcu.r[state.operand_reg as usize];
                    state.mcu.r[opcode_reg as usize] = r2;
                    state.mcu.r[state.operand_reg as usize] = r1;
                } else {
                    mcu_error_trap(state);
                }
            } else {
                let data = state.mcu.r[opcode_reg as usize] as u32;
                mcu_operand_write(state, data);
                mcu_set_status_common(state, data, state.operand_size as u32);
            }
        } else {
            let data = mcu_operand_read(state);
            if state.operand_size != 0 {
                state.mcu.r[opcode_reg as usize] = data as u16;
            } else {
                state.mcu.r[opcode_reg as usize] &= !0xFF;
                state.mcu.r[opcode_reg as usize] |= (data & 0xFF) as u16;
            }
            mcu_set_status_common(state, data, state.operand_size as u32);
        }
    }
}

pub(crate) fn mcu_opcode_btsti(state: &mut Sc55State, opcode: u8, opcode_reg: u8) {
    if state.operand_type != GENERAL_IMMEDIATE {
        let data = mcu_operand_read(state);
        let bit = (opcode_reg as u32) | (((opcode & 1) as u32) << 3);
        mcu_set_status(state, (data & (1 << bit) == 0) as u32, STATUS_Z);
    } else {
        mcu_error_trap(state);
    }
}

pub(crate) fn mcu_opcode_bnoti(state: &mut Sc55State, opcode: u8, opcode_reg: u8) {
    if state.operand_type != GENERAL_IMMEDIATE {
        let mut data = mcu_operand_read(state);
        let bit = (opcode_reg as u32) | (((opcode & 1) as u32) << 3);
        mcu_set_status(state, (data & (1 << bit) == 0) as u32, STATUS_Z);
        data ^= 1 << bit;
        mcu_operand_write(state, data);
    } else {
        mcu_error_trap(state);
    }
}

pub(crate) fn mcu_opcode_or(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    let data = mcu_operand_read(state);
    state.mcu.r[opcode_reg as usize] |= data as u16;
    mcu_set_status_common(
        state,
        state.mcu.r[opcode_reg as usize] as u32,
        state.operand_size as u32,
    );
}

pub(crate) fn mcu_opcode_cmp(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    let t1 = state.mcu.r[opcode_reg as usize] as i32;
    let t2 = mcu_operand_read(state) as i32;
    mcu_sub_common(state, t1, t2, 0, state.operand_size as u32);
}

pub(crate) fn mcu_opcode_addq(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    let mut t1 = mcu_operand_read(state) as i32;
    let t2 = match opcode_reg {
        0 => 1,
        1 => 2,
        4 => -1,
        5 => -2,
        _ => {
            mcu_error_trap(state);
            0
        }
    };
    t1 = mcu_add_common(state, t1, t2, 0, state.operand_size as u32);
    mcu_operand_write(state, t1 as u32);
}

pub(crate) fn mcu_opcode_add(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    let mut t1 = state.mcu.r[opcode_reg as usize] as i32;
    let t2 = mcu_operand_read(state) as i32;
    t1 = mcu_add_common(state, t1, t2, 0, state.operand_size as u32);
    if state.operand_size != 0 {
        state.mcu.r[opcode_reg as usize] = t1 as u16;
    } else {
        state.mcu.r[opcode_reg as usize] &= !0xFF;
        state.mcu.r[opcode_reg as usize] |= (t1 & 0xFF) as u16;
    }
}

pub(crate) fn mcu_opcode_sub(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    let mut t1 = state.mcu.r[opcode_reg as usize] as i32;
    let t2 = mcu_operand_read(state) as i32;
    t1 = mcu_sub_common(state, t1, t2, 0, state.operand_size as u32);
    if state.operand_size != 0 {
        state.mcu.r[opcode_reg as usize] = t1 as u16;
    } else {
        state.mcu.r[opcode_reg as usize] &= !0xFF;
        state.mcu.r[opcode_reg as usize] |= (t1 & 0xFF) as u16;
    }
}

pub(crate) fn mcu_opcode_subs(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    let t1 = state.mcu.r[opcode_reg as usize] as i32;
    let t2 = mcu_operand_read(state) as i32;
    if state.operand_size != 0 {
        state.mcu.r[opcode_reg as usize] = t1.wrapping_sub(t2) as u16;
    } else {
        state.mcu.r[opcode_reg as usize] = t1.wrapping_sub(t2 as i8 as i32) as u16;
    }
}

pub(crate) fn mcu_opcode_and(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    let mut data = state.mcu.r[opcode_reg as usize] as u32;
    data &= mcu_operand_read(state);
    if state.operand_size != 0 {
        state.mcu.r[opcode_reg as usize] = data as u16;
    } else {
        state.mcu.r[opcode_reg as usize] &= !0xFF;
        state.mcu.r[opcode_reg as usize] |= (data & 0xFF) as u16;
    }
    mcu_set_status_common(
        state,
        state.mcu.r[opcode_reg as usize] as u32,
        state.operand_size as u32,
    );
}

pub(crate) fn mcu_opcode_shlr(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    if opcode_reg == 0x03 && state.operand_type != GENERAL_IMMEDIATE {
        // SHLR
        let mut data = mcu_operand_read(state);
        let c = data & 1;
        data >>= 1;
        mcu_operand_write(state, data);
        mcu_set_status(state, c, STATUS_C);
        mcu_set_status_common(state, data, state.operand_size as u32);
    } else if opcode_reg == 0x02 && state.operand_type != GENERAL_IMMEDIATE {
        // SHLL
        let mut data = mcu_operand_read(state);
        let c = if state.operand_size != 0 {
            (data & 0x8000 != 0) as u32
        } else {
            (data & 0x80 != 0) as u32
        };
        data <<= 1;
        mcu_operand_write(state, data);
        mcu_set_status(state, c, STATUS_C);
        mcu_set_status_common(state, data, state.operand_size as u32);
    } else if opcode_reg == 0x06 && state.operand_type != GENERAL_IMMEDIATE {
        // ROTXL
        let mut data = mcu_operand_read(state);
        let bit = (state.mcu.sr & STATUS_C != 0) as u32;
        let c = if state.operand_size != 0 {
            (data & 0x8000 != 0) as u32
        } else {
            (data & 0x80 != 0) as u32
        };
        data <<= 1;
        data |= bit;
        mcu_operand_write(state, data);
        mcu_set_status(state, c, STATUS_C);
        mcu_set_status_common(state, data, state.operand_size as u32);
    } else if opcode_reg == 0x04 && state.operand_type != GENERAL_IMMEDIATE {
        // ROTL
        let mut data = mcu_operand_read(state);
        let c = if state.operand_size != 0 {
            (data & 0x8000 != 0) as u32
        } else {
            (data & 0x80 != 0) as u32
        };
        data <<= 1;
        data |= c;
        mcu_operand_write(state, data);
        mcu_set_status(state, c, STATUS_C);
        mcu_set_status_common(state, data, state.operand_size as u32);
    } else if opcode_reg == 0x00 && state.operand_type != GENERAL_IMMEDIATE {
        // SHAL
        let mut data = mcu_operand_read(state);
        let c = if state.operand_size != 0 {
            (data & 0x8000 != 0) as u32
        } else {
            (data & 0x80 != 0) as u32
        };
        data <<= 1;
        mcu_operand_write(state, data);
        mcu_set_status(state, c, STATUS_C);
        mcu_set_status_common(state, data, state.operand_size as u32);
    } else if opcode_reg == 0x01 && state.operand_type != GENERAL_IMMEDIATE {
        // SHAR
        let mut data = mcu_operand_read(state);
        let c = data & 0x1;
        let msb: u32;
        if state.operand_size != 0 {
            msb = data & 0x8000;
            data &= 0xFFFF;
        } else {
            msb = data & 0x80;
            data &= 0xFF;
        }
        data >>= 1;
        data |= msb;
        mcu_operand_write(state, data);
        mcu_set_status(state, c, STATUS_C);
        mcu_set_status_common(state, data, state.operand_size as u32);
    } else if opcode_reg == 0x05 && state.operand_type != GENERAL_IMMEDIATE {
        // ROTR
        let mut data = mcu_operand_read(state);
        let c = (data & 0x1 != 0) as u32;
        data >>= 1;
        if state.operand_size != 0 {
            data |= c << 15;
        } else {
            data |= c << 7;
        }
        mcu_operand_write(state, data);
        mcu_set_status(state, c, STATUS_C);
        mcu_set_status_common(state, data, state.operand_size as u32);
    } else {
        mcu_error_trap(state);
    }
}

pub(crate) fn mcu_opcode_mulxu(state: &mut Sc55State, _opcode: u8, mut opcode_reg: u8) {
    let t1 = mcu_operand_read(state);
    let mut t2 = state.mcu.r[opcode_reg as usize] as u32;
    let n: u32;
    if state.operand_size == 0 {
        t2 &= 0xFF;
    }
    let mut t1 = t1.wrapping_mul(t2);

    if state.operand_size != 0 {
        opcode_reg &= !1;
        state.mcu.r[opcode_reg as usize] = (t1 >> 16) as u16;
        state.mcu.r[(opcode_reg | 1) as usize] = t1 as u16;
        n = (t1 & 0x80000000) >> 31; // FIXME
    } else {
        t1 &= 0xFFFF;
        state.mcu.r[opcode_reg as usize] = t1 as u16;
        n = (t1 & 0x8000 != 0) as u32; // FIXME
    }
    let z = (t1 == 0) as u32;
    mcu_set_status(state, n, STATUS_N);
    mcu_set_status(state, z, STATUS_Z);
    mcu_set_status(state, 0, STATUS_V);
    mcu_set_status(state, 0, STATUS_C);
}

pub(crate) fn mcu_opcode_divxu(state: &mut Sc55State, _opcode: u8, mut opcode_reg: u8) {
    let t1 = mcu_operand_read(state);

    if t1 == 0 {
        mcu_error_trap(state); // FIXME: implement proper exception
        mcu_set_status(state, 0, STATUS_N);
        mcu_set_status(state, 1, STATUS_Z);
        mcu_set_status(state, 0, STATUS_V);
        mcu_set_status(state, 0, STATUS_C);
        return;
    }

    if state.operand_size != 0 {
        opcode_reg &= !1;
        let t2 = ((state.mcu.r[opcode_reg as usize] as u32) << 16)
            | state.mcu.r[(opcode_reg | 1) as usize] as u32;

        let r = t2 % t1;
        let q = t2 / t1;

        if q > u16::MAX as u32 {
            mcu_set_status(state, 0, STATUS_N);
            mcu_set_status(state, 0, STATUS_Z);
            mcu_set_status(state, 1, STATUS_V);
            mcu_set_status(state, 0, STATUS_C);
        } else {
            state.mcu.r[opcode_reg as usize] = r as u16;
            state.mcu.r[(opcode_reg | 1) as usize] = q as u16;
            mcu_set_status_common(state, q, OPERAND_WORD as u32);
            mcu_set_status(state, 0, STATUS_C);
        }
    } else {
        let t2 = state.mcu.r[opcode_reg as usize] as u32;

        let r = t2 % t1;
        let q = t2 / t1;

        if q > u8::MAX as u32 {
            mcu_set_status(state, 0, STATUS_N);
            mcu_set_status(state, 0, STATUS_Z);
            mcu_set_status(state, 1, STATUS_V);
            mcu_set_status(state, 0, STATUS_C);
        } else {
            let r = r & 0xFF;
            let q = q & 0xFF;
            state.mcu.r[opcode_reg as usize] = ((r << 8) | q) as u16;
            mcu_set_status_common(state, q, OPERAND_BYTE as u32);
            mcu_set_status(state, 0, STATUS_C);
        }
    }
}

pub(crate) fn mcu_opcode_adds(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    let data = mcu_operand_read(state);
    if state.operand_size == 0 {
        let data = data as u8 as i8 as i16 as u16;
        state.mcu.r[opcode_reg as usize] = state.mcu.r[opcode_reg as usize].wrapping_add(data);
    } else {
        state.mcu.r[opcode_reg as usize] =
            state.mcu.r[opcode_reg as usize].wrapping_add(data as u16);
    }
}

pub(crate) fn mcu_opcode_xor(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    let data = mcu_operand_read(state);
    state.mcu.r[opcode_reg as usize] ^= data as u16;
    mcu_set_status_common(
        state,
        state.mcu.r[opcode_reg as usize] as u32,
        state.operand_size as u32,
    );
}

pub(crate) fn mcu_opcode_addx(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    let mut t1 = state.mcu.r[opcode_reg as usize] as i32;
    let t2 = mcu_operand_read(state) as i32;
    let c = (state.mcu.sr & STATUS_C != 0) as i32;
    let z = (state.mcu.sr & STATUS_Z != 0) as i32;
    t1 = mcu_add_common(state, t1, t2, c, state.operand_size as u32);
    if z == 0 {
        mcu_set_status(state, 0, STATUS_Z);
    }

    if state.operand_size != 0 {
        state.mcu.r[opcode_reg as usize] = t1 as u16;
    } else {
        state.mcu.r[opcode_reg as usize] &= !0xFF;
        state.mcu.r[opcode_reg as usize] |= (t1 & 0xFF) as u16;
    }
}

pub(crate) fn mcu_opcode_subx(state: &mut Sc55State, _opcode: u8, opcode_reg: u8) {
    let mut t1 = state.mcu.r[opcode_reg as usize] as i32;
    let t2 = mcu_operand_read(state) as i32;
    let c = (state.mcu.sr & STATUS_C != 0) as i32;
    t1 = mcu_sub_common(state, t1, t2, c, state.operand_size as u32);
    if state.operand_size != 0 {
        state.mcu.r[opcode_reg as usize] = t1 as u16;
    } else {
        state.mcu.r[opcode_reg as usize] &= !0xFF;
        state.mcu.r[opcode_reg as usize] |= (t1 & 0xFF) as u16;
    }
}

pub(crate) fn mcu_operand_dispatch(state: &mut Sc55State, operand: u8) {
    match operand {
        0x00 => mcu_operand_nop(state, operand),
        0x01 | 0x06 | 0x07 | 0x10 | 0x11 => mcu_jump_jmp(state, operand),
        0x02 => mcu_ldm(state, operand),
        0x03 => mcu_jump_pjsr(state, operand),
        0x04 | 0x05 | 0x0C | 0x0D | 0x15 | 0x1D => mcu_operand_general(state, operand),
        0x08 => mcu_trapa(state, operand),
        0x09 | 0x0B | 0x0F | 0x16 | 0x17 | 0x1B | 0x1F => {
            mcu_operand_not_implemented(state, operand)
        }
        0x0A => mcu_jump_rte(state, operand),
        0x0E | 0x1E => mcu_jump_bsr(state, operand),
        0x12 => mcu_stm(state, operand),
        0x13 => mcu_jump_pjmp(state, operand),
        0x14 | 0x1C => mcu_jump_rtd(state, operand),
        0x18 => mcu_jump_jsr(state, operand),
        0x19 => mcu_jump_rts(state, operand),
        0x1A => mcu_operand_sleep(state, operand),
        0x20..=0x3F => mcu_jump_bcc(state, operand),
        0x40..=0x4F => mcu_opcode_short_cmp(state, operand),
        0x50..=0x57 => mcu_opcode_short_move(state, operand),
        0x58..=0x5F => mcu_opcode_short_movi(state, operand),
        0x60..=0x6F => mcu_opcode_short_movl(state, operand),
        0x70..=0x7F => mcu_opcode_short_movs(state, operand),
        0x80..=0x9F => mcu_opcode_short_movf(state, operand),
        0xA0..=0xFF => mcu_operand_general(state, operand),
    }
}

fn mcu_opcode_dispatch(state: &mut Sc55State, opcode: u8, opcode_reg: u8) {
    match opcode {
        0x00 => mcu_opcode_movg_immediate(state, opcode, opcode_reg),
        0x01 => mcu_opcode_addq(state, opcode, opcode_reg),
        0x02 => mcu_opcode_clr(state, opcode, opcode_reg),
        0x03 => mcu_opcode_shlr(state, opcode, opcode_reg),
        0x04 => mcu_opcode_add(state, opcode, opcode_reg),
        0x05 => mcu_opcode_adds(state, opcode, opcode_reg),
        0x06 => mcu_opcode_sub(state, opcode, opcode_reg),
        0x07 => mcu_opcode_subs(state, opcode, opcode_reg),
        0x08 => mcu_opcode_or(state, opcode, opcode_reg),
        0x09 => mcu_opcode_bset_orc(state, opcode, opcode_reg),
        0x0A => mcu_opcode_and(state, opcode, opcode_reg),
        0x0B => mcu_opcode_bclr_andc(state, opcode, opcode_reg),
        0x0C => mcu_opcode_xor(state, opcode, opcode_reg),
        0x0D => mcu_opcode_not_implemented(state, opcode, opcode_reg),
        0x0E => mcu_opcode_cmp(state, opcode, opcode_reg),
        0x0F => mcu_opcode_btst(state, opcode, opcode_reg),
        0x10 | 0x12 => mcu_opcode_movg(state, opcode, opcode_reg),
        0x11 => mcu_opcode_ldc(state, opcode, opcode_reg),
        0x13 => mcu_opcode_stc(state, opcode, opcode_reg),
        0x14 => mcu_opcode_addx(state, opcode, opcode_reg),
        0x15 => mcu_opcode_mulxu(state, opcode, opcode_reg),
        0x16 => mcu_opcode_subx(state, opcode, opcode_reg),
        0x17 => mcu_opcode_divxu(state, opcode, opcode_reg),
        0x18 | 0x19 => mcu_opcode_bset(state, opcode, opcode_reg),
        0x1A | 0x1B => mcu_opcode_bclr(state, opcode, opcode_reg),
        0x1C | 0x1D => mcu_opcode_bnoti(state, opcode, opcode_reg),
        0x1E | 0x1F => mcu_opcode_btsti(state, opcode, opcode_reg),
        _ => mcu_opcode_not_implemented(state, opcode, opcode_reg),
    }
}
