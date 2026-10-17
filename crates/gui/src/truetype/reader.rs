//! A big endian byte reader with bounds checks.

use std::cmp::Ordering;

use super::{FontError, Tag};

/// Reads big endian values in order from a byte slice.
///
/// Every read checks the bounds and returns [`FontError::Truncated`] when the
/// value reaches past the end of the data.
#[derive(Clone, Copy)]
pub(crate) struct Reader<'a> {
    /// The bytes to read.
    data: &'a [u8],
    /// The offset of the next read.
    position: usize,
}

impl<'a> Reader<'a> {
    /// A reader at the start of `data`.
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    /// A reader at `offset` of `data`.
    pub(crate) fn at(data: &'a [u8], offset: usize) -> Result<Self, FontError> {
        if offset > data.len() {
            return Err(FontError::Truncated);
        }
        Ok(Self {
            data,
            position: offset,
        })
    }

    /// Skips `count` bytes.
    pub(crate) fn skip(&mut self, count: usize) -> Result<(), FontError> {
        self.bytes(count).map(|_| ())
    }

    /// The next `count` bytes.
    pub(crate) fn bytes(&mut self, count: usize) -> Result<&'a [u8], FontError> {
        let bytes = slice(self.data, self.position, count)?;
        self.position += count;
        Ok(bytes)
    }

    /// The next `N` bytes as an array.
    fn array<const N: usize>(&mut self) -> Result<[u8; N], FontError> {
        let mut array = [0; N];
        array.copy_from_slice(self.bytes(N)?);
        Ok(array)
    }

    /// The next unsigned byte.
    pub(crate) fn u8(&mut self) -> Result<u8, FontError> {
        Ok(self.array::<1>()?[0])
    }

    /// The next signed byte.
    pub(crate) fn i8(&mut self) -> Result<i8, FontError> {
        Ok(i8::from_be_bytes(self.array()?))
    }

    /// The next unsigned 16-bit value.
    pub(crate) fn u16(&mut self) -> Result<u16, FontError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    /// The next signed 16-bit value.
    pub(crate) fn i16(&mut self) -> Result<i16, FontError> {
        Ok(i16::from_be_bytes(self.array()?))
    }

    /// The next unsigned 32-bit value.
    pub(crate) fn u32(&mut self) -> Result<u32, FontError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    /// The next 2.14 fixed point value.
    pub(crate) fn f2dot14(&mut self) -> Result<f32, FontError> {
        Ok(f32::from(self.i16()?) / 16384.0)
    }

    /// The next table tag.
    pub(crate) fn tag(&mut self) -> Result<Tag, FontError> {
        Ok(Tag(self.array()?))
    }
}

/// A range of bytes of the font data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Span {
    /// The offset of the first byte.
    pub(crate) offset: usize,
    /// The number of bytes.
    pub(crate) length: usize,
}

impl Span {
    /// The span of the `length` bytes at `offset` of `data`. A span past the
    /// end of the data fails.
    pub(crate) fn new(data: &[u8], offset: usize, length: usize) -> Result<Self, FontError> {
        slice(data, offset, length)?;
        Ok(Self { offset, length })
    }

    /// The bytes of the span in `data`. A span past the end of `data` has no
    /// bytes.
    pub(crate) fn get<'a>(&self, data: &'a [u8]) -> &'a [u8] {
        slice(data, self.offset, self.length).unwrap_or(&[])
    }
}

/// The `count` bytes at `offset` of `data`.
pub(crate) fn slice(data: &[u8], offset: usize, count: usize) -> Result<&[u8], FontError> {
    let end = offset.checked_add(count).ok_or(FontError::Truncated)?;
    data.get(offset..end).ok_or(FontError::Truncated)
}

/// The bytes of `data` from `offset` to the end.
pub(crate) fn tail(data: &[u8], offset: usize) -> Result<&[u8], FontError> {
    data.get(offset..).ok_or(FontError::Truncated)
}

/// The unsigned 16-bit value at `offset` of `data`.
pub(crate) fn u16_at(data: &[u8], offset: usize) -> Result<u16, FontError> {
    Reader::at(data, offset)?.u16()
}

/// The signed 16-bit value at `offset` of `data`.
pub(crate) fn i16_at(data: &[u8], offset: usize) -> Result<i16, FontError> {
    Reader::at(data, offset)?.i16()
}

/// The unsigned 32-bit value at `offset` of `data`.
pub(crate) fn u32_at(data: &[u8], offset: usize) -> Result<u32, FontError> {
    Reader::at(data, offset)?.u32()
}

/// Finds the record of a sorted array of `count` records that `compare`
/// reports as equal. `compare` returns the order of a record relative to the
/// searched one, or `None` when the record cannot be read. The search ends on
/// the first unreadable record.
pub(crate) fn binary_search(
    count: usize,
    compare: impl Fn(usize) -> Option<Ordering>,
) -> Option<usize> {
    let mut low = 0;
    let mut high = count;
    while low < high {
        let middle = low + (high - low) / 2;
        match compare(middle)? {
            Ordering::Less => low = middle + 1,
            Ordering::Greater => high = middle,
            Ordering::Equal => return Some(middle),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_big_endian() {
        let data = [
            0x12, 0x34, 0xFF, 0xFE, 0x80, 0x00, 0x00, 0x01, b'g', b'l', b'y', b'f',
        ];
        let mut reader = Reader::new(&data);
        assert_eq!(reader.u16(), Ok(0x1234));
        assert_eq!(reader.i16(), Ok(-2));
        assert_eq!(reader.u32(), Ok(0x8000_0001));
        assert_eq!(reader.tag(), Ok(Tag(*b"glyf")));
        assert_eq!(reader.u8(), Err(FontError::Truncated));
        assert_eq!(Reader::new(&[0x40, 0x00]).f2dot14(), Ok(1.0));
        assert_eq!(Reader::new(&[0xE0, 0x00]).f2dot14(), Ok(-0.5));
        assert_eq!(Reader::new(&[0xFF]).i8(), Ok(-1));
        assert_eq!(Reader::new(&[0xFF]).u8(), Ok(255));
    }

    #[test]
    fn reads_past_the_end_fail() {
        let data = [1, 2, 3];
        let mut reader = Reader::new(&data);
        assert_eq!(reader.u16(), Ok(0x0102));
        assert_eq!(reader.u16(), Err(FontError::Truncated));
        assert_eq!(reader.u8(), Ok(3));
        assert_eq!(reader.u8(), Err(FontError::Truncated));
        assert_eq!(reader.skip(1), Err(FontError::Truncated));
        assert!(Reader::at(&data, 3).is_ok());
        assert!(Reader::at(&data, 4).is_err());
        assert_eq!(u32_at(&data, 0), Err(FontError::Truncated));
        assert_eq!(slice(&data, usize::MAX, 2), Err(FontError::Truncated));
        assert_eq!(tail(&data, 4), Err(FontError::Truncated));
        assert_eq!(i16_at(&data, 1), Ok(0x0203));
        assert_eq!(u16_at(&data, 2), Err(FontError::Truncated));
    }

    #[test]
    fn spans_stay_inside_the_data() {
        let data = [1, 2, 3, 4];
        let span = Span::new(&data, 1, 2).unwrap();
        assert_eq!(span.get(&data), &[2, 3]);
        assert_eq!(span.get(&data[..2]), &[] as &[u8]);
        assert_eq!(Span::new(&data, 3, 2), Err(FontError::Truncated));
    }

    #[test]
    fn the_binary_search_finds_records() {
        let values = [1, 3, 5, 7, 9];
        let find =
            |value: i32| binary_search(values.len(), |index| Some(values[index].cmp(&value)));
        assert_eq!(find(1), Some(0));
        assert_eq!(find(9), Some(4));
        assert_eq!(find(4), None);
        assert_eq!(binary_search(values.len(), |_| None), None);
        assert_eq!(binary_search(0, |_| Some(Ordering::Equal)), None);
    }
}
