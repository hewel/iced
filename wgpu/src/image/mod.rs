pub(crate) mod cache;
pub(crate) use cache::Cache;

mod atlas;

#[cfg(feature = "image")]
mod raster;

#[cfg(feature = "image")]
mod progressive;

#[cfg(feature = "svg")]
mod vector;

use crate::Buffer;
use crate::core::border;
use crate::core::{Rectangle, Transformation};
use crate::graphics::{Shell, shape};

use bytemuck::{Pod, Zeroable};

use std::mem;
use std::sync::Arc;

pub use crate::graphics::Image;

pub type Batch = Vec<Image>;

pub fn needs_streaming(images: &[Image]) -> bool {
    images
        .iter()
        .any(|image| matches!(image, Image::Raster { image, .. } if image.blur.maximum() > 0.0))
}

#[derive(Debug, Clone)]
pub struct Pipeline {
    raw: wgpu::RenderPipeline,
    backend: wgpu::Backend,
    nearest_sampler: wgpu::Sampler,
    linear_sampler: wgpu::Sampler,
    texture_layout: wgpu::BindGroupLayout,
    constant_layout: wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
}

impl Pipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, backend: wgpu::Backend) -> Self {
        let nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            min_filter: wgpu::FilterMode::Nearest,
            mag_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            min_filter: wgpu::FilterMode::Linear,
            mag_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let constant_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("iced_wgpu::image constants layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(mem::size_of::<Uniforms>() as u64),
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

        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("iced_wgpu::image texture atlas layout"),
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

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("iced_wgpu::image pipeline layout"),
            bind_group_layouts: &[Some(&constant_layout), Some(&texture_layout)],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("iced_wgpu image shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(concat!(
                include_str!("../shader/vertex.wgsl"),
                "\n",
                include_str!("../shader/color.wgsl"),
                "\n",
                include_str!("../shader/shape.wgsl"),
                "\n",
                include_str!("../shader/image.wgsl"),
            ))),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("iced_wgpu::image pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &Instance::ATTRIBUTES,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Cw,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });

        Pipeline {
            raw: pipeline,
            backend,
            nearest_sampler,
            linear_sampler,
            texture_layout,
            constant_layout,
            format,
        }
    }

    pub fn create_cache(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shell: &Shell,
        queue_synchronization: Option<std::sync::Arc<dyn crate::QueueSynchronization>>,
    ) -> Cache {
        Cache::new(
            device,
            queue,
            self.backend,
            self.texture_layout.clone(),
            shell,
            queue_synchronization,
        )
    }
}

#[derive(Default)]
pub struct State {
    layers: Vec<Layer>,
    prepare_layer: usize,
    nearest_instances: Vec<Instance>,
    linear_instances: Vec<Instance>,
    blur: Option<crate::blur::Pipeline>,
    blurred: Vec<(u64, crate::blur::Target)>,
    scratch: Vec<crate::blur::Target>,
    retired: Vec<crate::blur::Target>,
    #[cfg(feature = "image")]
    progressive: progressive::State,
    pub image_hits: u64,
    pub image_misses: u64,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn prepare(
        &mut self,
        pipeline: &Pipeline,
        device: &wgpu::Device,
        belt: &mut wgpu::util::StagingBelt,
        encoder: &mut wgpu::CommandEncoder,
        cache: &mut Cache,
        images: &[Image],
        transformation: Transformation,
        scale: f32,
        _visible_bounds: Rectangle,
    ) {
        if self.layers.len() <= self.prepare_layer {
            self.layers.push(Layer::new(
                device,
                &pipeline.constant_layout,
                &pipeline.nearest_sampler,
                &pipeline.linear_sampler,
            ));
        }

        let layer = &mut self.layers[self.prepare_layer];

        if self.blur.is_none()
            && images.iter().any(
                |image| matches!(image, Image::Raster { image, .. } if image.blur.maximum() > 0.0),
            )
        {
            self.blur = Some(crate::blur::Pipeline::new(
                device,
                pipeline.format,
                pipeline.backend,
            ));
        }
        let blur = self.blur.as_ref();
        let mut atlas: Option<Arc<wgpu::BindGroup>> = None;

        for image in images {
            match &image {
                #[cfg(feature = "image")]
                Image::Raster {
                    image,
                    bounds,
                    clip_bounds,
                } => {
                    // Keep signed coordinates when snapping: scrolled content is
                    // clipped later and must not be saturated to the origin.
                    let bounds = shape::snap(*bounds * scale, image.snap);
                    let clip_bounds = shape::snap(*clip_bounds * scale, image.snap);

                    if !valid_bounds(bounds) || !valid_bounds(clip_bounds) {
                        continue;
                    }

                    if let Some((atlas_entry, bind_group, revision)) =
                        cache.upload_raster(device, encoder, belt, &image.handle)
                    {
                        match atlas.as_mut() {
                            None => {
                                atlas = Some(bind_group.clone());
                            }
                            Some(atlas) if atlas != bind_group => {
                                layer.push(atlas, &self.nearest_instances, &self.linear_instances);

                                *atlas = Arc::clone(bind_group);
                            }
                            _ => {}
                        }

                        let profile = image.blur;
                        let sigma = profile.maximum() * scale;
                        let rendition_size =
                            crate::blur::rendition_size([bounds.width, bounds.height], sigma);
                        // A smaller rendition must be filtered before either
                        // axis is reduced, otherwise fine source detail aliases
                        // into a constant that a later blur cannot remove.
                        if matches!(profile, crate::core::Blur::Linear(_))
                            || (sigma > 0.0
                                && ((rendition_size[0] as f32) < bounds.width
                                    || (rendition_size[1] as f32) < bounds.height))
                        {
                            layer.push(bind_group, &self.nearest_instances, &self.linear_instances);
                            let (hits, misses) = self.progressive.prepare(
                                pipeline,
                                blur.expect("image blur pipeline"),
                                device,
                                encoder,
                                belt,
                                image,
                                bounds,
                                clip_bounds,
                                _visible_bounds,
                                scale,
                                revision,
                                atlas_entry,
                                bind_group,
                                layer,
                                &self.nearest_instances,
                                &mut self.linear_instances,
                            );
                            self.image_hits += hits;
                            self.image_misses += misses;
                        } else if profile.maximum() > 0.0 {
                            let blur = blur.expect("image blur pipeline");
                            let size = rendition_size;
                            let key = crate::blur::fingerprint(&(
                                image.handle.id(),
                                revision,
                                image.crop,
                                size,
                                bounds.width.to_bits(),
                                bounds.height.to_bits(),
                                sigma.to_bits(),
                                image.filter_method,
                                atlas_entry,
                            ));
                            let index = self
                                .blurred
                                .iter()
                                .position(|(candidate, _)| *candidate == key);
                            let output = if let Some(index) = index {
                                self.image_hits += 1;
                                let entry = self.blurred.remove(index);
                                self.blurred.push(entry);
                                &self.blurred.last().unwrap().1
                            } else {
                                self.image_misses += 1;
                                let mut take = || {
                                    let mut target = self
                                        .scratch
                                        .pop()
                                        .unwrap_or_else(|| blur.pooled_target(device, size));
                                    target.size = size;
                                    target
                                };
                                let source = take();
                                let horizontal = take();
                                if self.blurred.len() >= 16 {
                                    retire_output(&mut self.retired, self.blurred.remove(0).1);
                                }
                                // A target can remain referenced by an earlier image in this
                                // prepared frame. Retire it, but never overwrite it until trim
                                // has released those bindings.
                                let mut output = self
                                    .retired
                                    .iter()
                                    .position(|target| Arc::strong_count(&target.binding) == 1)
                                    .map(|index| self.retired.swap_remove(index))
                                    .unwrap_or_else(|| blur.pooled_target(device, size));
                                output.size = size;
                                let mut rendition = Layer::new(
                                    device,
                                    &pipeline.constant_layout,
                                    &pipeline.nearest_sampler,
                                    &pipeline.linear_sampler,
                                );
                                let mut instances = Vec::new();
                                let frame = Rectangle::new(
                                    crate::core::Point::ORIGIN,
                                    crate::core::Size::new(size[0] as f32, size[1] as f32),
                                );
                                add_instances(
                                    frame,
                                    frame,
                                    border::radius(0),
                                    0.0,
                                    image.crop,
                                    0.0,
                                    1.0,
                                    atlas_entry,
                                    &mut instances,
                                );
                                // The isolated rendition has no edge coverage: its mask is applied only at final composition.
                                for instance in &mut instances {
                                    instance._edges |= 32;
                                    if image.filter_method
                                        == crate::core::image::FilterMethod::Nearest
                                    {
                                        instance._edges |= 64;
                                    }
                                }
                                let (nearest, linear) = match image.filter_method {
                                    crate::core::image::FilterMethod::Nearest => {
                                        (instances.as_slice(), &[][..])
                                    }
                                    crate::core::image::FilterMethod::Linear => {
                                        (&[][..], instances.as_slice())
                                    }
                                };
                                rendition.push(bind_group, nearest, linear);
                                rendition.prepare(
                                    device,
                                    encoder,
                                    belt,
                                    Transformation::orthographic(size[0], size[1]),
                                    nearest,
                                    linear,
                                );
                                {
                                    let mut pass =
                                        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                            label: Some("isolated image rendition"),
                                            color_attachments: &[Some(
                                                wgpu::RenderPassColorAttachment {
                                                    view: &source.view,
                                                    depth_slice: None,
                                                    resolve_target: None,
                                                    ops: wgpu::Operations {
                                                        load: wgpu::LoadOp::Clear(
                                                            wgpu::Color::TRANSPARENT,
                                                        ),
                                                        store: wgpu::StoreOp::Store,
                                                    },
                                                },
                                            )],
                                            depth_stencil_attachment: None,
                                            timestamp_writes: None,
                                            occlusion_query_set: None,
                                            multiview_mask: None,
                                        });
                                    pass.set_viewport(
                                        0.0,
                                        0.0,
                                        size[0] as f32,
                                        size[1] as f32,
                                        0.0,
                                        1.0,
                                    );
                                    pass.set_scissor_rect(0, 0, size[0], size[1]);
                                    pass.set_pipeline(&pipeline.raw);
                                    rendition.render(&mut pass);
                                }
                                blur.pass(
                                    device,
                                    encoder,
                                    &source,
                                    &horizontal.view,
                                    size,
                                    sigma * size[0] as f32 / bounds.width,
                                    true,
                                );
                                blur.pass(
                                    device,
                                    encoder,
                                    &horizontal,
                                    &output.view,
                                    size,
                                    sigma * size[1] as f32 / bounds.height,
                                    false,
                                );
                                self.scratch.extend([source, horizontal]);
                                while self.scratch.len() > 2 {
                                    drop(self.scratch.remove(0));
                                }
                                while self
                                    .blurred
                                    .iter()
                                    .map(|(_, target)| target.bytes())
                                    .sum::<usize>()
                                    + output.bytes()
                                    > 64 * 1024 * 1024
                                {
                                    retire_output(&mut self.retired, self.blurred.remove(0).1);
                                }
                                self.blurred.push((key, output));
                                &self.blurred.last().unwrap().1
                            };
                            layer.push(bind_group, &self.nearest_instances, &self.linear_instances);
                            let start = self.linear_instances.len();
                            add_instances(
                                bounds,
                                clip_bounds,
                                scaled_radius(image.border_radius, scale, clip_bounds),
                                image.border_smoothing,
                                image.crop,
                                f32::from(image.rotation),
                                image.opacity,
                                atlas_entry,
                                &mut self.linear_instances,
                            );
                            self.linear_instances
                                .truncate(start + usize::from(self.linear_instances.len() > start));
                            if let Some(instance) = self.linear_instances.get_mut(start) {
                                instance._tile = instance._bounds;
                                instance._atlas = [
                                    0.0,
                                    0.0,
                                    output.size[0] as f32 / output.capacity[0] as f32,
                                    output.size[1] as f32 / output.capacity[1] as f32,
                                ];
                                instance._layer = 0;
                                instance._edges = 15 | 16;
                            }
                            layer.push(
                                &output.binding,
                                &self.nearest_instances,
                                &self.linear_instances,
                            );
                        } else {
                            add_instances(
                                bounds,
                                clip_bounds,
                                scaled_radius(image.border_radius, scale, clip_bounds),
                                image.border_smoothing,
                                image.crop,
                                f32::from(image.rotation),
                                image.opacity,
                                atlas_entry,
                                match image.filter_method {
                                    crate::core::image::FilterMethod::Nearest => {
                                        &mut self.nearest_instances
                                    }
                                    crate::core::image::FilterMethod::Linear => {
                                        &mut self.linear_instances
                                    }
                                },
                            );
                            layer.push(bind_group, &self.nearest_instances, &self.linear_instances);
                        }
                    }
                }
                #[cfg(not(feature = "image"))]
                Image::Raster { .. } => continue,

                #[cfg(feature = "svg")]
                Image::Vector {
                    svg,
                    bounds,
                    clip_bounds,
                } => {
                    // Like raster images, vectors may extend above or left of
                    // the viewport; clipping happens after signed snapping.
                    let bounds = shape::snap(*bounds * scale, true);
                    let clip_bounds = shape::snap(*clip_bounds * scale, true);

                    if !valid_bounds(bounds) || !valid_bounds(clip_bounds) {
                        continue;
                    }

                    if let Some((atlas_entry, bind_group)) = cache.upload_vector(
                        device,
                        encoder,
                        belt,
                        &svg.handle,
                        svg.color,
                        crate::core::Size::new(bounds.width as u32, bounds.height as u32),
                    ) {
                        match atlas.as_mut() {
                            None => {
                                atlas = Some(bind_group.clone());
                            }
                            Some(atlas) if atlas != bind_group => {
                                layer.push(atlas, &self.nearest_instances, &self.linear_instances);

                                *atlas = bind_group.clone();
                            }
                            _ => {}
                        }

                        add_instances(
                            bounds,
                            clip_bounds,
                            border::radius(0),
                            0.0,
                            None,
                            f32::from(svg.rotation),
                            svg.opacity,
                            atlas_entry,
                            &mut self.nearest_instances,
                        );
                        layer.push(bind_group, &self.nearest_instances, &self.linear_instances);
                    }
                }
                #[cfg(not(feature = "svg"))]
                Image::Vector { .. } => continue,
            }
        }

        if let Some(atlas) = &atlas {
            layer.push(atlas, &self.nearest_instances, &self.linear_instances);
        }

        layer.prepare(
            device,
            encoder,
            belt,
            transformation,
            &self.nearest_instances,
            &self.linear_instances,
        );

        self.prepare_layer += 1;
        self.nearest_instances.clear();
        self.linear_instances.clear();
    }

    pub fn render<'a>(
        &'a self,
        pipeline: &'a Pipeline,
        layer: usize,
        bounds: Rectangle<u32>,
        render_pass: &mut wgpu::RenderPass<'a>,
    ) {
        if let Some(layer) = self.layers.get(layer) {
            render_pass.set_pipeline(&pipeline.raw);

            render_pass.set_scissor_rect(bounds.x, bounds.y, bounds.width, bounds.height);

            layer.render(render_pass);
        }
    }

    pub fn trim(&mut self) {
        // Drop bind groups before evicting textures from the rendition cache.
        for layer in &mut self.layers[..self.prepare_layer] {
            layer.clear();
        }

        self.prepare_layer = 0;
    }

    pub fn retained_bytes(&self) -> usize {
        let bytes = self
            .blurred
            .iter()
            .map(|(_, target)| target.bytes())
            .sum::<usize>()
            + self
                .scratch
                .iter()
                .map(crate::blur::Target::bytes)
                .sum::<usize>()
            + self
                .retired
                .iter()
                .map(crate::blur::Target::bytes)
                .sum::<usize>();
        #[cfg(feature = "image")]
        let bytes = bytes + self.progressive.bytes();
        bytes
    }
}

fn retire_output(pool: &mut Vec<crate::blur::Target>, target: crate::blur::Target) {
    while pool.len() >= 16
        || pool.iter().map(crate::blur::Target::bytes).sum::<usize>() + target.bytes()
            > 64 * 1024 * 1024
    {
        drop(pool.remove(0));
    }
    pool.push(target);
}

#[derive(Debug)]
struct Layer {
    uniforms: wgpu::Buffer,
    instances: Buffer<Instance>,
    nearest: Vec<Group>,
    nearest_layout: wgpu::BindGroup,
    nearest_total: usize,
    linear: Vec<Group>,
    linear_layout: wgpu::BindGroup,
    linear_total: usize,
    order: Vec<(bool, usize)>,
}

#[derive(Debug)]
struct Group {
    atlas: Arc<wgpu::BindGroup>,
    instance_count: usize,
    instance_start: usize,
}

impl Layer {
    fn new(
        device: &wgpu::Device,
        constant_layout: &wgpu::BindGroupLayout,
        nearest_sampler: &wgpu::Sampler,
        linear_sampler: &wgpu::Sampler,
    ) -> Self {
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("iced_wgpu::image uniforms buffer"),
            size: mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let instances = Buffer::new(
            device,
            "iced_wgpu::image instance buffer",
            Instance::INITIAL,
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        );

        let nearest_layout = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("iced_wgpu::image constants bind group"),
            layout: constant_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniforms,
                        offset: 0,
                        size: None,
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(nearest_sampler),
                },
            ],
        });

        let linear_layout = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("iced_wgpu::image constants bind group"),
            layout: constant_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniforms,
                        offset: 0,
                        size: None,
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(linear_sampler),
                },
            ],
        });

        Self {
            uniforms,
            instances,
            nearest: Vec::new(),
            nearest_layout,
            nearest_total: 0,
            linear: Vec::new(),
            linear_layout,
            linear_total: 0,
            order: Vec::new(),
        }
    }

    fn prepare(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        belt: &mut wgpu::util::StagingBelt,
        transformation: Transformation,
        nearest: &[Instance],
        linear: &[Instance],
    ) {
        let uniforms = Uniforms {
            transform: transformation.into(),
        };

        let bytes = bytemuck::bytes_of(&uniforms);

        belt.write_buffer(
            encoder,
            &self.uniforms,
            0,
            (bytes.len() as u64).try_into().expect("Sized uniforms"),
        )
        .copy_from_slice(bytes);

        let _ = self
            .instances
            .resize(device, self.nearest_total + self.linear_total);

        let mut offset = 0;

        if !nearest.is_empty() {
            offset += self.instances.write(encoder, belt, 0, nearest);
        }

        if !linear.is_empty() {
            let _ = self.instances.write(encoder, belt, offset, linear);
        }
    }

    fn push(&mut self, atlas: &Arc<wgpu::BindGroup>, nearest: &[Instance], linear: &[Instance]) {
        let new_nearest = nearest.len() - self.nearest_total;

        if new_nearest > 0 {
            if let Some(&(false, index)) = self.order.last()
                && self.nearest[index].atlas == *atlas
            {
                self.nearest[index].instance_count += new_nearest;
            } else {
                self.order.push((false, self.nearest.len()));
                self.nearest.push(Group {
                    atlas: atlas.clone(),
                    instance_count: new_nearest,
                    instance_start: self.nearest_total,
                });
            }

            self.nearest_total = nearest.len();
        }

        let new_linear = linear.len() - self.linear_total;

        if new_linear > 0 {
            if let Some(&(true, index)) = self.order.last()
                && self.linear[index].atlas == *atlas
            {
                self.linear[index].instance_count += new_linear;
            } else {
                self.order.push((true, self.linear.len()));
                self.linear.push(Group {
                    atlas: atlas.clone(),
                    instance_count: new_linear,
                    instance_start: self.linear_total,
                });
            }

            self.linear_total = linear.len();
        }
    }

    fn render<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        render_pass.set_vertex_buffer(0, self.instances.slice(..));

        for &(linear, index) in &self.order {
            let (groups, layout, base) = if linear {
                (&self.linear, &self.linear_layout, self.nearest_total)
            } else {
                (&self.nearest, &self.nearest_layout, 0)
            };
            let offset = (base + groups[index].instance_start) as u32;
            let group = &groups[index];
            render_pass.set_bind_group(0, layout, &[]);
            render_pass.set_bind_group(1, group.atlas.as_ref(), &[]);
            render_pass.draw(0..6, offset..offset + group.instance_count as u32);
        }
    }

    fn clear(&mut self) {
        self.order.clear();
        self.nearest.clear();
        self.nearest_total = 0;

        self.linear.clear();
        self.linear_total = 0;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
struct Instance {
    _bounds: [f32; 4],
    _clip_bounds: [f32; 4],
    _border_radius: [f32; 4],
    _tile: [f32; 4],
    _atlas: [f32; 4],
    _rotation: f32,
    _opacity: f32,
    _border_smoothing: f32,
    _layer: u32,
    _edges: u32,
}

impl Instance {
    pub const INITIAL: usize = 20;

    pub const ATTRIBUTES: [wgpu::VertexAttribute; 10] = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset: mem::offset_of!(Self, _bounds) as u64,
            shader_location: 0,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset: mem::offset_of!(Self, _clip_bounds) as u64,
            shader_location: 1,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset: mem::offset_of!(Self, _border_radius) as u64,
            shader_location: 2,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset: mem::offset_of!(Self, _tile) as u64,
            shader_location: 3,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset: mem::offset_of!(Self, _atlas) as u64,
            shader_location: 4,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32,
            offset: mem::offset_of!(Self, _rotation) as u64,
            shader_location: 5,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32,
            offset: mem::offset_of!(Self, _opacity) as u64,
            shader_location: 6,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32,
            offset: mem::offset_of!(Self, _border_smoothing) as u64,
            shader_location: 7,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint32,
            offset: mem::offset_of!(Self, _layer) as u64,
            shader_location: 8,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint32,
            offset: mem::offset_of!(Self, _edges) as u64,
            shader_location: 9,
        },
    ];
}

const _: () = {
    assert!(mem::size_of::<Instance>() == 100);
    assert!(mem::align_of::<Instance>() == 4);
    assert!(mem::offset_of!(Instance, _bounds) == 0);
    assert!(mem::offset_of!(Instance, _clip_bounds) == 16);
    assert!(mem::offset_of!(Instance, _border_radius) == 32);
    assert!(mem::offset_of!(Instance, _tile) == 48);
    assert!(mem::offset_of!(Instance, _atlas) == 64);
    assert!(mem::offset_of!(Instance, _rotation) == 80);
    assert!(mem::offset_of!(Instance, _opacity) == 84);
    assert!(mem::offset_of!(Instance, _border_smoothing) == 88);
    assert!(mem::offset_of!(Instance, _layer) == 92);
    assert!(mem::offset_of!(Instance, _edges) == 96);
};

#[repr(C)]
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
struct Uniforms {
    transform: [f32; 16],
}

fn valid_bounds(bounds: Rectangle) -> bool {
    bounds.x.is_finite()
        && bounds.y.is_finite()
        && bounds.width.is_finite()
        && bounds.height.is_finite()
        && bounds.width > 0.0
        && bounds.height > 0.0
        && (bounds.x + bounds.width).is_finite()
        && (bounds.y + bounds.height).is_finite()
}

fn scaled_radius(radius: border::Radius, scale: f32, frame: Rectangle) -> border::Radius {
    let cap = frame.width.min(frame.height) / 2.0;
    let scale_length = |value| shape::normalize_length(value).min(cap / scale) * scale;
    border::Radius {
        top_left: scale_length(radius.top_left),
        top_right: scale_length(radius.top_right),
        bottom_right: scale_length(radius.bottom_right),
        bottom_left: scale_length(radius.bottom_left),
    }
}

fn add_instances(
    bounds: Rectangle,
    clip_bounds: Rectangle,
    border_radius: border::Radius,
    border_smoothing: f32,
    crop: Option<Rectangle<u32>>,
    rotation: f32,
    opacity: f32,
    entry: &atlas::Entry,
    instances: &mut Vec<Instance>,
) {
    let Some(crop) = crate::core::image::crop_bounds(entry.size(), crop) else {
        return;
    };
    if !rotation.is_finite() {
        return;
    }
    let opacity = if opacity.is_nan() {
        0.0
    } else {
        opacity.clamp(0.0, 1.0)
    };
    let template = Instance {
        _bounds: [bounds.x, bounds.y, bounds.width, bounds.height],
        _clip_bounds: [
            clip_bounds.x,
            clip_bounds.y,
            clip_bounds.width,
            clip_bounds.height,
        ],
        _border_radius: border_radius.into(),
        _tile: [0.0; 4],
        _atlas: [0.0; 4],
        _rotation: rotation,
        _opacity: opacity,
        _border_smoothing: shape::normalize_smoothing(border_smoothing),
        _layer: 0,
        _edges: 0,
    };
    match entry {
        atlas::Entry::Contiguous(allocation) => {
            add_instance(template, crop, (0, 0), allocation, instances);
        }
        atlas::Entry::Fragmented { fragments, .. } => {
            for fragment in fragments {
                add_instance(
                    template,
                    crop,
                    fragment.position,
                    &fragment.allocation,
                    instances,
                );
            }
        }
    }
}

fn add_instance(
    mut instance: Instance,
    crop: Rectangle<u32>,
    origin: (u32, u32),
    allocation: &atlas::Allocation,
    instances: &mut Vec<Instance>,
) {
    let size = allocation.size();
    let left = crop.x.max(origin.0);
    let top = crop.y.max(origin.1);
    let right = (crop.x + crop.width).min(origin.0 + size.width);
    let bottom = (crop.y + crop.height).min(origin.1 + size.height);
    if left >= right || top >= bottom {
        return;
    }
    // Compute shared endpoints identically on both sides of a fragment seam.
    let bounds = instance._bounds;
    let x = |source: u32| bounds[0] + (source - crop.x) as f32 / crop.width as f32 * bounds[2];
    let y = |source: u32| bounds[1] + (source - crop.y) as f32 / crop.height as f32 * bounds[3];
    let tile_left = x(left);
    let tile_top = y(top);
    instance._tile = [
        tile_left,
        tile_top,
        x(right) - tile_left,
        y(bottom) - tile_top,
    ];
    let (atlas_x, atlas_y) = allocation.position();
    let atlas_size = allocation.atlas_size() as f32;
    instance._atlas = [
        (atlas_x + left - origin.0) as f32 / atlas_size,
        (atlas_y + top - origin.1) as f32 / atlas_size,
        (right - left) as f32 / atlas_size,
        (bottom - top) as f32 / atlas_size,
    ];
    instance._layer = allocation.layer() as u32;
    instance._edges = u32::from(left == crop.x)
        | (u32::from(top == crop.y) << 1)
        | (u32::from(right == crop.x + crop.width) << 2)
        | (u32::from(bottom == crop.y + crop.height) << 3);
    if instance._tile[2] > 0.0 && instance._tile[3] > 0.0 {
        instances.push(instance);
    }
}
