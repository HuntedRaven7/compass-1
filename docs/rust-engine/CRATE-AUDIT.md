# Crate audit: hand-written code a maintained crate could replace

**Date:** 2026-09-24. **Rule** (AGENTS.md): lean toward off-the-shelf crates over hand-rolled Rust.
Handle a small gap around a crate rather than reimplementing; hand-roll only when every candidate
would regress documented behaviour, and record behaviour differences in `PARITY.md`.

The workspace (~97k lines) already leans on crates where they fit: `nucleo-matcher`, `fend-core`,
`ignore`, `mime_guess`, `aes-gcm`/`hkdf`, `oo7`, `ashpd`, `landlock`/`seccompiler`, `roxmltree`,
`strsim`/`fst`, and iced's Markdown and SVG support. Most large modules (`compass-ui/app.rs`, the
indexer, the worker host) are project logic no crate covers. What a crate *can* replace is below,
with the decision taken and why.

| Code | Lines | Crate | Decision |
|---|---:|---|---|
| `compass-sqlcipher-sys` FFI wrapper, plus ~300 hand-bound `bind_*`/`column_*`/`step` sites in clipboard, db, local-storage, oauth-store and compass | ~800 + sites | `rusqlite` 0.40 with `bundled-sqlcipher` | **Replaced.** PLAN §12.0 item 2(b) had deferred this because the bundled SQLCipher was 4.6.1 against our vendored 4.16.0; `libsqlite3-sys` 0.38.2 bundles **SQLCipher 4.14.0** (SQLite 3.51.3), same SQLCipher 4 defaults. `compass_sqlcipher_sys::open` now keys, registers the tokenizer (the crate's only `unsafe`, through `Connection::handle`, compiled against `libsqlite3-sys`'s headers) and applies the pinned pragmas, and returns a `rusqlite::Connection`; the FFI, `Database`, `Statement` and `Transaction` are deleted and every call site uses `named_params!`, `query_row`, `query_map` and `prepare_cached`. On Linux it links the system `libcrypto`, as before. |
| Percent-encoding, five copies (two byte-identical) | ~140 | `percent-encoding` 2.3 | **Replaced.** Shared sets in `compass_core::uri`; the lenient image-URL decoder and the strict bookmark/file-chooser decoders keep their behaviour. |
| `compass-ipc` frame codec | ~90 of logic | `tokio-util` `LengthDelimitedCodec` | **Replaced.** The framing is the crate's; the codec keeps postcard on either side and reads the prefix at a frame boundary so an oversized frame is still reported with its length. |
| `compass-notify` (whole crate, 185 lines + tests) | ~25 of logic | `notify-rust` 4.18 (same `zbus` 5.19, no duplicate) | **Replaced; the crate is deleted.** Its tests checked the `Notify` call's argument layout, which is now the crate's job, and it brings macOS and Windows notifications for later phases. |
| `compass-power`, `compass-media` raw `call_method` | ~950 total | `#[zbus::proxy]` definitions, zbus's own `fdo` proxies; `mpris` 2.x is sync over libdbus | **Replaced the calls, kept the parsing:** logind, GNOME SessionManager, Plasma Shutdown and the MPRIS player are typed proxies, `ListNames`/`GetAll` are `zbus::fdo`; the MPRIS metadata corner cases, the call timeouts and the logind capability fix stay. |
| `compass-xdg` desktop-entry reader, entry, locale | ~1280 | `freedesktop-desktop-entry` 0.8 (MPL-2.0) | **Keep** (PLAN §12.0 item 5). Its `Exec` expansion splits on whitespace with no quoting, which would regress PARITY `compass-xdg` #3, and its directory discovery misses the Flatpak host roots (#95). |
| `compass-xdg/icon.rs` | ~420 | `freedesktop-icons` 0.4 | **Keep.** No way to add the Flatpak host roots without mutating the process environment. |
| `compass-xdg` mimeapps, MIME subclasses | ~790 | `xdg-mime` 0.4 | **Keep.** Nothing mature covers the desktop-prefixed list order. |
| `compass-search/translit.rs` | 218 | `deunicode`, `any_ascii` | **Keep the table** (deliberate under-expansion keeps subsequence matching); **`deunicode` adopted as the fold fallback** for Ł/đ/ħ (closes PARITY `compass-search` #3). |
| `compass-core/contrast.rs` | 268 | `palette` 0.7 | **Keep**; about 80 lines are replaceable, the contrast search and Qt-HSL rounding are not. |
| `slug.rs` | 89 | `slug` 0.1 | **Replaced.** Same results on every pinned case, and it transliterates non-Latin titles where the C++ emptied them (PARITY `compass-core::slug`). |
| `semver.rs` | 130 | `semver` | **Keep.** It is not semver: it takes any number of dotted integers (`1.2`, `2026.09.24`), which the crate refuses; the tag format decides. |
| `compass-script` (Rhai tier, new) | ~2400 | `rhai`, `notify`, `toml`, `time`, `uuid`, `getrandom`, `percent-encoding` | **Crates throughout.** The script language, file watching, manifest parsing, date formatting, UUIDs and URL encoding are all crates; what is hand-written is the view mapping and the sandbox policy. `toml` is the 1.x line, which shares its parser with what the workspace already locks. |
| `qt_date.rs` (a snippet's `{date format=…}`) | ~150 | `jiff`, `time`, `chrono` strftime | **Keep the formatter; `jiff` supplies the clock.** Stored snippets hold Qt format strings (`yyyy-MM-dd hh:mm`); every crate has its own syntax, so a translation would need the same tokenizer. `jiff` is used for the local time and zone, which `time` cannot give a multi-threaded process. |
| Browse Fonts' font enumeration (`compass/fonts.rs`) | ~60 | `fontdb` 0.23, `ttf-parser` 0.25 (both already in the tree through iced's text stack); `fontconfig` bindings | **Crates used.** `fontdb` lists the families the renderer will find, and `ttf-parser` answers the cmap lookups that decide each family's scripts. Binding libfontconfig for its language sets would add a C dependency the port otherwise avoids and would not exist on macOS or Windows; the difference is PARITY "Browse Fonts" #1. |
| User theme files (`compass-core::theme_file`) | ~350 | `toml` 1 (already in the tree for Rhai manifests); `palette` for `QColor::lighter`/`darker` | **`toml` used; the colour steps hand-written.** Parsing is the crate's. `palette` would add a dependency for two HSV scalings and a mix, about 40 lines, whose Qt-specific overflow rule (value past 1.0 is taken from the saturation) it does not have. |
| Reading and writing a hand-edited `compass.json` (`compass-core::config_issues`, `config_edit`; `compass::config_watch`) | ~450 | `serde_path_to_error` 0.1 (dtolnay); `jsonc-parser` 0.33 with `cst` (already in the tree for the settings migration); `strsim` (already in the tree); `notify` (already in the tree) | **Crates used.** `serde_path_to_error` names the key a wrongly typed value is at, so that value alone is set aside and the rest of the file loads; `jsonc-parser`'s concrete syntax tree edits the file in place, so a write keeps its key order, indentation and comments; `strsim` gives the "did you mean"; `notify` watches the file's directory (editors save by renaming). Hand-written: which section keeps a set-aside value, and the walk of the document against the published schema that names unknown keys. |
| `compass-core/key_combo.rs` (a root item's shortcut: `Keyboard::Shortcut`'s spelling, the recorder's chord tracking) | ~250 | `global-hotkey`, `keyboard-types` | **Hand-rolled.** The stored form is the C++'s (`control+shift+A`, its `keyMap` names and `modifierMap` aliases, a modifier alone), which both crates spell differently (`KeyA`, `Control`); converting through them would be the same table plus a second one. Iced already delivers the key events. |
| interval parsers, `script_output.rs` | ~300 | `humantime`, `vte` | **Keep.** `parse_interval` is 25 lines for Raycast's one-unit manifest syntax (`30s`, `5m`); `humantime` would also accept `1h30m` and `5 minutes`, which Raycast rejects, and change the error text. The tokenizer finds links and SGR codes in one resumable pass; `vte` would replace only the escape half and handles malformed sequences differently from the C++ it ports. |

Not replaceable, checked: crypto (already crates), keyring (`oo7`), XDG base dirs (`dirs` plus
Flatpak-specific roots), the uinput keyboard (a protocol model), clipboard filtering, file walking
(`ignore`), MIME detection (`mime_guess`), the calculator (`fend-core`).

The HTTP client is `ureq` 3 with native-tls (remote images in extension views): blocking is all a
fetch on a worker thread needs, and native-tls verifies against the system's roots through the
OpenSSL already linked for SQLCipher. URL parsing (the OAuth redirect) is `url`, already in the tree.
`oauth2` was considered and not used: Raycast's PKCE client builds the request and exchanges the
code in the extension, so the host never speaks OAuth itself.

## Phase 5 Track B: the wlroots family (2026-09-24)

| Need | Crate | Decision |
|---|---|---|
| Launcher as a `wlr-layer-shell` surface | `iced_layershell` 0.19.1 (built against `iced_core`/`iced_runtime` **0.14**, so it runs our `LauncherApp` unchanged) | **Used**, in `compass-ui` under a `cfg(target_os = "linux")` target section, default features off (its only default, `mundy` theme detection, duplicates `crate::appearance`). Two costs, both recorded: its `layershellev` needs **`libxkbcommon` headers at build time** (`pkg-config xkbcommon`; the Flatpak SDK has them, Ubuntu CI installs `libxkbcommon-dev`); and its unconditional `iced_exdevtools` 0.19.1 fails to compile against `winit-core` **0.31.0-beta.3** (a new `NativeKeyCode` variant), which `^0.31.0-beta.2` admits — `Cargo.lock` pins `winit-core`/`winit-common` to `0.31.0-beta.2`. A plain `cargo update` would re-break it; drop the pin once `iced_exdevtools` matches exhaustively. |
| Protocol bindings: foreign-toplevel, data-control (ext + wlr) | `wayland-protocols` 0.32 (`staging`), `wayland-protocols-wlr` 0.3 | **Used.** `compass-wayland` moved from `wayland-client` 0.30 to 0.31, the version winit, `iced_layershell` and `wl-clipboard-rs` already link, so the tree carries one Wayland stack for these rather than two. |
| Window list / activate / close | none maintained: crates.io has taskbar applications and screenshot tools that each embed their own client, no library | **Hand-rolled on the protocol crates** (`compass_wayland::toplevel`, ~450 lines incl. tests): a registry bind, one `Dispatch` per interface, a snapshot behind a mutex. |
| Set / read / clear the selection without focus | `wl-clipboard-rs` 0.9 (the library under `wl-copy`/`wl-paste`; ext **and** wlr data-control, chosen at runtime) | **Used** for set (`copy_multi`, served from its own thread), read and clear. |
| *Watch* the selection (clipboard history) | `wl-clipboard-rs` cannot: each call is one connection, one operation. `wayland-clipboard-listener` 0.6 can, but picks ext **or** wlr data-control at **compile time** (Sway before 1.11 has only wlr, newer compositors increasingly only ext), panics on a dispatch error inside its `Iterator`, reads one MIME type per selection, and has ~700 recent downloads | **Hand-rolled the watcher only** (`compass_wayland::clipboard::watch`, ~200 lines): one bound device over whichever manager is advertised, each selection filtered by the existing port of the C++ offer filter (`compass_wayland::data_control`) and read with a per-type timeout. Everything that is not watching stays on `wl-clipboard-rs`. |
| `xx-hotkey-v1` bindings | no crate carries this experimental protocol (it is upstream vicinae's own, #1936) | **Generated, not written**: `wayland-scanner` 0.31 — the generator `wayland-protocols` itself uses — over the checked-in XML, in the new `compass-wayland-protocols` crate. It is a separate crate only because generated interface tables contain `unsafe` and the workspace forbids it; that crate uses `deny` and allows it on the generated module alone. |

### Compositor IPC: Hyprland and niri (2026-09-25)

| Need | Crate | Decision |
|---|---|---|
| niri's socket: `Windows`, `Workspaces`, `FocusedWindow`, the focus/close/workspace actions | `niri-ipc` 26.4 (niri's own crate, GPL-3.0-or-later; serde and serde_json only, default features off) | **Used** for every type: the requests are its `Request`/`Action`, the replies its `Reply`, `Window` and `Workspace`. The one gap is taken around it: `niri_ipc::socket::Socket` sets no timeout, so `compass_platform_linux::compositor::niri` opens the `UnixStream` itself, writes the crate's serialisation and reads one line under a 2 s timeout. Its structs require the fields niri 25.08+ sends (`layout`, `focus_timestamp`); an older niri answers in a shape it cannot read, which is reported and treated as no windows (PARITY "wlroots" #3). |
| Hyprland's request socket: `clients`, `workspaces`, `activeworkspace`, `activewindow`, `dispatch` | `hyprland` 0.4.0-beta.3 (hyprland-rs) | **Hand-rolled** (`compass_platform_linux::compositor::hyprland`, ~300 lines incl. docs). The wire is "connect, write one command, read to EOF", so what a crate would bring is the data types and dispatchers, and both regress the C++: its `Client` is strict and narrow — `at`/`size` as `i16`, `focusHistoryID` as `i8`, a required `fullscreen` enum and `swallowing` address — where the C++ reads eight fields leniently, so a Hyprland release that adds or retypes a field fails the whole list; and its dispatchers speak the classic `dispatch focuswindow address:…` while the C++ dispatches Hyprland's Lua expressions (`hl.dsp.focus({ window = … })`). It is also a beta, pulling `tokio`, `async-stream`, `derive_more` and a proc-macro crate. The replies here are `serde` structs with every field optional and unknown ones ignored. |

### The gaps pass, KDE: KWin (2026-09-25)

| Need | Crate | Decision |
|---|---|---|
| KWin's scripting interface, its virtual desktops, kglobalaccel, and serving the tracker's callbacks | `zbus` 5 (already in the tree) | **Used** for all of it: dynamic `zbus::Proxy` calls to `org.kde.kwin.Scripting`/`org.kde.kwin.Script`, `org.kde.KWin.VirtualDesktopManager`'s properties and `org.kde.kglobalaccel.Component`, and a `#[zbus::interface]` object for `org.vicinae.WindowTracker`; the name watch is `zbus::fdo::DBusProxy`'s `NameOwnerChanged`. No KWin client crate exists on crates.io (the KDE-side crates are Qt bindings); what is hand-written (`compass_platform_linux::compositor::kwin`) is the tracker's cache and the two scripts, which are the C++'s JavaScript. |
| The script file KWin reads | `tempfile` 3 (already a workspace dependency) | **Used**, as `QTemporaryFile` in the C++. |

## Phase 5 Track A: the extension stores (2026-09-24)

| Need | Crate | Decision |
|---|---|---|
| Fetch the listings, READMEs and bundles | `ureq` 3.4 (already in the tree for remote images) | **Reused**, blocking on tokio's blocking pool with the same native-tls agent. Its workspace feature moved from `native-tls-no-default` to `native-tls`: ureq 3.4 tests `cfg!(feature = "native-tls")` before using the provider, so the old spelling panicked on every `https` request — remote images in extension views included, which no test had fetched over TLS. The cost is `webpki-root-certs`, compiled in and unused (the agents ask for the platform's roots). |
| Unpack a store bundle | `zip` 8.6, `deflate-flate2-zlib-rs` only (flate2 and zlib-rs were already in the tree) | **Used.** `enclosed_name` for the path check, `is_symlink` for links, the reader's CRC-32 check on every entry; the size caps, the refusal of absolute names that `enclosed_name` would quietly make relative, and the staging order are ours (`compass_core::store_bundle`). The C++ `Unzipper` (vendored minizip) is not ported. |
| A local store for the engine's end-to-end tests | `tiny_http` 0.12 (dev only) | **Used**: a real HTTP server on `127.0.0.1:0` serving fixture listings and zips, so the tests go through ureq, the listing parsers and the unpacker exactly as a real install does, without the network. |
| A `null` where the stores' JSON usually has a string | serde's `Option` | **A 5-line `deserialize_with`** in each store model, rather than `serde_with`'s `DefaultOnNull`: one helper, applied to every non-optional field, is less than a new dependency. |

## Phase 5 Track A: the input server and snippet keyword expansion (2026-09-25)

| Need | Crate | Decision |
|---|---|---|
| Read `/dev/input/event*`; create the `/dev/uinput` virtual keyboard | `evdev` 0.13 (safe wrappers over the ioctls; new deps `bitvec`, `funty`, `radium`, `tap`, `wyz`) | **Used** (`compass-input-server::device`). Nodes are opened read-only by us and handed over with `Device::from_fd` (the crate's own `open` tries read-write first); `VirtualDevice::emit` appends a `SYN_REPORT` to every write, so the sink batches the protocol model's events up to its `Sync` and emits the batch, which gives the reader the C++'s exact press–sync–release–sync sequence. No `unsafe`, no hand-written ioctls. |
| Keycode → text, modifier state, the virtual keyboard's character map | `xkbcommon` 0.8 (already locked through `smithay-client-toolkit`), default features off | **Used** (`compass-input-server::keymap`), as the C++ uses libxkbcommon: `key_get_utf8` before `update_key`, `mod_name_is_active(…, STATE_MODS_DEPRESSED)`, and the char map read unshifted then with Left Shift down. |
| Which nodes are keyboards and pointers; hot-plug | `udev` crate (libudev bindings) considered | **Not used.** It would add a build-time `libudev` dependency to every `cargo build --workspace` (CI, the Flatpak SDK) for a lookup of `ID_INPUT_KEYBOARD`/`ID_INPUT_MOUSE`, tags udev's `input_id` builtin computes from the capability bits. `device::is_keyboard`/`is_pointer` apply those tests to the bits `evdev` already reads, and hot-plug is a `notify` (inotify) watch on `/dev/input` — the C++ acts on nothing but "a node appeared" either. Differences: PARITY "Input server" #1. |
| The engine ↔ helper wire | `serde_json` (figura's JSON-RPC shape by hand, ~150 lines) | **Kept byte-compatible with the C++** rather than moved to `compass-ipc`'s postcard: either engine can drive either helper while both ship (PLAN §5), and the NixOS module wraps one path for both. Framing is the existing `compass_core::input_server::frame`/`MessageBuffer` (little-endian, as the C++'s `reinterpret_cast`). |
| Process supervision | `tokio::process` | **Used**, with the existing `RestartPolicy` port. |

## The gaps pass: icons, notifications and the tray (2026-09-25)

| Need | Crate | Decision |
|---|---|---|
| A file's MIME type, for its file-type icon (`renderFileIcon`) | `mime_guess` 2 (already in the tree through `compass-xdg`); `xdg-mime` 0.4 and `tree_magic_mini` 3 considered | **`mime_guess` used; the icon names hand-written** (`compass_ui::icons::names_for_mime`, 12 lines: `/` made `-`, and `<media>-x-generic`, shared-mime-info's defaults). `xdg-mime` reads the system database, globs, magic and `generic-icons` included, which would be exact, but it is unreleased since 2023, adds `nom` 7 and `dirs-next`, and parses the database on the window's first file row. `tree_magic_mini` would sniff extensionless files but reads the same system database at runtime. Both differences are PARITY "The gaps pass, icons and tray". |
| A `data:` URL notification icon | `data-url` 0.3 (already in the tree through usvg) | **Used**: `DataUrl::process` and `decode_to_vec`, base64 or percent-encoded, with its MIME type. |
| Fitting a PNG or JPEG into the notification's 128×128 PNG | `image` 0.25, PNG and JPEG only (the decoders `compass-ui` already builds) | **Used**: `load_from_memory`, `resize` (aspect kept), `imageops::overlay` to centre it. |
| The StatusNotifierItem host and watcher (`SniTrayHost`, `SniWatcher`, ~800 lines of C++) | `system-tray` 0.8 (ironbar's; on the zbus 5 already in the tree, one new package); `stray` 0.1 (zbus 3, unmaintained since 2023) and `ksni` (the *item* side) considered | **Used** (`compass::tray_host`). It serves the watcher when the name is free, registers a host, and follows items, their properties and their `dbusmenu` layouts. Handled around it: its `Activate` assumes `/StatusNotifierItem`, so the item's path is read back from the watcher's `RegisteredStatusNotifierItems`; `IconThemePath` is searched with the existing `best_icon`; pixmaps become PNGs with `image`. Its watcher has no claim grace and refuses `busname/path` registrations (PARITY "The gaps pass, icons and tray"). |

## The gaps pass, UI (2026-09-25)

| Need | Crate | Decision |
|---|---|---|
| Clipping an image to a circle or a rounded rectangle (`applyCircleMask`, `applyRoundedRectMask`) | `image` 0.25 (PNG, JPEG) and `resvg` 0.45 (SVG), both already in the tree; Iced's `image::border_radius` considered | **The crates decode and draw; the clip is hand-written** (`compass_ui::icons::apply_mask`, about 30 lines of coverage over the pixels). `border_radius` only rounds the widget's rectangle, cannot make the inscribed circle of a non-square image, and the tiny-skia renderer (the software fallback and the paint tier) ignores it on images; no maintained crate clips straight-alpha RGBA to these two shapes. |
| A tile's gradient stops (`applyBackdrop`'s `shifted` in HSL) | `palette` considered | **Hand-written** (`compass_ui::icons::shifted`, the HSL round trip, about 40 lines): `palette` would be a new package for one conversion, and `compass_core::contrast`'s HSL is on Qt's integer scale, which loses the 0.025-hue steps. |
| Which service a favicon comes from (`FaviconService`) | none needed | **Two URL templates** (`compass_core::favicon`); the fetch and the cache are `compass_ui::remote_image`'s (`ureq`, `sha2`). |
| Colouring a Markdown code block by its language (KSyntaxHighlighting) | Iced 0.14's `highlighter` feature: `iced_highlighter` over `syntect` 5 with `two-face`'s grammars (`fancy-regex`, pure Rust); six new packages | **Used**: `markdown::parse` highlights fenced blocks itself. The theme is Base16 Ocean, which Iced fixes (PARITY "The gaps pass, UI"). |
| Finding search words in a text to mark them (`MatchHighlighter`) | `aho-corasick` considered | **Hand-written** (`compass_search::term_ranges`, about 30 lines over the matcher's own case and diacritic fold): the fold has to keep one character for one character so a match's position is the text's, which a byte-level automaton over the raw text cannot do, and folding first means mapping positions back anyway. |
| A snippet's text with its backslash escape (`PlaceholderString::parse`) | the quicklink parser already ported | **Hand-written, as the quicklink parser is** (`compass_core::placeholder`): the C++ grammar is a bespoke state machine (ids ending at the first non-alphanumeric, first-wins keys, quoted values, a cut-off placeholder dropped) that no templating crate reproduces. |

## The gaps pass: settings (2026-09-25)

| Need | Crate | Decision |
|---|---|---|
| Reading a setting of `compass.json` by its dotted key (`launcher.clock.format`) | `serde_json`'s JSON Pointer (`Value::pointer`) | **Used** (`Config::get_path`): the dotted key becomes a pointer, `~` and `/` escaped. |
| Writing one, creating the objects on the way and dropping the ones a reset leaves empty | `serde_json` (`pointer_mut` finds, but does not create); `json-patch` 4 considered | **Hand-rolled** (`Config::set_path`, ~40 lines over `serde_json::Map`), then read back through `Config`'s own `Deserialize`, so a wrongly typed known key is refused. `json-patch`'s `add` needs every parent to exist and its `remove` leaves empty parents behind, which would need the same walk around it. |
| A `vicinae://settings/open?tab=` deeplink | `url` 2 (already used by `parse_launch_link`) | **Used**: `Url::parse` and `query_pairs`, which percent-decode the tab. |
| The settings view's controls (switch, list, text field, buttons) | `iced` 0.14's `toggler`, `pick_list`, `text_input`, `button` | **Used**; the shortcut recorder is `compass_ui::shortcut_recorder`, the sidebar `compass_ui::settings::SidebarModel`. |

## The gaps pass, HUD and onboarding (2026-09-25)

| Need | Crate | Decision |
|---|---|---|
| The HUD's surface | `iced_layershell` 0.19 (already the launcher's layer-shell runtime) | **Used**: a second `NewLayerShell` surface with no keyboard interactivity and `events_transparent`, as `HudWindowLayerShell.qml`. No new crate. |
| The onboarding record's time stamp | `jiff` 0.2 (already in `compass-ui`) | **Used**: `Timestamp::now().round(Unit::Second)`, which prints Qt's `ISODate` form. |
| `/etc/os-release`'s `PRETTY_NAME` and `VERSION`, for the bug report | `os-release` 0.1, `etc-os-release` 0.1 | **Hand-read** (`compass_core::bug_report::parse_os_release`, ~20 lines): two `KEY=value` lines unquoted, where either crate adds a file reader and error types this does not need and a new package to the Flatpak's sources. |
| The bug-report link's query | `url` 2 (already in `compass-core`) | **Used**: `Url::parse_with_params`. |
| `ext-background-effect-v1` (window-material) | `wayland-protocols` 0.32 `staging` (already in `compass-wayland`) | **Used** (`compass_wayland::material`): the generated client; only the rounded region (`createRoundedRegion`'s loop) is ours. Reaching the launcher's own `wl_surface` needs `wayland-backend`'s `client_system` foreign-display bridge, which is `unsafe`: done since in `compass-wayland-foreign` ("The window-material pass" below, ADR-0019). |

## The gaps pass, global shortcuts (2026-09-25)

| Need | Crate | Decision |
|---|---|---|
| The GlobalShortcuts portal, rebinding the whole set as the configuration changes | `ashpd` 0.13 (already the portal client) | **Used** (`compass_portals::ShortcutBinder`): a set is bound on a fresh `CreateSession` after `Session.Close` on the last, since `BindShortcuts` cannot take a shortcut back or change a trigger. The binder is ~70 lines of session bookkeeping around the crate's calls. |
| `vicinae-hotkey-v1` bindings | no crate carries Vicinae's own protocol | **Generated, not written**: `wayland-scanner` over the checked-in XML in `compass-wayland-protocols`, as for `xx-hotkey-v1`. The client (`compass_wayland::hotkey::HotkeyClient`, both protocols over one connection) is hand-written on the generated code, as `compass_wayland::toplevel` is on `wayland-protocols`: there is nothing else to use. |
| A fake compositor for the hotkey clients' tests | `wayland-server` 0.31 (Smithay's; the server half of the `wayland-client` stack already locked, one new package) | **Used, tests only**: `compass-wayland-protocols`' `server` feature generates the compositor side of both protocols, and `compass-wayland/tests/hotkey_protocols.rs` drives the real client against it over a socket pair. |
| A key's XKB keysym, and its name for the portal's trigger | `xkeysym` 0.2 (already locked through `smithay-client-toolkit`; no dependencies); `xkbcommon` considered | **Used** (`compass_core::global_shortcuts::{keysym, portal_trigger}`): its constants and `from_char` for the keysym, `Keysym::name` for the XKB name the "shortcuts" specification wants. The table from the recorder's key names to keysyms is the C++'s `xkbKeysymForQtKey`, 20 lines. `xkbcommon` would link libxkbcommon into `compass-core` for the same two lookups. |
| Diffing what is bound against what the configuration asks for (`reconcile`) | none needed | **Hand-written** (`compass_core::global_shortcuts::Reconciler`, ~60 lines over two `BTreeMap`s), the C++'s own two-map diff. |

## The gaps pass, wlroots paste and inhibit (2026-09-25)

| Need | Crate | Decision |
|---|---|---|
| Pressing the paste chord without the input server | `wayland-protocols-misc` 0.3 (`zwp_virtual_keyboard_v1`; already in the tree through `layershellev` and smithay-client-toolkit) | **Used** (`compass_wayland::virtual_keyboard`): the generated client. The keymap is a fixed three-key xkb string, as `wtype` sends; compiling one with `xkbcommon` would add a C library to link for three keys. `wtype`/`ydotool` as programs were not taken: a runtime dependency to install for what is six requests. |
| The paste's focus wait | `compass_core::paste::PasteService` (ours, ported earlier) | **Used**: driven on one blocking thread per paste. |
| keyboard-shortcuts-inhibit | `wayland-protocols` 0.32 `unstable` (already in `compass-wayland`) | **Used** (`compass_wayland::keyboard_inhibit`). The launcher's `wl_surface` is reached without `unsafe` by sharing a `wayland-client` `Connection` with `iced_layershell` (`Settings::with_connection`) and reading `wl_keyboard.enter`, rather than `wayland-backend`'s foreign-display bridge. winit 0.30 takes no connection, so the `xdg_toplevel` presentation has none. |

## Product decisions: the update check (2026-09-25)

| Need | Crate | Decision |
|---|---|---|
| Reading `repos/tuna-os/compass/releases/latest` | `ureq` 3.4 (already the engine's HTTP client, for the stores) | **Reused** (`compass::updates::GithubFeed`): the stores' native-tls agent with a 15 s per-request timeout; the reply is `serde_json` into `compass_core::update::Release`. `octocrab` was not taken: an async GitHub client with its own `hyper` and TLS stack, for one unauthenticated GET. |
| Comparing the release's tag with the running version | `compass_core::semver` (the existing port); `semver` 1 considered | **Kept the port, no crate added.** The comparison was already present and is load-bearing in the way the crate is not: a tag with a suffix (`v1.2.0-rc1`) does not parse at all, which keeps release candidates from ever being offered, and `1.0` equals `1.0.0`; `semver` would accept the first (as a prerelease that orders below `1.2.0`) and refuse the second. GitHub's `releases/latest` already leaves out drafts and prereleases; the gates stay for a feed that does not. |
| Throttling and caching the check | `serde_json` (already in the tree) | **Hand-written** (`compass_core::update::CheckCache`, ~50 lines): a timestamp and the last release in one JSON file under the cache directory. |
| A local feed for the tests | `tiny_http` 0.12 (dev only, already used for the stores) | **Used**: `the_github_feed_reads_the_release_from_its_url` and the engine's end-to-end tests serve the release from `127.0.0.1:0`; no test reaches the network. |

## The gaps pass, currency (2026-09-25)

| Need | Crate | Decision |
|---|---|---|
| Currency conversion in the calculator | `fend-core` 1.5.8 (already the calculator) | **Used**: its own currency units and `Context::set_exchange_rate_handler_v2` (`ExchangeRateFnV2`), fed the ECB's rates per euro (`compass_core::calculator::RateHandler`, ten lines). Only gap handled around it: fend reads `€5` as one unknown word (it takes `£8` and `$5`), so a `€` right before a number is moved after it before evaluating. |
| Reading the ECB's `eurofxref-daily.xml` | `roxmltree` 0.20 (already a workspace dependency, through `compass-xdg` and `compass-shell`); `quick-xml` also locked | **Used** (`compass_core::exchange_rates::parse_ecb`): a read-only DOM over a two-kilobyte file is what `roxmltree` is for; `quick-xml`'s streaming reader would be more code for no gain. No package added to `Cargo.lock`, only the edge to `compass-core`, so the Flatpak sources are unchanged. |
| Fetching the file | `ureq` 3 (the engine's HTTP client, `compass::stores::get`) | **Used**: the same agent, timeout and TLS roots as the stores. |
| The cache file | `serde_json` (already) | **Used**: the rates, the ECB's date and the fetch time as JSON under `$XDG_CACHE_HOME/compass`, written through a partial file and a rename. |

## The gaps pass, tray and sandbox (2026-09-25)

| Need | Crate | Decision |
|---|---|---|
| Compass's own StatusNotifierItem and its `dbusmenu` (`TrayServiceLinux`, ~450 lines of C++) | `ksni` 0.3 (the maintained *item* side, on the zbus 5 already in the tree; one new package besides, `pastey`); `system-tray` (already used) considered and not fit, being the host side only; `tray-icon` (tauri's) considered and not taken, as it draws through GTK and libappindicator | **Used** (`compass::tray_icon`). It exports the item and the menu, owns `org.kde.StatusNotifierItem-<pid>-<n>` (or only its unique name inside a Flatpak, `disable_dbus_name`), registers with the watcher and again whenever one appears (`assume_sni_available`). Around it: the menu model is `compass_core::tray`'s, the pixmaps are drawn from `extra/compass.svg` with the `resvg` already in the tree, and the switch is a `watch` channel. |
| Which `$HOME` paths an extension may read, and whether a link among them stays inside the list | none needed | **Hand-written** (`compass_sandbox::home`, ~60 lines): `std::fs::canonicalize` and a comparison against the list's own resolved places. Landlock itself (the `landlock` crate, already used) enforces the grant. |

## The window-material pass: an unsafe bridge (2026-09-25)

| Need | Crate | Decision |
|---|---|---|
| The launcher's own `wl_surface` under winit, as a `wayland-client` proxy | Looked for first, since a safe bridge would make the exception unnecessary: `wayland-client` 0.31 (no safe adoption of a foreign proxy; `ObjectId::from_ptr` and `Backend::from_foreign_display` are `unsafe` by design), `smithay-client-toolkit` 0.19/0.20 (makes its own connection, adopts none), `raw-window-handle` 0.6 (only lends the pointers), winit 0.30 (no connection in or out), and a crates.io search for raw-handle/foreign-display Wayland bridges (toolkits and FFI bindings only). None offers it: adopting a pointer from another library cannot be made safe by a library, only checked. | **`wayland-backend` 0.3 `client_system` (already in the tree through winit), in a new crate, `compass-wayland-foreign`** (ADR-0019): `deny(unsafe_code)` with one function allowed and two `unsafe` blocks, `from_foreign_display` and `ObjectId::from_ptr`, behind a safe `bridge(&window)`. It names `wayland-backend`'s `client_system` and `dlopen` and `wayland-sys`'s `client` and `dlopen`, as winit builds them, so it links nothing new; `raw-window-handle` 0.6 gets its `std` feature (for `HandleError` as an error source). Cargo.lock gains only additions: the crate itself, its edges, `raw-window-handle` under `compass-platform`, and `wayland-server` under `wayland-protocols` (the dev-only `server` feature below); no new package from the registry. |
| The window's handles in `compass-ui` | `iced::window::run` (iced 0.14; lends `&dyn Window: HasWindowHandle + HasDisplayHandle`) | **Used**. `iced_layershell` 0.19 drops the `Run` action, so the layer-shell presentation lends nothing; there the safe route is the inhibitor's shared connection and `wl_keyboard.enter`, not built yet. |
| Where the card is, to blur behind it | `iced::widget::sensor` (iced 0.14) | **Used**: the card's size on show and on resize, keyed on translucency and corner radius so a change of either reports again; no layout code of ours. |
| The compositor side of `ext-background-effect-v1`, for a fake that blurs | `wayland-protocols` 0.32 `server` (dev only; `wayland-server` 0.31 was already a dev dependency) | **Used** (`compass-wayland/tests/background_effect.rs`): Sway has no such protocol, so the region traffic and the destroyed-surface rule are checked against an in-process compositor. |

## Raycast extensions written for macOS: the shim and the broker (2026-09-25)

`docs/rust-engine/RAYCAST-LINUX-SHIM.md` has the design; these are its build-or-buy calls. The
runtime is TypeScript, so the candidates are npm packages, and one constraint weighs on every one
of them: a runtime dependency changes `src/typescript/extension-manager/package-lock.json`, which
the Nix build pins by hash (`fetchNpmDeps` in `nix/vicinae.nix`) and the Flatpak builds offline
from. None was added.

| Need | Candidate | Decision |
|---|---|---|
| Telling a simple shell line from one that needs the shell, and taking it apart into words | `shell-quote` 1.8 (maintained); `string-argv` | **Hand-written** (`linux-shim/shell.ts`, ~110 lines). `shell-quote`'s `parse` expands `$VAR` from an env it is given and turns globs and operators into objects, so what it hands back is not the line's words; and a line that is re-joined from its output can mean something else. What the shim needs is narrower: which words, and where each command starts in the raw text, so a line that keeps its shell is changed only by replacing a command word in place. `string-argv` splits on quotes and knows no operators. |
| Intercepting `child_process` for one extension | `mock-spawn`, `proxyquire`-style module mocking; `node:module` hooks | **A `Proxy` over the real module**, returned from the runtime's existing `require` override (`patch-require.ts`). Mocking libraries replace the module for tests; the shim must keep every function it does not change real, and only for the extension's `require`. |
| A child process that is not one (`pbcopy` from the clipboard, `brew` from the host, a refusal) | none: every "fake child process" package is a test double | **Hand-written** (`linux-shim/fake-child.ts`, ~250 lines): an `EventEmitter` with Node's streams, `exit`/`close`/`error`, the `exec` callback and `util.promisify.custom`, the sync forms' return values and errors. |
| Answering `execSync("brew --prefix")` while the extension's thread is blocked | `synckit` (worker plus `Atomics.wait`); `deasync` (native) | **Node's own `worker_threads.receiveMessageOnPort` plus `Atomics.wait`** (`linux-shim/sync-rpc.ts`, ~90 lines). The extension already runs in a worker whose port carries the API's answers; `synckit` would start a second worker and `deasync` is a native addon, which the runtime ships none of. |
| Load-time source patches from the overrides manifest | `patch-package` (patches `node_modules` on install); monkey-patching `Module.prototype._compile` | **`module.registerHooks`** (Node 22.15+, synchronous, in-thread): the official hook for changing a module's source as it loads, for `require` and `import` alike. `patch-package` patches installed files, which a store update overwrites; `_compile` is private. |
| Running the brokered program on the host | `compass_platform_linux::host_command` (already: `flatpak-spawn --host` inside the Flatpak, a direct spawn outside) | **Used**, with `env PATH=… program args…` so the program is looked up on the host's side, Linuxbrew's prefix included; `std::process::Command` does the rest. |
| The grants file | `serde_json` (already), the Rhai consent file's write-then-rename | **Used** (`compass::host_commands::Grants`). |
| The runtime's unit tests | `vitest`, `jest`, `tsx` | **Node's own `node:test`**, over bundles `esbuild` (already the runtime's bundler) makes: `npm test` needs nothing the offline builds would have to fetch. |

## The robustness pass: two patched crates (2026-10-01)

Two QA blockers on the wlroots track (TIL-01, TIL-02) were bugs in crates under Iced, not in ours.
Both are fixed with a small patch to the crate, vendored under `vendor/` and wired in by
`[patch.crates-io]`, rather than by a replacement: the crates are otherwise right, and a
replacement would be the hand-rolling this audit exists to avoid. Each patch is a few lines,
marked `Compass patch` in the source, and each vendored manifest says when to drop it. Both are
excluded from the workspace, so they keep upstream's lints and `unsafe`, and
`packaging/flatpak/cargo-sources.json` no longer downloads them.

| Need | Crate | Decision |
|---|---|---|
| The clipboard Iced's two shells use, on the UI's own Wayland connection | `smithay-clipboard` 0.7.3 (latest; through `window_clipboard` 0.5, under both `iced_winit` and `iced_layershell`) | **Patched** (`vendor/smithay-clipboard`). It released its `wl_data_device` when the seat lost its keyboard, and its `Drop` released it too, which both shells trigger on every hide. A `data_offer` the compositor had already sent then reached a zombie device; libwayland 1.22 discards such an event without reserving the server-side id it carries, so the next offer failed "not a valid new object id" and killed the launcher's connection (`WAYLAND_DEBUG` shows the `release`, then `discarded [unknown]@…[event 0]`, then the fatal offer). The patch keeps a device for the seat's life and keeps one worker per display for the process, handed out to every `Clipboard::new`. A side effect: a copy made just before the launcher hides survives it. Moving the clipboard to a connection of its own was considered and refused: a client with no surface never has the keyboard focus, and the selection is only offered to the focused client. |
| Layer surfaces whose output goes away | `layershellev` 0.19.1 (latest; under `iced_layershell`) | **Patched** (`vendor/layershellev`). `output_destroyed` took the surface's unit out of its list before reporting it closed, so `iced_layershell` could not map the close to a window and dropped it; the launcher then waited for its window to close forever, and every `toggle` after hung (the engine holds its window slot while it waits). The patch asks for the close instead, which goes through the ordinary path while the unit still exists and then destroys the surface. |
| Ending a line of text that does not fit with an ellipsis | Iced 0.14 `text` (wraps or clips, no ellipsis); `cosmic-text` (has none either) | **Hand-written** (`compass_ui::elided`, ~120 lines with tests): a widget that shapes the line with the renderer's own `Paragraph` at layout, when its width is known, and bisects for the longest prefix that fits with `…`. Over `iced_core`'s widget traits directly, since `iced`'s `advanced` re-export would rebuild Iced with another feature set. |
| Supervising the launcher under `compass start` | `std::process` (already) | **Used**: `start` holds the lease and the engine and runs the window as `compass launcher-child`, starting it again when it fails (`compass::supervise`). The child notices `start` going away by its parent pid changing, which needs no signal handling (and so no `tokio` `signal` feature, which would rebuild the workspace) and no `prctl`, which is `unsafe`. |


## The accessibility tree (2026-11-05)

ADR-0022 §1 chose the route; these are the crates under it. All are MIT OR Apache-2.0.

| Need | Crate | Decision |
|---|---|---|
| A platform-neutral accessibility tree the launcher can describe itself as | `accesskit` 0.25 | **Used** (`compass_ui::semantics`, `compass_platform::AccessibilityTree`). Iced 0.14 builds none (iced-rs/iced#552), so the tree is described from the launcher's state; AccessKit's macOS and Windows adapters take the same tree. |
| Serving it on AT-SPI | `accesskit_unix` 0.24, with `accesskit_atspi_common` and `atspi` 0.29 under it; hand-written AT-SPI over zbus | **`accesskit_unix` used** (`compass-a11y-linux`), with its `tokio` feature rather than the default `async-io`: the workspace's zbus 5.19 has `tokio`, which then drives zbus's executor in every crate sharing it, and the adapter's `async-io` thread would have no tokio runtime for it. It shares the workspace's zbus and zvariant; new in the tree are `accesskit_consumer`, `atspi`'s three crates, `zbus_xml`, `zbus-lockstep`, `phf` 0.14 and `tokio-stream`. The protocol is not hand-written (AGENTS.md). |
| A text field's characters, as a screen reader moves through them | `unicode-segmentation` 1 (already in the tree under Iced's text stack) | **Used**: AccessKit wants each character's length, and a character is a grapheme cluster, not a byte or a `char`. |
| Reading the tree back in a test, as Orca would | `atspi` 0.29's client proxies (dev only, already under `accesskit_unix`); `pyatspi` | **`atspi` used** (`compass-a11y-linux/tests/private_bus.rs`) under `dbus-run-session`, with the system's `at-spi-bus-launcher` and `at-spi2-registryd`. `pyatspi` would make a Rust test depend on Python bindings. |
