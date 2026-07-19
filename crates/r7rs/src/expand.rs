//! Hygienic-ish source expansion for the currently implemented core language.
//!
//! Reader datums remain compiler-owned. This module turns their acyclic code
//! view into [`crate::CoreExpr`] and keeps macro bindings local to one source
//! compilation unit.

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::{Datum, DatumKind, Error, ErrorKind, Limits, Number, Real, Span, Value};

mod binding;
mod control;
mod helpers;
mod lower;
mod macros;

#[derive(Clone, Debug)]
pub(crate) struct Form {
    pub(crate) kind: FormKind,
    pub(crate) span: Span,
}

#[derive(Clone, Debug)]
pub(crate) enum FormKind {
    Nil,
    Bool(bool),
    Char(char),
    String(String),
    Number(Number),
    Symbol(String),
    List(Vec<Form>, Option<Box<Form>>),
    Vector(Vec<Form>),
    Bytevector(Vec<u8>),
}

impl Form {
    pub(crate) fn symbol(&self) -> Option<&str> {
        if let FormKind::Symbol(s) = &self.kind {
            Some(s.strip_prefix("#syntax#").unwrap_or(s))
        } else {
            None
        }
    }
    pub(crate) fn proper_list(&self) -> Option<&[Form]> {
        if let FormKind::List(v, None) = &self.kind {
            Some(v)
        } else {
            None
        }
    }
}

/// Expands forms using the engine's configured feature identifiers.
pub(crate) fn expand_forms_with_features(
    forms: &[Form],
    limits: &Limits,
    imported_values: HashMap<String, String>,
    features: &crate::FeatureSet,
) -> Result<crate::CoreExpr, Error> {
    Ok(
        expand_forms_with_imports(forms, limits, imported_values, HashMap::new(), features)?
            .expression,
    )
}

/// Expanded code together with syntax bindings defined by the unit.
pub(crate) struct ExpansionOutput {
    pub(crate) expression: crate::CoreExpr,
    pub(crate) macros: HashMap<String, Macro>,
}

/// Expands source with imported value and syntax bindings.
pub(crate) fn expand_forms_with_imports(
    forms: &[Form],
    limits: &Limits,
    imported_values: HashMap<String, String>,
    imported_macros: HashMap<String, Macro>,
    features: &crate::FeatureSet,
) -> Result<ExpansionOutput, Error> {
    expand_forms_with_imports_and_mutable(
        forms,
        limits,
        imported_values,
        imported_macros,
        features,
        &HashSet::new(),
    )
}

pub(crate) fn expand_forms_with_imports_and_mutable(
    forms: &[Form],
    limits: &Limits,
    imported_values: HashMap<String, String>,
    imported_macros: HashMap<String, Macro>,
    features: &crate::FeatureSet,
    mutable_values: &HashSet<String>,
) -> Result<ExpansionOutput, Error> {
    let imported_syntax = imported_macros
        .iter()
        .map(|(name, transformer)| (name.clone(), transformer.binding.clone()))
        .collect::<HashMap<_, _>>();
    let mut macros = imported_macros;
    for transformer in macros.values().cloned().collect::<Vec<_>>() {
        macros
            .entry(transformer.binding.clone())
            .or_insert(transformer);
    }
    let mut expander = Expander {
        macros,
        limits,
        steps: 0,
        expansion_depth: 0,
        gensym: 0,
        features,
        immutable_imports: imported_values
            .values()
            .filter(|value| !mutable_values.contains(*value))
            .cloned()
            .collect(),
        hoisted: Vec::new(),
        hoisted_names: HashSet::new(),
    };
    let mut env = Env::default();
    if !imported_values.is_empty() {
        env.vars.push(imported_values);
    }
    if !imported_syntax.is_empty() {
        env.vars.push(imported_syntax);
    }
    let expression = expander.body(forms, &mut env)?;
    // Hidden literal definitions run before the unit body, so every hoisted
    // reference below reads an initialized global.
    let expression = if expander.hoisted.is_empty() {
        expression
    } else {
        let mut sequence = Vec::with_capacity(expander.hoisted.len() + 1);
        for (name, value) in expander.hoisted.drain(..) {
            sequence.push(crate::CoreExpr::Define {
                name,
                value: Box::new(value),
            });
        }
        sequence.push(expression);
        crate::CoreExpr::Begin(sequence)
    };
    Ok(ExpansionOutput {
        expression,
        macros: expander.macros,
    })
}

/// Converts reader datums into the expansion representation without expanding
/// them. Library parsing uses this to validate declaration grammar while
/// preserving source spans for diagnostics.
pub(crate) fn forms(datums: &[Datum]) -> Result<Vec<Form>, Error> {
    datums
        .iter()
        .map(|datum| convert(datum, datum.root(), &mut HashSet::new()))
        .collect()
}

fn convert(
    datum: &Datum,
    reference: crate::DatumRef,
    active: &mut HashSet<crate::DatumRef>,
) -> Result<Form, Error> {
    let resolved = datum
        .resolve(reference)
        .ok_or_else(|| Error::plain(ErrorKind::ExpandError, "invalid datum graph"))?;
    let span = datum
        .span(reference)
        .ok_or_else(|| Error::plain(ErrorKind::ExpandError, "invalid datum reference"))?;
    if !active.insert(resolved) {
        return Err(Error::plain(
            ErrorKind::ExpandError,
            "cyclic datum is valid only as data and is not yet supported by source compilation",
        ));
    }
    let kind = match datum
        .kind(reference)
        .ok_or_else(|| Error::plain(ErrorKind::ExpandError, "invalid datum graph"))?
    {
        DatumKind::Nil => FormKind::Nil,
        DatumKind::Boolean(v) => FormKind::Bool(v),
        DatumKind::Character(v) => FormKind::Char(v),
        DatumKind::String(v) => FormKind::String(v.to_owned()),
        DatumKind::Symbol(v) => FormKind::Symbol(v.to_owned()),
        DatumKind::Number(v) => FormKind::Number(*v),
        DatumKind::Bytevector(v) => FormKind::Bytevector(v.to_vec()),
        DatumKind::Vector(v) => FormKind::Vector(
            v.iter()
                .map(|x| convert(datum, *x, active))
                .collect::<Result<_, _>>()?,
        ),
        DatumKind::Pair { car, cdr } => {
            let mut values = vec![convert(datum, car, active)?];
            let mut tail = cdr;
            let mut spine = Vec::new();
            let kind = loop {
                match datum
                    .kind(tail)
                    .ok_or_else(|| Error::plain(ErrorKind::ExpandError, "invalid list"))?
                {
                    DatumKind::Nil => break FormKind::List(values, None),
                    DatumKind::Pair { car, cdr } => {
                        let resolved_tail = datum
                            .resolve(tail)
                            .ok_or_else(|| Error::plain(ErrorKind::ExpandError, "invalid list"))?;
                        if !active.insert(resolved_tail) {
                            return Err(Error::plain(
                                ErrorKind::ExpandError,
                                "cyclic datum is valid only as data and is not yet supported by source compilation",
                            ));
                        }
                        spine.push(resolved_tail);
                        values.push(convert(datum, car, active)?);
                        tail = cdr;
                    }
                    _ => {
                        break FormKind::List(
                            values,
                            Some(Box::new(convert(datum, tail, active)?)),
                        );
                    }
                }
            };
            for reference in spine {
                active.remove(&reference);
            }
            kind
        }
    };
    active.remove(&resolved);
    Ok(Form { kind, span })
}

/// Recovers the underlying identifier from a hygiene-tagged name. Template
/// expansion tags an introduced identifier as `#resolved#<mark>#<name>`, where
/// the per-expansion `<mark>` makes identifiers introduced by distinct
/// expansions distinct, and tags a forced keyword reference as `#syntax#<name>`.
/// Both tags are stripped here to recover the resolvable identifier. The mark is
/// decimal digits, so the first `#` after the prefix delimits it. `<name>` may
/// itself contain `#` (for example a renamed local), which stays intact.
fn strip_hygiene(name: &str) -> &str {
    if let Some(rest) = name.strip_prefix("#resolved#") {
        match rest.find('#') {
            Some(index) => &rest[index + 1..],
            None => rest,
        }
    } else if let Some(rest) = name.strip_prefix("#syntax#") {
        rest
    } else {
        name
    }
}

#[derive(Default, Clone, Debug)]
struct Env {
    vars: Vec<HashMap<String, String>>,
}

impl Env {
    fn resolve(&self, name: &str) -> String {
        let resolved = self.vars.iter().rev().find_map(|m| m.get(name)).cloned();
        resolved.unwrap_or_else(|| strip_hygiene(name).to_owned())
    }
    fn child(&self, binds: HashMap<String, String>) -> Self {
        let mut out = self.clone();
        out.vars.push(binds);
        out
    }

    fn keyword_shadowed(&self, name: &str) -> bool {
        self.vars
            .iter()
            .rev()
            .find_map(|bindings| bindings.get(name))
            .is_some_and(|resolved| {
                resolved.starts_with('#') && !resolved.starts_with("#syntax-binding:")
            })
    }

    fn bind_syntax(&mut self, name: String, binding: String) {
        if let Some(scope) = self.vars.last_mut() {
            scope.insert(name, binding);
        } else {
            self.vars.push(HashMap::from([(name, binding)]));
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Macro {
    literals: HashSet<String>,
    rules: Vec<Rule>,
    definition_env: Env,
    ellipsis: String,
    binding: String,
}

#[derive(Clone, Debug)]
struct Rule {
    pattern: Rc<Form>,
    template: Rc<Form>,
}

#[derive(Clone)]
enum Capture {
    One(Rc<Form>),
    Many(Vec<Capture>),
}

struct Expander<'a> {
    macros: HashMap<String, Macro>,
    limits: &'a Limits,
    steps: usize,
    expansion_depth: usize,
    gensym: u64,
    features: &'a crate::FeatureSet,
    immutable_imports: HashSet<String>,
    /// Hidden content-named definitions for hoisted heap literals, emitted
    /// once at the front of the expanded unit in first-appearance order.
    hoisted: Vec<(String, crate::CoreExpr)>,
    /// Names already in `hoisted`, so one unit defines each literal once.
    hoisted_names: HashSet<String>,
}

impl Expander<'_> {
    fn error(&self, span: Span, message: impl Into<String>) -> Error {
        Error::from_diagnostic(
            crate::Diagnostic::new(ErrorKind::ExpandError, message).with_label(
                span,
                crate::LabelStyle::Primary,
                "here",
            ),
        )
    }

    fn tick(&mut self, span: Span) -> Result<(), Error> {
        self.steps += 1;
        if self.steps > self.limits.max_expansion_steps() {
            Err(Error::from_diagnostic(
                crate::Diagnostic::new(
                    ErrorKind::ExpansionLimitExceeded,
                    "macro expansion step limit exceeded",
                )
                .with_label(
                    span,
                    crate::LabelStyle::Primary,
                    "expansion stopped here",
                ),
            ))
        } else {
            Ok(())
        }
    }

    fn fresh(&mut self, name: &str) -> String {
        self.gensym += 1;
        format!("#{}${}", strip_hygiene(name), self.gensym)
    }

    /// Allocates a mark that is unique to a single macro expansion. Introduced
    /// template identifiers carry it so that identifiers introduced by distinct
    /// expansions become distinct strings while staying identical within one
    /// expansion.
    fn mark(&mut self) -> u64 {
        self.gensym += 1;
        self.gensym
    }
}
