use super::*;

fn close(actual: f32, expected: f32) {
    assert!(
        actual.is_finite() && (actual - expected).abs() < 0.001,
        "{actual} != {expected}"
    );
}

fn geometry(size: f32) -> (Rectangle, Size) {
    (
        Rectangle {
            x: -40.0,
            y: -20.0,
            width: size,
            height: size,
        },
        Size::new(1_000_000.0, 2_000_000.0),
    )
}

fn both(scrollbar: Scrollbar) -> Direction {
    Direction::Both {
        vertical: scrollbar,
        horizontal: scrollbar,
    }
}

fn snap(state: &mut State, x: f32, y: f32, bounds: Rectangle, content: Size) {
    state.snap_to(
        RelativeOffset {
            x: Some(x),
            y: Some(y),
        },
        Animation::Instant,
        bounds,
        content,
        Source::Operation,
    );
}

fn bars(state: &State, direction: Direction, bounds: Rectangle, content: Size) -> Scrollbars {
    Scrollbars::new(
        state.translation(direction, bounds, content),
        direction,
        bounds,
        content,
    )
}

fn grab(scrollbars: &Scrollbars, axis: Axis, cursor: Point) -> f32 {
    let (_, Hit::Scroller { grabbed_at }) = scrollbars.hit(axis, cursor).unwrap() else {
        panic!("expected a scroller grab at {cursor:?}");
    };
    grabbed_at
}

#[test]
fn minimum_thumb_length_stays_inside_each_track() {
    for size in [0.0, 10.0, 32.0, 46.0, 300.0] {
        let (bounds, content) = geometry(size);
        let bars = bars(&State::new(), both(Scrollbar::new()), bounds, content);
        for axis in [Axis::X, Axis::Y] {
            let bar = bars.scrollbar(axis).unwrap();
            let thumb = bar.scroller.unwrap().bounds;
            let start = axis.coordinate(thumb.position());
            let length = axis.length(thumb);
            let track_start = axis.coordinate(bar.bounds.position());
            let track_length = axis.length(bar.bounds);
            close(length, track_length.min(32.0));
            assert!(start >= track_start && start + length <= track_start + track_length);
        }
    }
}

#[test]
fn dragging_round_trips_and_reaches_both_ends() {
    let (bounds, content) = geometry(300.0);
    for anchor in [Anchor::Start, Anchor::End] {
        let configuration = Scrollbar::new().anchor(anchor);
        for direction in [
            Direction::Vertical(configuration),
            Direction::Horizontal(configuration),
            both(configuration),
        ] {
            let mut state = State::new();
            snap(&mut state, 0.37, 0.61, bounds, content);
            let scrollbars = bars(&state, direction, bounds, content);
            for (axis, initial) in [(Axis::X, 0.37), (Axis::Y, 0.61)] {
                let Some(bar) = scrollbars.scrollbar(axis) else {
                    continue;
                };
                let thumb = bar.scroller.unwrap().bounds;
                let grabbed_at = 0.23;
                let cursor = Point::new(
                    thumb.x + thumb.width * grabbed_at,
                    thumb.y + thumb.height * grabbed_at,
                );
                let grabbed_at = grab(&scrollbars, axis, cursor);
                close(grabbed_at, 0.23);
                close(
                    bar.scroll_percentage(axis, grabbed_at, cursor).unwrap(),
                    initial,
                );
                for physical in [1.0, 0.0, 0.72, 0.18, 1.0, 0.0] {
                    let position = axis.coordinate(bar.bounds.position())
                        + physical * (axis.length(bar.bounds) - axis.length(thumb))
                        + grabbed_at * axis.length(thumb);
                    let cursor = cursor + axis.vector(position - axis.coordinate(cursor));
                    let percentage = bar.scroll_percentage(axis, grabbed_at, cursor).unwrap();
                    let expected = match anchor {
                        Anchor::Start => physical,
                        Anchor::End => 1.0 - physical,
                    };
                    close(percentage, expected);
                    state.scroll_to_percentage(axis, Some(percentage), bounds, content);
                    let updated = bars(&state, direction, bounds, content)
                        .scrollbar(axis)
                        .unwrap()
                        .scroller
                        .unwrap()
                        .bounds;
                    close(
                        axis.coordinate(updated.position()) + grabbed_at * axis.length(updated),
                        axis.coordinate(cursor),
                    );
                }
            }
        }
    }
}

#[test]
fn expanded_hit_area_keeps_axial_grab_position() {
    let (bounds, content) = geometry(300.0);
    for configuration in [
        Scrollbar::new(),
        Scrollbar::new().width(24).scroller_width(6).margin(3),
    ] {
        let mut state = State::new();
        snap(&mut state, 0.4, 0.6, bounds, content);
        let bars = bars(&state, both(configuration), bounds, content);
        for (axis, expected) in [(Axis::X, 0.4), (Axis::Y, 0.6)] {
            let bar = bars.scrollbar(axis).unwrap();
            let thumb = bar.scroller.unwrap().bounds;
            let cross_length = match axis {
                Axis::X => bar.total_bounds.height,
                Axis::Y => bar.total_bounds.width,
            };
            for side in [0.5, cross_length - 0.5] {
                let cursor = match axis {
                    Axis::X => Point::new(thumb.x + thumb.width * 0.2, bar.total_bounds.y + side),
                    Axis::Y => Point::new(bar.total_bounds.x + side, thumb.y + thumb.height * 0.2),
                };
                assert!(!thumb.contains(cursor));
                let grabbed_at = grab(&bars, axis, cursor);
                close(grabbed_at, 0.2);
                close(
                    bar.scroll_percentage(axis, grabbed_at, cursor).unwrap(),
                    expected,
                );
            }
            let track_cursor = Point::new(bar.total_bounds.x + 0.5, bar.total_bounds.y + 0.5);
            assert!(matches!(bars.hit(axis, track_cursor), Some((_, Hit::Rail))));
        }
    }
}

#[test]
fn short_tracks_do_not_change_the_scroll_position() {
    for size in [0.0, 10.0, 32.0, 46.0] {
        let (bounds, content) = geometry(size);
        for anchor in [Anchor::Start, Anchor::End] {
            let mut state = State::new();
            snap(&mut state, 0.4, 0.6, bounds, content);
            let bars = bars(
                &state,
                both(Scrollbar::new().anchor(anchor)),
                bounds,
                content,
            );
            for axis in [Axis::X, Axis::Y] {
                let bar = bars.scrollbar(axis).unwrap();
                for delta in [-100.0, 0.0, 100.0] {
                    let cursor = Point::new(bar.bounds.x + delta, bar.bounds.y + delta);
                    let percentage = bar.scroll_percentage(axis, 0.0, cursor);
                    assert_eq!(percentage, None);
                    let previous = state.axis_offset(axis, bounds, content);
                    state.scroll_to_percentage(axis, percentage, bounds, content);
                    close(state.axis_offset(axis, bounds, content), previous);
                }
            }
        }
    }
}

#[test]
fn custom_widths_and_hidden_scrollbars_preserve_native_scroll() {
    let (bounds, content) = geometry(300.0);
    for configuration in [
        Scrollbar::new().width(7).scroller_width(3),
        Scrollbar::hidden(),
    ] {
        let direction = both(configuration);
        let mut state = State::new();
        state.scroll_to(
            AbsoluteOffset {
                x: Some(50.0),
                y: Some(70.0),
            },
            Animation::Instant,
            bounds,
            content,
            Source::Operation,
        );
        state.scroll(Vector::new(12.0, -8.0), bounds, content);
        let bars = bars(&state, direction, bounds, content);
        let x = bars.x.unwrap();
        let y = bars.y.unwrap();
        close(x.bounds.height, configuration.width);
        close(y.bounds.width, configuration.width);
        close(
            x.scroller.unwrap().bounds.height,
            configuration.scroller_width,
        );
        close(
            y.scroller.unwrap().bounds.width,
            configuration.scroller_width,
        );
        let translation = state.translation(direction, bounds, content);
        close(translation.x, 62.0);
        close(translation.y, 62.0);
        if configuration.width == 0.0 {
            assert!(
                bars.hit(
                    Axis::X,
                    Point::new(bounds.x + 10.0, bounds.y + bounds.height)
                )
                .is_none()
            );
            assert!(
                bars.hit(
                    Axis::Y,
                    Point::new(bounds.x + bounds.width, bounds.y + 10.0)
                )
                .is_none()
            );
        }
    }
}

#[test]
fn native_pointer_events_preserve_grab_and_zero_travel_offsets() {
    use crate::core::shell::{Bus, Waker};

    for axis in [Axis::X, Axis::Y] {
        for anchor in [Anchor::Start, Anchor::End] {
            for touch_input in [false, true] {
                let configuration = Scrollbar::new().anchor(anchor);
                let direction = match axis {
                    Axis::X => Direction::Horizontal(configuration),
                    Axis::Y => Direction::Vertical(configuration),
                };
                let mut widget: Scrollable<'_, (), crate::Space, Theme> =
                    Scrollable::new(crate::Space::new()).direction(direction);
                let mut tree = Tree::new::<(), Theme, ()>(&widget);
                Widget::<(), Theme, ()>::diff(&mut widget, &mut tree);
                for size in [300.0, 32.0] {
                    let (_, content) = geometry(size);
                    let bounds = Rectangle::with_size(Size::new(size, size));
                    tree.size = bounds.size();
                    tree.children[0].size = content;
                    snap(
                        tree.state.downcast_mut::<State>(),
                        0.4,
                        0.4,
                        bounds,
                        content,
                    );
                    let layout = Layout::new(tree.size);
                    let mut send = |tree: &mut Tree, event: Event, cursor| {
                        let mut bus = Bus::new();
                        let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut bus);
                        widget.update(
                            tree,
                            &event,
                            layout,
                            mouse::Cursor::Available(cursor),
                            &(),
                            &mut shell,
                            &bounds,
                        );
                    };
                    send(
                        &mut tree,
                        Event::Window(window::Event::RedrawRequested(Instant::now())),
                        Point::ORIGIN,
                    );
                    let bars = bars(
                        tree.state.downcast_ref::<State>(),
                        direction,
                        bounds,
                        content,
                    );
                    let bar = bars.scrollbar(axis).unwrap();
                    let thumb = bar.scroller.unwrap().bounds;
                    let cursor = match axis {
                        Axis::X => {
                            Point::new(thumb.x + thumb.width * 0.2, bar.total_bounds.y + 0.5)
                        }
                        Axis::Y => {
                            Point::new(bar.total_bounds.x + 0.5, thumb.y + thumb.height * 0.2)
                        }
                    };
                    let press = if touch_input {
                        Event::Touch(touch::Event::FingerPressed {
                            id: touch::Finger(0),
                            position: cursor,
                        })
                    } else {
                        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                    };
                    send(&mut tree, press, cursor);
                    let state = tree.state.downcast_ref::<State>();
                    let (grabbed_axis, grabbed_at) = state.interaction.scroller_grabbed().unwrap();
                    assert_eq!(grabbed_axis, axis);
                    close(grabbed_at, 0.2);
                    close(
                        state.offset_x.absolute(bounds.width, content.width)
                            / (content.width - bounds.width),
                        0.4,
                    );
                    close(
                        state.offset_y.absolute(bounds.height, content.height)
                            / (content.height - bounds.height),
                        0.4,
                    );
                    let moved = cursor + axis.vector(50.0);
                    let movement = if touch_input {
                        Event::Touch(touch::Event::FingerMoved {
                            id: touch::Finger(0),
                            position: moved,
                        })
                    } else {
                        Event::Mouse(mouse::Event::CursorMoved { position: moved })
                    };
                    send(&mut tree, movement, moved);
                    let state = tree.state.downcast_ref::<State>();
                    let expected = if size == 32.0 {
                        0.4
                    } else {
                        0.4 + if anchor == Anchor::Start {
                            50.0 / 268.0
                        } else {
                            -50.0 / 268.0
                        }
                    };
                    close(
                        state.axis_offset(axis, bounds, content)
                            / (axis.length(content) - axis.length(bounds)),
                        expected,
                    );
                    let release = if touch_input {
                        Event::Touch(touch::Event::FingerLifted {
                            id: touch::Finger(0),
                            position: moved,
                        })
                    } else {
                        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                    };
                    send(&mut tree, release, moved);
                    assert!(
                        !tree
                            .state
                            .downcast_ref::<State>()
                            .interaction
                            .scrollers_grabbed()
                    );
                }
            }
        }
    }
}
