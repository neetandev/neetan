//! Conditional and quasiquote forms: `cond`/`case`/`guard`, `apply`, and
//! quasiquotation.

use super::{helpers::*, *};

impl Expander<'_> {
    pub(super) fn apply_lambda(
        &mut self,
        names: Vec<String>,
        values: Vec<crate::CoreExpr>,
        body: &[Form],
        env: &mut Env,
    ) -> Result<crate::CoreExpr, Error> {
        let binds = names
            .iter()
            .map(|n| (n.clone(), self.fresh(n)))
            .collect::<HashMap<_, _>>();
        let params = names.iter().map(|n| binds[n].clone()).collect();
        let mut child = env.child(binds);
        Ok(crate::CoreExpr::Call {
            procedure: Box::new(crate::CoreExpr::Lambda {
                params,
                body: Box::new(self.body_with_definitions(body, &mut child)?),
            }),
            arguments: values,
        })
    }

    pub(super) fn cond(
        &mut self,
        clauses: &[Form],
        env: &mut Env,
    ) -> Result<crate::CoreExpr, Error> {
        self.cond_with_fallback(clauses, env, crate::CoreExpr::Literal(Value::unspecified()))
    }

    pub(super) fn cond_with_fallback(
        &mut self,
        clauses: &[Form],
        env: &mut Env,
        fallback: crate::CoreExpr,
    ) -> Result<crate::CoreExpr, Error> {
        if clauses.is_empty() {
            return Ok(fallback);
        }
        let clause = clauses[0]
            .proper_list()
            .ok_or_else(|| self.error(clauses[0].span, "cond clause must be a proper list"))?;
        if clause.is_empty() {
            return Err(self.error(clauses[0].span, "cond clause cannot be empty"));
        }
        if clause[0].symbol() == Some("else") && !env.keyword_shadowed("else") {
            if clauses.len() != 1 {
                return Err(self.error(clauses[0].span, "else must be the final cond clause"));
            }
            return self.body(&clause[1..], env);
        }
        let test = self.expr(&clause[0], env)?;
        let next = self.cond_with_fallback(&clauses[1..], env, fallback)?;
        let temp = self.fresh("cond");
        let yes = if clause.len() == 1 {
            crate::CoreExpr::Variable(temp.clone())
        } else if clause.len() == 3
            && clause[1].symbol() == Some("=>")
            && !env.keyword_shadowed("=>")
        {
            crate::CoreExpr::Call {
                procedure: Box::new(self.expr(&clause[2], env)?),
                arguments: vec![crate::CoreExpr::Variable(temp.clone())],
            }
        } else {
            self.body(&clause[1..], env)?
        };
        Ok(crate::CoreExpr::Call {
            procedure: Box::new(crate::CoreExpr::Lambda {
                params: vec![temp.clone()],
                body: Box::new(crate::CoreExpr::If(
                    Box::new(crate::CoreExpr::Variable(temp)),
                    Box::new(yes),
                    Box::new(next),
                )),
            }),
            arguments: vec![test],
        })
    }

    pub(super) fn guard(
        &mut self,
        xs: &[Form],
        env: &mut Env,
        span: Span,
    ) -> Result<crate::CoreExpr, Error> {
        if xs.len() < 2 {
            return Err(self.error(span, "guard requires clauses and a body"));
        }
        let spec = xs[0]
            .proper_list()
            .ok_or_else(|| self.error(xs[0].span, "guard clause list must be proper"))?;
        let (variable, clauses) = spec
            .split_first()
            .ok_or_else(|| self.error(xs[0].span, "guard requires a condition variable"))?;
        let variable = variable.symbol().ok_or_else(|| {
            self.error(
                variable.span,
                "guard condition variable must be an identifier",
            )
        })?;
        let condition = self.fresh(variable);
        let escape = self.fresh("guard-k");
        let mut binding = HashMap::new();
        binding.insert(variable.to_owned(), condition.clone());
        let mut handler_env = env.child(binding);
        let fallback = crate::CoreExpr::Raise {
            object: Box::new(crate::CoreExpr::Variable(condition.clone())),
            continuable: true,
        };
        let dispatch = self.cond_with_fallback(clauses, &mut handler_env, fallback)?;
        let handler = crate::CoreExpr::Lambda {
            params: vec![condition],
            body: Box::new(crate::CoreExpr::CallWithValues {
                producer: Box::new(crate::CoreExpr::Lambda {
                    params: Vec::new(),
                    body: Box::new(dispatch),
                }),
                consumer: Box::new(crate::CoreExpr::Variable(escape.clone())),
            }),
        };
        let thunk = crate::CoreExpr::Lambda {
            params: Vec::new(),
            body: Box::new(self.body(&xs[1..], env)?),
        };
        Ok(crate::CoreExpr::CallWithCurrentContinuation(Box::new(
            crate::CoreExpr::Lambda {
                params: vec![escape],
                body: Box::new(crate::CoreExpr::WithExceptionHandler {
                    handler: Box::new(handler),
                    thunk: Box::new(thunk),
                }),
            },
        )))
    }

    pub(super) fn case_form(
        &mut self,
        xs: &[Form],
        env: &mut Env,
        span: Span,
    ) -> Result<crate::CoreExpr, Error> {
        if xs.len() < 2 {
            return Err(self.error(span, "case requires a key and clauses"));
        }
        let key = self.expr(&xs[0], env)?;
        let temp = self.fresh("case");
        let result = self.case_clauses(&xs[1..], env, &temp)?;
        Ok(crate::CoreExpr::Call {
            procedure: Box::new(crate::CoreExpr::Lambda {
                params: vec![temp],
                body: Box::new(result),
            }),
            arguments: vec![key],
        })
    }

    pub(super) fn case_clauses(
        &mut self,
        clauses: &[Form],
        env: &mut Env,
        key: &str,
    ) -> Result<crate::CoreExpr, Error> {
        if clauses.is_empty() {
            return Ok(crate::CoreExpr::Literal(Value::unspecified()));
        }
        let clause = clauses[0]
            .proper_list()
            .ok_or_else(|| self.error(clauses[0].span, "case clause must be a proper list"))?;
        if clause.is_empty() {
            return Err(self.error(clauses[0].span, "case clause cannot be empty"));
        }
        if clause[0].symbol() == Some("else") && !env.keyword_shadowed("else") {
            if clauses.len() != 1 {
                return Err(self.error(clauses[0].span, "else must be the final case clause"));
            }
            if clause.len() == 3 && clause[1].symbol() == Some("=>") && !env.keyword_shadowed("=>")
            {
                return Ok(crate::CoreExpr::Call {
                    procedure: Box::new(self.expr(&clause[2], env)?),
                    arguments: vec![crate::CoreExpr::Variable(key.to_owned())],
                });
            }
            return self.body(&clause[1..], env);
        }
        let datums = clause[0]
            .proper_list()
            .ok_or_else(|| self.error(clause[0].span, "case datum list must be proper"))?;
        let tests = datums
            .iter()
            .map(|d| {
                Ok(call(
                    "eqv?",
                    vec![crate::CoreExpr::Variable(key.to_owned()), literal(d)?],
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let test = self.or_core(tests);
        let yes =
            if clause.len() == 3 && clause[1].symbol() == Some("=>") && !env.keyword_shadowed("=>")
            {
                crate::CoreExpr::Call {
                    procedure: Box::new(self.expr(&clause[2], env)?),
                    arguments: vec![crate::CoreExpr::Variable(key.to_owned())],
                }
            } else {
                self.body(&clause[1..], env)?
            };
        let no = self.case_clauses(&clauses[1..], env, key)?;
        Ok(crate::CoreExpr::If(
            Box::new(test),
            Box::new(yes),
            Box::new(no),
        ))
    }

    pub(super) fn or_core(&self, mut values: Vec<crate::CoreExpr>) -> crate::CoreExpr {
        if values.is_empty() {
            return crate::CoreExpr::Literal(Value::boolean(false));
        }
        let first = values.remove(0);
        if values.is_empty() {
            first
        } else {
            crate::CoreExpr::If(
                Box::new(first),
                Box::new(crate::CoreExpr::Literal(Value::boolean(true))),
                Box::new(self.or_core(values)),
            )
        }
    }

    pub(super) fn quasiquote(
        &mut self,
        form: &Form,
        env: &mut Env,
        depth: usize,
    ) -> Result<crate::CoreExpr, Error> {
        if let Some(items) = form.proper_list() {
            if items.first().and_then(Form::symbol) == Some("unquote") {
                if items.len() != 2 {
                    return Err(self.error(form.span, "unquote requires one expression"));
                }
                if depth == 1 {
                    return self.expr(&items[1], env);
                }
                return self.quasiquote_tagged("unquote", &items[1], env, depth - 1);
            }
            if items.first().and_then(Form::symbol) == Some("unquote-splicing") {
                if items.len() != 2 {
                    return Err(self.error(form.span, "unquote-splicing requires one expression"));
                }
                if depth == 1 {
                    return Err(self.error(
                        form.span,
                        "unquote-splicing is valid only in a list or vector template",
                    ));
                }
                return self.quasiquote_tagged("unquote-splicing", &items[1], env, depth - 1);
            }
            if items.first().and_then(Form::symbol) == Some("quasiquote") {
                if items.len() != 2 {
                    return Err(self.error(form.span, "quasiquote requires one expression"));
                }
                return self.quasiquote_tagged("quasiquote", &items[1], env, depth + 1);
            }
        }
        match &form.kind {
            FormKind::List(items, tail) => {
                let mut result = match tail {
                    Some(tail) => self.quasiquote(tail, env, depth)?,
                    None => crate::CoreExpr::Literal(Value::nil()),
                };
                for item in items.iter().rev() {
                    if depth == 1
                        && let Some(splice) = item.proper_list()
                        && splice.first().and_then(Form::symbol) == Some("unquote-splicing")
                    {
                        if splice.len() != 2 {
                            return Err(
                                self.error(item.span, "unquote-splicing requires one expression")
                            );
                        }
                        result = call("append", vec![self.expr(&splice[1], env)?, result]);
                    } else {
                        result = call("cons", vec![self.quasiquote(item, env, depth)?, result]);
                    }
                }
                Ok(result)
            }
            FormKind::Vector(items) => {
                let list = Form {
                    kind: FormKind::List(items.clone(), None),
                    span: form.span,
                };
                Ok(call(
                    "list->vector",
                    vec![self.quasiquote(&list, env, depth)?],
                ))
            }
            _ => self.quote(form),
        }
    }

    pub(super) fn quasiquote_tagged(
        &mut self,
        tag: &str,
        value: &Form,
        env: &mut Env,
        depth: usize,
    ) -> Result<crate::CoreExpr, Error> {
        let tag = self.quote(&Form {
            kind: FormKind::Symbol(tag.into()),
            span: value.span,
        })?;
        let value = self.quasiquote(value, env, depth)?;
        Ok(call(
            "cons",
            vec![
                tag,
                call("cons", vec![value, crate::CoreExpr::Literal(Value::nil())]),
            ],
        ))
    }
}
