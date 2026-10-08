//! Pixel-level contracts for the experimental live glass material.
#![cfg(any(feature = "tiny-skia", feature = "wgpu"))]

use iced_core::glass::{Optics, Quality};
use iced_core::renderer::{Backdrop, Headless, Quad, Renderer as _};
use iced_core::{Blur, Color, Rectangle, Size, Transformation, Vector};

fn renderer() -> iced::Renderer {
    let backend = std::env::var("ICED_TEST_BACKEND").unwrap_or_else(|_| "tiny-skia".into());
    let renderer =
        iced::futures::executor::block_on(iced::Renderer::new(Default::default(), Some(&backend)))
            .expect("requested backend must be available");
    assert_eq!(renderer.name(), backend);
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

fn pixel(bytes: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    bytes[(y * width + x) * 4..(y * width + x + 1) * 4]
        .try_into()
        .unwrap()
}

fn quad(renderer: &mut iced::Renderer, bounds: Rectangle, color: Color) {
    renderer.fill_quad(
        Quad {
            bounds,
            snap: false,
            ..Default::default()
        },
        color,
    );
}

fn ramp(renderer: &mut iced::Renderer) {
    renderer.reset(rect(0.0, 0.0, 128.0, 128.0));
    for x in 0..128 {
        let red = x as f32 / 127.0;
        quad(
            renderer,
            rect(x as f32, 0.0, 1.0, 128.0),
            Color::from_rgb(red, 0.2, 1.0 - red),
        );
    }
}

fn material() -> Backdrop {
    Backdrop {
        bounds: rect(24.0, 24.0, 80.0, 80.0),
        border_radius: 16.0.into(),
        border_smoothing: 0.6,
        optics: Some(Optics {
            refraction: 10.0,
            depth: 20.0,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn capture(renderer: &mut iced::Renderer) -> Vec<u8> {
    renderer.screenshot(Size::new(128, 128), 1.0, Color::TRANSPARENT)
}

#[test]
fn clear_lens_refracts_edges_without_blurring_center_or_foreground() {
    let mut renderer = renderer();
    ramp(&mut renderer);
    let plain = capture(&mut renderer);
    ramp(&mut renderer);
    renderer.draw_backdrop(material());
    quad(&mut renderer, rect(52.0, 52.0, 24.0, 24.0), Color::WHITE);
    let glass = capture(&mut renderer);
    assert!(
        pixel(&glass, 128, 26, 64)[0] > pixel(&plain, 128, 26, 64)[0] + 10,
        "left lens must displace the scene inward"
    );
    assert!(
        pixel(&glass, 128, 101, 64)[0] + 10 < pixel(&plain, 128, 101, 64)[0],
        "right lens must displace the scene inward"
    );
    for (x, y) in [(22, 64), (105, 64), (24, 24), (64, 46)] {
        assert_eq!(
            pixel(&glass, 128, x, y),
            pixel(&plain, 128, x, y),
            "unchanged exterior/corner/center at {x},{y}"
        );
    }
    assert_eq!(pixel(&glass, 128, 64, 64), [255; 4]);
}

#[test]
fn neutral_optics_are_identity_and_lighting_follows_its_direction() {
    let mut renderer = renderer();
    let mut effect = material();
    effect.optics = Some(Optics::default());
    ramp(&mut renderer);
    let plain = capture(&mut renderer);
    ramp(&mut renderer);
    renderer.draw_backdrop(effect);
    assert_eq!(capture(&mut renderer), plain);
    let render = |renderer: &mut iced::Renderer, light| {
        renderer.reset(rect(0.0, 0.0, 128.0, 128.0));
        quad(
            renderer,
            rect(0.0, 0.0, 128.0, 128.0),
            Color::from_rgb(0.3, 0.3, 0.3),
        );
        effect.optics = Some(Optics {
            highlight: 0.8,
            shadow: 0.4,
            light,
            depth: 16.0,
            ..Default::default()
        });
        renderer.draw_backdrop(effect);
        capture(renderer)
    };
    let mut render = render;
    let upper = render(&mut renderer, Vector::new(0.0, -1.0));
    let lower = render(&mut renderer, Vector::new(0.0, 1.0));
    assert!(pixel(&upper, 128, 64, 26)[0] > pixel(&lower, 128, 64, 26)[0] + 30);
    assert!(pixel(&lower, 128, 64, 101)[0] > pixel(&upper, 128, 64, 101)[0] + 30);
    assert_eq!(pixel(&upper, 128, 64, 64), pixel(&lower, 128, 64, 64));
}

#[test]
fn tint_and_light_preserve_premultiplication_and_material_updates_reuse_filter() {
    let mut renderer = renderer();
    let mut effect = material();
    effect.blur = 8.0.into();
    let mut render = |tint| {
        renderer.reset(rect(0.0, 0.0, 128.0, 128.0));
        quad(
            &mut renderer,
            rect(0.0, 0.0, 128.0, 128.0),
            Color::from_rgba(0.0, 0.0, 1.0, 0.5),
        );
        effect.optics = Some(Optics {
            tint,
            highlight: 0.5,
            refraction: 10.0,
            depth: 20.0,
            ..Default::default()
        });
        renderer.draw_backdrop(effect);
        (capture(&mut renderer), renderer.blur_statistics())
    };
    let (clear, cold) = render(Color::TRANSPARENT);
    assert!(pixel(&clear, 128, 64, 64)[3].abs_diff(128) <= 1);
    let (tinted, warm) = render(Color::from_rgba(1.0, 0.0, 0.0, 0.5));
    let center = pixel(&tinted, 128, 64, 64);
    assert!(
        center[3].abs_diff(191) <= 2,
        "tint should source-over existing alpha once: {center:?}"
    );
    assert!(
        center[0] > center[2],
        "red tint must remain red: {center:?}"
    );
    assert_eq!(pixel(&tinted, 128, 20, 64), pixel(&clear, 128, 20, 64));
    assert_eq!(
        cold.scene_misses, warm.scene_misses,
        "tint should not rebuild convolution"
    );
    assert!(warm.scene_hits > cold.scene_hits);
}

#[test]
fn clipped_and_transformed_glass_keeps_its_original_optical_frame() {
    let mut renderer = renderer();
    let mut effect = material();
    effect.blur = 6.0.into();
    effect.quality = Quality::Balanced;
    ramp(&mut renderer);
    renderer.draw_backdrop(effect);
    let whole = capture(&mut renderer);
    ramp(&mut renderer);
    renderer.with_layer(rect(25.0, 45.0, 80.0, 30.0), |renderer| {
        renderer.draw_backdrop(effect)
    });
    let clipped = capture(&mut renderer);
    for y in 45..75 {
        for x in 25..104 {
            assert_eq!(
                pixel(&clipped, 128, x, y),
                pixel(&whole, 128, x, y),
                "clip changed lens at {x},{y}"
            );
        }
    }
    ramp(&mut renderer);
    let smaller = Backdrop {
        bounds: rect(12.0, 12.0, 40.0, 40.0),
        blur: 3.0.into(),
        border_radius: 8.0.into(),
        optics: Some(Optics {
            refraction: 5.0,
            depth: 10.0,
            ..Default::default()
        }),
        ..effect
    };
    renderer.with_transformation(Transformation::scale(2.0), |renderer| {
        renderer.draw_backdrop(smaller)
    });
    assert_eq!(
        capture(&mut renderer),
        whole,
        "transform must scale lens and sigma once"
    );
}

#[test]
fn reduced_quality_reduces_filter_allocations_but_keeps_mask_and_alpha() {
    let render = |quality| {
        let mut renderer = renderer();
        ramp(&mut renderer);
        renderer.draw_backdrop(Backdrop {
            blur: 12.0.into(),
            quality,
            ..material()
        });
        (capture(&mut renderer), renderer.blur_statistics())
    };
    let (full, high) = render(Quality::Quality);
    let (reduced, low) = render(Quality::Performance);
    assert!(
        low.retained_bytes < high.retained_bytes,
        "quality must reduce real filter allocations: {low:?} vs {high:?}"
    );
    for y in 0..128 {
        for x in 0..128 {
            assert_eq!(
                pixel(&full, 128, x, y)[3],
                pixel(&reduced, 128, x, y)[3],
                "mask/alpha changed at {x},{y}"
            );
        }
    }
    let left = pixel(&reduced, 128, 26, 64);
    let right = pixel(&reduced, 128, 101, 64);
    assert!(
        left[0] > 60 && right[0] < 195,
        "lower quality still refracts: {left:?}, {right:?}"
    );
}

#[test]
fn progressive_glass_keeps_sharp_regions_at_any_requested_quality() {
    let mut renderer = renderer();
    let mut draw = |quality| {
        ramp(&mut renderer);
        renderer.draw_backdrop(Backdrop {
            blur: Blur::vertical_gradient(0.0, 12.0).range(0.3, 0.7),
            quality,
            ..material()
        });
        capture(&mut renderer)
    };
    assert_eq!(
        draw(Quality::Quality),
        draw(Quality::Fixed(0.25)),
        "quality may not downsample a progressive clear end"
    );
}

#[test]
fn upper_glass_refreshes_when_only_the_lower_material_changes() {
    let mut renderer = renderer();
    let mut render = |tint| {
        ramp(&mut renderer);
        renderer.draw_backdrop(Backdrop {
            blur: 6.0.into(),
            optics: Some(Optics {
                tint,
                ..Default::default()
            }),
            ..material()
        });
        renderer.draw_backdrop(Backdrop {
            bounds: rect(32.0, 32.0, 64.0, 64.0),
            blur: 4.0.into(),
            optics: Some(Optics::default()),
            ..Default::default()
        });
        (capture(&mut renderer), renderer.blur_statistics())
    };
    let (red, cold) = render(Color::from_rgba(1.0, 0.0, 0.0, 0.8));
    let (blue, warm) = render(Color::from_rgba(0.0, 0.0, 1.0, 0.8));
    assert!(pixel(&red, 128, 64, 64)[0] > pixel(&blue, 128, 64, 64)[0] + 100);
    assert!(pixel(&blue, 128, 64, 64)[2] > pixel(&red, 128, 64, 64)[2] + 100);
    assert!(
        warm.scene_hits > cold.scene_hits,
        "lower convolution should be reused"
    );
    assert!(
        warm.scene_misses > cold.scene_misses,
        "upper convolution must see the new lower material"
    );
}

#[test]
fn filtered_refraction_crosses_gpu_tile_boundaries_without_changing_clip_results() {
    let mut renderer = renderer();
    let bounds = rect(0.0, 0.0, 1152.0, 96.0);
    let mut render = |clip| {
        renderer.reset(bounds);
        for x in 0..1152 {
            quad(
                &mut renderer,
                rect(x as f32, 0.0, 1.0, 96.0),
                if x % 32 < 16 {
                    Color::WHITE
                } else {
                    Color::BLACK
                },
            );
        }
        renderer.with_layer(clip, |renderer| {
            renderer.draw_backdrop(Backdrop {
                bounds: rect(12.0, 8.0, 1128.0, 80.0),
                blur: 5.0.into(),
                quality: Quality::Fixed(0.65),
                optics: Some(Optics {
                    refraction: 12.0,
                    depth: 30.0,
                    ..Default::default()
                }),
                ..Default::default()
            })
        });
        renderer.screenshot(Size::new(1152, 96), 1.0, Color::TRANSPARENT)
    };
    let all = render(bounds);
    let clipped = render(rect(400.0, 10.0, 300.0, 75.0));
    for y in 10..85 {
        for x in 400..700 {
            let a = pixel(&all, 1152, x, y);
            let b = pixel(&clipped, 1152, x, y);
            assert!(
                a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 1),
                "tile/crop changed filtered refraction at {x},{y}: {a:?} vs {b:?}"
            );
        }
    }
}
