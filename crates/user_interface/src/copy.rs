//! `neetan copy` subcommand: copy files between the host filesystem and
//! FAT-formatted disk images.
//!
//! Directory copies are recursive. A pre-flight pass checks every name before the
//! first file is written. Names written into an image must fit 8.3 ASCII and stay
//! distinct as 8.3 names. Names extracted to the host must be valid host file names
//! and stay distinct ignoring the case.

use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

use common::{Context, StringError, bail, info, shift_jis_to_string};
use device::{
    disk::{HddFormat, HddGeometry, HddImage, load_hdd_image, pc98_ide, pc98_sasi},
    floppy::{
        FLOPPY_EXTENSIONS, FloppyFormat, FloppyGeometry, FloppyImage, d88::D88Sector,
        load_floppy_image,
    },
};
use dos::{
    DiskIo,
    copy_common::{
        dos_display, dos_now, join_dos, split_dos_parent, trim_trailing_separator,
        validate_dos_basename, validate_dos_components,
    },
    drive_map::{PartitionScheme, pc98},
    filesystem::fat_api::{FatError, FatFs, Metadata},
};

use crate::config::CopyArg;

/// File extensions treated as hard disk images.
pub(crate) const HDD_EXTENSIONS: &[&str] = &["hdi", "nhd", "thd", "hdd"];

/// Characters that Windows forbids in a file name. Extraction rejects them on every host.
const HOST_RESERVED_CHARACTERS: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// Device names that Windows reserves with or without an extension, compared ignoring the
/// case. Extraction rejects them on every host.
const WINDOWS_DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Returns whether an extension identifies a supported disk image.
pub(crate) fn is_disk_image_extension(extension: &str) -> bool {
    HDD_EXTENSIONS
        .iter()
        .chain(FLOPPY_EXTENSIONS)
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
        let mut fs = mount_fat(&mut io, image_path)?;
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
    let source_node = {
        let mut io = load_image_as_disk_io(image_path)?;
        let mut fs = mount_fat(&mut io, image_path)?;
        let source_meta = fs
            .stat(&normalized_dos)
            .map_err(map_fat_err)?
            .ok_or_else(|| {
                StringError(format!(
                    "not found in image: {}",
                    String::from_utf8_lossy(&normalized_dos)
                ))
            })?;
        read_source_node(&mut fs, &normalized_dos, &source_meta, image_path)?
    };

    let target = if host_dest.is_dir() && !source_node.full_name.is_empty() {
        host_dest.join(source_node.host_name()?)
    } else {
        host_dest.to_path_buf()
    };

    match &source_node.kind {
        CopyKind::Dir(children) => {
            validate_image_host_names(children)?;
            std::fs::create_dir_all(&target)
                .with_context(|| format!("failed to create directory {}", target.display()))?;
            info!("mkdir host:{}", target.display());
            write_nodes_to_host(children, &target, image_path)?;
        }
        CopyKind::File(data) => {
            if let Some(parent) = target.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create directory {}", parent.display()))?;
            }
            std::fs::write(&target, data)
                .with_context(|| format!("failed to write {}", target.display()))?;
            info!(
                "copy image:{}:{} -> host:{}",
                image_path.display(),
                source_node.display_path,
                target.display()
            );
        }
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
        let mut fs = mount_fat(&mut source_io, source_image_path)?;
        let source_meta = fs
            .stat(&source_normalized)
            .map_err(map_fat_err)?
            .ok_or_else(|| {
                StringError(format!(
                    "not found in image: {}",
                    String::from_utf8_lossy(&source_normalized)
                ))
            })?;
        if source_meta.name.is_empty() {
            bail!("cannot copy the entire volume root; address a subdirectory or file");
        }
        read_source_node(&mut fs, &source_normalized, &source_meta, source_image_path)?
    };
    validate_image_target_names(&source_node)?;

    // Phase 2: write into the destination image.
    let mut dest_io = load_image_as_disk_io(dest_image_path)?;
    {
        let mut fs = mount_fat(&mut dest_io, dest_image_path)?;
        let existing = fs.stat(&dest_normalized).map_err(map_fat_err)?;
        let (time, date) = dos_now();

        let source_is_dir = source_node.is_dir();

        let target = match &existing {
            Some(meta) if meta.is_dir() => join_dos(&dest_normalized, &source_node.name),
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

/// A file or directory read from a FAT image into memory.
struct CopyNode {
    /// The 8.3 name of the entry. Image destinations take it.
    name: Vec<u8>,
    /// The raw name bytes of the entry, with the 18.3 name on a Human68k
    /// volume. Host destinations take its decoded form.
    full_name: Vec<u8>,
    /// The path of the entry in the image, decoded for messages.
    display_path: String,
    /// The data of a file or the entries of a directory.
    kind: CopyKind,
}

/// The content of a `CopyNode`.
enum CopyKind {
    /// A file with its data.
    File(Vec<u8>),
    /// A directory with its entries.
    Dir(Vec<CopyNode>),
}

impl CopyNode {
    /// Returns whether the entry is a directory.
    fn is_dir(&self) -> bool {
        matches!(self.kind, CopyKind::Dir(_))
    }

    /// Returns the host file name of the entry.
    fn host_name(&self) -> crate::Result<String> {
        host_name_from_dos_bytes(&self.full_name).map_err(|reason| {
            StringError(format!(
                "{} cannot be a host file name: {reason}",
                self.display_path
            ))
            .into()
        })
    }
}

/// Reads the source entry `meta` at `dos_path` and everything below it.
fn read_source_node(
    fs: &mut FatFs<'_>,
    dos_path: &[u8],
    meta: &Metadata,
    image_path: &Path,
) -> crate::Result<CopyNode> {
    let mut visited_directories = HashSet::new();
    read_image_node(
        fs,
        meta,
        dos_display(dos_path),
        image_path,
        &mut visited_directories,
    )
}

/// Reads the entry `meta` and everything below it through the start clusters
/// of the entries. A directory whose cluster the walk has read before fails.
fn read_image_node(
    fs: &mut FatFs<'_>,
    meta: &Metadata,
    display_path: String,
    image_path: &Path,
    visited_directories: &mut HashSet<u16>,
) -> crate::Result<CopyNode> {
    let kind = if meta.is_dir() {
        if !visited_directories.insert(meta.start_cluster()) {
            bail!(
                "directory cycle in image {}: {} leads to a directory that the copy has already read",
                image_path.display(),
                display_path
            );
        }
        let entries = fs
            .list_entry_dir(meta)
            .map_err(|error| map_entry_fat_err(error, &display_path))?;
        let mut children = Vec::new();
        for entry in entries {
            if entry.is_volume_label() {
                continue;
            }
            let child_path = format!("{display_path}\\{}", display_name(&entry.full_name));
            children.push(read_image_node(
                fs,
                &entry,
                child_path,
                image_path,
                visited_directories,
            )?);
        }
        info!(
            "read image:{}:{} ({} entries)",
            image_path.display(),
            display_path,
            children.len()
        );
        CopyKind::Dir(children)
    } else {
        let data = fs
            .read_entry(meta)
            .map_err(|error| map_entry_fat_err(error, &display_path))?;
        info!(
            "read image:{}:{} ({} bytes)",
            image_path.display(),
            display_path,
            data.len()
        );
        CopyKind::File(data)
    };
    Ok(CopyNode {
        name: meta.name.clone(),
        full_name: meta.full_name.clone(),
        display_path,
        kind,
    })
}

fn write_node_into_image(
    fs: &mut FatFs<'_>,
    node: &CopyNode,
    dos_target: &[u8],
    image_path: &Path,
    time: u16,
    date: u16,
) -> crate::Result<()> {
    match &node.kind {
        CopyKind::File(data) => {
            fs.write_file(dos_target, data, 0x20, time, date)
                .map_err(map_fat_err)?;
            info!(
                "copy -> image:{}:{} ({} bytes)",
                image_path.display(),
                dos_display(dos_target),
                data.len()
            );
        }
        CopyKind::Dir(children) => {
            for child in children {
                let child_target = join_dos(dos_target, &child.name);
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

/// Checks that the entries of each directory of `node` and below have
/// distinct 8.3 names ignoring the ASCII case. The error lists every
/// collision.
fn validate_image_target_names(node: &CopyNode) -> crate::Result<()> {
    let mut collisions = Vec::new();
    collect_target_name_collisions(node, &mut collisions);
    if collisions.is_empty() {
        return Ok(());
    }
    let mut message = String::from("the following image names share their 8.3 name:\n");
    for group in collisions {
        for path in group {
            message.push_str("  ");
            message.push_str(&path);
            message.push('\n');
        }
        message.push('\n');
    }
    message.push_str("no file was written");
    Err(StringError(message).into())
}

/// Collects the paths of the entries of each directory of `node` and below
/// that share their 8.3 name ignoring the ASCII case.
fn collect_target_name_collisions(node: &CopyNode, collisions: &mut Vec<Vec<String>>) {
    if let CopyKind::Dir(children) = &node.kind {
        for group in colliding_groups(children, |child| Some(child.name.to_ascii_uppercase())) {
            collisions.push(
                group
                    .iter()
                    .map(|child| child.display_path.clone())
                    .collect(),
            );
        }
        for child in children {
            collect_target_name_collisions(child, collisions);
        }
    }
}

/// Returns the groups of two or more entries of `nodes` with equal keys.
/// Entries without a key belong to no group.
fn colliding_groups<Key: Ord>(
    nodes: &[CopyNode],
    key: impl Fn(&CopyNode) -> Option<Key>,
) -> Vec<Vec<&CopyNode>> {
    let mut groups: BTreeMap<Key, Vec<&CopyNode>> = BTreeMap::new();
    for node in nodes {
        if let Some(node_key) = key(node) {
            groups.entry(node_key).or_default().push(node);
        }
    }
    groups
        .into_values()
        .filter(|group| group.len() > 1)
        .collect()
}

/// Checks that every entry of `nodes` and below has a valid host file name.
/// The error lists every entry that has none.
fn validate_image_host_names(nodes: &[CopyNode]) -> crate::Result<()> {
    let mut invalid = Vec::new();
    collect_invalid_host_names(nodes, &mut invalid);
    if invalid.is_empty() {
        return Ok(());
    }
    let mut message = String::from("the following image names cannot be host file names:\n");
    for line in invalid {
        message.push_str("  ");
        message.push_str(&line);
        message.push('\n');
    }
    message.push_str("no file was written");
    Err(StringError(message).into())
}

/// Collects a line for every entry of `nodes` and below without a valid host
/// file name and for every group of sibling host names that are equal
/// ignoring the case.
fn collect_invalid_host_names(nodes: &[CopyNode], invalid: &mut Vec<String>) {
    for node in nodes {
        if let Err(reason) = host_name_from_dos_bytes(&node.full_name) {
            invalid.push(format!("{} ({reason})", node.display_path));
        }
    }
    let host_name_key = |node: &CopyNode| {
        host_name_from_dos_bytes(&node.full_name)
            .ok()
            .map(|name| name.to_lowercase())
    };
    for group in colliding_groups(nodes, host_name_key) {
        let paths: Vec<&str> = group
            .iter()
            .map(|node| node.display_path.as_str())
            .collect();
        invalid.push(format!(
            "{} (the host names are equal ignoring the case)",
            paths.join(", ")
        ));
    }
    for node in nodes {
        if let CopyKind::Dir(children) = &node.kind {
            collect_invalid_host_names(children, invalid);
        }
    }
}

/// Writes the entries of `nodes` and below into the host directory `host_dir`.
fn write_nodes_to_host(
    nodes: &[CopyNode],
    host_dir: &Path,
    image_path: &Path,
) -> crate::Result<()> {
    for node in nodes {
        let child_host = host_dir.join(node.host_name()?);
        match &node.kind {
            CopyKind::Dir(children) => {
                std::fs::create_dir_all(&child_host).with_context(|| {
                    format!("failed to create directory {}", child_host.display())
                })?;
                info!("mkdir host:{}", child_host.display());
                write_nodes_to_host(children, &child_host, image_path)?;
            }
            CopyKind::File(data) => {
                std::fs::write(&child_host, data)
                    .with_context(|| format!("failed to write {}", child_host.display()))?;
                info!(
                    "copy image:{}:{} -> host:{}",
                    image_path.display(),
                    node.display_path,
                    child_host.display()
                );
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
        Some(ext) if FLOPPY_EXTENSIONS.contains(&ext) => {
            let image = load_floppy_image(path, &bytes)
                .with_context(|| format!("failed to parse FDD image {}", path.display()))?;
            FileImageDiskIo::new_fdd(image).map_err(|e| StringError(e).into())
        }
        _ => bail!("unrecognized image extension: {}", path.display()),
    }
}

/// Mounts the FAT volume of an image. A floppy without a FAT file system
/// fails with a message that names `image_path`.
fn mount_fat<'a>(io: &'a mut FileImageDiskIo, image_path: &Path) -> crate::Result<FatFs<'a>> {
    let drive_da = io.drive_da();
    let mut entry = pc98::drive_entry(drive_da)
        .ok_or_else(|| StringError(format!("unsupported DA/UA {drive_da:#04X}")))?;
    if io.uses_mbr_partitions() {
        entry.partition_scheme = PartitionScheme::MasterBootRecord;
    }
    FatFs::mount(io, &entry).map_err(|error| {
        let message = match error {
            FatError::UnknownMedia => format!(
                "{} holds no FAT file system. The copy tool reads FAT volumes only",
                image_path.display()
            ),
            other => format!("failed to mount FAT: {other}"),
        };
        StringError(message).into()
    })
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
        geometry: FloppyGeometry,
    },
}

impl FileImageDiskIo {
    /// Wraps a hard disk image. A PC-98 image laid out in its image geometry
    /// is migrated to the PC-98 IDE BIOS geometry. A SASI image that neetan
    /// formatted with its partition in cylinder 0 is migrated to the NEC
    /// layout.
    fn new_hdd(mut image: HddImage) -> Self {
        if pc98_ide::is_ide_image_format(image.format) {
            pc98_ide::migrate_to_bios_geometry(&mut image);
        }
        pc98_sasi::migrate_to_nec_layout(&mut image);
        Self {
            image: FileImage::Hdd(image),
            drive_da: 0x80,
        }
    }

    fn new_fdd(image: FloppyImage) -> Result<Self, String> {
        let Some(geometry) = image.physical_geometry() else {
            if image.format == FloppyFormat::IbmXdf {
                return Err(
                    "IBM XDF floppies use mixed-size sectors and cannot be accessed by the copy \
                     tool. Copy the files from inside the emulated DOS instead"
                        .to_string(),
                );
            }
            return Err("FDD image has no uniform track 0".to_string());
        };
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
                    let (_, sector) =
                        find_logical_sector(image, geometry, current).ok_or(0x40u8)?;
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
                    let (track_index, sector) =
                        find_logical_sector(image, geometry, current_lba).ok_or(0x40u8)?;
                    let (cylinder, head, record, size_code) = (
                        sector.cylinder,
                        sector.head,
                        sector.record,
                        sector.size_code,
                    );
                    image
                        .write_sector_payload(track_index, cylinder, head, record, size_code, chunk)
                        .ok_or(0x40u8)?;
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
            FileImage::Hdd(image) => {
                let geometry = hdd_chs_geometry(image);
                Some((
                    geometry.cylinders,
                    geometry.heads,
                    geometry.sectors_per_track,
                ))
            }
            FileImage::Fdd { geometry, .. } => Some((
                geometry.cylinders,
                geometry.heads,
                geometry.sectors_per_track,
            )),
        }
    }
}

/// Returns the geometry the partition table of a hard disk image uses.
fn hdd_chs_geometry(image: &HddImage) -> HddGeometry {
    if pc98_ide::is_ide_image_format(image.format) {
        pc98_ide::bios_geometry(image.geometry)
    } else {
        image.geometry
    }
}

/// Returns the cylinder, the head and the 0-based position in the track of
/// the logical sector `lba`, or `None` past the last cylinder.
fn lba_to_cylinder_head_rank(lba: u32, geometry: &FloppyGeometry) -> Option<(u8, u8, usize)> {
    let sectors_per_track = u32::from(geometry.sectors_per_track);
    let heads = u32::from(geometry.heads);
    let track = lba / sectors_per_track;
    let cylinder = track / heads;
    if cylinder >= u32::from(geometry.cylinders) {
        return None;
    }
    let head = (track % heads) as u8;
    let rank = (lba % sectors_per_track) as usize;
    Some((u8::try_from(cylinder).ok()?, head, rank))
}

/// Returns the track index of the lookup and the sector that holds the
/// logical sector `lba`. A missing sector and a sector with another byte
/// count give `None`.
fn find_logical_sector<'a>(
    image: &'a FloppyImage,
    geometry: &FloppyGeometry,
    lba: u32,
) -> Option<(usize, &'a D88Sector)> {
    let (cylinder, head, rank) = lba_to_cylinder_head_rank(lba, geometry)?;
    let (id_cylinder, id_head, record, size_code) =
        image.logical_sector_id(geometry, cylinder, head, rank)?;
    let track_index = usize::from(cylinder) * 2 + usize::from(head);
    let sector =
        image.find_sector_near_track_index(track_index, id_cylinder, id_head, record, size_code)?;
    (sector.data.len() == usize::from(geometry.sector_size)).then_some((track_index, sector))
}

fn map_fat_err(error: FatError) -> crate::Error {
    StringError(format!("FAT error: {error}")).into()
}

/// Returns the error of a failed access to the image entry at `display_path`.
fn map_entry_fat_err(error: FatError, display_path: &str) -> crate::Error {
    StringError(format!("FAT error at {display_path}: {error}")).into()
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

/// Decodes the Shift-JIS name bytes of an image entry into a host file name.
/// A name that breaks the Windows file name rules gives the reason as an
/// error on every host.
fn host_name_from_dos_bytes(name: &[u8]) -> Result<String, &'static str> {
    let decoded = shift_jis_to_string(name).ok_or("the name is no valid Shift-JIS text")?;
    if decoded.is_empty() || decoded == "." || decoded == ".." {
        return Err("the name is reserved on the host");
    }
    if decoded.chars().any(char::is_control) {
        return Err("the name holds a control character");
    }
    if decoded.contains(HOST_RESERVED_CHARACTERS) {
        return Err("the name holds a character that Windows forbids in a file name");
    }
    if decoded.ends_with('.') {
        return Err("the name ends in a dot");
    }
    if decoded.ends_with(' ') {
        return Err("the name ends in a space");
    }
    let stem = decoded.split('.').next().unwrap_or_default();
    if WINDOWS_DEVICE_NAMES
        .iter()
        .any(|device| device.eq_ignore_ascii_case(stem))
    {
        return Err("the name is a Windows device name");
    }
    Ok(decoded)
}

/// Returns the name bytes of an image entry as text for messages. Control
/// characters and bytes that do not decode as Shift-JIS appear as `\xNN`.
fn display_name(name: &[u8]) -> String {
    match shift_jis_to_string(name) {
        Some(decoded) => decoded
            .chars()
            .map(|character| {
                if character.is_control() {
                    format!("\\x{:02X}", u32::from(character))
                } else {
                    character.to_string()
                }
            })
            .collect(),
        None => name
            .iter()
            .map(|&byte| {
                if byte.is_ascii_graphic() || byte == b' ' {
                    char::from(byte).to_string()
                } else {
                    format!("\\x{byte:02X}")
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a geometry of `cylinders`, `heads` and `sectors_per_track` sectors of 1024 bytes.
    fn geometry(cylinders: u16, heads: u8, sectors_per_track: u8) -> FloppyGeometry {
        FloppyGeometry {
            cylinders,
            heads,
            sectors_per_track,
            sector_size: 1024,
        }
    }

    /// Logical sectors follow cylinder, head and position ordering.
    #[test]
    fn lba_to_cylinder_head_rank_of_double_sided_geometry() {
        let double_sided = geometry(77, 2, 8);
        assert_eq!(lba_to_cylinder_head_rank(0, &double_sided), Some((0, 0, 0)));
        assert_eq!(lba_to_cylinder_head_rank(7, &double_sided), Some((0, 0, 7)));
        assert_eq!(lba_to_cylinder_head_rank(8, &double_sided), Some((0, 1, 0)));
        assert_eq!(
            lba_to_cylinder_head_rank(16, &double_sided),
            Some((1, 0, 0))
        );
        assert_eq!(
            lba_to_cylinder_head_rank(1231, &double_sided),
            Some((76, 1, 7))
        );
        assert_eq!(lba_to_cylinder_head_rank(1232, &double_sided), None);
    }

    /// A single-sided geometry advances the cylinder after every track.
    #[test]
    fn lba_to_cylinder_head_rank_of_single_sided_geometry() {
        let single_sided = geometry(80, 1, 9);
        assert_eq!(lba_to_cylinder_head_rank(8, &single_sided), Some((0, 0, 8)));
        assert_eq!(lba_to_cylinder_head_rank(9, &single_sided), Some((1, 0, 0)));
        assert_eq!(
            lba_to_cylinder_head_rank(719, &single_sided),
            Some((79, 0, 8))
        );
        assert_eq!(lba_to_cylinder_head_rank(720, &single_sided), None);
    }

    /// Shift-JIS names decode, and names that the host cannot hold fail.
    #[test]
    fn host_names_decode_shift_jis_and_reject_reserved_characters() {
        assert_eq!(
            host_name_from_dos_bytes(b"\x83\x47\x83\x66\x83\x42\x83\x5E.X"),
            Ok("\u{30A8}\u{30C7}\u{30A3}\u{30BF}.X".to_string())
        );
        assert_eq!(
            host_name_from_dos_bytes(b"\x95\x5C.X"),
            Ok("\u{8868}.X".to_string())
        );
        assert_eq!(
            host_name_from_dos_bytes(b"\xCA\xDF.X"),
            Ok("\u{FF8A}\u{FF9F}.X".to_string())
        );
        for invalid in [
            &b"A/B.X"[..],
            b"A\\B.X",
            b"A:B",
            b"A*B",
            b"A?B",
            b"A\"B",
            b"A<B",
            b"A>B",
            b"A|B",
            b"A\0B",
            b"A\x01B",
            b"A\x7FB",
            b"\x81",
            b"",
            b".",
            b"..",
        ] {
            assert!(host_name_from_dos_bytes(invalid).is_err(), "{invalid:?}");
        }
        assert_eq!(
            host_name_from_dos_bytes(b"NAME."),
            Err("the name ends in a dot")
        );
        assert_eq!(
            host_name_from_dos_bytes(b"NAME "),
            Err("the name ends in a space")
        );
        for device in [
            &b"CON"[..],
            b"con.txt",
            b"PRN",
            b"Aux.X",
            b"NUL",
            b"COM1",
            b"com9.dat",
            b"LPT1",
            b"LPT9.X",
        ] {
            assert_eq!(
                host_name_from_dos_bytes(device),
                Err("the name is a Windows device name"),
                "{device:?}"
            );
        }
        for allowed in [&b"CONFIG.SYS"[..], b"COM10", b"LPT", b"NULL.X", b"AUXX"] {
            assert!(host_name_from_dos_bytes(allowed).is_ok(), "{allowed:?}");
        }
    }

    /// Messages show control characters and undecodable bytes as escapes.
    #[test]
    fn display_names_escape_control_characters_and_undecodable_bytes() {
        assert_eq!(display_name(b"FILE.TXT"), "FILE.TXT");
        assert_eq!(display_name(b"\x95\x5C.X"), "\u{8868}.X");
        assert_eq!(display_name(b"A\x01B"), "A\\x01B");
        assert_eq!(display_name(b"A\x81"), "A\\x81");
    }
}
