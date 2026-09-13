//! Spike B: can we sandbox a process *inside* an already-sandboxed Flatpak?
//!
//! Phase 4's extension host (issue #7) plans to confine each extension worker
//! with **Landlock** for the filesystem boundary and **seccomp** for the syscall
//! filter, and PLAN.md §6 flags the risk in one line: we are sandboxing inside
//! an already-sandboxed Flatpak, and the nesting must be verified early rather
//! than late. Nothing in that design can be trusted until it is measured —
//! bubblewrap has already installed a seccomp filter and dropped privileges
//! before our code runs.
//!
//! Reading documentation cannot settle it. Landlock's availability depends on
//! the kernel *and* on whether `no_new_privs` can still be set; seccomp nesting
//! depends on what bubblewrap's own filter permits. Both are properties of the
//! running system.
//!
//! Each question is asked in two halves — *can we apply it* and *does it
//! actually bite* — because those fail independently, and the dangerous outcome
//! is a ruleset that applies and denies nothing: it looks like a sandbox in
//! every log and confines nothing. So every "applied" field is paired with a
//! negative test that must fail closed **and** a positive control that must
//! still succeed, so that a total lockout is not mistaken for a boundary.

use std::fs;
use std::path::Path;

use serde::Serialize;

/// What the sandbox spike found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SandboxSpikeReport {
    /// Whether this process is inside a Flatpak, by `/.flatpak-info`.
    ///
    /// The nested case is the whole point, so a run with this `false` answers a
    /// different and much easier question. Recorded rather than assumed.
    pub in_flatpak: bool,
    /// Kernel release, since Landlock's ABI is a kernel property.
    pub kernel: String,
    /// The Landlock ABI this spike asked for.
    ///
    /// Fixed, not detected. The `landlock` crate deliberately offers no runtime
    /// ABI query, and says why: choosing rights from the running kernel makes
    /// sandboxing non-deterministic, so two machines — or two boots — can end
    /// up with different boundaries from the same code. The honest pair of
    /// facts is what we asked for and what the kernel granted, which is
    /// `landlock_status` below.
    pub landlock_target_abi: String,
    /// How much of the ruleset the kernel actually enforced.
    ///
    /// Landlock degrades rather than failing: an older kernel enforces the
    /// subset it understands. "Partially enforced" is a real answer, and not
    /// the same as success.
    pub landlock_status: Option<String>,
    /// Why the ruleset could not be applied, when it could not.
    pub landlock_error: Option<String>,
    /// Control: a read inside the allowed directory still works.
    pub landlock_allows_inside: Option<bool>,
    /// The assertion: a read outside it is denied.
    pub landlock_denies_outside: Option<bool>,
    /// Whether a seccomp filter could be installed on top of bubblewrap's.
    pub seccomp_applied: bool,
    /// The kernel's own view, from `/proc/self/status`, after we applied ours.
    ///
    /// This field exists to tell two very different failures apart. If the
    /// blocked syscall still succeeds, either the filter never took effect or
    /// the filter is wrong — and only the kernel can say which. Mode 2 with a
    /// non-zero filter count means our filter is loaded and the fault is ours;
    /// mode 0 means `apply_filter` returned success and nothing was installed.
    pub seccomp_mode: Option<String>,
    /// Why the filter could not be installed, when it could not.
    pub seccomp_error: Option<String>,
    /// The assertion: the syscall we blocked is refused.
    pub seccomp_denies_blocked_syscall: Option<bool>,
    /// Control: an unrelated syscall still works.
    pub seccomp_allows_others: Option<bool>,
    /// Anything a reader should know that is not one of the fields above.
    pub notes: Vec<String>,
}

/// Whether Landlock confined this process, both halves agreeing.
#[must_use]
fn landlock_confines(report: &SandboxSpikeReport) -> bool {
    matches!(report.landlock_denies_outside, Some(true))
        && matches!(report.landlock_allows_inside, Some(true))
}

/// Whether seccomp confined this process, both halves agreeing.
#[must_use]
fn seccomp_confines(report: &SandboxSpikeReport) -> bool {
    report.seccomp_applied
        && matches!(report.seccomp_denies_blocked_syscall, Some(true))
        && matches!(report.seccomp_allows_others, Some(true))
}

impl SandboxSpikeReport {
    /// One line saying what Phase 4 may rely on.
    ///
    /// Deliberately conservative: anything short of "applied, denied, and the
    /// control still passes" reads as not usable, because a boundary that is
    /// only probably there is worse than none — it would be designed against.
    #[must_use]
    pub fn verdict(&self) -> String {
        match (landlock_confines(self), seccomp_confines(self)) {
            (true, true) => {
                "both confine a process here; Phase 4's sandbox design stands".to_owned()
            }
            (true, false) => {
                "Landlock confines, seccomp does not; Phase 4 needs another syscall boundary"
                    .to_owned()
            }
            (false, true) => {
                "seccomp confines, Landlock does not; Phase 4 needs another filesystem boundary"
                    .to_owned()
            }
            (false, false) => {
                "NEITHER confines a process here; Phase 4's sandbox design does not hold as written"
                    .to_owned()
            }
        }
    }

    /// Renders the report for a person reading a CI log.
    #[must_use]
    pub fn render_human(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let yes_no = |value: Option<bool>| match value {
            Some(true) => "yes",
            Some(false) => "NO",
            None => "(not tested)",
        };

        let _ = writeln!(
            out,
            "inside a Flatpak:      {}",
            if self.in_flatpak {
                "yes"
            } else {
                "no — this answers an easier question than Phase 4 asks"
            }
        );
        let _ = writeln!(out, "kernel:                {}", self.kernel);
        let _ = writeln!(out, "Landlock (asked for): {}", self.landlock_target_abi);
        if let Some(status) = &self.landlock_status {
            let _ = writeln!(out, "  ruleset:             {status}");
        }
        if let Some(err) = &self.landlock_error {
            let _ = writeln!(out, "  could not apply:     {err}");
        }
        let _ = writeln!(
            out,
            "  reads inside allow:  {:<13} (control: must stay yes)",
            yes_no(self.landlock_allows_inside)
        );
        let _ = writeln!(
            out,
            "  reads outside deny:  {:<13} (the assertion)",
            yes_no(self.landlock_denies_outside)
        );
        let _ = writeln!(
            out,
            "seccomp filter:        {}",
            if self.seccomp_applied {
                "installed"
            } else {
                "NOT installed"
            }
        );
        if let Some(mode) = &self.seccomp_mode {
            let _ = writeln!(out, "  kernel says:         {mode}");
        }
        if let Some(err) = &self.seccomp_error {
            let _ = writeln!(out, "  could not apply:     {err}");
        }
        let _ = writeln!(
            out,
            "  blocked call denied: {:<13} (the assertion)",
            yes_no(self.seccomp_denies_blocked_syscall)
        );
        let _ = writeln!(
            out,
            "  other calls allowed: {:<13} (control: must stay yes)",
            yes_no(self.seccomp_allows_others)
        );
        for note in &self.notes {
            let _ = writeln!(out, "  note: {note}");
        }
        let _ = writeln!(out, "\nverdict: {}", self.verdict());
        out
    }
}

/// Runs Spike B against whatever this process is running inside.
///
/// Never returns an error: every failure is a finding.
///
/// The order is deliberate and cannot be rearranged. Landlock goes first and
/// seccomp second, because both are irreversible and a syscall filter installed
/// first would have to permit whatever Landlock's own setup needs. Everything
/// the report depends on afterwards — writing to an already-open stdout —
/// survives both.
#[must_use]
pub fn run() -> SandboxSpikeReport {
    let in_flatpak = Path::new("/.flatpak-info").exists();
    let mut notes = Vec::new();
    if !in_flatpak {
        notes.push(
            "not inside a Flatpak: nothing here says whether the nesting Phase 4 depends on works"
                .to_owned(),
        );
    }

    let mut report = SandboxSpikeReport {
        in_flatpak,
        kernel: fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|release| release.trim().to_owned())
            .unwrap_or_else(|err| format!("unknown ({err})")),
        landlock_target_abi: String::new(),
        landlock_status: None,
        landlock_error: None,
        landlock_allows_inside: None,
        landlock_denies_outside: None,
        seccomp_applied: false,
        seccomp_mode: None,
        seccomp_error: None,
        seccomp_denies_blocked_syscall: None,
        seccomp_allows_others: None,
        notes,
    };

    landlock_probe(&mut report);
    seccomp_probe(&mut report);
    report
}

/// Applies a read-only Landlock ruleset over one directory and tests both sides.
fn landlock_probe(report: &mut SandboxSpikeReport) {
    use landlock::{
        ABI, Access, AccessFs, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus,
        path_beneath_rules,
    };

    // V1 on purpose. It is the ABI that introduced the filesystem rights this
    // spike tests, so it is the weakest claim that still answers the question —
    // and asking for more would make "partially enforced" the expected result
    // on any kernel older than the newest, which would muddy the answer rather
    // than sharpen it. Phase 4 can raise this once it knows what it needs.
    let abi = ABI::V1;
    report.landlock_target_abi = format!("{abi:?}");

    // Two files, written before any restriction because afterwards neither
    // could be created: one the ruleset allows, one it does not.
    let allowed = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(err) => {
            report.landlock_error = Some(format!("no scratch directory: {err}"));
            return;
        }
    };
    let outside = match tempfile::NamedTempFile::new() {
        Ok(file) => file,
        Err(err) => {
            report.landlock_error = Some(format!("no outside file: {err}"));
            return;
        }
    };
    let inside = allowed.path().join("inside.txt");
    if let Err(err) = fs::write(&inside, b"inside").and_then(|()| fs::write(outside.path(), b"out"))
    {
        report.landlock_error = Some(format!("could not write the probe files: {err}"));
        return;
    }

    // From here this process is restricted for the rest of its life: a Landlock
    // ruleset cannot be undone. That is why the report goes to an already-open
    // stdout rather than to a file it would no longer be allowed to create.
    let restricted = Ruleset::default()
        .handle_access(AccessFs::from_all(abi))
        .and_then(|ruleset| ruleset.create())
        .and_then(|ruleset| {
            ruleset.add_rules(path_beneath_rules(
                [allowed.path()],
                AccessFs::from_read(abi),
            ))
        })
        .and_then(landlock::RulesetCreated::restrict_self);

    match restricted {
        Ok(status) => {
            report.landlock_status = Some(
                match status.ruleset {
                    RulesetStatus::FullyEnforced => "fully enforced",
                    RulesetStatus::PartiallyEnforced => "partially enforced",
                    RulesetStatus::NotEnforced => "NOT enforced",
                }
                .to_owned(),
            );
            if matches!(status.ruleset, RulesetStatus::NotEnforced) {
                report.notes.push(
                    "the ruleset applied but the kernel enforced none of it — which looks like a \
                     sandbox in every log and confines nothing"
                        .to_owned(),
                );
            }
        }
        Err(err) => {
            report.landlock_error = Some(err.to_string());
            return;
        }
    }

    report.landlock_allows_inside = Some(fs::read(&inside).is_ok());
    report.landlock_denies_outside = Some(fs::read(outside.path()).is_err());
}

/// The kernel's seccomp state for this process, as `/proc/self/status` reports
/// it: `mode=N filters=M`, where mode 2 is filter mode.
fn seccomp_mode_from_proc() -> Option<String> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let field = |name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name)?.split_whitespace().next())
            .unwrap_or("?")
            .to_owned()
    };
    Some(format!(
        "mode={} filters={}",
        field("Seccomp:"),
        field("Seccomp_filters:")
    ))
}

/// Installs a filter denying one syscall, then tests both sides.
fn seccomp_probe(report: &mut SandboxSpikeReport) {
    use seccompiler::{BpfProgram, SeccompAction, SeccompFilter, TargetArch};

    // Directory creation, because nothing this process does afterwards needs
    // it: the report goes to an already-open stdout. Blocking something
    // load-bearing would leave the spike unable to report its own findings.
    //
    // BOTH numbers, and that is not belt-and-braces. `std::fs::create_dir`
    // calls glibc `mkdir`, which on x86_64 is the legacy `SYS_mkdir` (83) and
    // on aarch64 — where no such syscall exists — is `SYS_mkdirat`. Blocking
    // only `mkdirat` produced a filter the kernel confirmed was loaded
    // (`mode=2 filters=1`) while the call sailed through, which reads exactly
    // like "seccomp does not nest here" and is instead "you blocked the wrong
    // number". A spike that gets this wrong argues Phase 4 out of a sandbox it
    // could have had.
    let mut blocked: std::collections::BTreeMap<i64, Vec<seccompiler::SeccompRule>> =
        [(libc::SYS_mkdirat, Vec::new())].into_iter().collect();
    #[cfg(target_arch = "x86_64")]
    blocked.insert(libc::SYS_mkdir, Vec::new());

    let arch = match std::env::consts::ARCH {
        "x86_64" => TargetArch::x86_64,
        "aarch64" => TargetArch::aarch64,
        other => {
            report.seccomp_error = Some(format!("no seccompiler target for {other}"));
            return;
        }
    };

    // Everything unnamed stays allowed. The spike measures whether a filter can
    // be installed and enforced at all, not what Phase 4's policy should be.
    let program: BpfProgram = match SeccompFilter::new(
        blocked,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM.unsigned_abs()),
        arch,
    )
    .and_then(TryInto::try_into)
    {
        Ok(program) => program,
        Err(err) => {
            report.seccomp_error = Some(format!("could not build the filter: {err}"));
            return;
        }
    };

    if let Err(err) = seccompiler::apply_filter(&program) {
        report.seccomp_error = Some(err.to_string());
        return;
    }
    report.seccomp_applied = true;
    report.seccomp_mode = seccomp_mode_from_proc();

    // The assertion. A path that does not exist, so a permitted syscall would
    // succeed — and the error is checked for EPERM specifically, because ENOENT
    // or EACCES would mean something other than the filter stopped it.
    let target = std::env::temp_dir().join("compass-spike-b-should-not-exist");
    report.seccomp_denies_blocked_syscall = Some(match fs::create_dir(&target) {
        Ok(()) => {
            let _ = fs::remove_dir(&target);
            report.notes.push(
                "the blocked syscall succeeded: the filter is installed but inert".to_owned(),
            );
            false
        }
        Err(err) => {
            let denied = err.raw_os_error() == Some(libc::EPERM);
            if !denied {
                report.notes.push(format!(
                    "the blocked syscall failed with `{err}` rather than EPERM, so something \
                     other than the filter may have stopped it"
                ));
            }
            denied
        }
    });

    // The control. If this fails too, the filter denies everything and the
    // assertion above proves nothing.
    report.seccomp_allows_others = Some(fs::metadata("/").is_ok());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> SandboxSpikeReport {
        SandboxSpikeReport {
            in_flatpak: true,
            kernel: "7.1.8-200.fc44.x86_64".to_owned(),
            landlock_target_abi: "V1".to_owned(),
            landlock_status: Some("fully enforced".to_owned()),
            landlock_error: None,
            landlock_allows_inside: Some(true),
            landlock_denies_outside: Some(true),
            seccomp_applied: true,
            seccomp_mode: Some("mode=2 filters=2".to_owned()),
            seccomp_error: None,
            seccomp_denies_blocked_syscall: Some(true),
            seccomp_allows_others: Some(true),
            notes: Vec::new(),
        }
    }

    #[test]
    fn both_confining_is_the_only_way_the_design_stands() {
        assert!(report().verdict().contains("design stands"));
    }

    #[test]
    fn a_boundary_that_denies_everything_does_not_count() {
        // The failure this pairing exists to catch: a ruleset so aggressive
        // that the allowed case fails too. It denies the forbidden thing, so a
        // one-sided check would call it a working sandbox, and Phase 4 would be
        // designed against a boundary that breaks every extension.
        let mut r = report();
        r.landlock_allows_inside = Some(false);
        assert!(r.verdict().contains("Landlock does not"));

        let mut r = report();
        r.seccomp_allows_others = Some(false);
        assert!(r.verdict().contains("seccomp does not"));
    }

    #[test]
    fn a_boundary_that_denies_nothing_does_not_count_either() {
        // The mirror image, and the more dangerous one: it looks like a sandbox
        // in every log and confines nothing.
        let mut r = report();
        r.landlock_denies_outside = Some(false);
        assert!(r.verdict().contains("Landlock does not"));

        let mut r = report();
        r.seccomp_denies_blocked_syscall = Some(false);
        assert!(r.verdict().contains("seccomp does not"));
    }

    #[test]
    fn an_untested_half_is_not_a_pass() {
        // `None` means the probe never got far enough to ask. Treating that as
        // success is how an unmeasured assumption becomes a design.
        let mut r = report();
        r.landlock_denies_outside = None;
        r.seccomp_denies_blocked_syscall = None;
        assert!(r.verdict().starts_with("NEITHER"));
    }

    #[test]
    fn a_filter_that_failed_to_apply_is_not_a_pass() {
        let mut r = report();
        r.seccomp_applied = false;
        assert!(r.verdict().contains("seccomp does not"));
    }

    #[test]
    fn the_human_rendering_names_the_controls_as_controls() {
        // A reader scanning this in CI must be able to tell the assertion from
        // the control without going to the source, or a failing control reads
        // as a failing sandbox.
        let rendered = report().render_human();
        assert!(rendered.contains("(control: must stay yes)"), "{rendered}");
        assert!(rendered.contains("(the assertion)"), "{rendered}");
        assert!(rendered.contains("mode=2 filters=2"), "{rendered}");
    }

    #[test]
    fn a_run_outside_a_flatpak_says_so_prominently() {
        // The nested case is the entire question. A green run in a plain
        // container answers something easier and must not be mistaken for it.
        let mut r = report();
        r.in_flatpak = false;
        assert!(r.render_human().contains("easier question"));
    }

    #[test]
    fn the_report_serialises_with_the_field_names_the_harness_asserts_on() {
        let value = serde_json::to_value(report()).expect("serialises");
        for field in [
            "in_flatpak",
            "kernel",
            "landlock_status",
            "landlock_allows_inside",
            "landlock_denies_outside",
            "seccomp_applied",
            "seccomp_mode",
            "seccomp_denies_blocked_syscall",
            "seccomp_allows_others",
            "notes",
        ] {
            assert!(value.get(field).is_some(), "missing `{field}` in {value}");
        }
    }
}
