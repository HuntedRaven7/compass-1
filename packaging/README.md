# Packaging

Every output of the Rust engine installs the same files in the same places. The list is
`scripts/packaging/install-rust-engine.sh`, and every output calls it; `scripts/packaging/smoke.sh`
checks it (PLAN.md §8.6, Suite 5).

| Path under the prefix | What |
|---|---|
| `bin/compass` | the engine and CLI |
| `libexec/compass/compass-sandbox-exec` (Arch: `lib/compass/`) | confines the extension runtime; extensions refuse to run without it |
| `libexec/compass/compass-file-indexer` (Arch: `lib/compass/`) | the file-search helper |
| `libexec/compass/compass-input-server` (Arch: `lib/compass/`) | snippet keyword expansion's keyboard helper; not in the Flatpak (see below) |
| `share/applications/org.tunaos.compass.desktop`, `share/metainfo/…`, `share/icons/hicolor/scalable/apps/…` | desktop entry, AppStream data, icon |
| `share/compass/builtin-icons/*.svg` | the `Icon.*` set extensions draw with |
| `lib/systemd/user/compass.service` | an opt-in user unit (`systemctl --user enable --now compass`); inert in the Flatpak, which does not export units |
| `share/compass/extension-runtime.js` | the extension runtime bundle (`scripts/build-extension-runtime.sh`) |
| `share/compass/compass.schema.json` | the `compass.json` JSON Schema |
| `share/compass/scripts/<name>/` | the first-party Rhai scripts (`extensions/rhai-examples/`), granted what they declare |

The engine finds the helpers, the runtime bundle, the schema and the scripts relative to its own binary, so a
prefix is relocatable. Node is the Flatpak's and the AppImage's own; the Arch and Nix packages use
the distribution's.

## Outputs

| Output | Definition | Built and smoke-tested by |
|---|---|---|
| Flatpak | `flatpak/org.tunaos.compass.yaml` (finish-args justified in `flatpak/README.md`) | `.github/workflows/flatpak.yaml`, on every PR touching it |
| AppImage | `appimage/build-appimage.sh` (linuxdeploy + appimagetool, pinned by checksum) | `.github/workflows/packaging.yaml` |
| Arch | `arch/PKGBUILD` (`compass-git`) | `packaging.yaml`, in an `archlinux:base-devel` container |
| Nix | `nix/compass.nix`, exposed by `flake.nix` as `.#compass` (alias `.#rust-vicinae`) | `packaging.yaml`, with `cachix/install-nix-action` |

`packaging.yaml` runs nightly and on changes to packaging; the Flatpak workflow runs on every PR
that touches the engine. Both end in the same `smoke.sh`: installed layout, `--version`,
`config schema`, `doctor` detecting the missing session, and a started engine answering `ping`.

The flake also exports two modules. `homeManagerModules.default` (`nix/home-manager-module.nix`)
is `programs.compass`: the package, `compass.json` settings, themes, extensions and a
`compass.service` user unit. `nixosModules.default` (`nix/nixos-module.nix`) is
`programs.compass.input-server`, described below. Both accept upstream's `programs.vicinae` names.

The distribution packages install `compass` and `org.tunaos.compass` files only, so they sit beside
upstream's `vicinae` packages rather than conflicting with them.

## The input server

`compass-input-server` (`crates/compass-input-server`) reads every keyboard under `/dev/input` to
notice a snippet keyword being typed, and creates a virtual keyboard through `/dev/uinput` to erase
the keyword and paste the expansion. It opens devices read-only and never grabs them. Both device
paths are root-only by default, so it needs `CAP_DAC_OVERRIDE` — the same model as upstream's
helper. The capability sits on this
one small program, which speaks only JSON-RPC to its parent over stdin and stdout; the launcher
itself runs unprivileged. Adding the user to the `input` group instead would hand every program
they run the keyboard, and still leave `/dev/uinput` closed on most distributions.

| Output | How the helper gets its capability |
|---|---|
| Arch | `arch/compass.install`: `setcap cap_dac_override+ep /usr/lib/compass/compass-input-server` after install and upgrade |
| Nix | the store cannot hold file capabilities. On NixOS, import the flake's `nixosModules.default`: `programs.compass.input-server.enable` (on by default) wraps it with `security.wrappers` and sets `COMPASS_INPUT_SERVER_BIN=/run/wrappers/bin/compass-input-server` for the session |
| AppImage | cannot carry capabilities from inside the image; run `sudo setcap cap_dac_override+ep` on an extracted copy and point `COMPASS_INPUT_SERVER_BIN` at it |
| Flatpak | **not shipped.** The sandbox has no `/dev/input` or `/dev/uinput` and no finish-arg grants them (`--device=all` exposes `/dev` nodes but the helper would still need a capability no Flatpak can hold). The engine does not start it there, and `compass doctor` says so; snippets are still copied and pasted from the launcher |
| From source | `cargo build -p compass-input-server`, then `sudo setcap cap_dac_override+ep target/debug/compass-input-server` |

`compass doctor` reports whether the helper is found, whether it carries the capability (via
`getcap`), and what the running engine says about it; `compass input-server status|enable|disable`
reads and flips `input_server.enabled`.

## Upgrades and removal

Compass runs as each logged-in user and never as root. A package therefore reaches a running engine
only through that user's own service manager or the engine's own IPC socket, as that user, so the
engine saves its state and exits the way `compass shutdown` makes it. No package signals a Compass
process, and no package touches anything under a home directory. Every output follows these rules:

- **An upgrade hands a running engine over to the new version, or says that it has not.** It never
  leaves two resident launchers: `compass start` holds a lease beside the IPC socket, so starting the
  new version while the old one still runs shows the old window rather than opening another.
- **Removal stops running engines and keeps every user's files.** The package's own files go
  (binaries, helpers, desktop entry, `compass.service`); nothing in any home directory does.
  Reinstalling picks everything up again.
- **Deleting a user's data is that user's explicit act**, with `compass data remove`.

### What each output does

| Output | Upgrade | Removal |
|---|---|---|
| Arch | `post_upgrade` in `arch/compass.install`, once the new files are in place: for each user whose `compass.service` is active, `systemctl --user --machine=<user>@.host daemon-reload` and `try-restart compass.service`, so their own manager restarts it on the new binary. A user running Compass any other way (desktop autostart, a terminal) is named in a message and keeps the old version until Compass next starts. | `pre_remove`: for each user, `stop compass.service` through their manager when it is active, and otherwise `runuser -u <user> -- compass shutdown` against `/run/user/<uid>/compass/ipc.sock`. A user whose engine does not stop is named; it stops when they log out. `post_remove` says where the data was kept. |
| Flatpak | Flatpak never stops a running app. `flatpak update` leaves the running Compass on the old version until it exits; `flatpak run org.tunaos.compass shutdown`, then opening Compass again, switches to the new one. | `flatpak uninstall org.tunaos.compass` keeps `~/.var/app/org.tunaos.compass`. `flatpak uninstall --delete-data org.tunaos.compass` deletes it, and with it every Compass file the sandbox could write. |
| Nix | Home Manager's `programs.compass.systemd` unit names the store path in `ExecStart`, so a new package is a changed unit, which Home Manager restarts when `systemd.user.startServices` is on. Otherwise run `systemctl --user restart compass`. | Removing the package or the module never touches a home directory. |
| Homebrew | `brew upgrade` replaces the files; run `brew services restart compass` if Compass runs as a service, or `compass shutdown` and start it again. | Run `brew services stop compass` before `brew uninstall compass`; neither touches a home directory. |
| AppImage | Replace the file, then `compass shutdown` and start the new one. | Delete the file. Nothing else was installed. |

There is no `.deb` or `.rpm` package yet. One must follow the Arch script: hand over in `postinst`
(`try-restart` through each user's manager, or a message), stop in `prerm`, and never delete a
user's files, not even on `purge`.

### What Compass keeps, and where

Under each XDG base directory Compass owns one `compass` directory. Inside the Flatpak, the bases
are under `~/.var/app/org.tunaos.compass`.

| Base | Path | What |
|---|---|---|
| `$XDG_CONFIG_HOME` | `~/.config/compass` | `compass.json`, themes, Rhai scripts |
| `$XDG_DATA_HOME` | `~/.local/share/compass` | the databases, extensions and clipboard history |
| `$XDG_CACHE_HOME` | `~/.cache/compass` | the file index, downloaded images, exchange rates |
| `$XDG_STATE_HOME` | `~/.local/state/compass` | the engine log |

Beside each there may be a `vicinae` symlink pointing at it, left by the move from Vicinae's names
(ADR-0020) so that an older build still finds the same data. A real `vicinae` directory belongs to
upstream Vicinae, which can be installed beside Compass. `$XDG_RUNTIME_DIR/compass` holds only a
running engine's socket and lock, and is emptied at logout. Secrets in the keyring use upstream's
labels, which Vicinae shares, so neither a package nor `compass data remove` deletes them.

### Deleting it: `compass data remove`

`compass data remove` lists every path above, says what would happen to each, and deletes nothing.
`compass data remove --yes` deletes:

- each `compass` directory, with everything in it;
- each `vicinae` symlink that points at the `compass` directory beside it.

It keeps:

- each real `vicinae` directory, and each `vicinae` link that points anywhere else, unless
  `--include-vicinae` is passed;
- whatever a symlink points to. A `compass` entry that is a link (into a dotfiles checkout, for
  example) is removed as a link, and so is any link inside the directories;
- everything else in the base directories.

It refuses while an engine answers on the socket, since the engine has these files open: quit it
with `compass shutdown` (or `systemctl --user stop compass`) first. A path that changed between the
listing and the deletion is left alone. Run it before uninstalling, since the command goes with the
package; after uninstalling, delete the four directories listed above by hand. In the Flatpak, run
`flatpak run org.tunaos.compass data remove --yes`, or uninstall with `--delete-data`.

If you enabled the unit (`systemctl --user enable compass`), run `systemctl --user disable compass`
before uninstalling, or the enablement link in `~/.config/systemd/user` is left pointing at nothing.

### Recovering

- **Two launchers, or one that ignores the new version.** `compass shutdown` stops the engine and the
  `compass start` that owns it. Then start Compass again from the application grid, or with
  `systemctl --user start compass`. `compass ping` reports the running engine's pid.
- **An engine that will not shut down.** `systemctl --user stop compass`, or end the `compass`
  process from a system monitor. The lease and socket are released when the process exits.
- **Data you meant to keep.** Uninstalling never deletes it; reinstalling finds it where it was.
  `compass data remove --yes` cannot be undone, which is why it lists everything first.

## System-wide defaults

A distribution sets defaults for every user in `compass/compass.json` under `$XDG_CONFIG_DIRS`,
usually `/etc/xdg/compass/compass.json`. It has the same format as the user's file, comments
allowed. The user's `~/.config/compass/compass.json` is layered over it key by key, so a setting
the user makes wins. When Compass writes the user's file, it leaves out values that came only from
the system file, so a later change to the system file still reaches existing users. With several
`$XDG_CONFIG_DIRS`, the directory listed first wins. A system file that does not parse is logged
and skipped. Compass reads it at start, so a change takes effect when the engine restarts.

Bluefin binds Super+Space with a GNOME custom keyboard shortcut that runs `compass toggle`,
because the GlobalShortcuts portal asks the user to approve a binding the first time. The image
then turns off the engine's own request with:

```json
{"launcher": {"hotkey": ""}}
```

## Configuration schema

`schema/compass.schema.json` is generated from `compass_core::config` by `schemars` and never edited
by hand. The `config_schema` test fails when it drifts from the types; regenerate it with

```sh
COMPASS_UPDATE_SCHEMA=1 cargo test -p compass-core --test config_schema
```

`scripts/packaging/check-config-schema.py` validates it, `schema/example.compass.json` and a set of
negative cases with the `jsonschema` package, which is what an editor would do with it.

Upstream Vicinae's `settings.json` is migrated with `compass config migrate` (dry run) and
`compass config migrate --write`; until a `compass.json` exists the engine migrates it in memory at
every start. See `crates/compass-core/src/config_migration.rs` for what maps and what is reported
as left behind.
