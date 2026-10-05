// Sharpening pass 4: detail, halo suppression, edge mask, and combine
// (version 1).
//
// The CPU reference is `Sharpener::combine_pass` in `sharpen.rs`. Keep the
// two in agreement.
//
// Inputs: 0 = developed image, 1 (binding 3) = encoded luminance E (red),
// 2 (binding 4) = blurred luminance B (red). All three and the output have
// the same size.
// Kernel contract: see `expose-gpu` `kernel.rs`.

const FLAG_MASK: u32 = 1u;

struct Params {
    // x: gain, y: detail (0..1), z: low mask threshold, w: high mask
    // threshold.
    factors: vec4<f32>,
    // x: halo soft limit K, y: luminance epsilon. z, w: unused.
    limits: vec4<f32>,
    // xyz: luminance weights. w: unused.
    luma: vec4<f32>,
    // x: flags. y, z, w: unused.
    flags: vec4<u32>,
}

@group(0) @binding(0) var input_image: texture_2d<f32>;
@group(0) @binding(1) var output_image: texture_storage_2d<OUTPUT_FORMAT, write>;
@group(0) @binding(2) var<uniform> params: Params;
@group(0) @binding(3) var encoded_image: texture_2d<f32>;
@group(0) @binding(4) var blurred_image: texture_2d<f32>;

fn blurred(x: i32, y: i32) -> f32 {
    return textureLoad(blurred_image, vec2<i32>(x, y), 0).r;
}

// Gradient magnitude of B, central differences, clamp to edge. Same as
// `gradient`.
fn gradient(x: i32, y: i32, size: vec2<u32>) -> f32 {
    let left = max(x - 1, 0);
    let right = min(x + 1, i32(size.x) - 1);
    let up = max(y - 1, 0);
    let down = min(y + 1, i32(size.y) - 1);
    let gx = 0.5 * (blurred(right, y) - blurred(left, y));
    let gy = 0.5 * (blurred(x, down) - blurred(x, up));
    return sqrt(gx * gx + gy * gy);
}

// Hermite smoothstep from lo to hi. Same as `smoothstep` in `sharpen.rs`.
fn smooth_ramp(lo: f32, hi: f32, value: f32) -> f32 {
    let t = clamp((value - lo) / (hi - lo), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// sRGB decode for values >= 0. Same as `TransferFunction::Srgb`.
fn srgb_decode(a: f32) -> f32 {
    if (a <= 0.04045) {
        return a / 12.92;
    }
    return pow((a + 0.055) / 1.055, 2.4);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_image);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let pixel = vec2<i32>(id.xy);
    let color = textureLoad(input_image, pixel, 0);
    let lum = params.luma.x * color.r + params.luma.y * color.g + params.luma.z * color.b;
    // `!(lum > epsilon)` is also true for NaN, as on the CPU.
    if (!(lum > params.limits.y)) {
        textureStore(output_image, pixel, color);
        return;
    }
    let e = textureLoad(encoded_image, pixel, 0).r;
    let diff = e - blurred(pixel.x, pixel.y);
    let soft = diff / (1.0 + abs(diff) / params.limits.x);
    let detail = params.factors.y;
    let limited = detail * diff + (1.0 - detail) * soft;
    var mask = 1.0;
    if ((params.flags.x & FLAG_MASK) != 0u) {
        mask = smooth_ramp(params.factors.z, params.factors.w, gradient(pixel.x, pixel.y, size));
    }
    let sharpened = e + params.factors.x * mask * limited;
    let ratio = srgb_decode(max(sharpened, 0.0)) / lum;
    textureStore(output_image, pixel, vec4<f32>(color.rgb * ratio, color.a));
}
