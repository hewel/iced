use super::*;

fn close(actual: f32, expected: f32) {
    assert!(
        actual.is_finite() && (actual - expected).abs() < 0.001,
        "{actual} != {expected}"
    );
}

fn geometry(size: f32) -> (Rectangle, Rectangle) {
    (
        Rectangle {
            x: -40.0,
            y: -20.0,
            width: size,
            height: size,
        },
        Rectangle {
            width: 1_000_000.0,
            height: 2_000_000.0,
            ..Rectangle::default()
        },
    )
}

fn both(scrollbar: Scrollbar) -> Direction {
    Direction::Both {
        vertical: scrollbar,
        horizontal: scrollbar,
    }
}

#[test]
fn minimum_thumb_length_stays_inside_each_track() {
    for size in [0.0, 10.0, 32.0, 46.0, 300.0] {
        let (bounds, content) = geometry(size);
        let bars = Scrollbars::new(&State::new(), both(Scrollbar::new()), bounds, content);
        for (bar, horizontal) in [(bars.x.unwrap(), true), (bars.y.unwrap(), false)] {
            let thumb = bar.scroller.unwrap().bounds;
            let (start, length, track_start, track_length) = if horizontal {
                (thumb.x, thumb.width, bar.bounds.x, bar.bounds.width)
            } else {
                (thumb.y, thumb.height, bar.bounds.y, bar.bounds.height)
            };
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
            state.snap_to(RelativeOffset {
                x: Some(0.37),
                y: Some(0.61),
            });
            let bars = Scrollbars::new(&state, direction, bounds, content);
            for (bar, horizontal, initial) in [(bars.x, true, 0.37), (bars.y, false, 0.61)] {
                let Some(bar) = bar else { continue };
                let thumb = bar.scroller.unwrap().bounds;
                let grabbed_at = 0.23;
                let cursor = Point::new(
                    thumb.x + thumb.width * grabbed_at,
                    thumb.y + thumb.height * grabbed_at,
                );
                let grab = if horizontal {
                    bars.grab_x_scroller(cursor)
                } else {
                    bars.grab_y_scroller(cursor)
                }
                .unwrap();
                close(grab, grabbed_at);
                let percentage = if horizontal {
                    bar.scroll_percentage_x(grab, cursor)
                } else {
                    bar.scroll_percentage_y(grab, cursor)
                }
                .unwrap();
                close(percentage, initial);
                for physical in [1.0, 0.0, 0.72, 0.18, 1.0, 0.0] {
                    let cursor = if horizontal {
                        Point::new(
                            bar.bounds.x
                                + physical * (bar.bounds.width - thumb.width)
                                + grab * thumb.width,
                            cursor.y,
                        )
                    } else {
                        Point::new(
                            cursor.x,
                            bar.bounds.y
                                + physical * (bar.bounds.height - thumb.height)
                                + grab * thumb.height,
                        )
                    };
                    let percentage = if horizontal {
                        bar.scroll_percentage_x(grab, cursor)
                    } else {
                        bar.scroll_percentage_y(grab, cursor)
                    }
                    .unwrap();
                    let expected = match anchor {
                        Anchor::Start => physical,
                        Anchor::End => 1.0 - physical,
                    };
                    close(percentage, expected);
                    if horizontal {
                        state.scroll_x_to(percentage, bounds, content);
                    } else {
                        state.scroll_y_to(percentage, bounds, content);
                    }
                    let updated = Scrollbars::new(&state, direction, bounds, content);
                    let updated = if horizontal { updated.x } else { updated.y }
                        .unwrap()
                        .scroller
                        .unwrap()
                        .bounds;
                    close(
                        if horizontal {
                            updated.x + grab * updated.width
                        } else {
                            updated.y + grab * updated.height
                        },
                        if horizontal { cursor.x } else { cursor.y },
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
        state.snap_to(RelativeOffset {
            x: Some(0.4),
            y: Some(0.6),
        });
        let bars = Scrollbars::new(&state, both(configuration), bounds, content);
        for (bar, horizontal, expected) in
            [(bars.x.unwrap(), true, 0.4), (bars.y.unwrap(), false, 0.6)]
        {
            let thumb = bar.scroller.unwrap().bounds;
            for side in [
                0.5,
                if horizontal {
                    bar.total_bounds.height - 0.5
                } else {
                    bar.total_bounds.width - 0.5
                },
            ] {
                let cursor = if horizontal {
                    Point::new(thumb.x + thumb.width * 0.2, bar.total_bounds.y + side)
                } else {
                    Point::new(bar.total_bounds.x + side, thumb.y + thumb.height * 0.2)
                };
                assert!(!thumb.contains(cursor));
                let grab = if horizontal {
                    bars.grab_x_scroller(cursor)
                } else {
                    bars.grab_y_scroller(cursor)
                }
                .unwrap();
                close(grab, 0.2);
                close(
                    if horizontal {
                        bar.scroll_percentage_x(grab, cursor)
                    } else {
                        bar.scroll_percentage_y(grab, cursor)
                    }
                    .unwrap(),
                    expected,
                );
            }
            let track_cursor = Point::new(bar.total_bounds.x + 0.5, bar.total_bounds.y + 0.5);
            close(
                if horizontal {
                    bars.grab_x_scroller(track_cursor)
                } else {
                    bars.grab_y_scroller(track_cursor)
                }
                .unwrap(),
                0.5,
            );
        }
    }
}

#[test]
fn short_tracks_do_not_change_the_scroll_position() {
    for size in [0.0, 10.0, 32.0, 46.0] {
        let (bounds, content) = geometry(size);
        for anchor in [Anchor::Start, Anchor::End] {
            let mut state = State::new();
            state.snap_to(RelativeOffset {
                x: Some(0.4),
                y: Some(0.6),
            });
            let bars = Scrollbars::new(
                &state,
                both(Scrollbar::new().anchor(anchor)),
                bounds,
                content,
            );
            for (bar, horizontal) in [(bars.x.unwrap(), true), (bars.y.unwrap(), false)] {
                for delta in [-100.0, 0.0, 100.0] {
                    let cursor = Point::new(bar.bounds.x + delta, bar.bounds.y + delta);
                    let percentage = if horizontal {
                        bar.scroll_percentage_x(0.0, cursor)
                    } else {
                        bar.scroll_percentage_y(0.0, cursor)
                    };
                    assert_eq!(percentage, None);
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
        state.scroll_to(AbsoluteOffset {
            x: Some(50.0),
            y: Some(70.0),
        });
        state.scroll(Vector::new(12.0, -8.0), bounds, content);
        let bars = Scrollbars::new(&state, direction, bounds, content);
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
                bars.grab_x_scroller(Point::new(bounds.x + 10.0, bounds.y + bounds.height))
                    .is_none()
            );
            assert!(
                bars.grab_y_scroller(Point::new(bounds.x + bounds.width, bounds.y + 10.0))
                    .is_none()
            );
        }
    }
}

#[test]
fn native_pointer_events_preserve_grab_and_zero_travel_offsets() {
    use crate::core::shell::{Bus, Waker};

    for horizontal in [false, true] {
        for anchor in [Anchor::Start, Anchor::End] {
            for touch_input in [false, true] {
                let configuration = Scrollbar::new().anchor(anchor);
                let direction = if horizontal {
                    Direction::Horizontal(configuration)
                } else {
                    Direction::Vertical(configuration)
                };
                let mut widget: Scrollable<'_, (), Theme, ()> =
                    Scrollable::new(crate::Space::new()).direction(direction);
                let mut tree = Tree::new(&widget as &dyn Widget<(), Theme, ()>);
                widget.diff(&mut tree);
                tree.state.downcast_mut::<State>().snap_to(RelativeOffset {
                    x: Some(0.4),
                    y: Some(0.4),
                });
                for size in [300.0, 32.0] {
                    let (bounds, content) = geometry(size);
                    let node = layout::Node::with_children(
                        bounds.size(),
                        vec![layout::Node::new(content.size())],
                    );
                    let bounds = node.bounds();
                    let content = node.children()[0].bounds();
                    let bars = Scrollbars::new(
                        tree.state.downcast_ref::<State>(),
                        direction,
                        bounds,
                        content,
                    );
                    let bar = if horizontal { bars.x } else { bars.y }.unwrap();
                    let thumb = bar.scroller.unwrap().bounds;
                    let cursor = if horizontal {
                        Point::new(thumb.x + thumb.width * 0.2, bar.total_bounds.y + 0.5)
                    } else {
                        Point::new(bar.total_bounds.x + 0.5, thumb.y + thumb.height * 0.2)
                    };
                    let mut send = |tree: &mut Tree, event: Event, cursor| {
                        let mut bus = Bus::new();
                        let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut bus);
                        widget.update(
                            tree,
                            &event,
                            Layout::new(&node),
                            mouse::Cursor::Available(cursor),
                            &(),
                            &mut shell,
                            &bounds,
                        );
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
                    close(
                        if horizontal {
                            state.x_scroller_grabbed_at()
                        } else {
                            state.y_scroller_grabbed_at()
                        }
                        .unwrap(),
                        0.2,
                    );
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
                    let moved = if horizontal {
                        Point::new(cursor.x + 50.0, cursor.y)
                    } else {
                        Point::new(cursor.x, cursor.y + 50.0)
                    };
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
                        if horizontal {
                            state.offset_x.absolute(bounds.width, content.width)
                                / (content.width - bounds.width)
                        } else {
                            state.offset_y.absolute(bounds.height, content.height)
                                / (content.height - bounds.height)
                        },
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
                    assert!(!tree.state.downcast_ref::<State>().scrollers_grabbed());
                    tree.state.downcast_mut::<State>().snap_to(RelativeOffset {
                        x: Some(0.4),
                        y: Some(0.4),
                    });
                }
            }
        }
    }
}
