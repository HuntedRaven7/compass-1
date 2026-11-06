//! Create Extension: the form, and the page that follows it.
//!
//! The rules and the generator are the engine's (`compass_core::
//! create_extension`, `boilerplate`); this is the form as
//! `CreateExtensionViewHost` lays it out, and the success page's text.

use crate::backend::{ExtensionDraft, PreferenceInput, PreferenceInputKind};
use crate::preferences_page::{FieldValue, PreferencesPage, Purpose};

/// The field names, which the submission is read back by.
const FIELDS: [(&str, &str, &str); 6] = [
    ("author", "Author", "Your name or handle"),
    ("title", "Extension title", "My Extension"),
    (
        "description",
        "Description",
        "What the extension does, in a sentence",
    ),
    ("location", "Location", "~/code"),
    ("command_title", "Command title", "Search Things"),
    (
        "command_description",
        "Command description",
        "What the first command does",
    ),
];

/// The template field's name.
const TEMPLATE: &str = "template";

/// The Create Extension form, as `CreateExtensionViewHost` lays it out: the
/// extension's fields, where to create it, the first command, and its
/// template (the first one chosen).
#[must_use]
pub fn form() -> PreferencesPage {
    let mut fields: Vec<PreferenceInput> = FIELDS
        .iter()
        .map(|(name, title, placeholder)| PreferenceInput {
            name: (*name).to_owned(),
            title: (*title).to_owned(),
            description: String::new(),
            placeholder: (*placeholder).to_owned(),
            required: true,
            kind: PreferenceInputKind::Text,
            value: None,
        })
        .collect();
    let options: Vec<(String, String)> = compass_core::boilerplate::COMMAND_BOILERPLATES
        .iter()
        .map(|template| (template.name.to_owned(), template.resource.to_owned()))
        .collect();
    fields.push(PreferenceInput {
        name: TEMPLATE.to_owned(),
        title: "Command template".to_owned(),
        description: String::new(),
        placeholder: String::new(),
        required: true,
        value: options
            .first()
            .map(|(_, resource)| serde_json::Value::String(resource.clone())),
        kind: PreferenceInputKind::Dropdown { options },
    });
    PreferencesPage::new(
        Purpose::CreateExtension,
        String::new(),
        "Create Extension".to_owned(),
        fields,
    )
}

/// What the form holds, as the engine takes it.
#[must_use]
pub fn draft(page: &PreferencesPage) -> ExtensionDraft {
    let value = |name: &str| {
        page.fields
            .iter()
            .zip(&page.values)
            .find(|(field, _)| field.name == name)
            .map(|(_, value)| match value {
                FieldValue::Text(text) => text.trim().to_owned(),
                FieldValue::Choice(choice) => choice.clone().unwrap_or_default(),
                FieldValue::Checked(_) | FieldValue::Kept(_) => String::new(),
            })
            .unwrap_or_default()
    };
    ExtensionDraft {
        author: value("author"),
        title: value("title"),
        description: value("description"),
        location: value("location"),
        command_title: value("command_title"),
        command_description: value("command_description"),
        template: value(TEMPLATE),
    }
}

/// The success page's text: `CreateExtensionSuccessViewHost`'s Markdown.
#[must_use]
pub fn success_markdown(title: &str, path: &str) -> String {
    format!(
        "# Extension successfully created\n\n\
         Your new extension {title} has been successfully created at `{path}`.\n\n\
         For commands from this extension to be picked up, you need to run your extension in \
         development mode at least once:\n\n\
         ```bash\ncd '{path}'\nnpm install\nnpm run dev\n```\n\n\
         Compass runs extensions built with the Vicinae SDK. You can learn more about \
         extension development in the \
         [Vicinae SDK documentation](https://docs.vicinae.com/extensions/introduction).\n"
    )
}

/// The keys under the success page's text, as the card shows them.
pub const CREATED_HINT: &str = "Enter: open the folder    Esc: back";

/// The page after a successful creation.
#[derive(Debug, Clone)]
pub struct CreatedPage {
    /// Where the extension was written.
    pub path: String,
    /// The Markdown, as written, for assistive technology to read.
    pub source: String,
    /// The Markdown, parsed once.
    pub markdown: Vec<iced::widget::markdown::Item>,
    /// Why opening the folder did not happen.
    pub notice: Option<String>,
}

impl CreatedPage {
    /// The page for an extension called `title` written to `path`.
    #[must_use]
    pub fn new(title: &str, path: String) -> Self {
        let source = success_markdown(title, &path);
        let markdown = iced::widget::markdown::parse(&source).collect();
        Self {
            path,
            source,
            markdown,
            notice: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_reads_back_as_the_draft_with_the_first_template_chosen() {
        let mut page = form();
        assert_eq!(page.fields.len(), 7);
        page.values[0] = FieldValue::Text(" zoe ".into());
        page.values[3] = FieldValue::Text("~/code".into());
        let draft = draft(&page);
        assert_eq!(draft.author, "zoe");
        assert_eq!(draft.location, "~/code");
        assert_eq!(draft.template, ":boilerplate/tmpl-list");
        assert!(
            page.submission().is_err(),
            "every field is required before the engine is asked"
        );
    }

    #[test]
    fn the_success_page_says_where_and_what_next() {
        let text = success_markdown("Hello", "/home/me/code/hello");
        assert!(text.contains("`/home/me/code/hello`"));
        assert!(text.contains("npm run dev"));
    }
}
