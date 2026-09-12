//! Turning a list of checks into something a person or a CI job can read.

use compass_ipc::{DoctorCheck, DoctorStatus};
use serde::Serialize;

use crate::engine::Engine;

/// Version of the `--json` document shape.
///
/// Bumped when a field changes meaning or disappears. CI jobs that assert on
/// the output (PLAN §8.8 Tier 2/3) should check it and refuse to guess.
pub const JSON_SCHEMA_VERSION: u32 = 1;

/// Overall verdict across every check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Every check passed.
    Ok,
    /// Some checks are degraded, none are broken.
    Warn,
    /// At least one check is broken.
    Fail,
}

impl Verdict {
    /// The lowercase spelling used in `--json`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
}

/// Counts per status, plus the resulting verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// Number of passing checks.
    pub ok: usize,
    /// Number of degraded checks.
    pub warn: usize,
    /// Number of broken checks.
    pub fail: usize,
    /// Total number of checks run.
    pub total: usize,
    /// Worst status seen, as a lowercase string.
    pub status: &'static str,
}

/// The finished diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Engine this invocation selected.
    pub engine: Engine,
    /// IPC socket path the checks were run against.
    pub socket: String,
    /// Every check, in the order they ran.
    pub checks: Vec<DoctorCheck>,
}

impl Report {
    /// Counts and verdict.
    #[must_use]
    pub fn summary(&self) -> Summary {
        let mut ok = 0;
        let mut warn = 0;
        let mut fail = 0;
        for c in &self.checks {
            match c.status {
                DoctorStatus::Ok => ok += 1,
                DoctorStatus::Warn => warn += 1,
                DoctorStatus::Fail => fail += 1,
            }
        }
        Summary {
            ok,
            warn,
            fail,
            total: self.checks.len(),
            status: self.verdict().as_str(),
        }
    }

    /// Worst status across every check.
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        if self.checks.iter().any(|c| c.status == DoctorStatus::Fail) {
            Verdict::Fail
        } else if self.checks.iter().any(|c| c.status == DoctorStatus::Warn) {
            Verdict::Warn
        } else {
            Verdict::Ok
        }
    }

    /// Whether `--check-only` should exit non-zero.
    ///
    /// Failures only. Warnings are degradations we have chosen to support —
    /// no Shell extension, no `XDG_RUNTIME_DIR` — and failing CI on them would
    /// train people to pass `--check-only` less, not to fix things.
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.verdict() == Verdict::Fail
    }

    /// The machine-readable document behind `--json`.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        #[derive(Serialize)]
        struct JsonCheck<'a> {
            name: &'a str,
            status: &'static str,
            detail: Option<&'a str>,
        }
        #[derive(Serialize)]
        struct JsonReport<'a> {
            schema: u32,
            engine: &'a str,
            socket: &'a str,
            summary: Summary,
            checks: Vec<JsonCheck<'a>>,
        }

        let document = JsonReport {
            schema: JSON_SCHEMA_VERSION,
            engine: self.engine.as_str(),
            socket: &self.socket,
            summary: self.summary(),
            checks: self
                .checks
                .iter()
                .map(|c| JsonCheck {
                    name: &c.name,
                    status: status_str(c.status),
                    detail: c.detail.as_deref(),
                })
                .collect(),
        };

        serde_json::to_value(document).expect("doctor report is always serialisable")
    }

    /// The aligned human report.
    ///
    /// `only_problems` drops passing checks, which is what `--check-only`
    /// prints: the point of that mode is the exit code, and a wall of green is
    /// noise around it.
    #[must_use]
    pub fn render_human(&self, only_problems: bool) -> String {
        let shown: Vec<&DoctorCheck> = self
            .checks
            .iter()
            .filter(|c| !only_problems || c.status != DoctorStatus::Ok)
            .collect();

        let width = shown.iter().map(|c| c.name.len()).max().unwrap_or(0);
        // marker + space + padded name + space
        let indent = MARKER_WIDTH + 1 + width + 1;

        let mut out = String::new();
        out.push_str(&format!(
            "vicinae doctor — engine: {}, socket: {}\n\n",
            self.engine, self.socket
        ));

        if shown.is_empty() {
            out.push_str("no problems found\n\n");
        }

        for c in shown {
            let detail = c.detail.as_deref().unwrap_or("");
            let body = wrap(detail, LINE_WIDTH.saturating_sub(indent));
            let mut lines = body.into_iter();
            let first = lines.next().unwrap_or_default();
            out.push_str(&format!(
                "{} {:<width$} {first}\n",
                marker(c.status),
                c.name,
            ));
            for line in lines {
                out.push_str(&format!("{:indent$}{line}\n", ""));
            }
        }

        let s = self.summary();
        out.push_str(&format!(
            "\n{} checks: {} ok, {} {}, {} {}\n",
            s.total,
            s.ok,
            s.warn,
            plural(s.warn, "warning", "warnings"),
            s.fail,
            plural(s.fail, "failure", "failures"),
        ));
        out
    }
}

const LINE_WIDTH: usize = 100;
const MARKER_WIDTH: usize = 6;

fn marker(status: DoctorStatus) -> &'static str {
    match status {
        DoctorStatus::Ok => "[ ok ]",
        DoctorStatus::Warn => "[warn]",
        DoctorStatus::Fail => "[FAIL]",
    }
}

fn status_str(status: DoctorStatus) -> &'static str {
    match status {
        DoctorStatus::Ok => "ok",
        DoctorStatus::Warn => "warn",
        DoctorStatus::Fail => "fail",
    }
}

fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

/// Word-wraps `text` to `width`, preserving the newlines already in it.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(20);
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if !line.is_empty() && line.len() + 1 + word.len() > width {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(name: &str, status: DoctorStatus) -> DoctorCheck {
        DoctorCheck {
            name: name.to_string(),
            status,
            detail: Some(format!("detail for {name}")),
        }
    }

    fn report(statuses: &[DoctorStatus]) -> Report {
        Report {
            engine: Engine::Rust,
            socket: "/run/user/1000/vicinae/ipc.sock".to_string(),
            checks: statuses
                .iter()
                .enumerate()
                .map(|(i, s)| check(&format!("check.{i}"), *s))
                .collect(),
        }
    }

    #[test]
    fn all_ok_is_a_clean_verdict() {
        let r = report(&[DoctorStatus::Ok, DoctorStatus::Ok]);
        assert_eq!(r.verdict(), Verdict::Ok);
        assert!(!r.has_failures());
        assert_eq!(
            r.summary(),
            Summary {
                ok: 2,
                warn: 0,
                fail: 0,
                total: 2,
                status: "ok"
            }
        );
    }

    #[test]
    fn warnings_alone_do_not_fail() {
        let r = report(&[DoctorStatus::Ok, DoctorStatus::Warn, DoctorStatus::Warn]);
        assert_eq!(r.verdict(), Verdict::Warn);
        assert!(!r.has_failures());
        assert_eq!(r.summary().warn, 2);
    }

    #[test]
    fn one_failure_dominates() {
        let r = report(&[DoctorStatus::Ok, DoctorStatus::Warn, DoctorStatus::Fail]);
        assert_eq!(r.verdict(), Verdict::Fail);
        assert!(r.has_failures());
        assert_eq!(r.summary().status, "fail");
    }

    #[test]
    fn empty_report_is_ok() {
        let r = report(&[]);
        assert_eq!(r.verdict(), Verdict::Ok);
        assert_eq!(r.summary().total, 0);
    }

    #[test]
    fn json_has_the_documented_shape() {
        let value = report(&[DoctorStatus::Ok, DoctorStatus::Fail]).to_json();
        assert_eq!(value["schema"], JSON_SCHEMA_VERSION);
        assert_eq!(value["engine"], "rust");
        assert_eq!(value["socket"], "/run/user/1000/vicinae/ipc.sock");
        assert_eq!(value["summary"]["ok"], 1);
        assert_eq!(value["summary"]["fail"], 1);
        assert_eq!(value["summary"]["total"], 2);
        assert_eq!(value["summary"]["status"], "fail");
        let checks = value["checks"].as_array().expect("checks is an array");
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0]["name"], "check.0");
        assert_eq!(checks[0]["status"], "ok");
        assert_eq!(checks[1]["status"], "fail");
        assert_eq!(checks[0]["detail"], "detail for check.0");
    }

    #[test]
    fn human_output_aligns_and_summarises() {
        let text =
            report(&[DoctorStatus::Ok, DoctorStatus::Warn, DoctorStatus::Fail]).render_human(false);
        assert!(text.contains("[ ok ] check.0"));
        assert!(text.contains("[warn] check.1"));
        assert!(text.contains("[FAIL] check.2"));
        assert!(text.contains("3 checks: 1 ok, 1 warning, 1 failure"));
    }

    #[test]
    fn check_only_output_hides_passing_checks() {
        let text = report(&[DoctorStatus::Ok, DoctorStatus::Fail]).render_human(true);
        assert!(!text.contains("check.0"));
        assert!(text.contains("[FAIL] check.1"));
    }

    #[test]
    fn check_only_output_says_so_when_clean() {
        let text = report(&[DoctorStatus::Ok]).render_human(true);
        assert!(text.contains("no problems found"));
    }

    #[test]
    fn long_details_wrap_and_stay_within_the_line_width() {
        let long = "word ".repeat(80);
        let r = Report {
            engine: Engine::Rust,
            socket: "/s".to_string(),
            checks: vec![DoctorCheck {
                name: "a.b".to_string(),
                status: DoctorStatus::Warn,
                detail: Some(long),
            }],
        };
        let text = r.render_human(false);
        for line in text.lines() {
            assert!(line.len() <= LINE_WIDTH, "line too long: {line:?}");
        }
    }

    #[test]
    fn wrap_preserves_explicit_newlines() {
        let lines = wrap("first\nsecond", 100);
        assert_eq!(lines, ["first", "second"]);
    }
}
