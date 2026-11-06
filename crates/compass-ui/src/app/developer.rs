//! Create Extension in the launcher: the form, then the success page.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the form and the page's text are in
//! [`crate::developer_page`].

use iced::keyboard::{Key, key::Named};

use super::a11y::PageSemantics;
use super::{
    Element, LauncherApp, Length, Message, Padding, Page, Task, column, container, scrollable,
};
use crate::developer_page::{self, CreatedPage};
use crate::preferences_page::Purpose;
use crate::semantics::{Block, Body, Document, markdown_blocks};

impl LauncherApp {
    /// Opens the Create Extension form.
    pub(super) fn open_create_extension(&mut self) -> Task<Message> {
        self.page = Page::Preferences(Box::new(developer_page::form()));
        Task::none()
    }

    /// Submits the Create Extension form. `None` when the form showing is
    /// not that one.
    pub(super) fn submit_create_extension(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(page) = &mut self.page else {
            return None;
        };
        if page.purpose != Purpose::CreateExtension {
            return None;
        }
        if let Err(missing) = page.submission() {
            page.notice = Some(format!("Form has errors: fill in {}", missing.join(", ")));
            return Some(Task::none());
        }
        let draft = developer_page::draft(page);
        let Some(backend) = self.backend.clone() else {
            page.notice = Some("Create Extension needs the Compass engine".to_owned());
            return Some(Task::none());
        };
        let title = draft.title.clone();
        Some(Task::perform(
            async move { backend.create_extension(draft).await },
            move |result| Message::ExtensionCreated {
                title: title.clone(),
                result,
            },
        ))
    }

    /// Handles the Create Extension messages.
    pub(super) fn developer_message(&mut self, message: Message) -> Task<Message> {
        match message {
            // `popSelf()` then the success view: the form is replaced.
            Message::ExtensionCreated {
                title,
                result: Ok(path),
            } => {
                self.page = Page::Created(CreatedPage::new(&title, path));
                Task::none()
            }
            Message::ExtensionCreated {
                result: Err(reason),
                ..
            } => {
                if let Page::Preferences(page) = &mut self.page {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            Message::CreatedFolderOpened(Ok(())) => self.conceal(),
            Message::CreatedFolderOpened(Err(reason)) => {
                if let Page::Created(page) = &mut self.page {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The success page's keys: Enter opens the new extension's folder, as
    /// the C++'s "Open in …" actions do; Escape goes back.
    pub(super) fn created_page_key(&mut self, key: &Key) -> Task<Message> {
        let Page::Created(page) = &self.page else {
            return Task::none();
        };
        if let Some(scroll) = crate::scroll::reading_key(key.as_ref()) {
            return scroll;
        }
        match key.as_ref() {
            Key::Named(Named::Escape) => self.update(Message::Back),
            Key::Named(Named::Enter) => {
                let (Some(backend), path) = (self.backend.clone(), page.path.clone()) else {
                    return Task::none();
                };
                Task::perform(
                    async move { backend.open_file(path, false).await },
                    Message::CreatedFolderOpened,
                )
            }
            _ => Task::none(),
        }
    }

    /// The success page's body.
    pub(super) fn created_body<'a>(&'a self, page: &'a CreatedPage) -> Element<'a, Message> {
        let markdown = iced::widget::markdown::view(&page.markdown, self.markdown_settings())
            .map(Message::ExtensionLinkClicked);
        let mut body = column![
            markdown,
            iced::widget::text(developer_page::CREATED_HINT)
                .font(self.font())
                .size(12)
        ]
        .spacing(10);
        if let Some(notice) = &page.notice {
            body = body.push(self.notice(notice));
        }
        scrollable(container(body).padding(Padding::new(14.0)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink)
            .into()
    }

    /// The success page for assistive technology: its text, then its keys.
    pub(super) fn created_semantics(&self, page: &CreatedPage) -> PageSemantics {
        let mut blocks = markdown_blocks(&page.source);
        blocks.push(Block {
            text: developer_page::CREATED_HINT.to_owned(),
            heading: None,
        });
        PageSemantics {
            body: Body::Document(Document {
                label: "Extension created".to_owned(),
                blocks,
                metadata: Vec::new(),
            }),
            status: page.notice.clone(),
            detail: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_success_page_is_read_as_its_text_then_its_keys() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let mut page = CreatedPage::new("Hello", "/home/me/code/hello".into());
        page.notice = Some("No application can open the folder".into());
        let semantics = app.created_semantics(&page);
        let Body::Document(document) = &semantics.body else {
            panic!("expected a document, got {:?}", semantics.body);
        };
        assert_eq!(
            document.blocks.first(),
            Some(&Block {
                text: "Extension successfully created".to_owned(),
                heading: Some(1),
            })
        );
        assert!(
            document
                .blocks
                .iter()
                .any(|block| block.text == "npm run dev"),
            "the code a line at a time: {:?}",
            document.blocks
        );
        assert_eq!(
            document.blocks.last().map(|block| block.text.as_str()),
            Some(developer_page::CREATED_HINT)
        );
        assert_eq!(
            semantics.status.as_deref(),
            Some("No application can open the folder")
        );
        app.page = Page::Created(page);
        assert!(matches!(app.semantics().body, Body::Document(_)));
    }
}
