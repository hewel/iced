//! Bounded, render-pass-only premultiplied blur targets.
use std::sync::Arc;
use wgpu::util::DeviceExt;

pub(crate) struct Target {
    _texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub binding: Arc<wgpu::BindGroup>,
    pub size: [u32; 2],
    pub capacity: [u32; 2],
}
impl Target {
    pub fn bytes(&self) -> usize {
        self.capacity[0] as usize * self.capacity[1] as usize * 4
    }
}

pub(crate) struct Pipeline {
    layout: wgpu::BindGroupLayout,
    constants: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
}
impl Pipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blur texture"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let constants = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blur constants"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(32),
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blur"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader/blur.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blur"),
            bind_group_layouts: &[Some(&layout), Some(&constants)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blur"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            min_filter: wgpu::FilterMode::Linear,
            mag_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            layout,
            constants,
            pipeline,
            sampler,
            format,
        }
    }
    pub fn target(&self, device: &wgpu::Device, size: [u32; 2]) -> Target {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blur target"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let array = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let binding = Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blur texture"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&array),
            }],
        }));
        Target {
            _texture: texture,
            view,
            binding,
            size,
            capacity: size,
        }
    }
    pub fn pooled_target(&self, device: &wgpu::Device, size: [u32; 2]) -> Target {
        let mut target = self.target(device, [1024, 1024]);
        target.size = size;
        target
    }
    pub fn pass(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &Target,
        destination: &wgpu::TextureView,
        size: [u32; 2],
        sigma: f32,
        horizontal: bool,
    ) {
        let params = [
            size[0] as f32,
            size[1] as f32,
            sigma,
            if horizontal { 1.0 } else { 0.0 },
            source.size[0] as f32,
            source.size[1] as f32,
            source.capacity[0] as f32,
            source.capacity[1] as f32,
        ];
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("blur parameters"),
            contents: bytemuck::cast_slice(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let constants = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blur parameters"),
            layout: &self.constants,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("blur"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(0.0, 0.0, size[0] as f32, size[1] as f32, 0.0, 1.0);
        pass.set_scissor_rect(0, 0, size[0], size[1]);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, source.binding.as_ref(), &[]);
        pass.set_bind_group(1, &constants, &[]);
        pass.draw(0..3, 0..1);
    }
}

pub(crate) fn rendition_size(size: [f32; 2], sigma: f32) -> [u32; 2] {
    let reduction = (sigma / 8.0)
        .max(size[0] / 1024.0)
        .max(size[1] / 1024.0)
        .max(1.0);
    [
        (size[0] / reduction).ceil().clamp(1.0, 1024.0) as u32,
        (size[1] / reduction).ceil().clamp(1.0, 1024.0) as u32,
    ]
}

pub(crate) fn fingerprint(value: &impl std::fmt::Debug) -> u64 {
    use std::fmt::Write;
    use std::hash::Hasher;
    struct Writer(rustc_hash::FxHasher);
    impl std::fmt::Write for Writer {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            self.0.write(text.as_bytes());
            Ok(())
        }
    }
    let mut writer = Writer(rustc_hash::FxHasher::default());
    write!(&mut writer, "{value:?}").expect("hash writer");
    writer.0.finish()
}

pub(crate) struct Scene {
    pub pipeline: Pipeline,
    pub source: Option<Target>,
    scratch: Vec<Target>,
    cache: Vec<(u64, Target)>,
    pub hits: u64,
    pub misses: u64,
}
impl Scene {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self {
            pipeline: Pipeline::new(device, format),
            source: None,
            scratch: Vec::new(),
            cache: Vec::new(),
            hits: 0,
            misses: 0,
        }
    }
    pub fn resize(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self
            .source
            .as_ref()
            .is_none_or(|source| source.size != size)
        {
            self.source = Some(self.pipeline.target(device, size));
            self.cache.clear();
            self.scratch.clear();
        }
    }
    pub fn apply(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        key: Option<u64>,
        sigma: f32,
    ) {
        let source = self.source.as_ref().expect("scene target");
        let size = rendition_size([source.size[0] as f32, source.size[1] as f32], sigma);
        let cached = key.and_then(|key| {
            self.cache
                .iter()
                .position(|(candidate, _)| *candidate == key)
        });
        if let Some(index) = cached {
            self.hits += 1;
            let entry = self.cache.remove(index);
            self.pipeline.pass(
                device,
                encoder,
                &entry.1,
                &source.view,
                source.size,
                0.0,
                true,
            );
            self.cache.push(entry);
            return;
        }
        self.misses += 1;
        let mut take = || {
            let mut target = self
                .scratch
                .pop()
                .unwrap_or_else(|| self.pipeline.pooled_target(device, size));
            target.size = size;
            target
        };
        let downsample = take();
        let horizontal = take();
        let recycled = if self.cache.len() >= 16 {
            Some(self.cache.remove(0).1)
        } else {
            None
        };
        let mut output = recycled.unwrap_or_else(&mut take);
        output.size = size;
        self.pipeline
            .pass(device, encoder, source, &downsample.view, size, 0.0, true);
        self.pipeline.pass(
            device,
            encoder,
            &downsample,
            &horizontal.view,
            size,
            sigma * size[0] as f32 / source.size[0] as f32,
            true,
        );
        self.pipeline.pass(
            device,
            encoder,
            &horizontal,
            &output.view,
            size,
            sigma * size[1] as f32 / source.size[1] as f32,
            false,
        );
        self.pipeline.pass(
            device,
            encoder,
            &output,
            &source.view,
            source.size,
            0.0,
            true,
        );
        self.scratch.extend([downsample, horizontal]);
        while self
            .cache
            .iter()
            .map(|(_, target)| target.bytes())
            .sum::<usize>()
            + output.bytes()
            > 64 * 1024 * 1024
        {
            drop(self.cache.remove(0));
        }
        if let Some(key) = key {
            self.cache.push((key, output));
        } else {
            self.scratch.push(output);
        }
        while self.scratch.len() > 3 {
            drop(self.scratch.remove(0));
        }
    }
    pub fn bytes(&self) -> usize {
        self.source.as_ref().map_or(0, Target::bytes)
            + self.scratch.iter().map(Target::bytes).sum::<usize>()
            + self
                .cache
                .iter()
                .map(|(_, target)| target.bytes())
                .sum::<usize>()
    }
}
