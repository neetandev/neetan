//! Content-addressed ROM set loading shared by the machine crates.
//!
//! ROMs are selected by content hash rather than by file name: a scan reads
//! every file in a ROM directory, computes its BLAKE3 digest, and maps the
//! digest to the file contents. A machine then looks up each of its slots by
//! the digests it accepts, so any dump layout works regardless of how the files
//! are named and stray files are ignored.

#![warn(missing_docs)]
#![deny(unsafe_code)]

use std::{collections::HashMap, fmt, path::Path};

/// One ROM slot: its human label, expected size, and the BLAKE3 digests
/// accepted as valid content for it. Multiple digests allow several known good
/// dumps to satisfy the same slot.
#[derive(Debug, Clone, Copy)]
pub struct RomSlot {
    /// The human-readable slot name used in error messages.
    pub label: &'static str,
    /// The expected image size in bytes.
    pub size: usize,
    /// The BLAKE3 digests accepted as valid content for this slot.
    pub accepted: &'static [&'static str],
}

impl RomSlot {
    /// Builds a slot from its label, size and accepted digests.
    pub const fn new(label: &'static str, size: usize, accepted: &'static [&'static str]) -> Self {
        Self {
            label,
            size,
            accepted,
        }
    }
}

/// A ROM directory could not be scanned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryScanError {
    /// The directory that failed to scan.
    pub directory: String,
    /// The underlying filesystem error message.
    pub message: String,
}

impl fmt::Display for DirectoryScanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "failed to read ROM directory {}: {}",
            self.directory, self.message
        )
    }
}

impl std::error::Error for DirectoryScanError {}

/// Error encountered while loading a ROM set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RomError {
    /// The ROM directory could not be scanned.
    Read {
        /// The directory that failed to scan.
        directory: String,
        /// The underlying filesystem error message.
        message: String,
    },
    /// No scanned image matched a slot's accepted digests.
    Missing {
        /// The ROM slot label.
        label: String,
        /// The accepted digests for that slot.
        accepted: Vec<String>,
    },
}

impl fmt::Display for RomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { directory, message } => {
                write!(
                    formatter,
                    "failed to read ROM directory {directory}: {message}"
                )
            }
            Self::Missing { label, accepted } => write!(
                formatter,
                "no ROM matched the {label} slot (accepted digests: {})",
                accepted.join(", ")
            ),
        }
    }
}

impl std::error::Error for RomError {}

impl From<DirectoryScanError> for RomError {
    fn from(error: DirectoryScanError) -> Self {
        Self::Read {
            directory: error.directory,
            message: error.message,
        }
    }
}

/// Builds the [`RomError::Missing`] error for a slot.
pub fn missing_rom(slot: &RomSlot) -> RomError {
    RomError::Missing {
        label: slot.label.to_string(),
        accepted: slot
            .accepted
            .iter()
            .map(|digest| digest.to_string())
            .collect(),
    }
}

/// Expands one scanned file into the candidate images it contains.
pub type ExpandImage = fn(&[u8]) -> Vec<Vec<u8>>;

/// How a ROM directory is scanned.
pub struct ScanOptions<'a> {
    /// File sizes the scan keeps. An empty slice keeps every file.
    pub accepted_sizes: &'a [usize],
    /// How many subdirectory levels the scan descends into.
    pub subdirectory_depth: usize,
    /// Expands one file into the candidate images it may contain. Applied
    /// before the size filter, so a packed container is replaced by its parts.
    pub expand: Option<ExpandImage>,
}

impl ScanOptions<'_> {
    /// Scan options that keep every file in the directory itself.
    pub const ANY_SIZE: Self = Self {
        accepted_sizes: &[],
        subdirectory_depth: 0,
        expand: None,
    };

    /// Scan options that keep the files whose size matches `accepted_sizes`.
    pub const fn sizes(accepted_sizes: &[usize]) -> ScanOptions<'_> {
        ScanOptions {
            accepted_sizes,
            subdirectory_depth: 0,
            expand: None,
        }
    }
}

struct IndexEntry {
    data: Vec<u8>,
    match_count: usize,
}

/// Candidate ROM images from one directory scan, keyed by BLAKE3 digest.
pub struct RomIndex {
    by_digest: HashMap<String, IndexEntry>,
}

impl RomIndex {
    /// Builds an index directly from a set of images, for tests and for callers
    /// that assemble candidates themselves.
    pub fn from_images(images: impl IntoIterator<Item = Vec<u8>>) -> Self {
        let mut index = Self {
            by_digest: HashMap::new(),
        };
        for image in images {
            index.insert(image);
        }
        index
    }

    /// Builds an index from digest and image pairs the caller computed itself.
    pub fn from_entries(entries: impl IntoIterator<Item = (String, Vec<u8>)>) -> Self {
        Self {
            by_digest: entries
                .into_iter()
                .map(|(digest, data)| {
                    (
                        digest,
                        IndexEntry {
                            data,
                            match_count: 1,
                        },
                    )
                })
                .collect(),
        }
    }

    fn insert(&mut self, data: Vec<u8>) {
        self.by_digest
            .entry(blake3_hex(&data))
            .and_modify(|entry| entry.match_count += 1)
            .or_insert(IndexEntry {
                data,
                match_count: 1,
            });
    }

    /// Returns the bytes stored under `digest`.
    pub fn bytes(&self, digest: &str) -> Option<&[u8]> {
        self.by_digest
            .get(digest)
            .map(|entry| entry.data.as_slice())
    }

    /// Returns how many scanned images produced `digest`.
    pub fn match_count(&self, digest: &str) -> usize {
        self.by_digest
            .get(digest)
            .map_or(0, |entry| entry.match_count)
    }

    /// Returns the first accepted image for `slot`.
    pub fn take(&self, slot: &RomSlot) -> Result<Vec<u8>, RomError> {
        self.take_optional(slot).ok_or_else(|| missing_rom(slot))
    }

    /// Returns the first accepted image for `slot`, or `None` when the scan
    /// found no match.
    pub fn take_optional(&self, slot: &RomSlot) -> Option<Vec<u8>> {
        slot.accepted
            .iter()
            .find_map(|digest| self.bytes(digest))
            .map(<[u8]>::to_vec)
    }
}

/// Maps every accepted file under `directory` to its BLAKE3 digest.
///
/// Files that cannot be read are skipped. When several files share a digest the
/// first one wins and the match count is raised, which lets a caller reject an
/// ambiguous ROM directory.
pub fn scan_directory(
    directory: &Path,
    options: &ScanOptions<'_>,
) -> Result<RomIndex, DirectoryScanError> {
    let mut index = RomIndex {
        by_digest: HashMap::new(),
    };
    scan_level(directory, options, options.subdirectory_depth, &mut index)?;
    Ok(index)
}

fn scan_level(
    directory: &Path,
    options: &ScanOptions<'_>,
    remaining_depth: usize,
    index: &mut RomIndex,
) -> Result<(), DirectoryScanError> {
    let scan_error = |message: String| DirectoryScanError {
        directory: directory.display().to_string(),
        message,
    };

    let entries = std::fs::read_dir(directory).map_err(|error| scan_error(error.to_string()))?;

    for entry in entries {
        let entry = entry.map_err(|error| scan_error(error.to_string()))?;
        let path = entry.path();
        if path.is_dir() {
            if remaining_depth > 0 {
                scan_level(&path, options, remaining_depth - 1, index)?;
            }
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        match options.expand {
            Some(expand) => {
                for candidate in expand(&data) {
                    if accepts_size(options, candidate.len()) {
                        index.insert(candidate);
                    }
                }
            }
            None => {
                if accepts_size(options, data.len()) {
                    index.insert(data);
                }
            }
        }
    }
    Ok(())
}

fn accepts_size(options: &ScanOptions<'_>, size: usize) -> bool {
    options.accepted_sizes.is_empty() || options.accepted_sizes.contains(&size)
}

/// Returns the lowercase hexadecimal BLAKE3 digest of `data`.
pub fn blake3_hex(data: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(data);
    let mut digest = [0u8; 32];
    hasher.finalize(&mut digest);

    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        hex.push(HEX_DIGITS[(byte & 0x0F) as usize] as char);
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_directory(files: &[(&str, &[u8])]) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "rom_loader_test_{}",
            blake3_hex(
                files
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<String>()
                    .as_bytes()
            )
        ));
        let _ = std::fs::remove_dir_all(&root);
        for (name, data) in files {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, data).unwrap();
        }
        root
    }

    #[test]
    fn missing_directory_reports_a_scan_error() {
        let result = scan_directory(Path::new("/nonexistent/rom/dir"), &ScanOptions::sizes(&[4]));
        assert!(result.is_err());
    }

    #[test]
    fn empty_accepted_sizes_keeps_every_file() {
        let root = write_directory(&[("a.rom", &[1, 2, 3]), ("b.rom", &[4, 5, 6, 7])]);
        let index = scan_directory(&root, &ScanOptions::ANY_SIZE).unwrap();
        assert_eq!(index.bytes(&blake3_hex(&[1, 2, 3])), Some(&[1, 2, 3][..]));
        assert_eq!(
            index.bytes(&blake3_hex(&[4, 5, 6, 7])),
            Some(&[4, 5, 6, 7][..])
        );
    }

    #[test]
    fn size_filter_drops_unrelated_files() {
        let root = write_directory(&[("a.rom", &[1, 2, 3]), ("stray.txt", &[4, 5, 6, 7])]);
        let index = scan_directory(&root, &ScanOptions::sizes(&[3])).unwrap();
        assert!(index.bytes(&blake3_hex(&[1, 2, 3])).is_some());
        assert!(index.bytes(&blake3_hex(&[4, 5, 6, 7])).is_none());
    }

    #[test]
    fn subdirectory_depth_controls_recursion() {
        let root = write_directory(&[("nested/a.rom", &[1, 2, 3])]);
        let flat = scan_directory(&root, &ScanOptions::sizes(&[3])).unwrap();
        assert!(flat.bytes(&blake3_hex(&[1, 2, 3])).is_none());

        let recursive = scan_directory(
            &root,
            &ScanOptions {
                accepted_sizes: &[3],
                subdirectory_depth: 1,
                expand: None,
            },
        )
        .unwrap();
        assert!(recursive.bytes(&blake3_hex(&[1, 2, 3])).is_some());
    }

    #[test]
    fn expand_splits_a_packed_container() {
        fn split_halves(data: &[u8]) -> Vec<Vec<u8>> {
            if data.len() == 4 {
                vec![data[..2].to_vec(), data[2..].to_vec()]
            } else {
                vec![data.to_vec()]
            }
        }

        let root = write_directory(&[("packed.rom", &[1, 2, 3, 4])]);
        let index = scan_directory(
            &root,
            &ScanOptions {
                accepted_sizes: &[2],
                subdirectory_depth: 0,
                expand: Some(split_halves),
            },
        )
        .unwrap();
        assert_eq!(index.bytes(&blake3_hex(&[1, 2])), Some(&[1, 2][..]));
        assert_eq!(index.bytes(&blake3_hex(&[3, 4])), Some(&[3, 4][..]));
        assert!(index.bytes(&blake3_hex(&[1, 2, 3, 4])).is_none());
    }

    #[test]
    fn match_count_reports_identical_files() {
        let index = RomIndex::from_images([vec![1, 2, 3], vec![1, 2, 3], vec![4, 5, 6]]);
        assert_eq!(index.match_count(&blake3_hex(&[1, 2, 3])), 2);
        assert_eq!(index.match_count(&blake3_hex(&[4, 5, 6])), 1);
        assert_eq!(index.match_count("unknown"), 0);
    }

    #[test]
    fn take_reports_the_slot_label_when_absent() {
        const ACCEPTED: &[&str] = &["0123", "4567"];
        let slot = RomSlot::new("ipl", 3, ACCEPTED);
        let index = RomIndex::from_images([vec![1, 2, 3]]);

        assert_eq!(index.take_optional(&slot), None);
        let error = index.take(&slot).unwrap_err();
        assert_eq!(
            error.to_string(),
            "no ROM matched the ipl slot (accepted digests: 0123, 4567)"
        );
    }

    #[test]
    fn take_returns_the_first_accepted_digest() {
        let first = blake3_hex(&[1, 2, 3]);
        let accepted: &'static [&'static str] =
            Box::leak(vec![Box::leak(first.into_boxed_str()) as &str].into_boxed_slice());
        let slot = RomSlot::new("ipl", 3, accepted);
        let index = RomIndex::from_images([vec![1, 2, 3]]);
        assert_eq!(index.take(&slot).unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn scan_error_converts_into_a_read_error() {
        let error = RomError::from(DirectoryScanError {
            directory: "/roms".to_string(),
            message: "not found".to_string(),
        });
        assert_eq!(
            error.to_string(),
            "failed to read ROM directory /roms: not found"
        );
    }
}
