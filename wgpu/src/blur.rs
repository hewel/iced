//! Bounded, render-pass-only premultiplied blur targets.
use crate::core::{Blur, Point, Rectangle, Size, renderer};
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
        self.capacity[0] as usize
            * self.capacity[1] as usize
            * self._texture.depth_or_array_layers() as usize
            * self._texture.format().block_copy_size(None).unwrap_or(4) as usize
    }
}

pub(crate) struct Pipeline {
    layout: wgpu::BindGroupLayout,
    constants: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    profile_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    composite_textures: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
    layers: u32,
}
impl Pipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, backend: wgpu::Backend) -> Self {
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
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blur"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(
                    include_str!("shader/gaussian.wgsl"),
                    "\n",
                    include_str!("shader/blur.wgsl")
                )
                .into(),
            ),
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
        let profile_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("progressive blur"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(
                    include_str!("shader/gaussian.wgsl"),
                    "\n",
                    include_str!("shader/progressive_blur.wgsl")
                )
                .into(),
            ),
        });
        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("backdrop replacement"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(
                    include_str!("shader/shape.wgsl"),
                    "\n",
                    include_str!("shader/backdrop.wgsl")
                )
                .into(),
            ),
        });
        let composite_textures =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("backdrop inputs"),
                entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                }),
            });
        let composite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("backdrop replacement"),
            bind_group_layouts: &[Some(&composite_textures), Some(&constants)],
            immediate_size: 0,
        });
        let create = |label, shader: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
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
            })
        };
        let profile_pipeline = create("progressive blur", &profile_shader, &pipeline_layout);
        let composite_pipeline =
            create("backdrop replacement", &composite_shader, &composite_layout);
        Self {
            layout,
            constants,
            pipeline,
            profile_pipeline,
            composite_pipeline,
            composite_textures,
            sampler,
            format,
            // GLES needs a real array allocation; a one-layer texture is typed
            // GL_TEXTURE_2D even when its sampled view requests D2Array.
            layers: if backend == wgpu::Backend::Gl { 2 } else { 1 },
        }
    }
    pub fn target(&self, device: &wgpu::Device, size: [u32; 2]) -> Target {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blur target"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: self.layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2),
            array_layer_count: Some(1),
            ..Default::default()
        });
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
    #[cfg(feature = "image")]
    pub fn pooled_target(&self, device: &wgpu::Device, size: [u32; 2]) -> Target {
        let mut target = self.target(device, [1024, 1024]);
        target.size = size;
        target
    }

    fn parameters(&self, device: &wgpu::Device, params: &[f32]) -> wgpu::BindGroup {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("blur parameters"),
            contents: bytemuck::cast_slice(params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
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
        })
    }

    /// Samples a full-resolution source region with a spatial Gaussian sigma.
    /// All profile coordinates and radii are in source pixels, including when
    /// the destination is a tile. Blur the gradient axis first so samples in
    /// the perpendicular second pass share one radius.
    pub fn profile_pass(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &Target,
        destination: &wgpu::TextureView,
        destination_region: Rectangle<u32>,
        source_region: Rectangle,
        profile: Blur,
        reference_bounds: Rectangle,
        horizontal: bool,
        load: wgpu::LoadOp<wgpu::Color>,
    ) {
        use crate::core::blur::Direction;
        let (radii, range, vertical) = match profile {
            Blur::Uniform(radius) => ([radius, radius], [0.0, 1.0], false),
            Blur::Linear(linear) => (
                [linear.start, linear.end],
                linear.range,
                linear.direction == Direction::Vertical,
            ),
        };
        let (origin, extent) = if vertical {
            (reference_bounds.y, reference_bounds.height)
        } else {
            (reference_bounds.x, reference_bounds.width)
        };
        let constants = self.parameters(
            device,
            &[
                destination_region.x as f32,
                destination_region.y as f32,
                destination_region.width as f32,
                destination_region.height as f32,
                source_region.x,
                source_region.y,
                source_region.width,
                source_region.height,
                source.size[0] as f32,
                source.size[1] as f32,
                source.capacity[0] as f32,
                source.capacity[1] as f32,
                radii[0],
                radii[1],
                range[0],
                range[1],
                u32::from(horizontal) as f32,
                u32::from(vertical) as f32,
                origin,
                extent.max(f32::MIN_POSITIVE),
            ],
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("progressive blur"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(
            destination_region.x as f32,
            destination_region.y as f32,
            destination_region.width as f32,
            destination_region.height as f32,
            0.0,
            1.0,
        );
        pass.set_scissor_rect(
            destination_region.x,
            destination_region.y,
            destination_region.width,
            destination_region.height,
        );
        pass.set_pipeline(&self.profile_pipeline);
        pass.set_bind_group(0, source.binding.as_ref(), &[]);
        pass.set_bind_group(1, &constants, &[]);
        pass.draw(0..3, 0..1);
    }

    fn composite(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        original: &Target,
        blurred: &Target,
        destination: &wgpu::TextureView,
        tile: Rectangle<u32>,
        backdrop: renderer::Backdrop,
        clip: Rectangle,
    ) {
        let original_view = original._texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let blurred_view = blurred._texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let textures = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("backdrop inputs"),
            layout: &self.composite_textures,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&original_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&blurred_view),
                },
            ],
        });
        let radii: [f32; 4] = backdrop.border_radius.into();
        let bounds = backdrop.bounds;
        let constants = self.parameters(
            device,
            &[
                original.size[0] as f32,
                original.size[1] as f32,
                original.capacity[0] as f32,
                original.capacity[1] as f32,
                tile.x as f32,
                tile.y as f32,
                tile.width as f32,
                tile.height as f32,
                blurred.size[0] as f32,
                blurred.size[1] as f32,
                blurred.capacity[0] as f32,
                blurred.capacity[1] as f32,
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                radii[0],
                radii[1],
                radii[2],
                radii[3],
                backdrop.border_smoothing,
                0.0,
                0.0,
                0.0,
                clip.x,
                clip.y,
                clip.width,
                clip.height,
            ],
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("backdrop replacement"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(
            tile.x as f32,
            tile.y as f32,
            tile.width as f32,
            tile.height as f32,
            0.0,
            1.0,
        );
        pass.set_scissor_rect(tile.x, tile.y, tile.width, tile.height);
        pass.set_pipeline(&self.composite_pipeline);
        pass.set_bind_group(0, &textures, &[]);
        pass.set_bind_group(1, &constants, &[]);
        pass.draw(0..3, 0..1);
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
    snapshot: Option<Target>,
    scratch: Vec<Target>,
    cache: Vec<(u64, Target)>,
    pub hits: u64,
    pub misses: u64,
}
impl Scene {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, backend: wgpu::Backend) -> Self {
        Self {
            pipeline: Pipeline::new(device, format, backend),
            source: None,
            snapshot: None,
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
            self.snapshot = None;
            self.cache.clear();
            self.scratch.clear();
        }
    }
    fn apply_uniform(
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
        // Filter an axis before reducing it. Reducing the untouched axis
        // first would fold fine detail into false low frequencies or DC.
        let horizontal_size = [size[0], source.size[1]];
        let horizontal = take_target(&self.pipeline, device, &mut self.scratch, horizontal_size);
        let output = take_target(&self.pipeline, device, &mut self.scratch, size);
        self.pipeline.pass(
            device,
            encoder,
            source,
            &horizontal.view,
            horizontal_size,
            sigma,
            true,
        );
        self.pipeline.pass(
            device,
            encoder,
            &horizontal,
            &output.view,
            size,
            sigma,
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
        recycle_target(&mut self.scratch, horizontal);
        while !self.cache.is_empty()
            && (self.cache.len() >= 16
                || self
                    .cache
                    .iter()
                    .map(|(_, target)| target.bytes())
                    .sum::<usize>()
                    + output.bytes()
                    > 64 * 1024 * 1024)
        {
            drop(self.cache.remove(0));
        }
        if let Some(key) = key {
            self.cache.push((key, output));
        } else {
            recycle_target(&mut self.scratch, output);
        }
        while self.scratch.len() > 3 {
            drop(self.scratch.remove(0));
        }
    }

    /// Filters a local region while keeping its entire lower scene immutable.
    /// Only the bounded cache and one axis tile are retained in addition to two
    /// viewport targets. No CPU readback or progressively modified neighbors.
    pub fn apply(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        key: Option<u64>,
        backdrop: renderer::Backdrop,
        clip: Rectangle,
    ) {
        use crate::core::blur::Direction;
        let source = self.source.as_ref().expect("scene target");
        let viewport =
            Rectangle::with_size(Size::new(source.size[0] as f32, source.size[1] as f32));
        let Some(clip) = clip.intersection(&viewport) else {
            return;
        };
        let Some(visible) = backdrop
            .bounds
            .expand(0.5)
            .intersection(&clip)
            .and_then(|bounds| bounds.intersection(&viewport))
        else {
            return;
        };
        if backdrop.blur.maximum() <= 0.0 {
            return;
        }

        // Preserve the established reduction path for an unmasked full scene.
        if backdrop.bounds == viewport
            && clip.intersection(&viewport) == Some(viewport)
            && <[f32; 4]>::from(backdrop.border_radius) == [0.0; 4]
            && let Blur::Uniform(radius) = backdrop.blur
        {
            self.apply_uniform(device, encoder, key, radius);
            return;
        }
        let left = visible.x.floor().max(0.0) as u32;
        let top = visible.y.floor().max(0.0) as u32;
        let right = (visible.x + visible.width).ceil().min(viewport.width) as u32;
        let bottom = (visible.y + visible.height).ceil().min(viewport.height) as u32;
        if left >= right || top >= bottom {
            return;
        }

        let snapshot = self
            .snapshot
            .get_or_insert_with(|| self.pipeline.target(device, source.size));
        self.pipeline.pass(
            device,
            encoder,
            source,
            &snapshot.view,
            source.size,
            0.0,
            true,
        );
        let horizontal = !matches!(backdrop.blur,
            Blur::Linear(linear) if linear.direction == Direction::Vertical);
        // Include a bilinear texel beyond the dense kernel support so tile
        // edges never become filter edges.
        let halo = (backdrop.blur.maximum() * 3.0).ceil() as u32;
        let halo = halo.saturating_add(1);
        const TILE: u32 = 512;
        for y in (top..bottom).step_by(TILE as usize) {
            for x in (left..right).step_by(TILE as usize) {
                let tile = Rectangle {
                    x,
                    y,
                    width: (right - x).min(TILE),
                    height: (bottom - y).min(TILE),
                };
                let tile_key = key.map(|key| fingerprint(&(key, backdrop, clip, tile)));
                let found = tile_key.and_then(|key| {
                    self.cache
                        .iter()
                        .position(|(candidate, _)| *candidate == key)
                });
                let output = if let Some(index) = found {
                    self.hits += 1;
                    self.cache.remove(index).1
                } else {
                    self.misses += 1;
                    let (a, b) = if horizontal {
                        (
                            Point::new(x as f32, y.saturating_sub(halo) as f32),
                            Point::new(
                                (x + tile.width) as f32,
                                (y + tile.height).saturating_add(halo).min(source.size[1]) as f32,
                            ),
                        )
                    } else {
                        (
                            Point::new(x.saturating_sub(halo) as f32, y as f32),
                            Point::new(
                                (x + tile.width).saturating_add(halo).min(source.size[0]) as f32,
                                (y + tile.height) as f32,
                            ),
                        )
                    };
                    let region = Rectangle::new(a, Size::new(b.x - a.x, b.y - a.y));
                    let size = [region.width as u32, region.height as u32];
                    let intermediate = take_target(&self.pipeline, device, &mut self.scratch, size);
                    self.pipeline.profile_pass(
                        device,
                        encoder,
                        snapshot,
                        &intermediate.view,
                        Rectangle::with_size(Size::new(size[0], size[1])),
                        region,
                        backdrop.blur,
                        backdrop.bounds,
                        horizontal,
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    );
                    let output = take_target(
                        &self.pipeline,
                        device,
                        &mut self.scratch,
                        [tile.width, tile.height],
                    );
                    self.pipeline.profile_pass(
                        device,
                        encoder,
                        &intermediate,
                        &output.view,
                        Rectangle::with_size(Size::new(tile.width, tile.height)),
                        Rectangle::new(
                            Point::new(x as f32 - a.x, y as f32 - a.y),
                            Size::new(tile.width as f32, tile.height as f32),
                        ),
                        backdrop.blur,
                        Rectangle {
                            x: backdrop.bounds.x - a.x,
                            y: backdrop.bounds.y - a.y,
                            ..backdrop.bounds
                        },
                        !horizontal,
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    );
                    recycle_target(&mut self.scratch, intermediate);
                    output
                };
                self.pipeline.composite(
                    device,
                    encoder,
                    snapshot,
                    &output,
                    &source.view,
                    tile,
                    backdrop,
                    clip,
                );
                if let Some(key) = tile_key {
                    while !self.cache.is_empty()
                        && (self.cache.len() >= 16
                            || self
                                .cache
                                .iter()
                                .map(|(_, target)| target.bytes())
                                .sum::<usize>()
                                + output.bytes()
                                > 64 * 1024 * 1024)
                    {
                        drop(self.cache.remove(0));
                    }
                    self.cache.push((key, output));
                } else {
                    recycle_target(&mut self.scratch, output);
                }
            }
        }
    }
    pub fn bytes(&self) -> usize {
        self.source.as_ref().map_or(0, Target::bytes)
            + self.snapshot.as_ref().map_or(0, Target::bytes)
            + self.scratch.iter().map(Target::bytes).sum::<usize>()
            + self
                .cache
                .iter()
                .map(|(_, target)| target.bytes())
                .sum::<usize>()
    }
}

fn take_target(
    pipeline: &Pipeline,
    device: &wgpu::Device,
    pool: &mut Vec<Target>,
    size: [u32; 2],
) -> Target {
    let mut target = pool
        .iter()
        .position(|target| target.capacity == size)
        .map(|index| pool.swap_remove(index))
        .unwrap_or_else(|| pipeline.target(device, size));
    target.size = size;
    target
}

fn recycle_target(pool: &mut Vec<Target>, target: Target) {
    const BUDGET: usize = 64 * 1024 * 1024;
    if target.bytes() > BUDGET {
        return;
    }
    while !pool.is_empty()
        && (pool.len() >= 3
            || pool.iter().map(Target::bytes).sum::<usize>() + target.bytes() > BUDGET)
    {
        drop(pool.remove(0));
    }
    pool.push(target);
}
