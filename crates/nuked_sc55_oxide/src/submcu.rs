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

use crate::Sc55State;

pub(crate) const SM_STATUS_C: u8 = 1;
pub(crate) const SM_STATUS_Z: u8 = 2;
pub(crate) const SM_STATUS_I: u8 = 4;
pub(crate) const SM_STATUS_D: u8 = 8;
pub(crate) const SM_STATUS_T: u8 = 32;
pub(crate) const SM_STATUS_N: u8 = 128;

pub(crate) const SM_VECTOR_UART3_TX: u32 = 0;
pub(crate) const SM_VECTOR_UART2_TX: u32 = 1;
pub(crate) const SM_VECTOR_UART1_TX: u32 = 2;
pub(crate) const SM_VECTOR_COLLISION: u32 = 3;
pub(crate) const SM_VECTOR_TIMER_X: u32 = 4;
pub(crate) const SM_VECTOR_IPCM0: u32 = 5;
pub(crate) const SM_VECTOR_UART3_RX: u32 = 6;
pub(crate) const SM_VECTOR_UART2_RX: u32 = 7;
pub(crate) const SM_VECTOR_UART1_RX: u32 = 8;
pub(crate) const SM_VECTOR_RESET: u32 = 9;

pub(crate) const SM_DEV_P1_DATA: usize = 0x00;
pub(crate) const SM_DEV_P1_DIR: usize = 0x01;
pub(crate) const SM_DEV_RAM_DIR: usize = 0x02;
pub(crate) const SM_DEV_UART1_MODE_STATUS: usize = 0x05;
pub(crate) const SM_DEV_UART1_CTRL: usize = 0x06;
pub(crate) const SM_DEV_UART2_DATA: usize = 0x08;
pub(crate) const SM_DEV_UART2_MODE_STATUS: usize = 0x09;
pub(crate) const SM_DEV_UART2_CTRL: usize = 0x0A;
pub(crate) const SM_DEV_UART3_MODE_STATUS: usize = 0x0D;
pub(crate) const SM_DEV_UART3_CTRL: usize = 0x0E;
pub(crate) const SM_DEV_IPCM0: usize = 0x10;
pub(crate) const SM_DEV_IPCM1: usize = 0x11;
pub(crate) const SM_DEV_IPCM2: usize = 0x12;
pub(crate) const SM_DEV_IPCM3: usize = 0x13;
pub(crate) const SM_DEV_IPCE0: usize = 0x14;
pub(crate) const SM_DEV_IPCE1: usize = 0x15;
pub(crate) const SM_DEV_IPCE2: usize = 0x16;
pub(crate) const SM_DEV_IPCE3: usize = 0x17;
pub(crate) const SM_DEV_SEMAPHORE: usize = 0x19;
pub(crate) const SM_DEV_COLLISION: usize = 0x1A;
pub(crate) const SM_DEV_INT_ENABLE: usize = 0x1B;
pub(crate) const SM_DEV_INT_REQUEST: usize = 0x1C;
pub(crate) const SM_DEV_PRESCALER: usize = 0x1D;
pub(crate) const SM_DEV_TIMER: usize = 0x1E;
pub(crate) const SM_DEV_TIMER_CTRL: usize = 0x1F;

#[derive(Default, Clone, Copy)]
/// Complete authoritative state of the SC-55 sub-MCU.
pub(crate) struct SubMcuState {
    pub pc: u16,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub s: u8,
    pub sr: u8,
    pub cycles: u64,
    pub sleep: u8,
}

crate::impl_state_codec!(SubMcuState {
    pc,
    a,
    x,
    y,
    s,
    sr,
    cycles,
    sleep,
});

fn sm_read(state: &mut Sc55State, address: u16) -> u8 {
    let address = address & 0x1FFF;
    if address & 0x1000 != 0 {
        state.sm_rom[(address & 0xFFF) as usize]
    } else if address < 0x80 {
        state.sm_ram[address as usize]
    } else if (0xC0..0xD8).contains(&address) {
        state.sm_access[(address & 0x1F) as usize]
    } else if (0xE0..0x100).contains(&address) {
        let address = (address & 0x1F) as usize;
        match address {
            SM_DEV_UART2_DATA => {
                state.sm_uart_rx_gotbyte = 0;
                return state.sm_uart_rx_byte;
            }
            SM_DEV_UART1_MODE_STATUS => {
                let mut ret: u8 = 0;
                ret |= 5;
                return ret;
            }
            SM_DEV_UART2_MODE_STATUS => {
                let mut ret: u8 = state.sm_uart_rx_gotbyte << 1;
                ret |= 5;
                return ret;
            }
            SM_DEV_UART3_MODE_STATUS => {
                let mut ret: u8 = 0;
                ret |= 5;
                return ret;
            }
            SM_DEV_P1_DATA => return 0xFF,
            SM_DEV_P1_DIR => return state.sm_p1_dir,
            SM_DEV_PRESCALER => return state.sm_timer_prescaler,
            SM_DEV_TIMER => return state.sm_timer_counter,
            _ => {}
        }
        state.sm_device_mode[address]
    } else if (0x200..0x2C0).contains(&address) {
        let address = (address & 0xFF) as usize;
        if state.sm_device_mode[SM_DEV_RAM_DIR] & (1 << (address >> 5)) != 0 {
            state.sm_access[address >> 3] &= !(1 << (address & 7));
        }
        state.sm_shared_ram[address]
    } else {
        0
    }
}

fn sm_write(state: &mut Sc55State, address: u16, data: u8) {
    let address = address & 0x1FFF;
    if address < 0x80 {
        state.sm_ram[address as usize] = data;
    } else if (0xE0..0x100).contains(&address) {
        let address = (address & 0x1F) as usize;
        match address {
            SM_DEV_P1_DATA => {}
            SM_DEV_P1_DIR => {
                state.sm_p1_dir = data;
            }
            SM_DEV_IPCM0 | SM_DEV_IPCM1 | SM_DEV_IPCM2 | SM_DEV_IPCM3 => {
                state.sm_device_mode[address] = data;
            }
            SM_DEV_IPCE0 | SM_DEV_IPCE1 | SM_DEV_IPCE2 | SM_DEV_IPCE3 => {
                state.sm_device_mode[address] = data;
            }
            SM_DEV_INT_REQUEST => {
                state.sm_device_mode[SM_DEV_INT_REQUEST] &= data;
            }
            SM_DEV_COLLISION => {
                state.sm_device_mode[SM_DEV_COLLISION] &= !0x7F;
                state.sm_device_mode[SM_DEV_COLLISION] |= data & 0x7F;
                if (data & 0x80) == 0 {
                    state.sm_device_mode[SM_DEV_COLLISION] &= !0x80;
                }
            }
            _ => {
                state.sm_device_mode[address] = data;
            }
        }
        if address == SM_DEV_UART3_MODE_STATUS || address == SM_DEV_UART3_CTRL {
            crate::mcu::mcu_ga_set_ga_int(
                state,
                5,
                ((state.sm_device_mode[SM_DEV_UART3_MODE_STATUS] & 0x80) != 0
                    && (state.sm_device_mode[SM_DEV_UART3_CTRL] & 0x20) == 0)
                    as i32,
            );
        }
    } else if (0x200..0x2C0).contains(&address) {
        let address = (address & 0xFF) as usize;
        state.sm_access[address >> 3] |= 1 << (address & 7);
        state.sm_shared_ram[address] = data;
    }
}

#[allow(clippy::if_same_then_else)]
pub(crate) fn sm_sys_write(state: &mut Sc55State, address: u32, data: u8) {
    let address = address & 0xFF;
    if address < 0xC0 {
        let address = (address & 0xFF) as usize;
        state.sm_access[address >> 3] |= 1 << (address & 7);
        state.sm_shared_ram[address] = data;
    } else if (0xF8..0xFC).contains(&address) {
        state.sm_device_mode[SM_DEV_IPCM0 + (address & 3) as usize] = data;
        if (address & 3) == 0 {
            state.sm_device_mode[SM_DEV_INT_REQUEST] |= 0x10;
            state.sm_device_mode[SM_DEV_SEMAPHORE] &= !0x80;
        }
    } else if address == 0xFF {
        state.sm_device_mode[SM_DEV_SEMAPHORE] &= !0x1F;
        state.sm_device_mode[SM_DEV_SEMAPHORE] |= data & 0x1F;
    } else if address == 0xF5 {
        /* We don't emulate the button logic */
    } else if address == 0xF6 {
        /* We don't emulate the button logic */
    } else if address == 0xF7 {
        state.sm_p0_dir = data;
    }
}

#[allow(clippy::if_same_then_else)]
pub(crate) fn sm_sys_read(state: &mut Sc55State, address: u32) -> u8 {
    let address = address & 0xFF;
    if address < 0xC0 {
        let address = address as usize;
        if (state.sm_device_mode[SM_DEV_RAM_DIR] & (1 << (address >> 5))) == 0 {
            state.sm_access[address >> 3] &= !(1 << (address & 7));
        }
        state.sm_shared_ram[address]
    } else if (0xF8..0xFC).contains(&address) {
        if (address & 3) == 0 {
            state.sm_device_mode[SM_DEV_INT_REQUEST] |= 0x10;
        }
        let val = state.sm_device_mode[SM_DEV_IPCE0 + (address & 3) as usize];
        state.sm_device_mode[SM_DEV_IPCE0 + (address & 3) as usize] = 0; // FIXME
        val
    } else if address == 0xFF {
        state.sm_device_mode[SM_DEV_SEMAPHORE]
    } else if address == 0xF5 {
        0xFF
    } else if address == 0xF6 {
        0xFF
    } else if address == 0xF7 {
        state.sm_p0_dir
    } else {
        0
    }
}

fn sm_get_vector_address(state: &mut Sc55State, vector: u32) -> u16 {
    let mut pc = sm_read(
        state,
        0x1FEC_u16.wrapping_add((vector as u16).wrapping_mul(2)),
    ) as u16;
    pc |= (sm_read(
        state,
        0x1FEC_u16
            .wrapping_add((vector as u16).wrapping_mul(2))
            .wrapping_add(1),
    ) as u16)
        << 8;
    pc
}

fn sm_set_status(state: &mut Sc55State, condition: u32, mask: u8) {
    if condition != 0 {
        state.sm.sr |= mask;
    } else {
        state.sm.sr &= !mask;
    }
}

pub(crate) fn sm_reset(state: &mut Sc55State) {
    state.sm = SubMcuState::default();
    state.sm.pc = sm_get_vector_address(state, SM_VECTOR_RESET);
}

fn sm_read_advance(state: &mut Sc55State) -> u8 {
    let byte = sm_read(state, state.sm.pc);
    state.sm.pc = state.sm.pc.wrapping_add(1);
    byte
}

fn sm_read_advance16(state: &mut Sc55State) -> u16 {
    let mut word = sm_read_advance(state) as u16;
    word |= (sm_read_advance(state) as u16) << 8;
    word
}

fn sm_read16(state: &mut Sc55State, address: u16) -> u16 {
    let mut word = sm_read(state, address) as u16;
    // BUG FIX: The original C++ reads `address` twice instead of `address` and `address + 1`.
    // This is likely a bug because SM_GetVectorAddress, which does the same thing, correctly
    // reads from two consecutive addresses.
    // word |= (sm_read(state, address) as u16) << 8;
    word |= (sm_read(state, address.wrapping_add(1)) as u16) << 8;
    word
}

fn sm_update_nz(state: &mut Sc55State, val: u8) {
    sm_set_status(state, (val == 0) as u32, SM_STATUS_Z);
    sm_set_status(state, (val & 0x80) as u32, SM_STATUS_N);
}

fn sm_push_stack(state: &mut Sc55State, data: u8) {
    let s = state.sm.s;
    sm_write(state, s as u16, data);
    state.sm.s = state.sm.s.wrapping_sub(1);
}

fn sm_pop_stack(state: &mut Sc55State) -> u8 {
    state.sm.s = state.sm.s.wrapping_add(1);
    let s = state.sm.s;
    sm_read(state, s as u16)
}

fn sm_opcode_sei(state: &mut Sc55State, _opcode: u8) {
    // 78
    sm_set_status(state, 1, SM_STATUS_I);
}

fn sm_opcode_cld(state: &mut Sc55State, _opcode: u8) {
    // d8
    sm_set_status(state, 0, SM_STATUS_D);
}

fn sm_opcode_clt(state: &mut Sc55State, _opcode: u8) {
    // 12
    sm_set_status(state, 0, SM_STATUS_T);
}

fn sm_opcode_ldx(state: &mut Sc55State, opcode: u8) {
    // a2, a6, ae, b6, be
    let mut val: u8 = 0;
    match opcode {
        0xA2 => {
            val = sm_read_advance(state);
        }
        0xA6 => {
            let addr = sm_read_advance(state);
            val = sm_read(state, addr as u16);
        }
        0xB6 => {
            let addr = sm_read_advance(state);
            val = sm_read(state, addr.wrapping_add(state.sm.y) as u16 & 0xFF);
        }
        0xAE => {
            let addr = sm_read_advance16(state);
            val = sm_read(state, addr);
        }
        0xBE => {
            let addr = sm_read_advance16(state);
            val = sm_read(state, addr.wrapping_add(state.sm.y as u16));
        }
        _ => {}
    }
    state.sm.x = val;
    sm_update_nz(state, state.sm.x);
}

fn sm_opcode_ldy(state: &mut Sc55State, opcode: u8) {
    // a0, a4, ac, b4, bc
    let mut val: u8 = 0;
    match opcode {
        0xA0 => {
            val = sm_read_advance(state);
        }
        0xA4 => {
            let addr = sm_read_advance(state);
            val = sm_read(state, addr as u16);
        }
        0xAC => {
            let addr = sm_read_advance16(state);
            val = sm_read(state, addr);
        }
        0xB4 => {
            let addr = sm_read_advance(state);
            val = sm_read(state, addr.wrapping_add(state.sm.x) as u16 & 0xFF);
        }
        0xBC => {
            let addr = sm_read_advance16(state);
            val = sm_read(state, addr.wrapping_add(state.sm.x as u16));
        }
        _ => {}
    }
    state.sm.y = val;
    sm_update_nz(state, state.sm.y);
}

fn sm_opcode_txs(state: &mut Sc55State, _opcode: u8) {
    // 9a
    state.sm.s = state.sm.x;
}

fn sm_opcode_txa(state: &mut Sc55State, _opcode: u8) {
    // 8a
    state.sm.a = state.sm.x;
    sm_update_nz(state, state.sm.a);
}

fn sm_opcode_sta(state: &mut Sc55State, opcode: u8) {
    // 85, 95, 8d, 9d, 99, 81, 91
    let mut dest: u16 = 0;
    match opcode {
        0x85 => {
            dest = sm_read_advance(state) as u16;
        }
        0x95 => {
            let addr = sm_read_advance(state) as u16;
            dest = addr.wrapping_add(state.sm.x as u16);
        }
        0x8D => {
            dest = sm_read_advance16(state);
        }
        0x9D => {
            let addr = sm_read_advance16(state);
            dest = addr.wrapping_add(state.sm.x as u16);
        }
        0x99 => {
            let addr = sm_read_advance16(state);
            dest = addr.wrapping_add(state.sm.y as u16);
        }
        0x81 => {
            let addr = sm_read_advance(state);
            dest = sm_read16(state, addr.wrapping_add(state.sm.x) as u16 & 0xFF);
        }
        0x91 => {
            let addr = sm_read_advance(state);
            dest = sm_read16(state, addr as u16).wrapping_add(state.sm.y as u16);
        }
        _ => {}
    }

    let a = state.sm.a;
    sm_write(state, dest, a);
}

fn sm_opcode_inx(state: &mut Sc55State, _opcode: u8) {
    // e8
    state.sm.x = state.sm.x.wrapping_add(1);
    sm_update_nz(state, state.sm.x);
}

fn sm_opcode_iny(state: &mut Sc55State, _opcode: u8) {
    // c8
    state.sm.y = state.sm.y.wrapping_add(1);
    sm_update_nz(state, state.sm.y);
}

fn sm_opcode_bbc_bbs(state: &mut Sc55State, opcode: u8) {
    let zp: i32 = (opcode & 4 != 0) as i32;
    let bit: i32 = ((opcode >> 5) & 7) as i32;
    let type_: i32 = ((opcode >> 4) & 1) as i32;
    let val = if zp == 0 {
        state.sm.a
    } else {
        let addr = sm_read_advance(state);
        sm_read(state, addr as u16)
    };

    let diff = sm_read_advance(state) as i8;

    let set = ((val >> bit) & 1) as i32;

    if set != type_ {
        state.sm.pc = state.sm.pc.wrapping_add(diff as u16);
    }
}

fn sm_opcode_cpx(state: &mut Sc55State, opcode: u8) {
    // e0, e4, ec
    let mut operand: u8 = 0;
    match opcode {
        0xE0 => {
            operand = sm_read_advance(state);
        }
        0xE4 => {
            let addr = sm_read_advance(state);
            operand = sm_read(state, addr as u16);
        }
        0xEC => {
            let addr = sm_read_advance16(state);
            operand = sm_read(state, addr);
        }
        _ => {}
    }
    let diff = (state.sm.x as i32).wrapping_sub(operand as i32);
    sm_set_status(state, ((diff & 0x100) == 0) as u32, SM_STATUS_C);
    sm_update_nz(state, (diff & 0xFF) as u8);
}

fn sm_opcode_cpy(state: &mut Sc55State, opcode: u8) {
    // c0, c4, cc
    let mut operand: u8 = 0;
    match opcode {
        0xC0 => {
            operand = sm_read_advance(state);
        }
        0xC4 => {
            let addr = sm_read_advance(state);
            operand = sm_read(state, addr as u16);
        }
        0xCC => {
            let addr = sm_read_advance16(state);
            operand = sm_read(state, addr);
        }
        _ => {}
    }
    let diff = (state.sm.y as i32).wrapping_sub(operand as i32);
    sm_set_status(state, ((diff & 0x100) == 0) as u32, SM_STATUS_C);
    sm_update_nz(state, (diff & 0xFF) as u8);
}

fn sm_opcode_beq(state: &mut Sc55State, _opcode: u8) {
    // f0
    let diff = sm_read_advance(state) as i8;
    if (state.sm.sr & SM_STATUS_Z) != 0 {
        state.sm.pc = state.sm.pc.wrapping_add(diff as u16);
    }
}

fn sm_opcode_bcc(state: &mut Sc55State, _opcode: u8) {
    // 90
    let diff = sm_read_advance(state) as i8;
    if (state.sm.sr & SM_STATUS_C) == 0 {
        state.sm.pc = state.sm.pc.wrapping_add(diff as u16);
    }
}

fn sm_opcode_bcs(state: &mut Sc55State, _opcode: u8) {
    // b0
    let diff = sm_read_advance(state) as i8;
    if (state.sm.sr & SM_STATUS_C) != 0 {
        state.sm.pc = state.sm.pc.wrapping_add(diff as u16);
    }
}

fn sm_opcode_ldm(state: &mut Sc55State, _opcode: u8) {
    // 3c
    let val = sm_read_advance(state);
    let addr = sm_read_advance(state);
    sm_write(state, addr as u16, val);
}

fn sm_opcode_lda(state: &mut Sc55State, opcode: u8) {
    // a9, a5, b5, ad, bd, b9, a1, b1
    let mut val: u8 = 0;
    match opcode {
        0xA9 => {
            val = sm_read_advance(state);
        }
        0xA5 => {
            let addr = sm_read_advance(state);
            val = sm_read(state, addr as u16);
        }
        0xB5 => {
            let addr = sm_read_advance(state);
            val = sm_read(state, addr.wrapping_add(state.sm.x) as u16 & 0xFF);
        }
        0xAD => {
            let addr = sm_read_advance16(state);
            val = sm_read(state, addr);
        }
        0xBD => {
            let addr = sm_read_advance16(state);
            val = sm_read(state, addr.wrapping_add(state.sm.x as u16));
        }
        0xB9 => {
            let addr = sm_read_advance16(state);
            val = sm_read(state, addr.wrapping_add(state.sm.y as u16));
        }
        0xA1 => {
            let addr = sm_read_advance(state);
            let indirect = sm_read16(state, addr.wrapping_add(state.sm.x) as u16 & 0xFF);
            val = sm_read(state, indirect);
        }
        0xB1 => {
            let addr = sm_read_advance(state);
            let indirect = sm_read16(state, addr as u16);
            val = sm_read(state, indirect.wrapping_add(state.sm.y as u16));
        }
        _ => {}
    }

    if (state.sm.sr & SM_STATUS_T) == 0 {
        state.sm.a = val;
        sm_update_nz(state, val);
    } else {
        // FIXME
        let x = state.sm.x;
        sm_write(state, x as u16, val);
    }
}

fn sm_opcode_cli(state: &mut Sc55State, _opcode: u8) {
    // 58
    sm_set_status(state, 0, SM_STATUS_I);
}

fn sm_opcode_stp(state: &mut Sc55State, _opcode: u8) {
    // 42
    state.sm.sleep = 1;
}

fn sm_opcode_pha(state: &mut Sc55State, _opcode: u8) {
    // 48
    let a = state.sm.a;
    sm_push_stack(state, a);
}

fn sm_opcode_seb_clb(state: &mut Sc55State, opcode: u8) {
    let zp: i32 = (opcode & 4 != 0) as i32;
    let bit: i32 = ((opcode >> 5) & 7) as i32;
    let type_: i32 = ((opcode >> 4) & 1) as i32;
    let mut val: u8;
    let mut dest: u8 = 0;

    if zp == 0 {
        val = state.sm.a;
    } else {
        dest = sm_read_advance(state);
        val = sm_read(state, dest as u16);
    }

    if type_ != 0 {
        val &= !(1 << bit);
    } else {
        val |= 1 << bit;
    }

    if zp == 0 {
        state.sm.a = val;
    } else {
        sm_write(state, dest as u16, val);
    }
}

fn sm_opcode_rti(state: &mut Sc55State, _opcode: u8) {
    // 40
    state.sm.sr = sm_pop_stack(state);
    state.sm.pc = sm_pop_stack(state) as u16;
    state.sm.pc |= (sm_pop_stack(state) as u16) << 8;
}

fn sm_opcode_pla(state: &mut Sc55State, _opcode: u8) {
    // 68
    state.sm.a = sm_pop_stack(state);
    sm_update_nz(state, state.sm.a);
}

fn sm_opcode_bra(state: &mut Sc55State, _opcode: u8) {
    // 80
    let disp = sm_read_advance(state) as i8;
    state.sm.pc = state.sm.pc.wrapping_add(disp as u16);
}

fn sm_opcode_jsr(state: &mut Sc55State, opcode: u8) {
    // 20, 02, 22
    let mut newpc: u16 = 0;
    match opcode {
        0x20 => {
            newpc = sm_read_advance16(state);
        }
        0x02 => {
            let addr = sm_read_advance(state);
            newpc = sm_read16(state, addr as u16);
        }
        0x22 => {
            let addr = sm_read_advance(state);
            newpc = 0xFF00 | addr as u16;
        }
        _ => {}
    }

    let pc_hi = (state.sm.pc >> 8) as u8;
    sm_push_stack(state, pc_hi);
    let pc_lo = (state.sm.pc & 0xFF) as u8;
    sm_push_stack(state, pc_lo);
    state.sm.pc = newpc;
}

fn sm_opcode_cmp(state: &mut Sc55State, opcode: u8) {
    // c9, c5, d5, cd, dd, d9, c1, d1
    let mut operand: u8 = 0;
    match opcode {
        0xC9 => {
            operand = sm_read_advance(state);
        }
        0xC5 => {
            let addr = sm_read_advance(state);
            operand = sm_read(state, addr as u16);
        }
        0xD5 => {
            let addr = sm_read_advance(state);
            operand = sm_read(state, addr.wrapping_add(state.sm.x) as u16 & 0xFF);
        }
        0xCD => {
            let addr = sm_read_advance16(state);
            operand = sm_read(state, addr);
        }
        0xDD => {
            let addr = sm_read_advance16(state);
            operand = sm_read(state, addr.wrapping_add(state.sm.x as u16));
        }
        0xD9 => {
            let addr = sm_read_advance16(state);
            operand = sm_read(state, addr.wrapping_add(state.sm.y as u16));
        }
        0xC1 => {
            let addr = sm_read_advance(state);
            let indirect = sm_read16(state, addr.wrapping_add(state.sm.x) as u16 & 0xFF);
            operand = sm_read(state, indirect);
        }
        0xD1 => {
            let addr = sm_read_advance(state);
            let indirect = sm_read16(state, addr as u16);
            operand = sm_read(state, indirect.wrapping_add(state.sm.y as u16));
        }
        _ => {}
    }
    let diff = (state.sm.a as i32).wrapping_sub(operand as i32);
    sm_set_status(state, ((diff & 0x100) == 0) as u32, SM_STATUS_C);
    sm_update_nz(state, (diff & 0xFF) as u8);
}

fn sm_opcode_bne(state: &mut Sc55State, _opcode: u8) {
    // d0
    let diff = sm_read_advance(state) as i8;
    if (state.sm.sr & SM_STATUS_Z) == 0 {
        state.sm.pc = state.sm.pc.wrapping_add(diff as u16);
    }
}

fn sm_opcode_rts(state: &mut Sc55State, _opcode: u8) {
    // 60
    state.sm.pc = sm_pop_stack(state) as u16;
    state.sm.pc |= (sm_pop_stack(state) as u16) << 8;
}

fn sm_opcode_jmp(state: &mut Sc55State, opcode: u8) {
    // 4c, 6c, b2
    match opcode {
        0x4C => {
            state.sm.pc = sm_read_advance16(state);
        }
        0x6C => {
            let addr = sm_read_advance16(state);
            state.sm.pc = sm_read16(state, addr);
        }
        0xB2 => {
            let addr = sm_read_advance(state);
            state.sm.pc = sm_read16(state, addr as u16);
        }
        _ => {}
    }
}

fn sm_opcode_ora(state: &mut Sc55State, opcode: u8) {
    // 09, 05, 15, 0d, 1d, 01, 11
    let mut val: u8;
    let mut val2: u8 = 0;

    if (state.sm.sr & SM_STATUS_T) == 0 {
        val = state.sm.a;
    } else {
        // FIXME
        let x = state.sm.x;
        val = sm_read(state, x as u16);
    }

    match opcode {
        0x09 => {
            val2 = sm_read_advance(state);
        }
        0x05 => {
            let addr = sm_read_advance(state);
            val2 = sm_read(state, addr as u16);
        }
        0x15 => {
            let addr = sm_read_advance(state);
            val2 = sm_read(state, addr.wrapping_add(state.sm.x) as u16 & 0xFF);
        }
        0x0D => {
            let addr = sm_read_advance16(state);
            val2 = sm_read(state, addr);
        }
        0x1D => {
            let addr = sm_read_advance16(state);
            val2 = sm_read(state, addr.wrapping_add(state.sm.x as u16));
        }
        0x19 => {
            let addr = sm_read_advance16(state);
            val2 = sm_read(state, addr.wrapping_add(state.sm.y as u16));
        }
        0x01 => {
            let addr = sm_read_advance(state);
            let indirect = sm_read16(state, addr.wrapping_add(state.sm.x) as u16 & 0xFF);
            val2 = sm_read(state, indirect);
        }
        0x11 => {
            let addr = sm_read_advance(state);
            let indirect = sm_read16(state, addr as u16);
            val2 = sm_read(state, indirect.wrapping_add(state.sm.y as u16));
        }
        _ => {}
    }

    val |= val2;

    if (state.sm.sr & SM_STATUS_T) == 0 {
        state.sm.a = val;

        sm_update_nz(state, val);
    } else {
        // FIXME
        let x = state.sm.x;
        sm_write(state, x as u16, val);
    }
}

fn sm_opcode_dec(state: &mut Sc55State, opcode: u8) {
    // 1a, c6, d6, ce, de
    let mut dest: u16 = 0;
    match opcode {
        0x1A => {
            state.sm.a = state.sm.a.wrapping_sub(1);
            sm_update_nz(state, state.sm.a);
            return;
        }
        0xC6 => {
            dest = sm_read_advance(state) as u16;
        }
        0xD6 => {
            let addr = sm_read_advance(state);
            dest = addr.wrapping_add(state.sm.x) as u16 & 0xFF;
        }
        0xCE => {
            dest = sm_read_advance16(state);
        }
        0xDE => {
            let addr = sm_read_advance16(state);
            dest = addr.wrapping_add(state.sm.x as u16);
        }
        _ => {}
    }
    let mut val = sm_read(state, dest);
    val = val.wrapping_sub(1);
    sm_write(state, dest, val);
    sm_update_nz(state, val);
}

fn sm_opcode_tax(state: &mut Sc55State, _opcode: u8) {
    // aa
    state.sm.x = state.sm.a;
    sm_update_nz(state, state.sm.x);
}

fn sm_opcode_stx(state: &mut Sc55State, opcode: u8) {
    // 86 96 8e
    let mut dest: u16 = 0;
    match opcode {
        0x86 => {
            dest = sm_read_advance(state) as u16;
        }
        0x96 => {
            let addr = sm_read_advance(state) as u16;
            dest = addr.wrapping_add(state.sm.x as u16);
        }
        0x8E => {
            dest = sm_read_advance16(state);
        }
        _ => {}
    }

    let x = state.sm.x;
    sm_write(state, dest, x);
}

fn sm_opcode_sty(state: &mut Sc55State, opcode: u8) {
    // 84 8c 94
    let mut dest: u16 = 0;
    match opcode {
        0x84 => {
            dest = sm_read_advance(state) as u16;
        }
        0x94 => {
            let addr = sm_read_advance(state);
            dest = addr.wrapping_add(state.sm.x) as u16 & 0xFF;
        }
        0x8C => {
            dest = sm_read_advance16(state);
        }
        _ => {}
    }

    let y = state.sm.y;
    sm_write(state, dest, y);
}

fn sm_opcode_sec(state: &mut Sc55State, _opcode: u8) {
    // 38
    sm_set_status(state, 1, SM_STATUS_C);
}

fn sm_opcode_nop(_state: &mut Sc55State, _opcode: u8) {
    // EA
}

fn sm_opcode_bpl(state: &mut Sc55State, _opcode: u8) {
    // 10
    let diff = sm_read_advance(state) as i8;
    if (state.sm.sr & SM_STATUS_N) == 0 {
        state.sm.pc = state.sm.pc.wrapping_add(diff as u16);
    }
}

fn sm_opcode_clc(state: &mut Sc55State, _opcode: u8) {
    // 18
    sm_set_status(state, 0, SM_STATUS_C);
}

fn sm_opcode_and(state: &mut Sc55State, opcode: u8) {
    // 29, 25, 35, 2d, 3d, 21, 31
    let mut val: u8;
    let mut val2: u8 = 0;

    if (state.sm.sr & SM_STATUS_T) == 0 {
        val = state.sm.a;
    } else {
        // FIXME
        let x = state.sm.x;
        val = sm_read(state, x as u16);
    }

    match opcode {
        0x29 => {
            val2 = sm_read_advance(state);
        }
        0x25 => {
            let addr = sm_read_advance(state);
            val2 = sm_read(state, addr as u16);
        }
        0x35 => {
            let addr = sm_read_advance(state);
            val2 = sm_read(state, addr.wrapping_add(state.sm.x) as u16 & 0xFF);
        }
        0x2D => {
            let addr = sm_read_advance16(state);
            val2 = sm_read(state, addr);
        }
        0x3D => {
            let addr = sm_read_advance16(state);
            val2 = sm_read(state, addr.wrapping_add(state.sm.x as u16));
        }
        0x39 => {
            let addr = sm_read_advance16(state);
            val2 = sm_read(state, addr.wrapping_add(state.sm.y as u16));
        }
        0x21 => {
            let addr = sm_read_advance(state);
            let indirect = sm_read16(state, addr.wrapping_add(state.sm.x) as u16 & 0xFF);
            val2 = sm_read(state, indirect);
        }
        0x31 => {
            let addr = sm_read_advance(state);
            let indirect = sm_read16(state, addr as u16);
            val2 = sm_read(state, indirect.wrapping_add(state.sm.y as u16));
        }
        _ => {}
    }

    val &= val2;

    if (state.sm.sr & SM_STATUS_T) == 0 {
        state.sm.a = val;

        sm_update_nz(state, val);
    } else {
        // FIXME
        let x = state.sm.x;
        sm_write(state, x as u16, val);
    }
}

fn sm_opcode_inc(state: &mut Sc55State, opcode: u8) {
    // 3a, e6, f6, ee, fe
    let mut dest: u16 = 0;
    match opcode {
        0x3A => {
            state.sm.a = state.sm.a.wrapping_add(1);
            sm_update_nz(state, state.sm.a);
            return;
        }
        0xE6 => {
            dest = sm_read_advance(state) as u16;
        }
        0xF6 => {
            let addr = sm_read_advance(state);
            dest = addr.wrapping_add(state.sm.x) as u16 & 0xFF;
        }
        0xEE => {
            dest = sm_read_advance16(state);
        }
        0xFE => {
            let addr = sm_read_advance16(state);
            dest = addr.wrapping_add(state.sm.x as u16);
        }
        _ => {}
    }
    let mut val = sm_read(state, dest);
    val = val.wrapping_add(1);
    sm_write(state, dest, val);
    sm_update_nz(state, val);
}

fn sm_opcode_dispatch(state: &mut Sc55State, opcode: u8) {
    match opcode {
        0x01 | 0x05 | 0x09 | 0x0D | 0x11 | 0x15 | 0x19 | 0x1D => sm_opcode_ora(state, opcode),
        0x02 | 0x20 | 0x22 => sm_opcode_jsr(state, opcode),
        0x03 | 0x07 | 0x13 | 0x17 | 0x23 | 0x27 | 0x33 | 0x37 | 0x43 | 0x47 | 0x53 | 0x57
        | 0x63 | 0x67 | 0x73 | 0x77 | 0x83 | 0x87 | 0x93 | 0x97 | 0xA3 | 0xA7 | 0xB3 | 0xB7
        | 0xC3 | 0xC7 | 0xD3 | 0xD7 | 0xE3 | 0xE7 | 0xF3 | 0xF7 => sm_opcode_bbc_bbs(state, opcode),
        0x0B | 0x0F | 0x1B | 0x1F | 0x2B | 0x2F | 0x3B | 0x3F | 0x4B | 0x4F | 0x5B | 0x5F
        | 0x6B | 0x6F | 0x7B | 0x7F | 0x8B | 0x8F | 0x9B | 0x9F | 0xAB | 0xAF | 0xBB | 0xBF
        | 0xCB | 0xCF | 0xDB | 0xDF | 0xEB | 0xEF | 0xFB | 0xFF => sm_opcode_seb_clb(state, opcode),
        0x10 => sm_opcode_bpl(state, opcode),
        0x12 => sm_opcode_clt(state, opcode),
        0x18 => sm_opcode_clc(state, opcode),
        0x1A | 0xC6 | 0xCE | 0xD6 | 0xDE => sm_opcode_dec(state, opcode),
        0x21 | 0x25 | 0x29 | 0x2D | 0x31 | 0x35 | 0x39 | 0x3D => sm_opcode_and(state, opcode),
        0x38 => sm_opcode_sec(state, opcode),
        0x3A | 0xE6 | 0xEE | 0xF6 | 0xFE => sm_opcode_inc(state, opcode),
        0x3C => sm_opcode_ldm(state, opcode),
        0x40 => sm_opcode_rti(state, opcode),
        0x42 => sm_opcode_stp(state, opcode),
        0x48 => sm_opcode_pha(state, opcode),
        0x4C | 0x6C | 0xB2 => sm_opcode_jmp(state, opcode),
        0x58 => sm_opcode_cli(state, opcode),
        0x60 => sm_opcode_rts(state, opcode),
        0x68 => sm_opcode_pla(state, opcode),
        0x78 => sm_opcode_sei(state, opcode),
        0x80 => sm_opcode_bra(state, opcode),
        0x81 | 0x85 | 0x8D | 0x91 | 0x95 | 0x99 | 0x9D => sm_opcode_sta(state, opcode),
        0x84 | 0x8C | 0x94 => sm_opcode_sty(state, opcode),
        0x86 | 0x8E | 0x96 => sm_opcode_stx(state, opcode),
        0x8A => sm_opcode_txa(state, opcode),
        0x90 => sm_opcode_bcc(state, opcode),
        0x9A => sm_opcode_txs(state, opcode),
        0xA0 | 0xA4 | 0xAC | 0xB4 | 0xBC => sm_opcode_ldy(state, opcode),
        0xA1 | 0xA5 | 0xA9 | 0xAD | 0xB1 | 0xB5 | 0xB9 | 0xBD => sm_opcode_lda(state, opcode),
        0xA2 | 0xA6 | 0xAE | 0xB6 | 0xBE => sm_opcode_ldx(state, opcode),
        0xAA => sm_opcode_tax(state, opcode),
        0xB0 => sm_opcode_bcs(state, opcode),
        0xC0 | 0xC4 | 0xCC => sm_opcode_cpy(state, opcode),
        0xC1 | 0xC5 | 0xC9 | 0xCD | 0xD1 | 0xD5 | 0xD9 | 0xDD => sm_opcode_cmp(state, opcode),
        0xC8 => sm_opcode_iny(state, opcode),
        0xD0 => sm_opcode_bne(state, opcode),
        0xD8 => sm_opcode_cld(state, opcode),
        0xE0 | 0xE4 | 0xEC => sm_opcode_cpx(state, opcode),
        0xE8 => sm_opcode_inx(state, opcode),
        0xEA => sm_opcode_nop(state, opcode),
        0xF0 => sm_opcode_beq(state, opcode),
        _ => {}
    }
}

fn sm_start_vector(state: &mut Sc55State, vector: u32) {
    let pc_hi = (state.sm.pc >> 8) as u8;
    sm_push_stack(state, pc_hi);
    let pc_lo = (state.sm.pc & 0xFF) as u8;
    sm_push_stack(state, pc_lo);
    let sr = state.sm.sr;
    sm_push_stack(state, sr);

    state.sm.sr |= SM_STATUS_I;
    state.sm.sleep = 0;

    state.sm.pc = sm_get_vector_address(state, vector);
}

fn sm_handle_interrupt(state: &mut Sc55State) {
    if state.sm.sr & SM_STATUS_I != 0 {
        return;
    }

    if (state.sm_device_mode[SM_DEV_UART1_CTRL] & 0x8) != 0
        && (state.sm_device_mode[SM_DEV_INT_ENABLE] & 0x80) != 0
        && (state.sm_device_mode[SM_DEV_INT_REQUEST] & 0x80) != 0
    {
        state.sm_device_mode[SM_DEV_INT_REQUEST] &= !0x80;
        sm_start_vector(state, SM_VECTOR_UART1_RX);
        return;
    }
    if (state.sm_device_mode[SM_DEV_UART2_CTRL] & 0x8) != 0
        && (state.sm_device_mode[SM_DEV_INT_ENABLE] & 0x40) != 0
        && (state.sm_device_mode[SM_DEV_INT_REQUEST] & 0x40) != 0
    {
        state.sm_device_mode[SM_DEV_INT_REQUEST] &= !0x40;
        sm_start_vector(state, SM_VECTOR_UART2_RX);
        return;
    }
    if (state.sm_device_mode[SM_DEV_UART3_CTRL] & 0x8) != 0
        && (state.sm_device_mode[SM_DEV_INT_ENABLE] & 0x20) != 0
        && (state.sm_device_mode[SM_DEV_INT_REQUEST] & 0x20) != 0
    {
        state.sm_device_mode[SM_DEV_INT_REQUEST] &= !0x20;
        sm_start_vector(state, SM_VECTOR_UART3_RX);
        return;
    }
    if (state.sm_device_mode[SM_DEV_TIMER_CTRL] & 0x80) != 0
        && (state.sm_device_mode[SM_DEV_INT_ENABLE] & 0x10) != 0
        && (state.sm_device_mode[SM_DEV_INT_REQUEST] & 0x10) != 0
    {
        state.sm_device_mode[SM_DEV_INT_REQUEST] &= !0x10;
        sm_start_vector(state, SM_VECTOR_IPCM0);
        return;
    }
    if (state.sm_device_mode[SM_DEV_TIMER_CTRL] & 0x40) != 0
        && (state.sm_device_mode[SM_DEV_INT_ENABLE] & 0x8) != 0
        && (state.sm_device_mode[SM_DEV_INT_REQUEST] & 0x8) != 0
    {
        state.sm_device_mode[SM_DEV_INT_REQUEST] &= !0x8;
        sm_start_vector(state, SM_VECTOR_TIMER_X);
        return;
    }
    if (state.sm_device_mode[SM_DEV_COLLISION] & 0xC0) == 0xC0 {
        state.sm_device_mode[SM_DEV_COLLISION] &= !0x80;
        sm_start_vector(state, SM_VECTOR_COLLISION);
        return;
    }
    if ((state.sm_device_mode[SM_DEV_UART1_CTRL] & 0x10) == 0 || (state.sm_cts & 1) != 0)
        && (state.sm_device_mode[SM_DEV_INT_ENABLE] & 0x4) != 0
        && (state.sm_device_mode[SM_DEV_INT_REQUEST] & 0x4) != 0
    {
        state.sm_device_mode[SM_DEV_INT_REQUEST] &= !0x4;
        sm_start_vector(state, SM_VECTOR_UART1_TX);
        return;
    }
    if ((state.sm_device_mode[SM_DEV_UART2_CTRL] & 0x10) == 0 || (state.sm_cts & 2) != 0)
        && (state.sm_device_mode[SM_DEV_INT_ENABLE] & 0x2) != 0
        && (state.sm_device_mode[SM_DEV_INT_REQUEST] & 0x2) != 0
    {
        state.sm_device_mode[SM_DEV_INT_REQUEST] &= !0x2;
        sm_start_vector(state, SM_VECTOR_UART2_TX);
        return;
    }
    if ((state.sm_device_mode[SM_DEV_UART3_CTRL] & 0x10) == 0 || (state.sm_cts & 4) != 0)
        && (state.sm_device_mode[SM_DEV_INT_ENABLE] & 0x1) != 0
        && (state.sm_device_mode[SM_DEV_INT_REQUEST] & 0x1) != 0
    {
        state.sm_device_mode[SM_DEV_INT_REQUEST] &= !0x1;
        sm_start_vector(state, SM_VECTOR_UART3_TX);
    }
}

fn sm_update_timer(state: &mut Sc55State) {
    while state.sm_timer_cycles < state.sm.cycles {
        if (state.sm_device_mode[SM_DEV_TIMER_CTRL] & 0x20) == 0 && state.sm.sleep == 0 {
            if state.sm_timer_prescaler == 0 {
                state.sm_timer_prescaler = state.sm_device_mode[SM_DEV_PRESCALER];

                if state.sm_timer_counter == 0 {
                    state.sm_timer_counter = state.sm_device_mode[SM_DEV_TIMER];
                    state.sm_device_mode[SM_DEV_INT_REQUEST] |= 0x8;
                } else {
                    state.sm_timer_counter = state.sm_timer_counter.wrapping_sub(1);
                }
            } else {
                state.sm_timer_prescaler = state.sm_timer_prescaler.wrapping_sub(1);
            }
        }
        state.sm_timer_cycles = state.sm_timer_cycles.wrapping_add(16);
    }
}

fn sm_update_uart(state: &mut Sc55State) {
    if (state.sm_device_mode[SM_DEV_UART1_CTRL] & 4) == 0 {
        // RX disabled
        return;
    }
    if state.uart_write_ptr == state.uart_read_ptr {
        // no byte
        return;
    }

    if state.sm_uart_rx_gotbyte != 0 {
        return;
    }

    if state.sm.cycles < state.sm_uart_rx_delay {
        return;
    }

    let read_ptr = state.uart_read_ptr as usize;
    state.sm_uart_rx_byte = state.uart_buffer[read_ptr];
    state.uart_read_ptr = (state.uart_read_ptr + 1) % state.uart_buffer.len() as u32;
    state.sm_uart_rx_gotbyte = 1;
    state.sm_device_mode[SM_DEV_INT_REQUEST] |= 0x40;

    state.sm_uart_rx_delay = state.sm.cycles + 3000 * 4;
}

pub(crate) fn sm_update(state: &mut Sc55State, cycles: u64) {
    while state.sm.cycles < cycles.wrapping_mul(5) {
        sm_handle_interrupt(state);

        if state.sm.sleep == 0 {
            let opcode = sm_read_advance(state);

            sm_opcode_dispatch(state, opcode);
        }

        state.sm.cycles = state.sm.cycles.wrapping_add(12 * 4); // FIXME

        sm_update_timer(state);
        sm_update_uart(state);
    }
}
