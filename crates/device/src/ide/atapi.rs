//! ATAPI (ATA Packet Interface) protocol and SCSI command handling.
//!
//! Implements the ATAPI packet protocol used by CD-ROM drives on the PC-98
//! IDE interface. ATAPI wraps SCSI commands inside ATA by using the PACKET
//! command (0xA0): the host writes a 12-byte SCSI Command Descriptor Block
//! (CDB) to the data register, and the device executes it.
//!
//! The SCSI command set implemented here covers the MMC (Multi-Media Commands)
//! subset needed for CD-ROM data access, TOC reading, and media change.

use crate::{
    cd_audio::{CdAudioPlayer, CdAudioState},
    cdrom::CdImage,
};

/// Maximum data buffer size (64 KB).
const MAX_BUFFER_SIZE: usize = 65536;

/// CD-ROM sector size for data reads.
const CDROM_SECTOR_SIZE: usize = 2048;

fn raw_user_data_offset(raw_sector: &[u8]) -> usize {
    if raw_sector.get(15) == Some(&0x02) {
        24
    } else {
        16
    }
}

// ATAPI signature registers (identifies device as ATAPI, not ATA).
pub(super) const ATAPI_SIGNATURE_CYLINDER_LOW: u8 = 0x14;
pub(super) const ATAPI_SIGNATURE_CYLINDER_HIGH: u8 = 0xEB;

// ATAPI interrupt reason register bits (overloaded on sector_count register).
// Used by the LLE controller directly (values 0x01, 0x02, 0x03).

// SCSI sense keys.
const SENSE_NO_SENSE: u8 = 0x00;
const SENSE_NOT_READY: u8 = 0x02;
const SENSE_ILLEGAL_REQUEST: u8 = 0x05;
const SENSE_UNIT_ATTENTION: u8 = 0x06;

// SCSI Additional Sense Codes (ASC).
const ASC_NO_ADDITIONAL_SENSE: u8 = 0x00;
const ASC_INVALID_COMMAND_OPERATION_CODE: u8 = 0x20;
const ASC_LOGICAL_BLOCK_OUT_OF_RANGE: u8 = 0x21;
const ASC_INVALID_FIELD_IN_CDB: u8 = 0x24;
const ASC_NOT_READY_TO_READY_TRANSITION: u8 = 0x28;
const ASC_SAVING_PARAMETERS_NOT_SUPPORTED: u8 = 0x39;
const ASC_MEDIUM_NOT_PRESENT: u8 = 0x3A;

// SCSI Additional Sense Code Qualifiers (ASCQ).
const ASCQ_NO_QUALIFIER: u8 = 0x00;

// ATA status bits used by ATAPI (defined in lle.rs, referenced here for documentation).
// STATUS_DRDY = 0x40, STATUS_DSC = 0x10, STATUS_DRQ = 0x08, STATUS_ERR/CHK = 0x01

/// ATAPI device state for the CD-ROM drive.
#[derive(Debug)]
pub(super) struct AtapiState {
    // Sense data (persistent until cleared by REQUEST SENSE).
    sense_key: u8,
    asc: u8,
    ascq: u8,

    // Media state.
    pub(super) media_loaded: bool,
    pub(super) media_changed: bool,
    prevent_removal: bool,

    // Packet command buffer.
    packet: [u8; 12],
    packet_position: usize,

    // NEC BCD address mode: when true, MSF addresses are BCD-encoded on output
    // and BCD-decoded on input. Set by MODE SENSE page 0x0F (neccdd.sys).
    pub(super) bcd_msf_mode: bool,

    // Data transfer buffer.
    data_buffer: Vec<u8>,
    data_position: usize,
    data_size: usize,
    byte_count_limit: u16,

    // Chunk tracking for per-sector interrupts during multi-sector reads.
    // Tracks how many bytes have been read within the current chunk (bounded
    // by byte_count_limit). When a chunk boundary is reached, the caller
    // fires another interrupt before the host reads the next chunk.
    pub(super) chunk_position: usize,
}

impl AtapiState {
    pub(super) fn new() -> Self {
        Self {
            sense_key: SENSE_NO_SENSE,
            asc: ASC_NO_ADDITIONAL_SENSE,
            ascq: ASCQ_NO_QUALIFIER,
            media_loaded: false,
            media_changed: false,
            prevent_removal: false,
            bcd_msf_mode: false,
            packet: [0u8; 12],
            packet_position: 0,
            data_buffer: Vec::with_capacity(MAX_BUFFER_SIZE),
            data_position: 0,
            data_size: 0,
            byte_count_limit: 0xFFFE,
            chunk_position: 0,
        }
    }

    /// Resets ATAPI state (called on DEVICE RESET command).
    pub(super) fn reset(&mut self) {
        self.bcd_msf_mode = false;
        self.sense_key = SENSE_NO_SENSE;
        self.asc = ASC_NO_ADDITIONAL_SENSE;
        self.ascq = ASCQ_NO_QUALIFIER;
        self.prevent_removal = false;
    }

    /// Sets the ATAPI signature in the drive registers.
    pub(super) fn set_signature(
        sector_count: &mut u8,
        sector_number: &mut u8,
        cylinder_low: &mut u8,
        cylinder_high: &mut u8,
    ) {
        *sector_count = 0x01;
        *sector_number = 0x01;
        *cylinder_low = ATAPI_SIGNATURE_CYLINDER_LOW;
        *cylinder_high = ATAPI_SIGNATURE_CYLINDER_HIGH;
    }

    /// Notifies that media has been inserted.
    pub(super) fn media_inserted(&mut self) {
        self.media_loaded = true;
        self.media_changed = true;
        self.sense_key = SENSE_UNIT_ATTENTION;
        self.asc = ASC_NOT_READY_TO_READY_TRANSITION;
        self.ascq = ASCQ_NO_QUALIFIER;
    }

    /// Notifies that media has been ejected.
    pub(super) fn media_ejected(&mut self) {
        self.media_loaded = false;
        self.media_changed = true;
        self.bcd_msf_mode = false;
        self.sense_key = SENSE_NOT_READY;
        self.asc = ASC_MEDIUM_NOT_PRESENT;
        self.ascq = ASCQ_NO_QUALIFIER;
    }

    /// Begins the PACKET command: saves byte_count_limit from cylinder registers,
    /// resets packet buffer, returns register values to set.
    pub(super) fn start_packet_command(&mut self, cylinder_low: u8, cylinder_high: u8) {
        self.byte_count_limit = u16::from(cylinder_low) | (u16::from(cylinder_high) << 8);
        if self.byte_count_limit == 0 {
            self.byte_count_limit = 0xFFFE;
        }
        self.packet_position = 0;
        self.chunk_position = 0;
    }

    /// Receives a word of packet data. Returns true when all 12 bytes have been received.
    pub(super) fn receive_packet_word(&mut self, value: u16) -> bool {
        if self.packet_position < 12 {
            self.packet[self.packet_position] = value as u8;
            if self.packet_position + 1 < 12 {
                self.packet[self.packet_position + 1] = (value >> 8) as u8;
            }
            self.packet_position += 2;
        }
        self.packet_position >= 12
    }

    /// Executes the received SCSI packet command.
    /// Returns (has_data, is_error): has_data means data_buffer is filled for transfer.
    pub(super) fn execute_packet(
        &mut self,
        cdrom: Option<&CdImage>,
        cd_audio: &mut CdAudioPlayer,
    ) -> (bool, bool) {
        let opcode = self.packet[0];
        match opcode {
            0x00 => self.cmd_test_unit_ready(),
            0x03 => self.cmd_request_sense(),
            0x12 => self.cmd_inquiry(),
            0x1B => self.cmd_start_stop_unit(),
            0x1E => self.cmd_prevent_allow_medium_removal(),
            0x25 => self.cmd_read_capacity(cdrom),
            0x28 => self.cmd_read_10(cdrom),
            0x2B => self.cmd_seek(cdrom),
            0x42 => self.cmd_read_sub_channel(cdrom, cd_audio),
            0x43 => self.cmd_read_toc(cdrom),
            0x45 => self.cmd_play_audio(cdrom, cd_audio),
            0x46 => self.cmd_get_configuration(),
            0x47 => self.cmd_play_audio_msf(cdrom, cd_audio),
            0x4B => self.cmd_pause_resume(cdrom, cd_audio),
            0x55 => self.cmd_mode_select_10(),
            0x5A => self.cmd_mode_sense_10(cdrom),
            0xB9 => self.cmd_read_cd_msf(cdrom),
            0xBD => self.cmd_mechanism_status(),
            0xBE => self.cmd_read_cd(cdrom),
            _ => self.cmd_unsupported(),
        }
    }

    /// Returns a word from the data buffer at the current position and advances.
    pub(super) fn read_data_word(&mut self) -> u16 {
        if self.data_position + 1 >= self.data_size {
            let low = if self.data_position < self.data_size {
                self.data_buffer[self.data_position] as u16
            } else {
                0
            };
            self.data_position = self.data_size;
            return low;
        }
        let low = self.data_buffer[self.data_position] as u16;
        let high = self.data_buffer[self.data_position + 1] as u16;
        self.data_position += 2;
        low | (high << 8)
    }

    /// Returns true if all data has been transferred.
    pub(super) fn transfer_complete(&self) -> bool {
        self.data_position >= self.data_size
    }

    /// Returns the current transfer size per DRQ assertion.
    ///
    /// For multi-sector reads (data aligned to CD sector boundaries), real
    /// ATAPI CD-ROM drives deliver one sector (2048 bytes) per DRQ assertion.
    /// For other responses the full data is delivered up to byte_count_limit.
    pub(super) fn current_transfer_size(&self) -> u16 {
        let remaining = (self.data_size - self.data_position) as u16;
        remaining.min(self.effective_chunk_size())
    }

    /// Returns true if the current DRQ chunk has been fully read by the host.
    pub(super) fn chunk_complete(&self) -> bool {
        self.chunk_position >= self.effective_chunk_size() as usize
    }

    /// Computes the effective chunk size for the current transfer.
    ///
    /// Multi-sector CD reads (data_size is a multiple of 2048 and spans more
    /// than one sector) use per-sector DRQ delivery. All other transfers use
    /// the host's byte_count_limit.
    fn effective_chunk_size(&self) -> u16 {
        let is_multi_sector_read =
            self.data_size > CDROM_SECTOR_SIZE && self.data_size.is_multiple_of(CDROM_SECTOR_SIZE);
        if is_multi_sector_read {
            (CDROM_SECTOR_SIZE as u16).min(self.byte_count_limit)
        } else {
            self.byte_count_limit
        }
    }

    /// Resets the chunk position for the next chunk transfer.
    pub(super) fn start_next_chunk(&mut self) {
        self.chunk_position = 0;
    }

    /// Returns the sense key for the error register (upper nibble).
    pub(super) fn error_register(&self) -> u8 {
        self.sense_key << 4
    }

    fn set_sense(&mut self, key: u8, asc: u8, ascq: u8) {
        self.sense_key = key;
        self.asc = asc;
        self.ascq = ascq;
    }

    fn cmd_complete_no_data(&mut self) -> (bool, bool) {
        self.data_size = 0;
        self.data_position = 0;
        (false, false)
    }

    fn cmd_complete_with_data(&mut self, size: usize) -> (bool, bool) {
        self.data_size = size;
        self.data_position = 0;
        self.set_sense(SENSE_NO_SENSE, ASC_NO_ADDITIONAL_SENSE, ASCQ_NO_QUALIFIER);
        (true, false)
    }

    fn cmd_error(&mut self, key: u8, asc: u8, ascq: u8) -> (bool, bool) {
        self.set_sense(key, asc, ascq);
        self.data_size = 0;
        self.data_position = 0;
        (false, true)
    }

    fn check_media_with_sense(&mut self) -> Result<(), (bool, bool)> {
        self.check_media_impl(false)
    }

    fn check_media_and_clear_attention(&mut self) -> Result<(), (bool, bool)> {
        self.check_media_impl(true)
    }

    fn check_media_impl(&mut self, clear_attention: bool) -> Result<(), (bool, bool)> {
        if self.media_changed {
            if clear_attention {
                self.media_changed = false;
            }
            if self.media_loaded {
                self.set_sense(
                    SENSE_UNIT_ATTENTION,
                    ASC_NOT_READY_TO_READY_TRANSITION,
                    ASCQ_NO_QUALIFIER,
                );
            } else {
                self.set_sense(SENSE_NOT_READY, ASC_MEDIUM_NOT_PRESENT, ASCQ_NO_QUALIFIER);
            }
            return Err((false, true));
        }
        if !self.media_loaded {
            self.set_sense(SENSE_NOT_READY, ASC_MEDIUM_NOT_PRESENT, ASCQ_NO_QUALIFIER);
            return Err((false, true));
        }
        Ok(())
    }

    // 0x00: TEST UNIT READY
    fn cmd_test_unit_ready(&mut self) -> (bool, bool) {
        if let Err(result) = self.check_media_and_clear_attention() {
            return result;
        }
        self.cmd_complete_no_data()
    }

    // 0x03: REQUEST SENSE
    fn cmd_request_sense(&mut self) -> (bool, bool) {
        let allocation_length = self.packet[4] as usize;
        let length = allocation_length.min(18);

        self.data_buffer.clear();
        self.data_buffer.resize(18, 0);
        self.data_buffer[0] = 0x70; // Response code: current errors, fixed format.
        self.data_buffer[2] = self.sense_key;
        self.data_buffer[7] = 10; // Additional sense length.
        self.data_buffer[12] = self.asc;
        self.data_buffer[13] = self.ascq;

        // Clear sense data after returning it.
        self.sense_key = SENSE_NO_SENSE;
        self.asc = ASC_NO_ADDITIONAL_SENSE;
        self.ascq = ASCQ_NO_QUALIFIER;
        self.media_changed = false;

        self.cmd_complete_with_data(length)
    }

    // 0x12: INQUIRY
    fn cmd_inquiry(&mut self) -> (bool, bool) {
        self.media_changed = false;

        let allocation_length = self.packet[4] as usize;
        let length = allocation_length.min(36);

        self.data_buffer.clear();
        self.data_buffer.resize(36, 0);
        self.data_buffer[0] = 0x05; // Device type: CD-ROM.
        self.data_buffer[1] = 0x80; // RMB: removable.
        self.data_buffer[2] = 0x00; // Version: no conformance.
        self.data_buffer[3] = 0x21; // Response data format: SPC-2.
        self.data_buffer[4] = 31; // Additional length.

        // Vendor identification (bytes 8-15).
        let vendor = b"NEC     ";
        self.data_buffer[8..16].copy_from_slice(vendor);

        // Product identification (bytes 16-31).
        let product = b"CD-ROM DRIVE:98 ";
        self.data_buffer[16..32].copy_from_slice(product);

        // Product revision (bytes 32-35).
        let revision = b"1.0 ";
        self.data_buffer[32..36].copy_from_slice(revision);

        self.cmd_complete_with_data(length)
    }

    // 0x1B: START/STOP UNIT
    fn cmd_start_stop_unit(&mut self) -> (bool, bool) {
        let loej = self.packet[4] & 0x02 != 0;
        let start = self.packet[4] & 0x01 != 0;

        if loej && !start {
            // Eject request.
            if self.prevent_removal {
                return self.cmd_error(
                    SENSE_ILLEGAL_REQUEST,
                    ASC_INVALID_FIELD_IN_CDB,
                    ASCQ_NO_QUALIFIER,
                );
            }
            self.media_loaded = false;
            self.media_changed = true;
        } else if loej && start {
            // Load request: re-load media if available (handled by media_inserted).
            // If no image is available, this is a no-op.
        }
        // start without loej: spin up/down, no-op for emulation.
        self.cmd_complete_no_data()
    }

    // 0x1E: PREVENT ALLOW MEDIUM REMOVAL
    fn cmd_prevent_allow_medium_removal(&mut self) -> (bool, bool) {
        self.prevent_removal = self.packet[4] & 0x01 != 0;
        self.cmd_complete_no_data()
    }

    // 0x25: READ CAPACITY
    fn cmd_read_capacity(&mut self, cdrom: Option<&CdImage>) -> (bool, bool) {
        if let Err(result) = self.check_media_with_sense() {
            return result;
        }
        let Some(cdrom) = cdrom else {
            return self.cmd_error(SENSE_NOT_READY, ASC_MEDIUM_NOT_PRESENT, ASCQ_NO_QUALIFIER);
        };

        let last_lba = cdrom.total_sectors().saturating_sub(1);
        let block_size: u32 = CDROM_SECTOR_SIZE as u32;

        self.data_buffer.clear();
        self.data_buffer.resize(8, 0);
        // Last LBA (big-endian).
        self.data_buffer[0] = (last_lba >> 24) as u8;
        self.data_buffer[1] = (last_lba >> 16) as u8;
        self.data_buffer[2] = (last_lba >> 8) as u8;
        self.data_buffer[3] = last_lba as u8;
        // Block size (big-endian).
        self.data_buffer[4] = (block_size >> 24) as u8;
        self.data_buffer[5] = (block_size >> 16) as u8;
        self.data_buffer[6] = (block_size >> 8) as u8;
        self.data_buffer[7] = block_size as u8;

        self.cmd_complete_with_data(8)
    }

    // 0x28: READ(10)
    fn cmd_read_10(&mut self, cdrom: Option<&CdImage>) -> (bool, bool) {
        if let Err(result) = self.check_media_with_sense() {
            return result;
        }
        let Some(cdrom) = cdrom else {
            return self.cmd_error(SENSE_NOT_READY, ASC_MEDIUM_NOT_PRESENT, ASCQ_NO_QUALIFIER);
        };

        let lba = u32::from(self.packet[2]) << 24
            | u32::from(self.packet[3]) << 16
            | u32::from(self.packet[4]) << 8
            | u32::from(self.packet[5]);
        let count = u16::from(self.packet[7]) << 8 | u16::from(self.packet[8]);

        if count == 0 {
            return self.cmd_complete_no_data();
        }

        let total_bytes = count as usize * CDROM_SECTOR_SIZE;
        self.data_buffer.clear();
        self.data_buffer.resize(total_bytes, 0);

        for i in 0..count as u32 {
            let sector_lba = lba + i;
            let offset = i as usize * CDROM_SECTOR_SIZE;
            if cdrom
                .read_sector(
                    sector_lba,
                    &mut self.data_buffer[offset..offset + CDROM_SECTOR_SIZE],
                )
                .is_none()
            {
                return self.cmd_error(
                    SENSE_ILLEGAL_REQUEST,
                    ASC_LOGICAL_BLOCK_OUT_OF_RANGE,
                    ASCQ_NO_QUALIFIER,
                );
            }
        }

        self.cmd_complete_with_data(total_bytes)
    }

    // 0x2B: SEEK(10)
    fn cmd_seek(&mut self, cdrom: Option<&CdImage>) -> (bool, bool) {
        if let Err(result) = self.check_media_with_sense() {
            return result;
        }
        let Some(cdrom) = cdrom else {
            return self.cmd_error(SENSE_NOT_READY, ASC_MEDIUM_NOT_PRESENT, ASCQ_NO_QUALIFIER);
        };

        let lba = u32::from(self.packet[2]) << 24
            | u32::from(self.packet[3]) << 16
            | u32::from(self.packet[4]) << 8
            | u32::from(self.packet[5]);

        if lba >= cdrom.total_sectors() {
            return self.cmd_error(
                SENSE_ILLEGAL_REQUEST,
                ASC_LOGICAL_BLOCK_OUT_OF_RANGE,
                ASCQ_NO_QUALIFIER,
            );
        }

        self.cmd_complete_no_data()
    }

    // 0x42: READ SUB-CHANNEL
    fn cmd_read_sub_channel(
        &mut self,
        cdrom: Option<&CdImage>,
        cd_audio: &CdAudioPlayer,
    ) -> (bool, bool) {
        if let Err(result) = self.check_media_with_sense() {
            return result;
        }

        let audio_status = match cd_audio.state() {
            CdAudioState::Playing => 0x11,
            CdAudioState::Paused => 0x12,
            CdAudioState::Stopped => 0x15,
        };

        let sub_q = self.packet[2] & 0x40 != 0;
        let format = self.packet[3];
        let msf = self.packet[1] & 0x02 != 0;
        let allocation_length = u16::from(self.packet[7]) << 8 | u16::from(self.packet[8]);

        if sub_q && format == 0x01 {
            // Format 0x01: Current Position - return 16-byte response.
            self.data_buffer.clear();
            self.data_buffer.resize(16, 0);
            self.data_buffer[0] = 0x00; // Reserved.
            self.data_buffer[1] = audio_status;
            self.data_buffer[2] = 0x00; // Sub-channel data length (MSB).
            self.data_buffer[3] = 0x0C; // Sub-channel data length = 12.
            self.data_buffer[4] = 0x01; // Sub-Q format code: current position.

            let (current_lba, _, _) = cd_audio.current_position();

            if let Some(sub_q) = cdrom.and_then(|cdrom| cdrom.read_subchannel_q(current_lba)) {
                decode_subq_into_response(&sub_q, &mut self.data_buffer, msf, self.bcd_msf_mode);
            } else if let Some(cdrom) = cdrom {
                let track = cdrom.track_for_lba(current_lba).or_else(|| cdrom.track(1));
                let adr_ctl = track.map_or(0x14, |t| match t.track_type {
                    crate::cdrom::TrackType::Data => 0x14,
                    crate::cdrom::TrackType::Audio => 0x10,
                });
                self.data_buffer[5] = adr_ctl;
                self.data_buffer[6] = track.map_or(1, |t| t.number);
                self.data_buffer[7] = 0x01;
                let track_relative_lba =
                    track.map_or(0, |t| current_lba.saturating_sub(t.start_lba));
                store_address(
                    &mut self.data_buffer[8..12],
                    current_lba,
                    msf,
                    self.bcd_msf_mode,
                );
                store_address(
                    &mut self.data_buffer[12..16],
                    track_relative_lba,
                    msf,
                    self.bcd_msf_mode,
                );
            } else {
                self.data_buffer[5] = 0x14;
                self.data_buffer[6] = 0x01;
                self.data_buffer[7] = 0x01;
                store_address(&mut self.data_buffer[8..12], 0, msf, self.bcd_msf_mode);
                store_address(&mut self.data_buffer[12..16], 0, msf, self.bcd_msf_mode);
            }

            let size = 16.min(allocation_length as usize);
            return self.cmd_complete_with_data(size);
        }

        // Default: return minimal 4-byte header with audio status.
        self.data_buffer.clear();
        self.data_buffer
            .resize(4.min(allocation_length as usize), 0);
        if !self.data_buffer.is_empty() {
            self.data_buffer[0] = 0x00; // Reserved.
        }
        if self.data_buffer.len() > 1 {
            self.data_buffer[1] = audio_status;
        }
        // Bytes 2-3: sub-channel data length = 0.

        let size = self.data_buffer.len();
        self.cmd_complete_with_data(size)
    }

    // 0x43: READ TOC
    fn cmd_read_toc(&mut self, cdrom: Option<&CdImage>) -> (bool, bool) {
        if let Err(result) = self.check_media_with_sense() {
            return result;
        }
        let Some(cdrom) = cdrom else {
            return self.cmd_error(SENSE_NOT_READY, ASC_MEDIUM_NOT_PRESENT, ASCQ_NO_QUALIFIER);
        };

        // Clear media_changed after successful media check.
        self.media_changed = false;

        let msf = self.packet[1] & 0x02 != 0;
        let format = self.packet[2] & 0x0F;
        if self.packet[9] & 0xC0 != 0 {
            // Format field in byte 9 bits 6-7 (SFF-8020i).
            let fmt2 = (self.packet[9] >> 6) & 0x03;
            if fmt2 != 0 {
                return self.read_toc_format(fmt2, msf, cdrom);
            }
        }
        self.read_toc_format(format, msf, cdrom)
    }

    fn read_toc_format(&mut self, format: u8, msf: bool, cdrom: &CdImage) -> (bool, bool) {
        let allocation_length = u16::from(self.packet[7]) << 8 | u16::from(self.packet[8]);
        let starting_track = self.packet[6];

        match format {
            0 => self.read_toc_format_0(cdrom, starting_track, allocation_length, msf),
            1 => self.read_toc_format_1(cdrom, allocation_length, msf),
            2 => self.read_toc_format_2(cdrom, allocation_length),
            _ => self.cmd_error(
                SENSE_ILLEGAL_REQUEST,
                ASC_INVALID_FIELD_IN_CDB,
                ASCQ_NO_QUALIFIER,
            ),
        }
    }

    // Format 0: TOC (track list + lead-out).
    fn read_toc_format_0(
        &mut self,
        cdrom: &CdImage,
        starting_track: u8,
        allocation_length: u16,
        msf: bool,
    ) -> (bool, bool) {
        let tracks = cdrom.tracks();
        let track_count = cdrom.track_count();
        let first_track = if starting_track == 0 {
            1
        } else {
            starting_track
        };

        // Filter tracks >= starting_track.
        let valid_tracks: Vec<&crate::cdrom::Track> =
            tracks.iter().filter(|t| t.number >= first_track).collect();

        // Header (4 bytes) + track descriptors (8 bytes each) + lead-out (8 bytes).
        let descriptor_count = valid_tracks.len() + 1; // +1 for lead-out
        let data_length = 2 + descriptor_count * 8; // 2 bytes header after length field
        let total_length = 2 + data_length; // +2 for length field itself

        self.data_buffer.clear();
        self.data_buffer.resize(total_length, 0);

        // TOC header.
        self.data_buffer[0] = (data_length >> 8) as u8;
        self.data_buffer[1] = data_length as u8;
        self.data_buffer[2] = 1; // First track number.
        self.data_buffer[3] = track_count; // Last track number.

        // Track descriptors.
        let mut offset = 4;
        for track in &valid_tracks {
            self.data_buffer[offset] = 0; // Reserved.
            self.data_buffer[offset + 1] = match track.track_type {
                crate::cdrom::TrackType::Data => 0x14,  // ADR/CTL: data track.
                crate::cdrom::TrackType::Audio => 0x10, // ADR/CTL: audio track.
            };
            self.data_buffer[offset + 2] = track.number;
            self.data_buffer[offset + 3] = 0; // Reserved.
            store_address(
                &mut self.data_buffer[offset + 4..offset + 8],
                track.start_lba,
                msf,
                self.bcd_msf_mode,
            );
            offset += 8;
        }

        // Lead-out entry (track 0xAA).
        let lead_out_lba = cdrom.total_sectors();
        self.data_buffer[offset] = 0;
        self.data_buffer[offset + 1] = 0x14; // ADR/CTL: data.
        self.data_buffer[offset + 2] = 0xAA; // Lead-out track number.
        self.data_buffer[offset + 3] = 0;
        store_address(
            &mut self.data_buffer[offset + 4..offset + 8],
            lead_out_lba,
            msf,
            self.bcd_msf_mode,
        );

        let size = total_length.min(allocation_length as usize);
        self.cmd_complete_with_data(size)
    }

    // Format 1: Session info.
    fn read_toc_format_1(
        &mut self,
        cdrom: &CdImage,
        allocation_length: u16,
        msf: bool,
    ) -> (bool, bool) {
        self.data_buffer.clear();
        self.data_buffer.resize(12, 0);

        // Header.
        self.data_buffer[0] = 0x00;
        self.data_buffer[1] = 0x0A; // Data length = 10.
        self.data_buffer[2] = 1; // First session.
        self.data_buffer[3] = 1; // Last session.

        // Session descriptor: first track of last session.
        self.data_buffer[4] = 0; // Reserved.
        self.data_buffer[5] = 0x14; // ADR/CTL.
        self.data_buffer[6] = 1; // First track in session.
        self.data_buffer[7] = 0; // Reserved.
        let lba = cdrom.track(1).map_or(0, |t| t.start_lba);
        store_address(&mut self.data_buffer[8..12], lba, msf, self.bcd_msf_mode);

        let size = 12.min(allocation_length as usize);
        self.cmd_complete_with_data(size)
    }

    // Format 2: Full TOC (raw Q sub-channel).
    fn read_toc_format_2(&mut self, cdrom: &CdImage, allocation_length: u16) -> (bool, bool) {
        let tracks = cdrom.tracks();
        let track_count = cdrom.track_count();

        // Header (4 bytes) + A0 entry (11 bytes) + A1 entry (11 bytes) + A2 entry (11 bytes)
        // + track entries (11 bytes each).
        let entry_count = 3 + tracks.len();
        let data_length = 2 + entry_count * 11;
        let total_length = 2 + data_length;

        self.data_buffer.clear();
        self.data_buffer.resize(total_length, 0);

        // Header.
        self.data_buffer[0] = (data_length >> 8) as u8;
        self.data_buffer[1] = data_length as u8;
        self.data_buffer[2] = 1; // First session.
        self.data_buffer[3] = 1; // Last session.

        let mut offset = 4;

        // Point A0: first track number.
        self.data_buffer[offset] = 1; // Session.
        self.data_buffer[offset + 1] = 0x14; // ADR/CTL.
        self.data_buffer[offset + 2] = 0; // TNO.
        self.data_buffer[offset + 3] = 0xA0; // Point.
        // PMIN = first track number.
        self.data_buffer[offset + 8] = 1;
        offset += 11;

        // Point A1: last track number.
        self.data_buffer[offset] = 1;
        self.data_buffer[offset + 1] = 0x14;
        self.data_buffer[offset + 2] = 0;
        self.data_buffer[offset + 3] = 0xA1;
        self.data_buffer[offset + 8] = track_count;
        offset += 11;

        // Point A2: lead-out position in MSF.
        let lead_out = cdrom.total_sectors();
        let (m, s, f) = lba_to_msf(lead_out);
        self.data_buffer[offset] = 1;
        self.data_buffer[offset + 1] = 0x14;
        self.data_buffer[offset + 2] = 0;
        self.data_buffer[offset + 3] = 0xA2;
        if self.bcd_msf_mode {
            self.data_buffer[offset + 8] = hex_to_bcd(m);
            self.data_buffer[offset + 9] = hex_to_bcd(s);
            self.data_buffer[offset + 10] = hex_to_bcd(f);
        } else {
            self.data_buffer[offset + 8] = m;
            self.data_buffer[offset + 9] = s;
            self.data_buffer[offset + 10] = f;
        }
        offset += 11;

        // Track entries.
        for track in tracks {
            let ctl = match track.track_type {
                crate::cdrom::TrackType::Data => 0x14,
                crate::cdrom::TrackType::Audio => 0x10,
            };
            let (m, s, f) = lba_to_msf(track.start_lba);
            self.data_buffer[offset] = 1; // Session.
            self.data_buffer[offset + 1] = ctl;
            self.data_buffer[offset + 2] = 0; // TNO.
            self.data_buffer[offset + 3] = track.number; // Point.
            if self.bcd_msf_mode {
                self.data_buffer[offset + 8] = hex_to_bcd(m); // PMIN.
                self.data_buffer[offset + 9] = hex_to_bcd(s); // PSEC.
                self.data_buffer[offset + 10] = hex_to_bcd(f); // PFRAME.
            } else {
                self.data_buffer[offset + 8] = m; // PMIN.
                self.data_buffer[offset + 9] = s; // PSEC.
                self.data_buffer[offset + 10] = f; // PFRAME.
            }
            offset += 11;
        }

        let size = total_length.min(allocation_length as usize);
        self.cmd_complete_with_data(size)
    }

    // 0x45: PLAY AUDIO(10)
    fn cmd_play_audio(
        &mut self,
        cdrom: Option<&CdImage>,
        cd_audio: &mut CdAudioPlayer,
    ) -> (bool, bool) {
        if let Err(result) = self.check_media_with_sense() {
            return result;
        }
        let Some(cdrom) = cdrom else {
            return self.cmd_error(SENSE_NOT_READY, ASC_MEDIUM_NOT_PRESENT, ASCQ_NO_QUALIFIER);
        };
        let start_lba = u32::from(self.packet[2]) << 24
            | u32::from(self.packet[3]) << 16
            | u32::from(self.packet[4]) << 8
            | u32::from(self.packet[5]);
        let transfer_length = u16::from(self.packet[7]) << 8 | u16::from(self.packet[8]);
        cd_audio.play(cdrom, start_lba, u32::from(transfer_length));
        self.cmd_complete_no_data()
    }

    // 0x46: GET CONFIGURATION
    fn cmd_get_configuration(&mut self) -> (bool, bool) {
        let allocation_length = u16::from(self.packet[7]) << 8 | u16::from(self.packet[8]);

        self.data_buffer.clear();
        self.data_buffer.resize(8, 0);

        // Feature header: data length (excluding first 4 bytes).
        let data_length: u32 = 4;
        self.data_buffer[0] = (data_length >> 24) as u8;
        self.data_buffer[1] = (data_length >> 16) as u8;
        self.data_buffer[2] = (data_length >> 8) as u8;
        self.data_buffer[3] = data_length as u8;
        // Current profile: 0x0008 (CD-ROM).
        self.data_buffer[6] = 0x00;
        self.data_buffer[7] = 0x08;

        let size = 8.min(allocation_length as usize);
        self.cmd_complete_with_data(size)
    }

    // 0x47: PLAY AUDIO MSF
    fn cmd_play_audio_msf(
        &mut self,
        cdrom: Option<&CdImage>,
        cd_audio: &mut CdAudioPlayer,
    ) -> (bool, bool) {
        if let Err(result) = self.check_media_with_sense() {
            return result;
        }
        let Some(cdrom) = cdrom else {
            return self.cmd_error(SENSE_NOT_READY, ASC_MEDIUM_NOT_PRESENT, ASCQ_NO_QUALIFIER);
        };
        let start_m = u32::from(self.packet[3]);
        let start_s = u32::from(self.packet[4]);
        let start_f = u32::from(self.packet[5]);
        let end_m = u32::from(self.packet[6]);
        let end_s = u32::from(self.packet[7]);
        let end_f = u32::from(self.packet[8]);
        let start_lba = msf_to_lba(start_m, start_s, start_f);
        let end_lba = msf_to_lba(end_m, end_s, end_f);
        let sector_count = end_lba.saturating_sub(start_lba);
        cd_audio.play(cdrom, start_lba, sector_count);
        self.cmd_complete_no_data()
    }

    // 0x4B: PAUSE/RESUME
    fn cmd_pause_resume(
        &mut self,
        cdrom: Option<&CdImage>,
        cd_audio: &mut CdAudioPlayer,
    ) -> (bool, bool) {
        let resume = self.packet[8] & 0x01 != 0;
        if resume {
            if let Some(cdrom) = cdrom {
                cd_audio.resume(cdrom);
            }
        } else {
            cd_audio.stop();
        }
        self.cmd_complete_no_data()
    }

    // 0x55: MODE SELECT(10)
    fn cmd_mode_select_10(&mut self) -> (bool, bool) {
        let save_pages = self.packet[1] & 0x01 != 0;
        if save_pages {
            return self.cmd_error(
                SENSE_ILLEGAL_REQUEST,
                ASC_SAVING_PARAMETERS_NOT_SUPPORTED,
                ASCQ_NO_QUALIFIER,
            );
        }
        self.cmd_complete_no_data()
    }

    // 0x5A: MODE SENSE(10)
    fn cmd_mode_sense_10(&mut self, cdrom: Option<&CdImage>) -> (bool, bool) {
        let page_code = self.packet[2] & 0x3F;
        let allocation_length = u16::from(self.packet[7]) << 8 | u16::from(self.packet[8]);

        self.data_buffer.clear();
        self.data_buffer
            .resize(MAX_BUFFER_SIZE.min(allocation_length as usize), 0);

        // Mode parameter header (8 bytes for MODE SENSE(10)).
        let mut offset = 8;

        match page_code {
            0x01 => offset = self.mode_page_01(offset),
            0x0D => offset = self.mode_page_0d(offset),
            0x0E => offset = self.mode_page_0e(offset),
            0x0F => {
                offset = self.mode_page_0f(offset);
                self.bcd_msf_mode = true;
            }
            0x2A => offset = self.mode_page_2a(offset),
            0x3F => {
                offset = self.mode_page_01(offset);
                offset = self.mode_page_0d(offset);
                offset = self.mode_page_0e(offset);
                offset = self.mode_page_0f(offset);
                self.bcd_msf_mode = true;
                offset = self.mode_page_2a(offset);
            }
            _ => {
                return self.cmd_error(
                    SENSE_ILLEGAL_REQUEST,
                    ASC_INVALID_FIELD_IN_CDB,
                    ASCQ_NO_QUALIFIER,
                );
            }
        }

        // The header must describe the truncated payload that will actually be
        // transferred, not the full unbounded page set. Real-mode CD drivers
        // issue short MODE SENSE requests and reject replies whose length field
        // advertises more bytes than the command returns.
        let size = offset.min(allocation_length as usize);
        let data_length = size.saturating_sub(2) as u16;
        self.data_buffer[0] = (data_length >> 8) as u8;
        self.data_buffer[1] = data_length as u8;
        // Byte 2: medium type.
        self.data_buffer[2] = medium_type(cdrom);

        self.cmd_complete_with_data(size)
    }

    // Page 0x01: Read Error Recovery Parameters.
    fn mode_page_01(&mut self, offset: usize) -> usize {
        let end = offset + 8;
        if end > self.data_buffer.len() {
            self.data_buffer.resize(end, 0);
        }
        self.data_buffer[offset] = 0x01; // Page code.
        self.data_buffer[offset + 1] = 0x06; // Page length.
        end
    }

    // Page 0x0D: CD-ROM Device Parameters.
    fn mode_page_0d(&mut self, offset: usize) -> usize {
        let end = offset + 8;
        if end > self.data_buffer.len() {
            self.data_buffer.resize(end, 0);
        }
        self.data_buffer[offset] = 0x0D; // Page code.
        self.data_buffer[offset + 1] = 0x06; // Page length.
        self.data_buffer[offset + 5] = 0x3C; // Inactivity timer multiplier.
        self.data_buffer[offset + 7] = 0x4B; // Number of MSF-S units per MSF-M unit (75).
        end
    }

    // Page 0x0E: CD-ROM Audio Control Parameters.
    fn mode_page_0e(&mut self, offset: usize) -> usize {
        let end = offset + 16;
        if end > self.data_buffer.len() {
            self.data_buffer.resize(end, 0);
        }
        self.data_buffer[offset] = 0x0E; // Page code.
        self.data_buffer[offset + 1] = 0x0E; // Page length.
        // Audio play control parameters.
        self.data_buffer[offset + 2] = 0x04; // Immed = 1.
        self.data_buffer[offset + 7] = 0x4B; // Number of frames per second (75).
        // Port 0: channel 0, volume 0xFF.
        self.data_buffer[offset + 8] = 0x01;
        self.data_buffer[offset + 9] = 0xFF;
        // Port 1: channel 1, volume 0xFF.
        self.data_buffer[offset + 10] = 0x02;
        self.data_buffer[offset + 11] = 0xFF;
        end
    }

    // Page 0x0F: NEC vendor-specific CD-ROM parameters.
    //
    // The page-length byte says 0x10, but the total structure exposed to the
    // guest is 16 bytes including the page code and length bytes. Old DOS
    // drivers expect this exact compact layout.
    fn mode_page_0f(&mut self, offset: usize) -> usize {
        let end = offset + 16;
        if end > self.data_buffer.len() {
            self.data_buffer.resize(end, 0);
        }
        self.data_buffer[offset] = 0x0F; // Page code (vendor specific).
        self.data_buffer[offset + 1] = 0x10; // Page length.
        // Byte 4: NEC drive capability flags for neccdd.sys.
        //   bit 0 = audio play supported  (from page 0x2A byte 4, bit 0)
        //   bit 7 = lock state            (from page 0x2A byte 6, bit 1)
        // Bit 7 is required for CD-audio playback under MS-DOS.
        self.data_buffer[offset + 4] = 0x81;
        // Byte 5 carries the related audio capability bits expected by the
        // same vendor-specific interface.
        self.data_buffer[offset + 5] = 0x18;
        end
    }

    // Page 0x2A: CD-ROM Capabilities & Mechanical Status.
    fn mode_page_2a(&mut self, offset: usize) -> usize {
        let end = offset + 20;
        if end > self.data_buffer.len() {
            self.data_buffer.resize(end, 0);
        }
        self.data_buffer[offset] = 0x2A; // Page code.
        self.data_buffer[offset + 1] = 0x12; // Page length (18 bytes).
        // Capability bytes chosen to match the behavior expected by DOS-era
        // ATAPI CD-ROM drivers. Page 0x0F above mirrors a subset of these
        // flags through the vendor-specific interface.
        self.data_buffer[offset + 4] = 0x71;
        self.data_buffer[offset + 5] = 0x65;
        self.data_buffer[offset + 6] = 0x2B;
        self.data_buffer[offset + 7] = 0x07;
        // Max speed: 4x (706 KB/s).
        self.data_buffer[offset + 8] = 0x02;
        self.data_buffer[offset + 9] = 0xC2;
        self.data_buffer[offset + 11] = 0xFF;
        // Buffer size (64 KB).
        self.data_buffer[offset + 12] = 0x00;
        self.data_buffer[offset + 13] = 0x80;
        // Current speed: 4x.
        self.data_buffer[offset + 14] = 0x02;
        self.data_buffer[offset + 15] = 0xC2;
        end
    }

    // 0xB9: READ CD MSF
    fn cmd_read_cd_msf(&mut self, cdrom: Option<&CdImage>) -> (bool, bool) {
        if let Err(result) = self.check_media_with_sense() {
            return result;
        }
        let Some(cdrom) = cdrom else {
            return self.cmd_error(SENSE_NOT_READY, ASC_MEDIUM_NOT_PRESENT, ASCQ_NO_QUALIFIER);
        };

        let (start_m, start_s, start_f, end_m, end_s, end_f) = if self.bcd_msf_mode {
            (
                u32::from(bcd_to_hex(self.packet[3])),
                u32::from(bcd_to_hex(self.packet[4])),
                u32::from(bcd_to_hex(self.packet[5])),
                u32::from(bcd_to_hex(self.packet[6])),
                u32::from(bcd_to_hex(self.packet[7])),
                u32::from(bcd_to_hex(self.packet[8])),
            )
        } else {
            (
                u32::from(self.packet[3]),
                u32::from(self.packet[4]),
                u32::from(self.packet[5]),
                u32::from(self.packet[6]),
                u32::from(self.packet[7]),
                u32::from(self.packet[8]),
            )
        };
        let flags = self.packet[9];

        let start_lba = msf_to_lba(start_m, start_s, start_f);
        let end_lba = msf_to_lba(end_m, end_s, end_f);

        if end_lba <= start_lba {
            return self.cmd_complete_no_data();
        }

        let count = end_lba - start_lba;

        // MMC-3 header code field (byte 9 bits 6-5).
        let header_code = (flags >> 5) & 0x03;
        let want_sync = flags & 0x80 != 0;
        let want_header = header_code == 0x01 || header_code == 0x03;
        let want_sub_header = header_code == 0x02 || header_code == 0x03;
        let want_user_data = flags & 0x10 != 0;
        let want_edc_ecc = flags & 0x08 != 0;

        // Wuirk: sync is only returned when header is also requested.
        let want_sync = want_sync && want_header;

        let mut sector_transfer_size: usize = 0;
        if want_sync {
            sector_transfer_size += 12;
        }
        if want_header {
            sector_transfer_size += 4;
        }
        if want_sub_header {
            sector_transfer_size += 8;
        }
        if want_user_data {
            sector_transfer_size += CDROM_SECTOR_SIZE;
        }
        if want_edc_ecc {
            sector_transfer_size += 288;
        }
        if sector_transfer_size == 0 {
            sector_transfer_size = CDROM_SECTOR_SIZE;
        }

        let total_bytes = count as usize * sector_transfer_size;
        self.data_buffer.clear();
        self.data_buffer.resize(total_bytes, 0);

        let mut raw_buf = [0u8; 2352];
        for i in 0..count {
            let sector_lba = start_lba + i;
            let out_offset = i as usize * sector_transfer_size;

            if let Some(raw_size) = cdrom.read_sector_raw(sector_lba, &mut raw_buf) {
                if raw_size == 2352 {
                    let user_data_offset = raw_user_data_offset(&raw_buf);
                    let is_mode2 = user_data_offset == 24;
                    let mut pos = out_offset;
                    if want_sync {
                        self.data_buffer[pos..pos + 12].copy_from_slice(&raw_buf[0..12]);
                        pos += 12;
                    }
                    if want_header {
                        self.data_buffer[pos..pos + 4].copy_from_slice(&raw_buf[12..16]);
                        pos += 4;
                    }
                    if want_sub_header {
                        if is_mode2 {
                            self.data_buffer[pos..pos + 8].copy_from_slice(&raw_buf[16..24]);
                        } else {
                            self.data_buffer[pos..pos + 8].fill(0);
                        }
                        pos += 8;
                    }
                    if want_user_data {
                        self.data_buffer[pos..pos + CDROM_SECTOR_SIZE].copy_from_slice(
                            &raw_buf[user_data_offset..user_data_offset + CDROM_SECTOR_SIZE],
                        );
                        pos += CDROM_SECTOR_SIZE;
                    }
                    if want_edc_ecc {
                        let edc_start = user_data_offset + CDROM_SECTOR_SIZE;
                        let edc_end = 2352;
                        let actual = edc_end - edc_start;
                        self.data_buffer[pos..pos + actual]
                            .copy_from_slice(&raw_buf[edc_start..edc_end]);
                    }
                } else {
                    self.data_buffer[out_offset..out_offset + raw_size.min(sector_transfer_size)]
                        .copy_from_slice(&raw_buf[..raw_size.min(sector_transfer_size)]);
                }
            } else {
                return self.cmd_error(
                    SENSE_ILLEGAL_REQUEST,
                    ASC_LOGICAL_BLOCK_OUT_OF_RANGE,
                    ASCQ_NO_QUALIFIER,
                );
            }
        }

        self.cmd_complete_with_data(total_bytes)
    }

    // 0xBD: MECHANISM STATUS
    fn cmd_mechanism_status(&mut self) -> (bool, bool) {
        let allocation_length = u16::from(self.packet[8]) << 8 | u16::from(self.packet[9]);

        self.data_buffer.clear();
        self.data_buffer.resize(8, 0);

        // Mechanism status header.
        self.data_buffer[0] = 0x00; // Fault = 0, changer state = ready.
        if self.media_loaded {
            self.data_buffer[1] = 0x20; // Door closed, disc present.
        } else {
            self.data_buffer[1] = 0x00; // Door closed, no disc.
        }
        // Bytes 2-4: current LBA (0).
        // Bytes 5: number of slots available.
        self.data_buffer[5] = 1;
        // Bytes 6-7: slot table length = 0.

        let size = 8.min(allocation_length as usize);
        self.cmd_complete_with_data(size)
    }

    // 0xBE: READ CD
    fn cmd_read_cd(&mut self, cdrom: Option<&CdImage>) -> (bool, bool) {
        if let Err(result) = self.check_media_with_sense() {
            return result;
        }
        let Some(cdrom) = cdrom else {
            return self.cmd_error(SENSE_NOT_READY, ASC_MEDIUM_NOT_PRESENT, ASCQ_NO_QUALIFIER);
        };

        let lba = u32::from(self.packet[2]) << 24
            | u32::from(self.packet[3]) << 16
            | u32::from(self.packet[4]) << 8
            | u32::from(self.packet[5]);
        let count = u32::from(self.packet[6]) << 16
            | u32::from(self.packet[7]) << 8
            | u32::from(self.packet[8]);
        let flags = self.packet[9];

        if count == 0 {
            return self.cmd_complete_no_data();
        }

        // MMC-3 header code field (byte 9 bits 6-5).
        let header_code = (flags >> 5) & 0x03;
        let want_sync = flags & 0x80 != 0;
        let want_header = header_code == 0x01 || header_code == 0x03;
        let want_sub_header = header_code == 0x02 || header_code == 0x03;
        let want_user_data = flags & 0x10 != 0;
        let want_edc_ecc = flags & 0x08 != 0;

        // Quirk: sync is only returned when header is also requested.
        let want_sync = want_sync && want_header;

        let mut sector_transfer_size: usize = 0;
        if want_sync {
            sector_transfer_size += 12;
        }
        if want_header {
            sector_transfer_size += 4;
        }
        if want_sub_header {
            sector_transfer_size += 8;
        }
        if want_user_data {
            sector_transfer_size += CDROM_SECTOR_SIZE;
        }
        if want_edc_ecc {
            sector_transfer_size += 288;
        }

        if sector_transfer_size == 0 {
            sector_transfer_size = CDROM_SECTOR_SIZE;
        }

        let total_bytes = count as usize * sector_transfer_size;
        self.data_buffer.clear();
        self.data_buffer.resize(total_bytes, 0);

        let mut raw_buf = [0u8; 2352];
        for i in 0..count {
            let sector_lba = lba + i;
            let out_offset = i as usize * sector_transfer_size;

            if let Some(raw_size) = cdrom.read_sector_raw(sector_lba, &mut raw_buf) {
                if raw_size == 2352 {
                    let user_data_offset = raw_user_data_offset(&raw_buf);
                    let is_mode2 = user_data_offset == 24;
                    let mut pos = out_offset;
                    if want_sync {
                        self.data_buffer[pos..pos + 12].copy_from_slice(&raw_buf[0..12]);
                        pos += 12;
                    }
                    if want_header {
                        self.data_buffer[pos..pos + 4].copy_from_slice(&raw_buf[12..16]);
                        pos += 4;
                    }
                    if want_sub_header {
                        if is_mode2 {
                            self.data_buffer[pos..pos + 8].copy_from_slice(&raw_buf[16..24]);
                        } else {
                            self.data_buffer[pos..pos + 8].fill(0);
                        }
                        pos += 8;
                    }
                    if want_user_data {
                        self.data_buffer[pos..pos + CDROM_SECTOR_SIZE].copy_from_slice(
                            &raw_buf[user_data_offset..user_data_offset + CDROM_SECTOR_SIZE],
                        );
                        pos += CDROM_SECTOR_SIZE;
                    }
                    if want_edc_ecc {
                        let edc_start = user_data_offset + CDROM_SECTOR_SIZE;
                        let edc_end = 2352;
                        let actual = edc_end - edc_start;
                        self.data_buffer[pos..pos + actual]
                            .copy_from_slice(&raw_buf[edc_start..edc_end]);
                    }
                } else {
                    self.data_buffer[out_offset..out_offset + raw_size.min(sector_transfer_size)]
                        .copy_from_slice(&raw_buf[..raw_size.min(sector_transfer_size)]);
                }
            } else {
                return self.cmd_error(
                    SENSE_ILLEGAL_REQUEST,
                    ASC_LOGICAL_BLOCK_OUT_OF_RANGE,
                    ASCQ_NO_QUALIFIER,
                );
            }
        }

        self.cmd_complete_with_data(total_bytes)
    }

    fn cmd_unsupported(&mut self) -> (bool, bool) {
        self.cmd_error(
            SENSE_ILLEGAL_REQUEST,
            ASC_INVALID_COMMAND_OPERATION_CODE,
            ASCQ_NO_QUALIFIER,
        )
    }
}

/// Converts a binary value to BCD (Binary-Coded Decimal).
fn hex_to_bcd(val: u8) -> u8 {
    ((val / 10) % 10) << 4 | (val % 10)
}

/// Converts a BCD (Binary-Coded Decimal) value to binary.
fn bcd_to_hex(val: u8) -> u8 {
    (val >> 4) * 10 + (val & 0x0F)
}

/// Converts an LBA to MSF (minute, second, frame).
fn lba_to_msf(lba: u32) -> (u8, u8, u8) {
    let total_frames = lba;
    let f = (total_frames % 75) as u8;
    let total_seconds = total_frames / 75;
    let s = (total_seconds % 60) as u8;
    let m = (total_seconds / 60) as u8;
    (m, s, f)
}

/// Converts MSF (minute, second, frame) to an absolute LBA.
/// MSF addresses include the 150-frame lead-in offset, so we subtract it.
fn msf_to_lba(m: u32, s: u32, f: u32) -> u32 {
    (m * 60 * 75 + s * 75 + f).saturating_sub(150)
}

/// Returns the medium type byte for MODE SENSE header byte 2.
fn medium_type(cdrom: Option<&CdImage>) -> u8 {
    let Some(cdrom) = cdrom else {
        return 0x70; // Door closed, no disc.
    };
    let mut has_data = false;
    let mut has_audio = false;
    for track in cdrom.tracks() {
        match track.track_type {
            crate::cdrom::TrackType::Data => has_data = true,
            crate::cdrom::TrackType::Audio => has_audio = true,
        }
    }
    match (has_data, has_audio) {
        (true, true) => 0x03,  // Data and audio.
        (true, false) => 0x01, // Data only.
        (false, true) => 0x02, // Audio only.
        (false, false) => 0x70,
    }
}

/// Writes a 4-byte address field as either MSF or LBA.
/// For MSF: adds the standard 150-frame (2-second) lead-in offset per Red Book.
/// When `bcd` is true, MSF values are BCD-encoded (NEC CD-ROM quirk).
///
/// Writes the bytes 5..16 of a `READ SUBCHANNEL` format-0x01 response from
/// the 12-byte raw Sub-Q recovered from disc.
///
/// Sub-Q byte 0 stores Control in the high nibble and ADR in the low nibble
/// (Red Book); the MMC response byte 5 uses the opposite nibble order. The
/// MSF fields in Sub-Q are BCD; the absolute MSF includes the standard
/// 150-sector lead-in, the relative MSF does not.
fn decode_subq_into_response(sub_q: &[u8; 12], buffer: &mut [u8], msf: bool, bcd: bool) {
    buffer[5] = ((sub_q[0] & 0x0F) << 4) | ((sub_q[0] & 0xF0) >> 4);
    buffer[6] = bcd_to_hex(sub_q[1]);
    buffer[7] = bcd_to_hex(sub_q[2]);

    write_subq_address(
        &mut buffer[8..12],
        bcd_to_hex(sub_q[7]),
        bcd_to_hex(sub_q[8]),
        bcd_to_hex(sub_q[9]),
        true,
        msf,
        bcd,
    );
    write_subq_address(
        &mut buffer[12..16],
        bcd_to_hex(sub_q[3]),
        bcd_to_hex(sub_q[4]),
        bcd_to_hex(sub_q[5]),
        false,
        msf,
        bcd,
    );
}

fn write_subq_address(buf: &mut [u8], m: u8, s: u8, f: u8, absolute: bool, msf: bool, bcd: bool) {
    if msf {
        buf[0] = 0;
        if bcd {
            buf[1] = hex_to_bcd(m);
            buf[2] = hex_to_bcd(s);
            buf[3] = hex_to_bcd(f);
        } else {
            buf[1] = m;
            buf[2] = s;
            buf[3] = f;
        }
    } else {
        let frames = u32::from(m) * 60 * 75 + u32::from(s) * 75 + u32::from(f);
        let lba = if absolute {
            frames.saturating_sub(150)
        } else {
            frames
        };
        buf[0] = (lba >> 24) as u8;
        buf[1] = (lba >> 16) as u8;
        buf[2] = (lba >> 8) as u8;
        buf[3] = lba as u8;
    }
}

fn store_address(buf: &mut [u8], lba: u32, msf: bool, bcd: bool) {
    if msf {
        let (m, s, f) = lba_to_msf(lba + 150);
        buf[0] = 0;
        if bcd {
            if m > 99 {
                buf[1] = 0xFF;
                buf[2] = 0x59;
                buf[3] = 0x74;
            } else {
                buf[1] = hex_to_bcd(m);
                buf[2] = hex_to_bcd(s);
                buf[3] = hex_to_bcd(f);
            }
        } else {
            buf[1] = m;
            buf[2] = s;
            buf[3] = f;
        }
    } else {
        buf[0] = (lba >> 24) as u8;
        buf[1] = (lba >> 16) as u8;
        buf[2] = (lba >> 8) as u8;
        buf[3] = lba as u8;
    }
}

/// Builds the 512-byte IDENTIFY PACKET DEVICE response for an ATAPI CD-ROM.
pub(super) fn build_identify_packet_device(buffer: &mut [u8]) {
    buffer.fill(0);

    let set_word = |buf: &mut [u8], word_index: usize, value: u16| {
        let byte_index = word_index * 2;
        buf[byte_index] = value as u8;
        buf[byte_index + 1] = (value >> 8) as u8;
    };

    // Word 0: General configuration.
    // Bit 15-14: 10 = ATAPI device.
    // Bit 12-8: 00101 = CD-ROM device type.
    // Bit 7: 1 = removable media.
    // Bit 6-5: 00 = 12-byte command packet.
    // Bit 1-0: 00 = microprocessor DRQ (fast DRQ after PACKET).
    set_word(buffer, 0, 0x8580);

    // Words 10-19: Serial number (20 ASCII chars).
    let serial = b"RPC98CDROM000000    ";
    for (i, chunk) in serial.chunks(2).enumerate() {
        set_word(
            buffer,
            10 + i,
            u16::from(chunk[0]) << 8 | u16::from(chunk[1]),
        );
    }

    // Words 23-26: Firmware revision (8 ASCII chars).
    let firmware = b"1.0     ";
    for (i, chunk) in firmware.chunks(2).enumerate() {
        set_word(
            buffer,
            23 + i,
            u16::from(chunk[0]) << 8 | u16::from(chunk[1]),
        );
    }

    // Words 27-46: Model number (40 ASCII chars).
    let model = b"NEC CD-ROM DRIVE:98                     ";
    for (i, chunk) in model.chunks(2).enumerate() {
        set_word(
            buffer,
            27 + i,
            u16::from(chunk[0]) << 8 | u16::from(chunk[1]),
        );
    }

    // Word 49: Capabilities (LBA supported).
    set_word(buffer, 49, 0x0200);

    // Word 51: PIO cycle timing mode.
    set_word(buffer, 51, 0x0278);

    // Word 53: validity flags (words 54-58 and 64-70 valid).
    set_word(buffer, 53, 0x0003);

    // Word 64: PIO modes supported (mode 3 and 4).
    set_word(buffer, 64, 0x0003);

    // Word 80: Major version (ATA/ATAPI-5).
    set_word(buffer, 80, 0x003E);

    // Word 82: Command set supported (PACKET, Device Reset).
    set_word(buffer, 82, 0x0214);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cd_audio::{CdAudioPlayer, CdAudioState},
        cdrom::CdImage,
    };

    fn make_test_cdimage() -> CdImage {
        let cue = r#"FILE "test.bin" BINARY
  TRACK 01 MODE1/2048
    INDEX 01 00:00:00
"#;
        let mut bin_data = vec![0u8; 2048 * 100];
        for i in 0..100u32 {
            let offset = i as usize * 2048;
            bin_data[offset] = (i >> 8) as u8;
            bin_data[offset + 1] = i as u8;
        }
        CdImage::from_cue(cue, bin_data).unwrap()
    }

    fn make_multi_track_cdimage() -> CdImage {
        let cue = r#"FILE "test.bin" BINARY
  TRACK 01 MODE1/2048
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    INDEX 01 00:02:00
"#;
        let mut bin_data = vec![0x11u8; 2048 * 150]; // 150 data sectors.
        bin_data.extend_from_slice(&vec![0xAAu8; 2352 * 50]); // 50 audio sectors.
        CdImage::from_cue(cue, bin_data).unwrap()
    }

    #[test]
    fn lba_to_msf_conversion() {
        assert_eq!(lba_to_msf(0), (0, 0, 0));
        assert_eq!(lba_to_msf(75), (0, 1, 0));
        assert_eq!(lba_to_msf(4500), (1, 0, 0));
        assert_eq!(lba_to_msf(4653), (1, 2, 3));
    }

    #[test]
    fn test_unit_ready_no_media() {
        let mut state = AtapiState::new();
        let (has_data, is_error) = state.cmd_test_unit_ready();
        assert!(!has_data);
        assert!(is_error);
        assert_eq!(state.sense_key, SENSE_NOT_READY);
        assert_eq!(state.asc, ASC_MEDIUM_NOT_PRESENT);
    }

    #[test]
    fn test_unit_ready_media_changed() {
        let mut state = AtapiState::new();
        state.media_inserted();
        let (has_data, is_error) = state.cmd_test_unit_ready();
        assert!(!has_data);
        assert!(is_error);
        assert_eq!(state.sense_key, SENSE_UNIT_ATTENTION);
        assert_eq!(state.asc, ASC_NOT_READY_TO_READY_TRANSITION);
    }

    #[test]
    fn test_unit_ready_after_acknowledgment() {
        let mut state = AtapiState::new();
        state.media_inserted();

        // First TEST UNIT READY: returns UNIT_ATTENTION.
        let (_, is_error) = state.cmd_test_unit_ready();
        assert!(is_error);

        // Second TEST UNIT READY: media_changed was cleared, should succeed.
        let (has_data, is_error) = state.cmd_test_unit_ready();
        assert!(!has_data);
        assert!(!is_error);
    }

    #[test]
    fn request_sense_returns_and_clears() {
        let mut state = AtapiState::new();
        state.set_sense(SENSE_UNIT_ATTENTION, ASC_NOT_READY_TO_READY_TRANSITION, 0);
        state.packet[4] = 18;

        let (has_data, is_error) = state.cmd_request_sense();
        assert!(has_data);
        assert!(!is_error);

        assert_eq!(state.data_buffer[0], 0x70);
        assert_eq!(state.data_buffer[2], SENSE_UNIT_ATTENTION);
        assert_eq!(state.data_buffer[12], ASC_NOT_READY_TO_READY_TRANSITION);

        // Sense should be cleared.
        assert_eq!(state.sense_key, SENSE_NO_SENSE);
    }

    #[test]
    fn inquiry_returns_cdrom_device() {
        let mut state = AtapiState::new();
        state.packet[4] = 36;

        let (has_data, _) = state.cmd_inquiry();
        assert!(has_data);
        assert_eq!(state.data_buffer[0], 0x05); // CD-ROM
        assert_eq!(state.data_buffer[1], 0x80); // Removable
    }

    #[test]
    fn read_capacity_returns_correct_values() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_test_cdimage();

        let (has_data, is_error) = state.cmd_read_capacity(Some(&cdrom));
        assert!(has_data);
        assert!(!is_error);

        // Last LBA = 99 (100 sectors - 1).
        let last_lba = u32::from(state.data_buffer[0]) << 24
            | u32::from(state.data_buffer[1]) << 16
            | u32::from(state.data_buffer[2]) << 8
            | u32::from(state.data_buffer[3]);
        assert_eq!(last_lba, 99);

        // Block size = 2048.
        let block_size = u32::from(state.data_buffer[4]) << 24
            | u32::from(state.data_buffer[5]) << 16
            | u32::from(state.data_buffer[6]) << 8
            | u32::from(state.data_buffer[7]);
        assert_eq!(block_size, 2048);
    }

    #[test]
    fn read_10_reads_correct_sector() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_test_cdimage();

        // READ(10): LBA 42, count 1.
        state.packet = [0x28, 0, 0, 0, 0, 42, 0, 0, 1, 0, 0, 0];

        let (has_data, is_error) = state.cmd_read_10(Some(&cdrom));
        assert!(has_data);
        assert!(!is_error);
        assert_eq!(state.data_size, 2048);

        // First two bytes should be sector 42 marker.
        assert_eq!(state.data_buffer[0], 0);
        assert_eq!(state.data_buffer[1], 42);
    }

    #[test]
    fn read_10_multi_sector() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_test_cdimage();

        // READ(10): LBA 0, count 3.
        state.packet = [0x28, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0];

        let (has_data, is_error) = state.cmd_read_10(Some(&cdrom));
        assert!(has_data);
        assert!(!is_error);
        assert_eq!(state.data_size, 2048 * 3);

        // Sector 0.
        assert_eq!(state.data_buffer[0], 0);
        assert_eq!(state.data_buffer[1], 0);
        // Sector 1.
        assert_eq!(state.data_buffer[2048], 0);
        assert_eq!(state.data_buffer[2049], 1);
        // Sector 2.
        assert_eq!(state.data_buffer[4096], 0);
        assert_eq!(state.data_buffer[4097], 2);
    }

    #[test]
    fn read_10_out_of_range() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_test_cdimage();

        // LBA 200, count 1 (out of range: only 100 sectors).
        state.packet = [0x28, 0, 0, 0, 0, 200, 0, 0, 1, 0, 0, 0];

        let (_, is_error) = state.cmd_read_10(Some(&cdrom));
        assert!(is_error);
        assert_eq!(state.sense_key, SENSE_ILLEGAL_REQUEST);
        assert_eq!(state.asc, ASC_LOGICAL_BLOCK_OUT_OF_RANGE);
    }

    #[test]
    fn read_toc_format_0() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_multi_track_cdimage();

        // READ TOC: format 0, starting track 0, allocation 1024.
        state.packet = [0x43, 0, 0, 0, 0, 0, 0, 0x04, 0x00, 0, 0, 0];

        let (has_data, is_error) = state.cmd_read_toc(Some(&cdrom));
        assert!(has_data);
        assert!(!is_error);

        // Header: first track 1, last track 2.
        assert_eq!(state.data_buffer[2], 1);
        assert_eq!(state.data_buffer[3], 2);

        // Track 1 descriptor at offset 4.
        assert_eq!(state.data_buffer[5], 0x14); // Data track.
        assert_eq!(state.data_buffer[6], 1); // Track number.
        // Track 1 LBA = 0.
        let t1_lba = u32::from(state.data_buffer[8]) << 24
            | u32::from(state.data_buffer[9]) << 16
            | u32::from(state.data_buffer[10]) << 8
            | u32::from(state.data_buffer[11]);
        assert_eq!(t1_lba, 0);

        // Track 2 descriptor at offset 12.
        assert_eq!(state.data_buffer[13], 0x10); // Audio track.
        assert_eq!(state.data_buffer[14], 2); // Track number.
        let t2_lba = u32::from(state.data_buffer[16]) << 24
            | u32::from(state.data_buffer[17]) << 16
            | u32::from(state.data_buffer[18]) << 8
            | u32::from(state.data_buffer[19]);
        assert_eq!(t2_lba, 150);

        // Lead-out at offset 20.
        assert_eq!(state.data_buffer[22], 0xAA);
    }

    #[test]
    fn mode_sense_page_0f_nec_compat() {
        let mut state = AtapiState::new();
        assert!(!state.bcd_msf_mode);

        // MODE SENSE(10): page 0x0F with a short 24-byte allocation. This is
        // the form used by the real-mode DOS CD-ROM stack.
        state.packet = [0x5A, 0, 0x0F, 0, 0, 0, 0, 0x00, 0x18, 0, 0, 0];

        let (has_data, is_error) = state.cmd_mode_sense_10(None);
        assert!(has_data);
        assert!(!is_error);

        // Mode parameter header is 8 bytes.
        // Mode page 0x0F should be at offset 8.
        assert_eq!(state.data_buffer[8], 0x0F); // Page code.
        assert_eq!(state.data_buffer[9], 0x10); // Page length = 16 bytes.
        // A 24-byte request should return 24 bytes and advertise 22 bytes
        // after the length field, not the larger untruncated page set.
        assert_eq!(state.data_size, 24);
        assert_eq!(state.data_buffer[0], 0x00);
        assert_eq!(state.data_buffer[1], 0x16);

        // Byte 4: bit 0 (audio play) | bit 7 (lock state).
        // Byte 5: audio manipulation bits.
        assert_eq!(state.data_buffer[12], 0x81);
        assert_eq!(state.data_buffer[13], 0x18);

        // Requesting page 0x0F activates BCD MSF mode (NEC neccdd.sys quirk).
        assert!(state.bcd_msf_mode);
    }

    #[test]
    fn start_stop_unit_eject() {
        let mut state = AtapiState::new();
        state.media_loaded = true;

        // START/STOP: LoEj=1, Start=0 -> eject.
        state.packet = [0x1B, 0, 0, 0, 0x02, 0, 0, 0, 0, 0, 0, 0];

        let (_, is_error) = state.cmd_start_stop_unit();
        assert!(!is_error);
        assert!(!state.media_loaded);
        assert!(state.media_changed);
    }

    #[test]
    fn start_stop_unit_eject_prevented() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        state.prevent_removal = true;

        // START/STOP: LoEj=1, Start=0 -> eject (should fail).
        state.packet = [0x1B, 0, 0, 0, 0x02, 0, 0, 0, 0, 0, 0, 0];

        let (_, is_error) = state.cmd_start_stop_unit();
        assert!(is_error);
        assert!(state.media_loaded); // Still loaded.
    }

    #[test]
    fn prevent_allow_medium_removal() {
        let mut state = AtapiState::new();

        // Prevent removal.
        state.packet = [0x1E, 0, 0, 0, 0x01, 0, 0, 0, 0, 0, 0, 0];
        state.cmd_prevent_allow_medium_removal();
        assert!(state.prevent_removal);

        // Allow removal.
        state.packet = [0x1E, 0, 0, 0, 0x00, 0, 0, 0, 0, 0, 0, 0];
        state.cmd_prevent_allow_medium_removal();
        assert!(!state.prevent_removal);
    }

    #[test]
    fn media_change_state_machine() {
        let mut state = AtapiState::new();

        // No media: TEST UNIT READY fails with NOT_READY / MEDIUM_NOT_PRESENT.
        let (_, is_error) = state.cmd_test_unit_ready();
        assert!(is_error);
        assert_eq!(state.sense_key, SENSE_NOT_READY);
        assert_eq!(state.asc, ASC_MEDIUM_NOT_PRESENT);

        // REQUEST SENSE returns the NOT_READY sense data.
        state.packet[4] = 18;
        let (has_data, _) = state.cmd_request_sense();
        assert!(has_data);
        assert_eq!(state.data_buffer[2], SENSE_NOT_READY);
        assert_eq!(state.data_buffer[12], ASC_MEDIUM_NOT_PRESENT);
        // After REQUEST SENSE, sense is cleared.
        assert_eq!(state.sense_key, SENSE_NO_SENSE);

        // Insert media.
        state.media_inserted();

        // TEST UNIT READY: UNIT_ATTENTION / NOT_READY_TO_READY_TRANSITION.
        let (_, is_error) = state.cmd_test_unit_ready();
        assert!(is_error);
        assert_eq!(state.sense_key, SENSE_UNIT_ATTENTION);
        assert_eq!(state.asc, ASC_NOT_READY_TO_READY_TRANSITION);

        // REQUEST SENSE returns the UNIT_ATTENTION sense data and clears it.
        state.packet[4] = 18;
        let (has_data, _) = state.cmd_request_sense();
        assert!(has_data);
        assert_eq!(state.data_buffer[2], SENSE_UNIT_ATTENTION);
        assert_eq!(state.data_buffer[12], ASC_NOT_READY_TO_READY_TRANSITION);

        // TEST UNIT READY: now succeeds (media_changed was cleared by REQUEST SENSE).
        let (_, is_error) = state.cmd_test_unit_ready();
        assert!(!is_error);

        // Eject media.
        state.media_ejected();

        // TEST UNIT READY: NOT_READY / MEDIUM_NOT_PRESENT.
        let (_, is_error) = state.cmd_test_unit_ready();
        assert!(is_error);
        assert_eq!(state.sense_key, SENSE_NOT_READY);
        assert_eq!(state.asc, ASC_MEDIUM_NOT_PRESENT);
    }

    #[test]
    fn unsupported_command() {
        let mut state = AtapiState::new();
        state.packet[0] = 0xFF;

        let (_, is_error) = state.cmd_unsupported();
        assert!(is_error);
        assert_eq!(state.sense_key, SENSE_ILLEGAL_REQUEST);
        assert_eq!(state.asc, ASC_INVALID_COMMAND_OPERATION_CODE);
    }

    #[test]
    fn identify_packet_device_data() {
        let mut buffer = [0u8; 512];
        build_identify_packet_device(&mut buffer);

        let word0 = u16::from(buffer[0]) | (u16::from(buffer[1]) << 8);
        assert_eq!(word0, 0x8580);

        // Word 49: LBA.
        let word49 = u16::from(buffer[98]) | (u16::from(buffer[99]) << 8);
        assert_eq!(word49, 0x0200);
    }

    #[test]
    fn packet_receive_flow() {
        let mut state = AtapiState::new();
        state.start_packet_command(0xFE, 0xFF);

        // Write 6 words (12 bytes). Bytes are stored in little-endian:
        // word low byte -> packet[pos], word high byte -> packet[pos+1].
        // Packet for READ(10) LBA=0 count=1:
        // Byte 0: 0x28, Byte 1: 0x00, ... Byte 7: 0x00, Byte 8: 0x01
        assert!(!state.receive_packet_word(0x0028)); // byte[0]=0x28, byte[1]=0x00
        assert!(!state.receive_packet_word(0x0000)); // byte[2]=0x00, byte[3]=0x00
        assert!(!state.receive_packet_word(0x0000)); // byte[4]=0x00, byte[5]=0x00
        assert!(!state.receive_packet_word(0x0000)); // byte[6]=0x00, byte[7]=0x00
        assert!(!state.receive_packet_word(0x0001)); // byte[8]=0x01, byte[9]=0x00
        assert!(state.receive_packet_word(0x0000)); // byte[10]=0x00, byte[11]=0x00

        assert_eq!(state.packet[0], 0x28); // READ(10)
        assert_eq!(state.packet[7], 0x00);
        assert_eq!(state.packet[8], 0x01);
    }

    #[test]
    fn seek_in_range() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_test_cdimage();

        state.packet = [0x2B, 0, 0, 0, 0, 50, 0, 0, 0, 0, 0, 0];
        let (_, is_error) = state.cmd_seek(Some(&cdrom));
        assert!(!is_error);
    }

    #[test]
    fn seek_out_of_range() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_test_cdimage();

        state.packet = [0x2B, 0, 0, 0, 0, 200, 0, 0, 0, 0, 0, 0];
        let (_, is_error) = state.cmd_seek(Some(&cdrom));
        assert!(is_error);
        assert_eq!(state.sense_key, SENSE_ILLEGAL_REQUEST);
    }

    #[test]
    fn mechanism_status_with_media() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        state.packet = [0xBD, 0, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0];

        let (has_data, _) = state.cmd_mechanism_status();
        assert!(has_data);
        assert_eq!(state.data_buffer[1] & 0x20, 0x20); // Disc present.
    }

    #[test]
    fn mechanism_status_without_media() {
        let mut state = AtapiState::new();
        state.packet = [0xBD, 0, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0];

        let (has_data, _) = state.cmd_mechanism_status();
        assert!(has_data);
        assert_eq!(state.data_buffer[1] & 0x20, 0x00); // No disc.
    }

    #[test]
    fn read_toc_session_info() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_test_cdimage();

        // READ TOC format 1 (session info).
        state.packet = [0x43, 0, 0, 0, 0, 0, 0, 0x00, 0x0C, 0x40, 0, 0];

        let (has_data, _) = state.cmd_read_toc(Some(&cdrom));
        assert!(has_data);
        assert_eq!(state.data_buffer[2], 1); // First session.
        assert_eq!(state.data_buffer[3], 1); // Last session.
    }

    #[test]
    fn mode_sense_all_pages() {
        let mut state = AtapiState::new();
        assert!(!state.bcd_msf_mode);

        // MODE SENSE(10): page 0x3F (all pages), large allocation.
        state.packet = [0x5A, 0, 0x3F, 0, 0, 0, 0, 0x04, 0x00, 0, 0, 0];

        let (has_data, is_error) = state.cmd_mode_sense_10(None);
        assert!(has_data);
        assert!(!is_error);

        // Verify all four pages are present by scanning for page codes after
        // the 8-byte mode parameter header.
        let mut found_pages = Vec::new();
        let mut offset = 8;
        while offset + 1 < state.data_size {
            let page_code = state.data_buffer[offset] & 0x3F;
            let page_length = state.data_buffer[offset + 1] as usize;
            found_pages.push(page_code);
            offset += if page_code == 0x0F {
                16
            } else {
                2 + page_length
            };
        }
        assert!(
            found_pages.contains(&0x01),
            "missing page 0x01 (error recovery)"
        );
        assert!(
            found_pages.contains(&0x0E),
            "missing page 0x0E (audio control)"
        );
        assert!(
            found_pages.contains(&0x0F),
            "missing page 0x0F (NEC vendor)"
        );
        assert!(
            found_pages.contains(&0x2A),
            "missing page 0x2A (capabilities)"
        );

        // All-pages includes 0x0F, so BCD mode should be activated.
        assert!(state.bcd_msf_mode);
    }

    #[test]
    fn mode_sense_invalid_page() {
        let mut state = AtapiState::new();
        state.packet = [0x5A, 0, 0x05, 0, 0, 0, 0, 0x01, 0x00, 0, 0, 0];

        let (_, is_error) = state.cmd_mode_sense_10(None);
        assert!(is_error);
        assert_eq!(state.sense_key, SENSE_ILLEGAL_REQUEST);
    }

    #[test]
    fn read_sub_channel_minimal() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_test_cdimage();

        state.packet = [0x42, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0];

        let cd_audio = CdAudioPlayer::new(44100);
        let (has_data, is_error) = state.cmd_read_sub_channel(Some(&cdrom), &cd_audio);
        assert!(has_data);
        assert!(!is_error);
        assert_eq!(state.data_buffer[1], 0x15); // No audio status.
    }

    fn interleave_subq(q: &[u8; 12]) -> [u8; 96] {
        let mut out = [0u8; 96];
        for (byte_index, byte) in q.iter().enumerate() {
            for bit in 0..8 {
                if byte & (0x80 >> bit) != 0 {
                    out[byte_index * 8 + bit] |= 0x40;
                }
            }
        }
        out
    }

    fn make_raw_data_sector_atapi(byte: u8) -> Vec<u8> {
        let mut sector = vec![0u8; 2352];
        sector[0] = 0x00;
        for value in &mut sector[1..11] {
            *value = 0xFF;
        }
        sector[11] = 0x00;
        sector[15] = 0x01;
        for value in &mut sector[16..16 + 2048] {
            *value = byte;
        }
        sector
    }

    #[test]
    fn read_sub_channel_uses_disc_subq_when_present() {
        let ccd = "[TRACK 1]\nMODE=1\nINDEX 1=0\n";
        let img: Vec<u8> = (0..2)
            .flat_map(|_| make_raw_data_sector_atapi(0x00))
            .collect();
        // Sub-Q for LBA 0: Control=4 (data), ADR=1, track 1, index 1,
        // relative MSF 00:00:00 (BCD), absolute MSF 00:02:00 (BCD).
        let sub_q = [
            0x41u8, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0xAB, 0xCD,
        ];
        let mut sub_data = Vec::new();
        sub_data.extend_from_slice(&interleave_subq(&sub_q));
        sub_data.extend_from_slice(&[0u8; 96]);

        let cdrom = CdImage::from_ccd(ccd, img, Some(sub_data)).unwrap();

        let mut state = AtapiState::new();
        state.media_loaded = true;
        // packet[1] bit 1 = MSF, packet[2] bit 6 = Sub-Q, packet[3] = format 0x01.
        state.packet = [0x42, 0x02, 0x40, 0x01, 0, 0, 0, 0, 16, 0, 0, 0];

        let cd_audio = CdAudioPlayer::new(44100);
        let (has_data, is_error) = state.cmd_read_sub_channel(Some(&cdrom), &cd_audio);
        assert!(has_data);
        assert!(!is_error);

        // ADR/Control: nibble-swap of Sub-Q byte 0.
        assert_eq!(state.data_buffer[5], 0x14);
        assert_eq!(state.data_buffer[6], 1); // Track.
        assert_eq!(state.data_buffer[7], 1); // Index.
        // Absolute MSF.
        assert_eq!(&state.data_buffer[8..12], &[0u8, 0, 2, 0]);
        // Relative MSF.
        assert_eq!(&state.data_buffer[12..16], &[0u8, 0, 0, 0]);
    }

    #[test]
    fn read_toc_multi_track() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_multi_track_cdimage();

        state.packet = [0x43, 0, 0, 0, 0, 0, 0, 0x04, 0x00, 0, 0, 0];

        let (has_data, is_error) = state.cmd_read_toc(Some(&cdrom));
        assert!(has_data);
        assert!(!is_error);

        // Should have 2 track entries + lead-out.
        assert_eq!(state.data_buffer[2], 1); // First track.
        assert_eq!(state.data_buffer[3], 2); // Last track.

        // Track 1: data at LBA 0.
        assert_eq!(state.data_buffer[5], 0x14); // Data.
        assert_eq!(state.data_buffer[6], 1);

        // Track 2: audio at LBA 150.
        assert_eq!(state.data_buffer[13], 0x10); // Audio.
        assert_eq!(state.data_buffer[14], 2);
        let t2_lba = u32::from(state.data_buffer[16]) << 24
            | u32::from(state.data_buffer[17]) << 16
            | u32::from(state.data_buffer[18]) << 8
            | u32::from(state.data_buffer[19]);
        assert_eq!(t2_lba, 150);
    }

    #[test]
    fn read_toc_format_0_msf() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_multi_track_cdimage();

        // TIME bit set (byte 1 bit 1), format 0, allocation_length=1024.
        state.packet = [0x43, 0x02, 0, 0, 0, 0, 0, 0x04, 0x00, 0, 0, 0];

        let (has_data, is_error) = state.cmd_read_toc(Some(&cdrom));
        assert!(has_data);
        assert!(!is_error);

        // Track 1 at LBA 0 => MSF = (0+150) = 0:02:00 => bytes: [0, 0, 2, 0].
        assert_eq!(state.data_buffer[4 + 4], 0); // Reserved.
        assert_eq!(state.data_buffer[4 + 5], 0); // M=0.
        assert_eq!(state.data_buffer[4 + 6], 2); // S=2.
        assert_eq!(state.data_buffer[4 + 7], 0); // F=0.

        // Track 2 at LBA 150 => MSF = (150+150)=300 = 0:04:00 => bytes: [0, 0, 4, 0].
        assert_eq!(state.data_buffer[12 + 4], 0);
        assert_eq!(state.data_buffer[12 + 5], 0);
        assert_eq!(state.data_buffer[12 + 6], 4);
        assert_eq!(state.data_buffer[12 + 7], 0);
    }

    #[test]
    fn read_toc_format_0_lba() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_multi_track_cdimage();

        // TIME bit NOT set, format 0.
        state.packet = [0x43, 0x00, 0, 0, 0, 0, 0, 0x04, 0x00, 0, 0, 0];

        let (has_data, is_error) = state.cmd_read_toc(Some(&cdrom));
        assert!(has_data);
        assert!(!is_error);

        // Track 1 at LBA 0 => big-endian [0, 0, 0, 0].
        let t1_lba = u32::from(state.data_buffer[8]) << 24
            | u32::from(state.data_buffer[9]) << 16
            | u32::from(state.data_buffer[10]) << 8
            | u32::from(state.data_buffer[11]);
        assert_eq!(t1_lba, 0);

        // Track 2 at LBA 150 => big-endian.
        let t2_lba = u32::from(state.data_buffer[16]) << 24
            | u32::from(state.data_buffer[17]) << 16
            | u32::from(state.data_buffer[18]) << 8
            | u32::from(state.data_buffer[19]);
        assert_eq!(t2_lba, 150);
    }

    #[test]
    fn read_toc_format_1_msf() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_test_cdimage();

        // TIME bit set, format 1 (via byte 9 bits 6-7 = 01).
        state.packet = [0x43, 0x02, 0, 0, 0, 0, 0, 0x00, 0x0C, 0x40, 0, 0];

        let (has_data, is_error) = state.cmd_read_toc(Some(&cdrom));
        assert!(has_data);
        assert!(!is_error);

        // Session descriptor: track 1 at LBA 0 => MSF = 0:02:00.
        assert_eq!(state.data_buffer[8], 0);
        assert_eq!(state.data_buffer[9], 0);
        assert_eq!(state.data_buffer[10], 2);
        assert_eq!(state.data_buffer[11], 0);
    }

    #[test]
    fn identify_packet_device_word_82() {
        let mut buffer = vec![0u8; 512];
        build_identify_packet_device(&mut buffer);

        // Word 82 should be 0x0214 (PACKET + Device Reset).
        let word_82 = u16::from(buffer[164]) | (u16::from(buffer[165]) << 8);
        assert_eq!(word_82, 0x0214);
    }

    #[test]
    fn store_address_lba() {
        let mut buf = [0u8; 4];
        store_address(&mut buf, 0x12345678, false, false);
        assert_eq!(buf, [0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn store_address_msf() {
        let mut buf = [0u8; 4];
        // LBA 0 + 150 = 150 frames = 0:02:00.
        store_address(&mut buf, 0, true, false);
        assert_eq!(buf, [0, 0, 2, 0]);

        // LBA 150 + 150 = 300 frames = 0:04:00.
        store_address(&mut buf, 150, true, false);
        assert_eq!(buf, [0, 0, 4, 0]);

        // LBA 4350 + 150 = 4500 = 1:00:00.
        store_address(&mut buf, 4350, true, false);
        assert_eq!(buf, [0, 1, 0, 0]);
    }

    #[test]
    fn store_address_msf_bcd() {
        let mut buf = [0u8; 4];
        // LBA 0 + 150 = 0:02:00 => BCD: 0x00, 0x02, 0x00.
        store_address(&mut buf, 0, true, true);
        assert_eq!(buf, [0, 0x00, 0x02, 0x00]);

        // LBA 4350 + 150 = 1:00:00 => BCD: 0x01, 0x00, 0x00.
        store_address(&mut buf, 4350, true, true);
        assert_eq!(buf, [0, 0x01, 0x00, 0x00]);

        // LBA 329925 + 150 = 330075 = 73:21:00 => BCD: 0x73, 0x21, 0x00.
        store_address(&mut buf, 329925, true, true);
        assert_eq!(buf, [0, 0x73, 0x21, 0x00]);

        // LBA 337424 + 150 = 337574 = 75:00:74 => BCD: 0x75, 0x00, 0x74.
        store_address(&mut buf, 337424, true, true);
        assert_eq!(buf, [0, 0x75, 0x00, 0x74]);
    }

    #[test]
    fn store_address_msf_bcd_overflow_clamped() {
        let mut buf = [0u8; 4];
        // 100 min = 100*60*75 = 450000 frames. LBA = 450000 - 150 = 449850.
        // store_address adds 150: lba_to_msf(449850 + 150) = lba_to_msf(450000)
        //   frame = 450000 % 75 = 0, seconds = (450000/75) % 60 = 0, minutes = (450000/75/60) = 100
        // Minutes > 99 in BCD mode => clamp to [0, 0xFF, 0x59, 0x74].
        store_address(&mut buf, 449850, true, true);
        assert_eq!(buf, [0, 0xFF, 0x59, 0x74]);
    }

    #[test]
    fn store_address_msf_bcd_boundary_99_minutes() {
        let mut buf = [0u8; 4];
        // 99 min exactly: 99*60*75 = 445500 frames. LBA = 445500 - 150 = 445350.
        // store_address adds 150: lba_to_msf(445350 + 150) = lba_to_msf(445500)
        //   = 99:00:00 => BCD [0, 0x99, 0x00, 0x00]. Should NOT be clamped.
        store_address(&mut buf, 445350, true, true);
        assert_eq!(buf, [0, 0x99, 0x00, 0x00]);
    }

    #[test]
    fn identify_packet_device_model_string() {
        let mut buffer = [0u8; 512];
        build_identify_packet_device(&mut buffer);
        // Words 27-46 = model string (40 bytes, ATA byte-swapped).
        let mut model = [0u8; 40];
        for i in 0..20 {
            let word_offset = (27 + i) * 2;
            model[i * 2] = buffer[word_offset + 1]; // High byte first (ATA swap).
            model[i * 2 + 1] = buffer[word_offset]; // Low byte second.
        }
        let model_str = core::str::from_utf8(&model).unwrap();
        assert!(model_str.starts_with("NEC CD-ROM DRIVE:98"));
    }

    #[test]
    fn inquiry_product_id_nec98() {
        let mut state = AtapiState::new();
        state.packet[4] = 36;
        let (has_data, _) = state.cmd_inquiry();
        assert!(has_data);
        // Product ID at bytes 16-31.
        let product = &state.data_buffer[16..32];
        assert_eq!(product, b"CD-ROM DRIVE:98 ");
    }

    #[test]
    fn hex_to_bcd_roundtrip() {
        for val in 0..100u8 {
            assert_eq!(bcd_to_hex(hex_to_bcd(val)), val);
        }
    }

    #[test]
    fn hex_to_bcd_specific_values() {
        assert_eq!(hex_to_bcd(0), 0x00);
        assert_eq!(hex_to_bcd(9), 0x09);
        assert_eq!(hex_to_bcd(10), 0x10);
        assert_eq!(hex_to_bcd(59), 0x59);
        assert_eq!(hex_to_bcd(74), 0x74);
        assert_eq!(hex_to_bcd(99), 0x99);
    }

    #[test]
    fn bcd_to_hex_specific_values() {
        assert_eq!(bcd_to_hex(0x00), 0);
        assert_eq!(bcd_to_hex(0x09), 9);
        assert_eq!(bcd_to_hex(0x10), 10);
        assert_eq!(bcd_to_hex(0x59), 59);
        assert_eq!(bcd_to_hex(0x74), 74);
        assert_eq!(bcd_to_hex(0x99), 99);
    }

    #[test]
    fn media_change_not_cleared_by_read_10() {
        let mut state = AtapiState::new();
        state.media_inserted();
        let cdrom = make_test_cdimage();

        // READ(10) with media_changed: should return UNIT_ATTENTION.
        state.packet = [0x28, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0];
        let (_, is_error) = state.cmd_read_10(Some(&cdrom));
        assert!(is_error);
        assert_eq!(state.sense_key, SENSE_UNIT_ATTENTION);

        // media_changed should NOT have been cleared by READ(10).
        assert!(state.media_changed);

        // TEST UNIT READY should also see the UNIT_ATTENTION and clear it.
        let (_, is_error) = state.cmd_test_unit_ready();
        assert!(is_error);
        assert_eq!(state.sense_key, SENSE_UNIT_ATTENTION);
        assert!(!state.media_changed);
    }

    #[test]
    fn inquiry_clears_media_changed() {
        let mut state = AtapiState::new();
        state.media_inserted();
        assert!(state.media_changed);

        state.packet = [0x12, 0, 0, 0, 36, 0, 0, 0, 0, 0, 0, 0];
        let (has_data, is_error) = state.cmd_inquiry();
        assert!(has_data);
        assert!(!is_error);
        assert!(!state.media_changed);
    }

    #[test]
    fn mode_sense_page_0d_present() {
        let mut state = AtapiState::new();
        state.packet = [0x5A, 0, 0x0D, 0, 0, 0, 0, 0x00, 0xFF, 0, 0, 0];

        let (has_data, is_error) = state.cmd_mode_sense_10(None);
        assert!(has_data);
        assert!(!is_error);

        // Page 0x0D should start at offset 8.
        assert_eq!(state.data_buffer[8], 0x0D);
        assert_eq!(state.data_buffer[9], 0x06);
        assert_eq!(state.data_buffer[13], 0x3C);
        assert_eq!(state.data_buffer[15], 0x4B);
    }

    #[test]
    fn mode_sense_all_pages_includes_0d() {
        let mut state = AtapiState::new();
        state.packet = [0x5A, 0, 0x3F, 0, 0, 0, 0, 0x01, 0x00, 0, 0, 0];

        let (has_data, is_error) = state.cmd_mode_sense_10(None);
        assert!(has_data);
        assert!(!is_error);

        // Find page 0x0D in the all-pages response (after page 0x01).
        // Page 0x01: 8 bytes. So 0x0D starts at offset 8+8=16.
        assert_eq!(state.data_buffer[16], 0x0D);
    }

    #[test]
    fn mode_sense_medium_type_no_disc() {
        let mut state = AtapiState::new();
        state.packet = [0x5A, 0, 0x01, 0, 0, 0, 0, 0x00, 0xFF, 0, 0, 0];

        let (has_data, _) = state.cmd_mode_sense_10(None);
        assert!(has_data);
        assert_eq!(state.data_buffer[2], 0x70);
    }

    #[test]
    fn mode_sense_medium_type_data_disc() {
        let mut state = AtapiState::new();
        let cdrom = make_test_cdimage();
        state.packet = [0x5A, 0, 0x01, 0, 0, 0, 0, 0x00, 0xFF, 0, 0, 0];

        let (has_data, _) = state.cmd_mode_sense_10(Some(&cdrom));
        assert!(has_data);
        assert_eq!(state.data_buffer[2], 0x01);
    }

    #[test]
    fn mode_sense_medium_type_mixed_disc() {
        let mut state = AtapiState::new();
        let cdrom = make_multi_track_cdimage();
        state.packet = [0x5A, 0, 0x01, 0, 0, 0, 0, 0x00, 0xFF, 0, 0, 0];

        let (has_data, _) = state.cmd_mode_sense_10(Some(&cdrom));
        assert!(has_data);
        assert_eq!(state.data_buffer[2], 0x03);
    }

    #[test]
    fn medium_type_helper() {
        assert_eq!(medium_type(None), 0x70);

        let data_disc = make_test_cdimage();
        assert_eq!(medium_type(Some(&data_disc)), 0x01);

        let mixed_disc = make_multi_track_cdimage();
        assert_eq!(medium_type(Some(&mixed_disc)), 0x03);
    }

    #[test]
    fn msf_to_lba_conversion() {
        // MSF 0:02:00 (150 frames) = LBA 0.
        assert_eq!(msf_to_lba(0, 2, 0), 0);
        // MSF 0:02:01 = LBA 1.
        assert_eq!(msf_to_lba(0, 2, 1), 1);
        // MSF 0:00:00 saturates to LBA 0 (pre-gap).
        assert_eq!(msf_to_lba(0, 0, 0), 0);
    }

    #[test]
    fn mode_select_10_without_save_pages() {
        let mut state = AtapiState::new();
        // MODE SELECT(10): no save pages bit.
        state.packet = [0x55, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let (has_data, is_error) = state.cmd_mode_select_10();
        assert!(!has_data);
        assert!(!is_error);
    }

    #[test]
    fn mode_select_10_with_save_pages() {
        let mut state = AtapiState::new();
        // MODE SELECT(10): save pages bit set.
        state.packet = [0x55, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let (has_data, is_error) = state.cmd_mode_select_10();
        assert!(!has_data);
        assert!(is_error);
        assert_eq!(state.sense_key, SENSE_ILLEGAL_REQUEST);
        assert_eq!(state.asc, ASC_SAVING_PARAMETERS_NOT_SUPPORTED);
    }

    #[test]
    fn get_configuration_returns_cdrom_profile() {
        let mut state = AtapiState::new();
        // GET CONFIGURATION: allocation length = 256.
        state.packet = [0x46, 0, 0, 0, 0, 0, 0, 0x01, 0x00, 0, 0, 0];
        let (has_data, is_error) = state.cmd_get_configuration();
        assert!(has_data);
        assert!(!is_error);
        assert_eq!(state.data_size, 8);
        // Current profile: CD-ROM (0x0008).
        assert_eq!(state.data_buffer[6], 0x00);
        assert_eq!(state.data_buffer[7], 0x08);
    }

    #[test]
    fn play_audio_succeeds() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_multi_track_cdimage();
        let mut cd_audio = CdAudioPlayer::new(44100);
        // PLAY AUDIO(10): start LBA 150, length 50 sectors.
        state.packet = [0x45, 0, 0, 0, 0, 150, 0, 0, 50, 0, 0, 0];
        let (has_data, is_error) = state.cmd_play_audio(Some(&cdrom), &mut cd_audio);
        assert!(!has_data);
        assert!(!is_error);
        assert_eq!(cd_audio.state(), CdAudioState::Playing);
    }

    #[test]
    fn play_audio_msf_succeeds() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_multi_track_cdimage();
        let mut cd_audio = CdAudioPlayer::new(44100);
        // PLAY AUDIO MSF: start 0:04:00 (LBA 150), end 0:04:50 (LBA 200).
        state.packet = [0x47, 0, 0, 0, 4, 0, 0, 4, 50, 0, 0, 0];
        let (has_data, is_error) = state.cmd_play_audio_msf(Some(&cdrom), &mut cd_audio);
        assert!(!has_data);
        assert!(!is_error);
        assert_eq!(cd_audio.state(), CdAudioState::Playing);
    }

    #[test]
    fn pause_resume_succeeds() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_multi_track_cdimage();
        let mut cd_audio = CdAudioPlayer::new(44100);
        // Start playback first.
        state.packet = [0x45, 0, 0, 0, 0, 150, 0, 0, 50, 0, 0, 0];
        state.cmd_play_audio(Some(&cdrom), &mut cd_audio);
        // Pause.
        state.packet = [0x4B, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let (has_data, is_error) = state.cmd_pause_resume(Some(&cdrom), &mut cd_audio);
        assert!(!has_data);
        assert!(!is_error);
        assert_eq!(cd_audio.state(), CdAudioState::Paused);
        // Resume.
        state.packet = [0x4B, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0];
        let (has_data, is_error) = state.cmd_pause_resume(Some(&cdrom), &mut cd_audio);
        assert!(!has_data);
        assert!(!is_error);
        assert_eq!(cd_audio.state(), CdAudioState::Playing);
    }

    #[test]
    fn read_cd_msf_reads_correct_sector() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        let cdrom = make_test_cdimage();

        // READ CD MSF: start MSF 0:02:42 (LBA 42), end MSF 0:02:43 (LBA 43), user data only.
        state.packet = [0xB9, 0, 0, 0, 2, 42, 0, 2, 43, 0x10, 0, 0];
        let (has_data, is_error) = state.cmd_read_cd_msf(Some(&cdrom));
        assert!(has_data);
        assert!(!is_error);
        assert_eq!(state.data_size, 2048);
        // Sector 42 marker.
        assert_eq!(state.data_buffer[0], 0);
        assert_eq!(state.data_buffer[1], 42);
    }

    #[test]
    fn read_cd_msf_bcd_mode() {
        let mut state = AtapiState::new();
        state.media_loaded = true;
        state.bcd_msf_mode = true;
        let cdrom = make_test_cdimage();

        // READ CD MSF with BCD-encoded addresses:
        // Start: 0:02:42 in BCD = 0x00, 0x02, 0x42. End: 0:02:43 in BCD = 0x00, 0x02, 0x43.
        state.packet = [0xB9, 0, 0, 0x00, 0x02, 0x42, 0x00, 0x02, 0x43, 0x10, 0, 0];
        let (has_data, is_error) = state.cmd_read_cd_msf(Some(&cdrom));
        assert!(has_data);
        assert!(!is_error);
        assert_eq!(state.data_size, 2048);
        // Sector 42 marker.
        assert_eq!(state.data_buffer[0], 0);
        assert_eq!(state.data_buffer[1], 42);
    }

    #[test]
    fn bcd_mode_reset_on_media_eject() {
        let mut state = AtapiState::new();
        state.bcd_msf_mode = true;
        state.media_ejected();
        assert!(!state.bcd_msf_mode);
    }

    #[test]
    fn bcd_mode_reset_on_device_reset() {
        let mut state = AtapiState::new();
        state.bcd_msf_mode = true;
        state.reset();
        assert!(!state.bcd_msf_mode);
    }

    #[test]
    fn mode_sense_other_page_does_not_set_bcd() {
        let mut state = AtapiState::new();
        // MODE SENSE(10): page 0x2A, should NOT set bcd_msf_mode.
        state.packet = [0x5A, 0, 0x2A, 0, 0, 0, 0, 0x01, 0x00, 0, 0, 0];
        let (has_data, is_error) = state.cmd_mode_sense_10(None);
        assert!(has_data);
        assert!(!is_error);
        assert!(!state.bcd_msf_mode);
    }

    #[test]
    fn mode_sense_page_01_correct_length() {
        let mut state = AtapiState::new();
        state.packet = [0x5A, 0, 0x01, 0, 0, 0, 0, 0x01, 0x00, 0, 0, 0];

        let (has_data, is_error) = state.cmd_mode_sense_10(None);
        assert!(has_data);
        assert!(!is_error);

        // Page 0x01 starts at offset 8 (after the 8-byte mode parameter header).
        assert_eq!(state.data_buffer[8], 0x01);
        assert_eq!(state.data_buffer[9], 0x06);
        // Total: 8 header + 2 page header + 6 page data = 16.
        assert_eq!(state.data_size, 16);
    }

    #[test]
    fn mode_sense_page_2a_correct_length() {
        let mut state = AtapiState::new();
        state.packet = [0x5A, 0, 0x2A, 0, 0, 0, 0, 0x01, 0x00, 0, 0, 0];

        let (has_data, is_error) = state.cmd_mode_sense_10(None);
        assert!(has_data);
        assert!(!is_error);

        // Page 0x2A starts at offset 8 (after the 8-byte mode parameter header).
        assert_eq!(state.data_buffer[8], 0x2A);
        assert_eq!(state.data_buffer[9], 0x12);
        // Total: 8 header + 2 page header + 18 page data = 28.
        assert_eq!(state.data_size, 28);
    }

    #[test]
    fn identify_packet_device_words_51_53_80() {
        let mut buffer = [0u8; 512];
        build_identify_packet_device(&mut buffer);

        let word = |n: usize| u16::from(buffer[n * 2]) | (u16::from(buffer[n * 2 + 1]) << 8);
        assert_eq!(word(51), 0x0278);
        assert_eq!(word(53), 0x0003);
        assert_eq!(word(80), 0x003E);
    }
}
