//! The engine-local native-procedure registry and the fast-path dispatch
//! classification used by the VM's inline arithmetic/vector opcodes.

use super::*;

type Callback = dyn for<'a> Fn(&mut NativeContext<'a>, &[Value]) -> Result<NativeValues, Error>;

struct NativeProcedure {
    name: String,
    arity: RangeInclusive<usize>,
    callback: Box<Callback>,
    fast: Option<FastProcedure>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FastProcedure {
    Add,
    Subtract,
    Multiply,
    Divide,
    Equal,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    VectorRef,
    VectorSet,
    Cons,
    Car,
    Cdr,
    NullP,
    PairP,
    StringRef,
    CharToInteger,
    StringLength,
    AssocEqv,
    MemberEqv,
    AssocEqual,
    MemberEqual,
    Length,
    ListRef,
    ListTail,
    Reverse,
    Append,
}

impl FastProcedure {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
            Self::Equal => "=",
            Self::Less => "<",
            Self::Greater => ">",
            Self::LessEqual => "<=",
            Self::GreaterEqual => ">=",
            Self::VectorRef => "vector-ref",
            Self::VectorSet => "vector-set!",
            Self::Cons => "cons",
            Self::Car => "car",
            Self::Cdr => "cdr",
            Self::NullP => "null?",
            Self::PairP => "pair?",
            Self::StringRef => "string-ref",
            Self::CharToInteger => "char->integer",
            Self::StringLength => "string-length",
            // The scan procedures have no register-operation opcode, so these
            // names are never used for a fallback lookup. assq/assv and
            // memq/memv share a variant and report the eq spelling.
            Self::AssocEqv => "assq",
            Self::MemberEqv => "memq",
            Self::AssocEqual => "%assoc",
            Self::MemberEqual => "%member",
            Self::Length => "length",
            Self::ListRef => "list-ref",
            Self::ListTail => "list-tail",
            Self::Reverse => "reverse",
            Self::Append => "append",
        }
    }

    fn named(name: &str) -> Option<Self> {
        Some(match name {
            "+" => Self::Add,
            "-" => Self::Subtract,
            "*" => Self::Multiply,
            "/" => Self::Divide,
            "=" => Self::Equal,
            "<" => Self::Less,
            ">" => Self::Greater,
            "<=" => Self::LessEqual,
            ">=" => Self::GreaterEqual,
            "vector-ref" => Self::VectorRef,
            "vector-set!" => Self::VectorSet,
            "cons" => Self::Cons,
            "car" => Self::Car,
            "cdr" => Self::Cdr,
            "null?" => Self::NullP,
            "pair?" => Self::PairP,
            "string-ref" => Self::StringRef,
            "char->integer" => Self::CharToInteger,
            "string-length" => Self::StringLength,
            "assq" | "assv" => Self::AssocEqv,
            "memq" | "memv" => Self::MemberEqv,
            "%assoc" => Self::AssocEqual,
            "%member" => Self::MemberEqual,
            "length" => Self::Length,
            "list-ref" => Self::ListRef,
            "list-tail" => Self::ListTail,
            "reverse" => Self::Reverse,
            "append" => Self::Append,
            _ => return None,
        })
    }
}

/// Engine-local registry for host and built-in procedures.
pub(crate) struct NativeRegistry {
    procedures: Vec<NativeProcedure>,
    /// When set, every host callback is invoked inside `catch_unwind` so a panic
    /// becomes a `NativePanic` error rather than unwinding through the VM.
    catch_panics: bool,
}

impl NativeRegistry {
    pub(crate) const fn new() -> Self {
        Self {
            procedures: Vec::new(),
            catch_panics: true,
        }
    }

    /// Sets whether host callbacks are guarded by `catch_unwind`. Disabling it
    /// (for engines that trust their natives) removes the per-call guard.
    pub(crate) fn set_catch_panics(&mut self, catch: bool) {
        self.catch_panics = catch;
    }

    pub(crate) fn register<F, R>(
        &mut self,
        heap: &mut Heap,
        globals: &mut crate::global::GlobalStore,
        name: String,
        arity: RangeInclusive<usize>,
        callback: F,
    ) -> Result<(), Error>
    where
        F: for<'a> Fn(&mut NativeContext<'a>, &[Value]) -> Result<R, Error> + 'static,
        R: IntoNativeValues + 'static,
    {
        self.register_at(heap, globals, name.clone(), name, arity, callback)
    }

    pub(crate) fn register_at<F, R>(
        &mut self,
        heap: &mut Heap,
        globals: &mut crate::global::GlobalStore,
        global_name: String,
        procedure_name: String,
        arity: RangeInclusive<usize>,
        callback: F,
    ) -> Result<(), Error>
    where
        F: for<'a> Fn(&mut NativeContext<'a>, &[Value]) -> Result<R, Error> + 'static,
        R: IntoNativeValues + 'static,
    {
        if arity.is_empty() || globals.contains_key(&global_name) {
            return Err(Error::plain(
                ErrorKind::RuntimeError,
                format!("procedure '{procedure_name}' is already bound or has an invalid arity"),
            ));
        }
        let id = u32::try_from(self.procedures.len()).map_err(|_| {
            Error::plain(
                ErrorKind::ImplementationRestriction,
                "native procedure registry exhausted",
            )
        })?;
        // The classification is stored both here and in the heap object, so
        // the VM's call fast paths read it with the callee probe itself.
        let fast = FastProcedure::named(&global_name);
        let value = heap.alloc(Object::Native { id, fast })?;
        self.procedures.push(NativeProcedure {
            name: procedure_name,
            arity,
            callback: Box::new(move |cx, args| {
                callback(cx, args).map(IntoNativeValues::into_native_values)
            }),
            fast,
        });
        globals.insert(global_name, value);
        Ok(())
    }

    /// Adds a callable placeholder for an Appendix A binding whose concrete
    /// implementation is not provided by this engine profile.
    pub(crate) fn register_unsupported(
        &mut self,
        heap: &mut Heap,
        globals: &mut crate::global::GlobalStore,
        name: &str,
    ) -> Result<(), Error> {
        self.register_unavailable(
            heap,
            globals,
            name,
            ErrorKind::ImplementationRestriction,
            "not implemented in this engine profile",
        )
    }

    /// Adds a placeholder for an operation denied by the embedding host.
    pub(crate) fn register_capability_denied(
        &mut self,
        heap: &mut Heap,
        globals: &mut crate::global::GlobalStore,
        name: &str,
    ) -> Result<(), Error> {
        self.register_unavailable(
            heap,
            globals,
            name,
            ErrorKind::CapabilityDenied,
            "denied by the embedding host",
        )
    }

    fn register_unavailable(
        &mut self,
        heap: &mut Heap,
        globals: &mut crate::global::GlobalStore,
        name: &str,
        kind: ErrorKind,
        reason: &str,
    ) -> Result<(), Error> {
        if globals.contains_key(name) {
            return Ok(());
        }
        let message = format!("procedure '{name}' is {reason}");
        self.register(
            heap,
            globals,
            name.to_owned(),
            0..=usize::MAX,
            move |_: &mut NativeContext<'_>, _: &[Value]| -> Result<Value, Error> {
                Err(Error::plain(kind, message.clone()))
            },
        )
    }

    /// Invokes a registered procedure: the classified inline fast path first
    /// (falling through on any shape it does not handle so the general path
    /// raises the canonical error), then the host callback inside a rooted
    /// region.
    ///
    /// `vm_roots` enumerates the live VM register file (including any pending
    /// `apply`-spread arguments) so a collection during the call can trace it
    /// directly. It also keeps `args`, which are register slots, reachable.
    /// Passing `None` is sound only on paths that run under VM-managed rooting
    /// (`vm.rs`'s register-operation fallback), where `alloc` defers collection
    /// to the next safe point instead of collecting mid-call.
    pub(crate) fn invoke(
        &self,
        id: u32,
        heap: &mut Heap,
        symbols: &mut HashMap<String, Value>,
        globals: &crate::global::GlobalStore,
        args: &[Value],
        vm_roots: Option<&crate::heap::RootGatherer<'_>>,
    ) -> Result<NativeValues, Error> {
        let procedure = self
            .procedures
            .get(id as usize)
            .ok_or_else(|| Error::plain(ErrorKind::RuntimeError, "unknown native procedure"))?;
        if let Some(fast) = procedure.fast {
            let mut out = Value::unspecified();
            if fast.invoke(heap, args, &mut out) {
                return Ok(NativeValues::one(out));
            }
        }
        if !procedure.arity.contains(&args.len()) {
            return Err(Error::plain(
                ErrorKind::ArityError,
                format!(
                    "{} expected {}..={} arguments, received {}",
                    procedure.name,
                    procedure.arity.start(),
                    procedure.arity.end(),
                    args.len()
                ),
            ));
        }
        let mark = heap.temporary_root_mark();
        // Inside the region `push_root` engages, keeping the callback's
        // allocations rooted across any collection it triggers.
        heap.enter_rooted_region();
        let mut context = NativeContext {
            heap,
            symbols,
            globals,
            vm_roots,
        };
        let result = if self.catch_panics {
            match catch_unwind(AssertUnwindSafe(|| {
                (procedure.callback)(&mut context, args)
            })) {
                Ok(result) => result,
                Err(_) => Err(Error::plain(
                    ErrorKind::NativePanic,
                    format!("native procedure '{}' panicked", procedure.name),
                )),
            }
        } else {
            (procedure.callback)(&mut context, args)
        };
        heap.exit_rooted_region();
        // Roots pushed by the callback (via `NativeContext::alloc`) end with the
        // call. The returned values are written into registers before the next
        // safe point.
        heap.truncate_temporary_roots(mark);
        // A newly interned symbol flags `engine_roots` for refresh inside
        // `NativeContext::intern_symbol`, the single place the table grows.
        result
    }
}

impl FastProcedure {
    /// Runs the classified fast path, writing the single result into `out`
    /// and returning true. Any shape the fast path does not handle returns
    /// false so the caller defers to the canonical native and its canonical
    /// error. Outlined with a register-friendly ABI on purpose: the VM
    /// dispatch loop runs this once per native call, and returning a by-value
    /// result packet here cost a measured store-to-load-forwarding stall on
    /// the 32-byte packet store every call.
    #[inline(never)]
    pub(crate) fn invoke(self, heap: &mut Heap, args: &[Value], out: &mut Value) -> bool {
        let value = match self {
            FastProcedure::Add => fast_arithmetic(args, i64::checked_add, |a, b| a + b),
            FastProcedure::Subtract => fast_arithmetic(args, i64::checked_sub, |a, b| a - b),
            FastProcedure::Multiply => fast_arithmetic(args, i64::checked_mul, |a, b| a * b),
            FastProcedure::Divide => fast_divide(args),
            FastProcedure::Equal => fast_compare(args, |ordering| ordering.is_eq()),
            FastProcedure::Less => fast_compare(args, |ordering| ordering.is_lt()),
            FastProcedure::Greater => fast_compare(args, |ordering| ordering.is_gt()),
            FastProcedure::LessEqual => fast_compare(args, |ordering| ordering.is_le()),
            FastProcedure::GreaterEqual => fast_compare(args, |ordering| ordering.is_ge()),
            FastProcedure::VectorRef => {
                let [vector, index] = args else {
                    return false;
                };
                let Some(index) = index.as_fixnum().and_then(|i| usize::try_from(i).ok()) else {
                    return false;
                };
                heap.vector_ref(*vector, index)
            }
            FastProcedure::VectorSet => {
                let [vector, index, value] = args else {
                    return false;
                };
                let Some(index) = index.as_fixnum().and_then(|i| usize::try_from(i).ok()) else {
                    return false;
                };
                heap.vector_set(*vector, index, *value)
                    .then(Value::unspecified)
            }
            FastProcedure::Cons => {
                let [car, cdr] = args else {
                    return false;
                };
                // `alloc` never collects inline while the VM manages roots (it only
                // arms a deferred collection), and the returned pair is written
                // straight into a register before the next safe point, so the
                // unrooted args are safe. A heap-limit `Err` defers to the slow
                // path, which raises the identical `HeapLimitExceeded`.
                heap.alloc_pair(*car, *cdr).ok()
            }
            FastProcedure::Car => {
                let [pair] = args else {
                    return false;
                };
                // A non-pair yields a miss, deferring to the slow path so the
                // same type error is raised.
                heap.pair(*pair).map(|(car, _)| car)
            }
            FastProcedure::Cdr => {
                let [pair] = args else {
                    return false;
                };
                heap.pair(*pair).map(|(_, cdr)| cdr)
            }
            FastProcedure::NullP => {
                let [value] = args else {
                    return false;
                };
                Some(Value::boolean(*value == Value::nil()))
            }
            FastProcedure::PairP => {
                let [value] = args else {
                    return false;
                };
                Some(Value::boolean(heap.pair(*value).is_some()))
            }
            FastProcedure::StringRef => {
                let [string, index] = args else {
                    return false;
                };
                // A non-fixnum/negative index or an out-of-range/non-string first
                // argument yields a miss, so the slow path raises the same
                // range-or-type error.
                let Some(index) = index.as_fixnum().and_then(|i| usize::try_from(i).ok()) else {
                    return false;
                };
                heap.string_ref(*string, index).map(Value::character)
            }
            FastProcedure::CharToInteger => {
                let [value] = args else {
                    return false;
                };
                match value.decode() {
                    crate::value::ValueRepr::Character(character) => {
                        Some(Value::integer(i64::from(character as u32)))
                    }
                    _ => None,
                }
            }
            FastProcedure::StringLength => {
                let [string] = args else {
                    return false;
                };
                heap.string_len(*string)
                    .and_then(|length| i64::try_from(length).ok())
                    .map(Value::integer)
            }
            // The bounded scans allocate nothing on the GC heap and return
            // existing heap values, which land in registers before the next
            // safe point. Any shape a fast scan does not handle is a miss,
            // so the canonical native raises the identical error.
            FastProcedure::AssocEqv => {
                let [object, alist] = args else {
                    return false;
                };
                return fast_assoc_scan(heap, *object, *alist, eqv_value, out);
            }
            FastProcedure::MemberEqv => {
                let [object, list] = args else {
                    return false;
                };
                return fast_member_scan(heap, *object, *list, eqv_value, out);
            }
            FastProcedure::AssocEqual => {
                let [object, alist] = args else {
                    return false;
                };
                return fast_assoc_scan(heap, *object, *alist, equal_value, out);
            }
            FastProcedure::MemberEqual => {
                let [object, list] = args else {
                    return false;
                };
                return fast_member_scan(heap, *object, *list, equal_value, out);
            }
            FastProcedure::Length => {
                let [list] = args else {
                    return false;
                };
                return fast_length(heap, *list, out);
            }
            FastProcedure::ListRef => {
                let [list, count] = args else {
                    return false;
                };
                return fast_list_ref(heap, *list, *count, out);
            }
            FastProcedure::ListTail => {
                let [list, count] = args else {
                    return false;
                };
                return fast_list_tail(heap, *list, *count, out);
            }
            FastProcedure::Reverse => {
                let [list] = args else {
                    return false;
                };
                return fast_reverse(heap, *list, out);
            }
            // Only the two-argument shape is handled. Zero, one, and n-ary
            // appends defer to the canonical native.
            FastProcedure::Append => {
                let [first, second] = args else {
                    return false;
                };
                return fast_append_two(heap, *first, *second, out);
            }
        };
        match value {
            Some(value) => {
                *out = value;
                true
            }
            None => false,
        }
    }
}

pub(super) fn fast_arithmetic(
    args: &[Value],
    exact: fn(i64, i64) -> Option<i64>,
    inexact: fn(f64, f64) -> f64,
) -> Option<Value> {
    let [left, right] = args else {
        return None;
    };
    let value = match (left.decode(), right.decode()) {
        (crate::value::ValueRepr::Fixnum(left), crate::value::ValueRepr::Fixnum(right)) => {
            // `None` only on real i64 overflow, which the slow path folds into a
            // heap-backed exact integer. Every in-range i64 result stays inline.
            Value::integer(exact(left, right)?)
        }
        (crate::value::ValueRepr::Float(left), crate::value::ValueRepr::Float(right)) => {
            Value::float(inexact(left, right))
        }
        (crate::value::ValueRepr::Fixnum(left), crate::value::ValueRepr::Float(right)) => {
            Value::float(inexact(left as f64, right))
        }
        (crate::value::ValueRepr::Float(left), crate::value::ValueRepr::Fixnum(right)) => {
            Value::float(inexact(left, right as f64))
        }
        _ => return None,
    };
    Some(value)
}

pub(super) fn fast_compare(
    args: &[Value],
    predicate: fn(std::cmp::Ordering) -> bool,
) -> Option<Value> {
    let [left, right] = args else {
        return None;
    };
    let ordering = match (left.decode(), right.decode()) {
        (crate::value::ValueRepr::Fixnum(left), crate::value::ValueRepr::Fixnum(right)) => {
            left.cmp(&right)
        }
        (crate::value::ValueRepr::Float(left), crate::value::ValueRepr::Float(right)) => {
            left.partial_cmp(&right)?
        }
        (crate::value::ValueRepr::Fixnum(left), crate::value::ValueRepr::Float(right)) => {
            if left.unsigned_abs() > (1_u64 << 53) {
                return None;
            }
            (left as f64).partial_cmp(&right)?
        }
        (crate::value::ValueRepr::Float(left), crate::value::ValueRepr::Fixnum(right)) => {
            if right.unsigned_abs() > (1_u64 << 53) {
                return None;
            }
            left.partial_cmp(&(right as f64))?
        }
        _ => return None,
    };
    Some(Value::boolean(predicate(ordering)))
}

pub(super) fn fast_divide(args: &[Value]) -> Option<Value> {
    let [left, right] = args else {
        return None;
    };
    let value = match (left.decode(), right.decode()) {
        (crate::value::ValueRepr::Fixnum(left), crate::value::ValueRepr::Fixnum(right))
            if right != 0 && left % right == 0 =>
        {
            Value::integer(left.checked_div(right)?)
        }
        (crate::value::ValueRepr::Float(left), crate::value::ValueRepr::Float(right)) => {
            Value::float(left / right)
        }
        (crate::value::ValueRepr::Fixnum(left), crate::value::ValueRepr::Float(right)) => {
            Value::float(left as f64 / right)
        }
        (crate::value::ValueRepr::Float(left), crate::value::ValueRepr::Fixnum(right)) => {
            Value::float(left / right as f64)
        }
        _ => return None,
    };
    Some(value)
}
