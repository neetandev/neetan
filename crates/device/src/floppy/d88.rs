//! D88 floppy disk image format parser.
//!
//! D88 is the standard disk image format used by Japanese PC emulators.
//! It stores per-sector metadata (C/H/R/N, status, density) alongside
//! raw sector data, preserving copy-protection quirks and format
//! variations that flat images cannot represent.

use std::fmt;

/// D88 header size: 32 bytes name/flags + 164 track pointers × 4 bytes.
const HEADER_SIZE: usize = 0x2B0;

/// Maximum number of track entries in a D88 image.
const TRACK_MAX: usize = 164;

/// D88 sector header size.
const SECTOR_HEADER_SIZE: usize = 16;

/// D88 disk media type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum D88MediaType {
    /// 2D: 320 KB, 40 tracks, single-sided or double-sided.
    Disk2D,
    /// 2DD: 640 KB / 720 KB, 80 tracks, double-sided.
    Disk2DD,
    /// 2HD: 1.2 MB (PC-98) / 1.44 MB, 77 or 80 tracks, double-sided.
    Disk2HD,
}

/// Error type for D88 parsing.
#[derive(Debug, Clone)]
pub enum D88Error {
    /// Image data too small for header.
    TooSmall,
    /// Header disk size field doesn't match data length.
    SizeMismatch {
        /// Size from the D88 header.
        header: u32,
        /// Actual byte count of the image data.
        actual: usize,
    },
    /// Unknown media type byte.
    UnknownMediaType(u8),
    /// Sector header extends past end of image.
    TruncatedSector {
        /// Track index where truncation was detected.
        track: usize,
        /// Byte offset within the image.
        offset: usize,
    },
}

impl fmt::Display for D88Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            D88Error::TooSmall => write!(f, "D88 image too small for header"),
            D88Error::SizeMismatch { header, actual } => {
                write!(
                    f,
                    "D88 size mismatch: header says {header}, data is {actual}"
                )
            }
            D88Error::UnknownMediaType(t) => write!(f, "Unknown D88 media type: {t:#04X}"),
            D88Error::TruncatedSector { track, offset } => {
                write!(
                    f,
                    "D88 sector header truncated at track {track}, offset {offset}"
                )
            }
        }
    }
}

/// A single sector parsed from a D88 image.
#[derive(Debug, Clone)]
pub struct D88Sector {
    /// Cylinder (C register).
    pub cylinder: u8,
    /// Head (H register).
    pub head: u8,
    /// Record/sector number (R register).
    pub record: u8,
    /// Size code (N register): actual size = 128 << n.
    pub size_code: u8,
    /// Number of sectors on this track (from sector header).
    pub sector_count: u16,
    /// Recording-density flag (bit 6: 0 = double density, 0x40 = single density).
    pub mfm_flag: u8,
    /// Deleted data flag.
    pub deleted: u8,
    /// FDC result status byte.
    pub status: u8,
    /// Reserved bytes 9-13 of the D88 sector header (preserved for lossless roundtrip).
    pub reserved: [u8; 5],
    /// Sector data.
    pub data: Vec<u8>,
    /// Byte offset of this sector's data within the on-disk image.
    /// Populated by every parser (D88, HDM, NFD); used by `MountedFloppy`
    /// to seek-and-overwrite the sector in the file. `None` for sectors
    /// created by `D88Disk::format_track` until a re-emit + reparse runs.
    pub source_offset: Option<u64>,
}

/// A parsed track containing its sectors.
#[derive(Debug, Clone)]
struct D88Track {
    sectors: Vec<D88Sector>,
}

/// A parsed D88 disk image.
#[derive(Debug, Clone)]
pub struct D88Disk {
    /// Disk name from header.
    pub name: String,
    /// Write-protect flag.
    pub write_protected: bool,
    /// Media type.
    pub media_type: D88MediaType,
    /// Parsed tracks indexed by track number (cylinder*2 + head).
    tracks: Vec<Option<D88Track>>,
}

impl D88Disk {
    /// Constructs a D88Disk from pre-built sector vectors.
    ///
    /// Each entry in `track_sectors` corresponds to a track index
    /// (cylinder × 2 + head). `None` means the track is empty.
    pub fn from_tracks(
        name: String,
        write_protected: bool,
        media_type: D88MediaType,
        track_sectors: Vec<Option<Vec<D88Sector>>>,
    ) -> Self {
        let tracks = track_sectors
            .into_iter()
            .map(|opt| opt.map(|sectors| D88Track { sectors }))
            .collect();

        D88Disk {
            name,
            write_protected,
            media_type,
            tracks,
        }
    }

    /// Parses a D88 disk image from raw bytes.
    #[allow(clippy::needless_range_loop)]
    pub fn from_bytes(data: &[u8]) -> Result<Self, D88Error> {
        if data.len() < HEADER_SIZE {
            return Err(D88Error::TooSmall);
        }

        // Parse name (first 17 bytes, null-terminated).
        let name_end = data[..17].iter().position(|&b| b == 0).unwrap_or(17);
        let name = String::from_utf8_lossy(&data[..name_end]).into_owned();

        let write_protected = data[0x1A] & 0x10 != 0;

        let media_type = match data[0x1B] >> 4 {
            0x00 => D88MediaType::Disk2D,
            0x01 => D88MediaType::Disk2DD,
            0x02 => D88MediaType::Disk2HD,
            _ => return Err(D88Error::UnknownMediaType(data[0x1B])),
        };

        let disk_size = u32::from_le_bytes([data[0x1C], data[0x1D], data[0x1E], data[0x1F]]);
        if disk_size as usize != data.len() {
            return Err(D88Error::SizeMismatch {
                header: disk_size,
                actual: data.len(),
            });
        }

        // Parse track pointers.
        let mut track_offsets = [0u32; TRACK_MAX];
        for i in 0..TRACK_MAX {
            let base = 0x20 + i * 4;
            track_offsets[i] =
                u32::from_le_bytes([data[base], data[base + 1], data[base + 2], data[base + 3]]);
        }

        // Parse each track's sectors.
        let mut tracks: Vec<Option<D88Track>> = vec![None; TRACK_MAX];

        for (track_idx, &offset) in track_offsets.iter().enumerate() {
            if offset == 0 {
                continue;
            }
            let mut pos = offset as usize;
            let mut sectors = Vec::new();

            // Read sectors until we run out of data or hit the next track.
            let track_end = Self::find_track_end(&track_offsets, offset, disk_size);

            loop {
                if pos + SECTOR_HEADER_SIZE > data.len() || pos >= track_end {
                    break;
                }

                let c = data[pos];
                let h = data[pos + 1];
                let r = data[pos + 2];
                let n = data[pos + 3];
                let sector_count = u16::from_le_bytes([data[pos + 4], data[pos + 5]]);
                let mfm_flag = data[pos + 6];
                let deleted = data[pos + 7];
                let status = data[pos + 8];
                let mut reserved = [0u8; 5];
                reserved.copy_from_slice(&data[pos + 9..pos + 14]);
                let sector_data_size =
                    u16::from_le_bytes([data[pos + 0x0E], data[pos + 0x0F]]) as usize;

                let data_start = pos + SECTOR_HEADER_SIZE;
                let data_end = data_start + sector_data_size;

                if data_end > data.len() {
                    return Err(D88Error::TruncatedSector {
                        track: track_idx,
                        offset: pos,
                    });
                }

                let sector_data = data[data_start..data_end].to_vec();

                sectors.push(D88Sector {
                    cylinder: c,
                    head: h,
                    record: r,
                    size_code: n,
                    sector_count,
                    mfm_flag,
                    deleted,
                    status,
                    reserved,
                    data: sector_data,
                    source_offset: Some(data_start as u64),
                });

                pos = data_end;

                if sectors.len() >= sector_count as usize && sector_count > 0 {
                    break;
                }
            }

            if !sectors.is_empty() {
                tracks[track_idx] = Some(D88Track { sectors });
            }
        }

        Ok(D88Disk {
            name,
            write_protected,
            media_type,
            tracks,
        })
    }

    /// Finds the sector matching the given C/H/R/N on the appropriate track.
    pub fn find_sector(
        &self,
        cylinder: u8,
        head: u8,
        record: u8,
        size_code: u8,
    ) -> Option<&D88Sector> {
        let track_index = (cylinder as usize) * 2 + head as usize;
        self.find_sector_near_track_index(track_index, cylinder, head, record, size_code)
    }

    /// Finds a sector by C/H/R/N, preferring the exact physical track index.
    pub fn find_sector_on_track_index(
        &self,
        track_index: usize,
        cylinder: u8,
        head: u8,
        record: u8,
        size_code: u8,
    ) -> Option<&D88Sector> {
        let track = self.tracks.get(track_index)?.as_ref()?;
        track.sectors.iter().find(|sector| {
            sector.cylinder == cylinder
                && sector.head == head
                && sector.record == record
                && sector.size_code == size_code
        })
    }

    /// Finds a sector by C/H/R/N near the provided physical track index.
    ///
    /// Some protected dumps place logical tracks at non-canonical pointer
    /// indices. This method preserves physical-track locality by searching
    /// outward from `track_index` instead of globally.
    pub fn find_sector_near_track_index(
        &self,
        track_index: usize,
        cylinder: u8,
        head: u8,
        record: u8,
        size_code: u8,
    ) -> Option<&D88Sector> {
        if track_index >= self.tracks.len() {
            return None;
        }
        if let Some(sector) =
            self.find_sector_on_track_index(track_index, cylinder, head, record, size_code)
        {
            return Some(sector);
        }

        for distance in 1..self.tracks.len() {
            if let Some(upper) = track_index.checked_add(distance)
                && upper < self.tracks.len()
                && let Some(sector) =
                    self.find_sector_on_track_index(upper, cylinder, head, record, size_code)
            {
                return Some(sector);
            }
            if let Some(lower) = track_index.checked_sub(distance)
                && let Some(sector) =
                    self.find_sector_on_track_index(lower, cylinder, head, record, size_code)
            {
                return Some(sector);
            }
        }

        None
    }

    /// Finds a sector by C/H/R on the given track index, ignoring the size code.
    pub fn find_sector_id_on_track_index(
        &self,
        track_index: usize,
        cylinder: u8,
        head: u8,
        record: u8,
    ) -> Option<&D88Sector> {
        let track = self.tracks.get(track_index)?.as_ref()?;
        track.sectors.iter().find(|sector| {
            sector.cylinder == cylinder && sector.head == head && sector.record == record
        })
    }

    /// Finds a sector by C/H/R near the provided physical track index, ignoring
    /// the size code. Mirrors [`Self::find_sector_near_track_index`] but matches
    /// on the sector ID alone.
    pub fn find_sector_id_near_track_index(
        &self,
        track_index: usize,
        cylinder: u8,
        head: u8,
        record: u8,
    ) -> Option<&D88Sector> {
        if track_index >= self.tracks.len() {
            return None;
        }
        if let Some(sector) =
            self.find_sector_id_on_track_index(track_index, cylinder, head, record)
        {
            return Some(sector);
        }
        for distance in 1..self.tracks.len() {
            if let Some(upper) = track_index.checked_add(distance)
                && upper < self.tracks.len()
                && let Some(sector) =
                    self.find_sector_id_on_track_index(upper, cylinder, head, record)
            {
                return Some(sector);
            }
            if let Some(lower) = track_index.checked_sub(distance)
                && let Some(sector) =
                    self.find_sector_id_on_track_index(lower, cylinder, head, record)
            {
                return Some(sector);
            }
        }
        None
    }

    /// Returns the sector at the given rotational index on the specified track.
    pub fn sector_at_index(&self, track_index: usize, sector_index: usize) -> Option<&D88Sector> {
        let track = self.tracks.get(track_index)?.as_ref()?;
        if track.sectors.is_empty() {
            return None;
        }
        let idx = sector_index % track.sectors.len();
        Some(&track.sectors[idx])
    }

    /// Returns a mutable reference to the sector at the given rotational index
    /// on the specified track.
    pub fn sector_at_index_mut(
        &mut self,
        track_index: usize,
        sector_index: usize,
    ) -> Option<&mut D88Sector> {
        let track = self.tracks.get_mut(track_index)?.as_mut()?;
        if track.sectors.is_empty() {
            return None;
        }
        let idx = sector_index % track.sectors.len();
        Some(&mut track.sectors[idx])
    }

    /// Locates a sector by C/H/R/N near `track_index`, returning its resolved
    /// track index and rotational slot within that track.
    pub fn find_sector_slot_near_track_index(
        &self,
        track_index: usize,
        cylinder: u8,
        head: u8,
        record: u8,
        size_code: u8,
    ) -> Option<(usize, usize)> {
        let target =
            self.find_target_track_index(track_index, cylinder, head, record, size_code)?;
        let track = self.tracks.get(target)?.as_ref()?;
        let slot = track.sectors.iter().position(|sector| {
            sector.cylinder == cylinder
                && sector.head == head
                && sector.record == record
                && sector.size_code == size_code
        })?;
        Some((target, slot))
    }

    /// Finds the sector matching C/H/R/N on the given track index, returning a mutable reference.
    pub fn find_sector_on_track_index_mut(
        &mut self,
        track_index: usize,
        cylinder: u8,
        head: u8,
        record: u8,
        size_code: u8,
    ) -> Option<&mut D88Sector> {
        let track = self.tracks.get_mut(track_index)?.as_mut()?;
        track.sectors.iter_mut().find(|sector| {
            sector.cylinder == cylinder
                && sector.head == head
                && sector.record == record
                && sector.size_code == size_code
        })
    }

    /// Finds a sector by C/H/R/N near the provided physical track index,
    /// returning a mutable reference. Uses a two-pass approach: first locates
    /// the target track index immutably, then gets a mutable reference.
    pub fn find_sector_near_track_index_mut(
        &mut self,
        track_index: usize,
        cylinder: u8,
        head: u8,
        record: u8,
        size_code: u8,
    ) -> Option<&mut D88Sector> {
        let target =
            self.find_target_track_index(track_index, cylinder, head, record, size_code)?;
        self.find_sector_on_track_index_mut(target, cylinder, head, record, size_code)
    }

    /// Locates which track index holds a sector matching C/H/R/N,
    /// searching outward from `track_index`.
    fn find_target_track_index(
        &self,
        track_index: usize,
        cylinder: u8,
        head: u8,
        record: u8,
        size_code: u8,
    ) -> Option<usize> {
        if track_index >= self.tracks.len() {
            return None;
        }
        if self
            .find_sector_on_track_index(track_index, cylinder, head, record, size_code)
            .is_some()
        {
            return Some(track_index);
        }
        for distance in 1..self.tracks.len() {
            if let Some(upper) = track_index.checked_add(distance)
                && upper < self.tracks.len()
                && self
                    .find_sector_on_track_index(upper, cylinder, head, record, size_code)
                    .is_some()
            {
                return Some(upper);
            }
            if let Some(lower) = track_index.checked_sub(distance)
                && self
                    .find_sector_on_track_index(lower, cylinder, head, record, size_code)
                    .is_some()
            {
                return Some(lower);
            }
        }
        None
    }

    /// Formats a track by replacing its sectors with new ones.
    ///
    /// Each entry in `chrn` is a `(cylinder, head, record, size_code)` tuple
    /// describing one sector. The data area of each sector is filled with
    /// `fill_byte`.
    pub fn format_track(
        &mut self,
        track_index: usize,
        chrn: &[(u8, u8, u8, u8)],
        data_n: u8,
        fill_byte: u8,
    ) {
        if track_index >= self.tracks.len() {
            self.tracks.resize_with(track_index + 1, || None);
        }
        let sector_size = 128usize << (data_n as usize).min(7);
        let sector_count = chrn.len() as u16;
        let sectors = chrn
            .iter()
            .map(|&(c, h, r, n)| D88Sector {
                cylinder: c,
                head: h,
                record: r,
                size_code: n,
                sector_count,
                mfm_flag: 0x00,
                deleted: 0x00,
                status: 0x00,
                reserved: [0u8; 5],
                data: vec![fill_byte; sector_size],
                source_offset: None,
            })
            .collect();
        self.tracks[track_index] = Some(D88Track { sectors });
    }

    /// Serializes the disk image back to the D88 binary format.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut track_blobs: Vec<Option<Vec<u8>>> = Vec::with_capacity(TRACK_MAX);
        for track_opt in &self.tracks {
            match track_opt {
                Some(track) => {
                    let mut blob = Vec::new();
                    for sector in &track.sectors {
                        let mut header = [0u8; SECTOR_HEADER_SIZE];
                        header[0] = sector.cylinder;
                        header[1] = sector.head;
                        header[2] = sector.record;
                        header[3] = sector.size_code;
                        header[4..6].copy_from_slice(&sector.sector_count.to_le_bytes());
                        header[6] = sector.mfm_flag;
                        header[7] = sector.deleted;
                        header[8] = sector.status;
                        header[9..14].copy_from_slice(&sector.reserved);
                        let data_size = sector.data.len() as u16;
                        header[0x0E..0x10].copy_from_slice(&data_size.to_le_bytes());
                        blob.extend_from_slice(&header);
                        blob.extend_from_slice(&sector.data);
                    }
                    track_blobs.push(Some(blob));
                }
                None => track_blobs.push(None),
            }
        }
        while track_blobs.len() < TRACK_MAX {
            track_blobs.push(None);
        }

        let mut offset = HEADER_SIZE as u32;
        let mut track_offsets = [0u32; TRACK_MAX];
        for (i, blob_opt) in track_blobs.iter().enumerate() {
            if let Some(blob) = blob_opt {
                track_offsets[i] = offset;
                offset += blob.len() as u32;
            }
        }
        let disk_size = offset;

        let mut image = vec![0u8; HEADER_SIZE];

        let name_bytes = self.name.as_bytes();
        let copy_len = name_bytes.len().min(16);
        image[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

        if self.write_protected {
            image[0x1A] = 0x10;
        }

        image[0x1B] = match self.media_type {
            D88MediaType::Disk2D => 0x00,
            D88MediaType::Disk2DD => 0x10,
            D88MediaType::Disk2HD => 0x20,
        };

        image[0x1C..0x20].copy_from_slice(&disk_size.to_le_bytes());

        for (i, &off) in track_offsets.iter().enumerate() {
            let base = 0x20 + i * 4;
            image[base..base + 4].copy_from_slice(&off.to_le_bytes());
        }

        for blob in track_blobs.iter().flatten() {
            image.extend_from_slice(blob);
        }

        image
    }

    /// Returns the total number of track slots (cylinder * 2 + head).
    pub fn track_slot_count(&self) -> usize {
        self.tracks
            .iter()
            .rposition(Option::is_some)
            .map_or(0, |index| index + 1)
    }

    /// Returns the number of sectors on the specified track.
    pub fn sector_count(&self, track_index: usize) -> usize {
        self.tracks
            .get(track_index)
            .and_then(|t| t.as_ref())
            .map(|t| t.sectors.len())
            .unwrap_or(0)
    }

    fn find_track_end(offsets: &[u32; TRACK_MAX], current: u32, disk_size: u32) -> usize {
        let mut end = disk_size;
        for &off in offsets {
            if off > current && off < end {
                end = off;
            }
        }
        end as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal D88 image with one track (track 0) containing
    /// the given sectors.
    fn build_test_d88(media_type: u8, sectors: &[(u8, u8, u8, u8, &[u8])]) -> Vec<u8> {
        let mut image = vec![0u8; HEADER_SIZE];

        let name = b"TEST";
        image[..name.len()].copy_from_slice(name);

        image[0x1B] = media_type;

        // Build track 0 data.
        let track_offset = HEADER_SIZE as u32;
        // Set track pointer 0.
        image[0x20..0x24].copy_from_slice(&track_offset.to_le_bytes());

        let mut track_data = Vec::new();
        for &(c, h, r, n, data) in sectors {
            // Sector header (16 bytes).
            let mut header = [0u8; SECTOR_HEADER_SIZE];
            header[0] = c;
            header[1] = h;
            header[2] = r;
            header[3] = n;
            let sc = sectors.len() as u16;
            header[4..6].copy_from_slice(&sc.to_le_bytes());
            header[6] = 0x00; // MFM flag
            header[7] = 0x00; // deleted
            header[8] = 0x00; // status (OK)
            let data_size = data.len() as u16;
            header[0x0E..0x10].copy_from_slice(&data_size.to_le_bytes());
            track_data.extend_from_slice(&header);
            track_data.extend_from_slice(data);
        }

        image.extend_from_slice(&track_data);

        let disk_size = image.len() as u32;
        image[0x1C..0x20].copy_from_slice(&disk_size.to_le_bytes());

        image
    }

    /// Builds a minimal D88 image where one logical track is stored at an
    /// arbitrary track pointer index.
    fn build_test_d88_with_track_index(
        media_type: u8,
        track_index: usize,
        sectors: &[(u8, u8, u8, u8, &[u8])],
    ) -> Vec<u8> {
        assert!(track_index < TRACK_MAX);
        let mut image = vec![0u8; HEADER_SIZE];

        let name = b"TEST";
        image[..name.len()].copy_from_slice(name);
        image[0x1B] = media_type;

        let track_offset = HEADER_SIZE as u32;
        let pointer_base = 0x20 + track_index * 4;
        image[pointer_base..pointer_base + 4].copy_from_slice(&track_offset.to_le_bytes());

        let mut track_data = Vec::new();
        for &(cylinder, head, record, size_code, data) in sectors {
            let mut header = [0u8; SECTOR_HEADER_SIZE];
            header[0] = cylinder;
            header[1] = head;
            header[2] = record;
            header[3] = size_code;
            let sector_count = sectors.len() as u16;
            header[4..6].copy_from_slice(&sector_count.to_le_bytes());
            header[6] = 0x00;
            header[7] = 0x00;
            header[8] = 0x00;
            let data_size = data.len() as u16;
            header[0x0E..0x10].copy_from_slice(&data_size.to_le_bytes());
            track_data.extend_from_slice(&header);
            track_data.extend_from_slice(data);
        }

        image.extend_from_slice(&track_data);
        let disk_size = image.len() as u32;
        image[0x1C..0x20].copy_from_slice(&disk_size.to_le_bytes());
        image
    }

    #[test]
    fn parse_minimal_2hd_image() {
        let sector_data = vec![0xAA; 1024];
        let image = build_test_d88(0x20, &[(0, 0, 1, 3, &sector_data)]);
        let disk = D88Disk::from_bytes(&image).unwrap();

        assert_eq!(disk.name, "TEST");
        assert_eq!(disk.media_type, D88MediaType::Disk2HD);
        assert!(!disk.write_protected);
    }

    #[test]
    fn find_sector_by_chrs() {
        let data1 = vec![0x11; 512];
        let data2 = vec![0x22; 512];
        let data3 = vec![0x33; 512];
        let image = build_test_d88(
            0x20,
            &[
                (0, 0, 1, 2, &data1),
                (0, 0, 2, 2, &data2),
                (0, 0, 3, 2, &data3),
            ],
        );
        let disk = D88Disk::from_bytes(&image).unwrap();

        let s = disk.find_sector(0, 0, 2, 2).unwrap();
        assert_eq!(s.record, 2);
        assert_eq!(s.data[0], 0x22);

        assert!(disk.find_sector(0, 0, 4, 2).is_none());
    }

    #[test]
    fn find_sector_falls_back_for_non_canonical_track_pointer_layout() {
        let sector_data = vec![0x5A; 1024];
        let image =
            build_test_d88_with_track_index(0x20, 5, &[(0, 1, 1, 3, sector_data.as_slice())]);
        let disk = D88Disk::from_bytes(&image).unwrap();

        // Canonical index for C=0,H=1 is 1, but this image stores the track
        // at pointer index 5 to emulate non-canonical/protected layouts.
        assert_eq!(disk.sector_count(1), 0);
        assert_eq!(disk.sector_count(5), 1);

        let sector = disk.find_sector(0, 1, 1, 3).unwrap();
        assert_eq!(sector.cylinder, 0);
        assert_eq!(sector.head, 1);
        assert_eq!(sector.record, 1);
        assert_eq!(sector.size_code, 3);
        assert_eq!(sector.data[0], 0x5A);
    }

    #[test]
    fn find_sector_near_track_index_preserves_duplicate_track_variant() {
        let mut image = vec![0u8; HEADER_SIZE];
        image[..4].copy_from_slice(b"TEST");
        image[0x1B] = 0x20; // 2HD

        let append_single_sector_track =
            |image: &mut Vec<u8>, pointer_index: usize, fill: u8| -> usize {
                let track_offset = image.len() as u32;
                let pointer_base = 0x20 + pointer_index * 4;
                image[pointer_base..pointer_base + 4].copy_from_slice(&track_offset.to_le_bytes());

                let mut header = [0u8; SECTOR_HEADER_SIZE];
                header[0] = 0; // C
                header[1] = 1; // H
                header[2] = 1; // R
                header[3] = 3; // N (1024 bytes)
                header[4..6].copy_from_slice(&1u16.to_le_bytes());
                header[0x0E..0x10].copy_from_slice(&1024u16.to_le_bytes());

                image.extend_from_slice(&header);
                image.extend_from_slice(&vec![fill; 1024]);
                track_offset as usize
            };

        let _track2_offset = append_single_sector_track(&mut image, 2, 0xAA);
        let _track3_offset = append_single_sector_track(&mut image, 3, 0xBB);

        let disk_size = image.len() as u32;
        image[0x1C..0x20].copy_from_slice(&disk_size.to_le_bytes());

        let disk = D88Disk::from_bytes(&image).unwrap();

        assert_eq!(
            disk.find_sector_on_track_index(2, 0, 1, 1, 3).unwrap().data[0],
            0xAA
        );
        assert_eq!(
            disk.find_sector_on_track_index(3, 0, 1, 1, 3).unwrap().data[0],
            0xBB
        );

        assert_eq!(
            disk.find_sector_near_track_index(2, 0, 1, 1, 3)
                .unwrap()
                .data[0],
            0xAA
        );
        assert_eq!(
            disk.find_sector_near_track_index(3, 0, 1, 1, 3)
                .unwrap()
                .data[0],
            0xBB
        );
    }

    #[test]
    fn sector_at_index_wraps() {
        let data = vec![0xBB; 256];
        let image = build_test_d88(
            0x20,
            &[
                (0, 0, 1, 1, &data),
                (0, 0, 2, 1, &data),
                (0, 0, 3, 1, &data),
            ],
        );
        let disk = D88Disk::from_bytes(&image).unwrap();

        // Track 0 = cylinder 0, head 0 -> index 0.
        assert_eq!(disk.sector_count(0), 3);
        assert_eq!(disk.sector_at_index(0, 0).unwrap().record, 1);
        assert_eq!(disk.sector_at_index(0, 1).unwrap().record, 2);
        assert_eq!(disk.sector_at_index(0, 2).unwrap().record, 3);
        // Wraps around.
        assert_eq!(disk.sector_at_index(0, 3).unwrap().record, 1);
    }

    #[test]
    fn empty_track_returns_none() {
        let data = vec![0xCC; 512];
        let image = build_test_d88(0x20, &[(0, 0, 1, 2, &data)]);
        let disk = D88Disk::from_bytes(&image).unwrap();

        // Track 1 has no data.
        assert_eq!(disk.sector_count(1), 0);
        assert!(disk.sector_at_index(1, 0).is_none());
    }

    #[test]
    fn too_small_image_rejected() {
        let image = vec![0u8; 100];
        assert!(matches!(
            D88Disk::from_bytes(&image),
            Err(D88Error::TooSmall)
        ));
    }

    #[test]
    fn size_mismatch_rejected() {
        let mut image = vec![0u8; HEADER_SIZE];
        image[0x1B] = 0x20;
        // Set disk size to something different from actual size.
        let wrong_size = 9999u32;
        image[0x1C..0x20].copy_from_slice(&wrong_size.to_le_bytes());
        assert!(matches!(
            D88Disk::from_bytes(&image),
            Err(D88Error::SizeMismatch { .. })
        ));
    }

    #[test]
    fn write_protect_flag() {
        let image = build_test_d88(0x20, &[(0, 0, 1, 2, &[0; 512])]);
        let disk = D88Disk::from_bytes(&image).unwrap();
        assert!(!disk.write_protected);

        let mut wp_image = image.clone();
        wp_image[0x1A] = 0x10;
        let disk = D88Disk::from_bytes(&wp_image).unwrap();
        assert!(disk.write_protected);
    }

    #[test]
    fn to_bytes_roundtrip() {
        let mut sectors = Vec::new();
        for r in 1..=8 {
            let mut data = vec![0u8; 1024];
            for (i, byte) in data.iter_mut().enumerate() {
                *byte = (r as usize).wrapping_add(i) as u8;
            }
            sectors.push((0u8, 0u8, r, 3u8, data));
        }
        let sector_refs: Vec<(u8, u8, u8, u8, &[u8])> = sectors
            .iter()
            .map(|(c, h, r, n, d)| (*c, *h, *r, *n, d.as_slice()))
            .collect();
        let image = build_test_d88(0x20, &sector_refs);
        let disk = D88Disk::from_bytes(&image).unwrap();

        let serialized = disk.to_bytes();
        assert_eq!(serialized, image);
    }

    #[test]
    fn find_sector_mut_modifies_data() {
        let sector_data = vec![0xAA; 1024];
        let image = build_test_d88(0x20, &[(0, 0, 1, 3, &sector_data)]);
        let mut disk = D88Disk::from_bytes(&image).unwrap();

        let sector = disk.find_sector_on_track_index_mut(0, 0, 0, 1, 3).unwrap();
        sector.data[0] = 0xBB;

        let sector = disk.find_sector(0, 0, 1, 3).unwrap();
        assert_eq!(sector.data[0], 0xBB);
    }

    #[test]
    fn find_sector_near_mut_modifies_data() {
        let sector_data = vec![0x5A; 1024];
        let image =
            build_test_d88_with_track_index(0x20, 5, &[(0, 1, 1, 3, sector_data.as_slice())]);
        let mut disk = D88Disk::from_bytes(&image).unwrap();

        let sector = disk
            .find_sector_near_track_index_mut(1, 0, 1, 1, 3)
            .unwrap();
        sector.data[0] = 0xCC;

        let sector = disk.find_sector(0, 1, 1, 3).unwrap();
        assert_eq!(sector.data[0], 0xCC);
    }

    #[test]
    fn multi_sector_boot_track() {
        // Simulate a typical PC-98 2HD boot track: 8 sectors × 1024 bytes.
        let mut sectors = Vec::new();
        for r in 1..=8 {
            let mut data = vec![0u8; 1024];
            data[0] = r;
            sectors.push((0u8, 0u8, r, 3u8, data));
        }
        let sector_refs: Vec<(u8, u8, u8, u8, &[u8])> = sectors
            .iter()
            .map(|(c, h, r, n, d)| (*c, *h, *r, *n, d.as_slice()))
            .collect();
        let image = build_test_d88(0x20, &sector_refs);
        let disk = D88Disk::from_bytes(&image).unwrap();

        assert_eq!(disk.sector_count(0), 8);
        for r in 1..=8u8 {
            let s = disk.find_sector(0, 0, r, 3).unwrap();
            assert_eq!(s.data[0], r);
        }
    }

    #[test]
    fn parser_records_source_offsets() {
        let image = build_test_d88(
            0x10,
            &[(0, 0, 1, 0, &[0xAA; 128]), (0, 0, 2, 0, &[0xBB; 128])],
        );
        let disk = D88Disk::from_bytes(&image).unwrap();

        // First sector data starts at HEADER_SIZE + SECTOR_HEADER_SIZE.
        let s1 = disk.find_sector(0, 0, 1, 0).unwrap();
        assert_eq!(
            s1.source_offset,
            Some((HEADER_SIZE + SECTOR_HEADER_SIZE) as u64)
        );
        // Second sector follows the first sector's data plus its own header.
        let s2 = disk.find_sector(0, 0, 2, 0).unwrap();
        assert_eq!(
            s2.source_offset,
            Some((HEADER_SIZE + SECTOR_HEADER_SIZE + 128 + SECTOR_HEADER_SIZE) as u64),
        );
    }

    #[test]
    fn format_track_clears_source_offsets() {
        let image = build_test_d88(0x10, &[(0, 0, 1, 0, &[0xAA; 128])]);
        let mut disk = D88Disk::from_bytes(&image).unwrap();

        disk.format_track(0, &[(0, 0, 1, 0)], 0, 0xE5);
        let sector = disk.sector_at_index(0, 0).unwrap();
        assert!(sector.source_offset.is_none());
    }
}
