//! Raw byte-stream cassette parser for `.cas` and `.p6` images.
//!
//! These formats store the already-demodulated tape bytes verbatim, including
//! the lead-in sync run, the file header and the data. The whole image is fed
//! to the deck in order as a single block.

use super::NormalizedTape;

pub(super) fn parse(data: &[u8]) -> NormalizedTape {
    NormalizedTape::from_raw(data.to_vec())
}
