use super::{Decision, decide};
use crate::core::widget::{Meta, Tree, Widget, tree};
use crate::core::{Event, Length, Rectangle, Shell, Size, layout, mouse, renderer, shell, window};
use crate::runtime::user_interface::{Cache, Change, State, UserInterface};

#[derive(Clone, Copy)]
enum Probe {
    Measurement(f32),
    Invalidating(shell::Invalidation, Option<usize>),
    Animation,
}

struct ProbeState {
    measured_height: Option<f32>,
    remaining: Option<usize>,
}

impl Meta for Probe {}

impl Widget<f32, (), ()> for Probe {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<ProbeState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(ProbeState {
            measured_height: None,
            remaining: match self {
                Self::Invalidating(_, remaining) => *remaining,
                _ => None,
            },
        })
    }

    fn layout(&mut self, tree: &mut Tree, _renderer: &(), limits: &layout::Limits) {
        let height = match self {
            Self::Measurement(height) => *height,
            _ => 20.0,
        };
        tree.size = layout::atomic(limits, Length::Fill, height);
    }

    fn draw(
        &self,
        _tree: &Tree,
        _renderer: &mut (),
        _theme: &(),
        _style: &renderer::Style,
        _layout: layout::Layout,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: layout::Layout,
        _cursor: mouse::Cursor,
        _renderer: &(),
        shell: &mut Shell<'_, f32>,
        _viewport: &Rectangle,
    ) {
        if !matches!(event, Event::Window(window::Event::RedrawRequested(_))) {
            return;
        }

        let state = tree.state.downcast_mut::<ProbeState>();
        match self {
            Self::Measurement(_) => {
                let height = layout.bounds().height;
                if state.measured_height != Some(height) {
                    state.measured_height = Some(height);
                    shell.publish(height);
                }
            }
            Self::Invalidating(invalidation, _) => {
                if state.remaining != Some(0) {
                    if let Some(remaining) = state.remaining.as_mut() {
                        *remaining -= 1;
                    }
                    shell.invalidate(*invalidation);
                }
            }
            Self::Animation => shell.request_redraw(),
        }
    }
}

struct Pass {
    decision: Decision,
    change: Option<Change>,
    messages: Vec<f32>,
    redraw_request: Option<window::RedrawRequest>,
}

// Drive the production decision with real UI updates. Measurement messages
// rebuild the view through the same Cache; every pass receives the same event.
fn redraw(mut probe: Probe, keep_growing: bool) -> Vec<Pass> {
    let mut renderer = ();
    let mut bus = shell::Bus::new();
    let mut ui = UserInterface::build(
        probe._boxed(),
        Size::new(100.0, 100.0),
        Cache::new(),
        &mut renderer,
    );
    let event = Event::Window(window::Event::RedrawRequested(
        crate::core::time::Instant::now(),
    ));
    let mut passes = Vec::new();

    // A failed guard must fail the test instead of hanging the test process.
    for redraw_count in 0..5 {
        let message_count = bus.len();
        let (state, _) = ui.update(
            &window::Headless,
            &shell::Waker::noop(),
            std::slice::from_ref(&event),
            mouse::Cursor::Unavailable,
            &mut renderer,
            &mut bus,
        );
        let decision = decide(redraw_count, &state, message_count != bus.len());
        let (change, redraw_request) = match &state {
            State::Updated {
                change,
                redraw_request,
                ..
            } => (Some(*change), Some(*redraw_request)),
            State::Outdated => (None, None),
        };

        if matches!(decision, Decision::Settled | Decision::LimitReached) {
            passes.push(Pass {
                decision,
                change,
                messages: bus.drain().map(|(message, _)| message).collect(),
                redraw_request,
            });
            return passes;
        }

        let messages: Vec<_> = bus.drain().map(|(message, _)| message).collect();
        if !messages.is_empty() || matches!(state, State::Outdated) {
            for height in &messages {
                probe = Probe::Measurement(if keep_growing { height + 20.0 } else { 40.0 });
            }
            ui = UserInterface::build(
                probe._boxed(),
                Size::new(100.0, 100.0),
                ui.into_cache(),
                &mut renderer,
            );
        }
        passes.push(Pass {
            decision,
            change,
            messages,
            redraw_request,
        });
    }
    panic!("redraw invalidation did not stop at its limit");
}

#[test]
fn measurement_messages_rebuild_twice_then_settle_without_a_warning() {
    let passes = redraw(Probe::Measurement(20.0), false);
    assert_eq!(passes.len(), 3);
    assert_eq!(passes[0].messages, [20.0]);
    assert_eq!(passes[1].messages, [40.0]);
    assert!(passes[2].messages.is_empty());
    assert!(passes.iter().all(|pass| pass.change == Some(Change::None)));
    assert_eq!(passes[0].decision, Decision::Continue);
    assert_eq!(passes[1].decision, Decision::Continue);
    assert_eq!(passes[2].decision, Decision::Settled);
}

#[test]
fn messages_still_reach_the_limit_even_without_layout_invalidation() {
    let passes = redraw(Probe::Measurement(20.0), true);
    assert_eq!(passes.len(), 3);
    assert_eq!(passes[2].messages, [60.0]);
    assert_eq!(passes[2].change, Some(Change::None));
    assert_eq!(passes[2].decision, Decision::LimitReached);
}

#[test]
fn continuous_layout_invalidation_reaches_the_limit() {
    let passes = redraw(
        Probe::Invalidating(shell::Invalidation::Layout(shell::Diff::Skip), None),
        false,
    );
    assert_eq!(passes.len(), 3);
    assert!(
        passes
            .iter()
            .all(|pass| pass.change == Some(Change::Layout))
    );
    assert_eq!(passes[2].decision, Decision::LimitReached);
}

#[test]
fn continuous_overlay_invalidation_cannot_bypass_the_limit() {
    let passes = redraw(
        Probe::Invalidating(shell::Invalidation::Overlay, None),
        false,
    );
    assert_eq!(passes.len(), 3);
    assert!(
        passes
            .iter()
            .all(|pass| pass.change == Some(Change::Overlay))
    );
    assert_eq!(passes[0].decision, Decision::RetryOverlay);
    assert_eq!(passes[1].decision, Decision::RetryOverlay);
    assert_eq!(passes[2].decision, Decision::LimitReached);
}

#[test]
fn continuous_widget_rebuilds_reach_the_limit() {
    let passes = redraw(
        Probe::Invalidating(shell::Invalidation::Widgets, None),
        false,
    );
    assert_eq!(passes.len(), 3);
    assert!(passes.iter().all(|pass| pass.change.is_none()));
    assert_eq!(passes[2].decision, Decision::LimitReached);
}

#[test]
fn two_overlay_invalidations_can_settle_on_the_third_pass() {
    let passes = redraw(
        Probe::Invalidating(shell::Invalidation::Overlay, Some(2)),
        false,
    );
    assert_eq!(passes.len(), 3);
    assert_eq!(passes[1].decision, Decision::RetryOverlay);
    assert_eq!(passes[2].change, Some(Change::None));
    assert_eq!(passes[2].decision, Decision::Settled);
}

#[test]
fn two_layout_invalidations_can_settle_on_the_third_pass() {
    let passes = redraw(
        Probe::Invalidating(shell::Invalidation::Layout(shell::Diff::Skip), Some(2)),
        false,
    );
    assert_eq!(passes.len(), 3);
    assert_eq!(passes[1].change, Some(Change::Layout));
    assert_eq!(passes[2].change, Some(Change::None));
    assert_eq!(passes[2].decision, Decision::Settled);
}

#[test]
fn next_frame_request_is_not_an_invalidation() {
    let passes = redraw(Probe::Animation, false);
    assert_eq!(passes.len(), 1);
    assert_eq!(passes[0].decision, Decision::Settled);
    assert_eq!(
        passes[0].redraw_request,
        Some(window::RedrawRequest::NextFrame)
    );
}
