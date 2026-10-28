use iced_widget::slider;
use iced_winit::core::{
    Event, Layout, Length, Rectangle, Shell, Size, Theme, Widget, keyboard, layout, mouse,
    renderer, touch,
    widget::{
        Meta, Operation,
        operation::Focusable,
        tree::{self, Tree},
    },
    window,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Preview(f64),
    Hover(Option<f64>),
    Commit(f64),
    Cancel,
}

pub struct Timeline {
    value: f64,
    duration: f64,
    generation: u64,
}

impl Timeline {
    pub fn new(value: f64, duration: f64) -> Self {
        Self {
            value,
            duration,
            generation: 0,
        }
    }
    pub fn generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Pointer {
    Mouse,
    Touch(touch::Finger),
}

#[derive(Default)]
struct State {
    pointer: Option<Pointer>,
    focused: bool,
    origin: f64,
    hovered: bool,
    suppressed: bool,
    generation: u64,
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

impl State {
    fn update(
        &mut self,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        value: f64,
        duration: f64,
    ) -> Option<Action> {
        let locate = |x: f32| {
            (f64::from(x - bounds.x) / f64::from(bounds.width.max(1.0)) * duration)
                .clamp(0.0, duration)
        };
        let press = match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                cursor.position().map(|position| (Pointer::Mouse, position))
            }
            Event::Touch(touch::Event::FingerPressed { id, position }) => {
                Some((Pointer::Touch(*id), *position))
            }
            _ => None,
        };
        if let Some((pointer, position)) = press {
            // A second finger cannot steal focus or finish the first gesture.
            if self.pointer.is_none() {
                self.focused = bounds.contains(position);
                if self.focused {
                    self.pointer = Some(pointer);
                    self.origin = value;
                    self.suppressed = false;
                    self.hovered = true;
                    return Some(Action::Preview(locate(position.x)));
                }
            }
        }
        let moved = match event {
            Event::Mouse(mouse::Event::CursorMoved { position })
                if self.pointer == Some(Pointer::Mouse) =>
            {
                Some(*position)
            }
            Event::Touch(touch::Event::FingerMoved { id, position })
                if self.pointer == Some(Pointer::Touch(*id)) =>
            {
                Some(*position)
            }
            _ => None,
        };
        if let Some(position) = moved {
            self.hovered = bounds.contains(position);
            return Some(if self.hovered {
                Action::Preview(locate(position.x))
            } else {
                Action::Hover(None)
            });
        }
        let released = match event {
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if self.pointer == Some(Pointer::Mouse) =>
            {
                Some(cursor.position())
            }
            Event::Touch(touch::Event::FingerLifted { id, position })
                if self.pointer == Some(Pointer::Touch(*id)) =>
            {
                Some(Some(*position))
            }
            _ => None,
        };
        if let Some(position) = released {
            self.pointer = None;
            self.suppressed = true;
            return Some(
                match position.filter(|position| bounds.contains(*position)) {
                    Some(position) => Action::Commit(locate(position.x)),
                    None => Action::Cancel,
                },
            );
        }
        let lost = matches!(event, Event::Window(window::Event::Unfocused))
            || matches!(event, Event::Touch(touch::Event::FingerLost { id, .. }) if self.pointer == Some(Pointer::Touch(*id)));
        if lost {
            self.focused = false;
            self.hovered = false;
            self.suppressed = true;
            return Some(if self.pointer.take().is_some() {
                Action::Cancel
            } else {
                Action::Hover(None)
            });
        }
        if let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event {
            use keyboard::key::Named;
            if matches!(key, keyboard::Key::Named(Named::Escape | Named::Tab)) {
                self.suppressed = true;
                if self.pointer.take().is_some() {
                    return Some(Action::Cancel);
                }
                if self.hovered {
                    self.hovered = false;
                    return Some(Action::Hover(None));
                }
            }
            // Keyboard input belongs to focus, not to whichever widget the mouse covers.
            if self.focused
                && self.pointer.is_none()
                && !modifiers.control()
                && !modifiers.alt()
                && !modifiers.logo()
            {
                let step = if modifiers.shift() { 1.0 } else { 5.0 };
                let target = match key {
                    keyboard::Key::Named(Named::ArrowLeft | Named::ArrowDown) => value - step,
                    keyboard::Key::Named(Named::ArrowRight | Named::ArrowUp) => value + step,
                    keyboard::Key::Named(Named::Home) => 0.0,
                    keyboard::Key::Named(Named::End) => duration,
                    _ => return None,
                }
                .clamp(0.0, duration);
                self.suppressed = true;
                return (target != value).then_some(Action::Commit(target));
            }
        }
        if let Event::Mouse(mouse::Event::CursorMoved { position }) = event
            && self.pointer.is_none()
        {
            let inside = bounds.contains(*position);
            let was_hovered = self.hovered;
            self.hovered = inside;
            if !inside {
                self.suppressed = false;
                return was_hovered.then_some(Action::Hover(None));
            }
            if !self.suppressed {
                return Some(Action::Hover(Some(locate(position.x))));
            }
        }
        if matches!(event, Event::Mouse(mouse::Event::CursorLeft)) {
            self.hovered = false;
            self.suppressed = false;
            return Some(Action::Hover(None));
        }
        None
    }
}

impl Meta for Timeline {}
impl<Renderer: renderer::Renderer> Widget<Action, Theme, Renderer> for Timeline {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State {
            generation: self.generation,
            ..State::default()
        })
    }
    fn diff(&mut self, tree: &mut Tree) {
        if tree.state.downcast_ref::<State>().generation != self.generation {
            tree.state = tree::State::new(State {
                generation: self.generation,
                ..State::default()
            });
        }
    }
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(40.0))
    }
    fn layout(&mut self, tree: &mut Tree, _: &Renderer, limits: &layout::Limits) {
        tree.size = layout::atomic(limits, Length::Fill, 40.0);
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        _: &Rectangle,
        _: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.focusable(None, layout.bounds(), tree.state.downcast_mut::<State>());
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        _: &Renderer,
        shell: &mut Shell<'_, Action>,
        _: &Rectangle,
    ) {
        if let Some(action) = tree.state.downcast_mut::<State>().update(
            event,
            layout.bounds(),
            cursor,
            self.value,
            self.duration,
        ) {
            if let Action::Commit(value) | Action::Preview(value) = action {
                // Several input events may precede the next application view rebuild.
                self.value = value;
            }
            if action == Action::Cancel {
                self.value = tree.state.downcast_ref::<State>().origin;
            }
            shell.publish(action);
            if !matches!(
                event,
                Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(keyboard::key::Named::Tab),
                    ..
                })
            ) {
                shell.capture_event();
            }
        }
        if !matches!(event, Event::Window(window::Event::RedrawRequested(_))) {
            shell.request_redraw();
        }
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
        let state = tree.state.downcast_ref::<State>();
        let status = if state.pointer.is_some() {
            slider::Status::Dragged
        } else if state.focused || cursor.is_over(layout.bounds()) {
            slider::Status::Hovered
        } else {
            slider::Status::Active
        };
        let visual = slider(0.0..=self.duration, self.value, Action::Preview)
            .height(40)
            .style(move |theme, _| slider::default(theme, status));
        Widget::<Action, Theme, Renderer>::draw(
            &visual, tree, renderer, theme, style, layout, cursor, viewport,
        );
    }
    fn mouse_interaction(
        &self,
        _: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        _: &Rectangle,
        _: &Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_winit::core::Point;

    fn replay(widget: &mut Timeline, tree: &mut Tree, event: Event, x: f32) -> Vec<Action> {
        use iced_winit::core::shell::{Bus, Waker};
        let mut bus = Bus::new();
        let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut bus);
        let layout = Layout::new(Size::new(100.0, 40.0));
        widget.update(
            tree,
            &event,
            layout,
            mouse::Cursor::Available(Point::new(x, 20.0)),
            &(),
            &mut shell,
            &layout.bounds(),
        );
        bus.into_iter().collect()
    }

    #[test]
    fn widget_hover_exit_and_media_change_invalidate_preview_and_drag() {
        let mut widget = Timeline::new(4.0, 100.0);
        let mut tree = Tree::new::<Action, Theme, ()>(&widget);
        let moved = |x| {
            Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(x, 20.0),
            })
        };
        assert_eq!(
            replay(&mut widget, &mut tree, moved(30.0), 30.0),
            vec![Action::Hover(Some(30.0))]
        );
        assert_eq!(
            replay(&mut widget, &mut tree, moved(120.0), 120.0),
            vec![Action::Hover(None)]
        );
        assert_eq!(widget.value, 4.0);
        replay(
            &mut widget,
            &mut tree,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            30.0,
        );
        widget = Timeline::new(0.0, 100.0).generation(1);
        Widget::<Action, Theme, ()>::diff(&mut widget, &mut tree);
        assert!(
            replay(
                &mut widget,
                &mut tree,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                70.0
            )
            .is_empty()
        );
    }

    #[test]
    fn widget_replay_commits_only_release_and_accumulates_keyboard_batch() {
        let mut widget = Timeline::new(4.0, 100.0);
        let mut tree = Tree::new::<Action, Theme, ()>(&widget);
        assert_eq!(
            replay(
                &mut widget,
                &mut tree,
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                30.0
            ),
            vec![Action::Preview(30.0)]
        );
        assert_eq!(
            replay(
                &mut widget,
                &mut tree,
                Event::Mouse(mouse::Event::CursorMoved {
                    position: Point::new(60.0, 20.0)
                }),
                60.0
            ),
            vec![Action::Preview(60.0)]
        );
        assert_eq!(
            replay(
                &mut widget,
                &mut tree,
                key(keyboard::key::Named::Escape),
                60.0
            ),
            vec![Action::Cancel]
        );
        assert!(
            replay(
                &mut widget,
                &mut tree,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                60.0
            )
            .is_empty()
        );
        assert_eq!(
            replay(
                &mut widget,
                &mut tree,
                key(keyboard::key::Named::ArrowRight),
                120.0
            ),
            vec![Action::Commit(9.0)]
        );
        assert_eq!(
            replay(
                &mut widget,
                &mut tree,
                key(keyboard::key::Named::ArrowRight),
                120.0
            ),
            vec![Action::Commit(14.0)]
        );
        assert_eq!(
            replay(
                &mut widget,
                &mut tree,
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                30.0
            ),
            vec![Action::Preview(30.0)]
        );
        assert_eq!(
            replay(
                &mut widget,
                &mut tree,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                70.0
            ),
            vec![Action::Commit(70.0)]
        );
    }

    fn input(state: &mut State, event: Event, x: f32) -> Option<Action> {
        state.update(
            &event,
            Rectangle::new(Point::ORIGIN, Size::new(100.0, 40.0)),
            mouse::Cursor::Available(Point::new(x, 20.0)),
            4.0,
            10.0,
        )
    }
    fn key(named: keyboard::key::Named) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(named),
            modified_key: keyboard::Key::Named(named),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        })
    }
    #[test]
    fn cancellation_consumes_gesture_without_late_commit() {
        for cancel in [
            key(keyboard::key::Named::Escape),
            Event::Window(window::Event::Unfocused),
        ] {
            let mut state = State::default();
            assert_eq!(
                input(
                    &mut state,
                    Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                    60.0
                ),
                Some(Action::Preview(6.0))
            );
            assert_eq!(input(&mut state, cancel, 60.0), Some(Action::Cancel));
            assert_eq!(
                input(
                    &mut state,
                    Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                    60.0
                ),
                None
            );
        }
        let mut state = State::default();
        input(
            &mut state,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            60.0,
        );
        assert_eq!(
            input(
                &mut state,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                120.0
            ),
            Some(Action::Cancel)
        );
    }
    #[test]
    fn touch_owner_is_preserved_and_loss_cancels() {
        let mut state = State::default();
        let event = |id, position| {
            Event::Touch(touch::Event::FingerPressed {
                id: touch::Finger(id),
                position,
            })
        };
        input(&mut state, event(1, Point::new(50.0, 20.0)), 50.0);
        input(&mut state, event(2, Point::new(120.0, 20.0)), 120.0);
        assert!(state.focused);
        assert_eq!(
            input(
                &mut state,
                Event::Touch(touch::Event::FingerLifted {
                    id: touch::Finger(2),
                    position: Point::new(80.0, 20.0)
                }),
                80.0
            ),
            None
        );
        assert_eq!(
            input(
                &mut state,
                Event::Touch(touch::Event::FingerLost {
                    id: touch::Finger(1),
                    position: Point::new(50.0, 20.0)
                }),
                50.0
            ),
            Some(Action::Cancel)
        );
    }
    #[test]
    fn keyboard_requires_focus_and_clamps_boundaries() {
        let mut state = State::default();
        assert_eq!(
            input(&mut state, key(keyboard::key::Named::ArrowRight), 50.0),
            None
        );
        state.focus();
        assert_eq!(
            input(&mut state, key(keyboard::key::Named::ArrowRight), 150.0),
            Some(Action::Commit(9.0))
        );
        assert_eq!(
            input(&mut state, key(keyboard::key::Named::Home), 150.0),
            Some(Action::Commit(0.0))
        );
        assert_eq!(
            input(&mut state, key(keyboard::key::Named::End), 150.0),
            Some(Action::Commit(10.0))
        );
    }
}
