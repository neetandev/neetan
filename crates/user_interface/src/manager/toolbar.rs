//! The toolbar of the library: the actions on the selected game and the refresh of the library.

use gui::{
    Rect,
    ui::{Button, ButtonKind, Icon, UiContext},
};

/// The id of the launch button.
pub(crate) const LAUNCH_ID: &str = "launch";

/// The id of the delete button.
pub(crate) const DELETE_ID: &str = "delete game";

/// The id of the refresh button.
pub(crate) const REFRESH_ID: &str = "refresh";

/// The buttons that the user activated in a pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ToolbarResponse {
    /// Launch was activated.
    pub(crate) launch: bool,
    /// Delete was activated.
    pub(crate) delete: bool,
    /// Refresh was activated.
    pub(crate) refresh: bool,
}

/// Draws the toolbar into `area` from its left edge: Launch, enabled while
/// `launchable`, Delete, enabled while `deletable`, and Refresh. A disabled
/// button leaves the focus ring.
pub(crate) fn show_toolbar(
    context: &mut UiContext,
    area: Rect,
    launchable: bool,
    deletable: bool,
) -> ToolbarResponse {
    let gap = context.theme.gap;
    let launch = Button::new(LAUNCH_ID, "Launch")
        .kind(ButtonKind::Primary)
        .icon(Icon::Play)
        .enabled(launchable);
    let delete = Button::new(DELETE_ID, "Delete")
        .icon(Icon::Delete)
        .enabled(deletable);
    let refresh = Button::new(REFRESH_ID, "Refresh").icon(Icon::Refresh);
    let (launch_area, rest) = area.cut_left(launch.width(context));
    let (delete_area, rest) = rest.cut_left(gap).1.cut_left(delete.width(context));
    let refresh_area = rest.cut_left(gap).1.cut_left(refresh.width(context)).0;
    ToolbarResponse {
        launch: launch.show(context, launch_area),
        delete: delete.show(context, delete_area),
        refresh: refresh.show(context, refresh_area),
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::{Navigation, UiId};

    use super::*;
    use crate::gui_test_support::GuiHarness;

    /// The area of the toolbar in the tests.
    const AREA: Rect = Rect::new(16.0, 60.0, 600.0, 32.0);

    /// Verifies that the enabled buttons join the focus ring and answer confirm, and that disabled ones leave it.
    #[test]
    fn enabled_buttons_answer_and_disabled_ones_leave_the_ring() {
        let mut harness = GuiHarness::new(960, 600);
        harness.frame(|context| show_toolbar(context, AREA, true, true));
        harness.input.navigation = Some(Navigation::Next);
        harness.frame(|context| show_toolbar(context, AREA, true, true));
        assert_eq!(harness.state.focused(), Some(UiId::from(LAUNCH_ID)));
        harness.input.confirm = true;
        let response = harness.frame(|context| show_toolbar(context, AREA, true, true));
        assert!(response.launch && !response.delete);
        harness.input.navigation = Some(Navigation::Next);
        harness.frame(|context| show_toolbar(context, AREA, true, true));
        harness.input.confirm = true;
        let response = harness.frame(|context| show_toolbar(context, AREA, true, true));
        assert!(response.delete && !response.launch);
        harness.input.navigation = Some(Navigation::Next);
        harness.frame(|context| show_toolbar(context, AREA, true, true));
        assert_eq!(harness.state.focused(), Some(UiId::from(REFRESH_ID)));
        harness.input.confirm = true;
        let response = harness.frame(|context| show_toolbar(context, AREA, true, true));
        assert!(response.refresh && !response.delete);

        harness.state.set_focus(UiId::from(LAUNCH_ID));
        harness.frame(|context| show_toolbar(context, AREA, false, true));
        assert_eq!(harness.state.focused(), None);
        harness.input.navigation = Some(Navigation::Next);
        harness.frame(|context| show_toolbar(context, AREA, false, true));
        assert_eq!(harness.state.focused(), Some(UiId::from(DELETE_ID)));
        harness.frame(|context| show_toolbar(context, AREA, false, false));
        assert_eq!(harness.state.focused(), None);
        harness.input.navigation = Some(Navigation::Next);
        harness.frame(|context| show_toolbar(context, AREA, false, false));
        assert_eq!(harness.state.focused(), Some(UiId::from(REFRESH_ID)));
    }
}
