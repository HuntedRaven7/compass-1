# ADR-0008: Browser control is an extension, not part of the port

**Status:** Accepted · **Date:** 2026-09-12 · Relates to: PLAN.md §6 Phase 5, PARITY.md

## Context

Compass ships browser tab search and switching as a builtin. It is four pieces:

| Piece | Where | What it is |
|---|---|---|
| WebExtension | `src/browser-extension/chrome/`, `manifest.firefox.json` | JS running inside Chrome/Firefox |
| Native messaging host | `src/browser-extension/src/browser.cpp` | a binary the *browser* spawns, bridging native messaging to the daemon's IPC socket |
| Host manifest installer | `src/server/src/services/browser-extension/` | writes per-user native-host manifests at startup for every detected browser |
| The launcher-side feature | `src/server/src/builtins/browser/` | tab list, model, view host |

748 lines of C++ plus the WebExtension. The original plan had all of it in Phase 5 Track A.

## Decision

**Browser control is out of scope for the Rust port. It becomes an extension.**

Nothing in the Rust engine implements tab search, tab switching, the native messaging host, or
native-host manifest installation. `compass-ipc` does not carry `BrowserInit`,
`BrowserTabsChanged` or `FocusTab`; their absence is now permanent rather than deferred.

## Why

- It is a **browser** feature that happens to be surfaced in a launcher. Nothing about it needs to
  live in the core: it has no coupling to the window manager, the compositor, the clipboard or the
  index, and it is the clearest possible case of something the extension API exists to serve.
- Carrying it as a builtin means the core owns a browser-vendor integration whose churn we do not
  control — manifest v2 → v3, per-browser manifest layouts, store review, extension IDs.
- Phase 5 is already the widest phase in the plan. Removing a subsystem with its own release
  cadence, its own store, and its own signing story is straightforwardly good for it.
- It exercises the `compass-extension-api` seam (ADR-0005) on a real feature rather than a toy,
  which is the best possible pressure test for that boundary.

## The constraint whoever builds this must know about

**Native messaging cannot simply move into a TypeScript extension**, and pretending otherwise would
set someone up to fail. A browser spawns a native host **binary by absolute path**, named in a
manifest it reads from a fixed per-user location. A Raycast-compatible extension is a TS bundle
spawned *by the launcher*; it has no stable path a browser manifest could point at, and on our first
target it lives inside a Flatpak sandbox.

So one of these has to be true:

1. **A small native host binary ships with the launcher** and relays to whatever is listening —
   the extension then talks to the launcher, not to the browser. Least invasive, but the core still
   ships a browser-shaped component, which only partly honours this decision.
2. **The extension ships its own host binary and registers it.** Cleanest conceptually; hardest in
   practice, since a sandboxed launcher writing a manifest that points into an extension directory
   is exactly the kind of filesystem grant ADR-0004 declined for the GNOME Shell extension.
3. **Drop native messaging** for a local socket or WebSocket the WebExtension connects to. Removes
   the host entirely, but opens a locally reachable endpoint, so it needs real authentication — any
   page in the browser can reach localhost.

This ADR does not choose between them. It records that the choice exists, that it is the actual
design problem, and that it belongs to whoever picks the feature up.

## Consequences

- Three rows leave the parity ledger as **out of scope** rather than pending: `src/browser-extension`,
  `src/services/browser-extension`, `src/builtins/browser`.
- The C++ implementation keeps working for C++-engine users until cutover, and is not deleted by the
  Rust port. If no extension exists by Phase 7, browser tab switching is a **feature regression at
  cutover** and must be called out in the release notes alongside the macOS/Windows narrowing
  (ADR-0007). That is a real cost of this decision, not a free simplification.
- The existing WebExtension and its native host remain useful reference material for whoever writes
  the replacement; they are not dead code to be deleted early.
