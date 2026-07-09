//! Reusable cassette deck.
//!
//! Models a cassette transport that yields demodulated data bytes to a host
//! sub-controller. A loaded tape is held as a normalized model - an ordered
//! list of data blocks - that decouples the on-disk image format from the
//! consumption path.

mod cas;
mod p6t;
mod t77;
mod tap;

use std::fmt;

/// Baud rate assumed for raw images that carry no rate metadata.
const DEFAULT_BAUD: u32 = 1200;

/// One logical block of tape data with its framing-derived baud rate.
#[derive(Debug, Clone)]
pub struct TapeBlock {
    /// Baud rate the block was recorded at (typically 600 or 1200).
    pub baud: u32,
    /// The demodulated data bytes of the block.
    pub bytes: Vec<u8>,
}

/// A tape image reduced to an ordered list of data blocks.
#[derive(Debug, Clone, Default)]
pub struct NormalizedTape {
    /// The data blocks in playback order.
    pub blocks: Vec<TapeBlock>,
}

impl NormalizedTape {
    /// Builds a tape from a single raw byte stream.
    fn from_raw(bytes: Vec<u8>) -> Self {
        Self {
            blocks: vec![TapeBlock {
                baud: DEFAULT_BAUD,
                bytes,
            }],
        }
    }

    /// Total number of data bytes across all blocks.
    pub fn len(&self) -> usize {
        self.blocks.iter().map(|block| block.bytes.len()).sum()
    }

    /// Whether the tape holds no data.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The byte at flattened position `index`, if any.
    fn byte_at(&self, index: usize) -> Option<u8> {
        let mut remaining = index;
        for block in &self.blocks {
            if remaining < block.bytes.len() {
                return Some(block.bytes[remaining]);
            }
            remaining -= block.bytes.len();
        }
        None
    }
}

/// A bit-level playback waveform: one bit per sample at `sample_rate`, packed
/// eight bits to a byte with the most-significant bit first. Each bit is the
/// EAR line level the deck presents while the tape moves under the head.
#[derive(Debug, Clone, Default)]
pub struct SampledSignal {
    /// Playback sample rate in Hz.
    pub sample_rate: u32,
    /// Packed sample bits.
    pub samples: Vec<u8>,
    /// Number of valid bits in `samples`.
    pub bit_count: usize,
}

impl SampledSignal {
    /// The EAR level of the sample at `index`, or `false` past the end.
    pub fn level_at(&self, index: usize) -> bool {
        if index >= self.bit_count {
            return false;
        }
        (self.samples[index >> 3] >> (7 - (index & 7))) & 1 != 0
    }

    /// The first sample index at or after `from` that ends a silent run of at
    /// least `gap` samples with no level change; the signal length if none.
    fn next_gap(&self, from: usize, gap: usize) -> usize {
        if gap == 0 || self.bit_count == 0 {
            return self.bit_count;
        }
        let mut run_start = from.min(self.bit_count);
        let mut index = run_start;
        while index < self.bit_count {
            if self.level_at(index) != self.level_at(run_start) {
                run_start = index;
            } else if index - run_start + 1 >= gap {
                let mut end = index;
                while end < self.bit_count && self.level_at(end) == self.level_at(run_start) {
                    end += 1;
                }
                return end;
            }
            index += 1;
        }
        self.bit_count
    }

    /// The last sample index at or before `from` that begins a silent run of at
    /// least `gap` samples with no level change; zero if none.
    fn previous_gap(&self, from: usize, gap: usize) -> usize {
        if gap == 0 || self.bit_count == 0 {
            return 0;
        }
        let mut run_end = from.min(self.bit_count.saturating_sub(1));
        let mut index = run_end;
        loop {
            if self.level_at(index) != self.level_at(run_end) {
                run_end = index;
            } else if run_end - index + 1 >= gap {
                let mut start = index;
                while start > 0 && self.level_at(start - 1) == self.level_at(run_end) {
                    start -= 1;
                }
                return start;
            }
            if index == 0 {
                break;
            }
            index -= 1;
        }
        0
    }
}

/// A parsed cassette image in its natural representation: demodulated bytes for
/// the PC-6000 formats, or a raw playback waveform for the X1 `.tap` format.
#[derive(Debug, Clone)]
pub enum CassetteMedia {
    /// Byte-oriented tape (PC-6000 `.cas`/`.p6`/`.p6t`).
    Bytes(NormalizedTape),
    /// Sample-oriented waveform (X1 `.tap`).
    Samples(SampledSignal),
}

/// Tape media loaded into the deck.
#[derive(Debug, Clone)]
enum Media {
    Bytes(NormalizedTape),
    Samples(SampledSignal),
}

/// Fast-wind speed relative to normal play.
const FAST_WIND_MULTIPLIER: u64 = 20;
/// Silent-run length (in seconds) that APSS treats as an inter-program gap.
const APSS_GAP_SECONDS: u64 = 4;

/// Transport state of the deck.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Transport {
    /// The tape is not moving.
    #[default]
    Stopped,
    /// The tape is moving forward under the read head.
    Playing,
    /// The tape is winding forward at speed (no reading).
    FastForward,
    /// The tape is winding backward at speed (no reading).
    Rewind,
    /// The tape is moving forward under the write head.
    Recording,
}

/// Result of reading the next byte from the deck.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CassetteRead {
    /// A demodulated data byte is available.
    Byte(u8),
    /// The tape is exhausted (or no tape is loaded).
    EndOfTape,
}

/// A cassette transport with a loaded tape image.
///
/// Two consumption paths coexist. The byte path ([`CassetteDeck::read_byte`])
/// serves hosts whose sub-controller yields already-demodulated bytes (the
/// PC-6000). The sample path ([`CassetteDeck::advance`] plus
/// [`CassetteDeck::ear_level`]) serves hosts that read the raw playback
/// waveform bit by bit (the X1). The transport controls are shared.
#[derive(Debug, Clone, Default)]
pub struct CassetteDeck {
    transport: Transport,
    motor: bool,
    byte_position: usize,
    sample_position: u64,
    sample_fraction: u64,
    last_update_cycle: u64,
    media: Option<Media>,
}

impl CassetteDeck {
    /// Creates an empty, stopped deck.
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads a byte-oriented tape, leaving the deck stopped at the start.
    pub fn insert(&mut self, tape: NormalizedTape) {
        self.load(Media::Bytes(tape));
    }

    /// Loads a sample-oriented waveform, leaving the deck stopped at the start.
    pub fn insert_signal(&mut self, signal: SampledSignal) {
        self.load(Media::Samples(signal));
    }

    /// Loads parsed media, dispatching to the matching representation.
    pub fn insert_media(&mut self, media: CassetteMedia) {
        match media {
            CassetteMedia::Bytes(tape) => self.insert(tape),
            CassetteMedia::Samples(signal) => self.insert_signal(signal),
        }
    }

    fn load(&mut self, media: Media) {
        self.media = Some(media);
        self.byte_position = 0;
        self.sample_position = 0;
        self.sample_fraction = 0;
        self.transport = Transport::Stopped;
        self.motor = false;
    }

    /// Removes the loaded tape.
    pub fn eject(&mut self) {
        self.media = None;
        self.byte_position = 0;
        self.sample_position = 0;
        self.sample_fraction = 0;
        self.transport = Transport::Stopped;
        self.motor = false;
    }

    /// Whether a tape is loaded.
    pub fn has_tape(&self) -> bool {
        self.media.is_some()
    }

    /// Whether the tape is moving forward under the read head.
    pub fn is_playing(&self) -> bool {
        self.transport == Transport::Playing
    }

    /// Whether the tape is moving forward (playing, winding, or recording).
    pub fn is_moving_forward(&self) -> bool {
        matches!(
            self.transport,
            Transport::Playing | Transport::FastForward | Transport::Recording
        )
    }

    /// Whether the tape is winding backward.
    pub fn is_rewinding(&self) -> bool {
        self.transport == Transport::Rewind
    }

    /// Whether the tape sits at its start.
    pub fn at_start(&self) -> bool {
        match self.media.as_ref() {
            Some(Media::Bytes(_)) => self.byte_position == 0,
            Some(Media::Samples(_)) => self.sample_position == 0,
            None => false,
        }
    }

    /// Whether the tape has reached (or passed) its end.
    pub fn at_end(&self) -> bool {
        match self.media.as_ref() {
            Some(Media::Bytes(tape)) => self.byte_position >= tape.len(),
            Some(Media::Samples(signal)) => self.sample_position >= signal.bit_count as u64,
            None => false,
        }
    }

    /// Turns the motor on or off. Inserting a tape positions it at the start;
    /// stopping and restarting the motor resumes from the current position.
    pub fn set_motor(&mut self, on: bool) {
        if on && !self.motor {
            self.transport = Transport::Playing;
        } else if !on {
            self.transport = Transport::Stopped;
        }
        self.motor = on;
    }

    /// Starts forward playback under the read head.
    pub fn play(&mut self) {
        if self.has_tape() {
            self.transport = Transport::Playing;
        }
    }

    /// Stops the transport, holding the current position.
    pub fn stop(&mut self) {
        self.transport = Transport::Stopped;
    }

    /// Winds forward at speed without reading.
    pub fn fast_forward(&mut self) {
        if self.has_tape() {
            self.transport = Transport::FastForward;
        }
    }

    /// Winds backward at speed without reading.
    pub fn rewind(&mut self) {
        if self.has_tape() {
            self.transport = Transport::Rewind;
        }
    }

    /// Starts forward motion under the write head.
    pub fn record(&mut self) {
        if self.has_tape() {
            self.transport = Transport::Recording;
        }
    }

    /// Seeks to the next (or previous) inter-program gap, then stops. Only the
    /// sample path carries the waveform APSS scans; the byte path just stops.
    pub fn automatic_program_search(&mut self, forward: bool) {
        if let Some(Media::Samples(signal)) = self.media.as_ref() {
            let gap = (u64::from(signal.sample_rate) * APSS_GAP_SECONDS) as usize;
            let position = self.sample_position as usize;
            let target = if forward {
                signal.next_gap(position, gap)
            } else {
                signal.previous_gap(position, gap)
            };
            self.sample_position = target as u64;
        }
        self.transport = Transport::Stopped;
    }

    /// Reads the next byte under the head, advancing the position. Returns
    /// `EndOfTape` once the tape is exhausted, no tape is loaded, or the loaded
    /// media is a sample waveform rather than a byte stream.
    pub fn read_byte(&mut self) -> CassetteRead {
        let Some(Media::Bytes(tape)) = self.media.as_ref() else {
            return CassetteRead::EndOfTape;
        };
        match tape.byte_at(self.byte_position) {
            Some(byte) => {
                self.byte_position += 1;
                CassetteRead::Byte(byte)
            }
            None => CassetteRead::EndOfTape,
        }
    }

    /// Advances a sample-path tape to `now` (in CPU cycles), moving the head by
    /// the elapsed time scaled to the tape sample rate and transport speed. A
    /// no-op for byte tapes and while stopped, but always tracks `now` so a
    /// later play starts from the current moment rather than jumping.
    pub fn advance(&mut self, now: u64, cpu_clock_hz: u32) {
        let elapsed = now.saturating_sub(self.last_update_cycle);
        self.last_update_cycle = now;

        let Some(Media::Samples(signal)) = self.media.as_ref() else {
            return;
        };
        if cpu_clock_hz == 0 || elapsed == 0 {
            return;
        }
        let (forward, multiplier) = match self.transport {
            Transport::Playing | Transport::Recording => (true, 1),
            Transport::FastForward => (true, FAST_WIND_MULTIPLIER),
            Transport::Rewind => (false, FAST_WIND_MULTIPLIER),
            Transport::Stopped => return,
        };
        let scaled = elapsed
            .saturating_mul(u64::from(signal.sample_rate))
            .saturating_mul(multiplier)
            + self.sample_fraction;
        let steps = scaled / u64::from(cpu_clock_hz);
        self.sample_fraction = scaled % u64::from(cpu_clock_hz);
        let bit_count = signal.bit_count as u64;
        if forward {
            self.sample_position = self.sample_position.saturating_add(steps).min(bit_count);
        } else {
            self.sample_position = self.sample_position.saturating_sub(steps);
        }
    }

    /// The current EAR (playback) level on the sample path; `false` on the byte
    /// path or with no tape.
    pub fn ear_level(&self) -> bool {
        match self.media.as_ref() {
            Some(Media::Samples(signal)) => signal.level_at(self.sample_position as usize),
            _ => false,
        }
    }
}

/// Error returned when a cassette image cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CassetteError {
    /// The image held no data.
    Empty,
    /// The file extension is not a recognised cassette format.
    UnknownFormat(String),
}

impl fmt::Display for CassetteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CassetteError::Empty => write!(formatter, "the cassette image is empty"),
            CassetteError::UnknownFormat(extension) => {
                write!(formatter, "unsupported cassette format \".{extension}\"")
            }
        }
    }
}

impl std::error::Error for CassetteError {}

/// Parses a cassette image into the normalized tape model. The format is chosen
/// by `extension` (case-insensitive, without the leading dot).
pub fn parse_tape(extension: &str, data: &[u8]) -> Result<NormalizedTape, CassetteError> {
    if data.is_empty() {
        return Err(CassetteError::Empty);
    }
    match extension.to_ascii_lowercase().as_str() {
        // `.cas` and `.p6` are identical raw post-demodulation byte streams.
        "cas" | "p6" => Ok(cas::parse(data)),
        "p6t" => Ok(p6t::parse(data)),
        other => Err(CassetteError::UnknownFormat(other.to_string())),
    }
}

/// Parses a cassette image into its natural media representation. Byte formats
/// yield [`CassetteMedia::Bytes`]; the X1 `.tap` waveform yields
/// [`CassetteMedia::Samples`]. The format is chosen by `extension`
/// (case-insensitive, without the leading dot).
pub fn load_cassette(extension: &str, data: &[u8]) -> Result<CassetteMedia, CassetteError> {
    if data.is_empty() {
        return Err(CassetteError::Empty);
    }
    match extension.to_ascii_lowercase().as_str() {
        "cas" | "p6" => Ok(CassetteMedia::Bytes(cas::parse(data))),
        "p6t" => Ok(CassetteMedia::Bytes(p6t::parse(data))),
        "tap" => Ok(CassetteMedia::Samples(tap::parse(data)?)),
        "t77" => Ok(CassetteMedia::Samples(t77::parse(data)?)),
        other => Err(CassetteError::UnknownFormat(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_image_is_one_block_of_verbatim_bytes() {
        let tape = parse_tape("p6", &[0xD3, 0xD3, 0x69, 0x6A]).expect("parses");
        assert_eq!(tape.blocks.len(), 1);
        assert_eq!(tape.blocks[0].bytes, vec![0xD3, 0xD3, 0x69, 0x6A]);
        assert_eq!(tape.len(), 4);
    }

    #[test]
    fn empty_image_is_rejected() {
        assert_eq!(parse_tape("cas", &[]).unwrap_err(), CassetteError::Empty);
    }

    #[test]
    fn unknown_extension_is_rejected() {
        assert_eq!(
            parse_tape("wav", &[0x00]).unwrap_err(),
            CassetteError::UnknownFormat("wav".to_string())
        );
    }

    #[test]
    fn deck_walks_bytes_then_ends() {
        let mut deck = CassetteDeck::new();
        deck.insert(NormalizedTape::from_raw(vec![0x01, 0x02]));
        deck.set_motor(true);
        assert_eq!(deck.read_byte(), CassetteRead::Byte(0x01));
        assert_eq!(deck.read_byte(), CassetteRead::Byte(0x02));
        assert_eq!(deck.read_byte(), CassetteRead::EndOfTape);
    }

    #[test]
    fn restarting_the_motor_resumes() {
        let mut deck = CassetteDeck::new();
        deck.insert(NormalizedTape::from_raw(vec![0xAA, 0xBB]));
        deck.set_motor(true);
        assert_eq!(deck.read_byte(), CassetteRead::Byte(0xAA));
        deck.set_motor(false);
        deck.set_motor(true);
        assert_eq!(deck.read_byte(), CassetteRead::Byte(0xBB));
    }

    #[test]
    fn empty_deck_reads_end_of_tape() {
        let mut deck = CassetteDeck::new();
        assert_eq!(deck.read_byte(), CassetteRead::EndOfTape);
        assert!(!deck.has_tape());
    }

    fn old_tap(sample_rate: u32, samples: &[u8]) -> Vec<u8> {
        let mut image = sample_rate.to_le_bytes().to_vec();
        image.extend_from_slice(samples);
        image
    }

    #[test]
    fn old_tap_maps_bytes_to_msb_first_bits() {
        let media = load_cassette("tap", &old_tap(8000, &[0b1010_0000])).expect("parses");
        let CassetteMedia::Samples(signal) = media else {
            panic!("expected a sample waveform");
        };
        assert_eq!(signal.sample_rate, 8000);
        assert_eq!(signal.bit_count, 8);
        assert!(signal.level_at(0));
        assert!(!signal.level_at(1));
        assert!(signal.level_at(2));
        assert!(!signal.level_at(3));
    }

    #[test]
    fn new_tap_uses_declared_rate_and_bit_count() {
        let mut image = b"TAPE".to_vec();
        image.resize(0x28, 0);
        image[0x1C..0x20].copy_from_slice(&4000u32.to_le_bytes());
        image[0x20..0x24].copy_from_slice(&4u32.to_le_bytes());
        image.push(0b1100_0000);
        let CassetteMedia::Samples(signal) = load_cassette("tap", &image).expect("parses") else {
            panic!("expected a sample waveform");
        };
        assert_eq!(signal.sample_rate, 4000);
        assert_eq!(signal.bit_count, 4);
        assert!(signal.level_at(0));
        assert!(signal.level_at(1));
        assert!(!signal.level_at(2));
    }

    #[test]
    fn playback_advances_the_head_at_the_sample_rate() {
        // Two samples per second at 2 Hz: bit 0 high, bit 1 low.
        let mut deck = CassetteDeck::new();
        deck.insert_signal(SampledSignal {
            sample_rate: 2,
            samples: vec![0b1000_0000],
            bit_count: 2,
        });
        deck.play();
        deck.advance(0, 4);
        assert!(deck.ear_level()); // sample 0
        deck.advance(2, 4); // half a second at 4 Hz clock -> one sample
        assert!(!deck.ear_level()); // sample 1
        deck.advance(4, 4);
        assert!(deck.at_end());
    }

    #[test]
    fn a_sample_tape_yields_no_bytes() {
        let mut deck = CassetteDeck::new();
        deck.insert_signal(SampledSignal {
            sample_rate: 8000,
            samples: vec![0xFF],
            bit_count: 8,
        });
        deck.play();
        assert_eq!(deck.read_byte(), CassetteRead::EndOfTape);
    }

    #[test]
    fn stopped_deck_holds_its_position() {
        let mut deck = CassetteDeck::new();
        deck.insert_signal(SampledSignal {
            sample_rate: 2,
            samples: vec![0b0100_0000],
            bit_count: 2,
        });
        // Never played: advancing tracks time but never moves the head.
        deck.advance(1_000, 4);
        assert!(!deck.ear_level()); // still sample 0 (low)
        deck.play();
        deck.advance(1_002, 4); // one sample later
        assert!(deck.ear_level()); // sample 1 (high)
    }

    #[test]
    fn apss_seeks_past_a_silent_gap() {
        // A short lead-in, a 4-sample all-low gap, then a high marker.
        let mut samples = vec![0b1000_0000];
        // bit 0 high; bits 1..=4 low (the gap); bit 5 high.
        samples[0] = 0b1000_0100;
        let signal = SampledSignal {
            sample_rate: 1,
            samples,
            bit_count: 6,
        };
        let mut deck = CassetteDeck::new();
        deck.insert_signal(signal);
        deck.play();
        deck.automatic_program_search(true);
        assert_eq!(deck.read_byte(), CassetteRead::EndOfTape); // sample media
        assert!(deck.ear_level()); // positioned on the high marker after the gap
    }

    fn t77_image(records: &[u16]) -> Vec<u8> {
        let mut image = b"XM7 TAPE IMAGE 0".to_vec();
        for record in records {
            image.extend_from_slice(&record.to_be_bytes());
        }
        image
    }

    #[test]
    fn t77_expands_level_and_count_records() {
        // One high tick, two low ticks, one high tick.
        let image = t77_image(&[0x8001, 0x0002, 0x8001]);
        let CassetteMedia::Samples(signal) = load_cassette("t77", &image).expect("parses") else {
            panic!("expected a sample waveform");
        };
        assert_eq!(signal.sample_rate, 111_111);
        assert_eq!(signal.bit_count, 4);
        assert!(signal.level_at(0));
        assert!(!signal.level_at(1));
        assert!(!signal.level_at(2));
        assert!(signal.level_at(3));
    }

    #[test]
    fn t77_skips_zero_width_records() {
        let image = t77_image(&[0x8000, 0x0003]);
        let CassetteMedia::Samples(signal) = load_cassette("t77", &image).expect("parses") else {
            panic!("expected a sample waveform");
        };
        assert_eq!(signal.bit_count, 3);
        assert!(!signal.level_at(0));
    }

    #[test]
    fn t77_rejects_a_bad_header() {
        let mut image = b"NOT A TAPE IMAGE".to_vec();
        image.extend_from_slice(&0x8004u16.to_be_bytes());
        assert_eq!(
            load_cassette("t77", &image).unwrap_err(),
            CassetteError::UnknownFormat("t77".to_string())
        );
    }

    #[test]
    fn flattened_byte_lookup_spans_blocks() {
        let tape = NormalizedTape {
            blocks: vec![
                TapeBlock {
                    baud: 1200,
                    bytes: vec![0x10, 0x11],
                },
                TapeBlock {
                    baud: 1200,
                    bytes: vec![0x20],
                },
            ],
        };
        assert_eq!(tape.byte_at(1), Some(0x11));
        assert_eq!(tape.byte_at(2), Some(0x20));
        assert_eq!(tape.byte_at(3), None);
    }
}
