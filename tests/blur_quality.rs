//! Quality regressions: repetitive source detail must not return as blur bands.
#![cfg(all(feature = "image", any(feature = "tiny-skia", feature = "wgpu")))]

use iced_core::image::Renderer as _;
use iced_core::renderer::{Backdrop, Headless, Renderer as _};
use iced_core::{Blur, Color, Rectangle, Size};

fn renderer() -> iced::Renderer {
    let backend = std::env::var("ICED_TEST_BACKEND").unwrap_or_else(|_| "tiny-skia".into());
    let renderer =
        iced::futures::executor::block_on(iced::Renderer::new(Default::default(), Some(&backend)))
            .unwrap();
    assert_eq!(renderer.name(), backend);
    renderer
}

fn save(name: &str, pixels: &[u8], width: u32, height: u32) {
    if let Ok(directory) = std::env::var("ICED_BLUR_CAPTURE") {
        std::fs::create_dir_all(&directory).unwrap();
        image::save_buffer(
            std::path::Path::new(&directory).join(format!("{name}.png")),
            pixels,
            width,
            height,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
}

fn stripes(renderer: &iced::Renderer, width: u32, height: u32) -> iced_core::image::Handle {
    let pixels = (0..width * height)
        .flat_map(|i| {
            let value = if i % width % 2 == 0 { 0 } else { 255 };
            [value, value, value, 255]
        })
        .collect::<Vec<_>>();
    let handle = iced_core::image::Handle::from_rgba(width, height, pixels);
    let _ = renderer.load_image(&handle).unwrap();
    handle
}

#[test]
fn reduced_quality_glass_preserves_the_average_of_fine_detail() {
    let mut renderer = renderer();
    let (width, height) = (256, 256);
    // These rows are perpendicular to the first filtering axis. Reducing both
    // axes before the second pass can turn them into a solid dark/light field.
    let pixels = (0..width * height)
        .flat_map(|i| {
            let value = if i / width % 4 == 3 { 255 } else { 0 };
            [value, value, value, 255]
        })
        .collect::<Vec<_>>();
    let handle = iced_core::image::Handle::from_rgba(width, height, pixels);
    let _ = renderer.load_image(&handle).unwrap();
    let bounds = Rectangle::with_size(Size::new(width as f32, height as f32));
    for fraction in [1.0, 0.75, 0.5, 0.25] {
        renderer.reset(bounds);
        renderer.draw_image(iced_core::Image::new(&handle).snap(false), bounds, bounds);
        renderer.draw_backdrop(Backdrop {
            bounds,
            blur: 16.0.into(),
            optics: Some(iced_core::glass::Optics::default()),
            quality: iced_core::glass::Quality::Fixed(fraction),
            ..Default::default()
        });
        let pixels = renderer.screenshot(Size::new(width, height), 1.0, Color::TRANSPARENT);
        save(&format!("glass-quality-{fraction}"), &pixels, width, height);
        let expected = if cfg!(feature = "web-colors") {
            64
        } else {
            137
        };
        for y in 120..136 {
            for x in 120..136 {
                let value = pixels[((y * width + x) * 4) as usize];
                assert!(
                    value.abs_diff(expected) <= 4,
                    "quality {fraction} changed average brightness: {value}, expected {expected}"
                );
            }
        }
    }
}

#[test]
fn progressive_blur_does_not_reconstruct_fine_stripes_as_bands() {
    let mut renderer = renderer();
    let (width, height) = (768, 256);
    let handle = stripes(&renderer, width, height);
    let bounds = Rectangle::with_size(Size::new(width as f32, height as f32));
    renderer.reset(bounds);
    renderer.draw_image(
        iced_core::Image::new(&handle)
            .blur(Blur::vertical_gradient(0.0, 64.0))
            .snap(false),
        bounds,
        bounds,
    );
    let pixels = renderer.screenshot(Size::new(width, height), 1.0, Color::TRANSPARENT);
    save("progressive-stripes", &pixels, width, height);
    let mut lowest = 255;
    let mut highest = 0;
    for y in 48..224 {
        for x in 380..388 {
            let value = pixels[((y * width + x) * 4) as usize];
            lowest = lowest.min(value);
            highest = highest.max(value);
        }
    }
    // Far from image edges, dense Gaussian filtering of alternating pixels
    // at sigma>=12 is uniformly mid-gray, regardless of output position.
    assert!(
        highest - lowest <= 4,
        "blur revived periodic detail: brightness {lowest}..{highest}"
    );
    let expected = if cfg!(feature = "web-colors") {
        128
    } else {
        188
    };
    assert!(
        lowest.abs_diff(expected) <= 4,
        "blur lost source brightness: {lowest}"
    );
}

#[test]
fn glass_filter_does_not_alias_fine_background_detail() {
    let mut renderer = renderer();
    let (width, height) = (768, 64);
    let handle = stripes(&renderer, width, height);
    let bounds = Rectangle::with_size(Size::new(width as f32, height as f32));
    renderer.reset(bounds);
    renderer.draw_image(iced_core::Image::new(&handle).snap(false), bounds, bounds);
    renderer.draw_backdrop(Backdrop {
        bounds,
        blur: 16.0.into(),
        optics: Some(iced_core::glass::Optics::default()),
        ..Default::default()
    });
    let pixels = renderer.screenshot(Size::new(width, height), 1.0, Color::TRANSPARENT);
    save("glass-stripes", &pixels, width, height);
    let values = (360..400)
        .map(|x| pixels[((32 * width + x) * 4) as usize])
        .collect::<Vec<_>>();
    let range = values.iter().max().unwrap() - values.iter().min().unwrap();
    assert!(
        range <= 4,
        "glass blur revived periodic detail: contrast {range}"
    );
    let expected = if cfg!(feature = "web-colors") {
        128
    } else {
        188
    };
    assert!(
        values[0].abs_diff(expected) <= 4,
        "glass lost source brightness: {}",
        values[0]
    );
}

#[test]
fn uniform_image_blur_preserves_fine_detail_average() {
    let mut renderer = renderer();
    let (width, height) = (256, 256);
    let pixels = (0..width * height)
        .flat_map(|i| {
            let value = if i % width % 4 == 3 { 255 } else { 0 };
            [value, value, value, 255]
        })
        .collect::<Vec<_>>();
    let handle = iced_core::image::Handle::from_rgba(width, height, pixels);
    let _ = renderer.load_image(&handle).unwrap();
    let bounds = Rectangle::with_size(Size::new(width as f32, height as f32));
    renderer.reset(bounds);
    renderer.draw_image(
        iced_core::Image::new(&handle).blur(32.0).snap(false),
        bounds,
        bounds,
    );
    let pixels = renderer.screenshot(Size::new(width, height), 1.0, Color::TRANSPARENT);
    save("uniform-image-stripes", &pixels, width, height);
    let expected = if cfg!(feature = "web-colors") {
        64
    } else {
        137
    };
    for y in 120..136 {
        for x in 120..136 {
            let value = pixels[((y * width + x) * 4) as usize];
            assert!(
                value.abs_diff(expected) <= 4,
                "uniform image blur changed average brightness: {value}, expected {expected}"
            );
        }
    }
}

#[cfg(feature = "wgpu")]
#[test]
fn wide_uniform_image_keeps_high_dpi_filter_tiles_within_texture_limits() {
    if std::env::var("ICED_TEST_BACKEND").as_deref() != Ok("wgpu") {
        return;
    }
    let mut renderer = renderer();
    let handle = stripes(&renderer, 4096, 128);
    let viewport = Rectangle::with_size(Size::new(4.0, 8.0));
    let bounds = Rectangle {
        x: -500.0,
        y: 0.0,
        width: 1024.0,
        height: 32.0,
    };
    renderer.reset(viewport);
    renderer.draw_image(
        iced_core::Image::new(&handle).blur(64.0).snap(false),
        bounds,
        bounds,
    );
    // The reduced output grid rounds outwards. Its gutter plus a sigma256
    // physical halo must still fit the bounded intermediate texture.
    let pixels = renderer.screenshot(Size::new(16, 32), 4.0, Color::TRANSPARENT);
    let gray = if cfg!(feature = "web-colors") {
        128
    } else {
        188
    };
    for pixel in pixels.chunks_exact(4) {
        assert!(pixel[0].abs_diff(gray) <= 4 && pixel[3] == 255, "{pixel:?}");
    }
    assert!(renderer.blur_statistics().retained_bytes < 64 * 1024 * 1024);
}

#[cfg(feature = "wgpu")]
#[test]
fn oversized_progressive_kernel_filters_detail_across_atlas_fragments() {
    if std::env::var("ICED_TEST_BACKEND").as_deref() != Ok("wgpu") {
        return;
    }
    let mut renderer = renderer();
    let handle = stripes(&renderer, 2400, 32);
    let transparent = iced_core::image::Handle::from_rgba(
        2400,
        32,
        (0..2400 * 32)
            .flat_map(|i| {
                if i % 2 == 0 {
                    [0, 0, 0, 255]
                } else {
                    [255, 0, 0, 0]
                }
            })
            .collect::<Vec<_>>(),
    );
    let _ = renderer.load_image(&transparent).unwrap();
    let viewport = Rectangle::with_size(Size::new(4.0, 8.0));
    let bounds = Rectangle {
        x: -300.0,
        y: 0.0,
        width: 600.0,
        height: 8.0,
    };
    for (axis, profile) in [
        (
            "vertical",
            Blur::vertical_gradient(0.0, 128.0).range(0.25, 0.5),
        ),
        (
            "horizontal",
            Blur::horizontal_gradient(0.0, 128.0).range(0.1, 0.2),
        ),
    ] {
        for (alpha, image) in [(false, &handle), (true, &transparent)] {
            renderer.reset(viewport);
            renderer.draw_image(
                iced_core::Image::new(image).blur(profile).snap(false),
                bounds,
                bounds,
            );
            // Sigma 512 physical pixels exceeds the tile halo budget. The
            // kernel crosses atlas fragments with only a small viewport visible.
            let pixels = renderer.screenshot(Size::new(16, 32), 4.0, Color::TRANSPARENT);
            save(&format!("oversized-{axis}-alpha-{alpha}"), &pixels, 16, 32);
            let gray = if cfg!(feature = "web-colors") {
                128
            } else {
                188
            };
            let expected = if alpha {
                [0, 0, 0, 128]
            } else {
                [gray, gray, gray, 255]
            };
            for y in 20..30 {
                for x in 2..14 {
                    let pixel = &pixels[(y * 16 + x) * 4..(y * 16 + x + 1) * 4];
                    assert!(
                        pixel
                            .iter()
                            .zip(expected)
                            .all(|(value, target)| value.abs_diff(target) <= 4),
                        "oversized {axis} blur changed detail/alpha at {x},{y}: {pixel:?}"
                    );
                }
            }
        }
    }
    assert!(renderer.blur_statistics().retained_bytes < 64 * 1024 * 1024);
}

#[test]
fn deep_glass_has_no_interior_lighting_crease() {
    let mut renderer = renderer();
    let bounds = Rectangle::with_size(Size::new(160.0, 160.0));
    renderer.reset(bounds);
    renderer.fill_quad(
        iced_core::renderer::Quad {
            bounds,
            ..Default::default()
        },
        Color::from_rgb(0.3, 0.3, 0.3),
    );
    renderer.draw_backdrop(Backdrop {
        bounds: Rectangle {
            x: 16.0,
            y: 16.0,
            width: 128.0,
            height: 128.0,
        },
        border_radius: 16.0.into(),
        border_smoothing: 0.6,
        optics: Some(iced_core::glass::Optics {
            depth: 96.0,
            highlight: 0.8,
            shadow: 0.4,
            light: iced_core::Vector::new(-1.0, 0.0),
            ..Default::default()
        }),
        ..Default::default()
    });
    let pixels = renderer.screenshot(Size::new(160, 160), 1.0, Color::TRANSPARENT);
    save("deep-glass", &pixels, 160, 160);
    let mut maximum = 0;
    // Entirely inside the smooth material; crossing the nearest-edge bisector
    // must not introduce a one-pixel jump in illumination.
    for y in 42..118 {
        for x in 42..117 {
            let a = pixels[(y * 160 + x) * 4];
            let b = pixels[(y * 160 + x + 1) * 4];
            maximum = maximum.max(a.abs_diff(b));
        }
    }
    assert!(
        maximum <= 8,
        "deep lens has an interior lighting crease of {maximum} levels"
    );
}

#[test]
fn deep_glass_does_not_fold_a_smooth_background_at_its_center() {
    let mut renderer = renderer();
    renderer.reset(Rectangle::with_size(Size::new(160.0, 160.0)));
    for x in 0..160 {
        renderer.fill_quad(
            iced_core::renderer::Quad {
                bounds: Rectangle {
                    x: x as f32,
                    y: 0.0,
                    width: 1.0,
                    height: 160.0,
                },
                ..Default::default()
            },
            Color::from_rgb(x as f32 / 159.0, 0.2, 0.4),
        );
    }
    renderer.draw_backdrop(Backdrop {
        bounds: Rectangle {
            x: 16.0,
            y: 16.0,
            width: 128.0,
            height: 128.0,
        },
        border_radius: 16.0.into(),
        border_smoothing: 0.6,
        optics: Some(iced_core::glass::Optics {
            depth: 96.0,
            refraction: 24.0,
            ..Default::default()
        }),
        ..Default::default()
    });
    let pixels = renderer.screenshot(Size::new(160, 160), 1.0, Color::TRANSPARENT);
    save("deep-glass-refraction", &pixels, 160, 160);
    for y in 42..118 {
        for x in 42..117 {
            let a = pixels[(y * 160 + x) * 4];
            let b = pixels[(y * 160 + x + 1) * 4];
            assert!(
                b + 1 >= a && a.abs_diff(b) <= 5,
                "smooth lens folded or creased the background at {x},{y}: {a}->{b}"
            );
        }
    }
}
