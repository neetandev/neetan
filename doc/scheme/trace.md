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
  also reports which numeric fields accept `(range minimum maximum)`. Each
  device entry lists its actions as alists with `action` and `fields`, where
  every field descriptor carries `name`, `type`, and `range`. Providers list
  their `call-fields` the same way.

## Continuous collection

- `(trace-start! machine filter)` -> unspecified. Begins continuous collection.
  `filter` is a declarative alist over the event shape, or `()` for all
  supported events. Missing keys are wildcards; scalars match with `equal?`;
  `(range minimum maximum)` is an inclusive numeric range on range-capable
  fields. Invalid keys, types, or ranges fail before machine execution.

  A device filter's `data` block may carry a nested `fields` alist constraining
  the provider-specific fields the schema declares for the named `device` and
  `action`, or for the named call `provider`. Constraints may be symbols,
  integers, booleans, bytevectors, ranges, or `#f` to match a falseable field
  with no value. An integer or range constraint on an `integer-list` field
  (such as escape `parameters`) matches when any list element satisfies it:

  ```scheme
  (wait-for-event machine
    '((class . device)
      (data . ((device . neetan.dos.stdout)
               (action . write)
               (fields . ((source . int21.40)
                          (handle . 1)))))))
  ```

  Unknown field names and wrong value types are rejected before the machine
  runs. Naming a specific device narrows collection so other high-volume
  device events are never built, and naming an `action` too narrows it
  further, so sibling actions of the same device are also never built.
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
  exhausted. Options keys `frames` (default `120`), `ticks`, and `snapshot` (a
  list holding one processor symbol, for example `(snapshot . (cpu.main))`). It
  is invalid while `trace-active?` is true (raises `neetan/trace-state`).

  With `snapshot`, the returned event carries a `snapshot` key mapping the
  processor to its register alist. The registers are captured at HLE dispatch
  entry, before the handler can clobber them, so events emitted during an HLE
  call expose the guest's syscall arguments. The boundary `call` events carry
  the same entry snapshot on both the `enter` and `exit` phases. Events
  outside an HLE dispatch marshal `snapshot` as `#f`.

## Artifacts

- `(save-trace! machine path)` -> alist with `path` and `bytes`. Writes the
  buffered events beneath the artifact root as Scheme data, one event datum per
  line in the `trace-drain!` alist shape, reading back with `read`. Byte fields
  are `#u8(...)` literals and integers are exact. Non-consuming; the buffer
  stays drainable. Note that `wait-for-event` drains the buffer when it
  matches, so a `save-trace!` right after a one-shot wait writes an empty
  artifact. Path traversal and absolute paths raise `neetan/path-escape`.

## Triggered ring capture

- `(trace-arm! machine spec)` -> alist. Runs a bounded before-and-after capture
  around a trigger event. `spec` is an alist with required keys `capture` and
  `trigger` (declarative filters as for `trace-start!`), `before` and `after`
  (event counts), and `artifact` (output path), plus optional `frames` and
  `ticks` run bounds:

  ```scheme
  (trace-arm! machine
    '((capture . ((class . access)))
      (trigger . ((class . device)
                  (data . ((device . neetan.dos.stdout)
                           (action . write)))))
      (before . 512)
      (after . 512)
      (artifact . "stdout-context.scm")))
  ```

  Events matching `capture` are kept in a window of at most `before` events
  preceding the first `trigger` match and `after` events following it. Storage
  is bounded from the moment of arming: the window is limited to the queue
  event capacity (`before + after + 1` beyond it is rejected up front) and the
  retained payload bytes are charged against the queue byte capacity, so a
  ring capture honours the same `queue-limits` the schema advertises. Both
  filters are evaluated in Rust before any owned payload is allocated, and the
  machine stops as soon as the post-trigger context is complete. Once the
  trigger has fired the retained events are written to `artifact` in the
  `save-trace!` format. Filter and path validation happen before the machine
  runs, and the capture always disarms on return.

  The result alist carries `triggered`, `complete`, `events`, `trigger-index`
  (`#f` until triggered), and `bytes` (`#f` when no artifact was written). It
  is invalid while `trace-active?` is true (raises `neetan/trace-state`). An
  oversized event payload or an exhausted byte capacity raises
  `neetan/trace-overflow` with the sticky failure available from
  `trace-failure`; when the trigger had already fired, the partially retained
  window is still written to `artifact` before the failure is raised.

## Event shape

Every normalized event is an alist with keys `schema-version`, `sequence`,
`epoch`, `tick`, `source`, `clock-domain`, `clock-cycle`, `clock-rate`, `class`,
`data`, and `snapshot` (`#f` unless a register snapshot was armed and the event
was emitted during an HLE dispatch). `clock-rate` is `#f` or an alist with
`numerator` and `denominator`. The required `data` keys by `class` are:

- `access`: `space`, `space-class`, `operation`, `address`, `width` (bits),
  `value` (`#f` when unavailable), and `handled?`.
- `interrupt`: `controller`, `interrupt-kind`, `line`, `action`, and `vector`
  (`line` and `vector` may be `#f`).
- `scheduled`: `event` and `fire-tick`.
- `presentation`: `display`, `frame`, `width`, and `height`.
- `device`: `device`, `action`, and `fields`.
- `call`: `provider`, `interface` (an alist with `kind` and `value`), `phase`,
  and `fields`.

For the HLE DOS devices, `neetan.dos.stdout` reports `handle` as `#f` for the
DOS APIs that take no handle argument (`int21.02`, `int21.06`, `int21.09`, and
`int29`); only `int21.40` carries a real handle. `neetan.dos.console` `escape`
events cover both CSI sequences and the complete non-CSI sequences
(`clear-screen`, `line-feed`, `next-line`, `reverse-line-feed`, `kanji-mode`,
`graphic-mode`, `cursor-address`, and the `set-mode` family), and their
`parameters` field is a list of exact integers.

A continuous or one-shot overflow raises `neetan/trace-overflow` on the active
run; the buffer keeps the earliest complete events.
