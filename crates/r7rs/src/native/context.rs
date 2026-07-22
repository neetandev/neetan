//! The native-call context handed to host procedures, and the result-packet
//! types (`NativeValues`, `IntoNativeValues`).

use super::*;

/// The context supplied to a registered host procedure.
///
/// Values returned by its constructors are valid for the callback activation;
/// callers that need to retain one after returning must arrange for Scheme to
/// retain it (for example by returning it or storing it in a pair/vector).
pub struct NativeContext<'a> {
    pub(in crate::native) heap: &'a mut Heap,
    pub(in crate::native) symbols: &'a mut HashMap<String, Value>,
    pub(in crate::native) globals: &'a crate::global::GlobalStore,
    /// Enumerates the live VM roots (register file, frames, pending `apply`
    /// arguments) for a collection that runs during this native call. `None`
    /// only on paths that run under VM-managed rooting, where `alloc` defers
    /// collection to the next safe point instead of collecting here.
    pub(in crate::native) vm_roots: Option<&'a crate::heap::RootGatherer<'a>>,
}

/// Zero or more Scheme values returned by a native callback.
pub struct NativeValues(NativeValuesRepr);

enum NativeValuesRepr {
    Zero,
    One(Value),
    Many(Vec<Value>),
}

impl NativeValues {
    /// Creates an empty result packet.
    #[must_use]
    pub const fn none() -> Self {
        Self(NativeValuesRepr::Zero)
    }

    /// Creates a single-value result packet.
    #[must_use]
    pub const fn one(value: Value) -> Self {
        Self(NativeValuesRepr::One(value))
    }

    /// Creates a result packet from values in delivery order.
    #[must_use]
    pub fn many(values: impl IntoIterator<Item = Value>) -> Self {
        let values: Vec<_> = values.into_iter().collect();
        match values.len() {
            0 => Self::none(),
            1 => Self::one(values[0]),
            _ => Self(NativeValuesRepr::Many(values)),
        }
    }

    pub(crate) fn into_results(self) -> crate::vm::Results {
        match self.0 {
            NativeValuesRepr::Zero => crate::vm::Results::Zero,
            NativeValuesRepr::One(value) => crate::vm::Results::One(value),
            NativeValuesRepr::Many(values) => crate::vm::Results::Many(values),
        }
    }

    /// Splits off the single-value hot case. The remainder converts to the
    /// VM's generic result packet.
    pub(crate) fn into_single(self) -> Result<Value, crate::vm::Results> {
        match self.0 {
            NativeValuesRepr::One(value) => Ok(value),
            NativeValuesRepr::Zero => Err(crate::vm::Results::Zero),
            NativeValuesRepr::Many(values) => Err(crate::vm::Results::Many(values)),
        }
    }
}

/// Converts a native callback result into a Scheme result packet.
pub trait IntoNativeValues {
    /// Whether this type always represents exactly one value.
    ///
    /// External implementations may leave the default in place. Implementations
    /// that override this to `true` must also override
    /// [`Self::into_single_native_value`] to return the promised value.
    const SINGLE_RESULT: bool = false;

    /// Performs the conversion.
    fn into_native_values(self) -> NativeValues;

    /// Converts a result type that promises exactly one value.
    ///
    /// The default preserves compatibility for external result types and leaves
    /// them on the general zero-or-more-values path.
    fn into_single_native_value(self) -> Result<Value, NativeValues>
    where
        Self: Sized,
    {
        Err(self.into_native_values())
    }
}

impl IntoNativeValues for Value {
    const SINGLE_RESULT: bool = true;

    fn into_native_values(self) -> NativeValues {
        NativeValues::one(self)
    }

    fn into_single_native_value(self) -> Result<Value, NativeValues> {
        Ok(self)
    }
}

impl IntoNativeValues for NativeValues {
    fn into_native_values(self) -> NativeValues {
        self
    }
}

impl NativeContext<'_> {
    /// Creates an exact integer, using the heap when it exceeds the inline range.
    pub fn integer(&mut self, value: i128) -> Result<Value, Error> {
        match i64::try_from(value).ok().map(Value::integer) {
            Some(value) => Ok(value),
            None => self.alloc(Object::Number(Box::new(RuntimeNumber::Real(
                Real::ExactInteger(value),
            )))),
        }
    }

    /// Converts an exact integer argument.
    pub fn to_i128(&self, value: Value) -> Result<i128, Error> {
        if let Some(value) = value.as_fixnum() {
            return Ok(i128::from(value));
        }
        match self.heap.number(value) {
            Some(RuntimeNumber::Real(Real::ExactInteger(value))) => Ok(value),
            _ => Err(type_error("exact integer", value, self.heap)),
        }
    }

    /// Converts an inexact floating-point argument.
    pub fn to_f64(&self, value: Value) -> Result<f64, Error> {
        value
            .as_float()
            .ok_or_else(|| type_error("inexact number", value, self.heap))
    }

    /// Returns the dynamic type of a callback value.
    #[must_use]
    pub fn kind(&self, value: Value) -> ValueKind {
        self.heap.kind(value)
    }

    /// Borrows the text of a string argument.
    ///
    /// The slice is valid for the callback activation. Callers that need to
    /// retain the text after returning should copy it with `to_owned`.
    pub fn to_str(&self, value: Value) -> Result<&str, Error> {
        self.heap
            .string_slice(value)
            .ok_or_else(|| type_error("string", value, self.heap))
    }

    /// Renders a value to its `write` external representation.
    ///
    /// The text matches the `write` procedure, so it reads back with `read`. This
    /// lets a host serialize Scheme data to an artifact and reload it later without
    /// a bespoke serializer.
    pub fn write_to_string(&self, value: Value) -> Result<String, Error> {
        crate::printer::write_value(self.heap, value, crate::printer::RuntimeWriteMode::Write)
    }

    /// Borrows the name of a symbol argument.
    ///
    /// The slice is valid for the callback activation. Callers that need to
    /// retain the name after returning should copy it with `to_owned`.
    pub fn to_symbol_name(&self, value: Value) -> Result<&str, Error> {
        self.heap
            .symbol_slice(value)
            .ok_or_else(|| type_error("symbol", value, self.heap))
    }

    /// Borrows the bytes of a bytevector argument.
    ///
    /// The slice is valid for the callback activation. Callers that need to
    /// retain the bytes after returning should copy them with `to_vec`.
    pub fn to_bytes(&self, value: Value) -> Result<&[u8], Error> {
        self.heap
            .bytevector_slice(value)
            .ok_or_else(|| type_error("bytevector", value, self.heap))
    }

    /// Reads the car and cdr of a pair argument.
    ///
    /// This is the primitive for decomposing list structure. Walk a chain by
    /// following the returned cdr, or use [`Self::to_list`] to collect a
    /// proper list in one call.
    pub fn to_pair(&self, value: Value) -> Result<(Value, Value), Error> {
        self.heap
            .pair(value)
            .ok_or_else(|| type_error("pair", value, self.heap))
    }

    /// Collects the elements of a proper list argument in order.
    ///
    /// A non-pair tail other than the empty list, or a circular list, is
    /// reported as a type error. The traversal uses a tortoise and hare so a
    /// cyclic argument terminates instead of looping forever.
    pub fn to_list(&self, value: Value) -> Result<Vec<Value>, Error> {
        let mut elements = Vec::new();
        let mut hare = value;
        let mut tortoise = value;
        loop {
            if hare == Value::nil() {
                return Ok(elements);
            }
            let Some((car, cdr)) = self.heap.pair(hare) else {
                return Err(type_error("proper list", value, self.heap));
            };
            elements.push(car);
            hare = cdr;
            if hare == Value::nil() {
                return Ok(elements);
            }
            let Some((car, cdr)) = self.heap.pair(hare) else {
                return Err(type_error("proper list", value, self.heap));
            };
            elements.push(car);
            hare = cdr;
            // The hare advanced two pairs, so step the tortoise one and
            // compare. Meeting means the chain is circular.
            if let Some((_, next)) = self.heap.pair(tortoise) {
                tortoise = next;
            }
            if hare == tortoise {
                return Err(type_error("proper list", value, self.heap));
            }
        }
    }

    /// Allocates a mutable pair. Pinned `inline(always)` like
    /// [`Self::alloc`], and built through [`Heap::alloc_pair_hot`] so the
    /// pair is constructed in place in its slot from the two `Value` halves:
    /// the pair-building loops in the list natives are the hottest
    /// allocation sites, and routing the 48-byte `Object` enum through the
    /// stack cost a measured ~27 cycles per pair on seq_copy_churn.
    #[inline(always)]
    pub fn pair(&mut self, car: Value, cdr: Value) -> Result<Value, Error> {
        // A pair carries no dynamic payload, so the soft-threshold check
        // sizes exactly one slot, like `alloc` would for `Object::Pair`.
        if self.vm_roots.is_some() && self.heap.needs_collection_for(size_of::<Object>()) {
            self.collect();
        }
        let value = self.heap.alloc_pair_hot(car, cdr)?;
        self.heap.push_root(value);
        Ok(value)
    }

    /// Allocates a mutable vector. The payload goes into the heap's value
    /// arena through the atomic constructor, so this path performs no
    /// per-object Rust allocation beyond the caller's element buffer.
    pub fn vector(&mut self, values: Vec<Value>) -> Result<Value, Error> {
        self.vector_from_slice(&values)
    }

    /// Allocates a mutable vector from a borrowed payload without consuming
    /// a `Vec`. The caller must keep every element rooted across the call
    /// (native arguments and freshly built context values already are).
    pub(in crate::native) fn vector_from_slice(
        &mut self,
        values: &[Value],
    ) -> Result<Value, Error> {
        // The soft-threshold check mirrors `alloc`: one slot plus the
        // payload span.
        if self.vm_roots.is_some()
            && self.heap.needs_collection_for(
                size_of::<Object>().saturating_add(values.len().saturating_mul(size_of::<Value>())),
            )
        {
            self.collect();
        }
        let value = self.heap.alloc_vector(values)?;
        self.heap.push_root(value);
        Ok(value)
    }

    /// Allocates a mutable vector of `count` copies of `fill`, written
    /// straight into the value arena with no temporary element buffer.
    pub(in crate::native) fn vector_filled(
        &mut self,
        fill: Value,
        count: usize,
    ) -> Result<Value, Error> {
        // The soft-threshold check mirrors `alloc`: one slot plus the
        // payload span.
        if self.vm_roots.is_some()
            && self.heap.needs_collection_for(
                size_of::<Object>().saturating_add(count.saturating_mul(size_of::<Value>())),
            )
        {
            self.collect();
        }
        let value = self.heap.alloc_vector_filled(fill, count)?;
        self.heap.push_root(value);
        Ok(value)
    }

    /// Allocates a mutable string of `count` copies of `fill` without first
    /// constructing an unbounded temporary `String`.
    pub(in crate::native) fn string_filled(
        &mut self,
        fill: char,
        count: usize,
    ) -> Result<Value, Error> {
        let payload = count.saturating_mul(fill.len_utf8());
        if self.vm_roots.is_some()
            && self
                .heap
                .needs_collection_for(size_of::<Object>().saturating_add(payload))
        {
            self.collect();
        }
        let value = self.heap.alloc_string_filled(fill, count)?;
        self.heap.push_root(value);
        Ok(value)
    }

    /// Allocates a mutable string from Unicode scalar values.
    pub fn string(&mut self, value: impl IntoIterator<Item = char>) -> Result<Value, Error> {
        // `String::extend` pre-reserves from the iterator's size hint, so
        // exact sources (collected argument vectors) build the temporary
        // text in one allocation while counting.
        let mut chars = 0usize;
        let mut text = String::new();
        text.extend(value.into_iter().inspect(|_| chars += 1));
        self.string_with_char_count(&text, chars)
    }

    /// Allocates a mutable string from owned UTF-8 text.
    pub fn string_utf8(&mut self, value: String) -> Result<Value, Error> {
        let chars = value.chars().count();
        self.string_with_char_count(&value, chars)
    }

    /// Allocates a string whose char count the caller already knows. The
    /// payload goes into the heap's byte arena through the atomic
    /// constructor, so this path performs no per-object Rust allocation.
    pub(in crate::native) fn string_with_char_count(
        &mut self,
        text: &str,
        chars: usize,
    ) -> Result<Value, Error> {
        // The soft-threshold check mirrors `alloc`: one slot plus the
        // payload span.
        if self.vm_roots.is_some()
            && self
                .heap
                .needs_collection_for(size_of::<Object>().saturating_add(text.len()))
        {
            self.collect();
        }
        let value = self.heap.alloc_string(text, chars)?;
        self.heap.push_root(value);
        Ok(value)
    }

    /// Allocates the concatenation of already-validated string arguments
    /// straight from their arena spans, with no temporary text and no
    /// re-validation. The caller must have type-checked every part and
    /// summed `total_bytes`/`total_chars` from the handles.
    pub(in crate::native) fn string_concat(
        &mut self,
        parts: &[Value],
        total_bytes: usize,
        total_chars: usize,
    ) -> Result<Value, Error> {
        // The soft-threshold check mirrors `alloc`: one slot plus the
        // payload span.
        if self.vm_roots.is_some()
            && self
                .heap
                .needs_collection_for(size_of::<Object>().saturating_add(total_bytes))
        {
            self.collect();
        }
        let value = self
            .heap
            .alloc_string_concat(parts, total_bytes, total_chars)?;
        self.heap.push_root(value);
        Ok(value)
    }

    /// Allocates a bytevector. The payload goes into the heap's byte arena
    /// through the atomic constructor, so this path performs no per-object
    /// Rust allocation.
    pub fn bytevector(&mut self, values: Vec<u8>) -> Result<Value, Error> {
        self.bytevector_from_slice(&values)
    }

    /// Allocates a mutable bytevector of `count` copies of `fill` without an
    /// unbounded temporary `Vec`.
    pub(in crate::native) fn bytevector_filled(
        &mut self,
        fill: u8,
        count: usize,
    ) -> Result<Value, Error> {
        if self.vm_roots.is_some()
            && self
                .heap
                .needs_collection_for(size_of::<Object>().saturating_add(count))
        {
            self.collect();
        }
        let value = self.heap.alloc_bytevector_filled(fill, count)?;
        self.heap.push_root(value);
        Ok(value)
    }

    /// Allocates a bytevector from a borrowed payload without consuming a
    /// `Vec`.
    pub(in crate::native) fn bytevector_from_slice(
        &mut self,
        values: &[u8],
    ) -> Result<Value, Error> {
        // The soft-threshold check mirrors `alloc`: one slot plus the
        // payload span.
        if self.vm_roots.is_some()
            && self
                .heap
                .needs_collection_for(size_of::<Object>().saturating_add(values.len()))
        {
            self.collect();
        }
        let value = self.heap.alloc_bytevector(values)?;
        self.heap.push_root(value);
        Ok(value)
    }

    /// Allocates a textual input port over a copy of `value`.
    pub fn input_string(&mut self, value: impl IntoIterator<Item = char>) -> Result<Value, Error> {
        let id = self
            .heap
            .ports_mut()
            .text_input(value.into_iter().collect())?;
        self.port(id)
    }

    /// Allocates a textual input port over owned UTF-8 text.
    pub fn input_string_utf8(&mut self, value: String) -> Result<Value, Error> {
        let id = self.heap.ports_mut().text_input(value)?;
        self.port(id)
    }

    /// Allocates a textual output port whose written content can be retrieved by Scheme.
    pub fn output_string(&mut self) -> Result<Value, Error> {
        let id = self.heap.ports_mut().new_text_output()?;
        self.port(id)
    }

    /// Allocates a binary input port over a copy of `value`.
    pub fn input_bytevector(&mut self, value: Vec<u8>) -> Result<Value, Error> {
        let id = self.heap.ports_mut().binary_input(value)?;
        self.port(id)
    }

    /// Allocates a binary output port whose written content can be retrieved by Scheme.
    pub fn output_bytevector(&mut self) -> Result<Value, Error> {
        let id = self.heap.ports_mut().new_binary_output()?;
        self.port(id)
    }

    /// Allocates the heap object that owns a freshly inserted port entry.
    /// Failed allocation must finalize the entry because no heap object exists
    /// for collection to discover later.
    pub(in crate::native) fn port(&mut self, id: crate::port::PortId) -> Result<Value, Error> {
        match self.alloc(Object::Port(crate::port::PortObject { id })) {
            Ok(value) => Ok(value),
            Err(error) => {
                self.heap.ports_mut().finalize(id);
                Err(error)
            }
        }
    }

    /// Returns the engine's canonical symbol for `name`.
    pub fn intern_symbol(&mut self, name: &str) -> Result<Value, Error> {
        if let Some(value) = self.symbols.get(name) {
            return Ok(*value);
        }
        let value = self.alloc(Object::Symbol(name.to_owned()))?;
        self.symbols.insert(name.to_owned(), value);
        // This is the single place the symbol table grows, so the cached
        // engine roots must be refreshed before the next collection.
        self.heap.mark_engine_roots_dirty();
        Ok(value)
    }

    /// Performs a collection while all native arguments and temporaries remain rooted.
    pub fn collect_now(&mut self) {
        self.collect();
    }

    /// Mutates one string character after collecting before arena growth when
    /// the soft threshold requires it. The collection runs before
    /// [`Heap::string_set`] borrows or appends the payload, then that method
    /// enforces the hard byte limit atomically with the mutation.
    pub(in crate::native) fn string_set(
        &mut self,
        value: Value,
        index: usize,
        replacement: char,
    ) -> Result<bool, Error> {
        if let Some(bytes) = self.heap.string_set_growth(value, index, replacement)
            && self.vm_roots.is_some()
            && self.heap.needs_collection_for(bytes)
        {
            self.collect();
        }
        self.heap.string_set(value, index, replacement)
    }

    /// Runs a collection against the full root set: engine tables (refreshed if
    /// stale), rooted temporaries, and, when this context carries one, the
    /// live VM register view. Outlined and cold: this sits on the rare edge
    /// of the inlined [`Self::alloc`] and must not bloat every native.
    #[cold]
    #[inline(never)]
    fn collect(&mut self) {
        self.heap.sync_engine_roots(self.globals, self.symbols);
        match self.vm_roots {
            Some(roots) => self.heap.collect_with(roots),
            // Without a view this context runs under VM-managed rooting. The
            // register file is untraceable here, so only the always-rooted sets
            // are collected against. Reached only from the register-operation
            // fallback, whose built-ins never force a collection.
            None => self.heap.collect(),
        }
    }

    pub(in crate::native) fn current_port(&self, name: &str) -> Result<Value, Error> {
        let parameter = self.globals.get(name).copied().ok_or_else(|| {
            Error::plain(
                ErrorKind::RuntimeError,
                format!("missing current port parameter '{name}'"),
            )
        })?;
        self.heap.parameter(parameter).ok_or_else(|| {
            Error::plain(
                ErrorKind::RuntimeError,
                format!("'{name}' is not a parameter"),
            )
        })
    }

    /// Pinned `inline(always)` so the caller's freshly built `Object` is
    /// constructed straight into its heap slot instead of being copied by
    /// value through every call in the chain (a plain hint was dropped, as
    /// with `prepare_tail_self_call`). Natives run outside the VM dispatch
    /// loop, so this inlining cannot disturb the loop's codegen (the dispatch
    /// loop allocates through the pinned out-of-line [`Heap::alloc`]).
    #[inline(always)]
    pub(in crate::native) fn alloc(&mut self, object: Object) -> Result<Value, Error> {
        // Natives can allocate without bound between VM safe points, so cross
        // the soft threshold here, against the precise register view, rather
        // than deferring. `Heap::alloc` itself never collects while the VM is
        // live (it only arms a deferred collection).
        let bytes = object.bytes();
        if self.vm_roots.is_some() && self.heap.needs_collection_for(bytes) {
            self.collect();
        }
        let value = self.heap.alloc_hot_sized(object, bytes)?;
        self.heap.push_root(value);
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn string_width_changes_collect_before_the_hard_byte_limit() {
        let limits = crate::Limits::default()
            .with_initial_gc_threshold(100)
            .with_max_heap_bytes(size_of::<Object>() + 8);
        let mut heap = Heap::new(&limits);
        let text = heap.alloc_string("a", 1).unwrap();
        let mut symbols = HashMap::new();
        let globals = crate::global::GlobalStore::default();
        let gather = |roots: &mut Vec<Value>| roots.push(text);
        let mut context = NativeContext {
            heap: &mut heap,
            symbols: &mut symbols,
            globals: &globals,
            vm_roots: Some(&gather),
        };

        assert!(context.string_set(text, 0, '\u{1f600}').unwrap());
        assert!(context.string_set(text, 0, 'a').unwrap());
        assert!(context.string_set(text, 0, '\u{1f600}').unwrap());
        assert_eq!(context.heap.string(text).as_deref(), Some("\u{1f600}"));
    }

    #[test]
    fn failed_port_allocation_removes_the_unowned_port_entry() {
        let limits = crate::Limits::default()
            .with_initial_gc_threshold(1)
            .with_max_heap_slots(1);
        let mut heap = Heap::new(&limits);
        let live = heap
            .alloc(Object::Pair(Value::nil(), Value::nil()))
            .unwrap();
        let mut symbols = HashMap::new();
        let mut globals = crate::global::GlobalStore::default();
        globals.insert("live".to_owned(), live);
        let gather = |_roots: &mut Vec<Value>| {};
        let mut context = NativeContext {
            heap: &mut heap,
            symbols: &mut symbols,
            globals: &globals,
            vm_roots: Some(&gather),
        };

        assert_eq!(
            context.output_string().unwrap_err().kind(),
            ErrorKind::HeapLimitExceeded
        );
        assert!(!context.heap.ports_mut().contains(crate::port::PortId(0)));
    }
}
