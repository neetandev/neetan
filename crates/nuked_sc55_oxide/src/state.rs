/*
 * Copyright (C) 2021, 2024 nukeykt
 *
 * This file is part of Nuked-SC55.
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU General Public License
 * as published by the Free Software Foundation; either version 2
 * of the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 */

use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

use crate::{
    mcu::McuState,
    mcu_timer::{FrtState, McuTimerState},
    pcm::PcmState,
    submcu::SubMcuState,
};

pub const UART_BUFFER_SIZE: usize = 8192;
pub const ROM1_SIZE: usize = 0x8000;
pub const ROM2_SIZE: usize = 0x80000;
pub const RAM_SIZE: usize = 0x400;
pub const SRAM_SIZE: usize = 0x8000;
pub const NVRAM_SIZE: usize = 0x8000;
pub const CARDRAM_SIZE: usize = 0x8000;

#[derive(Clone)]
pub(crate) struct ImmutableResource<Resource: Clone>(Arc<Resource>);

impl<Resource: Clone> ImmutableResource<Resource> {
    pub(crate) fn new(resource: Resource) -> Self {
        Self(Arc::new(resource))
    }
}

impl<Resource: Clone> Deref for ImmutableResource<Resource> {
    type Target = Resource;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<Resource: Clone> DerefMut for ImmutableResource<Resource> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.0)
    }
}

#[derive(Clone)]
/// Complete SC-55 synthesis state without retained ROM allocations.
pub struct Sc55State {
    pub mcu: McuState,
    pub dev_register: [u8; 0x80],
    pub romset: i32,
    pub mcu_mk1: bool,
    pub mcu_cm300: bool,
    pub mcu_st: bool,
    pub mcu_jv880: bool,
    pub mcu_scb55: bool,
    pub mcu_sc155: bool,
    pub uart_write_ptr: u32,
    pub uart_read_ptr: u32,
    pub uart_buffer: Box<[u8; UART_BUFFER_SIZE]>,

    pub(crate) rom1: ImmutableResource<[u8; ROM1_SIZE]>,
    pub(crate) rom2: ImmutableResource<Vec<u8>>,
    pub rom2_mask: i32,
    pub ram: [u8; RAM_SIZE],
    pub sram: Box<[u8; SRAM_SIZE]>,
    pub nvram: Box<[u8; NVRAM_SIZE]>,
    pub cardram: Box<[u8; CARDRAM_SIZE]>,

    pub ga_int: [i32; 8],
    pub ga_int_enable: i32,
    pub ga_int_trigger: i32,
    pub sw_pos: u8,
    pub io_sd: u8,
    pub adf_rd: i32,
    pub analog_end_time: u64,
    pub ssr_rd: i32,
    pub uart_rx_byte: u8,
    pub uart_rx_delay: u64,
    pub uart_tx_delay: u64,

    pub frt: [FrtState; 3],
    pub timer: McuTimerState,
    pub timer_cycles: u64,
    pub timer_tempreg: u8,

    pub pcm: PcmState,
    pub(crate) waverom1: ImmutableResource<Vec<u8>>,
    pub(crate) waverom2: ImmutableResource<Vec<u8>>,
    pub(crate) waverom3: ImmutableResource<Vec<u8>>,
    pub(crate) waverom_card: ImmutableResource<Vec<u8>>,
    pub(crate) waverom_exp: ImmutableResource<Vec<u8>>,

    pub sm: SubMcuState,
    pub(crate) sm_rom: ImmutableResource<[u8; 4096]>,

    pub sm_ram: [u8; 128],
    pub sm_shared_ram: [u8; 192],
    pub sm_access: [u8; 0x18],
    pub sm_p0_dir: u8,
    pub sm_p1_dir: u8,
    pub sm_device_mode: [u8; 32],
    pub sm_cts: u8,
    pub sm_timer_cycles: u64,
    pub sm_timer_prescaler: u8,
    pub sm_timer_counter: u8,
    pub sm_uart_rx_gotbyte: u8,
    pub sm_uart_rx_byte: u8,
    pub sm_uart_rx_delay: u64,

    pub operand_type: u32,
    pub operand_ea: u16,
    pub operand_ep: u8,
    pub operand_size: u8,
    pub operand_reg: u8,
    pub operand_status: u8,
    pub operand_data: u16,
    pub opcode_extended: u8,

    pub render_output: Vec<f32>,
    pub render_frames_written: u32,
    pub render_frames_requested: u32,
}

impl Default for Sc55State {
    fn default() -> Self {
        Self {
            mcu: McuState::default(),
            dev_register: [0; 0x80],
            romset: 0,
            mcu_mk1: false,
            mcu_cm300: false,
            mcu_st: false,
            mcu_jv880: false,
            mcu_scb55: false,
            mcu_sc155: false,
            uart_write_ptr: 0,
            uart_read_ptr: 0,
            uart_buffer: Box::new([0; UART_BUFFER_SIZE]),
            rom1: ImmutableResource::new([0; ROM1_SIZE]),
            rom2: ImmutableResource::new(vec![0; ROM2_SIZE]),
            rom2_mask: ROM2_SIZE as i32 - 1,
            ram: [0; RAM_SIZE],
            sram: Box::new([0; SRAM_SIZE]),
            nvram: Box::new([0; NVRAM_SIZE]),
            cardram: Box::new([0; CARDRAM_SIZE]),
            ga_int: [0; 8],
            ga_int_enable: 0,
            ga_int_trigger: 0,
            sw_pos: 3,
            io_sd: 0x00,
            adf_rd: 0,
            analog_end_time: 0,
            ssr_rd: 0,
            uart_rx_byte: 0,
            uart_rx_delay: 0,
            uart_tx_delay: 0,
            frt: [FrtState::default(); 3],
            timer: McuTimerState::default(),
            timer_cycles: 0,
            timer_tempreg: 0,
            pcm: PcmState::default(),
            waverom1: ImmutableResource::new(Vec::new()),
            waverom2: ImmutableResource::new(Vec::new()),
            waverom3: ImmutableResource::new(Vec::new()),
            waverom_card: ImmutableResource::new(Vec::new()),
            waverom_exp: ImmutableResource::new(Vec::new()),
            sm: SubMcuState::default(),
            sm_rom: ImmutableResource::new([0; 4096]),
            sm_ram: [0; 128],
            sm_shared_ram: [0; 192],
            sm_access: [0; 0x18],
            sm_p0_dir: 0,
            sm_p1_dir: 0,
            sm_device_mode: [0; 32],
            sm_cts: 0,
            sm_timer_cycles: 0,
            sm_timer_prescaler: 0,
            sm_timer_counter: 0,
            sm_uart_rx_gotbyte: 0,
            sm_uart_rx_byte: 0,
            sm_uart_rx_delay: 0,
            operand_type: 0,
            operand_ea: 0,
            operand_ep: 0,
            operand_size: 0,
            operand_reg: 0,
            operand_status: 0,
            operand_data: 0,
            opcode_extended: 0,
            render_output: Vec::new(),
            render_frames_written: 0,
            render_frames_requested: 0,
        }
    }
}

impl Sc55State {
    pub(crate) fn attach_resources(&mut self, active: &Self) {
        self.rom1 = active.rom1.clone();
        self.rom2 = active.rom2.clone();
        self.waverom1 = active.waverom1.clone();
        self.waverom2 = active.waverom2.clone();
        self.waverom3 = active.waverom3.clone();
        self.waverom_card = active.waverom_card.clone();
        self.waverom_exp = active.waverom_exp.clone();
        self.sm_rom = active.sm_rom.clone();
    }

    pub(crate) fn validate_for_restore(&self) -> Result<(), String> {
        if self.uart_write_ptr as usize >= self.uart_buffer.len()
            || self.uart_read_ptr as usize >= self.uart_buffer.len()
        {
            return Err("SC-55 UART position is invalid".to_owned());
        }
        if self.pcm.select_channel >= 32
            || self.pcm.irq_channel >= 32
            || self.render_frames_written > self.render_frames_requested
            || self.render_output.len() < self.render_frames_requested as usize * 2
        {
            return Err("SC-55 render state is invalid".to_owned());
        }
        Ok(())
    }
}

crate::impl_state_codec!(Sc55State {
    mcu,
    dev_register,
    romset,
    mcu_mk1,
    mcu_cm300,
    mcu_st,
    mcu_jv880,
    mcu_scb55,
    mcu_sc155,
    uart_write_ptr,
    uart_read_ptr,
    uart_buffer,
    rom2_mask,
    ram,
    sram,
    nvram,
    cardram,
    ga_int,
    ga_int_enable,
    ga_int_trigger,
    sw_pos,
    io_sd,
    adf_rd,
    analog_end_time,
    ssr_rd,
    uart_rx_byte,
    uart_rx_delay,
    uart_tx_delay,
    frt,
    timer,
    timer_cycles,
    timer_tempreg,
    pcm,
    sm,
    sm_ram,
    sm_shared_ram,
    sm_access,
    sm_p0_dir,
    sm_p1_dir,
    sm_device_mode,
    sm_cts,
    sm_timer_cycles,
    sm_timer_prescaler,
    sm_timer_counter,
    sm_uart_rx_gotbyte,
    sm_uart_rx_byte,
    sm_uart_rx_delay,
    operand_type,
    operand_ea,
    operand_ep,
    operand_size,
    operand_reg,
    operand_status,
    operand_data,
    opcode_extended,
    render_output,
    render_frames_written,
    render_frames_requested,
} defaults {
    rom1: ImmutableResource::new([0; ROM1_SIZE]),
    rom2: ImmutableResource::new(Vec::new()),
    waverom1: ImmutableResource::new(Vec::new()),
    waverom2: ImmutableResource::new(Vec::new()),
    waverom3: ImmutableResource::new(Vec::new()),
    waverom_card: ImmutableResource::new(Vec::new()),
    waverom_exp: ImmutableResource::new(Vec::new()),
    sm_rom: ImmutableResource::new([0; 4096]),
});
