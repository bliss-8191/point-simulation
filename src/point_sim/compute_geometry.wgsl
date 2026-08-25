@group(0) @binding(0)
var<storage, read> input: array<vec2<f32>>;

@group(0) @binding(1)
var<storage, read_write> output: array<array<vec2<f32>, 4>>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let i = global_id.x;

    if i > arrayLength(&input) {
        return;
    }

    // just write the same position 4 times
    // the vertex shader will add offsets to make rectangles
    let point_pos = input[i];
    let positions = array<vec2<f32>, 4>(
        point_pos,
        point_pos,
        point_pos,
        point_pos,
    );

    output[i] = positions;
}
