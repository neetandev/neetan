//! The hints of the status line of the library: the action of Enter and A on
//! the focused widget, the edit, the filter, the system switch, the refresh
//! and the settings page.

use gui::ui::{ACTIVATE_KEYS, CANCEL_KEYS, Hint, HintKey, StatusEnd};

use super::rows::GameRow;

/// The hint of the launch of the selected game.
const LAUNCH_HINT: Hint<'static> = Hint {
    keys: ACTIVATE_KEYS,
    label: "Launch",
};

/// The hint of the delete of the selected file.
const DELETE_HINT: Hint<'static> = Hint {
    keys: ACTIVATE_KEYS,
    label: "Delete",
};

/// The hint of the shortcut that opens the selected game in the editor.
const EDIT_HINT: Hint<'static> = Hint {
    keys: &[HintKey::Key("F2"), HintKey::Pad("X")],
    label: "Edit",
};

/// The hint of the add button.
const ADD_BUTTON_HINT: Hint<'static> = Hint {
    keys: ACTIVATE_KEYS,
    label: "Add",
};

/// The hint of the edit button.
const EDIT_BUTTON_HINT: Hint<'static> = Hint {
    keys: ACTIVATE_KEYS,
    label: "Edit",
};

/// The hint of the duplicate button.
const DUPLICATE_BUTTON_HINT: Hint<'static> = Hint {
    keys: ACTIVATE_KEYS,
    label: "Duplicate",
};

/// The hint of the system defaults button.
const SYSTEM_DEFAULTS_BUTTON_HINT: Hint<'static> = Hint {
    keys: ACTIVATE_KEYS,
    label: "System defaults",
};

/// The hint of the system list, which gives the focus to the game table.
const SELECT_HINT: Hint<'static> = Hint {
    keys: ACTIVATE_KEYS,
    label: "Select",
};

/// The hint of the column header, which sorts by the picked column.
const SORT_HINT: Hint<'static> = Hint {
    keys: ACTIVATE_KEYS,
    label: "Sort",
};

/// The hint of a filter with text, which cancel empties.
const CLEAR_HINT: Hint<'static> = Hint {
    keys: CANCEL_KEYS,
    label: "Clear",
};

/// The hint of the shortcut that focuses the filter.
const FILTER_HINT: Hint<'static> = Hint {
    keys: &[HintKey::Key("Ctrl+F"), HintKey::Pad("Y")],
    label: "Filter",
};

/// The hint of the refresh button, which activates it.
const REFRESH_BUTTON_HINT: Hint<'static> = Hint {
    keys: ACTIVATE_KEYS,
    label: "Refresh",
};

/// The hint of the shortcut that refreshes the library.
const REFRESH_HINT: Hint<'static> = Hint {
    keys: &[HintKey::Key("F5")],
    label: "Refresh",
};

/// The hint of the settings button, which activates it.
const SETTINGS_BUTTON_HINT: Hint<'static> = Hint {
    keys: ACTIVATE_KEYS,
    label: "Settings",
};

/// The hint of the shortcut that opens the settings page.
const SETTINGS_HINT: Hint<'static> = Hint {
    keys: &[HintKey::Key("F10"), HintKey::Key("Start")],
    label: "Settings",
};

/// The hint of the system switch.
const SYSTEM_HINT: Hint<'static> = Hint {
    keys: &[
        HintKey::Key("Ctrl+PgUp"),
        HintKey::Key("Ctrl+PgDn"),
        HintKey::Shoulder("LB"),
        HintKey::Shoulder("RB"),
    ],
    label: "System",
};

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
pub(crate) fn library_hints(focus: LibraryFocus, selected: Option<&GameRow>) -> Vec<Hint<'static>> {
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
    hints
}

/// The hints of the library before the settings shortcut while `focus`
/// holds the focus and `selected` is the selected row of the game table.
fn focus_hints(focus: LibraryFocus, selected: Option<&GameRow>) -> Vec<Hint<'static>> {
    let editable = selected.is_some_and(GameRow::launchable);
    match focus {
        LibraryFocus::SystemList if editable => {
            vec![
                SELECT_HINT,
                EDIT_HINT,
                FILTER_HINT,
                SYSTEM_HINT,
                REFRESH_HINT,
            ]
        }
        LibraryFocus::SystemList if selected.is_some() => {
            vec![SELECT_HINT, FILTER_HINT, SYSTEM_HINT, REFRESH_HINT]
        }
        LibraryFocus::SystemDefaults => vec![
            SYSTEM_DEFAULTS_BUTTON_HINT,
            FILTER_HINT,
            SYSTEM_HINT,
            REFRESH_HINT,
        ],
        LibraryFocus::Filter { has_text: true } => vec![CLEAR_HINT, SYSTEM_HINT],
        LibraryFocus::Filter { has_text: false } => vec![SYSTEM_HINT],
        LibraryFocus::Settings => {
            vec![SETTINGS_BUTTON_HINT, FILTER_HINT, SYSTEM_HINT, REFRESH_HINT]
        }
        LibraryFocus::Launch => vec![
            LAUNCH_HINT,
            EDIT_HINT,
            FILTER_HINT,
            SYSTEM_HINT,
            REFRESH_HINT,
        ],
        LibraryFocus::Add => vec![ADD_BUTTON_HINT, FILTER_HINT, SYSTEM_HINT, REFRESH_HINT],
        LibraryFocus::Edit => vec![EDIT_BUTTON_HINT, FILTER_HINT, SYSTEM_HINT, REFRESH_HINT],
        LibraryFocus::Duplicate => vec![
            DUPLICATE_BUTTON_HINT,
            FILTER_HINT,
            SYSTEM_HINT,
            REFRESH_HINT,
        ],
        LibraryFocus::Delete => vec![DELETE_HINT, FILTER_HINT, SYSTEM_HINT, REFRESH_HINT],
        LibraryFocus::Refresh => vec![REFRESH_BUTTON_HINT, FILTER_HINT, SYSTEM_HINT],
        LibraryFocus::Header => vec![SORT_HINT, FILTER_HINT, SYSTEM_HINT, REFRESH_HINT],
        LibraryFocus::Table => match selected {
            Some(row) if row.launchable() => {
                vec![
                    LAUNCH_HINT,
                    EDIT_HINT,
                    FILTER_HINT,
                    SYSTEM_HINT,
                    REFRESH_HINT,
                ]
            }
            Some(_) => vec![DELETE_HINT, SYSTEM_HINT, REFRESH_HINT],
            None => vec![FILTER_HINT, SYSTEM_HINT, REFRESH_HINT],
        },
        LibraryFocus::SystemList | LibraryFocus::Nothing => {
            vec![FILTER_HINT, SYSTEM_HINT, REFRESH_HINT]
        }
    }
}

/// The end of the status line: the path of the selected game, the error of
/// the selected file with an error, or else the folder `folder` of the
/// system.
pub(crate) fn status_end<'a>(selected: Option<&'a GameRow>, folder: &'a str) -> StatusEnd<'a> {
    match selected {
        Some(row) => match &row.located_error {
            Some(error) => StatusEnd::Error(error),
            None => StatusEnd::Path(&row.shown_path),
        },
        None => StatusEnd::Path(folder),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::rows::tests::{broken, game};

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
                labels(&library_hints(focus, selected)),
                expected,
                "{focus:?}"
            );
        }
    }

    /// Verifies the end of the status line for a game, a file with an error and no selection.
    #[test]
    fn the_end_shows_the_path_the_error_or_the_folder() {
        let edge = game("edge", "Edge", "", "", "");
        assert_eq!(
            status_end(Some(&edge), "config/pc98/"),
            StatusEnd::Path(&edge.shown_path)
        );
        let bad = broken("bad");
        assert!(matches!(
            status_end(Some(&bad), "config/pc98/"),
            StatusEnd::Error(error) if error.ends_with("unknown machine X68030")
        ));
        assert_eq!(
            status_end(None, "config/pc98/"),
            StatusEnd::Path("config/pc98/")
        );
    }
}
