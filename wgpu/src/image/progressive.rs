//! Tiled image convolution. Filtering precedes reduction along either axis;
//! progressive profiles retain full resolution through their sharp endpoint.

use super::{Instance, Layer, Pipeline, add_instances, atlas, scaled_radius};
use crate::blur::{self, Target};
use crate::core::blur::Direction;
use crate::core::image::{FilterMethod, Image};
use crate::core::{Blur, Point, Rectangle, Size, Transformation, border};
use std::sync::Arc;
use wgpu::util::DeviceExt;

const CACHE_BYTES: usize = 56 * 1024 * 1024;
const CACHE_ENTRIES: usize = 128;
const POOL_BYTES: usize = 8 * 1024 * 1024;
const SCRATCH_TARGET_BYTES: usize = 16 * 1024 * 1024;
const TILE_SIZE: u32 = 1024;

#[derive(Default)]
pub(super) struct State {
    cached: Vec<(u64, Target)>,
    pool: Vec<Target>,
    source: Option<Target>,
    intermediate: Option<Target>,
    direct: Option<Direct>,
}

impl State {
    pub(super) fn bytes(&self) -> usize {
        self.cached
            .iter()
            .map(|(_, target)| target.bytes())
            .sum::<usize>()
            + self.pool.iter().map(Target::bytes).sum::<usize>()
            + self.scratch_bytes()
    }

    fn scratch_bytes(&self) -> usize {
        self.source.as_ref().map_or(0, Target::bytes)
            + self.intermediate.as_ref().map_or(0, Target::bytes)
            + self.direct.as_ref().map_or(0, Direct::bytes)
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
        let first_horizontal = match profile {
            Blur::Uniform(_) => true,
            Blur::Linear(gradient) => gradient.direction == Direction::Horizontal,
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
        let bytes_per_pixel = pipeline.format.block_copy_size(None).unwrap_or(4) as usize
            * if pipeline.backend == wgpu::Backend::Gl {
                2
            } else {
                1
            };
        let source_limit = device
            .limits()
            .max_texture_dimension_2d
            .min(2048)
            .min(((SCRATCH_TARGET_BYTES / bytes_per_pixel) as f64).sqrt() as u32);
        let resolution = match profile {
            Blur::Uniform(sigma) => (sigma / 8.0).max(1.0).recip(),
            Blur::Linear(_) => 1.0,
        };
        let gutter = resolution.recip().ceil() as u32;
        // Each side needs one filtered gutter texel plus up to one texel for
        // rounding onto the globally aligned reduced grid. Reserve both before
        // choosing a physical tile so the expanded source fits its scratch.
        let available = source_limit.saturating_sub(
            halo.saturating_mul(2)
                .saturating_add(gutter.saturating_mul(4))
                .saturating_add(4),
        );
        let direct = available < 64 && (size[0] > source_limit || size[1] > source_limit);
        let grid_size = [
            (size[0] as f32 * resolution).ceil() as u32,
            (size[1] as f32 * resolution).ceil() as u32,
        ];
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
            resolution.to_bits(),
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
                let grid_x = (tile.x as f32 * resolution).floor() as u32;
                let grid_y = (tile.y as f32 * resolution).floor() as u32;
                let output_region = expand(
                    Rectangle {
                        x: grid_x,
                        y: grid_y,
                        width: ((tile.x + tile.width) as f32 * resolution).ceil() as u32 - grid_x,
                        height: ((tile.y + tile.height) as f32 * resolution).ceil() as u32 - grid_y,
                    },
                    1,
                    grid_size,
                );
                let output_physical = Rectangle {
                    x: output_region.x as f32 / resolution,
                    y: output_region.y as f32 / resolution,
                    width: output_region.width as f32 / resolution,
                    height: output_region.height as f32 / resolution,
                };
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
                            output_physical,
                            bounds,
                            profile,
                            image.filter_method,
                        );
                    } else {
                        let source_x = output_physical.x.floor() as u32;
                        let source_y = output_physical.y.floor() as u32;
                        let source_region = expand(
                            Rectangle {
                                x: source_x,
                                y: source_y,
                                width: (output_physical.x + output_physical.width).ceil() as u32
                                    - source_x,
                                height: (output_physical.y + output_physical.height).ceil() as u32
                                    - source_y,
                            },
                            halo + 1,
                            size,
                        );
                        let source_size = [source_region.width, source_region.height];
                        let source =
                            take_scratch(&mut self.source, blur, device, source_size, source_limit);
                        // Keep the untouched axis at source resolution until
                        // its convolution has removed detail that would alias.
                        let intermediate_size = if first_horizontal {
                            [output_region.width, source_size[1]]
                        } else {
                            [source_size[0], output_region.height]
                        };
                        let intermediate = take_scratch(
                            &mut self.intermediate,
                            blur,
                            device,
                            intermediate_size,
                            source_limit,
                        );
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
                        let first_destination = Rectangle {
                            x: 0,
                            y: 0,
                            width: intermediate_size[0],
                            height: intermediate_size[1],
                        };
                        let first_region = if first_horizontal {
                            Rectangle {
                                x: output_physical.x - source_region.x as f32,
                                y: 0.0,
                                width: output_physical.width,
                                height: source_size[1] as f32,
                            }
                        } else {
                            Rectangle {
                                x: 0.0,
                                y: output_physical.y - source_region.y as f32,
                                width: source_size[0] as f32,
                                height: output_physical.height,
                            }
                        };
                        blur.profile_pass(
                            device,
                            encoder,
                            &source,
                            &intermediate.view,
                            first_destination,
                            first_region,
                            profile,
                            reference,
                            first_horizontal,
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        );
                        let (second_region, second_reference) = if first_horizontal {
                            (
                                Rectangle {
                                    x: 0.0,
                                    y: output_physical.y - source_region.y as f32,
                                    width: output_region.width as f32,
                                    height: output_physical.height,
                                },
                                Rectangle {
                                    x: -output_physical.x * resolution,
                                    width: bounds.width * resolution,
                                    ..reference
                                },
                            )
                        } else {
                            (
                                Rectangle {
                                    x: output_physical.x - source_region.x as f32,
                                    y: 0.0,
                                    width: output_physical.width,
                                    height: output_region.height as f32,
                                },
                                Rectangle {
                                    y: -output_physical.y * resolution,
                                    height: bounds.height * resolution,
                                    ..reference
                                },
                            )
                        };
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
                            second_region,
                            profile,
                            second_reference,
                            !first_horizontal,
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        );
                        self.source = Some(source);
                        self.intermediate = Some(intermediate);
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
                        (x as f32 - output_physical.x) * resolution / output.capacity[0] as f32,
                        (y as f32 - output_physical.y) * resolution / output.capacity[1] as f32,
                        width * resolution / output.capacity[0] as f32,
                        height * resolution / output.capacity[1] as f32,
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
                let cache_budget = CACHE_BYTES.saturating_sub(self.scratch_bytes());
                while !self.cached.is_empty()
                    && (self.cached.len() >= CACHE_ENTRIES
                        || self
                            .cached
                            .iter()
                            .map(|(_, target)| target.bytes())
                            .sum::<usize>()
                            + output.bytes()
                            > cache_budget)
                {
                    recycle(&mut self.pool, self.cached.remove(0).1);
                }
                if output.bytes() <= cache_budget {
                    self.cached.push((tile_key, output));
                } else {
                    recycle(&mut self.pool, output);
                }
            }
        }
        (hits, misses)
    }
}

fn take_scratch(
    slot: &mut Option<Target>,
    blur: &blur::Pipeline,
    device: &wgpu::Device,
    size: [u32; 2],
    limit: u32,
) -> Target {
    // Scratch passes and their consumers are fully encoded before the next
    // tile. Reuse their storage even when that tile needs different dimensions.
    // Keeping both slots separate prevents a small output pool from evicting a
    // large source every tile while the encoder still retains its allocation.
    let previous = slot.take();
    if let Some(mut target) = previous {
        if target.capacity[0] >= size[0] && target.capacity[1] >= size[1] {
            target.size = size;
            return target;
        }
        let capacity = [
            target.capacity[0]
                .max(size[0])
                .next_power_of_two()
                .min(limit),
            target.capacity[1]
                .max(size[1])
                .next_power_of_two()
                .min(limit),
        ];
        let mut target = blur.target(device, capacity);
        target.size = size;
        return target;
    }
    let mut target = blur.target(
        device,
        [
            size[0].next_power_of_two().min(limit),
            size[1].next_power_of_two().min(limit),
        ],
    );
    target.size = size;
    target
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
    first: wgpu::RenderPipeline,
    accumulate: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    resolve_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    scratch: Option<NativeScratch>,
}

struct Fragments {
    view: wgpu::TextureView,
    count: u32,
    size: [u32; 2],
}

struct NativeTarget {
    views: [wgpu::TextureView; 2],
    array: wgpu::TextureView,
    binding: wgpu::BindGroup,
    size: [u32; 2],
}

struct NativeScratch {
    intermediate: NativeTarget,
    accumulation: [NativeTarget; 2],
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
        let entries = [
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(96),
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
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
        ];
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("direct progressive image parameters"),
            entries: &entries[..2],
        });
        let resolve_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("progressive image resolve parameters"),
            entries: &entries,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("direct progressive image"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shader/image_progressive.wgsl").into(),
            ),
        });
        let create = |entry_point, format, dual, constants: &wgpu::BindGroupLayout| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("native progressive image"),
                bind_group_layouts: &[Some(&image.texture_layout), Some(constants)],
                immediate_size: 0,
            });
            let target = Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            });
            let targets = [target.clone(), target];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                    entry_point: Some(entry_point),
                    targets: if dual { &targets } else { &targets[..1] },
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let first = create("fs_first", wgpu::TextureFormat::Rgba8Unorm, true, &layout);
        let accumulate = create(
            "fs_accumulate",
            wgpu::TextureFormat::Rgba8Unorm,
            true,
            &resolve_layout,
        );
        let resolve = create("fs_resolve", image.format, false, &resolve_layout);
        Self {
            first,
            accumulate,
            resolve,
            layout,
            resolve_layout,
            texture_layout: image.texture_layout.clone(),
            scratch: None,
        }
    }

    fn bytes(&self) -> usize {
        self.scratch.as_ref().map_or(0, |scratch| {
            [
                &scratch.intermediate,
                &scratch.accumulation[0],
                &scratch.accumulation[1],
            ]
            .into_iter()
            .map(|target| target.size[0] as usize * target.size[1] as usize * 8)
            .sum()
        })
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
            size: {
                let crop = crate::core::image::crop_bounds(entry.size(), image.crop)
                    .expect("validated image crop");
                [crop.width, crop.height]
            },
        }
    }

    fn render(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        atlas: &Arc<wgpu::BindGroup>,
        fragments: &Fragments,
        output: &Target,
        region: Rectangle,
        bounds: Rectangle,
        profile: Blur,
        filter: FilterMethod,
    ) {
        let gradient = match profile {
            Blur::Linear(gradient) => gradient,
            Blur::Uniform(sigma) => crate::core::blur::Linear {
                direction: Direction::Horizontal,
                start: sigma,
                end: sigma,
                range: [0.0, 1.0],
            },
        };
        let horizontal = gradient.direction == Direction::Horizontal;
        let axis_size = if horizontal {
            output.size[0]
        } else {
            output.size[1]
        };
        let (native_count, physical_extent, physical_start, physical_length) = if horizontal {
            (fragments.size[1], bounds.height, region.y, region.height)
        } else {
            (fragments.size[0], bounds.width, region.x, region.width)
        };
        let reference = Rectangle {
            x: 0.0,
            y: 0.0,
            width: bounds.width,
            height: bounds.height,
        };
        let pixel_width = region.width / output.size[0] as f32;
        let pixel_height = region.height / output.size[1] as f32;
        let maximum = profile
            .radius_at(
                Point::new(region.x + pixel_width * 0.5, region.y + pixel_height * 0.5),
                reference,
            )
            .max(profile.radius_at(
                Point::new(
                    region.x + region.width - pixel_width * 0.5,
                    region.y + region.height - pixel_height * 0.5,
                ),
                reference,
            ));
        let ratio = f64::from(native_count) / f64::from(physical_extent);
        let halo = 4.0 * f64::from(maximum) * ratio + 1.0;
        let half_pixel = f64::from(if horizontal {
            pixel_height
        } else {
            pixel_width
        }) * 0.5;
        let first = (((f64::from(physical_start) + half_pixel) * ratio - 0.5 - halo)
            .floor()
            .max(0.0) as u32)
            .min(native_count - 1);
        let end = (((f64::from(physical_start + physical_length) - half_pixel) * ratio - 0.5 + halo)
            .ceil()
            .max(0.0) as u32)
            .min(native_count - 1) + 1;
        // Reuse packed 16-bit scratch across all tiles and images. Command
        // ordering consumes every stripe before reuse; cached final outputs
        // never refer to these textures. Its capacity counts against the
        // existing cache budget instead of accumulating until queue submission.
        let stripe_rows = (end - first).min(512).max(1);
        if self.scratch.is_none() {
            let side = (TILE_SIZE + 2).min(device.limits().max_texture_dimension_2d);
            self.scratch = Some(NativeScratch {
                intermediate: self.target(device, [side, 512.min(side)]),
                accumulation: [
                    self.target(device, [side, side]),
                    self.target(device, [side, side]),
                ],
            });
        }
        let scratch = self.scratch.as_ref().expect("native image scratch");
        let intermediate = &scratch.intermediate;
        let mut parameters = [
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
            region.x,
            region.y,
            region.width,
            region.height,
            fragments.count as f32,
            fragments.size[0] as f32,
            fragments.size[1] as f32,
            0.0,
            first as f32,
            stripe_rows as f32,
            0.0,
            0.0,
            output.size[0] as f32,
            output.size[1] as f32,
            0.0,
            0.0,
        ];
        let mut latest = 0;
        for (stripe, start) in (first..end).step_by(stripe_rows as usize).enumerate() {
            let rows = (end - start).min(stripe_rows);
            parameters[16] = start as f32;
            parameters[17] = rows as f32;
            parameters[19] = if stripe == 0 { 0.0 } else { 1.0 };
            let constants = self.parameters(device, fragments, &parameters, None);
            Self::pass(
                encoder,
                &self.first,
                atlas.as_ref(),
                &constants,
                &intermediate.views,
                [axis_size, rows],
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            );
            let next = stripe % 2;
            let previous = &scratch.accumulation[1 - next];
            let constants = self.parameters(device, fragments, &parameters, Some(&previous.array));
            Self::pass(
                encoder,
                &self.accumulate,
                &intermediate.binding,
                &constants,
                &scratch.accumulation[next].views,
                output.size,
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            );
            latest = next;
        }
        let constants = self.parameters(
            device,
            fragments,
            &parameters,
            Some(&scratch.accumulation[latest].array),
        );
        Self::pass(
            encoder,
            &self.resolve,
            atlas.as_ref(),
            &constants,
            std::slice::from_ref(&output.view),
            output.size,
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
        );
    }

    fn target(&self, device: &wgpu::Device, size: [u32; 2]) -> NativeTarget {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("progressive image native scratch"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 2,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // GLES/WebGL2 does not universally support float render targets.
            // Two RGBA8 attachments store four 16-bit normalized channels;
            // shader ping-pong accumulation avoids optional float blending.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let views = [0, 1].map(|layer| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            })
        });
        let array = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("progressive image native stripe"),
            layout: &self.texture_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&array),
            }],
        });
        NativeTarget {
            views,
            array,
            binding,
            size,
        }
    }

    fn parameters(
        &self,
        device: &wgpu::Device,
        fragments: &Fragments,
        parameters: &[f32],
        accumulation: Option<&wgpu::TextureView>,
    ) -> wgpu::BindGroup {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("progressive image direct parameters"),
            contents: bytemuck::cast_slice(parameters),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&fragments.view),
            },
        ];
        if let Some(view) = accumulation {
            entries.push(wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("progressive image direct parameters"),
            layout: if accumulation.is_some() {
                &self.resolve_layout
            } else {
                &self.layout
            },
            entries: &entries,
        })
    }

    fn pass(
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        source: &wgpu::BindGroup,
        constants: &wgpu::BindGroup,
        destinations: &[wgpu::TextureView],
        size: [u32; 2],
        load: wgpu::LoadOp<wgpu::Color>,
    ) {
        let attachments: Vec<_> = destinations
            .iter()
            .map(|view| {
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })
            })
            .collect();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("progressive image native convolution"),
            color_attachments: &attachments,
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(0.0, 0.0, size[0] as f32, size[1] as f32, 0.0, 1.0);
        pass.set_scissor_rect(0, 0, size[0], size[1]);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, source, &[]);
        pass.set_bind_group(1, constants, &[]);
        pass.draw(0..3, 0..1);
    }
}
