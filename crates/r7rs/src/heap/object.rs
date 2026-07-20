//! Heap object kinds and their GC-trace and byte-accounting behavior.

use crate::Value;

/// A heap object.
///
/// Deliberately not `Clone`: the arena-handle variants (string, bytevector,
/// vector) own their payload span, and a cloned handle would alias one span
/// from two slots, which the sweep's compaction would silently fork.
#[derive(Debug)]
pub(crate) enum Object {
    Pair(Value, Value),
    /// A vector payload span in the heap's value arena. Constructed only by
    /// [`crate::heap::Heap::alloc_vector`], which appends the payload and
    /// charges the byte accounting atomically. The sweep compacts the arena
    /// and rewrites `off` in place, so the handle is only meaningful
    /// together with the owning heap.
    Vector {
        /// Element offset of the payload in the value arena.
        off: u32,
        /// Payload length in elements.
        len: u32,
    },
    /// A bytevector payload span in the heap's byte arena. Constructed only
    /// by [`crate::heap::Heap::alloc_bytevector`], which appends the payload
    /// and charges the byte accounting atomically. The sweep compacts the
    /// arena and rewrites `off` in place, so the handle is only meaningful
    /// together with the owning heap.
    Bytevector {
        /// Byte offset of the payload in the byte arena.
        off: u32,
        /// Payload length in bytes.
        len: u32,
    },
    /// A string payload span in the heap's byte arena, always valid UTF-8.
    /// Constructed only by [`crate::heap::Heap::alloc_string`]; a widening
    /// or narrowing `string-set!` rebuilds the span at the arena tail and
    /// rewrites this handle in place. The sweep compacts the arena, so the
    /// handle is only meaningful together with the owning heap.
    String {
        /// Byte offset of the payload in the byte arena.
        off: u32,
        /// Payload length in bytes.
        byte_len: u32,
        /// Number of Unicode scalar values. Equal to `byte_len` exactly for
        /// all-ASCII contents, which is the indexing fast-path test.
        chars: u32,
    },
    Symbol(String),
    Closure(crate::vm::Closure),
    CaseLambda(Vec<Value>),
    Native {
        /// Index into the engine's native-procedure registry.
        id: u32,
        /// The registry's classified fast path for this procedure, copied
        /// here at registration so the VM's call fast paths read it with the
        /// same arena probe that classifies the callee, instead of a second
        /// registry lookup per call.
        fast: Option<crate::native::FastProcedure>,
        /// Whether the callback's result type always produces one value.
        single_result: bool,
        /// Whether the callback can request process exit through the VM.
        may_exit: bool,
    },
    /// VM-integrated `apply`, which can invoke Scheme closures without a Rust
    /// reentrant call.
    Apply,
    Box(Value),
    Promise(Promise),
    Parameter(Box<Parameter>),
    Continuation(Box<crate::vm::Continuation>),
    Record(Box<Record>),
    RecordType(RecordType),
    RecordProcedure(Box<RecordProcedure>),
    Port(crate::port::PortObject),
    Error(Box<ErrorObject>),
    Environment {
        mutable: bool,
    },
    /// A non-immediate rational or complex numeric value.
    Number(Box<crate::number::RuntimeNumber>),
    /// A SRFI 27 random source. Holds only the 128-bit of Squares generator
    /// state, so it stays inline and owns no GC edges.
    RandomSource(crate::random::SquaresRng),
}

// Structural regression guard: every heap slot is sized to the largest `Object`
// variant, so every payload larger than `Pair`'s two inline `Value`s (the
// unavoidable floor) stays boxed. Any newly-oversized variant should be boxed
// rather than allowed to inflate every slot.
// The exact size is pinned rather than bounded so any regression is loud.
// `Value` has 16-byte alignment, so the enum is one aligned discriminant
// chunk plus `Pair`'s 32-byte payload floor. The arena handles (string,
// bytevector) sit far below that floor, and no other inline payload
// exceeds it.
const _: () = assert!(
    size_of::<Object>() == 48,
    "Object grew. Box the newly-large variant to keep heap slots cache-dense",
);

// The heap stores slots as `Option<Object>` and relies on the niche in the
// enum discriminant keeping the option at the same size. Rustc has always
// provided this optimization here, and this pin makes any regression loud.
const _: () = assert!(
    size_of::<Option<Object>>() == 48,
    "Option<Object> lost its niche. Heap slot density regressed",
);

#[derive(Clone, Debug)]
pub(crate) enum PromiseState {
    Pending { thunk: Value, flatten: bool },
    Forcing { thunk: Value, flatten: bool },
    Done(Vec<Value>),
    Forward(Value),
}

#[derive(Clone, Debug)]
pub(crate) struct Promise {
    pub(crate) state: PromiseState,
}

#[derive(Clone, Debug)]
pub(crate) struct Parameter {
    pub(crate) value: Value,
    pub(crate) converter: Option<Value>,
}

#[derive(Clone, Debug)]
pub(crate) struct ErrorObject {
    pub(crate) message: Value,
    pub(crate) irritants: Vec<Value>,
    pub(crate) kind: ConditionKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConditionKind {
    Error,
    Read,
    File,
}

#[derive(Clone, Debug)]
pub(crate) struct RecordType {
    pub(crate) fields: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct Record {
    pub(crate) record_type: Value,
    pub(crate) fields: Vec<Value>,
}

#[derive(Clone, Debug)]
pub(crate) enum RecordProcedure {
    Constructor {
        record_type: Value,
        fields: usize,
        mapping: Vec<usize>,
    },
    Predicate {
        record_type: Value,
    },
    Accessor {
        record_type: Value,
        field: usize,
    },
    Mutator {
        record_type: Value,
        field: usize,
    },
}

/// One-probe callee classification for the dispatch loop's call fast paths:
/// a single slot resolution and object match distinguish the two hot callable
/// kinds (closure, native) from everything else.
pub(crate) enum Callee<'heap> {
    Closure(&'heap crate::vm::Closure),
    Native {
        id: u32,
        fast: Option<crate::native::FastProcedure>,
        single_result: bool,
        may_exit: bool,
    },
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CallableKind {
    Parameter,
    Continuation,
    Apply,
    Record,
    Native,
    Closure,
    CaseLambda,
    Other,
}

impl Object {
    /// Appends every direct Scheme-value edge to the mark worklist.
    /// `value_arena` is the heap's vector payload arena, which holds the
    /// edges of `Vector` handles.
    ///
    /// Keeping this match exhaustive makes adding an object variant a
    /// compile-time prompt to decide whether it owns any GC edges.
    pub(super) fn trace(&self, worklist: &mut Vec<Value>, value_arena: &[Value]) {
        match self {
            Self::Pair(car, cdr) => worklist.extend([*car, *cdr]),
            Self::Vector { off, len } => {
                // The span lookup cannot fail for a live handle. `get` keeps
                // the contract panic-free regardless.
                if let Some(span) = value_arena.get(*off as usize..*off as usize + *len as usize) {
                    worklist.extend(span);
                }
            }
            Self::Record(record) => {
                worklist.push(record.record_type);
                worklist.extend(&record.fields);
            }
            Self::RecordType(_) => {}
            Self::RecordProcedure(procedure) => match procedure.as_ref() {
                RecordProcedure::Constructor { record_type, .. }
                | RecordProcedure::Predicate { record_type }
                | RecordProcedure::Accessor { record_type, .. }
                | RecordProcedure::Mutator { record_type, .. } => worklist.push(*record_type),
            },
            Self::Number(_) => {}
            Self::Error(error) => {
                worklist.push(error.message);
                worklist.extend(&error.irritants);
            }
            Self::Continuation(continuation) => continuation.trace(worklist),
            Self::Closure(closure) => worklist.extend(&*closure.captures),
            Self::CaseLambda(closures) => {
                worklist.extend(closures);
            }
            Self::Box(value) => worklist.push(*value),
            Self::Parameter(parameter) => {
                worklist.push(parameter.value);
                if let Some(converter) = parameter.converter {
                    worklist.push(converter);
                }
            }
            Self::Promise(Promise {
                state: PromiseState::Pending { thunk, .. },
            }) => worklist.push(*thunk),
            Self::Promise(Promise {
                state: PromiseState::Done(values),
            }) => worklist.extend(values),
            Self::Promise(Promise {
                state: PromiseState::Forcing { thunk, .. },
            }) => worklist.push(*thunk),
            Self::Promise(Promise {
                state: PromiseState::Forward(promise),
            }) => worklist.push(*promise),
            Self::Bytevector { .. }
            | Self::String { .. }
            | Self::Symbol(_)
            | Self::Native { .. }
            | Self::Apply
            | Self::Port(_)
            | Self::Environment { .. }
            | Self::RandomSource(_) => {}
        }
    }

    /// Returns approximate directly owned storage used for heap-limit checks.
    /// Boxed variants count their out-of-line payload so slimming the enum
    /// never under-reports an object's footprint.
    pub(crate) fn bytes(&self) -> usize {
        let dynamic = match self {
            Self::CaseLambda(values) => vec_bytes(values),
            // The arena spans are exact: handles carry no slack capacity. A
            // width-changing `string-set!` rebuilds its span and charges the
            // growth incrementally at the mutation site.
            Self::Vector { len, .. } => (*len as usize).saturating_mul(size_of::<Value>()),
            Self::Bytevector { len, .. } => *len as usize,
            Self::String { byte_len, .. } => *byte_len as usize,
            Self::Symbol(name) => name.capacity(),
            Self::Closure(closure) => slice_bytes(&closure.captures),
            Self::Promise(Promise {
                state: PromiseState::Done(values),
            }) => vec_bytes(values),
            Self::Continuation(continuation) => size_of::<crate::vm::Continuation>()
                .saturating_add(continuation_bytes(continuation)),
            Self::Record(record) => size_of::<Record>().saturating_add(vec_bytes(&record.fields)),
            Self::RecordProcedure(procedure) => {
                size_of::<RecordProcedure>().saturating_add(match procedure.as_ref() {
                    RecordProcedure::Constructor { mapping, .. } => vec_bytes(mapping),
                    _ => 0,
                })
            }
            Self::Error(error) => {
                size_of::<ErrorObject>().saturating_add(vec_bytes(&error.irritants))
            }
            Self::Parameter(..) => size_of::<Parameter>(),
            Self::Number(..) => size_of::<crate::number::RuntimeNumber>(),
            Self::Pair(..)
            | Self::Native { .. }
            | Self::Apply
            | Self::Box(..)
            | Self::Promise(..)
            | Self::RecordType(..)
            | Self::Port(..)
            | Self::Environment { .. }
            | Self::RandomSource(..) => 0,
        };
        size_of::<Self>().saturating_add(dynamic)
    }
}

fn vec_bytes<T>(values: &Vec<T>) -> usize {
    values.capacity().saturating_mul(size_of::<T>())
}

fn slice_bytes<T>(values: &[T]) -> usize {
    size_of_val(values)
}

fn continuation_bytes(continuation: &crate::vm::Continuation) -> usize {
    let mut bytes = continuation
        .frames
        .capacity()
        .saturating_mul(size_of::<crate::vm::Frame>());
    for frame in &continuation.frames {
        bytes = bytes.saturating_add(slice_bytes(&frame.captures));
    }
    bytes = bytes.saturating_add(continuation.frames.locals_bytes());
    bytes = bytes.saturating_add(continuation.stack.bytes());
    bytes
        .saturating_add(vec_bytes(&continuation.handlers))
        .saturating_add(vec_bytes(&continuation.parameters))
        .saturating_add(vec_bytes(&continuation.parameter_values))
        .saturating_add(vec_bytes(&continuation.winds))
}
