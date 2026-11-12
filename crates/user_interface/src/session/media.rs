//! The removable media of a session and its media dialog.

use std::convert::Infallible;

use common::{error, info};
use gui::component::{Status, run};
use stack_vec::StackVec;

use super::{Session, overlay::SessionOverlay};
use crate::{
    host::Host,
    media_dialog::{
        Drive, DriveMedia, Drives, HotkeyAction, MAXIMUM_DRIVES, MediaChoice, MediaDialog,
        available_drives, hotkey_action,
    },
};

impl Session {
    /// Writes the floppy disks back and ejects the floppy of `drive`.
    pub(super) fn eject_floppy(&mut self, drive: usize) {
        self.flush_floppies_before_swap();
        self.machine.eject_floppy(drive);
        match drive {
            0 => self.fdd1_index = None,
            1 => self.fdd2_index = None,
            _ => {}
        }
        info!("Ejected FDD{}", drive + 1);
    }

    /// Writes the floppy disks back and inserts the configured floppy image
    /// `index` into `drive`.
    pub(super) fn select_floppy(&mut self, drive: usize, index: usize) -> Result<(), String> {
        let entries = match drive {
            0 => &self.fdd1_entries,
            1 => &self.fdd2_entries,
            _ => return Err(format!("floppy drive {} is not available", drive + 1)),
        };

        if index >= entries.len() {
            return Err(format!("floppy image index {index} is not available"));
        }

        let path = entries[index].path.clone();
        self.flush_floppies_before_swap();
        match self.machine.insert_floppy_from_path(drive, &path) {
            Ok(description) => {
                info!(
                    "Selected FDD{}: {description} from {}",
                    drive + 1,
                    path.display()
                );
            }
            Err(error) => {
                error!("Failed to select FDD{}: {error}", drive + 1);
                return Err(error);
            }
        }

        match drive {
            0 => self.fdd1_index = Some(index),
            1 => self.fdd2_index = Some(index),
            _ => {}
        }
        Ok(())
    }

    /// Ejects the CD-ROM.
    pub(super) fn eject_cdrom(&mut self) {
        self.machine.eject_cdrom();
        self.cdrom_index = None;
        info!("Ejected CD-ROM");
    }

    /// Inserts the configured CD-ROM image `index`.
    pub(super) fn select_cdrom(&mut self, index: usize) -> Result<(), String> {
        if index >= self.cdrom_entries.len() {
            return Err(format!("CD-ROM image index {index} is not available"));
        }

        let path = &self.cdrom_entries[index].path;
        match self.machine.insert_cdrom(path) {
            Ok(description) => {
                info!("Selected CD-ROM: {description} from {}", path.display());
            }
            Err(error) => {
                error!("Failed to select CD-ROM: {error}");
                return Err(error);
            }
        }

        self.cdrom_index = Some(index);
        Ok(())
    }

    /// Ejects the cassette.
    pub(super) fn eject_cassette(&mut self) {
        self.machine.eject_cassette();
        info!("Ejected cassette");
    }

    /// Inserts the configured cassette.
    pub(super) fn select_cassette(&mut self) -> Result<(), String> {
        let path = self
            .config
            .cassette
            .clone()
            .ok_or_else(|| "cassette image is not configured".to_owned())?;
        match self.machine.insert_cassette(&path) {
            Ok(description) => info!("Selected cassette: {description}"),
            Err(error) => {
                error!("Failed to select cassette: {error}");
                return Err(error);
            }
        }
        Ok(())
    }

    /// The removable media drives of the machine, in the tab order of the
    /// media dialog.
    pub(super) fn media_drives(&self) -> Drives {
        let capabilities = self.machine.startup_capabilities();
        available_drives(capabilities.floppy_drives, capabilities.cdrom)
    }

    /// The index of the loaded image of `drive`.
    fn loaded_image(&self, drive: Drive) -> Option<usize> {
        match drive {
            Drive::Floppy(0) => self.fdd1_index,
            Drive::Floppy(_) => self.fdd2_index,
            Drive::CdRom => self.cdrom_index,
        }
    }

    /// Opens the media dialog on the tab of `drive`, switches to that tab or
    /// closes the dialog when it shows that tab already. An open help overlay
    /// gives way to the media dialog.
    pub(super) fn media_hotkey(&mut self, drive: Drive, host: &mut Host) {
        if !host.gui_available() {
            return;
        }
        let drives = self.media_drives();
        let open_tab = match &self.overlay {
            Some(SessionOverlay::Media(dialog)) => Some(dialog.tab()),
            Some(SessionOverlay::Help(_)) | None => None,
        };
        match hotkey_action(&drives, open_tab, drive) {
            HotkeyAction::Open(tab) => self.open_media_dialog(drives, tab, host),
            HotkeyAction::Switch(tab) => {
                let loaded = self.loaded_image(drive);
                if let Some(SessionOverlay::Media(dialog)) = self.overlay.as_mut() {
                    dialog.switch_tab(tab, loaded);
                }
                self.media_pass(host);
            }
            HotkeyAction::Close => self.close_overlay(host),
            HotkeyAction::Unavailable => {
                info!("This machine has no {} drive", drive.log_name());
            }
        }
    }

    /// Opens the media dialog over `drives` on the tab `tab`.
    fn open_media_dialog(&mut self, drives: Drives, tab: usize, host: &mut Host) {
        let loaded = self.loaded_image(drives[tab]);
        let dialog = MediaDialog::new(drives, tab, loaded);
        self.open_overlay(SessionOverlay::Media(Box::new(dialog)), host);
    }

    /// Runs one pass of the media dialog and applies the change that it closes
    /// with.
    pub(super) fn media_pass(&mut self, host: &mut Host) {
        let (Some(gui), Some(SessionOverlay::Media(dialog))) =
            (host.gui.as_mut(), self.overlay.as_mut())
        else {
            return;
        };
        let media: StackVec<DriveMedia, MAXIMUM_DRIVES> = dialog
            .drives()
            .iter()
            .map(|drive| match drive {
                Drive::Floppy(0) => DriveMedia {
                    entries: &self.fdd1_entries,
                    loaded: self.fdd1_index,
                },
                Drive::Floppy(_) => DriveMedia {
                    entries: &self.fdd2_entries,
                    loaded: self.fdd2_index,
                },
                Drive::CdRom => DriveMedia {
                    entries: &self.cdrom_entries,
                    loaded: self.cdrom_index,
                },
            })
            .collect();
        let status = gui.pass(|context| {
            let area = context.bounds();
            run(
                dialog.as_mut(),
                context,
                area,
                &media,
                &mut |never: Infallible| match never {},
            )
        });
        drop(media);
        gui.sync_text_input(&host.window);
        if self.apply_media_status(status) {
            self.close_overlay(host);
        }
    }

    /// Applies the change that the media dialog closes with in `status` and
    /// returns whether the dialog closes.
    fn apply_media_status(&mut self, status: Status<MediaChoice>) -> bool {
        match status {
            Status::Open => false,
            Status::Closed => true,
            Status::Done(MediaChoice::Insert { drive, index }) => {
                let _ = match drive {
                    Drive::Floppy(number) => self.select_floppy(number, index),
                    Drive::CdRom => self.select_cdrom(index),
                };
                true
            }
            Status::Done(MediaChoice::Eject { drive }) => {
                match drive {
                    Drive::Floppy(number) => self.eject_floppy(number),
                    Drive::CdRom => self.eject_cdrom(),
                }
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gui::component::Status;

    use super::super::test_support::{CallLog, fake_machine, session_with};
    use crate::media_dialog::{Drive, ImageEntry, MediaChoice};

    /// A session with two images for each floppy drive and the CD-ROM that records into `log`.
    fn session_with_media(log: &CallLog) -> super::Session {
        let mut session = session_with(fake_machine(log));
        session.fdd1_entries = vec![
            ImageEntry::new(PathBuf::from("first-a.d88")),
            ImageEntry::new(PathBuf::from("first-b.d88")),
        ];
        session.fdd2_entries = vec![
            ImageEntry::new(PathBuf::from("second-a.d88")),
            ImageEntry::new(PathBuf::from("second-b.d88")),
        ];
        session.cdrom_entries = vec![
            ImageEntry::new(PathBuf::from("disc-a.cue")),
            ImageEntry::new(PathBuf::from("disc-b.cue")),
        ];
        session
    }

    /// Verifies that an open dialog stays open and changes nothing.
    #[test]
    fn an_open_dialog_stays_open_and_changes_nothing() {
        let log = CallLog::default();
        let mut session = session_with_media(&log);
        assert!(!session.apply_media_status(Status::Open));
        assert!(log.borrow().is_empty());
        assert_eq!(session.fdd1_index, None);
        assert_eq!(session.fdd2_index, None);
        assert_eq!(session.cdrom_index, None);
    }

    /// Verifies that a closed dialog closes without a change of the media.
    #[test]
    fn a_closed_dialog_closes() {
        let log = CallLog::default();
        let mut session = session_with_media(&log);
        assert!(session.apply_media_status(Status::Closed));
        assert!(log.borrow().is_empty());
    }

    /// Verifies that an insert into floppy drive 1 selects its image and closes.
    #[test]
    fn an_insert_into_floppy_drive_one_selects_its_image_and_closes() {
        let log = CallLog::default();
        let mut session = session_with_media(&log);
        let choice = MediaChoice::Insert {
            drive: Drive::Floppy(0),
            index: 1,
        };
        assert!(session.apply_media_status(Status::Done(choice)));
        assert_eq!(*log.borrow(), ["floppies", "insert floppy 0"]);
        assert_eq!(session.fdd1_index, Some(1));
        assert_eq!(session.fdd2_index, None);
    }

    /// Verifies that an insert of an image that is not available changes nothing and still closes.
    #[test]
    fn a_failed_insert_still_closes() {
        let log = CallLog::default();
        let mut session = session_with_media(&log);
        let choice = MediaChoice::Insert {
            drive: Drive::Floppy(0),
            index: 5,
        };
        assert!(session.apply_media_status(Status::Done(choice)));
        assert!(log.borrow().is_empty());
        assert_eq!(session.fdd1_index, None);
    }

    /// Verifies that an insert into floppy drive 2 selects its image and closes.
    #[test]
    fn an_insert_into_floppy_drive_two_selects_its_image_and_closes() {
        let log = CallLog::default();
        let mut session = session_with_media(&log);
        let choice = MediaChoice::Insert {
            drive: Drive::Floppy(1),
            index: 0,
        };
        assert!(session.apply_media_status(Status::Done(choice)));
        assert_eq!(*log.borrow(), ["floppies", "insert floppy 1"]);
        assert_eq!(session.fdd1_index, None);
        assert_eq!(session.fdd2_index, Some(0));
    }

    /// Verifies that an insert into the CD-ROM drive selects its image and closes.
    #[test]
    fn an_insert_into_the_cdrom_drive_selects_its_image_and_closes() {
        let log = CallLog::default();
        let mut session = session_with_media(&log);
        let choice = MediaChoice::Insert {
            drive: Drive::CdRom,
            index: 1,
        };
        assert!(session.apply_media_status(Status::Done(choice)));
        assert_eq!(*log.borrow(), ["insert cdrom"]);
        assert_eq!(session.cdrom_index, Some(1));
    }

    /// Verifies that an eject of a floppy drive ejects its disk and closes.
    #[test]
    fn an_eject_of_a_floppy_drive_ejects_and_closes() {
        let log = CallLog::default();
        let mut session = session_with_media(&log);
        session.fdd1_index = Some(0);
        session.fdd2_index = Some(1);
        let choice = MediaChoice::Eject {
            drive: Drive::Floppy(1),
        };
        assert!(session.apply_media_status(Status::Done(choice)));
        assert_eq!(*log.borrow(), ["floppies", "eject floppy 1"]);
        assert_eq!(session.fdd1_index, Some(0));
        assert_eq!(session.fdd2_index, None);
    }

    /// Verifies that an eject of the CD-ROM drive ejects its disc and closes.
    #[test]
    fn an_eject_of_the_cdrom_drive_ejects_and_closes() {
        let log = CallLog::default();
        let mut session = session_with_media(&log);
        session.cdrom_index = Some(0);
        let choice = MediaChoice::Eject {
            drive: Drive::CdRom,
        };
        assert!(session.apply_media_status(Status::Done(choice)));
        assert_eq!(*log.borrow(), ["eject cdrom"]);
        assert_eq!(session.cdrom_index, None);
    }
}
