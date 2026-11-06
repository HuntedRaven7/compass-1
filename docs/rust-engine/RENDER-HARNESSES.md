# Seeing the launcher: three harnesses, and what each one proves

Written for whoever picks this up next, human or agent. The launcher is a
graphical program and this repository is usually worked on from a container
with no compositor, so "look at it" is a question with three different answers
and they are not interchangeable.

| Harness | Renders with | Round trip | Proves |
|---|---|---|---|
| Browser surrogate (`tools/design/`) | HTML + CSS | ~2 s | layout, colour, hierarchy, state |
| **Paint tier** (`crates/compass-ui/tests/paint.rs`) | the real Iced renderer — **wgpu** via lavapipe, and **tiny-skia** | **~3 s** | that Iced paints what the tree describes |
| VM tier (`scripts/vmtest/launcher.sh`) | the real thing on real GNOME | ~30 min | that it works on the target |

The middle row was "wgpu in a canvas (**not built**)" until the paint tier
replaced it. It turned out not to need a canvas or WebGL at all: `iced_test`
already renders headlessly through Iced's own renderer, and the missing piece
was reading the pixels back and asserting something robust about them.

Use the cheapest one that can answer your question, and **do not let a cheap
one answer an expensive one's question**. That is the whole discipline here.

## The ladder: one command per rung, cheapest first

```sh
make test-fast    # t0 + t1 — the per-edit loop
make test-t0      # logic: unit + integration, no display          seconds
make test-t1      # paint: the launcher rendered, pixels checked    ~3 s/backend
make test-t2      # session: real Mutter + portal in a container    ~92 s
make test-t3      # target: real GNOME in a VM (runs in CI)         ~30 min
```

Each rung proves something the one below it cannot, and costs roughly ten to
a hundred times more. **Climb only as far as the question needs**, and when a
rung fails, pull the failure *down* to the cheapest rung that reproduces it
before debugging it there.

| Question | Cheapest rung that answers it |
|---|---|
| Does the ranking / config / parser do the right thing? | **t0** |
| Is the widget tree right — rows present, in order, selected? | **t0** (`iced_test` structure, the `insta` snapshots) |
| Does the tree **paint** — selection filled, text visible, theme applied? | **t1** |
| Does it paint the same on the GPU pipeline as on the CPU? | **t1** (both backends, each forced) |
| Does a window actually appear under a real compositor? | **t2** |
| Does the portal bind the shortcut; does Mutter place the window? | **t2** / **t3** |
| Does it work on the real target, as a user installs it? | **t3** |

The two the ladder was missing are the middle ones. Before the paint tier, a
change that broke the selection fill passed every per-PR check and was caught,
if at all, by the nightly VM run half an hour later.

### A VM-tier assertion that did NOT port, and why

The VM tier's central check is geometric: summon the launcher, diff the
screenshots, and require a `card_width`-wide card centred in the window
(`scripts/vmtest/launcher.sh`, `--expect-box`). The obvious move was to check
the same thing on the paint tier in three seconds. **It does not port by pixels,
and an attempt was removed rather than shipped.**

The frame's background *is* the card's surface colour, in both appearances. The
card's edge is marked only by a one-pixel border hairline and by a drop shadow
cast downward — so the rows above the card are pure background, indistinguishable
from card. Every pixel heuristic for "where is the card" either found the whole
frame, or — worse — found a plausible-looking internal region: with the shadow
padding deliberately removed, one version **still passed**. A check whose control
fails silently is the exact false green these suites exist to prevent.

The measured geometry itself is fully accounted for, which is worth recording
because it briefly looked otherwise. `card_width` is 720 and the window is 768
with 24 px of shadow padding, so the card spans logical 24→744. The painted
surface interior measures **704.5** — which is `card_width − 2 × card_padding`,
i.e. 720 − 2 × 8 = **704**, to half a pixel, on both backends and in both
appearances. The first draft of this note called that a 15 px "unexplained
discrepancy"; it was `card_padding`, and reading `design.rs` before writing it
down is what caught it. The difficulty is purely in *locating* the card from
pixels, not in what its size should be.

The right way to move this check down the ladder is **layout bounds from the widget
tree**, which are exact, confirmed by a paint check at the location layout reports —
not a pixel search for an edge that the background colour hides.

## The paint tier — per PR, both backends

```sh
cargo test -p compass-ui --test paint                       # whichever backend Iced picks
ICED_TEST_BACKEND=wgpu PAINT_EXPECT_BACKEND=wgpu \
  cargo test -p compass-ui --test paint                     # force the GPU pipeline
```

`LauncherApp::view()` rendered through Iced's real renderer, pixels read back,
and **colour in regions** asserted against the app's own palette, tied to the
layout bounds the widget tree reports. It catches the one thing the tiers either
side of it could not on a pull request: **a tree that lays out correctly and
paints wrong.**

Proven by mutation, on both backends:

| broken | paint tier | 149 lib + 4 insta structural tests |
|---|---|---|
| selected row filled with the surface colour | **fails** | all green |
| unselected title drawn in the surface colour | **fails** | all green |

It asserts invariants, not images. A committed PNG compared byte-for-byte
reddens whenever a runner changes its font package; region colour does not
care where glyphs land, and a selection that stops painting cannot hide from
it. Thresholds were measured before they were set: selection share 0.777 in
the selected row against 0.000 in an unselected one.

**Which backend is not left to chance.** Iced tries wgpu and falls back to
tiny-skia silently — right for an application, wrong for a test. CI forces each
with `ICED_TEST_BACKEND` and confirms it with `PAINT_EXPECT_BACKEND`, so a wgpu
job cannot pass on tiny-skia. Locally, `apt install mesa-vulkan-drivers` gives
wgpu an adapter; without it, forcing wgpu panics rather than substituting.

What it still does **not** prove: that GNOME composites the window, that the
portal binds the shortcut, that the real font stack on the target matches.
Those are the VM tier's.

## 1. The browser surrogate — built, use it first

```sh
make design         # serve at http://127.0.0.1:8173
make design-shots   # screenshot every state, both appearances
```

`tools/design/README.md` has the details. The short version: the states come
from the real view models (`root_list::build`, `action_panel::flatten`) and the
colours and sizes come from `crates/compass-ui/src/design.rs`, both delivered
as `states.json`. The page retypes neither, so it cannot quietly disagree with
the Rust.

It does **not** prove Iced paints the same picture. It is HTML.

### Regular design audits

Run a browser audit after each user-visible UI change and before merging UI
work. During longer backend-only stretches, audit again at each integrated
launcher milestone. Generate fresh states, inspect the rendered images (not
just the screenshot command's exit status), and record the source commit,
states inspected, findings and remaining coverage gaps in the PR or an audit
note. Review light and dark appearances, the default preset's affected states,
and all four presets whenever geometry or shared tokens change.

Include screenshots of the affected feature directly in every UI PR body.
Use durable image URLs (for example committed PR evidence assets), not local
paths or expiring artifact links. Caption each image with its renderer and
scope: browser surrogate, headless Iced/wgpu, or native desktop session. Include
before/after or relevant interaction states when they explain the change. A
nonvisual PR should explicitly say that screenshots are not applicable.

Check hierarchy, spacing, selection contrast, truncation, action-panel layout,
and empty/loading/error states. If the surrogate cannot show the changed
production state, record that as missing coverage; a nearby fixture is not a
pass. Keep fixture-only actions distinct from working application actions.
Native widget behaviour and OS integration still need the other tiers above.

### Component review before shipping

For each changed component, write down its geometry invariants before changing
the layout: row height, vertical centering, shared text inset, divider thickness,
and focused-input treatment. Keep component dimensions in `design.rs`, consumed
by both Iced and the browser export rather than duplicated CSS literals.

Run a focused `iced_test::Simulator` layout test against the production widget
tree. Assert bounds, not merely that text exists. For action menus this includes
centered labels and shortcuts, aligned section labels, and a one-logical-pixel
painted divider with transparent surrounding padding. Capture focused light/dark
states for every preset with `COMPASS_UI_SCREENSHOT_DIR`; inspect the headless
Iced/wgpu images as well as the browser preview. These native-widget fixtures
exist today even though the wasm renderer described below does not.

Review short, filtered, empty and overflowing states. Preserve keyboard focus,
scrolling and Escape behavior while removing unwanted visual defaults. Include
the affected component captures in the PR body and state the renderer and
coverage limits. A screenshot of a nearby fixture does not approve an untested
production state. The installed desktop remains the check for compositor,
scaling, application discovery and keyboard delivery.

The [2026-09-20 audit](./design-audits/2026-09-20.md) records the starting
coverage and the current theme/default distinction.

## 2. The wgpu path — not built, and here is exactly what it takes

This is the missing middle: the *real* widget code and the *real* renderer, in
a canvas Playwright can drive, without booting a VM.

**It is supported by the iced we already depend on.** Verified against the
vendored source rather than the changelog:

- `iced_winit 0.14` declares `wasm32` dependencies — `web-sys` with
  `HtmlCanvasElement`, and `wasm-bindgen-futures`
  (`iced_winit-0.14.0/Cargo.toml`, the `cfg(target_arch = "wasm32")` block).
- `iced 0.14` has a `webgl` feature that routes to `iced_wgpu/webgl`
  (`iced-0.14.0/Cargo.toml`), which is how wgpu reaches a browser.

So the renderer is the same wgpu pipeline, on WebGL instead of Vulkan. That is
a far smaller gap than HTML, and much smaller than the gap `iced_web` would
have.

**What blocks it, concretely.** `compass-ui` depends on `compass-core`,
`compass-xdg` and `compass-platform`, and through them on filesystem scanning,
D-Bus and process spawning — none of which builds for `wasm32`. The work is:

1. A `wasm32` entry point beside `run()` that constructs `LauncherApp` from a
   **fixed in-memory index** rather than `AppIndex::from_environment()`. The
   view models are pure, so this is a corpus and a constructor, not a port.
2. A `NullLauncher`-style no-op for the launch action — `compass-platform`
   already has the seam (ADR-0013).
3. `iced` with `features = ["webgl"]`, built with `wasm-pack` or `trunk`, into
   `tools/design/wasm/`.
4. Cargo config so `wasm32-unknown-unknown` does not try to build the crates
   that cannot: a separate thin crate depending only on `compass-ui` and the
   view models is likely cleaner than feature-gating four crates.

**Do not resurrect `iced_web`.** It is archived, targets the Iced 0.1/0.2 API,
and renders to the **DOM** through dodrio rather than through wgpu. Reviving it
would be a multi-version port *and* would paint through a renderer we do not
ship — a fidelity harness that is not faithful is worse than no harness,
because it looks authoritative.

**What it would still not prove:** that it works on GNOME under Flatpak with a
real compositor and a real icon theme. That stays the VM tier's job. WebGL is
not Vulkan and llvmpipe is neither.

## 3. The VM tier — the authority

`scripts/vmtest/launcher.sh`, driven by corral, on a real Bluefin VM. See
PLAN.md §8.9. It is the only thing that can fail for a reason the other two
cannot see, and it found #91 and #95 exactly that way.

Every call it makes has a budget (#333): ssh connects within
`VMTEST_SSH_CONNECT_S` and notices a vanished guest through keepalives; each
guest command, corral call, state probe and the failure-time journal stop at
their own `VMTEST_*_BUDGET_S`; and hide, summon and the action panel are
waited for -- the launcher's own state line first, then a frame that framediff
says shows the result -- rather than slept through. The baseline frames are
waited for too (#345): nothing is looked at until GNOME Shell has logged that
it finished starting (`checks.sh shell-started`, within
`VMTEST_SESSION_BUDGET_S`), because corral's boot marker can arrive before the
wallpaper and the startup overview do, and the login and bare-desktop frames
are each kept only once `VMTEST_STABLE_SHOTS` screenshots in a row agree.
A failed gate keeps its own exit status even when the journal cannot be
collected, which is reported on a `SECONDARY:` line of its own.
`scripts/vmtest/launcher-selftest.py` runs the real script against a stub
guest (`fakevm.py`) and proves each of those paths, hangs and a late shell
included, without a VM; `scripts/tier2/spike-selftest.py` does the same for the
headless spike's readiness probes.

## Playwright mechanics that will bite you

Every one of these cost a run to discover.

**The pre-installed Chromium is pinned and will not match your Playwright.**
The image ships browsers under `/opt/pw-browsers` (`chromium-1194` at the time
of writing) and `PLAYWRIGHT_BROWSERS_PATH` points there. A freshly installed
`playwright` package expects a different build number and dies with
"Executable doesn't exist". Do **not** run `npx playwright install` — point at
what is there:

```sh
CHROMIUM_PATH=/opt/pw-browsers/chromium-1194/chrome-linux/chrome \
  node tools/design/shoot.mjs
```

`shoot.mjs` reads `CHROMIUM_PATH` and falls back to Playwright's own browser
when it is unset, which is right on a developer machine.

**`fetch` does not work on `file://`.** Chromium refuses it, so a page opened
as a file loads and then sits empty forever with no error you will notice.
`shoot.mjs` runs a ten-line static server on a random port for this reason.
Inlining the JSON into the HTML would also work and is worse: it puts a second
copy of the tokens somewhere it can go stale.

**Handle the stream error, not just the pipe.** A missing file — a browser
asking for `/favicon.ico`, say — emits `error` on the `ReadStream` before any
pipe, and an unhandled `error` takes the whole process down. Attach the handler
to the stream.

**Disable animations before screenshotting.** The search caret blinks; without
`emulateMedia({ reducedMotion: "reduce" })` and an `animation: none` style tag,
half your screenshots differ from the other half for no reason anyone can read.

**Reading frames the VM tier produced.** Its artifacts are downloadable and
worth looking at rather than trusting a green tick:

```sh
# the job log, via its signed URL — grep locally rather than pulling
# thousands of lines of GL extension spam through a tool result
curl -sS -o job.log "<logs_url from get_job_logs with return_content=false>"

# the frames
curl -sS -L -o launcher.zip \
  "https://api.github.com/repos/tuna-os/compass/actions/artifacts/<id>/zip"
unzip -q launcher.zip -d art
```

**The system Python's Pillow is broken in this image** (`cannot import name
'_imaging'`). Make a venv:

```sh
python3 -m venv /tmp/venv && /tmp/venv/bin/pip install -q pillow
```

Then crop the changed region out of a before/after pair and actually look at
it. A framediff tells you *that* something changed; only the image tells you
*what*, and the difference between those two mattered: the Ctrl+B assertion
reported 2,697 changed pixels in a 107×133 box, and it took opening the frames
to know that the box contained the action panel rather than something else.

## The wlroots rung — headless Sway, per PR

```sh
cargo build -p compass && scripts/wlroots/launcher-on-sway.sh   # ~15 s
```

The t2 of the second compositor family. Sway runs with `WLR_BACKENDS=headless`
and the pixman renderer — no container, no D-Bus, no GPU — so it is cheaper
than the Mutter tier and runs on the CI runner itself
(`.github/workflows/wlroots.yaml`). It answers what only a wlroots compositor
can: that the launcher is a **layer surface** (on screen, yet absent from
Sway's window tree), that `compass toggle` hides and re-shows it, and that
typed text reaches it (`wtype`). Screenshots come from `grim`; the screen
starts as a solid `#202020` background, so "on screen" is a pixel count.

Its control is the same launcher forced onto `xdg_toplevel`
(`COMPASS_LAYER_SHELL=0`), which must show up in the tree. The typing gate
has one too, found the hard way: its first version passed while the keys went
nowhere, because the root list filling in changed the screen on its own. It
now waits for a settled frame first, and it fails against the build that
reported `Opened` before the layer surface existed.

It proves nothing about GNOME, and nothing about Hyprland or niri beyond what
their shared protocols imply.
