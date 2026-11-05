//! The platform seam is a dependency-direction rule, so it is checked as one.
//!
//! [ADR-0013](../../../docs/rust-engine/adr/0013-qt-leaves-the-repository.md)
//! commits macOS and Windows to their own phases, which makes one property
//! load-bearing: the crates every platform shares must not reach a
//! Linux-specific one. That property is invisible in code review — it is a
//! line in a `Cargo.toml`, and it was already violated once. `compass-ui`
//! carried `compass-portals` and `compass-wayland` as dependencies while using
//! neither, and `compass-platform` — the crate *named* for the abstraction —
//! was the Linux launcher.
//!
//! A rule nobody checks is a rule that drifts back, so this reads the
//! manifests.
//!
//! WHAT THIS DOES NOT CLAIM
//!
//! It checks declared dependencies, not portability. A shared crate can be
//! unbuildable on macOS for reasons no manifest shows — a `/proc` path, a
//! Linux-only syscall through `libc`. Only building on macOS proves that, and
//! Phase 9 is where that happens. This catches the specific, cheap, recurring
//! mistake: the edge that should not exist.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Crates that may depend on Linux-specific ones: the per-platform backends
/// and the binary that composes them.
const MAY_BE_LINUX_BOUND: &[&str] = &[
    "compass-platform-linux",
    "compass-portals",
    "compass-shell",
    "compass-wayland",
    // Generated bindings for Wayland protocols no published crate carries
    // (xx-hotkey-v1), split out of compass-wayland only because the
    // generated code needs `unsafe`.
    "compass-wayland-protocols",
    // Turns a toolkit window's raw `wl_display*` and `wl_surface*` into
    // wayland-client proxies (background blur on the launcher's own surface).
    // libwayland's handles are the Linux mechanism, and the crate is split out
    // only because the bridge needs `unsafe` (ADR-0019). Other platforms'
    // `WindowMaterial` implementations would be their own crates.
    "compass-wayland-foreign",
    "compass",
    // The test harness drives Linux surfaces on purpose.
    "compass-testkit",
    // Same story: logind is the Linux mechanism, and the C++ has a separate
    // power manager for each platform.
    "compass-power",
    // MPRIS is a session-bus protocol; the C++ has a Windows media backend
    // beside this one for the same reason.
    "compass-media",
    // The worker host sets cgroup memory limits through the system bus
    // (logind/systemd); PLAN.md assigns cgroups v2 to this crate, and
    // cgroups are the Linux mechanism the way MPRIS is the Linux protocol —
    // the C++ has a separate supervisor per platform.
    "compass-worker-host",
    // compass-input-server: evdev and uinput are the Linux mechanism for
    // reading and injecting keys; the C++ has a separate snippet server for
    // macOS and Windows, and so will the ports of those.
    "compass-input-server",
    // The launcher's accessibility tree on AT-SPI, through `accesskit_unix`
    // (ADR-0022 §1). AT-SPI is the Linux accessibility bus the way MPRIS is
    // the Linux media protocol; the tree itself is described in `compass-ui`
    // with the platform-neutral `accesskit`, and AccessKit's macOS and
    // Windows adapters would be their own crates beside this one.
    "compass-a11y-linux",
];

/// Crates whose presence in a manifest makes that crate Linux-bound.
const LINUX_ONLY_CRATES: &[&str] = &[
    "compass-portals",
    "compass-shell",
    "compass-wayland",
    "compass-wayland-protocols",
    "compass-wayland-foreign",
    "compass-platform-linux",
    "compass-a11y-linux",
    // The direct ones, in case a shared crate reaches past our wrappers.
    "zbus",
    "ashpd",
    "wayland-client",
    "wayland-backend",
    "wayland-sys",
    "wayland-protocols",
    "smithay",
    "accesskit_unix",
    "atspi",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/compass-platform sits two levels below the workspace root")
        .to_path_buf()
}

/// Dependency names declared by a manifest, from its dependency sections.
///
/// Deliberately a line scan rather than a TOML parse: this must run with no
/// new dependency, and the manifests in this workspace are one dependency per
/// line. The control below proves the scan sees what it should.
fn declared_dependencies(manifest: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut in_deps = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_deps = line.contains("dependencies");
            continue;
        }
        if !in_deps || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = line.split_once('=') {
            // `compass-portals.workspace = true` — the key carries a suffix,
            // and the crate name is what comes before the first dot. The
            // first version of this kept the whole key, so every lookup for
            // "compass-portals" missed and both sweeps passed vacuously. The
            // controls below are what caught it.
            let key = key.trim().trim_matches('"');
            let name = key.split('.').next().unwrap_or(key);
            out.insert(name.to_owned());
        }
    }
    out
}

fn shared_crates() -> Vec<(String, PathBuf)> {
    let crates_dir = workspace_root().join("crates");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&crates_dir).expect("crates/ should be readable") {
        let path = entry.expect("readable entry").path();
        let manifest = path.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("crate directory name")
            .to_owned();
        if MAY_BE_LINUX_BOUND.contains(&name.as_str()) {
            continue;
        }
        out.push((name, manifest));
    }
    out.sort();
    assert!(
        out.len() >= 5,
        "found only {} shared crates, which means the scan is not finding the workspace",
        out.len()
    );
    out
}

#[test]
fn no_shared_crate_depends_on_a_linux_only_crate() {
    let mut violations = Vec::new();

    for (name, manifest_path) in shared_crates() {
        let manifest = std::fs::read_to_string(&manifest_path).expect("manifest is readable");
        let deps = declared_dependencies(&manifest);
        for linux in LINUX_ONLY_CRATES {
            if deps.contains(*linux) {
                violations.push(format!("{name} -> {linux}"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "a crate shared by every platform depends on a Linux-specific one:\n  {}\n\n\
         ADR-0013 commits macOS and Windows to their own phases. An edge like this makes that \
         work more expensive and is invisible in review, which is how `compass-ui` ended up \
         carrying compass-portals and compass-wayland without using either. Put the \
         implementation behind a trait in compass-platform and select it in the `compass` \
         binary; if the crate genuinely is a platform backend, add it to MAY_BE_LINUX_BOUND and \
         say why.",
        violations.join("\n  ")
    );
}

/// compass-platform is the seam, so it gets its own assertion rather than
/// relying on being one row of the sweep above.
#[test]
fn compass_platform_names_operations_without_implementing_them() {
    let manifest =
        std::fs::read_to_string(workspace_root().join("crates/compass-platform/Cargo.toml"))
            .expect("own manifest is readable");
    let deps = declared_dependencies(&manifest);

    for linux in LINUX_ONLY_CRATES {
        assert!(
            !deps.contains(*linux),
            "compass-platform depends on {linux}. It did before -- it WAS the Linux launcher, \
             flatpak-spawn and the OpenURI portal, in the crate named for the abstraction. \
             That is the thing ADR-0013 fixed; it should not come back."
        );
    }
}

// ---------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------

#[test]
fn control_the_scanner_finds_dependencies_that_are_there() {
    let sample = "\
[package]
name = \"x\"

[dependencies]
compass-xdg.workspace = true
tokio = { workspace = true }
# commented-out.workspace = true

[dev-dependencies]
tempfile.workspace = true
";
    let deps = declared_dependencies(sample);
    assert!(
        deps.contains("compass-xdg"),
        "missed a workspace dependency"
    );
    assert!(deps.contains("tokio"), "missed an inline-table dependency");
    assert!(deps.contains("tempfile"), "missed a dev-dependency");
    assert!(
        !deps.contains("commented-out"),
        "a commented-out line was read as a dependency"
    );
    assert!(
        !deps.contains("name"),
        "a [package] key was read as a dependency; the section tracking is wrong"
    );
}

/// The sweep must actually fail on a violating manifest. Without this, every
/// assertion above could be passing because the scanner returns nothing.
#[test]
fn control_a_linux_edge_in_a_shared_crate_is_detected() {
    let offending = "\
[package]
name = \"compass-search\"

[dependencies]
compass-portals.workspace = true
";
    let deps = declared_dependencies(offending);
    let found: Vec<_> = LINUX_ONLY_CRATES
        .iter()
        .filter(|l| deps.contains(**l))
        .collect();
    assert_eq!(
        found,
        vec![&"compass-portals"],
        "the check would not have noticed a shared crate depending on compass-portals"
    );
}

#[test]
fn control_the_sweep_actually_covers_the_shared_crates() {
    let names: Vec<String> = shared_crates().into_iter().map(|(n, _)| n).collect();
    for expected in [
        "compass-core",
        "compass-search",
        "compass-ui",
        "compass-platform",
    ] {
        assert!(
            names.contains(&expected.to_owned()),
            "{expected} is not in the swept set {names:?}, so the rule is not being applied to it"
        );
    }
}
