//! The hotkeys of a running game: Right Ctrl with one more key.

use gui::ui::HintKey;
use i18n::Key;
use sdl3::keyboard::Scancode;
use stack_vec::StackVec;

use crate::{
    media_dialog::{Drive, Drives},
    texts::{hotkeys as labels, keys},
};

/// The modifier key of every hotkey.
pub(crate) const MODIFIER: HintKey<'static> = HintKey::Key(keys::RIGHT_CTRL);

/// The hotkeys of a running machine, at most every hotkey.
pub(crate) type HotkeyList = StackVec<Hotkey, { Hotkey::ALL.len() }>;

/// The features of a running machine that decide which hotkeys work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HotkeyFeatures {
    /// The removable media drives of the machine.
    pub(crate) drives: Drives,
    /// Whether the machine has a mouse.
    pub(crate) mouse: bool,
    /// Whether the backend draws the CRT effect.
    pub(crate) crt_effect: bool,
    /// Whether the machine drives a composite monitor.
    pub(crate) composite_video: bool,
}

impl HotkeyFeatures {
    /// The hotkeys that work with these features, in the order of
    /// [`Hotkey::ALL`].
    pub(crate) fn hotkeys(&self) -> HotkeyList {
        Hotkey::ALL
            .into_iter()
            .filter(|hotkey| hotkey.available(self))
            .collect()
    }
}

/// A hotkey of a running game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hotkey {
    /// Shows the list of the hotkeys.
    Help,
    /// Ends the game.
    Exit,
    /// Switches between the window and fullscreen.
    Fullscreen,
    /// Captures or releases the mouse.
    MouseCapture,
    /// Rebuilds the machine.
    HardReset,
    /// Runs the machine fast while held.
    FastForward,
    /// Switches the CRT effect on or off.
    Crt,
    /// Selects the next scaling method.
    Scaling,
    /// Selects the next composite artifact phase.
    CompositePhase,
    /// Opens the media dialog on the drive: Right Ctrl with 1 for floppy
    /// drive 1, 2 for floppy drive 2 and 3 for the CD-ROM drive.
    Media(Drive),
    /// Captures the quick save.
    QuickSave,
    /// Restores the quick save.
    QuickLoad,
}

impl Hotkey {
    /// Every hotkey in the order of the help list.
    pub(crate) const ALL: [Hotkey; 14] = [
        Hotkey::Help,
        Hotkey::Exit,
        Hotkey::Fullscreen,
        Hotkey::MouseCapture,
        Hotkey::HardReset,
        Hotkey::FastForward,
        Hotkey::Crt,
        Hotkey::Scaling,
        Hotkey::CompositePhase,
        Hotkey::Media(Drive::Floppy(0)),
        Hotkey::Media(Drive::Floppy(1)),
        Hotkey::Media(Drive::CdRom),
        Hotkey::QuickSave,
        Hotkey::QuickLoad,
    ];

    /// The hotkey that `scancode` gives together with Right Ctrl.
    pub(crate) fn from_scancode(scancode: Scancode) -> Option<Hotkey> {
        Hotkey::ALL
            .into_iter()
            .find(|hotkey| hotkey.scancode() == scancode)
    }

    /// The key that gives the hotkey together with Right Ctrl.
    pub(crate) fn scancode(self) -> Scancode {
        match self {
            Hotkey::Help => Scancode::H,
            Hotkey::Exit => Scancode::Escape,
            Hotkey::Fullscreen => Scancode::Return,
            Hotkey::MouseCapture => Scancode::M,
            Hotkey::HardReset => Scancode::R,
            Hotkey::FastForward => Scancode::F,
            Hotkey::Crt => Scancode::C,
            Hotkey::Scaling => Scancode::S,
            Hotkey::CompositePhase => Scancode::P,
            Hotkey::Media(Drive::Floppy(0)) => Scancode::_1,
            Hotkey::Media(Drive::Floppy(_)) => Scancode::_2,
            Hotkey::Media(Drive::CdRom) => Scancode::_3,
            Hotkey::QuickSave => Scancode::_5,
            Hotkey::QuickLoad => Scancode::_9,
        }
    }

    /// The key cap of the key after Right Ctrl.
    pub(crate) fn key(self) -> HintKey<'static> {
        match self {
            Hotkey::Help => HintKey::Text("H"),
            Hotkey::Exit => HintKey::Key(keys::ESCAPE),
            Hotkey::Fullscreen => HintKey::Key(keys::ENTER),
            Hotkey::MouseCapture => HintKey::Text("M"),
            Hotkey::HardReset => HintKey::Text("R"),
            Hotkey::FastForward => HintKey::Text("F"),
            Hotkey::Crt => HintKey::Text("C"),
            Hotkey::Scaling => HintKey::Text("S"),
            Hotkey::CompositePhase => HintKey::Text("P"),
            Hotkey::Media(Drive::Floppy(0)) => HintKey::Text("1"),
            Hotkey::Media(Drive::Floppy(_)) => HintKey::Text("2"),
            Hotkey::Media(Drive::CdRom) => HintKey::Text("3"),
            Hotkey::QuickSave => HintKey::Text("5"),
            Hotkey::QuickLoad => HintKey::Text("9"),
        }
    }

    /// The key of the label of the hotkey in the help list.
    pub(crate) fn label(self) -> Key {
        match self {
            Hotkey::Help => labels::HELP,
            Hotkey::Exit => labels::EXIT,
            Hotkey::Fullscreen => labels::FULLSCREEN,
            Hotkey::MouseCapture => labels::MOUSE_CAPTURE,
            Hotkey::HardReset => labels::HARD_RESET,
            Hotkey::FastForward => labels::FAST_FORWARD,
            Hotkey::Crt => labels::CRT,
            Hotkey::Scaling => labels::SCALING,
            Hotkey::CompositePhase => labels::COMPOSITE_PHASE,
            Hotkey::Media(Drive::Floppy(0)) => labels::FLOPPY_1,
            Hotkey::Media(Drive::Floppy(_)) => labels::FLOPPY_2,
            Hotkey::Media(Drive::CdRom) => labels::CD_ROM,
            Hotkey::QuickSave => labels::QUICK_SAVE,
            Hotkey::QuickLoad => labels::QUICK_LOAD,
        }
    }

    /// Whether the hotkey works on a machine with `features`.
    pub(crate) fn available(self, features: &HotkeyFeatures) -> bool {
        match self {
            Hotkey::MouseCapture => features.mouse,
            Hotkey::Crt => features.crt_effect,
            Hotkey::CompositePhase => features.composite_video,
            Hotkey::Media(drive) => features.drives.contains(&drive),
            Hotkey::Help
            | Hotkey::Exit
            | Hotkey::Fullscreen
            | Hotkey::HardReset
            | Hotkey::FastForward
            | Hotkey::Scaling
            | Hotkey::QuickSave
            | Hotkey::QuickLoad => true,
        }
    }

    /// Whether the hotkey works while an overlay covers the paused game. The
    /// hotkeys of the overlays, the end of the game and fullscreen work.
    pub(crate) fn works_over_overlay(self) -> bool {
        match self {
            Hotkey::Help | Hotkey::Exit | Hotkey::Fullscreen | Hotkey::Media(_) => true,
            Hotkey::MouseCapture
            | Hotkey::HardReset
            | Hotkey::FastForward
            | Hotkey::Crt
            | Hotkey::Scaling
            | Hotkey::CompositePhase
            | Hotkey::QuickSave
            | Hotkey::QuickLoad => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{media_dialog::available_drives, texts::english};

    /// The features of a machine with `floppy_drives` floppy drives, a CD-ROM drive when
    /// `cdrom` is set, a mouse when `mouse` is set, the CRT effect when `crt_effect` is set and
    /// composite video when `composite_video` is set.
    fn features(
        floppy_drives: u8,
        cdrom: bool,
        mouse: bool,
        crt_effect: bool,
        composite_video: bool,
    ) -> HotkeyFeatures {
        HotkeyFeatures {
            drives: available_drives(floppy_drives, cdrom),
            mouse,
            crt_effect,
            composite_video,
        }
    }

    #[test]
    fn a_pc6000_with_two_drives_has_every_hotkey_but_the_mouse_and_the_cdrom() {
        let hotkeys = features(2, false, false, true, true).hotkeys();
        let expected: Vec<Hotkey> = Hotkey::ALL
            .into_iter()
            .filter(|&hotkey| {
                hotkey != Hotkey::Media(Drive::CdRom) && hotkey != Hotkey::MouseCapture
            })
            .collect();
        assert_eq!(*hotkeys, *expected);
    }

    #[test]
    fn missing_features_leave_their_hotkeys_out() {
        let hotkeys = features(1, true, true, false, false).hotkeys();
        for missing in [
            Hotkey::CompositePhase,
            Hotkey::Crt,
            Hotkey::Media(Drive::Floppy(1)),
        ] {
            assert!(!hotkeys.contains(&missing), "{missing:?}");
        }
        for present in [
            Hotkey::Media(Drive::Floppy(0)),
            Hotkey::Media(Drive::CdRom),
            Hotkey::MouseCapture,
            Hotkey::Help,
            Hotkey::Exit,
        ] {
            assert!(hotkeys.contains(&present), "{present:?}");
        }
        let hotkeys = features(0, false, false, true, false).hotkeys();
        assert!(hotkeys.contains(&Hotkey::Crt));
        assert!(
            hotkeys
                .iter()
                .all(|hotkey| !matches!(hotkey, Hotkey::Media(_)))
        );
    }

    #[test]
    fn the_overlay_hotkeys_end_the_game_and_switch_the_window_or_the_overlay() {
        let working: Vec<Hotkey> = Hotkey::ALL
            .into_iter()
            .filter(|hotkey| hotkey.works_over_overlay())
            .collect();
        assert_eq!(
            working,
            [
                Hotkey::Help,
                Hotkey::Exit,
                Hotkey::Fullscreen,
                Hotkey::Media(Drive::Floppy(0)),
                Hotkey::Media(Drive::Floppy(1)),
                Hotkey::Media(Drive::CdRom),
            ]
        );
    }

    #[test]
    fn every_hotkey_maps_back_from_its_scancode() {
        for hotkey in Hotkey::ALL {
            assert_eq!(Hotkey::from_scancode(hotkey.scancode()), Some(hotkey));
        }
        assert_eq!(Hotkey::from_scancode(Scancode::Q), None);
    }

    #[test]
    fn no_two_hotkeys_share_a_scancode_or_a_label() {
        for (index, hotkey) in Hotkey::ALL.iter().enumerate() {
            for other in &Hotkey::ALL[index + 1..] {
                assert_ne!(hotkey.scancode(), other.scancode(), "{hotkey:?} {other:?}");
                assert_ne!(
                    english().text(hotkey.label()),
                    english().text(other.label()),
                    "{hotkey:?} {other:?}"
                );
            }
        }
    }
}
