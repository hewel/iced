//! Blur the live scene behind foreground content.
use crate::core::border;
use crate::core::layout::{self, Layout};
use crate::core::mouse;
use crate::core::renderer;
use crate::core::widget::{self, Meta, Tree, tree};
use crate::core::{self, Background, Blur, Event, Length, Rectangle, Shell, Size, Widget};

/// Blurs the previously drawn scene behind `content`.
///
/// Place this after the background in a [`crate::stack`]. The background can
/// contain scrolling widgets, images, video, or custom shader output. Every
/// rendered frame uses the current lower scene. Foreground content is drawn
/// after the blur and tint and remains crisp, as do later stack children.
///
/// The effect takes its size and position from `content`. A progressive [`Blur`]
/// follows those full bounds, so clipping or scrolling the effect does not
/// restart its transition. Radii are approximate Gaussian sigma in logical
/// pixels. Sampling can include pixels outside the rounded region; only the
/// region itself is replaced. Use the image widget's `blur` method for
/// image-source blur.
///
/// Layout, events, focus, operations, and overlays are delegated to `content`.
/// Background animations and video producers must request their own redraws;
/// this wrapper does not keep an otherwise idle application rendering.
///
/// ```no_run
/// use iced_widget::{backdrop, container, stack, text, Widget};
/// use iced_widget::core::{Blur, Color};
/// let content: iced_widget::core::Element<'_, (), iced_widget::Theme, iced_widget::Renderer> =
///     stack![
///         text("Live background"),
///         backdrop(
///             Blur::vertical_gradient(0.0, 20.0),
///             container(text("Crisp foreground")).padding(24),
///         )
///         .border_radius(16)
///         .tint(Color::from_rgba(0.1, 0.1, 0.1, 0.2)),
///     ].boxed();
/// ```
pub fn backdrop<W>(blur: impl Into<Blur>, content: W) -> Backdrop<W> {
    Backdrop::new(blur, content)
}

/// Foreground content with a blurred region of the live scene behind it.
///
/// See [`backdrop()`] for composition and profile coordinates.
pub struct Backdrop<W> {
    blur: Blur,
    content: W,
    border_radius: border::Radius,
    border_smoothing: f32,
    tint: Option<Background>,
}

impl<W> Backdrop<W> {
    /// Creates a live backdrop behind `content`, with a rectangular mask.
    pub fn new(blur: impl Into<Blur>, content: W) -> Self {
        Self {
            blur: blur.into(),
            content,
            border_radius: border::Radius::default(),
            border_smoothing: 0.0,
            tint: None,
        }
    }

    /// Sets the corner radii shared by the backdrop and its tint.
    pub fn border_radius(mut self, radius: impl Into<border::Radius>) -> Self {
        self.border_radius = radius.into();
        self
    }

    /// Sets corner smoothing for the backdrop and its tint, from `0.0` to `1.0`.
    ///
    /// The default is `0.0`, producing circular corners.
    pub fn border_smoothing(mut self, smoothing: f32) -> Self {
        self.border_smoothing = smoothing;
        self
    }

    /// Paints a color or gradient over the blur, before drawing `content`.
    ///
    /// The tint uses the same bounds and rounded mask as the backdrop.
    pub fn tint(mut self, tint: impl Into<Background>) -> Self {
        self.tint = Some(tint.into());
        self
    }
}

impl<W: Meta> Meta for Backdrop<W> {
    fn is_void(&self) -> bool {
        self.content.is_void()
    }
}

impl<Message, Theme, Renderer, W> Widget<Message, Theme, Renderer> for Backdrop<W>
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
        let bounds = layout.bounds();
        let Some(visible) = bounds.intersection(viewport) else {
            return;
        };
        renderer.with_layer(visible, |renderer| {
            renderer.draw_backdrop(renderer::Backdrop {
                bounds,
                blur: self.blur,
                border_radius: self.border_radius,
                border_smoothing: self.border_smoothing,
            });
            if let Some(tint) = self.tint {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds,
                        border: core::Border {
                            radius: self.border_radius,
                            smoothing: self.border_smoothing,
                            ..Default::default()
                        },
                        snap: false,
                        ..Default::default()
                    },
                    tint,
                );
            }
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
