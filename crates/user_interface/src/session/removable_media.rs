//! The plan and the transaction of removable media changes that a runtime
//! snapshot asks for.

use std::path::Path;

use save_state::{MediaKind, MediaMismatch, MediaSlot, MediaSourcePath};

use crate::media_dialog::ImageEntry;

/// One planned change of a removable media drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RemovableMediaAction {
    /// Ejects the floppy of `drive`.
    EjectFloppy {
        /// The floppy drive, counted from zero.
        drive: usize,
    },
    /// Inserts the configured floppy image `index` into `drive`.
    SelectFloppy {
        /// The floppy drive, counted from zero.
        drive: usize,
        /// The index of the image in the configured list of the drive.
        index: usize,
    },
    /// Ejects the CD-ROM.
    EjectCdRom,
    /// Inserts the configured CD-ROM image `index`.
    SelectCdRom {
        /// The index of the image in the configured CD-ROM list.
        index: usize,
    },
    /// Ejects the cassette.
    EjectCassette,
    /// Inserts the configured cassette.
    SelectCassette,
}

impl RemovableMediaAction {
    /// Returns the host-visible slot affected by this action.
    pub(super) fn slot(self) -> MediaSlot {
        match self {
            Self::EjectFloppy { drive } | Self::SelectFloppy { drive, .. } => {
                MediaSlot::new(MediaKind::Floppy, drive as u32)
            }
            Self::EjectCdRom | Self::SelectCdRom { .. } => MediaSlot::new(MediaKind::CdRom, 0),
            Self::EjectCassette | Self::SelectCassette => MediaSlot::new(MediaKind::Cassette, 0),
        }
    }
}

/// The host-selected removable media of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RemovableMediaSelection {
    /// The loaded image index of each floppy drive.
    pub(super) floppy: [Option<usize>; 2],
    /// The loaded CD-ROM image index.
    pub(super) cdrom: Option<usize>,
    /// Whether the cassette is mounted, when the snapshot involves the cassette.
    pub(super) cassette: Option<bool>,
}

/// The media selection to restore when a media change fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RemovableMediaRollback {
    /// The selection before the change.
    pub(super) selection: RemovableMediaSelection,
    /// The slots the change touches.
    pub(super) slots: Vec<MediaSlot>,
}

/// A media change together with the state to restore on failure.
pub(super) struct RemovableMediaTransaction<State> {
    /// The planned changes, in order.
    pub(super) actions: Vec<RemovableMediaAction>,
    /// The media selection before the change.
    pub(super) previous_media: RemovableMediaRollback,
    /// The machine state before the change.
    pub(super) previous_state: State,
}

/// Finds a configured image by its normalized logical path.
fn configured_media_index(
    entries: &[ImageEntry],
    expected_path: &MediaSourcePath,
) -> Option<usize> {
    entries
        .iter()
        .position(|entry| MediaSourcePath::from_path(&entry.path) == *expected_path)
}

/// Plans a complete removable-media reconciliation without changing the machine.
pub(super) fn plan_removable_media_actions(
    mismatch: &MediaMismatch,
    fdd1_entries: &[ImageEntry],
    fdd2_entries: &[ImageEntry],
    cdrom_entries: &[ImageEntry],
    cassette_path: Option<&Path>,
) -> Result<Vec<RemovableMediaAction>, String> {
    let mut actions: Vec<RemovableMediaAction> = Vec::new();
    for entry in mismatch.entries() {
        let expected = entry.expected();
        let active = entry.active();
        let slot = expected.or(active).unwrap().slot;
        if expected.is_some_and(|binding| binding.slot != slot)
            || active.is_some_and(|binding| binding.slot != slot)
        {
            return Err("save state and active media use different logical slots".to_owned());
        }
        if actions.iter().any(|action| action.slot() == slot) {
            return Err(format!(
                "multiple media bindings refer to {:?} drive {}",
                slot.kind,
                slot.index + 1
            ));
        }

        let action = match slot.kind {
            MediaKind::Floppy => {
                let drive = slot.index as usize;
                let configured_entries = match drive {
                    0 => fdd1_entries,
                    1 => fdd2_entries,
                    _ => return Err(format!("floppy drive {} is not host-selectable", drive + 1)),
                };
                match expected {
                    Some(binding) => {
                        let expected_path = binding.source_path.as_ref().ok_or_else(|| {
                            format!("floppy drive {} has no configured source path", drive + 1)
                        })?;
                        let index = configured_media_index(configured_entries, expected_path)
                            .ok_or_else(|| {
                                format!(
                                    "expected floppy for drive {} is not configured: {}",
                                    drive + 1,
                                    expected_path
                                )
                            })?;
                        RemovableMediaAction::SelectFloppy { drive, index }
                    }
                    None => RemovableMediaAction::EjectFloppy { drive },
                }
            }
            MediaKind::CdRom => {
                if slot.index != 0 {
                    return Err(format!(
                        "CD-ROM drive {} is not host-selectable",
                        slot.index + 1
                    ));
                }
                match expected {
                    Some(binding) => {
                        let expected_path = binding
                            .source_path
                            .as_ref()
                            .ok_or_else(|| "CD-ROM has no configured source path".to_owned())?;
                        let index = configured_media_index(cdrom_entries, expected_path)
                            .ok_or_else(|| {
                                format!("expected CD-ROM is not configured: {expected_path}")
                            })?;
                        RemovableMediaAction::SelectCdRom { index }
                    }
                    None => RemovableMediaAction::EjectCdRom,
                }
            }
            MediaKind::HardDisk => {
                return Err(format!("hard disk drive {} differs", slot.index + 1));
            }
            MediaKind::Cassette => {
                if slot.index != 0 {
                    return Err(format!(
                        "cassette deck {} is not host-selectable",
                        slot.index + 1
                    ));
                }
                match expected {
                    Some(binding) => {
                        let expected_path = binding
                            .source_path
                            .as_ref()
                            .ok_or_else(|| "cassette has no configured source path".to_owned())?;
                        let configured_path = cassette_path
                            .ok_or_else(|| "expected cassette is not configured".to_owned())?;
                        if MediaSourcePath::from_path(configured_path) != *expected_path {
                            return Err(format!(
                                "expected cassette is not configured: {expected_path}"
                            ));
                        }
                        RemovableMediaAction::SelectCassette
                    }
                    None => RemovableMediaAction::EjectCassette,
                }
            }
        };
        actions.push(action);
    }
    Ok(actions)
}

/// Applies media and machine state together, restoring both on failure.
pub(super) fn apply_media_and_state_transactionally<
    Target,
    State,
    ApplyMedia,
    ApplyState,
    RestoreMedia,
    RestoreState,
>(
    target: &mut Target,
    transaction: RemovableMediaTransaction<State>,
    mut apply_media: ApplyMedia,
    apply_state: ApplyState,
    restore_media: RestoreMedia,
    restore_state: RestoreState,
) -> Result<(), String>
where
    ApplyMedia: FnMut(&mut Target, RemovableMediaAction) -> Result<(), String>,
    ApplyState: FnOnce(&mut Target) -> Result<(), String>,
    RestoreMedia: FnOnce(&mut Target, &RemovableMediaRollback) -> Result<(), String>,
    RestoreState: FnOnce(&mut Target, &State) -> Result<(), String>,
{
    let RemovableMediaTransaction {
        actions,
        previous_media,
        previous_state,
    } = transaction;
    for action in actions {
        if let Err(error) = apply_media(target, action) {
            return Err(rollback_media_and_state(
                target,
                &previous_media,
                &previous_state,
                error,
                restore_media,
                restore_state,
            ));
        }
    }
    if let Err(error) = apply_state(target) {
        return Err(rollback_media_and_state(
            target,
            &previous_media,
            &previous_state,
            error,
            restore_media,
            restore_state,
        ));
    }
    Ok(())
}

/// Restores the prior media and machine state and combines rollback failures.
fn rollback_media_and_state<Target, State, RestoreMedia, RestoreState>(
    target: &mut Target,
    previous_media: &RemovableMediaRollback,
    previous_state: &State,
    error: String,
    restore_media: RestoreMedia,
    restore_state: RestoreState,
) -> String
where
    RestoreMedia: FnOnce(&mut Target, &RemovableMediaRollback) -> Result<(), String>,
    RestoreState: FnOnce(&mut Target, &State) -> Result<(), String>,
{
    let media_error = restore_media(target, previous_media).err();
    let state_error = restore_state(target, previous_state).err();
    match (media_error, state_error) {
        (None, None) => error,
        (Some(rollback_error), None) => {
            format!("{error}; restoring the previous media also failed: {rollback_error}")
        }
        (None, Some(rollback_error)) => {
            format!("{error}; restoring the previous machine state also failed: {rollback_error}")
        }
        (Some(media_error), Some(state_error)) => format!(
            "{error}; restoring the previous media also failed: {media_error}; restoring the previous machine state also failed: {state_error}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use save_state::{
        MediaBinding, MediaBindingId, MediaKind, MediaManifest, MediaMismatch, MediaSlot,
        MediaSourcePath, ResourceIdentity,
    };

    use super::{
        RemovableMediaAction, RemovableMediaRollback, RemovableMediaSelection,
        RemovableMediaTransaction, apply_media_and_state_transactionally,
        plan_removable_media_actions,
    };
    use crate::media_dialog::ImageEntry;

    fn media_binding(
        identifier: &str,
        kind: MediaKind,
        index: u32,
        path: Option<&str>,
    ) -> MediaBinding {
        MediaBinding {
            identifier: MediaBindingId::new(identifier).unwrap(),
            slot: MediaSlot::new(kind, index),
            source_path: path.map(|path| MediaSourcePath::from_path(Path::new(path))),
            media_type: match kind {
                MediaKind::Floppy => "floppy",
                MediaKind::HardDisk => "hard-disk",
                MediaKind::CdRom => "cdrom",
                MediaKind::Cassette => "cassette",
            }
            .to_owned(),
            identity: ResourceIdentity::from_bytes(identifier.as_bytes()),
            geometry: None,
            write_protected: kind == MediaKind::CdRom,
            backend_generation: None,
        }
    }

    fn media_mismatch(expected: Vec<MediaBinding>, active: Vec<MediaBinding>) -> MediaMismatch {
        MediaManifest::new(expected)
            .unwrap()
            .compare_current(&MediaManifest::new(active).unwrap())
            .unwrap()
            .unwrap()
    }

    #[test]
    fn removable_media_plan_selects_and_ejects_all_differing_drives() {
        let mismatch = media_mismatch(
            vec![
                media_binding(
                    "cassette-0",
                    MediaKind::Cassette,
                    0,
                    Some("media/program.tap"),
                ),
                media_binding("floppy-0", MediaKind::Floppy, 0, Some("./media/first.d88")),
                media_binding("floppy-1", MediaKind::Floppy, 1, Some("media/second.d88")),
            ],
            vec![
                media_binding("cdrom-0", MediaKind::CdRom, 0, Some("media/disc.cue")),
                media_binding("floppy-0", MediaKind::Floppy, 0, Some("media/other.d88")),
            ],
        );
        let fdd1_entries = vec![
            ImageEntry::new(PathBuf::from("media/other.d88")),
            ImageEntry::new(PathBuf::from("games/../media/first.d88")),
        ];
        let fdd2_entries = vec![ImageEntry::new(PathBuf::from("media/second.d88"))];
        let cdrom_entries = vec![ImageEntry::new(PathBuf::from("media/disc.cue"))];

        let actions = plan_removable_media_actions(
            &mismatch,
            &fdd1_entries,
            &fdd2_entries,
            &cdrom_entries,
            Some(Path::new("games/../media/program.tap")),
        )
        .unwrap();

        assert_eq!(
            actions,
            vec![
                RemovableMediaAction::SelectCassette,
                RemovableMediaAction::EjectCdRom,
                RemovableMediaAction::SelectFloppy { drive: 0, index: 1 },
                RemovableMediaAction::SelectFloppy { drive: 1, index: 0 },
            ]
        );
    }

    #[test]
    fn removable_media_plan_rejects_unconfigured_and_fixed_media() {
        let unconfigured = media_mismatch(
            vec![media_binding(
                "floppy-0",
                MediaKind::Floppy,
                0,
                Some("missing.d88"),
            )],
            Vec::new(),
        );
        assert!(
            plan_removable_media_actions(&unconfigured, &[], &[], &[], None)
                .unwrap_err()
                .contains("not configured")
        );

        let hard_disk = media_mismatch(
            vec![media_binding(
                "ide-0",
                MediaKind::HardDisk,
                0,
                Some("disk.hdd"),
            )],
            Vec::new(),
        );
        assert!(
            plan_removable_media_actions(&hard_disk, &[], &[], &[], None)
                .unwrap_err()
                .contains("hard disk")
        );

        let cassette = media_mismatch(
            vec![media_binding(
                "cassette-0",
                MediaKind::Cassette,
                0,
                Some("program.tap"),
            )],
            Vec::new(),
        );
        assert!(
            plan_removable_media_actions(&cassette, &[], &[], &[], None)
                .unwrap_err()
                .contains("not configured")
        );
    }

    #[test]
    fn failed_media_switch_restores_the_previous_selection() {
        #[derive(Default)]
        struct FakeMediaTarget {
            applied: Vec<RemovableMediaAction>,
            restored: Option<RemovableMediaRollback>,
            state: u32,
        }

        let previous = RemovableMediaRollback {
            selection: RemovableMediaSelection {
                floppy: [Some(3), Some(4)],
                cdrom: Some(2),
                cassette: Some(true),
            },
            slots: vec![
                MediaSlot::new(MediaKind::Floppy, 0),
                MediaSlot::new(MediaKind::Floppy, 1),
            ],
        };
        let actions = vec![
            RemovableMediaAction::SelectFloppy { drive: 0, index: 0 },
            RemovableMediaAction::SelectFloppy { drive: 1, index: 1 },
        ];
        let mut target = FakeMediaTarget {
            state: 99,
            ..FakeMediaTarget::default()
        };
        let result = apply_media_and_state_transactionally(
            &mut target,
            RemovableMediaTransaction {
                actions,
                previous_media: previous.clone(),
                previous_state: 7,
            },
            |target, action| {
                target.applied.push(action);
                target.state += 1;
                if target.applied.len() == 2 {
                    return Err("second switch failed".into());
                }
                Ok(())
            },
            |_| Ok(()),
            |target, selection| {
                target.restored = Some(selection.clone());
                Ok(())
            },
            |target, state| {
                target.state = *state;
                Ok(())
            },
        );

        assert_eq!(result, Err("second switch failed".into()));
        assert_eq!(target.restored, Some(previous));
        assert_eq!(target.state, 7);
    }

    #[test]
    fn successful_media_switch_applies_snapshot_without_rollback() {
        #[derive(Default)]
        struct FakeTarget {
            media: Vec<RemovableMediaAction>,
            media_restored: bool,
            state: u32,
        }

        let mut target = FakeTarget {
            state: 7,
            ..Default::default()
        };
        let result = apply_media_and_state_transactionally(
            &mut target,
            RemovableMediaTransaction {
                actions: vec![
                    RemovableMediaAction::SelectFloppy { drive: 0, index: 1 },
                    RemovableMediaAction::SelectCdRom { index: 2 },
                ],
                previous_media: RemovableMediaRollback {
                    selection: RemovableMediaSelection {
                        floppy: [Some(0), None],
                        cdrom: Some(0),
                        cassette: None,
                    },
                    slots: vec![
                        MediaSlot::new(MediaKind::Floppy, 0),
                        MediaSlot::new(MediaKind::CdRom, 0),
                    ],
                },
                previous_state: 7,
            },
            |target, action| {
                target.media.push(action);
                Ok(())
            },
            |target| {
                target.state = 99;
                Ok(())
            },
            |target, _| {
                target.media_restored = true;
                Ok(())
            },
            |target, state| {
                target.state = *state;
                Ok(())
            },
        );

        assert_eq!(result, Ok(()));
        assert_eq!(
            target.media,
            vec![
                RemovableMediaAction::SelectFloppy { drive: 0, index: 1 },
                RemovableMediaAction::SelectCdRom { index: 2 },
            ],
        );
        assert_eq!(target.state, 99);
        assert!(!target.media_restored);
    }

    #[test]
    fn failed_state_restore_rolls_back_media_and_machine_state() {
        #[derive(Default)]
        struct FakeTarget {
            media_restored: bool,
            state: u32,
        }

        let previous = RemovableMediaRollback {
            selection: RemovableMediaSelection {
                floppy: [Some(1), None],
                cdrom: None,
                cassette: None,
            },
            slots: vec![MediaSlot::new(MediaKind::Floppy, 0)],
        };
        let mut target = FakeTarget {
            state: 42,
            ..Default::default()
        };
        let result = apply_media_and_state_transactionally(
            &mut target,
            RemovableMediaTransaction {
                actions: vec![RemovableMediaAction::SelectFloppy { drive: 0, index: 0 }],
                previous_media: previous,
                previous_state: 7,
            },
            |target, _| {
                target.state = 43;
                Ok(())
            },
            |target| {
                target.state = 100;
                Err("snapshot payload is invalid".to_owned())
            },
            |target, _| {
                target.media_restored = true;
                Ok(())
            },
            |target, state| {
                target.state = *state;
                Ok(())
            },
        );

        assert_eq!(result, Err("snapshot payload is invalid".to_owned()));
        assert!(target.media_restored);
        assert_eq!(target.state, 7);
    }
}
