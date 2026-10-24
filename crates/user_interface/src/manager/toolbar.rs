//! The toolbar of the library: the launch of the selected game, the add of
//! a game, the edit, the duplicate and the delete of the selected game and
//! the refresh of the library.

use gui::{
    Rect,
    ui::{Button, ButtonKind, Icon, UiContext},
};

/// The id of the launch button.
pub(crate) const LAUNCH_ID: &str = "launch";

/// The id of the add button.
pub(crate) const ADD_ID: &str = "add game";

/// The id of the edit button.
pub(crate) const EDIT_ID: &str = "edit game";

/// The id of the duplicate button.
pub(crate) const DUPLICATE_ID: &str = "duplicate game";

/// The id of the delete button.
pub(crate) const DELETE_ID: &str = "delete game";

/// The id of the refresh button.
pub(crate) const REFRESH_ID: &str = "refresh";

/// The buttons that the user activated in a pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ToolbarResponse {
    /// Launch was activated.
    pub(crate) launch: bool,
    /// Add was activated.
    pub(crate) add: bool,
    /// Edit was activated.
    pub(crate) edit: bool,
    /// Duplicate was activated.
    pub(crate) duplicate: bool,
    /// Delete was activated.
    pub(crate) delete: bool,
    /// Refresh was activated.
    pub(crate) refresh: bool,
}

/// The state of the buttons of the toolbar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ToolbarState {
    /// A system is selected. A game can join it.
    pub(crate) addable: bool,
    /// The selected game can start. Launch, Edit and Duplicate act on it.
    pub(crate) launchable: bool,
    /// A game file is selected. Delete acts on it.
    pub(crate) deletable: bool,
}

/// Draws the toolbar into `area` from its left edge: Launch, Add, Edit,
/// Duplicate, Delete and Refresh, enabled as `state` says. A disabled button
/// leaves the focus ring.
pub(crate) fn show_toolbar(
    context: &mut UiContext,
    area: Rect,
    state: ToolbarState,
) -> ToolbarResponse {
    let gap = context.theme.gap;
    let buttons = [
        Button::new(LAUNCH_ID, "Launch")
            .kind(ButtonKind::Primary)
            .icon(Icon::Play)
            .enabled(state.launchable),
        Button::new(ADD_ID, "Add")
            .icon(Icon::Add)
            .enabled(state.addable),
        Button::new(EDIT_ID, "Edit")
            .icon(Icon::Edit)
            .enabled(state.launchable),
        Button::new(DUPLICATE_ID, "Duplicate")
            .icon(Icon::Duplicate)
            .enabled(state.launchable),
        Button::new(DELETE_ID, "Delete")
            .icon(Icon::Delete)
            .enabled(state.deletable),
        Button::new(REFRESH_ID, "Refresh").icon(Icon::Refresh),
    ];
    let mut activated = [false; 6];
    let mut rest = area;
    for (index, button) in buttons.into_iter().enumerate() {
        let (button_area, after) = rest.cut_left(button.width(context));
        activated[index] = button.show(context, button_area);
        rest = after.cut_left(gap).1;
    }
    let [launch, add, edit, duplicate, delete, refresh] = activated;
    ToolbarResponse {
        launch,
        add,
        edit,
        duplicate,
        delete,
        refresh,
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::{Navigation, UiId};

    use super::*;
    use crate::gui_test_support::GuiHarness;

    /// The area of the toolbar in the tests.
    const AREA: Rect = Rect::new(16.0, 60.0, 700.0, 32.0);

    /// Every button enabled.
    const ALL: ToolbarState = ToolbarState {
        addable: true,
        launchable: true,
        deletable: true,
    };

    /// Verifies that the enabled buttons join the focus ring in toolbar order and answer confirm.
    #[test]
    fn enabled_buttons_answer_in_toolbar_order() {
        let mut harness = GuiHarness::new(960, 600);
        harness.frame(|context| show_toolbar(context, AREA, ALL));
        let order = [
            LAUNCH_ID,
            ADD_ID,
            EDIT_ID,
            DUPLICATE_ID,
            DELETE_ID,
            REFRESH_ID,
        ];
        for (index, id) in order.iter().enumerate() {
            harness.input.navigation = Some(Navigation::Next);
            harness.frame(|context| show_toolbar(context, AREA, ALL));
            assert_eq!(harness.state.focused(), Some(UiId::from(*id)));
            harness.input.confirm = true;
            let response = harness.frame(|context| show_toolbar(context, AREA, ALL));
            let pressed = [
                response.launch,
                response.add,
                response.edit,
                response.duplicate,
                response.delete,
                response.refresh,
            ];
            for (other, pressed) in pressed.iter().enumerate() {
                assert_eq!(*pressed, other == index, "{id}");
            }
        }
    }

    /// Verifies that the disabled buttons leave the ring: an empty system keeps Add and Refresh, a file with an error keeps Delete.
    #[test]
    fn disabled_buttons_leave_the_ring() {
        let mut harness = GuiHarness::new(960, 600);
        let empty = ToolbarState {
            addable: true,
            launchable: false,
            deletable: false,
        };
        harness.frame(|context| show_toolbar(context, AREA, empty));
        let mut seen = Vec::new();
        for _ in 0..3 {
            harness.input.navigation = Some(Navigation::Next);
            harness.frame(|context| show_toolbar(context, AREA, empty));
            seen.push(harness.state.focused());
        }
        assert_eq!(
            seen,
            [
                Some(UiId::from(ADD_ID)),
                Some(UiId::from(REFRESH_ID)),
                Some(UiId::from(ADD_ID))
            ]
        );
        let broken = ToolbarState {
            addable: true,
            launchable: false,
            deletable: true,
        };
        harness.state.set_focus(UiId::from(ADD_ID));
        harness.frame(|context| show_toolbar(context, AREA, broken));
        harness.input.navigation = Some(Navigation::Next);
        harness.frame(|context| show_toolbar(context, AREA, broken));
        assert_eq!(harness.state.focused(), Some(UiId::from(DELETE_ID)));
    }
}
