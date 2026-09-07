//! Observable native blur contracts; run with ICED_TEST_BACKEND=wgpu or tiny-skia.
#![cfg(all(feature = "image", any(feature = "tiny-skia", feature = "wgpu")))]

use iced_core::image::Renderer as _;
use iced_core::renderer::{Headless, Quad, Renderer as _};
use iced_core::{Color, Rectangle, Size};

fn renderer() -> iced::Renderer {
    let backend = std::env::var("ICED_TEST_BACKEND").unwrap_or_else(|_| "tiny-skia".into());
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

fn source(renderer: &iced::Renderer) -> iced_core::image::Handle {
    let pixels: Vec<u8> = (0..64 * 64)
        .flat_map(|i| {
            if i % 64 < 32 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        })
        .collect();
    let handle = iced_core::image::Handle::from_rgba(64, 64, pixels);
    let _allocation = renderer.load_image(&handle).expect("load source");
    handle
}

fn pixel(bytes: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    bytes[(y * width + x) * 4..(y * width + x + 1) * 4]
        .try_into()
        .unwrap()
}

#[test]
fn blur_samples_beyond_overlay_and_shares_without_scaling_image_into_it() {
    let mut renderer = renderer();
    let handle = source(&renderer);
    let frame = rect(0.0, 0.0, 64.0, 64.0);
    renderer.reset(frame);
    for visible in [rect(29.0, 8.0, 6.0, 12.0), rect(29.0, 36.0, 6.0, 12.0)] {
        renderer.with_layer(visible, |renderer| {
            renderer.draw_image(
                iced_core::Image::new(&handle).blur(6.0).snap(false),
                frame,
                visible,
            );
        });
    }
    let pixels = renderer.screenshot(Size::new(64, 64), 1.0, Color::TRANSPARENT);
    let first = pixel(&pixels, 64, 30, 12);
    assert!(
        first[0] > 30 && first[2] > 30,
        "kernel must sample both sides beyond tiny overlay: {first:?}"
    );
    assert_eq!(first, pixel(&pixels, 64, 30, 40));
    assert_eq!(pixel(&pixels, 64, 10, 12)[3], 0);
    let cold = renderer.blur_statistics();
    assert_eq!(
        cold.image_misses, 1,
        "same source rendition shared by both overlays"
    );
    renderer.reset(frame);
    renderer.draw_image(
        iced_core::Image::new(&handle).blur(6.0).opacity(0.5_f32),
        frame,
        rect(30.0, 20.0, 10.0, 20.0),
    );
    let _ = renderer.screenshot(Size::new(64, 64), 1.0, Color::TRANSPARENT);
    assert_eq!(
        renderer.blur_statistics().image_misses,
        cold.image_misses,
        "mask position and opacity must not rebuild blur"
    );
}

#[test]
fn four_pixel_strip_uses_full_card_mask_at_fractional_dpi() {
    let mut renderer = renderer();
    let handle = iced_core::image::Handle::from_rgba(1, 1, vec![255; 4]);
    let _allocation = renderer.load_image(&handle).unwrap();
    let viewport = rect(0.0, 0.0, 80.0, 80.0);
    let card = rect(8.25, 8.5, 64.0, 64.0);
    for scale in [1.0_f32, 1.25, 1.5, 2.0] {
        let size = Size::new((80.0 * scale) as u32, (80.0 * scale) as u32);
        let image = iced_core::Image::new(&handle)
            .blur(5.0)
            .border_radius(18.0)
            .border_smoothing(0.7)
            .snap(false);
        renderer.reset(viewport);
        renderer.draw_image(image.clone(), card, card);
        let whole = renderer.screenshot(size, scale, Color::TRANSPARENT);
        renderer.reset(viewport);
        renderer.with_layer(
            rect(card.x, card.y + card.height - 4.0, card.width, 4.0),
            |renderer| {
                renderer.draw_image(image, card, card);
            },
        );
        let strip = renderer.screenshot(size, scale, Color::TRANSPARENT);
        let y = ((card.y + card.height - 2.0) * scale) as usize;
        for x in 0..size.width as usize {
            assert!(
                pixel(&whole, size.width as usize, x, y)[3]
                    .abs_diff(pixel(&strip, size.width as usize, x, y)[3])
                    <= 2,
                "strip mask differs from full card at {scale}x, x={x}"
            );
        }
    }
}

fn scene(renderer: &mut iced::Renderer, blue: bool, foreground: Color) -> Vec<u8> {
    renderer.reset(rect(0.0, 0.0, 64.0, 64.0));
    renderer.fill_quad(
        Quad {
            bounds: rect(0.0, 0.0, 32.0, 64.0),
            ..Default::default()
        },
        Color::WHITE,
    );
    renderer.fill_quad(
        Quad {
            bounds: rect(32.0, 0.0, 32.0, 64.0),
            ..Default::default()
        },
        if blue {
            Color::from_rgb(0.0, 0.0, 1.0)
        } else {
            Color::BLACK
        },
    );
    renderer.blur_backdrop(6.0);
    renderer.fill_quad(
        Quad {
            bounds: rect(28.0, 28.0, 8.0, 8.0),
            ..Default::default()
        },
        foreground,
    );
    renderer.screenshot(Size::new(64, 64), 1.0, Color::BLACK)
}

#[test]
fn scene_barrier_blurs_prior_content_but_not_modal_and_tracks_only_background() {
    let mut renderer = renderer();
    let first = scene(&mut renderer, false, Color::from_rgb(1.0, 0.0, 0.0));
    let edge = pixel(&first, 64, 30, 12);
    assert!(
        edge[0] > 20 && edge[0] < 245,
        "prior scene boundary should blur: {edge:?}"
    );
    assert_eq!(
        pixel(&first, 64, 29, 29),
        [255, 0, 0, 255],
        "modal remains crisp"
    );
    let cold = renderer.blur_statistics();
    let second = scene(&mut renderer, false, Color::from_rgb(0.0, 1.0, 0.0));
    assert_eq!(pixel(&second, 64, 30, 12), edge);
    assert_eq!(pixel(&second, 64, 29, 29), [0, 255, 0, 255]);
    let warm = renderer.blur_statistics();
    assert_eq!(
        warm.scene_misses, cold.scene_misses,
        "foreground-only changes must reuse background blur"
    );
    assert!(warm.scene_hits > cold.scene_hits);
    let changed = scene(&mut renderer, true, Color::from_rgb(0.0, 1.0, 0.0));
    assert_ne!(
        pixel(&changed, 64, 30, 12),
        edge,
        "live lower content must propagate"
    );
    assert!(renderer.blur_statistics().scene_misses > warm.scene_misses);
}

#[test]
fn resuming_parent_after_marker_cannot_move_foreground_before_blur() {
    let mut renderer = renderer();
    let frame = rect(0.0, 0.0, 64.0, 64.0);
    renderer.reset(frame);
    renderer.fill_quad(
        Quad {
            bounds: frame,
            ..Default::default()
        },
        Color::BLACK,
    );
    renderer.with_layer(frame, |renderer| {
        renderer.fill_quad(
            Quad {
                bounds: rect(0.0, 0.0, 32.0, 64.0),
                ..Default::default()
            },
            Color::WHITE,
        );
        renderer.blur_backdrop(6.0);
    });
    renderer.fill_quad(
        Quad {
            bounds: rect(30.0, 10.0, 4.0, 30.0),
            ..Default::default()
        },
        Color::from_rgb(1.0, 0.0, 0.0),
    );
    let pixels = renderer.screenshot(Size::new(64, 64), 1.0, Color::BLACK);
    assert_eq!(pixel(&pixels, 64, 31, 20), [255, 0, 0, 255]);
}

#[test]
fn zero_blur_preserves_direct_path_and_exact_pixels() {
    let mut renderer = renderer();
    let handle = source(&renderer);
    let frame = rect(0.0, 0.0, 64.0, 64.0);
    let mut render = |radius| {
        renderer.reset(frame);
        renderer.draw_image(iced_core::Image::new(&handle).blur(radius), frame, frame);
        renderer.blur_backdrop(radius);
        renderer.screenshot(Size::new(64, 64), 1.0, Color::TRANSPARENT)
    };
    let baseline = render(0.0);
    assert_eq!(baseline, render(f32::NAN));
    assert_eq!(baseline, render(-1.0));
    let stats = renderer.blur_statistics();
    assert_eq!(stats.image_misses, 0);
    assert_eq!(stats.scene_misses, 0);
    assert_eq!(
        stats.retained_bytes, 0,
        "no blur must allocate no intermediate blur textures"
    );
}

#[test]
fn selected_crop_does_not_blur_adjacent_source_or_atlas_allocations() {
    let mut renderer = renderer();
    // Exceeds the atlas layer width, exercising reconstruction across fragments.
    let width = 2304;
    let bytes: Vec<u8> = (0..width * 8)
        .flat_map(|i| {
            let x = i % width;
            if (32..2272).contains(&x) {
                [255, 0, 0, 255]
            } else {
                [0, 255, 0, 255]
            }
        })
        .collect();
    let handle = iced_core::image::Handle::from_rgba(width, 8, bytes);
    let neighbor = iced_core::image::Handle::from_rgba(64, 64, vec![255; 64 * 64 * 4]);
    let _source = renderer.load_image(&handle).unwrap();
    let _neighbor = renderer.load_image(&neighbor).unwrap();
    let frame = rect(0.0, 0.0, 128.0, 32.0);
    renderer.reset(frame);
    renderer.draw_image(
        iced_core::Image::new(&handle)
            .crop(Rectangle {
                x: 32,
                y: 0,
                width: 2240,
                height: 8,
            })
            .blur(8.0),
        frame,
        frame,
    );
    let bytes = renderer.screenshot(Size::new(128, 32), 1.0, Color::BLACK);
    for x in [1, 32, 64, 96, 126] {
        let p = pixel(&bytes, 128, x, 16);
        assert!(
            p[0] > 245 && p[1] < 3 && p[2] < 3,
            "isolated red crop contaminated at {x}: {p:?}"
        );
    }
}

#[test]
fn transparent_color_does_not_produce_blur_halos() {
    let mut renderer = renderer();
    let source: Vec<u8> = (0..64 * 16)
        .flat_map(|i| {
            if i % 64 < 32 {
                [255, 0, 0, 255]
            } else {
                [0, 255, 0, 0]
            }
        })
        .collect();
    let handle = iced_core::image::Handle::from_rgba(64, 16, source);
    let _allocation = renderer.load_image(&handle).unwrap();
    let frame = rect(0.0, 0.0, 64.0, 16.0);
    renderer.reset(frame);
    renderer.draw_image(iced_core::Image::new(&handle).blur(6.0), frame, frame);
    let transparent = renderer.screenshot(Size::new(64, 16), 1.0, Color::TRANSPARENT);
    let edge = pixel(&transparent, 64, 32, 8);
    assert!(
        edge[3] > 10 && edge[3] < 245,
        "alpha boundary must blur: {edge:?}"
    );
    assert!(
        edge[1] <= 2 && edge[2] <= 2,
        "hidden transparent RGB must not leak: {edge:?}"
    );
    for background in [Color::BLACK, Color::WHITE] {
        for width in [47.5, 64.0, 95.25] {
            let scaled = rect(0.25, 0.0, width, 16.0);
            renderer.reset(rect(0.0, 0.0, 100.0, 16.0));
            renderer.draw_image(
                iced_core::Image::new(&handle).blur(6.0).snap(false),
                scaled,
                scaled,
            );
            let composited = renderer.screenshot(Size::new(100, 16), 1.0, background);
            let p = pixel(&composited, 100, (width / 2.0) as usize, 8);
            assert!(
                p[1].abs_diff(p[2]) <= 2,
                "transparent green cannot tint either theme at width {width}: {p:?}"
            );
        }
    }
}

#[test]
fn scene_marker_removal_restores_sharp_background() {
    let mut renderer = renderer();
    let _ = scene(&mut renderer, false, Color::WHITE);
    renderer.reset(rect(0.0, 0.0, 64.0, 64.0));
    renderer.fill_quad(
        Quad {
            bounds: rect(0.0, 0.0, 32.0, 64.0),
            ..Default::default()
        },
        Color::WHITE,
    );
    let sharp = renderer.screenshot(Size::new(64, 64), 1.0, Color::BLACK);
    assert_eq!(pixel(&sharp, 64, 31, 12), [255; 4]);
    assert_eq!(pixel(&sharp, 64, 32, 12), [0, 0, 0, 255]);
}

#[test]
fn image_rotation_and_translation_keep_sampling_aligned() {
    let mut renderer = renderer();
    let handle = source(&renderer);
    let viewport = rect(0.0, 0.0, 100.0, 100.0);
    let frame = rect(12.25, 10.5, 64.0, 64.0);
    let image = iced_core::Image::new(&handle)
        .blur(5.0)
        .rotation(iced_core::Radians(std::f32::consts::FRAC_PI_2))
        .snap(false);
    renderer.reset(viewport);
    renderer.draw_image(image.clone(), frame, frame);
    let first = renderer.screenshot(Size::new(100, 100), 1.0, Color::BLACK);
    let near_top = pixel(&first, 100, 44, 20);
    let near_bottom = pixel(&first, 100, 44, 65);
    assert!(near_top[0] > near_top[2] + 80);
    assert!(near_bottom[2] > near_bottom[0] + 80);
    let cold = renderer.blur_statistics();
    renderer.reset(viewport);
    renderer.with_translation(iced_core::Vector::new(8.0, 12.0), |renderer| {
        renderer.draw_image(image, frame, frame);
    });
    let translated = renderer.screenshot(Size::new(100, 100), 1.0, Color::BLACK);
    for (x, y) in [(30, 20), (44, 42), (44, 65)] {
        assert_eq!(
            pixel(&first, 100, x, y),
            pixel(&translated, 100, x + 8, y + 12)
        );
    }
    assert_eq!(renderer.blur_statistics().image_misses, cold.image_misses);
}

#[test]
fn progress_clips_reuse_blur_but_scale_and_source_changes_do_not() {
    let mut renderer = renderer();
    let handle = source(&renderer);
    let frame = rect(0.0, 0.0, 64.0, 64.0);
    let mut cold = None;
    for progress in [0.0, 0.5, 1.0] {
        renderer.reset(frame);
        renderer.with_layer(rect(0.0, 60.0, 64.0, 4.0), |renderer| {
            renderer.draw_image(iced_core::Image::new(&handle).blur(6.0), frame, frame);
        });
        renderer.with_layer(rect(0.0, 60.0, 64.0 * progress, 4.0), |renderer| {
            renderer.fill_quad(
                Quad {
                    bounds: frame,
                    ..Default::default()
                },
                Color::from_rgb(0.0, 1.0, 0.0),
            );
        });
        let pixels = renderer.screenshot(Size::new(64, 64), 1.0, Color::BLACK);
        let played = pixel(&pixels, 64, 8, 62);
        if progress > 0.0 {
            assert_eq!(
                played,
                [0, 255, 0, 255],
                "played color must be above frosted strip"
            );
        } else {
            assert!(played[0] > played[1] + 80);
        }
        let misses = renderer.blur_statistics().image_misses;
        assert_eq!(*cold.get_or_insert(misses), misses);
    }
    renderer.reset(frame);
    renderer.draw_image(iced_core::Image::new(&handle).blur(6.0), frame, frame);
    let _ = renderer.screenshot(Size::new(96, 96), 1.5, Color::BLACK);
    assert!(
        renderer.blur_statistics().image_misses > cold.unwrap(),
        "different effective display scale cannot share the old rendition"
    );
    let old = renderer.blur_statistics().image_misses;
    let changed = iced_core::image::Handle::from_rgba(64, 64, [0, 255, 0, 255].repeat(64 * 64));
    let _allocation = renderer.load_image(&changed).unwrap();
    renderer.reset(frame);
    renderer.draw_image(iced_core::Image::new(&changed).blur(6.0), frame, frame);
    let pixels = renderer.screenshot(Size::new(64, 64), 1.0, Color::BLACK);
    assert_eq!(
        pixel(&pixels, 64, 32, 32),
        [0, 255, 0, 255],
        "new image content must replace previous blur"
    );
    assert!(renderer.blur_statistics().image_misses > old);
}

fn widget_pixels(
    renderer: &mut iced::Renderer,
    element: iced::Element<'_, ()>,
    size: Size<u32>,
) -> Vec<u8> {
    let logical = Size::new(size.width as f32, size.height as f32);
    let mut ui = iced_runtime::UserInterface::build(element, logical, Default::default(), renderer);
    ui.draw(
        renderer,
        &iced::Theme::Dark,
        &iced_core::renderer::Style {
            text_color: Color::WHITE,
        },
        iced_core::mouse::Cursor::Unavailable,
    );
    renderer.screenshot(size, 1.0, Color::BLACK)
}

#[test]
fn widget_glass_tint_is_above_blur_and_hero_fade_keeps_display_coordinates() {
    let mut renderer = renderer();
    let handle = iced_core::image::Handle::from_rgba(1, 1, vec![255; 4]);
    let _allocation = renderer.load_image(&handle).unwrap();
    let tint = Color::from_rgba(0.0, 0.0, 0.0, 0.75);
    let tinted = widget_pixels(
        &mut renderer,
        iced::widget::image(&handle)
            .width(64)
            .height(64)
            .blur(6.0)
            .tint(tint)
            .into(),
        Size::new(64, 64),
    );
    let p = pixel(&tinted, 64, 32, 32);
    assert!(
        p[0] < 200,
        "post-blur tint must not be hidden beneath opaque image: {p:?}"
    );
    let fade = iced_core::gradient::Linear::new(iced_core::Radians(0.7))
        .add_stop(0.0, Color::TRANSPARENT)
        .add_stop(0.5, Color::from_rgba(0.0, 0.0, 0.0, 0.4))
        .add_stop(1.0, Color::BLACK);
    let full = widget_pixels(
        &mut renderer,
        iced::widget::image(&handle)
            .width(64)
            .height(64)
            .blur(6.0)
            .tint(fade)
            .into(),
        Size::new(64, 64),
    );
    let glass = widget_pixels(
        &mut renderer,
        iced::widget::image(&handle)
            .width(16)
            .height(64)
            .blur(6.0)
            .display_frame(rect(-24.0, 0.0, 64.0, 64.0))
            .mask_frame(rect(0.0, 0.0, 16.0, 64.0))
            .tint(fade)
            .into(),
        Size::new(16, 64),
    );
    for y in [8, 24, 40, 56] {
        let expected = pixel(&full, 64, 32, y);
        let actual = pixel(&glass, 16, 8, y);
        assert!(
            expected[0].abs_diff(actual[0]) <= 3,
            "Hero fade shifted when masked to button at y={y}: {expected:?} vs {actual:?}"
        );
    }
}

#[test]
fn software_blur_budget_counts_recycled_buffers_and_scratch() {
    let mut renderer = renderer();
    if renderer.name() != "tiny-skia" {
        return;
    }
    let handle = iced_core::image::Handle::from_rgba(1, 1, vec![255; 4]);
    let _allocation = renderer.load_image(&handle).unwrap();
    for side in [1536, 2048, 2048, 1600] {
        // Distinct strengths retain entries even when dimensions repeat.
        let strength = 3.0 + renderer.blur_statistics().image_misses as f32;
        let frame = rect(0.0, 0.0, side as f32, side as f32);
        renderer.reset(frame);
        renderer.with_layer(rect(0.0, 0.0, 4.0, 4.0), |renderer| {
            renderer.draw_image(iced_core::Image::new(&handle).blur(strength), frame, frame);
        });
        let _ = renderer.screenshot(Size::new(8, 8), 1.0, Color::BLACK);
        assert!(
            renderer.blur_statistics().retained_bytes <= 64 * 1024 * 1024,
            "resident blur storage includes recycled capacity and scratch: {:?}",
            renderer.blur_statistics()
        );
    }
}

#[test]
fn overflowing_image_cache_preserves_each_draw_and_stabilizes_residency() {
    let mut renderer = renderer();
    let handles: Vec<_> = (0..48)
        .map(|i| {
            iced_core::image::Handle::from_rgba(
                1,
                1,
                vec![
                    (i % 4 * 85) as u8,
                    (i / 4 % 4 * 85) as u8,
                    (i / 16 * 85) as u8,
                    255,
                ],
            )
        })
        .collect();
    let _allocations: Vec<_> = handles
        .iter()
        .map(|handle| renderer.load_image(handle).unwrap())
        .collect();
    let mut residency = None;
    for pass in 0..3 {
        renderer.reset(rect(0.0, 0.0, 256.0, 192.0));
        for (index, handle) in handles.iter().enumerate() {
            let bounds = rect(
                (index % 8 * 32) as f32,
                (index / 8 * 32) as f32,
                16.0 + (index % 17) as f32,
                16.0 + (index * 7 % 17) as f32,
            );
            renderer.draw_image(iced_core::Image::new(handle).blur(4.0), bounds, bounds);
        }
        let pixels = renderer.screenshot(Size::new(256, 192), 1.0, Color::BLACK);
        for i in 0..48 {
            let expected = [
                (i % 4 * 85) as u8,
                (i / 4 % 4 * 85) as u8,
                (i / 16 * 85) as u8,
                255,
            ];
            let actual = pixel(&pixels, 256, i % 8 * 32 + 8, i / 8 * 32 + 8);
            assert!(
                expected.iter().zip(actual).all(|(a, b)| a.abs_diff(b) <= 2),
                "recycling overwrote an outstanding image draw {i}, pass {pass}: {expected:?} vs {actual:?}"
            );
        }
        if pass > 0 {
            let bytes = renderer.blur_statistics().retained_bytes;
            assert_eq!(
                *residency.get_or_insert(bytes),
                bytes,
                "steady working set must not keep retaining textures"
            );
        }
    }
}

#[cfg(feature = "canvas")]
#[test]
fn software_static_geometry_does_not_invalidate_scene_convolution() {
    use iced::advanced::graphics::geometry::{Frame, Renderer as _};
    let mut renderer = renderer();
    if renderer.name() != "tiny-skia" {
        return;
    }
    let mut misses = None;
    for foreground in [Color::WHITE, Color::BLACK] {
        renderer.reset(rect(0.0, 0.0, 64.0, 64.0));
        let mut geometry = Frame::new(&renderer, Size::new(64.0, 64.0));
        geometry.fill_rectangle(
            iced_core::Point::ORIGIN,
            Size::new(32.0, 64.0),
            Color::WHITE,
        );
        renderer.draw_geometry(geometry.into_geometry());
        renderer.blur_backdrop(6.0);
        renderer.fill_quad(
            Quad {
                bounds: rect(24.0, 24.0, 16.0, 16.0),
                ..Default::default()
            },
            foreground,
        );
        let _ = renderer.screenshot(Size::new(64, 64), 1.0, Color::BLACK);
        let current = renderer.blur_statistics().scene_misses;
        assert_eq!(
            *misses.get_or_insert(current),
            current,
            "static rendered geometry must allow scene cache reuse"
        );
    }
}

#[cfg(feature = "canvas")]
#[test]
fn geometry_and_renderer_scaling_apply_blur_strength_once() {
    use iced::advanced::graphics::geometry::{Frame, Renderer as _};
    let mut renderer = renderer();
    let handle = source(&renderer);
    let bounds = rect(0.0, 0.0, 32.0, 32.0);
    let viewport = rect(0.0, 0.0, 64.0, 64.0);
    for sigma in [6.0, 80.0] {
        let image = iced_core::Image::new(&handle).blur(sigma).snap(false);
        renderer.reset(viewport);
        renderer.with_transformation(iced_core::Transformation::scale(2.0), |renderer| {
            renderer.draw_image(image.clone(), bounds, bounds);
        });
        let direct = renderer.screenshot(Size::new(64, 64), 1.0, Color::BLACK);
        renderer.reset(viewport);
        let mut frame = Frame::new(&renderer, viewport.size());
        frame.scale(2.0);
        frame.draw_image(bounds, image);
        renderer.draw_geometry(frame.into_geometry());
        let geometry = renderer.screenshot(Size::new(64, 64), 1.0, Color::BLACK);
        for x in 4..60 {
            let expected = pixel(&direct, 64, x, 32);
            let actual = pixel(&geometry, 64, x, 32);
            assert!(
                expected.iter().zip(actual).all(|(a, b)| a.abs_diff(b) <= 2),
                "blur transform mismatch sigma={sigma}, x={x}: {expected:?} vs {actual:?}"
            );
        }
    }
}

#[cfg(feature = "canvas")]
#[test]
fn geometry_rotation_does_not_change_blur_strength() {
    use iced::advanced::graphics::geometry::{Frame, Renderer as _};
    let mut renderer = renderer();
    let handle = source(&renderer);
    let viewport = rect(0.0, 0.0, 64.0, 64.0);
    for angle in [std::f32::consts::FRAC_PI_3, std::f32::consts::FRAC_PI_2] {
        renderer.reset(viewport);
        let image = iced_core::Image::new(&handle).blur(6.0).snap(false);
        renderer.draw_image(
            image.clone().rotation(iced_core::Radians(angle)),
            rect(16.0, 16.0, 32.0, 32.0),
            viewport,
        );
        let direct = renderer.screenshot(Size::new(64, 64), 1.0, Color::BLACK);
        renderer.reset(viewport);
        let mut frame = Frame::new(&renderer, viewport.size());
        frame.translate(iced_core::Vector::new(32.0, 32.0));
        frame.rotate(iced_core::Radians(angle));
        frame.draw_image(rect(-16.0, -16.0, 32.0, 32.0), image);
        renderer.draw_geometry(frame.into_geometry());
        let rotated = renderer.screenshot(Size::new(64, 64), 1.0, Color::BLACK);
        let error = direct
            .iter()
            .zip(rotated)
            .map(|(a, b)| a.abs_diff(b))
            .max()
            .unwrap();
        assert!(
            error <= 3,
            "frame rotation changed blur strength: angle={angle}, maximum channel error={error}"
        );
    }
}
