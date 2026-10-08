//! Read native FP16 scene values without the screenshot display conversion.
#![cfg(feature = "wgpu")]

use iced_core::glass::{Optics, Quality};
use iced_core::renderer::{Backdrop, Quad, Renderer as _};
use iced_core::{Blur, Color, Rectangle, Size};
use iced_wgpu::graphics::{Shell, Viewport};
use iced_wgpu::primitive::Renderer as _;
use iced_wgpu::{Engine, Primitive, Renderer, primitive, wgpu};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const SIZE: u32 = 128;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[derive(Default)]
struct QueueGate(AtomicBool);

// SAFETY: successful acquire grants exclusive ownership to the calling thread;
// release publishes its writes. QueueGuard keeps unlock on that owning thread.
#[allow(unsafe_code)]
unsafe impl iced_wgpu::QueueSynchronization for QueueGate {
    fn lock(&self) {
        while self
            .0
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::hint::spin_loop();
        }
    }

    unsafe fn unlock(&self) {
        self.0.store(false, Ordering::Release);
    }
}

struct Scene {
    renderer: Renderer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    gate: Arc<QueueGate>,
    target: wgpu::Texture,
}

impl Scene {
    fn new() -> Self {
        iced::futures::executor::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY),
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            });
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .unwrap();
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default())
                .await
                .unwrap();
            let gate = Arc::new(QueueGate::default());
            let engine = Engine::new_with_queue_synchronization(
                &adapter,
                device.clone(),
                queue.clone(),
                FORMAT,
                None,
                Shell::headless(),
                gate.clone(),
            );
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("floating scene numerical readback"),
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            Self {
                renderer: Renderer::new(engine, Default::default()),
                device,
                queue,
                gate,
                target,
            }
        })
    }

    fn begin(&mut self, left: [f32; 4], right: [f32; 4]) {
        self.renderer
            .reset(rect(0.0, 0.0, SIZE as f32, SIZE as f32));
        self.renderer.draw_primitive(
            rect(0.0, 0.0, SIZE as f32, SIZE as f32),
            VideoFrame { left, right },
        );
    }

    fn read(&mut self) -> Vec<[f32; 4]> {
        let view = self.target.create_view(&Default::default());
        let _ = self.renderer.present(
            Some(Color::TRANSPARENT),
            FORMAT,
            &view,
            &Viewport::with_physical_size(Size::new(SIZE, SIZE), Default::default()),
        );
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("raw FP16 scene bytes"),
            size: u64::from(SIZE * SIZE * 8),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 8),
                    rows_per_image: None,
                },
            },
            self.target.size(),
        );
        let submission = {
            let _guard = iced_wgpu::QueueGuard::acquire(self.gate.as_ref());
            self.queue.submit([encoder.finish()])
        };
        let (send, receive) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap()
            });
        let _ = self
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        receive.recv().unwrap().unwrap();
        let bytes = buffer.slice(..).get_mapped_range().unwrap();
        bytes
            .chunks_exact(8)
            .map(|pixel| {
                std::array::from_fn(|channel| {
                    half::f16::from_le_bytes([pixel[channel * 2], pixel[channel * 2 + 1]]).to_f32()
                })
            })
            .collect()
    }
}

struct VideoFrame {
    left: [f32; 4],
    right: [f32; 4],
}

impl std::fmt::Debug for VideoFrame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A live video's pixels can change without its primitive identity changing.
        formatter.write_str("VideoFrame")
    }
}

struct VideoPipeline {
    pipeline: wgpu::RenderPipeline,
    binding: wgpu::BindGroup,
    colors: wgpu::Buffer,
}

impl primitive::Pipeline for VideoPipeline {
    fn new(device: &wgpu::Device, _: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        assert_eq!(format, FORMAT);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("extended-range video fixture"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
                struct Colors { left: vec4<f32>, right: vec4<f32> }
                @group(0) @binding(0) var<uniform> colors: Colors;
                @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                    let p = array<vec2<f32>, 3>(vec2(-1., -1.), vec2(3., -1.), vec2(-1., 3.));
                    return vec4(p[i], 0., 1.);
                }
                @fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
                    return select(colors.left, colors.right, p.x >= 64.);
                }
            "#
                .into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let colors = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: colors.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            binding,
            colors,
        }
    }
}

impl Primitive for VideoFrame {
    type Pipeline = VideoPipeline;
    fn prepare(
        &self,
        pipeline: &mut VideoPipeline,
        _: &wgpu::Device,
        queue: &wgpu::Queue,
        _: &Rectangle,
        _: &Viewport,
    ) {
        let bytes = self
            .left
            .into_iter()
            .chain(self.right)
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>();
        queue.write_buffer(&pipeline.colors, 0, &bytes);
    }
    fn draw(&self, pipeline: &VideoPipeline, pass: &mut wgpu::RenderPass<'_>) -> bool {
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &pipeline.binding, &[]);
        pass.draw(0..3, 0..1);
        true
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rectangle {
    Rectangle {
        x,
        y,
        width,
        height,
    }
}
fn pixel(values: &[[f32; 4]], x: usize, y: usize) -> [f32; 4] {
    values[y * SIZE as usize + x]
}
fn close(actual: [f32; 4], expected: [f32; 4]) {
    for (a, e) in actual.into_iter().zip(expected) {
        assert!(
            (a - e).abs() < 0.025,
            "FP16 scene value {actual:?}, expected {expected:?}"
        );
    }
}

#[test]
fn extended_brightness_survives_every_scene_filter_path() {
    let mut scene = Scene::new();
    let input = [4.0, 2.0, 0.5, 0.5];
    let local = Backdrop {
        bounds: rect(16.0, 16.0, 96.0, 96.0),
        border_radius: 18.0.into(),
        border_smoothing: 0.6,
        blur: 16.0.into(),
        ..Default::default()
    };
    for effect in [
        Backdrop {
            bounds: rect(0.0, 0.0, 128.0, 128.0),
            blur: 16.0.into(),
            ..Default::default()
        },
        local,
        Backdrop {
            blur: Blur::vertical_gradient(0.0, 24.0),
            ..local
        },
        Backdrop {
            blur: Blur::horizontal_gradient(24.0, 0.0),
            ..local
        },
        Backdrop {
            optics: Some(Optics {
                refraction: 12.0,
                depth: 32.0,
                ..Default::default()
            }),
            ..local
        },
        Backdrop {
            quality: Quality::Balanced,
            optics: Some(Optics::default()),
            ..local
        },
        Backdrop {
            quality: Quality::Performance,
            optics: Some(Optics::default()),
            ..local
        },
    ] {
        scene.begin(input, input);
        scene.renderer.draw_backdrop(effect);
        let output = scene.read();
        for (x, y) in [(0, 0), (16, 16), (20, 64), (64, 32), (64, 64), (96, 64)] {
            close(pixel(&output, x, y), input);
        }
    }
    // Tint changes the material intentionally, but must not clip extended RGB.
    scene.begin(input, input);
    scene.renderer.draw_backdrop(Backdrop {
        optics: Some(Optics {
            tint: Color::from_rgba(1.0, 0.0, 0.0, 0.25),
            ..Default::default()
        }),
        ..local
    });
    close(pixel(&scene.read(), 64, 64), [3.25, 1.5, 0.375, 0.625]);

    // Active lighting may intentionally shade HDR content, but must not turn
    // the whole extended range into display white or alter its coverage.
    scene.begin(input, input);
    scene.renderer.draw_backdrop(Backdrop {
        optics: Some(Optics {
            depth: 32.0,
            refraction: 12.0,
            highlight: 0.4,
            shadow: 0.2,
            ..Default::default()
        }),
        ..local
    });
    let lit = pixel(&scene.read(), 20, 64);
    assert!(
        lit[0] > 2.0 && lit[0] < 3.9,
        "lighting clipped or ignored HDR input: {lit:?}"
    );
    assert!(
        (lit[3] - input[3]).abs() < 0.002,
        "lighting changed alpha: {lit:?}"
    );
}

#[test]
fn floating_video_keeps_premultiplied_alpha_clip_and_foreground_order() {
    let mut scene = Scene::new();
    let clip = rect(48.0, 24.0, 32.0, 80.0);
    let effect = Backdrop {
        bounds: rect(32.0, 16.0, 64.0, 96.0),
        border_radius: 20.0.into(),
        border_smoothing: 0.6,
        blur: 10.0.into(),
        optics: Some(Optics::default()),
        quality: Quality::Performance,
        ..Default::default()
    };
    for gain in [1.0, 2.0] {
        let left = [4.0 * gain, 2.0 * gain, 0.5 * gain, 0.5];
        let right = [0.5 * gain, 1.5 * gain, 3.0 * gain, 0.25];
        scene.begin(left, right);
        let plain = scene.read();
        scene.begin(left, right);
        scene
            .renderer
            .with_layer(clip, |renderer| renderer.draw_backdrop(effect));
        scene.renderer.fill_quad(
            Quad {
                bounds: rect(60.0, 76.0, 8.0, 8.0),
                ..Default::default()
            },
            Color::from_rgb(0.0, 1.0, 0.0),
        );
        let output = scene.read();
        for (x, y) in [(16, 64), (47, 64), (80, 64), (64, 23), (64, 104), (32, 16)] {
            assert_eq!(
                pixel(&output, x, y),
                pixel(&plain, x, y),
                "clip changed exterior at {x},{y}"
            );
        }
        close(pixel(&output, 64, 80), [0.0, 1.0, 0.0, 1.0]);
        for x in [58, 63, 64, 69] {
            let value = pixel(&output, x, 64);
            assert!(
                value[3] > 0.28 && value[3] < 0.47,
                "boundary was not filtered: {value:?}"
            );
            let weight = (value[3] - right[3]) / (left[3] - right[3]);
            close(
                value,
                std::array::from_fn(|i| right[i] + (left[i] - right[i]) * weight),
            );
            assert!(
                value[0] > 1.0 && value[1] > 1.0,
                "extended brightness clipped: {value:?}"
            );
        }
    }
}
