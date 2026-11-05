//! Idle RSS of the engine, measured on every commit.
//!
//! #13 asks for "per-commit RSS tracking with a >5% regression gate" and that
//! could not be built where the number already lived. The VM tier measures the
//! sandboxed engine (`checks.sh launcher-rss`), but the VM tier has no `push`
//! trigger — nightly, `workflow_dispatch`, and since the trigger change, `v*`
//! tags. Merging to `main` produces no tier run, so **there has never been a
//! per-commit series to regress against.**
//!
//! This is the Tier-1 half: a number on every PR and every commit, from a job
//! that already runs in under a minute.
//!
//! # What this is not
//!
//! **Not the §8.5 SLA.** That row says "< 30 MB" and §8.5 is explicit that
//! "the number users see is the sandboxed one" — measured inside the Flatpak.
//! This measures the bare binary on the runner, with no sandbox, no portal and
//! no compositor. The two numbers are not interchangeable and this one must
//! never be quoted as the SLA being met.
//!
//! What it is good for is the thing the sandboxed number cannot do: appear on
//! every commit, so a change that doubles the engine's resident set is visible
//! in the PR that caused it rather than in a nightly run nobody reads.
//!
//! # Where the regression gate lives
//!
//! ADR-0010: a threshold is measured before it is invented, so this test does
//! not hold one. With `COMPASS_PERF_POINTS` set it appends its figures, as JSON
//! lines, to that file; CI stamps them with the runner's environment and
//! `scripts/ci/perf-gate.py` judges them against the series from earlier `main`
//! runs measured in the same environment and profile
//! (`docs/rust-engine/PERF-GATES.md`). The test itself asserts only a **sanity
//! ceiling** — set well above the measured value, to catch a leak or a runaway
//! allocation rather than to police megabytes — so a developer running it
//! locally gets the same protection without a baseline.
//!
//! The metrics are `engine.idle_rss_kb` and `engine.idle_hwm_kb`: the engine,
//! named apart from the resident window process (`window.idle_rss_kb`), whose
//! resident set is mostly the renderer and is only comparable within one
//! renderer class.
//!
//! Measured on one machine, debug, four consecutive runs: **24 768 / 24 792 /
//! 24 860 / 24 844 kB** — a spread of 0.4%. The same commit read **26 760 kB**
//! on a runner, ~8% apart. Both facts shape the gate: the figure is stable
//! enough within one environment to gate, and different enough between
//! environments that a baseline from another machine is not a baseline.
//!
//! # The figure is only comparable within one build profile
//!
//! These are **debug** numbers, because `cargo test -p compass` is what CI
//! runs and that is a debug build. A release engine will differ, so comparing a
//! release reading against this series would manufacture a regression that is
//! really a profile change. Each figure therefore carries its profile, and the
//! gate refuses to compare across profiles.
//!
//! No release figure is recorded here yet: measuring one needs a release build
//! of the workspace, and the machine this was written on ran out of disk
//! allowance before finishing it. Left unmeasured rather than estimated.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::json;
use tempfile::TempDir;

/// A session bus address with nothing behind it, for every engine this file
/// starts. The engine opens clipboard history through the login keyring on
/// the session bus; pointed at a real one, running these tests would create a
/// Compass key in the developer's own keyring.
const NO_SESSION_BUS: &str = "unix:path=/nonexistent/compass-test-no-session-bus";

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// How many generated entries the engine indexes before being measured.
///
/// A real desktop is a few hundred; the Bluefin harvest is 757. Using a
/// realistic count rather than an empty index matters because an engine
/// measured with nothing loaded would report a number no user ever sees.
const ENTRIES: usize = 757;

/// How many engines are started and measured, one after another.
///
/// The figure reported is their median. One reading is at the mercy of
/// whatever else the runner was doing during the settle, and a gate that blocks
/// `main` should not be.
const SAMPLES: usize = 3;

/// Where CI asks for the figures as JSON lines, for `scripts/ci/perf-gate.py`.
const POINTS_ENV: &str = "COMPASS_PERF_POINTS";

/// The build profile the figures belong to. The gate keys on it, because a
/// debug engine's resident set is not a release engine's.
const PROFILE: &str = if cfg!(debug_assertions) {
    "debug"
} else {
    "release"
};

fn binary() -> PathBuf {
    let mut path = std::env::current_exe().expect("test binary path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("compass")
}

fn status_kb(pid: u32, field: &str) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let prefix = format!("{field}:");
    status
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
}

struct Engine {
    child: Child,
    _dirs: TempDir,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start() -> (Engine, u32) {
    let dirs = TempDir::new().expect("tempdir");
    let data = dirs.path().join("data/applications");
    std::fs::create_dir_all(&data).expect("applications dir");

    for i in 0..ENTRIES {
        std::fs::write(
            data.join(format!("generated-{i}.desktop")),
            format!(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=Generated Application {i}\n\
                 Comment=A synthetic entry standing in for a real one\n\
                 Exec=/usr/bin/true --instance {i}\n\
                 Icon=application-x-executable\n"
            ),
        )
        .expect("write entry");
    }

    let socket = dirs.path().join("ipc.sock");
    let child = Command::new(binary())
        .arg("--socket")
        .arg(&socket)
        .arg("serve")
        .env("DBUS_SESSION_BUS_ADDRESS", NO_SESSION_BUS)
        .env("COMPASS_DISABLE_AUTO_RATE_REFRESH", "1")
        .env("XDG_DATA_DIRS", dirs.path().join("data"))
        .env("XDG_DATA_HOME", dirs.path().join("data-home"))
        .env("XDG_CONFIG_HOME", dirs.path().join("config"))
        .env("XDG_CACHE_HOME", dirs.path().join(".cache"))
        .env("HOME", dirs.path())
        .env_remove("XDG_STATE_HOME")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the engine");

    let pid = child.id();
    let engine = Engine { child, _dirs: dirs };

    // Measured only once it answers: a process that has not finished starting
    // has not finished allocating, and the number would depend on how fast the
    // runner happened to be.
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut listening = false;
    while Instant::now() < deadline {
        let answered = Command::new(binary())
            .arg("--socket")
            .arg(socket.as_path())
            .arg("ping")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if answered {
            listening = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        listening,
        "the engine never answered a ping, so whatever RSS it has is not an idle engine's"
    );

    (engine, pid)
}

/// One idle engine's resident set and peak, in kB.
fn measure() -> (u64, u64) {
    let (engine, pid) = start();

    // Settle, so what is measured is a resting engine rather than one still
    // finishing its first index pass.
    std::thread::sleep(Duration::from_millis(500));

    let rss = status_kb(pid, "VmRSS").expect("the engine's VmRSS");
    let hwm = status_kb(pid, "VmHWM").expect("the engine's VmHWM");
    drop(engine);
    (rss, hwm)
}

fn median(samples: &[u64]) -> u64 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

/// Appends the figures to `$COMPASS_PERF_POINTS` as perf-gate points.
///
/// Only what this process knows goes in: the metric, the profile, the
/// architecture. The runner, image and CPU are stamped by CI on the same
/// machine, and the commit by the workflow that knows which head it built.
fn write_points(path: &std::ffi::OsStr, rss: &[u64], hwm: &[u64]) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap_or_else(|error| panic!("open {}: {error}", path.to_string_lossy()));
    for (metric, samples) in [("engine.idle_rss_kb", rss), ("engine.idle_hwm_kb", hwm)] {
        let point = json!({
            "metric": metric,
            "statistic": "median",
            "unit": "kB",
            "value": median(samples),
            "samples": samples,
            "env": {
                "profile": PROFILE,
                "arch": std::env::consts::ARCH,
                // The engine draws nothing, so no renderer can be in its figure.
                "renderer": "none",
            },
        });
        writeln!(file, "{point}").expect("append a perf point");
    }
}

#[test]
fn the_idle_engine_reports_its_resident_set() {
    if status_kb(std::process::id(), "VmRSS").is_none() {
        println!("skipping: /proc/self/status is unreadable, so this is not Linux");
        return;
    }

    let (rss, hwm): (Vec<u64>, Vec<u64>) = (0..SAMPLES).map(|_| measure()).unzip();
    let (median_rss, peak_hwm) = (median(&rss), hwm.iter().copied().max().unwrap_or(0));

    println!(
        "engine idle, {ENTRIES} entries indexed, UNSANDBOXED, {PROFILE} build, median of \
         {SAMPLES} starts: VmRSS {median_rss} kB ({} MB), peak VmHWM {peak_hwm} kB ({} MB); \
         samples VmRSS {rss:?}, VmHWM {hwm:?}",
        median_rss / 1024,
        peak_hwm / 1024
    );
    println!(
        "  NOT the §8.5 SLA. That row is < 30 MB measured inside the Flatpak; this is the \
         bare binary on a runner. Per-commit trend signal only — see the module docs."
    );

    // A floor, for the reason every other measurement here has one: an
    // unreadable or already-dead process reports a small number, and a small
    // number passes a ceiling. The VM tier's RSS gate had exactly this defect,
    // passing on `0 kB`.
    for &reading in &rss {
        assert!(
            reading > 1024,
            "the engine reported {reading} kB, which is not a running engine — the \
             measurement is what is wrong, not the engine"
        );
    }

    // The sanity ceiling. Deliberately loose: its job is to catch a leak or a
    // runaway allocation, not to police megabytes. Regressions inside it are
    // perf-gate.py's to judge, and the sandboxed budget in §8.5 is gated in the
    // VM tier at 20 MB.
    const CEILING_KB: u64 = 150 * 1024;
    assert!(
        peak_hwm < CEILING_KB,
        "the idle engine peaked at {peak_hwm} kB, over the {CEILING_KB} kB sanity ceiling. \
         This is not the SLA — it is loose enough that crossing it means something is \
         leaking or allocating without bound, which is worth a look before the number is \
         tightened"
    );

    // Written after the assertions, so a run that measured nothing usable
    // leaves no point for the gate to mistake for a baseline.
    if let Some(path) = std::env::var_os(POINTS_ENV) {
        write_points(&path, &rss, &hwm);
    }
}
