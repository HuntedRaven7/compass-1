//! View rendering for the launcher.

use iced::{
    Alignment, Element, Length, Padding,
    widget::{column, container, row, text, Space},
};

use compass_core::AppItem;

/// Render the search results list.
pub fn render_results(results: &[AppItem], selected: usize) -> Element<'_, crate::message::Message> {
    let mut col = column![].spacing(4).padding(Padding::new(10).horizontal(20));

    for (index, item) in results.iter().enumerate() {
        let is_selected = index == selected;
        col = col.push(render_item(item, is_selected));
    }

    container(scrollable(col).height(Length::Fill))
        .width(Length::Fill)
        .padding(Padding::new(0).horizontal(20))
        .into()
}

/// Render a single result item.
pub fn render_item(item: &AppItem, selected: bool) -> Element<'_, crate::message::Message> {
    let name = text(item.name()).size(16);
    let description = item.generic_name().or(item.comment()).map(|d| text(d).size(12).style(text::secondary()));

    let content = if let Some(desc) = description {
        column![name, desc].spacing(2)
    } else {
        column![name]
    };

    let item_content = container(content)
        .padding(Padding::new(8).horizontal(12))
        .width(Length::Fill)
        .style(if selected {
            container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb8(0x3d, 0x4a, 0x6d))),
                border: iced::Border {
                    color: iced::Color::TRANSPARENT,
                    width: 0.0,
                    radius: 6.0.into(),
                },
                ..Default::default()
            }
        } else {
            container::Style::default()
        });

    container(item_content)
        .width(Length::Fill)
        .into()
}