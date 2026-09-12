# Parity ledger

The definition of done for the Rust engine transformation (#2). Every row must be fully green
before its C++ source is deleted, and **nothing leaves `src/` until it is**.

Columns:

- **C++ ✓** — the behaviour exists in the C++ engine today. Effectively always true; it is here so a
  row with a false in this column stands out as new functionality rather than a port.
- **Rust ✓** — implemented in the Rust engine.
- **parity test ✓** — covered by a Suite 0 differential case (#12) or, where a differential is not
  meaningful, by ported tests from the C++ suite (`PLAN.md` §8.3). A checked box here means a test
  would *fail* if the two engines diverged.
- **C++ deleted ✓** — the C++ source is gone. `⏳` means "green, but the C++ engine still needs this
  code because it is still the shipping engine". **Deletion cannot happen before the Phase 7
  cutover, no matter how green the row is** — see the note below.

A row may go green with a **declared divergence** instead of exact parity: record it in the
Divergences section below with a rationale. Divergences are declared, never discovered.

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
| `compass-xdg` | 110 | desktop entries, locale, exec, reader — scope gaps listed below |
| `compass-search` | 52 | fuzzy, plus an exact port of fzf's coherence rule |
| `compass-ipc` | 57 | framing, transport, single-instance |
| `compass-core` | 71 | app index, frecency, config |
| `compass-shell` | 36 | GNOME Shell DBus client; 22 tests spawn a real `dbus-daemon` |
| `compass-portals` | 55 | XDG portals; availability is a three-state outcome, not a boolean |
| `compass-extension-api` | 74 | view tree, derived identity, diff, dispatch, capabilities, controlled inputs |
| `vicinae` | 137 | CLI, an 11-check `doctor`, and **the engine daemon** |
| `compass-testkit` | 5 | corpora |
| **Total** | **597** | all green under fmt, clippy `-D warnings`, doctests |

## Progress

Scaffolding, corpora and CI are in place, and nine crates have landed: 597 tests across the
workspace, all green, each count verified in a clean `git worktree` checkout of the committed tree
rather than in the working tree.

Almost no row is fully green, and no C++ directory may be deleted yet — see the partial markers and
the divergences below. 🟡 means implemented but not to the full scope of the C++ source. A green
`Rust ✓` with a 🟡 `parity test ✓` means the code exists and is tested as far as this container can
test it; `compass-portals` is the clearest case, since no amount of local testing can tell us
whether a real GNOME session grants the shortcut we ask for.

## Libraries and standalone binaries

| C++ source | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|---|---|:-:|:-:|:-:|:-:|
| `src/lib/xdgpp` | `compass-xdg` | Phase 1 | ✅ | 🟡 | ✅ | ❌ |
| `src/lib/fuzzy` | `compass-search` | Phase 1 | ✅ | ✅ | ✅ | ⏳ |
| `src/lib/crypto` | `compass-core` | Phase 3 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/glyph` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/script-command` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/vicinae-ipc` | `compass-ipc` | Phase 2 | ✅ | ✅ | 🟡 | ⏳ |
| `src/lib/figura` | `compass-ipc` | Phase 2 | ✅ | n/a | n/a | ⏳ |
| `src/lib/common` | `compass-core` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/linux-utils` | `compass-platform` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/soulver` | `—` | n/a (macOS) | ✅ | ❌ | ❌ | ❌ |
| `src/cli` | `crates/vicinae` | Phase 2 | ✅ | 🟡 | 🟡 | ❌ |
| `src/file-indexer` | `compass-platform` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/data-control-server` | `compass-wayland` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/snippet` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/browser-extension` | — | **out of scope** | ✅ | n/a | n/a | never |

## Services

| C++ source | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|---|---|:-:|:-:|:-:|:-:|
| `src/services/app-runtime` | `compass-core` | Phase 1 | ✅ | ❌ | ❌ | ❌ |
| `src/services/app-service` | `compass-core` | Phase 1 | ✅ | 🟡 | ✅ | ⏳ |
| `src/services/asset-resolver` | `compass-core` | Phase 1 | ✅ | ❌ | ❌ | ❌ |
| `src/services/audio-control` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/autostart` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/browser-extension` | — | **out of scope** | ✅ | n/a | n/a | never |
| `src/services/builtin-icon` | `compass-core` | Phase 1 | ✅ | ❌ | ❌ | ❌ |
| `src/services/calculator-service` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/clipboard` | `compass-core` | Phase 3 | ✅ | ❌ | ❌ | ❌ |
| `src/services/desktop-notification` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/extension-boilerplate-generator` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/extension-registry` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/extension-store` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/file-chooser` | `compass-core` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
| `src/services/files-service` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/font-service` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/global-shortcuts` | `compass-portals` | Phase 1 | ✅ | 🟡 | 🟡 | ❌ |
| `src/services/glyph-service` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/image-fetcher` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/input-server` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/keybinding` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/local-storage` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/media-control` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/menu-bar` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/navigation` | `compass-core` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
| `src/services/news` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/oauth` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/paste` | `compass-core` | Phase 3 | ✅ | ❌ | ❌ | ❌ |
| `src/services/permissions` | `compass-core` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
| `src/services/power-manager` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/raycast` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/root-item-manager` | `compass-core` | Phase 2 | ✅ | 🟡 | 🟡 | ⏳ |
| `src/services/script-command` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/selection` | `compass-core` | Phase 3 | ✅ | ❌ | ❌ | ❌ |
| `src/services/shortcut` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/shortcut-inhibit` | `compass-core` | Phase 3 | ✅ | ❌ | ❌ | ❌ |
| `src/services/snippet` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/telemetry` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/toast` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/tray` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/tray-host` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/update` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/url-scheme` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/wallpaper` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/window-manager` | `compass-core` | Phase 3 | ✅ | ❌ | ❌ | ❌ |
| `src/services/window-material` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |

## Builtins

| C++ source | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|---|---|:-:|:-:|:-:|:-:|
| `src/builtins/browser` | — | **out of scope** | ✅ | n/a | n/a | never |
| `src/builtins/calculator` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/clipboard` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/developer` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/file` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/font` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/internal` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/media` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/power-management` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/raycast` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/root` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/shortcut` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/snippet` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/system` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/theme` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/vicinae` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/wm` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |

## Not-yet-ported scope

Tracked here so a 🟡 does not quietly become a ✅.

**`src/lib/xdgpp` → `compass-xdg`** — the desktop-entry, locale, value, reader and exec layers are
ported (47 C++ cases, verbatim inputs). Still C++-only:

- the `xdg-terminal-exec` draft extension (`X-TerminalArg*` typed accessors; the keys are readable
  through `Reader` today);
- the `DesktopFile` layer — `fromId`, `relativeId`, directory search. `from_file` and
  `ParseOptions::{id,path}` exist, but id computation and lookup are a separate pass;
- the sibling modules `bookmark`, `env`, `file-uri`, `file`, `mime`, `special`.

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

### `compass-search` — nucleo is not fzf

`nucleo-matcher` uses a different algorithm from the C++ fzf port, so absolute scores are on another
scale and are never asserted. The normalized 0-100 `score`/`quality` values match the C++
expectations closely: every `score == 100`, `score == 50` and `quality == 100` assertion ports
verbatim and passes. What does not:

| # | Divergence | Impact | Pinned by |
|---|---|---|---|
| 1 | ~~No coherence signal~~ — **resolved.** The original claim here was wrong: the C++ backtracker only ever compares `B[j]` against *zero*, never uses its magnitude, and uses `H`/`C` solely to choose the alignment. So `coherent = !boundary_inside \|\| !mid_word_run_start` is a pure function of (haystack, indices), which is exactly what nucleo returns. It is an **exact port**, not a heuristic — no thresholds exist to tune. | None | the ported coherence suite |
| 2 | **One ordering flip** (upstream issue #946). For `"Spo"`, C++ gives `Spotify > Reload Script Directories > Sysprog`; we give `Spotify > Sysprog > Reload Script Directories`. nucleo prefers a short scatter inside one word starting at position 0; fzf's larger word-boundary bonuses pull the other way. Every other ordering case ports and passes. | Low | `diverges_spo_ordering` |
| 3 | **Narrower diacritic folding.** nucleo folds precomposed accents (é, ñ, ü) but not Latin Extended-A stroked/ogonek letters (Ł, ź, đ, ż), so `"lodz"` does not match `"Łódź Express"`. fzf's table covers them. | Affects Polish, Czech and Croatian app names | `diverges_latin_extended_a_is_not_folded` |
| 4 | **Ties that discriminate nothing.** For `"clip"`, all of `Clipboard History`, `Clear Current Clipboard Data` and `Clear Clipboard History` score identically, because nucleo's score depends only on the matched region, not on haystack length or match position. The C++ ordering test passes there only because `stable_sort` preserves input order — so that case discriminates nothing in *either* implementation. | Real discrimination needs a length or match-position penalty layered on top of nucleo | `diverges_clip_ordering_is_a_three_way_tie` |
| 5 | **Char, not byte, offsets** — a deliberate API change. `"Café Bar"`/`"bar"` reports `5..8` where C++ asserts bytes `6..9`. | None; byte offsets are recoverable | the range tests |

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
