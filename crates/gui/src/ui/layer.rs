//! The layers of a pass: the stack of open modals, the popup layer and the
//! focus scope of each layer.
//!
//! A modal draws in the layer of its place in the stack of open modals, and
//! a popup one layer above the widget that opens it. The top layer of the
//! last pass blocks every layer below it from the first widget of a pass
//! on, see [`UiContext::is_blocked`]. The focus ring keeps the widgets of
//! every layer, and navigation, tab switching and cancel reach only the
//! layer of the focused widget. A layer that opens takes the focus and a
//! layer that closes gives it back.

use super::{
    context::{UiContext, UiId},
    list::ListState,
    state::{OpenModal, OpenPopup},
};
use crate::Rect;

impl UiContext<'_> {
    /// Draws `draw` as the modal `id`, in a layer above everything drawn so
    /// far, and returns its result.
    ///
    /// A modal that the last pass did not draw opens: it joins the top of
    /// the stack of open modals, takes the focus and asks for another pass.
    /// The layer of a modal is its place in the stack, so modals drawn one
    /// after the other stack in the order they open. A modal that a pass
    /// does not draw closes at the end of the pass, see also
    /// [`UiContext::close_layer`]. A closing modal gives the focus back to
    /// the widget that held it when the modal opened.
    ///
    /// When no widget of the modal holds the focus after `draw`, the first
    /// preferred widget of the modal takes it, such as its primary button,
    /// or else its first widget that is a default choice. The close button
    /// of a header is never a default choice.
    pub fn layer<R>(&mut self, id: impl Into<UiId>, draw: impl FnOnce(&mut Self) -> R) -> R {
        let id = id.into();
        let index = match self.state.modals.iter().position(|modal| modal.id == id) {
            Some(index) => index,
            None => {
                self.state.modals.push(OpenModal {
                    id,
                    return_focus: self.state.focus,
                    seen: false,
                    drawn: false,
                });
                self.request_pass();
                self.state.modals.len() - 1
            }
        };
        let modal = &mut self.state.modals[index];
        modal.seen = true;
        if !modal.drawn {
            modal.drawn = true;
            if modal.return_focus.is_none() {
                modal.return_focus = self.state.focus;
            }
        }
        let depth = index as u32 + 1;
        let outer_depth = self.depth;
        let outer_capture = self.mouse_captured;
        let start = self.state.ring.len();
        self.depth = depth;
        self.deepest = self.deepest.max(depth);
        self.blocked = self.blocked.max(depth);
        self.mouse_captured = false;
        let result = draw(self);
        if self.state.modals.iter().any(|modal| modal.id == id) {
            self.take_focus_into_layer(start);
        }
        self.depth = outer_depth;
        self.mouse_captured = outer_capture;
        result
    }

    /// Closes the modal that draws at this point, for a modal that the
    /// caller stops drawing after this pass. It leaves the stack now, gives
    /// the focus back and asks for another pass, so the next key reaches the
    /// layer below.
    pub fn close_layer(&mut self) {
        let Some(index) = (self.depth as usize).checked_sub(1) else {
            return;
        };
        if index >= self.state.modals.len() {
            return;
        }
        let modal = self.state.modals.remove(index);
        let depth = self.depth;
        let inside = self.state.focus.is_none_or(|focus| {
            self.state
                .layer_of(focus)
                .is_none_or(|layer| layer >= depth)
        });
        if inside && let Some(focus) = modal.return_focus {
            self.state.focus = Some(focus);
        }
        self.request_pass();
    }

    /// Gives the focus to the default widget of the current layer,
    /// registered from the ring position `start` on, when the focus lies
    /// below the layer or outside the ring. Asks for another pass when the
    /// focus changes.
    fn take_focus_into_layer(&mut self, start: usize) {
        if self.is_blocked() {
            return;
        }
        let depth = self.depth;
        let inside = self.state.focus.is_some_and(|focus| {
            self.state
                .ring
                .iter()
                .find(|entry| entry.id == focus)
                .is_some_and(|entry| entry.layer >= depth)
        });
        if inside {
            return;
        }
        let entries = &self.state.ring[start..];
        let candidates = || {
            entries
                .iter()
                .filter(|entry| entry.layer == depth && entry.default_choice)
        };
        let choice = candidates()
            .find(|entry| entry.preferred)
            .or_else(|| candidates().next())
            .map(|entry| entry.id);
        if self.state.focus != choice {
            self.state.focus = choice;
            self.request_pass();
        }
    }

    /// Opens the popup of the widget `owner` one layer above the current
    /// layer. Every widget outside the popup is blocked from here on until
    /// the popup closes. The widget that holds the focus now takes it back
    /// when the popup closes. An open popup of another widget closes. Asks
    /// for another pass.
    pub fn open_popup(&mut self, owner: UiId) {
        let layer = self.depth + 1;
        self.state.popup = Some(OpenPopup {
            owner,
            layer,
            return_focus: self.state.focus,
            seen: false,
            fresh: true,
            list: ListState::new(),
        });
        self.blocked = self.blocked.max(layer);
        self.mouse_captured = true;
        self.request_pass();
    }

    /// Closes the open popup, gives the focus back to the widget that held
    /// it when the popup opened and asks for another pass. The rest of the
    /// pass stays blocked.
    pub fn close_popup(&mut self) {
        if let Some(popup) = self.state.popup.take() {
            if let Some(focus) = popup.return_focus {
                self.state.focus = Some(focus);
            }
            self.request_pass();
        }
    }

    /// Whether the popup of `owner` is open.
    pub fn popup_open(&self, owner: UiId) -> bool {
        self.state.popup.is_some_and(|popup| popup.owner == owner)
    }

    /// Draws `draw` on the popup layer of `owner` when its popup is open,
    /// and returns its result. The popup covers `rect`.
    ///
    /// The items of the popup layer draw after every other item of the pass
    /// with a clip of their own, and the widgets of `draw` form the focus
    /// scope of the popup. The popup captures the input while it is open:
    /// the widgets outside it are blocked. A press outside `rect` and a
    /// cancel that no widget of the popup takes close the popup and give the
    /// focus back, unless a layer above the popup blocks it.
    pub fn popup<R>(
        &mut self,
        owner: UiId,
        rect: Rect,
        draw: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        let popup = self.state.popup.filter(|popup| popup.owner == owner)?;
        if let Some(open) = self.state.popup.as_mut() {
            open.seen = true;
        }
        let outer_depth = self.depth;
        let outer_capture = self.mouse_captured;
        let outer_scroll = self.scroll.take();
        self.depth = popup.layer;
        self.deepest = self.deepest.max(popup.layer);
        self.mouse_captured = false;
        self.scene.begin_overlay();
        let result = draw(self);
        self.scene.end_overlay();
        let (x, y) = self.mouse();
        let outside_press =
            !popup.fresh && !self.is_blocked() && self.input.left_pressed() && !rect.contains(x, y);
        if self.popup_open(owner) && (outside_press || self.cancelled()) {
            self.close_popup();
        }
        self.depth = outer_depth;
        self.scroll = outer_scroll;
        self.mouse_captured = outer_capture || rect.contains(x, y);
        Some(result)
    }

    /// The list state of the open popup.
    pub(crate) fn popup_list(&self) -> ListState {
        self.state
            .popup
            .map_or_else(ListState::new, |popup| popup.list)
    }

    /// Keeps `list` as the list state of the open popup.
    pub(crate) fn set_popup_list(&mut self, list: ListState) {
        if let Some(popup) = self.state.popup.as_mut() {
            popup.list = list;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Color, Rect,
        ui::{
            ActionButton, Button, ButtonKind, List, ListState, Navigation, NavigationSet, Tab,
            UiContext, UiId, modal, tab_bar, tests::Harness,
        },
    };

    /// The area of the button below the modals.
    const BELOW: Rect = Rect::new(0.0, 0.0, 100.0, 30.0);

    /// The area of the first button of a modal.
    const FIRST: Rect = Rect::new(200.0, 200.0, 100.0, 30.0);

    /// The area of the second button of a modal.
    const SECOND: Rect = Rect::new(320.0, 200.0, 100.0, 30.0);

    /// Draws a focusable button with the id `label` in `rect` and returns
    /// whether a click or confirm activated it.
    fn button(context: &mut UiContext, label: &str, rect: Rect) -> bool {
        let id = UiId::from(label);
        context.focusable(id, rect, NavigationSet::EMPTY);
        context.button_behavior(id, rect).clicked || context.confirmed(id)
    }

    /// Draws a button below a modal and, when `open`, the modal `dialog`
    /// with two buttons. Returns the activations of the three buttons.
    fn draw(context: &mut UiContext, open: bool) -> [bool; 3] {
        let below = button(context, "below", BELOW);
        if !open {
            return [below, false, false];
        }
        context.layer("dialog", |context| {
            let first = button(context, "first", FIRST);
            let second = button(context, "second", SECOND);
            [below, first, second]
        })
    }

    #[test]
    fn a_modal_takes_the_focus_and_keeps_navigation_inside() {
        let mut harness = Harness::new();
        harness.state.set_focus(UiId::from("below"));
        harness.frame(|context| draw(context, false));
        harness.frame(|context| draw(context, true));
        assert_eq!(harness.state.focused(), Some(UiId::from("first")));
        assert!(harness.state.take_pass_request());
        for navigation in [Navigation::Next, Navigation::Next, Navigation::Left] {
            harness.navigate(navigation);
            harness.frame(|context| draw(context, true));
            assert_ne!(harness.state.focused(), Some(UiId::from("below")));
        }
        harness.navigate(Navigation::Up);
        harness.frame(|context| draw(context, true));
        assert_eq!(harness.state.focused(), Some(UiId::from("first")));
        harness.input.confirm = true;
        assert_eq!(
            harness.frame(|context| draw(context, true)),
            [false, true, false]
        );
    }

    #[test]
    fn a_base_click_under_a_modal_does_not_fire() {
        let mut harness = Harness::new();
        harness.frame(|context| draw(context, true));
        harness.press(50.0, 15.0);
        harness.frame(|context| draw(context, true));
        harness.release();
        assert_eq!(harness.frame(|context| draw(context, true)), [false; 3]);
        harness.press(250.0, 215.0);
        harness.frame(|context| draw(context, true));
        harness.release();
        assert_eq!(
            harness.frame(|context| draw(context, true)),
            [false, true, false]
        );
    }

    #[test]
    fn a_closed_modal_gives_the_focus_back_and_the_next_key_reaches_the_base() {
        let mut harness = Harness::new();
        harness.state.set_focus(UiId::from("below"));
        harness.frame(|context| draw(context, false));
        harness.frame(|context| draw(context, true));
        harness.frame(|context| draw(context, false));
        assert_eq!(harness.state.focused(), Some(UiId::from("below")));
        assert_eq!(harness.state.open_modals(), 0);
        assert!(harness.state.take_pass_request());
        harness.input.confirm = true;
        assert_eq!(
            harness.frame(|context| draw(context, false)),
            [true, false, false]
        );
    }

    #[test]
    fn close_layer_unblocks_the_base_for_the_next_key() {
        let mut harness = Harness::new();
        harness.state.set_focus(UiId::from("below"));
        harness.frame(|context| draw(context, false));
        harness.frame(|context| draw(context, true));
        harness.input.confirm = true;
        harness.frame(|context| {
            button(context, "below", BELOW);
            context.layer("dialog", |context| {
                if button(context, "first", FIRST) {
                    context.close_layer();
                }
                button(context, "second", SECOND);
            });
        });
        assert_eq!(harness.state.open_modals(), 0);
        assert_eq!(harness.state.focused(), Some(UiId::from("below")));
        assert!(harness.state.take_pass_request());
        harness.input.confirm = true;
        assert_eq!(
            harness.frame(|context| draw(context, false)),
            [true, false, false]
        );
    }

    /// Draws two sibling modals with one button each, `a` and then `b`.
    /// Returns whether each button is hovered.
    fn siblings(context: &mut UiContext) -> (bool, bool) {
        let rect_a = Rect::new(100.0, 100.0, 100.0, 30.0);
        let rect_b = Rect::new(120.0, 110.0, 100.0, 30.0);
        let a = context.layer("a", |context| {
            context.focusable(UiId::from("a"), rect_a, NavigationSet::EMPTY);
            context.button_behavior(UiId::from("a"), rect_a).hovered
        });
        let b = context.layer("b", |context| {
            context.focusable(UiId::from("b"), rect_b, NavigationSet::EMPTY);
            context.button_behavior(UiId::from("b"), rect_b).hovered
        });
        (a, b)
    }

    #[test]
    fn sibling_modals_stack_in_call_order() {
        let mut harness = Harness::new();
        harness.mouse(150.0, 120.0);
        harness.frame(siblings);
        assert_eq!(harness.state.open_modals(), 2);
        assert_eq!(harness.state.focused(), Some(UiId::from("b")));
        assert_eq!(harness.frame(siblings), (false, true));
        harness.navigate(Navigation::Next);
        harness.frame(siblings);
        assert_eq!(harness.state.focused(), Some(UiId::from("b")));
    }

    #[test]
    fn a_stack_of_dialogs_in_a_vec_draws_without_recursion() {
        let mut harness = Harness::new();
        let mut dialogs = vec!["editor", "confirm"];
        let draw = |context: &mut UiContext, dialogs: &[&str]| {
            button(context, "below", BELOW);
            let mut closed = None;
            for (index, name) in dialogs.iter().enumerate() {
                let rect = Rect::new(200.0 + index as f32 * 20.0, 200.0, 100.0, 30.0);
                if context.layer(*name, |context| {
                    let closed = modal(context, *name, (300.0, 200.0), name).closed;
                    button(context, name, rect);
                    closed
                }) {
                    closed = Some(index);
                }
            }
            closed
        };
        harness.frame(|context| draw(context, &dialogs));
        assert_eq!(harness.state.focused(), Some(UiId::from("confirm")));
        harness.input.cancel = true;
        assert_eq!(harness.frame(|context| draw(context, &dialogs)), Some(1));
        dialogs.pop();
        harness.frame(|context| draw(context, &dialogs));
        assert_eq!(harness.state.open_modals(), 1);
        assert_eq!(harness.state.focused(), Some(UiId::from("editor")));
        harness.input.cancel = true;
        assert_eq!(harness.frame(|context| draw(context, &dialogs)), Some(0));
    }

    /// Draws the button below and, once it opened, the dialog with its OK
    /// button. Returns whether OK was activated.
    fn draw_opened(context: &mut UiContext, open: &mut bool) -> bool {
        if button(context, "below", BELOW) {
            *open = true;
        }
        *open
            && context.layer("dialog", |context| {
                button(context, "ok", Rect::new(200.0, 200.0, 100.0, 30.0))
            })
    }

    /// Verifies that the widget that takes the focus back from a modal keeps
    /// its focus look while the modal blocks it, and that the others do not.
    #[test]
    fn the_return_focus_of_a_modal_keeps_its_focus_look() {
        let mut harness = Harness::new();
        let draw = |context: &mut UiContext, open: bool| {
            button(context, "below", BELOW);
            button(context, "other", Rect::new(20.0, 100.0, 100.0, 30.0));
            let looks = (
                context.shows_focus(UiId::from("below")),
                context.shows_focus(UiId::from("other")),
            );
            if open {
                context.layer("dialog", |context| {
                    button(context, "ok", Rect::new(200.0, 200.0, 100.0, 30.0));
                });
            }
            looks
        };
        harness.state.set_focus(UiId::from("below"));
        assert_eq!(harness.frame(|context| draw(context, false)), (true, false));
        harness.frame(|context| draw(context, true));
        assert_eq!(harness.state.focused(), Some(UiId::from("ok")));
        assert_eq!(harness.frame(|context| draw(context, true)), (true, false));
        harness.frame(|context| draw(context, false));
        assert_eq!(harness.state.focused(), Some(UiId::from("below")));
    }

    /// Verifies that a modal opened by a confirm takes the focus and ignores that confirm.
    #[test]
    fn a_modal_that_opens_in_a_pass_ignores_the_key_of_that_pass() {
        let mut harness = Harness::new();
        let mut open = false;
        harness.state.set_focus(UiId::from("below"));
        harness.frame(|context| draw_opened(context, &mut open));
        harness.input.confirm = true;
        assert!(!harness.frame(|context| draw_opened(context, &mut open)));
        assert!(open);
        assert_eq!(harness.state.open_modals(), 1);
        assert_eq!(harness.state.focused(), Some(UiId::from("ok")));
        assert!(harness.state.take_pass_request());
        assert!(!harness.frame(|context| draw_opened(context, &mut open)));
        harness.input.confirm = true;
        assert!(harness.frame(|context| draw_opened(context, &mut open)));
    }

    #[test]
    fn a_modal_prefers_its_primary_button_over_the_close_button() {
        let mut harness = Harness::new();
        let draw = |context: &mut UiContext| {
            context.layer("dialog", |context| {
                let response = modal(context, "dialog", (400.0, 200.0), "Dialog");
                let area = response.content;
                let cancel = Button::new("cancel", "Cancel")
                    .show(context, Rect::new(area.x, area.y + 50.0, 96.0, 32.0));
                let ok = Button::new("ok", "OK").kind(ButtonKind::Primary).show(
                    context,
                    Rect::new(area.x + 104.0, area.y + 50.0, 104.0, 32.0),
                );
                (response.closed, cancel, ok)
            })
        };
        harness.frame(draw);
        assert_eq!(harness.state.focused(), Some(UiId::from("ok")));
        harness.input.confirm = true;
        assert_eq!(harness.frame(draw), (false, false, true));

        let mut harness = Harness::new();
        let only_close = |context: &mut UiContext| {
            context.layer("dialog", |context| {
                modal(context, "dialog", (400.0, 200.0), "Dialog").closed
            })
        };
        harness.frame(only_close);
        assert_eq!(harness.state.focused(), None);
        harness.input.confirm = true;
        assert!(!harness.frame(only_close));
    }

    #[test]
    fn tab_bars_action_buttons_and_the_wheel_are_blocked_under_a_modal() {
        let tabs = [Tab::new("One", None), Tab::new("Two", None)];
        let mut harness = Harness::new();
        let mut selected = 0;
        let mut list = ListState::new();
        let mut draw = |harness: &mut Harness| {
            harness.frame(|context| {
                tab_bar(
                    context,
                    "tabs",
                    Rect::new(0.0, 0.0, 300.0, 40.0),
                    &tabs,
                    &mut selected,
                );
                let action = context.action_button();
                List::new("list", 40).show(
                    context,
                    Rect::new(0.0, 50.0, 300.0, 200.0),
                    &mut list,
                    |_, _| {},
                );
                let inside = context.layer("dialog", |context| context.action_button());
                (action, inside)
            })
        };
        draw(&mut harness);
        harness.navigate(Navigation::NextTab);
        draw(&mut harness);
        harness.input.action = Some(ActionButton::X);
        let (below, inside) = draw(&mut harness);
        harness.mouse(100.0, 100.0);
        harness.input.wheel = -3.0;
        draw(&mut harness);
        assert_eq!(selected, 0);
        assert_eq!((below, inside), (None, Some(ActionButton::X)));
        assert_eq!(list.scroll, 0.0);
    }

    #[test]
    fn one_cancel_closes_only_the_top_layer() {
        let mut harness = Harness::new();
        let draw = |context: &mut UiContext| {
            let base = context.cancelled();
            let lower = context.layer("lower", |context| {
                modal(context, "lower", (500.0, 400.0), "Lower").closed
            });
            let upper = context.layer("upper", |context| {
                modal(context, "upper", (300.0, 200.0), "Upper").closed
            });
            (base, lower, upper)
        };
        harness.frame(draw);
        harness.input.cancel = true;
        assert_eq!(harness.frame(draw), (false, false, true));
    }

    /// The area of the popup of the tests.
    const POPUP: Rect = Rect::new(100.0, 100.0, 200.0, 100.0);

    /// Draws an owner button that opens a popup with one focusable entry,
    /// and a button after it. Returns whether the button after the popup
    /// was hovered.
    fn draw_popup(context: &mut UiContext, open: bool) -> bool {
        let owner = UiId::from("owner");
        context.focusable(
            owner,
            Rect::new(100.0, 60.0, 200.0, 30.0),
            NavigationSet::EMPTY,
        );
        if open && !context.popup_open(owner) {
            context.open_popup(owner);
            context.set_focus(UiId::from("entry"));
        }
        context.popup(owner, POPUP, |context| {
            context.fill(POPUP, Color::WHITE);
            context.focusable(UiId::from("entry"), POPUP, NavigationSet::EMPTY);
        });
        context
            .button_behavior(UiId::from("after"), Rect::new(100.0, 150.0, 50.0, 20.0))
            .hovered
    }

    #[test]
    fn a_popup_captures_the_input_and_draws_last() {
        let mut harness = Harness::new();
        harness.state.set_focus(UiId::from("owner"));
        harness.mouse(120.0, 160.0);
        harness.frame(|context| draw_popup(context, false));
        assert!(!harness.frame(|context| {
            let hovered = draw_popup(context, true);
            context.fill(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK);
            hovered
        }));
        assert_eq!(harness.state.focused(), Some(UiId::from("entry")));
        let last = harness.scene.instances().last().unwrap();
        assert_eq!(last.rect, [100.0, 100.0, 200.0, 100.0]);
        harness.navigate(Navigation::Up);
        harness.frame(|context| draw_popup(context, false));
        assert_eq!(harness.state.focused(), Some(UiId::from("entry")));
        assert!(harness.state.popup_open());
    }

    #[test]
    fn a_press_outside_or_cancel_closes_the_popup() {
        let mut harness = Harness::new();
        harness.state.set_focus(UiId::from("owner"));
        harness.frame(|context| draw_popup(context, true));
        harness.press(10.0, 10.0);
        harness.frame(|context| draw_popup(context, false));
        assert!(!harness.state.popup_open());
        assert!(harness.state.take_pass_request());
        assert_eq!(harness.state.focused(), Some(UiId::from("owner")));
        harness.release();
        harness.frame(|context| draw_popup(context, true));
        harness.input.cancel = true;
        let cancelled = harness.frame(|context| {
            draw_popup(context, false);
            context.cancelled()
        });
        assert!(!cancelled);
        assert!(!harness.state.popup_open());
        assert_eq!(harness.state.focused(), Some(UiId::from("owner")));
    }

    #[test]
    fn a_popup_inside_a_modal_closes_before_the_modal() {
        let mut harness = Harness::new();
        let draw = |context: &mut UiContext, open: bool| {
            button(context, "below", BELOW);
            context.layer("dialog", |context| {
                let closed = modal(context, "dialog", (500.0, 400.0), "Dialog").closed;
                draw_popup(context, open);
                closed
            })
        };
        harness.state.set_focus(UiId::from("below"));
        harness.frame(|context| draw(context, false));
        assert_eq!(harness.state.focused(), Some(UiId::from("owner")));
        harness.frame(|context| draw(context, true));
        assert_eq!(harness.state.focused(), Some(UiId::from("entry")));
        harness.input.cancel = true;
        assert!(!harness.frame(|context| draw(context, false)));
        assert!(!harness.state.popup_open());
        assert_eq!(harness.state.focused(), Some(UiId::from("owner")));
        harness.input.cancel = true;
        assert!(harness.frame(|context| draw(context, false)));
    }

    #[test]
    fn a_modal_drawn_one_pass_late_still_opens() {
        let mut harness = Harness::new();
        harness.state.set_focus(UiId::from("below"));
        harness.frame(|context| draw(context, false));
        harness.state.open_layer(UiId::from("dialog"));
        assert!(harness.state.take_pass_request());
        harness.input.confirm = true;
        assert_eq!(harness.frame(|context| draw(context, true)), [false; 3]);
        assert_eq!(harness.state.focused(), Some(UiId::from("first")));
        harness.input.confirm = true;
        assert_eq!(
            harness.frame(|context| draw(context, true)),
            [false, true, false]
        );
        harness.frame(|context| draw(context, false));
        assert_eq!(harness.state.focused(), Some(UiId::from("below")));
    }

    #[test]
    fn a_modal_opened_without_a_focus_returns_to_the_focus_of_its_first_pass() {
        let mut harness = Harness::new();
        harness.state.open_layer(UiId::from("dialog"));
        harness.frame(|context| {
            context.set_focus(UiId::from("below"));
            draw(context, true)
        });
        assert_eq!(harness.state.focused(), Some(UiId::from("first")));
        harness.frame(|context| draw(context, false));
        assert_eq!(harness.state.focused(), Some(UiId::from("below")));
    }
}
