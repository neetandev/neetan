//! Typed, arena-safe accessors for reading and mutating individual heap objects.
//!
//! Every method here goes through [`Heap::object`]/[`Heap::object_mut`] (or a
//! direct `slots` read) rather than handing out references into `slots`, because
//! allocation or collection can reuse a slot and invalidate such a reference.

use super::{
    CallableKind, Callee, ErrorObject, Heap, Object, PromiseState, Record, RecordProcedure,
    RecordType, string,
};
use crate::{Value, value::GcRef};

/// Views bytes of one string span, or a sub-range of one at char
/// boundaries, as text.
///
/// This is one of the crate's narrow unsafe exceptions (see `vm/arith.rs`
/// and `bytecode.rs` for the others): re-validating on every read costs a
/// linear scan on hot accessor paths that the invariant below already pays
/// for once at each write.
///
/// SAFETY: string spans hold valid UTF-8 by construction. The only writers
/// are the atomic constructors `alloc_string` (copies a `&str`) and
/// `alloc_string_concat` (concatenates existing spans), `string_set`
/// (splices `char::encode_utf8` output at a resolved char boundary, in
/// place only when the width matches), and the sweep's whole-span
/// evacuation. Sub-range callers slice at offsets `char_to_byte` resolved,
/// which are char boundaries of the span. The debug assert keeps the
/// invariant checked in every test run.
fn utf8_span(bytes: &[u8]) -> &str {
    debug_assert!(std::str::from_utf8(bytes).is_ok());
    #[allow(unsafe_code)]
    unsafe {
        std::str::from_utf8_unchecked(bytes)
    }
}

/// Reads one vector element through its handle without re-checking the
/// arena range.
///
/// Handle-backed unchecked access, the value-arena counterpart of the VM's
/// verifier-backed register access: re-checking a range the handle already
/// proves cost a measured +3% instructions on the vector-heavy benchmarks.
///
/// SAFETY: the caller must have checked `index < len` against the owning
/// handle. Every handle is constructed by `Heap::alloc_vector`, whose
/// append guards `off + len <= value_arena.len()`, the sweep's evacuation
/// re-establishes the same bound for the compacted arena, and the arena
/// never shrinks between collections, so every in-handle position is in
/// the arena. The debug assert keeps the invariant checked in every test
/// run.
fn vector_element(arena: &[Value], position: usize) -> Value {
    debug_assert!(position < arena.len());
    #[allow(unsafe_code)]
    unsafe {
        *arena.get_unchecked(position)
    }
}

/// The mutable counterpart of [`vector_element`], with the same contract.
fn vector_element_mut(arena: &mut [Value], position: usize) -> &mut Value {
    debug_assert!(position < arena.len());
    #[allow(unsafe_code)]
    unsafe {
        arena.get_unchecked_mut(position)
    }
}

impl Heap {
    pub(crate) fn kind(&self, value: Value) -> crate::ValueKind {
        let Some(reference) = value.heap_ref() else {
            return value.kind();
        };
        match self
            .slots
            .get(reference.0 as usize)
            .and_then(Option::as_ref)
        {
            Some(Object::Pair(..)) => crate::ValueKind::Pair,
            Some(Object::Vector { .. }) => crate::ValueKind::Vector,
            Some(Object::Bytevector { .. }) => crate::ValueKind::Bytevector,
            Some(Object::String { .. }) => crate::ValueKind::String,
            Some(Object::Symbol(..)) => crate::ValueKind::Symbol,
            Some(Object::Closure(..))
            | Some(Object::CaseLambda(..))
            | Some(Object::Continuation(..))
            | Some(Object::RecordProcedure(..)) => crate::ValueKind::Procedure,
            Some(Object::Native { .. }) => crate::ValueKind::NativeProcedure,
            Some(Object::Apply) => crate::ValueKind::Procedure,
            Some(Object::Promise(..)) => crate::ValueKind::Promise,
            Some(Object::Parameter(..)) => crate::ValueKind::Parameter,
            Some(Object::Environment { .. }) => crate::ValueKind::Environment,
            Some(Object::Record(..)) => crate::ValueKind::Record,
            Some(Object::RecordType(..)) => crate::ValueKind::RecordType,
            Some(Object::Port(..)) => crate::ValueKind::Port,
            Some(Object::Number(..)) => crate::ValueKind::Number,
            Some(Object::RandomSource { .. }) => crate::ValueKind::RandomSource,
            Some(_) | None => crate::ValueKind::Heap,
        }
    }

    /// Returns a copy of a closure object without exposing an arena reference.
    pub(crate) fn closure(&self, value: Value) -> Option<crate::vm::Closure> {
        let reference = value.heap_ref()?;
        match self.slots.get(reference.0 as usize)?.as_ref()? {
            Object::Closure(closure) => Some(closure.clone()),
            _ => None,
        }
    }

    /// Borrows a closure object in place. The dispatch-loop call fast path uses
    /// this to read the callee's chunk/captures without the two `Rc` bumps of
    /// [`Self::closure`]. The borrow must end before the next heap mutation.
    pub(crate) fn closure_ref(&self, value: Value) -> Option<&crate::vm::Closure> {
        let reference = value.heap_ref()?;
        match self.slots.get(reference.0 as usize)?.as_ref()? {
            Object::Closure(closure) => Some(closure),
            _ => None,
        }
    }

    /// Classifies a callee with a single arena probe. The dispatch loop's call
    /// fast paths use this instead of probing [`Self::native`] and
    /// [`Self::closure_ref`] independently.
    pub(crate) fn callee(&self, value: Value) -> Callee<'_> {
        let Some(reference) = value.heap_ref() else {
            return Callee::Other;
        };
        match self
            .slots
            .get(reference.0 as usize)
            .and_then(Option::as_ref)
        {
            Some(Object::Closure(closure)) => Callee::Closure(closure),
            Some(&Object::Native { id, fast }) => Callee::Native { id, fast },
            _ => Callee::Other,
        }
    }

    pub(crate) fn native(&self, value: Value) -> Option<u32> {
        let reference = value.heap_ref()?;
        match self.slots.get(reference.0 as usize)?.as_ref()? {
            Object::Native { id, .. } => Some(*id),
            _ => None,
        }
    }

    /// Resolves a native callee to its registry id plus its classified fast
    /// path in one probe, for the dispatch loop's tail-call native fast path.
    pub(crate) fn native_callee(
        &self,
        value: Value,
    ) -> Option<(u32, Option<crate::native::FastProcedure>)> {
        let reference = value.heap_ref()?;
        match self.slots.get(reference.0 as usize)?.as_ref()? {
            &Object::Native { id, fast } => Some((id, fast)),
            _ => None,
        }
    }

    pub(crate) fn callable_kind(&self, value: Value) -> CallableKind {
        match self.object(value) {
            Some(Object::Parameter(..)) => CallableKind::Parameter,
            Some(Object::Continuation(..)) => CallableKind::Continuation,
            Some(Object::Apply) => CallableKind::Apply,
            Some(Object::RecordProcedure(..)) => CallableKind::Record,
            Some(Object::Native { .. }) => CallableKind::Native,
            Some(Object::Closure(..)) => CallableKind::Closure,
            Some(Object::CaseLambda(..)) => CallableKind::CaseLambda,
            _ => CallableKind::Other,
        }
    }

    pub(crate) fn record_procedure(&self, value: Value) -> Option<RecordProcedure> {
        match self.object(value)? {
            Object::RecordProcedure(procedure) => Some((**procedure).clone()),
            _ => None,
        }
    }

    pub(crate) fn record(&self, value: Value) -> Option<Record> {
        match self.object(value)? {
            Object::Record(record) => Some((**record).clone()),
            _ => None,
        }
    }

    pub(crate) fn record_type(&self, value: Value) -> Option<RecordType> {
        match self.object(value)? {
            Object::RecordType(record_type) => Some(record_type.clone()),
            _ => None,
        }
    }

    pub(crate) fn set_record_field(
        &mut self,
        value: Value,
        field: usize,
        replacement: Value,
    ) -> bool {
        let Some(Object::Record(record)) = self.object_mut(value) else {
            return false;
        };
        let Some(slot) = record.fields.get_mut(field) else {
            return false;
        };
        *slot = replacement;
        true
    }

    pub(crate) fn continuation(&self, value: Value) -> Option<crate::vm::Continuation> {
        match self.object(value)? {
            Object::Continuation(continuation) => Some(continuation.as_ref().clone()),
            _ => None,
        }
    }

    pub(crate) fn case_lambda(&self, value: Value, count: usize) -> Option<crate::vm::Closure> {
        let Object::CaseLambda(clauses) = self.object(value)? else {
            return None;
        };
        clauses.iter().find_map(|clause| {
            let closure = self.closure(*clause)?;
            closure.chunk.arity.accepts(count).then_some(closure)
        })
    }

    pub(crate) fn promise_state(&self, value: Value) -> Option<PromiseState> {
        match self.object(value)? {
            Object::Promise(promise) => Some(promise.state.clone()),
            _ => None,
        }
    }

    pub(crate) fn parameter(&self, value: Value) -> Option<Value> {
        match self.object(value)? {
            Object::Parameter(parameter) => Some(parameter.value),
            _ => None,
        }
    }

    pub(crate) fn parameter_converter(&self, value: Value) -> Option<Option<Value>> {
        match self.object(value)? {
            Object::Parameter(parameter) => Some(parameter.converter),
            _ => None,
        }
    }

    pub(crate) fn set_parameter(&mut self, value: Value, replacement: Value) -> bool {
        match self.object_mut(value) {
            Some(Object::Parameter(parameter)) => {
                parameter.value = replacement;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn error_object(&self, value: Value) -> Option<ErrorObject> {
        match self.object(value)? {
            Object::Error(error) => Some((**error).clone()),
            _ => None,
        }
    }

    pub(crate) fn environment_mutable(&self, value: Value) -> Option<bool> {
        match self.object(value)? {
            Object::Environment { mutable } => Some(*mutable),
            _ => None,
        }
    }

    pub(crate) fn random_source(&self, value: Value) -> Option<crate::random::SquaresRng> {
        match self.object(value)? {
            Object::RandomSource(rng) => Some(*rng),
            _ => None,
        }
    }

    pub(crate) fn set_random_source(
        &mut self,
        value: Value,
        replacement: crate::random::SquaresRng,
    ) -> bool {
        match self.object_mut(value) {
            Some(Object::RandomSource(rng)) => {
                *rng = replacement;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn set_promise_state(&mut self, value: Value, state: PromiseState) -> bool {
        match self.object_mut(value) {
            Some(Object::Promise(promise)) => {
                promise.state = state;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn pair(&self, value: Value) -> Option<(Value, Value)> {
        self.object(value).and_then(|object| match object {
            Object::Pair(car, cdr) => Some((*car, *cdr)),
            _ => None,
        })
    }

    pub(crate) fn set_pair_car(&mut self, value: Value, replacement: Value) -> bool {
        match self.mutable_object_mut(value) {
            Some(Object::Pair(car, _)) => {
                *car = replacement;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn set_pair_cdr(&mut self, value: Value, replacement: Value) -> bool {
        match self.mutable_object_mut(value) {
            Some(Object::Pair(_, cdr)) => {
                *cdr = replacement;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn vector(&self, value: Value) -> Option<Vec<Value>> {
        self.vector_slice(value).map(<[Value]>::to_vec)
    }

    pub(crate) fn vector_slice(&self, value: Value) -> Option<&[Value]> {
        self.object(value).and_then(|object| match object {
            Object::Vector { off, len } => self
                .value_arena
                .get(*off as usize..*off as usize + *len as usize),
            _ => None,
        })
    }

    /// Kept small enough to inline into the VM's `VectorRef` arms: the hit
    /// is one bounds compare against the handle and one arena load.
    #[inline]
    pub(crate) fn vector_ref(&self, value: Value, index: usize) -> Option<Value> {
        self.object(value).and_then(|object| match object {
            Object::Vector { off, len } => {
                if index >= *len as usize {
                    return None;
                }
                Some(vector_element(&self.value_arena, *off as usize + index))
            }
            _ => None,
        })
    }

    pub(crate) fn vector_len(&self, value: Value) -> Option<usize> {
        self.object(value).and_then(|object| match object {
            Object::Vector { len, .. } => Some(*len as usize),
            _ => None,
        })
    }

    pub(crate) fn vector_set(&mut self, value: Value, index: usize, replacement: Value) -> bool {
        // The handle is copied out first because the payload write needs the
        // arena mutably while the slot borrow must already be over. The
        // immutability guard mirrors `mutable_object_mut`.
        let Some(reference) = value.heap_ref() else {
            return false;
        };
        let Some(meta) = self.meta.get(reference.0 as usize) else {
            return false;
        };
        if meta & super::META_IMMUTABLE != 0 {
            return false;
        }
        let Some(Object::Vector { off, len }) = self
            .slots
            .get(reference.0 as usize)
            .and_then(Option::as_ref)
        else {
            return false;
        };
        if index >= *len as usize {
            return false;
        }
        let position = *off as usize + index;
        *vector_element_mut(&mut self.value_arena, position) = replacement;
        true
    }

    pub(crate) fn bytevector(&self, value: Value) -> Option<Vec<u8>> {
        self.bytevector_slice(value).map(<[u8]>::to_vec)
    }

    pub(crate) fn bytevector_slice(&self, value: Value) -> Option<&[u8]> {
        self.object(value).and_then(|object| match object {
            Object::Bytevector { off, len } => self
                .byte_arena
                .get(*off as usize..*off as usize + *len as usize),
            _ => None,
        })
    }

    /// Returns the mutable payload of a bytevector. Fails for non-bytevector
    /// values and for write-protected targets, so callers disambiguate their
    /// error on the cold path via `is_immutable` and `bytevector_len`. The
    /// returned slice borrows the heap mutably, which statically prevents any
    /// allocation (and thus arena compaction) while it is alive.
    pub(crate) fn bytevector_slice_mut(&mut self, value: Value) -> Option<&mut [u8]> {
        // The handle is copied out first because the payload borrow needs the
        // arena mutably while the slot borrow must already be over. The
        // immutability guard mirrors `bytevector_set`.
        let reference = value.heap_ref()?;
        let meta = self.meta.get(reference.0 as usize)?;
        if meta & super::META_IMMUTABLE != 0 {
            return None;
        }
        let Some(Object::Bytevector { off, len }) = self
            .slots
            .get(reference.0 as usize)
            .and_then(Option::as_ref)
        else {
            return None;
        };
        let (off, len) = (*off as usize, *len as usize);
        self.byte_arena.get_mut(off..off + len)
    }

    pub(crate) fn bytevector_ref(&self, value: Value, index: usize) -> Option<u8> {
        self.bytevector_slice(value)?.get(index).copied()
    }

    pub(crate) fn bytevector_len(&self, value: Value) -> Option<usize> {
        self.object(value).and_then(|object| match object {
            Object::Bytevector { len, .. } => Some(*len as usize),
            _ => None,
        })
    }

    pub(crate) fn bytevector_set(&mut self, value: Value, index: usize, replacement: u8) -> bool {
        // The handle is copied out first because the payload write needs the
        // arena mutably while the slot borrow must already be over. The
        // immutability guard mirrors `mutable_object_mut`.
        let Some(reference) = value.heap_ref() else {
            return false;
        };
        let Some(meta) = self.meta.get(reference.0 as usize) else {
            return false;
        };
        if meta & super::META_IMMUTABLE != 0 {
            return false;
        }
        let Some(Object::Bytevector { off, len }) = self
            .slots
            .get(reference.0 as usize)
            .and_then(Option::as_ref)
        else {
            return false;
        };
        if index >= *len as usize {
            return false;
        }
        let position = *off as usize + index;
        match self.byte_arena.get_mut(position) {
            Some(byte) => {
                *byte = replacement;
                true
            }
            None => false,
        }
    }

    /// Returns an owned copy of the string contents as UTF-8 text.
    pub(crate) fn string(&self, value: Value) -> Option<String> {
        self.string_slice(value).map(str::to_owned)
    }

    /// Views a string handle's arena span as UTF-8 text without
    /// re-validating it on every read.
    fn string_span(&self, off: u32, byte_len: u32) -> Option<&str> {
        let bytes = self
            .byte_arena
            .get(off as usize..off as usize + byte_len as usize)?;
        Some(utf8_span(bytes))
    }

    pub(crate) fn string_slice(&self, value: Value) -> Option<&str> {
        self.object(value).and_then(|object| match object {
            Object::String { off, byte_len, .. } => self.string_span(*off, *byte_len),
            _ => None,
        })
    }

    /// Returns the UTF-8 slice covering the char range `start..end`. The
    /// caller must have validated the range against `string_len`.
    pub(crate) fn string_range(&self, value: Value, start: usize, end: usize) -> Option<&str> {
        let reference = value.heap_ref()?;
        match self.slots.get(reference.0 as usize)?.as_ref()? {
            Object::String {
                off,
                byte_len,
                chars,
            } => {
                let bytes = self
                    .byte_arena
                    .get(*off as usize..*off as usize + *byte_len as usize)?;
                let hint = self.string_cursors.lookup(reference);
                let (from, to) =
                    string::char_range_to_bytes(bytes, *chars as usize, start, end, hint);
                bytes.get(from..to).map(utf8_span)
            }
            _ => None,
        }
    }

    /// Returns the byte length and char count in one slot resolution, read
    /// straight from the handle without touching the arena.
    pub(crate) fn string_dimensions(&self, value: Value) -> Option<(usize, usize)> {
        self.object(value).and_then(|object| match object {
            Object::String {
                byte_len, chars, ..
            } => Some((*byte_len as usize, *chars as usize)),
            _ => None,
        })
    }

    /// Kept small enough to inline into the VM's `StringRef` arm and the
    /// fused `AddStringRefCode` helper: the all-ASCII hit is one compare and
    /// one byte load, and the multibyte path (with its cursor bookkeeping)
    /// stays out of line so the dispatch loop's codegen does not grow.
    #[inline]
    pub(crate) fn string_ref(&self, value: Value, index: usize) -> Option<char> {
        let reference = value.heap_ref()?;
        match self.slots.get(reference.0 as usize)?.as_ref()? {
            Object::String {
                off,
                byte_len,
                chars,
            } if byte_len == chars => {
                // All-ASCII contents: char indexes are byte indexes and each
                // byte is its char. The explicit span bound keeps the arena
                // read inside this string's payload.
                if index >= *chars as usize {
                    return None;
                }
                self.byte_arena
                    .get(*off as usize + index)
                    .map(|byte| *byte as char)
            }
            Object::String { .. } => self.string_ref_multibyte(reference, index),
            _ => None,
        }
    }

    /// Cursor-assisted indexed access into non-ASCII contents. Not `#[cold]`:
    /// a loop over a multibyte string lands here legitimately every iteration.
    #[inline(never)]
    fn string_ref_multibyte(&self, reference: GcRef, index: usize) -> Option<char> {
        match self.slots.get(reference.0 as usize)?.as_ref()? {
            Object::String {
                off,
                byte_len,
                chars,
            } => {
                let bytes = self
                    .byte_arena
                    .get(*off as usize..*off as usize + *byte_len as usize)?;
                let hint = self.string_cursors.lookup(reference);
                let (value, position) = string::char_at(bytes, *chars as usize, index, hint)?;
                self.string_cursors.store(reference, position);
                Some(value)
            }
            _ => None,
        }
    }

    pub(crate) fn string_len(&self, value: Value) -> Option<usize> {
        self.object(value).and_then(|object| match object {
            Object::String { chars, .. } => Some(*chars as usize),
            _ => None,
        })
    }

    /// Returns the number of new arena bytes a string mutation would append.
    ///
    /// A width-changing write rebuilds the whole string at the arena tail, so
    /// the incoming size is the complete rebuilt span rather than only the
    /// difference between the old and new character widths. Invalid mutations
    /// return `None` and are diagnosed by [`Self::string_set`].
    pub(crate) fn string_set_growth(
        &self,
        value: Value,
        index: usize,
        replacement: char,
    ) -> Option<usize> {
        let reference = value.heap_ref()?;
        let Object::String {
            off,
            byte_len,
            chars,
        } = self.slots.get(reference.0 as usize)?.as_ref()?
        else {
            return None;
        };
        if index >= *chars as usize {
            return None;
        }
        let bytes = self
            .byte_arena
            .get(*off as usize..*off as usize + *byte_len as usize)?;
        let hint = self.string_cursors.lookup(reference);
        let start = if byte_len == chars {
            index
        } else {
            string::char_to_byte(bytes, *chars as usize, index, hint)
        };
        let current = string::decode_char_at(bytes, start)?;
        if current == replacement || current.len_utf8() == replacement.len_utf8() {
            return Some(0);
        }
        Some(*byte_len as usize - current.len_utf8() + replacement.len_utf8())
    }

    pub(crate) fn string_set(
        &mut self,
        value: Value,
        index: usize,
        replacement: char,
    ) -> Result<bool, crate::Error> {
        // The handle is copied out first because the payload write needs the
        // arena mutably while the slot borrow must already be over. The
        // immutability guard mirrors `mutable_object_mut`.
        let Some(reference) = value.heap_ref() else {
            return Ok(false);
        };
        let slot_index = reference.0 as usize;
        let Some(meta) = self.meta.get(slot_index) else {
            return Ok(false);
        };
        if meta & super::META_IMMUTABLE != 0 {
            return Ok(false);
        }
        let Some(Object::String {
            off,
            byte_len,
            chars,
        }) = self.slots.get(slot_index).and_then(Option::as_ref)
        else {
            return Ok(false);
        };
        let (off, len, chars) = (*off as usize, *byte_len as usize, *chars as usize);
        if index >= chars {
            return Ok(false);
        }
        let Some(bytes) = self.byte_arena.get(off..off + len) else {
            return Ok(false);
        };
        let hint = self.string_cursors.lookup(reference);
        let start = if len == chars {
            index
        } else {
            string::char_to_byte(bytes, chars, index, hint)
        };
        let Some(current) = string::decode_char_at(bytes, start) else {
            return Ok(false);
        };
        // Re-anchoring at the mutation point stays valid on every path
        // below: offsets in the prefix are unaffected by a width change and
        // a rebuilt span copies the prefix verbatim.
        if current == replacement {
            self.string_cursors.store(reference, (index, start));
            return Ok(true);
        }
        let old_width = current.len_utf8();
        let mut buffer = [0u8; 4];
        let encoded = replacement.encode_utf8(&mut buffer).as_bytes();
        if encoded.len() == old_width {
            // A same-width replacement keeps every other char's offset, so
            // the span is patched in place and stays valid UTF-8.
            let position = off + start;
            let Some(target) = self.byte_arena.get_mut(position..position + old_width) else {
                return Ok(false);
            };
            target.copy_from_slice(encoded);
            self.string_cursors.store(reference, (index, start));
            return Ok(true);
        }
        // A width change rebuilds the span at the arena tail: prefix, the
        // replacement char, then the suffix. The abandoned span stays
        // charged until the sweep recounts survivors, and the new span is
        // charged here so repeated widening garbage still trips the byte
        // threshold. The guards mirror `byte_arena_append` and report
        // failure without touching the string.
        let new_len = len - old_width + encoded.len();
        let Some(end) = self.byte_arena.len().checked_add(new_len) else {
            return Err(crate::Error::plain(
                crate::ErrorKind::HeapLimitExceeded,
                "payload arena exhausted",
            ));
        };
        if end > u32::MAX as usize {
            return Err(crate::Error::plain(
                crate::ErrorKind::HeapLimitExceeded,
                "payload arena exhausted",
            ));
        }
        if self.total_bytes().saturating_add(new_len) > self.max_bytes {
            return Err(crate::Error::plain(
                crate::ErrorKind::HeapLimitExceeded,
                "heap limit exceeded",
            ));
        }
        self.byte_arena.try_reserve(new_len).map_err(|_| {
            crate::Error::plain(
                crate::ErrorKind::HeapLimitExceeded,
                "payload arena allocation failed",
            )
        })?;
        let new_off = self.byte_arena.len();
        self.byte_arena.extend_from_within(off..off + start);
        self.byte_arena.extend_from_slice(encoded);
        self.byte_arena
            .extend_from_within(off + start + old_width..off + len);
        if let Some(Object::String { off, byte_len, .. }) =
            self.slots.get_mut(slot_index).and_then(Option::as_mut)
        {
            *off = new_off as u32;
            *byte_len = new_len as u32;
        }
        self.dynamic_bytes = self.dynamic_bytes.saturating_add(new_len);
        self.string_cursors.store(reference, (index, start));
        Ok(true)
    }

    pub(crate) fn symbol(&self, value: Value) -> Option<String> {
        self.object(value).and_then(|object| match object {
            Object::Symbol(name) => Some(name.clone()),
            _ => None,
        })
    }

    /// Borrowing variant of `symbol` for callers that only inspect the name.
    pub(crate) fn symbol_slice(&self, value: Value) -> Option<&str> {
        self.object(value).and_then(|object| match object {
            Object::Symbol(name) => Some(name.as_str()),
            _ => None,
        })
    }

    /// Reports whether a heap value is write-protected. Mutating natives use
    /// this on their failure edges to distinguish an immutable target from a
    /// type or range failure.
    pub(crate) fn is_immutable(&self, value: Value) -> bool {
        value.heap_ref().is_some_and(|reference| {
            self.meta
                .get(reference.0 as usize)
                .is_some_and(|byte| byte & super::META_IMMUTABLE != 0)
        })
    }

    pub(crate) fn make_immutable(&mut self, value: Value) {
        if let Some(reference) = value.heap_ref()
            && let Some(byte) = self.meta.get_mut(reference.0 as usize)
            && *byte & super::META_STATE_MASK != super::META_FREE
        {
            // Never tag a free slot: the sweep's all-free word test relies
            // on free meta bytes being entirely zero.
            *byte |= super::META_IMMUTABLE;
        }
    }

    /// Returns a copy of a heap-backed numeric value.
    pub(crate) fn number(&self, value: Value) -> Option<crate::number::RuntimeNumber> {
        match self.object(value)? {
            Object::Number(number) => Some(**number),
            _ => None,
        }
    }

    /// Returns the backing-store identifier for a port value.
    pub(crate) fn port(&self, value: Value) -> Option<crate::port::PortId> {
        match self.object(value)? {
            Object::Port(port) => Some(port.id),
            _ => None,
        }
    }

    fn object(&self, value: Value) -> Option<&Object> {
        let reference = value.heap_ref()?;
        self.slots.get(reference.0 as usize)?.as_ref()
    }
    fn object_mut(&mut self, value: Value) -> Option<&mut Object> {
        let reference = value.heap_ref()?;
        self.slots.get_mut(reference.0 as usize)?.as_mut()
    }

    /// Resolves a value for a guarded mutation: the object is returned only
    /// when the value is a live heap slot that is not write-protected. The
    /// immutable bit lives in the dense meta table, so the guard costs one
    /// extra byte load before the slot resolution.
    fn mutable_object_mut(&mut self, value: Value) -> Option<&mut Object> {
        let reference = value.heap_ref()?;
        let index = reference.0 as usize;
        if *self.meta.get(index)? & super::META_IMMUTABLE != 0 {
            return None;
        }
        self.slots.get_mut(index)?.as_mut()
    }

    pub(crate) fn boxed(&self, value: Value) -> Option<Value> {
        let reference = value.heap_ref()?;
        match self.slots.get(reference.0 as usize)?.as_ref()? {
            Object::Box(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn set_boxed(&mut self, value: Value, replacement: Value) -> bool {
        let Some(reference) = value.heap_ref() else {
            return false;
        };
        match self
            .slots
            .get_mut(reference.0 as usize)
            .and_then(Option::as_mut)
        {
            Some(Object::Box(current)) => {
                *current = replacement;
                true
            }
            _ => false,
        }
    }
}
