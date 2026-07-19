//! The deliberately small, already-expanded language lowered into bytecode by
//! the compiler.

use crate::{Number, Value};

/// An already-expanded core Scheme expression.
///
/// This type is intentionally independent from reader datums and syntax
/// objects. The hygienic expander lowers into it.
#[derive(Clone, Debug)]
pub enum CoreExpr {
    /// An immediate literal value.
    Literal(Value),
    /// A portable numeric literal materialized by the VM at execution time.
    NumberLiteral(Number),
    /// A lexical or global variable reference.
    Variable(String),
    /// A conditional; only `#f` selects the alternative.
    If(Box<Self>, Box<Self>, Box<Self>),
    /// A sequence whose result is the final expression's result.
    Begin(Vec<Self>),
    /// A fixed-arity procedure.
    Lambda {
        /// Parameter names in argument order.
        params: Vec<String>,
        /// The procedure body.
        body: Box<Self>,
    },
    /// A procedure with required arguments followed by a rest list.
    LambdaRest {
        /// Required parameter names.
        required: Vec<String>,
        /// Rest-list parameter name.
        rest: String,
        /// Procedure body.
        body: Box<Self>,
    },
    /// An ordered set of procedure clauses selected by arity.
    CaseLambda {
        /// Clause expressions, each a `Lambda` or `LambdaRest`.
        clauses: Vec<Self>,
    },
    /// Mutates an existing lexical or global binding.
    Set {
        /// Binding name.
        name: String,
        /// Replacement expression.
        value: Box<Self>,
    },
    /// Defines or replaces a global binding.
    Define {
        /// Global binding name.
        name: String,
        /// Value expression.
        value: Box<Self>,
    },
    /// Calls a procedure.
    Call {
        /// Procedure expression.
        procedure: Box<Self>,
        /// Argument expressions in evaluation order.
        arguments: Vec<Self>,
    },
    /// A self-recursive named loop (`named let`, and the desugaring target of
    /// `do`). When `name` does not escape and every self-call is a tail call the
    /// compiler flattens it into a register loop in the enclosing frame;
    /// otherwise it lowers exactly like the equivalent
    /// `((lambda (name) (begin (set! name (lambda (params) body)) (name inits))) #!unspecified)`.
    NamedLet {
        /// The loop's self-reference binding.
        name: String,
        /// Loop-variable names in argument order.
        params: Vec<String>,
        /// Initial value expressions, evaluated in the enclosing scope.
        inits: Vec<Self>,
        /// The loop body.
        body: Box<Self>,
    },
    /// Delivers zero or more values to the current continuation.
    Values(Vec<Self>),
    /// Calls a zero-argument producer and passes all of its values to a consumer.
    CallWithValues {
        /// The zero-argument producer.
        producer: Box<Self>,
        /// The procedure receiving the producer's values.
        consumer: Box<Self>,
    },
    /// Creates a delayed computation.
    Delay(Box<Self>),
    /// Creates a delayed computation whose result is forced in tail position.
    DelayForce(Box<Self>),
    /// Forces a delayed computation.
    Force(Box<Self>),
    /// Runs a thunk with an exception handler in the dynamic environment.
    WithExceptionHandler {
        /// The one-argument handler procedure.
        handler: Box<Self>,
        /// The zero-argument protected procedure.
        thunk: Box<Self>,
    },
    /// Raises a guest exception.
    Raise {
        /// The object supplied to the current handler.
        object: Box<Self>,
        /// Whether a handler return is permitted.
        continuable: bool,
    },
    /// Invokes a procedure with the current continuation.
    CallWithCurrentContinuation(Box<Self>),
    /// Runs a thunk between dynamic-entry and dynamic-exit procedures.
    DynamicWind {
        /// Called before entering the dynamic extent.
        before: Box<Self>,
        /// The protected computation.
        thunk: Box<Self>,
        /// Called after leaving the dynamic extent.
        after: Box<Self>,
    },
    /// Calls a procedure with a port and closes it only when that procedure
    /// returns normally. This is a VM primitive because continuations may
    /// leave and later re-enter the call's dynamic extent.
    CallWithPort {
        /// The port passed to the procedure.
        port: Box<Self>,
        /// The one-argument procedure.
        procedure: Box<Self>,
    },
    /// Opens a textual file, invokes a one-argument procedure, and closes the
    /// resulting port on normal return.
    CallWithFile {
        /// Whether the file is opened for input rather than output.
        input: bool,
        /// Expression producing the path string.
        path: Box<Self>,
        /// The one-argument procedure.
        procedure: Box<Self>,
    },
    /// Dynamically redirects a current textual port while invoking a thunk.
    WithFile {
        /// Whether the current input rather than output port is rebound.
        input: bool,
        /// Expression producing the path string.
        path: Box<Self>,
        /// The zero-argument procedure.
        thunk: Box<Self>,
    },
    /// Loads and evaluates Scheme source in an optional runtime environment.
    Load {
        /// Expression producing the source request string.
        path: Box<Self>,
        /// Explicit target environment, or the interaction environment.
        environment: Option<Box<Self>>,
    },
    /// Temporarily changes parameter values for a body.
    Parameterize {
        /// Parameter/value expressions in binding order.
        bindings: Vec<(Self, Self)>,
        /// The dynamically scoped body.
        body: Box<Self>,
    },
    /// Creates a parameter, optionally using a converter.
    MakeParameter {
        /// Initial input to the converter.
        initial: Box<Self>,
        /// Optional one-argument conversion procedure.
        converter: Option<Box<Self>>,
    },
    /// Creates and raises a standard Scheme error object.
    Error {
        /// Message expression.
        message: Box<Self>,
        /// Irritant expressions.
        irritants: Vec<Self>,
    },
}

impl CoreExpr {
    /// Builds an immediate literal expression.
    #[must_use]
    pub const fn literal(value: Value) -> Self {
        Self::Literal(value)
    }
    /// Builds a variable-reference expression.
    #[must_use]
    pub fn variable(name: impl Into<String>) -> Self {
        Self::Variable(name.into())
    }
}
