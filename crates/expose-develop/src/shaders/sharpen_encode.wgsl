// Sharpening pass 1: encoded luminance (version 1).
//
// E = srgb_encode(max(Y, 0)), with Y the Rec. 2020 luminance. The CPU
// reference is `encode_pass` in `sharpen.rs`. The result is in the red
// channel. Green and blue are 0, alpha is 1.
//
// Input 0 is the developed image. The output has the same size.
// Kernel contract: see `expose-gpu` `kernel.rs`.

struct Params {
    // xyz: luminance weights. w: unused.
    luma: vec4<f32>,
}

@group(0) @binding(0) var input_image: texture_2d<f32>;
@group(0) @binding(1) var output_image: texture_storage_2d<OUTPUT_FORMAT, write>;
@group(0) @binding(2) var<uniform> params: Params;

// sRGB encode for values >= 0. Same as `TransferFunction::Srgb`.
fn srgb_encode(a: f32) -> f32 {
    if (a <= 0.0031308) {
        return a * 12.92;
    }
    return 1.055 * pow(a, 1.0 / 2.4) - 0.055;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_image);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let pixel = vec2<i32>(id.xy);
    let c = textureLoad(input_image, pixel, 0);
    let y = params.luma.x * c.r + params.luma.y * c.g + params.luma.z * c.b;
    textureStore(output_image, pixel, vec4<f32>(srgb_encode(max(y, 0.0)), 0.0, 0.0, 1.0));
}
