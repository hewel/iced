//! Unsupported local backdrops preserve the player's existing scrim and content.
#![cfg(feature = "tiny-skia")]

use iced::widget::{Space, container, stack};
use iced::{Color, Element, Fill, Size};
use iced_core::renderer::{Headless, Renderer as _};

#[path = "support/player_backdrop.rs"]
mod composition;

#[derive(Debug, Clone, Copy)]
enum Controls {
    Minimal,
    Full,
    Hidden,
}

fn solid(
    color: Color,
    width: impl Into<iced::Length>,
    height: impl Into<iced::Length>,
) -> Element<'static, ()> {
    container(Space::new().width(width).height(height))
        .style(move |_| iced::widget::container::Style::default().background(color))
        .into()
}

fn player(state: Controls, with_backdrop: bool) -> Element<'static, ()> {
    let video = solid(Color::from_rgb(0.8, 0.6, 0.2), Fill, Fill);
    let controls = match state {
        Controls::Minimal => Some(solid(Color::from_rgb(0.0, 1.0, 0.0), 8, 8)),
        Controls::Full => Some(
            stack![
                // The scrim is original content, so unsupported effects cannot remove it.
                solid(Color::from_rgba(0.0, 0.0, 0.0, 0.5), Fill, 32),
                solid(Color::from_rgb(0.0, 1.0, 0.0), 8, 8),
            ]
            .width(Fill)
            .height(32)
            .into(),
        ),
        Controls::Hidden => None,
    };
    if with_backdrop {
        return composition::player_layers(
            video,
            controls,
            None,
            matches!(state, Controls::Full),
            32.0,
        );
    }
    // The same original placement without the newly introduced effect wrapper.
    let mut layers = stack![video].width(Fill).height(Fill);
    if let Some(controls) = controls {
        layers = if matches!(state, Controls::Full) {
            layers.push(
                container(container(controls).width(Fill).align_bottom(32))
                    .width(Fill)
                    .align_bottom(Fill),
            )
        } else {
            layers.push(controls)
        };
    }
    layers.into()
}

fn draw(renderer: &mut iced::Renderer, element: Element<'_, ()>) -> Vec<u8> {
    let mut ui = iced_runtime::UserInterface::build(
        element,
        Size::new(128.0, 128.0),
        Default::default(),
        renderer,
    );
    let _ = ui.draw(
        renderer,
        &iced::Theme::Dark,
        &iced_core::renderer::Style {
            text_color: Color::WHITE,
        },
        iced_core::mouse::Cursor::Unavailable,
    );
    renderer.screenshot(Size::new(128, 128), 1.0, Color::TRANSPARENT)
}

fn pixel(bytes: &[u8], x: usize, y: usize) -> [u8; 4] {
    bytes[(y * 128 + x) * 4..(y * 128 + x + 1) * 4]
        .try_into()
        .unwrap()
}

#[test]
fn unsupported_backdrop_preserves_original_scrim_through_full_minimal_and_hidden_states() {
    let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
        Default::default(),
        Some("tiny-skia"),
    ))
    .expect("software renderer must be available");
    assert_eq!(renderer.name(), "tiny-skia");
    for state in [
        Controls::Full,
        Controls::Minimal,
        Controls::Full,
        Controls::Hidden,
    ] {
        let expected = draw(&mut renderer, player(state, false));
        let actual = draw(&mut renderer, player(state, true));
        assert_eq!(
            actual, expected,
            "fallback changed original content in {state:?}"
        );
        match state {
            Controls::Full => {
                assert_eq!(pixel(&actual, 4, 100), [0, 255, 0, 255]);
                assert!(pixel(&actual, 64, 112)[0] < pixel(&actual, 64, 64)[0]);
            }
            Controls::Minimal => {
                assert_eq!(pixel(&actual, 4, 4), [0, 255, 0, 255]);
                assert_eq!(pixel(&actual, 64, 112), pixel(&actual, 64, 64));
            }
            Controls::Hidden => assert_eq!(pixel(&actual, 4, 4), pixel(&actual, 64, 112)),
        }
        assert_eq!(
            renderer.blur_statistics(),
            Default::default(),
            "unsupported local blur must not allocate or sample a scene filter in {state:?}"
        );
    }
}
