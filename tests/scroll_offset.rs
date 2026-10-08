//! Current scroll state and presented geometry have distinct public contracts.
#![cfg(feature = "tiny-skia")]

use iced::advanced::renderer::{Headless, Settings, Style};
use iced::advanced::shell::{Bus, Waker};
use iced::advanced::widget::{self, Widget as _, operation};
use iced::widget::scrollable::{Anchor, Scroll};
use iced::widget::{scrollable, space};
use iced::{Element, Event, Point, Rectangle, Size, Theme, Vector};
use iced_runtime::user_interface::{Cache, UserInterface};

type Ui<'a> = UserInterface<'a, Scroll, Theme, iced::Renderer>;

fn renderer() -> iced::Renderer {
    iced::futures::executor::block_on(iced::Renderer::new(Settings::default(), Some("tiny-skia")))
        .expect("headless software renderer")
}

fn page(height: f32, anchor: Anchor) -> Element<'static, Scroll> {
    scrollable(space().width(100).height(height))
        .id("page")
        .width(100)
        .height(100)
        .anchor_y(anchor)
        .smooth_scroll(false)
        .on_scroll(|scroll| scroll)
        ._boxed()
}

fn build(cache: Cache, renderer: &mut iced::Renderer, height: f32, anchor: Anchor) -> Ui<'static> {
    Ui::build(
        page(height, anchor),
        Size::new(100.0, 100.0),
        cache,
        renderer,
    )
}

#[derive(Default)]
struct Position {
    current: Option<operation::scrollable::AbsoluteOffset>,
    presented: Vector,
}

impl widget::Operation for Position {
    fn traverse(&mut self, visit: &mut dyn FnMut(&mut dyn widget::Operation)) {
        visit(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&widget::Id>,
        bounds: Rectangle,
        content: Size,
        translation: Vector,
        state: &mut dyn operation::Scrollable,
    ) {
        if id == Some(&widget::Id::new("page")) {
            self.current = state.absolute_offset(bounds, content);
            self.presented = translation;
        }
    }
}

fn position(ui: &mut Ui<'_>, renderer: &iced::Renderer) -> Position {
    let mut position = Position::default();
    ui.operate(renderer, &mut position);
    assert!(
        position.current.is_some(),
        "native scroll state supports queries"
    );
    position
}

fn frame(ui: &mut Ui<'_>, renderer: &mut iced::Renderer, bus: &mut Bus<Scroll>) {
    let _ = ui.update(
        &iced::window::Headless,
        &Waker::noop(),
        &[Event::Window(iced::window::Event::RedrawRequested(
            iced::time::Instant::now(),
        ))],
        iced::mouse::Cursor::Unavailable,
        renderer,
        bus,
    );
    ui.draw(
        renderer,
        &Theme::Dark,
        &Style::default(),
        iced::mouse::Cursor::Unavailable,
    );
}

fn scroll_to(ui: &mut Ui<'_>, renderer: &iced::Renderer, y: f32) {
    ui.operate(
        renderer,
        &mut operation::scrollable::scroll_to::<()>(
            widget::Id::new("page"),
            operation::scrollable::AbsoluteOffset {
                x: None,
                y: Some(y),
            },
            operation::Animation::Instant,
        ),
    );
}

#[test]
fn instant_offset_is_queryable_without_committing_presented_geometry() {
    let mut renderer = renderer();
    let mut bus = Bus::new();
    let mut ui = build(Cache::new(), &mut renderer, 1000.0, Anchor::Start);
    frame(&mut ui, &mut renderer, &mut bus);

    scroll_to(&mut ui, &renderer, 120.0);
    let before_frame = position(&mut ui, &renderer);
    assert_eq!(before_frame.current.unwrap().y, 120.0);
    assert_eq!(before_frame.presented.y, 0.0);

    frame(&mut ui, &mut renderer, &mut bus);
    let after_frame = position(&mut ui, &renderer);
    assert_eq!(after_frame.current.unwrap().y, 120.0);
    assert_eq!(after_frame.presented.y, 120.0);
}

#[test]
fn wheel_offset_survives_message_rebuild_before_the_next_frame() {
    let mut renderer = renderer();
    let mut bus = Bus::new();
    let mut ui = build(Cache::new(), &mut renderer, 1000.0, Anchor::Start);
    scroll_to(&mut ui, &renderer, 450.0);
    frame(&mut ui, &mut renderer, &mut bus);
    bus.drain().for_each(drop);

    let _ = ui.update(
        &iced::window::Headless,
        &Waker::noop(),
        &[Event::Mouse(iced::mouse::Event::WheelScrolled {
            delta: iced::mouse::ScrollDelta::Pixels { x: 0.0, y: -60.0 },
        })],
        iced::mouse::Cursor::Available(Point::new(50.0, 50.0)),
        &mut renderer,
        &mut bus,
    );
    let notified = bus
        .drain()
        .last()
        .expect("wheel notification")
        .0
        .viewport
        .absolute_offset()
        .y;
    assert_eq!(notified, 510.0);
    let captured = position(&mut ui, &renderer);
    assert_eq!(captured.presented.y, 450.0);
    let saved = captured.current.unwrap().y;
    assert_eq!(saved, notified);

    // The message rebuild and restoration happen before any new Redraw event.
    let mut ui = build(ui.into_cache(), &mut renderer, 1000.0, Anchor::Start);
    scroll_to(&mut ui, &renderer, saved);
    frame(&mut ui, &mut renderer, &mut bus);
    assert_eq!(position(&mut ui, &renderer).presented.y, 510.0);
}

#[test]
fn layout_clamping_does_not_masquerade_as_input_or_erase_the_stored_offset() {
    let mut renderer = renderer();
    let mut bus = Bus::new();
    let mut ui = build(Cache::new(), &mut renderer, 1000.0, Anchor::Start);
    scroll_to(&mut ui, &renderer, 450.0);
    frame(&mut ui, &mut renderer, &mut bus);

    let mut short = build(ui.into_cache(), &mut renderer, 50.0, Anchor::Start);
    let before_frame = position(&mut short, &renderer);
    assert_eq!(before_frame.current.unwrap().y, 0.0);
    assert_eq!(before_frame.presented.y, 450.0);
    frame(&mut short, &mut renderer, &mut bus);
    let after_frame = position(&mut short, &renderer);
    assert_eq!(after_frame.current, before_frame.current);
    assert_eq!(after_frame.presented.y, 0.0);

    let mut expanded = build(short.into_cache(), &mut renderer, 1000.0, Anchor::Start);
    let before_frame = position(&mut expanded, &renderer);
    assert_eq!(before_frame.current.unwrap().y, 450.0);
    assert_eq!(before_frame.presented.y, 0.0);
    frame(&mut expanded, &mut renderer, &mut bus);
    assert_eq!(position(&mut expanded, &renderer).presented.y, 450.0);
}

#[test]
fn anchored_snapped_offset_is_distinct_from_content_translation() {
    let mut renderer = renderer();
    let mut bus = Bus::new();
    let mut ui = build(Cache::new(), &mut renderer, 1000.0, Anchor::End);
    frame(&mut ui, &mut renderer, &mut bus);
    let initial = position(&mut ui, &renderer);
    assert_eq!(initial.current.unwrap().y, 0.0);
    assert_eq!(initial.presented.y, 900.0);

    ui.operate(
        &renderer,
        &mut operation::scrollable::snap_to::<()>(
            widget::Id::new("page"),
            operation::scrollable::RelativeOffset {
                x: None,
                y: Some(0.25),
            },
            operation::Animation::Instant,
        ),
    );
    let before_frame = position(&mut ui, &renderer);
    assert_eq!(before_frame.current.unwrap().y, 225.0);
    assert_eq!(before_frame.presented.y, 900.0);
    frame(&mut ui, &mut renderer, &mut bus);
    assert_eq!(position(&mut ui, &renderer).presented.y, 675.0);
}
