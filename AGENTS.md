Compass is a launcher for the Linux desktop, a fork of Vicinae. This file explains the general rules. For everything else, look at the codebase.

The engine is Rust: the Cargo workspace in `crates/`. The UI is Iced (`compass-ui`). React/TypeScript under `src/typescript` powers the extension API and runtime. The C++/Qt engine was removed ([ADR-0021](docs/rust-engine/adr/0021-remove-the-cpp-engine.md)). Upstream Vicinae releases are now the behavioural and benchmark reference.

For development, use `make dev`: it builds the workspace and runs `cargo run -p compass -- start`. The toolchain is pinned in `rust-toolchain.toml`. The MSRV is `rust-version` in `Cargo.toml`.

## Separation of concerns

`compass-ui` is presentation. It turns state into Iced widgets and turns input into messages. The logic lives in the other crates:

- `compass-core`: application state and the decisions about it (config, search and ranking, commands, calculator, snippets, themes and so on). It has no UI dependency.
- `compass-search`: fuzzy matching.
- `compass-db`, `compass-clipboard`, `compass-local-storage`, `compass-oauth-store`: storage.
- `compass-ipc`, `compass-worker-host`, `compass-extension-api`, `compass-script`, `compass-sandbox`: IPC and the extension tiers.
- `compass-platform` and its backends: platform services.
- `compass`: the binary that wires them together.

A decision that a test would want to check belongs in a crate that can be tested without a display. Be pragmatic about it: a few lines of layout arithmetic or hover state belong in the view.

## Rust rules

- **Crates first.** Prefer maintained, off-the-shelf crates to hand-rolled Rust. Before you port an upstream feature or write a general-purpose utility (parsing, encoding, framing, D-Bus clients, formats), look for a crate and use it, and handle small gaps around it rather than reimplementing it. For example, the calculator uses `fend-core` rather than a port of Numen. Hand-roll only when every candidate would regress documented behaviour, and say why in `docs/rust-engine/CRATE-AUDIT.md`. Record behaviour differences from upstream in `docs/rust-engine/PARITY.md`.
- **No `unsafe`.** The workspace sets `unsafe_code = "forbid"`. The only exceptions are `compass-sqlcipher-sys`, `compass-wayland-protocols` (its generated interface tables) and `compass-wayland-foreign`, which opt out of the workspace lints in their `Cargo.toml` and say why ([ADR-0019](docs/rust-engine/adr/0019-an-unsafe-bridge-for-the-launchers-surface.md)). New `unsafe` goes into a crate like those, split out for that reason alone. It never goes into a shared crate.
- **Lints.** Every crate uses `[lints] workspace = true` unless it has a documented reason not to. Clippy's `all` group is denied, and CI runs `cargo clippy --workspace --all-targets -- -D warnings`. Fix the code rather than adding an `#[allow]`. When an allow is the right call, scope it to the item and say why in a comment.
- **Docs.** Most library crates set `#![deny(missing_docs)]`, and CI builds `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS="-D warnings"`, so a broken intra-doc link fails the build. The module docs carry the reasoning in this repository, so keep them true when you change what they describe.
- **Absence** is `Option`, not a sentinel such as an empty string or `-1`, unless an external format fixes the sentinel.
- **Errors**: `thiserror` enums in library crates. Log through `tracing`, never `println!`, except for a CLI's actual output.
- **Copies**: borrow (`&str`, `&[T]`, `&Path`) where you can and move where you own. Clone when that is the clear and safe option; don't contort the code to avoid it.
- **Constants** are `UPPER_SNAKE_CASE`. Use `const` and `const fn` where they work.
- **Search is fuzzy** unless there is a good reason for it not to be. Use `compass-search` rather than `contains`.
- **Formats stay upstream's.** The database migrations, the stored enum values, the crypto key labels and the keyring entries must match upstream Vicinae, so an existing profile keeps working. Renaming the product never renames them (ADR-0020, ADR-0021).

## Cross-platform

Linux is the only target today ([ADR-0007](docs/rust-engine/adr/0007-fork-posture-and-platform-scope.md)), but the crates are laid out so that macOS and Windows can be added as backends. `compass-platform` says what the platform can be asked to do, and `compass-platform-linux` implements it. Shared crates must not depend on Linux-specific ones (Wayland, portals, the GNOME Shell helper, zbus, and so on).

`crates/compass-platform/tests/the_seam_holds.rs` enforces this by reading the manifests. A crate that is legitimately Linux-bound goes on its `MAY_BE_LINUX_BOUND` list, with a comment that says why. Do not work around the test.

On Linux, use the platform's own mechanism, such as portals, logind, MPRIS or the Wayland protocols, rather than a hack. Linux-specific code belongs in a Linux-bound crate. When a shared crate can't avoid it, as with `compass-ui`'s layer-shell entry point, keep it small and behind `#[cfg(target_os = "linux")]`.

## Tests and seeing the launcher

The launcher is graphical and this repository is usually worked on without a compositor. The test ladder climbs from cheapest to most expensive, and each rung proves something the one below can't:

- `make test-fast`: t0 (logic) plus t1 (paint), the per-edit loop.
- `make test-t2`: a real Mutter session in a container.
- `make test-t3`, or `just vm-tier`: real GNOME in a VM, in CI.

Read [`docs/rust-engine/RENDER-HARNESSES.md`](docs/rust-engine/RENDER-HARNESSES.md) before you reach for any rung, and especially before you let a cheap one answer an expensive one's question. `make design` serves the browser surrogate for design work.

`make check-rust` runs what Rust CI runs: formatting, Clippy and the workspace tests.

## Formatting and linting

- Rust: `cargo fmt --all` (`.rustfmt.toml`: edition 2024, width 100). CI checks it with `cargo fmt --all -- --check`.
- TypeScript: Biome, under `src/typescript`.
- Nix: `alejandra`. CI runs `alejandra --check .`.

`make format` runs the Rust and TypeScript formatters. Always run it, and `make lint-rust`, before you finish a change.

Keep comments few. Use them to explain why something is the way it is, not to narrate the code.

## Internationalization

The Rust engine does not translate yet. [ADR-0003](docs/rust-engine/adr/0003-fluent-for-i18n.md) chose `fluent-rs`. The donated Qt Linguist catalogues are kept in `extra/translations/qt/` as input for `scripts/ts-to-ftl.py`, the one-off `.ts` to `.ftl` converter, and they have not been converted yet. Until Fluent is wired in, write user-visible strings so that the conversion stays mechanical:

- The source language is English, and the English text is the message.
- Keep each user-visible sentence whole. Put values in with `format!` placeholders, and never build a sentence by concatenating fragments. Plurals are separate full forms (`1 window` / `3 windows`), not `window(s)`.
- Don't translate stable technical notation: ids, acronyms, unit symbols (`MB`), file names, protocol names, icon names, log output, storage keys, and strings compared as discriminants.
- Keep brand and product names as they are ("Raycast Store", "Extension Store", "Compass").
- Use plain, concise desktop-launcher wording, and use the platform's own names for its settings and features.
- Never hand-edit `extra/translations/qt/*.ts`. They are history and converter input.

## Code generation

- IPC: the `.fig` IDL files in `figura/` are compiled by `crates/compass-figura` into the committed TypeScript bindings under `src/typescript/*/src/proto`. `make figen` regenerates them, and a test fails when they are stale.
- Static datasets: `make emoji` regenerates the glyph table in `crates/compass-core/glyph/`, and `make genicon` regenerates the icons. These run from time to time, not as part of the build. Commit the result.
- The `compass.json` schema: `COMPASS_UPDATE_SCHEMA=1 cargo test -p compass-core --test config_schema`.

## React/TypeScript extensions

Everything about the extension SDK and runtime lives under `src/typescript`. Read the `README.md` files there before you work on it. The SDK keeps its npm name `@vicinae/api` so that store extensions run unchanged (ADR-0020). `scripts/build-extension-runtime.sh` (`make extension-runtime`) builds the bundle that the engine runs.

## Build hygiene

Disk is often tight in agent containers. When you only need tests, set `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`, and prefer `cargo test -p <crate>` to the whole workspace.
