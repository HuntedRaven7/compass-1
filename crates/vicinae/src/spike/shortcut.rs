//! Spike A: does the GlobalShortcuts portal actually give us Super+Space?
//!
//! [`compass_portals::shortcuts`] can already ask. What it structurally cannot
//! tell us, with no session to talk to, is what a real GNOME says back:
//!
//! 1. whether binding is **permitted** at all, or refused before the user sees
//!    anything;
//! 2. whether the trigger we are granted is the one we **asked for** — the
//!    portal documents `preferred_trigger` as a preference, and GNOME may
//!    assign something else or nothing;
//! 3. whether pressing the key actually **reaches us** as an activation.
//!
//! The third is the one no amount of reading answers, and it is why this runs
//! under a VM whose emulated keyboard can be driven from outside the guest
//! (ADR-0010). A hotkey synthesised inside the session under test would prove
//! nothing; a scancode at QEMU's keyboard is indistinguishable from a person
//! pressing the key.

use std::time::{Duration, Instant};

use serde::Serialize;

use compass_portals::{
    PortalConfig, Portals, ShortcutDescriptor, ShortcutEvent, ShortcutsOutcome, Trigger,
};

/// Written to stderr the moment the spike is ready to receive a keypress.
///
/// The harness watches for this line and only then injects the key. Anything
/// time-based would race: binding involves a portal round trip and, on a first
/// run, a permission dialog, and under llvmpipe those take an unpredictable
/// while (ADR-0010: key off states, never durations).
///
/// It means "I am as ready as I will ever be", NOT "everything worked". That
/// distinction is load-bearing and was got wrong: the marker was printed only
/// on the path where binding succeeded, so a spike that could not reach the
/// portal wrote its report, exited, and left the harness waiting 150 seconds
/// for a line that was never coming — and then discarding the answer it
/// already had. On a failure path the answer is simply final sooner.
pub const READY_MARKER: &str = "SPIKE-A-READY";

/// Announces readiness exactly once, whatever path the spike takes out.
#[derive(Debug, Default)]
struct ReadyOnce(bool);

impl ReadyOnce {
    /// Prints the marker unless it has already been printed. Returns whether
    /// this call was the one that printed it.
    fn announce(&mut self) -> bool {
        if self.0 {
            return false;
        }
        self.0 = true;
        eprintln!("{READY_MARKER}");
        true
    }
}

/// What to ask the desktop for.
#[derive(Debug, Clone)]
pub struct ShortcutSpike {
    /// Stable id the portal echoes back on activation.
    pub id: String,
    /// Description the desktop shows in its shortcut settings.
    pub description: String,
    /// Preferred trigger, in this project's syntax. `None` asks for no
    /// preference, which is a different and also interesting answer.
    pub preferred_trigger: Option<String>,
    /// How long to wait for an activation after binding.
    pub wait: Duration,
}

/// What the desktop said.
///
/// Every field is evidence rather than a verdict: this is a spike, and the
/// point is to record what happened accurately enough that a design decision
/// can be made from it later without re-running anything.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ShortcutSpikeReport {
    /// How the GlobalShortcuts interface looked before we tried anything.
    pub portal: String,
    /// Interface version, when a session was created.
    pub portal_version: Option<u32>,
    /// The trigger we asked for, echoed as the portal received it.
    pub requested_trigger: Option<String>,
    /// `granted`, `denied`, `refused`, or an error string.
    pub bind_outcome: String,
    /// What the desktop says it bound.
    pub bound: Vec<BoundReport>,
    /// Whether the granted trigger looks like the one requested.
    ///
    /// `None` when there was nothing to compare. See
    /// [`triggers_look_equivalent`] for why this is a heuristic and not an
    /// equality test.
    pub trigger_matches_request: Option<bool>,
    /// Whether an activation arrived within the wait.
    pub activated: bool,
    /// The id the activation carried, which should be the one we registered.
    pub activated_id: Option<String>,
    /// Whether the compositor supplied an `xdg-activation-v1` token.
    ///
    /// `xdg-desktop-portal-gnome` only started delivering this reliably in
    /// 50.alpha, so `Some(false)` is a finding rather than a failure.
    pub activation_token_present: Option<bool>,
    /// How long we actually waited.
    pub waited_seconds: f64,
    /// Anything a reader should know that is not one of the fields above.
    pub notes: Vec<String>,
}

/// One shortcut as the desktop reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoundReport {
    /// The id we asked for.
    pub id: String,
    /// Description as the portal echoes it.
    pub description: String,
    /// How the desktop says the user triggers it. May be empty.
    pub trigger_description: String,
}

impl ShortcutSpikeReport {
    fn failed(portal: String, note: impl Into<String>) -> Self {
        Self {
            portal,
            portal_version: None,
            requested_trigger: None,
            bind_outcome: "not attempted".to_owned(),
            bound: Vec::new(),
            trigger_matches_request: None,
            activated: false,
            activated_id: None,
            activation_token_present: None,
            waited_seconds: 0.0,
            notes: vec![note.into()],
        }
    }

    /// Renders the report for a person reading a CI log.
    #[must_use]
    pub fn render_human(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();

        let _ = writeln!(out, "GlobalShortcuts portal: {}", self.portal);
        if let Some(version) = self.portal_version {
            let _ = writeln!(out, "  interface version:    {version}");
        }
        let _ = writeln!(
            out,
            "  requested trigger:    {}",
            self.requested_trigger
                .as_deref()
                .unwrap_or("(no preference)")
        );
        let _ = writeln!(out, "  bind:                 {}", self.bind_outcome);

        for shortcut in &self.bound {
            let trigger = if shortcut.trigger_description.is_empty() {
                "(the desktop reported no trigger)"
            } else {
                &shortcut.trigger_description
            };
            let _ = writeln!(out, "  bound {}: {trigger}", shortcut.id);
        }
        match self.trigger_matches_request {
            Some(true) => {
                let _ = writeln!(out, "  granted what we asked for: yes");
            }
            Some(false) => {
                let _ = writeln!(
                    out,
                    "  granted what we asked for: NO — this is the answer Spike A exists for"
                );
            }
            None => {}
        }

        let _ = writeln!(
            out,
            "  activation:           {} (waited {:.1}s)",
            if self.activated {
                "received"
            } else {
                "never arrived"
            },
            self.waited_seconds
        );
        if let Some(token) = self.activation_token_present {
            let _ = writeln!(
                out,
                "  activation token:     {}",
                if token { "present" } else { "absent" }
            );
        }
        for note in &self.notes {
            let _ = writeln!(out, "  note: {note}");
        }
        out
    }
}

/// Whether a granted trigger looks like the one that was requested.
///
/// This is deliberately a heuristic, and saying so matters more than the
/// answer. `preferred_trigger` goes to the portal in the wire syntax
/// (`SUPER+space`), while `trigger_description` comes back as *display* text
/// chosen by the desktop for its settings UI — GNOME says `Super+Space`, and
/// nothing in the specification requires the two to be relatable at all.
///
/// So this normalises case and separators and compares what is left. It can be
/// fooled; the verbatim strings are both in the report precisely so a reader
/// never has to trust it.
#[must_use]
pub fn triggers_look_equivalent(requested: &str, granted: &str) -> bool {
    fn normalise(value: &str) -> Vec<String> {
        let mut parts: Vec<String> = value
            .split(['+', '-', ' '])
            .filter(|part| !part.is_empty())
            .map(|part| match part.to_ascii_lowercase().as_str() {
                // The names the two sides are most likely to disagree on.
                "meta" | "super" | "logo" | "win" => "super".to_owned(),
                "spc" | "space" => "space".to_owned(),
                "control" => "ctrl".to_owned(),
                other => other.to_owned(),
            })
            .collect();
        // A modifier set is unordered; "Super+Space" and "Space+Super" name the
        // same chord.
        parts.sort();
        parts
    }
    !requested.is_empty() && !granted.is_empty() && normalise(requested) == normalise(granted)
}

/// Runs Spike A against whatever desktop this process can reach.
///
/// Never returns an error: every failure is a finding and belongs in the
/// report. A spike that exits non-zero because the portal was missing would
/// tell a reader less than one that says so in a field.
pub async fn global_shortcut(spike: &ShortcutSpike) -> ShortcutSpikeReport {
    let mut ready = ReadyOnce::default();
    let portals = match Portals::connect(PortalConfig::default()).await {
        Ok(portals) => portals,
        Err(err) => {
            ready.announce();
            return ShortcutSpikeReport::failed(
                "unknown".to_owned(),
                format!("could not reach the session bus: {err}"),
            );
        }
    };

    let availability = portals.probe().await.global_shortcuts;
    let portal = availability.to_string();

    let session = match portals.global_shortcuts().await {
        Ok(session) => session,
        Err(err) => {
            ready.announce();
            return ShortcutSpikeReport::failed(portal, format!("no session: {err}"));
        }
    };

    let portal_version = Some(session.version());

    // Subscribe before binding: an activation that races the bind would
    // otherwise be dropped, and "the key did nothing" is exactly the wrong
    // conclusion to reach by accident.
    let mut events = session.subscribe();

    let mut notes = Vec::new();
    let mut descriptor = ShortcutDescriptor::new(spike.id.clone(), spike.description.clone());
    let mut requested_trigger = None;
    if let Some(text) = &spike.preferred_trigger {
        match Trigger::parse(text) {
            Ok(trigger) => {
                requested_trigger = Some(trigger.to_string());
                descriptor = descriptor.with_trigger(trigger);
            }
            Err(err) => notes.push(format!(
                "could not parse the preferred trigger `{text}` ({err}); asked for no preference"
            )),
        }
    }

    let outcome = session.bind(std::slice::from_ref(&descriptor)).await;
    let (bind_outcome, bound) = match &outcome {
        Ok(ShortcutsOutcome::Granted { shortcuts }) => (
            "granted".to_owned(),
            shortcuts
                .iter()
                .map(|s| BoundReport {
                    id: s.id.clone(),
                    description: s.description.clone(),
                    trigger_description: s.trigger_description.clone(),
                })
                .collect::<Vec<_>>(),
        ),
        Ok(ShortcutsOutcome::Denied) => ("denied".to_owned(), Vec::new()),
        Ok(ShortcutsOutcome::Refused) => ("refused".to_owned(), Vec::new()),
        Err(err) => (format!("error: {err}"), Vec::new()),
        // ShortcutsOutcome is #[non_exhaustive]. A new variant is a new answer
        // from the desktop, which is exactly what a spike wants to hear about,
        // so it is recorded rather than folded into one of the arms above.
        Ok(other) => (format!("unrecognised outcome: {other:?}"), Vec::new()),
    };

    let trigger_matches_request = match (&requested_trigger, bound.first()) {
        (Some(requested), Some(first)) if !first.trigger_description.is_empty() => Some(
            triggers_look_equivalent(requested, &first.trigger_description),
        ),
        (Some(_), Some(_)) => {
            notes.push(
                "the desktop bound the shortcut but reported no trigger for it, so there is \
                 nothing to compare — on GNOME this means the user has not assigned one"
                    .to_owned(),
            );
            None
        }
        _ => None,
    };

    // Only now is a keypress meaningful. The harness is watching stderr for
    // this and will not send the key before it appears.
    ready.announce();

    let started = Instant::now();
    let mut activated = false;
    let mut activated_id = None;
    let mut activation_token_present = None;

    let deadline = tokio::time::sleep(spike.wait);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            () = &mut deadline => break,
            event = events.recv() => match event {
                Some(ShortcutEvent::Activated { id, activation_token, .. }) => {
                    activated = true;
                    activated_id = Some(id);
                    activation_token_present = Some(activation_token.is_some());
                    break;
                }
                Some(ShortcutEvent::Changed { shortcuts }) => notes.push(format!(
                    "the desktop changed our shortcuts while we waited: {}",
                    shortcuts
                        .iter()
                        .map(|s| format!("{}={}", s.id, s.trigger_description))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                // Deactivated without Activated would be strange, and worth
                // recording rather than ignoring.
                Some(ShortcutEvent::Deactivated { id, .. }) => {
                    notes.push(format!("a Deactivated arrived for `{id}` with no Activated before it"));
                }
                None => {
                    notes.push("the session closed while waiting for an activation".to_owned());
                    break;
                }
                // ShortcutEvent is #[non_exhaustive]; see the outcome match.
                Some(other) => notes.push(format!("unrecognised event while waiting: {other:?}")),
            },
        }
    }
    let waited_seconds = started.elapsed().as_secs_f64();

    if let Err(err) = session.close().await {
        notes.push(format!("closing the session failed: {err}"));
    }

    ShortcutSpikeReport {
        portal,
        portal_version,
        requested_trigger,
        bind_outcome,
        bound,
        trigger_matches_request,
        activated,
        activated_id,
        activation_token_present,
        waited_seconds,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_spellings_of_the_same_chord_are_equivalent() {
        // The whole point: we send the wire syntax, GNOME answers with display
        // text, and a naive string compare would report every success as a
        // mismatch.
        assert!(triggers_look_equivalent("SUPER+space", "Super+Space"));
        assert!(triggers_look_equivalent("SUPER+space", "Meta+Space"));
        assert!(triggers_look_equivalent("CTRL+ALT+t", "Control+Alt+T"));
    }

    #[test]
    fn a_modifier_set_is_unordered() {
        assert!(triggers_look_equivalent("CTRL+SHIFT+k", "Shift+Ctrl+K"));
    }

    #[test]
    fn a_different_chord_is_not_equivalent() {
        // The finding Spike A is looking for: asked for one thing, given
        // another. This must not be smoothed over by the normalisation.
        assert!(!triggers_look_equivalent("SUPER+space", "Super+D"));
        assert!(!triggers_look_equivalent("SUPER+space", "Ctrl+Space"));
        assert!(!triggers_look_equivalent(
            "SUPER+space",
            "Super+Shift+Space"
        ));
    }

    #[test]
    fn an_empty_side_is_never_equivalent() {
        // GNOME reports an empty trigger_description when the user has not
        // assigned one. Treating that as a match would turn "not bound to
        // anything" into a pass.
        assert!(!triggers_look_equivalent("SUPER+space", ""));
        assert!(!triggers_look_equivalent("", "Super+Space"));
        assert!(!triggers_look_equivalent("", ""));
    }

    #[test]
    fn readiness_is_announced_once_and_only_once() {
        // The harness treats the marker as "go", so a second one after a
        // keypress has already been sent would be a lie about the state. And
        // the first must happen on every path out, including the failures —
        // a spike that exits without it leaves the harness waiting for a line
        // that is never coming, which is exactly what happened.
        let mut ready = ReadyOnce::default();
        assert!(ready.announce(), "the first call should announce");
        assert!(!ready.announce(), "the second call should not");
        assert!(!ready.announce());
    }

    #[test]
    fn a_failed_report_still_says_what_it_knows() {
        let report = ShortcutSpikeReport::failed("unavailable".to_owned(), "no bus");
        assert_eq!(report.bind_outcome, "not attempted");
        assert!(!report.activated);
        assert_eq!(report.notes, ["no bus"]);

        let rendered = report.render_human();
        assert!(rendered.contains("unavailable"), "{rendered}");
        assert!(rendered.contains("no bus"), "{rendered}");
        assert!(rendered.contains("never arrived"), "{rendered}");
    }

    #[test]
    fn a_mismatch_is_stated_plainly_rather_than_buried() {
        let report = ShortcutSpikeReport {
            portal: "available (version 1)".to_owned(),
            portal_version: Some(1),
            requested_trigger: Some("SUPER+space".to_owned()),
            bind_outcome: "granted".to_owned(),
            bound: vec![BoundReport {
                id: "compass.toggle".to_owned(),
                description: "Toggle compass".to_owned(),
                trigger_description: "Super+D".to_owned(),
            }],
            trigger_matches_request: Some(false),
            activated: false,
            activated_id: None,
            activation_token_present: None,
            waited_seconds: 30.0,
            notes: Vec::new(),
        };
        let rendered = report.render_human();
        assert!(rendered.contains("Super+D"), "{rendered}");
        assert!(rendered.contains("NO"), "{rendered}");
    }

    #[test]
    fn an_empty_trigger_description_is_rendered_as_words_not_as_nothing() {
        let report = ShortcutSpikeReport {
            portal: "available".to_owned(),
            portal_version: None,
            requested_trigger: None,
            bind_outcome: "granted".to_owned(),
            bound: vec![BoundReport {
                id: "compass.toggle".to_owned(),
                description: "Toggle compass".to_owned(),
                trigger_description: String::new(),
            }],
            trigger_matches_request: None,
            activated: true,
            activated_id: Some("compass.toggle".to_owned()),
            activation_token_present: Some(false),
            waited_seconds: 1.5,
            notes: Vec::new(),
        };
        let rendered = report.render_human();
        assert!(
            rendered.contains("the desktop reported no trigger"),
            "{rendered}"
        );
        assert!(rendered.contains("received"), "{rendered}");
        assert!(rendered.contains("absent"), "{rendered}");
    }

    #[test]
    fn the_report_serialises_with_the_field_names_the_harness_asserts_on() {
        // The VM job greps this JSON. A rename here that is not matched there
        // turns a real finding into a silent pass, so the names are pinned.
        let report = ShortcutSpikeReport::failed("unavailable".to_owned(), "no bus");
        let value: serde_json::Value = serde_json::to_value(&report).expect("serialises");
        for field in [
            "portal",
            "requested_trigger",
            "bind_outcome",
            "bound",
            "trigger_matches_request",
            "activated",
            "activation_token_present",
            "waited_seconds",
            "notes",
        ] {
            assert!(value.get(field).is_some(), "missing `{field}` in {value}");
        }
    }
}
