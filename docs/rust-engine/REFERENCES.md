# Reference material for the Rust engine transformation

Companion to [`PLAN.md`](./PLAN.md). Everything below was checked against the live registry
(crates.io versions and dates verified 2026-09-12); anything unverified is marked as such.

The point of this file is to keep us from re-deriving solved problems. Several of the hardest parts
of the plan — layer-shell-on-Iced, a language-agnostic plugin IPC protocol, portal fallbacks,
sandboxing a JS runtime — have working prior art we can read, and in some cases depend on directly.

---

## 1. Prior-art launchers worth reading

| Project | Stack | Why it matters to us |
|---|---|---|
| [pop-os/launcher](https://github.com/pop-os/launcher) | Rust, line-delimited JSON over stdin/stdout | **The closest existing model for our `compass-core` ↔ worker split.** Modular IPC launcher *service* with process-isolated plugins: one plugin crashing doesn't take the launcher down, and plugins can be written in any language. Read its `Request`/`Response`/`PluginResponse` enums before finalising `compass-ipc`. Frontends: pop-shell, cosmic-launcher, onagre. |
| [onagre-launcher/onagre](https://github.com/onagre-launcher/onagre) | Rust + **iced** + pop-launcher backend | An Iced launcher on Wayland that already made the frontend/backend split we're proposing. Closest architectural sibling to the Phase 1–2 target. |
| [HaruNashii/Icelauncher](https://github.com/HaruNashii/Icelauncher) | Rust + iced + `iced_layershell` | Tiny (31 commits, early beta) but it is `.desktop` scan → icon resolve → surface → launch in one readable afternoon — our Phase 1 minus the GNOME parts. **Its known bug, "crashing on GNOME due to missing Layer Shell protocol support", is §3.1 in the wild and is exactly why our Phase 1 uses `xdg_toplevel` instead.** |
| [anyrun-org/anyrun](https://github.com/anyrun-org/anyrun) | Rust + GTK4 + gtk4-layer-shell | Minimal plugin ABI (`init` / `info` / `get_matches` / `handler`) compiled to `.so` and loaded at runtime; uses `wlr-data-control` for clipboard. Good counterpoint to our JSON-RPC worker model — worth knowing why we're *not* doing dynamic-library plugins (no sandbox boundary). |
| [abenz1267/walker](https://github.com/abenz1267/walker) + [elephant](https://github.com/abenz1267/elephant) | Rust/Go, GTK4 + gtk4-layer-shell | Ships the daemon split as two separate products: `elephant` (provider daemon) must be running before `walker` (UI) starts. Useful evidence on how that split feels to package and to users. |
| [Skxxtz/sherlock](https://github.com/Skxxtz/sherlock) | Rust + GTK4 | Another Rust launcher data point; not verified in depth. |
| upstream [vicinaehq/vicinae](https://github.com/vicinaehq/vicinae) | C++/Qt6 | The tree we forked. Even after the rewrite it remains the reference for *behaviour* and the source of bug fixes worth porting — see PLAN.md §10.1 on fork posture. |
| [MystikoLab/rustcast](https://github.com/MystikoLab/rustcast) | Rust + iced 0.14, macOS | Our seed. See PLAN.md §1.2 for what is and isn't reusable. |

**Not-a-launcher but read anyway:** [pop-os/cosmic-launcher](https://github.com/pop-os/cosmic-launcher) and
[libcosmic](https://github.com/pop-os/libcosmic) — libcosmic is the largest production Iced
deployment that exists, including its own Wayland shell integration. If Iced can carry a whole
desktop environment it can carry a launcher; and when Iced fights us, libcosmic has probably already
patched around it.

---

## 2. Crates we should depend on (versions verified on crates.io, 2026-09-12)

### UI and Wayland

| Crate | Version | Role |
|---|---|---|
| [`iced`](https://crates.io/crates/iced) | 0.14.0 | UI. Same version rustcast pins — no migration needed to start. |
| [`iced_layershell`](https://crates.io/crates/iced_layershell) | 0.19.1 | Iced ↔ `wlr-layer-shell` binding, incl. popup windows. Actively maintained (updated 2026-07). From [waycrate/exwlshelleventloop](https://github.com/waycrate/exwlshelleventloop), which also binds `ext-session-lock`. **Not needed for the GNOME target (§3.1) — this is the Phase 5 wlroots path.** |
| [`smithay-client-toolkit`](https://crates.io/crates/smithay-client-toolkit) | 0.21.1 | Client-side Wayland helpers, as the spec calls for. |
| [`wayland-protocols`](https://crates.io/crates/wayland-protocols) | 0.32.13 | Stable + staging protocol bindings (`ext-foreign-toplevel-list-v1` lives in staging). |
| [`wayland-protocols-wlr`](https://crates.io/crates/wayland-protocols-wlr) | 0.3.12 | `wlr-layer-shell`, `wlr-foreign-toplevel-management`, `wlr-data-control`. |
| [`smithay`](https://crates.io/crates/smithay) | 0.7.0 | Compositor-side — only for the **mock compositor** test fixture, which is Phase 5 work (Suite 3c). |

### Desktop integration

| Crate | Version | Role |
|---|---|---|
| [`ashpd`](https://crates.io/crates/ashpd) | 0.13.13 | XDG portals: GlobalShortcuts, OpenURI, Screenshot, FileChooser, Secret. Very widely used (14M downloads). |
| [`zbus`](https://crates.io/crates/zbus) | 5.19.0 | DBus. **Not a fallback — it is the primary window/clipboard transport on our first target** (§3.4), and later the KWin path. |
| [`freedesktop-desktop-entry`](https://crates.io/crates/freedesktop-desktop-entry) | 0.8.3 | Desktop-entry parsing (pop-os). **Evaluate against porting `src/lib/xdgpp` rather than assuming either** — our xdgpp has 10 test files' worth of edge cases encoded in it, and this crate may not match. Decide with the Suite-0 corpus. |
| [`freedesktop-icons`](https://crates.io/crates/freedesktop-icons) | 0.4.0 | Icon theme lookup. |
| [`arboard`](https://crates.io/crates/arboard) | 3.6.1 | Clipboard; already used by rustcast, works on Wayland. |

### Search and storage

| Crate | Version | Role |
|---|---|---|
| [`nucleo-matcher`](https://crates.io/crates/nucleo-matcher) | 0.3.1 | The matcher, as the spec specifies. From the Helix editor. |
| [`nucleo`](https://crates.io/crates/nucleo) | 0.5.0 | The higher-level multi-threaded, lock-free *interactive* matcher on top of it — this is the one the spec's "multi-threaded, lock-free fuzzy search indexing" line actually describes. Prefer it over hand-rolling a worker pool around `nucleo-matcher`. |
| [`tantivy`](https://crates.io/crates/tantivy) | 0.26.2 | Full-text index — only if `file-indexer` needs more than fuzzy filename matching. Do not adopt speculatively. |
| `rusqlite` | (rustcast pins 0.40) | Bundled SQLite, already in the seed. |

### Sandboxing (Phase 4)

| Crate | Version | Role |
|---|---|---|
| [`landlock`](https://crates.io/crates/landlock) | 0.4.7 | **Filesystem sandboxing — the modern answer, and better than the spec's "read-only mounts".** Unprivileged, per-process FS access rules; no bind-mount gymnastics, no setuid helper. Kernel 5.13+, with graceful degradation via its compatibility API. This should be the primary FS boundary for extension workers. |
| [`seccompiler`](https://crates.io/crates/seccompiler) | 0.5.0 | seccomp-bpf filter compilation (from Firecracker — battle-tested on exactly this problem). Filters expressed as JSON, which makes the allowlist reviewable in-tree. |
| [`cgroups-rs`](https://crates.io/crates/cgroups-rs) | 0.5.1 | cgroups v2 memory/CPU limits for the 256 MB-per-extension cap. Note: on a systemd-managed host, prefer asking systemd for a transient scope over writing the cgroup tree directly. |

### Extension runtime (Phase 4)

| Option | Version | Notes |
|---|---|---|
| Keep Node.js out-of-process | — | **Recommended.** `src/typescript/` and the existing `node-runtime` host already work; the plan only replaces the host side. Lowest risk. |
| [`deno_core`](https://crates.io/crates/deno_core) | 0.411.0 | Embed V8 directly in Rust. Attractive because Deno's permission model gives a *second* sandbox layer inside the process, and it removes the Node dependency. But it means re-implementing the Node API surface that Raycast extensions rely on. A Phase-5+ investigation, not a Phase-4 commitment. |
| [`rustyscript`](https://crates.io/crates/rustyscript) | 0.12.3 | Friendlier wrapper over `deno_core`. Small user base (67k downloads) — evaluate, don't depend. |

### Scripting runtimes (the Rhai tier — PLAN.md §2.2)

| Crate | Version | Role |
|---|---|---|
| [`rhai`](https://crates.io/crates/rhai) | 1.26.1 | **The choice.** Pure Rust, no C dependency, embeds cleanly in a Flatpak. Very actively maintained (11.4M downloads; last release 2026-09-10). An optional bytecode compiler ("Rhai Grain") claims 1.8–2× over the AST walker if we ever need it. |
| [`mlua`](https://crates.io/crates/mlua) | 0.12.1 | Lua. Bigger author ecosystem, but a C dependency and a weaker sandboxing story — Lua's stdlib gives you `io` and `os` unless you strip them, i.e. subtracting rather than granting. |
| [`wasmtime`](https://crates.io/crates/wasmtime) + [`wasmtime-wasi`](https://crates.io/crates/wasmtime-wasi) | 48.0.2 | Strongest isolation and any source language. Wrong shape for forty-line scripts, and a large lift. Revisit only if the extension tier grows into something that needs real multi-language support. |
| [`rquickjs`](https://crates.io/crates/rquickjs) / [`boa_engine`](https://crates.io/crates/boa_engine) | 0.13.0 / 0.22.0 | In-process JS. Tempting — authors already know JS — but owning a second JS runtime with a *different* API surface from the Raycast tier is worse than Rhai's honest separateness. |

**Why Rhai's sandbox is structurally better than the Node one.** Rhai's standard library is pure
computation: no filesystem, no network, no process spawn. Every capability a script can reach is one
the host explicitly registered, so a script that did not declare `net` cannot make an HTTP call
because the function does not exist in its scope. That is capability-based security by construction.
The Node worker is the opposite: full access, subtracted with Landlock and seccomp.

**Limit APIs, all verified in [the Rhai book](https://rhai.rs/book/safety/):**

| API | Guards against |
|---|---|
| `Engine::set_max_operations` | runaway scripts / infinite loops |
| `Engine::on_progress` | the termination hook — return a value to abort mid-run |
| `Engine::set_max_call_levels` | deep recursion, stack exhaustion |
| `Engine::set_max_string_size` | memory blowups via string building |
| `Engine::set_max_array_size` | memory blowups via arrays |
| `Engine::set_max_expr_depths` | deeply nested expressions at parse time |
| `Engine::set_max_modules` | `import` in a loop hammering the filesystem |

**The sharp edge:** `Engine::new` installs `FileModuleResolver` by default, which loads `.rhai`
files from disk — so `import` is a filesystem read the script author never had to ask for. Close it
with `Engine::new_raw()` plus an explicit `StandardPackage`, or `DummyModuleResolver`, or a resolver
scoped to the script's own bundle directory. Make it a test, not a review note.

**No async.** Rhai is synchronous and host functions must be sync
([rhaiscript/rhai#215](https://github.com/rhaiscript/rhai/issues/215)). Run every script on
`tokio::task::spawn_blocking` and have I/O host functions block from that thread into the runtime.
Bound the blocking pool and set a wall-clock timeout, or a slow HTTP call becomes a stuck thread.

### Testing and tooling

| Crate | Version | Role |
|---|---|---|
| [`criterion`](https://crates.io/crates/criterion) | 0.8.2 | Suite 4 benchmarks / SLA gates. |
| [`insta`](https://crates.io/crates/insta) | 1.48.0 | Snapshot tests. |
| [`proptest`](https://crates.io/crates/proptest) | 1.11.0 | Property tests for the parsers (spec §Phase 2). |
| [`postcard`](https://crates.io/crates/postcard) | 1.1.3 | Compact `serde` wire format — the plan's default over Cap'n Proto (PLAN.md §2.1). |
| [`capnp`](https://crates.io/crates/capnp) | 0.27.2 | Available if Phase-4 benchmarks force it. |
| [`fluent`](https://crates.io/crates/fluent) | 0.17.0 | i18n, to replace the Qt Linguist catalogue (PLAN.md §2.1). |
| `cargo-nextest`, `sccache` | — | CI wall-clock budget (PLAN.md §7.7). |

**Considered and rejected:** [`i-slint-core`](https://crates.io/crates/i-slint-core) 1.17.1 — Slint is
a fine toolkit and the spec lists it as an option, but rustcast hands us a working Iced app and
libcosmic proves Iced at scale. Recorded in ADR-0001; not revisited without new evidence.

---

## 3. Protocol reality check — verified against primary sources

**First target is GNOME 50/51 on Bluefin**, and the gist spec does not survive contact with what
Mutter actually implements. Everything here was checked in the source, not inferred from docs.

### 3.1 What Mutter 51 actually supports

`src/meson.build` on `mutter` main (51.rc) lists the complete set of Wayland protocols it
implements. Present: `xdg-shell`, `xdg-activation` (staging v1), `xdg-dialog`, `xdg-toplevel-tag`,
`xdg-toplevel-drag`, `xdg-session-management`, `xdg-foreign` v1/v2, `xdg-output`,
`xdg-system-bell`, `keyboard-shortcuts-inhibit` v1, `text-input` v3, `fractional-scale`,
`cursor-shape`, `color-management`, `commit-timing`, `fifo`, `idle-inhibit`, `linux-dmabuf`,
`linux-drm-syncobj`, `pointer-constraints`, `pointer-gestures`, `pointer-warp`,
`presentation-time`, `primary-selection`, `relative-pointer`, `single-pixel-buffer`, `tablet` v2,
`viewporter`, `drm-lease`, `ext-background-effect`, plus the private `gtk-shell`.

Absent — and each one is load-bearing in the gist spec:

| Protocol | In Mutter 51? | Consequence |
|---|---|---|
| `wlr-layer-shell` | ❌ | Launcher must be a plain `xdg_toplevel` on GNOME |
| `ext-foreign-toplevel-list-v1` | ❌ | No protocol window list |
| `wlr-foreign-toplevel-management` | ❌ | No protocol activate/close |
| `wlr-data-control` / `ext-data-control` | ❌ | No protocol clipboard monitoring |
| `ext-workspace` | ❌ | No protocol workspace switching |

Corroborating: [mutter#973](https://gitlab.gnome.org/GNOME/mutter/-/issues/973) (layer-shell) has
been open since 2019, as has [gnome-shell#1141](https://gitlab.gnome.org/GNOME/gnome-shell/-/issues/1141).
[Icelauncher](https://github.com/HaruNashii/Icelauncher) documents "crashing on GNOME" for exactly
this reason. Mutter's own NEWS through 51.rc mentions neither protocol.

### 3.2 `ext-foreign-toplevel-list-v1` is list-only anyway

Per the [protocol registry](https://wayland.app/protocols/ext-foreign-toplevel-list-v1), its only
requests are `stop()` and `destroy()`; events are `toplevel`, `closed`, `done`, `title`, `app_id`,
`identifier`. The spec text is explicit: *"This protocol is intentionally minimalistic and expects
additional functionality … to be implemented in extension protocols."*

So the gist's "issues `activate` and `close` requests directly to the compositor" is wrong on
**every** compositor, not just GNOME. Activation needs `xdg-activation-v1` or
`wlr-foreign-toplevel-management`; there is no portable close.

Where it *is* supported: Hyprland 0.52.1+, COSMIC 1.0.0-beta.8+, Sway 1.11+, niri 26.04+,
labwc 0.20.2+, river 0.3.13+, Treeland, Mir, phoc, Jay. Not supported: **KWin, Mutter**, Wayfire,
Weston, Cage, GameScope. That is both of the two largest desktops.

### 3.3 `org.gnome.Shell.Introspect` is gated — we cannot use it

The obvious escape hatch is GNOME's own window-list DBus API. It is allowlisted.
`js/misc/introspect.js` on gnome-shell main:

```js
const APP_ALLOWLIST = [
    'org.freedesktop.impl.portal.desktop.gtk',
    'org.freedesktop.impl.portal.desktop.gnome',
];
```

and `GetWindowsAsync` runs every caller through `this._senderChecker.checkInvocation(invocation)`.
Third-party callers get "GetWindows is not allowed" — as
[reported on GNOME Discourse](https://discourse.gnome.org/t/unable-to-call-the-remote-getwindows-gnome-method-via-dbus/21201).
This is why every GNOME window-management tool (`window-calls`, `Window Calls Extended`, and
compass) ships a Shell extension.

### 3.4 Consequence: the Shell extension is load-bearing on our first target

Compass already depends on one, exposing `org.gnome.Shell.Extensions.Windows`
(`src/server/src/services/window-manager/gnome/`) and `org.gnome.Shell.Extensions.Clipboard`
(`src/server/src/services/clipboard/gnome/`). On GNOME 50/51 that is the **only** mechanism for
window switching, clipboard history and synthetic paste. The spec's headline goal — eliminating
GNOME Shell private APIs — is not achievable on the platform we are targeting first. See PLAN.md
§3.5 for the restated goal.

### 3.5 The one spec item that does land: GlobalShortcuts

From `xdg-desktop-portal-gnome`'s own [NEWS](https://github.com/GNOME/xdg-desktop-portal-gnome/blob/main/NEWS):

- **48.rc** — "Add global shortcuts portal backend"
- **49.beta** — "Improvements to the Global Shortcuts portal"
- **50.alpha** — "Properly send the Global Shortcut activation token to the portal frontend"
- **51.rc** — "Various fixed to the Global Shortcuts portal"

So on GNOME 50/51 `ashpd`'s GlobalShortcuts is implemented, actively maintained, and is the correct
hotkey path. (Secondhand claims that it is a no-op on GNOME 50 do not match the NEWS file; verify on
hardware in the Phase-0 spike.) Note it is inert under XWayland — native Wayland only.

Conversely, **`xdg-desktop-portal-wlr` ships no GlobalShortcuts backend at all**, so on Sway, river
and labwc the portal path does not exist. Hyprland has one via
[xdg-desktop-portal-hyprland](https://github.com/hyprwm/xdg-desktop-portal-hyprland). That inverts
the spec's ordering: `xx-hotkey-v1` from [#1936](https://github.com/tuna-os/compass/pull/1936) is
the *primary* path on wlroots, and the portal is primary on GNOME/KDE/Hyprland.

### 3.6 Also natively available on GNOME, and worth using

- `xdg-activation-v1` — raise/focus our own window and the app we launch.
- `keyboard-shortcuts-inhibit-v1` — grab keys while the launcher has focus. Compass already has a
  `shortcut-inhibit` service; keep it.
- `text-input-v3` — IME. Verify Iced/winit handles it before committing (PLAN.md §9 risk).
- Portals: OpenURI, FileChooser, Screenshot, Secret are well covered by `xdg-desktop-portal-gnome`.

### 3.7 Bluefin constraints

[Bluefin](https://projectbluefin.io/) is an immutable, OCI-composed Fedora Atomic desktop built by
[Universal Blue](https://universal-blue.org/): images are assembled in GitHub Actions by layering
RPMs, Flatpaks and config onto Fedora and pushed as signed OCI artifacts. There is **no traditional
package manager** — GUI apps are Flatpaks, CLI tools are Homebrew.

- Flatpak is the development target from day one, not a Phase-6 packaging step.
- `/dev/uinput` is unavailable, so the spec's `wtype`/`dotool` paste fallback is dead here.
- A sandboxed Flatpak cannot write `~/.local/share/gnome-shell/extensions/` by default — see
  PLAN.md §10.7, which blocks Phase 3.
- Host execution goes through `flatpak-spawn --host` / `OpenURI`; desktop-entry and icon indexing
  needs `--filesystem=host-os:ro` plus `~/.local/share/{applications,icons}`.
- GNOME 50 ships in Fedora 44; GNOME 51 releases 16 September 2026 and lands in Fedora 45. Plan for
  an extension-compat task every GNOME cycle.

## 4. Standing reference material

- [Wayland Explorer](https://wayland.app/protocols/) — canonical per-protocol registry with a
  live compositor support matrix. **Check every protocol here before planning around it**; §3 is
  what happens when you don't.
- [Wayland Bites](https://wayland-bites.github.io/) — X.Org-feature-to-Wayland-compositor coverage
  matrix across Mutter, KWin, wlroots, Mir, Enlightenment and Arcan. The best single page for
  "which desktop will this break on".
- [awesome-wayland](https://github.com/rcalixte/awesome-wayland) — curated index; useful for finding
  the current implementation of a given protocol.
- [ArchWiki: XDG Desktop Portal](https://wiki.archlinux.org/title/XDG_Desktop_Portal) — which backend
  provides which interface, and the `portals.conf` mechanics we'll need for `vicinae doctor`.
- [xdg-desktop-portal-hyprland](https://github.com/hyprwm/xdg-desktop-portal-hyprland) — the most
  complete non-GNOME/KDE portal backend; read its GlobalShortcuts implementation.
- [Raycast extensions monorepo](https://github.com/raycast/extensions) — the Suite-1 conformance
  corpus. Thousands of real extensions; take the top 25 by installs as the smoke set and keep the
  rest as a nightly soak.
- [Raycast API docs](https://developers.raycast.com/) — the contract `src/typescript` implements.
- [`landlock` docs](https://docs.rs/landlock) and the
  [kernel Landlock documentation](https://docs.kernel.org/userspace-api/landlock.html) — for the
  Phase 4 FS boundary.
- [Firecracker's seccomp filters](https://github.com/firecracker-microvm/firecracker/tree/main/resources/seccomp)
  — a production example of the JSON allowlist format `seccompiler` consumes. A good starting shape
  for our extension-worker filter.

---

## 5. What to read first

If someone is picking this up cold, in order:

1. `src/lib/xdgpp/tests/` and `src/lib/fuzzy/tests/` in this repo — the behaviour we must preserve.
2. [pop-os/launcher](https://github.com/pop-os/launcher)'s protocol types — the IPC design already solved.
3. §3 of this document, in full — it is why Phase 1 looks the way it does.
4. [Icelauncher](https://github.com/HaruNashii/Icelauncher) end to end — one afternoon, and it is most of Phase 1.
5. `js/misc/introspect.js` in gnome-shell and `src/meson.build` in mutter — 20 minutes, and they settle every "can't we just…" question about GNOME.
6. [Wayland Explorer](https://wayland.app/protocols/) for each protocol in PLAN.md §2 — before writing any of it.
