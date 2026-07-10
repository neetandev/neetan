//! XDF floppy disk image format parser.
//!
//! XDF is the headerless raw sector format for X68000 2HD floppies. It uses
//! the same fixed geometry as the PC-98 HDM container: 77 cylinders, 2 heads,
//! 8 sectors/track, 1024 bytes/sector, exactly 1,261,568 bytes. Parsing and
//! serialization are shared with the HDM module; only the format name and
//! error type differ.

use std::fmt;

use super::{d88::D88Disk, hdm};

/// Error type for XDF parsing.
#[derive(Debug, Clone)]
pub enum XdfError {
    /// Image data is not the expected size.
    InvalidSize {
        /// Actual byte count of the image data.
        actual: usize,
    },
}

impl fmt::Display for XdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XdfError::InvalidSize { actual } => {
                write!(
                    f,
                    "XDF image size is {actual} bytes, expected exactly {}",
                    hdm::HDM_FILE_SIZE
                )
            }
        }
    }
}

/// Parses an XDF disk image from raw bytes.
pub fn from_bytes(data: &[u8]) -> Result<D88Disk, XdfError> {
    hdm::from_bytes(data).map_err(|error| match error {
        hdm::HdmError::InvalidSize { actual } => XdfError::InvalidSize { actual },
    })
}

/// Serializes a `D88Disk` back into the fixed XDF raw layout.
pub fn to_bytes(disk: &D88Disk) -> Vec<u8> {
    hdm::to_bytes(disk)
}

/// Returns whether `disk` can be represented without data loss as XDF.
pub(crate) fn is_representable(disk: &D88Disk) -> bool {
    hdm::is_representable(disk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_wrong_size() {
        assert!(matches!(
            from_bytes(&[0; 512]),
            Err(XdfError::InvalidSize { actual: 512 })
        ));
    }

    #[test]
    fn roundtrip_unchanged() {
        let mut original = vec![0u8; hdm::HDM_FILE_SIZE];
        for (index, byte) in original.iter_mut().enumerate() {
            *byte = (index & 0xFF) as u8;
        }
        let disk = from_bytes(&original).unwrap();
        assert_eq!(to_bytes(&disk), original);
    }
}
