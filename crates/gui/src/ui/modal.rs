//! The modal panel.

use super::{
    button::icon_button,
    context::{UiContext, UiId},
    icons::Icon,
};
use crate::{Rect, RectStyle};

/// The height of the header of a modal panel.
pub const MODAL_HEADER_HEIGHT: f32 = 52.0;

/// The smallest distance between a modal panel and the edge of the target.
const MARGIN: f32 = 16.0;

/// The size of the close button in the header.
const CLOSE_SIZE: f32 = 32.0;

/// The vertical offset of the panel shadow.
const SHADOW_OFFSET: f32 = 6.0;

/// The layout and the events of a [`modal`] panel in a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ModalResponse {
    /// The whole panel.
    pub panel: Rect,
    /// The area below the header inside the padding, for the widgets of the
    /// panel.
    pub content: Rect,
    /// The close button or a cancel that no focused widget took asked to
    /// close the panel.
    pub closed: bool,
    /// The left button went down outside the panel.
    pub outside_press: bool,
}

/// A modal panel of `size` logical units with the title `title`, centered
/// over a veil that dims the whole target. The panel shrinks to fit the
/// target. The header holds the title and a close button. The caller draws
/// the widgets of the panel into [`ModalResponse::content`] after the call.
pub fn modal(
    context: &mut UiContext,
    id: impl Into<UiId>,
    size: (f32, f32),
    title: &str,
) -> ModalResponse {
    let id = id.into();
    let theme = context.theme;
    let bounds = context.bounds();
    context.scene.fill(bounds, 0.0, theme.veil);
    let width = size.0.min(bounds.width - MARGIN * 2.0).max(0.0);
    let height = size.1.min(bounds.height - MARGIN * 2.0).max(0.0);
    let panel = context.snap_rect(Rect::new(
        (bounds.width - width) * 0.5,
        (bounds.height - height) * 0.5,
        width,
        height,
    ));
    context.scene.shadow(
        panel,
        theme.panel_radius,
        theme.shadow_blur,
        (0.0, SHADOW_OFFSET),
        theme.shadow,
    );
    context.rect(
        panel,
        &RectStyle::filled(theme.panel, theme.panel_radius)
            .with_stroke(theme.stroke, theme.panel_edge),
    );
    let (header, body) = panel.cut_top(MODAL_HEADER_HEIGHT);
    let close_area = Rect::new(
        header.right() - theme.padding * 0.75 - CLOSE_SIZE,
        header.y + (header.height - CLOSE_SIZE) * 0.5,
        CLOSE_SIZE,
        CLOSE_SIZE,
    );
    let title_area = Rect::new(
        header.x + theme.padding,
        header.y,
        (close_area.x - theme.gap - header.x - theme.padding).max(0.0),
        header.height,
    );
    let style = theme.title();
    context.text_left(title_area, &style, title);
    let close = icon_button(context, id.child("close"), close_area, Icon::Close);
    let (x, y) = context.mouse();
    ModalResponse {
        panel,
        content: Rect::new(
            body.x + theme.padding,
            body.y,
            (body.width - theme.padding * 2.0).max(0.0),
            (body.height - theme.padding).max(0.0),
        ),
        closed: close || context.cancelled(),
        outside_press: context.input.left_pressed() && !panel.contains(x, y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Viewport, tests::Harness};

    #[test]
    fn the_panel_centers_and_fits_the_target() {
        let mut harness = Harness::new();
        let response = harness.frame(|context| modal(context, "media", (400.0, 300.0), "Media"));
        assert_eq!(response.panel, Rect::new(200.0, 150.0, 400.0, 300.0));
        assert_eq!(response.content.x, 216.0);
        assert_eq!(response.content.y, 150.0 + MODAL_HEADER_HEIGHT);
        assert_eq!(response.content.bottom(), 434.0);
        assert!(!response.closed);
        let veil = harness.scene.instances()[0];
        assert_eq!(veil.rect, [0.0, 0.0, 800.0, 600.0]);

        let response = harness.frame(|context| modal(context, "media", (2000.0, 300.0), "Media"));
        assert_eq!(response.panel.x, MARGIN);
        assert_eq!(response.panel.width, 800.0 - MARGIN * 2.0);
    }

    #[test]
    fn the_panel_snaps_to_physical_pixels() {
        let mut harness = Harness::with_viewport(Viewport::new(801, 601, 1.5));
        let response = harness.frame(|context| modal(context, "media", (301.0, 201.0), "Media"));
        for edge in [
            response.panel.x,
            response.panel.y,
            response.panel.right(),
            response.panel.bottom(),
        ] {
            let pixels = edge * 1.5;
            assert!((pixels - pixels.round()).abs() < 1e-3, "{edge}");
        }
    }

    #[test]
    fn cancel_the_close_button_and_outside_presses_are_reported() {
        let mut harness = Harness::new();
        let draw = |context: &mut UiContext| modal(context, "media", (400.0, 300.0), "Media");
        harness.input.cancel = true;
        assert!(harness.frame(draw).closed);
        harness.press(10.0, 10.0);
        let response = harness.frame(draw);
        assert!(response.outside_press && !response.closed);
        harness.release();
        harness.frame(draw);
        harness.press(600.0 - 12.0 - 16.0, 150.0 + 26.0);
        harness.frame(draw);
        harness.release();
        assert!(harness.frame(draw).closed);
    }
}
