//! The media list of the editor: the ordered images of one drive in a list,
//! with the buttons that add an image through the file browser, remove the
//! selected image and shift it up or down.

use gui::{
    Rect,
    ui::{
        ActionButton, Align, Button, Checkbox, Icon, LIST_PADDING, List, ListState, NavigationSet,
        RowContent, RowMark, Theme, Truncation, UiContext, UiId, label,
    },
};
use machine_factory::config::phrases::join_names;

use crate::config::resolve_path;

/// The fewest rows that a media list shows.
pub(crate) const MEDIA_LIST_MINIMUM_ROWS: usize = 2;

/// The most rows that a media list shows before it scrolls.
pub(crate) const MEDIA_LIST_MAXIMUM_ROWS: usize = 5;

/// The request of a media list for the file browser.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum MediaBrowse {
    /// The chosen image joins the end of the list.
    Add,
    /// The chosen image takes the place of the image at this index.
    Replace(usize),
}

/// What a media list did in a pass.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub(crate) struct MediaListResponse {
    /// The images changed.
    pub(crate) changed: bool,
    /// The list asks the file browser for an image.
    pub(crate) browse: Option<MediaBrowse>,
    /// The checkbox of the empty list flipped.
    pub(crate) empty_list_flipped: bool,
}

/// Shifts the image at `index` of `values` one place up or down. Returns
/// the new index, or `None` at the end of the list.
pub(crate) fn shift_image(values: &mut [String], index: usize, up: bool) -> Option<usize> {
    let target = if up { index.checked_sub(1)? } else { index + 1 };
    if index >= values.len() || target >= values.len() {
        return None;
    }
    values.swap(index, target);
    Some(target)
}

/// Removes the image at `index` of `values`. Returns the index of the image
/// that takes the selection.
pub(crate) fn remove_image(values: &mut Vec<String>, index: usize) -> usize {
    if index < values.len() {
        values.remove(index);
    }
    index.min(values.len().saturating_sub(1))
}

/// Puts `value` into `values` as `browse` asks. Returns the index of the image.
pub(crate) fn place_image(values: &mut Vec<String>, browse: MediaBrowse, value: String) -> usize {
    match browse {
        MediaBrowse::Replace(index) if index < values.len() => {
            values[index] = value;
            index
        }
        MediaBrowse::Replace(_) | MediaBrowse::Add => {
            values.push(value);
            values.len() - 1
        }
    }
}

/// Returns whether the image `value` lies outside the file system, resolved
/// against `root` as a launch resolves it.
pub(crate) fn is_missing(value: &str, root: &std::path::Path) -> bool {
    !value.trim().is_empty() && !resolve_path(root, std::path::Path::new(value.trim())).exists()
}

/// The warning under a list whose images at the 0-based `missing` indices do not exist.
pub(crate) fn missing_note(missing: &[usize]) -> Option<String> {
    let places: Vec<String> = missing
        .iter()
        .map(|index| (index + 1).to_string())
        .collect();
    let places: Vec<&str> = places.iter().map(String::as_str).collect();
    match places.as_slice() {
        [] => None,
        [one] => Some(format!(
            "Image {one} does not exist. The file saves, and the launch fails while it is \
             missing."
        )),
        many => Some(format!(
            "Images {} do not exist. The file saves, and the launch fails while they are \
             missing.",
            join_names(many, "and")
        )),
    }
}

/// The height of a media list of `rows` images: its rows between the
/// fewest and the most rows, with the padding and the edge of a list.
pub(crate) fn list_height(theme: &Theme, rows: usize) -> f32 {
    let shown = rows.clamp(MEDIA_LIST_MINIMUM_ROWS, MEDIA_LIST_MAXIMUM_ROWS) as f32;
    shown * theme.row_height + LIST_PADDING * 2.0 + theme.stroke * 2.0
}

/// The width of the two columns of buttons beside a media list.
pub(crate) fn buttons_width(theme: &Theme) -> f32 {
    theme.control_height * 2.0 + theme.gap
}

/// The height that the checkbox of the empty list adds below a media list.
pub(crate) fn empty_list_height(theme: &Theme) -> f32 {
    theme.gap * 0.5 + theme.control_height
}

/// The id of the checkbox of the empty list of the media list `id`.
pub(crate) fn empty_list_id(id: UiId) -> UiId {
    id.child("empty")
}

/// The ids of the buttons of the media list `id`.
pub(crate) fn button_ids(id: UiId) -> [UiId; 4] {
    [
        id.child("add"),
        id.child("remove"),
        id.child("up"),
        id.child("down"),
    ]
}

/// A media list of one drive.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MediaList<'a> {
    /// The id of the list.
    pub(crate) id: UiId,
    /// The icon of the images.
    pub(crate) icon: Icon,
    /// Whether each image is missing, in the order of the images.
    pub(crate) missing: &'a [bool],
    /// Whether the drive is there on the model of the game.
    pub(crate) enabled: bool,
    /// The text of a list without images: the empty drive or why the model has no drive.
    pub(crate) hint: &'a str,
    /// The cut of a hint wider than the list. A hint with inherited images
    /// cuts in the middle, a sentence cuts at its end.
    pub(crate) hint_truncation: Truncation,
    /// The caption of the checkbox below a list without images that empties
    /// the drive over its inherited list. `None` draws no checkbox.
    pub(crate) empty_list_caption: Option<&'a str>,
    /// Whether the file empties the drive over its inherited list.
    pub(crate) empty_list: bool,
}

impl MediaList<'_> {
    /// Draws the list with its buttons in `rect` over `values`. The list
    /// takes the height of `rect`, the buttons take two columns at its right.
    /// The checkbox of the empty list draws below `rect`. Gamepad X on the
    /// focused list removes the selected image.
    pub(crate) fn show(
        self,
        context: &mut UiContext,
        rect: Rect,
        state: &mut ListState,
        values: &mut Vec<String>,
    ) -> MediaListResponse {
        let theme = context.theme;
        let mut response = MediaListResponse::default();
        let (buttons_area, list_area) = rect.cut_right(buttons_width(theme) + theme.gap);
        let buttons_area = buttons_area.cut_left(theme.gap).1;
        if values.is_empty() {
            List::new(self.id, 0)
                .enabled(self.enabled)
                .show(context, list_area, state, |_, _| {});
            if self.enabled {
                context.focusable(self.id, list_area, NavigationSet::EMPTY);
                if context.confirmed(self.id) {
                    response.browse = Some(MediaBrowse::Add);
                }
                if context.button_behavior(self.id, list_area).pressed {
                    context.set_focus(self.id);
                }
                if context.shows_focus(self.id) {
                    context.focus_ring(list_area, theme.radius);
                }
            }
            let color = if self.enabled {
                theme.colors.text_dim
            } else {
                theme.colors.text_disabled
            };
            label(
                context,
                list_area.inset(theme.gap),
                &theme.small().with_color(color),
                self.hint,
                Align::Center,
                self.hint_truncation,
            );
        } else {
            let numbers: Vec<String> = (1..=values.len()).map(|place| place.to_string()).collect();
            let list_response = List::new(self.id, values.len()).enabled(self.enabled).show(
                context,
                list_area,
                state,
                |context, row| {
                    let missing = self.missing.get(row.index).copied().unwrap_or(false);
                    RowContent::new(&values[row.index])
                        .number(&numbers[row.index])
                        .icon(Some(self.icon))
                        .mark(missing.then_some(RowMark::Alert))
                        .show(context, row);
                },
            );
            if let Some(index) = list_response.activated {
                response.browse = Some(MediaBrowse::Replace(index));
            }
        }

        let selected = state.selected.min(values.len().saturating_sub(1));
        let has_images = !values.is_empty();
        let [add_id, remove_id, up_id, down_id] = button_ids(self.id);
        let size = theme.control_height;
        let cell = |column: usize, row: usize| {
            Rect::new(
                buttons_area.x + column as f32 * (size + theme.gap),
                buttons_area.y + row as f32 * (size + theme.gap),
                size,
                size,
            )
        };
        let buttons = [
            (add_id, Icon::Add, self.enabled, cell(0, 0)),
            (
                remove_id,
                Icon::Delete,
                self.enabled && has_images,
                cell(1, 0),
            ),
            (
                up_id,
                Icon::Up,
                self.enabled && has_images && selected > 0,
                cell(0, 1),
            ),
            (
                down_id,
                Icon::Down,
                self.enabled && selected + 1 < values.len(),
                cell(1, 1),
            ),
        ];
        let mut pressed = [false; 4];
        for (index, (id, icon, enabled, area)) in buttons.into_iter().enumerate() {
            pressed[index] = Button::new(id, "")
                .icon(icon)
                .enabled(enabled)
                .show(context, area);
        }
        let [add, mut remove, up, down] = pressed;
        if self.enabled
            && has_images
            && context.has_focus(self.id)
            && context.action_button() == Some(ActionButton::X)
        {
            remove = true;
        }
        if let Some(caption) = self.empty_list_caption.filter(|_| !has_images) {
            let area = Rect::new(
                list_area.x,
                rect.bottom() + theme.gap * 0.5,
                list_area.width,
                theme.control_height,
            );
            let mut empty_list = self.empty_list;
            response.empty_list_flipped = Checkbox::new(empty_list_id(self.id), caption)
                .enabled(self.enabled)
                .show(context, area, &mut empty_list);
        }
        if add {
            response.browse = Some(MediaBrowse::Add);
        }
        if remove {
            state.select(remove_image(values, selected));
            response.changed = true;
            if values.is_empty() {
                context.set_focus(add_id);
            }
        }
        for (shifted, up) in [(up, true), (down, false)] {
            if shifted && let Some(index) = shift_image(values, selected, up) {
                state.select(index);
                response.changed = true;
            }
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use gui::ui::Navigation;

    use super::*;
    use crate::gui_test_support::GuiHarness;

    /// The area of a list of four rows in the tests.
    const AREA: Rect = Rect::new(200.0, 100.0, 600.0, 130.0);

    /// Three images.
    fn images() -> Vec<String> {
        ["a.hdm", "b.hdm", "c.hdm"]
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    /// Verifies the shift, the removal and the placement of images.
    #[test]
    fn images_shift_remove_and_take_their_place() {
        let mut values = images();
        assert_eq!(shift_image(&mut values, 0, true), None);
        assert_eq!(shift_image(&mut values, 0, false), Some(1));
        assert_eq!(values, ["b.hdm", "a.hdm", "c.hdm"]);
        assert_eq!(shift_image(&mut values, 2, false), None);
        assert_eq!(shift_image(&mut values, 2, true), Some(1));
        assert_eq!(values, ["b.hdm", "c.hdm", "a.hdm"]);
        assert_eq!(remove_image(&mut values, 2), 1);
        assert_eq!(values, ["b.hdm", "c.hdm"]);
        assert_eq!(
            place_image(&mut values, MediaBrowse::Add, "d.hdm".to_owned()),
            2
        );
        assert_eq!(
            place_image(&mut values, MediaBrowse::Replace(0), "e.hdm".to_owned()),
            0
        );
        assert_eq!(values, ["e.hdm", "c.hdm", "d.hdm"]);
        assert_eq!(remove_image(&mut values, 0), 0);
        assert_eq!(remove_image(&mut values, 0), 0);
        assert_eq!(remove_image(&mut values, 0), 0);
        assert!(values.is_empty());
    }

    /// Verifies the warning of missing images and the missing check against the root.
    #[test]
    fn missing_images_give_a_warning() {
        assert_eq!(missing_note(&[]), None);
        assert_eq!(
            missing_note(&[2]).as_deref(),
            Some(
                "Image 3 does not exist. The file saves, and the launch fails while it is missing."
            )
        );
        assert_eq!(
            missing_note(&[0, 1, 3]).as_deref(),
            Some(
                "Images 1, 2 and 4 do not exist. The file saves, and the launch fails while they \
                 are missing."
            )
        );
        let root = std::env::temp_dir();
        assert!(is_missing("neetan-surely-absent/disk.d88", &root));
        assert!(!is_missing("", &root));
        assert!(!is_missing(&root.to_string_lossy(), &root));
    }

    /// Verifies the height of a list between two and five rows.
    #[test]
    fn lists_show_two_to_five_rows() {
        let harness = GuiHarness::new(960, 600);
        let theme = &harness.theme;
        let row = theme.row_height;
        let frame = LIST_PADDING * 2.0 + theme.stroke * 2.0;
        assert_eq!(list_height(theme, 0), row * 2.0 + frame);
        assert_eq!(list_height(theme, 4), row * 4.0 + frame);
        assert_eq!(list_height(theme, 9), row * 5.0 + frame);
    }

    /// Verifies that the list is one stop with its buttons after it, that the buttons reorder and remove the selected image, and that confirm asks the browser.
    #[test]
    fn the_buttons_reorder_and_remove_the_selected_image() {
        let mut harness = GuiHarness::new(960, 600);
        let mut state = ListState::new();
        let mut values = images();
        let missing = [false, true, false];
        let id = UiId::from("fdd1");
        let draw = |harness: &mut GuiHarness, values: &mut Vec<String>, state: &mut ListState| {
            harness.settle(|context| {
                MediaList {
                    id,
                    icon: Icon::Disk,
                    missing: &missing,
                    enabled: true,
                    hint: "Empty",
                    hint_truncation: Truncation::End,
                    empty_list_caption: None,
                    empty_list: false,
                }
                .show(context, AREA, state, values)
            })
        };
        harness.state.set_focus(id);
        draw(&mut harness, &mut values, &mut state);
        harness.input.confirm = true;
        let response = draw(&mut harness, &mut values, &mut state);
        assert_eq!(response.browse, Some(MediaBrowse::Replace(0)));

        let [add_id, remove_id, up_id, down_id] = button_ids(id);
        harness.state.set_focus(down_id);
        draw(&mut harness, &mut values, &mut state);
        harness.input.confirm = true;
        assert!(draw(&mut harness, &mut values, &mut state).changed);
        assert_eq!(values, ["b.hdm", "a.hdm", "c.hdm"]);
        assert_eq!(state.selected, 1);
        harness.state.set_focus(up_id);
        draw(&mut harness, &mut values, &mut state);
        harness.input.confirm = true;
        draw(&mut harness, &mut values, &mut state);
        assert_eq!(values, images());
        assert_eq!(state.selected, 0);

        harness.state.set_focus(id);
        draw(&mut harness, &mut values, &mut state);
        for expected in [add_id, remove_id, down_id] {
            harness.input.navigation = Some(Navigation::Next);
            draw(&mut harness, &mut values, &mut state);
            assert_eq!(harness.state.focused(), Some(expected));
        }

        harness.state.set_focus(remove_id);
        for _ in 0..3 {
            harness.input.confirm = true;
            draw(&mut harness, &mut values, &mut state);
        }
        assert!(values.is_empty());
        assert_eq!(harness.state.focused(), Some(add_id));
        harness.state.set_focus(id);
        draw(&mut harness, &mut values, &mut state);
        assert_eq!(harness.state.focused(), Some(id));
        harness.input.confirm = true;
        assert_eq!(
            draw(&mut harness, &mut values, &mut state).browse,
            Some(MediaBrowse::Add)
        );
    }

    /// Verifies that a disabled list leaves the ring with all its buttons.
    #[test]
    fn a_disabled_list_leaves_the_ring() {
        let mut harness = GuiHarness::new(960, 600);
        let mut state = ListState::new();
        let mut values = Vec::new();
        let id = UiId::from("cdrom");
        harness.state.set_focus(id);
        harness.settle(|context| {
            MediaList {
                id,
                icon: Icon::Disc,
                missing: &[],
                enabled: false,
                hint: "PC9801RA has no CD-ROM drive.",
                hint_truncation: Truncation::End,
                empty_list_caption: Some("Start without a disc"),
                empty_list: false,
            }
            .show(context, AREA, &mut state, &mut values)
        });
        assert_eq!(harness.state.focused(), None);
    }

    /// Verifies that gamepad X on the focused list removes the selected image and that X elsewhere leaves the images alone.
    #[test]
    fn gamepad_x_removes_the_selected_image() {
        let mut harness = GuiHarness::new(960, 600);
        let mut state = ListState::new();
        let mut values = images();
        let id = UiId::from("fdd1");
        let draw = |harness: &mut GuiHarness, values: &mut Vec<String>, state: &mut ListState| {
            harness.settle(|context| {
                MediaList {
                    id,
                    icon: Icon::Disk,
                    missing: &[],
                    enabled: true,
                    hint: "Empty",
                    hint_truncation: Truncation::End,
                    empty_list_caption: None,
                    empty_list: false,
                }
                .show(context, AREA, state, values)
            })
        };
        harness.state.set_focus(id);
        draw(&mut harness, &mut values, &mut state);
        state.select(1);
        harness.input.action = Some(ActionButton::X);
        assert!(draw(&mut harness, &mut values, &mut state).changed);
        assert_eq!(values, ["a.hdm", "c.hdm"]);
        let [add_id, ..] = button_ids(id);
        harness.state.set_focus(add_id);
        draw(&mut harness, &mut values, &mut state);
        harness.input.action = Some(ActionButton::X);
        assert!(!draw(&mut harness, &mut values, &mut state).changed);
        assert_eq!(values.len(), 2);
    }

    /// Verifies that the checkbox of an empty list follows the buttons in the ring and flips the empty list.
    #[test]
    fn the_empty_list_checkbox_flips_the_empty_list() {
        let mut harness = GuiHarness::new(960, 600);
        let mut state = ListState::new();
        let mut values = Vec::new();
        let id = UiId::from("fdd2");
        let mut empty_list = false;
        let mut draw = |harness: &mut GuiHarness, empty_list: &mut bool| {
            let response = harness.settle(|context| {
                MediaList {
                    id,
                    icon: Icon::Disk,
                    missing: &[],
                    enabled: true,
                    hint: "Settings default: games/boot.d88",
                    hint_truncation: Truncation::Middle,
                    empty_list_caption: Some("Start without a disk"),
                    empty_list: *empty_list,
                }
                .show(context, AREA, &mut state, &mut values)
            });
            if response.empty_list_flipped {
                *empty_list = !*empty_list;
            }
        };
        harness.state.set_focus(id);
        draw(&mut harness, &mut empty_list);
        harness.input.navigation = Some(Navigation::Next);
        draw(&mut harness, &mut empty_list);
        harness.input.navigation = Some(Navigation::Next);
        draw(&mut harness, &mut empty_list);
        assert_eq!(harness.state.focused(), Some(empty_list_id(id)));
        harness.input.confirm = true;
        draw(&mut harness, &mut empty_list);
        assert!(empty_list);
        harness.input.confirm = true;
        draw(&mut harness, &mut empty_list);
        assert!(!empty_list);
    }
}
