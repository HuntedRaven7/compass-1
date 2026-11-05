# ADR-0022: The November calls, decided — accessibility, login start, browser transport, zbus

**Status:** Accepted · **Date:** 2026-11-05 · Amends: ADR-0008, ADR-0016, ADR-0018 · Decides: #302, #312, #315, #326, #309

## Context

Several issues were stalled on a choice only the owner could make. The owner delegated them ("make
the other decisions on my behalf"), as with ADR-0018. This records each choice and what would
reopen it.

## Decisions

### 1. The accessibility tree comes from AccessKit's Unix adapter, fed by our own model (#302)

ADR-0018 set the order of evaluation: upstream Iced, then the `plushie-iced` fork, then our own
AT-SPI. Taking each in turn:

- **Upstream Iced** has no tree yet (iced-rs/iced#552 is open). Waiting leaves screen-reader users
  without a launcher indefinitely, and the C++ fallback ADR-0018 relied on is gone (ADR-0021).
- **`plushie-iced`** would replace the `iced` the whole UI is built on. `iced_layershell` is pinned
  to upstream `iced` 0.14, and the launcher's surface depends on it through the unsafe bridge
  (ADR-0019), so adopting the fork means forking that too. That is a second UI toolkit to maintain
  for one feature.
- **Hand-written AT-SPI** over zbus is a protocol implementation, which the crates-first rule
  forbids when a maintained crate exists.

The choice is the maintained crate in the third route: **`accesskit` plus `accesskit_unix`**. The
Unix adapter talks AT-SPI on the session bus and does not need a toolkit or a window handle. So
`compass-ui` describes the launcher as an `accesskit::TreeUpdate`: the query field, the result list
with its selection, the action panel, form fields. A Linux-bound crate owns the adapter and gets
updates whenever the view model changes. The tree is built from application state, not from Iced
widgets, so it is testable without a display. #303 and #304 add roles to that model, and #305
retires the `a11y_gap` sentinel once Orca reads it.

**Reopen when** Iced ships AccessKit upstream. At that point the adapter crate is deleted and the
model feeds Iced's tree instead.

### 2. Login start uses the Background portal, with XDG autostart as the fallback (#312)

- **Every package asks `org.freedesktop.portal.Background` `RequestBackground`** with `autostart`
  and the command line `compass start --hidden`, through `ashpd` (already a dependency). On GNOME
  and KDE the portal serves unsandboxed apps too, so one code path covers the Flatpak and native
  packages, and the desktop shows the user what it granted.
- **Only an unsandboxed install with no portal backend** (some wlroots sessions) writes
  `$XDG_CONFIG_HOME/autostart/compass.desktop` itself, and removes only that file when disabled.
- **`packaging/systemd/compass.service` stays a packager option.** The UI never enables or
  disables it, and it is reported if found, so the user is not given two competing switches.
- **Default off.** First run asks, Settings shows the state the backend reports, and a refusal or
  an unavailable portal is shown as such, never as "on".

### 3. Browser control uses a native messaging host shipped with the launcher (#315, amends ADR-0008)

Option 1 of ADR-0008. A small `compass-browser-host` binary relays native messaging to the
engine's existing IPC socket, and the tab-search extension talks to the engine, not the browser.

- **Option 3 (a localhost socket or WebSocket) is rejected.** Every page in the browser can reach
  it, so it needs a pairing token, its storage, and a pairing flow. The native messaging channel is
  opened only by the browser, for the extension the host manifest names.
- **Option 2 (the extension ships its own host) is rejected** for the reason ADR-0008 gave: a
  sandboxed launcher writing manifests into an extension directory is the filesystem grant
  ADR-0004 declined.
- **Flatpak browsers** are supported where the browser exposes native messaging to host binaries
  (Firefox through the WebExtensions portal). Flatpak Chromium-based browsers are out of scope until
  they do. #318 records the matrix.
- #315's prototype (connect, list, focus round trip) and threat model still gate #316–#318; this
  decision fixes which prototype to build.

### 4. The zvariant advisories stay ignored until our zbus 5 pins move (#326)

`deny.toml` ignores RUSTSEC-2026-0370/0371/0372 (#328) because `ashpd`, `oo7`, `ksni`, `mundy`,
`notify-rust` and `system-tray` all pin zbus 5. Unmaintained is not vulnerable, and forking six
crates to move early is the larger risk. #326 is re-scoped to the migration: when every one of
those publishes a zbus 6 release, move the workspace and delete the three ignores in the same
change.

### 5. Performance baselines run on GitHub-hosted runners, not Depot (#309)

Recorded in PLAN §8.8. Baselines are keyed on the runner label and the tolerance is derived from
repeated runs (#306), not bought with quieter hardware.

## Consequences

- #302 is unblocked; #303–#305 follow it unchanged. `accesskit`/`accesskit_unix` join the workspace,
  and the crate that owns the adapter goes on the seam test's `MAY_BE_LINUX_BOUND` list. Done: the
  adapter is `compass-a11y-linux`, the seam is `compass_platform::AccessibilityTree`, and the model
  is `compass_ui::semantics`; ADR-0016's "Where it stands" lists what is described and what #305
  still has to show. `accesskit_unix` runs with its `tokio` feature, because the workspace's zbus
  has `tokio` and that takes over zbus's executor in every crate that shares it.
- #312 is unblocked; #314's install-journey test exercises the portal path.
- #315 can build its prototype; #316–#318 still wait for it.
- #326 stays open as a migration tracker, not a CI failure.
