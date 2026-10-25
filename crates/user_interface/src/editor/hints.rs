//! The hints of the status line of the editor: the action of Enter and A on
//! the focused widget, the section switch, the save and the close.

use gui::ui::{ACTIVATE_KEYS, CANCEL_KEYS, HintKey, UiContext, UiId, distinct_hints};

use crate::texts::{self, HintText};

/// The kind of a focusable widget of the editor, which picks the first hint.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum WidgetKind {
    /// A text field.
    Text,
    /// A number field.
    Number,
    /// A dropdown.
    Dropdown,
    /// A media list.
    MediaList,
    /// A Browse button.
    Browse,
    /// The button that adds an image to a media list.
    AddImage,
    /// The button that removes the selected image of a media list.
    RemoveImage,
    /// The button that shifts the selected image up.
    ShiftUp,
    /// The button that shifts the selected image down.
    ShiftDown,
    /// The tab bar of the sections.
    Tabs,
    /// The cancel button of the footer.
    Cancel,
    /// The save button of the footer.
    Save,
    /// The close button of the header.
    Close,
    /// The checkbox of an empty media list.
    Checkbox,
}

/// A focusable widget of the editor in a pass.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) struct EditorWidget {
    /// The id of the widget.
    pub(crate) id: UiId,
    /// The kind, which picks the first hint.
    pub(crate) kind: WidgetKind,
    /// Whether gamepad X has an effect on the widget: it clears a set
    /// field or removes the selected image of a media list.
    pub(crate) clears: bool,
}

impl EditorWidget {
    /// The widget `id` of `kind` without an effect of gamepad X.
    pub(crate) fn new(id: UiId, kind: WidgetKind) -> Self {
        Self {
            id,
            kind,
            clears: false,
        }
    }

    /// The same widget, with an effect of gamepad X when `clears`.
    pub(crate) fn clears(self, clears: bool) -> Self {
        Self { clears, ..self }
    }
}

/// The result of a hint. Two hints of one action lead to one result, and
/// the status line shows only the first of them.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum EditorAction {
    /// The dropdown opens its popup.
    Select,
    /// The file browser opens.
    Browse,
    /// An image joins the media list.
    Add,
    /// The selected image leaves the media list.
    Remove,
    /// The selected image shifts up.
    ShiftUp,
    /// The selected image shifts down.
    ShiftDown,
    /// The editor closes, through the question with unsaved changes.
    Close,
    /// The file saves.
    Save,
    /// The checkbox flips.
    Switch,
    /// The field returns to unset.
    Clear,
    /// The number steps.
    Step,
    /// Digits type into the number.
    Type,
    /// The section switches.
    Section,
    /// The popup entry takes the value.
    Choose,
    /// The popup closes.
    ClosePopup,
}

/// A hint with its action.
type ActionHint = (EditorAction, HintText);

/// The hint of a dropdown.
const SELECT_HINT: ActionHint = (
    EditorAction::Select,
    HintText::new(ACTIVATE_KEYS, texts::common::SELECT),
);

/// The hint of a media list and a Browse button.
const BROWSE_HINT: ActionHint = (
    EditorAction::Browse,
    HintText::new(ACTIVATE_KEYS, texts::common::BROWSE),
);

/// The hint of the button that adds an image.
const ADD_HINT: ActionHint = (
    EditorAction::Add,
    HintText::new(ACTIVATE_KEYS, texts::common::ADD),
);

/// The hint of the button that removes an image.
const REMOVE_HINT: ActionHint = (
    EditorAction::Remove,
    HintText::new(ACTIVATE_KEYS, texts::common::REMOVE),
);

/// The hint of the button that shifts an image up.
const UP_HINT: ActionHint = (
    EditorAction::ShiftUp,
    HintText::new(ACTIVATE_KEYS, texts::hints::UP),
);

/// The hint of the button that shifts an image down.
const DOWN_HINT: ActionHint = (
    EditorAction::ShiftDown,
    HintText::new(ACTIVATE_KEYS, texts::hints::DOWN),
);

/// The hint of the cancel button.
const CANCEL_HINT: ActionHint = (
    EditorAction::Close,
    HintText::new(ACTIVATE_KEYS, texts::common::CANCEL),
);

/// The hint of the save button.
const SAVE_BUTTON_HINT: ActionHint = (
    EditorAction::Save,
    HintText::new(ACTIVATE_KEYS, texts::common::SAVE),
);

/// The hint of the close button.
const CLOSE_BUTTON_HINT: ActionHint = (
    EditorAction::Close,
    HintText::new(ACTIVATE_KEYS, texts::common::CLOSE),
);

/// The hint of the checkbox of an empty media list.
const SWITCH_HINT: ActionHint = (
    EditorAction::Switch,
    HintText::new(ACTIVATE_KEYS, texts::common::SWITCH),
);

/// The hint of gamepad X on a set field.
const CLEAR_HINT: ActionHint = (
    EditorAction::Clear,
    HintText::new(&[HintKey::Pad("X")], texts::common::CLEAR),
);

/// The hint of gamepad X on a media list with images.
const REMOVE_IMAGE_HINT: ActionHint = (
    EditorAction::Remove,
    HintText::new(&[HintKey::Pad("X")], texts::common::REMOVE),
);

/// The hint of the steps of a number field.
const STEP_HINT: ActionHint = (
    EditorAction::Step,
    HintText::new(
        &[HintKey::Key("Left"), HintKey::Key("Right")],
        texts::hints::STEP,
    ),
);

/// The hint of the digits of a number field.
const TYPE_HINT: ActionHint = (
    EditorAction::Type,
    HintText::new(&[HintKey::Key("0-9")], texts::hints::TYPE),
);

/// The hint of the section switch.
const SECTION_HINT: ActionHint = (
    EditorAction::Section,
    HintText::new(
        &[
            HintKey::Key("Ctrl+PgUp"),
            HintKey::Key("Ctrl+PgDn"),
            HintKey::Shoulder("LB"),
            HintKey::Shoulder("RB"),
        ],
        texts::hints::SECTION,
    ),
);

/// The hint of the save shortcut.
const SAVE_HINT: ActionHint = (
    EditorAction::Save,
    HintText::new(
        &[HintKey::Key("Ctrl+S"), HintKey::Key("Start")],
        texts::common::SAVE,
    ),
);

/// The hint of the close of the editor.
const CLOSE_HINT: ActionHint = (
    EditorAction::Close,
    HintText::new(CANCEL_KEYS, texts::common::CLOSE),
);

/// The hint of an entry of an open dropdown popup.
const CHOOSE_HINT: ActionHint = (
    EditorAction::Choose,
    HintText::new(ACTIVATE_KEYS, texts::common::CHOOSE),
);

/// The hint of the close of an open dropdown popup.
const CLOSE_POPUP_HINT: ActionHint = (
    EditorAction::ClosePopup,
    HintText::new(CANCEL_KEYS, texts::hints::CLOSE_POPUP),
);

/// The hints of the editor while the widget that `context` reports as
/// focused holds the focus. `widgets` are the focusable widgets of the pass.
/// An open dropdown popup shows only its own hints. A hint whose action an
/// earlier hint has leaves the line, such as Esc beside the focused close
/// button.
pub(crate) fn editor_hints(context: &UiContext, widgets: &[EditorWidget]) -> Vec<HintText> {
    if widgets
        .iter()
        .any(|widget| widget.kind == WidgetKind::Dropdown && context.popup_open(widget.id))
    {
        return distinct_hints([CHOOSE_HINT, CLOSE_POPUP_HINT]);
    }
    let focused = context
        .focused()
        .and_then(|focus| widgets.iter().find(|widget| widget.id == focus));
    let mut hints = match focused.map(|widget| widget.kind) {
        Some(WidgetKind::Dropdown) => vec![SELECT_HINT],
        Some(WidgetKind::Number) => vec![STEP_HINT, TYPE_HINT],
        Some(WidgetKind::MediaList | WidgetKind::Browse) => vec![BROWSE_HINT],
        Some(WidgetKind::AddImage) => vec![ADD_HINT],
        Some(WidgetKind::RemoveImage) => vec![REMOVE_HINT],
        Some(WidgetKind::ShiftUp) => vec![UP_HINT],
        Some(WidgetKind::ShiftDown) => vec![DOWN_HINT],
        Some(WidgetKind::Cancel) => vec![CANCEL_HINT],
        Some(WidgetKind::Save) => vec![SAVE_BUTTON_HINT],
        Some(WidgetKind::Close) => vec![CLOSE_BUTTON_HINT],
        Some(WidgetKind::Checkbox) => vec![SWITCH_HINT],
        Some(WidgetKind::Text | WidgetKind::Tabs) | None => Vec::new(),
    };
    if let Some(widget) = focused.filter(|widget| widget.clears) {
        hints.push(match widget.kind {
            WidgetKind::MediaList => REMOVE_IMAGE_HINT,
            WidgetKind::Text
            | WidgetKind::Number
            | WidgetKind::Dropdown
            | WidgetKind::Browse
            | WidgetKind::AddImage
            | WidgetKind::RemoveImage
            | WidgetKind::ShiftUp
            | WidgetKind::ShiftDown
            | WidgetKind::Tabs
            | WidgetKind::Cancel
            | WidgetKind::Save
            | WidgetKind::Close
            | WidgetKind::Checkbox => CLEAR_HINT,
        });
    }
    hints.extend([SECTION_HINT, SAVE_HINT, CLOSE_HINT]);
    distinct_hints(hints)
}

#[cfg(test)]
mod tests {
    use gui::{
        Rect,
        ui::{NavigationSet, UiState},
    };

    use super::*;
    use crate::gui_test_support::GuiHarness;

    /// The labels of the hints while the widget of `kind` holds the focus.
    fn labels(kind: WidgetKind) -> Vec<&'static str> {
        labels_of(EditorWidget::new(UiId::from("widget"), kind))
    }

    /// The labels of the hints while `widget` holds the focus.
    fn labels_of(widget: EditorWidget) -> Vec<&'static str> {
        let mut harness = GuiHarness::new(960, 600);
        let id = widget.id;
        harness.state = UiState::default();
        harness.state.set_focus(id);
        harness.frame(|context| {
            context.focusable(id, Rect::new(0.0, 0.0, 10.0, 10.0), NavigationSet::EMPTY);
            crate::texts::english_labels(&editor_hints(context, &[widget]))
        })
    }

    /// Verifies the first hint of each widget and the hints that follow every widget, without a second hint of one action: Esc beside the focused Cancel or close button, and Ctrl+S beside the focused Save button.
    #[test]
    fn the_first_hint_follows_the_focused_widget() {
        assert_eq!(
            labels(WidgetKind::Dropdown),
            ["Select", "Section", "Save", "Close"]
        );
        assert_eq!(
            labels(WidgetKind::Number),
            ["Step", "Type", "Section", "Save", "Close"]
        );
        assert_eq!(
            labels(WidgetKind::MediaList),
            ["Browse", "Section", "Save", "Close"]
        );
        assert_eq!(labels(WidgetKind::Text), ["Section", "Save", "Close"]);
        assert_eq!(labels(WidgetKind::ShiftDown)[0], "Down");
        assert_eq!(labels(WidgetKind::Save), ["Save", "Section", "Close"]);
        assert_eq!(labels(WidgetKind::Close), ["Close", "Section", "Save"]);
        assert_eq!(labels(WidgetKind::Cancel), ["Cancel", "Section", "Save"]);
        assert_eq!(
            labels(WidgetKind::Checkbox),
            ["Switch", "Section", "Save", "Close"]
        );
    }

    /// Verifies the hint of gamepad X on a set field and on a media list with images.
    #[test]
    fn gamepad_x_clears_and_removes() {
        let id = UiId::from("widget");
        assert_eq!(
            labels_of(EditorWidget::new(id, WidgetKind::Text).clears(true)),
            ["Clear", "Section", "Save", "Close"]
        );
        assert_eq!(
            labels_of(EditorWidget::new(id, WidgetKind::Number).clears(true)),
            ["Step", "Type", "Clear", "Section", "Save", "Close"]
        );
        assert_eq!(
            labels_of(EditorWidget::new(id, WidgetKind::MediaList).clears(true)),
            ["Browse", "Remove", "Section", "Save", "Close"]
        );
    }
}
