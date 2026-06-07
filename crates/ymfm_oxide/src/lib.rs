//! # ymfm_oxide
//!
//! Safe Rust reimplementation of [ymfm](https://github.com/aaronsgiles/ymfm) for Yamaha FM
//! synthesis chips.
//!
//! ## Ported chips
//!
//! Not all chips are ported. We only ported the chips that were used by different sound cards for the PC-98.
//!
//! | Chip   | Family | Features                                  | PC-98 sound card                 |
//! |--------|--------|-------------------------------------------|----------------------------------|
//! | YM2203 | OPN    | 3-ch FM + 3-ch SSG                        | PC-9801-26K                      |
//! | YM2608 | OPNA   | 6-ch stereo FM + SSG + ADPCM-A + ADPCM-B  | PC-9801-86                       |
//! | YM3526 | OPL    | 9-ch mono FM                              | (YM3812 predecessor, for compat) |
//! | Y8950  | OPL    | 9-ch mono FM + ADPCM-B                    | Sound Orchestra-V                |
//! | YM3812 | OPL2   | 9-ch mono FM, 4 waveforms                 | Sound Orchestra                  |
//! | YMF262 | OPL3   | 18-ch 4-output FM, 8 waveforms, 4-op mode | PC-9801-118, Sound Blaster 16    |
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

pub(crate) mod adpcm;
pub(crate) mod fm;
pub(crate) mod helpers;
pub(crate) mod opl;
pub(crate) mod opn;
pub(crate) mod ssg;
mod sys;
pub(crate) mod tables;

use adpcm::{AdpcmAEngine, AdpcmBChannel, AdpcmBEngine};
use fm::{FmEngine, FmRegisters};
use opl::{Opl2Registers, Opl3Registers, OplRegisters};
use opn::{OpnRegisters, OpnaRegisters, SsgResampler};
use ssg::SsgEngine;
pub use sys::{YmfmOpnFidelity, YmfmOutput1, YmfmOutput3, YmfmOutput4, YmfmTimerUpdate};

const YM2608_ADPCM_A_ROM_SIZE: usize = 8192;
static SILENT_ADPCM_MEMORY: [u8; 1] = [0];

/// Yamaha YM2203 (OPN) emulator.
pub struct Ym2203 {
    fm: FmEngine<OpnRegisters>,
    ssg: SsgEngine,
    ssg_resampler: SsgResampler,
    fidelity: YmfmOpnFidelity,
    address: u8,
    fm_samples_per_output: u32,
    last_fm: [i32; 1],
}

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
        };
        chip.update_prescale(prescale);
        chip
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
            self.ssg.read(self.address as u32 & 0x0F)
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

/// Yamaha YM2608 (OPNA) emulator.
///
/// The YM2608 adds stereo FM (6 channels), ADPCM-A rhythm, and ADPCM-B sample
/// playback over the YM2203.
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

const Y8950_STATUS_ADPCM_B_PLAYING: u8 = 0x01;
const Y8950_STATUS_ADPCM_B_BRDY: u8 = 0x08;
const Y8950_STATUS_ADPCM_B_EOS: u8 = 0x10;

/// Yamaha YM3526 (OPL) emulator.
///
/// 9-channel FM synthesis chip. Produces mono output.
pub struct Ym3526 {
    fm: FmEngine<OplRegisters>,
    address: u8,
}

impl Ym3526 {
    /// Creates a new YM3526 instance.
    pub fn new() -> Self {
        Self {
            fm: FmEngine::new(),
            address: 0,
        }
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

/// Yamaha Y8950 (OPL + ADPCM-B) emulator.
///
/// 9-channel FM synthesis chip with ADPCM-B sample playback.
/// Produces mono output.
pub struct Y8950 {
    fm: FmEngine<OplRegisters>,
    adpcm_b: AdpcmBEngine,
    adpcm_memory: Option<Vec<u8>>,
    address: u8,
    io_ddr: u8,
    io_input: [u8; 2],
    io_output: [Option<u8>; 2],
}

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

/// Yamaha YM3812 (OPL2) emulator.
///
/// 9-channel FM synthesis chip with enhanced waveform support.
/// Produces mono output.
pub struct Ym3812 {
    fm: FmEngine<Opl2Registers>,
    address: u8,
}

impl Ym3812 {
    /// Creates a new YM3812 instance.
    pub fn new() -> Self {
        Self {
            fm: FmEngine::new(),
            address: 0,
        }
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

/// Yamaha YMF262 (OPL3) emulator.
///
/// 18-channel FM synthesis chip with stereo output and 4-operator mode.
/// Produces 4-channel output: `[out0, out1, out2, out3]`.
pub struct Ymf262 {
    fm: FmEngine<Opl3Registers>,
    address: u16,
}

impl Ymf262 {
    /// Creates a new YMF262 instance.
    pub fn new() -> Self {
        Self {
            fm: FmEngine::new(),
            address: 0,
        }
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
