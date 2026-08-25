use std::borrow::Cow;

use bytemuck::{Pod, Zeroable, cast_ref};

use wgpu::RenderPipeline;
use wgpu::CommandEncoder;
use wgpu::{RenderPass, RenderPassDescriptor, RenderPassColorAttachment};
use wgpu::{RenderPipelineDescriptor, ShaderModuleDescriptor, ShaderSource};
use wgpu::{PipelineLayoutDescriptor};
use wgpu::{VertexState, PrimitiveState, MultisampleState, FragmentState, PipelineCompilationOptions};
use wgpu::{PrimitiveTopology, FrontFace, PolygonMode};
use wgpu::{Buffer, BufferDescriptor, BufferUsages};
use wgpu::{BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntry, ShaderStages};
use wgpu::{BindGroup, BindGroupDescriptor, BindGroupEntry, BindingType, BufferBindingType};
use wgpu::{TextureFormat, TextureView};
use wgpu::{Operations, LoadOp, StoreOp};
use wgpu::{CompareFunction, DepthBiasState, DepthStencilState, IndexFormat, RenderPassDepthStencilAttachment, StencilState};
use wgpu::{VertexBufferLayout, VertexStepMode, VertexAttribute, VertexFormat};

use crate::CommonWgpuObjects;
use super::PointPosition;

#[derive(Copy, Clone)]
pub struct DrawConstants {
    pub point_size: f32,
    pub corner_colors: [[f32; 4]; 4],
    pub points_circular: bool,
}
impl Default for DrawConstants {
    fn default() -> Self {
        Self {
            point_size: 0.001,
            corner_colors: [[1.0; 4]; 4],
            points_circular: false,
        }
    }
}
impl DrawConstants {
    fn into_constants_for_pipeline(&self) -> [(&str, f64); 18] {
        [
            ("POINT_SIZE", self.point_size as f64),
            ("CORNER_COLOR_0R", self.corner_colors[0][0] as f64),
            ("CORNER_COLOR_0G", self.corner_colors[0][1] as f64),
            ("CORNER_COLOR_0B", self.corner_colors[0][2] as f64),
            ("CORNER_COLOR_0A", self.corner_colors[0][3] as f64),
            ("CORNER_COLOR_1R", self.corner_colors[1][0] as f64),
            ("CORNER_COLOR_1G", self.corner_colors[1][1] as f64),
            ("CORNER_COLOR_1B", self.corner_colors[1][2] as f64),
            ("CORNER_COLOR_1A", self.corner_colors[1][3] as f64),
            ("CORNER_COLOR_2R", self.corner_colors[2][0] as f64),
            ("CORNER_COLOR_2G", self.corner_colors[2][1] as f64),
            ("CORNER_COLOR_2B", self.corner_colors[2][2] as f64),
            ("CORNER_COLOR_2A", self.corner_colors[2][3] as f64),
            ("CORNER_COLOR_3R", self.corner_colors[3][0] as f64),
            ("CORNER_COLOR_3G", self.corner_colors[3][1] as f64),
            ("CORNER_COLOR_3B", self.corner_colors[3][2] as f64),
            ("CORNER_COLOR_3A", self.corner_colors[3][3] as f64),
            ("POINTS_CIRCULAR", if self.points_circular { 1.0 } else { 0.0 }),
        ]
    }
}

/// Draws a point simulation on the screen
pub struct PointSimDraw {
    render_pipeline: RenderPipeline,
    draw_bind_group: BindGroup,
    draw_uniform_buffer: Buffer,
}
impl PointSimDraw {
    pub fn new(
        co: &CommonWgpuObjects,
        depth_texture_format: TextureFormat,
        render_pipeline_constants: &DrawConstants
    ) -> Self {
        let (draw_bind_group_layout, render_pipeline) = create_render_pipeline(
            &co,
            depth_texture_format,
            &render_pipeline_constants.into_constants_for_pipeline()
        );
        let draw_uniform_buffer = DrawUniform::create_buffer(&co);

        let draw_bind_group = co.device.create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &draw_bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: draw_uniform_buffer.as_entire_binding(),
                },
            ],
        });

        Self {
            render_pipeline,
            draw_bind_group,
            draw_uniform_buffer,
        }
    }
}
pub struct PointSimDrawInfo<'a,
IBF: Fn(&mut RenderPass),
VBF: Fn(&mut RenderPass, u32) -> u32,
VBFI: Iterator<Item=VBF>,
>
{
    pub window_size: &'a [f32; 2],
    pub clear_color: &'a [f64; 4],

    pub co: &'a CommonWgpuObjects,
    pub command_encoder: &'a mut CommandEncoder,
    pub surface_texture_view: &'a TextureView,
    pub depth_texture_view: &'a TextureView,
    pub index_buffer_binder: IBF,
    pub vertex_buffer_binders: VBFI,
}
impl PointSimDraw {
    fn calculate_view_scaling(window_size: &[f32; 2]) -> [f32; 2] {
        // always draw the 2x2 square centered at 0
        if window_size[0] > window_size[1] { // width > height
            [
                window_size[1] / window_size[0],
                1.0,
            ]
        } else { // height > width
            [
                1.0,
                window_size[0] / window_size[1],
            ]
        }
    }
    pub fn record_draw<
        IBF: Fn(&mut RenderPass),
        VBF: Fn(&mut RenderPass, u32) -> u32,
        VBFI: Iterator<Item=VBF>,
        > (&self, info: PointSimDrawInfo<IBF, VBF, VBFI>)
    {
        let render_pass_descriptor = RenderPassDescriptor {
            label: None,
            color_attachments: &[
                Some(RenderPassColorAttachment {
                    view: &info.surface_texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(wgpu::Color{
                            r: info.clear_color[0],
                            g: info.clear_color[1],
                            b: info.clear_color[2],
                            a: info.clear_color[3]
                        }),
                        store: StoreOp::Store,
                    },
                }),
            ],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &info.depth_texture_view,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(1.0),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        };

        // draw newly generated geometry
        { let mut render_pass = info.command_encoder.begin_render_pass(&render_pass_descriptor);
            render_pass.set_pipeline(&self.render_pipeline);

            DrawUniform {
                view_scaling: Self::calculate_view_scaling(info.window_size),
            }.write_buffer(info.co, &self.draw_uniform_buffer, 0);
            render_pass.set_bind_group(0, &self.draw_bind_group, &[]);

            // bind shared index buffer
            (info.index_buffer_binder)(&mut render_pass);
            // point sim object provides iterator of vertex buffers to be drawn
            // each iterator provides the number of indices required to draw all of its points
            for vertex_buffer_binder in info.vertex_buffer_binders {
                let index_count = vertex_buffer_binder(&mut render_pass, 0);
                render_pass.draw_indexed(0..index_count, 0, 0..1);
            }
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Zeroable, Pod)]
struct DrawUniform {
    view_scaling: [f32; 2],
}
impl DrawUniform {
    fn create_buffer(co: &CommonWgpuObjects) -> Buffer {
        co.device.create_buffer(&BufferDescriptor {
            label: None,
            size: size_of::<Self>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }
    fn write_buffer(&self, co: &CommonWgpuObjects, buffer: &Buffer, offset: u64) {
        co.queue.write_buffer(buffer, offset, cast_ref(self) as &[u8; size_of::<Self>()]);
    }
}

fn create_render_pipeline(
    co: &CommonWgpuObjects,
    depth_texture_format: TextureFormat,
    constants: &[(&str, f64)]
) -> (BindGroupLayout, RenderPipeline) {
    let bind_group_layout = co.device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            BindGroupLayoutEntry { // uniform buffer
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ]
    });
    let render_pipeline = {
        let point_shader_module = co.device.create_shader_module(ShaderModuleDescriptor {
            label: None,
            source: ShaderSource::Wgsl(Cow::Borrowed(include_str!("point.wgsl"))),
        });

        let vertex_buffer_layouts = [
            Some(VertexBufferLayout {
                array_stride: size_of::<PointPosition>() as u64,
                step_mode: VertexStepMode::Vertex,
                attributes: &[
                    VertexAttribute {
                        format: VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    },
                ],
            }),
        ];

        let pipeline_layout = co.device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        // TODO handle change of surface capabilities or format
        co.device.create_render_pipeline(&RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &point_shader_module,
                entry_point: Some("vertex_main"),
                compilation_options: PipelineCompilationOptions {
                    constants,
                    zero_initialize_workgroup_memory: false,
                },
                buffers: &vertex_buffer_layouts,
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleStrip,
                strip_index_format: Some(IndexFormat::Uint32),
                front_face: FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(DepthStencilState {
                format: depth_texture_format,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::Less),
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState::default(),
            fragment: Some(FragmentState {
                module: &point_shader_module,
                entry_point: Some("fragment_main"),
                compilation_options: PipelineCompilationOptions {
                    constants,
                    zero_initialize_workgroup_memory: false,
                },
                targets: &[Some(co.surface_format.into())],
            }),
            multiview_mask: None,
            cache: None,
        })
    };

    (bind_group_layout, render_pipeline)
}
