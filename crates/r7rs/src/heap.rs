//! The private, non-moving, handle-indexed object heap with a mark-sweep
//! collector using Nofl-style hole-skipping bump allocation.
//!
//! Heap objects are addressed only through [`GcRef`], a stable slot index.
//! This module never exposes references into `slots`, because allocating or
//! collecting can reuse a slot and invalidate such a reference.
//!
//! # Design
//!
//! The heap is a uniform-slot arena: every object is one 48-byte [`Object`]
//! in `slots`. Payloads that do not fit inline either live in ordinary
//! Rust allocations owned by the object (boxed variants, closure captures)
//! or as offset-addressed spans in the heap-owned payload arenas: strings
//! and bytevectors in `byte_arena`, vectors in `value_arena`. Because
//! slots are uniform, every free slot fits
//! every object and fragmentation cannot occur, so the collector never
//! moves objects and needs no forwarding state. Arena payload spans are
//! the one moving part: each span has exactly one owning slot, so the
//! sweep compacts the arena by evacuating survivors in scan order and
//! rewriting each handle's offset in place.
//!
//! Collection is stop-the-world mark-and-sweep. The allocator and sweep
//! mechanics are adapted from Nofl
//! (<https://arxiv.org/abs/2503.16971v1>), which in turn descends from
//! Immix (<https://www.steveblackburn.org/pubs/papers/immix-pldi-2008.pdf>):
//!
//! - All collector bookkeeping lives in a side metadata table (`meta`) with
//!   one byte per slot holding the mark state and the immutable bit.
//!   Objects carry no header bits at all.
//! - Marks rotate between two colors each cycle instead of being cleared,
//!   so the sweep never writes to surviving slots. Two colors suffice only
//!   because the sweep is eager and complete (see `current_mark`). Fresh
//!   allocations take a third young state distinct from both colors, so
//!   the allocation fast path never loads the current color and unmarked
//!   young objects die at the next cycle.
//! - The sweep scans the metadata table one 8-byte word at a time, drops
//!   dead objects in place (running their `Drop` and queueing port
//!   finalization), recounts live storage from survivors, evacuates
//!   surviving payload spans into the compacted arena, and records maximal
//!   runs of free slots as holes.
//! - Allocation bump-allocates through those holes. The refill slow path
//!   pre-authorizes a span (`cursor..limit`) clamped to the nearer of the
//!   next soft-threshold crossing and the hard limits, so the fast path is
//!   one span compare plus the slot and metadata writes, with no threshold
//!   checks and no counter updates. Live and byte accounting are derived
//!   from the span consumption instead.
//!
//! Collections are scheduled by adaptive soft thresholds (slots and bytes)
//! and bounded by hard limits from [`crate::Limits`]. Rooting and
//! collection run in one of three regimes. Host and setup code roots
//! through `push_root` and collects inside `alloc` against the explicit
//! root stacks. While the VM main loop is active `push_root` is a no-op,
//! a threshold crossing only arms the safe-point trap, and the collection
//! runs at the next safe point against a precise register trace. A native
//! call opens a rooted region in which `push_root` engages again and the
//! native context drives collections itself against the VM's precise
//! register view, because `alloc` keeps deferring while the VM is live.
//!
//! Inlining note: `alloc` and `alloc_pair` are pinned out of line. The
//! folded fast path is small enough that LLVM otherwise inlines it into
//! the VM dispatch loop at every allocation arm, and the added register
//! pressure spills the loop's retirement counter to the stack. Native
//! procedures allocate through the inlineable `alloc_hot` instead, where
//! inlining cannot disturb the dispatch loop.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use crate::{
    Error, ErrorKind, Value,
    value::{GcRef, HostRoots},
};

mod access;
mod object;
mod string;

pub(crate) use object::{
    CallableKind, Callee, ConditionKind, ErrorObject, Object, Parameter, Promise, PromiseState,
    Record, RecordProcedure, RecordType,
};
use string::StringCursors;

/// Metadata state mask covering the low three bits of a slot's meta byte.
pub(super) const META_STATE_MASK: u8 = 0b0000_0111;
/// The slot is free and holds `None`.
pub(super) const META_FREE: u8 = 0;
/// The slot was allocated after the most recent collection. A constant
/// distinct from both mark colors so the allocation fast path stores an
/// immediate and never loads `current_mark`.
pub(super) const META_YOUNG: u8 = 1;
/// First of the two rotating mark colors.
pub(super) const META_MARK_A: u8 = 2;
/// Second of the two rotating mark colors.
pub(super) const META_MARK_B: u8 = 3;

// The collector rotates colors with one xor. This pin keeps the constants
// honest should either color ever be renumbered.
const _: () = assert!(META_MARK_A ^ 1 == META_MARK_B);
/// Set while the object in this slot is immutable (write-protected). Kept in
/// the meta byte so mutation guards read the same dense side table the
/// collector uses and slot reuse clears it together with the state bits.
pub(super) const META_IMMUTABLE: u8 = 0b0000_1000;

/// A root scope backed by a shared temporary-root stack.
///
/// The scope does not borrow `Heap`, allowing allocations while it exists.
/// Dropping it restores the stack to its initial length.
pub(crate) struct RootScope {
    roots: Rc<RefCell<Vec<Value>>>,
    start: usize,
}

impl Drop for RootScope {
    /// Removes every temporary root registered after this scope began.
    fn drop(&mut self) {
        self.roots.borrow_mut().truncate(self.start);
    }
}

/// Callback that appends extra GC roots to a collection's mark worklist.
/// The VM's live register/frame view, enumerated lazily at collect time.
pub(crate) type RootGatherer<'a> = dyn Fn(&mut Vec<Value>) + 'a;

/// One maximal run of free slots found by the sweep, `start..end`.
struct Hole {
    start: u32,
    end: u32,
}

/// Stop-the-world, non-moving arena storage owned by one engine.
pub(crate) struct Heap {
    /// `None` denotes a free slot. Invariant: `slots[i]` is `None` exactly
    /// when the state bits of `meta[i]` are [`META_FREE`].
    pub(super) slots: Vec<Option<Object>>,
    /// Backing store for vector payload spans, bump-allocated at the tail.
    /// Every span has exactly one owning slot, so the sweep compacts by
    /// evacuating survivors into `value_arena_scratch` in slot order and
    /// rewriting each handle's offset in place. Dead spans need no action.
    /// The mark phase traces live spans through [`Object::trace`]. Declared
    /// beside `slots` so the indexed-access fast paths (`vector_ref`,
    /// `vector_set`) find both hot vector pointers on one cache line.
    value_arena: Vec<Value>,
    /// Backing store for string and bytevector payload spans, with the same
    /// single-owner and compaction rules. String spans are always valid
    /// UTF-8. Kept in the hot leading fields for the `string_ref` fast path.
    byte_arena: Vec<u8>,
    /// One metadata byte per slot, always the same length as `slots`. Holds
    /// the mark state and the immutable bit, so the collector never writes
    /// into live slots and mutation guards read a dense side table.
    pub(super) meta: Vec<u8>,
    /// Maximal runs of free slots recorded by the last sweep, ordered so
    /// `pop` hands out the lowest-address hole first. The allocator bump
    /// allocates through one hole at a time via `cursor`/`limit`.
    holes: Vec<Hole>,
    /// The next slot the bump allocator hands out. Valid while it is below
    /// `limit`.
    cursor: u32,
    /// Exclusive end of the authorized bump span. The refill slow path sets
    /// it no further than the nearer soft threshold or hard limit, which is
    /// what lets the allocation fast path skip every threshold check. Equal
    /// to `cursor` when no span is active.
    limit: u32,
    /// Exclusive physical end of the hole the allocator is consuming. The
    /// stretch `limit..region_end` is free memory the refill can authorize
    /// without popping another hole.
    region_end: u32,
    /// Start of the current authorization. Live accounting is derived, so
    /// the fast path maintains no counter: consumed slots are
    /// `cursor - auth_start` and are retired into `live_base` whenever the
    /// allocator moves or a collection runs.
    auth_start: u32,
    /// The mark color denoting "live in the current cycle". Alternates
    /// between [`META_MARK_A`] and [`META_MARK_B`] each collection. Two
    /// colors suffice only because the sweep is eager and complete: it
    /// frees every non-current-mark occupied slot and clears its meta byte,
    /// so no stale color can survive into the cycle that reuses it. A
    /// future lazy or partial sweep would need a third color.
    current_mark: u8,
    /// Live slots counted by the last sweep plus every retired authorization
    /// since. The instantaneous live count is [`Self::live_slots`].
    live_base: usize,
    /// Owned out-of-line payload bytes of live objects beyond the 48-byte
    /// slot base, recounted from survivors by every sweep and grown by
    /// allocations of dynamic payloads in between. The instantaneous byte
    /// footprint is [`Self::total_bytes`].
    dynamic_bytes: usize,
    slot_threshold: usize,
    byte_threshold: usize,
    minimum_slot_headroom: usize,
    minimum_byte_headroom: usize,
    max_slots: usize,
    max_bytes: usize,
    mark_worklist: Vec<Value>,
    temporary_roots: Rc<RefCell<Vec<Value>>>,
    host_roots: Rc<RefCell<HostRoots>>,
    engine_roots: Vec<Value>,
    ports: crate::port::PortStore,
    file_system: Option<Box<dyn crate::FileSystem>>,
    process_context: Option<Box<dyn crate::ProcessContext>>,
    clock: Option<Box<dyn crate::Clock>>,
    pending_exit: Option<crate::ExitStatus>,
    completed_exit: Option<crate::ExitStatus>,
    /// Depth of active "rooted regions" (native calls). While greater than
    /// zero, `push_root` engages even though the VM main loop is live, so
    /// `NativeContext` allocations stay rooted. Collections inside a region
    /// are driven by the native context through [`Self::collect_with`], and
    /// `alloc` itself keeps deferring to the next safe point.
    rooted_region_depth: u32,
    /// Set when `alloc` crossed a soft threshold but deferred collection because
    /// no rooted region was active. The VM polls this at safe points.
    pending_collection: bool,
    /// Fused safe-point trap: true whenever `pending_collection` or
    /// `completed_exit` is set. The VM's safe point tests only this flag on its
    /// fast path; the cold handler re-derives it from those source flags.
    trap: bool,
    /// Set when the globals/symbols captured in `engine_roots` may be stale (a
    /// global was assigned or a symbol interned). The VM refreshes `engine_roots`
    /// from the live tables before the next collection, so per-native rooting need
    /// not rescan those (usually large) tables every call.
    engine_roots_dirty: bool,
    /// True only while the VM main loop manages rooting through periodic precise
    /// scans. While set (and outside a rooted region), the VM opcodes skip
    /// `push_root` and `alloc` defers collection to the next safe point. Held
    /// behind an `Rc<Cell>` so an RAII guard can clear it on every `execute` exit.
    vm_active: Rc<Cell<bool>>,
    /// Slot-keyed cursors amortizing indexed access into non-ASCII strings.
    /// Cleared at every collection, before freed slots can be reused.
    string_cursors: StringCursors,
    /// Retained to-space for the sweep's arena compaction, swapped with
    /// `byte_arena` each collection so steady-state compaction allocates
    /// nothing.
    byte_arena_scratch: Vec<u8>,
    /// Retained to-space for `value_arena`, like `byte_arena_scratch`.
    value_arena_scratch: Vec<Value>,
}

/// Restores non-VM rooting behavior when the VM main loop returns. Dropping it
/// re-enables `push_root` and immediate collection for host/native/setup code.
pub(crate) struct VmGuard {
    flag: Rc<Cell<bool>>,
}

impl Drop for VmGuard {
    fn drop(&mut self) {
        self.flag.set(false);
    }
}

impl Heap {
    /// Creates an empty heap configured from one engine's resource limits.
    pub(crate) fn new(limits: &crate::Limits) -> Self {
        Self {
            slots: Vec::new(),
            meta: Vec::new(),
            holes: Vec::new(),
            cursor: 0,
            limit: 0,
            region_end: 0,
            auth_start: 0,
            current_mark: META_MARK_A,
            live_base: 0,
            dynamic_bytes: 0,
            slot_threshold: limits.initial_gc_threshold(),
            byte_threshold: limits.max_heap_bytes(),
            minimum_slot_headroom: limits.initial_gc_threshold(),
            minimum_byte_headroom: limits
                .initial_gc_threshold()
                .saturating_mul(size_of::<Object>()),
            max_slots: limits.max_heap_slots(),
            max_bytes: limits.max_heap_bytes(),
            mark_worklist: Vec::new(),
            temporary_roots: Rc::new(RefCell::new(Vec::new())),
            host_roots: Rc::new(RefCell::new(HostRoots::default())),
            engine_roots: Vec::new(),
            ports: crate::port::PortStore::new(),
            file_system: None,
            process_context: None,
            clock: None,
            pending_exit: None,
            completed_exit: None,
            rooted_region_depth: 0,
            pending_collection: false,
            trap: false,
            vm_active: Rc::new(Cell::new(false)),
            engine_roots_dirty: false,
            string_cursors: StringCursors::new(),
            byte_arena: Vec::new(),
            byte_arena_scratch: Vec::new(),
            value_arena: Vec::new(),
            value_arena_scratch: Vec::new(),
        }
    }

    /// Flags `engine_roots` as needing a refresh before the next collection.
    pub(crate) fn mark_engine_roots_dirty(&mut self) {
        self.engine_roots_dirty = true;
    }

    /// Marks the VM main loop as active and returns a guard that reverts to
    /// host/native rooting behavior when dropped (covering every `execute` exit).
    pub(crate) fn enter_vm(&self) -> VmGuard {
        self.vm_active.set(true);
        VmGuard {
            flag: self.vm_active.clone(),
        }
    }

    /// Whether the VM main loop is managing rooting through precise scans and no
    /// rooted region (native call) is currently active.
    #[inline(always)]
    fn vm_manages_roots(&self) -> bool {
        self.vm_active.get() && self.rooted_region_depth == 0
    }

    /// Enters a rooted region for the duration of a native call: while active,
    /// [`Self::push_root`] engages (so `NativeContext` allocations stay rooted)
    /// even though the VM main loop is live. Collections inside the region are
    /// driven exclusively by the native context's precise register view via
    /// [`Self::collect_with`].
    pub(crate) fn enter_rooted_region(&mut self) {
        self.rooted_region_depth += 1;
    }

    /// Leaves a rooted region opened by [`Self::enter_rooted_region`].
    pub(crate) fn exit_rooted_region(&mut self) {
        self.rooted_region_depth = self.rooted_region_depth.saturating_sub(1);
    }

    /// Reports whether a deferred collection is pending at the next VM safe point.
    pub(crate) fn needs_collection(&self) -> bool {
        self.pending_collection
    }

    /// Reports whether any rare safe-point condition (a deferred collection or
    /// a completed exit) is pending. This is the single load the VM's safe
    /// point performs on its fast path.
    pub(crate) const fn trap_pending(&self) -> bool {
        self.trap
    }

    /// Opens a temporary-root scope for multi-allocation construction.
    pub(crate) fn scope(&self) -> RootScope {
        RootScope {
            roots: self.temporary_roots.clone(),
            start: self.temporary_roots.borrow().len(),
        }
    }

    pub(crate) fn temporary_root_mark(&self) -> usize {
        self.temporary_roots.borrow().len()
    }

    /// Drops every temporary root pushed after `mark` (see
    /// [`Self::temporary_root_mark`]). The cheap, guard-free equivalent of a
    /// [`Self::scope`] for the native-call hot path.
    pub(crate) fn truncate_temporary_roots(&self, mark: usize) {
        self.temporary_roots.borrow_mut().truncate(mark);
    }

    /// Returns the engine-local port backing store.
    pub(crate) fn ports_mut(&mut self) -> &mut crate::port::PortStore {
        &mut self.ports
    }

    /// Installs or replaces the file-system capability for this heap.
    pub(crate) fn set_file_system(&mut self, file_system: Option<Box<dyn crate::FileSystem>>) {
        self.file_system = file_system;
    }

    pub(crate) fn set_process_context(&mut self, process: Option<Box<dyn crate::ProcessContext>>) {
        self.process_context = process;
    }

    pub(crate) fn set_clock(&mut self, clock: Option<Box<dyn crate::Clock>>) {
        self.clock = clock;
    }

    pub(crate) fn request_exit(&mut self, status: crate::ExitStatus) {
        self.pending_exit = Some(status);
    }

    /// Cheap read used by the dispatch loop's native fast path to avoid the
    /// unconditional `None` store that `take_exit_request` performs.
    pub(crate) const fn exit_request_pending(&self) -> bool {
        self.pending_exit.is_some()
    }

    pub(crate) fn take_exit_request(&mut self) -> Option<crate::ExitStatus> {
        self.pending_exit.take()
    }

    pub(crate) fn complete_exit(&mut self, status: crate::ExitStatus) {
        self.completed_exit = Some(status);
        self.trap = true;
    }

    pub(crate) fn take_completed_exit(&mut self) -> Option<crate::ExitStatus> {
        let status = self.completed_exit.take();
        self.trap = self.pending_collection;
        status
    }

    pub(crate) fn process_context(
        &mut self,
    ) -> Result<&mut (dyn crate::ProcessContext + '_), Error> {
        match &mut self.process_context {
            Some(process) => Ok(&mut **process),
            None => Err(Error::plain(
                ErrorKind::CapabilityDenied,
                "process context is denied because no process capability is installed",
            )),
        }
    }

    pub(crate) fn clock(&mut self) -> Result<&mut (dyn crate::Clock + '_), Error> {
        match &mut self.clock {
            Some(clock) => Ok(&mut **clock),
            None => Err(Error::plain(
                ErrorKind::CapabilityDenied,
                "time is denied because no clock capability is installed",
            )),
        }
    }

    pub(crate) fn open_file(
        &mut self,
        path: &str,
        input: bool,
        binary: bool,
    ) -> Result<crate::port::PortId, Error> {
        let files = self.file_system.as_mut().ok_or_else(|| {
            Error::plain(
                ErrorKind::CapabilityDenied,
                "file access is denied because no file-system capability is installed",
            )
        })?;
        let resource = if input {
            files.open_input(path, binary)
        } else {
            files.open_output(path, binary)
        }
        .map_err(|error| {
            Error::plain(
                ErrorKind::FileError,
                format!("file operation failed: {error}"),
            )
        })?;
        self.ports.host(resource, input, !input, binary)
    }

    pub(crate) fn file_exists(&mut self, path: &str) -> Result<bool, Error> {
        self.file_system
            .as_mut()
            .ok_or_else(|| {
                Error::plain(
                    ErrorKind::CapabilityDenied,
                    "file access is denied because no file-system capability is installed",
                )
            })?
            .exists(path)
            .map_err(|error| {
                Error::plain(
                    ErrorKind::FileError,
                    format!("file operation failed: {error}"),
                )
            })
    }

    pub(crate) fn delete_file(&mut self, path: &str) -> Result<(), Error> {
        self.file_system
            .as_mut()
            .ok_or_else(|| {
                Error::plain(
                    ErrorKind::CapabilityDenied,
                    "file access is denied because no file-system capability is installed",
                )
            })?
            .delete(path)
            .map_err(|error| {
                Error::plain(
                    ErrorKind::FileError,
                    format!("file operation failed: {error}"),
                )
            })
    }

    /// Adds a value that remains live until the active scope is dropped.
    #[inline(always)]
    pub(crate) fn push_root(&self, value: Value) {
        // While the VM main loop manages roots by precise scanning, this is a
        // cheap no-op that also prevents unbounded root growth in allocation-free
        // loops that never reach a collection. Host, native, and setup code
        // (whenever VM-managed rooting is not in effect) still root normally.
        if !self.vm_manages_roots() && value.heap_ref().is_some() {
            self.temporary_roots.borrow_mut().push(value);
        }
    }

    /// Creates a host-visible RAII root for an already-valid engine value.
    pub(crate) fn root(&self, value: Value) -> crate::Root {
        crate::Root::new(self.host_roots.clone(), value)
    }

    pub(crate) fn owns_root(&self, root: &crate::Root) -> bool {
        Rc::ptr_eq(&self.host_roots, &root.roots)
    }

    /// Replaces values owned by the engine (such as globals) used as GC roots.
    pub(crate) fn set_engine_roots(&mut self, values: impl IntoIterator<Item = Value>) {
        self.engine_roots.clear();
        self.engine_roots.extend(values);
        self.engine_roots_dirty = false;
    }

    /// Refreshes the cached engine roots from the live tables, but only when
    /// either dirty signal reports a change: the heap-side flag (set by symbol
    /// interning) or the global store's own mutation flag. Cheap when clean
    /// (two flag reads), so every collection point calls it unconditionally.
    /// Every path that reaches [`Self::collect`] or [`Self::collect_with`]
    /// must have called this (or [`Self::set_engine_roots`]) after the last
    /// globals/symbols mutation, or the collection traces stale roots.
    pub(crate) fn sync_engine_roots(
        &mut self,
        globals: &crate::global::GlobalStore,
        symbols: &std::collections::HashMap<String, Value>,
    ) {
        let globals_dirty = globals.take_dirty();
        if self.engine_roots_dirty || globals_dirty {
            self.set_engine_roots(globals.values().chain(symbols.values()).copied());
        }
    }

    /// Reports whether allocating `bytes` more would cross a soft collection
    /// threshold. Callers holding a precise root view (native contexts) use this
    /// to collect via [`Self::collect_with`] before allocating.
    pub(crate) fn needs_collection_for(&self, bytes: usize) -> bool {
        self.live_slots() >= self.slot_threshold
            || self.total_bytes().saturating_add(bytes) > self.byte_threshold
    }

    /// The instantaneous live slot count: the swept base plus everything the
    /// bump allocator has handed out of its current authorization.
    fn live_slots(&self) -> usize {
        self.live_base + (self.cursor - self.auth_start) as usize
    }

    /// The instantaneous byte footprint: every live slot's 48-byte base plus
    /// the tracked out-of-line payload bytes.
    fn total_bytes(&self) -> usize {
        self.live_slots()
            .saturating_mul(size_of::<Object>())
            .saturating_add(self.dynamic_bytes)
    }

    /// Allocates an object. When the soft threshold is met it collects
    /// first, or arms the safe-point trap instead while the VM is live.
    ///
    /// Kept out of line: the folded fast path made this small enough for
    /// LLVM to inline it into the VM dispatch loop at every allocation arm,
    /// and the added register pressure spilled the loop's retirement counter
    /// to the stack (the exact regression the safe-point design forbids).
    /// Pair allocation has its own out-of-line entry in [`Self::alloc_pair`]
    /// and the native-call path uses [`Self::alloc_hot`], where inlining
    /// cannot disturb the dispatch loop.
    #[inline(never)]
    pub(crate) fn alloc(&mut self, object: Object) -> Result<Value, Error> {
        self.alloc_hot(object)
    }

    /// The inlineable body behind [`Self::alloc`], for allocation-heavy
    /// callers outside the VM dispatch loop (native procedures).
    #[inline]
    pub(crate) fn alloc_hot(&mut self, object: Object) -> Result<Value, Error> {
        let bytes = object.bytes();
        self.alloc_hot_sized(object, bytes)
    }

    /// [`Self::alloc_hot`] for callers that already computed
    /// [`Object::bytes`], so one allocation never sizes its object twice.
    /// Pinned like its caller [`crate::native::NativeContext::alloc`] so the
    /// object is built in place in the slot.
    #[inline(always)]
    pub(crate) fn alloc_hot_sized(&mut self, object: Object, bytes: usize) -> Result<Value, Error> {
        let dynamic = bytes.saturating_sub(size_of::<Object>());
        if dynamic != 0 {
            self.pre_account_dynamic(dynamic)?;
        }
        let value = self.alloc_inner(object)?;
        // Charged only after the slot write cannot fail anymore.
        self.dynamic_bytes = self.dynamic_bytes.saturating_add(dynamic);
        Ok(value)
    }

    /// The inlineable body behind [`Self::alloc_pair`], for the native-call
    /// path (see [`crate::native::NativeContext::pair`]): the pair is
    /// constructed in place from its two register-resident `Value` halves,
    /// so no 48-byte `Object` ever round-trips through the stack.
    #[inline(always)]
    pub(crate) fn alloc_pair_hot(&mut self, car: Value, cdr: Value) -> Result<Value, Error> {
        self.alloc_inner(Object::Pair(car, cdr))
    }

    /// Allocates a pair without routing the (large) `Object` enum through a
    /// by-value call: the pair is constructed in place from its two `Value`
    /// halves and carries no dynamic payload, so the byte bookkeeping in
    /// [`Self::alloc`] vanishes entirely. Same contract otherwise.
    ///
    /// Kept out of line like [`Self::alloc`]: inlining the folded fast path
    /// into the VM's `Cons` arm spilled the dispatch loop's retirement
    /// counter to the stack, which costs more on call-heavy paths than this
    /// call does on cons-heavy ones. Both pins were measured together.
    #[inline(never)]
    pub(crate) fn alloc_pair(&mut self, car: Value, cdr: Value) -> Result<Value, Error> {
        self.alloc_pair_hot(car, cdr)
    }

    /// Allocates a bytevector whose payload lives in the byte arena.
    ///
    /// This is the only construction path for `Object::Bytevector`: it runs
    /// every collection-capable check first, then appends the payload and
    /// writes the owning slot with no collection point in between, so the
    /// recorded arena offset cannot be invalidated by a compaction before
    /// the handle exists.
    pub(crate) fn alloc_bytevector(&mut self, bytes: &[u8]) -> Result<Value, Error> {
        self.ensure_slot_and_bytes(bytes.len())?;
        let (off, len) = self.byte_arena_append(bytes)?;
        let value = self.write_authorized_slot(Object::Bytevector { off, len })?;
        // Charged only after the slot write cannot fail anymore.
        self.dynamic_bytes = self.dynamic_bytes.saturating_add(bytes.len());
        Ok(value)
    }

    /// Allocates a bytevector of `count` copies of one byte directly in the
    /// byte arena. Same ordering contract as [`Self::alloc_bytevector`].
    pub(crate) fn alloc_bytevector_filled(
        &mut self,
        fill: u8,
        count: usize,
    ) -> Result<Value, Error> {
        self.ensure_slot_and_bytes(count)?;
        let off = self.byte_arena.len();
        let end = off.checked_add(count);
        if end.is_none() || end.is_some_and(|end| end > u32::MAX as usize) {
            return Err(Error::plain(
                ErrorKind::HeapLimitExceeded,
                "payload arena exhausted",
            ));
        }
        self.byte_arena.try_reserve(count).map_err(|_| {
            Error::plain(
                ErrorKind::HeapLimitExceeded,
                "payload arena allocation failed",
            )
        })?;
        self.byte_arena.resize(off + count, fill);
        let value = self.write_authorized_slot(Object::Bytevector {
            off: off as u32,
            len: count as u32,
        })?;
        self.dynamic_bytes = self.dynamic_bytes.saturating_add(count);
        Ok(value)
    }

    /// Allocates a string whose UTF-8 payload lives in the byte arena.
    ///
    /// This is the only construction path for `Object::String` and follows
    /// the same ordering contract as [`Self::alloc_bytevector`]: every
    /// collection-capable check runs before the payload is appended, and no
    /// collection point exists between the append and the owning slot
    /// write. `chars` must be the exact char count of `text`.
    pub(crate) fn alloc_string(&mut self, text: &str, chars: usize) -> Result<Value, Error> {
        debug_assert_eq!(text.chars().count(), chars);
        self.ensure_slot_and_bytes(text.len())?;
        let (off, byte_len) = self.byte_arena_append(text.as_bytes())?;
        // Every char is at least one byte, so the append's u32 guard also
        // bounds the char count.
        let value = self.write_authorized_slot(Object::String {
            off,
            byte_len,
            chars: chars as u32,
        })?;
        // Charged only after the slot write cannot fail anymore.
        self.dynamic_bytes = self.dynamic_bytes.saturating_add(text.len());
        Ok(value)
    }

    /// Allocates a string of `count` copies of one character directly in the
    /// byte arena. Same ordering contract as [`Self::alloc_string`].
    pub(crate) fn alloc_string_filled(&mut self, fill: char, count: usize) -> Result<Value, Error> {
        let byte_len = count
            .checked_mul(fill.len_utf8())
            .ok_or_else(|| Error::plain(ErrorKind::HeapLimitExceeded, "payload arena exhausted"))?;
        self.ensure_slot_and_bytes(byte_len)?;
        if count > u32::MAX as usize {
            return Err(Error::plain(
                ErrorKind::HeapLimitExceeded,
                "payload arena exhausted",
            ));
        }
        let off = self.byte_arena.len();
        let end = off.checked_add(byte_len);
        if end.is_none() || end.is_some_and(|end| end > u32::MAX as usize) {
            return Err(Error::plain(
                ErrorKind::HeapLimitExceeded,
                "payload arena exhausted",
            ));
        }
        self.byte_arena.try_reserve(byte_len).map_err(|_| {
            Error::plain(
                ErrorKind::HeapLimitExceeded,
                "payload arena allocation failed",
            )
        })?;
        let mut encoded = [0_u8; 4];
        let encoded = fill.encode_utf8(&mut encoded).as_bytes();
        if encoded.len() == 1 {
            self.byte_arena.resize(off + byte_len, encoded[0]);
        } else {
            for _ in 0..count {
                self.byte_arena.extend_from_slice(encoded);
            }
        }
        let value = self.write_authorized_slot(Object::String {
            off: off as u32,
            byte_len: byte_len as u32,
            chars: count as u32,
        })?;
        self.dynamic_bytes = self.dynamic_bytes.saturating_add(byte_len);
        Ok(value)
    }

    /// Allocates a vector whose payload lives in the value arena.
    ///
    /// This is the only construction path for `Object::Vector` and follows
    /// the same ordering contract as [`Self::alloc_bytevector`]: every
    /// collection-capable check runs before the payload is appended, and no
    /// collection point exists between the append and the owning slot
    /// write. The caller must keep every element in `values` rooted across
    /// the call, because the checks may collect before the payload is
    /// copied in.
    pub(crate) fn alloc_vector(&mut self, values: &[Value]) -> Result<Value, Error> {
        let payload = values.len().saturating_mul(size_of::<Value>());
        self.ensure_slot_and_bytes(payload)?;
        let (off, len) = self.value_arena_append(values)?;
        let value = self.write_authorized_slot(Object::Vector { off, len })?;
        // Charged only after the slot write cannot fail anymore.
        self.dynamic_bytes = self.dynamic_bytes.saturating_add(payload);
        Ok(value)
    }

    /// Allocates a vector of `count` copies of one value, filling the arena
    /// span in place with no temporary element buffer. Same ordering
    /// contract as [`Self::alloc_vector`].
    pub(crate) fn alloc_vector_filled(
        &mut self,
        fill: Value,
        count: usize,
    ) -> Result<Value, Error> {
        let payload = count.saturating_mul(size_of::<Value>());
        self.ensure_slot_and_bytes(payload)?;
        // Whole-span guards, mirroring `value_arena_append`.
        let off = self.value_arena.len();
        let end = off.checked_add(count);
        if end.is_none() || end.is_some_and(|end| end > u32::MAX as usize) {
            return Err(Error::plain(
                ErrorKind::HeapLimitExceeded,
                "payload arena exhausted",
            ));
        }
        self.value_arena.try_reserve(count).map_err(|_| {
            Error::plain(
                ErrorKind::HeapLimitExceeded,
                "payload arena allocation failed",
            )
        })?;
        self.value_arena.resize(off + count, fill);
        let value = self.write_authorized_slot(Object::Vector {
            off: off as u32,
            len: count as u32,
        })?;
        // Charged only after the slot write cannot fail anymore.
        self.dynamic_bytes = self.dynamic_bytes.saturating_add(payload);
        Ok(value)
    }

    /// Appends one vector payload span at the value arena tail and returns
    /// its handle fields. The element-unit counterpart of
    /// [`Self::byte_arena_append`], with the same guarantees.
    fn value_arena_append(&mut self, values: &[Value]) -> Result<(u32, u32), Error> {
        let off = self.value_arena.len();
        let Some(end) = off.checked_add(values.len()) else {
            return Err(Error::plain(
                ErrorKind::HeapLimitExceeded,
                "payload arena exhausted",
            ));
        };
        if end > u32::MAX as usize {
            return Err(Error::plain(
                ErrorKind::HeapLimitExceeded,
                "payload arena exhausted",
            ));
        }
        self.value_arena.try_reserve(values.len()).map_err(|_| {
            Error::plain(
                ErrorKind::HeapLimitExceeded,
                "payload arena allocation failed",
            )
        })?;
        self.value_arena.extend_from_slice(values);
        // The guard above keeps both fields in range.
        Ok((off as u32, values.len() as u32))
    }

    /// Allocates the concatenation of string values whose payloads already
    /// live in the byte arena, copying each part's span directly into the
    /// result span with no temporary and no re-validation (concatenated
    /// valid UTF-8 stays valid). The caller must have type-validated every
    /// part and summed the sizes from the handles. Part spans are resolved
    /// only after [`Self::ensure_slot_and_bytes`], because its collection
    /// point moves spans, and values survive it through the caller's roots
    /// with their handles rewritten in place.
    pub(crate) fn alloc_string_concat(
        &mut self,
        parts: &[Value],
        total_bytes: usize,
        total_chars: usize,
    ) -> Result<Value, Error> {
        self.ensure_slot_and_bytes(total_bytes)?;
        // Whole-result guards, mirroring `byte_arena_append`.
        let off = self.byte_arena.len();
        let end = off.checked_add(total_bytes);
        if end.is_none() || end.is_some_and(|end| end > u32::MAX as usize) {
            return Err(Error::plain(
                ErrorKind::HeapLimitExceeded,
                "payload arena exhausted",
            ));
        }
        self.byte_arena.try_reserve(total_bytes).map_err(|_| {
            Error::plain(
                ErrorKind::HeapLimitExceeded,
                "payload arena allocation failed",
            )
        })?;
        for part in parts {
            let handle = part.heap_ref().and_then(|reference| {
                match self
                    .slots
                    .get(reference.0 as usize)
                    .and_then(Option::as_ref)
                {
                    Some(Object::String { off, byte_len, .. }) => {
                        Some((*off as usize, *byte_len as usize))
                    }
                    _ => None,
                }
            });
            // Unreachable after the caller's validation: nothing between it
            // and this loop can change a value's object kind.
            let Some((start, length)) = handle else {
                return Err(Error::plain(
                    ErrorKind::TypeError,
                    "string-append argument is not a string",
                ));
            };
            if start.saturating_add(length) > self.byte_arena.len() {
                return Err(Error::plain(
                    ErrorKind::HeapLimitExceeded,
                    "string handle escaped its arena",
                ));
            }
            self.byte_arena.extend_from_within(start..start + length);
        }
        debug_assert_eq!(self.byte_arena.len(), off + total_bytes);
        let value = self.write_authorized_slot(Object::String {
            off: off as u32,
            byte_len: total_bytes as u32,
            chars: total_chars as u32,
        })?;
        // Charged only after the slot write cannot fail anymore.
        self.dynamic_bytes = self.dynamic_bytes.saturating_add(total_bytes);
        Ok(value)
    }

    /// Prepares an allocation carrying `payload_bytes` of out-of-line
    /// payload: runs the byte accounting (which may collect or arm the
    /// safe-point trap, exactly like the plain allocation path), then
    /// secures an authorized slot. On return the arena append and the
    /// following [`Self::write_authorized_slot`] cannot collect, which is
    /// the ordering the payload constructors rely on.
    fn ensure_slot_and_bytes(&mut self, payload_bytes: usize) -> Result<(), Error> {
        if payload_bytes != 0 {
            self.pre_account_dynamic(payload_bytes)?;
        }
        if self.cursor >= self.limit {
            self.refill_span()?;
        }
        Ok(())
    }

    /// Appends one payload span at the arena tail and returns its handle
    /// fields. Fails without touching the arena when the span would push the
    /// arena past the `u32` offset range or the host allocator refuses the
    /// growth. Performs no collection.
    fn byte_arena_append(&mut self, bytes: &[u8]) -> Result<(u32, u32), Error> {
        let off = self.byte_arena.len();
        let Some(end) = off.checked_add(bytes.len()) else {
            return Err(Error::plain(
                ErrorKind::HeapLimitExceeded,
                "payload arena exhausted",
            ));
        };
        if end > u32::MAX as usize {
            return Err(Error::plain(
                ErrorKind::HeapLimitExceeded,
                "payload arena exhausted",
            ));
        }
        self.byte_arena.try_reserve(bytes.len()).map_err(|_| {
            Error::plain(
                ErrorKind::HeapLimitExceeded,
                "payload arena allocation failed",
            )
        })?;
        self.byte_arena.extend_from_slice(bytes);
        // The guard above keeps both fields in range.
        Ok((off as u32, bytes.len() as u32))
    }

    /// The shared allocation path behind [`Self::alloc`] and
    /// [`Self::alloc_pair`]. `inline(always)` so a caller with a statically
    /// known variant builds the object directly into the slot. Every
    /// threshold and limit check lives in the refill slow path: the span
    /// `cursor..limit` is pre-authorized against the soft thresholds and
    /// hard limits, so the fast path is one compare and the writes.
    #[inline(always)]
    fn alloc_inner(&mut self, object: Object) -> Result<Value, Error> {
        let index = self.cursor;
        // The refill guarantees limit is at most the arena length, so the
        // span compare already proves both accesses below are in bounds and
        // their checks fold into the same never-taken cold branch.
        if index >= self.limit {
            return self.alloc_refill(object);
        }
        let (Some(slot), Some(byte)) = (
            self.slots.get_mut(index as usize),
            self.meta.get_mut(index as usize),
        ) else {
            return self.alloc_refill(object);
        };
        // Writing the whole meta byte resets any immutability the slot's
        // prior tenant carried.
        *slot = Some(object);
        *byte = META_YOUNG;
        self.cursor = index + 1;
        Ok(Value::heap(GcRef(index)))
    }

    /// Refills the bump allocator and performs the allocation. Out of line
    /// so the bump fast path stays small at every inlined allocation site.
    #[cold]
    #[inline(never)]
    fn alloc_refill(&mut self, object: Object) -> Result<Value, Error> {
        self.refill_span()?;
        self.write_authorized_slot(object)
    }

    /// Refills the bump allocator: retires the consumed span, runs the
    /// deferred soft-threshold and hard-limit checks, then authorizes a new
    /// span clamped to the nearer of the next soft crossing and the hard
    /// allowance. On success at least one slot is authorized
    /// (`cursor < limit`) and no collection point remains before that slot
    /// is written: the payload-arena constructors rely on this to append
    /// payload bytes between the refill and the owning slot write without a
    /// compaction invalidating the recorded offset.
    #[cold]
    #[inline(never)]
    fn refill_span(&mut self) -> Result<(), Error> {
        let mut collected = false;
        loop {
            // Retire the consumed authorization so accounting is exact.
            self.live_base += (self.cursor - self.auth_start) as usize;
            self.auth_start = self.cursor;
            let live = self.live_base;
            let bytes = self.total_bytes();

            // Soft thresholds: while the VM is live the register file holds
            // roots that `temporary_roots` does not reflect, so collection
            // is deferred to the next safe point. Host/setup code collects
            // right away, at most once per refill.
            if live >= self.slot_threshold || bytes >= self.byte_threshold {
                if self.vm_active.get() {
                    self.pending_collection = true;
                    self.trap = true;
                } else if !collected {
                    collected = true;
                    self.collect();
                    continue;
                }
            }

            // Hard limits, folded into one slot allowance. Base allocations
            // cost exactly one slot and 48 bytes, so both limits reduce to
            // a span length and the fast path needs no checks at all.
            let slot_allowance = self.max_slots.saturating_sub(live);
            let byte_allowance = self.max_bytes.saturating_sub(bytes) / size_of::<Object>();
            let allowance = slot_allowance.min(byte_allowance);
            if allowance == 0 {
                return Err(Error::plain(
                    ErrorKind::HeapLimitExceeded,
                    "heap limit exceeded",
                ));
            }

            // While no collection is pending, stop the span at the nearer
            // soft threshold so the refill after the crossing arms the trap.
            // The crossing allocation itself still succeeds.
            let span_budget = if self.pending_collection {
                allowance
            } else {
                let soft_slots = self.slot_threshold.saturating_sub(live);
                let soft_bytes = self.byte_threshold.saturating_sub(bytes) / size_of::<Object>();
                soft_slots.min(soft_bytes).max(1).min(allowance)
            };

            // Authorize within the current hole first, then the next hole,
            // then grow the arena.
            if self.cursor < self.region_end {
                let span = span_budget.min((self.region_end - self.cursor) as usize);
                self.limit = self.cursor + span as u32;
                break;
            }
            if let Some(hole) = self.holes.pop() {
                let span = span_budget.min((hole.end - hole.start) as usize);
                self.cursor = hole.start;
                self.auth_start = hole.start;
                self.region_end = hole.end;
                self.limit = hole.start + span as u32;
                break;
            }
            let len = self.slots.len();
            if len >= u32::MAX as usize {
                return Err(Error::plain(
                    ErrorKind::HeapLimitExceeded,
                    "heap slot index exhausted",
                ));
            }
            // Grow in chunks bounded by the hard slot limit and the index
            // width. The soft thresholds keep growth in check by scheduling
            // a collection long before the hard limit.
            let room = self.max_slots.max(len + 1) - len;
            let grow = (len / 2)
                .max(64)
                .min(room)
                .max(1)
                .min(u32::MAX as usize - len);
            self.slots.resize_with(len + grow, || None);
            self.meta.resize(len + grow, META_FREE);
            let span = span_budget.min(grow);
            self.cursor = len as u32;
            self.auth_start = len as u32;
            self.region_end = (len + grow) as u32;
            self.limit = (len + span) as u32;
            break;
        }
        Ok(())
    }

    /// Writes an object into the authorized slot. The caller must have
    /// secured `cursor < limit` through a successful [`Self::refill_span`]
    /// (or the folded fast-path compare), so the write cannot fail and
    /// performs no collection.
    #[inline(always)]
    fn write_authorized_slot(&mut self, object: Object) -> Result<Value, Error> {
        let index = self.cursor;
        if index < self.limit
            && let Some(slot) = self.slots.get_mut(index as usize)
            && let Some(byte) = self.meta.get_mut(index as usize)
        {
            *slot = Some(object);
            *byte = META_YOUNG;
            self.cursor = index + 1;
            return Ok(Value::heap(GcRef(index)));
        }
        // Unreachable: an authorized span always has room for one slot.
        Err(Error::plain(
            ErrorKind::HeapLimitExceeded,
            "heap allocator has no usable slot",
        ))
    }

    /// Byte bookkeeping for an allocation with an out-of-line payload, run
    /// before the slot write. Performs the byte-threshold and byte-limit
    /// checks the folded fast path cannot see, then tightens the authorized
    /// span so subsequent base allocations cannot overshoot the byte limit.
    #[inline(never)]
    fn pre_account_dynamic(&mut self, dynamic: usize) -> Result<(), Error> {
        let incoming = size_of::<Object>().saturating_add(dynamic);
        let mut after = self.total_bytes().saturating_add(incoming);
        if after > self.byte_threshold {
            if self.vm_active.get() {
                self.pending_collection = true;
                self.trap = true;
            } else {
                self.collect();
                after = self.total_bytes().saturating_add(incoming);
            }
        }
        if after > self.max_bytes {
            return Err(Error::plain(
                ErrorKind::HeapLimitExceeded,
                "heap limit exceeded",
            ));
        }
        // Re-clamp the span: this payload consumes byte allowance that the
        // refill distributed as 48-byte slots. The clamp counts the current
        // allocation's own slot, which the addition above already covers.
        let remaining_slots = (self.max_bytes - after) / size_of::<Object>();
        let clamp = u32::try_from(remaining_slots.saturating_add(1))
            .unwrap_or(u32::MAX)
            .saturating_add(self.cursor);
        self.limit = self.limit.min(clamp);
        Ok(())
    }

    /// Performs iterative mark-and-sweep collection from all current roots.
    pub(crate) fn collect(&mut self) {
        self.collect_with(&|_| {});
    }

    /// Performs a collection whose root set additionally includes every value
    /// appended by `extra_roots`. The VM and native contexts pass a callback
    /// that enumerates the live register file and frame state directly, so no
    /// eager per-call root snapshot is ever materialized. `extra_roots` is a
    /// `Fn` (not `FnOnce`): several collections can run during one native call.
    pub(crate) fn collect_with(&mut self, extra_roots: &RootGatherer<'_>) {
        self.pending_collection = false;
        self.trap = self.completed_exit.is_some();
        self.string_cursors.clear();
        let previous_slots = self.live_slots();
        let previous_bytes = self.total_bytes();
        // Rotating the color makes every prior mark stale without touching
        // it. Objects allocated since the last collection carry YOUNG, which
        // never equals a mark color, so they are dead unless re-marked here.
        self.current_mark ^= 1;
        let current_mark = self.current_mark;
        let mut worklist = std::mem::take(&mut self.mark_worklist);
        worklist.clear();
        worklist.extend(self.temporary_roots.borrow().iter().copied());
        worklist.extend(self.host_roots.borrow().values().copied());
        worklist.extend(self.engine_roots.iter().copied());
        extra_roots(&mut worklist);
        while let Some(value) = worklist.pop() {
            let Some(reference) = value.heap_ref() else {
                continue;
            };
            let index = reference.0 as usize;
            let Some(byte) = self.meta.get_mut(index) else {
                continue;
            };
            let state = *byte & META_STATE_MASK;
            // The FREE skip keeps over-approximated roots safe: the VM's
            // register scan may hand in stale references to freed slots.
            if state == current_mark || state == META_FREE {
                continue;
            }
            *byte = (*byte & META_IMMUTABLE) | current_mark;
            if let Some(object) = self.slots.get(index).and_then(Option::as_ref) {
                object.trace(&mut worklist, &self.value_arena);
            }
        }
        let collected_ports = self.sweep();
        worklist.clear();
        self.mark_worklist = worklist;
        self.slot_threshold = adaptive_threshold(
            previous_slots,
            self.live_slots(),
            self.minimum_slot_headroom,
            self.max_slots,
        );
        self.byte_threshold = adaptive_threshold(
            previous_bytes,
            self.total_bytes(),
            self.minimum_byte_headroom,
            self.max_bytes,
        );
        for port in collected_ports {
            self.ports.finalize(port);
        }
    }

    /// Reclaims every slot the mark phase left unmarked, rebuilds the free
    /// index, and recounts live storage. Scans the metadata table eight
    /// bytes at a time so fully free runs never touch slot memory. Dead
    /// slots are touched exactly once to drop their object. Survivors are
    /// read for their byte size but never written: recomputing bytes from
    /// the (usually few) survivors is much cheaper than sizing every dead
    /// object in churn-heavy workloads, and it also self-corrects any
    /// capacity growth a mutation caused since the last collection.
    fn sweep(&mut self) -> Vec<crate::port::PortId> {
        // Every state byte equal to the current color, replicated lane-wise.
        let live_word = u64::from(self.current_mark) * 0x0101_0101_0101_0101;
        const STATE_WORD: u64 = 0x0707_0707_0707_0707;
        // Compact the payload arenas by evacuation: survivors copy their
        // spans into the retained to-spaces in scan order and have their
        // handles rewritten in place, dead spans are simply left behind in
        // the from-spaces. Single ownership (one slot per span) is what
        // makes this safe without forwarding state.
        let from_arena = std::mem::replace(
            &mut self.byte_arena,
            std::mem::take(&mut self.byte_arena_scratch),
        );
        self.byte_arena.clear();
        let from_values = std::mem::replace(
            &mut self.value_arena,
            std::mem::take(&mut self.value_arena_scratch),
        );
        self.value_arena.clear();
        // The bump region is retired before holes are rebuilt: its remainder
        // is free metadata, so the scan below re-records it as a hole. Not
        // resetting would alias the region with the rebuilt hole list. The
        // sweep recounts live storage from scratch, so the consumed span
        // needs no retirement into the base.
        self.cursor = 0;
        self.limit = 0;
        self.region_end = 0;
        self.auth_start = 0;
        self.holes.clear();
        let mut run_start: Option<u32> = None;
        let mut live = 0usize;
        let mut live_bytes = 0usize;
        let mut collected_ports = Vec::new();
        let len = self.meta.len();
        let mut index = 0;
        while index + 8 <= len {
            let Some(chunk) = self.meta.get(index..index + 8) else {
                break;
            };
            let Ok(states) = <[u8; 8]>::try_from(chunk) else {
                break;
            };
            let word = u64::from_ne_bytes(states);
            if word == 0 {
                // Eight free slots. Valid because freeing clears the whole
                // byte and the immutable bit is never set on a free slot.
                if run_start.is_none() {
                    run_start = Some(index as u32);
                }
                index += 8;
                continue;
            }
            if word & STATE_WORD == live_word {
                // Eight survivors: size reads plus payload evacuation, no
                // metadata writes.
                if let Some(start) = run_start.take() {
                    self.holes.push(Hole {
                        start,
                        end: index as u32,
                    });
                }
                live += 8;
                for position in index..index + 8 {
                    if let Some(object) = self.slots.get_mut(position).and_then(Option::as_mut) {
                        live_bytes = live_bytes.saturating_add(sweep_survivor(
                            object,
                            &from_arena,
                            &mut self.byte_arena,
                            &from_values,
                            &mut self.value_arena,
                        ));
                    }
                }
                index += 8;
                continue;
            }
            for position in index..index + 8 {
                self.sweep_slot(
                    position,
                    &from_arena,
                    &from_values,
                    &mut collected_ports,
                    &mut live,
                    &mut live_bytes,
                    &mut run_start,
                );
            }
            index += 8;
        }
        for position in index..len {
            self.sweep_slot(
                position,
                &from_arena,
                &from_values,
                &mut collected_ports,
                &mut live,
                &mut live_bytes,
                &mut run_start,
            );
        }
        if let Some(start) = run_start.take() {
            self.holes.push(Hole {
                start,
                end: len as u32,
            });
        }
        // Recorded ascending, reversed so pop consumes the lowest-address
        // hole first and reuse stays dense.
        self.holes.reverse();
        self.live_base = live;
        self.dynamic_bytes = live_bytes.saturating_sub(live.saturating_mul(size_of::<Object>()));
        // Retain the drained from-spaces as the next collection's to-spaces
        // so steady-state compaction allocates nothing.
        self.byte_arena_scratch = from_arena;
        self.byte_arena_scratch.clear();
        self.value_arena_scratch = from_values;
        self.value_arena_scratch.clear();
        collected_ports
    }

    /// Processes one metadata byte during the sweep: survivors count as
    /// live, contribute their byte size, and have their payload span
    /// evacuated into the compacted arena. Dead slots drop their object,
    /// queue port finalization, and join the free run. Pinned inline: the
    /// arena parameter pushed this over LLVM's implicit inline budget, and
    /// an out-of-line call per swept slot cost a measured +6% instructions
    /// on list_churn.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn sweep_slot(
        &mut self,
        index: usize,
        from_arena: &[u8],
        from_values: &[Value],
        collected_ports: &mut Vec<crate::port::PortId>,
        live: &mut usize,
        live_bytes: &mut usize,
        run_start: &mut Option<u32>,
    ) {
        let Some(byte) = self.meta.get_mut(index) else {
            return;
        };
        let state = *byte & META_STATE_MASK;
        if state == self.current_mark {
            if let Some(start) = run_start.take() {
                self.holes.push(Hole {
                    start,
                    end: index as u32,
                });
            }
            *live += 1;
            if let Some(object) = self.slots.get_mut(index).and_then(Option::as_mut) {
                *live_bytes = live_bytes.saturating_add(sweep_survivor(
                    object,
                    from_arena,
                    &mut self.byte_arena,
                    from_values,
                    &mut self.value_arena,
                ));
            }
            return;
        }
        if state != META_FREE {
            *byte = META_FREE;
            if let Some(slot) = self.slots.get_mut(index) {
                if let Some(Object::Port(port)) = slot.as_ref() {
                    collected_ports.push(port.id);
                }
                clear_dead_slot(slot);
            }
        }
        if run_start.is_none() {
            *run_start = Some(index as u32);
        }
    }

    /// Counts the slots currently holding an object.
    #[cfg(test)]
    fn live_len(&self) -> usize {
        self.meta
            .iter()
            .filter(|byte| **byte & META_STATE_MASK != META_FREE)
            .count()
    }

    /// Asserts the metadata and slot invariants. Byte accounting
    /// is only exact immediately after a collection (a mutation can grow an
    /// object's capacity between collections), so tests assert it separately
    /// via [`Self::recomputed_bytes`] at those points.
    #[cfg(test)]
    fn check_invariants(&self) {
        assert_eq!(self.meta.len(), self.slots.len());
        let mut live = 0usize;
        for (index, slot) in self.slots.iter().enumerate() {
            let state = self.meta[index] & META_STATE_MASK;
            assert_eq!(state == META_FREE, slot.is_none());
            if slot.is_some() {
                live += 1;
            }
        }
        assert_eq!(live, self.live_slots());
    }

    /// Sums the byte footprint of every live object.
    #[cfg(test)]
    fn recomputed_bytes(&self) -> usize {
        self.slots
            .iter()
            .flatten()
            .map(Object::bytes)
            .fold(0usize, usize::saturating_add)
    }

    /// Reports whether an internal test value still denotes an allocated slot.
    #[cfg(test)]
    pub(crate) fn contains(&self, value: Value) -> bool {
        match value.heap_ref() {
            Some(reference) => self
                .slots
                .get(reference.0 as usize)
                .is_some_and(Option::is_some),
            None => true,
        }
    }
}

/// Empties one dead slot, running the object's `Drop` in place. Kept out of
/// line and pointer-based so the `Object` drop glue neither inlines into
/// the sweep loops (with it inlined the sweep's code footprint tripled its
/// cycle cost on list_churn) nor moves the 48-byte object by value.
#[inline(never)]
fn clear_dead_slot(slot: &mut Option<Object>) {
    *slot = None;
}

/// Sizes one surviving object and, when it owns an arena payload span,
/// evacuates the span from the pre-collection arena into the compacted
/// arena and rewrites the handle's offset in place. Single ownership makes
/// the copy unconditional: no other slot can reference the same span.
/// Sizing and evacuation share one discriminant dispatch and the helper is
/// pinned inline, because the sweep pays this per survivor (out of line it
/// cost a measured +6% instructions on list_churn).
#[inline(always)]
fn sweep_survivor(
    object: &mut Object,
    from_arena: &[u8],
    to_arena: &mut Vec<u8>,
    from_values: &[Value],
    to_values: &mut Vec<Value>,
) -> usize {
    match object {
        Object::Vector { off, len } => {
            let start = *off as usize;
            let length = *len as usize;
            let span = from_values.get(start..start + length);
            debug_assert!(span.is_some(), "vector handle escaped its arena");
            let Some(span) = span else {
                // Unreachable by construction. Empty the handle rather than
                // aliasing unrelated elements.
                *off = 0;
                *len = 0;
                return size_of::<Object>();
            };
            // The to-space never outgrows the from-space, so the new offset
            // stays within the u32 range the allocator guards.
            *off = to_values.len() as u32;
            to_values.extend_from_slice(span);
            size_of::<Object>().saturating_add(length.saturating_mul(size_of::<Value>()))
        }
        Object::Bytevector { off, len } => {
            let start = *off as usize;
            let length = *len as usize;
            let span = from_arena.get(start..start + length);
            debug_assert!(span.is_some(), "bytevector handle escaped its arena");
            let Some(span) = span else {
                // Unreachable by construction. Empty the handle rather than
                // aliasing unrelated bytes.
                *off = 0;
                *len = 0;
                return size_of::<Object>();
            };
            // The to-space never outgrows the from-space, so the new offset
            // stays within the u32 range the allocator guards.
            *off = to_arena.len() as u32;
            to_arena.extend_from_slice(span);
            size_of::<Object>().saturating_add(length)
        }
        Object::String {
            off,
            byte_len,
            chars,
        } => {
            let start = *off as usize;
            let length = *byte_len as usize;
            let span = from_arena.get(start..start + length);
            debug_assert!(span.is_some(), "string handle escaped its arena");
            let Some(span) = span else {
                // Unreachable by construction. Empty the handle rather than
                // aliasing unrelated bytes.
                *off = 0;
                *byte_len = 0;
                *chars = 0;
                return size_of::<Object>();
            };
            // Whole-span copies preserve UTF-8 validity and the offsets any
            // cached cursor resolved, and the cursors were cleared at the
            // start of this collection anyway.
            *off = to_arena.len() as u32;
            to_arena.extend_from_slice(span);
            size_of::<Object>().saturating_add(length)
        }
        _ => object.bytes(),
    }
}

/// Selects more headroom when a collection recovered little and less when it
/// recovered most of the tracked storage. The next threshold lands at four to
/// six times the live storage. Total collection cost tracks the heap-to-live
/// ratio, so the wider headroom divides the number of mark phases directly.
/// Measured against the 2x-3x policy this cut instructions on the
/// allocation-heavy benchmarks without moving the allocation-light ones.
fn adaptive_threshold(previous: usize, live: usize, minimum: usize, maximum: usize) -> usize {
    let reclaimed = previous.saturating_sub(live);
    let headroom = if previous == 0 || reclaimed < previous / 4 {
        live.saturating_mul(5)
    } else if reclaimed > previous.saturating_sub(previous / 4) {
        live.saturating_mul(3)
    } else {
        live.saturating_mul(4)
    };
    live.saturating_add(headroom.max(minimum)).min(maximum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EngineConfig, Limits};

    #[test]
    fn roots_and_cycles_survive_then_collect() {
        let mut heap = Heap::new(
            EngineConfig::default()
                .with_limits(Limits::default().with_initial_gc_threshold(1))
                .limits(),
        );
        let value = heap
            .alloc(Object::Pair(Value::nil(), Value::nil()))
            .unwrap();
        let root = heap.root(value);
        heap.collect();
        assert!(heap.contains(value));
        drop(root);
        heap.collect();
        assert!(!heap.contains(value));
    }

    #[test]
    fn recycled_host_root_slots_do_not_unroot_live_values() {
        let mut heap = Heap::new(&Limits::default());
        let first_value = heap.alloc_vector(&[Value::integer(1)]).unwrap();
        let first_root = heap.root(first_value);

        // Cloning uses a distinct slab entry. Dropping it makes that entry
        // available for the next root without affecting the original.
        let cloned_root = first_root.clone();
        drop(cloned_root);
        let recycled_value = heap.alloc_vector(&[Value::integer(2)]).unwrap();
        let recycled_root = heap.root(recycled_value);
        drop(recycled_root);
        heap.collect();

        assert!(
            heap.contains(first_root.value()),
            "recycling a host-root slab entry removed another live root"
        );
        drop(first_root);
        heap.collect();
        assert!(!heap.contains(first_value));
    }

    #[test]
    fn scoped_root_traces_edges() {
        let mut heap = Heap::new(
            EngineConfig::default()
                .with_limits(Limits::default().with_initial_gc_threshold(1))
                .limits(),
        );
        let _scope = heap.scope();
        let child = heap.alloc_vector(&[]).unwrap();
        heap.push_root(child);
        let parent = heap.alloc(Object::Pair(child, Value::nil())).unwrap();
        heap.push_root(parent);
        heap.collect();
        assert!(heap.contains(child));
    }

    #[test]
    fn byte_estimates_include_base_and_owned_capacity() {
        let base = size_of::<Object>();
        assert_eq!(
            Object::Vector { off: 0, len: 7 }.bytes(),
            base + 7 * size_of::<Value>()
        );
        // Boxed variants count their out-of-line payload as well.
        let irritants = Vec::<Value>::with_capacity(5);
        assert_eq!(
            Object::Error(Box::new(ErrorObject {
                message: Value::nil(),
                irritants,
                kind: ConditionKind::Error,
            }))
            .bytes(),
            base + size_of::<ErrorObject>() + 5 * size_of::<Value>()
        );
        let mapping = Vec::<usize>::with_capacity(3);
        assert_eq!(
            Object::RecordProcedure(Box::new(RecordProcedure::Constructor {
                record_type: Value::nil(),
                fields: 0,
                mapping,
            }))
            .bytes(),
            base + size_of::<RecordProcedure>() + 3 * size_of::<usize>()
        );
    }

    #[test]
    fn adaptive_threshold_responds_to_reclamation_yield() {
        assert_eq!(adaptive_threshold(100, 90, 1, 1_000), 540);
        assert_eq!(adaptive_threshold(100, 50, 1, 1_000), 250);
        assert_eq!(adaptive_threshold(100, 10, 1, 1_000), 40);
        assert_eq!(adaptive_threshold(usize::MAX, usize::MAX, 1, 500), 500);
    }

    #[test]
    fn byte_pressure_collects_before_enforcing_the_hard_limit() {
        let limits = Limits::default()
            .with_initial_gc_threshold(100)
            .with_max_heap_slots(100);
        let mut heap = Heap::new(&limits);
        let first = heap.alloc_vector(&[Value::nil(); 16]).unwrap();
        // Thresholds only ever move at a collection, so a test poking them
        // directly must also invalidate the authorized span for the change
        // to be seen before the next natural refill.
        heap.byte_threshold = heap.total_bytes();
        heap.limit = heap.cursor;
        let second = heap.alloc_vector(&[Value::nil(); 16]).unwrap();
        assert_eq!(first, second);
        assert_eq!(heap.live_len(), 1);
        assert!(heap.contains(second));
        heap.check_invariants();
    }

    #[test]
    fn sweep_tracks_survivors_and_reused_slots_once() {
        let mut heap = Heap::new(&Limits::default());
        let dead = heap.alloc_vector(&[]).unwrap();
        let live = heap.alloc_vector(&[]).unwrap();
        let root = heap.root(live);
        heap.collect();
        assert_eq!(heap.live_len(), 1);
        assert!(!heap.contains(dead));
        heap.check_invariants();

        let reused = heap.alloc_vector(&[]).unwrap();
        assert_eq!(heap.live_len(), 2);
        assert_ne!(live, reused);
        drop(root);
        heap.collect();
        assert_eq!(heap.live_len(), 0);
        heap.check_invariants();
    }

    #[test]
    fn unreachable_ports_are_removed_after_sweeping() {
        let mut heap = Heap::new(&Limits::default());
        let id = heap.ports_mut().new_text_output().unwrap();
        let port = heap
            .alloc(Object::Port(crate::port::PortObject { id }))
            .unwrap();
        assert!(heap.contains(port));
        assert!(heap.ports.contains(id));
        heap.collect();
        assert!(!heap.contains(port));
        assert_eq!(heap.live_len(), 0);
        assert!(!heap.ports.contains(id));
    }

    #[test]
    fn mark_rotation_never_resurrects_stale_colors() {
        let mut heap = Heap::new(&Limits::default());
        let survivor = heap
            .alloc(Object::Pair(Value::nil(), Value::nil()))
            .unwrap();
        let root = heap.root(survivor);
        // Three collections cover both color rotations. Fresh garbage in
        // every cycle must be reclaimed even when its slot last held the
        // color that becomes current again.
        for _ in 0..3 {
            let garbage = heap.alloc_vector(&[survivor]).unwrap();
            heap.collect();
            assert!(heap.contains(survivor));
            assert!(!heap.contains(garbage));
            for byte in &heap.meta {
                let state = byte & META_STATE_MASK;
                assert!(state == META_FREE || state == heap.current_mark);
            }
            heap.check_invariants();
        }
        drop(root);
        heap.collect();
        assert_eq!(heap.live_len(), 0);
    }

    #[test]
    fn slot_reuse_clears_immutability() {
        let mut heap = Heap::new(&Limits::default());
        let frozen = heap
            .alloc(Object::Pair(Value::nil(), Value::nil()))
            .unwrap();
        heap.make_immutable(frozen);
        assert!(!heap.set_pair_car(frozen, Value::nil()));
        heap.collect();
        let reused = heap
            .alloc(Object::Pair(Value::nil(), Value::nil()))
            .unwrap();
        assert_eq!(frozen.heap_ref(), reused.heap_ref());
        assert!(heap.set_pair_car(reused, Value::nil()));
        heap.check_invariants();
    }

    #[test]
    fn byte_accounting_survives_capacity_growth() {
        let mut heap = Heap::new(&Limits::default());
        let text = heap.alloc_string("aaaa", 4).unwrap();
        let root = heap.root(text);
        // A widening write rebuilds the span at the arena tail and leaves
        // the old span behind as garbage. The sweep recomputes byte
        // accounting from survivors, so the next collection captures the
        // growth exactly and reclaims the abandoned span.
        assert!(heap.string_set(text, 1, '\u{00e9}').unwrap());
        assert_eq!(heap.string(text).unwrap(), "a\u{00e9}aa");
        heap.collect();
        assert_eq!(heap.total_bytes(), heap.recomputed_bytes());
        assert_eq!(heap.byte_arena.len(), 5);
        heap.check_invariants();
        drop(root);
        heap.collect();
        assert_eq!(heap.total_bytes(), 0);
        heap.check_invariants();
    }

    #[test]
    fn string_width_changes_respect_the_hard_byte_limit() {
        let limits = Limits::default()
            .with_initial_gc_threshold(1)
            .with_max_heap_bytes(size_of::<Object>() + 8);
        let mut heap = Heap::new(&limits);
        let text = heap.alloc_string("a", 1).unwrap();
        let _root = heap.root(text);

        assert!(heap.string_set(text, 0, '\u{1f600}').unwrap());
        assert!(heap.string_set(text, 0, 'a').unwrap());
        let error = heap.string_set(text, 0, '\u{1f600}').unwrap_err();
        assert_eq!(error.kind(), ErrorKind::HeapLimitExceeded);
        assert!(heap.total_bytes() <= limits.max_heap_bytes());
    }

    #[test]
    fn string_payloads_compact_and_survive_collection() {
        let mut heap = Heap::new(&Limits::default());
        let dead = heap.alloc_string("garbage", 7).unwrap();
        let live = heap.alloc_string("h\u{00e9}llo", 5).unwrap();
        let empty = heap.alloc_string("", 0).unwrap();
        let live_root = heap.root(live);
        let empty_root = heap.root(empty);
        heap.collect();
        // The dead span is left behind and the survivor is evacuated to the
        // front of the compacted arena with its handle rewritten.
        assert!(!heap.contains(dead));
        assert_eq!(heap.string(live).unwrap(), "h\u{00e9}llo");
        assert_eq!(heap.string(empty).unwrap(), "");
        assert_eq!(heap.byte_arena.len(), 6);
        match heap
            .slots
            .get(live.heap_ref().unwrap().0 as usize)
            .and_then(Option::as_ref)
        {
            Some(Object::String {
                off,
                byte_len,
                chars,
            }) => {
                assert_eq!(*off, 0);
                assert_eq!(*byte_len, 6);
                assert_eq!(*chars, 5);
            }
            other => panic!("expected a string handle, found {other:?}"),
        }
        assert_eq!(heap.total_bytes(), heap.recomputed_bytes());
        heap.check_invariants();
        drop(live_root);
        drop(empty_root);
        heap.collect();
        assert_eq!(heap.byte_arena.len(), 0);
        assert_eq!(heap.total_bytes(), 0);
        heap.check_invariants();
    }

    #[test]
    fn string_set_width_changes_persist_across_compaction() {
        let mut heap = Heap::new(&Limits::default());
        let text = heap.alloc_string("abcd", 4).unwrap();
        let _root = heap.root(text);
        // Widen, verify through a compaction, then narrow back.
        assert!(heap.string_set(text, 2, '\u{20ac}').unwrap());
        assert_eq!(heap.string(text).unwrap(), "ab\u{20ac}d");
        assert_eq!(heap.string_len(text), Some(4));
        assert_eq!(heap.string_ref(text, 2), Some('\u{20ac}'));
        heap.collect();
        assert_eq!(heap.string(text).unwrap(), "ab\u{20ac}d");
        assert_eq!(heap.byte_arena.len(), 6);
        assert_eq!(heap.total_bytes(), heap.recomputed_bytes());
        assert!(heap.string_set(text, 2, 'x').unwrap());
        assert_eq!(heap.string(text).unwrap(), "abxd");
        heap.collect();
        assert_eq!(heap.byte_arena.len(), 4);
        assert_eq!(heap.total_bytes(), heap.recomputed_bytes());
        heap.check_invariants();
    }

    #[test]
    fn string_mutation_respects_immutability_and_bounds() {
        let mut heap = Heap::new(&Limits::default());
        let text = heap.alloc_string("ab", 2).unwrap();
        let _root = heap.root(text);
        assert!(heap.string_set(text, 1, 'z').unwrap());
        assert_eq!(heap.string(text).unwrap(), "az");
        assert!(!heap.string_set(text, 2, 'q').unwrap());
        heap.make_immutable(text);
        assert!(!heap.string_set(text, 0, 'q').unwrap());
        assert_eq!(heap.string(text).unwrap(), "az");
    }

    #[test]
    fn promise_state_change_keeps_accounting_exact() {
        let mut heap = Heap::new(&Limits::default());
        let promise = heap
            .alloc(Object::Promise(Promise {
                state: PromiseState::Pending {
                    thunk: Value::nil(),
                    flatten: false,
                },
            }))
            .unwrap();
        let root = heap.root(promise);
        let values = Vec::with_capacity(9);
        assert!(heap.set_promise_state(promise, PromiseState::Done(values)));
        heap.collect();
        assert_eq!(heap.total_bytes(), heap.recomputed_bytes());
        heap.check_invariants();
        drop(root);
        heap.collect();
        assert_eq!(heap.total_bytes(), 0);
        heap.check_invariants();
    }

    #[test]
    fn bytevector_payloads_compact_and_survive_collection() {
        let mut heap = Heap::new(&Limits::default());
        let dead = heap.alloc_bytevector(&[1, 2, 3, 4]).unwrap();
        let live = heap.alloc_bytevector(&[9, 8, 7]).unwrap();
        let empty = heap.alloc_bytevector(&[]).unwrap();
        let live_root = heap.root(live);
        let empty_root = heap.root(empty);
        assert_eq!(heap.byte_arena.len(), 7);
        heap.collect();
        // The dead span is left behind and the survivor is evacuated to the
        // front of the compacted arena with its handle rewritten.
        assert!(!heap.contains(dead));
        assert_eq!(heap.bytevector(live).unwrap(), vec![9, 8, 7]);
        assert_eq!(heap.bytevector(empty).unwrap(), Vec::<u8>::new());
        assert_eq!(heap.byte_arena.len(), 3);
        match heap
            .slots
            .get(live.heap_ref().unwrap().0 as usize)
            .and_then(Option::as_ref)
        {
            Some(Object::Bytevector { off, len }) => {
                assert_eq!(*off, 0);
                assert_eq!(*len, 3);
            }
            other => panic!("expected a bytevector handle, found {other:?}"),
        }
        assert_eq!(heap.total_bytes(), heap.recomputed_bytes());
        heap.check_invariants();
        drop(live_root);
        drop(empty_root);
        heap.collect();
        assert_eq!(heap.byte_arena.len(), 0);
        assert_eq!(heap.total_bytes(), 0);
        heap.check_invariants();
    }

    #[test]
    fn bytevector_contents_survive_repeated_compactions() {
        // Crosses both mark color rotations with interleaved garbage so a
        // handle must stay correct through several evacuations.
        let mut heap = Heap::new(&Limits::default());
        let keeper = heap.alloc_bytevector(&[1, 2, 3]).unwrap();
        let keeper_root = heap.root(keeper);
        for round in 0..3u8 {
            let garbage = heap.alloc_bytevector(&[round; 8]).unwrap();
            let fresh = heap.alloc_bytevector(&[round, round]).unwrap();
            let fresh_root = heap.root(fresh);
            heap.collect();
            assert!(!heap.contains(garbage));
            assert_eq!(heap.bytevector(keeper).unwrap(), vec![1, 2, 3]);
            assert_eq!(heap.bytevector(fresh).unwrap(), vec![round, round]);
            assert_eq!(heap.byte_arena.len(), 5);
            assert_eq!(heap.total_bytes(), heap.recomputed_bytes());
            drop(fresh_root);
        }
        drop(keeper_root);
        heap.collect();
        assert_eq!(heap.byte_arena.len(), 0);
        heap.check_invariants();
    }

    #[test]
    fn bytevector_mutation_respects_immutability_and_bounds() {
        let mut heap = Heap::new(&Limits::default());
        let value = heap.alloc_bytevector(&[1, 2]).unwrap();
        let _root = heap.root(value);
        assert!(heap.bytevector_set(value, 1, 9));
        assert_eq!(heap.bytevector(value).unwrap(), vec![1, 9]);
        assert!(!heap.bytevector_set(value, 2, 9));
        heap.make_immutable(value);
        assert!(!heap.bytevector_set(value, 0, 5));
        assert_eq!(heap.bytevector(value).unwrap(), vec![1, 9]);
    }

    #[test]
    fn vector_payloads_compact_and_trace_through_collection() {
        let mut heap = Heap::new(&Limits::default());
        // The element is itself arena-backed, so tracing must run against
        // pre-sweep offsets and the element's own span must survive too.
        let element = heap.alloc_string("h\u{00e9}llo", 5).unwrap();
        let dead = heap.alloc_vector(&[element, element]).unwrap();
        let live = heap.alloc_vector(&[element, Value::integer(7)]).unwrap();
        let live_root = heap.root(live);
        heap.collect();
        assert!(!heap.contains(dead));
        assert_eq!(heap.vector(live).unwrap(), vec![element, Value::integer(7)]);
        assert_eq!(heap.string(element).unwrap(), "h\u{00e9}llo");
        assert_eq!(heap.value_arena.len(), 2);
        match heap
            .slots
            .get(live.heap_ref().unwrap().0 as usize)
            .and_then(Option::as_ref)
        {
            Some(Object::Vector { off, len }) => {
                assert_eq!(*off, 0);
                assert_eq!(*len, 2);
            }
            other => panic!("expected a vector handle, found {other:?}"),
        }
        assert_eq!(heap.total_bytes(), heap.recomputed_bytes());
        heap.check_invariants();
        drop(live_root);
        heap.collect();
        assert_eq!(heap.value_arena.len(), 0);
        assert_eq!(heap.byte_arena.len(), 0);
        assert_eq!(heap.total_bytes(), 0);
        heap.check_invariants();
    }

    #[test]
    fn vector_contents_survive_repeated_compactions() {
        // Crosses both mark color rotations with interleaved garbage so a
        // handle and its traced elements must stay correct through several
        // evacuations.
        let mut heap = Heap::new(&Limits::default());
        let inner = heap.alloc_pair(Value::integer(1), Value::nil()).unwrap();
        let keeper = heap.alloc_vector(&[inner, Value::integer(2)]).unwrap();
        let keeper_root = heap.root(keeper);
        for round in 0..3i64 {
            let garbage = heap.alloc_vector(&[keeper; 4]).unwrap();
            let fresh = heap.alloc_vector(&[Value::integer(round)]).unwrap();
            let fresh_root = heap.root(fresh);
            heap.collect();
            assert!(!heap.contains(garbage));
            assert_eq!(heap.vector(keeper).unwrap(), vec![inner, Value::integer(2)]);
            assert_eq!(heap.pair(inner).unwrap(), (Value::integer(1), Value::nil()));
            assert_eq!(heap.vector(fresh).unwrap(), vec![Value::integer(round)]);
            assert_eq!(heap.value_arena.len(), 3);
            assert_eq!(heap.total_bytes(), heap.recomputed_bytes());
            drop(fresh_root);
        }
        drop(keeper_root);
        heap.collect();
        assert_eq!(heap.value_arena.len(), 0);
        heap.check_invariants();
    }

    #[test]
    fn vector_mutation_respects_immutability_and_bounds() {
        let mut heap = Heap::new(&Limits::default());
        let value = heap
            .alloc_vector(&[Value::integer(1), Value::nil()])
            .unwrap();
        let _root = heap.root(value);
        assert!(heap.vector_set(value, 1, Value::integer(9)));
        assert_eq!(heap.vector_ref(value, 1), Some(Value::integer(9)));
        assert!(!heap.vector_set(value, 2, Value::integer(9)));
        heap.make_immutable(value);
        assert!(!heap.vector_set(value, 0, Value::integer(5)));
        assert_eq!(heap.vector_ref(value, 0), Some(Value::integer(1)));
    }

    #[test]
    fn folded_trigger_arms_the_trap_at_the_crossing_allocation() {
        let limits = Limits::default().with_initial_gc_threshold(3);
        let mut heap = Heap::new(&limits);
        let _guard = heap.enter_vm();
        // The span is clamped to the soft threshold, so the first three
        // allocations succeed untrapped and the fourth's refill arms the
        // trap while that allocation itself still succeeds.
        for expected_pending in [false, false, false, true] {
            heap.alloc(Object::Pair(Value::nil(), Value::nil()))
                .unwrap();
            assert_eq!(heap.needs_collection(), expected_pending);
            assert_eq!(heap.trap_pending(), expected_pending);
        }
        // Mid-span accounting stays exact without any per-alloc counter.
        assert_eq!(heap.live_slots(), 4);
        assert_eq!(heap.total_bytes(), 4 * size_of::<Object>());
        heap.check_invariants();
    }

    #[test]
    fn hard_slot_limit_reports_heap_exhaustion() {
        let limits = Limits::default()
            .with_initial_gc_threshold(2)
            .with_max_heap_slots(2);
        let mut heap = Heap::new(&limits);
        let first = heap
            .alloc(Object::Pair(Value::nil(), Value::nil()))
            .unwrap();
        let _first_root = heap.root(first);
        let second = heap
            .alloc(Object::Pair(Value::nil(), Value::nil()))
            .unwrap();
        let _second_root = heap.root(second);
        let error = heap
            .alloc(Object::Pair(Value::nil(), Value::nil()))
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::HeapLimitExceeded);
    }
}
