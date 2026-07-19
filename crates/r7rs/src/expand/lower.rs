//! Core lowering: body and internal-definition handling, the special-form
//! dispatcher, and the quote/feature/eval forms.

use super::{helpers::*, macros::*, *};

impl Expander<'_> {
    pub(super) fn body(&mut self, forms: &[Form], env: &mut Env) -> Result<crate::CoreExpr, Error> {
        let mut output = Vec::new();
        let mut index = 0;
        while index < forms.len() {
            if self.define_syntax(&forms[index], env)? {
                index += 1;
                continue;
            }
            output.push(self.expr(&forms[index], env)?);
            index += 1;
        }
        Ok(crate::CoreExpr::Begin(output))
    }

    pub(super) fn body_with_definitions(
        &mut self,
        forms: &[Form],
        env: &mut Env,
    ) -> Result<crate::CoreExpr, Error> {
        let mut predeclared = HashMap::new();
        for form in forms {
            let Some(items) = form.proper_list() else {
                break;
            };
            match items.first().and_then(Form::symbol) {
                Some("define-syntax") => continue,
                Some("define") if items.len() >= 2 => {
                    let name = items[1].symbol().or_else(|| {
                        items[1]
                            .proper_list()
                            .and_then(|signature| signature.first())
                            .and_then(Form::symbol)
                    });
                    if let Some(name) = name {
                        predeclared
                            .entry(name.to_owned())
                            .or_insert_with(|| self.fresh(name));
                    }
                }
                Some("define-values") if items.len() >= 2 => {
                    if let Ok((names, rest)) = formals(&items[1], self) {
                        for name in names.into_iter().chain(rest) {
                            predeclared
                                .entry(name.clone())
                                .or_insert_with(|| self.fresh(&name));
                        }
                    }
                }
                _ => break,
            }
        }
        let mut scoped = env.child(predeclared);
        let env = &mut scoped;
        let mut index = 0;
        while index < forms.len() && self.define_syntax(&forms[index], env)? {
            index += 1;
        }
        let mut bindings = Vec::new();
        while let Some(form) = forms.get(index) {
            let Some(items) = form.proper_list() else {
                break;
            };
            if items.first().and_then(Form::symbol) == Some("define-values") && items.len() == 3 {
                let (names, rest) = formals(&items[1], self)?;
                for name in names.into_iter().chain(rest) {
                    bindings.push(Form {
                        kind: FormKind::List(
                            vec![
                                Form {
                                    kind: FormKind::Symbol(name),
                                    span: form.span,
                                },
                                Form {
                                    kind: FormKind::Bool(false),
                                    span: form.span,
                                },
                            ],
                            None,
                        ),
                        span: form.span,
                    });
                }
                break;
            }
            if items.first().and_then(Form::symbol) != Some("define") {
                break;
            }
            if items.len() < 3 {
                return Err(self.error(form.span, "internal define requires a target and value"));
            }
            let (name, value) = if let Some(name) = items[1].symbol() {
                if items.len() != 3 {
                    return Err(self.error(form.span, "internal define requires one value"));
                }
                (name.to_owned(), items[2].clone())
            } else {
                let (signature, tail) = match &items[1].kind {
                    FormKind::List(signature, tail) if !signature.is_empty() => {
                        (signature, tail.clone())
                    }
                    _ => return Err(self.error(items[1].span, "invalid internal definition")),
                };
                let name = signature[0].symbol().ok_or_else(|| {
                    self.error(signature[0].span, "procedure name must be an identifier")
                })?;
                let formals = Form {
                    kind: if signature.len() == 1 {
                        tail.map_or(FormKind::Nil, |rest| rest.kind.clone())
                    } else {
                        FormKind::List(signature[1..].to_vec(), tail)
                    },
                    span: items[1].span,
                };
                let mut lambda = vec![
                    Form {
                        kind: FormKind::Symbol("lambda".into()),
                        span: form.span,
                    },
                    formals,
                ];
                lambda.extend_from_slice(&items[2..]);
                (
                    name.to_owned(),
                    Form {
                        kind: FormKind::List(lambda, None),
                        span: form.span,
                    },
                )
            };
            bindings.push(Form {
                kind: FormKind::List(
                    vec![
                        Form {
                            kind: FormKind::Symbol(name),
                            span: form.span,
                        },
                        value,
                    ],
                    None,
                ),
                span: form.span,
            });
            index += 1;
        }
        if bindings.is_empty() {
            return self.body(&forms[index..], env);
        }
        if index == forms.len() {
            return Err(self.error(forms[0].span, "body must end with an expression"));
        }
        let binding_form = Form {
            kind: FormKind::List(bindings, None),
            span: forms[0].span,
        };
        let mut letrec = vec![binding_form];
        letrec.extend_from_slice(&forms[index..]);
        self.letrec(&letrec, env, forms[0].span)
    }

    pub(super) fn define_syntax(&mut self, form: &Form, env: &mut Env) -> Result<bool, Error> {
        let Some(items) = form.proper_list() else {
            return Ok(false);
        };
        if items.len() != 3 || items[0].symbol() != Some("define-syntax") {
            return Ok(false);
        }
        let name = items[1]
            .symbol()
            .ok_or_else(|| self.error(items[1].span, "define-syntax name must be an identifier"))?;
        let name = canonical_identifier(name).to_owned();
        let binding = format!(
            "#syntax-binding:{}:{}:{}",
            form.span.source().index(),
            form.span.start(),
            self.fresh(&name)
        );
        let transformer = parse_macro(&items[2], self, env, binding.clone())?;
        self.macros.insert(binding.clone(), transformer.clone());
        self.macros.insert(name.clone(), transformer);
        env.bind_syntax(name, binding);
        Ok(true)
    }

    pub(super) fn expr(&mut self, form: &Form, env: &mut Env) -> Result<crate::CoreExpr, Error> {
        self.expansion_depth += 1;
        if self.expansion_depth > self.limits.max_expansion_depth() {
            self.expansion_depth -= 1;
            return Err(Error::from_diagnostic(
                crate::Diagnostic::new(
                    ErrorKind::ExpansionLimitExceeded,
                    "macro expansion depth limit exceeded",
                )
                .with_label(
                    form.span,
                    crate::LabelStyle::Primary,
                    "expansion stopped here",
                ),
            ));
        }
        let result = self.expr_inner(form, env);
        self.expansion_depth -= 1;
        result
    }

    fn expr_inner(&mut self, form: &Form, env: &mut Env) -> Result<crate::CoreExpr, Error> {
        match &form.kind {
            FormKind::Nil
            | FormKind::Bool(_)
            | FormKind::Char(_)
            | FormKind::String(_)
            | FormKind::Number(_)
            | FormKind::Vector(_)
            | FormKind::Bytevector(_) => self.quote(form),
            FormKind::Symbol(name) => Ok(crate::CoreExpr::Variable(env.resolve(name))),
            FormKind::List(items, tail) => {
                if tail.is_some() {
                    return Err(
                        self.error(form.span, "an expression application must be a proper list")
                    );
                }
                if items.is_empty() {
                    return Err(self.error(form.span, "the empty list is not an expression"));
                }
                if let Some(raw_name) = items[0].symbol() {
                    let name = canonical_identifier(raw_name);
                    let forced_syntax = matches!(
                        &items[0].kind,
                        FormKind::Symbol(symbol) if symbol.starts_with("#syntax#")
                    );
                    if forced_syntax || !env.keyword_shadowed(name) {
                        if let Some(transformer) = self.macros.get(name).cloned() {
                            self.tick(form.span)?;
                            let expanded = apply_macro(&transformer, form, env, self)?;
                            if self.define_syntax(&expanded, env)? {
                                return Ok(crate::CoreExpr::Literal(Value::unspecified()));
                            }
                            return self.expr(&expanded, env);
                        }
                        match name {
                            "quote" => return one(items, form.span, self, |x, e| e.quote(x)),
                            "cond-expand" => return self.cond_expand(&items[1..], env, form.span),
                            "features" if items.len() == 1 => return self.features(form.span),
                            "eval" => return self.static_eval(&items[1..], form.span),
                            "call-with-port" => {
                                if items.len() != 3 {
                                    return Err(self.error(
                                        form.span,
                                        "call-with-port requires a port and procedure",
                                    ));
                                }
                                return Ok(crate::CoreExpr::CallWithPort {
                                    port: Box::new(self.expr(&items[1], env)?),
                                    procedure: Box::new(self.expr(&items[2], env)?),
                                });
                            }
                            "call-with-input-file" | "call-with-output-file" => {
                                if items.len() != 3 {
                                    return Err(self.error(
                                        form.span,
                                        "call-with-*-file requires a path and procedure",
                                    ));
                                }
                                return Ok(crate::CoreExpr::CallWithFile {
                                    input: name == "call-with-input-file",
                                    path: Box::new(self.expr(&items[1], env)?),
                                    procedure: Box::new(self.expr(&items[2], env)?),
                                });
                            }
                            "with-input-from-file" | "with-output-to-file" => {
                                if items.len() != 3 {
                                    return Err(self.error(
                                        form.span,
                                        "with-*-file requires a path and thunk",
                                    ));
                                }
                                return Ok(crate::CoreExpr::WithFile {
                                    input: name == "with-input-from-file",
                                    path: Box::new(self.expr(&items[1], env)?),
                                    thunk: Box::new(self.expr(&items[2], env)?),
                                });
                            }
                            "load" => {
                                if !(2..=3).contains(&items.len()) {
                                    return Err(self.error(
                                        form.span,
                                        "load requires a path and optional environment",
                                    ));
                                }
                                return Ok(crate::CoreExpr::Load {
                                    path: Box::new(self.expr(&items[1], env)?),
                                    environment: items
                                        .get(2)
                                        .map(|item| self.expr(item, env).map(Box::new))
                                        .transpose()?,
                                });
                            }
                            "values" => {
                                return Ok(crate::CoreExpr::Values(
                                    items[1..]
                                        .iter()
                                        .map(|item| self.expr(item, env))
                                        .collect::<Result<_, _>>()?,
                                ));
                            }
                            "call-with-values" => {
                                if items.len() != 3 {
                                    return Err(self.error(
                                        form.span,
                                        "call-with-values requires a producer and consumer",
                                    ));
                                }
                                return Ok(crate::CoreExpr::CallWithValues {
                                    producer: Box::new(self.expr(&items[1], env)?),
                                    consumer: Box::new(self.expr(&items[2], env)?),
                                });
                            }
                            "if" => {
                                if !(3..=4).contains(&items.len()) {
                                    return Err(self.error(
                                        form.span,
                                        "if requires a test, consequent, and optional alternate",
                                    ));
                                }
                                let alternate = if items.len() == 4 {
                                    self.expr(&items[3], env)?
                                } else {
                                    crate::CoreExpr::Literal(Value::unspecified())
                                };
                                return Ok(crate::CoreExpr::If(
                                    Box::new(self.expr(&items[1], env)?),
                                    Box::new(self.expr(&items[2], env)?),
                                    Box::new(alternate),
                                ));
                            }
                            "begin" => return self.body(&items[1..], env),
                            "lambda" => return self.lambda(items, env, form.span),
                            "set!" => {
                                if items.len() != 3 {
                                    return Err(self.error(
                                        form.span,
                                        "set! requires an identifier and value",
                                    ));
                                }
                                let n = items[1].symbol().ok_or_else(|| {
                                    self.error(items[1].span, "set! target must be an identifier")
                                })?;
                                let name = env.resolve(n);
                                if self.immutable_imports.contains(&name) {
                                    return Err(self.error(
                                        items[1].span,
                                        "cannot mutate an imported binding",
                                    ));
                                }
                                return Ok(crate::CoreExpr::Set {
                                    name,
                                    value: Box::new(self.expr(&items[2], env)?),
                                });
                            }
                            "define" => return self.define(items, env, form.span),
                            "define-record-type" => {
                                return self.define_record_type(items, env, form.span);
                            }
                            "and" => return self.and(&items[1..], env),
                            "or" => return self.or(&items[1..], env),
                            "when" | "unless" => {
                                return self.when(name, &items[1..], env, form.span);
                            }
                            "let" => return self.let_form(&items[1..], env, form.span),
                            "let*" => return self.let_star(&items[1..], env, form.span),
                            "letrec" | "letrec*" => {
                                return self.letrec(&items[1..], env, form.span);
                            }
                            "let-syntax" | "letrec-syntax" => {
                                return self.local_syntax(
                                    &items[1..],
                                    env,
                                    form.span,
                                    name == "letrec-syntax",
                                );
                            }
                            "do" => return self.do_form(&items[1..], env, form.span),
                            "syntax-error" => {
                                let message = items
                                    .get(1)
                                    .and_then(|form| match &form.kind {
                                        FormKind::String(message) => Some(message.as_str()),
                                        _ => None,
                                    })
                                    .unwrap_or("syntax-error");
                                return Err(self.error(form.span, message));
                            }
                            "cond" => return self.cond(&items[1..], env),
                            "case" => return self.case_form(&items[1..], env, form.span),
                            "quasiquote" => {
                                return one(items, form.span, self, |x, e| e.quasiquote(x, env, 1));
                            }
                            "unquote" | "unquote-splicing" => {
                                return Err(self
                                    .error(form.span, "unquote is valid only inside quasiquote"));
                            }
                            "let-values" => return self.let_values(&items[1..], env, form.span),
                            "let*-values" => {
                                return self.let_star_values(&items[1..], env, form.span);
                            }
                            "define-values" => {
                                return self.define_values(&items[1..], env, form.span);
                            }
                            "delay" => {
                                return one(items, form.span, self, |item, expander| {
                                    Ok(crate::CoreExpr::Delay(Box::new(expander.expr(item, env)?)))
                                });
                            }
                            "delay-force" => {
                                return one(items, form.span, self, |item, expander| {
                                    Ok(crate::CoreExpr::DelayForce(Box::new(
                                        expander.expr(item, env)?,
                                    )))
                                });
                            }
                            "force" => {
                                return one(items, form.span, self, |item, expander| {
                                    Ok(crate::CoreExpr::Force(Box::new(expander.expr(item, env)?)))
                                });
                            }
                            "with-exception-handler" => {
                                if items.len() != 3 {
                                    return Err(self.error(
                                        form.span,
                                        "with-exception-handler requires handler and thunk",
                                    ));
                                }
                                return Ok(crate::CoreExpr::WithExceptionHandler {
                                    handler: Box::new(self.expr(&items[1], env)?),
                                    thunk: Box::new(self.expr(&items[2], env)?),
                                });
                            }
                            "raise" | "raise-continuable" => {
                                return one(items, form.span, self, |item, expander| {
                                    Ok(crate::CoreExpr::Raise {
                                        object: Box::new(expander.expr(item, env)?),
                                        continuable: name == "raise-continuable",
                                    })
                                });
                            }
                            "call-with-current-continuation" | "call/cc" => {
                                return one(items, form.span, self, |item, expander| {
                                    Ok(crate::CoreExpr::CallWithCurrentContinuation(Box::new(
                                        expander.expr(item, env)?,
                                    )))
                                });
                            }
                            "dynamic-wind" => {
                                if items.len() != 4 {
                                    return Err(self.error(
                                        form.span,
                                        "dynamic-wind requires before, thunk, and after",
                                    ));
                                }
                                return Ok(crate::CoreExpr::DynamicWind {
                                    before: Box::new(self.expr(&items[1], env)?),
                                    thunk: Box::new(self.expr(&items[2], env)?),
                                    after: Box::new(self.expr(&items[3], env)?),
                                });
                            }
                            "parameterize" => {
                                if items.len() < 3 {
                                    return Err(self.error(
                                        form.span,
                                        "parameterize requires bindings and a body",
                                    ));
                                }
                                let bindings = match &items[1].kind {
                                    FormKind::Nil => &[][..],
                                    _ => items[1].proper_list().ok_or_else(|| {
                                        self.error(
                                            items[1].span,
                                            "parameter bindings must be a proper list",
                                        )
                                    })?,
                                };
                                let mut output = Vec::new();
                                for binding in bindings {
                                    let pair = binding.proper_list().ok_or_else(|| {
                                        self.error(
                                            binding.span,
                                            "parameter binding must be (parameter value)",
                                        )
                                    })?;
                                    if pair.len() != 2 {
                                        return Err(self.error(
                                            binding.span,
                                            "parameter binding must be (parameter value)",
                                        ));
                                    }
                                    output.push((
                                        self.expr(&pair[0], env)?,
                                        self.expr(&pair[1], env)?,
                                    ));
                                }
                                return Ok(crate::CoreExpr::Parameterize {
                                    bindings: output,
                                    body: Box::new(self.body(&items[2..], env)?),
                                });
                            }
                            "make-parameter" => {
                                if !(items.len() == 2 || items.len() == 3) {
                                    return Err(self.error(
                                    form.span,
                                    "make-parameter requires an initial value and optional converter",
                                ));
                                }
                                return Ok(crate::CoreExpr::MakeParameter {
                                    initial: Box::new(self.expr(&items[1], env)?),
                                    converter: items
                                        .get(2)
                                        .map(|item| self.expr(item, env).map(Box::new))
                                        .transpose()?,
                                });
                            }
                            "error" => {
                                if items.len() < 2 {
                                    return Err(self.error(form.span, "error requires a message"));
                                }
                                return Ok(crate::CoreExpr::Error {
                                    message: Box::new(self.expr(&items[1], env)?),
                                    irritants: items[2..]
                                        .iter()
                                        .map(|item| self.expr(item, env))
                                        .collect::<Result<_, _>>()?,
                                });
                            }
                            "case-lambda" => return self.case_lambda(&items[1..], env, form.span),
                            "guard" => return self.guard(&items[1..], env, form.span),
                            _ => {}
                        }
                    }
                }
                let procedure = Box::new(self.expr(&items[0], env)?);
                let arguments = items[1..]
                    .iter()
                    .map(|x| self.expr(x, env))
                    .collect::<Result<_, _>>()?;
                Ok(crate::CoreExpr::Call {
                    procedure,
                    arguments,
                })
            }
        }
    }

    /// Lowers a literal datum. Immediates stay inline literals. Datums that
    /// allocate (strings, vectors, bytevectors, lists, symbols) hoist to one
    /// hidden content-named global per unit, defined at unit entry and
    /// deep-frozen through the `%literal` native, so a literal inside a loop
    /// no longer allocates per evaluation and mutating an R7RS constant
    /// raises. Content naming keeps the hidden names collision-free across
    /// compilation units: any redefinition rebinds an equal immutable value,
    /// and R7RS permits a literal to evaluate to the same object every time.
    pub(super) fn quote(&mut self, form: &Form) -> Result<crate::CoreExpr, Error> {
        match &form.kind {
            FormKind::Nil | FormKind::Bool(_) | FormKind::Char(_) | FormKind::Number(_) => {
                literal(form)
            }
            FormKind::String(_)
            | FormKind::Symbol(_)
            | FormKind::List(..)
            | FormKind::Vector(_)
            | FormKind::Bytevector(_) => {
                let mut name = String::from("#quote#");
                literal_key(form, &mut name);
                if self.hoisted_names.insert(name.clone()) {
                    let value = call("%literal", vec![literal(form)?]);
                    self.hoisted.push((name.clone(), value));
                }
                Ok(crate::CoreExpr::Variable(name))
            }
        }
    }

    pub(super) fn cond_expand(
        &mut self,
        clauses: &[Form],
        env: &mut Env,
        span: Span,
    ) -> Result<crate::CoreExpr, Error> {
        let mut selected = None;
        for (index, clause) in clauses.iter().enumerate() {
            let items = clause.proper_list().ok_or_else(|| {
                self.error(clause.span, "cond-expand clause must be a proper list")
            })?;
            if items.is_empty() {
                return Err(self.error(clause.span, "cond-expand clause is empty"));
            }
            let matches = if items[0].symbol() == Some("else") {
                if index + 1 != clauses.len() {
                    return Err(self.error(items[0].span, "cond-expand else clause must be last"));
                }
                true
            } else {
                self.feature_requirement(&items[0])?
            };
            if matches && selected.is_none() {
                selected = Some(items[1..].to_vec());
            }
        }
        let selected = selected.ok_or_else(|| {
            self.error(
                span,
                "no cond-expand clause matched the configured feature set",
            )
        })?;
        self.body(&selected, env)
    }

    pub(super) fn feature_requirement(&self, requirement: &Form) -> Result<bool, Error> {
        if let Some(identifier) = requirement.symbol() {
            return Ok(self.features.contains(identifier));
        }
        let items = requirement
            .proper_list()
            .ok_or_else(|| self.error(requirement.span, "invalid cond-expand requirement"))?;
        let Some(keyword) = items.first().and_then(Form::symbol) else {
            return Err(self.error(requirement.span, "invalid cond-expand requirement"));
        };
        match keyword {
            "and" => items[1..]
                .iter()
                .map(|item| self.feature_requirement(item))
                .collect::<Result<Vec<_>, _>>()
                .map(|items| items.into_iter().all(|item| item)),
            "or" => items[1..]
                .iter()
                .map(|item| self.feature_requirement(item))
                .collect::<Result<Vec<_>, _>>()
                .map(|items| items.into_iter().any(|item| item)),
            "not" if items.len() == 2 => Ok(!self.feature_requirement(&items[1])?),
            "library" if items.len() == 2 => {
                let name = crate::library::parse_name(&items[1])
                    .map_err(|_| self.error(items[1].span, "invalid library requirement"))?;
                Ok(self.features.contains_library(&name))
            }
            _ => Err(self.error(requirement.span, "invalid cond-expand feature requirement")),
        }
    }

    pub(super) fn features(&mut self, span: Span) -> Result<crate::CoreExpr, Error> {
        let values = self
            .features
            .identifiers()
            .into_iter()
            .map(|identifier| Form {
                kind: FormKind::Symbol(identifier.into()),
                span,
            })
            .collect();
        self.quote(&Form {
            kind: FormKind::List(values, None),
            span,
        })
    }

    /// Expands the portable, quoted form of `eval` without exposing Rust VM
    /// recursion. Dynamic datum evaluation is completed by the library VM
    /// path. This path covers the required standard idiom and keeps quoted
    /// definitions from mutating an immutable `(environment ...)`.
    pub(super) fn static_eval(
        &mut self,
        arguments: &[Form],
        span: Span,
    ) -> Result<crate::CoreExpr, Error> {
        if arguments.len() != 2 {
            return Err(self.error(span, "eval requires a datum and an environment"));
        }
        let quoted = arguments[0]
            .proper_list()
            .and_then(|items| {
                (items.len() == 2 && items[0].symbol() == Some("quote")).then_some(&items[1])
            })
            .ok_or_else(|| {
                self.error(arguments[0].span, "eval currently requires a quoted datum")
            })?;
        // Environment objects are immutable capability descriptors. All
        // standard bindings are resolved to engine globals, so a non-literal
        // environment expression does not need to be executed merely to
        // compile an already quoted datum.
        if quoted
            .proper_list()
            .and_then(|items| items.first())
            .and_then(Form::symbol)
            == Some("define")
        {
            return Err(self.error(quoted.span, "cannot define in an immutable environment"));
        }
        self.expr(quoted, &mut Env::default())
    }
}
