# (r7rs basic-hash-table)

```scheme
(import (r7rs basic-hash-table))   ; also available as (srfi 69)
```

Basic hash tables with a configurable equivalence and hash function.
This is SRFI 69.

## Constructors and predicates

- `(make-hash-table [equivalence hash arg ...])` - create an empty hash table
  using the given equivalence and hash procedures -> hash table.
- `(hash-table? object)` - `#t` if object is a hash table.
- `(alist->hash-table alist [equivalence hash arg ...])` - build a hash table
  from an association list -> hash table.
- `(hash-table-equivalence-function hash-table)` - the equivalence procedure of
  the table.
- `(hash-table-hash-function hash-table)` - the hash procedure of the table.

## Access and mutation

- `(hash-table-ref hash-table key [thunk])` - value for key; call thunk when
  absent, else error -> value.
- `(hash-table-ref/default hash-table key default)` - value for key, or default
  when absent -> value.
- `(hash-table-set! hash-table key value)` - associate key with value.
- `(hash-table-delete! hash-table key)` - remove any entry for key.
- `(hash-table-exists? hash-table key)` - `#t` if key has an entry.
- `(hash-table-update! hash-table key updater [thunk])` - replace key's value
  with `(updater old)`, using thunk for the missing case.
- `(hash-table-update!/default hash-table key updater default)` - like update!
  but use default when key is absent.

## Whole-table operations

- `(hash-table-size hash-table)` - number of entries -> integer.
- `(hash-table-keys hash-table)` - list of all keys -> list.
- `(hash-table-values hash-table)` - list of all values -> list.
- `(hash-table-walk hash-table procedure)` - call `(procedure key value)` for
  every entry.
- `(hash-table-fold hash-table procedure seed)` - fold `(procedure key value
  acc)` over all entries -> final accumulator.
- `(hash-table->alist hash-table)` - association list of all entries -> list.
- `(hash-table-copy hash-table)` - a fresh copy of the table -> hash table.
- `(hash-table-merge! hash-table other)` - add all entries of other into
  hash-table -> hash-table.

## Hash functions

- `(hash object [bound])` - a general-purpose hash value, below bound if given
  -> integer.
- `(string-hash string [bound])` - hash for strings -> integer.
- `(string-ci-hash string [bound])` - case-insensitive string hash -> integer.
- `(hash-by-identity object [bound])` - hash based on object identity ->
  integer.
