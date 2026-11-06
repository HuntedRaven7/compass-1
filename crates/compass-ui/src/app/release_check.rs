//! The root search's Update section (`RootUpdateSection`): a newer Compass
//! release, above everything else for the empty query, with its release notes
//! and "Skip this version". Whether there is one is the engine's
//! (`compass::updates`); Compass checks and never installs, so the row's
//! primary action opens the release page where the C++'s installs.

use super::{LauncherApp, Message, Page, PanelSection, PanelState, RootRow, Task, focus_search};
use crate::action_panel::Action;
use crate::backend::UpdateOffer;

/// The section's heading.
pub(super) const UPDATE_HEADING: &str = "Update";

/// The row's accessory.
const ACCESSORY: &str = "Update";

const RELEASE_NOTES: &str = "update.release-notes";
const SKIP: &str = "update.skip";

/// The row's title: `Vicinae %1 is available`, for Compass.
#[must_use]
pub fn title(offer: &UpdateOffer) -> String {
    format!("Compass {} is available", offer.tag)
}

/// The row's subtitle.
#[must_use]
pub fn subtitle(offer: &UpdateOffer) -> String {
    format!("You are running {}", offer.current)
}

impl LauncherApp {
    /// Asks the engine whether a newer release is out.
    pub(super) fn refresh_update_task(&self) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.update_status().await },
            Message::UpdateStatusLoaded,
        )
    }

    /// Puts the Update row first for the empty query at the root, as
    /// `RootSearchModel` sets the section only when nothing is typed.
    pub(super) fn apply_update(&mut self) {
        let had = self.results.first() == Some(&RootRow::Update);
        self.results.retain(|row| *row != RootRow::Update);
        if self.update.is_some()
            && self.query.is_empty()
            && self.provider_scope.is_none()
            && matches!(self.page, Page::Root)
        {
            self.results.insert(0, RootRow::Update);
            if !had && self.selected != 0 {
                self.selected += 1;
            }
        } else if had {
            self.selected = self.selected.saturating_sub(1);
        }
    }

    /// Whether the Update row leads the list.
    pub(super) fn update_shown(&self) -> bool {
        self.results.first() == Some(&RootRow::Update)
    }

    /// Handles [`Message::UpdateStatusLoaded`] and [`Message::UpdateSkipped`].
    pub(super) fn release_check_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::UpdateStatusLoaded(Ok(offer)) => {
                self.update = offer;
                self.apply_update();
                Task::none()
            }
            Message::UpdateStatusLoaded(Err(reason)) => {
                tracing::debug!(%reason, "no update status");
                Task::none()
            }
            Message::UpdateSkipped(tag, Ok(())) => {
                if self.update.as_ref().is_some_and(|offer| offer.tag == tag) {
                    self.update = None;
                }
                // The row going is the answer: the launcher has no toast
                // that leaves it open, and a HUD would close it.
                self.apply_update();
                focus_search()
            }
            Message::UpdateSkipped(_, Err(reason)) => {
                self.error = Some(reason);
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The Update row's primary action: its release notes, in the browser.
    pub(super) fn open_release_notes(&mut self) -> Task<Message> {
        let Some(offer) = &self.update else {
            return Task::none();
        };
        self.open_link(offer.release_url.clone())
    }

    /// "Skip this version": the engine remembers the tag and the row goes.
    fn skip_update(&mut self) -> Task<Message> {
        let (Some(offer), Some(backend)) = (&self.update, self.backend.clone()) else {
            return Task::none();
        };
        let tag = offer.tag.clone();
        Task::perform(
            {
                let tag = tag.clone();
                async move { backend.skip_update(tag).await }
            },
            move |result| Message::UpdateSkipped(tag.clone(), result),
        )
    }

    /// The Update row's panel (`RootUpdateSection::actionPanel`).
    pub(super) fn open_update_panel(&mut self) -> Option<Task<Message>> {
        if !matches!(self.page, Page::Root) || self.selected_row() != Some(RootRow::Update) {
            return None;
        }
        self.update.as_ref()?;
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions: vec![
                Action::new("View release notes")
                    .with_id(RELEASE_NOTES)
                    .with_shortcut("enter"),
                Action::new("Skip this version")
                    .with_id(SKIP)
                    .with_shortcut("ctrl+x"),
            ],
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs an Update panel action, if `id` is one.
    pub(super) fn update_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        let task = match id {
            RELEASE_NOTES => self.open_release_notes(),
            SKIP => self.skip_update(),
            _ => return None,
        };
        self.panel = None;
        Some(task)
    }

    /// The Update row, drawn.
    pub(super) fn update_row(&self, selected: bool) -> Option<iced::Element<'_, Message>> {
        let offer = self.update.as_ref()?;
        let glyph = self
            .icons
            .then(|| crate::icons::update_glyph(self.palette().accent));
        let title = title(offer);
        Some(self.list_row_with(
            self.glyph_or_initial(glyph.as_ref(), &title, selected),
            title,
            Some(subtitle(offer)),
            Some(ACCESSORY.to_owned()),
            selected,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::release_check;
    use crate::app::test_support::{TestBackend, app, settle};
    use std::sync::Arc;

    #[test]
    fn a_newer_release_leads_the_empty_query_and_its_panel_opens_or_skips_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let backend = Arc::new(TestBackend::default());
        let url = "https://github.com/tuna-os/compass/releases/tag/v0.2.0";
        *backend.update.lock().unwrap() = Some(crate::backend::UpdateOffer {
            tag: "v0.2.0".into(),
            version: "0.2.0".into(),
            release_url: url.into(),
            current: "v0.1.0".into(),
        });
        let mut app = app(dir.path());
        app.backend = Some(backend.clone());

        // What the window asks each time it opens.
        let task = app.refresh_update_task();
        settle(&mut app, task);
        assert_eq!(app.results.first(), Some(&RootRow::Update));
        assert_eq!(app.root_heading_at(0), Some("Update"));
        assert!(
            app.state_line()
                .contains("selected_title=\"Compass v0.2.0 is available\""),
            "{}",
            app.state_line()
        );
        let offer = app.update.as_ref().expect("held");
        assert_eq!(release_check::subtitle(offer), "You are running v0.1.0");

        // Only for the empty query.
        let task = app.update(Message::QueryChanged("fire".into()));
        settle(&mut app, task);
        assert!(!app.results.contains(&RootRow::Update));
        let task = app.update(Message::QueryChanged(String::new()));
        settle(&mut app, task);
        assert_eq!(app.results.first(), Some(&RootRow::Update));

        // Enter reads the release notes.
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        assert_eq!(backend.opened_urls.lock().unwrap().as_slice(), [url]);

        // The panel skips it, and the row goes.
        let _ = app.update(Message::TogglePanel);
        let skip = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Skip this version"))
            .expect("the panel offers to skip");
        assert!(
            app.panel
                .as_ref()
                .and_then(|panel| panel.row_titled("View release notes"))
                .is_some()
        );
        let task = app.update(Message::PanelClicked(skip));
        settle(&mut app, task);
        assert_eq!(backend.skipped.lock().unwrap().as_slice(), ["v0.2.0"]);
        assert!(app.update.is_none());
        assert!(!app.results.contains(&RootRow::Update));
        assert_eq!(app.root_heading_at(0), None);
    }
}
