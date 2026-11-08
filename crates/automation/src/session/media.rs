//! Media mounting: startup and runtime insert, eject, save, capture, and the
//! creation and formatting of blank floppy and hard disks.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use common::{MediaBacking, MediaImage};
use device::{
    disk::{
        HddSizeType, blank_hdd_image,
        format::{FormatError, PartitionTableType, format_hdd_image},
        load_hdd_image,
    },
    floppy::{
        FddType, FloppyContainer, blank_floppy_image_bytes,
        format::{FloppyLayout, format_floppy_image},
        load_floppy_image,
    },
};
use machine_factory::config::{
    Target, model_name,
    settings::{
        blank_images::{BlankFormat, blank_images_for_id},
        file_types::{FLOPPY_ID, HARD_DISK_ID},
    },
    target_symbol_name,
};

use super::{ActiveMachine, AutomationSession, OpError};
use crate::{
    capabilities::resolve_within,
    media::{MediaKind, MediaMount, MediaRequest},
};

/// Returns the partition table the BIOS and DOS of the machine family
/// `target` read from a hard disk, or `None` for a family whose table
/// `format_hdd_image` does not write.
fn machine_partition_table(target: Target) -> Option<PartitionTableType> {
    match target {
        #[cfg(feature = "pc98")]
        Target::Pc98 => Some(PartitionTableType::Pc98),
        #[cfg(feature = "pc88")]
        Target::Pc88 => None,
        #[cfg(feature = "pc88va")]
        Target::Pc88Va => None,
        #[cfg(feature = "pc60")]
        Target::Pc60 => None,
        #[cfg(feature = "msx")]
        Target::Msx => None,
        #[cfg(feature = "towns")]
        Target::Towns => None,
        #[cfg(feature = "x1")]
        Target::X1 => None,
        #[cfg(feature = "fm7")]
        Target::Fm7 => None,
        #[cfg(feature = "x68k")]
        Target::X68k => None,
        #[cfg(feature = "at")]
        Target::At => Some(PartitionTableType::At),
    }
}

/// Returns the FAT12 floppy layout of the machine family `target`, or `None`
/// for a family whose floppies `format_floppy_image` does not write.
fn machine_floppy_layout(target: Target) -> Option<FloppyLayout> {
    match target {
        #[cfg(feature = "pc98")]
        Target::Pc98 => Some(FloppyLayout::Pc98),
        #[cfg(feature = "pc88")]
        Target::Pc88 => None,
        #[cfg(feature = "pc88va")]
        Target::Pc88Va => None,
        #[cfg(feature = "pc60")]
        Target::Pc60 => None,
        #[cfg(feature = "msx")]
        Target::Msx => Some(FloppyLayout::Msx),
        #[cfg(feature = "towns")]
        Target::Towns => Some(FloppyLayout::Towns),
        #[cfg(feature = "x1")]
        Target::X1 => None,
        #[cfg(feature = "fm7")]
        Target::Fm7 => None,
        #[cfg(feature = "x68k")]
        Target::X68k => None,
        #[cfg(feature = "at")]
        Target::At => Some(FloppyLayout::At),
    }
}

/// Returns the container of a blank floppy image with the file extension
/// `extension`.
fn floppy_container_of(extension: &str) -> FloppyContainer {
    if extension.eq_ignore_ascii_case("d88") {
        FloppyContainer::D88
    } else if extension.eq_ignore_ascii_case("fdi") {
        FloppyContainer::Fdi
    } else {
        FloppyContainer::Raw
    }
}

/// Returns the token of a blank image format, the text that the format
/// argument of `media-create!` takes.
fn blank_format_token(format: BlankFormat) -> &'static str {
    match format {
        BlankFormat::Floppy(fdd_type) => fdd_type.as_token(),
        BlankFormat::HardDisk(size) => size.as_token(),
    }
}

/// The prefix of the synthetic source of a blank floppy or hard disk.
const BLANK_SOURCE_PREFIX: &str = "blank:";

/// Returns the Scheme symbol name of a partition table type.
fn partition_table_symbol(table: PartitionTableType) -> &'static str {
    match table {
        PartitionTableType::Pc98 => "pc98",
        PartitionTableType::At => "at",
    }
}

impl AutomationSession {
    /// Returns a mount request for every currently mounted media entry.
    pub(super) fn mount_requests(&self) -> Vec<MediaRequest> {
        self.active
            .as_ref()
            .into_iter()
            .flat_map(|active| active.mounts.values())
            .map(|mount| MediaRequest {
                kind: mount.kind,
                slot: mount.slot,
                source: mount.requested.clone(),
            })
            .collect()
    }

    /// Reads back the current image bytes of every writable mounted disk.
    pub(super) fn capture_written_media(&self) -> BTreeMap<(MediaKind, usize), Vec<u8>> {
        let mut captured = BTreeMap::new();
        let Some(active) = self.active.as_ref() else {
            return captured;
        };
        for &(kind, slot) in active.mounts.keys() {
            let bytes = match kind {
                MediaKind::Floppy => active.machine.floppy_image_bytes(slot),
                MediaKind::Hdd => active.machine.hdd_image_bytes(slot),
                MediaKind::Cdrom
                | MediaKind::Cartridge
                | MediaKind::Cassette
                | MediaKind::Printer => None,
            };
            if let Some(bytes) = bytes {
                captured.insert((kind, slot), bytes);
            }
        }
        captured
    }

    /// Mounts one media request, replacing any existing mount in the same slot.
    ///
    /// Writable floppy and hard-disk fixtures are read into memory and mounted
    /// with a RAM backing so the on-disk source stays byte-identical. When
    /// `written` is supplied (a hard reset) it becomes the mounted image bytes so
    /// guest writes survive the reconstruction. Read-only media mounts from its
    /// resolved source path. Printer output is created beneath the artifact root.
    pub(super) fn mount(
        &mut self,
        request: &MediaRequest,
        written: Option<&[u8]>,
    ) -> Result<MediaMount, OpError> {
        let active = self.active.as_mut().ok_or(OpError::NoMachine)?;
        Self::mount_into(
            active,
            &self.read_root,
            &self.artifact_root,
            request,
            written,
        )
    }

    /// Mounts one request into a not-yet-committed active machine.
    pub(super) fn mount_into(
        active: &mut ActiveMachine,
        read_root: &Path,
        artifact_root: &Path,
        request: &MediaRequest,
        written: Option<&[u8]>,
    ) -> Result<MediaMount, OpError> {
        let kind = request.kind;
        let slot = request.slot;
        if slot >= kind.slot_count() {
            return Err(OpError::Argument(format!(
                "{} slot {slot} is out of range, expected 0..{}",
                kind.symbol(),
                kind.slot_count()
            )));
        }
        Self::check_media_supported(active, kind, slot)?;
        if active.mounts.contains_key(&(kind, slot)) {
            Self::eject_from(active, kind, slot);
        }

        // A synthetic "blank:" source mounts a zeroed disk built in memory,
        // with no host file to read.
        if matches!(kind, MediaKind::Floppy | MediaKind::Hdd)
            && let Some(token) = request.source.strip_prefix(BLANK_SOURCE_PREFIX)
        {
            return Self::mount_blank(active, request, token, written);
        }

        let root = match kind {
            MediaKind::Printer => artifact_root,
            MediaKind::Floppy
            | MediaKind::Hdd
            | MediaKind::Cdrom
            | MediaKind::Cartridge
            | MediaKind::Cassette => read_root,
        };
        let resolved = resolve_within(root, &request.source).map_err(OpError::PathEscape)?;
        let file_name = resolved
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("media")
            .to_owned();
        let format = resolved
            .extension()
            .and_then(|extension| extension.to_str())
            .map_or_else(|| "unknown".to_owned(), str::to_uppercase);

        let mount = match kind {
            MediaKind::Floppy | MediaKind::Hdd => {
                let bytes = match written {
                    Some(bytes) => bytes.to_vec(),
                    None => std::fs::read(&resolved).map_err(|error| {
                        OpError::Io(format!("cannot read {}: {error}", request.source))
                    })?,
                };
                let image = MediaImage {
                    name: &file_name,
                    bytes: &bytes,
                };
                let machine = &mut active.machine;
                let description = match kind {
                    MediaKind::Floppy => machine.insert_floppy(slot, image, MediaBacking::Ram),
                    MediaKind::Hdd => machine.insert_hdd(slot, image, MediaBacking::Ram),
                    MediaKind::Cdrom
                    | MediaKind::Cartridge
                    | MediaKind::Cassette
                    | MediaKind::Printer => unreachable!(),
                }
                .map_err(OpError::Io)?;
                MediaMount {
                    kind,
                    slot,
                    requested: request.source.clone(),
                    format,
                    description,
                    write_protected: false,
                    printer_artifact: None,
                }
            }
            MediaKind::Cdrom | MediaKind::Cartridge | MediaKind::Cassette => {
                let machine = &mut active.machine;
                let description = match kind {
                    MediaKind::Cdrom => machine.insert_cdrom(&resolved),
                    MediaKind::Cartridge => machine.insert_cartridge(&resolved),
                    MediaKind::Cassette => machine.insert_cassette(&resolved),
                    MediaKind::Floppy | MediaKind::Hdd | MediaKind::Printer => unreachable!(),
                }
                .map_err(OpError::Io)?;
                MediaMount {
                    kind,
                    slot,
                    requested: request.source.clone(),
                    format,
                    description,
                    write_protected: true,
                    printer_artifact: None,
                }
            }
            MediaKind::Printer => {
                std::fs::File::create(&resolved).map_err(|error| {
                    OpError::Io(format!(
                        "cannot create printer output {}: {error}",
                        request.source
                    ))
                })?;
                let machine = &mut active.machine;
                machine.attach_printer(&resolved).map_err(OpError::Io)?;
                MediaMount {
                    kind,
                    slot,
                    requested: request.source.clone(),
                    format,
                    description: format!("printer output to {}", request.source),
                    write_protected: false,
                    printer_artifact: Some(resolved),
                }
            }
        };
        active.mounts.insert((kind, slot), mount.clone());
        Ok(mount)
    }

    /// Synthesizes a blank disk from the token of a synthetic source and
    /// mounts it RAM-backed. A hard disk token is a size token such as
    /// `sasi40`. A floppy token is a type token and the file extension of the
    /// container, such as `2hd.d88`.
    ///
    /// On a hard-reset replay `written` carries the captured image so the
    /// formatted-and-installed content survives the reconstruction.
    fn mount_blank(
        active: &mut ActiveMachine,
        request: &MediaRequest,
        token: &str,
        written: Option<&[u8]>,
    ) -> Result<MediaMount, OpError> {
        let kind = request.kind;
        let slot = request.slot;
        let (extension, blank_bytes) = match kind {
            MediaKind::Hdd => {
                let size: HddSizeType = token
                    .parse()
                    .map_err(|message: String| OpError::Argument(message))?;
                (size.file_extension(), blank_hdd_image(size).to_bytes())
            }
            MediaKind::Floppy => {
                let (type_token, extension) = token.split_once('.').ok_or_else(|| {
                    OpError::Argument(format!(
                        "blank floppy source '{token}' needs a type and an extension"
                    ))
                })?;
                let fdd_type: FddType = type_token
                    .parse()
                    .map_err(|message: String| OpError::Argument(message))?;
                let container = floppy_container_of(extension);
                (extension, blank_floppy_image_bytes(fdd_type, container))
            }
            MediaKind::Cdrom | MediaKind::Cartridge | MediaKind::Cassette | MediaKind::Printer => {
                unreachable!()
            }
        };
        let name = format!("blank.{extension}");
        let bytes = match written {
            Some(bytes) => bytes.to_vec(),
            None => blank_bytes,
        };
        let media = MediaImage {
            name: &name,
            bytes: &bytes,
        };
        let machine = &mut active.machine;
        let description = match kind {
            MediaKind::Floppy => machine.insert_floppy(slot, media, MediaBacking::Ram),
            MediaKind::Hdd => machine.insert_hdd(slot, media, MediaBacking::Ram),
            MediaKind::Cdrom | MediaKind::Cartridge | MediaKind::Cassette | MediaKind::Printer => {
                unreachable!()
            }
        }
        .map_err(OpError::Io)?;
        let mount = MediaMount {
            kind,
            slot,
            requested: request.source.clone(),
            format: extension.to_uppercase(),
            description,
            write_protected: false,
            printer_artifact: None,
        };
        active.mounts.insert((kind, slot), mount.clone());
        Ok(mount)
    }

    /// Rejects a media kind or a floppy drive `slot` the current machine
    /// does not have.
    fn check_media_supported(
        active: &ActiveMachine,
        kind: MediaKind,
        slot: usize,
    ) -> Result<(), OpError> {
        let capabilities = active.machine.startup_capabilities();
        let supported = match kind {
            MediaKind::Floppy => slot < usize::from(capabilities.floppy_drives),
            MediaKind::Cdrom => capabilities.cdrom,
            MediaKind::Hdd => capabilities.hard_disk,
            MediaKind::Cartridge => capabilities.cartridge,
            MediaKind::Cassette => capabilities.cassette,
            MediaKind::Printer => capabilities.printer,
        };
        if supported {
            Ok(())
        } else {
            Err(OpError::Unsupported(format!(
                "{} media is not supported by this machine",
                kind.symbol()
            )))
        }
    }

    /// Ejects a mounted entry from the machine and forgets it.
    ///
    /// RAM-backed writes are discarded. Hard disks and the printer have no trait
    /// eject, so only the mount record is dropped for those kinds.
    fn eject_from(active: &mut ActiveMachine, kind: MediaKind, slot: usize) {
        let machine = &mut active.machine;
        match kind {
            MediaKind::Floppy => machine.eject_floppy(slot),
            MediaKind::Cdrom => machine.eject_cdrom(),
            MediaKind::Cartridge => machine.eject_cartridge(),
            MediaKind::Cassette => machine.eject_cassette(),
            MediaKind::Hdd | MediaKind::Printer => {}
        }
        active.mounts.remove(&(kind, slot));
    }

    /// Ejects one mount from the active machine.
    fn eject_mount(&mut self, kind: MediaKind, slot: usize) {
        if let Some(active) = self.active.as_mut() {
            Self::eject_from(active, kind, slot);
        }
    }

    /// Mounts media at runtime. Runtime inserts are not part of the startup set.
    pub fn media_insert(
        &mut self,
        kind: MediaKind,
        slot: usize,
        source: String,
    ) -> Result<MediaMount, OpError> {
        let request = MediaRequest { kind, slot, source };
        self.mount(&request, None)
    }

    /// Synthesizes a blank floppy or hard disk of `format` in memory and
    /// mounts it RAM-backed in `slot`. No host file is created or read. The
    /// format must be one of the blank images that the drive of the model
    /// takes. A floppy takes the container of the first listed image of its
    /// type.
    pub fn media_create(
        &mut self,
        kind: MediaKind,
        slot: usize,
        format: BlankFormat,
    ) -> Result<MediaMount, OpError> {
        let active = self.active.as_ref().ok_or(OpError::NoMachine)?;
        let (types_id, key_prefix) = match (kind, format) {
            (MediaKind::Floppy, BlankFormat::Floppy(_)) => (FLOPPY_ID, "fdd"),
            (MediaKind::Hdd, BlankFormat::HardDisk(_)) => (HARD_DISK_ID, "hdd"),
            (MediaKind::Floppy | MediaKind::Hdd, _) => {
                return Err(OpError::Argument(format!(
                    "'{}' is not a {} format",
                    blank_format_token(format),
                    kind.symbol()
                )));
            }
            (
                MediaKind::Cdrom | MediaKind::Cartridge | MediaKind::Cassette | MediaKind::Printer,
                _,
            ) => {
                return Err(OpError::Argument(format!(
                    "{} media cannot be created",
                    kind.symbol()
                )));
            }
        };
        let target = active.startup_spec.target;
        let model = model_name(&active.startup_spec);
        let key = format!("{key_prefix}{}", slot + 1);
        let entry = blank_images_for_id(types_id, target, &key, model).ok_or_else(|| {
            OpError::Unsupported(format!(
                "{} slot {slot} of the {model} takes no blank image",
                kind.symbol()
            ))
        })?;
        let image = entry
            .images
            .iter()
            .find(|image| image.format == format)
            .ok_or_else(|| {
                let mut tokens: Vec<&str> = entry
                    .images
                    .iter()
                    .map(|image| blank_format_token(image.format))
                    .collect();
                tokens.dedup();
                OpError::Argument(format!(
                    "the {model} takes no {} '{}', expected one of {}",
                    kind.symbol(),
                    blank_format_token(format),
                    tokens.join(", ")
                ))
            })?;
        let token = match format {
            BlankFormat::Floppy(fdd_type) => {
                format!("{}.{}", fdd_type.as_token(), image.extensions[0])
            }
            BlankFormat::HardDisk(size) => size.as_token().to_owned(),
        };
        let request = MediaRequest {
            kind,
            slot,
            source: format!("{BLANK_SOURCE_PREFIX}{token}"),
        };
        self.mount(&request, None)
    }

    /// Formats the floppy or hard disk mounted in `slot` in place with the
    /// FAT format of the machine family, then re-mounts the formatted image
    /// RAM-backed. A hard disk takes the partition table of the family. A
    /// `table` that differs from it is an argument error. A floppy takes the
    /// FAT12 layout of the family and no `table`. Intended to run before the
    /// guest boots.
    pub fn media_format(
        &mut self,
        kind: MediaKind,
        slot: usize,
        table: Option<PartitionTableType>,
    ) -> Result<(), OpError> {
        match kind {
            MediaKind::Hdd => self.format_hard_disk(slot, table),
            MediaKind::Floppy => {
                if table.is_some() {
                    return Err(OpError::Argument(
                        "a floppy takes no partition table".to_owned(),
                    ));
                }
                self.format_floppy(slot)
            }
            MediaKind::Cdrom | MediaKind::Cartridge | MediaKind::Cassette | MediaKind::Printer => {
                Err(OpError::Argument(format!(
                    "{} media cannot be formatted",
                    kind.symbol()
                )))
            }
        }
    }

    /// Formats the hard disk mounted in `slot` with `format_hdd_image` and the
    /// partition table of the machine family.
    fn format_hard_disk(
        &mut self,
        slot: usize,
        table: Option<PartitionTableType>,
    ) -> Result<(), OpError> {
        let active = self.active.as_mut().ok_or(OpError::NoMachine)?;
        let target = active.startup_spec.target;
        let machine_table = machine_partition_table(target).ok_or_else(|| {
            OpError::Unsupported(format!(
                "hard disks of the {} machine family cannot be formatted",
                target_symbol_name(target)
            ))
        })?;
        if let Some(table) = table
            && table != machine_table
        {
            return Err(OpError::Argument(format!(
                "the {} machine family takes the {} partition table, got {}",
                target_symbol_name(target),
                partition_table_symbol(machine_table),
                partition_table_symbol(table)
            )));
        }
        let mount = active
            .mounts
            .get(&(MediaKind::Hdd, slot))
            .cloned()
            .ok_or_else(|| OpError::Argument(format!("no hard disk is mounted in slot {slot}")))?;
        let bytes = active.machine.hdd_image_bytes(slot).ok_or_else(|| {
            OpError::Argument(format!("hard disk in slot {slot} cannot be read back"))
        })?;

        let name = format!("disk.{}", mount.format.to_ascii_lowercase());
        let mut image = load_hdd_image(Path::new(&name), &bytes)
            .map_err(|error| OpError::Io(format!("cannot parse hard disk image: {error}")))?;
        format_hdd_image(&mut image, machine_table)
            .map_err(|error| OpError::Io(format!("cannot format hard disk: {error}")))?;
        let formatted = image.to_bytes();

        let media = MediaImage {
            name: &name,
            bytes: &formatted,
        };
        active
            .machine
            .insert_hdd(slot, media, MediaBacking::Ram)
            .map_err(OpError::Io)?;
        Ok(())
    }

    /// Formats the floppy mounted in `slot` with `format_floppy_image` and the
    /// FAT12 layout of the machine family.
    fn format_floppy(&mut self, slot: usize) -> Result<(), OpError> {
        let active = self.active.as_mut().ok_or(OpError::NoMachine)?;
        let target = active.startup_spec.target;
        let layout = machine_floppy_layout(target).ok_or_else(|| {
            OpError::Unsupported(format!(
                "floppies of the {} machine family cannot be formatted",
                target_symbol_name(target)
            ))
        })?;
        let mount = active
            .mounts
            .get(&(MediaKind::Floppy, slot))
            .cloned()
            .ok_or_else(|| OpError::Argument(format!("no floppy is mounted in slot {slot}")))?;
        let bytes = active.machine.floppy_image_bytes(slot).ok_or_else(|| {
            OpError::Argument(format!("floppy in slot {slot} cannot be read back"))
        })?;

        let name = format!("disk.{}", mount.format.to_ascii_lowercase());
        let mut image = load_floppy_image(Path::new(&name), &bytes)
            .map_err(|error| OpError::Io(format!("cannot parse floppy image: {error}")))?;
        format_floppy_image(&mut image, layout).map_err(|error| match error {
            FormatError::UnsupportedGeometry => OpError::Unsupported(format!(
                "the floppy in slot {slot} has no format on the {} machine family",
                target_symbol_name(target)
            )),
            FormatError::ImageTooSmall | FormatError::SectorOutOfRange => {
                OpError::Io(format!("cannot format floppy: {error}"))
            }
        })?;
        let formatted = image.to_bytes();

        let media = MediaImage {
            name: &name,
            bytes: &formatted,
        };
        active
            .machine
            .insert_floppy(slot, media, MediaBacking::Ram)
            .map_err(OpError::Io)?;
        Ok(())
    }

    /// Returns the current image bytes of the floppy or hard disk mounted in
    /// `slot`, in the format of the mounted image.
    pub fn media_image_bytes(&self, kind: MediaKind, slot: usize) -> Result<Vec<u8>, OpError> {
        let active = self.active.as_ref().ok_or(OpError::NoMachine)?;
        let bytes = match kind {
            MediaKind::Floppy | MediaKind::Hdd => {
                if !active.mounts.contains_key(&(kind, slot)) {
                    return Err(OpError::Argument(format!(
                        "no {} media is mounted in slot {slot}",
                        kind.symbol()
                    )));
                }
                match kind {
                    MediaKind::Floppy => active.machine.floppy_image_bytes(slot),
                    MediaKind::Hdd => active.machine.hdd_image_bytes(slot),
                    MediaKind::Cdrom
                    | MediaKind::Cartridge
                    | MediaKind::Cassette
                    | MediaKind::Printer => unreachable!(),
                }
            }
            MediaKind::Cdrom | MediaKind::Cartridge | MediaKind::Cassette | MediaKind::Printer => {
                return Err(OpError::Unsupported(format!(
                    "{} media has no image to read back",
                    kind.symbol()
                )));
            }
        };
        bytes.ok_or_else(|| {
            OpError::Unsupported(format!(
                "{} media in slot {slot} cannot be read back on this machine",
                kind.symbol()
            ))
        })
    }

    /// Writes the current image of the floppy or hard disk mounted in `slot`
    /// beneath the artifact root. The extension of `path` must match the
    /// format of the mounted image so the saved file mounts again.
    pub fn media_save(
        &mut self,
        kind: MediaKind,
        slot: usize,
        path: &str,
    ) -> Result<PathBuf, OpError> {
        let bytes = self.media_image_bytes(kind, slot)?;
        let format = self
            .active
            .as_ref()
            .and_then(|active| active.mounts.get(&(kind, slot)))
            .map(|mount| mount.format.clone())
            .ok_or(OpError::NoMachine)?;
        let extension = Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("");
        if !extension.eq_ignore_ascii_case(&format) {
            return Err(OpError::Argument(format!(
                "the {} media in slot {slot} is a {format} image, the path {path} needs the \
                 .{} extension",
                kind.symbol(),
                format.to_ascii_lowercase()
            )));
        }
        self.write_artifact(path, &bytes)
    }

    /// Ejects mounted media from a slot.
    pub fn media_eject(&mut self, kind: MediaKind, slot: usize) -> Result<(), OpError> {
        if !self.has_machine() {
            return Err(OpError::NoMachine);
        }
        if !kind.supports_eject() {
            return Err(OpError::Unsupported(format!(
                "{} media cannot be ejected",
                kind.symbol()
            )));
        }
        if !self
            .active
            .as_ref()
            .expect("machine present")
            .mounts
            .contains_key(&(kind, slot))
        {
            return Err(OpError::Argument(format!(
                "no {} media is mounted in slot {slot}",
                kind.symbol()
            )));
        }
        self.eject_mount(kind, slot);
        Ok(())
    }

    /// Returns the mount in a slot, if one is present.
    #[must_use]
    pub fn media_info(&self, kind: MediaKind, slot: usize) -> Option<MediaMount> {
        self.active
            .as_ref()
            .and_then(|active| active.mounts.get(&(kind, slot)))
            .cloned()
    }

    /// Flushes the printer output and releases the mount records on session
    /// teardown. Floppies and hard disks are RAM-backed and write nothing.
    pub fn release_media(&mut self) {
        if let Some(active) = self.active.as_mut() {
            if let Err(error) = active.machine.flush_printer() {
                common::error!("Flushing the printer output on teardown failed: {error}");
            }
            active.mounts.clear();
        }
    }
}
