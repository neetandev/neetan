//! The tab bar.

use super::{
    context::{UiContext, UiId},
    focus::{Navigation, NavigationSet},
    icons::Icon,
};
use crate::Rect;

/// A tab of a [`tab_bar`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tab<'a> {
    /// The caption.
    pub caption: &'a str,
    /// The icon before the caption.
    pub icon: Option<Icon>,
}

impl<'a> Tab<'a> {
    /// A tab with the caption `caption` and the icon `icon`.
    pub fn new(caption: &'a str, icon: Option<Icon>) -> Self {
        Self { caption, icon }
    }
}

/// The smallest width of a tab.
const MINIMUM_TAB_WIDTH: f32 = 72.0;

/// The height of the line under the selected tab.
const INDICATOR_HEIGHT: f32 = 2.0;

/// The width of `tab` with its padding.
fn tab_width(context: &mut UiContext, tab: &Tab) -> f32 {
    let theme = context.theme;
    let icon = if tab.icon.is_some() {
        theme.icon_size + theme.gap * 0.75
    } else {
        0.0
    };
    let caption = context.measure(&theme.body(), tab.caption);
    (caption + icon + theme.padding * 2.0)
        .max(MINIMUM_TAB_WIDTH)
        .ceil()
}

/// The area of the tab `index` of `tabs` in a bar in `rect`. The tabs take
/// their own widths from the left edge, or share the width equally when
/// they do not fit.
fn tab_area(context: &mut UiContext, rect: Rect, tabs: &[Tab], index: usize) -> Rect {
    let mut x = rect.x;
    let mut area = Rect::new(rect.x, rect.y, 0.0, rect.height);
    for (position, tab) in tabs.iter().enumerate() {
        let width = tab_width(context, tab);
        if position == index {
            area = Rect::new(x, rect.y, width, rect.height);
        }
        x += width;
    }
    if x - rect.x > rect.width {
        let width = rect.width / tabs.len() as f32;
        area = Rect::new(rect.x + index as f32 * width, rect.y, width, rect.height);
    }
    area
}

/// A row of `tabs` in `rect` with the tab `selected` chosen. Returns whether
/// the selection changed in this pass.
///
/// A click selects a tab. While the bar holds the focus, left and right
/// select the neighbor tab, and the focus leaves the bar past its first and
/// last tab. [`Navigation::PreviousTab`] and [`Navigation::NextTab`] select
/// the neighbor tab with wrap around whatever widget holds the focus, unless
/// a layer above blocks the bar. A selection past the last tab selects the
/// last tab without counting as a change.
pub fn tab_bar(
    context: &mut UiContext,
    id: impl Into<UiId>,
    rect: Rect,
    tabs: &[Tab],
    selected: &mut usize,
) -> bool {
    let id = id.into();
    let count = tabs.len();
    if count == 0 {
        return false;
    }
    *selected = (*selected).min(count - 1);
    let before = *selected;
    let shoulder = context.tab_navigation();
    if let Some(navigation) = context.navigation(id).or(shoulder) {
        *selected = match navigation {
            Navigation::Left => selected.saturating_sub(1),
            Navigation::Right => (*selected + 1).min(count - 1),
            Navigation::PreviousTab => (*selected + count - 1) % count,
            Navigation::NextTab => (*selected + 1) % count,
            Navigation::Up
            | Navigation::Down
            | Navigation::PageUp
            | Navigation::PageDown
            | Navigation::Home
            | Navigation::End
            | Navigation::Next
            | Navigation::Previous => *selected,
        };
    }
    for index in 0..count {
        let area = tab_area(context, rect, tabs, index);
        if context.button_behavior(id.indexed(index), area).pressed {
            *selected = index;
            context.set_focus(id);
        }
    }
    let captures = NavigationSet::EMPTY
        .with_if(*selected > 0, Navigation::Left)
        .with_if(*selected + 1 < count, Navigation::Right);
    let selected_area = tab_area(context, rect, tabs, *selected);
    context.focusable(id, selected_area, captures);

    let theme = context.theme;
    let padding = theme.padding;
    let spacing = theme.gap * 0.75;
    context.scene.fill(
        Rect::new(
            rect.x,
            rect.bottom() - theme.stroke,
            rect.width,
            theme.stroke,
        ),
        0.0,
        theme.panel_edge,
    );
    for (index, tab) in tabs.iter().enumerate() {
        let area = tab_area(context, rect, tabs, index);
        let chosen = index == *selected;
        let hot = context.is_hot(id.indexed(index));
        let color = if chosen {
            theme.text
        } else if hot {
            theme.text.mix(theme.text_dim, 0.35)
        } else {
            theme.text_dim
        };
        if hot && !chosen {
            context.scene.fill(
                Rect::new(
                    area.x + 2.0,
                    area.y + 3.0,
                    area.width - 4.0,
                    area.height - 7.0,
                ),
                theme.radius,
                theme.row_hover,
            );
        }
        if chosen {
            context.scene.fill(
                Rect::new(
                    area.x + padding * 0.5,
                    area.bottom() - INDICATOR_HEIGHT,
                    area.width - padding,
                    INDICATOR_HEIGHT,
                ),
                INDICATOR_HEIGHT * 0.5,
                theme.accent,
            );
        }
        let inset = padding.min(area.width * 0.25);
        let inner = Rect::new(
            area.x + inset,
            area.y,
            area.width - inset * 2.0,
            area.height,
        );
        let style = theme.body().with_color(color);
        let Some(icon) = tab.icon else {
            context.text_centered(inner, &style, tab.caption);
            continue;
        };
        let caption_width = context
            .measure(&style, tab.caption)
            .min(inner.width - theme.icon_size - spacing)
            .max(0.0);
        let group = theme.icon_size + spacing + caption_width;
        let left = context.snap(area.x + (area.width - group) * 0.5);
        context.icon(
            Rect::new(left, area.y, theme.icon_size, area.height),
            icon,
            theme.icon_size,
            color,
        );
        let caption = Rect::new(
            left + theme.icon_size + spacing,
            area.y,
            caption_width + 1.0,
            area.height,
        );
        context.text_left(caption, &style, tab.caption);
    }
    if context.has_focus(id) {
        let area = tab_area(context, rect, tabs, *selected);
        context.focus_ring(
            Rect::new(
                area.x + 2.0,
                area.y + 2.0,
                area.width - 4.0,
                area.height - 6.0,
            ),
            theme.radius,
        );
    }
    *selected != before
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Icon, tests::Harness};

    /// The tabs of the media dialog.
    const TABS: [Tab; 3] = [
        Tab {
            caption: "FDD 1",
            icon: Some(Icon::Disk),
        },
        Tab {
            caption: "FDD 2",
            icon: Some(Icon::Disk),
        },
        Tab {
            caption: "CD-ROM",
            icon: Some(Icon::Disc),
        },
    ];

    /// The area of the tab bar.
    const BAR: Rect = Rect::new(0.0, 0.0, 600.0, 36.0);

    #[test]
    fn a_click_selects_a_tab_and_takes_the_focus() {
        let mut harness = Harness::new();
        let mut selected = 0;
        let third = harness.frame(|context| {
            tab_bar(context, "tabs", BAR, &TABS, &mut selected);
            tab_area(context, BAR, &TABS, 2)
        });
        assert!(third.x > 150.0 && third.right() < 400.0, "{third:?}");
        let (x, y) = third.center();
        harness.press(x, y);
        let changed = harness.frame(|context| tab_bar(context, "tabs", BAR, &TABS, &mut selected));
        assert!(changed);
        assert_eq!(selected, 2);
        assert_eq!(harness.state.focused(), Some(UiId::from("tabs")));
        harness.release();
        harness.frame(|context| tab_bar(context, "tabs", BAR, &TABS, &mut selected));
        harness.press(x, y);
        let again = harness.frame(|context| tab_bar(context, "tabs", BAR, &TABS, &mut selected));
        assert!(!again);
        harness.release();
        let mut out_of_range = 7;
        let clamped =
            harness.frame(|context| tab_bar(context, "tabs", BAR, &TABS, &mut out_of_range));
        assert!(!clamped);
        assert_eq!(out_of_range, 2);
    }

    #[test]
    fn arrows_walk_the_tabs_and_leave_at_the_ends() {
        let mut harness = Harness::new();
        let mut selected = 0;
        let other = UiId::from("other");
        let mut draw = |harness: &mut Harness| {
            harness.frame(|context| {
                let changed = tab_bar(context, "tabs", BAR, &TABS, &mut selected);
                context.focusable(
                    other,
                    Rect::new(620.0, 0.0, 30.0, 36.0),
                    NavigationSet::EMPTY,
                );
                changed
            })
        };
        harness.state.set_focus(UiId::from("tabs"));
        draw(&mut harness);
        harness.navigate(Navigation::Right);
        assert!(draw(&mut harness));
        harness.navigate(Navigation::Right);
        assert!(draw(&mut harness));
        harness.navigate(Navigation::Right);
        assert!(!draw(&mut harness));
        assert_eq!(harness.state.focused(), Some(other));
        harness.navigate(Navigation::Left);
        draw(&mut harness);
        harness.navigate(Navigation::Left);
        draw(&mut harness);
        assert_eq!(harness.state.focused(), Some(UiId::from("tabs")));
        assert_eq!(selected, 1);
    }

    #[test]
    fn tabs_that_do_not_fit_share_the_width() {
        let mut harness = Harness::new();
        let narrow = Rect::new(0.0, 0.0, 180.0, 36.0);
        let areas =
            harness.frame(|context| [0, 1, 2].map(|index| tab_area(context, narrow, &TABS, index)));
        assert_eq!(areas[1], Rect::new(60.0, 0.0, 60.0, 36.0));
        assert_eq!(areas[2].right(), 180.0);
    }

    #[test]
    fn shoulder_buttons_switch_tabs_without_the_focus() {
        let mut harness = Harness::new();
        let mut selected = 0;
        harness.navigate(Navigation::PreviousTab);
        harness.frame(|context| tab_bar(context, "tabs", BAR, &TABS, &mut selected));
        assert_eq!(selected, 2);
        harness.navigate(Navigation::NextTab);
        harness.frame(|context| tab_bar(context, "tabs", BAR, &TABS, &mut selected));
        harness.navigate(Navigation::NextTab);
        harness.frame(|context| tab_bar(context, "tabs", BAR, &TABS, &mut selected));
        assert_eq!(selected, 1);
        assert_eq!(harness.state.focused(), None);
    }

    #[test]
    fn a_blocked_tab_bar_ignores_the_shoulder_buttons() {
        let mut harness = Harness::new();
        let mut selected = 0;
        let mut draw = |harness: &mut Harness| {
            harness.frame(|context| {
                tab_bar(context, "tabs", BAR, &TABS, &mut selected);
                context.layer("dialog", |_| {});
            });
        };
        draw(&mut harness);
        harness.navigate(Navigation::NextTab);
        draw(&mut harness);
        assert_eq!(selected, 0);
    }
}
