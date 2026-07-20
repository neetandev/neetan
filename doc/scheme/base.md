# (scheme base)

```scheme
(import (scheme base))
```

This is R7RS-small's core library. It is active by default in every
neetan-auto script, so no import is strictly needed for the globals, but
importing is idiomatic and required under `compile_program`. It provides
the fundamental data types, syntax, and procedures.

## Core syntax and binding forms

- `(define name value)` - (syntax) Binds name to value in the current scope.
- `(define (name arg ...) body ...)` - (syntax) Defines a procedure named name.
- `(define-values (formals ...) expr)` - (syntax) Binds multiple names to the
  values produced by expr.
- `(define-syntax name transformer)` - (syntax) Binds name to a macro
  transformer.
- `(define-record-type ...)` - (syntax) Defines a new record type with
  constructor, predicate, and field accessors.
- `(lambda (arg ...) body ...)` - (syntax) Creates an anonymous procedure.
- `(let ((name val) ...) body ...)` - (syntax) Binds names to values for the
  body; the named-let form `(let name ((v i) ...) body)` also creates a loop.
- `(let* ((name val) ...) body ...)` - (syntax) Like let, but each binding sees
  the earlier ones.
- `(letrec ((name val) ...) body ...)` - (syntax) Bindings whose values may
  refer to each other, for mutual recursion.
- `(letrec* ((name val) ...) body ...)` - (syntax) Like letrec, but evaluates
  the bindings in sequence.
- `(let-values (((formals ...) expr) ...) body ...)` - (syntax) Binds names to
  the multiple values returned by each expr.
- `(let*-values (((formals ...) expr) ...) body ...)` - (syntax) Like
  let-values, but each binding sees the earlier ones.
- `(let-syntax ((name transformer) ...) body ...)` - (syntax) Binds local
  macros for the body.
- `(letrec-syntax ((name transformer) ...) body ...)` - (syntax) Binds mutually
  recursive local macros for the body.
- `(begin expr ...)` - (syntax) Evaluates the expressions in order and returns
  the last one's value.
- `(set! name value)` - (syntax) Assigns value to an existing binding.
- `(quote datum)` - (syntax) Returns datum unevaluated; also written 'datum.
- `(quasiquote template)` - (syntax) Returns template with unquoted parts
  substituted; also written `template.
- `(unquote expr)` - (syntax) Inside quasiquote, evaluates expr; also written
  ,expr.
- `(unquote-splicing expr)` - (syntax) Inside quasiquote, splices a list into
  place; also written ,@expr.
- `(syntax-rules (literal ...) (pattern template) ...)` - (syntax) Creates a
  pattern-matching macro transformer.
- `(syntax-error message arg ...)` - (syntax) Signals an error at macro
  expansion time.
- `(include filename ...)` - (syntax) Includes the contents of the named source
  files; usable only inside a define-library declaration.
- `(include-ci filename ...)` - (syntax) Like include but reads the files
  case-insensitively; usable only inside a define-library declaration.
- `...` - (syntax) The ellipsis used in syntax-rules patterns and templates.
- `=>` - (syntax) Auxiliary keyword used by cond clauses and guard.
- `else` - (syntax) Auxiliary keyword marking the default clause in cond, case,
  and guard.

## Conditionals and control

- `(if test then else)` - (syntax) Evaluates then when test is true, otherwise
  else.
- `(cond clause ...)` - (syntax) Tries each clause's test and evaluates the body
  of the first that is true.
- `(case key clause ...)` - (syntax) Selects a clause by comparing key against
  lists of datums with eqv?.
- `(when test body ...)` - (syntax) Evaluates the body when test is true.
- `(unless test body ...)` - (syntax) Evaluates the body when test is false.
- `(and expr ...)` - (syntax) Evaluates left to right, returning the first false
  value or the last value.
- `(or expr ...)` - (syntax) Evaluates left to right, returning the first true
  value or false.
- `(do ((var init step) ...) (test result ...) body ...)` - (syntax) A general
  iteration loop.
- `(cond-expand clause ...)` - (syntax) Expands the body of the first clause
  whose feature requirement is satisfied.
- `(apply proc arg ... list)` - Calls proc with the given args followed by the
  elements of list.
- `(procedure? obj)` -> boolean. Returns #t if obj is a procedure.
- `(map proc list1 list2 ...)` - Applies proc across the lists and returns a
  list of results.
- `(for-each proc list1 list2 ...)` - Applies proc across the lists for its side
  effects and returns an unspecified value.
- `(call-with-current-continuation proc)` - Calls proc with the current
  continuation as an escape procedure.
- `(call/cc proc)` - Shorthand for call-with-current-continuation.
- `(dynamic-wind before thunk after)` - Runs thunk, calling before on entry and
  after on exit, even across continuation jumps.
- `(values obj ...)` - Returns its arguments as multiple values.
- `(call-with-values producer consumer)` - Calls producer, then applies consumer
  to the values it returns.

## Equivalence and booleans

- `(eq? obj1 obj2)` -> boolean. Tests identity. In this implementation `eq?` and
  `eqv?` are the same procedure, so `eq?` also equates equal numbers and
  characters.
- `(eqv? obj1 obj2)` -> boolean. Tests identity and also equates equal numbers
  and characters.
- `(equal? obj1 obj2)` -> boolean. Recursive structural equivalence for pairs,
  strings, vectors, and bytevectors.
- `(not obj)` -> boolean. Returns #t if obj is false, otherwise #f.
- `(boolean? obj)` -> boolean. Returns #t if obj is #t or #f.
- `(boolean=? bool1 bool2 ...)` -> boolean. Returns #t if all the booleans are
  the same.

## Pairs and lists

- `(pair? obj)` -> boolean. Returns #t if obj is a pair.
- `(cons obj1 obj2)` -> pair. Constructs a new pair with car obj1 and cdr obj2.
- `(car pair)` -> obj. Returns the car of pair.
- `(cdr pair)` -> obj. Returns the cdr of pair.
- `(set-car! pair obj)` - Sets the car of pair to obj.
- `(set-cdr! pair obj)` - Sets the cdr of pair to obj.
- `(caar pair)` -> obj. Returns the car of the car of pair.
- `(cadr pair)` -> obj. Returns the car of the cdr of pair.
- `(cdar pair)` -> obj. Returns the cdr of the car of pair.
- `(cddr pair)` -> obj. Returns the cdr of the cdr of pair.
- `(null? obj)` -> boolean. Returns #t if obj is the empty list.
- `(list? obj)` -> boolean. Returns #t if obj is a proper list.
- `(list obj ...)` -> list. Builds a new list of the given objects.
- `(make-list k [fill])` -> list. Builds a list of k elements, each fill.
- `(length list)` -> integer. Returns the number of elements in list.
- `(append list ... obj)` -> list. Concatenates the lists, sharing the last
  argument.
- `(reverse list)` -> list. Returns a new list with the elements reversed.
- `(list-tail list k)` -> list. Returns the sublist of list after k elements.
- `(list-ref list k)` -> obj. Returns element k of list.
- `(list-set! list k obj)` - Sets element k of list to obj.
- `(list-copy obj)` -> list. Returns a shallow copy of a list.
- `(memq obj list)` - Returns the first sublist whose car is eq? to obj, else
  #f.
- `(memv obj list)` - Returns the first sublist whose car is eqv? to obj, else
  #f.
- `(member obj list [compare])` - Returns the first sublist whose car matches
  obj under compare (equal? by default), else #f.
- `(assq obj alist)` - Returns the first pair whose car is eq? to obj, else #f.
- `(assv obj alist)` - Returns the first pair whose car is eqv? to obj, else #f.
- `(assoc obj alist [compare])` - Returns the first pair whose car matches obj
  under compare (equal? by default), else #f.

## Symbols

- `(symbol? obj)` -> boolean. Returns #t if obj is a symbol.
- `(symbol=? sym1 sym2 ...)` -> boolean. Returns #t if all the symbols are the
  same.
- `(symbol->string symbol)` -> string. Returns the name of symbol as a fresh
  immutable string.
- `(string->symbol string)` -> symbol. Returns the symbol whose name is string.

## Characters

- `(char? obj)` -> boolean. Returns #t if obj is a character.
- `(char->integer char)` -> integer. Returns the Unicode scalar value of char.
- `(integer->char n)` -> char. Returns the character with scalar value n.
- `(char=? char1 char2 ...)` -> boolean. Returns #t if the characters are equal.
- `(char<? char1 char2 ...)` -> boolean. Returns #t if the characters are in
  increasing order.
- `(char>? char1 char2 ...)` -> boolean. Returns #t if the characters are in
  decreasing order.
- `(char<=? char1 char2 ...)` -> boolean. Returns #t if the characters are in
  non-decreasing order.
- `(char>=? char1 char2 ...)` -> boolean. Returns #t if the characters are in
  non-increasing order.

## Strings

- `(string? obj)` -> boolean. Returns #t if obj is a string.
- `(string char ...)` -> string. Builds a string from the given characters.
- `(make-string k [char])` -> string. Builds a string of k copies of char.
- `(string-length string)` -> integer. Returns the number of characters in
  string.
- `(string-ref string k)` -> char. Returns character k of string.
- `(string-set! string k char)` - Sets character k of string to char.
- `(string=? str1 str2 ...)` -> boolean. Returns #t if the strings are equal.
- `(string<? str1 str2 ...)` -> boolean. Returns #t if the strings are in
  increasing lexicographic order.
- `(string>? str1 str2 ...)` -> boolean. Returns #t if the strings are in
  decreasing lexicographic order.
- `(string<=? str1 str2 ...)` -> boolean. Returns #t if the strings are in
  non-decreasing order.
- `(string>=? str1 str2 ...)` -> boolean. Returns #t if the strings are in
  non-increasing order.
- `(substring string start end)` -> string. Returns the substring from start
  to end.
- `(string-append str ...)` -> string. Concatenates the strings.
- `(string-copy string [start [end]])` -> string. Returns a copy of the given
  portion of string.
- `(string-copy! to at from [start [end]])` - Copies characters from from into
  to starting at index at.
- `(string-fill! string fill [start [end]])` - Sets the given portion of string
  to fill.
- `(string->list string [start [end]])` -> list. Returns the characters of the
  given portion as a list.
- `(list->string list)` -> string. Builds a string from a list of characters.
- `(string->vector string [start [end]])` -> vector. Returns the characters of
  the given portion as a vector.
- `(vector->string vector [start [end]])` -> string. Builds a string from a
  vector of characters.
- `(string->number string [radix])` - Parses string as a number in the given
  radix, or #f if it is not a number.
- `(number->string z [radix])` -> string. Renders number z as a string in the
  given radix.
- `(string-map proc str1 str2 ...)` -> string. Returns a string of the results
  of applying proc across the strings.
- `(string-for-each proc str1 str2 ...)` - Applies proc across the strings for
  side effects.

## Vectors

- `(vector? obj)` -> boolean. Returns #t if obj is a vector.
- `(vector obj ...)` -> vector. Builds a vector from the given objects.
- `(make-vector k [fill])` -> vector. Builds a vector of k elements, each fill.
- `(vector-length vector)` -> integer. Returns the number of elements in vector.
- `(vector-ref vector k)` -> obj. Returns element k of vector.
- `(vector-set! vector k obj)` - Sets element k of vector to obj.
- `(vector-copy vector [start [end]])` -> vector. Returns a copy of the given
  portion of vector.
- `(vector-copy! to at from [start [end]])` - Copies elements from from into to
  starting at index at.
- `(vector-fill! vector fill [start [end]])` - Sets the given portion of vector
  to fill.
- `(vector-append vector ...)` -> vector. Concatenates the vectors.
- `(vector->list vector [start [end]])` -> list. Returns the elements of the
  given portion as a list.
- `(list->vector list)` -> vector. Builds a vector from a list.
- `(vector-map proc vec1 vec2 ...)` -> vector. Returns a vector of the results
  of applying proc across the vectors.
- `(vector-for-each proc vec1 vec2 ...)` - Applies proc across the vectors for
  side effects.

## Bytevectors

- `(bytevector? obj)` -> boolean. Returns #t if obj is a bytevector.
- `(bytevector byte ...)` -> bytevector. Builds a bytevector from the given
  bytes.
- `(make-bytevector k [byte])` -> bytevector. Builds a bytevector of k elements,
  each byte.
- `(bytevector-length bytevector)` -> integer. Returns the number of bytes in
  bytevector.
- `(bytevector-u8-ref bytevector k)` -> byte. Returns byte k of bytevector.
- `(bytevector-u8-set! bytevector k byte)` - Sets byte k of bytevector.
- `(bytevector-copy bytevector [start [end]])` -> bytevector. Returns a copy of
  the given portion of bytevector.
- `(bytevector-copy! to at from [start [end]])` - Copies bytes from from into to
  starting at index at.
- `(bytevector-append bytevector ...)` -> bytevector. Concatenates the
  bytevectors.
- `(utf8->string bytevector [start [end]])` -> string. Decodes the given portion
  as UTF-8.
- `(string->utf8 string [start [end]])` -> bytevector. Encodes the given portion
  as UTF-8.

## Numbers and arithmetic

- `(number? obj)` -> boolean. Returns #t if obj is a number.
- `(complex? obj)` -> boolean. Returns #t if obj is a complex number.
- `(real? obj)` -> boolean. Returns #t if obj is a real number.
- `(rational? obj)` -> boolean. Returns #t if obj is a rational number.
- `(integer? obj)` -> boolean. Returns #t if obj is an integer.
- `(exact? z)` -> boolean. Returns #t if z is an exact number.
- `(inexact? z)` -> boolean. Returns #t if z is an inexact number.
- `(exact-integer? z)` -> boolean. Returns #t if z is an exact integer.
- `(exact z)` -> number. Returns an exact representation of z.
- `(inexact z)` -> number. Returns an inexact representation of z.
- `(= z1 z2 ...)` -> boolean. Returns #t if the numbers are all equal.
- `(< x1 x2 ...)` -> boolean. Returns #t if the numbers are in increasing order.
- `(> x1 x2 ...)` -> boolean. Returns #t if the numbers are in decreasing order.
- `(<= x1 x2 ...)` -> boolean. Returns #t if the numbers are in non-decreasing
  order.
- `(>= x1 x2 ...)` -> boolean. Returns #t if the numbers are in non-increasing
  order.
- `(zero? z)` -> boolean. Returns #t if z is zero.
- `(positive? x)` -> boolean. Returns #t if x is positive.
- `(negative? x)` -> boolean. Returns #t if x is negative.
- `(odd? n)` -> boolean. Returns #t if n is odd.
- `(even? n)` -> boolean. Returns #t if n is even.
- `(+ z ...)` -> number. Returns the sum of its arguments.
- `(- z ...)` -> number. Subtracts, or negates with one argument.
- `(* z ...)` -> number. Returns the product of its arguments.
- `(/ z ...)` -> number. Divides, or reciprocates with one argument.
- `(abs x)` -> number. Returns the absolute value of x.
- `(min x1 x2 ...)` -> number. Returns the smallest argument.
- `(max x1 x2 ...)` -> number. Returns the largest argument.
- `(quotient n1 n2)` -> integer. Returns the truncating integer quotient.
- `(remainder n1 n2)` -> integer. Returns the remainder matching quotient.
- `(modulo n1 n2)` -> integer. Returns the remainder matching floor division.
- `(floor/ n1 n2)` - Returns the floor quotient and remainder as two values.
- `(floor-quotient n1 n2)` -> integer. Returns the floor integer quotient.
- `(floor-remainder n1 n2)` -> integer. Returns the remainder matching floor
  division.
- `(truncate/ n1 n2)` - Returns the truncating quotient and remainder as two
  values.
- `(truncate-quotient n1 n2)` -> integer. Returns the truncating integer
  quotient.
- `(truncate-remainder n1 n2)` -> integer. Returns the remainder matching
  truncating division.
- `(gcd n ...)` -> integer. Returns the greatest common divisor.
- `(lcm n ...)` -> integer. Returns the least common multiple.
- `(numerator q)` -> number. Returns the numerator of rational q.
- `(denominator q)` -> number. Returns the denominator of rational q.
- `(floor x)` -> number. Returns the largest integer not greater than x.
- `(ceiling x)` -> number. Returns the smallest integer not less than x.
- `(truncate x)` -> number. Returns x rounded toward zero.
- `(round x)` -> number. Returns x rounded to the nearest integer, ties to even.
- `(rationalize x y)` -> number. Returns the simplest rational within y of x.
- `(expt z1 z2)` -> number. Returns z1 raised to the power z2.
- `(square z)` -> number. Returns z multiplied by itself.
- `(exact-integer-sqrt k)` - Returns the integer square root of k and the
  remainder as two values.

## Ports and I/O

- `(port? obj)` -> boolean. Returns #t if obj is a port.
- `(input-port? obj)` -> boolean. Returns #t if obj is an input port.
- `(output-port? obj)` -> boolean. Returns #t if obj is an output port.
- `(textual-port? obj)` -> boolean. Returns #t if obj is a textual port.
- `(binary-port? obj)` -> boolean. Returns #t if obj is a binary port.
- `(input-port-open? port)` -> boolean. Returns #t if the input port is open.
- `(output-port-open? port)` -> boolean. Returns #t if the output port is open.
- `(current-input-port)` -> port. Returns the current default input port.
- `(current-output-port)` -> port. Returns the current default output port.
- `(current-error-port)` -> port. Returns the current default error port.
- `(close-port port)` - Closes the port.
- `(close-input-port port)` - Closes the input port.
- `(close-output-port port)` - Closes the output port.
- `(call-with-port port proc)` - Calls proc with port, then closes port and
  returns proc's values.
- `(open-input-string string)` -> port. Opens a textual input port reading from
  string.
- `(open-output-string)` -> port. Opens a textual output port accumulating a
  string.
- `(get-output-string port)` -> string. Returns the accumulated string of a
  string output port.
- `(open-input-bytevector bytevector)` -> port. Opens a binary input port
  reading from bytevector.
- `(open-output-bytevector)` -> port. Opens a binary output port accumulating a
  bytevector.
- `(get-output-bytevector port)` -> bytevector. Returns the accumulated bytes of
  a bytevector output port.
- `(eof-object)` -> eof. Returns the end-of-file object.
- `(eof-object? obj)` -> boolean. Returns #t if obj is the end-of-file object.
- `(read-char [port])` - Reads and returns the next character, or the eof
  object.
- `(peek-char [port])` - Returns the next character without consuming it, or the
  eof object.
- `(read-line [port])` - Reads and returns the next line as a string, or the eof
  object.
- `(read-string k [port])` - Reads up to k characters as a string, or the eof
  object.
- `(char-ready? [port])` -> boolean. Returns #t if a character is ready on the
  input port.
- `(read-u8 [port])` - Reads and returns the next byte, or the eof object.
- `(peek-u8 [port])` - Returns the next byte without consuming it, or the eof
  object.
- `(read-bytevector k [port])` - Reads up to k bytes as a bytevector, or the eof
  object.
- `(read-bytevector! bytevector [port [start [end]]])` - Reads bytes into
  bytevector and returns the count, or the eof object.
- `(u8-ready? [port])` -> boolean. Returns #t if a byte is ready on the binary
  input port.
- `(write-char char [port])` - Writes char to the output port.
- `(write-string string [port [start [end]]])` - Writes the given portion of
  string to the output port.
- `(write-u8 byte [port])` - Writes byte to the binary output port.
- `(write-bytevector bytevector [port [start [end]]])` - Writes the given
  portion of bytevector to the binary output port.
- `(newline [port])` - Writes a newline to the output port.
- `(flush-output-port [port])` - Flushes buffered output of the port.

## Exceptions

- `(error message obj ...)` - Raises an error object with the message and
  irritants.
- `(error-object? obj)` -> boolean. Returns #t if obj is an error object created
  by error.
- `(error-object-message obj)` -> string. Returns the message of an error
  object.
- `(error-object-irritants obj)` -> list. Returns the irritants of an error
  object.
- `(raise obj)` - Raises obj as an exception to the current handler.
- `(raise-continuable obj)` - Raises obj such that the handler may return a
  value.
- `(with-exception-handler handler thunk)` - Runs thunk with handler installed
  for raised exceptions.
- `(guard (var clause ...) body ...)` - (syntax) Runs body, dispatching any
  raised exception through the cond-style clauses.
- `(file-error? obj)` -> boolean. Returns #t if obj is a file-related error.
- `(read-error? obj)` -> boolean. Returns #t if obj is a read/parse error.

## Records

- `(define-record-type name constructor predicate field ...)` - (syntax) Defines
  a record type together with its constructor, predicate, and field accessors
  and mutators.

## Multiple values

- `(values obj ...)` - Returns its arguments as multiple values.
- `(call-with-values producer consumer)` - Calls producer, then applies consumer
  to the values it returns.
- `(define-values (formals ...) expr)` - (syntax) Binds multiple names to the
  values produced by expr.
- `(let-values (((formals ...) expr) ...) body ...)` - (syntax) Binds names to
  the multiple values returned by each expr.
- `(let*-values (((formals ...) expr) ...) body ...)` - (syntax) Like
  let-values, but each binding sees the earlier ones.

## Parameters and dynamic-wind

- `(make-parameter init [converter])` -> parameter. Creates a parameter object
  with an initial value.
- `(parameterize ((param value) ...) body ...)` - (syntax) Runs body with the
  parameters bound to new values, restoring them on exit.
- `(dynamic-wind before thunk after)` - Runs thunk, calling before on entry and
  after on exit, even across continuation jumps.

## Feature detection

- `(features)` -> list. Returns the list of feature identifiers of this
  implementation.
- `(cond-expand clause ...)` - (syntax) Expands the body of the first clause
  whose feature requirement is satisfied.
