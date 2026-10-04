//! Live scene blur for existing modal compositions.
use crate::core::layout::{self, Layout};
use crate::core::mouse;
use crate::core::renderer;
use crate::core::widget::{self, Meta, Tree, tree};
use crate::core::{self, Event, Length, Rectangle, Shell, Size, Widget};

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
/// use iced_widget::{button, center, modal, opaque, stack, text, Widget};
/// let content: iced_widget::core::Element<'_, (), iced_widget::Theme, iced_widget::Renderer> =
///     stack![text("Live background"), modal(12.0, opaque(center(button("Dialog"))))].boxed();
/// ```
pub fn modal<Message, Theme, Renderer>(
    sigma: f32,
    content: impl Widget<Message, Theme, Renderer>,
) -> impl Widget<Message, Theme, Renderer>
where
    Renderer: core::Renderer,
{
    Modal { sigma, content }
}

struct Modal<W> {
    sigma: f32,
    content: W,
}

impl<W: Meta> Meta for Modal<W> {
    fn is_void(&self) -> bool {
        self.content.is_void()
    }
}

impl<Message, Theme, Renderer, W> Widget<Message, Theme, Renderer> for Modal<W>
where
    Renderer: core::Renderer,
    W: Widget<Message, Theme, Renderer>,
{
    fn tag(&self) -> tree::Tag {
        self.content.tag()
    }
    fn state(&self) -> tree::State {
        self.content.state()
    }
    fn diff(&mut self, tree: &mut Tree) {
        self.content.diff(tree);
    }
    fn size(&self) -> Size<Length> {
        self.content.size()
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        self.content.layout(tree, renderer, limits);
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let Some(bounds) = layout.bounds().intersection(viewport) else {
            return;
        };
        renderer.with_layer(bounds, |renderer| {
            renderer.blur_backdrop(self.sigma);
            self.content
                .draw(tree, renderer, theme, style, layout, cursor, viewport);
        });
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .operate(tree, layout, viewport, renderer, operation);
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content
            .update(tree, event, layout, cursor, renderer, shell, viewport);
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: core::Vector,
        window: Size,
    ) -> Vec<core::overlay::Element<'b, Message, Theme, Renderer>> {
        self.content
            .overlay(tree, layout, renderer, viewport, translation, window)
    }
}
