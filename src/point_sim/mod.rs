mod draw;
pub use draw::{DrawConstants, PointSimDraw, PointSimDrawInfo};

use std::mem::{swap, size_of, offset_of};
use std::cmp::{min, max};
use std::borrow::Cow;

use bytemuck::{Pod, Zeroable, cast_ref, cast_slice};

use wgpu::util::DeviceExt as _;
use wgpu::CommandEncoder;
use wgpu::{IndexFormat, RenderPass};
use wgpu::{ShaderModuleDescriptor, ShaderSource};
use wgpu::{PipelineLayoutDescriptor, PipelineCompilationOptions};
use wgpu::{ComputePipeline, ComputePipelineDescriptor};
use wgpu::{Buffer, BufferDescriptor, util::BufferInitDescriptor, BufferUsages};
use wgpu::{BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntry, ShaderStages};
use wgpu::{BindGroup, BindGroupDescriptor, BindGroupEntry, BindingType, BufferBindingType};
use wgpu::{ComputePass, ComputePassDescriptor};

use crate::CommonWgpuObjects;

pub const MAX_INPUT_POINTS: u16 = 32; // WARNING: This should match shader value.
                                      // WARNING: This should be divisible by 2 (since it is packed into
                                      // vec4 in the shader)
                                      // (it seems that specialization constants can't set array length)
const WORKGROUP_SIZE: u32 = 256; // WARNING: This should match shader value.
                                 // If this is increased above 256, device limits should be checked.

pub type PointPosition = [f32; 2];

#[derive(Copy, Clone)]
pub struct SimulationConstants {
    pub input_force: f32,
    pub decay_factor: f32,
    pub target_radius: f32,
    pub force_falloff: f32,
}
impl Default for SimulationConstants {
    fn default() -> Self {
        Self {
            input_force: 0.0000003,
            decay_factor: 0.99987,
            target_radius: 0.2,
            force_falloff: 0.5,
        }
    }
}
impl SimulationConstants {
    fn into_constants_for_pipeline(&self) -> [(&str, f64); 4] {
        [
            ("INPUT_FORCE", self.input_force as f64),
            ("DECAY_FACTOR", self.decay_factor as f64),
            ("TARGET_RADIUS", self.target_radius as f64),
            ("FORCE_FALLOFF", self.force_falloff as f64),
        ]
    }
}

pub struct PointSimBuilder<'a> {
    co: &'a CommonWgpuObjects,
    update_rate: f64,
    initial_point_positions: &'a [PointPosition],
    constants: SimulationConstants,
}
impl<'a> PointSimBuilder<'a> {
    pub fn new(co: &'a CommonWgpuObjects) -> Self {
        Self {
            co,
            update_rate: 10000.0,
            initial_point_positions: &[],
            constants: SimulationConstants::default(),
        }
    }
    /// updates/second
    pub fn update_rate(mut self, rate: f64) -> Self {
        self.update_rate = rate; self
    }
    pub fn initial_point_positions(mut self, positions: &'a [PointPosition]) -> Self {
        self.initial_point_positions = positions; self
    }
    pub fn constants(mut self, constants: SimulationConstants) -> Self {
        self.constants = constants; self
    }
}

/// Actual point simulation
pub struct PointSim {
    update_rate: f64,

    simulation_pipelines: Vec<ComputePipeline>,
    simulation_uniform_buffer: Buffer,
    geometry_pipeline: ComputePipeline,

    slices: PointSimSlices,
}
impl From<PointSimBuilder<'_>> for PointSim {
    fn from(settings: PointSimBuilder) -> Self {
        Self::from(&settings)
    }
}
impl From<&PointSimBuilder<'_>> for PointSim {
    fn from(b: &PointSimBuilder) -> Self {

        let (
            simulation_bind_group_layout,
            simulation_pipelines
        ) = create_simulation_pipelines(&b.co, &b.constants.into_constants_for_pipeline());

        let simulation_uniform_buffer = SimulationUniform::create_buffer(&b.co);
        simulation_uniform_buffer.as_entire_binding();

        let (geometry_bind_group_layout, geometry_pipeline) = create_geometry_pipeline(&b.co);

        let slice_binding_info = PointSimSliceBindingInfo {
            simulation_uniform_buffer: &simulation_uniform_buffer,
            simulation_bind_group_layout: &simulation_bind_group_layout,
            geometry_bind_group_layout: &geometry_bind_group_layout,
        };
        let slices = PointSimSlices::new(&b.co, slice_binding_info, b.initial_point_positions);

        Self {
            update_rate: b.update_rate,

            simulation_pipelines,
            simulation_uniform_buffer,
            geometry_pipeline,

            slices,
        }
    }
}
impl PointSim {
    pub fn record_update<'a>(
        &self,
        co: &CommonWgpuObjects,
        cmd: &mut CommandEncoder,
        dt: f64,
        input_positions: &[PointPosition],
    ) {
        let mut uniform_data = SimulationUniform {
            update_count: (self.update_rate * dt) as u32,
            _padding0: 0,
            _padding1: 0,
            _padding2: 0,
            input_positions: [[0.0, 0.0]; MAX_INPUT_POINTS as usize],
        };
        let mut input_count = 0usize;
        for (i, input_position) in input_positions.iter().enumerate() {
            if input_count+1 < MAX_INPUT_POINTS.into() {
                input_count += 1;
                uniform_data.input_positions[i] = input_position.clone();
            }
        }
        uniform_data.write_buffer(co, &self.simulation_uniform_buffer, input_count, 0);

        let compute_pass_descriptor = ComputePassDescriptor {
            label: None, timestamp_writes: None,
        };

        // this pass updates the front buffers and the vertex buffer according to the back buffers
        { let mut compute_pass = cmd.begin_compute_pass(&compute_pass_descriptor);
            // use the pipeline that corresponds to this number of inputs
            compute_pass.set_pipeline(&self.simulation_pipelines[input_count]);
            self.slices.record_simulation_update(&mut compute_pass);
            compute_pass.set_pipeline(&self.geometry_pipeline);
            self.slices.record_geometry_generation(&mut compute_pass);
        }
    }
    pub fn vertex_buffer_binders_iter(&self) -> impl Iterator<Item=impl Fn(&mut RenderPass, u32) -> u32> {
        self.slices.vertex_buffer_binders_iter()
    }
    pub fn bind_index_buffer(&self, render_pass: &mut RenderPass) {
        self.slices.bind_index_buffer(render_pass);
    }
    pub fn swap_buffers(&mut self) {
        self.slices.swap_buffers();
    }
}

/// Break up simulation into "slices" if the device limits require
struct PointSimSlices {
    slices: Vec<PointSimSlice>,
    index_buffer: Buffer,
}
impl PointSimSlices {
    fn new(co: &CommonWgpuObjects, binding_info: PointSimSliceBindingInfo, initial_point_positions: &[PointPosition]) -> Self {
        let limits = co.device.limits();

        // calculate the largest number of points a slice can hold according to the device limits
        let max_points_per_slice: usize = [
            // can do WORKGROUP_SIZE points per workgroup
            limits.max_compute_workgroups_per_dimension as usize * WORKGROUP_SIZE as usize,
            // vertex buffer is also used as a storage buffer
            // 4 verts per point
            limits.max_storage_buffer_binding_size as usize / 4 / size_of::<PointPosition>(),
            // vertex buffer is a buffer
            limits.max_buffer_size as usize / 4 / size_of::<PointPosition>(),

            // the simulation storage buffers are smaller than the vertex buffer
            // the index buffer is smaller than the vertex buffer
        ].into_iter().min().unwrap();

        let point_count = initial_point_positions.len();

        let slice_count = (point_count + max_points_per_slice - 1) / max_points_per_slice;

        // largest point count among slices
        let mut largest_point_count = 0;
        // create slices
        let slices = (0..slice_count).into_iter().map(|i| {
            let initial_positions_slice = &initial_point_positions[
                i * max_points_per_slice .. min((i+1) * max_points_per_slice, point_count)
            ];
            largest_point_count = max(largest_point_count, initial_positions_slice.len());
            PointSimSlice::new(co, binding_info, initial_positions_slice)
        }).collect();

        // fill index buffer with enough indices to work for any slice
        // this way it can just be shared across all of them
        let index_buffer = {
            let mut indices = Vec::with_capacity(largest_point_count.try_into().unwrap());
            for i in 0..u32::try_from(largest_point_count).unwrap() {
                // draw each point 4 times, then do primitive restart
                indices.push([
                    4*i+0,
                    4*i+1,
                    4*i+2,
                    4*i+3,
                    0xffff_ffff,
                ]);
            }
            co.device.create_buffer_init(&BufferInitDescriptor {
                label: None,
                contents: cast_slice(&indices),
                usage: BufferUsages::INDEX,
            })
        };

        Self {
            slices,
            index_buffer,
        }
    }
    fn record_simulation_update(&self, compute_pass: &mut ComputePass) {
        for slice in &self.slices {
            slice.record_simulation_update(compute_pass);
        }
    }
    fn record_geometry_generation(&self, compute_pass: &mut ComputePass) {
        for slice in &self.slices {
            slice.record_geometry_generation(compute_pass);
        }
    }
    // returns iterator of functions that bind the vertex & index buffers
    // each function returns the number of indices to draw
    fn vertex_buffer_binders_iter(&self) -> impl Iterator<Item=impl Fn(&mut RenderPass, u32) -> u32> {
        self.slices.iter().map(|slice| {
            |render_pass: &mut RenderPass, slot: u32| {
                slice.bind_vertex_buffer(render_pass, slot);
                // 4 corners, 1 triangle strip reset
                5 * slice.point_count
            }
        })
    }
    fn bind_index_buffer(&self, render_pass: &mut RenderPass) {
        render_pass.set_index_buffer(self.index_buffer.slice(..), IndexFormat::Uint32);
    }
    fn swap_buffers(&mut self) {
        for slice in &mut self.slices {
            slice.swap_buffers();
        }
    }
}

#[derive(Copy, Clone)]
struct PointSimSliceBindingInfo<'a> {
    simulation_uniform_buffer: &'a Buffer,
    simulation_bind_group_layout: &'a BindGroupLayout,
    geometry_bind_group_layout: &'a BindGroupLayout,
}

/// Contains everything that may need to be duplicated due to device limitations
struct PointSimSlice {
    point_count: u32,

    vertex_buffer: Buffer,

    simulation_bind_group_back: BindGroup,
    simulation_bind_group: BindGroup,
    geometry_bind_group_back: BindGroup,
    geometry_bind_group: BindGroup,
}
impl PointSimSlice {
    fn new(
        co: &CommonWgpuObjects,
        binding_info: PointSimSliceBindingInfo,
        initial_point_positions: &[PointPosition],
    ) -> Self {
        let point_count: u32 = initial_point_positions.len().try_into().unwrap();

        let velocity_buffer = co.device.create_buffer(&BufferDescriptor {
            label: None,
            // data is zero-initialized by default
            size: size_of::<PointPosition>() as u64 * u64::from(point_count),
            usage: BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let velocity_buffer_back = co.device.create_buffer(&BufferDescriptor {
            label: None,
            // data is zero-initialized by default
            size: size_of::<PointPosition>() as u64 * u64::from(point_count),
            usage: BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let position_buffer = co.device.create_buffer(&BufferDescriptor {
            label: None,
            size: size_of::<PointPosition>() as u64 * initial_point_positions.len() as u64,
            usage: BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        // initialize back buffer with initial positions
        // the first simulation update updates the front buffer according to the back
        // the first mesh generation call uses the back buffer
        // ^ this way they can be parallel since the back buffer is read only for both
        let position_buffer_back = co.device.create_buffer_init(&BufferInitDescriptor {
            label: None,
            contents: cast_slice(initial_point_positions),
            usage: BufferUsages::STORAGE,
        });
        let vertex_buffer = co.device.create_buffer(&BufferDescriptor {
            label: None,
            // 4 verts per point
            size: 4 * size_of::<PointPosition>() as u64 * u64::from(point_count),
            usage: BufferUsages::STORAGE | BufferUsages::VERTEX,
            mapped_at_creation: false,
        });

        let simulation_bind_group = co.device.create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &binding_info.simulation_bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: binding_info.simulation_uniform_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: position_buffer_back.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: velocity_buffer_back.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: position_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: velocity_buffer.as_entire_binding(),
                },
            ],
        });
        let simulation_bind_group_back = co.device.create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &binding_info.simulation_bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: binding_info.simulation_uniform_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: position_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: velocity_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: position_buffer_back.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: velocity_buffer_back.as_entire_binding(),
                },
            ],
        });
        // generates geometry from the back buffer
        let geometry_bind_group = co.device.create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &binding_info.geometry_bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: position_buffer_back.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: vertex_buffer.as_entire_binding(),
                },
            ],
        });
        let geometry_bind_group_back = co.device.create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &binding_info.geometry_bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: position_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: vertex_buffer.as_entire_binding(),
                },
            ],
        });

        Self {
            point_count,

            vertex_buffer,

            simulation_bind_group,
            simulation_bind_group_back,
            geometry_bind_group,
            geometry_bind_group_back,
        }
    }
    fn record_simulation_update(&self, compute_pass: &mut ComputePass) {
        // update "front" buffers based on "back" buffers
        compute_pass.set_bind_group(0, &self.simulation_bind_group, &[]);
        compute_pass.dispatch_workgroups((self.point_count+WORKGROUP_SIZE-1) / WORKGROUP_SIZE, 1, 1);
    }
    fn record_geometry_generation(&self, compute_pass: &mut ComputePass) {
        compute_pass.set_bind_group(0, &self.geometry_bind_group, &[]);
        compute_pass.dispatch_workgroups((self.point_count+WORKGROUP_SIZE-1) / WORKGROUP_SIZE, 1, 1);
    }
    fn bind_vertex_buffer(&self, render_pass: &mut RenderPass, slot: u32) {
        render_pass.set_vertex_buffer(slot, self.vertex_buffer.slice(..));
    }
    fn swap_buffers(&mut self) {
        // swap bind groups
        swap(&mut self.simulation_bind_group, &mut self.simulation_bind_group_back);
        swap(&mut self.geometry_bind_group, &mut self.geometry_bind_group_back);
    }
}

#[repr(C)]
#[derive(Copy, Clone, Zeroable, Pod)]
struct SimulationUniform {
    update_count: u32,
    _padding0: u32,
    _padding1: u32,
    _padding2: u32,
    // Memory layout here:
    // [
    //  vec2(p0.x, p0.y),
    //  vec2(p1.x, p1.y),
    //  vec2(p2.x, p2.y),
    //  vec2(p3.x, p3.y),
    //  ...
    // ]
    // Memory layout in shader:
    // [
    //  vec4(p0.x, p0.y, p1.x, p1.y)
    //  vec4(p2.x, p2.y, p3.x, p3.y)
    //  ...
    // ]
    //
    // I can't find it in the spec, but it seems like arrays in uniforms require a stride of 16
    input_positions: [PointPosition; MAX_INPUT_POINTS as usize],
}
impl SimulationUniform {
    fn create_buffer(co: &CommonWgpuObjects) -> Buffer {
        co.device.create_buffer(&BufferDescriptor {
            label: None,
            size: size_of::<Self>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }
    /// uploads relevant part of the struct to a uniform buffer
    /// (ex. if there are only 3 input points, only need to upload those 3 and not the unused ones)
    fn write_buffer(&self, co: &CommonWgpuObjects, buffer: &Buffer, input_count: usize, offset: u64) {
        let all_bytes: &[u8; size_of::<Self>()] = cast_ref(self);
        let byte_count: usize =
            // start of `input_positions` array
            offset_of!(Self, input_positions)
            // size of point
            + size_of::<PointPosition>()
            // times number of points
            * input_count;
        let bytes_to_upload = &all_bytes[0..byte_count];
        co.queue.write_buffer(buffer, offset, bytes_to_upload);
    }
}

fn create_simulation_pipelines(co: &CommonWgpuObjects, constants: &[(&str, f64)]) -> (BindGroupLayout, Vec<ComputePipeline>) {
    let bind_group_layout = co.device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            BindGroupLayoutEntry { // uniform buffer
                binding: 0,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry { // point position buffer read
                binding: 1,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry { // velocity buffer read
                binding: 2,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry { // point position buffer write
                binding: 3,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry { // velocity buffer write
                binding: 4,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            ]
    });

    let pipeline_layout = co.device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });

    let compute_simulation_shader_module = co.device.create_shader_module(ShaderModuleDescriptor {
        label: None,
        source: ShaderSource::Wgsl(Cow::Borrowed(include_str!("compute_simulation.wgsl"))),
    });

    // create a pipeline for each possible number of inputs
    let compute_pipelines = (0..MAX_INPUT_POINTS).map(|i| {
        let mut constants = Vec::from(constants);
        constants.push(("INPUT_COUNT", i as f64));
        co.device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            module: &compute_simulation_shader_module,
            entry_point: Some("main"),
            compilation_options: PipelineCompilationOptions {
                constants: &constants,
                zero_initialize_workgroup_memory: false,
            },
            cache: None,
        })
    }).collect();

    (bind_group_layout, compute_pipelines)
}

fn create_geometry_pipeline(co: &CommonWgpuObjects) -> (BindGroupLayout, ComputePipeline) {
    let bind_group_layout = co.device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            BindGroupLayoutEntry { // point position buffer
                binding: 0,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry { // vertex position buffer
                binding: 1,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });

    let pipeline_layout = co.device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });

    let compute_geometry_shader_module = co.device.create_shader_module(ShaderModuleDescriptor {
        label: None,
        source: ShaderSource::Wgsl(Cow::Borrowed(include_str!("compute_geometry.wgsl"))),
    });

    let compute_pipeline = co.device.create_compute_pipeline(&ComputePipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        module: &compute_geometry_shader_module,
        entry_point: Some("main"),
        compilation_options: PipelineCompilationOptions {
            constants: &[],
            zero_initialize_workgroup_memory: false,
        },
        cache: None,
    });

    (bind_group_layout, compute_pipeline)
}
