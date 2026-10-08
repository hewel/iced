use crate::runtime::user_interface::{Change, State};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    Settled,
    RetryOverlay,
    Continue,
    LimitReached,
}

pub(crate) fn decide(redraw_count: usize, state: &State, messages_changed: bool) -> Decision {
    // A third pass may only confirm that the preceding invalidations settled.
    // Count another invalidation only if this update still changes the UI.
    if !messages_changed
        && matches!(
            state,
            State::Updated {
                change: Change::None,
                ..
            }
        )
    {
        return Decision::Settled;
    }

    if redraw_count >= 2 {
        return Decision::LimitReached;
    }

    if !messages_changed
        && matches!(
            state,
            State::Updated {
                change: Change::Overlay,
                ..
            }
        )
    {
        return Decision::RetryOverlay;
    }

    Decision::Continue
}

#[cfg(all(test, debug_assertions))]
mod tests;
