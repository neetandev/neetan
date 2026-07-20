# (r7rs sorting)

```scheme
(import (r7rs sorting))   ; also available as (srfi 132)
```

The SRFI 132 sort libraries: stable and unstable sorting, merging, median
selection, and neighbor-duplicate deletion for both lists and vectors, all
driven by a caller-supplied less-than comparator. This is SRFI 132.

Note: `list-sort!` and `list-stable-sort!` are functional aliases of their
non-destructive counterparts and do not mutate. The other destructive list
variants (`list-merge!`, `list-delete-neighbor-dups!`) and every destructive
vector variant truly mutate their argument in place. In all cases you must use
the return value.

## Predicates

- `(list-sorted? < list)` - `#t` if the list is ordered by `<`.
- `(vector-sorted? < vec)` / `(vector-sorted? < vec start)` /
  `(vector-sorted? < vec start end)` - `#t` if the vector range is ordered by `<`.

## Sorting

- `(list-sort < list)` - sorted copy of the list -> list.
- `(list-stable-sort < list)` - stably sorted copy of the list -> list.
- `(list-sort! < list)` - like `list-sort` (linear-update alias).
- `(list-stable-sort! < list)` - like `list-stable-sort` (linear-update alias).
- `(vector-sort < vec)` - sorted copy of the vector -> vector.
- `(vector-stable-sort < vec)` - stably sorted copy of the vector -> vector.
- `(vector-sort! < vec)` / `(vector-sort! < vec start)` /
  `(vector-sort! < vec start end)` - sort the vector range in place.
- `(vector-stable-sort! < vec)` / `(vector-stable-sort! < vec start)` /
  `(vector-stable-sort! < vec start end)` - stably sort the vector range in place.

## Merging

- `(list-merge < list1 list2)` - merge two ordered lists into one ordered list
  -> list.
- `(list-merge! < list1 list2)` - like `list-merge` (linear-update alias).
- `(vector-merge < vec1 vec2)` - merge two ordered vectors into a new ordered
  vector -> vector.
- `(vector-merge! < to from1 from2)` / `(vector-merge! < to from1 from2 start)` /
  `(vector-merge! < to from1 from2 start start1)` /
  `(vector-merge! < to from1 from2 start start1 end1)` /
  `(vector-merge! < to from1 from2 start start1 end1 start2)` /
  `(vector-merge! < to from1 from2 start start1 end1 start2 end2)` - merge two
  ordered vector ranges into `to` in place.

## Deleting neighbor duplicates

- `(list-delete-neighbor-dups = list)` - drop each element equal to its
  predecessor -> list.
- `(list-delete-neighbor-dups! = list)` - like `list-delete-neighbor-dups`
  (linear-update alias).
- `(vector-delete-neighbor-dups = vec)` / `(... = vec start)` /
  `(... = vec start end)` - copy the range dropping adjacent duplicates -> vector.
- `(vector-delete-neighbor-dups! = vec)` / `(... = vec start)` /
  `(... = vec start end)` - compact adjacent duplicates in place -> new end index.

## Selection and median

- `(vector-find-median < vec knil)` / `(vector-find-median < vec knil mean)` -
  median of a copy of the vector, using `mean` for even lengths -> value.
- `(vector-find-median! < vec knil)` / `(vector-find-median! < vec knil mean)` -
  like `vector-find-median` but sorts the vector in place -> value.
- `(vector-select! < vec k)` / `(vector-select! < vec k start)` /
  `(vector-select! < vec k start end)` - the `k`th smallest element of the range,
  reordering the vector -> value.
- `(vector-separate! < vec k)` / `(vector-separate! < vec k start)` /
  `(vector-separate! < vec k start end)` - partition so the `k` smallest elements
  occupy the front of the range, unordered among themselves.
