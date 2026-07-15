//! PC-8801-31 CD-ROM interface driving the PC-8801-30 SCSI CD-ROM drive.
//!
//! The PC-8801MC's built-in CD-ROM is a SCSI-1 drive (the PC-8801-30) behind the
//! PC-8801-31 interface board, decoded at I/O ports 0x90-0x9F. The CD-System BIOS
//! bit-bangs the SCSI bus through the status (0x90) and data (0x91) registers, so
//! this module emulates a SCSI *target*: the selection, command, data, status and
//! message phases, plus the small command subset the drive supports (the standard
//! SCSI-1 commands and the NEC vendor audio commands 0xD8-0xDE).
//!
//! This is PC-88 specific and is not shared with the PC-98, whose CD-ROM is ATAPI
//! over the IDE interface (see [`crate::ide`]).

use crate::{
    cd_audio::{CdAudioPlayer, CdAudioPlayerState, CdAudioState},
    cdrom::{CdImage, TrackType},
};

/// Cooked CD-ROM data sector size in bytes.
const DATA_SECTOR_SIZE: usize = 2048;
/// Redbook lead-in offset: absolute MSF frame 150 maps to LBA 0.
const LEAD_IN_FRAMES: u32 = 150;

// Port 0x90 status register bits.
const STATUS_BSY: u8 = 0x80;
const STATUS_REQ: u8 = 0x40;
const STATUS_MSG: u8 = 0x20;
const STATUS_CD: u8 = 0x10;
const STATUS_IO: u8 = 0x08;
const STATUS_DRIVE_ENABLE: u8 = 0x01;
/// Signals masked off the status register while SEL is asserted.
const STATUS_SELECTION_MASK: u8 = STATUS_BSY | STATUS_MSG | STATUS_CD | STATUS_IO;

/// Identifier returned at port 0x99, marking the board as a PC-8801MC CD-ROM.
const BOARD_ID: u8 = 0xCD;
/// Port 0x99 bit 4 selects the CD-ROM BIOS ROM bank.
const ROM_BANK_SELECT: u8 = 0x10;
/// Port 0x9F bit 0 enables the CD-ROM drive.
const CONTROL_DRIVE_ENABLE: u8 = 0x01;
/// Port 0x9F bit 6 enables DMA transfers.
const CONTROL_DMA_ENABLE: u8 = 0x40;
/// Port 0x94 bit 7 asserts SCSI bus reset.
const RESET_ASSERT: u8 = 0x80;

// SCSI status codes.
const SCSI_GOOD: u8 = 0x00;
const SCSI_CHECK_CONDITION: u8 = 0x02;

// SCSI sense keys.
const SENSE_NO_SENSE: u8 = 0x00;
const SENSE_NOT_READY: u8 = 0x02;
const SENSE_ILLEGAL_REQUEST: u8 = 0x05;
const SENSE_UNIT_ATTENTION: u8 = 0x06;

// Additional sense codes.
const ASC_INVALID_COMMAND: u8 = 0x20;
const ASC_INVALID_FIELD_IN_CDB: u8 = 0x24;
const ASC_MEDIUM_NOT_PRESENT: u8 = 0x3A;

// Standard SCSI-1 opcodes.
const CMD_TEST_UNIT_READY: u8 = 0x00;
const CMD_REQUEST_SENSE: u8 = 0x03;
const CMD_READ_6: u8 = 0x08;
const CMD_INQUIRY: u8 = 0x12;
const CMD_MODE_SELECT_6: u8 = 0x15;
const CMD_START_STOP_UNIT: u8 = 0x1B;
const CMD_PREVENT_ALLOW_REMOVAL: u8 = 0x1E;
const CMD_READ_CAPACITY: u8 = 0x25;
const CMD_READ_10: u8 = 0x28;
const CMD_READ_TOC: u8 = 0x43;

// NEC vendor opcodes (all 10-byte CDBs).
const CMD_NEC_AUDIO_START: u8 = 0xD8;
const CMD_NEC_AUDIO_END: u8 = 0xD9;
const CMD_NEC_PAUSE: u8 = 0xDA;
const CMD_NEC_GET_SUBQ: u8 = 0xDD;
const CMD_NEC_GET_DIR_INFO: u8 = 0xDE;

/// SCSI bus phase from the target's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No active nexus; BSY deasserted.
    BusFree,
    /// Receiving the command descriptor block from the initiator.
    Command,
    /// Sending command response data to the initiator.
    DataIn,
    /// Receiving parameter data from the initiator (MODE SELECT).
    DataOut,
    /// Sending the one-byte status code.
    Status,
    /// Sending the COMMAND COMPLETE message byte.
    MessageIn,
}

/// CD-DA playback status reported through the NEC SUBQ command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioStatus {
    Off,
    Playing,
    Paused,
}

save_state::runtime_state! {
/// Mutable PC-88 CD-ROM electronics and audio state.
#[derive(Clone)]
pub struct Pc88CdromState {
    media_identity: Option<save_state::ResourceIdentity>,
    audio: CdAudioPlayerState,
    phase: u8,
    request: bool,
    select_line: bool,
    command: [u8; 16],
    command_length: usize,
    command_position: usize,
    data: Vec<u8>,
    data_position: usize,
    data_out_remaining: usize,
    status_code: u8,
    sense_key: u8,
    sense_asc: u8,
    sense_ascq: u8,
    drive_enable: bool,
    dma_enable: bool,
    clock_heartbeat: bool,
    cdda_gain: f32,
    audio_status: u8,
    audio_play_mode: u8,
    current_frame: u32,
    end_frame: u32,
    last_frame: u32,
}}

/// PC-8801-31 CD-ROM interface and PC-8801-30 SCSI CD-ROM drive.
pub struct Pc88Cdrom {
    image: Option<CdImage>,
    audio: CdAudioPlayer,

    phase: Phase,
    request: bool,
    select_line: bool,

    command: [u8; 16],
    command_length: usize,
    command_position: usize,

    data: Vec<u8>,
    data_position: usize,
    data_out_remaining: usize,
    status_code: u8,

    sense_key: u8,
    sense_asc: u8,
    sense_ascq: u8,

    drive_enable: bool,
    dma_enable: bool,
    clock_heartbeat: bool,

    cdda_gain: f32,
    audio_status: AudioStatus,
    audio_play_mode: u8,
    current_frame: u32,
    end_frame: u32,
    last_frame: u32,
}

impl Pc88Cdrom {
    /// Creates the controller targeting the given audio output sample rate.
    pub fn new(sample_rate: u32) -> Self {
        Self {
            image: None,
            audio: CdAudioPlayer::new(sample_rate),
            phase: Phase::BusFree,
            request: false,
            select_line: false,
            command: [0; 16],
            command_length: 0,
            command_position: 0,
            data: Vec::new(),
            data_position: 0,
            data_out_remaining: 0,
            status_code: SCSI_GOOD,
            sense_key: SENSE_NO_SENSE,
            sense_asc: 0,
            sense_ascq: 0,
            drive_enable: false,
            dma_enable: false,
            clock_heartbeat: false,
            cdda_gain: 1.0,
            audio_status: AudioStatus::Off,
            audio_play_mode: 0,
            current_frame: 0,
            end_frame: 0,
            last_frame: 0,
        }
    }

    /// Inserts a disc image.
    pub fn insert(&mut self, image: CdImage) {
        self.last_frame = image.total_sectors();
        self.image = Some(image);
        self.sense_key = SENSE_UNIT_ATTENTION;
        self.audio.reset();
        self.audio_status = AudioStatus::Off;
    }

    /// Removes the current disc image, if any.
    pub fn eject(&mut self) {
        self.image = None;
        self.audio.reset();
        self.audio_status = AudioStatus::Off;
        self.sense_key = SENSE_UNIT_ATTENTION;
    }

    /// Whether a disc image is loaded.
    pub fn has_disc(&self) -> bool {
        self.image.is_some()
    }

    /// Returns the identity of the mounted disc.
    pub fn media_identity(&self) -> Option<save_state::ResourceIdentity> {
        self.image.as_ref().map(CdImage::identity)
    }

    /// Returns the normalized configured path of the mounted disc.
    pub fn media_source_path(&self) -> Option<&save_state::MediaSourcePath> {
        self.image.as_ref().and_then(CdImage::source_path)
    }

    /// Captures the controller without copying mounted disc data.
    pub fn capture_state(&self) -> Pc88CdromState {
        Pc88CdromState {
            media_identity: self.media_identity(),
            audio: self.audio.capture_state(),
            phase: self.phase as u8,
            request: self.request,
            select_line: self.select_line,
            command: self.command,
            command_length: self.command_length,
            command_position: self.command_position,
            data: self.data.clone(),
            data_position: self.data_position,
            data_out_remaining: self.data_out_remaining,
            status_code: self.status_code,
            sense_key: self.sense_key,
            sense_asc: self.sense_asc,
            sense_ascq: self.sense_ascq,
            drive_enable: self.drive_enable,
            dma_enable: self.dma_enable,
            clock_heartbeat: self.clock_heartbeat,
            cdda_gain: self.cdda_gain,
            audio_status: self.audio_status as u8,
            audio_play_mode: self.audio_play_mode,
            current_frame: self.current_frame,
            end_frame: self.end_frame,
            last_frame: self.last_frame,
        }
    }

    /// Restores controller electronics while retaining the mounted disc.
    pub fn restore_state(
        &mut self,
        state: Pc88CdromState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.media_identity != self.media_identity()
            || state.command_length > state.command.len()
            || state.command_position > state.command_length
            || state.data_position > state.data.len()
            || !state.cdda_gain.is_finite()
        {
            return Err(save_state::StateValidationError::new(
                "PC-88 CD-ROM state is invalid",
            ));
        }
        let phase = match state.phase {
            0 => Phase::BusFree,
            1 => Phase::Command,
            2 => Phase::DataIn,
            3 => Phase::DataOut,
            4 => Phase::Status,
            5 => Phase::MessageIn,
            _ => {
                return Err(save_state::StateValidationError::new(
                    "PC-88 CD-ROM phase is invalid",
                ));
            }
        };
        let audio_status = match state.audio_status {
            0 => AudioStatus::Off,
            1 => AudioStatus::Playing,
            2 => AudioStatus::Paused,
            _ => {
                return Err(save_state::StateValidationError::new(
                    "PC-88 CD-ROM audio status is invalid",
                ));
            }
        };
        self.audio.restore_state(state.audio)?;
        self.phase = phase;
        self.request = state.request;
        self.select_line = state.select_line;
        self.command = state.command;
        self.command_length = state.command_length;
        self.command_position = state.command_position;
        self.data = state.data;
        self.data_position = state.data_position;
        self.data_out_remaining = state.data_out_remaining;
        self.status_code = state.status_code;
        self.sense_key = state.sense_key;
        self.sense_asc = state.sense_asc;
        self.sense_ascq = state.sense_ascq;
        self.drive_enable = state.drive_enable;
        self.dma_enable = state.dma_enable;
        self.clock_heartbeat = state.clock_heartbeat;
        self.cdda_gain = state.cdda_gain;
        self.audio_status = audio_status;
        self.audio_play_mode = state.audio_play_mode;
        self.current_frame = state.current_frame;
        self.end_frame = state.end_frame;
        self.last_frame = state.last_frame;
        Ok(())
    }

    /// Updates the audio output sample rate.
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        self.audio.set_sample_rate(sample_rate);
    }

    /// Additively mixes CD-DA audio into `output` (interleaved stereo), scaled by
    /// the host `volume` and the current fader gain.
    pub fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) {
        let Some(image) = self.image.as_ref() else {
            return;
        };
        let gained_volume = volume * self.cdda_gain;
        self.audio
            .generate_samples(image, [gained_volume, gained_volume], output);
        if self.audio.state() == CdAudioState::Stopped {
            self.audio_status = AudioStatus::Off;
        }
    }

    /// Port 0x90 read: SCSI bus status.
    pub fn read_status(&self) -> u8 {
        let mut value = if self.drive_enable {
            STATUS_DRIVE_ENABLE
        } else {
            0
        };
        if self.phase != Phase::BusFree {
            value |= STATUS_BSY;
        }
        if self.request {
            value |= STATUS_REQ;
        }
        value |= match self.phase {
            Phase::BusFree => 0,
            Phase::Command => STATUS_CD,
            Phase::DataIn => STATUS_IO,
            Phase::DataOut => 0,
            Phase::Status => STATUS_CD | STATUS_IO,
            Phase::MessageIn => STATUS_MSG | STATUS_CD | STATUS_IO,
        };
        if self.select_line {
            value &= !STATUS_SELECTION_MASK;
        }
        value
    }

    /// Port 0x90 write: SCSI SELECT control (bit 0).
    pub fn write_select(&mut self, value: u8) {
        if value & 0x01 != 0 {
            if self.drive_enable {
                self.select_line = true;
                self.begin_selection();
            }
        } else {
            self.select_line = false;
        }
    }

    /// Port 0x91 read: one byte from the SCSI data bus, advancing the phase.
    pub fn read_data(&mut self) -> u8 {
        match self.phase {
            Phase::DataIn => {
                let byte = self.data.get(self.data_position).copied().unwrap_or(0);
                self.data_position += 1;
                if self.data_position >= self.data.len() {
                    self.enter_status();
                }
                byte
            }
            Phase::Status => {
                let byte = self.status_code;
                self.phase = Phase::MessageIn;
                self.request = true;
                byte
            }
            Phase::MessageIn => {
                self.phase = Phase::BusFree;
                self.request = false;
                0x00
            }
            _ => 0xFF,
        }
    }

    /// Port 0x91 write: one byte to the SCSI data bus, advancing the phase.
    pub fn write_data(&mut self, value: u8) {
        match self.phase {
            Phase::Command => {
                if self.command_position < self.command.len() {
                    self.command[self.command_position] = value;
                }
                if self.command_position == 0 {
                    self.command_length = command_length(value);
                }
                self.command_position += 1;
                if self.command_position >= self.command_length {
                    self.execute_command();
                }
            }
            Phase::DataOut => {
                if self.data_out_remaining > 0 {
                    self.data_out_remaining -= 1;
                }
                if self.data_out_remaining == 0 {
                    self.enter_status();
                }
            }
            _ => {}
        }
    }

    /// Port 0x94 write: SCSI bus reset (bit 7).
    pub fn write_reset(&mut self, value: u8) {
        if value & RESET_ASSERT != 0 {
            self.phase = Phase::BusFree;
            self.request = false;
            self.select_line = false;
            self.command_position = 0;
            self.data.clear();
            self.data_position = 0;
            self.data_out_remaining = 0;
            self.audio.reset();
            self.audio_status = AudioStatus::Off;
        }
    }

    /// Port 0x98 read: motor/board clock heartbeat. Toggles bit 7 while a disc is
    /// present (the motor runs); reads 0 otherwise.
    pub fn read_clock(&mut self) -> u8 {
        if !self.has_disc() {
            return 0;
        }
        self.clock_heartbeat = !self.clock_heartbeat;
        if self.clock_heartbeat { 0x80 } else { 0x00 }
    }

    /// Port 0x98 write: CD-DA fader control (bits 0-2).
    pub fn write_fader(&mut self, value: u8) {
        // Bits 1-2 select the action; the ramp duration in bit 0 is approximated
        // by jumping straight to the target gain.
        self.cdda_gain = match (value >> 1) & 0x03 {
            0 => 1.0, // enable
            1 => 0.0, // disable
            2 => 1.0, // fade-in (target full)
            _ => 0.0, // fade-out (target silent)
        };
    }

    /// Port 0x99 read: board identifier.
    pub fn read_id(&self) -> u8 {
        BOARD_ID
    }

    /// Port 0x99 write: CD-ROM BIOS ROM bank select. Returns the new bank-enable
    /// state (bit 4) so the bus can update the memory mapping.
    pub fn write_rom_bank(&mut self, value: u8) -> bool {
        value & ROM_BANK_SELECT != 0
    }

    /// Port 0x9B / 0x9D read: CD-DA channel volume meter (cosmetic).
    pub fn read_volume_meter(&self, _channel: usize) -> u8 {
        0x00
    }

    /// Port 0x9F write: drive enable (bit 0) and DMA enable (bit 6).
    pub fn write_control(&mut self, value: u8) {
        self.dma_enable = value & CONTROL_DMA_ENABLE != 0;
        self.drive_enable = value & CONTROL_DRIVE_ENABLE != 0;
    }

    /// Whether a DMA byte is available for transfer on channel 1. True only when
    /// DMA is enabled and the target is presenting read data.
    pub fn dma_request(&self) -> bool {
        self.dma_enable && self.phase == Phase::DataIn && self.request
    }

    /// Reads one DMA byte (mirrors the data port read used by DMA channel 1).
    pub fn dma_read_byte(&mut self) -> u8 {
        self.read_data()
    }

    fn begin_selection(&mut self) {
        self.phase = Phase::Command;
        self.command_position = 0;
        self.command_length = self.command.len();
        self.request = true;
    }

    fn enter_status(&mut self) {
        self.phase = Phase::Status;
        self.request = true;
    }

    fn set_sense(&mut self, key: u8, asc: u8, ascq: u8) {
        self.sense_key = key;
        self.sense_asc = asc;
        self.sense_ascq = ascq;
    }

    fn complete_good(&mut self) {
        self.status_code = SCSI_GOOD;
        self.enter_status();
    }

    fn complete_check(&mut self, key: u8, asc: u8, ascq: u8) {
        self.set_sense(key, asc, ascq);
        self.status_code = SCSI_CHECK_CONDITION;
        self.enter_status();
    }

    fn return_no_disc(&mut self) {
        self.complete_check(SENSE_NOT_READY, ASC_MEDIUM_NOT_PRESENT, 0x00);
    }

    /// Sets up the DATA IN phase from the bytes currently in `self.data`.
    fn send_data(&mut self, length: usize) {
        self.data.truncate(length);
        self.data_position = 0;
        self.status_code = SCSI_GOOD;
        if self.data.is_empty() {
            self.enter_status();
        } else {
            self.phase = Phase::DataIn;
            self.request = true;
        }
    }

    fn execute_command(&mut self) {
        let opcode = self.command[0];
        self.data.clear();
        match opcode {
            CMD_TEST_UNIT_READY => {
                if self.has_disc() {
                    self.complete_good();
                } else {
                    self.return_no_disc();
                }
            }
            CMD_REQUEST_SENSE => self.cmd_request_sense(),
            CMD_INQUIRY => self.cmd_inquiry(),
            CMD_MODE_SELECT_6 => self.cmd_mode_select_6(),
            CMD_START_STOP_UNIT => self.complete_good(),
            CMD_PREVENT_ALLOW_REMOVAL => self.complete_good(),
            CMD_READ_CAPACITY => self.cmd_read_capacity(),
            CMD_READ_6 | CMD_READ_10 => self.cmd_read(),
            CMD_READ_TOC => self.cmd_read_toc(),
            CMD_NEC_AUDIO_START => self.cmd_nec_audio_start(),
            CMD_NEC_AUDIO_END => self.cmd_nec_audio_end(),
            CMD_NEC_PAUSE => self.cmd_nec_pause(),
            CMD_NEC_GET_SUBQ => self.cmd_nec_get_subq(),
            CMD_NEC_GET_DIR_INFO => self.cmd_nec_get_dir_info(),
            _ => self.complete_check(SENSE_ILLEGAL_REQUEST, ASC_INVALID_COMMAND, 0x00),
        }
    }

    fn cmd_request_sense(&mut self) {
        let allocation = self.command[4] as usize;
        let mut sense = vec![0u8; 18];
        sense[0] = 0x70; // Current error, fixed format.
        sense[2] = self.sense_key;
        sense[7] = 10; // Additional sense length.
        sense[12] = self.sense_asc;
        sense[13] = self.sense_ascq;
        self.data = sense;
        self.sense_key = SENSE_NO_SENSE;
        self.sense_asc = 0;
        self.sense_ascq = 0;
        let length = if allocation == 0 {
            self.data.len()
        } else {
            allocation.min(self.data.len())
        };
        self.send_data(length);
    }

    fn cmd_inquiry(&mut self) {
        let allocation = self.command[4] as usize;
        let mut inquiry = vec![0u8; 36];
        inquiry[0] = 0x05; // CD-ROM device.
        inquiry[1] = 0x80; // Removable medium.
        inquiry[2] = 0x01; // SCSI-1 compliance.
        inquiry[3] = 0x01;
        inquiry[4] = 0x1F; // Additional length.
        inquiry[8..16].copy_from_slice(b"NEC     ");
        inquiry[16..32].copy_from_slice(b"CD-ROM DRIVE    ");
        inquiry[32..36].copy_from_slice(b"1.0 ");
        self.data = inquiry;
        let length = if allocation == 0 {
            self.data.len()
        } else {
            allocation.min(self.data.len())
        };
        self.send_data(length);
    }

    fn cmd_mode_select_6(&mut self) {
        // The PC-8801-30 expects one parameter byte more than the CDB declares.
        let length = self.command[4] as usize + 1;
        if length == 0 {
            self.complete_good();
        } else {
            self.data_out_remaining = length;
            self.status_code = SCSI_GOOD;
            self.phase = Phase::DataOut;
            self.request = true;
        }
    }

    fn cmd_read_capacity(&mut self) {
        if !self.has_disc() {
            self.return_no_disc();
            return;
        }
        let last_block = self.total_sectors().saturating_sub(1);
        let mut buffer = vec![0u8; 8];
        buffer[0..4].copy_from_slice(&last_block.to_be_bytes());
        buffer[4..8].copy_from_slice(&(DATA_SECTOR_SIZE as u32).to_be_bytes());
        self.data = buffer;
        self.send_data(8);
    }

    fn cmd_read(&mut self) {
        if !self.has_disc() {
            self.return_no_disc();
            return;
        }
        let (lba, blocks) = if self.command[0] == CMD_READ_6 {
            let lba = (u32::from(self.command[1] & 0x1F) << 16)
                | (u32::from(self.command[2]) << 8)
                | u32::from(self.command[3]);
            let blocks = if self.command[4] == 0 {
                256
            } else {
                u32::from(self.command[4])
            };
            (lba, blocks)
        } else {
            let lba = u32::from_be_bytes([
                self.command[2],
                self.command[3],
                self.command[4],
                self.command[5],
            ]);
            let blocks = u32::from(u16::from_be_bytes([self.command[7], self.command[8]]));
            (lba, blocks)
        };

        let image = self.image.as_ref().unwrap();
        let mut buffer = vec![0u8; blocks as usize * DATA_SECTOR_SIZE];
        for index in 0..blocks {
            let offset = index as usize * DATA_SECTOR_SIZE;
            let target = &mut buffer[offset..offset + DATA_SECTOR_SIZE];
            if image.read_sector(lba + index, target).is_none() {
                self.complete_check(SENSE_ILLEGAL_REQUEST, ASC_INVALID_FIELD_IN_CDB, 0x00);
                return;
            }
        }
        let length = buffer.len();
        self.data = buffer;
        self.send_data(length);
    }

    fn cmd_read_toc(&mut self) {
        if !self.has_disc() {
            self.return_no_disc();
            return;
        }
        let msf = self.command[1] & 0x02 != 0;
        let allocation = u16::from_be_bytes([self.command[7], self.command[8]]) as usize;
        let start_track = self.command[6];

        let image = self.image.as_ref().unwrap();
        let last_track = image.track_count();

        let mut buffer = vec![0u8; 4];
        buffer[2] = 1; // First track.
        buffer[3] = last_track; // Last track.

        let begin = if start_track == 0 { 1 } else { start_track };
        for number in begin..=last_track {
            if let Some(track) = image.track(number) {
                let control = track_control(track.track_type);
                let position = self.encode_lba(track.start_lba, msf);
                buffer.extend_from_slice(&[0x00, control, number, 0x00]);
                buffer.extend_from_slice(&position.to_be_bytes());
            }
        }
        // Lead-out entry (track 0xAA).
        let lead_out = self.encode_lba(self.total_sectors(), msf);
        buffer.extend_from_slice(&[0x00, 0x14, 0xAA, 0x00]);
        buffer.extend_from_slice(&lead_out.to_be_bytes());

        let total_length = buffer.len();
        let data_length = (total_length - 2) as u16;
        buffer[0..2].copy_from_slice(&data_length.to_be_bytes());

        self.data = buffer;
        let length = if allocation == 0 {
            total_length
        } else {
            allocation.min(total_length)
        };
        self.send_data(length);
    }

    fn cmd_nec_audio_start(&mut self) {
        if !self.has_disc() {
            self.return_no_disc();
            return;
        }
        let Some(frame) = self.nec_position(&self.command.clone()) else {
            self.complete_check(SENSE_ILLEGAL_REQUEST, ASC_INVALID_FIELD_IN_CDB, 0x00);
            return;
        };
        self.current_frame = frame;
        self.audio_status = AudioStatus::Paused;

        let play_mode = self.command[1] & 0x03;
        if play_mode != 0 {
            // Play to the end of the disc until an end position arrives.
            self.end_frame = self.last_frame;
            self.audio_play_mode = play_mode;
            self.start_audio();
        } else {
            // Position only; the end-position command will start playback.
            self.audio_play_mode = 3;
        }
        self.complete_good();
    }

    fn cmd_nec_audio_end(&mut self) {
        if !self.has_disc() {
            self.return_no_disc();
            return;
        }
        let Some(frame) = self.nec_position(&self.command.clone()) else {
            self.complete_check(SENSE_ILLEGAL_REQUEST, ASC_INVALID_FIELD_IN_CDB, 0x00);
            return;
        };
        self.end_frame = frame;
        self.audio_play_mode = self.command[1] & 0x03;

        if self.audio_play_mode != 0 {
            self.start_audio();
        } else {
            self.audio.reset();
            self.audio_status = AudioStatus::Off;
            self.end_frame = self.last_frame;
        }
        self.complete_good();
    }

    fn cmd_nec_pause(&mut self) {
        if !self.has_disc() {
            self.return_no_disc();
            return;
        }
        if self.audio_status == AudioStatus::Off {
            self.complete_check(SENSE_NO_SENSE, 0x00, 0x00);
            return;
        }
        self.current_frame = self.audio.current_position().0;
        self.audio.stop();
        self.audio_status = AudioStatus::Paused;
        self.complete_good();
    }

    fn cmd_nec_get_subq(&mut self) {
        if !self.has_disc() {
            self.return_no_disc();
            return;
        }
        let frame = match self.audio_status {
            AudioStatus::Off => self.current_frame,
            _ => self.audio.current_position().0,
        };
        let status = match self.audio_status {
            AudioStatus::Playing => 0x00,
            AudioStatus::Paused => 0x02,
            AudioStatus::Off => 0x03,
        };

        let image = self.image.as_ref().unwrap();
        let track_number = image.track_for_lba(frame).map_or(1, |track| track.number);
        let track_start = image
            .track_for_lba(frame)
            .map_or(0, |track| track.start_lba);
        let control = image
            .track_for_lba(frame)
            .map_or(0x14, |track| track_control(track.track_type));
        let (rel_m, rel_s, rel_f) = lba_to_msf(frame.saturating_sub(track_start));
        let (abs_m, abs_s, abs_f) = lba_to_msf(frame + LEAD_IN_FRAMES);

        let mut buffer = vec![0u8; 10];
        buffer[0] = status;
        buffer[1] = (control << 4) | 0x01;
        buffer[2] = dec_to_bcd(track_number);
        buffer[3] = 0x01; // Index.
        buffer[4] = dec_to_bcd(rel_m);
        buffer[5] = dec_to_bcd(rel_s);
        buffer[6] = dec_to_bcd(rel_f);
        buffer[7] = dec_to_bcd(abs_m);
        buffer[8] = dec_to_bcd(abs_s);
        buffer[9] = dec_to_bcd(abs_f);
        self.data = buffer;
        self.send_data(10);
    }

    fn cmd_nec_get_dir_info(&mut self) {
        if !self.has_disc() {
            self.return_no_disc();
            return;
        }
        let image = self.image.as_ref().unwrap();
        let last_track = image.track_count();
        let mut buffer = vec![0u8; 4];
        match self.command[1] {
            0x00 => {
                buffer[0] = dec_to_bcd(1);
                buffer[1] = dec_to_bcd(last_track);
            }
            0x01 => {
                let (m, s, f) = lba_to_msf(self.total_sectors());
                buffer[0] = dec_to_bcd(m);
                buffer[1] = dec_to_bcd(s);
                buffer[2] = dec_to_bcd(f);
            }
            0x02 => {
                let (frame, track_type) = if self.command[2] == 0xAA {
                    (self.total_sectors(), TrackType::Data)
                } else {
                    let number = bcd_to_dec(self.command[2]).max(1);
                    match image.track(number) {
                        Some(track) => (track.start_lba, track.track_type),
                        None => (0, TrackType::Data),
                    }
                };
                let (m, s, f) = lba_to_msf(frame + LEAD_IN_FRAMES);
                buffer[0] = dec_to_bcd(m);
                buffer[1] = dec_to_bcd(s);
                buffer[2] = dec_to_bcd(f);
                buffer[3] = match track_type {
                    TrackType::Data => 0x04,
                    TrackType::Audio => 0x00,
                };
            }
            _ => {
                self.complete_check(SENSE_ILLEGAL_REQUEST, ASC_INVALID_FIELD_IN_CDB, 0x00);
                return;
            }
        }
        self.data = buffer;
        self.send_data(4);
    }

    /// Starts CD-DA playback for the current start/end frame window.
    fn start_audio(&mut self) {
        if let Some(image) = self.image.as_ref()
            && self.end_frame > self.current_frame
        {
            let count = self.end_frame - self.current_frame;
            self.audio.play(image, self.current_frame, count);
            self.audio_status = AudioStatus::Playing;
        }
    }

    /// Decodes a NEC audio start/stop position CDB into an absolute LBA.
    fn nec_position(&self, command: &[u8; 16]) -> Option<u32> {
        match command[9] & 0xC0 {
            0x00 => Some(
                (u32::from(command[3]) << 16)
                    | (u32::from(command[4]) << 8)
                    | u32::from(command[5]),
            ),
            0x40 => {
                let minutes = u32::from(bcd_to_dec(command[2]));
                let seconds = u32::from(bcd_to_dec(command[3]));
                let frames = u32::from(bcd_to_dec(command[4]));
                let total = frames + 75 * (seconds + minutes * 60);
                Some(total.saturating_sub(LEAD_IN_FRAMES))
            }
            0x80 => {
                let number = bcd_to_dec(command[2]).max(1);
                self.image
                    .as_ref()
                    .and_then(|image| image.track(number))
                    .map(|track| track.start_lba)
            }
            _ => None,
        }
    }

    fn total_sectors(&self) -> u32 {
        self.image.as_ref().map_or(0, |image| image.total_sectors())
    }

    /// Encodes an LBA for a TOC entry: as packed MSF (absolute) or raw LBA.
    fn encode_lba(&self, lba: u32, msf: bool) -> u32 {
        if msf {
            let (m, s, f) = lba_to_msf(lba + LEAD_IN_FRAMES);
            (u32::from(m) << 16) | (u32::from(s) << 8) | u32::from(f)
        } else {
            lba
        }
    }
}

/// Returns the CDB length in bytes for a SCSI opcode.
fn command_length(opcode: u8) -> usize {
    match opcode {
        CMD_NEC_AUDIO_START..=CMD_NEC_GET_DIR_INFO => 10,
        _ => match (opcode >> 5) & 0x07 {
            0 => 6,
            1 | 2 => 10,
            4 => 16,
            5 => 12,
            _ => 6,
        },
    }
}

/// TOC ADR/control byte for a track type (data = 0x14, audio = 0x10).
fn track_control(track_type: TrackType) -> u8 {
    match track_type {
        TrackType::Data => 0x14,
        TrackType::Audio => 0x10,
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
