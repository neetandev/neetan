//! NFD (T98FDDIMAGE) floppy disk image format parser.
//!
//! NFD is a per-sector metadata format from the T98-Next emulator that stores
//! C/H/R/N, MFM flag, FDC status, and PDA disk type alongside raw sector data.
//! Two revisions exist:
//!
//! - **R0**: Fixed 68,112-byte header with a flat 163×26 sector map.
//! - **R1**: Compact header with per-track pointers and variable sector counts.

use std::fmt;

use common::warn;

use super::d88::{D88Disk, D88MediaType, D88Sector};

const NFD_R0_MAGIC: &[u8; 15] = b"T98FDDIMAGE.R0\0";
const NFD_R1_MAGIC: &[u8; 15] = b"T98FDDIMAGE.R1\0";

const COMMON_HEADER_SIZE: usize = 0x120;
const NFD_COMMENT_SIZE: usize = 0x100;
const R0_TRACK_MAX: usize = 163;
const R1_TRACK_MAX: usize = 164;
const SECTORS_PER_TRACK: usize = 26;
const SECTOR_ENTRY_SIZE: usize = 16;
const DIAG_ENTRY_SIZE: usize = 16;

/// Number of leading bytes an empty R0 sector-map entry fills with 0xFF (the
/// C/H/R/N, MFM, DDAM, status, ST0-2, and PDA fields); the trailing reserved
/// bytes stay zero.
const R0_EMPTY_ENTRY_FILL: usize = 11;

/// R1 track-offset table size (164 tracks x 4 bytes each).
const R1_TRACK_TABLE_SIZE: usize = R1_TRACK_MAX * 4;

/// R1 per-track header size (u16 sector_count + u16 diag_count + 12 reserved bytes).
const R1_TRACK_HEADER_SIZE: usize = 16;

/// Byte offset where the R0 sector map ends (common header + 163 x 26 entries).
/// Real files append a Reserve3 block before the sector data, so the actual
/// dwHeadSize can be larger; those extra bytes are preserved as `header_tail`.
const R0_SECTOR_MAP_END: usize =
    COMMON_HEADER_SIZE + R0_TRACK_MAX * SECTORS_PER_TRACK * SECTOR_ENTRY_SIZE;

/// Byte offset where the R1 fixed header ends (common header + track table).
const R1_FIXED_HEADER_END: usize = COMMON_HEADER_SIZE + R1_TRACK_TABLE_SIZE;

/// Error type for NFD parsing.
#[derive(Debug, Clone)]
pub enum NfdError {
    /// Image data too small for common header.
    TooSmall,
    /// Not a valid NFD R0 or R1 image.
    InvalidMagic,
    /// Header size from dwHeadSize exceeds file length.
    HeaderTruncated {
        /// Header size declared in the file.
        header_size: u32,
        /// Actual byte count of the image data.
        actual: usize,
    },
    /// Sector data runs past end of file.
    DataTruncated {
        /// Track index where truncation was detected.
        track: usize,
        /// Byte offset within the image.
        offset: usize,
    },
    /// R1 track pointer out of bounds.
    InvalidTrackOffset {
        /// Track index with the invalid pointer.
        track: usize,
        /// The invalid offset value.
        offset: u32,
    },
    /// Unrecognized PDA byte value.
    UnknownPda(u8),
}

impl fmt::Display for NfdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NfdError::TooSmall => write!(f, "NFD image too small for header"),
            NfdError::InvalidMagic => write!(f, "not a valid NFD R0 or R1 image"),
            NfdError::HeaderTruncated {
                header_size,
                actual,
            } => {
                write!(
                    f,
                    "NFD header truncated: dwHeadSize={header_size}, file is {actual} bytes"
                )
            }
            NfdError::DataTruncated { track, offset } => {
                write!(
                    f,
                    "NFD sector data truncated at track {track}, offset {offset}"
                )
            }
            NfdError::InvalidTrackOffset { track, offset } => {
                write!(f, "NFD R1 track {track} offset {offset:#X} out of bounds")
            }
            NfdError::UnknownPda(pda) => write!(f, "unknown NFD PDA byte: {pda:#04X}"),
        }
    }
}

fn media_type_from_pda(pda: u8, size_code: u8) -> Result<D88MediaType, NfdError> {
    match pda {
        0x10 => Ok(D88MediaType::Disk2DD),
        0x30 | 0x90 => Ok(D88MediaType::Disk2HD),
        0x00 => {
            if size_code <= 1 {
                Ok(D88MediaType::Disk2HD)
            } else {
                Ok(D88MediaType::Disk2DD)
            }
        }
        _ => Err(NfdError::UnknownPda(pda)),
    }
}

/// Maps a `D88MediaType` to the canonical NFD PDA byte, used as a fallback
/// when per-sector metadata is unavailable. The `0x30 -> 0x90` direction is
/// lossy; both decode to 2HD on parse. The real per-sector PDA is preserved
/// in `NfdExtra`, so canonical images do not depend on this mapping.
fn pda_from_media_type(media_type: D88MediaType) -> u8 {
    match media_type {
        D88MediaType::Disk2D | D88MediaType::Disk2DD => 0x10,
        D88MediaType::Disk2HD => 0x90,
    }
}

fn read_u16_le(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

/// Which NFD revision a parsed image came from. Used to select the
/// matching serializer when re-emitting the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NfdRevision {
    /// R0 revision (fixed 163 x 26 sector map).
    R0,
    /// R1 revision (per-track headers and offset table).
    R1,
}

/// NFD container metadata preserved for lossless re-emit.
#[derive(Debug, Clone)]
pub struct NfdExtra {
    /// Raw reserved byte at header offset 0x0F (after the 15-byte magic).
    pub magic_pad: u8,
    /// Raw comment block, header bytes 0x10..0x110.
    pub comment: Box<[u8; NFD_COMMENT_SIZE]>,
    /// Raw write-protect byte at 0x114 (any nonzero value means protected).
    pub protect_byte: u8,
    /// Raw head-count byte at 0x115.
    pub head_count: u8,
    /// Raw reserved header bytes 0x116..0x120.
    pub header_reserved: [u8; 10],
    /// File bytes past the last parsed data byte, preserved verbatim.
    pub trailing: Vec<u8>,
    /// Revision-specific preserved metadata.
    pub revision: NfdRevisionExtra,
}

/// Revision-specific NFD metadata.
#[derive(Debug, Clone)]
pub enum NfdRevisionExtra {
    /// R0 metadata (flat 163 x 26 sector map).
    R0(NfdR0Extra),
    /// R1 metadata (per-track blocks, retries, diagnostic reads).
    R1(NfdR1Extra),
}

/// Preserved R0 metadata.
#[derive(Debug, Clone)]
pub struct NfdR0Extra {
    /// Raw header bytes between the sector-map end and dwHeadSize.
    pub header_tail: Vec<u8>,
    /// Per-track sector-entry extras, aligned with the parsed track sectors.
    pub tracks: Vec<Vec<NfdR0SectorExtra>>,
}

/// Preserved R0 per-sector entry fields not carried by `D88Sector`.
#[derive(Debug, Clone)]
pub struct NfdR0SectorExtra {
    /// Raw flMFM byte (0 = FM, nonzero = MFM).
    pub mfm_byte: u8,
    /// FDC ST0/ST1/ST2 result bytes (entry bytes 7..10).
    pub fdc_status: [u8; 3],
    /// PDA byte (entry byte 10).
    pub pda: u8,
    /// Raw reserved entry bytes 11..16.
    pub reserved: [u8; 5],
}

/// Preserved R1 metadata.
#[derive(Debug, Clone)]
pub struct NfdR1Extra {
    /// Raw bytes between the track-offset table and the first track block:
    /// dwAddInfo plus reserved bytes in spec files, empty in compact files.
    pub header_gap: Vec<u8>,
    /// dwAddInfo value carried in the header gap; nonzero blocks re-emit.
    pub additional_info_offset: u32,
    /// Whether track blocks follow the packed canonical layout the serializer
    /// reproduces; false keeps the image re-emit gated.
    pub canonical_layout: bool,
    /// Per-track extras; `None` where the source had no track block.
    pub tracks: Vec<Option<NfdR1TrackExtra>>,
}

/// Preserved R1 per-track metadata.
#[derive(Debug, Clone)]
pub struct NfdR1TrackExtra {
    /// Raw reserved bytes 4..16 of the track header.
    pub header_reserved: [u8; 12],
    /// Aligned with the track's parsed sectors.
    pub sectors: Vec<NfdR1SectorExtra>,
    /// Special-read (diagnostic) entries with their raw payloads.
    pub diag: Vec<NfdDiagExtra>,
}

/// Preserved R1 per-sector entry fields not carried by `D88Sector`.
#[derive(Debug, Clone)]
pub struct NfdR1SectorExtra {
    /// Raw flMFM byte (0 = FM, nonzero = MFM).
    pub mfm_byte: u8,
    /// FDC ST0/ST1/ST2 result bytes (entry bytes 7..10).
    pub fdc_status: [u8; 3],
    /// PDA byte (entry byte 11).
    pub pda: u8,
    /// Raw reserved entry bytes 12..16.
    pub reserved: [u8; 4],
    /// Retry read copies following the primary data, each sector-sized.
    pub retry_data: Vec<Vec<u8>>,
}

/// Preserved R1 diagnostic ("special read") entry with its raw payload.
#[derive(Debug, Clone)]
pub struct NfdDiagExtra {
    /// Raw 16-byte special-read entry as stored in the file.
    pub entry: [u8; DIAG_ENTRY_SIZE],
    /// Raw payload: dwDataLen * (1 + byRetry) bytes.
    pub payload: Vec<u8>,
}

impl NfdExtra {
    /// Returns which NFD revision this metadata came from.
    pub fn revision(&self) -> NfdRevision {
        match self.revision {
            NfdRevisionExtra::R0(_) => NfdRevision::R0,
            NfdRevisionExtra::R1(_) => NfdRevision::R1,
        }
    }

    /// Resets a track's preserved metadata to defaults after a format. Newly
    /// formatted sectors are MFM with cleared FDC status; the track's retry
    /// copies and diagnostic entries are dropped.
    pub(crate) fn reset_track(
        &mut self,
        track_index: usize,
        sector_count: usize,
        media_type: D88MediaType,
    ) {
        let pda = pda_from_media_type(media_type);
        match &mut self.revision {
            NfdRevisionExtra::R0(r0) => {
                if track_index >= r0.tracks.len() {
                    r0.tracks.resize_with(track_index + 1, Vec::new);
                }
                r0.tracks[track_index] = (0..sector_count)
                    .map(|_| NfdR0SectorExtra {
                        mfm_byte: 1,
                        fdc_status: [0; 3],
                        pda,
                        reserved: [0; 5],
                    })
                    .collect();
            }
            NfdRevisionExtra::R1(r1) => {
                if track_index >= r1.tracks.len() {
                    r1.tracks.resize_with(track_index + 1, || None);
                }
                r1.tracks[track_index] = Some(NfdR1TrackExtra {
                    header_reserved: [0; 12],
                    sectors: (0..sector_count)
                        .map(|_| NfdR1SectorExtra {
                            mfm_byte: 1,
                            fdc_status: [0; 3],
                            pda,
                            reserved: [0; 4],
                            retry_data: Vec::new(),
                        })
                        .collect(),
                    diag: Vec::new(),
                });
            }
        }
    }
}

fn read_comment(data: &[u8]) -> Box<[u8; NFD_COMMENT_SIZE]> {
    let mut comment = Box::new([0u8; NFD_COMMENT_SIZE]);
    comment.copy_from_slice(&data[0x10..0x10 + NFD_COMMENT_SIZE]);
    comment
}

fn read_header_reserved(data: &[u8]) -> [u8; 10] {
    let mut reserved = [0u8; 10];
    reserved.copy_from_slice(&data[0x116..0x120]);
    reserved
}

/// Parses an NFD disk image (R0 or R1) from raw bytes, returning the parsed
/// disk and the preserved container metadata.
pub fn from_bytes(data: &[u8]) -> Result<(D88Disk, Box<NfdExtra>), NfdError> {
    if data.len() < COMMON_HEADER_SIZE {
        return Err(NfdError::TooSmall);
    }

    if &data[..15] == NFD_R0_MAGIC {
        parse_r0(data)
    } else if &data[..15] == NFD_R1_MAGIC {
        parse_r1(data)
    } else {
        Err(NfdError::InvalidMagic)
    }
}

fn parse_r0(data: &[u8]) -> Result<(D88Disk, Box<NfdExtra>), NfdError> {
    let head_size = read_u32_le(data, 0x110) as usize;
    let write_protected = data[0x114] != 0;

    if head_size > data.len() || head_size < R0_SECTOR_MAP_END || data.len() < R0_SECTOR_MAP_END {
        return Err(NfdError::HeaderTruncated {
            header_size: head_size as u32,
            actual: data.len(),
        });
    }

    // Count valid sectors per track for the sector_count field.
    let mut valid_counts = [0u16; R0_TRACK_MAX];
    for (track_idx, count) in valid_counts.iter_mut().enumerate() {
        for slot in 0..SECTORS_PER_TRACK {
            let entry_offset =
                COMMON_HEADER_SIZE + (track_idx * SECTORS_PER_TRACK + slot) * SECTOR_ENTRY_SIZE;
            if data[entry_offset] != 0xFF {
                *count += 1;
            }
        }
    }

    // Detect media type from the first valid sector.
    let mut media_type = D88MediaType::Disk2HD;
    'detect: for track_idx in 0..R0_TRACK_MAX {
        for slot in 0..SECTORS_PER_TRACK {
            let entry_offset =
                COMMON_HEADER_SIZE + (track_idx * SECTORS_PER_TRACK + slot) * SECTOR_ENTRY_SIZE;
            if data[entry_offset] != 0xFF {
                let size_code = data[entry_offset + 3];
                let pda = data[entry_offset + 10];
                media_type = media_type_from_pda(pda, size_code)?;
                break 'detect;
            }
        }
    }

    let mut data_offset = head_size;
    let mut track_sectors: Vec<Option<Vec<D88Sector>>> = vec![None; R0_TRACK_MAX];
    let mut track_extras: Vec<Vec<NfdR0SectorExtra>> = vec![Vec::new(); R0_TRACK_MAX];

    for track_idx in 0..R0_TRACK_MAX {
        let mut sectors = Vec::new();
        let mut extras = Vec::new();

        for slot in 0..SECTORS_PER_TRACK {
            let entry_offset =
                COMMON_HEADER_SIZE + (track_idx * SECTORS_PER_TRACK + slot) * SECTOR_ENTRY_SIZE;
            let c = data[entry_offset];

            if c == 0xFF {
                continue;
            }

            let h = data[entry_offset + 1];
            let r = data[entry_offset + 2];
            let n = data[entry_offset + 3];
            let fl_mfm = data[entry_offset + 4];
            let fl_ddam = data[entry_offset + 5];
            let by_status = data[entry_offset + 6];
            let fdc_status = [
                data[entry_offset + 7],
                data[entry_offset + 8],
                data[entry_offset + 9],
            ];
            let pda = data[entry_offset + 10];
            let mut reserved = [0u8; 5];
            reserved.copy_from_slice(&data[entry_offset + 11..entry_offset + 16]);

            let sector_size = 128usize << n;

            if data_offset + sector_size > data.len() {
                return Err(NfdError::DataTruncated {
                    track: track_idx,
                    offset: data_offset,
                });
            }

            let sector_data = data[data_offset..data_offset + sector_size].to_vec();
            let sector_data_offset = data_offset;
            data_offset += sector_size;

            sectors.push(D88Sector {
                cylinder: c,
                head: h,
                record: r,
                size_code: n,
                sector_count: valid_counts[track_idx],
                mfm_flag: if fl_mfm == 0 { 0x40 } else { 0x00 },
                deleted: fl_ddam,
                status: by_status,
                reserved: [0u8; 5],
                data: sector_data,
                source_offset: Some(sector_data_offset as u64),
            });
            extras.push(NfdR0SectorExtra {
                mfm_byte: fl_mfm,
                fdc_status,
                pda,
                reserved,
            });
        }

        if !sectors.is_empty() {
            track_sectors[track_idx] = Some(sectors);
        }
        track_extras[track_idx] = extras;
    }

    let header_tail = data[R0_SECTOR_MAP_END..head_size].to_vec();
    let trailing = data[data_offset..].to_vec();

    let extra = Box::new(NfdExtra {
        magic_pad: data[0x0F],
        comment: read_comment(data),
        protect_byte: data[0x114],
        head_count: data[0x115],
        header_reserved: read_header_reserved(data),
        trailing,
        revision: NfdRevisionExtra::R0(NfdR0Extra {
            header_tail,
            tracks: track_extras,
        }),
    });

    let disk = D88Disk::from_tracks(String::new(), write_protected, media_type, track_sectors);
    Ok((disk, extra))
}

fn parse_r1(data: &[u8]) -> Result<(D88Disk, Box<NfdExtra>), NfdError> {
    let head_size = read_u32_le(data, 0x110) as usize;
    let write_protected = data[0x114] != 0;

    if head_size > data.len() || data.len() < R1_FIXED_HEADER_END || head_size < R1_FIXED_HEADER_END
    {
        return Err(NfdError::HeaderTruncated {
            header_size: head_size as u32,
            actual: data.len(),
        });
    }

    let mut track_offsets = [0u32; R1_TRACK_MAX];
    for (i, offset) in track_offsets.iter_mut().enumerate() {
        *offset = read_u32_le(data, COMMON_HEADER_SIZE + i * 4);
    }

    // Bytes between the fixed header and the first track block (dwAddInfo plus
    // reserved bytes in spec files; empty in the compact layout we emit).
    let first_block_offset = track_offsets
        .iter()
        .copied()
        .filter(|&offset| offset != 0)
        .min()
        .map(|offset| offset as usize)
        .unwrap_or(head_size);

    if first_block_offset < R1_FIXED_HEADER_END || first_block_offset > data.len() {
        return Err(NfdError::InvalidTrackOffset {
            track: 0,
            offset: first_block_offset as u32,
        });
    }

    let header_gap = data[R1_FIXED_HEADER_END..first_block_offset].to_vec();
    let additional_info_offset = if header_gap.len() >= 4 {
        u32::from_le_bytes([header_gap[0], header_gap[1], header_gap[2], header_gap[3]])
    } else {
        0
    };

    let mut data_offset = head_size;
    let mut media_type = D88MediaType::Disk2HD;
    let mut media_type_detected = false;
    let mut track_sectors: Vec<Option<Vec<D88Sector>>> = vec![None; R1_TRACK_MAX];
    let mut track_extras: Vec<Option<NfdR1TrackExtra>> = vec![None; R1_TRACK_MAX];

    let mut canonical_layout = true;
    let mut expected_metadata_offset = R1_FIXED_HEADER_END + header_gap.len();

    for track_idx in 0..R1_TRACK_MAX {
        let track_offset = track_offsets[track_idx];
        if track_offset == 0 {
            continue;
        }

        let track_meta = track_offset as usize;
        if track_meta + R1_TRACK_HEADER_SIZE > data.len() {
            return Err(NfdError::InvalidTrackOffset {
                track: track_idx,
                offset: track_offset,
            });
        }
        if track_meta != expected_metadata_offset {
            canonical_layout = false;
        }

        let sector_count = read_u16_le(data, track_meta) as usize;
        let diag_count = read_u16_le(data, track_meta + 2) as usize;
        let mut header_reserved = [0u8; 12];
        header_reserved.copy_from_slice(&data[track_meta + 4..track_meta + 16]);

        let entries_start = track_meta + R1_TRACK_HEADER_SIZE;
        let entries_end = entries_start + sector_count * SECTOR_ENTRY_SIZE;
        let diag_entries_end = entries_end + diag_count * DIAG_ENTRY_SIZE;

        if diag_entries_end > data.len() {
            return Err(NfdError::InvalidTrackOffset {
                track: track_idx,
                offset: track_offset,
            });
        }

        let mut sectors = Vec::with_capacity(sector_count);
        let mut sector_extras = Vec::with_capacity(sector_count);

        for i in 0..sector_count {
            let entry_offset = entries_start + i * SECTOR_ENTRY_SIZE;
            let c = data[entry_offset];
            let h = data[entry_offset + 1];
            let r = data[entry_offset + 2];
            let n = data[entry_offset + 3];
            let fl_mfm = data[entry_offset + 4];
            let fl_ddam = data[entry_offset + 5];
            let by_status = data[entry_offset + 6];
            let fdc_status = [
                data[entry_offset + 7],
                data[entry_offset + 8],
                data[entry_offset + 9],
            ];
            let by_retry = data[entry_offset + 10] as usize;
            let pda = data[entry_offset + 11];
            let mut reserved = [0u8; 4];
            reserved.copy_from_slice(&data[entry_offset + 12..entry_offset + 16]);

            let sector_size = 128usize << n;
            let total_size = sector_size * (1 + by_retry);

            if data_offset + total_size > data.len() {
                return Err(NfdError::DataTruncated {
                    track: track_idx,
                    offset: data_offset,
                });
            }

            if !media_type_detected {
                media_type = media_type_from_pda(pda, n)?;
                media_type_detected = true;
            }

            let sector_data = data[data_offset..data_offset + sector_size].to_vec();
            let sector_data_offset = data_offset;
            let mut retry_data = Vec::with_capacity(by_retry);
            for copy in 1..=by_retry {
                let start = data_offset + copy * sector_size;
                retry_data.push(data[start..start + sector_size].to_vec());
            }
            data_offset += total_size;

            sectors.push(D88Sector {
                cylinder: c,
                head: h,
                record: r,
                size_code: n,
                sector_count: sector_count as u16,
                mfm_flag: if fl_mfm == 0 { 0x40 } else { 0x00 },
                deleted: fl_ddam,
                status: by_status,
                reserved: [0u8; 5],
                data: sector_data,
                source_offset: Some(sector_data_offset as u64),
            });
            sector_extras.push(NfdR1SectorExtra {
                mfm_byte: fl_mfm,
                fdc_status,
                pda,
                reserved,
                retry_data,
            });
        }

        // Diagnostic ("special read") entries and their payloads.
        let mut diag = Vec::with_capacity(diag_count);
        for i in 0..diag_count {
            let diag_offset = entries_end + i * DIAG_ENTRY_SIZE;
            let mut entry = [0u8; DIAG_ENTRY_SIZE];
            entry.copy_from_slice(&data[diag_offset..diag_offset + DIAG_ENTRY_SIZE]);
            let by_retry = entry[9] as usize;
            let dw_data_len =
                u32::from_le_bytes([entry[10], entry[11], entry[12], entry[13]]) as usize;
            let total_diag_size = dw_data_len * (1 + by_retry);

            if data_offset + total_diag_size > data.len() {
                return Err(NfdError::DataTruncated {
                    track: track_idx,
                    offset: data_offset,
                });
            }

            let payload = data[data_offset..data_offset + total_diag_size].to_vec();
            data_offset += total_diag_size;
            diag.push(NfdDiagExtra { entry, payload });
        }

        expected_metadata_offset +=
            R1_TRACK_HEADER_SIZE + sector_count * SECTOR_ENTRY_SIZE + diag_count * DIAG_ENTRY_SIZE;

        if !sectors.is_empty() {
            track_sectors[track_idx] = Some(sectors);
        }
        track_extras[track_idx] = Some(NfdR1TrackExtra {
            header_reserved,
            sectors: sector_extras,
            diag,
        });
    }

    if expected_metadata_offset != head_size {
        canonical_layout = false;
    }

    let trailing = data[data_offset..].to_vec();

    let extra = Box::new(NfdExtra {
        magic_pad: data[0x0F],
        comment: read_comment(data),
        protect_byte: data[0x114],
        head_count: data[0x115],
        header_reserved: read_header_reserved(data),
        trailing,
        revision: NfdRevisionExtra::R1(NfdR1Extra {
            header_gap,
            additional_info_offset,
            canonical_layout,
            tracks: track_extras,
        }),
    });

    let disk = D88Disk::from_tracks(String::new(), write_protected, media_type, track_sectors);
    Ok((disk, extra))
}

/// Writes the common NFD header fields shared by both revisions. The caller
/// writes the revision magic before calling this.
fn write_common_header(
    out: &mut [u8],
    extra: Option<&NfdExtra>,
    write_protected: bool,
    head_size: u32,
) {
    if let Some(extra) = extra {
        out[0x0F] = extra.magic_pad;
        out[0x10..0x110].copy_from_slice(extra.comment.as_ref());
        out[0x115] = extra.head_count;
        out[0x116..0x120].copy_from_slice(&extra.header_reserved);
    }
    out[0x110..0x114].copy_from_slice(&head_size.to_le_bytes());

    // Preserve the raw write-protect byte when it agrees with the flag,
    // otherwise emit the canonical value.
    out[0x114] = match extra {
        Some(extra) if (extra.protect_byte != 0) == write_protected => extra.protect_byte,
        _ if write_protected => 0x10,
        _ => 0x00,
    };
}

/// Serializes a `D88Disk` into NFD R0 bytes. When `extra` carries the source
/// R0 metadata, the output reproduces the original byte-for-byte for an
/// unchanged image. Tracks with more than 26 sectors are truncated (R0 cannot
/// represent them) and a warning is logged.
pub fn to_bytes_r0(disk: &D88Disk, extra: Option<&NfdExtra>) -> Vec<u8> {
    let r0_extra = match extra.map(|extra| &extra.revision) {
        Some(NfdRevisionExtra::R0(r0)) => Some(r0),
        _ => None,
    };
    let header_tail: &[u8] = r0_extra.map(|r0| r0.header_tail.as_slice()).unwrap_or(&[]);
    let head_size = R0_SECTOR_MAP_END + header_tail.len();
    let media_pda = pda_from_media_type(disk.media_type);
    let mut warned_truncate = false;

    let mut data_size = 0usize;
    for track in 0..R0_TRACK_MAX {
        let emit = disk.sector_count(track).min(SECTORS_PER_TRACK);
        for slot in 0..emit {
            if let Some(sector) = disk.sector_at_index(track, slot) {
                data_size += sector.data.len();
            }
        }
    }

    let trailing: &[u8] = extra.map(|extra| extra.trailing.as_slice()).unwrap_or(&[]);
    let mut out = vec![0u8; head_size + data_size + trailing.len()];

    out[..15].copy_from_slice(NFD_R0_MAGIC);
    write_common_header(&mut out, extra, disk.write_protected, head_size as u32);
    out[R0_SECTOR_MAP_END..head_size].copy_from_slice(header_tail);

    let mut data_offset = head_size;
    for track in 0..R0_TRACK_MAX {
        let count = disk.sector_count(track);
        if count > SECTORS_PER_TRACK && !warned_truncate {
            warn!(
                "NFD R0 serializer: track {track} has {count} sectors; \
                 R0 supports at most {SECTORS_PER_TRACK} per track, truncating."
            );
            warned_truncate = true;
        }
        let emit = count.min(SECTORS_PER_TRACK);
        let track_extra = r0_extra.and_then(|r0| r0.tracks.get(track));

        for slot in 0..SECTORS_PER_TRACK {
            let entry_offset =
                COMMON_HEADER_SIZE + (track * SECTORS_PER_TRACK + slot) * SECTOR_ENTRY_SIZE;

            if slot >= emit {
                out[entry_offset..entry_offset + R0_EMPTY_ENTRY_FILL].fill(0xFF);
                continue;
            }
            let Some(sector) = disk.sector_at_index(track, slot) else {
                out[entry_offset..entry_offset + R0_EMPTY_ENTRY_FILL].fill(0xFF);
                continue;
            };

            out[entry_offset] = sector.cylinder;
            out[entry_offset + 1] = sector.head;
            out[entry_offset + 2] = sector.record;
            out[entry_offset + 3] = sector.size_code;

            let sector_extra = track_extra.and_then(|track| track.get(slot));
            out[entry_offset + 4] = sector_extra.map(|extra| extra.mfm_byte).unwrap_or(
                if sector.mfm_flag & 0x40 != 0 {
                    0x00
                } else {
                    0x01
                },
            );
            out[entry_offset + 5] = sector.deleted;
            out[entry_offset + 6] = sector.status;
            if let Some(sector_extra) = sector_extra {
                out[entry_offset + 7] = sector_extra.fdc_status[0];
                out[entry_offset + 8] = sector_extra.fdc_status[1];
                out[entry_offset + 9] = sector_extra.fdc_status[2];
                out[entry_offset + 10] = sector_extra.pda;
                out[entry_offset + 11..entry_offset + 16].copy_from_slice(&sector_extra.reserved);
            } else {
                out[entry_offset + 10] = media_pda;
            }

            let sector_data_size = sector.data.len();
            out[data_offset..data_offset + sector_data_size].copy_from_slice(&sector.data);
            data_offset += sector_data_size;
        }
    }

    out[data_offset..data_offset + trailing.len()].copy_from_slice(trailing);
    out
}

/// Serializes a `D88Disk` into NFD R1 bytes. When `extra` carries the source
/// R1 metadata, the output reproduces the original byte-for-byte for an
/// unchanged image, including per-sector retry copies and diagnostic entries.
pub fn to_bytes_r1(disk: &D88Disk, extra: Option<&NfdExtra>) -> Vec<u8> {
    let r1_extra = match extra.map(|extra| &extra.revision) {
        Some(NfdRevisionExtra::R1(r1)) => Some(r1),
        _ => None,
    };
    let media_pda = pda_from_media_type(disk.media_type);
    let header_gap: &[u8] = r1_extra.map(|r1| r1.header_gap.as_slice()).unwrap_or(&[]);

    let track_extra_at = |track: usize| -> Option<&NfdR1TrackExtra> {
        r1_extra
            .and_then(|r1| r1.tracks.get(track))
            .and_then(|track| track.as_ref())
    };

    // A track emits a metadata block when it has sectors or a recorded block.
    let mut emit_block = [false; R1_TRACK_MAX];
    for (track, emit) in emit_block.iter_mut().enumerate() {
        *emit = disk.sector_count(track) > 0 || track_extra_at(track).is_some();
    }

    // Metadata section: fixed header + gap + per-track blocks.
    let mut block_size = [0usize; R1_TRACK_MAX];
    let mut metadata_section_size = R1_FIXED_HEADER_END + header_gap.len();
    for track in 0..R1_TRACK_MAX {
        if !emit_block[track] {
            continue;
        }
        let sector_count = disk.sector_count(track);
        let diag_count = track_extra_at(track)
            .map(|track| track.diag.len())
            .unwrap_or(0);
        let size =
            R1_TRACK_HEADER_SIZE + sector_count * SECTOR_ENTRY_SIZE + diag_count * DIAG_ENTRY_SIZE;
        block_size[track] = size;
        metadata_section_size += size;
    }
    let head_size = metadata_section_size;

    // Data section: primary sector data with retries, then diagnostic payloads.
    let mut data_size = 0usize;
    for (track, &emit) in emit_block.iter().enumerate() {
        if !emit {
            continue;
        }
        let track_extra = track_extra_at(track);
        for slot in 0..disk.sector_count(track) {
            if let Some(sector) = disk.sector_at_index(track, slot) {
                let retries = track_extra
                    .and_then(|track| track.sectors.get(slot))
                    .map(|sector| sector.retry_data.len())
                    .unwrap_or(0);
                data_size += sector.data.len() * (1 + retries);
            }
        }
        if let Some(track_extra) = track_extra {
            for diag in &track_extra.diag {
                data_size += diag.payload.len();
            }
        }
    }

    let trailing: &[u8] = extra.map(|extra| extra.trailing.as_slice()).unwrap_or(&[]);
    let mut out = vec![0u8; head_size + data_size + trailing.len()];

    out[..15].copy_from_slice(NFD_R1_MAGIC);
    write_common_header(&mut out, extra, disk.write_protected, head_size as u32);
    out[R1_FIXED_HEADER_END..R1_FIXED_HEADER_END + header_gap.len()].copy_from_slice(header_gap);

    let mut metadata_offset = R1_FIXED_HEADER_END + header_gap.len();
    let mut data_offset = head_size;

    for track in 0..R1_TRACK_MAX {
        if !emit_block[track] {
            continue;
        }
        let table_entry = COMMON_HEADER_SIZE + track * 4;
        out[table_entry..table_entry + 4].copy_from_slice(&(metadata_offset as u32).to_le_bytes());

        let sector_count = disk.sector_count(track);
        let track_extra = track_extra_at(track);
        let diag_count = track_extra.map(|track| track.diag.len()).unwrap_or(0);

        out[metadata_offset..metadata_offset + 2]
            .copy_from_slice(&(sector_count as u16).to_le_bytes());
        out[metadata_offset + 2..metadata_offset + 4]
            .copy_from_slice(&(diag_count as u16).to_le_bytes());
        if let Some(track_extra) = track_extra {
            out[metadata_offset + 4..metadata_offset + 16]
                .copy_from_slice(&track_extra.header_reserved);
        }

        let mut entry_offset = metadata_offset + R1_TRACK_HEADER_SIZE;
        for slot in 0..sector_count {
            let Some(sector) = disk.sector_at_index(track, slot) else {
                entry_offset += SECTOR_ENTRY_SIZE;
                continue;
            };
            out[entry_offset] = sector.cylinder;
            out[entry_offset + 1] = sector.head;
            out[entry_offset + 2] = sector.record;
            out[entry_offset + 3] = sector.size_code;

            let sector_extra = track_extra.and_then(|track| track.sectors.get(slot));
            out[entry_offset + 4] = sector_extra.map(|extra| extra.mfm_byte).unwrap_or(
                if sector.mfm_flag & 0x40 != 0 {
                    0x00
                } else {
                    0x01
                },
            );
            out[entry_offset + 5] = sector.deleted;
            out[entry_offset + 6] = sector.status;
            let retry_data: &[Vec<u8>] = sector_extra
                .map(|extra| extra.retry_data.as_slice())
                .unwrap_or(&[]);
            if let Some(sector_extra) = sector_extra {
                out[entry_offset + 7] = sector_extra.fdc_status[0];
                out[entry_offset + 8] = sector_extra.fdc_status[1];
                out[entry_offset + 9] = sector_extra.fdc_status[2];
                out[entry_offset + 10] = retry_data.len() as u8;
                out[entry_offset + 11] = sector_extra.pda;
                out[entry_offset + 12..entry_offset + 16].copy_from_slice(&sector_extra.reserved);
            } else {
                out[entry_offset + 11] = media_pda;
            }
            entry_offset += SECTOR_ENTRY_SIZE;

            let sector_data_size = sector.data.len();
            out[data_offset..data_offset + sector_data_size].copy_from_slice(&sector.data);
            data_offset += sector_data_size;
            for copy in retry_data {
                let len = copy.len().min(sector_data_size);
                out[data_offset..data_offset + len].copy_from_slice(&copy[..len]);
                data_offset += sector_data_size;
            }
        }

        if let Some(track_extra) = track_extra {
            for diag in &track_extra.diag {
                out[entry_offset..entry_offset + DIAG_ENTRY_SIZE].copy_from_slice(&diag.entry);
                entry_offset += DIAG_ENTRY_SIZE;
                out[data_offset..data_offset + diag.payload.len()].copy_from_slice(&diag.payload);
                data_offset += diag.payload.len();
            }
        }

        metadata_offset += block_size[track];
    }

    out[data_offset..data_offset + trailing.len()].copy_from_slice(trailing);
    out
}

/// Returns the reason an NFD R0 re-emit would lose data, if any.
pub(crate) fn r0_reemit_error(disk: &D88Disk) -> Option<&'static str> {
    if disk.track_slot_count() > R0_TRACK_MAX {
        return Some("NFD R0 cannot represent more than 163 tracks");
    }
    for track in 0..disk.track_slot_count() {
        if disk.sector_count(track) > SECTORS_PER_TRACK {
            return Some("NFD R0 cannot represent more than 26 sectors per track");
        }
    }
    None
}

/// Returns the reason an NFD R1 re-emit would lose data, if any.
pub(crate) fn r1_reemit_error(disk: &D88Disk, extra: Option<&NfdExtra>) -> Option<&'static str> {
    if disk.track_slot_count() > R1_TRACK_MAX {
        return Some("NFD R1 cannot represent more than 164 tracks");
    }
    if let Some(NfdRevisionExtra::R1(r1)) = extra.map(|extra| &extra.revision) {
        if !r1.canonical_layout {
            return Some(
                "NFD R1 source layout is non-canonical and cannot be re-emitted losslessly",
            );
        }
        if r1.additional_info_offset != 0 {
            return Some("NFD R1 additional-info block cannot be relocated on re-emit");
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_too_small() {
        assert!(matches!(from_bytes(&[0; 100]), Err(NfdError::TooSmall)));
    }

    #[test]
    fn reject_invalid_magic() {
        let data = vec![0u8; COMMON_HEADER_SIZE];
        assert!(matches!(from_bytes(&data), Err(NfdError::InvalidMagic)));
    }

    #[test]
    fn reject_truncated_header() {
        let mut data = vec![0u8; COMMON_HEADER_SIZE];
        data[..15].copy_from_slice(NFD_R0_MAGIC);
        // Set dwHeadSize larger than file.
        data[0x110..0x114].copy_from_slice(&0x00FF_FFFFu32.to_le_bytes());
        assert!(matches!(
            from_bytes(&data),
            Err(NfdError::HeaderTruncated { .. })
        ));
    }

    /// Builds a minimal NFD R0 image with a single 256-byte sector at
    /// track 0, slot 0 (C=0 H=0 R=1 N=1) carrying a fill pattern.
    fn build_minimal_r0(fill: u8) -> Vec<u8> {
        let header_size = R0_SECTOR_MAP_END;
        let mut out = vec![0u8; header_size + 256];

        out[..15].copy_from_slice(NFD_R0_MAGIC);
        out[0x110..0x114].copy_from_slice(&(header_size as u32).to_le_bytes());

        // Track 0, slot 0 entry.
        let entry = COMMON_HEADER_SIZE;
        out[entry] = 0; // C
        out[entry + 1] = 0; // H
        out[entry + 2] = 1; // R
        out[entry + 3] = 1; // N (256 bytes)
        // fl_mfm = 0 means FM; matches D88 mfm_flag = 0x40.
        out[entry + 4] = 0;
        out[entry + 10] = 0x90; // PDA = 2HD

        // Fill all other slots with 0xFF (empty).
        for track in 0..R0_TRACK_MAX {
            for slot in 0..SECTORS_PER_TRACK {
                if track == 0 && slot == 0 {
                    continue;
                }
                let off =
                    COMMON_HEADER_SIZE + (track * SECTORS_PER_TRACK + slot) * SECTOR_ENTRY_SIZE;
                out[off..off + R0_EMPTY_ENTRY_FILL].fill(0xFF);
            }
        }

        // Sector data after header.
        for byte in &mut out[header_size..] {
            *byte = fill;
        }

        out
    }

    #[test]
    fn r0_roundtrip_unchanged() {
        let original = build_minimal_r0(0xAB);
        let (disk, extra) = from_bytes(&original).unwrap();
        assert_eq!(extra.revision(), NfdRevision::R0);
        let serialized = to_bytes_r0(&disk, Some(&extra));
        assert_eq!(serialized.len(), original.len());
        assert_eq!(serialized, original);
    }

    #[test]
    fn r0_roundtrip_preserves_full_metadata() {
        // Real files carry a 16-byte Reserve3 tail (dwHeadSize = 68,112),
        // byHead = 2, a comment, FDC status, PDA 0x30, entry reserved bytes,
        // and trailing bytes. All of these must survive a roundtrip.
        let header_size = R0_SECTOR_MAP_END + 16;
        let trailing = [0x11u8, 0x22, 0x33, 0x44];
        let mut original = vec![0u8; header_size + 256 + trailing.len()];

        original[..15].copy_from_slice(NFD_R0_MAGIC);
        original[0x0F] = 0x5A;
        original[0x10..0x1A].copy_from_slice(b"HELLO-NFD!");
        original[0x110..0x114].copy_from_slice(&(header_size as u32).to_le_bytes());
        original[0x114] = 0x10; // write protected
        original[0x115] = 0x02; // byHead
        original[0x116] = 0x77; // reserved header byte
        original[0x10A00 + 3] = 0x99; // Reserve3 tail byte

        let entry = COMMON_HEADER_SIZE;
        original[entry] = 0;
        original[entry + 1] = 0;
        original[entry + 2] = 1;
        original[entry + 3] = 1;
        original[entry + 4] = 0; // FM
        original[entry + 5] = 0; // DDAM
        original[entry + 6] = 0x00; // status
        original[entry + 7] = 0xAA; // ST0
        original[entry + 8] = 0xBB; // ST1
        original[entry + 9] = 0xCC; // ST2
        original[entry + 10] = 0x30; // PDA (must survive, not regenerate to 0x90)
        original[entry + 11] = 0xDD; // reserved
        original[entry + 15] = 0xEE; // reserved

        for track in 0..R0_TRACK_MAX {
            for slot in 0..SECTORS_PER_TRACK {
                if track == 0 && slot == 0 {
                    continue;
                }
                let off =
                    COMMON_HEADER_SIZE + (track * SECTORS_PER_TRACK + slot) * SECTOR_ENTRY_SIZE;
                original[off..off + R0_EMPTY_ENTRY_FILL].fill(0xFF);
            }
        }

        original[header_size..header_size + 256].fill(0xA5);
        original[header_size + 256..].copy_from_slice(&trailing);

        let (disk, extra) = from_bytes(&original).unwrap();
        assert!(disk.write_protected);
        let serialized = to_bytes_r0(&disk, Some(&extra));
        assert_eq!(serialized, original);
    }

    #[test]
    fn r0_after_sector_mutation() {
        let original = build_minimal_r0(0xAB);
        let (mut disk, extra) = from_bytes(&original).unwrap();

        let sector = disk.find_sector_on_track_index_mut(0, 0, 0, 1, 1).unwrap();
        sector.data.fill(0x77);

        let serialized = to_bytes_r0(&disk, Some(&extra));
        let (reparsed, _) = from_bytes(&serialized).unwrap();
        let s = reparsed.find_sector(0, 0, 1, 1).unwrap();
        assert!(s.data.iter().all(|&b| b == 0x77));
    }

    #[test]
    fn r0_reemit_gated_above_26_sectors() {
        let original = build_minimal_r0(0xAB);
        let (mut disk, _) = from_bytes(&original).unwrap();
        let chrn: Vec<(u8, u8, u8, u8)> = (1..=27u8).map(|r| (0, 0, r, 1)).collect();
        disk.format_track(0, &chrn, 1, 0xE5);
        assert!(r0_reemit_error(&disk).is_some());
    }

    /// Builds a minimal NFD R1 image with two sectors on track 0
    /// (C=0 H=0 R=1, R=2; both 256 bytes; no diag/retry).
    fn build_minimal_r1(fill1: u8, fill2: u8) -> Vec<u8> {
        let track_metadata_size = R1_TRACK_HEADER_SIZE + 2 * SECTOR_ENTRY_SIZE;
        let header_section_size = R1_FIXED_HEADER_END + track_metadata_size;
        let total = header_section_size + 2 * 256;
        let mut out = vec![0u8; total];

        out[..15].copy_from_slice(NFD_R1_MAGIC);
        out[0x110..0x114].copy_from_slice(&(header_section_size as u32).to_le_bytes());

        let track_meta_offset = R1_FIXED_HEADER_END;
        out[COMMON_HEADER_SIZE..COMMON_HEADER_SIZE + 4]
            .copy_from_slice(&(track_meta_offset as u32).to_le_bytes());

        out[track_meta_offset..track_meta_offset + 2].copy_from_slice(&2u16.to_le_bytes());

        let entry0 = track_meta_offset + R1_TRACK_HEADER_SIZE;
        out[entry0] = 0;
        out[entry0 + 1] = 0;
        out[entry0 + 2] = 1; // R
        out[entry0 + 3] = 1; // N
        out[entry0 + 11] = 0x90;
        let entry1 = entry0 + SECTOR_ENTRY_SIZE;
        out[entry1] = 0;
        out[entry1 + 1] = 0;
        out[entry1 + 2] = 2; // R
        out[entry1 + 3] = 1;
        out[entry1 + 11] = 0x90;

        let data1_offset = header_section_size;
        out[data1_offset..data1_offset + 256].fill(fill1);
        let data2_offset = data1_offset + 256;
        out[data2_offset..data2_offset + 256].fill(fill2);

        out
    }

    #[test]
    fn r1_roundtrip_unchanged() {
        let original = build_minimal_r1(0xAA, 0xBB);
        let (disk, extra) = from_bytes(&original).unwrap();
        assert_eq!(extra.revision(), NfdRevision::R1);
        let serialized = to_bytes_r1(&disk, Some(&extra));
        assert_eq!(serialized, original);
    }

    /// Builds an R1 image with a spec-style 16-byte header gap (dwAddInfo
    /// plus reserved), two tracks, per-sector retry copies, a diagnostic
    /// entry, and nonzero track-header reserved bytes.
    fn build_full_r1(add_info: u32) -> Vec<u8> {
        let gap = 16usize;
        // Track 0: one sector with byRetry = 2, and one diag entry.
        let track0_meta = R1_TRACK_HEADER_SIZE + SECTOR_ENTRY_SIZE + DIAG_ENTRY_SIZE;
        // Track 1: one sector, no retry, no diag.
        let track1_meta = R1_TRACK_HEADER_SIZE + SECTOR_ENTRY_SIZE;
        let head_size = R1_FIXED_HEADER_END + gap + track0_meta + track1_meta;

        // Data section: track 0 sector (primary + 2 retries) + diag payload,
        // then track 1 sector.
        let diag_payload_len = 128usize;
        let data_len = 256 * 3 + diag_payload_len + 256;
        let mut out = vec![0u8; head_size + data_len];

        out[..15].copy_from_slice(NFD_R1_MAGIC);
        out[0x10..0x18].copy_from_slice(b"FULL-R1!");
        out[0x110..0x114].copy_from_slice(&(head_size as u32).to_le_bytes());
        out[0x115] = 0x02;

        // Header gap: dwAddInfo + reserved.
        out[R1_FIXED_HEADER_END..R1_FIXED_HEADER_END + 4].copy_from_slice(&add_info.to_le_bytes());
        out[R1_FIXED_HEADER_END + 4] = 0xAB;

        let track0_offset = R1_FIXED_HEADER_END + gap;
        let track1_offset = track0_offset + track0_meta;
        out[COMMON_HEADER_SIZE..COMMON_HEADER_SIZE + 4]
            .copy_from_slice(&(track0_offset as u32).to_le_bytes());
        out[COMMON_HEADER_SIZE + 4..COMMON_HEADER_SIZE + 8]
            .copy_from_slice(&(track1_offset as u32).to_le_bytes());

        // Track 0 header: 1 sector, 1 diag, reserved bytes.
        out[track0_offset..track0_offset + 2].copy_from_slice(&1u16.to_le_bytes());
        out[track0_offset + 2..track0_offset + 4].copy_from_slice(&1u16.to_le_bytes());
        out[track0_offset + 4] = 0x11; // reserved
        out[track0_offset + 15] = 0x22; // reserved

        let entry = track0_offset + R1_TRACK_HEADER_SIZE;
        out[entry] = 0;
        out[entry + 1] = 0;
        out[entry + 2] = 1; // R
        out[entry + 3] = 1; // N
        out[entry + 7] = 0xA0; // ST0
        out[entry + 8] = 0xA1; // ST1
        out[entry + 9] = 0xA2; // ST2
        out[entry + 10] = 2; // byRetry
        out[entry + 11] = 0x90; // PDA
        out[entry + 12] = 0x33; // reserved

        // Diag entry (byRetry = 0, dwDataLen = 128).
        let diag = entry + SECTOR_ENTRY_SIZE;
        out[diag] = 0x4A; // Cmd
        out[diag + 9] = 0; // byRetry
        out[diag + 10..diag + 14].copy_from_slice(&(diag_payload_len as u32).to_le_bytes());
        out[diag + 14] = 0x90; // PDA

        // Track 1 header: 1 sector, 0 diag.
        out[track1_offset..track1_offset + 2].copy_from_slice(&1u16.to_le_bytes());
        let entry1 = track1_offset + R1_TRACK_HEADER_SIZE;
        out[entry1] = 1; // C
        out[entry1 + 1] = 0;
        out[entry1 + 2] = 1;
        out[entry1 + 3] = 1;
        out[entry1 + 11] = 0x90;

        // Data section.
        let mut offset = head_size;
        out[offset..offset + 256].fill(0x10); // track 0 sector primary
        offset += 256;
        out[offset..offset + 256].fill(0x11); // retry copy 1
        offset += 256;
        out[offset..offset + 256].fill(0x12); // retry copy 2
        offset += 256;
        out[offset..offset + diag_payload_len].fill(0x13); // diag payload
        offset += diag_payload_len;
        out[offset..offset + 256].fill(0x20); // track 1 sector

        out
    }

    #[test]
    fn r1_roundtrip_preserves_retries_and_diag() {
        let original = build_full_r1(0);
        let (disk, extra) = from_bytes(&original).unwrap();
        let serialized = to_bytes_r1(&disk, Some(&extra));
        assert_eq!(serialized, original);
    }

    #[test]
    fn r1_reemit_gated_with_additional_info() {
        let original = build_full_r1(0x1234);
        let (disk, extra) = from_bytes(&original).unwrap();
        // Untouched roundtrip is still byte-identical.
        assert_eq!(to_bytes_r1(&disk, Some(&extra)), original);
        // But re-emit after mutation is gated.
        assert!(r1_reemit_error(&disk, Some(&extra)).is_some());
    }

    #[test]
    fn r1_non_canonical_layout_is_gated() {
        // Place track 1's block before track 0's block (reverse offset order).
        let track_meta = R1_TRACK_HEADER_SIZE + SECTOR_ENTRY_SIZE;
        let head_size = R1_FIXED_HEADER_END + 2 * track_meta;
        let mut out = vec![0u8; head_size + 2 * 256];
        out[..15].copy_from_slice(NFD_R1_MAGIC);
        out[0x110..0x114].copy_from_slice(&(head_size as u32).to_le_bytes());

        let block_a = R1_FIXED_HEADER_END; // first in file
        let block_b = R1_FIXED_HEADER_END + track_meta; // second in file
        // Track 0 points at the SECOND block, track 1 at the FIRST block.
        out[COMMON_HEADER_SIZE..COMMON_HEADER_SIZE + 4]
            .copy_from_slice(&(block_b as u32).to_le_bytes());
        out[COMMON_HEADER_SIZE + 4..COMMON_HEADER_SIZE + 8]
            .copy_from_slice(&(block_a as u32).to_le_bytes());

        for block in [block_a, block_b] {
            out[block..block + 2].copy_from_slice(&1u16.to_le_bytes());
            let entry = block + R1_TRACK_HEADER_SIZE;
            out[entry + 2] = 1;
            out[entry + 3] = 1;
            out[entry + 11] = 0x90;
        }

        let (disk, extra) = from_bytes(&out).unwrap();
        if let NfdRevisionExtra::R1(r1) = &extra.revision {
            assert!(!r1.canonical_layout);
        } else {
            panic!("expected R1 extra");
        }
        assert!(r1_reemit_error(&disk, Some(&extra)).is_some());
    }

    #[test]
    fn r1_format_track_resets_track_metadata() {
        let original = build_full_r1(0);
        let (mut disk, mut extra) = from_bytes(&original).unwrap();

        // Format track 0 (which had retries and a diag entry).
        disk.format_track(0, &[(0, 0, 1, 1)], 1, 0xE5);
        extra.reset_track(0, 1, disk.media_type);

        let serialized = to_bytes_r1(&disk, Some(&extra));
        let (reparsed, reparsed_extra) = from_bytes(&serialized).unwrap();

        // Track 0 now has one plain sector, no retries, no diag.
        if let NfdRevisionExtra::R1(r1) = &reparsed_extra.revision {
            let track0 = r1.tracks[0].as_ref().unwrap();
            assert_eq!(track0.sectors.len(), 1);
            assert!(track0.sectors[0].retry_data.is_empty());
            assert!(track0.diag.is_empty());
            // Track 1 survives with its sector.
            assert!(r1.tracks[1].is_some());
        } else {
            panic!("expected R1 extra");
        }
        let s = reparsed.find_sector(0, 0, 1, 1).unwrap();
        assert!(s.data.iter().all(|&b| b == 0xE5));
    }

    #[test]
    fn r1_after_sector_mutation() {
        let original = build_minimal_r1(0xAA, 0xBB);
        let (mut disk, extra) = from_bytes(&original).unwrap();

        let sector = disk.find_sector_on_track_index_mut(0, 0, 0, 2, 1).unwrap();
        sector.data.fill(0x55);

        let serialized = to_bytes_r1(&disk, Some(&extra));
        let (reparsed, _) = from_bytes(&serialized).unwrap();

        let s1 = reparsed.find_sector(0, 0, 1, 1).unwrap();
        assert!(s1.data.iter().all(|&b| b == 0xAA));
        let s2 = reparsed.find_sector(0, 0, 2, 1).unwrap();
        assert!(s2.data.iter().all(|&b| b == 0x55));
    }
}
