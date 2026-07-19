//! Span-based UTF-8 string operations and the heap-side access cursors.
//!
//! R7RS-small section 6.7 explicitly permits `string-ref` and `string-set!`
//! to take other than constant time, which makes a UTF-8 backing spec-legal.
//! Indexed access stays cheap anyway. All-ASCII contents are indexed
//! directly. Non-ASCII contents are located by walking from the nearest of
//! the string start, the string end, or a cached cursor remembering the last
//! resolved position, so ascending and descending index loops are amortized
//! constant time.
//!
//! String payloads live as spans in the heap's byte arena, so everything
//! here is a free function over the span's bytes. Every offset, including
//! the cached cursor positions, is relative to the span start: spans only
//! move at a collection and every collection clears the cursors, so a
//! cached position can never outlive the span layout it was resolved in.
//!
//! The cursors live beside the heap in [`StringCursors`] rather than inside
//! each string, and two entries let a loop that walks one string while
//! writing another keep both positions warm.

use std::cell::Cell;

use crate::value::GcRef;

/// A resolved (char index, byte offset) position inside one string span.
pub(crate) type CursorHint = (usize, usize);

/// Decodes the char whose encoding starts at `offset`. Constant time: the
/// lead byte bounds the encoded width, so validation never scans past one
/// char. Returns `None` when `offset` is out of range or not a char
/// boundary of valid UTF-8.
pub(crate) fn decode_char_at(bytes: &[u8], offset: usize) -> Option<char> {
    let lead = *bytes.get(offset)?;
    let width = match lead {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    };
    let encoded = bytes.get(offset..offset + width)?;
    std::str::from_utf8(encoded).ok()?.chars().next()
}

/// Returns the char at a char index together with the position to cache,
/// or `None` when out of range. `bytes` is the string's span, `chars` its
/// char count, and `hint` a previously resolved position within this exact
/// span contents. Multibyte-contents path, kept out of the VM's inline
/// fast paths by its caller.
pub(crate) fn char_at(
    bytes: &[u8],
    chars: usize,
    index: usize,
    hint: Option<CursorHint>,
) -> Option<(char, CursorHint)> {
    if index >= chars {
        return None;
    }
    if bytes.len() == chars {
        return Some((*bytes.get(index)? as char, (index, index)));
    }
    let offset = char_to_byte(bytes, chars, index, hint);
    let value = decode_char_at(bytes, offset)?;
    Some((value, (index, offset)))
}

/// Resolves the char range `start..end` to its byte range within the span.
/// The caller must have validated `start <= end <= chars`.
pub(crate) fn char_range_to_bytes(
    bytes: &[u8],
    chars: usize,
    start: usize,
    end: usize,
    hint: Option<CursorHint>,
) -> (usize, usize) {
    debug_assert!(start <= end && end <= chars);
    if bytes.len() == chars {
        return (start, end);
    }
    let from = char_to_byte(bytes, chars, start, hint);
    let to = char_to_byte(bytes, chars, end, Some((start, from)));
    (from, to)
}

/// Resolves a char index to its byte offset on non-ASCII contents by
/// stepping from the nearest anchor. `index` may equal the char count,
/// resolving to the byte length. The caller must have validated
/// `index <= chars` against a span of valid UTF-8.
pub(crate) fn char_to_byte(
    bytes: &[u8],
    chars: usize,
    index: usize,
    hint: Option<CursorHint>,
) -> usize {
    debug_assert!(index <= chars);
    let mut anchor_char = 0usize;
    let mut anchor_byte = 0usize;
    let mut distance = index;
    if chars - index < distance {
        anchor_char = chars;
        anchor_byte = bytes.len();
        distance = chars - index;
    }
    if let Some((hint_char, hint_byte)) = hint
        && hint_char.abs_diff(index) < distance
    {
        anchor_char = hint_char;
        anchor_byte = hint_byte;
    }
    let mut offset = anchor_byte;
    if index >= anchor_char {
        for _ in 0..index - anchor_char {
            offset += 1;
            while offset < bytes.len() && bytes[offset] & 0xC0 == 0x80 {
                offset += 1;
            }
        }
    } else {
        for _ in 0..anchor_char - index {
            offset -= 1;
            while bytes[offset] & 0xC0 == 0x80 {
                offset -= 1;
            }
        }
    }
    offset
}

/// One cached cursor: the owning slot plus a resolved position. `slot` is
/// `u32::MAX` (never a valid arena index, the heap limit caps growth long
/// before) while the entry is empty.
type CursorEntry = (u32, u32, u32);

const EMPTY: CursorEntry = (u32::MAX, 0, 0);

/// Two slot-keyed string cursors owned by the heap.
///
/// Entries are only trusted while their slot provably holds the same string
/// contents at the same span offsets: mutation re-anchors the affected entry
/// and every collection clears both, which covers both slot reuse and span
/// movement because freed slots re-enter circulation and spans move only
/// through a collection.
pub(crate) struct StringCursors {
    entries: [Cell<CursorEntry>; 2],
}

impl StringCursors {
    pub(crate) fn new() -> Self {
        Self {
            entries: [Cell::new(EMPTY), Cell::new(EMPTY)],
        }
    }

    /// Returns the cached position for a slot, when present.
    #[inline]
    pub(crate) fn lookup(&self, reference: GcRef) -> Option<CursorHint> {
        for entry in &self.entries {
            let (slot, char_index, byte_offset) = entry.get();
            if slot == reference.0 {
                return Some((char_index as usize, byte_offset as usize));
            }
        }
        None
    }

    /// Caches a resolved position, replacing the slot's previous entry or
    /// evicting the older of the two. Positions beyond `u32` stay uncached,
    /// so enormous strings fall back to the start and end anchors.
    #[inline]
    pub(crate) fn store(&self, reference: GcRef, position: CursorHint) {
        let (Ok(char_index), Ok(byte_offset)) =
            (u32::try_from(position.0), u32::try_from(position.1))
        else {
            return;
        };
        let entry = (reference.0, char_index, byte_offset);
        if self.entries[0].get().0 != reference.0 && self.entries[1].get().0 != reference.0 {
            self.entries[1].set(self.entries[0].get());
        }
        let target = if self.entries[1].get().0 == reference.0 {
            &self.entries[1]
        } else {
            &self.entries[0]
        };
        target.set(entry);
    }

    /// Drops every entry. Called at collection, after which freed slots can
    /// be reused and surviving spans have moved.
    pub(crate) fn clear(&self) {
        for entry in &self.entries {
            entry.set(EMPTY);
        }
    }
}
