use crate::Buffer;
use crate::graphics::gradient;
use crate::quad::{self, Quad};

use bytemuck::{Pod, Zeroable};
use std::ops::Range;

/// A quad filled with interpolated colors.
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct Gradient {
    /// The background gradient data of the quad.
    pub gradient: gradient::Packed,

    /// The [`Quad`] data of the [`Gradient`].
    pub quad: Quad,
}

const _: () = {
    assert!(std::mem::size_of::<Gradient>() == 184);
    assert!(std::mem::align_of::<Gradient>() == 4);
    assert!(std::mem::size_of::<gradient::Packed>() == 96);
    assert!(std::mem::offset_of!(Gradient, quad) == 96);
    assert!(std::mem::offset_of!(Gradient, quad) + std::mem::offset_of!(Quad, snap) == 176);
    assert!(std::mem::offset_of!(Gradient, quad) + std::mem::offset_of!(Quad, smoothing) == 180);
};

#[derive(Debug)]
pub struct Layer {
    instances: Buffer<Gradient>,
    instance_count: usize,
}

impl Layer {
    pub fn new(device: &wgpu::Device) -> Self {
        let instances = Buffer::new(
            device,
            "iced_wgpu.quad.gradient.buffer",
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
        instances: &[Gradient],
    ) {
        let _ = self.instances.resize(device, instances.len());
        let _ = self.instances.write(encoder, belt, 0, instances);

        self.instance_count = instances.len();
    }
}

#[derive(Debug, Clone)]
pub struct Pipeline {
    #[cfg(not(target_arch = "wasm32"))]
    pipeline: wgpu::RenderPipeline,
}

impl Pipeline {
    #[allow(unused_variables)]
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        constants_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("iced_wgpu.quad.gradient.pipeline"),
                bind_group_layouts: &[Some(constants_layout)],
                immediate_size: 0,
            });

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("iced_wgpu.quad.gradient.shader"),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(concat!(
                    include_str!("../shader/quad/snap.wgsl"),
                    "\n",
                    include_str!("../shader/shape.wgsl"),
                    "\n",
                    include_str!("../shader/quad.wgsl"),
                    "\n",
                    include_str!("../shader/vertex.wgsl"),
                    "\n",
                    include_str!("../shader/quad/gradient.wgsl"),
                    "\n",
                    include_str!("../shader/color.wgsl"),
                    "\n",
                    include_str!("../shader/color/linear_rgb.wgsl")
                ))),
            });

            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("iced_wgpu.quad.gradient.pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("gradient_vs_main"),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Gradient>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Uint32x4,
                                offset: std::mem::offset_of!(Gradient, gradient) as u64,
                                shader_location: 0,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Uint32x4,
                                offset: (std::mem::offset_of!(Gradient, gradient) + 16) as u64,
                                shader_location: 1,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Uint32x4,
                                offset: (std::mem::offset_of!(Gradient, gradient) + 32) as u64,
                                shader_location: 2,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Uint32x4,
                                offset: (std::mem::offset_of!(Gradient, gradient) + 48) as u64,
                                shader_location: 3,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Uint32x4,
                                offset: (std::mem::offset_of!(Gradient, gradient) + 64) as u64,
                                shader_location: 4,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x4,
                                offset: (std::mem::offset_of!(Gradient, gradient) + 80) as u64,
                                shader_location: 5,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x4,
                                offset: (std::mem::offset_of!(Gradient, quad)
                                    + std::mem::offset_of!(Quad, position))
                                    as u64,
                                shader_location: 6,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x4,
                                offset: (std::mem::offset_of!(Gradient, quad)
                                    + std::mem::offset_of!(Quad, border_color))
                                    as u64,
                                shader_location: 7,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x4,
                                offset: (std::mem::offset_of!(Gradient, quad)
                                    + std::mem::offset_of!(Quad, border_radius))
                                    as u64,
                                shader_location: 8,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32,
                                offset: (std::mem::offset_of!(Gradient, quad)
                                    + std::mem::offset_of!(Quad, border_width))
                                    as u64,
                                shader_location: 9,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x4,
                                offset: (std::mem::offset_of!(Gradient, quad)
                                    + std::mem::offset_of!(Quad, shadow_color))
                                    as u64,
                                shader_location: 10,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: (std::mem::offset_of!(Gradient, quad)
                                    + std::mem::offset_of!(Quad, shadow_offset))
                                    as u64,
                                shader_location: 11,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32,
                                offset: (std::mem::offset_of!(Gradient, quad)
                                    + std::mem::offset_of!(Quad, shadow_blur_radius))
                                    as u64,
                                shader_location: 12,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Uint32,
                                offset: (std::mem::offset_of!(Gradient, quad)
                                    + std::mem::offset_of!(Quad, snap))
                                    as u64,
                                shader_location: 13,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32,
                                offset: (std::mem::offset_of!(Gradient, quad)
                                    + std::mem::offset_of!(Quad, smoothing))
                                    as u64,
                                shader_location: 14,
                            },
                        ],
                    }],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("gradient_fs_main"),
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

        #[cfg(target_arch = "wasm32")]
        Self {}
    }

    #[allow(unused_variables)]
    pub fn render<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        constants: &'a wgpu::BindGroup,
        layer: &'a Layer,
        range: Range<usize>,
    ) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, constants, &[]);
            render_pass.set_vertex_buffer(0, layer.instances.slice(..));

            render_pass.draw(0..6, range.start as u32..range.end as u32);
        }
    }
}
