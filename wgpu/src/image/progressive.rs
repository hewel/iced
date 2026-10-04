//! Full-resolution image convolution. Tiles include convolution halos and a
//! filtered output gutter; neither the crop nor the profile depends on clipping.

use super::{Instance, Layer, Pipeline, add_instances, atlas, scaled_radius};
use crate::blur::{self, Target};
use crate::core::blur::Direction;
use crate::core::image::{FilterMethod, Image};
use crate::core::{Blur, Point, Rectangle, Size, Transformation, border};
use std::sync::Arc;
use wgpu::util::DeviceExt;

const CACHE_BYTES: usize = 48 * 1024 * 1024;
const CACHE_ENTRIES: usize = 128;
const POOL_BYTES: usize = 16 * 1024 * 1024;
const TILE_SIZE: u32 = 1024;

#[derive(Default)]
pub(super) struct State {
    cached: Vec<(u64, Target)>,
    pool: Vec<Target>,
    direct: Option<Direct>,
}

impl State {
    pub(super) fn bytes(&self) -> usize {
        self.cached
            .iter()
            .map(|(_, target)| target.bytes())
            .sum::<usize>()
            + self.pool.iter().map(Target::bytes).sum::<usize>()
    }

    pub(super) fn prepare(
        &mut self,
        pipeline: &Pipeline,
        blur: &blur::Pipeline,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        belt: &mut wgpu::util::StagingBelt,
        image: &Image,
        bounds: Rectangle,
        clip: Rectangle,
        visible_bounds: Rectangle,
        scale: f32,
        revision: u64,
        entry: &atlas::Entry,
        binding: &Arc<wgpu::BindGroup>,
        layer: &mut Layer,
        nearest: &[Instance],
        linear: &mut Vec<Instance>,
    ) -> (u64, u64) {
        let profile = image.blur.scaled(scale);
        let Blur::Linear(gradient) = profile else {
            return (0, 0);
        };
        if crate::core::image::crop_bounds(entry.size(), image.crop).is_none()
            || !f32::from(image.rotation).is_finite()
        {
            return (0, 0);
        }
        let size = [bounds.width.ceil() as u32, bounds.height.ceil() as u32];
        let halo = (profile.maximum() * 3.0).max(24.0).ceil() as u32;
        // Bound scratch allocations as well as individual device dimensions.
        // Exceptionally broad kernels sample the atlas directly instead of
        // allocating an enormous halo or reducing the sharp end's resolution.
        let source_limit = device.limits().max_texture_dimension_2d.min(2048);
        let available = source_limit.saturating_sub(halo.saturating_mul(2).saturating_add(4));
        let direct = available < 64 && (size[0] > source_limit || size[1] > source_limit);
        let tile_size = if available < 64 {
            TILE_SIZE.min(source_limit.saturating_sub(2)).max(1)
        } else {
            TILE_SIZE.min(available)
        };
        let key = blur::fingerprint(&(
            image.handle.id(),
            revision,
            image.crop,
            bounds.width.to_bits(),
            bounds.height.to_bits(),
            profile,
            image.filter_method,
            entry,
        ));
        let Some(visible_clip) = clip.intersection(&visible_bounds) else {
            return (0, 0);
        };
        let visible = visible_tiles(
            bounds,
            visible_clip,
            f32::from(image.rotation),
            size,
            tile_size,
        );
        let mut hits = 0;
        let mut misses = 0;
        let mut direct_fragments = None;

        for y in (visible.y..visible.y + visible.height).step_by(tile_size as usize) {
            for x in (visible.x..visible.x + visible.width).step_by(tile_size as usize) {
                let tile = Rectangle {
                    x,
                    y,
                    width: tile_size.min(size[0] - x),
                    height: tile_size.min(size[1] - y),
                };
                // Linear filtering of a rotated tile needs the adjacent output
                // texel. The gutter is excluded from geometric ownership.
                let output_region = expand(tile, 1, size);
                let tile_key = blur::fingerprint(&(key, output_region));
                let output = if let Some(index) =
                    self.cached.iter().position(|(key, _)| *key == tile_key)
                {
                    hits += 1;
                    self.cached.remove(index).1
                } else {
                    misses += 1;
                    let output = take_target(
                        &mut self.pool,
                        blur,
                        device,
                        [output_region.width, output_region.height],
                    );
                    if direct {
                        let direct = self
                            .direct
                            .get_or_insert_with(|| Direct::new(device, pipeline));
                        let fragments = direct_fragments.get_or_insert_with(|| {
                            direct.fragments(device, encoder, image, bounds, entry)
                        });
                        direct.render(
                            device,
                            encoder,
                            binding,
                            fragments,
                            &output,
                            output_region,
                            bounds,
                            profile,
                            image.filter_method,
                        );
                    } else {
                        let source_region = expand(output_region, halo + 1, size);
                        let source_size = [source_region.width, source_region.height];
                        let source = take_target(&mut self.pool, blur, device, source_size);
                        let intermediate = take_target(&mut self.pool, blur, device, source_size);
                        render_source(
                            pipeline,
                            device,
                            encoder,
                            belt,
                            image,
                            bounds,
                            entry,
                            binding,
                            &source,
                            source_region,
                        );
                        let reference = Rectangle {
                            x: -(source_region.x as f32),
                            y: -(source_region.y as f32),
                            width: bounds.width,
                            height: bounds.height,
                        };
                        let first_horizontal = gradient.direction == Direction::Horizontal;
                        let whole = Rectangle {
                            x: 0,
                            y: 0,
                            width: source_size[0],
                            height: source_size[1],
                        };
                        blur.profile_pass(
                            device,
                            encoder,
                            &source,
                            &intermediate.view,
                            whole,
                            float_region(whole),
                            profile,
                            reference,
                            first_horizontal,
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        );
                        blur.profile_pass(
                            device,
                            encoder,
                            &intermediate,
                            &output.view,
                            Rectangle {
                                x: 0,
                                y: 0,
                                width: output.size[0],
                                height: output.size[1],
                            },
                            Rectangle {
                                x: (output_region.x - source_region.x) as f32,
                                y: (output_region.y - source_region.y) as f32,
                                width: output.size[0] as f32,
                                height: output.size[1] as f32,
                            },
                            profile,
                            reference,
                            !first_horizontal,
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        );
                        recycle(&mut self.pool, source);
                        recycle(&mut self.pool, intermediate);
                    }
                    output
                };

                let width = (bounds.width - x as f32).min(tile.width as f32);
                let height = (bounds.height - y as f32).min(tile.height as f32);
                linear.push(Instance {
                    _bounds: [bounds.x, bounds.y, bounds.width, bounds.height],
                    _clip_bounds: [clip.x, clip.y, clip.width, clip.height],
                    _border_radius: scaled_radius(image.border_radius, scale, clip).into(),
                    _tile: [bounds.x + x as f32, bounds.y + y as f32, width, height],
                    _atlas: [
                        (x - output_region.x) as f32 / output.capacity[0] as f32,
                        (y - output_region.y) as f32 / output.capacity[1] as f32,
                        width / output.capacity[0] as f32,
                        height / output.capacity[1] as f32,
                    ],
                    _rotation: f32::from(image.rotation),
                    _opacity: if image.opacity.is_nan() {
                        0.0
                    } else {
                        image.opacity.clamp(0.0, 1.0)
                    },
                    _border_smoothing: crate::graphics::shape::normalize_smoothing(
                        image.border_smoothing,
                    ),
                    _layer: 0,
                    _edges: 16
                        | u32::from(x == 0)
                        | (u32::from(y == 0) << 1)
                        | (u32::from(x + tile.width == size[0]) << 2)
                        | (u32::from(y + tile.height == size[1]) << 3),
                });
                layer.push(&output.binding, nearest, linear);
                while !self.cached.is_empty()
                    && (self.cached.len() >= CACHE_ENTRIES
                        || self
                            .cached
                            .iter()
                            .map(|(_, target)| target.bytes())
                            .sum::<usize>()
                            + output.bytes()
                            > CACHE_BYTES)
                {
                    recycle(&mut self.pool, self.cached.remove(0).1);
                }
                if output.bytes() <= CACHE_BYTES {
                    self.cached.push((tile_key, output));
                } else {
                    recycle(&mut self.pool, output);
                }
            }
        }
        (hits, misses)
    }
}

fn take_target(
    pool: &mut Vec<Target>,
    blur: &blur::Pipeline,
    device: &wgpu::Device,
    size: [u32; 2],
) -> Target {
    // A retired output can still be referenced by an earlier draw this frame.
    // Scratch textures are reusable because their passes are ordered in one
    // encoder; final output bindings remain owned by Layer until trim.
    pool.iter()
        .position(|target| target.size == size && Arc::strong_count(&target.binding) == 1)
        .map(|index| pool.swap_remove(index))
        .unwrap_or_else(|| blur.target(device, size))
}

fn recycle(pool: &mut Vec<Target>, target: Target) {
    if target.bytes() > POOL_BYTES {
        return;
    }
    while !pool.is_empty()
        && (pool.len() >= 16
            || pool.iter().map(Target::bytes).sum::<usize>() + target.bytes() > POOL_BYTES)
    {
        drop(pool.remove(0));
    }
    pool.push(target);
}

fn float_region(region: Rectangle<u32>) -> Rectangle {
    Rectangle {
        x: region.x as f32,
        y: region.y as f32,
        width: region.width as f32,
        height: region.height as f32,
    }
}

fn expand(region: Rectangle<u32>, amount: u32, size: [u32; 2]) -> Rectangle<u32> {
    let x = region.x.saturating_sub(amount);
    let y = region.y.saturating_sub(amount);
    Rectangle {
        x,
        y,
        width: (region.x + region.width)
            .saturating_add(amount)
            .min(size[0])
            - x,
        height: (region.y + region.height)
            .saturating_add(amount)
            .min(size[1])
            - y,
    }
}

fn visible_tiles(
    bounds: Rectangle,
    clip: Rectangle,
    rotation: f32,
    size: [u32; 2],
    tile: u32,
) -> Rectangle<u32> {
    let center = bounds.center();
    let (sin, cos) = rotation.sin_cos();
    let mut low = [f32::INFINITY; 2];
    let mut high = [f32::NEG_INFINITY; 2];
    for (x, y) in [
        (clip.x - 1.0, clip.y - 1.0),
        (clip.x + clip.width + 1.0, clip.y - 1.0),
        (clip.x - 1.0, clip.y + clip.height + 1.0),
        (clip.x + clip.width + 1.0, clip.y + clip.height + 1.0),
    ] {
        let x = x - center.x;
        let y = y - center.y;
        let p = [
            x * cos + y * sin + bounds.width * 0.5,
            -x * sin + y * cos + bounds.height * 0.5,
        ];
        for axis in 0..2 {
            low[axis] = low[axis].min(p[axis]);
            high[axis] = high[axis].max(p[axis]);
        }
    }
    let x = ((low[0].max(0.0) as u32 / tile) * tile).min(size[0]);
    let y = ((low[1].max(0.0) as u32 / tile) * tile).min(size[1]);
    Rectangle {
        x,
        y,
        width: (high[0].ceil().max(0.0) as u32)
            .min(size[0])
            .saturating_sub(x),
        height: (high[1].ceil().max(0.0) as u32)
            .min(size[1])
            .saturating_sub(y),
    }
}

fn render_source(
    pipeline: &Pipeline,
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    belt: &mut wgpu::util::StagingBelt,
    image: &Image,
    bounds: Rectangle,
    entry: &atlas::Entry,
    binding: &Arc<wgpu::BindGroup>,
    source: &Target,
    region: Rectangle<u32>,
) {
    let mut rendition = Layer::new(
        device,
        &pipeline.constant_layout,
        &pipeline.nearest_sampler,
        &pipeline.linear_sampler,
    );
    let mut instances = Vec::new();
    add_instances(
        Rectangle {
            x: -(region.x as f32),
            y: -(region.y as f32),
            width: bounds.width,
            height: bounds.height,
        },
        Rectangle::new(
            Point::ORIGIN,
            Size::new(source.size[0] as f32, source.size[1] as f32),
        ),
        border::radius(0),
        0.0,
        image.crop,
        0.0,
        1.0,
        entry,
        &mut instances,
    );
    for instance in &mut instances {
        instance._edges |= 32;
        if image.filter_method == FilterMethod::Nearest {
            instance._edges |= 64;
        }
    }
    let (nearest, linear) = match image.filter_method {
        FilterMethod::Nearest => (instances.as_slice(), &[][..]),
        FilterMethod::Linear => (&[][..], instances.as_slice()),
    };
    rendition.push(binding, nearest, linear);
    rendition.prepare(
        device,
        encoder,
        belt,
        Transformation::orthographic(source.size[0], source.size[1]),
        nearest,
        linear,
    );
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("progressive image source tile"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &source.view,
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
    pass.set_viewport(
        0.0,
        0.0,
        source.size[0] as f32,
        source.size[1] as f32,
        0.0,
        1.0,
    );
    pass.set_scissor_rect(0, 0, source.size[0], source.size[1]);
    pass.set_pipeline(&pipeline.raw);
    rendition.render(&mut pass);
}

struct Direct {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

struct Fragments {
    view: wgpu::TextureView,
    count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Fragment {
    tile: [f32; 4],
    atlas: [f32; 4],
    flags: [f32; 4],
}

impl Direct {
    fn new(device: &wgpu::Device, image: &Pipeline) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("direct progressive image parameters"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("direct progressive image"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shader/image_progressive.wgsl").into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("direct progressive image"),
            bind_group_layouts: &[Some(&image.texture_layout), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("direct progressive image"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: image.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, layout }
    }

    fn fragments(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        image: &Image,
        bounds: Rectangle,
        entry: &atlas::Entry,
    ) -> Fragments {
        let frame = Rectangle {
            x: 0.0,
            y: 0.0,
            width: bounds.width,
            height: bounds.height,
        };
        let mut instances = Vec::new();
        add_instances(
            frame,
            frame,
            border::radius(0),
            0.0,
            image.crop,
            0.0,
            1.0,
            entry,
            &mut instances,
        );
        let fragments: Vec<_> = instances
            .into_iter()
            .map(|instance| Fragment {
                tile: instance._tile,
                atlas: instance._atlas,
                flags: [instance._layer as f32, instance._edges as f32, 0.0, 0.0],
            })
            .collect();
        // A sampled lookup texture also works on WebGL2, whose fragment-stage
        // storage-buffer limit may be zero. Each descriptor occupies 3 texels.
        let texels = fragments.len() as u32 * 3;
        let width = texels.min(device.limits().max_texture_dimension_2d.min(1024));
        let height = texels.div_ceil(width);
        let bytes_per_row = (width * 16).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let mut upload = vec![0u8; bytes_per_row as usize * height as usize];
        for (row, bytes) in bytemuck::cast_slice::<_, u8>(&fragments)
            .chunks(width as usize * 16)
            .enumerate()
        {
            let start = row * bytes_per_row as usize;
            upload[start..start + bytes.len()].copy_from_slice(bytes);
        }
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("progressive image atlas fragments upload"),
            contents: &upload,
            usage: wgpu::BufferUsages::COPY_SRC,
        });
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("progressive image atlas fragments"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            texture.as_image_copy(),
            size,
        );
        Fragments {
            count: fragments.len() as u32,
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        }
    }

    fn render(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        atlas: &Arc<wgpu::BindGroup>,
        fragments: &Fragments,
        output: &Target,
        region: Rectangle<u32>,
        bounds: Rectangle,
        profile: Blur,
        filter: FilterMethod,
    ) {
        let Blur::Linear(gradient) = profile else {
            return;
        };
        let parameters = [
            bounds.width,
            bounds.height,
            gradient.start,
            gradient.end,
            gradient.range[0],
            gradient.range[1],
            if gradient.direction == Direction::Horizontal {
                1.0
            } else {
                0.0
            },
            if filter == FilterMethod::Nearest {
                1.0
            } else {
                0.0
            },
            region.x as f32,
            region.y as f32,
            region.width as f32,
            region.height as f32,
            fragments.count as f32,
            0.0,
            0.0,
            0.0,
        ];
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("progressive image direct parameters"),
            contents: bytemuck::cast_slice(&parameters),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let constants = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("progressive image direct parameters"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&fragments.view),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("progressive image broad kernel tile"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &output.view,
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
        pass.set_viewport(
            0.0,
            0.0,
            output.size[0] as f32,
            output.size[1] as f32,
            0.0,
            1.0,
        );
        pass.set_scissor_rect(0, 0, output.size[0], output.size[1]);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, atlas.as_ref(), &[]);
        pass.set_bind_group(1, &constants, &[]);
        pass.draw(0..3, 0..1);
    }
}
