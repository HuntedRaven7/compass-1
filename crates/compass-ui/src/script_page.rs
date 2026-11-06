//! Script commands: the full-output view, and the form a script's arguments
//! (or its confirmation) are entered in.
//!
//! The engine runs the script; this keeps what the view shows — the output
//! split into styled runs by the ported tokenizer, whether it is still
//! running, and how long it took.

use compass_core::script_command::ArgumentType;
use compass_core::script_output::{Color, Tokenizer};
use compass_core::script_scan::ScriptItem;

use crate::backend::{PreferenceInput, PreferenceInputKind, ScriptOutputState};
use crate::preferences_page::{FieldValue, PreferencesPage, Purpose};

/// One run of output with one style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledRun {
    /// The text.
    pub text: String,
    /// Its foreground, when the output named one.
    pub foreground: Option<Color>,
    /// Its background, when the output named one.
    pub background: Option<Color>,
    /// Whether it is a link.
    pub link: bool,
}

/// Splits raw output into styled runs: a colour set by an escape sequence
/// applies until the next one, and a reset clears it, as `tokenToHtml` spans
/// it.
#[must_use]
pub fn styled_runs(output: &str) -> Vec<StyledRun> {
    let mut tokenizer = Tokenizer::new(output);
    let mut runs: Vec<StyledRun> = Vec::new();
    let (mut foreground, mut background) = (None, None);
    while let Some(token) = tokenizer.next_token() {
        if let Some(fmt) = token.fmt {
            if fmt.reset {
                foreground = None;
                background = None;
            }
            if fmt.foreground.is_some() {
                foreground = fmt.foreground;
            }
            if fmt.background.is_some() {
                background = fmt.background;
            }
        }
        if token.text.is_empty() {
            continue;
        }
        match runs.last_mut() {
            Some(last)
                if !token.url
                    && !last.link
                    && last.foreground == foreground
                    && last.background == background =>
            {
                last.text.push_str(&token.text);
            }
            _ => runs.push(StyledRun {
                text: token.text,
                foreground,
                background,
                link: token.url,
            }),
        }
    }
    runs
}

/// Output with its escape sequences taken out: what a one-line mode shows,
/// where there is no room for colour.
#[must_use]
pub fn plain_text(output: &str) -> String {
    styled_runs(output)
        .into_iter()
        .map(|run| run.text)
        .collect()
}

/// The full-output view's state.
#[derive(Debug, Clone)]
pub struct ScriptOutputPage {
    /// The script, to run again.
    pub script_id: String,
    /// Its arguments, to run again with.
    pub arguments: Vec<String>,
    /// Its title.
    pub title: String,
    /// The run being followed.
    pub session: u64,
    /// What the engine last reported.
    pub state: ScriptOutputState,
    /// The output as styled runs.
    pub runs: Vec<StyledRun>,
    /// Why following or running failed.
    pub notice: Option<String>,
}

impl ScriptOutputPage {
    /// A view following `session`.
    #[must_use]
    pub fn new(script_id: String, arguments: Vec<String>, title: String, session: u64) -> Self {
        Self {
            script_id,
            arguments,
            title,
            session,
            state: ScriptOutputState::default(),
            runs: Vec::new(),
            notice: None,
        }
    }

    /// Takes a report for `session`; one for another run is dropped.
    /// Returns whether the run is still going (and should be asked again).
    pub fn apply(&mut self, session: u64, state: ScriptOutputState) -> bool {
        if session != self.session {
            return false;
        }
        if state.output != self.state.output {
            self.runs = styled_runs(&state.output);
        }
        self.state = state;
        !self.state.finished
    }

    /// The line above the output: running and for how long, or how it ended
    /// (`Done in %1s (exit=%2)`).
    #[must_use]
    pub fn heading(&self) -> String {
        #[allow(clippy::cast_precision_loss)]
        let seconds = self.state.elapsed_ms as f64 / 1000.0;
        if !self.state.finished {
            return if self.state.elapsed_ms < 1000 {
                "Running...".to_owned()
            } else {
                format!("Running... ({}s ago)", self.state.elapsed_ms / 1000)
            };
        }
        match self.state.exit_code {
            Some(0) => format!("Done in {seconds}s (exit=0)"),
            Some(code) => format!("Failed after {seconds}s (exit code {code})"),
            None => format!("Stopped after {seconds}s"),
        }
    }

    /// The line above the output, as the view draws it: the script, then
    /// how the run stands.
    #[must_use]
    pub fn heading_line(&self) -> String {
        format!("{} — {}", self.title, self.heading())
    }

    /// The output without its styling.
    #[must_use]
    pub fn text(&self) -> String {
        self.runs.iter().map(|run| run.text.as_str()).collect()
    }

    /// Whether the run has ended in failure: a non-zero exit, or stopped.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.state.finished && self.state.exit_code != Some(0)
    }
}

/// The form a script asks for before it runs: one field per argument, and
/// the confirmation it needs, or `None` when it needs neither.
#[must_use]
pub fn arguments_form(script: &ScriptItem) -> Option<PreferencesPage> {
    if script.arguments.is_empty() && !script.needs_confirmation {
        return None;
    }
    let fields = script
        .arguments
        .iter()
        .enumerate()
        .map(|(index, argument)| {
            let name = format!("argument{index}");
            PreferenceInput {
                title: argument
                    .placeholder
                    .clone()
                    .unwrap_or_else(|| format!("Argument {}", index + 1)),
                placeholder: argument.placeholder.clone().unwrap_or_else(|| name.clone()),
                name,
                description: String::new(),
                required: !argument.optional,
                kind: match argument.argument_type {
                    ArgumentType::Text => PreferenceInputKind::Text,
                    ArgumentType::Password => PreferenceInputKind::Password,
                    ArgumentType::Dropdown => PreferenceInputKind::Dropdown {
                        options: argument
                            .data
                            .iter()
                            .map(|option| (option.title.clone(), option.value.clone()))
                            .collect(),
                    },
                },
                value: None,
            }
        })
        .collect();
    let title = if script.needs_confirmation {
        format!(
            "{}: are you sure? This script needs confirmation to execute",
            script.title
        )
    } else {
        script.title.clone()
    };
    Some(PreferencesPage::new(
        Purpose::ScriptArguments,
        script.id.clone(),
        title,
        fields,
    ))
}

/// The argument values a script's form holds, in order.
#[must_use]
pub fn argument_values(page: &PreferencesPage) -> Vec<String> {
    page.values
        .iter()
        .map(|value| match value {
            FieldValue::Text(text) => text.clone(),
            FieldValue::Choice(choice) => choice.clone().unwrap_or_default(),
            FieldValue::Checked(_) | FieldValue::Kept(_) => String::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_core::script_command::{ArgumentDataOption, OutputMode, ScriptArgument};

    #[test]
    fn colours_last_until_a_reset_and_links_stand_alone() {
        let runs = styled_runs("plain \u{1b}[31mred https://x.test more\u{1b}[0m back");
        let summary: Vec<(&str, Option<Color>, bool)> = runs
            .iter()
            .map(|run| (run.text.as_str(), run.foreground, run.link))
            .collect();
        assert_eq!(
            summary,
            [
                ("plain ", None, false),
                ("red ", Some(Color::Red), false),
                ("https://x.test", Some(Color::Red), true),
                (" more", Some(Color::Red), false),
                (" back", None, false),
            ]
        );
    }

    #[test]
    fn the_heading_says_running_then_how_it_ended() {
        let mut page = ScriptOutputPage::new("s".into(), vec![], "S".into(), 4);
        assert!(!page.apply(
            5,
            ScriptOutputState {
                finished: true,
                ..ScriptOutputState::default()
            }
        ));
        assert_eq!(page.heading(), "Running...");
        assert!(page.apply(
            4,
            ScriptOutputState {
                output: "a".into(),
                elapsed_ms: 2500,
                ..ScriptOutputState::default()
            }
        ));
        assert_eq!(page.heading(), "Running... (2s ago)");
        assert!(!page.apply(
            4,
            ScriptOutputState {
                output: "a\n".into(),
                finished: true,
                exit_code: Some(0),
                elapsed_ms: 2500,
            }
        ));
        assert_eq!(page.heading(), "Done in 2.5s (exit=0)");
    }

    #[test]
    fn a_script_asks_for_its_arguments_or_its_confirmation() {
        let mut script = ScriptItem {
            id: "a.sh".into(),
            title: "A".into(),
            subtitle: String::new(),
            keywords: vec![],
            mode: OutputMode::Compact,
            needs_confirmation: false,
            arguments: vec![],
            path: "/x/a.sh".into(),
        };
        assert!(arguments_form(&script).is_none());
        script.needs_confirmation = true;
        let confirm = arguments_form(&script).expect("a confirmation");
        assert!(confirm.fields.is_empty());
        assert!(confirm.title.contains("needs confirmation"));
        script.arguments = vec![
            ScriptArgument {
                argument_type: ArgumentType::Text,
                placeholder: Some("Query".into()),
                optional: false,
                percent_encoded: false,
                data: None,
            },
            ScriptArgument {
                argument_type: ArgumentType::Dropdown,
                placeholder: None,
                optional: true,
                percent_encoded: false,
                data: Some(ArgumentDataOption {
                    title: "Fast".into(),
                    value: "fast".into(),
                }),
            },
        ];
        let mut form = arguments_form(&script).expect("two arguments");
        assert_eq!(form.fields[0].title, "Query");
        assert!(form.fields[0].required);
        assert_eq!(form.fields[1].title, "Argument 2");
        form.values[0] = FieldValue::Text("rust".into());
        form.values[1] = FieldValue::Choice(Some("fast".into()));
        assert_eq!(argument_values(&form), ["rust", "fast"]);
    }
}
