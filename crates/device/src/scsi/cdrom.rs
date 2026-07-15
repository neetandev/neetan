//! A SCSI CD-ROM target backed by a [`CdImage`] and a [`CdAudioPlayer`].
//!
//! The module has two layers. The free payload-builder functions implement
//! the optical command payloads (capacity, data reads, TOC, sub-channel,
//! mode pages) shared between this SCSI target and the ATAPI packet transport.
//! [`ScsiCdrom`] combines those builders with media, audio, and sense state
//! behind the same CDB interface as [`crate::scsi::ScsiDisk`].

use crate::{
    cd_audio::{CdAudioPlayer, CdAudioPlayerState, CdAudioState},
    cdrom::{CdImage, TrackType},
    scsi::command::{
        Direction, SenseData, asc, cdb_lun, opcode, read_write_lba_length, sense_key, status,
    },
};

save_state::runtime_state! {
/// Complete SCSI CD-ROM electronics state and retained media identity.
#[derive(Clone)]
pub struct ScsiCdromState {
    audio: CdAudioPlayerState,
    sense: SenseData,
    media_loaded: bool,
    media_changed: bool,
    prevent_removal: bool,
    media_identity: Option<save_state::ResourceIdentity>,
}}

/// CD-ROM sector size for data reads.
pub(crate) const CDROM_SECTOR_SIZE: usize = 2048;

/// Returns the user-data offset inside a raw 2352-byte sector (mode 2 uses a
/// sub-header before the user data).
pub(crate) fn raw_user_data_offset(raw_sector: &[u8]) -> usize {
    if raw_sector.get(15) == Some(&0x02) {
        24
    } else {
        16
    }
}

/// Converts a binary value to BCD (Binary-Coded Decimal).
pub(crate) fn hex_to_bcd(val: u8) -> u8 {
    ((val / 10) % 10) << 4 | (val % 10)
}

/// Converts a BCD (Binary-Coded Decimal) value to binary.
pub(crate) fn bcd_to_hex(val: u8) -> u8 {
    (val >> 4) * 10 + (val & 0x0F)
}

/// Decodes one MSF component byte, honoring the BCD encoding quirk.
pub(crate) fn decode_msf_byte(value: u8, bcd: bool) -> u32 {
    if bcd {
        u32::from(bcd_to_hex(value))
    } else {
        u32::from(value)
    }
}

/// Converts an LBA to MSF (minute, second, frame).
pub(crate) fn lba_to_msf(lba: u32) -> (u8, u8, u8) {
    let total_frames = lba;
    let f = (total_frames % 75) as u8;
    let total_seconds = total_frames / 75;
    let s = (total_seconds % 60) as u8;
    let m = (total_seconds / 60) as u8;
    (m, s, f)
}

/// Converts MSF (minute, second, frame) to an absolute LBA.
/// MSF addresses include the 150-frame lead-in offset, so we subtract it.
pub(crate) fn msf_to_lba(m: u32, s: u32, f: u32) -> u32 {
    (m * 60 * 75 + s * 75 + f).saturating_sub(150)
}

/// Returns the medium type byte for the MODE SENSE header.
pub(crate) fn medium_type(cdrom: Option<&CdImage>) -> u8 {
    let Some(cdrom) = cdrom else {
        return 0x70; // Door closed, no disc.
    };
    let mut has_data = false;
    let mut has_audio = false;
    for track in cdrom.tracks() {
        match track.track_type {
            TrackType::Data => has_data = true,
            TrackType::Audio => has_audio = true,
        }
    }
    match (has_data, has_audio) {
        (true, true) => 0x03,  // Data and audio.
        (true, false) => 0x01, // Data only.
        (false, true) => 0x02, // Audio only.
        (false, false) => 0x70,
    }
}

/// Maps the audio player state to the MMC audio-status byte.
pub(crate) fn audio_status_byte(state: CdAudioState) -> u8 {
    match state {
        CdAudioState::Playing => 0x11,
        CdAudioState::Paused => 0x12,
        CdAudioState::Stopped => 0x15,
    }
}

/// Writes a 4-byte address field as either MSF or LBA.
/// For MSF: adds the standard 150-frame (2-second) lead-in offset per Red
/// Book. When `bcd` is true, MSF values are BCD-encoded.
pub(crate) fn store_address(buf: &mut [u8], lba: u32, msf: bool, bcd: bool) {
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

/// Writes bytes 5..16 of a READ SUB-CHANNEL format-0x01 response from the
/// 12-byte raw Sub-Q recovered from disc.
///
/// Sub-Q byte 0 stores Control in the high nibble and ADR in the low nibble
/// (Red Book); the MMC response byte 5 uses the opposite nibble order. The
/// MSF fields in Sub-Q are BCD; the absolute MSF includes the standard
/// 150-sector lead-in, the relative MSF does not.
pub(crate) fn decode_subq_into_response(sub_q: &[u8; 12], buffer: &mut [u8], msf: bool, bcd: bool) {
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

/// Builds the 8-byte READ CAPACITY(10) payload (last LBA and block size).
pub(crate) fn read_capacity_payload(cdrom: &CdImage) -> [u8; 8] {
    let last_lba = cdrom.total_sectors().saturating_sub(1);
    let block_size = CDROM_SECTOR_SIZE as u32;
    let mut data = [0u8; 8];
    data[0..4].copy_from_slice(&last_lba.to_be_bytes());
    data[4..8].copy_from_slice(&block_size.to_be_bytes());
    data
}

/// Reads `count` cooked 2048-byte data sectors starting at `lba`. Returns
/// `None` if any sector is unreadable or out of range.
pub(crate) fn read_data_sectors(cdrom: &CdImage, lba: u32, count: u32) -> Option<Vec<u8>> {
    let total_bytes = count as usize * CDROM_SECTOR_SIZE;
    let mut data = vec![0u8; total_bytes];
    for index in 0..count {
        let offset = index as usize * CDROM_SECTOR_SIZE;
        cdrom.read_sector(lba + index, &mut data[offset..offset + CDROM_SECTOR_SIZE])?;
    }
    Some(data)
}

/// Builds the READ TOC format 0 payload (track list plus lead-out).
pub(crate) fn toc_format_0_payload(
    cdrom: &CdImage,
    starting_track: u8,
    msf: bool,
    bcd: bool,
) -> Vec<u8> {
    let tracks = cdrom.tracks();
    let track_count = cdrom.track_count();
    let first_track = if starting_track == 0 {
        1
    } else {
        starting_track
    };

    let valid_tracks: Vec<&crate::cdrom::Track> =
        tracks.iter().filter(|t| t.number >= first_track).collect();

    // Header (4 bytes) + track descriptors (8 bytes each) + lead-out (8 bytes).
    let descriptor_count = valid_tracks.len() + 1;
    let data_length = 2 + descriptor_count * 8;
    let total_length = 2 + data_length;

    let mut data = vec![0u8; total_length];
    data[0] = (data_length >> 8) as u8;
    data[1] = data_length as u8;
    data[2] = 1; // First track number.
    data[3] = track_count; // Last track number.

    let mut offset = 4;
    for track in &valid_tracks {
        data[offset] = 0;
        data[offset + 1] = match track.track_type {
            TrackType::Data => 0x14,
            TrackType::Audio => 0x10,
        };
        data[offset + 2] = track.number;
        data[offset + 3] = 0;
        store_address(&mut data[offset + 4..offset + 8], track.start_lba, msf, bcd);
        offset += 8;
    }

    // Lead-out entry (track 0xAA).
    let lead_out_lba = cdrom.total_sectors();
    data[offset] = 0;
    data[offset + 1] = 0x14;
    data[offset + 2] = 0xAA;
    data[offset + 3] = 0;
    store_address(&mut data[offset + 4..offset + 8], lead_out_lba, msf, bcd);

    data
}

/// Builds the READ TOC format 1 payload (session info).
pub(crate) fn toc_format_1_payload(cdrom: &CdImage, msf: bool, bcd: bool) -> Vec<u8> {
    let mut data = vec![0u8; 12];
    data[0] = 0x00;
    data[1] = 0x0A; // Data length = 10.
    data[2] = 1; // First session.
    data[3] = 1; // Last session.

    data[4] = 0;
    data[5] = 0x14;
    data[6] = 1; // First track in session.
    data[7] = 0;
    let lba = cdrom.track(1).map_or(0, |t| t.start_lba);
    store_address(&mut data[8..12], lba, msf, bcd);

    data
}

/// Builds the READ TOC format 2 payload (full TOC, raw Q sub-channel).
pub(crate) fn toc_format_2_payload(cdrom: &CdImage, bcd: bool) -> Vec<u8> {
    let tracks = cdrom.tracks();
    let track_count = cdrom.track_count();

    // Header (4 bytes) + A0/A1/A2 entries + track entries (11 bytes each).
    let entry_count = 3 + tracks.len();
    let data_length = 2 + entry_count * 11;
    let total_length = 2 + data_length;

    let mut data = vec![0u8; total_length];
    data[0] = (data_length >> 8) as u8;
    data[1] = data_length as u8;
    data[2] = 1; // First session.
    data[3] = 1; // Last session.

    let mut offset = 4;

    // Point A0: first track number.
    data[offset] = 1;
    data[offset + 1] = 0x14;
    data[offset + 2] = 0;
    data[offset + 3] = 0xA0;
    data[offset + 8] = 1;
    offset += 11;

    // Point A1: last track number.
    data[offset] = 1;
    data[offset + 1] = 0x14;
    data[offset + 2] = 0;
    data[offset + 3] = 0xA1;
    data[offset + 8] = track_count;
    offset += 11;

    // Point A2: lead-out position in MSF.
    let lead_out = cdrom.total_sectors();
    let (m, s, f) = lba_to_msf(lead_out);
    data[offset] = 1;
    data[offset + 1] = 0x14;
    data[offset + 2] = 0;
    data[offset + 3] = 0xA2;
    if bcd {
        data[offset + 8] = hex_to_bcd(m);
        data[offset + 9] = hex_to_bcd(s);
        data[offset + 10] = hex_to_bcd(f);
    } else {
        data[offset + 8] = m;
        data[offset + 9] = s;
        data[offset + 10] = f;
    }
    offset += 11;

    for track in tracks {
        let ctl = match track.track_type {
            TrackType::Data => 0x14,
            TrackType::Audio => 0x10,
        };
        let (m, s, f) = lba_to_msf(track.start_lba);
        data[offset] = 1;
        data[offset + 1] = ctl;
        data[offset + 2] = 0;
        data[offset + 3] = track.number;
        if bcd {
            data[offset + 8] = hex_to_bcd(m);
            data[offset + 9] = hex_to_bcd(s);
            data[offset + 10] = hex_to_bcd(f);
        } else {
            data[offset + 8] = m;
            data[offset + 9] = s;
            data[offset + 10] = f;
        }
        offset += 11;
    }

    data
}

/// Builds the 16-byte READ SUB-CHANNEL format-0x01 (current position) payload.
pub(crate) fn sub_channel_position_payload(
    cdrom: Option<&CdImage>,
    current_lba: u32,
    audio_status: u8,
    msf: bool,
    bcd: bool,
) -> [u8; 16] {
    let mut data = [0u8; 16];
    data[0] = 0x00; // Reserved.
    data[1] = audio_status;
    data[2] = 0x00; // Sub-channel data length (MSB).
    data[3] = 0x0C; // Sub-channel data length = 12.
    data[4] = 0x01; // Sub-Q format code: current position.

    if let Some(sub_q) = cdrom.and_then(|cdrom| cdrom.read_subchannel_q(current_lba)) {
        decode_subq_into_response(&sub_q, &mut data, msf, bcd);
    } else if let Some(cdrom) = cdrom {
        let track = cdrom.track_for_lba(current_lba).or_else(|| cdrom.track(1));
        let adr_ctl = track.map_or(0x14, |t| match t.track_type {
            TrackType::Data => 0x14,
            TrackType::Audio => 0x10,
        });
        data[5] = adr_ctl;
        data[6] = track.map_or(1, |t| t.number);
        data[7] = 0x01;
        let track_relative_lba = track.map_or(0, |t| current_lba.saturating_sub(t.start_lba));
        store_address(&mut data[8..12], current_lba, msf, bcd);
        store_address(&mut data[12..16], track_relative_lba, msf, bcd);
    } else {
        data[5] = 0x14;
        data[6] = 0x01;
        data[7] = 0x01;
        store_address(&mut data[8..12], 0, msf, bcd);
        store_address(&mut data[12..16], 0, msf, bcd);
    }

    data
}

/// Writes mode page 0x01 (Read Error Recovery Parameters) at `offset`,
/// returning the end offset.
pub(crate) fn write_mode_page_01(buffer: &mut Vec<u8>, offset: usize) -> usize {
    let end = offset + 8;
    if end > buffer.len() {
        buffer.resize(end, 0);
    }
    buffer[offset] = 0x01; // Page code.
    buffer[offset + 1] = 0x06; // Page length.
    end
}

/// Writes mode page 0x0D (CD-ROM Device Parameters) at `offset`, returning
/// the end offset.
pub(crate) fn write_mode_page_0d(buffer: &mut Vec<u8>, offset: usize) -> usize {
    let end = offset + 8;
    if end > buffer.len() {
        buffer.resize(end, 0);
    }
    buffer[offset] = 0x0D; // Page code.
    buffer[offset + 1] = 0x06; // Page length.
    buffer[offset + 5] = 0x3C; // Inactivity timer multiplier.
    buffer[offset + 7] = 0x4B; // Number of MSF-S units per MSF-M unit (75).
    end
}

/// Writes mode page 0x0E (CD-ROM Audio Control Parameters) at `offset`,
/// returning the end offset.
pub(crate) fn write_mode_page_0e(buffer: &mut Vec<u8>, offset: usize) -> usize {
    let end = offset + 16;
    if end > buffer.len() {
        buffer.resize(end, 0);
    }
    buffer[offset] = 0x0E; // Page code.
    buffer[offset + 1] = 0x0E; // Page length.
    buffer[offset + 2] = 0x04; // Immed = 1.
    buffer[offset + 7] = 0x4B; // Number of frames per second (75).
    // Port 0: channel 0, volume 0xFF.
    buffer[offset + 8] = 0x01;
    buffer[offset + 9] = 0xFF;
    // Port 1: channel 1, volume 0xFF.
    buffer[offset + 10] = 0x02;
    buffer[offset + 11] = 0xFF;
    end
}

/// Writes mode page 0x2A (CD-ROM Capabilities and Mechanical Status) at
/// `offset`, returning the end offset.
pub(crate) fn write_mode_page_2a(buffer: &mut Vec<u8>, offset: usize) -> usize {
    let end = offset + 20;
    if end > buffer.len() {
        buffer.resize(end, 0);
    }
    buffer[offset] = 0x2A; // Page code.
    buffer[offset + 1] = 0x12; // Page length (18 bytes).
    buffer[offset + 4] = 0x71;
    buffer[offset + 5] = 0x65;
    buffer[offset + 6] = 0x2B;
    buffer[offset + 7] = 0x07;
    // Max speed: 4x (706 KB/s).
    buffer[offset + 8] = 0x02;
    buffer[offset + 9] = 0xC2;
    buffer[offset + 11] = 0xFF;
    // Buffer size (64 KB).
    buffer[offset + 12] = 0x00;
    buffer[offset + 13] = 0x80;
    // Current speed: 4x.
    buffer[offset + 14] = 0x02;
    buffer[offset + 15] = 0xC2;
    end
}

/// A SCSI CD-ROM target (LUN 0, device type 0x05) owning its media and audio
/// playback state.
#[derive(Debug)]
pub struct ScsiCdrom {
    media: Option<CdImage>,
    audio: CdAudioPlayer,
    sense: SenseData,
    media_loaded: bool,
    media_changed: bool,
    prevent_removal: bool,
}

impl ScsiCdrom {
    /// Creates an empty CD-ROM target mixing audio at the given sample rate.
    pub fn new(output_sample_rate: u32) -> Self {
        Self {
            media: None,
            audio: CdAudioPlayer::new(output_sample_rate),
            sense: SenseData::CLEAR,
            media_loaded: false,
            media_changed: false,
            prevent_removal: false,
        }
    }

    /// Captures target electronics, audio history, and disc identity.
    pub fn capture_state(&self) -> ScsiCdromState {
        ScsiCdromState {
            audio: self.audio.capture_state(),
            sense: self.sense,
            media_loaded: self.media_loaded,
            media_changed: self.media_changed,
            prevent_removal: self.prevent_removal,
            media_identity: self.media.as_ref().map(CdImage::identity),
        }
    }

    /// Validates target state against the retained disc and output stream.
    pub fn validate_state(
        &self,
        state: &ScsiCdromState,
    ) -> Result<(), save_state::StateValidationError> {
        let current_identity = self.media.as_ref().map(CdImage::identity);
        if state.media_identity != current_identity || state.media_loaded != self.media.is_some() {
            return Err(save_state::StateValidationError::new(
                "SCSI CD-ROM media identity differs",
            ));
        }
        self.audio.validate_state(&state.audio)
    }

    /// Restores target electronics while retaining the inserted disc.
    pub fn restore_state(
        &mut self,
        state: ScsiCdromState,
    ) -> Result<(), save_state::StateValidationError> {
        self.validate_state(&state)?;
        self.audio.restore_state(state.audio)?;
        self.sense = state.sense;
        self.media_loaded = state.media_loaded;
        self.media_changed = state.media_changed;
        self.prevent_removal = state.prevent_removal;
        Ok(())
    }

    /// Inserts a disc, raising a unit attention for the media change.
    pub fn insert_media(&mut self, image: CdImage) {
        self.audio.reset();
        self.media = Some(image);
        self.media_loaded = true;
        self.media_changed = true;
        self.sense = SenseData::new(
            sense_key::UNIT_ATTENTION,
            asc::NOT_READY_TO_READY_TRANSITION,
        );
    }

    /// Removes the disc, raising a media change.
    pub fn eject_media(&mut self) {
        self.audio.reset();
        self.media = None;
        self.media_loaded = false;
        self.media_changed = true;
        self.sense = SenseData::new(sense_key::NOT_READY, asc::MEDIUM_NOT_PRESENT);
    }

    /// Whether a disc image is inserted.
    pub fn has_media(&self) -> bool {
        self.media.is_some()
    }

    /// The inserted disc image, if any.
    pub fn media(&self) -> Option<&CdImage> {
        self.media.as_ref()
    }

    /// The audio playback engine.
    pub fn audio(&self) -> &CdAudioPlayer {
        &self.audio
    }

    /// Mutable access to the audio playback engine.
    pub fn audio_mut(&mut self) -> &mut CdAudioPlayer {
        &mut self.audio
    }

    /// Splits the target into the media reference and the audio engine for
    /// transports that drive both independently.
    pub fn media_and_audio_mut(&mut self) -> (Option<&CdImage>, &mut CdAudioPlayer) {
        (self.media.as_ref(), &mut self.audio)
    }

    /// Mixes any playing CD audio into the interleaved stereo output buffer.
    pub fn generate_audio_samples(&mut self, volumes: [f32; 2], output: &mut [f32]) {
        if let Some(media) = self.media.as_ref() {
            self.audio.generate_samples(media, volumes, output);
        }
    }

    /// Flushes pending state; a read-only target has nothing to persist.
    pub fn flush(&mut self) {}

    /// Records sense data and returns CHECK CONDITION.
    fn fail(&mut self, key: u8, additional: u8) -> u8 {
        self.sense = SenseData::new(key, additional);
        status::CHECK_CONDITION
    }

    /// Clears sense data and returns GOOD.
    fn ok(&mut self) -> u8 {
        self.sense = SenseData::CLEAR;
        status::GOOD
    }

    /// Whether the medium is present and loaded.
    fn medium_ready(&self) -> bool {
        self.media_loaded && self.media.is_some()
    }

    /// Verifies the medium is ready, recording unit-attention or not-ready
    /// sense on failure.
    fn check_ready(&mut self, clear_attention: bool) -> Result<(), u8> {
        if self.media_changed {
            if clear_attention {
                self.media_changed = false;
            }
            if self.medium_ready() {
                return Err(self.fail(
                    sense_key::UNIT_ATTENTION,
                    asc::NOT_READY_TO_READY_TRANSITION,
                ));
            }
            return Err(self.fail(sense_key::NOT_READY, asc::MEDIUM_NOT_PRESENT));
        }
        if !self.medium_ready() {
            return Err(self.fail(sense_key::NOT_READY, asc::MEDIUM_NOT_PRESENT));
        }
        Ok(())
    }

    /// Returns CHECK CONDITION status if the CDB targets a non-zero LUN.
    fn check_lun(&mut self, cdb: &[u8]) -> Option<u8> {
        if cdb_lun(cdb) != 0 {
            Some(self.fail(sense_key::ILLEGAL_REQUEST, asc::LOGICAL_UNIT_NOT_SUPPORTED))
        } else {
            None
        }
    }

    /// The data-phase direction a command requires.
    pub fn direction(&self, cdb: &[u8]) -> Direction {
        match cdb.first().copied().unwrap_or(0xFF) {
            opcode::REQUEST_SENSE
            | opcode::INQUIRY
            | opcode::MODE_SENSE6
            | opcode::READ_CAPACITY
            | opcode::READ6
            | opcode::READ10
            | opcode::READ_TOC
            | opcode::READ_SUB_CHANNEL => Direction::In,
            opcode::MODE_SELECT6 => Direction::Out,
            _ => Direction::None,
        }
    }

    /// Number of DATA OUT bytes a command expects.
    pub fn data_out_length(&self, cdb: &[u8]) -> usize {
        match cdb.first().copied().unwrap_or(0xFF) {
            opcode::MODE_SELECT6 => *cdb.get(4).unwrap_or(&0) as usize,
            _ => 0,
        }
    }

    /// Executes a command with no data phase and returns the STATUS byte.
    pub fn execute_no_data(&mut self, cdb: &[u8]) -> u8 {
        if let Some(bad) = self.check_lun(cdb) {
            return bad;
        }
        match cdb.first().copied().unwrap_or(0xFF) {
            opcode::TEST_UNIT_READY => match self.check_ready(true) {
                Ok(()) => self.ok(),
                Err(bad) => bad,
            },
            opcode::REZERO_UNIT => match self.check_ready(false) {
                Ok(()) => self.ok(),
                Err(bad) => bad,
            },
            opcode::START_STOP => self.start_stop(cdb),
            opcode::PREVENT_ALLOW => {
                self.prevent_removal = cdb.get(4).unwrap_or(&0) & 0x01 != 0;
                self.ok()
            }
            opcode::SEEK6 | opcode::SEEK10 => self.seek(cdb),
            opcode::PLAY_AUDIO10 => self.play_audio_10(cdb),
            opcode::PLAY_AUDIO_MSF => self.play_audio_msf(cdb),
            opcode::PAUSE_RESUME => self.pause_resume(cdb),
            _ => self.fail(sense_key::ILLEGAL_REQUEST, asc::INVALID_COMMAND),
        }
    }

    /// Produces the DATA IN bytes for a read-type command, plus its STATUS byte.
    pub fn data_in(&mut self, cdb: &[u8]) -> (Vec<u8>, u8) {
        if let Some(bad) = self.check_lun(cdb) {
            return (Vec::new(), bad);
        }
        match cdb.first().copied().unwrap_or(0xFF) {
            opcode::REQUEST_SENSE => {
                let data = self.request_sense_data(cdb);
                self.sense = SenseData::CLEAR;
                self.media_changed = false;
                (data, status::GOOD)
            }
            opcode::INQUIRY => {
                self.media_changed = false;
                (self.inquiry_data(cdb), self.ok())
            }
            opcode::READ_CAPACITY => match self.check_ready(false) {
                Ok(()) => {
                    let media = self.media.as_ref().expect("checked by check_ready");
                    let data = read_capacity_payload(media).to_vec();
                    (data, self.ok())
                }
                Err(bad) => (Vec::new(), bad),
            },
            opcode::MODE_SENSE6 => self.mode_sense_6(cdb),
            opcode::READ6 | opcode::READ10 => self.read_data(cdb),
            opcode::READ_TOC => self.read_toc(cdb),
            opcode::READ_SUB_CHANNEL => self.read_sub_channel(cdb),
            _ => (
                Vec::new(),
                self.fail(sense_key::ILLEGAL_REQUEST, asc::INVALID_COMMAND),
            ),
        }
    }

    /// Consumes the DATA OUT bytes for a command and returns its STATUS.
    pub fn write_data_out(&mut self, cdb: &[u8], _data: &[u8]) -> u8 {
        match cdb.first().copied().unwrap_or(0xFF) {
            // Mode parameters are accepted and ignored.
            opcode::MODE_SELECT6 => self.ok(),
            _ => self.fail(sense_key::ILLEGAL_REQUEST, asc::INVALID_COMMAND),
        }
    }

    fn start_stop(&mut self, cdb: &[u8]) -> u8 {
        let flags = *cdb.get(4).unwrap_or(&0);
        let load_eject = flags & 0x02 != 0;
        let start = flags & 0x01 != 0;

        if load_eject && !start {
            if self.prevent_removal {
                return self.fail(sense_key::ILLEGAL_REQUEST, asc::INVALID_FIELD_IN_CDB);
            }
            self.audio.reset();
            self.media_loaded = false;
            self.media_changed = true;
        } else if load_eject && start && self.media.is_some() && !self.media_loaded {
            self.media_loaded = true;
            self.media_changed = true;
        }
        self.ok()
    }

    fn seek(&mut self, cdb: &[u8]) -> u8 {
        if let Err(bad) = self.check_ready(false) {
            return bad;
        }
        let media = self.media.as_ref().expect("checked by check_ready");
        let lba = match cdb.first().copied().unwrap_or(0xFF) {
            opcode::SEEK6 => {
                ((*cdb.get(1).unwrap_or(&0) & 0x1F) as u32) << 16
                    | (*cdb.get(2).unwrap_or(&0) as u32) << 8
                    | *cdb.get(3).unwrap_or(&0) as u32
            }
            _ => u32::from_be_bytes([
                *cdb.get(2).unwrap_or(&0),
                *cdb.get(3).unwrap_or(&0),
                *cdb.get(4).unwrap_or(&0),
                *cdb.get(5).unwrap_or(&0),
            ]),
        };
        if lba >= media.total_sectors() {
            return self.fail(sense_key::ILLEGAL_REQUEST, asc::LBA_OUT_OF_RANGE);
        }
        self.ok()
    }

    fn play_audio_10(&mut self, cdb: &[u8]) -> u8 {
        if let Err(bad) = self.check_ready(false) {
            return bad;
        }
        let media = self.media.as_ref().expect("checked by check_ready");
        let start_lba = u32::from_be_bytes([
            *cdb.get(2).unwrap_or(&0),
            *cdb.get(3).unwrap_or(&0),
            *cdb.get(4).unwrap_or(&0),
            *cdb.get(5).unwrap_or(&0),
        ]);
        let sector_count =
            u32::from(*cdb.get(7).unwrap_or(&0)) << 8 | u32::from(*cdb.get(8).unwrap_or(&0));
        self.audio.play(media, start_lba, sector_count);
        self.ok()
    }

    fn play_audio_msf(&mut self, cdb: &[u8]) -> u8 {
        if let Err(bad) = self.check_ready(false) {
            return bad;
        }
        let media = self.media.as_ref().expect("checked by check_ready");
        let start_m = u32::from(*cdb.get(3).unwrap_or(&0));
        let start_s = u32::from(*cdb.get(4).unwrap_or(&0));
        let start_f = u32::from(*cdb.get(5).unwrap_or(&0));
        let end_m = u32::from(*cdb.get(6).unwrap_or(&0));
        let end_s = u32::from(*cdb.get(7).unwrap_or(&0));
        let end_f = u32::from(*cdb.get(8).unwrap_or(&0));
        let start_lba = msf_to_lba(start_m, start_s, start_f);
        let end_lba = msf_to_lba(end_m, end_s, end_f);
        let sector_count = end_lba.saturating_sub(start_lba);
        self.audio.play(media, start_lba, sector_count);
        self.ok()
    }

    fn pause_resume(&mut self, cdb: &[u8]) -> u8 {
        let resume = cdb.get(8).unwrap_or(&0) & 0x01 != 0;
        if resume {
            if let Some(media) = self.media.as_ref() {
                self.audio.resume(media);
            }
        } else {
            self.audio.stop();
        }
        self.ok()
    }

    fn read_data(&mut self, cdb: &[u8]) -> (Vec<u8>, u8) {
        if let Err(bad) = self.check_ready(false) {
            return (Vec::new(), bad);
        }
        let Some((lba, blocks)) = read_write_lba_length(cdb) else {
            return (
                Vec::new(),
                self.fail(sense_key::ILLEGAL_REQUEST, asc::INVALID_FIELD_IN_CDB),
            );
        };
        if blocks == 0 {
            return (Vec::new(), self.ok());
        }
        let media = self.media.as_ref().expect("checked by check_ready");
        match read_data_sectors(media, lba, blocks) {
            Some(data) => (data, self.ok()),
            None => (
                Vec::new(),
                self.fail(sense_key::ILLEGAL_REQUEST, asc::LBA_OUT_OF_RANGE),
            ),
        }
    }

    fn read_toc(&mut self, cdb: &[u8]) -> (Vec<u8>, u8) {
        if let Err(bad) = self.check_ready(false) {
            return (Vec::new(), bad);
        }
        self.media_changed = false;
        let media = self.media.as_ref().expect("checked by check_ready");

        let msf = cdb.get(1).unwrap_or(&0) & 0x02 != 0;
        let format = cdb.get(2).unwrap_or(&0) & 0x0F;
        let starting_track = *cdb.get(6).unwrap_or(&0);
        let allocation_length =
            usize::from(*cdb.get(7).unwrap_or(&0)) << 8 | usize::from(*cdb.get(8).unwrap_or(&0));

        let data = match format {
            0 => toc_format_0_payload(media, starting_track, msf, false),
            1 => toc_format_1_payload(media, msf, false),
            2 => toc_format_2_payload(media, false),
            _ => {
                return (
                    Vec::new(),
                    self.fail(sense_key::ILLEGAL_REQUEST, asc::INVALID_FIELD_IN_CDB),
                );
            }
        };
        (truncate_to(data, allocation_length), self.ok())
    }

    fn read_sub_channel(&mut self, cdb: &[u8]) -> (Vec<u8>, u8) {
        if let Err(bad) = self.check_ready(false) {
            return (Vec::new(), bad);
        }

        let audio_status = audio_status_byte(self.audio.state());
        let sub_q = cdb.get(2).unwrap_or(&0) & 0x40 != 0;
        let format = *cdb.get(3).unwrap_or(&0);
        let msf = cdb.get(1).unwrap_or(&0) & 0x02 != 0;
        let allocation_length =
            usize::from(*cdb.get(7).unwrap_or(&0)) << 8 | usize::from(*cdb.get(8).unwrap_or(&0));

        if sub_q && format == 0x01 {
            let (current_lba, _, _) = self.audio.current_position();
            let data = sub_channel_position_payload(
                self.media.as_ref(),
                current_lba,
                audio_status,
                msf,
                false,
            );
            return (truncate_to(data.to_vec(), allocation_length), self.ok());
        }

        // Default: minimal 4-byte header with the audio status.
        let mut data = vec![0u8; 4];
        data[1] = audio_status;
        (truncate_to(data, allocation_length), self.ok())
    }

    fn mode_sense_6(&mut self, cdb: &[u8]) -> (Vec<u8>, u8) {
        let page_code = cdb.get(2).unwrap_or(&0) & 0x3F;
        let disable_block_descriptor = cdb.get(1).unwrap_or(&0) & 0x08 != 0;
        let allocation_length = *cdb.get(4).unwrap_or(&0) as usize;

        // 4-byte MODE SENSE(6) header.
        let mut data = vec![0u8; 4];
        data[1] = medium_type(self.media.as_ref());

        if !disable_block_descriptor {
            let blocks = self.media.as_ref().map_or(0, CdImage::total_sectors);
            let mut descriptor = [0u8; 8];
            descriptor[1] = (blocks >> 16) as u8;
            descriptor[2] = (blocks >> 8) as u8;
            descriptor[3] = blocks as u8;
            descriptor[5] = (CDROM_SECTOR_SIZE >> 16) as u8;
            descriptor[6] = (CDROM_SECTOR_SIZE >> 8) as u8;
            descriptor[7] = CDROM_SECTOR_SIZE as u8;
            data.extend_from_slice(&descriptor);
            data[3] = 8; // Block descriptor length.
        }

        let mut offset = data.len();
        match page_code {
            0x01 => offset = write_mode_page_01(&mut data, offset),
            0x0D => offset = write_mode_page_0d(&mut data, offset),
            0x0E => offset = write_mode_page_0e(&mut data, offset),
            0x2A => offset = write_mode_page_2a(&mut data, offset),
            0x3F => {
                offset = write_mode_page_01(&mut data, offset);
                offset = write_mode_page_0d(&mut data, offset);
                offset = write_mode_page_0e(&mut data, offset);
                offset = write_mode_page_2a(&mut data, offset);
            }
            0x00 => {}
            _ => {
                return (
                    Vec::new(),
                    self.fail(sense_key::ILLEGAL_REQUEST, asc::INVALID_FIELD_IN_CDB),
                );
            }
        }
        data.truncate(offset);

        let mut data = truncate_to(data, allocation_length);
        if !data.is_empty() {
            data[0] = (data.len() - 1) as u8;
        }
        (data, self.ok())
    }

    /// Fixed-format REQUEST SENSE data (up to the allocation length in cdb[4]).
    fn request_sense_data(&self, cdb: &[u8]) -> Vec<u8> {
        let mut data = vec![0u8; 18];
        data[0] = 0x70; // Current error, fixed format.
        data[2] = self.sense.key;
        data[7] = 10; // Additional sense length.
        data[12] = self.sense.asc;
        data[13] = self.sense.ascq;
        let allocation = *cdb.get(4).unwrap_or(&18) as usize;
        if allocation != 0 && allocation < data.len() {
            data.truncate(allocation);
        }
        data
    }

    /// Standard INQUIRY data (36 bytes) for a removable CD-ROM device.
    fn inquiry_data(&self, cdb: &[u8]) -> Vec<u8> {
        let mut data = vec![0u8; 36];
        data[0] = 0x05; // Peripheral device type: CD-ROM.
        data[1] = 0x80; // Removable medium.
        data[2] = 0x02; // SCSI-2.
        data[3] = 0x02; // Response data format.
        data[4] = 31; // Additional length (36 - 5).
        data[8..16].copy_from_slice(b"NEETAN  ");
        data[16..32].copy_from_slice(b"SCSI CD-ROM     ");
        data[32..36].copy_from_slice(b"1.0 ");
        let allocation = *cdb.get(4).unwrap_or(&36) as usize;
        if allocation != 0 && allocation < data.len() {
            data.truncate(allocation);
        }
        data
    }
}

/// Truncates payload data to a non-zero allocation length.
fn truncate_to(mut data: Vec<u8>, allocation_length: usize) -> Vec<u8> {
    if allocation_length != 0 && allocation_length < data.len() {
        data.truncate(allocation_length);
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdrom::CdImage;

    /// Builds a small mixed-mode image: one 2048-byte data track (16 sectors)
    /// and one audio track (75 sectors).
    fn make_mixed_image() -> CdImage {
        let cue = "FILE \"disc.bin\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    INDEX 01 00:00:16\n";
        let mut bin = vec![0u8; 16 * 2048 + 75 * 2352];
        for sector in 0..16 {
            bin[sector * 2048] = sector as u8 + 1;
        }
        for (index, byte) in bin[16 * 2048..].iter_mut().enumerate() {
            *byte = (index % 251) as u8;
        }
        CdImage::from_cue_files(cue, vec![bin]).unwrap()
    }

    fn ready_cdrom() -> ScsiCdrom {
        let mut cdrom = ScsiCdrom::new(44100);
        cdrom.insert_media(make_mixed_image());
        // Acknowledge the insertion unit attention.
        let first = cdrom.execute_no_data(&[opcode::TEST_UNIT_READY, 0, 0, 0, 0, 0]);
        assert_eq!(first, status::CHECK_CONDITION);
        cdrom.data_in(&[opcode::REQUEST_SENSE, 0, 0, 0, 18, 0]);
        cdrom
    }

    #[test]
    fn inquiry_reports_removable_cdrom() {
        let mut cdrom = ScsiCdrom::new(44100);
        let (data, st) = cdrom.data_in(&[opcode::INQUIRY, 0, 0, 0, 36, 0]);
        assert_eq!(st, status::GOOD);
        assert_eq!(data[0], 0x05);
        assert_eq!(data[1], 0x80);
        assert_eq!(&data[16..32], b"SCSI CD-ROM     ");
    }

    #[test]
    fn unit_attention_on_insertion_then_ready() {
        let mut cdrom = ScsiCdrom::new(44100);
        cdrom.insert_media(make_mixed_image());

        let st = cdrom.execute_no_data(&[opcode::TEST_UNIT_READY, 0, 0, 0, 0, 0]);
        assert_eq!(st, status::CHECK_CONDITION);
        let (sense, _) = cdrom.data_in(&[opcode::REQUEST_SENSE, 0, 0, 0, 18, 0]);
        assert_eq!(sense[2], sense_key::UNIT_ATTENTION);
        assert_eq!(sense[12], asc::NOT_READY_TO_READY_TRANSITION);

        let st = cdrom.execute_no_data(&[opcode::TEST_UNIT_READY, 0, 0, 0, 0, 0]);
        assert_eq!(st, status::GOOD);
    }

    #[test]
    fn no_medium_reports_not_ready() {
        let mut cdrom = ScsiCdrom::new(44100);
        let st = cdrom.execute_no_data(&[opcode::TEST_UNIT_READY, 0, 0, 0, 0, 0]);
        assert_eq!(st, status::CHECK_CONDITION);
        let (sense, _) = cdrom.data_in(&[opcode::REQUEST_SENSE, 0, 0, 0, 18, 0]);
        assert_eq!(sense[2], sense_key::NOT_READY);
        assert_eq!(sense[12], asc::MEDIUM_NOT_PRESENT);
    }

    #[test]
    fn read_capacity_reports_block_size_2048() {
        let expected_last_lba = make_mixed_image().total_sectors() - 1;
        let mut cdrom = ready_cdrom();
        let (data, st) = cdrom.data_in(&[opcode::READ_CAPACITY, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(st, status::GOOD);
        assert_eq!(
            u32::from_be_bytes([data[4], data[5], data[6], data[7]]),
            2048
        );
        let last_lba = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        assert_eq!(last_lba, expected_last_lba);
    }

    #[test]
    fn read_10_returns_data_sectors() {
        let mut cdrom = ready_cdrom();
        let (data, st) = cdrom.data_in(&[opcode::READ10, 0, 0, 0, 0, 2, 0, 0, 2, 0]);
        assert_eq!(st, status::GOOD);
        assert_eq!(data.len(), 2 * 2048);
        assert_eq!(data[0], 3);
        assert_eq!(data[2048], 4);
    }

    #[test]
    fn read_out_of_range_sets_sense() {
        let mut cdrom = ready_cdrom();
        let (data, st) = cdrom.data_in(&[opcode::READ10, 0, 0, 0x10, 0, 0, 0, 0, 1, 0]);
        assert_eq!(st, status::CHECK_CONDITION);
        assert!(data.is_empty());
        let (sense, _) = cdrom.data_in(&[opcode::REQUEST_SENSE, 0, 0, 0, 18, 0]);
        assert_eq!(sense[2], sense_key::ILLEGAL_REQUEST);
        assert_eq!(sense[12], asc::LBA_OUT_OF_RANGE);
    }

    #[test]
    fn read_toc_format_0_lists_tracks_and_lead_out() {
        let mut cdrom = ready_cdrom();
        let (data, st) = cdrom.data_in(&[opcode::READ_TOC, 0, 0, 0, 0, 0, 0, 4, 0, 0]);
        assert_eq!(st, status::GOOD);
        assert_eq!(data[2], 1);
        assert_eq!(data[3], 2);
        // Track 1 (data), track 2 (audio), lead-out.
        assert_eq!(data[4 + 1], 0x14);
        assert_eq!(data[4 + 2], 1);
        assert_eq!(data[12 + 1], 0x10);
        assert_eq!(data[12 + 2], 2);
        assert_eq!(data[20 + 2], 0xAA);
    }

    #[test]
    fn play_and_sub_channel_report_playback() {
        let mut cdrom = ready_cdrom();
        // PLAY AUDIO(10) at the audio track start (LBA 16), 75 sectors.
        let st = cdrom.execute_no_data(&[opcode::PLAY_AUDIO10, 0, 0, 0, 0, 16, 0, 0, 75, 0]);
        assert_eq!(st, status::GOOD);
        assert_eq!(cdrom.audio().state(), CdAudioState::Playing);

        let (data, st) =
            cdrom.data_in(&[opcode::READ_SUB_CHANNEL, 0, 0x40, 0x01, 0, 0, 0, 0, 16, 0]);
        assert_eq!(st, status::GOOD);
        assert_eq!(data[1], 0x11); // Audio status: playing.
        assert_eq!(data[6], 2); // Track number.

        // PAUSE.
        let st = cdrom.execute_no_data(&[opcode::PAUSE_RESUME, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(st, status::GOOD);
        assert_eq!(cdrom.audio().state(), CdAudioState::Paused);

        // RESUME.
        let st = cdrom.execute_no_data(&[opcode::PAUSE_RESUME, 0, 0, 0, 0, 0, 0, 0, 1, 0]);
        assert_eq!(st, status::GOOD);
        assert_eq!(cdrom.audio().state(), CdAudioState::Playing);
    }

    #[test]
    fn play_audio_msf_selects_range() {
        let mut cdrom = ready_cdrom();
        // 00:02:16 to 00:03:16 -> LBA 16 to 91.
        let st = cdrom.execute_no_data(&[opcode::PLAY_AUDIO_MSF, 0, 0, 0, 2, 16, 0, 3, 16, 0]);
        assert_eq!(st, status::GOOD);
        let (_, start, end) = cdrom.audio().current_position();
        assert_eq!(start, 16);
        assert_eq!(end, 91);
    }

    #[test]
    fn eject_via_start_stop_honors_prevent() {
        let mut cdrom = ready_cdrom();
        // Prevent removal, then try to eject.
        assert_eq!(
            cdrom.execute_no_data(&[opcode::PREVENT_ALLOW, 0, 0, 0, 1, 0]),
            status::GOOD
        );
        assert_eq!(
            cdrom.execute_no_data(&[opcode::START_STOP, 0, 0, 0, 0x02, 0]),
            status::CHECK_CONDITION
        );
        // Allow removal, eject succeeds and the drive reports not ready.
        assert_eq!(
            cdrom.execute_no_data(&[opcode::PREVENT_ALLOW, 0, 0, 0, 0, 0]),
            status::GOOD
        );
        assert_eq!(
            cdrom.execute_no_data(&[opcode::START_STOP, 0, 0, 0, 0x02, 0]),
            status::GOOD
        );
        let st = cdrom.execute_no_data(&[opcode::TEST_UNIT_READY, 0, 0, 0, 0, 0]);
        assert_eq!(st, status::CHECK_CONDITION);
        let (sense, _) = cdrom.data_in(&[opcode::REQUEST_SENSE, 0, 0, 0, 18, 0]);
        assert_eq!(sense[12], asc::MEDIUM_NOT_PRESENT);
    }

    #[test]
    fn mode_sense_6_reports_medium_and_block_descriptor() {
        let mut cdrom = ready_cdrom();
        let (data, st) = cdrom.data_in(&[opcode::MODE_SENSE6, 0, 0x3F, 0, 255, 0]);
        assert_eq!(st, status::GOOD);
        assert_eq!(data[0] as usize, data.len() - 1);
        assert_eq!(data[1], 0x03); // Mixed data and audio.
        assert_eq!(data[3], 8); // Block descriptor length.
        // Block length 2048 in the descriptor.
        assert_eq!(data[4 + 6], 0x08);
        assert_eq!(data[4 + 7], 0x00);
    }

    #[test]
    fn media_change_after_reinsert_raises_attention() {
        let mut cdrom = ready_cdrom();
        cdrom.eject_media();
        let st = cdrom.execute_no_data(&[opcode::TEST_UNIT_READY, 0, 0, 0, 0, 0]);
        assert_eq!(st, status::CHECK_CONDITION);
        cdrom.data_in(&[opcode::REQUEST_SENSE, 0, 0, 0, 18, 0]);

        cdrom.insert_media(make_mixed_image());
        let st = cdrom.execute_no_data(&[opcode::TEST_UNIT_READY, 0, 0, 0, 0, 0]);
        assert_eq!(st, status::CHECK_CONDITION);
        let (sense, _) = cdrom.data_in(&[opcode::REQUEST_SENSE, 0, 0, 0, 18, 0]);
        assert_eq!(sense[2], sense_key::UNIT_ATTENTION);
        let st = cdrom.execute_no_data(&[opcode::TEST_UNIT_READY, 0, 0, 0, 0, 0]);
        assert_eq!(st, status::GOOD);
    }

    #[test]
    fn generate_audio_produces_samples_while_playing() {
        let mut cdrom = ready_cdrom();
        cdrom.execute_no_data(&[opcode::PLAY_AUDIO10, 0, 0, 0, 0, 16, 0, 0, 75, 0]);
        let mut output = vec![0f32; 512];
        cdrom.generate_audio_samples([1.0, 1.0], &mut output);
        assert!(output.iter().any(|sample| *sample != 0.0));
    }
}
