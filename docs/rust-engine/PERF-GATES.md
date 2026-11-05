# Performance regression gates

§8.8 asks for Suite 4 to be "every PR informational; **blocking on `main`**". #127 asks for a
per-commit idle-RSS gate. Both use one comparator, [`scripts/ci/perf-gate.py`](../../scripts/ci/perf-gate.py),
and one workflow, [`perf.yaml`](../../.github/workflows/perf.yaml) (#306, #307).

## What is measured, and under which name

| metric | statistic | where it comes from | gated |
|---|---|---|---|
| `slas.fuzzy_rank_top20_of_10k` | criterion median, ns | `perf.yaml`, `criterion-report.py --points` | yes, on `main` |
| `slas.ipc_roundtrip_ping` | criterion median, ns | the same | yes, on `main` |
| `engine.idle_rss_kb` | median of 3 starts, kB, debug | `perf.yaml`, `crates/compass/tests/idle_memory.rs` | yes, on `main` |
| `engine.idle_hwm_kb` | median of 3 starts, kB, debug | the same | yes, on `main` |
| `engine.idle_rss_kb` (profile `flatpak`) | one reading, kB | VM tier, `launcher-rss` | recorded only |
| `window.idle_rss_kb` (renderer `llvmpipe`) | one reading, kB | VM tier, `launcher-rss` | recorded only |
| `window.idle_rss_kb` (a hardware renderer) | one reading, kB | `perf-gate.py measure-rss`, by hand | **not yet recorded** |

The engine and the resident window process have separate names. The engine draws nothing, so its
figure means the same thing in a VM and on a real machine. The window's figure is mostly the
renderer: it is about 135 MB under llvmpipe, where wgpu's software rasteriser lives inside the
process. A threshold set from that figure would gate Mesa.

The series stores criterion's **median**, not the mean that criterion prints. On the IPC row the two
statistics differ by about 6%, so a series has to use one of them throughout.

## A point

Each figure is one line of JSON. The measuring step writes the metric, the statistic, the unit and
the value. It also writes the parts of the environment that only it knows: the build profile, the
architecture and the renderer. `perf-gate.py stamp` then adds the rest on the same machine: the runner
label, the image (`ImageOS`), the image version, the CPU model from `/proc/cpuinfo`, the core count,
the head SHA, the ref, the event, the run id and the time. A point that lacks any of these is
rejected.

Two points are compared only when `runner`, `image`, `cpu`, `cores`, `arch`, `profile` and `renderer`
all match. The image version is recorded but is not part of the key. The hosted image is rebuilt
every week, so keying on it would keep every series too short to judge.

## Where the baseline comes from

`perf.yaml` runs on every pull request, on every push to `main`, and **every six hours on `main`**.
Each run measures all four gated figures in one job, on one machine, and uploads them as
`perf-points-<head sha>` with 90-day retention. The `perf-gate` job reads this run's points. It then
runs `perf-gate.py fetch`, which uses `gh run download` with `actions: read` to collect `perf-points-*`
from up to 150 earlier `perf.yaml` runs on `main`, from pushes and from the schedule. Cancelled runs
are skipped.

The schedule exists because pushes alone are too slow. `main` had 26 pushes in October 2026, spread
over at least two CPU models, and a series needs 30 points per model. A scheduled run measures code
that has already been measured, on whatever machine the pool provides. That is the runner-to-runner
spread on unchanged code, which is the spread the tolerance should come from. ADR-0022 §5 says "the
tolerance is derived from repeated runs (#306), not bought with quieter hardware". At four runs a
day, a CPU model that serves 70% of runs fills its window in about eleven days. A model that serves
30% fills it in about 25 days.

The baseline comes from artifacts, not from the actions cache. A cache entry is evicted after seven
days without use, and an eviction would silently restart every series. An artifact is kept for its
stated retention, and it belongs to the run and the commit that produced it.

## The tolerance, and the analysis behind it

For each metric, the gate takes the last **30** compatible points from `main`. It leaves out points
from the commit being judged. With fewer than 30 points it reports the count and does not judge. With
30, a figure has regressed when it is above

$$\text{median} + \max(4 \times 1.4826 \times \text{MAD},\ 1\% \times \text{median})$$

MAD is the median absolute deviation of the window, and $1.4826 \times \text{MAD}$ estimates the
standard deviation of the noise while ignoring outliers. Only an increase counts, because lower is
better for every metric here.

The constants come from replaying the history that existed before the gate.
[`benchmarks/2026-11-05-suite4-main-series/`](./benchmarks/2026-11-05-suite4-main-series/) holds the
criterion medians of all 122 pushes to `main` between 2026-09-20 and 2026-11-05 whose
`slas-criterion-*` artifacts were still retained, with each run's region and image version taken from
its log. Each table below comes from one command:

```sh
dir=docs/rust-engine/benchmarks/2026-11-05-suite4-main-series
python3 scripts/ci/perf-gate.py analyse $dir/series.jsonl                  # 1
python3 scripts/ci/perf-gate.py analyse $dir/series-split-by-mode.jsonl    # 2
for w in 10 15 20 30; do python3 scripts/ci/perf-gate.py analyse $dir/series-step-free.jsonl --window $w; done  # 3
```

**1. The hosted pool is not one machine.** The fuzzy row's medians form two clusters, with nothing
between them: 36 runs around 620 µs, up to 761 µs, and 86 runs around 904 µs, from 804 µs. The IPC
row splits the same way, at about 23 µs and 34 µs. The CPU model was not recorded, so the cause is an
inference. The clusters follow no region and no image version: both clusters appear in `centralus`,
`westus2` and `westus3`. They are taken to be different hardware behind the same `ubuntu-24.04` label.
When the clusters are mixed, the replay's typical tolerance is 22% on the fuzzy row and 48% on the
IPC row, which is too loose to catch anything. **This is why the CPU model is part of the key.**

**2. Within one cluster, the spread can be measured.** `series-split-by-mode.jsonl` splits the
history at 780 µs on the fuzzy row, as a stand-in for the CPU model that was not recorded. At K = 4
with a window of 30:

| series | points | scaled MAD | tolerance, smallest / typical | flagged |
|---|---|---|---|---|
| fuzzy, slow mode | 86 | 4.19% | 6.5% / 9.1% | 2 of 56: +8.9%, +21.4% |
| IPC, slow mode | 86 | 3.79% | 8.7% / 17.0% | 0 of 56 |
| fuzzy, fast mode | 36 | 13.67% | 47.1% / 54.2% | 0 of 6 |
| IPC, fast mode | 36 | 6.80% | 26.0% / 26.1% | 0 of 6 |

A point is flagged when it is above the limit computed from the 30 points before it. The +8.9% point is
the first of a sustained step. From commit `c7749088` (#217) onward, the slow-mode fuzzy median moved
from 878 µs to 950 µs and stayed there. The IPC row and the image version did not move, which is
consistent with a code change. K = 4 flags that first point and K = 5 lets it through. The +21.4%
point is a lone reading: the runs after it were back at 957 µs and 918 µs. K = 4, 5 and 6 all flag it.

The fast mode's spread is much wider than the slow mode's. That suggests the fast mode is itself more
than one CPU model, which the CPU key now separates. Keyed series should give tighter tolerances than
this table shows.

**3. Short windows underestimate the spread.** `series-step-free.jsonl` is the same history with the
step split into its own series, so that every flag in it is a false alarm. At K = 4:

| window | flagged |
|---|---|
| 10 | 8 of 194 |
| 15 | 5 of 169 |
| 20 | 3 of 144 |
| 30 | **0 of 94** |

Each of those flags would have been a red `main` with no regression behind it. That is why the gate
judges only with a full window of 30. At a window of 30, K = 4 and K = 5 both flag none of the 94
points. K = 4 is chosen because it also catches the real step in table 2.

**4. What the gate can catch.** In this history, a single run is flagged when it is about 9% above
the slow-mode fuzzy median, or about 17% above the slow-mode IPC median. A smaller regression shows
in the series but does not block `main`. The step in table 2 blocked one run and then became the new
baseline, as a rolling median does. Rerun `analyse` on the keyed series once it has 30 points per
model, and revisit K and the window if the spread has changed.

**5. The 1% minimum** matters in one case: a window whose MAD is zero, such as a run of identical
page-granular RSS readings. In that window, any increase at all would block `main`. The minimum is
not the operative tolerance on any series measured so far, because the smallest 4-sigma tolerance in
the replay is 6.5%.

No idle-RSS series existed before this change, because the figure was only printed. Its tolerance
comes from the same rule, applied to its own series, once 30 points exist for an environment.

## What blocks, and when

- **A push to `main`:** a regression fails the `perf-gate` job (`--blocking`).
- **A pull request, a scheduled run or a manual run:** a regression produces a warning annotation and
  a row in the step summary. The job stays green.
- **Any event:** a run with no figure for an expected metric fails. Either the bench did not run or a
  metric was renamed, and a gate that measured nothing must not pass.
- **Not judged, on any event:** an environment with fewer than 30 points, a metric with no baseline,
  and a metric whose only baseline is from another environment. In the last case the report lists the
  fields that differ. The gate never compares across environments.

## Accepting a deliberate regression

The window is a rolling median, so it adapts to a sustained change. A regression that is larger than
the tolerance would still keep `main` red until about fifteen runs had recorded it. To accept such a
regression, add an entry to [`scripts/ci/perf-resets.json`](../../scripts/ci/perf-resets.json) in the
PR that causes it:

```json
{"metric": "slas.fuzzy_rank_top20_of_10k", "after": "2026-11-20T00:00:00Z",
 "reason": "Ranking now scores acronyms; +12% measured in #NNN."}
```

Points recorded before `after` no longer count. The metric is reported, not judged, until 30 new
points exist. At the scheduled rate that takes about eleven days for the commonest CPU model.

## The resident window on real hardware

ADR-0018 moved the resident window's idle RSS on real hardware to #127. That series must be recorded
**before** a threshold is chosen. It cannot be gathered in CI or in the containers this repository is
usually worked on, because neither has a GPU. On a real GPU session, run Compass with `compass start`,
leave the window open and idle, and then run:

```sh
python3 scripts/ci/perf-gate.py measure-rss --pid "$(pgrep -f 'compass launcher-child' | head -1)" \
  --metric window.idle_rss_kb --renderer amdgpu --profile release --runner local \
  --sha "$(git rev-parse HEAD)" --out window-rss-hardware.jsonl
```

Name the renderer after the driver that wgpu actually used, such as `amdgpu`, `i915`, `xe` or
`nvidia`. Readings from different renderers are never compared. Once there are enough readings,
`perf-gate.py analyse window-rss-hardware.jsonl` gives the series' spread, which is the evidence a
window budget needs. **No such series has been recorded yet.**
