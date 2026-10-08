use super::*;
use crate::core::image;
use crate::core::renderer::Renderer as _;
use crate::core::shell::{Bus, Waker};
use crate::core::widget::operation::{Focusable, focusable};
use crate::core::{Background, Transformation};

#[derive(Debug)]
enum Draw {
    Backdrop(renderer::Backdrop),
    Quad(renderer::Quad, Background),
}

#[derive(Default)]
struct Recorder(Vec<Draw>);

impl renderer::Renderer for Recorder {
    fn blur_backdrop(&mut self, _blur: impl Into<Blur>) {
        panic!("glass must provide effect bounds and optics");
    }
    fn draw_backdrop(&mut self, backdrop: renderer::Backdrop) {
        self.0.push(Draw::Backdrop(backdrop));
    }
    fn start_layer(&mut self, _bounds: Rectangle) {}
    fn end_layer(&mut self) {}
    fn start_transformation(&mut self, _transformation: Transformation) {}
    fn end_transformation(&mut self) {}
    fn fill_quad(&mut self, quad: renderer::Quad, background: impl Into<Background>) {
        self.0.push(Draw::Quad(quad, background.into()));
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

#[derive(Default)]
struct ProbeState {
    focused: bool,
    updates: usize,
}

impl Focusable for ProbeState {
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

struct Probe;
impl Meta for Probe {}

impl Widget<(), (), Recorder> for Probe {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<ProbeState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(ProbeState::default())
    }
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(100.0), Length::Fixed(80.0))
    }
    fn layout(&mut self, tree: &mut Tree, _renderer: &Recorder, _limits: &layout::Limits) {
        tree.size = Size::new(100.0, 80.0);
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
            Color::from_rgb(0.8, 0.1, 0.2),
        );
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        _viewport: &Rectangle,
        _renderer: &Recorder,
        operation: &mut dyn Operation,
    ) {
        operation.focusable(
            Some(&widget::Id::new("child")),
            layout.bounds(),
            tree.state.downcast_mut::<ProbeState>(),
        );
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        _layout: Layout,
        _cursor: mouse::Cursor,
        _renderer: &Recorder,
        shell: &mut Shell<'_, ()>,
        _viewport: &Rectangle,
    ) {
        tree.state.downcast_mut::<ProbeState>().updates += 1;
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        ) {
            shell.publish(());
            shell.capture_event();
        }
    }
}

struct Harness {
    widget: Glass<Probe>,
    tree: Tree,
    renderer: Recorder,
    layout: Layout,
    viewport: Rectangle,
}

impl Harness {
    fn new(widget: Glass<Probe>) -> Self {
        let mut harness = Self {
            tree: Tree::new(&widget),
            widget,
            renderer: Recorder::default(),
            layout: Layout::new(Size::ZERO),
            viewport: Rectangle::with_size(Size::new(500.0, 400.0)),
        };
        harness.widget.diff(&mut harness.tree);
        harness.widget.layout(
            &mut harness.tree,
            &harness.renderer,
            &layout::Limits::new(Size::ZERO, Size::INFINITE),
        );
        harness.layout = Layout::new(harness.tree.size).move_to(Point::new(20.0, 30.0));
        harness
    }
    fn send(&mut self, event: Event, cursor: mouse::Cursor) -> (bool, bool, window::RedrawRequest) {
        let mut bus = Bus::new();
        let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut bus);
        self.widget.update(
            &mut self.tree,
            &event,
            self.layout,
            cursor,
            &self.renderer,
            &mut shell,
            &self.viewport,
        );
        (
            shell.is_event_captured(),
            !shell.is_empty(),
            shell.redraw_request(),
        )
    }
    fn draw(&mut self) {
        self.renderer.0.clear();
        self.widget.draw(
            &self.tree,
            &mut self.renderer,
            &(),
            &renderer::Style::default(),
            self.layout,
            mouse::Cursor::Unavailable,
            &self.viewport,
        );
    }
    fn state(&self) -> &State {
        self.tree.state.downcast_ref::<State>()
    }
    fn operate(&mut self, operation: &mut dyn Operation) {
        self.widget.operate(
            &mut self.tree,
            self.layout,
            &self.viewport,
            &self.renderer,
            operation,
        );
    }
}

fn inside() -> mouse::Cursor {
    mouse::Cursor::Available(Point::new(50.0, 60.0))
}
fn outside() -> mouse::Cursor {
    mouse::Cursor::Available(Point::new(200.0, 200.0))
}
fn frame(now: Instant) -> Event {
    Event::Window(window::Event::RedrawRequested(now))
}

#[test]
fn child_events_and_geometry_are_preserved_while_glass_observes_captured_presses() {
    let mut harness = Harness::new(glass(Probe).reduce_motion(true));
    let cursor_move = Event::Mouse(mouse::Event::CursorMoved {
        position: Point::new(50.0, 60.0),
    });
    let (captured, published, _) = harness.send(cursor_move, inside());
    assert!(!captured && !published);
    let (captured, published, _) = harness.send(
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        inside(),
    );
    assert!(captured && published);
    assert_eq!(harness.state().current.pressed, 1.0);
    assert_eq!(
        harness.tree.children[0]
            .state
            .downcast_ref::<ProbeState>()
            .updates,
        2
    );
    harness.widget.diff(&mut harness.tree);
    assert_eq!(
        harness.tree.children[0]
            .state
            .downcast_ref::<ProbeState>()
            .updates,
        2
    );
    harness.draw();
    let [Draw::Backdrop(effect), Draw::Quad(foreground, _)] = harness.renderer.0.as_slice() else {
        panic!("unexpected drawing: {:?}", harness.renderer.0)
    };
    assert_eq!(effect.bounds, harness.layout.bounds());
    assert_eq!(foreground.bounds, harness.layout.bounds());
    assert!(effect.optics.unwrap().refraction > 0.0);
}

#[test]
fn leaving_releasing_or_losing_input_clears_press_feedback() {
    let mut harness = Harness::new(glass(Probe).reduce_motion(true));
    for cancellation in [
        Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(200.0, 200.0),
        }),
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        Event::Mouse(mouse::Event::CursorLeft),
        Event::Window(window::Event::Unfocused),
    ] {
        let _ = harness.send(Event::Window(window::Event::Focused), inside());
        let _ = harness.send(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            inside(),
        );
        assert_eq!(harness.state().current.pressed, 1.0);
        let _ = harness.send(cancellation, outside());
        assert_eq!(harness.state().current.pressed, 0.0);
    }
    for cancellation in [
        touch::Event::FingerMoved {
            id: touch::Finger(1),
            position: Point::new(200.0, 200.0),
        },
        touch::Event::FingerLifted {
            id: touch::Finger(1),
            position: Point::new(200.0, 200.0),
        },
        touch::Event::FingerLost {
            id: touch::Finger(1),
            position: Point::new(50.0, 60.0),
        },
    ] {
        let _ = harness.send(
            Event::Window(window::Event::Focused),
            mouse::Cursor::Unavailable,
        );
        let _ = harness.send(
            Event::Touch(touch::Event::FingerPressed {
                id: touch::Finger(1),
                position: Point::new(50.0, 60.0),
            }),
            mouse::Cursor::Unavailable,
        );
        assert_eq!(harness.state().current.pressed, 1.0);
        let _ = harness.send(Event::Touch(cancellation), mouse::Cursor::Unavailable);
        assert_eq!(harness.state().current.pressed, 0.0);
    }
}

#[test]
fn feedback_animates_then_stops_requesting_frames() {
    let mut harness = Harness::new(glass(Probe));
    let now = Instant::now();
    let (_, _, redraw) = harness.send(frame(now), inside());
    assert_eq!(redraw, window::RedrawRequest::NextFrame);
    let _ = harness.send(frame(now + State::DURATION / 2), inside());
    assert!(harness.state().current.hovered > 0.0 && harness.state().current.hovered < 1.0);
    let (_, _, redraw) = harness.send(frame(now + State::DURATION), inside());
    assert_eq!(harness.state().current.hovered, 1.0);
    assert_eq!(redraw, window::RedrawRequest::Wait);
    assert_eq!(
        harness.send(frame(now + State::DURATION * 2), inside()).2,
        window::RedrawRequest::Wait
    );
}

#[test]
fn focus_operations_are_observed_and_reduced_motion_has_no_transition() {
    let mut harness = Harness::new(glass(Probe).reduce_motion(true));
    harness.operate(&mut focusable::focus(widget::Id::new("child")));
    let (_, _, redraw) = harness.send(frame(Instant::now()), mouse::Cursor::Unavailable);
    assert_eq!(harness.state().current.focused, 1.0);
    assert!(harness.state().transition.is_none());
    assert_eq!(redraw, window::RedrawRequest::Wait);
    harness.operate(&mut focusable::unfocus());
    let _ = harness.send(frame(Instant::now()), mouse::Cursor::Unavailable);
    assert_eq!(harness.state().current.focused, 0.0);
    harness.widget = harness.widget.focused(true);
    let _ = harness.send(frame(Instant::now()), mouse::Cursor::Unavailable);
    assert_eq!(harness.state().current.focused, 1.0);
}

#[test]
fn reduced_transparency_bypasses_backdrop_and_high_contrast_keeps_a_focus_cue() {
    let mut harness = Harness::new(
        glass(Probe)
            .reduced_transparency(true)
            .high_contrast(true)
            .reduce_motion(true),
    );
    let _ = harness.send(frame(Instant::now()), inside());
    assert_eq!(harness.state().current.hovered, 0.0);
    harness.draw();
    assert!(
        !harness
            .renderer
            .0
            .iter()
            .any(|draw| matches!(draw, Draw::Backdrop(_)))
    );
    let [
        Draw::Quad(_, Background::Color(fill)),
        Draw::Quad(outer, _),
        Draw::Quad(inner, _),
        Draw::Quad(_, _),
    ] = harness.renderer.0.as_slice()
    else {
        panic!("unexpected drawing: {:?}", harness.renderer.0)
    };
    assert_eq!(fill.a, 1.0);
    assert_eq!(outer.border.color, Color::BLACK);
    assert_eq!(inner.border.color, Color::WHITE);
    let resting_width = outer.border.width;
    harness.operate(&mut focusable::focus(widget::Id::new("child")));
    let _ = harness.send(frame(Instant::now()), inside());
    harness.draw();
    let Draw::Quad(focused_outline, _) = &harness.renderer.0[1] else {
        unreachable!()
    };
    assert!(focused_outline.border.width > resting_width);
}

#[test]
fn noninteractive_glass_keeps_child_actions_without_pointer_feedback() {
    let mut harness = Harness::new(glass(Probe).interactive(false));
    let (captured, published, redraw) = harness.send(
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        inside(),
    );
    assert!(captured && published);
    assert_eq!(harness.state().target, Feedback::default());
    assert_eq!(redraw, window::RedrawRequest::Wait);
}
