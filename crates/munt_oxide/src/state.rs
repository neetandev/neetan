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

// Central mutable state for the MT-32 emulator. Every free function receives
// `&mut MuntState` instead of scattered `this` pointers. The design mirrors
// the approach in `nuked_sc55_oxide::state::Sc55State`.

use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

use crate::{
    enumerations::{DacInputMode, MidiDelayMode, PolyState, ReverbMode},
    structures::{
        ControlROMFeatureSet, ControlROMMap, ControlROMPCMStruct, MemParams, PCMWaveEntry,
        PatchCache,
    },
    tables::Tables,
};

pub(crate) const SAMPLE_RATE: u32 = 32000;
pub(crate) const DEFAULT_MAX_PARTIALS: usize = 32;
pub(crate) const MAX_PARTS: usize = 9;
pub(crate) const MAX_SAMPLES_PER_RUN: usize = 4096;
pub(crate) const MAX_STREAM_BUFFER_SIZE: usize = 32768;
pub(crate) const DEFAULT_MIDI_EVENT_QUEUE_SIZE: usize = 1024;
pub(crate) const CONTROL_ROM_SIZE: usize = 64 * 1024;

/// The maximum number of drum timbres in the rhythm part cache.
pub(crate) const DRUM_CACHE_COUNT: usize = 85;

pub(crate) const SYSEX_MANUFACTURER_ROLAND: u8 = 0x41;
pub(crate) const SYSEX_MDL_MT32: u8 = 0x16;
pub(crate) const SYSEX_CMD_RQ1: u8 = 0x11;
pub(crate) const SYSEX_CMD_DT1: u8 = 0x12;
pub(crate) const SYSEX_CMD_WSD: u8 = 0x40;
pub(crate) const SYSEX_CMD_RQD: u8 = 0x41;
pub(crate) const SYSEX_CMD_DAT: u8 = 0x42;
pub(crate) const SYSEX_CMD_EOD: u8 = 0x45;

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

/// Coarse LPF delay line length (must be a power of 2).
pub(crate) const COARSE_LPF_DELAY_LINE_LENGTH: usize = 8;

/// TVA envelope phases.
/// Note that when entering next_phase(), new_phase is set to phase + 1,
/// and the descriptions/names below refer to new_phase's value.
///
/// In this phase, the base amp (as calculated in calc_basic_amp()) is targeted with an instant time.
/// This phase is entered by reset() only if time[0] != 0.
pub(crate) const TVA_PHASE_BASIC: i32 = 0;
/// In this phase, level[0] is targeted within time[0], and velocity potentially affects time.
pub(crate) const TVA_PHASE_ATTACK: i32 = 1;
/// In this phase, level[1] is targeted within time[1].
pub(crate) const TVA_PHASE_2: i32 = 2;
/// In this phase, level[2] is targeted within time[2].
pub(crate) const TVA_PHASE_3: i32 = 3;
/// In this phase, level[3] is targeted within time[3].
pub(crate) const TVA_PHASE_4: i32 = 4;
/// In this phase, immediately goes to PHASE_RELEASE unless the poly is set to sustain.
/// Aborts the partial if level[3] is 0.
/// Otherwise level[3] is continued, no phase change will occur until some external influence
/// (like pedal release).
pub(crate) const TVA_PHASE_SUSTAIN: i32 = 5;
/// In this phase, 0 is targeted within time[4]
/// (the time calculation is quite different from the other phases).
pub(crate) const TVA_PHASE_RELEASE: i32 = 6;
/// It's PHASE_DEAD, Jim.
pub(crate) const TVA_PHASE_DEAD: i32 = 7;

/// LA32 pair type (master or slave).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum PairType {
    Master = 0,
    Slave = 1,
}

#[derive(Clone, Default)]
/// Authoritative progress of one LA32 ramp generator.
pub(crate) struct La32RampState {
    pub(crate) current: u32,
    pub(crate) large_target: u32,
    pub(crate) large_increment: u32,
    pub(crate) descending: bool,
    pub(crate) interrupt_countdown: i32,
    pub(crate) interrupt_raised: bool,
}

#[derive(Clone, Default)]
/// Authoritative progress of one LA32 waveform generator.
pub(crate) struct La32FloatWaveGeneratorState {
    pub(crate) active: bool,
    pub(crate) sawtooth_waveform: bool,
    pub(crate) resonance: u8,
    pub(crate) pulse_width: u8,

    /// PCM wave parameters (only valid when generating PCM output).
    pub(crate) pcm_wave_address_offset: u32,
    pub(crate) pcm_wave_length: u32,
    pub(crate) pcm_wave_looped: bool,
    pub(crate) pcm_wave_interpolated: bool,

    /// Internal variables.
    pub(crate) wave_pos: f32,
    pub(crate) last_freq: f32,
    pub(crate) pcm_position: f32,
}

#[derive(Clone, Default)]
/// Authoritative state of one paired LA32 synthesis path.
pub(crate) struct La32PairState {
    pub(crate) master: La32FloatWaveGeneratorState,
    pub(crate) slave: La32FloatWaveGeneratorState,
    pub(crate) ring_modulated: bool,
    pub(crate) mixed: bool,
    pub(crate) master_output_sample: f32,
    pub(crate) slave_output_sample: f32,
}

#[derive(Clone, Default)]
/// Authoritative time-variant amplifier progress.
pub(crate) struct TvaState {
    pub(crate) playing: bool,
    pub(crate) bias_amp_subtraction: i32,
    pub(crate) velo_amp_subtraction: i32,
    pub(crate) key_time_subtraction: i32,
    pub(crate) target: u8,
    pub(crate) phase: i32,
}

#[derive(Clone, Default)]
/// Authoritative time-variant filter progress.
pub(crate) struct TvfState {
    pub(crate) base_cutoff: u8,
    pub(crate) key_time_subtraction: i32,
    pub(crate) level_mult: u32,
    pub(crate) target: u8,
    pub(crate) phase: u32,
}

#[derive(Clone, Default)]
/// Authoritative time-variant pitch progress.
pub(crate) struct TvpState {
    pub(crate) process_timer_increment: i32,
    pub(crate) counter: i32,
    pub(crate) time_elapsed: u32,

    pub(crate) phase: i32,
    pub(crate) base_pitch: u32,
    pub(crate) target_pitch_offset_without_lfo: i32,
    pub(crate) current_pitch_offset: i32,

    pub(crate) lfo_pitch_offset: i16,
    /// In range -12..36.
    pub(crate) time_keyfollow_subtraction: i8,

    pub(crate) pitch_offset_change_per_big_tick: i16,
    pub(crate) target_pitch_offset_reached_big_tick: u16,
    pub(crate) shifts: u32,

    pub(crate) pitch: u16,
}

#[derive(Clone, Default)]
/// Complete authoritative state of one active MT-32 partial.
pub(crate) struct PartialState {
    /// Number of the sample currently being rendered (debug only).
    pub(crate) sample_num: u32,

    /// Pan values. LA-32 receives only 3 bits as a pan setting, but we abuse
    /// these to emulate inverted partial mixing. Doubled for NicePanning mode.
    pub(crate) left_pan_value: i32,
    pub(crate) right_pan_value: i32,

    /// -1 if unassigned.
    pub(crate) owner_part: i32,
    pub(crate) mix_type: i32,
    /// 0 or 1 of a structure pair.
    pub(crate) structure_position: i32,

    /// Only used for PCM partials.
    pub(crate) pcm_num: i32,
    /// Index into the pcm_waves table, or None.
    pub(crate) pcm_wave_index: Option<usize>,

    /// Final pulse width value, with velfollow applied (range 0-255).
    pub(crate) pulse_width_val: i32,

    /// Index of the Poly that owns this partial.
    pub(crate) poly_index: Option<usize>,
    /// Index of the paired Partial, if any.
    pub(crate) pair_index: Option<usize>,
    /// Index into the rhythm_temp table (0-84), set during start_partial for rhythm parts.
    pub(crate) rhythm_temp_index: Option<usize>,

    pub(crate) tva: TvaState,
    pub(crate) tvp: TvpState,
    pub(crate) tvf: TvfState,

    pub(crate) amp_ramp: La32RampState,
    pub(crate) cutoff_modifier_ramp: La32RampState,

    pub(crate) la32_pair: La32PairState,

    pub(crate) patch_cache: PatchCache,
    pub(crate) cache_backup: PatchCache,

    pub(crate) already_outputed: bool,

    /// Whether this partial is active (allocated to a poly).
    pub(crate) active: bool,
}

#[derive(Clone, Default)]
/// Authoritative voice-allocation state of one MT-32 poly.
pub(crate) struct PolyStateData {
    /// Index of the owning Part.
    pub(crate) part_index: Option<usize>,
    pub(crate) key: u32,
    pub(crate) velocity: u32,
    pub(crate) active_partial_count: u32,
    pub(crate) sustain: bool,
    pub(crate) state: PolyState,
    /// Indices into the partial pool. Up to 4 partials per poly.
    pub(crate) partial_indices: [Option<usize>; 4],
    /// Linked-list replacement: index of the next Poly, or None.
    pub(crate) next_index: Option<usize>,
}

#[derive(Clone)]
/// Authoritative controller and voice state of one MT-32 part.
pub(crate) struct PartState {
    /// 0=Part 1, .. 7=Part 8, 8=Rhythm.
    pub(crate) part_num: u32,
    pub(crate) hold_pedal: bool,
    pub(crate) active_partial_count: u32,
    pub(crate) active_non_releasing_poly_count: u32,
    pub(crate) patch_cache: [PatchCache; 4],

    /// Intrusive PolyList replaced by head/tail indices into the poly pool.
    pub(crate) active_polys_first: Option<usize>,
    pub(crate) active_polys_last: Option<usize>,

    /// Name: "Part 1".."Part 8", "Rhythm".
    pub(crate) name: [u8; 8],
    pub(crate) current_instr: [u8; 11],

    /// Values outside the valid range 0..100 imply no override.
    pub(crate) volume_override: u8,
    pub(crate) modulation: u8,
    pub(crate) expression: u8,
    pub(crate) pitch_bend: i32,
    pub(crate) nrpn: bool,
    pub(crate) rpn: u16,
    /// (patchTemp.patch.benderRange * 683) at the time of the last MIDI program change or MIDI data entry.
    pub(crate) pitch_bender_range: u16,

    /// True if this is the rhythm part (index 8).
    pub(crate) is_rhythm: bool,

    /// RhythmPart-specific: cached timbres/settings for each drum note.
    pub(crate) drum_cache: Vec<[PatchCache; 4]>,
}

impl Default for PartState {
    fn default() -> Self {
        Self {
            part_num: 0,
            hold_pedal: false,
            active_partial_count: 0,
            active_non_releasing_poly_count: 0,
            patch_cache: core::array::from_fn(|_| PatchCache::default()),
            active_polys_first: None,
            active_polys_last: None,
            name: [0; 8],
            current_instr: [0; 11],
            volume_override: 0xFF,
            modulation: 0,
            expression: 100,
            pitch_bend: 0,
            nrpn: false,
            rpn: 0xFFFF,
            pitch_bender_range: 0,
            is_rhythm: false,
            drum_cache: Vec::new(),
        }
    }
}

#[derive(Clone, Default)]
/// Authoritative MT-32 partial-pool allocation state.
pub(crate) struct PartialManagerState {
    pub(crate) num_reserved_partials_for_part: [u8; MAX_PARTS],
    pub(crate) free_polys: Vec<usize>,
    /// Holds indices of inactive partials in the partial table.
    pub(crate) inactive_partials: Vec<i32>,
    pub(crate) inactive_partial_count: u32,
}

#[derive(Clone, Default)]
/// One queued MT-32 MIDI event and its timestamp.
pub(crate) struct MidiEvent {
    pub(crate) is_sysex: bool,
    pub(crate) sysex_offset: u32,
    pub(crate) sysex_length: u32,
    pub(crate) short_message_data: u32,
    pub(crate) timestamp: u32,
}

#[derive(Clone, Default)]
/// Authoritative contents and cursors of the MT-32 MIDI event queue.
pub(crate) struct MidiEventQueueState {
    pub(crate) ring_buffer: Vec<MidiEvent>,
    pub(crate) ring_buffer_mask: u32,
    pub(crate) start_position: u32,
    pub(crate) end_position: u32,
    pub(crate) sysex_buffer: Vec<u8>,
    pub(crate) sysex_write_position: u32,
    pub(crate) sysex_used: u32,
}

#[derive(Clone)]
/// Authoritative progress of the MT-32 MIDI byte-stream parser.
pub(crate) struct MidiStreamParserState {
    pub(crate) running_status: u8,
    pub(crate) stream_buffer: Vec<u8>,
    pub(crate) stream_buffer_size: u32,
}

impl Default for MidiStreamParserState {
    fn default() -> Self {
        Self {
            running_status: 0,
            stream_buffer: vec![0; MAX_STREAM_BUFFER_SIZE],
            stream_buffer_size: 0,
        }
    }
}

/// Coarse LPF filter state. Float-only variant for AnalogOutputMode::Coarse.
#[derive(Clone)]
pub(crate) struct CoarseLpfState {
    pub(crate) ring_buffer: [f32; COARSE_LPF_DELAY_LINE_LENGTH],
    pub(crate) ring_buffer_position: u32,
}

impl Default for CoarseLpfState {
    fn default() -> Self {
        Self {
            ring_buffer: [0.0; COARSE_LPF_DELAY_LINE_LENGTH],
            ring_buffer_position: 0,
        }
    }
}

/// Analog stage state. Float-only, coarse mode only.
#[derive(Clone, Default)]
pub(crate) struct AnalogState {
    pub(crate) left_channel_lpf: CoarseLpfState,
    pub(crate) right_channel_lpf: CoarseLpfState,
    pub(crate) synth_gain: f32,
    pub(crate) reverb_gain: f32,
    pub(crate) old_mt32_analog_lpf: bool,
}

/// Ring buffer used by allpass and comb filters inside the reverb.
#[derive(Clone, Default)]
pub(crate) struct ReverbRingBuffer {
    pub(crate) buffer: Vec<f32>,
    pub(crate) size: u32,
    pub(crate) index: u32,
}

/// Allpass filter state inside the reverb.
#[derive(Clone, Default)]
pub(crate) struct ReverbAllpassState {
    pub(crate) ring: ReverbRingBuffer,
}

/// Comb filter state inside the reverb.
#[derive(Clone, Default)]
pub(crate) struct ReverbCombState {
    pub(crate) ring: ReverbRingBuffer,
    pub(crate) filter_factor: u8,
    pub(crate) feedback_factor: u8,
}

/// Tap-delay comb filter state (mode 3 reverb).
#[derive(Clone, Default)]
pub(crate) struct ReverbTapDelayCombState {
    pub(crate) comb: ReverbCombState,
    pub(crate) out_l: u32,
    pub(crate) out_r: u32,
}

/// Delay-with-LPF state (mode 0/1/2 entrance filter).
#[derive(Clone, Default)]
pub(crate) struct ReverbDelayWithLpfState {
    pub(crate) comb: ReverbCombState,
    pub(crate) amp: u8,
}

/// Per-mode reverb state. Virtual dispatch replaced by enum.
#[derive(Clone, Default)]
pub(crate) enum BReverbModelState {
    /// Modes 0 (Room), 1 (Hall), 2 (Plate): 3 allpasses + entrance delay + 3 combs.
    Standard {
        allpasses: Vec<ReverbAllpassState>,
        entrance_delay: ReverbDelayWithLpfState,
        combs: Vec<ReverbCombState>,
        dry_amp: u8,
        wet_level: u8,
        mt32_compatible: bool,
        mode: ReverbMode,
        opened: bool,
    },
    /// Mode 3 (Tap delay): no allpasses, single tap delay comb.
    TapDelay {
        tap_delay_comb: ReverbTapDelayCombState,
        dry_amp: u8,
        wet_level: u8,
        mt32_compatible: bool,
        opened: bool,
    },
    #[default]
    Closed,
}

/// Renderer temporary buffers. Float-only.
#[derive(Clone)]
pub(crate) struct RendererState {
    pub(crate) tmp_non_reverb_left: Vec<f32>,
    pub(crate) tmp_non_reverb_right: Vec<f32>,
    pub(crate) tmp_reverb_dry_left: Vec<f32>,
    pub(crate) tmp_reverb_dry_right: Vec<f32>,
    pub(crate) tmp_reverb_wet_left: Vec<f32>,
    pub(crate) tmp_reverb_wet_right: Vec<f32>,
    pub(crate) tmp_partial_left: Vec<f32>,
    pub(crate) tmp_partial_right: Vec<f32>,
}

impl Default for RendererState {
    fn default() -> Self {
        Self {
            tmp_non_reverb_left: vec![0.0; MAX_SAMPLES_PER_RUN],
            tmp_non_reverb_right: vec![0.0; MAX_SAMPLES_PER_RUN],
            tmp_reverb_dry_left: vec![0.0; MAX_SAMPLES_PER_RUN],
            tmp_reverb_dry_right: vec![0.0; MAX_SAMPLES_PER_RUN],
            tmp_reverb_wet_left: vec![0.0; MAX_SAMPLES_PER_RUN],
            tmp_reverb_wet_right: vec![0.0; MAX_SAMPLES_PER_RUN],
            tmp_partial_left: vec![0.0; MAX_SAMPLES_PER_RUN],
            tmp_partial_right: vec![0.0; MAX_SAMPLES_PER_RUN],
        }
    }
}

/// Memory region descriptor for sysex-addressable memory.
#[derive(Clone, Default)]
pub(crate) struct MemoryRegionDescriptor {
    pub(crate) start_addr: u32,
    pub(crate) entry_size: u32,
    pub(crate) entries: u32,
}

/// ROM data: parsed control ROM + PCM ROM data.
#[derive(Clone)]
pub(crate) struct RomData {
    pub(crate) control_rom_data: Vec<u8>,
    pub(crate) pcm_rom_data: Vec<i16>,
    pub(crate) control_rom_map: Option<&'static ControlROMMap>,
    pub(crate) control_rom_features: ControlROMFeatureSet,
    pub(crate) pcm_waves: Vec<PCMWaveEntry>,
    pub(crate) pcm_rom_structs: Vec<ControlROMPCMStruct>,
    /// Padded timbre max table from the control ROM.
    pub(crate) padded_timbre_max_table: Vec<u8>,
    /// Sound group index: for each standard timbre, the index of its sound group.
    pub(crate) sound_group_ix: [u8; 128],
    pub(crate) sound_group_names: Vec<[u8; 9]>,
}

impl Default for RomData {
    fn default() -> Self {
        Self {
            control_rom_data: vec![0; CONTROL_ROM_SIZE],
            pcm_rom_data: Vec::new(),
            control_rom_map: None,
            control_rom_features: ControlROMFeatureSet::default(),
            pcm_waves: Vec::new(),
            pcm_rom_structs: Vec::new(),
            padded_timbre_max_table: Vec::new(),
            sound_group_ix: [0; 128],
            sound_group_names: Vec::new(),
        }
    }
}

#[derive(Clone)]
/// Mutable state of the MT-32 extension controls.
pub(crate) struct ExtensionsState {
    pub(crate) master_tune_pitch_delta: i32,
    pub(crate) master_volume_override: u8,
    pub(crate) nice_amp_ramp: bool,
    pub(crate) nice_panning: bool,
    pub(crate) nice_partial_mixing: bool,
    /// Reverse mapping of assigned parts per MIDI channel.
    /// Value above 8 means that the channel is not assigned.
    pub(crate) chan_table: [[u8; MAX_PARTS]; 16],
    /// Index of Part in chan_table that failed to play and required partial abortion.
    pub(crate) aborting_part_ix: u32,
}

impl Default for ExtensionsState {
    fn default() -> Self {
        Self {
            master_tune_pitch_delta: 0,
            master_volume_override: 0xFF,
            nice_amp_ramp: true,
            nice_panning: false,
            nice_partial_mixing: false,
            chan_table: [[0xFF; MAX_PARTS]; 16],
            aborting_part_ix: 0,
        }
    }
}

/// The top-level emulator state struct. All mutable state lives here.
#[derive(Clone)]
pub(crate) struct MuntState {
    /// Synth open status.
    pub(crate) opened: bool,
    /// Synth activated status.
    pub(crate) activated: bool,

    /// Main sysex-addressable RAM and its power-on defaults.
    pub(crate) mt32_ram: MemParams,
    pub(crate) mt32_default: MemParams,

    /// ROM data (loaded at open time).
    pub(crate) rom: ImmutableResource<RomData>,
    pub(crate) tables: ImmutableResource<Tables>,

    /// Parts: indices 0..7 = melodic Part 1..8, index 8 = Rhythm.
    pub(crate) parts: [PartState; MAX_PARTS],

    /// Global partial pool. Sized to `partial_count` at open time.
    pub(crate) partials: Vec<PartialState>,
    pub(crate) partial_count: u32,

    /// Global poly pool. Sized to `partial_count * MAX_PARTS` at open time.
    pub(crate) polys: Vec<PolyStateData>,

    /// Partial manager bookkeeping.
    pub(crate) partial_manager: PartialManagerState,

    /// Reverb models for the 4 modes. Index = ReverbMode as usize.
    pub(crate) reverb_models: [BReverbModelState; 4],
    /// Index of the currently active reverb model.
    pub(crate) active_reverb_model: usize,
    pub(crate) reverb_overridden: bool,

    /// MIDI event queue.
    pub(crate) midi_queue: MidiEventQueueState,
    pub(crate) last_received_midi_event_timestamp: u32,
    pub(crate) rendered_sample_count: u32,

    /// MIDI stream parser.
    pub(crate) midi_stream_parser: MidiStreamParserState,
    pub(crate) midi_sysex_scratch: Option<Vec<u8>>,

    /// When a partial needs to be aborted to free it up for use by a new Poly,
    /// the controller will busy-loop waiting for the sound to finish.
    /// We emulate this by delaying new MIDI events processing until abortion finishes.
    pub(crate) aborting_poly_index: Option<usize>,

    /// Analog output stage (coarse float mode).
    pub(crate) analog: AnalogState,

    /// Renderer temporary buffers.
    pub(crate) renderer: RendererState,

    /// Configuration.
    pub(crate) midi_delay_mode: MidiDelayMode,
    pub(crate) dac_input_mode: DacInputMode,
    pub(crate) output_gain: f32,
    pub(crate) reverb_output_gain: f32,
    pub(crate) reversed_stereo_enabled: bool,

    pub(crate) extensions: ExtensionsState,

    /// Memory region descriptors for sysex address mapping.
    pub(crate) memory_regions: MemoryRegionDescriptors,

    /// Lehmer64 PRNG state for pitch deviation noise.
    pub(crate) prng_state: u128,
}

/// Memory region descriptors.
#[derive(Clone, Default)]
pub(crate) struct MemoryRegionDescriptors {
    pub(crate) patch_temp: MemoryRegionDescriptor,
    pub(crate) rhythm_temp: MemoryRegionDescriptor,
    pub(crate) timbre_temp: MemoryRegionDescriptor,
    pub(crate) patches: MemoryRegionDescriptor,
    pub(crate) timbres: MemoryRegionDescriptor,
    pub(crate) system: MemoryRegionDescriptor,
    pub(crate) reset: MemoryRegionDescriptor,
}

impl Default for MuntState {
    fn default() -> Self {
        Self {
            opened: false,
            activated: false,

            mt32_ram: MemParams::default(),
            mt32_default: MemParams::default(),

            rom: ImmutableResource::new(RomData::default()),
            tables: ImmutableResource::new(Tables::new()),

            parts: core::array::from_fn(|_| PartState::default()),
            partials: Vec::new(),
            partial_count: DEFAULT_MAX_PARTIALS as u32,
            polys: Vec::new(),

            partial_manager: PartialManagerState::default(),

            reverb_models: core::array::from_fn(|_| BReverbModelState::default()),
            active_reverb_model: 0,
            reverb_overridden: false,

            midi_queue: MidiEventQueueState::default(),
            last_received_midi_event_timestamp: 0,
            rendered_sample_count: 0,

            midi_stream_parser: MidiStreamParserState::default(),
            midi_sysex_scratch: Some(vec![0; MAX_STREAM_BUFFER_SIZE]),

            aborting_poly_index: None,

            analog: AnalogState::default(),
            renderer: RendererState::default(),

            midi_delay_mode: MidiDelayMode::DelayShortMessagesOnly,
            dac_input_mode: DacInputMode::Nice,
            output_gain: 1.0,
            reverb_output_gain: 1.0,
            reversed_stereo_enabled: false,

            extensions: ExtensionsState::default(),

            memory_regions: MemoryRegionDescriptors::default(),

            // https://xkcd.com/221
            //
            // In this case this is fine.
            // We do not need real randomness here, only input independent
            // randomness for small pitch deviations.
            prng_state: 0x12345678_9ABCDEF0_13579BDF_2468ACE0u128,
        }
    }
}

impl MuntState {
    pub(crate) fn attach_resources(&mut self, active: &Self) {
        self.rom = active.rom.clone();
        self.tables = active.tables.clone();
    }

    pub(crate) fn validate_for_restore(&self) -> Result<(), String> {
        if self.mt32_ram.raw.len() != MemParams::SIZE
            || self.mt32_default.raw.len() != MemParams::SIZE
        {
            return Err("MT-32 memory size differs".to_owned());
        }
        if self.partial_count as usize != self.partials.len()
            || self.polys.len() != self.partials.len()
        {
            return Err("MT-32 partial pool size is invalid".to_owned());
        }
        if self.active_reverb_model >= self.reverb_models.len() {
            return Err("MT-32 active reverb index is invalid".to_owned());
        }
        let queue_length = self.midi_queue.ring_buffer.len();
        if queue_length == 0
            || !queue_length.is_power_of_two()
            || self.midi_queue.ring_buffer_mask as usize != queue_length - 1
            || self.midi_queue.start_position as usize >= queue_length
            || self.midi_queue.end_position as usize >= queue_length
        {
            return Err("MT-32 MIDI queue is invalid".to_owned());
        }
        if self.midi_stream_parser.stream_buffer.len() != MAX_STREAM_BUFFER_SIZE
            || self.midi_stream_parser.stream_buffer_size as usize
                > self.midi_stream_parser.stream_buffer.len()
        {
            return Err("MT-32 MIDI parser buffer is invalid".to_owned());
        }
        if self.midi_queue.sysex_buffer.len() != MAX_STREAM_BUFFER_SIZE
            || self.midi_queue.sysex_write_position as usize >= MAX_STREAM_BUFFER_SIZE
            || self.midi_queue.sysex_used as usize > MAX_STREAM_BUFFER_SIZE
            || self
                .midi_sysex_scratch
                .as_ref()
                .is_none_or(|scratch| scratch.len() != MAX_STREAM_BUFFER_SIZE)
        {
            return Err("MT-32 SysEx storage is invalid".to_owned());
        }
        let mut queued_sysex_bytes = 0usize;
        let mut event_position = self.midi_queue.start_position;
        while event_position != self.midi_queue.end_position {
            let event = &self.midi_queue.ring_buffer[event_position as usize];
            if event.is_sysex {
                if event.sysex_offset as usize >= MAX_STREAM_BUFFER_SIZE
                    || event.sysex_length as usize > MAX_STREAM_BUFFER_SIZE
                {
                    return Err("MT-32 queued SysEx range is invalid".to_owned());
                }
                queued_sysex_bytes += event.sysex_length as usize;
            }
            event_position = (event_position + 1) & self.midi_queue.ring_buffer_mask;
        }
        if queued_sysex_bytes != self.midi_queue.sysex_used as usize {
            return Err("MT-32 queued SysEx byte count is invalid".to_owned());
        }
        if self.renderer.tmp_non_reverb_left.len() < MAX_SAMPLES_PER_RUN
            || self.renderer.tmp_non_reverb_right.len() < MAX_SAMPLES_PER_RUN
            || self.renderer.tmp_reverb_dry_left.len() < MAX_SAMPLES_PER_RUN
            || self.renderer.tmp_reverb_dry_right.len() < MAX_SAMPLES_PER_RUN
            || self.renderer.tmp_reverb_wet_left.len() < MAX_SAMPLES_PER_RUN
            || self.renderer.tmp_reverb_wet_right.len() < MAX_SAMPLES_PER_RUN
            || self.renderer.tmp_partial_left.len() < MAX_SAMPLES_PER_RUN
            || self.renderer.tmp_partial_right.len() < MAX_SAMPLES_PER_RUN
        {
            return Err("MT-32 renderer buffer is invalid".to_owned());
        }
        for partial in &self.partials {
            if partial
                .pcm_wave_index
                .is_some_and(|index| index >= self.rom.pcm_waves.len())
                || partial
                    .poly_index
                    .is_some_and(|index| index >= self.polys.len())
                || partial
                    .pair_index
                    .is_some_and(|index| index >= self.partials.len())
                || partial
                    .rhythm_temp_index
                    .is_some_and(|index| index >= DRUM_CACHE_COUNT)
            {
                return Err("MT-32 partial reference is invalid".to_owned());
            }
        }
        for poly in &self.polys {
            if poly.part_index.is_some_and(|index| index >= MAX_PARTS)
                || poly
                    .next_index
                    .is_some_and(|index| index >= self.polys.len())
                || poly
                    .partial_indices
                    .iter()
                    .flatten()
                    .any(|&index| index >= self.partials.len())
            {
                return Err("MT-32 poly reference is invalid".to_owned());
            }
        }
        for model in &self.reverb_models {
            validate_reverb_model(model)?;
        }
        Ok(())
    }
}

fn validate_reverb_ring(ring: &ReverbRingBuffer) -> Result<(), String> {
    if ring.size as usize != ring.buffer.len() || (ring.size != 0 && ring.index >= ring.size) {
        return Err("MT-32 reverb ring is invalid".to_owned());
    }
    Ok(())
}

fn validate_reverb_model(model: &BReverbModelState) -> Result<(), String> {
    match model {
        BReverbModelState::Standard {
            allpasses,
            entrance_delay,
            combs,
            ..
        } => {
            for allpass in allpasses {
                validate_reverb_ring(&allpass.ring)?;
            }
            validate_reverb_ring(&entrance_delay.comb.ring)?;
            for comb in combs {
                validate_reverb_ring(&comb.ring)?;
            }
        }
        BReverbModelState::TapDelay { tap_delay_comb, .. } => {
            validate_reverb_ring(&tap_delay_comb.comb.ring)?;
        }
        BReverbModelState::Closed => {}
    }
    Ok(())
}

crate::impl_state_codec!(La32RampState {
    current,
    large_target,
    large_increment,
    descending,
    interrupt_countdown,
    interrupt_raised,
});

crate::impl_state_codec!(La32FloatWaveGeneratorState {
    active,
    sawtooth_waveform,
    resonance,
    pulse_width,
    pcm_wave_address_offset,
    pcm_wave_length,
    pcm_wave_looped,
    pcm_wave_interpolated,
    wave_pos,
    last_freq,
    pcm_position,
});

crate::impl_state_codec!(La32PairState {
    master,
    slave,
    ring_modulated,
    mixed,
    master_output_sample,
    slave_output_sample,
});

crate::impl_state_codec!(TvaState {
    playing,
    bias_amp_subtraction,
    velo_amp_subtraction,
    key_time_subtraction,
    target,
    phase,
});

crate::impl_state_codec!(TvfState {
    base_cutoff,
    key_time_subtraction,
    level_mult,
    target,
    phase,
});

crate::impl_state_codec!(TvpState {
    process_timer_increment,
    counter,
    time_elapsed,
    phase,
    base_pitch,
    target_pitch_offset_without_lfo,
    current_pitch_offset,
    lfo_pitch_offset,
    time_keyfollow_subtraction,
    pitch_offset_change_per_big_tick,
    target_pitch_offset_reached_big_tick,
    shifts,
    pitch,
});

crate::impl_state_codec!(PartialState {
    sample_num,
    left_pan_value,
    right_pan_value,
    owner_part,
    mix_type,
    structure_position,
    pcm_num,
    pcm_wave_index,
    pulse_width_val,
    poly_index,
    pair_index,
    rhythm_temp_index,
    tva,
    tvp,
    tvf,
    amp_ramp,
    cutoff_modifier_ramp,
    la32_pair,
    patch_cache,
    cache_backup,
    already_outputed,
    active,
});

crate::impl_state_codec!(PolyStateData {
    part_index,
    key,
    velocity,
    active_partial_count,
    sustain,
    state,
    partial_indices,
    next_index,
});

crate::impl_state_codec!(PartState {
    part_num,
    hold_pedal,
    active_partial_count,
    active_non_releasing_poly_count,
    patch_cache,
    active_polys_first,
    active_polys_last,
    name,
    current_instr,
    volume_override,
    modulation,
    expression,
    pitch_bend,
    nrpn,
    rpn,
    pitch_bender_range,
    is_rhythm,
    drum_cache,
});

crate::impl_state_codec!(PartialManagerState {
    num_reserved_partials_for_part,
    free_polys,
    inactive_partials,
    inactive_partial_count,
});

crate::impl_state_codec!(MidiEvent {
    is_sysex,
    sysex_offset,
    sysex_length,
    short_message_data,
    timestamp,
});

crate::impl_state_codec!(MidiEventQueueState {
    ring_buffer,
    ring_buffer_mask,
    start_position,
    end_position,
    sysex_buffer,
    sysex_write_position,
    sysex_used,
});

crate::impl_state_codec!(MidiStreamParserState {
    running_status,
    stream_buffer,
    stream_buffer_size,
});

crate::impl_state_codec!(CoarseLpfState {
    ring_buffer,
    ring_buffer_position,
});

crate::impl_state_codec!(AnalogState {
    left_channel_lpf,
    right_channel_lpf,
    synth_gain,
    reverb_gain,
    old_mt32_analog_lpf,
});

crate::impl_state_codec!(ReverbRingBuffer {
    buffer,
    size,
    index,
});

crate::impl_state_codec!(ReverbAllpassState { ring });

crate::impl_state_codec!(ReverbCombState {
    ring,
    filter_factor,
    feedback_factor,
});

crate::impl_state_codec!(ReverbTapDelayCombState { comb, out_l, out_r });

crate::impl_state_codec!(ReverbDelayWithLpfState { comb, amp });

impl save_state::StateEncode for BReverbModelState {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Self::Standard {
                allpasses,
                entrance_delay,
                combs,
                dry_amp,
                wet_level,
                mt32_compatible,
                mode,
                opened,
            } => {
                save_state::StateEncode::encode_state(&0u8, output);
                save_state::StateEncode::encode_state(allpasses, output);
                save_state::StateEncode::encode_state(entrance_delay, output);
                save_state::StateEncode::encode_state(combs, output);
                save_state::StateEncode::encode_state(dry_amp, output);
                save_state::StateEncode::encode_state(wet_level, output);
                save_state::StateEncode::encode_state(mt32_compatible, output);
                save_state::StateEncode::encode_state(mode, output);
                save_state::StateEncode::encode_state(opened, output);
            }
            Self::TapDelay {
                tap_delay_comb,
                dry_amp,
                wet_level,
                mt32_compatible,
                opened,
            } => {
                save_state::StateEncode::encode_state(&1u8, output);
                save_state::StateEncode::encode_state(tap_delay_comb, output);
                save_state::StateEncode::encode_state(dry_amp, output);
                save_state::StateEncode::encode_state(wet_level, output);
                save_state::StateEncode::encode_state(mt32_compatible, output);
                save_state::StateEncode::encode_state(opened, output);
            }
            Self::Closed => save_state::StateEncode::encode_state(&2u8, output),
        }
    }
}

impl save_state::StateDecode for BReverbModelState {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            0 => Ok(Self::Standard {
                allpasses: save_state::StateDecode::decode_state(decoder)?,
                entrance_delay: save_state::StateDecode::decode_state(decoder)?,
                combs: save_state::StateDecode::decode_state(decoder)?,
                dry_amp: save_state::StateDecode::decode_state(decoder)?,
                wet_level: save_state::StateDecode::decode_state(decoder)?,
                mt32_compatible: save_state::StateDecode::decode_state(decoder)?,
                mode: save_state::StateDecode::decode_state(decoder)?,
                opened: save_state::StateDecode::decode_state(decoder)?,
            }),
            1 => Ok(Self::TapDelay {
                tap_delay_comb: save_state::StateDecode::decode_state(decoder)?,
                dry_amp: save_state::StateDecode::decode_state(decoder)?,
                wet_level: save_state::StateDecode::decode_state(decoder)?,
                mt32_compatible: save_state::StateDecode::decode_state(decoder)?,
                opened: save_state::StateDecode::decode_state(decoder)?,
            }),
            2 => Ok(Self::Closed),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

crate::impl_state_codec!(RendererState {
    tmp_non_reverb_left,
    tmp_non_reverb_right,
    tmp_reverb_dry_left,
    tmp_reverb_dry_right,
    tmp_reverb_wet_left,
    tmp_reverb_wet_right,
    tmp_partial_left,
    tmp_partial_right,
});

crate::impl_state_codec!(MemoryRegionDescriptor {
    start_addr,
    entry_size,
    entries,
});

crate::impl_state_codec!(ExtensionsState {
    master_tune_pitch_delta,
    master_volume_override,
    nice_amp_ramp,
    nice_panning,
    nice_partial_mixing,
    chan_table,
    aborting_part_ix,
});

crate::impl_state_codec!(MemoryRegionDescriptors {
    patch_temp,
    rhythm_temp,
    timbre_temp,
    patches,
    timbres,
    system,
    reset,
});

crate::impl_state_codec!(MuntState {
    opened,
    activated,
    mt32_ram,
    mt32_default,
    parts,
    partials,
    partial_count,
    polys,
    partial_manager,
    reverb_models,
    active_reverb_model,
    reverb_overridden,
    midi_queue,
    last_received_midi_event_timestamp,
    rendered_sample_count,
    midi_stream_parser,
    midi_sysex_scratch,
    aborting_poly_index,
    analog,
    renderer,
    midi_delay_mode,
    dac_input_mode,
    output_gain,
    reverb_output_gain,
    reversed_stereo_enabled,
    extensions,
    memory_regions,
    prng_state,
} defaults {
    rom: ImmutableResource::new(RomData::default()),
    tables: ImmutableResource::new(Tables::new()),
});
