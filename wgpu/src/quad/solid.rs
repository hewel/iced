use crate::Buffer;
use crate::graphics::color;
use crate::quad::{self, Quad};

use bytemuck::{Pod, Zeroable};
use std::ops::Range;

/// A quad filled with a solid color.
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct Solid {
    /// The background color data of the quad.
    pub color: color::Packed,

    /// The [`Quad`] data of the [`Solid`].
    pub quad: Quad,
}

const _: () = {
    assert!(std::mem::size_of::<Solid>() == 104);
    assert!(std::mem::align_of::<Solid>() == 4);
    assert!(std::mem::offset_of!(Solid, quad) == 16);
    assert!(std::mem::offset_of!(Solid, quad) + std::mem::offset_of!(Quad, smoothing) == 100);
};

#[derive(Debug)]
pub struct Layer {
    instances: Buffer<Solid>,
    instance_count: usize,
}

impl Layer {
    pub fn new(device: &wgpu::Device) -> Self {
        let instances = Buffer::new(
            device,
            "iced_wgpu.quad.solid.buffer",
            quad::INITIAL_INSTANCES,
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        );

        Self {
            instances,
            instance_count: 0,
        }
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        belt: &mut wgpu::util::StagingBelt,
        instances: &[Solid],
    ) {
        let _ = self.instances.resize(device, instances.len());
        let _ = self.instances.write(encoder, belt, 0, instances);

        self.instance_count = instances.len();
    }
}

#[derive(Debug, Clone)]
pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
}

impl Pipeline {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        constants_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("iced_wgpu.quad.solid.pipeline"),
            bind_group_layouts: &[Some(constants_layout)],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("iced_wgpu.quad.solid.shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(concat!(
                include_str!("../shader/quad/snap.wgsl"),
                "\n",
                include_str!("../shader/color.wgsl"),
                "\n",
                include_str!("../shader/shape.wgsl"),
                "\n",
                include_str!("../shader/quad.wgsl"),
                "\n",
                include_str!("../shader/vertex.wgsl"),
                "\n",
                include_str!("../shader/quad/solid.wgsl"),
            ))),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("iced_wgpu.quad.solid.pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("solid_vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Solid>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: (std::mem::offset_of!(Solid, color)) as u64,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: (std::mem::offset_of!(Solid, quad)
                                + std::mem::offset_of!(Quad, position))
                                as u64,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: (std::mem::offset_of!(Solid, quad)
                                + std::mem::offset_of!(Quad, size))
                                as u64,
                            shader_location: 2,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: (std::mem::offset_of!(Solid, quad)
                                + std::mem::offset_of!(Quad, border_color))
                                as u64,
                            shader_location: 3,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: (std::mem::offset_of!(Solid, quad)
                                + std::mem::offset_of!(Quad, border_radius))
                                as u64,
                            shader_location: 4,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32,
                            offset: (std::mem::offset_of!(Solid, quad)
                                + std::mem::offset_of!(Quad, border_width))
                                as u64,
                            shader_location: 5,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: (std::mem::offset_of!(Solid, quad)
                                + std::mem::offset_of!(Quad, shadow_color))
                                as u64,
                            shader_location: 6,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: (std::mem::offset_of!(Solid, quad)
                                + std::mem::offset_of!(Quad, shadow_offset))
                                as u64,
                            shader_location: 7,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32,
                            offset: (std::mem::offset_of!(Solid, quad)
                                + std::mem::offset_of!(Quad, shadow_blur_radius))
                                as u64,
                            shader_location: 8,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Uint32,
                            offset: (std::mem::offset_of!(Solid, quad)
                                + std::mem::offset_of!(Quad, snap))
                                as u64,
                            shader_location: 9,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32,
                            offset: (std::mem::offset_of!(Solid, quad)
                                + std::mem::offset_of!(Quad, smoothing))
                                as u64,
                            shader_location: 10,
                        },
                    ],
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("solid_fs_main"),
                targets: &quad::color_target_state(format),
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

        Self { pipeline }
    }

    pub fn render<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        constants: &'a wgpu::BindGroup,
        layer: &'a Layer,
        range: Range<usize>,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, constants, &[]);
        render_pass.set_vertex_buffer(0, layer.instances.slice(..));

        render_pass.draw(0..6, range.start as u32..range.end as u32);
    }
}
