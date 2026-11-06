//! Drawing a file's preview beside a list, and a view's footer.
//!
//! A child module of `app` so it can use the launcher's palette and font;
//! reading the file is [`crate::file_preview`].

use super::{
    Alignment, Element, LauncherApp, Length, Message, Padding, Space, column, container, image,
    row, scrollable, text,
};
use crate::file_preview::{Content, FilePreview};

/// How wide the preview is against the list: the list takes two parts, the
/// pane three, as the QML detail split leaves the preview the larger side.
pub(super) const LIST_PORTION: u16 = 2;
pub(super) const PANE_PORTION: u16 = 3;

/// A footer's height: one 12 px line and its padding.
pub(super) const FOOTER_HEIGHT: f32 = 30.0;

impl LauncherApp {
    /// The preview pane: the file drawn or quoted, then its metadata unless
    /// `metadata` is off (dmenu's `--no-metadata`).
    pub(super) fn file_preview_pane<'a>(
        &'a self,
        preview: &'a FilePreview,
        metadata: bool,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let drawn: Element<'a, Message> = match &preview.content {
            Content::Image(path) => container(
                image(path.clone())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .content_fit(iced::ContentFit::Contain),
            )
            .height(Length::Fill)
            .padding(8)
            .into(),
            Content::Text(body) => scrollable(
                container(
                    text(body.as_str())
                        .font(iced::Font::MONOSPACE)
                        .size(12)
                        .color(palette.text.to_iced()),
                )
                .padding(8),
            )
            .height(Length::Fill)
            .into(),
            Content::None => container(
                text(preview.mime.as_str())
                    .font(self.font())
                    .size(12)
                    .color(palette.muted.to_iced()),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .into(),
        };
        let mut pane = column![drawn].spacing(6);
        if metadata {
            let mut rows = column![].spacing(4);
            for (label, value) in preview.fields() {
                rows = rows.push(
                    row![
                        text(label)
                            .font(self.font())
                            .size(12)
                            .color(palette.muted.to_iced()),
                        Space::new().width(Length::Fill),
                        text(value)
                            .font(self.font())
                            .size(12)
                            .color(palette.text.to_iced()),
                    ]
                    .spacing(12),
                );
            }
            pane = pane.push(container(rows).padding(Padding::new(8.0)).style(
                move |_: &iced::Theme| container::Style {
                    border: iced::Border {
                        color: palette.border.to_iced(),
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..container::Style::default()
                },
            ));
        }
        container(pane)
            .width(Length::FillPortion(PANE_PORTION))
            .height(Length::Fixed(320.0))
            .padding(Padding::new(6.0).top(8))
            .into()
    }

    /// A view's footer: its navigation title at the left, the primary
    /// action and the panel's chord at the right, as the C++ status bar.
    pub(super) fn footer<'a>(
        &'a self,
        title: Option<&'a str>,
        primary: Option<&'a str>,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let label = |value: &'a str, strong: bool| {
            text(value).font(self.font()).size(12).color(if strong {
                palette.text.to_iced()
            } else {
                palette.muted.to_iced()
            })
        };
        let mut line = row![
            label(title.unwrap_or(""), true),
            Space::new().width(Length::Fill)
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if let Some(primary) = primary {
            line = line.push(label(primary, true)).push(label("↵", false));
        }
        let line = line
            .push(label("Actions", true))
            .push(label("Ctrl+B", false));
        container(line)
            .width(Length::Fill)
            .height(Length::Fixed(FOOTER_HEIGHT))
            .align_y(Alignment::Center)
            .padding(Padding::new(0.0).left(14).right(14))
            .into()
    }
}
