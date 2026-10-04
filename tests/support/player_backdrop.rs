//! Compiled integration recipe. The caller owns controls, visibility, and popovers.
use iced::widget::{backdrop, container, stack};
use iced::{Blur, Element, Fill, Theme};

pub fn player_layers<'a, Message: 'a, Renderer: iced_core::Renderer + 'a>(
    video: Element<'a, Message, Theme, Renderer>,
    controls: Option<Element<'a, Message, Theme, Renderer>>,
    popover: Option<Element<'a, Message, Theme, Renderer>>,
    full_visible: bool,
    panel_height: f32,
) -> Element<'a, Message, Theme, Renderer> {
    let mut layers = stack![video].width(Fill).height(Fill);

    if let Some(controls) = controls {
        if full_visible {
            // Keep the existing scrim inside controls: it also supplies the
            // fallback when a renderer does not implement draw_backdrop.
            let panel = backdrop(
                Blur::vertical_gradient(0.0, 16.0).range(0.15, 1.0),
                container(controls).width(Fill).align_bottom(panel_height),
            );
            layers = layers.push(container(panel).width(Fill).align_bottom(Fill));
        } else {
            // Minimal controls keep their original placement and draw sharply.
            layers = layers.push(controls);
        }
    }

    if let Some(popover) = popover {
        layers = layers.push(popover);
    }

    layers.into()
}
