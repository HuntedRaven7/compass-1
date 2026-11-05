//! A widget that says what it shows.
//!
//! Iced 0.14's `rich_text` takes part in no widget operation, so the Markdown
//! an extension's detail and a store README are drawn with is invisible to a
//! find by text: a headless test of either view cannot see a word of it. This
//! wraps an element, leaves its layout, drawing and input untouched, and
//! reports `label` as its text, as [`crate::elided`] does for a cut line.

use iced::{Element, Length, Rectangle, Size, Vector};
use iced_core::layout::{self, Layout};
use iced_core::widget::{Operation, Tree, Widget, tree};
use iced_core::{Clipboard, Event, Shell, mouse, overlay, renderer};

/// `content`, reporting `label` as its text to widget operations.
pub fn labelled<'a, Message: 'a, Theme: 'a, Renderer: iced_core::Renderer + 'a>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    label: String,
) -> Element<'a, Message, Theme, Renderer> {
    Element::new(Labelled {
        content: content.into(),
        label,
    })
}

struct Labelled<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    label: String,
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Labelled<'_, Message, Theme, Renderer>
where
    Renderer: iced_core::Renderer,
{
    fn tag(&self) -> tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut Tree) {
        self.content.as_widget().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.text(None, layout.bounds(), &self.label);
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}
