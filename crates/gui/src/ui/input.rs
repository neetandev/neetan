//! The input of a pass and the target it draws into.

use super::focus::Navigation;

/// A gamepad button that triggers an action of the caller, such as a
/// shortcut of a screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionButton {
    /// The left face button.
    X,
    /// The top face button.
    Y,
    /// The start button.
    Start,
}

/// The device of the navigation event of a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NavigationSource {
    /// A key of the keyboard.
    #[default]
    Keyboard,
    /// The D-pad, the left stick or a shoulder button of a gamepad.
    Gamepad,
}

/// The modifier keys held with the navigation key of a pass. They change
/// the step of the text cursor in a focused text widget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct EditModifiers {
    /// The step extends the selection.
    pub extend: bool,
    /// Left and right step by one word.
    pub word: bool,
    /// Left and right step to the start and the end of the text.
    pub edge: bool,
}

/// An edit of the text of the focused text widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditCommand {
    /// Selects the whole text.
    SelectAll,
    /// Removes the selection or else the character after the cursor. It is
    /// the delete key, and every widget other than a text widget reads it as
    /// [`Shortcut::Delete`].
    DeleteForward,
}

/// A keyboard shortcut of a screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shortcut {
    /// The delete key. It arrives as [`EditCommand::DeleteForward`] in
    /// [`Input::edit`].
    Delete,
    /// The insert key.
    Insert,
    /// The F2 key.
    F2,
    /// The F5 key.
    F5,
    /// The F10 key.
    F10,
    /// The backspace key.
    Backspace,
    /// The F key with the left control key.
    ControlF,
    /// The D key with the left control key.
    ControlD,
    /// The S key with the left control key.
    ControlS,
}

impl Shortcut {
    /// Whether the shortcut holds the control key. A shortcut without it is
    /// inactive while a text widget holds the focus.
    pub fn has_modifier(self) -> bool {
        match self {
            Self::Delete | Self::Insert | Self::F2 | Self::F5 | Self::F10 | Self::Backspace => {
                false
            }
            Self::ControlF | Self::ControlD | Self::ControlS => true,
        }
    }
}

/// The input of one pass of the user interface.
///
/// The caller runs one pass for each discrete event: a press or a release of
/// a mouse button, a navigation event, an action button, a shortcut, a text
/// commit, a backspace, an edit command, confirm or cancel. A pass holds at
/// most one of them. Pointer motion and wheel steps add up until the next
/// pass. The caller shows the scene of the last pass and calls
/// [`Input::advance`] after each pass.
///
/// The pointer position is in physical pixels: the window coordinates of the
/// platform times the pixel density of the window, as
/// `SDL_GetWindowPixelDensity` reports it. The context divides it by the
/// scale factor of the viewport.
#[derive(Clone, Debug, Default)]
pub struct Input {
    /// The pointer x in physical pixels.
    pub mouse_x: f32,
    /// The pointer y in physical pixels.
    pub mouse_y: f32,
    /// The left button is held in this pass.
    pub left: bool,
    /// The right button is held in this pass.
    pub right: bool,
    /// The middle button is held in this pass.
    pub middle: bool,
    /// The left button was held in the last pass.
    pub left_previous: bool,
    /// The right button was held in the last pass.
    pub right_previous: bool,
    /// The middle button was held in the last pass.
    pub middle_previous: bool,
    /// The vertical wheel steps since the last pass. Positive values scroll
    /// up.
    pub wheel: f32,
    /// The press of the left button in this pass is the second press of a
    /// double click.
    pub left_double: bool,
    /// The text that the platform committed in this pass.
    pub text: String,
    /// The text that an input method composes and has not committed. The
    /// caller replaces it on every change of the composition and empties it
    /// with the commit. It stays from pass to pass.
    pub composition: String,
    /// The backspace key was pressed.
    pub backspace: bool,
    /// A confirm key or the confirm button of a gamepad was pressed.
    pub confirm: bool,
    /// The escape key or the cancel button of a gamepad was pressed.
    pub cancel: bool,
    /// The navigation event of this pass.
    pub navigation: Option<Navigation>,
    /// The device of the navigation event of this pass.
    pub navigation_source: NavigationSource,
    /// The modifier keys held with the navigation key of this pass.
    pub modifiers: EditModifiers,
    /// The edit command of this pass.
    pub edit: Option<EditCommand>,
    /// The action button of a gamepad pressed in this pass.
    pub action: Option<ActionButton>,
    /// The shortcut key pressed in this pass. Backspace arrives in
    /// [`Input::backspace`] and delete in [`Input::edit`].
    pub shortcut: Option<Shortcut>,
}

impl Input {
    /// Whether the left button went down in this pass.
    pub fn left_pressed(&self) -> bool {
        self.left && !self.left_previous
    }

    /// Whether the left button went up in this pass.
    pub fn left_released(&self) -> bool {
        !self.left && self.left_previous
    }

    /// Whether the right button went down in this pass.
    pub fn right_pressed(&self) -> bool {
        self.right && !self.right_previous
    }

    /// Whether the pass holds a key or gamepad event: a navigation event,
    /// an action button, a shortcut, an edit command, confirm, cancel, a
    /// text commit or a backspace.
    pub fn has_key_event(&self) -> bool {
        self.navigation.is_some()
            || self.action.is_some()
            || self.shortcut.is_some()
            || self.edit.is_some()
            || self.confirm
            || self.cancel
            || !self.text.is_empty()
            || self.backspace
    }

    /// Carries the button state of this pass into the fields of the last
    /// pass and clears the events of this pass. The composition stays.
    pub fn advance(&mut self) {
        self.left_previous = self.left;
        self.right_previous = self.right;
        self.middle_previous = self.middle;
        self.wheel = 0.0;
        self.left_double = false;
        self.text.clear();
        self.backspace = false;
        self.confirm = false;
        self.cancel = false;
        self.navigation = None;
        self.navigation_source = NavigationSource::Keyboard;
        self.modifiers = EditModifiers::default();
        self.edit = None;
        self.action = None;
        self.shortcut = None;
    }
}

/// The target of a pass: its size in physical pixels and the physical
/// pixels for each logical unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// The width in physical pixels.
    pub width: u32,
    /// The height in physical pixels.
    pub height: u32,
    /// The physical pixels for each logical unit.
    pub scale: f32,
}

impl Viewport {
    /// A target of `width` x `height` physical pixels at `scale`. A scale
    /// that is not a positive finite number counts as 1.
    pub fn new(width: u32, height: u32, scale: f32) -> Self {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        Self {
            width,
            height,
            scale,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_clears_the_events_of_a_pass() {
        let mut input = Input {
            left: true,
            wheel: 1.0,
            text: "a".into(),
            composition: "か".into(),
            backspace: true,
            confirm: true,
            navigation: Some(Navigation::Down),
            navigation_source: NavigationSource::Gamepad,
            modifiers: EditModifiers {
                extend: true,
                word: true,
                edge: true,
            },
            edit: Some(EditCommand::SelectAll),
            action: Some(ActionButton::Start),
            shortcut: Some(Shortcut::F2),
            ..Input::default()
        };
        assert!(input.left_pressed());
        assert!(input.has_key_event());
        input.advance();
        assert!(input.left_previous);
        assert!(!input.left_pressed());
        assert!(!input.has_key_event());
        assert_eq!(input.wheel, 0.0);
        assert_eq!(input.action, None);
        assert_eq!(input.shortcut, None);
        assert_eq!(input.edit, None);
        assert_eq!(input.navigation_source, NavigationSource::Keyboard);
        assert_eq!(input.modifiers, EditModifiers::default());
        assert_eq!(input.composition, "か");
        input.left = false;
        assert!(input.left_released());
    }

    #[test]
    fn an_action_button_and_an_edit_command_are_key_events() {
        let input = Input {
            action: Some(ActionButton::Y),
            ..Input::default()
        };
        assert!(input.has_key_event());
        let input = Input {
            edit: Some(EditCommand::DeleteForward),
            ..Input::default()
        };
        assert!(input.has_key_event());
    }

    #[test]
    fn a_broken_scale_counts_as_one() {
        assert_eq!(Viewport::new(1, 1, f32::NAN).scale, 1.0);
        assert_eq!(Viewport::new(1, 1, -2.0).scale, 1.0);
        assert_eq!(Viewport::new(1, 1, 1.5).scale, 1.5);
    }
}
