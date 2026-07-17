//! Floppy disk image format parsers for FDC emulation.
//!
//! Supports these floppy image formats:
//! - **D88** (.d88/.d98/.88d/.98d): Standard D88 format with per-sector metadata.
//! - **D77** (.d77): Fujitsu FM-7 disk format; a byte-compatible D88 container.
//! - **HDM** (.hdm): Headerless raw sector format for 2HD floppies.
//! - **NFD** (.nfd): T98Next format with per-sector metadata (R0 and R1 revisions).
//! - **2D** (.2d): Headerless raw sector format for Sharp X1 2D floppies.
//! - **DIM** (.dim): X68000 DIFC.X container with a per-track saved-flag table.
//! - **XDF** (.xdf/.2hd): Headerless raw sector format for X68000 2HD floppies.
//! - **IMG** (.img/.ima): Headerless raw sector format for IBM PC floppies.
//! - **IBM XDF** (.xdf/.img, 1,884,160 bytes): IBM Extended Density Format.
//! - **MSX DSK** (.dsk): Headerless raw sector format for MSX floppies.

pub mod d88;
pub mod dim;
pub mod hdm;
pub mod ibm_xdf;
pub mod img;
pub mod msx_dsk;
pub mod nfd;
pub mod two_d;
pub mod xdf;

use std::{
    error::Error,
    fmt,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

use common::error;
pub use d88::{D88Disk, D88Error, D88MediaType, D88Sector};
pub use msx_dsk::{MsxDskError, MsxDskGeometry};
pub use nfd::NfdRevision;

use crate::disk_backend::DiskBackend;

/// The original format of a loaded floppy image (used for serialization).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloppyFormat {
    /// Standard D88 format (.d88/.d98/.88d/.98d).
    D88,
    /// Fujitsu FM-7 disk format (.d77); a byte-compatible D88 container.
    D77,
    /// Headerless raw sector format (.hdm).
    Hdm,
    /// T98Next floppy format, R0 revision (.nfd, magic `T98FDDIMAGE.R0`).
    NfdR0,
    /// T98Next floppy format, R1 revision (.nfd, magic `T98FDDIMAGE.R1`).
    NfdR1,
    /// Headerless raw 2D sector format (.2d).
    TwoD,
    /// X68000 DIFC.X container (.dim).
    Dim,
    /// Headerless raw X68000 2HD sector format (.xdf/.2hd).
    Xdf,
    /// Headerless raw IBM PC sector format (.img/.ima).
    Img,
    /// IBM Extended Density Format (.xdf/.img, 1,884,160 bytes).
    IbmXdf,
    /// Headerless raw MSX sector format (.dsk).
    MsxDsk(MsxDskGeometry),
}

/// A parsed floppy disk image.
#[derive(Debug, Clone)]
pub struct FloppyImage {
    /// The underlying D88 disk data (canonical internal representation).
    disk: D88Disk,
    /// Original image format.
    pub format: FloppyFormat,
    /// Original container header preserved for lossless re-emit (DIM only).
    container_header: Option<Box<[u8; dim::DIM_HEADER_SIZE]>>,
}

impl Deref for FloppyImage {
    type Target = D88Disk;
    fn deref(&self) -> &Self::Target {
        &self.disk
    }
}

impl DerefMut for FloppyImage {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.disk
    }
}

impl FloppyImage {
    /// Creates a `FloppyImage` from a pre-parsed `D88Disk`.
    pub fn from_d88(disk: D88Disk) -> Self {
        Self {
            disk,
            format: FloppyFormat::D88,
            container_header: None,
        }
    }

    /// Parses a D88 floppy image from raw bytes.
    pub fn from_d88_bytes(data: &[u8]) -> Result<Self, FloppyError> {
        let disk = D88Disk::from_bytes(data).map_err(FloppyError::D88)?;
        Ok(Self {
            disk,
            format: FloppyFormat::D88,
            container_header: None,
        })
    }

    /// Parses a D77 floppy image from raw bytes. D77 is the FM-7 disk format and
    /// shares the D88 container layout, so it parses through the same path while
    /// keeping its own format tag for lossless re-emit.
    pub fn from_d77_bytes(data: &[u8]) -> Result<Self, FloppyError> {
        let disk = D88Disk::from_bytes(data).map_err(FloppyError::D88)?;
        Ok(Self {
            disk,
            format: FloppyFormat::D77,
            container_header: None,
        })
    }

    /// Parses an HDM floppy image from raw bytes.
    pub fn from_hdm_bytes(data: &[u8]) -> Result<Self, FloppyError> {
        let disk = hdm::from_bytes(data).map_err(FloppyError::Hdm)?;
        Ok(Self {
            disk,
            format: FloppyFormat::Hdm,
            container_header: None,
        })
    }

    /// Parses an NFD floppy image from raw bytes.
    pub fn from_nfd_bytes(data: &[u8]) -> Result<Self, FloppyError> {
        let (disk, revision) = nfd::from_bytes(data).map_err(FloppyError::Nfd)?;
        let format = match revision {
            NfdRevision::R0 => FloppyFormat::NfdR0,
            NfdRevision::R1 => FloppyFormat::NfdR1,
        };
        Ok(Self {
            disk,
            format,
            container_header: None,
        })
    }

    /// Parses a 2D floppy image from raw bytes.
    pub fn from_2d_bytes(data: &[u8]) -> Result<Self, FloppyError> {
        let disk = two_d::from_bytes(data).map_err(FloppyError::TwoD)?;
        Ok(Self {
            disk,
            format: FloppyFormat::TwoD,
            container_header: None,
        })
    }

    /// Parses a DIM floppy image from raw bytes, keeping the container header.
    pub fn from_dim_bytes(data: &[u8]) -> Result<Self, FloppyError> {
        let (disk, header) = dim::from_bytes(data).map_err(FloppyError::Dim)?;
        Ok(Self {
            disk,
            format: FloppyFormat::Dim,
            container_header: Some(header),
        })
    }

    /// Parses an XDF floppy image from raw bytes.
    pub fn from_xdf_bytes(data: &[u8]) -> Result<Self, FloppyError> {
        let disk = xdf::from_bytes(data).map_err(FloppyError::Xdf)?;
        Ok(Self {
            disk,
            format: FloppyFormat::Xdf,
            container_header: None,
        })
    }

    /// Parses a raw IMG floppy image from raw bytes.
    pub fn from_img_bytes(data: &[u8]) -> Result<Self, FloppyError> {
        let disk = img::from_bytes(data).map_err(FloppyError::Img)?;
        Ok(Self {
            disk,
            format: FloppyFormat::Img,
            container_header: None,
        })
    }

    /// Parses an IBM XDF floppy image from raw bytes.
    pub fn from_ibm_xdf_bytes(data: &[u8]) -> Result<Self, FloppyError> {
        let disk = ibm_xdf::from_bytes(data).map_err(FloppyError::IbmXdf)?;
        Ok(Self {
            disk,
            format: FloppyFormat::IbmXdf,
            container_header: None,
        })
    }

    /// Parses a raw MSX DSK floppy image with detected geometry.
    pub fn from_msx_dsk_bytes(data: &[u8]) -> Result<Self, FloppyError> {
        let (disk, geometry) = msx_dsk::from_bytes(data).map_err(FloppyError::MsxDsk)?;
        Ok(Self {
            disk,
            format: FloppyFormat::MsxDsk(geometry),
            container_header: None,
        })
    }

    /// Parses a raw MSX DSK floppy image with explicit geometry.
    pub fn from_msx_dsk_bytes_with_geometry(
        data: &[u8],
        geometry: MsxDskGeometry,
    ) -> Result<Self, FloppyError> {
        let disk =
            msx_dsk::from_bytes_with_geometry(data, geometry).map_err(FloppyError::MsxDsk)?;
        Ok(Self {
            disk,
            format: FloppyFormat::MsxDsk(geometry),
            container_header: None,
        })
    }

    /// Returns a human-readable format name.
    pub fn format_name(&self) -> &'static str {
        match self.format {
            FloppyFormat::D88 => "D88",
            FloppyFormat::D77 => "D77",
            FloppyFormat::Hdm => "HDM",
            FloppyFormat::NfdR0 => "NFD R0",
            FloppyFormat::NfdR1 => "NFD R1",
            FloppyFormat::TwoD => "2D",
            FloppyFormat::Dim => "DIM",
            FloppyFormat::Xdf => "XDF",
            FloppyFormat::Img => "IMG",
            FloppyFormat::IbmXdf => "IBM XDF",
            FloppyFormat::MsxDsk(_) => "MSX DSK",
        }
    }

    /// Serializes the image back to its source on-disk format.
    pub fn to_bytes(&self) -> Vec<u8> {
        match self.format {
            FloppyFormat::D88 | FloppyFormat::D77 => self.disk.to_bytes(),
            FloppyFormat::Hdm => hdm::to_bytes(&self.disk),
            FloppyFormat::NfdR0 => nfd::to_bytes_r0(&self.disk),
            FloppyFormat::NfdR1 => nfd::to_bytes_r1(&self.disk),
            FloppyFormat::TwoD => two_d::to_bytes(&self.disk),
            FloppyFormat::Dim => dim::to_bytes(&self.disk, self.dim_header()),
            FloppyFormat::Xdf => xdf::to_bytes(&self.disk),
            FloppyFormat::Img => img::to_bytes(&self.disk),
            FloppyFormat::IbmXdf => ibm_xdf::to_bytes(&self.disk),
            FloppyFormat::MsxDsk(geometry) => msx_dsk::to_bytes(&self.disk, geometry),
        }
    }

    fn lossless_reemit_error(&self) -> Option<&'static str> {
        match self.format {
            FloppyFormat::D88 | FloppyFormat::D77 => None,
            FloppyFormat::Hdm => {
                if hdm::is_representable(&self.disk) {
                    None
                } else {
                    Some("HDM cannot represent the current track layout")
                }
            }
            FloppyFormat::NfdR0 | FloppyFormat::NfdR1 => {
                Some("NFD full-image serialization does not preserve all source metadata")
            }
            FloppyFormat::TwoD => {
                if two_d::is_representable(&self.disk) {
                    None
                } else {
                    Some("2D cannot represent the current track layout")
                }
            }
            FloppyFormat::Dim => {
                if dim::is_representable(&self.disk, self.dim_header()) {
                    None
                } else {
                    Some("DIM cannot represent the current track layout")
                }
            }
            FloppyFormat::Xdf => {
                if xdf::is_representable(&self.disk) {
                    None
                } else {
                    Some("XDF cannot represent the current track layout")
                }
            }
            FloppyFormat::Img => {
                if img::is_representable(&self.disk) {
                    None
                } else {
                    Some("IMG cannot represent the current track layout")
                }
            }
            FloppyFormat::IbmXdf => {
                if ibm_xdf::is_representable(&self.disk) {
                    None
                } else {
                    Some("IBM XDF cannot represent the current track layout")
                }
            }
            FloppyFormat::MsxDsk(geometry) => {
                if msx_dsk::is_representable(&self.disk, geometry) {
                    None
                } else {
                    Some("MSX DSK cannot represent the current track layout")
                }
            }
        }
    }

    /// Returns the preserved DIM header, or a blank 2HD header when absent.
    fn dim_header(&self) -> &[u8; dim::DIM_HEADER_SIZE] {
        const DEFAULT_DIM_HEADER: [u8; dim::DIM_HEADER_SIZE] = dim::blank_header();
        self.container_header
            .as_deref()
            .unwrap_or(&DEFAULT_DIM_HEADER)
    }
}

/// A floppy image bound to its source file for synchronous write-through.
#[derive(Debug)]
pub struct MountedFloppy {
    image: FloppyImage,
    backend: Option<DiskBackend>,
    dirty: bool,
    identity: save_state::ResourceIdentity,
    source_path: Option<save_state::MediaSourcePath>,
}

impl MountedFloppy {
    /// Constructs a new mount. If `path` is `None` or the file cannot be
    /// opened for write, writes only land in memory.
    pub fn new(image: FloppyImage, path: Option<PathBuf>) -> Self {
        let structure = [floppy_format_tag(image.format)];
        let source_path = path.as_deref().map(save_state::MediaSourcePath::from_path);
        let identity = match source_path.as_ref() {
            Some(source_path) => crate::media_identity::path_identity(
                "neetan-floppy-source-v1",
                source_path,
                0,
                &structure,
            ),
            None => {
                crate::media_identity::anonymous_identity("neetan-floppy-source-v1", 0, &structure)
            }
        };
        let backend = path.and_then(|p| match DiskBackend::open(p.clone()) {
            Ok(b) => Some(b),
            Err(err) => {
                error!(
                    "Failed to open floppy {} for write-through: {err}",
                    p.display()
                );
                None
            }
        });
        Self {
            image,
            backend,
            dirty: false,
            identity,
            source_path,
        }
    }

    /// Returns a read-only reference to the parsed image.
    pub fn image(&self) -> &FloppyImage {
        &self.image
    }

    /// Returns the stable identity recorded when the image was mounted.
    pub const fn identity(&self) -> save_state::ResourceIdentity {
        self.identity
    }

    /// Returns the normalized configured source path, when file-backed.
    pub const fn source_path(&self) -> Option<&save_state::MediaSourcePath> {
        self.source_path.as_ref()
    }

    /// Returns whether the in-memory image has unwritten changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Writes sector data identified by C/H/R/N near `track_index`.
    /// Returns `true` if the sector was found.
    #[allow(clippy::too_many_arguments)]
    pub fn write_sector_data(
        &mut self,
        track_index: usize,
        c: u8,
        h: u8,
        r: u8,
        n: u8,
        data: &[u8],
    ) -> bool {
        let Some(sector) = self
            .image
            .find_sector_near_track_index_mut(track_index, c, h, r, n)
        else {
            return false;
        };
        let copy_len = data.len().min(sector.data.len());
        sector.data[..copy_len].copy_from_slice(&data[..copy_len]);
        let source_offset = sector.source_offset;

        if let (Some(backend), Some(offset)) = (self.backend.as_mut(), source_offset) {
            if let Err(err) = backend.write_at(offset, &data[..copy_len]) {
                self.dirty = true;
                error!("Floppy write-through failed at offset {offset}: {err}");
            }
        } else {
            self.dirty = true;
        }
        true
    }

    /// Formats a track and re-emits the image atomically. The image is
    /// reparsed from the new bytes so per-sector source offsets stay
    /// valid for subsequent write-through.
    pub fn format_track(
        &mut self,
        track_index: usize,
        chrn: &[(u8, u8, u8, u8)],
        data_n: u8,
        fill_byte: u8,
    ) {
        self.image
            .format_track(track_index, chrn, data_n, fill_byte);
        self.dirty = true;

        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        if let Some(reason) = self.image.lossless_reemit_error() {
            error!(
                "Floppy FORMAT TRACK cannot be written back to {} without loss: {reason}",
                self.image.format_name()
            );
            return;
        }
        let bytes = self.image.to_bytes();
        if let Err(err) = backend.replace_atomic(&bytes) {
            error!("Floppy FORMAT TRACK re-emit failed: {err}");
            return;
        }
        match load_floppy_image(backend.path(), &bytes) {
            Ok(reparsed) => {
                self.image = reparsed;
                self.dirty = false;
            }
            Err(err) => error!("Floppy reparse after FORMAT TRACK failed: {err}"),
        }
    }

    /// Re-emits the entire image if dirty and reparses it so per-sector
    /// source offsets match the rewritten file. The dirty flag remains set
    /// only when a write-through or re-emit step reported an error.
    pub fn flush_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        if let Some(reason) = self.image.lossless_reemit_error() {
            error!(
                "Floppy dirty flush cannot write back {} without loss: {reason}",
                self.image.format_name()
            );
            return;
        }
        let bytes = self.image.to_bytes();
        if let Err(err) = backend.replace_atomic(&bytes) {
            error!("Floppy eject-time flush failed: {err}");
            return;
        }
        match load_floppy_image(backend.path(), &bytes) {
            Ok(reparsed) => {
                self.image = reparsed;
                self.dirty = false;
            }
            Err(err) => error!("Floppy reparse after dirty flush failed: {err}"),
        }
    }

    /// Flushes dirty fallback data and any buffered successful writes.
    pub fn flush(&mut self) {
        self.flush_if_dirty();
        if let Some(backend) = self.backend.as_mut()
            && let Err(err) = backend.flush()
        {
            self.dirty = true;
            error!("Floppy flush failed: {err}");
        }
    }

    /// Flushes any pending writes and drops the backend handle.
    pub fn eject(mut self) {
        self.flush();
    }
}

/// Returns the stable media-manifest tag for a floppy format.
const fn floppy_format_tag(format: FloppyFormat) -> u8 {
    match format {
        FloppyFormat::D88 => 0,
        FloppyFormat::D77 => 1,
        FloppyFormat::Hdm => 2,
        FloppyFormat::NfdR0 => 3,
        FloppyFormat::NfdR1 => 4,
        FloppyFormat::TwoD => 5,
        FloppyFormat::Dim => 6,
        FloppyFormat::Xdf => 7,
        FloppyFormat::Img => 8,
        FloppyFormat::IbmXdf => 9,
        FloppyFormat::MsxDsk(MsxDskGeometry::Tracks80Sides1) => 10,
        FloppyFormat::MsxDsk(MsxDskGeometry::Tracks40Sides2) => 11,
        FloppyFormat::MsxDsk(MsxDskGeometry::Tracks80Sides2) => 12,
    }
}

/// Loads a floppy image, auto-detecting the format by file extension.
pub fn load_floppy_image(path: &Path, data: &[u8]) -> Result<FloppyImage, FloppyError> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());

    match extension.as_deref() {
        Some("hdm") => FloppyImage::from_hdm_bytes(data),
        Some("nfd") => FloppyImage::from_nfd_bytes(data),
        Some("2d") => FloppyImage::from_2d_bytes(data),
        Some("d77") => FloppyImage::from_d77_bytes(data),
        Some("dim") => FloppyImage::from_dim_bytes(data),
        Some("xdf") if data.len() == ibm_xdf::IBM_XDF_FILE_SIZE => {
            FloppyImage::from_ibm_xdf_bytes(data)
        }
        Some("xdf") | Some("2hd") => FloppyImage::from_xdf_bytes(data),
        Some("img") | Some("ima") if data.len() == ibm_xdf::IBM_XDF_FILE_SIZE => {
            FloppyImage::from_ibm_xdf_bytes(data)
        }
        Some("img") | Some("ima") => FloppyImage::from_img_bytes(data),
        Some("dsk") => FloppyImage::from_msx_dsk_bytes(data),
        Some("d88") | Some("d98") | Some("88d") | Some("98d") => FloppyImage::from_d88_bytes(data),
        _ => FloppyImage::from_d88_bytes(data),
    }
}

/// Error type for floppy image parsing.
#[derive(Debug, Clone)]
pub enum FloppyError {
    /// D88 format parsing error.
    D88(D88Error),
    /// HDM format parsing error.
    Hdm(hdm::HdmError),
    /// NFD format parsing error.
    Nfd(nfd::NfdError),
    /// 2D format parsing error.
    TwoD(two_d::TwoDError),
    /// DIM format parsing error.
    Dim(dim::DimError),
    /// XDF format parsing error.
    Xdf(xdf::XdfError),
    /// IMG format parsing error.
    Img(img::ImgError),
    /// IBM XDF format parsing error.
    IbmXdf(ibm_xdf::IbmXdfError),
    /// Raw MSX DSK parsing error.
    MsxDsk(MsxDskError),
    /// File extension not recognized as a supported floppy format.
    UnrecognizedFormat,
}

impl fmt::Display for FloppyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FloppyError::D88(err) => write!(f, "{err}"),
            FloppyError::Hdm(err) => write!(f, "{err}"),
            FloppyError::Nfd(err) => write!(f, "{err}"),
            FloppyError::TwoD(err) => write!(f, "{err}"),
            FloppyError::Dim(err) => write!(f, "{err}"),
            FloppyError::Xdf(err) => write!(f, "{err}"),
            FloppyError::Img(err) => write!(f, "{err}"),
            FloppyError::IbmXdf(err) => write!(f, "{err}"),
            FloppyError::MsxDsk(err) => write!(f, "{err}"),
            FloppyError::UnrecognizedFormat => write!(f, "unrecognized floppy image format"),
        }
    }
}

impl Error for FloppyError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempfile_with(bytes: &[u8], suffix: &str) -> PathBuf {
        use std::{
            fs::OpenOptions,
            io::Write,
            sync::atomic::{AtomicU64, Ordering},
        };

        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let dir = std::env::temp_dir();
        let unique = format!(
            "neetan_floppy_test_{}_{}_{}{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
            suffix
        );
        let path = dir.join(unique);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("create temp file");
        file.write_all(bytes).expect("write temp file");
        path
    }

    fn build_minimal_d88(payload_byte: u8) -> Vec<u8> {
        // One track, one 256-byte sector (N=1), C=0 H=0 R=1.
        const HEADER_SIZE: usize = 0x2B0;
        const SECTOR_HEADER_SIZE: usize = 16;
        let mut image = vec![0u8; HEADER_SIZE];
        image[0x1B] = 0x10; // 2DD
        let track_offset = HEADER_SIZE as u32;
        image[0x20..0x24].copy_from_slice(&track_offset.to_le_bytes());

        let mut sector = vec![0u8; SECTOR_HEADER_SIZE];
        sector[0] = 0; // C
        sector[1] = 0; // H
        sector[2] = 1; // R
        sector[3] = 1; // N (256 bytes)
        sector[4..6].copy_from_slice(&1u16.to_le_bytes()); // sector_count
        sector[0x0E..0x10].copy_from_slice(&256u16.to_le_bytes());
        let mut data = vec![payload_byte; 256];
        sector.append(&mut data);
        image.extend_from_slice(&sector);

        let total = image.len() as u32;
        image[0x1C..0x20].copy_from_slice(&total.to_le_bytes());
        image
    }

    #[test]
    fn d77_extension_loads_as_d77_and_round_trips() {
        // D77 is the FM-7 disk format sharing the D88 container, so a `.d77` file
        // parses through the D88 path but keeps its own format tag.
        let original = build_minimal_d88(0x5A);
        let path = tempfile_with(&original, ".d77");

        let image = load_floppy_image(&path, &original).expect("load d77");
        assert_eq!(image.format, FloppyFormat::D77);
        assert_eq!(image.format_name(), "D77");
        assert_eq!(image.to_bytes(), original);
    }

    #[test]
    fn mounted_floppy_d88_sector_write_through() {
        let original = build_minimal_d88(0x00);
        let path = tempfile_with(&original, ".d88");

        let image = FloppyImage::from_d88_bytes(&original).unwrap();
        let mut mounted = MountedFloppy::new(image, Some(path.clone()));

        let pattern = [0x42u8; 256];
        assert!(mounted.write_sector_data(0, 0, 0, 1, 1, &pattern));

        // Flush the BufWriter before re-reading the file.
        mounted.flush();
        drop(mounted);
        let raw = std::fs::read(&path).unwrap();
        let reparsed = FloppyImage::from_d88_bytes(&raw).unwrap();
        let s = reparsed.find_sector(0, 0, 1, 1).unwrap();
        assert!(s.data.iter().all(|&b| b == 0x42));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn mounted_floppy_d88_format_track_reemits_file() {
        let original = build_minimal_d88(0x00);
        let path = tempfile_with(&original, ".d88");

        let image = FloppyImage::from_d88_bytes(&original).unwrap();
        let mut mounted = MountedFloppy::new(image, Some(path.clone()));
        let identity = mounted.identity();

        // Re-format track 0 with two sectors instead of one.
        mounted.format_track(0, &[(0, 0, 1, 1), (0, 0, 2, 1)], 1, 0xE5);

        // After format_track + re-emit + reparse, source_offsets should be
        // populated again, and a subsequent sector write reaches the file.
        let pattern = [0x55u8; 256];
        assert!(mounted.write_sector_data(0, 0, 0, 2, 1, &pattern));

        mounted.flush();
        drop(mounted);
        let raw = std::fs::read(&path).unwrap();
        let reparsed = FloppyImage::from_d88_bytes(&raw).unwrap();
        assert_eq!(reparsed.sector_count(0), 2);
        let s2 = reparsed.find_sector(0, 0, 2, 1).unwrap();
        assert!(s2.data.iter().all(|&b| b == 0x55));
        let remounted = MountedFloppy::new(reparsed, Some(path.clone()));
        assert_eq!(remounted.identity(), identity);
        assert_eq!(
            remounted.source_path(),
            Some(&save_state::MediaSourcePath::from_path(&path))
        );
        drop(remounted);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn mounted_floppy_hdm_sector_write_through() {
        // Build a 1.26MB HDM image with a known pattern.
        let mut original = vec![0u8; 1_261_568];
        for (i, byte) in original.iter_mut().enumerate() {
            *byte = (i & 0xFF) as u8;
        }
        let path = tempfile_with(&original, ".hdm");

        let image = FloppyImage::from_hdm_bytes(&original).unwrap();
        let mut mounted = MountedFloppy::new(image, Some(path.clone()));

        // Overwrite C=0 H=0 R=1.
        let pattern = [0xCCu8; 1024];
        assert!(mounted.write_sector_data(0, 0, 0, 1, 3, &pattern));

        mounted.flush();
        drop(mounted);
        let raw = std::fs::read(&path).unwrap();
        // First 1024 bytes should now be 0xCC.
        assert!(raw[..1024].iter().all(|&b| b == 0xCC));
        // Subsequent sectors unchanged.
        assert_eq!(raw[1024], (1024 & 0xFF) as u8);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn mounted_floppy_hdm_incompatible_format_stays_dirty_and_leaves_file_unchanged() {
        let mut original = vec![0u8; 1_261_568];
        for (i, byte) in original.iter_mut().enumerate() {
            *byte = (i & 0xFF) as u8;
        }
        let path = tempfile_with(&original, ".hdm");

        let image = FloppyImage::from_hdm_bytes(&original).unwrap();
        let mut mounted = MountedFloppy::new(image, Some(path.clone()));

        // HDM can only represent 1024-byte sectors. Formatting a track as
        // 256-byte sectors must not silently rewrite the file as zeroed HDM.
        mounted.format_track(0, &[(0, 0, 1, 1)], 1, 0xE5);

        assert!(mounted.is_dirty());
        let raw = std::fs::read(&path).unwrap();
        assert_eq!(raw, original);

        drop(mounted);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn mounted_floppy_nfd_format_stays_dirty_and_leaves_file_unchanged() {
        const COMMON_HEADER_SIZE: usize = 0x120;
        const R1_TRACK_TABLE_SIZE: usize = 164 * 4;
        const R1_TRACK_HEADER_SIZE: usize = 16;
        const SECTOR_ENTRY_SIZE: usize = 16;

        let track_metadata_size = R1_TRACK_HEADER_SIZE + SECTOR_ENTRY_SIZE;
        let header_section_size = COMMON_HEADER_SIZE + R1_TRACK_TABLE_SIZE + track_metadata_size;
        let mut original = vec![0u8; header_section_size + 256];
        original[..15].copy_from_slice(b"T98FDDIMAGE.R1\0");
        original[0x10..0x1A].copy_from_slice(b"KEEP-TITLE");
        original[0x110..0x114].copy_from_slice(&(header_section_size as u32).to_le_bytes());

        let track_meta_offset = COMMON_HEADER_SIZE + R1_TRACK_TABLE_SIZE;
        original[COMMON_HEADER_SIZE..COMMON_HEADER_SIZE + 4]
            .copy_from_slice(&(track_meta_offset as u32).to_le_bytes());
        original[track_meta_offset..track_meta_offset + 2].copy_from_slice(&1u16.to_le_bytes());

        let entry = track_meta_offset + R1_TRACK_HEADER_SIZE;
        original[entry] = 0;
        original[entry + 1] = 0;
        original[entry + 2] = 1;
        original[entry + 3] = 1;
        original[entry + 11] = 0x90;
        original[header_section_size..].fill(0xA5);

        let path = tempfile_with(&original, ".nfd");
        let image = FloppyImage::from_nfd_bytes(&original).unwrap();
        let mut mounted = MountedFloppy::new(image, Some(path.clone()));

        mounted.format_track(0, &[(0, 0, 1, 1)], 1, 0xE5);

        assert!(mounted.is_dirty());
        let raw = std::fs::read(&path).unwrap();
        assert_eq!(raw, original);

        drop(mounted);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn mounted_floppy_write_sector_preserves_existing_dirty_bit() {
        let original = build_minimal_d88(0x00);
        let path = tempfile_with(&original, ".d88");

        let image = FloppyImage::from_d88_bytes(&original).unwrap();
        let mut mounted = MountedFloppy::new(image, Some(path.clone()));

        // Simulate a prior write-through error.
        mounted.dirty = true;

        let pattern = [0x42u8; 256];
        assert!(mounted.write_sector_data(0, 0, 0, 1, 1, &pattern));
        assert!(
            mounted.is_dirty(),
            "dirty must remain set after later successful write"
        );

        drop(mounted);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn mounted_floppy_format_track_clears_dirty_only_on_full_success() {
        let original = build_minimal_d88(0x00);
        let path = tempfile_with(&original, ".d88");

        let image = FloppyImage::from_d88_bytes(&original).unwrap();
        let mut mounted = MountedFloppy::new(image, Some(path.clone()));

        // After a successful format_track + replace_atomic + reparse,
        // dirty must be false. Pins down the success-path post-condition
        // for the planned restructure of the assignment site.
        mounted.format_track(0, &[(0, 0, 1, 1), (0, 0, 2, 1)], 1, 0xE5);
        assert!(
            !mounted.is_dirty(),
            "successful format_track must clear dirty"
        );

        drop(mounted);
        std::fs::remove_file(&path).ok();
    }
}
