//! Opt-in offscreen measurements, not generic performance guarantees.
#![cfg(all(feature = "image", feature = "wgpu"))]

use iced_core::image::Renderer as _;
use iced_core::renderer::{Quad, Renderer as _, Scale};
use iced_core::{Color, Rectangle, Size};
use iced_wgpu::{Engine, Renderer, graphics, wgpu};
use std::time::Instant;

#[test]
#[ignore = "opt-in release GPU timing; reads query timestamps, not scene pixels"]
fn measure_native_blur() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter =
        iced::futures::executor::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .unwrap();
    let features =
        wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    assert!(
        adapter.features().contains(features),
        "actual GPU timestamps unavailable on this adapter"
    );
    eprintln!(
        "adapter={:?}; optimized={}",
        adapter.get_info(),
        !cfg!(debug_assertions)
    );
    let (device, queue) =
        iced::futures::executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: features,
            ..Default::default()
        }))
        .unwrap();
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let engine = Engine::new(
        &adapter,
        device.clone(),
        queue.clone(),
        format,
        None,
        graphics::Shell::headless(),
    );
    let mut renderer = Renderer::new(engine, Default::default());
    let query = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: None,
        ty: wgpu::QueryType::Timestamp,
        count: 2,
    });
    let resolved = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let pixels: Vec<u8> = (0..1920 * 1080)
        .flat_map(|i| {
            if (i % 1920) / 32 % 2 == 0 {
                [240, 80, 20, 255]
            } else {
                [20, 60, 220, 255]
            }
        })
        .collect();
    let handle = iced_core::image::Handle::from_rgba(1920, 1080, pixels.clone());
    let changed = iced_core::image::Handle::from_rgba(1920, 1080, pixels);
    let _first = renderer.load_image(&handle).unwrap();
    let _changed = renderer.load_image(&changed).unwrap();
    for (name, width, height, blur, modal, source_changed) in [
        ("direct", 1920, 1080, false, false, false),
        ("image-miss", 1920, 1080, true, false, false),
        ("image-hit", 1920, 1080, true, false, false),
        ("image-source-change", 1920, 1080, true, false, true),
        ("scene-miss", 1920, 1080, true, true, true),
        ("scene-hit", 1920, 1080, true, true, true),
        ("resize-miss", 2560, 1440, true, true, true),
        ("resize-hit", 2560, 1440, true, true, true),
    ] {
        let viewport = graphics::Viewport::with_physical_size(
            Size::new(width, height),
            Scale {
                window: 1.0,
                application: 1.0,
            },
        );
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bounds = Rectangle::with_size(Size::new(width as f32, height as f32));
        renderer.reset(bounds);
        let source = if source_changed { &changed } else { &handle };
        renderer.draw_image(iced_core::Image::new(source), bounds, bounds);
        if blur {
            for x in [100.0, 320.0, 540.0] {
                let mask = Rectangle {
                    x,
                    y: 700.0,
                    width: 180.0,
                    height: 48.0,
                };
                renderer.with_layer(mask, |r| {
                    r.draw_image(
                        iced_core::Image::new(source).blur(12.0).border_radius(12.0),
                        bounds,
                        mask,
                    )
                });
            }
        }
        if modal {
            renderer.blur_backdrop(12.0);
            renderer.fill_quad(
                Quad {
                    bounds: Rectangle {
                        x: 600.0,
                        y: 300.0,
                        width: 500.0,
                        height: 400.0,
                    },
                    ..Default::default()
                },
                Color::WHITE,
            );
        }
        let mut before = device.create_command_encoder(&Default::default());
        before.write_timestamp(&query, 0);
        let cpu_start = Instant::now();
        let draw = renderer.draw(Some(Color::BLACK), &view, &viewport);
        let cpu_encode = cpu_start.elapsed();
        renderer.finish();
        let mut after = device.create_command_encoder(&Default::default());
        after.write_timestamp(&query, 1);
        after.resolve_query_set(&query, 0..2, &resolved, 0);
        after.copy_buffer_to_buffer(&resolved, 0, &readback, 0, 16);
        let submit_start = Instant::now();
        let index = queue.submit([before.finish(), draw.finish(), after.finish()]);
        let cpu_submit = submit_start.elapsed();
        renderer.recall();
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
        let _ = device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: None,
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let mapping = readback.slice(..).get_mapped_range().unwrap();
        let start = u64::from_le_bytes(mapping[0..8].try_into().unwrap());
        let end = u64::from_le_bytes(mapping[8..16].try_into().unwrap());
        let gpu_ms = (end - start) as f64 * f64::from(queue.get_timestamp_period()) / 1_000_000.0;
        drop(mapping);
        readback.unmap();
        eprintln!(
            "{name}: {width}x{height}; CPU encode={cpu_encode:?}, submit={cpu_submit:?}, GPU={gpu_ms:.3}ms; {:?}",
            renderer.blur_statistics()
        );
    }
}

#[cfg(feature = "tiny-skia")]
#[test]
#[ignore = "opt-in release software timing; includes final offscreen pixel copy"]
fn measure_software_blur() {
    use iced_core::renderer::Headless;
    let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
        Default::default(),
        Some("tiny-skia"),
    ))
    .unwrap();
    assert_eq!(renderer.name(), "tiny-skia");
    let source: Vec<u8> = (0..1920 * 1080)
        .flat_map(|i| {
            if i % 1920 / 32 % 2 == 0 {
                [240, 80, 20, 255]
            } else {
                [20, 60, 220, 255]
            }
        })
        .collect();
    let handle = iced_core::image::Handle::from_rgba(1920, 1080, source.clone());
    let changed = iced_core::image::Handle::from_rgba(1920, 1080, source);
    let _first = renderer.load_image(&handle).unwrap();
    let _changed = renderer.load_image(&changed).unwrap();
    for (name, width, height, blur, modal, source_changed) in [
        ("software-direct", 1280, 720, false, false, false),
        ("software-image-miss", 1280, 720, true, false, false),
        ("software-image-hit", 1280, 720, true, false, false),
        ("software-source-change", 1280, 720, true, false, true),
        ("software-scene-miss", 1280, 720, true, true, true),
        ("software-scene-hit", 1280, 720, true, true, true),
        ("software-resize-miss", 1920, 1080, true, true, true),
        ("software-resize-hit", 1920, 1080, true, true, true),
    ] {
        let bounds = Rectangle::with_size(Size::new(width as f32, height as f32));
        let source = if source_changed { &changed } else { &handle };
        renderer.reset(bounds);
        renderer.draw_image(iced_core::Image::new(source), bounds, bounds);
        if blur {
            for x in [100.0, 320.0, 540.0] {
                let mask = Rectangle {
                    x,
                    y: 500.0,
                    width: 180.0,
                    height: 48.0,
                };
                renderer.with_layer(mask, |r| {
                    r.draw_image(
                        iced_core::Image::new(source).blur(12.0).border_radius(12.0),
                        bounds,
                        mask,
                    );
                });
            }
        }
        if modal {
            renderer.blur_backdrop(12.0);
            renderer.fill_quad(
                Quad {
                    bounds: Rectangle {
                        x: 400.0,
                        y: 200.0,
                        width: 400.0,
                        height: 300.0,
                    },
                    ..Default::default()
                },
                Color::WHITE,
            );
        }
        let start = Instant::now();
        let pixels = renderer.screenshot(Size::new(width, height), 1.0, Color::BLACK);
        let cpu = start.elapsed();
        drop(std::hint::black_box(pixels));
        eprintln!(
            "{name}: {width}x{height}; CPU full raster+copy={cpu:?}; {:?}",
            renderer.blur_statistics()
        );
    }
}
