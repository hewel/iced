use iced_wgpu::wgpu;

pub struct Video {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    current: Option<wgpu::BindGroup>,
}

impl Video {
    pub fn new(device: &wgpu::Device) -> Self {
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
                    format: wgpu::TextureFormat::Rgb10a2Unorm,
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
            current: None,
        }
    }

    /// Binds the application's private GPU copy, never the borrowed mpv image.
    /// The caller records the copy and its synchronization before `draw` and
    /// retains the texture until all commands using it have completed.
    pub fn set_texture(&mut self, device: &wgpu::Device, texture: &wgpu::Texture) {
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
        self.current = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
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
        }));
    }

    /// Clears to opaque black, then draws the current frame if one is bound.
    /// The caller renders iced afterward with a load operation on this target.
    /// This method only records commands; queue submission belongs to main.
    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        assert_eq!(target.texture().format(), wgpu::TextureFormat::Rgb10a2Unorm);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("mpv video pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        if let Some(current) = &self.current {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, current, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
