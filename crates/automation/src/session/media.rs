//! Media mounting: startup and runtime insert, eject, flush, and capture.

use std::{collections::BTreeMap, path::Path};

use common::{MediaBacking, MediaImage};

use super::{ActiveMachine, AutomationSession, OpError};
use crate::{
    capabilities::resolve_within,
    media::{MediaKind, MediaMount, MediaRequest},
};

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
        Self::check_media_supported(active, kind)?;
        if active.mounts.contains_key(&(kind, slot)) {
            Self::eject_from(active, kind, slot);
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
                    dirty: false,
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
                    dirty: false,
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
                    dirty: false,
                    printer_artifact: Some(resolved),
                }
            }
        };
        active.mounts.insert((kind, slot), mount.clone());
        Ok(mount)
    }

    /// Rejects a media kind the current machine does not support.
    ///
    /// Floppy and CD-ROM have no `StartupCapabilities` flag, so an unsupported
    /// drive surfaces later as the insert failure rather than here.
    fn check_media_supported(active: &ActiveMachine, kind: MediaKind) -> Result<(), OpError> {
        let capabilities = active.machine.startup_capabilities();
        let supported = match kind {
            MediaKind::Floppy | MediaKind::Cdrom => true,
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

    /// Flushes writable media and printer output, then clears the dirty flags.
    ///
    /// Cartridges are deliberately not flushed so a read-only cartridge is never
    /// written back to its baseline.
    pub fn media_flush(&mut self) -> Result<(), OpError> {
        let active = self.active.as_mut().ok_or(OpError::NoMachine)?;
        active.machine.flush_floppies();
        active.machine.flush_hdds();
        active.machine.flush_printer();
        for mount in active.mounts.values_mut() {
            mount.dirty = false;
        }
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

    /// Flushes writable media and releases mount records on session teardown.
    ///
    /// Called on every orderly termination path. Cartridges are excluded from the
    /// flush; RAM-backed disks have nothing to write through.
    pub fn flush_and_release_media(&mut self) {
        if self.has_machine() {
            let _ = self.media_flush();
        }
        if let Some(active) = self.active.as_mut() {
            active.mounts.clear();
        }
    }
}
