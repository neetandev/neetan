//! The immediate mode context: the input of a frame, the hot, active and
//! focused widget and the drawing helpers over the scene in logical units.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use super::{
    focus::{Focusable, Navigation, NavigationSet, next_focus},
    icons::Icon,
    label::{Align, Truncation},
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

/// The keyboard modifiers of a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// Either control key is held.
    pub control: bool,
    /// Either shift key is held.
    pub shift: bool,
    /// Either alt key is held.
    pub alt: bool,
}

/// The input of one pass of the user interface.
///
/// The caller runs one pass for each discrete event: a press or a release of
/// a mouse button, a navigation event, a text commit, a backspace, confirm
/// or cancel. A pass holds at most one of them. Pointer motion and wheel
/// steps add up until the next pass. The caller shows the scene of the last
/// pass and calls [`Input::advance`] after each pass.
///
/// The pointer position is in physical pixels: the window coordinates of the
/// platform times the pixel density of the window, as
/// `SDL_GetWindowPixelDensity` reports it. The context divides it by the
/// scale factor of the viewport.
#[derive(Clone, Debug, Default)]
pub struct Input {
    /// The pointer x in physical pixels.
    pub mouse_x: f32,
    /// The pointer y in physical pixels.
    pub mouse_y: f32,
    /// The left button is held in this pass.
    pub left: bool,
    /// The right button is held in this pass.
    pub right: bool,
    /// The middle button is held in this pass.
    pub middle: bool,
    /// The left button was held in the last pass.
    pub left_previous: bool,
    /// The right button was held in the last pass.
    pub right_previous: bool,
    /// The middle button was held in the last pass.
    pub middle_previous: bool,
    /// The vertical wheel steps since the last pass. Positive values scroll
    /// up.
    pub wheel: f32,
    /// The press of the left button in this pass is the second press of a
    /// double click.
    pub left_double: bool,
    /// The text that the platform committed in this pass.
    pub text: String,
    /// The text that an input method composes and has not committed. The
    /// caller replaces it on every change of the composition and empties it
    /// with the commit. It stays from pass to pass.
    pub composition: String,
    /// The backspace key was pressed.
    pub backspace: bool,
    /// A confirm key or the confirm button of a gamepad was pressed.
    pub confirm: bool,
    /// The escape key or the cancel button of a gamepad was pressed.
    pub cancel: bool,
    /// The navigation event of this pass.
    pub navigation: Option<Navigation>,
    /// The keyboard modifiers.
    pub modifiers: Modifiers,
}

impl Input {
    /// Whether the left button went down in this pass.
    pub fn left_pressed(&self) -> bool {
        self.left && !self.left_previous
    }

    /// Whether the left button went up in this pass.
    pub fn left_released(&self) -> bool {
        !self.left && self.left_previous
    }

    /// Whether the right button went down in this pass.
    pub fn right_pressed(&self) -> bool {
        self.right && !self.right_previous
    }

    /// Whether the pass holds a key or gamepad event: a navigation event,
    /// confirm, cancel, a text commit or a backspace.
    pub fn has_key_event(&self) -> bool {
        self.navigation.is_some()
            || self.confirm
            || self.cancel
            || !self.text.is_empty()
            || self.backspace
    }

    /// Carries the button state of this pass into the fields of the last
    /// pass and clears the events of this pass. The composition stays.
    pub fn advance(&mut self) {
        self.left_previous = self.left;
        self.right_previous = self.right;
        self.middle_previous = self.middle;
        self.wheel = 0.0;
        self.left_double = false;
        self.text.clear();
        self.backspace = false;
        self.confirm = false;
        self.cancel = false;
        self.navigation = None;
    }
}

/// The target of a pass: its size in physical pixels and the physical
/// pixels for each logical unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// The width in physical pixels.
    pub width: u32,
    /// The height in physical pixels.
    pub height: u32,
    /// The physical pixels for each logical unit.
    pub scale: f32,
}

impl Viewport {
    /// A target of `width` x `height` physical pixels at `scale`. A scale
    /// that is not a positive finite number counts as 1.
    pub fn new(width: u32, height: u32, scale: f32) -> Self {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        Self {
            width,
            height,
            scale,
        }
    }
}

/// The drag of the active widget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Drag {
    /// The distance from the grabbed edge to the pointer along the drag.
    pub(crate) offset: f32,
}

/// The widget state that outlives a pass.
///
/// The hot widget under the pointer follows from each pass. The pressed
/// widget, the focus and the focus ring persist, so a press in one pass and
/// the release in a later pass reach the same widget and a navigation event
/// finds the widgets of the last pass. The caller keeps one state between
/// passes and lends it to each [`UiContext`].
#[derive(Debug)]
pub struct UiState {
    /// The number of the window of the state, unique in the process. The
    /// fonts that windows share tell their frames apart by it.
    window: u64,
    /// The widget the left button pressed.
    active: Option<UiId>,
    /// Whether the active widget ran its pointer behavior in this pass.
    active_seen: bool,
    /// The widget with the keyboard focus.
    focus: Option<UiId>,
    /// Whether the focus indicator shows. A key event shows it and a press
    /// of the mouse hides it.
    focus_visible: bool,
    /// The focusable widgets of this pass in drawing order.
    ring: Vec<Focusable>,
    /// The focusable widgets of the last pass in drawing order.
    previous_ring: Vec<Focusable>,
    /// The navigation event of this pass that the focused widget handles.
    routed: Option<(UiId, Navigation)>,
    /// The focused widget that takes the cancel of this pass.
    cancel_taker: Option<UiId>,
    /// The last area of a focused widget that left the ring. The next
    /// directional move starts from it.
    lost: Option<Rect>,
    /// The area of the focused text widget in physical pixels.
    text_area: Option<Rect>,
    /// The offset of the text cursor from the left edge of the text area in
    /// physical pixels.
    text_caret: Option<f32>,
    /// The drag of the active widget.
    drag: Option<Drag>,
}

impl Default for UiState {
    fn default() -> Self {
        static WINDOWS: AtomicU64 = AtomicU64::new(0);
        Self {
            window: WINDOWS.fetch_add(1, Ordering::Relaxed),
            active: None,
            active_seen: false,
            focus: None,
            focus_visible: true,
            ring: Vec::new(),
            previous_ring: Vec::new(),
            routed: None,
            cancel_taker: None,
            lost: None,
            text_area: None,
            text_caret: None,
            drag: None,
        }
    }
}

impl UiState {
    /// The widget with the keyboard focus.
    pub fn focused(&self) -> Option<UiId> {
        self.focus
    }

    /// Whether a widget holds the keyboard focus.
    pub fn has_keyboard_focus(&self) -> bool {
        self.focus.is_some()
    }

    /// Gives `id` the keyboard focus before a pass, for the widget a dialog
    /// opens with. The focus stays when the pass draws the widget.
    pub fn set_focus(&mut self, id: UiId) {
        self.focus = Some(id);
    }

    /// Whether the focused widget of the last pass takes typed text. The
    /// platform layer accepts text input while it does.
    pub fn wants_text_input(&self) -> bool {
        let Some(focus) = self.focus else {
            return false;
        };
        self.ring
            .iter()
            .any(|entry| entry.id == focus && entry.text)
    }

    /// The area of the focused text widget of the last pass in physical
    /// pixels, for the candidate window of an input method
    /// (`SDL_SetTextInputArea`).
    pub fn text_input_area(&self) -> Option<Rect> {
        self.text_area
    }

    /// The offset of the text cursor of the focused text widget of the last
    /// pass from the left edge of [`UiState::text_input_area`], in physical
    /// pixels.
    pub fn text_input_caret(&self) -> Option<f32> {
        self.text_caret
    }

    /// Whether the focus indicator shows.
    pub fn focus_visible(&self) -> bool {
        self.focus_visible
    }

    /// Drops the focus, so the next navigation event starts at the first
    /// widget.
    pub fn clear_focus(&mut self) {
        self.focus = None;
        self.lost = None;
    }

    /// Forgets the focus, the pressed widget, the drag and the focus ring,
    /// for a dialog that opens.
    pub fn reset(&mut self) {
        self.active = None;
        self.active_seen = false;
        self.focus = None;
        self.focus_visible = true;
        self.ring.clear();
        self.previous_ring.clear();
        self.routed = None;
        self.cancel_taker = None;
        self.lost = None;
        self.text_area = None;
        self.text_caret = None;
        self.drag = None;
    }

    /// Starts a pass: the ring of the last pass resolves the navigation
    /// event of `input` into a focus change or an event for the focused
    /// widget, and decides who takes cancel.
    fn begin_frame(&mut self, input: &Input) {
        std::mem::swap(&mut self.ring, &mut self.previous_ring);
        self.ring.clear();
        self.routed = None;
        self.cancel_taker = None;
        self.text_area = None;
        self.text_caret = None;
        self.active_seen = false;
        let lost = self.lost.take();
        if input.left_pressed() {
            self.focus_visible = false;
        }
        if input.navigation.is_some() || input.confirm {
            self.focus_visible = true;
        }
        let current = self.focus.and_then(|focus| {
            self.previous_ring
                .iter()
                .position(|entry| entry.id == focus)
        });
        if input.cancel
            && let Some(index) = current
            && self.previous_ring[index].takes_cancel
        {
            self.cancel_taker = Some(self.previous_ring[index].id);
        }
        let Some(navigation) = input.navigation else {
            return;
        };
        match current {
            Some(index) if self.previous_ring[index].captures.contains(navigation) => {
                self.routed = Some((self.previous_ring[index].id, navigation));
            }
            _ => {
                let from = current.map(|index| self.previous_ring[index].rect).or(lost);
                if let Some(next) = next_focus(&self.previous_ring, current, from, navigation) {
                    self.focus = Some(self.previous_ring[next].id);
                }
            }
        }
    }

    /// Closes a pass. A focus on a widget outside the ring of the pass is
    /// dropped, and its last area stays for the next directional move. A
    /// pressed widget that skipped its pointer behavior loses the press and
    /// its drag.
    fn end_frame(&mut self) {
        if let Some(focus) = self.focus
            && !self.ring.iter().any(|entry| entry.id == focus)
        {
            self.lost = self
                .previous_ring
                .iter()
                .find(|entry| entry.id == focus)
                .map(|entry| entry.rect);
            self.focus = None;
        }
        if !self.active_seen {
            self.active = None;
            self.drag = None;
        }
    }
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
    state: &'a mut UiState,
    /// The target of the pass.
    viewport: Viewport,
    /// The widget under the pointer.
    hot: Option<UiId>,
    /// Set once a widget takes the pointer, so the widgets below ignore it.
    pub mouse_captured: bool,
}

impl<'a> UiContext<'a> {
    /// Starts a pass: empties `scene` for the scale of `viewport` with a
    /// transparent background, starts the frame of the window of `state` in
    /// `fonts` and resolves the navigation event of `input`. The caller ends
    /// the pass with [`UiContext::finish`].
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
        Self {
            scene,
            fonts,
            theme,
            input,
            state,
            viewport,
            hot: None,
            mouse_captured: false,
        }
    }

    /// Ends the pass. A focus on a widget that the pass did not draw is
    /// dropped, and so is a press on a widget that the pass did not run.
    pub fn finish(self) {
        self.state.end_frame();
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

    /// Whether the pointer is over `rect` and no widget above took it.
    pub fn hovered(&self, rect: Rect) -> bool {
        let (x, y) = self.mouse();
        !self.mouse_captured && rect.contains(x, y)
    }

    /// Runs the hot and active tracking of a pressable widget `id` over
    /// `rect`. A press makes the widget active until the release, and a
    /// release over the widget completes a click. A press on a widget of the
    /// focus ring of this pass gives it the focus. A widget that skips this
    /// call in a pass loses its press.
    pub fn button_behavior(&mut self, id: UiId, rect: Rect) -> PointerResponse {
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
        self.register(Focusable {
            id,
            rect,
            captures,
            text: false,
            takes_cancel: false,
            preferred: false,
        })
    }

    /// Like [`UiContext::focusable`] for the main action of a panel. A
    /// directional move picks it over a neighbor at the same distance.
    pub fn focusable_preferred(&mut self, id: UiId, rect: Rect, captures: NavigationSet) -> bool {
        self.register(Focusable {
            id,
            rect,
            captures,
            text: false,
            takes_cancel: false,
            preferred: true,
        })
    }

    /// Adds `id` with the area `rect` to the focus ring of this pass as a
    /// widget that takes typed text, see [`UiState::wants_text_input`]. It
    /// captures left, right, home and end, and cancel when `takes_cancel` is
    /// set. Returns whether it holds the focus.
    pub fn text_focusable(&mut self, id: UiId, rect: Rect, takes_cancel: bool) -> bool {
        let focused = self.register(Focusable {
            id,
            rect,
            captures: NavigationSet::of(&[
                Navigation::Left,
                Navigation::Right,
                Navigation::Home,
                Navigation::End,
            ]),
            text: true,
            takes_cancel,
            preferred: false,
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

    /// Adds `entry` to the focus ring of this pass and returns whether its
    /// widget holds the focus.
    fn register(&mut self, entry: Focusable) -> bool {
        let id = entry.id;
        self.state.ring.push(entry);
        self.has_focus(id)
    }

    /// The navigation event of this pass for `id`. A widget gets the event
    /// while it holds the focus and when it captured the event in the last
    /// pass.
    pub fn navigation(&self, id: UiId) -> Option<Navigation> {
        self.state
            .routed
            .filter(|&(target, _)| target == id)
            .map(|(_, navigation)| navigation)
    }

    /// Whether `id` holds the focus and confirm was pressed in this pass.
    pub fn confirmed(&self, id: UiId) -> bool {
        self.input.confirm && self.state.focus == Some(id)
    }

    /// Whether cancel was pressed in this pass and the focused widget left
    /// it to the panel and the caller.
    pub fn cancelled(&self) -> bool {
        self.input.cancel && self.state.cancel_taker.is_none()
    }

    /// Whether the focused widget `id` takes the cancel of this pass.
    pub fn cancel_taken(&self, id: UiId) -> bool {
        self.input.cancel && self.state.cancel_taker == Some(id)
    }

    /// Whether `id` holds the keyboard focus.
    pub fn has_focus(&self, id: UiId) -> bool {
        self.state.focus == Some(id)
    }

    /// Gives `id` the keyboard focus. The focus stays when the pass draws
    /// the widget.
    pub fn set_focus(&mut self, id: UiId) {
        self.state.focus = Some(id);
    }

    /// Gives `id` the keyboard focus when no widget holds it, for the widget
    /// a dialog starts with. The caller calls it before it draws the widget.
    pub fn default_focus(&mut self, id: UiId) {
        if self.state.focus.is_none() {
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

    /// The drag of the active widget.
    pub(crate) fn drag(&self) -> Option<Drag> {
        self.state.drag
    }

    /// Sets the drag of the active widget.
    pub(crate) fn set_drag(&mut self, drag: Option<Drag>) {
        self.state.drag = drag;
    }

    /// Stops every widget drawn from here on from seeing the pointer. A
    /// caller calls it before it draws the widgets underneath a modal, so
    /// they draw as usual and stay inert.
    pub fn block_input(&mut self) {
        self.mouse_captured = true;
    }

    /// Draws a layer above everything placed so far. The widgets of `draw`
    /// see the pointer, and the widgets drawn after the layer stay inert for
    /// the rest of the pass when they were inert before.
    pub fn layer<R>(&mut self, draw: impl FnOnce(&mut Self) -> R) -> R {
        let outer = self.mouse_captured;
        self.mouse_captured = false;
        let result = draw(self);
        self.mouse_captured = outer;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tests::Harness;

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
    fn advance_clears_the_events_of_a_pass() {
        let mut input = Input {
            left: true,
            wheel: 1.0,
            text: "a".into(),
            composition: "か".into(),
            backspace: true,
            confirm: true,
            navigation: Some(Navigation::Down),
            ..Input::default()
        };
        assert!(input.left_pressed());
        assert!(input.has_key_event());
        input.advance();
        assert!(input.left_previous);
        assert!(!input.left_pressed());
        assert!(!input.has_key_event());
        assert_eq!(input.wheel, 0.0);
        assert_eq!(input.composition, "か");
        input.left = false;
        assert!(input.left_released());
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
    fn a_captured_pointer_skips_the_widgets_below() {
        let mut harness = Harness::new();
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        harness.mouse(50.0, 50.0);
        let (first, second, layered) = harness.frame(|context| {
            let first = context.button_behavior(ui_id("top"), rect);
            let second = context.button_behavior(ui_id("below"), rect);
            let layered = context.layer(|context| context.hovered(rect));
            (first, second, layered)
        });
        assert!(first.hovered);
        assert!(!second.hovered);
        assert!(layered);
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
        harness.frame(|_| {});
        assert_eq!(harness.state.focused(), None);
        assert!(!harness.state.has_keyboard_focus());
        harness.frame(|context| context.default_focus(ui_id("ghost")));
        assert_eq!(harness.state.focused(), None);
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
}
