# Phase 7 Cutover — what ships and what does not

**Status:** Planned, not yet cut over · **Owner:** #10 · **Relates to:** ADR-0013, ADR-0016, ADR-0020, #118

## What cutover means

* **Linux:** the Rust engine becomes the default. `crates/compass` already defaults to
  `Engine::Rust` (`src/cli.rs:47`); the dispatcher keeps `--engine cpp` as an
  escape hatch for one release (it parses, is reported by `doctor` as a warning,
  and engine-dependent commands refuse rather than silently do the Rust thing).
* **Names:** the rebrand has landed ([ADR-0020](./adr/0020-phase-7-rebrand.md)). The app ID is
  `org.tunaos.compass`, the command `compass`, and an existing `~/.config/vicinae` (with its data,
  cache and state directories) is moved to `compass` on first start.
* **macOS and Windows:** continue to ship the C++ engine. There is no Rust engine
  for those targets until Phases 9/10. This is a **visible narrowing** for non-Linux
  users and belongs in the release notes, not in a bug report.

## What does not ship

* **A screen-reader pass by a person on Bluefin** — the launcher describes
  every view (its search field, lists, action panel, forms, settings and text
  pages) on AT-SPI (`compass-a11y-linux`, ADR-0022 §1), and Orca reads and
  operates its search, results, action panel and forms in a real session on
  every pull request (the Orca gate, `scripts/tier2/orca.sh`, #305). The manual
  pass with a voice on a booted Bluefin
  ([ORCA-CHECKLIST.md](./ORCA-CHECKLIST.md)) has not been done yet. What is left
  is recorded in **ADR-0016** and will be stated in the release notes rather
  than discovered by users.

* **C++ deletion** — `PARITY.md`'s Rust and test columns are fully green
  (`python3 scripts/ci/parity-score.py`), but no C++ directory is deleted before
  cutover regardless of ledger colour — see `PARITY.md`.

## Gate

One full release cycle with no P0 regressions. “P0” includes a screen reader
losing what the Orca gate covers — the search field, the typed query, the
result count, the selection, the action panel and a form's controls — since
#305 that is a regression to file, not a known absence. So is any builtin view
losing the description #304 gave it.

## How to use the escape hatch

```sh
compass --engine cpp toggle   # run the C++ engine once
COMPASS_ENGINE=cpp compass doctor  # doctor reports the mismatch
```

`doctor` surfaces `engine: cpp (Rust is default)` as a warning so a user who
deliberately stayed on C++ is not told their system is broken, and a user who
tripped over the flag knows why behaviour differs.

## Checklist before tagging the cutover release

- [ ] `doctor` warns when `--engine cpp` is in use on Linux
- [ ] Release notes contain both of the “what does not ship” bullets above verbatim
- [ ] `PARITY.md` still marks undeleted C++ rows `⏳` — deletion is Phase 8, Linux-only
- [ ] Flatpak `org.tunaos.compass` boots on Bluefin and the VM tier's launcher job shows the window centred correctly (768×608, box 236..1044, 16px radius)
