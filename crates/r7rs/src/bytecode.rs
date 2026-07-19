//! Fixed-width register bytecode and verification.

use std::{cell::Cell, collections::HashSet, rc::Rc};

use crate::{Error, ErrorKind, Number, Value};

pub(crate) const MAX_REGISTERS: usize = 255;
const SIZE_OP: u32 = 7;
const SIZE_A: u32 = 8;
const SIZE_B: u32 = 8;
const POS_A: u32 = SIZE_OP;
const POS_K: u32 = POS_A + SIZE_A;
const POS_B: u32 = POS_K + 1;
const POS_C: u32 = POS_B + SIZE_B;
const POS_BX: u32 = POS_K;
const POS_AX: u32 = POS_A;
const MAX_BX: u32 = (1 << 17) - 1;
const MAX_AX: u32 = (1 << 25) - 1;
const OFFSET_SBX: i32 = (MAX_BX >> 1) as i32;
const OFFSET_SJ: i32 = (MAX_AX >> 1) as i32;

/// Register-machine opcodes, grouped by category.
///
/// Discriminant values are the implicit `#[repr(u8)]` positions (0 upward in
/// declaration order), so they follow the category order below.
/// `from_bits` and the VM dispatch in `vm.rs` follow the same order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum Opcode {
    // Data movement and constants.
    Move,
    LoadK,
    LoadKx,
    ExtraArg,

    // Global access.
    GetGlobal,
    SetGlobal,

    // Capture access.
    GetCapture,
    SetCapture,
    GetCaptureValue,

    // Local box (mutable cell) access.
    GetLocalBox,
    SetLocalBox,
    BoxLocal,

    // Closures.
    Closure,
    CaseLambda,

    // Control flow and calls.
    Jump,
    JumpFalse,
    Call,
    TailCall,
    Return,
    Cold,

    // Arithmetic.
    Add,
    Subtract,
    Multiply,
    Divide,

    // Numeric comparison.
    NumericEqual,
    NumericLess,
    NumericLessEqual,
    NumericGreater,
    NumericGreaterEqual,

    // Pair and list primitives.
    Cons,
    Car,
    Cdr,
    NullP,
    PairP,

    // Vector primitives.
    VectorRef,
    VectorSet,

    // String and char primitives.
    StringRef,
    StringLength,
    CharToInteger,

    // Compare-and-branch (each consumes a following Jump word).
    TestEqual,
    TestLess,
    TestLessEqual,
    TestGreater,
    TestGreaterEqual,
    TestNull,
    TestPair,
    TestVectorRef,

    // Fused loop back-edges.
    LoopBack,
    LoopBackWhileNotEqual,
    LoopBackWhileNotLess,
    LoopBackWhileNotLessEqual,
    LoopBackStepWhileLess,
    LoopBackStepWhileLessEqual,

    // Fused accumulates.
    AddVectorRef,
    AddCar,
    AddStringRefCode,
    AddMul,
    SubMul,
    AddMulVectorRef,
    VectorRefVectorRef,

    // Fixnum-constant specializations.
    AddFixnumK,
    SubtractFixnumK,
    AddSubFixnumK,
    TestLessFixnum,
    TestLessEqualFixnum,
    TestEqualFixnum,
    LoopBackWhileNotEqualFixnum,
}

impl Opcode {
    pub(crate) fn from_bits(bits: u8) -> Option<Self> {
        Some(match bits {
            // Data movement and constants.
            0 => Self::Move,
            1 => Self::LoadK,
            2 => Self::LoadKx,
            3 => Self::ExtraArg,
            // Global access.
            4 => Self::GetGlobal,
            5 => Self::SetGlobal,
            // Capture access.
            6 => Self::GetCapture,
            7 => Self::SetCapture,
            8 => Self::GetCaptureValue,
            // Local box (mutable cell) access.
            9 => Self::GetLocalBox,
            10 => Self::SetLocalBox,
            11 => Self::BoxLocal,
            // Closures.
            12 => Self::Closure,
            13 => Self::CaseLambda,
            // Control flow and calls.
            14 => Self::Jump,
            15 => Self::JumpFalse,
            16 => Self::Call,
            17 => Self::TailCall,
            18 => Self::Return,
            19 => Self::Cold,
            // Arithmetic.
            20 => Self::Add,
            21 => Self::Subtract,
            22 => Self::Multiply,
            23 => Self::Divide,
            // Numeric comparison.
            24 => Self::NumericEqual,
            25 => Self::NumericLess,
            26 => Self::NumericLessEqual,
            27 => Self::NumericGreater,
            28 => Self::NumericGreaterEqual,
            // Pair and list primitives.
            29 => Self::Cons,
            30 => Self::Car,
            31 => Self::Cdr,
            32 => Self::NullP,
            33 => Self::PairP,
            // Vector primitives.
            34 => Self::VectorRef,
            35 => Self::VectorSet,
            // String and char primitives.
            36 => Self::StringRef,
            37 => Self::StringLength,
            38 => Self::CharToInteger,
            // Compare-and-branch.
            39 => Self::TestEqual,
            40 => Self::TestLess,
            41 => Self::TestLessEqual,
            42 => Self::TestGreater,
            43 => Self::TestGreaterEqual,
            44 => Self::TestNull,
            45 => Self::TestPair,
            46 => Self::TestVectorRef,
            // Fused loop back-edges.
            47 => Self::LoopBack,
            48 => Self::LoopBackWhileNotEqual,
            49 => Self::LoopBackWhileNotLess,
            50 => Self::LoopBackWhileNotLessEqual,
            51 => Self::LoopBackStepWhileLess,
            52 => Self::LoopBackStepWhileLessEqual,
            // Fused accumulates.
            53 => Self::AddVectorRef,
            54 => Self::AddCar,
            55 => Self::AddStringRefCode,
            56 => Self::AddMul,
            57 => Self::SubMul,
            58 => Self::AddMulVectorRef,
            59 => Self::VectorRefVectorRef,
            // Fixnum-constant specializations.
            60 => Self::AddFixnumK,
            61 => Self::SubtractFixnumK,
            62 => Self::AddSubFixnumK,
            63 => Self::TestLessFixnum,
            64 => Self::TestLessEqualFixnum,
            65 => Self::TestEqualFixnum,
            66 => Self::LoopBackWhileNotEqualFixnum,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Word(pub(crate) u32);

impl Word {
    pub(crate) fn abc(opcode: Opcode, a: u8, b: u8, c: u8, k: bool) -> Self {
        Self(
            opcode as u32
                | (u32::from(a) << POS_A)
                | (u32::from(k) << POS_K)
                | (u32::from(b) << POS_B)
                | (u32::from(c) << POS_C),
        )
    }

    pub(crate) fn abx(opcode: Opcode, a: u8, bx: u32) -> Result<Self, Error> {
        if bx > MAX_BX {
            return Err(compile_error("bytecode operand exceeds the 17-bit field"));
        }
        Ok(Self(
            opcode as u32 | (u32::from(a) << POS_A) | (bx << POS_BX),
        ))
    }

    pub(crate) fn ax(opcode: Opcode, ax: u32) -> Result<Self, Error> {
        if ax > MAX_AX {
            return Err(compile_error("bytecode operand exceeds the 25-bit field"));
        }
        Ok(Self(opcode as u32 | (ax << POS_AX)))
    }

    pub(crate) fn sj(opcode: Opcode, offset: isize) -> Result<Self, Error> {
        let offset = i32::try_from(offset).map_err(|_| compile_error("jump is too large"))?;
        let encoded = offset
            .checked_add(OFFSET_SJ)
            .filter(|value| (0..=MAX_AX as i32).contains(value))
            .ok_or_else(|| compile_error("jump is too large"))?;
        Self::ax(opcode, encoded as u32)
    }

    pub(crate) fn asbx(opcode: Opcode, a: u8, offset: isize) -> Result<Self, Error> {
        let offset = i32::try_from(offset).map_err(|_| compile_error("branch is too large"))?;
        let encoded = offset
            .checked_add(OFFSET_SBX)
            .filter(|value| (0..=MAX_BX as i32).contains(value))
            .ok_or_else(|| compile_error("branch is too large"))?;
        Self::abx(opcode, a, encoded as u32)
    }

    pub(crate) fn opcode(self) -> Result<Opcode, Error> {
        Opcode::from_bits((self.0 & 0x7F) as u8).ok_or_else(|| invalid("unknown register opcode"))
    }

    /// Decodes the opcode for execution of an already-verified chunk.
    ///
    /// `verify` (run on every chunk before `execute`) decodes every word of
    /// every reachable chunk via [`Self::opcode`], so the low 7 bits of any
    /// installed word always name a known opcode and no per-instruction
    /// validation is needed. Telling the optimizer so removes the decode clamp
    /// that otherwise sits in front of every instruction the VM dispatches.
    #[inline(always)]
    pub(crate) fn opcode_verified(self) -> Opcode {
        match Opcode::from_bits((self.0 & 0x7F) as u8) {
            Some(opcode) => opcode,
            None => {
                debug_assert!(false, "dispatch on a word that never passed verification");
                // SAFETY: every chunk reaching the executor passed `verify`,
                // which calls `Word::opcode` on each of its words. A word
                // with an unknown opcode cannot be installed.
                #[allow(unsafe_code)]
                unsafe {
                    std::hint::unreachable_unchecked()
                }
            }
        }
    }

    pub(crate) const fn a(self) -> u8 {
        ((self.0 >> POS_A) & 0xFF) as u8
    }

    pub(crate) const fn b(self) -> u8 {
        ((self.0 >> POS_B) & 0xFF) as u8
    }

    pub(crate) const fn c(self) -> u8 {
        ((self.0 >> POS_C) & 0xFF) as u8
    }

    pub(crate) const fn k(self) -> bool {
        ((self.0 >> POS_K) & 1) != 0
    }

    pub(crate) const fn bx(self) -> u32 {
        (self.0 >> POS_BX) & MAX_BX
    }

    pub(crate) const fn ax_value(self) -> u32 {
        (self.0 >> POS_AX) & MAX_AX
    }

    pub(crate) const fn sbx(self) -> isize {
        self.bx() as isize - OFFSET_SBX as isize
    }

    pub(crate) const fn signed_jump(self) -> isize {
        self.ax_value() as isize - OFFSET_SJ as isize
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum CaptureSource {
    Local(u8),
    Capture(u8),
}

/// How a closure holds one captured variable. The kind is a property of the
/// originating binding, computed once at its origin frame and propagated
/// unchanged along every capture chain, so all closures over one binding agree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CaptureKind {
    /// The binding is never `set!`-mutated: the capture slot holds the raw
    /// value, snapshotted when the closure is created. Read by
    /// `GetCaptureValue`.
    Value,
    /// The binding is mutated somewhere: the capture slot holds a shared heap
    /// `Box` cell. Read/written by `GetCapture`/`SetCapture`.
    Cell,
}

#[derive(Clone, Debug)]
pub(crate) struct ClosurePrototype {
    pub(crate) chunk: Rc<Chunk>,
    pub(crate) captures: Vec<CaptureSource>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExpectedResults {
    Discard,
    One,
    All,
}

impl ExpectedResults {
    pub(crate) const fn call_field(self) -> u8 {
        match self {
            Self::All => 0,
            Self::Discard => 1,
            Self::One => 2,
        }
    }

    pub(crate) fn from_call_field(value: u8) -> Result<Self, Error> {
        match value {
            0 => Ok(Self::All),
            1 => Ok(Self::Discard),
            2 => Ok(Self::One),
            _ => Err(invalid("unsupported result-count encoding")),
        }
    }

    /// Decodes the call-results field of an already-verified `Call` word. The
    /// verifier runs [`Self::from_call_field`] on every `Call` word, so a word
    /// fetched from a verified chunk always decodes. The dispatch loop uses
    /// this to skip the fallible decoder's cold error branch and call overhead.
    #[inline(always)]
    pub(crate) fn from_call_field_verified(value: u8) -> Self {
        debug_assert!(value <= 2, "unverified call-results field {value}");
        match value {
            0 => Self::All,
            1 => Self::Discard,
            _ => Self::One,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum ColdInstruction {
    ValueCountError {
        expected: usize,
        actual: usize,
    },
    CallWithValues {
        destination: u8,
        producer: u8,
        consumer: u8,
        expected: ExpectedResults,
    },
    MakePromise {
        destination: u8,
        thunk: u8,
        flatten: bool,
    },
    Force {
        destination: u8,
        promise: u8,
        expected: ExpectedResults,
    },
    PushHandler {
        handler: u8,
    },
    PopHandler,
    Raise {
        destination: u8,
        object: u8,
        continuable: bool,
        expected: ExpectedResults,
    },
    CaptureContinuation {
        destination: u8,
        procedure: u8,
        expected: ExpectedResults,
    },
    DynamicWind {
        destination: u8,
        before: u8,
        thunk: u8,
        after: u8,
        expected: ExpectedResults,
    },
    CallWithPort {
        destination: u8,
        port: u8,
        procedure: u8,
        expected: ExpectedResults,
    },
    CallWithFile {
        destination: u8,
        path: u8,
        procedure: u8,
        input: bool,
        expected: ExpectedResults,
    },
    WithFile {
        destination: u8,
        path: u8,
        thunk: u8,
        input: bool,
        expected: ExpectedResults,
    },
    Load {
        destination: u8,
        path: u8,
        environment: Option<u8>,
        expected: ExpectedResults,
    },
    PushParameters {
        first: u8,
        count: u8,
    },
    PopParameters {
        count: u8,
    },
    MakeParameter {
        destination: u8,
        initial: u8,
        converter: Option<u8>,
    },
    MakeError {
        destination: u8,
        message: u8,
        first_irritant: u8,
        count: u8,
    },
    /// Materializes a numeric literal that has no inline `Value` representation
    /// (a heap-backed exact integer or rational). Kept off the hot constant
    /// table so `LoadK` and RK operands never allocate: the hot table holds
    /// only immediate values.
    LoadNumber {
        destination: u8,
        number: Number,
    },
}

#[derive(Debug)]
pub(crate) struct GlobalOperand {
    pub(crate) name: Rc<str>,
    linked: Cell<Option<crate::global::GlobalId>>,
}

impl GlobalOperand {
    pub(crate) fn new(name: Rc<str>) -> Self {
        Self {
            name,
            linked: Cell::new(None),
        }
    }

    // Always inlined: after the first execution the inline cache hits and this
    // is a single `Cell` load on the `GetGlobal`/`SetGlobal` fast path. The
    // name lookup that fills the cache is outlined below.
    #[inline(always)]
    pub(crate) fn resolve(
        &self,
        globals: &mut crate::global::GlobalStore,
    ) -> Result<crate::global::GlobalId, Error> {
        if let Some(id) = self.linked.get() {
            return Ok(id);
        }
        self.resolve_slow(globals)
    }

    #[cold]
    #[inline(never)]
    fn resolve_slow(
        &self,
        globals: &mut crate::global::GlobalStore,
    ) -> Result<crate::global::GlobalId, Error> {
        let id = globals.ensure(&self.name)?;
        self.linked.set(Some(id));
        Ok(id)
    }
}

/// Immutable verified code reusable within its owning engine.
#[derive(Clone, Debug)]
pub struct CompiledModule {
    pub(crate) entry: Rc<Chunk>,
    pub(crate) owner: Option<Rc<()>>,
}

#[derive(Clone, Debug)]
pub(crate) struct Chunk {
    pub(crate) code: Vec<Word>,
    /// Immediate (non-heap) values only, so the executor's `LoadK`/RK reads are
    /// single indexed loads with no allocation. Enforced by the verifier;
    /// literals needing heap storage compile to [`ColdInstruction::LoadNumber`].
    pub(crate) constants: Vec<Value>,
    pub(crate) global_operands: Vec<Rc<GlobalOperand>>,
    pub(crate) closures: Vec<ClosurePrototype>,
    pub(crate) cold: Vec<ColdInstruction>,
    pub(crate) arity: Arity,
    pub(crate) max_registers: u8,
    /// Per-capture-slot kinds (also the capture count). See [`CaptureKind`].
    pub(crate) capture_kinds: Box<[CaptureKind]>,
    /// Parameter register indices that hold a heap `Box` cell for the frame's
    /// duration. A local is boxed exactly when it is `set!`-mutated anywhere
    /// (captured-but-immutable locals stay raw values that closures snapshot);
    /// the frame boxes these slots on entry and all access goes through
    /// `GetLocalBox`/`SetLocalBox`.
    pub(crate) boxed_locals: Box<[u8]>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Arity {
    Exact(u8),
    AtLeast(u8),
}

impl Arity {
    pub(crate) fn accepts(self, count: usize) -> bool {
        match self {
            Self::Exact(expected) => count == expected as usize,
            Self::AtLeast(required) => count >= required as usize,
        }
    }
}

impl CompiledModule {
    pub(crate) fn new(entry: Rc<Chunk>) -> Result<Self, Error> {
        verify(&entry)?;
        Ok(Self { entry, owner: None })
    }

    pub(crate) fn with_owner(mut self, owner: &Rc<()>) -> Self {
        self.owner = Some(owner.clone());
        self
    }

    pub(crate) fn belongs_to(&self, owner: &Rc<()>) -> bool {
        self.owner
            .as_ref()
            .is_none_or(|candidate| Rc::ptr_eq(candidate, owner))
    }
}

fn verify(entry: &Rc<Chunk>) -> Result<(), Error> {
    let mut visited = HashSet::new();
    verify_chunk(entry, &mut visited)
}

fn verify_chunk(chunk: &Rc<Chunk>, visited: &mut HashSet<*const Chunk>) -> Result<(), Error> {
    if !visited.insert(Rc::as_ptr(chunk)) {
        return Ok(());
    }
    if chunk.code.is_empty() || chunk.max_registers == 0 {
        return Err(invalid("empty register chunk"));
    }
    // The executor reads constants without touching the heap, so the table may
    // hold only immediate values (heap-backed literals go through
    // `ColdInstruction::LoadNumber`). A heap reference here could also dangle:
    // chunks outlive any particular collection cycle and constants are not
    // traced as roots.
    if chunk
        .constants
        .iter()
        .any(|value| value.heap_ref().is_some())
    {
        return Err(invalid("constant holds a heap reference"));
    }
    let registers = usize::from(chunk.max_registers);
    let register = |value: u8| {
        if usize::from(value) < registers {
            Ok(())
        } else {
            Err(invalid("register operand is out of bounds"))
        }
    };
    // A capture operand must exist and hold the representation its opcode
    // expects: `GetCapture`/`SetCapture` dereference a heap cell, while
    // `GetCaptureValue` copies a raw value.
    let capture = |index: u8, kind: CaptureKind| match chunk.capture_kinds.get(usize::from(index)) {
        Some(&actual) if actual == kind => Ok(()),
        Some(_) => Err(invalid("capture opcode does not match the slot's kind")),
        None => Err(invalid("capture operand is out of bounds")),
    };
    for (pc, word) in chunk.code.iter().copied().enumerate() {
        let opcode = word.opcode()?;
        // No instruction may fall off the end of the chunk: together with the
        // `jump_target` checks this proves every pc the executor ever fetches
        // is in bounds, which the dispatch loop's unchecked instruction fetch
        // relies on. Terminal opcodes never fall through (`LoopBack`
        // unconditionally takes its consumed `Jump`). `LoadKx` and the `Test*`
        // family consume the following word, so their fall-through target is
        // `pc + 2`. `ExtraArg` is exempt: it is never an execution point (it
        // must follow an `ExtraArg`-consuming word, which skips it, and jumps
        // may not target it).
        let fall_through = match opcode {
            Opcode::Return | Opcode::TailCall | Opcode::Jump | Opcode::LoopBack => None,
            Opcode::LoadKx
            | Opcode::AddMulVectorRef
            | Opcode::VectorRefVectorRef
            | Opcode::TestLess
            | Opcode::TestLessEqual
            | Opcode::TestEqual
            | Opcode::TestGreater
            | Opcode::TestGreaterEqual
            | Opcode::TestNull
            | Opcode::TestPair
            | Opcode::TestVectorRef
            | Opcode::TestLessFixnum
            | Opcode::TestLessEqualFixnum
            | Opcode::TestEqualFixnum
            | Opcode::LoopBackWhileNotEqual
            | Opcode::LoopBackWhileNotEqualFixnum
            | Opcode::LoopBackWhileNotLess
            | Opcode::LoopBackWhileNotLessEqual
            | Opcode::LoopBackStepWhileLessEqual
            | Opcode::LoopBackStepWhileLess => Some(pc + 2),
            Opcode::ExtraArg => None,
            _ => Some(pc + 1),
        };
        if fall_through.is_some_and(|target| target >= chunk.code.len()) {
            return Err(invalid("instruction falls off the end of the chunk"));
        }
        match opcode {
            Opcode::Move => {
                register(word.a())?;
                register(word.b())?;
            }
            Opcode::GetCapture | Opcode::SetCapture => {
                register(word.a())?;
                capture(word.b(), CaptureKind::Cell)?;
            }
            Opcode::GetCaptureValue => {
                register(word.a())?;
                capture(word.b(), CaptureKind::Value)?;
            }
            Opcode::GetLocalBox | Opcode::SetLocalBox => {
                register(word.a())?;
                register(word.b())?;
            }
            Opcode::BoxLocal => {
                register(word.a())?;
            }
            Opcode::LoadK => {
                register(word.a())?;
                constant(chunk, word.bx())?;
            }
            Opcode::LoadKx => {
                register(word.a())?;
                let extra = chunk
                    .code
                    .get(pc + 1)
                    .copied()
                    .ok_or_else(|| invalid("LOADKX lacks EXTRAARG"))?;
                if extra.opcode()? != Opcode::ExtraArg {
                    return Err(invalid("LOADKX lacks EXTRAARG"));
                }
                constant(chunk, extra.ax_value())?;
            }
            Opcode::ExtraArg => {
                let consumer = matches!(
                    pc.checked_sub(1).map(|prior| chunk.code[prior].opcode()),
                    Some(Ok(Opcode::LoadKx
                        | Opcode::AddMulVectorRef
                        | Opcode::VectorRefVectorRef))
                );
                if !consumer {
                    return Err(invalid("orphan EXTRAARG"));
                }
            }
            Opcode::GetGlobal | Opcode::SetGlobal => {
                register(word.a())?;
                global(chunk, word.bx())?;
            }
            Opcode::Closure => {
                register(word.a())?;
                closure(chunk, word.bx())?;
            }
            Opcode::CaseLambda => {
                register(word.a())?;
                register_count(registers, word.b(), word.c())?;
            }
            Opcode::Jump => {
                jump_target(chunk, pc, word.signed_jump())?;
            }
            Opcode::JumpFalse => {
                register(word.a())?;
                jump_target(chunk, pc, word.sbx())?;
            }
            Opcode::Call => {
                register_range(registers, word.a(), word.b())?;
                ExpectedResults::from_call_field(word.c())?;
            }
            Opcode::TailCall => {
                register_range(registers, word.a(), word.b())?;
            }
            Opcode::Return => {
                register_range(registers, word.a(), word.b())?;
            }
            Opcode::Add
            | Opcode::Subtract
            | Opcode::Multiply
            | Opcode::Divide
            | Opcode::NumericEqual
            | Opcode::NumericLess
            | Opcode::NumericGreater
            | Opcode::NumericLessEqual
            | Opcode::NumericGreaterEqual
            | Opcode::VectorRef
            | Opcode::AddMul
            | Opcode::SubMul => {
                register(word.a())?;
                register(word.b())?;
                if word.k() {
                    constant(chunk, u32::from(word.c()))?;
                } else {
                    register(word.c())?;
                }
            }
            Opcode::VectorSet => {
                register(word.a())?;
                register(word.b())?;
                register_count(registers, word.c(), 2)?;
            }
            // Unary primitives: A destination, B source register (`AddCar`
            // additionally reads A as its accumulator).
            Opcode::Car
            | Opcode::Cdr
            | Opcode::NullP
            | Opcode::PairP
            | Opcode::StringLength
            | Opcode::CharToInteger
            | Opcode::AddCar => {
                register(word.a())?;
                register(word.b())?;
            }
            // `cons`, `string-ref`, and the fused element accumulates: A
            // destination, B/C the two register operands (the executor reads C
            // as a register unconditionally, so no RK form is permitted here).
            Opcode::Cons | Opcode::StringRef | Opcode::AddVectorRef | Opcode::AddStringRefCode => {
                register(word.a())?;
                register(word.b())?;
                register(word.c())?;
            }
            // The two-fetch fusions carry their extra register operands in a
            // consumed `ExtraArg` successor: `AddMulVectorRef` packs the second
            // vector/index pair as `(b2 << 8) | c2`, `VectorRefVectorRef` holds
            // the outer index register. The executor reads the packed fields as
            // registers unconditionally, so all of them must be proven here.
            Opcode::AddMulVectorRef | Opcode::VectorRefVectorRef => {
                register(word.a())?;
                register(word.b())?;
                register(word.c())?;
                let extra = chunk
                    .code
                    .get(pc + 1)
                    .copied()
                    .ok_or_else(|| invalid("fused vector fetch lacks EXTRAARG"))?;
                if extra.opcode()? != Opcode::ExtraArg {
                    return Err(invalid("fused vector fetch lacks EXTRAARG"));
                }
                let packed = extra.ax_value();
                if opcode == Opcode::AddMulVectorRef {
                    if packed >= 1 << 16 {
                        return Err(invalid("packed register operands exceed two bytes"));
                    }
                    register((packed >> 8) as u8)?;
                    register((packed & 0xFF) as u8)?;
                } else {
                    if packed >= 1 << 8 {
                        return Err(invalid("packed register operand exceeds one byte"));
                    }
                    register(packed as u8)?;
                }
            }
            Opcode::TestLess
            | Opcode::TestLessEqual
            | Opcode::TestEqual
            | Opcode::TestGreater
            | Opcode::TestGreaterEqual => {
                // A carries the polarity bit (0/1). B and RK(C) are the operands.
                if word.a() > 1 {
                    return Err(invalid("test opcode polarity must be 0 or 1"));
                }
                register(word.b())?;
                if word.k() {
                    constant(chunk, u32::from(word.c()))?;
                } else {
                    register(word.c())?;
                }
                // The executor consumes the following word as the branch target,
                // so it must be a `Jump` (whose own arm validates the target).
                let successor = chunk
                    .code
                    .get(pc + 1)
                    .ok_or_else(|| invalid("test opcode must be followed by a jump"))?;
                if successor.opcode()? != Opcode::Jump {
                    return Err(invalid("test opcode must be followed by a jump"));
                }
            }
            Opcode::TestNull | Opcode::TestPair => {
                // Fused predicate-and-branch: A carries the polarity bit (0/1),
                // B the sole source register (C/k are unused). Like the
                // comparison `Test*` family the executor consumes the following
                // word as the branch target, so it must be a `Jump`.
                if word.a() > 1 {
                    return Err(invalid("test opcode polarity must be 0 or 1"));
                }
                register(word.b())?;
                let successor = chunk
                    .code
                    .get(pc + 1)
                    .ok_or_else(|| invalid("test opcode must be followed by a jump"))?;
                if successor.opcode()? != Opcode::Jump {
                    return Err(invalid("test opcode must be followed by a jump"));
                }
            }
            Opcode::LoopBack => {
                // A is the counter register. C is a signed step (i8). B/k are
                // unused. Like the `Test*` family the executor consumes the
                // following word as the back-edge, so it must be a `Jump`.
                register(word.a())?;
                let successor = chunk
                    .code
                    .get(pc + 1)
                    .ok_or_else(|| invalid("loop-back opcode must be followed by a jump"))?;
                if successor.opcode()? != Opcode::Jump {
                    return Err(invalid("loop-back opcode must be followed by a jump"));
                }
            }
            Opcode::LoopBackWhileNotEqual
            | Opcode::LoopBackWhileNotLess
            | Opcode::LoopBackWhileNotLessEqual => {
                // A is the counter register (stepped +1 in place); B and RK(C)
                // are the comparison operands replicated verbatim from the loop
                // header's exit test (`=`, `<`, or `<=` respectively - the
                // compare kind is opcode identity, never an operand swap). The
                // executor consumes the following word as the body back-edge,
                // so it must be a `Jump`. The exit `Jump` emitted after it
                // verifies as an ordinary word (and the `fall_through` rule
                // proves it exists).
                register(word.a())?;
                register(word.b())?;
                if word.k() {
                    constant(chunk, u32::from(word.c()))?;
                } else {
                    register(word.c())?;
                }
                let successor = chunk
                    .code
                    .get(pc + 1)
                    .ok_or_else(|| invalid("loop-back opcode must be followed by a jump"))?;
                if successor.opcode()? != Opcode::Jump {
                    return Err(invalid("loop-back opcode must be followed by a jump"));
                }
            }
            Opcode::TestLessFixnum | Opcode::TestLessEqualFixnum | Opcode::TestEqualFixnum => {
                // The fixnum-constant specialization of the comparison `Test*`
                // family: identical layout, `k` must be set and the constant
                // must hold an inline fixnum, so the executor compares raw
                // payloads without re-classifying the constant.
                if word.a() > 1 {
                    return Err(invalid("test opcode polarity must be 0 or 1"));
                }
                register(word.b())?;
                if !word.k() {
                    return Err(invalid("fixnum test requires a constant operand"));
                }
                constant(chunk, u32::from(word.c()))?;
                if chunk.constants[word.c() as usize].as_fixnum().is_none() {
                    return Err(invalid("fixnum test requires a fixnum constant"));
                }
                let successor = chunk
                    .code
                    .get(pc + 1)
                    .ok_or_else(|| invalid("test opcode must be followed by a jump"))?;
                if successor.opcode()? != Opcode::Jump {
                    return Err(invalid("test opcode must be followed by a jump"));
                }
            }
            Opcode::AddFixnumK | Opcode::SubtractFixnumK => {
                // Arithmetic specialized for a fixnum constant operand (proved
                // here, relied on by the executor to skip re-classifying the
                // constant): `rA = rB op constants[C]` with `k` always set.
                register(word.a())?;
                register(word.b())?;
                if !word.k() {
                    return Err(invalid("fixnum arithmetic requires a constant operand"));
                }
                constant(chunk, u32::from(word.c()))?;
                if chunk.constants[word.c() as usize].as_fixnum().is_none() {
                    return Err(invalid("fixnum arithmetic requires a fixnum constant"));
                }
            }
            Opcode::AddSubFixnumK => {
                // The fused wide-literal accumulate `rA = (rA + constants[B])
                // - constants[C]` with `k` always set. Nonstandard field use
                // (`B` is a constant index, not a register - precedent: `LoadK`
                // bx, `LoopBack` C-as-step). Both constants must hold inline
                // fixnums, so the executor operates on raw payloads without
                // re-classifying either one.
                register(word.a())?;
                if !word.k() {
                    return Err(invalid("fixnum arithmetic requires a constant operand"));
                }
                constant(chunk, u32::from(word.b()))?;
                if chunk.constants[word.b() as usize].as_fixnum().is_none() {
                    return Err(invalid("fixnum arithmetic requires a fixnum constant"));
                }
                constant(chunk, u32::from(word.c()))?;
                if chunk.constants[word.c() as usize].as_fixnum().is_none() {
                    return Err(invalid("fixnum arithmetic requires a fixnum constant"));
                }
            }
            Opcode::LoopBackWhileNotEqualFixnum => {
                // The fixnum-constant specialization of `LoopBackWhileNotEqual`:
                // identical layout, but `k` must be set and the constant must
                // hold an inline fixnum. The executor relies on this to compare
                // raw payloads without re-classifying the constant every
                // iteration.
                register(word.a())?;
                register(word.b())?;
                if !word.k() {
                    return Err(invalid("fixnum loop-back requires a constant operand"));
                }
                constant(chunk, u32::from(word.c()))?;
                if chunk.constants[word.c() as usize].as_fixnum().is_none() {
                    return Err(invalid("fixnum loop-back requires a fixnum constant"));
                }
                let successor = chunk
                    .code
                    .get(pc + 1)
                    .ok_or_else(|| invalid("loop-back opcode must be followed by a jump"))?;
                if successor.opcode()? != Opcode::Jump {
                    return Err(invalid("loop-back opcode must be followed by a jump"));
                }
            }
            Opcode::LoopBackStepWhileLessEqual | Opcode::LoopBackStepWhileLess => {
                // A is the counter register, stepped in place by the step
                // REGISTER B, and doubles as the comparison's left operand
                // (the fall-into-body loop shape guarantees the header test
                // compares the counter itself). RK(C) is the limit. The
                // executor consumes the following word as the body back-edge,
                // so it must be a `Jump`.
                register(word.a())?;
                register(word.b())?;
                if word.k() {
                    constant(chunk, u32::from(word.c()))?;
                } else {
                    register(word.c())?;
                }
                let successor = chunk
                    .code
                    .get(pc + 1)
                    .ok_or_else(|| invalid("loop-back opcode must be followed by a jump"))?;
                if successor.opcode()? != Opcode::Jump {
                    return Err(invalid("loop-back opcode must be followed by a jump"));
                }
            }
            Opcode::TestVectorRef => {
                // Fused `(vector-ref B C)`-as-condition branch: A carries the
                // polarity bit (0/1), B the vector register, C the index
                // register (always a register - mirrors `VectorRef`'s
                // register-only index rule for the fused accumulate family).
                // Like the `Test*` family the executor consumes the following
                // word as the branch target, so it must be a `Jump`.
                if word.a() > 1 {
                    return Err(invalid("test opcode polarity must be 0 or 1"));
                }
                register(word.b())?;
                register(word.c())?;
                let successor = chunk
                    .code
                    .get(pc + 1)
                    .ok_or_else(|| invalid("test opcode must be followed by a jump"))?;
                if successor.opcode()? != Opcode::Jump {
                    return Err(invalid("test opcode must be followed by a jump"));
                }
            }
            Opcode::Cold => {
                let instruction = chunk
                    .cold
                    .get(word.bx() as usize)
                    .ok_or_else(|| invalid("cold operand is out of bounds"))?;
                verify_cold(instruction, &register, registers)?;
            }
        }
    }
    for &index in chunk.boxed_locals.iter() {
        register(index)?;
    }
    for prototype in &chunk.closures {
        if prototype.captures.len() != prototype.chunk.capture_kinds.len() {
            return Err(invalid("closure capture count does not match child chunk"));
        }
        for (slot, source) in prototype.captures.iter().enumerate() {
            let child_kind = prototype.chunk.capture_kinds[slot];
            match source {
                // A local source's representation is flow-dependent (`BoxLocal`
                // can box a slot mid-body), so it cannot be checked statically;
                // the capture opcodes fail closed at runtime instead.
                CaptureSource::Local(index) => register(*index)?,
                // A chained capture must keep the originating binding's kind:
                // the child's slot and the parent's source slot have to agree.
                CaptureSource::Capture(index) => {
                    match chunk.capture_kinds.get(usize::from(*index)) {
                        Some(&parent_kind) if parent_kind == child_kind => {}
                        Some(_) => {
                            return Err(invalid("capture chain changes the capture's kind"));
                        }
                        None => return Err(invalid("capture source is out of bounds")),
                    }
                }
            }
        }
        verify_chunk(&prototype.chunk, visited)?;
    }
    Ok(())
}

fn register_range(registers: usize, first: u8, encoded_count: u8) -> Result<(), Error> {
    let count = encoded_count.saturating_sub(1) as usize;
    let count = count.max(1);
    if usize::from(first).saturating_add(count) <= registers {
        Ok(())
    } else {
        Err(invalid("register range is out of bounds"))
    }
}

fn register_count(registers: usize, first: u8, count: u8) -> Result<(), Error> {
    if usize::from(first).saturating_add(usize::from(count)) <= registers {
        Ok(())
    } else {
        Err(invalid("register range is out of bounds"))
    }
}

fn verify_cold(
    instruction: &ColdInstruction,
    register: &impl Fn(u8) -> Result<(), Error>,
    registers: usize,
) -> Result<(), Error> {
    match *instruction {
        ColdInstruction::ValueCountError { .. } | ColdInstruction::PopHandler => {}
        ColdInstruction::CallWithValues {
            destination,
            producer,
            consumer,
            ..
        } => {
            register(destination)?;
            register(producer)?;
            register(consumer)?;
        }
        ColdInstruction::MakePromise {
            destination, thunk, ..
        } => {
            register(destination)?;
            register(thunk)?;
        }
        ColdInstruction::Force {
            destination,
            promise,
            ..
        } => {
            register(destination)?;
            register(promise)?;
        }
        ColdInstruction::PushHandler { handler } => register(handler)?,
        ColdInstruction::Raise {
            destination,
            object,
            ..
        } => {
            register(destination)?;
            register(object)?;
        }
        ColdInstruction::CaptureContinuation {
            destination,
            procedure,
            ..
        } => {
            register(destination)?;
            register(procedure)?;
        }
        ColdInstruction::DynamicWind {
            destination,
            before,
            thunk,
            after,
            ..
        } => {
            register(destination)?;
            register(before)?;
            register(thunk)?;
            register(after)?;
        }
        ColdInstruction::CallWithPort {
            destination,
            port,
            procedure,
            ..
        } => {
            register(destination)?;
            register(port)?;
            register(procedure)?;
        }
        ColdInstruction::CallWithFile {
            destination,
            path,
            procedure,
            ..
        } => {
            register(destination)?;
            register(path)?;
            register(procedure)?;
        }
        ColdInstruction::WithFile {
            destination,
            path,
            thunk,
            ..
        } => {
            register(destination)?;
            register(path)?;
            register(thunk)?;
        }
        ColdInstruction::Load {
            destination,
            path,
            environment,
            ..
        } => {
            register(destination)?;
            register(path)?;
            if let Some(environment) = environment {
                register(environment)?;
            }
        }
        ColdInstruction::PushParameters { first, count } => {
            register_count(registers, first, count.saturating_mul(2))?;
        }
        ColdInstruction::PopParameters { .. } => {}
        ColdInstruction::MakeParameter {
            destination,
            initial,
            converter,
        } => {
            register(destination)?;
            register(initial)?;
            if let Some(converter) = converter {
                register(converter)?;
            }
        }
        ColdInstruction::MakeError {
            destination,
            message,
            first_irritant,
            count,
        } => {
            register(destination)?;
            register(message)?;
            register_count(registers, first_irritant, count)?;
        }
        ColdInstruction::LoadNumber { destination, .. } => register(destination)?,
    }
    Ok(())
}

fn constant(chunk: &Chunk, index: u32) -> Result<(), Error> {
    if (index as usize) < chunk.constants.len() {
        Ok(())
    } else {
        Err(invalid("constant operand is out of bounds"))
    }
}

fn global(chunk: &Chunk, index: u32) -> Result<(), Error> {
    if (index as usize) < chunk.global_operands.len() {
        Ok(())
    } else {
        Err(invalid("global operand is out of bounds"))
    }
}

fn closure(chunk: &Chunk, index: u32) -> Result<(), Error> {
    if (index as usize) < chunk.closures.len() {
        Ok(())
    } else {
        Err(invalid("closure operand is out of bounds"))
    }
}

fn jump_target(chunk: &Chunk, pc: usize, offset: isize) -> Result<(), Error> {
    let target = pc
        .checked_add(1)
        .and_then(|next| next.checked_add_signed(offset))
        .ok_or_else(|| invalid("jump target is out of bounds"))?;
    if target >= chunk.code.len() || chunk.code[target].opcode()? == Opcode::ExtraArg {
        Err(invalid("jump target is out of bounds"))
    } else {
        Ok(())
    }
}

fn compile_error(message: &str) -> Error {
    Error::plain(ErrorKind::CompileError, message)
}

fn invalid(message: &str) -> Error {
    Error::plain(ErrorKind::InvalidBytecode, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_four_bytes_and_fields_round_trip() {
        assert_eq!(size_of::<Word>(), 4);
        let word = Word::abc(Opcode::Add, 3, 4, 5, true);
        assert_eq!(
            (
                word.opcode().unwrap(),
                word.a(),
                word.b(),
                word.c(),
                word.k()
            ),
            (Opcode::Add, 3, 4, 5, true)
        );
    }

    #[test]
    fn signed_jumps_round_trip() {
        assert_eq!(Word::sj(Opcode::Jump, -17).unwrap().signed_jump(), -17);
        assert_eq!(Word::asbx(Opcode::JumpFalse, 2, 91).unwrap().sbx(), 91);
    }

    fn capture_chunk(kind: CaptureKind, opcode: Opcode) -> Chunk {
        Chunk {
            code: vec![
                Word::abc(opcode, 0, 0, 0, false),
                Word::abc(Opcode::Return, 0, 1, 0, false),
            ],
            constants: Vec::new(),
            global_operands: Vec::new(),
            closures: Vec::new(),
            cold: Vec::new(),
            arity: Arity::Exact(0),
            max_registers: 1,
            capture_kinds: Box::from([kind]),
            boxed_locals: Box::from([]),
        }
    }

    #[test]
    fn verifier_enforces_capture_kinds() {
        // Matching kinds pass.
        for (kind, opcode) in [
            (CaptureKind::Cell, Opcode::GetCapture),
            (CaptureKind::Cell, Opcode::SetCapture),
            (CaptureKind::Value, Opcode::GetCaptureValue),
        ] {
            assert!(CompiledModule::new(Rc::new(capture_chunk(kind, opcode))).is_ok());
        }
        // A cell opcode on a value slot (and vice versa) is rejected.
        for (kind, opcode) in [
            (CaptureKind::Value, Opcode::GetCapture),
            (CaptureKind::Value, Opcode::SetCapture),
            (CaptureKind::Cell, Opcode::GetCaptureValue),
        ] {
            assert!(CompiledModule::new(Rc::new(capture_chunk(kind, opcode))).is_err());
        }
        // Out-of-bounds capture slots are rejected (slot 0 with no captures).
        let mut chunk = capture_chunk(CaptureKind::Cell, Opcode::GetCapture);
        chunk.capture_kinds = Box::from([]);
        assert!(CompiledModule::new(Rc::new(chunk)).is_err());
    }

    #[test]
    fn verifier_rejects_kind_changing_capture_chains() {
        // A parent whose only capture is a `Value` must not feed a child slot
        // declared `Cell` (the kind belongs to the originating binding).
        let child = capture_chunk(CaptureKind::Cell, Opcode::GetCapture);
        let mut parent = capture_chunk(CaptureKind::Value, Opcode::GetCaptureValue);
        parent
            .code
            .insert(0, Word::abx(Opcode::Closure, 0, 0).unwrap());
        parent.closures.push(ClosurePrototype {
            chunk: Rc::new(child),
            captures: vec![CaptureSource::Capture(0)],
        });
        assert!(CompiledModule::new(Rc::new(parent)).is_err());
    }

    fn linear_chunk(code: Vec<Word>) -> Chunk {
        Chunk {
            code,
            constants: Vec::new(),
            global_operands: Vec::new(),
            closures: Vec::new(),
            cold: Vec::new(),
            arity: Arity::Exact(0),
            max_registers: 1,
            capture_kinds: Box::from([]),
            boxed_locals: Box::from([]),
        }
    }

    #[test]
    fn verifier_rejects_instructions_that_fall_off_the_end() {
        // A non-terminal opcode as the last word would fall through past the
        // end of the code.
        let chunk = linear_chunk(vec![Word::abc(Opcode::Move, 0, 0, 0, false)]);
        assert!(CompiledModule::new(Rc::new(chunk)).is_err());
        // A fused test consumes the following jump, so its not-taken path
        // resumes at pc + 2, which must also exist.
        let chunk = linear_chunk(vec![
            Word::abc(Opcode::TestEqual, 0, 0, 0, false),
            Word::sj(Opcode::Jump, -2).unwrap(),
        ]);
        assert!(CompiledModule::new(Rc::new(chunk)).is_err());
        // A cold instruction resumes at pc + 1 after its out-of-line work.
        let mut chunk = linear_chunk(vec![Word::abx(Opcode::Cold, 0, 0).unwrap()]);
        chunk.cold = vec![ColdInstruction::PopHandler];
        assert!(CompiledModule::new(Rc::new(chunk)).is_err());
    }

    #[test]
    fn verifier_constrains_the_fused_vector_fetch_extra_arg() {
        // A fused double-fetch word without its `ExtraArg` successor is
        // rejected (the executor reads it unconditionally).
        let chunk = linear_chunk(vec![
            Word::abc(Opcode::AddMulVectorRef, 0, 0, 0, false),
            Word::abc(Opcode::Return, 0, 1, 0, false),
        ]);
        assert!(CompiledModule::new(Rc::new(chunk)).is_err());
        // A packed register beyond max_registers is rejected (second vector
        // register 1 with a one-register frame).
        let chunk = linear_chunk(vec![
            Word::abc(Opcode::AddMulVectorRef, 0, 0, 0, false),
            Word::ax(Opcode::ExtraArg, 1 << 8).unwrap(),
            Word::abc(Opcode::Return, 0, 1, 0, false),
        ]);
        assert!(CompiledModule::new(Rc::new(chunk)).is_err());
        // Same for the chained fetch's outer index register.
        let chunk = linear_chunk(vec![
            Word::abc(Opcode::VectorRefVectorRef, 0, 0, 0, false),
            Word::ax(Opcode::ExtraArg, 1).unwrap(),
            Word::abc(Opcode::Return, 0, 1, 0, false),
        ]);
        assert!(CompiledModule::new(Rc::new(chunk)).is_err());
        // The valid shapes pass.
        let chunk = linear_chunk(vec![
            Word::abc(Opcode::AddMulVectorRef, 0, 0, 0, false),
            Word::ax(Opcode::ExtraArg, 0).unwrap(),
            Word::abc(Opcode::Return, 0, 1, 0, false),
        ]);
        assert!(CompiledModule::new(Rc::new(chunk)).is_ok());
        let chunk = linear_chunk(vec![
            Word::abc(Opcode::VectorRefVectorRef, 0, 0, 0, false),
            Word::ax(Opcode::ExtraArg, 0).unwrap(),
            Word::abc(Opcode::Return, 0, 1, 0, false),
        ]);
        assert!(CompiledModule::new(Rc::new(chunk)).is_ok());
        // An `ExtraArg` after a non-consuming word stays rejected.
        let chunk = linear_chunk(vec![
            Word::abc(Opcode::Move, 0, 0, 0, false),
            Word::ax(Opcode::ExtraArg, 0).unwrap(),
            Word::abc(Opcode::Return, 0, 1, 0, false),
        ]);
        assert!(CompiledModule::new(Rc::new(chunk)).is_err());
    }

    #[test]
    fn verifier_requires_a_jump_after_the_greater_tests() {
        for opcode in [Opcode::TestGreater, Opcode::TestGreaterEqual] {
            // The executor consumes the following word as the branch target,
            // so a non-`Jump` successor is rejected.
            let chunk = linear_chunk(vec![
                Word::abc(opcode, 0, 0, 0, false),
                Word::abc(Opcode::Return, 0, 1, 0, false),
                Word::abc(Opcode::Return, 0, 1, 0, false),
            ]);
            assert!(CompiledModule::new(Rc::new(chunk)).is_err());
            // The same shape with the `Jump` in place passes.
            let chunk = linear_chunk(vec![
                Word::abc(opcode, 0, 0, 0, false),
                Word::sj(Opcode::Jump, 0).unwrap(),
                Word::abc(Opcode::Return, 0, 1, 0, false),
            ]);
            assert!(CompiledModule::new(Rc::new(chunk)).is_ok());
        }
    }

    #[test]
    fn verifier_constrains_the_fused_add_sub_constants() {
        let words = |k: bool| {
            vec![
                Word::abc(Opcode::AddSubFixnumK, 0, 0, 1, k),
                Word::abc(Opcode::Return, 0, 1, 0, false),
            ]
        };
        // Register operands (no k bit) are rejected.
        let mut chunk = linear_chunk(words(false));
        chunk.constants = vec![Value::integer(1), Value::integer(2)];
        assert!(CompiledModule::new(Rc::new(chunk)).is_err());
        // A non-fixnum constant in either slot is rejected (the executor
        // reads raw fixnum payloads from both).
        let mut chunk = linear_chunk(words(true));
        chunk.constants = vec![Value::float(1.0), Value::integer(2)];
        assert!(CompiledModule::new(Rc::new(chunk)).is_err());
        let mut chunk = linear_chunk(words(true));
        chunk.constants = vec![Value::integer(1), Value::float(2.0)];
        assert!(CompiledModule::new(Rc::new(chunk)).is_err());
        // Both slots holding inline fixnums pass.
        let mut chunk = linear_chunk(words(true));
        chunk.constants = vec![Value::integer(1), Value::integer(2)];
        assert!(CompiledModule::new(Rc::new(chunk)).is_ok());
    }

    #[test]
    fn verifier_accepts_a_terminal_backward_jump() {
        // An infinite self-loop ends in a backward `Jump`: terminal, because
        // it never falls through and its target is verified in bounds.
        let chunk = linear_chunk(vec![
            Word::abc(Opcode::Move, 0, 0, 0, false),
            Word::sj(Opcode::Jump, -2).unwrap(),
        ]);
        assert!(CompiledModule::new(Rc::new(chunk)).is_ok());
    }

    #[test]
    fn verifier_rejects_heap_references_in_the_constant_table() {
        let code = vec![
            Word::abx(Opcode::LoadK, 0, 0).unwrap(),
            Word::abc(Opcode::Return, 0, 2, 0, false),
        ];
        let mut chunk = linear_chunk(code.clone());
        chunk.constants = vec![Value::heap(crate::value::GcRef(0))];
        assert!(CompiledModule::new(Rc::new(chunk)).is_err());
        // The same chunk with an immediate constant passes.
        let mut chunk = linear_chunk(code);
        chunk.constants = vec![Value::integer(1)];
        assert!(CompiledModule::new(Rc::new(chunk)).is_ok());
    }

    #[test]
    fn test_opcodes_round_trip() {
        for opcode in [
            Opcode::TestLess,
            Opcode::TestLessEqual,
            Opcode::TestEqual,
            Opcode::TestGreater,
            Opcode::TestGreaterEqual,
        ] {
            let word = Word::abc(opcode, 1, 4, 7, true);
            assert_eq!(
                (
                    word.opcode().unwrap(),
                    word.a(),
                    word.b(),
                    word.c(),
                    word.k()
                ),
                (opcode, 1, 4, 7, true)
            );
        }
    }
}
