//! `neetan copy` subcommand: copy files between the host filesystem and
//! FAT-formatted disk images.
//!
//! Directory copies are recursive. Long host filenames that don't fit
//! 8.3 ASCII are rejected during a pre-flight pass before any file is written.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use common::{Context, StringError, bail, info};
use device::{
    disk::{HddFormat, HddImage, load_hdd_image},
    floppy::{FloppyFormat, FloppyImage, MsxDskGeometry, d88::D88MediaType, load_floppy_image},
};
use dos::{
    DiskIo,
    copy_common::{
        dos_display, dos_leaf_basename, dos_now, join_dos, split_dos_parent,
        trim_trailing_separator, validate_dos_basename, validate_dos_components,
    },
    filesystem::fat_api::{FatError, FatFs, Metadata},
};

use crate::config::CopyArg;

/// File extensions treated as hard disk images.
pub(crate) const HDD_EXTENSIONS: &[&str] = &["hdi", "nhd", "thd", "hdd"];
/// File extensions treated as floppy disk images.
pub(crate) const FDD_EXTENSIONS: &[&str] = &[
    "d88", "d98", "88d", "98d", "hdm", "nfd", "2d", "img", "ima", "dsk",
];

/// Returns whether an extension identifies a supported disk image.
pub(crate) fn is_disk_image_extension(extension: &str) -> bool {
    HDD_EXTENSIONS
        .iter()
        .chain(FDD_EXTENSIONS)
        .any(|known| extension.eq_ignore_ascii_case(known))
}

/// Top-level dispatcher for the `copy` subcommand.
pub fn copy(source: CopyArg, dest: CopyArg) -> crate::Result<()> {
    match (source, dest) {
        (
            CopyArg::Host(src),
            CopyArg::Image {
                image_path,
                dos_path,
            },
        ) => copy_host_to_image(&src, &image_path, &dos_path),
        (
            CopyArg::Image {
                image_path,
                dos_path,
            },
            CopyArg::Host(dst),
        ) => copy_image_to_host(&image_path, &dos_path, &dst),
        (
            CopyArg::Image {
                image_path: source_image_path,
                dos_path: source_dos_path,
            },
            CopyArg::Image {
                image_path: dest_image_path,
                dos_path: dest_dos_path,
            },
        ) => copy_image_to_image(
            &source_image_path,
            &source_dos_path,
            &dest_image_path,
            &dest_dos_path,
        ),
        (CopyArg::Host(_), CopyArg::Host(_)) => {
            bail!(
                "neither argument refers to a disk image; use a host filesystem copy tool instead"
            )
        }
    }
}

fn copy_host_to_image(host_source: &Path, image_path: &Path, dos_path: &[u8]) -> crate::Result<()> {
    let trailing_slash = dos_path.last() == Some(&b'\\');
    let normalized_dos = trim_trailing_separator(dos_path).to_vec();

    let metadata = std::fs::symlink_metadata(host_source)
        .with_context(|| format!("failed to stat {}", host_source.display()))?;

    let host_tree = if metadata.is_dir() {
        Some(walk_host_dir(host_source)?)
    } else if metadata.is_file() {
        None
    } else {
        bail!(
            "source is not a regular file or directory: {}",
            host_source.display()
        );
    };

    // Pre-flight name validation. Every leaf (basename) on the host side and
    // every component of the destination DOS path must fit 8.3 ASCII.
    validate_dos_components(&normalized_dos).map_err(|e| {
        StringError(format!(
            "destination path component is not valid 8.3 ASCII: {e}"
        ))
    })?;
    let source_basename = host_basename(host_source)?;
    if let Some(tree) = &host_tree {
        validate_host_tree_names(tree)?;
    }

    let mut io = load_image_as_disk_io(image_path)?;
    {
        let mut fs = mount_fat(&mut io)?;
        let existing = fs.stat(&normalized_dos).map_err(map_fat_err)?;

        if let Some(tree) = host_tree {
            // Directory source: build the target root according to standard copy semantics.
            let target_root = match &existing {
                Some(meta) if meta.is_dir() => join_dos(&normalized_dos, &source_basename),
                Some(_) => bail!(
                    "destination exists and is not a directory: {}",
                    String::from_utf8_lossy(&normalized_dos)
                ),
                None => normalized_dos.clone(),
            };
            let (time, date) = dos_now();
            ensure_parent_exists(&mut fs, &target_root, time, date)?;
            fs.mkdir_p(&target_root, time, date).map_err(map_fat_err)?;
            info!(
                "mkdir image:{}:{}",
                image_path.display(),
                dos_display(&target_root)
            );
            write_tree_into_image(&mut fs, &tree, &target_root, image_path, time, date)?;
        } else {
            // Single file source.
            let target = match &existing {
                Some(meta) if meta.is_dir() => join_dos(&normalized_dos, &source_basename),
                Some(_) => normalized_dos.clone(),
                None if trailing_slash => bail!(
                    "destination directory does not exist: {}",
                    String::from_utf8_lossy(&normalized_dos)
                ),
                None => normalized_dos.clone(),
            };
            let (time, date) = dos_now();
            ensure_parent_exists(&mut fs, &target, time, date)?;
            let data = std::fs::read(host_source)
                .with_context(|| format!("failed to read {}", host_source.display()))?;
            fs.write_file(&target, &data, 0x20, time, date)
                .map_err(map_fat_err)?;
            info!(
                "copy host:{} -> image:{}:{}",
                host_source.display(),
                image_path.display(),
                dos_display(&target)
            );
        }

        fs.flush().map_err(map_fat_err)?;
    }
    write_image_back(image_path, io)?;
    Ok(())
}

fn copy_image_to_host(image_path: &Path, dos_path: &[u8], host_dest: &Path) -> crate::Result<()> {
    let normalized_dos = trim_trailing_separator(dos_path).to_vec();
    validate_dos_components(&normalized_dos)
        .map_err(|e| StringError(format!("source path component is not valid 8.3 ASCII: {e}")))?;
    let mut io = load_image_as_disk_io(image_path)?;
    let mut fs = mount_fat(&mut io)?;
    let source_meta = fs
        .stat(&normalized_dos)
        .map_err(map_fat_err)?
        .ok_or_else(|| {
            StringError(format!(
                "not found in image: {}",
                String::from_utf8_lossy(&normalized_dos)
            ))
        })?;

    if source_meta.is_dir() {
        let target_root = if host_dest.exists() && host_dest.is_dir() {
            // Recursive copy semantics: source goes inside dest using its basename.
            let basename =
                dos_leaf_basename(&normalized_dos).unwrap_or_else(|| source_meta.name.clone());
            host_dest.join(host_path_from_dos_bytes(&basename)?)
        } else {
            host_dest.to_path_buf()
        };
        std::fs::create_dir_all(&target_root)
            .with_context(|| format!("failed to create directory {}", target_root.display()))?;
        info!("mkdir host:{}", target_root.display());
        write_image_dir_to_host(&mut fs, &normalized_dos, &target_root, image_path)?;
    } else {
        let target = if host_dest.exists() && host_dest.is_dir() {
            host_dest.join(host_path_from_dos_bytes(&source_meta.name)?)
        } else {
            host_dest.to_path_buf()
        };
        if let Some(parent) = target.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        let data = fs.read_file(&normalized_dos).map_err(map_fat_err)?;
        std::fs::write(&target, &data)
            .with_context(|| format!("failed to write {}", target.display()))?;
        info!(
            "copy image:{}:{} -> host:{}",
            image_path.display(),
            dos_display(&normalized_dos),
            target.display()
        );
    }
    Ok(())
}

fn copy_image_to_image(
    source_image_path: &Path,
    source_dos_path: &[u8],
    dest_image_path: &Path,
    dest_dos_path: &[u8],
) -> crate::Result<()> {
    let source_normalized = trim_trailing_separator(source_dos_path).to_vec();
    validate_dos_components(&source_normalized)
        .map_err(|e| StringError(format!("source path component is not valid 8.3 ASCII: {e}")))?;
    let dest_trailing_slash = dest_dos_path.last() == Some(&b'\\');
    let dest_normalized = trim_trailing_separator(dest_dos_path).to_vec();
    validate_dos_components(&dest_normalized).map_err(|e| {
        StringError(format!(
            "destination path component is not valid 8.3 ASCII: {e}"
        ))
    })?;

    // Phase 1: materialize the source tree in memory. The source image is
    // closed before we open the destination.
    let source_node = {
        let mut source_io = load_image_as_disk_io(source_image_path)?;
        let mut fs = mount_fat(&mut source_io)?;
        let source_meta = fs
            .stat(&source_normalized)
            .map_err(map_fat_err)?
            .ok_or_else(|| {
                StringError(format!(
                    "not found in image: {}",
                    String::from_utf8_lossy(&source_normalized)
                ))
            })?;
        read_image_node(&mut fs, &source_normalized, &source_meta, source_image_path)?
    };

    // Phase 2: write into the destination image.
    let mut dest_io = load_image_as_disk_io(dest_image_path)?;
    {
        let mut fs = mount_fat(&mut dest_io)?;
        let existing = fs.stat(&dest_normalized).map_err(map_fat_err)?;
        let (time, date) = dos_now();

        let source_is_dir = source_node.is_dir();

        let target = match &existing {
            Some(meta) if meta.is_dir() => join_dos(&dest_normalized, source_node.name()),
            Some(_) if source_is_dir => bail!(
                "destination exists and is not a directory: {}",
                String::from_utf8_lossy(&dest_normalized)
            ),
            Some(_) => dest_normalized.clone(),
            None if dest_trailing_slash => bail!(
                "destination directory does not exist: {}",
                String::from_utf8_lossy(&dest_normalized)
            ),
            None => dest_normalized.clone(),
        };

        ensure_parent_exists(&mut fs, &target, time, date)?;
        if source_is_dir {
            fs.mkdir_p(&target, time, date).map_err(map_fat_err)?;
            info!(
                "mkdir image:{}:{}",
                dest_image_path.display(),
                dos_display(&target)
            );
        }
        write_node_into_image(&mut fs, &source_node, &target, dest_image_path, time, date)?;
        fs.flush().map_err(map_fat_err)?;
    }
    write_image_back(dest_image_path, dest_io)?;
    Ok(())
}

/// In-memory representation of a source subtree read from a FAT image.
enum CopyNode {
    File {
        name: Vec<u8>,
        data: Vec<u8>,
    },
    Dir {
        name: Vec<u8>,
        children: Vec<CopyNode>,
    },
}

impl CopyNode {
    fn name(&self) -> &[u8] {
        match self {
            CopyNode::File { name, .. } | CopyNode::Dir { name, .. } => name,
        }
    }

    fn is_dir(&self) -> bool {
        matches!(self, CopyNode::Dir { .. })
    }
}

fn read_image_node(
    fs: &mut FatFs<'_>,
    dos_path: &[u8],
    meta: &Metadata,
    image_path: &Path,
) -> crate::Result<CopyNode> {
    let name = if meta.name.is_empty() {
        // Root directory: use a synthetic name; this should only happen if the
        // caller explicitly addressed the whole volume.
        bail!("cannot copy the entire volume root; address a subdirectory or file");
    } else {
        meta.name.clone()
    };
    if meta.is_dir() {
        let entries = fs.list_dir(dos_path).map_err(map_fat_err)?;
        let mut children = Vec::new();
        for entry in entries {
            if entry.is_volume_label() {
                continue;
            }
            let child_path = join_dos(dos_path, &entry.name);
            let child = read_image_node(fs, &child_path, &entry, image_path)?;
            children.push(child);
        }
        info!(
            "read image:{}:{} ({} entries)",
            image_path.display(),
            dos_display(dos_path),
            children.len()
        );
        Ok(CopyNode::Dir { name, children })
    } else {
        let data = fs.read_file(dos_path).map_err(map_fat_err)?;
        info!(
            "read image:{}:{} ({} bytes)",
            image_path.display(),
            dos_display(dos_path),
            data.len()
        );
        Ok(CopyNode::File { name, data })
    }
}

fn write_node_into_image(
    fs: &mut FatFs<'_>,
    node: &CopyNode,
    dos_target: &[u8],
    image_path: &Path,
    time: u16,
    date: u16,
) -> crate::Result<()> {
    match node {
        CopyNode::File { data, .. } => {
            fs.write_file(dos_target, data, 0x20, time, date)
                .map_err(map_fat_err)?;
            info!(
                "copy -> image:{}:{} ({} bytes)",
                image_path.display(),
                dos_display(dos_target),
                data.len()
            );
        }
        CopyNode::Dir { children, .. } => {
            for child in children {
                let child_target = join_dos(dos_target, child.name());
                if child.is_dir() {
                    fs.mkdir_p(&child_target, time, date).map_err(map_fat_err)?;
                    info!(
                        "mkdir image:{}:{}",
                        image_path.display(),
                        dos_display(&child_target)
                    );
                }
                write_node_into_image(fs, child, &child_target, image_path, time, date)?;
            }
        }
    }
    Ok(())
}

/// Host filesystem tree, mirroring the on-disk structure of a directory.
struct HostNode {
    /// Host basename of this entry.
    basename: String,
    /// Full host path.
    path: PathBuf,
    kind: HostKind,
}

enum HostKind {
    File,
    Dir(Vec<HostNode>),
}

fn walk_host_dir(path: &Path) -> crate::Result<Vec<HostNode>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(path)
        .with_context(|| format!("failed to read directory {}", path.display()))?
    {
        let entry = entry
            .with_context(|| format!("failed to read directory entry in {}", path.display()))?;
        let entry_path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to stat {}", entry_path.display()))?;
        let basename = entry
            .file_name()
            .into_string()
            .map_err(|os| StringError(format!("non-UTF-8 host name: {os:?}")))?;
        if file_type.is_dir() {
            let children = walk_host_dir(&entry_path)?;
            out.push(HostNode {
                basename,
                path: entry_path,
                kind: HostKind::Dir(children),
            });
        } else if file_type.is_file() {
            out.push(HostNode {
                basename,
                path: entry_path,
                kind: HostKind::File,
            });
        } else {
            common::warn!("skipping non-regular file: {}", entry_path.display());
        }
    }
    Ok(out)
}

fn validate_host_tree_names(nodes: &[HostNode]) -> crate::Result<()> {
    let mut invalid: Vec<String> = Vec::new();
    let mut collisions: Vec<Vec<String>> = Vec::new();
    collect_host_tree_name_issues(nodes, &mut invalid, &mut collisions);
    if !invalid.is_empty() {
        let mut msg = String::from("the following host names do not fit 8.3 ASCII:\n");
        for name in invalid {
            msg.push_str("  ");
            msg.push_str(&name);
            msg.push('\n');
        }
        msg.push_str("rename the offending files before copying");
        return Err(StringError(msg).into());
    }
    if !collisions.is_empty() {
        let mut msg = String::from("the following host names collide after 8.3 normalization:\n");
        for group in collisions {
            for name in group {
                msg.push_str("  ");
                msg.push_str(&name);
                msg.push('\n');
            }
            msg.push('\n');
        }
        msg.push_str("rename the offending files before copying");
        return Err(StringError(msg).into());
    }
    Ok(())
}

fn collect_host_tree_name_issues(
    nodes: &[HostNode],
    invalid: &mut Vec<String>,
    collisions: &mut Vec<Vec<String>>,
) {
    let mut names: BTreeMap<Vec<u8>, Vec<String>> = BTreeMap::new();
    for node in nodes {
        match validate_dos_basename(node.basename.as_bytes()) {
            Ok(dos_name) => {
                names
                    .entry(dos_name)
                    .or_default()
                    .push(node.path.display().to_string());
            }
            Err(_) => invalid.push(node.path.display().to_string()),
        }
        if let HostKind::Dir(children) = &node.kind {
            collect_host_tree_name_issues(children, invalid, collisions);
        }
    }
    for group in names.into_values() {
        if group.len() > 1 {
            collisions.push(group);
        }
    }
}

fn write_tree_into_image(
    fs: &mut FatFs<'_>,
    children: &[HostNode],
    dos_target: &[u8],
    image_path: &Path,
    time: u16,
    date: u16,
) -> crate::Result<()> {
    for node in children {
        let dos_name = host_name_to_dos(&node.basename)?;
        let child_dos = join_dos(dos_target, &dos_name);
        match &node.kind {
            HostKind::File => {
                let data = std::fs::read(&node.path)
                    .with_context(|| format!("failed to read {}", node.path.display()))?;
                fs.write_file(&child_dos, &data, 0x20, time, date)
                    .map_err(map_fat_err)?;
                info!(
                    "copy host:{} -> image:{}:{}",
                    node.path.display(),
                    image_path.display(),
                    dos_display(&child_dos)
                );
            }
            HostKind::Dir(grandchildren) => {
                fs.mkdir_p(&child_dos, time, date).map_err(map_fat_err)?;
                info!(
                    "mkdir image:{}:{}",
                    image_path.display(),
                    dos_display(&child_dos)
                );
                write_tree_into_image(fs, grandchildren, &child_dos, image_path, time, date)?;
            }
        }
    }
    Ok(())
}

fn write_image_dir_to_host(
    fs: &mut FatFs<'_>,
    dos_path: &[u8],
    host_dir: &Path,
    image_path: &Path,
) -> crate::Result<()> {
    let entries = fs.list_dir(dos_path).map_err(map_fat_err)?;
    for entry in entries {
        if entry.is_volume_label() {
            continue;
        }
        let child_dos = join_dos(dos_path, &entry.name);
        let host_basename = host_path_from_dos_bytes(&entry.name)?;
        let child_host = host_dir.join(host_basename);
        if entry.is_dir() {
            std::fs::create_dir_all(&child_host)
                .with_context(|| format!("failed to create directory {}", child_host.display()))?;
            info!("mkdir host:{}", child_host.display());
            write_image_dir_to_host(fs, &child_dos, &child_host, image_path)?;
        } else {
            let data = fs.read_file(&child_dos).map_err(map_fat_err)?;
            std::fs::write(&child_host, &data)
                .with_context(|| format!("failed to write {}", child_host.display()))?;
            info!(
                "copy image:{}:{} -> host:{}",
                image_path.display(),
                dos_display(&child_dos),
                child_host.display()
            );
        }
    }
    Ok(())
}

/// Loads an image file and wraps it in a `FileImageDiskIo`. The extension
/// determines the format. Returns `Err` for unrecognized extensions.
fn load_image_as_disk_io(path: &Path) -> crate::Result<FileImageDiskIo> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read image {}", path.display()))?;
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());

    match extension.as_deref() {
        Some(ext) if HDD_EXTENSIONS.contains(&ext) => {
            let image = load_hdd_image(path, &bytes)
                .with_context(|| format!("failed to parse HDD image {}", path.display()))?;
            Ok(FileImageDiskIo::new_hdd(image))
        }
        Some(ext) if FDD_EXTENSIONS.contains(&ext) => {
            let image = load_floppy_image(path, &bytes)
                .with_context(|| format!("failed to parse FDD image {}", path.display()))?;
            FileImageDiskIo::new_fdd(image).map_err(|e| StringError(e).into())
        }
        _ => bail!("unrecognized image extension: {}", path.display()),
    }
}

fn mount_fat<'a>(io: &'a mut FileImageDiskIo) -> crate::Result<FatFs<'a>> {
    let drive_da = io.drive_da();
    if drive_da & 0xF0 == 0x80 {
        if io.uses_mbr_partitions() {
            FatFs::mount_hdd_mbr(io, drive_da)
                .map_err(|e| StringError(format!("failed to mount FAT: {e}")).into())
        } else {
            FatFs::mount_hdd(io, drive_da)
                .map_err(|e| StringError(format!("failed to mount FAT: {e}")).into())
        }
    } else {
        FatFs::mount_fdd(io, drive_da)
            .map_err(|e| StringError(format!("failed to mount FAT: {e}")).into())
    }
}

fn write_image_back(path: &Path, io: FileImageDiskIo) -> crate::Result<()> {
    let bytes = io.into_bytes();
    let tmp_extension = match path.extension().and_then(|e| e.to_str()) {
        Some(orig) => format!("{orig}.tmp"),
        None => "tmp".to_string(),
    };
    let tmp_path = path.with_extension(tmp_extension);
    std::fs::write(&tmp_path, &bytes)
        .with_context(|| format!("failed to write temp {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "failed to rename {} -> {}",
            tmp_path.display(),
            path.display()
        )
    })?;
    info!("Updated image: {} ({} bytes)", path.display(), bytes.len());
    Ok(())
}

fn ensure_parent_exists(
    fs: &mut FatFs<'_>,
    dos_path: &[u8],
    time: u16,
    date: u16,
) -> crate::Result<()> {
    if let Some(parent) = split_dos_parent(dos_path)
        && !parent.is_empty()
    {
        fs.mkdir_p(&parent, time, date).map_err(map_fat_err)?;
    }
    Ok(())
}

/// Adapter that exposes a host-side image file as a `DiskIo`.
pub(crate) struct FileImageDiskIo {
    image: FileImage,
    drive_da: u8,
}

enum FileImage {
    Hdd(HddImage),
    Fdd {
        image: FloppyImage,
        geometry: FddGeometry,
    },
}

#[derive(Clone, Copy)]
struct FddGeometry {
    cylinders: u16,
    heads: u8,
    sectors_per_track: u8,
    sector_size: u16,
    size_code: u8,
}

impl FileImageDiskIo {
    fn new_hdd(image: HddImage) -> Self {
        Self {
            image: FileImage::Hdd(image),
            drive_da: 0x80,
        }
    }

    fn new_fdd(image: FloppyImage) -> Result<Self, String> {
        if image.format == FloppyFormat::IbmXdf {
            return Err(
                "IBM XDF floppies use mixed-size sectors and cannot be accessed by the copy \
                 tool. Copy the files from inside the emulated DOS instead"
                    .to_string(),
            );
        }
        let geometry = derive_fdd_geometry(&image)?;
        let drive_da = match geometry.sector_size {
            1024 => 0x90,
            256 | 512 => 0x70,
            other => return Err(format!("unsupported FDD sector size: {other}")),
        };
        Ok(Self {
            image: FileImage::Fdd { image, geometry },
            drive_da,
        })
    }

    fn drive_da(&self) -> u8 {
        self.drive_da
    }

    /// Whether the image carries a PC/AT master boot record rather than a
    /// PC-98 IPL partition table.
    fn uses_mbr_partitions(&self) -> bool {
        match &self.image {
            FileImage::Hdd(image) => image.format == HddFormat::AtFlat,
            FileImage::Fdd { .. } => false,
        }
    }

    fn into_bytes(self) -> Vec<u8> {
        match self.image {
            FileImage::Hdd(image) => image.to_bytes(),
            FileImage::Fdd { image, .. } => image.to_bytes(),
        }
    }
}

fn derive_fdd_geometry(image: &FloppyImage) -> Result<FddGeometry, String> {
    let first = image
        .sector_at_index(0, 0)
        .ok_or_else(|| "FDD image has no sectors on track 0".to_string())?;
    let size_code = first.size_code;
    let sector_size = 128u16 << size_code;
    let sectors_per_track = image.sector_count(0);
    if sectors_per_track == 0 {
        return Err("FDD track 0 has no sectors".to_string());
    }
    let (cylinders, heads) = match image.format {
        FloppyFormat::MsxDsk(MsxDskGeometry::Tracks80Sides1) => (80, 1),
        FloppyFormat::MsxDsk(MsxDskGeometry::Tracks40Sides2) => (40, 2),
        FloppyFormat::MsxDsk(MsxDskGeometry::Tracks80Sides2) => (80, 2),
        _ => match image.media_type {
            D88MediaType::Disk2HD => (77u16, 2u8),
            D88MediaType::Disk2DD => (80, 2),
            D88MediaType::Disk2D => (40, 2),
        },
    };
    Ok(FddGeometry {
        cylinders,
        heads,
        sectors_per_track: sectors_per_track as u8,
        sector_size,
        size_code,
    })
}

impl DiskIo for FileImageDiskIo {
    fn read_sectors(&mut self, drive_da: u8, lba: u32, count: u32) -> Result<Vec<u8>, u8> {
        if drive_da != self.drive_da {
            return Err(0x40);
        }
        let sector_size = self.sector_size(drive_da).ok_or(0x40)? as usize;
        let mut out = Vec::with_capacity(count as usize * sector_size);
        for i in 0..count {
            let current = lba + i;
            match &self.image {
                FileImage::Hdd(image) => {
                    let sector = image.read_sector(current).ok_or(0x40u8)?;
                    out.extend_from_slice(sector);
                }
                FileImage::Fdd { image, geometry } => {
                    let (cylinder, head, record) = lba_to_chr(current, *geometry);
                    let sector = image
                        .find_sector(cylinder, head, record, geometry.size_code)
                        .ok_or(0x40u8)?;
                    out.extend_from_slice(&sector.data);
                }
            }
        }
        Ok(out)
    }

    fn write_sectors(&mut self, drive_da: u8, lba: u32, data: &[u8]) -> Result<(), u8> {
        if drive_da != self.drive_da {
            return Err(0x40);
        }
        let sector_size = self.sector_size(drive_da).ok_or(0x40)? as usize;
        if !data.len().is_multiple_of(sector_size) {
            return Err(0x40);
        }
        let count = data.len() / sector_size;
        for i in 0..count {
            let current_lba = lba + i as u32;
            let chunk = &data[i * sector_size..(i + 1) * sector_size];
            match &mut self.image {
                FileImage::Hdd(image) => {
                    if !image.write_sector(current_lba, chunk) {
                        return Err(0x40);
                    }
                }
                FileImage::Fdd { image, geometry } => {
                    let (cylinder, head, record) = lba_to_chr(current_lba, *geometry);
                    let track_index = cylinder as usize * geometry.heads as usize + head as usize;
                    let sector = image
                        .find_sector_near_track_index_mut(
                            track_index,
                            cylinder,
                            head,
                            record,
                            geometry.size_code,
                        )
                        .ok_or(0x40u8)?;
                    let bytes_to_copy = chunk.len().min(sector.data.len());
                    sector.data[..bytes_to_copy].copy_from_slice(&chunk[..bytes_to_copy]);
                }
            }
        }
        Ok(())
    }

    fn sector_size(&self, drive_da: u8) -> Option<u16> {
        if drive_da != self.drive_da {
            return None;
        }
        match &self.image {
            FileImage::Hdd(image) => Some(image.geometry.sector_size),
            FileImage::Fdd { geometry, .. } => Some(geometry.sector_size),
        }
    }

    fn total_sectors(&self, drive_da: u8) -> Option<u32> {
        if drive_da != self.drive_da {
            return None;
        }
        match &self.image {
            FileImage::Hdd(image) => Some(image.geometry.total_sectors()),
            FileImage::Fdd { geometry, .. } => Some(
                geometry.cylinders as u32
                    * geometry.heads as u32
                    * geometry.sectors_per_track as u32,
            ),
        }
    }

    fn drive_geometry(&self, drive_da: u8) -> Option<(u16, u8, u8)> {
        if drive_da != self.drive_da {
            return None;
        }
        match &self.image {
            FileImage::Hdd(image) => Some((
                image.geometry.cylinders,
                image.geometry.heads,
                image.geometry.sectors_per_track,
            )),
            FileImage::Fdd { geometry, .. } => Some((
                geometry.cylinders,
                geometry.heads,
                geometry.sectors_per_track,
            )),
        }
    }
}

fn lba_to_chr(lba: u32, geom: FddGeometry) -> (u8, u8, u8) {
    let spt = geom.sectors_per_track as u32;
    let heads = geom.heads as u32;
    let track = lba / spt;
    let cylinder = (track / heads) as u8;
    let head = (track % heads) as u8;
    let record = ((lba % spt) + 1) as u8;
    (cylinder, head, record)
}

fn map_fat_err(error: FatError) -> crate::Error {
    StringError(format!("FAT error: {error}")).into()
}

/// Returns the host basename of a path, converted to a DOS 8.3 display name.
fn host_basename(path: &Path) -> crate::Result<Vec<u8>> {
    let name = path
        .file_name()
        .ok_or_else(|| StringError(format!("source has no basename: {}", path.display())))?
        .to_str()
        .ok_or_else(|| StringError(format!("non-UTF-8 source name: {}", path.display())))?;
    host_name_to_dos(name)
}

/// Validates and converts a host basename to a DOS 8.3 display name. ASCII
/// only. Rejects characters illegal in DOS (`<`, `>`, `:`, `"`, `|`, `?`, `*`,
/// backslash, forward slash, control bytes) and names with a base > 8 chars
/// or extension > 3 chars.
fn host_name_to_dos(name: &str) -> crate::Result<Vec<u8>> {
    validate_dos_basename(name.as_bytes()).map_err(|e| {
        StringError(format!(
            "host name '{name}' cannot be represented as 8.3 ASCII: {e}"
        ))
        .into()
    })
}

fn host_path_from_dos_bytes(name: &[u8]) -> crate::Result<String> {
    std::str::from_utf8(name)
        .map(|s| s.to_owned())
        .map_err(|_| StringError(format!("DOS name is not UTF-8: {name:?}")).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Logical sectors follow cylinder, head and record ordering.
    fn lba_to_chr_matches_geometry() {
        let geom = FddGeometry {
            cylinders: 77,
            heads: 2,
            sectors_per_track: 8,
            sector_size: 1024,
            size_code: 3,
        };
        // LBA 0 -> C=0, H=0, R=1
        assert_eq!(lba_to_chr(0, geom), (0, 0, 1));
        // LBA 7 -> last sector of track 0/head 0
        assert_eq!(lba_to_chr(7, geom), (0, 0, 8));
        // LBA 8 -> first sector of head 1 on cylinder 0
        assert_eq!(lba_to_chr(8, geom), (0, 1, 1));
        // LBA 16 -> first sector of cylinder 1, head 0
        assert_eq!(lba_to_chr(16, geom), (1, 0, 1));
    }

    #[test]
    /// The copy utility preserves detected single-sided MSX geometry.
    fn derives_single_sided_msx_geometry() {
        let image = FloppyImage::from_msx_dsk_bytes(&vec![0; 360 * 1024]).expect("MSX disk parses");
        let geometry = derive_fdd_geometry(&image).expect("geometry is available");
        assert_eq!(geometry.cylinders, 80);
        assert_eq!(geometry.heads, 1);
        assert_eq!(geometry.sectors_per_track, 9);
        assert_eq!(geometry.sector_size, 512);
    }
}
