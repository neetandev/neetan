//! MSX CAS waveform conversion.

use core::fmt;

use device::cassette::SampledSignal;

/// Marker introducing every fMSX CAS block.
const CAS_MARKER: [u8; 8] = [0x1F, 0xA6, 0xDE, 0xBA, 0xCC, 0x13, 0x7D, 0x74];
/// ASCII file-type header.
const ASCII_HEADER: [u8; 10] = [0xEA; 10];
/// Binary file-type header.
const BINARY_HEADER: [u8; 10] = [0xD0; 10];
/// Tokenized BASIC file-type header.
const BASIC_HEADER: [u8; 10] = [0xD3; 10];
/// Waveform sample rate in Hz.
const SIGNAL_RATE: u32 = 14_976;
/// Samples emitted for one serial bit.
const SAMPLES_PER_BIT: usize = 4;
/// Long leader length in serial one bits.
const LONG_LEADER_BITS: usize = 8_000;
/// Short leader length in serial one bits.
const SHORT_LEADER_BITS: usize = 2_000;
/// Silence before a long leader in seconds.
const LONG_GAP_SECONDS: usize = 2;
/// Silence before a short leader in seconds.
const SHORT_GAP_SECONDS: usize = 1;

/// Failure while parsing an MSX cassette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MsxCassetteError {
    /// The cassette image is empty.
    Empty,
    /// The file is not an MSX CAS image.
    MissingMarker,
    /// The file extension is unsupported.
    UnknownFormat(String),
    /// Bytes occur where an fMSX block marker is required.
    UnexpectedData {
        /// Byte offset of the unexpected data.
        offset: usize,
    },
    /// A binary or BASIC header has no following data block.
    TruncatedBlock {
        /// Byte offset of the file-type header.
        offset: usize,
    },
}

impl fmt::Display for MsxCassetteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("the cassette image is empty"),
            Self::MissingMarker => formatter.write_str("the cassette has no MSX CAS marker"),
            Self::UnknownFormat(extension) => {
                write!(
                    formatter,
                    "unsupported MSX cassette format \".{extension}\""
                )
            }
            Self::UnexpectedData { offset } => {
                write!(formatter, "unexpected cassette data at byte {offset}")
            }
            Self::TruncatedBlock { offset } => {
                write!(formatter, "cassette block at byte {offset} is truncated")
            }
        }
    }
}

impl std::error::Error for MsxCassetteError {}

/// Converts an MSX `.cas` byte stream into an EAR waveform.
pub fn load_msx_cassette(extension: &str, bytes: &[u8]) -> Result<SampledSignal, MsxCassetteError> {
    if !extension.eq_ignore_ascii_case("cas") {
        return Err(MsxCassetteError::UnknownFormat(extension.to_owned()));
    }
    if bytes.is_empty() {
        return Err(MsxCassetteError::Empty);
    }
    if !bytes.starts_with(&CAS_MARKER) {
        return Err(MsxCassetteError::MissingMarker);
    }

    let mut builder = SignalBuilder::default();
    let mut position = 0;
    while position < bytes.len() {
        if !bytes[position..].starts_with(&CAS_MARKER) {
            return Err(MsxCassetteError::UnexpectedData { offset: position });
        }
        position += CAS_MARKER.len();
        builder.append_leader(true);
        let header_offset = position;
        let file_type = bytes.get(position..position + ASCII_HEADER.len());
        if file_type == Some(ASCII_HEADER.as_slice()) {
            let mut end_of_file = builder.append_data(bytes, &mut position);
            while !end_of_file && bytes[position..].starts_with(&CAS_MARKER) {
                position += CAS_MARKER.len();
                builder.append_leader(false);
                end_of_file = builder.append_data(bytes, &mut position);
            }
        } else if matches!(
            file_type,
            Some(header) if header == BINARY_HEADER || header == BASIC_HEADER
        ) {
            builder.append_data(bytes, &mut position);
            if !bytes[position..].starts_with(&CAS_MARKER) {
                return Err(MsxCassetteError::TruncatedBlock {
                    offset: header_offset,
                });
            }
            position += CAS_MARKER.len();
            builder.append_leader(false);
            builder.append_data(bytes, &mut position);
        } else {
            builder.append_data(bytes, &mut position);
        }
    }
    Ok(builder.finish())
}

#[derive(Default)]
struct SignalBuilder {
    samples: Vec<u8>,
    bit_count: usize,
}

impl SignalBuilder {
    fn append_leader(&mut self, long: bool) {
        let gap_seconds = if long {
            LONG_GAP_SECONDS
        } else {
            SHORT_GAP_SECONDS
        };
        self.append_level(false, SIGNAL_RATE as usize * gap_seconds);
        let leader_bits = if long {
            LONG_LEADER_BITS
        } else {
            SHORT_LEADER_BITS
        };
        for _ in 0..leader_bits {
            self.append_serial_bit(true);
        }
    }

    fn append_data(&mut self, bytes: &[u8], position: &mut usize) -> bool {
        let mut end_of_file = false;
        while *position < bytes.len() && !bytes[*position..].starts_with(&CAS_MARKER) {
            let value = bytes[*position];
            self.append_byte(value);
            end_of_file |= value == 0x1A;
            *position += 1;
        }
        end_of_file
    }

    fn append_level(&mut self, high: bool, count: usize) {
        for _ in 0..count {
            self.append_sample(high);
        }
    }

    fn append_serial_bit(&mut self, one: bool) {
        let levels = if one {
            [true, false, true, false]
        } else {
            [true, true, false, false]
        };
        debug_assert_eq!(levels.len(), SAMPLES_PER_BIT);
        for level in levels {
            self.append_sample(level);
        }
    }

    fn append_byte(&mut self, value: u8) {
        self.append_serial_bit(false);
        for bit in 0..8 {
            self.append_serial_bit(value & (1 << bit) != 0);
        }
        self.append_serial_bit(true);
        self.append_serial_bit(true);
    }

    fn append_sample(&mut self, high: bool) {
        if self.bit_count & 7 == 0 {
            self.samples.push(0);
        }
        if high {
            let index = self.samples.len() - 1;
            self.samples[index] |= 1 << (7 - (self.bit_count & 7));
        }
        self.bit_count += 1;
    }

    fn finish(self) -> SampledSignal {
        SampledSignal {
            sample_rate: SIGNAL_RATE,
            samples: self.samples,
            bit_count: self.bit_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_block_has_gap_leader_and_serial_data() {
        let mut image = CAS_MARKER.to_vec();
        image.push(1);
        let signal = load_msx_cassette("cas", &image).unwrap();
        let gap = SIGNAL_RATE as usize * LONG_GAP_SECONDS;
        assert!(!signal.level_at(gap - 1));
        assert!(signal.level_at(gap));
        assert!(!signal.level_at(gap + 1));
        let data = gap + LONG_LEADER_BITS * SAMPLES_PER_BIT;
        assert!(signal.level_at(data));
        assert!(signal.level_at(data + 1));
        assert!(!signal.level_at(data + 2));
        assert!(!signal.level_at(data + 3));
    }

    #[test]
    fn rejects_empty_unknown_and_unmarked_images() {
        assert_eq!(
            load_msx_cassette("cas", &[]).unwrap_err(),
            MsxCassetteError::Empty
        );
        assert_eq!(
            load_msx_cassette("wav", &[1]).unwrap_err(),
            MsxCassetteError::UnknownFormat("wav".to_owned())
        );
        assert_eq!(
            load_msx_cassette("cas", &[1]).unwrap_err(),
            MsxCassetteError::MissingMarker
        );
    }

    #[test]
    fn binary_files_require_and_use_a_short_second_block() {
        let mut image = CAS_MARKER.to_vec();
        image.extend_from_slice(&BINARY_HEADER);
        image.push(1);
        assert!(matches!(
            load_msx_cassette("cas", &image),
            Err(MsxCassetteError::TruncatedBlock { .. })
        ));
        image.extend_from_slice(&CAS_MARKER);
        image.push(2);
        let signal = load_msx_cassette("cas", &image).unwrap();
        let long_size =
            SIGNAL_RATE as usize * LONG_GAP_SECONDS + LONG_LEADER_BITS * SAMPLES_PER_BIT;
        let first_data_size = (BINARY_HEADER.len() + 1) * 11 * SAMPLES_PER_BIT;
        let short_start = long_size + first_data_size;
        assert!(!signal.level_at(short_start));
        let second_data = short_start
            + SIGNAL_RATE as usize * SHORT_GAP_SECONDS
            + SHORT_LEADER_BITS * SAMPLES_PER_BIT;
        assert!(signal.level_at(second_data));
    }

    #[test]
    fn ascii_end_marker_starts_the_next_file_with_a_long_leader() {
        let mut image = CAS_MARKER.to_vec();
        image.extend_from_slice(&ASCII_HEADER);
        image.push(0x1A);
        image.extend_from_slice(&CAS_MARKER);
        image.extend_from_slice(&BASIC_HEADER);
        image.extend_from_slice(&CAS_MARKER);
        image.push(0);
        let signal = load_msx_cassette("cas", &image).unwrap();
        assert!(signal.bit_count > 2 * SIGNAL_RATE as usize * LONG_GAP_SECONDS);
    }

    #[test]
    fn arbitrary_data_blocks_are_preserved() {
        let mut image = CAS_MARKER.to_vec();
        image.extend_from_slice(&ASCII_HEADER);
        image.push(0x1A);
        image.push(0x55);
        assert!(load_msx_cassette("cas", &image).is_ok());

        let mut invalid = CAS_MARKER.to_vec();
        invalid.push(1);
        invalid.extend_from_slice(&CAS_MARKER);
        invalid.push(2);
        invalid.push(3);
        assert!(load_msx_cassette("cas", &invalid).is_ok());
    }
}
