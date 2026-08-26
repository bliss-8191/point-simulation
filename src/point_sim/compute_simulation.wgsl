const MAX_INPUT_POINTS: u32 = 32; // should be divisible by 2
override INPUT_COUNT: u32;
override INPUT_FORCE: f32;
override DECAY_FACTOR: f32;
override TARGET_RADIUS: f32;
// set to FORCE_FALLOFF to 0 to disable
override FORCE_FALLOFF: f32;

struct Uniform {
    update_count: u32,
    // implicit 12 byte padding
    input_positions: array<vec4<f32>, MAX_INPUT_POINTS/2>,
};

@group(0) @binding(0)
var<uniform> uniform_buffer: Uniform;

@group(0) @binding(1)
var<storage, read> positions_read: array<vec2<f32>>;

@group(0) @binding(2)
var<storage, read> velocities_read: array<vec2<f32>>;

@group(0) @binding(3)
var<storage, read_write> positions_write: array<vec2<f32>>;

@group(0) @binding(4)
var<storage, read_write> velocities_write: array<vec2<f32>>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let i = global_id.x;
    if i > arrayLength(&positions_read) {
        return;
    }

    var p = positions_read[i];
    var v = velocities_read[i];

    // get input points in vec2 array for convenience
    var input_positions: array<vec2<f32>, MAX_INPUT_POINTS>;
    for (var i: u32 = 0; i < INPUT_COUNT; i += 2) {
        // first half of vec4 (xy)
        input_positions[i] = uniform_buffer.input_positions[i>>1].xy;

        // input array is packed tightly into vec4 array
        if i+1 < INPUT_COUNT {
            // second half of vec4 (zw)
            input_positions[i+1] = uniform_buffer.input_positions[i>>1].zw;
        }
    }

    // weight inputs based on distance (for multitouch)
    var input_weights: array<f32, MAX_INPUT_POINTS>;
    for (var i: u32 = 0; i < INPUT_COUNT; i += 1) {
        let dist = length(p - input_positions[i]);
        input_weights[i] = 1.0 / pow(dist, 5.0);
    }
    // normalize weights vector ("preserves" force")
    // this way inputs won't "add up"
    var sum: f32 = 0.0;
    for (var i: u32 = 0; i < INPUT_COUNT; i++) {
        sum += input_weights[i] * input_weights[i];
    }
    let sum_inv = 1.0 / sqrt(sum);
    for (var i: u32 = 0; i < INPUT_COUNT; i++) {
        input_weights[i] *= sum_inv;
    }

    // update N times based on uniform input
    // this keeps the simulation running at full speed while keeping delta time constant
    // this also avoids excessive reads+writes to point buffers
    for (var j: u32 = 0; j < uniform_buffer.update_count; j++) {
        var a = vec2<f32>(0.0);
        // accelerate for each input point
        for (var i: u32 = 0; i < INPUT_COUNT; i++) {
            let input_point = input_positions[i];

            // direction to the input position
            let dir = normalize(input_point - p);
            // distance to the input
            let dist = length(input_point - p);

            let force_falloff = INPUT_FORCE * exp(-FORCE_FALLOFF * dist);

            a += input_weights[i] * force_falloff * (input_point - p - TARGET_RADIUS*dir);
        }

        // move point
        p += v;
        // update velocity
        v += a;

        // decay velocity
        v *= DECAY_FACTOR;
    }

    positions_write[i] = p;
    velocities_write[i] = v;
}
