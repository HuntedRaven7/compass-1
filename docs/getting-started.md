# Getting started with Compass

Compass is a keyboard launcher for the Linux desktop. Open it with a key, type a few letters, and
press Enter to start an application, run a command, or search your clipboard history, files and
more.

## Install

Compass is published in the TunaOS Flatpak remote. Its runtime comes from Flathub, so add both:

```sh
flatpak remote-add --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak remote-add --if-not-exists tuna-os https://tunaos.org/flatpak/tuna-os.flatpakrepo
flatpak install tuna-os org.tunaos.compass
```

The AppImage, the Arch package (`compass-git`), the Nix flake and building from source are
described in the [README](https://github.com/tuna-os/compass#install).

## First run

Open **Compass** from your application grid. The first time, a short setup walks you through the
theme, the keyboard shortcut and a few recommended extensions. You can change all of them later.

Compass then keeps running in the background. Press Escape to hide the launcher, and your
keyboard shortcut to bring it back.

The examples on this page use the `compass` command. From the Flatpak, run it as
`flatpak run org.tunaos.compass`, for example `flatpak run org.tunaos.compass toggle`.

## Start Compass when you log in

Opening Compass from the application grid starts it for the rest of the session. To start it at
every login, turn on **Start at login**, in the first-run setup's "Make it your own" step or in
Settings, General. It is off until you turn it on. Compass asks your desktop's Background portal,
which shows you what it allows, to run `compass start --hidden` when you log in. Where there is no
Background portal (some wlroots sessions) and Compass is not in the Flatpak, it writes
`~/.config/autostart/compass.desktop` itself, and turning the switch off removes only that file.
If your desktop declines, the switch stays off and says so.

Packages also install a systemd user unit, which Compass never turns on or off. Settings says when
it is installed and whether it is enabled. To use the unit instead of the switch:

```sh
systemctl --user enable --now compass
```

The unit starts with `graphical-session.target` and needs `WAYLAND_DISPLAY` in the systemd user
environment. GNOME and KDE Plasma provide both. Plain Sway does neither by default, so either
start Sway through [uwsm](https://github.com/Vladimir-csp/uwsm) or
[sway-systemd](https://github.com/alebastr/sway-systemd), or add these lines to the end of
`~/.config/sway/config`:

```text
exec systemctl --user import-environment WAYLAND_DISPLAY SWAYSOCK XDG_CURRENT_DESKTOP
exec systemctl --user start sway-session.target
```

with a `~/.config/systemd/user/sway-session.target` that binds to the graphical session:

```ini
[Unit]
Description=Sway session
BindsTo=graphical-session.target
Wants=graphical-session-pre.target
After=graphical-session-pre.target
```

Hyprland and niri sessions started through their own systemd integration (or uwsm) already
provide the target and the environment. Without systemd, run `compass start --hidden` from your
compositor's autostart instead, for example `exec compass start --hidden` in Sway.

## Set a keyboard shortcut

The launcher opens with `compass toggle`, which shows it or hides it. Compass has to be running
for this to work: open it once from the application grid, or run `compass start --hidden` when you
log in.

Compass asks for <kbd>Super</kbd>+<kbd>Space</kbd> by default. To change it, run **Open Settings**
and set **Launcher hotkey** under General, or set `launcher.hotkey` in the configuration file.

### GNOME

On GNOME 48 and later, Compass asks for the shortcut through the desktop's GlobalShortcuts portal
when it starts. GNOME shows a dialog that names the shortcut "Open the Compass launcher". Accept
it, and the key opens Compass from then on.

If you declined the dialog, or you prefer a different key, add a custom shortcut instead:

1. Open **Settings**, then **Keyboard**, then **View and Customize Shortcuts**.
2. Choose **Custom Shortcuts** and add one.
3. Set the command to `compass toggle` (or `flatpak run org.tunaos.compass toggle`) and pick a
   key.

### KDE Plasma

Plasma provides the same GlobalShortcuts portal, so Compass asks for the shortcut when it starts.
To use another key, open **System Settings**, then **Keyboard**, then **Shortcuts**, add a new
command shortcut, and set its command to `compass toggle`.

### Sway, Hyprland and niri

Bind the key in your compositor's configuration file.

Sway, in `~/.config/sway/config`:

```text
bindsym $mod+space exec compass toggle
```

Hyprland, in `~/.config/hypr/hyprland.conf`:

```text
bind = SUPER, SPACE, exec, compass toggle
```

niri, in the `binds` section of `~/.config/niri/config.kdl`:

```kdl
Mod+Space { spawn "compass" "toggle"; }
```

With the Flatpak, the command is `flatpak run org.tunaos.compass toggle`:

```text
bindsym $mod+space exec flatpak run org.tunaos.compass toggle
bind = SUPER, SPACE, exec, flatpak run org.tunaos.compass toggle
Mod+Space { spawn "flatpak" "run" "org.tunaos.compass" "toggle"; }
```

Reload the compositor's configuration after you edit it. On these compositors the
**Launcher hotkey** setting has no effect, because the compositor, not Compass, owns the key.

### Other desktops

Compass binds the key itself where the compositor offers a hotkey protocol (`xx-hotkey-v1`) or
the GlobalShortcuts portal. Anywhere else, bind a key to `compass toggle` in your desktop's
keyboard settings.

`compass doctor` says which of these mechanisms your session has.

## Extensions

Extensions add commands to Compass. Open the launcher and search for **Extension Store** to browse
community extensions, or **Raycast Store** to install Raycast extensions. Each Raycast extension
shows how well it works on Linux. Installed extensions show up in the launcher's search right away.

Compass runs extensions written for Vicinae unchanged. To write your own, see
[Writing extensions for Compass](extensions.md).

### Installing extensions from the configuration file

To give every machine, or every user of one, the same extensions, list them under
`extensions.installed` in `compass.json` (or in the system-wide `/etc/xdg/compass/compass.json`):

```json
{
  "extensions": {
    "installed": ["raycast:raycast/github", "vicinae:zoe/clock"]
  }
}
```

Each entry is `raycast:<owner>/<name>` or `vicinae:<owner>/<name>`, where the owner and name are the
ones in the extension's store address. The id an extension installs under, such as
`store.raycast.github`, also works; Compass then looks the owner up in the store.

Compass installs whatever is listed and missing when it starts and whenever the file changes, in the
background. When the store cannot be reached, it tries again after one, five and fifteen minutes,
then every hour. It never uninstalls anything: an extension you take out of the list stays
installed, with its settings, until you uninstall it; one you uninstall while it is still listed
comes back the next time Compass starts or the file changes.

`compass ext sync` installs the missing ones now and says what happened to each entry, and
`compass ext sync --dry-run` only says what it would install. `compass doctor` lists the entries
that are not installed yet and the ones that cannot be.

### Signing in to a service

Some extensions connect to an account, such as GitHub or Linear. The first time you use one, Compass
shows "Continue in your browser to connect" and opens the service's sign-in page in your default
browser. After you approve access there, the browser hands the result back to Compass, which says
"Connected to" the service. The extension then works without asking again.

The tokens are stored in Compass's encrypted extension database, whose key is kept in your
desktop's keyring. Each extension can read only its own tokens. To disconnect, run **Manage OAuth
Token Sets** in the launcher, choose the extension's entry and remove it; the service's own
settings page is where to revoke the access completely. Without a keyring (some bare window
managers), extensions that keep tokens or preferences cannot save them; `compass doctor` says
when this is the case.

### Permissions

Extensions run in a sandbox. They can use the network and their own data, but not the rest of your
home folder. A few Raycast extensions need a program on your computer, such as `brew`; Compass asks
before running one, with Allow Once, Always Allow or Deny. **Script Permissions** lists what you
allowed, and takes it back.

## Rhai scripts

Rhai scripts are small commands with a searchable list, written in one file with no build step.
Compass ships five, which are in root search like any other command:

- **Web Search**: search the web, a wiki or a code host
- **Unit Converter**: convert lengths, weights, volumes and temperatures
- **Epoch Converter**: Unix timestamps to dates and back
- **Generate**: UUIDs, lorem ipsum and passwords
- **Quick Notes**: jot something down, find it again later

Your own scripts go in a folder each under `~/.local/share/compass/scripts/` (the Flatpak uses
`~/.var/app/org.tunaos.compass/data/compass/scripts/`). A new script appears in root search on its
own. The first time you open one that asks for something, such as the clipboard or saved data,
Compass lists what it wants and runs it only if you allow it. **Script Permissions** shows what each
script was allowed and revokes it.

To write one, start from an example: [Writing Rhai scripts](rust-engine/RHAI-SCRIPTS.md).

## Configuration

Settings are stored in `~/.config/compass/compass.json`. The Flatpak keeps its own copy in
`~/.var/app/org.tunaos.compass/config/compass/compass.json`. Most settings can be changed with the
**Open Settings** command, and the file has a
[JSON Schema](https://github.com/tuna-os/compass/blob/main/packaging/schema/compass.schema.json)
that editors can use for completion and to flag misspelled keys.

Changes to the file apply as soon as it is saved, whether you edit it by hand or run a command such
as `compass theme set dracula`. A key Compass does not know, or a value of the wrong type, does not
stop the rest of the file from loading: Compass uses the default for that setting and says what is
wrong in its log, in `compass doctor` and at the bottom of the settings view.
File-index settings have stricter handling so invalid values cannot widen indexing;
see [Live file-index settings](#live-file-index-settings).

If you used Vicinae before, Compass moves its settings over on first start.

## Themes

Choose a theme in **Settings › Appearance**, or with `compass theme list` and
`compass theme set <name>`. To make your own, start from the template:

```sh
compass theme paths          # the folders Compass reads themes from
mkdir -p ~/.local/share/compass/themes
compass theme template > ~/.local/share/compass/themes/my-theme.toml
compass theme check ~/.local/share/compass/themes/my-theme.toml
compass theme set my-theme
```

Compass reads theme files in the same format as Vicinae, so a Vicinae theme works as it is.

## Privacy

Compass sends no telemetry. On its own, it makes two kinds of request:

- It asks Compass's GitHub releases whether a newer version is out, at most every six hours. Turn
  this off with **Check for updates** in Settings, or `launcher.check_for_updates` in the
  configuration file.
- For the calculator's currency conversions, it downloads the European Central Bank's daily
  exchange rates from `www.ecb.europa.eu`, once a day while it runs. To stop this, set
  `COMPASS_DISABLE_AUTO_RATE_REFRESH=1` in Compass's environment; currency conversions then use
  the last rates downloaded, if any.

Everything else happens only when you ask for it, such as browsing the Vicinae Store or the
Raycast Store and installing an extension. What an installed extension does is up to it.

## Troubleshooting

`compass doctor` checks what works on your machine and explains what is missing, such as the
GlobalShortcuts portal or the GNOME Shell helper extension:

```sh
compass doctor
compass doctor --check-only
```

`--check-only` prints only the problems. When you
[report a bug](https://github.com/tuna-os/compass/issues/new), include the full `compass doctor`
output, your distribution and desktop, and how you installed Compass.

### Live file-index settings

Changes to `providers.files.preferences.autoIndexing`, `indexingPaths`, and
`excludedIndexingPaths` take effect while Compass is running, whether saved in
Settings or edited in `compass.json`. Unchanged settings do not restart the
helper. Turning automatic indexing off stops it; turning it back on uses the
latest folders and exclusions. Direct-path search and recently used files
remain available when indexing is off.

Folders are literal paths. `~` and `~/` expand to your home directory; other
relative paths are resolved against the engine's working directory. Dot
components and duplicate paths are normalized without following symlinks.
Environment variables, named-user tildes and shell wildcards are not expanded.
An explicit empty indexed-folder list indexes nothing; removing that setting
restores the home-directory default.

Malformed file-index settings keep the last valid settings during a session.
At startup, unreadable or malformed configuration leaves indexing off until a
valid configuration is saved. On every helper start, cached rows outside the
current roots or inside exclusions are removed before indexed queries become
available. Live changes apply the same boundary, including to delayed scanner
writes and query results. This changes only the derived file-search index;
source files are never deleted, and no schema migration or full index rebuild
is requested. A rejected configuration can be retried by saving the same valid
settings again.

### File-index size diagnostics

Run `compass fs stats` (or `compass fs stats --json`) to measure the existing
file-search index without starting the engine. It reports the main database,
WAL and SHM file lengths in bytes, SQLite page size, page count and freelist
pages, indexed rows (files and directories), and the two FTS indexes' page
footprint when this SQLite build provides `dbstat`. Missing files and unavailable
measurements have explicit statuses; they are never silently reported as zero.
The report contains no indexed paths or clipboard data. It also splits the pages
into in-use bytes (rows, their indexes and the FTS indexes) and reclaimable bytes
(free pages SQLite reuses before growing the file, which only compaction returns
to the disk). `compass doctor` summarizes the same measurements in its
`files.index` check.

The database is `$XDG_CACHE_HOME/compass/file-indexer/compass-file-index.db`,
falling back to `~/.cache/compass/file-indexer/compass-file-index.db`. The command
opens it read-only and never creates a missing database, changes its schema or
journal mode, checkpoints, vacuums, or rebuilds the index. No schema migration
is needed: this reads the existing version and reports unsupported versions.
SQLite can update reader-lock bookkeeping in an existing SHM file. If a WAL
sidecar is absent or unreadable, SQL measurements are skipped to avoid creating
sidecars; retry while the indexer is running. A concurrently exiting indexer
can still remove sidecars after this preflight, in which case SQLite may recreate
them for reader coordination.

SQL measurements share a read transaction that includes committed WAL pages.
Queries have a two-second interruption budget and a 100 ms lock wait; busy,
unreadable, corrupt, unsupported, or timed-out databases produce unavailable
reasons. File lengths are sampled separately and may change during a scan.
They are logical lengths, not allocated disk blocks. WAL length includes reusable
frames; FTS bytes include shadow-table pages but exclude the external
`indexed_file` content and WAL overhead. Do not add FTS bytes or freelist bytes
to the database size: those pages are already part of the database. These are
measurements, not a performance comparison or a recommendation to compact.

The command exits 1 when required measurements are unavailable and 0 when they
are available. Optional FTS-footprint unavailability does not fail the command.
