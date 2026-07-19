//! The `syntax-rules` macro subsystem: pattern parsing and validation, matching,
//! and template expansion.

use super::{helpers::*, *};

pub(super) fn parse_macro(
    form: &Form,
    e: &Expander<'_>,
    definition_env: &Env,
    binding: String,
) -> Result<Macro, Error> {
    let xs = form
        .proper_list()
        .ok_or_else(|| e.error(form.span, "syntax-rules must be a list"))?;
    if xs.len() < 3 || xs[0].symbol() != Some("syntax-rules") {
        return Err(e.error(form.span, "only syntax-rules transformers are supported"));
    }
    let (ellipsis, literals_index, rules_index) = if xs[1].symbol().is_some() {
        if xs.len() < 4 {
            return Err(e.error(form.span, "syntax-rules requires at least one rule"));
        }
        (xs[1].symbol().unwrap().to_owned(), 2, 3)
    } else {
        ("...".to_owned(), 1, 2)
    };
    let literal_forms = match &xs[literals_index].kind {
        FormKind::Nil => &[][..],
        _ => xs[literals_index].proper_list().ok_or_else(|| {
            e.error(
                xs[literals_index].span,
                "syntax-rules literals must be a proper list",
            )
        })?,
    };
    let mut literals = HashSet::new();
    for literal in literal_forms {
        let name = literal
            .symbol()
            .ok_or_else(|| e.error(literal.span, "syntax-rules literal must be an identifier"))?;
        if !literals.insert(name.to_owned()) {
            return Err(e.error(literal.span, "duplicate syntax-rules literal"));
        }
    }
    let mut rules = Vec::new();
    for r in &xs[rules_index..] {
        let p = r
            .proper_list()
            .ok_or_else(|| e.error(r.span, "syntax-rules rule must be a pair"))?;
        if p.len() != 2 {
            return Err(e.error(
                r.span,
                "syntax-rules rule must contain pattern and template",
            ));
        }
        let mut pattern = p[0].clone();
        let FormKind::List(pattern_items, _) = &mut pattern.kind else {
            return Err(e.error(pattern.span, "syntax-rules pattern must be a list"));
        };
        let Some(keyword) = pattern_items.first_mut() else {
            return Err(e.error(pattern.span, "syntax-rules pattern must not be empty"));
        };
        if keyword.symbol().is_none() {
            return Err(e.error(
                keyword.span,
                "syntax-rules pattern must begin with an identifier",
            ));
        }
        keyword.kind = FormKind::Symbol("#pattern-keyword#".to_owned());
        validate_pattern(&pattern, &literals, &ellipsis, e)?;
        rules.push(Rule {
            pattern: Rc::new(pattern),
            template: Rc::new(p[1].clone()),
        })
    }
    Ok(Macro {
        literals,
        rules,
        definition_env: definition_env.clone(),
        ellipsis,
        binding,
    })
}

pub(super) fn validate_pattern(
    pattern: &Form,
    literals: &HashSet<String>,
    ellipsis: &str,
    e: &Expander<'_>,
) -> Result<(), Error> {
    fn visit(
        pattern: &Form,
        literals: &HashSet<String>,
        ellipsis: &str,
        variables: &mut HashSet<String>,
        e: &Expander<'_>,
    ) -> Result<(), Error> {
        match &pattern.kind {
            FormKind::Symbol(name) => {
                let name = canonical_identifier(name);
                if name != "#pattern-keyword#"
                    && name != "_"
                    && name != ellipsis
                    && !literals.contains(name)
                    && !variables.insert(name.to_owned())
                {
                    return Err(e.error(
                        pattern.span,
                        format!("pattern variable '{name}' appears more than once"),
                    ));
                }
            }
            FormKind::List(items, tail) => {
                validate_pattern_sequence(items, literals, ellipsis, e)?;
                for item in items.iter() {
                    if item.symbol() != Some(ellipsis) || literals.contains(ellipsis) {
                        visit(item, literals, ellipsis, variables, e)?;
                    }
                }
                if let Some(tail) = tail {
                    visit(tail, literals, ellipsis, variables, e)?;
                }
            }
            FormKind::Vector(items) => {
                validate_pattern_sequence(items, literals, ellipsis, e)?;
                for item in items.iter() {
                    if item.symbol() != Some(ellipsis) || literals.contains(ellipsis) {
                        visit(item, literals, ellipsis, variables, e)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    let mut variables = HashSet::new();
    visit(pattern, literals, ellipsis, &mut variables, e)
}

pub(super) fn validate_pattern_sequence(
    items: &[Form],
    literals: &HashSet<String>,
    ellipsis: &str,
    e: &Expander<'_>,
) -> Result<(), Error> {
    if literals.contains(ellipsis) {
        return Ok(());
    }
    let positions = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| (item.symbol() == Some(ellipsis)).then_some(index))
        .collect::<Vec<_>>();
    if positions.len() > 1
        || positions.first() == Some(&0)
        || positions.first().is_some_and(|position| {
            items
                .get(position - 1)
                .and_then(Form::symbol)
                .is_some_and(|name| name == "#pattern-keyword#")
        })
    {
        let span = positions
            .first()
            .and_then(|index| items.get(*index))
            .map_or_else(|| items[0].span, |item| item.span);
        return Err(e.error(span, "invalid ellipsis placement in syntax-rules pattern"));
    }
    Ok(())
}

pub(super) fn apply_macro(
    m: &Macro,
    input: &Form,
    use_env: &Env,
    e: &mut Expander<'_>,
) -> Result<Form, Error> {
    for r in &m.rules {
        let mut caps = HashMap::new();
        if matches_pattern(
            &r.pattern,
            input,
            &m.literals,
            &m.ellipsis,
            &m.definition_env,
            use_env,
            &mut caps,
        ) {
            return expand_template(
                &r.template,
                &caps,
                e,
                &m.definition_env,
                &m.ellipsis,
                !m.literals.contains(&m.ellipsis),
            );
        }
    }
    Err(e.error(
        input.span,
        format!("no syntax-rules pattern matched this use: {:?}", input.kind),
    ))
}

pub(super) fn matches_pattern(
    p: &Form,
    x: &Form,
    literals: &HashSet<String>,
    ellipsis: &str,
    definition_env: &Env,
    use_env: &Env,
    c: &mut HashMap<String, Capture>,
) -> bool {
    match (&p.kind, &x.kind) {
        (FormKind::Symbol(n), _) if n == "#pattern-keyword#" => true,
        (FormKind::Symbol(n), _) if literals.contains(canonical_identifier(n)) => {
            let Some(input_name) = x.symbol() else {
                return false;
            };
            definition_env.resolve(canonical_identifier(n))
                == use_env.resolve(canonical_identifier(input_name))
        }
        (FormKind::Symbol(n), _) if canonical_identifier(n) == "_" => true,
        (FormKind::Symbol(n), _) => match c.get(canonical_identifier(n)) {
            Some(Capture::One(v)) => same(v, x),
            Some(Capture::Many(_)) => false,
            None => {
                c.insert(
                    canonical_identifier(n).to_owned(),
                    Capture::One(Rc::new(x.clone())),
                );
                true
            }
        },
        (FormKind::List(..), FormKind::Nil) => {
            // An empty input list is `Nil`, but a `(x ...)` pattern must still
            // match it as zero repetitions. Reuse the list path with no elements.
            matches_pattern(
                p,
                &Form {
                    kind: FormKind::List(Vec::new(), None),
                    span: x.span,
                },
                literals,
                ellipsis,
                definition_env,
                use_env,
                c,
            )
        }
        (FormKind::List(ps, pattern_tail), FormKind::List(xs, input_tail)) => {
            if !literals.contains(ellipsis)
                && let Some(position) = ps.iter().position(|item| item.symbol() == Some(ellipsis))
            {
                if position == 0 {
                    return false;
                }
                let repeated = &ps[position - 1];
                let prefix = &ps[..position - 1];
                let suffix = &ps[position + 1..];
                if xs.len() < prefix.len() + suffix.len() {
                    return false;
                }
                for (a, b) in prefix.iter().zip(&xs[..prefix.len()]) {
                    if !matches_pattern(a, b, literals, ellipsis, definition_env, use_env, c) {
                        return false;
                    }
                }
                let suffix_start = xs.len() - suffix.len();
                for (a, b) in suffix.iter().zip(&xs[suffix_start..]) {
                    if !matches_pattern(a, b, literals, ellipsis, definition_env, use_env, c) {
                        return false;
                    }
                }
                let mut repeated_names = Vec::new();
                pattern_variables(repeated, literals, ellipsis, &mut repeated_names);
                for name in repeated_names {
                    c.entry(name).or_insert_with(|| Capture::Many(Vec::new()));
                }
                let mut repetition = HashMap::new();
                for value in &xs[prefix.len()..suffix_start] {
                    repetition.clear();
                    if !matches_pattern(
                        repeated,
                        value,
                        literals,
                        ellipsis,
                        definition_env,
                        use_env,
                        &mut repetition,
                    ) || !merge_repetition(c, &mut repetition)
                    {
                        return false;
                    }
                }
                match (pattern_tail.as_deref(), input_tail.as_deref()) {
                    (None, None) => true,
                    (Some(pattern), Some(input)) => matches_pattern(
                        pattern,
                        input,
                        literals,
                        ellipsis,
                        definition_env,
                        use_env,
                        c,
                    ),
                    (Some(pattern), None) => matches_pattern(
                        pattern,
                        &Form {
                            kind: FormKind::Nil,
                            span: x.span,
                        },
                        literals,
                        ellipsis,
                        definition_env,
                        use_env,
                        c,
                    ),
                    _ => false,
                }
            } else {
                if xs.len() < ps.len()
                    || (pattern_tail.is_none() && (xs.len() != ps.len() || input_tail.is_some()))
                    || !ps.iter().zip(xs.iter()).all(|(a, b)| {
                        matches_pattern(a, b, literals, ellipsis, definition_env, use_env, c)
                    })
                {
                    false
                } else if let Some(pattern_tail) = pattern_tail.as_deref() {
                    let remainder = if xs.len() == ps.len() {
                        input_tail.as_deref().cloned().unwrap_or(Form {
                            kind: FormKind::Nil,
                            span: x.span,
                        })
                    } else {
                        Form {
                            kind: FormKind::List(xs[ps.len()..].to_vec(), input_tail.clone()),
                            span: x.span,
                        }
                    };
                    matches_pattern(
                        pattern_tail,
                        &remainder,
                        literals,
                        ellipsis,
                        definition_env,
                        use_env,
                        c,
                    )
                } else {
                    true
                }
            }
        }
        (FormKind::Vector(patterns), FormKind::Vector(inputs)) => matches_vector_pattern(
            patterns,
            inputs,
            literals,
            ellipsis,
            definition_env,
            use_env,
            c,
        ),
        _ => same(p, x),
    }
}

pub(super) fn matches_vector_pattern(
    patterns: &[Form],
    inputs: &[Form],
    literals: &HashSet<String>,
    ellipsis: &str,
    definition_env: &Env,
    use_env: &Env,
    captures: &mut HashMap<String, Capture>,
) -> bool {
    let repeated = (!literals.contains(ellipsis))
        .then(|| {
            patterns
                .iter()
                .position(|item| item.symbol() == Some(ellipsis))
        })
        .flatten();
    let Some(position) = repeated else {
        return patterns.len() == inputs.len()
            && patterns.iter().zip(inputs).all(|(pattern, input)| {
                matches_pattern(
                    pattern,
                    input,
                    literals,
                    ellipsis,
                    definition_env,
                    use_env,
                    captures,
                )
            });
    };
    if position == 0 {
        return false;
    }
    let repeated = &patterns[position - 1];
    let prefix = &patterns[..position - 1];
    let suffix = &patterns[position + 1..];
    if inputs.len() < prefix.len() + suffix.len() {
        return false;
    }
    if !prefix.iter().zip(inputs).all(|(pattern, input)| {
        matches_pattern(
            pattern,
            input,
            literals,
            ellipsis,
            definition_env,
            use_env,
            captures,
        )
    }) {
        return false;
    }
    let suffix_start = inputs.len() - suffix.len();
    if !suffix
        .iter()
        .zip(&inputs[suffix_start..])
        .all(|(pattern, input)| {
            matches_pattern(
                pattern,
                input,
                literals,
                ellipsis,
                definition_env,
                use_env,
                captures,
            )
        })
    {
        return false;
    }
    let mut repeated_names = Vec::new();
    pattern_variables(repeated, literals, ellipsis, &mut repeated_names);
    for name in repeated_names {
        captures
            .entry(name)
            .or_insert_with(|| Capture::Many(Vec::new()));
    }
    let mut repetition = HashMap::new();
    for input in &inputs[prefix.len()..suffix_start] {
        repetition.clear();
        if !matches_pattern(
            repeated,
            input,
            literals,
            ellipsis,
            definition_env,
            use_env,
            &mut repetition,
        ) || !merge_repetition(captures, &mut repetition)
        {
            return false;
        }
    }
    true
}

pub(super) fn canonical_identifier(name: &str) -> &str {
    super::strip_hygiene(name)
}

pub(super) fn pattern_variables(
    pattern: &Form,
    literals: &HashSet<String>,
    ellipsis: &str,
    output: &mut Vec<String>,
) {
    match &pattern.kind {
        FormKind::Symbol(name)
            if canonical_identifier(name) != "_"
                && canonical_identifier(name) != ellipsis
                && !literals.contains(canonical_identifier(name)) =>
        {
            let name = canonical_identifier(name).to_owned();
            if !output.contains(&name) {
                output.push(name);
            }
        }
        FormKind::List(items, tail) => {
            for item in items.iter() {
                pattern_variables(item, literals, ellipsis, output);
            }
            if let Some(tail) = tail {
                pattern_variables(tail, literals, ellipsis, output);
            }
        }
        FormKind::Vector(items) => {
            for item in items.iter() {
                pattern_variables(item, literals, ellipsis, output);
            }
        }
        _ => {}
    }
}

pub(super) fn merge_repetition(
    captures: &mut HashMap<String, Capture>,
    repetition: &mut HashMap<String, Capture>,
) -> bool {
    for (name, value) in repetition.drain() {
        match captures.entry(name) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Capture::Many(vec![value]));
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => match entry.get_mut() {
                Capture::Many(values) => values.push(value),
                Capture::One(_) => return false,
            },
        }
    }
    true
}

pub(super) fn expand_template(
    t: &Form,
    c: &HashMap<String, Capture>,
    e: &mut Expander<'_>,
    definition_env: &Env,
    ellipsis: &str,
    ellipsis_active: bool,
) -> Result<Form, Error> {
    let mark = e.mark();
    let mut repetition_path = Vec::new();
    expand_template_mode(
        t,
        c,
        e,
        definition_env,
        ellipsis,
        ellipsis_active,
        true,
        mark,
        &mut repetition_path,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn expand_template_mode(
    t: &Form,
    c: &HashMap<String, Capture>,
    e: &mut Expander<'_>,
    definition_env: &Env,
    ellipsis: &str,
    ellipsis_active: bool,
    resolve_free: bool,
    mark: u64,
    repetition_path: &mut Vec<usize>,
) -> Result<Form, Error> {
    if let Some(n) = t.symbol().map(canonical_identifier) {
        return Ok(
            match c
                .get(n)
                .and_then(|capture| capture_at(capture, repetition_path))
            {
                Some(Capture::One(v)) => v.as_ref().clone(),
                Some(Capture::Many(_)) => {
                    return Err(e.error(
                        t.span,
                        "pattern variable requires another template ellipsis",
                    ));
                }
                None if template_keyword(n) && resolve_free => Form {
                    kind: FormKind::Symbol(format!("#syntax#{n}")),
                    span: t.span,
                },
                None if !resolve_free => t.clone(),
                None => Form {
                    kind: FormKind::Symbol(format!(
                        "#resolved#{mark}#{}",
                        definition_env.resolve(n)
                    )),
                    span: t.span,
                },
            },
        );
    }
    match &t.kind {
        FormKind::List(ts, None) if ts.len() == 2 && ts[0].symbol() == Some("quote") => {
            let mut quoted = expand_template_mode(
                &ts[1],
                c,
                e,
                definition_env,
                ellipsis,
                ellipsis_active,
                false,
                mark,
                repetition_path,
            )?;
            protect_literal_identifiers(&mut quoted);
            Ok(Form {
                kind: FormKind::List(
                    vec![
                        Form {
                            kind: FormKind::Symbol("#syntax#quote".to_owned()),
                            span: ts[0].span,
                        },
                        quoted,
                    ],
                    None,
                ),
                span: t.span,
            })
        }
        FormKind::List(ts, None)
            if ts.len() == 2 && ts[0].symbol() == Some(ellipsis) && ellipsis_active =>
        {
            expand_template_mode(
                &ts[1],
                c,
                e,
                definition_env,
                ellipsis,
                false,
                false,
                mark,
                repetition_path,
            )
        }
        FormKind::List(ts, tail) => {
            let out = expand_template_sequence(
                ts,
                c,
                e,
                definition_env,
                ellipsis,
                ellipsis_active,
                resolve_free,
                mark,
                repetition_path,
            )?;
            let tail = tail
                .as_deref()
                .map(|tail| {
                    expand_template_mode(
                        tail,
                        c,
                        e,
                        definition_env,
                        ellipsis,
                        ellipsis_active,
                        resolve_free,
                        mark,
                        repetition_path,
                    )
                    .map(Box::new)
                })
                .transpose()?;
            // An ellipsis that expanded to nothing leaves a proper list with no
            // elements. The reader represents such an empty list as `Nil`, so
            // normalize to the same shape here. Otherwise this template-built
            // `()` would not match a literal `()` pattern in a recursive
            // expansion and would lower differently from a reader `()`. A `( . x)`
            // form with an empty prefix is just its tail.
            let kind = match (out.is_empty(), tail) {
                (true, None) => FormKind::Nil,
                (true, Some(tail)) => tail.kind,
                (false, tail) => FormKind::List(out, tail),
            };
            Ok(Form { kind, span: t.span })
        }
        FormKind::Vector(ts) => Ok(Form {
            kind: FormKind::Vector(expand_template_sequence(
                ts,
                c,
                e,
                definition_env,
                ellipsis,
                ellipsis_active,
                resolve_free,
                mark,
                repetition_path,
            )?),
            span: t.span,
        }),
        _ => Ok(t.clone()),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn expand_template_sequence(
    templates: &[Form],
    captures: &HashMap<String, Capture>,
    e: &mut Expander<'_>,
    definition_env: &Env,
    ellipsis: &str,
    ellipsis_active: bool,
    resolve_free: bool,
    mark: u64,
    repetition_path: &mut Vec<usize>,
) -> Result<Vec<Form>, Error> {
    let mut output = Vec::new();
    let mut index = 0;
    while index < templates.len() {
        if ellipsis_active
            && index + 1 < templates.len()
            && templates[index + 1].symbol() == Some(ellipsis)
        {
            let count =
                template_repetition_count(&templates[index], captures, repetition_path, ellipsis)?
                    .ok_or_else(|| {
                        e.error(
                            templates[index].span,
                            "repeated template contains no pattern variable at this depth",
                        )
                    })?;
            for repetition in 0..count {
                repetition_path.push(repetition);
                let expanded = expand_template_mode(
                    &templates[index],
                    captures,
                    e,
                    definition_env,
                    ellipsis,
                    ellipsis_active,
                    resolve_free,
                    mark,
                    repetition_path,
                );
                repetition_path.pop();
                output.push(expanded?);
            }
            index += 2;
        } else {
            output.push(expand_template_mode(
                &templates[index],
                captures,
                e,
                definition_env,
                ellipsis,
                ellipsis_active,
                resolve_free,
                mark,
                repetition_path,
            )?);
            index += 1;
        }
    }
    Ok(output)
}

pub(super) fn capture_at<'a>(capture: &'a Capture, path: &[usize]) -> Option<&'a Capture> {
    let mut capture = capture;
    for index in path {
        let Capture::Many(values) = capture else {
            return None;
        };
        capture = values.get(*index)?;
    }
    Some(capture)
}

pub(super) fn template_repetition_count(
    template: &Form,
    captures: &HashMap<String, Capture>,
    path: &[usize],
    ellipsis: &str,
) -> Result<Option<usize>, Error> {
    fn visit(
        template: &Form,
        captures: &HashMap<String, Capture>,
        path: &[usize],
        ellipsis: &str,
        count: &mut Option<usize>,
    ) -> Result<(), Error> {
        if let Some(name) = template.symbol().map(canonical_identifier)
            && name != ellipsis
            && let Some(Capture::Many(values)) = captures
                .get(name)
                .and_then(|capture| capture_at(capture, path))
        {
            if count.is_some_and(|count| count != values.len()) {
                return Err(Error::plain(
                    ErrorKind::ExpandError,
                    "repeated template variables have incompatible lengths",
                ));
            }
            *count = Some(values.len());
        }
        match &template.kind {
            FormKind::List(items, tail) => {
                for item in items.iter() {
                    visit(item, captures, path, ellipsis, count)?;
                }
                if let Some(tail) = tail {
                    visit(tail, captures, path, ellipsis, count)?;
                }
            }
            FormKind::Vector(items) => {
                for item in items.iter() {
                    visit(item, captures, path, ellipsis, count)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    let mut count = None;
    visit(template, captures, path, ellipsis, &mut count)?;
    Ok(count)
}

pub(super) fn template_keyword(name: &str) -> bool {
    matches!(
        name,
        "_" | "..."
            | "quote"
            | "quasiquote"
            | "unquote"
            | "unquote-splicing"
            | "if"
            | "begin"
            | "lambda"
            | "set!"
            | "define"
            | "define-values"
            | "define-syntax"
            | "define-record-type"
            | "syntax-rules"
            | "syntax-error"
            | "and"
            | "or"
            | "when"
            | "unless"
            | "let"
            | "let*"
            | "letrec"
            | "letrec*"
            | "let-values"
            | "let*-values"
            | "let-syntax"
            | "letrec-syntax"
            | "cond"
            | "case"
            | "else"
            | "=>"
            | "do"
            | "delay"
            | "delay-force"
            | "parameterize"
            | "guard"
            | "case-lambda"
            | "error"
            | "raise"
            | "raise-continuable"
            | "with-exception-handler"
            | "call-with-current-continuation"
            | "call/cc"
            | "dynamic-wind"
            | "values"
            | "call-with-values"
            | "make-parameter"
            | "force"
            | "eval"
    )
}

pub(super) fn protect_literal_identifiers(form: &mut Form) {
    match &mut form.kind {
        FormKind::Symbol(name) if !name.starts_with('#') => {
            *name = format!("#literal#{name}");
        }
        FormKind::List(items, tail) => {
            for item in items {
                protect_literal_identifiers(item);
            }
            if let Some(tail) = tail.as_mut() {
                protect_literal_identifiers(tail);
            }
        }
        FormKind::Vector(items) => {
            for item in items {
                protect_literal_identifiers(item);
            }
        }
        _ => {}
    }
}
