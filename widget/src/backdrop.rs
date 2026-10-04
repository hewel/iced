//! Blur the live scene behind foreground content without changing its behavior.
use crate::core::border;
use crate::core::layout::{self, Layout};
use crate::core::mouse;
use crate::core::renderer;
use crate::core::widget::{self, Tree, tree};
use crate::core::{self, Background, Blur, Element, Event, Length, Rectangle, Shell, Size, Widget};

/// Blurs the scene drawn before `content`, then draws `content` sharply.
///
/// Place this after the background in a [`crate::stack`]. The background may
/// contain a live video primitive. Every rendered frame samples the lower scene
/// before this widget's tint and foreground, so later text and controls stay
/// crisp. The wrapper does not request frames or animate; background producers
/// and the application own redraws and visibility transitions.
///
/// The effect follows the content's full layout bounds. Clipping does not
/// restart a progressive [`Blur`] profile. Blur radii are approximate Gaussian
/// sigma in logical pixels. Layout, input, focus operations, and overlays are
/// delegated unchanged. [`enabled(false)`](Backdrop::enabled) draws only the child.
///
/// ```no_run
/// use iced_widget::{backdrop, container, stack, text};
/// use iced_widget::core::{Blur, Color};
/// let content: iced_widget::core::Element<'_, (), iced_widget::Theme, iced_widget::Renderer> =
///     stack![
///         text("Live background"),
///         backdrop(
///             Blur::vertical_gradient(0.0, 20.0),
///             container(text("Crisp controls")).padding(24),
///         )
///         .border_radius(16)
///         .tint(Color::from_rgba(0.1, 0.1, 0.1, 0.2)),
///     ].into();
/// ```
pub fn backdrop<'a, Message, Theme, Renderer>(
    blur: impl Into<Blur>,
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Backdrop<'a, Message, Theme, Renderer>
where
    Renderer: core::Renderer,
{
    Backdrop::new(blur, content)
}

/// Foreground content with a blurred region of the live scene behind it.
///
/// See [`backdrop()`] for draw order and coordinate contracts.
pub struct Backdrop<'a, Message, Theme = crate::Theme, Renderer = crate::Renderer> {
    blur: Blur,
    content: Element<'a, Message, Theme, Renderer>,
    border_radius: border::Radius,
    border_smoothing: f32,
    tint: Option<Background>,
    enabled: bool,
}

impl<'a, Message, Theme, Renderer> Backdrop<'a, Message, Theme, Renderer>
where
    Renderer: core::Renderer,
{
    /// Creates an enabled live backdrop with a rectangular mask.
    pub fn new(
        blur: impl Into<Blur>,
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        Self {
            blur: blur.into(),
            content: content.into(),
            border_radius: border::Radius::default(),
            border_smoothing: 0.0,
            tint: None,
            enabled: true,
        }
    }

    /// Sets the corner radii shared by the backdrop and its tint.
    pub fn border_radius(mut self, radius: impl Into<border::Radius>) -> Self {
        self.border_radius = radius.into();
        self
    }

    /// Sets corner smoothing, from `0.0` to `1.0`. The default is circular corners.
    pub fn border_smoothing(mut self, smoothing: f32) -> Self {
        self.border_smoothing = smoothing;
        self
    }

    /// Paints a color or gradient over the blur, before drawing the foreground.
    /// The tint uses the same full bounds and rounded mask as the backdrop.
    pub fn tint(mut self, tint: impl Into<Background>) -> Self {
        self.tint = Some(tint.into());
        self
    }

    /// Enables the background effect. Defaults to `true`.
    ///
    /// When disabled, no blur marker, tint, or extra drawing layer is emitted.
    /// The child retains its layout, state, events, focus, and overlays. The
    /// application decides separately whether to show or hide that content.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Backdrop<'_, Message, Theme, Renderer>
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
        if !self.enabled {
            self.content
                .as_widget()
                .draw(tree, renderer, theme, style, layout, cursor, viewport);
            return;
        }
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

impl<'a, Message, Theme, Renderer> From<Backdrop<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: core::Renderer + 'a,
{
    fn from(backdrop: Backdrop<'a, Message, Theme, Renderer>) -> Self {
        Element::new(backdrop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core;
    use core::Renderer as _;
    use core::widget::operation::Focusable;
    use core::{Color, Point, Transformation, Vector, image, overlay, shell, window};

    #[derive(Debug, PartialEq)]
    enum Command {
        Clip(Rectangle),
        Backdrop(renderer::Backdrop),
        Quad(renderer::Quad, Background),
        EndClip,
    }

    #[derive(Default)]
    struct Recorder(Vec<Command>);

    impl core::Renderer for Recorder {
        fn blur_backdrop(&mut self, _radius: f32) {
            panic!("the bounded effect must use draw_backdrop");
        }

        fn draw_backdrop(&mut self, backdrop: renderer::Backdrop) {
            self.0.push(Command::Backdrop(backdrop));
        }

        fn start_layer(&mut self, bounds: Rectangle) {
            self.0.push(Command::Clip(bounds));
        }

        fn end_layer(&mut self) {
            self.0.push(Command::EndClip);
        }

        fn start_transformation(&mut self, _transformation: Transformation) {}

        fn end_transformation(&mut self) {}

        fn fill_quad(&mut self, quad: renderer::Quad, background: impl Into<Background>) {
            self.0.push(Command::Quad(quad, background.into()));
        }

        fn allocate_image(
            &self,
            _handle: &image::Handle,
            _callback: impl FnOnce(Result<image::Allocation, image::Error>) + Send + 'static,
        ) {
            panic!("the fixture does not allocate images");
        }

        fn hint(&mut self, _scale: renderer::Scale) {}

        fn scale(&self) -> Option<renderer::Scale> {
            None
        }

        fn reset(&mut self, _new_bounds: Rectangle) {
            self.0.clear();
        }

        fn settings(&self) -> renderer::Settings {
            renderer::Settings::default()
        }
    }

    struct Child;

    #[derive(Default)]
    struct State {
        updates: usize,
        focused: bool,
    }

    impl Focusable for State {
        fn is_focused(&self) -> bool {
            self.focused
        }

        fn focus(&mut self) {
            self.focused = true;
        }

        fn unfocus(&mut self) {
            self.focused = false;
        }
    }

    impl Widget<(), (), Recorder> for Child {
        fn size(&self) -> Size<Length> {
            Size::new(Length::Fixed(80.0), Length::Fixed(60.0))
        }

        fn tag(&self) -> tree::Tag {
            tree::Tag::of::<State>()
        }

        fn state(&self) -> tree::State {
            tree::State::new(State::default())
        }

        fn layout(
            &mut self,
            _tree: &mut Tree,
            _renderer: &Recorder,
            _limits: &layout::Limits,
        ) -> layout::Node {
            layout::Node::new(Size::new(80.0, 60.0))
        }

        fn draw(
            &self,
            _tree: &Tree,
            renderer: &mut Recorder,
            _theme: &(),
            _style: &renderer::Style,
            layout: Layout<'_>,
            _cursor: mouse::Cursor,
            _viewport: &Rectangle,
        ) {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: layout.bounds(),
                    ..Default::default()
                },
                Color::BLACK,
            );
        }

        fn update(
            &mut self,
            tree: &mut Tree,
            _event: &Event,
            _layout: Layout<'_>,
            _cursor: mouse::Cursor,
            _renderer: &Recorder,
            shell: &mut Shell<'_, ()>,
            _viewport: &Rectangle,
        ) {
            tree.state.downcast_mut::<State>().updates += 1;
            shell.publish(());
            shell.capture_event();
        }

        fn operate(
            &mut self,
            tree: &mut Tree,
            layout: Layout<'_>,
            _renderer: &Recorder,
            operation: &mut dyn widget::Operation,
        ) {
            operation.focusable(
                Some(&widget::Id::new("child")),
                layout.bounds(),
                tree.state.downcast_mut::<State>(),
            );
        }

        fn overlay<'b>(
            &'b mut self,
            tree: &'b mut Tree,
            layout: Layout<'b>,
            _renderer: &Recorder,
            _viewport: &Rectangle,
            translation: Vector,
        ) -> Vec<overlay::Element<'b, (), (), Recorder>> {
            vec![overlay::Element::new(Box::new(Popup {
                position: layout.position() + translation,
                state: tree.state.downcast_mut::<State>(),
            }))]
        }
    }

    struct Popup<'a> {
        position: Point,
        state: &'a mut State,
    }

    impl core::Overlay<(), (), Recorder> for Popup<'_> {
        fn layout(&mut self, _renderer: &Recorder, _bounds: Size) -> layout::Node {
            layout::Node::new(Size::new(20.0, 10.0)).move_to(self.position)
        }

        fn draw(
            &self,
            _renderer: &mut Recorder,
            _theme: &(),
            _style: &renderer::Style,
            _layout: Layout<'_>,
            _cursor: mouse::Cursor,
        ) {
        }

        fn update(
            &mut self,
            _event: &Event,
            _layout: Layout<'_>,
            _cursor: mouse::Cursor,
            _renderer: &Recorder,
            shell: &mut Shell<'_, ()>,
        ) {
            self.state.updates += 1;
            shell.publish(());
            shell.capture_event();
        }
    }

    #[test]
    fn clipped_profile_and_tint_precede_foreground_and_disabled_emits_only_foreground() {
        let blur = Blur::vertical_gradient(0.0, 20.0);
        let tint = Color::from_rgba(0.1, 0.2, 0.3, 0.4);
        let mut backdrop = backdrop(blur, Element::new(Child))
            .border_radius(12.0)
            .border_smoothing(0.7)
            .tint(tint);
        let mut tree = Tree::new(&backdrop as &dyn Widget<(), (), Recorder>);
        let mut renderer = Recorder::default();
        let node = backdrop
            .layout(
                &mut tree,
                &renderer,
                &layout::Limits::new(Size::ZERO, Size::new(200.0, 200.0)),
            )
            .move_to(Point::new(10.0, 20.0));
        let bounds = node.bounds();
        let viewport = Rectangle::new(Point::new(30.0, 40.0), Size::new(100.0, 100.0));
        let foreground = Command::Quad(
            renderer::Quad {
                bounds,
                ..Default::default()
            },
            Color::BLACK.into(),
        );
        backdrop.draw(
            &tree,
            &mut renderer,
            &(),
            &renderer::Style::default(),
            Layout::new(&node),
            mouse::Cursor::Unavailable,
            &viewport,
        );
        assert_eq!(
            renderer.0,
            [
                Command::Clip(bounds.intersection(&viewport).unwrap()),
                Command::Backdrop(renderer::Backdrop {
                    bounds,
                    blur,
                    border_radius: 12.0.into(),
                    border_smoothing: 0.7,
                }),
                Command::Quad(
                    renderer::Quad {
                        bounds,
                        border: core::Border {
                            radius: 12.0.into(),
                            smoothing: 0.7,
                            ..Default::default()
                        },
                        snap: false,
                        ..Default::default()
                    },
                    tint.into(),
                ),
                foreground,
                Command::EndClip,
            ]
        );

        renderer.0.clear();
        backdrop.enabled(false).draw(
            &tree,
            &mut renderer,
            &(),
            &renderer::Style::default(),
            Layout::new(&node),
            mouse::Cursor::Unavailable,
            &viewport,
        );
        assert_eq!(
            renderer.0,
            [Command::Quad(
                renderer::Quad {
                    bounds,
                    ..Default::default()
                },
                Color::BLACK.into(),
            )]
        );
    }

    #[test]
    fn toggling_effect_preserves_child_state_focus_events_and_translated_overlay() {
        let renderer = Recorder::default();
        let mut tree = Tree::empty();
        let viewport = Rectangle::with_size(Size::new(200.0, 200.0));
        let event = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let mut bus = shell::Bus::default();

        for (iteration, enabled) in [true, false, true].into_iter().enumerate() {
            let mut backdrop = backdrop(10.0, Element::new(Child)).enabled(enabled);
            tree.diff(&mut backdrop as &mut dyn Widget<(), (), Recorder>);
            let node = backdrop
                .layout(
                    &mut tree,
                    &renderer,
                    &layout::Limits::new(Size::ZERO, viewport.size()),
                )
                .move_to(Point::new(10.0, 20.0));
            let mut shell = Shell::new(&window::Headless, shell::Waker::noop(), &mut bus);
            backdrop.update(
                &mut tree,
                &event,
                Layout::new(&node),
                mouse::Cursor::Unavailable,
                &renderer,
                &mut shell,
                &viewport,
            );
            assert!(shell.is_event_captured());
            assert_eq!(shell.redraw_request(), window::RedrawRequest::Wait);
            assert_eq!(
                tree.state.downcast_ref::<State>().updates,
                iteration * 2 + 1
            );
            if iteration == 0 {
                backdrop.operate(
                    &mut tree,
                    Layout::new(&node),
                    &renderer,
                    &mut widget::operation::focusable::focus(widget::Id::new("child")),
                );
            }
            assert!(tree.state.downcast_ref::<State>().focused);

            let mut overlays = backdrop.overlay(
                &mut tree,
                Layout::new(&node),
                &renderer,
                &viewport,
                Vector::new(3.0, 7.0),
            );
            assert_eq!(overlays.len(), 1);
            let popup = overlays[0].as_overlay_mut();
            let popup_node = popup.layout(&renderer, viewport.size());
            assert_eq!(popup_node.bounds().position(), Point::new(13.0, 27.0));
            popup.update(
                &event,
                Layout::new(&popup_node),
                mouse::Cursor::Unavailable,
                &renderer,
                &mut shell,
            );
            assert_eq!(shell.redraw_request(), window::RedrawRequest::Wait);
            drop(overlays);
            assert_eq!(
                tree.state.downcast_ref::<State>().updates,
                (iteration + 1) * 2
            );
        }

        assert_eq!(bus.drain().count(), 6);
    }

    #[test]
    fn first_runtime_layout_and_click_work_with_nested_containers_and_buttons() {
        use iced_runtime::user_interface::{Cache, UserInterface};

        let content: Element<'_, (), crate::Theme, Recorder> = crate::container(
            crate::column![
                crate::button(crate::space().width(30).height(20))
                    .padding(5)
                    .on_press(()),
                crate::button(crate::space().width(50).height(10))
                    .padding(5)
                    .on_press(()),
            ]
            .spacing(6),
        )
        .padding(8)
        .into();
        let mut renderer = Recorder::default();
        let mut ui = UserInterface::build(
            backdrop(10.0, content),
            Size::new(200.0, 200.0),
            Cache::new(),
            &mut renderer,
        );
        let cursor = mouse::Cursor::Available(Point::new(15.0, 15.0));
        let mut bus = shell::Bus::new();
        let (_, statuses) = ui.update(
            &window::Headless,
            &shell::Waker::noop(),
            &[
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ],
            cursor,
            &mut renderer,
            &mut bus,
        );
        assert_eq!(statuses, [core::event::Status::Captured; 2]);
        assert_eq!(bus.drain().count(), 1);

        ui.draw(
            &mut renderer,
            &crate::Theme::Light,
            &renderer::Style::default(),
            cursor,
        );
        assert!(renderer.0.contains(&Command::Backdrop(renderer::Backdrop {
            bounds: Rectangle::with_size(Size::new(76.0, 72.0)),
            blur: 10.0.into(),
            ..Default::default()
        })));
    }
}
