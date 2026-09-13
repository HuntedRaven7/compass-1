# ADR-0011: The launcher window is its own command, not the engine's

**Status:** Accepted · **Date:** 2026-09-13 · Relates to: PLAN.md §6 Phase 1 and Phase 2,
issue #4, `compass-ui`, `vicinae::serve`

## Context

`vicinae serve` owns the process: it builds a multi-threaded Tokio runtime, indexes applications and
serves the IPC socket until a shutdown request. It answers `toggle`, `show` and `hide` by refusing
them, with an error that says the engine is headless and has no window yet.

`compass-ui` now has a launcher that can search, move a selection and launch. Something has to start
it, and there is a constraint that makes "call it from `serve`" not a wiring job:

- **Iced's event loop owns the thread it is started on**, and `winit` on Wayland requires that to be
  the process's *main* thread. It cannot be spawned onto a worker, and it cannot run inside
  `runtime.block_on(...)`, which is how every other subcommand is dispatched.
- So a headed `serve` means inverting the process: Iced on the main thread, the Tokio runtime and
  the IPC listener on a background thread, and IPC requests reaching the UI through a channel fed
  into an Iced `Subscription`. Shutdown, single-instance handling and the socket lifecycle all move
  with it.

That inversion is a real design, and it is not forced yet. Phase 1's gate is "opens a window, fuzzy-
matches installed apps, launches one, closes" — none of which needs a daemon. Phase 2's `toggle`
does, but `toggle` is only useful once a global hotkey can invoke it, and
[Spike A](./0010-corral-vm-tier.md) has just established that binding one requires user consent that
CI cannot currently give.

## Decision

`vicinae ui` opens the launcher, in the foreground, in its own process. It indexes and ranks
in-process, needs no running engine, and exits when something is launched or Escape is pressed.

`serve` stays headless. Its window commands keep refusing, and keep explaining why.

The check for a display happens in `vicinae`, before Iced is reached: with no `WAYLAND_DISPLAY` or
`DISPLAY`, `iced::run` does not return an error — `winit` panics inside it, and a user who ran the
launcher over ssh gets a backtrace naming winit's source file. We return an ordinary error pointing
at `vicinae doctor`, which diagnoses this properly.

## Consequences

- **Phase 1's gate becomes evaluable.** There is a window to open, a list to move and an application
  to launch, and the VM tier can drive all three.
- **`toggle`, `show` and `hide` remain unimplemented**, and their refusal message stays accurate.
  This is deferred, not forgotten, and this ADR is where the deferral is recorded.
- Two processes can index the same applications at once. Harmless today — the index is read-only and
  rebuilt per process — and a reason not to leave it this way once the daemon hosts the window.
- The Flatpak binary now links Iced and wgpu, so the packaged build is larger and slower to compile.

## Alternatives considered

**Invert `serve` now.** The likely eventual design, and rejected only on timing: it would be written
against a `toggle` path that cannot yet be exercised end to end, because the hotkey that makes
`toggle` worth having is blocked on portal consent. Building it now means designing the interesting
half — what happens when a hotkey arrives while the window is already open — with no way to run it.

**Two processes permanently,** with the UI as a client of the engine over IPC. Clean separation, and
it survives the thread-ownership problem by ignoring it. Rejected because it doubles the moving
parts for a launcher whose entire job is to appear in under 100 ms, and because the engine already
holds the index the UI needs.

## What would change our mind

The moment a global hotkey can be bound and delivered — whether by pre-seeding the portal grant, by
the GNOME Shell extension, or on a wlroots compositor via `xx-hotkey-v1` — a daemon-hosted window
stops being optional, and the inversion above is the design to implement. This ADR should be
superseded then rather than quietly outgrown.
