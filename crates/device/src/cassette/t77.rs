//! Fujitsu FM-7 `.t77` waveform cassette parser.
//!
//! A T77 image (the XM7 emulator's tape format) is a run-length encoded
//! playback waveform. After a fixed ASCII header it stores a sequence of 16-bit
//! big-endian records: the top bit is the EAR signal level and the low 15 bits
//! are a pulse width counting how many sample ticks hold that level. One tick is
//! nine microseconds, so the expanded waveform plays back at 111111 Hz. Each
//! expanded tick becomes one bit that drives the FM-7 cassette EAR input
//! (`0xFD02` bit 7).

use super::{CassetteError, SampledSignal};

/// ASCII header that identifies a T77 image.
const HEADER_MAGIC: [u8; 16] = *b"XM7 TAPE IMAGE 0";
/// Playback sample rate: one tick every nine microseconds (1_000_000 / 9).
const SAMPLE_RATE_HZ: u32 = 111_111;
/// Bit of a record word carrying the EAR signal level.
const RECORD_LEVEL_BIT: u16 = 0x8000;
/// Mask selecting the pulse-width tick count of a record word.
const RECORD_COUNT_MASK: u16 = 0x7FFF;
/// Most-significant bit position within a packed sample byte.
const PACKED_BIT_HIGH: u32 = 7;

/// Parses a `.t77` image into a playback waveform.
pub(super) fn parse(data: &[u8]) -> Result<SampledSignal, CassetteError> {
    if data.len() < HEADER_MAGIC.len() {
        return Err(CassetteError::Empty);
    }
    if data[..HEADER_MAGIC.len()] != HEADER_MAGIC {
        return Err(CassetteError::UnknownFormat("t77".to_string()));
    }

    let mut waveform = WaveformBuilder::default();
    for record in data[HEADER_MAGIC.len()..].as_chunks::<2>().0 {
        let word = u16::from_be_bytes(*record);
        let count = usize::from(word & RECORD_COUNT_MASK);
        if count == 0 {
            continue;
        }
        let level = word & RECORD_LEVEL_BIT != 0;
        waveform.push_run(level, count);
    }

    Ok(waveform.finish(SAMPLE_RATE_HZ))
}

/// Accumulates EAR levels into a packed, most-significant-bit-first bitstream.
#[derive(Default)]
struct WaveformBuilder {
    samples: Vec<u8>,
    bit_count: usize,
}

impl WaveformBuilder {
    /// Appends `count` samples that all hold `level`.
    fn push_run(&mut self, level: bool, count: usize) {
        for _ in 0..count {
            if self.bit_count.is_multiple_of(8) {
                self.samples.push(0);
            }
            if level {
                let byte = self.samples.last_mut().expect("byte was just pushed");
                *byte |= 1 << (PACKED_BIT_HIGH - (self.bit_count as u32 % 8));
            }
            self.bit_count += 1;
        }
    }

    /// Finalizes the accumulated waveform at `sample_rate`.
    fn finish(self, sample_rate: u32) -> SampledSignal {
        SampledSignal {
            sample_rate,
            samples: self.samples,
            bit_count: self.bit_count,
        }
    }
}
