# Issues already resolved on `main`

This records open issues whose work has landed, so they can be closed rather
than mistaken for outstanding tasks. Each row names the change that closed the
gap and the file where the result can be checked. It is a status note, not a
new commitment of work.

| Issue | What it asked for | Where it landed | Check here |
|---|---|---|---|
| #127 | The four unfinished performance gates from #13: blocking Suite 4, a per-commit RSS gate, four `insta` snapshots, and the Depot decision. | Suite 4 and idle RSS are gated against an environment-keyed baseline from `main` (#339, #349); five semantic `insta` snapshots exist (#169, #334); Depot was declined in ADR-0022 §5. | `PERF-GATES.md`, `crates/compass-ui/tests/insta_snapshots.rs`, `PLAN.md` §8.8 |
| #154 | Minimal-effort install and first-run flow, with clean restart, update and uninstall. | The install flow and its durable tests landed in #167; the full fresh-install / login-start / restart / upgrade / uninstall journey was gated in #350. | `docs/getting-started.md`, `scripts/tier2/install-journey.sh` |
| #258 | `compass-ui/src/app.rs` was a 19k-line god object. | Split into a coordinator and per-concern modules in #353. `app.rs` is now about 1.3k lines and no longer listed in the module-size allowlist. | `crates/compass-ui/src/app.rs`, `scripts/architecture/allowlist.txt` |
| #307 | Gate idle RSS with environment-keyed baselines, keeping incompatible environments separate. | The environment-keyed RSS series, tolerance from measured spread, and refusal to compare incompatible baselines landed in #339 and #349; the resident-window series is recorded in `PERF-GATES.md`. | `PERF-GATES.md`, `.github/workflows/perf.yaml` |
| #314 | Gate fresh install, restart, upgrade and uninstall journeys in a real session. | The install-journey gate landed in #350, covering fresh install, app-grid first run, shortcut, login start, logout/login, update, repeated activation and uninstall. | `scripts/tier2/install-journey.sh`, `.github/workflows/tier2.yaml` |

Other open issues are deliberately not listed here: some await an owner's
decision, some wait on upstream releases, and some are still being built. This
table is only for issues whose work has already landed.
