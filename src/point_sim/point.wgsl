override POINT_SIZE: f32;
override CORNER_COLOR_0R: f32;
override CORNER_COLOR_0G: f32;
override CORNER_COLOR_0B: f32;
override CORNER_COLOR_0A: f32;
override CORNER_COLOR_1R: f32;
override CORNER_COLOR_1G: f32;
override CORNER_COLOR_1B: f32;
override CORNER_COLOR_1A: f32;
override CORNER_COLOR_2R: f32;
override CORNER_COLOR_2G: f32;
override CORNER_COLOR_2B: f32;
override CORNER_COLOR_2A: f32;
override CORNER_COLOR_3R: f32;
override CORNER_COLOR_3G: f32;
override CORNER_COLOR_3B: f32;
override CORNER_COLOR_3A: f32;

struct Uniform {
    view_scaling: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> uniform_buffer: Uniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @builtin(vertex_index) vertex_index: u32,
};
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(linear) color: vec4<f32>,
    @location(1) @interpolate(linear) uv: vec2<f32>,
};

@vertex
fn vertex_main(in: VertexInput) -> VertexOutput {
    let offsets = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0,  1.0),
    );
    let colors = array<vec4<f32>, 4>(
        vec4<f32>(CORNER_COLOR_0R, CORNER_COLOR_0G, CORNER_COLOR_0B, CORNER_COLOR_0A),
        vec4<f32>(CORNER_COLOR_1R, CORNER_COLOR_1G, CORNER_COLOR_1B, CORNER_COLOR_1A),
        vec4<f32>(CORNER_COLOR_2R, CORNER_COLOR_2G, CORNER_COLOR_2B, CORNER_COLOR_2A),
        vec4<f32>(CORNER_COLOR_3R, CORNER_COLOR_3G, CORNER_COLOR_3B, CORNER_COLOR_3A),
    );

    let i: u32 = in.vertex_index % 4;
    let position = uniform_buffer.view_scaling * (
        in.position + POINT_SIZE * offsets[i]
    );
    let uv = offsets[i];

    return VertexOutput (
        vec4<f32>(position, 0.0, 1.0),
        colors[i],
        uv,
    );
}

override POINTS_CIRCULAR: bool;

struct FragmentInput {
    @location(0) @interpolate(linear) color: vec4<f32>,
    @location(1) @interpolate(linear) uv: vec2<f32>,
};

@fragment
fn fragment_main(in: FragmentInput) -> @location(0) vec4<f32> {
    if POINTS_CIRCULAR {
        if (dot(in.uv, in.uv) > 1.0) {
            discard;
        }
    }
    return in.color;
}
