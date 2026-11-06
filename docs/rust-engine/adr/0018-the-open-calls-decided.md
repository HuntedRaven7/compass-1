# ADR-0018: The open calls, decided — screen readers keep a working launcher, and four smaller ones

**Status:** Accepted · **Date:** 2026-09-24 · Amends: ADR-0016, the Phase 1 exit gate (#4) · Closes: #14's remaining items

## Context

Five questions had been left open for the project owner: the accessibility gap (#118), the wording
of Phase 1's GNOME gate (#4), team size and the rustcast relationship (#14), whether the C++ top-result
comparison should keep blocking merges, and whether to report the six desktop-entry bugs upstream.
The owner delegated all five: "do what you think is best, don't block on me". This records what was
chosen and why, so each can be revisited on its merits rather than rediscovered.

## Decisions

### 1. Screen-reader users are never moved onto a launcher they cannot read (amends ADR-0016)

ADR-0016 accepted shipping the Phase 7 cutover with the gap noted in release notes. That would take
a working launcher away from blind users: the C++ engine exposes its query field, results and action
panel to Orca through Qt's `Accessible` properties, across 15 QML files. A release note does not give
that back.

- **Phase 7 may still make the Rust engine the default**, but the packaged entry point selects the
  C++ engine when `org.a11y.Status.ScreenReaderEnabled` is true, until the Rust launcher has an
  accessibility tree. That is a packaging change inside #10, not a blocker on it.
- **Phase 8 — removing the Linux C++ engine — is gated on the tree.** It is the step that would
  otherwise strand those users, so it is the step that waits.
- **`doctor` warns now** (`a11y.screen-reader`): a screen reader on and this launcher running is a
  warning that names #118, not silence. ADR-0016 asked for this and it had not been built.
- **The route to a tree** is evaluated in this order, cheapest credible first: upstream Iced
  (iced-rs/iced#552, still open); the `plushie-iced` fork, which claims AccessKit across all built-in
  widgets and would be a source to adopt or upstream from rather than a rewrite; then our own AT-SPI
  behind `compass-ui`. The COSMIC fork's support is partial (pop-os/libcosmic#1429: its text input
  emits no node), so it is a reference, not a base.

**Where decision 1 stands (2026-11-06).** Its condition is met. The tree landed through ADR-0022's
route (AccessKit's Unix adapter over `compass_ui::semantics`), and Orca reads and operates the
launcher's search, results, action panel and forms in a real session, gated on every pull request
by the Orca gate (`scripts/tier2/orca.sh`, #305; ADR-0016's "Where it stands"). The packaged
screen-reader fallback never shipped: ADR-0021 removed the C++ engine first, so there is no
fallback to retire and no entry point routes on `ScreenReaderEnabled`. With #304 every view is
described, so `doctor` reports a running screen reader as fine rather than warning.

### 2. Phase 1's GNOME gate, reworded

"Runs from a Flatpak on Bluefin under GNOME 50 and 51" cannot be met: no Bluefin image is built on
Fedora 45, so none ships GNOME 51. The gate protected two things, and both are covered:

> Runs from a Flatpak under GNOME **50 and 51** (Suite 3b), and on **Bluefin** at whatever GNOME
> version Bluefin currently ships (the VM tier).

When Bluefin moves to Fedora 45 the VM tier's `stable` tag follows it with no edit. The screen-reader
item from Phase 1's "watch for" clause moves to #10 and #118 under decision 1: it was found early, as
the clause asked, and it is now owned where it bites. One criterion is moved rather than met: **idle RSS < 30 MB** holds for the engine (6.2 MB idle,
gated at 20 MB), but since ADR-0015 the window process stays resident too, and its idle RSS has only
been measured under a software renderer (~135 MB, mostly llvmpipe). The hardware figure is a
performance measurement, so it moves to #127 with the other §8.5 numbers instead of holding the
slice open. With those three, Phase 1 (#4) is complete.

### 3. Team size: planned against one person

The schedule in `PLAN.md` is costed at 1 FTE, serially, about seven months. That stays the plan.
More people shorten it, but a plan that assumes help nobody has committed is the plan that slips.

### 4. rustcast was a one-time seed

No ongoing sync. ADR-0017 already made Compass its own launcher, the crate split has proceeded on
this assumption for twenty-odd crates, and syncing would constrain crate boundaries for code we no
longer share. Improvements worth sending back are sent as ordinary upstream contributions.

### 5. The C++ top-result comparison keeps blocking

It was proposed to make `--gate-top-result` report instead of fail, on the grounds that ADR-0017 made
parity a tripwire rather than the spec. The answer is no: a tripwire that does not fail is not a
tripwire. It is green today, it has a declared-divergence path for every intended difference, and
the cost of keeping it is one declaration per deliberate change. What ADR-0017 changes is how a
failure is resolved — declare the divergence when Compass is right — not whether it is seen.

### 6. Nothing is reported upstream

ADR-0007 suggested reporting the six desktop-entry bugs to `vicinaehq/vicinae` as a courtesy. The
owner's answer: this is a hard fork, and upstream is not to be bothered. The bugs remain recorded as
declared divergences in `PARITY.md`, which is the record this project needs; the drafted report is
deleted.

## Consequences

- #4 and #14 close. #127 gains the resident window's idle RSS on hardware. #118 stays open as the tracker for the tree itself. #10 gains two items: the
  screen-reader fallback in the packaged entry point, and no Linux C++ removal before the tree.
- Revisit decision 1 when any route to a tree lands; revisit 3 if someone commits time.
