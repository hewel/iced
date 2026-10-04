//! Backdrop ordering and coordinate contracts at the widget/renderer boundary.
use iced_widget::backdrop;
use iced_widget::core::layout;
use iced_widget::core::mouse;
use iced_widget::core::renderer::{self, Renderer as _};
use iced_widget::core::widget::{Meta, Tree};
use iced_widget::core::{
    Background, Blur, Color, Layout, Length, Point, Rectangle, Size, Transformation, Widget, image,
};

#[derive(Debug, PartialEq)]
enum Command {
    Clip(Rectangle),
    Backdrop(renderer::Backdrop),
    Quad(renderer::Quad, Background),
    EndClip,
}

#[derive(Default)]
struct Recorder(Vec<Command>);

impl renderer::Renderer for Recorder {
    fn blur_backdrop(&mut self, _blur: impl Into<Blur>) {
        panic!("a widget backdrop must specify its full effect bounds");
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
        unreachable!();
    }

    fn hint(&mut self, _scale: renderer::Scale) {}
    fn scale(&self) -> Option<renderer::Scale> {
        None
    }
    fn reset(&mut self, _bounds: Rectangle) {
        self.0.clear();
    }
    fn settings(&self) -> renderer::Settings {
        renderer::Settings::default()
    }
}

struct Foreground;
impl Meta for Foreground {}

impl Widget<(), (), Recorder> for Foreground {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(120.0), Length::Fixed(150.0))
    }

    fn layout(&mut self, tree: &mut Tree, _renderer: &Recorder, _limits: &layout::Limits) {
        tree.size = Size::new(120.0, 150.0);
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Recorder,
        _theme: &(),
        _style: &renderer::Style,
        layout: Layout,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        renderer.fill_quad(
            renderer::Quad {
                bounds: layout.bounds(),
                ..Default::default()
            },
            Color::WHITE,
        );
    }
}

#[test]
fn clipped_backdrop_keeps_profile_bounds_and_draws_tint_before_foreground() {
    let blur = Blur::vertical_gradient(0.0, 24.0).range(0.25, 0.75);
    let tint = Color::from_rgba(0.1, 0.2, 0.3, 0.4);
    let mut widget = backdrop(blur, Foreground)
        .border_radius(16)
        .border_smoothing(0.6)
        .tint(tint);
    let mut tree = Tree::new(&widget);
    let mut renderer = Recorder::default();
    widget.diff(&mut tree);
    widget.layout(
        &mut tree,
        &renderer,
        &layout::Limits::new(Size::ZERO, Size::INFINITE),
    );
    let layout = Layout::new(tree.size).move_to(Point::new(10.0, -40.0));
    let viewport = Rectangle::with_size(Size::new(100.0, 70.0));

    widget.draw(
        &tree,
        &mut renderer,
        &(),
        &renderer::Style::default(),
        layout,
        mouse::Cursor::Unavailable,
        &viewport,
    );

    let [
        Command::Clip(clip),
        Command::Backdrop(effect),
        Command::Quad(tint_quad, color),
        Command::Quad(foreground, foreground_color),
        Command::EndClip,
    ] = renderer.0.as_slice()
    else {
        panic!("unexpected draw order: {:?}", renderer.0);
    };
    assert_eq!(*clip, layout.bounds().intersection(&viewport).unwrap());
    assert_eq!(effect.bounds, layout.bounds());
    assert_eq!(effect.blur, blur);
    assert_eq!(effect.border_radius, 16.0.into());
    assert_eq!(effect.border_smoothing, 0.6);
    assert_eq!(tint_quad.bounds, effect.bounds);
    assert_eq!(tint_quad.border.radius, effect.border_radius);
    assert_eq!(tint_quad.border.smoothing, effect.border_smoothing);
    assert!(!tint_quad.snap);
    assert_eq!(*color, tint.into());
    assert_eq!(foreground.bounds, layout.bounds());
    assert_eq!(*foreground_color, Color::WHITE.into());
}
