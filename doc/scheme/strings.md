# (r7rs strings)

```scheme
(import (r7rs strings))   ; also available as (srfi 152)
```

The SRFI 152 string library: predicates, comparison, prefix and suffix testing,
searching, folding, mapping, padding, trimming, joining, and splitting over
strings, with cursor and character-set friendly conventions. This is SRFI 152.

## Re-exported string primitives

These names come from `(scheme base)` and `(scheme char)` and behave exactly as
they do there.

- `(string? obj)` - `#t` if the object is a string (same as in (scheme base)).
- `(make-string k)` / `(make-string k char)` - string of `k` characters (same as
  in (scheme base)).
- `(string char ...)` - string of the given characters (same as in (scheme base)).
- `(string-length string)` - number of characters -> integer (same as in (scheme
  base)).
- `(string-ref string k)` - the `k`th character (same as in (scheme base)).
- `(substring string start end)` - substring of the range (same as in (scheme
  base)).
- `(string-copy string)` / `(string-copy string start)` / `(string-copy string
  start end)` - copy of the range (same as in (scheme base)).
- `(string-copy! to at from)` / `(... from start)` / `(... from start end)` -
  copy into `to` (same as in (scheme base)).
- `(string-fill! string fill)` / `(... start)` / `(... start end)` - set the
  range to `fill` (same as in (scheme base)).
- `(string-set! string k char)` - store a character (same as in (scheme base)).
- `(string->list string)` / `(... start)` / `(... start end)` - characters of the
  range -> list (same as in (scheme base)).
- `(list->string list)` - string from a list of characters (same as in (scheme
  base)).
- `(string->vector string)` / `(... start)` / `(... start end)` - characters as a
  vector (same as in (scheme base)).
- `(vector->string vec)` / `(... start)` / `(... start end)` - string from a
  vector of characters (same as in (scheme base)).
- `(string-append string ...)` - concatenation of the strings (same as in (scheme
  base)).
- `(string-for-each proc string1 ...)` - apply `proc` per character for effect
  (same as in (scheme base)).
- `(read-string k)` / `(read-string k port)` - read up to `k` characters -> string
  or eof (same as in (scheme base)).
- `(write-string string)` / `(... port)` / `(... port start)` / `(... port start
  end)` - write the range (same as in (scheme base)).
- `(string=? string1 string2 ...)` - equality comparison (same as in (scheme
  base)).
- `(string<? string1 string2 ...)` - monotonic < comparison (same as in (scheme
  base)).
- `(string>? string1 string2 ...)` - monotonic > comparison (same as in (scheme
  base)).
- `(string<=? string1 string2 ...)` - monotonic <= comparison (same as in (scheme
  base)).
- `(string>=? string1 string2 ...)` - monotonic >= comparison (same as in (scheme
  base)).
- `(string-ci=? string1 string2 ...)` - case-insensitive equality (same as in
  (scheme char)).
- `(string-ci<? string1 string2 ...)` - case-insensitive < comparison (same as in
  (scheme char)).
- `(string-ci>? string1 string2 ...)` - case-insensitive > comparison (same as in
  (scheme char)).
- `(string-ci<=? string1 string2 ...)` - case-insensitive <= comparison (same as
  in (scheme char)).
- `(string-ci>=? string1 string2 ...)` - case-insensitive >= comparison (same as
  in (scheme char)).

## Predicates

- `(string-null? string)` - `#t` if the string is empty.
- `(string-every pred string)` / `(... start)` / `(... start end)` - last `pred`
  result if every character satisfies it, else `#f`.
- `(string-any pred string)` / `(... start)` / `(... start end)` - first true
  `pred` result over the characters, else `#f`.

## Constructors

- `(string-tabulate proc len)` - length-`len` string of `(proc i)` per index
  -> string.
- `(string-unfold stop? mapper successor seed)` / `(... base)` / `(... base
  make-final)` - build a string left to right from a seed -> string.
- `(string-unfold-right stop? mapper successor seed)` / `(... base)` / `(...
  base make-final)` - build a string right to left from a seed -> string.
- `(reverse-list->string char-list)` - string from a reversed list of characters
  -> string.

## Prefixes, suffixes, and search

- `(string-prefix-length s1 s2)` / `(... start1)` / `(... start1 end1)` / `(...
  start1 end1 start2)` / `(... start1 end1 start2 end2)` - length of the common
  prefix -> integer.
- `(string-suffix-length s1 s2)` / `(... start1)` / `(... start1 end1)` / `(...
  start1 end1 start2)` / `(... start1 end1 start2 end2)` - length of the common
  suffix -> integer.
- `(string-prefix? s1 s2)` / `(... start1)` / `(... start1 end1)` / `(... start1
  end1 start2)` / `(... start1 end1 start2 end2)` - `#t` if `s1` is a prefix of
  `s2`.
- `(string-suffix? s1 s2)` / `(... start1)` / `(... start1 end1)` / `(... start1
  end1 start2)` / `(... start1 end1 start2 end2)` - `#t` if `s1` is a suffix of
  `s2`.
- `(string-contains s1 s2)` / `(... start1)` / `(... start1 end1)` / `(... start1
  end1 start2)` / `(... start1 end1 start2 end2)` - index of the first occurrence
  of `s2` in `s1`, else `#f`.
- `(string-contains-right s1 s2)` / `(... start1)` / `(... start1 end1)` / `(...
  start1 end1 start2)` / `(... start1 end1 start2 end2)` - index of the last
  occurrence of `s2` in `s1`, else `#f`.
- `(string-index string pred)` / `(... start)` / `(... start end)` - index of the
  first character satisfying `pred`, else `#f`.
- `(string-index-right string pred)` / `(... start)` / `(... start end)` - index
  of the last character satisfying `pred`, else `#f`.
- `(string-skip string pred)` / `(... start)` / `(... start end)` - index of the
  first character not satisfying `pred`, else `#f`.
- `(string-skip-right string pred)` / `(... start)` / `(... start end)` - index
  of the last character not satisfying `pred`, else `#f`.

## Selection

- `(string-take string nchars)` - the first `nchars` characters -> string.
- `(string-drop string nchars)` - the string without its first `nchars`
  characters -> string.
- `(string-take-right string nchars)` - the last `nchars` characters -> string.
- `(string-drop-right string nchars)` - the string without its last `nchars`
  characters -> string.
- `(string-take-while string pred)` / `(... start)` / `(... start end)` - longest
  prefix of characters satisfying `pred` -> string.
- `(string-take-while-right string pred)` / `(... start)` / `(... start end)` -
  longest suffix of characters satisfying `pred` -> string.
- `(string-drop-while string pred)` / `(... start)` / `(... start end)` - the
  string past that longest prefix -> string.
- `(string-drop-while-right string pred)` / `(... start)` / `(... start end)` -
  the string before that longest suffix -> string.
- `(string-span string pred)` / `(... start)` / `(... start end)` - the
  take-while prefix and the remaining string -> two values.
- `(string-break string pred)` / `(... start)` / `(... start end)` - split before
  the first character satisfying `pred` -> two values.

## Padding and trimming

- `(string-pad string len)` / `(... char)` / `(... char start)` / `(... char
  start end)` - right-justify to width `len`, padding or clipping on the left
  -> string.
- `(string-pad-right string len)` / `(... char)` / `(... char start)` / `(...
  char start end)` - left-justify to width `len`, padding or clipping on the
  right -> string.
- `(string-trim string)` / `(... pred)` / `(... pred start)` / `(... pred start
  end)` - drop leading characters satisfying `pred` -> string.
- `(string-trim-right string)` / `(... pred)` / `(... pred start)` / `(... pred
  start end)` - drop trailing characters satisfying `pred` -> string.
- `(string-trim-both string)` / `(... pred)` / `(... pred start)` / `(... pred
  start end)` - drop leading and trailing characters satisfying `pred` -> string.

## Replacement, folding, and mapping

- `(string-replace s1 s2 start1 end1)` / `(... start2)` / `(... start2 end2)` -
  replace the `s1` range with the `s2` range -> string.
- `(string-replicate string from to)` / `(... start)` / `(... start end)` - the
  circularly indexed range `[from, to)` of the string -> string.
- `(string-fold kons knil string)` / `(... start)` / `(... start end)` - left
  fold over the characters -> value.
- `(string-fold-right kons knil string)` / `(... start)` / `(... start end)` -
  right fold over the characters -> value.
- `(string-count string pred)` / `(... start)` / `(... start end)` - number of
  characters satisfying `pred` -> integer.
- `(string-filter pred string)` / `(... start)` / `(... start end)` - the
  characters satisfying `pred` -> string.
- `(string-remove pred string)` / `(... start)` / `(... start end)` - the
  characters not satisfying `pred` -> string.
- `(string-map proc string1 string2 ...)` - map `proc` over the characters
  -> string.

## Concatenation, joining, and splitting

- `(string-concatenate string-list)` - append the strings in the list -> string.
- `(string-concatenate-reverse string-list)` / `(... final-string)` / `(...
  final-string end)` - append the reversed list of strings -> string.
- `(string-join string-list)` / `(... delimiter)` / `(... delimiter grammar)` -
  join with a delimiter under the given grammar -> string.
- `(string-segment string k)` - split into consecutive chunks of at most `k`
  characters -> list of strings.
- `(string-split string delimiter)` / `(... grammar)` / `(... grammar limit)` /
  `(... grammar limit start)` / `(... grammar limit start end)` - split on the
  delimiter -> list of strings.
