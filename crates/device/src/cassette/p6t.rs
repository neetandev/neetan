//! Tagged-container cassette parser for `.p6t` images.
//!
//! A `.p6t` file stores the demodulated tape bytes followed by a tag table that
//! describes how the bytes split into blocks and the baud rate of each. The
//! table opens with a `P6` magic and version, then one record per block giving
//! the block's baud rate and its `(offset, length)` slice into the leading data
//! region.
//!
//! The table is best-effort metadata: if the magic is absent or any record is
//! inconsistent with the data region, the whole image is treated as one raw
//! block so the tape still plays.

use super::{NormalizedTape, TapeBlock};

/// Signature that introduces the tag table: `P6` then version `2.2` and three
/// reserved zero bytes.
const TABLE_MAGIC: [u8; 7] = [b'P', b'6', 0x02, 0x02, 0x00, 0x00, 0x00];
/// Bytes of header that follow the magic before the first record.
const HEADER_LEN: usize = TABLE_MAGIC.len() + 4;
/// A block record opens with `TI`.
const RECORD_MAGIC: [u8; 2] = [b'T', b'I'];
/// Size of one block record.
const RECORD_LEN: usize = 33;
/// Byte offsets of the fields within a record.
const RECORD_BAUD: usize = 19;
const RECORD_OFFSET: usize = 25;
const RECORD_LENGTH: usize = 29;

pub(super) fn parse(data: &[u8]) -> NormalizedTape {
    parse_tagged(data).unwrap_or_else(|| NormalizedTape::from_raw(data.to_vec()))
}

/// Parses the tag table into blocks, or returns `None` to fall back to raw.
fn parse_tagged(data: &[u8]) -> Option<NormalizedTape> {
    let table_start = find_magic(data)?;
    let region = &data[..table_start];

    let mut blocks = Vec::new();
    let mut cursor = table_start + HEADER_LEN;
    while data.get(cursor..cursor + RECORD_MAGIC.len()) == Some(&RECORD_MAGIC) {
        let record = data.get(cursor..cursor + RECORD_LEN)?;
        let baud = u32::from(read_u16(record, RECORD_BAUD));
        let offset = read_u32(record, RECORD_OFFSET) as usize;
        let length = read_u32(record, RECORD_LENGTH) as usize;

        let bytes = region.get(offset..offset + length)?;
        if !bytes.is_empty() {
            blocks.push(TapeBlock {
                baud,
                bytes: bytes.to_vec(),
            });
        }
        cursor += RECORD_LEN;
    }

    if blocks.is_empty() {
        return None;
    }
    Some(NormalizedTape { blocks })
}

fn find_magic(data: &[u8]) -> Option<usize> {
    data.windows(TABLE_MAGIC.len())
        .position(|window| window == TABLE_MAGIC)
}

fn read_u16(record: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([record[at], record[at + 1]])
}

fn read_u32(record: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([record[at], record[at + 1], record[at + 2], record[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(baud: u16, offset: u32, length: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; RECORD_LEN];
        bytes[0] = b'T';
        bytes[1] = b'I';
        bytes[RECORD_BAUD..RECORD_BAUD + 2].copy_from_slice(&baud.to_le_bytes());
        bytes[RECORD_OFFSET..RECORD_OFFSET + 4].copy_from_slice(&offset.to_le_bytes());
        bytes[RECORD_LENGTH..RECORD_LENGTH + 4].copy_from_slice(&length.to_le_bytes());
        bytes
    }

    #[test]
    fn tag_table_splits_into_blocks() {
        let mut image = Vec::new();
        // Data region: a 4-byte header block then a 3-byte data block.
        image.extend_from_slice(&[0xD3, 0xD3, 0x41, 0x42, 0x10, 0x20, 0x30]);
        image.extend_from_slice(&TABLE_MAGIC);
        image.extend_from_slice(&[0, 0, 0, 0]); // header tail
        image.extend_from_slice(&record(600, 0, 4));
        image.extend_from_slice(&record(1200, 4, 3));
        image.extend_from_slice(&7u32.to_le_bytes()); // trailer

        let tape = parse(&image);
        assert_eq!(tape.blocks.len(), 2);
        assert_eq!(tape.blocks[0].baud, 600);
        assert_eq!(tape.blocks[0].bytes, vec![0xD3, 0xD3, 0x41, 0x42]);
        assert_eq!(tape.blocks[1].baud, 1200);
        assert_eq!(tape.blocks[1].bytes, vec![0x10, 0x20, 0x30]);
    }

    #[test]
    fn missing_table_falls_back_to_raw() {
        let tape = parse(&[0x01, 0x02, 0x03]);
        assert_eq!(tape.blocks.len(), 1);
        assert_eq!(tape.blocks[0].bytes, vec![0x01, 0x02, 0x03]);
    }

    #[test]
    fn out_of_range_record_falls_back_to_raw() {
        let mut image = vec![0xAA, 0xBB];
        image.extend_from_slice(&TABLE_MAGIC);
        image.extend_from_slice(&[0, 0, 0, 0]);
        image.extend_from_slice(&record(1200, 0, 999)); // length past the region
        let tape = parse(&image);
        assert_eq!(tape.blocks.len(), 1);
        assert_eq!(tape.blocks[0].bytes.len(), image.len());
    }
}
