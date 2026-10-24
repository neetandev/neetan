//! The immediate mode context: the hot, active and focused widget of a pass
//! and the drawing helpers over the scene in logical units.

use std::sync::Arc;

use super::{
    focus::{Focusable, Navigation, NavigationSet, ScrollAnchor},
    icons::Icon,
    input::{ActionButton, Input, Shortcut, Viewport},
    label::{Align, Truncation},
    state::{Drag, UiState},
    theme::Theme,
};
use crate::{Color, FontId, Fonts, Rect, RectStyle, Run, Scene};

/// A stable widget identifier derived from a label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UiId(pub u64);

impl UiId {
    /// Combines the id with an index for repeated rows such as list items.
    /// The rows of an indexed id differ from the id itself.
    pub fn indexed(self, index: usize) -> UiId {
        UiId(mix(
            self.0 ^ (index as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ))
    }

    /// The id of the part `label` of the widget, such as its scroll bar.
    pub fn child(self, label: &str) -> UiId {
        UiId(mix(self.0.rotate_left(32) ^ ui_id(label).0))
    }
}

impl From<&str> for UiId {
    /// The id of the label `label`, like [`ui_id`].
    fn from(label: &str) -> Self {
        ui_id(label)
    }
}

/// The finalizer of SplitMix64, which spreads every input bit over the
/// output.
fn mix(value: u64) -> u64 {
    let mut hash = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    hash = (hash ^ (hash >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    hash ^ (hash >> 31)
}

/// Computes a widget id from a label with an FNV-1a hash.
pub fn ui_id(label: &str) -> UiId {
    let mut hash: u64 = 0xCBF2_9CE4_8422_2325;
    for byte in label.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    UiId(hash)
}

/// Combines a label and an index for repeated rows such as list items.
pub fn ui_id_indexed(label: &str, index: usize) -> UiId {
    ui_id(label).indexed(index)
}

/// The font, the size and the color of a text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    /// The font family.
    pub font: FontId,
    /// The size in logical units.
    pub size: f32,
    /// The color.
    pub color: Color,
}

impl TextStyle {
    /// The same style in another color.
    pub fn with_color(self, color: Color) -> Self {
        Self { color, ..self }
    }

    /// The same style at another size.
    pub fn with_size(self, size: f32) -> Self {
        Self { size, ..self }
    }
}

/// What the pointer did with a widget in a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PointerResponse {
    /// The pointer is over the widget and no widget above took it.
    pub hovered: bool,
    /// The left button went down over the widget in this pass.
    pub pressed: bool,
    /// The left button went up over the widget after a press on it.
    pub clicked: bool,
    /// A press on the widget holds the left button.
    pub held: bool,
}

/// The immediate mode context of one pass.
///
/// The pass draws in layers. The base is layer 0. A modal draws its panel
/// in [`UiContext::layer`], in the layer of its place in the stack of open
/// modals. A popup draws in [`UiContext::popup`], one layer above the
/// widget that draws it. The widgets below the top layer are blocked: they
/// draw as usual and ignore the pointer, the keys and the gamepad.
pub struct UiContext<'a> {
    /// The scene the pass draws into.
    pub scene: &'a mut Scene,
    /// The fonts of the process.
    pub fonts: &'a mut Fonts,
    /// The theme.
    pub theme: &'a Theme,
    /// The input of this pass.
    pub input: &'a Input,
    /// The state between passes.
    pub(super) state: &'a mut UiState,
    /// The target of the pass.
    viewport: Viewport,
    /// The widget under the pointer.
    hot: Option<UiId>,
    /// Set once a widget takes the pointer, so the widgets below ignore it.
    pub mouse_captured: bool,
    /// The layer the widgets draw in.
    pub(super) depth: u32,
    /// The widgets of the layers below this layer are blocked.
    pub(super) blocked: u32,
    /// Whether the layer the widgets draw in opened in this pass. A fresh
    /// layer takes no input of the pass.
    pub(super) fresh_layer: bool,
    /// The deepest layer of the pass so far.
    pub(super) deepest: u32,
    /// The innermost scroll area around the widgets that draw.
    pub(super) scroll: Option<ScrollAnchor>,
}

impl<'a> UiContext<'a> {
    /// Starts a pass: empties `scene` for the scale of `viewport` with a
    /// transparent background, starts the frame of the window of `state` in
    /// `fonts` and resolves the navigation event of `input`. The top layer of
    /// the last pass blocks every layer below it from the first widget on.
    /// The caller ends the pass with [`UiContext::finish`].
    pub fn new(
        scene: &'a mut Scene,
        fonts: &'a mut Fonts,
        theme: &'a Theme,
        input: &'a Input,
        state: &'a mut UiState,
        viewport: Viewport,
    ) -> Self {
        let viewport = Viewport::new(viewport.width, viewport.height, viewport.scale);
        scene.begin(viewport.scale, Color::TRANSPARENT);
        fonts.begin_frame(state.window);
        state.begin_frame(input);
        let blocked = state.top_layer();
        Self {
            scene,
            fonts,
            theme,
            input,
            state,
            viewport,
            hot: None,
            mouse_captured: false,
            depth: 0,
            fresh_layer: false,
            blocked,
            deepest: 0,
            scroll: None,
        }
    }

    /// Ends the pass. A focus on a widget that the pass did not draw is
    /// dropped, and so is a press on a widget that the pass did not run.
    /// The items of the popup layer follow every other item of the scene.
    pub fn finish(self) {
        self.state.end_frame(self.deepest);
        self.scene.finish();
    }

    /// The physical pixels for each logical unit.
    pub fn scale(&self) -> f32 {
        self.viewport.scale
    }

    /// The whole target in logical units.
    pub fn bounds(&self) -> Rect {
        Rect::new(
            0.0,
            0.0,
            self.viewport.width as f32 / self.viewport.scale,
            self.viewport.height as f32 / self.viewport.scale,
        )
    }

    /// `value` in logical units rounded to whole physical pixels.
    pub fn snap(&self, value: f32) -> f32 {
        (value * self.viewport.scale).round() / self.viewport.scale
    }

    /// `rect` with its edges rounded to whole physical pixels.
    pub fn snap_rect(&self, rect: Rect) -> Rect {
        let left = self.snap(rect.x);
        let top = self.snap(rect.y);
        Rect::new(
            left,
            top,
            self.snap(rect.right()) - left,
            self.snap(rect.bottom()) - top,
        )
    }

    /// The pointer position in logical units.
    pub fn mouse(&self) -> (f32, f32) {
        (
            self.input.mouse_x / self.viewport.scale,
            self.input.mouse_y / self.viewport.scale,
        )
    }

    /// Fills a rectangle with a solid color.
    pub fn fill(&mut self, rect: Rect, color: Color) {
        self.scene.fill(rect, 0.0, color);
    }

    /// Draws a rectangle with the given style.
    pub fn rect(&mut self, rect: Rect, style: &RectStyle) {
        self.scene.rect(rect, style);
    }

    /// The advance width of `text` in `style`.
    pub fn measure(&mut self, style: &TextStyle, text: &str) -> f32 {
        self.fonts.measure(style.font, style.size, text)
    }

    /// The baseline that centers the capital letters of `style` in `rect`.
    pub fn centered_baseline(&self, rect: Rect, style: &TextStyle) -> f32 {
        let cap_height = self.fonts.metrics(style.font).cap_height * style.size;
        self.snap(rect.y + (rect.height + cap_height) * 0.5)
    }

    /// Draws `text` with the start of its baseline at `(x, baseline)`.
    pub fn text(&mut self, x: f32, baseline: f32, style: &TextStyle, text: &str) {
        let run = self.fonts.shape(style.font, text);
        self.scene.run((x, baseline), style.size, &run, style.color);
    }

    /// Draws `text` in `rect`, centered vertically and placed by `align`. A
    /// text wider than `rect` is cut as `truncation` says. Returns whether
    /// the text was cut.
    pub fn text_in(
        &mut self,
        rect: Rect,
        style: &TextStyle,
        text: &str,
        align: Align,
        truncation: Truncation,
    ) -> bool {
        let width = rect.width.max(0.0);
        let run: Arc<Run> = match truncation {
            Truncation::End => self.fonts.fit(style.font, style.size, text, width),
            Truncation::Middle => self.fonts.fit_middle(style.font, style.size, text, width),
        };
        let advance = run.advance * style.size;
        let x = match align {
            Align::Start => rect.x,
            Align::Center => rect.x + (rect.width - advance) * 0.5,
            Align::End => rect.right() - advance,
        };
        let baseline = self.centered_baseline(rect, style);
        self.scene
            .run((self.snap(x), baseline), style.size, &run, style.color);
        run.truncated
    }

    /// Draws `text` left aligned in `rect` and centered vertically. A text
    /// wider than `rect` ends in an ellipsis.
    pub fn text_left(&mut self, rect: Rect, style: &TextStyle, text: &str) {
        self.text_in(rect, style, text, Align::Start, Truncation::End);
    }

    /// Draws `text` centered in `rect`. A text wider than `rect` ends in an
    /// ellipsis.
    pub fn text_centered(&mut self, rect: Rect, style: &TextStyle, text: &str) {
        self.text_in(rect, style, text, Align::Center, Truncation::End);
    }

    /// Draws `text` right aligned in `rect` and centered vertically. A text
    /// wider than `rect` ends in an ellipsis.
    pub fn text_right(&mut self, rect: Rect, style: &TextStyle, text: &str) {
        self.text_in(rect, style, text, Align::End, Truncation::End);
    }

    /// Draws `icon` centered in `rect` in a box of `size` logical units. The
    /// box snaps to whole physical pixels.
    pub fn icon(&mut self, rect: Rect, icon: Icon, size: f32, color: Color) {
        let (x, y) = rect.center();
        let size = self.snap(size);
        let origin = (self.snap(x - size * 0.5), self.snap(y - size * 0.5));
        self.theme.icons.draw(self.scene, origin, size, icon, color);
    }

    /// Whether the widgets drawn at this point are blocked by a layer above
    /// them. A blocked widget draws as usual and ignores the pointer, the
    /// keys and the gamepad.
    pub fn is_blocked(&self) -> bool {
        self.depth < self.blocked
    }

    /// Whether the widgets drawn at this point take the input of the pass:
    /// they are not blocked, and their layer was open when the pass started.
    /// A modal or a popup that opens during a pass takes the input of the
    /// next pass on.
    pub(super) fn takes_input(&self) -> bool {
        !self.is_blocked() && !self.fresh_layer
    }

    /// Whether the pointer is over `rect`, no widget above took it and the
    /// widgets at this point are not blocked.
    pub fn hovered(&self, rect: Rect) -> bool {
        let (x, y) = self.mouse();
        !self.mouse_captured && self.takes_input() && rect.contains(x, y)
    }

    /// Runs the hot and active tracking of a pressable widget `id` over
    /// `rect`. A press makes the widget active until the release, and a
    /// release over the widget completes a click. A press on a widget of the
    /// focus ring of this pass gives it the focus. A widget that skips this
    /// call in a pass loses its press, and so does a blocked widget.
    pub fn button_behavior(&mut self, id: UiId, rect: Rect) -> PointerResponse {
        if !self.takes_input() {
            return PointerResponse::default();
        }
        let hovered = self.hovered(rect);
        let mut response = PointerResponse {
            hovered,
            ..PointerResponse::default()
        };
        if hovered {
            self.hot = Some(id);
            self.mouse_captured = true;
        }
        if self.state.active == Some(id) {
            self.state.active_seen = true;
            if self.input.left {
                response.held = true;
            } else {
                response.clicked = hovered && self.input.left_released();
                self.state.active = None;
            }
        } else if hovered && self.input.left_pressed() {
            self.state.active = Some(id);
            self.state.active_seen = true;
            response.pressed = true;
            response.held = true;
            if self.state.ring.iter().any(|entry| entry.id == id) {
                self.set_focus(id);
            }
        }
        response
    }

    /// Whether `id` is the hot widget under the pointer.
    pub fn is_hot(&self, id: UiId) -> bool {
        self.hot == Some(id)
    }

    /// Marks `id` as the hot widget.
    pub fn set_hot(&mut self, id: UiId) {
        self.hot = Some(id);
    }

    /// Whether `id` is the active widget that the left button pressed.
    pub fn is_active(&self, id: UiId) -> bool {
        self.state.active == Some(id)
    }

    /// Whether any widget is active.
    pub fn any_active(&self) -> bool {
        self.state.active.is_some()
    }

    /// Marks `id` as the active widget and takes the pointer.
    pub fn set_active(&mut self, id: UiId) {
        self.state.active = Some(id);
        self.state.active_seen = true;
        self.mouse_captured = true;
    }

    /// Clears `id` as the active widget if it is active.
    pub fn clear_active(&mut self, id: UiId) {
        if self.state.active == Some(id) {
            self.state.active = None;
        }
    }

    /// Adds `id` with the area `rect` to the focus ring of this pass and
    /// returns whether it holds the focus. While it holds the focus, the
    /// navigation events in `captures` reach the widget through
    /// [`UiContext::navigation`] and leave the focus where it is.
    pub fn focusable(&mut self, id: UiId, rect: Rect, captures: NavigationSet) -> bool {
        self.register(Focusable::new(id, rect, captures))
    }

    /// Like [`UiContext::focusable`] for the main action of a panel. A
    /// directional move picks it over a neighbor at the same distance, and a
    /// modal that opens gives it the focus.
    pub fn focusable_preferred(&mut self, id: UiId, rect: Rect, captures: NavigationSet) -> bool {
        self.register(Focusable {
            preferred: true,
            ..Focusable::new(id, rect, captures)
        })
    }

    /// Like [`UiContext::focusable`] for a widget that a modal that opens
    /// never gives the focus, such as the close button in its header.
    pub fn focusable_secondary(&mut self, id: UiId, rect: Rect, captures: NavigationSet) -> bool {
        self.register(Focusable {
            default_choice: false,
            ..Focusable::new(id, rect, captures)
        })
    }

    /// Adds `id` with the area `rect` to the focus ring of this pass as a
    /// widget that takes typed text, see [`UiState::wants_text_input`]. It
    /// captures left, right, home and end, and cancel when `takes_cancel` is
    /// set. Returns whether it holds the focus.
    pub fn text_focusable(&mut self, id: UiId, rect: Rect, takes_cancel: bool) -> bool {
        let captures = NavigationSet::of(&[
            Navigation::Left,
            Navigation::Right,
            Navigation::Home,
            Navigation::End,
        ]);
        let focused = self.register(Focusable {
            text: true,
            takes_cancel,
            ..Focusable::new(id, rect, captures)
        });
        if focused {
            let scale = self.viewport.scale;
            self.state.text_area = Some(Rect::new(
                rect.x * scale,
                rect.y * scale,
                rect.width * scale,
                rect.height * scale,
            ));
        }
        focused
    }

    /// Records the text cursor of the focused text widget at `x` in logical
    /// units, for the candidate window of an input method.
    pub fn set_text_caret(&mut self, x: f32) {
        if let Some(area) = self.state.text_area {
            self.state.text_caret = Some((x * self.viewport.scale - area.x).max(0.0));
        }
    }

    /// Adds `entry` to the focus ring of this pass in the current layer and
    /// the current scroll area, and returns whether its widget holds the
    /// focus.
    fn register(&mut self, entry: Focusable) -> bool {
        let id = entry.id;
        self.state.ring.push(Focusable {
            layer: self.depth,
            scroll: self.scroll,
            ..entry
        });
        self.has_focus(id)
    }

    /// The navigation event of this pass for `id`. A widget gets the event
    /// while it holds the focus and when it captured the event in the last
    /// pass.
    pub fn navigation(&self, id: UiId) -> Option<Navigation> {
        if self.is_blocked() {
            return None;
        }
        self.state
            .routed
            .filter(|&(target, _)| target == id)
            .map(|(_, navigation)| navigation)
    }

    /// The shoulder button of this pass, [`Navigation::PreviousTab`] or
    /// [`Navigation::NextTab`], for a tab bar that switches its tab whatever
    /// widget holds the focus. A blocked tab bar gets none, and so does a
    /// tab bar in a layer that opened in this pass.
    pub fn tab_navigation(&self) -> Option<Navigation> {
        if !self.takes_input() {
            return None;
        }
        self.input.navigation.filter(|navigation| {
            matches!(navigation, Navigation::PreviousTab | Navigation::NextTab)
        })
    }

    /// The action button of this pass, unless the widgets at this point are
    /// blocked or their layer opened in this pass.
    pub fn action_button(&self) -> Option<ActionButton> {
        if !self.takes_input() {
            return None;
        }
        self.input.action
    }

    /// The shortcut of this pass, unless the widgets at this point are
    /// blocked or their layer opened in this pass. A shortcut without the
    /// control key is inactive while a text widget holds the focus.
    /// Backspace then edits the text.
    pub fn shortcut(&self) -> Option<Shortcut> {
        if !self.takes_input() {
            return None;
        }
        let shortcut = self
            .input
            .shortcut
            .or(self.input.backspace.then_some(Shortcut::Backspace))?;
        if !shortcut.has_modifier() && self.text_widget_focused() {
            return None;
        }
        Some(shortcut)
    }

    /// Whether the focused widget takes typed text, in the ring of this
    /// pass or else of the last pass.
    fn text_widget_focused(&self) -> bool {
        let Some(focus) = self.state.focus else {
            return false;
        };
        self.state
            .ring
            .iter()
            .chain(&self.state.previous_ring)
            .find(|entry| entry.id == focus)
            .is_some_and(|entry| entry.text)
    }

    /// Whether `id` holds the focus and confirm was pressed in this pass
    /// while `id` held the focus at the start of the pass. A widget that
    /// takes the focus during a pass never gets the confirm of that pass.
    pub fn confirmed(&self, id: UiId) -> bool {
        self.input.confirm && self.state.confirm_target == Some(id) && self.has_focus(id)
    }

    /// Whether cancel was pressed in this pass and the focused widget left
    /// it to the panel and the caller. A blocked panel gets no cancel, and
    /// neither does a panel that opened in this pass.
    pub fn cancelled(&self) -> bool {
        self.input.cancel && self.state.cancel_taker.is_none() && self.takes_input()
    }

    /// Whether the focused widget `id` takes the cancel of this pass.
    pub fn cancel_taken(&self, id: UiId) -> bool {
        self.input.cancel && self.state.cancel_taker == Some(id) && self.takes_input()
    }

    /// The widget that holds the keyboard focus, in any layer.
    pub fn focused(&self) -> Option<UiId> {
        self.state.focus
    }

    /// Whether `id` holds the keyboard focus and the widgets at this point
    /// are not blocked.
    pub fn has_focus(&self, id: UiId) -> bool {
        self.state.focus == Some(id) && !self.is_blocked()
    }

    /// Whether `id` draws its focus look: it holds the focus and is not
    /// blocked, or it is blocked and takes the focus back when the modal
    /// directly above its layer closes.
    pub fn shows_focus(&self, id: UiId) -> bool {
        if !self.is_blocked() {
            return self.has_focus(id);
        }
        self.state
            .modals
            .get(self.depth as usize)
            .is_some_and(|modal| modal.return_focus == Some(id))
    }

    /// Gives `id` the keyboard focus. The focus stays when the pass draws
    /// the widget. A change of the focus asks for another pass, so the
    /// widgets drawn before the change show their new look.
    pub fn set_focus(&mut self, id: UiId) {
        if self.state.focus != Some(id) {
            self.state.focus = Some(id);
            self.request_pass();
        }
    }

    /// Gives `id` the keyboard focus when no widget of this layer or a layer
    /// above holds it, for the widget a dialog starts with. The caller calls
    /// it before it draws the widget.
    pub fn default_focus(&mut self, id: UiId) {
        if self.is_blocked() {
            return;
        }
        let take = match self.state.focus {
            None => true,
            Some(focus) => self
                .state
                .layer_of(focus)
                .is_some_and(|layer| layer < self.depth),
        };
        if take {
            self.set_focus(id);
        }
    }

    /// Drops the keyboard focus if `id` holds it.
    pub fn clear_focus(&mut self, id: UiId) {
        if self.state.focus == Some(id) {
            self.state.focus = None;
        }
    }

    /// Whether the focus indicator shows.
    pub fn focus_visible(&self) -> bool {
        self.state.focus_visible
    }

    /// Draws the focus indicator around `rect` with the corner radius
    /// `radius` when it shows.
    pub fn focus_ring(&mut self, rect: Rect, radius: f32) {
        if !self.state.focus_visible {
            return;
        }
        let theme = self.theme;
        let gap = theme.focus_gap;
        let width = theme.focus_width;
        self.scene.rect(
            rect.expand(gap + width),
            &RectStyle::filled(Color::TRANSPARENT, radius + gap + width)
                .with_stroke(width, theme.focus),
        );
    }

    /// Asks the caller for another pass, for a widget that changed its look
    /// after it drew, such as a scroll area that scrolls a widget into view.
    /// See [`UiState::take_pass_request`].
    pub fn request_pass(&mut self) {
        self.state.pass_requested = true;
    }

    /// The drag of the active widget.
    pub(crate) fn drag(&self) -> Option<Drag> {
        self.state.drag
    }

    /// Sets the drag of the active widget.
    pub(crate) fn set_drag(&mut self, drag: Option<Drag>) {
        self.state.drag = drag;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Viewport, tests::Harness};

    #[test]
    fn ids_of_rows_and_parts_differ() {
        let id = ui_id("list");
        assert_eq!(id, UiId::from("list"));
        assert_ne!(id.indexed(0), id);
        assert_ne!(id.indexed(0), id.indexed(1));
        assert_ne!(id.child("bar"), id);
        assert_ne!(id.child("bar"), id.child("thumb"));
        assert_eq!(ui_id_indexed("list", 3), id.indexed(3));
    }

    #[test]
    fn a_viewport_divides_the_pointer_by_its_scale() {
        let mut harness = Harness::with_viewport(Viewport::new(400, 300, 2.0));
        harness.mouse(100.0, 60.0);
        let (mouse, bounds, snapped) = harness.frame(|context| {
            (
                context.mouse(),
                context.bounds(),
                context.snap_rect(Rect::new(0.3, 0.6, 10.1, 5.0)),
            )
        });
        assert_eq!(mouse, (50.0, 30.0));
        assert_eq!(bounds, Rect::new(0.0, 0.0, 200.0, 150.0));
        assert_eq!(snapped, Rect::new(0.5, 0.5, 10.0, 5.0));
        assert_eq!(harness.scene.scale(), 2.0);
        assert_eq!(Viewport::new(1, 1, f32::NAN).scale, 1.0);
    }

    #[test]
    fn a_click_completes_on_the_release_over_the_widget() {
        let mut harness = Harness::new();
        let id = ui_id("button");
        let rect = Rect::new(10.0, 10.0, 50.0, 20.0);
        harness.press(20.0, 20.0);
        let pressed = harness.frame(|context| context.button_behavior(id, rect));
        assert!(pressed.pressed && pressed.held && !pressed.clicked);
        harness.release();
        let released = harness.frame(|context| context.button_behavior(id, rect));
        assert!(released.clicked && !released.held);

        harness.press(20.0, 20.0);
        harness.frame(|context| context.button_behavior(id, rect));
        harness.mouse(100.0, 100.0);
        harness.release();
        let outside = harness.frame(|context| context.button_behavior(id, rect));
        assert!(!outside.clicked);
        assert!(!harness.frame(|context| context.any_active()));
    }

    #[test]
    fn a_press_on_a_widget_that_leaves_never_clicks() {
        let mut harness = Harness::new();
        let id = ui_id("insert");
        let rect = Rect::new(10.0, 10.0, 80.0, 30.0);
        harness.press(20.0, 20.0);
        harness.frame(|context| context.button_behavior(id, rect));
        assert!(harness.frame(|context| context.is_active(id)));
        harness.frame(|_| {});
        harness.release();
        harness.frame(|_| {});
        let response = harness.frame(|context| {
            assert!(!context.any_active());
            context.button_behavior(id, rect)
        });
        assert!(!response.clicked);
    }

    #[test]
    fn a_focus_on_a_widget_that_is_not_drawn_is_dropped() {
        let mut harness = Harness::new();
        let id = ui_id("field");
        harness.frame(|context| {
            context.set_focus(id);
            context.focusable(id, Rect::new(0.0, 0.0, 10.0, 10.0), NavigationSet::EMPTY);
        });
        assert_eq!(harness.state.focused(), Some(id));
        harness.state.take_pass_request();
        harness.frame(|_| {});
        assert_eq!(harness.state.focused(), None);
        assert!(!harness.state.has_keyboard_focus());
        assert!(harness.state.take_pass_request());
        harness.frame(|context| context.default_focus(ui_id("ghost")));
        assert_eq!(harness.state.focused(), None);
    }

    /// Verifies that a focus change asks for another pass and that the same focus does not.
    #[test]
    fn a_focus_change_asks_for_a_pass() {
        let mut harness = Harness::new();
        let id = ui_id("field");
        let draw = |context: &mut UiContext| {
            context.set_focus(id);
            context.focusable(id, Rect::new(0.0, 0.0, 10.0, 10.0), NavigationSet::EMPTY);
        };
        harness.frame(draw);
        assert!(harness.state.take_pass_request());
        harness.frame(draw);
        assert!(!harness.state.take_pass_request());
    }

    /// Verifies that confirm reaches only the widget that held the focus at the start of the pass.
    #[test]
    fn a_widget_that_takes_the_focus_in_a_pass_never_gets_its_confirm() {
        let mut harness = Harness::new();
        let first = ui_id("first");
        let second = ui_id("second");
        let draw = |context: &mut UiContext, hand_over: bool| {
            context.focusable(first, Rect::new(0.0, 0.0, 10.0, 10.0), NavigationSet::EMPTY);
            let first_confirmed = context.confirmed(first);
            if hand_over && first_confirmed {
                context.set_focus(second);
            }
            context.focusable(
                second,
                Rect::new(0.0, 20.0, 10.0, 10.0),
                NavigationSet::EMPTY,
            );
            (first_confirmed, context.confirmed(second))
        };
        harness.state.set_focus(first);
        harness.frame(|context| draw(context, true));
        harness.input.confirm = true;
        assert_eq!(harness.frame(|context| draw(context, true)), (true, false));
        assert_eq!(harness.state.focused(), Some(second));
        harness.input.confirm = true;
        assert_eq!(harness.frame(|context| draw(context, true)), (false, true));
    }

    #[test]
    fn a_move_after_the_focused_widget_left_starts_from_its_area() {
        let mut harness = Harness::new();
        let rects = [
            Rect::new(0.0, 0.0, 100.0, 20.0),
            Rect::new(0.0, 30.0, 100.0, 20.0),
            Rect::new(0.0, 60.0, 100.0, 20.0),
        ];
        let draw = |context: &mut UiContext, shown: [bool; 3]| {
            for (index, rect) in rects.iter().enumerate() {
                if shown[index] {
                    context.focusable(ui_id_indexed("row", index), *rect, NavigationSet::EMPTY);
                }
            }
        };
        harness.state.set_focus(ui_id_indexed("row", 1));
        harness.frame(|context| draw(context, [true; 3]));
        assert_eq!(harness.state.focused(), Some(ui_id_indexed("row", 1)));
        harness.frame(|context| draw(context, [true, false, true]));
        assert_eq!(harness.state.focused(), None);
        harness.navigate(Navigation::Down);
        harness.frame(|context| draw(context, [true, false, true]));
        assert_eq!(harness.state.focused(), Some(ui_id_indexed("row", 2)));
    }

    #[test]
    fn reset_forgets_the_focus_and_the_press() {
        let mut harness = Harness::new();
        let id = ui_id("button");
        let rect = Rect::new(0.0, 0.0, 100.0, 20.0);
        harness.press(10.0, 10.0);
        harness.frame(|context| {
            context.focusable(id, rect, NavigationSet::EMPTY);
            context.button_behavior(id, rect);
        });
        assert_eq!(harness.state.focused(), Some(id));
        harness.state.reset();
        assert_eq!(harness.state.focused(), None);
        assert!(harness.state.focus_visible());
        assert!(!harness.frame(|context| context.any_active()));
    }

    #[test]
    fn a_text_widget_asks_for_text_input() {
        let mut harness = Harness::new();
        let field = ui_id("field");
        let button = ui_id("button");
        let draw = |context: &mut UiContext| {
            context.default_focus(field);
            context.text_focusable(field, Rect::new(0.0, 0.0, 100.0, 20.0), false);
            context.focusable(
                button,
                Rect::new(0.0, 30.0, 100.0, 20.0),
                NavigationSet::EMPTY,
            );
        };
        harness.frame(draw);
        assert!(harness.state.wants_text_input());
        assert_eq!(
            harness.state.text_input_area(),
            Some(Rect::new(0.0, 0.0, 100.0, 20.0))
        );
        harness.navigate(Navigation::Right);
        harness.frame(draw);
        assert_eq!(harness.state.focused(), Some(field));
        harness.navigate(Navigation::Next);
        harness.frame(draw);
        assert_eq!(harness.state.focused(), Some(button));
        assert!(!harness.state.wants_text_input());
        assert_eq!(harness.state.text_input_area(), None);
    }

    #[test]
    fn the_text_caret_is_relative_to_the_text_area() {
        let mut harness = Harness::with_viewport(Viewport::new(800, 600, 2.0));
        let field = ui_id("field");
        let draw = |context: &mut UiContext, focused: bool| {
            if focused {
                context.set_focus(field);
            } else {
                context.clear_focus(field);
            }
            context.text_focusable(field, Rect::new(10.0, 5.0, 100.0, 20.0), false);
            context.set_text_caret(40.0);
        };
        harness.frame(|context| draw(context, true));
        assert_eq!(harness.state.text_input_caret(), Some(60.0));
        harness.frame(|context| draw(context, false));
        assert_eq!(harness.state.text_input_caret(), None);
    }

    #[test]
    fn a_widget_that_takes_cancel_keeps_it_from_the_caller() {
        let mut harness = Harness::new();
        let field = ui_id("field");
        let draw = |context: &mut UiContext, takes: bool| {
            context.text_focusable(field, Rect::new(0.0, 0.0, 100.0, 20.0), takes);
            (context.cancelled(), context.cancel_taken(field))
        };
        harness.state.set_focus(field);
        harness.frame(|context| draw(context, true));
        harness.input.cancel = true;
        assert_eq!(harness.frame(|context| draw(context, false)), (false, true));
        harness.input.cancel = true;
        assert_eq!(harness.frame(|context| draw(context, false)), (true, false));
    }

    #[test]
    fn navigation_moves_the_focus_and_reaches_capturing_widgets() {
        let mut harness = Harness::new();
        let top = ui_id("top");
        let list = ui_id("list");
        let captures = NavigationSet::of(&[Navigation::Down, Navigation::Home]);
        let draw = |context: &mut UiContext| {
            context.focusable(top, Rect::new(0.0, 0.0, 100.0, 20.0), NavigationSet::EMPTY);
            context.focusable(list, Rect::new(0.0, 30.0, 100.0, 100.0), captures);
            (
                context.confirmed(top),
                context.confirmed(list),
                context.navigation(list),
            )
        };
        harness.frame(draw);
        assert_eq!(harness.state.focused(), None);

        harness.navigate(Navigation::Down);
        harness.frame(draw);
        assert_eq!(harness.state.focused(), Some(top));

        harness.navigate(Navigation::Down);
        harness.frame(draw);
        assert_eq!(harness.state.focused(), Some(list));
        harness.navigate(Navigation::Down);
        let (_, _, event) = harness.frame(draw);
        assert_eq!(event, Some(Navigation::Down));
        harness.navigate(Navigation::Up);
        let (_, _, event) = harness.frame(draw);
        assert_eq!(event, None);
        assert_eq!(harness.state.focused(), Some(top));
        harness.input.confirm = true;
        let (top_confirmed, list_confirmed, _) = harness.frame(draw);
        assert!(top_confirmed);
        assert!(!list_confirmed);
    }

    #[test]
    fn hover_leaves_the_focus_and_a_click_takes_it() {
        let mut harness = Harness::new();
        let first = ui_id("first");
        let second = ui_id("second");
        let second_rect = Rect::new(0.0, 30.0, 100.0, 20.0);
        let draw = |context: &mut UiContext| {
            context.default_focus(first);
            let rect = Rect::new(0.0, 0.0, 100.0, 20.0);
            context.focusable(first, rect, NavigationSet::EMPTY);
            context.button_behavior(first, rect);
            context.focusable(second, second_rect, NavigationSet::EMPTY);
            context.button_behavior(second, second_rect).hovered
        };
        harness.frame(draw);
        harness.mouse(50.0, 40.0);
        assert!(harness.frame(draw));
        assert_eq!(harness.state.focused(), Some(first));
        assert!(harness.state.focus_visible());

        harness.press(50.0, 40.0);
        harness.frame(draw);
        assert_eq!(harness.state.focused(), Some(second));
        assert!(!harness.state.focus_visible());
        harness.release();
        harness.navigate(Navigation::Up);
        harness.frame(draw);
        assert_eq!(harness.state.focused(), Some(first));
        assert!(harness.state.focus_visible());
    }

    #[test]
    fn a_shortcut_is_blocked_under_a_modal_and_held_back_by_a_text_field() {
        let mut harness = Harness::new();
        let field = ui_id("field");
        let draw = |context: &mut UiContext, open: bool| {
            let base = context.shortcut();
            context.text_focusable(field, Rect::new(0.0, 0.0, 100.0, 20.0), false);
            let top = if open {
                context.layer("dialog", |context| context.shortcut())
            } else {
                None
            };
            (base, top)
        };
        harness.input.shortcut = Some(crate::ui::Shortcut::Delete);
        assert_eq!(
            harness.frame(|context| draw(context, false)),
            (Some(crate::ui::Shortcut::Delete), None)
        );
        harness.frame(|context| draw(context, true));
        harness.input.shortcut = Some(crate::ui::Shortcut::ControlS);
        assert_eq!(
            harness.frame(|context| draw(context, true)),
            (None, Some(crate::ui::Shortcut::ControlS))
        );
        harness.frame(|context| draw(context, false));
        harness.state.set_focus(field);
        harness.frame(|context| draw(context, false));
        harness.input.backspace = true;
        assert_eq!(harness.frame(|context| draw(context, false)), (None, None));
        harness.input.shortcut = Some(crate::ui::Shortcut::ControlF);
        assert_eq!(
            harness.frame(|context| draw(context, false)),
            (Some(crate::ui::Shortcut::ControlF), None)
        );
        harness.state.clear_focus();
        harness.frame(|context| draw(context, false));
        harness.input.backspace = true;
        assert_eq!(
            harness.frame(|context| draw(context, false)),
            (Some(crate::ui::Shortcut::Backspace), None)
        );
    }

    /// Verifies that the base layer below a modal reads the focused widget of the modal while the modal blocks it.
    #[test]
    fn the_focused_widget_shows_through_a_modal() {
        let mut harness = Harness::new();
        let draw = |context: &mut UiContext| {
            let area = Rect::new(0.0, 0.0, 10.0, 10.0);
            context.focusable(UiId::from("base"), area, NavigationSet::EMPTY);
            context.layer("dialog", |context| {
                context.focusable(UiId::from("ok"), area, NavigationSet::EMPTY);
            });
            (context.focused(), context.has_focus(UiId::from("ok")))
        };
        harness.frame(draw);
        harness.frame(draw);
        let (focused, seen_from_below) = harness.frame(draw);
        assert_eq!(focused, Some(UiId::from("ok")));
        assert!(!seen_from_below);
    }
}
