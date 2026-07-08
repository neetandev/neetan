//! Sharp X1 `.tap` waveform cassette parser.
//!
//! An X1 tape image is a raw playback waveform: one bit per sample at a fixed
//! sample rate. Each bit drives the EAR line (PPI port B bit 1) that the main
//! CPU demodulates in software.
//!
//! Two layouts exist. The "new" layout opens with the ASCII magic `TAPE` and a
//! header that carries the sample rate and the bit count; the "old" layout is
//! just a 4-byte sample rate followed by the sample bytes. In both, each sample
//! byte holds eight consecutive bits, most-significant bit first.

use super::{CassetteError, SampledSignal};

/// Magic that identifies the new header layout.
const NEW_HEADER_MAGIC: [u8; 4] = *b"TAPE";
/// Offset of the sample data in the new layout.
const NEW_DATA_OFFSET: usize = 0x28;
/// Offset of the sample-rate field in the new layout.
const NEW_SAMPLE_RATE_OFFSET: usize = 0x1C;
/// Offset of the bit-count field in the new layout.
const NEW_BIT_COUNT_OFFSET: usize = 0x20;
/// Offset of the sample data in the old layout (past the 4-byte sample rate).
const OLD_DATA_OFFSET: usize = 0x04;
/// Sample rate assumed when the header records zero.
const DEFAULT_SAMPLE_RATE: u32 = 8000;

pub(super) fn parse(data: &[u8]) -> Result<SampledSignal, CassetteError> {
    if data.len() >= NEW_HEADER_MAGIC.len() && data[..NEW_HEADER_MAGIC.len()] == NEW_HEADER_MAGIC {
        parse_new(data)
    } else {
        parse_old(data)
    }
}

fn read_u32le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn rate_or_default(rate: u32) -> u32 {
    if rate == 0 { DEFAULT_SAMPLE_RATE } else { rate }
}

fn parse_new(data: &[u8]) -> Result<SampledSignal, CassetteError> {
    if data.len() <= NEW_DATA_OFFSET {
        return Err(CassetteError::Empty);
    }
    let sample_rate = rate_or_default(read_u32le(data, NEW_SAMPLE_RATE_OFFSET));
    let declared_bits = read_u32le(data, NEW_BIT_COUNT_OFFSET) as usize;
    let samples = data[NEW_DATA_OFFSET..].to_vec();
    let bit_count = declared_bits.min(samples.len() * 8);
    Ok(SampledSignal {
        sample_rate,
        samples,
        bit_count,
    })
}

fn parse_old(data: &[u8]) -> Result<SampledSignal, CassetteError> {
    if data.len() <= OLD_DATA_OFFSET {
        return Err(CassetteError::Empty);
    }
    let sample_rate = rate_or_default(read_u32le(data, 0));
    let samples = data[OLD_DATA_OFFSET..].to_vec();
    let bit_count = samples.len() * 8;
    Ok(SampledSignal {
        sample_rate,
        samples,
        bit_count,
    })
}
