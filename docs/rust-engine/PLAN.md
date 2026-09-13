# Compass Rust Engine — Transformation Plan

**Status:** proposal, not yet approved for implementation
**First target:** **GNOME 50 / 51 on [Bluefin](https://projectbluefin.io/)** — everything below is
sequenced around that, with other compositors as later phases.
**Scope:** replace the C++23/Qt6 core of this repository with a Rust workspace, seeded from
[MystikoLab/rustcast](https://github.com/MystikoLab/rustcast), following the Rust engine
specification in [this gist](https://gist.github.com/hanthor/ba051ebc406ddb4b8f958601378a5234)
(the spec is in the gist **comment**; the gist body is the earlier C++/Qt6 variant and is treated
here as reference).

Companion document: [`REFERENCES.md`](./REFERENCES.md) — prior art, verified crate versions, and
the protocol-support evidence behind §3.

---

## 0. Decisions taken

| Decision | Choice | Reversibility |
|---|---|---|
| Direction | Full Rust rewrite of the core, seeded from rustcast | Hard — this is the plan |
| Host repo | `tuna-os/compass`, Rust workspace alongside the C++ tree | Easy |
| **First target** | **GNOME 50/51 on Bluefin, shipped as a Flatpak** | Hard — reshapes phases 1–3 |
| UI toolkit | **Iced 0.14** (rustcast already ships a working Iced launcher) over Slint | Medium — contained in `compass-ui` |
| Surface strategy | plain `xdg_toplevel` first (**GNOME has no layer-shell**); `wlr-layer-shell` added in Phase 5 | Easy |
| Extension runtime | Keep `src/typescript/` (Raycast-compat SDK) **unchanged**; only its host is rewritten | Easy |
| Third extension tier | **Rhai** scripts in-process, behind the same capability layer as the TS host (§2.2) | Easy — drop it if the seam doesn't materialise |
| Crate prefix | `compass-*`, binary stays `vicinae` for CLI/config/socket compatibility | Trivial |
| Licence | Compass is GPL-3.0, rustcast is MIT; MIT → GPL-3.0 is one-way compatible, so rustcast code may be incorporated with its copyright header plus a provenance note | N/A |

Decisions are recorded as ADRs in [`adr/`](./adr/); §10 summarises them and lists what is still open.

---

## 1. Honest inventory

### 1.1 What compass is today

| Area | LOC | Notes |
|---|---|---|
| C++ (`.cpp/.hpp/.h`) | 144,534 | of which `src/lib/glyph` is 42,636 — mostly **generated** emoji/symbol datasets |
| QML | 14,660 | presentation only, per the `AGENTS.md` bridge pattern |
| TypeScript | 11,753 | `src/typescript/{api,extension-manager,raycast-api-compat}` |

Hand-written C++ to be replaced is ≈ **100k LOC**:

| Subsystem | LOC | Weight |
|---|---|---|
| `src/server/src/services/` (45 services) | 31,765 | heaviest; the actual product |
| `src/server/src/ui/` | 15,243 | view hosts feeding QML |
| `src/server/src/builtins/` (17 groups) | 11,974 | breadth, individually small |
| `src/server/src/extension/` | 6,585 | Node host + React reconciler bridge |
| `src/file-indexer/` | 5,646 | standalone daemon, already isolated |
| `src/lib/xdgpp/` | 4,235 | desktop entry / MIME / locale, **10 test files** |
| `src/lib/figura/` | 2,875 | in-tree IPC code generator |
| `src/cli/`, `src/lib/fuzzy/`, `src/data-control-server/`, `src/snippet/` | 5,450 | |
| `src/browser-extension/` | 250 | **out of scope** — becomes an extension, ADR-0008 |

Assets the rewrite inherits rather than invents: Wayland protocol XML
(`wlr-layer-shell`, `wlr-foreign-toplevel-management`, `xx-hotkey-v1`, `vicinae-hotkey-v1`);
window-manager backends for gnome/hyprland/kde/niri/wayland/x11/macos/windows; global-shortcut
backends including the `xx-hotkey-v1` one from
[#1936](https://github.com/tuna-os/compass/pull/1936); and Catch2/CTest suites under
`src/lib/{fuzzy,xdgpp,crypto,glyph,script-command}/tests`, `src/lib/vicinae-ipc/tests`,
`src/file-indexer/tests`, `src/snippet/tests`, all run by `make test`.

### 1.2 What rustcast gives us

**10,670 LOC of Rust, single crate, macOS-only, edition 2024.**

Reusable (≈3–4k LOC after de-macOS-ing): `app.rs` + `app/tile*` + `app/pages/*` (a working Iced 0.14
launcher shell), `styles.rs` (501), `config.rs` (407), `database.rs` + `migrations/`,
`unit_conversion.rs` (478), `debounce.rs`, `utils.rs`, `commands.rs` — plus dependency choices that
already match the spec (`tokio`, `nucleo-matcher`, `rusqlite`, `arboard`, `iced`).

Discarded: all of `src/platform/macos/` and every `objc2*` crate; `global-hotkey` (→ portal);
`icns`, `tray-icon`, `objc2-app-kit` screen handling; and the single-crate layout.

**Calibration:** rustcast has 5 pages; compass has 17 builtin groups and 45 services. rustcast covers
roughly **5% of compass's functional surface**. It is a seed and a proof that the Iced shell works —
not a base to bolt features onto. Schedule accordingly.

---

## 2. Target architecture

```
                        ┌──────────────────────┐
                        │  bin/vicinae (CLI)   │  ext install/remove/list, doctor, toggle
                        └───────────┬──────────┘
     ┌──────────────┬───────────────┼───────────────┬──────────────────┐
     ▼              ▼               ▼               ▼                  ▼
┌──────────┐  ┌───────────┐  ┌────────────┐  ┌─────────────┐  ┌────────────────┐
│compass-ui│  │compass-   │  │compass-    │  │compass-     │  │compass-worker- │
│  (Iced)  │  │  core     │  │  search    │  │  ipc        │  │      host      │
└────┬─────┘  │(state,    │  │(nucleo)    │  │(UDS, framed)│  │(spawn+sandbox) │
     │        │ registry, │  └────────────┘  └─────────────┘  └───────┬────────┘
     │        │ dispatch) │                                            │ JSON-RPC 2.0
     │        └─────┬─────┘                                            ▼
     ▼              ▼                                        ┌──────────────────┐
┌──────────────┬──────────────────┬───────────────┬──────────│ vicinae-worker-ts│
│compass-shell │ compass-portals  │compass-wayland│ compass- │ (Node, unchanged │
│ (GNOME Shell │ (ashpd: hotkeys, │(xdg_toplevel, │ platform │  @raycast/api)   │
│  DBus, zbus) │  OpenURI, files) │ activation,   │ (fs,exec,└──────────────────┘
│              │                  │ layer-shell*) │  icons)
└──────────────┴──────────────────┴───────────────┴──────────┘
                                     * Phase 5, wlroots only
```

| Crate | Owns | Ported from |
|---|---|---|
| `compass-core` | state machine, config, root-item registry, dispatch, frecency, SQLite | C++ `server/src/{root-search,config,command,services}` + rustcast `database.rs`, `config.rs` |
| `compass-search` | fuzzy index + matcher, `FuzzySearchable`-equivalent trait | C++ `lib/fuzzy` semantics on `nucleo` |
| `compass-shell` | **GNOME Shell DBus client** (windows, clipboard, paste) via `zbus` | C++ `services/{window-manager,clipboard}/gnome` |
| `compass-wayland` | `xdg_toplevel` surface, `xdg-activation-v1`, `keyboard-shortcuts-inhibit`; later `wlr-layer-shell` + `ext-foreign-toplevel-list-v1` | C++ `internal/wayland`, `ui/windows`, `services/window-manager/*` |
| `compass-portals` | `ashpd`: GlobalShortcuts, OpenURI, FileChooser, Screenshot, Secret | C++ `services/{global-shortcuts,file-chooser,permissions}` |
| `compass-ipc` | UDS at `$XDG_RUNTIME_DIR/vicinae/ipc.sock`, length-prefixed frames | C++ `lib/vicinae-ipc`, `lib/figura` |
| `compass-xdg` | desktop entries, MIME, icon theme, locale | C++ `lib/xdgpp` |
| `compass-extension-api` | **front-end-agnostic seam**: capability registry, view tree, action dispatch. Knows nothing about Node or Rhai | new — see §2.2 |
| `compass-worker-host` | Node worker lifecycle, Landlock + seccomp, cgroups v2, state dirs | C++ `server/src/extension/node-runtime` |
| `compass-script` | in-process [Rhai](https://rhai.rs) host: engine per script, capability-gated registration, operation budget | new — see §2.2 |
| `compass-ui` | Iced views, theming, tiles, pages | rustcast `app/*`, `styles.rs` |
| `compass-platform` | process exec (incl. `flatpak-spawn`), file indexing, clipboard | C++ `file-indexer`, `services/{clipboard,paste}` |

### 2.1 Three deliberate departures from the gist spec

(A fourth, the Rhai tier, is additive rather than a departure and is described in §2.2.)

1. **postcard, not Cap'n Proto.** The spec wants Cap'n Proto zero-copy on the core socket. We already
   have a working framed protocol and an in-tree generator (`figura`); a second wire format buys
   latency we have not measured a need for. Start with `serde` + length-prefixed
   [postcard](https://crates.io/crates/postcard) frames, keep JSON-RPC on the *worker* socket for
   `@raycast/api` compatibility, and adopt Cap'n Proto only if Phase-4 benchmarks miss the 0.5 ms
   SLA. Recorded as ADR-0002 — reversible, not rejected.
2. **Internationalisation, which the spec omits entirely.** Compass has a live Qt Linguist catalogue
   (`src/server/translations/*.ts`) and strict i18n rules in `AGENTS.md`. Proposal: `fluent-rs` plus
   a build-time extractor and a one-off `.ts` → `.ftl` converter so existing translations survive.
   Dropping the catalogue is a regression users notice immediately. ADR-0003.
3. **The GNOME Shell extension stays.** See §3 — on GNOME 50/51 there is no protocol alternative for
   window management, clipboard history, or paste. The spec's headline goal ("eliminate reliance on
   GNOME Shell private APIs") is not achievable on our first target, and pretending otherwise would
   design us into a corner. The realistic goal is restated in §3.5.

### 2.2 A third extension tier: Rhai scripts

Compass has two extensibility tiers today and they leave a gap in the middle:

| Tier | Power | Cost to the author | Cost to us |
|---|---|---|---|
| Raycast TS/React extensions | full | Node, npm, a bundler, React | a sandboxed worker process, ~256 MB ceiling, tens of ms to spawn |
| Script commands (shell, python, …) | one-shot output | trivial | arbitrary process execution, no sandbox, no interactive view |
| **← the gap →** | **interactive, stateful, sandboxed, no runtime dependency** | | |

[Rhai](https://rhai.rs) fills it. A forty-line `.rhai` file dropped in a folder gets a filterable list
view with actions, at the cost of parsing an AST (microseconds) rather than spawning Node. On an
immutable Flatpak target where Node has to be bundled, that matters for our own SLAs in §8.5.

**Rhai's sandbox is stronger than the Node one, and for a structural reason.** Rhai's standard
library is pure computation — no filesystem, no network, no process. The host registers every
capability a script can reach, so a script that did not declare `net` cannot make an HTTP call
because the function does not exist in its scope. That is capability-based security by
construction, versus the Node worker where we start from full access and subtract with Landlock and
seccomp. Verified limit APIs: `set_max_operations`, `set_max_call_levels`, `set_max_string_size`,
`set_max_array_size`, `set_max_expr_depths`, `set_max_modules`, and `on_progress` for
budget-exhaustion termination.

**One sharp edge:** `Engine::new` installs `FileModuleResolver` by default, so `import` reads
`.rhai` files off disk. Use `Engine::new_raw()` with an explicit package, or
`DummyModuleResolver`, or a resolver scoped to the script's own bundle. This must be a test, not a
code review note.

Sketch of the shape, illustrative and not settled:

```rhai
fn metadata() {
    #{ title: "Jira Issues", icon: "jira", mode: "list", capabilities: ["net"] }
}

fn search(query) {
    http::get_json(`https://example.invalid/search?q=${query}`).map(|i| #{
        title: i.summary,
        subtitle: i.key,
        actions: [ #{ title: "Open", run: || shell::open(i.url) } ],
    })
}
```

**The architectural consequence, and the reason this is written down now rather than in Phase 6.**
A third extension tier is only affordable if it is a second *front-end* onto one capability layer,
not a parallel stack. Otherwise every new host capability — clipboard read, window list, storage,
OAuth — has to be exposed three times and will drift. So `compass-extension-api` is carved out as
a seam in **Phase 4**, when we are designing the TS host's view protocol anyway, and Rhai becomes a
consumer of it in Phase 5. Getting that seam wrong is what makes this expensive; getting it right
makes Rhai mostly a binding exercise.

Two honest counterpoints, recorded so nobody is surprised later:

- **The ecosystem is zero.** The Raycast store is why people choose Vicinae. Nobody has written a
  Rhai launcher extension, and we would have to seed the tier with first-party examples and real
  docs. This is a product bet, not a technical one — see §10.9.
- **Rhai is synchronous and in-process**, so a script can hang the UI. Every script runs on
  `tokio::task::spawn_blocking`, never the render thread, with an operation budget and a wall-clock
  timeout. Host functions that do I/O block from the script thread into the runtime, which bounds
  how many can be in flight.

Alternatives considered: `mlua` (Lua — bigger ecosystem, but a C dependency and a weaker sandbox
story), `wasmtime` + WASI (strongest isolation and any source language, but a heavy lift and a poor
fit for forty-line scripts), and `rquickjs` / `boa_engine` (JS in-process — tempting since authors
already know JS, but owning a second JS runtime with a *different* API surface from the TS tier is
worse than Rhai's honest separateness). Recorded as ADR-0005.

---

## 3. Target-platform reality: GNOME 50/51

This section is the most important in the document, because it contradicts the spec. Every claim
below was checked against primary sources — see [`REFERENCES.md`](./REFERENCES.md) §3 for the
receipts.

### 3.1 What Mutter actually implements

From `src/meson.build` on `mutter` main (51.rc), the full Wayland protocol list includes
`xdg-shell`, `xdg-activation-v1`, `xdg-dialog`, `xdg-toplevel-tag`, `xdg-session-management`,
`keyboard-shortcuts-inhibit-v1`, `text-input-v3`, `fractional-scale-v1`, `cursor-shape-v1`,
`idle-inhibit`, `pointer-*`, `viewporter`.

It does **not** include:

| Protocol the spec relies on | In Mutter 51? | Consequence for us |
|---|---|---|
| `wlr-layer-shell` | ❌ (mutter#973, open since 2019) | Launcher must be a plain `xdg_toplevel` |
| `ext-foreign-toplevel-list-v1` | ❌ | No protocol window list |
| `wlr-foreign-toplevel-management` | ❌ | No protocol activate/close |
| `wlr-data-control` / `ext-data-control` | ❌ | No protocol clipboard monitoring |
| `ext-workspace` | ❌ | No protocol workspace switching |

Note also that `ext-foreign-toplevel-list-v1` is **list-only** even where it exists — its only
requests are `stop()` and `destroy()`. The gist's "issues `activate` and `close` requests directly to
the compositor" is wrong on every compositor. Activation is `xdg-activation-v1`; close has no
portable equivalent.

### 3.2 `org.gnome.Shell.Introspect` is not available to us

The obvious escape hatch — GNOME's own `GetWindows` DBus API — is gated. `js/misc/introspect.js`
defines `APP_ALLOWLIST = ['org.freedesktop.impl.portal.desktop.gtk',
'org.freedesktop.impl.portal.desktop.gnome']` and `GetWindowsAsync` runs every caller through
`this._senderChecker.checkInvocation(invocation)`. Third-party apps get "GetWindows is not allowed".

### 3.3 Therefore: the Shell extension is load-bearing

Compass already depends on a GNOME Shell helper extension exposing
`org.gnome.Shell.Extensions.Windows` (`services/window-manager/gnome/`) and
`org.gnome.Shell.Extensions.Clipboard` (`services/clipboard/gnome/`). On GNOME 50/51 **that is the
only mechanism available** for window switching, clipboard history and synthetic paste. It is
ported to Rust as `compass-shell`, not deleted.

### 3.4 What *does* work natively on GNOME

- **Global hotkeys — the one spec item that lands.** `xdg-desktop-portal-gnome` added the
  GlobalShortcuts backend in **48.rc**, improved it in **49.beta**, fixed activation-token delivery
  in **50.alpha**, and lists "various fixes to the Global Shortcuts portal" in **51.rc**. So `ashpd`
  GlobalShortcuts is the correct and only hotkey path on our target. `xx-hotkey-v1` and the X11
  backend are for *other* compositors and move to Phase 5.
- **`xdg-activation-v1`** for raising/focusing our own window and the app we launch.
- **`keyboard-shortcuts-inhibit-v1`** for grabbing keys while the launcher has focus — compass
  already has a `shortcut-inhibit` service; keep it.
- **Portals** generally: OpenURI, FileChooser, Screenshot, Secret are well covered by
  `xdg-desktop-portal-gnome`.

### 3.5 Restated goal for the GNOME integration

Not "eliminate the extension" — that is not on offer. Instead:

1. **Nothing in the critical path may require it.** App search, launch, calculator, emoji, snippets,
   file search and extensions must all work with **zero extension installed**. Only window
   switching, clipboard history and paste degrade.
2. **Shrink the extension to a minimal, versioned DBus contract** — ideally three methods and one
   signal — so a GNOME release breaks a 200-line extension, not the launcher. GNOME 51 ships
   16 September 2026; treat each GNOME release as a scheduled extension-compat task.
3. **Version the contract explicitly** (`org.gnome.Shell.Extensions.Vicinae` with a `Version`
   property) and make `compass-shell` degrade gracefully on mismatch instead of hard-failing.
4. **`vicinae doctor` must diagnose this precisely** — extension present / absent / version-mismatch,
   and what specifically is degraded as a result.

### 3.6 Bluefin's constraints

Bluefin is an immutable, OCI-composed Fedora Atomic desktop with **no traditional package manager**;
apps arrive as Flatpaks (GUI) or Homebrew (CLI). That has three consequences the spec does not
anticipate:

- **Flatpak is not a Phase-6 packaging chore, it is the Phase-0 development target.** If the dev
  loop is `cargo run` on a mutable host for six months, we will build something that does not work
  where it ships. The Flatpak manifest lands in Phase 0 and CI builds it from Phase 1.
- **`/dev/uinput` is unavailable**, so the spec's `wtype`/`dotool` paste fallback is dead on this
  target. Paste goes through the Shell extension, full stop.
- **Installing the Shell extension from inside a Flatpak is a real UX problem.** A sandboxed app
  cannot drop files into `~/.local/share/gnome-shell/extensions/` by default. Options: ship it on
  extensions.gnome.org and deep-link the user there; request
  `--filesystem=~/.local/share/gnome-shell/extensions`; or land it in the Bluefin image itself.
  **This needs a decision in Phase 0** (§10.7) — it gates the whole window/clipboard feature set on
  our first platform.
- Host app execution goes through `flatpak-spawn --host` / `OpenURI`; desktop-entry and icon
  indexing needs `--filesystem=host-os:ro` plus the `~/.local/share/{applications,icons}` reads.

---

## 4. Repository layout

```
compass/
├── CMakeLists.txt            # unchanged; C++ keeps building throughout
├── Cargo.toml                # [workspace] members = ["crates/*"]
├── rust-toolchain.toml
├── crates/
│   ├── compass-core/  compass-search/  compass-shell/   compass-wayland/
│   ├── compass-portals/ compass-ipc/   compass-xdg/     compass-platform/
│   ├── compass-ui/    compass-worker-host/
│   ├── compass-testkit/      # fixtures, mock GNOME Shell bus, parity harness
│   └── vicinae/              # the binary
├── packaging/flatpak/        # manifest + Bluefin CI, from Phase 0
├── src/                      # C++ tree, deleted directory-by-directory as parity lands
└── docs/rust-engine/         # this plan, REFERENCES.md, ADRs, PARITY.md
```

Build integration: `make dev-rust`, `make test-rust`, `make flatpak-rust`; an optional
`COMPASS_BUILD_RUST=ON` CMake switch so C++-only builds need no cargo until Phase 7. `make format`
gains `cargo fmt`; the lint gate is `cargo clippy -- -D warnings`. Nix gets a `crane`-based
derivation alongside the existing one.

---

## 5. Migration strategy

Two mechanisms stop this becoming an 18-month branch that never ships.

**(a) The engine switch.** From Phase 1 both engines are installed and selectable:

```
vicinae --engine=cpp
vicinae --engine=rust
COMPASS_ENGINE=rust     # env override for CI and dogfooding
```

Same config and same SQLite file, so users and CI can flip back mid-migration.

Two details this section originally got wrong, corrected once the CLI existed:

*Which default, and whose.* "Default `cpp` until Phase 7" is a property of the **dispatcher** — the
thing installed at `/usr/bin/vicinae` that decides which engine to exec. It is not a property of the
Rust binary, which cannot exec the C++ one: defaulting *that* to `cpp` would make every invocation
fail. So `crates/vicinae` defaults to `rust`, and `--engine=cpp` there parses, is reported by
`doctor` as a warning, and makes engine-dependent commands refuse with exit 1 rather than silently
doing the Rust thing. The `cpp` default lives with the dispatcher when one exists.

*The name collision.* Both engines want to be `vicinae`, and both want the same socket. Whoever does
Phase 6 packaging has to resolve that — a dispatcher that execs one of two differently-named
binaries is the obvious shape, but it is unbuilt and unspecified. Note the socket filename already
differs (`ipc.sock` versus the C++ `vicinae.sock`), deliberately, so the two cannot meet on one
socket and produce a confusing decode failure instead of a clear error.

**(b) The parity ledger.** `docs/rust-engine/PARITY.md` — a checked-in table of every service,
builtin and CLI command with columns `C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓`. Nothing
leaves `src/` until its row is fully green. It is the definition of done for the project.

**Direction of travel:** vertical slice first (a launcher a person can actually use on Bluefin),
then breadth. rustcast already hands us the vertical slice's UI, so this is the cheap direction.

---

## 6. Phases

Each phase has a blocking, checkable exit gate.

### Phase 0 — Scaffolding and the Bluefin loop (≈1.5 weeks)

- Workspace, toolchain pin, CI (`build`, `fmt`, `clippy`, `test`) on Fedora 44/45 containers.
- **Flatpak manifest + `make flatpak-rust`, working from day one** (§3.6).
- Import rustcast under GPL-3.0 with provenance headers; strip every `objc2*` dep and all of
  `src/platform/macos/`; get `compass-ui` compiling as a library rendering a static list.
- `crates/compass-testkit` skeleton; empty `PARITY.md`.
- ADR-0001 Iced over Slint · ADR-0002 postcard over Cap'n Proto · ADR-0003 fluent-rs i18n ·
  **ADR-0004 GNOME Shell extension posture and distribution** (§3.5, §10.7) ·
  ADR-0005 Rhai as a third extension tier (§2.2, §10.9).

**Gate:** `cargo test --workspace` and `clippy -D warnings` green in CI; a Flatpak bundle builds and
launches a blank window on a Bluefin VM.

### Phase 1 — Thin vertical slice on GNOME (≈3 weeks)

Goal: `vicinae --engine=rust`, installed as a Flatpak on Bluefin, binds Super+Space via the portal,
opens a window, fuzzy-matches installed apps, launches one, closes. Nothing else.

- `compass-xdg`: desktop-entry parsing (port `lib/xdgpp` semantics), icon theme lookup — including
  host apps visible via `--filesystem=host-os:ro` and Flatpak exports.
- `compass-search`: `nucleo` behind a `FuzzySearchable`-equivalent trait.
- `compass-wayland`: plain `xdg_toplevel` via stock Iced/winit, centred, `xdg-activation-v1` to
  raise, `keyboard-shortcuts-inhibit-v1` while focused, dismiss on focus loss.
- `compass-portals`: `ashpd` GlobalShortcuts bind, including the first-run permission dialog.
- `compass-platform`: launch via `flatpak-spawn --host` with an `OpenURI` fallback.
- `compass-ui`: rustcast's shell wired to real results.

**Gate:** Suite 0 parity (§8.1) green for app-search ranking on the 500-entry corpus; runs from a
Flatpak on Bluefin with GNOME 50 **and** 51; idle RSS < 30 MB; **works with no Shell extension
installed** (§3.5.1).

### Phase 2 — IPC, CLI, doctor (≈2 weeks)

- `compass-ipc` socket + framing; `vicinae toggle`, `vicinae ext list --json`.
- `vicinae doctor`: portal backends, DBus, socket, Flatpak permissions, **and Shell-extension
  presence/version** (§3.5.4), with `--check-only` exit codes.
- Single-instance handling and `$XDG_RUNTIME_DIR` socket lifecycle inside the sandbox.

**Gate:** IPC round-trip p99 < 0.5 ms (criterion); `doctor` output diffed against the C++ build on
the same machine; `doctor` correctly reports each degradation with the extension uninstalled.

### Phase 3 — GNOME Shell integration (≈3 weeks)

- `compass-shell`: `zbus` client for `org.gnome.Shell.Extensions.{Windows,Clipboard}`.
- Shrink and version the extension's DBus contract per §3.5.2–3; publish the versioned interface XML
  in-tree so the extension and the engine can be reviewed against one another.
- Window switcher; clipboard history with the encrypted SQLite store (port `lib/crypto` semantics);
  paste via the extension.
- Graceful degradation paths and user-visible messaging for every missing capability.

**Gate:** mock-Shell-bus suite (§8.4a) green; clipboard DB readable and writable by both engines
interchangeably; extension-absent and version-mismatch paths both tested; a week of dogfooding by
≥2 people on Bluefin.

### Phase 4 — Extension host (≈6–8 weeks, the hard one)

- **Carve out `compass-extension-api` first**, before the Node host is written against it: the
  capability registry, the view tree and action dispatch, with no knowledge of Node, JSON-RPC or
  Rhai. This is the seam that makes the Rhai tier (§2.2) a binding exercise instead of a parallel
  stack. It costs perhaps three days now and saves weeks in Phase 5.
- `compass-worker-host` spawns `vicinae-worker-ts` per extension over UDS with JSON-RPC 2.0,
  consuming `compass-extension-api` rather than defining its own view model.
- **`src/typescript/` is not rewritten.** The React reconciler and `@raycast/api` shim keep working;
  only the host changes. Any change forced on the SDK is a design smell — escalate it.
- Sandbox: **Landlock** for the filesystem boundary (unprivileged, no bind mounts — a better fit for
  a Flatpak than the spec's read-only mounts), `seccompiler` for the syscall filter, cgroups v2 for
  the 256 MB cap. Note that we are sandboxing *inside* an already-sandboxed Flatpak; verify the
  nesting works on Bluefin early rather than late.
- OAuth, local-storage, toast and navigation host APIs.

**Gate:** Suite 1 (§8.2) — top 25 Raycast store extensions plus every Vicinae store extension run
unmodified **inside the Flatpak**; every negative sandbox test fails closed; and
`compass-extension-api` compiles and passes its tests with `compass-worker-host` removed from the
dependency graph — the cheap mechanical proof that the seam is real.

### Phase 5 — Breadth, and the second compositor (≈8–10 weeks, parallelisable)

Three tracks that do not block each other:

*Track A — builtins:* calculator · clipboard · emoji/glyph · file search (+ `file-indexer`) · font ·
media control · power management · shortcuts · snippets · system · theme · script commands ·
dmenu · store front-ends · window/workspace · developer tools.

**Browser tab search and switching is not in this list.** It is out of scope for the port and
becomes an extension — see [ADR-0008](./adr/0008-browser-control-is-an-extension.md). It is a
browser feature surfaced in a launcher, with no coupling to the compositor, clipboard or index, and
its churn (manifest v2 → v3, per-browser layouts, store review) is not churn we control. Note the
consequence: if no extension exists by Phase 7, this is a **feature regression at cutover** and
belongs in the release notes next to the macOS/Windows narrowing.

*Track B — compositor #2 (wlroots — Hyprland/Sway/niri):* add `wlr-layer-shell` via
[`iced_layershell`](https://crates.io/crates/iced_layershell), `ext-foreign-toplevel-list-v1` +
`wlr-foreign-toplevel-management`, `wlr-data-control` clipboard, and the `xx-hotkey-v1` backend from
#1936 (necessary because `xdg-desktop-portal-wlr` ships **no** GlobalShortcuts backend). KDE is a
third target after that.

*Track C — Rhai extension tier (§2.2).* Independent of both, once `compass-extension-api` exists:
`compass-script` with a hardened engine (`Engine::new_raw()`, explicit package, no
`FileModuleResolver`, the full set of `set_max_*` limits, `on_progress` budget termination); the
capability-gated function registry; `spawn_blocking` execution with a wall-clock timeout; script
discovery and hot reload; and first-party example scripts with authoring docs. The tier ships only
when the examples are good enough that someone can copy one and be productive — an empty tier is
worse than no tier.

**Gate:** parity ledger ≥ 95% green, with every ported group's Catch2 tests ported to Rust (§8.3).
For Track C: the Rhai sandbox negative tests (§8.2) all fail closed, and at least four first-party
example scripts ship with docs.

Note that a row going green does **not** mean its C++ directory is deleted. An earlier draft of this
plan said it did, which was simply wrong: both engines ship side by side until Phase 7, so the C++
engine still needs its own matcher, IPC and parsers however complete the Rust ones are. Deletion is
Phase 8 work, and `PARITY.md` marks such rows `⏳`.

### Phase 6 — Packaging breadth (≈2 weeks)

Flatpak already exists from Phase 0; this phase adds back AppImage, Arch, Nix and the
`vicinae.json` declarative config with a published JSON Schema plus migration from today's config.

**Gate:** Suite 5 (§8.6) green across all outputs.

### Phase 7 — Cutover (≈2 weeks)

Default flips to `--engine=rust` on Linux; the C++ engine stays one release behind a flag.
**macOS and Windows are explicitly out of scope for cutover** — the C++ engine remains the shipping
build there until a follow-up project. Compass supports all three today, and a Linux-only Rust
default is a visible narrowing: say so loudly in the release notes.

**Gate:** one full release cycle with no P0 regressions.

### Phase 8 — Removal

Delete `src/server`, the C++ `src/lib`, and the Linux CMake targets. Keep macOS/Windows targets
until their own migration. Repo becomes Rust-primary.

---

## 7. Rough schedule

| Phase | Effort (1 FTE) | Parallelises to |
|---|---|---|
| 0 Scaffolding + Flatpak loop | 1.5 wks | 1 |
| 1 GNOME vertical slice | 3 wks | 2 |
| 2 IPC / CLI / doctor | 2 wks | 2 |
| 3 GNOME Shell integration | 3 wks | 2 |
| 4 Extension host | 6–8 wks | 2 |
| 5 Breadth + compositor #2 | 8–10 wks | 4+ |
| 6 Packaging breadth | 2 wks | 1 |
| 7 Cutover | 2 wks | — |
| **Total** | **~7 months serial** | **~4 months at 3–4 FTE** |

Order-of-magnitude only. Phase 4 is the one most likely to double.

---

## 8. Testing plan

The gist specifies five suites. They are good and are adopted below, but they share one blind spot:
**nothing in them compares the Rust implementation against the C++ implementation it replaces.** In
a strangler rewrite that differential is the highest-value test available, so it is added as
Suite 0 and it is the gate that matters most in Phases 1–5. The Wayland-mock suite is also
re-weighted: on a GNOME-first target, a **mock GNOME Shell DBus service** catches far more than a
mock wlroots compositor.

### 8.1 Suite 0 — Cross-implementation parity (new; the safety net)

A golden-corpus differential harness in `crates/compass-testkit`.

**Corpora, checked in under `tests/corpus/`:**
- ~500 real `.desktop` files: localised `Name[xx]`, all `Exec` field codes (`%f %F %u %U %i %c %k`),
  `TryExec`, absolute and themed icons, `NoDisplay`, `OnlyShowIn`, malformed and non-UTF-8 entries.
  **Harvest from a live Bluefin image** — host RPM apps, Flatpak exports and Homebrew entries
  together — since that is the exact mix our first users have.
- ~2,000 query/expected-ranking pairs harvested from `src/lib/fuzzy/tests` and
  `src/file-indexer/tests/query-quality.cpp`.
- A recorded clipboard/history fixture and a set of theme files.

**Runner:** for each item, run the operation against both engines (`vicinae --engine=cpp|rust --json`)
and diff structured output.

**Verdicts:** `identical` / `known-divergence` (must cite a ledger row and a rationale) /
`regression` (fails CI). Divergences are *declared*, never discovered.

Plus two compatibility checks that matter because both engines coexist for months:
- *Schema compat:* open the same SQLite file with both engines in both orders; assert no corruption
  and no lost rows.
- *Config compat:* every config in the corpus yields an equivalent effective configuration in both.

This is also how the ledger's "parity test ✓" column gets filled — a row cannot go green without a
Suite-0 case.

### 8.2 Suite 1 — Raycast extension API conformance

- `@vicinae/test-harness` in `src/typescript/`: drives `<List>`, `<Detail>`, `<Form>`,
  `<ActionPanel>`, `<Grid>`, `<MenuBarExtra>` and asserts the reconciler's state tree serialises to
  the expected RPC frames.
- Hooks: `useFetch`, `useCachedState`, `useLocalStorage`, `useExec`, `useNavigation`, `usePromise`,
  `useSQL`.
- Host built-ins mocked and asserted: `environment.assetsPath`, `environment.supportPath`,
  `environment.isDevelopment`, preference resolution, OAuth PKCE.
- **Persistence across restart:** write via `useLocalStorage`, kill the worker, restart, read back.
  Sandboxed state dirs usually break exactly here.
- **Real-extension corpus:** top 25 Raycast store extensions by installs plus every Vicinae store
  extension, installed and driven headlessly through one command each, snapshotting the first frame.
  **Run this inside the Flatpak**, not on a mutable host — extensions that shell out are precisely
  what the sandbox breaks.
- **Negative sandbox tests (Node):** an extension attempting `fork`, raw sockets, writes outside its
  state dir, or a 512 MB allocation must fail closed with a diagnostic, not crash the core.
- **Negative sandbox tests (Rhai), from Phase 5:** a script that did not declare a capability cannot
  reach it — the function is simply absent from its scope. Plus `import` resolves nothing (the
  default `FileModuleResolver` must not be installed); an infinite loop is terminated by the
  `on_progress` operation budget; a script exceeding the wall-clock timeout is killed without
  stalling the render thread; and `set_max_string_size` / `set_max_array_size` /
  `set_max_call_levels` / `set_max_expr_depths` each reject their overflow case.
- **Shared-seam test:** the same fixture extension expressed once in TypeScript and once in Rhai
  must produce the same view tree through `compass-extension-api`. This is the regression test that
  keeps the two tiers from drifting.

`npm --prefix src/typescript test -- --filter=raycast-api-conformance`

### 8.3 Suite 2 — Upstream test harvest ("steal and port")

Compass's Catch2 suites encode years of bug fixes. They are an asset, not legacy.

| Existing suite | Port target | Method |
|---|---|---|
| `src/lib/fuzzy/tests` | `compass-search` | direct port + `proptest` ranking invariants |
| `src/lib/xdgpp/tests` (entry, mime, locale, file-uri, bookmark, special, xdg-terminal-exec, …) | `compass-xdg` | direct port — **the highest-value harvest in the repo** |
| `src/lib/crypto/tests` | `compass-core` | direct port; verify ciphertext compat with existing DBs |
| `src/lib/script-command/tests` | `compass-core` | direct port |
| `src/lib/glyph/tests` | `compass-core` | direct port |
| `src/lib/vicinae-ipc/tests` | `compass-ipc` | re-express against the new framing |
| `src/file-indexer/tests/query-quality.cpp` | `compass-platform` | becomes a Suite-0 corpus |
| `src/snippet/tests` | `compass-core` | direct port |

```rust
proptest! {
    #[test]
    fn desktop_entry_parsing_never_panics(s in "\\PC*") {
        let _ = compass_xdg::DesktopEntry::parse_str(&s);
    }
}
```

**Rule:** a C++ test file may only be deleted in the same PR that adds its Rust equivalent, and the
PR body must show both passing.

### 8.4 Suite 3 — Desktop integration

Re-weighted for a GNOME-first target, in three tiers.

**(a) Mock GNOME Shell bus — per-PR, the workhorse.** A fake `org.gnome.Shell.Extensions.{Windows,
Clipboard}` service on a private DBus in `compass-testkit`. Cheap, fast, no display server. Assert:
window list mirrors state across add/remove/rename races; clipboard signals produce correct history
rows; **extension absent** and **version mismatch** both degrade correctly and surface the right
`doctor` diagnosis; DBus disconnect mid-session reconnects.

**(b) Headless GNOME session — nightly.** `gnome-shell --headless --virtual-monitor` in a Fedora
44/45 container running a scripted 10-step session against both GNOME 50 and 51. This is the tier
that catches real portal behaviour, the GlobalShortcuts permission dialog, and
`xdg-activation-v1` focus semantics.

**(c) Wayland mock compositor — from Phase 5.** The spec's `smithay` headless fixture in
`compass-testkit/src/wayland_mock.rs`, for the wlroots track: layer-shell anchors and margins across
single/dual/mixed-DPI outputs, `ext-foreign-toplevel-list-v1` events, focus-loss dismissal, and
correct degradation when a protocol is absent. Deferred until there is wlroots code to test.

### 8.5 Suite 4 — Benchmarks and resource regression

`criterion` benches with **SLAs enforced as CI failures**, not advisory numbers:

| Metric | SLA | Source |
|---|---|---|
| Fuzzy search, top-20 of 10,000 items | < 2.0 ms | gist spec |
| IPC round-trip, local UDS | < 0.5 ms | gist spec |
| Cold start to first frame | < 120 ms | new — the number users feel |
| Idle RSS | < 30 MB | gist spec |
| Peak RSS, 10k index + 3 extensions | < 150 MB | new |

The spec claims sub-30 MB but proposes no test for it; without a gate the claim decays. Track RSS
per commit, fail on >5% regression. **Measure inside the Flatpak** — sandbox overhead is real and
the number users see is the sandboxed one.

Plus `insta` snapshot tests rendering views to a headless framebuffer. Keep these *few* and
semantic (results list, empty state, detail view, form). Large pixel-snapshot suites get
rubber-stamped and stop catching anything.

### 8.6 Suite 5 — Packaging and deployment

- **Bluefin end-to-end, the headline test:** install the Flatpak on a Bluefin image in CI, bind the
  hotkey through the portal, launch a host RPM app, a Flatpak app and a Homebrew binary, and assert
  no permission panic. This is our first target — it belongs in CI from Phase 1, not Phase 6.
- Flatpak conformance via `flatpak-builder`; host execution through `flatpak-spawn --host` /
  `OpenURI`.
- `vicinae doctor --check-only` asserted to detect present **and absent** portals, protocols, DBus,
  sockets and the Shell extension. Test both directions — a doctor that always says "fine" is worse
  than no doctor.
- Install-matrix smoke for AppImage, Arch, Nix (from Phase 6): `--version` + `doctor`.

### 8.7 Pre-flight command

```sh
cargo test --all-targets --workspace
cargo clippy --all-targets --workspace -- -D warnings
cargo bench --bench slas -- --save-baseline pr
npm --prefix src/typescript test
cargo run -p compass-testkit --bin parity -- --corpus tests/corpus --engines cpp,rust
flatpak run com.vicinae.Vicinae -- doctor --check-only
```

### 8.8 CI wiring

Four tiers, ordered by how real the environment is and how much it costs. A tier only exists if the
tier below it cannot catch the bug.

**Tier 1 — per PR, no display server.** Pure logic; seconds to minutes.

| Job | Trigger | Budget |
|---|---|---|
| build + clippy + fmt + unit | every PR | < 8 min |
| Suite 0 parity (fast corpus) | every PR touching `crates/` | < 5 min |
| Suite 3a mock GNOME Shell bus | every PR touching `compass-shell` | < 3 min |
| Suite 1 TS conformance | every PR touching `src/typescript` or `compass-worker-host` | < 10 min |
| Suite 4 benches | every PR informational; **blocking on `main`** | < 10 min |

**Tier 2 — per PR, headless GNOME in a container.** `gnome-shell --headless --virtual-monitor` in a
Fedora 44/45 container gives real Mutter and a real `xdg-desktop-portal-gnome` without a VM, so it
catches most integration bugs at container cost. Reach for this before reaching for QEMU.

| Job | Trigger | Budget |
|---|---|---|
| Flatpak build | every PR from Phase 1 | < 12 min |
| Headless GNOME session smoke | every PR from Phase 1 | < 10 min |

**Tier 3 — merge queue, a real Bluefin VM.** See §8.9. The only tier that tests what users install.

**Tier 4 — nightly, unbounded.** Suite 0 full corpus; the GNOME 50 **and** 51 matrix; Suite 5
packaging; the wlroots compositor matrix from Phase 5; perf and RSS trends.

`sccache` + `cargo-nextest` keep the per-PR budget honest.

### 8.9 The VM tier: a real Bluefin VM, driven by corral

Tiers 1 and 2 never boot the operating system we ship on. A launcher is a system-integration product
— portals, session, compositor, Flatpak sandbox, Shell extension — so the things most likely to break
are exactly the things a container cannot exercise.

**Substrate: [`tuna-os/corral`](https://github.com/tuna-os/corral), not a hand-rolled harness.** Its
`corral vmtest` builds a bootc image into a disk, boots it under QEMU, waits for the guest, runs
assertions, and writes the evidence — serial console, per-interval screenshots, a timelapse `.webm`,
`result.json`, failed units and `bootc status`. Bluefin *is* a bootc image, so the VM under test is
the target built the way the target is built. See [ADR-0010](./adr/0010-corral-vm-tier.md) for why
this is adopted rather than built.

Three of its properties are the reason it is worth adopting rather than approximating:

- **`--require-paint`.** It measures the standard deviation of the final frame's luminance and fails
  when nothing was drawn. A desktop that boots to a black screen is exactly the failure an SSH probe
  reports as success, and it is the single most likely way a launcher breaks.
- **One exit code per failure class.** Code 2 is "this host cannot run it" — distinct from code 6,
  "the guest never became ready", and code 9, "painted nothing". A red pipeline that cannot tell a
  broken runner from a broken image is a red pipeline people learn to ignore.
- **Console keyboard injection.** `corral key <vm> meta_l spc` sends Super+Space at QEMU's emulated
  keyboard over QMP, and `corral screenshot` grabs the framebuffer. That is the only way to test a
  global hotkey, because a hotkey that works when synthesised by the test harness has not been
  tested at all.

**The KVM question is settled, and the previous answer here was wrong.** This section used to assert
that GitHub-hosted runners expose no `/dev/kvm`, citing a 2022 community discussion, and designed
around Depot sandboxes and self-hosted runners on that basis. A probe run on this branch
([run 34686387919](https://github.com/tuna-os/compass/actions/runs/34686387919)) measured it instead:

| Runner | `/dev/kvm` | `kvm-ok` | QEMU accelerators |
|---|---|---|---|
| `ubuntu-24.04` (x86_64) | present, `nested=1` | "KVM acceleration can be used" | `tcg kvm` |
| `ubuntu-24.04-arm` | **absent** | "does not exist" | `kvm tcg` compiled in, unusable |

So the tier runs accelerated on the x86_64 hosted runners we already have, at no additional
infrastructure cost, and none of the Depot/self-hosted machinery this section used to propose is
needed. On arm64 it would fall back to TCG, so the tier is **x86_64 only** — acceptable, since
Bluefin's own primary target is x86_64.

**Shape of a run:**

1. Build the Flatpak (reuse the Tier-2 artifact).
2. Build a test image: `FROM ghcr.io/ublue-os/bluefin:stable`, plus our Flatpak, the Shell
   extension, and GDM autologin. `corral vmtest` accepts a locally built image, so this needs no
   registry round trip.
3. `corral vmtest --ready-marker 'Reached target Graphical' --require-paint --video`.
4. Assert over SSH: `vicinae doctor --check-only`, the IPC socket, the app index against the guest's
   real `.desktop` files.
5. Drive the hotkey path through the console keyboard, screenshotting each step.
6. Upload the artifact directory unconditionally.

**Assert over IPC, not over pixels.** Screenshots are evidence for humans; `--require-paint` is the
one pixel assertion worth gating on, because "did anything draw" is a question no other probe
answers. Everything else goes through our own IPC socket and `doctor`. Pixel-scraping a desktop
session is the classic way to build an e2e suite everyone learns to ignore.

**Two things this tier will be bad at, stated up front.** Under llvmpipe software rendering a GNOME
session is slow and its timing is variable, so any assertion phrased as "within N seconds" will
flake; phrase them as "after this marker appears". And a screenshot diff against a stored reference
will break on every font, theme and Bluefin update — which is why none is proposed here.

**Promote it; do not start with it.** Run it nightly first and move it into the merge queue only once
it has been stable for a couple of weeks. Then hold it to the same rule as everything else: a
failure is a bug until proven otherwise, and "flake" is not a root cause.

Fedora solves this problem at scale with [openQA](https://openqa.fedoraproject.org/), worth knowing
about if our own harness starts to sprawl — but corral covers the ground we need and openQA is a
much heavier commitment.

---

## 9. Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| GNOME 51 (16 Sep 2026) breaks the Shell extension | **Certain, recurring** | High | §3.5 — tiny versioned extension, graceful degradation, a scheduled compat task each GNOME cycle, and nothing critical-path behind it |
| Can't install the Shell extension from a Flatpak | High | High | ADR-0004 in Phase 0; extensions.gnome.org deep-link, a `--filesystem` hole, or shipping it in the Bluefin image |
| Phase 4 (extension host) overruns | High | High | Freeze `src/typescript`; if the Rust host stalls, ship Phases 1–3 with the **C++ extension host bridged over IPC** as a transitional hybrid |
| Sandbox-in-sandbox (Landlock/seccomp inside Flatpak) doesn't work on Bluefin | Medium | High | Prove it in a Phase-0 spike, before Phase 4 depends on it |
| Iced can't match Qt/QML polish (fonts, **IME**, a11y, RTL) | Medium | High | Prove in Phase 1 with the real theme set. Test IME and screen-reader support early, not late — GNOME users notice. ADR-0001 revisitable until Phase 5 |
| Losing translations | Medium | Medium | §2.1 — `.ts` → `.ftl` converter in Phase 0, catalogue coverage checked in CI |
| macOS/Windows users stranded | Certain | Medium | Stated in Phase 7; the C++ engine remains their build. Do not let it be a surprise |
| Divergence from upstream vicinaehq/vicinae becomes unmergeable | High | Medium | Accept it: after Phase 1 this is a hard fork in practice. Decide deliberately in Phase 1, not by drift |
| A 100k-LOC rewrite never finishes | Medium | Fatal | The parity ledger + engine switch mean partial completion is still shippable value |
| Sandbox breaks legitimate extensions | Medium | Medium | Ship log-only filters first, enforce a release later |
| A third extension API surface (Rhai) drifts from the TS one | Medium | Medium | `compass-extension-api` as a single capability layer, carved out in Phase 4 and proven by the dependency-graph gate. If that seam does not materialise, **drop the Rhai tier** rather than maintain two stacks |
| Rhai tier ships into an empty ecosystem and nobody uses it | Medium | Low | Cheap if the seam exists; gate the tier on four good first-party examples and treat §10.9 as a real go/no-go |
| A Rhai script hangs the UI | Medium | Medium | `spawn_blocking` only, operation budget via `on_progress`, wall-clock timeout, bounded blocking-thread pool |

---

## 10. Decisions and remaining questions

The questions that were open when this plan was written have been decided and recorded as ADRs in
[`adr/`](./adr/). Summary:

| Was | Decided | ADR |
|---|---|---|
| Iced or Slint? | Iced 0.14 | [0001](./adr/0001-iced-over-slint.md) |
| Cap'n Proto or something simpler? | postcard now, Cap'n Proto held in reserve behind the 0.5 ms SLA | [0002](./adr/0002-postcard-over-capnproto.md) |
| i18n, absent from the spec | fluent-rs, with the Qt Linguist catalogue converted rather than lost | [0003](./adr/0003-fluent-for-i18n.md) |
| **How does a Flatpak install the Shell extension?** | extensions.gnome.org **and** baked into the Bluefin image; explicitly **not** a `--filesystem` hole into GNOME's directory | [0004](./adr/0004-gnome-shell-extension-distribution.md) |
| Is the Rhai tier worth it? | Build the seam now; the tier is a product go/no-go at the end of Phase 4 | [0005](./adr/0005-rhai-seam-now-tier-later.md) |
| The fuzzy coherence gap | Reconstruct the signal over nucleo's indices, rather than raising the gate or accepting looser matching | [0006](./adr/0006-fuzzy-coherence-classifier.md) |
| Fork posture, branding, platform scope, GNOME versions | Hard fork acknowledged; `vicinae` user-facing names kept; Linux-first with macOS/Windows on the C++ engine; GNOME 50 **and** 51 in CI | [0007](./adr/0007-fork-posture-and-platform-scope.md) |
| Does browser control belong in the core? | No — it becomes an extension and leaves the port's scope entirely | [0008](./adr/0008-browser-control-is-an-extension.md) |

### Still genuinely open

1. **Team size.** The schedule in §7 swings between four and seven months on this work alone. Nobody
   can answer this from inside the plan.
2. **rustcast relationship** — one-time seed (what the plan assumes and what the crate split
   reflects), or an ongoing sync? The latter would constrain the crate boundaries in §2 and cost
   design freedom. Assumed one-time until someone says otherwise.
3. **Whether to report the six C++ desktop-entry bugs upstream.** ADR-0007 says we should as a
   courtesy; someone has to actually do it. See PARITY.md for the list, one of which is an unbounded
   loop reachable from any malformed `.desktop` file on disk.

### Decided by doing, not by discussion

Some things in the original list resolved themselves once code existed, and are recorded here so
they are not re-litigated: `nucleo` over hand-rolling a matcher; a caller-owned buffer in
`FuzzySearchable` rather than returning `Vec` or a GAT-flavoured iterator; total-order ranking so
results are deterministic; and raw bytes rather than `String` in the corpus API, because part of the
corpus is deliberately not valid UTF-8.

## 11. Current state

Updated as work lands. See [`PARITY.md`](./PARITY.md) for the per-subsystem ledger and
[`adr/`](./adr/) for the decisions.

### Done

Nine crates, 597 tests, and an engine that runs. Every count below was verified in a clean `git worktree` checkout of the
committed tree, not in the working tree — three commits early on built only because the dirty tree
supplied files they had not committed, and that is now checked rather than assumed.

- **Workspace and CI.** Pinned 1.94.1, edition 2024, `unsafe_code` forbidden and `clippy::all`
  denied workspace-wide. Rust CI workflow, Makefile targets kept separate from the C++ ones. All
  workflows migrated off Depot onto GitHub-hosted runners.
- **Corpora.** 8 real `.desktop` entries plus 19 synthetic edge cases, and a harvester for growing
  the real half on a machine that has applications installed. Corpus files are `-text` in
  `.gitattributes`, with a test that fails loudly if a checkout ever normalises the CRLF and
  Latin-1 fixtures into fixtures that test nothing.
- **`compass-xdg`** (110) — desktop-entry, locale, value, reader and exec layers, with all 47
  in-scope C++ cases ported verbatim.
- **`compass-search`** (52) — fuzzy matching on `nucleo`, with the C++ ordering suite ported and
  fzf's coherence signal reconstructed exactly (ADR-0006).
- **`compass-ipc`** (57) — length-prefixed postcard framing, with the length checked against
  `MAX_FRAME_LEN` before any allocation.
- **`compass-core`** (71) — app index with desktop-ID precedence, frecency, `vicinae.json`.
- **`compass-shell`** (36) — GNOME Shell DBus client; 22 of its tests spawn a real `dbus-daemon`.
- **`compass-portals`** (55) — XDG portals via `ashpd`, with availability a three-state outcome
  rather than a boolean, version-property probing, and a timeout on every call.
- **`compass-extension-api`** (74) — the view tree, derived identity, diffing, dispatch and the
  capability registry, behind a mechanical seam gate that fails if host transport or runtime is
  named anywhere in the crate. The gate was itself tested by injecting a violation.
- **`vicinae`** (137) — CLI, an 11-check `doctor`, and **`vicinae serve`: the engine**. It
  indexes applications, ranks queries with frecency and answers over the IPC socket. Headless, and
  the window commands refuse rather than answer `Ack`, so a client can tell "no window yet" from
  "the window was shown". Eleven end-to-end tests spawn the real binary on its own socket with every
  XDG variable pointed into a tempdir.
- **`compass-testkit`** (5) — corpus loader; entries expose raw bytes, not `String`.
- **ADRs 0001–0009.**
- **Flatpak manifest** for the Bluefin target — built, installed and run in CI on every change
  (`.github/workflows/flatpak.yaml`), and layered into the VM tier's test image.
- **i18n converter** — 7,347 messages across 7 locales, all parsing with the real `fluent-syntax`
  crate. ADR-0003's claim that the donated translations survive is demonstrated, not asserted.

### What testing has actually caught

Recorded because the point of §8 is to find defects, and a testing plan that has never failed is
not evidence of anything. In rough order of how quietly each would have shipped:

- **Duplicate sibling keys collapsed two nodes onto one id** (`compass-extension-api`). A property
  test failed on one clean-worktree run and passed on the previous one — proptest draws a fresh
  seed per run. A UI patching on the resulting diff would repaint the wrong row. Fixed with an
  ordinal fallback; the counterexample is now a checked-in regression seed.
- **None of the five proptest suites could persist a counterexample.** The default persistence
  looks for `lib.rs`/`main.rs` beside the test, finds neither under `tests/`, and discards the
  seed — so a rare failure was unreplayable. Found by reading the output of the failure above.
- **`from_file` decoded with `read_to_string`** (`compass-xdg`): one Latin-1 byte lost an entire
  application from the index.
- **The i18n converter's brace escaping corrupted 6 of 7 locales** — caught only because the
  converter's output is validated by the real `fluent-syntax` parser rather than eyeballed.
- **An unbounded loop in `parseRawLocale`** reachable from any malformed `.desktop` file, in
  shipping C++, found while porting.

### What the dev container cannot verify

Worth stating plainly, because "the tests pass" means less than it sounds like until these are
covered. The container has no display server, no `flatpak`, no `qemu`, and no `/dev/kvm`:

- the Wayland surface and anything in `compass-ui`;
- the GlobalShortcuts portal path — an `ashpd` call needs a portal implementation on the bus;
- the Flatpak build, and therefore every claim in `packaging/flatpak/`;
- the VM tier in §8.9 itself.

Every one of those is now reachable **in CI** even though it is unreachable *here*, via the corral
VM tier (ADR-0010) — and three of the four have since been exercised there rather than merely made
reachable: the Flatpak builds and runs, the VM boots a real GNOME session that paints, and the
GlobalShortcuts portal has been asked a real question (§11.1). `compass-ui` is the one still
untested, because nothing starts a window yet.

It *can* run a real DBus session bus (`dbus-run-session` works), so the GNOME Shell integration and
its mock-bus suite are genuinely testable here. That is why Phase 3's testing is further along than
Phase 1's, which inverts the plan's order — deliberately, because verified work beats sequenced
work.

### Blocked on someone with access

- **Run the corpus harvester on a real Bluefin box.** The synthetic corpus is a model of the spec,
  not of reality. (Or take it from the VM tier below, which boots one.)

**Spikes A and B are no longer blocked, and both have now run.** They were filed here as needing
hardware this container cannot provide. [ADR-0010](./adr/0010-corral-vm-tier.md) removed that:
`corral vmtest` boots a real Bluefin VM on the x86_64 hosted runners we already have — measured, not
assumed, see §8.9 — with a real GNOME session, a real portal, a real Flatpak sandbox and console
keyboard injection for the hotkey. What they found is §11.1.

### 11.1 What the spikes measured

**Spike A — the GlobalShortcuts portal.** All three questions now have answers, though the third
turned out to be a fact about the harness rather than about GNOME:

| Question | Answer |
|---|---|
| Is the portal there? | **Yes**, interface v1. The premise of `compass-portals` holds on the target. |
| Is binding permitted unattended? | **Not by default** — but the consent can be pre-seeded, and now is. |
| Does a keypress reach us? | **No — but the kernel gets it.** The loss is above the kernel, not in corral. |

That third row was "unknown" for as long as there was no binding for `meta_l spc` to trigger. With
the consent pre-seeded there is one, so the question was finally asked — and the answer is that the
key never arrives. **Nothing here says the hotkey does not work on real hardware**; it says this
harness cannot press it.

The second row was "No" and is now qualified, because the cause has been traced through all three
components rather than inferred from the symptom (ADR-0010). The portal frontend checks no
permission at all; `xdg-desktop-portal-gnome` forwards to gnome-control-center on a proxy whose
D-Bus timeout is `G_MAXINT`, so no timeout ever fires and "no answer in 30 s" is the designed
behaviour when nobody answers; and gnome-control-center skips its dialog entirely when every
requested shortcut *id* is already stored, which is GSettings on a relocatable schema — dconf, and
therefore image content. `packaging/vmtest/compass-shortcuts.dconf` seeds it.

The third row now has an answer too, and it is about the harness rather than about GNOME. The
control run pressed **Super alone** — which opens the Activities overview, an unmissable change —
and the frames either side are **byte-identical**, while frames from the same mechanism in the
launcher job on the same commit differ at the pixel level. So the capture is live and the key
genuinely did not arrive. Spike A's `activated: false` is a fact about corral's QMP injection, not
about the portal: **the hotkey half of Spike A needs another mechanism**, exactly as ADR-0010's
"what would change our mind" anticipated. Whether the guest has a keyboard device at all is the
next measurement; corral adds none to its QEMU command line.

The consequence for the plan has changed accordingly. It previously read "either the permission is
pre-seeded into the test image, or Phase 1's gate is verified by a human on a real machine". The
first branch is taken: **Phase 1's gate can be demonstrated unattended**, subject to the one thing
the pre-seed does not settle — whether the compositor actually routes Super+Space to us, or to
GNOME's own input-source switcher, which owns that combination by default. The job now records
both the seeded state and the colliding bindings before the spike runs, because from inside the
spike a collision and a portal that does not deliver look identical.

**Spike B — sandbox nesting.** Answered, and the answer is yes to both:

```
Landlock (asked for): V1   ruleset: fully enforced
  reads inside allow:  yes (control)    reads outside deny: yes (assertion)
seccomp filter:        installed
  blocked call denied: yes (assertion)  other calls allowed: yes (control)
```

Measured inside a real bubblewrap sandbox on kernel `6.17.0-1022-azure`. Every assertion is paired
with a control, so a boundary that denies everything is not mistaken for one that works, nor one
that denies nothing for a sandbox at all — and here both halves passed on both mechanisms. **Phase
4's extension host may be designed on Landlock and seccomp**; the risk §6 flagged is retired.

It has since run again in the VM on **Bluefin's own kernel, `7.1.8-200.fc44.x86_64`**, inside the
real Flatpak in a real GNOME session, with the same verdict and the same four rows green. Two
kernels, two sandboxes, one answer.

Two caveats still travel with it: the Landlock ABI is requested at V1 and never detected,
deliberately, since detection makes a security boundary non-deterministic across machines; and
`seccomp_mode` read back `null` inside the Flatpak although the filter provably worked, which means
`/proc/self/status` is not a usable self-check for confinement in the environment we ship into.

### 11.2 Phase 1's gate, evaluated

This section could not exist until now. Phase 1's gate was unevaluable in
principle while there was no launcher — §12 said so — and the launcher now
opens, draws and is verified on every VM run. So the gate can be scored
honestly rather than deferred, and three of its four criteria turn out to be
answerable today.

| Gate criterion | State | Evidence |
|---|---|---|
| Suite 0 parity for app-search ranking on the **500-entry corpus** | ❌ **not met, but moving** | the corpus is **115** entries — 19 synthetic, 96 harvested — after the VM harvest. Was 27. Still 23% of what the gate names. |
| Runs from a Flatpak on Bluefin with **GNOME 50 and 51** | 🟡 **half** | it runs from a Flatpak on Bluefin in CI on every change. One GNOME, not two, and the version was not recorded — the evidence check now prints `gnome-shell --version`. |
| **Idle RSS < 30 MB** | 🟡 **now measured** | never measured before, because there was nothing running to measure. `checks.sh launcher-rss` reads `VmRSS` once the window is up. Reported, not gated. |
| **Works with no Shell extension installed** | ✅ **met** | we ship none at all (ADR-0004), the VM has none, and `doctor` records `gnome.shell-extension` as evidence rather than gating on it. |

**A correction to this section's own first draft.** It said the corpus was the
binding constraint on Phase 1. That is not right, and reading §8.1 properly is
what showed it. Suite 0 is a **differential** harness — "run the operation
against both engines and diff structured output" — so the gate needs three
things, and the corpus is only one:

| Suite 0 needs | state |
|---|---|
| the desktop corpus | **115 of ~500** — partial, and growing |
| a runner that diffs the two engines | **exists** — `compass-testkit`'s `parity` bin, and nothing has ever invoked it |
| a C++ engine runnable on the target | **missing** |

The third is the keystone. The corpus can grow to five hundred entries and
Suite 0 still cannot run, because there is no second engine to diff against.
That makes §12 item 3 — the C++ baseline — the real blocker on Phase 1's gate,
and item 4 a necessary companion rather than the thing in front.

`.github/workflows/cpp-on-target.yaml` takes the cheap half: it installs the
Fedora dependencies inside the Bluefin image and runs `cmake` configure, which
exercises every `find_package(Qt6 … COMPONENTS …)` in the tree without
compiling 144k lines. About a minute against twenty-plus, and it answers the
riskiest unknown — whether Fedora's packages cover what Arch's do — before the
expensive half is written.

It answered on its first run, and the answer is mostly yes:

- **Every Qt6 component resolves.** Configure reached
  `src/lib/script-command/CMakeLists.txt:26` before failing, which is well past
  `find_package(Qt6 6.9 REQUIRED …)` and past `src/server`'s eleven components
  including `GuiPrivate`. Fedora's Qt6 packaging covers what the engine needs.
- **Catch2 does not.** Fedora 44 ships 2.13.10 and the tests require Catch2 3.
  There is no v3 package — checked against Fedora's package database, not
  assumed.

The fix is `-DBUILD_TESTS=OFF` and it is the right answer rather than a
workaround: Suite 0 diffs engine *behaviour* through
`vicinae --engine=cpp --json`, not by running the C++ unit tests, and those
already run on Arch in `build-linux.yaml` where Catch2 is v3. If they ever need
to run on Bluefin, Catch2 3 can be vendored through `FetchContent` exactly as
qtkeychain, layer-shell and cmark-gfm already are.

Two further things this scoring makes concrete, which "the gate cannot be
evaluated" hid:

- **The corpus is a real constraint, just not the binding one.** 115 entries
  against a gate that names 500 is a genuine distance, and one Bluefin image
  yields 88, so closing it needs different machines rather than more runs.
  §8.1 also asks for "host RPM apps, Flatpak exports and Homebrew entries
  together"; the VM harvest produced only the first, because a freshly
  installed bootc image has no user Flatpaks and an empty `/home/linuxbrew`.
- **The RSS figure is an upper bound, not the shipping number.** It is taken
  under llvmpipe, where the renderer keeps buffers it would not need on
  hardware. It is reported rather than gated for the same reason the paint
  deviation was: one sample is not a budget, and a memory gate set from a single
  software-rendered run would be the deviation mistake again in a different
  costume. It goes in the log so the gate can be set from a distribution.

## 12. Immediate next steps

Rewritten as items land; the previous version listed the VM tier and both spikes as the work to do,
and all three now exist.

**Done since the last revision:** the Flatpak builds and runs in CI; the VM tier boots Bluefin with
our Flatpak in it and asserts from a real GNOME session; Spike A has an answer (§11.1); Spike B runs
in both the Flatpak job and the VM; every workflow defaults to read-only permissions.

**The largest gap is that there is no launcher.** `compass-ui` and `compass-wayland` exist as
libraries, but nothing wires them into the binary: `vicinae` still answers `toggle`, `show` and
`hide` with "this engine is headless", `LaunchSelected` returns `Task::none()`, and no code starts a
window. Until that changes, Phase 1's gate cannot be evaluated at all and the VM tier's subject is
the portal and sandbox questions rather than the launcher. This is issue #4 and it is the next thing
that matters.

Ordered by what unblocks the most:

0. ~~**The launcher does not draw a window on the target.**~~ **Retracted — it draws.** This item
   was written from three VM runs that screenshotted before the launcher had painted. The run that
   added a stock GNOME application as a control also delayed the shot by a few seconds, and the
   launcher's window is plainly in it: 8.22% of pixels changed in a box at x 335–942, y 152–796,
   against a window configured 640×480 centred (x 320–960, y 160–640). The same run carries 119
   wgpu records where the previous had none — Vulkan through lavapipe, `llvmpipe (LLVM 19.1.7)`,
   Mesa 26.1.8. Software rendering works.

   What was really wrong is that startup under llvmpipe is slow and variable — 2.4s to wgpu in one
   run, not yet there at 8.1s in another — and `launcher-start` waited for the *process* to exist
   rather than for the renderer to be up. That broke this plan's own rule, and ADR-0010's: key off
   a state, never a moment. It now waits for `Adapter AdapterInfo` in the launcher's log, which
   wgpu emits only once it has a surface.

   Still true and unaffected: QMP key injection does not reach the session (below).
1. ~~**Wire the UI into the binary**~~ — done (#29). `vicinae ui` opens a window, moves a
   selection with the arrow keys, launches through `compass-platform` and dismisses. It draws on
   the target, verified on every VM run against a measurement that reproduced three times
   (ADR-0010). Phase 1's gate is consequently evaluable for the first time — scored in §11.2.
2. ~~**Settle Spike A's consent question**~~ — done (§11.1, ADR-0010). Traced through all three
   components and pre-seeded; what remains is to read the first run that gets a binding, and in
   particular whether Super+Space survives GNOME's own claim on it.
3. **Capture the C++ baseline on the target.** Today's parity suites compare the port against *our
   reading* of the C++ source; this compares it against the C++ behaviour on the real OS.

   The prerequisite — getting a Qt6 build into the VM — is now costed, and it is much cheaper than
   this item assumed. The assumption was that it meant reviving the AppImage path, whose build-env
   image compiles GCC 15.2 and Qt 6.10 from source and takes hours. **That source build is
   AppImage's portability requirement, not the engine's**: `build-linux.yaml` already builds the
   whole C++ engine against distro Qt, in an `archlinux:latest` container, in about two minutes,
   from a dep list of a dozen packages.

   So the route is to build the C++ engine **inside the Bluefin image itself** — it is a container
   image, so `podman run` it, `dnf install` the Qt6 devel packages and build there — and layer the
   resulting binary into the test image. Building in the exact image the VM boots is not fussiness:
   `src/server/CMakeLists.txt` links `Qt6::GuiPrivate`, so the binary is bound to a specific Qt
   build, and a `fedora:44` container would drift from a pinned Bluefin tag with no warning until
   something crashed at load. One dnf transaction, one build job of roughly the Arch job's cost,
   and no toolchain compiled from source anywhere.

   The dependency side has since been checked rather than assumed, and it is smaller than the Arch
   list suggests. `qt6-qtbase-private-devel` is the package carrying the private headers Arch ships
   inside `qt6-base`, and it exists in Fedora 44 at **Qt 6.11.2** — comfortably past the `Qt6 6.9`
   the top-level `CMakeLists.txt` requires. (An earlier revision of this item said 6.10; that was
   wrong, and 6.11.2 is what Fedora 44 actually has.)

   **A correction: an earlier revision of this item had the vendoring backwards**, and two red CI
   runs paid for it. It claimed `qtkeychain` and `layer-shell-qt` are vendored through
   `FetchContent` "and neither is on by default". The opposite is true on Linux:

   - `USE_SYSTEM_DEFAULT` is `ON`, and `OFF` only for `APPLE OR WIN32`;
   - `USE_SYSTEM_LAYER_SHELL` is `ON` unconditionally (`CMakeLists.txt:53`);
   - `USE_SYSTEM_QT_KEYCHAIN` follows `USE_SYSTEM_DEFAULT`, so `ON` here.

   The `FetchContent` calls I had read are reached only under `PREFER_STATIC_LIBS` — the AppImage
   path. A normal Linux build links system libraries, which is exactly what the comment above those
   options says it prefers. So the Arch list is not padding: it names what the build genuinely
   wants.

   For Fedora that means `layer-shell-qt-devel` is required and available (6.7.5), while
   `qtkeychain` has no Qt6 build at all and has to be switched to the vendored copy explicitly with
   `-DUSE_SYSTEM_QT_KEYCHAIN=OFF`. That flag makes the build fetch, so the build container needs
   network — which a runner has.

   The AppImage path stays disabled either way. Nothing here needs it.
4. **Grow the corpus — Phase 1's binding constraint (§11.2), and now unblocked.** The gate names a
   500-entry corpus for Suite 0 ranking parity; there are 27. Not the launcher, not the portal, not
   the sandbox: the corpus.

   The harvester's own header asks for "a real desktop — ideally a Bluefin box, since that is the
   first target and its RPM + Flatpak + Homebrew mix is what users actually have". **The VM tier
   boots exactly that, every run.** The machine the script was waiting for has been in CI since the
   tier existed, and nobody noticed — including this plan, which listed the corpus as blocked on
   hardware nobody had.

   The `launcher` job now runs it and publishes `corpus.tar.gz` as an artifact. What it does *not*
   do is write into `crates/compass-testkit/corpus/`: a corpus shapes every ranking assertion the
   project makes, and it must not grow by a job quietly appending to it.

   **The first harvest is committed: 88 new entries, taking the real set from 8 to 96 and the
   corpus to 115.** All stock Fedora/GNOME, reviewed before landing. What a single Bluefin image
   yields is 88, so the remaining distance to 500 is not one more run of the same job — it needs
   *different* machines, or an image with more Flatpaks installed. That is the shape of the
   remaining work, and it is now a known shape rather than an unknown one.

5. **Widen the parity port.** Both halves of this item turned out to be nearly done when looked at,
   so what is left is now stated precisely rather than as a direction:

   - *`compass-core`'s index against the harvested corpus* — the corpus is exercised, but its floor
     was `>= 19` against a corpus of 27, which is exactly the synthetic count: the eight entries
     harvested from a real host were added later and the guard was never raised, so all eight could
     have been deleted silently. The floors are now per-provenance and the harvested set has its
     own test. `Provenance` had been defined in the testkit and used by no consumer at all.
   - *The remaining Catch2 ordering cases in `compass-search`* — all 21 C++ `TEST_CASE`s are ported
     across 26 tests. Two assertions remain deferred, and both are **declared divergences with
     pinning tests**, not gaps: Latin Extended-A folding (nucleo does not fold `Ł`/`ź`) and one
     ordering case from upstream issue #946. Neither can be closed without shipping our own fold
     table or reproducing fzf's bonus constants, so neither is a to-do — they are decisions.

   What genuinely remains under this heading is the corpus itself: 8 harvested entries from one
   host is a thin sample, and `scripts/harvest-desktop-corpus.sh` is how it grows.
6. **Promote the VM tier to the merge queue** once it has been stable for a couple of weeks
   (ADR-0010). It has three consecutive green runs; that is not two weeks.

Both corral bugs this tier found on locally built bootc images are now filed upstream:
`podman create` on a CMD-less image (tuna-os/corral#303) and the layer builder pulling a
`localhost/` reference its own disk builder already guards against (tuna-os/corral#304). Our
workarounds stay until they are fixed; neither is blocking.
