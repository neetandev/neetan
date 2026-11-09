//! Components: one shape for the screens and the dialogs of an application
//! on the immediate mode widgets of [`crate::ui`].
//!
//! Every screen and every dialog is a [`Component`] with three parts:
//!
//! 1. [`Component::view`] draws one pass with the widgets of [`crate::ui`]
//!    and records what the user asked for as messages. Its widgets write the
//!    values they are bound to.
//! 2. [`Component::update`] applies one message. It is the only place that
//!    computes derived values, does file I/O, opens or closes children and
//!    emits effects.
//! 3. [`Status`] is the answer of a component after a pass: still open,
//!    closed, or done with a result. The owner of a child receives only the
//!    end of the child, an [`Ended`], as a message.
//!
//! [`run`] and [`run_child`] execute a component for one pass: the view, the
//! updates, the modal layer of a child and the request for the next pass.
//! The split follows the Model / View / Update architecture of Elm. The view
//! stays immediate: it draws the whole component from its state in every
//! pass and needs no widget tree. Nothing in [`crate::ui`] depends on this
//! module.
//!
//! # Who writes what
//!
//! The widgets in `view` write the values they are bound to: the values that
//! the user changes directly through a widget, such as the text of a text
//! field, the selection of a list or a dropdown, the sort of a table, the
//! selected tab and the scroll position. The view also refills its caches,
//! such as a [`crate::ui::TextBuffer`] or the rows of the last pass. The
//! widgets need `&mut` access to these values, and a message for each
//! keystroke would only copy every value twice.
//!
//! `update` writes everything else: every value derived from the bound
//! values (validation, a dirty flag, filtered entries), every side effect
//! (file I/O, a rescan), every open and close of a child, every focus
//! request that follows an update, and every effect.
//!
//! State derived from [`Component::Input`] stays in the view, because
//! `update` cannot see the input, and the site carries a short comment.
//!
//! When a widget changes a bound value that has consequences, the view emits
//! a message for it (`FilterChanged`, `FieldChanged`), and the update of the
//! message computes the consequences. Rule of thumb: a change that would be
//! worth a log line or a test assertion belongs in `update`.
//!
//! # Messages
//!
//! Each component declares one message enum, [`Component::Message`]. The
//! enum lists everything the component can do, and it is the first thing a
//! reader of the component reads.
//!
//! - A message names an intent of the user or a result of a child
//!   (`Delete(Ended<()>)`, `Child(ChildEvent<ChildEffect, ()>)`). It never
//!   names a widget (`ButtonClicked(3)`).
//! - Keystrokes inside a text field change its bound value. The view emits
//!   the meaning: `FilterChanged`, `Submit`.
//! - A message carries what its update needs from the input of the pass,
//!   because `update` has no access to [`Component::Input`].
//! - Shortcuts, action buttons and toolbar buttons that mean the same thing
//!   emit the same message.
//! - The view emits the messages in drawing order, and the updates apply
//!   them in that order.
//! - A view emits a message only for the input of the pass, or for a
//!   condition that the update of the message makes false. A pass without
//!   input therefore emits no message, and the chain of passes ends. A
//!   message that fires in every pass would chain passes up to the cap of
//!   the application in every event.
//! - A view emits the message that ends its component last. [`run`] stops at
//!   the first update that returns [`Status::Closed`] or [`Status::Done`],
//!   drops the messages after it, and a debug build panics.
//!
//! # Status, end and child events
//!
//! [`Status`] reports how a component ends. A component with one result
//! uses `Status<()>`, a component with more than one result uses an enum for
//! [`Component::Output`], and a root component without an owner uses
//! [`std::convert::Infallible`]. [`Status::Open`] never becomes a message, so
//! an open child adds nothing to the messages of its owner and a steady pass
//! stays without messages.
//!
//! [`run_child`] hands the owner each event of a child as a [`ChildEvent`]:
//! an effect while the child stays open, or its end. A child without effects
//! maps its events with [`ChildEvent::into_ended`]
//! (`|event| Message::Delete(event.into_ended())`). A child with effects
//! maps the whole event (`Message::Child`).
//!
//! # Effects
//!
//! An effect reports something that happened while a component stays open,
//! or a request to the owner that the component cannot carry out itself.
//! Each component names its effect type as [`Component::Effect`], and an
//! update reports an effect with [`Update::emit`]. The effect goes straight
//! to the receiver that the caller of [`run`] gave, so no component keeps a
//! buffer of effects.
//!
//! - A leaf dialog without effects uses [`std::convert::Infallible`].
//! - A child with effects hands them to its owner: [`run_child`] turns each
//!   one into a message of the owner, which the owner applies in its own
//!   update.
//! - The root component of a pass hands its effects to the application, such
//!   as the start of a game or a switch of the language.
//!
//! # Layers
//!
//! A child dialog draws in a modal layer, see [`Layer`].
//!
//! - The component names its layer once, in [`Component::LAYER`]. [`run`]
//!   and [`Update::open`] both read it.
//! - With [`Layer::Modal`], [`run`] opens the layer around the view and the
//!   updates and closes it when the component ends. The view never calls
//!   [`UiContext::layer`] or [`UiContext::close_layer`] itself, because only
//!   the status of an update knows whether the component ends. A dialog
//!   whose update fails stays open in its layer and keeps its focus.
//! - The update that sets an overlay calls [`Update::open`] for the new
//!   child. After the updates, [`run`] opens the requested layers for the
//!   next pass, so the layer blocks the widgets below it from the start of
//!   the next pass. A queued key of the same batch of events then stays out
//!   of the screen below the child, although the child draws only in the
//!   next pass.
//! - A widget that opens and closes its own layer, such as
//!   [`crate::ui::ConfirmDialog`], belongs to a component with
//!   [`Layer::Widget`]. [`run`] leaves the layer to the widget, and
//!   [`Update::open`] still opens it for the next pass.
//! - A child of a child draws in a layer inside the layer of its owner.
//! - An update replaces an overlay only after its child ended. A child that
//!   ends has closed its layer in [`run`], so the layer of the next child
//!   opens on a clean stack.
//!
//! # Overlays
//!
//! A component that opens dialogs keeps them in one enum, its overlay, with
//! one variant for each child and one for none. Two dialogs that can never
//! be open together never sit in two `Option` fields. The drawing order and
//! the hints of a status line follow from one `match` on the overlay: the
//! open child decides. A child keeps what its owner draws from it, such as
//! its hints, as its own state of the last pass, and the owner reads it
//! after [`run_child`]. The input of a child comes from fields of the owner,
//! because the view borrows the overlay mutably for [`run_child`] and a
//! `&self` method would borrow the whole owner. The owner removes the child
//! from its overlay in the update of the end message.
//!
//! # Focus
//!
//! [`Update`] has no access to the focus.
//!
//! - A child in a modal layer needs no focus request: a layer that opens
//!   takes the focus, and a layer that closes gives it back.
//! - A focus change for the input of the same pass stays in the view, right
//!   where the view reads the shortcut, with [`UiContext::set_focus`].
//! - A focus change that follows an update is a request in a field of the
//!   component, an `Option` of a small focus enum of the component. The
//!   update sets the field. The view takes it at the start of the next pass,
//!   resolves it and calls [`UiContext::set_focus`]. This take is the one
//!   write of the view into a field that the update owns: a target such as
//!   "the first row" depends on what the pass draws.
//!
//! # The pass loop of the application
//!
//! Every pass of [`run`] that changes state and leaves the component open
//! asks for another pass. A pass that ends a child still leads to a pass of
//! its owner, because the end becomes a message of the owner. An
//! application runs every chained pass before it renders a frame: after the
//! pass of an event, it runs passes while the user interface asks for one,
//! up to its cap of chained passes. The rendered frame then shows the
//! settled state, and an intermediate pass, such as a screen that a new
//! dialog blocks before the dialog draws, never reaches the screen. The loop
//! stops early when an effect of a pass leaves the user interface, such as
//! the start of a game.
//!
//! # How to read and write a component
//!
//! To understand a component, read its message enum (what it can do), its
//! overlay enum (which children it can open), its `update` (what each
//! message changes) and its `view` (how it looks and which input emits
//! which message).
//!
//! To add a feature, add a message, emit it in `view` where the widget or
//! the shortcut is, handle it in `update`, test `update` directly and add
//! one test of the input that emits it.
//!
//! ```
//! use std::convert::Infallible;
//!
//! use gui::{
//!     Rect,
//!     component::{Component, Layer, Status, Update},
//!     ui::{Button, UiContext, modal},
//! };
//!
//! /// Asks whether to quit.
//! struct QuitQuestion {
//!     messages: Vec<QuitMessage>,
//! }
//!
//! /// What the user can answer.
//! enum QuitMessage {
//!     Quit,
//!     Stay,
//! }
//!
//! impl Component for QuitQuestion {
//!     type Message = QuitMessage;
//!     type Output = ();
//!     type Effect = Infallible;
//!     type Input<'a> = ();
//!
//!     const LAYER: Layer = Layer::Modal("quit");
//!
//!     fn messages(&mut self) -> &mut Vec<QuitMessage> {
//!         &mut self.messages
//!     }
//!
//!     fn view(
//!         &mut self,
//!         context: &mut UiContext,
//!         area: Rect,
//!         _input: (),
//!         messages: &mut Vec<QuitMessage>,
//!     ) {
//!         let panel = modal(context, "quit", (area.width.min(320.0), 160.0), "Quit?");
//!         let content = panel.content;
//!         let stay = Button::new("stay", "Stay")
//!             .show(context, Rect::new(content.x, content.y, 96.0, 32.0));
//!         let quit = Button::new("quit", "Quit")
//!             .show(context, Rect::new(content.x + 104.0, content.y, 96.0, 32.0));
//!         if stay || panel.closed {
//!             messages.push(QuitMessage::Stay);
//!         } else if quit {
//!             messages.push(QuitMessage::Quit);
//!         }
//!     }
//!
//!     fn update(
//!         &mut self,
//!         message: QuitMessage,
//!         _update: &mut Update<'_, Infallible>,
//!     ) -> Status<()> {
//!         match message {
//!             QuitMessage::Quit => Status::Done(()),
//!             QuitMessage::Stay => Status::Closed,
//!         }
//!     }
//! }
//! ```

use std::convert::Infallible;

use i18n::Catalog;
use stack_vec::StackVec;

use crate::{
    Rect,
    ui::{UiContext, UiId},
};

/// The answer of a component to its owner after a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status<T> {
    /// The component stays open.
    Open,
    /// The component closes without a result.
    Closed,
    /// The component closes with a result.
    Done(T),
}

impl<T> Status<T> {
    /// The end of the component, or `None` while it stays open.
    pub fn ended(self) -> Option<Ended<T>> {
        match self {
            Status::Open => None,
            Status::Closed => Some(Ended::Closed),
            Status::Done(output) => Some(Ended::Done(output)),
        }
    }
}

/// How a child ended: the part of a [`Status`] that its owner receives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended<T> {
    /// The child closed without a result.
    Closed,
    /// The child closed with a result.
    Done(T),
}

/// An event of a child for its owner: an effect while it stays open, or its
/// end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildEvent<E, O> {
    /// The child reports an effect.
    Effect(E),
    /// The child ended.
    Ended(Ended<O>),
}

impl<O> ChildEvent<Infallible, O> {
    /// The end of a child without effects.
    pub fn into_ended(self) -> Ended<O> {
        match self {
            ChildEvent::Effect(never) => match never {},
            ChildEvent::Ended(ended) => ended,
        }
    }
}

/// The layer a component draws in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// The layer of the owner. Screens and the root component.
    Owner,
    /// A modal layer that [`run`] opens around the component and closes with
    /// its end.
    Modal(&'static str),
    /// A modal layer that a widget in the view opens and closes itself, such
    /// as [`crate::ui::ConfirmDialog`].
    Widget(&'static str),
}

impl Layer {
    /// The id of the modal layer, or `None` for the layer of the owner.
    fn modal_id(self) -> Option<UiId> {
        match self {
            Layer::Owner => None,
            Layer::Modal(id) | Layer::Widget(id) => Some(UiId::from(id)),
        }
    }
}

/// A screen or a dialog: a view that emits messages and an update that
/// applies them.
pub trait Component {
    /// What the user can ask this component to do.
    type Message;
    /// The result that the owner receives in [`Ended::Done`].
    type Output;
    /// What the component reports to its owner while it stays open.
    type Effect;
    /// Borrowed data of the owner for one pass. `()` for most components.
    type Input<'a>;

    /// The layer the component draws in. A screen draws in the layer of its
    /// owner.
    const LAYER: Layer = Layer::Owner;

    /// The reusable message buffer of the component.
    fn messages(&mut self) -> &mut Vec<Self::Message>;

    /// Draws one pass in `area` and pushes the messages of the pass to
    /// `messages`.
    fn view(
        &mut self,
        context: &mut UiContext,
        area: Rect,
        input: Self::Input<'_>,
        messages: &mut Vec<Self::Message>,
    );

    /// Applies one message.
    fn update(
        &mut self,
        message: Self::Message,
        update: &mut Update<'_, Self::Effect>,
    ) -> Status<Self::Output>;
}

/// The most modal layers that the updates of one pass of a component open.
const MAXIMUM_LAYER_REQUESTS: usize = 4;

/// The modal layers that the updates of a pass open for the next pass, in a
/// buffer of fixed capacity.
#[derive(Debug, Default)]
pub struct LayerRequests {
    /// The ids of the requested layers, each once.
    ids: StackVec<UiId, MAXIMUM_LAYER_REQUESTS>,
}

impl LayerRequests {
    /// No requested layer.
    pub fn new() -> Self {
        Self::default()
    }

    /// The ids of the requested layers in the order of their requests.
    pub fn ids(&self) -> &[UiId] {
        &self.ids
    }

    /// Requests the layer `id`. A layer requested already keeps its place.
    /// More than four layers in one pass are a programming error and panic.
    fn request(&mut self, id: UiId) {
        if !self.ids.contains(&id) {
            self.ids.push(id);
        }
    }
}

/// The access of an update to the pass.
pub struct Update<'a, E> {
    /// The texts of the pass.
    catalog: &'a Catalog,
    /// The receiver of the effects.
    emit: &'a mut dyn FnMut(E),
    /// The layers that open for the next pass.
    layers: &'a mut LayerRequests,
}

impl<'a, E> Update<'a, E> {
    /// The access of an update to the texts `catalog`, with `emit` as the
    /// receiver of its effects and `layers` for the layers it opens.
    pub fn new(
        catalog: &'a Catalog,
        emit: &'a mut dyn FnMut(E),
        layers: &'a mut LayerRequests,
    ) -> Self {
        Self {
            catalog,
            emit,
            layers,
        }
    }

    /// The texts of the pass, for messages that format names.
    pub fn catalog(&self) -> &Catalog {
        self.catalog
    }

    /// Reports `effect` to the owner.
    pub fn emit(&mut self, effect: E) {
        (self.emit)(effect);
    }

    /// Opens the modal layer of the child component `C` for the next pass.
    /// An update that sets an overlay opens the layer of the new child with
    /// it. For a `C` in the layer of its owner it does nothing. A layer
    /// requested already in this pass keeps its place. A fifth distinct layer
    /// in one pass panics, because the requests sit in a buffer of fixed
    /// capacity.
    pub fn open<C: Component>(&mut self) {
        if let Some(id) = C::LAYER.modal_id() {
            self.layers.request(id);
        }
    }
}

/// Runs one pass of `component` in `area`: its view, then its update for
/// each message of the view, in order. The effects of the updates go to
/// `emit`. A pass with messages asks for another pass while the component
/// stays open.
///
/// With [`Layer::Modal`], the component draws in that modal layer, and the
/// layer closes when the component ends. The first update that ends the
/// component ends the pass, and the messages after it are dropped. A debug
/// build panics on them, because a view emits the message that ends its
/// component last.
pub fn run<C: Component>(
    component: &mut C,
    context: &mut UiContext,
    area: Rect,
    input: C::Input<'_>,
    emit: &mut dyn FnMut(C::Effect),
) -> Status<C::Output> {
    match C::LAYER {
        Layer::Modal(id) => context.layer(id, |context| {
            let status = run_pass(component, context, area, input, emit);
            if !matches!(status, Status::Open) {
                context.close_layer();
            }
            status
        }),
        Layer::Owner | Layer::Widget(_) => run_pass(component, context, area, input, emit),
    }
}

/// Runs the view and the updates of one pass of `component` in the current
/// layer, opens the layers that the updates request for the next pass and
/// asks for another pass when the view emitted messages and the component
/// stays open.
fn run_pass<C: Component>(
    component: &mut C,
    context: &mut UiContext,
    area: Rect,
    input: C::Input<'_>,
    emit: &mut dyn FnMut(C::Effect),
) -> Status<C::Output> {
    let mut messages = std::mem::take(component.messages());
    messages.clear();
    component.view(context, area, input, &mut messages);
    let changed = !messages.is_empty();
    let mut layers = LayerRequests::new();
    let mut status = Status::Open;
    let mut pending = messages.drain(..);
    for message in pending.by_ref() {
        let mut update = Update::new(context.catalog, &mut *emit, &mut layers);
        status = component.update(message, &mut update);
        if !matches!(status, Status::Open) {
            break;
        }
    }
    debug_assert_eq!(
        pending.len(),
        0,
        "a view emits the message that ends its component last"
    );
    drop(pending);
    *component.messages() = messages;
    for &id in layers.ids() {
        context.open_layer_for_next_pass(id);
    }
    if changed && matches!(status, Status::Open) {
        context.request_pass();
    }
    status
}

/// Runs one pass of the child `child` in `area` from the view of its owner.
/// Each effect of the child and its end become messages of the owner
/// through `map`, pushed to `messages`. Returns whether the child stays
/// open. An open child adds no message besides its effects.
pub fn run_child<C: Component, M>(
    child: &mut C,
    context: &mut UiContext,
    area: Rect,
    input: C::Input<'_>,
    messages: &mut Vec<M>,
    map: impl Fn(ChildEvent<C::Effect, C::Output>) -> M,
) -> bool {
    let status = run(child, context, area, input, &mut |effect| {
        messages.push(map(ChildEvent::Effect(effect)));
    });
    match status.ended() {
        None => true,
        Some(ended) => {
            messages.push(map(ChildEvent::Ended(ended)));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::*;
    use crate::ui::{
        ConfirmDialog, ConfirmLines, Navigation, NavigationSet,
        tests::{Harness, test_catalog},
    };

    /// The area of the components of the tests.
    const AREA: Rect = Rect::new(0.0, 0.0, 800.0, 600.0);

    /// The area of the button below the dialogs.
    const BELOW: Rect = Rect::new(0.0, 0.0, 100.0, 30.0);

    /// The area of the cancel button of the dialog.
    const FIRST: Rect = Rect::new(200.0, 200.0, 100.0, 30.0);

    /// The area of the save button of the dialog.
    const SECOND: Rect = Rect::new(320.0, 200.0, 100.0, 30.0);

    /// Draws a focusable button with the id `label` in `rect` and returns
    /// whether a click or confirm activated it.
    fn button(context: &mut UiContext, label: &str, rect: Rect) -> bool {
        let id = UiId::from(label);
        context.focusable(id, rect, NavigationSet::EMPTY);
        context.button_behavior(id, rect).clicked || context.confirmed(id)
    }

    /// A message of the [`Script`] component.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Step {
        /// Stays open.
        Record(u32),
        /// Reports the effect.
        Effect(u32),
        /// Closes without a result.
        Close,
        /// Closes with the result.
        Finish(u32),
    }

    /// A screen whose view emits the queued steps.
    #[derive(Default)]
    struct Script {
        /// The message buffer.
        messages: Vec<Step>,
        /// The steps that the next view emits.
        queued: Vec<Step>,
        /// The steps that the updates applied.
        applied: Vec<Step>,
    }

    impl Component for Script {
        type Message = Step;
        type Output = u32;
        type Effect = u32;
        type Input<'a> = ();

        fn messages(&mut self) -> &mut Vec<Step> {
            &mut self.messages
        }

        fn view(
            &mut self,
            _context: &mut UiContext,
            _area: Rect,
            _input: (),
            messages: &mut Vec<Step>,
        ) {
            messages.append(&mut self.queued);
        }

        fn update(&mut self, message: Step, update: &mut Update<'_, u32>) -> Status<u32> {
            self.applied.push(message);
            match message {
                Step::Record(_) => Status::Open,
                Step::Effect(effect) => {
                    update.emit(effect);
                    Status::Open
                }
                Step::Close => Status::Closed,
                Step::Finish(output) => Status::Done(output),
            }
        }
    }

    /// Runs one pass of `script` with the steps `queued` in `harness`.
    /// Returns the status and the effects of the pass.
    fn run_script(
        harness: &mut Harness,
        script: &mut Script,
        queued: &[Step],
    ) -> (Status<u32>, Vec<u32>) {
        script.queued.extend_from_slice(queued);
        let mut effects = Vec::new();
        let status = harness.frame(|context| {
            run(script, context, AREA, (), &mut |effect| {
                effects.push(effect)
            })
        });
        (status, effects)
    }

    /// Verifies that the updates apply the messages of a pass in the order of the view and that
    /// the effects reach the receiver in that order.
    #[test]
    fn messages_apply_in_order() {
        let mut harness = Harness::new();
        let mut script = Script::default();
        let steps = [
            Step::Record(1),
            Step::Effect(2),
            Step::Record(3),
            Step::Effect(4),
        ];
        let (status, effects) = run_script(&mut harness, &mut script, &steps);
        assert_eq!(status, Status::Open);
        assert_eq!(script.applied, steps);
        assert_eq!(effects, [2, 4]);
        let (status, _) = run_script(&mut harness, &mut script, &[Step::Record(5), Step::Close]);
        assert_eq!(status, Status::Closed);
    }

    /// Verifies that the first update that ends the component ends the pass, and that a debug
    /// build reports the dropped messages after it.
    #[test]
    fn an_end_stops_the_remaining_messages() {
        let mut harness = Harness::new();
        let mut script = Script::default();
        let steps = [Step::Record(1), Step::Finish(9), Step::Effect(2)];
        let result = catch_unwind(AssertUnwindSafe(|| {
            run_script(&mut harness, &mut script, &steps)
        }));
        assert_eq!(script.applied, [Step::Record(1), Step::Finish(9)]);
        if cfg!(debug_assertions) {
            let payload = result.expect_err("a debug build reports the dropped message");
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .unwrap_or_default();
            assert!(message.contains("a view emits the message that ends its component last"));
        } else {
            assert_eq!(result.ok(), Some((Status::Done(9), Vec::new())));
        }
    }

    /// Verifies that a pass with messages asks for another pass and that a pass without messages
    /// does not.
    #[test]
    fn a_pass_with_messages_asks_for_another_pass() {
        let mut harness = Harness::new();
        let mut script = Script::default();
        run_script(&mut harness, &mut script, &[]);
        assert!(!harness.state.take_pass_request());
        run_script(&mut harness, &mut script, &[Step::Record(1)]);
        assert!(harness.state.take_pass_request());
        run_script(&mut harness, &mut script, &[]);
        assert!(!harness.state.take_pass_request());
    }

    /// Verifies that a pass whose update ends the component asks for no other pass, and that
    /// an open component with messages still asks for one.
    #[test]
    fn a_pass_that_ends_the_component_asks_for_no_other_pass() {
        let mut harness = Harness::new();
        let mut script = Script::default();
        let (status, _) = run_script(&mut harness, &mut script, &[Step::Record(1), Step::Close]);
        assert_eq!(status, Status::Closed);
        assert!(!harness.state.take_pass_request());
        let (status, _) = run_script(&mut harness, &mut script, &[Step::Finish(3)]);
        assert_eq!(status, Status::Done(3));
        assert!(!harness.state.take_pass_request());
        let (status, _) = run_script(&mut harness, &mut script, &[Step::Record(2)]);
        assert_eq!(status, Status::Open);
        assert!(harness.state.take_pass_request());
    }

    /// A screen that runs a [`Script`] as its child.
    #[derive(Default)]
    struct ScriptOwner {
        /// The message buffer.
        messages: Vec<ChildEvent<u32, u32>>,
        /// The child.
        child: Script,
        /// The messages that the updates applied.
        applied: Vec<ChildEvent<u32, u32>>,
    }

    impl Component for ScriptOwner {
        type Message = ChildEvent<u32, u32>;
        type Output = Infallible;
        type Effect = Infallible;
        type Input<'a> = ();

        fn messages(&mut self) -> &mut Vec<ChildEvent<u32, u32>> {
            &mut self.messages
        }

        fn view(
            &mut self,
            context: &mut UiContext,
            area: Rect,
            _input: (),
            messages: &mut Vec<ChildEvent<u32, u32>>,
        ) {
            run_child(&mut self.child, context, area, (), messages, |event| event);
        }

        fn update(
            &mut self,
            message: ChildEvent<u32, u32>,
            _update: &mut Update<'_, Infallible>,
        ) -> Status<Infallible> {
            self.applied.push(message);
            Status::Open
        }
    }

    /// Verifies that the end of a child asks for another pass of its owner.
    #[test]
    fn an_owner_whose_child_ends_asks_for_another_pass() {
        let mut harness = Harness::new();
        let mut owner = ScriptOwner::default();
        owner.child.queued.push(Step::Finish(5));
        let status = harness
            .frame(|context| run(&mut owner, context, AREA, (), &mut |never| match never {}));
        assert_eq!(status, Status::Open);
        assert_eq!(owner.applied, [ChildEvent::Ended(Ended::Done(5))]);
        assert!(harness.state.take_pass_request());
    }

    /// A message of the [`Dialog`] component.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum DialogMessage {
        /// The cancel button.
        Cancel,
        /// The save button.
        Save,
    }

    /// A dialog in a modal layer whose save can fail.
    #[derive(Default)]
    struct Dialog {
        /// The message buffer.
        messages: Vec<DialogMessage>,
        /// Whether the next save fails.
        fail: bool,
    }

    impl Component for Dialog {
        type Message = DialogMessage;
        type Output = u32;
        type Effect = u32;
        type Input<'a> = ();

        const LAYER: Layer = Layer::Modal("dialog");

        fn messages(&mut self) -> &mut Vec<DialogMessage> {
            &mut self.messages
        }

        fn view(
            &mut self,
            context: &mut UiContext,
            _area: Rect,
            _input: (),
            messages: &mut Vec<DialogMessage>,
        ) {
            if button(context, "first", FIRST) {
                messages.push(DialogMessage::Cancel);
            }
            if button(context, "second", SECOND) {
                messages.push(DialogMessage::Save);
            }
        }

        fn update(&mut self, message: DialogMessage, update: &mut Update<'_, u32>) -> Status<u32> {
            match message {
                DialogMessage::Cancel => Status::Closed,
                DialogMessage::Save if self.fail => Status::Open,
                DialogMessage::Save => {
                    update.emit(1);
                    Status::Done(7)
                }
            }
        }
    }

    /// Verifies that `run` opens the modal layer of a component, keeps it while the component
    /// stays open, keeps the focus after a failing update and closes the layer with the end.
    #[test]
    fn run_opens_and_closes_the_modal_layer() {
        let mut harness = Harness::new();
        let mut dialog = Dialog::default();
        let mut effects = Vec::new();
        let mut pass = |harness: &mut Harness, dialog: &mut Dialog| {
            harness.frame(|context| {
                button(context, "below", BELOW);
                run(dialog, context, AREA, (), &mut |effect| {
                    effects.push(effect)
                })
            })
        };
        harness.state.set_focus(UiId::from("below"));
        assert_eq!(pass(&mut harness, &mut dialog), Status::Open);
        assert_eq!(harness.state.open_modals(), 1);
        assert_eq!(harness.state.focused(), Some(UiId::from("first")));
        harness.navigate(Navigation::Next);
        pass(&mut harness, &mut dialog);
        assert_eq!(harness.state.focused(), Some(UiId::from("second")));
        dialog.fail = true;
        harness.input.confirm = true;
        assert_eq!(pass(&mut harness, &mut dialog), Status::Open);
        assert_eq!(harness.state.open_modals(), 1);
        assert_eq!(harness.state.focused(), Some(UiId::from("second")));
        dialog.fail = false;
        harness.input.confirm = true;
        assert_eq!(pass(&mut harness, &mut dialog), Status::Done(7));
        assert_eq!(harness.state.open_modals(), 0);
        assert_eq!(harness.state.focused(), Some(UiId::from("below")));
        assert_eq!(effects, [1]);
    }

    /// A message of the [`Question`] component.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum QuestionMessage {
        /// The action button.
        Confirm,
        /// The cancel button or a cancel.
        Cancel,
    }

    /// A question that a [`ConfirmDialog`] draws in its own layer.
    #[derive(Default)]
    struct Question {
        /// The message buffer.
        messages: Vec<QuestionMessage>,
        /// The wrapped lines of the dialog.
        lines: ConfirmLines,
        /// Whether the view started blocked in the last pass.
        blocked: bool,
    }

    impl Component for Question {
        type Message = QuestionMessage;
        type Output = ();
        type Effect = Infallible;
        type Input<'a> = ();

        const LAYER: Layer = Layer::Widget("question");

        fn messages(&mut self) -> &mut Vec<QuestionMessage> {
            &mut self.messages
        }

        fn view(
            &mut self,
            context: &mut UiContext,
            _area: Rect,
            _input: (),
            messages: &mut Vec<QuestionMessage>,
        ) {
            self.blocked = context.is_blocked();
            let response = ConfirmDialog::new("question", "Question", "Go on?", "Cancel", "OK")
                .show(context, &mut self.lines);
            if response.confirmed {
                messages.push(QuestionMessage::Confirm);
            } else if response.cancelled {
                messages.push(QuestionMessage::Cancel);
            }
        }

        fn update(
            &mut self,
            message: QuestionMessage,
            _update: &mut Update<'_, Infallible>,
        ) -> Status<()> {
            match message {
                QuestionMessage::Confirm => Status::Done(()),
                QuestionMessage::Cancel => Status::Closed,
            }
        }
    }

    /// Verifies that with `Layer::Widget` the view draws in the layer of the owner and the widget
    /// opens and closes its own layer.
    #[test]
    fn run_leaves_a_widget_layer_to_the_widget() {
        let mut harness = Harness::new();
        let mut question = Question::default();
        let pass = |harness: &mut Harness, question: &mut Question| {
            harness.frame(|context| run(question, context, AREA, (), &mut |never| match never {}))
        };
        assert_eq!(pass(&mut harness, &mut question), Status::Open);
        assert_eq!(harness.state.open_modals(), 1);
        assert_eq!(pass(&mut harness, &mut question), Status::Open);
        assert!(question.blocked);
        assert_eq!(harness.state.open_modals(), 1);
        harness.input.cancel = true;
        assert_eq!(pass(&mut harness, &mut question), Status::Closed);
        assert_eq!(harness.state.open_modals(), 0);
    }

    /// Verifies that `Update::open` requests the modal layer of a child once, for `Layer::Modal`
    /// and `Layer::Widget`, and nothing for `Layer::Owner`.
    #[test]
    fn update_open_requests_the_modal_layer_of_the_child_once() {
        let mut layers = LayerRequests::new();
        let mut emit = |_: u32| {};
        let mut update = Update::new(test_catalog(), &mut emit, &mut layers);
        update.open::<Script>();
        update.open::<Dialog>();
        update.open::<Question>();
        update.open::<Dialog>();
        assert_eq!(layers.ids(), [UiId::from("dialog"), UiId::from("question")]);
    }

    /// A message of the [`Opener`] component.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum OpenerMessage {
        /// The button below opens the dialog.
        Open,
        /// An event of the dialog.
        Dialog(ChildEvent<u32, u32>),
    }

    /// A screen with a button that opens a [`Dialog`].
    #[derive(Default)]
    struct Opener {
        /// The message buffer.
        messages: Vec<OpenerMessage>,
        /// The open dialog.
        dialog: Option<Dialog>,
        /// The messages that the updates applied.
        applied: Vec<OpenerMessage>,
        /// Whether the dialog stayed open in the last pass.
        dialog_open: bool,
    }

    impl Component for Opener {
        type Message = OpenerMessage;
        type Output = Infallible;
        type Effect = Infallible;
        type Input<'a> = ();

        fn messages(&mut self) -> &mut Vec<OpenerMessage> {
            &mut self.messages
        }

        fn view(
            &mut self,
            context: &mut UiContext,
            area: Rect,
            _input: (),
            messages: &mut Vec<OpenerMessage>,
        ) {
            if button(context, "below", BELOW) {
                messages.push(OpenerMessage::Open);
            }
            self.dialog_open = match self.dialog.as_mut() {
                Some(dialog) => {
                    run_child(dialog, context, area, (), messages, OpenerMessage::Dialog)
                }
                None => false,
            };
        }

        fn update(
            &mut self,
            message: OpenerMessage,
            update: &mut Update<'_, Infallible>,
        ) -> Status<Infallible> {
            self.applied.push(message);
            match message {
                OpenerMessage::Open => {
                    self.dialog = Some(Dialog::default());
                    update.open::<Dialog>();
                }
                OpenerMessage::Dialog(ChildEvent::Effect(_)) => {}
                OpenerMessage::Dialog(ChildEvent::Ended(_)) => self.dialog = None,
            }
            Status::Open
        }
    }

    /// Runs one pass of `opener` in `harness`.
    fn run_opener(harness: &mut Harness, opener: &mut Opener) {
        let status =
            harness.frame(|context| run(opener, context, AREA, (), &mut |never| match never {}));
        assert_eq!(status, Status::Open);
    }

    /// Verifies that an update that opens a child blocks the widgets below the layer of the
    /// child from the start of the next pass, and that a key of that pass stays out of the base.
    #[test]
    fn update_open_blocks_the_base_from_the_next_pass() {
        let mut harness = Harness::new();
        let mut opener = Opener::default();
        harness.state.set_focus(UiId::from("below"));
        run_opener(&mut harness, &mut opener);
        harness.input.confirm = true;
        run_opener(&mut harness, &mut opener);
        assert_eq!(opener.applied, [OpenerMessage::Open]);
        assert!(opener.dialog.is_some());
        assert!(!opener.dialog_open);
        assert_eq!(harness.state.open_modals(), 1);
        assert!(harness.state.take_pass_request());
        harness.input.confirm = true;
        run_opener(&mut harness, &mut opener);
        assert_eq!(opener.applied, [OpenerMessage::Open]);
        assert!(opener.dialog_open);
        assert_eq!(harness.state.open_modals(), 1);
        assert_eq!(harness.state.focused(), Some(UiId::from("first")));
    }

    /// Verifies that `run_child` turns the effects and the end of a child into messages of its
    /// owner and returns whether the child stays open, and that an open child adds no message.
    #[test]
    fn run_child_turns_the_events_of_a_child_into_messages() {
        let mut harness = Harness::new();
        let mut opener = Opener::default();
        harness.state.set_focus(UiId::from("below"));
        run_opener(&mut harness, &mut opener);
        harness.input.confirm = true;
        run_opener(&mut harness, &mut opener);
        run_opener(&mut harness, &mut opener);
        run_opener(&mut harness, &mut opener);
        assert!(opener.dialog_open);
        assert!(!harness.state.take_pass_request());
        assert_eq!(opener.applied, [OpenerMessage::Open]);
        harness.navigate(Navigation::Next);
        run_opener(&mut harness, &mut opener);
        harness.input.confirm = true;
        run_opener(&mut harness, &mut opener);
        assert_eq!(
            opener.applied,
            [
                OpenerMessage::Open,
                OpenerMessage::Dialog(ChildEvent::Effect(1)),
                OpenerMessage::Dialog(ChildEvent::Ended(Ended::Done(7))),
            ]
        );
        assert!(!opener.dialog_open);
        assert!(opener.dialog.is_none());
        assert_eq!(harness.state.open_modals(), 0);
        assert_eq!(harness.state.focused(), Some(UiId::from("below")));
    }

    /// Verifies that the end of a child without effects maps to its `Ended` and that only an
    /// open status has no end.
    #[test]
    fn a_child_without_effects_maps_its_end() {
        let event: ChildEvent<Infallible, u32> = ChildEvent::Ended(Ended::Done(3));
        assert_eq!(event.into_ended(), Ended::Done(3));
        assert_eq!(Status::<u32>::Open.ended(), None);
        assert_eq!(Status::<u32>::Closed.ended(), Some(Ended::Closed));
    }

    /// Verifies that a steady pass of an owner with an open child allocates nothing.
    #[test]
    fn a_steady_pass_allocates_nothing() {
        let mut harness = Harness::new();
        let mut opener = Opener::default();
        harness.state.set_focus(UiId::from("below"));
        run_opener(&mut harness, &mut opener);
        harness.input.confirm = true;
        for _ in 0..4 {
            run_opener(&mut harness, &mut opener);
        }
        assert!(opener.dialog_open);
        let (allocations, ()) =
            allocation_counter::count_allocations(|| run_opener(&mut harness, &mut opener));
        assert_eq!(allocations, 0);
        assert!(!harness.state.take_pass_request());
    }
}
