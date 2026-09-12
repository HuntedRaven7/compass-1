# ADR-0006: Reconstruct fzf's coherence signal over nucleo's match indices

**Status:** Accepted, and the outcome was better than the decision assumed — see *Outcome* · **Date:** 2026-09-12 · Relates to: PARITY.md, PLAN.md §8.3

## Context

Porting `src/lib/fuzzy` onto `nucleo-matcher` reproduced the C++ behaviour closely — every
normalized `score`/`quality` assertion in the C++ suite ports verbatim — with one significant gap.

The C++ matcher computes a `coherent` flag inside its backtracking pass, from the DP matrix's
per-position boundary bonuses, and forces `quality = 0` for incoherent matches. nucleo exposes
neither the matrix nor an equivalent. Consequence: C++ rejects `"time"` against
*"Play this game on Steam"* and *"Start Input Method"*; we accepted them at quality 69 and 73
against a gate of 60.

That is a user-visible false-positive source — a launcher that surfaces nonsense for short queries
is a launcher people stop trusting — and it blocked deleting `src/lib/fuzzy`.

Three options: layer a classifier over nucleo's indices; raise `MIN_QUALITY` globally; accept the
looser matching as a product decision.

## Decision

**Layer a coherence classifier over nucleo's match indices.**

## Why not the alternatives

- **Raising `MIN_QUALITY`** trades false positives for false negatives globally. The failing cases
  score 69–73 while legitimate matches live in the same band, so any threshold that rejects
  *"time"* against *"Play this game on Steam"* also rejects real matches. It treats a
  shape problem as a magnitude problem.
- **Accepting looser matching** silently changes behaviour every existing user relies on, in the
  direction of "the launcher got worse". Not a decision to make by default.

## What we expected this to cost

At the time of deciding: "we are approximating an algorithm we cannot call, from less information
than it had — nucleo gives us matched indices, the haystack and the needle, not the DP matrix. The
classifier is a heuristic with tuned parameters that will need re-tuning."

## Outcome: it is an exact port, not a heuristic

That premise was wrong, and the correction is the more useful part of this record.

Reading `fzf.hpp` phase 4 closely: the backtracker only ever compares `B[j]` against **zero**
(`B[j] > 0`, `B[j] == 0`). It never reads the bonus magnitude, and it uses `H`/`C` only to choose
which cell to step to — that is, only to pick the *alignment*. And `B[j]` is itself a function of two
adjacent characters. So

```
coherent = !boundary_inside || !mid_word_run_start
```

is a pure function of `(haystack, alignment)`, and the alignment is exactly what nucleo hands back.
**There are no thresholds, ratios or run-count limits to tune, because none exist in the original.**

Validated against an oracle rather than by argument: the real C++ matcher was built and used to
compare flags directly. Feeding the C++ matcher's own positions into the Rust rule gives **0
mismatches across all 48 corpus cases and 3,537 random pairs** — the classifier is exact.

One trap worth recording: `ascii_fuzzy_index` starts the bonus window one character *before* the
needle's first occurrence, so every inspected position sees its true predecessor. Misreading that
window as starting *at* the first occurrence makes `B[first]` spuriously non-zero and yields wrong
answers.

## What actually remains, one level up

Coherence is a property *of an alignment*, and nucleo's DP does not always choose fzf's. Alignments
differ in ~10.6% of cases; end-to-end flag agreement is 99.57% (3032/3045), and 100% across the
ported corpus and every ranking case. Closing that means replacing nucleo's DP, not improving the
classifier — a different and much larger decision, and not one worth making for a gap with no
observed instance.

## Consequences

- There is nothing to re-tune. If behaviour must change, it changes in `is_boundary`, and the oracle
  harness (~20 lines against `src/lib/fuzzy/include`) re-validates it.
- **A recall cost arrives with parity**, inherited from the C++ rule rather than introduced:
  cross-word abbreviations that pick up a mid-word letter are rejected — `"ffb"` against
  *"Firefox Web Browser"*, `"txted"` against *"Text Editor"*. The oracle confirms C++ rejects them
  too. Whether that is the *right* rule is a product question we have now inherited rather than
  answered.
- The blocker on deleting `src/lib/fuzzy` is lifted, apart from the alignment gap above.

## Lesson

The plan asserted the signal was unreconstructible without checking. One careful read of 45 lines of
C++ turned a heuristic-with-tuning into an exact port. Read the source before designing around a
limitation.
