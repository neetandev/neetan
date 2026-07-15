//! FM Towns CD-ROM controller (I/O 0x04C0-0x04CF, IRQ 9, DMA channel 3).
//!
//! The Towns CD-ROM is a custom Fujitsu controller with a command/status register
//! file, an 8-deep parameter queue, and a status queue read back four bytes at a
//! time. Behavior is ported from the Tsugaru emulator's reverse-engineered
//! `cdrom.cpp`/`cdrom.h`, which derived the protocol from the Linux FM Towns CD
//! driver.
//!
//! ## License
//!
//! Copyright 2020 Soji Yamakawa (CaptainYS, http://www.ysflight.com)
//!
//! Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//! following conditions are met:
//!
//! 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//!    disclaimer.
//!
//! 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//!    following disclaimer in the documentation and/or other materials provided with the distribution.
//!
//! 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//!    products derived from this software without specific prior written permission.
//!
//! THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//! INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//! DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//! SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//! SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//! WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//! USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use crate::{
    cd_audio::{CdAudioPlayer, CdAudioPlayerState},
    cdrom::{CdImage, TrackType},
};

/// Cooked Mode 1 user-data sector size.
const MODE1_BYTES: usize = 2048;
/// Mode 2 (form-less) data size.
const MODE2_BYTES: usize = 2336;
/// Raw sector size.
const RAW_BYTES: usize = 2352;
/// Redbook lead-in offset: absolute MSF frame 150 maps to LBA 0.
const LEAD_IN_FRAMES: u32 = 150;
/// Sectors (frames) per second at 1x.
const SECTORS_PER_SECOND: u64 = 75;

/// Command opcode mask (bits 5 and 6 carry the status-request and IRQ flags).
const CMD_MASK: u8 = 0x9F;
/// Command flag: return status (bit 5).
const CMDFLAG_STATUS_REQUEST: u8 = 0x20;
/// Command flag: raise the status IRQ (bit 6).
const CMDFLAG_IRQ: u8 = 0x40;

// Command opcodes (after masking with CMD_MASK).
const CDCMD_SEEK: u8 = 0x00;
const CDCMD_MODE2READ: u8 = 0x01;
const CDCMD_MODE1READ: u8 = 0x02;
const CDCMD_RAWREAD: u8 = 0x03;
const CDCMD_CDDAPLAY: u8 = 0x04;
const CDCMD_TOCREAD: u8 = 0x05;
const CDCMD_SUBQREAD: u8 = 0x06;
const CDCMD_UNKNOWN1: u8 = 0x1F;
const CDCMD_SETSTATE: u8 = 0x80;
const CDCMD_CDDASET: u8 = 0x81;
const CDCMD_CDDASTOP: u8 = 0x84;
const CDCMD_CDDAPAUSE: u8 = 0x85;
const CDCMD_CDDARESUME: u8 = 0x87;
const CDCMD_UNKNOWN3: u8 = 0x9F;

// I/O port offsets within the 0x04C0-0x04CF window (plus the 0x04B0 caps port).
const PORT_CAPS: u16 = 0x04B0;
const PORT_MASTER_CTRL_STATUS: u16 = 0x04C0;
const PORT_COMMAND_STATUS: u16 = 0x04C2;
const PORT_PARAMETER_DATA: u16 = 0x04C4;
const PORT_TRANSFER_CTRL: u16 = 0x04C6;
const PORT_CACHE_2XSPEED: u16 = 0x04C8;
const PORT_SUBCODE_STATUS: u16 = 0x04CC;
const PORT_SUBCODE_DATA: u16 = 0x04CD;

// Master control (0x04C0 write) bits.
const MASTER_SMIC: u8 = 0x80; // Clear SIRQ.
const MASTER_DEIC: u8 = 0x40; // Clear DEI.
const MASTER_RESET_MPU: u8 = 0x04;
const MASTER_ENABLE_SIRQ: u8 = 0x02;
const MASTER_ENABLE_DEI: u8 = 0x01;

// Master status (0x04C0 read) bits.
const STATUS_SIRQ: u8 = 0x80;
const STATUS_DEI: u8 = 0x40;
const STATUS_STSF: u8 = 0x20;
const STATUS_DTSF: u8 = 0x10;
const STATUS_QUEUE_NOT_EMPTY: u8 = 0x02;
const STATUS_DRY: u8 = 0x01;

// Transfer control (0x04C6 write) bits.
const TRANSFER_DMA: u8 = 0x10;
const TRANSFER_CPU: u8 = 0x08;

// Timing constants in nanoseconds.
const DELAYED_STATUS_IRQ_NS: u64 = 50_000;
const NOTIFICATION_NS: u64 = 1_000_000;
const CDDASTOP_NS: u64 = 1_000_000;
const SEEK_NS: u64 = 100_000_000;
const LOSTDATA_TIMEOUT_NS: u64 = 100_000_000;
const STATUS_CHECKBACK_NS: u64 = 1_000_000;
/// Per-sector read time of the default timing mode. Deliberately faster than
/// a real 1x drive (13.3 ms per sector) and independent of the drive's speed
/// rating.
const READ_SECTOR_TIME_NS: u64 = 5_000_000;
/// Per-sector read time of a real 1x drive, used by the compatibility mode.
const COMPAT_READ_SECTOR_TIME_1X_NS: u64 = 13_300_000;
/// Full-stroke seek time of a real 1x drive, used by the compatibility mode.
const COMPAT_SEEK_TIME_1X_NS: u64 = 2_000_000_000;
/// Largest seek distance used to scale the compatibility-mode seek time.
const MAX_NUM_SECTORS: u64 = 350_000;
const CDDA_POLLING_INTERVAL_NS: u64 = 1_000_000_000 / SECTORS_PER_SECOND;

/// CD-DA playback state. `Stopping` is the one-poll pseudo-state some titles
/// (RAYXANBER) depend on: it reports the final frame for one polling interval
/// before the drive reports the play as ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CddaState {
    Idle,
    Playing,
    Paused,
    Stopping,
    Ended,
}

save_state::runtime_state! {
/// Authoritative FM Towns CD controller and CD audio state.
#[derive(Clone)]
pub struct TownsCdControllerState {
    audio: CdAudioPlayerState,
    cpu_clock_hz: u32,
    sirq: bool,
    dei: bool,
    stsf: bool,
    dtsf: bool,
    dry: bool,
    enable_sirq: bool,
    enable_dei: bool,
    irq_asserted: bool,
    command_received: bool,
    command: u8,
    param_queue: [u8; 8],
    param_count: usize,
    status_queue: std::collections::VecDeque<u8>,
    reading_sector_lba: u32,
    end_sector_lba: u32,
    head_position_lba: u32,
    dma_transfer: bool,
    cpu_transfer: bool,
    cpu_transfer_pointer: usize,
    wait_for_dts_sts: bool,
    sector_cache: Vec<u8>,
    disc_changed: bool,
    lid_closed: bool,
    lid_locked: bool,
    can_open_close: bool,
    delayed_sirq: bool,
    cdda_state: u8,
    cdda_start_lba: u32,
    cdda_end_lba: u32,
    cdda_start_cycle: u64,
    cdda_repeat: bool,
    cdda_paused_lba: u32,
    command_task_cycle: Option<u64>,
    cdda_poll_cycle: Option<u64>,
    delayed_status_irq_cycles: u64,
    notification_cycles: u64,
    cddastop_cycles: u64,
    seek_command_cycles: u64,
    lostdata_timeout_cycles: u64,
    status_checkback_cycles: u64,
    read_sector_cycles: u64,
    max_seek_cycles: u64,
    cdda_poll_interval_cycles: u64,
    sector_read_delay_cycles: u64,
    media: save_state::MediaManifest,
}}

/// The outcome of a scheduled task run: an optional sector the bus must push
/// through DMA channel 3.
#[derive(Debug, Default)]
pub struct CdTaskOutcome {
    /// Sector bytes to transfer to memory via DMA channel 3, if a transfer is due.
    pub dma_sector: Option<Vec<u8>>,
}

/// CD drive timing model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdTimingMode {
    /// Default timing: 5 ms per sector, no seek delay. Faster than any real
    /// drive.
    Fast,
    /// Realistic drive timing for games that depend on the slow drive:
    /// 13.3 ms per sector and a distance-scaled seek of up to 2 s at 1x,
    /// both divided by the drive's speed rating.
    Compatibility {
        /// Drive speed rating (1 for a 1x drive, 2 for a 2x drive).
        drive_speed: u32,
    },
}

/// FM Towns CD-ROM controller.
pub struct TownsCdController {
    image: Option<CdImage>,
    audio: CdAudioPlayer,
    cpu_clock_hz: u32,

    // Master register flags (0x04C0).
    sirq: bool,
    dei: bool,
    stsf: bool,
    dtsf: bool,
    dry: bool,
    enable_sirq: bool,
    enable_dei: bool,
    /// The interrupt line into PIC IRQ 9 (the controller's IRR output).
    irq_asserted: bool,

    // Command / parameter / status queues.
    command_received: bool,
    command: u8,
    param_queue: [u8; 8],
    param_count: usize,
    status_queue: std::collections::VecDeque<u8>,

    // Read positions, in data-area LBA (lead-in already subtracted).
    reading_sector_lba: u32,
    end_sector_lba: u32,
    head_position_lba: u32,

    // Transfer state.
    dma_transfer: bool,
    cpu_transfer: bool,
    cpu_transfer_pointer: usize,
    wait_for_dts_sts: bool,
    sector_cache: Vec<u8>,

    // Media.
    disc_changed: bool,
    lid_closed: bool,
    lid_locked: bool,
    can_open_close: bool,

    /// Fractal Engine relies on every command's status IRQ being deferred.
    delayed_sirq: bool,

    // CD-DA state (position derived from emulated time until Phase 5 pumps audio).
    cdda_state: CddaState,
    cdda_start_lba: u32,
    cdda_end_lba: u32,
    cdda_start_cycle: u64,
    cdda_repeat: bool,
    /// Play position captured when the playback was paused.
    cdda_paused_lba: u32,

    // Scheduling: the command task and the CD-DA poll each have their own deadline.
    command_task_cycle: Option<u64>,
    cdda_poll_cycle: Option<u64>,

    // Timing, in CPU cycles.
    delayed_status_irq_cycles: u64,
    notification_cycles: u64,
    cddastop_cycles: u64,
    seek_command_cycles: u64,
    lostdata_timeout_cycles: u64,
    status_checkback_cycles: u64,
    read_sector_cycles: u64,
    max_seek_cycles: u64,
    cdda_poll_interval_cycles: u64,
    sector_read_delay_cycles: u64,
}

/// Converts a nanosecond duration to CPU cycles at the given clock.
fn ns_to_cycles(ns: u64, cpu_clock_hz: u32) -> u64 {
    ns.saturating_mul(u64::from(cpu_clock_hz)) / 1_000_000_000
}

impl TownsCdController {
    /// Creates a controller for the given audio sample rate and CPU clock.
    pub fn new(sample_rate: u32, cpu_clock_hz: u32) -> Self {
        let mut controller = Self {
            image: None,
            audio: CdAudioPlayer::new(sample_rate),
            cpu_clock_hz,
            sirq: false,
            dei: false,
            stsf: false,
            dtsf: false,
            dry: true,
            enable_sirq: false,
            enable_dei: false,
            irq_asserted: false,
            command_received: false,
            command: 0,
            param_queue: [0; 8],
            param_count: 0,
            status_queue: std::collections::VecDeque::new(),
            reading_sector_lba: 0,
            end_sector_lba: 0,
            head_position_lba: 0,
            dma_transfer: false,
            cpu_transfer: false,
            cpu_transfer_pointer: 0,
            wait_for_dts_sts: false,
            sector_cache: Vec::new(),
            disc_changed: false,
            lid_closed: true,
            lid_locked: false,
            can_open_close: true,
            delayed_sirq: false,
            cdda_state: CddaState::Idle,
            cdda_start_lba: 0,
            cdda_end_lba: 0,
            cdda_start_cycle: 0,
            cdda_repeat: false,
            cdda_paused_lba: 0,
            command_task_cycle: None,
            cdda_poll_cycle: None,
            delayed_status_irq_cycles: ns_to_cycles(DELAYED_STATUS_IRQ_NS, cpu_clock_hz).max(1),
            notification_cycles: ns_to_cycles(NOTIFICATION_NS, cpu_clock_hz).max(1),
            cddastop_cycles: ns_to_cycles(CDDASTOP_NS, cpu_clock_hz).max(1),
            seek_command_cycles: ns_to_cycles(SEEK_NS, cpu_clock_hz).max(1),
            lostdata_timeout_cycles: ns_to_cycles(LOSTDATA_TIMEOUT_NS, cpu_clock_hz).max(1),
            status_checkback_cycles: ns_to_cycles(STATUS_CHECKBACK_NS, cpu_clock_hz).max(1),
            read_sector_cycles: ns_to_cycles(READ_SECTOR_TIME_NS, cpu_clock_hz).max(1),
            max_seek_cycles: 0,
            cdda_poll_interval_cycles: ns_to_cycles(CDDA_POLLING_INTERVAL_NS, cpu_clock_hz).max(1),
            sector_read_delay_cycles: 0,
        };
        controller.reset_mpu();
        controller
    }

    /// Captures command transport, CD audio, timing, and mounted disc identity.
    pub fn capture_state(
        &self,
    ) -> Result<TownsCdControllerState, save_state::StateValidationError> {
        Ok(TownsCdControllerState {
            audio: self.audio.capture_state(),
            cpu_clock_hz: self.cpu_clock_hz,
            sirq: self.sirq,
            dei: self.dei,
            stsf: self.stsf,
            dtsf: self.dtsf,
            dry: self.dry,
            enable_sirq: self.enable_sirq,
            enable_dei: self.enable_dei,
            irq_asserted: self.irq_asserted,
            command_received: self.command_received,
            command: self.command,
            param_queue: self.param_queue,
            param_count: self.param_count,
            status_queue: self.status_queue.clone(),
            reading_sector_lba: self.reading_sector_lba,
            end_sector_lba: self.end_sector_lba,
            head_position_lba: self.head_position_lba,
            dma_transfer: self.dma_transfer,
            cpu_transfer: self.cpu_transfer,
            cpu_transfer_pointer: self.cpu_transfer_pointer,
            wait_for_dts_sts: self.wait_for_dts_sts,
            sector_cache: self.sector_cache.clone(),
            disc_changed: self.disc_changed,
            lid_closed: self.lid_closed,
            lid_locked: self.lid_locked,
            can_open_close: self.can_open_close,
            delayed_sirq: self.delayed_sirq,
            cdda_state: match self.cdda_state {
                CddaState::Idle => 0,
                CddaState::Playing => 1,
                CddaState::Paused => 2,
                CddaState::Stopping => 3,
                CddaState::Ended => 4,
            },
            cdda_start_lba: self.cdda_start_lba,
            cdda_end_lba: self.cdda_end_lba,
            cdda_start_cycle: self.cdda_start_cycle,
            cdda_repeat: self.cdda_repeat,
            cdda_paused_lba: self.cdda_paused_lba,
            command_task_cycle: self.command_task_cycle,
            cdda_poll_cycle: self.cdda_poll_cycle,
            delayed_status_irq_cycles: self.delayed_status_irq_cycles,
            notification_cycles: self.notification_cycles,
            cddastop_cycles: self.cddastop_cycles,
            seek_command_cycles: self.seek_command_cycles,
            lostdata_timeout_cycles: self.lostdata_timeout_cycles,
            status_checkback_cycles: self.status_checkback_cycles,
            read_sector_cycles: self.read_sector_cycles,
            max_seek_cycles: self.max_seek_cycles,
            cdda_poll_interval_cycles: self.cdda_poll_interval_cycles,
            sector_read_delay_cycles: self.sector_read_delay_cycles,
            media: self.media_manifest()?,
        })
    }

    /// Restores command transport and CD audio while retaining disc contents.
    pub fn restore_state(
        &mut self,
        state: TownsCdControllerState,
    ) -> Result<(), save_state::StateValidationError> {
        let cdda_state = match state.cdda_state {
            0 => CddaState::Idle,
            1 => CddaState::Playing,
            2 => CddaState::Paused,
            3 => CddaState::Stopping,
            4 => CddaState::Ended,
            _ => {
                return Err(save_state::StateValidationError::new(
                    "FM Towns CD audio state is invalid",
                ));
            }
        };
        if state.cpu_clock_hz != self.cpu_clock_hz
            || state.param_count > state.param_queue.len()
            || state.cpu_transfer_pointer > state.sector_cache.len()
            || state.sector_cache.len() > RAW_BYTES
        {
            return Err(save_state::StateValidationError::new(
                "FM Towns CD controller state is invalid",
            ));
        }
        state.media.verify_current(&self.media_manifest()?)?;
        self.audio.validate_state(&state.audio)?;
        self.audio.restore_state(state.audio)?;
        self.sirq = state.sirq;
        self.dei = state.dei;
        self.stsf = state.stsf;
        self.dtsf = state.dtsf;
        self.dry = state.dry;
        self.enable_sirq = state.enable_sirq;
        self.enable_dei = state.enable_dei;
        self.irq_asserted = state.irq_asserted;
        self.command_received = state.command_received;
        self.command = state.command;
        self.param_queue = state.param_queue;
        self.param_count = state.param_count;
        self.status_queue = state.status_queue;
        self.reading_sector_lba = state.reading_sector_lba;
        self.end_sector_lba = state.end_sector_lba;
        self.head_position_lba = state.head_position_lba;
        self.dma_transfer = state.dma_transfer;
        self.cpu_transfer = state.cpu_transfer;
        self.cpu_transfer_pointer = state.cpu_transfer_pointer;
        self.wait_for_dts_sts = state.wait_for_dts_sts;
        self.sector_cache = state.sector_cache;
        self.disc_changed = state.disc_changed;
        self.lid_closed = state.lid_closed;
        self.lid_locked = state.lid_locked;
        self.can_open_close = state.can_open_close;
        self.delayed_sirq = state.delayed_sirq;
        self.cdda_state = cdda_state;
        self.cdda_start_lba = state.cdda_start_lba;
        self.cdda_end_lba = state.cdda_end_lba;
        self.cdda_start_cycle = state.cdda_start_cycle;
        self.cdda_repeat = state.cdda_repeat;
        self.cdda_paused_lba = state.cdda_paused_lba;
        self.command_task_cycle = state.command_task_cycle;
        self.cdda_poll_cycle = state.cdda_poll_cycle;
        self.delayed_status_irq_cycles = state.delayed_status_irq_cycles;
        self.notification_cycles = state.notification_cycles;
        self.cddastop_cycles = state.cddastop_cycles;
        self.seek_command_cycles = state.seek_command_cycles;
        self.lostdata_timeout_cycles = state.lostdata_timeout_cycles;
        self.status_checkback_cycles = state.status_checkback_cycles;
        self.read_sector_cycles = state.read_sector_cycles;
        self.max_seek_cycles = state.max_seek_cycles;
        self.cdda_poll_interval_cycles = state.cdda_poll_interval_cycles;
        self.sector_read_delay_cycles = state.sector_read_delay_cycles;
        Ok(())
    }

    /// Returns the mounted disc identity.
    pub fn media_manifest(
        &self,
    ) -> Result<save_state::MediaManifest, save_state::StateValidationError> {
        let bindings = self
            .image
            .as_ref()
            .map(|image| {
                Ok(save_state::MediaBinding {
                    identifier: save_state::MediaBindingId::new("cdrom-0")?,
                    slot: save_state::MediaSlot::new(save_state::MediaKind::CdRom, 0),
                    source_path: image.source_path().cloned(),
                    media_type: "cdrom".to_owned(),
                    identity: image.identity(),
                    geometry: None,
                    write_protected: true,
                    backend_generation: None,
                })
            })
            .transpose()?
            .into_iter()
            .collect();
        save_state::MediaManifest::new(bindings)
    }

    /// Inserts a disc image, raising the media-changed condition.
    pub fn insert(&mut self, image: CdImage) {
        self.image = Some(image);
        self.disc_changed = true;
        self.lid_closed = true;
        self.lid_locked = false;
        self.audio.reset();
        self.cdda_state = CddaState::Idle;
    }

    /// Removes the current disc image, if any.
    pub fn eject(&mut self) {
        self.image = None;
        self.disc_changed = true;
        self.lid_closed = false;
        self.audio.reset();
        self.cdda_state = CddaState::Idle;
    }

    /// Whether a disc image is loaded.
    pub fn has_disc(&self) -> bool {
        self.image
            .as_ref()
            .is_some_and(|image| image.track_count() > 0)
    }

    /// Updates the audio output sample rate.
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        self.audio.set_sample_rate(sample_rate);
    }

    /// Selects the drive timing model.
    pub fn set_timing_mode(&mut self, mode: CdTimingMode) {
        match mode {
            CdTimingMode::Fast => {
                self.read_sector_cycles =
                    ns_to_cycles(READ_SECTOR_TIME_NS, self.cpu_clock_hz).max(1);
                self.max_seek_cycles = 0;
            }
            CdTimingMode::Compatibility { drive_speed } => {
                let drive_speed = u64::from(drive_speed.max(1));
                self.read_sector_cycles = ns_to_cycles(
                    COMPAT_READ_SECTOR_TIME_1X_NS / drive_speed,
                    self.cpu_clock_hz,
                )
                .max(1);
                self.max_seek_cycles =
                    ns_to_cycles(COMPAT_SEEK_TIME_1X_NS / drive_speed, self.cpu_clock_hz);
            }
        }
    }

    /// Additively mixes CD-DA audio into `output` (interleaved stereo) at the
    /// given `[left, right]` volumes.
    pub fn generate_audio_samples(&mut self, volumes: [f32; 2], output: &mut [f32]) {
        if let Some(image) = self.image.as_ref() {
            self.audio.generate_samples(image, volumes, output);
        }
    }

    /// The interrupt line level into PIC IRQ 9.
    pub fn irq_line(&self) -> bool {
        self.irq_asserted
    }

    /// The current parameter queue contents, for tracing.
    pub fn params(&self) -> &[u8] {
        &self.param_queue[..self.param_count]
    }

    /// The current `(status IRQ, DMA-end IRQ)` status-register flags, for tracing.
    pub fn interrupt_flags(&self) -> (bool, bool) {
        (self.sirq, self.dei)
    }

    /// The cycle of the controller's next scheduled task, if any.
    pub fn next_task_cycle(&self) -> Option<u64> {
        match (self.command_task_cycle, self.cdda_poll_cycle) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    fn reset_mpu(&mut self) {
        self.sirq = false;
        self.dei = false;
        self.stsf = false;
        self.dtsf = false;
        self.dry = true;
        self.command_received = false;
        self.param_count = 0;
        self.param_queue = [0; 8];
        self.reading_sector_lba = 0;
        self.end_sector_lba = 0;
        self.head_position_lba = 0;
        self.status_queue.clear();
        self.dma_transfer = false;
        self.cpu_transfer = false;
        self.lid_closed = true;
        self.lid_locked = false;
    }

    /// Reads an I/O port in the CD-ROM window.
    pub fn io_read(&mut self, port: u16, now: u64) -> u8 {
        match port {
            PORT_MASTER_CTRL_STATUS => {
                let mut data = 0;
                if self.sirq {
                    data |= STATUS_SIRQ;
                }
                if self.dei {
                    data |= STATUS_DEI;
                }
                if self.stsf {
                    data |= STATUS_STSF;
                }
                if self.dtsf {
                    data |= STATUS_DTSF;
                }
                if !self.status_queue.is_empty() {
                    data |= STATUS_QUEUE_NOT_EMPTY;
                }
                if self.dry {
                    data |= STATUS_DRY;
                }
                data
            }
            PORT_COMMAND_STATUS => {
                if let Some(byte) = self.status_queue.pop_front() {
                    if self.status_queue.len().is_multiple_of(4) && self.command & CMDFLAG_IRQ != 0
                    {
                        self.set_sirq_irr();
                    }
                    byte
                } else {
                    0xFF
                }
            }
            PORT_PARAMETER_DATA => self.read_pio_data(now),
            PORT_CAPS if self.can_open_close => 0x0F,
            // Cache / 2x-speed register: no cache is modeled, so it reads idle.
            PORT_CACHE_2XSPEED => 0xFF,
            // Subcode (Q-subchannel) status/data: the streaming protocol is not
            // modeled, so the status reports no subcode ready and the data port
            // returns zero rather than an undecoded bus value.
            PORT_SUBCODE_STATUS | PORT_SUBCODE_DATA => 0x00,
            _ => 0xFF,
        }
    }

    /// Writes an I/O port in the CD-ROM window. `dma_ready` is true when DMA
    /// channel 3 is unmasked with a nonzero remaining count.
    pub fn io_write(&mut self, port: u16, value: u8, now: u64, dma_ready: bool) {
        let mut command_or_param = false;
        match port {
            PORT_MASTER_CTRL_STATUS => {
                if value & MASTER_SMIC != 0 {
                    self.sirq = false;
                    if !self.sirq && !self.dei {
                        self.irq_asserted = false;
                    }
                }
                if value & MASTER_DEIC != 0 {
                    self.dei = false;
                    if !self.sirq && !self.dei {
                        self.irq_asserted = false;
                    }
                }
                if value & MASTER_RESET_MPU != 0 {
                    self.reset_mpu();
                }
                self.enable_sirq = value & MASTER_ENABLE_SIRQ != 0;
                self.enable_dei = value & MASTER_ENABLE_DEI != 0;
            }
            PORT_COMMAND_STATUS => {
                let mut command = value;
                // Fractal Engine writes a new command before clearing the previous
                // SIRQ; the command inherits the old status/IRQ flags.
                if self.sirq {
                    command |= self.command & 0x60;
                }
                self.command = command;
                self.command_received = true;
                command_or_param = true;
            }
            PORT_PARAMETER_DATA => {
                if self.param_count >= self.param_queue.len() {
                    for index in 0..self.param_queue.len() - 1 {
                        self.param_queue[index] = self.param_queue[index + 1];
                    }
                    self.param_count = self.param_queue.len() - 1;
                }
                self.param_queue[self.param_count] = value;
                self.param_count += 1;
                command_or_param = true;
            }
            PORT_TRANSFER_CTRL => {
                self.dma_transfer = value & TRANSFER_DMA != 0;
                self.cpu_transfer = value & TRANSFER_CPU != 0;
                if self.dma_transfer {
                    self.wait_for_dts_sts = false;
                    if dma_ready && !self.dtsf {
                        self.dtsf = true;
                        self.command_task_cycle = Some(now.saturating_add(self.read_sector_cycles));
                    }
                } else if self.cpu_transfer {
                    self.wait_for_dts_sts = false;
                    self.stsf = true;
                }
            }
            PORT_CACHE_2XSPEED => {
                // Speed/cache register: accepted and ignored; speed is a divisor.
            }
            // Subcode (Q-subchannel) status/data: not modeled, writes are dropped.
            PORT_SUBCODE_STATUS | PORT_SUBCODE_DATA => {}
            _ => {}
        }

        if command_or_param
            && self.command_received
            && (self.param_count >= self.param_queue.len()
                || (self.command & CMD_MASK) == CDCMD_TOCREAD)
        {
            self.execute_command(now);
        }
    }

    /// Reads one byte of PIO sector data (0x04C4 read), filling the sector cache on
    /// demand and closing out the sector at its end.
    fn read_pio_data(&mut self, now: u64) -> u8 {
        if !self.stsf {
            return 0xFF;
        }
        if self.sector_cache.is_empty() {
            self.sector_cache = self.read_data_sector(self.reading_sector_lba);
        }
        let byte = self
            .sector_cache
            .get(self.cpu_transfer_pointer)
            .copied()
            .unwrap_or(0xFF);
        self.cpu_transfer_pointer += 1;
        if self.cpu_transfer_pointer >= self.sector_cache.len() {
            self.reading_sector_lba += 1;
            self.command_task_cycle = Some(now.saturating_add(self.notification_cycles));
            self.cpu_transfer = false;
            self.stsf = false;
            self.dei = true;
            self.cpu_transfer_pointer = 0;
            self.sector_cache.clear();
        }
        byte
    }

    fn execute_command(&mut self, now: u64) {
        self.dry = false;
        self.delayed_sirq = true;
        self.command_task_cycle = Some(now.saturating_add(self.delayed_status_irq_cycles));
    }

    /// Runs the controller's scheduled work due at `now`. Returns a sector for the
    /// bus to push through DMA channel 3 when a DMA transfer is due.
    pub fn run_task(&mut self, now: u64, dma_ready: bool) -> CdTaskOutcome {
        if let Some(poll) = self.cdda_poll_cycle
            && now >= poll
        {
            self.update_cdda_state(now);
        }
        if let Some(task) = self.command_task_cycle
            && now >= task
        {
            self.command_task_cycle = None;
            return self.run_command_task(now, dma_ready);
        }
        CdTaskOutcome::default()
    }

    fn run_command_task(&mut self, now: u64, dma_ready: bool) -> CdTaskOutcome {
        if self.delayed_sirq {
            self.delayed_command_execution(now);
            return CdTaskOutcome::default();
        }
        match self.command & CMD_MASK {
            CDCMD_SEEK => {
                if self.status_request() {
                    self.push_status(4, 0, 0, 0);
                    if self.command & CMDFLAG_IRQ != 0 {
                        self.irq_asserted = true;
                        self.sirq = true;
                    }
                }
                CdTaskOutcome::default()
            }
            CDCMD_MODE1READ | CDCMD_MODE2READ | CDCMD_RAWREAD => self.run_read_task(now, dma_ready),
            CDCMD_CDDASTOP => {
                self.stop_cdda();
                CdTaskOutcome::default()
            }
            _ => CdTaskOutcome::default(),
        }
    }

    fn run_read_task(&mut self, now: u64, dma_ready: bool) -> CdTaskOutcome {
        if self.reading_sector_lba > self.end_sector_lba {
            // All sectors done.
            self.dry = true;
            self.status_queue.clear();
            self.dtsf = false;
            self.push_status(0x06, 0, 0, 0); // Read done.
            if self.status_request() {
                self.sirq = true;
                if self.command & CMDFLAG_IRQ != 0 && self.enable_sirq {
                    self.irq_asserted = true;
                }
            } else {
                self.sirq = false;
            }
            return CdTaskOutcome::default();
        }

        if self.wait_for_dts_sts {
            // DMA was not armed before the lost-data timeout elapsed.
            self.status_queue.clear();
            self.push_status(0x21, 0x0F, 0, 0); // Abnormal termination.
            self.sirq = false;
            if self.status_request() && self.command & CMDFLAG_IRQ != 0 && self.enable_sirq {
                self.irq_asserted = true;
                self.sirq = true;
            }
            self.dry = true;
            self.dei = false;
            self.dtsf = false;
            self.stsf = false;
            return CdTaskOutcome::default();
        }

        if !dma_ready || !self.dma_transfer {
            // Data-ready state: the CPU has not armed the DMA transfer yet.
            if self.irq_asserted && self.dei {
                // A prior DMA-end IRQ is still unconsumed; check back shortly so the
                // SIRQ(data-ready)/DEI(data-end) alternation is preserved.
                self.command_task_cycle = Some(now.saturating_add(self.status_checkback_cycles));
                return CdTaskOutcome::default();
            }
            self.push_status(0x22, 0, 0, 0); // Data ready.
            if self.status_request() && self.command & CMDFLAG_IRQ != 0 && self.enable_sirq {
                self.irq_asserted = true;
            }
            self.sirq = true;
            self.dei = false;
            self.dtsf = false;
            self.wait_for_dts_sts = true;
            self.command_task_cycle = Some(now.saturating_add(self.lostdata_timeout_cycles));
            return CdTaskOutcome::default();
        }

        if self.dtsf {
            // Transfer one sector via DMA channel 3.
            self.head_position_lba = self.reading_sector_lba;
            let sector = self.read_data_sector(self.reading_sector_lba);
            return CdTaskOutcome {
                dma_sector: Some(sector),
            };
        }
        CdTaskOutcome::default()
    }

    /// Completes a DMA sector transfer performed by the bus. Advances to the next
    /// sector, sets the DMA-end interrupt, and schedules the next task.
    pub fn on_dma_transfer_complete(&mut self, now: u64) {
        self.reading_sector_lba += 1;
        self.command_task_cycle = Some(now.saturating_add(self.notification_cycles));
        self.dma_transfer = false;
        self.dtsf = false;
        self.dei = true;
        if self.enable_dei {
            self.irq_asserted = true;
        }
    }

    fn delayed_command_execution(&mut self, now: u64) {
        self.dry = true;
        self.delayed_sirq = false;

        match self.command & CMD_MASK {
            CDCMD_SEEK => {
                // The drive accepts commands while the head is moving; report seek
                // done after SEEK_TIME.
                self.dry = true;
                self.command_task_cycle = Some(now.saturating_add(self.seek_command_cycles));
                self.status_queue.clear();
                if self.status_request() && !self.set_status_drive_not_ready_or_disc_changed() {
                    self.set_status_no_error();
                    if self.command & CMDFLAG_IRQ != 0 {
                        self.irq_asserted = true;
                        self.sirq = true;
                    }
                }
            }
            CDCMD_MODE1READ | CDCMD_MODE2READ | CDCMD_RAWREAD => {
                // A data read while CD-DA is playing silences the playback; the
                // single pickup cannot read data and play audio at once.
                self.cdda_state = CddaState::Idle;
                self.audio.reset();
                if !self.disc_loaded_and_lid_closed() {
                    self.push_status(0x21, 9, 0, 0);
                } else {
                    let begin = self.param_msf_frames(0);
                    let end = self.param_msf_frames(3);
                    self.begin_read_sector(now, begin, end);
                }
            }
            CDCMD_CDDAPLAY => {
                let offset = LEAD_IN_FRAMES;
                let begin = self.param_msf_frames(0).saturating_sub(offset);
                let end = self.param_msf_frames(3).saturating_sub(offset);
                let repeat = self.param_queue[6] == 1;
                self.status_queue.clear();
                self.dry = true;
                self.cdda_start_lba = begin;
                self.cdda_end_lba = end;
                self.cdda_start_cycle = now;
                self.cdda_repeat = repeat;
                self.cdda_state = CddaState::Playing;
                self.cdda_poll_cycle = Some(now.saturating_add(self.cdda_poll_interval_cycles));
                if let Some(image) = self.image.as_ref()
                    && end > begin
                {
                    self.audio.play(image, begin, end - begin);
                }
                if self.status_request() {
                    self.set_status_drive_not_ready_or_disc_changed_or_no_error();
                    self.sirq = true;
                    if self.command & CMDFLAG_IRQ != 0 && self.enable_sirq {
                        self.irq_asserted = true;
                    }
                }
            }
            CDCMD_TOCREAD => {
                self.status_queue.clear();
                if self.status_request() {
                    if self.set_status_drive_not_ready_or_disc_changed() {
                        self.finish_command();
                        return;
                    }
                    self.set_status_no_error();
                }
                self.set_status_queue_for_toc();
                if self.command & CMDFLAG_IRQ != 0 {
                    self.set_sirq_irr();
                }
            }
            CDCMD_SUBQREAD => {
                if self.status_request() {
                    self.set_status_subq_read(now);
                }
            }
            CDCMD_UNKNOWN1 => {
                if self.status_request() {
                    if self.param_queue[0] == 3 {
                        // Disc-info query: reports the disc type (0x41 with a data
                        // track, 0x21 otherwise) in an 18/19/19/20 status sequence.
                        if !self.disc_loaded_and_lid_closed() {
                            self.push_status(0, 9, 0, 0);
                            self.finish_command();
                            return;
                        }
                        if self.disc_changed {
                            self.push_status(0x21, 8, 0, 0);
                            self.disc_changed = false;
                        }
                        self.set_status_no_error();
                        let disc_type = if self.image.as_ref().is_some_and(|image| {
                            image
                                .tracks()
                                .iter()
                                .any(|track| track.track_type == TrackType::Data)
                        }) {
                            0x41
                        } else {
                            0x21
                        };
                        self.push_status(0x18, disc_type, 0, 0);
                        self.push_status(0x19, 0, 0, 0);
                        self.push_status(0x19, 0, 0, 0);
                        self.push_status(0x20, 0, 0, 0);
                    } else {
                        if self.set_status_drive_not_ready_or_disc_changed() {
                            self.finish_command();
                            return;
                        }
                        self.set_status_no_error();
                    }
                    if self.command & CMDFLAG_IRQ != 0 {
                        self.set_sirq_irr();
                    }
                }
            }
            CDCMD_UNKNOWN3 => {
                if self.param_queue[1] == 0x5F
                    && self.param_queue[2] == 0xFC
                    && self.param_queue[3] == 0x5F
                    && self.param_queue[4] == 0xFC
                {
                    self.push_status(0, 0, 0, 0);
                    self.push_status(0x1F, 0x5F, 0xFC, 0x01);
                } else {
                    self.push_status(0x21, 0, 0, 0);
                }
            }
            CDCMD_SETSTATE => {
                if self.status_request() {
                    self.command_task_cycle = None;
                    self.set_status_drive_not_ready_or_disc_changed_or_no_error();
                    if self.status_queue_has_media_changed_only() {
                        self.push_status(0, 0, 0, 0);
                    }
                    if self.cdda_state == CddaState::Ended {
                        self.cdda_state = CddaState::Idle;
                    }
                    if self.command & CMDFLAG_IRQ != 0 {
                        self.set_sirq_irr();
                    }
                }
            }
            CDCMD_CDDASET => self.command_cddaset(),
            CDCMD_CDDASTOP => {
                if self.cdda_is_playing() {
                    self.command_task_cycle = Some(now.saturating_add(self.cddastop_cycles));
                } else {
                    self.stop_cdda();
                }
            }
            CDCMD_CDDAPAUSE => {
                self.cdda_paused_lba = self.cdda_current_lba(now);
                self.cdda_state = CddaState::Paused;
                self.audio.stop();
                if self.status_request() && !self.set_status_drive_not_ready_or_disc_changed() {
                    self.push_status(0, 0x01, 0, 0); // 2nd byte 01 = paused.
                    self.push_status(0x12, 0, 0, 0);
                }
            }
            CDCMD_CDDARESUME => {
                if self.cdda_state == CddaState::Paused {
                    self.cdda_state = CddaState::Playing;
                    self.cdda_start_lba = self.cdda_paused_lba;
                    self.cdda_start_cycle = now;
                    self.cdda_poll_cycle = Some(now.saturating_add(self.cdda_poll_interval_cycles));
                    if let Some(image) = self.image.as_ref() {
                        self.audio.resume(image);
                    }
                }
                if self.status_request() && !self.set_status_drive_not_ready_or_disc_changed() {
                    self.push_status(0, 0, 0, 0);
                    self.push_status(0x13, 0, 0, 0);
                }
            }
            _ => {}
        }
        self.finish_command();
    }

    fn command_cddaset(&mut self) {
        if self.can_open_close && self.status_request() {
            match (self.param_queue[0], self.param_queue[1]) {
                (2, 0) => {
                    // Unlock.
                    self.set_status_no_error();
                    self.lid_locked = false;
                    self.raise_irq_if_requested();
                    return;
                }
                (2, 1) => {
                    // Lock.
                    self.set_status_no_error();
                    self.lid_locked = true;
                    self.raise_irq_if_requested();
                    return;
                }
                (2, 2) => {
                    // Open.
                    let second_byte = if self.cdda_is_playing() { 3 } else { 9 };
                    self.push_status(0, second_byte, 0, 0);
                    self.push_status(9, 9, 0, 0);
                    self.open_lid();
                    self.raise_irq_if_requested();
                    return;
                }
                (2, 4) => {
                    // Close.
                    if !self.lid_closed {
                        if self.has_disc() {
                            self.push_status(0, 9, 0, 0);
                        } else {
                            self.push_status(0, 9, 0, 0);
                            self.push_status(0x10, 9, 0, 0);
                        }
                    } else {
                        let mut second_byte = 0;
                        if self.cdda_is_playing() {
                            second_byte = 3;
                            self.stop_cdda();
                        }
                        self.push_status(0, second_byte, 0, 0);
                    }
                    self.raise_irq_if_requested();
                    return;
                }
                (2, 8) => {
                    // Check door state (Towns OS V2.1 L51).
                    self.set_status_drive_not_ready_or_disc_changed_or_no_error();
                    let door_state = if !self.lid_closed {
                        1
                    } else if self.lid_locked {
                        2
                    } else {
                        0
                    };
                    let second = self.status_second_byte();
                    self.push_status(0x24, second, door_state, 0);
                    self.raise_irq_if_requested();
                    return;
                }
                _ => {}
            }
        }
        if self.status_request() {
            self.set_status_drive_not_ready_or_disc_changed_or_no_error();
        }
    }

    fn raise_irq_if_requested(&mut self) {
        if self.command & CMDFLAG_IRQ != 0 {
            self.irq_asserted = true;
            self.sirq = true;
        }
    }

    fn finish_command(&mut self) {
        self.command_received = false;
        self.param_count = 0;
    }

    fn begin_read_sector(&mut self, now: u64, begin_frames: u32, end_frames: u32) {
        if begin_frames > end_frames || begin_frames < LEAD_IN_FRAMES {
            self.push_status(0x21, 0x01, 0, 0); // Parameter error.
            self.reading_sector_lba = 0;
            self.end_sector_lba = 0;
            return;
        }
        self.reading_sector_lba = begin_frames - LEAD_IN_FRAMES;
        self.end_sector_lba = end_frames - LEAD_IN_FRAMES;

        // Zero in the default timing mode; the compatibility mode charges a
        // distance-scaled seek per read command.
        let distance = u64::from(self.reading_sector_lba.abs_diff(self.head_position_lba));
        let seek_cycles = distance.saturating_mul(self.max_seek_cycles) / MAX_NUM_SECTORS;

        self.set_status_no_error();
        if self.enable_sirq {
            self.sirq = true;
            self.irq_asserted = true;
        }
        let delay = self
            .read_sector_cycles
            .saturating_add(seek_cycles)
            .saturating_add(self.sector_read_delay_cycles);
        self.command_task_cycle = Some(now.saturating_add(delay));
        self.dry = false;
        self.dtsf = false;
        self.wait_for_dts_sts = false;
    }

    fn update_cdda_state(&mut self, now: u64) {
        match self.cdda_state {
            CddaState::Playing => {
                if self.cdda_current_lba(now) >= self.cdda_end_lba {
                    if self.cdda_repeat {
                        self.cdda_start_cycle = now;
                        if let Some(image) = self.image.as_ref()
                            && self.cdda_end_lba > self.cdda_start_lba
                        {
                            self.audio.play(
                                image,
                                self.cdda_start_lba,
                                self.cdda_end_lba - self.cdda_start_lba,
                            );
                        }
                    } else {
                        self.cdda_state = CddaState::Stopping;
                    }
                }
                self.cdda_poll_cycle = Some(now.saturating_add(self.cdda_poll_interval_cycles));
            }
            CddaState::Stopping => {
                self.cdda_state = CddaState::Ended;
                self.cdda_poll_cycle = None;
            }
            _ => {
                self.cdda_poll_cycle = None;
            }
        }
    }

    /// The current CD-DA play-head LBA, derived from elapsed emulated time.
    fn cdda_current_lba(&self, now: u64) -> u32 {
        if self.cdda_state == CddaState::Playing {
            let elapsed = now.saturating_sub(self.cdda_start_cycle);
            let sectors = elapsed.saturating_mul(SECTORS_PER_SECOND) / u64::from(self.cpu_clock_hz);
            let lba = u64::from(self.cdda_start_lba) + sectors;
            lba.min(u64::from(self.cdda_end_lba)) as u32
        } else {
            self.cdda_end_lba
        }
    }

    fn stop_cdda(&mut self) {
        self.status_queue.clear();
        if !self.set_status_drive_not_ready_or_disc_changed() {
            self.cdda_state = CddaState::Ended;
            self.audio.reset();
            self.set_status_no_error();
            self.push_status(0x11, 0, 0, 0); // Stop done.
            self.push_status(0, 0x0D, 0, 0);
        } else {
            self.cdda_state = CddaState::Idle;
        }
    }

    fn open_lid(&mut self) {
        if self.cdda_is_playing() {
            self.stop_cdda();
        }
        self.lid_closed = false;
    }

    fn set_sirq_irr(&mut self) {
        if !self.status_queue.is_empty() {
            self.sirq = true;
            if self.enable_sirq {
                self.irq_asserted = true;
            }
        }
    }

    fn status_request(&self) -> bool {
        self.command & CMDFLAG_STATUS_REQUEST != 0
    }

    fn cdda_is_playing(&self) -> bool {
        matches!(self.cdda_state, CddaState::Playing | CddaState::Stopping)
    }

    fn disc_loaded_and_lid_closed(&self) -> bool {
        self.has_disc() && self.lid_closed
    }

    fn push_status(&mut self, d0: u8, d1: u8, d2: u8, d3: u8) {
        self.status_queue.push_back(d0);
        self.status_queue.push_back(d1);
        self.status_queue.push_back(d2);
        self.status_queue.push_back(d3);
    }

    fn status_second_byte(&self) -> u8 {
        if !self.disc_loaded_and_lid_closed() {
            9
        } else if self.cdda_state == CddaState::Paused {
            1
        } else if self.cdda_is_playing() {
            3
        } else {
            0
        }
    }

    fn set_status_no_error(&mut self) {
        let second = self.status_second_byte();
        self.push_status(0, second, 0, 0);
    }

    fn set_status_drive_not_ready_or_disc_changed(&mut self) -> bool {
        if !self.disc_loaded_and_lid_closed() {
            self.push_status(0, 9, 0, 0);
            true
        } else if self.disc_changed {
            self.push_status(0x21, 8, 0, 0);
            self.disc_changed = false;
            true
        } else {
            false
        }
    }

    fn set_status_drive_not_ready_or_disc_changed_or_no_error(&mut self) {
        if !self.set_status_drive_not_ready_or_disc_changed() {
            self.set_status_no_error();
        }
    }

    fn status_queue_has_media_changed_only(&self) -> bool {
        let queue = &self.status_queue;
        if queue.len() < 2 {
            return false;
        }
        let mut has_media_changed = false;
        let mut index = 0;
        while index + 1 < queue.len() {
            if queue[index] == 0x21 && queue[index + 1] == 0x08 {
                has_media_changed = true;
            } else {
                return false;
            }
            index += 4;
        }
        has_media_changed
    }

    fn set_status_queue_for_toc(&mut self) {
        let Some(image) = self.image.as_ref() else {
            return;
        };
        let track_count = image.track_count();
        let total_sectors = image.total_sectors();
        let tracks: Vec<(u8, TrackType, u32)> = image
            .tracks()
            .iter()
            .map(|track| (track.number, track.track_type, track.start_lba))
            .collect();

        self.push_status(0x16, 0, 0xA0, 0);
        self.push_status(0x17, 1, 0, 0);
        self.push_status(0x16, 0, 0xA1, 0);
        self.push_status(0x17, dec_to_bcd(track_count), 0, 0);

        let (length_m, length_s, length_f) = lba_to_msf(total_sectors + LEAD_IN_FRAMES);
        self.push_status(0x16, 0, 0xA2, 0);
        self.push_status(
            0x17,
            dec_to_bcd(length_m),
            dec_to_bcd(length_s),
            dec_to_bcd(length_f),
        );

        for (number, track_type, start_lba) in tracks {
            let second_byte = match track_type {
                TrackType::Audio => 0,
                TrackType::Data => 0x40,
            };
            self.push_status(0x16, second_byte, dec_to_bcd(number), 0);
            let (m, s, f) = lba_to_msf(start_lba + LEAD_IN_FRAMES);
            self.push_status(0x17, dec_to_bcd(m), dec_to_bcd(s), dec_to_bcd(f));
        }
    }

    fn set_status_subq_read(&mut self, now: u64) {
        if self.set_status_drive_not_ready_or_disc_changed() {
            return;
        }
        self.push_status(0, 0, 0, 0);

        let current_lba = self.cdda_current_lba(now);
        let (track_number, track_relative_lba) = self
            .image
            .as_ref()
            .and_then(|image| image.track_for_lba(current_lba))
            .map(|track| (track.number, current_lba.saturating_sub(track.start_lba)))
            .unwrap_or((1, 0));

        let (track_m, track_s, track_f) = lba_to_msf(track_relative_lba);
        let (disc_m, disc_s, disc_f) = lba_to_msf(current_lba + LEAD_IN_FRAMES);

        self.push_status(0x18, 0, dec_to_bcd(track_number), 0);
        self.push_status(
            0x19,
            dec_to_bcd(track_m),
            dec_to_bcd(track_s),
            dec_to_bcd(track_f),
        );
        self.push_status(0x19, 0, dec_to_bcd(disc_m), dec_to_bcd(disc_s));
        self.push_status(0x20, dec_to_bcd(disc_f), 0, 0);
    }

    /// Decodes the BCD MSF triplet at `param_queue[offset..offset+3]` into an
    /// absolute frame count (including the lead-in).
    fn param_msf_frames(&self, offset: usize) -> u32 {
        let minutes = u32::from(bcd_to_dec(self.param_queue[offset]));
        let seconds = u32::from(bcd_to_dec(self.param_queue[offset + 1]));
        let frames = u32::from(bcd_to_dec(self.param_queue[offset + 2]));
        (minutes * 60 + seconds) * SECTORS_PER_SECOND as u32 + frames
    }

    /// Reads the sector at `lba` in the format selected by the current command.
    fn read_data_sector(&self, lba: u32) -> Vec<u8> {
        let Some(image) = self.image.as_ref() else {
            return Vec::new();
        };
        match self.command & CMD_MASK {
            CDCMD_MODE1READ => {
                let mut buffer = vec![0u8; MODE1_BYTES];
                if image.read_sector(lba, &mut buffer).is_some() {
                    buffer
                } else {
                    Vec::new()
                }
            }
            CDCMD_MODE2READ => {
                let mut raw = vec![0u8; RAW_BYTES];
                if image
                    .read_sector_raw(lba, &mut raw)
                    .is_some_and(|copied| copied >= RAW_BYTES)
                {
                    let mut out = vec![0u8; RAW_BYTES];
                    out[..MODE2_BYTES].copy_from_slice(&raw[16..16 + MODE2_BYTES]);
                    out
                } else {
                    Vec::new()
                }
            }
            _ => {
                let mut raw = vec![0u8; RAW_BYTES];
                match image.read_sector_raw(lba, &mut raw) {
                    Some(RAW_BYTES) => raw,
                    Some(MODE1_BYTES) => {
                        let mut out = vec![0u8; RAW_BYTES];
                        out[4..4 + MODE1_BYTES].copy_from_slice(&raw[..MODE1_BYTES]);
                        out
                    }
                    _ => Vec::new(),
                }
            }
        }
    }
}

/// Converts an LBA to minute/second/frame components (75 frames per second).
fn lba_to_msf(lba: u32) -> (u8, u8, u8) {
    let minutes = lba / (75 * 60);
    let seconds = (lba / 75) % 60;
    let frames = lba % 75;
    (minutes as u8, seconds as u8, frames as u8)
}

fn dec_to_bcd(value: u8) -> u8 {
    ((value / 10) << 4) | (value % 10)
}

fn bcd_to_dec(value: u8) -> u8 {
    (value >> 4) * 10 + (value & 0x0F)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPU_HZ: u32 = 66_000_000;

    fn build_controller() -> TownsCdController {
        TownsCdController::new(48_000, CPU_HZ)
    }

    /// A minimal single data-track disc for command tests.
    fn build_disc(sectors: u32) -> CdImage {
        let cue = "FILE \"test.bin\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n";
        let data = vec![0u8; sectors as usize * MODE1_BYTES];
        CdImage::from_cue(cue, data).expect("build disc")
    }

    #[test]
    fn sector_read_time_uses_reference_timing() {
        let cdc = build_controller();
        assert_eq!(
            cdc.read_sector_cycles,
            ns_to_cycles(READ_SECTOR_TIME_NS, CPU_HZ)
        );
        assert_eq!(cdc.max_seek_cycles, 0);
    }

    #[test]
    fn compatibility_timing_scales_with_drive_speed() {
        let mut cdc = build_controller();

        cdc.set_timing_mode(CdTimingMode::Compatibility { drive_speed: 1 });
        assert_eq!(
            cdc.read_sector_cycles,
            ns_to_cycles(COMPAT_READ_SECTOR_TIME_1X_NS, CPU_HZ)
        );
        assert_eq!(
            cdc.max_seek_cycles,
            ns_to_cycles(COMPAT_SEEK_TIME_1X_NS, CPU_HZ)
        );

        cdc.set_timing_mode(CdTimingMode::Compatibility { drive_speed: 2 });
        assert_eq!(
            cdc.read_sector_cycles,
            ns_to_cycles(COMPAT_READ_SECTOR_TIME_1X_NS / 2, CPU_HZ)
        );
        assert_eq!(
            cdc.max_seek_cycles,
            ns_to_cycles(COMPAT_SEEK_TIME_1X_NS / 2, CPU_HZ)
        );

        cdc.set_timing_mode(CdTimingMode::Fast);
        assert_eq!(
            cdc.read_sector_cycles,
            ns_to_cycles(READ_SECTOR_TIME_NS, CPU_HZ)
        );
        assert_eq!(cdc.max_seek_cycles, 0);
    }

    #[test]
    fn master_status_reflects_flags() {
        let mut cdc = build_controller();
        // Power-on: ready to receive a command, nothing pending.
        assert_eq!(
            cdc.io_read(PORT_MASTER_CTRL_STATUS, 0) & STATUS_DRY,
            STATUS_DRY
        );
        assert_eq!(cdc.io_read(PORT_MASTER_CTRL_STATUS, 0) & STATUS_SIRQ, 0);

        cdc.sirq = true;
        cdc.dei = true;
        let status = cdc.io_read(PORT_MASTER_CTRL_STATUS, 0);
        assert_eq!(status & STATUS_SIRQ, STATUS_SIRQ);
        assert_eq!(status & STATUS_DEI, STATUS_DEI);
    }

    #[test]
    fn master_control_clears_interrupts() {
        let mut cdc = build_controller();
        cdc.sirq = true;
        cdc.dei = true;
        cdc.irq_asserted = true;
        // SMIC clears SIRQ but DEI keeps the line asserted.
        cdc.io_write(PORT_MASTER_CTRL_STATUS, MASTER_SMIC, 0, false);
        assert!(!cdc.sirq);
        assert!(cdc.irq_line());
        // DEIC clears DEI; now the line drops.
        cdc.io_write(PORT_MASTER_CTRL_STATUS, MASTER_DEIC, 0, false);
        assert!(!cdc.dei);
        assert!(!cdc.irq_line());
    }

    #[test]
    fn parameter_queue_slides_when_full() {
        let mut cdc = build_controller();
        for value in 0..8u8 {
            cdc.io_write(PORT_PARAMETER_DATA, value, 0, false);
        }
        // A ninth byte drops the oldest and appends.
        cdc.io_write(PORT_PARAMETER_DATA, 0xAA, 0, false);
        assert_eq!(cdc.param_queue[7], 0xAA);
        assert_eq!(cdc.param_queue[0], 1);
    }

    #[test]
    fn tocread_fires_without_full_queue_and_reports_tracks() {
        let mut cdc = build_controller();
        cdc.insert(build_disc(200));
        cdc.disc_changed = false; // A prior GETSTATE consumed the media-changed condition.
        // TOCREAD with the status-request + IRQ flags.
        cdc.io_write(
            PORT_COMMAND_STATUS,
            0x05 | CMDFLAG_STATUS_REQUEST | CMDFLAG_IRQ,
            0,
            false,
        );
        // The command is deferred; run the delayed execution.
        let next = cdc.next_task_cycle().expect("scheduled");
        cdc.run_task(next, false);
        // First status group is No-Error, then the A0 (first track) entry.
        assert_eq!(cdc.io_read(PORT_COMMAND_STATUS, next), 0x00);
        cdc.io_read(PORT_COMMAND_STATUS, next);
        cdc.io_read(PORT_COMMAND_STATUS, next);
        cdc.io_read(PORT_COMMAND_STATUS, next);
        assert_eq!(cdc.io_read(PORT_COMMAND_STATUS, next), 0x16);
    }

    #[test]
    fn mode1_read_transfers_a_sector_via_dma() {
        let mut cdc = build_controller();
        cdc.insert(build_disc(300));
        cdc.enable_sirq = true;
        cdc.enable_dei = true;

        // MODE1READ from 00:02:00 (LBA 0) to 00:02:00 (one sector), BCD MSF. The
        // BIOS always writes a full 8-parameter queue before the command.
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(0), 0, false); // begin min
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(2), 0, false); // begin sec
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(0), 0, false); // begin frm
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(0), 0, false); // end min
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(2), 0, false); // end sec
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(0), 0, false); // end frm
        cdc.io_write(PORT_PARAMETER_DATA, 0, 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, 0, 0, false);
        cdc.io_write(PORT_COMMAND_STATUS, 0x02 | CMDFLAG_IRQ, 0, false); // MODE1READ+IRQ

        // Delayed command execution -> begin read sector, which pushes a No-Error
        // status group immediately.
        let mut now = cdc.next_task_cycle().expect("delayed");
        assert!(cdc.run_task(now, false).dma_sector.is_none());
        assert_eq!(cdc.io_read(PORT_COMMAND_STATUS, now), 0x00);
        cdc.io_read(PORT_COMMAND_STATUS, now);
        cdc.io_read(PORT_COMMAND_STATUS, now);
        cdc.io_read(PORT_COMMAND_STATUS, now);

        // The read task fires; DMA not armed yet -> data ready.
        now = cdc.next_task_cycle().expect("read task");
        assert!(cdc.run_task(now, false).dma_sector.is_none());
        // Status queue holds a Data-Ready (0x22) group.
        assert_eq!(cdc.io_read(PORT_COMMAND_STATUS, now), 0x22);
        cdc.io_read(PORT_COMMAND_STATUS, now);
        cdc.io_read(PORT_COMMAND_STATUS, now);
        cdc.io_read(PORT_COMMAND_STATUS, now);

        // Arm the DMA transfer (0x4C6 DTS=1) with the channel ready.
        cdc.io_write(PORT_TRANSFER_CTRL, TRANSFER_DMA, now, true);
        now = cdc.next_task_cycle().expect("transfer task");
        let outcome = cdc.run_task(now, true);
        let sector = outcome.dma_sector.expect("a sector to DMA");
        assert_eq!(sector.len(), MODE1_BYTES);

        // The bus performs the DMA, then signals completion.
        cdc.on_dma_transfer_complete(now);
        assert!(cdc.dei);
        assert!(cdc.irq_line());

        // Next task: all sectors done -> Read Done (0x06).
        now = cdc.next_task_cycle().expect("read-done task");
        cdc.run_task(now, false);
        assert_eq!(cdc.io_read(PORT_COMMAND_STATUS, now), 0x06);
    }

    #[test]
    fn unknown1_param3_reports_disc_info() {
        let mut cdc = build_controller();
        cdc.insert(build_disc(200));
        cdc.disc_changed = false;

        cdc.io_write(PORT_PARAMETER_DATA, 3, 0, false);
        for _ in 0..7 {
            cdc.io_write(PORT_PARAMETER_DATA, 0, 0, false);
        }
        cdc.io_write(
            PORT_COMMAND_STATUS,
            CDCMD_UNKNOWN1 | CMDFLAG_STATUS_REQUEST,
            0,
            false,
        );

        let now = cdc.next_task_cycle().expect("delayed");
        cdc.run_task(now, false);

        // No-Error group first.
        assert_eq!(cdc.io_read(PORT_COMMAND_STATUS, now), 0x00);
        cdc.io_read(PORT_COMMAND_STATUS, now);
        cdc.io_read(PORT_COMMAND_STATUS, now);
        cdc.io_read(PORT_COMMAND_STATUS, now);
        // Disc-info: 0x18 with disc type 0x41 (disc has a data track).
        assert_eq!(cdc.io_read(PORT_COMMAND_STATUS, now), 0x18);
        assert_eq!(cdc.io_read(PORT_COMMAND_STATUS, now), 0x41);
        cdc.io_read(PORT_COMMAND_STATUS, now);
        cdc.io_read(PORT_COMMAND_STATUS, now);
        assert_eq!(cdc.io_read(PORT_COMMAND_STATUS, now), 0x19);
    }

    #[test]
    fn cdda_repeat_wraps_instead_of_ending() {
        let mut cdc = build_controller();
        cdc.insert(build_disc(200));

        // CDDAPLAY from 00:02:00 (LBA 0) to 00:03:00 (LBA 75), repeat enabled.
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(0), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(2), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(0), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(0), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(3), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(0), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, 1, 0, false); // Repeat.
        cdc.io_write(PORT_PARAMETER_DATA, 0, 0, false);
        cdc.io_write(PORT_COMMAND_STATUS, 0x04, 0, false);

        let now = cdc.next_task_cycle().expect("delayed cdda");
        cdc.run_task(now, false);
        assert_eq!(cdc.cdda_state, CddaState::Playing);

        // One second later the play head has reached the end; with repeat set the
        // playback restarts instead of stopping.
        let one_second = now + u64::from(CPU_HZ);
        cdc.cdda_poll_cycle = Some(one_second);
        cdc.run_task(one_second, false);
        assert_eq!(cdc.cdda_state, CddaState::Playing);
        assert_eq!(cdc.cdda_start_cycle, one_second);
        assert!(cdc.cdda_poll_cycle.is_some());
    }

    #[test]
    fn cdda_resume_rearms_poll_and_position() {
        let mut cdc = build_controller();
        cdc.insert(build_disc(200));

        // CDDAPLAY from 00:02:00 (LBA 0) to 00:03:00 (LBA 75), no repeat.
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(0), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(2), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(0), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(0), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(3), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(0), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, 0, 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, 0, 0, false);
        cdc.io_write(PORT_COMMAND_STATUS, 0x04, 0, false);
        let play_time = cdc.next_task_cycle().expect("delayed cdda");
        cdc.run_task(play_time, false);
        assert_eq!(cdc.cdda_state, CddaState::Playing);

        // Pause half a second in (play head near LBA 37).
        let pause_time = play_time + u64::from(CPU_HZ) / 2;
        for _ in 0..8 {
            cdc.io_write(PORT_PARAMETER_DATA, 0, pause_time, false);
        }
        cdc.io_write(
            PORT_COMMAND_STATUS,
            CDCMD_CDDAPAUSE | CMDFLAG_STATUS_REQUEST,
            pause_time,
            false,
        );
        let pause_exec = cdc.command_task_cycle.expect("delayed pause");
        cdc.run_task(pause_exec, false);
        assert_eq!(cdc.cdda_state, CddaState::Paused);
        assert!(cdc.cdda_paused_lba > 0 && cdc.cdda_paused_lba < 75);
        let paused_lba = cdc.cdda_paused_lba;

        // Resume two seconds later: the pause duration must not count as played.
        let resume_time = pause_exec + 2 * u64::from(CPU_HZ);
        for _ in 0..8 {
            cdc.io_write(PORT_PARAMETER_DATA, 0, resume_time, false);
        }
        cdc.io_write(
            PORT_COMMAND_STATUS,
            CDCMD_CDDARESUME | CMDFLAG_STATUS_REQUEST,
            resume_time,
            false,
        );
        let resume_exec = cdc.command_task_cycle.expect("delayed resume");
        cdc.run_task(resume_exec, false);
        assert_eq!(cdc.cdda_state, CddaState::Playing);
        assert_eq!(cdc.cdda_start_lba, paused_lba);
        assert_eq!(cdc.cdda_start_cycle, resume_exec);
        assert!(cdc.cdda_poll_cycle.is_some());

        // One second after the resume the remaining sectors are done; the state
        // machine reaches Stopping and then Ended again.
        let end_poll = resume_exec + u64::from(CPU_HZ);
        cdc.cdda_poll_cycle = Some(end_poll);
        cdc.run_task(end_poll, false);
        assert_eq!(cdc.cdda_state, CddaState::Stopping);
        let final_poll = cdc.next_task_cycle().expect("stopping poll");
        cdc.run_task(final_poll, false);
        assert_eq!(cdc.cdda_state, CddaState::Ended);
    }

    #[test]
    fn cdda_play_advances_and_ends() {
        let mut cdc = build_controller();
        cdc.insert(build_disc(200));

        // CDDAPLAY from 00:02:00 (LBA 0) to 00:03:00 (LBA 75), no repeat.
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(0), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(2), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(0), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(0), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(3), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, dec_to_bcd(0), 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, 0, 0, false);
        cdc.io_write(PORT_PARAMETER_DATA, 0, 0, false);
        cdc.io_write(PORT_COMMAND_STATUS, 0x04, 0, false);

        let now = cdc.next_task_cycle().expect("delayed cdda");
        cdc.run_task(now, false);
        assert_eq!(cdc.cdda_state, CddaState::Playing);

        // One second later the play head has advanced ~75 sectors (to the end);
        // the poll transitions Playing -> Stopping -> Ended.
        let one_second = now + u64::from(CPU_HZ);
        cdc.cdda_poll_cycle = Some(one_second);
        cdc.run_task(one_second, false);
        assert_eq!(cdc.cdda_state, CddaState::Stopping);
        let next_poll = cdc.next_task_cycle().expect("stopping poll");
        cdc.run_task(next_poll, false);
        assert_eq!(cdc.cdda_state, CddaState::Ended);
    }
}
