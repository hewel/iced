//! Read native FP16 scene values without the screenshot display conversion.
#![cfg(feature = "wgpu")]

use iced_core::renderer::{Backdrop, Quad, Renderer as _};
use iced_core::{Blur, Color, Rectangle, Size};
use iced_wgpu::graphics::{Shell, Viewport};
use iced_wgpu::primitive::Renderer as _;
use iced_wgpu::{Engine, Primitive, Renderer, primitive, wgpu};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[path = "support/player_backdrop.rs"]
mod composition;

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
fn live_bottom_gradient_preserves_hdr_alpha_clip_and_foreground() {
    let mut scene = Scene::new();
    let effect = Backdrop {
        bounds: rect(0.0, 64.0, 128.0, 64.0),
        blur: Blur::vertical_gradient(0.0, 16.0).range(0.15, 1.0),
        border_radius: 12.0.into(),
        border_smoothing: 0.6,
    };
    let clip = rect(48.0, 80.0, 32.0, 40.0);
    let mut previous: Option<[f32; 4]> = None;
    for gain in [1.0, 2.0] {
        let left = [4.0 * gain, 2.0 * gain, 0.5 * gain, 0.5];
        let right = [0.5 * gain, 1.5 * gain, 3.0 * gain, 0.25];
        scene.begin(left, right);
        let plain = scene.read();
        scene.begin(left, right);
        scene.renderer.draw_backdrop(effect);
        let full = scene.read();
        // The upper picture and zero-radius plateau stay exactly sharp.
        for (x, y) in [(63, 32), (64, 63), (63, 68), (64, 68), (0, 64)] {
            assert_eq!(pixel(&full, x, y), pixel(&plain, x, y));
        }
        let lower = pixel(&full, 58, 112);
        let upper = pixel(&full, 58, 80);
        assert!(
            lower[0] < upper[0] - 0.3 * gain,
            "gradient did not strengthen: {upper:?} -> {lower:?}"
        );
        if let Some(previous) = previous {
            close(
                lower,
                [
                    previous[0] * 2.0,
                    previous[1] * 2.0,
                    previous[2] * 2.0,
                    previous[3],
                ],
            );
        }
        previous = Some(lower);
        scene.begin(left, right);
        scene
            .renderer
            .with_layer(clip, |renderer| renderer.draw_backdrop(effect));
        for (bounds, color) in [
            (rect(60.0, 104.0, 8.0, 8.0), Color::from_rgb(0.0, 1.0, 0.0)),
            (rect(60.0, 88.0, 8.0, 8.0), Color::from_rgb(0.0, 0.0, 1.0)),
        ] {
            scene.renderer.fill_quad(
                Quad {
                    bounds,
                    ..Default::default()
                },
                color,
            );
        }
        let clipped = scene.read();
        close(pixel(&clipped, 64, 108), [0.0, 1.0, 0.0, 1.0]);
        close(pixel(&clipped, 64, 92), [0.0, 0.0, 1.0, 1.0]);
        for (x, y) in [(47, 112), (80, 112), (64, 79), (64, 120)] {
            assert_eq!(pixel(&clipped, x, y), pixel(&plain, x, y));
        }
        for x in [58, 63, 64, 69] {
            let value = pixel(&clipped, x, 116);
            close(value, pixel(&full, x, 116)); // Clipping must not restart the profile.
            assert!(
                value[3] > 0.28 && value[3] < 0.47,
                "unfiltered boundary: {value:?}"
            );
            let weight = (value[3] - right[3]) / (left[3] - right[3]);
            close(
                value,
                std::array::from_fn(|i| right[i] + (left[i] - right[i]) * weight),
            );
            assert!(value[0] > 1.0 && value[1] > 1.0, "HDR clipped: {value:?}");
        }
        eprintln!("live frame gain={gain}: upper={upper:?}, lower={lower:?}");
    }
}

// A normal widget submits a normal Primitive::draw, as the embedded player does.
struct VideoWidget(VideoFrame);

impl iced_core::Widget<(), iced::Theme, Renderer> for VideoWidget {
    fn size(&self) -> Size<iced::Length> {
        Size::new(iced::Fill, iced::Fill)
    }

    fn layout(
        &mut self,
        _: &mut iced_core::widget::Tree,
        _: &Renderer,
        limits: &iced_core::layout::Limits,
    ) -> iced_core::layout::Node {
        iced_core::layout::Node::new(limits.max())
    }

    fn draw(
        &self,
        _: &iced_core::widget::Tree,
        renderer: &mut Renderer,
        _: &iced::Theme,
        _: &iced_core::renderer::Style,
        layout: iced_core::Layout<'_>,
        _: iced_core::mouse::Cursor,
        _: &Rectangle,
    ) {
        renderer.draw_primitive(
            layout.bounds(),
            VideoFrame {
                left: self.0.left,
                right: self.0.right,
            },
        );
    }
}

fn draw_player(scene: &mut Scene, gain: f32, height: Option<f32>, enabled: bool) {
    use iced::widget::{Space, backdrop, container};
    use iced_core::{Element, mouse};
    let left = [4.0 * gain, 2.0 * gain, 0.5 * gain, 0.5];
    let right = [0.5 * gain, 1.5 * gain, 3.0 * gain, 0.25];
    let video = Element::new(VideoWidget(VideoFrame { left, right }));
    let controls = Some({
        container(Space::new().width(8).height(8))
            .style(|_| {
                iced::widget::container::Style::default().background(Color::from_rgb(0.0, 1.0, 0.0))
            })
            .into()
    });
    let popover = container(container(Space::new().width(8).height(8)).style(|_| {
        iced::widget::container::Style::default().background(Color::from_rgb(0.0, 0.0, 1.0))
    }))
    .center(iced::Fill)
    .into();
    let content: Element<'_, (), iced::Theme, Renderer> = if enabled {
        composition::player_layers(
            video,
            controls,
            Some(popover),
            height.is_some(),
            height.unwrap_or(64.0),
        )
    } else {
        // Disabling an effect is also supported without removing its child.
        iced::widget::stack![
            video,
            container(
                backdrop(
                    Blur::vertical_gradient(0.0, 16.0),
                    container(Space::new().width(128).height(64))
                )
                .tint(Color::BLACK)
                .enabled(false)
            )
            .align_bottom(iced::Fill),
            popover,
        ]
        .into()
    };
    let mut ui = iced_runtime::UserInterface::build(
        content,
        Size::new(128.0, 128.0),
        Default::default(),
        &mut scene.renderer,
    );
    let _ = ui.draw(
        &mut scene.renderer,
        &iced::Theme::Dark,
        &iced_core::renderer::Style {
            text_color: Color::WHITE,
        },
        mouse::Cursor::Unavailable,
    );
}

#[test]
fn player_composition_minimal_full_hidden_and_disabled_follow_live_frames() {
    let mut scene = Scene::new();
    draw_player(&mut scene, 1.0, None, true);
    let hidden = scene.read();
    let cold = scene.renderer.blur_statistics();
    assert_eq!(cold.scene_misses, 0);
    assert_eq!(cold.scene_hits, 0);
    assert_eq!(cold.retained_bytes, 0);
    close(pixel(&hidden, 4, 4), [0.0, 1.0, 0.0, 1.0]); // Minimal stays visible and sharp.
    for height in [32.0, 64.0] {
        draw_player(&mut scene, 1.0, Some(height), true);
        let shown = scene.read();
        assert_eq!(
            pixel(&shown, 63, (127.0 - height) as usize),
            pixel(&hidden, 63, (127.0 - height) as usize)
        );
        assert!(pixel(&shown, 63, 116)[3] < 0.45);
        close(pixel(&shown, 4, 124), [0.0, 1.0, 0.0, 1.0]);
        close(pixel(&shown, 64, 64), [0.0, 0.0, 1.0, 1.0]);
    }
    let active = scene.renderer.blur_statistics();
    assert!(active.scene_misses > 0);
    for enabled in [true, false] {
        draw_player(&mut scene, 2.0, None, enabled);
        let hidden = scene.read();
        let after = scene.renderer.blur_statistics();
        assert_eq!(
            after.scene_misses, active.scene_misses,
            "hidden controls recomputed blur"
        );
        assert_eq!(
            after.scene_hits, active.scene_hits,
            "hidden controls sampled a blur cache"
        );
        close(pixel(&hidden, 63, 116), [8.0, 4.0, 1.0, 0.5]);
        close(pixel(&hidden, 64, 116), [1.0, 3.0, 6.0, 0.25]);
        eprintln!(
            "hidden enabled={enabled}: scene_misses={}, scene_hits={}, retained_bytes={}",
            after.scene_misses, after.scene_hits, after.retained_bytes
        );
    }
}
