# ADR-0023: The launcher starts on demand, not from a service

**Status:** Accepted · **Date:** 2026-11-18 · Amends: ADR-0015, ADR-0022 · Decides: the Bluefin PR's question — can Compass work with no service running?

## Context

Bluefin's packaging PR included a `compass.service` user unit, and its maintainer asked whether the
launcher could work without a service on the machine. Two ways already existed:

- `compass start` starts its own engine on demand (`session::ensure`) and runs a resident window
  under a supervisor, so the app-grid entry and a bare `compass` need no service.
- Login start asks the Background portal to run `compass start --hidden`, with an XDG autostart
  entry as the fallback (ADR-0022 §2). The systemd unit is a packager option, never enabled by
  the app.

What was missing was the cold path. Every other command is a pure IPC client, and with no engine
listening `compass toggle` — the command Bluefin binds to Super+Space — fails with "no Compass
engine is listening". Without either the service or an enabled login-start switch, the first press
of the hotkey on a fresh machine does nothing useful. ADR-0015 deliberately left open *what starts
the resident window*: autostart, the daemon, or the user. This is the decision it left open.

## Decision

The commands a person binds to a key start what they need, and let it outlive the short-lived CLI
process that asked for it:

1. **`toggle` and `show`** check whether an engine answers a ping. If none does, they spawn a
   detached `compass start` — engine plus a *visible* resident window — in a new process group
   with null stdio, and wait until the engine's own `DescribeWindow` reports the window shown.
   The window is already up, so these commands return without sending a second request.
2. **`query`** does the same check. With a graphical session it spawns `compass start --hidden`
   (engine plus a hidden window, ready for the next summon); with none it spawns
   `compass serve --no-hotkey` (engine only, so a diagnostic command never triggers a hotkey
   consent prompt). It waits for the engine to answer before querying.
3. **`hide` and `ping`** keep the honest refusal: there is nothing to hide or ping when cold.
4. An engine that *is* listening keeps the ordinary path, including ADR-0015's typed
   "no launcher window is connected" refusal. The on-demand path does not paper over a broken
   window; it only covers the case where nothing was started.
5. **`packaging/systemd/compass.service` stays**, as an optional fallback for packagers who
   prefer it (ADR-0022 §2). Nothing enables it, and nothing requires it. Login start stays off
   by default; on-demand start makes it an optimisation rather than a prerequisite.

The implementation is one module, `crate::activation`. There is no new IPC request, no protocol
bump, no D-Bus well-known name and no socket activation. The existing UI lease
(`<socket>.ui.lock`) makes two simultaneous cold starts safe: one wins the lease and the other
shows the window that the first started. `process_group(0)` is a safe API, so the workspace's
`unsafe_code = "forbid"` stands.

## Consequences

- A fresh install works from the first key press: no service, no login-start switch and no
  first-run consent to *start* the launcher. The global shortcut itself still needs its portal
  grant, or Bluefin's GNOME-level custom shortcut calling `compass toggle`.
- The cold start's latency is paid once, on the first summon. Subsequent shows keep the resident
  window's latency; ADR-0015's 120 ms figure was never a cold-start figure and remains one.
- `DescribeWindow` cannot tell "attached but hidden" from "not attached". Spawning a *visible*
  `start` and waiting for `open: true` sidesteps the ambiguity without a new protocol field; a
  hidden window from login start is shown by the same `start` through the UI lease.
- A detached engine started for a headless `query` has no supervising process. It lives until
  `compass shutdown` or logout, which is the ordinary lifetime of a `serve` engine; a later
  `start` reuses it rather than owning it.
- Tests changed accordingly: window-summon commands now send a `Ping` readiness probe first,
  the no-daemon test covers `hide` and `ping`, and new integration tests prove a cold `query`
  starts an engine that survives the query, is reused by the next client and stops on shutdown.

## Alternatives considered

- **Require the systemd service.** Rejected: it is what Bluefin asked to avoid, it needs a
  `graphical-session.target` with a Wayland environment, and it is not present on every target.
- **Turn login autostart on by default.** Rejected: ADR-0022 chose default off, the portal asks
  the user's permission, and neither helps before the first login after installation.
- **D-Bus activation of an `org.tunaos.compass` name.** Rejected: it needs an activation file,
  a well-known name and a way to hand the graphical session environment to the activated
  process. The Flatpak's `--own-name=org.tunaos.compass.*` grant is not backed by a name owner,
  and the Background portal explicitly sets `set_dbus_activatable(false)`.
- **A new `WindowPresence`/wait-for-attach request.** Rejected: a protocol version bump and a new
  long-poll in the engine for a question `DescribeWindow` already answers well enough once the
  session we start is visible.
- **Socket activation.** Rejected for the same environment problem as D-Bus activation: a socket
  activated by a manager has no `WAYLAND_DISPLAY` to open a launcher on.

## Reopen when

A maintained activation mechanism supplies the graphical session environment reliably on every
target, or the first summon has to meet the 120 ms SLA — then a resident-at-login model is needed
and this on-demand path should become the fallback instead of the primary route.
