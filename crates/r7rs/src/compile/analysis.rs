//! Static capture/mutation analysis over `CoreExpr`: how lambda parameters are
//! used (mutated, captured), and whether a variable is mentioned within an
//! expression.

use super::{
    expr::{can_inline_application, classify_named_let},
    *,
};

/// Conservatively reports whether `name` occurs as a variable reference anywhere
/// in `expr`. Over-approximates (ignores shadowing), which can only suppress an
/// optimization, never enable an unsound one.
pub(super) fn mentions_variable(expr: &CoreExpr, name: &str) -> bool {
    match expr {
        CoreExpr::Literal(_) | CoreExpr::NumberLiteral(_) => false,
        CoreExpr::Variable(value) => value == name,
        CoreExpr::If(a, b, c) => {
            mentions_variable(a, name) || mentions_variable(b, name) || mentions_variable(c, name)
        }
        CoreExpr::Begin(items) | CoreExpr::Values(items) => {
            items.iter().any(|item| mentions_variable(item, name))
        }
        CoreExpr::Lambda { body, .. } | CoreExpr::LambdaRest { body, .. } => {
            mentions_variable(body, name)
        }
        CoreExpr::CaseLambda { clauses } => {
            clauses.iter().any(|clause| mentions_variable(clause, name))
        }
        CoreExpr::Set { value, .. } | CoreExpr::Define { value, .. } => {
            mentions_variable(value, name)
        }
        CoreExpr::Call {
            procedure,
            arguments,
        } => {
            mentions_variable(procedure, name)
                || arguments.iter().any(|a| mentions_variable(a, name))
        }
        CoreExpr::CallWithValues { producer, consumer } => {
            mentions_variable(producer, name) || mentions_variable(consumer, name)
        }
        CoreExpr::NamedLet { inits, body, .. } => {
            inits.iter().any(|init| mentions_variable(init, name)) || mentions_variable(body, name)
        }
        CoreExpr::Delay(e)
        | CoreExpr::DelayForce(e)
        | CoreExpr::Force(e)
        | CoreExpr::CallWithCurrentContinuation(e) => mentions_variable(e, name),
        CoreExpr::WithExceptionHandler { handler, thunk } => {
            mentions_variable(handler, name) || mentions_variable(thunk, name)
        }
        CoreExpr::Raise { object, .. } => mentions_variable(object, name),
        CoreExpr::DynamicWind {
            before,
            thunk,
            after,
        } => {
            mentions_variable(before, name)
                || mentions_variable(thunk, name)
                || mentions_variable(after, name)
        }
        CoreExpr::CallWithPort { port, procedure } => {
            mentions_variable(port, name) || mentions_variable(procedure, name)
        }
        CoreExpr::CallWithFile {
            path, procedure, ..
        } => mentions_variable(path, name) || mentions_variable(procedure, name),
        CoreExpr::WithFile { path, thunk, .. } => {
            mentions_variable(path, name) || mentions_variable(thunk, name)
        }
        CoreExpr::Load { path, environment } => {
            mentions_variable(path, name)
                || environment
                    .as_ref()
                    .is_some_and(|environment| mentions_variable(environment, name))
        }
        CoreExpr::Parameterize { bindings, body } => {
            bindings
                .iter()
                .any(|(p, v)| mentions_variable(p, name) || mentions_variable(v, name))
                || mentions_variable(body, name)
        }
        CoreExpr::MakeParameter { initial, converter } => {
            mentions_variable(initial, name)
                || converter
                    .as_ref()
                    .is_some_and(|converter| mentions_variable(converter, name))
        }
        CoreExpr::Error { message, irritants } => {
            mentions_variable(message, name) || irritants.iter().any(|i| mentions_variable(i, name))
        }
    }
}

/// How a lambda's parameters are used by its body: which are `set!`-mutated
/// anywhere (directly or from a nested lambda) and which are referenced from a
/// nested lambda (captured). A mutated parameter needs a heap `Box` cell so
/// closures and continuations share one mutable location. A captured parameter
/// that is never mutated is immutable for its whole extent, so closures may
/// capture its raw value instead of a cell.
pub(super) struct ParameterUsage {
    /// Parameters `set!`-mutated anywhere in the body, whether the `set!` sits
    /// directly in the body or inside a nested lambda.
    pub(super) mutated: HashSet<u8>,
    /// Parameters referenced (read or written) from inside a nested lambda.
    pub(super) captured: HashSet<u8>,
}

impl ParameterUsage {
    pub(super) fn is_empty(&self) -> bool {
        self.mutated.is_empty() && self.captured.is_empty()
    }
}

/// Determines how a lambda's parameters are used (see [`ParameterUsage`]).
/// Shadowing by inner binders is respected, so an inner parameter of the same
/// name does not count.
///
/// This mirrors the compiler's inlining: an immediately-applied lambda that
/// `compile_call` inlines is not a capture boundary, so variables referenced
/// only by it are not captured. The same predicate (`can_inline_application`)
/// gates both this scan and the compiler, so they agree.
pub(super) fn parameter_usage<'src>(
    params: &[&'src String],
    body: &'src CoreExpr,
) -> ParameterUsage {
    let mut usage = ParameterUsage {
        mutated: HashSet::new(),
        captured: HashSet::new(),
    };
    let visible = params
        .iter()
        .enumerate()
        .map(|(index, name)| (name.as_str(), index as u8))
        .collect::<HashMap<&'src str, u8>>();
    // A frame body begins in tail (`Mode::Return`) position, which is not the
    // multiple-values `Mode::All` position that suppresses inlining.
    scan_usage(body, &visible, false, false, &mut usage);
    usage
}

/// `all_pos` tracks whether `node` sits in a `Mode::All` (multiple-values)
/// position, which reaches a sub-expression only from a `parameterize` body
/// (see `compile_expression`) and then propagates through tail `Begin`/`If`.
/// It must agree with `compile_call`: an immediately-applied lambda is inlined
/// (so its body does not capture the enclosing frame's variables) exactly when
/// it is inlinable and not in an `All` position. Getting `all_pos` wrong in the
/// false direction would leave a captured local misclassified, so it
/// over-approximates nothing. It mirrors the compiler's `Mode::All` propagation
/// exactly.
fn scan_usage<'src>(
    node: &'src CoreExpr,
    visible: &HashMap<&'src str, u8>,
    nested: bool,
    all_pos: bool,
    usage: &mut ParameterUsage,
) {
    match node {
        CoreExpr::Literal(_) | CoreExpr::NumberLiteral(_) => {}
        CoreExpr::Variable(name) => {
            // A read is a capture only when it escapes into a nested lambda.
            if nested && let Some(&index) = visible.get(name.as_str()) {
                usage.captured.insert(index);
            }
        }
        CoreExpr::Set { name, value } => {
            // Any mutation of this frame's parameter, whether the `set!` sits
            // directly in the body or inside a nested lambda.
            if let Some(&index) = visible.get(name.as_str()) {
                usage.mutated.insert(index);
                if nested {
                    usage.captured.insert(index);
                }
            }
            scan_usage(value, visible, nested, false, usage);
        }
        CoreExpr::Lambda { params, body } => {
            scan_nested(body, visible, params.iter(), usage);
        }
        CoreExpr::LambdaRest {
            required,
            rest,
            body,
        } => {
            scan_nested(
                body,
                visible,
                required.iter().chain(std::iter::once(rest)),
                usage,
            );
        }
        CoreExpr::CaseLambda { clauses } => {
            for clause in clauses {
                scan_usage(clause, visible, nested, false, usage);
            }
        }
        CoreExpr::If(test, consequent, alternate) => {
            scan_usage(test, visible, nested, false, usage);
            // The branches inherit the `if`'s own position (they are its tails).
            scan_usage(consequent, visible, nested, all_pos, usage);
            scan_usage(alternate, visible, nested, all_pos, usage);
        }
        CoreExpr::Begin(items) => {
            if let Some((last, leading)) = items.split_last() {
                for item in leading {
                    scan_usage(item, visible, nested, false, usage);
                }
                // Only the final expression inherits the `begin`'s position.
                scan_usage(last, visible, nested, all_pos, usage);
            }
        }
        CoreExpr::Values(items) => {
            // Each value is produced in a single-value slot (`Mode::One`).
            for item in items {
                scan_usage(item, visible, nested, false, usage);
            }
        }
        CoreExpr::Define { value, .. } => scan_usage(value, visible, nested, false, usage),
        CoreExpr::NamedLet {
            name,
            params,
            inits,
            body,
        } => {
            for init in inits {
                scan_usage(init, visible, nested, false, usage);
            }
            // Must mirror `compile_named_let` exactly (the same `all_pos` gate as
            // inlining): when the loop flattens, its body is compiled in place,
            // so scan it in-frame with the loop's bindings shadowed (the
            // enclosing free variables it uses stay direct locals). Otherwise
            // the body is a closure boundary and its free enclosing variables
            // are captured.
            if !all_pos && classify_named_let(name, params, body) {
                let mut inner = visible.clone();
                for parameter in params {
                    inner.remove(parameter.as_str());
                }
                inner.remove(name.as_str());
                scan_usage(body, &inner, nested, false, usage);
            } else {
                scan_nested(
                    body,
                    visible,
                    params.iter().chain(std::iter::once(name)),
                    usage,
                );
            }
        }
        CoreExpr::Call {
            procedure,
            arguments,
        } => {
            if let CoreExpr::Lambda { params, body } = procedure.as_ref()
                && !all_pos
                && can_inline_application(params, arguments)
            {
                // `compile_call` inlines this application into the current frame,
                // so its body is not a capture boundary: scan it in place with
                // the inlined parameters shadowing the frame's, keeping `nested`.
                for argument in arguments {
                    scan_usage(argument, visible, nested, false, usage);
                }
                let mut inner = visible.clone();
                for name in params {
                    inner.remove(name.as_str());
                }
                scan_usage(body, &inner, nested, false, usage);
            } else {
                scan_usage(procedure, visible, nested, false, usage);
                for argument in arguments {
                    scan_usage(argument, visible, nested, false, usage);
                }
            }
        }
        CoreExpr::CallWithValues { producer, consumer } => {
            scan_usage(producer, visible, nested, false, usage);
            scan_usage(consumer, visible, nested, false, usage);
        }
        CoreExpr::Delay(value) | CoreExpr::DelayForce(value) => {
            // `delay`/`delay-force` wrap their expression in an implicit
            // zero-argument thunk, so any enclosing parameter referenced inside
            // is captured (and a `set!` inside marks it mutated).
            scan_usage(value, visible, true, false, usage);
        }
        CoreExpr::Force(value) | CoreExpr::CallWithCurrentContinuation(value) => {
            scan_usage(value, visible, nested, false, usage);
        }
        CoreExpr::WithExceptionHandler { handler, thunk } => {
            scan_usage(handler, visible, nested, false, usage);
            scan_usage(thunk, visible, nested, false, usage);
        }
        CoreExpr::Raise { object, .. } => scan_usage(object, visible, nested, false, usage),
        CoreExpr::DynamicWind {
            before,
            thunk,
            after,
        } => {
            scan_usage(before, visible, nested, false, usage);
            scan_usage(thunk, visible, nested, false, usage);
            scan_usage(after, visible, nested, false, usage);
        }
        CoreExpr::CallWithPort { port, procedure } => {
            scan_usage(port, visible, nested, false, usage);
            scan_usage(procedure, visible, nested, false, usage);
        }
        CoreExpr::CallWithFile {
            path, procedure, ..
        } => {
            scan_usage(path, visible, nested, false, usage);
            scan_usage(procedure, visible, nested, false, usage);
        }
        CoreExpr::WithFile { path, thunk, .. } => {
            scan_usage(path, visible, nested, false, usage);
            scan_usage(thunk, visible, nested, false, usage);
        }
        CoreExpr::Load { path, environment } => {
            scan_usage(path, visible, nested, false, usage);
            if let Some(environment) = environment {
                scan_usage(environment, visible, nested, false, usage);
            }
        }
        CoreExpr::Parameterize { bindings, body } => {
            for (parameter, value) in bindings {
                scan_usage(parameter, visible, nested, false, usage);
                scan_usage(value, visible, nested, false, usage);
            }
            // The body is compiled in `Mode::All` (see `compile_expression`).
            scan_usage(body, visible, nested, true, usage);
        }
        CoreExpr::MakeParameter { initial, converter } => {
            scan_usage(initial, visible, nested, false, usage);
            if let Some(converter) = converter {
                scan_usage(converter, visible, nested, false, usage);
            }
        }
        CoreExpr::Error { message, irritants } => {
            scan_usage(message, visible, nested, false, usage);
            for irritant in irritants {
                scan_usage(irritant, visible, nested, false, usage);
            }
        }
    }
}

/// Scans a nested lambda body with the enclosing frame's parameters that its
/// binders do not shadow, recording captures and mutations. The nested
/// body is a fresh frame compiled in tail position, so it is never in the
/// enclosing `Mode::All` position.
fn scan_nested<'src>(
    body: &'src CoreExpr,
    visible: &HashMap<&'src str, u8>,
    binders: impl Iterator<Item = &'src String>,
    usage: &mut ParameterUsage,
) {
    let mut inner = visible.clone();
    for binder in binders {
        inner.remove(binder.as_str());
    }
    scan_usage(body, &inner, true, false, usage);
}
