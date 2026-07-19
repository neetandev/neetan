//! Binding and definition forms: lambda/case-lambda, define, record types, the
//! `let` family, `letrec`, `do`, the multiple-values lets, and `and`/`or`/`when`.

use super::{helpers::*, macros::*, *};

impl Expander<'_> {
    pub(super) fn lambda(
        &mut self,
        items: &[Form],
        env: &mut Env,
        span: Span,
    ) -> Result<crate::CoreExpr, Error> {
        if items.len() < 3 {
            return Err(self.error(span, "lambda requires formals and a body"));
        };
        let (names, rest) = formals(&items[1], self)?;
        let mut all_names = names.clone();
        if let Some(rest) = &rest {
            all_names.push(rest.clone());
        }
        let binds = all_names
            .iter()
            .map(|n| (n.clone(), self.fresh(n)))
            .collect::<HashMap<_, _>>();
        let params = names.iter().map(|n| binds[n].clone()).collect();
        let rest = rest.as_ref().map(|name| binds[name].clone());
        let mut child = env.child(binds);
        let body = Box::new(self.body_with_definitions(&items[2..], &mut child)?);
        Ok(match rest {
            Some(rest) => crate::CoreExpr::LambdaRest {
                required: params,
                rest,
                body,
            },
            None => crate::CoreExpr::Lambda { params, body },
        })
    }

    pub(super) fn case_lambda(
        &mut self,
        clauses: &[Form],
        env: &mut Env,
        span: Span,
    ) -> Result<crate::CoreExpr, Error> {
        if clauses.is_empty() {
            return Err(self.error(span, "case-lambda requires at least one clause"));
        }
        let mut output = Vec::with_capacity(clauses.len());
        for source in clauses {
            let clause = source.proper_list().ok_or_else(|| {
                self.error(source.span, "case-lambda clause must be a proper list")
            })?;
            if clause.len() < 2 {
                return Err(self.error(
                    source.span,
                    "case-lambda clause requires formals and a body",
                ));
            }
            let fake = [
                Form {
                    kind: FormKind::Symbol("lambda".into()),
                    span: source.span,
                },
                clause[0].clone(),
            ];
            let mut lambda = fake.to_vec();
            lambda.extend_from_slice(&clause[1..]);
            output.push(self.lambda(&lambda, env, source.span)?);
        }
        Ok(crate::CoreExpr::CaseLambda { clauses: output })
    }

    pub(super) fn define(
        &mut self,
        items: &[Form],
        env: &mut Env,
        span: Span,
    ) -> Result<crate::CoreExpr, Error> {
        if items.len() < 3 {
            return Err(self.error(span, "define requires a target and value"));
        };
        if let Some(name) = items[1].symbol() {
            if items.len() != 3 {
                return Err(self.error(span, "define requires one value"));
            };
            let name = env.resolve(name);
            if self.immutable_imports.contains(&name) {
                return Err(self.error(span, "cannot redefine an imported binding"));
            }
            return Ok(crate::CoreExpr::Define {
                name,
                value: Box::new(self.expr(&items[2], env)?),
            });
        }
        let (sig, tail) = match &items[1].kind {
            FormKind::List(sig, tail) => (sig.as_slice(), tail.clone()),
            _ => return Err(self.error(items[1].span, "invalid procedure definition")),
        };
        let name = sig
            .first()
            .and_then(Form::symbol)
            .ok_or_else(|| self.error(items[1].span, "procedure name must be an identifier"))?;
        let formals = Form {
            kind: if sig.len() == 1 {
                tail.map_or(FormKind::Nil, |rest| rest.kind.clone())
            } else {
                FormKind::List(sig[1..].to_vec(), tail)
            },
            span: items[1].span,
        };
        let mut lambda = vec![
            Form {
                kind: FormKind::Symbol("lambda".into()),
                span,
            },
            formals,
        ];
        lambda.extend_from_slice(&items[2..]);
        let l = Form {
            kind: FormKind::List(lambda, None),
            span,
        };
        let name = env.resolve(name);
        if self.immutable_imports.contains(&name) {
            return Err(self.error(span, "cannot redefine an imported binding"));
        }
        Ok(crate::CoreExpr::Define {
            name,
            value: Box::new(self.expr(&l, env)?),
        })
    }

    pub(super) fn define_record_type(
        &mut self,
        items: &[Form],
        env: &mut Env,
        span: Span,
    ) -> Result<crate::CoreExpr, Error> {
        if items.len() < 4 {
            return Err(self.error(
                span,
                "define-record-type requires a type, constructor, predicate, and fields",
            ));
        }
        let type_name = items[1]
            .symbol()
            .ok_or_else(|| self.error(items[1].span, "record type name must be an identifier"))?;
        let constructor = items[2].proper_list().ok_or_else(|| {
            self.error(
                items[2].span,
                "record constructor specification must be a list",
            )
        })?;
        let constructor_name = constructor
            .first()
            .and_then(Form::symbol)
            .ok_or_else(|| self.error(items[2].span, "record constructor needs a name"))?;
        let predicate_name = items[3].symbol().ok_or_else(|| {
            self.error(items[3].span, "record predicate name must be an identifier")
        })?;
        let mut fields = Vec::new();
        for field in &items[4..] {
            let specification = field.proper_list().ok_or_else(|| {
                self.error(field.span, "record field specification must be a list")
            })?;
            if !(2..=3).contains(&specification.len()) {
                return Err(self.error(
                    field.span,
                    "record field requires a tag, accessor, and optional mutator",
                ));
            }
            let tag = specification[0].symbol().ok_or_else(|| {
                self.error(
                    specification[0].span,
                    "record field tag must be an identifier",
                )
            })?;
            let accessor = specification[1].symbol().ok_or_else(|| {
                self.error(
                    specification[1].span,
                    "record accessor must be an identifier",
                )
            })?;
            let mutator = specification
                .get(2)
                .map(|form| {
                    form.symbol().map(str::to_owned).ok_or_else(|| {
                        self.error(form.span, "record mutator must be an identifier")
                    })
                })
                .transpose()?;
            if fields.iter().any(|(existing, _, _)| existing == tag) {
                return Err(self.error(field.span, "duplicate record field tag"));
            }
            fields.push((tag.to_owned(), accessor.to_owned(), mutator));
        }
        let hidden_type = self.fresh(type_name);
        let mut expressions = vec![crate::CoreExpr::Define {
            name: hidden_type.clone(),
            value: Box::new(call(
                "%make-record-type",
                vec![crate::CoreExpr::Literal(Value::integer(
                    i64::try_from(fields.len())
                        .map_err(|_| self.error(span, "record type has too many fields"))?,
                ))],
            )),
        }];
        let mut constructor_arguments = vec![crate::CoreExpr::Variable(hidden_type.clone())];
        for tag in &constructor[1..] {
            let tag = tag.symbol().ok_or_else(|| {
                self.error(tag.span, "constructor field tag must be an identifier")
            })?;
            let field = fields
                .iter()
                .position(|(candidate, _, _)| candidate == tag)
                .ok_or_else(|| self.error(items[2].span, "constructor names an unknown field"))?;
            constructor_arguments.push(crate::CoreExpr::Literal(Value::integer(
                i64::try_from(field)
                    .map_err(|_| self.error(items[2].span, "record field index overflow"))?,
            )));
        }
        expressions.push(crate::CoreExpr::Define {
            name: env.resolve(constructor_name),
            value: Box::new(call("%make-record-constructor", constructor_arguments)),
        });
        expressions.push(crate::CoreExpr::Define {
            name: env.resolve(predicate_name),
            value: Box::new(call(
                "%make-record-predicate",
                vec![crate::CoreExpr::Variable(hidden_type.clone())],
            )),
        });
        for (field, (_, accessor, mutator)) in fields.iter().enumerate() {
            let arguments = vec![
                crate::CoreExpr::Variable(hidden_type.clone()),
                crate::CoreExpr::Literal(Value::integer(
                    i64::try_from(field)
                        .map_err(|_| self.error(span, "record field index overflow"))?,
                )),
            ];
            expressions.push(crate::CoreExpr::Define {
                name: env.resolve(accessor),
                value: Box::new(call("%make-record-accessor", arguments.clone())),
            });
            if let Some(mutator) = mutator {
                expressions.push(crate::CoreExpr::Define {
                    name: env.resolve(mutator),
                    value: Box::new(call("%make-record-mutator", arguments)),
                });
            }
        }
        Ok(crate::CoreExpr::Begin(expressions))
    }

    pub(super) fn and(&mut self, xs: &[Form], env: &mut Env) -> Result<crate::CoreExpr, Error> {
        if xs.is_empty() {
            return Ok(crate::CoreExpr::Literal(Value::boolean(true)));
        }
        let first = self.expr(&xs[0], env)?;
        if xs.len() == 1 {
            return Ok(first);
        }
        Ok(crate::CoreExpr::If(
            Box::new(first),
            Box::new(self.and(&xs[1..], env)?),
            Box::new(crate::CoreExpr::Literal(Value::boolean(false))),
        ))
    }

    pub(super) fn or(&mut self, xs: &[Form], env: &mut Env) -> Result<crate::CoreExpr, Error> {
        if xs.is_empty() {
            return Ok(crate::CoreExpr::Literal(Value::boolean(false)));
        }
        if xs.len() == 1 {
            return self.expr(&xs[0], env);
        }
        let n = self.fresh("or");
        let first = self.expr(&xs[0], env)?;
        let rest = self.or(&xs[1..], env)?;
        Ok(crate::CoreExpr::Call {
            procedure: Box::new(crate::CoreExpr::Lambda {
                params: vec![n.clone()],
                body: Box::new(crate::CoreExpr::If(
                    Box::new(crate::CoreExpr::Variable(n.clone())),
                    Box::new(crate::CoreExpr::Variable(n)),
                    Box::new(rest),
                )),
            }),
            arguments: vec![first],
        })
    }

    pub(super) fn when(
        &mut self,
        name: &str,
        xs: &[Form],
        env: &mut Env,
        span: Span,
    ) -> Result<crate::CoreExpr, Error> {
        if xs.is_empty() {
            return Err(self.error(span, "conditional body requires a test"));
        }
        let test = self.expr(&xs[0], env)?;
        let body = self.body(&xs[1..], env)?;
        let (yes, no) = if name == "when" {
            (body, crate::CoreExpr::Literal(Value::unspecified()))
        } else {
            (crate::CoreExpr::Literal(Value::unspecified()), body)
        };
        Ok(crate::CoreExpr::If(
            Box::new(test),
            Box::new(yes),
            Box::new(no),
        ))
    }

    pub(super) fn let_form(
        &mut self,
        xs: &[Form],
        env: &mut Env,
        span: Span,
    ) -> Result<crate::CoreExpr, Error> {
        if xs.len() < 2 {
            return Err(self.error(
                span,
                format!(
                    "let requires bindings and a body; received {} operands",
                    xs.len()
                ),
            ));
        }
        if let Some(name) = xs[0].symbol() {
            if xs.len() < 3 {
                return Err(self.error(span, "named let requires bindings and a body"));
            }
            let bindings = bindings(&xs[1], self)?;
            let arguments = bindings
                .iter()
                .map(|(_, value)| self.expr(value, env))
                .collect::<Result<Vec<_>, _>>()?;
            let procedure_name = self.fresh(name);
            let mut procedure_binding = HashMap::new();
            procedure_binding.insert(name.to_owned(), procedure_name.clone());
            let procedure_env = env.child(procedure_binding);
            let mut parameter_bindings = HashMap::new();
            let mut parameters = Vec::new();
            for (parameter, _) in &bindings {
                let unique = self.fresh(parameter);
                parameter_bindings.insert(parameter.clone(), unique.clone());
                parameters.push(unique);
            }
            let mut body_env = procedure_env.child(parameter_bindings);
            let body = Box::new(self.body(&xs[2..], &mut body_env)?);
            // The compiler flattens this into a register loop when `name` does
            // not escape and all self-calls are tail calls, and otherwise lowers
            // it to the equivalent closure desugaring.
            return Ok(crate::CoreExpr::NamedLet {
                name: procedure_name,
                params: parameters,
                inits: arguments,
                body,
            });
        }
        let bindings = bindings(&xs[0], self)?;
        let values = bindings
            .iter()
            .map(|(_, v)| self.expr(v, env))
            .collect::<Result<Vec<_>, _>>()?;
        let names = bindings.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
        self.apply_lambda(names, values, &xs[1..], env)
    }

    pub(super) fn local_syntax(
        &mut self,
        xs: &[Form],
        env: &mut Env,
        span: Span,
        recursive: bool,
    ) -> Result<crate::CoreExpr, Error> {
        if xs.len() < 2 {
            return Err(self.error(span, "local syntax requires bindings and a body"));
        }
        let specifications = match &xs[0].kind {
            FormKind::Nil => &[][..],
            _ => xs[0]
                .proper_list()
                .ok_or_else(|| self.error(xs[0].span, "syntax bindings must be a proper list"))?,
        };
        let saved = self.macros.clone();
        let mut bindings = HashMap::new();
        for specification in specifications {
            let pair = specification
                .proper_list()
                .ok_or_else(|| self.error(specification.span, "syntax binding must be a pair"))?;
            let Some(name) = pair.first().and_then(Form::symbol) else {
                return Err(self.error(specification.span, "syntax binding needs an identifier"));
            };
            let binding = format!(
                "#syntax-binding:{}:{}:{}",
                specification.span.source().index(),
                specification.span.start(),
                self.fresh(name)
            );
            if bindings.insert(name.to_owned(), binding).is_some() {
                return Err(self.error(specification.span, "duplicate local syntax binding"));
            }
        }
        let mut scoped = env.child(bindings.clone());
        let transformer_env = if recursive { &scoped } else { env };
        let parsed = specifications
            .iter()
            .map(|specification| {
                let pair = specification.proper_list().ok_or_else(|| {
                    self.error(specification.span, "syntax binding must be a pair")
                })?;
                if pair.len() != 2 {
                    return Err(self.error(
                        specification.span,
                        "syntax binding requires an identifier and transformer",
                    ));
                }
                let name = pair[0].symbol().ok_or_else(|| {
                    self.error(pair[0].span, "syntax binding name must be an identifier")
                })?;
                Ok((
                    canonical_identifier(name).to_owned(),
                    parse_macro(&pair[1], self, transformer_env, bindings[name].clone())?,
                ))
            })
            .collect::<Result<Vec<_>, Error>>();
        let result = match parsed {
            Ok(parsed) => {
                for (name, transformer) in parsed {
                    self.macros
                        .insert(transformer.binding.clone(), transformer.clone());
                    self.macros.insert(name, transformer);
                }
                self.body(&xs[1..], &mut scoped)
            }
            Err(error) => Err(error),
        };
        self.macros = saved;
        result
    }

    pub(super) fn do_form(
        &mut self,
        xs: &[Form],
        env: &mut Env,
        span: Span,
    ) -> Result<crate::CoreExpr, Error> {
        if xs.len() < 2 {
            return Err(self.error(span, "do requires bindings and a termination clause"));
        }
        let specifications = xs[0]
            .proper_list()
            .ok_or_else(|| self.error(xs[0].span, "do bindings must be a proper list"))?;
        let termination = xs[1]
            .proper_list()
            .ok_or_else(|| self.error(xs[1].span, "do termination clause must be a proper list"))?;
        if termination.is_empty() {
            return Err(self.error(xs[1].span, "do termination clause needs a test"));
        }
        let loop_name = self.fresh("do");
        let mut bindings = Vec::new();
        let mut steps = Vec::new();
        for specification in specifications {
            let binding = specification.proper_list().ok_or_else(|| {
                self.error(specification.span, "do binding must be a proper list")
            })?;
            if !(2..=3).contains(&binding.len()) {
                return Err(self.error(
                    specification.span,
                    "do binding requires a variable, initial value, and optional step",
                ));
            }
            let name = binding[0]
                .symbol()
                .ok_or_else(|| self.error(binding[0].span, "do variable must be an identifier"))?;
            bindings.push(Form {
                kind: FormKind::List(vec![binding[0].clone(), binding[1].clone()], None),
                span: specification.span,
            });
            steps.push(binding.get(2).cloned().unwrap_or(Form {
                kind: FormKind::Symbol(name.into()),
                span: binding[0].span,
            }));
        }
        let loop_call = Form {
            kind: FormKind::List(
                std::iter::once(Form {
                    kind: FormKind::Symbol(loop_name.clone()),
                    span,
                })
                .chain(steps)
                .collect(),
                None,
            ),
            span,
        };
        let mut continue_body = vec![Form {
            kind: FormKind::Symbol("begin".into()),
            span,
        }];
        continue_body.extend_from_slice(&xs[2..]);
        continue_body.push(loop_call);
        let consequent = if termination.len() == 1 {
            Form {
                kind: FormKind::List(
                    vec![
                        Form {
                            kind: FormKind::Symbol("if".into()),
                            span,
                        },
                        Form {
                            kind: FormKind::Bool(false),
                            span,
                        },
                        Form {
                            kind: FormKind::Bool(false),
                            span,
                        },
                    ],
                    None,
                ),
                span,
            }
        } else {
            let mut begin = vec![Form {
                kind: FormKind::Symbol("begin".into()),
                span,
            }];
            begin.extend_from_slice(&termination[1..]);
            Form {
                kind: FormKind::List(begin, None),
                span,
            }
        };
        let conditional = Form {
            kind: FormKind::List(
                vec![
                    Form {
                        kind: FormKind::Symbol("if".into()),
                        span,
                    },
                    termination[0].clone(),
                    consequent,
                    Form {
                        kind: FormKind::List(continue_body, None),
                        span,
                    },
                ],
                None,
            ),
            span,
        };
        let named_let = vec![
            Form {
                kind: FormKind::Symbol(loop_name),
                span,
            },
            Form {
                kind: FormKind::List(bindings, None),
                span,
            },
            conditional,
        ];
        self.let_form(&named_let, env, span)
    }

    pub(super) fn let_star(
        &mut self,
        xs: &[Form],
        env: &mut Env,
        span: Span,
    ) -> Result<crate::CoreExpr, Error> {
        if xs.len() < 2 {
            return Err(self.error(span, "let* requires bindings and a body"));
        }
        let bs = bindings(&xs[0], self)?;
        self.let_star_bindings(&bs, &xs[1..], env)
    }

    pub(super) fn let_star_bindings(
        &mut self,
        bs: &[(String, Form)],
        body: &[Form],
        env: &mut Env,
    ) -> Result<crate::CoreExpr, Error> {
        let Some((name, value)) = bs.first() else {
            return self.body_with_definitions(body, env);
        };
        let argument = self.expr(value, env)?;
        let unique = self.fresh(name);
        let mut binds = HashMap::new();
        binds.insert(name.clone(), unique.clone());
        let mut child = env.child(binds);
        let inner = self.let_star_bindings(&bs[1..], body, &mut child)?;
        Ok(crate::CoreExpr::Call {
            procedure: Box::new(crate::CoreExpr::Lambda {
                params: vec![unique],
                body: Box::new(inner),
            }),
            arguments: vec![argument],
        })
    }

    pub(super) fn letrec(
        &mut self,
        xs: &[Form],
        env: &mut Env,
        span: Span,
    ) -> Result<crate::CoreExpr, Error> {
        if xs.len() < 2 {
            return Err(self.error(span, "letrec requires bindings and a body"));
        }
        let bindings = bindings(&xs[0], self)?;
        let mut names = Vec::new();
        let mut renamed = HashMap::new();
        for (name, _) in &bindings {
            let existing = env.resolve(name);
            let unique = if existing.starts_with('#') {
                existing
            } else {
                self.fresh(name)
            };
            renamed.insert(name.clone(), unique.clone());
            names.push(unique);
        }
        let mut child = env.child(renamed);
        let mut expressions = Vec::new();
        for ((_, value), name) in bindings.iter().zip(&names) {
            expressions.push(crate::CoreExpr::Set {
                name: name.clone(),
                value: Box::new(self.expr(value, &mut child)?),
            });
        }
        expressions.push(self.body(&xs[1..], &mut child)?);
        Ok(crate::CoreExpr::Call {
            procedure: Box::new(crate::CoreExpr::Lambda {
                params: names.clone(),
                body: Box::new(crate::CoreExpr::Begin(expressions)),
            }),
            arguments: names
                .iter()
                .map(|_| crate::CoreExpr::Literal(Value::unspecified()))
                .collect(),
        })
    }

    pub(super) fn let_star_values(
        &mut self,
        xs: &[Form],
        env: &mut Env,
        span: Span,
    ) -> Result<crate::CoreExpr, Error> {
        if xs.len() < 2 {
            return Err(self.error(span, "let*-values requires bindings and a body"));
        }
        let bindings = match &xs[0].kind {
            FormKind::Nil => &[][..],
            _ => xs[0]
                .proper_list()
                .ok_or_else(|| self.error(xs[0].span, "value bindings must be a proper list"))?,
        };
        self.let_star_values_bindings(bindings, &xs[1..], env)
    }

    pub(super) fn let_values(
        &mut self,
        xs: &[Form],
        env: &mut Env,
        span: Span,
    ) -> Result<crate::CoreExpr, Error> {
        if xs.len() < 2 {
            return Err(self.error(span, "let-values requires bindings and a body"));
        }
        let bindings = match &xs[0].kind {
            FormKind::Nil => &[][..],
            _ => xs[0]
                .proper_list()
                .ok_or_else(|| self.error(xs[0].span, "value bindings must be a proper list"))?,
        };
        let mut prepared = Vec::with_capacity(bindings.len());
        let mut names_to_bind = HashMap::new();
        for binding in bindings {
            let parts = binding.proper_list().ok_or_else(|| {
                self.error(binding.span, "value binding must be (formals expression)")
            })?;
            if parts.len() != 2 {
                return Err(self.error(binding.span, "value binding must be (formals expression)"));
            }
            let names = list_symbols(&parts[0], self)?;
            let expression = self.expr(&parts[1], env)?;
            let params = names
                .iter()
                .map(|name| self.fresh(name))
                .collect::<Vec<_>>();
            for (name, param) in names.iter().zip(&params) {
                if names_to_bind.insert(name.clone(), param.clone()).is_some() {
                    return Err(self.error(parts[0].span, "duplicate let-values binding"));
                }
            }
            prepared.push((expression, params));
        }
        let mut body_env = env.child(names_to_bind);
        let mut result = self.body(&xs[1..], &mut body_env)?;
        for (expression, params) in prepared.into_iter().rev() {
            result = crate::CoreExpr::CallWithValues {
                producer: Box::new(crate::CoreExpr::Lambda {
                    params: Vec::new(),
                    body: Box::new(expression),
                }),
                consumer: Box::new(crate::CoreExpr::Lambda {
                    params,
                    body: Box::new(result),
                }),
            };
        }
        Ok(result)
    }

    pub(super) fn let_star_values_bindings(
        &mut self,
        bindings: &[Form],
        body: &[Form],
        env: &mut Env,
    ) -> Result<crate::CoreExpr, Error> {
        let Some((binding, rest)) = bindings.split_first() else {
            return self.body(body, env);
        };
        let parts = binding.proper_list().ok_or_else(|| {
            self.error(binding.span, "value binding must be (formals expression)")
        })?;
        if parts.len() != 2 {
            return Err(self.error(binding.span, "value binding must be (formals expression)"));
        }
        let names = list_symbols(&parts[0], self)?;
        let producer = Box::new(crate::CoreExpr::Lambda {
            params: Vec::new(),
            body: Box::new(self.expr(&parts[1], env)?),
        });
        let renamed = names
            .iter()
            .map(|name| (name.clone(), self.fresh(name)))
            .collect::<HashMap<_, _>>();
        let params = names.iter().map(|name| renamed[name].clone()).collect();
        let mut child = env.child(renamed);
        let body = self.let_star_values_bindings(rest, body, &mut child)?;
        Ok(crate::CoreExpr::CallWithValues {
            producer,
            consumer: Box::new(crate::CoreExpr::Lambda {
                params,
                body: Box::new(body),
            }),
        })
    }

    pub(super) fn define_values(
        &mut self,
        xs: &[Form],
        env: &mut Env,
        span: Span,
    ) -> Result<crate::CoreExpr, Error> {
        if xs.len() != 2 {
            return Err(self.error(span, "define-values requires formals and an expression"));
        }
        let (names, rest) = formals(&xs[0], self)?;
        let producer = Box::new(crate::CoreExpr::Lambda {
            params: Vec::new(),
            body: Box::new(self.expr(&xs[1], env)?),
        });
        let params = names
            .iter()
            .map(|name| self.fresh(name))
            .collect::<Vec<_>>();
        let rest_param = rest.as_ref().map(|name| self.fresh(name));
        let mut definitions: Vec<_> = names
            .iter()
            .zip(&params)
            .map(|(name, param)| {
                let resolved = env.resolve(name);
                if resolved == *name {
                    crate::CoreExpr::Define {
                        name: resolved,
                        value: Box::new(crate::CoreExpr::Variable(param.clone())),
                    }
                } else {
                    crate::CoreExpr::Set {
                        name: resolved,
                        value: Box::new(crate::CoreExpr::Variable(param.clone())),
                    }
                }
            })
            .collect();
        if let (Some(name), Some(param)) = (rest, rest_param.as_ref()) {
            let resolved = env.resolve(&name);
            definitions.push(if resolved == name {
                crate::CoreExpr::Define {
                    name: resolved,
                    value: Box::new(crate::CoreExpr::Variable(param.clone())),
                }
            } else {
                crate::CoreExpr::Set {
                    name: resolved,
                    value: Box::new(crate::CoreExpr::Variable(param.clone())),
                }
            });
        }
        let consumer = match rest_param {
            Some(rest) => crate::CoreExpr::LambdaRest {
                required: params,
                rest,
                body: Box::new(crate::CoreExpr::Begin(definitions)),
            },
            None => crate::CoreExpr::Lambda {
                params,
                body: Box::new(crate::CoreExpr::Begin(definitions)),
            },
        };
        Ok(crate::CoreExpr::CallWithValues {
            producer,
            consumer: Box::new(consumer),
        })
    }
}
