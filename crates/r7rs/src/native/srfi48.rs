//! Native primitive backing the SRFI 48 (Intermediate Format Strings)
//! extension.
//!
//! SRFI 48 extends the SRFI 28 `format` with radix, character, fixed-width,
//! cycle-safe write, and indirection directives. The whole operation is
//! structural text work with no user callback, so it lives entirely in Rust.
//! The `~a`, `~s`, `~w`, and `~y` directives render their argument through the
//! same printer the built-in `display` and `write` procedures use, so the
//! output matches those procedures by construction. Two deliberate
//! simplifications the SRFI permits: `~y` pretty-prints as plain `write`, and
//! `~F` on very large or very small numbers uses fixed notation rather than
//! switching to exponential notation.
//!
//! Destination dispatch (the optional port argument of the public `format`)
//! lives in the `(srfi 48)` Scheme wrapper. This native always builds and
//! returns a string.

use super::{
    NativeContext, character,
    number::{format_real, format_runtime_number, number_argument},
    type_error,
};
use crate::{
    Error, ErrorKind, Value,
    number::{self, Real, RuntimeNumber},
    printer::{RuntimeWriteMode, write_value},
};

/// The upper bound for a `~F` field width or digit count. Larger values raise
/// a range error instead of allocating enormous padding.
const MAX_FIELD_WIDTH: usize = 65_535;

/// How deep `~?` indirection may nest. Templates arrive as data, so the bound
/// keeps self-referential inputs from recursing without limit.
const MAX_INDIRECTION_DEPTH: usize = 8;

/// The `~h` help text: the call synopsis and one line per supported directive,
/// mirroring the shape the SRFI shows.
const HELP_TEXT: &str = "\
(format [<port>] <format-string> [<arg>...]) -- <port> is #t, #f or an output-port
OPTION\t[MNEMONIC]\tDESCRIPTION\t-- This implementation uses Unicode text
~H\t[Help]\t\toutput this text
~A\t[Any]\t\t(display arg) for humans
~S\t[Slashified]\t(write arg) for parsers
~W\t[WriteCircular]\tlike ~S, but handles recursive structures
~D\t[Decimal]\tthe arg is a number which is output in decimal radix
~X\t[heXadecimal]\tthe arg is a number which is output in hexadecimal radix
~O\t[Octal]\t\tthe arg is a number which is output in octal radix
~B\t[Binary]\tthe arg is a number which is output in binary radix
~w,dF\t[Fixed]\t\tthe arg is a string or number which has width w and d digits after the decimal
~C\t[Character]\tcharacter arg is output by write-char
~_\t[Space]\t\ta single space character is output
~Y\t[Yuppify]\tthe list arg is pretty-printed to the output
~?\t[Indirection]\trecursive format: next arg is a format-string and the following arg a list of arguments
~K\t[Indirection]\tsame as ~?
~~\t[tilde]\t\toutput a tilde
~T\t[Tab]\t\toutput a tab character
~%\t[Newline]\toutput a newline character
~&\t[Freshline]\toutput a newline character if the previous output was not a newline
";

/// Implements the string-building core of SRFI 48 `format`: `args[0]` is the
/// template string and `args[1..]` are the substitution values, consumed left
/// to right by the directives that take one. The SRFI makes both leftover and
/// missing arguments an error.
///
/// The template is copied out of the heap up front so the accumulator can be
/// built without holding a borrow across the final string allocation.
/// Rendering borrows the heap immutably through `write_value`, which never
/// re-enters the VM, so no user code runs mid-format.
pub(crate) fn format(cx: &mut NativeContext<'_>, args: &[Value]) -> Result<Value, Error> {
    let template = cx
        .heap
        .string(args[0])
        .ok_or_else(|| type_error("string", args[0], cx.heap))?;

    let mut out = String::with_capacity(template.len());
    let consumed = format_into(cx, &template, &args[1..], &mut out, 0)?;
    if consumed != args.len() - 1 {
        return Err(Error::plain(
            ErrorKind::ArityError,
            "format: more arguments than the escape sequences in the template consume",
        ));
    }
    cx.string_utf8(out)
}

/// Walks one template, appending to `out`, and returns how many of `args` the
/// directives consumed. `depth` counts `~?` nesting.
fn format_into(
    cx: &NativeContext<'_>,
    template: &str,
    args: &[Value],
    out: &mut String,
    depth: usize,
) -> Result<usize, Error> {
    let mut next = 0usize;
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '~' {
            out.push(ch);
            continue;
        }
        let Some(directive) = chars.next() else {
            return Err(Error::plain(
                ErrorKind::RuntimeError,
                "format: incomplete escape sequence at end of template",
            ));
        };
        if directive.is_ascii_digit() {
            let (width, digits) = parse_field(&mut chars, directive)?;
            fixed(cx, args, &mut next, out, width, digits)?;
            continue;
        }
        match directive.to_ascii_lowercase() {
            'a' => render(cx, args, &mut next, out, RuntimeWriteMode::Display)?,
            's' => render(cx, args, &mut next, out, RuntimeWriteMode::Write)?,
            'w' => render(cx, args, &mut next, out, RuntimeWriteMode::Shared)?,
            // Pretty-printing as plain write is permitted by the SRFI.
            'y' => render(cx, args, &mut next, out, RuntimeWriteMode::Write)?,
            'c' => {
                let value = take_argument(args, &mut next)?;
                out.push(character(cx, value)?);
            }
            'd' => radix(cx, args, &mut next, out, 10)?,
            'x' => radix(cx, args, &mut next, out, 16)?,
            'o' => radix(cx, args, &mut next, out, 8)?,
            'b' => radix(cx, args, &mut next, out, 2)?,
            'f' => fixed(cx, args, &mut next, out, 0, None)?,
            '?' | 'k' => indirect(cx, args, &mut next, out, depth)?,
            '~' => out.push('~'),
            '%' => out.push('\n'),
            't' => out.push('\t'),
            '_' => out.push(' '),
            '&' => {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            'h' => out.push_str(HELP_TEXT),
            other => {
                return Err(Error::plain(
                    ErrorKind::RuntimeError,
                    format!("format: unrecognized escape sequence ~{other}"),
                ));
            }
        }
    }
    Ok(next)
}

/// Takes the next substitution value and advances the cursor. Errors if the
/// template asks for more values than were supplied.
fn take_argument(args: &[Value], next: &mut usize) -> Result<Value, Error> {
    let value = *args.get(*next).ok_or_else(|| {
        Error::plain(
            ErrorKind::ArityError,
            "format: not enough arguments for the escape sequences in the template",
        )
    })?;
    *next += 1;
    Ok(value)
}

/// Renders the next substitution value in `mode`.
fn render(
    cx: &NativeContext<'_>,
    args: &[Value],
    next: &mut usize,
    out: &mut String,
    mode: RuntimeWriteMode,
) -> Result<(), Error> {
    let value = take_argument(args, next)?;
    out.push_str(&write_value(cx.heap, value, mode)?);
    Ok(())
}

/// Renders the next substitution value, which must be a number, in `radix`.
fn radix(
    cx: &NativeContext<'_>,
    args: &[Value],
    next: &mut usize,
    out: &mut String,
    radix: u32,
) -> Result<(), Error> {
    let value = take_argument(args, next)?;
    out.push_str(&format_runtime_number(number_argument(cx, value)?, radix));
    Ok(())
}

/// Parses the rest of a `~w[,d]F` directive after its first width digit.
fn parse_field(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    first: char,
) -> Result<(usize, Option<usize>), Error> {
    let width = parse_field_number(chars, first.to_digit(10).expect("checked digit"))?;
    let mut digits = None;
    if chars.peek() == Some(&',') {
        chars.next();
        let leading = chars.next().and_then(|ch| ch.to_digit(10)).ok_or_else(|| {
            Error::plain(
                ErrorKind::RuntimeError,
                "format: expected a digit count after the comma in a ~F field",
            )
        })?;
        digits = Some(parse_field_number(chars, leading)?);
    }
    match chars.next() {
        Some('f' | 'F') => Ok((width, digits)),
        Some(other) => Err(Error::plain(
            ErrorKind::RuntimeError,
            format!("format: expected F to end a fixed-format field, found {other}"),
        )),
        None => Err(Error::plain(
            ErrorKind::RuntimeError,
            "format: incomplete escape sequence at end of template",
        )),
    }
}

/// Accumulates the remaining digits of a `~F` field number, bounded by
/// [`MAX_FIELD_WIDTH`].
fn parse_field_number(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    leading: u32,
) -> Result<usize, Error> {
    let mut value = leading as usize;
    while let Some(digit) = chars.peek().and_then(|ch| ch.to_digit(10)) {
        chars.next();
        value = value * 10 + digit as usize;
        if value > MAX_FIELD_WIDTH {
            return Err(Error::plain(
                ErrorKind::RangeError,
                format!("format: a ~F field bound must not exceed {MAX_FIELD_WIDTH}"),
            ));
        }
    }
    Ok(value)
}

/// Implements `~[w[,d]]F`. Strings pad on the left with spaces to `width` and
/// ignore `digits`. With `digits`, a number is coerced to inexact and rendered
/// with that many places after the decimal point, both parts of a complex
/// number included. Without `digits`, the ordinary `number->string` form is
/// padded. A rendering wider than `width` is emitted whole.
fn fixed(
    cx: &NativeContext<'_>,
    args: &[Value],
    next: &mut usize,
    out: &mut String,
    width: usize,
    digits: Option<usize>,
) -> Result<(), Error> {
    let value = take_argument(args, next)?;
    let text = if let Some(text) = cx.heap.string(value) {
        text
    } else {
        let number = number_argument(cx, value)
            .map_err(|_| type_error("string or number", value, cx.heap))?;
        match digits {
            None => format_runtime_number(number, 10),
            Some(digits) => fixed_number(number, digits),
        }
    };
    for _ in text.chars().count()..width {
        out.push(' ');
    }
    out.push_str(&text);
    Ok(())
}

/// Renders a number with a fixed count of decimal digits, following the sign
/// conventions of the ordinary complex printer.
fn fixed_number(value: RuntimeNumber, digits: usize) -> String {
    let (real, imaginary) = value.components();
    if number::is_zero(imaginary) {
        return fixed_f64(number::to_f64(real), digits);
    }
    let mut output = fixed_f64(number::to_f64(real), digits);
    let imaginary = fixed_f64(number::to_f64(imaginary), digits);
    if !imaginary.starts_with('-') && !imaginary.starts_with('+') {
        output.push('+');
    }
    output.push_str(&imaginary);
    output.push('i');
    output
}

/// Fixed-decimal rendering of one real component. Non-finite values keep
/// their Scheme spellings.
fn fixed_f64(value: f64, digits: usize) -> String {
    if value.is_finite() {
        format!("{value:.digits$}")
    } else {
        format_real(Real::Inexact(value), 10)
    }
}

/// Implements `~?` and `~K`: the next argument is a nested template and the
/// one after it is a proper list of that template's arguments.
fn indirect(
    cx: &NativeContext<'_>,
    args: &[Value],
    next: &mut usize,
    out: &mut String,
    depth: usize,
) -> Result<(), Error> {
    if depth >= MAX_INDIRECTION_DEPTH {
        return Err(Error::plain(
            ErrorKind::RuntimeError,
            format!("format: ~? indirection nested deeper than {MAX_INDIRECTION_DEPTH}"),
        ));
    }
    let template_value = take_argument(args, next)?;
    let template = cx
        .heap
        .string(template_value)
        .ok_or_else(|| type_error("string", template_value, cx.heap))?;
    let list_value = take_argument(args, next)?;
    let sub_args = list_elements(cx, list_value)?;
    let consumed = format_into(cx, &template, &sub_args, out, depth + 1)?;
    if consumed != sub_args.len() {
        return Err(Error::plain(
            ErrorKind::ArityError,
            "format: more arguments than the escape sequences in the template consume",
        ));
    }
    Ok(())
}

/// Collects a proper list into a vector with a tortoise and hare, so a
/// circular argument list raises instead of spinning.
fn list_elements(cx: &NativeContext<'_>, list: Value) -> Result<Vec<Value>, Error> {
    let mut elements = Vec::new();
    let mut hare = list;
    let mut tortoise = list;
    loop {
        for _ in 0..2 {
            if hare == Value::nil() {
                return Ok(elements);
            }
            let Some((car, tail)) = cx.heap.pair(hare) else {
                return Err(type_error("proper list", hare, cx.heap));
            };
            elements.push(car);
            hare = tail;
        }
        if let Some((_, tail)) = cx.heap.pair(tortoise) {
            tortoise = tail;
        }
        if tortoise == hare {
            return Err(Error::plain(
                ErrorKind::TypeError,
                "format: expected a proper list of arguments, received a circular list",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Engine, EngineConfig, ErrorKind, Extension};

    /// Builds an engine with SRFI 48 installed.
    fn engine() -> Engine {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi48).unwrap();
        engine
    }

    /// Evaluates a program and returns the error kind it raises.
    fn error_kind(engine: &mut Engine, source: &str) -> ErrorKind {
        let module = engine.compile("program.scm", source).unwrap();
        engine.eval(&module).unwrap_err().kind()
    }

    #[test]
    fn a_non_string_template_is_a_type_error() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, "(import (srfi 48)) (format 42)"),
            ErrorKind::TypeError
        );
    }

    #[test]
    fn an_incomplete_escape_is_a_runtime_error() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 48)) (format "ends with ~")"#),
            ErrorKind::RuntimeError
        );
    }

    #[test]
    fn an_unrecognized_escape_is_a_runtime_error() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 48)) (format "~q" 1)"#),
            ErrorKind::RuntimeError
        );
    }

    #[test]
    fn too_few_arguments_is_an_arity_error() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 48)) (format "~a ~a" 1)"#),
            ErrorKind::ArityError
        );
    }

    #[test]
    fn too_many_arguments_is_an_arity_error() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 48)) (format "~a" 1 2)"#),
            ErrorKind::ArityError
        );
    }

    #[test]
    fn a_non_character_for_the_character_escape_is_a_type_error() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 48)) (format "~c" 1)"#),
            ErrorKind::TypeError
        );
    }

    #[test]
    fn a_non_number_for_a_radix_escape_is_a_type_error() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 48)) (format "~x" "ff")"#),
            ErrorKind::TypeError
        );
    }

    #[test]
    fn a_non_string_non_number_for_fixed_format_is_a_type_error() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 48)) (format "~4F" 'sym)"#),
            ErrorKind::TypeError
        );
    }

    #[test]
    fn an_unterminated_fixed_field_is_a_runtime_error() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 48)) (format "~8,2" 1)"#),
            ErrorKind::RuntimeError
        );
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 48)) (format "~8,F" 1)"#),
            ErrorKind::RuntimeError
        );
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 48)) (format "~8x" 1)"#),
            ErrorKind::RuntimeError
        );
    }

    #[test]
    fn an_oversized_fixed_field_width_is_a_range_error() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 48)) (format "~99999F" 1)"#),
            ErrorKind::RangeError
        );
    }

    #[test]
    fn indirection_rejects_a_non_list_argument_pack() {
        let mut engine = engine();
        assert_eq!(
            error_kind(&mut engine, r#"(import (srfi 48)) (format "~?" "~a" 1)"#),
            ErrorKind::TypeError
        );
    }

    #[test]
    fn indirection_rejects_a_circular_argument_list() {
        let mut engine = engine();
        assert_eq!(
            error_kind(
                &mut engine,
                r#"
                (import (scheme base) (srfi 48))
                (define args (list 1 2))
                (set-cdr! (cdr args) args)
                (format "~?" "~a" args)
                "#
            ),
            ErrorKind::TypeError
        );
    }

    #[test]
    fn indirection_nested_too_deeply_is_a_runtime_error() {
        let mut engine = engine();
        // Each level formats "~?" with itself as the nested template, so the
        // recursion only ends at the depth limit.
        assert_eq!(
            error_kind(
                &mut engine,
                r#"
                (import (scheme base) (srfi 48))
                (define (self-args depth)
                  (if (zero? depth)
                      (list "no directives" '())
                      (list "~?" (self-args (- depth 1)))))
                (format "~?" "~?" (self-args 20))
                "#
            ),
            ErrorKind::RuntimeError
        );
    }
}
