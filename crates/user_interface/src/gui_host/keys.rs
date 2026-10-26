//! The mapping of the key, pointer and text events of SDL to the events of
//! the user interface.

use common::warn;
use gui::ui::{EditCommand, EditModifiers, Navigation, Shortcut};
use sdl3::{
    Error,
    event::Event,
    keyboard::{Mod, Scancode},
};

use super::GuiEvent;

/// The event of a pressed key. A repeated key press counts only for the
/// navigation keys, backspace, delete and paste. Left Ctrl with Page Up and
/// Page Down switches the tab like the shoulder buttons. Delete is the edit
/// command [`EditCommand::DeleteForward`]. The command key with A selects
/// all, with C copies, with X cuts and with V pastes. A paste calls
/// `read_clipboard` for the text of its event while `text_input` is set,
/// see [`paste_event`]. Without `text_input` the paste key reads nothing
/// and gives no event. The right control key belongs to the shell and never
/// forms a shortcut or an edit command.
pub(super) fn key_event(
    scancode: Scancode,
    keymod: &Mod,
    repeat: bool,
    text_input: bool,
    read_clipboard: impl FnOnce() -> Result<Option<String>, Error>,
) -> Option<GuiEvent> {
    let control = keymod.lctrl();
    let navigation = match scancode {
        Scancode::Up => Some(Navigation::Up),
        Scancode::Down => Some(Navigation::Down),
        Scancode::Left => Some(Navigation::Left),
        Scancode::Right => Some(Navigation::Right),
        Scancode::PageUp if control => Some(Navigation::PreviousTab),
        Scancode::PageDown if control => Some(Navigation::NextTab),
        Scancode::PageUp => Some(Navigation::PageUp),
        Scancode::PageDown => Some(Navigation::PageDown),
        Scancode::Home => Some(Navigation::Home),
        Scancode::End => Some(Navigation::End),
        Scancode::Tab if keymod.shift() => Some(Navigation::Previous),
        Scancode::Tab => Some(Navigation::Next),
        _ => None,
    };
    if let Some(navigation) = navigation {
        return Some(GuiEvent::KeyNavigation(
            navigation,
            edit_modifiers(scancode, keymod),
        ));
    }
    if scancode == Scancode::Backspace {
        return Some(GuiEvent::Backspace);
    }
    if keymod.rctrl() {
        return None;
    }
    if scancode == Scancode::Delete {
        return Some(GuiEvent::Edit(EditCommand::DeleteForward));
    }
    if scancode == Scancode::V && command_modifier(keymod) {
        if !text_input {
            return None;
        }
        return paste_event(read_clipboard());
    }
    if repeat {
        return None;
    }
    let shortcut = match scancode {
        Scancode::Return | Scancode::KpEnter => return Some(GuiEvent::Confirm),
        Scancode::Escape => return Some(GuiEvent::Cancel),
        Scancode::A if command_modifier(keymod) => {
            return Some(GuiEvent::Edit(EditCommand::SelectAll));
        }
        Scancode::C if command_modifier(keymod) => {
            return Some(GuiEvent::Edit(EditCommand::Copy));
        }
        Scancode::X if command_modifier(keymod) => {
            return Some(GuiEvent::Edit(EditCommand::Cut));
        }
        Scancode::Insert => Shortcut::Insert,
        Scancode::F2 => Shortcut::F2,
        Scancode::F5 => Shortcut::F5,
        Scancode::F10 => Shortcut::F10,
        Scancode::F if control => Shortcut::ControlF,
        Scancode::D if control => Shortcut::ControlD,
        Scancode::S if control => Shortcut::ControlS,
        _ => return None,
    };
    Some(GuiEvent::Shortcut(shortcut))
}

/// The paste event of the clipboard read `read`. A text gives the event. An
/// empty clipboard gives none, and so does a failed read, which logs a
/// warning.
fn paste_event(read: Result<Option<String>, Error>) -> Option<GuiEvent> {
    match read {
        Ok(Some(text)) if !text.is_empty() => Some(GuiEvent::Paste(text)),
        Ok(_) => None,
        Err(error) => {
            warn!("Failed to read the clipboard: {error}");
            None
        }
    }
}

/// The modifiers of the navigation key `scancode` with the held keys
/// `keymod`. Shift extends the selection with left, right, home and end.
/// The word modifier turns left and right into word steps, and the edge
/// modifier into steps to the start and the end.
fn edit_modifiers(scancode: Scancode, keymod: &Mod) -> EditModifiers {
    let horizontal = matches!(scancode, Scancode::Left | Scancode::Right);
    let cursor_key = horizontal || matches!(scancode, Scancode::Home | Scancode::End);
    EditModifiers {
        extend: cursor_key && keymod.shift(),
        word: horizontal && word_modifier(keymod),
        edge: horizontal && edge_modifier(keymod),
    }
}

/// Whether the modifier of a word step is held: Option on macOS.
#[cfg(target_os = "macos")]
fn word_modifier(keymod: &Mod) -> bool {
    keymod.alt()
}

/// Whether the modifier of a word step is held: the left control key.
#[cfg(not(target_os = "macos"))]
fn word_modifier(keymod: &Mod) -> bool {
    keymod.lctrl()
}

/// Whether the modifier of a step to the start or the end of a text is
/// held: Cmd on macOS.
#[cfg(target_os = "macos")]
fn edge_modifier(keymod: &Mod) -> bool {
    keymod.gui()
}

/// Whether the modifier of a step to the start or the end of a text is
/// held. Only macOS has one. Home and end take the step everywhere.
#[cfg(not(target_os = "macos"))]
fn edge_modifier(_keymod: &Mod) -> bool {
    false
}

/// Whether the command key of select all, copy, cut and paste is held: Cmd
/// on macOS.
#[cfg(target_os = "macos")]
fn command_modifier(keymod: &Mod) -> bool {
    keymod.gui()
}

/// Whether the command key of select all, copy, cut and paste is held: the
/// left control key.
#[cfg(not(target_os = "macos"))]
fn command_modifier(keymod: &Mod) -> bool {
    keymod.lctrl()
}

/// Whether Shift is held after `event` with `shift_held` before it. A key
/// press and a key release carry the modifier keys that the keyboard holds
/// after them. Every other event keeps `shift_held`.
pub(crate) fn shift_after(event: &Event, shift_held: bool) -> bool {
    match event {
        Event::KeyDown { keymod, .. } | Event::KeyUp { keymod, .. } => keymod.shift(),
        _ => shift_held,
    }
}

/// The edit modifiers held with a press. Shift gives the extend modifier.
fn pointer_modifiers(shift_held: bool) -> EditModifiers {
    EditModifiers {
        extend: shift_held,
        word: false,
        edge: false,
    }
}

/// The event of a mouse, text or input method event of SDL with Shift held
/// when `shift_held` is set. Keyboard and gamepad events have their own
/// mapping.
pub(crate) fn pointer_or_text_event(event: &Event, shift_held: bool) -> Option<GuiEvent> {
    match event {
        Event::MouseMotion { x, y, .. } => Some(GuiEvent::PointerMoved { x: *x, y: *y }),
        Event::MouseButtonDown {
            mouse_btn,
            clicks,
            x,
            y,
        } => Some(GuiEvent::Button {
            button: *mouse_btn,
            pressed: true,
            double: *clicks >= 2,
            modifiers: pointer_modifiers(shift_held),
            x: *x,
            y: *y,
        }),
        Event::MouseButtonUp {
            mouse_btn, x, y, ..
        } => Some(GuiEvent::Button {
            button: *mouse_btn,
            pressed: false,
            double: false,
            modifiers: EditModifiers::default(),
            x: *x,
            y: *y,
        }),
        Event::MouseWheel {
            y,
            mouse_x,
            mouse_y,
            ..
        } => Some(GuiEvent::Wheel {
            steps: *y,
            x: *mouse_x,
            y: *mouse_y,
        }),
        Event::TextInput { text } => Some(GuiEvent::Text(text.clone())),
        Event::TextEditing { text, .. } => Some(GuiEvent::Composition(text.clone())),
        _ => None,
    }
}

#[cfg(test)]
pub(super) mod tests {
    use sdl3::mouse::MouseButton;

    use super::*;

    /// No modifier keys.
    pub(crate) const NO_MODIFIERS: Mod = Mod(0);

    /// The left shift key.
    const SHIFT: Mod = Mod(0x0001);

    /// The right shift key.
    const RIGHT_SHIFT: Mod = Mod(0x0002);

    /// The left control key.
    pub(crate) const LEFT_CONTROL: Mod = Mod(0x0040);

    /// The right control key.
    const RIGHT_CONTROL: Mod = Mod(0x0080);

    /// The left shift key and the left control key.
    const SHIFT_LEFT_CONTROL: Mod = Mod(0x0041);

    /// The left Alt key, Option on macOS.
    const LEFT_ALT: Mod = Mod(0x0100);

    /// The left GUI key, Cmd on macOS.
    const LEFT_GUI: Mod = Mod(0x0400);

    /// The left shift key and the left GUI key.
    #[cfg(target_os = "macos")]
    const SHIFT_LEFT_GUI: Mod = Mod(0x0401);

    /// No edit modifiers.
    pub(crate) const NO_EDIT_MODIFIERS: EditModifiers = EditModifiers {
        extend: false,
        word: false,
        edge: false,
    };

    /// The edit modifier that extends the selection.
    pub(crate) const EXTEND: EditModifiers = EditModifiers {
        extend: true,
        ..NO_EDIT_MODIFIERS
    };

    /// The edit modifier of a word step.
    pub(crate) const WORD: EditModifiers = EditModifiers {
        word: true,
        ..NO_EDIT_MODIFIERS
    };

    /// The command key of select all, copy, cut and paste of the platform.
    #[cfg(target_os = "macos")]
    pub(crate) const COMMAND: Mod = LEFT_GUI;

    /// The command key of select all, copy, cut and paste of the platform.
    #[cfg(not(target_os = "macos"))]
    pub(crate) const COMMAND: Mod = LEFT_CONTROL;

    /// The command key of the other platform, which forms no edit command.
    #[cfg(target_os = "macos")]
    const OTHER_COMMAND: Mod = LEFT_CONTROL;

    /// The command key of the other platform, which forms no edit command.
    #[cfg(not(target_os = "macos"))]
    const OTHER_COMMAND: Mod = LEFT_GUI;

    /// A clipboard read that fails the test. Only the paste key reads the
    /// clipboard.
    pub(crate) fn unread_clipboard() -> Result<Option<String>, Error> {
        panic!("only the paste key reads the clipboard")
    }

    /// A clipboard read that gives `text`.
    pub(crate) fn clipboard_with(text: &str) -> impl FnOnce() -> Result<Option<String>, Error> {
        move || Ok(Some(text.to_owned()))
    }

    /// Verifies that the navigation keys, confirm, cancel and backspace map to their events.
    #[test]
    fn keys_map_to_navigation_and_actions() {
        let key = |scancode, keymod: &Mod, repeat| {
            key_event(scancode, keymod, repeat, true, unread_clipboard)
        };
        assert_eq!(
            key(Scancode::Up, &NO_MODIFIERS, false),
            Some(GuiEvent::KeyNavigation(Navigation::Up, NO_EDIT_MODIFIERS))
        );
        assert_eq!(
            key(Scancode::PageDown, &NO_MODIFIERS, false),
            Some(GuiEvent::KeyNavigation(
                Navigation::PageDown,
                NO_EDIT_MODIFIERS
            ))
        );
        assert_eq!(
            key(Scancode::End, &NO_MODIFIERS, false),
            Some(GuiEvent::KeyNavigation(Navigation::End, NO_EDIT_MODIFIERS))
        );
        assert_eq!(
            key(Scancode::Tab, &NO_MODIFIERS, false),
            Some(GuiEvent::KeyNavigation(Navigation::Next, NO_EDIT_MODIFIERS))
        );
        assert_eq!(
            key(Scancode::Tab, &SHIFT, false),
            Some(GuiEvent::KeyNavigation(
                Navigation::Previous,
                NO_EDIT_MODIFIERS
            ))
        );
        assert_eq!(
            key(Scancode::Return, &NO_MODIFIERS, false),
            Some(GuiEvent::Confirm)
        );
        assert_eq!(
            key(Scancode::KpEnter, &NO_MODIFIERS, false),
            Some(GuiEvent::Confirm)
        );
        assert_eq!(
            key(Scancode::Escape, &NO_MODIFIERS, false),
            Some(GuiEvent::Cancel)
        );
        assert_eq!(
            key(Scancode::Backspace, &NO_MODIFIERS, false),
            Some(GuiEvent::Backspace)
        );
        assert_eq!(key(Scancode::A, &NO_MODIFIERS, false), None);
    }

    /// Verifies that a repeated press counts for the navigation keys, backspace, delete and
    /// paste, and for no other key.
    #[test]
    fn only_navigation_backspace_delete_and_paste_repeat() {
        assert_eq!(
            key_event(Scancode::Down, &NO_MODIFIERS, true, true, unread_clipboard),
            Some(GuiEvent::KeyNavigation(Navigation::Down, NO_EDIT_MODIFIERS))
        );
        assert_eq!(
            key_event(Scancode::Left, &SHIFT, true, true, unread_clipboard),
            Some(GuiEvent::KeyNavigation(Navigation::Left, EXTEND))
        );
        assert_eq!(
            key_event(
                Scancode::Backspace,
                &NO_MODIFIERS,
                true,
                true,
                unread_clipboard
            ),
            Some(GuiEvent::Backspace)
        );
        assert_eq!(
            key_event(
                Scancode::Delete,
                &NO_MODIFIERS,
                true,
                true,
                unread_clipboard
            ),
            Some(GuiEvent::Edit(EditCommand::DeleteForward))
        );
        assert_eq!(
            key_event(Scancode::V, &COMMAND, true, true, clipboard_with("disk")),
            Some(GuiEvent::Paste("disk".into()))
        );
        assert_eq!(
            key_event(Scancode::C, &COMMAND, true, true, unread_clipboard),
            None
        );
        assert_eq!(
            key_event(Scancode::X, &COMMAND, true, true, unread_clipboard),
            None
        );
        assert_eq!(
            key_event(Scancode::A, &LEFT_CONTROL, true, true, unread_clipboard),
            None
        );
        assert_eq!(
            key_event(Scancode::A, &LEFT_GUI, true, true, unread_clipboard),
            None
        );
        assert_eq!(
            key_event(
                Scancode::Return,
                &NO_MODIFIERS,
                true,
                true,
                unread_clipboard
            ),
            None
        );
        assert_eq!(
            key_event(
                Scancode::KpEnter,
                &NO_MODIFIERS,
                true,
                true,
                unread_clipboard
            ),
            None
        );
        assert_eq!(
            key_event(
                Scancode::Escape,
                &NO_MODIFIERS,
                true,
                true,
                unread_clipboard
            ),
            None
        );
    }

    /// Verifies that mouse, text and composition events map to their events.
    #[test]
    fn mouse_and_text_events_map_to_the_input() {
        assert_eq!(
            pointer_or_text_event(
                &Event::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    clicks: 2,
                    x: 10.0,
                    y: 20.0,
                },
                false
            ),
            Some(GuiEvent::Button {
                button: MouseButton::Left,
                pressed: true,
                double: true,
                modifiers: NO_EDIT_MODIFIERS,
                x: 10.0,
                y: 20.0,
            })
        );
        assert_eq!(
            pointer_or_text_event(
                &Event::MouseWheel {
                    x: 0.0,
                    y: -1.0,
                    mouse_x: 3.0,
                    mouse_y: 4.0,
                },
                false
            ),
            Some(GuiEvent::Wheel {
                steps: -1.0,
                x: 3.0,
                y: 4.0,
            })
        );
        assert_eq!(
            pointer_or_text_event(&Event::TextInput { text: "か".into() }, false),
            Some(GuiEvent::Text("か".into()))
        );
        assert_eq!(
            pointer_or_text_event(
                &Event::TextEditing {
                    text: "かな".into(),
                    start: 2,
                    length: 0,
                },
                false
            ),
            Some(GuiEvent::Composition("かな".into()))
        );
        assert_eq!(pointer_or_text_event(&Event::Quit, false), None);
    }

    /// A press of the left shift key with the held keys `keymod`.
    fn shift_down(keymod: Mod) -> Event {
        Event::KeyDown {
            scancode: Some(Scancode::LShift),
            keymod,
            repeat: false,
        }
    }

    /// A release of the left shift key with the held keys `keymod`.
    fn shift_up(keymod: Mod) -> Event {
        Event::KeyUp {
            scancode: Some(Scancode::LShift),
            keymod,
            repeat: false,
        }
    }

    /// A left press at `(5, 6)`.
    fn left_press() -> Event {
        Event::MouseButtonDown {
            mouse_btn: MouseButton::Left,
            clicks: 1,
            x: 5.0,
            y: 6.0,
        }
    }

    /// A left release at `(5, 6)`.
    fn left_release() -> Event {
        Event::MouseButtonUp {
            mouse_btn: MouseButton::Left,
            clicks: 1,
            x: 5.0,
            y: 6.0,
        }
    }

    /// The left button event at `(5, 6)`, pressed when `pressed` is set,
    /// with `modifiers`.
    fn left_button(pressed: bool, modifiers: EditModifiers) -> GuiEvent {
        GuiEvent::Button {
            button: MouseButton::Left,
            pressed,
            double: false,
            modifiers,
            x: 5.0,
            y: 6.0,
        }
    }

    /// The pointer events of the batch `events` with the Shift state that
    /// the key events before each of them leave, in the order of the batch.
    fn pointer_events(events: &[Event]) -> Vec<GuiEvent> {
        let mut shift_held = false;
        events
            .iter()
            .filter_map(|event| {
                shift_held = shift_after(event, shift_held);
                pointer_or_text_event(event, shift_held)
            })
            .collect()
    }

    /// Verifies that the modifier keys of a key press and a key release give the held Shift,
    /// that the control keys, Alt and the GUI key hold no Shift and that other events keep it.
    #[test]
    fn the_key_events_give_the_held_shift() {
        for keymod in [SHIFT, RIGHT_SHIFT, SHIFT_LEFT_CONTROL] {
            assert!(shift_after(&shift_down(keymod), false));
        }
        for keymod in [
            NO_MODIFIERS,
            LEFT_CONTROL,
            RIGHT_CONTROL,
            LEFT_ALT,
            LEFT_GUI,
        ] {
            assert!(!shift_after(&shift_down(keymod), true));
        }
        assert!(!shift_after(&shift_up(NO_MODIFIERS), true));
        assert!(shift_after(&shift_up(RIGHT_SHIFT), false));
        assert!(shift_after(&left_press(), true));
        assert!(!shift_after(&left_press(), false));
        assert!(shift_after(&Event::TextInput { text: "A".into() }, true));
    }

    /// Verifies that a press between a Shift press and its release in one batch extends the
    /// selection, that a press before the Shift press or after its release does not, and that a
    /// release carries no modifier.
    #[test]
    fn shift_with_a_button_press_extends_the_selection_in_event_order() {
        assert_eq!(
            pointer_events(&[
                shift_down(SHIFT),
                left_press(),
                shift_up(NO_MODIFIERS),
                left_release(),
            ]),
            [
                left_button(true, EXTEND),
                left_button(false, NO_EDIT_MODIFIERS)
            ]
        );
        assert_eq!(
            pointer_events(&[
                left_press(),
                shift_down(SHIFT),
                left_release(),
                shift_up(NO_MODIFIERS),
                left_press(),
            ]),
            [
                left_button(true, NO_EDIT_MODIFIERS),
                left_button(false, NO_EDIT_MODIFIERS),
                left_button(true, NO_EDIT_MODIFIERS)
            ]
        );
    }

    /// Verifies the shortcut keys, the left control key and that the right control key forms no shortcut.
    #[test]
    fn shortcut_keys_map_to_shortcuts() {
        let key =
            |scancode, keymod: &Mod| key_event(scancode, keymod, false, true, unread_clipboard);
        assert_eq!(
            key(Scancode::Delete, &NO_MODIFIERS),
            Some(GuiEvent::Edit(EditCommand::DeleteForward))
        );
        assert_eq!(
            key(Scancode::Insert, &NO_MODIFIERS),
            Some(GuiEvent::Shortcut(Shortcut::Insert))
        );
        assert_eq!(
            key(Scancode::F2, &NO_MODIFIERS),
            Some(GuiEvent::Shortcut(Shortcut::F2))
        );
        assert_eq!(
            key(Scancode::F5, &NO_MODIFIERS),
            Some(GuiEvent::Shortcut(Shortcut::F5))
        );
        assert_eq!(
            key(Scancode::F10, &NO_MODIFIERS),
            Some(GuiEvent::Shortcut(Shortcut::F10))
        );
        assert_eq!(
            key(Scancode::F, &LEFT_CONTROL),
            Some(GuiEvent::Shortcut(Shortcut::ControlF))
        );
        assert_eq!(
            key(Scancode::D, &LEFT_CONTROL),
            Some(GuiEvent::Shortcut(Shortcut::ControlD))
        );
        assert_eq!(
            key(Scancode::S, &LEFT_CONTROL),
            Some(GuiEvent::Shortcut(Shortcut::ControlS))
        );
        assert_eq!(key(Scancode::S, &NO_MODIFIERS), None);
        assert_eq!(key(Scancode::S, &RIGHT_CONTROL), None);
        assert_eq!(key(Scancode::Delete, &RIGHT_CONTROL), None);
        assert_eq!(
            key(Scancode::PageUp, &LEFT_CONTROL),
            Some(GuiEvent::KeyNavigation(
                Navigation::PreviousTab,
                NO_EDIT_MODIFIERS
            ))
        );
        assert_eq!(
            key(Scancode::PageDown, &LEFT_CONTROL),
            Some(GuiEvent::KeyNavigation(
                Navigation::NextTab,
                NO_EDIT_MODIFIERS
            ))
        );
        assert_eq!(
            key(Scancode::PageDown, &RIGHT_CONTROL),
            Some(GuiEvent::KeyNavigation(
                Navigation::PageDown,
                NO_EDIT_MODIFIERS
            ))
        );
    }

    /// Verifies that Shift extends the selection with the cursor keys, that the left control key
    /// turns left and right into word steps and forms select all with A, and that the right
    /// control key and the GUI key form neither outside macOS.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn edit_keys_map_to_steps_and_commands() {
        let key =
            |scancode, keymod: &Mod| key_event(scancode, keymod, false, true, unread_clipboard);
        for (scancode, navigation) in [
            (Scancode::Left, Navigation::Left),
            (Scancode::Right, Navigation::Right),
            (Scancode::Home, Navigation::Home),
            (Scancode::End, Navigation::End),
        ] {
            assert_eq!(
                key(scancode, &SHIFT),
                Some(GuiEvent::KeyNavigation(navigation, EXTEND))
            );
        }
        for (scancode, navigation) in [
            (Scancode::Left, Navigation::Left),
            (Scancode::Right, Navigation::Right),
        ] {
            assert_eq!(
                key(scancode, &LEFT_CONTROL),
                Some(GuiEvent::KeyNavigation(navigation, WORD))
            );
            assert_eq!(
                key(scancode, &SHIFT_LEFT_CONTROL),
                Some(GuiEvent::KeyNavigation(
                    navigation,
                    EditModifiers {
                        extend: true,
                        word: true,
                        edge: false,
                    }
                ))
            );
            for keymod in [RIGHT_CONTROL, LEFT_ALT, LEFT_GUI] {
                assert_eq!(
                    key(scancode, &keymod),
                    Some(GuiEvent::KeyNavigation(navigation, NO_EDIT_MODIFIERS))
                );
            }
        }
        assert_eq!(
            key(Scancode::Home, &LEFT_CONTROL),
            Some(GuiEvent::KeyNavigation(Navigation::Home, NO_EDIT_MODIFIERS))
        );
        assert_eq!(
            key(Scancode::Up, &SHIFT),
            Some(GuiEvent::KeyNavigation(Navigation::Up, NO_EDIT_MODIFIERS))
        );
        assert_eq!(
            key(Scancode::A, &LEFT_CONTROL),
            Some(GuiEvent::Edit(EditCommand::SelectAll))
        );
        assert_eq!(key(Scancode::A, &RIGHT_CONTROL), None);
        assert_eq!(key(Scancode::A, &LEFT_GUI), None);
        assert_eq!(key(Scancode::A, &NO_MODIFIERS), None);
    }

    /// Verifies that Option turns left and right into word steps, that Cmd turns them into steps
    /// to the start and the end and forms select all with A, and that Shift extends each of them
    /// on macOS.
    #[cfg(target_os = "macos")]
    #[test]
    fn edit_keys_map_to_steps_and_commands() {
        let key =
            |scancode, keymod: &Mod| key_event(scancode, keymod, false, true, unread_clipboard);
        let edge = EditModifiers {
            edge: true,
            ..NO_EDIT_MODIFIERS
        };
        for (scancode, navigation) in [
            (Scancode::Left, Navigation::Left),
            (Scancode::Right, Navigation::Right),
        ] {
            assert_eq!(
                key(scancode, &SHIFT),
                Some(GuiEvent::KeyNavigation(navigation, EXTEND))
            );
            assert_eq!(
                key(scancode, &LEFT_ALT),
                Some(GuiEvent::KeyNavigation(navigation, WORD))
            );
            assert_eq!(
                key(scancode, &LEFT_GUI),
                Some(GuiEvent::KeyNavigation(navigation, edge))
            );
            assert_eq!(
                key(scancode, &SHIFT_LEFT_GUI),
                Some(GuiEvent::KeyNavigation(
                    navigation,
                    EditModifiers {
                        extend: true,
                        ..edge
                    }
                ))
            );
            assert_eq!(
                key(scancode, &LEFT_CONTROL),
                Some(GuiEvent::KeyNavigation(navigation, NO_EDIT_MODIFIERS))
            );
        }
        assert_eq!(
            key(Scancode::A, &LEFT_GUI),
            Some(GuiEvent::Edit(EditCommand::SelectAll))
        );
        assert_eq!(key(Scancode::A, &LEFT_CONTROL), None);
        assert_eq!(key(Scancode::A, &SHIFT_LEFT_CONTROL), None);
    }

    /// Verifies that the command key with C and X forms copy and cut, that it forms a paste with
    /// the text of the clipboard read on V while a text widget wants text input, and that the
    /// paste key without text input, the right control key and the command key of the other
    /// platform read no clipboard.
    #[test]
    fn clipboard_keys_map_to_copy_cut_and_paste() {
        assert_eq!(
            key_event(Scancode::C, &COMMAND, false, true, unread_clipboard),
            Some(GuiEvent::Edit(EditCommand::Copy))
        );
        assert_eq!(
            key_event(Scancode::X, &COMMAND, false, true, unread_clipboard),
            Some(GuiEvent::Edit(EditCommand::Cut))
        );
        assert_eq!(
            key_event(
                Scancode::V,
                &COMMAND,
                false,
                true,
                clipboard_with("ディスク")
            ),
            Some(GuiEvent::Paste("ディスク".into()))
        );
        assert_eq!(
            key_event(Scancode::V, &COMMAND, false, true, || Ok(None)),
            None
        );
        assert_eq!(
            key_event(Scancode::V, &COMMAND, false, false, unread_clipboard),
            None
        );
        assert_eq!(
            key_event(Scancode::V, &COMMAND, true, false, unread_clipboard),
            None
        );
        for keymod in [NO_MODIFIERS, RIGHT_CONTROL, OTHER_COMMAND] {
            for scancode in [Scancode::C, Scancode::X, Scancode::V] {
                assert_eq!(
                    key_event(scancode, &keymod, false, true, unread_clipboard),
                    None
                );
            }
        }
    }

    /// Verifies that a clipboard text turns into a paste event and that an empty clipboard and
    /// a failed read give none.
    #[test]
    fn a_clipboard_read_turns_into_a_paste_event() {
        assert_eq!(
            paste_event(Ok(Some("C:\\games\ndisk".into()))),
            Some(GuiEvent::Paste("C:\\games\ndisk".into()))
        );
        assert_eq!(paste_event(Ok(Some(String::new()))), None);
        assert_eq!(paste_event(Ok(None)), None);
        assert_eq!(paste_event(Err(Error("no video".into()))), None);
    }
}
