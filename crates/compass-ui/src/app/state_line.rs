//! The launcher's one-line account of what it shows, logged after every
//! message; see [`LauncherApp::state_line`] for why it exists.

use super::{AppItem, LauncherApp, Page, RootRow, release_check};

impl LauncherApp {
    /// One line saying what the launcher is showing, for a log.
    ///
    /// # Why this exists, and why it is a function rather than a `tracing!`
    ///
    /// The VM tier asserts what the launcher did by counting changed pixels.
    /// That is the only way to prove something reached the screen, and it is
    /// terrible at everything else: "5.44% of pixels in a box changed" cannot
    /// say *which* row is selected, what the query matched, or what the panel
    /// contains, and it moves with the font, the theme and the card geometry.
    /// Two of this tier's runs were spent on a containment box that had gone
    /// stale, asserting nothing about the launcher at all.
    ///
    /// So the state machine says what it did, in a line a grep can check
    /// exactly, and the pixels go back to proving only the thing they are
    /// uniquely good for: that a window was drawn.
    ///
    /// It is a pure function returning a `String` rather than a `tracing`
    /// macro at the call site so that its content is unit-testable. A log line
    /// asserted only inside a 25-minute VM run is a log line nobody can
    /// control.
    ///
    /// The shape is `key=value`, space separated, with titles quoted, because
    /// that is what survives being grepped out of a log interleaved with
    /// wgpu's debug output.
    #[must_use]
    pub fn state_line(&self) -> String {
        let mut line = format!(
            "query={:?} results={} selected={}",
            self.query,
            self.results.len(),
            self.selected
        );
        // The title, not just the index: an index is only meaningful against a
        // corpus the reader cannot see, and "selected=0" is equally true of a
        // right and a wrong first row.
        match self.selected_row() {
            Some(RootRow::App(_)) => {
                let name = self.selected_item().map_or("", AppItem::name);
                line.push_str(&format!(" selected_title={name:?}"));
            }
            Some(RootRow::Command(command)) => {
                line.push_str(&format!(" selected_title={:?}", command.title));
            }
            Some(RootRow::Extension(index)) => {
                let title = self
                    .app_index
                    .extensions()
                    .get(index)
                    .map_or("", |command| command.title.as_str());
                line.push_str(&format!(" selected_title={title:?}"));
            }
            Some(RootRow::Calculator) => {
                let answer = self.calculator.as_ref().map_or("", |a| a.answer.as_str());
                line.push_str(&format!(" selected_title={answer:?}"));
            }
            Some(RootRow::Update) => {
                let title = self
                    .update
                    .as_ref()
                    .map(release_check::title)
                    .unwrap_or_default();
                line.push_str(&format!(" selected_title={title:?}"));
            }
            Some(RootRow::Shortcut(index)) => {
                let title = self
                    .app_index
                    .shortcuts()
                    .get(index)
                    .map_or("", crate::shortcuts_page::display_name);
                line.push_str(&format!(" selected_title={title:?}"));
            }
            Some(RootRow::Script(index)) => {
                let title = self
                    .app_index
                    .scripts()
                    .get(index)
                    .map_or("", |script| script.title.as_str());
                line.push_str(&format!(" selected_title={title:?}"));
            }
            Some(RootRow::RhaiScript(index)) => {
                let title = self
                    .app_index
                    .rhai_scripts()
                    .get(index)
                    .map_or("", |script| script.title.as_str());
                line.push_str(&format!(" selected_title={title:?}"));
            }
            Some(RootRow::Fallback(fallback)) => {
                let title = self.fallback_title(fallback).unwrap_or_default();
                line.push_str(&format!(" selected_title={title:?} fallback"));
            }
            None => line.push_str(" selected_title=none"),
        }
        if let Page::Themes(page) = &self.page {
            line.push_str(&format!(
                " page=themes themes_query={:?} themes_rows={} themes_selected={} theme={}",
                page.query,
                page.rows.len(),
                page.selected,
                self.view.theme_choice.name()
            ));
        }
        if let Page::Dmenu(page) = &self.page {
            line.push_str(&format!(
                " page=dmenu dmenu_token={} dmenu_query={:?} dmenu_shown={} dmenu_selected={}",
                page.token,
                page.query,
                page.shown.len(),
                page.selected
            ));
        }
        if let Page::Programs(page) = &self.page {
            line.push_str(&format!(
                " page=programs programs_query={:?} programs_rows={} programs_selected={}",
                page.query,
                page.rows.len(),
                page.selected
            ));
        }
        if let Page::ScriptOutput(page) = &self.page {
            line.push_str(&format!(
                " page=script_output script_session={} script_finished={} script_exit={:?}",
                page.session, page.state.finished, page.state.exit_code
            ));
        }
        if let Page::Snippets(page) = &self.page {
            line.push_str(&format!(
                " page=snippets snippets_query={:?} snippets_shown={} snippets_selected={}",
                page.query,
                page.shown.len(),
                page.selected
            ));
        }
        if let Page::Shortcuts(page) = &self.page {
            line.push_str(&format!(
                " page=shortcuts shortcuts_query={:?} shortcuts_shown={} shortcuts_selected={}",
                page.query,
                page.shown.len(),
                page.selected
            ));
        }
        if let Page::Windows(page) = &self.page {
            line.push_str(&format!(
                " page=windows windows_query={:?} windows_shown={} windows_selected={}",
                page.query,
                page.shown.len(),
                page.selected
            ));
        }
        if let Page::Workspaces(page) = &self.page {
            line.push_str(&format!(
                " page=workspaces workspaces_query={:?} workspaces_shown={} workspaces_selected={}",
                page.query,
                page.shown.len(),
                page.selected
            ));
        }
        if let Page::Clipboard(page) = &self.page {
            line.push_str(&format!(
                " page=clipboard clipboard_query={:?} clipboard_rows={} clipboard_selected={}",
                page.query,
                page.rows.len(),
                page.selected
            ));
        }
        if let Page::Files(page) = &self.page {
            line.push_str(&format!(
                " page=files files_query={:?} files_heading={:?} files_rows={} files_selected={}",
                page.query,
                page.heading,
                page.rows.len(),
                page.selected
            ));
        }
        match &self.panel {
            None => line.push_str(" panel=closed"),
            Some(panel) => {
                line.push_str(&format!(
                    " panel=open panel_filter={:?} panel_rows={} panel_selected={}",
                    panel.filter,
                    panel.rows.len(),
                    panel.selected
                ));
                let title = usize::try_from(panel.selected)
                    .ok()
                    .and_then(|index| panel.rows.get(index))
                    .and_then(|row| {
                        let action = row.action?;
                        Some(
                            panel
                                .sections
                                .get(row.section)?
                                .actions
                                .get(action)?
                                .title
                                .as_str(),
                        )
                    });
                match title {
                    Some(title) => line.push_str(&format!(" panel_title={title:?}")),
                    None => line.push_str(" panel_title=none"),
                }
            }
        }
        if let Some(error) = &self.error {
            line.push_str(&format!(" error={error:?}"));
        }
        line.push_str(if self.window.is_some() {
            " window=open"
        } else {
            " window=hidden"
        });
        line
    }
}

#[cfg(test)]
mod tests {
    use crate::app::test_support::app;
    use crate::message::{Direction, Message};

    /// The log every test in this binary writes into, installed once.
    ///
    /// A thread-local `with_default` subscriber was the obvious way to do this
    /// and does not work here: `tracing` keeps a PROCESS-GLOBAL max-level
    /// hint, so with tests running in parallel a `debug!` can be filtered out
    /// while a thread-local DEBUG subscriber is active. The test passed alone
    /// and failed in the suite -- 38/38 pass under `--test-threads=1`, which
    /// is how that was pinned down rather than guessed.
    ///
    /// Installing one global subscriber at DEBUG raises that hint for the
    /// whole run and keeps it raised, so there is no race to lose.
    fn log() -> &'static std::sync::Mutex<Vec<u8>> {
        use std::sync::{Mutex, OnceLock};
        use tracing_subscriber::fmt::MakeWriter;

        static LOG: OnceLock<&'static Mutex<Vec<u8>>> = OnceLock::new();

        #[derive(Clone, Copy)]
        struct Writer(&'static Mutex<Vec<u8>>);

        impl std::io::Write for Writer {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        impl<'a> MakeWriter<'a> for Writer {
            type Writer = Self;
            fn make_writer(&'a self) -> Self::Writer {
                *self
            }
        }

        LOG.get_or_init(|| {
            let buffer: &'static Mutex<Vec<u8>> = Box::leak(Box::new(Mutex::new(Vec::new())));
            let subscriber = tracing_subscriber::fmt()
                .with_max_level(tracing::Level::DEBUG)
                .with_writer(Writer(buffer))
                .without_time()
                .finish();
            // Another test binary in the same process may have got here first;
            // either way a DEBUG subscriber is installed, which is what this
            // needs.
            let _ = tracing::subscriber::set_global_default(subscriber);
            buffer
        })
    }

    fn logged() -> String {
        let bytes = log()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn every_message_leaves_its_state_in_the_log() {
        // A query no other test uses, so the lines below are this test's even
        // though every test in the binary shares one log. Counting lines that
        // merely say `compass_ui::state` would race with whatever else is
        // running.
        const QUERY: &str = "compass-log-state-sentinel";

        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("log-state.desktop"),
            format!("[Desktop Entry]\nType=Application\nName={QUERY}\nExec=true\n"),
        )
        .expect("log-state fixture");
        let _ = log();
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged(QUERY.to_owned()));
        let _ = app.update(Message::TogglePanel);

        let log = logged();
        let mine: Vec<&str> = log
            .lines()
            .filter(|line| line.contains(&format!(r#"query="{QUERY}""#)))
            .collect();

        // If `update` stops emitting, this fails here in milliseconds instead
        // of in a 25-minute VM run against a line the launcher no longer
        // writes. A control confirmed the other state-line tests do NOT catch
        // that: deleting the `tracing::debug!` left all of them green.
        assert_eq!(
            mine.len(),
            2,
            "one line per message, no more and no fewer:\n{log}"
        );
        assert!(mine[0].contains("compass_ui::state"), "{:?}", mine[0]);
        assert!(mine[0].contains("panel=closed"), "{:?}", mine[0]);
        assert!(mine[1].contains("panel=open"), "{:?}", mine[1]);
    }

    #[test]
    fn the_state_line_names_what_is_on_screen() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());

        let _ = app.update(Message::QueryChanged("fir".to_owned()));
        let line = app.state_line();
        // Everything the VM tier wants to assert, in one greppable line.
        assert!(line.contains(r#"query="fir""#), "{line}");
        assert!(line.contains("results=1"), "{line}");
        assert!(line.contains("selected=0"), "{line}");
        assert!(line.contains(r#"selected_title="Firefox""#), "{line}");
        assert!(line.contains("panel=closed"), "{line}");
    }

    #[test]
    fn the_state_line_follows_the_selection_rather_than_the_index_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());

        // A query matching more than one row, so moving down has somewhere to
        // go. Not the empty query: the launcher shows nothing for that, which
        // this test discovered. The title is what makes "selected=1" mean
        // anything.
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        let first = app.state_line();
        let _ = app.update(Message::MoveSelection(Direction::Down));
        let second = app.state_line();

        assert!(first.contains("selected=0"), "{first}");
        assert!(second.contains("selected=1"), "{second}");
        assert_ne!(
            first
                .split_once("selected_title=")
                .map(|(_, rest)| rest.to_owned()),
            second
                .split_once("selected_title=")
                .map(|(_, rest)| rest.to_owned()),
            "the title must move with the selection, not just the number"
        );
    }

    #[test]
    fn the_state_line_describes_the_panel_when_it_is_open() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fir".to_owned()));

        assert!(app.state_line().contains("panel=closed"));

        let _ = app.update(Message::TogglePanel);
        let line = app.state_line();
        assert!(line.contains("panel=open"), "{line}");
        assert!(line.contains("panel_selected=0"), "{line}");
        // The first SELECTABLE row, which is the thing the VM frame had to be
        // read by eye to confirm. Now it is a string.
        assert!(line.contains(r#"panel_title="Open""#), "{line}");

        let _ = app.update(Message::PanelFilterChanged("copy".to_owned()));
        let filtered = app.state_line();
        assert!(filtered.contains(r#"panel_filter="copy""#), "{filtered}");
        assert!(filtered.contains(r#"panel_title="Copy"#), "{filtered}");

        let _ = app.update(Message::TogglePanel);
        assert!(app.state_line().contains("panel=closed"));
    }

    #[test]
    fn the_state_line_says_when_nothing_matched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("zzzz".to_owned()));
        let line = app.state_line();
        // "results=0" and "no window" are different failures and the tier has
        // to be able to tell them apart from a log.
        assert!(line.contains("results=0"), "{line}");
        assert!(line.contains("selected_title=none"), "{line}");
    }
}
