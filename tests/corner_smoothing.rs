//! Native shape coverage regressions shared by the real renderers.
#![cfg(all(feature = "image", any(feature = "tiny-skia", feature = "wgpu")))]

use iced_core::image::Renderer as _;
use iced_core::renderer::{Headless, Quad, Renderer as _};
use iced_core::{Border, Color, Rectangle, Shadow, Size, Vector};

fn renderer() -> iced::Renderer {
    let backend = std::env::var("ICED_TEST_BACKEND").unwrap_or_else(|_| "tiny-skia".into());
    let renderer =
        iced::futures::executor::block_on(iced::Renderer::new(Default::default(), Some(&backend)))
            .expect("requested native renderer must be available");
    assert_eq!(
        renderer.name(),
        backend,
        "requested backend must not silently fall back"
    );
    renderer
}

fn allocate(
    renderer: &mut iced::Renderer,
    handle: &iced_core::image::Handle,
) -> iced_core::image::Allocation {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    renderer.allocate_image(handle, move |result| {
        let _ = send.send(result);
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        renderer.tick();
        match receive.try_recv() {
            Ok(result) => return result.expect("source allocation failed"),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("allocation callback disconnected")
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "image allocation exceeded 30 seconds"
                );
                std::thread::yield_now();
            }
        }
    }
}

#[test]
fn quad_and_image_share_corner_coverage() {
    let mut renderer = renderer();
    let handle = iced_core::image::Handle::from_rgba(1, 1, vec![255; 4]);
    let _allocation = allocate(&mut renderer, &handle);
    let viewport = Rectangle::with_size(Size::new(220.0, 140.0));
    let bounds = Rectangle {
        x: 20.25,
        y: 18.5,
        width: 160.5,
        height: 96.25,
    };
    for radius in [
        iced_core::border::Radius {
            top_left: 24.0,
            top_right: 12.0,
            bottom_right: 0.0,
            bottom_left: 32.0,
        },
        24.0.into(),
        iced_core::border::top(24),
    ] {
        for smoothing in [0.0, 0.6, 1.0] {
            for scale in [1.0_f32, 1.25, 1.5, 2.0] {
                for snap in [false, true] {
                    let size =
                        Size::new((220.0 * scale).ceil() as u32, (140.0 * scale).ceil() as u32);
                    renderer.reset(viewport);
                    let quad = Quad {
                        bounds,
                        border: Border::default().rounded(radius).smoothing(smoothing),
                        snap,
                        ..Quad::default()
                    };
                    renderer.fill_quad(quad, Color::WHITE);
                    let solid = renderer.screenshot(size, scale, Color::TRANSPARENT);
                    renderer.reset(viewport);
                    renderer.draw_image(
                        iced_core::image::Image::new(&handle)
                            .border_radius(radius)
                            .border_smoothing(smoothing)
                            .snap(snap),
                        bounds,
                        bounds,
                    );
                    let image = renderer.screenshot(size, scale, Color::TRANSPARENT);
                    renderer.reset(viewport);
                    renderer.fill_quad(
                        quad,
                        iced_core::gradient::Linear::new(iced_core::Radians(0.0))
                            .add_stop(0.0, Color::WHITE)
                            .add_stop(1.0, Color::WHITE),
                    );
                    let gradient = renderer.screenshot(size, scale, Color::TRANSPARENT);
                    for (name, pixels) in [("image", image), ("gradient", gradient)] {
                        let error = solid
                            .chunks_exact(4)
                            .zip(pixels.chunks_exact(4))
                            .map(|(a, b)| a[3].abs_diff(b[3]))
                            .max()
                            .unwrap();
                        assert!(
                            error <= 3,
                            "{name} alpha error {error}/255; radius={radius:?} s={smoothing} scale={scale} snap={snap}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn solid_and_gradient_shadows_share_coverage() {
    let mut renderer = renderer();
    let viewport = Rectangle::with_size(Size::new(280.0, 110.0));
    for offset in [Vector::new(8.0, 4.0), Vector::new(-8.0, -4.0)] {
        for blur in [0.0, 4.0, 16.0] {
            let mut screenshots = Vec::new();
            for gradient in [false, true] {
                renderer.reset(viewport);
                for (index, smoothing) in [0.0, 0.6, 1.0].into_iter().enumerate() {
                    let quad = Quad {
                        bounds: Rectangle {
                            x: 24.25 + index as f32 * 80.0,
                            y: 24.5,
                            width: 40.0,
                            height: 40.0,
                        },
                        border: Border::default().rounded(12).smoothing(smoothing),
                        shadow: Shadow {
                            color: Color::WHITE,
                            offset,
                            blur_radius: blur,
                        },
                        snap: index == 1,
                    };
                    if gradient {
                        renderer.fill_quad(
                            quad,
                            iced_core::gradient::Linear::new(iced_core::Radians(0.0))
                                .add_stop(0.0, Color::WHITE)
                                .add_stop(1.0, Color::WHITE),
                        );
                    } else {
                        renderer.fill_quad(quad, Color::WHITE);
                    }
                }
                screenshots.push(renderer.screenshot(Size::new(280, 110), 1.0, Color::TRANSPARENT));
            }
            let error = screenshots[0]
                .chunks_exact(4)
                .zip(screenshots[1].chunks_exact(4))
                .map(|(a, b)| a[3].abs_diff(b[3]))
                .max()
                .unwrap();
            assert!(
                error <= 3,
                "solid/gradient shadow alpha error: {error}/255 offset={offset:?} blur={blur}"
            );
        }
    }
}

#[test]
fn saturated_border_has_no_remaining_fill() {
    let mut renderer = renderer();
    for height in [21.0, 41.0] {
        for radius in [0.0, 10.5] {
            for width in [10.5, f32::INFINITY] {
                renderer.reset(Rectangle::with_size(Size::new(64.0, 80.0)));
                renderer.fill_quad(
                    Quad {
                        bounds: Rectangle {
                            x: 20.0,
                            y: 20.0,
                            width: 21.0,
                            height,
                        },
                        border: Border::default()
                            .rounded(radius)
                            .width(width)
                            .color(Color::from_rgb(0.0, 0.0, 1.0)),
                        snap: false,
                        ..Quad::default()
                    },
                    Color::from_rgb(1.0, 0.0, 0.0),
                );
                let pixels = renderer.screenshot(Size::new(64, 80), 1.0, Color::TRANSPARENT);
                assert!(
                    pixels.chunks_exact(4).all(|p| p[0] == 0),
                    "saturated border must replace all fill height={height} radius={radius} width={width}"
                );
            }
        }
    }
}

#[test]
fn atlas_fragment_rotation_matches_unfragmented_source() {
    let mut renderer = renderer();
    let viewport = Rectangle::with_size(Size::new(160.0, 160.0));
    let bounds = Rectangle {
        x: 30.0,
        y: 45.0,
        width: 100.0,
        height: 60.0,
    };
    let mut screenshots = Vec::new();
    for width in [1, 4097] {
        let handle = iced_core::image::Handle::from_rgba(width, 1, vec![255; width as usize * 4]);
        let _allocation = allocate(&mut renderer, &handle);
        renderer.reset(viewport);
        renderer.draw_image(
            iced_core::image::Image::new(handle)
                .rotation(iced_core::Radians(std::f32::consts::FRAC_PI_4)),
            bounds,
            viewport,
        );
        screenshots.push(renderer.screenshot(Size::new(160, 160), 1.0, Color::TRANSPARENT));
    }
    let error = screenshots[0]
        .chunks_exact(4)
        .zip(screenshots[1].chunks_exact(4))
        .map(|(a, b)| a[3].abs_diff(b[3]))
        .max()
        .unwrap();
    assert!(error <= 3, "fragmented rotation alpha error: {error}/255");
}

#[test]
fn thin_border_and_shadow_are_distance_scaled() {
    use iced_renderer::graphics::shape::{RoundedRectangle, coverage, snap};
    let mut renderer = renderer();
    let viewport = Rectangle::with_size(Size::new(96.0, 80.0));
    for smoothing in [0.0, 0.6, 1.0] {
        for scale in [1.0_f32, 1.25, 1.5, 2.0] {
            for width in [0.5, 1.0, 1.5, 2.0] {
                for phase in [0.0, 0.25, 0.5, 0.75] {
                    let bounds = Rectangle {
                        x: 12.0 + phase,
                        y: 10.0 + phase,
                        width: 64.0,
                        height: 48.0,
                    };
                    for snapping in [false, true] {
                        renderer.reset(viewport);
                        renderer.fill_quad(
                            Quad {
                                bounds,
                                border: Border::default()
                                    .rounded(16)
                                    .smoothing(smoothing)
                                    .width(width)
                                    .color(Color::WHITE),
                                snap: snapping,
                                ..Quad::default()
                            },
                            Color::TRANSPARENT,
                        );
                        let size =
                            Size::new((96.0 * scale).ceil() as u32, (80.0 * scale).ceil() as u32);
                        let pixels = renderer.screenshot(size, scale, Color::TRANSPARENT);
                        let physical = snap(bounds * scale, snapping);
                        let shape =
                            RoundedRectangle::new(physical, (16.0 * scale).into(), smoothing)
                                .unwrap();
                        let mut error = 0;
                        for y in 0..size.height {
                            for x in 0..size.width {
                                let d = shape.distance(iced_core::Point::new(
                                    x as f32 + 0.5,
                                    y as f32 + 0.5,
                                ));
                                let expected =
                                    ((coverage(d) - coverage(d + width * scale)).max(0.0) * 255.0)
                                        .round() as u8;
                                error = error.max(
                                    expected
                                        .abs_diff(pixels[((y * size.width + x) * 4 + 3) as usize]),
                                );
                            }
                        }
                        assert!(
                            error <= 3,
                            "thin border error={error}/255 s={smoothing} scale={scale} width={width} phase={phase} snap={snapping}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn image_crop_rotation_atlas_and_scroll() {
    let mut renderer = renderer();
    let viewport = Rectangle::with_size(Size::new(144.0, 144.0));
    for (width, height, crop) in [
        (
            4097,
            97,
            Rectangle {
                x: 2010,
                y: 5,
                width: 80,
                height: 67,
            },
        ),
        (
            97,
            4097,
            Rectangle {
                x: 5,
                y: 2010,
                width: 67,
                height: 80,
            },
        ),
        (
            4097,
            4097,
            Rectangle {
                x: 2010,
                y: 2010,
                width: 80,
                height: 80,
            },
        ),
    ] {
        let mut source = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                source.extend_from_slice(&[
                    (x % 251) as u8,
                    (y % 251) as u8,
                    ((x + y) % 251) as u8,
                    if (x + y) % 17 == 0 { 0 } else { 255 },
                ]);
            }
        }
        let mut cropped = Vec::with_capacity(crop.width as usize * crop.height as usize * 4);
        for y in crop.y..crop.y + crop.height {
            let start = ((y * width + crop.x) * 4) as usize;
            cropped.extend_from_slice(&source[start..start + crop.width as usize * 4]);
        }
        let source = iced_core::image::Handle::from_rgba(width, height, source);
        let cropped = iced_core::image::Handle::from_rgba(crop.width, crop.height, cropped);
        let _source_allocation = allocate(&mut renderer, &source);
        let _crop_allocation = allocate(&mut renderer, &cropped);
        for angle in [0.0_f32, 15.0, 45.0, 90.0] {
            for scale in [0.75, 1.0, 1.5, 2.0] {
                for opacity in [0.0_f32, 0.5, 1.0] {
                    // Keep the real asynchronous two-dimensional atlas case bounded.
                    if height == 4097
                        && width == 4097
                        && (angle != 45.0 || scale != 1.0 || opacity != 0.5)
                    {
                        continue;
                    }
                    let bounds = Rectangle {
                        x: 72.0 - 40.0 * scale,
                        y: 72.0 - 40.0 * scale,
                        width: 80.0 * scale,
                        height: 80.0 * scale,
                    };
                    let mut shots = Vec::new();
                    for (handle, region) in [(&source, Some(crop)), (&cropped, None)] {
                        renderer.reset(viewport);
                        let mut image = iced_core::image::Image::new(handle)
                            .rotation(iced_core::Radians(angle.to_radians()))
                            .opacity(opacity)
                            .border_radius(24)
                            .border_smoothing(0.6)
                            .snap(false);
                        image.crop = region;
                        renderer.draw_image(image, bounds, viewport);
                        shots.push(renderer.screenshot(
                            Size::new(144, 144),
                            1.0,
                            Color::TRANSPARENT,
                        ));
                    }
                    let error = shots[0]
                        .iter()
                        .zip(&shots[1])
                        .map(|(a, b)| a.abs_diff(*b))
                        .max()
                        .unwrap();
                    assert!(
                        error <= 3,
                        "crop/pre-crop error={error}/255 source={width}x{height} angle={angle} scale={scale} opacity={opacity}"
                    );
                }
            }
        }
    }
}

#[test]
fn scroll_clip_and_software_damage_preserve_shape() {
    let mut renderer = renderer();
    let viewport = Rectangle::with_size(Size::new(160.0, 160.0));
    let bounds = Rectangle {
        x: 16.25,
        y: 16.5,
        width: 120.0,
        height: 120.0,
    };
    let handle = iced_core::image::Handle::from_rgba(1, 1, vec![255; 4]);
    let _allocation = allocate(&mut renderer, &handle);
    let image = iced_core::image::Image::new(handle)
        .border_radius(24)
        .border_smoothing(0.6)
        .snap(false);
    renderer.reset(viewport);
    renderer.draw_image(image.clone(), bounds, bounds);
    let full = renderer.screenshot(Size::new(160, 160), 1.0, Color::TRANSPARENT);
    for clip in [
        Rectangle {
            x: 64.0,
            y: 0.0,
            width: 96.0,
            height: 160.0,
        },
        Rectangle {
            x: 0.0,
            y: 64.0,
            width: 160.0,
            height: 96.0,
        },
    ] {
        renderer.reset(viewport);
        renderer.with_layer(clip, |renderer| {
            renderer.with_translation(Vector::new(3.0, 2.0), |renderer| {
                renderer.draw_image(
                    image.clone(),
                    bounds - Vector::new(3.0, 2.0),
                    bounds - Vector::new(3.0, 2.0),
                );
            });
        });
        let partial = renderer.screenshot(Size::new(160, 160), 1.0, Color::TRANSPARENT);
        for y in 0..160 {
            for x in 0..160 {
                let index = (y * 160 + x) * 4;
                if clip.contains(iced_core::Point::new(x as f32 + 0.5, y as f32 + 0.5)) {
                    assert_eq!(
                        &partial[index..index + 4],
                        &full[index..index + 4],
                        "scroll changed shape at {x},{y}"
                    );
                } else {
                    assert_eq!(partial[index + 3], 0, "scroll leaked at {x},{y}");
                }
            }
        }
    }
}

#[test]
fn image_frame_does_not_pollute_following_image_or_text() {
    let mut renderer = renderer();
    let viewport = Rectangle::with_size(Size::new(160.0, 120.0));
    let handle = iced_core::image::Handle::from_rgba(1, 1, vec![255; 4]);
    let _allocation = allocate(&mut renderer, &handle);
    let mut shots = Vec::new();
    for preceding in [false, true] {
        renderer.reset(viewport);
        if preceding {
            let bounds = Rectangle {
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 40.0,
            };
            renderer.draw_image(
                iced_core::image::Image::new(&handle)
                    .border_radius(20)
                    .border_smoothing(1.0),
                bounds,
                bounds,
            );
        }
        let bounds = Rectangle {
            x: 80.0,
            y: 10.0,
            width: 60.0,
            height: 40.0,
        };
        renderer.draw_image(
            iced_core::image::Image::new(&handle)
                .border_radius(8)
                .border_smoothing(0.6),
            bounds,
            bounds,
        );
        iced_core::text::Renderer::fill_text(
            &mut renderer,
            iced_core::text::Text {
                content: "MASK".into(),
                bounds: Size::new(70.0, 30.0),
                size: iced_core::Pixels(18.0),
                line_height: Default::default(),
                font: Default::default(),
                align_x: Default::default(),
                align_y: iced_core::alignment::Vertical::Top,
                shaping: Default::default(),
                wrapping: Default::default(),
                ellipsis: Default::default(),
                hint_factor: None,
            },
            iced_core::Point::new(80.0, 70.0),
            Color::WHITE,
            viewport,
        );
        shots.push(renderer.screenshot(Size::new(160, 120), 1.0, Color::TRANSPARENT));
    }
    let mut glyph_pixels = 0;
    for y in 0..120 {
        for x in 80..160 {
            let index = (y * 160 + x) * 4;
            assert_eq!(
                &shots[0][index..index + 4],
                &shots[1][index..index + 4],
                "preceding frame clipped later content at {x},{y}"
            );
            if y >= 70 && shots[0][index + 3] > 0 {
                glyph_pixels += 1;
            }
        }
    }
    assert!(glyph_pixels > 0, "regression requires visible text glyphs");
}

#[test]
fn source_crop_clamps_excluded_texels_and_retains_subpixel_coverage() {
    let mut renderer = renderer();
    let viewport = Rectangle::with_size(Size::new(32.0, 32.0));
    let source = iced_core::image::Handle::from_rgba(
        3,
        1,
        vec![255, 0, 255, 255, 0, 255, 0, 255, 255, 0, 255, 255],
    );
    let _allocation = allocate(&mut renderer, &source);
    for width in [0.1, 12.0] {
        renderer.reset(viewport);
        let bounds = Rectangle {
            x: 10.45,
            y: 8.5,
            width,
            height: 12.0,
        };
        renderer.draw_image(
            iced_core::image::Image::new(&source)
                .crop(Rectangle {
                    x: 1,
                    y: 0,
                    width: 1,
                    height: 1,
                })
                .snap(false)
                .rotation(iced_core::Radians(0.25)),
            bounds,
            viewport,
        );
        let pixels = renderer.screenshot(Size::new(32, 32), 1.0, Color::TRANSPARENT);
        assert!(
            pixels.chunks_exact(4).any(|p| p[3] > 0),
            "positive subpixel image was discarded"
        );
        assert!(
            pixels.chunks_exact(4).all(|p| p[0] == 0 && p[2] == 0),
            "excluded magenta texels leaked"
        );
    }
    for crop in [
        Rectangle {
            x: 3,
            y: 0,
            width: 1,
            height: 1,
        },
        Rectangle {
            x: u32::MAX,
            y: 0,
            width: u32::MAX,
            height: 1,
        },
        Rectangle {
            x: 0,
            y: 0,
            width: 0,
            height: 1,
        },
    ] {
        renderer.reset(viewport);
        renderer.draw_image(
            iced_core::image::Image::new(&source).crop(crop),
            viewport,
            viewport,
        );
        assert!(
            renderer
                .screenshot(Size::new(32, 32), 1.0, Color::TRANSPARENT)
                .chunks_exact(4)
                .all(|p| p[3] == 0),
            "empty crop drew pixels"
        );
    }
}

#[test]
fn widget_crop_fitting_and_rotation_match_precropped_image() {
    let mut renderer = renderer();
    let source = iced_core::image::Handle::from_rgba(
        32,
        24,
        (0..32 * 24)
            .flat_map(|i| [(i % 32 * 7) as u8, (i / 32 * 9) as u8, 80, 255])
            .collect::<Vec<_>>(),
    );
    let mut selected = Vec::new();
    for y in 5..17 {
        for x in 7..23 {
            selected.extend_from_slice(&[(x * 7) as u8, (y * 9) as u8, 80, 255]);
        }
    }
    let cropped = iced_core::image::Handle::from_rgba(16, 12, selected);
    let _source_allocation = allocate(&mut renderer, &source);
    let _crop_allocation = allocate(&mut renderer, &cropped);
    for fit in [
        iced::ContentFit::Cover,
        iced::ContentFit::Contain,
        iced::ContentFit::None,
    ] {
        for angle in [0.0_f32, 15.0, 45.0, 90.0] {
            for scale in [0.75_f32, 1.0, 1.5, 2.0] {
                let mut shots = Vec::new();
                for (handle, crop) in [(&source, true), (&cropped, false)] {
                    let mut image = iced::widget::image(handle)
                        .width(96)
                        .height(80)
                        .content_fit(fit)
                        .rotation(iced::Radians(angle.to_radians()))
                        .scale(scale)
                        .border_radius(iced_core::border::top(24))
                        .border_smoothing(0.6)
                        .snap(false);
                    if crop {
                        image = image.crop(Rectangle {
                            x: 7,
                            y: 5,
                            width: 16,
                            height: 12,
                        });
                    }
                    let mut interface = iced_runtime::UserInterface::<(), _, _>::build(
                        image,
                        Size::new(96.0, 80.0),
                        Default::default(),
                        &mut renderer,
                    );
                    interface.draw(
                        &mut renderer,
                        &iced::Theme::Light,
                        &Default::default(),
                        iced_core::mouse::Cursor::Unavailable,
                    );
                    shots.push(renderer.screenshot(Size::new(96, 80), 1.0, Color::TRANSPARENT));
                }
                let error = shots[0]
                    .iter()
                    .zip(&shots[1])
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap();
                assert!(
                    error <= 3,
                    "widget crop error={error} fit={fit:?} angle={angle} scale={scale}"
                );
            }
        }
    }
}

#[cfg(feature = "tiny-skia")]
#[test]
fn full_source_atlas_coordinates_match_software_reference() {
    let mut actual = renderer();
    let mut reference = iced::futures::executor::block_on(iced::Renderer::new(
        Default::default(),
        Some("tiny-skia"),
    ))
    .expect("software reference renderer");
    for (width, height) in [(4097, 97), (97, 4097), (4097, 4097)] {
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&[
                    (x % 251) as u8,
                    (y % 251) as u8,
                    ((x / 13 + y / 17) % 251) as u8,
                    255,
                ]);
            }
        }
        let handle = iced_core::image::Handle::from_rgba(width, height, rgba);
        let _actual_allocation = allocate(&mut actual, &handle);
        let _reference_allocation = allocate(&mut reference, &handle);
        let side = (width.max(height) as f32 * 0.25 + 32.0).ceil() as u32;
        let bounds = Rectangle {
            x: (side as f32 - width as f32 * 0.25) * 0.5,
            y: (side as f32 - height as f32 * 0.25) * 0.5,
            width: width as f32 * 0.25,
            height: height as f32 * 0.25,
        };
        let size = Size::new(side, side);
        let viewport = Rectangle::with_size(Size::new(side as f32, side as f32));
        for angle in [0.0, std::f32::consts::FRAC_PI_4] {
            let mut shots = Vec::new();
            for renderer in [&mut actual, &mut reference] {
                renderer.reset(viewport);
                renderer.draw_image(
                    iced_core::image::Image::new(&handle)
                        .snap(false)
                        .rotation(iced_core::Radians(angle)),
                    bounds,
                    viewport,
                );
                shots.push(renderer.screenshot(size, 1.0, Color::TRANSPARENT));
            }
            let error = shots[0]
                .iter()
                .zip(&shots[1])
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(
                error <= 3,
                "full source coordinate/seam error={error}/255 source={width}x{height} angle={angle}"
            );
        }
    }
}

#[test]
fn shadow_remains_visible_when_original_quad_is_clipped_out() {
    use iced_renderer::graphics::shape::{RoundedRectangle, coverage};
    let mut renderer = renderer();
    let viewport = Rectangle::with_size(Size::new(80.0, 80.0));
    let clip = Rectangle {
        x: 40.0,
        y: 0.0,
        width: 40.0,
        height: 80.0,
    };
    let bounds = Rectangle {
        x: 10.0,
        y: 20.0,
        width: 20.0,
        height: 30.0,
    };
    for blur in [0.0, 4.0, 16.0] {
        for gradient in [false, true] {
            renderer.reset(viewport);
            renderer.with_layer(clip, |renderer| {
                let quad = Quad {
                    bounds,
                    border: Border::default().rounded(8).smoothing(0.6),
                    shadow: Shadow {
                        color: Color::WHITE,
                        offset: Vector::new(30.0, 0.0),
                        blur_radius: blur,
                    },
                    snap: false,
                };
                if gradient {
                    renderer.fill_quad(
                        quad,
                        iced_core::gradient::Linear::new(iced_core::Radians(0.0))
                            .add_stop(0.0, Color::WHITE)
                            .add_stop(1.0, Color::WHITE),
                    );
                } else {
                    renderer.fill_quad(quad, Color::WHITE);
                }
            });
            let rgba = renderer.screenshot(Size::new(80, 80), 1.0, Color::TRANSPARENT);
            let shadow =
                RoundedRectangle::new(bounds + Vector::new(30.0, 0.0), 8.0.into(), 0.6).unwrap();
            for y in 0..80 {
                for x in 40..80 {
                    let d = shadow.distance(iced_core::Point::new(x as f32 + 0.5, y as f32 + 0.5));
                    let alpha = if blur == 0.0 {
                        coverage(d)
                    } else {
                        let t = ((d + blur) / (2.0 * blur)).clamp(0.0, 1.0);
                        1.0 - t * t * (3.0 - 2.0 * t)
                    };
                    let expected = (alpha * 255.0).round() as u8;
                    assert!(
                        expected.abs_diff(rgba[(y * 80 + x) * 4 + 3]) <= 3,
                        "clipped original lost shadow at {x},{y} blur={blur} gradient={gradient}"
                    );
                }
            }
        }
    }
}
