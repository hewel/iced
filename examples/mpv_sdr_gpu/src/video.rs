use iced_wgpu::wgpu;

pub struct Video {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl Video {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mpv video texture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mpv code-preserving nearest sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("video.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mpv video pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mpv 10-bit SDR video"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // RGB is already premultiplied. Over opaque black this
                    // leaves its gamma-2.2 code values intact and outputs A=1.
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            layout,
            sampler,
        }
    }

    /// Binds the application's private GPU copy, never the borrowed mpv image.
    /// The caller records the copy and its synchronization before `draw` and
    /// retains the texture until all commands using it have completed.
    fn binding(&self, device: &wgpu::Device, texture: &wgpu::Texture) -> wgpu::BindGroup {
        assert_eq!(texture.format(), wgpu::TextureFormat::Rgb10a2Unorm);
        assert_eq!(texture.dimension(), wgpu::TextureDimension::D2);
        assert_eq!(texture.depth_or_array_layers(), 1);
        assert_eq!(texture.sample_count(), 1);
        assert!(
            texture
                .usage()
                .contains(wgpu::TextureUsages::TEXTURE_BINDING)
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("mpv private 10-bit video view"),
            format: Some(wgpu::TextureFormat::Rgb10a2Unorm),
            dimension: Some(wgpu::TextureViewDimension::D2),
            mip_level_count: Some(1),
            ..Default::default()
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mpv current private video"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }
}

use iced_wgpu::{Primitive, graphics::Viewport, primitive::Pipeline};
use iced_winit::core::{Rectangle, mouse};
use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone)]
pub(crate) struct Frame {
    texture: Option<wgpu::Texture>,
    binding: Arc<OnceLock<wgpu::BindGroup>>,
}
impl Frame {
    pub fn new(texture: Option<wgpu::Texture>) -> Self {
        Self {
            texture,
            binding: Arc::new(OnceLock::new()),
        }
    }
}
impl Pipeline for Video {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self::new(device, format)
    }
}
impl Primitive for Frame {
    type Pipeline = Video;
    fn prepare(
        &self,
        pipeline: &mut Video,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _bounds: &Rectangle,
        _viewport: &Viewport,
    ) {
        if let Some(texture) = &self.texture {
            self.binding
                .get_or_init(|| pipeline.binding(device, texture));
        }
    }
    fn draw(&self, pipeline: &Video, pass: &mut wgpu::RenderPass<'_>) -> bool {
        if let Some(binding) = self.binding.get() {
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, binding, &[]);
            pass.draw(0..3, 0..1);
        }
        true
    }
}
impl<Message> iced_widget::shader::Program<Message> for Frame {
    type State = ();
    type Primitive = Self;
    fn draw(&self, _state: &(), _cursor: mouse::Cursor, _bounds: Rectangle) -> Self {
        self.clone()
    }
}
