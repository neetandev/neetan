//! VM data layer: closures, call frames, the return-action model, the frame and
//! register stacks, continuations, dynamic-wind records, and result packets.

use std::rc::Rc;

use crate::{
    Error, ErrorKind, Value,
    bytecode::{Chunk, ExpectedResults},
};

#[derive(Clone, Debug)]
pub(crate) struct Closure {
    pub(crate) chunk: Rc<Chunk>,
    pub(crate) captures: Rc<[Value]>,
}

#[derive(Clone, Debug)]
pub(crate) struct Frame {
    pub(crate) chunk: Rc<Chunk>,
    pub(crate) pc: usize,
    pub(crate) base: usize,
    pub(crate) top: usize,
    pub(crate) return_base: usize,
    pub(crate) expected: ExpectedResults,
    pub(crate) captures: Rc<[Value]>,
    pub(crate) procedure: Value,
    // `Normal` (every ordinary call) is `None`, so the common frame carries no
    // 32-byte action inline and allocates nothing; the rare cold-path actions
    // are boxed. Keeps `Frame` small and cache-dense for the per-call in-place
    // write and the GC frame scan.
    pub(super) return_action: Option<Box<ReturnAction>>,
}

/// Boxes a return action for storage in a `Frame`; `Normal` stores as `None`.
pub(super) fn boxed_action(action: ReturnAction) -> Option<Box<ReturnAction>> {
    match action {
        ReturnAction::Normal => None,
        other => Some(Box::new(other)),
    }
}

/// Recovers a frame's return action, mapping the `None` (common) case to
/// `Normal`.
pub(super) fn unbox_action(action: Option<Box<ReturnAction>>) -> ReturnAction {
    action.map_or(ReturnAction::Normal, |boxed| *boxed)
}

// The overwhelmingly common return action is `Normal` (every ordinary call
// frame). The rare variants carrying large payloads are boxed so `ReturnAction`,
// and therefore every `Frame` on the call stack, stays cache-dense on deep
// recursion and cheap to clone during `call/cc` capture. The extra allocation
// only happens on the cold dynamic-wind / parameterize / continuation-transfer /
// exit paths.
#[derive(Clone, Debug)]
pub(super) enum ReturnAction {
    Normal,
    InvokeConsumer(Value),
    StorePromise { promise: Value, flatten: bool },
    CreateParameter { converter: Value },
    RaiseReturned,
    ReinstallHandler(Value),
    StartWind(Box<StartWindData>),
    FinishWind(Box<Wind>),
    RestoreResults(Vec<Value>),
    ClosePort(crate::port::PortId),
    RestorePort(Box<RestorePortData>),
    LoadComplete,
    ConvertedParameter(Box<ConvertedParameterData>),
    ContinueTransfer(Box<ContinueTransferData>),
    ExitCleanup(Box<ExitCleanupData>),
}

#[derive(Clone, Debug)]
pub(super) struct StartWindData {
    pub(super) thunk: Value,
    pub(super) wind: Wind,
}

#[derive(Clone, Debug)]
pub(super) struct RestorePortData {
    pub(super) port: crate::port::PortId,
    pub(super) parameter: Value,
    pub(super) old: Value,
}

#[derive(Clone, Debug)]
pub(super) struct ConvertedParameterData {
    pub(super) call_base: usize,
    pub(super) parameter: Value,
    pub(super) old: Value,
    pub(super) remaining: Vec<(Value, Value, Value)>,
    pub(super) converted: Vec<(Value, Value, Value)>,
}

#[derive(Clone, Debug)]
pub(super) struct ContinueTransferData {
    pub(super) call_base: usize,
    pub(super) thunks: Vec<Value>,
    pub(super) continuation: Continuation,
    pub(super) values: Vec<Value>,
}

#[derive(Clone, Debug)]
pub(super) struct ExitCleanupData {
    pub(super) call_base: usize,
    pub(super) remaining: Vec<Value>,
    pub(super) status: crate::ExitStatus,
}

const _: () = assert!(
    size_of::<ReturnAction>() <= 32,
    "ReturnAction grew. Box the newly-large variant to keep Frame cache-dense",
);

impl ReturnAction {
    /// Appends every `Value` held by a pending return action. These live only
    /// in the action (a Rust-owned field), not on the register stack, so a
    /// precise scan must include them. `ContinueTransfer` embeds a whole
    /// continuation and recurses into [`Continuation::trace`]. That recursion
    /// mirrors the ownership tree `Drop` already walks, so it introduces no
    /// new depth hazard.
    pub(super) fn trace(&self, out: &mut Vec<Value>) {
        match self {
            Self::Normal | Self::RaiseReturned | Self::LoadComplete | Self::ClosePort(_) => {}
            Self::InvokeConsumer(value)
            | Self::CreateParameter { converter: value }
            | Self::ReinstallHandler(value)
            | Self::StorePromise { promise: value, .. } => out.push(*value),
            Self::StartWind(data) => {
                out.push(data.thunk);
                out.push(data.wind.before);
                out.push(data.wind.after);
            }
            Self::FinishWind(wind) => {
                out.push(wind.before);
                out.push(wind.after);
            }
            Self::RestoreResults(values) => out.extend(values.iter().copied()),
            Self::RestorePort(data) => {
                out.push(data.parameter);
                out.push(data.old);
            }
            Self::ConvertedParameter(data) => {
                out.push(data.parameter);
                out.push(data.old);
                for (a, b, c) in data.remaining.iter().chain(data.converted.iter()) {
                    out.push(*a);
                    out.push(*b);
                    out.push(*c);
                }
            }
            Self::ContinueTransfer(data) => {
                out.extend(data.thunks.iter().copied());
                out.extend(data.values.iter().copied());
                data.continuation.trace(out);
            }
            Self::ExitCleanup(data) => out.extend(data.remaining.iter().copied()),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FrameStack {
    /// Grow-only frame arena (Lua-`CallInfo`-style). Only `buffer[..depth]` is
    /// live; the tail above `depth` retains dead frames for reuse so an ordinary
    /// call writes fields directly into a recycled slot rather than constructing
    /// and copying a fresh `Frame`. The dead tail is never scanned or read.
    pub(super) buffer: Vec<Frame>,
    pub(super) depth: usize,
    pub(super) handlers: Vec<Value>,
    pub(super) parameters: Vec<(Value, Value)>,
    pub(super) winds: Vec<Wind>,
    pub(super) next_wind: u64,
}

/// The return-time state salvaged from a frame as it is popped. The rest of the
/// `Frame` stays physically in the grow-only buffer above the new `depth`.
pub(super) struct PoppedFrame {
    pub(super) return_base: usize,
    pub(super) expected: ExpectedResults,
    pub(super) return_action: Option<Box<ReturnAction>>,
}

impl FrameStack {
    pub(crate) fn trace_locals(&self, _: &mut Vec<Value>) {}
    pub(crate) const fn locals_bytes(&self) -> usize {
        0
    }

    /// Bytes retained by the frame arena (used for continuation size accounting).
    pub(crate) fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.depth == 0
    }

    pub(super) fn last(&self) -> Option<&Frame> {
        self.depth.checked_sub(1).map(|index| &self.buffer[index])
    }

    pub(super) fn last_mut(&mut self) -> Option<&mut Frame> {
        let index = self.depth.checked_sub(1)?;
        Some(&mut self.buffer[index])
    }

    pub(super) fn iter(&self) -> std::slice::Iter<'_, Frame> {
        self.buffer[..self.depth].iter()
    }

    /// Reserves the next frame slot for in-place initialization and returns it.
    /// Extends the buffer (cloning the current top as filler, which the caller
    /// fully overwrites) only when the call depth passes its previous high-water
    /// mark; the steady state writes fields into a recycled slot with no
    /// allocation and no per-call `Frame` copy.
    ///
    /// Dead-slot invariant: every slot at or above `depth` holds
    /// `return_action == None`, so the hot frame-push paths skip the action
    /// write (and its drop check) entirely. It holds globally because the only
    /// ways a slot leaves the live region are [`Self::pop_frame`] (which
    /// `take`s the action) and a wholesale continuation-restore replacement
    /// with a truncated snapshot (no dead tail). The grow path below is the
    /// one place a stale action could otherwise enter a dead slot.
    ///
    /// Always inlined into the frame-push helpers (not the dispatch loop):
    /// the hot half is a compare, an increment, and an indexed borrow.
    #[inline(always)]
    pub(super) fn reserve(&mut self) -> &mut Frame {
        if self.depth == self.buffer.len() {
            std::hint::cold_path();
            self.grow();
        }
        let index = self.depth;
        self.depth += 1;
        &mut self.buffer[index]
    }

    #[cold]
    #[inline(never)]
    fn grow(&mut self) {
        let mut filler = self.buffer[self.depth - 1].clone();
        // The caller's live frame may carry a boxed return action; clear the
        // clone so the fresh dead slot keeps the invariant documented on
        // `reserve`.
        filler.return_action = None;
        self.buffer.push(filler);
    }

    /// Drops the top frame, returning its return-time state.
    pub(super) fn pop_frame(&mut self) -> PoppedFrame {
        let index = self.depth - 1;
        let frame = &mut self.buffer[index];
        let popped = PoppedFrame {
            return_base: frame.return_base,
            expected: frame.expected,
            return_action: frame.return_action.take(),
        };
        self.depth = index;
        popped
    }

    /// Captures a compact copy of the live frames for a continuation, excluding
    /// the grow-only dead tail so the snapshot's length equals its live depth.
    pub(super) fn snapshot(&self) -> Self {
        Self {
            buffer: self.buffer[..self.depth].to_vec(),
            depth: self.depth,
            handlers: self.handlers.clone(),
            parameters: self.parameters.clone(),
            winds: self.winds.clone(),
            next_wind: self.next_wind,
        }
    }
}

impl<'a> IntoIterator for &'a FrameStack {
    type Item = &'a Frame;
    type IntoIter = std::slice::Iter<'a, Frame>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RegisterStack(pub(super) Vec<Value>);

impl RegisterStack {
    /// Creates a register file with room for a full frame's registers already
    /// reserved, so the first `execute` call does not pay a heap allocation.
    pub(crate) fn preallocated() -> Self {
        Self(Vec::with_capacity(crate::bytecode::MAX_REGISTERS + 1))
    }

    pub(crate) fn trace(&self, worklist: &mut Vec<Value>) {
        worklist.extend(&self.0);
    }

    pub(crate) fn bytes(&self) -> usize {
        self.0.capacity() * size_of::<Value>()
    }

    // Always inlined: this length compare sits on the per-call and per-return
    // fast paths, where it is a never-taken branch in steady state (the file
    // is grow-only, so it only fires while descending to a new maximum depth).
    #[inline(always)]
    pub(super) fn ensure(&mut self, len: usize) {
        if self.0.len() < len {
            self.grow(len);
        }
    }

    #[cold]
    #[inline(never)]
    fn grow(&mut self, len: usize) {
        self.0.resize(len, Value::unspecified());
    }

    /// Clones the live prefix `[0, len)` of the register file. Used to snapshot
    /// a continuation without copying the grow-only dead tail above `len`.
    pub(super) fn snapshot(&self, len: usize) -> Self {
        Self(self.0[..len.min(self.0.len())].to_vec())
    }

    pub(super) fn get(&self, index: usize) -> Result<Value, Error> {
        self.0.get(index).copied().ok_or_else(|| {
            Error::plain(
                ErrorKind::InvalidBytecode,
                format!(
                    "invalid register {index} with stack length {}",
                    self.0.len()
                ),
            )
        })
    }
}

impl std::ops::Deref for RegisterStack {
    type Target = Vec<Value>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for RegisterStack {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Continuation {
    pub(crate) frames: FrameStack,
    pub(crate) stack: RegisterStack,
    pub(crate) handlers: Vec<Value>,
    pub(crate) parameters: Vec<(Value, Value)>,
    pub(crate) parameter_values: Vec<(Value, Value)>,
    pub(crate) winds: Vec<Wind>,
    pub(super) destination: usize,
    pub(super) expected: ExpectedResults,
}

impl Continuation {
    /// Appends every `Value` reachable from this continuation: per-frame
    /// captures, procedures, and pending return actions, the captured register
    /// stack, handlers, parameter shadows, cached parameter values, and
    /// dynamic winds. This is the single tracer shared by the VM root
    /// collector (in-flight continuations held by return actions) and the
    /// `Object::Continuation` arm of `Object::trace` (heap-stored
    /// continuations), so the two can never diverge.
    pub(crate) fn trace(&self, out: &mut Vec<Value>) {
        for frame in self.frames.iter() {
            out.extend(frame.captures.iter().copied());
            out.push(frame.procedure);
            if let Some(action) = &frame.return_action {
                action.trace(out);
            }
        }
        self.frames.trace_locals(out);
        self.stack.trace(out);
        out.extend(self.handlers.iter().copied());
        for (parameter, old) in &self.parameters {
            out.push(*parameter);
            out.push(*old);
        }
        for (parameter, value) in &self.parameter_values {
            out.push(*parameter);
            out.push(*value);
        }
        for wind in &self.winds {
            out.push(wind.before);
            out.push(wind.after);
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Wind {
    pub(crate) id: u64,
    pub(crate) before: Value,
    pub(crate) after: Value,
}

#[derive(Clone, Debug)]
pub(crate) enum Results {
    Zero,
    One(Value),
    Many(Vec<Value>),
}

impl Results {
    pub(super) fn into_vec(self) -> Vec<Value> {
        match self {
            Self::Zero => Vec::new(),
            Self::One(value) => vec![value],
            Self::Many(values) => values,
        }
    }
}
