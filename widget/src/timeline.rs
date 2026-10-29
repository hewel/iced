//! A continuous timeline with transactional dragging and caller-owned previews.
//!
//! Dragging publishes targets, never commits. Releasing inside commits once;
//! Escape, focus loss, owning-touch loss and outside release cancel. Applications
//! own admission, units, media identity and preview work. Retire external preview
//! demand when removing/hiding this widget: a removed widget cannot emit events.
use crate::core::{
    Element, Event, Layout, Length, Pixels, Point, Rectangle, Shell, Size, Vector, Widget,
    keyboard, layout, mouse, overlay, renderer, touch,
    widget::{
        Meta, Operation,
        operation::Focusable,
        tree::{self, Tree},
    },
    window,
};
use crate::slider;
pub use slider::{Catalog, Status, Style, StyleFn};
use std::ops::RangeInclusive;

/// A timeline interaction, in the caller's units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// A held pointer/touch proposes a target without committing.
    Preview(f64),
    /// Hover demand changed or left the timeline.
    Hover(Option<f64>),
    /// Inside release or a focused keyboard action commits a target.
    Commit(f64),
    /// Abandon the gesture and any external preview demand.
    Cancel,
}

/// A media-agnostic rail with focus, cancellation and a display-only preview.
pub struct Timeline<'a, Message, Theme = crate::Theme, Renderer = crate::Renderer>
where
    Theme: Catalog,
    Renderer: renderer::Renderer,
{
    value: f64,
    range: RangeInclusive<f64>,
    generation: u64,
    enabled: bool,
    step: f64,
    shift_step: f64,
    width: Length,
    height: f32,
    gap: f32,
    on_action: Box<dyn Fn(Action) -> Message + 'a>,
    class: Theme::Class<'a>,
    track: Option<Element<'a, Message, Theme, Renderer>>,
    preview: Option<Element<'a, Message, Theme, Renderer>>,
}

impl<'a, Message, Theme, Renderer> Timeline<'a, Message, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: renderer::Renderer,
{
    /// Creates a continuous timeline. Invalid/empty ranges disable interaction.
    pub fn new(
        range: RangeInclusive<f64>,
        value: f64,
        on_action: impl Fn(Action) -> Message + 'a,
    ) -> Self {
        let range =
            if range.start().is_finite() && range.end().is_finite() && range.end() >= range.start()
            {
                range
            } else {
                0.0..=0.0
            };
        let value = if value.is_finite() {
            value.clamp(*range.start(), *range.end())
        } else {
            *range.start()
        };
        Self {
            value,
            range,
            generation: 0,
            enabled: true,
            step: 1.0,
            shift_step: 0.1,
            width: Length::Fill,
            height: 40.0,
            gap: 8.0,
            on_action: Box::new(on_action),
            class: Theme::default(),
            track: None,
            preview: None,
        }
    }
    /// Invalidates existing gestures when the caller replaces its source.
    pub fn generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }
    /// Disables all interaction and overlay visibility. The next update cancels
    /// existing demand; also retire external work when removing the widget.
    /// Temporary disabling preserves existing keyboard focus, but does not admit
    /// new focus or input. Source generation changes and unfocus still retire it.
    /// While focused, consumes arrows/Home/End (including Shift) without actions.
    /// Tab and Ctrl/Alt/Logo shortcuts remain available to other handlers.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    /// Sets the normal keyboard increment; pointer targets remain continuous.
    pub fn step(mut self, step: f64) -> Self {
        self.step = step.abs();
        self
    }
    /// Sets the Shift keyboard increment.
    pub fn shift_step(mut self, step: f64) -> Self {
        self.shift_step = step.abs();
        self
    }
    /// Sets the rail width.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }
    /// Sets the rail's interaction height.
    pub fn height(mut self, height: impl Into<Pixels>) -> Self {
        self.height = height.into().0;
        self
    }
    /// Sets the distance between the rail and the preview.
    pub fn preview_gap(mut self, gap: impl Into<Pixels>) -> Self {
        self.gap = gap.into().0.max(0.0);
        self
    }
    /// Provides display-only content. The widget owns visibility and placement;
    /// caller content owns image, timestamp, loading, error and chapter styling.
    pub fn preview(mut self, content: impl Widget<Message, Theme, Renderer> + 'a) -> Self {
        self.preview = Some(content._boxed());
        self
    }
    /// Replaces default rail drawing with render-only decoration. Input/focus
    /// always belongs to the timeline; this content receives no input events.
    pub fn track(mut self, content: impl Widget<Message, Theme, Renderer> + 'a) -> Self {
        self.track = Some(content._boxed());
        self
    }
    /// Uses the same styling catalog as a slider without changing slider input.
    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self
    where
        Theme::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        self.class = (Box::new(style) as StyleFn<'a, Theme>).into();
        self
    }
    /// Sets the style class.
    #[cfg(feature = "advanced")]
    pub fn class(mut self, class: impl Into<Theme::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }
    fn active(&self) -> bool {
        self.enabled
            && self.range.start().is_finite()
            && self.range.end().is_finite()
            && self.range.end() > self.range.start()
            && self.step.is_finite()
            && self.shift_step.is_finite()
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
    anchor: f32,
    cancel_pending: bool,
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
        self.cancel_pending |= self.pointer.take().is_some() || self.hovered;
        self.hovered = false;
        self.suppressed = true;
    }
}

impl State {
    fn update(
        &mut self,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        value: f64,
        range: &RangeInclusive<f64>,
        (step, shift_step): (f64, f64),
    ) -> Option<Action> {
        let locate = |x: f32| {
            let fraction = f64::from(x - bounds.x) / f64::from(bounds.width.max(1.0));
            (range.start() + fraction * (range.end() - range.start()))
                .clamp(*range.start(), *range.end())
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
                let step = if modifiers.shift() { shift_step } else { step };
                let target = match key {
                    keyboard::Key::Named(Named::ArrowLeft | Named::ArrowDown) => value - step,
                    keyboard::Key::Named(Named::ArrowRight | Named::ArrowUp) => value + step,
                    keyboard::Key::Named(Named::Home) => *range.start(),
                    keyboard::Key::Named(Named::End) => *range.end(),
                    _ => return None,
                }
                .clamp(*range.start(), *range.end());
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

impl<Message, Theme: Catalog, Renderer: renderer::Renderer> Meta
    for Timeline<'_, Message, Theme, Renderer>
{
}
impl<Message, Theme: Catalog, Renderer: renderer::Renderer> Widget<Message, Theme, Renderer>
    for Timeline<'_, Message, Theme, Renderer>
{
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
        let state = tree.state.downcast_mut::<State>();
        if state.generation != self.generation || !self.active() {
            *state = State {
                focused: state.generation == self.generation && state.focused,
                generation: self.generation,
                cancel_pending: state.cancel_pending || state.pointer.is_some() || state.hovered,
                ..State::default()
            };
        }
        if tree.children.len() != 2 {
            tree.children = vec![Tree::empty(), Tree::empty()];
        }
        for (content, child) in [&mut self.track, &mut self.preview]
            .into_iter()
            .zip(&mut tree.children)
        {
            if let Some(content) = content {
                child.diff(content);
            } else {
                *child = Tree::empty();
            }
        }
    }
    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Fixed(self.height))
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        tree.size = layout::atomic(limits, self.width, self.height);
        if let Some(track) = &mut self.track {
            track.layout(
                &mut tree.children[0],
                renderer,
                &layout::Limits::new(tree.size, tree.size),
            );
        }
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        _: &Rectangle,
        _: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();
        if self.active() || state.focused {
            operation.focusable(None, layout.bounds(), state);
        }
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        _: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        if std::mem::take(&mut state.cancel_pending) {
            shell.publish((self.on_action)(Action::Cancel));
            shell.request_redraw();
        }
        if !self.active() {
            let outside_press = match event {
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => cursor
                    .position()
                    .is_some_and(|position| !layout.bounds().contains(position)),
                Event::Touch(touch::Event::FingerPressed { position, .. }) => {
                    !layout.bounds().contains(*position)
                }
                _ => false,
            };
            if outside_press || matches!(event, Event::Window(window::Event::Unfocused)) {
                state.unfocus();
            }
            if state.focused
                && let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event
                && !modifiers.control()
                && !modifiers.alt()
                && !modifiers.logo()
                && matches!(
                    key,
                    keyboard::Key::Named(
                        keyboard::key::Named::ArrowLeft
                            | keyboard::key::Named::ArrowRight
                            | keyboard::key::Named::ArrowUp
                            | keyboard::key::Named::ArrowDown
                            | keyboard::key::Named::Home
                            | keyboard::key::Named::End
                    )
                )
            {
                // Busy focus still owns these keys; do not fall through to global seeks.
                shell.capture_event();
            }
            return;
        }
        let bounds = layout.bounds();
        let visible = bounds.intersection(viewport);
        // Do not admit pointer input in clipped-away portions of a rail.
        let cursor = if cursor
            .position()
            .is_some_and(|p| visible.is_none_or(|v| !v.contains(p)))
        {
            mouse::Cursor::Unavailable
        } else {
            cursor
        };
        let outside_viewport = match event {
            Event::Mouse(mouse::Event::CursorMoved { position })
            | Event::Touch(touch::Event::FingerMoved { position, .. })
            | Event::Touch(touch::Event::FingerPressed { position, .. })
            | Event::Touch(touch::Event::FingerLifted { position, .. }) => {
                visible.is_none_or(|v| !v.contains(*position))
            }
            _ => false,
        };
        let input_bounds = if outside_viewport {
            Rectangle::new(Point::new(f32::INFINITY, f32::INFINITY), Size::ZERO)
        } else {
            bounds
        };
        if let Some(action) = state.update(
            event,
            input_bounds,
            cursor,
            self.value,
            &self.range,
            (self.step, self.shift_step),
        ) {
            match action {
                Action::Commit(value) | Action::Preview(value) => self.value = value,
                Action::Cancel => self.value = state.origin,
                Action::Hover(_) => {}
            }
            if let Action::Preview(value) | Action::Hover(Some(value)) = action {
                state.anchor =
                    ((value - self.range.start()) / (self.range.end() - self.range.start())) as f32;
            }
            shell.publish((self.on_action)(action));
            if !matches!(
                event,
                Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(keyboard::key::Named::Tab),
                    ..
                })
            ) {
                shell.capture_event();
            }
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
        if let Some(track) = &self.track {
            track.draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout,
                cursor,
                viewport,
            );
        } else {
            let state = tree.state.downcast_ref::<State>();
            let status = if self.active() && state.pointer.is_some() {
                Status::Dragged
            } else if self.active() && (state.focused || cursor.is_over(layout.bounds())) {
                Status::Hovered
            } else {
                Status::Active
            };
            slider::draw(
                renderer,
                layout.bounds(),
                self.value as f32,
                *self.range.start() as f32,
                *self.range.end() as f32,
                theme.style(&self.class, status),
            );
        }
    }
    fn mouse_interaction(
        &self,
        _: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        _: &Renderer,
    ) -> mouse::Interaction {
        if self.active() && cursor.is_over(layout.bounds()) && cursor.is_over(*viewport) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        }
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<overlay::Element<'b, Message, Theme, Renderer>> {
        let state = tree.state.downcast_ref::<State>();
        if !self.active()
            || !state.hovered
            || state.suppressed
            || layout.bounds().intersection(viewport).is_none()
        {
            return Vec::new();
        }
        let Some(content) = &mut self.preview else {
            return Vec::new();
        };
        let child = &mut tree.children[1];
        content.layout(child, renderer, &layout::Limits::new(Size::ZERO, window));
        let bounds = layout.bounds() + translation;
        let position = preview_position(bounds, state.anchor, child.size, window, self.gap);
        let layout = Layout::new(child.size).move_to(position);
        vec![overlay::Element::new(Box::new(PreviewOverlay {
            content,
            tree: child,
            layout,
            window,
        }))]
    }
}

fn preview_position(rail: Rectangle, anchor: f32, size: Size, window: Size, gap: f32) -> Point {
    let x = rail.x + rail.width * anchor - size.width / 2.0;
    let above = rail.y - gap - size.height;
    let y = if above >= 0.0 {
        above
    } else {
        rail.y + rail.height + gap
    };
    Point::new(
        x.clamp(0.0, (window.width - size.width).max(0.0)),
        y.clamp(0.0, (window.height - size.height).max(0.0)),
    )
}

struct PreviewOverlay<'a, 'b, Message, Theme, Renderer> {
    content: &'b mut Element<'a, Message, Theme, Renderer>,
    tree: &'b mut Tree,
    layout: Layout,
    window: Size,
}
impl<Message, Theme, Renderer: renderer::Renderer> overlay::Overlay<Message, Theme, Renderer>
    for PreviewOverlay<'_, '_, Message, Theme, Renderer>
{
    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        _: mouse::Cursor,
    ) {
        let viewport = Rectangle::with_size(self.window);
        renderer.with_layer(viewport, |renderer| {
            self.content.draw(
                self.tree,
                renderer,
                theme,
                style,
                self.layout,
                mouse::Cursor::Unavailable,
                &viewport,
            );
        });
    }
    fn update(
        &mut self,
        event: &Event,
        _: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
    ) {
        if matches!(event, Event::Window(window::Event::RedrawRequested(_))) {
            self.content.update(
                self.tree,
                event,
                self.layout,
                mouse::Cursor::Unavailable,
                renderer,
                shell,
                &Rectangle::with_size(self.window),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Point, Theme};

    fn replay(
        widget: &mut Timeline<'_, Action, Theme, ()>,
        tree: &mut Tree,
        event: Event,
        x: f32,
    ) -> Vec<Action> {
        replay_with_status(widget, tree, event, x).0
    }

    fn replay_with_status(
        widget: &mut Timeline<'_, Action, Theme, ()>,
        tree: &mut Tree,
        event: Event,
        x: f32,
    ) -> (Vec<Action>, crate::core::event::Status) {
        use crate::core::shell::{Bus, Waker};
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
        let status = shell.event_status();
        (bus.into_iter().collect(), status)
    }

    #[test]
    fn widget_hover_exit_and_media_change_invalidate_preview_and_drag() {
        let mut widget = Timeline::new(0.0..=100.0, 4.0, |action| action)
            .step(5.0)
            .shift_step(1.0);
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
        let _ = replay(
            &mut widget,
            &mut tree,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            30.0,
        );
        widget = Timeline::new(0.0..=100.0, 0.0, |action| action).generation(1);
        Widget::<Action, Theme, ()>::diff(&mut widget, &mut tree);
        assert_eq!(
            replay(
                &mut widget,
                &mut tree,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                70.0
            ),
            vec![Action::Cancel]
        );
    }

    #[test]
    fn widget_replay_commits_only_release_and_accumulates_keyboard_batch() {
        let mut widget = Timeline::new(0.0..=100.0, 4.0, |action| action)
            .step(5.0)
            .shift_step(1.0);
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
            &(0.0..=10.0),
            (5.0, 1.0),
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
        let _ = input(
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
        let _ = input(&mut state, event(1, Point::new(50.0, 20.0)), 50.0);
        let _ = input(&mut state, event(2, Point::new(120.0, 20.0)), 120.0);
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

    #[test]
    fn nonzero_range_and_disabled_gesture_do_not_commit_stale_targets() {
        let mut widget = Timeline::new(10.0..=110.0, 40.0, |action| action).step(2.5);
        let mut tree = Tree::new::<Action, Theme, ()>(&widget);
        assert_eq!(
            replay(
                &mut widget,
                &mut tree,
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                25.0
            ),
            vec![Action::Preview(35.0)]
        );
        widget = widget.enabled(false);
        tree.diff(&mut widget);
        assert_eq!(
            replay(
                &mut widget,
                &mut tree,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                80.0
            ),
            vec![Action::Cancel]
        );
        widget = widget.enabled(true);
        tree.diff(&mut widget);
        assert!(
            replay(
                &mut widget,
                &mut tree,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                80.0
            )
            .is_empty()
        );
        tree.state.downcast_mut::<State>().focus();
        assert_eq!(
            replay(
                &mut widget,
                &mut tree,
                key(keyboard::key::Named::Home),
                150.0
            ),
            vec![Action::Commit(10.0)]
        );
        assert_eq!(
            replay(
                &mut widget,
                &mut tree,
                key(keyboard::key::Named::ArrowRight),
                150.0
            ),
            vec![Action::Commit(12.5)]
        );
    }

    #[test]
    fn busy_disable_preserves_keyboard_focus_without_admitting_input() {
        use keyboard::key::Named;
        let mut widget = Timeline::new(0.0..=20.0, 4.0, |action| action)
            .step(2.0)
            .shift_step(0.5);
        let mut tree = Tree::new::<Action, Theme, ()>(&widget);
        assert_eq!(
            replay(
                &mut widget,
                &mut tree,
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                55.0
            ),
            vec![Action::Preview(11.0)]
        );
        assert_eq!(
            replay(
                &mut widget,
                &mut tree,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                55.0
            ),
            vec![Action::Commit(11.0)]
        );
        widget = widget.enabled(false);
        tree.diff(&mut widget);
        tree.diff(&mut widget);
        assert_eq!(
            replay_with_status(&mut widget, &mut tree, key(Named::ArrowLeft), 55.0),
            (vec![Action::Cancel], crate::core::event::Status::Captured)
        );
        for named in [
            Named::ArrowLeft,
            Named::ArrowRight,
            Named::ArrowUp,
            Named::ArrowDown,
            Named::Home,
            Named::End,
        ] {
            for modifiers in [keyboard::Modifiers::empty(), keyboard::Modifiers::SHIFT] {
                let mut event = key(named);
                if let Event::Keyboard(keyboard::Event::KeyPressed {
                    modifiers: actual, ..
                }) = &mut event
                {
                    *actual = modifiers;
                }
                assert_eq!(
                    replay_with_status(&mut widget, &mut tree, event, 150.0),
                    (vec![], crate::core::event::Status::Captured)
                );
            }
        }
        for (named, modifiers) in [
            (Named::Tab, keyboard::Modifiers::empty()),
            (Named::Tab, keyboard::Modifiers::SHIFT),
            (Named::Escape, keyboard::Modifiers::empty()),
            (Named::Space, keyboard::Modifiers::empty()),
            (Named::ArrowLeft, keyboard::Modifiers::CTRL),
            (Named::Home, keyboard::Modifiers::ALT),
            (Named::End, keyboard::Modifiers::LOGO),
        ] {
            let mut event = key(named);
            if let Event::Keyboard(keyboard::Event::KeyPressed {
                modifiers: actual, ..
            }) = &mut event
            {
                *actual = modifiers;
            }
            assert_eq!(
                replay_with_status(&mut widget, &mut tree, event, 150.0),
                (vec![], crate::core::event::Status::Ignored)
            );
        }
        for event in [
            key(Named::Home),
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(80.0, 20.0),
            }),
        ] {
            assert!(replay(&mut widget, &mut tree, event, 80.0).is_empty());
        }
        widget = widget.enabled(true);
        tree.diff(&mut widget);
        assert!(
            replay(
                &mut widget,
                &mut tree,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                80.0
            )
            .is_empty()
        );
        let mut shift_left = key(Named::ArrowLeft);
        if let Event::Keyboard(keyboard::Event::KeyPressed { modifiers, .. }) = &mut shift_left {
            *modifiers = keyboard::Modifiers::SHIFT;
        }
        for (event, target) in [
            (key(Named::ArrowLeft), 9.0),
            (key(Named::ArrowRight), 11.0),
            (shift_left, 10.5),
            (key(Named::Home), 0.0),
        ] {
            assert_eq!(
                replay(&mut widget, &mut tree, event, 150.0),
                vec![Action::Commit(target)]
            );
            widget = widget.enabled(false);
            tree.diff(&mut widget);
            assert!(replay(&mut widget, &mut tree, key(Named::ArrowRight), 150.0).is_empty());
            widget = widget.enabled(true);
            tree.diff(&mut widget);
        }
    }

    #[test]
    fn disabled_focus_is_retired_by_generation_window_or_operation() {
        for retirement in 0..4 {
            let mut widget = Timeline::new(0.0..=100.0, 4.0, |action| action);
            let mut tree = Tree::new::<Action, Theme, ()>(&widget);
            let _ = replay(
                &mut widget,
                &mut tree,
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                30.0,
            );
            widget = widget.enabled(false);
            tree.diff(&mut widget);
            assert_eq!(
                replay(
                    &mut widget,
                    &mut tree,
                    Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                    30.0
                ),
                vec![Action::Cancel]
            );
            match retirement {
                0 => widget = widget.generation(1),
                1 => assert!(
                    replay(
                        &mut widget,
                        &mut tree,
                        Event::Window(window::Event::Unfocused),
                        30.0
                    )
                    .is_empty()
                ),
                2 => {
                    let layout = Layout::new(Size::new(100.0, 40.0));
                    widget.operate(
                        &mut tree,
                        layout,
                        &layout.bounds(),
                        &(),
                        &mut crate::core::widget::operation::focusable::unfocus(),
                    );
                }
                _ => assert!(
                    replay(
                        &mut widget,
                        &mut tree,
                        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                        150.0
                    )
                    .is_empty()
                ),
            }
            tree.diff(&mut widget);
            assert_eq!(
                replay_with_status(
                    &mut widget,
                    &mut tree,
                    key(keyboard::key::Named::ArrowRight),
                    30.0
                ),
                (vec![], crate::core::event::Status::Ignored)
            );
            widget = widget.enabled(true);
            tree.diff(&mut widget);
            assert!(
                replay(
                    &mut widget,
                    &mut tree,
                    key(keyboard::key::Named::ArrowRight),
                    30.0
                )
                .is_empty()
            );
        }
    }

    struct PreviewBounds<'a>(&'a std::cell::Cell<Rectangle>);
    impl Meta for PreviewBounds<'_> {}
    impl Widget<Action, Theme, ()> for PreviewBounds<'_> {
        fn size(&self) -> Size<Length> {
            Size::new(80.into(), 20.into())
        }
        fn layout(&mut self, tree: &mut Tree, _: &(), limits: &layout::Limits) {
            tree.size = layout::atomic(limits, 80, 20);
        }
        fn draw(
            &self,
            _: &Tree,
            _: &mut (),
            _: &Theme,
            _: &renderer::Style,
            layout: Layout,
            _: mouse::Cursor,
            _: &Rectangle,
        ) {
            self.0.set(layout.bounds());
        }
    }

    #[test]
    fn caller_preview_draws_inside_window_and_disappears_on_cancel() {
        let drawn = std::cell::Cell::new(Rectangle::default());
        let mut widget =
            Timeline::new(0.0..=100.0, 10.0, |action| action).preview(PreviewBounds(&drawn));
        let mut tree = Tree::new::<Action, Theme, ()>(&widget);
        tree.diff(&mut widget);
        let window = Size::new(120.0, 100.0);
        for (x, expected_x, rail_width) in
            [(0.0, 0.0, 100.0), (99.0, 40.0, 100.0), (99.0, 21.5, 50.0)]
        {
            let _ = replay(
                &mut widget,
                &mut tree,
                Event::Mouse(mouse::Event::CursorMoved {
                    position: Point::new(x, 20.0),
                }),
                x,
            );
            let rail = Layout::new(Size::new(rail_width, 40.0)).move_to(Point::new(5.0, 50.0));
            let overlays = widget.overlay(
                &mut tree,
                rail,
                &(),
                &Rectangle::with_size(window),
                Vector::new(7.0, 0.0),
                window,
            );
            overlays[0].as_overlay().draw(
                &mut (),
                &Theme::Dark,
                &renderer::Style::default(),
                mouse::Cursor::Unavailable,
            );
            assert_eq!(
                drawn.get(),
                Rectangle::new(Point::new(expected_x, 22.0), Size::new(80.0, 20.0))
            );
        }
        let rail = Layout::new(Size::new(100.0, 40.0)).move_to(Point::new(5.0, 1.0));
        {
            let overlays = widget.overlay(
                &mut tree,
                rail,
                &(),
                &Rectangle::with_size(window),
                Vector::ZERO,
                window,
            );
            overlays[0].as_overlay().draw(
                &mut (),
                &Theme::Dark,
                &renderer::Style::default(),
                mouse::Cursor::Unavailable,
            );
            assert_eq!(drawn.get().y, 49.0);
        }
        let _ = replay(
            &mut widget,
            &mut tree,
            key(keyboard::key::Named::Escape),
            50.0,
        );
        assert!(
            widget
                .overlay(
                    &mut tree,
                    rail,
                    &(),
                    &Rectangle::with_size(window),
                    Vector::ZERO,
                    window
                )
                .is_empty()
        );
    }
}
