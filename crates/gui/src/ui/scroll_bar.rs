//! The vertical scroll bar of a list and a scroll area.

use super::{
    context::{UiContext, UiId},
    list::LIST_PADDING,
    state::Drag,
};
use crate::Rect;

/// The smallest height of the scroll bar thumb.
pub(super) const MINIMUM_THUMB: f32 = 24.0;

/// The width of the thumb inside the scroll bar while the pointer is away.
const THUMB_WIDTH: f32 = 4.0;

/// The width of the thumb inside the scroll bar under the pointer.
const THUMB_WIDTH_HOVER: f32 = 6.0;

/// The scroll bar at the right edge of a view over a taller content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ScrollBar {
    /// The id of the bar.
    id: UiId,
    /// The view the bar scrolls.
    view: Rect,
    /// The area of the bar at the right edge of the view.
    bar: Rect,
    /// The distance the content reaches past the view.
    overflow: f32,
    /// The height of the thumb.
    thumb_height: f32,
    /// The distance the thumb travels.
    travel: f32,
}

impl ScrollBar {
    /// The bar `id` of `view` over a content of `content_height`.
    pub(super) fn new(context: &UiContext, id: UiId, view: Rect, content_height: f32) -> Self {
        let theme = context.theme;
        let content_height = content_height.max(view.height);
        let thumb_height = (view.height * view.height / content_height.max(1.0))
            .max(MINIMUM_THUMB)
            .min(view.height);
        Self {
            id,
            view,
            bar: Rect::new(
                view.right() - theme.scroll_bar_width,
                view.y,
                theme.scroll_bar_width,
                view.height,
            ),
            overflow: (content_height - view.height).max(0.0),
            thumb_height,
            travel: (view.height - thumb_height).max(1.0),
        }
    }

    /// The distance the content reaches past the view.
    pub(super) fn overflow(&self) -> f32 {
        self.overflow
    }

    /// The top of the thumb at the scroll offset `offset`.
    fn thumb_top(&self, offset: f32) -> f32 {
        let share = if self.overflow > 0.0 {
            offset / self.overflow
        } else {
            0.0
        };
        self.view.y + share * self.travel
    }

    /// Runs the pointer behavior of the bar over the scroll offset `offset`
    /// and returns whether the bar is under the pointer or in a drag. A
    /// press on the thumb drags it, and a press on the track centers the
    /// thumb under the pointer and drags it. A bar without overflow does
    /// nothing.
    pub(super) fn interact(&self, context: &mut UiContext, offset: &mut f32) -> bool {
        if self.overflow <= 0.0 {
            return false;
        }
        let dragging = context.is_active(self.id);
        let pointer = context.button_behavior(self.id, self.bar);
        let (_, mouse_y) = context.mouse();
        if pointer.pressed {
            let thumb_y = self.thumb_top(*offset);
            let grab = if mouse_y >= thumb_y && mouse_y < thumb_y + self.thumb_height {
                mouse_y - thumb_y
            } else {
                self.thumb_height * 0.5
            };
            context.set_drag(Some(Drag { offset: grab }));
        }
        if pointer.held {
            if let Some(drag) = context.drag() {
                *offset = (mouse_y - drag.offset - self.view.y) / self.travel * self.overflow;
            }
            context.mouse_captured = true;
        } else if dragging {
            context.set_drag(None);
        }
        pointer.hovered || pointer.held
    }

    /// Draws the thumb at the scroll offset `offset`. A `hot` thumb under
    /// the pointer or in a drag draws wider and brighter. A bar without
    /// overflow draws nothing.
    pub(super) fn draw(&self, context: &mut UiContext, offset: f32, hot: bool) {
        if self.overflow <= 0.0 {
            return;
        }
        let theme = context.theme;
        let width = if hot { THUMB_WIDTH_HOVER } else { THUMB_WIDTH };
        let thumb = Rect::new(
            self.bar.right() - LIST_PADDING * 0.5 - width,
            self.thumb_top(offset) + 2.0,
            width,
            self.thumb_height - 4.0,
        );
        let color = if hot {
            theme.colors.scroll_thumb_hover
        } else {
            theme.colors.scroll_thumb
        };
        context.scene.fill(thumb, width * 0.5, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tests::Harness;

    #[test]
    fn the_thumb_shrinks_with_the_content_down_to_its_minimum() {
        let mut harness = Harness::new();
        let view = Rect::new(0.0, 0.0, 100.0, 200.0);
        let (short, long, tall) = harness.frame(|context| {
            (
                ScrollBar::new(context, UiId::from("bar"), view, 100.0),
                ScrollBar::new(context, UiId::from("bar"), view, 400.0),
                ScrollBar::new(context, UiId::from("bar"), view, 100_000.0),
            )
        });
        assert_eq!(short.overflow(), 0.0);
        assert_eq!(long.thumb_height, 100.0);
        assert_eq!(long.overflow(), 200.0);
        assert_eq!(tall.thumb_height, MINIMUM_THUMB);
    }
}
