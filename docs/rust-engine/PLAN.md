# Compass Rust Engine — Transformation Plan

**Status:** proposal, not yet approved for implementation
**First target:** **GNOME 50 / 51 on [Bluefin](https://projectbluefin.io/)** — everything below is
sequenced around that, with other compositors as later phases.
**Scope:** replace the C++23/Qt6 core of this repository with a Rust workspace, seeded from
[MystikoLab/rustcast](https://github.com/MystikoLab/rustcast), following the Rust engine
specification in [this gist](https://gist.github.com/hanthor/ba051ebc406ddb4b8f958601378a5234)
(the spec is in the gist **comment**; the gist body is the earlier C++/Qt6 variant and is treated
here as reference).

**Direction, as of [ADR-0017](./adr/0017-a-new-launcher-not-a-reimplementation.md):** Compass is a
new launcher in the spirit of Vicinae, not a byte-for-byte reimplementation of it and not a
replacement to upstream. Wherever this plan says "parity", read it as *provenance and tripwire*:
quality is asserted by tests that say what good looks like, the C++ engine is a reference, and
off-the-shelf crates beat hand-rolled code unless nothing maintained does the job.

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
| **Product posture** | **A new launcher in the spirit of Vicinae — quality asserted absolutely, crates first, user data imported rather than shared ([ADR-0017](./adr/0017-a-new-launcher-not-a-reimplementation.md))** | Medium — reversing it means re-adopting byte compatibility |
| Crate prefix | `compass-*`; the binary, config, socket and app ID are `compass` / `org.tunaos.compass` since the Phase 7 rebrand ([ADR-0020](./adr/0020-phase-7-rebrand.md)) | Trivial |
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
                        │  bin/compass (CLI)   │  ext install/remove/list, doctor, toggle
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
| `compass-ipc` | UDS at `$XDG_RUNTIME_DIR/compass/ipc.sock`, length-prefixed frames | C++ `lib/vicinae-ipc`, `lib/figura` |
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
4. **`compass doctor` must diagnose this precisely** — extension present / absent / version-mismatch,
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
│   └── compass/              # the binary
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
compass --engine=cpp
compass --engine=rust
COMPASS_ENGINE=rust     # env override for CI and dogfooding
```

Same config and same SQLite file, so users and CI can flip back mid-migration.

Two details this section originally got wrong, corrected once the CLI existed:

*Which default, and whose.* "Default `cpp` until Phase 7" is a property of the **dispatcher** — the
thing installed at `/usr/bin/vicinae` that decides which engine to exec. It is not a property of the
Rust binary, which cannot exec the C++ one: defaulting *that* to `cpp` would make every invocation
fail. So `crates/compass` defaults to `rust`, and `--engine=cpp` there parses, is reported by
`doctor` as a warning, and makes engine-dependent commands refuse with exit 1 rather than silently
doing the Rust thing. The `cpp` default lives with the dispatcher when one exists.

*The name collision.* Both engines want to be `vicinae`, and both want the same socket. Whoever does
Phase 6 packaging has to resolve that — a dispatcher that execs one of two differently-named
binaries is the obvious shape, but it is unbuilt and unspecified. Note the socket filename already
differs (`ipc.sock` versus the C++ `vicinae.sock`), deliberately, so the two cannot meet on one
socket and produce a confusing decode failure instead of a clear error. The Phase 7 rebrand
([ADR-0020](./adr/0020-phase-7-rebrand.md)) settled the collision by renaming the Rust binary to
`compass`.

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

Goal: `compass --engine=rust`, installed as a Flatpak on Bluefin, binds Super+Space via the portal,
opens a window, fuzzy-matches installed apps, launches one, closes. Nothing else.

- `compass-xdg`: desktop-entry parsing (port `lib/xdgpp` semantics), icon theme lookup — including
  host apps visible via `--filesystem=host-os:ro` and Flatpak exports.
- `compass-search`: `nucleo` behind a `FuzzySearchable`-equivalent trait.
- `compass-wayland`: plain `xdg_toplevel` via stock Iced/winit, centred, `xdg-activation-v1` to
  raise, `keyboard-shortcuts-inhibit-v1` while focused, dismiss on focus loss.
- `compass-portals`: `ashpd` GlobalShortcuts bind, including the first-run permission dialog.
- `compass-platform`: launch via `flatpak-spawn --host` with an `OpenURI` fallback.
- `compass-ui`: rustcast's shell wired to real results.

**Gate:** the app-search quality suite (`crates/compass-core/tests/search_quality.rs`) green on the
real 757-entry corpus, and Suite 0 (§8.1) green at its CI gate as a tripwire (ADR-0017); runs from a
Flatpak on Bluefin with GNOME 50 **and** 51; idle RSS < 30 MB; **works with no Shell extension
installed** (§3.5.1).

### Phase 2 — IPC, CLI, doctor (≈2 weeks)

- `compass-ipc` socket + framing; `compass toggle`, `compass ext list --json`.
- `compass doctor`: portal backends, DBus, socket, Flatpak permissions, **and Shell-extension
  presence/version** (§3.5.4), with `--check-only` exit codes.
- Single-instance handling and `$XDG_RUNTIME_DIR` socket lifecycle inside the sandbox.

**Gate:** IPC round-trip p99 < 0.5 ms (criterion); ~~`doctor` output diffed against the C++ build on
the same machine~~ — **not achievable as written, see below**; `doctor` correctly reports each
degradation with the extension uninstalled.

**Where this gate stands, measured rather than assumed:**

- **IPC round-trip: met.** p99 **47.9 µs** against the 500 µs budget, ~10× headroom, now asserted by
  `crates/compass-ipc/tests/roundtrip_budget.rs` rather than printed. §8.5 records how the previous
  benchmark reported 11.9 ms by timing its own setup.

- **`doctor` diffed against the C++ build: withdraw it.** The C++ engine has **no `doctor`
  command** — its entire CLI is `launch app`, `ls`, `launch cmd`, `ping`, `toggle`, `open`, `close`,
  `dmenu`, `version`, `deeplink`, `logs`. There is nothing to diff against, and this is the second
  gate criterion found to assume a C++ interface that has never existed (the first was Suite 0's
  `compass --engine=cpp --json query`, §8.1a). Both were written against an imagined C++ CLI rather
  than the one in `src/cli`.

  Worth noting even if someone built that command: **the diff would mostly prove nothing.** Eleven of
  `doctor`'s fourteen checks — `dbus.session`, `session.type`, `xdg.runtime-dir`, `xdg.application-dirs`,
  `desktop.environment`, `flatpak.sandbox`, `portal.desktop`, `portal.global-shortcuts`,
  `gnome.shell-extension`, `wlroots.capabilities`, `a11y.screen-reader` — are probes (`input-server`, added later, is half each) of the *environment*. Two processes on one machine observe the
  same environment by construction, so they would agree trivially, in the same way "same top result"
  would be trivially 100% over single-hit queries. Only `engine.selected` and `ipc.socket` describe
  the engine itself, and those map to the C++ `version` and `ping`.

  **What the criterion actually wants is that `doctor`'s picture of the machine is accurate, and
  that is already tested — non-differentially, against reality.** The VM tier runs
  `checks.sh doctor` and `doctor-assert` inside a real GNOME session every run. Restate the
  criterion as that, and keep the differential ambition for `version`/`ping`, where the two engines
  genuinely have something to compare.

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
- `compass-worker-host` spawns `vicinae-worker-ts` per extension over **stdio**, speaking
  **JSON-RPC 2.0 inside a four-byte big-endian length prefix**, consuming `compass-extension-api`
  rather than defining its own view model.

  This line used to say "over UDS", which contradicted the next bullet: the worker that is not to
  be rewritten speaks stdio, and adding a socket to it would rewrite it for no capability the host
  needs. The encoding was never in dispute — figura is an IDL that generates JSON-RPC 2.0
  bindings, not a wire format of its own — so keeping stdio costs nothing and the conflict was
  only ever the transport. §11.4a has the evidence and #101 the history.
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

*Track A status:* running end to end (engine, IPC, launcher page, tests) — calculator, clipboard
history, emoji, window switching, power, media and volume, Search Files, shortcuts (Create
Shortcut, Manage Shortcuts, shortcuts in root search; IPC v13), and snippets (Create Snippet,
Manage Snippets: copy, paste, edit; keyword expansion through the ported `vicinae-input-server`,
`crates/compass-input-server`, figura-wire-compatible with the C++ helper; IPC v15), script commands (scanned into root search, run in all five output modes), Run
Terminal Program, `compass dmenu`, Set Theme, Create Extension, Browse Fonts, and the two store
front-ends (Extension Store and Raycast Store: browse, search, detail with README and screenshots,
install with zip-slip and size guards, uninstall, update detection; IPC v14). What each still
lacks is in `PARITY.md`, one section per builtin.

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

> **Track B status (2026-09-24).** Landed and verified on **headless Sway 1.9** (not a mock): the
> launcher is an `iced_layershell` surface (`top` layer, centred, exclusive keyboard) chosen at
> runtime when `zwlr_layer_shell_v1` is advertised and the desktop is not GNOME — GNOME is decided
> by name first, so its path cannot move; window list/focus/close over
> `zwlr_foreign_toplevel_manager_v1` (list-only fallback on `ext_foreign_toplevel_list_v1`)
> answering the engine's existing `ListWindows`/`ActivateWindow`/`CloseWindow`; clipboard history
> watched over `ext`/`wlr` data-control, and the extension `Clipboard` API set/read/cleared through
> `wl-clipboard-rs`; the `xx-hotkey-v1` client (bindings generated from the C++ tree's XML), which
> no released compositor carries yet, so the documented fallback — bind `compass toggle` in the
> compositor, with a per-compositor hint in the log — is what users get today. Gated by
> `.github/workflows/wlroots.yaml`: `compass-wayland` against Sway, the engine against Sway, and
> the launcher on Sway (on screen, absent from Sway's window tree, toggles, takes typed text), each
> gate with a control that fails. Not done: synthetic paste (copy only; since done, `PARITY.md`
> "The gaps pass, wlroots paste and inhibit"), `ext-workspace`,
> focus-back via `xdg-activation` serials, the C++ `launcherWindow.layerShell.*` config keys, and
> Hyprland/niri themselves (only Sway runs in CI). PARITY.md "wlroots" has the differences.
>
> **Track B, 2026-09-25: the Hyprland and niri providers.** Ported
> (`compass_platform_linux::compositor`): Hyprland over its request socket (hand-rolled, lenient
> replies, the C++'s Lua dispatchers with the classic ones as a fallback) and niri over `niri-ipc`
> (CRATE-AUDIT "Compositor IPC"). They feed `WindowManagement` (windows with workspace, pid and
> Hyprland's geometry; the workspace list; the active workspace; Hyprland's frontmost window) and
> give the window switcher's toplevels their pid and workspace. `compass doctor` gains
> `wlroots.capabilities` (layer-shell, foreign-toplevel, data-control, xx-hotkey, portal
> GlobalShortcuts, compositor IPC). Tested against fake sockets replaying captured replies and
> against headless Sway with a fake Hyprland socket; neither compositor runs in CI.

*Track C — Rhai extension tier (§2.2).* Independent of both, once `compass-extension-api` exists:
`compass-script` with a hardened engine (`Engine::new_raw()`, explicit package, no
`FileModuleResolver`, the full set of `set_max_*` limits, `on_progress` budget termination); the
capability-gated function registry; `spawn_blocking` execution with a wall-clock timeout; script
discovery and hot reload; and first-party example scripts with authoring docs. The tier ships only
when the examples are good enough that someone can copy one and be productive — an empty tier is
worse than no tier.

*Track C status (2026-09-24):* **wired into the launcher.** `compass-script` has the hardened
engine, capability-gated registry, blocking-pool execution with a terminating deadline,
`script.toml` discovery under `$XDG_DATA_HOME/compass/scripts`, and `notify` hot reload; 26
negative sandbox tests fail closed, a shared-seam test holds it to the TS tier's `to_view`, and
five examples ship in `extensions/rhai-examples/` with [RHAI-SCRIPTS.md](./RHAI-SCRIPTS.md). The
engine (`crates/compass/src/rhai_scripts.rs`, `rhai_host.rs`) loads them at start, lists each as a
root-search command (`rhai:script.<name>`), and opens one as an extension view session, so the
launcher's extension page draws it unchanged: the search text goes to `search`, actions run
through the seam's `ActionIndex`/`Pending`, and toasts, HUDs, re-rendering, closing and popping are
carried out. The real `ScriptHost` reaches the extensions' clipboard (GNOME Shell extension or
wlroots data-control), the default-application opener, Compass's encrypted local storage (a
namespace per script) and `notify-rust`. Grants: packaged scripts get what they declare; the user's
own get it after a one-time consent prompt in the launcher, kept in
`$XDG_CONFIG_HOME/compass/script-grants.json`. The user directory is created and watched; edits
rebuild the script, re-render an open view, and update root search. The five examples install
under `share/compass/scripts` in every package (`install-rust-engine.sh`). Its share of IPC v14 is only
`ListRhaiScripts`/`RhaiScripts`. Tested in-process against a `MemoryHost` (`tests/rhai_scripts.rs`)
and against the real process (`engine_end_to_end.rs`). Not done: script icons in the root list
(rows use the initial badge, as extension commands do), an action panel on a script's root row,
and a settings page to review or revoke consent (edit or delete the file).

**Gate:** every feature area in the ledger has absolute tests — ported Catch2 cases count where they
state intended behaviour, not where they pin a C++ quirk (§8.3, ADR-0017).
For Track C: the Rhai sandbox negative tests (§8.2) all fail closed, and at least four first-party
example scripts ship with docs.

Note that a row going green does **not** mean its C++ directory is deleted. An earlier draft of this
plan said it did, which was simply wrong: both engines ship side by side until Phase 7, so the C++
engine still needs its own matcher, IPC and parsers however complete the Rust ones are. Deletion is
Phase 8 work, and `PARITY.md` marks such rows `⏳`.

### Phase 6 — Packaging breadth (≈2 weeks)

Flatpak already exists from Phase 0; this phase adds back AppImage, Arch, Nix and the
`compass.json` declarative config with a published JSON Schema plus migration from today's config.

**Gate:** Suite 5 (§8.6) green across all outputs.

**Status (2026-09-24):** built, gate not yet observed green. `compass.json`'s schema is generated
by `schemars` into `packaging/schema/compass.schema.json` (drift test: `config_schema`), and
`compass_core::config_migration` + `compass config migrate` carry the C++ `settings.json` across.
AppImage, Arch (`compass-git`) and Nix (`.#compass`) share one install script with the Flatpak and
one smoke (`scripts/packaging/smoke.sh`), run by `packaging.yaml` and `flatpak.yaml`. See
`packaging/README.md`. The `/usr/bin/vicinae` collision with the C++ packages is gone: the binary is `/usr/bin/compass`
since ADR-0020.

### Phase 7 — Cutover (≈2 weeks)

Default flips to `--engine=rust` on Linux; the C++ engine stays one release behind a flag.
**macOS and Windows are explicitly out of scope for cutover** — the C++ engine remains the shipping
build there until a follow-up project. Compass supports all three today, and a Linux-only Rust
default is a visible narrowing: say so loudly in the release notes.

"A follow-up project" was doing a lot of work in that sentence — it is 229 shared translation units
and 202 platform conditionals inside them, and until it happens the repository keeps two engines and
every shared change is made twice. It is now
Phases 9 and 10 rather than an unowned successor; see
[ADR-0013](./adr/0013-qt-leaves-the-repository.md).

**Gate:** one full release cycle with no P0 regressions.

### Phase 8 — Remove the Linux C++ engine

**Linux targets only.** Not "Removal" — the repository is not Rust-primary at the end of this phase
and the plan no longer claims it is. See [ADR-0013](./adr/0013-qt-leaves-the-repository.md).

This section previously read *"Delete `src/server`, the C++ `src/lib`, and the Linux CMake targets.
Keep macOS/Windows targets until their own migration. Repo becomes Rust-primary."* That is not
executable: **every** macOS and Windows source file lives *inside* `src/server`, so it cannot be
deleted while those targets are kept.

#### How much this removes, measured from the build rather than from paths

The counts here were originally derived by matching path fragments (`wayland`, `macos`, `windows`
and so on). That method is wrong in both directions and the numbers it produced were wrong: it
counted `src/server/src/ui/windows` — the UI's *window* classes, 11 files — as Windows-platform
code, and it counted 28 "macOS files" when only **three** macOS translation units exist, the rest
being headers and directory-name matches.

Attributing each translation unit to the `if (APPLE)` / `if (WIN32)` / `if (UNIX AND NOT APPLE)`
block that lists it in `src/server/CMakeLists.txt` is the measure that matches what a deletion
actually removes. All 320 `.cpp` files under `src/server/src` are listed there, so this covers the
whole build:

| | `.cpp` | share |
|---|---:|---:|
| Linux-only | **59** | 18% |
| Windows-only | 33 | 10% |
| macOS-only | 3 | 1% |
| compiled everywhere | 229 | 70% |

Headers are a different denominator — 520 on disk, 337 named in `CMakeLists.txt` — and travel with
whichever unit includes them, so they are not counted separately.

**So this phase removes on the order of 59 translation units and the Linux CMake targets.** That is
the smaller half of the work, and the plan used to stop here.

#### What Phase 8 does *not* remove, and the plan used not to say

Platform-specific behaviour is not confined to platform-specific files. It is also conditional
compilation inside the 229 units that compile everywhere:

| guard | sites | files |
|---|---:|---:|
| `Q_OS_MAC` | 102 | 38 |
| `Q_OS_WIN` | 100 | 43 |
| `Q_OS_LINUX` | 72 | 30 |

**61 shared files carry at least one platform conditional**, concentrated in `server.cpp` (29
sites), `vicinae.cpp` (14), `utils/environment.hpp` (11) and `utils/capabilities.cpp` (11).

Deleting the 59 Linux units therefore leaves **72 `Q_OS_LINUX` sites inside files that stay**. They
are dead code the moment the Linux engine is Rust, and dead conditional compilation is worse than
dead functions: it does not warn, it is not covered by any test on any platform, and it silently
changes what the *other* platforms compile when someone edits around it. Removing them is part of
this phase, not a tidy-up for later.

### Phase 9 — macOS (≈6–8 weeks)

This phase used to read, in full: *"Implement the platform seam for macOS: clipboard, window
management, tray, global shortcuts, file indexing. Delete the macOS C++ targets (28 files plus
their share of the cross-platform core)."*

Two things were wrong with that. The 28 is a path-match artefact — there are **three** macOS
translation units. And **"their share of the cross-platform core" is not executable**, which is the
same defect ADR-0013 corrected in Phase 8: a file compiled on three platforms has no share that can
be deleted on one of them. The shared core goes when the *last* platform leaves it, in Phase 10, or
it does not go at all.

So the macOS work is not "port three files". It is the **102 `Q_OS_MAC` sites across 38 shared
files**, each of which has to become either a Rust implementation behind a trait or a deliberate
decision not to support it.

**The traits this needs.** `compass-platform` today declares exactly one, `AppLauncher` (#64), with
`NullLauncher` as its second implementation. Each of the following is a trait added when the port
reaches it, implemented once for Linux and once for macOS:

| seam | Linux today | macOS backend |
|---|---|---|
| clipboard read/write | `wlr-data-control` | `NSPasteboard` |
| window management | portal + compositor | Accessibility API, needs a user grant |
| tray | StatusNotifierItem | `NSStatusItem` |
| global shortcuts | XDG portal | `RegisterEventHotKey` / Carbon |
| file indexing | `inotify` + walk | Spotlight (`NSMetadataQuery`) or the same walk |
| autostart | `.desktop` in autostart dir | `SMAppService` |

**Two decisions this phase forces**, neither of which has a Linux precedent to copy:

1. **Window management needs Accessibility permission**, which the user grants in System Settings
   and which cannot be requested silently. The Linux engine has no equivalent step, so the
   onboarding flow gains a macOS-only branch — a product decision, not only an engineering one.
2. **SQLCipher's crypto provider is `SQLCIPHER_CRYPTO_CC` (CommonCrypto) on macOS**, chosen at
   compile time. `libsqlite3-sys`'s `bundled-sqlcipher` build (through `rusqlite`) selects it when
   no OpenSSL is configured, as `vendor/sqlcipher/CMakeLists.txt` does — but **that path has never
   been built or run** for Compass, by CI or by anyone, since CI went Linux-only. It is a reading, not a green check. Phase 9 starts by
   re-enabling `Build (macOS)` and finding out; see [ADR-0014](./adr/0014-clipboard-storage-is-sqlcipher-plus-a-vendored-tokenizer.md).

**What this phase deletes:** the three macOS translation units, the `if (APPLE)` CMake blocks, and
the 102 `Q_OS_MAC` sites. **Not** the shared core.

**Gate:** the same suites the Linux engine gates on, running on macOS, plus a clipboard database
written by the C++ engine on macOS and read by the Rust one; one release cycle with no P0
regressions.

### Phase 10 — Windows, and Qt leaves (≈8–10 weeks)

The same shape, and larger: **33 Windows translation units and 100 `Q_OS_WIN` sites across 43
files**. It is last because it is the platform furthest from the others — no XDG, no D-Bus, a
different shortcut model, and the only one whose SQLCipher provider is a custom hook
(`SQLCIPHER_CRYPTO_CUSTOM=sqlcipher_cng_setup`, backed by `bcrypt.dll`) rather than a stock one.
A Windows build that silently picks OpenSSL instead writes a database the C++ engine cannot read,
which is why that selection is asserted in `build.rs` rather than left to a default.

**Windows-specific work with no Linux or macOS precedent:**

- `files-service/windows` is **7 translation units** (plus 8 headers), the largest single platform
  backend in the tree, and it wraps the third-party Everything SDK (`vendor/everything-sdk3`) over
  a named pipe. Either that dependency is carried into Rust or file search on Windows is
  reimplemented — a scope decision this phase has to take explicitly.
- Global shortcuts, paste, selection and the snippet server each have a `windows-*` implementation
  sitting beside their Linux counterparts rather than under a `windows/` directory, so they are
  easy to miss when enumerating by path. They are listed in `CMakeLists.txt` under `if(WIN32)`,
  which is why the build is the right thing to enumerate from.

**When this lands**, the last conditional leaves the shared core, `src/server` and the C++
`src/lib` are deleted **in full**, the CMake targets go with them, and **the repository is
Rust-primary** — the claim Phase 8 used to make three phases early.

#### What is left in `vendor/` when Qt goes

"No Qt" is not "no C", and the plan did not previously say what happens to the eleven vendored
trees. Checked by looking for each name in the C++ `CMakeLists.txt` files and in the Rust crates'
manifests:

| tree | after Phase 10 |
|---|---|
| `sqlcipher` | **goes** with the C++ engine, which is the only thing still compiling it. The format stays (ADR-0014): the Rust engine links `libsqlite3-sys`'s bundled SQLCipher 4, which reads and writes the same files |
| `fuzzy-trigram` | **stays** — same reason: without it the FTS table cannot be opened at all |
| `everything-sdk3` | **stays only if** Phase 10 keeps Everything for Windows file search; goes with that decision |
| `cmark-gfm`, `pugixml`, `spellfix`, `kirigami-wheelhandler` | **go** with the C++ engine — referenced only by its CMake |
| `CLI11`, `tomlplusplus`, `rang` | **already unreferenced** by either build; they can go at any time and are not Phase 10's problem |
| `zip` | referenced by the C++ CMake only. The Rust extension host will need archive extraction, but from a Rust crate rather than this tree — no Rust manifest depends on it |

A caution for whoever checks this again: `rang` and `zip` produce dozens of false hits in the Rust
tree (`range`, `ranger`, `.zip()`). The counts above come from dependency declarations in
`Cargo.toml`, not from grepping source.

**Gate:** as Phase 9, on Windows; plus `grep -r Q_OS_ src/` returning nothing, because there is no
`src/server` left to search.

---

**Phases 9 and 10 exist because of [ADR-0013](./adr/0013-qt-leaves-the-repository.md).** ADR-0007
decision 3 left them as "a follow-up project", which has no owner, no phase and no gate — so Qt
would not have left late, it would not have left at all.

**The one thing this changes before Phase 4**, and the reason the ADR was worth writing now rather
than at cutover: the platform seam gets built while it is still cheap. Measured today —

- Linux-only dependencies are confined to `compass-portals`, `compass-shell`, `compass-wayland` and
  the `compass` binary, with **zero** `cfg(target_os)` guards anywhere. That part is in good shape.
- **`compass-platform` defines no traits.** It is named like a seam and is not one: two files, and
  it depends on `compass-portals`.
- **`compass-ui` depends directly on `compass-portals` and `compass-wayland`**, so the crate built
  on the portable renderer (`iced` + `winit` + `wgpu`, no Qt) is itself Linux-bound.

Phase 4's extension host and Phase 5's breadth will be written against whatever shape those crates
have when they land. Fixing the dependency direction now is days; retrofitting it afterwards is
weeks.

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
| **Linux subtotal** | **~7 months serial** | **~4 months at 3–4 FTE** |
| 8 Remove the Linux C++ engine | 1–2 wks | — |
| 9 macOS | 6–8 wks | 2 |
| 10 Windows, and Qt leaves | 8–10 wks | 2 |
| **Total to Qt leaving** | **~11 months serial** | **~6 months at 3–4 FTE** |

Order-of-magnitude only. Phase 4 is the one most likely to double.

**The subtotal row is the point.** This table used to end at Phase 7 and call ~7 months the total,
which quietly described a port that leaves Qt in the repository, ~229 shared translation units
still compiled by CMake, and every change to shared behaviour made twice — the outcome ADR-0013
rejected. Phases 8–10 are the other four months, and they are what the word *complete* is doing in
"the complete port".

Phases 9 and 10 parallelise to 2 rather than 4: each is one platform backend behind traits that
already exist by then, so the limit is how many people can usefully work on one operating system's
seam, not how much work there is.

**What is not in this estimate:** neither 9 nor 10 has been costed against a working build. CI has
been Linux-only since #71, so the macOS and Windows paths of the SQLCipher build Compass links
(`libsqlite3-sys`'s `bundled-sqlcipher`) have never run anywhere. On Windows that build wants
OpenSSL (`OPENSSL_DIR`), where the C++ engine uses a CNG provider; both write SQLCipher 4 files. The first task of Phase 9 is re-enabling `Build (macOS)` and replacing
that estimate with a measured one.

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

**Runner:** for each item, run the operation against both engines (`compass --engine=cpp|rust --json`)
and diff structured output.

**Verdicts:** `identical` / `known-divergence` (must cite a ledger row and a rationale) /
`regression` (fails CI). Divergences are *declared*, never discovered.

Plus two compatibility checks that matter because both engines coexist for months:
- *Schema compat:* open the same SQLite file with both engines in both orders; assert no corruption
  and no lost rows.
- *Config compat:* every config in the corpus yields an equivalent effective configuration in both.

This is also how the ledger's "parity test ✓" column gets filled — a row cannot go green without a
Suite-0 case.

### 8.1a What Suite 0's harness actually does today

`crates/compass-testkit/src/parity.rs` has existed since early in the port and **has never been
run**. §11.2's first scoring called it "exists — and nothing has ever invoked it", which was
generous: running it is how the following came to light, and none of it is visible from reading the
file.

It shells out once per query, as `<engine> --engine <name> --json query <text>`, and parses stdout
as a JSON array. Measured against the real binaries:

| | state |
|---|---|
| the argv it built | **rejected by clap** — `unexpected argument '--json' found; tip: 'query --json' exists`. `json` is a flag on the subcommand, not a global. Fixed, and pinned by a test in `crates/compass/src/cli.rs`. |
| `query` against the Rust engine | **needs a running engine.** It asks over the IPC socket, so every call returns *"no Compass engine is listening on /tmp/vicinae-default/ipc.sock"*. The harness starts nothing. |
| `query` against the C++ engine | **the interface does not exist.** `src/cli` has no `--engine` flag and no `query` subcommand; its only `--json` is on the command-list subcommand. |

A fifth problem only appeared once the first four were fixed and the thing actually ran: it passed
`--engine <name>`, which is wrong in principle rather than in spelling. **Until the Phase 7 cutover
(§5), the binary *is* the engine.** The Rust binary refuses `--engine cpp` by design, and the C++
binary has no such flag at all, so passing it can only turn a working invocation into a failing
one. Suite 0 picks an engine by choosing which path to exec — which is what `--cpp` and `--rust`
are for.

**The harness now runs.** `RunningEngine` starts a `serve` per side on its own socket, waits until
`ping` answers — a state, not a sleep, per ADR-0010 — runs the queries, and shuts both down. Against
the Rust engine on both sides it completes 378 queries and reports 378 identical.

That number is an **identity control only**, and on its own it is indistinguishable from a
comparison that never compares. So the comparison is separately controlled: perturbing one side
(dropping its top hit) turns the same run into 105 regressions and a non-zero exit, and
`compare_results` has unit tests for reordering, rescoring, a missing hit and an empty side. The
JSON test asserts the old `{key, name, score, quality}` shape is *rejected*, so it would have caught
the original mismatch rather than passing either way.

#### The C++ side: decided, in two rungs

The question was whether to add the interface §8.1 assumes to the C++ engine, or to re-aim §8.1 at
an interface it already has. **The second is not available**, which is worth stating rather than
leaving as an option:

- `src/cli` has no command that emits ranked results. Its `-q/--query` flags on `toggle` and `open`
  send a deeplink that opens the window with fallback text; nothing prints a ranking.
- The IPC protocol (`figura/ipc.fig`) has no ranked-search method either. Its only query is
  `fsQuery`, which searches **files**, not root items.

So there is no existing surface to diff through. What there *is*, and what changes the cost
completely:

**`vicinae::fuzzy` is a header-only INTERFACE library with no Qt dependency.** All five of its
public headers compile standalone under plain `g++ -std=c++23` with nothing but their own include
directory — verified, not assumed. The C++ scorer is separable from the server, the IPC, the window
and Qt entirely.

That gives a first rung far cheaper than anything previously costed:

1. **A test-only probe binary linking `vicinae::fuzzy`**, emitting the same JSON for a query over a
   corpus. Seconds to build, no Qt, no 812-object link, no VM, and no product surface added to a
   tree we are deleting — it dies with `src/`. Diffed against `compass-search`, it covers the part
   of ranking most likely to drift silently and least likely to be noticed: the scorer's bonus
   constants and tie-breaks. §8.3 already ports all 21 Catch2 cases, but those compare against
   *our reading* of the algorithm; this compares against the algorithm.

2. **The full pipeline still needs the engine.** The probe is not a substitute and must not be
   described as one. `RootItemManager::searchGroupedByProvider` wraps the scorer in provider
   bucketing, a separate provider-name score, favourite and enabled filtering, and per-item
   `fuzzyScore` — so scorer parity is not ranking parity, and Phase 1's gate names ranking. Closing
   that means giving the C++ engine a ranked-output path: an IPC method, a server handler and a CLI
   command. That is real work in a tree being deleted, and it is justified only because Suite 0 is
   the migration's safety net — §12 item 3 exists precisely because every other parity test we have
   compares the port against our reading of the C++ source rather than its behaviour.

**Rung 1 is built and running** (`src/lib/fuzzy/probe/main.cpp`, `compass-testkit`'s
`scorer-parity` bin, and the `scorer-parity` job in `rust.yaml`). It compiles the C++ scorer with a
bare `c++ -std=c++23 -Isrc/lib/fuzzy/include` — one translation unit, no CMake, no Qt — and diffs
it against `compass-search` over the harvested corpus.

Only one corpus parser exists, on the Rust side: the probe scores `id<TAB>text` lines handed to it
on stdin, so a disagreement about which `Name=` line to take cannot masquerade as a scoring
divergence.

**Its first run found six queries where the two scorers disagree**, out of 293 derived from the
115-entry corpus. Every one had the query matching NON-CONTIGUOUSLY with the C++ engine stricter,
and this section originally generalised that to "the Rust port is systematically more permissive".

**Both the count and the generalisation were artefacts of a small corpus, and the next harvest
destroyed them.** At 738 real entries the same harness reports:

| | |
|---|---|
| queries | 1685 |
| identical | 1333 |
| **divergent queries** | **352 (20.8%)** |
| **divergent (query, entry) pairs** | **1417** |

And the direction is not one-way:

| shape | count |
|---|---|
| C++ rejected, Rust accepted | 771 |
| both accepted, C++ higher | 440 |
| both accepted, **Rust** higher | **145** |
| **Rust** rejected, C++ accepted | **61** |

C++ stricter or higher in 1211 cases, Rust in 206. So "systematically more permissive" describes
the dominant direction and is false as a rule — 206 cases go the other way, and the 115-entry
corpus contained none of them. One distribution's stock application set is not a sample.

**This is not a new defect, and an earlier revision of this section wrongly called it one.**

`PARITY.md`'s "`compass-search` — nucleo is not fzf" already records that the two use different
algorithms, that absolute scores are on different scales and are never asserted, and that the
normalized values "match the C++ expectations closely". Both shapes measured above are already
listed there:

- divergence **#4**, *"nucleo's score depends only on the matched region, not on haystack length or
  match position"* — that is the single-character case, C++ 30 against Rust 26;
- divergence **#2**, *"nucleo prefers a short scatter inside one word starting at position 0; fzf's
  larger word-boundary bonuses pull the other way"* — that is the non-contiguous case.

Choosing `nucleo` over hand-rolling a matcher is a settled decision (§10, "not re-litigated").

**What is new is the number.** `PARITY.md` said "closely" and had no way to say more, because the
only evidence was a ported ordering suite over hand-written cases. Against 738 real entries,
"closely" means **79.2% of queries identical**, with the remainder localised to the raw matcher and
counted in both directions. That is the contribution: a documented qualitative divergence turned
into a measured one that cannot drift unnoticed.

So the ratchet is not a defect being driven to zero. **Zero would mean replacing nucleo**, which
§10 settles the other way. The ratchet exists so that this known divergence stays *exactly* as big
as it is, and any change — a nucleo bump, a scoring tweak, a corpus edit — has to be looked at.

#### The score gap is almost invisible in ranking terms

**2026-09-20 follow-up:** the single-character Unicode boundary correction
documented in `PARITY.md` removes six Bear Factory score discrepancies. The
current ratchet is 351 divergent queries / 1411 pairs (previously 352 / 1417).
Top-1 remains 1684/1684, including all 920 contested queries; top-3 improves to
1638 and full-order agreement to 1425. The historical measurements below describe
the earlier capture. This small correction does not close end-to-end root ranking
or authorize treating the lower-level scorer gate as a full-engine benchmark.

Everything above measures **score equality**. Phase 1's gate does not: it names *ranking* parity,
and a user sees an ordered list, not a number. Those turn out to be very different questions.

Over the same 1685 queries:

| | |
|---|---|
| **same top result** | **1684 of 1684 — 100%** |
| same top 3 | 1637 (97.2%) |
| same full order | 1421 (84.4%) |

**The control matters here more than the figure.** "Same top result" would be trivially 100% over a
corpus where most queries return one hit, so the harness also counts the contested ones: **920
queries return more than one hit, and the engines agree on the best match in all 920.** The
assertion is control-tested too — reversing the Rust ranking makes it fail on 918 of 1684.

So the 20.8% score divergence is very nearly **invisible where it would matter**. Two engines can
disagree that `LibreOffice` scores 83 or 72 for `O` and still put the same entry first, and across
this corpus they always do.

That makes top-1 agreement an *assertion* rather than a ratchet: a regression from 100% is a
user-visible change in what the launcher puts first, and it is not a known nucleo-vs-fzf
consequence to be held still. The two measures answer different questions, which is why both are
kept.

It also revises this section's third framing in a row, and this time in the port's favour. The
score gap was called a new defect (wrong — it is declared in `PARITY.md`), then a large parity gap
(true of scores, misleading about behaviour). What it actually is: an internals difference between
two matching libraries that the ranking almost entirely absorbs.

#### Why the score assertion is a ratchet

Enumerating 1417 exceptions is not a declaration, it is surrender: nobody reads a list that long,
and one that long hides a regression as well as no check at all. So `scorer-parity` pins the
measured totals and fails when they get **worse** — the regression it exists to catch — and also
when they get **better**, because a baseline nobody lowers rots into a rubber stamp. Either way
someone has to look at what changed before moving the number.

The ratchet caught its own baseline being wrong on the first run: the figures were copied from a
run that reported divergences minus the ten then declared, so it failed at 352/1417 against
351/1407. Both directions are control-tested.

**The target is not zero.** Zero means replacing nucleo, and §10 settles that the other way. The
baseline holds a known, declared divergence still so that it cannot move unnoticed — which is what
`PARITY.md`'s qualitative entries could not do on their own.

These are **not** the two divergences §8.3 and `PARITY.md` already declare — Latin Extended-A
folding and an ordering case from upstream #946. Those are unrelated; these are ASCII and about
match contiguity and scoring scale.

**Which engine is right is still not decided here**, and at this scale it is a real question about
search behaviour rather than a bug with an obvious side.

#### Where the divergence actually is

Both `score_query` implementations normalise identically — `raw * 100 / self`, same structure, same
tie-breaks. **The raw matcher is what differs**, and comparing it directly on single strings gives a
reproducer small enough to debug:

| haystack | needle | C++ raw | Rust raw | |
|---|---|---|---|---|
| `3` | `3` | 36 | 36 | agree (this is `self`) |
| `Appearance` | `A` | 36 | 36 | agree — match at index 0 |
| `System` | `Sy` | 62 | 62 | agree — contiguous |
| `a3` | `3` | **30** | **26** | C++ higher |
| `LibreOffice` | `O` | **30** | **26** | C++ higher |
| `System` | `Se` | **29** | **47** | **Rust** much higher |
| `Appearance` | `Ac` | **29** | **43** | **Rust** much higher |

Matches at index 0 and fully contiguous matches agree exactly. Everything else diverges, and these
are **two separate defects pulling opposite ways**:

1. **A single character not at the start** scores 30 in C++ and 26 in Rust — a constant offset in
   whatever bonus applies to a non-boundary position. This is the one that made the small corpus
   look one-directional, because a 115-entry stock GNOME set is mostly short single-word names
   where this is the only case that arises.
2. **A non-contiguous multi-character match** scores 29 in C++ and 43–47 in Rust. Rust is applying a
   far weaker gap penalty, which is why it accepts `Se`/`System` and `Ac`/`Appearance` where C++
   rejects them outright at `MIN_QUALITY`.

Defect 2 is the larger effect and the one the enlarged corpus exposed: longer, multi-word
application names give non-contiguous alignments a chance to occur at all.

Both are `nucleo_matcher` behaviours, not arithmetic errors in this codebase:
`compass-search`'s `Matcher` wraps `nucleo_matcher::Matcher` with `Config::DEFAULT`, while the C++
side is a vendored fzf. Changing either number means configuring nucleo away from its defaults or
replacing it — the decision §10 records as settled — rather than fixing a bug.

The value of pinning it here is that a **nucleo version bump** now shows up as a ratchet failure
with a number attached, instead of as a silent change in what users see.

Rung 2 stays scoped as its own item.

3. *Then* wire it into the VM tier, where the C++ binary now is. That step is genuinely just
   wiring, and it was not before.

Three spellings of this harness's invocation were in the repository at once — `--cpp`/`--rust` in
the code, `--engines cpp,rust` in §8.7, and `--cpp-engine` in §12 — which is what an interface with
no caller looks like after a while. §8.7 now matches the code.

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

**Where this stands, and one thing it did not cover.** The suite lives in
`crates/compass-shell/tests/` rather than `compass-testkit` (the mock needs the crate's own
`contract` constants, and nothing outside `compass-shell` consumes it), and every assertion listed
above is implemented: 21 tests across window round-trips, malformed replies, timeouts, signals,
extension-absent, one-sided extensions, version mismatch and shell restart.

What it did **not** cover was the contract document itself. Phase 3 asks us to "publish the
versioned interface XML in-tree so the extension and the engine can be reviewed against one
another", and `dbus/*.xml` was published — but the only check on it was a substring test asserting
the XML `contains` `<method name="ActivateWindow">`, compared against a list of member names typed
into the same test file. That check could not fail for the reason its comment gave: it never
touched the proxies, so a method renamed in both `proxy.rs` and the mock left the XML stale and the
test green; and it never looked at a signature, so `ActivateWindow(u)` could become
`ActivateWindow(s)` on the wire with the document unchanged. The XML was decorative, and
`contract.rs` and `proxy.rs` both asserted in prose that it was not.

`tests/contract_introspection.rs` now serves both interfaces on a private bus, reads their
`org.freedesktop.DBus.Introspectable.Introspect` output, and compares it to the checked-in document
member by member and argument by argument — method and signal sets, argument count, order, type and
direction, and property type and access. Renaming `CloseWindow` to `DestroyWindow` in the mock was
run as a control and the check reports both halves of the drift.

Two limits are worth stating rather than leaving to be discovered. `zbus` emits no **names for out
arguments**, so argument names are compared only where both documents supply one; a control pins
that as intended. And `zbus` offers no way to introspect a `#[zbus::proxy]` trait, so the XML cannot
be compared to `proxy.rs` directly: the chain is XML ≡ mock (this test) plus mock ≡ proxy
(`every_contract_member_is_reached_through_the_proxy`, which drives the whole surface through the
real client and fails if the contract grows a member it does not exercise). What nothing in this
repository can prove is that the real GNOME Shell extension implements the contract — the extension
is not in this tree. The XML is the artefact the two sides are reviewed against; this makes our side
of it true.

**A second thing the suite did not defend: whether it runs at all.** Every D-Bus test opens with
`start_or_skip`, which returns `None` and prints a banner when there is no `dbus-daemon` on PATH —
correct on a developer machine, and in CI indistinguishable from success, because a job whose 21
tests all skip is a green job. The GitHub Actions Ubuntu image does ship `dbus-daemon`: confirmed by
reading a run's log rather than by assuming, and the tests are really executing today. But nothing
made that a requirement, so the whole of Suite 3a rested on an unstated property of a runner image.
The Rust workflow now sets `COMPASS_REQUIRE_DBUS=1`, under which a missing `dbus-daemon` is a
failure instead of a skip. Three controls were run: absent and unguarded skips and exits zero,
absent and guarded fails with the reason, present and guarded passes all twelve.

**The clipboard crypto is now covered too, in the per-PR tier.** Phase 3's gate wants the clipboard
store "readable and writable by both engines interchangeably", and that looked like VM-tier work.
It is not: `vicinae::crypto` is a standalone static library whose only Linux dependency is OpenSSL —
no Qt, no CMake needed to consume it — so three translation units give the REAL C++ implementation
to test against, exactly the property that made the fuzzy-scorer probe affordable (§8.1a).

**The instrument had to differ from the scorer's, and that is the interesting part.** Scoring is a
pure function, so `scorer-parity` compares outputs directly. Encryption is not: the IV comes from
`RAND_bytes`, so two *correct* implementations produce different bytes on every call, and a harness
that diffed ciphertexts would fail on a correct port — the same error as a ratchet that fires when
a number improves. So `crypto-parity` **cross-decrypts**: each engine reads what the other wrote,
in both directions. That is what "interchangeably" means, and it is stronger than a diff, because
it exercises each side as reader and as writer. `deriveKey` is deterministic (HKDF-SHA256) and *is*
compared byte for byte.

Cross-decryption alone would be passed by an implementation that ignored the GCM tag, so every run
also asserts both engines **refuse** what they should, with the specific error each should give: a
flipped bit at every offset is `AuthFailed`, the right blob under the wrong key is `AuthFailed`, a
buffer too short for an IV plus a tag is `DataTooShort`. Measured on the first green run: **15 KDF
vectors identical, 10 cross-decryptions each way, 64 controls refused.**

Five controls were run against the harness itself, because a parity harness that has only ever seen
agreeing implementations is evidence of nothing:

| broken thing | what the harness said |
|---|---|
| Rust appends the IV instead of prefixing it | the C++ engine could not read what Rust wrote |
| Rust HKDF uses a salt | KDF disagreement, with both hex values |
| Rust `decrypt` falls back to raw ciphertext when the tag fails | Rust accepted a flipped bit at offset 0 |
| probe path does not exist | exit 1, naming the path |
| probe exits without answering | exit 1, naming the unanswered request |

Two things it does **not** prove. It is the crypto, not the store: `clipboard-db.hpp` and
`clipboard-encrypter.cpp` add a SQLite schema, key management and a mime model, none of it
exercised. And the CI job pins one HKDF vector before running the harness, because a probe that
built but could not answer would otherwise surface as "no divergences" — the harness's own failure
looking like success, which is the defect this whole suite exists to rule out.

A protocol hole showed up on the first run and is worth recording: an empty KDF label hex-encodes to
an empty string, which whitespace-separated fields cannot distinguish from a missing argument, so
the probe rejected it as malformed. Empty labels and empty plaintexts are both legitimate, and both
are in the corpus precisely because they sit on boundaries; the wire format now spells the empty
string `-`. A corpus of only comfortable inputs would have left that hole in place.

**Key derivation, and a tautology caught in the act.** One master key in the login keyring expands
by HKDF into a SQLCipher key and a clipboard key, under the labels `vicinae-db` and
`vicinae-clipboard`. Those labels and the keyring entry name `vicinae-master-key` are a *data
format*: get one wrong and `database-key.cpp`'s own error message is what a user sees — *"the
affected database files must be deleted to reset"*. `compass-crypto::keys` ports the derivation, and
`compass-crypto/tests/cpp_constants.rs` parses the constants back out of `database-key.cpp`,
`vicinae.hpp` and `aes-gcm.hpp` rather than trusting the copy. Shown to fire on a renamed label, a
renamed keyring entry, and a third derived purpose appearing.

The obvious companion check — have `crypto-parity` derive with the shipped labels and diff — **was
written and is a tautology**, because the label handed to the C++ probe comes from the Rust
constant, so changing that constant changes what the probe is asked for and the two agree again. It
was caught by control-testing it: setting `CLIPBOARD_LABEL` to `vicinae-clipboard-v2` left the run
green. It has been removed rather than kept as reassurance. The real claim decomposes into two
checks that each *can* fail — the Rust labels equal the C++ source labels (`cpp_constants`), and
HKDF agrees byte for byte for arbitrary labels (`crypto-parity`) — and together they give "Rust
derives what C++ derives, for the label C++ uses".

Reading the keyring is deliberately not ported yet: it needs a Secret Service backend and a running
daemon to test against, and it is separable from the derivation, which is where the irreversible
mistake lives.

**What the keyring entry actually looks like, which is not what you would guess.** The C++ engine
reaches the keyring through qtkeychain v0.14.0 (pinned in `cmake/QtKeychain.cmake`), which on Linux
goes through libsecret. Compass has to find the *same* entry, and none of what that requires is
documented anywhere — it was read out of that tag's `libsecret.cpp`:

| | |
|---|---|
| attribute `user` | `vicinae-master-key` |
| attribute `server` | `vicinae` |
| attribute `type` | `base64` |
| attribute `xdg:schema` | `org.qt.keychain`, added by libsecret itself |
| the secret | **base64 text of the 32 raw bytes, not the bytes** |

The encoding is the trap. `database-key.cpp` calls `setBinaryData`, and qtkeychain's binary mode
does `password.toBase64()` on write and `QByteArray::fromBase64` on read. A port that stored 32 raw
bytes would write an entry the C++ engine base64-decodes into garbage — and the failure is silent
until a user's database will not open. The `keyring` crate's default attributes
(`application`/`service`/`username`) miss on every count as well.

A second subtlety: `findPassword` searches `type="plaintext"` **first**, and only retries
`type="base64"` on a miss. So a text-mode entry shadows a binary-mode one, and Compass must write
what the C++ engine *writes*, not what it looks for first.

`compass-crypto::keyring` carries the contract and the encode/decode, with a test asserting the
stored form is *not* the raw bytes — so "simplifying" it fails a test rather than a migration.
The D-Bus client is not written: this machine has no `gnome-keyring-daemon`, no `libsecret` and no
`secret-tool`, so it could only be tested against a mock written alongside it, which would prove the
two agree with each other and nothing about the real thing. The VM tier boots a full GNOME session
and is where that work belongs.

Because the format is a property of **v0.14.0** and cannot be re-verified offline,
`cpp_constants.rs` asserts the pin has not moved, and says what to do if it has.

**(b) Headless GNOME session — nightly.** `gnome-shell --headless --virtual-monitor` in a Fedora
44/45 container running a scripted 10-step session against both GNOME 50 and 51. This is the tier
that catches real portal behaviour, the GlobalShortcuts permission dialog, and
`xdg-activation-v1` focus semantics.

**(c) Wayland mock compositor — from Phase 5.** The spec's `smithay` headless fixture in
`compass-testkit/src/wayland_mock.rs`, for the wlroots track: layer-shell anchors and margins across
single/dual/mixed-DPI outputs, `ext-foreign-toplevel-list-v1` events, focus-loss dismissal, and
correct degradation when a protocol is absent. Deferred until there is wlroots code to test.
**Built instead as a real compositor** (2026-09-24): headless Sway per test, in
`crates/compass-wayland/tests/support`, which proves the client against a compositor people run
rather than against a mock written beside it. Multi-output and mixed-DPI layouts are not covered
yet.

### 8.5 Suite 4 — Benchmarks and resource regression

`criterion` benches with **SLAs enforced as CI failures**, not advisory numbers:

| Metric | SLA | Source |
|---|---|---|
| Fuzzy search, top-20 of 10,000 items | < 2.0 ms | gist spec |
| IPC round-trip, local UDS | < 0.5 ms | gist spec |
| Cold start to first frame | < 120 ms | new — see the split below |
| **Summon to first frame** | **< 120 ms** | **new — the number users actually feel** |
| Idle RSS | < 30 MB | gist spec |
| Peak RSS, 10k index + 3 extensions | < 150 MB | new |

**The first-frame row split, and the second half is the one that matters now.**
[ADR-0015](./adr/0015-the-launcher-window-is-resident.md) made the launcher window resident, so a
user pressing Super+Space is no longer waiting on a cold start at all — they are waiting on a warm
process opening a surface. Those are different numbers with different costs:

* **Cold start to first frame** is paid once, when the window process is first started (autostart,
  or by hand). It includes process spawn, dynamic linking, Iced and winit initialisation, and wgpu
  enumerating and bringing up an adapter. 120 ms was never a realistic budget for it — ADR-0015
  rejected spawn-per-summon precisely because this is seconds, not milliseconds, and the VM tier's
  own figures (2.4 s in one run, not yet there at 8.1 s in another) are two orders of magnitude off.
* **Summon to first frame** is paid on every keypress, and it is what the SLA was always about. It
  is a `WindowCommand::Show` arriving on an attached window, the window opening a surface, and the
  first paint. Everything expensive — the process, the adapter, the font atlas, the application
  index — is already warm.

**The warm half now has a proxy, and it is not the SLA.** `scripts/vmtest/launcher.sh` times a
`compass show` against an attached window: CLI to engine, engine to window, and the window's answer
back. Two runs report **338 ms and 480 ms** under llvmpipe.

That number is an **upper bound with a whole Flatpak launch inside it** — the client is `compass`
rather than a keypress, so a process spawn, a Flatpak sandbox setup and a socket connection are all
counted before the engine is even asked. On the real path the portal delivers an activation
straight into a running engine and none of that happens. It is also not a frame: ADR-0010 settles
that nothing inside the guest can observe one, so what is timed ends at the window's *answer*,
which the launcher now sends only once the window actually exists or is actually gone.

**So the 120 ms figure is still a target carried over from the spec, not a result.** Closing the
gap needs the host-side paint gate the cold number uses, with the clock started at the toggle. What
the round trip does establish is a ceiling and a regression signal, which is more than the row had
before.

The spec claims sub-30 MB but proposes no test for it; without a gate the claim decays. Track RSS
per commit, fail on >5% regression. **Measure inside the Flatpak** — sandbox overhead is real and
the number users see is the sandboxed one.

**"Enforced as CI failures, not advisory numbers" was not true of ANY row.** An audit of the five:

| SLA | what existed |
|---|---|
| Fuzzy search, top-20 of 10,000 | no benchmark — now measured, see below |
| IPC round-trip | one benchmark, which timed a sleep — see below |
| Cold start to first frame | no benchmark — **nearest observable proxy now reported**, see below |
| Summon to first frame | no benchmark — **the round trip is now reported**, see below; still not a frame |
| Idle RSS | VM tier reports it; documented as reported-not-gated (§11.2) |
| Peak RSS, 10k index + 3 extensions | no benchmark — **now measured end to end, and missed 1.7x**, see below |

The workspace contained **exactly one benchmark**, `compass-ipc`'s. **No CI job ran `cargo bench`
at all**, so no benchmark could have failed anything even had it been correct. And §8.7's
pre-flight command invoked `cargo bench --bench slas`, **a target that did not exist** — the third
documented-but-absent interface found this week, after Suite 0's `--engine=cpp --json query` and
the C++ `doctor`.

The fix for the two that are measurable without a display or a sandbox is to assert them in
**tests**, which CI already runs on every PR, rather than in benches, which it does not run at all.

##### The `slas` target now exists, and it is a baseline rather than a gate

`crates/compass-testkit/benches/slas.rs`. The documented command runs:

```
$ cargo bench --bench slas -- --save-baseline pr
slas/fuzzy_rank_top20_of_10k   time: [2.0104 ms 2.0106 ms 2.0113 ms]
slas/ipc_roundtrip_ping        time: [44.318 µs 45.323 µs 45.574 µs]
```

Only the two rows that are honestly measurable in-process are in it. The other four need a
display, a compositor or the shipped process inside its sandbox, and a bench that printed a number
for them would be measuring the harness — the mistake this section was written to undo.

**It does not enforce anything, on purpose.** A criterion bench exits zero whatever it prints,
which is how `compass-ipc`'s 24x miss went unnoticed; the thresholds stay in tests. What the target
adds is the thing a threshold cannot give: `--save-baseline pr` against `--baseline main` turns
"is this over the line" into "did this change move", which is the only way to see a 4% regression
that never crosses a limit.

Both measurements were control-tested before being believed. Shrinking the haystack from 10,000 to
200 items moved the fuzzy figure from 2.011 ms to 36.9 µs; a 1 ms sleep in the server's request
handler moved the round trip from 45.3 µs to 2.200 ms. Neither number is scaffolding.

The fuzzy figure landing at 2.0106 ms — within 0.5% of its own 2.0 ms SLA — is worth reading
alongside the percentile discussion below rather than as a separate result: criterion reports a
mean, `ranking_budget.rs` asserts a median, and the two agreeing this closely on the line is the
same marginality seen from a second direction.

#### Fuzzy search, top-20 of 10,000 — met at the tail, and now gated there

`crates/compass-search/tests/ranking_budget.rs`. Five release runs of 1000 samples, before and
after ranking moved onto rayon's pool:

| | single-threaded | **parallel** |
|---|---|---|
| p50 | 1209–1242 µs | **788–806 µs** |
| **p99** | **1589–2548 µs — over budget in two runs of five** | **1116–1237 µs — over in none of five** |
| max | 2277–2654 µs | 2325–6269 µs |

**The row does not say which statistic it means, and for a long time the answer mattered.** At the
median the SLA was met with ~1.6× headroom; at p99 it was not reliably met on an unloaded machine,
so the test asserted the median and only reported the tail — gating a number that failed two runs
in five teaches people to re-run until it passes.

**That question is now moot, so the gate is the strict reading.** p99 sits ~1.6× inside the budget
across five runs, so `ranking_budget.rs` asserts p99 as well as p50. The stricter interpretation of
the row is the one that holds, which is a better outcome than picking a percentile by argument.

**`max` is still not asserted, and is now noisier than it was.** A worker pool trades a tighter p99
for a longer tail: a sample landing while the pool wakes costs milliseconds, which is scheduling
rather than ranking. Asserting the single worst sample of a thousand would reintroduce exactly the
flaky gate this section argues against.

##### What was tried, and what did not work

Three experiments, measured rather than reasoned about. The haystack matters: the SLA bench uses
`&str` items with **one** weighted field, while `AppItem::fuzzy_fields` emits **five**, so real
ranking does roughly 4.3× the work the SLA bench measures (`"ed"` over 10,000: 2.13 ms plain
against 9.21 ms rich).

| experiment | result |
|---|---|
| Swap the matcher for a different crate | **Not attempted, and should not be.** `compass-search` already uses `nucleo-matcher`, the fzf-class matcher from Helix. It is the right crate. |
| Subsequence prefilter before the alignment | **Rejected — 1.8× *slower*.** Folding each haystack char through `chars::normalize` costs more than the alignment it skips (2.13 → 3.97 ms plain, 9.21 → 16.6 ms rich). An ASCII fast path recovered it to ~9% better than baseline, which is not worth the parity surface. nucleo already prefilters internally; this was duplicating its work. |
| Score across rayon's pool | **Adopted — 1.8–2.7× faster, with identical output.** |

The pool wins at every corpus size on four cores, with no crossover where its overhead dominates,
which is why it is the default path rather than an opt-in:

| corpus | sequential | parallel |
|---|---|---|
| 200 (a typical desktop) | 77.1 µs | 42.5 µs |
| 757 (the Bluefin harvest) | 291 µs | 126 µs |
| 2 000 | 778 µs | 335 µs |
| 10 000 (the SLA's number) | 3.89 ms | 1.44 ms |

**Identical, not equivalent.** Ranking order is a contract — §8.1's Suite 0 diffs ranked output
against the C++ engine — so `tests/parallel_equivalence.rs` compares the two implementations
element for element across 18 query shapes on a corpus built with deliberate score ties, and
`rank_indices_sequential` is kept public precisely so the parallel ranker has something to be
checked against. Removing the index tiebreak from the parallel merge makes query `"f"` diverge at
rank 0, so the control fires.

Two measurement errors were made getting here, both worth recording because both produced
confident wrong numbers:

- **Debug builds are meaningless for this.** The first run reported 26 879 µs and looked like a 13×
  SLA violation. In release the same code is 1411 µs — nineteen times faster. The test now asserts
  the real budget only when optimised, and a loose ceiling otherwise, rather than skipping silently.
- **A "p99" over 100 samples is the maximum.** `timings[100 * 99 / 100]` is the last element, so the
  statistic was the single worst sample of the run. Two consecutive release runs then read 1411 µs
  and 2800 µs, which looked like a flaky SLA and was a flaky statistic. A thousand samples puts ten
  above the p99, and the spread above narrowed accordingly.

#### IME and screen readers — Phase 1's "watch for", answered

Issue #4 flags both with a deadline: "Test both here, not in Phase 5 — if Iced can't do them,
ADR-0001 needs revisiting while that is still cheap." Nothing tested either, so the risk was carried
rather than resolved. Both are now answered, and **the answers differ**.

**IME works, end to end, and is now pinned by tests.** The chain exists at every layer:

* `winit` 0.30 implements `zwp_text_input_v3` on Wayland
  (`platform_impl/linux/wayland/seat/text_input/`) and emits
  `WindowEvent::Ime(Enabled | Preedit | Commit | Disabled)`;
* `iced_winit` 0.14 converts those to `Event::InputMethod` and drives `set_ime_allowed`,
  `set_ime_cursor_area` and `set_ime_purpose` from `enable_ime`, which runs when a widget asks for
  an input method — so a focused search field turns the IME on by itself;
* `iced_core` carries `InputMethod` and `Preedit`.

`app.rs`'s `ime_tests` drive `Event::InputMethod` through the real widget tree: a committed
composition reaches the query, and an uncommitted pre-edit does not. **So ADR-0001 does not need
revisiting on this axis.** The tests exist because that is a claim about libraries, and libraries
change.

The harness needed a control and the control earned its place immediately. The first version
asserted that a commit reached the query and *failed* — which reads like "Iced cannot do IME". It
was not: the simulator does not run `Task`s, so `focus_search` never ran and the field was
unfocused. A plain-typing control failed in exactly the same way, which is what identified the
harness rather than the input method. Both now click the field first.

**Screen readers are a different answer: there is no accessibility tree at all.** Neither `iced`
0.14 nor `winit` 0.30 depends on `accesskit`, and the workspace's `Cargo.lock` contains **zero**
occurrences of `accesskit`, `atspi` or any AT-SPI binding. Orca — the screen reader GNOME ships and
enables by default for its users — has nothing to read: not the query field, not the result list,
not the selected item.

This is the case #4 wanted found early, and it is found. It is **not** a bug to fix in passing:
adding an accessibility tree means AccessKit support in Iced (upstream work) or an AT-SPI
implementation of our own, and the choice between waiting, contributing upstream, and accepting the
gap for now is exactly the kind of decision ADR-0001 exists to record. Flagged here rather than
decided.

#### Cold start — reported from the VM tier, and not the number the SLA names

`packaging/vmtest/checks.sh launcher-start` now times three points: spawn to
process, process to `Adapter AdapterInfo`, and the total.

**It is deliberately not the SLA.** "Cold start to first frame" needs a frame,
and ADR-0010 settles that nothing inside the guest can observe one — the paint
gate lives on the host with corral's screenshots precisely because the
framebuffer's only observer is on the far side of QEMU. What the guest can see
is the renderer choosing an adapter, which wgpu reports only once it has a
surface. First paint follows shortly after.

**Reported, not gated**, for the reason §11.2 gives for RSS. Under llvmpipe on
an emulated GPU the spread is enormous: ADR-0010 records wgpu initialising 2.4 s
into one run and not yet touched 8.1 s into another. A 120 ms budget checked
there would be measuring QEMU, and gating on it would turn the tier red for
reasons unrelated to the code.

The split is the useful part. Spawn cost is Flatpak and process start; render
cost is wgpu bringing up a software adapter. Only the second is what the SLA is
about, and only the first would shrink on real hardware — so the two numbers
are worth having separately rather than as one total that hides which is which.

#### Peak RSS — the index costs 15.5 MB of the 150 MB budget

`crates/compass-core/tests/index_memory.rs`. An index of 10,000 generated
desktop entries, measured as the `VmHWM` delta across the build:

| | |
|---|---|
| peak RSS growth | **15.4–15.6 MB** across release and debug |
| per entry | ~1620 bytes |

Stable to within 1% over repeated runs and near-identical between profiles,
which is what one would expect of memory and is worth stating because the
timing rows above are nothing like that stable.

**That test does not evaluate the SLA, and says so.** The row is "10k index
+ **3 extensions** < 150 MB", and when it was written the extension host did not
exist. It asserts a loose 100 MB ceiling rather than the 150 MB SLA, because
asserting the SLA there would quietly convert a whole-system budget into an
index-only one and report it met.

##### The other half now exists, and the row is missed by 1.7x

`crates/compass-worker-host/tests/peak_memory.rs`. The host drives the real
runtime — the bundle the C++ engine ships as `vicinae-worker-ts` — so three
extensions can be loaded and measured alongside the index:

| | |
|---|---|
| index, 10,000 entries | **15.7 MB** (agrees with `index_memory.rs`'s 15.4–15.6 MB) |
| extension 1 / 2 / 3 | **82.1 / 82.0 / 82.1 MB** |
| **total** | **261 MB against a 150 MB budget** |

##### The 261 MB figure over-counts, and the real number is 175 MB

**Summing RSS across processes triple-charges the interpreter.** Three node
processes share its text pages, and RSS bills every one of them in full.
Measured with `smaps_rollup`:

| | one node alone | three concurrent, each |
|---|---|---|
| Rss | 43 104 kB | ~41 000 kB |
| **Pss** | 41 316 kB | **~18 500 kB** |
| Shared_Clean | 2 308 kB | **~34 700 kB** |
| Private_Dirty | 6 340 kB | 6 340 kB |

Counting private memory in full and the shared mapping once gives **175 MB**,
not 261 MB. Still a miss, but 1.14× rather than 1.7×. `peak_memory.rs` now
prints both and explains the difference rather than leading with the inflated
one.

##### Where it actually goes — and it is not node's baseline

Decomposed by `smaps_rollup`, three processes running concurrently so shared
pages are genuinely shared:

| | private | note |
|---|---|---|
| idle node | 6.4 MB | the interpreter's own dirty pages |
| \+ one empty `worker_thread` | 16.0 MB | **a second V8 isolate costs ~9.6 MB** |
| a real loaded worker | 39.6 MB | **the bundle and API add ~23.6 MB** |

So node's much-quoted 44 MB is mostly *shared, file-backed* and paid once. Heap
tuning is a dead end: `--jitless`, `--max-semi-space-size=1` and
`--max-old-space-size=64` together move the baseline from 45.4 MB to 45.0 MB,
because the V8 heap is only 5.5 MB of it.

##### The fix: the runtime already multiplexes and the host is not using it

`extension-manager/src/index.ts` keeps `workerMap: Map<sessionId, WorkerInfo>`
and spawns `new Worker(__filename)` per session — **many extensions, one
process, one isolate each.** The `session_id` the load reply carries exists for
precisely this. The host spawns a fresh node process per command anyway, so
node's fixed cost is paid three times instead of once.

`crates/compass-worker-host/tests/multiplex_memory.rs` measures both
arrangements back to back, and the result is stable to ±0.1% across runs:

| arrangement | footprint |
|---|---|
| three processes — what the host does today | 159.4 MB |
| one process, three sessions — what the runtime is built for | **108.0 MB** |
| | **saves 51 MB, 32%** |

With the index's 15.8 MB that is **124 MB against the 150 MB budget — inside
it.** The marginal cost of a fourth extension falls from ~53 MB to ~19 MB.

**This is not a redesign.** It is using the runtime as written; the host's
one-process-per-command spawn is the part that has to change, and the protocol
already carries what it needs. What the change does cost is a shared failure
domain: three worker threads in one process die together if the process does,
where three processes do not. `index.ts` already treats a worker exiting
unexpectedly as a crash and reports it per session, so the reporting path
exists — but the blast radius is a real trade and belongs to whoever owns
Phase 4, not to this test.

The row stays **reported, not gated**, for the ADR-0010 reason the cold-start
figure gets: measured once is not a threshold, and gating now would redden
every PR over a pre-existing condition no PR caused.

**The measurement had a false green in it, and the control found it.** Summing
only the pid the host holds reports 1776 kB per extension and a 21 MB total —
comfortably *inside* the SLA. The pid the host holds is `timeout`'s, not node's,
so the real worker was never being read. The test walks the process tree for
this reason, and carries a 16 MB per-extension floor to catch the mistake
returning; 16 MB is a third of an empty interpreter, so it fails a broken probe
without policing memory. The earlier 1 MB floor would have let the false green
through, which is the same defect as the VM tier's RSS gate reading `0 kB` and
calling it lean.

#### The IPC row

**The way that one was untrue is worth recording separately.** `benches/ipc_bench.rs` timed a closure that created a Tokio runtime,
bound a listener, **slept 10 ms**, connected a client, sent one request and tore it all down. It
reported **11.9 ms** against a 0.5 ms SLA — a 24× miss on a stated gate, sitting in a benchmark
nobody had read, because a criterion bench prints a number and exits zero whatever it says.

The tell was in its own output: `concurrent_1` 12.03 ms against `concurrent_100` 13.08 ms, so
ninety-nine extra in-flight requests cost about a millisecond between them. The per-request cost was
always small; the harness was measuring its own scaffolding.

With setup hoisted out of the timed region, one request on an established connection measures:

| | |
|---|---|
| p50 | 30.6 µs |
| p95 | 36.8 µs |
| **p99** | **47.9 µs** |
| max | 57.1 µs |

**The SLA is met with about 10× headroom**, and it is now asserted rather than printed:
`crates/compass-ipc/tests/roundtrip_budget.rs` fails the build if p99 crosses 500 µs. Control-tested
by tightening the bound below the real p99.

Two things this does **not** establish. The SLA says *inside Flatpak* and this is a host
measurement, so it is necessary evidence and not the whole gate. And it measures `Ping`, the
cheapest request there is; a `Query` round-trip over a real index is a different number that nothing
yet records.

Plus `insta` snapshot tests rendering views to a headless framebuffer. Keep these *few* and
semantic (results list, empty state, detail view, form). Large pixel-snapshot suites get
rubber-stamped and stop catching anything.

### 8.5b Suite 4b — head to head against the C++ engine

**Baseline clarification:** the requested baseline is the latest upstream release,
now pinned to Vicinae v0.29.0 (`c3415a3ed56676d2960d90975ab319ae8a7aba6e`), not this
fork's C++ artifact. That unmodified release has no root-query IPC endpoint.
The new `compass-testkit` `head-to-head` binary measures persistent ping,
instrumented queries when available, and process-tree RSS/PSS, retaining raw
samples and explicit exclusions. See [HEAD-TO-HEAD.md](./HEAD-TO-HEAD.md) for its
contract and the remaining comparable-workload requirements. The historical
query/CLI claims below apply to our instrumented C++ fork, not pristine upstream.

Tracked as #117. This began because §8.5 measured budgets, not the engine being replaced.
The [first three upstream runs](./benchmarks/2026-09-20-upstream-v0.29.0/README.md) now
measure warm ping: Rust's median was lower in all three. Search speed remains unmeasured
against upstream, and the recorded memory readings are not feature-equivalent. A 2.0 ms
fuzzy-search SLA still says nothing about whether users will feel the port as an improvement
or regression; the C++ engine is their baseline.

Suite 0 asks *"same results?"*. Suite 4b asks *"at least as fast, in no more memory?"* — same
corpus, same harness shape, different question. It is the evidence the Phase 7 cutover needs, and
it should be green before that phase starts rather than reconstructed afterwards.

#### The second engine already exists as a CI artifact

`.github/workflows/cpp-on-target.yaml` configures the C++ engine against Bluefin's actual Qt,
builds it on the target image, and publishes `vicinae-cpp-bluefin.tar.gz`. Its own closing step
states what it unblocks: the second engine Suite 0 has been missing, with layering it into the VM
image named as the next step.

So the prerequisite is not a Qt build — that exists. It is (a) giving `cpp-on-target.yaml` a
trigger other than `workflow_dispatch`, and (b) layering the tarball into the VM image. **Both are
shared with §8.1's outstanding parity work, and are done once for both.**

#### What can honestly be compared

Both engines expose a comparable CLI. The overlap is the action set:

| action | timed from → to | comparable |
|---|---|---|
| `ping` | request → reply, established connection | yes |
| `query` | parse → rank → serialise → reply, over the 757-entry corpus | yes |
| `launch` | request → child spawned | yes |
| `toggle` / `show` | request → the window's *answer* | partly — not a paint |
| `close` / `hide` | request → the window's *answer* | partly — not a paint |
| idle RSS | resident set with the window open, inside the Flatpak | yes |
| **cold start** | — | **no: excluded** |

**Cold start is excluded rather than fudged.** ADR-0015 made the Rust window resident. If the C++
engine spawns per summon, the two are answering different questions and the Rust engine "wins" by
architecture rather than by speed. What users feel is summon, and summon is comparable.

**Nothing inside the guest can observe a frame** (ADR-0010), so the `toggle`/`close` rows end at
the window's answer, not at a paint, and the harness must say so in its own output rather than let
a reader assume otherwise. For the same reason the comparable set stops at the engine boundary:
llvmpipe distorts anything GPU-bound, while ranking and IPC are CPU-bound and fine in the VM.

Fairness is a property of the harness, not an intention:

* the **same corpus** for both, `crates/compass-testkit/corpus/desktop-entries`;
* the **same warm state** — connection established and index built *before* the timed region. The
  `compass-ipc` bench learned this the hard way, reporting a 24× miss because it timed its own
  setup;
* **interleaved** A/B/A/B runs rather than all-A-then-all-B, so machine drift does not land on one
  engine;
* **medians gated, tails reported**, the same argument §8.5 already settled for the fuzzy row.

#### The gate is a second step, deliberately

Per ADR-0010 a threshold is measured before it is invented, so the first deliverable is recorded
numbers and the gate follows from them. Shape, to be confirmed against the measurements rather
than assumed: gate the median per action at `rust <= cpp`, allow a tolerance derived from the
observed run-to-run spread rather than a round number picked for comfort, and fail by **naming the
action** so a red bench is never a mystery. Actions recorded as not comparable are excluded
visibly, never dropped quietly.

**If an action cannot be made to match or beat, that is a finding and it gets recorded here** — the
same treatment the peak-RSS row gets for missing its budget. A bench that can only report good news
is not a bench.

### 8.6 Suite 5 — Packaging and deployment

- **Bluefin end-to-end, the headline test:** install the Flatpak on a Bluefin image in CI, bind the
  hotkey through the portal, launch a host RPM app, a Flatpak app and a Homebrew binary, and assert
  no permission panic. This is our first target — it belongs in CI from Phase 1, not Phase 6.
- Flatpak conformance via `flatpak-builder`; host execution through `flatpak-spawn --host` /
  `OpenURI`.
- `compass doctor --check-only` asserted to detect present **and absent** portals, protocols, DBus,
  sockets and the Shell extension. Test both directions — a doctor that always says "fine" is worse
  than no doctor.
- Install-matrix smoke for AppImage, Arch, Nix (from Phase 6): `--version` + `doctor`.

### 8.7 Pre-flight command

```sh
cargo test --all-targets --workspace
cargo clippy --all-targets --workspace -- -D warnings
cargo bench --bench slas -- --save-baseline pr
npm --prefix src/typescript test
cargo run -p compass-testkit --bin parity -- --cpp <path> --rust <path> --corpus crates/compass-testkit/corpus/desktop-entries
flatpak run org.tunaos.compass -- doctor --check-only
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

**Runner class (#309).** Every tier, the performance gates included, runs on GitHub-hosted runners;
Depot is declined. Benchmark and RSS baselines (Suite 4, #306, #307, #310) are keyed on the runner
label they were measured on — `ubuntu-24.04` x86_64 — and a change of label starts a new baseline
series rather than comparing across machines. Hosted-runner variance is handled by deriving the
tolerance from repeated runs, not by buying quieter hardware.

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
4. Assert over SSH: `compass doctor --check-only`, the IPC socket, the app index against the guest's
   real `.desktop` files.
5. Drive the hotkey path through the console keyboard, screenshotting each step.
6. Upload the artifact directory unconditionally.

**"Assert over IPC, not over pixels" was half right, and the half that was wrong cost a bug.** This
section used to say `--require-paint` was the one pixel assertion worth gating on, everything else
belonging on the IPC socket, because pixel-scraping a desktop is how an e2e suite becomes one
everybody ignores. The tier now gates on six frame comparisons, and the reason is #91: the launcher
drew a search box, accepted no keystrokes, and **every IPC probe passed** — the socket answered, the
process was healthy, `doctor` was content. `launcher-02-typed.png` came back byte-identical to
`launcher-01-open.png`, and only a pixel could say so. Iced delivers typed characters to a
`text_input` holding widget focus and nothing had focused ours; a person clicks the box without
noticing, so only a harness that types finds it.

What the original advice got right is the *reference* it warned against. A screenshot diff against a
stored image does break on every font, theme and Bluefin update, and none exists here. Every
assertion in the tier compares **two frames from the same run** — before against after, each pair
taken seconds apart on one boot — which is immune to all three, because whatever the theme renders
renders identically in both. Two properties make that a gate rather than a vibe: a floor on how much
changed, and `--expect-box`, which fails if the change lands anywhere but our window and so doubles
as "nothing else moved".

The six: the engine starting must paint **nothing**; the launcher window appearing must paint
something; a typed query must reach our field (#91); hiding must return the desktop and summoning
must bring the window back (ADR-0015, and the pair matters — a frame that never changes passes one
and fails the other); and Ctrl+B must open the action panel. That last one is the newest and shows
the discipline the rest were earned by: it ran **recorded, not gated** for two runs first, because
ADR-0010 forbids inventing a threshold before seeing one. Both runs printed 2,697 pixels in the same
box, byte-identical; the frames were then read to confirm the panel — not merely *a* change — had
drawn; and the floor was set at roughly a quarter of the measurement rather than fitted to it, so a
panel with fewer actions still passes while a chord that never arrives (0.00%) fails.

**The thing this tier is still bad at, stated up front.** Under llvmpipe software rendering a GNOME
session is slow and its timing is variable, so any assertion phrased as "within N seconds" will
flake; phrase them as "after this marker appears".

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
| **Does Qt ever actually leave?** | Yes — Linux-first becomes a *sequence*, not a scope limit; macOS and Windows get committed phases 9 and 10, and the platform seam is built before Phase 4 | [0013](./adr/0013-qt-leaves-the-repository.md) |
| Does browser control belong in the core? | No — it becomes an extension and leaves the port's scope entirely | [0008](./adr/0008-browser-control-is-an-extension.md) |
| **Port or new launcher?** | New launcher in Vicinae's spirit: absolute quality tests, C++ as tripwire, crates first; storage is Compass's own and Vicinae data is imported — supersedes ADR-0014 | [0017](./adr/0017-a-new-launcher-not-a-reimplementation.md) |
| **When do the `vicinae` identifiers go?** | At the Phase 7 rebrand: `org.tunaos.compass`, the `compass` command, `~/.config/compass` with a one-time move of the `vicinae` directories; `@vicinae/api`, `vicinae://` and the Vicinae Store stay | [0020](./adr/0020-phase-7-rebrand.md) |

### Still genuinely open

1. **Team size.** The schedule in §7 swings between four and seven months on this work alone. Nobody
   can answer this from inside the plan.
2. **rustcast relationship** — one-time seed (what the plan assumes and what the crate split
   reflects), or an ongoing sync? The latter would constrain the crate boundaries in §2 and cost
   design freedom. Assumed one-time until someone says otherwise.
3. ~~Whether to report the six C++ desktop-entry bugs upstream.~~ **No** (ADR-0018): a hard fork
   does not report back. The bugs stay recorded as declared divergences in PARITY.md.

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

**Sixteen crates, 828 tests, and an engine that runs.** Counts verified against the committed tree
rather than a dirty one — three commits early on built only because the working tree supplied files
they had not committed, and that is checked rather than assumed.

Each crate's figure below is what `cargo test -p <crate> -- --test-threads=1` reports, doc-tests
included, and **they sum to the total** — a property a reader can check with one command, which is
the point of stating them. Several had drifted below the tree (`vicinae` read 137 against a real
166) because they were maintained by hand while the total was recomputed; all sixteen were
re-measured rather than adjusted.

- **Workspace and CI.** Pinned 1.94.1, edition 2024, `unsafe_code` forbidden and `clippy::all`
  denied workspace-wide. Rust CI workflow, Makefile targets kept separate from the C++ ones. All
  workflows migrated off Depot onto GitHub-hosted runners.
- **Corpora.** 8 real `.desktop` entries plus 19 synthetic edge cases, and a harvester for growing
  the real half on a machine that has applications installed. Corpus files are `-text` in
  `.gitattributes`, with a test that fails loudly if a checkout ever normalises the CRLF and
  Latin-1 fixtures into fixtures that test nothing.
- **`compass-xdg`** (118) — desktop-entry, locale, value, reader and exec layers, with all 47
  in-scope C++ cases ported verbatim.
- **`compass-search`** (59) — fuzzy matching on `nucleo`, with the C++ ordering suite ported and
  fzf's coherence signal reconstructed exactly (ADR-0006).
- **`compass-ipc`** (74) — length-prefixed postcard framing, with the length checked against
  `MAX_FRAME_LEN` before any allocation.
- **`compass-core`** (74) — app index with desktop-ID precedence, frecency, `compass.json`.
- **`compass-shell`** (47) — GNOME Shell DBus client; 22 of its tests spawn a real `dbus-daemon`.
- **`compass-portals`** (55) — XDG portals via `ashpd`, with availability a three-state outcome
  rather than a boolean, version-property probing, and a timeout on every call.
- **`compass-extension-api`** (74) — the view tree, derived identity, diffing, dispatch and the
  capability registry, behind a mechanical seam gate that fails if host transport or runtime is
  named anywhere in the crate. The gate was itself tested by injecting a violation.
- **`compass`** (180) — CLI, an 11-check `doctor`, and **`compass serve`: the engine**. It
  indexes applications, ranks queries with frecency and answers over the IPC socket. It holds no
  window of its own and never opens one; `show`, `hide` and `toggle` are forwarded to a **resident
  launcher window** that attached over the same socket
  ([ADR-0015](./adr/0015-the-launcher-window-is-resident.md)), and refused when none has. So a
  client can still tell "no window" from "the window was shown". Fifteen end-to-end tests spawn the
  real binary on its own socket with every XDG variable pointed into a tempdir; four of them attach
  a fake window from the test process and assert across the process boundary.
- **`compass-ui`** (22) and **`compass-wayland`** (2) — the Iced launcher shell and the Wayland
  surface under it. `compass-ui` is now **resident** (ADR-0015): it runs on `iced::daemon`, opens
  and closes its window on command, and reports the state it ended in. That state machine is
  testable with no display and is, which is where the 11 new tests came from. Everything that
  actually draws still needs a compositor, which is why the VM tier exists — read the numbers as
  "the logic is covered, the rendering is not".
- **`compass-testkit`** (8) — corpus loader; entries expose raw bytes, not `String`.
- **`compass-crypto`** (24) — the clipboard's AES-256-GCM and its HKDF key derivation, ported from
  `aes-gcm.cpp` and `database-key.cpp`. CI cross-decrypts against the real C++ implementation in
  both directions, which is the right check for randomised-IV crypto where a byte diff would fail
  on a *correct* port.
- **`compass-platform`** (6) and **`compass-platform-linux`** (2) — the platform seam ADR-0013
  requires before Phase 4. `compass-platform` names what a launcher is and has **zero** Linux
  dependencies; the implementation moved out. A manifest test fails if a crate shared by every
  platform takes a dependency on a Linux-specific one.
- **`compass-sqlcipher-sys`** (7) — opens every Compass database as a `rusqlite::Connection` over
  SQLCipher (`bundled-sqlcipher`), keyed, with the `fuzzy_trigram` FTS5 tokenizer from `vendor/`
  (ADR-0014) registered and the C++ engine's pragmas applied. The one crate that declines the
  workspace's `unsafe_code = "forbid"`, because tokenizer registration is FFI on a raw `sqlite3*`;
  it restates every other workspace lint so the exception is visible as a missing manifest line.
- **`compass-clipboard`** (76) — **`clipboard-db.cpp` ported in full**: query planning, the schema
  and migrations, the paginated read, and the whole write path. Four C++ bugs fixed rather than
  reproduced, each pinned by a control that fails when the original shape is restored. The layer
  above it, `clipboard-service.cpp`, is still C++.
- **ADRs 0001–0014.**
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
| Suite 0 parity for app-search ranking on the **500-entry corpus** | 🟢 **corpus met; top-1 ranking parity met** | **757** entries, past the 500 the gate names. The engines pick the **same top result on 100% of queries** (920 of them contested), and the same top 3 on 97.2%. Scores differ on 20.8% — the declared nucleo-vs-fzf divergence — but the ranking absorbs it (§8.1a). Full-order parity is 84.4%. |
| Runs from a Flatpak on Bluefin with **GNOME 50 and 51** | ✅ **met as reworded by ADR-0018**: GNOME 50 and 51 in Suite 3b, Bluefin at the version it ships in the VM tier | **GNOME 50 and 51 are both covered**, by Suite 3b (`tier2.yaml`, #124): the shipped Flatpak bundle is installed and exercised against a real Mutter compositor on `fedora:44` (GNOME 50, 62 s) and `fedora:45` (GNOME 51, 70 s), per PR, with `prove-smoke.sh` as the control. **On Bluefin specifically**, the VM tier boots `ghcr.io/ublue-os/bluefin:stable` and runs it there on GNOME Shell 50.3. The earlier caveat — *"not checked: whether a non-stable Bluefin tag or a different Fedora base carries 51 today"* — **has now been checked, and none does.** All 441 named tags in `ghcr.io/ublue-os/bluefin` collapse to 44 stems; every Fedora-based one is Fedora 44: `stable` and `gts` are `44.20260915` and resolve to the *identical* config digest (`sha256:1708919d…`, the same image twice), `latest` and `44` are `latest-44.20260908.1`, `stable-daily` is `44.20260915`. The remaining stems (`lts*`, `stream*`, `10*`) are the CentOS Stream 10 line, not a newer Fedora. There is no `45` stem. So the criterion's *first* half is met and its *second* half is unsatisfiable until Bluefin builds on Fedora 45 — at which point `:stable` follows it and the VM tier tightens with no edit. Rewording proposed on #4. |
| **Idle RSS < 30 MB** | ✅ **engine met (6.2 MB, gated at 20); the resident window's hardware figure moved to #127 by ADR-0018** | the window idles at ~135 MB under llvmpipe and that was read as five times over budget. Most of it is wgpu's software renderer, which lives in that process's RSS in a VM and not on hardware. **The engine — the part that is actually resident, holds the index, serves IPC and draws nothing — idles at 6.2 MB**, measured on an ordinary container outside any VM. `launcher-rss` now reports both, labelled. Still reported rather than gated: a threshold set from a software-rendered number would be fiction. |
| **Works with no Shell extension installed** | ✅ **met** | we ship none at all (ADR-0004), the VM has none, and `doctor` records `gnome.shell-extension` as evidence rather than gating on it. |

**A correction to this section's own first draft.** It said the corpus was the
binding constraint on Phase 1. That is not right, and reading §8.1 properly is
what showed it. Suite 0 is a **differential** harness — "run the operation
against both engines and diff structured output" — so the gate needs three
things, and the corpus is only one:

| Suite 0 needs | state |
|---|---|
| the desktop corpus | **757, past the gate's ~500** — met |
| a runner that diffs the two engines | **exists as a file, and does not yet work** — see below |
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

**It is green, and the answer is yes.** CMake reaches *Configuring done /
Generating done* against Fedora's Qt 6.11.2, so every `find_package` in the tree
— `Qt6 6.9 REQUIRED`, `src/server`'s eleven components including `GuiPrivate`,
ECM, KF6SyntaxHighlighting, LayerShellQt, OpenSSL and X11 with its xcb
components — resolves on the target.

Two things it found along the way:

- **Catch2 is the one gap.** Fedora 44 ships 2.13.10 and the tests require
  Catch2 3. There is no v3 package — checked against Fedora's package database,
  not assumed.
- **Four red runs is a poor way to enumerate a dependency list.** Three of them
  each found exactly one missing package, and the last was avoidable from text
  already quoted in the job's own comment: the `USE_SYSTEM_QT_KEYCHAIN` option
  says *"Note: still depends on system libsecret"*. The list is now derived from
  the tree's `find_package` calls and lives in
  `scripts/runners/bluefin/install-deps.sh`, next to the Arch one, so the build
  job and the configure job cannot drift apart.

The fix is `-DBUILD_TESTS=OFF` and it is the right answer rather than a
workaround: Suite 0 diffs engine *behaviour* through
`compass --engine=cpp --json`, not by running the C++ unit tests, and those
already run on Arch in `build-linux.yaml` where Catch2 is v3. If they ever need
to run on Bluefin, Catch2 3 can be vendored through `FetchContent` exactly as
qtkeychain, layer-shell and cmark-gfm already are.

Two further things this scoring makes concrete, which "the gate cannot be
evaluated" hid:

- **The corpus is a real constraint, just not the binding one.** 115 entries
  against a gate that names 500 is a genuine distance, and one Bluefin image
  yields 88 — re-running the same job yields the same 88. An earlier revision
  concluded from that that closing the gap "needs different machines". **That
  was a misreading**: the constraint is the APP SET, not the hardware, and the
  harvester's own closing note says so — run it on more machines *"or install
  more Flatpaks"*. Only half that sentence got read.
  `.github/workflows/corpus-harvest.yaml` acts on the other half.
- **The RSS figure is an upper bound, not the shipping number.** It is taken
  under llvmpipe, where the renderer keeps buffers it would not need on
  hardware. It is reported rather than gated for the same reason the paint
  deviation was: one sample is not a budget, and a memory gate set from a single
  software-rendered run would be the deviation mistake again in a different
  costume. It goes in the log so the gate can be set from a distribution.

### 11.3 Phases 2 and 3, evaluated

§11.2 scored Phase 1 because the launcher finally existed to score. The same
is now true one and two phases further on, and the answer is further along than
§12's ordering implies: `compass-shell`, `compass-clipboard` and
`compass-crypto` are built and tested, so these gates can be read against
evidence rather than deferred.

**Phase 2's gate**

| Criterion | State | Evidence |
|---|---|---|
| IPC round-trip **p99 < 0.5 ms** | 🟢 **met** | **47.9 µs**, ~10× headroom, asserted by `crates/compass-ipc/tests/roundtrip_budget.rs` rather than printed. §8.5 records how the previous benchmark reported 11.9 ms by timing its own setup. |
| `doctor` diffed against the C++ build | ⚪ **withdrawn, with reasons** | the C++ engine has no `doctor`. §6 sets out why the diff would mostly prove nothing even if built: nine of eleven checks probe the *environment*, which two processes on one machine agree about by construction. |
| `doctor` reports each degradation with the extension uninstalled | 🟢 **met** | the VM has no extension, and `checks.sh doctor` plus `doctor-assert` run inside a real GNOME session every tier run. `gnome.shell-extension` is recorded as evidence rather than gated, which is the honest shape for a capability we deliberately do not ship (ADR-0004). |

So **Phase 2's gate is met**, once the withdrawn criterion is read as §6 restates
it: that `doctor`'s picture of the machine is accurate, tested non-differentially
against reality.

**Phase 3's gate**

| Criterion | State | Evidence |
|---|---|---|
| mock-Shell-bus suite green | 🟢 **met** | `crates/compass-shell/tests/mock_bus.rs` — **21 tests**, plus 12 in `contract_introspection.rs` over the versioned interface XML, and 13 unit tests. |
| clipboard DB readable and writable by **both engines interchangeably** | 🟡 **the crypto is cross-verified; the database file is not** | **An earlier revision of this row said no cross-engine test existed at all. That was wrong, and `src/lib/crypto/probe/main.cpp` says so in its own header.** The crypto half is genuinely interchangeable and checked per-PR: the probe speaks a request/response protocol and the driver uses it for **cross-decryption — C++ encrypts and Rust decrypts, then the reverse** — deliberately rather than byte-diffing, because the IV comes from `RAND_bytes` and two correct implementations differ on every call. `deriveKey` is deterministic and is diffed directly, and the tamper control asserts the specific `AuthFailed` rather than "it errored". On top of that the stored contract is pinned against the C++ source by `cpp_enum_values.rs` and `cpp_constants.rs`, and our own side has 76 tests. **A second correction, in the other direction: the structural layer is in better shape than the first two revisions of this row said.** Going to look turned up that `compass-clipboard`'s `MIGRATIONS` does not *copy* the C++ schema — it `include_str!`s the very files the C++ engine compiles in as Qt resources (`src/server/database/clipboard/migrations/001_init.sql` and `002_trigram_fts.sql`). There is one copy of the DDL, shared, so the tables, indexes, triggers and the FTS tokenizer cannot drift by construction. The `schema_migrations` contract is ported deliberately down to MD5 checksums — *"a port that wrote SHA-256 there would make every existing row unreadable to the other engine"*. The connection pragmas are now pinned too, by parsing `CLIPBOARD_PRAGMAS` out of `clipboard-db.cpp` and comparing in order, because `journal_mode` is a property of the *database* rather than the connection and `foreign_keys` decides whether one engine orphans rows the other would refuse to. **So crypto, DDL, stored enums, crypto constants and pragmas are each shared or pinned.** What is genuinely left is only the end-to-end artefact: a database file written by the C++ *binary* and opened by Rust. That is a smaller and much more specific thing than "the database file is not cross-verified", which is what this row said twice. |
| extension-absent and version-mismatch paths both tested | 🟢 **met** | the capability probe treats a bus error as an absence rather than a failure (`probe_errors_are_an_absence`), and the versioned contract is introspected rather than assumed. |
| a week of dogfooding by ≥2 people on Bluefin | 🔴 **not started** | needs people, not code. Nothing in CI can stand in for it, and it should not be quietly reinterpreted as something that can. |

**And a dangling reference, which is the third of its kind — cited twice.**
Phase 3's gate cites *"the mock-Shell-bus suite (§8.4a)"*, and
`src/lib/crypto/probe/main.cpp` opens by citing §8.4a as well. **There is no
§8.4a.** The suite
exists and is green, so the gate is satisfiable — but its citation points
nowhere, exactly as Suite 0's gate cited a `compass --engine=cpp --json query`
that never existed (§8.1a) and Phase 2's cited a C++ `doctor` that never
existed. Three gates written against an imagined artefact is a pattern worth
naming: **a gate that cites something should be checked against the thing it
cites, at the time it is written.**

**What actually remains on the Linux path**, with the phases above scored:

| | |
|---|---|
| Phase 1 | GNOME 51 — **blocked upstream**: Bluefin stable is 50.3 (§11.2) |
| Phase 2 | met |
| Phase 3 | one end-to-end artefact test (a DB written by the C++ *binary*, opened by Rust) — crypto, DDL, enums and pragmas are already shared or pinned; dogfooding |
| Phase 4+ | `compass-extension-api` exists at 5.5k LOC and 73 tests; the Node host is the open half |

### 11.4 Phases 4 to 10, evaluated

Scored the same way, and the answer is short: **the Linux path is close to done
through Phase 3, Phase 4 has a spine and not much breadth, and Phases 5 onwards
are unstarted.** Saying so with numbers is more useful than a phase list that
reads as uniformly in-progress.

| Phase | Gate | State | Evidence |
|---|---|---|---|
| **4 — Extension host** | Suite 1: top 25 Raycast store extensions plus every Vicinae one, running | 🟡 **spine built, breadth and the gate not** | the prerequisite carve-out is done (`compass-extension-api`, **5,546 LOC, 73 tests**), and the host now exists: `compass-worker-host` (**8,610 LOC, 170 tests**) frames, spawns, speaks the manager and tsapi protocols and routes a session; `compass-sandbox` (**1,280 LOC, 23 tests**) confines it; `compass-local-storage`, `compass-oauth-store` and `compass-db` back the two host APIs that are storage. **44 of tsapi's 49 methods** are implemented, the gate's extensions have never been run, and the transport is stdio, which §6 now names after this was reconciled — see §11.4a and #101. |
| **5 — Breadth, second compositor** | parity ledger ≥ 95% green | 🟢 **100%** | `PARITY.md` holds **152 ✅, 0 ❌, 0 🟡** over the 152 cells of the two columns that measure this port — `Rust ✓` and `parity test ✓`, across 87 rows — plus 22 marked n/a (the figures below this sentence are the history of how it got here). Counted by `scripts/ci/parity-score.py`, which also prints the other two columns. **The earlier 37% was wrong, and wrong in our favour.** It was taken over all four checkbox columns, which meant counting `C++ ✓` — 87 rows, every one of them ✅, because that column says the C++ exists, not that anything was ported. Those 87 free greens were three quarters of the "120 ✅" the figure was built on. It also counted `C++ deleted ✓`, which by this ledger's own rule cannot go green before Phase 8. Restating over the two columns that are Phase 5 work puts the real figure at 70 of 158. Nothing regressed to cause the drop from 37% to 35%; the earlier number was measuring the wrong thing. Ported rows have since carried the corrected figure back up past it, which was a coincidence of arithmetic and not a return to the old method: the corrected figure is 70 of 158 over two columns, the old one was 120 of 331 over four. Of the 75 it then held, only 18 rows were green in `Rust ✓` — the rest are rows with a passing parity test over a model that has no view yet. (Earlier revisions said 115 of 331 and 96 of 340 on the same inflated basis.) This remains the single largest number in the project. It was described here as "a breadth problem rather than a hard one: most rows are individual builtins", and that has stopped being true — the builtins are ported. `scripts/ci/parity-score.py` now reports what the remainder *is*, by reading the `Still C++-only:` sentences the notes carry, and at the time of writing it is: **view 12, backend 7, process 2, storage 1, network 1**. Twelve of the nineteen named gaps are drawing, seven are DBus, MPRIS or compositor providers. The view figure has gone *up* as rows landed, which is not a regression: each newly written note names what its row still lacks, and what these rows lack is drawing. One of the changes since is a correction rather than movement: a `Still C++-only:` sentence in the shortcut row had been edited into saying the opposite of what it opened with, and was being counted as a storage gap that no longer existed. None of that is transcription, and most of it cannot be verified in a container — the VM tier is what answers for the drawing, and it runs on this PR rather than only nightly. The number to watch is no longer the percentage on its own but that breakdown beside it: a ledger at 44% whose remainder is typing and one whose remainder is compositor integration are not the same project. The launcher-parity pass (media arguments and Now Playing, dmenu's size, footer and quick look, theme files, the fonts grid, Search Files' filter, preview, recent files, `ShowItems` and fallback, the stores' avatars, dots, README images, dialog and deeplinks, Script Permissions) moved Media, Set Theme and Browse Fonts to `Rust ✓` and, on top of the input server and the last extension host routes, took the ledger from 72 to 75 of 158 (46% to 47%); its breakdown was then **view 7, backend 6, process 2, storage 1, network 1, other 3**. **The ledger truth pass (2026-09-25)** then checked every 🟡 and ❌ cell against the code rather than against its note, because most of what those notes called missing had landed: the file indexer whole, the window manager's GNOME, wlroots, Hyprland and niri providers, the stores' HTTP, the data-control plumbing, the power, media and volume commands, fonts, themes, snippets and the input server. A cell went green only with a named Rust module and named tests that fail on a regression (the evidence table is in `PARITY.md`, "The ledger truth pass"), and five small gaps were closed on the way (the two `xdgpp` writers, `glyph`'s `is_emoji`, `{selection}` in shortcuts, the power commands' `confirm` and `customProgram`, a notification's urgency and file icon). `src/lib/soulver` moved to n/a, being macOS-only like the other macOS rows. That took the ledger from 75 of 158 to **116 of 156 (47% to 74%)**: 42 rows green in `Rust ✓` (from 18) and 74 in `parity test ✓` (from 57). The breakdown is now **view 11, backend 4, process 2, network 2, other 12** over 29 notes. The count of notes went up from 15 because every amber row now has a sentence saying what it lacks, where before about half had none; it is a truer list, not a longer one. `other` is large because the classifier's keywords predate most of these sentences (quit and force quit, eviction, the tray icon, per-command global shortcuts). What the remaining 40 cells are, sized and with what blocks each, is §12.0's first list. The gaps pass that followed (the applications and extension-directory watches, Browse Apps and the default-app pickers, notification icons, the emoji picker's memory, clipboard retention and the detail pane, root favourites and the row's panel, the rest of the CLI, Quit and Force Quit, and the calculator's history; IPC v17) took it to **125 of 156 (80%)**, with 31 cells left. A second gaps pass (drawn builtin, file-type, window and clipboard icons; the tray host over `system-tray` and Search Tray; the provider search view, every fallback kind, the shortcut recorder, the emoji picker's paste and Browse Apps' Focus Window; Switch Workspaces and the window toggles, Open with…, the shortcut detail pane, the file action panel and clipboard Open; IPC v18) took it to **133 of 156 (85%)**, with 23 cells left. The UI pass (root rows' icons, masks and favicons, the snippet pane and escape, match and Markdown highlighting, extension grids) and the settings pass (the settings window's pages as a view of the launcher, every setting Compass reads; IPC v19) took it to **139 of 156 (89%)**, with 17 cells left. The HUD, onboarding and the rest of `builtins/vicinae` (the fallback manager, installed extensions, the storage and OAuth browsers, the stores' intros) closed the view layer, `ui/qml`, `ui/quick` and `ui/windows` included, at **145 of 156 (93%)**, with 11 cells left. The wlroots gaps pass (synthetic paste through the input server or `zwp_virtual_keyboard_v1`, and shortcut inhibition on a connection shared with `iced_layershell`, both verified on headless Sway; `PARITY.md`, "The gaps pass, wlroots paste and inhibit") took it to **147 of 156 (94%)**, with 9 cells left: the news, update and telemetry services and Compass's own tray icon (a product decision each), the calculator's currency rates (a rate source), and global shortcuts, the KDE and X11 window managers and window material (a real compositor, the VM tier). With the global-shortcuts pass (the configuration's shortcuts bound over `xx-hotkey-v1`, `vicinae-hotkey-v1` or the portal, IPC v20) and the KDE provider in the same change, the ledger reached **148 of 156 (95%)**, the Phase 5 gate, with 8 cells left: the news, update and telemetry services and Compass's own tray icon (a product decision each), the calculator's currency rates (a rate source), global shortcuts' X11 backend and the window manager's X11 provider (the X11 decision), and window material (applying blur to the launcher's surface, and a compositor with the protocol, the VM tier). The product decisions of 2026-09-25 (`PARITY.md`, "Product decisions") then settled four of them: the update check is wired (Compass's own GitHub releases, shown in the root search, never installed; IPC v21), news and telemetry are n/a in both columns (a hard fork: no news feed, no telemetry), and Compass is Wayland only, so the X11 backends are n/a — which flips neither the global-shortcuts row (`inhibitApps`, `probeBind`) nor the window-manager row (GNOME's workspace list). That is **147 of 152 (97%)**, with 5 cells left: the calculator's currency rates (a rate source), Compass's own tray icon (a decision), global shortcuts' `inhibitApps` and `probeBind`, GNOME's workspace list (the Shell extension's contract), and window material (the VM tier). The window-material pass (an `unsafe` bridge from winit's raw handles to `wayland-client` proxies, approved as ADR-0019 and kept to one function of one crate, `compass-wayland-foreign`; the blur applied behind a translucent card under the `xdg_toplevel` presentation, verified on headless Sway and an in-process compositor; `PARITY.md`, "The window-material pass: an unsafe bridge") closed the last of them: **152 of 152 (100%)**. What remains is declared rather than counted: real blur on KWin (the VM tier) and the layer-shell presentation, which `iced_layershell` gives no handles for. |
| **6 — Packaging breadth** | Suite 5 green across all outputs | 🟡 **all Linux outputs defined, gate not yet observed** | the Flatpak builds, is installed and is smoke-tested on every run. The Rust engine's AppImage, Arch package and Nix package now exist and are built and smoke-tested by `packaging.yaml` (nightly + packaging changes), sharing the Flatpak's install layout and one smoke script; `compass.json` has a generated, drift-tested JSON Schema and a migration from `settings.json`. None of the three new jobs had run when this was written. The C++-engine AppImage, tarball, dmg and Windows workflows stay `workflow_dispatch` only. |
| **7 — Cutover** | one full release cycle with no P0 regressions | ⚪ **not startable** | requires 5 and 6. There has also been no release cycle: the repository has **no tagged release**. |
| **8 — Remove the Linux C++ engine** | — | ⚪ **not startable** | requires 7. Several tests are written to die with `src/` at this point and say so (`cpp_enum_values.rs`, `cpp_constants.rs`, the new pragma pin), which is the intended shape. |
| **9 — macOS** | — | ⚪ **sequenced, not blocked** | ADR-0013 makes Linux-first a sequence rather than a scope limit. 102 `Q_OS_MAC` sites are inventoried in #78. |
| **10 — Windows, Qt leaves** | — | ⚪ **sequenced** | #79. |

**What this means for "the roadmap", stated plainly.** Phases 0–3 are the
launcher and its foundations, and they are essentially done — the launcher
opens on a real GNOME session, indexes the host's applications, ranks them at
100% top-1 parity with the C++ scorer, accepts typing, hides and summons over
IPC, and idles at 6.2 MB. Phases 4–10 are the *rest of the product*. Phase 4 now has a
working spine — a worker can be spawned confined, a session runs, and a real
Node process has driven a storage call through the host and read it back — but
the phase is 45 of 49 API methods and none of its gate. The rest is 226 unported
parity rows, packaging breadth, a cutover and two further platforms. §7's own schedule puts the whole
sequence at roughly a year.

#### 11.4b What Phase 4 still needs

Ordered by what blocks what, not by size.

| Piece | State |
|---|---|
| framing, manager protocol, tsapi envelope | done, pinned against the IDL and the generator |
| worker lifecycle (spawn, request, read, shutdown) | done |
| Landlock boundary + seccomp denylist + launcher | done, with §8.2's negative list: a program the extension wrote cannot be run (read no longer implies execute), raw and packet sockets answer `EPERM`, and `RLIMIT_DATA` (512 MiB) refuses a 512 MiB `Buffer` where no cgroup is reachable; each beside a positive control (`compass-sandbox/tests/boundary.rs`, `engine_end_to_end.rs`). The systemd scope's `MemoryMax` still applies only outside a Flatpak |
| session routing (event → service → reply) | done |
| `Storage`, the three storage `OAuth` methods, `UI/render` | done — 9 of tsapi's 49 |
| `Wallpaper/set`, `BrowserExtension` (both) | **done**: the engine serves `Wallpaper/set` (`compass::extension_wallpaper`) with the C++'s six Linux backends in its order — hyprpaper, swww/awww, GNOME, KDE, Cinnamon, MATE — and `BrowserExtension` as the C++ answers with no browser connected (ADR-0008: none ever connects). PARITY "The extension host API" #6–#7 |
| `WindowManagement` (all seven) | the adapter is done and pinned (`compass-worker-host::window_service`), behind a `Windows` trait — 30 of 49 — and **the engine serves it** (`compass::extension_windows`): Hyprland's and niri's own IPC first (workspaces, pids, Hyprland's geometry), the Shell extension's windows on GNOME (contract 3 adds each window's frame and full-screen state), the foreign-toplevel list on other wlroots compositors, and `wl_output`/`xdg_output` for screens on any. Moving a window is not served; PARITY "The extension host API" |
| `Command` (all four) | **done** (`compass::extension_commands`): a launch is handed to the launcher window under a token (`WindowCommand::Launch`, IPC v15), which runs the sibling as if picked in root search; preferences open the command's form; the subtitle override shows in root search. PARITY "The extension host API" #8 |
| `Application` (all five) | the adapter is done and pinned (`compass-worker-host::application_service`), behind an `Apps` trait — 19 of 49. `compass-core::AppIndex` and `compass-xdg::mimeapps` already answer most of what the trait needs; wiring them together, launching, and the terminal are still ahead |
| `Clipboard` (all four) | the adapter is done and pinned (`compass-worker-host::clipboard_service`), behind a `Clipboard` trait — 14 of 49. The Wayland backend behind it is Phase 3/5 work and does not exist yet |
| `FileSearch/search` | **done**: the engine's file indexer answers it (`compass::extension_files`). **All 49 of tsapi's methods are now routed by the engine** |
| reading an extension's `package.json` | done (`compass-core::manifest`): commands, modes, arguments, preferences, intervals |
| finding installed extensions | done (`compass-core::manifest::registry`): the XDG search order, shadowing by directory name, staging directories skipped |
| `UI`'s shell half (toasts, HUD, navigation, search text, selected text, desktop notifications) | the adapter is done and pinned (`compass-worker-host::ui_shell_service`), behind a `Shell` trait — 45 of 49. Nothing draws yet, but nothing pretends to either: the calls delegate, they do not no-op. `getSelectedText` reads the primary selection: over data-control on wlroots, through the Shell extension on GNOME (`GetPrimarySelection`, contract 3) |
| `UI/confirmAlert` | done: drawn by the launcher, and settled on confirmation, cancellation, replacement (a second alert) and navigation (the launcher popping, or the extension pushing or popping) |
| `EventCore/handlerActivated` | done: actions, search text and form fields fire it (`Views::activate`) |
| `OAuth/authorize` | done without the overlay: the browser opens on the default https handler, a toast says so, and the `raycast://oauth` redirect comes back through `compass deeplink` (IPC v12) keyed by `state`; the token store is routed in the engine. 46 of 49 |
| remote images, date/tag/file pickers | done: `ureq` into Compass's own image cache; a typed date field, tag toggles, and the FileChooser portal (PARITY "Extension views") |
| running the real `vicinae-worker-ts` | **done for one command**: `scripts/build-extension-runtime.sh` builds figura standalone, generates the protos and bundles `src/typescript/extension-manager`; `tests/real_runtime.rs` loads a real no-view command into it and serves its `Storage` calls, and CI runs that with `COMPASS_REQUIRE_RUNTIME=1`. A view command still needs a front end, and the gate's 25 extensions need far more of the API than `Storage` |
| Suite 1 (the gate) | **the harness runs, and the gate is not met.** `compass conformance` runs installed extensions against an engine of its own and judges each command's first frame; `scripts/suite1/` pins the corpus (top 25 Raycast store extensions by installs that can run on Linux at all, plus all 95 Vicinae store extensions), fetches the stores' own bundles, and ratchets against `expected.json`; `.github/workflows/suite1.yaml` runs it on the host (gating on regressions) and inside the Flatpak (report-only until seen green, judged with the ledger's `flatpak` overrides). First measured run, in the dev container: 71 of 120. **Now 79 of 120 on the runner's ledger** — Raycast 12 of 25, Vicinae 67 of 95 — from host fixes and harness inputs, not from relaxing the judgement: `LocalStorage.getItem` of a missing key resolves `undefined` as Raycast's does (Google Search had crashed on `null`); an extension's view may now exceed the IPC's old 1 MiB frame; `getSelectedText` and all of `WindowManagement` are served by the engine instead of refused; a grid's own empty view counts as drawn, as a list's already did (`player-pilot`, `wallhaven`); `ssh`, `supergenpass` and `google-search` are typed into, as a person would, and `wikipedia` is given a title; and the D-Bus extensions (`bluetooth`, `dbus`, `systemd`) render on the runner, which has a system bus. Of the 41 failures, 7 wait on an OAuth sign-in a headless run cannot give; 4 need a selection or a focused window a headless run does not have (`json-format`, `wiktionary`, `case-converter`, `html-symbol-finder` — the APIs now answer, with the C++'s own "Unable to get selected text" and "No active window"); 2 are the sandbox by design (`speedtest` running a binary it downloaded, `reminders` writing outside its directories); 1 is the heap cap, kept by decision (`dashboard-icons`, PARITY sandbox row 3); 1 is an extension that only knows macOS and Windows paths (`visual-studio-code`); and 26 need a program, file, service, server or account the runner does not have (hyprctl, pactl, mise, goldfish, a SearXNG instance, API keys, GSettings schemas — `gnome-dnd` renders inside the Flatpak). Not measurable here and the largest open question for "running unmodified": the sandbox reads nothing of `$HOME`, so extensions that read the user's own files (`ssh`'s config, `pass`, Firefox profiles) find nothing on a real desktop either (PARITY sandbox row 6). None of it has been seen green inside the Flatpak yet |
| the seam (the gate's third condition) | **done and in CI**: `scripts/ci/extension-api-seam.sh` checks `cargo tree -p compass-extension-api` (normal, build and dev edges) never reaches `compass-worker-host`, then copies the crate into a workspace where the host does not exist and runs `cargo test` there against the same `Cargo.lock` pins. Rust workflow job `extension-api-seam` |

#### 11.4a Phase 4 specified a transport the worker does not speak — resolved

**Resolved: §6 now names stdio, which is what the host already does.** What
follows is the reasoning and the evidence, kept because the *shape* of the
mistake is the lesson rather than the mistake itself.

Phase 4 used to say the host *"spawns `vicinae-worker-ts` per extension **over
UDS with JSON-RPC 2.0**"*, and in the same breath that **`src/typescript/` is
not rewritten** — the reconciler and the `@raycast/api` shim keep working. Those
two sentences were in conflict, because the worker that is not to be rewritten
does not speak UDS.

What `src/typescript/extension-manager/src/index.ts` actually does:

```ts
private async writePacket(message: Buffer) {
  const packet = Buffer.allocUnsafe(message.length + 4);
  packet.writeUint32BE(message.length, 0);      // 4-byte big-endian length
  message.copy(packet, 4, 0);
  process.stdout.write(packet);                  // ... over STDOUT
}
```

and on the way in it reads a `UInt32BE` length, slices that many bytes, and hands
them to `manager.Server`.

So, measured against the running code rather than the design note:

| Phase 4 says | the worker does |
|---|---|
| UDS | **stdio** — `process.stdout` / `process.stdin` |
| JSON-RPC 2.0 | **JSON-RPC 2.0** — the plan is right, and an earlier revision of this section said otherwise |
| — | framing is a **4-byte big-endian length prefix**, not `Content-Length` headers and not newline-delimited |

**A correction, made in the same sitting that introduced the error.** This
section first claimed the payload was "figura-generated RPC, not JSON-RPC 2.0",
reasoning from `import * as manager from "./proto/manager"` and assuming a
binary codec behind it. Reading figura's own code settles it the other way.
`src/lib/figura/src/codegen/typescript.hpp` emits

```ts
jsonrpc: "2.0";
this.sendMessage({ jsonrpc: '2.0', method, params });
this.transport.send(JSON.stringify(msg));
const msg = JSON.parse(data) as JsonRpcMessage;
```

and the glaze backend emits a matching `std::string jsonrpc` with
`glz::raw_json params`. `index.ts` corroborates it from the other end: it does
`packet.toString("utf8")` before routing, which no binary codec would want.

**figura is an IDL that generates JSON-RPC 2.0 bindings**, not a wire format of
its own. So the payload is JSON text and the plan's encoding was never wrong —
only its transport.

`figura/` is the project's own IDL: `manager.fig` and `manager-extension.fig`
define this boundary in 120 lines, and `figura_compile` generates both sides.
`manager.fig`'s own header describes the layering — the manager is *"unaware
what the payload is made of"*, because the payload is a second RPC message from
the `vicinae↔extension` spec (`tsapi.fig`, 357 lines).

**With the encoding settled, what is left to decide is much smaller.** The host
needs JSON-RPC 2.0 — which is off-the-shelf — inside a four-byte length prefix,
over stdio rather than a socket. The `.fig` files define the method names and
payload shapes, so the Rust types can be generated from them or hand-written and
pinned to them, the way three other boundaries in this repository already are.

The one real decision was the transport: **keep stdio**, which is what the
worker does and what `src/typescript/` not being rewritten requires, or add UDS
to the worker, which contradicts that constraint for no capability the host
needs.

**Taken: keep stdio.** §6's bullet now says so, so the spec and the code agree
and nobody building to §6 alone is sent at a socket. Recorded here so it is a
decision rather than a default nobody noticed — and it was very nearly the
latter: `compass-worker-host` had already been written against stdio while §6
still said UDS, which is how a default becomes a fact without anyone choosing
it.

Nothing here is hard. What makes it worth a section is *when* it is found: the
phase is costed at 6–8 weeks, and the wire format is the first thing a host
commits to. The transport error joins Suite 0's `compass --engine=cpp --json
query`, Phase 2's C++ `doctor`, and §8.4a — four specs written against an
artefact nobody checked.

And this section's own first draft joins them, which is the more useful half of
the lesson: **reading one layer and inferring the next is the same mistake as
not reading at all.** `./proto/manager` was read; what it generated was assumed.
The rule, stated for both: before building to a spec — this document's or an
import's — read the thing it describes, all the way down to the bytes.

So the remaining roadmap is not a list of oversights to be closed in a sitting.
It is the bulk of the port, and the honest next move is Phase 4's first slice:
`compass-worker-host`, built the way `compass-ipc` was — transport and framing
first, with the protocol pinned by tests, before anything is spawned.

## 12. Immediate next steps

### 12.0 The order as of 2026-09-24 (ADR-0017)

This list supersedes the ordering further down, which is kept as the record of how each item got
where it is.

#### Remaining parity work (after the ledger truth pass)

The ledger is at **152 of 152 (100%)** after the truth pass, the gaps passes (the window-material pass, GNOME workspaces and shortcut probes, Compass's own tray icon, currency, global shortcuts and wlroots paste and inhibit the latest), the UI pass, the settings pass and the product decisions of 2026-09-25 (§11.4, and `PARITY.md`,
"The ledger truth pass"). The gaps pass that followed (`PARITY.md`, "The gaps pass") closes rows
from this table one at a time and strikes them through here; `scripts/ci/parity-score.py` has the
current figure. These were the 40 cells still amber, by row, and nothing else: each is what
the row's `Still C++-only:` sentence names, checked against the code. Sizes: **small** is under
about 150 lines with its tests, **medium** a few hundred or several small pieces, **large** a
subsystem. "None" under *blocked by* means only the work.

| Row | Missing | Size | Blocked by |
|---|---|---|---|
| ~~`src/cli`~~ | **Done in the gaps pass** (`PARITY.md`, "The gaps pass"): every C++ subcommand, over IPC v17 | — | — |
| ~~`src/services/app-runtime`~~ | **Done in the gaps pass**: Quit, Force Quit, running and frontmost (IPC v17) | — | — |
| ~~`src/services/app-service`~~ | **Done** (`compass::catalog_watch`, `EngineApps`' lookups and `set_web_browser`; PARITY.md, "Gaps closed after the truth pass") | — | — |
| ~~`src/services/calculator-service`~~ | **Done in the currency pass** (`PARITY.md`, "The gaps pass, currency"): currency conversion over the ECB's daily reference rates (`compass_core::exchange_rates`, `compass::exchange_rates`, fend's exchange-rate handler) and Refresh Exchange Rates, IPC v21; the history view was done in the gaps pass; the backend dropdown is not wanted (fend by design) | — | — |
| ~~`src/builtins/clipboard`~~ | **Done in the views pass**: Open and Open with… (the rest landed in the gaps pass); drag stays a declared difference, Iced having no drag out of a window | — | — |
| ~~`src/services/desktop-notification`~~ | **Done** (`compass::notification_icon`; PARITY.md, "Gaps closed after the truth pass") | — | — |
| ~~`src/services/extension-registry`~~ | **Done** (`compass::catalog_watch::watch_extensions`; PARITY.md, "Gaps closed after the truth pass") | — | — |
| ~~`src/services/global-shortcuts`~~ | **Done**: per-command global shortcuts, conflict detection, `vicinae-hotkey-v1`, the launcher hotkey and close on focus loss (`PARITY.md`, "The gaps pass, global shortcuts", IPC v20); `inhibitApps` and `probeBind` ("The gaps pass, GNOME workspaces and shortcut probes", IPC v21); X11 **n/a** (Wayland only, decided 2026-09-25) | — | GNOME's portal grant is VM-tier only |
| ~~`src/builtins/vicinae` (picker half)~~ | **Done**: the paste action, over IPC v18 `PasteText` (`PARITY.md`, "The gaps pass, root and actions") | — | — |
| ~~`src/services/news`, `update`, `telemetry`~~ | **Decided 2026-09-25** (`PARITY.md`, "Product decisions"): the update check is **done** (`compass::updates` over `tuna-os/compass`'s releases, the root search's Update row, `launcher.check_for_updates`, IPC v21); news and telemetry are **n/a** (a hard fork: no news feed, no telemetry) | — | — |
| ~~`src/services/paste`~~ | **Done in the wlroots gaps pass** (`PARITY.md`, "The gaps pass, wlroots paste and inhibit"): the input server's `injectPaste`, else a `zwp_virtual_keyboard_v1` keyboard (which the Flatpak can use), verified on headless Sway | — | — |
| ~~`src/services/root-item-manager`, `src/builtins/root`~~ | **Done in the gaps passes** (`PARITY.md`, "The gaps pass, root and actions"): the provider search view, every fallback, the alias completer, per-item shortcuts (IPC v18) | — | — |
| ~~`src/services/shortcut-inhibit`~~, ~~`window-material`~~ | ~~the keyboard-shortcuts-inhibit protocol~~ (**done in the wlroots gaps pass**, `compass_wayland::keyboard_inhibit` on a connection shared with `iced_layershell`; the `xdg_toplevel` presentation does not inhibit, a declared difference); for `window-material`, the ~~`ext-background-effect-v1` client~~ (**done**, `compass_wayland::material`) and ~~applying it to the launcher's surface~~ (**done in the window-material pass** under `xdg_toplevel`, through the `unsafe` bridge ADR-0019 approved, `compass-wayland-foreign`; the layer-shell presentation is a declared difference) | — | real blur on KWin is VM tier |
| ~~`src/services/tray`~~ (~~`tray-host`~~) | **Done in the tray and sandbox pass** (`PARITY.md`, "The gaps pass, tray and sandbox"): Compass's own tray icon, `compass::tray_icon` over `ksni`, with the C++'s menu and `tray.enabled` applied live (the StatusNotifierWatcher plumbing and the tray search view were done in the gaps pass, `compass::tray_host` over `system-tray`, IPC v18) | — | — (decided: a StatusNotifierItem, on by default as in the C++) |
| ~~`src/services/window-manager`~~ | **Done**: the KDE provider (`compass_platform_linux::compositor::kwin`, "The gaps pass, KDE"; real KWin is VM tier) and GNOME's workspace list (the Shell extension's contract v4 `ListWorkspaces` and `ActivateWorkspace`, "The gaps pass, GNOME workspaces and shortcut probes"); X11 **n/a** (Wayland only, decided 2026-09-25) | — | — |
| ~~`src/builtins/wm`~~ | **Done in the views pass** (`PARITY.md`, "The gaps pass, views"): Switch Workspaces and the fullscreen, floating and overview toggles (IPC v18) | — | — |
| ~~`src/builtins/file`~~ | **Done in the views pass**: the rest of the action panel and the loading indicator (IPC v18); drag stays a declared difference (Iced has no drag out of a window) | — | — |
| ~~`src/builtins/shortcut`~~ | **Done in the views pass**: Open with… (the app-selector view, IPC v18), the detail pane, shortcuts as fallback rows | — | — |
| ~~`src/builtins/snippet`~~ | **Done in the UI pass** (`PARITY.md`, "The gaps pass, UI"): the detail pane (IPC v19 `PreviewSnippet`) and the `\{` escape, in a parser of its own (`compass_core::placeholder`) so quicklinks keep theirs | — | — |
| ~~`src/builtins/system`~~ | **Done** (Browse Apps, Set Default Browser, Set Default Terminal; PARITY.md, "Gaps closed after the truth pass") | — | — |
| ~~`src/builtins/vicinae`~~ | **Done** (`compass_ui::app::vicinae`, IPC v19; `PARITY.md`, "The gaps pass, HUD and onboarding"): the fallback manager, installed extensions, the OAuth token and local-storage browsers, the icon gallery, report bug, refresh apps, open config, the store intros; ~~tray search~~ (done with `tray-host`) | — | — |
| ~~`ui/image`~~ | **Done**: the builtin icon set, file-type icons, command tiles and badges (`PARITY.md`, "The gaps pass, icons and tray"); masks, extension, script and shortcut row icons in root search, favicons, `ImageURL(source)` for a bare string, and the tile's gradient and shadow (`PARITY.md`, "The gaps pass, UI", IPC v19 `ScriptIcons`) | — | — |
| ~~`ui/bridges`~~ | **Done in the UI pass**: a Markdown detail's images, fetched and drawn with the store page's viewer (`PARITY.md`, "The gaps pass, UI") | — | — |
| ~~`ui/action-panel`~~ | **Done** (`compass_ui::shortcut_recorder` over `compass_core::key_combo`; `PARITY.md`, "The gaps pass, root and actions") | — | — |
| ~~`ui/qml`, `ui/quick`, `ui/views`, `ui/settings`, `ui/windows`~~ | ~~the settings window and its pages~~ (**done in the settings pass**, as a view of the launcher: `PARITY.md`, "The gaps pass, settings", IPC v19), ~~onboarding~~ and ~~the HUD~~ (**done**, `compass_core::onboarding`, `compass_ui::hud` and IPC v19 `WindowCommand::Hud`; `PARITY.md`, "The gaps pass, HUD and onboarding") (~~match and Markdown highlighting, the edit-keywords and app-selector views~~, and extension grids: **done**, `PARITY.md`, "The gaps pass, UI"; drag and drop is a declared difference) | large | ~~a design decision on the settings window~~ (decided: a view, declared); the VM tier for anything drawn |

Closed in the truth pass rather than listed: the `xdgpp` writers (`setDefaultApplication`,
`setDefaultTerminal`), `glyph`'s `isUtf8EncodedEmoji`, `{selection}`/`{selected}` in shortcuts, the
power commands' `confirm` and `customProgram` preferences, and a notification's urgency and file
icon.

Closed in the gaps pass (`PARITY.md`, "The gaps pass"): `glyph-service` wired into the emoji picker;
clipboard eviction and its timer, the monitoring switch and preferences, the kind filter, keyword
editing, remove-all and the detail pane (118 of 156); and in the root view, favourites, the row's
panel, the alias form, the space-bar alias, up-arrow history and the clock (no row flips: the
provider search view, per-item shortcuts and other fallbacks remain).

Closed in the views pass (`PARITY.md`, "The gaps pass, views", IPC v18): Switch Workspaces and the
window toggles, shortcuts' Open with…, detail pane and fallback rows, Search Files' action panel
and loading indicator, and clipboard history's Open and Open with…, over one app-selector view
(125 → 129 of 156).

Closed in the KDE pass (`PARITY.md`, "The gaps pass, KDE (2026-09-25)", no IPC change): the KWin
window-manager provider — windows, focus, close, virtual desktops as workspaces, fullscreen and the
overview — over KWin scripting on the session bus, with a `kde.kwin` doctor check. No row flips: the
window-manager row stays amber for X11 and GNOME's workspace list (145 of 156, 93%).

Decided on 2026-09-25 (`PARITY.md`, "Product decisions", IPC v21 `UpdateStatus`, `SkipUpdate`): the
update check wired over Compass's own releases (`src/services/update` `Rust ✓` green); news and
telemetry n/a in both columns; X11 n/a for global shortcuts and the window manager, neither row
flipping on it (147 of 152, 97%).

Closed in the tray and sandbox pass (`PARITY.md`, "The gaps pass, tray and sandbox (2026-09-25)",
no IPC change): Compass's own tray icon (`compass::tray_icon` over `ksni`), its `tray.enabled`
setting offered and applied live, and a read-only `$HOME` allowlist for extensions
(`compass_sandbox::home::HOME_READ_ALLOWLIST`: `~/.ssh/config`, `~/.password-store`, the Hyprland,
Sway and niri configurations) — `src/services/tray` `Rust ✓`; with the product decisions and the currency pass, 149 of 152 (98%).

Closed in the GNOME workspaces and shortcut probes pass (`PARITY.md`, "The gaps pass, GNOME
workspaces and shortcut probes (2026-09-25)", IPC v21 `ProbeShortcut` and `ShortcutProbe`): Switch
Workspaces on GNOME over the Shell extension's contract v4 (`ListWorkspaces`, `ActivateWorkspace`;
an extension at contract 3 still switches windows, and `compass doctor` reports v4),
`global_shortcuts.inhibit_apps` pausing every global shortcut while a listed application is
focused, and the recorder's `probeBind`. With X11 n/a by the product decisions, the window-manager and global-shortcuts rows flip to
`Rust ✓` green.

Closed in the settings pass (`PARITY.md`, "The gaps pass, settings", IPC v19 `SetSetting`,
`SetProviderEnabled`, `RootItemEdit::Enabled`): the settings window's sidebar and pages as a view of
the launcher, over `compass_core::settings_catalog`; every C++ setting the Rust engine reads is
edited and written where the engine reads it, and the rest are declared with their reasons
(`ui/settings` both columns, 133 → 135 of 156, 87%).

**Landed in this round:**

- **Suite 0 runs for real.** Both engines in one Bluefin container, 1817 queries, 99.0% top-result
  agreement; gated in CI on the top result for queries of four or more characters, verified to fail
  and pass on the real C++ engine ([`SUITE0-BASELINE.md`](./SUITE0-BASELINE.md)). Item 3 below is
  therefore done. Under ADR-0017 this is a tripwire, not the spec.
- **An absolute search-quality suite** over the real corpus (`search_quality.rs`). It found
  [#204](https://github.com/tuna-os/compass/issues/204), which the differential structurally
  cannot.
- **The paint tier** (`crates/compass-ui/tests/paint.rs`): the real launcher view rendered to
  pixels on wgpu (lavapipe on CI) and tiny-skia, with invariants tied to layout bounds. Verified on
  a GitHub runner and by mutations the structural tests miss.
- **A test ladder**: `make test-t0` … `test-t3`, cheapest first, described in
  [`RENDER-HARNESSES.md`](./RENDER-HARNESSES.md).

**Next, in order:**

1. ~~**#204 — typo tolerance in app search.**~~ **Done:** a one-edit `strsim` fallback, ranked
   after every real match; the formerly ignored `search_quality.rs` test is the acceptance
   criterion and passes. The C++ engine has the same gap, which is the point of ADR-0017.
2. **Storage onto `rusqlite`** (ADR-0017 decision 4), in steps that are each their own PR:
   (a) ~~`spellfix1` out~~ **done** — a plain `vocabulary` table and a `strsim` suggester
   (`compass_db::vocabulary`), passing the ported file-search quality suite (23/23, including the
   four cases that depend on typo correction); Compass's index moved to its own file,
   `compass-file-index.db`, at schema v2, so the two engines stop purging each other's.
   (b) ~~re-base the wrapper on `rusqlite`~~ **done (2026-09-24):** the hand-written FFI and its
   `Database`/`Statement`/`Transaction` are gone; `compass_sqlcipher_sys::open` returns a
   `rusqlite::Connection` and every caller uses rusqlite's API (see
   [`CRATE-AUDIT.md`](./CRATE-AUDIT.md)). The history of the decision: it was first
   **deferred, and re-ranked below items 3–4.** Re-basing the wrapper on `rusqlite` was
   justified by removing the workspace's one `unsafe` opt-out, and that premise did not survive
   (a): `fuzzy_trigram` stays, its registration needs the raw `sqlite3*` after keying, so the
   crate keeps `unsafe` either way. What (b) would still buy is ~500 lines of FFI replaced by a
   crate, at the price of linking our C tokenizer against `libsqlite3-sys`'s own SQLCipher (4.6.1,
   against the vendored 4.16.0) — a real risk for a modest gain. Revisit if the wrapper grows or
   a bug lands in it. **Re-ranked up (2026-09-24):** `libsqlite3-sys` 0.38.2 now bundles SQLCipher
   4.14.0, so the version gap is mostly gone and the ~300 hand-bound call sites are the payoff; see
   [`CRATE-AUDIT.md`](./CRATE-AUDIT.md).
3. **A Vicinae importer** for clipboard history, extension storage and OAuth tokens (decision 3),
   reading content tables only. Needed before cutover, not before item 2.
   **Clipboard history: done** (`crates/compass/src/vicinae_import.rs`). On the first engine
   start that can read it, Vicinae's `clipboard.db` and `clipboard-data/` are read with Vicinae's
   own keyring key. Entries go into Compass's store re-encrypted, with their times (seconds become
   milliseconds), pins and keywords. Content Compass already has is left alone, and a marker makes
   it one-shot. A locked or unkeyed database writes no marker, so the next start retries.
   **Extension storage and OAuth tokens wait for Phase 4:** nothing in the engine opens them yet
   (only `compass-worker-host`'s tests do), so there is no Compass-side store to import into
   until the extension host owns one.
4. **Summon-to-first-frame — now recorded.** The launcher logs `summon_draw_ms`, from the
   engine's `Show` to the new window's first redraw request, on the same terms as cold start's
   `first_draw_ms` (a floor: the paint after the request is not in it). Tier 2's `session.sh`
   reports it from real Mutter on GNOME 50 and 51 on every PR that touches `crates/**`. Recorded,
   not gated (ADR-0010): the threshold comes from the numbers once there are some.
5. **Re-evaluate `compass-xdg` against `freedesktop-desktop-entry`** — lowest priority; ours
   exists for good reasons, but decision 2 says to check.
6. **Promote the VM tier to the merge queue** — unchanged from item 6 below.

**Phase 4's first wired slice (#7):** installed extensions are in root search, and the engine runs
their `no-view` commands. Until now the Phase 4 crates were tested libraries that nothing called.
- `AppIndex::from_environment` scans the manifest registry. Each command is a root item with the
  C++ id `@<author>/<extension>:<command>`, subtitled by its extension. A manifest-disabled command
  is known but hidden.
- `RunExtensionCommand` (IPC v7) starts the runtime bundle under Node (`crates/compass/src/extension_runner.rs`):
  - local storage comes from Compass's own `compass-extension-storage.db`, keyed from the keyring;
  - HUDs, failure toasts and notifications become desktop notifications;
  - alerts are answered "no".
- The runtime never says when a `no-view` command has finished, so a run ends after 10 s of quiet
  or 5 minutes.
- Refused, each with a sentence the launcher shows:
  - a `view` command (no view renderer yet);
  - a required preference without a default (no preference editor yet);
  - a missing runtime or Node.
- The runtime runs **confined**, behind `compass-sandbox-exec` (Landlock + seccomp, strict). It
  may read the system, Node, the bundle and its own extension, and write only that extension's
  support and asset directories. It gets a private `TMPDIR` rather than `/tmp`. The engine
  refuses to run extensions without the launcher unless `COMPASS_EXTENSION_SANDBOX=off`. The
  end-to-end test requires a write outside those directories to fail with `EACCES`, and fails
  with the sandbox off.
- The Flatpak ships Node (`org.freedesktop.Sdk.Extension.node22`), the bundle and the launcher.
  Its CI asserts all three inside the installed sandbox.
- **View commands draw** (`List` and `Detail`), in a first cut:
  - The engine publishes each render as a typed `compass_extension_api::View`
    (`compass_worker_host::view_model`). The launcher long-polls it (`ExtensionView`, IPC v8) and
    draws it as a page.
  - Enter runs the selected row's first action through `EventCore/handlerActivated`.
  - A list the host filters is filtered fuzzily in the launcher. A list that filters itself gets
    the text and the echo count (ADR-0009).
  - Escape stops the command.
  - Extensions' `Clipboard` API is the GNOME Shell extension's clipboard.
- **Since then (#211),** each proven against the real runtime in `engine_end_to_end.rs`:
  - Ctrl+B opens a view's action panel, and actions' keyboard shortcuts run them;
  - a `Detail` draws its Markdown;
  - `push` shows the pushed view and Escape pops it; `confirmAlert` is shown and answered;
  - required preferences are asked for in a form and kept encrypted in the extension store;
    without a keyring the run is refused with the reason;
  - command arguments are asked for and passed as `props.arguments`, never stored;
  - `open()`, `Action.OpenInBrowser` and `getApplications()` are served by the engine, which
    launches outside the sandbox;
  - `Grid` renders typed and is searched and acted on as rows;
  - `Form` fields are drawn and edited, echoes are counted so typing is never undone
    (ADR-0009), and `SubmitForm` gets the values;
  - memory: Node's `--max-old-space-size=160` caps every isolate (measured: it overrides the
    runtime's 1000 MB worker limit), and the worker's pid goes into a user-systemd scope with
    `MemoryMax=256M` where that manager is reachable. A command allocating 400 MiB is stopped.
- Still to come:
  - icons and image tiles (a grid is drawn as rows until then);
  - multi-line text areas, and the date, tag and file pickers in forms;
  - toasts drawn in the launcher rather than as desktop notifications;
  - `runInTerminal`.

**Needs the project owner:** nothing. ADR-0018 decided the Suite 0 gate (it keeps blocking), GNOME 51
(reworded gate, #4 closed), team size (one person), rustcast (a seed), and the upstream report (none).

**Current implementation check:** `UI/confirmAlert` already has a deferred transport and
adapter; it must not be reimplemented from the older “not started” entry. The application
action panel now dispatches Open, Copy name and Copy path by stable action IDs, offers a focused
fuzzy filter, and routes Enter through one keyboard handler. Copy uses Iced's native clipboard
task while leaving the launcher alive. Headless widget and task tests cover these paths;
clipboard delivery and focus under GNOME still require desktop integration checks. This does
not close the extension-rendering, builtin-view, platform or release gates below.

Long native action panels now keep the filter outside a bounded scroller.
Keyboard navigation reveals the selected widget using its measured layout
bounds rather than estimated row heights. Headless Iced tests exercise wheel
scrolling, full keyboard traversal, wraparound and filtering after scrolling.
PR evidence includes real headless wgpu renders; those do not replace the
GNOME/Flatpak integration checks.

The application results list now uses the same measured-selection reveal
operation and a bounded scroller, keeping the query fixed. Native regressions
cover reaching the last result with the wheel and keyboard navigation/query
resets across all appearance presets. Browser long-list states and actual
headless wgpu captures accompany the change; target-session checks remain
required before merging.

Rewritten as items land; the previous version listed the VM tier and both spikes as the work to do,
and all three now exist.

**Done since the last revision:** the Flatpak builds and runs in CI; the VM tier boots Bluefin with
our Flatpak in it and asserts from a real GNOME session; Spike A has an answer (§11.1); Spike B runs
in both the Flatpak job and the VM; every workflow defaults to read-only permissions.

**That gap has moved rather than closed, and the section previously said otherwise.** It read: *"The
largest gap is that there is no launcher — `LaunchSelected` returns `Task::none()`, and no code
starts a window."* Both halves are now false. `crates/compass/src/lib.rs` calls `compass_ui::run`
with a real `LinuxLauncher`, and `LaunchSelected` launches through the `AppLauncher` trait (#64).
The VM tier watches it draw in a real GNOME session.

**The daemon's half is now built, and the remaining gap is the window's.**
[ADR-0015](./adr/0015-the-launcher-window-is-resident.md) settled the shape: a resident window
process attaches to `serve` over the same socket, and `serve` pushes `show`/`hide`/`toggle` to it.
Both halves of that protocol exist — `compass-ipc` carries the push direction (`WindowLink` on the
engine's side, `WindowClient` on the window's), and `serve` holds at most one attached window and
forwards to it. End-to-end tests attach a window from the test process to a real spawned daemon and
assert the command arrives as itself and the answer comes back.

**`compass ui` now attaches, and the loop is closed in code.** It runs on `iced::daemon` rather than
`iced::application`, so the window is something it opens and closes rather than something it *is*:
dismissing hides, a successful launch hides, and the engine's `show` opens a window again. On
Wayland that is what hiding means — `xdg_toplevel` has no hide, so a hidden window is a closed one
— and what residency preserves is the process, the wgpu adapter, the font atlas and the index.

With no engine listening, `compass ui` still starts and Escape still exits: a window that hid with
nothing able to summon it back would be an invisible process.

**The shortcut is bound too.** `compass serve` opens a GlobalShortcuts session, asks for
`LOGO+space`, and turns each activation into a `Toggle` pushed to the attached window. On GNOME
50/51 that portal is the only path an unprivileged application has to a global hotkey; where it
does not exist — every wlroots compositor — the engine says so and `compass toggle` still works.
Nothing about the hotkey can stop the engine starting: the socket is the contract, the hotkey is a
convenience. `serve --no-hotkey` declines to ask at all, for a user whose compositor already binds
a key — and, measurably, for the VM tier, where GNOME's permission dialog is 1.62% of the screen
sitting in the middle of a gate about the launcher.

**The loop is verified on a real GNOME session.** `scripts/vmtest/launcher.sh` starts the engine,
starts the launcher, asks the engine to hide the window and then to show it again, and gates on
what the screen does. As of `5918e2a` every gate passes on Bluefin under corral:

| gate | result |
|---|---|
| starting the engine draws nothing | `IDENTICAL` |
| a launcher window appeared | 9.99%, box x 335..942 y 152..796 |
| the window answered a toggle over the link | passed |
| summoning it back | 480 ms round trip |
| hidden looks like the bare desktop again | `IDENTICAL` |
| summoned looks like a launcher again | 9.96%, **the same box it first opened in** |

The last pair is the part worth reading twice. Hiding returns the screen to byte-identical with the
desktop, and summoning reproduces the opened frame's bounding box to within three pixels of area —
so the window genuinely goes away and genuinely comes back, rather than something merely changing.

This is also the first thing in this tier that can observe a *connection* rather than a process:
`serve` refuses `toggle` when no window has attached, so a `toggle` that succeeds is proof of the
whole chain — CLI, socket, engine, window link, and a window that answered on the other end.

**What is still missing is the keypress, and it is not the code's fault.** Injected input does not
reach this VM's compositor at all — `launcher.sh` documents the chain and where it breaks, and
GNOME's own Super binding is equally inert there. So the client in the tier is `compass`, not
Super+Space, and what stays untested is the portal delivering an activation. Everything after the
activation is exercised.

**And the number that matters is still unmeasured.** See §8.5's split SLA row: summon to first
frame still has no harness. The 480 ms the tier now reports is a round trip, not a frame, and an
upper bound with a whole Flatpak launch inside it.

**One more thing the tier has to be told to ignore.** Two strips of GNOME's own furniture change
without us: the top bar carries a clock, and the dash redraws its backdrop when any process starts.
Both are excluded from the "nothing drew" gates. The middle of the screen — where a window or a
permission dialog would land — is still compared exactly, and `framediff-selftest.py` holds twelve
controls proving each gate still fails for every reason it exists to catch.

Which is also why the refusal stays a refusal. A client can tell "no window" from "the window was
shown", and that distinction is the only thing standing between an honest gap and a `toggle` that
silently does nothing.

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
1. ~~**Wire the UI into the binary**~~ — done (#29). `compass ui` opens a window, moves a
   selection with the arrow keys, launches through `compass-platform` and dismisses. It draws on
   the target, verified on every VM run against a measurement that reproduced three times
   (ADR-0010). Phase 1's gate is consequently evaluable for the first time — scored in §11.2.
2. ~~**Settle Spike A's consent question**~~ — done (§11.1, ADR-0010). Traced through all three
   components and pre-seeded; what remains is to read the first run that gets a binding, and in
   particular whether Super+Space survives GNOME's own claim on it.
3. ~~**Capture the C++ baseline on the target.**~~ **Done — see 12.0 and `SUITE0-BASELINE.md`.** Today's parity suites compare the port against *our
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

   **Where this stands.** `.github/workflows/cpp-on-target.yaml` now has both halves.
   `configure` is green (§11.2). `build` compiles the tree in the same image, stages an
   installable tree with `DESTDIR=… cmake --install`, and publishes it as `vicinae-cpp-bluefin`.
   It asserts the two files that matter rather than trusting the install — `usr/bin/vicinae`, the
   entrypoint `parity --cpp-engine` invokes, and `usr/libexec/vicinae/vicinae-server` — because
   `cmake --install` succeeding says nothing about which targets carried an `install()` rule. It
   is gated on `needs: configure` so a wrong package name costs a minute rather than twenty, and
   ccache is mounted in from the host so a rerun that changed only the workflow is cheap.

   **What is left is NOT just wiring.** An earlier revision of this item said it was — "layer the
   tarball into the VM test image and have `checks.sh` run `parity --cpp-engine`". That was wrong
   in two ways, and measuring the harness rather than reading it is what showed them. See §8.1a.
4. ~~**Grow the corpus**~~ — **done: 757 entries, past the gate's 500.** `corpus-harvest.yaml`
   produced 730 desktop entries from 419 Fedora packages in about three minutes; 642 were new, and
   the real set went from 96 to 738. What that harvest also did was destroy this section's
   headline finding about scorer parity — see §8.1a. The remaining text is kept because the
   reasoning that got here was wrong twice and both corrections are worth having.

   **Original heading: grow the corpus — a real constraint, though not the binding one.** An earlier revision of
   this item called it "Phase 1's binding constraint" and put the count at 27. Both are now wrong:
   §11.2 retracted the first (Suite 0 is differential, so item 3 above is the keystone) and the VM
   harvest answered the second. The gate names a 500-entry corpus for Suite 0 ranking parity, and
   there are 115.

   The harvester's own header asks for "a real desktop — ideally a Bluefin box, since that is the
   first target and its RPM + Flatpak + Homebrew mix is what users actually have". **The VM tier
   boots exactly that, every run.** The machine the script was waiting for has been in CI since the
   tier existed, and nobody noticed — including this plan, which listed the corpus as blocked on
   hardware nobody had.

   The `launcher` job now runs it and publishes `corpus.tar.gz` as an artifact. What it does *not*
   do is write into `crates/compass-testkit/corpus/`: a corpus shapes every ranking assertion the
   project makes, and it must not grow by a job quietly appending to it.

   **The first harvest is committed: 88 new entries, taking the real set from 8 to 96 and the
   corpus to 115.** All stock Fedora/GNOME, reviewed before landing.

   One Bluefin image yields 88, and running that job again yields the same 88. An earlier revision
   read that as needing *different machines*, and that was wrong — **it needs a different app set,
   which is not the same problem and is not blocked on hardware at all.**

   `.github/workflows/corpus-harvest.yaml` produces one. It asks dnf which packages ship a
   `/usr/share/applications/*.desktop`, downloads a bounded batch of them, and extracts only the
   desktop entries — no installation, so there is no dependency resolution and no gigabytes of
   runtime for applications nobody launches. The bytes are the same ones that would land on a
   user's disk. It runs on `workflow_dispatch`, publishes an artifact, and like the VM harvest it
   **never writes into the corpus itself**.

   The cheap step runs first and asserts its own premise: if the `repoquery` names fewer than fifty
   packages the job fails in about a minute, because an empty list would otherwise present as a
   successful harvest of nothing. §12 item 3's four red runs are why that ordering is deliberate.

   What this still does not produce is the Flatpak and Homebrew halves of §8.1's "host RPM apps,
   Flatpak exports and Homebrew entries together" — those export paths shape entry *names* and
   `Exec` lines differently, and RPM extraction cannot fake them.

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

   ~~What genuinely remains under this heading is the corpus itself: 8 harvested entries from one
   host is a thin sample.~~ **Stale, and it contradicted item 4 three paragraphs above.** The
   harvested set is **738 real entries** against 19 synthetic, so the thin-sample concern this
   sentence described was answered by the same harvests item 4 records. `scripts/harvest-desktop-corpus.sh`
   remains how it grows, and one distribution's application set is still one sample — §8.1a's
   divergence table is the standing reminder that a 115-entry corpus produced a generalisation the
   738-entry one destroyed. But nothing under this heading is now outstanding.
6. **Promote the VM tier to the merge queue** once it has been stable for a couple of weeks
   (ADR-0010). It has three consecutive green runs; that is not two weeks.

Both corral bugs this tier found on locally built bootc images are now filed upstream:
`podman create` on a CMD-less image (tuna-os/corral#303) and the layer builder pulling a
`localhost/` reference its own disk builder already guards against (tuna-os/corral#304). Our
workarounds stay until they are fixed; neither is blocking.
