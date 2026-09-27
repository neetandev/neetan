//! # ymfm_oxide
//!
//! Safe Rust reimplementation of [ymfm](https://github.com/aaronsgiles/ymfm) for Yamaha FM
//! synthesis chips.
//!
//! ## Ported chips
//!
//! Not all chips are ported. We only ported the chips that we use in our emulated machines.
//!
//! | Chip    | Family | Features                                  |
//! |---------|--------|-------------------------------------------|
//! | YM2203  | OPN    | 3-ch FM + 3-ch SSG                        |
//! | YM2608  | OPNA   | 6-ch stereo FM + SSG + ADPCM-A + ADPCM-B  |
//! | YM2610  | OPNB   | 4-ch stereo FM + SSG + ADPCM-A + ADPCM-B  |
//! | YM2610B | OPNB2  | 6-ch stereo FM + SSG + ADPCM-A + ADPCM-B  |
//! | YMF276  | OPN2   | 6-ch stereo FM + channel-6 DAC            |
//! | YM3526  | OPL    | 9-ch mono FM                              |
//! | Y8950   | OPL    | 9-ch mono FM + ADPCM-B                    |
//! | YM3812  | OPL2   | 9-ch mono FM, 4 waveforms                 |
//! | YMF262  | OPL3   | 18-ch 4-output FM, 8 waveforms, 4-op mode |
//! | YM2151  | OPM    | 8-ch stereo FM + noise + LFO              |
//! | YM2413  | OPLL   | 9-ch mono FM + rhythm                     |
//!
//! # Usage
//!
//! ```no_run
//! use ymfm_oxide::{Ym2203, YmfmOutput4};
//!
//! let mut chip = Ym2203::new();
//! chip.reset();
//!
//! // Write a register: first set the address, then write the data.
//! chip.write_address(0x28); // Key on/off register
//! chip.write_data(0xF0); // Key-on all operators, channel 0
//!
//! // Generate audio samples.
//! let mut output = [YmfmOutput4 { data: [0; 4] }; 128];
//! chip.generate(&mut output);
//! ```
//!
//! ## Signal updates
//!
//! Timer and IRQ outputs are exposed as pull-based, coalesced updates through
//! `take_timer_update()` and `take_irq_update()`. Callers that need to observe
//! every scheduling or IRQ edge should drain those updates after each operation
//! that can change chip signals, including `reset`, register writes,
//! `timer_expired`, and status reconciliation reads.
//!
//! ## License
//!
//! This project is licensed under [3-clause BSD](https://opensource.org/license/bsd-3-clause) license.

#![deny(missing_docs)]
#![deny(unsafe_code)]
#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::vec::Vec;

pub(crate) mod adpcm;
pub(crate) mod fm;
pub(crate) mod helpers;
pub(crate) mod opl;
pub(crate) mod opm;
pub(crate) mod opn;
pub(crate) mod opq;
pub(crate) mod opz;
pub(crate) mod ssg;
mod sys;
pub(crate) mod tables;

use adpcm::{AdpcmAEngine, AdpcmBChannel, AdpcmBEngine};
use fm::{FmEngine, FmRegisters};
use opl::{OPLL_INSTRUMENT_DATA_SIZE, Opl2Registers, Opl3Registers, OplRegisters, OpllRegisters};
use opm::OpmRegisters;
use opn::{OpnRegisters, OpnaRegisters, SsgResampler};
use opq::OpqRegisters;
use opz::OpzRegisters;
use ssg::{SsgEngine, SsgOutput};
pub use sys::{
    YmfmOpnFidelity, YmfmOutput1, YmfmOutput2, YmfmOutput3, YmfmOutput4, YmfmOutput6,
    YmfmTimerUpdate,
};

const YM2608_ADPCM_A_ROM_SIZE: usize = 8192;
static SILENT_ADPCM_MEMORY: [u8; 1] = [0];
/// Number of bytes in a YM2413 instrument table.
pub const YM2413_INSTRUMENT_DATA_SIZE: usize = OPLL_INSTRUMENT_DATA_SIZE;
/// Redistributable YMFM default YM2413 instrument table.
pub const YM2413_DEFAULT_INSTRUMENTS: [u8; YM2413_INSTRUMENT_DATA_SIZE] = [
    0x71, 0x61, 0x1E, 0x17, 0xEF, 0x7F, 0x00, 0x17, 0x13, 0x41, 0x1A, 0x0D, 0xF8, 0xF7, 0x23, 0x13,
    0x13, 0x01, 0x99, 0x00, 0xF2, 0xC4, 0x11, 0x23, 0x31, 0x61, 0x0E, 0x07, 0x98, 0x64, 0x70, 0x27,
    0x22, 0x21, 0x1E, 0x06, 0xBF, 0x76, 0x00, 0x28, 0x31, 0x22, 0x16, 0x05, 0xE0, 0x71, 0x0F, 0x18,
    0x21, 0x61, 0x1D, 0x07, 0x82, 0x8F, 0x10, 0x07, 0x23, 0x21, 0x2D, 0x14, 0xFF, 0x7F, 0x00, 0x07,
    0x41, 0x61, 0x1B, 0x06, 0x64, 0x65, 0x10, 0x17, 0x61, 0x61, 0x0B, 0x18, 0x85, 0xFF, 0x81, 0x07,
    0x13, 0x01, 0x83, 0x11, 0xFA, 0xE4, 0x10, 0x04, 0x17, 0x81, 0x23, 0x07, 0xF8, 0xF8, 0x22, 0x12,
    0x61, 0x50, 0x0C, 0x05, 0xF2, 0xF5, 0x29, 0x42, 0x01, 0x01, 0x54, 0x03, 0xC3, 0x92, 0x03, 0x02,
    0x41, 0x41, 0x89, 0x03, 0xF1, 0xE5, 0x11, 0x13, 0x01, 0x01, 0x18, 0x0F, 0xDF, 0xF8, 0x6A, 0x6D,
    0x01, 0x01, 0x00, 0x00, 0xC8, 0xD8, 0xA7, 0x48, 0x05, 0x01, 0x00, 0x00, 0xF8, 0xAA, 0x59, 0x55,
];

/// Redistributable YMFM default YM2423 (OPLL-X) instrument table.
pub const YM2423_DEFAULT_INSTRUMENTS: [u8; YM2413_INSTRUMENT_DATA_SIZE] = [
    0x61, 0x61, 0x1B, 0x07, 0x94, 0x5F, 0x10, 0x06, 0x93, 0xB1, 0x51, 0x04, 0xF3, 0xF2, 0x70, 0xFB,
    0x41, 0x21, 0x11, 0x85, 0xF2, 0xF2, 0x70, 0x75, 0x93, 0xB2, 0x28, 0x07, 0xF3, 0xF2, 0x70, 0xB4,
    0x72, 0x31, 0x97, 0x05, 0x51, 0x6F, 0x60, 0x09, 0x13, 0x30, 0x18, 0x06, 0xF7, 0xF4, 0x50, 0x85,
    0x51, 0x31, 0x1C, 0x07, 0x51, 0x71, 0x20, 0x26, 0x41, 0xF4, 0x1B, 0x07, 0x74, 0x34, 0x00, 0x06,
    0x50, 0x30, 0x4D, 0x03, 0x42, 0x65, 0x20, 0x06, 0x40, 0x20, 0x10, 0x85, 0xF3, 0xF5, 0x20, 0x04,
    0x61, 0x61, 0x1B, 0x07, 0xC5, 0x96, 0xF3, 0xF6, 0xF9, 0xF1, 0xDC, 0x00, 0xF5, 0xF3, 0x77, 0xF2,
    0x60, 0xA2, 0x91, 0x03, 0x94, 0xC1, 0xF7, 0xF7, 0x30, 0x30, 0x17, 0x06, 0xF3, 0xF1, 0xB7, 0xFC,
    0x31, 0x36, 0x0D, 0x05, 0xF2, 0xF4, 0x27, 0x9C, 0x01, 0x01, 0x18, 0x0F, 0xDF, 0xF8, 0x6A, 0x6D,
    0x01, 0x01, 0x00, 0x00, 0xC8, 0xD8, 0xA7, 0x48, 0x05, 0x01, 0x00, 0x00, 0xF8, 0xAA, 0x59, 0x55,
];
/// Redistributable YMFM default YMF281 (OPLLP) instrument table.
pub const YMF281_DEFAULT_INSTRUMENTS: [u8; YM2413_INSTRUMENT_DATA_SIZE] = [
    0x72, 0x21, 0x1A, 0x07, 0xF6, 0x64, 0x01, 0x16, 0x00, 0x10, 0x45, 0x00, 0xF6, 0x83, 0x73, 0x63,
    0x13, 0x01, 0x96, 0x00, 0xF1, 0xF4, 0x31, 0x23, 0x71, 0x21, 0x0B, 0x0F, 0xF9, 0x64, 0x70, 0x17,
    0x02, 0x21, 0x1E, 0x06, 0xF9, 0x76, 0x00, 0x28, 0x00, 0x61, 0x82, 0x0E, 0xF9, 0x61, 0x20, 0x27,
    0x21, 0x61, 0x1B, 0x07, 0x84, 0x8F, 0x10, 0x07, 0x37, 0x32, 0xCA, 0x02, 0x66, 0x64, 0x47, 0x29,
    0x41, 0x41, 0x07, 0x03, 0xF5, 0x70, 0x51, 0xF5, 0x36, 0x01, 0x5E, 0x07, 0xF2, 0xF3, 0xF7, 0xF7,
    0x00, 0x00, 0x18, 0x06, 0xC5, 0xF3, 0x20, 0xF2, 0x17, 0x81, 0x25, 0x07, 0xF7, 0xF3, 0x21, 0xF7,
    0x35, 0x64, 0x00, 0x00, 0xFF, 0xF3, 0x77, 0xF5, 0x11, 0x31, 0x00, 0x07, 0xDD, 0xF3, 0xFF, 0xFB,
    0x3A, 0x21, 0x00, 0x07, 0x95, 0x84, 0x0F, 0xF5, 0x01, 0x01, 0x18, 0x0F, 0xDF, 0xF8, 0x6A, 0x6D,
    0x01, 0x01, 0x00, 0x00, 0xC8, 0xD8, 0xA7, 0x48, 0x05, 0x01, 0x00, 0x00, 0xF8, 0xAA, 0x59, 0x55,
];
/// Redistributable YMFM default DS1001 (Konami VRC7) instrument table.
pub const DS1001_DEFAULT_INSTRUMENTS: [u8; YM2413_INSTRUMENT_DATA_SIZE] = [
    0x03, 0x21, 0x05, 0x06, 0xC8, 0x81, 0x42, 0x27, 0x13, 0x41, 0x14, 0x0D, 0xF8, 0xF7, 0x23, 0x12,
    0x31, 0x11, 0x08, 0x08, 0xFA, 0xC2, 0x28, 0x22, 0x31, 0x61, 0x0C, 0x07, 0xF8, 0x64, 0x60, 0x27,
    0x22, 0x21, 0x1E, 0x06, 0xFF, 0x76, 0x00, 0x28, 0x02, 0x01, 0x05, 0x00, 0xAC, 0xF2, 0x03, 0x02,
    0x21, 0x61, 0x1D, 0x07, 0x82, 0x8F, 0x10, 0x07, 0x23, 0x21, 0x22, 0x17, 0xFF, 0x73, 0x00, 0x17,
    0x15, 0x11, 0x25, 0x00, 0x41, 0x71, 0x00, 0xF1, 0x95, 0x01, 0x10, 0x0F, 0xB8, 0xAA, 0x50, 0x02,
    0x17, 0xC1, 0x5E, 0x07, 0xFA, 0xF8, 0x22, 0x12, 0x71, 0x23, 0x11, 0x06, 0x65, 0x74, 0x10, 0x16,
    0x01, 0x02, 0xD3, 0x05, 0xF3, 0x92, 0x83, 0xF2, 0x61, 0x63, 0x0C, 0x00, 0xA4, 0xFF, 0x30, 0x06,
    0x21, 0x62, 0x0D, 0x00, 0xA1, 0xFF, 0x50, 0x08, 0x01, 0x01, 0x18, 0x0F, 0xDF, 0xF8, 0x6A, 0x6D,
    0x01, 0x01, 0x00, 0x00, 0xC8, 0xD8, 0xA7, 0x48, 0x05, 0x01, 0x00, 0x00, 0xF8, 0xAA, 0x59, 0x55,
];

/// OPLL variant with the YM2413 instrument ROM.
pub const OPLL_VARIANT_YM2413: u8 = 0;
/// OPLL variant with the YM2423 (OPLL-X) instrument ROM.
pub const OPLL_VARIANT_YM2423: u8 = 1;
/// OPLL variant with the YMF281 (OPLLP) instrument ROM.
pub const OPLL_VARIANT_YMF281: u8 = 2;
/// OPLL variant with the DS1001 (Konami VRC7) instrument ROM.
pub const OPLL_VARIANT_DS1001: u8 = 3;

save_state::runtime_state! {
/// Yamaha OPLL family authoritative state and emulator.
///
/// The YM2413, YM2423, YMF281 and DS1001 share one engine and differ only in
/// their built-in instrument ROM.
#[derive(Clone)]
pub struct OpllFamily<const VARIANT: u8> {
    fm: FmEngine<OpllRegisters>,
    address: u8,
}}

/// Yamaha YM2413 (OPLL) emulator.
pub type Ym2413 = OpllFamily<OPLL_VARIANT_YM2413>;
/// Yamaha YM2423 (OPLL-X) emulator.
pub type Ym2423 = OpllFamily<OPLL_VARIANT_YM2423>;
/// Yamaha YMF281 (OPLLP) emulator.
pub type Ymf281 = OpllFamily<OPLL_VARIANT_YMF281>;
/// Yamaha DS1001 (Konami VRC7) emulator.
pub type Ds1001 = OpllFamily<OPLL_VARIANT_DS1001>;

impl<const VARIANT: u8> OpllFamily<VARIANT> {
    /// Default instrument table of this variant.
    const DEFAULT_INSTRUMENTS: [u8; YM2413_INSTRUMENT_DATA_SIZE] = match VARIANT {
        OPLL_VARIANT_YM2413 => YM2413_DEFAULT_INSTRUMENTS,
        OPLL_VARIANT_YM2423 => YM2423_DEFAULT_INSTRUMENTS,
        OPLL_VARIANT_YMF281 => YMF281_DEFAULT_INSTRUMENTS,
        _ => DS1001_DEFAULT_INSTRUMENTS,
    };

    /// Creates a chip with the redistributable default instrument table of
    /// its variant.
    pub fn new() -> Self {
        Self::new_with_instruments(Self::DEFAULT_INSTRUMENTS)
    }

    /// Creates a chip with the supplied 144-byte instrument table.
    pub fn new_with_instruments(instrument_data: [u8; YM2413_INSTRUMENT_DATA_SIZE]) -> Self {
        let mut fm: FmEngine<OpllRegisters> = FmEngine::new();
        fm.regs.set_instrument_data(&instrument_data);
        Self { fm, address: 0 }
    }

    /// Captures the complete chip state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores the complete chip state.
    pub fn restore_state(&mut self, state: Self) -> Result<(), save_state::StateValidationError> {
        save_state::restore_root(self, state, &())
    }

    /// Resets the FM engine. The instrument table and the address latch keep
    /// their values.
    pub fn reset(&mut self) {
        self.fm.reset();
    }

    /// Replaces the 144-byte instrument table and invalidates cached operators.
    pub fn set_instrument_data(&mut self, instrument_data: &[u8; YM2413_INSTRUMENT_DATA_SIZE]) {
        self.fm.regs.set_instrument_data(instrument_data);
        self.fm.invalidate_caches();
    }

    /// Returns the native output sample rate for `input_clock`.
    pub fn sample_rate(&self, input_clock: u32) -> u32 {
        input_clock / (OpllRegisters::OPERATORS as u32 * self.fm.clock_prescale())
    }

    /// Latches the register address for a subsequent data write.
    pub fn write_address(&mut self, data: u8) -> u32 {
        self.address = data;
        12
    }

    /// Writes a value to the latched register.
    pub fn write_data(&mut self, data: u8) -> u32 {
        self.fm.write(self.address as u16, data);
        84
    }

    /// Generates separate melodic and rhythm samples.
    pub fn generate(&mut self, output: &mut [YmfmOutput2]) {
        for sample in output {
            self.fm.clock(OpllRegisters::ALL_CHANNELS);
            sample.data = [0; 2];
            self.fm
                .output_mut(&mut sample.data, 5, 256, OpllRegisters::ALL_CHANNELS);
            sample.data[0] = sample.data[0] * 128 / 9;
            sample.data[1] = sample.data[1] * 128 / 9;
        }
    }
}

impl<const VARIANT: u8> Default for OpllFamily<VARIANT> {
    fn default() -> Self {
        Self::new()
    }
}

save_state::runtime_state! {
/// Yamaha YM2203 authoritative state and emulator.
#[derive(Clone)]
pub struct Ym2203 {
    fm: FmEngine<OpnRegisters>,
    ssg: SsgEngine,
    ssg_resampler: SsgResampler,
    fidelity: YmfmOpnFidelity,
    address: u8,
    fm_samples_per_output: u32,
    last_fm: [i32; 1],
    io_input: [u8; 2],
}}

impl Ym2203 {
    /// Creates a new YM2203 instance.
    ///
    /// The chip is not automatically reset; call [`reset`](Self::reset)
    /// before first use.
    pub fn new() -> Self {
        let fm = FmEngine::new();
        let prescale = fm.clock_prescale();
        let mut chip = Self {
            fm,
            ssg: SsgEngine::new(),
            ssg_resampler: SsgResampler::new(false, 1),
            fidelity: YmfmOpnFidelity::Max,
            address: 0,
            fm_samples_per_output: 0,
            last_fm: [0],
            io_input: [0; 2],
        };
        chip.update_prescale(prescale);
        chip
    }

    /// Captures the complete chip state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores the complete chip state.
    pub fn restore_state(&mut self, state: Self) -> Result<(), save_state::StateValidationError> {
        save_state::restore_root(self, state, &())
    }

    /// Sets the value read back from an SSG parallel I/O port when that port is
    /// configured as an input. Port 0 is A, port 1 is B.
    pub fn set_io_input(&mut self, port: u8, value: u8) {
        if let Some(input) = self.io_input.get_mut(port as usize) {
            *input = value;
        }
    }

    /// Resets the chip to its initial power-on state.
    pub fn reset(&mut self) {
        self.fm.reset();
        self.ssg.reset();
    }

    /// Sets the output fidelity level, which controls the internal sample
    /// rate relative to the input clock.
    pub fn set_fidelity(&mut self, fidelity: YmfmOpnFidelity) {
        self.fidelity = fidelity;
        let prescale = self.fm.clock_prescale();
        self.update_prescale(prescale);
    }

    /// Returns the output sample rate in Hz for the given `input_clock` in Hz.
    ///
    /// The result depends on the current fidelity setting. For example, at
    /// [`YmfmOpnFidelity::Max`] the output rate is `input_clock / 4`.
    pub fn sample_rate(&mut self, input_clock: u32) -> u32 {
        // Fidelity controls the output sample rate divisor:
        //   Min = clock/24, Med = clock/12, Max = clock/4
        match self.fidelity {
            YmfmOpnFidelity::Min => input_clock / 24,
            YmfmOpnFidelity::Med => input_clock / 12,
            YmfmOpnFidelity::Max => input_clock / 4,
        }
    }

    /// Reads the chip status register.
    ///
    /// Bit 0 = Timer A flag, bit 1 = Timer B flag, bit 7 = busy flag.
    pub fn read_status(&mut self, busy: bool) -> u8 {
        let mut result = self.fm.status();
        if busy {
            result |= OpnRegisters::STATUS_BUSY;
        }
        result
    }

    /// Reads back data from the currently addressed register.
    ///
    /// Only meaningful for SSG registers (0x00-0x0F); FM registers are
    /// write-only.
    pub fn read_data(&mut self) -> u8 {
        if self.address < 0x10 {
            let register = self.address as u32 & 0x0F;
            // Port A/B read back the external input when configured as inputs
            // (register 0x07 bits 6/7 clear).
            if register == 0x0E && self.ssg.read(0x07) & 0x40 == 0 {
                return self.io_input[0];
            }
            if register == 0x0F && self.ssg.read(0x07) & 0x80 == 0 {
                return self.io_input[1];
            }
            self.ssg.read(register)
        } else {
            0
        }
    }

    /// Latches the register address for a subsequent
    /// [`write_data`](Self::write_data) or [`read_data`](Self::read_data).
    pub fn write_address(&mut self, data: u8) -> u32 {
        self.address = data;

        // 2D-2F: prescaler select
        // Writing 0x2D sets prescale to 6 (default).
        // Writing 0x2E halves it to 3 (only if currently 6).
        // Writing 0x2F sets prescale to 2.
        if self.address >= 0x2D && self.address <= 0x2F {
            if self.address == 0x2D {
                self.update_prescale(6);
            } else if self.address == 0x2E && self.fm.clock_prescale() == 6 {
                self.update_prescale(3);
            } else if self.address == 0x2F {
                self.update_prescale(2);
            }
        }
        0
    }

    /// Writes a value to the previously addressed register.
    pub fn write_data(&mut self, data: u8) -> u32 {
        // 00-0F: write to SSG
        if self.address < 0x10 {
            self.ssg.write(self.address as u32 & 0x0F, data);
        } else {
            // 10-FF: write to FM
            self.fm.write(self.address as u16, data);
        }

        32 * self.fm.clock_prescale()
    }

    /// Generates audio samples into `output`.
    ///
    /// Fills `output.len()` samples, each containing four channels:
    /// `[FM, SSG-A, SSG-B, SSG-C]`.
    pub fn generate(&mut self, output: &mut [YmfmOutput4]) {
        let numsamples = output.len();
        let sampindex = self.ssg_resampler.sampindex();

        // FM output is just repeated the prescale number of times;
        // fm_samples_per_output == 0 is a special 1.5:1 case.
        if self.fm_samples_per_output != 0 {
            for (samp, out) in output.iter_mut().enumerate() {
                if (sampindex + samp as u32).is_multiple_of(self.fm_samples_per_output) {
                    self.clock_fm();
                }
                out.data[0] = self.last_fm[0];
            }
        } else {
            // 1.5:1 ratio: clock FM on steps 0 and 1 of every 3, averaging
            // the two results on step 1 to approximate the half-sample offset.
            for (samp, out) in output.iter_mut().enumerate() {
                let step = (sampindex + samp as u32) % 3;
                if step == 0 {
                    self.clock_fm();
                }
                out.data[0] = self.last_fm[0];
                if step == 1 {
                    self.clock_fm();
                    out.data[0] = (out.data[0] + self.last_fm[0]) / 2;
                }
            }
        }

        // SAFETY: YmfmOutput4 is #[repr(C)] with a single [i32; 4] field,
        // so &mut [YmfmOutput4] has the same layout as &mut [[i32; 4]].
        #[allow(unsafe_code)]
        let output_nested = unsafe { &mut *(output as *mut [YmfmOutput4] as *mut [[i32; 4]]) };
        const _: () = assert!(size_of::<YmfmOutput4>() == size_of::<[i32; 4]>());

        let output_flat = output_nested.as_flattened_mut();
        self.ssg_resampler
            .resample(&mut self.ssg, output_flat, numsamples);
    }

    /// Notifies the chip that the specified timer has expired.
    ///
    /// The emulator should call this when the duration previously reported
    /// by [`take_timer_update`](Self::take_timer_update) has elapsed.
    /// `timer_id` is 0 (Timer A) or 1 (Timer B).
    pub fn timer_expired(&mut self, timer_id: u32) {
        self.fm.engine_timer_expired(timer_id);
    }

    /// Returns and clears the pending update for a timer.
    pub fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate> {
        self.fm.take_timer_update(timer_id)
    }

    /// Returns and clears the pending IRQ output update.
    pub fn take_irq_update(&mut self) -> Option<bool> {
        self.fm.take_irq_update()
    }

    /// Returns whether the chip IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.fm.irq_asserted()
    }

    fn clock_fm(&mut self) {
        self.fm.clock(OpnRegisters::ALL_CHANNELS);

        // Update the FM content; OPN is full 14-bit with no intermediate clipping.
        self.last_fm = [0];
        self.fm
            .output_mut(&mut self.last_fm, 0, 32767, OpnRegisters::ALL_CHANNELS);

        // Convert to 10.3 floating point value for the DAC and back.
        self.last_fm[0] = helpers::roundtrip_fp(self.last_fm[0]) as i32;
    }

    fn update_prescale(&mut self, prescale: u32) {
        self.fm.set_clock_prescale(prescale);

        // Fidelity:   ---- minimum ----    ---- medium -----    ---- maximum-----
        //              rate = clock/24      rate = clock/12      rate = clock/4
        // Prescale    FM rate  SSG rate    FM rate  SSG rate    FM rate  SSG rate
        //     6          3:1     2:3          6:1     4:3         18:1     4:1
        //     3        1.5:1     1:3          3:1     2:3          9:1     2:1
        //     2          1:1     1:6          2:1     1:3          6:1     1:1
        match self.fidelity {
            YmfmOpnFidelity::Min => match prescale {
                3 => {
                    self.fm_samples_per_output = 0; // 1.5:1
                    self.ssg_resampler.configure(1, 3);
                }
                2 => {
                    self.fm_samples_per_output = 1;
                    self.ssg_resampler.configure(1, 6);
                }
                _ => {
                    self.fm_samples_per_output = 3;
                    self.ssg_resampler.configure(2, 3);
                }
            },
            YmfmOpnFidelity::Med => match prescale {
                3 => {
                    self.fm_samples_per_output = 3;
                    self.ssg_resampler.configure(2, 3);
                }
                2 => {
                    self.fm_samples_per_output = 2;
                    self.ssg_resampler.configure(1, 3);
                }
                _ => {
                    self.fm_samples_per_output = 6;
                    self.ssg_resampler.configure(4, 3);
                }
            },
            YmfmOpnFidelity::Max => match prescale {
                3 => {
                    self.fm_samples_per_output = 9;
                    self.ssg_resampler.configure(2, 1);
                }
                2 => {
                    self.fm_samples_per_output = 6;
                    self.ssg_resampler.configure(1, 1);
                }
                _ => {
                    self.fm_samples_per_output = 18;
                    self.ssg_resampler.configure(4, 1);
                }
            },
        }
    }
}

const STATUS_ADPCM_B_EOS: u8 = 0x04;
const STATUS_ADPCM_B_BRDY: u8 = 0x08;
const STATUS_ADPCM_B_PLAYING: u8 = 0x20;

save_state::runtime_state! {
/// Complete mutable state of a YM2608 chip.
#[derive(Clone)]
pub struct Ym2608State {
    fm: FmEngine<OpnaRegisters>,
    ssg: SsgEngine,
    ssg_resampler: SsgResampler,
    adpcm_a: AdpcmAEngine,
    adpcm_b: AdpcmBEngine,
    adpcm_b_ram: Option<Vec<u8>>,
    fidelity: YmfmOpnFidelity,
    address: u16,
    fm_samples_per_output: u32,
    last_fm: [i32; 2],
    irq_enable: u8,
    flag_control: u8,
    adpcm_a_rom_identity: save_state::ResourceIdentity,
}}

/// Yamaha YM2608 (OPNA) emulator.
///
/// The YM2608 adds stereo FM (6 channels), ADPCM-A rhythm, and ADPCM-B sample
/// playback over the YM2203.
#[derive(Clone)]
pub struct Ym2608 {
    fm: FmEngine<OpnaRegisters>,
    ssg: SsgEngine,
    ssg_resampler: SsgResampler,
    adpcm_a: AdpcmAEngine,
    adpcm_b: AdpcmBEngine,
    adpcm_a_rom: Vec<u8>,
    adpcm_b_ram: Option<Vec<u8>>,
    fidelity: YmfmOpnFidelity,
    address: u16,
    fm_samples_per_output: u32,
    last_fm: [i32; 2],
    irq_enable: u8,
    flag_control: u8,
}

impl Ym2608 {
    /// Creates a new YM2608 instance.
    pub fn new() -> Self {
        let fm = FmEngine::new();
        let prescale = fm.clock_prescale();
        let mut chip = Self {
            fm,
            ssg: SsgEngine::new(),
            ssg_resampler: SsgResampler::new(true, 2),
            adpcm_a: AdpcmAEngine::new(0),
            adpcm_b: AdpcmBEngine::new(0),
            adpcm_a_rom: SILENT_ADPCM_MEMORY.to_vec(),
            adpcm_b_ram: None,
            fidelity: YmfmOpnFidelity::Max,
            address: 0,
            fm_samples_per_output: 0,
            last_fm: [0, 0],
            irq_enable: 0x1F,
            flag_control: 0x1C,
        };
        chip.update_prescale(prescale);
        chip
    }

    /// Captures mutable chip state and the retained rhythm ROM identity.
    pub fn capture_state(&self) -> Ym2608State {
        Ym2608State {
            fm: self.fm.clone(),
            ssg: self.ssg.clone(),
            ssg_resampler: self.ssg_resampler.clone(),
            adpcm_a: self.adpcm_a.clone(),
            adpcm_b: self.adpcm_b.clone(),
            adpcm_b_ram: self.adpcm_b_ram.clone(),
            fidelity: self.fidelity,
            address: self.address,
            fm_samples_per_output: self.fm_samples_per_output,
            last_fm: self.last_fm,
            irq_enable: self.irq_enable,
            flag_control: self.flag_control,
            adpcm_a_rom_identity: save_state::ResourceIdentity::from_bytes(&self.adpcm_a_rom),
        }
    }

    /// Restores mutable state while retaining the rhythm ROM.
    pub fn restore_state(
        &mut self,
        state: Ym2608State,
    ) -> Result<(), save_state::StateValidationError> {
        let identity = save_state::ResourceIdentity::from_bytes(&self.adpcm_a_rom);
        save_state::restore_root(self, state, &identity)
    }

    /// Resets the chip to its initial power-on state.
    pub fn reset(&mut self) {
        self.fm.reset();
        self.ssg.reset();
        self.adpcm_a.reset();
        self.adpcm_b.reset();

        // Configure ADPCM percussion sounds; these are present in an embedded ROM.
        self.adpcm_a.set_start_end(0, 0x0000, 0x01BF); // bass drum
        self.adpcm_a.set_start_end(1, 0x01C0, 0x043F); // snare drum
        self.adpcm_a.set_start_end(2, 0x0440, 0x1B7F); // top cymbal
        self.adpcm_a.set_start_end(3, 0x1B80, 0x1CFF); // high hat
        self.adpcm_a.set_start_end(4, 0x1D00, 0x1F7F); // tom tom
        self.adpcm_a.set_start_end(5, 0x1F80, 0x1FFF); // rim shot

        // Initialize our special interrupt states, then read the upper status
        // register, which updates the IRQs.
        self.irq_enable = 0x1F;
        self.flag_control = 0x1C;
        self.read_status_hi(false);
    }

    /// Copies ADPCM-A rhythm ROM data into the chip.
    ///
    /// Panics if `data` is empty. Short data is zero-padded to the YM2608
    /// rhythm ROM size, and oversized data is truncated.
    pub fn set_adpcm_a_rom(&mut self, data: &[u8]) {
        assert!(!data.is_empty(), "ADPCM-A ROM data must not be empty");
        self.adpcm_a_rom.clear();
        self.adpcm_a_rom.resize(YM2608_ADPCM_A_ROM_SIZE, 0);
        let length = data.len().min(YM2608_ADPCM_A_ROM_SIZE);
        self.adpcm_a_rom[..length].copy_from_slice(&data[..length]);
    }

    /// Clears ADPCM-A rhythm ROM data. Reads from missing ROM data return zero.
    pub fn clear_adpcm_a_rom(&mut self) {
        self.adpcm_a_rom.clear();
        self.adpcm_a_rom.extend_from_slice(&SILENT_ADPCM_MEMORY);
    }

    /// Replaces ADPCM-B RAM with `data`.
    ///
    /// Panics if `data` is empty. Use [`clear_adpcm_b_ram`](Self::clear_adpcm_b_ram)
    /// to remove ADPCM-B RAM.
    pub fn set_adpcm_b_ram(&mut self, data: Vec<u8>) {
        assert!(!data.is_empty(), "ADPCM-B RAM must not be empty");
        self.adpcm_b_ram = Some(data);
    }

    /// Removes ADPCM-B RAM. Reads return zero and writes are ignored.
    pub fn clear_adpcm_b_ram(&mut self) {
        self.adpcm_b_ram = None;
    }

    /// Returns ADPCM-B RAM when present.
    pub fn adpcm_b_ram(&self) -> Option<&[u8]> {
        self.adpcm_b_ram.as_deref()
    }

    /// Returns mutable ADPCM-B RAM when present.
    pub fn adpcm_b_ram_mut(&mut self) -> Option<&mut [u8]> {
        self.adpcm_b_ram.as_deref_mut()
    }

    /// Sets the output fidelity level.
    pub fn set_fidelity(&mut self, fidelity: YmfmOpnFidelity) {
        self.fidelity = fidelity;
        let prescale = self.fm.clock_prescale();
        self.update_prescale(prescale);
    }

    /// Returns the output sample rate in Hz for the given `input_clock` in Hz.
    pub fn sample_rate(&mut self, input_clock: u32) -> u32 {
        // Fidelity controls the output sample rate divisor:
        //   Min = clock/48, Med = clock/24, Max = clock/8
        match self.fidelity {
            YmfmOpnFidelity::Min => input_clock / 48,
            YmfmOpnFidelity::Med => input_clock / 24,
            YmfmOpnFidelity::Max => input_clock / 8,
        }
    }

    /// Reads the chip status register (low).
    pub fn read_status(&mut self, busy: bool) -> u8 {
        let mut result =
            self.fm.status() & (OpnaRegisters::STATUS_TIMERA | OpnaRegisters::STATUS_TIMERB);
        if busy {
            result |= OpnaRegisters::STATUS_BUSY;
        }
        result
    }

    /// Reads data from the currently addressed register (low bank).
    pub fn read_data(&mut self) -> u8 {
        if self.address < 0x10 {
            // 00-0F: Read from SSG
            self.ssg.read(self.address as u32 & 0x0F)
        } else if self.address == 0xFF {
            // FF: ID code (1 = YM2608)
            1
        } else {
            0
        }
    }

    /// Reads the chip status register (high - ADPCM flags).
    pub fn read_status_hi(&mut self, busy: bool) -> u8 {
        // Fetch regular status, masking out the ADPCM-B bits we'll re-derive.
        let mut status =
            self.fm.status() & !(STATUS_ADPCM_B_EOS | STATUS_ADPCM_B_BRDY | STATUS_ADPCM_B_PLAYING);

        // Fetch ADPCM-B status, and merge in the bits.
        let adpcm_status = self.adpcm_b.status() as u32;
        if adpcm_status & AdpcmBChannel::STATUS_EOS != 0 {
            status |= STATUS_ADPCM_B_EOS;
        }
        if adpcm_status & AdpcmBChannel::STATUS_BRDY != 0 {
            status |= STATUS_ADPCM_B_BRDY;
        }
        if adpcm_status & AdpcmBChannel::STATUS_PLAYING != 0 {
            status |= STATUS_ADPCM_B_PLAYING;
        }

        // Turn off any bits that have been requested to be masked.
        status &= !(self.flag_control & 0x1F);

        // Update the status so that IRQs are propagated.
        self.fm.set_reset_status(status, !status);

        // Merge in the busy flag.
        if busy {
            status |= OpnaRegisters::STATUS_BUSY;
        }
        status
    }

    /// Reads data from the currently addressed register (high bank).
    pub fn read_data_hi(&mut self) -> u8 {
        if (self.address & 0xFF) < 0x10 {
            // 00-0F: Read from ADPCM-B
            self.adpcm_b
                .read(self.address as u32 & 0x0F, self.adpcm_b_ram.as_deref_mut()) as u8
        } else {
            0
        }
    }

    /// Latches the register address for the low bank.
    pub fn write_address(&mut self, data: u8) -> u32 {
        self.address = data as u16;

        // 2D-2F: prescaler select
        if (0x2D..=0x2F).contains(&data) {
            if data == 0x2D {
                self.update_prescale(6);
            } else if data == 0x2E && self.fm.clock_prescale() == 6 {
                self.update_prescale(3);
            } else if data == 0x2F {
                self.update_prescale(2);
            }
        }
        0
    }

    /// Writes a value to the previously addressed register (low bank).
    pub fn write_data(&mut self, data: u8) -> u32 {
        // Ignore if paired with upper address (port 1 data to port 0).
        if helpers::bit(self.address as u32, 8) != 0 {
            return 0;
        }

        if self.address < 0x10 {
            // 00-0F: write to SSG
            self.ssg.write(self.address as u32 & 0x0F, data);
        } else if self.address < 0x20 {
            // 10-1F: write to ADPCM-A
            self.adpcm_a.write(self.address as u32 & 0x0F, data);
        } else if self.address == 0x29 {
            // 29: special IRQ mask register
            self.irq_enable = data;
            self.fm
                .set_irq_mask(self.irq_enable & !self.flag_control & 0x1F);
        } else {
            // 20-28, 2A-FF: write to FM
            self.fm.write(self.address, data);
        }

        32 * self.fm.clock_prescale()
    }

    /// Latches the register address for the high bank.
    pub fn write_address_hi(&mut self, data: u8) -> u32 {
        // Port 1 address: set bit 8 to distinguish from port 0.
        self.address = 0x100 | data as u16;
        0
    }

    /// Writes a value to the previously addressed register (high bank).
    pub fn write_data_hi(&mut self, data: u8) -> u32 {
        // Ignore if paired with lower address (port 0 data to port 1).
        if helpers::bit(self.address as u32, 8) == 0 {
            return 0;
        }

        if self.address < 0x110 {
            // 100-10F: write to ADPCM-B
            self.adpcm_b.write(
                self.address as u32 & 0x0F,
                data,
                self.adpcm_b_ram.as_deref_mut(),
            );
        } else if self.address == 0x110 {
            // 110: IRQ flag control
            if helpers::bit(data as u32, 7) != 0 {
                self.fm.set_reset_status(0, 0xFF);
                self.adpcm_b
                    .clear_status(AdpcmBChannel::STATUS_EOS | AdpcmBChannel::STATUS_PLAYING);
            } else {
                self.flag_control = data;
                self.fm
                    .set_irq_mask(self.irq_enable & !self.flag_control & 0x1F);
            }
        } else {
            // 111-1FF: write to FM
            self.fm.write(self.address, data);
        }

        32 * self.fm.clock_prescale()
    }

    /// Generates audio samples into `output`.
    ///
    /// Each sample contains three channels: `[FM_L, FM_R, SSG]`.
    pub fn generate(&mut self, output: &mut [YmfmOutput3]) {
        let numsamples = output.len();
        let sampindex = self.ssg_resampler.sampindex();

        // FM output is just repeated the prescale number of times;
        // fm_samples_per_output == 0 is a special 1.5:1 case.
        if self.fm_samples_per_output != 0 {
            for (samp, out) in output.iter_mut().enumerate() {
                if (sampindex + samp as u32).is_multiple_of(self.fm_samples_per_output) {
                    self.clock_fm_and_adpcm();
                }
                out.data[0] = self.last_fm[0];
                out.data[1] = self.last_fm[1];
            }
        } else {
            // 1.5:1 ratio: clock FM on steps 0 and 1 of every 3, averaging
            // the two results on step 1 to approximate the half-sample offset.
            for (samp, out) in output.iter_mut().enumerate() {
                let step = (sampindex + samp as u32) % 3;
                if step == 0 {
                    self.clock_fm_and_adpcm();
                }
                out.data[0] = self.last_fm[0];
                out.data[1] = self.last_fm[1];
                if step == 1 {
                    self.clock_fm_and_adpcm();
                    out.data[0] = (out.data[0] + self.last_fm[0]) / 2;
                    out.data[1] = (out.data[1] + self.last_fm[1]) / 2;
                }
            }
        }

        // Resample the SSG as configured.
        // SAFETY: YmfmOutput3 is #[repr(C)] with a single [i32; 3] field,
        // so &mut [YmfmOutput3] has the same layout as &mut [[i32; 3]].
        #[allow(unsafe_code)]
        let output_nested = unsafe { &mut *(output as *mut [YmfmOutput3] as *mut [[i32; 3]]) };
        const _: () = assert!(size_of::<YmfmOutput3>() == size_of::<[i32; 3]>());

        let output_flat = output_nested.as_flattened_mut();
        self.ssg_resampler
            .resample(&mut self.ssg, output_flat, numsamples);
    }

    /// Notifies the chip that the specified timer has expired.
    pub fn timer_expired(&mut self, timer_id: u32) {
        self.fm.engine_timer_expired(timer_id);
    }

    /// Returns and clears the pending update for a timer.
    pub fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate> {
        self.fm.take_timer_update(timer_id)
    }

    /// Returns and clears the pending IRQ output update.
    pub fn take_irq_update(&mut self) -> Option<bool> {
        self.fm.take_irq_update()
    }

    /// Returns whether the chip IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.fm.irq_asserted()
    }

    fn clock_fm_and_adpcm(&mut self) {
        // Top bit of the IRQ enable flags controls 3-channel vs 6-channel mode.
        let fmmask = if helpers::bit(self.irq_enable as u32, 7) != 0 {
            0x3F
        } else {
            0x07
        };

        let env_counter = self.fm.clock(OpnaRegisters::ALL_CHANNELS);

        // Clock the ADPCM-A engine on every envelope cycle
        // (channels 4 and 5 clock every 2 envelope clocks).
        if helpers::bitfield(env_counter, 0, 2) == 0 {
            let chanmask = if helpers::bitfield(env_counter, 2, 1) != 0 {
                0x0F
            } else {
                0x3F
            };
            self.adpcm_a.clock(chanmask, &self.adpcm_a_rom);
        }

        // Clock the ADPCM-B engine every cycle.
        self.adpcm_b.clock(self.adpcm_b_ram.as_deref_mut());

        // Update the FM content; OPNA is 13-bit with no intermediate clipping.
        self.last_fm = [0, 0];
        self.fm.output_mut(&mut self.last_fm, 1, 32767, fmmask);

        // Mix in the ADPCM and clamp.
        self.adpcm_a.output::<2>(&mut self.last_fm, 0x3F);
        self.adpcm_b.output::<2>(&mut self.last_fm, 1);

        for v in &mut self.last_fm {
            *v = (*v).clamp(-32768, 32767);
        }
    }

    fn update_prescale(&mut self, prescale: u32) {
        self.fm.set_clock_prescale(prescale);

        // Fidelity:   ---- minimum ----    ---- medium -----    ---- maximum-----
        //              rate = clock/48      rate = clock/24      rate = clock/8
        // Prescale    FM rate  SSG rate    FM rate  SSG rate    FM rate  SSG rate
        //     6          3:1     2:3          6:1     4:3         18:1     4:1
        //     3        1.5:1     1:3          3:1     2:3          9:1     2:1
        //     2          1:1     1:6          2:1     1:3          6:1     1:1
        match self.fidelity {
            YmfmOpnFidelity::Min => match prescale {
                3 => {
                    self.fm_samples_per_output = 0; // 1.5:1
                    self.ssg_resampler.configure(1, 3);
                }
                2 => {
                    self.fm_samples_per_output = 1;
                    self.ssg_resampler.configure(1, 6);
                }
                _ => {
                    self.fm_samples_per_output = 3;
                    self.ssg_resampler.configure(2, 3);
                }
            },
            YmfmOpnFidelity::Med => match prescale {
                3 => {
                    self.fm_samples_per_output = 3;
                    self.ssg_resampler.configure(2, 3);
                }
                2 => {
                    self.fm_samples_per_output = 2;
                    self.ssg_resampler.configure(1, 3);
                }
                _ => {
                    self.fm_samples_per_output = 6;
                    self.ssg_resampler.configure(4, 3);
                }
            },
            YmfmOpnFidelity::Max => match prescale {
                3 => {
                    self.fm_samples_per_output = 9;
                    self.ssg_resampler.configure(2, 1);
                }
                2 => {
                    self.fm_samples_per_output = 6;
                    self.ssg_resampler.configure(1, 1);
                }
                _ => {
                    self.fm_samples_per_output = 18;
                    self.ssg_resampler.configure(4, 1);
                }
            },
        }
    }
}

/// Busy time in input clocks of a YMF288 register access in YMF288 mode.
const YMF288_MODE_BUSY_CLOCKS: u32 = 16;
/// ID code the YMF288 returns from register 0xFF.
const YMF288_ID: u8 = 2;

save_state::runtime_state! {
/// Complete mutable state of a YMF288 chip.
#[derive(Clone)]
pub struct Ymf288State {
    fm: FmEngine<OpnaRegisters>,
    ssg: SsgEngine,
    ssg_resampler: SsgResampler,
    adpcm_a: AdpcmAEngine,
    fidelity: YmfmOpnFidelity,
    address: u16,
    fm_samples_per_output: u32,
    last_fm: [i32; 2],
    irq_enable: u8,
    flag_control: u8,
    adpcm_a_rom_identity: save_state::ResourceIdentity,
}}

/// Yamaha YMF288 (OPN3L) emulator.
///
/// The YMF288 is a YM2608 without the ADPCM-B unit, the prescaler, CSM and the
/// I/O ports. It has shorter busy times, and in YMF288 mode every register
/// can be read back.
#[derive(Clone)]
pub struct Ymf288 {
    fm: FmEngine<OpnaRegisters>,
    ssg: SsgEngine,
    ssg_resampler: SsgResampler,
    adpcm_a: AdpcmAEngine,
    adpcm_a_rom: Vec<u8>,
    fidelity: YmfmOpnFidelity,
    address: u16,
    fm_samples_per_output: u32,
    last_fm: [i32; 2],
    irq_enable: u8,
    flag_control: u8,
}

impl Ymf288 {
    /// Creates a new YMF288 instance.
    pub fn new() -> Self {
        let mut chip = Self {
            fm: FmEngine::new(),
            ssg: SsgEngine::new(),
            ssg_resampler: SsgResampler::new(true, 2),
            adpcm_a: AdpcmAEngine::new(0),
            adpcm_a_rom: SILENT_ADPCM_MEMORY.to_vec(),
            fidelity: YmfmOpnFidelity::Max,
            address: 0,
            fm_samples_per_output: 0,
            last_fm: [0, 0],
            irq_enable: 0x03,
            flag_control: 0x03,
        };
        chip.update_prescale();
        chip
    }

    /// Captures mutable chip state and the retained rhythm ROM identity.
    pub fn capture_state(&self) -> Ymf288State {
        Ymf288State {
            fm: self.fm.clone(),
            ssg: self.ssg.clone(),
            ssg_resampler: self.ssg_resampler.clone(),
            adpcm_a: self.adpcm_a.clone(),
            fidelity: self.fidelity,
            address: self.address,
            fm_samples_per_output: self.fm_samples_per_output,
            last_fm: self.last_fm,
            irq_enable: self.irq_enable,
            flag_control: self.flag_control,
            adpcm_a_rom_identity: save_state::ResourceIdentity::from_bytes(&self.adpcm_a_rom),
        }
    }

    /// Restores mutable state while retaining the rhythm ROM.
    pub fn restore_state(
        &mut self,
        state: Ymf288State,
    ) -> Result<(), save_state::StateValidationError> {
        let identity = save_state::ResourceIdentity::from_bytes(&self.adpcm_a_rom);
        save_state::restore_root(self, state, &identity)
    }

    /// Resets the chip to its initial power-on state.
    pub fn reset(&mut self) {
        self.fm.reset();
        self.ssg.reset();
        self.adpcm_a.reset();

        // Configure ADPCM percussion sounds; these are present in an embedded ROM.
        self.adpcm_a.set_start_end(0, 0x0000, 0x01BF); // bass drum
        self.adpcm_a.set_start_end(1, 0x01C0, 0x043F); // snare drum
        self.adpcm_a.set_start_end(2, 0x0440, 0x1B7F); // top cymbal
        self.adpcm_a.set_start_end(3, 0x1B80, 0x1CFF); // high hat
        self.adpcm_a.set_start_end(4, 0x1D00, 0x1F7F); // tom tom
        self.adpcm_a.set_start_end(5, 0x1F80, 0x1FFF); // rim shot

        // Initialize our special interrupt states, then read the upper status
        // register, which updates the IRQs.
        self.irq_enable = 0x03;
        self.flag_control = 0x00;
        self.read_status_hi(false);
    }

    /// Copies ADPCM-A rhythm ROM data into the chip.
    ///
    /// Panics if `data` is empty. Short data is zero-padded to the rhythm ROM
    /// size, and oversized data is truncated.
    pub fn set_adpcm_a_rom(&mut self, data: &[u8]) {
        assert!(!data.is_empty(), "ADPCM-A ROM data must not be empty");
        self.adpcm_a_rom.clear();
        self.adpcm_a_rom.resize(YM2608_ADPCM_A_ROM_SIZE, 0);
        let length = data.len().min(YM2608_ADPCM_A_ROM_SIZE);
        self.adpcm_a_rom[..length].copy_from_slice(&data[..length]);
    }

    /// Clears ADPCM-A rhythm ROM data. Reads from missing ROM data return zero.
    pub fn clear_adpcm_a_rom(&mut self) {
        self.adpcm_a_rom.clear();
        self.adpcm_a_rom.extend_from_slice(&SILENT_ADPCM_MEMORY);
    }

    /// Sets the output fidelity level.
    pub fn set_fidelity(&mut self, fidelity: YmfmOpnFidelity) {
        self.fidelity = fidelity;
        self.update_prescale();
    }

    /// Returns the output sample rate in Hz for the given `input_clock` in Hz.
    pub fn sample_rate(&self, input_clock: u32) -> u32 {
        match self.fidelity {
            YmfmOpnFidelity::Min | YmfmOpnFidelity::Med => input_clock / 144,
            YmfmOpnFidelity::Max => input_clock / 16,
        }
    }

    /// Returns the effective SSG clock in Hz for the given `input_clock` in Hz.
    pub fn ssg_effective_clock(&self, input_clock: u32) -> u32 {
        input_clock / 4
    }

    /// Reads the chip status register (low).
    pub fn read_status(&mut self, busy: bool) -> u8 {
        let mut result =
            self.fm.status() & (OpnaRegisters::STATUS_TIMERA | OpnaRegisters::STATUS_TIMERB);
        if busy {
            result |= OpnaRegisters::STATUS_BUSY;
        }
        result
    }

    /// Reads data from the currently addressed register.
    ///
    /// In YMF288 mode every register reads back its value.
    pub fn read_data(&mut self) -> u8 {
        if self.address < 0x0E {
            // 00-0D: Read from SSG
            self.ssg.read(self.address as u32 & 0x0F)
        } else if self.address < 0x10 {
            // 0E-0F: I/O ports not supported
            0xFF
        } else if self.address == 0xFF {
            YMF288_ID
        } else if self.ymf288_mode() {
            self.fm.regs.read(self.address)
        } else {
            0
        }
    }

    /// Reads the extended status register, which holds the timer flags only.
    pub fn read_status_hi(&mut self, busy: bool) -> u8 {
        let mut status =
            self.fm.status() & (OpnaRegisters::STATUS_TIMERA | OpnaRegisters::STATUS_TIMERB);

        // Turn off any bits that have been requested to be masked.
        status &= !(self.flag_control & 0x03);

        // Update the status so that IRQs are propagated.
        self.fm.set_reset_status(status, !status);

        if busy {
            status |= OpnaRegisters::STATUS_BUSY;
        }
        status
    }

    /// Latches the register address for the low bank.
    pub fn write_address(&mut self, data: u8) -> u32 {
        self.address = data as u16;
        self.address_busy_clocks()
    }

    /// Writes a value to the previously addressed register (low bank).
    pub fn write_data(&mut self, data: u8) -> u32 {
        // Ignore if paired with upper address (port 1 data to port 0).
        if helpers::bit(self.address as u32, 8) != 0 {
            return 0;
        }

        let mut busy_clocks = self.data_busy_clocks();
        if self.address < 0x0E {
            // 00-0D: write to SSG
            self.ssg.write(self.address as u32 & 0x0F, data);
        } else if self.address < 0x10 {
            // 0E-0F: I/O ports not supported
        } else if self.address < 0x20 {
            // 10-1F: write to ADPCM-A
            self.adpcm_a.write(self.address as u32 & 0x0F, data);
            busy_clocks = 32 * self.fm.clock_prescale();
        } else if self.address == 0x27 {
            // 27: mode register; CSM is not supported
            self.fm.write(self.address, data & 0x7F);
        } else if self.address == 0x29 {
            // 29: special IRQ mask register
            self.irq_enable = data;
            self.fm
                .set_irq_mask(self.irq_enable & !self.flag_control & 0x03);
        } else {
            // 20-26, 28, 2A-FF: write to FM
            self.fm.write(self.address, data);
        }
        busy_clocks
    }

    /// Latches the register address for the high bank.
    pub fn write_address_hi(&mut self, data: u8) -> u32 {
        self.address = 0x100 | data as u16;
        self.address_busy_clocks()
    }

    /// Writes a value to the previously addressed register (high bank).
    pub fn write_data_hi(&mut self, data: u8) -> u32 {
        // Ignore if paired with lower address (port 0 data to port 1).
        if helpers::bit(self.address as u32, 8) == 0 {
            return 0;
        }

        let busy_clocks = self.data_busy_clocks();
        if self.address == 0x110 {
            // 110: IRQ flag control
            if helpers::bit(data as u32, 7) != 0 {
                self.fm.set_reset_status(0, 0xFF);
            } else {
                self.flag_control = data;
                self.fm
                    .set_irq_mask(self.irq_enable & !self.flag_control & 0x03);
            }
        } else {
            // 100-10F, 111-1FF: write to FM
            self.fm.write(self.address, data);
        }
        busy_clocks
    }

    /// Generates audio samples into `output`.
    ///
    /// Each sample contains three channels: `[FM_L, FM_R, SSG]`.
    pub fn generate(&mut self, output: &mut [YmfmOutput3]) {
        let numsamples = output.len();
        let sampindex = self.ssg_resampler.sampindex();

        for (samp, out) in output.iter_mut().enumerate() {
            if (sampindex + samp as u32).is_multiple_of(self.fm_samples_per_output) {
                self.clock_fm_and_adpcm();
            }
            out.data[0] = self.last_fm[0];
            out.data[1] = self.last_fm[1];
        }

        // Resample the SSG as configured.
        // SAFETY: YmfmOutput3 is #[repr(C)] with a single [i32; 3] field,
        // so &mut [YmfmOutput3] has the same layout as &mut [[i32; 3]].
        #[allow(unsafe_code)]
        let output_nested = unsafe { &mut *(output as *mut [YmfmOutput3] as *mut [[i32; 3]]) };
        const _: () = assert!(size_of::<YmfmOutput3>() == size_of::<[i32; 3]>());

        let output_flat = output_nested.as_flattened_mut();
        self.ssg_resampler
            .resample(&mut self.ssg, output_flat, numsamples);
    }

    /// Notifies the chip that the specified timer has expired.
    pub fn timer_expired(&mut self, timer_id: u32) {
        self.fm.engine_timer_expired(timer_id);
    }

    /// Returns and clears the pending update for a timer.
    pub fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate> {
        self.fm.take_timer_update(timer_id)
    }

    /// Returns and clears the pending IRQ output update.
    pub fn take_irq_update(&mut self) -> Option<bool> {
        self.fm.take_irq_update()
    }

    /// Returns whether the chip IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.fm.irq_asserted()
    }

    /// Returns whether YMF288 mode (register 0x20 bit 1) is enabled.
    fn ymf288_mode(&self) -> bool {
        self.fm.regs.read(0x20) & 0x02 != 0
    }

    /// Returns the busy time of an address write.
    fn address_busy_clocks(&self) -> u32 {
        if self.ymf288_mode() {
            YMF288_MODE_BUSY_CLOCKS
        } else {
            0
        }
    }

    /// Returns the busy time of a data write outside the ADPCM-A registers.
    fn data_busy_clocks(&self) -> u32 {
        if self.ymf288_mode() {
            YMF288_MODE_BUSY_CLOCKS
        } else {
            32 * self.fm.clock_prescale()
        }
    }

    fn clock_fm_and_adpcm(&mut self) {
        // Top bit of the IRQ enable flags controls 3-channel vs 6-channel mode.
        let fmmask = if helpers::bit(self.irq_enable as u32, 7) != 0 {
            0x3F
        } else {
            0x07
        };

        let env_counter = self.fm.clock(OpnaRegisters::ALL_CHANNELS);

        // Clock the ADPCM-A engine on every envelope cycle
        // (channels 4 and 5 clock every 2 envelope clocks).
        if helpers::bitfield(env_counter, 0, 2) == 0 {
            let chanmask = if helpers::bitfield(env_counter, 2, 1) != 0 {
                0x0F
            } else {
                0x3F
            };
            self.adpcm_a.clock(chanmask, &self.adpcm_a_rom);
        }

        // Update the FM content; OPNA is 13-bit with no intermediate clipping.
        self.last_fm = [0, 0];
        self.fm.output_mut(&mut self.last_fm, 1, 32767, fmmask);

        // Mix in the ADPCM.
        self.adpcm_a.output::<2>(&mut self.last_fm, 0x3F);
    }

    fn update_prescale(&mut self) {
        // Fidelity:   ---- minimum ----    ---- medium -----    ---- maximum-----
        //              rate = clock/144     rate = clock/144     rate = clock/16
        // Prescale    FM rate  SSG rate    FM rate  SSG rate    FM rate  SSG rate
        //     6          1:1     2:9          1:1     2:9         9:1     2:1
        match self.fidelity {
            YmfmOpnFidelity::Min | YmfmOpnFidelity::Med => {
                self.fm_samples_per_output = 1;
                self.ssg_resampler.configure(2, 9);
            }
            YmfmOpnFidelity::Max => {
                self.fm_samples_per_output = 9;
                self.ssg_resampler.configure(2, 1);
            }
        }
    }
}

impl Default for Ymf288 {
    fn default() -> Self {
        Self::new()
    }
}

impl save_state::ValidateState<save_state::ResourceIdentity> for Ymf288State {
    fn validate_state(
        &self,
        current_rom_identity: &save_state::ResourceIdentity,
    ) -> Result<(), save_state::StateValidationError> {
        if &self.adpcm_a_rom_identity != current_rom_identity {
            return Err(save_state::StateValidationError::new(
                "YMF288 rhythm ROM identity differs",
            ));
        }
        if self.fm.operators.len() != OpnaRegisters::OPERATORS
            || self.fm.channels.len() != OpnaRegisters::CHANNELS
        {
            return Err(save_state::StateValidationError::new(
                "YMF288 state topology is invalid",
            ));
        }
        Ok(())
    }
}

impl save_state::AfterRestore for Ymf288 {
    fn after_restore(&mut self) {}
}

impl save_state::RestoreTarget for Ymf288 {
    type State = Ymf288State;
    type ValidationContext = save_state::ResourceIdentity;

    fn replace_state(&mut self, state: Self::State) {
        self.fm = state.fm;
        self.ssg = state.ssg;
        self.ssg_resampler = state.ssg_resampler;
        self.adpcm_a = state.adpcm_a;
        self.fidelity = state.fidelity;
        self.address = state.address;
        self.fm_samples_per_output = state.fm_samples_per_output;
        self.last_fm = state.last_fm;
        self.irq_enable = state.irq_enable;
        self.flag_control = state.flag_control;
    }
}

/// FM channel mask of the YM2610. The chip has FM channels 1, 2, 4 and 5.
pub const YM2610_FM_CHANNEL_MASK: u32 = 0x36;
/// FM channel mask of the YM2610B. All six channels are present.
pub const YM2610B_FM_CHANNEL_MASK: u32 = 0x3F;
/// Visible bits of the YM2610 end-of-sample status. Bit 6 holds the hidden live ADPCM-B EOS.
const YM2610_EOS_FLAGS_MASK: u8 = 0xBF;
/// Hidden status bit that tracks the live ADPCM-B end-of-sample signal.
const YM2610_ADPCM_B_LIVE_EOS: u8 = 0x40;
/// Mask of all six ADPCM-A channels.
const YM2610_ADPCM_A_ALL_CHANNELS: u32 = 0x3F;
/// Address shift of the ADPCM-A and ADPCM-B units on the YM2610 external memory bus.
const YM2610_ADPCM_ADDRESS_SHIFT: u32 = 8;
/// ADPCM-B control register 1 bit that selects external memory.
const YM2610_ADPCM_B_EXTERNAL: u8 = 0x20;
/// ADPCM-B control register 1 bit that selects recording.
const YM2610_ADPCM_B_RECORD: u8 = 0x40;

save_state::runtime_state! {
/// Complete mutable state of a YM2610 family chip.
#[derive(Clone)]
pub struct Ym2610FamilyState<const FM_CHANNEL_MASK: u32> {
    fm: FmEngine<OpnaRegisters>,
    ssg: SsgEngine,
    ssg_resampler: SsgResampler,
    adpcm_a: AdpcmAEngine,
    adpcm_b: AdpcmBEngine,
    fidelity: YmfmOpnFidelity,
    address: u16,
    fm_samples_per_output: u32,
    last_fm: [i32; 2],
    eos_status: u8,
    flag_mask: u8,
    adpcm_a_rom_identity: save_state::ResourceIdentity,
    adpcm_b_rom_identity: save_state::ResourceIdentity,
}}

/// Complete mutable state of a YM2610 chip.
pub type Ym2610State = Ym2610FamilyState<YM2610_FM_CHANNEL_MASK>;
/// Complete mutable state of a YM2610B chip.
pub type Ym2610bState = Ym2610FamilyState<YM2610B_FM_CHANNEL_MASK>;

/// Identities of the ADPCM-A and ADPCM-B ROMs attached to a YM2610 family chip.
pub type Ym2610RomIdentities = (save_state::ResourceIdentity, save_state::ResourceIdentity);

/// Yamaha YM2610 family (OPNB) emulator.
///
/// The YM2610 family has the OPNA FM core, an SSG, six ADPCM-A channels and one
/// ADPCM-B channel. Both ADPCM units play samples from external ROMs.
/// `FM_CHANNEL_MASK` selects the FM channels that the chip has.
#[derive(Clone)]
pub struct Ym2610Family<const FM_CHANNEL_MASK: u32> {
    fm: FmEngine<OpnaRegisters>,
    ssg: SsgEngine,
    ssg_resampler: SsgResampler,
    adpcm_a: AdpcmAEngine,
    adpcm_b: AdpcmBEngine,
    adpcm_a_rom: Vec<u8>,
    adpcm_b_rom: Option<Vec<u8>>,
    fidelity: YmfmOpnFidelity,
    address: u16,
    fm_samples_per_output: u32,
    last_fm: [i32; 2],
    eos_status: u8,
    flag_mask: u8,
}

/// Yamaha YM2610 (OPNB) with four FM channels.
pub type Ym2610 = Ym2610Family<YM2610_FM_CHANNEL_MASK>;
/// Yamaha YM2610B (OPNB2) with six FM channels.
pub type Ym2610b = Ym2610Family<YM2610B_FM_CHANNEL_MASK>;

impl<const FM_CHANNEL_MASK: u32> Ym2610Family<FM_CHANNEL_MASK> {
    /// Creates a new YM2610 family instance.
    pub fn new() -> Self {
        let mut chip = Self {
            fm: FmEngine::new(),
            ssg: SsgEngine::new(),
            ssg_resampler: SsgResampler::new(true, 2),
            adpcm_a: AdpcmAEngine::new(YM2610_ADPCM_ADDRESS_SHIFT),
            adpcm_b: AdpcmBEngine::new(YM2610_ADPCM_ADDRESS_SHIFT),
            adpcm_a_rom: SILENT_ADPCM_MEMORY.to_vec(),
            adpcm_b_rom: None,
            fidelity: YmfmOpnFidelity::Max,
            address: 0,
            fm_samples_per_output: 0,
            last_fm: [0, 0],
            eos_status: 0x00,
            flag_mask: YM2610_EOS_FLAGS_MASK,
        };
        chip.update_prescale();
        chip
    }

    /// Captures mutable chip state and the identities of the attached ROMs.
    pub fn capture_state(&self) -> Ym2610FamilyState<FM_CHANNEL_MASK> {
        let (adpcm_a_rom_identity, adpcm_b_rom_identity) = self.rom_identities();
        Ym2610FamilyState {
            fm: self.fm.clone(),
            ssg: self.ssg.clone(),
            ssg_resampler: self.ssg_resampler.clone(),
            adpcm_a: self.adpcm_a.clone(),
            adpcm_b: self.adpcm_b.clone(),
            fidelity: self.fidelity,
            address: self.address,
            fm_samples_per_output: self.fm_samples_per_output,
            last_fm: self.last_fm,
            eos_status: self.eos_status,
            flag_mask: self.flag_mask,
            adpcm_a_rom_identity,
            adpcm_b_rom_identity,
        }
    }

    /// Restores mutable state while retaining the attached ROMs.
    pub fn restore_state(
        &mut self,
        state: Ym2610FamilyState<FM_CHANNEL_MASK>,
    ) -> Result<(), save_state::StateValidationError> {
        let identities = self.rom_identities();
        save_state::restore_root(self, state, &identities)
    }

    /// Resets the chip to its initial power-on state.
    pub fn reset(&mut self) {
        self.fm.reset();
        self.ssg.reset();
        self.adpcm_a.reset();
        self.adpcm_b.reset();

        self.eos_status = 0x00;
        self.flag_mask = YM2610_EOS_FLAGS_MASK;
    }

    /// Attaches the ADPCM-A sample ROM.
    ///
    /// Panics if `data` is empty. Reads past the end of the ROM wrap around.
    pub fn set_adpcm_a_rom(&mut self, data: Vec<u8>) {
        assert!(!data.is_empty(), "ADPCM-A ROM data must not be empty");
        self.adpcm_a_rom = data;
    }

    /// Removes the ADPCM-A sample ROM. Reads from a missing ROM return zero.
    pub fn clear_adpcm_a_rom(&mut self) {
        self.adpcm_a_rom = SILENT_ADPCM_MEMORY.to_vec();
    }

    /// Attaches the ADPCM-B sample ROM.
    ///
    /// Panics if `data` is empty. Reads past the end of the ROM wrap around.
    pub fn set_adpcm_b_rom(&mut self, data: Vec<u8>) {
        assert!(!data.is_empty(), "ADPCM-B ROM data must not be empty");
        self.adpcm_b_rom = Some(data);
    }

    /// Removes the ADPCM-B sample ROM. Reads from a missing ROM return zero.
    pub fn clear_adpcm_b_rom(&mut self) {
        self.adpcm_b_rom = None;
    }

    /// Sets the output fidelity level.
    pub fn set_fidelity(&mut self, fidelity: YmfmOpnFidelity) {
        self.fidelity = fidelity;
        self.update_prescale();
    }

    /// Returns the output sample rate in Hz for the given `input_clock` in Hz.
    pub fn sample_rate(&mut self, input_clock: u32) -> u32 {
        match self.fidelity {
            YmfmOpnFidelity::Min | YmfmOpnFidelity::Med => input_clock / 144,
            YmfmOpnFidelity::Max => input_clock / 16,
        }
    }

    /// Reads the chip status register (low).
    pub fn read_status(&mut self, busy: bool) -> u8 {
        let mut result =
            self.fm.status() & (OpnaRegisters::STATUS_TIMERA | OpnaRegisters::STATUS_TIMERB);
        if busy {
            result |= OpnaRegisters::STATUS_BUSY;
        }
        result
    }

    /// Reads data from the currently addressed register (low bank).
    pub fn read_data(&mut self) -> u8 {
        if self.address < 0x0E {
            // 00-0D: Read from SSG
            self.ssg.read(self.address as u32 & 0x0F)
        } else if self.address < 0x10 {
            // 0E-0F: I/O ports are not present
            0xFF
        } else if self.address == 0xFF {
            // FF: ID code
            1
        } else {
            0
        }
    }

    /// Reads the end-of-sample status register (high).
    ///
    /// Bits 0-5 flag the ADPCM-A channels and bit 7 flags ADPCM-B.
    pub fn read_status_hi(&mut self) -> u8 {
        self.eos_status & self.flag_mask
    }

    /// Reads data from the currently addressed register (high bank).
    pub fn read_data_hi(&mut self) -> u8 {
        0
    }

    /// Latches the register address for the low bank.
    pub fn write_address(&mut self, data: u8) -> u32 {
        self.address = data as u16;
        0
    }

    /// Writes a value to the previously addressed register (low bank).
    pub fn write_data(&mut self, data: u8) -> u32 {
        // Ignore if paired with upper address (port 1 address, port 0 data).
        if helpers::bit(self.address as u32, 8) != 0 {
            return 0;
        }

        if self.address < 0x0E {
            // 00-0D: write to SSG
            self.ssg.write(self.address as u32 & 0x0F, data);
        } else if self.address < 0x10 {
            // 0E-0F: I/O ports are not present
        } else if self.address < 0x1C {
            // 10-1B: write to ADPCM-B. The chip forces external memory on and recording off.
            let data = if self.address == 0x10 {
                (data | YM2610_ADPCM_B_EXTERNAL) & !YM2610_ADPCM_B_RECORD
            } else {
                data
            };
            self.adpcm_b.write(
                self.address as u32 & 0x0F,
                data,
                self.adpcm_b_rom.as_deref_mut(),
            );
        } else if self.address == 0x1C {
            // 1C: EOS flag reset
            self.flag_mask = !data & YM2610_EOS_FLAGS_MASK;
            self.eos_status &= !(data & YM2610_EOS_FLAGS_MASK);
        } else {
            // 1D-FF: write to FM
            self.fm.write(self.address, data);
        }

        32 * self.fm.clock_prescale()
    }

    /// Latches the register address for the high bank.
    pub fn write_address_hi(&mut self, data: u8) -> u32 {
        self.address = 0x100 | data as u16;
        0
    }

    /// Writes a value to the previously addressed register (high bank).
    pub fn write_data_hi(&mut self, data: u8) -> u32 {
        // Ignore if paired with lower address (port 0 address, port 1 data).
        if helpers::bit(self.address as u32, 8) == 0 {
            return 0;
        }

        if self.address < 0x130 {
            // 100-12F: write to ADPCM-A
            self.adpcm_a.write(self.address as u32 & 0x3F, data);
        } else {
            // 130-1FF: write to FM
            self.fm.write(self.address, data);
        }

        32 * self.fm.clock_prescale()
    }

    /// Generates audio samples into `output`.
    ///
    /// Each sample contains three channels: `[FM_L, FM_R, SSG]`.
    pub fn generate(&mut self, output: &mut [YmfmOutput3]) {
        let numsamples = output.len();
        let sampindex = self.ssg_resampler.sampindex();

        for (samp, out) in output.iter_mut().enumerate() {
            if (sampindex + samp as u32).is_multiple_of(self.fm_samples_per_output) {
                self.clock_fm_and_adpcm();
            }
            out.data[0] = self.last_fm[0];
            out.data[1] = self.last_fm[1];
        }

        // SAFETY: YmfmOutput3 is #[repr(C)] with a single [i32; 3] field,
        // so &mut [YmfmOutput3] has the same layout as &mut [[i32; 3]].
        #[allow(unsafe_code)]
        let output_nested = unsafe { &mut *(output as *mut [YmfmOutput3] as *mut [[i32; 3]]) };
        const _: () = assert!(size_of::<YmfmOutput3>() == size_of::<[i32; 3]>());

        let output_flat = output_nested.as_flattened_mut();
        self.ssg_resampler
            .resample(&mut self.ssg, output_flat, numsamples);
    }

    /// Notifies the chip that the specified timer has expired.
    pub fn timer_expired(&mut self, timer_id: u32) {
        self.fm.engine_timer_expired(timer_id);
    }

    /// Returns and clears the pending update for a timer.
    pub fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate> {
        self.fm.take_timer_update(timer_id)
    }

    /// Returns and clears the pending IRQ output update.
    pub fn take_irq_update(&mut self) -> Option<bool> {
        self.fm.take_irq_update()
    }

    /// Returns whether the chip IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.fm.irq_asserted()
    }

    fn rom_identities(&self) -> Ym2610RomIdentities {
        (
            save_state::ResourceIdentity::from_bytes(&self.adpcm_a_rom),
            save_state::ResourceIdentity::from_bytes(self.adpcm_b_rom.as_deref().unwrap_or(&[])),
        )
    }

    fn clock_fm_and_adpcm(&mut self) {
        let env_counter = self.fm.clock(FM_CHANNEL_MASK);

        // Clock all ADPCM-A channels on every envelope cycle.
        if helpers::bitfield(env_counter, 0, 2) == 0 {
            self.eos_status |=
                self.adpcm_a
                    .clock(YM2610_ADPCM_A_ALL_CHANNELS, &self.adpcm_a_rom) as u8;
        }

        // Clock the ADPCM-B engine every cycle.
        self.adpcm_b.clock(self.adpcm_b_rom.as_deref_mut());

        // Bit 6 tracks the live ADPCM-B EOS. A change latches it into the visible bit 7.
        let live_eos = if self.adpcm_b.status() as u32 & AdpcmBChannel::STATUS_EOS != 0 {
            YM2610_ADPCM_B_LIVE_EOS
        } else {
            0x00
        };
        if (live_eos ^ self.eos_status) & YM2610_ADPCM_B_LIVE_EOS != 0 {
            self.eos_status = (self.eos_status & !0xC0) | live_eos | (live_eos << 1);
        }

        // OPNB is 13-bit with no intermediate clipping.
        self.last_fm = [0, 0];
        self.fm
            .output_mut(&mut self.last_fm, 1, 32767, FM_CHANNEL_MASK);

        self.adpcm_a
            .output::<2>(&mut self.last_fm, YM2610_ADPCM_A_ALL_CHANNELS);
        self.adpcm_b.output::<2>(&mut self.last_fm, 1);

        for value in &mut self.last_fm {
            *value = (*value).clamp(-32768, 32767);
        }
    }

    fn update_prescale(&mut self) {
        // Fidelity:   ---- minimum ----    ---- medium -----    ---- maximum-----
        //              rate = clock/144     rate = clock/144     rate = clock/16
        // Prescale    FM rate  SSG rate    FM rate  SSG rate    FM rate  SSG rate
        //     6          1:1     2:9          1:1     2:9         9:1     2:1
        match self.fidelity {
            YmfmOpnFidelity::Min | YmfmOpnFidelity::Med => {
                self.fm_samples_per_output = 1;
                self.ssg_resampler.configure(2, 9);
            }
            YmfmOpnFidelity::Max => {
                self.fm_samples_per_output = 9;
                self.ssg_resampler.configure(2, 1);
            }
        }
    }
}

impl<const FM_CHANNEL_MASK: u32> Default for Ym2610Family<FM_CHANNEL_MASK> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const FM_CHANNEL_MASK: u32> save_state::ValidateState<Ym2610RomIdentities>
    for Ym2610FamilyState<FM_CHANNEL_MASK>
{
    fn validate_state(
        &self,
        current_rom_identities: &Ym2610RomIdentities,
    ) -> Result<(), save_state::StateValidationError> {
        if self.adpcm_a_rom_identity != current_rom_identities.0 {
            return Err(save_state::StateValidationError::new(
                "YM2610 ADPCM-A ROM identity differs",
            ));
        }
        if self.adpcm_b_rom_identity != current_rom_identities.1 {
            return Err(save_state::StateValidationError::new(
                "YM2610 ADPCM-B ROM identity differs",
            ));
        }
        if self.fm.operators.len() != OpnaRegisters::OPERATORS
            || self.fm.channels.len() != OpnaRegisters::CHANNELS
        {
            return Err(save_state::StateValidationError::new(
                "YM2610 state topology is invalid",
            ));
        }
        Ok(())
    }
}

impl<const FM_CHANNEL_MASK: u32> save_state::AfterRestore for Ym2610Family<FM_CHANNEL_MASK> {
    fn after_restore(&mut self) {}
}

impl<const FM_CHANNEL_MASK: u32> save_state::RestoreTarget for Ym2610Family<FM_CHANNEL_MASK> {
    type State = Ym2610FamilyState<FM_CHANNEL_MASK>;
    type ValidationContext = Ym2610RomIdentities;

    fn replace_state(&mut self, state: Self::State) {
        self.fm = state.fm;
        self.ssg = state.ssg;
        self.ssg_resampler = state.ssg_resampler;
        self.adpcm_a = state.adpcm_a;
        self.adpcm_b = state.adpcm_b;
        self.fidelity = state.fidelity;
        self.address = state.address;
        self.fm_samples_per_output = state.fm_samples_per_output;
        self.last_fm = state.last_fm;
        self.eos_status = state.eos_status;
        self.flag_mask = state.flag_mask;
    }
}

/// OPN2 variant with the 9-bit DAC ladder effect (YM2612).
pub const OPN2_VARIANT_YM2612: u8 = 0;
/// OPN2 variant with a multiplexed 9-bit DAC and no ladder effect (YM3438).
pub const OPN2_VARIANT_YM3438: u8 = 1;
/// OPN2 variant with a properly mixed 14-bit output (YMF276).
pub const OPN2_VARIANT_YMF276: u8 = 2;

save_state::runtime_state! {
/// Yamaha OPN2 family authoritative state and emulator.
///
/// The YM2612, YM3438 and YMF276 share the OPNA FM core and the 9-bit DAC on
/// channel 6. They differ only in how the channel outputs reach the pins.
#[derive(Clone)]
pub struct Opn2Family<const VARIANT: u8> {
    fm: FmEngine<OpnaRegisters>,
    address: u16,
    dac_data: u16,
    dac_enable: bool,
}}

/// Yamaha YM2612 (OPN2) emulator.
pub type Ym2612 = Opn2Family<OPN2_VARIANT_YM2612>;
/// Yamaha YM3438 (OPN2C) emulator.
pub type Ym3438 = Opn2Family<OPN2_VARIANT_YM3438>;
/// Yamaha YMF276 (OPN2L) emulator.
pub type Ymf276 = Opn2Family<OPN2_VARIANT_YMF276>;

impl<const VARIANT: u8> Opn2Family<VARIANT> {
    /// Creates a new OPN2 family instance.
    pub fn new() -> Self {
        Self {
            fm: FmEngine::new(),
            address: 0,
            dac_data: 0,
            dac_enable: false,
        }
    }

    /// Captures the complete chip state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores the complete chip state.
    pub fn restore_state(&mut self, state: Self) -> Result<(), save_state::StateValidationError> {
        save_state::restore_root(self, state, &())
    }

    /// Resets the FM engine. The DAC data and the DAC enable keep their values.
    pub fn reset(&mut self) {
        self.fm.reset();
    }

    /// Returns the output sample rate in Hz for the given `input_clock` in Hz.
    ///
    /// The OPN2 has a fixed prescaler of 6 and 24 operators, so the native FM
    /// rate is `input_clock / (prescale * 24)`.
    pub fn sample_rate(&self, input_clock: u32) -> u32 {
        input_clock / (self.fm.clock_prescale() * OpnaRegisters::OPERATORS as u32)
    }

    /// Reads the chip status register.
    pub fn read_status(&mut self, busy: bool) -> u8 {
        let mut result = self.fm.status();
        if busy {
            result |= OpnaRegisters::STATUS_BUSY;
        }
        result
    }

    /// Reads data from the currently addressed register. The OPN2 data port is
    /// write-only.
    pub fn read_data(&mut self) -> u8 {
        0
    }

    /// Latches the register address for the low bank.
    pub fn write_address(&mut self, data: u8) -> u32 {
        self.address = data as u16;
        0
    }

    /// Writes a value to the previously addressed register (low bank).
    pub fn write_data(&mut self, data: u8) -> u32 {
        // Ignore if paired with the upper address (port 1 data to port 0).
        if helpers::bit(self.address as u32, 8) != 0 {
            return 0;
        }

        match self.address {
            // 2A: DAC data (most significant 8 bits).
            0x2A => self.dac_data = (self.dac_data & !0x1FE) | (((data ^ 0x80) as u16) << 1),
            // 2B: DAC enable (bit 7).
            0x2B => self.dac_enable = helpers::bit(data as u32, 7) != 0,
            // 2C: test register; bit 3 is the low DAC bit.
            0x2C => self.dac_data = (self.dac_data & !1) | helpers::bit(data as u32, 3) as u16,
            // 00-29, 2D-FF: write to FM.
            _ => self.fm.write(self.address, data),
        }

        32 * self.fm.clock_prescale()
    }

    /// Latches the register address for the high bank.
    pub fn write_address_hi(&mut self, data: u8) -> u32 {
        self.address = 0x100 | data as u16;
        0
    }

    /// Writes a value to the previously addressed register (high bank).
    pub fn write_data_hi(&mut self, data: u8) -> u32 {
        // Ignore if paired with the lower address (port 0 data to port 1).
        if helpers::bit(self.address as u32, 8) == 0 {
            return 0;
        }

        // 100-1FF: write to FM.
        self.fm.write(self.address, data);
        32 * self.fm.clock_prescale()
    }

    /// Generates audio samples into `output`.
    ///
    /// Each sample is a stereo `[FM_L, FM_R]` pair.
    pub fn generate(&mut self, output: &mut [YmfmOutput2]) {
        for out in output.iter_mut() {
            self.fm.clock(OpnaRegisters::ALL_CHANNELS);
            out.data = match VARIANT {
                OPN2_VARIANT_YM2612 => self.ladder_output(),
                OPN2_VARIANT_YM3438 => self.multiplexed_output(),
                _ => self.mixed_output(),
            };
        }
    }

    /// Returns the sign-extended 9-bit DAC sample.
    fn dac_value(&self) -> i32 {
        i32::from(((self.dac_data << 7) as i16) >> 7)
    }

    /// Returns the DAC value routed to the left and right outputs of channel 6.
    fn dac_outputs(&self, value: i32, silent: i32) -> [i32; 2] {
        [
            if self.fm.regs.ch_output_0(0x102) != 0 {
                value
            } else {
                silent
            },
            if self.fm.regs.ch_output_1(0x102) != 0 {
                value
            } else {
                silent
            },
        ]
    }

    /// YM2612 output: each channel is clipped to 9 bits and passes the DAC
    /// discontinuity on its own.
    fn ladder_output(&mut self) -> [i32; 2] {
        let mut data = [0i32; 2];
        let last_fm_channel = if self.dac_enable { 5 } else { 6 };
        for channel in 0..last_fm_channel {
            let mut temp = [0i32; 2];
            self.fm.output_mut(&mut temp, 5, 256, 1 << channel);
            data[0] += dac_discontinuity(temp[0]);
            data[1] += dac_discontinuity(temp[1]);
        }

        if self.dac_enable {
            let dac = self.dac_outputs(dac_discontinuity(self.dac_value()), dac_discontinuity(0));
            data[0] += dac[0];
            data[1] += dac[1];
        }

        // The six channels are multiplexed; average them and apply 64/65 to
        // compensate for the discontinuity.
        [
            (data[0] * 128) * 64 / (6 * 65),
            (data[1] * 128) * 64 / (6 * 65),
        ]
    }

    /// YM3438 output: 9-bit channels multiplexed without the discontinuity.
    fn multiplexed_output(&mut self) -> [i32; 2] {
        let mut data = if self.dac_enable {
            self.dac_outputs(self.dac_value(), 0)
        } else {
            [0; 2]
        };
        let channel_mask = if self.dac_enable {
            OpnaRegisters::ALL_CHANNELS ^ (1 << 5)
        } else {
            OpnaRegisters::ALL_CHANNELS
        };
        self.fm.output_mut(&mut data, 5, 256, channel_mask);
        [(data[0] * 128) / 6, (data[1] * 128) / 6]
    }

    /// YMF276 output: 14-bit channels mixed, then shifted down by one bit.
    fn mixed_output(&mut self) -> [i32; 2] {
        let mut data = if self.dac_enable {
            self.dac_outputs(self.dac_value(), 0)
        } else {
            [0; 2]
        };
        let channel_mask = if self.dac_enable {
            OpnaRegisters::ALL_CHANNELS ^ (1 << 5)
        } else {
            OpnaRegisters::ALL_CHANNELS
        };
        self.fm.output_mut(&mut data, 0, 8191, channel_mask);
        [
            helpers::clamp(data[0] >> 1, -32768, 32767),
            helpers::clamp(data[1] >> 1, -32768, 32767),
        ]
    }

    /// Notifies the chip that the specified timer has expired.
    pub fn timer_expired(&mut self, timer_id: u32) {
        self.fm.engine_timer_expired(timer_id);
    }

    /// Returns and clears the pending update for a timer.
    pub fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate> {
        self.fm.take_timer_update(timer_id)
    }

    /// Returns and clears the pending IRQ output update.
    pub fn take_irq_update(&mut self) -> Option<bool> {
        self.fm.take_irq_update()
    }

    /// Returns whether the chip IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.fm.irq_asserted()
    }
}

impl<const VARIANT: u8> Default for Opn2Family<VARIANT> {
    fn default() -> Self {
        Self::new()
    }
}

/// Simulates the YM2612 DAC discontinuity between negative and positive values.
const fn dac_discontinuity(value: i32) -> i32 {
    if value < 0 { value - 3 } else { value + 4 }
}

const Y8950_STATUS_ADPCM_B_PLAYING: u8 = 0x01;
const Y8950_STATUS_ADPCM_B_BRDY: u8 = 0x08;
const Y8950_STATUS_ADPCM_B_EOS: u8 = 0x10;

save_state::runtime_state! {
/// Yamaha YM3526 authoritative state and emulator.
#[derive(Clone)]
pub struct Ym3526 {
    fm: FmEngine<OplRegisters>,
    address: u8,
}}

impl Ym3526 {
    /// Creates a new YM3526 instance.
    pub fn new() -> Self {
        Self {
            fm: FmEngine::new(),
            address: 0,
        }
    }

    /// Captures the complete chip state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores the complete chip state.
    pub fn restore_state(&mut self, state: Self) -> Result<(), save_state::StateValidationError> {
        save_state::restore_root(self, state, &())
    }

    /// Resets the chip to its initial power-on state.
    pub fn reset(&mut self) {
        self.fm.reset();
    }

    /// Returns the output sample rate in Hz for the given `input_clock` in Hz.
    pub fn sample_rate(&self, input_clock: u32) -> u32 {
        input_clock / (OplRegisters::OPERATORS as u32 * self.fm.clock_prescale())
    }

    /// Reads the chip status register.
    pub fn read_status(&mut self) -> u8 {
        self.fm.status() | 0x06
    }

    /// Latches the register address for a subsequent
    /// [`write_data`](Self::write_data).
    pub fn write_address(&mut self, data: u8) -> u32 {
        self.address = data;
        12 * self.fm.clock_prescale()
    }

    /// Writes a value to the previously addressed register.
    pub fn write_data(&mut self, data: u8) -> u32 {
        self.fm.write(self.address as u16, data);
        84 * self.fm.clock_prescale()
    }

    /// Generates audio samples into `output`.
    pub fn generate(&mut self, output: &mut [YmfmOutput1]) {
        for out in output.iter_mut() {
            self.fm.clock(OplRegisters::ALL_CHANNELS);

            out.data = [0];
            self.fm
                .output_mut(&mut out.data, 1, 32767, OplRegisters::ALL_CHANNELS);

            out.data[0] = helpers::roundtrip_fp(out.data[0]) as i32;
        }
    }

    /// Notifies the chip that the specified timer has expired.
    pub fn timer_expired(&mut self, timer_id: u32) {
        self.fm.engine_timer_expired(timer_id);
    }

    /// Returns and clears the pending update for a timer.
    pub fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate> {
        self.fm.take_timer_update(timer_id)
    }

    /// Returns and clears the pending IRQ output update.
    pub fn take_irq_update(&mut self) -> Option<bool> {
        self.fm.take_irq_update()
    }

    /// Returns whether the chip IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.fm.irq_asserted()
    }
}

save_state::runtime_state! {
/// Yamaha Y8950 authoritative state and emulator.
#[derive(Clone)]
pub struct Y8950 {
    fm: FmEngine<OplRegisters>,
    adpcm_b: AdpcmBEngine,
    adpcm_memory: Option<Vec<u8>>,
    address: u8,
    io_ddr: u8,
    io_input: [u8; 2],
    io_output: [Option<u8>; 2],
}}

impl Y8950 {
    /// Creates a new Y8950 instance.
    pub fn new() -> Self {
        Self {
            fm: FmEngine::new(),
            adpcm_b: AdpcmBEngine::new(0),
            adpcm_memory: None,
            address: 0,
            io_ddr: 0,
            io_input: [0; 2],
            io_output: [None; 2],
        }
    }

    /// Captures the complete chip and ADPCM memory state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores the complete chip and ADPCM memory state.
    pub fn restore_state(&mut self, state: Self) -> Result<(), save_state::StateValidationError> {
        save_state::restore_root(self, state, &())
    }

    /// Resets the chip to its initial power-on state.
    pub fn reset(&mut self) {
        self.fm.reset();
        self.adpcm_b.reset();
    }

    /// Replaces ADPCM memory with `data`.
    ///
    /// Panics if `data` is empty. Use [`clear_adpcm_memory`](Self::clear_adpcm_memory)
    /// to remove ADPCM memory.
    pub fn set_adpcm_memory(&mut self, data: Vec<u8>) {
        assert!(!data.is_empty(), "ADPCM memory must not be empty");
        self.adpcm_memory = Some(data);
    }

    /// Clears ADPCM memory. Reads return zero and writes are ignored.
    pub fn clear_adpcm_memory(&mut self) {
        self.adpcm_memory = None;
    }

    /// Returns ADPCM memory, or an empty slice when no memory is installed.
    pub fn adpcm_memory(&self) -> &[u8] {
        self.adpcm_memory.as_deref().unwrap_or(&[])
    }

    /// Returns mutable ADPCM memory when installed.
    pub fn adpcm_memory_mut(&mut self) -> Option<&mut [u8]> {
        self.adpcm_memory.as_deref_mut()
    }

    /// Sets the latched value read from an I/O port.
    pub fn set_io_input(&mut self, port: u8, value: u8) {
        if let Some(input) = self.io_input.get_mut(port as usize) {
            *input = value;
        }
    }

    /// Returns and clears a latched output value for an I/O port.
    pub fn take_io_output(&mut self, port: u8) -> Option<u8> {
        self.io_output.get_mut(port as usize).and_then(Option::take)
    }

    /// Returns the output sample rate in Hz for the given `input_clock` in Hz.
    pub fn sample_rate(&self, input_clock: u32) -> u32 {
        input_clock / (OplRegisters::OPERATORS as u32 * self.fm.clock_prescale())
    }

    /// Reads the chip status register.
    pub fn read_status(&mut self) -> u8 {
        let mut status = self.fm.status()
            & !(Y8950_STATUS_ADPCM_B_EOS
                | Y8950_STATUS_ADPCM_B_BRDY
                | Y8950_STATUS_ADPCM_B_PLAYING);

        let adpcm_status = self.adpcm_b.status() as u32;
        if adpcm_status & AdpcmBChannel::STATUS_EOS != 0 {
            status |= Y8950_STATUS_ADPCM_B_EOS;
        }
        if adpcm_status & AdpcmBChannel::STATUS_BRDY != 0 {
            status |= Y8950_STATUS_ADPCM_B_BRDY;
        }
        if adpcm_status & AdpcmBChannel::STATUS_PLAYING != 0 {
            status |= Y8950_STATUS_ADPCM_B_PLAYING;
        }

        self.fm.set_reset_status(status, !status)
    }

    /// Reads back data from the chip.
    pub fn read_data(&mut self) -> u8 {
        match self.address {
            0x05 => {
                // keyboard in
                self.io_input[1]
            }
            0x09 | 0x1A => {
                // ADPCM data
                self.adpcm_b
                    .read(self.address as u32 - 0x07, self.adpcm_memory.as_deref_mut())
                    as u8
            }
            0x19 => {
                // I/O data
                self.io_input[0]
            }
            _ => 0xFF,
        }
    }

    /// Latches the register address for a subsequent
    /// [`write_data`](Self::write_data) or [`read_data`](Self::read_data).
    pub fn write_address(&mut self, data: u8) -> u32 {
        self.address = data;
        12 * self.fm.clock_prescale()
    }

    /// Writes a value to the previously addressed register.
    pub fn write_data(&mut self, data: u8) -> u32 {
        let busy_clocks = if self.address <= 0x1A { 12 } else { 84 } * self.fm.clock_prescale();

        match self.address {
            0x04 => {
                // IRQ control
                self.fm.write(self.address as u16, data);
                if (data & Y8950_STATUS_ADPCM_B_EOS) != 0 {
                    self.adpcm_b.clear_status(AdpcmBChannel::STATUS_EOS);
                }
                self.read_status();
            }
            0x06 => {
                // keyboard out
                self.io_output[1] = Some(data);
            }
            0x08 => {
                // split FM/ADPCM-B
                self.adpcm_b.write(
                    self.address as u32 - 0x07,
                    (data & 0x0F) | 0x80,
                    self.adpcm_memory.as_deref_mut(),
                );
                self.fm.write(self.address as u16, data & 0xC0);
            }
            0x07 | 0x09..=0x12 | 0x15..=0x17 => {
                // ADPCM-B registers
                self.adpcm_b.write(
                    self.address as u32 - 0x07,
                    data,
                    self.adpcm_memory.as_deref_mut(),
                );
            }
            0x18 => {
                // I/O direction
                self.io_ddr = data & 0x0F;
            }
            0x19 => {
                // I/O data
                self.io_output[0] = Some(data & self.io_ddr);
            }
            _ => {
                // everything else to FM
                self.fm.write(self.address as u16, data);
            }
        }
        busy_clocks
    }

    /// Generates audio samples into `output`.
    pub fn generate(&mut self, output: &mut [YmfmOutput1]) {
        for out in output.iter_mut() {
            self.fm.clock(OplRegisters::ALL_CHANNELS);
            self.adpcm_b.clock(self.adpcm_memory.as_deref_mut());

            out.data = [0];
            self.fm
                .output_mut(&mut out.data, 1, 32767, OplRegisters::ALL_CHANNELS);

            // mix in the ADPCM-B; mono output, shift by 3
            self.adpcm_b.output::<1>(&mut out.data, 3);

            out.data[0] = helpers::roundtrip_fp(out.data[0]) as i32;
        }
    }

    /// Notifies the chip that the specified timer has expired.
    pub fn timer_expired(&mut self, timer_id: u32) {
        self.fm.engine_timer_expired(timer_id);
    }

    /// Returns and clears the pending update for a timer.
    pub fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate> {
        self.fm.take_timer_update(timer_id)
    }

    /// Returns and clears the pending IRQ output update.
    pub fn take_irq_update(&mut self) -> Option<bool> {
        self.fm.take_irq_update()
    }

    /// Returns whether the chip IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.fm.irq_asserted()
    }
}

save_state::runtime_state! {
/// Yamaha YM3812 authoritative state and emulator.
#[derive(Clone)]
pub struct Ym3812 {
    fm: FmEngine<Opl2Registers>,
    address: u8,
}}

impl Ym3812 {
    /// Creates a new YM3812 instance.
    pub fn new() -> Self {
        Self {
            fm: FmEngine::new(),
            address: 0,
        }
    }

    /// Captures the complete chip state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores the complete chip state.
    pub fn restore_state(&mut self, state: Self) -> Result<(), save_state::StateValidationError> {
        save_state::restore_root(self, state, &())
    }

    /// Resets the chip to its initial power-on state.
    pub fn reset(&mut self) {
        self.fm.reset();
    }

    /// Returns the output sample rate in Hz for the given `input_clock` in Hz.
    pub fn sample_rate(&self, input_clock: u32) -> u32 {
        input_clock / (Opl2Registers::OPERATORS as u32 * self.fm.clock_prescale())
    }

    /// Reads the chip status register.
    pub fn read_status(&mut self) -> u8 {
        self.fm.status() | 0x06
    }

    /// Latches the register address for a subsequent
    /// [`write_data`](Self::write_data).
    pub fn write_address(&mut self, data: u8) -> u32 {
        self.address = data;
        12 * self.fm.clock_prescale()
    }

    /// Writes a value to the previously addressed register.
    pub fn write_data(&mut self, data: u8) -> u32 {
        self.fm.write(self.address as u16, data);
        84 * self.fm.clock_prescale()
    }

    /// Generates audio samples into `output`.
    pub fn generate(&mut self, output: &mut [YmfmOutput1]) {
        for out in output.iter_mut() {
            self.fm.clock(Opl2Registers::ALL_CHANNELS);

            out.data = [0];
            self.fm
                .output_mut(&mut out.data, 1, 32767, Opl2Registers::ALL_CHANNELS);

            out.data[0] = helpers::roundtrip_fp(out.data[0]) as i32;
        }
    }

    /// Notifies the chip that the specified timer has expired.
    pub fn timer_expired(&mut self, timer_id: u32) {
        self.fm.engine_timer_expired(timer_id);
    }

    /// Returns and clears the pending update for a timer.
    pub fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate> {
        self.fm.take_timer_update(timer_id)
    }

    /// Returns and clears the pending IRQ output update.
    pub fn take_irq_update(&mut self) -> Option<bool> {
        self.fm.take_irq_update()
    }

    /// Returns whether the chip IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.fm.irq_asserted()
    }
}

save_state::runtime_state! {
/// Yamaha YMF262 authoritative state and emulator.
#[derive(Clone)]
pub struct Ymf262 {
    fm: FmEngine<Opl3Registers>,
    address: u16,
}}

impl Ymf262 {
    /// Creates a new YMF262 instance.
    pub fn new() -> Self {
        Self {
            fm: FmEngine::new(),
            address: 0,
        }
    }

    /// Captures the complete chip state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores the complete chip state.
    pub fn restore_state(&mut self, state: Self) -> Result<(), save_state::StateValidationError> {
        save_state::restore_root(self, state, &())
    }

    /// Resets the chip to its initial power-on state.
    pub fn reset(&mut self) {
        self.fm.reset();
    }

    /// Returns the output sample rate in Hz for the given `input_clock` in Hz.
    pub fn sample_rate(&self, input_clock: u32) -> u32 {
        input_clock / (Opl3Registers::OPERATORS as u32 * self.fm.clock_prescale())
    }

    /// Reads the chip status register.
    pub fn read_status(&mut self) -> u8 {
        self.fm.status()
    }

    /// Latches the register address for the low bank (0x00–0xFF).
    pub fn write_address(&mut self, data: u8) -> u32 {
        self.address = data as u16;
        32 * self.fm.clock_prescale()
    }

    /// Writes a value to the previously addressed register.
    pub fn write_data(&mut self, data: u8) -> u32 {
        self.fm.write(self.address, data);
        32 * self.fm.clock_prescale()
    }

    /// Latches the register address for the high bank (0x100–0x1FF).
    pub fn write_address_hi(&mut self, data: u8) -> u32 {
        self.address = data as u16 | 0x100;

        // in compatibility mode, upper bit is masked except for register 0x105
        if self.fm.regs.newflag() == 0 && self.address != 0x105 {
            self.address &= 0xFF;
        }
        32 * self.fm.clock_prescale()
    }

    /// Generates audio samples into `output`.
    pub fn generate(&mut self, output: &mut [YmfmOutput4]) {
        for out in output.iter_mut() {
            self.fm.clock(Opl3Registers::ALL_CHANNELS);

            out.data = [0; 4];
            self.fm
                .output_mut(&mut out.data, 0, 32767, Opl3Registers::ALL_CHANNELS);

            // YMF262 output is 16-bit; clamp to 16-bit range
            for v in &mut out.data {
                *v = (*v).clamp(-32768, 32767);
            }
        }
    }

    /// Notifies the chip that the specified timer has expired.
    pub fn timer_expired(&mut self, timer_id: u32) {
        self.fm.engine_timer_expired(timer_id);
    }

    /// Returns and clears the pending update for a timer.
    pub fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate> {
        self.fm.take_timer_update(timer_id)
    }

    /// Returns and clears the pending IRQ output update.
    pub fn take_irq_update(&mut self) -> Option<bool> {
        self.fm.take_irq_update()
    }

    /// Returns whether the chip IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.fm.irq_asserted()
    }
}

/// Status bits the YMF289B sets while busy in YMF289B mode.
const YMF289B_STATUS_BUSY_FLAGS: u8 = 0x05;
/// Busy time in input clocks of every YMF289B register access.
const YMF289B_BUSY_CLOCKS: u32 = 56;

save_state::runtime_state! {
/// Yamaha YMF289B (OPL3L) authoritative state and emulator.
///
/// The YMF289B is a YMF262 with a power down mode, a bulk register clear, a
/// busy flag in the status register, shorter busy times, readable registers
/// and only two of the four outputs.
#[derive(Clone)]
pub struct Ymf289b {
    fm: FmEngine<Opl3Registers>,
    address: u16,
}}

impl Ymf289b {
    /// Creates a new YMF289B instance.
    pub fn new() -> Self {
        Self {
            fm: FmEngine::new(),
            address: 0,
        }
    }

    /// Captures the complete chip state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores the complete chip state.
    pub fn restore_state(&mut self, state: Self) -> Result<(), save_state::StateValidationError> {
        save_state::restore_root(self, state, &())
    }

    /// Resets the chip to its initial power-on state.
    pub fn reset(&mut self) {
        self.fm.reset();
    }

    /// Returns the output sample rate in Hz for the given `input_clock` in Hz.
    pub fn sample_rate(&self, input_clock: u32) -> u32 {
        input_clock / (Opl3Registers::OPERATORS as u32 * self.fm.clock_prescale())
    }

    /// Reads the chip status register. In YMF289B mode the busy flags are set
    /// while `busy` is true.
    pub fn read_status(&mut self, busy: bool) -> u8 {
        let mut result = self.fm.status();
        if self.ymf289b_mode() && busy {
            result |= YMF289B_STATUS_BUSY_FLAGS;
        }
        result
    }

    /// Reads back the addressed register in YMF289B mode. Returns 0xFF otherwise.
    pub fn read_data(&mut self) -> u8 {
        if self.ymf289b_mode() {
            self.fm.regs.read(self.address)
        } else {
            0xFF
        }
    }

    /// Latches the register address for the low bank (0x00-0xFF).
    pub fn write_address(&mut self, data: u8) -> u32 {
        self.address = data as u16;
        YMF289B_BUSY_CLOCKS
    }

    /// Writes a value to the previously addressed register.
    pub fn write_data(&mut self, data: u8) -> u32 {
        self.fm.write(self.address, data);

        // Writes to 0x108 with the CLR flag set clear the registers.
        if self.address == 0x108 && helpers::bit(data as u32, 2) != 0 {
            self.fm.regs.reset();
        }
        YMF289B_BUSY_CLOCKS
    }

    /// Latches the register address for the high bank (0x100-0x1FF).
    pub fn write_address_hi(&mut self, data: u8) -> u32 {
        self.address = data as u16 | 0x100;

        // in compatibility mode, upper bit is masked except for register 0x105
        if self.fm.regs.newflag() == 0 && self.address != 0x105 {
            self.address &= 0xFF;
        }
        YMF289B_BUSY_CLOCKS
    }

    /// Generates audio samples into `output`.
    ///
    /// Each sample holds the two exposed outputs of the four OPL3 outputs.
    pub fn generate(&mut self, output: &mut [YmfmOutput2]) {
        for out in output.iter_mut() {
            self.fm.clock(Opl3Registers::ALL_CHANNELS);

            let mut full = [0i32; 4];
            self.fm
                .output_mut(&mut full, 0, 32767, Opl3Registers::ALL_CHANNELS);
            out.data = [full[0].clamp(-32768, 32767), full[1].clamp(-32768, 32767)];
        }
    }

    /// Notifies the chip that the specified timer has expired.
    pub fn timer_expired(&mut self, timer_id: u32) {
        self.fm.engine_timer_expired(timer_id);
    }

    /// Returns and clears the pending update for a timer.
    pub fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate> {
        self.fm.take_timer_update(timer_id)
    }

    /// Returns and clears the pending IRQ output update.
    pub fn take_irq_update(&mut self) -> Option<bool> {
        self.fm.take_irq_update()
    }

    /// Returns whether the chip IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.fm.irq_asserted()
    }

    /// Returns whether YMF289B mode (register 0x105 bit 2) is enabled.
    fn ymf289b_mode(&self) -> bool {
        self.fm.regs.read(0x105) & 0x04 != 0
    }
}

impl Default for Ymf289b {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for Ym2203 {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for Ym2608 {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for Ym3526 {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for Y8950 {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for Ym3812 {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for Ymf262 {
    fn default() -> Self {
        Self::new()
    }
}

save_state::runtime_state! {
/// Yamaha YM2151 authoritative state and emulator.
#[derive(Clone)]
pub struct Ym2151 {
    fm: FmEngine<OpmRegisters>,
    address: u8,
    ct_state: u8,
    ct_update: Option<u8>,
}}

impl Ym2151 {
    /// Creates a new YM2151 instance.
    pub fn new() -> Self {
        Self {
            fm: FmEngine::new(),
            address: 0,
            ct_state: 0,
            ct_update: None,
        }
    }

    /// Captures the complete chip state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores the complete chip state.
    pub fn restore_state(&mut self, state: Self) -> Result<(), save_state::StateValidationError> {
        save_state::restore_root(self, state, &())
    }

    /// Resets the FM engine. The CT output lines keep their levels.
    pub fn reset(&mut self) {
        self.fm.reset();
    }

    /// Returns the output sample rate in Hz for the given `input_clock` in Hz.
    ///
    /// The OPM has a fixed prescaler of 2 and 32 operators, so the native FM
    /// rate is `input_clock / (prescale * 32)`.
    pub fn sample_rate(&self, input_clock: u32) -> u32 {
        input_clock / (OpmRegisters::OPERATORS as u32 * self.fm.clock_prescale())
    }

    /// Reads the chip status register.
    pub fn read_status(&mut self, busy: bool) -> u8 {
        let mut result =
            self.fm.status() & (OpmRegisters::STATUS_TIMERA | OpmRegisters::STATUS_TIMERB);
        if busy {
            result |= OpmRegisters::STATUS_BUSY;
        }
        result
    }

    /// Latches the register address for a subsequent
    /// [`write_data`](Self::write_data).
    pub fn write_address(&mut self, data: u8) -> u32 {
        self.address = data;
        0
    }

    /// Writes a value to the previously addressed register.
    pub fn write_data(&mut self, data: u8) -> u32 {
        if self.address == 0x1B {
            let ct = (data >> 6) & 3;
            if ct != self.ct_state {
                self.ct_state = ct;
                self.ct_update = Some(ct);
            }
        }
        self.fm.write(self.address as u16, data);
        32 * self.fm.clock_prescale()
    }

    /// Generates audio samples into `output`.
    ///
    /// Each sample is a stereo `[left, right]` pair.
    pub fn generate(&mut self, output: &mut [YmfmOutput2]) {
        for out in output.iter_mut() {
            self.fm.clock(OpmRegisters::ALL_CHANNELS);

            // OPM is full 14-bit with no intermediate clipping
            out.data = [0; 2];
            self.fm
                .output_mut(&mut out.data, 0, 32767, OpmRegisters::ALL_CHANNELS);

            // the YM2151 uses an external DAC (YM3012) with mantissa/exponent
            // format; simulate the truncation with a 10.3 float round trip
            out.data[0] = helpers::roundtrip_fp(out.data[0]) as i32;
            out.data[1] = helpers::roundtrip_fp(out.data[1]) as i32;
        }
    }

    /// Notifies the chip that the specified timer has expired.
    pub fn timer_expired(&mut self, timer_id: u32) {
        self.fm.engine_timer_expired(timer_id);
    }

    /// Returns and clears the pending update for a timer.
    pub fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate> {
        self.fm.take_timer_update(timer_id)
    }

    /// Returns and clears the pending IRQ output update.
    pub fn take_irq_update(&mut self) -> Option<bool> {
        self.fm.take_irq_update()
    }

    /// Returns whether the chip IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.fm.irq_asserted()
    }

    /// Returns the current CT output pair; bit 0 is register 0x1B bit 6 and
    /// bit 1 is register 0x1B bit 7.
    pub fn ct_state(&self) -> u8 {
        self.ct_state
    }

    /// Returns and clears the pending CT output update.
    pub fn take_ct_update(&mut self) -> Option<u8> {
        self.ct_update.take()
    }
}

impl Default for Ym2151 {
    fn default() -> Self {
        Self::new()
    }
}

/// Yamaha YM2164 (OPP). It behaves exactly like the YM2151.
pub type Ym2164 = Ym2151;

save_state::runtime_state! {
/// Yamaha YM2149 (SSG) authoritative state and emulator.
#[derive(Clone)]
pub struct Ym2149 {
    ssg: SsgEngine,
    address: u8,
    io_input: [u8; 2],
    io_output: [Option<u8>; 2],
}}

impl Ym2149 {
    /// Creates a new YM2149 instance.
    ///
    /// The chip is not automatically reset; call [`reset`](Self::reset)
    /// before first use.
    pub fn new() -> Self {
        Self {
            ssg: SsgEngine::new(),
            address: 0,
            io_input: [0; 2],
            io_output: [None; 2],
        }
    }

    /// Captures the complete chip state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores the complete chip state.
    pub fn restore_state(&mut self, state: Self) -> Result<(), save_state::StateValidationError> {
        save_state::restore_root(self, state, &())
    }

    /// Resets the SSG engine. The address latch keeps its value.
    pub fn reset(&mut self) {
        self.ssg.reset();
    }

    /// Returns the output sample rate in Hz for the given `input_clock` in Hz.
    pub fn sample_rate(&self, input_clock: u32) -> u32 {
        input_clock / 8 / 8
    }

    /// Sets the value read back from a parallel I/O port when that port is
    /// configured as an input. Port 0 is A, port 1 is B.
    pub fn set_io_input(&mut self, port: u8, value: u8) {
        if let Some(input) = self.io_input.get_mut(port as usize) {
            *input = value;
        }
    }

    /// Returns and clears the last value written to a parallel I/O port that
    /// is configured as an output. Port 0 is A, port 1 is B.
    pub fn take_io_output(&mut self, port: u8) -> Option<u8> {
        self.io_output.get_mut(port as usize).and_then(Option::take)
    }

    /// Reads the currently addressed register.
    pub fn read_data(&mut self) -> u8 {
        let register = self.address as u32 & 0x0F;
        if register == 0x0E && self.ssg.read(0x07) & 0x40 == 0 {
            return self.io_input[0];
        }
        if register == 0x0F && self.ssg.read(0x07) & 0x80 == 0 {
            return self.io_input[1];
        }
        self.ssg.read(register)
    }

    /// Reads through the bus interface. Offset bits 1 and 0 are BC2 and BC1.
    pub fn read(&mut self, offset: u32) -> u8 {
        match offset & 3 {
            3 => self.read_data(),
            _ => 0xFF,
        }
    }

    /// Latches the register address for a subsequent
    /// [`write_data`](Self::write_data) or [`read_data`](Self::read_data).
    pub fn write_address(&mut self, data: u8) {
        self.address = data;
    }

    /// Writes a value to the previously addressed register.
    pub fn write_data(&mut self, data: u8) {
        let register = self.address as u32 & 0x0F;
        self.ssg.write(register, data);
        if register == 0x0E && self.ssg.read(0x07) & 0x40 != 0 {
            self.io_output[0] = Some(data);
        } else if register == 0x0F && self.ssg.read(0x07) & 0x80 != 0 {
            self.io_output[1] = Some(data);
        }
    }

    /// Writes through the bus interface. Offset bits 1 and 0 are BC2 and BC1.
    pub fn write(&mut self, offset: u32, data: u8) {
        match offset & 3 {
            0 | 3 => self.write_address(data),
            2 => self.write_data(data),
            _ => {}
        }
    }

    /// Generates audio samples into `output`.
    ///
    /// Each sample holds the three unmixed SSG channels `[A, B, C]`.
    pub fn generate(&mut self, output: &mut [YmfmOutput3]) {
        let mut sample = SsgOutput { data: [0; 3] };
        for out in output.iter_mut() {
            self.ssg.clock();
            self.ssg.output(&mut sample);
            out.data = sample.data;
        }
    }
}

impl Default for Ym2149 {
    fn default() -> Self {
        Self::new()
    }
}

save_state::runtime_state! {
/// Yamaha YM3806 (OPQ) authoritative state and emulator.
#[derive(Clone)]
pub struct Ym3806 {
    fm: FmEngine<OpqRegisters>,
}}

impl Ym3806 {
    /// Creates a new YM3806 instance.
    ///
    /// The chip is not automatically reset; call [`reset`](Self::reset)
    /// before first use.
    pub fn new() -> Self {
        Self {
            fm: FmEngine::new(),
        }
    }

    /// Captures the complete chip state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores the complete chip state.
    pub fn restore_state(&mut self, state: Self) -> Result<(), save_state::StateValidationError> {
        save_state::restore_root(self, state, &())
    }

    /// Resets the chip to its initial power-on state.
    pub fn reset(&mut self) {
        self.fm.reset();
    }

    /// Returns the output sample rate in Hz for the given `input_clock` in Hz.
    pub fn sample_rate(&self, input_clock: u32) -> u32 {
        input_clock / (OpqRegisters::OPERATORS as u32 * self.fm.clock_prescale())
    }

    /// Reads the chip status register. Bit 2 is the timer B flag.
    pub fn read_status(&mut self, busy: bool) -> u8 {
        let mut result = self.fm.status();
        if busy {
            result |= OpqRegisters::STATUS_BUSY;
        }
        result
    }

    /// Reads through the bus interface. Offset 0 is the status port and
    /// every other offset reads 0xFF.
    pub fn read(&mut self, offset: u32, busy: bool) -> u8 {
        match offset {
            0 => self.read_status(busy),
            _ => 0xFF,
        }
    }

    /// Writes `data` directly to the register at `address`.
    ///
    /// The chip has no address latch. Every register write takes its
    /// address from the bus.
    pub fn write(&mut self, address: u8, data: u8) {
        self.fm.write(address as u16, data);
    }

    /// Generates audio samples into `output`.
    ///
    /// Each sample is a stereo `[left, right]` pair.
    pub fn generate(&mut self, output: &mut [YmfmOutput2]) {
        for out in output.iter_mut() {
            self.fm.clock(OpqRegisters::ALL_CHANNELS);

            // OPQ is full 14-bit with no intermediate clipping
            out.data = [0; 2];
            self.fm
                .output_mut(&mut out.data, 0, 32767, OpqRegisters::ALL_CHANNELS);

            // the YM3806 feeds a YM3012 DAC; simulate its 10.3 float truncation
            out.data[0] = helpers::roundtrip_fp(out.data[0]) as i32;
            out.data[1] = helpers::roundtrip_fp(out.data[1]) as i32;
        }
    }

    /// Notifies the chip that the specified timer has expired.
    pub fn timer_expired(&mut self, timer_id: u32) {
        self.fm.engine_timer_expired(timer_id);
    }

    /// Returns and clears the pending update for a timer.
    pub fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate> {
        self.fm.take_timer_update(timer_id)
    }

    /// Returns and clears the pending IRQ output update.
    pub fn take_irq_update(&mut self) -> Option<bool> {
        self.fm.take_irq_update()
    }

    /// Returns whether the chip IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.fm.irq_asserted()
    }
}

impl Default for Ym3806 {
    fn default() -> Self {
        Self::new()
    }
}

/// Yamaha YM3533 (OPQ). It behaves exactly like the YM3806.
pub type Ym3533 = Ym3806;

save_state::runtime_state! {
/// Yamaha YM2414 (OPZ) authoritative state and emulator.
#[derive(Clone)]
pub struct Ym2414 {
    fm: FmEngine<OpzRegisters>,
    address: u8,
    ct_state: u8,
    ct_update: Option<u8>,
}}

impl Ym2414 {
    /// Creates a new YM2414 instance.
    ///
    /// The chip is not automatically reset; call [`reset`](Self::reset)
    /// before first use.
    pub fn new() -> Self {
        Self {
            fm: FmEngine::new(),
            address: 0,
            ct_state: 0,
            ct_update: None,
        }
    }

    /// Captures the complete chip state.
    pub fn capture_state(&self) -> Self {
        self.clone()
    }

    /// Restores the complete chip state.
    pub fn restore_state(&mut self, state: Self) -> Result<(), save_state::StateValidationError> {
        save_state::restore_root(self, state, &())
    }

    /// Resets the FM engine. The CT output lines and the address latch keep
    /// their values.
    pub fn reset(&mut self) {
        self.fm.reset();
    }

    /// Returns the output sample rate in Hz for the given `input_clock` in Hz.
    pub fn sample_rate(&self, input_clock: u32) -> u32 {
        input_clock / (OpzRegisters::OPERATORS as u32 * self.fm.clock_prescale())
    }

    /// Reads the chip status register.
    ///
    /// Bit 0 = Timer A flag, bit 1 = Timer B flag, bit 7 = busy flag.
    pub fn read_status(&mut self, busy: bool) -> u8 {
        let mut result = self.fm.status();
        if busy {
            result |= OpzRegisters::STATUS_BUSY;
        }
        result
    }

    /// Reads through the bus interface. Odd offsets read the status and even
    /// offsets read 0xFF.
    pub fn read(&mut self, offset: u32, busy: bool) -> u8 {
        match offset & 1 {
            1 => self.read_status(busy),
            _ => 0xFF,
        }
    }

    /// Latches the register address for a subsequent
    /// [`write_data`](Self::write_data).
    pub fn write_address(&mut self, data: u8) -> u32 {
        self.address = data;
        0
    }

    /// Writes a value to the previously addressed register.
    pub fn write_data(&mut self, data: u8) -> u32 {
        self.fm.write(self.address as u16, data);
        if self.address == 0x1B {
            let ct = data >> 6;
            if ct != self.ct_state {
                self.ct_state = ct;
                self.ct_update = Some(ct);
            }
        }
        32 * self.fm.clock_prescale()
    }

    /// Writes through the bus interface. Even offsets write the address and
    /// odd offsets write the data.
    pub fn write(&mut self, offset: u32, data: u8) -> u32 {
        match offset & 1 {
            0 => self.write_address(data),
            _ => self.write_data(data),
        }
    }

    /// Generates audio samples into `output`.
    ///
    /// Each sample is a stereo `[left, right]` pair.
    pub fn generate(&mut self, output: &mut [YmfmOutput2]) {
        for out in output.iter_mut() {
            self.fm.clock(OpzRegisters::ALL_CHANNELS);

            // OPZ is full 14-bit with no intermediate clipping
            out.data = [0; 2];
            self.fm
                .output_mut(&mut out.data, 0, 32767, OpzRegisters::ALL_CHANNELS);

            // simulate the 10.3 float truncation of a YM3012 style DAC
            out.data[0] = helpers::roundtrip_fp(out.data[0]) as i32;
            out.data[1] = helpers::roundtrip_fp(out.data[1]) as i32;
        }
    }

    /// Notifies the chip that the specified timer has expired.
    pub fn timer_expired(&mut self, timer_id: u32) {
        self.fm.engine_timer_expired(timer_id);
    }

    /// Returns and clears the pending update for a timer.
    pub fn take_timer_update(&mut self, timer_id: u8) -> Option<YmfmTimerUpdate> {
        self.fm.take_timer_update(timer_id)
    }

    /// Returns and clears the pending IRQ output update.
    pub fn take_irq_update(&mut self) -> Option<bool> {
        self.fm.take_irq_update()
    }

    /// Returns whether the chip IRQ output is currently asserted.
    pub fn irq_asserted(&self) -> bool {
        self.fm.irq_asserted()
    }

    /// Returns the current CT output pair; bit 0 is register 0x1B bit 6 and
    /// bit 1 is register 0x1B bit 7.
    pub fn ct_state(&self) -> u8 {
        self.ct_state
    }

    /// Returns and clears the pending CT output update.
    pub fn take_ct_update(&mut self) -> Option<u8> {
        self.ct_update.take()
    }
}

impl Default for Ym2414 {
    fn default() -> Self {
        Self::new()
    }
}

impl save_state::ValidateState for Ym2149 {
    fn validate_state(&self, _context: &()) -> Result<(), save_state::StateValidationError> {
        Ok(())
    }
}

impl save_state::AfterRestore for Ym2149 {
    fn after_restore(&mut self) {}
}

impl save_state::RestoreTarget for Ym2149 {
    type State = Self;
    type ValidationContext = ();

    fn replace_state(&mut self, state: Self::State) {
        *self = state;
    }
}

impl save_state::ValidateState<save_state::ResourceIdentity> for Ym2608State {
    fn validate_state(
        &self,
        current_rom_identity: &save_state::ResourceIdentity,
    ) -> Result<(), save_state::StateValidationError> {
        if &self.adpcm_a_rom_identity != current_rom_identity {
            return Err(save_state::StateValidationError::new(
                "YM2608 rhythm ROM identity differs",
            ));
        }
        if self.fm.operators.len() != OpnaRegisters::OPERATORS
            || self.fm.channels.len() != OpnaRegisters::CHANNELS
            || self.adpcm_b_ram.as_ref().is_some_and(Vec::is_empty)
        {
            return Err(save_state::StateValidationError::new(
                "YM2608 state topology is invalid",
            ));
        }
        Ok(())
    }
}

impl save_state::AfterRestore for Ym2608 {
    fn after_restore(&mut self) {}
}

impl save_state::RestoreTarget for Ym2608 {
    type State = Ym2608State;
    type ValidationContext = save_state::ResourceIdentity;

    fn replace_state(&mut self, state: Self::State) {
        self.fm = state.fm;
        self.ssg = state.ssg;
        self.ssg_resampler = state.ssg_resampler;
        self.adpcm_a = state.adpcm_a;
        self.adpcm_b = state.adpcm_b;
        self.adpcm_b_ram = state.adpcm_b_ram;
        self.fidelity = state.fidelity;
        self.address = state.address;
        self.fm_samples_per_output = state.fm_samples_per_output;
        self.last_fm = state.last_fm;
        self.irq_enable = state.irq_enable;
        self.flag_control = state.flag_control;
    }
}

impl save_state::ValidateState for Ym2203 {
    fn validate_state(&self, _context: &()) -> Result<(), save_state::StateValidationError> {
        if self.fm.operators.len() != OpnRegisters::OPERATORS
            || self.fm.channels.len() != OpnRegisters::CHANNELS
        {
            return Err(save_state::StateValidationError::new(
                "YM2203 engine topology is invalid",
            ));
        }
        Ok(())
    }
}

impl save_state::AfterRestore for Ym2203 {
    fn after_restore(&mut self) {}
}

impl save_state::RestoreTarget for Ym2203 {
    type State = Self;
    type ValidationContext = ();

    fn replace_state(&mut self, state: Self::State) {
        *self = state;
    }
}

impl save_state::ValidateState for Ymf262 {
    fn validate_state(&self, _context: &()) -> Result<(), save_state::StateValidationError> {
        if self.fm.operators.len() != Opl3Registers::OPERATORS
            || self.fm.channels.len() != Opl3Registers::CHANNELS
        {
            return Err(save_state::StateValidationError::new(
                "YMF262 engine topology is invalid",
            ));
        }
        Ok(())
    }
}

impl save_state::AfterRestore for Ymf262 {
    fn after_restore(&mut self) {}
}

impl save_state::RestoreTarget for Ymf262 {
    type State = Self;
    type ValidationContext = ();

    fn replace_state(&mut self, state: Self::State) {
        *self = state;
    }
}

/// Adds validated direct save-state replacement to a complete YMFM chip.
///
/// Use this for chips whose object is entirely authoritative and whose only
/// decoded invariant is the fixed operator and channel topology.
macro_rules! impl_direct_chip_restore {
    (@impl [$($generics:tt)*] $chip:ty, $registers:ty, $name:literal) => {
        impl<$($generics)*> save_state::ValidateState for $chip {
            fn validate_state(
                &self,
                _context: &(),
            ) -> Result<(), save_state::StateValidationError> {
                if self.fm.operators.len() != <$registers>::OPERATORS
                    || self.fm.channels.len() != <$registers>::CHANNELS
                {
                    return Err(save_state::StateValidationError::new(concat!(
                        $name,
                        " engine topology is invalid"
                    )));
                }
                Ok(())
            }
        }

        impl<$($generics)*> save_state::AfterRestore for $chip {
            fn after_restore(&mut self) {}
        }

        impl<$($generics)*> save_state::RestoreTarget for $chip {
            type State = Self;
            type ValidationContext = ();

            fn replace_state(&mut self, state: Self::State) {
                *self = state;
            }
        }
    };
    ($chip:ident<const $parameter:ident: $parameter_type:ty>, $registers:ty, $name:literal) => {
        impl_direct_chip_restore!(
            @impl [const $parameter: $parameter_type] $chip<$parameter>, $registers, $name
        );
    };
    ($chip:ty, $registers:ty, $name:literal) => {
        impl_direct_chip_restore!(@impl [] $chip, $registers, $name);
    };
}

impl_direct_chip_restore!(Opn2Family<const VARIANT: u8>, OpnaRegisters, "OPN2");
impl_direct_chip_restore!(Ym2151, OpmRegisters, "YM2151");
impl_direct_chip_restore!(Ym3806, OpqRegisters, "YM3806");
impl_direct_chip_restore!(Ym2414, OpzRegisters, "YM2414");
impl_direct_chip_restore!(OpllFamily<const VARIANT: u8>, OpllRegisters, "OPLL");
impl_direct_chip_restore!(Ym3526, OplRegisters, "YM3526");
impl_direct_chip_restore!(Ym3812, Opl2Registers, "YM3812");
impl_direct_chip_restore!(Y8950, OplRegisters, "Y8950");
impl_direct_chip_restore!(Ymf289b, Opl3Registers, "YMF289B");

#[cfg(test)]
mod state_tests {
    use super::*;

    #[test]
    fn ym2203_state_replays_exact_samples() {
        let mut chip = Ym2203::new();
        chip.reset();
        chip.write_address(0xA0);
        chip.write_data(0x34);
        chip.write_address(0xA4);
        chip.write_data(0x22);
        chip.write_address(0x28);
        chip.write_data(0xF0);
        chip.generate(&mut [YmfmOutput4 { data: [0; 4] }; 37]);

        let encoded = save_state::encode_runtime_state(&chip.capture_state());
        let decoded = save_state::decode_runtime_state::<Ym2203>(&encoded, 1 << 20).unwrap();
        let mut restored = Ym2203::new();
        restored.restore_state(decoded).unwrap();

        let mut expected = [YmfmOutput4 { data: [0; 4] }; 64];
        let mut actual = [YmfmOutput4 { data: [0; 4] }; 64];
        chip.generate(&mut expected);
        restored.generate(&mut actual);
        assert!(
            expected
                .iter()
                .zip(actual)
                .all(|(left, right)| left.data == right.data)
        );
    }

    #[test]
    fn ym2608_state_replays_exact_samples_and_retains_rom() {
        let rhythm_rom = [0x5Au8; YM2608_ADPCM_A_ROM_SIZE];
        let mut chip = Ym2608::new();
        chip.set_adpcm_a_rom(&rhythm_rom);
        chip.reset();
        chip.write_address(0xA0);
        chip.write_data(0x41);
        chip.write_address(0xA4);
        chip.write_data(0x24);
        chip.write_address(0x28);
        chip.write_data(0xF0);
        chip.generate(&mut [YmfmOutput3 { data: [0; 3] }; 41]);

        let encoded = save_state::encode_runtime_state(&chip.capture_state());
        let decoded = save_state::decode_runtime_state::<Ym2608State>(&encoded, 1 << 20).unwrap();
        let mut restored = Ym2608::new();
        restored.set_adpcm_a_rom(&rhythm_rom);
        restored.restore_state(decoded).unwrap();

        let mut expected = [YmfmOutput3 { data: [0; 3] }; 64];
        let mut actual = [YmfmOutput3 { data: [0; 3] }; 64];
        chip.generate(&mut expected);
        restored.generate(&mut actual);
        assert!(
            expected
                .iter()
                .zip(actual)
                .all(|(left, right)| left.data == right.data)
        );
    }

    #[test]
    fn ym2608_rejects_a_different_retained_rom() {
        let mut chip = Ym2608::new();
        chip.set_adpcm_a_rom(&[0x11; YM2608_ADPCM_A_ROM_SIZE]);
        let state = chip.capture_state();
        let mut restored = Ym2608::new();
        restored.set_adpcm_a_rom(&[0x22; YM2608_ADPCM_A_ROM_SIZE]);
        assert!(restored.restore_state(state).is_err());
    }

    #[test]
    fn ym2612_state_replays_exact_samples() {
        let mut chip = Ym2612::new();
        chip.reset();
        for (address, data) in [(0xA0, 0x41), (0xA4, 0x24), (0xB4, 0xC0), (0x28, 0xF0)] {
            chip.write_address(address);
            chip.write_data(data);
        }
        chip.write_address(0x2A);
        chip.write_data(0xC4);
        chip.write_address(0x2B);
        chip.write_data(0x80);
        chip.generate(&mut [YmfmOutput2 { data: [0; 2] }; 29]);

        let encoded = save_state::encode_runtime_state(&chip.capture_state());
        let decoded = save_state::decode_runtime_state::<Ym2612>(&encoded, 1 << 20).unwrap();
        let mut restored = Ym2612::new();
        restored.restore_state(decoded).unwrap();

        let mut expected = [YmfmOutput2 { data: [0; 2] }; 64];
        let mut actual = [YmfmOutput2 { data: [0; 2] }; 64];
        chip.generate(&mut expected);
        restored.generate(&mut actual);
        assert!(
            expected
                .iter()
                .zip(actual)
                .all(|(left, right)| left.data == right.data)
        );
    }

    #[test]
    fn ymf288_state_replays_exact_samples_and_retains_rom() {
        let rhythm_rom = [0x5Au8; YM2608_ADPCM_A_ROM_SIZE];
        let mut chip = Ymf288::new();
        chip.set_adpcm_a_rom(&rhythm_rom);
        chip.reset();
        for (address, data) in [
            (0xA0, 0x41),
            (0xA4, 0x24),
            (0x28, 0xF0),
            (0x11, 0x3F),
            (0x18, 0xDF),
            (0x10, 0x01),
            (0x08, 0x0F),
            (0x00, 0x20),
        ] {
            chip.write_address(address);
            chip.write_data(data);
        }
        chip.generate(&mut [YmfmOutput3 { data: [0; 3] }; 41]);

        let encoded = save_state::encode_runtime_state(&chip.capture_state());
        let decoded = save_state::decode_runtime_state::<Ymf288State>(&encoded, 1 << 20).unwrap();
        let mut restored = Ymf288::new();
        restored.set_adpcm_a_rom(&rhythm_rom);
        restored.restore_state(decoded).unwrap();

        let mut expected = [YmfmOutput3 { data: [0; 3] }; 64];
        let mut actual = [YmfmOutput3 { data: [0; 3] }; 64];
        chip.generate(&mut expected);
        restored.generate(&mut actual);
        assert!(expected.iter().any(|sample| sample.data != [0; 3]));
        assert!(
            expected
                .iter()
                .zip(actual)
                .all(|(left, right)| left.data == right.data)
        );
    }

    #[test]
    fn ymf288_rejects_a_different_retained_rom() {
        let mut chip = Ymf288::new();
        chip.set_adpcm_a_rom(&[0x11; YM2608_ADPCM_A_ROM_SIZE]);
        let state = chip.capture_state();
        let mut restored = Ymf288::new();
        restored.set_adpcm_a_rom(&[0x22; YM2608_ADPCM_A_ROM_SIZE]);
        assert!(restored.restore_state(state).is_err());
    }

    #[test]
    fn ymf289b_state_replays_exact_samples() {
        let mut chip = Ymf289b::new();
        chip.reset();
        chip.write_address_hi(0x05);
        chip.write_data(0x05);
        for (address, data) in [
            (0x20, 0x21),
            (0x60, 0xF0),
            (0xC0, 0x31),
            (0xA0, 0x41),
            (0xB0, 0x31),
        ] {
            chip.write_address(address);
            chip.write_data(data);
        }
        chip.generate(&mut [YmfmOutput2 { data: [0; 2] }; 33]);

        let encoded = save_state::encode_runtime_state(&chip.capture_state());
        let decoded = save_state::decode_runtime_state::<Ymf289b>(&encoded, 1 << 20).unwrap();
        let mut restored = Ymf289b::new();
        restored.restore_state(decoded).unwrap();

        let mut expected = [YmfmOutput2 { data: [0; 2] }; 64];
        let mut actual = [YmfmOutput2 { data: [0; 2] }; 64];
        chip.generate(&mut expected);
        restored.generate(&mut actual);
        assert!(expected.iter().any(|sample| sample.data != [0; 2]));
        assert!(
            expected
                .iter()
                .zip(actual)
                .all(|(left, right)| left.data == right.data)
        );
    }

    #[test]
    fn ym2149_state_replays_exact_samples() {
        let mut chip = Ym2149::new();
        chip.reset();
        for (address, data) in [
            (0x00, 0x21),
            (0x06, 0x07),
            (0x07, 0x30),
            (0x08, 0x0F),
            (0x09, 0x10),
            (0x0A, 0x09),
            (0x0B, 0x05),
            (0x0D, 0x0E),
        ] {
            chip.write_address(address);
            chip.write_data(data);
        }
        chip.generate(&mut [YmfmOutput3 { data: [0; 3] }; 77]);

        let encoded = save_state::encode_runtime_state(&chip.capture_state());
        let decoded = save_state::decode_runtime_state::<Ym2149>(&encoded, 1 << 20).unwrap();
        let mut restored = Ym2149::new();
        restored.restore_state(decoded).unwrap();

        let mut expected = [YmfmOutput3 { data: [0; 3] }; 256];
        let mut actual = [YmfmOutput3 { data: [0; 3] }; 256];
        chip.generate(&mut expected);
        restored.generate(&mut actual);
        assert!(expected.iter().any(|sample| sample.data != [0; 3]));
        assert!(
            expected
                .iter()
                .zip(actual)
                .all(|(expected, actual)| expected.data == actual.data)
        );
    }

    #[test]
    fn ym3806_state_replays_exact_samples() {
        let mut chip = Ym3806::new();
        chip.reset();
        for (address, data) in [
            (0x04, 0x05),
            (0x10, 0xC4),
            (0x18, 0xF3),
            (0x28, 0x4C),
            (0x38, 0x80),
            (0x20, 0x37),
            (0x30, 0x10),
            (0x40, 0x81),
            (0x48, 0x2A),
            (0x48, 0x83),
            (0x50, 0x40),
            (0x58, 0x82),
            (0x80, 0x1F),
            (0x88, 0x1C),
            (0x90, 0x1F),
            (0x98, 0x1E),
            (0xA0, 0x80),
            (0xB8, 0x40),
            (0xE0, 0x08),
            (0x05, 0x78),
        ] {
            chip.write(address, data);
        }
        chip.generate(&mut [YmfmOutput2 { data: [0; 2] }; 300]);
        chip.write(0x05, 0x00);
        chip.generate(&mut [YmfmOutput2 { data: [0; 2] }; 41]);

        let encoded = save_state::encode_runtime_state(&chip.capture_state());
        let decoded = save_state::decode_runtime_state::<Ym3806>(&encoded, 1 << 20).unwrap();
        let mut restored = Ym3806::new();
        restored.restore_state(decoded).unwrap();

        let mut expected = [YmfmOutput2 { data: [0; 2] }; 512];
        let mut actual = [YmfmOutput2 { data: [0; 2] }; 512];
        chip.generate(&mut expected);
        restored.generate(&mut actual);
        assert!(expected.iter().any(|sample| sample.data != [0; 2]));
        assert!(
            expected
                .iter()
                .zip(actual)
                .all(|(expected, actual)| expected.data == actual.data)
        );
    }

    #[test]
    fn ym2414_state_replays_exact_samples() {
        let mut chip = Ym2414::new();
        chip.reset();
        let mut writes = alloc::vec![
            (0x08, 0x02),
            (0x18, 0xC5),
            (0x19, 0x40),
            (0x19, 0xB0),
            (0x16, 0xA7),
            (0x17, 0x30),
            (0x17, 0xC0),
            (0x1B, 0x16),
            (0x28 + 2, 0x4C),
            (0x30 + 2, 0x55),
            (0x38 + 2, 0x61),
            (0x38 + 2, 0xD2),
        ];
        for op in 0..4u8 {
            let offset = 2 + op * 8;
            writes.extend([
                (0x40 + offset, 0x31 + op * 0x11),
                (0x40 + offset, 0x80 | (op << 4) | 0x03),
                (0x60 + offset, 0x0C),
                (0x80 + offset, if op & 1 != 0 { 0x3F } else { 0x1F }),
                (0xA0 + offset, 0x80 | 0x06),
                (0xC0 + offset, 0x20 | (op << 6) | 0x03),
                (0xE0 + offset, 0x47),
            ]);
        }
        writes.push((0x20 + 2, 0xC4));
        for (address, data) in writes {
            chip.write_address(address);
            chip.write_data(data);
        }
        chip.generate(&mut [YmfmOutput2 { data: [0; 2] }; 431]);

        let encoded = save_state::encode_runtime_state(&chip.capture_state());
        let decoded = save_state::decode_runtime_state::<Ym2414>(&encoded, 1 << 20).unwrap();
        let mut restored = Ym2414::new();
        restored.restore_state(decoded).unwrap();

        let mut expected = [YmfmOutput2 { data: [0; 2] }; 1024];
        let mut actual = [YmfmOutput2 { data: [0; 2] }; 1024];
        chip.generate(&mut expected);
        restored.generate(&mut actual);
        assert!(expected.iter().any(|sample| sample.data != [0; 2]));
        assert!(
            expected
                .iter()
                .zip(actual)
                .all(|(expected, actual)| expected.data == actual.data)
        );
    }

    #[test]
    fn ym2423_state_replays_exact_samples() {
        let mut chip = Ym2423::new();
        chip.reset();
        for (address, data) in [(0x30, 0x30), (0x10, 0x80), (0x20, 0x15)] {
            chip.write_address(address);
            chip.write_data(data);
        }
        chip.generate(&mut [YmfmOutput2 { data: [0; 2] }; 57]);

        let encoded = save_state::encode_runtime_state(&chip.capture_state());
        let decoded = save_state::decode_runtime_state::<Ym2423>(&encoded, 1 << 20).unwrap();
        let mut restored = Ym2423::new();
        restored.restore_state(decoded).unwrap();

        let mut expected = [YmfmOutput2 { data: [0; 2] }; 64];
        let mut actual = [YmfmOutput2 { data: [0; 2] }; 64];
        chip.generate(&mut expected);
        restored.generate(&mut actual);
        assert!(expected.iter().any(|sample| sample.data != [0; 2]));
        assert!(
            expected
                .iter()
                .zip(actual)
                .all(|(left, right)| left.data == right.data)
        );
    }

    /// Builds a YM2610 family chip with ROMs, playing FM channel 1, ADPCM-A and ADPCM-B.
    fn ym2610_family_playing<const FM_CHANNEL_MASK: u32>(
        adpcm_a_rom: &[u8],
        adpcm_b_rom: &[u8],
    ) -> Ym2610Family<FM_CHANNEL_MASK> {
        let mut chip = Ym2610Family::<FM_CHANNEL_MASK>::new();
        chip.set_adpcm_a_rom(adpcm_a_rom.to_vec());
        chip.set_adpcm_b_rom(adpcm_b_rom.to_vec());
        chip.reset();
        for (address, data) in [(0xA5, 0x24), (0xA1, 0x41), (0xB1, 0x07), (0x41, 0x00)] {
            chip.write_address(address);
            chip.write_data(data);
        }
        for (address, data) in [(0x08, 0xDF), (0x01, 0x3F), (0x00, 0x01)] {
            chip.write_address_hi(address);
            chip.write_data_hi(data);
        }
        for (address, data) in [
            (0x11, 0xC0),
            (0x15, 0x01),
            (0x19, 0x55),
            (0x1B, 0xFF),
            (0x10, 0x80),
        ] {
            chip.write_address(address);
            chip.write_data(data);
        }
        chip.write_address(0x28);
        chip.write_data(0xF1);
        chip
    }

    /// Checks that a restored YM2610 family chip generates the same samples as the original.
    fn assert_ym2610_family_replays<const FM_CHANNEL_MASK: u32>() {
        let adpcm_a_rom = [0x5Au8; 0x1000];
        let adpcm_b_rom = [0x3Cu8; 0x800];
        let mut chip = ym2610_family_playing::<FM_CHANNEL_MASK>(&adpcm_a_rom, &adpcm_b_rom);
        chip.generate(&mut [YmfmOutput3 { data: [0; 3] }; 41]);

        let encoded = save_state::encode_runtime_state(&chip.capture_state());
        let decoded = save_state::decode_runtime_state::<Ym2610FamilyState<FM_CHANNEL_MASK>>(
            &encoded,
            1 << 20,
        )
        .unwrap();
        let mut restored = Ym2610Family::<FM_CHANNEL_MASK>::new();
        restored.set_adpcm_a_rom(adpcm_a_rom.to_vec());
        restored.set_adpcm_b_rom(adpcm_b_rom.to_vec());
        restored.restore_state(decoded).unwrap();

        let mut expected = [YmfmOutput3 { data: [0; 3] }; 256];
        let mut actual = [YmfmOutput3 { data: [0; 3] }; 256];
        chip.generate(&mut expected);
        restored.generate(&mut actual);
        assert!(expected.iter().any(|sample| sample.data[0] != 0));
        assert!(
            expected
                .iter()
                .zip(actual)
                .all(|(left, right)| left.data == right.data)
        );
    }

    #[test]
    fn ym2610_state_replays_exact_samples() {
        assert_ym2610_family_replays::<YM2610_FM_CHANNEL_MASK>();
    }

    #[test]
    fn ym2610b_state_replays_exact_samples() {
        assert_ym2610_family_replays::<YM2610B_FM_CHANNEL_MASK>();
    }

    #[test]
    fn ym2610_rejects_different_roms() {
        let chip = ym2610_family_playing::<YM2610_FM_CHANNEL_MASK>(&[0x11; 0x100], &[0x22; 0x100]);

        let mut other_adpcm_a = Ym2610::new();
        other_adpcm_a.set_adpcm_a_rom(vec![0x33; 0x100]);
        other_adpcm_a.set_adpcm_b_rom(vec![0x22; 0x100]);
        assert!(other_adpcm_a.restore_state(chip.capture_state()).is_err());

        let mut other_adpcm_b = Ym2610::new();
        other_adpcm_b.set_adpcm_a_rom(vec![0x11; 0x100]);
        other_adpcm_b.clear_adpcm_b_rom();
        assert!(other_adpcm_b.restore_state(chip.capture_state()).is_err());

        let mut same_roms = Ym2610::new();
        same_roms.set_adpcm_a_rom(vec![0x11; 0x100]);
        same_roms.set_adpcm_b_rom(vec![0x22; 0x100]);
        assert!(same_roms.restore_state(chip.capture_state()).is_ok());
    }

    #[test]
    fn ymf262_state_replays_exact_samples() {
        let mut chip = Ymf262::new();
        chip.reset();
        chip.write_address(0x20);
        chip.write_data(0x01);
        chip.write_address(0xA0);
        chip.write_data(0x80);
        chip.write_address(0xB0);
        chip.write_data(0x31);
        chip.generate(&mut [YmfmOutput4 { data: [0; 4] }; 29]);

        let encoded = save_state::encode_runtime_state(&chip.capture_state());
        let decoded = save_state::decode_runtime_state::<Ymf262>(&encoded, 1 << 20).unwrap();
        let mut restored = Ymf262::new();
        restored.restore_state(decoded).unwrap();

        let mut expected = [YmfmOutput4 { data: [0; 4] }; 64];
        let mut actual = [YmfmOutput4 { data: [0; 4] }; 64];
        chip.generate(&mut expected);
        restored.generate(&mut actual);
        assert!(
            expected
                .iter()
                .zip(actual)
                .all(|(left, right)| left.data == right.data)
        );
    }
}
