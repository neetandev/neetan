//! The quick save and the quick load of a session with the removable media
//! changes a snapshot asks for.

use std::time::Instant;

use common::{info, warn};
use save_state::{MachineStateBlob, MediaKind, MediaMismatch, MediaSlot, SaveStateError};

use super::{
    Session,
    removable_media::{
        RemovableMediaAction, RemovableMediaRollback, RemovableMediaSelection,
        RemovableMediaTransaction, apply_media_and_state_transactionally,
        plan_removable_media_actions,
    },
};
use crate::{host::Host, input::KeyboardForwardingState};

impl Session {
    /// Captures the machine state into the runtime snapshot.
    pub(super) fn quick_save(&mut self) {
        match self.machine.capture_state() {
            Ok(snapshot) => {
                self.runtime_snapshot = Some(snapshot);
                info!("Quick save captured");
            }
            Err(error) => warn!("Quick save failed: {error}"),
        }
    }

    /// Returns the current host-selected removable media state.
    fn removable_media_selection(&self, mismatch: &MediaMismatch) -> RemovableMediaSelection {
        let cassette = mismatch.entries().iter().find_map(|entry| {
            let binding = entry.expected().or(entry.active())?;
            (binding.slot.kind == MediaKind::Cassette).then_some(entry.active().is_some())
        });
        RemovableMediaSelection {
            floppy: [self.fdd1_index, self.fdd2_index],
            cdrom: self.cdrom_index,
            cassette,
        }
    }

    /// Applies one planned removable-media change.
    fn apply_removable_media_action(&mut self, action: RemovableMediaAction) -> Result<(), String> {
        match action {
            RemovableMediaAction::EjectFloppy { drive } => self.eject_floppy(drive),
            RemovableMediaAction::SelectFloppy { drive, index } => {
                self.select_floppy(drive, index)?;
            }
            RemovableMediaAction::EjectCdRom => self.eject_cdrom(),
            RemovableMediaAction::SelectCdRom { index } => {
                self.select_cdrom(index)?;
            }
            RemovableMediaAction::EjectCassette => self.eject_cassette(),
            RemovableMediaAction::SelectCassette => self.select_cassette()?,
        }
        Ok(())
    }

    /// Restores a previously recorded removable-media selection.
    fn restore_removable_media_selection(
        &mut self,
        rollback: &RemovableMediaRollback,
    ) -> Result<(), String> {
        let selection = rollback.selection;
        for drive in 0..2 {
            let current = match drive {
                0 => self.fdd1_index,
                1 => self.fdd2_index,
                _ => unreachable!(),
            };
            let slot = MediaSlot::new(MediaKind::Floppy, drive as u32);
            if current == selection.floppy[drive] && !rollback.slots.contains(&slot) {
                continue;
            }
            match selection.floppy[drive] {
                Some(index) => self.select_floppy(drive, index)?,
                None => self.eject_floppy(drive),
            }
        }
        let cdrom_slot = MediaSlot::new(MediaKind::CdRom, 0);
        if self.cdrom_index != selection.cdrom || rollback.slots.contains(&cdrom_slot) {
            match selection.cdrom {
                Some(index) => self.select_cdrom(index)?,
                None => self.eject_cdrom(),
            }
        }
        let cassette_slot = MediaSlot::new(MediaKind::Cassette, 0);
        if let Some(cassette_mounted) = selection.cassette
            && rollback.slots.contains(&cassette_slot)
        {
            if cassette_mounted {
                self.select_cassette()?;
            } else {
                self.eject_cassette();
            }
        }
        Ok(())
    }

    /// Plans snapshot media changes and captures their rollback selection.
    fn removable_media_transaction(
        &mut self,
        mismatch: &MediaMismatch,
    ) -> Result<(Vec<RemovableMediaAction>, RemovableMediaRollback), String> {
        let actions = plan_removable_media_actions(
            mismatch,
            &self.fdd1_entries,
            &self.fdd2_entries,
            &self.cdrom_entries,
            self.config.cassette.as_deref(),
        )?;
        let previous = RemovableMediaRollback {
            selection: self.removable_media_selection(mismatch),
            slots: actions.iter().map(|action| action.slot()).collect(),
        };
        Ok((actions, previous))
    }

    /// Restores one runtime snapshot, including its removable-media set.
    fn restore_runtime_snapshot(
        &mut self,
        snapshot: &MachineStateBlob,
    ) -> Result<(), SaveStateError> {
        let mismatch = match self.machine.restore_state(snapshot) {
            Err(SaveStateError::MediaMismatch(mismatch)) => mismatch,
            result => return result,
        };
        info!("Quick load is changing removable media: {mismatch}");
        let rollback_snapshot = self.machine.capture_state()?;
        let (actions, previous_media) =
            self.removable_media_transaction(&mismatch)
                .map_err(|error| {
                    SaveStateError::InvalidInvariant(format!(
                        "quick load media change failed: {mismatch}: {error}"
                    ))
                })?;
        apply_media_and_state_transactionally(
            self,
            RemovableMediaTransaction {
                actions,
                previous_media,
                previous_state: rollback_snapshot,
            },
            Session::apply_removable_media_action,
            |session| {
                session
                    .machine
                    .restore_state(snapshot)
                    .map_err(|error| error.to_string())
            },
            Session::restore_removable_media_selection,
            |session, rollback_snapshot| {
                session
                    .machine
                    .restore_state(rollback_snapshot)
                    .map_err(|error| error.to_string())
            },
        )
        .map_err(|error| {
            SaveStateError::InvalidInvariant(format!(
                "quick load media change failed: {mismatch}: {error}"
            ))
        })
    }

    /// Restores the runtime snapshot and resumes the audio.
    pub(super) fn quick_load(&mut self, host: &mut Host) {
        let Some(snapshot) = self.runtime_snapshot.take() else {
            warn!("Quick load ignored because no runtime snapshot exists");
            return;
        };
        let restore_result = self.restore_runtime_snapshot(&snapshot);
        self.runtime_snapshot = Some(snapshot);
        match restore_result {
            Ok(()) => {
                self.cycle_overshoot = 0;
                self.mouse_dx = 0.0;
                self.mouse_dy = 0.0;
                self.keyboard_forwarding_state = KeyboardForwardingState::new();
                self.machine.set_mouse_buttons(
                    self.mouse_left,
                    self.mouse_right,
                    self.mouse_middle,
                );
                self.update_joystick(host);
                self.last_emulation_tick = Instant::now();
                self.set_fast_forward(false, host);
                host.audio_engine.reset_buffer();
                host.audio_engine.resume();
                info!("Quick load restored");
            }
            Err(error) => warn!("Quick load failed: {error}"),
        }
    }
}
