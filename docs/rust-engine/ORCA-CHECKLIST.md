# Orca checklist — reading and operating the launcher with a screen reader

**Owner:** #305 · **Relates to:** ADR-0016, ADR-0022 §1, #118, #304

The Orca gate (`scripts/tier2/orca.sh`, run in `tier2.yaml`) checks this on every pull request with
Orca itself, on headless Mutter, reading speech from Orca's debug log. It cannot hear a voice, see
GNOME Shell, or use a physical keyboard. This checklist is the pass a person does on a booted
Bluefin before a release, and after any change to `compass_ui::semantics`,
`compass_ui::accessibility` or `compass-a11y-linux`.

## Setup

1. Bluefin, with Compass installed as users install it (the Flatpak).
2. Settings → Accessibility → Seeing → Screen Reader on (or Super+Alt+S). Orca says
   "Screen reader on".
3. The launcher's shortcut bound (`compass doctor` says how).

## Steps, and what Orca should say

| # | Do | Orca should say (in this order, words may vary by Orca version) | Pass |
|---|---|---|---|
| 1 | Summon the launcher | "Compass", then "Search…, entry". Not the results first. | |
| 2 | Type `snippet` | each letter, then a result count such as "3 results" | |
| 3 | Down, then Up, to Create Snippet | "Results, List with 3 items", then each row's name and description | |
| 4 | Ctrl+B | "Actions, menu", then the current action ("Open command") | |
| 5 | Down / Up in the panel, then Enter on Open command | each action's name; then "Create Snippet", "Name, entry, Snippet name, required" | |
| 6 | Type `orca gate`, then Insert+Tab ("where am I") | each letter; the field and its text | |
| 7 | Tab, Tab, Tab | "Content, entry…", "Keyword, entry…", "Expansion, check box checked" | |
| 8 | Space | "not checked" | |
| 9 | Escape, Escape, Escape | back to the field with its text; the query cleared; the launcher hidden, and Orca back on the previous window | |
| 10 | Summon again, type `xyzzy` | "No results" | |
| 11 | Insert+S (speech off/on) and Orca flat review (KP_7/8/9) on the results | flat review reads the rows; it cannot place them on screen (no bounds, ADR-0016) | |

Known, and not failures (ADR-0016 "Still open"): when the selection moves without a key (clearing
the query after moving through the results) Orca reads the new row; an extension or preference
form's dropdown can be chosen from but not opened from Orca; flat review cannot place nodes on
screen. Also check a few builtin views (clipboard history, emoji, windows, settings): their rows
read and press as the root's do (#304).

Record each pass below: date, Bluefin version (`rpm-ostree status`), `orca --version`,
`flatpak info org.tunaos.compass`, the step numbers that failed and what Orca said instead.

## Observed

### 2026-11-06 — the gate on the Bluefin image (automated, no voice)

The Orca gate's probe, run inside `ghcr.io/ublue-os/bluefin:stable` (Bluefin 44.20261103:
Orca 50.3, at-spi2-core 2.60.7, mutter 50.6) on headless Mutter, against this branch's launcher
inside the shipped Flatpak sandbox with `org.a11y.Bus` granted. Every step passed and the control
(the sandbox without `org.a11y.Bus`) failed. What Orca said, step by step:

| Step | The bus | Orca said |
|---|---|---|
| summon | entry "Search…" focused | "Compass", "Search…", "entry." |
| type `snippet` | text "snippet", 3 results, first selected, the field keeps the focus | the letters, "50 results", "27 results", "3 results" |
| arrow to Create Snippet | list item focused and selected | "Results", "List with 3 items", "Manage Snippets.", "Copy, paste, edit and remove your snippets.", "Create Snippet.", "Save text to paste or expand as you type." |
| Ctrl+B | menu "Actions" of 9 actions, "Open command" focused | "Actions", "menu", "Open command." |
| Open command, pressed through AT-SPI | form "Create Snippet": Name, Content, Keyword, Expansion; Name focused | "Create Snippet", "Name", "entry", "Snippet name", "required." |
| type, Tab ×3, Space | Name holds "orca gate"; the check box toggled | "Content…", "Keyword…", "Expansion", "check box checked.", "not checked" |
| Escape ×3 | no launcher window on the bus | — |

Not yet observed: a person on a booted Bluefin with GNOME Shell and audio. That is the table
above, at the next release.

## Running the gate yourself

The gate needs Orca, at-spi2-core, python3-gobject, mutter and flatpak, as in `tier2.yaml`. In a
Fedora 44 container with the Flatpak installed:

```sh
podman run --rm --privileged -v "$PWD:/src:ro" fedora-with-the-above \
  /src/scripts/tier2/headless-gnome-spike.sh /src/scripts/tier2/orca.sh
```

`COMPASS_BIN=target/debug/compass` runs a local build instead of the Flatpak. The transcript is
written to `/tmp/orca-transcript.md` and Orca's whole log to `/tmp/orca-debug.log`.
