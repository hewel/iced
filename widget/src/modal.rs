//! Live scene blur for existing modal compositions.
use crate::core::layout::{self, Layout};
use crate::core::mouse;
use crate::core::renderer;
use crate::core::widget::{self, Tree, tree};
use crate::core::{self, Element, Event, Length, Rectangle, Shell, Size, Widget};

/// Blurs the lower scene before drawing `content`, including its dimming layer.
///
/// Compose this as a later child of `stack`. The content should include your
/// existing full-window scrim, dismissal handling, and dialog. This helper only
/// changes drawing: focus, events, operations, and overlays are delegated
/// unchanged. Toasts placed later in the stack remain crisp. Lower-scene changes
/// remain live; changing only the dialog can reuse the cached lower scene.
///
/// `sigma` is approximate Gaussian sigma in logical pixels. Non-positive values
/// and NaN are sharp; positive values, including infinity, clamp to 128.
///
/// ```no_run
/// use iced_widget::{button, center, modal, opaque, stack, text};
/// let content: iced_widget::core::Element<'_, (), iced_widget::Theme, iced_widget::Renderer> =
///     stack![text("Live background"), modal(12.0, opaque(center(button("Dialog"))))].into();
/// ```
pub fn modal<'a, Message, Theme, Renderer>(
    sigma: f32,
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: core::Renderer + 'a,
{
    Element::new(Modal {
        sigma,
        content: content.into(),
    })
}

struct Modal<'a, Message, Theme, Renderer> {
    sigma: f32,
    content: Element<'a, Message, Theme, Renderer>,
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Modal<'_, Message, Theme, Renderer>
where
    Renderer: core::Renderer,
{
    fn tag(&self) -> tree::Tag {
        self.content.as_widget().tag()
    }
    fn state(&self) -> tree::State {
        self.content.as_widget().state()
    }
    fn diff(&mut self, tree: &mut Tree) {
        self.content.as_widget_mut().diff(tree);
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn is_void(&self) -> bool {
        self.content.as_widget().is_void()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
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
        let Some(bounds) = layout.bounds().intersection(viewport) else {
            return;
        };
        renderer.with_layer(bounds, |renderer| {
            renderer.blur_backdrop(self.sigma);
            self.content
                .as_widget()
                .draw(tree, renderer, theme, style, layout, cursor, viewport);
        });
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
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
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content
            .as_widget_mut()
            .update(tree, event, layout, cursor, renderer, shell, viewport);
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
        translation: core::Vector,
    ) -> Vec<core::overlay::Element<'b, Message, Theme, Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}
