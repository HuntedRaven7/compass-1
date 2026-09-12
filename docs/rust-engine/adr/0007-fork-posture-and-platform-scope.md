# ADR-0007: Hard fork in practice, `vicinae` names kept, Linux-first

**Status:** Accepted · **Date:** 2026-09-12 · Relates to: PLAN.md §10.1, §10.2, §10.3, §10.8

## Context

Four related questions were left open and were blocking naming and scope decisions: our relationship
to upstream `vicinaehq/vicinae`; whether to rebrand from `vicinae` to `compass`; whether macOS and
Windows follow the Rust engine; and whether to support GNOME 50 as well as 51.

They are answered together because they are one question: what is this project, and for whom.

## Decisions

**1. Hard fork, acknowledged rather than drifted into.** Rewriting the core in Rust makes merging
upstream C++ changes impossible after Phase 1. Say so, rather than maintaining the fiction of a
tracking fork and discovering it six months in. Upstream remains valuable as a *behavioural*
reference and a source of bug reports worth porting by hand — the six C++ bugs found while porting
the desktop-entry parser are worth reporting upstream, and that courtesy should continue.

**2. Keep the `vicinae` binary, socket, config and DBus names.** Crates are `compass-*`; the things
users and their configs touch stay `vicinae`. Renaming buys nothing and breaks every existing
config, script command, `dmenu` invocation and shell alias. The repository and the crates can carry
the fork's name without dragging the user-visible surface along.

**3. Linux-first; macOS and Windows keep the C++ engine.** The Rust engine targets Linux, and
GNOME/Bluefin specifically. macOS and Windows continue to ship the C++ build until a follow-up
project. This is a real narrowing and must be stated in release notes at cutover rather than
discovered by users — compass supports all three today.

**4. Support GNOME 50 and 51 both, with both in CI.** GNOME 51 ships 16 September 2026 and Bluefin
users will straddle the two for months. Testing only the newer one guarantees we ship breakage to
everyone who has not rebased their image yet.

## Consequences

- Being a hard fork means we own our own bugs. No upstream to inherit fixes from.
- Keeping `vicinae` names means a user can switch engines with `--engine=rust` and keep everything
  else — which is exactly what the migration strategy depends on.
- Two GNOME versions in the nightly matrix, and an extension-compat task every GNOME cycle.
- The macOS and Windows C++ code stays alive in-tree well past Phase 8, so Phase 8's deletion is
  Linux targets only.
