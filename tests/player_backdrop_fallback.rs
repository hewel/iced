//! Numeric local-backdrop contracts for the supported GPU renderer.
#![cfg(feature = "wgpu")]

use iced_core::renderer::{Backdrop, Headless, Quad, Renderer as _};
use iced_core::{Blur, Color, Rectangle, Size, Transformation, Vector};

fn renderer() -> iced::Renderer {
    let backend = std::env::var("ICED_TEST_BACKEND").unwrap_or_else(|_| "wgpu".into());
    let renderer =
        iced::futures::executor::block_on(iced::Renderer::new(Default::default(), Some(&backend)))
            .expect("requested renderer must be available");
    assert_eq!(renderer.name(), backend, "no silent backend fallback");
    renderer
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rectangle {
    Rectangle {
        x,
        y,
        width,
        height,
    }
}

fn pixel(bytes: &[u8], x: usize, y: usize) -> [u8; 4] {
    bytes[(y * 128 + x) * 4..(y * 128 + x + 1) * 4]
        .try_into()
        .unwrap()
}

fn background(renderer: &mut iced::Renderer, blue: bool, alpha: f32) {
    renderer.reset(rect(0.0, 0.0, 128.0, 128.0));
    for (bounds, color) in [
        (
            rect(0.0, 0.0, 64.0, 128.0),
            Color::from_rgba(1.0, 0.0, 0.0, alpha),
        ),
        (
            rect(64.0, 0.0, 64.0, 128.0),
            if blue {
                Color::from_rgba(0.0, 0.0, 1.0, alpha)
            } else {
                Color::from_rgba(0.0, 1.0, 0.0, alpha)
            },
        ),
    ] {
        renderer.fill_quad(
            Quad {
                bounds,
                ..Default::default()
            },
            color,
        );
    }
}

fn effect(blur: Blur) -> Backdrop {
    Backdrop {
        bounds: rect(48.0, 16.0, 32.0, 96.0),
        blur,
        border_radius: 12.0.into(),
        border_smoothing: 0.6,
    }
}

fn read(renderer: &mut iced::Renderer) -> Vec<u8> {
    renderer.screenshot(Size::new(128, 128), 1.0, Color::TRANSPARENT)
}

#[test]
fn rounded_backdrop_preserves_alpha_and_uses_full_profile_coordinates_when_clipped() {
    let mut renderer = renderer();
    let profile = Blur::vertical_gradient(0.0, 12.0).range(0.2, 0.8);
    background(&mut renderer, true, 0.5);
    let sharp = read(&mut renderer);
    background(&mut renderer, true, 0.5);
    renderer.draw_backdrop(effect(profile));
    let full = read(&mut renderer);
    for (x, y) in [(46, 88), (81, 88), (48, 16), (79, 111), (60, 22)] {
        assert_eq!(
            pixel(&full, x, y),
            pixel(&sharp, x, y),
            "outside mask or sharp plateau at {x},{y}"
        );
    }
    assert!(
        pixel(&full, 60, 92)[2] > 15,
        "the interior must contain filtered neighboring blue pixels"
    );
    for y in 0..128 {
        for x in 0..128 {
            assert!(
                pixel(&full, x, y)[3].abs_diff(pixel(&sharp, x, y)[3]) <= 1,
                "backdrop replacement must preserve lower-scene alpha at {x},{y}"
            );
        }
    }
    background(&mut renderer, true, 0.5);
    renderer.with_layer(rect(58.0, 56.0, 12.0, 42.0), |renderer| {
        renderer.draw_backdrop(effect(profile));
    });
    let clipped = read(&mut renderer);
    for y in 56..98 {
        for x in 58..70 {
            assert_eq!(
                pixel(&clipped, x, y),
                pixel(&full, x, y),
                "clipping changed the source halo or gradient coordinates at {x},{y}"
            );
        }
    }
    assert_eq!(pixel(&clipped, 60, 100), pixel(&sharp, 60, 100));
}

#[test]
fn live_backdrop_preserves_foreground_order_and_disappears_with_its_marker() {
    let mut renderer = renderer();
    let mut render = |blue, foreground, enabled| {
        background(&mut renderer, blue, 1.0);
        if enabled {
            renderer.draw_backdrop(effect(Blur::vertical_gradient(2.0, 12.0)));
        }
        renderer.fill_quad(
            Quad {
                bounds: rect(56.0, 72.0, 16.0, 12.0),
                ..Default::default()
            },
            foreground,
        );
        let pixels = read(&mut renderer);
        (pixels, renderer.blur_statistics())
    };
    let (first, cold) = render(true, Color::WHITE, true);
    assert_eq!(pixel(&first, 60, 78), [255; 4]);
    let (foreground_changed, warm) = render(true, Color::BLACK, true);
    assert_eq!(pixel(&foreground_changed, 60, 78), [0, 0, 0, 255]);
    assert_eq!(pixel(&first, 60, 94), pixel(&foreground_changed, 60, 94));
    assert_eq!(cold.scene_misses, warm.scene_misses);
    assert!(warm.scene_hits > cold.scene_hits);

    let (background_changed, changed) = render(false, Color::WHITE, true);
    assert_ne!(pixel(&first, 60, 94), pixel(&background_changed, 60, 94));
    assert!(changed.scene_misses > warm.scene_misses);
    let (hidden, removed) = render(true, Color::WHITE, false);
    assert_eq!(pixel(&hidden, 60, 94), [255, 0, 0, 255]);
    assert_eq!(pixel(&hidden, 60, 78), [255; 4]);
    assert_eq!(removed.scene_misses, changed.scene_misses);
    assert_eq!(removed.scene_hits, changed.scene_hits);
}

#[test]
fn backdrop_profile_and_rounded_mask_follow_scale_and_translation_once() {
    let mut renderer = renderer();
    let profile = Blur::horizontal_gradient(0.0, 10.0);
    background(&mut renderer, true, 1.0);
    renderer.with_translation(Vector::new(16.0, 8.0), |renderer| {
        renderer.with_transformation(Transformation::scale(2.0), |renderer| {
            renderer.draw_backdrop(Backdrop {
                bounds: rect(8.0, 8.0, 40.0, 40.0),
                blur: profile,
                border_radius: 8.0.into(),
                border_smoothing: 0.6,
            });
        });
    });
    let transformed = read(&mut renderer);
    background(&mut renderer, true, 1.0);
    renderer.draw_backdrop(Backdrop {
        bounds: rect(32.0, 24.0, 80.0, 80.0),
        blur: profile.scaled(2.0),
        border_radius: 16.0.into(),
        border_smoothing: 0.6,
    });
    assert_eq!(transformed, read(&mut renderer));
}

#[test]
fn legacy_scalar_backdrop_filters_the_full_scene_inside_clipped_layers() {
    let mut renderer = renderer();
    background(&mut renderer, true, 1.0);
    renderer.blur_backdrop(12.0);
    let full = read(&mut renderer);
    assert!(
        pixel(&full, 60, 92)[2] > 15,
        "the legacy operation must blur the lower scene outside the marker clip"
    );

    for clip in [rect(8.0, 8.0, 4.0, 4.0), rect(256.0, 256.0, 4.0, 4.0)] {
        background(&mut renderer, true, 1.0);
        renderer.with_layer(clip, |renderer| renderer.blur_backdrop(12.0));
        assert_eq!(
            read(&mut renderer),
            full,
            "legacy scalar blur changed its full-scene scope inside {clip:?}"
        );
    }
}
