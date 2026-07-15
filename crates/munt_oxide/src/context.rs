// Copyright (C) 2003, 2004, 2005, 2006, 2008, 2009 Dean Beeler, Jerome Fisher
// Copyright (C) 2011-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
//  This program is free software: you can redistribute it and/or modify
//  it under the terms of the GNU Lesser General Public License as published by
//  the Free Software Foundation, either version 2.1 of the License, or
//  (at your option) any later version.
//
//  This program is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU Lesser General Public License for more details.
//
//  You should have received a copy of the GNU Lesser General Public License
//  along with this program.  If not, see <http://www.gnu.org/licenses/>.

use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

use common::info;
use save_state::{ResourceBinding, ResourceBindingId, ResourceIdentity};

use crate::{
    rom_info::{self, PairType, RomInfo, RomType},
    state::{MuntState, SAMPLE_RATE},
};

const MAX_PARSED_MIDI_MESSAGES: usize = 32768;
const MAX_PARSED_SYSEX_BYTES: usize = 65536;

#[derive(Clone, Copy)]
enum ParsedMidiMessage {
    Short(u32),
    Sysex { start: usize, length: usize },
}

struct ParsedMidiScratch<'a> {
    messages: &'a mut Vec<ParsedMidiMessage>,
    sysex: &'a mut Vec<u8>,
}

pub(crate) struct MuntContext {
    state: MuntState,
    sample_rate: u32,
    resource_bindings: Vec<ResourceBinding>,
    parsed_messages: Vec<ParsedMidiMessage>,
    parsed_sysex: Vec<u8>,
}

impl MuntContext {
    pub(crate) fn new(rom_directory: &Path) -> Result<Self, MuntContextError> {
        if !rom_directory.is_dir() {
            return Err(MuntContextError::DirectoryNotFound(
                rom_directory.to_path_buf(),
            ));
        }

        let entries = fs::read_dir(rom_directory).map_err(|error| {
            MuntContextError::DirectoryReadFailed(rom_directory.to_path_buf(), error)
        })?;

        let mut control_rom: Option<(Vec<u8>, &'static RomInfo)> = None;
        let mut pcm_rom: Option<(Vec<u8>, &'static RomInfo)> = None;

        // Collect all identified partial ROMs for potential pairing.
        let mut partial_control_roms: Vec<(Vec<u8>, &'static RomInfo)> = Vec::new();
        let mut partial_pcm_roms: Vec<(Vec<u8>, &'static RomInfo)> = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            let is_rom = path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rom"));
            if !is_rom {
                continue;
            }

            let data = match fs::read(&path) {
                Ok(data) => data,
                Err(_) => continue,
            };

            let Some(info) = rom_info::get_rom_info(&data) else {
                continue;
            };

            match info.pair_type {
                PairType::Full => match info.rom_type {
                    RomType::Control => {
                        if control_rom.is_none() {
                            control_rom = Some((data, info));
                        }
                    }
                    RomType::Pcm => {
                        if pcm_rom.is_none() {
                            pcm_rom = Some((data, info));
                        }
                    }
                },
                _ => match info.rom_type {
                    RomType::Control => {
                        partial_control_roms.push((data, info));
                    }
                    RomType::Pcm => {
                        partial_pcm_roms.push((data, info));
                    }
                },
            }
        }

        // Try to assemble full ROMs from partial halves if we are missing a full one.
        if control_rom.is_none() {
            control_rom = try_merge_partials(&partial_control_roms);
        }
        if pcm_rom.is_none() {
            pcm_rom = try_merge_partials(&partial_pcm_roms);
        }

        let (control_data, control_info) =
            control_rom.ok_or(MuntContextError::NoRomsFound(rom_directory.to_path_buf()))?;
        let (pcm_data, pcm_info) =
            pcm_rom.ok_or(MuntContextError::NoRomsFound(rom_directory.to_path_buf()))?;

        let resource_bindings = vec![
            ResourceBinding {
                identifier: ResourceBindingId::new("mt32-control-rom").unwrap(),
                identity: ResourceIdentity::from_bytes(&control_data),
            },
            ResourceBinding {
                identifier: ResourceBindingId::new("mt32-pcm-rom").unwrap(),
                identity: ResourceIdentity::from_bytes(&pcm_data),
            },
        ];

        info!(
            "MT-32 ROMs identified: {} + {}",
            control_info.description, pcm_info.description
        );

        let mut state = MuntState::default();

        if !crate::synth::open(
            &mut state,
            &control_data,
            &pcm_data,
            control_info.short_name,
        ) {
            return Err(MuntContextError::SynthOpenFailed(
                "synth::open returned false".to_string(),
            ));
        }

        let sample_rate = SAMPLE_RATE;

        info!("MT-32 synth opened successfully (munt_oxide, {sample_rate} Hz)");

        Ok(Self {
            state,
            sample_rate,
            resource_bindings,
            parsed_messages: Vec::with_capacity(MAX_PARSED_MIDI_MESSAGES),
            parsed_sysex: Vec::with_capacity(MAX_PARSED_SYSEX_BYTES),
        })
    }

    pub(crate) fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub(crate) fn resource_bindings(&self) -> &[ResourceBinding] {
        &self.resource_bindings
    }

    pub(crate) fn parse_stream(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        self.parsed_messages.clear();
        self.parsed_sysex.clear();
        let scratch = core::cell::RefCell::new(ParsedMidiScratch {
            messages: &mut self.parsed_messages,
            sysex: &mut self.parsed_sysex,
        });
        self.state.midi_stream_parser.parse_stream(
            data,
            &mut |message| {
                let mut scratch = scratch.borrow_mut();
                if scratch.messages.len() < MAX_PARSED_MIDI_MESSAGES {
                    scratch.messages.push(ParsedMidiMessage::Short(message));
                }
            },
            &mut |sysex| {
                let mut scratch = scratch.borrow_mut();
                if scratch.messages.len() >= MAX_PARSED_MIDI_MESSAGES
                    || sysex.len() > MAX_PARSED_SYSEX_BYTES - scratch.sysex.len()
                {
                    return;
                }
                let start = scratch.sysex.len();
                scratch.sysex.extend_from_slice(sysex);
                scratch.messages.push(ParsedMidiMessage::Sysex {
                    start,
                    length: sysex.len(),
                });
            },
            &mut |_realtime| {},
        );
        let state = &mut self.state;
        for &message in &self.parsed_messages {
            match message {
                ParsedMidiMessage::Short(message) => {
                    let _ = crate::synth::play_msg(state, message);
                }
                ParsedMidiMessage::Sysex { start, length } => {
                    let _ =
                        crate::synth::play_sysex(state, &self.parsed_sysex[start..start + length]);
                }
            }
        }
    }

    pub(crate) fn render(&mut self, output: &mut [f32], num_frames: u32) {
        crate::synth::render(&mut self.state, output, num_frames);
    }

    pub(crate) fn capture_state(&self) -> MuntState {
        self.state.clone()
    }

    pub(crate) fn attach_resources(&self, state: &mut MuntState) {
        state.attach_resources(&self.state);
    }

    pub(crate) fn restore_state(&mut self, mut state: MuntState) {
        state.attach_resources(&self.state);
        self.state = state;
    }
}

impl Drop for MuntContext {
    fn drop(&mut self) {
        crate::synth::close(&mut self.state);
    }
}

fn try_merge_partials(
    partials: &[(Vec<u8>, &'static RomInfo)],
) -> Option<(Vec<u8>, &'static RomInfo)> {
    for (i, (data_a, info_a)) in partials.iter().enumerate() {
        let Some(pair_index) = info_a.pair_rom_info_index else {
            continue;
        };
        for (data_b, info_b) in partials.iter().skip(i + 1) {
            let all = rom_info::get_all_rom_infos();
            if !std::ptr::eq(*info_b, &all[pair_index]) {
                continue;
            }
            let merged = merge_rom_pair(data_a, info_a, data_b, info_b);
            if let Some(merged_data) = merged {
                // After merging, identify the full ROM.
                if let Some(full_info) = rom_info::get_rom_info(&merged_data)
                    && full_info.pair_type == PairType::Full
                {
                    return Some((merged_data, full_info));
                }
            }
        }
    }
    None
}

fn merge_rom_pair(
    data_a: &[u8],
    info_a: &RomInfo,
    data_b: &[u8],
    info_b: &RomInfo,
) -> Option<Vec<u8>> {
    // Determine which is first/mux0 and which is second/mux1.
    let (first_data, second_data, first_info) = match (info_a.pair_type, info_b.pair_type) {
        (PairType::FirstHalf, PairType::SecondHalf) | (PairType::Mux0, PairType::Mux1) => {
            (data_a, data_b, info_a)
        }
        (PairType::SecondHalf, PairType::FirstHalf) | (PairType::Mux1, PairType::Mux0) => {
            (data_b, data_a, info_b)
        }
        _ => return None,
    };

    match first_info.pair_type {
        PairType::FirstHalf => {
            let mut merged = Vec::with_capacity(first_data.len() + second_data.len());
            merged.extend_from_slice(first_data);
            merged.extend_from_slice(second_data);
            Some(merged)
        }
        PairType::Mux0 => {
            let mut merged = vec![0u8; first_data.len() * 2];
            for (i, &byte) in first_data.iter().enumerate() {
                merged[i * 2] = byte;
            }
            for (i, &byte) in second_data.iter().enumerate() {
                merged[i * 2 + 1] = byte;
            }
            Some(merged)
        }
        _ => None,
    }
}

#[derive(Debug)]
pub enum MuntContextError {
    DirectoryNotFound(PathBuf),
    DirectoryReadFailed(PathBuf, std::io::Error),
    NoRomsFound(PathBuf),
    SynthOpenFailed(String),
}

impl fmt::Display for MuntContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirectoryNotFound(path) => {
                write!(f, "ROM directory not found: {}", path.display())
            }
            Self::DirectoryReadFailed(path, error) => {
                write!(
                    f,
                    "failed to read ROM directory {}: {error}",
                    path.display()
                )
            }
            Self::NoRomsFound(path) => {
                write!(f, "no MT-32 ROM files found in {}", path.display())
            }
            Self::SynthOpenFailed(reason) => {
                write!(f, "failed to open MT-32 synth: {reason}")
            }
        }
    }
}

impl std::error::Error for MuntContextError {}
