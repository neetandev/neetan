//! Registered host procedures and the built-in base library primitives.

use std::{collections::HashMap, ops::RangeInclusive};

use crate::{
    Error, ErrorKind, Value, ValueKind,
    heap::{Heap, Object},
    number::{Real, RuntimeNumber},
};

pub(crate) mod bytevector;
mod collection;
mod context;
mod integer;
mod io;
mod number;
mod registry;
mod scalar;
pub(crate) mod srfi1;
pub(crate) mod srfi151;
pub(crate) mod srfi152;
pub(crate) mod srfi175;
pub(crate) mod srfi27;
pub(crate) mod srfi48;
pub(crate) mod srfi69;

use collection::*;
pub use context::{IntoNativeValues, NativeContext, NativeValues};
use integer::*;
use io::*;
use number::*;
pub(crate) use registry::{FastProcedure, NativeRegistry};
use scalar::*;

pub(crate) fn install_base(
    registry: &mut NativeRegistry,
    heap: &mut Heap,
    globals: &mut crate::global::GlobalStore,
) -> Result<(), Error> {
    struct Installer<'a> {
        registry: &'a mut NativeRegistry,
        heap: &'a mut Heap,
        globals: &'a mut crate::global::GlobalStore,
    }

    impl Installer<'_> {
        fn register<F, R>(
            &mut self,
            name: &str,
            arity: RangeInclusive<usize>,
            callback: F,
        ) -> Result<(), Error>
        where
            F: for<'a> Fn(&mut NativeContext<'a>, &[Value]) -> Result<R, Error> + 'static,
            R: IntoNativeValues + 'static,
        {
            self.refresh_roots();
            self.registry
                .register(self.heap, self.globals, name.to_owned(), arity, callback)
        }

        fn install_value(&mut self, name: &str, object: Object) -> Result<(), Error> {
            self.refresh_roots();
            let value = self.heap.alloc(object)?;
            self.globals.insert(name.to_owned(), value);
            Ok(())
        }

        fn refresh_roots(&mut self) {
            self.heap.set_engine_roots(self.globals.values().copied());
        }
    }

    let mut installer = Installer {
        registry,
        heap,
        globals,
    };
    macro_rules! native {
        ($name:literal, $arity:expr, $function:ident) => {{ installer.register($name, $arity, $function)? }};
    }
    native!("boolean?", 1..=1, predicate_boolean);
    native!("char?", 1..=1, predicate_char);
    native!("char=?", 2..=usize::MAX, char_equal);
    native!("char<?", 2..=usize::MAX, char_less);
    native!("char>?", 2..=usize::MAX, char_greater);
    native!("char<=?", 2..=usize::MAX, char_less_equal);
    native!("char>=?", 2..=usize::MAX, char_greater_equal);
    native!("char->integer", 1..=1, char_to_integer);
    native!("integer->char", 1..=1, integer_to_char);
    native!("null?", 1..=1, predicate_null);
    native!("pair?", 1..=1, predicate_pair);
    native!("vector?", 1..=1, predicate_vector);
    native!("string?", 1..=1, predicate_string);
    native!("bytevector?", 1..=1, predicate_bytevector);
    native!("symbol?", 1..=1, predicate_symbol);
    native!("procedure?", 1..=1, predicate_procedure);
    native!("promise?", 1..=1, predicate_promise);
    native!("make-parameter", 1..=1, make_parameter);
    native!("make-promise", 1..=1, make_promise);
    native!("environment", 1..=usize::MAX, environment);
    native!("interaction-environment", 0..=0, interaction_environment);
    native!("error-object?", 1..=1, error_object_predicate);
    native!("error-object-message", 1..=1, error_object_message);
    native!("error-object-irritants", 1..=1, error_object_irritants);
    native!("read-error?", 1..=1, read_error_predicate);
    native!("file-error?", 1..=1, file_error_predicate);
    native!("values", 0..=usize::MAX, values_procedure);
    native!("number?", 1..=1, predicate_number);
    native!("exact?", 1..=1, predicate_exact);
    native!("inexact?", 1..=1, predicate_inexact);
    native!("exact-integer?", 1..=1, predicate_exact_integer);
    native!("integer?", 1..=1, predicate_integer);
    native!("rational?", 1..=1, predicate_rational);
    native!("real?", 1..=1, predicate_real);
    native!("complex?", 1..=1, predicate_number);
    native!("cons", 2..=2, cons);
    native!("car", 1..=1, car);
    native!("cdr", 1..=1, cdr);
    native!("set-car!", 2..=2, set_car);
    native!("set-cdr!", 2..=2, set_cdr);
    native!("caar", 1..=1, caar);
    native!("cadr", 1..=1, cadr);
    native!("cdar", 1..=1, cdar);
    native!("cddr", 1..=1, cddr);
    native!("caaar", 1..=1, caaar);
    native!("caadr", 1..=1, caadr);
    native!("cadar", 1..=1, cadar);
    native!("caddr", 1..=1, caddr);
    native!("cdaar", 1..=1, cdaar);
    native!("cdadr", 1..=1, cdadr);
    native!("cddar", 1..=1, cddar);
    native!("cdddr", 1..=1, cdddr);
    native!("caaaar", 1..=1, caaaar);
    native!("caaadr", 1..=1, caaadr);
    native!("caadar", 1..=1, caadar);
    native!("caaddr", 1..=1, caaddr);
    native!("cadaar", 1..=1, cadaar);
    native!("cadadr", 1..=1, cadadr);
    native!("caddar", 1..=1, caddar);
    native!("cadddr", 1..=1, cadddr);
    native!("cdaaar", 1..=1, cdaaar);
    native!("cdaadr", 1..=1, cdaadr);
    native!("cdadar", 1..=1, cdadar);
    native!("cdaddr", 1..=1, cdaddr);
    native!("cddaar", 1..=1, cddaar);
    native!("cddadr", 1..=1, cddadr);
    native!("cdddar", 1..=1, cdddar);
    native!("cddddr", 1..=1, cddddr);
    native!("list", 0..=usize::MAX, list);
    native!("list?", 1..=1, list_predicate);
    native!("length", 1..=1, list_length);
    native!("reverse", 1..=1, list_reverse);
    native!("append", 0..=usize::MAX, list_append);
    native!("list-tail", 2..=2, list_tail);
    native!("list-ref", 2..=2, list_ref);
    native!("list-set!", 3..=3, list_set);
    native!("make-list", 1..=2, make_list);
    native!("list-copy", 1..=1, list_copy);
    native!("memq", 2..=2, member_by_eqv);
    native!("memv", 2..=2, member_by_eqv);
    native!("%member", 2..=2, member_by_equal);
    native!("assq", 2..=2, assoc_by_eqv);
    native!("assv", 2..=2, assoc_by_eqv);
    native!("%assoc", 2..=2, assoc_by_equal);
    native!("%literal", 1..=1, literal_freeze);
    native!("vector", 0..=usize::MAX, vector);
    native!("make-vector", 1..=2, make_vector);
    native!("vector-length", 1..=1, vector_length);
    native!("vector-ref", 2..=2, vector_ref);
    native!("vector-set!", 3..=3, vector_set);
    native!("string", 0..=usize::MAX, string);
    native!("make-string", 1..=2, make_string);
    native!("string-length", 1..=1, string_length);
    native!("string-ref", 2..=2, string_ref);
    native!("string-set!", 3..=3, string_set);
    native!("string-append", 0..=usize::MAX, string_append);
    native!("string->list", 1..=3, string_to_list);
    native!("list->string", 1..=1, list_to_string);
    native!("vector->list", 1..=3, vector_to_list);
    native!("list->vector", 1..=1, list_to_vector);
    native!("string->vector", 1..=3, string_to_vector);
    native!("vector->string", 1..=3, vector_to_string);
    native!("vector-append", 0..=usize::MAX, vector_append);
    native!("string-copy", 1..=3, string_copy);
    native!("substring", 3..=3, substring);
    native!("vector-copy", 1..=3, vector_copy);
    native!("string-fill!", 2..=4, string_fill);
    native!("vector-fill!", 2..=4, vector_fill);
    native!("string-copy!", 3..=5, string_copy_mut);
    native!("vector-copy!", 3..=5, vector_copy_mut);
    native!("string=?", 2..=usize::MAX, string_equal);
    native!("string<?", 2..=usize::MAX, string_less);
    native!("string>?", 2..=usize::MAX, string_greater);
    native!("string<=?", 2..=usize::MAX, string_less_equal);
    native!("string>=?", 2..=usize::MAX, string_greater_equal);
    native!("bytevector", 0..=usize::MAX, bytevector);
    native!("make-bytevector", 1..=2, make_bytevector);
    native!("bytevector-length", 1..=1, bytevector_length);
    native!("bytevector-u8-ref", 2..=2, bytevector_ref);
    native!("bytevector-u8-set!", 3..=3, bytevector_set);
    native!("bytevector-copy", 1..=3, bytevector_copy);
    native!("bytevector-copy!", 3..=5, bytevector_copy_mut);
    native!("bytevector-append", 0..=usize::MAX, bytevector_append);
    native!("string->utf8", 1..=3, string_to_utf8);
    native!("utf8->string", 1..=3, utf8_to_string);
    native!("string->symbol", 1..=1, string_to_symbol);
    native!("symbol->string", 1..=1, symbol_to_string);
    native!("eq?", 2..=2, eqv);
    native!("eqv?", 2..=2, eqv);
    native!("equal?", 2..=2, equal);
    native!("not", 1..=1, not);
    native!("boolean=?", 2..=usize::MAX, boolean_equal);
    native!("symbol=?", 2..=usize::MAX, symbol_equal);
    native!("+", 0..=usize::MAX, numeric_add);
    native!("-", 1..=usize::MAX, numeric_subtract);
    native!("*", 0..=usize::MAX, numeric_multiply);
    native!("/", 1..=usize::MAX, numeric_divide);
    native!("=", 2..=usize::MAX, numeric_equal);
    native!("<", 2..=usize::MAX, numeric_less);
    native!(">", 2..=usize::MAX, numeric_greater);
    native!("<=", 2..=usize::MAX, numeric_less_equal);
    native!(">=", 2..=usize::MAX, numeric_greater_equal);
    native!("exact", 1..=1, numeric_exact);
    native!("inexact", 1..=1, numeric_inexact);
    native!("zero?", 1..=1, numeric_zero);
    native!("positive?", 1..=1, numeric_positive);
    native!("negative?", 1..=1, numeric_negative);
    native!("odd?", 1..=1, numeric_odd);
    native!("even?", 1..=1, numeric_even);
    native!("abs", 1..=1, numeric_abs);
    native!("numerator", 1..=1, numeric_numerator);
    native!("denominator", 1..=1, numeric_denominator);
    native!("rationalize", 2..=2, rationalize);
    native!("number->string", 1..=2, number_to_string);
    native!("string->number", 1..=2, string_to_number);
    native!("make-rectangular", 2..=2, make_rectangular);
    native!("make-polar", 2..=2, make_polar);
    native!("real-part", 1..=1, real_part);
    native!("imag-part", 1..=1, imag_part);
    native!("magnitude", 1..=1, magnitude);
    native!("angle", 1..=1, angle);
    native!("finite?", 1..=1, numeric_finite);
    native!("infinite?", 1..=1, numeric_infinite);
    native!("nan?", 1..=1, numeric_nan);
    native!("max", 1..=usize::MAX, numeric_max);
    native!("min", 1..=usize::MAX, numeric_min);
    native!("floor", 1..=1, numeric_floor);
    native!("ceiling", 1..=1, numeric_ceiling);
    native!("truncate", 1..=1, numeric_truncate);
    native!("round", 1..=1, numeric_round);
    native!("quotient", 2..=2, quotient);
    native!("remainder", 2..=2, remainder);
    native!("modulo", 2..=2, modulo);
    native!("floor/", 2..=2, floor_divide);
    native!("floor-quotient", 2..=2, floor_quotient);
    native!("floor-remainder", 2..=2, floor_remainder);
    native!("truncate/", 2..=2, truncate_divide);
    native!("truncate-quotient", 2..=2, truncate_quotient);
    native!("truncate-remainder", 2..=2, truncate_remainder);
    native!("gcd", 0..=usize::MAX, gcd_procedure);
    native!("lcm", 0..=usize::MAX, lcm_procedure);
    native!("square", 1..=1, square);
    native!("expt", 2..=2, expt);
    native!("exact-integer-sqrt", 1..=1, exact_integer_sqrt);
    native!("sqrt", 1..=1, sqrt);
    native!("sin", 1..=1, sin);
    native!("cos", 1..=1, cos);
    native!("tan", 1..=1, tan);
    native!("exp", 1..=1, exp);
    native!("log", 1..=2, log);
    native!("asin", 1..=1, asin);
    native!("acos", 1..=1, acos);
    native!("atan", 1..=2, atan);
    native!("char-alphabetic?", 1..=1, char_alphabetic);
    native!("char-numeric?", 1..=1, char_numeric);
    native!("char-whitespace?", 1..=1, char_whitespace);
    native!("char-upper-case?", 1..=1, char_upper_case);
    native!("char-lower-case?", 1..=1, char_lower_case);
    native!("digit-value", 1..=1, digit_value);
    native!("char-upcase", 1..=1, char_upcase);
    native!("char-downcase", 1..=1, char_downcase);
    native!("char-foldcase", 1..=1, char_foldcase);
    native!("char-ci=?", 2..=usize::MAX, char_ci_equal);
    native!("char-ci<?", 2..=usize::MAX, char_ci_less);
    native!("char-ci>?", 2..=usize::MAX, char_ci_greater);
    native!("char-ci<=?", 2..=usize::MAX, char_ci_less_equal);
    native!("char-ci>=?", 2..=usize::MAX, char_ci_greater_equal);
    native!("string-upcase", 1..=1, string_upcase);
    native!("string-downcase", 1..=1, string_downcase);
    native!("string-foldcase", 1..=1, string_foldcase);
    native!("string-ci=?", 2..=usize::MAX, string_ci_equal);
    native!("string-ci<?", 2..=usize::MAX, string_ci_less);
    native!("string-ci>?", 2..=usize::MAX, string_ci_greater);
    native!("string-ci<=?", 2..=usize::MAX, string_ci_less_equal);
    native!("string-ci>=?", 2..=usize::MAX, string_ci_greater_equal);
    native!("input-port?", 1..=1, input_port_predicate);
    native!("output-port?", 1..=1, output_port_predicate);
    native!("textual-port?", 1..=1, textual_port_predicate);
    native!("binary-port?", 1..=1, binary_port_predicate);
    native!("port?", 1..=1, port_predicate);
    native!("input-port-open?", 1..=1, input_port_open);
    native!("output-port-open?", 1..=1, output_port_open);
    native!("open-input-string", 1..=1, open_input_string);
    native!("open-output-string", 0..=0, open_output_string);
    native!("get-output-string", 1..=1, get_output_string);
    native!("open-input-bytevector", 1..=1, open_input_bytevector);
    native!("open-output-bytevector", 0..=0, open_output_bytevector);
    native!("get-output-bytevector", 1..=1, get_output_bytevector);
    native!("close-port", 1..=1, close_port);
    native!("close-input-port", 1..=1, close_input_port);
    native!("close-output-port", 1..=1, close_output_port);
    native!("read-char", 0..=1, read_char);
    native!("peek-char", 0..=1, peek_char);
    native!("read-u8", 0..=1, read_u8);
    native!("peek-u8", 0..=1, peek_u8);
    native!("char-ready?", 0..=1, char_ready);
    native!("u8-ready?", 0..=1, u8_ready);
    native!("read-line", 0..=1, read_line);
    native!("read-string", 1..=2, read_string);
    native!("read-bytevector", 1..=2, read_bytevector);
    native!("read-bytevector!", 1..=4, read_bytevector_mut);
    native!("write-char", 1..=2, write_char);
    native!("write-string", 1..=4, write_string);
    native!("newline", 0..=1, newline);
    native!("write-u8", 1..=2, write_u8);
    native!("write-bytevector", 1..=4, write_bytevector);
    native!("flush-output-port", 0..=1, flush_output_port);
    native!("eof-object", 0..=0, eof_object);
    native!("eof-object?", 1..=1, eof_object_predicate);
    native!("read", 0..=1, read);
    native!("write", 1..=2, write);
    native!("write-shared", 1..=2, write_shared);
    native!("write-simple", 1..=2, write_simple);
    native!("display", 1..=2, display);
    native!("open-input-file", 1..=1, open_input_file);
    native!("open-output-file", 1..=1, open_output_file);
    native!("open-binary-input-file", 1..=1, open_binary_input_file);
    native!("open-binary-output-file", 1..=1, open_binary_output_file);
    native!("file-exists?", 1..=1, file_exists);
    native!("delete-file", 1..=1, delete_file);
    native!("command-line", 0..=0, command_line);
    native!("get-environment-variable", 1..=1, get_environment_variable);
    native!(
        "get-environment-variables",
        0..=0,
        get_environment_variables
    );
    native!("current-second", 0..=0, current_second);
    native!("current-jiffy", 0..=0, current_jiffy);
    native!("jiffies-per-second", 0..=0, jiffies_per_second);
    native!("exit", 0..=1, exit);
    native!("emergency-exit", 0..=1, emergency_exit);
    native!("%make-record-type", 1..=1, make_record_type);
    native!(
        "%make-record-constructor",
        1..=usize::MAX,
        make_record_constructor
    );
    native!("%make-record-predicate", 1..=1, make_record_predicate);
    native!("%make-record-accessor", 2..=2, make_record_accessor);
    native!("%make-record-mutator", 2..=2, make_record_mutator);
    installer.install_value("apply", Object::Apply)?;
    Ok(())
}

fn bool_value(value: bool) -> Result<Value, Error> {
    Ok(Value::boolean(value))
}

fn type_error(expected: &str, value: Value, heap: &Heap) -> Error {
    Error::plain(
        ErrorKind::TypeError,
        format!("expected {expected}, received {:?}", heap.kind(value)),
    )
}

fn range_or_type(length: Option<usize>, expected: &str, value: Value) -> Error {
    match length {
        Some(_) => Error::plain(ErrorKind::RangeError, "index is outside the sequence"),
        None => Error::plain(
            ErrorKind::TypeError,
            format!("expected {expected}, received {:?}", value.kind()),
        ),
    }
}

fn length(
    length: Option<usize>,
    expected: &str,
    value: Value,
    heap: &Heap,
) -> Result<Value, Error> {
    length
        .and_then(|value| i64::try_from(value).ok().map(Value::integer))
        .ok_or_else(|| type_error(expected, value, heap))
}

fn index(cx: &NativeContext<'_>, value: Value) -> Result<usize, Error> {
    usize::try_from(exact_integer(cx, value)?).map_err(|_| {
        Error::plain(
            ErrorKind::RangeError,
            "index must be a non-negative exact integer",
        )
    })
}

fn character(cx: &NativeContext<'_>, value: Value) -> Result<char, Error> {
    match value.decode() {
        crate::value::ValueRepr::Character(value) => Ok(value),
        _ => Err(type_error("character", value, cx.heap)),
    }
}

fn byte(cx: &NativeContext<'_>, value: Value) -> Result<u8, Error> {
    u8::try_from(exact_integer(cx, value)?).map_err(|_| {
        Error::plain(
            ErrorKind::RangeError,
            "byte must be an exact integer from 0 through 255",
        )
    })
}

/// Decodes a fill argument for `make-bytevector` and `bytevector-fill!`. The
/// R6RS bytevectors library allows a signed byte here, so negative values are
/// stored as their two's complement.
fn fill_byte(cx: &NativeContext<'_>, value: Value) -> Result<u8, Error> {
    let n = exact_integer(cx, value)?;
    if (-128..=255).contains(&n) {
        Ok((n & 0xFF) as u8)
    } else {
        Err(Error::plain(
            ErrorKind::RangeError,
            "fill must be an exact integer from -128 through 255",
        ))
    }
}

fn bytevector_argument(cx: &NativeContext<'_>, value: Value) -> Result<Vec<u8>, Error> {
    cx.heap
        .bytevector(value)
        .ok_or_else(|| type_error("bytevector", value, cx.heap))
}
