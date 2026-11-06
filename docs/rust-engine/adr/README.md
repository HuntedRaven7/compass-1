# Architecture decision records

Short records of decisions that were expensive to make and would be expensive to revisit. Each
states what was decided, what it costs, and what would make us change our mind.

An ADR is not a promise. "Superseded by ADR-00NN" is a normal and healthy outcome; silently
drifting away from a recorded decision is not.

| ADR | Decision | Status |
|---|---|---|
| [0001](./0001-iced-over-slint.md) | Iced for the UI, not Slint | Accepted |
| [0002](./0002-postcard-over-capnproto.md) | postcard framing, not Cap'n Proto | Accepted |
| [0003](./0003-fluent-for-i18n.md) | fluent-rs, with the Qt Linguist catalogue converted | Accepted |
| [0004](./0004-gnome-shell-extension-distribution.md) | Extension via extensions.gnome.org, plus baked into Bluefin | Accepted |
| [0005](./0005-rhai-seam-now-tier-later.md) | Build the extension-API seam now; defer the Rhai tier | Accepted |
| [0006](./0006-fuzzy-coherence-classifier.md) | Reconstruct fzf's coherence signal over nucleo's indices | Accepted |
| [0007](./0007-fork-posture-and-platform-scope.md) | Hard fork in practice; Linux-first; naming superseded by ADR-0012, scope by ADR-0013 | Partly superseded |
| [0008](./0008-browser-control-is-an-extension.md) | Browser control is an extension, not part of the port | Accepted; transport chosen by ADR-0022 |
| [0009](./0009-controlled-input-echo-counter.md) | A controlled input's value carries the edit it answers | Accepted |
| [0010](./0010-corral-vm-tier.md) | The VM test tier is corral, on hosted runners | Accepted |
| [0011](./0011-the-window-is-its-own-command.md) | The launcher window is its own command, not the engine's | Accepted |
| [0012](./0012-compass-public-brand.md) | Compass is the public brand; legacy identifiers migrate at cutover | Accepted; compatibility list superseded by ADR-0020 |
| [0013](./0013-qt-leaves-the-repository.md) | Qt leaves the repository; Linux-first is a sequence, not a scope limit | Accepted |
| [0014](./0014-clipboard-storage-is-sqlcipher-plus-a-vendored-tokenizer.md) | The clipboard store is SQLCipher plus a vendored FTS5 tokenizer; the Rust engine links both | Superseded by ADR-0017 |
| [0015](./0015-the-launcher-window-is-resident.md) | The launcher window is resident and `serve` summons it; amends ADR-0011 | Accepted |
| [0016](./0016-a11y-gap.md) | Screen-reader gap — Orca cannot see the Rust launcher | Amended by ADR-0018 and ADR-0022; the tree is on AT-SPI and Orca reads it, checked per PR (#305); the manual Bluefin pass remains |
| [0017](./0017-a-new-launcher-not-a-reimplementation.md) | Compass is a new launcher, not a reimplementation; quality asserted absolutely, crates first, user data imported | Accepted |
| [0018](./0018-the-open-calls-decided.md) | Screen-reader users keep the accessible engine until the Rust one has a tree; Phase 1's gate reworded; one-person plan; rustcast was a seed; the parity gate keeps blocking; nothing reported upstream | Accepted; accessibility route chosen by ADR-0022; decision 1's condition met (#305) |
| [0019](./0019-an-unsafe-bridge-for-the-launchers-surface.md) | One `unsafe` bridge from a toolkit window's raw handles to wayland-client proxies, in `compass-wayland-foreign`, for blur behind the launcher | Accepted |
| [0020](./0020-phase-7-rebrand.md) | The Phase 7 rebrand: `org.tunaos.compass`, the `compass` command and `~/.config/compass`, with a one-time move of the `vicinae` directories; `@vicinae/api`, `vicinae://`, the Vicinae Store and `VICINAE_*` fallbacks stay | Accepted |
| [0021](./0021-remove-the-cpp-engine.md) | The C++ engine leaves the repository; upstream Vicinae releases are the benchmark and differential reference | Accepted |
| [0022](./0022-the-november-calls-decided.md) | The accessibility tree is AccessKit's Unix adapter over our own model; login start is the Background portal; browser control uses a launcher-shipped native messaging host; zbus 5 advisories stay ignored until the pins move; baselines on hosted runners | Accepted |
