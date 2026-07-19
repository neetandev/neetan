//! Form-construction helpers and structural/number equality used across the
//! expander and macro subsystem.

use super::*;

pub(super) fn one<T>(
    items: &[Form],
    span: Span,
    e: &mut Expander<'_>,
    f: impl FnOnce(&Form, &mut Expander<'_>) -> Result<T, Error>,
) -> Result<T, Error> {
    if items.len() != 2 {
        Err(e.error(span, "form requires exactly one argument"))
    } else {
        f(&items[1], e)
    }
}

pub(super) fn list_symbols(f: &Form, e: &Expander<'_>) -> Result<Vec<String>, Error> {
    let (required, rest) = formals(f, e)?;
    if rest.is_some() {
        return Err(e.error(f.span, "formals must be a proper identifier list"));
    }
    Ok(required)
}

pub(super) fn formals(f: &Form, e: &Expander<'_>) -> Result<(Vec<String>, Option<String>), Error> {
    let (xs, rest) = match &f.kind {
        FormKind::Nil => (&[][..], None),
        FormKind::Symbol(name) => return Ok((Vec::new(), Some(name.clone()))),
        FormKind::List(xs, tail) => (xs.as_slice(), tail.as_deref()),
        _ => return Err(e.error(f.span, "invalid procedure formals")),
    };
    let mut seen = HashSet::new();
    let required = xs
        .iter()
        .map(|x| {
            let n = x
                .symbol()
                .ok_or_else(|| e.error(x.span, "formal must be an identifier"))?;
            if !seen.insert(n.to_owned()) {
                return Err(e.error(x.span, "duplicate formal"));
            }
            Ok(n.to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let rest = rest
        .map(|tail| {
            let name = tail
                .symbol()
                .ok_or_else(|| e.error(tail.span, "rest formal must be an identifier"))?;
            if !seen.insert(name.to_owned()) {
                return Err(e.error(tail.span, "duplicate formal"));
            }
            Ok(name.to_owned())
        })
        .transpose()?;
    Ok((required, rest))
}

pub(super) fn bindings(f: &Form, e: &Expander<'_>) -> Result<Vec<(String, Form)>, Error> {
    let xs = match &f.kind {
        FormKind::Nil => &[][..],
        _ => f
            .proper_list()
            .ok_or_else(|| e.error(f.span, "bindings must be a proper list"))?,
    };
    xs.iter()
        .map(|x| {
            let p = x
                .proper_list()
                .ok_or_else(|| e.error(x.span, "binding must be (identifier expression)"))?;
            if p.len() != 2 {
                return Err(e.error(x.span, "binding must be (identifier expression)"));
            }
            Ok((
                p[0].symbol()
                    .ok_or_else(|| e.error(p[0].span, "binding name must be an identifier"))?
                    .to_owned(),
                p[1].clone(),
            ))
        })
        .collect()
}

pub(super) fn literal(f: &Form) -> Result<crate::CoreExpr, Error> {
    match &f.kind {
        FormKind::Nil => Ok(crate::CoreExpr::Literal(Value::nil())),
        FormKind::Bool(v) => Ok(crate::CoreExpr::Literal(Value::boolean(*v))),
        FormKind::Char(v) => Ok(crate::CoreExpr::Literal(Value::character(*v))),
        FormKind::Number(Number::Real(Real::ExactInteger(v))) => {
            match i64::try_from(*v).ok().map(Value::integer) {
                Some(value) => Ok(crate::CoreExpr::Literal(value)),
                None => Ok(crate::CoreExpr::NumberLiteral(Number::Real(
                    Real::ExactInteger(*v),
                ))),
            }
        }
        FormKind::Number(Number::Real(Real::Inexact(v))) => {
            Ok(crate::CoreExpr::Literal(Value::float(*v)))
        }
        FormKind::Number(value) => Ok(crate::CoreExpr::NumberLiteral(*value)),
        FormKind::String(s) => Ok(call(
            "string",
            s.chars()
                .map(|c| crate::CoreExpr::Literal(Value::character(c)))
                .collect(),
        )),
        FormKind::Symbol(s) => Ok(call(
            "string->symbol",
            vec![literal(&Form {
                kind: FormKind::String(s.strip_prefix("#literal#").unwrap_or(s).to_owned()),
                span: f.span,
            })?],
        )),
        FormKind::Bytevector(v) => Ok(call(
            "bytevector",
            v.iter()
                .map(|x| crate::CoreExpr::Literal(Value::integer(i64::from(*x))))
                .collect(),
        )),
        FormKind::Vector(v) => Ok(call(
            "vector",
            v.iter().map(literal).collect::<Result<_, _>>()?,
        )),
        FormKind::List(v, tail) => {
            let mut out = match tail {
                Some(t) => literal(t)?,
                None => crate::CoreExpr::Literal(Value::nil()),
            };
            for x in v.iter().rev() {
                out = call("cons", vec![literal(x)?, out])
            }
            Ok(out)
        }
    }
}

/// Appends an injective serialization of a literal datum to `out`.
///
/// Hoisted literals are named by their content, so equal datums in any
/// compilation unit share one hidden global and distinct datums can never
/// collide: a colliding redefinition always rebinds an equal immutable
/// value. Every kind carries its own tag letter and every variable-length
/// field is length-prefixed, which keeps concatenation unambiguous.
pub(super) fn literal_key(f: &Form, out: &mut String) {
    use std::fmt::Write;
    match &f.kind {
        FormKind::Nil => out.push('e'),
        FormKind::Bool(v) => out.push(if *v { 't' } else { 'f' }),
        FormKind::Char(v) => {
            let _ = write!(out, "c{:x};", u32::from(*v));
        }
        FormKind::Number(v) => {
            let _ = write!(out, "n{v:?};");
        }
        FormKind::String(s) => {
            let _ = write!(out, "s{}:", s.len());
            out.push_str(s);
        }
        FormKind::Symbol(s) => {
            let _ = write!(out, "y{}:", s.len());
            out.push_str(s);
        }
        FormKind::Bytevector(v) => {
            let _ = write!(out, "u{}:", v.len());
            for byte in v {
                let _ = write!(out, "{byte:02x}");
            }
        }
        FormKind::Vector(v) => {
            let _ = write!(out, "v{}(", v.len());
            for x in v {
                literal_key(x, out);
            }
            out.push(')');
        }
        FormKind::List(v, tail) => {
            let _ = write!(out, "l{}(", v.len());
            for x in v {
                literal_key(x, out);
            }
            if let Some(t) = tail {
                out.push('.');
                literal_key(t, out);
            }
            out.push(')');
        }
    }
}

pub(super) fn call(name: &str, args: Vec<crate::CoreExpr>) -> crate::CoreExpr {
    crate::CoreExpr::Call {
        procedure: Box::new(crate::CoreExpr::Variable(name.into())),
        arguments: args,
    }
}

pub(super) fn same(a: &Form, b: &Form) -> bool {
    match (&a.kind, &b.kind) {
        (FormKind::Nil, FormKind::Nil) => true,
        (FormKind::Bool(a), FormKind::Bool(b)) => a == b,
        (FormKind::Char(a), FormKind::Char(b)) => a == b,
        (FormKind::String(a), FormKind::String(b)) => a == b,
        (FormKind::Number(a), FormKind::Number(b)) => same_number(*a, *b),
        (FormKind::Symbol(a), FormKind::Symbol(b)) => a == b,
        (FormKind::List(a, a_tail), FormKind::List(b, b_tail)) => {
            a.len() == b.len()
                && a.iter().zip(b).all(|(a, b)| same(a, b))
                && match (a_tail, b_tail) {
                    (None, None) => true,
                    (Some(a), Some(b)) => same(a, b),
                    _ => false,
                }
        }
        (FormKind::Vector(a), FormKind::Vector(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b))
        }
        (FormKind::Bytevector(a), FormKind::Bytevector(b)) => a == b,
        _ => false,
    }
}

pub(super) fn same_number(a: Number, b: Number) -> bool {
    fn real(a: Real, b: Real) -> bool {
        match (a, b) {
            (Real::Inexact(a), Real::Inexact(b)) => a.to_bits() == b.to_bits(),
            _ => a == b,
        }
    }
    match (a, b) {
        (Number::Real(a), Number::Real(b)) => real(a, b),
        (
            Number::Rectangular {
                real: ar,
                imaginary: ai,
            },
            Number::Rectangular {
                real: br,
                imaginary: bi,
            },
        ) => real(ar, br) && real(ai, bi),
        (
            Number::Polar {
                magnitude: am,
                angle: aa,
            },
            Number::Polar {
                magnitude: bm,
                angle: ba,
            },
        ) => real(am, bm) && real(aa, ba),
        _ => false,
    }
}
