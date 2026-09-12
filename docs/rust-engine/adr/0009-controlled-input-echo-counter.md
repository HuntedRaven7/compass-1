# ADR-0009: A controlled input's value carries the edit it answers

**Status:** Accepted · **Date:** 2026-09-12 · Relates to: ADR-0005, PLAN.md §6 Phase 4,
`compass-extension-api::input`

## Context

A *controlled* input is one whose value the extension owns. The user types, the host reports the
change, the extension re-renders with a value, and the host displays it. Raycast's API works this
way, so our extension surface has to as well: `SearchBar.text` + `onSearchTextChange`,
`Form.TextField.value` + `onChange`, `Dropdown.value` + `onChange`.

That round trip crosses a process boundary and the user does not stop typing during it:

```
user types "a"     -> host sends change("a")
user types "ab"    -> host sends change("ab")
user types "abc"   -> host sends change("abc")
                   <- extension renders value "a"      (answering the first change)
```

Applying that render puts `"a"` back in a box the user has typed `"abc"` into, and their next
keystroke lands after the truncation. This is not a rare race. It is what happens whenever the
extension is slower than a typist, which for anything doing I/O is most of the time.

The C++ engine sidesteps it by running the extension's JS in-process with a synchronous-enough
event loop that the window is small. The Rust engine cannot: extensions are out-of-process by
design (ADR-0005), and the Rhai tier makes the latency spread wider still.

The options were:

1. **Ignore it.** What the current code did. The bug is intermittent, blamed on "lag", and
   effectively unfixable once extensions depend on the timing.
2. **Debounce.** Delay applying a render until the user pauses. Trades a wrong value for a laggy
   one, and picking the delay is picking which users to fail.
3. **Last-write-wins on the host.** Never apply an extension's value to a focused input. Breaks
   every legitimate programmatic set — validation, formatting, a Clear button.
4. **Count the edits.** Make the render say which edit it answers.

## Decision

**Each local edit to a controlled input mints a `Seq`; the change carries it; a rendered value
echoes back the `Seq` whose change it answers.** The host compares and drops values computed before
the user's latest keystroke, keeping the rest of the render.

The rule lives in `compass-extension-api::input` as a pure decision — an `EchoTracker` holding a
`u64` per node and a `verdict` function. No transport, no clock, no task; the seam's constraint
holds.

### An absent echo means something different from a stale one

This is the part that will look like an inconsistency to someone tidying up later, so it is the
part worth recording.

| Rendered value | Meaning | Applied? |
|---|---|---|
| no `Seq` | the extension is **setting** the value | always |
| `Seq` == the latest local edit | a current answer | yes |
| `Seq` < the latest local edit | computed before the user's latest keystroke | no |
| `Seq` > the latest local edit | not reachable from this host | no, and logged |

A value with no echo is not an old answer — there is nothing for it to be old relative to. It is the
extension setting the value: a Clear action, a validation rewrite, the initial render. Those must
win over local state, or a Clear button typed over would never clear. Collapsing `None` into
`Some(ZERO)` would break exactly that case, which is why the field is `Option<Seq>` on the wire and
why the encoding is tested to keep them distinct.

The fourth row is unreachable from a correct extension, since a `Seq` originates in the host and is
only ever echoed back. It is refused rather than trusted — applying a value the host cannot place is
the worse of the two mistakes — and reported separately from `Stale` so a host can log it instead of
silently absorbing it.

## Costs

- **Extensions must echo.** An extension that renders a value without the `Seq` it was given is
  saying "I am setting this", and will fight the user's typing. The TS shim will thread it
  automatically; a hand-written Rhai extension can get it wrong. This is the real cost, and it is
  the reason the distinction above is documented at the type rather than in a guide.
- **A `u64` per edited node** on the host, bounded by `retain_only` against the live tree. Without
  that call the map grows for the lifetime of the extension.
- **Derived ids make forgetting necessary.** A node removed and re-created in the same slot gets the
  same `NodeId` (ADR-0006's identity scheme), so its counter has to be dropped with it or its first
  genuine echo compares as stale.
- **The echo must stay out of node fingerprints**, or every keystroke's answer reports the whole
  view as updated. Form fields get that free; the search bar and its accessory dropdown are
  projected field-by-field, which is fragile enough to need a test that fails by name when a new
  field is missed.

## What would change our mind

- If extensions turn out to render controlled values *without* echoing often enough that the
  "setting" case becomes the accidental default, the polarity is wrong: the wire should carry an
  explicit `set` marker and treat a bare value as an echo. That is a wire change, so it is worth
  watching for during Phase 4 rather than after.
- If a future tier runs extensions in-process with genuinely synchronous rendering, the counter is
  dead weight for that tier — but it costs a `u64` and an integer compare, so it would not be worth
  a second code path.

`Seq::next` saturates rather than wraps. A wrap would make an ancient echo compare as current, which
is the exact bug this exists to prevent; saturating freezes the counter, which degrades to the
un-counted behaviour instead. Reaching `u64::MAX` takes about 585 years of keystrokes at one per
nanosecond, so this is a statement about which failure is acceptable rather than a case anyone hits.
