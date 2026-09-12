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
| [0007](./0007-fork-posture-and-platform-scope.md) | Hard fork in practice; `vicinae` names kept; Linux-first | Accepted |
| [0008](./0008-browser-control-is-an-extension.md) | Browser control is an extension, not part of the port | Accepted |
| [0009](./0009-controlled-input-echo-counter.md) | A controlled input's value carries the edit it answers | Accepted |
| [0010](./0010-corral-vm-tier.md) | The VM test tier is corral, on hosted runners | Accepted |
