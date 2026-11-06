# Flatpak packaging

Bluefin is the first target and has no traditional package manager, so the Flatpak is the
**development** target from Phase 0 rather than a packaging step at the end. See
[`../../docs/rust-engine/PLAN.md`](../../docs/rust-engine/PLAN.md) §3.6.

## Status

The manifest is built on pull requests and the bundle is exercised in the Bluefin VM tier. That
tier verifies the Wayland session, portals, sandbox nesting and that the Rust launcher paints. The
nightly VM remains the authoritative target integration check; syntax validation alone is not
treated as packaging evidence.

## Building

After installation, open **Compass** from the application grid. Its `start`
entrypoint reuses a running launcher or starts an engine and a resident window
together. On first run, the "Make it your own" step asks for the native
global-shortcut permission when you choose Set Up Shortcut (or when the setup
is finished or closed), and says whether the desktop granted it, left the key
unassigned because something else has it, or was declined, with what to do
next. Escape hides the launcher so the granted shortcut can bring it back.

Start at login is off until chosen, in the same step or in Settings, General.
Turning it on asks the Background portal for autostart with
`compass start --hidden` (ADR-0022 §2); the Flatpak never writes an autostart
file itself, so without a Background portal the switch reports that it is
unavailable. Closing the graphical session stops an engine it started itself,
but does not stop an independently managed engine. The development `ui` and
`serve` commands remain available. An app-grid session exits if its engine
disconnects, so a subsequent app-grid activation can start a fresh session
instead of finding an undriven window. The explicit `ui` command retains its
previous keep-running behavior.

`compass start --hidden` prepares a resident session without opening a window;
it is the command the desktop runs at login. A duplicate hidden start leaves
the existing launcher's visibility unchanged, so a login start and a packaged
unit never make two launchers. Ordinary `start` still opens it.

The native color mode is also the default. Leave
`launcher.appearance.color_scheme` absent (or set it to `"system"`) to follow
the desktop's light/dark preference, including changes while Compass is
running. Set it to `"light"` or `"dark"` for an explicit override; clearing the
key restores System. If the Settings portal is unavailable, System falls back
to the readable Adwaita light palette.

```sh
# One-off: the offline dependency manifest Flathub builds require.
python3 flatpak-cargo-generator.py ../../Cargo.lock -o cargo-sources.json

flatpak-builder --user --install --force-clean build org.tunaos.compass.yaml
flatpak run org.tunaos.compass -- doctor --check-only
```

`flatpak-cargo-generator.py` comes from
[flatpak-builder-tools](https://github.com/flatpak/flatpak-builder-tools). It is why `Cargo.lock`
must stay committed: Flathub builds have no network access, so every dependency is declared up
front.

## Updating and uninstalling

`flatpak update` never stops a running Compass: it keeps the old version until it exits.
`flatpak run org.tunaos.compass shutdown`, then opening Compass again, switches to the new one.
`flatpak uninstall org.tunaos.compass` keeps the user's settings and data in
`~/.var/app/org.tunaos.compass`. To delete them, run
`flatpak run org.tunaos.compass data remove --yes` before uninstalling, or uninstall with
`--delete-data`. See "Upgrades and removal" in [`../README.md`](../README.md).

## The store icon

`icons/{64x64,128x128}/org.tunaos.compass.png` are the PNGs `appstreamcli compose` renders from
the SVG during the build. The TunaOS remote is an OCI remote, and Bazaar cannot find a `cached`
icon on one, so the publish workflow adds `<icon type="remote">` URLs that point at these files.
`scripts/flatpak/appstream-icons.py` explains this in more detail. The Flatpak workflow fails when
the files stop matching the build, and its error says how to update them.

## Why each permission exists

Flathub review asks for this, and "we needed it" is not a reason a reviewer can check. Do not add a
`finish-arg` without adding its justification here.

| Permission | Why | Could we drop it? |
|---|---|---|
| `--socket=wayland` | The engine draws a Wayland surface. | No. |
| `--device=dri` | Iced renders through wgpu. Without GPU access it falls back to software rendering, if it starts at all. | No. |
| `--own-name=org.tunaos.compass.*` | Our own bus names: `org.tunaos.compass` for single-instance and CLI activation, and the services under it such as `org.tunaos.compass.WindowTracker`. Flatpak already allows an app its own ID and subnames; the grant is explicit so review sees it. | No. |
| `--talk-name=org.freedesktop.portal.Desktop` | GlobalShortcuts (the hotkey mechanism on GNOME), OpenURI, FileChooser, Screenshot. | No. |
| `--share=network` | Extensions fetch their data, views load remote images, OAuth exchanges its code, and the stores download bundles. | No. |
| `--talk-name=org.gnome.Shell` | On GNOME 50/51 the Shell extension (`compass@tunaos.org`) is the **only** mechanism for window switching, clipboard history and paste — Mutter implements none of the relevant protocols and `org.gnome.Shell.Introspect` is allowlisted to the portal backends. See [REFERENCES.md §3](../../docs/rust-engine/REFERENCES.md). A talk-name filters bus names, and the extension's `org.tunaos.compass.Shell.*` interfaces live on the Shell's own connection, so the grant has to name `org.gnome.Shell`. | Only if GNOME ships a portal for window listing. |
| `--talk-name=org.a11y.Bus` | Screen readers. Flatpak proxies the accessibility bus into the sandbox by itself, but not `org.a11y.Bus` on the session bus, whose `org.a11y.Status.IsEnabled` is how the launcher's accessibility tree (`compass-a11y-linux`) learns that Orca is running. Without it Orca reads nothing; the Orca gate (`scripts/tier2/orca.sh`) found that, and runs the bundle without the grant as its control. | No. |
| `--talk-name=org.kde.StatusNotifierWatcher` | Compass's tray icon registers with the desktop's watcher, and Search Tray asks it for other applications' icons. | Only by dropping the tray. |
| `--system-talk-name=org.freedesktop.login1`, `--talk-name=org.gnome.SessionManager`, `--talk-name=org.kde.Shutdown` | Power off, reboot, suspend, lock and log out. Every action is still subject to logind's polkit policy. | Only by dropping the power commands. |
| `--talk-name=org.freedesktop.Flatpak` | `flatpak-spawn --host`, to launch host applications, and to run the host programs a Rhai script with `process.execute` asks for (such as `systemctl` for the System Health script). | No — but see below. |
| `--filesystem=host-os:ro` and the `applications`/`icons` paths | Indexing the application catalogue: what exists, what it is called, what its icon is. Read-only; we never write to any of them. | No, though the list could be trimmed if we dropped Homebrew or system Flatpak discovery. |
| `--filesystem=xdg-data/flatpak/app:ro`, `--filesystem=/var/lib/flatpak/app:ro` | The Flatpak exports directories hold symlinks into these deploy trees; without them every Flatpak application disappears from search (#105). Read-only. | No. |
| `--filesystem=home:ro` | Search Files indexes and watches the directories in its search paths, the home directory by default. Read-only; the index lives in the sandbox's cache. | Only by narrowing the default search paths. |

### The one that deserves scrutiny

`--talk-name=org.freedesktop.Flatpak` grants `flatpak-spawn --host`, which is **equivalent to running
outside the sandbox**. A launcher that cannot launch host applications is not a launcher, so this is
unavoidable for the category — but it should be stated plainly in review rather than buried in a
list, and it is worth remembering that the Flatpak boundary provides us much less isolation than the
manifest's other entries suggest.

It also means the extension-worker sandbox in Phase 4 (Landlock + seccomp) is doing real work: it is
not redundant with the Flatpak sandbox, because the Flatpak sandbox is this porous by necessity.

### Deliberately absent

- **`--socket=fallback-x11`.** The engine is Wayland-only by design. Claiming an X11 socket we never
  use is asking for a permission we do not need. (The original spec had it; we do not.)
- **`--socket=pulseaudio`.** The spec included it for "media extensions". We have no such feature
  yet. Add it when something needs it, not before.
- **`--filesystem=~/.local/share/gnome-shell/extensions`.** Deliberately refused — see
  [ADR-0004](../../docs/rust-engine/adr/0004-gnome-shell-extension-distribution.md). We never install
  the Shell extension ourselves; the user installs it from extensions.gnome.org, or it arrives baked
  into the Bluefin image.
- **`org.kde.Platform`.** The original spec specified the KDE runtime, which made sense for a Qt
  application. The Rust engine needs neither Qt nor GTK at runtime, so `org.freedesktop.Platform` is
  smaller, faster to download, and a smaller attack surface.
