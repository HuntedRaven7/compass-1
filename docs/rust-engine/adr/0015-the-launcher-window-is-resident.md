# ADR-0015: The launcher window is resident, and `serve` summons it

**Status:** Accepted · **Date:** 2026-09-17 · Amends: ADR-0011 · Relates to: PLAN.md §6 Phase 3, §8 SLA

## Context

PLAN.md §12 names the next thing that matters: `vicinae serve` answers `toggle`, `show` and `hide`
with *"this engine is headless … it has no window yet"*. The window itself exists — `vicinae ui`
opens it, it draws in a real GNOME session on Bluefin, it searches and launches — but nothing can
summon it. That is the difference between a demo and a launcher.

[ADR-0011](./0011-the-window-is-its-own-command.md) made `vicinae ui` **one-shot**: foreground, its
own process, indexing in-process, *"exits when something is launched or Escape is pressed"*. That
was right for proving the renderer. It cannot be how Super+Space works.

**The SLA decides this, and it decides it before any design discussion.** PLAN.md §8 sets *cold
start to first frame < 120 ms* — "the number users feel". A design that starts a process per
summon must fit, inside that budget: process spawn, dynamic linking, Iced and winit
initialisation, wgpu enumerating and bringing up an adapter, surface creation, and the first frame.

The VM tier times the first two of those separately for exactly this reason, and records wgpu
reaching `Adapter AdapterInfo` **2.4 s** into one run and not yet at **8.1 s** into another. Those
are llvmpipe-under-QEMU numbers and real hardware is far quicker — but the gap to 120 ms is two
orders of magnitude, not a tuning problem. **Spawning per summon is not slow, it is impossible.**

So the window must already be running when the shortcut fires. The only real question is where it
lives.

## Options

**A — spawn `vicinae ui` per summon.** Simplest, no protocol change, and the one the code is
shaped for today. Rejected on the SLA above.

**B — a resident window process that `serve` signals.** `serve` keeps the engine, the socket and
its headless capability; a long-lived `vicinae ui` connects to it and is told when to show. Costs a
direction the protocol does not have: the server must push to a client rather than only answer it.

**C — one process: Iced on the main thread, the IPC server on a worker.** Fewest moving parts.
Rejected because it makes `serve` graphical: `vicinae serve` on a machine with no display is a real
use — `query` and `doctor` work there today, and the VM tier depends on it. Option C would make the
daemon refuse to start without a compositor, which trades a missing feature for a lost one.

## Decision

**Option B.** Three parts:

1. **`vicinae ui` becomes resident.** It no longer exits when something is launched; it hides. It
   still exits on an explicit quit. This amends ADR-0011's "exits when something is launched or
   Escape is pressed" — that sentence described a one-shot prover, and the prover worked.
2. **`serve` gains a push direction** so it can tell a connected window to show, hide or toggle.
   The request/response protocol stays exactly as it is for everything else; this is an addition,
   not a redesign.
3. **The window connects to `serve`, not the other way round.** A client dialling a known socket is
   ordinary; a daemon dialling back into a process it did not start is not. It also means the
   window can be started by anything — autostart, the user, a test — without `serve` knowing how.

**`serve` stays headless.** It holds at most one window client. With none connected it still
refuses `toggle`, `show` and `hide` — and that refusal gets *more* accurate rather than being
deleted: "no launcher window is connected" is a different and more useful thing to be told than
"this engine is headless", and a client can still tell it from "the window was shown".

## Consequences

**The honest-refusal property is preserved, which is the point.** `vicinae` has been careful that a
client can distinguish "no window yet" from "the window was shown" — that distinction is the only
thing between an honest gap and a `toggle` that silently does nothing. The resident design keeps
it: the refusal now names a connection rather than a build.

**Two processes must agree on lifetime.** If the window dies, `serve` must notice and go back to
refusing rather than reporting success into a closed socket. If `serve` dies, the window should not
wedge. Neither is hard, both are easy to get silently wrong, and both need a test that kills one
side.

**The 120 ms SLA becomes measurable for the first time.** With the window resident, "cold start to
first frame" is no longer the thing users feel on Super+Space — *show latency* is, and that is a
different measurement the VM tier does not yet take. The SLA row should split, and this ADR does
not do that: it is PLAN.md §8's to revise once there is something to measure.

**What this does not settle.** Whether the resident window is started by autostart, by `serve`
spawning it when a display exists, or by the user, is deliberately open. The protocol above works
for all three, and picking one before the shortcut path works would be guessing at a problem that
is not yet in front of us. [ADR-0023](./0023-the-launcher-starts-on-demand.md) later settled it:
the summoning commands start the session on demand, which needs neither a service nor autostart.
