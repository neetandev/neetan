//! The widget state that outlives a pass: the pressed widget, the focus and
//! the focus ring, the open popup and the requests for another pass.

use std::sync::atomic::{AtomicU64, Ordering};

use super::{
    context::UiId,
    focus::{Focusable, next_focus},
    input::Input,
    list::ListState,
};
use crate::Rect;

/// The drag of the active widget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Drag {
    /// The distance from the grabbed edge to the pointer along the drag.
    pub(crate) offset: f32,
}

/// The popup that is open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OpenPopup {
    /// The widget that opened the popup.
    pub(crate) owner: UiId,
    /// The layer of the popup.
    pub(crate) layer: u32,
    /// The widget that takes the focus back when the popup closes.
    pub(crate) return_focus: Option<UiId>,
    /// Whether the pass drew the popup.
    pub(crate) seen: bool,
    /// Whether the popup opened in this pass. The press that opens it never
    /// closes it.
    pub(crate) fresh: bool,
    /// The selection and the scroll offset of the list of the popup.
    pub(crate) list: ListState,
}

/// A modal of the stack of open modals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OpenModal {
    /// The id of the modal.
    pub(crate) id: UiId,
    /// The widget that takes the focus back when the modal closes.
    pub(crate) return_focus: Option<UiId>,
    /// Whether the pass drew the modal.
    pub(crate) seen: bool,
    /// Whether a pass drew the modal since it opened.
    pub(crate) drawn: bool,
}

/// The widget state that outlives a pass.
///
/// The hot widget under the pointer follows from each pass. The pressed
/// widget, the focus, the focus ring and the open popup persist, so a press
/// in one pass and the release in a later pass reach the same widget and a
/// navigation event finds the widgets of the last pass. The caller keeps one
/// state between passes and lends it to each [`super::UiContext`].
#[derive(Debug)]
pub struct UiState {
    /// The number of the window of the state, unique in the process. The
    /// fonts that windows share tell their frames apart by it.
    pub(super) window: u64,
    /// The widget the left button pressed.
    pub(super) active: Option<UiId>,
    /// Whether the active widget ran its pointer behavior in this pass.
    pub(super) active_seen: bool,
    /// The widget with the keyboard focus.
    pub(super) focus: Option<UiId>,
    /// Whether the focus indicator shows. A key event shows it and a press
    /// of the mouse hides it.
    pub(super) focus_visible: bool,
    /// The focusable widgets of this pass in drawing order.
    pub(super) ring: Vec<Focusable>,
    /// The focusable widgets of the last pass in drawing order.
    pub(super) previous_ring: Vec<Focusable>,
    /// The navigation event of this pass that the focused widget handles.
    pub(super) routed: Option<(UiId, super::Navigation)>,
    /// The focused widget that takes the cancel of this pass.
    pub(super) cancel_taker: Option<UiId>,
    /// The last area of a focused widget that left the ring. The next
    /// directional move starts from it.
    pub(super) lost: Option<Rect>,
    /// The area of the focused text widget in physical pixels.
    pub(super) text_area: Option<Rect>,
    /// The offset of the text cursor from the left edge of the text area in
    /// physical pixels.
    pub(super) text_caret: Option<f32>,
    /// The drag of the active widget.
    pub(super) drag: Option<Drag>,
    /// The deepest layer that the last pass drew.
    pub(super) deepest: u32,
    /// The open modals from the lowest to the top one. The modal at index
    /// `i` draws in the layer `i + 1`.
    pub(super) modals: Vec<OpenModal>,
    /// The open popup.
    pub(super) popup: Option<OpenPopup>,
    /// Whether a widget changed its look after it drew and asks for another
    /// pass.
    pub(super) pass_requested: bool,
    /// Whether a widget scrolled by the wheel steps of this pass.
    pub(super) wheel_taken: bool,
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
            deepest: 0,
            modals: Vec::new(),
            popup: None,
            pass_requested: false,
            wheel_taken: false,
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

    /// Whether a popup is open.
    pub fn popup_open(&self) -> bool {
        self.popup.is_some()
    }

    /// The number of open modals.
    pub fn open_modals(&self) -> usize {
        self.modals.len()
    }

    /// Opens the modal `id` between passes, for a dialog that the caller
    /// opens outside a pass. The modal joins the top of the stack. The next
    /// pass must draw the id in [`super::UiContext::layer`]. The layers below
    /// the modal stay blocked from the first widget of a pass on until a pass
    /// draws it. A pass that does not draw it closes it at its end. The
    /// widget that holds the focus now takes it back when the modal closes.
    /// Asks for a pass. A modal that is open already keeps its place.
    pub fn open_layer(&mut self, id: UiId) {
        if !self.modals.iter().any(|modal| modal.id == id) {
            self.modals.push(OpenModal {
                id,
                return_focus: self.focus,
                seen: false,
                drawn: false,
            });
        }
        self.pass_requested = true;
    }

    /// The top layer of the last pass: the layer of the open popup or of
    /// the top modal, or 0 for the base.
    pub(super) fn top_layer(&self) -> u32 {
        let modals = self.modals.len() as u32;
        self.popup.map_or(modals, |popup| popup.layer.max(modals))
    }

    /// Gives the focus to `return_focus` when the focused widget is not in
    /// the ring of this pass, for a layer that closes.
    pub(super) fn give_focus_back(&mut self, return_focus: Option<UiId>) {
        let focus_drawn = self
            .focus
            .is_some_and(|focus| self.ring.iter().any(|entry| entry.id == focus));
        if !focus_drawn && let Some(focus) = return_focus {
            self.focus = Some(focus);
        }
        self.pass_requested = true;
    }

    /// Whether a widget of the last pass changed its look after it drew and
    /// asked for another pass. Clears the request.
    pub fn take_pass_request(&mut self) -> bool {
        std::mem::take(&mut self.pass_requested)
    }

    /// Drops the focus, so the next navigation event starts at the first
    /// widget.
    pub fn clear_focus(&mut self) {
        self.focus = None;
        self.lost = None;
    }

    /// Forgets the focus, the pressed widget, the drag, the focus ring, the
    /// open modals and the open popup, for a dialog that opens.
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
        self.deepest = 0;
        self.modals.clear();
        self.popup = None;
        self.pass_requested = false;
    }

    /// The layer of the widget `id` in the ring of this pass, or else in the
    /// ring of the last pass.
    pub(super) fn layer_of(&self, id: UiId) -> Option<u32> {
        self.ring
            .iter()
            .chain(&self.previous_ring)
            .find(|entry| entry.id == id)
            .map(|entry| entry.layer)
    }

    /// Starts a pass: the ring of the last pass resolves the navigation
    /// event of `input` into a focus change or an event for the focused
    /// widget, and decides who takes cancel.
    ///
    /// Only the widgets of one layer of the ring take part: the layer of the
    /// focused widget, or the deepest layer of the last pass without a
    /// focused widget in the ring.
    pub(super) fn begin_frame(&mut self, input: &Input) {
        std::mem::swap(&mut self.ring, &mut self.previous_ring);
        self.ring.clear();
        self.routed = None;
        self.cancel_taker = None;
        self.text_area = None;
        self.text_caret = None;
        self.active_seen = false;
        self.pass_requested = false;
        self.wheel_taken = false;
        if let Some(popup) = self.popup.as_mut() {
            popup.seen = false;
            popup.fresh = false;
        }
        for modal in &mut self.modals {
            modal.seen = false;
        }
        let lost = self.lost.take();
        if input.left_pressed() {
            self.focus_visible = false;
        }
        if input.navigation.is_some()
            || input.confirm
            || input.action.is_some()
            || input.shortcut.is_some()
        {
            self.focus_visible = true;
        }
        let focused_layer = self.focus.and_then(|focus| {
            self.previous_ring
                .iter()
                .find(|entry| entry.id == focus)
                .map(|entry| entry.layer)
        });
        let scope = focused_layer.unwrap_or(self.deepest);
        let candidates: Vec<Focusable> = self
            .previous_ring
            .iter()
            .filter(|entry| entry.layer == scope)
            .copied()
            .collect();
        let current = self
            .focus
            .and_then(|focus| candidates.iter().position(|entry| entry.id == focus));
        if input.cancel
            && let Some(index) = current
            && candidates[index].takes_cancel
        {
            self.cancel_taker = Some(candidates[index].id);
        }
        let Some(navigation) = input.navigation else {
            return;
        };
        match current {
            Some(index) if candidates[index].captures.contains(navigation) => {
                self.routed = Some((candidates[index].id, navigation));
            }
            _ => {
                let from = current.map(|index| candidates[index].rect).or(lost);
                if let Some(next) = next_focus(&candidates, current, from, navigation) {
                    self.focus = Some(candidates[next].id);
                }
            }
        }
    }

    /// Closes a pass that drew layers down to `deepest`. A modal that the
    /// pass did not draw leaves the stack and gives the focus back, and so
    /// does a popup. A focus on a widget outside the ring of the pass is
    /// dropped, and its last area stays for the next directional move. A
    /// pressed widget that skipped its pointer behavior loses the press and
    /// its drag.
    pub(super) fn end_frame(&mut self, deepest: u32) {
        if let Some(popup) = self.popup.filter(|popup| !popup.seen) {
            self.popup = None;
            self.give_focus_back(popup.return_focus);
        }
        while let Some(index) = self.modals.iter().rposition(|modal| !modal.seen) {
            let modal = self.modals.remove(index);
            self.give_focus_back(modal.return_focus);
        }
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
        self.deepest = deepest;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Navigation, NavigationSet, focus::Focusable, ui_id_indexed};

    /// A focusable entry `index` at `rect` in `layer`.
    fn entry(index: usize, rect: Rect, layer: u32) -> Focusable {
        Focusable {
            layer,
            ..Focusable::new(ui_id_indexed("widget", index), rect, NavigationSet::EMPTY)
        }
    }

    /// Runs a pass with `navigation` over the ring `ring`.
    fn pass(state: &mut UiState, ring: &[Focusable], navigation: Navigation) {
        state.ring = ring.to_vec();
        let input = Input {
            navigation: Some(navigation),
            ..Input::default()
        };
        state.begin_frame(&input);
        state.ring = ring.to_vec();
        state.end_frame(ring.iter().map(|entry| entry.layer).max().unwrap_or(0));
    }

    #[test]
    fn navigation_stays_in_the_layer_of_the_focused_widget() {
        let ring = [
            entry(0, Rect::new(0.0, 0.0, 100.0, 20.0), 0),
            entry(1, Rect::new(0.0, 30.0, 100.0, 20.0), 1),
            entry(2, Rect::new(0.0, 60.0, 100.0, 20.0), 0),
            entry(3, Rect::new(0.0, 90.0, 100.0, 20.0), 1),
        ];
        let mut state = UiState {
            ring: ring.to_vec(),
            ..UiState::default()
        };
        state.end_frame(1);
        state.set_focus(ring[1].id);
        pass(&mut state, &ring, Navigation::Down);
        assert_eq!(state.focused(), Some(ring[3].id));
        pass(&mut state, &ring, Navigation::Next);
        assert_eq!(state.focused(), Some(ring[1].id));
        state.set_focus(ring[0].id);
        pass(&mut state, &ring, Navigation::Down);
        assert_eq!(state.focused(), Some(ring[2].id));
    }

    #[test]
    fn without_a_focus_navigation_starts_in_the_deepest_layer() {
        let ring = [
            entry(0, Rect::new(0.0, 0.0, 100.0, 20.0), 0),
            entry(1, Rect::new(0.0, 30.0, 100.0, 20.0), 1),
        ];
        let mut state = UiState {
            ring: ring.to_vec(),
            ..UiState::default()
        };
        state.end_frame(1);
        pass(&mut state, &ring, Navigation::Next);
        assert_eq!(state.focused(), Some(ring[1].id));
    }

    #[test]
    fn reset_closes_the_popup_and_the_pass_request() {
        let mut state = UiState {
            popup: Some(OpenPopup {
                owner: UiId::from("model"),
                layer: 1,
                return_focus: None,
                seen: true,
                fresh: false,
                list: ListState::new(),
            }),
            pass_requested: true,
            ..UiState::default()
        };
        assert!(state.popup_open());
        state.reset();
        assert!(!state.popup_open());
        assert!(!state.take_pass_request());
    }

    #[test]
    fn a_popup_that_is_not_drawn_closes() {
        let mut state = UiState {
            popup: Some(OpenPopup {
                owner: UiId::from("model"),
                layer: 1,
                return_focus: None,
                seen: true,
                fresh: false,
                list: ListState::new(),
            }),
            ..UiState::default()
        };
        state.begin_frame(&Input::default());
        state.end_frame(0);
        assert!(!state.popup_open());
    }
}
