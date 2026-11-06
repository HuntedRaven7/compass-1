# Comparing the port with upstream

> For the user-facing comparison the README quotes (cold start, first frame, typing latency,
> memory, scorer throughput and size, on headless Sway), see [BENCHMARKS.md](./BENCHMARKS.md)
> and `just bench-compare`. This page covers the containerised IPC harness. `just bench-compare`
> also runs this page's `head-to-head` binary for its warm ping, and `perf.yaml` gates its
> comparable actions against upstream ([PERF-GATES.md](./PERF-GATES.md#against-upstream-vicinae)).
> The exclusions on this page still apply to that gate: search over IPC is not compared, and idle
> RSS that is no worse than upstream's is not a whole-launcher memory-efficiency verdict.

The selected baseline is **Vicinae v0.29.0**, upstream commit
`c3415a3ed56676d2960d90975ab319ae8a7aba6e`:
[release](https://github.com/vicinaehq/vicinae/releases/tag/v0.29.0).
Do not label this fork's C++ artifact as that release. Record any instrumentation
patch separately from the upstream revision.

The [first three recorded runs](./benchmarks/2026-09-20-upstream-v0.29.0/README.md)
show lower Rust ping latency in that setup. They do not establish search speed
or whole-launcher memory efficiency.

## One command, locally and in CI

On Linux x86_64, install `just`, Python 3 and Podman, then run:

```sh
just bench-head-to-head
# Optional output parent (each invocation creates a new run directory):
just bench-head-to-head /tmp/compass-results
```

The same command runs in `.github/workflows/head-to-head.yaml` on relevant PRs,
main updates, weekly, and manual dispatch. The first run downloads a container
and compiles the Rust release; allow several minutes and several GB of disk.
Podman's image layers and the named `compass-head-to-head-cargo-v1` volume cache
subsequent builds. No host Rust, Qt or display server is required. Local cache
removal is deliberately not part of the benchmark command.

The runner verifies the pinned AppImage SHA256, mounts the checkout read-only,
builds release binaries, then starts a separate network-disabled measurement
container with fresh XDG profiles and its own Xvfb display. It marks only the
temporary upstream profile's onboarding complete before launch and verifies both
launcher windows are mapped. It includes the independent Rust UI process in
memory accounting. Owned processes are stopped and reaped on success or failure.

Results live under `target/head-to-head/run-*/`: human summary, three raw reports,
configuration, source/build identity, corpus hash, image ID, package/toolchain
versions, window IDs and logs. CI uploads these even on failure. Dirty local
source is marked and content-hashed, not silently called the recorded commit.
The base image, Rust toolchain and upstream artifact are pinned; Fedora package
updates and different host hardware can still change measurements. Compare
reported environments, not just the headline numbers.

`just bench-check` runs fast orchestration tests. For an advanced attached
workload, use `just bench-attached CONFIG.json REPORT.json`.

**Current default scope: unmodified upstream release, ping and diagnostic
RSS/PSS. Search remains excluded, and unequal feature coverage still prevents
a whole-launcher memory-efficiency verdict.** The instrumentation patch beside
the runner is for the next search workload; this recipe does not silently
substitute that modified build for the user's chosen pristine baseline.

`cargo run --release -p compass-testkit --bin head-to-head -- CONFIG.json REPORT.json`
compares two already-running engines over persistent Unix sockets. It does not
start, stop, configure or mutate their indexes. Use isolated test profiles, not
the user's running launcher. Reports refuse to overwrite an existing file.

```json
{
  "cpp": {
    "socket": "/tmp/bench/cpp/runtime/vicinae/vicinae.sock",
    "revision": "vicinaehq/vicinae@c3415a3ed56676d2960d90975ab319ae8a7aba6e (v0.29.0, unmodified)",
    "build": "release AppImage; record downloaded artifact SHA256 here",
    "process_roots": [1234]
  },
  "rust": {
    "socket": "/tmp/bench/rust/runtime/ipc.sock",
    "revision": "record exact port commit and dirty diff hash here",
    "build": "cargo --release; record rustc version and binary SHA256 here",
    "process_roots": [5678, 5679]
  },
  "workload": "record corpus SHA256, entry count, config, package, compositor/renderer, UI visibility, extension count, CPU governor and environment here",
  "samples": 1000,
  "warmup": 100,
  "queries": []
}
```

PIDs are examples. Include the Rust UI's independently spawned process as well
as its daemon; include other independent worker roots if applicable. Descendants
are discovered through all processes' PPids, not just the leader thread's child
list. The engine's ping PID must appear among its roots. Run in the same PID
namespace as the measured engines. Missing/inaccessible memory, zero memory,
overlapping process trees, RPC errors and timeouts fail the run.

## What the output means

- Each action is warmed before timing. AB/BA pairs alternate order; connection
  setup and CLI process spawn are outside the timed interval.
- Latency includes client encode, server execution, reply decode and extraction
  of IDs. It is not an isolated algorithm benchmark. The report retains every
  sample and reports nearest-rank median/p95/p99, min and max in nanoseconds.
- Memory records 20 interleaved process-tree samples, in KiB. Summed RSS counts
  shared pages more than once; PSS apportions those pages. Neither is peak memory.
- Environment and supplied build/workload provenance travel with the results.
  The harness cannot verify that an operator's workload description is true.
  Review process lists, package hashes, corpus and visible UI before interpreting
  memory as an efficiency comparison. A daemon-only Rust measurement versus the
  full Qt launcher is **not** a whole-launcher memory win.
- No pass/fail performance threshold is invented from one run. Repeat on the
  target machine and measure spread before choosing a tolerance.

## Search is a separate prerequisite

The [instrumented upstream inventory audit](./benchmarks/2026-09-20-query-audit/README.md)
initially demonstrated an end-to-end failure: 448 upstream root applications versus 452
Rust items, with desktop-action, TryExec, entry-type and search-field differences.
The [root integration follow-up](./benchmarks/2026-09-20-root-integration/README.md)
now uses the ported root scorer in daemon and UI. Its sampled nonempty queries
have equal membership, and four positive ASCII queries have identical ordering.
The [desktop-link follow-up](./benchmarks/2026-09-20-desktop-links/README.md)
also matches all 448 empty-query results in order. Unicode ranking still fails.
The [Unicode boundary fix](./benchmarks/2026-09-20-unicode-boundaries/README.md)
corrects the Bear Factory editor matches; 14 later positions still differ.
Resolve the remaining ranking difference and rerun
ordered-ID parity before timing search; do not select only matching queries to
conceal the failing workload.

Unmodified v0.29.0 has `Ipc/ping` but **no `Ipc/rootQuery`**. `queries: []`
therefore excludes search visibly. The C++ instrumentation merged in #126 is
useful for the fork, but is not evidence about an unmodified upstream release.
An upstream search comparison needs a documented minimal instrumentation patch
on the pinned release, or compositor-driven measurements of its actual UI.

When rootQuery instrumentation is present, supply independent expected results:

```json
{"text":"Benchmark Sentinel","expected_ids":["benchmark-sentinel.desktop"]}
```

The harness selects C++'s `applications` provider, requests all results and
normalizes `applications:<id>` to Rust's `<id>.desktop`. Every response must match
the expected ordered IDs before its timings can be reported. At least one query
must expect a hit; two empty indexes cannot pass. Use the same installed corpus,
locale, frecency/config and result limit for both. Include broad, narrow, Unicode
and miss queries; a single sentinel verifies readiness, not ranking parity.

## Remaining Suite 4b work

This harness covers persistent ping, instrumented query and process-tree RSS/PSS.
It does **not** cover launch, show/hide reply latency, first-frame latency,
peak memory with extensions, or the equivalent visible Flatpak workload. `just bench-compare`
now covers cold launch and first frame on headless Sway, and the upstream gate judges them. Show/hide
latency, peak memory with extensions and the Flatpak workload are still required by PLAN
§8.5b/#117. Cold start and GPU timing must not be inferred
from an IPC reply. The first report is evidence, not completion of the port or
proof that it is faster and more memory efficient overall.
