// Sharpening pass 2: Gaussian blur along x (version 1).
//
// The CPU reference is `blur_horizontal` in `sharpen.rs`. The kernel
// blurs the red channel. Pixels outside the image take the value of the
// nearest edge pixel (clamp to edge). The weight order and the sum order
// are the same as on the CPU.
//
// Input 0 is the encoded luminance. The output has the same size.
// Kernel contract: see `expose-gpu` `kernel.rs`.

struct Params {
    // Weights for the offsets 0..=15, four in each vec4. Center first.
    weights: array<vec4<f32>, 4>,
    // x: half-width (1..=15). y, z, w: unused.
    info: vec4<u32>,
}

@group(0) @binding(0) var input_image: texture_2d<f32>;
@group(0) @binding(1) var output_image: texture_storage_2d<OUTPUT_FORMAT, write>;
@group(0) @binding(2) var<uniform> params: Params;

fn weight(i: u32) -> f32 {
    return params.weights[i >> 2u][i & 3u];
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_image);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let x = i32(id.x);
    let y = i32(id.y);
    let last = i32(size.x) - 1;
    var acc = weight(0u) * textureLoad(input_image, vec2<i32>(x, y), 0).r;
    let half_width = min(params.info.x, 15u);
    for (var i = 1u; i <= half_width; i = i + 1u) {
        let left = max(x - i32(i), 0);
        let right = min(x + i32(i), last);
        let a = textureLoad(input_image, vec2<i32>(left, y), 0).r;
        let b = textureLoad(input_image, vec2<i32>(right, y), 0).r;
        acc = acc + weight(i) * (a + b);
    }
    textureStore(output_image, vec2<i32>(id.xy), vec4<f32>(acc, 0.0, 0.0, 1.0));
}
