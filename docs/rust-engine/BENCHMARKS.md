# Benchmarks: Compass against Vicinae

This page backs every number in the README. Each figure says which kind it is:

- **head-to-head**: Compass and upstream Vicinae measured by the same script, on the same
  machine, in the same session, with alternating run order.
- **Compass only**: no upstream figure measured the same way exists yet.

Nothing here is estimated. A figure we could not measure is listed under
[Still to measure](#still-to-measure) and left out of the tables.

## Reproduce

```sh
just bench-compare                 # or: scripts/bench/compare.sh [OUTPUT_DIR] [RUNS]
```

It needs the pinned Rust toolchain, a C++23 compiler, `python3`, `sway`, `grim`, `wtype`,
`dbus-daemon` and binutils. The upstream AppImage bundles Qt but loads GL dispatch, font and X client
libraries from the host. On Ubuntu these are `libopengl0`, `libglx0`, `libegl1`, `libfontconfig1`,
`libfreetype6` and `libx11-xcb1`, and the script names any that are missing before it builds
anything. No GPU and no running desktop are needed. The script:

1. Downloads the upstream **Vicinae v0.29.0** x86_64 AppImage named in
   [`scripts/bench/baseline.json`](../../scripts/bench/baseline.json) and refuses it unless its
   SHA-256 matches (`4d32f758…fe86bc`). It extracts the AppImage and does not mount it, so FUSE is
   not needed. The binary is unmodified.
2. Builds `compass`, `compass-file-indexer`, `compass-sandbox-exec`, `fuzzy-throughput` and
   `head-to-head` in release mode, and builds upstream's header-only fuzzy scorer into
   [`scripts/bench/fuzzy/cpp_rank.cpp`](../../scripts/bench/fuzzy/cpp_rank.cpp) with `-O2`.
3. Runs [`scripts/bench/compare.py`](../../scripts/bench/compare.py). It starts a private
   headless Sway (pixman renderer, 1280×800), a private D-Bus bus with no activatable services,
   and throwaway `HOME`/`XDG_*` directories. Each engine runs under `unshare --net` where the
   machine permits it, because upstream fetches currency rates at startup. Where it does not, as on
   a hosted CI runner, both engines have the host's network and the report says so. It indexes the
   738-entry real-host desktop corpus (`crates/compass-testkit/corpus/desktop-entries/real`).
   Nothing touches the caller's session, bus, home or input devices.
4. Writes `report.json` (every run, every sample, both sides' versions and SHA-256s, and the machine
   details), `points.jsonl` for the upstream gate, `versions.txt`, the warm-ping reports under
   `ping/`, and per-run logs and screenshots.

Each run cold-starts one engine with its window open: `compass start` for Compass and
`AppRun server --open --no-extension-runtime` for upstream, with onboarding marked complete. Neither
engine runs TypeScript extensions. The measurements are:

| Metric | Definition |
|---|---|
| Engine ready | exec until the first successful IPC ping (upstream: JSON-RPC `Ipc/ping`; Compass: `compass ping`, which pays a CLI spawn on every poll, so the figure is conservative) |
| First frame | exec until the first screenshot that differs from the empty output |
| Launcher populated | exec until the last change to the picture before it idles. Rows that change on their own (the footer clock, the caret) are learned per run and excluded |
| Keystroke to frame | `wtype fire` until the first changed screenshot outside those rows |
| Idle / after-search memory | the summed `smaps_rollup` RSS and PSS of every process carrying the run's environment tag, so detached helpers count. Taken 5 s after the first frame, and again 1 s after the typed frame settles |
| Threads, processes, shared objects | from `/proc` at idle, over the same process set |
| Warm IPC ping | after the cold starts, both engines are started once and left idle, and `head-to-head` (`compass-testkit`) pings both over persistent sockets: 1,000 samples after 100 warm-ups, alternating AB/BA, once per run. Its median excludes connection setup and process spawn |

One screenshot takes a median of 16 ms, so that is the time resolution of the frame metrics.

`perf.yaml` runs this on a hosted runner for every pull request, every push to `main` and every six
hours. Its `upstream-gate` job holds five of these actions to no worse than upstream: the warm ping,
keystroke to frame, first frame, launcher populated and idle RSS. The tolerance comes from the
measured spread of earlier runs. [PERF-GATES.md](./PERF-GATES.md#against-upstream-vicinae) explains
the rule and lists what is excluded and why.

## Results: 2026-09-25

Machine: 4 vCPU Intel Xeon @ 2.10 GHz, 16 GB, Ubuntu 24.04.4, kernel 6.18.44. Both engines draw
through Mesa's llvmpipe; `libgallium` is mapped in both. There is no GPU. The machine was shared
with other builds, with a load average of 6–8 throughout. Both engines ran under the same load,
alternating, but absolute times on an idle desktop with a GPU will be lower. Compass is commit
`a037d82` plus this change, built with rustc 1.94.1. Raw data:
[`benchmarks/2026-09-25-compare/report.json`](./benchmarks/2026-09-25-compare/report.json).

### Cold start, latency and memory (head-to-head, median of 5 runs each)

| | Compass | Vicinae 0.29.0 | |
|---|---:|---:|---|
| Engine ready (IPC answers) | **96 ms** | 1,746 ms | 18× sooner |
| First frame on screen | **873 ms** | 1,716 ms | 2.0× sooner |
| Launcher populated | **908 ms** | 2,330 ms | 2.6× sooner |
| Keystroke to updated frame | **56 ms** | 153 ms | 2.7× sooner |
| Idle memory, PSS | **218 MiB** | 263 MiB | 17% less |
| Idle memory, RSS | **249 MiB** | 283 MiB | 12% less |
| Memory after a search, PSS | **218 MiB** | 265 MiB | 18% less |
| Shared objects mapped | **46** | 136 | |
| Processes | 3 | 3 | engine, window, file indexer / server, file indexer, input server |
| Threads | 48 | 39 | **Compass uses more** (Tokio and rayon pools) |

Per-run values (ms: ready / first frame / populated / keystroke):

| Run | Compass | Vicinae |
|---|---|---|
| 1 | 96 / 721 / 770 / 52 | 1,904 / 1,716 / 2,560 / 153 |
| 2 | 84 / 875 / 908 / 64 | 1,847 / 1,881 / 3,810 / 137 |
| 3 | 97 / 873 / 920 / 53 | 1,746 / 1,710 / 2,330 / 154 |
| 4 | 104 / 702 / 702 / 56 | 1,710 / 1,743 / 2,318 / 144 |
| 5 | 92 / 1,028 / 1,068 / 76 | 1,582 / 1,451 / 2,225 / 196 |

Screenshots of the typed frame from run 1:
[Compass](./benchmarks/2026-09-25-compare/compass-1-typed.png) and
[Vicinae](./benchmarks/2026-09-25-compare/upstream-1-typed.png). Both show the same three
applications for `fire`.

Memory is dominated by the software renderer on both sides. The earlier Xvfb comparison
([2026-09-20](./benchmarks/2026-09-20-upstream-v0.29.0/README.md)) measured 117 MiB against
215 MiB PSS with a different renderer path. Treat the ratio as the result, not the absolute
figure.

### Fuzzy ranking, top 20 of 10,000 items (head-to-head)

This uses the same 10,000 names that `benches/slas.rs` ranks, and the same queries, with 500 timed
iterations after 10 warm-ups. The time covers scoring the haystack, sorting the matches and taking
the top 20. Medians, in µs:

| Query | Vicinae scorer (C++, 1 core) | Compass scorer (1 core) | Compass as shipped (rayon, 4 cores) |
|---|---:|---:|---:|
| `ed` | 884 | 1,388 | **724** |
| `fire` | **440** | 1,010 | 530 |
| `sysmon` | **563** | 1,304 | 608 |
| `calc pro` | 590 | 1,171 | **555** |
| `q` (no match) | **270** | 594 | 310 |
| `zzzz` (no match) | **285** | 849 | 621 |

**This one is not a win.** Per core, the Compass scorer is 2–3× slower than upstream's. What
Compass ships spreads the scoring across cores with rayon, which is roughly at parity on this
4-core machine: ahead on two queries and behind on four. Both are well inside the 2 ms SLA. For
`ed`, Compass accepts 1,450 matches where upstream accepts 500, so it does more sorting work. This
is a real target for optimisation, and the README does not claim it.

### Size and dependencies (head-to-head, static)

| | Compass | Vicinae 0.29.0 AppImage |
|---|---:|---:|
| Download | not yet published as a single artifact | 101.5 MB (AppImage) |
| Program files, uncompressed | 72.4 MB: `compass` 62.3 MB + file indexer 9.7 MB + sandbox launcher 0.5 MB, stripped | 310.1 MB extracted |
| Libraries linked (`DT_NEEDED`, main binary) | 6 (libc, libm, libgcc_s, libssl, libcrypto, ld.so) | 32 for `vicinae-server`, 16 of them Qt/KF6 |
| Libraries bundled | 0 | 122 |
| Qt | none | Qt 6 (Quick, QML, Widgets, Wayland, DBus, Network, Svg…) |

The AppImage bundles its libraries. A distribution package of Vicinae would share the system's Qt,
so the 310 MB overstates what Vicinae costs on a machine that already has Qt 6.

### Compass-only: the SLA benches

These come from `cargo bench -p compass-testkit --bench slas` on the same machine, reported by
`scripts/ci/criterion-report.py`, the same pair the "Suite 4: SLA benches" CI job runs:

| Row | Median | SLA |
|---|---:|---:|
| Fuzzy search, top 20 of 10,000 items | 1,450 µs | < 2,000 µs |
| IPC round trip over the local socket | 8.5 µs | < 500 µs |

The earlier head-to-head ping measurement, over persistent sockets, is in
[HEAD-TO-HEAD.md](./HEAD-TO-HEAD.md). Compass answered in 8.6–17.4 µs and upstream in 20.7–37.4 µs
(median).

## Still to measure

- **Flatpak size**, for both. `flatpak` is not available in the measurement container, and
  upstream publishes no Flatpak. The CI bundle (`flatpak-bundle` artifact) is where Compass's figure
  will come from.
- **The same run on a GPU and on GNOME** (Mutter, portals, the Shell extension). Everything above
  used llvmpipe on wlroots. The VM tier boots GNOME but does not time upstream.
- **Summon latency** (resident, hidden to shown) for both. Only cold start is timed here.
- **Peak memory with extensions running**, for both. Neither engine ran TypeScript extensions.
- **Root search over IPC on the same index**. Unmodified upstream has no `rootQuery` endpoint;
  [HEAD-TO-HEAD.md](./HEAD-TO-HEAD.md) has the instrumented-build route.
- **An idle machine.** Every figure above was taken under a load average of 6–8 from unrelated
  builds. Rerun `just bench-compare` on a quiet machine before quoting absolute milliseconds.
