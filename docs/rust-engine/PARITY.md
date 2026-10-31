# Parity ledger

> **Read under [ADR-0017](./adr/0017-a-new-launcher-not-a-reimplementation.md).** Compass is a new
> launcher, not a reimplementation. The **parity test ✓** column below now means *tested*: an
> absolute test that fails on a regression satisfies it, whether or not the C++ engine agrees. A
> differential test still counts, as a tripwire. Divergences that improve on Vicinae are declared
> here and kept.

The definition of done for the Rust engine transformation (#2). Every row must be fully green
before its C++ source is deleted, and **nothing leaves `src/` until it is**.

Columns:

- **C++ ✓** — the behaviour exists in the C++ engine today. Effectively always true; it is here so a
  row with a false in this column stands out as new functionality rather than a port.
- **Rust ✓** — implemented in the Rust engine.
- **parity test ✓** — covered by a Suite 0 differential case (#12) or, where a differential is not
  meaningful, by ported tests from the C++ suite (`PLAN.md` §8.3). A checked box here means a test
  would *fail* if the two engines diverged.
- **C++ deleted ✓** — the C++ source is gone. Every row reads ✅: the whole C++ tree left the
  repository with ADR-0021, including the out-of-scope rows that were once marked `never`. The C++
  paths in the left-hand column are history; upstream Vicinae releases are the reference now.

A row may go green with a **declared divergence** instead of exact parity: record it in the
Divergences section below with a rationale. Divergences are declared, never discovered.

## Scope of this ledger

**Every row below is the Linux port.** That is Phases 0–8, and it is what the row set was built to
track. It is not the whole port, and reading it as one would badly understate what remains: a fully
green ledger here is the state at the end of Phase 8, with Qt still in the repository.

The reason is that the non-Linux surface is not shaped like these rows. Rows here map a C++
directory to a Rust crate, because Linux behaviour lives in Linux files. macOS and Windows
behaviour mostly does not — it lives in conditional compilation inside files that compile
everywhere, so there is no directory to put in the left-hand column:

| | dedicated `.cpp` | `#ifdef` sites | shared files touched |
|---|---:|---:|---:|
| Linux | 59 | 72 (`Q_OS_LINUX`) | 30 |
| Windows | 33 | 100 (`Q_OS_WIN`) | 43 |
| macOS | 3 | 102 (`Q_OS_MAC`) | 38 |

Counted by attributing each translation unit to the `if (APPLE)` / `if (WIN32)` /
`if (UNIX AND NOT APPLE)` block that lists it in `src/server/CMakeLists.txt`, and by grepping the
guards. 61 shared files carry at least one conditional; `server.cpp` alone has 29.

So the macOS and Windows work is tracked as **conditional sites resolved**, not as directories
ported, and it lives in [PLAN.md](./PLAN.md) §6 Phases 9 and 10 rather than here. A row in this
ledger going green says nothing about either.

Update this file in the same PR that changes a box. It is meant to be read in standup.

### A correction to the plan's deletion rule

`PLAN.md` §6 Phase 5 said each ported group's `src/` directory is deleted "in the same PR that turns
the row green". That cannot be right, and following it would break the build: both engines ship side
by side until Phase 7, so the C++ engine still needs its own matcher, IPC and parsers no matter how
complete the Rust replacements are.

The rule is therefore: **a row goes green in the PR that ports it and proves parity; the `C++
deleted` column is worked at Phase 8, after cutover.** `⏳` marks rows that are done but waiting on
that. The plan has been corrected.

### Progress at a glance

| Crate | Tests | State |
|---|---|---|
| `compass-core` | 1,743 | app index, frecency, config, root search, glyphs, snippets, toasts, quicklinks, the extension boilerplate generator, the image fetch queue, the confirm dialog, volume and mute, the paste handoff, the telemetry record, update checks, the news notices, the selected text, the file dialog, both extension stores, emoji metadata, the snippet input server's framing, the icon URL scheme, contrast colours, the two per-window Wayland registries, six desktops' wallpaper vocabularies, the font browser's grouping, snippet expansion, the tray menu and the StatusNotifierItem host, the script-command scan, the calculator history view, the window and workspace switchers, the media and volume commands, the file search command, the Markdown showcase, the Raycast store views, the root list's clock and shortcuts, the emoji picker's skin tones, the bug report and fallback manager, the window-manager dispatch and focus memory, the indexer's entry filter, query policy and result ranking, the staged extension install, the quicklink list, and the indexer's tree walk, incremental rules, scan scheduling, root compaction, the index reconciliation, the watch policy and script output styling |
| `compass` | 184 | CLI, a 14-check `doctor`, and **the engine daemon** |
| `compass-worker-host` | 187 | the extension host: framing, sandboxed spawn, 45 of tsapi's 49 methods, and the real runtime |
| `compass-xdg` | 229 | desktop entries, locale, exec, reader, mimeapps, bookmarks — scope gaps listed below |
| `compass-clipboard` | 114 | history store, ingest, migrations, and the history command's own decisions; stored enums pinned to the C++ header |
| `compass-extension-api` | 73 | view tree, derived identity, diff, dispatch, capabilities, controlled inputs |
| `compass-ipc` | 73 | framing, transport, single-instance |
| `compass-search` | 58 | fuzzy, plus an exact port of fzf's coherence rule |
| `compass-portals` | 54 | XDG portals; availability is a three-state outcome, not a boolean |
| `compass-shell` | 46 | GNOME Shell DBus client; tests spawn a real `dbus-daemon` |
| `compass-ui` | 118 | the launcher window and its views, the root list's sections and selection, and the action panel — open, filtered, navigated and drawn |
| `compass-crypto` | 24 | AES-GCM and HKDF; cross-decrypted against the C++ probe per-PR |
| `compass-sandbox` | 23 | Landlock, a seccomp denylist, and the launcher that applies them to itself |
| `compass-local-storage` | 36 | the extension key-value store, lossy typing and all, and the calculator history with its time grouping |
| `compass-media` | 13 | MPRIS players, with the timeout the C++ has for a reason |
| `compass-power` | 11 | logind; one C++ bug deliberately not reproduced |
| `compass-db` | 10 | the shared migration runner and the `vicinae` schema |
| `compass-oauth-store` | 9 | the extension token store |
| `compass-sqlcipher-sys` | 8 | `rusqlite` over SQLCipher, and the vendored tokenizer; connection pragmas pinned to the C++ |
| `compass-testkit` | 8 | corpora — **757 desktop entries, 738 harvested from real hosts** |
| `compass-platform` | 6 | the launcher seam (ADR-0013) |
| `compass-platform-linux` | 26 | the launcher, and the uinput virtual keyboard's protocol |
| `compass-wayland` | 33 | activation and keyboard inhibit, and the clipboard offer filter |
| **Total** | **3,099** | what `make check-rust` reports, doctests included, all green under fmt and clippy `-D warnings` |

The per-crate column is measured with `cargo test -p <crate> --all-targets` and sums to 2,071;
the 3,099 is the workspace figure `make check-rust` prints, which additionally covers doctests and
harnesses not attributable to a single package. Both numbers are given rather than one reconciled
figure, because quietly picking whichever is larger is how a count stops meaning anything.

## Progress

Scaffolding, corpora and CI are in place, and **twenty** crates have landed: **1,151 tests** across
the workspace, all green. `compass-db` (the shared migration runner and the `vicinae` schema, extracted from
`compass-clipboard`), `compass-local-storage`, `compass-oauth-store` and `compass-sandbox` are the four newest. An earlier revision of this
paragraph said nine crates and 597 tests, and both had drifted — `compass-clipboard`,
`compass-crypto`, `compass-sqlcipher-sys`, `compass-platform`, `compass-platform-linux`,
`compass-wayland` and `compass-worker-host` were missing from the table entirely, and the corpus
line still read 115 entries against an actual 757.

Almost no row is fully green, and no C++ directory may be deleted yet — see the partial markers and
the divergences below. 🟡 means implemented but not to the full scope of the C++ source. A green
`Rust ✓` with a 🟡 `parity test ✓` means the code exists and is tested as far as this container can
test it; `compass-portals` is the clearest case, since no amount of local testing can tell us
whether a real GNOME session grants the shortcut we ask for.

## Libraries and standalone binaries

| C++ source | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|---|---|:-:|:-:|:-:|:-:|
| `src/lib/xdgpp` | `compass-xdg` | Phase 1 | ✅ | ✅ | ✅ | ✅ |
| `src/lib/fuzzy` | `compass-search` | Phase 1 | ✅ | ✅ | ✅ | ✅ |
| `src/lib/crypto` | `compass-crypto` | Phase 3 | ✅ | ✅ | ✅ | ✅ |
| `src/lib/glyph` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/lib/script-command` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/lib/vicinae-ipc` | `compass-ipc` | Phase 2 | ✅ | ✅ | ✅ | ✅ |
| `src/lib/figura` | `compass-ipc` | Phase 2 | ✅ | n/a | n/a | ✅ |
| `src/lib/common` | `compass-core` | Phase 2 | ✅ | ✅ | ✅ | ✅ |
| `src/lib/linux-utils` | `compass-platform-linux` | Phase 2 | ✅ | ✅ | ✅ | ✅ |
| `src/lib/soulver` | `—` | n/a (macOS) | ✅ | n/a | n/a | ✅ |
| `src/cli` | `crates/compass` | Phase 2 | ✅ | ✅ | ✅ | ✅ |
| `src/file-indexer` | `compass-db`, `compass-file-indexer` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/data-control-server` | `compass-wayland` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/snippet` | `compass-input-server` (`vicinae-input-server`), `compass-core::snippet` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/browser-extension` | — | **out of scope** | ✅ | n/a | n/a | ✅ |

## Services

| C++ source | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|---|---|:-:|:-:|:-:|:-:|
| `src/services/app-runtime` | `compass-core`, `compass::serve::app_runtime` | Phase 1 | ✅ | ✅ | ✅ | ✅ |
| `src/services/app-service` | `compass-core` | Phase 1 | ✅ | ✅ | ✅ | ✅ |
| `src/services/asset-resolver` | `compass-core` | Phase 1 | ✅ | ✅ | ✅ | ✅ |
| `src/services/audio-control` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/autostart` | `—` | n/a (macOS) | ✅ | n/a | n/a | ✅ |
| `src/services/browser-extension` | — | **out of scope** | ✅ | n/a | n/a | ✅ |
| `src/services/builtin-icon` | `compass-core` | Phase 1 | ✅ | ✅ | ✅ | ✅ |
| `src/services/calculator-service` | `compass-local-storage`, `compass-core::exchange_rates`, `compass::exchange_rates` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/clipboard` | `compass-clipboard` | Phase 3 | ✅ | ✅ | ✅ | ✅ |
| `src/services/desktop-notification` | `notify-rust` (crate) | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/extension-boilerplate-generator` | `compass-core` | Phase 4 | ✅ | ✅ | ✅ | ✅ |
| `src/services/extension-registry` | `compass-core` | Phase 4 | ✅ | ✅ | ✅ | ✅ |
| `src/services/extension-store` | `compass-core` | Phase 4 | ✅ | ✅ | ✅ | ✅ |
| `src/services/file-chooser` | `compass-core` | Phase 2 | ✅ | ✅ | ✅ | ✅ |
| `src/services/files-service` | `compass-xdg` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/font-service` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/global-shortcuts` | `compass-core::global_shortcuts`, `compass-wayland::hotkey`, `compass-portals`, `compass::global_shortcuts` | Phase 1 | ✅ | ✅ | ✅ | ✅ |
| `src/services/glyph-service` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/image-fetcher` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/input-server` | `compass::input_server`, `compass-core::input_server` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/keybinding` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/local-storage` | `compass-local-storage` | Phase 4 | ✅ | ✅ | ✅ | ✅ |
| `src/services/media-control` | `compass-media` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/menu-bar` | `—` | n/a (macOS) | ✅ | n/a | n/a | ✅ |
| `src/services/navigation` | `compass-core` | Phase 2 | ✅ | ✅ | ✅ | ✅ |
| `src/services/news` | `—` | n/a (decision) | ✅ | n/a | n/a | ✅ |
| `src/services/oauth` | `compass-oauth-store` | Phase 4 | ✅ | ✅ | ✅ | ✅ |
| `src/services/paste` | `compass-core`, `compass::paste`, `compass-wayland::virtual_keyboard` | Phase 3 | ✅ | ✅ | ✅ | ✅ |
| `src/services/permissions` | `—` | n/a (macOS) | ✅ | n/a | n/a | ✅ |
| `src/services/power-manager` | `compass-power` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/raycast` | `compass-core` | Phase 4 | ✅ | ✅ | ✅ | ✅ |
| `src/services/root-item-manager` | `compass-core` | Phase 2 | ✅ | ✅ | ✅ | ✅ |
| `src/services/script-command` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/selection` | `compass-core` | Phase 3 | ✅ | ✅ | ✅ | ✅ |
| `src/services/shortcut` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/shortcut-inhibit` | `compass-core`, `compass-wayland::keyboard_inhibit` | Phase 3 | ✅ | ✅ | ✅ | ✅ |
| `src/services/snippet` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/telemetry` | `—` | n/a (decision) | ✅ | n/a | n/a | ✅ |
| `src/services/toast` | `compass-core` | Phase 4 | ✅ | ✅ | ✅ | ✅ |
| `src/services/tray` | `compass-core`, `compass::tray_icon` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/tray-host` | `compass-core`, `compass::tray_host` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/update` | `compass-core::update`, `compass::updates` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/url-scheme` | `—` | n/a (Windows) | ✅ | n/a | n/a | ✅ |
| `src/services/wallpaper` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/services/window-manager` | `compass-core` | Phase 3 | ✅ | ✅ | ✅ | ✅ |
| `src/services/window-material` | `compass-core`, `compass_wayland::material`, `compass-wayland-foreign`, `compass::window_material`, `compass_ui::material` | Phase 5 | ✅ | ✅ | ✅ | ✅ |

## Builtins

| C++ source | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|---|---|:-:|:-:|:-:|:-:|
| `src/builtins/browser` | — | **out of scope** | ✅ | n/a | n/a | ✅ |
| `src/builtins/calculator` | `compass-core`, `compass_ui::calculator_page` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/builtins/clipboard` | `compass-clipboard` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/builtins/developer` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/builtins/file` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/builtins/font` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/builtins/internal` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/builtins/media` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/builtins/power-management` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/builtins/raycast` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/builtins/root` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/builtins/shortcut` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/builtins/snippet` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/builtins/system` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/builtins/theme` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/builtins/vicinae` | `compass-core`, `compass_ui::app::vicinae` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/builtins/wm` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |

## The window

`src/server/src/ui` — about **29,700 lines** across Qt Widgets and QML, and until this section
existed the ledger did not mention it. That was the largest omission in the file: 110 rows covered
every service, library and builtin, and none of them covered the thing a user actually looks at.
A row per subdirectory, with its C++ size, so that the distance is visible rather than implied.

| C++ source | lines | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|--:|---|---|:-:|:-:|:-:|:-:|
| `src/server/src/ui/qml` | 14,660 | `compass-ui` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/server/src/ui/quick` | 3,806 | `compass-ui` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/server/src/ui/views` | 2,760 | `compass-ui` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/server/src/ui/settings` | 2,292 | `compass-ui` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/server/src/ui/image` | 2,154 | `compass-ui` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/server/src/ui/windows` | 1,881 | `compass-ui` | Phase 3 | ✅ | ✅ | ✅ | ✅ |
| `src/server/src/ui/action-panel` | 1,366 | `compass-ui` | Phase 5 | ✅ | ✅ | ✅ | ✅ |
| `src/server/src/ui/bridges` | 539 | `compass-ui` | Phase 4 | ✅ | ✅ | ✅ | ✅ |
| `src/server/src/ui/alert` | 279 | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ✅ |

`compass-ui` opens a window, searches applications, moves the selection, launches on Enter and
dismisses on Escape. It also draws themed application icons and an action panel. The panel now
has its own focused fuzzy filter, dispatches Open and both copy actions by stable IDs, accepts
clicks, and restores search focus when closed. Copy actions emit native clipboard writes;
headless tests inspect those writes and exercise the widgets, but delivery to another application
still needs a desktop check. Since then the launcher has grown a page per builtin, extension
views (list, grid, detail, form) and dialogs, which is why no row here is ❌ any more (the ledger
truth pass below). The HUD and onboarding landed in "The gaps pass, HUD and onboarding"; `alert`,
`action-panel` and, since the settings pass, `settings` are the rows fully green.

Three things about this section are worth stating plainly, because a table of ❌s invited the wrong
reading:

- **These rows are not all ports.** ADR-0001 chose Iced over Qt Widgets and QML, so most of this
  tree has no Rust counterpart to write — it has a *replacement* to design. `parity test ✓` on a
  QML row cannot mean a differential against the C++ widget; it will mean the ported behavioural
  tests §8.3 describes, or nothing.
- **`bridges` is Phase 4, not Phase 5**, because it is the seam the extension host renders through
  rather than chrome. `compass-extension-api` already models the view tree and its diff; what is
  missing is the half that turns a diff into pixels.
- **No line of this could be deleted before the Phase 7 cutover**, like every other row; all of it went with ADR-0021.
- **The pointer on lists.** A click selects the row and activates it, as the C++ does with
  `activate_on_single_click` (Compass has only that mode; the C++ default selects on the first
  click and activates on a double one). Hover never moves the selection, as `SelectableDelegate.qml`.
  Not ported: the C++'s hover tint (`list.item.hover.background`, shown only once the pointer has
  moved since the list last changed, `HoverActivation`).

## Not-yet-ported scope

Tracked here so a 🟡 does not quietly become a ✅.

### The ledger truth pass (2026-09-25)

Every `Rust ✓` and `parity test ✓` cell that was 🟡 or ❌ was checked against the code, because much
had landed since the notes were written and many of them described a port that no longer existed.
The ledger went from **75 of 158 (47%) to 116 of 156 (74%)**. The countable total fell by two
because `src/lib/soulver` is a macOS-only calculator backend and was marked ❌/❌ where the other
macOS rows (`autostart`, `menu-bar`, `permissions`) are n/a.

The rule the flips follow: `Rust ✓` is ✅ only where the Rust covers the row's Linux scope, with
anything that behaves differently declared in this file; `parity test ✓` is ✅ where the Rust that
exists has named tests that fail on a regression (ADR-0017), which is the reading the rest of the
ledger already used for rows whose model landed before their backend. For the window rows that
reading is applied only where the tested part is the row's core: `qml`, `quick` and `settings` stay
🟡 in both columns because what is tested there is a sliver of the row. Five gaps were closed in the
same pass rather than listed: the two `xdgpp` writers, `glyph`'s `is_emoji`, `{selection}` in
shortcuts, the power commands' two preferences, and a notification's urgency and file icon.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/lib/xdgpp` | Rust ✅ | `compass_xdg::{mimeapps_writer, terminal}` joined the rest | `mimeapps_writer::tests` (`mime.cpp`'s writer cases), `a_chosen_terminal_*` and `choosing_again_replaces_the_previous_choice_and_keeps_comments` (`xdg-terminal-exec.cpp`), `tests/special.rs` (`special.cpp`) |
| `src/lib/glyph` | Rust ✅ | `compass_core::glyph::is_emoji`, a declared difference in method | `tests/glyph.rs`: `special-cases.cpp` and `tones.cpp` verbatim |
| `src/lib/vicinae-ipc` | parity ✅ | `compass-ipc` | `framing` (`every_request_variant_round_trips`, `an_oversized_length_prefix_is_rejected_without_allocating`), `transport` (`a_stale_socket_is_reclaimed`), `proptests`; the C++ suite is one case with no assertion |
| `src/lib/common` | Rust ✅ | `compass_ipc::path` (`ensurePrivateDir`, the per-user fallback), `compass_core::file_category`, `compass::cli` (`isAppDeeplink`), `compass::indexer_client` (`findHelperProgram`); `clipboard-protocol.hpp` framed a helper process the Rust does not have | `socket_dir::a_symlink_is_refused_without_being_followed`, `a_world_writable_directory_is_refused`, `file_category::every_list_is_the_cpps_exactly`, `a_bare_deeplink_becomes_the_deeplink_command`, `helper_search_covers_the_installed_layout` |
| `src/lib/linux-utils` | Rust ✅ | `compass_platform_linux::keyboard`, and the device and keymap in `compass-input-server` | `the_virtual_keyboard_is_created_and_the_server_does_not_read_itself`, `xkbcommon_agrees_with_the_table`, `a_shifted_keystroke_is_exactly_this_sequence` |
| `src/lib/soulver` | n/a | macOS only | — |
| `src/cli` | parity ✅ | `compass::cli` | `window_commands_send_the_matching_request`, `ping_reports_the_protocol_version_and_pid`, `commands_without_a_daemon_explain_themselves`, `dmenu_shows_stdin_in_the_attached_window_and_prints_the_choice` |
| `src/file-indexer` | both ✅ | `compass-db`, `compass::{indexer_service, indexer_watch}`, `compass-file-indexer` | `query_quality.rs` (23/23), `query_round_trips_through_a_fake_helper`, `configure_replies_null_and_scans_notify`, `search_files_indexes_the_home_directory_and_finds_a_file_by_a_misspelled_query` |
| `src/data-control-server` | Rust ✅ | `compass_wayland::{data_control, clipboard}` | `a_copy_is_seen_by_the_watcher_and_round_trips_through_wl_clipboard_rs`, `a_password_manager_copy_is_marked_concealed` (headless Sway) |
| `src/services/audio-control` | Rust ✅ | `compass_core::audio_control` over the engine's host `pactl` | `compass-core/tests/audio_control.rs`, `a_volume_command_runs_pactl_with_the_cpp_arguments` |
| `src/services/clipboard` | parity ✅ | `compass-clipboard`, `compass::clipboard_service` | see "Why `parity test ✓` is now ✅" below |
| `src/services/extension-store` | Rust ✅ | `compass_core::extension_store`, `compass::stores` | `the_vicinae_store_lists_installs_into_root_search_and_uninstalls`, `a_search_containing_an_ampersand_stays_one_parameter` |
| `src/services/file-chooser` | Rust ✅ | `compass_portals::file_chooser`, served to extensions' `FilePicker` | `file_chooser::tests` (`outcomes_expose_paths_only_when_selected`, `percent_escapes_decode`) |
| `src/services/files-service` | Rust ✅ | `compass_xdg::bookmarks` (recent files), `compass::indexer_client` (the indexer) | `recording_an_access_adds_then_bumps_and_keeps_the_rest`, `search_files_lists_recent_files_for_the_empty_query_and_a_typed_path_directly` |
| `src/services/font-service` | Rust ✅ | `compass_core::font_service`, `compass::fonts` | `a_font_file_is_found_and_classified_by_what_it_covers`, `browse_fonts_lists_families_and_previews_one` |
| `src/services/image-fetcher` | Rust ✅ | `compass_ui::remote_image` (declared under "Extension views" #1) | `a_stored_image_is_found_again_and_a_different_url_is_not`, `pruning_removes_the_oldest_until_the_budget_holds` |
| `src/services/media-control` | Rust ✅ | `compass-media`, Now Playing | `now_playing_lists_the_players_and_controls_the_selected_one`, `a_player_argument_picks_the_player_and_now_playing_lists_and_drives_them` |
| `src/services/oauth` | Rust ✅ | `compass-oauth-store`, `compass_worker_host::oauth_service` (authorize) | `a_token_set_round_trips`, `one_extension_cannot_read_anothers_tokens`, `an_oauth_authorization_opens_the_browser_and_the_redirect_answers_it` |
| `src/services/power-manager` | Rust ✅ | `compass_power::PowerManager` (every call the abstract manager declares) | `logind_replies_are_read_rather_than_counted`, `the_capability_reply_is_read_and_not_merely_counted` |
| `src/services/raycast` | Rust ✅ | `compass_core::raycast_store`, `compass::stores` | `the_raycast_store_badges_compatibility_and_notices_an_update` |
| `src/services/script-command` | Rust ✅ | `compass_core::script_scan`, `compass::scripts` (rescan on summon, declared) | `script_commands_are_scanned_searched_and_run_in_their_modes` |
| `src/services/selection` | Rust ✅ | data-control on wlroots, the Shell extension on GNOME | `the_primary_selection_is_its_text_or_nothing`, `on_sway_an_extension_reads_the_selection_the_windows_and_the_monitors`, `on_sway_a_shortcut_expands_the_selected_text` |
| `src/services/snippet` | Rust ✅ | `compass_core::{snippet_store, snippet_expander}`, `compass-input-server` | `snippets_are_imported_created_expanded_edited_and_removed`, `the_input_server_is_told_the_keywords_and_follows_the_setting` |
| `src/services/wallpaper` | Rust ✅ | `compass_core::wallpaper`, `compass::extension_wallpaper` (all six Linux backends) | `compass-core/tests/wallpaper.rs`, `a_failing_command_says_its_stderr_else_its_code` |
| `src/services/window-manager` | Rust ✅, parity ✅ | dispatch plus GNOME, wlroots, Hyprland, niri and KDE (KWin, "The gaps pass, KDE"), and GNOME's workspaces ("The gaps pass, GNOME workspaces and shortcut probes"); X11 is n/a ("Product decisions") | `compass-core/tests/window_manager.rs`, `on_gnome_switch_workspaces_lists_and_switches_through_the_shell_extension`, `compositor_ipc.rs`, `compass-platform-linux/tests/kwin.rs`, `on_hyprland_windows_workspaces_and_focus_come_from_its_socket`, `on_sway_the_engine_lists_focuses_and_closes_windows_without_the_shell_extension` |
| `src/builtins/developer` | both ✅ | Create Extension end to end | `a_valid_form_writes_the_boilerplate_and_an_invalid_one_says_why`, `create_extension_sends_the_form_and_shows_where_it_went` |
| `src/builtins/font` | parity ✅ | Browse Fonts | `browse_fonts_is_a_grid_that_remembers_its_category_and_sets_the_font`, `set_as_vicinae_font_writes_the_family_and_keeps_the_rest_of_font` |
| `src/builtins/power-management` | both ✅ | the plan, both preferences, the dialog | `the_confirm_preference_decides_whether_a_power_command_asks`, `a_power_command_with_a_custom_program_runs_it_instead` |
| `src/builtins/raycast` | Rust ✅ | the Raycast store's views | `the_raycast_store_badges_compatibility_and_notices_an_update`, `a_deeplink_opens_the_detail_page_and_uninstalling_asks_in_a_dialog` |
| `src/builtins/shortcut` | parity ✅ | Create and Manage Shortcuts | `a_shortcut_in_root_search_asks_for_its_argument_then_opens`, `manage_shortcuts_filters_edits_and_removes` |
| `src/builtins/snippet` | parity ✅ | Create and Manage Snippets | `manage_snippets_copies_asking_for_arguments_first`, `editing_a_snippet_keeps_its_apps_and_returns_to_the_list` |
| `src/builtins/system` | parity ✅ | Run Terminal Program, the Browse Apps model | `run_terminal_program_lists_path_and_runs_directly_or_refuses`, `compass-core/tests/browse_apps.rs` |
| `src/builtins/theme` | parity ✅ | Set Theme | `set_theme_keeps_the_theme_in_the_configuration`, `a_theme_file_is_read_and_resolved_with_its_derivations` |
| `ui/views` | Rust 🟡, parity ✅ | root sections, dmenu, the list and grid pages | `compass-ui/tests/root_list.rs`, `dmenu_page::tests`, `the_grid_moves_by_tile_and_by_row` |
| `ui/image` | Rust 🟡, parity ✅ | `compass_core::{image_url, contrast}`, `compass_ui::{icons, remote_image}` | `icons::tests`, `row_icons_resolve_assets_file_urls_themes_and_colour_cells` |
| `ui/windows` | parity ✅ | the resident launcher window | `compass-ui/tests/resident.rs`, `compass-ui/tests/paint.rs` |
| `ui/action-panel` | Rust 🟡, parity ✅ | `compass_ui::action_panel` | `compass-ui/tests/action_panel.rs` (sections, filter, shortcuts, `in_launcher::*`) |
| `ui/bridges` | Rust 🟡, parity ✅ | extension views drawn from the view tree | `the_host_filters_fuzzily_and_enter_runs_the_selected_rows_first_action`, `a_form_keeps_typing_over_stale_echoes_and_submits_its_values` |
| `ui/alert` | Rust ✅ | `compass_core::alert`, the launcher's dialog | `a_second_alert_cancels_the_first_and_reports_it`, `a_replaced_alert_and_one_navigated_away_from_both_answer_no` |
| `ui/qml`, `ui/quick`, `ui/settings` | Rust 🟡, parity 🟡 | Iced replacements for part of each | widget tests and the paint tier, over a small part of each row |

**Rows still amber that had no note saying why.** Each sentence names only what is genuinely missing;
PLAN §12.0 sizes them and says what blocks each.

- `src/cli`: closed in the gaps pass below.
- `src/services/app-runtime`: closed in the gaps pass below.
- `src/services/calculator-service`: the history is served since the gaps pass; currency
  conversion and Refresh Exchange Rates closed in "The gaps pass, currency" below, over the ECB's
  daily reference rates.
- `src/services/desktop-notification`: the urgency and an icon that is a file are passed since this
  pass (`a_notification_carries_the_urgency_and_an_icon_file`); rendering any other icon (a builtin
  one, a remote one) to a temporary PNG landed after it (see "Gaps closed after the truth pass").
- `src/services/global-shortcuts`: per-command global shortcuts, `vicinae-hotkey-v1`, the
  launcher hotkey from the configuration, close on focus loss and conflict detection landed in "The
  gaps pass, global shortcuts", and `globalShortcuts.inhibitApps` and the recorder's `probeBind` in
  "The gaps pass, GNOME workspaces and shortcut probes". The X11 backend is n/a: Compass is
  Wayland only ("Product decisions"), so nothing is left.
- `src/services/news`, `src/services/telemetry`: n/a in both columns. A hard fork: no news feed,
  no telemetry, by decision 2026-09-25 ("Product decisions"). The C++ models stay in
  `compass-core` (`news`, `telemetry`) as the record of the port and for the store's base URL;
  nothing fetches or sends them.
- `src/services/update`: closed in "Product decisions" below: Compass checks its own GitHub
  releases and says so in the root search; it never installs.
- `src/services/paste`: synthetic paste on wlroots closed in "The gaps pass, wlroots paste and
  inhibit" (the input server's `injectPaste`, else `zwp_virtual_keyboard_v1`).
- `src/services/shortcut-inhibit`: the keyboard-shortcuts-inhibit client and the recorder's use of
  it closed in "The gaps pass, wlroots paste and inhibit".
- `src/services/window-material`: the `ext-background-effect-v1` client is ported
  (`compass_wayland::material`, "The gaps pass, HUD and onboarding") and applied to the launcher's
  own surface under the `xdg_toplevel` presentation through an approved `unsafe` bridge
  (`compass-wayland-foreign`, ADR-0019; "The window-material pass: an unsafe bridge"). The
  layer-shell presentation is not blurred, a declared difference there.
- `src/services/tray`: Compass's own tray icon closed in "The gaps pass, tray and sandbox".
- `src/builtins/snippet`: closed in "The gaps pass, UI" below (the detail pane and the `\{`
  escape).
- `ui/qml`, `ui/quick`: The HUD and onboarding closed in "The gaps pass, HUD and onboarding".
  `ui/views` closed in "The gaps pass, UI" below (match and Markdown highlighting, extension
  grids; the edit-keywords view had landed with clipboard history and the emoji picker, the
  app-selector in the views pass), the settings pages in "The gaps pass, settings"; dragging
  out of the window is a declared difference, Iced having no drag out of a window.
- `ui/settings`: closed in "The gaps pass, settings" below.
- `ui/windows`: the settings window is a view of the launcher since "The gaps pass, settings"
  (declared there). The HUD and onboarding closed in "The gaps pass, HUD and onboarding"
  (onboarding drawn in the launcher card, declared there).
- `ui/image`: the builtin icon set, command tiles and badges and file-type icons are drawn since
  "The gaps pass, icons and tray"; masks, root rows' icons, favicons, `ImageURL(source)` and the
  tile's gradient and shadow since "The gaps pass, UI" below.
- `ui/action-panel`: closed in "The gaps pass, root and actions" below.
- `ui/bridges`: closed in "The gaps pass, UI" below (a Markdown detail's images are fetched and
  drawn).

### Product decisions: updates, news, telemetry, X11 (2026-09-25)

Four rows waited on a decision rather than on work. The maintainer decided them on 2026-09-25:

1. **Update check only.** Compass checks GitHub releases of `tuna-os/compass` for a newer version
   and says so; the package manager that installed it installs it.
2. **No news, no telemetry.** A hard fork: no news feed, no telemetry, by decision 2026-09-25.
   `src/services/news` and `src/services/telemetry` are n/a in both columns.
3. **Wayland only: no X11.** Global shortcuts' X11 backend and the window manager's X11 provider
   are n/a. Neither row flips on it: global shortcuts keeps `inhibitApps` and `probeBind`
   (`Rust ✓` 🟡), and the window manager keeps GNOME's workspace list (the Shell extension's
   contract has no `ListWorkspaces`), so both stay amber for those alone.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/services/update` | Rust 🟡 → ✅ | `compass_core::update` (`ReleaseCheck`, `CheckCache`, `load_cache`/`save_cache`, the feed at `repos/tuna-os/compass/releases/latest`), `compass::updates` (`Updates`, `GithubFeed` over the engine's `ureq` agent, `ReleaseFeed`), IPC v21 `Request::UpdateStatus`/`Response::UpdateStatus` and `Request::SkipUpdate`, `launcher.check_for_updates`, `compass_ui::app::release_check` (the root search's Update row and its panel) | `a_release_check_offers_any_newer_published_release_without_an_asset`, `a_release_check_keeps_every_other_gate`, `a_check_is_fresh_for_six_hours_and_not_after`, `the_check_cache_round_trips_and_a_bad_one_is_empty`, `compass_reads_its_own_releases`; `a_newer_release_is_offered_and_asked_for_once_in_six_hours`, `checking_off_asks_nothing`, `a_build_that_is_not_a_release_asks_nothing`, `a_skipped_release_is_not_offered_but_the_next_one_is`, `an_unreachable_feed_keeps_the_last_answer_and_waits_six_hours`, `the_github_feed_reads_the_release_from_its_url` (a local `tiny_http` feed), `the_github_feed_reports_a_refusal`; `a_newer_release_is_offered_once_checked_and_skipping_it_is_remembered`, `with_update_checks_off_the_feed_is_never_asked` (a real engine against a local feed); `a_newer_release_leads_the_empty_query_and_its_panel_opens_or_skips_it` |
| `src/services/news` | Rust, parity 🟡/✅ → n/a | — | — |
| `src/services/telemetry` | Rust, parity 🟡/✅ → n/a | — | — |

No test reaches the network: the engine tests point `COMPASS_UPDATE_FEED_URL` at a local
`tiny_http` feed or a closed local port, and an engine built around a test's index has a feed that
is never reached (`Updates::offline`).

What differs from `UpdateService`:

| Row | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| `update` | Checks only where it can install (`checksSupported` needs the installer): macOS; on Linux the `NullUpdateInstaller` turns checking off. | Checks on Linux, installing nothing (`ReleaseCheck`): any newer published release is an offer, whatever its assets. | `a_release_check_offers_any_newer_published_release_without_an_asset` |
| `update` | The feed is `vicinaehq/vicinae`'s releases. | `tuna-os/compass`'s, compared against Compass's own version (`v` and `CARGO_PKG_VERSION`); `COMPASS_UPDATE_FEED_URL` and `COMPASS_UPDATE_VERSION` still override both. | `compass_reads_its_own_releases`, `a_newer_release_is_offered_once_checked_and_skipping_it_is_remembered` |
| `update` | Checks at start-up and every six hours on a timer, whether or not anyone looks; nothing is kept between runs but the skipped version. | Checks when the launcher opens and the last check is six hours old or more; the last answer (and when it was asked) is kept in `$XDG_CACHE_HOME/compass/latest-release.json`, so a restarted engine does not ask again, and a failed check waits six hours too, keeping the release from the one before. An engine nobody opens a launcher on never asks. | `a_newer_release_is_offered_and_asked_for_once_in_six_hours`, `an_unreachable_feed_keeps_the_last_answer_and_waits_six_hours` |
| `update` | No switch. | `launcher.check_for_updates` (on by default, on the settings view's General page): off, nothing is asked. | `checking_off_asks_nothing`, `with_update_checks_off_the_feed_is_never_asked` |
| `update` | The Update row's primary action installs (Install Update); View Release Notes second; Skip This Version (`Keybind::RemoveAction`) toasts "Skipped %1". | View Release Notes is primary (Enter), Skip This Version on Ctrl+X; the row going is the answer, without a toast (the launcher has none that leaves it open). The title says "Compass %1 is available". | `a_newer_release_leads_the_empty_query_and_its_panel_opens_or_skips_it` |
| `update` | The tray's "Check for Updates…" item. | Not offered: Compass has no tray icon of its own (`src/services/tray`). | — |
| `builtins/vicinae` | Forget Past Vicinae Telemetry. | Not offered, and never will be: Compass sends none. | `the_vicinae_extensions_commands_keep_their_cpp_ids` |
| `global-shortcuts`, `window-manager` | X11 backends (`x11/`). | None: Compass is Wayland only. | — |

### Gaps closed after the truth pass (2026-09-25)

Genuine gaps from PLAN §12.0's list, closed one commit each against the C++ in
`src/server/src/services` and `builtins`. The same rule as the truth pass: a cell flips only with a
named Rust module and named tests that fail on a regression.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/builtins/system` | Rust ✅ | Browse Apps (`compass_ui::apps_page` over `compass_core::browse_apps`, with `AppIndex::hidden_applications` for `showHidden` and `BuiltinCommand::default_disabled` for `isDefaultDisabled`); Set Default Browser and Set Default Terminal (`compass_core::default_app`'s pickers, served by the engine over `Request::{ListDefaultApps, SetDefaultApp}`, IPC v17, writing through `compass_xdg::{mimeapps_writer, terminal}`) | `browse_apps_lists_filters_opens_and_copies`, `a_default_picker_lists_the_engines_candidates_and_sets_the_chosen_one`, `the_default_browser_and_terminal_are_listed_and_set_in_the_users_files` (a real engine, temp XDG dirs), `the_list_hides_no_display_entries_unless_asked_and_sorts_on_request`, `browse_apps_is_disabled_until_the_configuration_enables_it`, `apps_page::tests` |
| `src/services/desktop-notification` | Rust ✅ | `compass::notification_icon`: a builtin icon drawn from its SVG with `resvg` (already in the tree through iced) into a 128×128 `vicinae-notif-*.png` in the temporary directory, tinted when asked; a remote image fetched through `compass_ui::remote_image`'s cache; an SVG file drawn, a PNG or JPEG passed; the fallback when the source cannot be drawn | `a_builtin_icon_is_drawn_into_a_tinted_square_png` (decodes the PNG and checks its size, centring and tint), `a_remote_image_is_fetched_and_a_file_is_passed_or_drawn` (a fake fetch; no network), `a_notification_carries_the_urgency_and_an_icon_file` |
| `src/services/extension-registry` | Rust ✅ | `compass::catalog_watch::watch_extensions` (debounced 100 ms as the registry's `m_rescanDebounce`), `EngineState::rescan_extensions`, `AppIndex::extension_dirs`; the window follows through the catalog generation, as for applications | `an_extension_built_into_place_while_the_engine_runs_joins_root_search` (a real engine over temp XDG dirs: the directory, then the manifest, then removal), `only_an_entry_of_an_extension_directory_or_a_manifest_matters` |
| `src/services/app-service` | Rust ✅ | `compass::catalog_watch` (the directory watch, debounced 500 ms as `m_rescanDebounce`), `AppIndex::{rescan_applications, replace_applications}`, `EngineApps::{web_browser, file_browser, text_editor, terminal_emulator, set_web_browser}`; the window rescans its own copy when `Request::CatalogGeneration` (IPC v17) moves | `an_application_installed_while_the_engine_runs_is_found_without_a_restart` (a real engine over temp XDG dirs), `a_rescan_takes_installed_and_removed_applications_and_keeps_the_rest`, `a_burst_of_changes_is_one_change_and_an_ignored_path_is_none`, `a_moved_catalog_generation_rescans_and_the_same_one_does_not`, `the_browser_file_manager_and_editor_are_the_defaults_then_the_category_then_a_claim` |

What differs, by row:

| Row | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| `app-service` | `QFileSystemWatcher` on each application directory itself, so a file added in a subdirectory (`applications/kde4/`) waits for the next change at the top. | The watch is recursive, as the scan it triggers is. | `a_burst_of_changes_is_one_change_and_an_ignored_path_is_none` |
| `builtins/system` | Browse Apps' panel starts with Focus Window when the application has a window open. | The same since "The gaps pass, root and actions". | `browse_apps_offers_focus_window_first_and_reads_its_preferences_on_opening` |
| `builtins/system` | Browse Apps reads `showHidden` and `sortAlphabetically` each time it opens. | The same since "The gaps pass, root and actions". | `browse_apps_offers_focus_window_first_and_reads_its_preferences_on_opening` |
| `builtins/system` | Unsorted, the list is `m_apps` in scan order, hidden entries among the rest. | Unsorted, the shown applications in scan order, then the hidden ones. | `the_list_hides_no_display_entries_unless_asked_and_sorts_on_request` |
| `builtins/system` | The current default carries a green check icon. | The same icon (`CheckCircle` in green) since "The gaps pass, icons and tray"; the `✓ Default` text only where the builtin icon set is not installed. | `the_default_pickers_mark_is_the_green_check_icon` |
| `desktop-notification` | Every icon is rendered to a 128×128 PNG, with the theme's side of a themed image, and a file icon or `data:` URL drawn as the launcher would. | Since "The gaps pass, icons and tray" every source is: a PNG or JPEG is fitted into the 128×128 PNG, a file icon and a `data:` URL drawn. A file that does not decode is still passed as it is, for the server to try; a themed image uses its light side (a notification has no theme). | `a_remote_image_is_fetched_and_a_file_is_passed_or_drawn`, `a_data_url_is_decoded_and_drawn_into_the_square`, `a_file_icon_is_the_themes_mime_icon_or_the_builtin_document` |
| `extension-registry` | `QFileSystemWatcher` on each extension directory: an extension appearing is seen, a `package.json` written into it afterwards is not, so `vicinae develop` (which creates the directory before building) waits for the next change or its own deeplink. | Each extension's directory is watched too, for its `package.json` only; a bundle being written is not a rescan. | `an_extension_built_into_place_while_the_engine_runs_joins_root_search` |
| `app-service` | One process: `appsChanged` reloads the root items the window shows. | Two: the engine rescans on the watch; the window asks for the catalog generation on every summon and rescans its own index when it moved, so an open window catches up on its next summon. | `a_moved_catalog_generation_rescans_and_the_same_one_does_not` |

### The gaps pass, icons and tray (2026-09-25)

Three gaps from PLAN §12.0, ported from `src/server/src/ui/image`, `services/ui` and
`services/tray-host`, one commit each (IPC v18 for the tray). The rule is the truth pass's: a cell
flips only with a named Rust module and named tests that would fail on a regression.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `ui/image` | none (see its note) | `compass_ui::icons::{Glyph, command_glyph, file_glyph, clipboard_glyph, default_mark, FileGlyphCache, tile_tone, on_tile}`; `compass_core::commands::{Tile, CommandKind::tile, CommandKind::badge}`; `LauncherApp::glyph` draws a builtin in the row's colour, or on the command's tile (`applyBackdrop`'s rounded square, the glyph inset 19% in `getTonalContrastColor(tile, 5, 0.1)`) with `applyBadge`'s black disc | `a_builtin_command_draws_its_tiled_icon_and_without_the_set_its_initial`, `search_files_rows_draw_their_file_type_icons`, `a_window_row_draws_its_applications_icon_or_the_app_window_builtin`, `the_default_pickers_mark_is_the_green_check_icon`, `clipboard_rows_draw_the_builtin_for_their_kind`, `icons::tests::{a_file_takes_its_mime_icon_then_the_generic_one_then_a_builtin, a_command_is_drawn_on_its_tile_with_a_light_glyph, a_grey_tile_stays_grey_and_accent_follows_the_palette, clipboard_rows_and_the_default_mark_use_the_cpps_builtins, file_glyphs_are_resolved_once_per_path}`, `each_command_keeps_the_cpps_tile_and_badge` |
| `src/services/tray-host` | Rust ✅ | `compass::tray_host::TrayHost` over the `system-tray` crate (its `StatusNotifierWatcher` when none owns the name, host registration, item and menu tracking), started with the engine; an item's own object path read back from the watcher for `Activate`/`SecondaryActivate`; `IconThemePath` searched (`find_in_theme_path`, with `compass_core::tray_host::best_icon`); the largest pixmap made a PNG (`pixmap_png`). Served over IPC v18 `TrayItems`, `TrayActivate`, `TrayMenu`, `TrayTriggerMenu`; drawn by Search Tray (`commands:search-tray`, `compass_ui::{tray_page, app::tray}`) with the C++ panel (Activate, Browse Menu, Secondary Activate; a menu-only item only browses), the Attention accessory, and the flattened menu (`compass_core::tray_host::flatten_menu`) whose toggles keep it open | `the_tray_host_lists_activates_and_browses_another_applications_item` (a private `dbus-daemon`, a fake item at a non-default path with a `dbusmenu`), `with_no_session_bus_the_tray_is_refused_by_name`, `tray_host::tests::{an_items_path_is_read_back_from_the_watchers_list, the_largest_pixmap_becomes_a_png_with_its_channels_reordered, an_icon_shipped_in_the_items_theme_path_is_found_there_svg_first}`, `search_tray_lists_activates_and_browses_an_items_menu`, `tray_page::tests::*`, `a_tray_menu_is_flattened_with_its_submenus_labels`, `a_tray_rows_title_falls_back_to_its_id_and_its_subtitle_to_the_tooltip_body` |
| `src/services/desktop-notification` | already ✅; its declared difference closed | `compass::notification_icon`: a `data:` URL decoded with `data-url` (already in the tree through usvg), a file icon through `compass_ui::icons::file_glyph`, and a PNG or JPEG fitted into the 128×128 PNG with `image` | `a_data_url_is_decoded_and_drawn_into_the_square`, `a_file_icon_is_the_themes_mime_icon_or_the_builtin_document`, `a_remote_image_is_fetched_and_a_file_is_passed_or_drawn` |

What differs, by row:

| Row | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| `ui/image` | `renderFileIcon` asks `QMimeDatabase` (`MatchDefault`: the name's globs, then the content's magic) and the type's `iconName` and `genericIconName`, which the shared-mime-info database may override per type. | The type by extension (`mime_guess`, already in the tree), `inode/directory` for a directory; the icon names by shared-mime-info's defaults (`image/png` → `image-png`, generic `image-x-generic`; a directory's generic `folder`). An extensionless file is `application/octet-stream` rather than sniffed, and a type's own `<generic-icon>` is not read. | `a_file_takes_its_mime_icon_then_the_generic_one_then_a_builtin`, `mime_icon_names_follow_the_shared_mime_info_defaults` |
| `ui/image` | A tile is a vertical gradient with a hairline and a drop shadow under the glyph, the tile colour from the theme's semantic colours. | The gradient and the shadow since "The gaps pass, UI"; the tile colour is still the Vicinae dark theme's accents (the launcher's own palette carries only an accent), clamped into `clampTileTone`'s band. | `a_command_is_drawn_on_its_tile_with_a_light_glyph`, `a_tile_is_a_gradient_lighter_at_the_top_and_deeper_at_the_bottom` |
| `ui/image` | A clipboard link row shows the site's favicon. | The same since "The gaps pass, UI": the favicon, the builtin link icon until it has been fetched. | `a_favicon_is_fetched_once_into_the_cache_and_then_drawn` |
| `tray-host` | `SniWatcher` claims the watcher name only after a three-second grace, releases it when another connection queues for it, and accepts an item registered as `busname/path`. | The `system-tray` crate's watcher claims the name at once when it is free and keeps it; it accepts a bus name or an object path (what libappindicator and KDE send) and refuses the combined `busname/path` form. A desktop's own watcher (a bar, KDE, GNOME's AppIndicator extension) is used when it is already there. | `the_tray_host_lists_activates_and_browses_another_applications_item` |
| `tray-host` | An item is keyed by bus name and path, and its menu fetched with `GetLayout` when the view opens. | Keyed by the bus name it registered from, as the crate keeps it (one item per connection); the menu is the layout the crate follows through `LayoutUpdated`, after an `AboutToShow`. | `the_tray_host_lists_activates_and_browses_another_applications_item` |
| `ui/image` | Builtin icons are compiled into the binary as Qt resources. | Read from the installed `compass/builtin-icons` directory (`compass_core::builtin_icon::directory`); where it is missing a row keeps its initial, as before. | `a_builtin_command_draws_its_tiled_icon_and_without_the_set_its_initial` |

### The gaps pass: glyphs, clipboard, root (2026-09-25)

Three rows the truth pass left amber with nothing blocking them but the work. A cell flips only
with a named module and named tests that fail on a regression.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/services/glyph-service` | Rust ✅ | `compass_core::glyph_service` (file I/O, `score`), `compass_ui::emoji_page`, `compass_ui::app::emoji` | `tests/glyph_service.rs` (`the_cpp_file_is_read_with_its_camel_case_keys`, `the_file_is_written_and_read_back_and_a_missing_one_is_empty`, `a_visit_raises_a_glyph_among_matches_and_a_keyword_makes_it_match`), `emoji_page::tests` (pins and visits head the empty query, keywords, per-glyph tones, the panel), `the_picker_remembers_a_pick_a_pin_and_a_keyword_in_its_file` |
| `src/services/clipboard` | Rust ✅ | `compass_clipboard::retention`, `compass_clipboard::store::entry`, `compass::clipboard_service::{Settings, Control, run_eviction}`, `ClipboardStore::{evict, remove_all, detail, set_keywords, history_of_kind}` | `retention::tests`, `eviction_removes_what_is_older_than_the_threshold_and_reports_the_next`, `remove_all_spares_tagged_entries_when_asked_and_unlinks_the_rest`, `nothing_is_recorded_while_monitoring_is_off`, `the_preferences_are_read_with_the_cpp_defaults`, `pausing_the_clipboard_is_answered_and_kept_as_the_monitoring_preference` |
| `src/builtins/clipboard` | — (the open actions landed in the views pass; drag is declared there) | `compass_ui::clipboard_page`, `compass_ui::app::clipboard` | `clipboard_page::tests` (filter vocabulary, `format_size`, the pane's content, stale answers), `the_kind_filter_the_pane_keywords_remove_all_and_monitoring` |
| `src/builtins/root`, `src/services/root-item-manager` | — (the provider search view; per-item shortcuts and other fallbacks remain) | `compass_core::root_items::{apply_edit, deeplink}`, `Config::{favorite_ids, apply_root_edit}`, `root_view::SearchHistory`, `ClockConfig`, `compass_ui::app::root`, IPC `RootItemEdit` | `favouriting_inserts_first_and_moving_swaps_within_the_list_only`, `an_alias_and_the_switch_are_written_under_the_items_provider_and_merged`, `the_root_panel_writes_favorites_whole_and_an_items_alias_and_switch`, `the_search_history_keeps_one_of_each_newest_first_in_the_cpp_shape`, `the_clock_is_on_every_minute_in_hh_mm_unless_set`, `the_root_panel_favourites_aliases_and_the_up_arrow_recalls_searches` |

**`src/services/clipboard` → retention and monitoring.** The clipboard extension's preferences
(`providers.clipboard.preferences`) are read with the C++ defaults: `monitoring`,
`ignorePasswords` and `preserveTagged` on, `evictionThreshold` never, `eraseOnStartup` off. With a
threshold the engine sweeps after the C++'s one-minute misconfiguration grace, then each time the
oldest evictable entry comes due (`next_delay`: that entry plus the threshold plus a second, clamped
to one second and six hours), and — with nothing evictable — a threshold after the next copy, which
is when `armEvictionTimer(now)` re-arms in the C++. Each pass unlinks the payloads it removed. The
monitoring switch is the history view's status button (in the panel here); turning it off stops
recording rather than stopping the watcher, which is the same to anyone copying, and the choice is
written back as the `monitoring` preference, as `toggleMonitoring` patches it. `ignorePasswords`
decides whether a selection a password manager marked is left out (data-control; the GNOME path has
no such mark in either engine). `store-all-offerings` has no effect in the C++ (below).
Remove-all spares pinned and keyworded entries when `preserveTagged` is on, as
`removeAllSelections` does.

**`src/builtins/root` → the root view's behaviour, wired.** The empty query shows the favourites
first under **Favorites**, in the order arranged, then the rest under **Suggestions**, a favourite
not suggested twice (`queryFavorites`). Unset, `favorites` is the C++ default file's
`["clipboard:history"]`; C++ builtin ids (`clipboard:history`, `files:search`,
`core:search-emojis`) are read as the Compass commands they name. The row's panel is
`RootSearchActionGenerator`'s: the row's own actions, then Copy Deeplink
(`vicinae://launch/<provider>/<entrypoint>`), Reset ranking (asks first), Add to / Remove from
favorites, Move up / down in favorites (only where there is room), Set alias (the one-field form of
`AliasFormViewHost`), Copy ID and Disable item (asks first). The window applies each change at once
and the engine keeps it (IPC v17 `RootItemEdit`): favourites and alias and switch in
`compass.json`, the ranking in the launch history. A space typed after exactly the selected
command's alias opens it when the command opens a view. Up at the top of the list (navigation not
wrapping) walks back through past searches, kept in the C++'s own file,
`$XDG_DATA_HOME/compass/search-history.json` (`{"entries":[{"q","ts"}]}`, one of each, newest
first, 1000 at most), which a search is added to when a row runs from it. The clock shows under the
list in `launcher.clock`'s format (`hh:mm` unless set; `settings.json`'s
`launcher_window.clock` migrates to it), redrawn on multiples of its interval. The provider search
view, the other fallbacks and the completer branch of the space shortcut landed after it (see "The
gaps pass, root and actions").

**`src/builtins/clipboard` → the rest of the view.** The kind filter is a dropdown above the list;
it asks the engine for one kind (`ClipboardHistoryOfKind`, the query's `kind` filter), clears the
search text as `setKindFilter` does, and is remembered as `clipboard.filter` in the launcher's view
memory with the stored vocabulary (`image`, not `Images`). The detail pane beside the list follows
the selection; a late answer for an entry no longer selected is dropped. It shows a single local
file that exists as Search Files previews it, an image as copied, text and URI lists up to 10 KiB,
and the metadata `loadDetail` shows (type, MIME type, size in `formatSize`'s units, copied at, MD5,
encryption and keywords). Keyword editing is a one-field form over the entry's stored keywords
(Ctrl+E); remove-all asks first ("Are you sure?", Enter to delete all, Escape to keep); the panel
is `actionPanel`'s paste/copy, pin/unpin, edit keywords, remove, remove all, plus pause/resume.

**`src/services/glyph-service` → the emoji picker.** The picker reads and writes the C++'s own file,
`$XDG_DATA_HOME/compass/emojis/emojis.json`, so both engines remember the same visits, pins, tones
and keywords. That file turned up a bug: the port serialised `visit_count`, `pinned_at`,
`last_visited_at` and `skin_tone`, where glaze writes the members as declared (`visitCount`,
`pinnedAt`, …), so neither engine could read the other's file. The keys are camelCase now, and the
snake_case spellings are still read so a file the earlier build wrote is not lost. The empty query
shows the pinned glyphs, then the recently used, then the table under its category headings; a
query ranks by the person's keyword (twice the name's weight), the name, the CLDR keywords and the
category, plus the frecency boost of each glyph's visits. Copy registers a visit and copies the glyph
in its own tone, else the picker's `skinTone` preference
(`providers.core.entrypoints.search-emojis.preferences`). The panel is `buildEmojiActionPanel`'s:
copy, copy name, codepoint and category, the keyword form (Ctrl+E), reset ranking, pin or unpin,
and the skin-tone section. Not ported, and deliberately: the one-off migration from the legacy
`visited_emoji` table of `omni.db`, which only a database from before the JSON file holds; and the
picker's paste action, which landed after it (see "The gaps pass, root and actions").

### The gaps pass, root and actions (2026-09-25)

The rest of the root view, the action panel's shortcut recorder, the emoji picker's paste action
and Browse Apps' Focus Window, one commit each against `src/server/src/builtins/root`,
`ui/action-panel`, `builtins/vicinae` and `builtins/system`. A cell flips only with a named module
and named tests that fail on a regression.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/builtins/root`, `src/services/root-item-manager` | Rust ✅ (the per-item shortcut in the row below) | `compass_core::root_items::{parse_launch_link, LaunchLink::target}`, `AppIndex::{search_root_with, has_provider, provider_title}`, `compass_ui::app::{Fallback, ProviderScope}`, `compass_ui::app::root::{open_launch_link, open_provider_search, has_completer}`, `compass::serve::launch::open_launch_link` | `a_launch_link_names_a_provider_or_an_item_with_its_text`, `a_launch_deeplink_to_a_provider_searches_its_items_alone`, `a_launch_deeplink_to_an_item_launches_it_with_its_text`, `fallbacks_open_a_one_argument_shortcut_and_an_extension_with_the_query`, `an_alias_and_a_space_open_an_items_arguments` |
| `ui/action-panel`, and the per-item shortcuts of `src/builtins/root` and `src/services/root-item-manager` | Rust ✅ | `compass_core::key_combo` (`Keyboard::Shortcut`'s spelling and parser, the capture's chord tracking, `shortcut_conflict::validate`), `compass_ui::shortcut_recorder`, `compass_ui::app::root::recorder_event`, `RootEdit::Shortcut` over IPC v18 `RootItemEdit::Shortcut`, `Config::apply_root_edit`, `RootItem::merge_config` | `a_combination_is_stored_in_the_cpps_spelling_and_read_back`, `a_recording_is_a_key_with_modifiers_or_modifiers_released_alone`, `a_combination_needs_a_modifier_and_must_not_be_anothers`, `the_badge_names_the_modifiers_then_the_key`, `shortcut_recorder::tests` (four), `the_root_panel_records_an_items_shortcut_and_backspace_removes_it`, `a_root_items_shortcut_is_written_in_the_cpps_spelling_and_cleared` |
| `src/builtins/vicinae` | — (the picker half is done; the other views remain) | `compass_ui::app::emoji::{paste_selected_emoji, emoji_pasted}`, `EmojiPage::{supports_paste, default_action}` over `compass_core::emoji_grid::main_actions`, `compass::serve::paste_text` over IPC v18 `Request::PasteText` | `the_picker_pastes_the_glyph_and_copies_where_the_engine_cannot`, `window_requests_without_a_session_bus_are_refused_by_name` |
| `src/builtins/system` | — (already Rust ✅; two declared differences closed) | `compass_ui::app::apps::{open_browse_apps, apps_runtime_task}`, `AppsPage::running`, `AppFlags::config_path`, over `compass::serve::app_runtime` (IPC v17 `AppRuntime`) | `browse_apps_offers_focus_window_first_and_reads_its_preferences_on_opening` |

**The provider search view (`ProviderSearchViewHost`).** `vicinae://launch/<provider>` — the link
`compass deeplink` sends and a desktop shortcut can carry — opens root search over that provider's
items alone (`search(text, {.providerId})`), every one of them for the empty query, with no
favourites, calculator or fallbacks, the field reading `Search <provider>` and the link's
`fallbackText` typed in. Leaving it closes the window, as the deeplink's `setInstantDismiss` does.
`vicinae://launch/<provider>/<entrypoint>` launches the item as `cmd launch` does, with
`fallbackText` as its query; `toggle=true` hides an open window instead; a path that names no
provider and has no `/` is refused with the C++'s "Invalid format for launch deeplink". The engine
reads the link (`Request::OpenDeeplink`, no new variant) and hands the window only what is the
window's.

**Fallbacks (`RootFallbackSection`).** Every `fallbacks` entry the C++ would offer
(`isSuitableForFallback`): Search Files, any extension command, and a quicklink with exactly one
argument. An extension command is launched through the engine with the query as its fallback text
(`OpenBuiltinCommandAction::setForwardSearchText`), the way `cmd launch --query` reaches it; a
quicklink opens with the query as its argument (`OpenShortcutFromSearchText`).

**The completer branch of the space shortcut.** An item that takes arguments (a quicklink, an
extension command or a script command with any) opens its arguments form when its alias is typed
and then a space, where the C++ focuses the search bar's completer; with nothing yet typed into the
form, which is the C++'s "every completion value empty" condition.

**The shortcut recorder (`SetRootItemShortcutAction`, `ShortcutRecorderPanelView`).** The root
row's panel offers Set Global Shortcut after Set alias. It turns the panel into the recorder: the
item's title, the shortcut it has (or the chord being held), and a status line. A key with a
modifier, a function key, or modifiers pressed and let go on their own is a combination
(`handleKey`); a bare key is refused with "Modifier required", and one another root item has with
`Already bound to "<title>"`. An accepted one is written as `Shortcut::toString` spells it
(`super+control+alt+shift+KEY`) to `providers.<p>.entrypoints.<e>.shortcut` in `compass.json`, and
the panel closes; Escape goes back to the actions; Backspace, while the item has a shortcut,
removes it. Binding the shortcut to the desktop is `src/services/global-shortcuts`' row, which is
where it stays: the configuration is written as the C++ writes it, so either engine binds it.

**The emoji picker's paste (`PasteToFocusedWindowAction`).** The panel is `buildEmojiActionPanel`'s
with paste in it: Paste to active window and Copy, in the order the `defaultAction` preference
(`providers.core.entrypoints.search-emojis.preferences`, `paste` unless set) puts them, the first on
Enter. Pasting counts a visit, as copying does, and hands the glyph in its tone to the engine
(`Request::PasteText`), which puts it on the clipboard and pastes it through the Shell extension
into the window the launcher hides back to, as it pastes a clipboard entry or a snippet. Where the
engine cannot paste (no Shell extension and, on wlroots, neither the input server nor a virtual
keyboard; or a missing session bus) it refuses by
name and the window copies the glyph instead, which is where the C++'s `pasteContent` leaves it too:
copied first, then "the current platform cannot paste".

**Browse Apps: Focus Window, and the preferences read on opening.** As the selection moves (and
when the view opens) the window asks the engine whether the selected application runs
(`Request::AppRuntime`, the app runtime's `isRunning`); when it has a window open, the panel starts
with Focus Window, which Enter runs, raising its first window as `activeWindows.front()` does; an
answer for a row no longer selected is dropped, and one that arrives with the panel open joins it.
`showHidden` and `sortAlphabetically` are read from `compass.json` each time the view opens, as
`BrowseAppsView` reads them, rather than when the window starts; a file that cannot be read keeps
the ones the window started with.

**A regression found on the way.** The root row's panel (the first gaps pass) had taken over the
panel an application's row opens, so Quit, Force Quit, Focus Window and Close Window (the
app-runtime commit) never joined it. The engine is asked again when the root panel opens over an
application, and the running-only actions are added to the panel as it is, root actions and all
(`a_running_applications_panel_offers_quit_and_force_quit_and_they_reach_the_engine`).

What differs, by row:

| Row | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| `builtins/root` | The provider view ranks by visits as root search does. | It ranks in the window, without the engine's launch history: matches by score, the empty query in index order. | `a_launch_deeplink_to_a_provider_searches_its_items_alone` |
| `builtins/root` | The provider view carries the provider's icon as its navigation icon. | The field's placeholder names it; the launcher has no navigation title bar. | — |
| `builtins/vicinae` | Where the platform cannot paste, the picker offers no paste action and `defaultAction` defaults to copy. | The window cannot know before asking, so paste is offered whenever an engine is attached, and a refusal copies; the result is the same glyph on the clipboard. | `the_picker_pastes_the_glyph_and_copies_where_the_engine_cannot` |
| `builtins/vicinae` | The paste action is titled `Paste to <frontmost app>` with its icon. | `Paste to active window`, the C++'s title when no application is frontmost. | `the_picker_pastes_the_glyph_and_copies_where_the_engine_cannot` |
| `ui/action-panel` | Action titles mix sentence case ("Reset ranking", "Add to favorites", "Disable item") and title case ("Copy Deeplink", "Set Global Shortcut", "Quit Application"); shortcut badges show the binding as stored (`ctrl+shift+c`). | Sentence case throughout, the case most of upstream's v0.29.0 action titles use (42 of 74 multi-word titles); acronyms and names keep their capitals ("Copy ID", "Open in Ptyxis"). Badges read `Ctrl+Shift+C`, `Enter` (`key_combo::badge`). The root search's status bar shows the primary action and "Actions Ctrl+B" beside the clock, as dmenu's does. | `a_badge_reads_the_same_whatever_the_spelling`, `the_root_footer_names_enter_and_the_action_panel` |
| `ui/action-panel` | Set Global Shortcut is offered only where `platform::supports(GlobalShortcuts)`. | Always offered: the shortcut is kept in the configuration either way, and the engine binds it where it has a backend ("The gaps pass, global shortcuts"). | `the_root_panel_records_an_items_shortcut_and_backspace_removes_it` |
| `ui/action-panel` | The capture suspends the global shortcuts and inhibits the compositor's while it records. | Both: the global shortcuts are suspended (IPC v20 `ShortcutCapture`, "The gaps pass, global shortcuts") and the compositor's inhibited under the layer-shell presentation ("The gaps pass, wlroots paste and inhibit"); under `xdg_toplevel` the compositor's are not inhibited. | `the_recorder_suspends_the_global_shortcuts_while_it_captures`, `the_root_panel_records_an_items_shortcut_and_backspace_removes_it` |
| `ui/action-panel` | "Already bound" also checks the launcher's own keybinds (`KeybindManager`). | Checked, as far as Compass has them: its keys are fixed (Toggle action panel, Open settings, Quick launch), and the launcher hotkey with them. | `the_recorder_refuses_the_launchers_own_keys`, `the_launchers_own_keys_and_its_hotkey_are_taken` |
| `root-item-manager` | Clearing a shortcut writes `""`, which comes back as an empty shortcut after a restart. | Cleared as absent, and an empty stored one reads as none (see "`compass-core::root_items`"). | `a_root_items_shortcut_is_written_in_the_cpps_spelling_and_cleared` |

### The gaps pass (2026-09-25)

Genuine gaps from the list above, closed one row at a time, each flip with the module that does it
and the tests that fail on a regression (IPC v17).

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/builtins/calculator` | Rust ✅ | `compass_ui::{calculator_page, app::calculator}`, `compass::serve::calculator` over `compass-local-storage::calculator` | `copied_answers_are_remembered_grouped_filtered_pinned_and_removed`, `the_boundaries_are_the_cpps_monday_weeks_and_calendar_months`, `a_copied_answer_is_remembered_and_calculator_history_lists_pins_and_removes_it`, `the_live_result_leads_and_a_stale_answer_is_dropped`, `calculator_history_is_a_builtin_and_without_a_keyring_is_refused_by_name`, `conversions_are_told_from_arithmetic_by_their_keyword` |
| `src/services/app-runtime` | Rust ✅ | `compass::serve::app_runtime` (`isRunning`, `frontmostApp`, `quit`, `forceQuit`), `compass_ui::app::runtime` (the root row's and the window switcher's actions) | `quit_closes_an_applications_windows_and_force_quit_kills_their_processes` (a private `dbus-daemon`, the mock Shell extension, and `sleep`s the test started), `force_quit_kills_each_process_once_and_closes_the_windows_that_name_none`, `killing_a_process_this_test_started_ends_it_with_sigkill`, `a_running_applications_panel_offers_quit_and_force_quit_and_they_reach_the_engine`, `the_window_switchers_panel_quits_a_known_windows_application`, `a_quit_that_does_nothing_says_so` |
| `src/cli` | Rust ✅ | `compass::{cli, cli_commands, logs}`, `serve::launch`; `compass_core::{script_template, extension_commands::launch_arguments, config::default_document, file_search::CLI_CATEGORY_NAMES}` | `cmd_ls_lists_every_root_item_by_id_and_cmd_launch_hands_the_window_a_launch`, `app_launch_and_cmd_launch_start_the_application_with_its_arguments`, `state_open_asks_the_window_and_exits_by_the_answer`, `the_engine_keeps_a_log_file_and_logs_prints_its_last_lines`, `fs_query_asks_the_index_alone_and_names_categories_as_the_cpp_does`, `server_refuses_a_running_engine_and_replace_kills_it_and_serves_in_its_place`, `config_cli::{script_template_*, theme_template_check_and_paths, config_default_*, version_*}`, `positional_launch_arguments_are_checked_as_the_cpp_checks_them`, `a_command_line_launch_opens_the_builtin_and_types_its_fallback_text`, `describe_answers_whether_the_window_is_open_and_changes_nothing` |

**`src/cli`.** Every subcommand `CommandLineInterface::execute` registers now has a counterpart:
`version` (`ver`), `server`, `ping`, `query`, `toggle`/`open`/`close`, `cmd ls` (`list`, `--json`)
and `cmd launch` (positional arguments, `--cwd`, `--query`), `deeplink` (`link`), `dmenu`, `theme
set`/`template` (`th`, `tmpl`), `fs query` (`q`, `--limit`, `--category`, `--json`), `app launch`
(`--new`), `config default`, `script template`/`check`, `state open` and `logs` (`-n`, `--follow`).
The engine answers the five that need it with IPC v17's `ListCommands`, `LaunchCommand`,
`LaunchApp`, `DescribeWindow` (the window answers a new `WindowCommand::Describe` without changing)
and `FsQuery`. `cmd launch` checks its arguments with the C++'s `buildLaunchArguments` sentences,
launches an application itself, and hands anything else to the window as a launch, whose
`--query` the window types into the view it opens (`Response::CommandLaunch`). `app launch` focuses
the application's first window unless `--new`, matched by `findAppWindows`'s class-or-title rule,
and launches it with its arguments otherwise. Declared differences:

- `logs` reads `$XDG_STATE_HOME/compass/compass.log`, which the engine writes (rotated to `.1` past
  five mebibytes, as the C++'s), not `vicinae.log`: two engines appending to one file would
  interleave, and ADR-0017 gives Compass its own files. The file is opened once the socket is bound,
  so a refused second engine writes nothing.
- `server` starts `serve` in the foreground (`start`, the engine and its window, with `--open`);
  `--config` reaches the engine as `COMPASS_CONFIG` and `--no-extension-runtime` as
  `COMPASS_NO_EXTENSION_RUNTIME`, which makes extension commands refuse by name. `--replace` sends
  `SIGKILL`, as the C++ does.
- `fs query --json` prints each file's `path` and `category` (the C++'s category names); the score
  and MIME type the C++ adds are not on the wire.
- `cmd launch --cwd` is carried to the engine and logged; no Linux command reads a working
  directory from its launch.
- `config default` prints this engine's `compass.json` at its defaults, read from the schema's
  `default`s, rather than the C++'s `settings.json` template.
- `theme check` and `theme paths` are registered in the C++ but commented out; here they work.
- `version`'s commit and provenance come from `COMPASS_GIT_COMMIT` and `COMPASS_PROVENANCE` at build
  time, `unknown` and `local` without them.

**`src/builtins/calculator`.** Calculator History is a builtin (`commands:calculator-history`),
served by IPC v17 `CalculatorHistory`, `AddCalculatorRecord` and `EditCalculatorHistory`. The rows
live in the `calculator_history` table of Compass's own encrypted database (the one extension
storage uses, keyed from the login keyring); without a keyring the history is refused by name.
Copying the calculator's answer, from the root list or the view's live result, remembers it, as
`CopyCalculatorAnswerAction`; the engine groups by the C++'s boundaries read from the local calendar
(Monday weeks, calendar months and years) and drops the empty groups; the view shows the live result
first under the C++'s `live_calc` gate and offers the C++ panel (pin or unpin, copy answer, question,
or both, delete, delete all). Declared differences: a row's conversion flag comes from the question's
`to`/`in`/`as`/`->` keyword, since fend reports no answer type; "Delete all entries" deletes, where
the C++ action's `execute` is empty; pinning and removing say so in the view rather than a toast.
The answer in root search offers v0.29.0's `RootCalculatorSection::actionPanel` (Copy Result, Copy
Question And Answer, Put answer in search bar, Open Calculator History;
`compass_core::calculator_history::live_action_panel`), without "Copy unformatted answer", which the
C++ adds only when its backend returns a second, unformatted form: fend prints only one.
Copying shows the C++'s HUD ("Answer copied to clipboard", "Copied to clipboard"; see "The gaps pass,
HUD and onboarding").
Currency conversion and Refresh Exchange Rates stay unported, blocked on a rate source.

**`src/services/app-runtime`.** `LinuxAppRuntime` over the engine's window providers (IPC v17
`AppRuntime`, `QuitApp`, `QuitWindowApp`): an application is running when `findAppWindows` finds it
a window (by `StartupWMClass` or desktop id, or a title that is its name), frontmost when one of
those has focus; Quit closes every one of its windows; Force Quit sends `SIGKILL` once to each
process that owns one and closes the windows that name no process; either is refused ("Failed to
quit Files") only when it did nothing. The pids come from the GNOME Shell extension and, on
wlroots, from Hyprland's or niri's IPC; a toplevel with no pid is closed instead, as the C++ does.
The root row's panel opens at once and gains Focus Window, Close Window, Quit Application (shown as
Ctrl+Q) and Force Quit Application when the engine says the application runs, as
`AppRootItem::newActionPanel`; the window switcher gets the C++'s panel (Focus Window, Close Window
on Ctrl+Q, and Quit and Force Quit for a window whose application is known). Neither asks first,
as the C++ does not; success hides the launcher with the C++'s HUD ("Quit Files", "Force quit
Files"). Declared differences: Ctrl+Q is shown beside Quit but, as with every
builtin panel here, the chord is not bound yet, so Quit runs from its row; the pin-window and bring-to-workspace actions are not offered, no provider here
having the capability. `frontmost` is answered but nothing reads it yet: its C++ reader is the
global-shortcut inhibition, which is that row's gap.

### The gaps pass, views (2026-09-25)

The builtins' remaining views, from PLAN §12.0, one commit each against the C++ in
`src/server/src/builtins` (IPC v18). A cell flips only with a named module and named tests that
fail on a regression.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/builtins/shortcut` | Rust ✅ | `compass_ui::{open_with_page, app::open_with}` (the app-selector), `compass::serve::openers` over `EngineApps` (IPC v18 `ListOpeners`, `OpenWith`), `compass_ui::shortcuts_page::{Detail, detail_fields, suitable_for_fallback}`, `compass_ui::app::shortcuts` (the pane, Open with…), `RootRow::ShortcutFallback` | `open_with_lists_what_opens_a_target_the_default_first` (a real engine over temp XDG dirs), `the_default_opener_comes_first_and_is_marked`, `open_with_launches_the_chosen_application_and_refuses_an_unknown_one`, `open_with_page::tests`, `manage_shortcuts_shows_the_detail_pane_and_opens_with_a_chosen_application`, `a_one_argument_shortcut_named_as_a_fallback_opens_with_the_query`, `the_pane_lists_what_load_detail_lists_in_its_order` |
| `src/builtins/file` | Rust ✅ | `compass_ui::app::file_actions` (`file_panel_sections`, the actions), `compass_ui::files_page::{runs_as_executable, FilesPage::searching}`, `compass::serve::files` (IPC v18 `FileActions`, `CopyFile`, `RunExecutable`, `SetWallpaper`), Open with… through `compass_ui::app::open_with` | `search_files_panel_is_the_cpps_file_actions`, `a_files_panel_learns_its_mime_type_and_an_appimage_is_made_executable_and_run` (a real engine; the AppImage is a script in the test's tempdir), `an_executable_is_given_the_owners_execute_permission`, `a_file_is_copied_as_its_escaped_file_uri`, `a_superseded_answer_is_dropped` |
| `src/builtins/clipboard` | Rust ✅ | `compass_ui::clipboard_page::{open_target, OpenTarget}`, `compass_ui::app::clipboard` (Open, Open with… through `compass_ui::app::open_with`) | `a_link_or_one_existing_file_is_what_open_acts_on`, `a_copied_link_opens_and_opens_with_a_chosen_application`, `plain_text_offers_no_open`, with the gaps pass's `clipboard_page::tests` and `the_kind_filter_the_pane_keywords_remove_all_and_monitoring` |
| `src/builtins/wm` | Rust ✅ | `compass_platform_linux::compositor` (`Provider::{capabilities, toggle_fullscreen, toggle_floating, toggle_overview}`), `compass::serve::workspaces`, `compass_core::window_switcher::command_offered`, `AppIndex::set_window_capabilities`, `compass_ui::{workspaces_page, app::workspaces}` | `niri_toggles_fullscreen_floating_and_the_overview`, `hyprland_toggles_a_window_and_has_no_overview` (fake sockets replaying captured replies), `workspaces_count_their_windows_and_name_their_applications_once`, `a_toggle_acts_on_the_active_window_and_refuses_one_elsewhere`, `a_window_on_another_workspace_is_not_on_the_active_one`, `without_a_compositor_everything_is_refused_and_nothing_is_offered`, `a_window_command_is_offered_only_where_it_is_registered`, `workspaces_page::tests`, `workspaces_and_the_toggles_are_offered_where_the_compositor_has_them`, `a_refused_toggle_says_why` |

**`src/builtins/shortcut`.** "Open with…" is a view of its own, the app-selector the
`ui/views` row names: the applications that open a target, the default first and marked, filtered
fuzzily as typed; Enter opens the target with the chosen one and hides, Escape goes back to the
view it was opened over. The C++ has it as a panel submenu (`OpenWithAction`,
`OpenCompletedShortcutWithAction`); a view gives it the same search and keys as every other list
here. Search Files and clipboard history open the same view. The shortcut panel (root row and
Manage Shortcuts) offers it after Open (Ctrl+O): the openers of the stored link, the link expanded.
Manage Shortcuts' detail pane shows the link expanded and `loadDetail`'s metadata: Name,
Application (with `(Default)` for the default opener), Opened, Last Opened (`Never`), Created at,
dates as `QDateTime::toString()` writes them. Shortcut fallbacks: see the shortcut table, rows 5, 6
and 8.

**`src/builtins/file`.** Search Files' panel is `FileActions::actionPanel`'s, in its order: Open
(when an application opens the file), Run executable (an AppImage, made executable first, primary
when nothing opens it), Show in file browser (Ctrl+Enter), Open with… (Ctrl+O, the app-selector),
Set as wallpaper (an image, where a wallpaper backend answers; Ctrl+Shift+W), Create shortcut (the
form with the file's name and path); then Paste to active window (where the engine can paste),
Copy file (a `text/uri-list`, Ctrl+Shift+C), Copy file path, Copy file name and Copy mime type. The
panel asks the engine first (`FileActions`: the MIME type, an opener, the wallpaper backend, paste),
then opens. "Searching…" shows beside the category filter while a query is out, as `setLoading`.
Declared differences:

- The file browser action is always offered; the C++ offers it only when a file browser is
  installed.
- Paste is offered where the engine can press it: the GNOME Shell client, or on wlroots the input
  server or a virtual keyboard ("The gaps pass, wlroots paste and inhibit").
- Success hides the launcher with the C++'s HUD ("Wallpaper set", "Copied to clipboard");
  failures show under the list rather than as a toast.
- Dragging a file out of the list: Iced offers no drag out of a window (`src/builtins/clipboard`
  shares this).

**`src/builtins/clipboard`.** A link, or a copy of exactly one file that still exists (its
`file://` URI decoded as `QUrl::path` decodes it), offers Open (Ctrl+O) and Open with…
(Ctrl+Shift+O, the app-selector) after the copy and paste actions, as `actionPanel` adds them.
Declared differences: Open is offered whether or not a default application claims the target (the
C++ adds it only then); without one the engine's refusal shows under the list. The target is read
from the detail pane's content, so the two appear once the pane has loaded. Dragging an entry out
stays unported: Iced has no drag out of a window.

**`src/builtins/wm`.** Switch Workspaces, Toggle Fullscreen, Toggle Floating and Toggle Overview
are builtins, offered in root search only where `WindowManagementExtension` registers them: the
window asks the engine what the compositor can do (IPC v18 `WindowManagerCapabilities`) each time
it opens, and until it hears, offers none of them, as the C++'s dummy window manager does. Hyprland
has workspaces, fullscreen and floating; niri those and the overview. Switch Workspaces lists the
compositor's workspaces (`ListWorkspaces`) under "Open Workspaces", each with its window count and
monitor (`3 windows - DP-1`, `empty`) and the applications with a window on it, searchable by name,
monitor and application at the C++'s weights; Enter or the panel's "Switch to workspace" switches
(`FocusWorkspace`) and hides. A toggle (`ToggleWindowState`) acts on the window the person was in,
the launcher's own left out, and is refused with the C++'s sentences ("No window to fullscreen",
"No window to toggle", "Active window is not on the current workspace"); on success the launcher
hides. Declared differences:

- The C++ toggles `getFocusedWindowSync()`; here the target is the provider's frontmost window
  skipping the launcher (on Hyprland the most recent on the active workspace, on niri the focused
  one else the most recent), since niri reports no focused window while a layer surface has focus.
- An unnamed niri workspace is called by its number; the C++ shows an empty title.
- The monitor is shown whenever the compositor names one; the C++ shows it only when it matches a
  Qt screen's name.
- The applications on a workspace are its accessory as names, not icons.
- On GNOME the C++ lists workspaces through its Shell extension's `ListWorkspaces`; Compass's Shell
  extension has no such call, so GNOME offers Switch Windows only. That is a gap in the GNOME
  provider (`src/services/window-manager`), not in this view.
- Hyprland's classic dispatcher fallback for fullscreen is `fullscreen 0`, which acts on the active
  window: the classic form has no window argument.

### The gaps pass, UI (2026-09-25)

The rest of `ui/image`, the snippet view and the Markdown detail's images, against the C++ in
`src/server/src/ui/image`, `src/server/src/favicon`, `builtins/snippet` and `utils/placeholder.cpp`
(IPC v19). A cell flips only with a named module and named tests that fail on a regression.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `ui/image` | Rust ✅ | `compass_core::image_url::{ImageUrl::from_source, SourceLookup}` (`ImageURL(const ImageLikeModel &)` for a bare string: an `icon://`, `file:`, `data:` or `http(s):` URL, an emoji, a glyph of the table, a builtin, a file, an asset, a theme icon); `compass_core::favicon::Service` (`favicon_service`: `twenty`, `google`, `none`); `compass_core::extension_commands::ExtensionCommand::icon_url` (`ExtensionCommand::iconUrl`); `compass_ui::icons::{url_glyph, UrlLookup, remote_source, semantic_color, Glyph::Text, tile_gradient, apply_mask, rasterize, MaskedCache}`; `LauncherApp::{url_icon, warm_urls, root_icon_arrived}` with `shortcut_url` (`RootShortcutItem::iconUrl`'s purple tile) and `clipboard_url` (the favicon with the link builtin as fallback); script icons over IPC v19 `ScriptIcons` (`compass::scripts::Scripts::icons`); `Image.mask` read by `compass_worker_host::view_model` and kept per row (`ExtensionPage::mask`) | `a_bare_source_is_read_as_image_url_reads_one`, `a_remote_images_own_query_survives_the_round_trip`, `each_service_asks_for_the_cpps_url_and_none_asks_nothing`, `the_configuration_names_the_service_and_twenty_is_the_default`, `the_icon_is_the_commands_then_the_extensions_then_the_hammer`, `an_image_url_is_drawn_as_its_type_says`, `a_tile_is_a_gradient_lighter_at_the_top_and_deeper_at_the_bottom`, `a_circle_mask_clears_the_corners_and_keeps_the_middle`, `a_rounded_mask_rounds_a_quarter_of_the_side`, `a_masked_image_is_drawn_once_from_a_png_or_an_svg`, `extension_script_and_shortcut_rows_draw_their_icons_in_root_search`, `a_favicon_is_fetched_once_into_the_cache_and_then_drawn`, `a_bare_icon_string_is_an_emoji_a_theme_icon_or_an_asset_and_masks_are_kept`, `an_images_mask_is_kept_in_either_spelling`, `script_commands_are_scanned_searched_and_run_in_their_modes` (a real engine) |
| `src/builtins/snippet` | Rust ✅ | `compass_core::placeholder::{parse_snippet_text, parse}` (`PlaceholderString::parse`, with its backslash escape), used by `compass::snippets`, the save path's cursor count and `snippets_page::arguments_form`; the detail pane: `compass_ui::snippets_page::{Detail, detail_fields}`, `compass_ui::app::snippets::{snippet_detail_task, snippet_detail_pane}`, `compass::snippets::preview` over IPC v19 `PreviewSnippet` | `an_escaped_brace_is_text_and_not_a_placeholder`, `a_doubled_backslash_is_one_and_the_brace_after_it_opens_a_placeholder`, `another_escaped_character_loses_its_backslash_and_a_trailing_one_stays`, `without_a_backslash_it_reads_as_a_quicklink_does`, `an_escaped_brace_expands_as_a_brace`, `an_escaped_brace_asks_for_no_argument`, `a_preview_shows_a_shell_placeholder_instead_of_running_it`, `the_pane_lists_what_load_detail_lists_in_its_order`, `manage_snippets_shows_the_selected_snippets_detail_pane`, `snippets_are_imported_created_expanded_edited_and_removed` (a real engine) |
| `ui/bridges` | Rust ✅ | `ExtensionPage::{wanted_images, image_arrived, markdown_art}` fetch a detail's Markdown images through `compass_ui::remote_image`'s cache, drawn by the store page's viewer (`app::stores::StoreMarkdown`) | `a_details_markdown_images_are_fetched_and_drawn` |
| `ui/views` | Rust ✅ | `compass_search::term_ranges` (`MatchHighlighter`: each search word found literally, ignoring case and accents) with `compass_ui::clipboard_page::highlighted`, drawn as `rich_text` spans behind the accent at 35% in clipboard history's detail text; Markdown code blocks highlighted by their language (Iced's `highlighter` feature, syntect through `two-face`); an extension's grid drawn as a grid (`ExtensionPage::{grid_columns, grid_groups, section_columns, grid_step}`, `LauncherApp::extension_grid`), each section in its own columns, the arrows moving as `SectionGridModel::navigate*`; the edit-keywords view is clipboard history's and the emoji picker's keyword form | `every_occurrence_of_each_term_is_found_ignoring_case_and_accents`, `a_term_does_not_overlap_itself_and_overlapping_terms_merge`, `the_searched_words_are_marked_in_the_detail_text`, `a_code_block_is_highlighted_by_its_language`, `a_grid_moves_by_cell_and_by_its_sections_columns`, `an_extension_grid_is_drawn_as_tiles_and_the_arrows_move_by_cell_and_row`, `the_kind_filter_the_pane_keywords_remove_all_and_monitoring`, `the_picker_remembers_a_pick_a_pin_and_a_keyword_in_its_file` |

**`ui/image` → every `ImageURL` a root row carries.** An extension command's row draws the command's
icon from the extension's assets, else the extension's, else the hammer on a cyan tile; a script's,
what the engine resolved from its `@raycast.icon` (an emoji, a file beside the script, an `https`
image, else `code` on the accent tile); a shortcut's, the `ImageURL` it stored, a builtin on a
purple tile; a clipboard link, its site's favicon with the link builtin as the fallback. Each is
resolved once per URL from `update` (`warm_urls`), never in a draw. A remote image (an `https` icon,
a favicon through `favicon_service`'s service) is fetched once into `compass_ui::remote_image`'s
cache and the row redrawn when it lands; until then the URL's fallback, else the initial. An
extension's image string that is not a builtin is read as `ImageURL(source)` reads it, so an emoji
is drawn as text and a theme icon's name as that icon. `Image.mask` is honoured: the image is drawn
into pixels (`image` for PNG and JPEG, `resvg` for SVG, both already in the tree) and clipped as
`applyCircleMask` (the inscribed ellipse) and `applyRoundedRectMask` (a quarter of the shorter side)
clip it, antialiased; the result is kept per file, mask and tint. A command tile is
`applyBackdrop`'s: the vertical gradient (`shifted(tile, 0.025, -0.03, 0.10)` to
`shifted(tile, -0.015, 0.06, -0.05)`), the hairline, and the glyph's silhouette at 70/255 black,
3.5% of the side lower, under the glyph.

**`src/builtins/snippet` → the pane and the escape.** `\{` is a literal brace and `\\` one
backslash, as `PlaceholderString::parse` reads them, everywhere a snippet's text is parsed: copying
and pasting, the arguments form, the save path's `{cursor}` count and keyword expansion. Manage
Snippets shows `loadDetail`'s pane beside the list, following the selection (a late answer for
another row is dropped): the text expanded by the engine with `executeShell` off, so a shell
placeholder reads `$(code)` and nothing runs, then Type, Created at, Updated at (when edited),
Keyword and Apps.

**`ui/views` → highlighting and grids.** Clipboard history's detail text marks every occurrence of
each word of the search, as `ClipboardHistoryView` hands `searchTerms` to `TextViewer`'s
`MatchHighlighter`; a fenced code block in any Markdown the launcher draws (an extension's detail,
a store README, release notes) is coloured by its language. An extension's `Grid` was drawn as a
single-column list; it is a grid now, eight columns unless the grid or the section says otherwise,
Left and Right in reading order, Up and Down by the section's columns into the neighbouring
section's nearest row, wrapping only where navigation wraps.

**`ui/bridges` → a Markdown detail's images.** An extension's detail view asks for the remote images
its Markdown shows once, with its rows' images, and draws each where it stands once fetched, the
placeholder until then, as the store's README does.

What differs, by row:

| Row | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| `ui/image` | `FaviconService` keeps favicons in its own database and `favicon-data/`, and asks its service for 128 px (its fallback to smaller sizes is not connected). | The service's 128 px image through `compass_ui::remote_image`'s cache, as every remote image here is kept (ADR-0017); `none` fetches nothing and the fallback stays. | `a_favicon_is_fetched_once_into_the_cache_and_then_drawn`, `each_service_asks_for_the_cpps_url_and_none_asks_nothing` |
| `ui/image` | A masked image is clipped at the size it is drawn. | Drawn into at most 128 px, clipped, and scaled to the slot. | `a_masked_image_is_drawn_once_from_a_png_or_an_svg` |
| `ui/image` | An `ImageURL`'s `badge` is drawn on any icon. | On builtin commands' tiles, as before; a badge in a stored `ImageURL` is not drawn. | — |
| `ui/image` | `ImageURL(source)` tests a relative path against the working directory (`QFile(source).exists()`). | Only an absolute path is a file; a relative one is an asset or a theme name. | `a_bare_icon_string_is_an_emoji_a_theme_icon_or_an_asset_and_masks_are_kept` |
| `ui/image` | Remote icons are fetched by every build. | By the launcher `compass` starts (`AppFlags::remote_icons`); off in tests, which never reach the network. | `a_favicon_is_fetched_once_into_the_cache_and_then_drawn` |
| `ui/image` | `QUrl::toString()` escapes a name's `?`, `#` and `%` in an `icon://` URL. | The same (`ImageUrl::to_url`); before this pass an `https` image with a query string did not survive the round trip. | `a_remote_images_own_query_survives_the_round_trip` |
| `ui/views` | `TextViewer` scrolls to the first match. | The matches are marked; the pane is not scrolled to them. | `the_searched_words_are_marked_in_the_detail_text` |
| `ui/views` | Code blocks are coloured by KSyntaxHighlighting in the theme's semantic colours. | By syntect's grammars in the Base16 Ocean theme, which Iced's Markdown fixes. | `a_code_block_is_highlighted_by_its_language` |
| `ui/views` | A grid's cells follow its `aspectRatio`, `fit` and `inset`. | Square tiles, the content at 70% of the tile; the three are read but not applied. | `an_extension_grid_is_drawn_as_tiles_and_the_arrows_move_by_cell_and_row` |
| `ui/views` | A grid section's title stays pinned as it scrolls, and PageUp/PageDown jump by section. | The title scrolls with its cells; no section jumps. | — |
| `builtins/snippet` | The pane re-expands as argument values are typed into the search bar's completer. | Manage Snippets has no completer: arguments expand empty. | `manage_snippets_shows_the_selected_snippets_detail_pane` |
| `builtins/snippet` | The pane lists the keyword's applications as icons with their names as tooltips. | Their names, comma-separated. | `the_pane_lists_what_load_detail_lists_in_its_order` |

### The gaps pass, settings (2026-09-25)

The C++ settings window (`src/server/src/ui/settings`, `ui/windows/settings-window.*` and
`ui/qml/settings/*.qml`), against its models: `GeneralSettingsModel`, `ExtensionSettingsModel`,
`PreferenceFormModel`, `ProviderCommandModel`, `KeybindSettingsModel`, `SettingsSidebarModel` and
`SettingsController` (IPC v19).

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/server/src/ui/settings` | Rust ✅, parity ✅ | `compass_core::settings_catalog` (every setting's key, kind, default and C++ property; the C++ settings with no reader, declared; `parse_settings_link`), `Config::{get_path, set_path, set_provider_enabled}`, `RootEdit::Enabled`; `compass::serve::settings` (IPC v19 `SetSetting`, `SetProviderEnabled`, `RootItemEdit::Enabled`); `compass_ui::settings_page` over `compass_ui::settings::SidebarModel`, `compass_ui::app::settings_view` | `the_settings_view_writes_each_setting_and_switch_into_the_configuration` (a real engine over temp XDG dirs: settings written where the engine reads them, refusals write nothing, the switches change root search), `every_cpp_general_settings_property_is_ported_or_declared`, `a_default_the_schema_documents_is_the_same_here`, `applying_writes_the_key_the_engine_reads_and_keeps_the_rest`, `a_value_the_setting_does_not_take_is_refused_and_nothing_changes`, `each_control_writes_the_file_and_the_launcher_follows_at_once`, `the_extension_page_switches_aliases_and_records_shortcuts`, `the_hotkey_is_recorded_into_the_launcher_section`, `with_an_engine_the_engine_writes_and_a_theme_is_kept_or_put_back`, `a_commands_preferences_open_over_the_settings_and_go_back_to_them`, `a_root_rows_open_preferences_opens_the_settings_at_its_provider`, `open_settings_is_a_root_command_and_ctrl_comma_and_escape_leaves`, `a_deeplink_opens_the_tab_it_names`, `settings_page::tests`, `the_settings_switches_turn_an_item_and_a_provider_back_on` |

**What it is.** Open Settings (the vicinae extension's `settings` command, a root command here),
Ctrl+, from the root (`Keybind::OpenSettings`), a root row's Open Preferences
(`OpenItemPreferencesAction`, at the item's provider) and `vicinae://settings/open?tab=` (the
`settings` IPC command, `openTab`'s aliases `keybinds`, `shortcuts` and `extensions` included) open
the settings: the C++ sidebar (its five pages, a divider, the providers, filtered fuzzily by the
search field) and the selected page. The pages draw `settings_catalog`'s settings by kind — a
switch, a list, a number or text field kept on Enter, a folder list, the shortcut recorder for the
launcher hotkey, the theme list — and every control writes its dotted key through `SetSetting`,
which the engine checks against the catalogue, writes into `compass.json` (a file that does not
parse is left alone) and applies where it holds the value (the clipboard's preferences, Run
Terminal Program's default action, the input server, the result count). The window applies what it
holds itself: the navigation scheme and wrapping, quick launch, the layout preset, icons and tint,
the clock, the font, the theme (previewed, and put back when the engine refuses it), the power
confirmations and the emoji picker's preferences. A provider's page is `ExtensionSettingsModel`'s:
its switch (`SetProviderEnabled`), and for each item its switch (`RootItemEdit::Enabled`), alias,
recorded shortcut, the preferences form of an extension command (the existing form, returning to
the settings) and the builtin preferences that belong to it (`clipboard`, `files`, `snippets`,
Browse Apps, Run Terminal Program, Search Emojis, the power commands; the script directories on the
Script Commands page). About shows the version and opens the documentation and the bug tracker.

Declared differences:

- **A view of the launcher, not a second window.** `SettingsController::openWindow` opens an
  independent floating window; Compass shows the same sidebar and pages in the launcher card.
  A second Iced window would need the resident daemon's per-window views and a second surface on
  both compositor paths (layer shell and `xdg_toplevel`) for pages that are a sidebar and a form.
  Escape leaves the settings for the root search.
- **The C++ settings Compass has no reader for are not offered**, each listed with its reason at
  the foot of its page (`settings_catalog::NOT_IN_COMPASS`), rather than written to a file that
  would then look as though it honoured them (the rule `config_migration` follows): Close on
  Escape, Language, usage statistics, Font size, Icon Theme, Window material
  and opacity, Compact mode, Floating status bar, layer shell, client-side decorations and their
  rounding, border and shadow, native font rendering, Pop on backspace, Activate on single click,
  IME handling, Root file search, Favicon fetching, Encrypt sensitive data, and
  rebinding the launcher's keys (the Keybindings page lists the fixed ones).
- **Settings only Compass has are offered beside them**: quick launch, the result count, the clock,
  the colour scheme, the layout preset, application icons and translucency.
- **Clear the search on close** (`launcher.pop_to_root_on_close`, Vicinae's `pop_to_root_on_close`,
  off by default as in v0.29.0) clears the search text when the launcher hides. A view opened from
  the search closes on hide whatever it says; in Vicinae, with the setting off, the view stays open
  for the next summon.
- The launcher hotkey and Close on focus loss are written to `launcher.hotkey` and
  `launcher.close_on_focus_loss`, the schema's keys; the engine binds the hotkey from the file and
  rebinds it when it changes, and the window hides on focus loss when the switch is on ("The gaps
  pass, global shortcuts").
- The font is a text field (empty for the desktop's interface font), where the C++ has a list of
  the installed families; Browse Fonts' "Set as vicinae font" remains the way to pick from them.
- A folder list is one field with `:` between folders, where the C++ has a file picker per entry.
- The clipboard, file index and snippet preferences sit under Clipboard History, Search Files and
  Manage Snippets on the Commands page, since Compass's builtins are one provider; the C++ shows
  them on the Clipboard, File Search and Snippets extension pages.
- A provider's provenance is Built-in, Raycast or Extension; the C++ also tells the Vicinae store
  from a local build.
- The file index, snippet and script preferences are read where they are used or when the engine
  next starts, as `compass.json` edited by hand is.

### The gaps pass, KDE (2026-09-25)

The KDE window-manager provider, from PLAN §12.0's `src/services/window-manager` row, against the
C++ in `src/server/src/services/window-manager/kde/`. No IPC change: KWin's windows reach the
switcher, Switch Workspaces, the toggles and `WindowManagement` through the existing requests.

**How it works, as the C++.** KWin has no socket and no D-Bus call that lists windows, so the engine
owns `org.vicinae.WindowTracker` on the session bus and loads a tracker script into KWin over
`org.kde.kwin.Scripting` (`loadScript` of a temporary file, then `run` on `/Scripting/Script<n>`,
the file kept until `run` answers because KWin opens it then). The script walks
`workspace.stackingOrder` once and forwards `windowAdded`, `windowRemoved`, `windowActivated` and
each window's `captionChanged` with `callDBus`; the engine answers from what it has been told. A
tracker left by an earlier run is unloaded first, the tracker is reloaded whenever `org.kde.KWin`
gets an owner (KWin restarted) and its windows forgotten when it loses one, and it is unloaded when
the engine stops. Focusing a window is a one-shot script (`workspace.activeWindow = w`) loaded, run
and unloaded under a unique plugin name. Chosen as the C++ chooses it: `kde` in
`$XDG_CURRENT_DESKTOP` on a Wayland session (`Environment::isWaylandPlasmaDesktop`).

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/services/window-manager` | — (stays `Rust ✓` 🟡: GNOME's workspace list remains; X11 has since been decided n/a, "Product decisions") | `compass_platform_linux::compositor::kwin` (`Kwin`, the tracker object, the scripts), `Provider::Kwin`, `compass::wlroots::{start_kwin, stop_kwin}`, `compass::window_service::{kwin_list, kwin_windows, kwin_act}`, the `kde.kwin` doctor check | `the_tracker_lists_kwins_normal_windows_and_which_is_active`, `focus_close_and_fullscreen_are_one_shot_scripts_unloaded_after`, `virtual_desktops_are_the_workspaces_and_the_overview_is_kwins_shortcut`, `a_kwin_restart_forgets_its_windows_and_reloads_the_tracker`, `a_second_tracker_is_refused_the_name_and_stop_gives_it_up`, `plasma_on_wayland_is_a_kde_entry_and_a_display`, `a_one_shot_script_quotes_its_target`, `handles_are_numbered_once_and_never_reused`, `kwin_windows_leave_out_the_launcher_and_number_their_desktop`, `kwin_with_its_desktops_passes_and_counts_them`, `kwin_absent_on_plasma_warns_and_elsewhere_is_not_asked`, `kwin_without_virtual_desktops_warns_that_there_are_no_workspaces` |

The tests run against a private `dbus-daemon` with a fake `org.kde.KWin`
(`compass-platform-linux/tests/kwin.rs`): it answers the scripting interface as KWin does (a number
from `loadScript`, a `run` that reads the file it was given) and, having no JavaScript engine,
replays what KWin would do running each script — the tracker's `add`/`activated` calls, a one-shot's
change and the signal it causes. That the scripts themselves are correct JavaScript against KWin's
API is **VM tier**: real KWin (Plasma 6) is the only proof, and no KWin runs in CI.

Declared differences, beyond the C++:

- **More than the C++ offers.** The C++ provider lists and focuses windows and declares no
  capabilities (`canClose` is false, no workspaces). Compass also closes windows (`closeWindow()`),
  toggles fullscreen (`fullScreen`), lists and switches virtual desktops as workspaces through KWin's
  own `org.kde.KWin.VirtualDesktopManager` (`desktops`, `current`), and opens the overview through
  kglobalaccel's `invokeShortcut("Overview")` on `/component/kwin`. The tracker's `add` therefore
  carries two more arguments than the C++'s (the window's first desktop id, empty on all desktops,
  and whether it is fullscreen) and is re-sent on `desktopsChanged` and `fullScreenChanged`.
- **No floating toggle.** KWin floats every window already; `ToggleFloating` is refused by name and
  the capability is off.
- **Window handles.** KWin's ids are UUIDs and the engine's window requests carry a `u32`, so each
  UUID is numbered the first time it is seen and the number is never reused in the process, across
  KWin restarts included.
- **"The window the person was in"** for the toggles is the most recently activated window, not the
  launcher's, on the current desktop (or on all desktops), from the tracker's activation order.
- **No geometry.** The tracker does not follow `frameGeometryChanged` (it fires on every step of a
  drag), so KWin windows report no bounds to `WindowManagement`.
- **Start-up is detached.** The C++ starts the provider synchronously; Compass starts it on a task
  after the socket is bound (bounded to 10 s), so a KWin that never answers costs window switching,
  never the engine. Until it is up, window requests go the non-KDE way.
- **The tracker trusts its callers**, as the C++'s does: any session-bus client may call
  `org.vicinae.WindowTracker`. And the script file is written to the temporary directory, as the
  C++ writes it; inside the Flatpak that is the sandbox's own `/tmp`, which KWin cannot read, so
  KDE window management there needs a path KWin can see (VM tier, with the Flatpak).
- `compass doctor` gains `kde.kwin` (the C++ has no doctor): whether KWin owns its name and how many
  virtual desktops it reports, a warning without either, and a warning for KDE on X11.

**X11 is not implemented**, and since "Product decisions" (2026-09-25) will not be: Compass is
Wayland only. What it would have taken is recorded in PLAN §12.0: the C++ `x11/` provider is 1,163 lines over XCB and EWMH
(`_NET_CLIENT_LIST`, `_NET_ACTIVE_WINDOW`, `_NET_WM_DESKTOP`, `_NET_CURRENT_DESKTOP`,
`_NET_DESKTOP_NAMES`, `WM_DELETE_WINDOW`, `_NET_WM_STATE_STICKY`) with a `PropertyNotify` listener
on the root window.

### The gaps pass, HUD and onboarding (2026-09-25)

The view layer's HUD and first-run flow and `src/builtins/vicinae`'s remaining views, from PLAN
§12.0, against the C++ in `src/server/src/ui` and `builtins/vicinae` (IPC v19). A cell flips only
with a named module and named tests that fail on a regression.

**The HUD** (`ui/windows/hud-bridge.*`, `ui/qml/hud`). `compass_ui::hud` holds the pill's state:
what it says (a line and a builtin icon or an emoji), which surface is its own, and its deadline,
1.5 s after the last message, which a new message moves as `m_timer.start()` restarts the C++'s. The
surface is a second layer surface (`crate::surface::open_hud`: namespace `vicinae-hud`, the `top`
layer, unanchored so centred, on the active output, no keyboard interactivity and transparent to
the pointer, as `HudWindowLayerShell.qml`), drawn by `LauncherApp::view_for` as `HudWindow.qml`'s
pill: the background at 90%, the divider for its edge, a 16 px icon and a line elided at 270 px. It
is offered where the C++ offers it on Linux, a layer-shell presentation
(`Environment::isHudSupported`); on GNOME's toplevel `showHud` only hides, and so does Compass.
`LauncherApp::show_hud` is `NavigationController::showHud`: it hides the launcher and puts up the
pill. It is wired where the C++ calls it: Quit and Force Quit ("Quit Files", "Force quit Files"),
the calculator's copies ("Answer copied to clipboard"), `CopyToClipboardAction`'s copies ("Copied to
clipboard" with its icon: the emoji picker, including a refused paste's copy, Browse Apps, Run
Terminal Program, Search Files, Calculator History's rows), clipboard history's copy ("Selection
copied to clipboard"), a shortcut's and a snippet's copy, and Set as wallpaper ("Wallpaper set").
The engine's own HUDs (the media commands, a `silent` script's line, a Rhai script's `hud`, Set
Default Browser and Terminal) go to the window as IPC v19 `WindowCommand::Hud`, which the window
answers `Failed` where it has no HUD; the engine then posts the transient notification it posted
before.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `ui/qml`, `ui/quick`, `ui/windows` | — (the HUD is closed; the settings window keeps each amber) | `compass_ui::hud`, `compass_ui::app::hud` (`show_hud`, `copy_with_hud`, `view_for`), `compass_ui::surface::{open_hud, layer::hud_settings}`, `compass::serve::show_hud` over `WindowCommand::Hud` | `a_second_message_reuses_the_surface_and_restarts_the_timer`, `a_surface_closed_under_it_is_forgotten`, `the_hud_surface_takes_no_keyboard_and_no_pointer`, `a_toplevel_presentation_opens_no_hud_surface`, `quit_and_force_quit_hide_with_the_cpps_hud`, `a_copied_answer_shows_the_calculators_hud_until_its_time_is_up`, `without_a_hud_a_copy_only_hides_and_an_exiting_launcher_shows_none`, `the_hud_surface_is_not_taken_for_the_launcher_window`, `the_engines_hud_is_refused_where_the_presentation_has_none`, `a_refused_paste_copies_the_glyph_with_the_copy_hud`, `a_set_wallpaper_and_a_copied_file_say_so_and_running_does_not`, `the_engines_hud_reaches_the_launchers_hud` (a real engine and a fake window) |

Declared differences:

- The surface is a fixed 336×48 with the pill centred in it, rather than sized to the pill: a layer
  surface's size is asked for before anything is laid out, and the rest of it is transparent and
  takes no input.
- A no-view extension's `showHUD` goes to the launcher's HUD (`HeadlessShell` is handed the
  launcher window), falling back to a desktop notification where there is no HUD. Its failure
  toasts and its crash go to the launcher as `WindowCommand::Failure` (IPC v23): the reason in the
  HUD now and the launcher's error line when it is next shown, since the launcher has hidden by
  then and a desktop notification is lost where no notification daemon runs. The notification is
  still posted too, and outlasts the HUD.
- Where there is no HUD, the engine's HUDs become a transient notification rather than nothing.
- Set Default Terminal's HUD has no icon (the C++'s is a green `$` symbol, which the builtin set does
  not have).

**Onboarding** (`ui/windows/onboarding-window.*`, `ui/qml/onboarding`). `compass_core::onboarding`
is `OnboardingWindow`'s gate and record and the QML's step logic: the flow is due when
`$XDG_STATE_HOME/compass/onboarding.json` records a version older than `ONBOARDING_VERSION` (1) or
cannot be read, and finishing writes `{"version":1,"completedAt":"…"}` there, the C++'s own file,
so a person who finished it under either engine is not asked again. The steps are the QML's on
Linux: "Welcome to Vicinae", "Make it your own" (the theme, kept as Set Theme keeps it, and the
global hotkey row) and "Setup complete" (GitHub and Sponsor), with Back, the step dots (a click
jumps), Continue and Finish; Enter continues. **Difference:** Escape goes back a step and closes
only from the first, and Tab and Shift+Tab walk each step's controls (the theme dropdown, Open Docs,
each Install, GitHub, Back, Continue) with Space or Enter to press, as in Settings; the QML flow is
mouse-first. `compass` passes the state file to the window when the flow is due
(`AppFlags::onboarding`). `COMPASS_NO_ONBOARDING` is the C++'s `ENABLE_ONBOARDING=OFF`, and the VM
tier, the sway harness and the session bench set it.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `ui/qml`, `ui/quick`, `ui/windows` | — (onboarding is closed; the settings window keeps each amber) | `compass_core::onboarding` (`should_show`, `mark_completed`, `came_from_vicinae`, `Flow`), `compass_ui::onboarding_page`, `compass_ui::app::onboarding`, `compass::onboarding_due` | `it_is_due_until_the_current_version_is_recorded`, `the_cpps_own_file_is_read`, `linux_has_four_steps_and_continue_finishes_on_the_last`, `the_recommendations_are_installable_store_extensions`, `suite_1_shows_every_recommendation_rendering`, `an_unreachable_store_is_reported_and_can_be_retried`, `every_onboarding_button_does_what_it_says`, `every_onboarding_step_paints_its_heading_and_buttons` (paint), `the_permissions_step_is_macos_only`, `the_switch_reads_like_a_boolean_environment_variable`, `a_due_onboarding_started_hidden_waits_for_the_first_summon`, `finishing_the_onboarding_records_it_and_hides`, `escape_closes_the_onboarding_and_records_it_as_seen`, `a_home_carried_over_from_vicinae_is_recognised`, `every_onboarding_step_draws_its_heading_and_buttons` |

Declared differences:

- **Started hidden, the flow waits for the first summon.** The C++ opens its window at server start;
  `compass start --hidden` runs at login, where a window nobody asked for is in the way.
- **Closing the flow counts as having seen it.** The C++ records only Finish, so closing its window
  shows the flow again at every start; here Escape or the hotkey record it too. Finish leaves the
  launcher open at its search rather than hiding.
- **A home carried over from Vicinae skips the flow**: its `vicinae` directory (or the symlink the
  move leaves) or Vicinae's `settings.json` means the launcher is already set up.
- The flow is drawn in the launcher's card rather than a 700×480 window of its own: the launcher has
  one surface, and a second toplevel would be a second window for the compositor to place.
- The global hotkey row and the last step say how to open the launcher on this desktop: on Sway,
  Hyprland and niri the line for the compositor's configuration, elsewhere the hotkey with
  `compass toggle` as the fallback, and inside the Flatpak `flatpak run org.tunaos.compass toggle`
  (`compass_core::hotkey_guide`).
- The macOS permissions step and Launch at login are not offered, as on the C++'s Linux build.
- An "Add extensions" step before the last recommends `compass_core::onboarding::RECOMMENDED_EXTENSIONS`
  (store extensions Suite 1 shows rendering) with an Install button each, through the store's own
  `store_install`. The C++ flow never mentions extensions (#250). A failed install, offline
  included, says why and leaves Continue working. Four come from the Raycast store (Google
  Translate, Google Search, GIF Search, Tailwind CSS: no account, no API key, no AppleScript) and
  three from Vicinae's, each labelled with its store.
- The step's buttons are Adwaita's (`compass_ui::adwaita`): Continue and Finish the suggested
  action, the rest flat and neutral, Installed and Installing… bare. A link button says on the card
  that it opened ("Opened in browser") or why not, where the C++ calls `Qt.openUrlExternally` and
  says nothing.
- Open Docs goes to Compass's guide (`docs/getting-started.md`, "Set a keyboard shortcut", on
  tunaos.org) rather than Vicinae's FAQ, and the last step names Compass and links tuna-os/compass
  with no Sponsor button: Compass has no sponsor page, and asking a new user to fund another project
  at setup reads as Compass's own request (#248, #249). The tray keeps "Sponsor Upstream Vicinae".

**`src/builtins/vicinae`'s remaining views** (`VicinaeExtension`). Each command is a builtin under
its C++ id (`commands:<id>`, and `core:<id>` names it too), as `CommandKind::Vicinae`, dispatched by
`compass_ui::app::vicinae`:

- **Configure Fallback Commands** (`ManageFallbackViewHost`): the items `isSuitableForFallback`
  admits (Search Files, every extension command, every quicklink with one argument) in "Enabled", in
  the configured order (`bug_report::order_enabled`), and "Available"; fuzzy over the title and, at
  0.3, the keywords. Enter or the panel's one action enables an item first in `fallbacks` or disables
  it, at once in the window and in `compass.json` through IPC v19 `RootItemEdit::Fallback`
  (`compass_core::root_items::set_fallback`, as `enableFallback` and `disableFallback`). Search Files
  is written by the C++'s id, `files:search`, and disabled by whichever id names it. A root fallback
  row's panel is Open (command) and Manage Fallback Actions (`fallbackActionPanel`).
- **Show Installed Extensions**: every installed extension's manifest, with its provenance badge
  (Raycast, Vicinae, Local), Uninstall (asking first, as `UninstallExtensionAction`, through the
  store's uninstall) and Copy Name, ID, Path and Author.
- **Search Builtin Icons**: every builtin icon, drawn, with Copy Icon Name.
- **Inspect Local Storage** and **Manage OAuth Token Sets**: the encrypted database's namespaces,
  then a namespace's keys with Show value; the token sets with "Expired", Remove token set (asking
  first) and the copies (access, refresh and ID token, scopes, expiration date), over IPC v19
  `LocalStorageNamespaces`, `LocalStorageItems`, `OAuthTokenSets` and `RemoveOAuthTokenSet`
  (`compass::serve::storage`), refused by name without a keyring as calculator history is.
- **Refresh Apps**, **Reload Script Directories**: rescan and say so, with the C++'s sentences.
- **Report a Vicinae Bug**, **Donate to Vicinae**, **Join the Discord Server**: open the link and
  hide with "Opened in browser"; the report is pre-filled from this build and `/etc/os-release`
  (`bug_report::{report_url, parse_os_release}`).
- **Open Config File** (writing `compass.json` first when there is none), **Open Default Config
  File** (this engine's defaults written read-only to the cache directory), **Show Log File**
  (`compass.log`, in the file browser). See "Opening Compass's own files" below for where these
  differ from the C++.
- **The store intros** (`StoreIntroViewHost`): the Vicinae and Raycast stores open on their intro
  until "Continue to store", or always with `alwaysShowIntro`.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/builtins/vicinae` | Rust ✅ | `compass_ui::{app::vicinae, fallbacks_page, vicinae_pages}`, `compass_core::commands::CommandKind::Vicinae`, `compass_core::root_items::set_fallback`, `compass_core::bug_report::{report_url, parse_os_release}`, `compass::serve::storage` | `configure_fallback_commands_moves_items_between_its_sections`, `enabled_come_first_in_the_configured_order_then_the_available`, `the_filter_narrows_both_sections_and_an_available_row_enables`, `a_fallback_is_enabled_first_and_disabled_as_the_cpp_writes_them`, `the_fallback_manager_writes_the_users_fallbacks` (a real engine), `installed_extensions_are_listed_copied_and_uninstalled_after_asking`, `search_builtin_icons_copies_the_name`, `inspect_local_storage_browses_a_namespace_and_shows_a_value`, `manage_oauth_token_sets_copies_and_removes_after_asking`, `local_storage_lists_its_namespaces_and_their_items_as_text`, `token_sets_are_listed_with_their_expiry_and_removed`, `the_link_and_refresh_commands_do_what_their_cpp_ones_do`, `the_default_config_is_written_read_only_and_replaced`, `the_vicinae_extensions_commands_keep_their_cpp_ids`, `os_release_gives_the_pretty_name_and_version_unquoted`, `the_report_link_carries_the_title_body_and_type`, `the_extension_store_installs_into_root_search_and_uninstalls_after_asking` (the intro) |

Declared differences:

- Open Vicinae Settings is not offered: the settings window is `ui/settings`' gap. Forget Past
  Vicinae Telemetry is not offered: Compass sends no telemetry (a hard fork, by decision
  2026-09-25; "Product decisions").
- Report a Vicinae Bug's optional title argument is not asked for; the issue opens untitled. Its
  "QT Platform" line says `wayland`.
- The Available section lists Search Files, then the extensions, then the quicklinks, rather than in
  root search's empty-query order; a filter orders it by score as the C++'s does.
- A store intro's continuation is remembered in the view memory (`compass-view-state.json`) rather
  than the command's local storage, and the intro's Markdown has no icon above it.
- Show Installed Extensions shows each extension's initial rather than its `assets` icon
  (`ui/image`'s gap).
- Show value says the value under the list rather than in a toast.

**`src/services/window-material`** (`ExtBackgroundEffectV1Manager`, `createRoundedRegion`).
`compass_wayland::material::BackgroundEffects` binds `ext_background_effect_manager_v1` (from
`wayland-protocols`' staging set, already in the tree) with `wl_compositor`, reads the one-shot
`capabilities` with a roundtrip before reporting blur, and gives each surface one
`ext_background_effect_surface_v1` whose blur region is sent again only when its parameters change
(`Applied::{Created, Updated, Unchanged, Unsupported}`, as `compass_core::window_effects` models
it); `rounded_region` is `createRoundedRegion`'s corner cut, row by row. The row stays amber: the
launcher's `wl_surface` belongs to winit's or `iced_layershell`'s connection and is exposed only as a
raw pointer, and bridging it (`Backend::from_foreign_display`, `ObjectId::from_ptr`) is `unsafe`,
which the workspace forbids. It needs a compositor with the protocol to verify (KWin 6.3, niri;
Sway has none), which the headless-Sway test covers for the refusal and the VM tier would for the
rest.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/services/window-material` | — (the launcher's surface is not reachable safely) | `compass_wayland::material::{BackgroundEffects, rounded_region, supports_blur}` | `the_corners_are_cut_as_the_cpp_cuts_them`, `a_region_off_the_origin_is_cut_where_it_is`, `a_square_region_has_nothing_taken_away`, `blur_is_the_capability_bit`, `background_effect_is_bound_where_advertised_and_refused_by_name_where_not` (headless Sway) |

### The gaps pass, wlroots paste and inhibit (2026-09-25)

The last two compositor rows of PLAN §12.0 that need no product decision, against
`src/server/src/services/paste` and `services/shortcut-inhibit`. Both are proved on headless Sway,
where a test window records every keyboard event it is sent; no real device or session is touched.
No IPC change: the existing requests (`PasteText`, `PasteSnippet`, `ClipboardPaste`, `CopyFile`,
`FileActions.can_paste`) now succeed on wlroots where they were refused.

**Paste** (`LinuxPasteService`, `PasteService`). `compass::paste` is the engine's one paste path, and
the emoji picker, snippets, clipboard history, a file's Paste and an extension's `Clipboard.paste`
all go through it. On GNOME it is the Shell extension's, as before. On a wlroots compositor with
data-control the content goes on the clipboard, and `compass_core::paste::PasteService` (the C++'s
state machine, ported earlier and now driven) waits for focus and presses the chord into the window
that has it: Ctrl+Shift+V when that window's `app_id` is a `TerminalEmulator` application in the app
index, Ctrl+V elsewhere. The chord is pressed by the input server's `injectPaste` (uinput, what the
C++ uses) when the helper runs with injection, else on a `zwp_virtual_keyboard_v1` keyboard
(`compass_wayland::virtual_keyboard`, over `wayland-protocols-misc`), which needs no device node or
capability: its own three-key keymap, Control and Shift with the modifier masks the protocol asks
the client to send, released in reverse. With neither the content is copied and the request refused
by name ("the content was copied instead"), which the launcher already answers by copying, as the
C++ copies first and then says the platform cannot paste. A newer paste cancels one still waiting.
`compass doctor` names `virtual-keyboard` and `shortcuts-inhibit` in `wlroots.capabilities`.

**Shortcut inhibit** (`WaylandShortcutInhibitManager`, `ShortcutInhibitorAttached`).
`compass_wayland::keyboard_inhibit::ShortcutInhibit` is the protocol client, replacing the stub. The
launcher's surface belongs to `iced_layershell`, which hands it out only as a raw pointer this
workspace may not use (`unsafe`), so the launcher now makes its own Wayland connection, gives it to
`iced_layershell` (`Settings::with_connection`) and binds the inhibitor and a `wl_keyboard` of its
own on the same connection: the compositor's `enter` names the launcher surface that took the
keyboard, as a proxy on that connection. While a shortcut recorder records (the action panel's or
the settings'; `ShortcutInhibitor.enabled: capture.capturing`) the focused launcher surface gets an
inhibitor; it is destroyed when recording ends or the keyboard leaves. `LauncherApp::update` says
when, from the recorder state (`shortcuts_inhibited`). The inhibitor never reads the socket: the
toolkit does, and each update dispatches what was read for the inhibitor's queue (a second reader
on one connection can take the toolkit's events and leave its event loop waiting, which the first
version of the Sway test caught as a hang).

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/services/paste` | Rust ✅ | `compass::paste` (`paste`, `can_paste`, `injector`, `paste_blocking`), `compass_core::paste::PasteService`, `compass_wayland::virtual_keyboard::{VirtualKeyboard, paste_steps}`, `compass_wayland::compositor::Capabilities::virtual_keyboard` | `on_sway_a_paste_is_copied_and_pressed_into_the_focused_window`, `on_sway_a_terminal_is_pasted_into_with_ctrl_shift_v`, `on_sway_the_input_server_presses_the_paste_when_it_runs` (engine on Sway), `on_sway_the_paste_chord_reaches_the_focused_window`, `on_sway_a_terminal_is_sent_ctrl_shift_v`, `ctrl_v_holds_control_around_the_v`, `a_terminal_gets_shift_as_well_released_in_reverse`, `the_keymap_names_the_codes_the_steps_press` |
| `src/services/shortcut-inhibit` | Rust ✅ | `compass_wayland::keyboard_inhibit::{ShortcutInhibit, InhibitHandle, InhibitState}`, `compass_ui::shortcut_inhibit`, `LauncherApp::recording_shortcut` | `on_sway_shortcuts_are_inhibited_on_the_focused_surface_while_wanted` (created, active, destroyed, created again without a protocol error), `on_sway_the_inhibitor_goes_with_the_keyboard`, `the_root_panel_records_an_items_shortcut_and_backspace_removes_it`, `the_hotkey_is_recorded_into_the_launcher_section` |

Declared differences:

- The input server is preferred, as the C++ has only it; the virtual keyboard is the fallback for a
  session without the helper (not installed with its capability, a Flatpak). Its keymap is the
  chord's own, so a layout without a V key on that position still pastes; the focused window is
  sent that small keymap with the chord and the seat's own again with the next real key.
- Focus handoff is watched on the foreign-toplevel list only where the launcher is itself a toplevel
  (`COMPASS_LAYER_SHELL=0`, or no layer shell). A layer-shell launcher is not in the list, so the
  window under it reads as focused before the launcher closes; there the C++'s blind delay (150 ms)
  is used instead of the poll.
- The clipboard is not restored after a wlroots paste (`scheduleClipboardRestore`), as on the GNOME
  path; the pasted content stays on it.
- The inhibitor is made when a launcher surface takes the keyboard and destroyed when it leaves,
  rather than kept for the window's life and deactivated by the compositor; the person's keyboard
  is the same. It needs the layer-shell presentation: under the `xdg_toplevel` one (GNOME, KDE,
  `COMPASS_LAYER_SHELL=0`) winit makes its connection itself and offers no way to share it, and a
  raw-pointer bridge is `unsafe`, so the recorder does not inhibit there. The C++ inhibits on any
  Wayland session.
- An extension host run without the engine (none today) copies rather than pastes.

### The view layer, closed (2026-09-25)

The HUD and onboarding tables above each left `ui/qml`, `ui/quick` and `ui/windows` amber for the
settings window alone; "The gaps pass, settings" landed it in the same change, so the three rows
flip to ✅ in both columns on the tests those three sections name: the settings view's
(`the_settings_view_writes_each_setting_and_switch_into_the_configuration`,
`every_cpp_general_settings_property_is_ported_or_declared` and the launcher-state tests in
`compass-ui/src/app/settings_view/tests.rs`), the HUD's and onboarding's (their tables above), on
top of the resident window and paint tier tests. What stays different is declared where it
lands: the settings and onboarding drawn in the launcher card rather than windows of their own,
drag out of the window, which Iced cannot do, and blur behind the launcher
(`src/services/window-material`, still amber).

### The gaps pass, global shortcuts (2026-09-25)

`src/services/global-shortcuts` from PLAN §12.0, against `GlobalShortcutService`, its backends and
`shortcut_conflict::validate` (IPC v20). A cell flips only with a named module and named tests that
fail on a regression.

**What is bound.** `compass_core::global_shortcuts::desired` reads the configuration as
`reconcile` does: the launcher hotkey (`launcher.hotkey`, `super+space` unless set, nothing when
set empty), and every entrypoint's `providers.<p>.entrypoints.<e>.shortcut` that is not empty, not
turned off and reads as a combination, each described by its item's title. A `Reconciler` keeps
what is bound and what each bound id does (`m_appliedTriggers`, `m_actions`): a shortcut no longer
asked for, or asked for with another trigger, is released; a new one is bound; one the desktop
refused counts as applied, so it is not asked for again until it changes, and does nothing when
pressed. `compass::global_shortcuts::Service` runs that against a backend, from the engine's start
and again on each `SetSetting` and root-item edit (`configChanged`), so the settings view's
hotkey recorder and the action panel's Set Global Shortcut take effect at once.

**Pressing one** (`onActivated`). The launcher hotkey toggles the attached window; a command's
launches it as `cmd launch` launches it (`launch_command`, the path `activateEntrypoint` shares with
root search: an application is launched and its launch recorded, anything else is handed to the
window as a launch).

**The backends**, in the C++ factory's order and then the portal the C++ does not have:
`xx-hotkey-v1`, then `vicinae-hotkey-v1`, over one Wayland connection
(`compass_wayland::hotkey::HotkeyClient`, ports of `XxHotkeyGlobalShortcutBackend` and
`VicinaeHotkeyGlobalShortcutBackend`: one hotkey object per shortcut, the bind waiting for `bound`
or `denied`, the C++'s sentence for a denial without a message, a revocation logged, the hotkey
destroyed on unbind) on any compositor but GNOME's; then the GlobalShortcuts portal
(`compass_portals::ShortcutBinder`), which binds a set at a time, so the binds of one reconcile are
bound together on a fresh session after the last one is closed. Keys become keysyms as
`xkbKeysymForQtKey` makes them and the protocols' modifier mask as `fromQtMods`
(`compass_core::global_shortcuts::{keysym, modifier_mask}`); the portal is asked in the "shortcuts"
specification's spelling (`portal_trigger`: `LOGO+space`, `CTRL+ALT+SHIFT+LOGO+a`).

**The recorder** (`setCapturing`, `shortcut_conflict::validate`). While either recorder captures
(the action panel's and the settings view's), the window says so to the engine (IPC v20
`Request::ShortcutCapture`), which releases every global shortcut and binds them again after, so
the combination reaches the recorder rather than the desktop. The check is the C++'s order: a
modifier (unless a function key or modifiers alone), then the launcher's own keys (`KeybindManager`:
Toggle action panel, Open settings, Quick launch), then the global shortcuts (`findConflict`: the
launcher hotkey as "the launcher hotkey", except when recording the hotkey itself, and any other
item's shortcut by its title, or "another command").

**Close on focus loss** (`setWindowActivated`). With `launcher.close_on_focus_loss` on, the window
hides when it loses a focus it had, and not when the focus goes to the file chooser it opened
(`m_pendingLauncherFileChoice`); the settings view's switch applies at once.

Differences on the layer-shell presentation and after (2026-10-01, the robustness pass):

- **A click outside a layer-shell launcher hides it.** A layer surface that holds the keyboard
  exclusively never loses the focus, so the C++ falls back to a pointer that leaves the window
  ("only works on some compositors", `LauncherWindow::eventFilter`), and on Sway a click on
  another window went through and left the launcher up (TIL-03). With the option on, Compass puts
  a transparent layer surface under the launcher (`surface::open_backdrop`), whose input region
  leaves the launcher out, and a click on it hides the launcher. It is the "layer behind ours"
  the C++ comment suggests.
- **`toggle` brings back a toplevel left behind another window** rather than hiding it, as a new
  window, which the compositor focuses as it does every summon; raising the old one is refused
  without an activation token. The C++ hides it. A launcher that has never reported the focus (a
  seat with no keyboard) still toggles closed.
- **The window fits small and scaled outputs** (TIL-04): the layer surface first opens at the
  output's full size to learn the room, then takes the launcher's size fitted to it; a toplevel
  is fitted to its monitor once mapped, and opens fitted after that. The C++ sizes its window
  from the config and lets an oversized one be cut off.
- **`compass start` runs the window as a child and starts it again when it fails**, keeping the
  engine; five failures in a minute end it. The C++ runs one process.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/services/global-shortcuts` | `parity test ✓` 🟡 → ✅ (`Rust ✓` stays 🟡: X11, `inhibitApps`, `probeBind`) | `compass_core::global_shortcuts` (`desired`, `Reconciler`, `validate`, `find_conflict`, `launcher_keybind`, `keysym`, `modifier_mask`, `portal_trigger`), `compass_wayland::hotkey::HotkeyClient`, `compass_wayland_protocols::vicinae_hotkey_v1`, `compass_portals::ShortcutBinder`, `compass::global_shortcuts` (`Service`, `WaylandBackend`, `PortalBackend`, `activate`, `serve`, `Control`), `compass_ui::app::global_shortcuts`, `compass_ui::shortcut_recorder` | `the_launcher_and_every_enabled_items_shortcut_are_desired`, `the_launcher_hotkey_defaults_to_super_space_and_an_empty_one_binds_nothing`, `reconciling_binds_what_is_new_rebinds_what_changed_and_keeps_the_rest`, `a_refused_bind_does_nothing_when_pressed_and_is_not_asked_for_again`, `a_bound_shortcut_runs_its_action`, `the_recorder_refuses_the_launchers_own_keys`, `the_recorder_refuses_the_launcher_hotkey_except_for_itself`, `the_recorder_refuses_another_items_shortcut_by_its_title`, `keys_become_the_keysyms_the_cpp_asks_for`, `modifiers_become_the_protocols_mask`, `the_portal_is_asked_in_the_specifications_spelling`; `vicinae_hotkey_binds_presses_refuses_and_releases`, `xx_hotkey_is_preferred_and_speaks_its_own_requests`, `a_compositor_with_neither_protocol_is_unsupported` (a fake compositor over `wayland-server`); `the_binder_binds_a_changed_set_on_a_new_session_and_closes_the_old_one`, `the_binder_delivers_the_current_sessions_activations`, `an_empty_set_closes_the_session_and_binds_nothing`, `a_denied_set_is_reported_as_denied` (a private `dbus-daemon` and a fake GlobalShortcuts portal); `the_launcher_hotkey_and_a_commands_shortcut_are_bound_and_rebound_when_changed`, `a_refused_shortcut_does_nothing_when_pressed`, `capturing_releases_everything_and_binds_it_again_after`, `pressing_the_launcher_hotkey_reaches_the_window`, `pressing_a_commands_shortcut_launches_it_as_cmd_launch_does`, `a_reload_binds_what_the_configuration_now_says`, `the_recorders_capture_reaches_the_service_over_ipc`, `the_portal_is_asked_for_the_configured_trigger`, `a_release_is_not_a_press`; `losing_the_focus_hides_the_launcher_when_close_on_focus_loss_is_on`, `losing_the_focus_keeps_the_launcher_when_close_on_focus_loss_is_off`, `only_losing_a_focus_the_launcher_had_hides_it`, `the_recorder_suspends_the_global_shortcuts_while_it_captures`, `the_launchers_own_keys_and_its_hotkey_are_taken`, `recording_the_launcher_hotkey_does_not_conflict_with_itself` |

Declared differences:

- The launcher hotkey is `launcher.hotkey` (the C++'s `globalShortcuts.toggle`), and its id is
  `toggle` (the C++'s `@toggle-launcher`): Compass has bound its launcher under `toggle` since its
  first release, and the portal keeps the user's trigger against that id.
- The portal is a backend here and not in the C++, which has none on GNOME. It binds a set at a
  time, so a changed set is bound on a new session; the trigger asked for is a preference the
  desktop may override, and GNOME keeps the one the user chose in its own settings. Suspending
  for the recorder closes the portal session, and the set is bound on a new one after.
- A configuration edited by hand is bound at the next start or the next change made through the
  settings view or the action panel: the engine does not watch `compass.json` as `config::Manager`
  does.
- The recorder cannot tell the compositor refused a combination (`probeBind`): the engine logs the
  refusal when it binds, and the shortcut stays in the configuration. (Closed in "The gaps pass,
  GNOME workspaces and shortcut probes".)
- `globalShortcuts.inhibitApps` (suspending every shortcut while a listed application is frontmost)
  is not in Compass's configuration and not ported. (Closed in the same pass, as
  `global_shortcuts.inhibit_apps`.)
- The conflict check does not know whether the desktop has a backend: the C++ reports no conflict
  where global shortcuts are unsupported, Compass always checks.
- A command whose arguments are required is refused by name, as `cmd launch` refuses it, where the
  C++ opens its arguments form.
- X11 has no backend: no X11 path exists in the tree (`x11rb` is only winit's), and Compass is
  Wayland only (decided 2026-09-25, "Product decisions").

VM tier (declared, not verifiable in a container): GNOME's portal grant dialog on the first bind and
whether `xdg-desktop-portal-gnome` honours the preferred trigger; a real compositor carrying
`xx-hotkey-v1` or `vicinae-hotkey-v1` (none released does); focus loss on a real compositor,
where a layer surface with exclusive keyboard focus may never report it.

### The gaps pass, currency (2026-09-25)

`src/services/calculator-service`'s last amber cell, against `NumenVicinaeCurrencyProvider`,
`NumenCalculatorBackend`'s refresh timer and `CalculatorRefreshRatesCommand` (IPC v21). The rate
source is the maintainer's decision: the European Central Bank's daily reference rates
(`eurofxref-daily.xml`, free and keyless, about thirty currencies against the euro).

**The rates** (`compass_core::exchange_rates`). `parse_ecb` reads the ECB's file with `roxmltree`
into the reference date and each currency's units per euro, the euro included at 1; a file that is
not the ECB's shape is refused by name. `RateCache` holds what was fetched and caches it as JSON at
`$XDG_CACHE_HOME/compass/exchange-rates.json` with the date and the fetch time; `refresh` takes
the fetch as a closure, keeps and caches a good answer and leaves the rates in hand after a failed
fetch or an unreadable answer. Rates are stale a day after they were fetched.

**The engine** (`compass::exchange_rates::ExchangeRateService`). It reads the cache the first time
the rates are asked for, fetches through the engine's HTTP client (`compass::stores::get`, the
stores' `ureq` agent) at start when there are none or they are stale, and looks hourly after that,
as the C++'s hourly timer; `COMPASS_DISABLE_AUTO_RATE_REFRESH` turns the automatic refresh off, as
`Environment::isAutoRateRefreshDisabled`. `COMPASS_EXCHANGE_RATES_URL` points it at another copy
of the file (tests, mirrors). It answers IPC v21 `Request::ExchangeRates` with what it holds and
`Request::RefreshExchangeRates` with a fresh fetch or the reason it failed, both as
`Response::ExchangeRates`.

**The calculator** (`compass_core::calculator`). The launcher asks for the engine's rates each time
its window opens and after Refresh Exchange Rates, and installs them (`set_exchange_rates`); every
evaluation, in root search and in Calculator History's live result, hands them to fend through its
exchange-rate handler (`ExchangeRateFnV2`), so `10 usd to eur`, `€5 in gbp` and `£8 to usd` are
answered, and copied ones are remembered as conversions (`is_conversion`). Without rates a currency
expression answers nothing, as the C++ does when its provider has none.

**Refresh Exchange Rates** (`commands:refresh-rates`, `compass_ui::app::calculator`). The command
asks the engine to fetch now and says how it went: "Refreshing rates...", then "Rates successfully
refreshed" or the error.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/services/calculator-service` | `Rust ✓` 🟡 → ✅ | `compass_core::exchange_rates` (`parse_ecb`, `ExchangeRates`, `RateCache`, `cache_path`, `source_url`), `compass_core::calculator` (`set_exchange_rates`, `compute_with_rates`, `RateHandler`), `compass::exchange_rates` (`ExchangeRateService`, `run`), `compass_ui::app::calculator` (`exchange_rates_task`, `refresh_exchange_rates`), `CommandKind::RefreshExchangeRates` | `the_ecb_daily_file_is_read_with_its_date_and_the_euro`, `a_file_that_is_not_the_ecbs_is_refused_by_name`, `currency_expressions_are_answered_with_the_rates`, `a_currency_the_rates_do_not_list_answers_nothing`, `without_rates_currency_expressions_answer_nothing`, `the_fixture_s_rates_convert_through_the_euro`, `the_process_rates_reach_root_search`, `rates_go_stale_after_a_day_or_when_the_clock_goes_back`, `a_refresh_keeps_and_caches_the_rates_and_a_restart_reads_them_back`, `a_failed_refresh_keeps_the_rates_in_hand`, `an_unreadable_cache_is_no_rates` (`compass-core/tests/exchange_rates.rs`, over a checked-in ECB file and fake fetches); `the_engine_fetches_the_ecb_rates_at_start_caches_them_and_keeps_them_when_a_refresh_fails`, `an_offline_engine_answers_with_the_cached_rates_or_none` (a real engine against a local fake ECB); `refresh_exchange_rates_installs_the_fresh_rates_and_says_so_or_why_not` |

Declared differences:

- The source is the ECB, not the Vicinae API: about thirty fiat currencies, and no crypto
  (`$SOL` and the other `$`-tickers answer nothing), nor any currency the ECB does not quote.
- The C++ refetches when its cache file is 30 minutes old; Compass when its rates are a day old,
  the ECB publishing once a working day. Both look hourly. The age is the fetch time kept in the
  file, not the file's modification time.
- The C++'s Refresh Exchange Rates reports success as soon as the fetch is started
  (`NumenCalculatorBackend::refreshExchangeRates` returns a ready future); Compass waits for the
  fetch and reports a failure by its reason.
- The C++ keeps the window open, clears the search text and shows toasts; Compass hides the window
  and shows the HUD, and a failure is also the launcher's error line when it next opens. Where the
  presentation has no HUD (GNOME's `xdg_toplevel`), only that error line says so.
- The launcher computes, so it learns rates the engine fetched in the background the next time its
  window opens.
- `€` before a number is moved after it before fend evaluates (`€5` is asked as `5€`), since fend
  reads `€5` as a word; the question remembered is the one typed.
- `COMPASS_DISABLE_AUTO_RATE_REFRESH` set to the empty string does not disable the refresh (the C++
  counts any value); the tests use this to turn it back on for one engine.
- As in the C++, remembered conversions are not re-evaluated after a refresh:
  `CalculatorService::updateConversionRecords` exists there but nothing calls it.

Tests never reach the ECB: the parser and fend read a checked-in copy of the file, the refresh
rules use closures, and the engine tests serve the file from a local `tiny_http` server; every other
engine a test starts has the automatic refresh turned off.

### The gaps pass, tray and sandbox (2026-09-25)

Two decisions the maintainer took. No IPC change: the tray's
entries reach the window over the `WindowCommand`s it already has, and its switch is a setting.

**Compass's own tray icon** (`src/services/tray`). `compass::tray_icon` serves a
StatusNotifierItem over the `ksni` crate with the C++'s menu (`compass_core::tray`): Toggle
(also what `Activate` and `SecondaryActivate` do), the version, About (the settings view's About
page), Settings… (the settings view, as `vicinae://settings/open` opens it), the three community
links, and Quit, which stops the engine as `compass shutdown` does and is left out under systemd
(`INVOCATION_ID`). It owns `org.kde.StatusNotifierItem-<pid>-<n>` and registers with the
desktop's `StatusNotifierWatcher`, again whenever one appears. `tray.enabled` (the C++'s
`config::Tray`, on by default) moved from `settings_catalog::NOT_IN_COMPASS` to an offered setting
on the Advanced page; the engine reads it at start and applies it at once when the view changes it
(`configChanged`'s `show`/`hide`), and `config migrate` now carries it over.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/services/tray` | `Rust ✓` 🟡 → ✅ | `compass::tray_icon` (`Control`, `run`, `perform`, `pixmaps`, `settings_link`) over `ksni`, `compass_core::tray` (the menu model), `compass_core::config::TrayConfig`, `settings_catalog`'s `tray.enabled`, `serve::settings::apply_live`, `config_migration`'s `tray.enabled` | `compass_shows_its_own_tray_icon_with_the_cpp_menu_as_the_setting_says` (a private `dbus-daemon` and a fake `StatusNotifierWatcher`: off in the file registers nothing; turned on it registers under the specification's name with `Id`, `Title` and the menu's labels; `Activate` and the entries reach the window as `Toggle` and the settings deeplinks; off it leaves the bus, on it registers again; Quit stops the engine), `the_tray_host_lists_activates_and_browses_another_applications_item` (Compass's own icon is not among Search Tray's), `tray_icon::tests::{the_menu_is_the_models_with_its_labels_and_actions, a_supervised_engine_offers_no_quit, the_pixmaps_are_the_cpp_sizes_in_argb_network_order, the_settings_entries_open_the_settings_view_by_its_deeplink, turning_the_setting_on_and_off_is_seen_once_each}`, `the_labels_are_the_cpp_ones_under_the_compass_name`, `applying_writes_the_key_the_engine_reads_and_keeps_the_rest`, `every_cpp_general_settings_property_is_ported_or_declared`, `every_documented_key_is_in_the_schema_with_its_default`, `migrate_writes_vicinae_json_and_leaves_the_cpp_file_alone` |

Declared differences:

- The product's name in the menu is Compass (ADR-0012): Toggle Compass, About Compass,
  Quit Compass, and the version as `Compass <version>` (the C++ passes its git tag and commit).
  The three community links are upstream Vicinae's, as the C++ has them.
- The item's `Id` and `IconName` are the application id `org.tunaos.compass`, the icon the
  package installs; the C++ uses `vicinae`. Its pixmaps are drawn from `extra/compass.svg`, where
  the C++ draws `vicinae.svg` over a white disc.
- Turned off, the item leaves the bus; the C++ keeps it and reports `Passive`, which some hosts
  still draw.
- Search Tray leaves Compass's own icon out (`tray_host::is_own`, by the connection's process id);
  the C++ host would list it, and toggling the launcher from its own list only closes it.
- Inside a Flatpak the item registers under its unique name only (ksni's `disable_dbus_name`),
  since the sandbox may not own `org.kde.StatusNotifierItem-*`, and the manifest now talks to
  `org.kde.StatusNotifierWatcher`, without which neither the icon nor the tray host reaches it.
- "Check for Updates…" and "Update Available" are not in the menu, as they are not in the C++'s
  Linux menu (`setCheckForUpdatesVisible` and `setAvailableUpdate` do nothing there).

VM tier (declared, not verifiable in a container): a real host drawing the item and its menu
(KDE's panel, GNOME's AppIndicator extension, Waybar), and the Flatpak's bus proxy passing the
host's calls to the item.

**A read-only `$HOME` allowlist for extensions** (decision of 2026-09-25; the extension sandbox,
row 6 of "The extension sandbox" below). `compass_sandbox::home::HOME_READ_ALLOWLIST` is the one
list: `~/.ssh/config` (the file alone, not the keys or anything else in `~/.ssh`),
`~/.password-store`, and `~/.config/hypr`, `~/.config/sway` and `~/.config/niri`, each only when it
exists. `extension_runner::policy_in` adds `home_reads`' paths to the worker's read set, never to
write or execute; the rest of `$HOME` stays denied. The same policy is applied inside the Flatpak,
whose `home:ro` makes these paths visible to it in the first place. A symbolic link is followed
only to a place the list itself names: Landlock grants the inode a path resolves to, so a
`~/.ssh/config` linked to a key, or a `~/.config/sway` linked into a dotfiles repository or to
`$HOME`, grants nothing (logged), and inside a granted directory Landlock refuses a link out of it.
A home that is itself a link (`/home` to `/var/home`) is resolved first. No row flips (the
extension host rows were green); the tests are `compass_sandbox::home::tests::*`
(`what_exists_is_granted_and_what_does_not_is_skipped`,
`a_link_to_a_key_or_out_of_the_list_grants_nothing`, `a_link_to_another_listed_place_is_followed`,
`a_home_that_is_itself_a_link_is_followed`, `the_wrong_kind_is_refused`,
`the_list_is_the_decided_five_and_only_ssh_config_is_a_file`),
`extension_runner::tests::the_policy_reads_only_the_allowlisted_home_paths_and_never_writes_them`
and, through the real `compass-sandbox-exec`,
`an_extension_reads_the_home_allowlist_and_nothing_else_of_home` (the allowed files read; the key
beside `~/.ssh/config`, another file of `$HOME` and the target of a linked-out `~/.config/sway`
refused, each readable unconfined; a write into the password store refused).

Suite 1's ledger does not change: its `HOME` is empty, so `ssh`, `pass`, `niri` and the `hypr*`
keybinding lists find nothing to read under either policy, as row 6 always said.

### The gaps pass, GNOME workspaces and shortcut probes (2026-09-25)

Three of the `Still C++-only:` items PLAN §12.0 listed under `src/services/window-manager` and
`src/services/global-shortcuts`, against `GnomeWindowManager::listWorkspaces`,
`GlobalShortcutService::{updateInhibition, computeInhibited, probeBind}` and
`GlobalShortcutBridge::validate` (IPC v21: `Request::ProbeShortcut`, `Response::ShortcutProbe`,
both appended last). **No cell flips**: both rows stay `Rust ✓` 🟡 for X11 alone, which is the open
X11 decision, so the ledger stays at 148 of 156.

**GNOME's workspaces: the Shell extension's contract v4.** Mutter's workspaces are reached only
from inside the Shell, so the extension's windows interface gains `ListWorkspaces()` (an `aa{sv}`
per workspace in order: `index`, `name` from `Meta.prefs_get_workspace_name`, `active`,
`has_fullscreen`) and `ActivateWorkspace(i)` (`Meta.Workspace.activate`), and `WindowsChanged` now
also fires on `active-workspace-changed`, `workspace-added` and `workspace-removed`, so Switch
Workspaces refreshes as windows do. The C++ provider's unversioned surface had `ListWorkspaces`
and `GetActiveWorkspace` as JSON strings; here the active one is a key of each entry. The engine
still speaks contract 3 (`compass_shell::OLDEST_CONTRACT_VERSION`): an extension a release behind
switches windows, reads the clipboard and pastes as before, and a workspace call is refused before
it reaches the bus (`ShellError::TooOld`), which the engine words as "update the extension".
`compass doctor`'s `gnome.shell-extension` reports `extension present, contract v4`, and warns
for v3 that Switch Workspaces needs the update. With no compositor IPC (Hyprland, niri, KWin) and
the extension answering, `WindowManagerCapabilities`, `ListWorkspaces` and `FocusWorkspace` go to
the extension (`compass::serve::workspaces::{gnome, gnome_workspace, gnome_window}`): each
workspace counts its windows and names each application on it once, as on the other compositors.

**`globalShortcuts.inhibitApps`** is `global_shortcuts.inhibit_apps` in `compass.json` (schema,
the migration from `settings.json` carries it, and the settings view's General page lists it as
"Pause shortcuts in", a list of application ids). While the focused application's desktop id is
listed, every global shortcut is released so its keys reach it (`Service::set_inhibited`, as
`updateInhibition`): nothing binds while paused, not a reload and not the end of a recording,
and the configuration is bound again once it is not. The focused application comes from the
window-manager providers (`compass::frontmost`, factored out of snippet expansion, which uses the
same answer): the toplevel list on wlroots, KWin's tracker, the Shell extension on GNOME, each
window recognised in the app index as `AppRuntime::frontmostApp` recognises it.

**`probeBind`.** When a recorder (the action panel's or the settings view's) captures a combination
that passes the conflict check, it asks the engine (`Request::ProbeShortcut`) and shows
"Checking..." until the answer: the engine binds the combination under `@probe` and releases it at
once, while every other binding is suspended for the capture, and the desktop's refusal is shown in
the recorder as the error it keeps recording under, as `validate` returns it. Only a combination
the desktop takes is saved.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/services/window-manager` | — (stays `Rust ✓` 🟡: the X11 provider) | `compass_shell` contract v4 (`ShellClient::{list_workspaces, activate_workspace}`, `Workspace`, `Availability::supports`, `ShellError::TooOld`, `OLDEST_CONTRACT_VERSION`, `WORKSPACES_SINCE`), the extension's `ListWorkspaces` and `ActivateWorkspace`, `compass::serve::workspaces::{gnome, gnome_workspace, gnome_window}`, `compass::window_service::refusal`, the `gnome.shell-extension` doctor check | `workspaces_are_listed_and_switched_through_the_extension`, `an_extension_a_release_behind_switches_windows_but_refuses_workspaces`, `every_contract_member_is_reached_through_the_proxy`, `the_contract_is_exactly_these_members`, `served_interfaces_match_the_checked_in_xml` (a private `dbus-daemon` and the mock Shell); `an_extension_a_release_behind_is_available_without_workspaces`, `a_workspace_needs_its_index_and_defaults_the_rest`; `list_workspaces_fills_every_key_the_client_decodes`, `a_workspace_switch_tells_the_client_to_look_again`, `the_vm_tier_asks_for_the_workspaces`, `the_extension_speaks_this_contract_version`, `the_vm_tier_waits_for_this_contract_version`; `on_gnome_switch_workspaces_lists_and_switches_through_the_shell_extension`, `an_extension_a_release_behind_offers_no_switch_workspaces_and_says_to_update` (the engine against the mock Shell); `an_extension_too_old_for_workspaces_is_asked_to_update`, `extension_at_contract_v4_passes_and_one_a_release_behind_warns_about_workspaces` |
| `src/services/global-shortcuts` | — (stays `Rust ✓` 🟡: the X11 backend) | `compass_core::config::GlobalShortcutsConfig`, `compass_core::global_shortcuts::{inhibited, probe, PROBE_ID}`, `compass_core::settings_catalog::Kind::Names`, `compass::global_shortcuts` (`Service::{set_inhibited, probe}`, `Backend::probe`, `Control::{set_frontmost, probe}`), `compass::frontmost`, `compass_ui::shortcut_recorder` (`Outcome::Probe`, `probed`), `compass_ui::app::global_shortcuts::{probe_shortcut, shortcut_probed}` | `a_listed_frontmost_application_pauses_the_shortcuts`, `a_probe_binds_the_combination_under_its_own_id`, `the_apps_that_pause_the_shortcuts_are_names_the_engine_reads`, `the_migrated_file_round_trips_through_the_rust_reader`, `every_shared_setting_is_carried_across`; `a_listed_application_in_front_releases_every_shortcut_until_it_leaves`, `the_frontmost_application_pauses_the_shortcuts_the_configuration_names`, `a_probe_binds_and_releases_the_combination_and_says_why_it_was_refused`, `the_recorders_probe_reaches_the_backend_over_ipc`; `a_combination_the_desktop_refuses_is_shown_and_recording_goes_on`, `the_recorder_shows_the_desktops_refusal_and_keeps_what_it_takes`, `a_chord_is_recorded_in_the_cpps_spelling`; `every_request_variant_round_trips`, `request_variants_are_exhaustive`, `response_variants_are_exhaustive` |

Declared differences:

- **GNOME switches workspaces; the C++ does not.** `GnomeWindowManager` lists GNOME's workspaces
  but leaves `focusWorkspaceSync` empty, so choosing one did nothing; the extension's
  `ActivateWorkspace` switches to it. An unnamed workspace is called by its number, as on the
  other compositors here, where the C++ says "Workspace N" (Mutter's own default name usually
  arrives first anyway).
- **Contract negotiation is a range, not an equality.** Before this pass the engine used an
  extension only at exactly its own contract version; it now uses any version from 3 to 4 and
  refuses per call what an older one lacks. `compass doctor` warns rather than fails for v3.
- **`inhibit_apps` accepts an id with or without `.desktop`.** The C++ compares the desktop file id
  exactly (`org.gnome.Boxes.desktop`); `org.gnome.Boxes` matches it too here.
- **KWin's focus is looked at twice a second** for inhibition (and for snippet expansion, which
  had no KWin path before): its tracker keeps windows in memory and signals nothing the engine
  can wait on. The toplevel list on wlroots and `WindowsChanged` on GNOME are waited on, as before.
- **The portal is not probed.** It binds a whole set behind the desktop's own dialog, where the
  person picks the trigger, so a probe would open that dialog for a throwaway shortcut; the
  recorder takes the combination and the desktop decides when it binds. No backend (X11, or
  none found) answers nothing, as `probeBind` does when `isSupported()` is false.
- **An engine that does not answer a probe within 3 s**, or cannot be reached, lets the recorder
  keep the combination, as it did before this pass.

VM tier (declared): the extension's `ListWorkspaces` and `ActivateWorkspace` against GNOME Shell 50
and 51 (`packaging/vmtest/checks.sh shell-extension` now calls both and gates on the reply's shape);
inhibition following focus on a real session; a compositor refusing a probe (no released one
carries `xx-hotkey-v1` or `vicinae-hotkey-v1`).

### Global shortcuts and the window manager, closed (2026-09-25)

"The gaps pass, GNOME workspaces and shortcut probes" left both rows amber for X11 alone, and the
product decisions made X11 n/a (Compass is Wayland only) in the same change, so both flip to
`Rust ✓` ✅ on the tests their sections name: for `src/services/global-shortcuts` the backends,
reconcile and conflict tests of "The gaps pass, global shortcuts" with the `inhibit_apps` and
probe tests; for `src/services/window-manager` the GNOME, wlroots, Hyprland, niri and KWin provider
tests with `on_gnome_switch_workspaces_lists_and_switches_through_the_shell_extension`. What stays
unverified here is VM-tier and declared in those sections: GNOME's portal grant and the extension's
workspace calls on real GNOME, a compositor refusing a probe, and real KWin.

### The window-material pass: an unsafe bridge (2026-09-25)

The last amber cell, `src/services/window-material` `Rust ✓`, against `ExtBackgroundEffectV1Manager`,
`WindowMaterialManager` and `WindowMaterialAttached`. The protocol client was ported in "The gaps
pass, HUD and onboarding"; what was missing was the launcher's own `wl_surface`, which winit makes on
its own connection and hands out only as raw `wl_display*` and `wl_surface*` pointers.

**The exception, and why it is justified.** No maintained crate adopts a foreign Wayland surface
safely (`CRATE-AUDIT.md`, "The window-material pass"), and winit, unlike `iced_layershell`, takes no
connection of ours, so the inhibitor's trick (a shared connection and `wl_keyboard.enter`) is not
available. The maintainer approved an `unsafe` exception, recorded as ADR-0019 and shaped like the
SQLCipher one: a dedicated crate, `compass-wayland-foreign`, that does not inherit the workspace's
`forbid`, sets `unsafe_code = "deny"`, restates the other lints, and allows `unsafe` in one function
(`adopt`) holding two blocks, `Backend::from_foreign_display` and `ObjectId::from_ptr`. Its API,
`bridge(&window)`, is safe: both handles are read from one window inside the toolkit's borrow; only
Wayland handles are accepted; the pointer's interface is checked to be `wl_surface`; a surface that
is not a `wayland-rs` proxy (whose destruction could not be followed) is refused; the
`client_system` backend is named so the wrong backend does not compile; missing libwayland is an
error; and one connection per display is kept for the process's life so its `Drop` never touches a
closed display. The invariant the types cannot express, that winit's display outlives our
connection, holds because the window is resident (ADR-0015) and the connection is used only inside
`iced::window::run`.

**Applying it.** `compass_platform::WindowMaterial` is the seam (`apply(window, region)`, the region
`None` to take the blur away), implemented by `compass::window_material::LauncherMaterial` over the
bridge and `compass_wayland::material::BackgroundEffects`, and handed to `compass_ui::run_resident`
by the binary as the shortcut inhibitor is to the layer shell. `compass-ui` wraps the card in a
sensor keyed on translucency and corner radius, so the card's size is reported when it is shown,
resized, or its look changes (the C++'s `widthChanged`/`heightChanged` and a new surface), and asks
through `iced::window::run` for the card's rectangle inside the shadow padding, rounded to the
preset's `card_radius`, only while the card is translucent (`launcher.appearance.tint`, the
`blurEnabled` gate); turning translucency off takes it away. A region equal to the last one asks
nothing; each new window (a hide destroys the surface) is asked again. `BackgroundEffects` now drops
the effects of destroyed surfaces before sending anything, since `set_blur_region` on one is
`surface_destroyed`, which on winit's display would end the launcher.

| Row | Flipped | Rust | Tests that would fail on a regression |
|---|---|---|---|
| `src/services/window-material` | Rust ✅ | `compass_wayland_foreign::bridge`, `compass::window_material::LauncherMaterial`, `compass_ui::material`, `LauncherApp::card_measured`, `compass_platform::WindowMaterial`, `compass_wayland::material::BackgroundEffects` | `a_toolkits_surface_is_bridged_into_a_usable_proxy`, `a_pointer_that_is_not_a_surface_is_refused`, `on_sway_the_bridged_connection_reports_no_blur_without_an_error` (headless Sway), `a_withheld_display_is_the_toolkits_error`, `a_display_of_another_platform_is_not_wayland`, `a_blurring_compositor_gets_the_region_once_per_change`, `a_destroyed_surface_is_never_sent_a_region`, `without_the_blur_capability_nothing_is_asked` (in-process compositor), `a_translucent_card_asks_for_blur_behind_itself_and_an_opaque_one_takes_it_away`, `a_closed_window_forgets_its_blur`, `a_translucent_card_is_blurred_where_it_is_drawn`, `a_window_that_is_not_wayland_is_unsupported_and_not_asked_again`, `what_the_client_did_is_what_the_window_is_told`, with the earlier `the_corners_are_cut_as_the_cpp_cuts_them` and `background_effect_is_bound_where_advertised_and_refused_by_name_where_not` |

Declared differences:

- **The layer-shell presentation is not blurred.** `iced_layershell` 0.19 drops `window::run`, so it
  lends no handles. The C++ blurs its layer surface too (`LauncherWindowLayerShell.qml`). The way
  there without `unsafe` is the inhibitor's: `BackgroundEffects` bound on the connection the binary
  already shares with `iced_layershell`, learning the surface from `wl_keyboard.enter`.
- **Real blur is VM tier.** Sway and Mutter have no `ext_background_effect_manager_v1`; KWin has it.
  Here the bridge is proved on headless Sway and the region traffic on an in-process compositor.
- **The gate is translucency.** The C++ blurs when `launcher_window.blur.enabled` and the compositor
  can; Compass blurs behind a translucent card (`tint`), which is where there is anything to see,
  and the C++'s opacity bump for blur (`BLUR_OPACITY`) is not copied: the card keeps `TINT_ALPHA`.
- **The region takes effect on the surface's next commit**, which winit makes on its next frame (the
  search field's caret blinks, so within about half a second); the C++'s Qt commits at once.
- **The shortcut inhibitor stays layer-shell only.** The same bridge could give it the
  `xdg_toplevel` surface; not done in this pass.

### Raycast extensions written for macOS (2026-09-25)

Compass-only, with no C++ counterpart: the C++ runs a Raycast extension's
`brew`, `open` and `osascript` as it finds them, inside no sandbox. Three
pieces, documented with the measurement behind them in
[RAYCAST-LINUX-SHIM.md](RAYCAST-LINUX-SHIM.md):

- The runtime's shim (`src/typescript/extension-manager/src/linux-shim/`):
  a Raycast extension's `open` runs `xdg-open`, `pbcopy`/`pbpaste` use the
  runtime's clipboard, `osascript` and other macOS-only programs fail by name
  rather than with `ENOENT`, and Homebrew's macOS paths resolve to Linuxbrew's.
  `process.platform` stays `linux`.
- The host-command broker (`compass::host_commands`, `HostCommand/run`): `brew`,
  for any extension, runs on the host as the engine's child once the person
  allows it (Allow Once, Always Allow, Deny), never by widening the
  extension's Landlock policy. "Always Allow" is kept in
  `$XDG_CONFIG_HOME/compass/host-command-grants.json` and listed and revoked in
  Script Permissions. The Qt engine refuses the call by name.
- The overrides manifest (`extensions/raycast-linux-overrides.json`): per
  extension, more host programs, path and command maps, load-time patches and
  install redirects.

IPC v22: `ExtensionAlert::remember_text` and `Request::ExtensionAlertRemember`
(the alert's third answer, Ctrl+Enter), and extension grants in
`ListScriptGrants`/`RevokeScriptGrant`. Raycast's Brew renders Show Installed
and Search against a Linuxbrew `brew`; Suite 1's ledger is unchanged, since its
corpus leaves macOS-only listings out.

### Earlier row notes

**`src/lib/xdgpp` → `compass-xdg`** — ported whole, so the row is green. The desktop-entry, locale,
value, reader and exec layers (47 C++ cases, verbatim inputs); the `DesktopFile` layer
(`compass_xdg::desktop_file`: `relativeId`, `fromId`'s two-candidate lookup and the standalone
filename id, 24 tests and 16 controls); `bookmark` and `file-uri`, reading and writing
(`compass_xdg::bookmarks`); `mime` reading (`compass_xdg::mimeapps`) and its writer,
`setDefaultApplication` (`compass_xdg::mimeapps_writer`, `mime.cpp`'s three writer cases verbatim);
the `xdg-terminal-exec` list, its `X-TerminalArg*` table and its writer, `setDefaultTerminal`
(`compass_xdg::terminal`, the three `xdg-terminal-exec.cpp` cases verbatim at the end of
`tests/terminal.rs`); and `env` (`compass_xdg::xdg_dirs`, the runtime directory being
`compass-ipc`'s). `special.cpp`'s four real-world files — Wine's escaped path, single quotes, empty
values, a locale with no translation of its own — are `tests/special.rs`. The two writers were the
last of it; Set Default Browser and Set Default Terminal call them (see `src/builtins/system`).


#### An unresolved disagreement: two desktop file id schemes

Porting `relativeId` turned up a disagreement between the two engines that nothing was recording.

The XDG Desktop Entry Specification says a desktop file ID is the path below the applications
directory with `/` turned into **`-`**. The C++ turns it into **`.`**. For a file directly in the
directory the two agree; for anything nested they do not — `kde4/konsole.desktop` is
`kde4-konsole.desktop` by the specification and `kde4.konsole.desktop` by the C++.

This matters because the id is the key. `xdg-app-database.cpp` keys every application by
`relativeId`, and that id is what an application's frecency score, alias, and enabled or disabled
state are stored under. `compass_xdg::scan` keys by `desktop_file_id`, which follows the
specification. **So the two engines would not find each other's records for any nested
application** — a user moving from one to the other would silently lose per-app state for anything
installed in a subdirectory, which is where distribution-packaged KDE and GNOME applications often
live.

**Resolved: the Rust engine keys on the C++'s dotted id.** `compass_xdg::scan::desktop_file_id` now
joins with `.`, and the application index, its tests and the parity tests follow.

The reasoning is which engine has users. The id is a key, not a display string — the separator is
never shown to anyone — and the C++ has already written these keys on real machines, while the Rust
engine has no tagged release and therefore no stored records to protect. Matching the specification
would have orphaned existing frecency scores, aliases and enable/disable state for every nested
application in order to fix a divergence nobody can observe.

The one cost is the ambiguity the dotted scheme carries: `kde4.konsole.desktop` could be a nested
`konsole` or a flat file of that exact name, and nothing recovers the difference. That needs a file
deliberately named to collide, which is a smaller risk than silently losing everyone's settings.

Inherited rather than endorsed. Both spellings live in `compass_xdg::desktop_file`, a test asserts
the scan and the C++ agree, and a second records that the specification's separator is deliberately
unused — so if the ids are ever migrated, the tests to change first are named.

The dotted scheme has a second property worth knowing if it is kept: because the `.desktop` suffix
is part of the id, it is indistinguishable from a separator. An id of `kde4.konsole.desktop` could
be a nested `konsole` or a flat file of that exact name, and nothing recovers the difference.

**`src/extension/api` → `compass-worker-host`** — the extension API's service adapters. `Storage`,
three of `OAuth`'s four, `UI/render`, `FileSearch/search` and all four `Clipboard` methods are
ported and pinned, as are all five `Application` methods, all four `Command` ones and all seven
`WindowManagement` ones, plus `Wallpaper/set` and both `BrowserExtension` methods: 33 of the 49
methods `figura/tsapi.fig` declares; with `UI`'s shell half that is 45 of 49. `UI/confirmAlert` joined them
once the host learned to hold a reply open: it is answered through `tsapi::Deferral` rather than by
a service returning a value, which is what a call that waits on a *person* needs. What is left is
`OAuth/authorize`, which needs a browser. Each sits behind a trait (`FileIndexer`, `Clipboard`, `Apps`,
`Commands`, `Windows`, `Wallpaper`, `Browser`, `Shell`) mirroring the C++'s own indirection, so the adapters are finished and tested while the backends they will call — the file
index, the Wayland clipboard, the launcher, the navigation and settings controllers — are still
ahead. A method not on `tsapi::IMPLEMENTED` answers with an error naming
itself rather than hanging the caller.

**`src/services/asset-resolver` → `compass-core::asset_resolver`** — ported **whole**, which is why
this row is green rather than amber: the base-path list, the first-match lookup, and the two
behaviours that make shared asset directories work — `addPath` does not deduplicate and `removePath`
erases one entry, so the first command to unload does not blind the second. The C++ singleton is an
ordinary value here; a singleton is how that file is reached, not what it does.

**`src/builtins/snippet` → `compass-core::snippet_form`** — the snippet form's validation and
`Expansion::validateKeyword`: a two-character minimum name, non-empty content with at most one
`{cursor}`, and a keyword that is optional but, when given, must be printable ASCII with no spaces
and at most 32 bytes. The two success toasts are not symmetrical ("Snippet updated" against
"Snippet successfully created") and are copied as they are. The store is
`compass-core::snippet_store` and Create Snippet / Manage Snippets run end to end (see "Snippets —
what the port does not have yet"); keyword expansion as you type is still C++-only.

**`src/builtins/shortcut` → `compass-core::shortcut_form`** — the quicklink form: what each mode
prefills (`Copy of %1` only when duplicating, the quoted navigation titles), the reverts to
`default` when the saved app or icon no longer exists, the three required fields — link, app and
icon, but **not** the name — the `default` icon being stored as whatever it resolved to rather than
as the word, the favicon-over-opener rule for `http*` links, and the three link completions with the
cursor offset that lands inside `{argument name="|"}`. Duplicating takes the *create* path, as the
C++ does by branching on `Mode::Edit` alone. The form, the favicon and Manage Shortcuts' panel run
in the launcher ("Shortcuts — what the port does not have yet"). Open with…, Manage Shortcuts'
detail pane and shortcuts as fallback rows landed in the views pass (see "The gaps pass, views").

**`src/builtins/font` → `compass-core::font_browser`** — the grid model's decisions are ported:
the category dropdown (only categories some installed font belongs to, "All" at index 0, the
index-minus-one arithmetic, and the remembered choice that is restored only when it is not "All"),
the two headings with their counts, the search that scores the display name alone, the missing-glyph
placeholder and the colour-font rule that leaves an emoji font untinted, and the action panel whose
*primary* action is Preview rather than apply. The thirty-three-category table itself belongs to
`src/services/font-service`, which is its own row. The grid (six columns), the specimen view,
"Set as vicinae font" and the remembered category are in the launcher; what differs is under
"Browse Fonts" below.

**`src/builtins/developer` → `compass-core::create_extension`** — the Create Extension form's
validation and what follows it: all six checks run every time so every mistake shows at once, the
description is held to 16 characters where the rest need 3, the location is the one check that asks
the filesystem, `expandPath` handles `~` and `~/` only (so `~root/x` is taken literally and fails),
and a success *replaces* the form on the navigation stack rather than stacking on it. **The row is
green**: the form, the boilerplate generator and the success view run in the launcher, with what
differs declared under "Create Extension" below.

**`src/builtins/theme` → `compass-core::theme_picker`** — the list model and the view's own logic
are ported: the current/available split (and that the configured theme is filtered out like any
other when it does not match), the sort that only happens when something is typed, the name/
description weights with the id *not* searchable, the `Default theme description` fallback
subtitle, the eight palette swatches in the row's order, the action panel's two conditional
actions, and the live preview — selecting a row applies the theme and leaving the view puts the
configured one back. The view, the swatches and the theme files are in the launcher and
`compass-core::theme_file`; what differs is under "Set Theme" below.

**`src/builtins/power-management` → `compass-core::power_commands`** — the catalogue and the run
plan are ported: eight commands in registration order with their titles, long descriptions and
keywords, the `confirm` preference (on for everything but Lock), the `customProgram` escape hatch
that exists only where a shell makes sense, and the two failure messages per command — whose
"can't" / "cannot" wording is inconsistent and stays that way, because these strings are
translated. The logind calls behind them are `compass-power`. **The row is green**: the plan runs
end to end — the launcher asks in a dialog when the `confirm` preference says so
(`a_power_command_asks_first_and_only_a_yes_runs_it`,
`the_confirm_preference_decides_whether_a_power_command_asks`), the engine runs `customProgram`
with `$SHELL -c` on the host in place of logind when one is set
(`a_power_command_with_a_custom_program_runs_it_instead`), and a refusal comes back as the command's
own sentence (`a_power_command_answers_with_its_own_sentences_and_never_touches_this_machine`). Both
preferences are read from `providers.power.entrypoints.<id>.preferences`, where the C++ keeps them;
there is no settings page to edit them yet.

**`src/builtins/system` → `compass-core::browse_apps`** — the "Search Applications" builtin's
*model* is ported: the field weights (name 1.0, description 0.5, keywords 0.3 — **not** the root
list's 0.6), the `Hidden` accessory for a `NoDisplay` entry, and the action panel as data: focus the
first open window if there is one, open (clearing the search), each desktop action with
`control+shift+1..9` for the first nine only, then open-location behind the `action.open` keybind,
copy id, copy location. Run Terminal Program is in the launcher ("System: Run Terminal Program"
below). Since the truth pass the row is green: the Browse Apps view over this model, Set Default
Browser and Set Default Terminal are in the launcher (see "Gaps closed after the truth pass").

**`src/services/shortcut` → `compass-core::shortcut`** — `Shortcut::parseLink`'s state machine and
`insertPlaceholder`'s argument rules are ported: literal text and placeholders in order, reserved
ids that expand on their own, `name=` / `default=` with and without quotes, and the two behaviours a
rewrite would "fix" by accident — a repeated key keeps its **first** value (`std::map::insert`), and
a link that ends inside a placeholder loses everything from the opening brace. The store behind it is ported
too (`compass-core::shortcut_store`): the JSON file, the 10,000 limit, the two different not-found
sentences, the rollback when a write fails after the list already changed, and the `value_or({})`
that turns a corrupt file into an empty list rather than a refusal to start.

The migration from the old `OmniDatabase` and `resolveApp` are ported too — 15 more tests and 15
controls, closing this row's named gap. The migration's three outcomes are kept apart because they
mean different things: a **missing table** is every installation that never ran the old version, an
**empty table** writes nothing *at all* (an empty write would still create the JSON file and make
the next start think a migration had happened), and rows are written whole — including
`last_used_at`, the one nullable column, where turning a null into 0 would make a shortcut nobody
has opened look used at the epoch. It runs only into an empty store: anything already there has
been migrated or used since, and re-running would duplicate every shortcut or overwrite work done
after the move.

`resolveApp` narrows in three steps. A shortcut naming an application uses it even if that
application is gone — the lookup returns nothing and the caller reports it, which is better than
silently opening something else. A shortcut set to the default uses whatever opens *that target*,
which for a `mailto:` is the mail client; the web browser is the last resort and not the rule,
because it is right for a quicklink and wrong for anything else. That middle step only became
portable once the MIME parent-chain walk landed.

**Two claims were written into this port and then removed, because no mutation could make a test
fail on either**: restoring the previous list when the write fails, and a comment saying the
write-then-reload order was load-bearing. Neither is observable, for the same reason — the migration
runs only into an empty store, so there is no previous list and no good file to clobber. The order
still matches the C++; it is just not doing the work the comment claimed. Recorded because a comment
that overstates what a line does is the kind of thing that survives review and then misleads
whoever changes it.

**This row is green.** `ShortcutService` — the in-memory list the launcher reads from — is ported as
`compass-core::shortcut_service`, and with it the last of the directory. 23 tests, 18 controls, 17
of which fired.

The list holds each quicklink with its link **already parsed**, which is the reason it exists: every
row that draws a quicklink needs its arguments, and re-parsing on each keystroke of a search would
parse every link in the store on every keystroke. `lastUsedAt` stays absent when the stored value is
absent — turning it into 0 would date a quicklink nobody has opened to the epoch, which reads as
opened rather than as never opened.

One rule arranges the whole type: **the file is written first, and the list changes only if that
succeeded.** A list showing an edit the file does not have is a launcher that forgets the edit at
the next start and cannot say why. Three controls cover it — a create, an update and a visit the
file refused, each of which must leave the list as it was.

**Three guards were removed because no mutation could make them fail.** The C++ looks the entry up
in its list *after* writing to the database and returns false if it has gone; that is a check for
the service and the database having drifted apart, and it is real there because they are separate
objects. Here the service owns the store, so there is nothing to drift: both entries are located
before anything is written, and read back afterwards without a second check. The conditional around
`lastOpenedAt` went the same way — after a successful visit the stored value is always present.

Two fixtures were too small and controls said so: with one shortcut in the store, writing to the
first entry and writing to the *right* entry are the same thing. Both now hold two, and the one that
was not touched is asserted on.

One thing is **not** claimed: looking the id up before the write rather than after it. A control
could not make it fail, because the store refuses an unknown id anyway, so it is a shape rather than
a behaviour.

**`src/data-control-server` → `compass-wayland::data_control`** — the part that decides which of a
Wayland client's offered MIME types belong in a clipboard entry, and which of them get their bytes
read over a pipe. Ported: the accept test (`text/`, `image/`, `application/`, plus
`x-special/gnome-copied-files` by name) with `application/x-qt-image` checked against the ignore
list **first**, so the hint that an image exists does not survive its own prefix; the flag types,
kept for their presence and never read, which is the only way a password hint survives at all;
the single-image rule, where a later encoding must be **strictly** better ranked to displace an
earlier one, so two encodings the preference list does not name keep the first offered rather than
the last; and the drop of `text/plain` when `text/plain;charset=utf-8` is also there, matched on
that one spelling and not on any charset. The primary selection is much stricter and is ported that
way: a concealed selection is dropped without being read, only plain text is kept (UTF-8 first, and
the loop breaks so both spellings never both appear), and a selection over 1 MiB is dropped rather
than truncated *and not retried* with the other spelling.

One test here is not control-backed and says so in place: the offers come back in lexicographic
order because the C++ collects into a `std::set` and the port returns a `BTreeSet`, so the ordering
is a guarantee of the type rather than behaviour a mutation could change. **The row is green**: the
Wayland plumbing is `compass_wayland::clipboard` — `ext-data-control-v1`, else
`zwlr_data_control_manager_v1`, in the engine's process rather than a helper's — tested on headless
Sway (`a_copy_is_seen_by_the_watcher_and_round_trips_through_wl_clipboard_rs`,
`a_password_manager_copy_is_marked_concealed`,
`the_primary_selection_reads_back_and_is_not_the_clipboard`). What it records differently is
declared under "wlroots" #4.

**`src/builtins/vicinae` → `compass-core::{emoji_grid, bug_report}`** — the largest builtin
directory (2,800 lines across 45 files). Two of its pieces are ported, the two with arithmetic and
rules in them rather than view plumbing.

**The emoji picker.** A skin tone goes after the **first** codepoint, not at the end, because in a
person-joined-to-an-object sequence the tone belongs to the person. Every variation selector in what
follows is then stripped, and the C++ comment gives a reason that is not cosmetic: a tone modifier
already forces the coloured presentation, and leaving the selector in produces a sequence some fonts
refuse to compose, so the glyph breaks apart into its parts. The `Default` tone carries an **empty**
modifier, which is what makes applying it a no-op rather than a special case in the applier. A
glyph's own remembered tone beats the picker's, because someone who set a tone on one glyph meant
that glyph. The copied codepoint is the **first** one only — a sequence reports the codepoint of the
thing it is a sequence *of*, which is the number someone looking it up wants.

The tone menu's two exclusions each do something. Skipping the tone already in force keeps the panel
from offering to do nothing; skipping the picker's default keeps it from offering a second route to
what the "reset to preference" row already does — and that row only appears when the glyph is
actually overriding, because otherwise there is nothing to reset to. Only copying and pasting
register a visit: copying a glyph's *name* or codepoint is looking something up, and counting it
would let a search for a name drift the picker's ordering.

**The bug report.** The template was copied out of the C++ mechanically and its measurements are
pinned. An extension bug goes to a different repository *and* a different path
(`/issues/new/choose`, not `/issues/new`), because that repository offers templates. An empty title
is left out rather than sent empty, which would leave GitHub's own placeholder unused and the field
looking filled in. The two OS descriptions are deliberately different shapes — a dash between the
os-release fields, parentheses around the architecture — so a reader can tell which one they are
looking at.

**The fallback manager.** A command that cannot be a fallback appears in *neither* list: the manager
is not a list of everything with a switch beside it, and showing commands that cannot be turned on
would be showing switches that do nothing. The enabled list is ordered by the stored fallback order
rather than by relevance, because a fallback's position decides which of them answers a query first
— any other order would show a ranking that is not the one in force.

The emoji picker and both stores are in the launcher now, the picker with the visits, pins, tones
and keywords `glyph-service` keeps, and its paste action. Still C++-only: the
installed-extensions list, the OAuth
token store and local-storage browser views, the builtin-icon gallery, the fallback
manager's view, and the small commands (report a bug, refresh apps, open the config file, the
store's intro page).

**`src/builtins/root` → `compass-core::root_view`** — the root list's own behaviour: the clock in
the title bar, the space-bar alias shortcut, and reaching back through past searches with the up
arrow.

The clock's next tick is `interval - (now % interval)`, which lands on a multiple of the interval
rather than one interval from now — so a clock showing minutes updates *on* the minute instead of
drifting to whenever the window happened to open, and one re-enabled mid-interval falls back into
step with a single short tick. Turning it off clears the title as well as stopping the timer, which
matters as much: leaving the last time on screen would show a clock that had silently stopped.

The space-bar shortcut fires only when what was typed *is* the selected item's alias, compared
lowercased. With a completer open the space goes to the completer instead — but only while every
completion field is still empty, because once something has been typed into one, stealing the space
would make the arguments unwritable. Without a completer the item has to support the shortcut; a
no-view command does not, as there would be nothing to show for it. The completer branch returns
*before* the support check, so a completer overrides the item's own answer either way.

Reaching history with the up arrow needs two things, and the first is a **conflict rather than a
preference**: wrapping navigation makes the up arrow at the top jump to the bottom, so it cannot
also mean "previous search" — where wrapping is on, history is unreachable by design. The second is
that the selection is already on the first row. The first press takes offset 0, so one press reaches
the last thing typed, and the skip past an entry equal to the current text is a **loop**: history
can hold the same query several times in a row, and stopping after one would leave the arrow doing
nothing on the second press.

A control found a gap the suite had: nothing covered an empty query against an item with **no**
alias — the one case that separates an absent alias from an empty one, where treating the two alike
would make every press of space over an empty search box activate whatever was selected. There is a
test for it now.

Root search now ranks applications, builtin commands, extension commands, scripts, shortcuts and
Rhai scripts, with the calculator and the fallbacks. Favourites, the clock, the space-bar alias,
the up-arrow history, the alias form, the row's panel, the provider search view and every kind of
fallback have landed since (see "The gaps pass: glyphs, clipboard, root" and "The gaps pass, root
and actions").

**`src/builtins/clipboard` → `compass-clipboard::history_view`** — the history command's own
decisions. The ledger had this row down for `compass-core`; it landed in `compass-clipboard`
instead, because everything it decides is decided *about* `OfferKind` and `EncryptionType`, which
live there — and `compass-core` does not depend on that crate. The crate column was a plan; the
types decide.

The query controller is **single-flight with one waiting slot**, and the port keeps both halves. At
most one query runs and at most one waits; a third request while one is running collapses into the
same slot rather than queueing, because only the newest matters. And a result that arrives while
something newer is wanted is **dropped without ever reaching the list** — the C++ comment says why,
and it is not about wasted work: each delivery consumes the view's one-shot "select the first row"
flag, so a stale delivery would move the selection out from under whoever was reading. The same
flag is what makes every delivery after the first *incremental*: a pin, a rename, or a new copy
landing while the list is open leaves the selection where it is.

The action panel is gated in three independent ways. An entry whose contents cannot be read — an
encrypted one before the keyring is unlocked — offers a way into the settings and **neither** copy
nor paste, rather than an action that would fail. Where pasting is unsupported only copying appears.
And where both appear, the stored preference decides only their *order*, the first being what the
return key runs; only the exact string `paste` selects pasting, and everything else falls back to
copying, which is the safer of the two to get wrong.

Opening is ported with its asymmetry. A file entry holds a URI list, and open actions appear only
when it holds **exactly one** entry that still exists — a copy of three files has no single thing to
open, and a copy of one that has since been deleted would offer to open nothing. A link needs no
such check, because the payload *is* the target. In both cases the chooser appears whenever the
target is usable and `open` additionally needs a default application, so a file type nothing claims
still gets a chooser.

The filter's stored vocabulary is the enum's and not the interface's — the option reads `Images` and
stores `image` — and keeping them apart is what lets either change without the other. A kind with no
option (`Unknown`, and the enum's count sentinel) answers index 0 rather than an out-of-range index
that would select nothing.

One fixture was too permissive and a control caught it: an `exists` stub that accepted every path
let a mangled path through, so splitting the URI list on the wrong separator — which leaves a stray
carriage return — looked correct. The stub now names the paths it knows.

Clipboard History is in the launcher: search, copy, paste, pin and remove
(`compass-ui::clipboard_page`), and since the gaps pass the kind filter, the detail pane, keyword
editing, remove-all and the monitoring switch (see "The gaps pass"), and since the views pass Open
and Open with… (see "The gaps pass, views"). The drag payload is a declared difference, blocked
because Iced has no drag-and-drop out of its window.

**`src/builtins/raycast` → `compass-core::raycast_store_view`** — the store's two views. Its API
client was already ported (`compass-core::raycast_store`); this is what the views do with what it
returns.

The list is a **rendezvous, not a sequence**. The page and the compatibility sheet are fetched
independently and may finish in either order, and whichever lands second is what draws the list —
drawing on the page alone would show every row with no badge and then flicker when the sheet
arrived. The two stale-result guards differ correctly: the browsable list is only ever shown for the
empty query, so it checks that the box is *still* empty, while a search compares against the query
it was sent for.

Three distinctions in the compatibility handling are ported because each means something different.
A platform with **no sheet** shows no badge at all, which is not the same as an `Unknown` badge —
one says the question does not apply, the other that it was asked and not answered; the view model
separates them with `-1`. An extension the sheet does not mention gets a *different* muted banner
from one the sheet mentions with an unrecognised status: "may or may not work" against "no data is
available". They look identical and read differently, and only the first can be fixed by someone
adding a row to the sheet. And an unrecognised status is `Unknown` rather than an error, so a status
the sheet gains later degrades instead of breaking.

`formatCount` is ported with its arithmetic intact: both thresholds are **strict**, so exactly 1,000
prints as `1000` and 1,000,000 prints as `1000K`; and the figure is rounded to one decimal by
*ceiling*, so 1,001 downloads reads as `1.1K`. That is generous and it is what ships.

**Ported as-is rather than fixed:** a failed fetch leaves the spinner running. Both handlers report
the failure and return before clearing the loading state. It is visible behaviour, and correcting it
here would make the two implementations disagree while the C++ is still the one shipping, so it is
pinned by a constant and a test instead.

One line was written and then removed because a control could not make it fail: a special case for
printing a whole number without its fraction. Rust's `f64` `Display` already does that, the same way
`QString::arg(float)` does.

**The row is green**: the HTTP calls are the engine's (`compass::stores`, over `ureq`) and the views
the launcher's, with what differs declared under "Extension Store and Raycast Store" below
(`the_raycast_store_badges_compatibility_and_notices_an_update`,
`a_deeplink_opens_the_detail_page_and_uninstalling_asks_in_a_dialog`). The install-from-zip path
is ported in `compass-core::extension_install`, below.

**`src/services/extension-registry` → `compass-core::extension_install`** — `installFromZip`. The
archive arrives from the network, so the whole shape exists for one property: **a bad download must
not destroy the installation it was going to replace.**

The module returns the install as an ordered list of steps rather than performing it, because the
ordering *is* the safety and nothing else about it is interesting. Clear the staging directory, so a
previous install that died part-way cannot mix two extensions into one; unpack into staging, never
into the target; check the manifest while still in staging, so a truncated archive is discarded with
the installed version untouched; only then remove the target; and rename staging into place, which
is atomic within a filesystem, so there is no moment where the extension is half-written. Staging
sits *beside* the target rather than in a temporary directory, because a rename across filesystems
is a copy that can fail halfway; the leading dot keeps it out of the registry's own listing.
`strip_components` is 1 because a published bundle wraps everything in a directory named after the
extension, and without stripping it every extension would install one level too deep and its
manifest would never be found.

Cleanup removes the staging directory and **only** the staging directory, for every failure.
Reaching for the target in a cleanup path is exactly how a working extension gets deleted because
its replacement was broken. `may_have_removed_previous` answers true for a failed rename alone — the
one failure that happens after the target is removed — because "the install did not happen" and "the
extension is now missing" send someone looking in different places.

The download, the install and uninstall and the rescan after either now run through the stores
(`the_vicinae_store_lists_installs_into_root_search_and_uninstalls`). The directory watch that
notices an extension appearing outside the store, such as a developer's build, landed after the
truth pass and turns the row green (`compass::catalog_watch::watch_extensions`; see "Gaps closed
after the truth pass").

**`src/builtins/internal` → `compass-core::internal_commands`** — a hidden extension holding one
command: a fixed Markdown document rendered to check that every construct the renderer claims to
support actually renders. This row is **green rather than partial**, because there is nothing else
in the directory — the view that displays the document is shared with the store intro and belongs to
that row.

The document is a *test fixture that ships*, and it is treated as one. It was copied out of the C++
raw string literal mechanically rather than retyped, because a fixture whose job is to exercise a
renderer is exactly where a character typed wrong still looks right in review; its byte and line
counts are pinned so a later edit cannot quietly resize it. The tests then enumerate what it must
contain — five heading levels, the five inline styles, both kinds of line break, six labelled code
languages *and* an unlabelled fence, both list kinds, a table with all three column alignments,
inline formatting inside a list item and inside a table cell, both blockquote shapes including the
empty quoted line that makes a second paragraph, all five callout kinds, a plain image and an image
wrapped in a link, a horizontal rule, and the closing line that is the only way to see from the
rendered page that nothing was truncated. Each of those is a separate path through the renderer, so
a missing one is a missing code path rather than a missing sentence, and every one of them has a
control that deletes it from the document.

One thing is kept as it is: the extension's display name and description are the same string in the
C++. It is not meant to be found by searching, so a description distinguishing it from its own name
would be describing it to nobody.

**`src/builtins/file` → `compass-core::file_search`** — when a query is read as a path rather than a
search, which of three result modes the view is in, how a late answer is discarded, and how the
category filter is stored and read back.

The path test is deliberately anchored: `~`, `.` and `..` count only as the *whole* query, and the
prefixed forms need their separator, so `~notes` stays a search rather than becoming a home-relative
path that does not exist and `notes..txt` is not mistaken for a traversal. The direct-path branch
then needs three things at once — the text must look like a path, the path must exist, and it must
not be `/`. That last one matters: the root exists everywhere, and matching it would turn a single
slash into a one-item list instead of a search for names containing a slash. When a category filter
excludes the named file the result is an **empty** direct-path section rather than a fall-through to
the index: the user asked for that file, and answering with a list of other files would be a
different question.

The three stale-result guards are ported as three, because each catches something the others do not:
the task may have been cancelled, the view may have changed mode (a direct path typed while a search
was in flight), and the query may have moved on — the last being what stops out-of-order answers
leaving the list showing a question already finished with. Recent files are guarded differently and
correctly so: they are only ever requested for the empty query, so the test is that the box is
*still* empty rather than that it matches a remembered string.

The filter stores the **untranslated** key, so a filter chosen in one language is still readable in
another, and `restored_filter_index` folds three cases into one: nothing stored, an unknown value,
and `All` all restore nothing — which is what the C++'s `index <= 0` means, `-1` being not-found.

Two lines are noted rather than silently kept or dropped. The C++'s empty-string guard in the path
test is unreachable here (none of the tests below can match an empty string) and is left out rather
than carried over as a line no mutation can reach. The `index >= 0` bound check *is* kept although
the cast to `usize` already covers it, because it says what is meant and survives a future signed
comparison. Rebuilding the index is written and deliberately unregistered in the C++, with a comment
saying the indexer's timed sweeps and deleting its cache directory have the same effect; the port
keeps it unregistered for the same reason, and a test pins that.

The indexer behind it is ported (`src/file-indexer`, now green). The rest of the action panel and
the loading indicator landed in the views pass (see "The gaps pass, views"); the drag payload is a
declared difference there, Iced having no drag out of a window.

**`src/builtins/media` → `compass-core::media_commands`** — which commands exist on which platform,
how a player is chosen from what was typed, what the on-screen display says, and which speaker glyph
goes with a volume. The player commands need MPRIS and are registered only where it exists; the
volume commands go through the audio service and are registered everywhere, so a platform without
MPRIS gets a shorter list rather than commands that fail.

`trackLabel` narrows in a fixed order and the order is only visible in one case: with a title but no
artist, or an artist but no title, either order gives the same answer — it is the player with
*neither* that shows it, where testing the artist first would return the empty title and leave the
row blank. A test covers exactly that case. Choosing a player is two different things: an empty
query takes the player the media service already considers active, and anything else is a search.
Their two failure messages are different sentences on purpose — one is a fact about the system, the
other quotes the query back, which is what says "you misspelled it" rather than "your music
stopped".

The play/pause message is built from the state *before* the toggle, because reading it back after
would race the player's own reply. A skip asks the player whether it can before calling, so the
refusal names the player instead of reporting a bare failure.

The four speaker bands put each boundary in the *lower* band, and silence is its own case rather
than the bottom of the first, so a muted system shows a crossed-out speaker and not a quiet one.
The percentage is rounded half away from zero, the way `qRound` does and unlike Rust's default
`round`-to-even would be if written casually — so a nudge that changed something never reads as if
it changed nothing.

Two things are ported as they are rather than tidied:

- The preset list disagrees with `volumeIcon`. 50% is listed with the *low* speaker while
  `volumeIcon(0.5)` returns the *down* one. The list is written out by hand in the C++ and drifted
  from the function. Changing either changes what someone sees today and neither is more right, so
  both are kept and a test pins the disagreement.
- `volume-down 5` turns the volume **up** by five. Both commands share one `adjustVolume` call and
  neither negates its argument, so the defaults are the only thing carrying the direction. Pinned
  rather than quietly corrected.

The MPRIS provider is `compass-media`, the audio provider the ported `pactl` adapter, and Now
Playing a launcher view (`compass-ui::media_page`, IPC v16 `ListMediaPlayers` and
`ControlMediaPlayer`); the `player` and `step` arguments reach the engine as
`RunMediaCommandWith`. What still differs is declared under "Media commands" below.

**`src/builtins/wm` → `compass-core::window_switcher`** — which commands the window-management
extension offers and how a window and a workspace are described in the list. Switching windows is
unconditional; everything else is gated on a capability, because a command certain to fail is worse
than a command that is absent. The C++ also tests `SetSticky` and registers nothing in that branch;
the port keeps the empty test rather than tidying it away, because the capability *is* used — a
window's action panel offers pinning when it is present — so the branch is the only written record
that someone meant a command to go there.

A window's row falls back twice and the two are independent: the subtitle is the application's
display name or the raw `WM_CLASS`, and the icon is the application's or the generic window glyph.
The accessory has three distinct cases, not two: a named workspace shows its name, a numbered one
shows `WS n`, and a window on no workspace shows nothing at all — an absent accessory is not the
same as `WS ` with nothing after it. The rule that decides between the first two is the subtle one:
a compositor with no workspace names reports the **id as the name**, so a name equal to the id is
treated as no name, which is what keeps a bare `3` out of the slot where a name belongs. Searching
keeps the `WM_CLASS` at low weight even when an application was recognised, so someone who knows a
window as `org.gnome.Geary` still finds it when the desktop entry calls it Mail.

A workspace's applications are deduplicated but its windows are counted, so three terminals show one
icon and the subtitle still says three; an unrecognised window is counted and contributes no icon.
The applications are searchable at low weight, which is what lets someone find "the workspace with
the browser on it" without knowing its name. The Windows/other naming split is a compile-time
`#ifdef` in the C++ and an argument here, so both namings are reachable from one build and both are
tested.

**A declared divergence.** The C++ writes the count as Qt's `tr("%n window(s)", "", n)`. Qt applies
plural forms only where a translation supplies them, and there is no English entry for this string —
Russian and Ukrainian have real plural forms and English falls back to the source text. The shipped
English therefore reads `3 window(s)`, with the translator's placeholder left in the interface. The
port writes `1 window` and `3 windows`: what every translated locale already does, and what English
would do if the entry existed. A test pins it.

Switch Windows runs end to end over the ported providers (GNOME through the Shell extension, the
wlroots foreign-toplevel list, Hyprland and niri). Switch Workspaces and the toggle-floating,
toggle-fullscreen and toggle-overview commands landed in the views pass (see "The gaps pass,
views").

**`src/builtins/calculator` → `compass-core::calculator_history`, and the grouping in
`compass-local-storage::calculator`** — the view's own decisions and the half of `CalculatorService`
that is not persistence. The live-calculation gate is ported with both of its rules: three
characters before the search box is also read as a sum (below that almost anything parses as
*something*, and a result flickering in on the way to typing a word is worse than none), and a
leading `=` that gets under the length rule entirely and is stripped before the rest is computed —
including the bare `=`, which the C++ hands to the backend to decline rather than short-circuiting.
The length is counted in characters, so a two-character accented word does not slip through a byte
count. Also ported: the `question = answer` title with its spaces, the conversion/arithmetic icon
split with its `default` arm, and both action panels as sections — pinning alone, the three copies
with **the answer** primary, then the two destructive actions behind a section break, which is the
only thing standing between them and the primary action.

`group_records_by_time` is a **sequential scan, not a classification**, and the port keeps it that
way because the difference is visible. Each group consumes a *prefix* of the rows and stops at the
first that does not match, leaving the rest to the next group; nothing rewinds. Two things follow.
A row older than every boundary reaches `A few years ago` without any group needing a lower bound.
And a row out of order cannot go back to an earlier group — which is safe only because the query
sorts `pinned_at DESC, created_at DESC`, so the `ORDER BY` and this scan are one mechanism and not
two. Every group is produced on every call, including the empty ones, and the view drops those;
that split is the C++'s and is pinned on both sides. `query` is a filter and not a ranking, and an
empty query short-circuits to the full list rather than matching everything — which is what makes
an empty search box show the grouped history rather than a fuzzy-ordered one.

The calendar arithmetic is **not** ported: `group_records_by_time` takes the eight boundary instants
as an argument rather than reading a clock. Computing them belongs to whoever owns the clock, and
keeping them out is what lets the scan be tested without freezing a timezone. The `dividers` vector
the C++ declares at the top of that function is dead — nothing reads it — and is not carried over.
The history view landed in the gaps pass (below). Not ported by design: the backends and the
preference dropdown that selects one (fend answers instead, see the crate docs). Still C++-only:
currency conversion and the refresh-rates command, blocked on a rate source.

**`src/services/script-command` → `compass-core::script_scan`** — the header parser was already
ported (`src/lib/script-command`); this is the layer around it. The scan's rules are ported with
their order intact, and the order is observable: a directory is classified **before** the duplicate
check, so a directory never consumes an id and one named like an already-seen script still
contributes its children; and the duplicate check comes **before** the extension check, so nothing
a rejected `.md` does can shadow a real script of the same id found later in the walk. Also ported:
the `.template` marker matched **anywhere** in a name rather than only as a suffix, the
`depth + 1 < MAX_DEPTH` comparison that lists a directory at depth 4 without opening it, the
"is this text?" test that is nothing more than a NUL byte in the first 8 KiB (so an empty file
passes and reaches the parser), custom directories searched before the packaged ones so a user's
script shadows a stock one, and the case-**sensitive** extension check that lets `README.MD`
through. The command line is ported with its zip: extra values are dropped rather than appended and
a short call passes fewer arguments rather than empty ones, and `percentEncoded` is applied per
argument over bytes, not characters. `packageName` shows an inline script's last line of output —
`No data` until it has run — in the slot a package name would occupy, so declaring one on an inline
script has no effect. The icon chain is emoji, path as written, path beside the script, `https`
URL, then the tinted `code` glyph; `http` is refused because the C++ tests the scheme for `https`
exactly. The metadata store keeps each line base64-encoded for the reason the C++ comment gives —
the output is arbitrary bytes from someone else's script — and a corrupt file leaves an empty store
rather than an error, because a lost output cache is no reason to stop listing scripts. The output tokenizer is ported too, as
`compass-core::script_output`: 32 tests, 18 controls, all of which fired.

A script's stdout is arbitrary bytes from someone else's program and the launcher renders it, so
this decides what is a link, what is coloured, and what is neither. Three of its rules are the kind
that only a test notices:

- Three of the four states end a run **without consuming the character that ended it**, so the next
  call sees it again in the new state. That is what starts a link at the `h` of `http` rather than
  one character late, and what keeps the character after a link from being dropped.
- A link that runs to the end of the output is **not** marked as a link — the loop falls out of the
  end and returns the run with the flag unset. Visible behaviour: the last link in a stream is not
  clickable until more output arrives after it. Pinned rather than fixed.
- Quotes and brackets end a link, because a URL printed inside them — how most prose prints one —
  would otherwise swallow the closing mark and produce a link that 404s.

37 is deliberately absent from the colour table, so white text draws in the theme's ordinary
foreground; a script colouring its output white would otherwise be invisible on a light theme. Two
arms of the C++ switch, for 0 and 97, **cannot be reached** — everything is normalised into 30–37
before the lookup — and are not ported.

**Three controls were silent and each needed an input that could tell the two readings apart.** Two
sequences with no text between them is the only case where the one-format-per-run check does
anything, because with text between them the text flush ends the run first. `ESC 3 1 m [ 3 2 m` is
the malformed escape that distinguishes "swallow everything until `[`" from the other reading.
And `ESC[287m` is red, because the accumulator is a `uint8_t` and 287 wraps to 31 — with 999,
wrapping and saturating are indistinguishable.

**The service row is green**: the scan runs in the engine at start and on each summon, which
replaces the watcher, its 100 ms debounce and the 15-minute refresh (declared as "Script commands"
#3), and the executor is the launcher's script view
(`script_commands_are_scanned_searched_and_run_in_their_modes`,
`the_scan_finds_scripts_ids_them_by_path_and_lets_custom_dirs_win`). What a script's root row does
not have yet — its two file actions and its own icon — is listed there too, as #6 and #9.


**`compass-xdg::terminal`** — how to run a command inside a terminal emulator, from
`XdgAppDatabase::inferTermExec` and the `X-TerminalArg*` keys.

There is no specification for any of this, so it is a table of what each emulator actually accepts,
and the gaps in it carry as much weight as the entries. An absent flag means *this terminal has no
such flag*, not "use the default": passing `--title` to konsole is an error and no window, not an
untitled one, so the absence has to survive into the caller. Two entries are the kind of thing a
tidy-up would break — the new GNOME Console takes `working-directory` with **no leading dashes**,
and `mate-terminal` and `xfce4-terminal` kept `-x` where GNOME moved to `--`.

The table is keyed on the *program* rather than the desktop id, because the same emulator ships
under different ids on different distributions while the binary keeps its name. A terminal's own
desktop file beats the table, since it is the only source that can be right about one released after
the table was written; `X-TerminalArgExec` is the gate, because a file that cannot say how to run a
command is not describing a terminal this can drive, whatever else it declares.

The fallback guesses `-e` and **nothing else**, deliberately: a terminal nobody has listed still
opens, it just gets no title, directory or hold. Guessing more would not be an improvement — a wrong
`--title` is an error and no window, where a missing one is a window with the wrong name.

**`src/services/app-service` → `compass-core::app_service`, with the MIME hierarchy in
`compass-xdg::mime_subclasses`** — the lookups are ported:
`findById` (with its `.desktop` retry), `findByClass`, `find`'s id-then-class order,
`findCuratedOpeners`' dedupe by display name, and `list`'s case-insensitive sort. Launching, the
file browser (`ShowItems`) and the terminal (`compass-xdg::terminal`) are the engine's now.
The rest landed after the ledger truth pass, which turns the row green (see "Gaps closed after
the truth pass" above): the watch on the application directories (`compass::catalog_watch`,
`AppIndex::rescan_applications`), and the text-editor, file-browser and web-browser lookups with
`setWebBrowser` (`compass::extension_apps::EngineApps`). Two things differ, both declared there.

`findOpeners` / `findDefaultOpener` are no longer among them. The per-type lookup was already in
`compass-xdg::mimeapps`; what was missing was the **parent-chain walk**, which the C++ gets from
`QMimeDatabase` and which now has its own shared-mime-info reader in `compass-xdg::mime_subclasses`
— 30 tests and 17 controls.

Without the walk, a `.tar.gz` typed as `application/x-compressed-tar` finds nothing unless something
registered for that exact name, even with an archive manager installed that registered for
`application/gzip`. The walk is breadth-first, so a type's own associations are considered before its
parents' and a near ancestor before a distant one — which is what makes a reader registered for
`application/pdf` beat one registered for `application/octet-stream`. An application claiming both a
type and its parent appears once, in the position its *nearest* claim earned.

The table is parsed forgivingly: a malformed line is skipped rather than refusing the file, because
it is generated by `update-mime-database` from whatever packages installed and one bad line should
not cost every association on the system. Comments need their own check and not just a field count —
a two-word comment has exactly the two fields a real line has, and would otherwise register `#` as a
type.

**A declared divergence.** The C++ keeps no record of which types it has already walked. A type
reachable by two routes is visited twice (harmless — the association lookup deduplicates by
application), but a **cycle in the table loops forever**. `subclasses` is generated, so a cycle would
be a bug in `update-mime-database` or in a package's XML rather than something a user writes; it is
still a file on disk that this reads, and a launcher that hangs on a malformed system file is worse
than one that copes. This keeps a visited set, which terminates on any input and removes the
duplicate visits at the same time. Two tests pin it, one for a cycle and one for a self-referential
type.


**`src/root-search/apps` → `compass-core::root_items`** — what an application looks like in the root
list, ported so `compass_ui::root_list` has something real to arrange. 14 tests, 12 controls.

The provider is spelled **`applications`**, not `apps`: it is half of every application's entrypoint
id and so is written into the config file, which makes the spelling a stored format rather than a
label. The entrypoint half is the desktop id with `.desktop` removed, and the C++ uses
`QString::remove`, which takes out **every** occurrence rather than the suffix. For an ordinary id
that is the same thing; with dotted ids it is reachable — a file named `desktop.desktop` in a
directory called `my` has the id `my.desktop.desktop` and loses both — and the port keeps the C++'s
answer, because the result is a stored key.

Three things in the conversion are deliberate and each has a test saying so. The **subtitle is
empty**, because an application's comment is its description in the settings and filling it would
give every row a paragraph. The **unlocalized name has its own title-weight field**, so someone who knows an
application by its English name still finds it on a localised desktop where the title is something
else. And `enabled` starts true: the root item manager's merge is what turns an item off, and an
application is not disabled by being converted.

The pinned upstream v0.29.0 audit corrected the previous keyword-weight handling:
the untranslated name scores at 1.0, not 0.6. A regression test checks equality
with a display-title match and precedence over a keyword match. Restoring the old
weight fails it (60 versus 100). The conversion preserves keywords and omits an
untranslated name identical to the display name, as upstream does. The root-item
suite now has 64 tests; this correction alone does not establish end-to-end search parity.

**Application root scoring is now wired into daemon and UI search.** `AppIndex`
caches root fields and their catalog positions when scanning, and both consumers
use the root manager's scorer. The XDG adapter combines categories then keywords
at keyword weight, as upstream does; generic names and comments are not root
fields. Unresolved TryExec remains a diagnostic rather than hiding a host app.
The daemon retains the existing desktop-file IDs, result limit and frecency keys;
its wire match score still excludes the frecency boost. This does not yet connect
all builtin/extension providers, root configuration or the grouped `root_list`
presentation. Attached UI windows request history-ranked application suggestions
on opening and when clearing the query; returned rows replace the idle greeting.
End-to-end upstream Unicode ordering must still be
proved before search timing is a comparable benchmark.

The daemon now accepts `RecordLaunch` over IPC for indexed application/action
keys. It serializes history updates through its existing store and performs
blocking persistence off the async executor; unknown keys return BadRequest and
store failures return Internal rather than Ack. Protocol v3 distinguishes the
new request from older peers, with the variant appended to preserve existing
discriminants. The existing unreadable-store fallback remains in-memory for that
session.

Attached UI windows now use a socket-free backend interface supplied by `compass`
to query the same daemon ranking and report successful launches. The adapter
bounds each IPC operation, and Iced explicitly uses its Tokio executor. The UI
cancels superseded queries, rejects late generations, and clears stale rows while
waiting; backend errors or catalog mismatches do not silently substitute local
ranking. Launch reports capture the original item key, including desktop-action
panel launches recorded against their root application, and history failures do
not turn an already successful launch into a launch error. A real-daemon test
drives UI tasks through selection, launch and reopening, verifying the persisted
visit changes the new UI's order. Headless tests do not prove an external app
opened. Standalone UI without a daemon still uses local search without persisted
history and retains its idle greeting. Non-application providers, favourites and
grouped root presentation remain unfinished. Widget tests distinguish initial
suggestions from the greeting and search failures from launch failures; the
real-daemon UI test also verifies history ordering when reopening with no query.

Desktop `Type=Link` entries now enter the application index without an Exec,
including the harvested Singular manual fixture. Missing or empty URLs are
reported, and link entries do not expose application-only desktop actions.
Linux dispatches the URL as a single `xdg-open` argument, started on the host inside
Flatpak (the session helper's `HostCommand`) so host file links are resolved on the host.
Invalid schemes, option-like paths and control characters are rejected; Exec is
never substituted for the URL. Index, UI and real daemon tests cover visibility,
and argument tests cover URI dispatch. Opening the target in another application
still needs an on-target desktop check; this is not a completed search-parity or
launch-performance gate.
The [same-corpus audit](benchmarks/2026-09-20-desktop-links/README.md) now returns
all 448 empty-query IDs in upstream order, but the Unicode query still differs.

An integration prerequisite found during the upstream audit is corrected: the
launcher's selected desktop-action row now dispatches its stable action ID to
`AppLauncher::launch_action`, instead of launching the parent entry. Linux resolves
that declared action's own Exec/URI arguments through the existing launch route;
unknown IDs and missing Exec return errors, never a parent-launch fallback.
Backends without action support report that explicitly. UI task tests distinguish
ordinary and action dispatch, and Linux argument tests distinguish the two Execs.
Desktop actions now appear in the owning application's panel instead of root
search results. A UI task test drives query, panel filtering and activation by
stable action ID; a real-daemon IPC test rejects action and description matches
while retaining an unresolved TryExec application. Target-session action launch
verification and the rest of root-provider integration remain required.

**`src/services/root-item-manager` → `compass-core::root_items`** — the *search* is ported in
full: the weighted fields (title and unlocalized title 1.0, subtitle 0.5, alias 1.0, keyword 0.6), the `MIN_QUALITY` gate,
the frecency boost, the empty-query `100 - FRECENCY_WEIGHT + FRECENCY_WEIGHT * frecency` ranking, the
enabled/provider/favourite filters, and the stable sort with its alias-prefix prioritisation. Twelve
tests, twelve controls, each read off `root-item-manager.cpp`. `mergeConfigWithMetadata`,
`registerVisit` and `resetRanking` are ported too — the enabled precedence (the item's own default,
then the user's per-item setting, then a *disabled* provider, which wins), aliases, shortcuts,
favourite positions and fallback flags. `searchGroupedByProvider` is ported
too, with its two rules that differ from the flat search — a provider whose *display name* matches
contributes all of its items, including ones scoring zero, and `providerId` is not applied — for
another nine tests and nine controls.

The daemon now loads the top-level `providers`, `favorites` and `fallbacks`
settings and merges them into application root metadata at startup. Application
aliases and enabled settings affect real IPC queries (and therefore attached UI
search); a disabled provider overrides an enabled entrypoint. Configuration keys
use upstream identities such as `applications:org.example.Editor`, while IPC and
launch-history keys remain `org.example.Editor.desktop`. Unknown provider and
entrypoint fields, including preferences, survive configuration round trips.
Real-daemon tests cover alias lookup and both levels of enabled precedence;
catalog tests cover clearing settings without retaining stale aliases. This is
startup configuration, not live reload or a settings editor. Favourite sections (wired since the gaps pass),
shortcut registration and fallback dispatch remain unwired; parsing their
metadata is not completion of those features. Standalone UI startup now applies
the same root configuration to its local catalog. UI state-machine tests cover
alias queries, disabled applications, provider precedence and clearing settings;
attached searches remain authoritative in the daemon. This does not add standalone
history persistence or change its empty-query greeting.

The manager's state changes are ported too, and this note previously understated that:
`mergeConfigWithMetadata` and `registerVisit` were already done when it was written, and the config
writes behind `setAlias`, `setShortcut`, `setItemEnabled` and `setProviderEnabled` are done now —
seventeen more tests and fourteen controls. A write is a *merge*: setting an alias must not clear a
shortcut set earlier, and the provider and entrypoint entries are created on first write because the
config file holds only what the user changed. Each pairing writes **both** halves, memory and file,
because doing only the first is a change that shows immediately and vanishes on restart.

`setItemEnabled` deliberately writes only the file. The merge applies the provider's setting *after*
the item's, so enabling an item whose provider is off does not make it appear — and only the merge
knows about the provider. There is a test for that interaction end to end rather than for the
absence of a line.

**A declared divergence in `setShortcut`.** The C++ is asymmetric and the asymmetry is a bug: an
empty shortcut *resets* the metadata but still writes `std::string{""}` into the config. On the next
merge that stored empty string is present, so `if (auto shortcut = itemConfig->shortcut)` takes it
and the metadata comes back as `Some("")` rather than `None`. Clearing a shortcut therefore looks as
though it worked until the launcher restarts, and then the item has an empty shortcut instead of
none. This port writes `None`, and two tests pin it — one on the write, one on the round trip
through a merge.

The providers load now (applications, builtins, extensions, scripts, shortcuts, Rhai scripts).
Favourites and the search history landed in the gaps pass (see there). Still C++-only: per-item
keyboard shortcuts and the fallbacks other than Search Files, whose settings are parsed and merged
but not acted on.

**`src/services/tray-host` → `compass-core::tray_host`** — `TrayItem` and `TrayMenuItem` are
ported: the item key is the bus name *and* the object path, because one application can export
several items on one connection; a menu path of `/` means no menu at all; and the icon falls back
through resolved theme path, raw pixmap, theme name, then `application-x-executable`. Each
attention field falls back **on its own**, so an application that sets an attention icon name but
no attention pixmap still shows its ordinary pixmap rather than nothing. `resolve_icon` keeps
`findIconInThemePath`'s first line — an empty name or an empty theme path returns before any
directory is walked — and `best_icon` returns an SVG immediately and otherwise takes a *strictly*
larger file, so equal-sized candidates take the first and the answer does not depend on directory
read order. `plain_label` strips a single `_` mnemonic and folds `__` to one literal underscore,
iterating by `char` so a label with accented text is not cut mid-codepoint. 
The `com.canonical.dbusmenu` layout is read here too (`menu_item_from_layout`, 22 tests and 20
controls): the transport is the bus's, but turning a property bag into a menu entry is not, and
every default in it is the protocol's rather than a guess.

Three of those defaults carry weight. `enabled` and `visible` default to **true**, because an
application that sends neither wants an ordinary entry and defaulting either way round renders a
menu of grey nothing. `toggle-state` is read only when the key is **present**, because its default
is `-1` — indeterminate — and that is a different state from `0`, which is off: a checkbox nobody has
answered is not an unchecked one. And an unknown `toggle-type` falls back to none, so a type the
protocol gains later draws as a plain entry rather than an empty checkbox.

Two more are about a bus that is loosely typed and carries whatever an application sends. A property
of the wrong shape takes its default rather than refusing the menu, and an **empty** `icon-data`
payload is no icon rather than an empty one — which would draw as a blank space where the
application meant nothing at all. The menu itself is the root node's *children*: returning the root
would put an unnamed entry above every menu.

The DBus plumbing (the StatusNotifierWatcher registration, the menu layout and the property-change
signals) landed in "The gaps pass, icons and tray", as `compass::tray_host` over the `system-tray`
crate.

**`vendor/sqlcipher` + `vendor/fuzzy-trigram` → `compass-sqlcipher-sys`** — the storage engine
itself (ADR-0014). SQLCipher is `rusqlite`'s `bundled-sqlcipher` build (`libsqlite3-sys` 0.38,
SQLCipher 4.14.0 on SQLite 3.51.3, OpenSSL crypto on Linux as the C++ build uses) rather than the
vendored 4.16.0, which only the C++ engine still compiles; both are SQLCipher 4 with its default
cipher settings, so each reads the other's files. `fuzzy_trigram` is still the vendored C, compiled
against `libsqlite3-sys`'s own headers. `compass_sqlcipher_sys::open` does what
`ClipboardDatabase`'s constructor does and in the same order — open, key with raw bytes in
SQLCipher's `x'...'` form, register `fuzzy_trigram`, apply the four pragmas — because that order is
load-bearing, and hands back a `rusqlite::Connection`. Tested against real encrypted files rather
than SQL strings: the file has no `SQLite format 3` magic and does not contain its own payload in
the clear, a wrong key is refused by `open`, the FTS table the clipboard schema declares can be
created and queried, and a second connection to an existing encrypted file still has the
tokenizer. `Connection::changes` is available now but deliberately unused — see
`tryBubbleUpSelection` below.

**`src/services/clipboard` → `compass-clipboard`** — **`clipboard-db.cpp` (478 lines) is ported in
full.** Every function `clipboard-db.hpp` declares has a Rust counterpart:

| C++ | Rust |
|---|---|
| `searchTerms` + the trigram predicate | `search::plan` |
| `runMigrations` | `schema::run` |
| `query` | `store::query` |
| `insertSelection`, `insertOffer`, `indexSelectionContent` | `write::insert_selection`, `insert_offer`, `index_content` |
| `removeSelection`, `removeAll`, `evictOlderThan` | `write::remove_selection`, `remove_all`, `evict_older_than` |
| `tryBubbleUpSelection` | `write::bubble_up` |
| `setPinned`, `setKeywords`, `retrieveKeywords` | `write::set_pinned`, `set_keywords`, `keywords_of` |
| `oldestEvictableTimestamp` | `write::oldest_evictable` |
| `findSelection`, `findPreferredOffer` | `write::find_selection`, `find_preferred_offer` |

It runs on `compass-sqlcipher-sys`: SQLCipher 4 through `rusqlite`, and `vendor/fuzzy-trigram`, the
same tokenizer C the C++ engine links ([ADR-0014](./adr/0014-clipboard-storage-is-sqlcipher-plus-a-vendored-tokenizer.md)),
so both engines read and write the same encrypted files with the same tokenizer.

The layer above the database is the engine's now (`compass::clipboard_service`): the selection is
watched through the Shell extension on GNOME and over data-control on wlroots, and payloads are
stored on disk encrypted under Compass's own keyring key (`a_recorded_copy_is_listed_and_found`, the
encrypted-at-rest assertions beside it). Eviction by age and its timer, the monitoring switch and
the extension's preferences landed in the gaps pass (see there). `store-all-offerings` is not
ported because there is nothing to port: the C++ reads it into `m_recordAllOffers` and never reads
that member again.

Four C++ bugs are fixed rather than reproduced, each pinned by a control that fails when the
original shape is put back: the eviction blob leak, `tryBubbleUpSelection` answering from a
connection-wide counter, the migration checksum that was written and never compared, and
`runMigrations` swallowing its own failures. All four are in the divergences table below.

**Why `parity test ✓` is now ✅.** `vicinae::fuzzy` and `vicinae::crypto` each compile standalone,
which is what lets CI diff the real C++ implementation against ours. `clipboard-db.cpp` does not —
it pulls in Qt, `db::Database` and `MigrationManager` — so there is no C++ binary to diff against,
and until the ledger truth pass that kept this cell amber. Under ADR-0017 an absolute test that
fails on a regression is what the column asks for, and that is what exists: every assertion is
shown to fail against a deliberately wrong port, and the tests run against real SQLCipher files
rather than SQL strings (`compass-clipboard`'s `write_path`, `query_reads_history`,
`migrations_apply` and `ingest` suites).

The gap worth naming is `QString` iterating UTF-16 code units, so a non-BMP character counts as
*two* word characters: `"a😀"` is a trigram run to the C++ engine and would not be to a port walking
Rust `char`s. `has_trigram_run` walks `encode_utf16` for that reason, and
`a_non_bmp_character_is_two_word_characters_not_one` holds the scalar-walking version alongside it
so the case is shown to discriminate. The same trap recurs in `index_content`, where
`QString::left(65536)` can split a surrogate pair.

## Out of scope for the port

Rows marked **out of scope** are not "not yet" — the Rust engine will never implement them.

- **Browser tab search and switching** (`src/browser-extension`, `src/services/browser-extension`,
  `src/builtins/browser`) becomes an extension rather than a builtin, per
  [ADR-0008](./adr/0008-browser-control-is-an-extension.md). The C++ implementation keeps working
  for C++-engine users and is not deleted by the port. If no extension exists by Phase 7, this is a
  feature regression at cutover and must be in the release notes.

  Note for whoever builds it: native messaging does not simply move into a TypeScript extension. A
  browser spawns a native host **binary by absolute path** from a manifest in a fixed location, and
  a Raycast-compatible extension is a TS bundle spawned by the launcher with no such path — inside a
  Flatpak on our first target. ADR-0008 sets out the three ways round that and deliberately does not
  pick one.

## Deprioritised

**AppImage — disabled, not deleted.** Not a distribution channel we plan to use, so the AppImage
build is not being chased. Its jobs could only ever fail: `ghcr.io/tuna-os/compass/build-env:appimage-*`
was never published after the move off Depot, and nobody is going to publish it.

Rather than accept a permanently red check — which is how a team learns to ignore CI — the automatic
triggers are gone:

| Where | State |
|---|---|
| `build-appimage.yaml` | `workflow_dispatch` only; `push`/`pull_request` removed |
| `build-appimage-image.yaml` | `workflow_dispatch` only; weekly `schedule` and `push` removed |
| `build-appimage` job in `release.yml` | `if: false`, and dropped from the `needs:` of `publish-npm` and `publish-release` |
| `scripts/runners/appimage/` | untouched |

That last row mattered more than the others. Both publish jobs listed `build-appimage` in `needs:`,
so its failure skipped them — **releases were already impossible**, not merely missing an AppImage.
Ungating them is a fix, not a regression; the cost is that a release no longer ships a `.AppImage`,
which is the intent rather than a side effect.

To restore: publish the build-env image by dispatching `build-appimage-image.yaml` once (it compiles
GCC 15.2 and Qt 6.10 from source, so expect hours), then restore the triggers and the `needs:`
entries. Restoring the triggers alone just brings the red back.

Bluefin is the first target and is Flatpak-only, so the Flatpak manifest in `packaging/flatpak/` is
the channel that matters.

## Declared divergences

Behaviour that intentionally differs from the C++ engine. Each is pinned by a test that fails if
the behaviour changes, so a future fix is loud rather than silent.

### `compass-core::calculator` — fend instead of Numen, and a digit required

The C++ calculator is Numen, an in-tree library; porting it was not the job, so root search uses
[`fend-core`](https://crates.io/crates/fend-core), an existing Rust calculator with no dependencies
of its own. The two engines therefore format some answers differently (fend writes `approx.` before
an inexact result). Currency conversion is fend's own, fed the ECB's daily rates since "The gaps
pass, currency". When to try is the C++ rule (a leading `=` always; otherwise at
least three characters and nothing else matched), plus one: without the `=`, the query must contain
a digit, because fend reads almost any word as something (`a` is one ampere).

### `compass-db` — typo correction without `spellfix1`, in its own file

**Kept on purpose (ADR-0017).** The C++ file indexer asks SQLite's `spellfix1` extension for
corrections; Compass keeps a plain `vocabulary(word, rank)` table and suggests in Rust
(`compass_db::vocabulary::suggest`): optimal-string-alignment distance from `strsim`, reported at
100 per edit so the ported correction policy's thresholds read unchanged. Spellfix's phonetic
candidate hash and per-character-class substitution costs are not reproduced, so individual
suggestion lists differ; the ported quality suite (`compass-db/tests/query_quality.rs`) passes
23/23 either way, and its four correction-dependent cases fail when suggestions are stubbed out.

Because the schema differs (v2), Compass's index lives in `compass-file-index.db` rather than
`file-indexer.db`, in the same directory. It is a cache: the first scan fills it, nothing migrates.

### `compass-core::root_items` — one slip no longer makes an app vanish (#204)

**An improvement, kept on purpose (ADR-0017).** Both engines match root items as an ordered
subsequence, so a transposed, doubled or substituted keystroke (`alacrtity`, `alacrittyy`,
`blemder`) returned nothing from either. Compass now falls back to a one-edit
optimal-string-alignment distance (`compass_search::typo_distance`, via `strsim`) for queries of five
or more characters, over title, untranslated title and alias words, for items the matcher did not
reach. Those hits are appended after every real match with a score of `EPSILON × (1 + frecency)`,
so they add an answer but never displace one.

Where C++ returns `[]` for such a query, Rust now returns the intended app. Suite 0's top-result
gate skips queries where either side is empty, and typo hits only follow real matches, so the gate
is unaffected. Pinned by `search_quality.rs::a_transposed_or_doubled_character_still_finds_it`
(real corpus) and four `root_items.rs` tests, including that a typo hit never outranks a real match
however often it has been opened.

### Suite 0's ranked-output path — two divergences that are **scope**, not behaviour

Not bugs on either side. These are the two places where `vicinae query --json` (C++, added for
§8.1a rung 2) and the Rust engine's `query` are answering *different questions*, and Suite 0 would
otherwise report both as regressions on every query with a match. §8.1 requires a divergence to
cite a rationale and be declared rather than discovered; this is that citation.

| # | The two engines | Why it is not reconcilable | Pinned by |
|---|---|---|---|
| 1 | **The score is on two scales.** Rust puts `QueryHit.score` on the wire — the 0..=100 match score, with the frecency boost *deliberately* excluded, so hits are legitimately not in descending score order. C++ `rootQuery` emits what `RootItemManager::search` **orders by**, which `SearchableRootItem::fuzzyScore` returns as `score.score + FRECENCY_WEIGHT * frecency()`. | Reporting the other engine's number means recomputing it at a second site: on the C++ side that is `fzf::threadLocalMatcher().score_query` over the field weights (title 1.0, subtitle 0.5, alias 1.0, keywords 0.6), duplicated away from the one place that owns them. A second copy of a weighting is a divergence generator, not a fix. **The gate Phase 1 names is ranking, not scoring** — §8.1a already measures scores differing on 20.8% of queries while the ranking absorbs it. | `same_ranking_different_scores_is_a_known_divergence` — and, so the tolerance cannot quietly swallow a real regression, the same test asserts that a reordering sharing those scores is still a `Regression` |
| 2 | **The two rank different sets.** The Rust root ranks applications and its own builtin commands (`commands:*`, from `compass_core::commands`). C++ `RootItemManager` ranks applications plus its commands, extension entrypoints and fallbacks, under different ids. | Both are correct for their engine. Each CLI takes `--provider` (C++ `rootQuery`'s `providerId`; Rust filters its ranked list by id prefix), and the harness passes the same provider to both, so like is compared with like. Deliberately **not** defaulted — a default would silently decide a parity question that belongs to whoever runs the comparison. | `each_engine_is_invoked_the_way_its_own_cli_parses` pins the argv that carries it |

A third difference is mechanical rather than semantic and is recorded here because it looks like the
others: the C++ engine takes **no `--socket` flag**. `vicinae::serverSocketName()` is
`runtimeDir() / "vicinae.sock"` and `runtimeDir()` reads `XDG_RUNTIME_DIR`, so Suite 0 isolates the
C++ engine with a private runtime directory instead. Adding a flag to a tree being deleted was the
obvious move and turned out to be unnecessary.

### `compass-clipboard` — two C++ behaviours deliberately **not** reproduced

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| -2 | `evictOlderThan` computes its cutoff with `unixepoch()` in **both** the `SELECT` that collects the offer ids to unlink from disk and the `DELETE` that removes the rows. Those are separate statements with separate readings of the clock (measured: inside `BEGIN`, `unixepoch('subsec')` advanced after 434 consecutive statements), so the `DELETE` set is a superset and anything crossing the threshold in between is deleted but never reported — its payload stays on disk forever. | Compute the cutoff once and bind it to both statements, which makes the two sets identical by construction. | `eviction_returns_every_offer_it_deletes` (fails when the second reading is reintroduced) |
| -1 | `tryBubbleUpSelection` runs its `UPDATE`, discards whether it succeeded, and answers from `m_db.changes()` — a connection-wide counter holding the most recent *successful* statement's count. A failed or no-op update can therefore report success, and its caller (`clipboard-service.cpp:508`) then skips `insertSelection`, so the copied content never reaches the history. | `RETURNING id`: did *this* statement touch a row. `bubble_up` deliberately does not call `Connection::changes`, the same connection-wide counter. | `bubbling_up_something_absent_reports_false_even_after_a_successful_write` |
| 0 | `query` divides by `limit` to compute `totalPages` (`ceil(totalCount / limit)`), so a zero `limit` is a division by zero whose result is cast to `int`. It also interpolates `limit` and `offset` into the SQL text with `.arg()` rather than binding them. | Refuse a non-positive `limit`; bind both. `current_page`'s ceiling rounding *is* reproduced, oddity included — it is a display value the C++ UI already agrees with. | `a_zero_limit_is_refused_rather_than_dividing_by_it` |
| 1 | `MigrationManager::runMigrations` catches every exception, logs it, rolls back and returns `void`; `ClipboardDatabase::runMigrations` returns `void` too. A failed migration is silent, and the next thing the user sees is every query failing against a schema that was never created. | `schema::run` returns a `Result`. | `an_edited_migration_is_refused`, `a_database_from_a_newer_build_is_refused` |
| 2 | The `checksum` column exists to detect a migration edited after it was applied. `insertMigration` writes it and `loadDatabaseMigrations` reads it back into a struct field — and nothing ever compares the two. It is a stored value with no reader, so the detection it exists for never happens. | Compare it, and refuse on a mismatch. The expected hashes are also pinned in `schema.rs`'s tests, so editing a migration fails at development time rather than on a user's machine. | `an_edited_migration_is_refused`, `the_embedded_content_hashes_to_what_the_cpp_engine_recorded` |
| 3 | `updated_at` is whole seconds (`QDateTime::currentSecsSinceEpoch()`) and the list orders by it alone, so two copies within one second tie and SQLite picks their order: copy A, then B, and A can be listed on top. Re-copying an older entry in the same second as a new one can leave it below. | Compass owns its store (ADR-0017), so stamps are milliseconds, and every copy is stamped `max(now, newest + 1)`, strictly after everything before it; the list breaks any remaining tie by insertion order. A Vicinae importer multiplies its seconds by 1000. | `copies_made_in_quick_succession_list_newest_first`, `a_recopied_entry_goes_on_top_even_when_the_clock_is_behind` (both fail when the fix is reverted) |

### `compass-xdg` — six C++ bugs deliberately **not** reproduced

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | `parseRawLocale` loops `while (!isPeek(']'))`, and `isPeek` is false at EOF because `peek()` returns `0`. A truncated `Name[fr` at end of file appends NUL forever: an unbounded loop with unbounded allocation. | Terminate on `]`, newline, or EOF. | `a_truncated_locale_suffix_does_not_swallow_the_next_line` |
| 2 | A key with no separator consumes the following character and, on mismatch, skips to the *next* newline — so when the consumed character *was* the newline, the next line is silently swallowed. | Skip only when the mismatched character is not the line terminator. | `a_key_with_no_separator_is_skipped` |
| 3 | Field codes push directly into `args` while the in-progress word sits in `part`, so `Exec=prog --name=%c` yields `["prog", "MyFile", "--name="]` — the pending word lands *after* the expansion. `--file=%f` forms are common in the wild. | Substitute single-valued codes (`%f %u %c %k`) into the current word; flush the word before multi-valued ones (`%F %U %i`). Every C++ test result is unchanged, since they all use standalone codes. | `a_field_code_may_be_glued_to_the_rest_of_a_word` |
| 4 | `asStringList` unescapes each element at a separator but pushes the trailing residue raw, so `Keywords=a;b\sc` yields a literal `b\sc`. | Unescape consistently. | `escapes_apply_to_an_unterminated_last_element` |
| 5 | An unknown `Type` silently becomes `Application` (the if/else chain has no `else`). | `EntryType::Other(String)`; `is_application()` is false for `Type=ServiceType`. A *missing* `Type` still defaults to Application, matching C++. | ported `Type` cases |
| 6 | `genericName()`, `version()` and `unlocalizedName()` return `std::optional` built from a plain `std::string`, so they are never `nullopt` — an absent key reads as `Some("")`. | Return `None`. | `unlocalized_name_is_absent_when_only_a_localized_name_exists` |

Matched deliberately, for the record: field codes are not expanded inside quotes; unknown and
deprecated field codes expand to nothing; a redeclared group replaces rather than merges; localized
score ties resolve to the last declaration.

### `compass-core::default_app` — one picker sorts stably and the other does not

`set-default-browser-view-host.hpp` sorts its candidates with `std::ranges::stable_sort`;
`set-default-terminal-view-host.hpp`, which is otherwise a copy of it, uses `std::ranges::sort`.
Both comparators are `isDefault(a) > isDefault(b)` and say nothing about two non-defaults, so the
terminal list below the current default is in an unspecified order — two runs of the same binary may
disagree, and nothing in the view depends on it being one way or the other.

Compass sorts both stably, so the rest keep the order the application database gave them.
`the_rest_keep_the_order_the_database_gave_them` pins it for the browser picker, where the C++ makes
the same promise; the terminal picker shares the implementation, so it inherits a guarantee the C++
does not make rather than a different behaviour.

### `compass-core::root_items` — an unfavourited item that stays unfavourited

`mergeConfigWithMetadata` only *assigns* `favoriteIdx` when the entrypoint id is in `cfg.favorites`:

```cpp
if (auto it = std::ranges::find(cfg.favorites, std::string{entrypointId}); it != cfg.favorites.end()) {
  meta.favoriteIdx = std::distance(cfg.favorites.begin(), it);
}
```

`m_metadata` is a map that outlives the merge, so removing an item from favourites leaves the old
index in place. Every search that passes `includeFavorites = false` — the root list, which renders
favourites separately — keeps dropping that item until the launcher restarts. `meta.fallback` two
lines below is assigned unconditionally and does not have the bug.

Compass clears the index when the id is absent. `unfavouriting_clears_the_index_rather_than_leaving_it_behind`
pins the fixed behaviour; `a_fallback_that_is_removed_stops_being_one` pins the neighbouring line
that was already right, so a future "fix" that changed the wrong one would be caught.

### `compass-worker-host::ui_shell_service` — a method that does nothing, faithfully

`ExtUIService::updateToast` is `{ return Void::ok(); }`. The body is empty: an extension that calls
`toast.title = "..."` after showing a toast gets a resolved promise and no change on screen. The
Rust host does the same, and `updating_a_toast_does_nothing_at_all_because_the_cpp_does_nothing`
pins it — with a control that fires if it ever starts working.

Implementing it would be the more useful behaviour and the wrong port: the extension cannot tell
from the reply which host it is talking to, so a Compass that updated the toast would show text a
Vicinae user never sees, and an extension author would tune their toasts against the wrong one. When
the C++ grows a body, this test is the one that should fail.

### `compass-core::slug` — the `slug` crate, which transliterates

`slugify` is the [`slug`](https://crates.io/crates/slug) crate rather than a port of the Qt
function (AGENTS.md: crates over hand-rolled code). The pinned cases agree — accents fold
(`Café au Lait` → `cafe-au-lait`), whitespace and underscore runs make one `-`, punctuation goes,
ends are trimmed — with one difference: the crate **transliterates** non-Latin text
(`日本語 ツール` → `ri-ben-yu-turu`) where the C++ strips it and leaves an empty string, which as an
extension's directory name was a bug. The C++'s configurable separator is gone; nothing used it.

### `compass-core::boilerplate` — two commands with the same slug, and the first one wins

`QFile::copy` refuses to overwrite an existing destination and reports the failure through a return
value the C++ does not check. So an extension generated with two commands whose titles slugify alike
— "Show Things" and "show things" — gets two entries in its manifest and one source file, holding
the *first* command's template. The second command points at a file that is not the template it
asked for.

The port reproduces this, and `two_commands_that_slugify_alike_do_not_clobber_each_other` pins it,
because the alternative readings are both worse: overwriting would make the *second* command win and
leave the first pointing at the wrong template instead, and rejecting the config outright would fail
a generation the C++ completes. It is a bug worth fixing upstream, not worth diverging on here — the
test is named for what it protects, and is the one that should fail when the C++ starts checking
that return value.

### `src/server/src/ui/image` — its wire format and its contrast maths

**Since the ledger truth pass (2026-09-25) the row is `Rust ✓` 🟡 and `parity test ✓` ✅**: themed
application icons (`compass_ui::icons`) and remote images (`compass_ui::remote_image`) are drawn
now, and what is still missing is listed there. The section below is the record of the two ports it
describes.

`compass-core::image_url` is a complete port of `url.cpp` — the `icon://` scheme every icon in the
system is referred to by — and `compass-core::contrast` of `contrast-helper.hpp`, which picks a
colour that can be read against another one. Between them, 53 tests and 43 controls. The row stays
❌, for the same reason
`src/file-indexer` does: the directory is 2,154 lines across fourteen files, and what is ported is
one of them. The renderer, the streaming decoder, the painter and the platform icon loaders are the
rest, and they are the part that needs a UI layer.

The contrast maths splits into a standard and a judgement, and they are tested differently on
purpose. `getRelativeLuminance` and `getContrastRatio` are WCAG 2.x, defined to the digit, so they
are pinned against the standard's own values — black on white is exactly 21:1, and a pure green is
0.7152 where a pure blue is 0.0722. `getTonalContrastColor` is not a standard: it is this project's
answer to "given an album cover, what colour do I write on it", and what is pinned there is the
properties it must hold — it moves away from the background, it keeps the background's hue, and it
gives up after thirty steps rather than looping, because on a mid grey no lightness reaches 4.5 and
a search without the bound would never end.

One thing this port does *not* verify is that its HSL conversion agrees with Qt's to the last bit.
The tests hold the properties rather than exact RGB triples for that reason; a caller needing the
same pixel as the C++ would need a cross-check that does not exist yet.

It was worth porting ahead of them because an `ImageURL` is a *string*, not a widget. That string
crosses every boundary in the system: it is what a root-search row stores, what an extension gets
handed back, what an alert carries. Two details in it are load-bearing for data already written
down, and neither is obvious from reading the code once:

`nameForType` returns the *first* table entry for a type, so `Builtin` prints as `omnicast` rather
than `builtin`, and `Https` prints as `http`. Both spellings parse, so the tables are asymmetric on
purpose and reordering them would silently change every URL a new build writes.

And `resolveThemedLocalPath` inserts `@dark` before the first dot *after the last separator*, not
before the last dot: `a.tar.gz` becomes `a@dark.tar.gz`, and `~/.local/share/logo` becomes
`logo@dark` rather than being confused by the dot in the directory above it.


### The first view-layer work: the root list's sections and selection

(Written before the backends landed; the ledger truth pass above has the current state, and
`scripts/ci/parity-score.py` the current breakdown.) Every row still 🟡 is 🟡 for the same reason —
the model is ported and tested, and the *backend* is not. Counted across the notes below, what is left is views (4), providers (3), QML (2), MPRIS and
HTTP. That is the engine rather than more transcription, and it is where the remaining Phase 5
percentage lives.

**What the VM tier proves about this work, and what it does not.** Run 195 is green on `443efa6`:
stock Bluefin boots under QEMU, the Flatpak installs, the engine starts *without putting anything on
screen*, it finds applications (#95), a launcher window appears, a typed query reaches **our** field
(#91), and the window hides and comes back (ADR-0015) — each against a control frame, with the
changed region asserted to lie inside a box so that "something else moved" fails too.

**The action panel has now been seen in a real session.** `scripts/vmtest/launcher.sh` presses
Ctrl+B after the hide/summon pair, and run 199 on `3da5283` is the first measurement:

```
2697 of 716800 pixels differ (0.38%)  box x 360..466 (107w) y 315..447 (133h)
ok: 0.38% >= 0.10%
ok: changed region is inside 300,140..980,800
```

Two things follow, and only the first is a framediff's to say. The chord **reached the
application** — which was genuinely open, because the launcher had just been hidden and summoned
and nothing established that our window still held keyboard focus afterwards. And reading
`launcher-06-panel.png` against `launcher-05-summoned.png` in that box, what drew is the panel
itself: the root list's `> Firewall / Files / New Window` is replaced by an `Actions` header, `>
Open`, a `---` divider, a `Copy` section header, and `Copy name` / `Copy path` beneath it.

That is the flattened structure `compass_ui::action_panel`'s tests pin, confirmed on screen: a
header, selectable rows, a divider, a section header and its rows — **and the caret on `Open`,
the first *selectable* row**, not on the `Actions` header above it. The unit tests assert that the
selection skips headers and dividers; this is the same claim, drawn by a real compositor.

**It is now a gate.** Run 200 on `317940f` printed the same figure from a separate VM boot —
2,697 pixels, 0.38%, the same box — byte-identical rather than merely close, which is what makes a
threshold defensible after two runs instead of a dozen: there is no spread to fit to. The floor is
0.1%, about a quarter of what was measured and deliberately *not* fitted to 0.38%, because a panel
with fewer actions must still pass; what must fail is the chord never arriving, which reads 0.00%.
The containment box carries the half that does not move with content, and also asserts that nothing
outside our window changed. The root list's sections and the
selection moving through *them* still have no VM coverage, only the tests in this crate. The tier answers "does the launcher paint and
receive keystrokes in a real GNOME session", which is the question #91 came from; it does not yet
answer "is what it paints the right thing". Saying otherwise — as an earlier version of this PR's
description did — would claim verification that no assertion performs.

`compass_ui::root_list` is the first piece of it. The launcher's main list is not one list: it is
favourites, then results, then — when nothing matched — the fallbacks, each under its own heading.
The arrangement and the selection moving *through* it are kept out of the Iced `view` function, so
they can be tested in a container with no display server; only the drawing needs a compositor. 31
tests, 29 controls.

The selection arithmetic is deliberately **flat**, and the sections are invisible to it. A heading
is not a position, so nothing ever lands on one and nothing is skipped crossing a boundary — which
is the bug this shape prevents, and one that is invisible until someone tries a list with more than
one section. Two tests walk the whole list rather than taking one step, because a one-step test
passes against an off-by-one that makes the last row unreachable.

Favourites lead the empty query and vanish once something is typed: a favourite that does not match
is not an answer, and keeping it above the results would push the thing asked for down the page. One
that *does* match appears among the results instead — not hidden, just without its special position.
The search excludes favourites for the empty query precisely so the same row is not drawn twice, and
a test checks for the duplicate rather than trusting the flag.

Fallbacks appear only when nothing matched, which is what makes them fallbacks rather than a section
always on screen. An empty section is never drawn, because a heading with nothing under it is a
heading that lies.


`compass_ui::action_panel` is the second piece, and the one every command needs: the panel its
actions are shown in. Ported from `action-panel-model.cpp`. 41 tests, 29 controls.

A panel is sections flattened into rows, where headings and dividers are rows that cannot be
selected — so almost all of it is the difference between a row and a selectable row. Three
flattening rules, each a thing that looks wrong on screen if it is missed: a section filtered down to
nothing contributes **no heading and no divider** (a heading over an empty space being the most
visible way to get it wrong); the divider is emitted *before* the next section's heading rather than
after the previous section's last action, which is what keeps "between" true when a middle section
drops out under the filter; and an unnamed section gets its divider but no heading, because the
separation is what the section is for even when it has nothing to say about itself.

Moving by *section* has the asymmetry the C++ has, and it is a good one: down goes to the first
action of the next section, while up goes to the top of the **current** section when the selection
is not already there. One press takes you to the top of what you are in, a second to the section
above, and neither requires counting rows. Moving up onto a section's first row scrolls to its
heading, without which the heading sits just above the viewport and the first row of a section looks
like the middle of the one before.

A shortcut runs the first bound action in panel order. Two actions sharing one is a mistake nothing
reports, so the order decides it rather than nothing happening — and a shortcut on an action the
filter has removed is not reachable, because running something not on screen is worse than the key
doing nothing.

Two guards were written and then deleted because no mutation could reach them: an empty-filter early
return (`all` over an empty iterator is already true) and an empty-panel early return (both search
loops are bounded by the row count). Three controls found tests that proved less than they looked
like they did — the section-down test started from the last row of its section, where the next row is
already the next section and the rule under test never fires.


The panel is now *in* the launcher rather than beside it: Ctrl+B opens it over the selected row, it
takes the arrow keys and the vim chords while it is open, Escape closes it rather than the window,
and running an action closes it either way.

**Ctrl+B and not Ctrl+K, and the reason is in the C++.** `keybind-manager.cpp` binds the panel to
Ctrl+K on macOS and Ctrl+B everywhere else. I had written Ctrl+K before reading that, which would
have taken the vim chord for "move up" from every Linux user of the default scheme — there is a test
asserting that chord, and it would have caught the clash a moment later. Reading the source first
was cheaper.

Three test premises were wrong and the suite said so, and one of them is the shape of this component
in miniature: the panel's second *selectable* row is index 3, not 1, because index 1 is a divider
and index 2 a heading. Expecting 1 was expecting the selection to land on a divider — which is the
exact bug the flattening is built to prevent, written into a test of it.

**One assertion is not control-backed and says so in place.** "Escape closes the panel and *not* the
window" — the first half fires under a mutation, the second cannot: `conceal` closes the window
through a Task and clears the field only when `Message::Closed` returns, and with no engine link
`on_dismiss` exits instead. Proving it needs an app built around a live `EngineLink`, which this
crate has no harness for. Recorded rather than left looking covered.

### `src/services/window-manager` — the dispatch layer, and then the providers

**Superseded by the ledger truth pass (2026-09-25):** the providers this section says are missing
have since landed — GNOME through the Shell extension (`compass-shell`), the wlroots
foreign-toplevel list (`compass_wayland::toplevel`), and Hyprland and niri over their own IPC
(`compass_platform_linux::compositor`) — so the row is `Rust ✓` 🟡 and `parity test ✓` ✅.
Still C++-only: GNOME's workspace list (the Shell extension's contract has no `ListWorkspaces`).
The KDE provider landed in "The gaps pass, KDE (2026-09-25)"; the X11 provider is n/a, Compass
being Wayland only ("Product decisions").
The section below is kept as the record of the dispatch port.

`compass-core::window_manager` is a complete port of `window-manager.cpp` — which backend gets
picked, and the focus bookkeeping that lets the launcher act on the window the user was in *before*
they opened it. 39 tests and 32 controls.

The row covers 5,380 lines across thirteen files, and twelve of them are per-compositor providers:
Hyprland, GNOME, KDE, X11, Niri, generic Wayland, Windows virtual desktops, and the event listeners
under each. One file of thirteen is not the row, on the same rule that keeps `src/file-indexer` ❌.
**So this moves the Phase 3 gate by nothing**, and that is the right answer rather than a
disappointing one.

What it is worth is the part that is not compositor-specific and therefore not testable by running
one. The provider order *is* the mechanism: the first candidate that says it can run wins, and the
generic Wayland provider is offered **last** because it is good enough for most standalone
compositors and would otherwise claim Hyprland and Niri too — and then neither would get its own
workspace support. A test pins that ordering, with two activatable candidates rather than one,
because with one the first and the last are the same and the test would pass against either rule.

The focus memory is the other half. The launcher takes keyboard focus when it opens, so "the focused
window" is almost always its own, and every action that means "do this to what I was looking at"
depends on remembering. Three rules carry it, and each is pinned:

- A compositor that reports the *frontmost* window is trusted outright and no memory is kept at all,
  because it knows what is in front even while the launcher holds focus.
- The launcher's own window is never remembered and **does not clear** the memory. That is the whole
  point: opening the launcher must not lose what was underneath it.
- Nothing focused clears the memory only when the launcher does not have focus either — otherwise
  the memory is still the answer.

`isOnActiveWorkspace` answers **yes** at every unknown: no workspaces, no workspace on the window, an
empty workspace id, no active workspace. That is deliberate rather than lax. Callers use it to decide
whether to act on a window at all, so on a compositor reporting partial data, saying no would refuse
every action; saying yes only risks acting on a window the user cannot see.

Two C++ looseness are reproduced rather than tightened. An application's windows are matched by class
*or* by the window title equalling the application's display name case-insensitively — loose enough
that a document window titled after its file will not match, but it is what finds a window that
carries no usable class. And the remembered window is checked by **id** on every refresh, which is
what catches a window the compositor destroyed and replaced rather than moved.

### `src/file-indexer` — ported in parts, and then whole

**Superseded by the ledger truth pass (2026-09-25): the row is green.** What this section calls the
missing four fifths has landed: the SQLite schema and writer (`compass_db::db_writer`,
`sqlite_writer`), the query engine and its pool (`query_engine`, `query_reader`, `query_pool`), the
scanners and dispatcher (`indexer_scanner`, `incremental_scanner`, `scan_dispatcher`), the indexer
itself (`compass_db::file_indexer`), the watcher (`compass::indexer_watch` over
`compass_platform_linux::dir_watcher`), the JSON-RPC service (`compass::indexer_service`) and the
process (`compass-file-indexer`). `query-quality.cpp` is `compass-db/tests/query_quality.rs`
(23/23), `main.cpp`'s cases are spread over `compass-core`'s `vocabulary`, `query_policy`,
`io_pacer` and `scan_roots` suites, and the whole path is driven end to end by
`search_files_indexes_the_home_directory_and_finds_a_file_by_a_misspelled_query`. The one
behavioural difference, typo correction without `spellfix1`, is declared under `compass-db` below.
The section is kept as the record of how the pieces were ported.

`compass-core::entry_filter` is a complete port of `entry-filter.cpp` — the rules deciding which
directory entries the indexer walks into — `compass-core::file_walk` of `filesystem-walker.cpp`,
which decides which of them are *reached* and in what order,
`compass-core::incremental_scan` of `incremental-scanner.cpp`, which decides what a re-scan reads
at all, `compass-core::scan_dispatch` of the decisions in `scan-dispatcher.cpp` — when a change
becomes a scan and which scans run — `compass-core::scan_roots` of the path arithmetic in
`util.hpp`, `compass-core::index_reconcile` of what a settings change and a startup do,
`compass-core::watch_policy` of which directories earn an inotify watch, `compass-core::query_policy` of
`file-indexer-query-policy.cpp`, which decides what a typed query asks the index,
`compass-core::vocabulary` of `vocabulary.hpp`, which decides what words a file is findable by at
all, and `compass-core::query_ranking` of the scoring half of `file-indexer-query-engine.cpp`, which
decides what order the answers come back in. Between them, 307 tests and 215 controls. The row stays
❌ anyway.

It covers 5,646 lines across fifteen files: the SQLite schema and its writer, the query engine and
its policy, the incremental scanner, the scan dispatcher, the filesystem walker and the watchers.
Ten files of those fifteen are not the row — about 2,265 lines of the 5,646, two fifths — and marking it
🟡 would put a colour on this ledger that means "a model landed without its backend" when what
actually happened is "a fifth of the row landed". The percentage in PLAN.md is only worth anything
if a row's colour means one thing. **So this work moves the Phase 5 figure by nothing, and that is
the right answer rather than a disappointing one.**

**The walk** (`filesystem-walker.cpp`) is ported with the tree supplied by the caller, because every
rule in it is about which entries are reached rather than about how to read a directory. 16 tests,
12 controls, all of which fired.

Four of its rules are load-bearing and none of them is obvious:

- It is a **stack, not a queue**. The last directory listed is the first entered, which decides
  which half of a large tree is indexed first when a scan is interrupted — and scans are interrupted
  routinely.
- A `CACHEDIR.TAG` abandons its directory **whole**, including the entries already listed *before*
  the tag turned up: the C++ breaks out of the listing loop and drops the vector it was filling. So
  what a cache directory contributes does not depend on where in the filesystem's listing order the
  tag happens to sit. Two tests cover it, one with the tag first and one with it in the middle.
- `depth <= maxDepth` bounds what is **entered**, and entering a directory reports its contents, so
  a limit of 1 yields entries at depth 2. That is off by one against the obvious reading of the
  name, and it is what ships. Controls for both directions of the comparison fire.
- The root is never reported. The walk answers what is *in* a tree, and a caller that wanted the
  root already had it.

A fixture was too weak and a control said so: the non-directory root check could be deleted without
failing anything, because the fake tree had nothing to list at that path either way. It now lists
contents there, so only the check stands between the walk and reporting them.

**The incremental rules** (`incremental-scanner.cpp`) are ported as decisions over a supplied disk
and index. 23 tests, 14 controls, all of which fired.

A full scan reads everything and needs no decisions. An incremental one exists to read as little as
possible, and **the two failures are not symmetric**: reading a directory needlessly costs one
listing, and failing to read one loses its files from the index silently — the search simply does
not find them and nothing says why. Every rule here leans the same way because of that.

- `lastModified >= cutOff` is **not** strict. Filesystem timestamps and scan records both land on
  whole seconds, so a strict comparison drops the directory written while the scan was finishing,
  which is the one most likely to have changed.
- The test is `changed **or** not tracked`. A directory can be older than the cut-off and still
  unknown to the index — a mounted disk, a restored backup, a folder moved with its timestamps
  intact — and an mtime test alone walks straight past it.
- A timestamp that cannot be read means **yes**, same asymmetry.
- The pruned scan's cut-off is found by walking *up* the parents, because a scan of `~` is what
  makes `~/code/project` up to date; asking only about the exact path finds nothing. With no record
  anywhere the cut-off is 0, so everything is newer and the whole tree is read, which is right for
  an index that has never been built.
- The scan path is read **however recently it was scanned** — it is the one directory the scan was
  asked about — but a path never scanned to completion yields only itself and the tree is not
  walked. An incremental pass with no cut-off to be incremental against would re-read everything
  while calling itself incremental.
- Deletion is *absence*: a path the index holds and the re-read listing did not produce. Nothing
  tells the scanner a file is gone, so a listing it re-read is the only evidence there is.
- The queue is first-in-first-out, so a scan interrupted early has covered the breadth it was asked
  about rather than one deep branch of it.

**One deliberate difference, declared.** The C++ dedupes the directories it *discovers* but not the
ones it starts with; here the check covers both. Unreachable through `scannable_directories`, which
cannot return a path twice, and the uniform rule is the better one if a caller ever does hand it a
repeat.

**The scheduling** (`scan-dispatcher.cpp`) is ported for its decisions, not its threads: the
debounce that turns a burst of filesystem events into one scan, and the queue rules that keep two
scans off the same directory. 28 tests, 16 controls, all of which fired — three only after a test
was added that they could fail.

The debounce is a quiet period **with a ceiling**, and both halves earn their place. Saving a file
in an editor is several filesystem events and a build is thousands, so a scan per event would keep
the indexer re-reading a tree that is still changing. But a quiet period alone never fires for a
directory that is never quiet — a log directory, a build tree, a folder mid-download — so the
deadline is the *earlier* of "five seconds after the last event" and "thirty seconds after the
first". The `min` is the whole mechanism; a control replacing it with `max` fires, as does one
measuring the ceiling from the last event rather than the first.

Two keys differ deliberately, and that is not an inconsistency. **Pending** scans are keyed by path
*and* type, because a full and an incremental scan of one directory do different work and neither
substitutes for the other. **Accepted** scans are checked by path *alone*, because two scans reading
one directory race each other's writes whatever kinds they are. One is about what to schedule; the
other about what may run at the same time.

A scan refused because its path is busy is **re-armed rather than dropped**: the events that asked
for it are real, and the running scan may have passed those files before they were touched. An
interrupted scan that is already running stays in the queue — a scanner told to stop has not stopped
yet, and removing it would let a second scan of that path start beside it.

**Three controls were silent and the tests were the reason.** Every timing test read
`DEBOUNCE_QUIET_SECS` and friends through the constants, so changing a constant moved the
expectation with it. The values are now pinned literally in a test of their own: 5 seconds, 30
seconds, 2 workers. A test written in terms of the thing it is checking cannot check it.

**The scan roots** (`util.hpp`) decide which directories are handed to all of the above. The list
comes from settings a user edits by hand, so it arrives with duplicates, relative paths and
overlapping subtrees, and scanning `~` and `~/code` separately does not merely waste a pass — the
two scans race each other's writes for the same rows. 29 tests, 15 controls, all of which fired.

Descent is compared **component by component**, never as text. `/home/user2` starts with the
characters of `/home/user` and is not inside it, and a string prefix test would silently drop one of
the two from the scan set — a bug that appears only for the user whose name is a prefix of someone
else's. The control replacing the comparison with `starts_with` fires.

The compaction sorts by **component count first**, then alphabetically, and the first half is what
makes it correct: an ancestor always has fewer components than its descendants, so it is accepted
before them. A plain alphabetical sort is not enough — `/a/b/c` sorts before `/a/bb`, so the
descendant would be weighed against a set that did not yet hold `/a/b`. The alphabetical tie-break
keeps the answer stable, which matters because a scan interrupted halfway should cover the same half
next time.

**Two controls were silent for a reason worth writing down.** `Path`'s own `Components` iterator
drops `.` on its own, and `PathBuf` compares by components — so a test asserting
`PathBuf == PathBuf` cannot tell a normalised path from an unnormalised one, and a `.` in the middle
of a path never reaches the code that removes it. The assertions now compare rendered text, and a
leading `.` — the only one the match arm ever sees — has a case of its own.

**Reconciliation and startup** (`file-indexer.cpp`) answer the same question from two directions:
which files should be in the index and are not, and which are in it and should not be. The second is
the worse to get wrong — a search returning files the user asked it to forget. 31 tests, 20 controls,
19 of which fired.

The settings diff has four rules and one of them turns on a single word. A new root is skipped when
an old root already covers it — but only when that old root **stays**. An old root that is itself
being removed is no coverage at all, because its rows are about to be deleted, and treating it as
coverage leaves the new root unscanned with its files gone from the index. The control dropping the
"stays" half fires.

The other three: a root the new settings no longer cover is deleted; a new exclusion is deleted
unless it was already excluded, since then there is nothing of it indexed to remove; and an
exclusion that has been **lifted** is scanned, but only inside a root — those files were skipped
while it stood and nothing else would ever go back for them.

The pending-full-scan set is what makes a full scan survive a restart. A full scan of a large tree
takes minutes and the launcher can be closed inside one; without the set, a settings change followed
by a restart leaves a root that was never scanned and never will be, because the config already
lists it and the startup path sees nothing to do. A finished scan clears everything **beneath** its
root rather than an exact match, and `roots_for` reports without pruning — a config change that is
later undone must not have lost the roots it was about to drop.

Startup has three cases and the middle one is why the other two are not enough. A full scan that did
not succeed means the index holds *part* of that tree with nothing recording how much: an
incremental pass would compare against the cut-off the interrupted scan wrote and skip everything it
never reached. So it is redone whole, and the old record is marked interrupted so it stops being
read as a cut-off. The watcher waits until no full scan is needed, because a full scan is already
walking the tree the watcher would report on.

**One guard was not ported because no mutation could make it fail.** The C++ skips an old exclusion
that is still excluded before deciding whether to rescan it; the drop that runs a few lines later
removes exactly the same paths. A guard whose whole effect is undone by a later line reads like it
is carrying weight, and it is not.

**The watch policy** (`important-dir-watcher-linux.cpp`) decides which directories earn an inotify
watch, which is a finite and *shared* resource: `fs.inotify.max_user_watches` is 8,192 on many
systems and every program on the desktop draws from it. A launcher that watched a whole home
directory would take all of them and break whatever asked next. 22 tests, 21 controls, all of which
fired.

The answer is not to watch less accurately but to watch **shallowly**, and to treat running out as
an expected outcome rather than an error. Two levels below each important root are watched;
`~/code/project/src` is not, and a change in it still shows up within a scan cycle, for a budget
that instead covers a hundred other projects' top levels. When the budget runs out the remaining
directories fall back to the scan cadence — which would have covered them anyway.

The walk is **breadth-first, and that is the whole design**: with a budget that can run out, the
order decides what is covered when it does. Depth-first would spend the budget inside the first root
and leave the others entirely unwatched; breadth-first covers every root's top level before any
root's second.

The roots are the home directory, its visible subdirectories, and then the XDG config and data
homes — which are hidden, so the enumeration skips them, and which are indexed regardless. Adding
them back explicitly is the only reason the function is not simply "list the home directory". With
no home there are no roots **at all**, the XDG directories included, because the C++ returns before
reaching them.

The two constants are pinned literally, having learned that from `scan_dispatch`: every other test
reads them through the names, so nothing else would notice them changing.

The ranking is the fourth leg. A fuzzy score alone would rank an editor's swap file above the file
it is a swap of, and `finalreport.pdf` above `report.pdf`. Every multiplier in the engine exists to
stop one of those, and each is now pinned with a test naming what it prevents:

- The substring bonus applies to the **remaining headroom** — `score + (100 - score) * bonus` —
  rather than multiplying the score. A candidate already near 100 gains almost nothing and one at 40
  gains a lot, so the bonus re-orders the middle of the list without letting a weak match overtake a
  strong one, and cannot push anything past 100.
- A token-start match is worth 1.5 and an inner one 1.05. The second is a tie-break rather than a
  ranking, because a query buried inside a longer word is weak evidence; treating it as strong is
  exactly what would put `finalreport.pdf` above `report.pdf`. The boundary test is
  **alphanumeric**, so `2024report` is one word to the ranker as it is to a reader.
- Editor and compiler leavings are **demoted, not removed**: a swap file is sometimes what you are
  looking for right after a crash, and a search that cannot find it is worse than one that ranks it
  last.
- A correction plan is an **and**. One word scoring zero drops the whole plan, because a correction
  finding files that match two of its three words is not a reading of the query but a different
  query. The surviving words are averaged rather than summed, so a three-word plan is not worth
  three times a one-word plan, and each is normalised against what it scores against *itself*, or a
  six-letter word would always outscore a three-letter one on the same quality of match.
- The four sort keys each exist because the one before it ties, and the third is the interesting
  one: a file comes before a directory, because a directory matching as well as a file inside it is
  usually not what was meant — the file is the thing you open.

One line of the C++ is deliberately not carried over, and a control is why: the guard against a
query longer than the text is unreachable here, since the loop's own bound already covers it. The
*empty*-query guard beside it is kept and is load-bearing — `find("")` succeeds at offset 0, which
would make every text a token-start match.

What the ports are worth is not in the score. An indexer that walks `/proc` never finishes, and one
that walks `~/.cargo/registry` fills the index with vendored sources that rank above the file
somebody wanted. Those rules are a long list of specific names rather than a general principle, and
a name quietly dropped from the list is not a bug anyone reports — it is a search that stops being
useful. So the list is now pinned, whatever colour the row is.

The query policy is the other half of that argument. Splitting and quoting a query is mechanical;
deciding *which* misspellings to try is not. Too few and a typo finds nothing, too many and the
index is asked several questions per keystroke, each ranking worse than the one the person meant.
None of that fails anything — it makes search feel slightly unreliable. Every rule now has a test
naming what it prevents: why a word the corpus knows well is left alone, why only one word per
family is tried and why the shorter one wins it, and why every plan after the first differs from it
in exactly one word rather than enumerating combinations nobody typed.

The tokenizer is the third leg of the same argument: a file turns up in a search only if one of its
tokens matches what was typed. Split too coarsely and `AnnualReport2024.pdf` is findable only by its
whole name; too finely and the index fills with fragments matching everything. Its two junk rules
are narrow on purpose — anything over 24 bytes, and anything twelve bytes or longer that is entirely
hexadecimal — because the twelve-byte floor is what stops the hash rule eating `deface`, `facade`
and `decade`, all of which are hex-shaped. Porting it turned up two bugs in the port rather than in
the C++: accumulating a token as `char`s corrupts a UTF-8 filename, and the C++'s length limits are
in bytes rather than characters, which makes them stricter for a non-Latin name.

Two C++ behaviours it reproduces rather than fixes, both in `GitIgnoreReader`: only the *filename* is
matched against a pattern, so `build/*.o` never matches anything, and a leading `/` is stripped
rather than anchoring, so `/target` matches a `target` anywhere in the tree. The C++ calls this a
hack in its own comment. Narrowing it would start indexing directories people's `.gitignore` files
currently keep out; widening it would drop files they expect to find. Every line of an ignore file
becomes a pattern too, comments included — a `#comment` glob never matches anything real, which is
why nobody has noticed.

### `compass-core::raycast_store` — the same escaping fix, and a filter deliberately skipped

`search` and `fetchExtension` build their URLs with `QString::arg` exactly as the Vicinae store's do,
so the same divergence applies and for the same reason: an extension name containing a `/` would
otherwise add a path segment and ask the API for something else.

What is *not* changed is the platform filter being skipped on Linux entirely. Raycast is a macOS
product and its extensions advertise `macos`; filtering on that here would produce an empty store
rather than a best-effort one, which is what the C++ comment says. The compatibility sheet is the
other half of the bargain — fetched only on the platform where the filter was skipped, and saying
which of those extensions actually work. `the_compat_sheet_exists_only_where_the_filter_was_skipped`
pins that the two conditions are exact opposites, so a platform can never both skip the filter and
have nothing to consult.

A failed compat fetch is not an error: the C++ warns, returns an empty map, and leaves its
`m_compatFetched` flag false, so the store keeps working without notes and the next request tries
again. Both halves are reproduced.

### `compass-core::extension_store` — a search query that is actually escaped

`VicinaeStoreService::search` builds its URL with
`QString("/store/search?q=%1").arg(query)`, which substitutes the query verbatim. A query containing
`&`, `#` or `=` therefore changes the *shape* of the URL rather than the value of `q`: searching the
store for `a & b` asks the server for `q=a ` plus a parameter called ` b`, and searching for `c#`
sends `q=c` with a fragment.

The port percent-encodes the value. Everything unreserved is left alone, so an ordinary search
produces byte-identical output to the C++ and only the queries that were already broken change.
`an_ordinary_search_makes_the_url_it_always_made` pins the first half and
`a_search_containing_an_ampersand_stays_one_parameter` the second.

### `compass-core::semver` — an overflowing component is refused rather than wrapped

`Semver::parse` accumulates into an `unsigned` with `current * 10 + digit` and no overflow check, so
a component past 2^32 wraps. `4294967296.0.0` therefore compares equal to `0.0.0`, and a release
tagged that way would look like no release at all. The port returns `None`, which makes the tag "not
a release tag" — a refusal the update service already knows how to ignore — rather than a wrong
answer it would act on.

What is *not* changed is that `Semver` is not semver. It parses a dotted run of decimal integers and
nothing else: `v1.2.3-rc1` does not parse. That is load-bearing, because it is what keeps release
candidates from being offered as updates without anyone having to filter them, and
`a_prerelease_tag_does_not_parse_at_all` says so where someone might otherwise "fix" it.

One bug found while porting, in the port rather than the C++: deriving `PartialEq` compares the
component lists structurally, which would make `1.0` and `1.0.0` unequal *and* neither greater — a
contradiction a sort or a hash map can act on. The C++ defines `operator==` in terms of `<=>`; the
port now does the same, and hashes on the components with trailing zeroes removed so `Hash` agrees
with `Eq`. `equality_agrees_with_the_comparison` pins it.

### `compass-core::telemetry` — two C++ bugs deliberately **not** reproduced

`TelemetryService::setEnabled` remembers its previous value in a *function-local `static`*. That
makes the flag process-wide rather than per-instance: it is shared by every `TelemetryService` and
survives one being destroyed. With a single service per process this is invisible, which is
presumably why it has lasted. With two, the second one's first `setEnabled(true)` is swallowed as
"no change" and its telemetry silently never starts. The port keeps the flag on the instance, which
is what the code reads as though it did.
`each_service_keeps_its_own_enabled_flag` pins it.

`loadState` warns on an unreadable state file and carries on with whatever glaze left in `m_state` —
for a parse failure that is a default-constructed `State`, so `userId` is the empty string and every
record from then on is filed under `""`. Not a crash, and not visible locally: it just quietly
detaches that machine's records from each other. The port generates a fresh id instead and writes it
back, so the records stay attributable to *a* machine and the next run is stable again.
`a_corrupt_state_file_gets_a_fresh_id_rather_than_an_empty_one` pins both halves.

Neither divergence changes what is collected, only whether it is coherent. What *is* reproduced
exactly is the record's shape: glaze serialises C++ member names as written, so the wire keys are
`userId`, `vicinaeVersion`, `systemInfoLastSentAt` and the rest in camelCase, and the port renames
its snake_case fields to match. Also reproduced is which fields are lowercased — `architecture`,
`buildProvenance`, `vicinaeVersion` and each entry of `desktops`, and not the other seven. That
asymmetry looks accidental, but normalising the rest would make this engine's records group
differently from the C++'s in the same dataset.

### `compass-core::paste` — a copy that happens even when the paste cannot

`PasteService::pasteContent` calls `copyContent` first and only then asks whether the platform
supports pasting. On a platform that does not, the content is on the clipboard and the caller is
told `false`.

Reproduced rather than tidied. It is arguably the more useful outcome — the person can paste it
themselves — and reordering would break a caller that retried on `false`, which would then copy
twice. `a_platform_that_cannot_paste_still_gets_the_copy` pins both halves: the copy happened, and
nothing was scheduled.

The timeout path is the same kind of thing and is pinned the same way: when focus never lands the
paste is dropped and the clipboard is *not* restored, so what was copied is still there. A port that
helpfully restored it would take away the only consolation prize the failure has.

### Paste on GNOME — the focus wait moved into the Shell, and it is event-driven

The C++ polls `focusedForeignWindow` every 5 ms for up to 5 s, waits another 30 ms and injects
Ctrl+V (Ctrl+Shift+V for a terminal) through its input server. On GNOME 50/51 the Rust engine can do
none of that: no virtual-keyboard protocol, no `/dev/uinput` on Bluefin, and no focus signal outside
the Shell. So contract v2's `Clipboard.Paste(as)` does it inside the Shell. The extension waits on
`notify::focus-window` for focus to leave the window that had it when it was called, then 30 ms, then
presses the shortcut through a Clutter virtual keyboard.

Three things differ on purpose:
- the wait is 2 s, not 5 s, because it is a signal, not a poll that can miss;
- "is this a terminal" is decided before focus moves. The engine sends every `TerminalEmulator`
  application's normalised window classes, and the extension matches the window it lands on;
- the clipboard is not restored afterwards.

What is kept is the copy that happens anyway (above). The engine sets the clipboard before arming the
paste, and a launcher whose engine cannot paste copies the entry itself.

### `compass-core::audio_control` — a volume that is not a number no longer takes the process down

`toAudioSink` reads a channel's volume with `std::stod(percent)`, which parses the leading number
and ignores the `%`. On a string with no leading number it does not return anything — it throws
`std::invalid_argument`, from inside a function with no `try` anywhere above it. A `pactl` that
printed `"n/a"` for a channel would end the process.

The port reads the leading numeric run and falls back to 0.0, which is what the sink already reports
when its channel map is empty, so the failure mode is "this sink reads as silent" rather than
"Compass exited". `a_volume_that_is_not_a_number_reads_as_zero_rather_than_crashing` pins it.

The lexicographic channel rule *is* reproduced: the C++ takes `volume.begin()->second` from a
`std::map`, so a stereo sink reports its `front-left` level, and this port uses a `BTreeMap` to get
the same answer. Two controls pin it — taking the last channel instead, and averaging — both of
which fail the suite. (Swapping the `BTreeMap` for a `HashMap` does *not* reliably fail it, which is
a defect in that mutation rather than in the test: it makes the result vary per process instead of
being wrong in a fixed way, so a suite that passed once proves nothing either way.)

### `compass-core::snippet_expander` — an argument value is not marked as a placeholder

Every substitution the expander makes is pushed with `placeholder = true` except one: an argument's
value goes in through `result.parts.emplace_back(it2->second)`, which takes the struct's default of
`false`. So whatever highlights placeholders in a preview shows a clipboard or a date as
substituted and an argument's value as ordinary text.

Reproduced rather than unified. It is arguably the right answer — the person typed that value, so it
*is* their text — and changing it would alter what an existing preview highlights without anyone
having asked. `substituted_placeholders_are_marked_and_arguments_are_not` pins it either way.

Two more worth knowing, both found by writing the tests rather than by reading the code. A shell
placeholder whose command contains spaces must be quoted: the parser ends an unquoted value at the
first space and then drops the whole placeholder, so `{shell code=rm -rf /tmp/x}` expands to nothing
at all. That is the safe failure — nothing dangerous is half-run — and it now has a test saying so.
And the shell result index advances whether or not a result was available, so a run that returned
fewer outputs than there were placeholders shows each remaining one as its own `$(code)` rather than
shifting every later one onto the wrong command.

### `compass-core::font_service` — two tables extracted, not retyped

The category names and the per-script pangrams were pulled out of the C++ with a script and written
into the Rust source mechanically. That is not laziness: a pangram exists to exercise every letter
of a script, and one retyped with a character wrong still looks right to anyone reviewing the diff
— particularly in Thai, Devanagari or Arabic. Earlier in this session three power-command
descriptions were written from memory and all three were wrong, which is the same failure caught
late rather than avoided.

Two classification rules are worth reading twice. A Nerd Font outranks monospace, because a patched
font is almost always a monospace Latin one and without that order the Monospace section would
contain nothing else; both are still tagged, so filtering by either finds it. And a font covering
several distinctive scripts *plus* a European one is filed under Latin rather than under the first
of them — that is a pan-Unicode font, and burying it under Gujarati would hide a general-purpose
font from everyone.

### `compass-core::window_effects` — two registries whose support checks are in opposite orders

`WaylandShortcutInhibitManager::inhibit` tests whether the window already has an inhibitor *before*
it tests whether the compositor still supports the protocol.
`ExtBackgroundEffectV1Manager::apply` does it the other way round: `if (!isSupported()) return
false;` comes first, so a window that already has a blur is told the apply failed once blur goes
away.

Reading it once, that looks like an inconsistency to tidy. It is the right way round. A Wayland
global can be withdrawn while the process runs, and the two stale states are not comparable: a blur
that lingers is cosmetic, while an inhibitor that lingers means the keyboard is still grabbed. A
manager answering "no" for a grab it is still holding would invite its caller to stop tracking it,
and the person is then locked out of their own desktop shortcuts with nothing to release them.

Both orders are pinned, and pinning them needed the support flag to be *settable* — with support
fixed at construction there is no way to reach the case that distinguishes the two, which is why
the port models `isActive()` as something that changes rather than as a constructor argument.

### `compass-core::alert` — a replaced alert is a cancelled alert, and the caller has to be told

`AlertModel::handleAlertRequested` calls `triggerCancel()` on the alert already showing before it
takes the new one. The TypeScript API documents the consequence — "Calling this function when
another alert is currently pending will result in the pending alert to be automatically canceled" —
and it matters because the cancelled alert is some extension's un-settled promise.

So [`AlertModel::show`] returns the previous alert's resolution rather than dropping it. That is a
shape change from the C++, where the resolution goes out through a callback the widget owns, and it
is deliberate: a caller that ignores a `#[must_use]`-shaped return is a caller a compiler can
complain about, where a caller that forgets to connect a signal is not.

The port also keeps the C++'s routing of *every* exit that is not the confirm button — cancel,
navigating away, being replaced — through one place that answers `false`. A fifth exit added later
should have to opt in to `true` rather than out of it.

### `compass-platform-linux::keyboard` — four modifiers that are declared and ignored

`UInputKeyboard::Modifier` names six modifiers. `applyMods` and `clearMods` test two of them. A
caller passing `Alt`, `Logo`, `Altgr` or `Capslock` gets a bare keystroke with no modifier held —
and still gets the *slow* path, because the C++ chooses its delay on `mods ? ... : ...`, the raw
integer, before deciding what to do with it. So an ignored modifier costs 10ms per key and changes
nothing.

Both halves are reproduced and pinned. Making Alt work would be the obvious fix and the wrong port:
a snippet bound to Alt+F would send a keystroke on Compass that it does not send on Vicinae, with no
way for the caller to tell which build it is on. The tests are named for what they protect, and are
the ones that should fail when the C++ grows the other four branches.

Two trailing `SYN_REPORT`s in `sendKey(code, mods)` are likewise redundant — `sendKey(code)` already
ends with one — and likewise kept, with `a_shifted_keystroke_is_exactly_this_sequence` pinning the
whole wire in order.

### `compass-core::fetch_queue` — an abort that frees a slot without filling it

`NetworkFetcher`'s abort handler erases the reply from the in-flight map and emits
`abortRequested`. It does not call `startRequests`. So aborting an in-flight image fetch leaves one
of the six slots empty until some *other* request finishes, and a queued request that could have
taken it waits instead.

The port reproduces this, and `aborting_an_in_flight_request_does_not_start_the_next_one` pins it
with a control that fires the moment the slot is refilled. Calling `start_requests` there is the
better scheduler and the wrong port: a person scrolling a list of remote icons fast enough to
cancel requests would see Compass issue a different number of them than Vicinae, at different
times, and any comparison of the two under load would be measuring this difference rather than the
thing being compared. `the_slot_an_abort_freed_is_taken_by_the_next_completion` pins the other half
— the queue is delayed, not wedged.

### `compass-core::root_items` — a hash order made deterministic

`searchGroupedByProvider` buckets into a `std::unordered_map<std::string, Bucket>` and then stable-sorts
the groups by score. Stable sort preserves the order it was given, and the order it is given is the
map's iteration order — a hash order over the provider ids present. Two groups that score equally
therefore come out in an order that depends on which other providers matched, and can change between
builds of the same binary.

Compass buckets in first-appearance order instead: the first item belonging to a provider creates its
group, so a tie resolves to the order the items were registered in. `groups_that_tie_keep_first_appearance_order`
pins it. There is no "fails if the C++ is fixed" test to pair with this one, because the C++ is not
wrong in a way a test can name — it is unspecified, and this is a choice within it.

### `compass-power` — one C++ bug deliberately **not** reproduced

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | `SystemdPowerManager::can` calls `CanPowerOff`/`CanSuspend`/`CanHibernate`/`CanReboot` and then answers `!reply.arguments().isEmpty()` — it never reads the reply. logind answers with a *string*: `"yes"`, `"no"`, `"challenge"` or `"na"`. All four are a non-empty argument list, so a machine that cannot hibernate is offered Hibernate, and the menu entry does nothing. | Read the string. `Capability::is_offerable` is true for `yes` and `challenge` (polkit will ask), false for `no`, `na` and anything this build does not recognise. | `logind_replies_are_read_rather_than_counted`, `the_capability_reply_is_read_and_not_merely_counted` (drives a real reply through a mock logind), and `the_cpp_still_has_the_bug_this_port_declines_to_copy`, which fails if the C++ is fixed |

### Media commands — what the port does not have yet

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | Play / Pause, Next Track and Previous Track confirm in the launcher's HUD (`Paused`, `Playing A Song — Artist`, `Next Track`); where there is no HUD (no layer shell) nothing is shown. | The engine sends the sentence to the launcher's HUD (IPC v19 `WindowCommand::Hud`), with the C++'s icon for the player commands; where the window has no HUD it posts a transient desktop notification (1.5 s, `transient` hint) instead of showing nothing. The volume commands' HUD has no icon. Refusals ("No media player is running", "Spotify cannot skip to the next track") show in the HUD, since the launcher has already hidden, and in the launcher when it is next summoned, as the power commands' do (#254). | `a_media_command_says_why_it_did_nothing`, `a_media_command_runs_at_once_and_shows_why_it_did_nothing` |
| 2 | The player commands take an optional `player` argument, fuzzy-matched over the running players (title 1.0, artist 0.8, identity 0.6); Turn Volume Up/Down take an optional `step`. Both are typed inline beside the search field. | The same matching and the same refusals ("No media player matches …", "Invalid step value"), with no argument taking the default player (last acted on, else playing, else first) or ±5. The launcher has no inline argument fields, so Enter runs the command at once and the row's action panel offers "Choose player…" / "Choose step…", a one-field form. | `a_player_argument_picks_the_player_and_now_playing_lists_and_drives_them`, `a_media_command_runs_with_the_player_chosen_in_its_form`, `a_volume_command_runs_pactl_with_the_cpp_arguments` |
| 3 | Volume goes through `pactl`. | The same `pactl` invocations, through `flatpak-spawn --host` inside the Flatpak, with the C++'s 3 s timeout. `libpulse-binding` was considered and not taken: a C build dependency and a threaded mainloop for five calls the ported `pactl` adapter already makes. | `a_volume_command_runs_pactl_with_the_cpp_arguments` |
| 4 | Now Playing lists the players ("Players", fuzzy over title, artist and name), with Playing/Paused accessories, the player application's icon, and Play or Pause, Next Track and Previous Track; it reloads on `playersChanged`. | The same list, filter, accessories and actions (Enter is the first); a row shows the player's initial rather than its application's icon, and the list is asked again 300 ms after each action rather than on a bus signal, so a player changed from elsewhere shows when the view is next opened. | `now_playing_lists_the_players_and_controls_the_selected_one`, `a_player_is_found_by_track_artist_or_name_and_stays_selected` |

### Opening Compass's own files (#253)

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | Open Config File opens the user's settings file. | This engine writes `compass.json` only when something is saved, so a fresh install had no file and the command was refused. The command now writes it first (the C++ settings migrated if they are there, else the defaults, with `$schema`), then opens it. | `the_config_and_log_commands_open_compass_files_and_say_why_not`, `a_missing_file_is_written_and_an_existing_one_is_left_alone` |
| 2 | The file opens with what the desktop opens its type with. | The same, and, for a text file whose type has no opener of its own, the text editor (`text/plain`'s default, then the first `TextEditor`). On GNOME the editors claim `text/plain` only, so JSON had no opener and nothing opened. An editor with `Terminal=true` (Vim) runs in the terminal rather than with nowhere to draw. | `a_text_file_with_no_opener_of_its_own_opens_in_the_text_editor`, `search_files_lists_recent_files_for_the_empty_query_and_a_typed_path_directly` |
| 3 | The open is fire-and-forget. | The engine waits a second to hear that the application started; when nothing opens the file, or the launch fails, it asks the `OpenURI` portal (`OpenFile` and `OpenDirectory` take a descriptor, so this also works from inside the Flatpak), and otherwise answers with the reason, which the launcher shows instead of hiding. | `search_files_lists_recent_files_for_the_empty_query_and_a_typed_path_directly`, `the_config_and_log_commands_open_compass_files_and_say_why_not` |
| 4 | Open Default Config File writes `default-config.jsonc` to the runtime directory. | It writes it to the cache directory's `compass`: inside a Flatpak `$XDG_RUNTIME_DIR` is the sandbox's own, and the host's editor cannot see a file there. | `the_config_and_log_commands_open_compass_files_and_say_why_not` |
| 5 | Show Log File opens `$XDG_STATE_HOME/vicinae/vicinae.log`. | `$XDG_STATE_HOME/compass/compass.log`, the one path `compass_core::xdg_dirs::log_file` names for both the engine that writes it and the command that shows it; before the engine has written it, the command says so. | `the_config_and_log_commands_open_compass_files_and_say_why_not` |

### Search Files — what the port does not have yet

The command runs end to end: the engine starts `compass-file-indexer` with the file extension's
preferences (`providers.files.preferences` in `compass.json`: `autoIndexing`, `indexingPaths`,
`excludedIndexingPaths`, defaulting to on, the home directory, nothing), restarts it with the C++'s
backoff, answers `SearchFiles` with recent files, a direct path or ranked index matches, and opens
a file with its default application (Enter) or shows it in the file browser (Ctrl+Enter). What
differs:

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | An index query while the indexer is not running answers an empty list. | Refused with `Unsupported` and a sentence saying the indexer is off or missing, which the launcher shows; an empty list would read as "no such file". | `search_files_without_indexing_says_the_index_is_unavailable`, `an_index_query_without_an_indexer_says_so` |
| 2 | The category filter is a dropdown beside the search field, stored per command (`fileCategory`) and restored when it is not "All". | A dropdown over the list with the same keys, sent as `SearchFiles.category` and applied by the engine and the indexer; remembered with the same rule in the launcher's state file (`view_memory`), as Browse Fonts' is, rather than the keyring-backed command storage. | `search_files_filters_by_a_remembered_category_and_previews_the_selection`, `search_files_indexes_the_home_directory_and_finds_a_file_by_a_misspelled_query` |
| 3 | A detail pane previews the selected file (name, path, MIME type, modified time, image or text). | The same pane (`compass_ui::file_preview`, shared with dmenu's quick look): the path with home folded, the modified time as `QDateTime::toString()` writes it, an image drawn or the first 10 KiB of a text file. Rows also carry the folder as their subtitle, where the C++ row has none. The MIME type comes from the extension. | `search_files_filters_by_a_remembered_category_and_previews_the_selection`, `a_text_file_shows_its_start_and_an_image_itself` |
| 4 | The action panel: Open with…, Run executable (AppImage), Set as wallpaper, Create shortcut, Paste, Copy file / path / name / MIME type. | Only the primary action (open with the default application for the file's MIME type) and Show in file browser. | `enter_opens_the_file_and_ctrl_enter_shows_it_in_the_file_browser` |
| 5 | Opening a file records it in `recently-used.xbel` (`recordAccess`: `vicinae` added as an application, the MIME type set, the whole file written back owner-only through a temporary file), so it tops the empty query next time. | The same, through `compass_xdg::bookmarks::record_access` (written with `xmlwriter`); as in the C++, elements the bookmark model does not hold are not written back. | `recording_an_access_adds_then_bumps_and_keeps_the_rest`, `search_files_lists_recent_files_for_the_empty_query_and_a_typed_path_directly` |
| 6 | Show in file browser selects the file through `org.freedesktop.FileManager1`. | The same `ShowItems` call (a `zbus` proxy, 3 s timeout), for extensions' `showInFileBrowser` too; when nothing on the bus implements it, the folder is opened with the `inode/directory` handler. | `show_in_file_browser_asks_file_manager1_to_select_the_file` |
| 7 | Search Files is a fallback command: a non-empty query lists the `fallbacks` (default `["files:search"]`) under `Use "<query>" with...`, and choosing one opens it searching for the query. | The same section and heading after the results, from `fallbacks` (Search Files by its C++ id or its Compass one); the C++ hides the section while a file search it runs in root search is still answering, which Compass's root search does not do. | `a_query_offers_search_files_as_a_fallback_that_searches_for_it`, `search_files_is_the_one_fallback_by_either_id` |
| 8 | Scan progress (`scanStatusChanged`) feeds a status indicator. | The client tracks scans, and nothing shows them. | — |
| 9 | Recent files come from `$XDG_DATA_HOME/recently-used.xbel`. | The same — which inside the Flatpak is the sandbox's own data home, not the host's, so there the empty query falls through to "Recently Modified" from the index. | — |

### The extension sandbox — what an extension may not do that the C++ let it

The C++ runs extensions unconfined. Compass runs them behind `compass-sandbox-exec` (Landlock,
seccomp, a heap cap and now a data limit), so every row here is a divergence by construction.
The negative tests are §8.2's list; each has a positive control beside it.

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | An extension may run a program it wrote itself (Raycast's `speedtest` downloads its CLI into `supportPath` and runs it). | Execute is granted on the system trees (`/usr`, `/bin`, `/lib*`, `/app`), Node and the extension's installed directory, never on the directories it may write: running what it wrote fails with `EACCES`. The Landlock crate's "read" set includes `Execute`, which had made every readable path executable; read no longer implies execute. Suite 1's `speedtest` fails here, by design. | `a_program_the_worker_wrote_itself_cannot_be_run`, `a_command_sees_its_own_paths_and_preferences_and_may_exec_but_not_unshare` |
| 2 | A raw socket is whatever the kernel allows the process. | `socket()` with `SOCK_RAW` or `SOCK_PACKET`, or in `AF_PACKET`, answers `EPERM` from the seccomp filter, root or not; an ordinary socket is unaffected. | `a_raw_socket_is_refused_while_an_ordinary_one_is_not` |
| 3 | No memory limit; the worker asks V8 for 1000 MB of heap. | The heap is capped at 160 MiB (`--max-old-space-size`), and `RLIMIT_DATA` at 512 MiB, which bounds `Buffer`s and native allocations where no cgroup is reachable (a Flatpak): a 512 MiB `Buffer` is a `RangeError` the extension can catch. Measured over Suite 1 the worker's `VmData` peaks at 340 MiB. The heap cap costs one real extension, **kept deliberately**: `dashboard-icons` groups a 1.2 MB catalogue into 4,473 grid items, each with its own action panel. Measured (2026-09-24, caps lifted one at a time): it runs out of heap at 160 MiB and renders at 192 MiB, and at 192 MiB the worker peaks at about 450 MiB resident and 490 MiB `VmData`. That is past the 256 MiB process budget (§6) by more than the heap alone, so raising the heap flag would only move its failure to the cgroup or `RLIMIT_DATA`; admitting it means a different budget, not a different flag. It also needed row 1 of "The extension host API" (a view past a mebibyte). | `an_allocation_past_the_data_limit_fails_and_the_process_carries_on`, `an_extension_that_allocates_past_the_heap_cap_is_stopped` |
| 4 | Writes anywhere the user may. | Writes only its support and asset directories: `reminders` (Vicinae store) fails making `~/.local/share/vicinae-reminders`. | `an_installed_extension_command_is_found_and_a_no_view_one_runs` |
| 5 | TLS trusts whatever `NODE_EXTRA_CA_CERTS` names. | The same, because the file it names (and `SSL_CERT_FILE`, `SSL_CERT_DIR`) is granted read; otherwise Node could not load a corporate CA from `$HOME`. | — |
| 6 | Reads anywhere the user may. | Reads the system trees, Node, the runtime bundle, the extension's own directory and its support and asset directories, and of `$HOME` only a short read-only allowlist, **decided 2026-09-25** ("The gaps pass, tray and sandbox"): `~/.ssh/config` (not the keys or anything else in `~/.ssh`), `~/.password-store`, and `~/.config/hypr`, `~/.config/sway` and `~/.config/niri`, each where it exists, one constant (`compass_sandbox::home::HOME_READ_ALLOWLIST`). A link among them is followed only to a place the list names. So `ssh`, `pass` and the `niri` and `hypr*` keybinding lists read what they need; `firefox` the profiles, `zoxide-recent-directories` its database and Raycast's `obsidian` a vault still see nothing, where the C++ let them read them, and `pass` lists its entries but cannot decrypt them (`~/.gnupg` stays denied). **Suite 1 cannot see this**: its `HOME` is empty, so these fail (or pass, as `ssh` does with a typed host) for the same reason under either policy. Widening reads to `$HOME` would admit every one of these and also every secret in it; the allowlist grows one named entry at a time. | `compass_sandbox::home::tests::*`, `the_policy_reads_only_the_allowlisted_home_paths_and_never_writes_them`, `an_extension_reads_the_home_allowlist_and_nothing_else_of_home` |

### The extension host API — where the engine answers differently, and what it serves

Found by Suite 1 (`scripts/suite1/`), each against a real store extension. The adapters in
`compass-worker-host` were pinned against the C++ before; these are the engine's backends behind
them and three places where the answer an extension gets differs.

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | The UI receives an extension's view in-process, whatever its size. | The launcher and `compass conformance` receive it over IPC as `ExtensionView`, whose frames were capped at 1 MiB — about a thousand list items with their actions. The cap is now 32 MiB (`compass_ipc::MAX_FRAME_LEN`), still checked from the prefix before anything is reserved. No wire change and no protocol bump: an older peer refuses a large frame as it always did. | `an_extension_view_of_several_mebibytes_is_carried` |
| 2 | `Storage/get` of a missing key answers `null`. Raycast resolves `undefined`, and Google Search tests `=== undefined` before `JSON.parse`, so on the C++ host it crashes on `null.filter` with an empty history. **A C++ bug not reproduced.** | The reply carries no `result` member, which the generated client resolves as `undefined`. A stored value cannot be `null`, so nothing else changes. | `a_missing_key_reads_as_undefined_not_null` |
| 3 | `getSelectedText` reads the primary selection the data-control clipboard server last reported, else Qt's (which needs the launcher focused), else fails "Unable to get selected text". | The primary selection over data-control on a wlroots compositor (`compass_wayland::clipboard::read_primary_text`), and through the Shell extension on GNOME (`GetPrimarySelection`, contract version 3: Mutter has no data-control and a Wayland client may read the primary selection only while it has keyboard focus, which the engine never has). The failure is the C++'s, verbatim. | `the_primary_selection_reads_back_and_is_not_the_clipboard`, `on_sway_an_extension_reads_the_selection_the_windows_and_the_monitors`, `the_primary_selection_is_its_text_or_nothing` |
| 4 | `WindowManagement` is the window manager provider's: windows, workspaces, screens, bounds. | Served by the engine (`extension_windows`), over the window switcher's backends in the C++ order: **Hyprland's or niri's own IPC** first (the C++ providers, ported: windows with their workspace and pid, Hyprland's with geometry; the workspace list; the active workspace; focus through the compositor), then the foreign-toplevel list on other wlroots compositors (no bounds, no workspace), then the Shell extension on GNOME (whose `ListWindows` gained a frame and `fullscreen` in contract 3). Screens come from `wl_output`, with `zxdg_output_manager_v1` for the logical layout, on any compositor; the active one is the one under the active window, or the only one. The *active window* is the focused one, or — with the launcher focused, as it is when an extension asks — the one before it in most-recently-used order, which is the window the C++'s focus memory returns; on Hyprland it is the C++'s `getFrontmostWindowSync` (lowest focus history on the active workspace). **Not served** off Hyprland and niri: workspaces (`getActiveWorkspace` fails "No active workspace", `getWorkspaces` is `[]`). `setWindowBounds` is refused everywhere ("Failed to set window bounds"), as the C++ Hyprland and niri providers refuse it. | `the_focused_window_is_active_unless_it_is_the_launcher`, `with_the_launcher_focused_the_window_before_it_is_active`, `the_screen_under_the_focused_window_is_the_active_one`, `the_headless_output_is_listed_with_its_name_and_mode`, `without_a_desktop_the_selection_and_window_apis_answer_as_the_cpp_does`, `on_hyprland_windows_workspaces_and_focus_come_from_its_socket`, `an_extension_searches_files_sets_the_wallpaper_and_sees_hyprland_workspaces` |
| 5 | `FileSearch/search` asks the file indexer's `queryAsync`. | The same helper Search Files asks (`compass-file-indexer`, supervised by the engine), its rows only — no recent files, no direct path — with the adapter's C++ quirks (an omitted `limit` asks for none). With indexing off or the helper not running the answer is `[]`, and `environment.canAccess(FileSearch)` is `false`. | `an_extension_searches_files_sets_the_wallpaper_and_sees_hyprland_workspaces` |
| 6 | `Wallpaper/set` goes to `WallpaperManager`: the first activatable of hyprpaper, swww/awww, GNOME, KDE, Cinnamon, MATE. | The same order and the same tests of activatability (`hyprctl hyprpaper listactive`, `swww query`, the desktop name with `gsettings` on the path, `org.kde.plasmashell` on the bus), resolved once per engine; the commands and the Plasma script are `compass_core::wallpaper`'s, run on the host (`flatpak-spawn --host` in the Flatpak). The C++'s messages, verbatim: "Setting the wallpaper is not supported in the current environment", "No such file: …", a failing program's stderr else "… exited with code N". Inside the Flatpak the host's path cannot be searched, so a program counts as present and running it decides. | `an_extension_searches_files_sets_the_wallpaper_and_sees_hyprland_workspaces`, `a_failing_command_says_its_stderr_else_its_code`, `compass-core/tests/wallpaper.rs` |
| 7 | `BrowserExtension/*` reads the tabs the browser extension's native host reported. | No browser ever connects (ADR-0008 took browser control out of the port), so the engine answers as the C++ does with none connected: `getTabs` is `[]`, `focusTab` succeeds and reaches nothing, and `canAccess(BrowserExtension)` is `false` (the C++'s own test: `!browsers().empty()`). | `an_extension_searches_files_sets_the_wallpaper_and_sees_hyprland_workspaces` |
| 8 | `Command/launchCommand` pushes the sibling on the navigation stack, over the view that asked; `openCommandPreferences`/`openExtensionPreferences` open the settings window at the command; `updateCommandMetadata` overrides the root row's subtitle in memory. | The launcher is another process, so a launch is kept under a token and the window is told to take it (`WindowCommand::Launch`, IPC v15), then runs the command as if it had been picked in root search — its arguments form, preferences form and view included. **The view that asked is closed**, not kept beneath: the launcher shows one extension view at a time. The launch context and fallback text ride along to the command's next run (within five minutes). With no window attached a no-view sibling runs in the engine; a view sibling has nowhere to go and is dropped with a warning. Preferences open the command's preferences form in the launcher (saved, not run) rather than a settings window. The subtitle override is in memory as in the C++, shown by root search and served to the window (`ExtensionSubtitles`). "No such command", verbatim, for a command that is not installed. | `an_extension_launches_a_sibling_relabels_itself_and_opens_its_preferences`, `a_sibling_is_launched_through_the_window_with_its_arguments_and_context`, `an_extension_s_launch_runs_its_command_and_its_subtitle_override_shows`, `preferences_an_extension_opens_are_saved_without_running_it` |

### Extension views — remote images, date, tag and file pickers, and dialogs

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | Remote images go through a `QNetworkDiskCache` under the cache directory with `PreferCache`, up to 5 GB. | Fetched with `ureq` into `$XDG_CACHE_HOME/compass/images` (ADR-0017: Compass's own files), one file per URL by SHA-256, served from disk once fetched, oldest pruned past **256 MB**. Only PNG, JPEG and SVG are kept, recognised by their bytes; anything else keeps the row's initial. Markdown images in a `Detail` are not fetched yet. | `a_stored_image_is_found_again_and_a_different_url_is_not`, `the_bytes_decide_the_kind_not_the_url`, `pruning_removes_the_oldest_until_the_budget_holds`, `row_icons_resolve_assets_file_urls_themes_and_colour_cells` |
| 2 | `Form.DatePicker` is a calendar. | A text field in `YYYY-MM-DD` (or `YYYY-MM-DD HH:MM`), sent as a local timestamp without a zone once it parses; half-typed or impossible dates are kept as typed and not sent. An extension's own value is shown by its first characters, so a `Z` value shows its UTC time. | `a_typed_date_becomes_a_local_timestamp_javascript_parses`, `half_a_date_or_an_impossible_one_is_not_sent` |
| 3 | `Form.TagPicker` is a searchable token field. | Every option as a toggle, chosen ones marked; the value is the chosen values in the options' order. No search within the options. | `toggling_a_tag_keeps_the_options_order` |
| 4 | `Form.FilePicker` opens a Qt file dialog. | The XDG FileChooser portal (`compass-portals`), which inside a Flatpak is also what grants the extension the file. A picker that takes directories and not files asks for a directory; one that takes both asks for files, since the portal offers one or the other. | — (portal; the VM tier) |
| 5 | A second `confirmAlert` cancels the first; leaving the view cancels an open one. | The same: both answer the waiting promise `false`, whether the launcher pops the view or the extension pushes or pops one itself. | `a_replaced_alert_and_one_navigated_away_from_both_answer_no` |

### `OAuth/authorize` — no overlay, and the redirect as a deeplink

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | An overlay names the provider and waits for "Open browser". | The browser opens at once, with the default `x-scheme-handler/https` application, and the view shows a toast ("Continue in your browser to connect …") until the redirect arrives; then "Connected to …" or the provider's refusal. | `an_oauth_authorization_opens_the_browser_and_the_redirect_answers_it` |
| 2 | `vicinae raycast://oauth?code=…&state=…` reaches the running server through the C++ IPC `oauth` command. | `vicinae <url>` becomes `compass deeplink <url>`, which sends `OAuthRedirect` (IPC v12); the Flatpak exports `org.tunaos.compass.UrlHandler.desktop` for `compass:`, `raycast:`, `com.raycast:` and `vicinae:`. The store's extensions links open a detail page (IPC v16 `OpenDeeplink`, "Extension Store and Raycast Store" #13); every other deeplink the C++ takes is refused by name. | `a_bare_deeplink_becomes_the_deeplink_command`, `every_redirect_shape_raycast_uses_parses` |
| 3 | An authorize URL without a `state` waits for ever. | Refused at once: nothing could match a redirect to it. | `a_url_without_a_state_is_refused_rather_than_waited_on` |
| 4 | A redirect with `error=` leaves the request waiting. | The extension's `authorize()` rejects with `error_description` (else `error`). | `every_redirect_shape_raycast_uses_parses` |
### Shortcuts — what the port does not have yet

Create Shortcut, Manage Shortcuts and shortcuts in root search run end to end: the engine keeps
the list in `$XDG_DATA_HOME/compass/compass-shortcuts.json` (ADR-0017 decision 3; the first start
with no such file copies Vicinae's `shortcuts/shortcuts.json`, whose shape is the same), answers
`ListShortcuts`/`SaveShortcut`/`RemoveShortcut`/`OpenShortcut`/`ExpandShortcut` (IPC v13), ranks
shortcuts in root search by name and link, resolves the opener and the `default` icon, and counts
visits. What differs:

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | Arguments are completion fields beside the search text of the selected root row. | A form with one field per argument opens when the shortcut is launched; required unless it has a `default=`. | `a_shortcut_in_root_search_asks_for_its_argument_then_opens` |
| 2 | An argument left empty expands to nothing, even with a `default=` — `expandShortcut` never reads the default. | It expands to its default. | `arguments_fill_their_placeholders_in_order` |
| 3 | `{date}` is reserved (so not an argument) and then falls into the expansion's argument branch, eating the next argument's value. | Expands to nothing; the arguments stay aligned with their placeholders. | `reserved_placeholders_take_their_values` |
| 4 | `{selection}`/`{selected}` read the focused application's selection. | The same: the primary selection, read as `getSelectedText` reads it (data-control on wlroots, the Shell extension on GNOME), and nothing when there is none. `{clipboard}` is read through the GNOME Shell extension, and is empty without it. | `on_sway_a_shortcut_expands_the_selected_text` |
| 5 | Open with… lists the link's openers in a submenu, and opens the link expanded with the completer's argument values. | An app-selector view (`compass_ui::open_with_page`, IPC v18 `ListOpeners`/`OpenWith`) lists the openers of the stored link, the default first, and opens it expanded; arguments are not asked for first, so a placeholder argument expands empty. | `manage_shortcuts_shows_the_detail_pane_and_opens_with_a_chosen_application` |
| 6 | Manage Shortcuts shows a detail pane (application, times opened, last opened, created, the expanded link), the link re-expanded as the completer's values change. | The pane (`shortcuts_page::detail_fields`) follows the selection; the link is expanded with no arguments, Manage Shortcuts having no completer. | `manage_shortcuts_shows_the_detail_pane_and_opens_with_a_chosen_application`, `the_pane_lists_what_load_detail_lists_in_its_order` |
| 7 | The form's link field offers placeholder completions (Selected Text, Clipboard Text, Argument, UUID) and the app list updates to the link's default opener on blur; the default icon previews the favicon. | The field's help text names the placeholders; `default` app and icon are resolved by the engine when saving (favicon for `http*`, else the opener's icon, else the link glyph). | `the_default_icon_is_the_favicon_then_the_opener_then_the_link_glyph` |
| 8 | Root rows weigh shortcuts at `baseScoreWeight` 1.4, and a shortcut with one argument can be a fallback command that opens with the search text; its fallback panel adds Manage Fallback Actions. | Ranked like every other root item. A `shortcuts:<id>` entry in `fallbacks` whose link takes one argument is a fallback row, in the configured order, opening with the query; its panel is Open and Manage Fallback Actions, which opens Configure Fallback Commands. | `a_one_argument_shortcut_named_as_a_fallback_opens_with_the_query`, `configure_fallback_commands_moves_items_between_its_sections` |
| 9 | The migration from the pre-JSON SQLite `shortcut` table. | Not run: the one-shot import is from Vicinae's JSON file, which already holds a migrated list. | — |
| 10 | A removal toast ("Removed link") and success toasts after saving. | The list updates in place; failures show in the view. | `manage_shortcuts_filters_edits_and_removes` |
| 11 | Arguments, `{clipboard}` and `{selection}` go into the link as typed, so `?q={query}` with `a & b` ends the query at the `&`. | In a URL template (a scheme and `://` before any placeholder, not `file:`) each value is percent-encoded, all but RFC 3986's unreserved characters and `/`; a link that is all placeholder, a path or a command line is filled in as typed. | `values_in_a_url_template_are_percent_encoded_and_elsewhere_left_alone` |

### Snippets — what the port does not have yet

Create Snippet and Manage Snippets run end to end: the engine keeps snippets in
`$XDG_DATA_HOME/compass/compass-snippets.json` (the first start without one copies Vicinae's
`snippets/snippets.json`, which glaze writes in the same shape), answers
`ListSnippets`/`SaveSnippet`/`RemoveSnippet`/`ExpandSnippet`/`PasteSnippet` (IPC v13), validates
with the form's rules and the store's (a keyword belongs to one snippet), and expands with the
ported expander: `{clipboard}` through the Shell extension, `{uuid}`, `{date format=…}` in Qt's
syntax on the local clock (`jiff`), `{shell}` placeholders run concurrently under the 2 s limit,
arguments by name. What differs:

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | Typing a keyword anywhere expands the snippet: `vicinae-input-server` reads `/dev/input` (libudev, xkbcommon), injects through uinput and the clipboard, with undo on backspace, per-app limits and the extension's delay/layout preferences. | **Ported** (`compass-input-server`, `compass::snippet_expansion`); the differences are in "Input server and keyword expansion" below. | `compass-input-server` tests, `the_input_server_is_told_the_keywords_and_follows_the_setting` |
| 2 | Arguments are completion fields beside the search text. | A form with one field per argument (named once, in order of first use); an empty optional one takes its default. | `manage_snippets_copies_asking_for_arguments_first` |
| 3 | Copy to clipboard copies text as transient (not recorded in history), and a file snippet as the file. | The launcher writes the expanded text to the clipboard itself; a file snippet copies its path as text. No form creates file snippets (the C++ form does not either). | — |
| 4 | — | Paste, which the C++ list does not offer: the expansion is put on the clipboard and pasted through the Shell extension, as clipboard history pastes. | `snippets_are_imported_created_expanded_edited_and_removed` |
| 5 | The form edits the keyword's application list, and offers placeholder completions in the content field. | The list is kept as it was (a duplicate keeps it too); the content field's help text names the placeholders. | `editing_a_snippet_keeps_its_apps_and_returns_to_the_list` |
| 6 | A detail pane shows the type, the dates, the keyword and its apps, and the expansion as arguments are typed (shell placeholders shown as `$(code)`). | The pane since "The gaps pass, UI" (IPC v19 `PreviewSnippet`): the type, the dates, the keyword, its applications by name, and the text expanded with its shell placeholders shown as `$(code)`. Manage Snippets has no completer, so arguments expand empty (to their defaults); the applications are names rather than icons. | `manage_snippets_shows_the_selected_snippets_detail_pane`, `the_pane_lists_what_load_detail_lists_in_its_order`, `a_preview_shows_a_shell_placeholder_instead_of_running_it`, `snippets_are_imported_created_expanded_edited_and_removed` |
| 7 | `parseSnippetText` takes `\` as an escape for a literal `{`. | The same since "The gaps pass, UI" (`compass_core::placeholder::parse_snippet_text`), for copying, pasting, the form's arguments and keyword expansion; a quicklink's link keeps the quicklink parser, which has none, as `Shortcut::parseLink` does. | `an_escaped_brace_is_text_and_not_a_placeholder`, `a_doubled_backslash_is_one_and_the_brace_after_it_opens_a_placeholder`, `an_escaped_brace_expands_as_a_brace`, `an_escaped_brace_asks_for_no_argument` |
| 8 | `{argument}` with no `name=` is collected as an argument with an empty name. | Left out of the form; it expands to nothing either way. | `arguments_are_named_once_and_reserved_ids_are_not_arguments` |

### Input server and keyword expansion — what differs

`compass-input-server` (the C++ `vicinae-input-server`) is `crates/compass-input-server`: the same process split, permissions
(`cap_dac_override`, packaging/README.md "The input server") and wire as the C++ — figura's
JSON-RPC, byte for byte, in little-endian frames — so either engine can drive either helper. The
engine starts it when `input_server.enabled` (default on), restarts it with the C++ backoff,
registers every keyword on ready and diffs them on each save or removal, and carries out
`handleKeywordTrigger`/`handleUndo` (`compass::snippet_expansion`). IPC v15 adds
`InputServerStatus` and `SetInputServerEnabled` (`compass input-server status|enable|disable`), and
`compass doctor` has an `input-server` check. What differs:

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | Keyboards and pointers are the nodes libudev tags `ID_INPUT_KEYBOARD`/`ID_INPUT_MOUSE`; hot-plug is a udev monitor. | The same tests `input_id` applies (keys 1–31; relative or non-pen, non-touchpad absolute X/Y with a left button) on the capability bits `evdev` reads; hot-plug is an inotify watch on `/dev/input`, retried briefly while udev sets the node up. A device udev tags by hwdb override rather than by bits is not recognised. | `device::is_keyboard` via the uinput test |
| 2 | Triggers of equal length are ordered by an unstable sort. | Stable: the earlier registration wins a tie. The earlier Rust matcher said *first registered wins* regardless of length, which was wrong — the C++ sorts longest first; fixed. | `the_longest_trigger_wins_whatever_the_registration_order` |
| 3 | `setKeymap` with a layout xkbcommon cannot compile installs a null keymap. | Refused with an error reply; the old keymap stays. | `an_unknown_layout_is_refused_and_the_old_one_kept` |
| 4 | `setKeyDelay` with a negative value hands it to `usleep` as unsigned. | Clamped to 0. | `a_negative_key_delay_is_zero` |
| 5 | After the paste, the clipboard's last selection (every offer) is restored after 800 ms. | Its text is restored after 800 ms; an image or file list on the clipboard before the expansion is not put back. | — |
| 6 | Cursor walk-back and undo count UTF-16 units. | Characters (the expander's unit). The two agree outside astral characters (emoji), where the C++ walks too far. | `a_cursor_placeholder_walks_back_and_forgoes_undo` |
| 7 | The expansion is copied as a concealed selection, so history skips it. | Concealed on wlroots (data-control's marker type); the GNOME Shell extension's `SetClipboard` carries no marker, so there it depends on the extension. | — |
| 8 | The focused application comes from the window manager, nulled while Vicinae itself is focused without focus-handoff detection. | The focused window from the Shell extension (GNOME) or the foreign-toplevel list (wlroots), recognised in the app index by `WM_CLASS`/`app_id`. With neither, the app is unknown: keywords limited to apps do not expand, terminals paste with Ctrl+V. | `a_keyword_limited_to_apps_expands_only_in_them` |
| 9 | Focus changes reset the typed text and the undo. | The same, from the Shell extension's window signal or the toplevel list's changes; without either, nothing resets it. | — |
| 10 | The Snippets extension's preferences (`enabled`, `undo`, `layout`, `prePasteDelay`, `keyDelay`) apply when changed in settings. | Read from `providers.snippets.preferences` in `compass.json` on every trigger (layout and key delay are pushed to the helper when they change); there is no settings page to edit them yet. | `preferences_are_read_and_clamped` |
| 11 | Clipboard-history and extension paste inject Ctrl+V through the input server (`LinuxPasteService`). | On wlroots the same, through `injectPaste` when the helper runs with injection, else a `zwp_virtual_keyboard_v1` keyboard; GNOME pastes through the Shell extension. | `on_sway_the_input_server_presses_the_paste_when_it_runs`, `on_sway_a_paste_is_copied_and_pressed_into_the_focused_window` |
| 12 | Without a clipboard there is no case to handle: the C++ always has Qt's. | With neither the Shell extension nor data-control, a typed keyword is logged and not expanded. | — |
| 13 | — | Inside a Flatpak the helper is not started (no `/dev/input` or `/dev/uinput` there) and `doctor` says so; the C++ has no Flatpak. | `a_flatpak_is_told_keyword_expansion_cannot_work_there` |

### Script commands — what the port does not have yet

Script commands run end to end: the engine scans `compass/scripts` under the data home and each
data directory, after the `customDirs` in `providers.scripts.preferences`, lists them in root search
(title, package name, keywords; `scripts:<id>`), rescans whenever the launcher lists them, re-reads
a script before running it, and runs it in its mode (IPC v13 `ListScripts`, `RunScript`,
`ScriptOutput`, `StopScript`): `fullOutput` streams stdout and stderr with `FORCE_COLOR=1` to a view
that colours them with the ported tokenizer; `compact` and `inline` take the first stdout line
within 10 s, an inline line becoming the script's subtitle (kept in
`compass-script-metadata.json`); `silent` says its line in the launcher's HUD (a transient notification where there is none); `terminal` runs
in the terminal emulator with the header's options. What differs:

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | Every root is pushed on one stack, so the *last* directory is walked first and a packaged script shadows a custom one with the same id, although the preference promises the opposite. | Roots are walked in order, so a custom directory wins. | `the_scan_finds_scripts_ids_them_by_path_and_lets_custom_dirs_win`, `script_commands_are_scanned_searched_and_run_in_their_modes` |
| 2 | Arguments are completion fields beside the search text; confirmation is an alert. | One form carries both: a field per argument (text, password, dropdown), and the confirmation sentence in its title when the header asks for one. | `a_script_asks_for_its_arguments_or_its_confirmation` |
| 3 | The directories are watched (100 ms debounce) and rescanned every 15 minutes. | Rescanned at start and each time the launcher is summoned; no watcher. | — |
| 4 | `compact` and `inline` results are toasts; the window is reopened with the title as search text if it had closed. | The result shows in the root list's notice line; the window is not reopened. `silent`'s line goes to the launcher's HUD, or a transient notification where there is none, as the media commands' does. | `a_compact_script_says_its_first_line_and_a_silent_one_hides_the_launcher` |
| 5 | The full-output view's action panel runs the script again or kills it, and a toast counts the seconds. | The same two actions (Ctrl+R to run again), and the count is in the view's heading; Escape kills a running script, as leaving the view does. | `a_full_output_script_asks_for_its_argument_and_shows_its_output` |
| 6 | The root row's panel opens the script in the text editor and its folder in the file browser. | Run and Copy path only. | — |
| 7 | `refreshTime` (inline) is parsed and validated. | Parsed and validated, and not acted on — nor is it in the C++. | — |
| 8 | Links in full output are clickable. | Drawn as links; a click is logged, the launcher having no URL opener yet (as for extension views). | — |
| 9 | The script's icon (emoji, file, `https`). | Rows use the initial badge, like every root row without resolved art. | — |

### System: Run Terminal Program — what the port does not have yet

Run Terminal Program runs end to end: the engine lists every entry of every `PATH` directory
(`ListPrograms`, with the terminal's name and the command's `default-action` preference from
`providers.commands.entrypoints.run-program.preferences`, default `run-in-terminal` as the C++
declares), and runs a command line in the terminal emulator (held open or not) or directly
(`RunProgram`, refusing "Not a valid executable"). The view parses the typed text with the desktop
entry `Exec` parser, offers it as a command-line row when its first word is a program, lists the
fuzzy-matching programs (100 at most), and orders the actions as `compass-core::system_run` pins.
What differs:

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | The command takes an optional `command` argument in root search and runs it without opening the view. | The view always opens. | — |
| 2 | Browse Apps, Set Default Browser and Set Default Terminal are also in the system extension. | So they are here ("Gaps closed after the truth pass"). | `browse_apps_lists_filters_opens_and_copies`, `a_default_picker_lists_the_engines_candidates_and_sets_the_chosen_one` |
| 3 | Programs are scanned once per view in the background, with a loading state. | Scanned by the engine on each opening (a blocking task), the view showing "Looking for programs…" until then. | `run_terminal_program_lists_path_and_runs_directly_or_refuses` |
| 4 | Inside the Flatpak, `PATH` is the host's through the portal's environment. | The engine's own `PATH` (the sandbox's inside the Flatpak); runs start on the host through the session helper's `HostCommand`, the call `flatpak-spawn --host` makes, and count as started once the process is running. | — |

### dmenu — what the port does not have yet

`compass dmenu` runs end to end with the C++ CLI's options: it reads stdin, the engine keeps the
list under a token and pushes `WindowCommand::Dmenu(token)` to the resident window (IPC v13), which
fetches the list, shows it (non-empty lines, fuzzy filter keeping input order among equals, a path
shown by its name and folder, the `{count}` section heading, the placeholder and initial query), and
answers the choice: the entry, its index with `--format index`, or the search text when nothing
matches. Printing it exits 0; a dismissal (Escape, the window hiding, a newer list) exits 1 with
nothing printed, as the C++ does. What differs:

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | `--width`/`--height` resize the window for the list (`requestWindowSize`, the side not given keeping the configured one), and `--navigation-title` sets the title in the status bar. | The same: the card takes the asked size and the window is resized around it (`window::resize`, or a size change on a layer surface), and back when a list without a size replaces it; the title is the footer's left side. A width under 500 turns quick look and the footer off, as in the C++. | `a_dmenu_size_resizes_the_window_until_a_list_without_one` |
| 2 | Quick look previews a highlighted file (name, path, MIME type over the image, the first 10 KiB of a text file up to 2 MiB, or the file's icon); `--no-metadata` hides the metadata; `--no-footer` hides the status bar; with quick look off a path row shows its folder instead. | The same pane beside the list (`compass_ui::file_preview`), the same limits and flags, and a footer with the title, the primary action and the panel chord. The MIME type comes from the extension (`mime_guess`), not from shared-mime-info's content sniffing, and a file that is neither image nor text shows its type where the C++ draws its icon. | `quick_look_previews_a_selected_file_and_the_size_is_asked_for`, `a_path_shows_its_name_and_folder`, `a_text_file_shows_its_start_and_an_image_itself` |
| 3 | A path entry shows its file icon. | The initial badge, like every row without resolved art. | — |
| 4 | Without a running launcher the C++ server starts showing its own window. | Refused like `compass show` is, when no window is attached. | `dmenu_shows_stdin_in_the_attached_window_and_prints_the_choice` |

### Set Theme — what the port does not have yet

Set Theme runs end to end: the view lists the themes in the ported sections ("Current Theme", then
"Available Themes", fuzzy over name and description), previews a theme as soon as its row is
selected, and puts the configured one back when it is left, as `ThemeViewHost` does; Enter keeps
the selected theme through the engine (`SetTheme`, IPC v13), which writes it to `compass.json` as
`compass theme set` does. What differs:

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | The themes are TOML files found in the theme directories (`$XDG_DATA_HOME/compass/themes`, then each `$XDG_DATA_DIRS/compass/themes`, the first id winning), each with its own palette, icon and path, over the built-in Vicinae Inkwell and Sandstone. | Compass's curated themes (System, Catppuccin, Dracula, Nord, Gruvbox, Tokyo Night, Solarized), then the same theme files, read by `compass_core::theme_file` with the C++'s rules (`[meta]`'s three strings, `colors.<key>` references, `opacity`/`lighter`/`darker`, `inherits`, circular references refused) and resolved to the launcher's nine palette slots through the ported `deriveSemantic` steps and the two built-in bases, which are inheritance bases here rather than listed themes. Colours are hex only: an SVG colour name (`red`) is a diagnostic, where `QColor` accepts it. The engine and `compass theme set`/`list` read the same directories, so a file's id is a theme everywhere. The files are read when Set Theme opens rather than watched. | `a_theme_file_is_read_and_resolved_with_its_derivations`, `a_child_inherits_from_its_parent_and_bad_files_are_refused`, `the_first_directory_wins_and_the_bases_cannot_be_replaced`, `theme_files_are_offered_after_the_curated_themes`, `set_theme_keeps_the_theme_in_the_configuration` |
| 2 | The action panel opens the theme file in the text editor, and copies its id or path; rows show the palette's colour dots. | The same panel (`theme_picker::action_panel`: Set theme, Open theme file, Copy ID, Copy path), the file opened with its default application; a theme file's row shows its eight swatches. The curated themes have no file and no swatches. | `set_theme_keeps_the_chosen_theme` |
| 3 | Choosing a theme applies it to every window at once through the theme service. | This window applies it at once; another launcher process picks it up from the configuration when it next reads it. | `set_theme_keeps_the_theme_in_the_configuration` |

### Create Extension — what the port does not have yet

Create Extension runs end to end: the launcher's form has the C++ fields (author, title,
description, location, first command's title and description, command template), the engine
validates them with the ported rules ("Min. 3 chars", "Min. 16 chars", "Must exist" after `~`
expansion) and writes the ported boilerplate (`CreateExtension`, IPC v13), and a success page shows
the C++'s Markdown with the path and the `npm` steps; Enter opens the new folder. What differs:

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | Field errors show beside each field. | The engine's refusal names the fields in one sentence under the form. | `a_valid_form_writes_the_boilerplate_and_an_invalid_one_says_why` |
| 2 | The success view offers "Open in …" for every application that opens folders. | Enter opens the folder with the default one. | `create_extension_sends_the_form_and_shows_where_it_went` |
| 3 | The API dependency is pinned to the build's git tag. | Pinned to `v` + the crate version (`^0.1.0` today), through the same `api_dependency_version` rule. | `create_extension_writes_the_boilerplate_under_home` |

### Browse Fonts — what the port does not have yet

Browse Fonts runs end to end: the engine reads the installed families once (warmed five seconds
after start, or on first use), folds the members of a typeface together and classifies each with
the ported `font_service` rules (`ListFonts`, IPC v13). The launcher lists them under the ported
heading ("All Fonts (n)", "<Category> (n)", "Results (n)"), with a category filter that offers only
categories some font has. Each row draws its glyph in its own font. Enter opens the ported specimen
(`FontSpecimen`), drawn in the family, and Escape returns to the list with its filter kept. The
panel offers "Preview font" and "Copy font family". What differs:

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | A family's scripts come from `QFontDatabase::writingSystems`, which on Linux is fontconfig's language coverage. | Read from the font's character map (`ttf-parser`), one or two sample characters per script (`font_service::SCRIPT_SAMPLES`), over the fonts `fontdb` finds on the fontconfig path. A font whose coverage claims and cmap disagree can land in a different category. | `a_font_file_is_found_and_classified_by_what_it_covers`, `browse_fonts_lists_families_and_previews_one` |
| 2 | A six-column grid of glyph tiles. | The same (`font_browser::COLUMNS`): each tile the glyph in the family over its name; arrows move along a row and between rows keeping the column (`fonts_page::grid_step`). | `the_grid_moves_by_tile_and_by_row`, `browse_fonts_is_a_grid_that_remembers_its_category_and_sets_the_font` |
| 3 | "Set as vicinae font" merges `font.normal.family` into `compass.json`, and the launcher redraws in it. | The same write (IPC v16 `SetFont`, keeping the rest of `font`), and this window switches at once. A configured family now wins over the desktop's interface font at start, which the launcher then stops following; `auto` and `system` mean the desktop's (the C++'s `auto` is its bundled Inter, which Compass does not ship). Markdown views (an extension's detail, the store's detail page and intros, the created-extension page) are drawn in it too, code still monospace; they used to fall back to Iced's generic sans-serif, which cosmic-text resolves to a hard-coded "Open Sans" and, where that is missing, through its fallback list. | `set_as_vicinae_font_writes_the_family_and_keeps_the_rest_of_font`, `set_theme_keeps_the_theme_in_the_configuration`, `markdown_is_drawn_in_the_launchers_font` |
| 4 | The chosen category is remembered across openings (`fontCategory` in the command's local storage), restored only when some font still has it. | Remembered across openings and restarts with the same restore rule (`index_for_saved`), in `$XDG_STATE_HOME/compass/compass-view-state.json` rather than the command's local storage: that is the engine's encrypted database, which needs the login keyring, and a filter is not a secret. | `browse_fonts_is_a_grid_that_remembers_its_category_and_sets_the_font`, `a_value_survives_a_new_process` |
| 5 | The specimen is Markdown rendered in the family. | The same Markdown read back line by line (heading, regular, bold, italic, rule) and drawn in the family; bold and italic ask the renderer for that face, which synthesises nothing when the family has none. | `a_specimen_reads_back_as_lines` |

### Rhai scripts — a Compass addition, with no C++ counterpart

Rhai scripts (PLAN §2.2, [RHAI-SCRIPTS.md](./RHAI-SCRIPTS.md)) are new in Compass, so nothing here
is a divergence from the C++ so much as a boundary of it. Their root entries use their own provider,
`rhai:script.<name>`, so frecency, aliases and favourites the Rust engine records for them are keys
the C++ engine has no item for and ignores. They are opened as extension view sessions over IPC
v14 (`ListRhaiScripts`, then the v8 `RunExtensionCommand` / `ExtensionView` / `ExtensionEvent`
requests); a v13 launcher does not list them. A script's `paste` on a wlroots compositor copies
and does not type, as an extension's paste does there ("wlroots" below). A script's root row draws
its manifest `icon` (a builtin icon's name) when that icon is installed, and its initial otherwise.
What a user allowed their own scripts is reviewed and revoked in the launcher's **Script
Permissions** command (IPC v16 `ListScriptGrants`, `RevokeScriptGrant`), which rewrites
`script-grants.json`, rebuilds the script without the grant and ends a view open on it, so the next
opening asks again (`script_permissions_are_listed_and_revoking_asks_again`,
`script_permissions_lists_what_was_allowed_and_revokes_it`).

### Extension Store and Raycast Store — what the port does not have yet

Both stores run end to end (IPC v14: `StoreBrowse`, `StoreExtension`, `StoreInstall`,
`StoreUninstall`, `OpenUrl`). The engine fetches with `ureq` on the blocking pool:
the Vicinae store's whole list (`/store/list?page=1&limit=500`, `postProcess` dropping other
platforms and renaming to `store.vicinae.<name>`), filtered locally as the user types with the C++
weights (title 1.0, author 0.5, description 0.3); the Raycast store's first page (cached for the
session, as `m_cachedPages`) or its server-side search after the ported 200 ms pause, with the
Linux compatibility sheet from `/raycast/get-compat` fetched once (a failure is an empty sheet and
is retried next time, as the C++). Rows carry the ported download count (`1.1K`), whether the
extension is installed, and on Linux its compatibility tier; the detail page carries the ported
banner ("This extension works but has a few quirks." and the sheet's notes), the metadata, the
command list, the README, and the Raycast screenshots. Install downloads the bundle, unpacks it
through `compass_core::store_bundle` in the ported staging order, and the engine and the launcher
both rescan the extension directories, so the new commands are in root search at once; uninstall
removes the extension, its support directory and its local-storage and preference namespaces, and
root search forgets it. What differs:

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | `Unzipper` extracts whatever the entry names say. | An entry that leaves its directory (`../`, an absolute path, a drive prefix) or is a symbolic link refuses the whole archive before anything is written; the download (128 MiB), the entry count (20,000) and the unpacked total (512 MiB, counted as bytes are inflated, not taken from the headers) are capped; every entry is read to its end so its CRC-32 is checked. | `store_bundle::tests`, `the_vicinae_store_lists_installs_into_root_search_and_uninstalls` |
| 2 | Install checks only that `package.json` exists. | It must also parse as an extension manifest, and the id built from the store's name must be one ordinary directory name (`store.vicinae../x` is refused). | `ids_that_would_leave_the_directory_are_refused` |
| 3 | No update detection. | An install leaves `.compass-store.json` beside the manifest with the store's version key (the Vicinae store's `checksum`, the Raycast store's `commit_sha`); a row whose store key differs says "Update available", and the detail page offers "Update extension" (a reinstall) first. An extension installed by the C++ engine, by hand or by Suite 1's harness has no marker and is never called out of date: the bundles' own timestamps land seconds before the store's publication time, so guessing from file times would flag every fresh install. | `only_a_marked_install_with_a_different_build_is_out_of_date`, `the_raycast_store_badges_compatibility_and_notices_an_update` |
| 4 | "Verify" is not attempted. | Nor is it possible beyond the CRC: neither store publishes a signature, and the Vicinae store's `checksum` matched no hash of the archive or of its `package.json` (SHA-256 and MD5 tried on a live bundle), so it is used only as a version key. | — |
| 5 | The detail page links the README (`readmeUrl`); the Vicinae store shows no screenshots. | The README is fetched (a GitHub `tree/`/`blob/` page is rewritten to its `raw.githubusercontent.com` text, 512 KiB at most) and rendered below the details in the launcher's Markdown view; a failed fetch leaves it out. Its relative image links, Markdown (found with `pulldown-cmark`) and `<img src>`, are made absolute against the README's URL, `<img>` tags become Markdown images, and the images are fetched through the remote-image cache and drawn in place. Raycast screenshots are drawn below. | `a_github_readme_page_is_fetched_as_raw_text`, `a_readme_s_relative_images_are_made_absolute_and_html_ones_drawable` |
| 6 | Rows show an author avatar, a download count, an installed check and a coloured compatibility dot. | The same at the row's right: "Installed" or "Update available" and "↓ 1.1K" as text, the tier as a coloured dot (green, orange, red, grey) with its name, and the author's avatar (IPC v16 `StoreEntry.author_avatar`) once fetched; square rather than round, as the renderer does not clip an image to a circle. | `the_accessory_says_installed_or_out_of_date_and_the_tier`, `a_deeplink_opens_the_detail_page_and_uninstalling_asks_in_a_dialog` |
| 7 | The first opening shows an intro page (`alwaysShowIntro`, `introCompleted` in command storage). | No intro: the store opens straight to its list. | — |
| 8 | "Uninstall Extension" is on every row's panel, and fails for one that is not installed. | Offered only on an installed row. The confirmation is the C++'s alert ("Are you sure?" and its message) as a dialog over the page with Cancel and Uninstall buttons, also answered with Enter or Escape. | `the_extension_store_installs_into_root_search_and_uninstalls_after_asking`, `a_deeplink_opens_the_detail_page_and_uninstalling_asks_in_a_dialog` |
| 9 | A failed list fetch shows a toast and leaves the spinner running (`FAILED_FETCH_CLEARS_LOADING`). | The failure is said under the list (or in place of it, when nothing has loaded), and loading stops. | `a_failure_after_rows_keeps_them` |
| 10 | The list is fetched with `PreferCache` and reused while Qt's disk cache keeps it. | The Vicinae list is kept in memory for ten minutes; the Raycast pages for the session, as the C++. | — |
| 11 | The Raycast API is always `backend.raycast.com`. | `COMPASS_RAYCAST_API_URL` overrides it, as `COMPASS_API_URL` already overrides the Vicinae API, so tests serve both stores locally. | `raycast_store::api_base_url` |
| 12 | Only the store builtins' links open (`openTarget`). | `OpenUrl` opens any `http(s)` link with the default browser (anything else is refused), and the launcher now uses it for links clicked in Markdown, including an extension view's, which were only logged before. | `only_web_urls_are_opened` |
| 13 | Deep links (`vicinae://extensions/<author>/<name>` into a detail host; `raycast://` and `com.raycast:` into the Raycast store's) exist, and a link with the wrong number of segments answers the usage sentence. | The same: `compass deeplink <url>` (or a bare `compass <url>`) sends IPC v16 `OpenDeeplink`, the engine pushes `WindowCommand::Deeplink` to the window, which opens the detail page; Escape goes to that store's list rather than the root. A third segment, Raycast's `raycast://extensions/<owner>/<name>/<command>` (what `createDeeplink` makes), runs the installed command it names, found by owner or author, and opens the store page when it is not installed. | `an_extensions_link_names_the_store_owner_and_extension`, `a_command_link_finds_the_installed_command_by_owner_or_author`, `an_extensions_deeplink_goes_to_the_window_and_a_malformed_one_is_refused`, `a_deeplink_opens_the_detail_page_and_uninstalling_asks_in_a_dialog` |
| 14 | A Raycast extension's detail and install URLs are built from `author.handle`, so an organisation's extension (GitHub, Linear and Todoist are filed under `raycast`, `linear` and `doist`) is a 404 and cannot be opened or installed. | They are built from the listing's `owner.handle`, falling back to the author's; the row carries both (IPC v23 `StoreEntry.owner`) and shows the author. The installed id (`store.raycast.<name>`) has no handle in it, so existing installs are unchanged. The runtime's `ownerOrAuthorName` is the manifest's `owner` when it has one, as Raycast's is. | `an_organisations_extension_is_addressed_by_its_owner_not_its_author`, `an_organisations_raycast_extension_opens_and_installs_by_its_owner` |

### `compass-crypto` — one error variant the C++ API cannot express

Not a behavioural divergence; a faithful reproduction of an awkward C++ signature, recorded so the
next person does not "fix" it on one side only.

`Crypto::AES256GCM::EncryptError` has exactly one value, `CipherError`, so C++ `encrypt` reports a
wrong-sized key, an unavailable CSPRNG and a refusing cipher identically. `decrypt`'s error set, by
contrast, *does* distinguish `InvalidKeySize`. `compass-crypto` mirrors both, including the
asymmetry, because the parity harness compares error names and a more precise Rust variant would
read as a divergence rather than as the improvement it is. Fixing it means changing both sides in
the same commit.

The reverse case is also worth naming: C++ `DecryptError` declares a `CipherError` variant that
**no code path produces** — `gcmDecrypt` failure maps to `AuthFailed`. So the reachable C++ set is
`{InvalidKeySize, DataTooShort, AuthFailed}`, which is exactly the Rust set. `aes-gcm` collapses
every decryption failure into one opaque error by design, and that is the right call: telling
"the cipher broke" apart from "the tag did not verify" is a padding-oracle-shaped invitation.

### `compass-search` — nucleo is not fzf

`nucleo-matcher` uses a different algorithm from the C++ fzf port, so absolute scores are on another
scale and are never asserted. The normalized 0-100 `score`/`quality` values match the C++
expectations closely: every `score == 100`, `score == 50` and `quality == 100` assertion ports
verbatim and passes. What does not:

| # | Divergence | Impact | Pinned by |
|---|---|---|---|
| 1 | ~~No coherence signal~~ — **resolved.** The original claim here was wrong: the C++ backtracker only ever compares `B[j]` against *zero*, never uses its magnitude, and uses `H`/`C` solely to choose the alignment. So `coherent = !boundary_inside \|\| !mid_word_run_start` is a pure function of (haystack, indices), which is exactly what nucleo returns. It is an **exact port**, not a heuristic — no thresholds exist to tune. | None | the ported coherence suite |
| 2 | **One ordering flip** (upstream issue #946). For `"Spo"`, C++ gives `Spotify > Reload Script Directories > Sysprog`; we give `Spotify > Sysprog > Reload Script Directories`. nucleo prefers a short scatter inside one word starting at position 0; fzf's larger word-boundary bonuses pull the other way. Every other ordering case ports and passes. | Low | `diverges_spo_ordering` |
| 3 | ~~**Narrower diacritic folding.**~~ **Closed.** nucleo folds precomposed accents (é, ñ, ü) but not stroked letters (Ł, đ, ħ); the matcher now falls back to `deunicode` for single-letter Latin answers, so `"lodz"` matches `"Łódź Express"`. Other scripts are not romanised (`"a"` does not match `"α"`). | None | `latin_extended_a_is_folded` |
| 4 | **Ties that discriminate nothing.** For `"clip"`, all of `Clipboard History`, `Clear Current Clipboard Data` and `Clear Clipboard History` score identically, because nucleo's score depends only on the matched region, not on haystack length or match position. The C++ ordering test passes there only because `stable_sort` preserves input order — so that case discriminates nothing in *either* implementation. | Real discrimination needs a length or match-position penalty layered on top of nucleo | `diverges_clip_ordering_is_a_three_way_tie` |
| 5 | **Char, not byte, offsets** — a deliberate API change. `"Café Bar"`/`"bar"` reports `5..8` where C++ asserts bytes `6..9`. | None; byte offsets are recoverable | the range tests |

### How closely is "closely"? 79.2%

The 2026-09-20 end-to-end audit exposed a separate nucleo 0.3.1 defect, not an
intended fzf divergence: its single-character Unicode path updates the previous
character class only when a character matches, losing intervening boundaries.
The adapter now evaluates matching positions through nucleo's public postfix
scorer, retaining the actual preceding character, earliest ties and char indices.
It remains linear and does not duplicate scoring constants or replace nucleo.
The reproducer failed at 26 versus 36 before the fix; ASCII/Unicode metamorphic
tests, Cyrillic/CJK cases and six harvested-title regressions cover the correction.
The six changed corpus pairs are A/a against Animation Editor, E against all
three Bear Factory editors, and I against Spritedesc interpreter. Their normalized
quality now agrees with C++; the last pair was previously rejected.

The table above was written from a ported ordering suite over hand-written cases, which could say
*that* nucleo and fzf differ but not *how much*. `compass-testkit`'s `scorer-parity` bin now
measures it directly, against the real C++ scorer compiled from `src/lib/fuzzy` — that library is
header-only with no Qt dependency, so it costs one translation unit.

Over 738 harvested entries and 1685 queries derived from them:

| | |
|---|---|
| identical | 1334 (79.2%) |
| divergent queries | 351 |
| divergent (query, entry) pairs | 1411 |

| shape | count |
|---|---|
| C++ rejected, Rust accepted | 771 |
| both accepted, C++ higher | 435 |
| both accepted, **Rust** higher | 145 |
| **Rust** rejected, C++ accepted | 60 |

Both directions occur, which is what two different algorithms produce and what a smaller corpus
hid: over the previous 115-entry set only six queries diverged and every one had C++ stricter.

The divergence is entirely in the **raw matcher** — both `score_query` implementations normalise
identically. Matches at index 0 and fully contiguous matches agree exactly; `a3`/`3` scores 30
against 26 (divergence #4, position is not scored), and `System`/`Se` scores 29 against 47
(divergence #2, the word-boundary bonus).

**But rank agreement is near-total.** Score equality is not what a user sees; over the same 1685
queries the two engines pick the **same top result 100% of the time** — including all 920 queries
that return more than one hit, so it is not the trivial single-candidate kind — the same top 3 in
97.2%, and the same full order in 84.4%. The scoring difference is real and almost entirely
absorbed by the ranking. `scorer-parity` therefore *asserts* top-1 agreement exactly, rather than
ratcheting it.

`scorer-parity` pins the score totals as a ratchet: it fails if they get worse **and** if they get
better, so a nucleo bump cannot move what users see without someone looking at it. Driving them to
zero would mean replacing nucleo, which PLAN §10 settles the other way.

### The residual coherence gap, one level up

Coherence itself is now exact, but it is a property *of an alignment*, and nucleo's DP does not
always pick the same alignment fzf does. Alignments differ in about 10.6% of cases; validated
against a C++ oracle, end-to-end agreement on the `coherent` flag is **99.57%** (3032/3045), and
**100% across the entire ported corpus and every `main.cpp` ranking case**. Every disagreement is an
alignment difference, e.g. `"Settings System"/"stem"`: C++ aligns `S(0) t(2) e(13) m(14)` and calls
it incoherent, nucleo aligns `Sys[tem]` and calls it coherent — arguably the better read.

Closing that would mean replacing nucleo's DP, not improving the classifier. Since it has no
observable instance in the corpus, it is pinned by `diverges_coherence_depends_on_nucleos_alignment`
(which also freezes nucleo's alignment, so a future nucleo bump surfaces the change) rather than
carried as a behavioural divergence.

### A recall cost that arrives with parity

Inherited from the C++ rule, not introduced by the port, and worth knowing before anyone reports it
as a regression: cross-word abbreviations that pick up a mid-word letter are rejected.
`"Firefox Web Browser"/"ffb"`, `"Text Editor"/"txted"` and `"Power Statistics"/"pwrstat"` are all
rejected — and the oracle confirms the C++ rejects all three too. The rule is working as designed;
whether it is the *right* design is a separate product question.

### wlroots (Phase 5 Track B) — Sway, Hyprland, niri, labwc, river

Verified on headless Sway 1.9 (`.github/workflows/wlroots.yaml`); Hyprland and niri are expected
to behave the same because every choice below is made from the advertised globals, but neither runs
in CI. Their IPC providers are tested against fake sockets replaying captured replies
(`compass-platform-linux/tests/compositor_ipc.rs`), and against headless Sway with a fake Hyprland
socket (`on_sway_with_a_hyprland_socket_windows_learn_their_pid_and_workspace`).

1. **Launcher surface.** A layer surface through `iced_layershell`, centred, `top` layer,
   `exclusive` keyboard, namespace `vicinae` — the C++ `LayerShellConfig` defaults. The C++ keys
   that change them (`launcherWindow.layerShell.enabled`/`.layer`/`.keyboardInteractivity`) are
   **not ported**; `COMPASS_LAYER_SHELL=0` stands in for `enabled = false`. The C++ drops
   exclusive focus while a file chooser opened from the launcher is up; the Rust launcher has no
   such flow yet.
2. **Which sessions get it.** The C++ asks only whether the compositor advertises the layer shell
   (`Environment::isLayerShellSupported`). The Rust engine decides **GNOME by
   `$XDG_CURRENT_DESKTOP` first** and only then looks at globals, so a future Mutter with a layer
   shell stays on the tested GNOME path. Same outcome on every compositor today.
3. **Window switching.** The C++ has per-compositor providers (Hyprland and niri over their IPC,
   with workspaces) ahead of a generic Wayland one. The Rust engine switches windows on the
   generic path: `zwlr_foreign_toplevel_manager_v1` (list, focus state, activate, close) or,
   failing that, `ext_foreign_toplevel_list_v1` (list only; activate/close are refused by name).
   The **Hyprland and niri providers are ported** (`compass_platform_linux::compositor`, chosen
   from `HYPRLAND_INSTANCE_SIGNATURE` and `$NIRI_SOCKET` as the C++ chooses them), and on those two
   each toplevel is given the pid and workspace number the compositor reports for the window of
   the same class and title — the toplevel protocols carry neither, and the two numberings share
   no id. Two windows of one application with one title are paired by order. Elsewhere on wlroots:
   no workspaces and no pid (the launcher's own window is recognised by `app_id`). `WindowManagement`
   uses the providers directly ("The extension host API" #4). Differences in the providers
   themselves: they ask when asked rather than mirroring niri's event stream (same answers, no
   thread); Hyprland dispatches the C++'s Lua form (`hl.dsp.focus({ window = … })`) and, when a
   Hyprland older than the Lua dispatchers refuses it, the classic `focuswindow address:…`; niri's
   replies are read with `niri-ipc` 26.4's types, so a niri older than 25.08 (no `layout` or
   `focus_timestamp`) is unreadable and counts as no windows. Order is most-recently-activated
   first, with the focused window last, as on GNOME. `compass doctor` reports which of the
   protocols this track uses are advertised (`wlroots.capabilities`: layer-shell,
   foreign-toplevel, data-control, xx-hotkey, the portal's GlobalShortcuts, compositor IPC).
4. **Clipboard history.** Watched over `ext-data-control-v1`, else `zwlr_data_control_manager_v1`,
   with the C++ offer filter (`compass_wayland::data_control`). The C++ stores every kept type of
   a selection; the Rust store takes one per selection, so the **preferred** one is recorded
   (image, then `text/uri-list`, UTF-8 text, plain text, HTML). A selection carrying
   `x-kde-passwordManagerHint` or `vicinae/concealed` is **not recorded at all**. The primary
   selection is not recorded. The source application is unknown (data-control does not say).
5. **Paste.** The C++ injects Ctrl+V through its uinput input server. The Rust engine does too
   when the helper runs with injection, and otherwise presses the chord on a
   `zwp_virtual_keyboard_v1` keyboard (`compass::paste`, "The gaps pass, wlroots paste and
   inhibit"); with neither, the content is copied and the launcher told so. Copy, read and clear
   work, over `wl-clipboard-rs`; an HTML copy keeps its plain-text alternative, which the GNOME
   path cannot.
6. **Global hotkey.** The C++ tries `xx-hotkey-v1` and then `vicinae-hotkey-v1`. The Rust engine
   tries `xx-hotkey-v1` (fixed `Super+Space`), then the GlobalShortcuts portal, and otherwise logs
   how to bind `compass toggle` in the running compositor's config. `vicinae-hotkey-v1` is not
   ported, and the trigger's input serial is not yet passed to `xdg-activation`. No released
   compositor carries `xx-hotkey-v1`, so the manual binding is what users have today.
7. **Flatpak.** Nothing beyond `--socket=wayland` is needed, and nothing can add more: a compositor
   that honours `wp_security_context_v1` may hide data-control and foreign-toplevel from a
   sandboxed client, and the features above then degrade as if the compositor lacked them.
