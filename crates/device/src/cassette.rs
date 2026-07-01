//! Reusable cassette deck.
//!
//! Models a cassette transport that yields demodulated data bytes to a host
//! sub-controller. A loaded tape is held as a normalized model - an ordered
//! list of data blocks - that decouples the on-disk image format from the
//! consumption path.

mod cas;
mod p6t;

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

/// Transport state of the deck.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Transport {
    /// The tape is not moving.
    #[default]
    Stopped,
    /// The tape is moving forward under the read head.
    Playing,
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
#[derive(Debug, Clone, Default)]
pub struct CassetteDeck {
    transport: Transport,
    motor: bool,
    position: usize,
    tape: Option<NormalizedTape>,
}

impl CassetteDeck {
    /// Creates an empty, stopped deck.
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads a tape, leaving the deck stopped at the start.
    pub fn insert(&mut self, tape: NormalizedTape) {
        self.tape = Some(tape);
        self.position = 0;
        self.transport = Transport::Stopped;
        self.motor = false;
    }

    /// Removes the loaded tape.
    pub fn eject(&mut self) {
        self.tape = None;
        self.position = 0;
        self.transport = Transport::Stopped;
        self.motor = false;
    }

    /// Whether a tape is loaded.
    pub fn has_tape(&self) -> bool {
        self.tape.is_some()
    }

    /// Whether the tape is moving.
    pub fn is_playing(&self) -> bool {
        self.transport == Transport::Playing
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

    /// Reads the next byte under the head, advancing the position. Returns
    /// `EndOfTape` once the tape is exhausted or when no tape is loaded.
    pub fn read_byte(&mut self) -> CassetteRead {
        let Some(tape) = self.tape.as_ref() else {
            return CassetteRead::EndOfTape;
        };
        match tape.byte_at(self.position) {
            Some(byte) => {
                self.position += 1;
                CassetteRead::Byte(byte)
            }
            None => CassetteRead::EndOfTape,
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
