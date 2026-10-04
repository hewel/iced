//! Live scene blur for existing modal compositions.
use crate::core::Blur;
use crate::{Backdrop, backdrop};

/// Blurs the lower scene before drawing a modal's content and dimming layer.
///
/// Compose this as a later child of [`crate::stack`]. The content should include
/// your full-window scrim, dismissal handling, and dialog. Layout, events,
/// operations, and overlays are delegated unchanged; later toasts remain crisp.
/// Lower-scene changes remain live on every rendered frame.
///
/// This is a convenience constructor for [`Backdrop`] with a rectangular mask
/// and no extra tint. It accepts a uniform sigma in logical pixels or a
/// progressive [`Blur`] profile following the complete modal bounds.
///
/// ```no_run
/// use iced_widget::{button, center, modal, opaque, stack, text, Widget};
/// let content: iced_widget::core::Element<'_, (), iced_widget::Theme, iced_widget::Renderer> =
///     stack![text("Live background"), modal(12.0, opaque(center(button("Dialog"))))].boxed();
/// ```
pub fn modal<W>(blur: impl Into<Blur>, content: W) -> Backdrop<W> {
    backdrop(blur, content)
}
