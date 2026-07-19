//! Optional, per-engine extension libraries.
//!
//! Extensions are enabled at runtime with [`Engine::install_extension`] rather
//! than through a Cargo feature, so each engine opts in independently. An
//! extension registers its libraries through the ordinary native-library
//! machinery and enables a `cond-expand` feature identifier. Each extension
//! also registers a discoverable alias of its public library in the
//! `(r7rs ...)` namespace, for example `(r7rs lists)` for `(srfi 1)`.

use super::Engine;
use crate::{
    Error, ErrorKind, LibraryName, LibraryNameComponent, Value,
    heap::Object,
    native::{srfi1, srfi27, srfi48, srfi69},
    random::SquaresRng,
};

/// An optional library that an engine can install after construction.
///
/// Installing an extension registers its libraries and enables its
/// [`Extension::feature_identifier`], so guest code can detect it with
/// `cond-expand` and `features`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Extension {
    /// SRFI 1, List Library. Provides `(srfi 1)` and the alias `(r7rs lists)`.
    Srfi1,
    /// SRFI 2, `AND-LET*`. Provides `(srfi 2)` and the alias `(r7rs and-let*)`.
    Srfi2,
    /// SRFI 8, `receive`. Provides `(srfi 8)` and the alias `(r7rs receive)`.
    Srfi8,
    /// SRFI 26, `cut`/`cute`. Provides `(srfi 26)` and the alias `(r7rs cut)`.
    Srfi26,
    /// SRFI 27, Sources of Random Bits. Provides `(srfi 27)` and the alias
    /// `(r7rs random-bits)`.
    Srfi27,
    /// SRFI 48, Intermediate Format Strings. Provides `(srfi 48)`, the
    /// compatibility name `(srfi 28)`, and the alias `(r7rs intermediate-format-strings)`.
    Srfi48,
    /// SRFI 69, Basic Hash Tables. Provides `(srfi 69)` and the alias
    /// `(r7rs basic-hash-table)`.
    Srfi69,
    /// SRFI 132, Sort Libraries. Provides `(srfi 132)` and the alias
    /// `(r7rs sorting)`.
    Srfi132,
    /// SRFI 151, Bitwise Operations. Provides `(srfi 151)` and the alias
    /// `(r7rs bitwise-operations)`.
    Srfi151,
    /// SRFI 152, String Library. Provides `(srfi 152)` and the alias
    /// `(r7rs strings)`.
    Srfi152,
    /// SRFI 175, ASCII Character Library. Provides `(srfi 175)` and the alias
    /// `(r7rs ascii)`.
    Srfi175,
    /// R6RS Bytevectors under its R7RS-large name. Provides
    /// `(scheme bytevector)` and the alias `(r7rs bytevector)`.
    Bytevector,
}

impl Extension {
    /// Every extension this build can install. Useful for enabling them all in
    /// a loop.
    pub const ALL: &'static [Extension] = &[
        Extension::Srfi1,
        Extension::Srfi2,
        Extension::Srfi8,
        Extension::Srfi26,
        Extension::Srfi27,
        Extension::Srfi48,
        Extension::Srfi69,
        Extension::Srfi132,
        Extension::Srfi151,
        Extension::Srfi152,
        Extension::Srfi175,
        Extension::Bytevector,
    ];

    /// The `cond-expand` feature identifier enabled when this extension is
    /// installed. SRFI 48 additionally enables `srfi-28`, because its install
    /// registers `(srfi 28)` as a compatibility name.
    #[must_use]
    pub const fn feature_identifier(self) -> &'static str {
        match self {
            Extension::Srfi1 => "srfi-1",
            Extension::Srfi2 => "srfi-2",
            Extension::Srfi8 => "srfi-8",
            Extension::Srfi26 => "srfi-26",
            Extension::Srfi27 => "srfi-27",
            Extension::Srfi48 => "srfi-48",
            Extension::Srfi69 => "srfi-69",
            Extension::Srfi132 => "srfi-132",
            Extension::Srfi151 => "srfi-151",
            Extension::Srfi152 => "srfi-152",
            Extension::Srfi175 => "srfi-175",
            Extension::Bytevector => "scheme-bytevector",
        }
    }

    /// The last component of the `(r7rs ...)` alias library registered
    /// alongside the SRFI library. Installing SRFI 69, for example, registers
    /// both `(srfi 69)` and the alias `(r7rs basic-hash-table)`.
    #[must_use]
    pub const fn alias_identifier(self) -> &'static str {
        match self {
            Extension::Srfi1 => "lists",
            Extension::Srfi2 => "and-let*",
            Extension::Srfi8 => "receive",
            Extension::Srfi26 => "cut",
            Extension::Srfi27 => "random-bits",
            Extension::Srfi48 => "intermediate-format-strings",
            Extension::Srfi69 => "basic-hash-table",
            Extension::Srfi132 => "sorting",
            Extension::Srfi151 => "bitwise-operations",
            Extension::Srfi152 => "strings",
            Extension::Srfi175 => "ascii",
            Extension::Bytevector => "bytevector",
        }
    }

    /// The specification name used in diagnostics.
    const fn spec_name(self) -> &'static str {
        match self {
            Extension::Srfi1 => "SRFI 1",
            Extension::Srfi2 => "SRFI 2",
            Extension::Srfi8 => "SRFI 8",
            Extension::Srfi26 => "SRFI 26",
            Extension::Srfi27 => "SRFI 27",
            Extension::Srfi48 => "SRFI 48",
            Extension::Srfi69 => "SRFI 69",
            Extension::Srfi132 => "SRFI 132",
            Extension::Srfi151 => "SRFI 151",
            Extension::Srfi152 => "SRFI 152",
            Extension::Srfi175 => "SRFI 175",
            Extension::Bytevector => "R6RS Bytevectors",
        }
    }
}

/// The Scheme wrapper library for `(srfi 1)`. It re-exports the R7RS list
/// overlap from `(scheme base)` and `(scheme cxr)`, re-exports the structural
/// primitives from `(srfi 1 native)`, and defines every higher-order procedure
/// (which must call back into a Scheme argument) plus the trivial selectors and
/// destructive aliases here. Internal helpers use a `%srfi1-` prefix and are not
/// exported. The destructive `!` procedures are functional aliases, which SRFI 1
/// permits.
const SRFI_1_SOURCE: &str = r#"
(define-library (srfi 1)
  (export
    ;; Re-exported from (scheme base) and (scheme cxr).
    cons car cdr caar cadr cdar cddr
    caaar caadr cadar caddr cdaar cdadr cddar cdddr
    caaaar caaadr caadar caaddr cadaar cadadr caddar cadddr
    cdaaar cdaadr cdadar cdaddr cddaar cddadr cdddar cddddr
    pair? null? list list? length append reverse list-ref list-copy make-list
    map for-each member memq memv assoc assq assv set-car! set-cdr!
    ;; Re-exported native structural primitives from (srfi 1 native).
    xcons cons* take take-right drop-right last last-pair length+
    append-reverse circular-list? dotted-list?
    ;; Predicates and constructors defined below.
    proper-list? not-pair? null-list? list=
    list-tabulate circular-list iota
    ;; Selectors defined below.
    first second third fourth fifth sixth seventh eighth ninth tenth
    car+cdr drop split-at
    ;; Miscellaneous.
    concatenate zip unzip1 unzip2 unzip3 unzip4 unzip5 count
    ;; Fold, unfold, and map.
    fold unfold pair-fold reduce fold-right unfold-right pair-fold-right
    reduce-right append-map filter-map map-in-order pair-for-each
    ;; Filtering and searching.
    filter partition remove find find-tail any every list-index
    take-while drop-while span break
    ;; Deletion and association lists.
    delete delete-duplicates alist-cons alist-copy alist-delete
    ;; Set operations on lists.
    lset<= lset= lset-adjoin lset-union lset-intersection lset-difference
    lset-xor lset-diff+intersection
    ;; Linear-update aliases (functional here, as the spec permits).
    take! drop-right! split-at! append! concatenate! reverse! append-reverse!
    map! filter! partition! remove! delete! delete-duplicates! alist-delete!
    take-while! span! break! append-map!
    lset-union! lset-intersection! lset-difference! lset-xor!
    lset-diff+intersection!)
  (import (scheme base) (scheme cxr) (srfi 1 native))
  (begin
    ;; Internal helpers over a list of lists. Not exported.
    (define (%srfi1-cars lists) (map car lists))
    (define (%srfi1-cdrs lists) (map cdr lists))
    (define (%srfi1-any-null? lists)
      (and (pair? lists)
           (or (null? (car lists)) (%srfi1-any-null? (cdr lists)))))
    (define (%srfi1-member? elt= x lst)
      (any (lambda (y) (elt= x y)) lst))
    (define (%srfi1-subset? elt= a b)
      (every (lambda (x) (%srfi1-member? elt= x b)) a))

    ;; Predicates.
    (define proper-list? list?)
    (define (not-pair? x) (not (pair? x)))
    (define (null-list? x)
      (cond ((pair? x) #f)
            ((null? x) #t)
            (else (error "null-list?: argument out of domain" x))))
    (define (list= elt= . lists)
      (or (null? lists)
          (let loop ((first (car lists)) (rest (cdr lists)))
            (or (null? rest)
                (let ((second (car rest)))
                  (and (%srfi1-list=2 elt= first second)
                       (loop second (cdr rest))))))))
    (define (%srfi1-list=2 elt= a b)
      (let loop ((a a) (b b))
        (cond ((null? a) (null? b))
              ((null? b) #f)
              ((elt= (car a) (car b)) (loop (cdr a) (cdr b)))
              (else #f))))

    ;; Constructors.
    (define (list-tabulate n init-proc)
      (let loop ((i (- n 1)) (acc '()))
        (if (< i 0) acc (loop (- i 1) (cons (init-proc i) acc)))))
    (define (circular-list elt1 . rest)
      (let ((elts (cons elt1 rest)))
        (set-cdr! (last-pair elts) elts)
        elts))
    (define (iota count . rest)
      (let ((start (if (pair? rest) (car rest) 0))
            (step (if (and (pair? rest) (pair? (cdr rest))) (cadr rest) 1)))
        (let loop ((i (- count 1)) (acc '()))
          (if (< i 0) acc (loop (- i 1) (cons (+ start (* i step)) acc))))))

    ;; Selectors.
    (define (first x) (car x))
    (define (second x) (cadr x))
    (define (third x) (caddr x))
    (define (fourth x) (cadddr x))
    (define (fifth x) (list-ref x 4))
    (define (sixth x) (list-ref x 5))
    (define (seventh x) (list-ref x 6))
    (define (eighth x) (list-ref x 7))
    (define (ninth x) (list-ref x 8))
    (define (tenth x) (list-ref x 9))
    (define (car+cdr p) (values (car p) (cdr p)))
    (define (drop lst i) (list-tail lst i))
    (define (split-at lst i) (values (take lst i) (drop lst i)))

    ;; Miscellaneous.
    (define (concatenate lists) (apply append lists))
    (define (zip . lists) (apply map list lists))
    (define (unzip1 lst) (map car lst))
    (define (unzip2 lst) (values (map car lst) (map cadr lst)))
    (define (unzip3 lst) (values (map car lst) (map cadr lst) (map caddr lst)))
    (define (unzip4 lst)
      (values (map car lst) (map cadr lst) (map caddr lst) (map cadddr lst)))
    (define (unzip5 lst)
      (values (map car lst) (map cadr lst) (map caddr lst) (map cadddr lst)
              (map (lambda (x) (list-ref x 4)) lst)))
    (define (count pred lst1 . rest)
      (let loop ((lists (cons lst1 rest)) (n 0))
        (if (%srfi1-any-null? lists)
            n
            (loop (%srfi1-cdrs lists)
                  (if (apply pred (%srfi1-cars lists)) (+ n 1) n)))))

    ;; Fold, unfold, and map.
    (define (fold kons knil lst1 . rest)
      (if (null? rest)
          (let loop ((acc knil) (lst lst1))
            (if (pair? lst) (loop (kons (car lst) acc) (cdr lst)) acc))
          (let loop ((acc knil) (lists (cons lst1 rest)))
            (if (%srfi1-any-null? lists)
                acc
                (loop (apply kons (append (%srfi1-cars lists) (list acc)))
                      (%srfi1-cdrs lists))))))
    (define (fold-right kons knil lst1 . rest)
      (if (null? rest)
          (let loop ((lst lst1))
            (if (pair? lst) (kons (car lst) (loop (cdr lst))) knil))
          (let loop ((lists (cons lst1 rest)))
            (if (%srfi1-any-null? lists)
                knil
                (apply kons (append (%srfi1-cars lists)
                                    (list (loop (%srfi1-cdrs lists)))))))))
    (define (reduce f ridentity lst)
      (if (null? lst) ridentity (fold f (car lst) (cdr lst))))
    (define (reduce-right f ridentity lst)
      (if (null? lst)
          ridentity
          (let recur ((head (car lst)) (rest (cdr lst)))
            (if (pair? rest) (f head (recur (car rest) (cdr rest))) head))))
    (define (pair-fold kons knil lst1 . rest)
      (let loop ((acc knil) (lists (cons lst1 rest)))
        (if (%srfi1-any-null? lists)
            acc
            (let ((tails (%srfi1-cdrs lists)))
              (loop (apply kons (append lists (list acc))) tails)))))
    (define (pair-fold-right kons knil lst1 . rest)
      (let loop ((lists (cons lst1 rest)))
        (if (%srfi1-any-null? lists)
            knil
            (apply kons (append lists (list (loop (%srfi1-cdrs lists))))))))
    (define (unfold p f g seed . maybe-tail)
      (let ((tail-gen (if (pair? maybe-tail) (car maybe-tail) (lambda (x) '()))))
        (let recur ((seed seed))
          (if (p seed) (tail-gen seed) (cons (f seed) (recur (g seed)))))))
    (define (unfold-right p f g seed . maybe-tail)
      (let loop ((seed seed) (acc (if (pair? maybe-tail) (car maybe-tail) '())))
        (if (p seed) acc (loop (g seed) (cons (f seed) acc)))))
    (define (append-map f . lists) (apply append (apply map f lists)))
    (define (filter-map f lst1 . rest)
      (let loop ((lists (cons lst1 rest)) (acc '()))
        (if (%srfi1-any-null? lists)
            (reverse acc)
            (let ((result (apply f (%srfi1-cars lists))))
              (loop (%srfi1-cdrs lists) (if result (cons result acc) acc))))))
    (define (map-in-order f lst1 . rest)
      (let loop ((lists (cons lst1 rest)) (acc '()))
        (if (%srfi1-any-null? lists)
            (reverse acc)
            (let ((value (apply f (%srfi1-cars lists))))
              (loop (%srfi1-cdrs lists) (cons value acc))))))
    (define (pair-for-each f lst1 . rest)
      (let loop ((lists (cons lst1 rest)))
        (if (%srfi1-any-null? lists)
            (if #f #f)
            (let ((tails (%srfi1-cdrs lists)))
              (apply f lists)
              (loop tails)))))

    ;; Filtering and searching.
    (define (filter pred lst)
      (let loop ((lst lst) (acc '()))
        (cond ((null? lst) (reverse acc))
              ((pred (car lst)) (loop (cdr lst) (cons (car lst) acc)))
              (else (loop (cdr lst) acc)))))
    (define (remove pred lst) (filter (lambda (x) (not (pred x))) lst))
    (define (partition pred lst)
      (let loop ((lst lst) (yes '()) (no '()))
        (cond ((null? lst) (values (reverse yes) (reverse no)))
              ((pred (car lst)) (loop (cdr lst) (cons (car lst) yes) no))
              (else (loop (cdr lst) yes (cons (car lst) no))))))
    (define (find-tail pred lst)
      (let loop ((lst lst))
        (cond ((not (pair? lst)) #f)
              ((pred (car lst)) lst)
              (else (loop (cdr lst))))))
    (define (find pred lst)
      (let ((tail (find-tail pred lst))) (and tail (car tail))))
    (define (any pred lst1 . rest)
      (let loop ((lists (cons lst1 rest)))
        (and (not (%srfi1-any-null? lists))
             (let ((vals (%srfi1-cars lists)) (tails (%srfi1-cdrs lists)))
               (if (%srfi1-any-null? tails)
                   (apply pred vals)
                   (or (apply pred vals) (loop tails)))))))
    (define (every pred lst1 . rest)
      (let loop ((lists (cons lst1 rest)))
        (if (%srfi1-any-null? lists)
            #t
            (let ((vals (%srfi1-cars lists)) (tails (%srfi1-cdrs lists)))
              (if (%srfi1-any-null? tails)
                  (apply pred vals)
                  (and (apply pred vals) (loop tails)))))))
    (define (list-index pred lst1 . rest)
      (let loop ((lists (cons lst1 rest)) (i 0))
        (if (%srfi1-any-null? lists)
            #f
            (if (apply pred (%srfi1-cars lists)) i
                (loop (%srfi1-cdrs lists) (+ i 1))))))
    (define (take-while pred lst)
      (let loop ((lst lst) (acc '()))
        (if (and (pair? lst) (pred (car lst)))
            (loop (cdr lst) (cons (car lst) acc))
            (reverse acc))))
    (define (drop-while pred lst)
      (let loop ((lst lst))
        (if (and (pair? lst) (pred (car lst))) (loop (cdr lst)) lst)))
    (define (span pred lst)
      (values (take-while pred lst) (drop-while pred lst)))
    (define (break pred lst)
      (span (lambda (x) (not (pred x))) lst))

    ;; Deletion and association lists.
    (define (delete x lst . maybe=)
      (let ((elt= (if (pair? maybe=) (car maybe=) equal?)))
        (filter (lambda (y) (not (elt= x y))) lst)))
    (define (delete-duplicates lst . maybe=)
      (let ((elt= (if (pair? maybe=) (car maybe=) equal?)))
        (let loop ((lst lst) (acc '()))
          (if (null? lst)
              (reverse acc)
              (let ((head (car lst)))
                (loop (cdr lst)
                      (if (any (lambda (kept) (elt= kept head)) acc)
                          acc
                          (cons head acc))))))))
    (define (alist-cons key datum alist) (cons (cons key datum) alist))
    (define (alist-copy alist)
      (map (lambda (pair) (cons (car pair) (cdr pair))) alist))
    (define (alist-delete key alist . maybe=)
      (let ((elt= (if (pair? maybe=) (car maybe=) equal?)))
        (filter (lambda (pair) (not (elt= key (car pair)))) alist)))

    ;; Set operations on lists.
    (define (lset<= elt= . lists)
      (or (null? lists)
          (let loop ((a (car lists)) (rest (cdr lists)))
            (or (null? rest)
                (let ((b (car rest)))
                  (and (%srfi1-subset? elt= a b) (loop b (cdr rest))))))))
    (define (lset= elt= . lists)
      (or (null? lists)
          (let loop ((a (car lists)) (rest (cdr lists)))
            (or (null? rest)
                (let ((b (car rest)))
                  (and (%srfi1-subset? elt= a b)
                       (%srfi1-subset? elt= b a)
                       (loop b (cdr rest))))))))
    (define (lset-adjoin elt= lst . elts)
      (fold (lambda (x result)
              (if (%srfi1-member? elt= x result) result (cons x result)))
            lst elts))
    (define (lset-union elt= . lists)
      (if (null? lists)
          '()
          (fold (lambda (lst result)
                  (fold (lambda (x result)
                          (if (%srfi1-member? elt= x result)
                              result
                              (cons x result)))
                        result lst))
                (car lists) (cdr lists))))
    (define (lset-intersection elt= list1 . rest)
      (if (null? rest)
          list1
          (filter (lambda (x)
                    (every (lambda (other) (%srfi1-member? elt= x other)) rest))
                  list1)))
    (define (lset-difference elt= list1 . rest)
      (if (null? rest)
          list1
          (filter (lambda (x)
                    (not (any (lambda (other) (%srfi1-member? elt= x other))
                              rest)))
                  list1)))
    (define (lset-xor elt= . lists)
      (fold (lambda (b a)
              (append
                (filter (lambda (x) (not (%srfi1-member? elt= x b))) a)
                (filter (lambda (x) (not (%srfi1-member? elt= x a))) b)))
            '() lists))
    (define (lset-diff+intersection elt= list1 . rest)
      (partition (lambda (x)
                   (not (any (lambda (other) (%srfi1-member? elt= x other))
                             rest)))
                 list1))

    ;; Linear-update aliases (functional here, as the spec permits).
    (define take! take)
    (define drop-right! drop-right)
    (define split-at! split-at)
    (define append! append)
    (define concatenate! concatenate)
    (define reverse! reverse)
    (define append-reverse! append-reverse)
    (define map! map)
    (define filter! filter)
    (define partition! partition)
    (define remove! remove)
    (define delete! delete)
    (define delete-duplicates! delete-duplicates)
    (define alist-delete! alist-delete)
    (define take-while! take-while)
    (define span! span)
    (define break! break)
    (define append-map! append-map)
    (define lset-union! lset-union)
    (define lset-intersection! lset-intersection)
    (define lset-difference! lset-difference)
    (define lset-xor! lset-xor)
    (define lset-diff+intersection! lset-diff+intersection)))
"#;

/// The Scheme library for `(srfi 2)`. `and-let*` is pure syntax: it expands to
/// core `let`, `and`, and `begin` forms that the compiler already lowers and
/// fuses, so there is nothing representation-dependent to make native and the
/// library carries no `(srfi 2 native)` companion.
///
/// The rules dispatch on the three claw shapes SRFI 2 allows: `(var expr)`
/// binds, `(expr)` tests, and a bare identifier tests a bound variable. Ordering
/// is deliberate. The no-body terminal cases come before the general recursive
/// cases, because a trailing claw with no body must yield the claw's value
/// rather than the recursive `#t`.
const SRFI_2_SOURCE: &str = r#"
(define-library (srfi 2)
  (export and-let*)
  (import (scheme base))
  (begin
    (define-syntax and-let*
      (syntax-rules ()
        ;; No claws.
        ((_ ()) #t)
        ((_ () body ...) (let () body ...))
        ;; Last claw, no body: yield the claw's value or test result.
        ((_ ((var expr))) expr)
        ((_ ((expr))) expr)
        ((_ (bound-var)) bound-var)
        ;; More claws follow: bind or test, guard on non-#f, recurse.
        ((_ ((var expr) rest ...) body ...)
         (let ((var expr)) (and var (and-let* (rest ...) body ...))))
        ((_ ((expr) rest ...) body ...)
         (and expr (and-let* (rest ...) body ...)))
        ((_ (bound-var rest ...) body ...)
         (and bound-var (and-let* (rest ...) body ...)))))))
"#;

/// The Scheme library for `(srfi 8)`. `receive` is pure syntax: it expands to a
/// `call-with-values` over a thunk and a `lambda`, both of which the compiler
/// already lowers, so there is nothing representation-dependent to make native
/// and the library carries no `(srfi 8 native)` companion.
///
/// The single rule handles all three `<formals>` shapes SRFI 8 permits, because
/// `formals` is spliced straight into `(lambda formals body ...)`: a proper list
/// binds each value, a bare identifier collects every value into a list, and a
/// dotted list binds the leading values and gathers the rest.
const SRFI_8_SOURCE: &str = r#"
(define-library (srfi 8)
  (export receive)
  (import (scheme base))
  (begin
    (define-syntax receive
      (syntax-rules ()
        ((receive formals expression body ...)
         (call-with-values (lambda () expression)
                           (lambda formals body ...)))))))
"#;

/// The Scheme wrapper library for `(srfi 26)`. `cut` and `cute` are pure syntax
/// and the library carries no `(srfi 26 native)` companion.
///
/// Both macros are self-recursive rather than delegating to a private helper
/// macro. A private helper would not be visible when the exported macro expands
/// in an importing library, and exporting it would leak an internal name. To
/// avoid that, each macro walks the argument list itself, using the internal
/// literal `<!>` to tell its recursive accumulator form apart from the entry
/// form a user writes. Each step consumes one `<slot-or-expr>`: a `<>` slot
/// contributes a fresh formal, an ordinary expression is spliced into the call
/// position, and a trailing `<...>` turns the result into a variable-arity
/// procedure via `apply`. `cute` additionally binds each non-slot expression in
/// an enclosing `let`, so it is evaluated once when the procedure is specialized
/// rather than on every call. Each recursive step introduces its own `x`, which
/// the expander freshens per expansion so the accumulated formals stay distinct.
const SRFI_26_SOURCE: &str = r#"
(define-library (srfi 26)
  (export cut cute)
  (import (scheme base))
  (begin
    (define-syntax cut
      (syntax-rules (<> <...> <!>)
        ((cut <!> (slot ...) (proc arg ...))
         (lambda (slot ...) (proc arg ...)))
        ((cut <!> (slot ...) (proc arg ...) <...>)
         (lambda (slot ... . rest) (apply proc arg ... rest)))
        ((cut <!> (slot ...) (pos ...) <> se ...)
         (cut <!> (slot ... x) (pos ... x) se ...))
        ((cut <!> (slot ...) (pos ...) nse se ...)
         (cut <!> (slot ...) (pos ... nse) se ...))
        ((cut se ...)
         (cut <!> () () se ...))))
    (define-syntax cute
      (syntax-rules (<> <...> <!>)
        ((cute <!> (slot ...) (bind ...) (proc arg ...))
         (let (bind ...) (lambda (slot ...) (proc arg ...))))
        ((cute <!> (slot ...) (bind ...) (proc arg ...) <...>)
         (let (bind ...) (lambda (slot ... . rest) (apply proc arg ... rest))))
        ((cute <!> (slot ...) (bind ...) (pos ...) <> se ...)
         (cute <!> (slot ... x) (bind ...) (pos ... x) se ...))
        ((cute <!> (slot ...) (bind ...) (pos ...) nse se ...)
         (cute <!> (slot ...) (bind ... (x nse)) (pos ... x) se ...))
        ((cute se ...)
         (cute <!> () () () se ...))))))
"#;

/// The Scheme wrapper library for `(srfi 27)`. It re-exports the native draw
/// and source procedures directly and defines only the pieces that must live in
/// Scheme: the generator constructors, which return closures, and
/// `default-random-source`, which is bound from a native accessor.
const SRFI_27_SOURCE: &str = r#"
(define-library (srfi 27)
  (export random-integer random-real default-random-source
          make-random-source random-source?
          random-source-state-ref random-source-state-set!
          random-source-randomize! random-source-pseudo-randomize!
          random-source-make-integers random-source-make-reals)
  (import (scheme base)
          (srfi 27 native))
  (begin
    (define default-random-source (%default-random-source))
    (define (random-source-make-integers s)
      (if (random-source? s)
          (lambda (n) (%random-integer-on s n))
          (error "random-source-make-integers: not a random source" s)))
    (define (random-source-make-reals s . maybe-unit)
      (cond
        ((not (random-source? s))
         (error "random-source-make-reals: not a random source" s))
        ((null? maybe-unit)
         (lambda () (%random-real-on s)))
        (else
         (let ((unit (car maybe-unit)))
           (if (and (real? unit) (< 0 unit) (< unit 1))
               (let ((m (max 1 (- (exact (floor (/ 1 unit))) 1))))
                 (lambda () (* unit (+ 1 (%random-integer-on s m)))))
               (error "random-source-make-reals: invalid unit" unit))))))))
"#;

/// The Scheme wrapper library for `(srfi 132)`, the Sort Libraries. Every
/// procedure takes a user comparison or equality argument that must be called
/// per element, and natives cannot reenter the VM to invoke a Scheme closure, so
/// the whole library is implemented in Scheme and there is no `(srfi 132 native)`
/// companion. Internal helpers use a `%` prefix and are not exported. The stable
/// vector sort is a top-down merge sort with a scratch buffer, list sorts route
/// through the vector sort, and selection uses an in-place quickselect. The
/// destructive `!` list procedures are functional aliases, which the SRFI
/// permits. The destructive vector procedures genuinely mutate their target.
const SRFI_132_SOURCE: &str = r#"
(define-library (srfi 132)
  (export
    list-sorted? vector-sorted?
    list-sort list-stable-sort list-sort! list-stable-sort!
    vector-sort vector-stable-sort vector-sort! vector-stable-sort!
    list-merge list-merge! vector-merge vector-merge!
    list-delete-neighbor-dups list-delete-neighbor-dups!
    vector-delete-neighbor-dups vector-delete-neighbor-dups!
    vector-find-median vector-find-median!
    vector-select! vector-separate!)
  (import (scheme base))
  (begin
    ;; Optional subrange parsing. The comparison argument is always named less?
    ;; or elt= internally so it never shadows the builtin < used for indices.
    (define (%start rest) (if (null? rest) 0 (car rest)))
    (define (%end rest v)
      (if (or (null? rest) (null? (cdr rest))) (vector-length v) (car (cdr rest))))
    (define (%opt rest n default)
      (if (> (length rest) n) (list-ref rest n) default))

    (define (%append-reverse rev tail)
      (if (null? rev) tail (%append-reverse (cdr rev) (cons (car rev) tail))))

    (define (%swap! v a b)
      (let ((t (vector-ref v a)))
        (vector-set! v a (vector-ref v b))
        (vector-set! v b t)))

    ;; Stable merge of src[lo,mid) and src[mid,hi) into dst[lo,hi).
    (define (%merge! less? src dst lo mid hi)
      (let loop ((i lo) (j mid) (k lo))
        (cond
          ((= i mid)
           (do ((j j (+ j 1)) (k k (+ k 1))) ((= j hi))
             (vector-set! dst k (vector-ref src j))))
          ((= j hi)
           (do ((i i (+ i 1)) (k k (+ k 1))) ((= i mid))
             (vector-set! dst k (vector-ref src i))))
          (else
           (let ((a (vector-ref src i)) (b (vector-ref src j)))
             ;; Take from the left run unless the right is strictly smaller, so
             ;; equal elements keep their original order (stability).
             (if (less? b a)
                 (begin (vector-set! dst k b) (loop i (+ j 1) (+ k 1)))
                 (begin (vector-set! dst k a) (loop (+ i 1) j (+ k 1)))))))))

    ;; Top-down stable merge sort of a[lo,hi) in place; b is scratch of a's size.
    (define (%msort! less? a lo hi b)
      (when (> (- hi lo) 1)
        (let ((mid (+ lo (quotient (- hi lo) 2))))
          (%msort! less? a lo mid b)
          (%msort! less? a mid hi b)
          (%merge! less? a b lo mid hi)
          (do ((k lo (+ k 1))) ((= k hi))
            (vector-set! a k (vector-ref b k))))))

    ;; Sort v[start,end) in place, stable.
    (define (%sort-range! less? v start end)
      (let ((n (- end start)))
        (when (> n 1)
          (let ((a (make-vector n)) (b (make-vector n)))
            (do ((i 0 (+ i 1))) ((= i n))
              (vector-set! a i (vector-ref v (+ start i))))
            (%msort! less? a 0 n b)
            (do ((i 0 (+ i 1))) ((= i n))
              (vector-set! v (+ start i) (vector-ref a i)))))))

    ;; Predicates.
    (define (list-sorted? less? lis)
      (or (null? lis)
          (let loop ((prev (car lis)) (rest (cdr lis)))
            (or (null? rest)
                (and (not (less? (car rest) prev))
                     (loop (car rest) (cdr rest)))))))
    (define (vector-sorted? less? v . rest)
      (let ((start (%start rest)) (end (%end rest v)))
        (let loop ((i (+ start 1)))
          (or (>= i end)
              (and (not (less? (vector-ref v i) (vector-ref v (- i 1))))
                   (loop (+ i 1)))))))

    ;; General vector sort.
    (define (vector-stable-sort! less? v . rest)
      (%sort-range! less? v (%start rest) (%end rest v)))
    (define vector-sort! vector-stable-sort!)
    (define (vector-stable-sort less? v . rest)
      (let* ((start (%start rest)) (end (%end rest v))
             (out (make-vector (- end start))))
        (do ((i 0 (+ i 1))) ((= i (- end start)))
          (vector-set! out i (vector-ref v (+ start i))))
        (%sort-range! less? out 0 (vector-length out))
        out))
    (define vector-sort vector-stable-sort)

    ;; General list sort, routed through the vector sort.
    (define (list-stable-sort less? lis)
      (let ((v (list->vector lis)))
        (%sort-range! less? v 0 (vector-length v))
        (vector->list v)))
    (define list-sort list-stable-sort)
    (define list-stable-sort! list-stable-sort)
    (define list-sort! list-stable-sort)

    ;; Stable merge, favouring the first data set on ties.
    (define (list-merge less? lis1 lis2)
      (let loop ((a lis1) (b lis2) (acc '()))
        (cond
          ((null? a) (%append-reverse acc b))
          ((null? b) (%append-reverse acc a))
          (else
           (let ((x (car a)) (y (car b)))
             (if (less? y x)
                 (loop a (cdr b) (cons y acc))
                 (loop (cdr a) b (cons x acc))))))))
    (define (list-merge! less? lis1 lis2)
      (cond
        ((null? lis1) lis2)
        ((null? lis2) lis1)
        (else
         (let* ((take-second? (less? (car lis2) (car lis1)))
                (head (if take-second? lis2 lis1))
                (first (if take-second? lis1 (cdr lis1)))
                (second (if take-second? (cdr lis2) lis2)))
           (let loop ((tail head) (first first) (second second))
             (cond
               ((null? first)
                (set-cdr! tail second)
                head)
               ((null? second)
                (set-cdr! tail first)
                head)
               ((less? (car second) (car first))
                (set-cdr! tail second)
                (loop second first (cdr second)))
               (else
                (set-cdr! tail first)
                (loop first (cdr first) second))))))))
    (define (%vmerge! less? dst dstart v1 s1 e1 v2 s2 e2)
      (let loop ((i s1) (j s2) (k dstart))
        (cond
          ((= i e1)
           (do ((j j (+ j 1)) (k k (+ k 1))) ((= j e2))
             (vector-set! dst k (vector-ref v2 j))))
          ((= j e2)
           (do ((i i (+ i 1)) (k k (+ k 1))) ((= i e1))
             (vector-set! dst k (vector-ref v1 i))))
          (else
           (let ((a (vector-ref v1 i)) (b (vector-ref v2 j)))
             (if (less? b a)
                 (begin (vector-set! dst k b) (loop i (+ j 1) (+ k 1)))
                 (begin (vector-set! dst k a) (loop (+ i 1) j (+ k 1)))))))))
    (define (vector-merge less? v1 v2 . rest)
      (let* ((s1 (%opt rest 0 0))
             (e1 (%opt rest 1 (vector-length v1)))
             (s2 (%opt rest 2 0))
             (e2 (%opt rest 3 (vector-length v2)))
             (out (make-vector (+ (- e1 s1) (- e2 s2)))))
        (%vmerge! less? out 0 v1 s1 e1 v2 s2 e2)
        out))
    (define (vector-merge! less? to from1 from2 . rest)
      (let ((start (%opt rest 0 0))
            (s1 (%opt rest 1 0))
            (e1 (%opt rest 2 (vector-length from1)))
            (s2 (%opt rest 3 0))
            (e2 (%opt rest 4 (vector-length from2))))
        (%vmerge! less? to start from1 s1 e1 from2 s2 e2)))

    ;; Deleting duplicate neighbors. Equality is invoked as (elt= x y) with x
    ;; before y; the first element of each equal run survives.
    (define (list-delete-neighbor-dups elt= lis)
      (if (null? lis)
          '()
          (let loop ((prev (car lis)) (rest (cdr lis)) (acc (list (car lis))))
            (if (null? rest)
                (reverse acc)
                (let ((x (car rest)))
                  (if (elt= prev x)
                      (loop prev (cdr rest) acc)
                      (loop x (cdr rest) (cons x acc))))))))
    (define (list-delete-neighbor-dups! elt= lis)
      (if (null? lis)
          '()
          (let loop ((kept lis) (rest (cdr lis)))
            (cond
              ((null? rest)
               (set-cdr! kept '())
               lis)
              ((elt= (car kept) (car rest))
               (loop kept (cdr rest)))
              (else
               (set-cdr! kept rest)
               (loop rest (cdr rest)))))))
    (define (vector-delete-neighbor-dups elt= v . rest)
      (let ((start (%start rest)) (end (%end rest v)))
        (if (>= start end)
            (make-vector 0)
            (let ((tmp (make-vector (- end start))))
              (vector-set! tmp 0 (vector-ref v start))
              (let loop ((i (+ start 1)) (prev (vector-ref v start)) (w 1))
                (if (= i end)
                    (vector-copy tmp 0 w)
                    (let ((x (vector-ref v i)))
                      (if (elt= prev x)
                          (loop (+ i 1) prev w)
                          (begin (vector-set! tmp w x)
                                 (loop (+ i 1) x (+ w 1)))))))))))
    (define (vector-delete-neighbor-dups! elt= v . rest)
      (let ((start (%start rest)) (end (%end rest v)))
        (if (>= start end)
            start
            (let loop ((i (+ start 1)) (prev (vector-ref v start)) (w (+ start 1)))
              (if (= i end)
                  w
                  (let ((x (vector-ref v i)))
                    (if (elt= prev x)
                        (loop (+ i 1) prev w)
                        (begin (vector-set! v w x)
                               (loop (+ i 1) x (+ w 1))))))))))

    ;; Median.
    (define (%default-mean a b) (/ (+ a b) 2))
    (define (%median-of-sorted v n knil mean)
      (cond
        ((= n 0) knil)
        ((odd? n) (vector-ref v (quotient n 2)))
        (else (mean (vector-ref v (- (quotient n 2) 1))
                    (vector-ref v (quotient n 2))))))
    (define (vector-find-median! less? v knil . rest)
      (let ((mean (if (pair? rest) (car rest) %default-mean))
            (n (vector-length v)))
        (%sort-range! less? v 0 n)
        (%median-of-sorted v n knil mean)))
    (define (vector-find-median less? v knil . rest)
      (let ((mean (if (pair? rest) (car rest) %default-mean)))
        (let* ((n (vector-length v)) (copy (vector-copy v)))
          (%sort-range! less? copy 0 n)
          (%median-of-sorted copy n knil mean))))

    ;; Selection via in-place quickselect over [start,end).
    (define (%partition! less? v lo hi)
      (let ((pivot (vector-ref v hi)))
        (let loop ((i lo) (j lo))
          (if (= j hi)
              (begin (%swap! v i hi) i)
              (if (less? (vector-ref v j) pivot)
                  (begin (%swap! v i j) (loop (+ i 1) (+ j 1)))
                  (loop i (+ j 1)))))))
    (define (%quickselect! less? v ti lo hi)
      (if (= lo hi)
          (vector-ref v lo)
          (let ((p (%partition! less? v lo hi)))
            (cond
              ((= ti p) (vector-ref v p))
              ((< ti p) (%quickselect! less? v ti lo (- p 1)))
              (else (%quickselect! less? v ti (+ p 1) hi))))))
    (define (vector-select! less? v k . rest)
      (let ((start (%start rest)) (end (%end rest v)))
        (%quickselect! less? v (+ start k) start (- end 1))))
    (define (vector-separate! less? v k . rest)
      (let ((start (%start rest)) (end (%end rest v)))
        (when (and (> k 0) (< k (- end start)))
          (%quickselect! less? v (+ start (- k 1)) start (- end 1)))))))
"#;

/// Builds the public library name `(srfi 132)`. SRFI 132 has no native companion.
fn srfi_132_public_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(132),
    ])
}

/// Builds the public library name `(srfi 2)`. SRFI 2 has no native companion.
fn srfi_2_public_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(2),
    ])
}

/// Builds the public library name `(srfi 8)`. SRFI 8 has no native companion.
fn srfi_8_public_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(8),
    ])
}

/// Builds the public library name `(srfi 26)`. SRFI 26 has no native companion.
fn srfi_26_public_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(26),
    ])
}

/// Builds the private native library name `(srfi 1 native)`.
fn srfi_1_native_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(1),
        LibraryNameComponent::identifier("native"),
    ])
}

/// Builds the public library name `(srfi 1)`.
fn srfi_1_public_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(1),
    ])
}

/// Builds the private native library name `(srfi 27 native)`.
fn srfi_27_native_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(27),
        LibraryNameComponent::identifier("native"),
    ])
}

/// Builds the public library name `(srfi 27)`.
fn srfi_27_public_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(27),
    ])
}

/// The Scheme wrapper library for `(srfi 48)`. The template walk is fully
/// native and always builds a string, so the wrapper only implements the
/// destination dispatch of the optional first argument: a string first
/// argument is the template itself, `#f` returns the string, `#t` writes it to
/// the current output port, and an output port receives it directly.
const SRFI_48_SOURCE: &str = r#"
(define-library (srfi 48)
  (export format)
  (import (scheme base) (srfi 48 native))
  (begin
    (define (format first . rest)
      (cond
        ((string? first) (apply %format48 first rest))
        ((eq? first #f) (apply %format48 rest))
        ((eq? first #t) (write-string (apply %format48 rest)))
        ((output-port? first) (write-string (apply %format48 rest) first))
        ;; Anything else is neither a destination nor a template. Handing it to
        ;; the native raises the same type error a non-string template gets.
        (else (apply %format48 first rest))))))
"#;

/// The `(srfi 28)` compatibility library registered by the SRFI 48 install.
/// SRFI 48 is the upward-compatible revision of SRFI 28, so the richer
/// `format` satisfies the SRFI 28 contract unchanged.
const SRFI_28_COMPAT_SOURCE: &str = r#"
(define-library (srfi 28)
  (export format)
  (import (srfi 48)))
"#;

/// The Scheme wrapper library for `(srfi 69)`. The four hash functions are
/// native and re-exported from `(srfi 69 native)`. Everything else takes a user
/// comparison, hash function, or callback, none of which a native can invoke, so
/// the table lives here as a record over a vector of bucket alists, adapted from
/// the SRFI 69 reference implementation. It uses R7RS `define-record-type` and
/// `error` in place of the reference's SRFI 9 and SRFI 23. Internal helpers use a
/// `%` prefix and the record accessors that are not SRFI names stay unexported.
const SRFI_69_SOURCE: &str = r#"
(define-library (srfi 69)
  (export make-hash-table hash-table? alist->hash-table
          hash-table-equivalence-function hash-table-hash-function
          hash-table-ref hash-table-ref/default hash-table-set!
          hash-table-delete! hash-table-exists?
          hash-table-update! hash-table-update!/default
          hash-table-size hash-table-keys hash-table-values
          hash-table-walk hash-table-fold hash-table->alist
          hash-table-copy hash-table-merge!
          hash string-hash string-ci-hash hash-by-identity)
  (import (scheme base) (scheme char) (srfi 69 native))
  (begin
    (define %default-table-size 64)

    ;; A bucket entry is a mutable (key . value) pair.
    (define %make-hash-node cons)
    (define (%hash-node-set-value! node value) (set-cdr! node value))
    (define %hash-node-key car)
    (define %hash-node-value cdr)

    ;; A hash table is a disjoint record type. Only the three reflective
    ;; accessors are exported (hash-table-size, hash-table-hash-function,
    ;; hash-table-equivalence-function). The constructor and the remaining
    ;; accessors are %-prefixed and stay unexported.
    (define-record-type <srfi-69-hash-table>
      (%make-hash-table size hash compare associate entries)
      hash-table?
      (size hash-table-size %hash-table-set-size!)
      (hash hash-table-hash-function)
      (compare hash-table-equivalence-function)
      (associate %hash-table-association-function)
      (entries %hash-table-entries %hash-table-set-entries!))

    (define (%appropriate-hash-function-for comparison)
      (or (and (eq? comparison eq?) hash-by-identity)
          (and (eq? comparison string=?) string-hash)
          (and (eq? comparison string-ci=?) string-ci-hash)
          hash))

    (define (make-hash-table . args)
      (let* ((comparison (if (null? args) equal? (car args)))
             (hash-function
               (if (or (null? args) (null? (cdr args)))
                   (%appropriate-hash-function-for comparison) (cadr args)))
             (size
               (if (or (null? args) (null? (cdr args)) (null? (cddr args)))
                   %default-table-size (caddr args)))
             (association
               (or (and (eq? comparison eq?) assq)
                   (and (eq? comparison eqv?) assv)
                   (and (eq? comparison equal?) assoc)
                   (letrec
                     ((associate
                        (lambda (value alist)
                          (cond ((null? alist) #f)
                                ((comparison value (caar alist)) (car alist))
                                (else (associate value (cdr alist)))))))
                     associate))))
        (%make-hash-table 0 hash-function comparison association
                          (make-vector
                            (if (<= size 0) %default-table-size size)
                            '()))))

    (define (%hash-table-hash hash-table key)
      ((hash-table-hash-function hash-table)
         key (vector-length (%hash-table-entries hash-table))))

    (define (%hash-table-find entries associate hash key)
      (associate key (vector-ref entries hash)))

    (define (%hash-table-add! entries hash key value)
      (vector-set! entries hash
                   (cons (%make-hash-node key value)
                         (vector-ref entries hash))))

    (define (%hash-table-remove! entries compare hash key)
      (let ((bucket (vector-ref entries hash)))
        (cond ((null? bucket) #f)
              ((compare key (caar bucket))
               (vector-set! entries hash (cdr bucket)) #t)
              (else
                (let loop ((current (cdr bucket)) (previous bucket))
                  (cond ((null? current) #f)
                        ((compare key (caar current))
                         (set-cdr! previous (cdr current)) #t)
                        (else (loop (cdr current) current))))))))

    (define (%hash-table-walk-nodes proc entries)
      (do ((index (- (vector-length entries) 1) (- index 1)))
          ((< index 0))
        (for-each proc (vector-ref entries index))))

    (define (%hash-table-maybe-resize! hash-table)
      (let* ((old-entries (%hash-table-entries hash-table))
             (old-length (vector-length old-entries)))
        (if (> (hash-table-size hash-table) old-length)
            (let* ((new-length (* 2 old-length))
                   (new-entries (make-vector new-length '()))
                   (hash-function (hash-table-hash-function hash-table)))
              (%hash-table-walk-nodes
                (lambda (node)
                  (%hash-table-add! new-entries
                                    (hash-function (%hash-node-key node) new-length)
                                    (%hash-node-key node) (%hash-node-value node)))
                old-entries)
              (%hash-table-set-entries! hash-table new-entries)))))

    (define (hash-table-ref hash-table key . maybe-default)
      (cond ((%hash-table-find (%hash-table-entries hash-table)
                               (%hash-table-association-function hash-table)
                               (%hash-table-hash hash-table key) key)
             => %hash-node-value)
            ((null? maybe-default)
             (error "hash-table-ref: no value associated with key" key))
            (else ((car maybe-default)))))

    (define (hash-table-ref/default hash-table key default)
      (hash-table-ref hash-table key (lambda () default)))

    (define (hash-table-set! hash-table key value)
      (let ((hash (%hash-table-hash hash-table key))
            (entries (%hash-table-entries hash-table)))
        (cond ((%hash-table-find entries
                                 (%hash-table-association-function hash-table)
                                 hash key)
               => (lambda (node) (%hash-node-set-value! node value)))
              (else (%hash-table-add! entries hash key value)
                    (%hash-table-set-size! hash-table
                                          (+ 1 (hash-table-size hash-table)))
                    (%hash-table-maybe-resize! hash-table)))))

    (define (hash-table-update! hash-table key function . maybe-default)
      (let ((hash (%hash-table-hash hash-table key))
            (entries (%hash-table-entries hash-table)))
        (cond ((%hash-table-find entries
                                 (%hash-table-association-function hash-table)
                                 hash key)
               => (lambda (node)
                    (%hash-node-set-value!
                      node (function (%hash-node-value node)))))
              ((null? maybe-default)
               (error "hash-table-update!: no value exists for key" key))
              (else (%hash-table-add! entries hash key
                                      (function ((car maybe-default))))
                    (%hash-table-set-size! hash-table
                                          (+ 1 (hash-table-size hash-table)))
                    (%hash-table-maybe-resize! hash-table)))))

    (define (hash-table-update!/default hash-table key function default)
      (hash-table-update! hash-table key function (lambda () default)))

    (define (hash-table-delete! hash-table key)
      (if (%hash-table-remove! (%hash-table-entries hash-table)
                               (hash-table-equivalence-function hash-table)
                               (%hash-table-hash hash-table key) key)
          (%hash-table-set-size! hash-table (- (hash-table-size hash-table) 1))))

    (define (hash-table-exists? hash-table key)
      (and (%hash-table-find (%hash-table-entries hash-table)
                             (%hash-table-association-function hash-table)
                             (%hash-table-hash hash-table key) key)
           #t))

    (define (hash-table-walk hash-table proc)
      (%hash-table-walk-nodes
        (lambda (node) (proc (%hash-node-key node) (%hash-node-value node)))
        (%hash-table-entries hash-table)))

    (define (hash-table-fold hash-table f accumulator)
      (hash-table-walk hash-table
                       (lambda (key value)
                         (set! accumulator (f key value accumulator))))
      accumulator)

    (define (alist->hash-table alist . args)
      (let* ((comparison (if (null? args) equal? (car args)))
             (hash-function
               (if (or (null? args) (null? (cdr args)))
                   (%appropriate-hash-function-for comparison) (cadr args)))
             (size
               (if (or (null? args) (null? (cdr args)) (null? (cddr args)))
                   (max %default-table-size (* 2 (length alist))) (caddr args)))
             (hash-table (make-hash-table comparison hash-function size)))
        (for-each
          (lambda (entry)
            (hash-table-update!/default
              hash-table (car entry) (lambda (existing) existing) (cdr entry)))
          alist)
        hash-table))

    (define (hash-table->alist hash-table)
      (hash-table-fold hash-table
                       (lambda (key value acc) (cons (cons key value) acc)) '()))

    (define (hash-table-copy hash-table)
      (let ((copy (make-hash-table (hash-table-equivalence-function hash-table)
                                   (hash-table-hash-function hash-table)
                                   (max %default-table-size
                                        (* 2 (hash-table-size hash-table))))))
        (hash-table-walk hash-table
                         (lambda (key value) (hash-table-set! copy key value)))
        copy))

    (define (hash-table-merge! hash-table1 hash-table2)
      (hash-table-walk
        hash-table2
        (lambda (key value) (hash-table-set! hash-table1 key value)))
      hash-table1)

    (define (hash-table-keys hash-table)
      (hash-table-fold hash-table (lambda (key value acc) (cons key acc)) '()))

    (define (hash-table-values hash-table)
      (hash-table-fold hash-table (lambda (key value acc) (cons value acc)) '()))))
"#;

/// The Scheme wrapper library for `(srfi 151)`. The 30 pure bit operations are
/// native and re-exported directly. The five aggregate conversions and the four
/// callback or generator procedures are defined here on top of those natives,
/// because they build heap sequences or call a user procedure. Internal helpers
/// use a `%` prefix and are not exported.
const SRFI_151_SOURCE: &str = r#"
(define-library (srfi 151)
  (export
    ;; Re-exported native bit operations from (srfi 151 native).
    bitwise-not bitwise-and bitwise-ior bitwise-xor bitwise-eqv
    bitwise-nand bitwise-nor bitwise-andc1 bitwise-andc2 bitwise-orc1 bitwise-orc2
    arithmetic-shift bit-count integer-length bitwise-if
    bit-set? copy-bit bit-swap any-bit-set? every-bit-set? first-set-bit
    bit-field bit-field-any? bit-field-every? bit-field-clear bit-field-set
    bit-field-replace bit-field-replace-same bit-field-rotate bit-field-reverse
    ;; Defined in this wrapper.
    bits->list bits->vector list->bits vector->bits bits
    bitwise-fold bitwise-for-each bitwise-unfold make-bitwise-generator)
  (import (scheme base) (srfi 151 native))
  (begin
    ;; Bits conversion. Bit 0 is the first element, little-endian.
    (define (bits->list i . rest)
      (let ((len (if (pair? rest) (car rest) (integer-length i))))
        (let loop ((k (- len 1)) (acc '()))
          (if (< k 0)
              acc
              (loop (- k 1) (cons (bit-set? k i) acc))))))

    (define (bits->vector i . rest)
      (list->vector (apply bits->list i rest)))

    (define (%booleans->integer bools)
      (let loop ((bools bools) (k 0) (acc 0))
        (if (pair? bools)
            (let ((bit (car bools)))
              (if (boolean? bit)
                  (loop (cdr bools)
                        (+ k 1)
                        (if bit
                            (bitwise-ior acc (arithmetic-shift 1 k))
                            acc))
                  (error "expected boolean bit" bit)))
            acc)))

    (define (list->bits lst) (%booleans->integer lst))
    (define (vector->bits vec) (%booleans->integer (vector->list vec)))
    (define (bits . bools) (%booleans->integer bools))

    ;; Fold, for-each, unfold, and generate over the bits of an integer.
    (define (bitwise-fold proc seed i)
      (let ((len (integer-length i)))
        (let loop ((k 0) (acc seed))
          (if (< k len)
              (loop (+ k 1) (proc (bit-set? k i) acc))
              acc))))

    (define (bitwise-for-each proc i)
      (let ((len (integer-length i)))
        (let loop ((k 0))
          (when (< k len)
            (proc (bit-set? k i))
            (loop (+ k 1))))))

    (define (bitwise-unfold stop? mapper successor seed)
      (let loop ((k 0) (state seed) (acc 0))
        (if (stop? state)
            acc
            (loop (+ k 1)
                  (successor state)
                  (if (mapper state)
                      (bitwise-ior acc (arithmetic-shift 1 k))
                      acc)))))

    (define (make-bitwise-generator i)
      (let ((k 0))
        (lambda ()
          (let ((bit (bit-set? k i)))
            (set! k (+ k 1))
            bit))))))
"#;

/// The Scheme wrapper library for `(srfi 152)`, the reduced String Library. The
/// R7RS-small string procedures are re-exported from `(scheme base)` and
/// `(scheme char)` unchanged. The representation-dependent scans and builders
/// that take no callback are native and re-exported from `(srfi 152 native)`.
/// Everything that takes a predicate, mapper, or fold procedure is defined here,
/// because a native cannot call back into a Scheme argument. Internal helpers use
/// a `%` prefix and are not exported. `string-map` and `string-for-each` are the
/// R7RS-small ones re-exported unchanged, so the common `(import (srfi 152)
/// (scheme base))` needs no renames.
const SRFI_152_SOURCE: &str = r#"
(define-library (srfi 152)
  (export
    ;; Re-exported from (scheme base) and (scheme char).
    string? make-string string string-length string-ref substring
    string-copy string-copy! string-fill! string-set!
    string->list list->string string->vector vector->string
    string-append string-for-each read-string write-string
    string=? string<? string>? string<=? string>=?
    string-ci=? string-ci<? string-ci>? string-ci<=? string-ci>=?
    ;; Native scans and builders from (srfi 152 native).
    string-null? reverse-list->string
    string-prefix-length string-suffix-length string-prefix? string-suffix?
    string-contains string-contains-right
    string-concatenate string-replicate
    ;; Defined in this wrapper.
    string-every string-any
    string-tabulate string-unfold string-unfold-right
    string-take string-drop string-take-right string-drop-right
    string-pad string-pad-right
    string-trim string-trim-right string-trim-both
    string-replace
    string-index string-index-right string-skip string-skip-right
    string-take-while string-take-while-right
    string-drop-while string-drop-while-right
    string-break string-span
    string-concatenate-reverse string-join
    string-fold string-fold-right string-count string-filter string-remove
    string-map
    string-segment string-split)
  (import (scheme base) (scheme char) (srfi 152 native))
  (begin
    ;; Optional-argument helpers. start defaults to 0 and end to the length.
    (define (%start rest) (if (pair? rest) (car rest) 0))
    (define (%end rest s)
      (if (and (pair? rest) (pair? (cdr rest))) (cadr rest) (string-length s)))
    (define (%opt rest n default)
      (if (> (length rest) n) (list-ref rest n) default))
    ;; Coerce a mapper result (a character or a string) to a string.
    (define (%->string x) (if (char? x) (string x) x))

    ;; Predicates.
    (define (string-every pred s . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let loop ((i start) (last #t))
          (if (>= i end)
              last
              (let ((v (pred (string-ref s i))))
                (if v (loop (+ i 1) v) #f))))))
    (define (string-any pred s . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let loop ((i start))
          (if (>= i end)
              #f
              (let ((v (pred (string-ref s i))))
                (if v v (loop (+ i 1))))))))

    ;; Constructors.
    (define (string-tabulate proc len)
      (let loop ((i (- len 1)) (acc '()))
        (if (< i 0) (list->string acc) (loop (- i 1) (cons (proc i) acc)))))
    (define (string-unfold stop? mapper successor seed . rest)
      (let ((base (%opt rest 0 "")) (make-final (%opt rest 1 (lambda (x) ""))))
        (let loop ((seed seed) (chunks (list (%->string base))))
          (if (stop? seed)
              (string-concatenate
               (reverse (cons (%->string (make-final seed)) chunks)))
              (loop (successor seed) (cons (%->string (mapper seed)) chunks))))))
    (define (string-unfold-right stop? mapper successor seed . rest)
      (let ((base (%opt rest 0 "")) (make-final (%opt rest 1 (lambda (x) ""))))
        (let loop ((seed seed) (chunks (list (%->string base))))
          (if (stop? seed)
              (string-concatenate (cons (%->string (make-final seed)) chunks))
              (loop (successor seed) (cons (%->string (mapper seed)) chunks))))))

    ;; Selection.
    (define (string-take s n) (substring s 0 n))
    (define (string-drop s n) (substring s n (string-length s)))
    (define (string-take-right s n)
      (let ((len (string-length s))) (substring s (- len n) len)))
    (define (string-drop-right s n) (substring s 0 (- (string-length s) n)))
    (define (string-pad s n . rest)
      (let* ((char (%opt rest 0 #\space)) (start (%opt rest 1 0))
             (end (%opt rest 2 (string-length s)))
             (sub (substring s start end)) (slen (- end start)))
        (if (>= slen n)
            (substring sub (- slen n) slen)
            (string-append (make-string (- n slen) char) sub))))
    (define (string-pad-right s n . rest)
      (let* ((char (%opt rest 0 #\space)) (start (%opt rest 1 0))
             (end (%opt rest 2 (string-length s)))
             (sub (substring s start end)) (slen (- end start)))
        (if (>= slen n)
            (substring sub 0 n)
            (string-append sub (make-string (- n slen) char)))))
    (define (string-trim s . rest)
      (let* ((pred (%opt rest 0 char-whitespace?)) (start (%opt rest 1 0))
             (end (%opt rest 2 (string-length s))))
        (let loop ((i start))
          (cond ((>= i end) "")
                ((pred (string-ref s i)) (loop (+ i 1)))
                (else (substring s i end))))))
    (define (string-trim-right s . rest)
      (let* ((pred (%opt rest 0 char-whitespace?)) (start (%opt rest 1 0))
             (end (%opt rest 2 (string-length s))))
        (let loop ((i end))
          (cond ((<= i start) "")
                ((pred (string-ref s (- i 1))) (loop (- i 1)))
                (else (substring s start i))))))
    (define (string-trim-both s . rest)
      (let* ((pred (%opt rest 0 char-whitespace?)) (start (%opt rest 1 0))
             (end (%opt rest 2 (string-length s)))
             (left (string-trim (substring s start end) pred)))
        (string-trim-right left pred)))

    ;; Replacement.
    (define (string-replace s1 s2 start1 end1 . rest)
      (let ((start2 (%opt rest 0 0)) (end2 (%opt rest 1 (string-length s2))))
        (string-append (substring s1 0 start1)
                       (substring s2 start2 end2)
                       (substring s1 end1 (string-length s1)))))

    ;; Searching.
    (define (string-index s pred . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let loop ((i start))
          (cond ((>= i end) #f)
                ((pred (string-ref s i)) i)
                (else (loop (+ i 1)))))))
    (define (string-index-right s pred . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let loop ((i end))
          (cond ((<= i start) #f)
                ((pred (string-ref s (- i 1))) (- i 1))
                (else (loop (- i 1)))))))
    (define (string-skip s pred . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let loop ((i start))
          (cond ((>= i end) #f)
                ((pred (string-ref s i)) (loop (+ i 1)))
                (else i)))))
    (define (string-skip-right s pred . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let loop ((i end))
          (cond ((<= i start) #f)
                ((pred (string-ref s (- i 1))) (loop (- i 1)))
                (else (- i 1))))))
    (define (string-take-while s pred . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let ((k (string-skip s pred start end)))
          (substring s start (or k end)))))
    (define (string-drop-while s pred . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let ((k (string-skip s pred start end)))
          (substring s (or k end) end))))
    (define (string-take-while-right s pred . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let ((k (string-skip-right s pred start end)))
          (substring s (if k (+ k 1) start) end))))
    (define (string-drop-while-right s pred . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let ((k (string-skip-right s pred start end)))
          (substring s start (if k (+ k 1) start)))))
    (define (string-span s pred . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let ((k (or (string-skip s pred start end) end)))
          (values (substring s start k) (substring s k end)))))
    (define (string-break s pred . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let ((k (or (string-index s pred start end) end)))
          (values (substring s start k) (substring s k end)))))

    ;; Concatenation.
    (define (string-concatenate-reverse string-list . rest)
      (let* ((final (%opt rest 0 "")) (end (%opt rest 1 (string-length final))))
        (string-concatenate
         (reverse (cons (substring final 0 end) string-list)))))
    (define (string-join string-list . rest)
      (let ((delim (%opt rest 0 " ")) (grammar (%opt rest 1 'infix)))
        (case grammar
          ((infix strict-infix)
           (if (null? string-list)
               (if (eq? grammar 'strict-infix)
                   (error "string-join: empty list with strict-infix grammar")
                   "")
               (let loop ((lst (cdr string-list)) (acc (car string-list)))
                 (if (null? lst)
                     acc
                     (loop (cdr lst) (string-append acc delim (car lst)))))))
          ((suffix)
           (let loop ((lst string-list) (acc ""))
             (if (null? lst) acc (loop (cdr lst) (string-append acc (car lst) delim)))))
          ((prefix)
           (let loop ((lst string-list) (acc ""))
             (if (null? lst) acc (loop (cdr lst) (string-append acc delim (car lst))))))
          (else (error "string-join: invalid grammar" grammar)))))

    ;; Fold, map, and friends.
    (define (string-fold kons knil s . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let loop ((i start) (acc knil))
          (if (>= i end) acc (loop (+ i 1) (kons (string-ref s i) acc))))))
    (define (string-fold-right kons knil s . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let loop ((i end) (acc knil))
          (if (<= i start) acc (loop (- i 1) (kons (string-ref s (- i 1)) acc))))))
    (define (string-count s pred . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let loop ((i start) (n 0))
          (cond ((>= i end) n)
                ((pred (string-ref s i)) (loop (+ i 1) (+ n 1)))
                (else (loop (+ i 1) n))))))
    (define (string-filter pred s . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let loop ((i start) (acc '()))
          (cond ((>= i end) (list->string (reverse acc)))
                ((pred (string-ref s i)) (loop (+ i 1) (cons (string-ref s i) acc)))
                (else (loop (+ i 1) acc))))))
    (define (string-remove pred s . rest)
      (let ((start (%start rest)) (end (%end rest s)))
        (let loop ((i start) (acc '()))
          (cond ((>= i end) (list->string (reverse acc)))
                ((pred (string-ref s i)) (loop (+ i 1) acc))
                (else (loop (+ i 1) (cons (string-ref s i) acc)))))))

    ;; Replication and splitting.
    (define (string-segment s k)
      (if (<= k 0)
          (error "string-segment: segment length must be positive" k)
          (let ((len (string-length s)))
            (let loop ((i 0) (acc '()))
              (if (>= i len)
                  (reverse acc)
                  (loop (+ i k) (cons (substring s i (min (+ i k) len)) acc)))))))
    (define (%strip-last-empty parts)
      (cond ((null? parts) parts)
            ((and (null? (cdr parts)) (string=? (car parts) "")) '())
            (else (cons (car parts) (%strip-last-empty (cdr parts))))))
    (define (%split-each-char s from end limit)
      (let loop ((i from) (splits 0) (acc '()))
        (cond ((>= i end) (reverse acc))
              ((and limit (>= splits limit)) (reverse (cons (substring s i end) acc)))
              (else (loop (+ i 1) (+ splits 1) (cons (substring s i (+ i 1)) acc))))))
    (define (string-split s delimiter . rest)
      (let* ((grammar (%opt rest 0 'infix)) (limit (%opt rest 1 #f))
             (start (%opt rest 2 0)) (end (%opt rest 3 (string-length s)))
             (dlen (string-length delimiter)))
        (define (finish parts)
          (case grammar
            ((infix strict-infix) parts)
            ((prefix) (if (and (pair? parts) (string=? (car parts) "")) (cdr parts) parts))
            ((suffix) (%strip-last-empty parts))
            (else (error "string-split: invalid grammar" grammar))))
        (cond
          ((= start end)
           (if (eq? grammar 'strict-infix)
               (error "string-split: empty string with strict-infix grammar")
               '()))
          ((= dlen 0) (finish (%split-each-char s start end limit)))
          (else
           (finish
            (let loop ((from start) (splits 0) (acc '()))
              (if (and limit (>= splits limit))
                  (reverse (cons (substring s from end) acc))
                  (let ((hit (string-contains s delimiter from end 0 dlen)))
                    (if hit
                        (loop (+ hit dlen) (+ splits 1) (cons (substring s from hit) acc))
                        (reverse (cons (substring s from end) acc)))))))))))))
"#;

/// The public `(srfi 175)` library. All procedures are re-exported from the
/// private native library because none accepts a Scheme callback.
const SRFI_175_SOURCE: &str = r#"
(define-library (srfi 175)
  (export
    ascii-codepoint? ascii-bytevector? ascii-char? ascii-string?
    ascii-control? ascii-non-control? ascii-whitespace? ascii-space-or-tab?
    ascii-other-graphic? ascii-upper-case? ascii-lower-case?
    ascii-alphabetic? ascii-alphanumeric? ascii-numeric?
    ascii-digit-value ascii-upper-case-value ascii-lower-case-value
    ascii-nth-digit ascii-nth-upper-case ascii-nth-lower-case
    ascii-upcase ascii-downcase ascii-control->graphic ascii-graphic->control
    ascii-mirror-bracket
    ascii-ci=? ascii-ci<? ascii-ci>? ascii-ci<=? ascii-ci>=?
    ascii-string-ci=? ascii-string-ci<? ascii-string-ci>?
    ascii-string-ci<=? ascii-string-ci>=?)
  (import (srfi 175 native)))
"#;

/// The Scheme wrapper library for `(scheme bytevector)`, the R6RS bytevectors
/// library under its R7RS-large name. Every representation-dependent procedure
/// is native and re-exported directly. The overlap with `(scheme base)` is
/// re-exported from there, so a joint import needs no renames: the base
/// versions are identical or supersets, and `make-bytevector` accepts the R6RS
/// signed fill. Only the `endianness` macro is defined here, because it must
/// reject an unknown endianness symbol at expansion time. Two deliberate
/// deviations from the R6RS chapter: the R6RS-argument-order `bytevector-copy!`
/// is not provided (R7RS-large keeps the R7RS-small ordering, which the base
/// re-export supplies), and `bytevector-append` is not exported (it is not part
/// of the R6RS chapter).
const SCHEME_BYTEVECTOR_SOURCE: &str = r#"
(define-library (scheme bytevector)
  (export
    ;; Syntax defined in this wrapper.
    endianness
    ;; Re-exported from (scheme base).
    bytevector? make-bytevector bytevector-length
    bytevector-u8-ref bytevector-u8-set! bytevector-copy bytevector-copy!
    string->utf8 utf8->string
    ;; Native general operations from (scheme bytevector native).
    native-endianness bytevector=? bytevector-fill!
    bytevector-s8-ref bytevector-s8-set!
    bytevector->u8-list u8-list->bytevector
    ;; Native arbitrary-size integer operations.
    bytevector-uint-ref bytevector-sint-ref
    bytevector-uint-set! bytevector-sint-set!
    bytevector->uint-list bytevector->sint-list
    uint-list->bytevector sint-list->bytevector
    ;; Native fixed-size integer operations.
    bytevector-u16-ref bytevector-s16-ref
    bytevector-u16-native-ref bytevector-s16-native-ref
    bytevector-u16-set! bytevector-s16-set!
    bytevector-u16-native-set! bytevector-s16-native-set!
    bytevector-u32-ref bytevector-s32-ref
    bytevector-u32-native-ref bytevector-s32-native-ref
    bytevector-u32-set! bytevector-s32-set!
    bytevector-u32-native-set! bytevector-s32-native-set!
    bytevector-u64-ref bytevector-s64-ref
    bytevector-u64-native-ref bytevector-s64-native-ref
    bytevector-u64-set! bytevector-s64-set!
    bytevector-u64-native-set! bytevector-s64-native-set!
    ;; Native IEEE-754 operations.
    bytevector-ieee-single-ref bytevector-ieee-single-native-ref
    bytevector-ieee-single-set! bytevector-ieee-single-native-set!
    bytevector-ieee-double-ref bytevector-ieee-double-native-ref
    bytevector-ieee-double-set! bytevector-ieee-double-native-set!
    ;; Native string transcoders.
    string->utf16 string->utf32 utf16->string utf32->string)
  (import (scheme base) (scheme bytevector native))
  (begin
    ;; An unknown operand fails syntax-rules matching, so misuse is rejected
    ;; at expansion time as R6RS requires.
    (define-syntax endianness
      (syntax-rules (little big)
        ((_ little) 'little)
        ((_ big) 'big)))))
"#;

/// Builds the private native library name `(srfi 48 native)`.
fn srfi_48_native_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(48),
        LibraryNameComponent::identifier("native"),
    ])
}

/// Builds the public library name `(srfi 48)`.
fn srfi_48_public_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(48),
    ])
}

/// Builds the compatibility library name `(srfi 28)`.
fn srfi_28_compat_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(28),
    ])
}

/// Builds the private native library name `(srfi 69 native)`.
fn srfi_69_native_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(69),
        LibraryNameComponent::identifier("native"),
    ])
}

/// Builds the public library name `(srfi 69)`.
fn srfi_69_public_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(69),
    ])
}

/// Builds the private native library name `(srfi 151 native)`.
fn srfi_151_native_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(151),
        LibraryNameComponent::identifier("native"),
    ])
}

/// Builds the public library name `(srfi 151)`.
fn srfi_151_public_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(151),
    ])
}

/// Builds the private native library name `(srfi 152 native)`.
fn srfi_152_native_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(152),
        LibraryNameComponent::identifier("native"),
    ])
}

/// Builds the public library name `(srfi 152)`.
fn srfi_152_public_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(152),
    ])
}

/// Builds the private native library name `(srfi 175 native)`.
fn srfi_175_native_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(175),
        LibraryNameComponent::identifier("native"),
    ])
}

/// Builds the public library name `(srfi 175)`.
fn srfi_175_public_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("srfi"),
        LibraryNameComponent::number(175),
    ])
}

/// Builds the private native library name `(scheme bytevector native)`.
fn scheme_bytevector_native_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("scheme"),
        LibraryNameComponent::identifier("bytevector"),
        LibraryNameComponent::identifier("native"),
    ])
}

/// Builds the public library name `(scheme bytevector)`.
fn scheme_bytevector_public_name() -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("scheme"),
        LibraryNameComponent::identifier("bytevector"),
    ])
}

/// Builds the public library name for an extension.
fn extension_public_name(extension: Extension) -> Result<LibraryName, Error> {
    match extension {
        Extension::Srfi1 => srfi_1_public_name(),
        Extension::Srfi2 => srfi_2_public_name(),
        Extension::Srfi8 => srfi_8_public_name(),
        Extension::Srfi26 => srfi_26_public_name(),
        Extension::Srfi27 => srfi_27_public_name(),
        Extension::Srfi48 => srfi_48_public_name(),
        Extension::Srfi69 => srfi_69_public_name(),
        Extension::Srfi132 => srfi_132_public_name(),
        Extension::Srfi151 => srfi_151_public_name(),
        Extension::Srfi152 => srfi_152_public_name(),
        Extension::Srfi175 => srfi_175_public_name(),
        Extension::Bytevector => scheme_bytevector_public_name(),
    }
}

/// Builds the `(r7rs ...)` alias library name for an extension.
fn r7rs_alias_name(extension: Extension) -> Result<LibraryName, Error> {
    LibraryName::new([
        LibraryNameComponent::identifier("r7rs"),
        LibraryNameComponent::identifier(extension.alias_identifier()),
    ])
}

impl Engine {
    /// Installs an optional extension library on this engine.
    ///
    /// Installing enables the extension's [`Extension::feature_identifier`], so
    /// `cond-expand` and `features` report it, and registers its libraries for
    /// import, including the discoverable `(r7rs ...)` alias named by
    /// [`Extension::alias_identifier`]. Installing an already-installed
    /// extension is a no-op that returns `Ok(())`.
    ///
    /// # Errors
    ///
    /// Returns an error if a library the extension would provide, including its
    /// `(r7rs ...)` alias, is already registered on this engine, or if
    /// registration hits a resource limit. On a mid-install resource error some
    /// of the extension's private native bindings may remain registered, but
    /// the extension is not marked installed and its public library stays
    /// unavailable.
    pub fn install_extension(&mut self, extension: Extension) -> Result<(), Error> {
        if self.installed_extensions.contains(&extension) {
            return Ok(());
        }
        let alias_name = r7rs_alias_name(extension)?;
        if self.libraries.contains(&alias_name) {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!(
                    "cannot install {}: library {alias_name} is already registered",
                    extension.spec_name()
                ),
            ));
        }
        match extension {
            Extension::Srfi1 => self.install_srfi_1()?,
            Extension::Srfi2 => self.install_srfi_2()?,
            Extension::Srfi8 => self.install_srfi_8()?,
            Extension::Srfi26 => self.install_srfi_26()?,
            Extension::Srfi27 => self.install_srfi_27()?,
            Extension::Srfi48 => self.install_srfi_48()?,
            Extension::Srfi69 => self.install_srfi_69()?,
            Extension::Srfi132 => self.install_srfi_132()?,
            Extension::Srfi151 => self.install_srfi_151()?,
            Extension::Srfi152 => self.install_srfi_152()?,
            Extension::Srfi175 => self.install_srfi_175()?,
            Extension::Bytevector => self.install_bytevector()?,
        }
        self.register_r7rs_alias(extension, alias_name)?;
        self.installed_extensions.push(extension);
        Ok(())
    }

    /// Registers the `(r7rs ...)` alias for a just-installed extension. The
    /// alias imports the public SRFI library and re-exports its complete export
    /// list. The list is read back from the registered declaration, so the
    /// alias cannot drift from the wrapper it mirrors.
    fn register_r7rs_alias(
        &mut self,
        extension: Extension,
        alias_name: LibraryName,
    ) -> Result<(), Error> {
        let target = extension_public_name(extension)?;
        let declaration = self.libraries.declaration(&target)?;
        let exports = declaration
            .exports
            .iter()
            .map(|export| export.external.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let display = alias_name.display_name();
        let source = format!("(define-library {display} (export {exports}) (import {target}))");
        self.register_library_source(alias_name, display, source)
    }

    /// Registers the `(srfi 2)` library and enables the `srfi-2` feature.
    /// `and-let*` is pure syntax, so there is no native library to register and
    /// SRFI 2 carries no per-engine mutable state.
    fn install_srfi_2(&mut self) -> Result<(), Error> {
        let public_name = srfi_2_public_name()?;
        if self.libraries.contains(&public_name) {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!("cannot install SRFI 2: library {public_name} is already registered"),
            ));
        }

        self.register_library_source(public_name, "(srfi 2)", SRFI_2_SOURCE)?;

        self.config
            .add_feature(Extension::Srfi2.feature_identifier());
        self.refresh_feature_identifiers();
        Ok(())
    }

    /// Registers the `(srfi 132)` library and enables the `srfi-132` feature.
    /// Every sort procedure calls a user comparator, so the library is pure
    /// Scheme with no native companion and SRFI 132 carries no per-engine
    /// mutable state.
    fn install_srfi_132(&mut self) -> Result<(), Error> {
        let public_name = srfi_132_public_name()?;
        if self.libraries.contains(&public_name) {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!("cannot install SRFI 132: library {public_name} is already registered"),
            ));
        }

        self.register_library_source(public_name, "(srfi 132)", SRFI_132_SOURCE)?;

        self.config
            .add_feature(Extension::Srfi132.feature_identifier());
        self.refresh_feature_identifiers();
        Ok(())
    }

    /// Registers the `(srfi 8)` library and enables the `srfi-8` feature.
    /// `receive` is pure syntax, so there is no native library to register and
    /// SRFI 8 carries no per-engine mutable state.
    fn install_srfi_8(&mut self) -> Result<(), Error> {
        let public_name = srfi_8_public_name()?;
        if self.libraries.contains(&public_name) {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!("cannot install SRFI 8: library {public_name} is already registered"),
            ));
        }

        self.register_library_source(public_name, "(srfi 8)", SRFI_8_SOURCE)?;

        self.config
            .add_feature(Extension::Srfi8.feature_identifier());
        self.refresh_feature_identifiers();
        Ok(())
    }

    /// Registers the `(srfi 26)` library and enables the `srfi-26` feature.
    /// `cut` and `cute` are pure syntax, so there is no native library to
    /// register and SRFI 26 carries no per-engine mutable state.
    fn install_srfi_26(&mut self) -> Result<(), Error> {
        let public_name = srfi_26_public_name()?;
        if self.libraries.contains(&public_name) {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!("cannot install SRFI 26: library {public_name} is already registered"),
            ));
        }

        self.register_library_source(public_name, "(srfi 26)", SRFI_26_SOURCE)?;

        self.config
            .add_feature(Extension::Srfi26.feature_identifier());
        self.refresh_feature_identifiers();
        Ok(())
    }

    /// Registers the `(srfi 1)` and `(srfi 1 native)` libraries and enables the
    /// `srfi-1` feature. SRFI 1 carries no per-engine mutable state, so this only
    /// registers the structural natives and the Scheme wrapper.
    fn install_srfi_1(&mut self) -> Result<(), Error> {
        let native_name = srfi_1_native_name()?;
        let public_name = srfi_1_public_name()?;
        if self.libraries.contains(&public_name) || self.libraries.contains(&native_name) {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!(
                    "cannot install SRFI 1: library {public_name} or {native_name} is already registered"
                ),
            ));
        }

        let variadic = 1..=usize::MAX;
        self.register_library_fn(&native_name, "xcons", 2..=2, srfi1::xcons)?;
        self.register_library_fn(&native_name, "cons*", variadic, srfi1::cons_star)?;
        self.register_library_fn(&native_name, "take", 2..=2, srfi1::take)?;
        self.register_library_fn(&native_name, "take-right", 2..=2, srfi1::take_right)?;
        self.register_library_fn(&native_name, "drop-right", 2..=2, srfi1::drop_right)?;
        self.register_library_fn(&native_name, "last", 1..=1, srfi1::last)?;
        self.register_library_fn(&native_name, "last-pair", 1..=1, srfi1::last_pair)?;
        self.register_library_fn(&native_name, "length+", 1..=1, srfi1::length_plus)?;
        self.register_library_fn(&native_name, "append-reverse", 2..=2, srfi1::append_reverse)?;
        self.register_library_fn(
            &native_name,
            "circular-list?",
            1..=1,
            srfi1::circular_list_p,
        )?;
        self.register_library_fn(&native_name, "dotted-list?", 1..=1, srfi1::dotted_list_p)?;

        self.register_library_source(public_name, "(srfi 1)", SRFI_1_SOURCE)?;

        self.config
            .add_feature(Extension::Srfi1.feature_identifier());
        self.refresh_feature_identifiers();
        Ok(())
    }

    /// Registers the `(srfi 27)` and `(srfi 27 native)` libraries and enables
    /// the `srfi-27` feature.
    fn install_srfi_27(&mut self) -> Result<(), Error> {
        let native_name = srfi_27_native_name()?;
        let public_name = srfi_27_public_name()?;
        if self.libraries.contains(&public_name) || self.libraries.contains(&native_name) {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!(
                    "cannot install SRFI 27: library {public_name} or {native_name} is already registered"
                ),
            ));
        }

        // The default source is time-seeded once, here at install. It is kept
        // alive by a root captured in the draw closures, so it is distinct from
        // any source the guest later makes and reassigning default-random-source
        // in Scheme does not disturb random-integer or random-real.
        let seed = srfi27::system_time_seed();
        let source = self
            .heap
            .alloc(Object::RandomSource(SquaresRng::from_seed(seed)))?;
        let default_root = self.heap.root(source);

        let integer_root = default_root.clone();
        self.register_library_fn(&native_name, "random-integer", 1..=1, move |cx, args| {
            srfi27::draw_integer(cx, integer_root.value(), args[0])
        })?;
        let real_root = default_root.clone();
        self.register_library_fn(&native_name, "random-real", 0..=0, move |cx, _args| {
            srfi27::draw_real(cx, real_root.value())
        })?;
        let accessor_root = default_root.clone();
        self.register_library_fn(
            &native_name,
            "%default-random-source",
            0..=0,
            move |_cx, _args| Ok::<Value, Error>(accessor_root.value()),
        )?;

        self.register_library_fn(
            &native_name,
            "make-random-source",
            0..=1,
            srfi27::make_random_source,
        )?;
        self.register_library_fn(
            &native_name,
            "random-source?",
            1..=1,
            srfi27::random_source_p,
        )?;
        self.register_library_fn(
            &native_name,
            "random-source-state-ref",
            1..=1,
            srfi27::random_source_state_ref,
        )?;
        self.register_library_fn(
            &native_name,
            "random-source-state-set!",
            2..=2,
            srfi27::random_source_state_set,
        )?;
        self.register_library_fn(
            &native_name,
            "random-source-randomize!",
            1..=2,
            srfi27::random_source_randomize,
        )?;
        self.register_library_fn(
            &native_name,
            "random-source-pseudo-randomize!",
            3..=3,
            srfi27::random_source_pseudo_randomize,
        )?;
        self.register_library_fn(
            &native_name,
            "%random-integer-on",
            2..=2,
            srfi27::random_integer_on,
        )?;
        self.register_library_fn(
            &native_name,
            "%random-real-on",
            1..=1,
            srfi27::random_real_on,
        )?;

        self.register_library_source(public_name, "(srfi 27)", SRFI_27_SOURCE)?;

        self.config
            .add_feature(Extension::Srfi27.feature_identifier());
        self.refresh_feature_identifiers();
        Ok(())
    }

    /// Registers the `(srfi 48)` and `(srfi 48 native)` libraries plus the
    /// `(srfi 28)` compatibility name, and enables the `srfi-48` and `srfi-28`
    /// features. `%format48` is the single native primitive; the wrapper adds
    /// only the destination dispatch, so SRFI 48 carries no per-engine mutable
    /// state.
    fn install_srfi_48(&mut self) -> Result<(), Error> {
        let native_name = srfi_48_native_name()?;
        let public_name = srfi_48_public_name()?;
        let compat_name = srfi_28_compat_name()?;
        if self.libraries.contains(&public_name)
            || self.libraries.contains(&native_name)
            || self.libraries.contains(&compat_name)
        {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!(
                    "cannot install SRFI 48: library {public_name}, {native_name}, or {compat_name} is already registered"
                ),
            ));
        }

        self.register_library_fn(&native_name, "%format48", 1..=usize::MAX, srfi48::format)?;

        self.register_library_source(public_name, "(srfi 48)", SRFI_48_SOURCE)?;
        self.register_library_source(compat_name, "(srfi 28)", SRFI_28_COMPAT_SOURCE)?;

        self.config
            .add_feature(Extension::Srfi48.feature_identifier());
        // The compatibility name satisfies the SRFI 28 contract, so guest code
        // probing for it through cond-expand succeeds too.
        self.config.add_feature("srfi-28");
        self.refresh_feature_identifiers();
        Ok(())
    }

    /// Registers the `(srfi 69)` and `(srfi 69 native)` libraries and enables
    /// the `srfi-69` feature. Only the four hash functions are native. The table
    /// operations live in the Scheme wrapper because they call user comparison,
    /// hash, and callback procedures, so SRFI 69 carries no per-engine mutable
    /// state.
    fn install_srfi_69(&mut self) -> Result<(), Error> {
        let native_name = srfi_69_native_name()?;
        let public_name = srfi_69_public_name()?;
        if self.libraries.contains(&public_name) || self.libraries.contains(&native_name) {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!(
                    "cannot install SRFI 69: library {public_name} or {native_name} is already registered"
                ),
            ));
        }

        self.register_library_fn(&native_name, "hash", 1..=2, srfi69::hash)?;
        self.register_library_fn(&native_name, "string-hash", 1..=2, srfi69::string_hash)?;
        self.register_library_fn(
            &native_name,
            "string-ci-hash",
            1..=2,
            srfi69::string_ci_hash,
        )?;
        self.register_library_fn(
            &native_name,
            "hash-by-identity",
            1..=2,
            srfi69::hash_by_identity,
        )?;

        self.register_library_source(public_name, "(srfi 69)", SRFI_69_SOURCE)?;

        self.config
            .add_feature(Extension::Srfi69.feature_identifier());
        self.refresh_feature_identifiers();
        Ok(())
    }

    /// Registers the `(srfi 151)` and `(srfi 151 native)` libraries and enables
    /// the `srfi-151` feature. The 30 pure bit operations are native; the
    /// aggregate conversions and the callback or generator procedures live in
    /// the Scheme wrapper, so SRFI 151 carries no per-engine mutable state.
    fn install_srfi_151(&mut self) -> Result<(), Error> {
        let native_name = srfi_151_native_name()?;
        let public_name = srfi_151_public_name()?;
        if self.libraries.contains(&public_name) || self.libraries.contains(&native_name) {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!(
                    "cannot install SRFI 151: library {public_name} or {native_name} is already registered"
                ),
            ));
        }

        use crate::native::srfi151;
        self.register_library_fn(&native_name, "bitwise-not", 1..=1, srfi151::bitwise_not)?;
        self.register_library_fn(
            &native_name,
            "bitwise-and",
            0..=usize::MAX,
            srfi151::bitwise_and,
        )?;
        self.register_library_fn(
            &native_name,
            "bitwise-ior",
            0..=usize::MAX,
            srfi151::bitwise_ior,
        )?;
        self.register_library_fn(
            &native_name,
            "bitwise-xor",
            0..=usize::MAX,
            srfi151::bitwise_xor,
        )?;
        self.register_library_fn(
            &native_name,
            "bitwise-eqv",
            0..=usize::MAX,
            srfi151::bitwise_eqv,
        )?;
        self.register_library_fn(&native_name, "bitwise-nand", 2..=2, srfi151::bitwise_nand)?;
        self.register_library_fn(&native_name, "bitwise-nor", 2..=2, srfi151::bitwise_nor)?;
        self.register_library_fn(&native_name, "bitwise-andc1", 2..=2, srfi151::bitwise_andc1)?;
        self.register_library_fn(&native_name, "bitwise-andc2", 2..=2, srfi151::bitwise_andc2)?;
        self.register_library_fn(&native_name, "bitwise-orc1", 2..=2, srfi151::bitwise_orc1)?;
        self.register_library_fn(&native_name, "bitwise-orc2", 2..=2, srfi151::bitwise_orc2)?;
        self.register_library_fn(
            &native_name,
            "arithmetic-shift",
            2..=2,
            srfi151::arithmetic_shift,
        )?;
        self.register_library_fn(&native_name, "bit-count", 1..=1, srfi151::bit_count)?;
        self.register_library_fn(
            &native_name,
            "integer-length",
            1..=1,
            srfi151::integer_length,
        )?;
        self.register_library_fn(&native_name, "bitwise-if", 3..=3, srfi151::bitwise_if)?;
        self.register_library_fn(&native_name, "bit-set?", 2..=2, srfi151::bit_set_p)?;
        self.register_library_fn(&native_name, "copy-bit", 3..=3, srfi151::copy_bit)?;
        self.register_library_fn(&native_name, "bit-swap", 3..=3, srfi151::bit_swap)?;
        self.register_library_fn(&native_name, "any-bit-set?", 2..=2, srfi151::any_bit_set_p)?;
        self.register_library_fn(
            &native_name,
            "every-bit-set?",
            2..=2,
            srfi151::every_bit_set_p,
        )?;
        self.register_library_fn(&native_name, "first-set-bit", 1..=1, srfi151::first_set_bit)?;
        self.register_library_fn(&native_name, "bit-field", 3..=3, srfi151::bit_field)?;
        self.register_library_fn(
            &native_name,
            "bit-field-any?",
            3..=3,
            srfi151::bit_field_any_p,
        )?;
        self.register_library_fn(
            &native_name,
            "bit-field-every?",
            3..=3,
            srfi151::bit_field_every_p,
        )?;
        self.register_library_fn(
            &native_name,
            "bit-field-clear",
            3..=3,
            srfi151::bit_field_clear,
        )?;
        self.register_library_fn(&native_name, "bit-field-set", 3..=3, srfi151::bit_field_set)?;
        self.register_library_fn(
            &native_name,
            "bit-field-replace",
            4..=4,
            srfi151::bit_field_replace,
        )?;
        self.register_library_fn(
            &native_name,
            "bit-field-replace-same",
            4..=4,
            srfi151::bit_field_replace_same,
        )?;
        self.register_library_fn(
            &native_name,
            "bit-field-rotate",
            4..=4,
            srfi151::bit_field_rotate,
        )?;
        self.register_library_fn(
            &native_name,
            "bit-field-reverse",
            3..=3,
            srfi151::bit_field_reverse,
        )?;

        self.register_library_source(public_name, "(srfi 151)", SRFI_151_SOURCE)?;

        self.config
            .add_feature(Extension::Srfi151.feature_identifier());
        self.refresh_feature_identifiers();
        Ok(())
    }

    /// Registers the `(srfi 152)` and `(srfi 152 native)` libraries and enables
    /// the `srfi-152` feature. The representation-dependent scans and builders
    /// that take no callback are native; every predicate, mapper, and fold
    /// procedure lives in the wrapper, and the R7RS-small string procedures are
    /// re-exported from `(scheme base)` and `(scheme char)`.
    fn install_srfi_152(&mut self) -> Result<(), Error> {
        let native_name = srfi_152_native_name()?;
        let public_name = srfi_152_public_name()?;
        if self.libraries.contains(&public_name) || self.libraries.contains(&native_name) {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!(
                    "cannot install SRFI 152: library {public_name} or {native_name} is already registered"
                ),
            ));
        }

        use crate::native::srfi152;
        self.register_library_fn(&native_name, "string-null?", 1..=1, srfi152::string_null_p)?;
        self.register_library_fn(
            &native_name,
            "reverse-list->string",
            1..=1,
            srfi152::reverse_list_to_string,
        )?;
        self.register_library_fn(
            &native_name,
            "string-prefix-length",
            2..=6,
            srfi152::string_prefix_length,
        )?;
        self.register_library_fn(
            &native_name,
            "string-suffix-length",
            2..=6,
            srfi152::string_suffix_length,
        )?;
        self.register_library_fn(
            &native_name,
            "string-prefix?",
            2..=6,
            srfi152::string_prefix_p,
        )?;
        self.register_library_fn(
            &native_name,
            "string-suffix?",
            2..=6,
            srfi152::string_suffix_p,
        )?;
        self.register_library_fn(
            &native_name,
            "string-contains",
            2..=6,
            srfi152::string_contains,
        )?;
        self.register_library_fn(
            &native_name,
            "string-contains-right",
            2..=6,
            srfi152::string_contains_right,
        )?;
        self.register_library_fn(
            &native_name,
            "string-concatenate",
            1..=1,
            srfi152::string_concatenate,
        )?;
        self.register_library_fn(
            &native_name,
            "string-replicate",
            3..=5,
            srfi152::string_replicate,
        )?;

        self.register_library_source(public_name, "(srfi 152)", SRFI_152_SOURCE)?;

        self.config
            .add_feature(Extension::Srfi152.feature_identifier());
        self.refresh_feature_identifiers();
        Ok(())
    }

    /// Registers the `(srfi 175)` and `(srfi 175 native)` libraries and
    /// enables the `srfi-175` feature. Every procedure is native because the
    /// library has no callbacks into Scheme.
    fn install_srfi_175(&mut self) -> Result<(), Error> {
        let native_name = srfi_175_native_name()?;
        let public_name = srfi_175_public_name()?;
        if self.libraries.contains(&public_name) || self.libraries.contains(&native_name) {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!(
                    "cannot install SRFI 175: library {public_name} or {native_name} is already registered"
                ),
            ));
        }

        use crate::native::srfi175;
        let n = &native_name;
        self.register_library_fn(n, "ascii-codepoint?", 1..=1, srfi175::ascii_codepoint_p)?;
        self.register_library_fn(n, "ascii-bytevector?", 1..=1, srfi175::ascii_bytevector_p)?;
        self.register_library_fn(n, "ascii-char?", 1..=1, srfi175::ascii_char_p)?;
        self.register_library_fn(n, "ascii-string?", 1..=1, srfi175::ascii_string_p)?;
        self.register_library_fn(n, "ascii-control?", 1..=1, srfi175::ascii_control_p)?;
        self.register_library_fn(n, "ascii-non-control?", 1..=1, srfi175::ascii_non_control_p)?;
        self.register_library_fn(n, "ascii-whitespace?", 1..=1, srfi175::ascii_whitespace_p)?;
        self.register_library_fn(
            n,
            "ascii-space-or-tab?",
            1..=1,
            srfi175::ascii_space_or_tab_p,
        )?;
        self.register_library_fn(
            n,
            "ascii-other-graphic?",
            1..=1,
            srfi175::ascii_other_graphic_p,
        )?;
        self.register_library_fn(n, "ascii-upper-case?", 1..=1, srfi175::ascii_upper_case_p)?;
        self.register_library_fn(n, "ascii-lower-case?", 1..=1, srfi175::ascii_lower_case_p)?;
        self.register_library_fn(n, "ascii-alphabetic?", 1..=1, srfi175::ascii_alphabetic_p)?;
        self.register_library_fn(
            n,
            "ascii-alphanumeric?",
            1..=1,
            srfi175::ascii_alphanumeric_p,
        )?;
        self.register_library_fn(n, "ascii-numeric?", 1..=1, srfi175::ascii_numeric_p)?;
        self.register_library_fn(n, "ascii-digit-value", 2..=2, srfi175::ascii_digit_value)?;
        self.register_library_fn(
            n,
            "ascii-upper-case-value",
            3..=3,
            srfi175::ascii_upper_case_value,
        )?;
        self.register_library_fn(
            n,
            "ascii-lower-case-value",
            3..=3,
            srfi175::ascii_lower_case_value,
        )?;
        self.register_library_fn(n, "ascii-nth-digit", 1..=1, srfi175::ascii_nth_digit)?;
        self.register_library_fn(
            n,
            "ascii-nth-upper-case",
            1..=1,
            srfi175::ascii_nth_upper_case,
        )?;
        self.register_library_fn(
            n,
            "ascii-nth-lower-case",
            1..=1,
            srfi175::ascii_nth_lower_case,
        )?;
        self.register_library_fn(n, "ascii-upcase", 1..=1, srfi175::ascii_upcase)?;
        self.register_library_fn(n, "ascii-downcase", 1..=1, srfi175::ascii_downcase)?;
        self.register_library_fn(
            n,
            "ascii-control->graphic",
            1..=1,
            srfi175::ascii_control_to_graphic,
        )?;
        self.register_library_fn(
            n,
            "ascii-graphic->control",
            1..=1,
            srfi175::ascii_graphic_to_control,
        )?;
        self.register_library_fn(
            n,
            "ascii-mirror-bracket",
            1..=1,
            srfi175::ascii_mirror_bracket,
        )?;
        self.register_library_fn(n, "ascii-ci=?", 2..=2, srfi175::ascii_ci_equal)?;
        self.register_library_fn(n, "ascii-ci<?", 2..=2, srfi175::ascii_ci_less)?;
        self.register_library_fn(n, "ascii-ci>?", 2..=2, srfi175::ascii_ci_greater)?;
        self.register_library_fn(n, "ascii-ci<=?", 2..=2, srfi175::ascii_ci_less_equal)?;
        self.register_library_fn(n, "ascii-ci>=?", 2..=2, srfi175::ascii_ci_greater_equal)?;
        self.register_library_fn(
            n,
            "ascii-string-ci=?",
            2..=2,
            srfi175::ascii_string_ci_equal,
        )?;
        self.register_library_fn(n, "ascii-string-ci<?", 2..=2, srfi175::ascii_string_ci_less)?;
        self.register_library_fn(
            n,
            "ascii-string-ci>?",
            2..=2,
            srfi175::ascii_string_ci_greater,
        )?;
        self.register_library_fn(
            n,
            "ascii-string-ci<=?",
            2..=2,
            srfi175::ascii_string_ci_less_equal,
        )?;
        self.register_library_fn(
            n,
            "ascii-string-ci>=?",
            2..=2,
            srfi175::ascii_string_ci_greater_equal,
        )?;

        self.register_library_source(public_name, "(srfi 175)", SRFI_175_SOURCE)?;
        self.config
            .add_feature(Extension::Srfi175.feature_identifier());
        self.refresh_feature_identifiers();
        Ok(())
    }

    /// Registers the `(scheme bytevector)` and `(scheme bytevector native)`
    /// libraries and enables the `scheme-bytevector` feature. Every
    /// representation-dependent procedure is native, only the `endianness`
    /// macro lives in the wrapper, and the extension carries no per-engine
    /// mutable state.
    fn install_bytevector(&mut self) -> Result<(), Error> {
        let native_name = scheme_bytevector_native_name()?;
        let public_name = scheme_bytevector_public_name()?;
        if self.libraries.contains(&public_name) || self.libraries.contains(&native_name) {
            return Err(Error::plain(
                ErrorKind::LibraryError,
                format!(
                    "cannot install R6RS Bytevectors: library {public_name} or {native_name} is already registered"
                ),
            ));
        }

        use crate::native::bytevector as bv;
        let n = &native_name;
        self.register_library_fn(n, "native-endianness", 0..=0, bv::native_endianness)?;
        self.register_library_fn(n, "bytevector=?", 2..=2, bv::bytevector_equal)?;
        self.register_library_fn(n, "bytevector-fill!", 2..=2, bv::bytevector_fill)?;
        self.register_library_fn(n, "bytevector-s8-ref", 2..=2, bv::bytevector_s8_ref)?;
        self.register_library_fn(n, "bytevector-s8-set!", 3..=3, bv::bytevector_s8_set)?;
        self.register_library_fn(n, "bytevector->u8-list", 1..=1, bv::bytevector_to_u8_list)?;
        self.register_library_fn(n, "u8-list->bytevector", 1..=1, bv::u8_list_to_bytevector)?;
        self.register_library_fn(n, "bytevector-uint-ref", 4..=4, bv::bytevector_uint_ref)?;
        self.register_library_fn(n, "bytevector-sint-ref", 4..=4, bv::bytevector_sint_ref)?;
        self.register_library_fn(n, "bytevector-uint-set!", 5..=5, bv::bytevector_uint_set)?;
        self.register_library_fn(n, "bytevector-sint-set!", 5..=5, bv::bytevector_sint_set)?;
        self.register_library_fn(
            n,
            "bytevector->uint-list",
            3..=3,
            bv::bytevector_to_uint_list,
        )?;
        self.register_library_fn(
            n,
            "bytevector->sint-list",
            3..=3,
            bv::bytevector_to_sint_list,
        )?;
        self.register_library_fn(
            n,
            "uint-list->bytevector",
            3..=3,
            bv::uint_list_to_bytevector,
        )?;
        self.register_library_fn(
            n,
            "sint-list->bytevector",
            3..=3,
            bv::sint_list_to_bytevector,
        )?;
        self.register_library_fn(n, "bytevector-u16-ref", 3..=3, bv::bytevector_u16_ref)?;
        self.register_library_fn(n, "bytevector-s16-ref", 3..=3, bv::bytevector_s16_ref)?;
        self.register_library_fn(n, "bytevector-u32-ref", 3..=3, bv::bytevector_u32_ref)?;
        self.register_library_fn(n, "bytevector-s32-ref", 3..=3, bv::bytevector_s32_ref)?;
        self.register_library_fn(n, "bytevector-u64-ref", 3..=3, bv::bytevector_u64_ref)?;
        self.register_library_fn(n, "bytevector-s64-ref", 3..=3, bv::bytevector_s64_ref)?;
        self.register_library_fn(n, "bytevector-u16-set!", 4..=4, bv::bytevector_u16_set)?;
        self.register_library_fn(n, "bytevector-s16-set!", 4..=4, bv::bytevector_s16_set)?;
        self.register_library_fn(n, "bytevector-u32-set!", 4..=4, bv::bytevector_u32_set)?;
        self.register_library_fn(n, "bytevector-s32-set!", 4..=4, bv::bytevector_s32_set)?;
        self.register_library_fn(n, "bytevector-u64-set!", 4..=4, bv::bytevector_u64_set)?;
        self.register_library_fn(n, "bytevector-s64-set!", 4..=4, bv::bytevector_s64_set)?;
        self.register_library_fn(
            n,
            "bytevector-u16-native-ref",
            2..=2,
            bv::bytevector_u16_native_ref,
        )?;
        self.register_library_fn(
            n,
            "bytevector-s16-native-ref",
            2..=2,
            bv::bytevector_s16_native_ref,
        )?;
        self.register_library_fn(
            n,
            "bytevector-u32-native-ref",
            2..=2,
            bv::bytevector_u32_native_ref,
        )?;
        self.register_library_fn(
            n,
            "bytevector-s32-native-ref",
            2..=2,
            bv::bytevector_s32_native_ref,
        )?;
        self.register_library_fn(
            n,
            "bytevector-u64-native-ref",
            2..=2,
            bv::bytevector_u64_native_ref,
        )?;
        self.register_library_fn(
            n,
            "bytevector-s64-native-ref",
            2..=2,
            bv::bytevector_s64_native_ref,
        )?;
        self.register_library_fn(
            n,
            "bytevector-u16-native-set!",
            3..=3,
            bv::bytevector_u16_native_set,
        )?;
        self.register_library_fn(
            n,
            "bytevector-s16-native-set!",
            3..=3,
            bv::bytevector_s16_native_set,
        )?;
        self.register_library_fn(
            n,
            "bytevector-u32-native-set!",
            3..=3,
            bv::bytevector_u32_native_set,
        )?;
        self.register_library_fn(
            n,
            "bytevector-s32-native-set!",
            3..=3,
            bv::bytevector_s32_native_set,
        )?;
        self.register_library_fn(
            n,
            "bytevector-u64-native-set!",
            3..=3,
            bv::bytevector_u64_native_set,
        )?;
        self.register_library_fn(
            n,
            "bytevector-s64-native-set!",
            3..=3,
            bv::bytevector_s64_native_set,
        )?;
        self.register_library_fn(
            n,
            "bytevector-ieee-single-ref",
            3..=3,
            bv::bytevector_ieee_single_ref,
        )?;
        self.register_library_fn(
            n,
            "bytevector-ieee-double-ref",
            3..=3,
            bv::bytevector_ieee_double_ref,
        )?;
        self.register_library_fn(
            n,
            "bytevector-ieee-single-set!",
            4..=4,
            bv::bytevector_ieee_single_set,
        )?;
        self.register_library_fn(
            n,
            "bytevector-ieee-double-set!",
            4..=4,
            bv::bytevector_ieee_double_set,
        )?;
        self.register_library_fn(
            n,
            "bytevector-ieee-single-native-ref",
            2..=2,
            bv::bytevector_ieee_single_native_ref,
        )?;
        self.register_library_fn(
            n,
            "bytevector-ieee-double-native-ref",
            2..=2,
            bv::bytevector_ieee_double_native_ref,
        )?;
        self.register_library_fn(
            n,
            "bytevector-ieee-single-native-set!",
            3..=3,
            bv::bytevector_ieee_single_native_set,
        )?;
        self.register_library_fn(
            n,
            "bytevector-ieee-double-native-set!",
            3..=3,
            bv::bytevector_ieee_double_native_set,
        )?;
        self.register_library_fn(n, "string->utf16", 1..=2, bv::string_to_utf16)?;
        self.register_library_fn(n, "string->utf32", 1..=2, bv::string_to_utf32)?;
        self.register_library_fn(n, "utf16->string", 2..=3, bv::utf16_to_string)?;
        self.register_library_fn(n, "utf32->string", 2..=3, bv::utf32_to_string)?;

        self.register_library_source(public_name, "(scheme bytevector)", SCHEME_BYTEVECTOR_SOURCE)?;

        self.config
            .add_feature(Extension::Bytevector.feature_identifier());
        self.refresh_feature_identifiers();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{Engine, EngineConfig, ErrorKind, Extension, LibraryName, LibraryNameComponent};

    #[test]
    fn double_install_is_an_idempotent_no_op() {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi27).unwrap();
        // A second install returns Ok without re-registering the library.
        engine.install_extension(Extension::Srfi27).unwrap();
        let module = engine
            .compile("test.scm", "(import (srfi 27)) (random-integer 1)")
            .unwrap();
        assert_eq!(
            engine.eval(&module).unwrap().into_one().unwrap().value(),
            crate::Value::integer(0)
        );
    }

    #[test]
    fn install_conflicts_with_a_user_registered_library() {
        let public_name = LibraryName::new([
            LibraryNameComponent::identifier("srfi"),
            LibraryNameComponent::number(27),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine
            .register_library_source(
                public_name,
                "(srfi 27)",
                "(define-library (srfi 27) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
            )
            .unwrap();
        assert_eq!(
            engine
                .install_extension(Extension::Srfi27)
                .unwrap_err()
                .kind(),
            ErrorKind::LibraryError
        );
    }

    #[test]
    fn user_registration_conflicts_after_install() {
        let public_name = LibraryName::new([
            LibraryNameComponent::identifier("srfi"),
            LibraryNameComponent::number(27),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi27).unwrap();
        let result = engine.register_library_source(
            public_name,
            "(srfi 27)",
            "(define-library (srfi 27) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
        );
        assert!(result.is_err());
    }

    #[test]
    fn srfi_1_double_install_is_an_idempotent_no_op() {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi1).unwrap();
        // A second install returns Ok without re-registering the library.
        engine.install_extension(Extension::Srfi1).unwrap();
        let module = engine
            .compile("test.scm", "(import (srfi 1)) (fold + 0 (iota 5))")
            .unwrap();
        assert_eq!(
            engine.eval(&module).unwrap().into_one().unwrap().value(),
            crate::Value::integer(10)
        );
    }

    #[test]
    fn srfi_1_install_conflicts_with_a_user_registered_library() {
        let public_name = LibraryName::new([
            LibraryNameComponent::identifier("srfi"),
            LibraryNameComponent::number(1),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine
            .register_library_source(
                public_name,
                "(srfi 1)",
                "(define-library (srfi 1) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
            )
            .unwrap();
        assert_eq!(
            engine
                .install_extension(Extension::Srfi1)
                .unwrap_err()
                .kind(),
            ErrorKind::LibraryError
        );
    }

    #[test]
    fn srfi_1_user_registration_conflicts_after_install() {
        let public_name = LibraryName::new([
            LibraryNameComponent::identifier("srfi"),
            LibraryNameComponent::number(1),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi1).unwrap();
        let result = engine.register_library_source(
            public_name,
            "(srfi 1)",
            "(define-library (srfi 1) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
        );
        assert!(result.is_err());
    }

    #[test]
    fn srfi_2_double_install_is_an_idempotent_no_op() {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi2).unwrap();
        // A second install returns Ok without re-registering the library.
        engine.install_extension(Extension::Srfi2).unwrap();
        let module = engine
            .compile(
                "test.scm",
                "(import (srfi 2)) (and-let* ((x 1) (y 2)) (+ x y))",
            )
            .unwrap();
        assert_eq!(
            engine.eval(&module).unwrap().into_one().unwrap().value(),
            crate::Value::integer(3)
        );
    }

    #[test]
    fn srfi_2_install_conflicts_with_a_user_registered_library() {
        let public_name = LibraryName::new([
            LibraryNameComponent::identifier("srfi"),
            LibraryNameComponent::number(2),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine
            .register_library_source(
                public_name,
                "(srfi 2)",
                "(define-library (srfi 2) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
            )
            .unwrap();
        assert_eq!(
            engine
                .install_extension(Extension::Srfi2)
                .unwrap_err()
                .kind(),
            ErrorKind::LibraryError
        );
    }

    #[test]
    fn srfi_2_user_registration_conflicts_after_install() {
        let public_name = LibraryName::new([
            LibraryNameComponent::identifier("srfi"),
            LibraryNameComponent::number(2),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi2).unwrap();
        let result = engine.register_library_source(
            public_name,
            "(srfi 2)",
            "(define-library (srfi 2) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
        );
        assert!(result.is_err());
    }

    #[test]
    fn srfi_48_double_install_is_an_idempotent_no_op() {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi48).unwrap();
        // A second install returns Ok without re-registering the libraries.
        engine.install_extension(Extension::Srfi48).unwrap();
        let module = engine
            .compile("test.scm", r#"(import (srfi 48)) (format "~a-~x" 1 255)"#)
            .unwrap();
        let root = engine.eval(&module).unwrap().into_one().unwrap();
        assert_eq!(engine.write_root(&root).unwrap(), "\"1-ff\"");
    }

    #[test]
    fn srfi_48_install_conflicts_with_a_user_registered_library() {
        let public_name = LibraryName::new([
            LibraryNameComponent::identifier("srfi"),
            LibraryNameComponent::number(48),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine
            .register_library_source(
                public_name,
                "(srfi 48)",
                "(define-library (srfi 48) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
            )
            .unwrap();
        assert_eq!(
            engine
                .install_extension(Extension::Srfi48)
                .unwrap_err()
                .kind(),
            ErrorKind::LibraryError
        );
    }

    #[test]
    fn srfi_48_install_conflicts_with_a_user_registered_compat_library() {
        let compat_name = LibraryName::new([
            LibraryNameComponent::identifier("srfi"),
            LibraryNameComponent::number(28),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine
            .register_library_source(
                compat_name,
                "(srfi 28)",
                "(define-library (srfi 28) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
            )
            .unwrap();
        assert_eq!(
            engine
                .install_extension(Extension::Srfi48)
                .unwrap_err()
                .kind(),
            ErrorKind::LibraryError
        );
    }

    #[test]
    fn srfi_48_user_registration_conflicts_after_install() {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi48).unwrap();
        for number in [48u64, 28] {
            let name = LibraryName::new([
                LibraryNameComponent::identifier("srfi"),
                LibraryNameComponent::number(number),
            ])
            .unwrap();
            let result = engine.register_library_source(
                name,
                format!("(srfi {number})"),
                format!(
                    "(define-library (srfi {number}) (export nothing) (import (scheme base)) (begin (define nothing 0)))"
                ),
            );
            assert!(result.is_err());
        }
    }

    #[test]
    fn srfi_69_double_install_is_an_idempotent_no_op() {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi69).unwrap();
        // A second install returns Ok without re-registering the library.
        engine.install_extension(Extension::Srfi69).unwrap();
        let module = engine
            .compile(
                "test.scm",
                "(import (srfi 69)) (let ((h (make-hash-table))) (hash-table-set! h 'a 1) (hash-table-ref h 'a))",
            )
            .unwrap();
        assert_eq!(
            engine.eval(&module).unwrap().into_one().unwrap().value(),
            crate::Value::integer(1)
        );
    }

    #[test]
    fn srfi_69_install_conflicts_with_a_user_registered_library() {
        let public_name = LibraryName::new([
            LibraryNameComponent::identifier("srfi"),
            LibraryNameComponent::number(69),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine
            .register_library_source(
                public_name,
                "(srfi 69)",
                "(define-library (srfi 69) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
            )
            .unwrap();
        assert_eq!(
            engine
                .install_extension(Extension::Srfi69)
                .unwrap_err()
                .kind(),
            ErrorKind::LibraryError
        );
    }

    #[test]
    fn srfi_69_user_registration_conflicts_after_install() {
        let public_name = LibraryName::new([
            LibraryNameComponent::identifier("srfi"),
            LibraryNameComponent::number(69),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi69).unwrap();
        let result = engine.register_library_source(
            public_name,
            "(srfi 69)",
            "(define-library (srfi 69) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
        );
        assert!(result.is_err());
    }

    #[test]
    fn srfi_151_double_install_is_an_idempotent_no_op() {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi151).unwrap();
        // A second install returns Ok without re-registering the library.
        engine.install_extension(Extension::Srfi151).unwrap();
        let module = engine
            .compile("test.scm", "(import (srfi 151)) (bitwise-and 11 26)")
            .unwrap();
        assert_eq!(
            engine.eval(&module).unwrap().into_one().unwrap().value(),
            crate::Value::integer(10)
        );
    }

    #[test]
    fn srfi_151_install_conflicts_with_a_user_registered_library() {
        let public_name = LibraryName::new([
            LibraryNameComponent::identifier("srfi"),
            LibraryNameComponent::number(151),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine
            .register_library_source(
                public_name,
                "(srfi 151)",
                "(define-library (srfi 151) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
            )
            .unwrap();
        assert_eq!(
            engine
                .install_extension(Extension::Srfi151)
                .unwrap_err()
                .kind(),
            ErrorKind::LibraryError
        );
    }

    #[test]
    fn install_conflicts_with_a_user_registered_alias_library() {
        let alias_name = LibraryName::new([
            LibraryNameComponent::identifier("r7rs"),
            LibraryNameComponent::identifier("random-bits"),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine
            .register_library_source(
                alias_name,
                "(r7rs random-bits)",
                "(define-library (r7rs random-bits) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
            )
            .unwrap();
        assert_eq!(
            engine
                .install_extension(Extension::Srfi27)
                .unwrap_err()
                .kind(),
            ErrorKind::LibraryError
        );
        // The alias conflict is detected before any SRFI library is
        // registered, so the failed install leaves (srfi 27) unavailable.
        assert!(
            engine
                .compile("test.scm", "(import (srfi 27)) (random-integer 1)")
                .is_err()
        );
    }

    #[test]
    fn user_alias_registration_conflicts_after_install() {
        let alias_name = LibraryName::new([
            LibraryNameComponent::identifier("r7rs"),
            LibraryNameComponent::identifier("random"),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi27).unwrap();
        let result = engine.register_library_source(
            alias_name,
            "(r7rs random-bits)",
            "(define-library (r7rs random-bits) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
        );
        assert!(result.is_err());
    }

    #[test]
    fn srfi_151_user_registration_conflicts_after_install() {
        let public_name = LibraryName::new([
            LibraryNameComponent::identifier("srfi"),
            LibraryNameComponent::number(151),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi151).unwrap();
        let result = engine.register_library_source(
            public_name,
            "(srfi 151)",
            "(define-library (srfi 151) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
        );
        assert!(result.is_err());
    }

    #[test]
    fn srfi_175_double_install_is_an_idempotent_no_op() {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Srfi175).unwrap();
        engine.install_extension(Extension::Srfi175).unwrap();
        let module = engine
            .compile("test.scm", "(import (srfi 175)) (ascii-ci=? #\\A 97)")
            .unwrap();
        assert_eq!(
            engine.eval(&module).unwrap().into_one().unwrap().value(),
            crate::Value::boolean(true)
        );
    }

    #[test]
    fn srfi_175_install_conflicts_with_a_user_registered_public_library() {
        let public_name = LibraryName::new([
            LibraryNameComponent::identifier("srfi"),
            LibraryNameComponent::number(175),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine
            .register_library_source(
                public_name,
                "(srfi 175)",
                "(define-library (srfi 175) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
            )
            .unwrap();
        assert_eq!(
            engine
                .install_extension(Extension::Srfi175)
                .unwrap_err()
                .kind(),
            ErrorKind::LibraryError
        );
    }

    #[test]
    fn srfi_175_install_conflicts_with_a_user_registered_native_library() {
        let native_name = LibraryName::new([
            LibraryNameComponent::identifier("srfi"),
            LibraryNameComponent::number(175),
            LibraryNameComponent::identifier("native"),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine
            .register_library_source(
                native_name,
                "(srfi 175 native)",
                "(define-library (srfi 175 native) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
            )
            .unwrap();
        assert_eq!(
            engine
                .install_extension(Extension::Srfi175)
                .unwrap_err()
                .kind(),
            ErrorKind::LibraryError
        );
    }

    #[test]
    fn bytevector_double_install_is_an_idempotent_no_op() {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Bytevector).unwrap();
        // A second install returns Ok without re-registering the library.
        engine.install_extension(Extension::Bytevector).unwrap();
        let module = engine
            .compile(
                "test.scm",
                "(import (scheme bytevector))
                 (bytevector-u16-ref (u8-list->bytevector '(1 2)) 0 (endianness little))",
            )
            .unwrap();
        assert_eq!(
            engine.eval(&module).unwrap().into_one().unwrap().value(),
            crate::Value::integer(513)
        );
    }

    #[test]
    fn bytevector_install_conflicts_with_a_user_registered_library() {
        let public_name = LibraryName::new([
            LibraryNameComponent::identifier("scheme"),
            LibraryNameComponent::identifier("bytevector"),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine
            .register_library_source(
                public_name,
                "(scheme bytevector)",
                "(define-library (scheme bytevector) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
            )
            .unwrap();
        assert_eq!(
            engine
                .install_extension(Extension::Bytevector)
                .unwrap_err()
                .kind(),
            ErrorKind::LibraryError
        );
    }

    #[test]
    fn bytevector_user_registration_conflicts_after_install() {
        let public_name = LibraryName::new([
            LibraryNameComponent::identifier("scheme"),
            LibraryNameComponent::identifier("bytevector"),
        ])
        .unwrap();
        let mut engine = Engine::new(EngineConfig::default()).unwrap();
        engine.install_extension(Extension::Bytevector).unwrap();
        let result = engine.register_library_source(
            public_name,
            "(scheme bytevector)",
            "(define-library (scheme bytevector) (export nothing) (import (scheme base)) (begin (define nothing 0)))",
        );
        assert!(result.is_err());
    }
}
