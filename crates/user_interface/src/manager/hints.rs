//! The hints of the status line of the library: the action of Enter and A on
//! the focused widget, the edit, the filter, the system switch, the refresh
//! and the settings page.

use std::fmt::Write as _;

use gui::ui::{HintKey, StatusEnd, TextBuffer};
use i18n::Catalog;

use super::rows::GameRow;
use crate::{
    config::Diagnostic,
    texts::{self, ACTIVATE_KEYS, CANCEL_KEYS, HintText, HintTexts, Hints, in_catalog, keys},
};

/// The hint of the launch of the selected game.
const LAUNCH_HINT: HintText = HintText::new(ACTIVATE_KEYS, texts::common::LAUNCH);

/// The hint of the delete of the selected file.
const DELETE_HINT: HintText = HintText::new(ACTIVATE_KEYS, texts::common::DELETE);

/// The hint of the shortcut that opens the selected game in the editor.
const EDIT_HINT: HintText = HintText::new(
    &[HintKey::Key(keys::F2), HintKey::Pad("X")],
    texts::common::EDIT,
);

/// The hint of the add button.
const ADD_BUTTON_HINT: HintText = HintText::new(ACTIVATE_KEYS, texts::common::ADD);

/// The hint of the edit button.
const EDIT_BUTTON_HINT: HintText = HintText::new(ACTIVATE_KEYS, texts::common::EDIT);

/// The hint of the duplicate button.
const DUPLICATE_BUTTON_HINT: HintText = HintText::new(ACTIVATE_KEYS, texts::common::DUPLICATE);

/// The hint of the system defaults button.
const SYSTEM_DEFAULTS_BUTTON_HINT: HintText =
    HintText::new(ACTIVATE_KEYS, texts::common::SYSTEM_DEFAULTS);

/// The hint of the system list, which gives the focus to the game table.
const SELECT_HINT: HintText = HintText::new(ACTIVATE_KEYS, texts::common::SELECT);

/// The hint of the column header, which sorts by the picked column.
const SORT_HINT: HintText = HintText::new(ACTIVATE_KEYS, texts::hints::SORT);

/// The hint of a filter with text, which cancel empties.
const CLEAR_HINT: HintText = HintText::new(CANCEL_KEYS, texts::common::CLEAR);

/// The hint of the shortcut that focuses the filter.
const FILTER_HINT: HintText = HintText::new(
    &[HintKey::Key(keys::CTRL_F), HintKey::Pad("Y")],
    texts::hints::FILTER,
);

/// The hint of the refresh button, which activates it.
const REFRESH_BUTTON_HINT: HintText = HintText::new(ACTIVATE_KEYS, texts::common::REFRESH);

/// The hint of the shortcut that refreshes the library.
const REFRESH_HINT: HintText = HintText::new(&[HintKey::Key(keys::F5)], texts::common::REFRESH);

/// The hint of the settings button, which activates it.
const SETTINGS_BUTTON_HINT: HintText = HintText::new(ACTIVATE_KEYS, texts::common::SETTINGS);

/// The hint of the shortcut that opens the settings page.
const SETTINGS_HINT: HintText = HintText::new(
    &[HintKey::Key(keys::F10), HintKey::Menu("Start")],
    texts::common::SETTINGS,
);

/// The hint of the system switch.
const SYSTEM_HINT: HintText = HintText::new(
    &[
        HintKey::Key(keys::CTRL_PAGE_UP),
        HintKey::Key(keys::CTRL_PAGE_DOWN),
        HintKey::Shoulder("LB"),
        HintKey::Shoulder("RB"),
    ],
    texts::hints::SYSTEM,
);

/// The widget of the library that holds the focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LibraryFocus {
    /// The system list.
    SystemList,
    /// The system defaults button.
    SystemDefaults,
    /// The filter field, with or without text.
    Filter {
        /// Whether the field holds text.
        has_text: bool,
    },
    /// The button that opens the settings page.
    Settings,
    /// The launch button.
    Launch,
    /// The add button.
    Add,
    /// The edit button.
    Edit,
    /// The duplicate button.
    Duplicate,
    /// The delete button.
    Delete,
    /// The refresh button.
    Refresh,
    /// The column header of the game table.
    Header,
    /// The rows of the game table.
    Table,
    /// No widget of the library.
    Nothing,
}

/// The hints of the library while `focus` holds the focus and `selected`
/// is the selected row of the game table. The edit shortcut follows a
/// widget that shows a game that can start. The refresh and the settings
/// shortcuts follow the system switch while no text field holds the focus.
/// The settings button drops the settings shortcut, which has the same
/// result.
pub(crate) fn library_hints<'c>(
    catalog: &'c Catalog,
    focus: LibraryFocus,
    selected: Option<&GameRow>,
) -> Hints<'c> {
    let mut hints = focus_hints(focus, selected);
    match focus {
        LibraryFocus::Filter { .. } | LibraryFocus::Settings => {}
        LibraryFocus::SystemList
        | LibraryFocus::SystemDefaults
        | LibraryFocus::Launch
        | LibraryFocus::Add
        | LibraryFocus::Edit
        | LibraryFocus::Duplicate
        | LibraryFocus::Delete
        | LibraryFocus::Refresh
        | LibraryFocus::Header
        | LibraryFocus::Table
        | LibraryFocus::Nothing => hints.push(SETTINGS_HINT),
    }
    texts::hints_in(catalog, &hints)
}

/// The hints of the library before the settings shortcut while `focus`
/// holds the focus and `selected` is the selected row of the game table.
fn focus_hints(focus: LibraryFocus, selected: Option<&GameRow>) -> HintTexts {
    let editable = selected.is_some_and(GameRow::launchable);
    match focus {
        LibraryFocus::SystemList if editable => HintTexts::from([
            SELECT_HINT,
            EDIT_HINT,
            FILTER_HINT,
            SYSTEM_HINT,
            REFRESH_HINT,
        ]),
        LibraryFocus::SystemList if selected.is_some() => {
            HintTexts::from([SELECT_HINT, FILTER_HINT, SYSTEM_HINT, REFRESH_HINT])
        }
        LibraryFocus::SystemDefaults => HintTexts::from([
            SYSTEM_DEFAULTS_BUTTON_HINT,
            FILTER_HINT,
            SYSTEM_HINT,
            REFRESH_HINT,
        ]),
        LibraryFocus::Filter { has_text: true } => HintTexts::from([CLEAR_HINT, SYSTEM_HINT]),
        LibraryFocus::Filter { has_text: false } => HintTexts::from([SYSTEM_HINT]),
        LibraryFocus::Settings => {
            HintTexts::from([SETTINGS_BUTTON_HINT, FILTER_HINT, SYSTEM_HINT, REFRESH_HINT])
        }
        LibraryFocus::Launch => HintTexts::from([
            LAUNCH_HINT,
            EDIT_HINT,
            FILTER_HINT,
            SYSTEM_HINT,
            REFRESH_HINT,
        ]),
        LibraryFocus::Add => {
            HintTexts::from([ADD_BUTTON_HINT, FILTER_HINT, SYSTEM_HINT, REFRESH_HINT])
        }
        LibraryFocus::Edit => {
            HintTexts::from([EDIT_BUTTON_HINT, FILTER_HINT, SYSTEM_HINT, REFRESH_HINT])
        }
        LibraryFocus::Duplicate => HintTexts::from([
            DUPLICATE_BUTTON_HINT,
            FILTER_HINT,
            SYSTEM_HINT,
            REFRESH_HINT,
        ]),
        LibraryFocus::Delete => {
            HintTexts::from([DELETE_HINT, FILTER_HINT, SYSTEM_HINT, REFRESH_HINT])
        }
        LibraryFocus::Refresh => HintTexts::from([REFRESH_BUTTON_HINT, FILTER_HINT, SYSTEM_HINT]),
        LibraryFocus::Header => {
            HintTexts::from([SORT_HINT, FILTER_HINT, SYSTEM_HINT, REFRESH_HINT])
        }
        LibraryFocus::Table => match selected {
            Some(row) if row.launchable() => HintTexts::from([
                LAUNCH_HINT,
                EDIT_HINT,
                FILTER_HINT,
                SYSTEM_HINT,
                REFRESH_HINT,
            ]),
            Some(_) => HintTexts::from([DELETE_HINT, SYSTEM_HINT, REFRESH_HINT]),
            None => HintTexts::from([FILTER_HINT, SYSTEM_HINT, REFRESH_HINT]),
        },
        LibraryFocus::SystemList | LibraryFocus::Nothing => {
            HintTexts::from([FILTER_HINT, SYSTEM_HINT, REFRESH_HINT])
        }
    }
}

/// The end of the status line: the path of the selected game, the error of
/// the selected file with an error after its shown path, or else the folder
/// `folder` of the system. `located` holds the text of the error in
/// `catalog` at the catalog generation `generation`.
pub(crate) fn status_end<'a>(
    catalog: &Catalog,
    generation: u64,
    selected: Option<&'a GameRow>,
    folder: &'a str,
    located: &'a mut TextBuffer<(String, Diagnostic)>,
) -> StatusEnd<'a> {
    match selected {
        Some(row) => match &row.error {
            Some(error) => StatusEnd::Error(located.text_in(
                catalog,
                generation,
                (row.shown_path.as_str(), error),
                |catalog, (path, error), out| {
                    let _ = write!(out, "{}", in_catalog(catalog, &error.located_at(path)));
                },
            )),
            None => StatusEnd::Path(&row.shown_path),
        },
        None => StatusEnd::Path(folder),
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::Hint;

    use super::*;
    use crate::{
        gui_test_support::assert_hints_fit,
        manager::rows::tests::{broken, game},
        texts::english,
    };

    /// Verifies that the six hints of a game that starts show every key cap at the minimum window size in every language.
    #[test]
    fn the_hints_of_a_game_that_starts_fit_at_the_minimum_window_size() {
        let edge = game("edge", "Edge", "", "", "");
        assert_hints_fit(|catalog| library_hints(catalog, LibraryFocus::Table, Some(&edge)));
    }

    /// The labels of `hints`.
    fn labels<'a>(hints: &[Hint<'a>]) -> Vec<&'a str> {
        hints.iter().map(|hint| hint.label).collect()
    }

    /// Verifies the hints of each focused widget of the library.
    #[test]
    fn the_first_hint_follows_the_focused_widget() {
        let edge = game("edge", "Edge", "", "", "");
        let bad = broken("bad");
        let cases = [
            (
                LibraryFocus::Table,
                Some(&edge),
                vec!["Launch", "Edit", "Filter", "System", "Refresh", "Settings"],
            ),
            (
                LibraryFocus::Table,
                Some(&bad),
                vec!["Delete", "System", "Refresh", "Settings"],
            ),
            (
                LibraryFocus::Table,
                None,
                vec!["Filter", "System", "Refresh", "Settings"],
            ),
            (
                LibraryFocus::SystemList,
                Some(&edge),
                vec!["Select", "Edit", "Filter", "System", "Refresh", "Settings"],
            ),
            (
                LibraryFocus::SystemList,
                Some(&bad),
                vec!["Select", "Filter", "System", "Refresh", "Settings"],
            ),
            (
                LibraryFocus::SystemList,
                None,
                vec!["Filter", "System", "Refresh", "Settings"],
            ),
            (
                LibraryFocus::SystemDefaults,
                Some(&edge),
                vec!["System defaults", "Filter", "System", "Refresh", "Settings"],
            ),
            (
                LibraryFocus::Settings,
                Some(&edge),
                vec!["Settings", "Filter", "System", "Refresh"],
            ),
            (
                LibraryFocus::Filter { has_text: true },
                None,
                vec!["Clear", "System"],
            ),
            (
                LibraryFocus::Filter { has_text: false },
                Some(&edge),
                vec!["System"],
            ),
            (
                LibraryFocus::Launch,
                Some(&edge),
                vec!["Launch", "Edit", "Filter", "System", "Refresh", "Settings"],
            ),
            (
                LibraryFocus::Add,
                None,
                vec!["Add", "Filter", "System", "Refresh", "Settings"],
            ),
            (
                LibraryFocus::Edit,
                Some(&edge),
                vec!["Edit", "Filter", "System", "Refresh", "Settings"],
            ),
            (
                LibraryFocus::Duplicate,
                Some(&edge),
                vec!["Duplicate", "Filter", "System", "Refresh", "Settings"],
            ),
            (
                LibraryFocus::Delete,
                Some(&edge),
                vec!["Delete", "Filter", "System", "Refresh", "Settings"],
            ),
            (
                LibraryFocus::Refresh,
                Some(&edge),
                vec!["Refresh", "Filter", "System", "Settings"],
            ),
            (
                LibraryFocus::Header,
                Some(&edge),
                vec!["Sort", "Filter", "System", "Refresh", "Settings"],
            ),
            (
                LibraryFocus::Nothing,
                None,
                vec!["Filter", "System", "Refresh", "Settings"],
            ),
        ];
        for (focus, selected, expected) in cases {
            assert_eq!(
                labels(&library_hints(english(), focus, selected)),
                expected,
                "{focus:?}"
            );
        }
    }

    /// Verifies the end of the status line for a game, a file with an error and no selection.
    #[test]
    fn the_end_shows_the_path_the_error_or_the_folder() {
        let edge = game("edge", "Edge", "", "", "");
        let mut located = TextBuffer::new();
        let mut end =
            |selected| match status_end(english(), 0, selected, "config/pc98/", &mut located) {
                StatusEnd::None => "none".to_owned(),
                StatusEnd::Path(path) => format!("path {path}"),
                StatusEnd::Error(error) => format!("error {error}"),
            };
        assert_eq!(end(Some(&edge)), format!("path {}", edge.shown_path));
        let bad = broken("bad");
        assert_eq!(
            end(Some(&bad)),
            format!("error {} line 1: unknown machine X68030", bad.shown_path)
        );
        assert_eq!(end(None), "path config/pc98/");
    }
}
