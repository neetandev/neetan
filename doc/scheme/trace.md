# (neetan trace 1)

```scheme
(import (neetan trace 1))
```

Generic event tracing over the machine's `TraceSink`: memory and I/O accesses,
interrupts, scheduled events, framebuffer presentations, device actions, and
firmware or OS calls. Requires the machine's `trace` capability. Use
`trace-schema` to discover what the selected machine emits rather than assuming
every target emits every class. Every procedure takes a `machine` handle from
[`(neetan automation 1)`](automation.md).

## Discovery

- `(trace-schema machine)` -> alist. The discovery descriptor: `schema-version`,
  `queue-limits`, `envelope-fields`, `classes`, `supported-classes`,
  `address-spaces`, `controllers`, `scheduled`, `devices`, and `providers`. It
  also reports which numeric fields accept `(range minimum maximum)`.

## Continuous collection

- `(trace-start! machine filter)` -> unspecified. Begins continuous collection.
  `filter` is a declarative alist over the event shape, or `()` for all
  supported events. Missing keys are wildcards; scalars match with `equal?`;
  `(range minimum maximum)` is an inclusive numeric range on range-capable
  fields. Invalid keys, types, or ranges fail before machine execution.
- `(trace-active? machine)` -> boolean. Whether continuous collection is on.
- `(trace-stop! machine)` -> unspecified. Stops collection without discarding
  buffered events or the failure report.
- `(trace-drain! machine)` -> list of event alists. Returns buffered events in
  increasing `sequence` order and empties the event buffer (but not the failure
  report).
- `(trace-failure machine)` -> alist or `#f`. The sticky first overflow report,
  whose `reason` is `queue-overflow`, `event-payload-too-large`, or
  `sequence-exhausted`, with the applicable limits. The next `trace-start!`
  clears it.

## One-shot wait

- `(wait-for-event machine filter)` / `(wait-for-event machine filter options)`
  -> event alist or `#f`. A standalone one-shot: runs the machine until the
  first event matching `filter`, then returns it, or `#f` when a bound is
  exhausted. Options keys `frames` (default `120`) and `ticks`. It is invalid
  while `trace-active?` is true (raises `neetan/trace-state`).

## Event shape

Every normalized event is an alist with keys `schema-version`, `sequence`,
`epoch`, `tick`, `source`, `clock-domain`, `clock-cycle`, `clock-rate`, `class`,
and `data`. `clock-rate` is `#f` or an alist with `numerator` and `denominator`.
The required `data` keys by `class` are:

- `access`: `space`, `space-class`, `operation`, `address`, `width` (bits),
  `value` (`#f` when unavailable), and `handled?`.
- `interrupt`: `controller`, `interrupt-kind`, `line`, `action`, and `vector`
  (`line` and `vector` may be `#f`).
- `scheduled`: `event` and `fire-tick`.
- `presentation`: `display`, `frame`, `width`, and `height`.
- `device`: `device`, `action`, and `fields`.
- `call`: `provider`, `interface` (an alist with `kind` and `value`), `phase`,
  and `fields`.

A continuous or one-shot overflow raises `neetan/trace-overflow` on the active
run; the buffer keeps the earliest complete events.
