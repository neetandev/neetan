# (r7rs lists)

```scheme
(import (r7rs lists))   ; also available as (srfi 1)
```

The SRFI 1 list library: a large toolbox of constructors, selectors,
predicates, folds, filters, searches, and set operations over proper, circular,
and dotted lists. This is SRFI 1.

Note: the `!` linear-update procedures are permitted to reuse the storage of
their arguments, but in this implementation they are functional aliases of their
non-destructive counterparts, so you must still use their return value.

## Re-exported list primitives

These names come straight from `(scheme base)` and `(scheme cxr)` and behave
exactly as they do there.

- `(cons obj1 obj2)` - new pair (same as in (scheme base)).
- `(car pair)` - the car field (same as in (scheme base)).
- `(cdr pair)` - the cdr field (same as in (scheme base)).
- `(caar pair)` - `(car (car pair))` (same as in (scheme cxr)).
- `(cadr pair)` - `(car (cdr pair))` (same as in (scheme cxr)).
- `(cdar pair)` - `(cdr (car pair))` (same as in (scheme cxr)).
- `(cddr pair)` - `(cdr (cdr pair))` (same as in (scheme cxr)).
- `(caaar pair)` - three-deep car/cdr composition (same as in (scheme cxr)).
- `(caadr pair)` - three-deep car/cdr composition (same as in (scheme cxr)).
- `(cadar pair)` - three-deep car/cdr composition (same as in (scheme cxr)).
- `(caddr pair)` - three-deep car/cdr composition (same as in (scheme cxr)).
- `(cdaar pair)` - three-deep car/cdr composition (same as in (scheme cxr)).
- `(cdadr pair)` - three-deep car/cdr composition (same as in (scheme cxr)).
- `(cddar pair)` - three-deep car/cdr composition (same as in (scheme cxr)).
- `(cdddr pair)` - three-deep car/cdr composition (same as in (scheme cxr)).
- `(caaaar pair)` - four-deep car/cdr composition (same as in (scheme cxr)).
- `(caaadr pair)` - four-deep car/cdr composition (same as in (scheme cxr)).
- `(caadar pair)` - four-deep car/cdr composition (same as in (scheme cxr)).
- `(caaddr pair)` - four-deep car/cdr composition (same as in (scheme cxr)).
- `(cadaar pair)` - four-deep car/cdr composition (same as in (scheme cxr)).
- `(cadadr pair)` - four-deep car/cdr composition (same as in (scheme cxr)).
- `(caddar pair)` - four-deep car/cdr composition (same as in (scheme cxr)).
- `(cadddr pair)` - four-deep car/cdr composition (same as in (scheme cxr)).
- `(cdaaar pair)` - four-deep car/cdr composition (same as in (scheme cxr)).
- `(cdaadr pair)` - four-deep car/cdr composition (same as in (scheme cxr)).
- `(cdadar pair)` - four-deep car/cdr composition (same as in (scheme cxr)).
- `(cdaddr pair)` - four-deep car/cdr composition (same as in (scheme cxr)).
- `(cddaar pair)` - four-deep car/cdr composition (same as in (scheme cxr)).
- `(cddadr pair)` - four-deep car/cdr composition (same as in (scheme cxr)).
- `(cdddar pair)` - four-deep car/cdr composition (same as in (scheme cxr)).
- `(cddddr pair)` - four-deep car/cdr composition (same as in (scheme cxr)).
- `(pair? obj)` - `#t` if the object is a pair (same as in (scheme base)).
- `(null? obj)` - `#t` if the object is the empty list (same as in (scheme base)).
- `(list obj ...)` - list of the arguments (same as in (scheme base)).
- `(list? obj)` - `#t` if the object is a proper list (same as in (scheme base)).
- `(length list)` - number of elements -> integer (same as in (scheme base)).
- `(append list ...)` - concatenation of the lists (same as in (scheme base)).
- `(reverse list)` - reversed copy of the list (same as in (scheme base)).
- `(list-ref list k)` - the `k`th element (same as in (scheme base)).
- `(list-copy obj)` - shallow copy of the list (same as in (scheme base)).
- `(make-list k)` / `(make-list k fill)` - list of `k` elements (same as in
  (scheme base)).
- `(map proc list1 list2 ...)` - map `proc` over the lists (same as in (scheme
  base)).
- `(for-each proc list1 list2 ...)` - apply `proc` for effect (same as in
  (scheme base)).
- `(member obj list)` / `(member obj list compare)` - first tail whose head
  matches (same as in (scheme base)).
- `(memq obj list)` - `member` using `eq?` (same as in (scheme base)).
- `(memv obj list)` - `member` using `eqv?` (same as in (scheme base)).
- `(assoc obj alist)` / `(assoc obj alist compare)` - first matching pair (same
  as in (scheme base)).
- `(assq obj alist)` - `assoc` using `eq?` (same as in (scheme base)).
- `(assv obj alist)` - `assoc` using `eqv?` (same as in (scheme base)).
- `(set-car! pair obj)` - store into the car field (same as in (scheme base)).
- `(set-cdr! pair obj)` - store into the cdr field (same as in (scheme base)).

## Constructors

- `(xcons d a)` - `(cons a d)` with the arguments exchanged -> pair.
- `(cons* elt1 elt2 ... tail)` - list of the leading elements ending in `tail`
  -> improper or proper list.
- `(list-tabulate n init-proc)` - length-`n` list of `(init-proc i)` for each
  index -> list.
- `(circular-list elt1 elt2 ...)` - circular list of the elements -> pair.
- `(iota count)` / `(iota count start)` / `(iota count start step)` - list of
  `count` numbers from `start` by `step` -> list.

## Predicates

- `(proper-list? x)` - `#t` if `x` is a finite nil-terminated list.
- `(circular-list? x)` - `#t` if `x` is a circular list.
- `(dotted-list? x)` - `#t` if `x` is a non-nil-terminated (improper) list.
- `(not-pair? x)` - `#t` if `x` is not a pair (`(lambda (x) (not (pair? x)))`).
- `(null-list? list)` - `#t` if the list is empty, error if it is not a list.
- `(list= elt=? list1 ...)` - `#t` if all lists are equal elementwise under
  `elt=?`.

## Selectors

- `(first pair)` - first element of the list.
- `(second pair)` - second element of the list.
- `(third pair)` - third element of the list.
- `(fourth pair)` - fourth element of the list.
- `(fifth pair)` - fifth element of the list.
- `(sixth pair)` - sixth element of the list.
- `(seventh pair)` - seventh element of the list.
- `(eighth pair)` - eighth element of the list.
- `(ninth pair)` - ninth element of the list.
- `(tenth pair)` - tenth element of the list.
- `(car+cdr pair)` - the car and the cdr -> two values.
- `(take list k)` - list of the first `k` elements -> list.
- `(drop list k)` - the list with its first `k` elements removed -> list.
- `(take-right flist k)` - the last `k` elements of the list -> list.
- `(drop-right flist k)` - the list with its last `k` elements removed -> list.
- `(split-at list k)` - the prefix of length `k` and the rest -> two values.
- `(last pair)` - the last element of the non-empty list.
- `(last-pair pair)` - the last pair of the non-empty list -> pair.

## Miscellaneous

- `(length+ x)` - length of the list, or `#f` if it is circular -> integer or #f.
- `(concatenate list-of-lists)` - append the member lists together -> list.
- `(append! list1 ...)` - like `append` (linear-update alias).
- `(concatenate! list-of-lists)` - like `concatenate` (linear-update alias).
- `(reverse! list)` - like `reverse` (linear-update alias).
- `(append-reverse rev-head tail)` - `(append (reverse rev-head) tail)` -> list.
- `(append-reverse! rev-head tail)` - like `append-reverse` (linear-update alias).
- `(zip list1 list2 ...)` - list of lists grouping the `i`th elements -> list.
- `(unzip1 list)` - list of the first elements of each sublist -> list.
- `(unzip2 list)` - the first and second element lists -> two values.
- `(unzip3 list)` - the first three element lists -> three values.
- `(unzip4 list)` - the first four element lists -> four values.
- `(unzip5 list)` - the first five element lists -> five values.
- `(count pred list1 list2 ...)` - number of index tuples for which `pred` is
  true -> integer.

## Fold, unfold, and map

- `(fold kons knil list1 list2 ...)` - left fold, `(kons elt ... acc)` -> value.
- `(fold-right kons knil list1 list2 ...)` - right fold, `(kons elt ... acc)`
  -> value.
- `(reduce f ridentity list)` - fold using the first element as the seed
  -> value.
- `(reduce-right f ridentity list)` - right fold using the last element as the
  seed -> value.
- `(unfold p f g seed)` / `(unfold p f g seed tail-gen)` - build a list from a
  seed until `p` holds -> list.
- `(unfold-right p f g seed)` / `(unfold-right p f g seed tail)` - build a list
  right to left from a seed -> list.
- `(pair-fold kons knil list1 ...)` - left fold over successive pairs of the list
  -> value.
- `(pair-fold-right kons knil list1 ...)` - right fold over successive pairs
  -> value.
- `(append-map f list1 list2 ...)` - map then append the resulting lists -> list.
- `(append-map! f list1 list2 ...)` - like `append-map` (linear-update alias).
- `(map-in-order proc list1 list2 ...)` - like `map` but guarantees
  left-to-right application order -> list.
- `(filter-map proc list1 list2 ...)` - map `proc`, keeping only true results
  -> list.
- `(map! proc list1 list2 ...)` - like `map` (linear-update alias).
- `(pair-for-each proc list1 ...)` - apply `proc` to successive pairs for effect.

## Filtering and partitioning

- `(filter pred list)` - elements satisfying `pred` -> list.
- `(remove pred list)` - elements not satisfying `pred` -> list.
- `(partition pred list)` - the matching and non-matching elements -> two values.
- `(filter! pred list)` - like `filter` (linear-update alias).
- `(remove! pred list)` - like `remove` (linear-update alias).
- `(partition! pred list)` - like `partition` (linear-update alias).

## Searching

- `(find pred list)` - first element satisfying `pred`, else `#f`.
- `(find-tail pred list)` - first tail whose head satisfies `pred`, else `#f`.
- `(any pred list1 list2 ...)` - first true `(pred elt ...)` result, else `#f`.
- `(every pred list1 list2 ...)` - last `(pred elt ...)` result if all true, else
  `#f`.
- `(list-index pred list1 list2 ...)` - index of the first satisfying tuple, else
  `#f`.
- `(take-while pred list)` - longest prefix whose elements satisfy `pred` -> list.
- `(take-while! pred list)` - like `take-while` (linear-update alias).
- `(drop-while pred list)` - the list past that longest prefix -> list.
- `(span pred list)` - the `take-while` prefix and the remaining tail -> two
  values.
- `(span! pred list)` - like `span` (linear-update alias).
- `(break pred list)` - split before the first element satisfying `pred` -> two
  values.
- `(break! pred list)` - like `break` (linear-update alias).

## Deletion

- `(delete x list)` / `(delete x list =)` - remove elements equal to `x` -> list.
- `(delete! x list)` / `(delete! x list =)` - like `delete` (linear-update alias).
- `(delete-duplicates list)` / `(delete-duplicates list =)` - remove later
  duplicates -> list.
- `(delete-duplicates! list)` / `(delete-duplicates! list =)` - like
  `delete-duplicates` (linear-update alias).

## Association lists

- `(alist-cons key datum alist)` - prepend `(key . datum)` to the alist -> list.
- `(alist-copy alist)` - fresh copy of the alist and its pairs -> list.
- `(alist-delete key alist)` / `(alist-delete key alist =)` - remove entries
  whose key matches -> list.
- `(alist-delete! key alist)` / `(alist-delete! key alist =)` - like
  `alist-delete` (linear-update alias).

## Set operations on lists

- `(lset<= = list1 ...)` - `#t` if each list is a subset of the next under `=`.
- `(lset= = list1 ...)` - `#t` if all lists are set-equal under `=`.
- `(lset-adjoin = list elt1 ...)` - add the elements not already present -> list.
- `(lset-union = list1 ...)` - set union of the lists -> list.
- `(lset-intersection = list1 list2 ...)` - set intersection of the lists -> list.
- `(lset-difference = list1 list2 ...)` - elements of `list1` in no later list
  -> list.
- `(lset-xor = list1 ...)` - set symmetric difference of the lists -> list.
- `(lset-diff+intersection = list1 list2 ...)` - the difference and the
  intersection -> two values.
- `(lset-union! = list1 ...)` - like `lset-union` (linear-update alias).
- `(lset-intersection! = list1 list2 ...)` - like `lset-intersection`
  (linear-update alias).
- `(lset-difference! = list1 list2 ...)` - like `lset-difference` (linear-update
  alias).
- `(lset-xor! = list1 ...)` - like `lset-xor` (linear-update alias).
- `(lset-diff+intersection! = list1 list2 ...)` - like `lset-diff+intersection`
  (linear-update alias).

## Linear-update selectors

- `(take! list k)` - like `take` (linear-update alias).
- `(drop-right! flist k)` - like `drop-right` (linear-update alias).
- `(split-at! list k)` - like `split-at` (linear-update alias).
