//! Run Terminal Program: the typed command line and the programs on `PATH`
//! that match it.
//!
//! The engine lists the programs and runs them; this keeps what the view
//! decides — whether the text names a program, which programs match it, and
//! the actions each row offers, in the order `compass_core::system_run`
//! gives them.

use compass_core::system_run::{self, DefaultAction, ProgramAction, RunAction};
use compass_search::{MIN_QUALITY, Query, WeightedField, score_weighted};

/// How many programs the list shows, as `m_programDb.search(str, 100)`.
pub const PROGRAM_LIMIT: usize = 100;

/// What the view is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The programs are on their way.
    Loading,
    /// They arrived.
    Ready,
    /// They cannot be listed, and why.
    Failed(String),
}

/// One row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunRow {
    /// The command line as typed, when its first word is a program.
    CommandLine(Vec<String>),
    /// A program, by path.
    Program(String),
}

/// What an action does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// Run `argv`, in a terminal (kept open when `hold`) or directly.
    Run {
        /// The command line.
        argv: Vec<String>,
        /// In a terminal.
        terminal: bool,
        /// Keep the terminal open.
        hold: bool,
    },
    /// Copy a program's path.
    CopyPath(String),
}

/// The view's state.
#[derive(Debug, Clone)]
pub struct ProgramsPage {
    /// The typed text.
    pub query: String,
    /// Every program the engine listed.
    pub programs: Vec<String>,
    /// The terminal's name, when one is installed.
    pub terminal: Option<String>,
    /// Which run action comes first.
    pub default_action: DefaultAction,
    /// The rows for the current text.
    pub rows: Vec<RunRow>,
    /// Position in `rows`.
    pub selected: usize,
    /// What the view is showing.
    pub status: Status,
    /// Why the last run did not happen.
    pub notice: Option<String>,
}

impl Default for ProgramsPage {
    fn default() -> Self {
        Self {
            query: String::new(),
            programs: Vec::new(),
            terminal: None,
            default_action: DefaultAction::default(),
            rows: Vec::new(),
            selected: 0,
            status: Status::Loading,
            notice: None,
        }
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

impl ProgramsPage {
    /// Recomputes the rows for the current text, back at the top.
    pub fn refilter(&mut self) {
        self.selected = 0;
        let text = self.query.trim();
        let argv = compass_xdg::exec::ExecParser::new("").parse(text, &[]);
        let has_program = argv.first().is_some_and(|program| {
            std::path::Path::new(program).is_file()
                || self
                    .programs
                    .iter()
                    .any(|path| file_name(path) == program.as_str())
        });
        let mut rows = Vec::new();
        if has_program {
            rows.push(RunRow::CommandLine(argv));
        }
        let query = Query::new(text);
        let mut scored: Vec<(u32, &String)> = self
            .programs
            .iter()
            .filter_map(|path| {
                if query.is_empty() {
                    return Some((0, path));
                }
                let found = score_weighted(&[WeightedField::new(path, 1.0)], &query);
                (found.quality >= MIN_QUALITY && found.score > 0).then_some((found.score, path))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        rows.extend(
            scored
                .into_iter()
                .take(PROGRAM_LIMIT)
                .map(|(_, path)| RunRow::Program(path.clone())),
        );
        self.rows = rows;
    }

    /// Takes the engine's list.
    pub fn apply(&mut self, result: Result<(Vec<String>, Option<String>, DefaultAction), String>) {
        match result {
            Ok((programs, terminal, default_action)) => {
                self.programs = programs;
                self.terminal = terminal;
                self.default_action = default_action;
                self.status = Status::Ready;
            }
            Err(reason) => self.status = Status::Failed(reason),
        }
        self.refilter();
    }

    /// The actions a row offers, titled, in order; the first is what Enter
    /// does.
    #[must_use]
    pub fn actions(&self, row: &RunRow) -> Vec<(String, Act)> {
        let terminal = self.terminal.as_deref();
        let run = |action: RunAction, argv: Vec<String>| {
            let (terminal_run, hold) = match action {
                RunAction::OpenInTerminalHold => (true, true),
                RunAction::OpenInTerminal => (true, false),
                RunAction::ExecuteProgram => (false, false),
            };
            (
                action.title(terminal.unwrap_or_default()),
                Act::Run {
                    argv,
                    terminal: terminal_run,
                    hold,
                },
            )
        };
        match row {
            RunRow::CommandLine(argv) => {
                system_run::command_line_row(argv, terminal.is_some(), self.default_action)
                    .actions
                    .into_iter()
                    .map(|action| run(action, argv.clone()))
                    .collect()
            }
            RunRow::Program(path) => {
                system_run::program_row(path, "", terminal.is_some(), self.default_action)
                    .actions
                    .into_iter()
                    .map(|action| match action {
                        ProgramAction::Run(action) => run(action, vec![path.clone()]),
                        ProgramAction::CopyExecPath => (
                            system_run::COPY_EXEC_PATH_TITLE.to_owned(),
                            Act::CopyPath(path.clone()),
                        ),
                    })
                    .collect()
            }
        }
    }

    /// The selected row, if any.
    #[must_use]
    pub fn selected_row(&self) -> Option<&RunRow> {
        self.rows.get(self.selected)
    }

    /// What the view says in place of the list: loading, why it failed, or
    /// that nothing matches; `None` while it shows rows.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&str> {
        match &self.status {
            Status::Loading => Some("Looking for programs…"),
            Status::Failed(reason) => Some(reason),
            Status::Ready if self.rows.is_empty() => Some("No programs match"),
            Status::Ready => None,
        }
    }
}

/// A row's title and second line (`~` for the home directory).
#[must_use]
pub fn row_text(row: &RunRow, home: &str) -> (String, Option<String>) {
    match row {
        RunRow::CommandLine(argv) => (argv.join(" "), None),
        RunRow::Program(path) => (
            file_name(path).to_owned(),
            Some(if home.is_empty() {
                path.clone()
            } else {
                system_run::compress_path(path, home)
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(terminal: Option<&str>, default_action: DefaultAction) -> ProgramsPage {
        let mut page = ProgramsPage::default();
        page.apply(Ok((
            vec![
                "/usr/bin/htop".into(),
                "/usr/bin/top".into(),
                "/bin/ls".into(),
            ],
            terminal.map(str::to_owned),
            default_action,
        )));
        page
    }

    #[test]
    fn a_typed_program_is_a_command_line_row_above_the_matches() {
        let mut page = page(Some("Ptyxis"), DefaultAction::RunInTerminal);
        assert_eq!(page.rows.len(), 3, "an empty query lists every program");
        page.query = "htop -d 5".into();
        page.refilter();
        assert_eq!(
            page.rows.first(),
            Some(&RunRow::CommandLine(vec![
                "htop".into(),
                "-d".into(),
                "5".into()
            ]))
        );
        page.query = "nothere --x".into();
        page.refilter();
        assert!(
            !matches!(page.rows.first(), Some(RunRow::CommandLine(_))),
            "not a program, so no command-line row"
        );
    }

    #[test]
    fn the_default_action_comes_first_and_without_a_terminal_only_copy_is_left() {
        let page_ = page(Some("Ptyxis"), DefaultAction::Run);
        let actions = page_.actions(&RunRow::Program("/usr/bin/top".into()));
        let titles: Vec<&str> = actions.iter().map(|(title, _)| title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Execute program",
                "Open in Ptyxis",
                "Open in Ptyxis (hold)",
                "Copy exec path"
            ]
        );
        assert_eq!(
            actions[0].1,
            Act::Run {
                argv: vec!["/usr/bin/top".into()],
                terminal: false,
                hold: false
            }
        );
        let bare = page(None, DefaultAction::Run);
        assert_eq!(
            bare.actions(&RunRow::Program("/usr/bin/top".into()))
                .into_iter()
                .map(|(title, _)| title)
                .collect::<Vec<_>>(),
            ["Copy exec path"]
        );
        assert!(
            bare.actions(&RunRow::CommandLine(vec!["top".into()]))
                .is_empty()
        );
    }

    #[test]
    fn a_program_row_shows_its_folder_with_home_folded() {
        assert_eq!(
            row_text(&RunRow::Program("/home/me/bin/tool".into()), "/home/me"),
            ("tool".to_owned(), Some("~/bin/tool".to_owned()))
        );
    }
}
