//! Observable progressive-image and live-backdrop contracts on both renderers.
#![cfg(all(feature = "image", any(feature = "tiny-skia", feature = "wgpu")))]

use iced_core::image::Renderer as _;
use iced_core::renderer::{Backdrop, Headless, Quad, Renderer as _};
use iced_core::{Blur, Color, Rectangle, Size, Transformation};

fn renderer() -> iced::Renderer {
    let backend = std::env::var("ICED_TEST_BACKEND").unwrap_or_else(|_| "tiny-skia".into());
    let renderer =
        iced::futures::executor::block_on(iced::Renderer::new(Default::default(), Some(&backend)))
            .expect("requested renderer");
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

fn pixel(bytes: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    bytes[(y * width + x) * 4..(y * width + x + 1) * 4]
        .try_into()
        .unwrap()
}

fn split_source(renderer: &iced::Renderer, horizontal: bool) -> iced_core::image::Handle {
    let pixels = (0..128 * 128)
        .flat_map(|i| {
            let first = if horizontal {
                i / 128 < 64
            } else {
                i % 128 < 64
            };
            if first {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        })
        .collect::<Vec<_>>();
    let handle = iced_core::image::Handle::from_rgba(128, 128, pixels);
    let _ = renderer.load_image(&handle).expect("load split image");
    handle
}

fn draw_image(
    renderer: &mut iced::Renderer,
    handle: &iced_core::image::Handle,
    blur: Blur,
) -> Vec<u8> {
    let bounds = rect(0.0, 0.0, 128.0, 128.0);
    renderer.reset(bounds);
    renderer.draw_image(
        iced_core::Image::new(handle).blur(blur).snap(false),
        bounds,
        bounds,
    );
    renderer.screenshot(Size::new(128, 128), 1.0, Color::TRANSPARENT)
}

#[test]
fn image_radius_varies_in_both_directions_and_can_be_reversed() {
    let mut renderer = renderer();
    for horizontal in [false, true] {
        let handle = split_source(&renderer, horizontal);
        for reverse in [false, true] {
            let (start, end) = if reverse { (12.0, 0.0) } else { (0.0, 12.0) };
            let profile = if horizontal {
                Blur::horizontal_gradient(start, end)
            } else {
                Blur::vertical_gradient(start, end)
            }
            .range(0.25, 0.75);
            let pixels = draw_image(&mut renderer, &handle, profile);
            let (sharp_axis, blurred_axis) = if reverse { (112, 16) } else { (16, 112) };
            let sample = |axis| {
                if horizontal {
                    pixel(&pixels, 128, axis, 58)
                } else {
                    pixel(&pixels, 128, 58, axis)
                }
            };
            assert_eq!(sample(sharp_axis), [255, 0, 0, 255]);
            let blurred = sample(blurred_axis);
            assert!(
                blurred[0] > 50 && blurred[2] > 15,
                "blur must spread the opposite color: {blurred:?}"
            );
        }
    }
}

#[test]
fn sharp_plateau_retains_pixel_detail_beyond_old_rendition_limits() {
    let mut renderer = renderer();
    for horizontal in [false, true] {
        let (width, height) = if horizontal { (48, 2304) } else { (2304, 48) };
        let pixels = (0..width * height)
            .flat_map(|i| {
                let stripe = if horizontal { i / width } else { i % width };
                if stripe % 2 == 0 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                }
            })
            .collect::<Vec<_>>();
        let handle = iced_core::image::Handle::from_rgba(width, height, pixels);
        let _ = renderer.load_image(&handle).unwrap();
        let frame = rect(0.0, 0.0, width as f32, height as f32);
        let profile = if horizontal {
            Blur::horizontal_gradient(0.0, 8.0)
        } else {
            Blur::vertical_gradient(0.0, 8.0)
        }
        .range(0.5, 0.9);
        renderer.reset(frame);
        renderer.draw_image(
            iced_core::Image::new(&handle).blur(profile).snap(false),
            frame,
            frame,
        );
        let pixels = renderer.screenshot(Size::new(width, height), 1.0, Color::TRANSPARENT);
        for position in [127, 128, 255, 256, 511, 512, 1023, 1024, 2047, 2048, 2201] {
            let (x, y) = if horizontal {
                (8, position)
            } else {
                (position, 8)
            };
            let expected = if position % 2 == 0 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            };
            assert_eq!(
                pixel(&pixels, width as usize, x, y),
                expected,
                "sharp pixel at {x},{y}"
            );
        }
    }
}

#[test]
fn small_viewport_only_prepares_visible_tiles_of_a_large_image() {
    let mut renderer = renderer();
    let handle = iced_core::image::Handle::from_rgba(1, 1, vec![255; 4]);
    let _ = renderer.load_image(&handle).unwrap();
    let viewport = rect(0.0, 0.0, 64.0, 64.0);
    let huge = rect(-16000.0, -16000.0, 32768.0, 32768.0);
    renderer.reset(viewport);
    renderer.with_layer(viewport, |renderer| {
        renderer.draw_image(
            iced_core::Image::new(&handle)
                .blur(Blur::vertical_gradient(0.0, 12.0))
                .snap(false),
            huge,
            huge,
        )
    });
    let pixels = renderer.screenshot(Size::new(64, 64), 1.0, Color::TRANSPARENT);
    assert_eq!(pixel(&pixels, 64, 32, 32), [255; 4]);
    let stats = renderer.blur_statistics();
    assert!(
        stats.image_misses <= 4,
        "only viewport-intersecting tiles should be prepared: {stats:?}"
    );
}

#[test]
fn broad_physical_kernels_keep_the_crop_and_sharp_endpoint() {
    let mut renderer = renderer();
    let pixels = (0..16 * 16)
        .flat_map(|i| {
            if !(4..12).contains(&(i % 16)) || !(4..12).contains(&(i / 16)) {
                [255, 0, 0, 255]
            } else if i % 16 < 8 {
                [0, 255, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        })
        .collect::<Vec<_>>();
    let handle = iced_core::image::Handle::from_rgba(16, 16, pixels);
    let _ = renderer.load_image(&handle).unwrap();
    let bounds = rect(0.0, 0.0, 8.0, 8.0);
    renderer.reset(bounds);
    renderer.draw_image(
        iced_core::Image::new(&handle)
            .crop(Rectangle {
                x: 4,
                y: 4,
                width: 8,
                height: 8,
            })
            .blur(Blur::vertical_gradient(0.0, 128.0).range(0.4, 0.6))
            .snap(false),
        bounds,
        bounds,
    );
    // Physical sigma must not be clamped back to the logical128 pixel limit.
    let pixels = renderer.screenshot(Size::new(32, 32), 4.0, Color::TRANSPARENT);
    assert_eq!(pixel(&pixels, 32, 4, 4), [0, 255, 0, 255]);
    let blurred = pixel(&pixels, 32, 12, 28);
    assert_eq!(blurred[0], 0, "crop must exclude the red source margin");
    assert_eq!(blurred[3], 255);
    assert!(
        blurred[1] > 20 && blurred[2] > 20,
        "large physical kernel must still mix the crop: {blurred:?}"
    );
}

#[cfg(feature = "wgpu")]
#[test]
fn oversized_physical_kernel_uses_bounded_direct_sampling() {
    if std::env::var("ICED_TEST_BACKEND").as_deref() != Ok("wgpu") {
        return;
    }
    let mut renderer = renderer();
    let handle = iced_core::image::Handle::from_rgba(
        3,
        1,
        vec![255, 0, 0, 255, 0, 255, 0, 255, 255, 0, 0, 255],
    );
    let _ = renderer.load_image(&handle).unwrap();
    let bounds = rect(0.0, 0.0, 600.0, 8.0);
    renderer.reset(rect(0.0, 0.0, 4.0, 8.0));
    renderer.draw_image(
        iced_core::Image::new(&handle)
            .crop(Rectangle {
                x: 1,
                y: 0,
                width: 1,
                height: 1,
            })
            .blur(Blur::vertical_gradient(0.0, 128.0).range(0.25, 0.5))
            .snap(false),
        bounds,
        bounds,
    );
    let pixels = renderer.screenshot(Size::new(16, 32), 4.0, Color::TRANSPARENT);
    for (x, y) in [(4, 4), (12, 28)] {
        assert_eq!(
            pixel(&pixels, 16, x, y),
            [0, 255, 0, 255],
            "direct sampling must preserve crop and alpha"
        );
    }
    assert!(renderer.blur_statistics().retained_bytes < 64 * 1024 * 1024);
}

#[test]
fn profile_cache_reuses_identical_input_and_invalidates_on_profile_changes() {
    let mut renderer = renderer();
    let handle = split_source(&renderer, false);
    let profile = Blur::vertical_gradient(0.0, 12.0).range(0.25, 0.75);
    let first = draw_image(&mut renderer, &handle, profile);
    let cold = renderer.blur_statistics();
    assert!(cold.image_misses > 0);
    assert_eq!(draw_image(&mut renderer, &handle, profile), first);
    assert_eq!(renderer.blur_statistics().image_misses, cold.image_misses);
    let reversed = draw_image(
        &mut renderer,
        &handle,
        Blur::vertical_gradient(12.0, 0.0).range(0.25, 0.75),
    );
    assert_ne!(first, reversed);
    assert!(renderer.blur_statistics().image_misses > cold.image_misses);
    let uniform = draw_image(&mut renderer, &handle, 8.0.into());
    let cold = renderer.blur_statistics();
    assert_eq!(
        draw_image(&mut renderer, &handle, Blur::horizontal_gradient(8.0, 8.0)),
        uniform
    );
    assert_eq!(renderer.blur_statistics().image_misses, cold.image_misses);
    assert_eq!(
        draw_image(
            &mut renderer,
            &handle,
            Blur::vertical_gradient(f32::NAN, -8.0)
        ),
        draw_image(&mut renderer, &handle, 0.0.into())
    );
}

#[test]
fn progressive_image_scales_sigma_once_for_renderer_transform_and_dpi() {
    let mut renderer = renderer();
    let handle = split_source(&renderer, false);
    let bounds = rect(0.0, 0.0, 64.0, 64.0);
    let image = iced_core::Image::new(&handle)
        .blur(Blur::vertical_gradient(0.0, 8.0).range(0.2, 0.8))
        .snap(false);
    renderer.reset(bounds);
    renderer.draw_image(image.clone(), bounds, bounds);
    let dpi = renderer.screenshot(Size::new(128, 128), 2.0, Color::TRANSPARENT);
    renderer.reset(rect(0.0, 0.0, 128.0, 128.0));
    renderer.with_transformation(Transformation::scale(2.0), |renderer| {
        renderer.draw_image(image, bounds, bounds);
    });
    let transformed = renderer.screenshot(Size::new(128, 128), 1.0, Color::TRANSPARENT);
    let maximum = dpi
        .iter()
        .zip(&transformed)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(
        maximum <= 2,
        "blur was scaled differently: maximum error {maximum}"
    );
}

fn background(renderer: &mut iced::Renderer, blue: bool, alpha: f32) {
    let bounds = rect(0.0, 0.0, 128.0, 128.0);
    renderer.reset(bounds);
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
        ..Default::default()
    }
}

#[test]
fn local_backdrop_preserves_exterior_alpha_and_unclipped_profile_coordinates() {
    let mut renderer = renderer();
    let profile = Blur::vertical_gradient(0.0, 12.0).range(0.2, 0.8);
    background(&mut renderer, true, 0.5);
    let sharp = renderer.screenshot(Size::new(128, 128), 1.0, Color::TRANSPARENT);
    background(&mut renderer, true, 0.5);
    renderer.draw_backdrop(effect(profile));
    let full = renderer.screenshot(Size::new(128, 128), 1.0, Color::TRANSPARENT);
    for (x, y) in [(46, 88), (81, 88), (48, 16), (79, 111), (60, 22)] {
        assert_eq!(
            pixel(&full, 128, x, y),
            pixel(&sharp, 128, x, y),
            "outside mask or sharp plateau at {x},{y}"
        );
    }
    let blurred = pixel(&full, 128, 60, 92);
    assert!(
        blurred[2] > 15,
        "interior blur samples outside its effect region: {blurred:?}"
    );
    for y in 0..128 {
        for x in 0..128 {
            assert!(
                pixel(&full, 128, x, y)[3].abs_diff(pixel(&sharp, 128, x, y)[3]) <= 1,
                "backdrop must replace premultiplied pixels without increasing alpha at {x},{y}"
            );
        }
    }
    let clip = rect(58.0, 56.0, 12.0, 42.0);
    background(&mut renderer, true, 0.5);
    renderer.with_layer(clip, |renderer| renderer.draw_backdrop(effect(profile)));
    let clipped = renderer.screenshot(Size::new(128, 128), 1.0, Color::TRANSPARENT);
    for y in 56..98 {
        for x in 58..70 {
            assert_eq!(
                pixel(&clipped, 128, x, y),
                pixel(&full, 128, x, y),
                "clipping shifted profile at {x},{y}"
            );
        }
    }
    assert_eq!(pixel(&clipped, 128, 60, 100), pixel(&sharp, 128, 60, 100));
}

#[test]
fn live_backdrop_refreshes_background_and_reuses_it_for_foreground_only_changes() {
    let mut renderer = renderer();
    let profile = Blur::vertical_gradient(2.0, 12.0);
    let mut render = |blue: bool, foreground: Color| {
        background(&mut renderer, blue, 1.0);
        renderer.draw_backdrop(effect(profile));
        renderer.fill_quad(
            Quad {
                bounds: rect(56.0, 72.0, 16.0, 12.0),
                ..Default::default()
            },
            foreground,
        );
        let pixels = renderer.screenshot(Size::new(128, 128), 1.0, Color::TRANSPARENT);
        (pixels, renderer.blur_statistics())
    };
    let (first, cold) = render(true, Color::WHITE);
    assert_eq!(pixel(&first, 128, 60, 78), [255; 4]);
    let (second, warm) = render(true, Color::BLACK);
    assert_eq!(pixel(&second, 128, 60, 78), [0, 0, 0, 255]);
    assert_eq!(pixel(&first, 128, 60, 94), pixel(&second, 128, 60, 94));
    assert_eq!(cold.scene_misses, warm.scene_misses);
    assert!(warm.scene_hits > cold.scene_hits);
    let (third, changed) = render(false, Color::WHITE);
    assert_ne!(pixel(&first, 128, 60, 94), pixel(&third, 128, 60, 94));
    assert!(changed.scene_misses > warm.scene_misses);
    background(&mut renderer, true, 1.0);
    let removed = renderer.screenshot(Size::new(128, 128), 1.0, Color::TRANSPARENT);
    assert_eq!(pixel(&removed, 128, 60, 94), [255, 0, 0, 255]);
}

#[test]
fn clipped_effect_samples_neighbors_beyond_its_visible_region() {
    let mut renderer = renderer();
    background(&mut renderer, true, 1.0);
    let mut effect = effect(8.0.into());
    // This region is entirely red. Blue is four pixels beyond its right edge.
    effect.bounds = rect(48.0, 24.0, 12.0, 80.0);
    effect.border_radius = 0.0.into();
    renderer.draw_backdrop(effect);
    let pixels = renderer.screenshot(Size::new(128, 128), 1.0, Color::TRANSPARENT);
    assert!(
        pixel(&pixels, 128, 58, 64)[2] > 10,
        "kernel must sample beyond the mask"
    );
    assert_eq!(pixel(&pixels, 128, 61, 64), [255, 0, 0, 255]);
}

#[test]
fn backdrop_masks_and_profiles_follow_renderer_scale_and_translation() {
    let mut renderer = renderer();
    let profile = Blur::horizontal_gradient(0.0, 10.0);
    background(&mut renderer, true, 1.0);
    let transformed = Backdrop {
        bounds: rect(8.0, 8.0, 40.0, 40.0),
        blur: profile,
        border_radius: 8.0.into(),
        border_smoothing: 0.6,
        ..Default::default()
    };
    renderer.with_translation(iced_core::Vector::new(16.0, 8.0), |renderer| {
        renderer.with_transformation(Transformation::scale(2.0), |renderer| {
            renderer.draw_backdrop(transformed)
        });
    });
    let a = renderer.screenshot(Size::new(128, 128), 1.0, Color::TRANSPARENT);
    background(&mut renderer, true, 1.0);
    renderer.draw_backdrop(Backdrop {
        bounds: rect(32.0, 24.0, 80.0, 80.0),
        blur: profile.scaled(2.0),
        border_radius: 16.0.into(),
        border_smoothing: 0.6,
        ..Default::default()
    });
    let b = renderer.screenshot(Size::new(128, 128), 1.0, Color::TRANSPARENT);
    assert_eq!(a, b);
}

#[test]
fn overlapping_backdrops_sample_intermediate_foreground_in_draw_order() {
    let mut renderer = renderer();
    let mut render = |color| {
        background(&mut renderer, true, 1.0);
        renderer.draw_backdrop(effect(Blur::vertical_gradient(0.0, 12.0)));
        renderer.fill_quad(
            Quad {
                bounds: rect(60.0, 40.0, 8.0, 48.0),
                ..Default::default()
            },
            color,
        );
        renderer.draw_backdrop(Backdrop {
            bounds: rect(32.0, 48.0, 64.0, 48.0),
            blur: Blur::horizontal_gradient(6.0, 10.0),
            border_radius: 0.0.into(),
            border_smoothing: 0.0,
            ..Default::default()
        });
        renderer.fill_quad(
            Quad {
                bounds: rect(56.0, 80.0, 16.0, 8.0),
                ..Default::default()
            },
            Color::WHITE,
        );
        renderer.screenshot(Size::new(128, 128), 1.0, Color::TRANSPARENT)
    };
    let first = render(Color::from_rgb(0.0, 1.0, 0.0));
    assert!(
        pixel(&first, 128, 56, 64)[1] > 15,
        "upper blur must sample intermediate green foreground"
    );
    assert_eq!(pixel(&first, 128, 64, 44), [0, 255, 0, 255]);
    assert_eq!(pixel(&first, 128, 64, 84), [255; 4]);
    let second = render(Color::BLACK);
    assert_eq!(
        pixel(&second, 128, 56, 64)[1],
        0,
        "upper cache must invalidate with intermediate foreground"
    );
}

#[cfg(feature = "wgpu")]
#[test]
fn live_custom_primitive_updates_backdrop_without_changing_its_identity() {
    use iced_wgpu::primitive::{Pipeline, Renderer as _};
    use iced_wgpu::wgpu;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    if std::env::var("ICED_TEST_BACKEND").as_deref() != Ok("wgpu") {
        return;
    }

    struct LiveFrame(Arc<AtomicBool>);
    impl std::fmt::Debug for LiveFrame {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            // Video producers may change texture contents without changing the primitive.
            f.write_str("LiveFrame")
        }
    }
    struct FramePipeline;
    impl Pipeline for FramePipeline {
        fn new(_: &wgpu::Device, _: &wgpu::Queue, _: wgpu::TextureFormat) -> Self {
            Self
        }
    }
    impl iced_wgpu::Primitive for LiveFrame {
        type Pipeline = FramePipeline;
        fn prepare(
            &self,
            _: &mut FramePipeline,
            _: &wgpu::Device,
            _: &wgpu::Queue,
            _: &Rectangle,
            _: &iced_wgpu::graphics::Viewport,
        ) {
        }
        fn render(
            &self,
            _: &FramePipeline,
            encoder: &mut wgpu::CommandEncoder,
            target: &wgpu::TextureView,
            _: &Rectangle<u32>,
        ) {
            let color = if self.0.load(Ordering::Relaxed) {
                wgpu::Color::BLUE
            } else {
                wgpu::Color::RED
            };
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("live frame regression"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
    }

    let mut renderer = iced::futures::executor::block_on(<iced_wgpu::Renderer as Headless>::new(
        Default::default(),
        Some("wgpu"),
    ))
    .expect("wgpu renderer");
    let frame = Arc::new(AtomicBool::new(false));
    for blue in [false, true, false] {
        frame.store(blue, Ordering::Relaxed);
        let bounds = rect(0.0, 0.0, 128.0, 128.0);
        renderer.reset(bounds);
        renderer.draw_primitive(bounds, LiveFrame(frame.clone()));
        renderer.draw_backdrop(Backdrop {
            optics: Some(iced_core::glass::Optics {
                refraction: 8.0,
                ..Default::default()
            }),
            ..effect(Blur::vertical_gradient(0.0, 12.0))
        });
        let pixels =
            Headless::screenshot(&mut renderer, Size::new(128, 128), 1.0, Color::TRANSPARENT);
        assert_eq!(
            pixel(&pixels, 128, 60, 92),
            if blue {
                [0, 0, 255, 255]
            } else {
                [255, 0, 0, 255]
            }
        );
    }
}
