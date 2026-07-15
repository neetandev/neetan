use alloc::vec::Vec;

use crate::{
    helpers::{bit, bitfield, clamp},
    sys::YmfmTimerUpdate,
    tables::{attenuation_increment, attenuation_to_volume},
};

// "quiet" value, used to optimize when we can skip doing work
const EG_QUIET: u32 = 0x380;

save_state::runtime_state_enum! {
/// Current phase of an FM operator envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum EnvelopeState {
    Depress = 0,
    Attack = 1,
    Decay = 2,
    Sustain = 3,
    Release = 4,
    Reverb = 5,
}}

impl EnvelopeState {
    pub(crate) const STATES: usize = 6;
}

// Three different keyon sources; actual keyon is an OR over all of these.
save_state::runtime_state_enum! {
/// Source of an FM operator key-on request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum KeyonType {
    Normal = 0,
    Rhythm = 1,
    Csm = 2,
}}

// Data that is computed once at the start of clocking
// and remains static during subsequent sound generation.
save_state::runtime_state! {
/// Cached register-derived parameters for one FM operator.
#[derive(Clone)]
pub(crate) struct OpdataCache {
    pub(crate) phase_step: u32, // phase step, or PHASE_STEP_DYNAMIC if PM is active
    pub(crate) total_level: u32, // total level * 8 + KSL
    pub(crate) block_freq: u32, // raw block frequency value (used to compute phase_step)
    pub(crate) detune: i32,     // detuning value (used to compute phase_step)
    pub(crate) multiple: u32,   // multiple value (x.1, used to compute phase_step)
    pub(crate) eg_sustain: u32, // sustain level, shifted up to envelope values
    pub(crate) eg_rate: [u8; EnvelopeState::STATES], // envelope rate, including KSR
    pub(crate) eg_shift: u8,
    pub(crate) waveform_index: u32, // waveform index (OPL only; 0 for OPN)
}}

impl OpdataCache {
    // Set phase_step to this value to recalculate it each sample;
    // needed in the case of PM LFO changes.
    pub(crate) const PHASE_STEP_DYNAMIC: u32 = 1;
}

impl Default for OpdataCache {
    fn default() -> Self {
        Self {
            phase_step: 0,
            total_level: 0,
            block_freq: 0,
            detune: 0,
            multiple: 0,
            eg_sustain: 0,
            eg_rate: [0; EnvelopeState::STATES],
            eg_shift: 0,
            waveform_index: 0,
        }
    }
}

// Apply KSR to the raw ADSR rate, ignoring ksr if the
// raw value is 0, and clamping to 63.
pub(crate) fn effective_rate(rawrate: u32, ksr: u32) -> u32 {
    if rawrate == 0 {
        0
    } else {
        (rawrate + ksr).min(63)
    }
}

// Encode four operator numbers into a 32-bit value in the operator maps.
pub(crate) const fn operator_list(o1: u8, o2: u8, o3: u8, o4: u8) -> u32 {
    o1 as u32 | ((o2 as u32) << 8) | ((o3 as u32) << 16) | ((o4 as u32) << 24)
}

pub(crate) trait FmRegisters: Sized {
    const OUTPUTS: usize;
    const CHANNELS: usize;
    const ALL_CHANNELS: u32;
    const OPERATORS: usize;
    const DEFAULT_PRESCALE: u32;
    const EG_CLOCK_DIVIDER: u32;
    const CSM_TRIGGER_MASK: u32;
    const REG_MODE: u32;
    const EG_HAS_DEPRESS: bool;
    const EG_HAS_REVERB: bool;
    const EG_HAS_SSG: bool;
    const MODULATOR_DELAY: bool;
    const DYNAMIC_OPS: bool;

    const STATUS_TIMERA: u8;
    const STATUS_TIMERB: u8;
    const STATUS_BUSY: u8;
    const STATUS_IRQ: u8;

    const RHYTHM_CHANNEL: u32;

    fn new() -> Self;
    fn reset(&mut self);
    fn operator_map(&self, index: usize) -> u32;
    fn write(
        &mut self,
        index: u32,
        data: u8,
        keyon_channel: &mut u32,
        keyon_opmask: &mut u32,
    ) -> bool;

    fn channel_offset(chnum: u32) -> u32;
    fn operator_offset(opnum: u32) -> u32;

    fn op_ssg_eg_enable(&self, opoffs: u32) -> u32;
    fn op_ssg_eg_mode(&self, opoffs: u32) -> u32;
    fn op_lfo_am_enable(&self, opoffs: u32) -> u32;

    fn ch_output_any(&self, choffs: u32) -> u32;
    fn ch_output_0(&self, choffs: u32) -> u32;
    fn ch_output_1(&self, choffs: u32) -> u32;
    fn ch_output_2(&self, choffs: u32) -> u32;
    fn ch_output_3(&self, choffs: u32) -> u32;
    fn ch_feedback(&self, choffs: u32) -> u32;
    fn ch_algorithm(&self, choffs: u32) -> u32;

    fn noise_state(&self) -> u32;
    fn timer_a_value(&self) -> u32;
    fn timer_b_value(&self) -> u32;
    fn csm(&self) -> u32;
    fn reset_timer_a(&self) -> u32;
    fn reset_timer_b(&self) -> u32;
    fn enable_timer_a(&self) -> u32;
    fn enable_timer_b(&self) -> u32;
    fn load_timer_a(&self) -> u32;
    fn load_timer_b(&self) -> u32;

    fn cache_operator_data(&self, choffs: u32, opoffs: u32, cache: &mut OpdataCache);
    fn compute_phase_step(
        &self,
        choffs: u32,
        opoffs: u32,
        cache: &OpdataCache,
        lfo_raw_pm: i32,
    ) -> u32;
    fn clock_noise_and_lfo(&mut self) -> i32;
    fn lfo_am_offset(&self, choffs: u32) -> u32;

    fn waveform(&self, index: u32, phase: u32) -> u16;

    fn status_mask(&self) -> u8;
    fn irq_reset(&self) -> u32;

    fn noise_enable(&self) -> u32;
    fn rhythm_enable(&self) -> u32;
}

// An FM operator (or "slot" in FM parlance), which produces an
// output sine wave modulated by an envelope.
save_state::runtime_state! {
/// Authoritative phase and envelope state of one FM operator.
#[derive(Clone)]
pub(crate) struct FmOperator {
    choffs: u32,          // channel offset in registers
    opoffs: u32,          // operator offset in registers
    phase: u32,           // current phase value (10.10 format)
    env_attenuation: u16, // computed envelope attenuation (4.6 format)
    env_state: EnvelopeState,
    ssg_inverted: u8, // non-zero if the output should be inverted (bit 0)
    key_state: u8,    // current key state: on or off (bit 0)
    keyon_live: u8,   // live key on state (bit 0 = direct, bit 1 = rhythm, bit 2 = CSM)
    cache: OpdataCache,
}}

impl FmOperator {
    pub(crate) fn new(opoffs: u32) -> Self {
        Self {
            choffs: 0,
            opoffs,
            phase: 0,
            env_attenuation: 0x3FF,
            env_state: EnvelopeState::Release,
            ssg_inverted: 0,
            key_state: 0,
            keyon_live: 0,
            cache: OpdataCache::default(),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.phase = 0;
        self.env_attenuation = 0x3FF;
        self.env_state = EnvelopeState::Release;
        self.ssg_inverted = 0;
        self.key_state = 0;
        self.keyon_live = 0;
    }

    pub(crate) fn set_choffs(&mut self, choffs: u32) {
        self.choffs = choffs;
    }

    pub(crate) fn phase(&self) -> u32 {
        self.phase >> 10
    }

    pub(crate) fn prepare<R: FmRegisters>(&mut self, regs: &R) -> bool {
        regs.cache_operator_data(self.choffs, self.opoffs, &mut self.cache);

        let keyon_live = self.keyon_live;
        self.clock_keystate::<R>(if keyon_live != 0 { 1 } else { 0 }, regs);
        self.keyon_live &= !(1 << KeyonType::Csm as u32);

        // We're active until we're quiet after the release.
        let terminal_state = if R::EG_HAS_REVERB {
            EnvelopeState::Reverb
        } else {
            EnvelopeState::Release
        };
        self.env_state != terminal_state || self.env_attenuation < EG_QUIET as u16
    }

    pub(crate) fn clock<R: FmRegisters>(&mut self, env_counter: u32, lfo_raw_pm: i32, regs: &R) {
        // Clock the SSG-EG state (OPN/OPNA).
        if regs.op_ssg_eg_enable(self.opoffs) != 0 {
            self.clock_ssg_eg_state::<R>(regs);
        } else {
            self.ssg_inverted = 0;
        }

        // Clock the envelope if on an envelope cycle; env_counter is a x.2 value.
        if bitfield(env_counter, 0, 2) == 0 {
            self.clock_envelope::<R>(env_counter >> 2, regs);
        }

        self.clock_phase::<R>(lfo_raw_pm, regs);
    }

    // Compute the 14-bit signed volume of this operator, given a phase
    // modulation and an AM LFO offset.
    //
    // The low 10 bits of phase represents a full 2*PI period over
    // the full sin wave.
    pub(crate) fn compute_volume<R: FmRegisters>(
        &self,
        phase: u32,
        am_offset: u32,
        regs: &R,
    ) -> i32 {
        // early out if the envelope is effectively off
        if self.env_attenuation > EG_QUIET as u16 {
            return 0;
        }

        // get the absolute value of the sin, as attenuation, as a 4.8 fixed point value
        let sin_attenuation = regs.waveform(self.cache.waveform_index, phase) as u32;
        // get the attenuation from the envelope generator as a 4.6 value, shifted up to 4.8
        let env_attenuation = self.envelope_attenuation::<R>(am_offset, regs) << 2;
        // combine into a 5.8 value, then convert from attenuation to 13-bit linear volume
        let result =
            attenuation_to_volume((sin_attenuation & 0x7FFF).wrapping_add(env_attenuation)) as i32;

        // negate if in the negative part of the sin wave (sign bit gives 14 bits)
        if bitfield(sin_attenuation, 15, 1) != 0 {
            -result
        } else {
            result
        }
    }

    // Compute the 14-bit signed noise volume.
    // Application manual says the logarithmic transform is not applied here, so we
    // just use the raw envelope attenuation, inverted (since 0 attenuation should be
    // maximum), and shift it up from a 10-bit value to an 11-bit value.
    // QUESTION: is AM applied still?
    pub(crate) fn compute_noise_volume<R: FmRegisters>(&self, am_offset: u32, regs: &R) -> i32 {
        let result = ((self.envelope_attenuation::<R>(am_offset, regs) ^ 0x3FF) << 1) as i32;
        // negate based on the noise state
        if bit(regs.noise_state(), 0) != 0 {
            -result
        } else {
            result
        }
    }

    pub(crate) fn keyonoff(&mut self, on: u32, keyon_type: KeyonType) {
        self.keyon_live = (self.keyon_live & !(1 << keyon_type as u32))
            | ((bit(on, 0) as u8) << keyon_type as u32);
    }

    // Start the attack phase; called when a keyon happens or when an
    // SSG-EG cycle is complete and restarts.
    fn start_attack<R: FmRegisters>(&mut self, is_restart: bool, regs: &R) {
        if self.env_state == EnvelopeState::Attack {
            return;
        }
        self.env_state = EnvelopeState::Attack;

        // Generally not inverted at start, except if SSG-EG is enabled and
        // one of the inverted modes is specified; leave this alone on a
        // restart, as it is managed by the clock_ssg_eg_state() code.
        if R::EG_HAS_SSG && !is_restart {
            self.ssg_inverted = (regs.op_ssg_eg_enable(self.opoffs)
                & bit(regs.op_ssg_eg_mode(self.opoffs), 2)) as u8;
        }

        // Reset the phase when we start an attack due to a key on
        // (but not when due to an SSG-EG restart except in certain cases
        // managed directly by the SSG-EG code).
        if !is_restart {
            self.phase = 0;
        }

        // If the attack rate >= 62 then immediately go to max attenuation.
        if self.cache.eg_rate[EnvelopeState::Attack as usize] >= 62 {
            self.env_attenuation = 0;
        }
    }

    // Start the release phase; called when a keyoff happens.
    fn start_release<R: FmRegisters>(&mut self) {
        if self.env_state as u32 >= EnvelopeState::Release as u32 {
            return;
        }
        self.env_state = EnvelopeState::Release;

        // If attenuation is inverted due to SSG-EG, snap the inverted
        // attenuation as the starting point.
        if R::EG_HAS_SSG && self.ssg_inverted != 0 {
            self.env_attenuation = (0x200u16.wrapping_sub(self.env_attenuation)) & 0x3FF;
            self.ssg_inverted = 0;
        }
    }

    fn clock_keystate<R: FmRegisters>(&mut self, keystate: u32, regs: &R) {
        debug_assert!(keystate == 0 || keystate == 1);

        if (keystate ^ self.key_state as u32) != 0 {
            self.key_state = keystate as u8;

            if keystate != 0 {
                // OPLL has a DP ("depress"?) state to bring the volume
                // down before starting the attack.
                if R::EG_HAS_DEPRESS && self.env_attenuation < 0x200 {
                    self.env_state = EnvelopeState::Depress;
                } else {
                    self.start_attack::<R>(false, regs);
                }
            } else {
                self.start_release::<R>();
            }
        }
    }

    // Clock the SSG-EG state; should only be called if SSG-EG is enabled.
    fn clock_ssg_eg_state<R: FmRegisters>(&mut self, regs: &R) {
        // Work only happens once the attenuation crosses above 0x200.
        if bit(self.env_attenuation as u32, 9) == 0 {
            return;
        }

        // 8 SSG-EG modes:
        //    000: repeat normally
        //    001: run once, hold low
        //    010: repeat, alternating between inverted/non-inverted
        //    011: run once, hold high
        //    100: inverted repeat normally
        //    101: inverted run once, hold low
        //    110: inverted repeat, alternating between inverted/non-inverted
        //    111: inverted run once, hold high
        let mode = regs.op_ssg_eg_mode(self.opoffs);

        // Hold modes (1/3/5/7)
        if bit(mode, 0) != 0 {
            // Set the inverted flag to the end state (0 for modes 1/7, 1 for modes 3/5)
            self.ssg_inverted = (bit(mode, 2) ^ bit(mode, 1)) as u8;

            // If holding, force the attenuation to the expected value once we're
            // past the attack phase.
            if self.env_state != EnvelopeState::Attack {
                self.env_attenuation = if self.ssg_inverted != 0 { 0x200 } else { 0x3FF };
            }
        // Continuous modes (0/2/4/6)
        } else {
            // Toggle invert in alternating mode (even in attack state)
            self.ssg_inverted ^= bit(mode, 1) as u8;

            // Restart attack if in decay/sustain states
            if self.env_state == EnvelopeState::Decay || self.env_state == EnvelopeState::Sustain {
                self.start_attack::<R>(true, regs);
            }

            // Phase is reset to 0 in modes 0/4
            if bit(mode, 1) == 0 {
                self.phase = 0;
            }
        }

        // In all modes, once we hit release state, attenuation is forced to maximum.
        if self.env_state == EnvelopeState::Release {
            self.env_attenuation = 0x3FF;
        }
    }

    fn clock_envelope<R: FmRegisters>(&mut self, env_counter: u32, regs: &R) {
        // Handle attack->decay transitions.
        if self.env_state == EnvelopeState::Attack && self.env_attenuation == 0 {
            self.env_state = EnvelopeState::Decay;
        }

        // Handle decay->sustain transitions; it is important to do this immediately
        // after the attack->decay transition above in the event that the sustain level
        // is set to 0 (in which case we will skip right to sustain without doing any
        // decay); as an example where this can be heard, check the cymbals sound
        // in channel 0 of shinobi's test mode sound #5.
        if self.env_state == EnvelopeState::Decay
            && self.env_attenuation as u32 >= self.cache.eg_sustain
        {
            self.env_state = EnvelopeState::Sustain;
        }

        // Fetch the appropriate 6-bit rate value from the cache.
        let rate = self.cache.eg_rate[self.env_state as usize] as u32;
        // Compute the rate shift value; this is the shift needed to
        // apply to the env_counter such that it becomes a 5.11 fixed
        // point number.
        let rate_shift = rate >> 2;
        let env_counter = env_counter << rate_shift;

        // See if the fractional part is 0; if not, it's not time to clock.
        if bitfield(env_counter, 0, 11) != 0 {
            return;
        }

        // Determine the increment based on the non-fractional part of env_counter.
        let relevant_bits = bitfield(
            env_counter,
            if rate_shift <= 11 {
                11
            } else {
                rate_shift as i32
            },
            3,
        );
        let increment = attenuation_increment(rate, relevant_bits);

        // Attack is the only one that increases.
        if self.env_state == EnvelopeState::Attack {
            // Glitch means that attack rates of 62/63 don't increment if
            // changed after the initial key on (where they are handled
            // specially); nukeykt confirms this happens on OPM, OPN, OPL/OPLL
            // at least so assuming it is true for everyone.
            if rate < 62 {
                let not_att = !(self.env_attenuation as u32);
                self.env_attenuation = self
                    .env_attenuation
                    .wrapping_add(((not_att.wrapping_mul(increment)) >> 4) as u16);
            }
        } else {
            // Non-SSG-EG cases just apply the increment.
            if regs.op_ssg_eg_enable(self.opoffs) == 0 {
                self.env_attenuation = self.env_attenuation.wrapping_add(increment as u16);
            // SSG-EG only applies if less than mid-point, and then at 4x.
            } else if (self.env_attenuation as u32) < 0x200 {
                self.env_attenuation = self.env_attenuation.wrapping_add((4 * increment) as u16);
            }

            // Clamp the final attenuation.
            if self.env_attenuation as u32 >= 0x400 {
                self.env_attenuation = 0x3FF;
            }

            // Transition from depress to attack.
            if R::EG_HAS_DEPRESS
                && self.env_state == EnvelopeState::Depress
                && self.env_attenuation >= 0x200
            {
                self.start_attack::<R>(false, regs);
            }

            // Transition from release to reverb, should switch at -18dB.
            if R::EG_HAS_REVERB
                && self.env_state == EnvelopeState::Release
                && self.env_attenuation >= 0xC0
            {
                self.env_state = EnvelopeState::Reverb;
            }
        }
    }

    // Clock the 10.10 phase value; the OPN version of the logic has been
    // verified against the Nuked phase generator.
    fn clock_phase<R: FmRegisters>(&mut self, lfo_raw_pm: i32, regs: &R) {
        // Read from the cache, or recalculate if PM active.
        let mut phase_step = self.cache.phase_step;
        if phase_step == OpdataCache::PHASE_STEP_DYNAMIC {
            phase_step = regs.compute_phase_step(self.choffs, self.opoffs, &self.cache, lfo_raw_pm);
        }
        self.phase = self.phase.wrapping_add(phase_step);
    }

    // Return the effective attenuation of the envelope.
    fn envelope_attenuation<R: FmRegisters>(&self, am_offset: u32, regs: &R) -> u32 {
        let mut result = (self.env_attenuation >> self.cache.eg_shift) as u32;

        // Invert if necessary due to SSG-EG.
        if R::EG_HAS_SSG && self.ssg_inverted != 0 {
            result = (0x200u32.wrapping_sub(result)) & 0x3FF;
        }

        // Add in LFO AM modulation.
        if regs.op_lfo_am_enable(self.opoffs) != 0 {
            result = result.wrapping_add(am_offset);
        }

        // Add in total level and KSL from the cache.
        result = result.wrapping_add(self.cache.total_level);

        // Clamp to max.
        result.min(0x3FF)
    }
}

// An FM channel which combines the output of 2 or 4
// operators into a final result.
save_state::runtime_state! {
/// Authoritative routing and feedback state of one FM channel.
#[derive(Clone)]
pub(crate) struct FmChannel {
    choffs: u32,
    feedback: [i16; 2],     // feedback memory for operator 1
    feedback_in: i16,       // next input value for op 1 feedback (set in output)
    op: [Option<usize>; 4], // up to 4 operators
}}

impl FmChannel {
    pub(crate) fn new(choffs: u32) -> Self {
        Self {
            choffs,
            feedback: [0, 0],
            feedback_in: 0,
            op: [None; 4],
        }
    }

    pub(crate) fn reset(&mut self) {
        self.feedback[0] = 0;
        self.feedback[1] = 0;
        self.feedback_in = 0;
    }

    pub(crate) fn assign(
        &mut self,
        index: u32,
        opnum: Option<usize>,
        operators: &mut [FmOperator],
    ) {
        self.op[index as usize] = opnum;
        if let Some(op_idx) = opnum {
            operators[op_idx].set_choffs(self.choffs);
        }
    }

    pub(crate) fn keyonoff(
        &self,
        states: u32,
        keyon_type: KeyonType,
        _chnum: u32,
        operators: &mut [FmOperator],
    ) {
        for opnum in 0..4 {
            if let Some(op_idx) = self.op[opnum] {
                operators[op_idx].keyonoff(bit(states, opnum as i32), keyon_type);
            }
        }
    }

    pub(crate) fn prepare<R: FmRegisters>(
        &mut self,
        regs: &R,
        operators: &mut [FmOperator],
    ) -> bool {
        let mut active_mask = 0u32;
        for opnum in 0..4 {
            if let Some(op_idx) = self.op[opnum]
                && operators[op_idx].prepare::<R>(regs)
            {
                active_mask |= 1 << opnum;
            }
        }
        active_mask != 0
    }

    pub(crate) fn clock<R: FmRegisters>(
        &mut self,
        env_counter: u32,
        lfo_raw_pm: i32,
        operators: &mut [FmOperator],
        regs: &R,
    ) {
        self.feedback[0] = self.feedback[1];
        self.feedback[1] = self.feedback_in;

        for opnum in 0..4 {
            if let Some(op_idx) = self.op[opnum] {
                operators[op_idx].clock::<R>(env_counter, lfo_raw_pm, regs);
            }
        }
    }

    pub(crate) fn is4op<R: FmRegisters>(&self) -> bool {
        if R::DYNAMIC_OPS {
            return self.op[2].is_some();
        }
        R::OPERATORS / R::CHANNELS == 4
    }

    // Combine 2 operators according to the specified algorithm, returning a sum
    // according to the rshift and clipmax parameters.
    //
    // Algorithms for two-operator case:
    //    0: O1 -> O2 -> out
    //    1: (O1 + O2) -> out
    pub(crate) fn output_2op<R: FmRegisters>(
        &mut self,
        output: &mut [i32],
        rshift: u32,
        clipmax: i32,
        regs: &R,
        operators: &[FmOperator],
    ) {
        let op0_idx = self.op[0].unwrap();
        let op1_idx = self.op[1].unwrap();

        // AM amount is the same across all operators; compute it once.
        let am_offset = regs.lfo_am_offset(self.choffs);

        // Operator 1 has optional self-feedback.
        let mut opmod: i32 = 0;
        let feedback = regs.ch_feedback(self.choffs);
        if feedback != 0 {
            opmod = ((self.feedback[0] as i32) + (self.feedback[1] as i32)) >> (10 - feedback);
        }

        // Compute the 14-bit volume/value of operator 1 and update the feedback.
        let op1value = operators[op0_idx].compute_volume::<R>(
            operators[op0_idx].phase().wrapping_add(opmod as u32),
            am_offset,
            regs,
        );
        self.feedback_in = op1value as i16;

        // Now that the feedback has been computed, skip the rest if all volumes
        // are clear; no need to do all this work for nothing.
        if regs.ch_output_any(self.choffs) == 0 {
            return;
        }

        let result;
        if bit(regs.ch_algorithm(self.choffs), 0) == 0 {
            // Some OPL chips use the previous sample for modulation instead of
            // the current sample.
            opmod = if R::MODULATOR_DELAY {
                self.feedback[1] as i32
            } else {
                op1value
            } >> 1;
            result = operators[op1_idx].compute_volume::<R>(
                operators[op1_idx].phase().wrapping_add(opmod as u32),
                am_offset,
                regs,
            ) >> rshift;
        } else {
            let mut r = if R::MODULATOR_DELAY {
                self.feedback[1] as i32
            } else {
                op1value
            } >> rshift;
            r +=
                operators[op1_idx].compute_volume::<R>(operators[op1_idx].phase(), am_offset, regs)
                    >> rshift;
            let clipmin = -clipmax - 1;
            result = clamp(r, clipmin, clipmax);
        }

        self.add_to_output::<R>(self.choffs, output, result, regs);
    }

    // Combine 4 operators according to the specified algorithm, returning a sum
    // according to the rshift and clipmax parameters.
    //
    // OPM/OPN offer 8 different connection algorithms for 4 operators,
    // and OPL3 offers 4 more, which we designate here as 8-11.
    //
    // The operators are computed in order, with the inputs pulled from
    // an array of values (opout) that is populated as we go:
    //    0 = 0
    //    1 = O1
    //    2 = O2
    //    3 = O3
    //    4 = (O4)
    //    5 = O1+O2
    //    6 = O1+O3
    //    7 = O2+O3
    pub(crate) fn output_4op<R: FmRegisters>(
        &mut self,
        output: &mut [i32],
        rshift: u32,
        clipmax: i32,
        regs: &R,
        operators: &[FmOperator],
    ) {
        let op0_idx = self.op[0].unwrap();
        let op1_idx = self.op[1].unwrap();
        let op2_idx = self.op[2].unwrap();
        let op3_idx = self.op[3].unwrap();

        // AM amount is the same across all operators; compute it once.
        let am_offset = regs.lfo_am_offset(self.choffs);

        // Operator 1 has optional self-feedback.
        let mut opmod: i32 = 0;
        let feedback = regs.ch_feedback(self.choffs);
        if feedback != 0 {
            opmod = ((self.feedback[0] as i32) + (self.feedback[1] as i32)) >> (10 - feedback);
        }

        // Compute the 14-bit volume/value of operator 1 and update the feedback.
        let op1value = operators[op0_idx].compute_volume::<R>(
            operators[op0_idx].phase().wrapping_add(opmod as u32),
            am_offset,
            regs,
        );
        self.feedback_in = op1value as i16;

        // Now that the feedback has been computed, skip the rest if all volumes
        // are clear; no need to do all this work for nothing.
        if regs.ch_output_any(self.choffs) == 0 {
            return;
        }

        let algorithm_ops = S_ALGORITHM_OPS[regs.ch_algorithm(self.choffs) as usize];

        let mut opout = [0i16; 8];
        opout[0] = 0;
        opout[1] = op1value as i16;

        // Compute the 14-bit volume/value of operator 2.
        opmod = (opout[bitfield(algorithm_ops, 0, 1) as usize] as i32) >> 1;
        opout[2] = operators[op1_idx].compute_volume::<R>(
            operators[op1_idx].phase().wrapping_add(opmod as u32),
            am_offset,
            regs,
        ) as i16;
        opout[5] = opout[1].wrapping_add(opout[2]);

        // Compute the 14-bit volume/value of operator 3.
        opmod = (opout[bitfield(algorithm_ops, 1, 3) as usize] as i32) >> 1;
        opout[3] = operators[op2_idx].compute_volume::<R>(
            operators[op2_idx].phase().wrapping_add(opmod as u32),
            am_offset,
            regs,
        ) as i16;
        opout[6] = opout[1].wrapping_add(opout[3]);
        opout[7] = opout[2].wrapping_add(opout[3]);

        // Compute the 14-bit volume/value of operator 4; this could be a noise
        // value on the OPM; all algorithms consume OP4 output at a minimum.
        let mut result;
        if regs.noise_enable() != 0 && self.choffs == 7 {
            result = operators[op3_idx].compute_noise_volume::<R>(am_offset, regs);
        } else {
            opmod = (opout[bitfield(algorithm_ops, 4, 3) as usize] as i32) >> 1;
            result = operators[op3_idx].compute_volume::<R>(
                operators[op3_idx].phase().wrapping_add(opmod as u32),
                am_offset,
                regs,
            );
        }
        result >>= rshift;

        // Optionally add OP1, OP2, OP3.
        let clipmin = -clipmax - 1;
        if bit(algorithm_ops, 7) != 0 {
            result = clamp(result + ((opout[1] as i32) >> rshift), clipmin, clipmax);
        }
        if bit(algorithm_ops, 8) != 0 {
            result = clamp(result + ((opout[2] as i32) >> rshift), clipmin, clipmax);
        }
        if bit(algorithm_ops, 9) != 0 {
            result = clamp(result + ((opout[3] as i32) >> rshift), clipmin, clipmax);
        }

        self.add_to_output::<R>(self.choffs, output, result, regs);
    }

    // Bass Drum: this uses operators 12 and 15 (i.e., channel 6)
    // in an almost-normal way, except that if the algorithm is 1,
    // the first operator is ignored instead of added in.
    pub(crate) fn output_rhythm_ch6<R: FmRegisters>(
        &mut self,
        output: &mut [i32],
        rshift: u32,
        _clipmax: i32,
        regs: &R,
        operators: &[FmOperator],
    ) {
        let op0_idx = self.op[0].unwrap();
        let op1_idx = self.op[1].unwrap();

        let am_offset = regs.lfo_am_offset(self.choffs);

        let mut opmod: i32 = 0;
        let feedback = regs.ch_feedback(self.choffs);
        if feedback != 0 {
            opmod = ((self.feedback[0] as i32) + (self.feedback[1] as i32)) >> (10 - feedback);
        }

        let opout1 = operators[op0_idx].compute_volume::<R>(
            operators[op0_idx].phase().wrapping_add(opmod as u32),
            am_offset,
            regs,
        );
        self.feedback_in = opout1 as i16;

        opmod = if bit(regs.ch_algorithm(self.choffs), 0) != 0 {
            0
        } else {
            opout1 >> 1
        };
        let result = operators[op1_idx].compute_volume::<R>(
            operators[op1_idx].phase().wrapping_add(opmod as u32),
            am_offset,
            regs,
        ) >> rshift;

        self.add_to_output::<R>(self.choffs, output, result * 2, regs);
    }

    // High Hat and Snare Drum (channel 7 in rhythm mode).
    pub(crate) fn output_rhythm_ch7<R: FmRegisters>(
        &mut self,
        phase_select: u32,
        output: &mut [i32],
        rshift: u32,
        clipmax: i32,
        regs: &R,
        operators: &[FmOperator],
    ) {
        let op0_idx = self.op[0].unwrap();
        let op1_idx = self.op[1].unwrap();

        let am_offset = regs.lfo_am_offset(self.choffs);
        let noise_state = bit(regs.noise_state(), 0);

        // High Hat: this uses the envelope from operator 13 (channel 7),
        // and a combination of noise and the operator 13/17 phase select
        // to compute the phase.
        let phase = (phase_select << 9) | (0xD0 >> (2 * (noise_state ^ phase_select)));
        let mut result = operators[op0_idx].compute_volume::<R>(phase, am_offset, regs) >> rshift;

        // Snare Drum: this uses the envelope from operator 16 (channel 7),
        // and a combination of noise and operator 13 phase to pick a phase.
        let op13phase = operators[op0_idx].phase();
        let phase = (0x100 << bit(op13phase, 8)) ^ (noise_state << 8);
        result += operators[op1_idx].compute_volume::<R>(phase, am_offset, regs) >> rshift;
        result = clamp(result, -clipmax - 1, clipmax);

        self.add_to_output::<R>(self.choffs, output, result * 2, regs);
    }

    // Tom Tom and Top Cymbal (channel 8 in rhythm mode).
    pub(crate) fn output_rhythm_ch8<R: FmRegisters>(
        &mut self,
        phase_select: u32,
        output: &mut [i32],
        rshift: u32,
        clipmax: i32,
        regs: &R,
        operators: &[FmOperator],
    ) {
        let op0_idx = self.op[0].unwrap();
        let op1_idx = self.op[1].unwrap();

        let am_offset = regs.lfo_am_offset(self.choffs);

        // Tom Tom: this is just a single operator processed normally.
        let mut result =
            operators[op0_idx].compute_volume::<R>(operators[op0_idx].phase(), am_offset, regs)
                >> rshift;

        // Top Cymbal: this uses the envelope from operator 17 (channel 8),
        // and the operator 13/17 phase select to compute the phase.
        let phase = 0x100 | (phase_select << 9);
        result += operators[op1_idx].compute_volume::<R>(phase, am_offset, regs) >> rshift;
        result = clamp(result, -clipmax - 1, clipmax);

        self.add_to_output::<R>(self.choffs, output, result * 2, regs);
    }

    fn add_to_output<R: FmRegisters>(&self, choffs: u32, output: &mut [i32], value: i32, regs: &R) {
        if R::OUTPUTS == 1 || regs.ch_output_0(choffs) != 0 {
            output[0] += value;
        }
        if R::OUTPUTS >= 2 && regs.ch_output_1(choffs) != 0 {
            output[1 % R::OUTPUTS] += value;
        }
        if R::OUTPUTS >= 3 && regs.ch_output_2(choffs) != 0 {
            output[2 % R::OUTPUTS] += value;
        }
        if R::OUTPUTS >= 4 && regs.ch_output_3(choffs) != 0 {
            output[3 % R::OUTPUTS] += value;
        }
    }
}

// The s_algorithm_ops table describes the inputs and outputs of each
// algorithm as follows:
//
//      ---------x use opout[x] as operator 2 input
//      ------xxx- use opout[x] as operator 3 input
//      ---xxx---- use opout[x] as operator 4 input
//      --x------- include opout[1] in final sum
//      -x-------- include opout[2] in final sum
//      x--------- include opout[3] in final sum
const fn algorithm_encode(
    op2in: u16,
    op3in: u16,
    op4in: u16,
    op1out: u16,
    op2out: u16,
    op3out: u16,
) -> u32 {
    (op2in | (op3in << 1) | (op4in << 4) | (op1out << 7) | (op2out << 8) | (op3out << 9)) as u32
}

static S_ALGORITHM_OPS: [u32; 12] = [
    algorithm_encode(1, 2, 3, 0, 0, 0), //  0: O1 -> O2 -> O3 -> O4 -> out (O4)
    algorithm_encode(0, 5, 3, 0, 0, 0), //  1: (O1 + O2) -> O3 -> O4 -> out (O4)
    algorithm_encode(0, 2, 6, 0, 0, 0), //  2: (O1 + (O2->O3)) -> O4 -> out (O4)
    algorithm_encode(1, 0, 7, 0, 0, 0), //  3: ((O1->O2) + O3) -> O4 -> out (O4)
    algorithm_encode(1, 0, 3, 0, 1, 0), //  4: ((O1->O2) + (O3->O4)) -> out (O2+O4)
    algorithm_encode(1, 1, 1, 0, 1, 1), //  5: ((O1->O2) + (O1->O3) + (O1->O4)) -> out (O2+O3+O4)
    algorithm_encode(1, 0, 0, 0, 1, 1), //  6: ((O1->O2) + O3 + O4) -> out (O2+O3+O4)
    algorithm_encode(0, 0, 0, 1, 1, 1), //  7: (O1 + O2 + O3 + O4) -> out (O1+O2+O3+O4)
    algorithm_encode(1, 2, 3, 0, 0, 0), //  8: O1 -> O2 -> O3 -> O4 -> out (O4)         [same as 0]
    algorithm_encode(0, 2, 3, 1, 0, 0), //  9: (O1 + (O2->O3->O4)) -> out (O1+O4)   [unique]
    algorithm_encode(1, 0, 3, 0, 1, 0), // 10: ((O1->O2) + (O3->O4)) -> out (O2+O4) [same as 4]
    algorithm_encode(0, 2, 0, 1, 0, 1), // 11: (O1 + (O2->O3) + O4) -> out (O1+O3+O4) [unique]
];

// A set of operators and channels which together form a Yamaha FM core;
// chips that implement other engines (ADPCM, wavetable, etc) take this
// output and combine it with the others externally.
save_state::runtime_state! {
/// Complete authoritative state of a register-specialized FM engine.
#[derive(Clone)]
pub(crate) struct FmEngine<R> {
    pub(crate) regs: R,
    env_counter: u32, // envelope counter; low 2 bits are sub-counter
    status: u8,
    clock_prescale: u8, // prescale factor (2/3/6)
    irq_mask: u8,       // mask of which bits signal IRQs
    irq_state: u8,
    timer_running: [u8; 2],
    timer_updates: [Option<YmfmTimerUpdate>; 2],
    irq_update: Option<bool>,
    total_clocks: u8,       // low 8 bits of the total number of clocks processed
    active_channels: u32,   // mask of active channels (computed by prepare)
    modified_channels: u32, // mask of channels that have been modified
    prepare_count: u32,     // counter to do periodic prepare sweeps
    pub(crate) operators: Vec<FmOperator>,
    pub(crate) channels: Vec<FmChannel>,
}}

impl<R: FmRegisters> FmEngine<R> {
    pub(crate) fn new() -> Self {
        let mut operators = Vec::with_capacity(R::OPERATORS);
        for opnum in 0..R::OPERATORS {
            operators.push(FmOperator::new(R::operator_offset(opnum as u32)));
        }

        let mut channels = Vec::with_capacity(R::CHANNELS);
        for chnum in 0..R::CHANNELS {
            channels.push(FmChannel::new(R::channel_offset(chnum as u32)));
        }

        let mut engine = Self {
            regs: R::new(),
            env_counter: 0,
            status: 0,
            clock_prescale: R::DEFAULT_PRESCALE as u8,
            irq_mask: R::STATUS_TIMERA | R::STATUS_TIMERB,
            irq_state: 0,
            timer_running: [0, 0],
            timer_updates: [None, None],
            irq_update: None,
            total_clocks: 0,
            active_channels: R::ALL_CHANNELS,
            modified_channels: R::ALL_CHANNELS,
            prepare_count: 0,
            operators,
            channels,
        };

        engine.assign_operators();
        engine
    }

    pub(crate) fn reset(&mut self) {
        self.set_reset_status(0, 0xFF);
        self.regs.reset();
        // Explicitly write to the mode register since it has side-effects.
        // QUESTION: old cores initialize this to 0x30 -- who is right?
        self.write(R::REG_MODE as u16, 0);

        for chan in &mut self.channels {
            chan.reset();
        }
        for op in &mut self.operators {
            op.reset();
        }
    }

    pub(crate) fn clock(&mut self, chanmask: u32) -> u32 {
        self.total_clocks = self.total_clocks.wrapping_add(1);

        // If something was modified, prepare.
        // Also prepare every 4k samples to catch ending notes.
        if self.modified_channels != 0 || self.prepare_count >= 4096 {
            if R::DYNAMIC_OPS {
                self.assign_operators();
            }

            self.active_channels = 0;
            for chnum in 0..R::CHANNELS {
                if bit(chanmask, chnum as i32) != 0
                    && self.channels[chnum].prepare::<R>(&self.regs, &mut self.operators)
                {
                    self.active_channels |= 1 << chnum;
                }
            }

            self.modified_channels = 0;
            self.prepare_count = 0;
        } else {
            self.prepare_count += 1;
        }

        // If the envelope clock divider is 1, just increment by 4;
        // otherwise, increment by 1 and manually wrap when we reach the divide count.
        if R::EG_CLOCK_DIVIDER == 1 {
            self.env_counter = self.env_counter.wrapping_add(4);
        } else {
            self.env_counter = self.env_counter.wrapping_add(1);
            if bitfield(self.env_counter, 0, 2) == R::EG_CLOCK_DIVIDER {
                self.env_counter = self.env_counter.wrapping_add(4 - R::EG_CLOCK_DIVIDER);
            }
        }

        let lfo_raw_pm = self.regs.clock_noise_and_lfo();

        for chnum in 0..R::CHANNELS {
            if bit(chanmask, chnum as i32) != 0 {
                self.channels[chnum].clock::<R>(
                    self.env_counter,
                    lfo_raw_pm,
                    &mut self.operators,
                    &self.regs,
                );
            }
        }

        // Return the envelope counter as it is used to clock ADPCM-A.
        self.env_counter
    }

    pub(crate) fn output_mut(
        &mut self,
        output: &mut [i32],
        rshift: u32,
        clipmax: i32,
        chanmask: u32,
    ) {
        let chanmask = chanmask & self.active_channels;

        // Handle the rhythm case, where some of the operators are dedicated
        // to percussion (this is an OPL-specific feature).
        if self.regs.rhythm_enable() != 0 {
            // Precompute the operator 13+17 phase selection value.
            let op13phase = if self.operators.len() > 13 {
                self.operators[13].phase()
            } else {
                0
            };
            let op17phase = if self.operators.len() > 17 {
                self.operators[17].phase()
            } else {
                0
            };
            let phase_select = (bit(op13phase, 2) ^ bit(op13phase, 7))
                | bit(op13phase, 3)
                | (bit(op17phase, 5) ^ bit(op17phase, 3));

            for chnum in 0..R::CHANNELS {
                if bit(chanmask, chnum as i32) != 0 {
                    if chnum == 6 {
                        self.channels[chnum].output_rhythm_ch6::<R>(
                            output,
                            rshift,
                            clipmax,
                            &self.regs,
                            &self.operators,
                        );
                    } else if chnum == 7 {
                        self.channels[chnum].output_rhythm_ch7::<R>(
                            phase_select,
                            output,
                            rshift,
                            clipmax,
                            &self.regs,
                            &self.operators,
                        );
                    } else if chnum == 8 {
                        self.channels[chnum].output_rhythm_ch8::<R>(
                            phase_select,
                            output,
                            rshift,
                            clipmax,
                            &self.regs,
                            &self.operators,
                        );
                    } else if self.channels[chnum].is4op::<R>() {
                        self.channels[chnum].output_4op::<R>(
                            output,
                            rshift,
                            clipmax,
                            &self.regs,
                            &self.operators,
                        );
                    } else {
                        self.channels[chnum].output_2op::<R>(
                            output,
                            rshift,
                            clipmax,
                            &self.regs,
                            &self.operators,
                        );
                    }
                }
            }
        } else {
            for chnum in 0..R::CHANNELS {
                if bit(chanmask, chnum as i32) != 0 {
                    if self.channels[chnum].is4op::<R>() {
                        self.channels[chnum].output_4op::<R>(
                            output,
                            rshift,
                            clipmax,
                            &self.regs,
                            &self.operators,
                        );
                    } else {
                        self.channels[chnum].output_2op::<R>(
                            output,
                            rshift,
                            clipmax,
                            &self.regs,
                            &self.operators,
                        );
                    }
                }
            }
        }
    }

    pub(crate) fn write(&mut self, regnum: u16, data: u8) {
        if regnum as u32 == R::REG_MODE {
            self.engine_mode_write(data);
            return;
        }

        self.modified_channels = R::ALL_CHANNELS;

        let mut keyon_channel = 0u32;
        let mut keyon_opmask = 0u32;
        if self
            .regs
            .write(regnum as u32, data, &mut keyon_channel, &mut keyon_opmask)
        {
            if keyon_channel < R::CHANNELS as u32 {
                self.channels[keyon_channel as usize].keyonoff(
                    keyon_opmask,
                    KeyonType::Normal,
                    keyon_channel,
                    &mut self.operators,
                );
            } else if R::CHANNELS >= 9 && keyon_channel == R::RHYTHM_CHANNEL {
                self.channels[6].keyonoff(
                    if bit(keyon_opmask, 4) != 0 { 3 } else { 0 },
                    KeyonType::Rhythm,
                    6,
                    &mut self.operators,
                );
                self.channels[7].keyonoff(
                    bit(keyon_opmask, 0) | (bit(keyon_opmask, 3) << 1),
                    KeyonType::Rhythm,
                    7,
                    &mut self.operators,
                );
                self.channels[8].keyonoff(
                    bit(keyon_opmask, 2) | (bit(keyon_opmask, 1) << 1),
                    KeyonType::Rhythm,
                    8,
                    &mut self.operators,
                );
            }
        }
    }

    pub(crate) fn status(&self) -> u8 {
        self.status & !R::STATUS_BUSY & !self.regs.status_mask()
    }

    pub(crate) fn set_reset_status(&mut self, set: u8, reset: u8) -> u8 {
        self.status = (self.status | set) & !(reset | R::STATUS_BUSY);
        self.engine_check_interrupts();
        self.status & !self.regs.status_mask()
    }

    pub(crate) fn set_irq_mask(&mut self, mask: u8) {
        self.irq_mask = mask;
        self.engine_check_interrupts();
    }

    pub(crate) fn clock_prescale(&self) -> u32 {
        self.clock_prescale as u32
    }

    pub(crate) fn set_clock_prescale(&mut self, prescale: u32) {
        self.clock_prescale = prescale as u8;
    }

    pub(crate) fn assign_operators(&mut self) {
        for chnum in 0..R::CHANNELS {
            let map = self.regs.operator_map(chnum);
            for index in 0..4u32 {
                let opnum = bitfield(map, (8 * index) as i32, 8) as usize;
                let op_opt = if opnum == 0xFF { None } else { Some(opnum) };
                self.channels[chnum].assign(index, op_opt, &mut self.operators);
            }
        }
    }

    pub(crate) fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate> {
        self.timer_updates
            .get_mut(timer_id as usize)
            .and_then(Option::take)
    }

    pub(crate) fn take_irq_update(&mut self) -> Option<bool> {
        self.irq_update.take()
    }

    pub(crate) fn irq_asserted(&self) -> bool {
        self.irq_state != 0
    }

    pub(crate) fn update_timer(&mut self, tnum: u32, enable: u32, delta_clocks: i32) {
        let idx = tnum as usize;
        if enable != 0 && self.timer_running[idx] == 0 {
            let period = if tnum == 0 {
                (1024 - self.regs.timer_a_value()) as i32
            } else {
                (16 * (256 - self.regs.timer_b_value())) as i32
            };

            let period = (period + delta_clocks) as u32;

            self.timer_updates[idx] = Some(YmfmTimerUpdate::Schedule(
                period
                    .wrapping_mul(R::OPERATORS as u32)
                    .wrapping_mul(self.clock_prescale as u32),
            ));
            self.timer_running[idx] = 1;
        } else if enable == 0 {
            self.timer_updates[idx] = Some(YmfmTimerUpdate::Cancel);
            self.timer_running[idx] = 0;
        }
    }

    pub(crate) fn engine_timer_expired(&mut self, tnum: u32) {
        debug_assert!(tnum == 0 || tnum == 1);

        if tnum == 0 && self.regs.enable_timer_a() != 0 {
            self.set_reset_status(R::STATUS_TIMERA, 0);
        } else if tnum == 1 && self.regs.enable_timer_b() != 0 {
            self.set_reset_status(R::STATUS_TIMERB, 0);
        }

        // If timer A fired in CSM mode, trigger CSM on all relevant channels.
        if tnum == 0 && self.regs.csm() != 0 {
            for chnum in 0..R::CHANNELS {
                if bit(R::CSM_TRIGGER_MASK, chnum as i32) != 0 {
                    self.channels[chnum].keyonoff(
                        0xF,
                        KeyonType::Csm,
                        chnum as u32,
                        &mut self.operators,
                    );
                    self.modified_channels |= 1 << chnum;
                }
            }
        }

        self.timer_running[tnum as usize] = 0;
        self.update_timer(tnum, 1, 0);
    }

    pub(crate) fn engine_check_interrupts(&mut self) {
        let old_state = self.irq_state;
        self.irq_state = if (self.status & self.irq_mask & !self.regs.status_mask()) != 0 {
            1
        } else {
            0
        };

        if self.irq_state != 0 {
            self.status |= R::STATUS_IRQ;
        } else {
            self.status &= !R::STATUS_IRQ;
        }

        if old_state != self.irq_state {
            self.irq_update = Some(self.irq_state != 0);
        }
    }

    pub(crate) fn engine_mode_write(&mut self, data: u8) {
        self.modified_channels = R::ALL_CHANNELS;

        let mut dummy1 = 0u32;
        let mut dummy2 = 0u32;
        self.regs.write(R::REG_MODE, data, &mut dummy1, &mut dummy2);

        // Reset IRQ status -- when written, all other bits are ignored.
        // QUESTION: should this maybe just reset the IRQ bit and not all the bits?
        //   That is, check_interrupts would only set, this would only clear?
        if self.regs.irq_reset() != 0 {
            self.set_reset_status(0, 0x78);
        } else {
            let mut reset_mask: u8 = 0;
            if self.regs.reset_timer_b() != 0 {
                reset_mask |= R::STATUS_TIMERB;
            }
            if self.regs.reset_timer_a() != 0 {
                reset_mask |= R::STATUS_TIMERA;
            }
            self.set_reset_status(0, reset_mask);

            // Load timers; note that timer B gets a small negative adjustment because
            // the *16 multiplier is free-running, so the first tick of the clock
            // is a bit shorter.
            let delta = -((self.total_clocks & 15) as i32);
            self.update_timer(1, self.regs.load_timer_b(), delta);
            self.update_timer(0, self.regs.load_timer_a(), 0);
        }
    }
}
