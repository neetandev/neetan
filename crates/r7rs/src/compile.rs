//! Core-expression to fixed-width register bytecode compilation.

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::{
    CoreExpr, Error, ErrorKind, Limits, Value,
    bytecode::{
        Arity, CaptureKind, CaptureSource, Chunk, ClosurePrototype, ColdInstruction,
        CompiledModule, ExpectedResults, GlobalOperand, MAX_REGISTERS, Opcode, Word,
    },
};

/// A literal as the compiler sees it, before deciding how it loads at runtime.
/// `Value` literals land in the chunk's constant table (immediate values only,
/// read by `LoadK`/RK operands with no allocation); `Number` literals have no
/// inline representation and compile to [`ColdInstruction::LoadNumber`].
#[derive(Clone, Copy, Debug)]
enum Constant {
    Value(Value),
    Number(crate::Number),
}

mod analysis;
mod condition;
mod expr;

use condition::*;
use expr::*;

#[derive(Clone, Copy)]
enum Access {
    Local(u8),
    Capture(u8, CaptureKind),
    Global,
}

struct Environment<'a> {
    locals: HashMap<String, u8>,
    /// Capture slot index and kind per captured name. The kind is the
    /// originating binding's (see [`CaptureKind`]), recorded when the capture
    /// is created and propagated unchanged along chained captures.
    captures: HashMap<String, (u8, CaptureKind)>,
    /// Local register indices whose slot holds a heap `Box` cell (see
    /// [`Chunk::boxed_locals`]). Reads/writes of these locals are emitted as
    /// `GetLocalBox`/`SetLocalBox` rather than plain register moves.
    boxed: HashSet<u8>,
    /// Register indices of *inlined* let-bindings that need a box cell. Unlike
    /// [`boxed`] (frame parameters, boxed at entry via `boxed_locals`), these
    /// are boxed mid-body by a `BoxLocal` opcode at their binding point and are
    /// tracked only for the extent of the inlined body (see
    /// [`compile_inlined_application`]), so they never enter `boxed_locals`.
    inline_boxed: HashSet<u8>,
    outer: Option<&'a Environment<'a>>,
}

impl Environment<'_> {
    fn is_boxed_local(&self, index: u8) -> bool {
        self.boxed.contains(&index) || self.inline_boxed.contains(&index)
    }

    /// The capture kind `name` would have if captured from this environment
    /// chain: a local origin is [`CaptureKind::Cell`] iff its slot is boxed
    /// (i.e. the binding is mutated), [`CaptureKind::Value`] otherwise. An
    /// existing capture propagates its recorded kind unchanged. `None` means
    /// the name resolves to a global.
    fn capture_kind(&self, name: &str) -> Option<CaptureKind> {
        if let Some(&index) = self.locals.get(name) {
            return Some(if self.is_boxed_local(index) {
                CaptureKind::Cell
            } else {
                CaptureKind::Value
            });
        }
        if let Some((_, kind)) = self.captures.get(name) {
            return Some(*kind);
        }
        self.outer.and_then(|outer| outer.capture_kind(name))
    }

    fn resolve(&mut self, name: &str) -> Result<Access, Error> {
        if let Some(index) = self.locals.get(name) {
            return Ok(Access::Local(*index));
        }
        if let Some((index, kind)) = self.captures.get(name) {
            return Ok(Access::Capture(*index, *kind));
        }
        let Some(outer) = self.outer else {
            return Ok(Access::Global);
        };
        let Some(kind) = outer.capture_kind(name) else {
            return Ok(Access::Global);
        };
        let index = register_index(self.captures.len(), "too many closure captures")?;
        self.captures.insert(name.to_owned(), (index, kind));
        Ok(Access::Capture(index, kind))
    }
}

#[derive(Clone, Copy)]
enum Mode {
    One(u8),
    Discard,
    All(u8),
    Return,
}

impl Mode {
    fn expected(self) -> ExpectedResults {
        match self {
            Self::One(_) => ExpectedResults::One,
            Self::Discard => ExpectedResults::Discard,
            Self::All(_) | Self::Return => ExpectedResults::All,
        }
    }
}

struct Registers {
    next: u16,
    high: u16,
}

impl Registers {
    fn new(locals: usize) -> Result<Self, Error> {
        if locals > MAX_REGISTERS {
            return Err(compile_error("procedure requires more than 255 registers"));
        }
        Ok(Self {
            next: locals as u16,
            high: locals as u16,
        })
    }

    fn mark(&self) -> u16 {
        self.next
    }

    fn reset(&mut self, mark: u16) {
        self.next = mark;
    }

    fn acquire(&mut self) -> Result<u8, Error> {
        self.acquire_block(1)
    }

    fn acquire_block(&mut self, count: usize) -> Result<u8, Error> {
        let end = self.next.saturating_add(count as u16);
        if end > MAX_REGISTERS as u16 {
            return Err(compile_error("procedure requires more than 255 registers"));
        }
        let first = self.next as u8;
        self.next = end;
        self.high = self.high.max(end);
        Ok(first)
    }

    fn ensure(&mut self, last_exclusive: usize) -> Result<(), Error> {
        if last_exclusive > MAX_REGISTERS {
            return Err(compile_error("procedure requires more than 255 registers"));
        }
        self.next = self.next.max(last_exclusive as u16);
        self.high = self.high.max(self.next);
        Ok(())
    }
}

/// A flattened named-loop active while its body compiles. A tail call to `name`
/// updates the parameter registers and jumps back to `header`, turning the
/// self-recursion into a real loop within the enclosing frame. Nested loops
/// stack, so an inner body may also tail-jump to an outer loop's header. A fresh
/// [`Code`] is built per lambda frame, so this stack can never cross a frame
/// boundary.
struct LoopFrame {
    name: String,
    base: u8,
    arity: usize,
    header: usize,
}

#[derive(Default)]
struct Code {
    words: Vec<Word>,
    constants: Vec<Value>,
    globals: Vec<Rc<GlobalOperand>>,
    closures: Vec<ClosurePrototype>,
    cold: Vec<ColdInstruction>,
    loops: Vec<LoopFrame>,
}

impl Code {
    fn emit(&mut self, word: Word) -> usize {
        let at = self.words.len();
        self.words.push(word);
        at
    }

    fn patch_jump(
        &mut self,
        at: usize,
        target: usize,
        conditional: Option<u8>,
    ) -> Result<(), Error> {
        let offset = target as isize - at as isize - 1;
        self.words[at] = match conditional {
            Some(register) => Word::asbx(Opcode::JumpFalse, register, offset)?,
            None => Word::sj(Opcode::Jump, offset)?,
        };
        Ok(())
    }

    fn constant(&mut self, value: Value) -> Result<u32, Error> {
        // Bitwise identity dedups value constants while still keeping distinct
        // signed-zero literals apart (their raw bits differ).
        if let Some(index) = self
            .constants
            .iter()
            .position(|candidate| candidate.0 == value.0)
        {
            return Ok(index as u32);
        }
        let index =
            u32::try_from(self.constants.len()).map_err(|_| compile_error("too many constants"))?;
        self.constants.push(value);
        Ok(index)
    }

    fn global(&mut self, name: &str) -> Result<u32, Error> {
        if let Some(index) = self
            .globals
            .iter()
            .position(|operand| operand.name.as_ref() == name)
        {
            return Ok(index as u32);
        }
        let index =
            u32::try_from(self.globals.len()).map_err(|_| compile_error("too many globals"))?;
        self.globals
            .push(Rc::new(GlobalOperand::new(Rc::from(name))));
        Ok(index)
    }

    fn cold(&mut self, instruction: ColdInstruction) -> Result<Word, Error> {
        let index =
            u32::try_from(self.cold.len()).map_err(|_| compile_error("too many cold operands"))?;
        self.cold.push(instruction);
        Word::abx(Opcode::Cold, 0, index)
    }

    fn emit_cold(&mut self, instruction: ColdInstruction) -> Result<usize, Error> {
        let word = self.cold(instruction)?;
        Ok(self.emit(word))
    }

    fn load_constant(&mut self, destination: u8, constant: Constant) -> Result<(), Error> {
        let value = match constant {
            Constant::Value(value) => value,
            // No inline representation: materialized by a cold instruction so
            // the hot constant table stays allocation-free.
            Constant::Number(number) => {
                self.emit_cold(ColdInstruction::LoadNumber {
                    destination,
                    number,
                })?;
                return Ok(());
            }
        };
        let index = self.constant(value)?;
        if let Ok(word) = Word::abx(Opcode::LoadK, destination, index) {
            self.emit(word);
        } else {
            self.emit(Word::abc(Opcode::LoadKx, destination, 0, 0, false));
            self.emit(Word::ax(Opcode::ExtraArg, index)?);
        }
        Ok(())
    }
}

pub(crate) fn compile(expression: &CoreExpr, _limits: &Limits) -> Result<CompiledModule, Error> {
    let env = Environment {
        locals: HashMap::new(),
        captures: HashMap::new(),
        boxed: HashSet::new(),
        inline_boxed: HashSet::new(),
        outer: None,
    };
    let (chunk, _) = compile_chunk(expression, env, Arity::Exact(0))?;
    CompiledModule::new(Rc::new(chunk))
}

fn compile_chunk<'a>(
    expression: &CoreExpr,
    mut env: Environment<'a>,
    arity: Arity,
) -> Result<(Chunk, Vec<(u8, String)>), Error> {
    let mut code = Code::default();
    let mut registers = Registers::new(env.locals.len())?;
    compile_expression(
        expression,
        &mut env,
        &mut code,
        &mut registers,
        Mode::Return,
    )?;
    let mut capture_list = env
        .captures
        .into_iter()
        .map(|(name, (index, kind))| (index, name, kind))
        .collect::<Vec<_>>();
    capture_list.sort_by_key(|(index, _, _)| *index);
    register_index(capture_list.len(), "too many closure captures")?;
    let capture_kinds = capture_list
        .iter()
        .map(|(_, _, kind)| *kind)
        .collect::<Box<[_]>>();
    let capture_names = capture_list
        .into_iter()
        .map(|(index, name, _)| (index, name))
        .collect::<Vec<_>>();
    let max_registers = registers.high.max(1) as u8;
    let mut boxed_locals = env.boxed.iter().copied().collect::<Vec<_>>();
    boxed_locals.sort_unstable();
    Ok((
        Chunk {
            code: code.words,
            constants: code.constants,
            global_operands: code.globals,
            closures: code.closures,
            cold: code.cold,
            arity,
            max_registers,
            capture_kinds,
            boxed_locals: boxed_locals.into_boxed_slice(),
        },
        capture_names,
    ))
}

fn compile_expression(
    expression: &CoreExpr,
    env: &mut Environment<'_>,
    code: &mut Code,
    registers: &mut Registers,
    mode: Mode,
) -> Result<Option<u8>, Error> {
    match expression {
        CoreExpr::Literal(value) => {
            compile_constant(Constant::Value(*value), code, registers, mode)
        }
        CoreExpr::NumberLiteral(value) => {
            compile_constant(Constant::Number(*value), code, registers, mode)
        }
        CoreExpr::Variable(name) => {
            let access = env.resolve(name)?;
            // Tail-returning a non-boxed local reads its home register directly:
            // `Return home` with no intervening `Move` into a fresh result
            // register (the value already lives in a stable slot, and nothing
            // runs between here and the return that could clobber it).
            if matches!(mode, Mode::Return)
                && let Access::Local(source) = access
                && !env.is_boxed_local(source)
            {
                code.emit(Word::abc(Opcode::Return, source, 2, 0, false));
                return Ok(None);
            }
            let Some(destination) = output_register(registers, mode)? else {
                return Ok(Some(0));
            };
            match access {
                Access::Local(source) => {
                    if env.is_boxed_local(source) {
                        code.emit(Word::abc(
                            Opcode::GetLocalBox,
                            destination,
                            source,
                            0,
                            false,
                        ));
                    } else if destination != source {
                        // A non-boxed local read into its own home register is a
                        // no-op (arises when an accumulator's loop-update branch
                        // leaves the value unchanged, writing straight to home).
                        code.emit(Word::abc(Opcode::Move, destination, source, 0, false));
                    }
                }
                Access::Capture(source, kind) => {
                    // A mutable (cell) capture reads through its shared heap
                    // cell; an immutable one reads the raw captured value.
                    let opcode = match kind {
                        CaptureKind::Cell => Opcode::GetCapture,
                        CaptureKind::Value => Opcode::GetCaptureValue,
                    };
                    code.emit(Word::abc(opcode, destination, source, 0, false));
                }
                Access::Global => {
                    let global = code.global(name)?;
                    code.emit(Word::abx(Opcode::GetGlobal, destination, global)?);
                }
            };
            finish_value(code, destination, mode)
        }
        CoreExpr::Set { name, value } => {
            let mark = registers.mark();
            let source = registers.acquire()?;
            compile_expression(value, env, code, registers, Mode::One(source))?;
            match env.resolve(name)? {
                Access::Local(destination) => {
                    // A `set!`-mutated local is always boxed (see the boxing
                    // pre-pass), so the store goes through its heap cell.
                    debug_assert!(env.is_boxed_local(destination));
                    code.emit(Word::abc(
                        Opcode::SetLocalBox,
                        destination,
                        source,
                        0,
                        false,
                    ));
                }
                Access::Capture(destination, kind) => {
                    // A `set!` marks the binding mutated at its origin, so every
                    // capture of it is a cell by construction.
                    debug_assert!(matches!(kind, CaptureKind::Cell));
                    code.emit(Word::abc(Opcode::SetCapture, source, destination, 0, false));
                }
                Access::Global => {
                    let global = code.global(name)?;
                    code.emit(Word::abx(Opcode::SetGlobal, source, global)?);
                }
            }
            registers.reset(mark);
            compile_constant(Constant::Value(Value::unspecified()), code, registers, mode)
        }
        CoreExpr::Define { name, value } => {
            let mark = registers.mark();
            let source = registers.acquire()?;
            compile_expression(value, env, code, registers, Mode::One(source))?;
            let global = code.global(name)?;
            code.emit(Word::abx(Opcode::SetGlobal, source, global)?);
            registers.reset(mark);
            compile_constant(Constant::Value(Value::unspecified()), code, registers, mode)
        }
        CoreExpr::Begin(expressions) => {
            if expressions.is_empty() {
                return compile_constant(
                    Constant::Value(Value::unspecified()),
                    code,
                    registers,
                    mode,
                );
            }
            for expression in &expressions[..expressions.len() - 1] {
                compile_expression(expression, env, code, registers, Mode::Discard)?;
            }
            compile_expression(
                expressions.last().expect("last"),
                env,
                code,
                registers,
                mode,
            )
        }
        CoreExpr::If(test, consequent, alternate) => {
            let mark = registers.mark();
            // Lower the test in boolean context: emit branches that jump to the
            // alternate when the test is false and fall through into the
            // consequent when true. Leaf comparisons fuse into `Test` + `Jump`
            // and `and`/`or`/`not` short-circuit down to those leaves.
            let else_jumps = compile_condition(test, env, code, registers, false)?;
            registers.reset(mark);
            let left = compile_expression(consequent, env, code, registers, mode)?;
            if matches!(mode, Mode::Return) {
                patch_all(code, &else_jumps, code.words.len())?;
                registers.reset(mark);
                compile_expression(alternate, env, code, registers, mode)?;
                return Ok(None);
            }
            let done = code.emit(Word::sj(Opcode::Jump, 0)?);
            patch_all(code, &else_jumps, code.words.len())?;
            registers.reset(mark);
            let right = compile_expression(alternate, env, code, registers, mode)?;
            if code.words.len() == done + 1 {
                // The alternate emitted nothing, so `done` is the last word and
                // would jump to the instruction immediately after itself - a
                // dead jump. Drop it and land the else-branch on the fall-through
                // (both branches converge here regardless).
                code.words.pop();
                patch_all(code, &else_jumps, code.words.len())?;
            } else {
                code.patch_jump(done, code.words.len(), None)?;
            }
            Ok(if left == right { left } else { None })
        }
        CoreExpr::Lambda { params, body } => {
            compile_lambda(params, None, body, env, code, registers, mode)
        }
        CoreExpr::LambdaRest {
            required,
            rest,
            body,
        } => compile_lambda(required, Some(rest), body, env, code, registers, mode),
        CoreExpr::CaseLambda { clauses } => {
            if clauses.iter().any(|clause| {
                !matches!(
                    clause,
                    CoreExpr::Lambda { .. } | CoreExpr::LambdaRest { .. }
                )
            }) {
                return Err(compile_error(
                    "case-lambda clauses must be lambda expressions",
                ));
            }
            let Some(destination) = output_register(registers, mode)? else {
                for clause in clauses {
                    compile_expression(clause, env, code, registers, Mode::Discard)?;
                }
                return Ok(Some(0));
            };
            let mark = registers.mark();
            let first = registers.acquire_block(clauses.len())?;
            for (offset, clause) in clauses.iter().enumerate() {
                compile_expression(
                    clause,
                    env,
                    code,
                    registers,
                    Mode::One(first + offset as u8),
                )?;
            }
            code.emit(Word::abc(
                Opcode::CaseLambda,
                destination,
                first,
                clauses.len() as u8,
                false,
            ));
            registers.reset(mark);
            finish_value(code, destination, mode)
        }
        CoreExpr::Call {
            procedure,
            arguments,
        } => compile_call(procedure, arguments, env, code, registers, mode),
        CoreExpr::NamedLet {
            name,
            params,
            inits,
            body,
        } => {
            if params.len() != inits.len() {
                return Err(compile_error(
                    "named-let parameter and initial-value counts differ",
                ));
            }
            let mut unique = HashSet::with_capacity(params.len());
            if params.iter().any(|parameter| !unique.insert(parameter)) {
                return Err(compile_error("duplicate named-let parameter"));
            }
            compile_named_let(name, params, inits, body, env, code, registers, mode)
        }
        CoreExpr::Values(values) => compile_values(values, env, code, registers, mode),
        CoreExpr::CallWithValues { producer, consumer } => compile_cold_call(
            [producer.as_ref(), consumer.as_ref()],
            env,
            code,
            registers,
            mode,
            |destination, inputs, expected| ColdInstruction::CallWithValues {
                destination,
                producer: inputs[0],
                consumer: inputs[1],
                expected,
            },
        ),
        CoreExpr::Delay(value) | CoreExpr::DelayForce(value) => {
            let lambda = CoreExpr::Lambda {
                params: Vec::new(),
                body: value.clone(),
            };
            let mark = registers.mark();
            let thunk = registers.acquire()?;
            compile_expression(&lambda, env, code, registers, Mode::One(thunk))?;
            let result = emit_cold_result(code, registers, mode, |destination, _| {
                ColdInstruction::MakePromise {
                    destination,
                    thunk,
                    flatten: matches!(expression, CoreExpr::DelayForce(_)),
                }
            })?;
            registers.reset(mark);
            Ok(result)
        }
        CoreExpr::Force(value) => compile_unary_cold(
            value,
            env,
            code,
            registers,
            mode,
            |destination, source, expected| ColdInstruction::Force {
                destination,
                promise: source,
                expected,
            },
        ),
        CoreExpr::WithExceptionHandler { handler, thunk } => {
            let mark = registers.mark();
            let first = registers.acquire_block(2)?;
            compile_expression(handler, env, code, registers, Mode::One(first))?;
            code.emit_cold(ColdInstruction::PushHandler { handler: first })?;
            compile_expression(thunk, env, code, registers, Mode::One(first + 1))?;
            emit_call(code, first + 1, 0, mode);
            code.emit_cold(ColdInstruction::PopHandler)?;
            let result = finish_call_result(code, first + 1, mode)?;
            registers.reset(mark);
            Ok(result)
        }
        CoreExpr::Raise {
            object,
            continuable,
        } => compile_unary_cold(
            object,
            env,
            code,
            registers,
            mode,
            |destination, source, expected| ColdInstruction::Raise {
                destination,
                object: source,
                continuable: *continuable,
                expected,
            },
        ),
        CoreExpr::CallWithCurrentContinuation(procedure) => compile_unary_cold(
            procedure,
            env,
            code,
            registers,
            mode,
            |destination, source, expected| ColdInstruction::CaptureContinuation {
                destination,
                procedure: source,
                expected,
            },
        ),
        CoreExpr::DynamicWind {
            before,
            thunk,
            after,
        } => compile_cold_call(
            [before.as_ref(), thunk.as_ref(), after.as_ref()],
            env,
            code,
            registers,
            mode,
            |destination, inputs, expected| ColdInstruction::DynamicWind {
                destination,
                before: inputs[0],
                thunk: inputs[1],
                after: inputs[2],
                expected,
            },
        ),
        CoreExpr::CallWithPort { port, procedure } => compile_cold_call(
            [port.as_ref(), procedure.as_ref()],
            env,
            code,
            registers,
            mode,
            |destination, inputs, expected| ColdInstruction::CallWithPort {
                destination,
                port: inputs[0],
                procedure: inputs[1],
                expected,
            },
        ),
        CoreExpr::CallWithFile {
            input,
            path,
            procedure,
        } => compile_cold_call(
            [path.as_ref(), procedure.as_ref()],
            env,
            code,
            registers,
            mode,
            |destination, inputs, expected| ColdInstruction::CallWithFile {
                destination,
                path: inputs[0],
                procedure: inputs[1],
                input: *input,
                expected,
            },
        ),
        CoreExpr::WithFile { input, path, thunk } => compile_cold_call(
            [path.as_ref(), thunk.as_ref()],
            env,
            code,
            registers,
            mode,
            |destination, inputs, expected| ColdInstruction::WithFile {
                destination,
                path: inputs[0],
                thunk: inputs[1],
                input: *input,
                expected,
            },
        ),
        CoreExpr::Load { path, environment } => {
            let mark = registers.mark();
            let path_register = registers.acquire()?;
            compile_expression(path, env, code, registers, Mode::One(path_register))?;
            let environment_register = if let Some(environment) = environment {
                let register = registers.acquire()?;
                compile_expression(environment, env, code, registers, Mode::One(register))?;
                Some(register)
            } else {
                None
            };
            let result = emit_cold_result(code, registers, mode, |destination, expected| {
                ColdInstruction::Load {
                    destination,
                    path: path_register,
                    environment: environment_register,
                    expected,
                }
            })?;
            registers.reset(mark);
            Ok(result)
        }
        CoreExpr::Parameterize { bindings, body } => {
            let mark = registers.mark();
            let first = registers.acquire_block(bindings.len() * 2)?;
            for (index, (parameter, value)) in bindings.iter().enumerate() {
                compile_expression(
                    parameter,
                    env,
                    code,
                    registers,
                    Mode::One(first + (index * 2) as u8),
                )?;
                compile_expression(
                    value,
                    env,
                    code,
                    registers,
                    Mode::One(first + (index * 2 + 1) as u8),
                )?;
            }
            code.emit_cold(ColdInstruction::PushParameters {
                first,
                count: bindings.len() as u8,
            })?;
            let destination = output_register(registers, mode)?
                .unwrap_or_else(|| registers.acquire().expect("register"));
            let produced = compile_expression(body, env, code, registers, Mode::All(destination))?;
            code.emit_cold(ColdInstruction::PopParameters {
                count: bindings.len() as u8,
            })?;
            let result = finish_produced(code, destination, produced, mode)?;
            registers.reset(mark);
            Ok(result)
        }
        CoreExpr::MakeParameter { initial, converter } => {
            let mark = registers.mark();
            let initial_register = registers.acquire()?;
            compile_expression(initial, env, code, registers, Mode::One(initial_register))?;
            let converter_register = if let Some(converter) = converter {
                let register = registers.acquire()?;
                compile_expression(converter, env, code, registers, Mode::One(register))?;
                Some(register)
            } else {
                None
            };
            let result = emit_cold_result(code, registers, mode, |destination, _| {
                ColdInstruction::MakeParameter {
                    destination,
                    initial: initial_register,
                    converter: converter_register,
                }
            })?;
            registers.reset(mark);
            Ok(result)
        }
        CoreExpr::Error { message, irritants } => {
            let mark = registers.mark();
            let message_register = registers.acquire()?;
            compile_expression(message, env, code, registers, Mode::One(message_register))?;
            let first = registers.acquire_block(irritants.len())?;
            for (offset, irritant) in irritants.iter().enumerate() {
                compile_expression(
                    irritant,
                    env,
                    code,
                    registers,
                    Mode::One(first + offset as u8),
                )?;
            }
            let destination = registers.acquire()?;
            code.emit_cold(ColdInstruction::MakeError {
                destination,
                message: message_register,
                first_irritant: first,
                count: irritants.len() as u8,
            })?;
            code.emit_cold(ColdInstruction::Raise {
                destination,
                object: destination,
                continuable: false,
                expected: ExpectedResults::All,
            })?;
            // Unreachable: a non-continuable raise never resumes (a handler
            // that returns hits `ReturnAction::RaiseReturned`, which always
            // errors). Emitted so the chunk satisfies the verifier's rule that
            // no instruction can fall off the end of the code.
            code.emit(Word::abc(Opcode::Return, destination, 2, 0, false));
            registers.reset(mark);
            Ok(None)
        }
    }
}

fn register_index(value: usize, message: &str) -> Result<u8, Error> {
    if value < MAX_REGISTERS {
        Ok(value as u8)
    } else {
        Err(compile_error(message))
    }
}

fn compile_error(message: &str) -> Error {
    Error::plain(ErrorKind::CompileError, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compiles one source program (with `(scheme base)` imported) and returns
    /// the entry chunk's opcode sequence. Emission-shape pins below assert the
    /// exact sequence so any emitter change to a hot loop shape is a
    /// deliberate, reviewed test update rather than a silent drift.
    fn program_opcodes(source: &str) -> Vec<Opcode> {
        let mut engine = crate::Engine::new(crate::EngineConfig::default()).unwrap();
        let module = engine
            .compile("emission-test", format!("(import (scheme base)) {source}"))
            .unwrap();
        module
            .entry
            .code
            .iter()
            .map(|word| word.opcode().unwrap())
            .collect()
    }

    #[test]
    fn back_edge_test_fusion_requires_a_materialization_free_header() {
        // The `LoopBackWhileNotEqual` guard anchors on `words[header]` being
        // the `TestEqual` itself, so a fused word can only ever replicate
        // operands that are k-constants or named locals (parameter homes /
        // outer locals). A comparison operand needing materialization - here
        // `(string-length s)` computed into a scratch register every header
        // evaluation - puts that word at `header` and MUST reject fusion:
        // replicating the freed scratch register into the back-edge word
        // would read it after the loop body or argument staging reused it.
        use Opcode::*;
        let swap = program_opcodes(
            "(let ((s (make-string 3 #\\a)))
               (let loop ((a 1) (b 2) (i 0))
                 (if (= i (string-length s)) (list a b) (loop b a (+ i 1)))))",
        );
        assert!(swap.contains(&LoopBack));
        assert!(!swap.contains(&LoopBackWhileNotEqual));

        // Hoisting the length into a named local restores fusion: the operand
        // is now a stable register below the loop base.
        let hoisted = program_opcodes(
            "(let* ((s (make-string 3 #\\a)) (n (string-length s)))
               (let loop ((a 1) (b 2) (i 0))
                 (if (= i n) (list a b) (loop b a (+ i 1)))))",
        );
        assert!(hoisted.contains(&LoopBackWhileNotEqual));
        assert!(!hoisted.contains(&LoopBack));
    }

    #[test]
    fn shadowed_operators_disable_back_edge_and_vector_test_fusion() {
        // Every fused recognizer routes through `fast_operation`, so a local
        // rebinding of the operator falls back to a plain call: no fused
        // back-edge and no TestVectorRef may be emitted.
        use Opcode::*;
        let strided = program_opcodes(
            "(let ((<= (lambda (a b) #f)) (p 3) (limit 20))
               (let loop ((m 9)) (if (<= m limit) (loop (+ m p)) m)))",
        );
        assert!(!strided.contains(&LoopBackStepWhileLessEqual));
        assert!(!strided.contains(&TestLessEqual));
        let vector_test = program_opcodes(
            "(let ((vector-ref (lambda (v i) #t)) (v (vector 1)))
               (if (vector-ref v 0) 1 2))",
        );
        assert!(!vector_test.contains(&TestVectorRef));
    }

    #[test]
    fn emission_shape_counted_accumulate_loop() {
        // Steady state is two dispatches: the in-place accumulate `Add` and the
        // fused back-edge (step + replicated `=` test, specialized for the
        // fixnum constant limit). No staging Moves.
        use Opcode::*;
        assert_eq!(
            program_opcodes("(let loop ((i 0) (a 0)) (if (= i 1000) a (loop (+ i 1) (+ a i))))"),
            [
                LoadK,
                LoadK,
                TestEqualFixnum,
                Jump,
                Return,
                Add,
                LoopBackWhileNotEqualFixnum,
                Jump,
                Jump
            ],
        );
    }

    #[test]
    fn emission_shape_wide_fixnum_loop() {
        // The `(- (+ acc K1) K2)` accumulate collapses into one
        // `AddSubFixnumK` word (both wide literals as fixnum constants, the
        // accumulator read and written in place), plus the fused fixnum-limit
        // back-edge: two dispatches per iteration and no staging Moves.
        use Opcode::*;
        assert_eq!(
            program_opcodes(
                "(let loop ((i 0) (acc 1000000000000000))
                   (if (= i 100000) acc
                       (loop (+ i 1) (- (+ acc 1000000000000000) 999999999999993))))",
            ),
            [
                LoadK,
                LoadK,
                TestEqualFixnum,
                Jump,
                Return,
                AddSubFixnumK,
                LoopBackWhileNotEqualFixnum,
                Jump,
                Jump
            ],
        );
    }

    #[test]
    fn emission_shape_sieve_mark_multiples_loop() {
        // The register-step (`(+ multiple p)`) strided loop with a `<=` guard
        // fuses into `LoopBackStepWhileLessEqual` (step by the register, re-run
        // the guard, branch): four dispatches per iteration (Move, LoadK,
        // VectorSet, back-edge).
        use Opcode::*;
        assert_eq!(
            program_opcodes(
                "(let* ((limit 2000) (prime? (make-vector (+ limit 1) #t)) (p 3))
                   (let mark-multiples ((multiple (* p p)))
                     (if (<= multiple limit)
                         (begin
                           (vector-set! prime? multiple #f)
                           (mark-multiples (+ multiple p))))))",
            ),
            [
                LoadK,
                GetGlobal,
                AddFixnumK,
                LoadK,
                Call,
                Move,
                LoadK,
                Multiply,
                TestLessEqual,
                Jump,
                Move,
                LoadK,
                VectorSet,
                LoopBackStepWhileLessEqual,
                Jump,
                Jump,
                LoadK,
                Return,
            ],
        );
    }

    #[test]
    fn emission_shape_sieve_count_primes_loop() {
        // Step +1 with a `>` guard (swapped TestLess, counter on the right):
        // the back-edge fuses into `LoopBackWhileNotLess` (replicated header
        // test), and the vector-ref condition fuses into `TestVectorRef` -
        // three dispatches per prime iteration, two per non-prime.
        use Opcode::*;
        assert_eq!(
            program_opcodes(
                "(let* ((limit 2000) (prime? (make-vector (+ limit 1) #t)))
                   (let count-primes ((i 2) (count 0))
                     (if (> i limit)
                         count
                         (count-primes (+ i 1)
                                       (if (vector-ref prime? i) (+ count 1) count)))))",
            ),
            [
                LoadK,
                GetGlobal,
                AddFixnumK,
                LoadK,
                Call,
                Move,
                LoadK,
                LoadK,
                TestLess,
                Jump,
                Return,
                TestVectorRef,
                Jump,
                AddFixnumK,
                LoopBackWhileNotLess,
                Jump,
                Jump,
            ],
        );
    }

    #[test]
    fn emission_shape_mandelbrot_iterate_loop() {
        // Pins the float-chain fusion state: AddMul/SubMul fire for the
        // `(- (* zr zr) (* zi zi))` shapes, `(* 2.0 zr zi)` folds its constant
        // first operand into the first Multiply's RK slot (no LoadK in the
        // loop), and the zr/zi parallel assignment costs one Move (a genuine
        // two-cycle). The `(> ... 4.0)` escape test keeps its own opcode
        // (`TestGreater`), so 4.0 folds into the k slot with no per-iteration
        // LoadK.
        use Opcode::*;
        assert_eq!(
            program_opcodes(
                "(let ((cr -0.5) (ci 0.25) (max-iterations 40))
                   (let iterate ((zr 0.0) (zi 0.0) (iteration 0))
                     (if (or (= iteration max-iterations)
                             (> (+ (* zr zr) (* zi zi)) 4.0))
                         iteration
                         (iterate (+ (- (* zr zr) (* zi zi)) cr)
                                  (+ (* 2.0 zr zi) ci)
                                  (+ iteration 1)))))",
            ),
            [
                LoadK,
                LoadK,
                LoadK,
                LoadK,
                LoadK,
                LoadK,
                TestEqual,
                Jump,
                Multiply,
                AddMul,
                TestGreater,
                Jump,
                Return,
                Multiply,
                SubMul,
                Add,
                Multiply,
                Multiply,
                Add,
                Move,
                LoopBack,
                Jump,
            ],
        );
    }

    #[test]
    fn emission_shape_float_dot_loop() {
        // The dot-product accumulate `(+ sum (* (vector-ref v i) (vector-ref v i)))`
        // with all four inner operands home-readable fuses both element
        // fetches into one `AddMulVectorRef` (plus its consumed `ExtraArg`):
        // two dispatches per iteration with the fused back-edge.
        use Opcode::*;
        assert_eq!(
            program_opcodes(
                "(let* ((n 1000) (v (make-vector n 0.0)))
                   (let loop ((i 0) (sum 0.0))
                     (if (= i n)
                         sum
                         (loop (+ i 1) (+ sum (* (vector-ref v i) (vector-ref v i)))))))",
            ),
            [
                LoadK,
                GetGlobal,
                Move,
                LoadK,
                Call,
                Move,
                LoadK,
                LoadK,
                TestEqual,
                Jump,
                Return,
                AddMulVectorRef,
                ExtraArg,
                LoopBackWhileNotEqual,
                Jump,
                Jump,
            ],
        );
    }

    #[test]
    fn emission_shape_matrix_inner_loop() {
        // The matrix-multiply inner loop: the row fetch stays a standalone
        // `VectorRef`, the chained `(vector-ref (vector-ref b k) j)` fuses
        // into `VectorRefVectorRef` (outer index in the consumed `ExtraArg`),
        // and the multiply-accumulate fuses into `AddMul`: four dispatches per
        // iteration.
        use Opcode::*;
        assert_eq!(
            program_opcodes(
                "(let ((n 15) (j 3)
                       (row-a (make-vector 15 1))
                       (b (make-vector 15 0)))
                   (let loop-k ((k 0) (sum 0))
                     (if (= k n)
                         sum
                         (loop-k (+ k 1)
                                 (+ sum (* (vector-ref row-a k)
                                           (vector-ref (vector-ref b k) j)))))))",
            ),
            [
                LoadK,
                LoadK,
                GetGlobal,
                LoadK,
                LoadK,
                Call,
                Move,
                GetGlobal,
                LoadK,
                LoadK,
                Call,
                Move,
                LoadK,
                LoadK,
                TestEqual,
                Jump,
                Return,
                VectorRef,
                VectorRefVectorRef,
                ExtraArg,
                AddMul,
                LoopBackWhileNotEqual,
                Jump,
                Jump,
            ],
        );
    }

    fn lambda(params: &[&str], body: CoreExpr) -> CoreExpr {
        CoreExpr::Lambda {
            params: params.iter().map(|name| (*name).to_string()).collect(),
            body: Box::new(body),
        }
    }

    fn contains_opcode(chunk: &Chunk, opcode: Opcode) -> bool {
        chunk
            .code
            .iter()
            .any(|word| word.opcode().unwrap() == opcode)
    }

    #[test]
    fn immutable_captures_compile_by_value() {
        // (lambda (n) (lambda (x) n)): `n` is captured and never mutated, so it
        // must stay an unboxed register and be read via `GetCaptureValue`.
        let expr = lambda(&["n"], lambda(&["x"], CoreExpr::Variable("n".into())));
        let module = compile(&expr, &Limits::default()).unwrap();
        let outer = &module.entry.closures[0].chunk;
        let inner = &outer.closures[0].chunk;
        assert!(outer.boxed_locals.is_empty());
        assert_eq!(inner.capture_kinds.as_ref(), [CaptureKind::Value]);
        assert!(contains_opcode(inner, Opcode::GetCaptureValue));
        assert!(!contains_opcode(inner, Opcode::GetCapture));
    }

    #[test]
    fn mutated_captures_stay_heap_cells() {
        // (lambda (n) (lambda () (set! n n))): the nested mutation forces `n`
        // into a boxed local at its origin and a cell capture in the closure.
        let expr = lambda(
            &["n"],
            lambda(
                &[],
                CoreExpr::Set {
                    name: "n".into(),
                    value: Box::new(CoreExpr::Variable("n".into())),
                },
            ),
        );
        let module = compile(&expr, &Limits::default()).unwrap();
        let outer = &module.entry.closures[0].chunk;
        let inner = &outer.closures[0].chunk;
        assert_eq!(outer.boxed_locals.as_ref(), [0]);
        assert_eq!(inner.capture_kinds.as_ref(), [CaptureKind::Cell]);
        assert!(contains_opcode(inner, Opcode::SetCapture));
        assert!(!contains_opcode(inner, Opcode::GetCaptureValue));
    }

    #[test]
    fn chained_captures_propagate_the_origin_kind() {
        // (lambda (n) (lambda () (lambda () n))): the innermost read reaches
        // `n` through a middle chunk, and every link must stay `Value`.
        let expr = lambda(
            &["n"],
            lambda(&[], lambda(&[], CoreExpr::Variable("n".into()))),
        );
        let module = compile(&expr, &Limits::default()).unwrap();
        let outer = &module.entry.closures[0].chunk;
        let middle = &outer.closures[0].chunk;
        let inner = &middle.closures[0].chunk;
        assert_eq!(middle.capture_kinds.as_ref(), [CaptureKind::Value]);
        assert_eq!(inner.capture_kinds.as_ref(), [CaptureKind::Value]);
        assert!(matches!(
            middle.closures[0].captures.as_slice(),
            [CaptureSource::Capture(0)]
        ));
        assert!(contains_opcode(inner, Opcode::GetCaptureValue));
    }
}
