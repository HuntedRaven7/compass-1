# ADR-0005: Build the extension-API seam now; decide the Rhai tier later

**Status:** Accepted · **Date:** 2026-09-12 · Relates to: PLAN.md §2.2, §10.9

## Context

A third extension tier was proposed: [Rhai](https://rhai.rs) scripts, sitting between one-shot
script commands and full Raycast TypeScript extensions. Technically it fits well — pure Rust, no
runtime dependency, and a capability-based sandbox that is structurally stronger than the Node
worker's, because Rhai's standard library has no I/O at all unless the host registers it.

The counterweight is that the Rhai launcher-extension ecosystem is precisely zero, and a third
extension API is a permanent documentation, support and example-maintenance burden.

## Decision

**Split the decision in two, because the two halves have very different risk.**

1. **Build `compass-extension-api` now**, in Phase 4, before the Node host is written against it:
   capability registry, view tree, action dispatch, knowing nothing about Node, JSON-RPC or Rhai.
   Proven mechanically by a gate — the crate must build and pass its tests with
   `compass-worker-host` removed from the dependency graph.
2. **Defer the Rhai tier itself** to a go/no-go at the end of Phase 4.

## Why

The seam is worth building on its own merits: it is roughly three days, and without it every host
capability — clipboard read, window list, storage, OAuth — gets exposed once per tier and drifts.
That cost lands whether or not Rhai ever ships.

The tier is a product bet, not an engineering one, and product bets should be made by someone who
owns the product, with evidence. Deferring it costs nothing once the seam exists; making it now
costs a permanent maintenance obligation decided on a hunch.

## Consequences

- If the tier is later cancelled, we have lost nothing: the seam still pays for itself.
- If the seam does not materialise — if Phase 4 pressure collapses it back into the Node host —
  then **drop the Rhai tier outright** rather than maintain two parallel stacks.
- Shipping the tier is gated on four first-party example scripts good enough to copy from. An empty
  tier is worse than no tier.
