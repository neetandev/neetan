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

use std::{
    fs,
    path::{Path, PathBuf},
};

use common::info;
use save_state::{ResourceBinding, ResourceBindingId, ResourceIdentity};

use crate::{
    Sc55State, mcu, mcu_interrupt, mcu_timer, pcm,
    state::{ROM1_SIZE, ROM2_SIZE},
    submcu,
};

const SM_ROM_SIZE: usize = 0x1000;

const ROM_SET_COUNT: usize = 9;
const FILES_PER_SET: usize = 6;

#[rustfmt::skip]
const ROMS: [[&str; FILES_PER_SET]; ROM_SET_COUNT] = [
    // Mk2
    ["rom1.bin", "rom2.bin", "waverom1.bin", "waverom2.bin", "rom_sm.bin", ""],
    // St
    ["rom1.bin", "rom2_st.bin", "waverom1.bin", "waverom2.bin", "rom_sm.bin", ""],
    // Mk1
    ["sc55_rom1.bin", "sc55_rom2.bin", "sc55_waverom1.bin", "sc55_waverom2.bin", "sc55_waverom3.bin", ""],
    // Cm300
    ["cm300_rom1.bin", "cm300_rom2.bin", "cm300_waverom1.bin", "cm300_waverom2.bin", "cm300_waverom3.bin", ""],
    // Jv880
    ["jv880_rom1.bin", "jv880_rom2.bin", "jv880_waverom1.bin", "jv880_waverom2.bin", "jv880_waverom_expansion.bin", "jv880_waverom_pcmcard.bin"],
    // Scb55
    ["scb55_rom1.bin", "scb55_rom2.bin", "scb55_waverom1.bin", "scb55_waverom2.bin", "", ""],
    // Rlp3237
    ["rlp3237_rom1.bin", "rlp3237_rom2.bin", "rlp3237_waverom1.bin", "", "", ""],
    // Sc155
    ["sc155_rom1.bin", "sc155_rom2.bin", "sc155_waverom1.bin", "sc155_waverom2.bin", "sc155_waverom3.bin", ""],
    // Sc155Mk2
    ["rom1.bin", "rom2.bin", "waverom1.bin", "waverom2.bin", "rom_sm.bin", ""],
];

const ROM_SET_NAMES: [&str; ROM_SET_COUNT] = [
    "SC-55mk2",
    "SC-55st",
    "SC-55mk1",
    "CM-300/SCC-1",
    "JV-880",
    "SCB-55",
    "RLP-3237",
    "SC-155",
    "SC-155mk2",
];

#[repr(i32)]
#[derive(Debug, Clone, Copy)]
enum RomSet {
    Mk2 = 0,
    St = 1,
    Mk1 = 2,
    Cm300 = 3,
    Jv880 = 4,
    Scb55 = 5,
    Rlp3237 = 6,
    Sc155 = 7,
    Sc155Mk2 = 8,
}

impl RomSet {
    fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Mk2),
            1 => Some(Self::St),
            2 => Some(Self::Mk1),
            3 => Some(Self::Cm300),
            4 => Some(Self::Jv880),
            5 => Some(Self::Scb55),
            6 => Some(Self::Rlp3237),
            7 => Some(Self::Sc155),
            8 => Some(Self::Sc155Mk2),
            _ => None,
        }
    }

    fn is_mk1(self) -> bool {
        matches!(self, Self::Mk1 | Self::Cm300 | Self::Sc155)
    }

    fn is_jv880(self) -> bool {
        matches!(self, Self::Jv880)
    }

    fn is_scb55(self) -> bool {
        matches!(self, Self::Scb55 | Self::Rlp3237)
    }
}

fn detect_romset(rom_directory: &Path) -> Option<RomSet> {
    for (set_index, filenames) in ROMS.iter().enumerate() {
        let all_found = filenames[..5]
            .iter()
            .all(|name| name.is_empty() || rom_directory.join(name).exists());
        if all_found {
            return RomSet::from_index(set_index);
        }
    }
    None
}

fn read_rom(
    rom_directory: &Path,
    romset: RomSet,
    file_index: usize,
) -> Result<Vec<u8>, Sc55ContextError> {
    let filename = ROMS[romset as usize][file_index];
    let path = rom_directory.join(filename);
    fs::read(&path).map_err(|source| Sc55ContextError::RomReadError { path, source })
}

fn set_romset(state: &mut Sc55State, set: RomSet) {
    state.romset = set as i32;
    state.mcu_mk1 = false;
    state.mcu_cm300 = false;
    state.mcu_st = false;
    state.mcu_jv880 = false;
    state.mcu_scb55 = false;
    state.mcu_sc155 = false;

    match set {
        RomSet::Mk2 => {}
        RomSet::Sc155Mk2 => {
            state.mcu_sc155 = true;
        }
        RomSet::St => {
            state.mcu_st = true;
        }
        RomSet::Mk1 => {
            state.mcu_mk1 = true;
        }
        RomSet::Sc155 => {
            state.mcu_mk1 = true;
            state.mcu_sc155 = true;
        }
        RomSet::Cm300 => {
            state.mcu_mk1 = true;
            state.mcu_cm300 = true;
        }
        RomSet::Jv880 => {
            state.mcu_jv880 = true;
            state.rom2_mask = (0x80000 / 2) - 1;
        }
        RomSet::Scb55 | RomSet::Rlp3237 => {
            state.mcu_scb55 = true;
        }
    }
}

fn load_roms(
    state: &mut Sc55State,
    rom_directory: &Path,
    romset: RomSet,
) -> Result<u32, Sc55ContextError> {
    set_romset(state, romset);

    let filenames = &ROMS[romset as usize];

    // ROM1
    let rom1_data = read_rom(rom_directory, romset, 0)?;
    if rom1_data.len() != ROM1_SIZE {
        return Err(Sc55ContextError::RomSizeMismatch {
            path: rom_directory.join(filenames[0]),
            expected: ROM1_SIZE,
            actual: rom1_data.len(),
        });
    }
    state.rom1[..rom1_data.len()].copy_from_slice(&rom1_data);

    // ROM2 (accepts full or half size)
    let rom2_data = read_rom(rom_directory, romset, 1)?;
    if rom2_data.len() != ROM2_SIZE && rom2_data.len() != ROM2_SIZE / 2 {
        return Err(Sc55ContextError::RomSizeMismatch {
            path: rom_directory.join(filenames[1]),
            expected: ROM2_SIZE,
            actual: rom2_data.len(),
        });
    }
    state.rom2.resize(rom2_data.len(), 0);
    state.rom2[..rom2_data.len()].copy_from_slice(&rom2_data);
    state.rom2_mask = rom2_data.len() as i32 - 1;

    // Wave ROMs (model-dependent)
    if romset.is_mk1() {
        load_required_waverom(state, rom_directory, romset, 2, 1, 0x100000)?;
        load_required_waverom(state, rom_directory, romset, 3, 2, 0x100000)?;
        load_required_waverom(state, rom_directory, romset, 4, 3, 0x100000)?;
    } else if romset.is_jv880() {
        load_required_waverom(state, rom_directory, romset, 2, 1, 0x200000)?;
        load_required_waverom(state, rom_directory, romset, 3, 2, 0x200000)?;
        load_optional_waverom(state, rom_directory, romset, 4, 4, 0x800000);
        load_optional_waverom(state, rom_directory, romset, 5, 5, 0x200000);
    } else {
        load_required_waverom(state, rom_directory, romset, 2, 1, 0x200000)?;

        if !filenames[3].is_empty() {
            let waverom_index = if romset.is_scb55() { 3 } else { 2 };
            load_required_waverom(state, rom_directory, romset, 3, waverom_index, 0x100000)?;
        }

        if !filenames[4].is_empty() {
            let sm_data = read_rom(rom_directory, romset, 4)?;
            if sm_data.len() != SM_ROM_SIZE {
                return Err(Sc55ContextError::RomSizeMismatch {
                    path: rom_directory.join(filenames[4]),
                    expected: SM_ROM_SIZE,
                    actual: sm_data.len(),
                });
            }
            state.sm_rom[..sm_data.len()].copy_from_slice(&sm_data);
        }
    }

    let native_rate = if state.mcu_mk1 || state.mcu_jv880 {
        64000
    } else {
        66207
    };

    Ok(native_rate)
}

fn load_waverom(state: &mut Sc55State, waverom_index: i32, data: &[u8]) {
    let len = data.len();

    let destination = match waverom_index {
        1 => &mut state.waverom1,
        2 => &mut state.waverom2,
        3 => &mut state.waverom3,
        4 => &mut state.waverom_exp,
        5 => &mut state.waverom_card,
        _ => return,
    };

    destination.resize(len, 0);
    mcu::unscramble(data, destination, len);
}

fn load_required_waverom(
    state: &mut Sc55State,
    rom_directory: &Path,
    romset: RomSet,
    file_index: usize,
    waverom_index: i32,
    expected_size: usize,
) -> Result<(), Sc55ContextError> {
    let data = read_rom(rom_directory, romset, file_index)?;
    if data.len() != expected_size {
        return Err(Sc55ContextError::RomSizeMismatch {
            path: rom_directory.join(ROMS[romset as usize][file_index]),
            expected: expected_size,
            actual: data.len(),
        });
    }
    load_waverom(state, waverom_index, &data);
    Ok(())
}

fn load_optional_waverom(
    state: &mut Sc55State,
    rom_directory: &Path,
    romset: RomSet,
    file_index: usize,
    waverom_index: i32,
    expected_size: usize,
) {
    let filename = ROMS[romset as usize][file_index];
    if filename.is_empty() {
        return;
    }
    let path = rom_directory.join(filename);
    if let Ok(data) = fs::read(&path)
        && data.len() == expected_size
    {
        load_waverom(state, waverom_index, &data);
    }
}

pub(crate) struct Sc55Context {
    state: Sc55State,
    native_sample_rate: u32,
    resource_bindings: Vec<ResourceBinding>,
}

impl Sc55Context {
    pub(crate) fn new(rom_directory: &Path) -> Result<Self, Sc55ContextError> {
        let mut state = Sc55State::default();

        let romset = detect_romset(rom_directory).ok_or(Sc55ContextError::NoRomSetFound)?;

        info!(
            "Detected Roland {} ROM set, starting emulation",
            ROM_SET_NAMES[romset as usize]
        );

        let native_sample_rate = load_roms(&mut state, rom_directory, romset)?;
        let resource_bindings = ROMS[romset as usize]
            .iter()
            .enumerate()
            .filter(|(_, filename)| !filename.is_empty())
            .filter(|(_, filename)| rom_directory.join(filename).exists())
            .map(|(index, filename)| {
                let path = rom_directory.join(filename);
                fs::read(&path)
                    .map(|bytes| ResourceBinding {
                        identifier: ResourceBindingId::new(format!("sc55-rom-{index}")).unwrap(),
                        identity: ResourceIdentity::from_bytes(&bytes),
                    })
                    .map_err(|source| Sc55ContextError::RomReadError { path, source })
            })
            .collect::<Result<Vec<_>, _>>()?;

        mcu::mcu_init(&mut state);
        mcu::mcu_patch_rom(&mut state);
        mcu::mcu_reset(&mut state);
        submcu::sm_reset(&mut state);
        pcm::pcm_reset(&mut state);

        // Send GS Reset SysEx to initialize the sound module properly.
        let gs_reset: [u8; 11] = [
            0xF0, 0x41, 0x10, 0x42, 0x12, 0x40, 0x00, 0x7F, 0x00, 0x41, 0xF7,
        ];
        for &byte in &gs_reset {
            mcu::mcu_post_uart(&mut state, byte);
        }

        Ok(Self {
            state,
            native_sample_rate,
            resource_bindings,
        })
    }

    pub(crate) fn post_midi(&mut self, byte: u8) {
        mcu::mcu_post_uart(&mut self.state, byte);
    }

    pub(crate) fn render(&mut self, output: &mut [f32], num_frames: u32) {
        if num_frames == 0 {
            return;
        }
        assert!(output.len() >= (num_frames as usize) * 2);

        let state = &mut self.state;

        // Set up render output buffer.
        state.render_output.clear();
        state.render_output.resize((num_frames as usize) * 2, 0.0);
        state.render_frames_written = 0;
        state.render_frames_requested = num_frames;

        while state.render_frames_written < num_frames {
            if state.mcu.ex_ignore == 0 {
                mcu_interrupt::mcu_interrupt_handle(state);
            } else {
                state.mcu.ex_ignore = 0;
            }

            if state.mcu.sleep == 0 {
                mcu::mcu_read_instruction(state);
            }

            state.mcu.cycles += 12;

            pcm::pcm_update(state, state.mcu.cycles);
            mcu_timer::timer_clock(state, state.mcu.cycles);

            if !state.mcu_mk1 && !state.mcu_jv880 && !state.mcu_scb55 {
                submcu::sm_update(state, state.mcu.cycles);
            } else {
                mcu::mcu_update_uart_rx(state);
                mcu::mcu_update_uart_tx(state);
            }

            mcu::mcu_update_analog(state, state.mcu.cycles);
        }

        // Copy rendered samples to output.
        let produced = state.render_frames_written as usize * 2;
        output[..produced].copy_from_slice(&state.render_output[..produced]);
    }

    pub(crate) fn sample_rate(&self) -> u32 {
        self.native_sample_rate
    }

    pub(crate) fn resource_bindings(&self) -> &[ResourceBinding] {
        &self.resource_bindings
    }

    pub(crate) fn capture_state(&self) -> Sc55State {
        self.state.clone()
    }

    pub(crate) fn attach_resources(&self, state: &mut Sc55State) {
        state.attach_resources(&self.state);
    }

    pub(crate) fn restore_state(&mut self, mut state: Sc55State) {
        state.attach_resources(&self.state);
        self.state = state;
    }
}

#[derive(Debug)]
pub enum Sc55ContextError {
    NoRomSetFound,
    RomReadError {
        path: PathBuf,
        source: std::io::Error,
    },
    RomSizeMismatch {
        path: PathBuf,
        expected: usize,
        actual: usize,
    },
}

impl std::fmt::Display for Sc55ContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoRomSetFound => write!(f, "no compatible SC-55 ROM set found"),
            Self::RomReadError { path, source } => {
                write!(f, "failed to read ROM file {}: {source}", path.display())
            }
            Self::RomSizeMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "ROM file {} has wrong size (expected 0x{expected:X}, got 0x{actual:X})",
                path.display()
            ),
        }
    }
}
