//! Experimental glass surfaces with live backdrop optics and interaction feedback.
//!
//! This is iced's composition API, not an emulation of another UI toolkit.
//! Put a glass surface after its background in a [`crate::stack`]. Its child
//! provides layout and interaction; the material only changes drawing.
use crate::core::border;
use crate::core::layout;
use crate::core::mouse;
use crate::core::overlay;
use crate::core::renderer;
use crate::core::time::{Duration, Instant};
use crate::core::touch;
use crate::core::widget::{self, Meta, Operation, Tree, tree};
use crate::core::window;
use crate::core::{
    self, Blur, Color, Event, Layout, Length, Point, Rectangle, Shell, Size, Vector, Widget,
};

pub use crate::core::glass::{Optics, Quality};

/// Creates a glass surface around `content` using [`Style::regular`].
///
/// Background widgets must be drawn first. The live lower scene is sampled
/// before drawing the child, so text and hit areas stay unchanged. The child
/// owns actions: wrap a transparent button or another control to make glass
/// clickable. Glass observes input without capturing it.
///
/// Animations request frames only while their feedback is changing. The
/// background is responsible for requesting frames when its own content changes.
pub fn glass<W>(content: W) -> Glass<W> {
    Glass::new(content)
}

/// The reusable appearance of a [`Glass`] surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// Blur profile in logical pixels, before optional adaptive scaling.
    pub blur: Blur,
    /// Optical material settings, before interaction feedback.
    pub optics: Optics,
    /// Corner radii shared by the material, border, and opaque fallback.
    pub border_radius: border::Radius,
    /// Corner smoothing from `0.0` to `1.0`.
    pub border_smoothing: f32,
    /// Scale blur and edge depth with the shorter content dimension.
    ///
    /// A 100 px surface uses the supplied values; scaling is bounded to
    /// `0.65..=2.4`. Set this to `false` for exact logical-pixel settings.
    pub adaptive: bool,
}

impl Style {
    /// A softly frosted material whose blur and edge depth adapt to its size.
    pub fn regular() -> Self {
        Self {
            blur: Blur::Uniform(8.0),
            optics: Optics {
                refraction: 8.0,
                depth: 18.0,
                light: Vector::new(-0.55, -0.8),
                highlight: 0.5,
                shadow: 0.18,
                tint: Color::from_rgba(0.92, 0.96, 1.0, 0.14),
            },
            border_radius: 16.0.into(),
            border_smoothing: 0.6,
            adaptive: true,
        }
    }

    /// A shallow, mostly clear material with fixed blur and edge depth.
    pub fn clear() -> Self {
        Self {
            blur: Blur::Uniform(2.0),
            optics: Optics {
                refraction: 5.0,
                depth: 12.0,
                highlight: 0.24,
                shadow: 0.10,
                tint: Color::from_rgba(0.92, 0.96, 1.0, 0.025),
                ..Self::regular().optics
            },
            adaptive: false,
            ..Self::regular()
        }
    }

    fn material(self, size: Size, feedback: Feedback) -> (Blur, Optics) {
        let scale = if self.adaptive {
            (size.width.min(size.height) / 100.0).clamp(0.65, 2.4)
        } else {
            1.0
        };
        let mut optics = self.optics;
        optics.depth *= scale;
        optics.refraction *= 1.0 + 0.10 * feedback.hovered - 0.12 * feedback.pressed;
        optics.highlight +=
            0.10 * feedback.hovered + 0.12 * feedback.pressed + 0.16 * feedback.focused;
        optics.light += feedback.light * 0.35;
        (self.blur.normalized().scaled(scale), optics.normalized())
    }
}

impl Default for Style {
    fn default() -> Self {
        Self::regular()
    }
}

/// A glass material behind an unchanged foreground widget.
///
/// Focus feedback is derived from descendants that expose their focus through
/// widget operations. [`Self::focused`] supplies an explicit override for custom
/// widgets. Accessibility preferences are explicit application inputs; this
/// widget does not infer platform settings.
pub struct Glass<W> {
    content: W,
    style: Style,
    quality: Quality,
    interactive: bool,
    reduce_motion: bool,
    reduced_transparency: bool,
    high_contrast: bool,
    focused: Option<bool>,
}

impl<W> Glass<W> {
    /// Creates a regular glass surface around `content`.
    pub fn new(content: W) -> Self {
        Self {
            content,
            style: Style::regular(),
            quality: Quality::Adaptive,
            interactive: true,
            reduce_motion: false,
            reduced_transparency: false,
            high_contrast: false,
            focused: None,
        }
    }

    /// Sets the reusable material appearance.
    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// Sets the blur profile before the style's adaptive scaling.
    pub fn blur(mut self, blur: impl Into<Blur>) -> Self {
        self.style.blur = blur.into();
        self
    }

    /// Sets the optical settings before sizing and interaction feedback.
    pub fn optics(mut self, optics: Optics) -> Self {
        self.style.optics = optics;
        self
    }

    /// Sets the material's corner radii.
    pub fn border_radius(mut self, radius: impl Into<border::Radius>) -> Self {
        self.style.border_radius = radius.into();
        self
    }

    /// Sets corner smoothing, from `0.0` to `1.0`.
    pub fn border_smoothing(mut self, smoothing: f32) -> Self {
        self.style.border_smoothing = smoothing;
        self
    }

    /// Sets the renderer's quality policy for this surface. Defaults to
    /// [`Quality::Adaptive`].
    pub fn quality(mut self, quality: Quality) -> Self {
        self.quality = quality;
        self
    }

    /// Enables hover, press, and pointer-light feedback. Defaults to `true`.
    ///
    /// Disabling this does not disable child controls or their focus indicator.
    pub fn interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }

    /// Applies feedback immediately instead of animating it.
    pub fn reduce_motion(mut self, reduce_motion: bool) -> Self {
        self.reduce_motion = reduce_motion;
        self
    }

    /// Replaces optical effects with an opaque, contrasting surface.
    ///
    /// No backdrop is captured. The fill is chosen for contrast with the
    /// inherited text color; explicitly styled children retain their own colors.
    pub fn reduced_transparency(mut self, reduced_transparency: bool) -> Self {
        self.reduced_transparency = reduced_transparency;
        self
    }

    /// Adds a black-and-white outline visible over light and dark backgrounds.
    pub fn high_contrast(mut self, high_contrast: bool) -> Self {
        self.high_contrast = high_contrast;
        self
    }

    /// Overrides focus feedback, for children with externally managed focus.
    ///
    /// Without this override, focusable descendants are queried automatically.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = Some(focused);
        self
    }
}

impl<W: Meta> Meta for Glass<W> {
    fn is_void(&self) -> bool {
        self.content.is_void()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Feedback {
    hovered: f32,
    pressed: f32,
    focused: f32,
    light: Vector,
}

impl Feedback {
    fn towards(self, target: Self, fraction: f32) -> Self {
        let mix = |from, to| from + (to - from) * fraction;
        Self {
            hovered: mix(self.hovered, target.hovered),
            pressed: mix(self.pressed, target.pressed),
            focused: mix(self.focused, target.focused),
            light: self.light + (target.light - self.light) * fraction,
        }
    }
}

struct Transition {
    from: Feedback,
    started: Instant,
}

struct State {
    hovered: bool,
    mouse_pressed: bool,
    touch_pressed: Option<touch::Finger>,
    pointer: Option<Point>,
    focused: bool,
    window_focused: bool,
    current: Feedback,
    target: Feedback,
    transition: Option<Transition>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            hovered: false,
            mouse_pressed: false,
            touch_pressed: None,
            pointer: None,
            focused: false,
            window_focused: true,
            current: Feedback::default(),
            target: Feedback::default(),
            transition: None,
        }
    }
}

impl State {
    const DURATION: Duration = Duration::from_millis(150);

    fn sample(&mut self, now: Instant) {
        if let Some(transition) = &self.transition {
            let fraction = (now
                .saturating_duration_since(transition.started)
                .as_secs_f32()
                / Self::DURATION.as_secs_f32())
            .min(1.0);
            let eased = fraction * fraction * (3.0 - 2.0 * fraction);
            self.current = transition.from.towards(self.target, eased);
            if fraction >= 1.0 {
                self.current = self.target;
                self.transition = None;
            }
        }
    }

    fn target(&self, bounds: Rectangle, interactive: bool, focused: Option<bool>) -> Feedback {
        let hovered = interactive && self.window_focused && self.hovered;
        let pressed = interactive
            && self.window_focused
            && (self.mouse_pressed || self.touch_pressed.is_some());
        let light = if interactive && self.window_focused {
            self.pointer.map_or(Vector::ZERO, |point| {
                Vector::new(
                    ((point.x - bounds.center_x()) / (bounds.width / 2.0).max(1.0))
                        .clamp(-1.0, 1.0),
                    ((point.y - bounds.center_y()) / (bounds.height / 2.0).max(1.0))
                        .clamp(-1.0, 1.0),
                )
            })
        } else {
            Vector::ZERO
        };
        Feedback {
            hovered: if hovered { 1.0 } else { 0.0 },
            pressed: if pressed { 1.0 } else { 0.0 },
            focused: if self.window_focused && focused.unwrap_or(self.focused) {
                1.0
            } else {
                0.0
            },
            light,
        }
    }

    fn retarget(&mut self, target: Feedback, now: Instant, immediate: bool) -> bool {
        self.sample(now);
        let changed = target != self.target;
        if immediate {
            let changed = self.current != target || self.transition.is_some();
            self.current = target;
            self.target = target;
            self.transition = None;
            return changed;
        }
        if changed {
            self.target = target;
            self.transition = Some(Transition {
                from: self.current,
                started: now,
            });
        }
        changed
    }

    fn observe(&mut self, event: &Event, cursor: mouse::Cursor, visible: Option<Rectangle>) {
        let inside = |point| visible.is_some_and(|bounds| bounds.contains(point));
        match event {
            Event::Window(window::Event::Unfocused | window::Event::Closed) => {
                self.window_focused = false;
                self.clear_pointer();
                return;
            }
            Event::Window(window::Event::Focused) => self.window_focused = true,
            Event::Mouse(mouse::Event::CursorLeft) => {
                self.clear_pointer();
                return;
            }
            Event::Touch(touch::Event::FingerPressed { id, position }) => {
                if self.touch_pressed.is_none() && inside(*position) {
                    self.touch_pressed = Some(*id);
                    self.pointer = Some(*position);
                }
                return;
            }
            Event::Touch(touch::Event::FingerMoved { id, position }) => {
                if self.touch_pressed == Some(*id) {
                    if inside(*position) {
                        self.pointer = Some(*position);
                    } else {
                        self.touch_pressed = None;
                        self.pointer = None;
                    }
                }
                return;
            }
            Event::Touch(
                touch::Event::FingerLifted { id, .. } | touch::Event::FingerLost { id, .. },
            ) => {
                if self.touch_pressed == Some(*id) {
                    self.touch_pressed = None;
                    self.pointer = None;
                }
                return;
            }
            _ => {}
        }
        if self.touch_pressed.is_some() && !self.pointer.is_some_and(inside) {
            self.touch_pressed = None;
            self.pointer = None;
        }
        let position = cursor.position().filter(|&point| inside(point));
        self.hovered = position.is_some();
        if self.touch_pressed.is_none() {
            self.pointer = position;
        }
        if !self.hovered {
            self.mouse_pressed = false;
        }
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if self.hovered => {
                self.mouse_pressed = true;
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                self.mouse_pressed = false;
            }
            _ => {}
        }
    }

    fn clear_pointer(&mut self) {
        self.hovered = false;
        self.mouse_pressed = false;
        self.touch_pressed = None;
        self.pointer = None;
    }
}

#[derive(Default)]
struct FocusQuery(bool);

impl Operation for FocusQuery {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn focusable(
        &mut self,
        _id: Option<&widget::Id>,
        _bounds: Rectangle,
        state: &mut dyn widget::operation::Focusable,
    ) {
        self.0 |= state.is_focused();
    }
}

impl<Message, Theme, Renderer, W> Widget<Message, Theme, Renderer> for Glass<W>
where
    Renderer: core::Renderer,
    W: Widget<Message, Theme, Renderer>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        let child = &mut tree.children[0];
        self.content.layout(child, renderer, limits);
        child.translation = Vector::ZERO;
        tree.size = child.size;
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let Some(visible) = bounds.intersection(viewport) else {
            return;
        };
        let state = tree.state.downcast_ref::<State>();
        let (blur, optics) = self.style.material(bounds.size(), state.current);
        let border = core::Border {
            radius: self.style.border_radius,
            smoothing: self.style.border_smoothing,
            ..Default::default()
        };
        renderer.with_layer(visible, |renderer| {
            if self.reduced_transparency {
                let fill = if defaults.text_color.relative_contrast(Color::WHITE)
                    >= defaults.text_color.relative_contrast(Color::BLACK)
                {
                    Color::WHITE
                } else {
                    Color::BLACK
                };
                renderer.fill_quad(
                    renderer::Quad {
                        bounds,
                        border,
                        snap: false,
                        ..Default::default()
                    },
                    fill,
                );
            } else {
                renderer.draw_backdrop(renderer::Backdrop {
                    bounds,
                    blur,
                    border_radius: self.style.border_radius,
                    border_smoothing: self.style.border_smoothing,
                    optics: Some(optics),
                    quality: self.quality,
                });
            }

            if self.high_contrast {
                for (width, color) in [
                    (3.0 + 2.0 * state.current.focused, Color::BLACK),
                    (1.0 + state.current.focused, Color::WHITE),
                ] {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds,
                            border: core::Border {
                                width,
                                color,
                                ..border
                            },
                            snap: false,
                            ..Default::default()
                        },
                        Color::TRANSPARENT,
                    );
                }
            } else if state.current.focused > 0.0 {
                let mut color = defaults.text_color;
                color.a *= state.current.focused;
                renderer.fill_quad(
                    renderer::Quad {
                        bounds,
                        border: core::Border {
                            width: 2.0,
                            color,
                            ..border
                        },
                        snap: false,
                        ..Default::default()
                    },
                    Color::TRANSPARENT,
                );
            }
            self.content.draw(
                &tree.children[0],
                renderer,
                theme,
                defaults,
                layout,
                cursor,
                viewport,
            );
        });
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .operate(&mut tree.children[0], layout, viewport, renderer, operation);
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
        let child = &mut tree.children[0];
        self.content
            .update(child, event, layout, cursor, renderer, shell, viewport);
        let mut focus = FocusQuery::default();
        self.content
            .operate(child, layout, viewport, renderer, &mut focus);

        let state = tree.state.downcast_mut::<State>();
        state.focused = focus.0;
        state.observe(event, cursor, layout.bounds().intersection(viewport));
        if !self.interactive {
            state.clear_pointer();
        }
        let now = match event {
            Event::Window(window::Event::RedrawRequested(now)) => *now,
            _ => Instant::now(),
        };
        let target = state.target(
            layout.bounds(),
            self.interactive && !self.reduced_transparency,
            self.focused,
        );
        let changed = state.retarget(target, now, self.reduce_motion);
        if state.transition.is_some()
            || (changed && !matches!(event, Event::Window(window::Event::RedrawRequested(_))))
        {
            shell.request_redraw();
        }
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
            .mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<overlay::Element<'a, Message, Theme, Renderer>> {
        self.content.overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
            window,
        )
    }
}

#[cfg(test)]
mod tests;
