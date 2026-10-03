// Fused develop kernel (version 1).
//
// One pass does all basic develop stages. The CPU reference in
// `pipeline.rs` (`Pipeline::apply_pixel`) does the same arithmetic. Keep
// the two in agreement.
//
// 1. Matrix: white balance and exposure, fused into one 3x3 matrix.
// 2. Tone: highlights, shadows, whites, blacks, and contrast. One LUT of
//    log2(luminance ratio) over e = log2(Y / mid gray). RGB is multiplied
//    by 2^delta, so the channel ratios do not change.
// 3. Tone curve: sRGB encode, master curve, channel curve, sRGB decode.
//    Linear extension below 0 and above 1.
//
// `params.flags.x` has one bit for each active stage. The kernel skips the
// other stages. Alpha is copied.
//
// Kernel contract: see `expose-gpu` `kernel.rs`.

const FLAG_MATRIX: u32 = 1u;
const FLAG_TONE: u32 = 2u;
// Bit (FLAG_CURVE_MASTER << c) is curve c: 0 master, 1 red, 2 green, 3 blue.
const FLAG_CURVE_MASTER: u32 = 4u;

const TONE_LUT_SIZE: u32 = 2048u;
const CURVE_LUT_SIZE: u32 = 1024u;

struct Params {
    // Rows of the RGB matrix (w is unused).
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    // x: stage flags. y, z, w: unused.
    flags: vec4<u32>,
    // x: lowest e of the LUT, y: LUT samples per EV, z: luminance floor,
    // w: mid gray.
    tone: vec4<f32>,
    // xyz: luminance weights. w: unused.
    luma: vec4<f32>,
    // For each curve: (y at 0, slope below 0, y at 1, slope above 1).
    curve_ends: array<vec4<f32>, 4>,
    // TONE_LUT_SIZE values, four in each vec4.
    tone_lut: array<vec4<f32>, 512>,
    // Sample i: values of the (master, red, green, blue) curves.
    curve_lut: array<vec4<f32>, 1024>,
}

@group(0) @binding(0) var input_image: texture_2d<f32>;
@group(0) @binding(1) var output_image: texture_storage_2d<OUTPUT_FORMAT, write>;
@group(0) @binding(2) var<uniform> params: Params;

fn tone_lut_at(i: u32) -> f32 {
    return params.tone_lut[i >> 2u][i & 3u];
}

// log2 of the luminance ratio. Same as `tone::lut_delta`.
fn tone_delta(y: f32) -> f32 {
    let e = log2(max(y, params.tone.z) / params.tone.w);
    let t = clamp((e - params.tone.x) * params.tone.y, 0.0, f32(TONE_LUT_SIZE - 1u));
    let i = min(u32(floor(t)), TONE_LUT_SIZE - 2u);
    let f = t - f32(i);
    let a = tone_lut_at(i);
    let b = tone_lut_at(i + 1u);
    return a + f * (b - a);
}

// Curve c at encoded value x. Same as `CurveTables::apply`.
fn curve(c: u32, x: f32) -> f32 {
    let ends = params.curve_ends[c];
    if (x < 0.0) {
        return ends.x + ends.y * x;
    }
    if (x > 1.0) {
        return ends.z + ends.w * (x - 1.0);
    }
    let t = x * f32(CURVE_LUT_SIZE - 1u);
    let i = min(u32(floor(t)), CURVE_LUT_SIZE - 2u);
    let f = t - f32(i);
    let a = params.curve_lut[i][c];
    let b = params.curve_lut[i + 1u][c];
    return a + f * (b - a);
}

// sRGB transfer function with the odd extension for negative values. Same
// as `TransferFunction::Srgb` in `expose-core`.
fn srgb_encode(v: f32) -> f32 {
    let a = abs(v);
    var e: f32;
    if (a <= 0.0031308) {
        e = a * 12.92;
    } else {
        e = 1.055 * pow(a, 1.0 / 2.4) - 0.055;
    }
    return select(e, -e, v < 0.0);
}

fn srgb_decode(v: f32) -> f32 {
    let a = abs(v);
    var l: f32;
    if (a <= 0.04045) {
        l = a / 12.92;
    } else {
        l = pow((a + 0.055) / 1.055, 2.4);
    }
    return select(l, -l, v < 0.0);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_image);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let pixel = vec2<i32>(id.xy);
    let color = textureLoad(input_image, pixel, 0);
    var rgb = color.rgb;
    let flags = params.flags.x;

    if ((flags & FLAG_MATRIX) != 0u) {
        rgb = vec3<f32>(
            params.m0.x * rgb.x + params.m0.y * rgb.y + params.m0.z * rgb.z,
            params.m1.x * rgb.x + params.m1.y * rgb.y + params.m1.z * rgb.z,
            params.m2.x * rgb.x + params.m2.y * rgb.y + params.m2.z * rgb.z,
        );
    }

    if ((flags & FLAG_TONE) != 0u) {
        let y = params.luma.x * rgb.x + params.luma.y * rgb.y + params.luma.z * rgb.z;
        rgb = rgb * exp2(tone_delta(y));
    }

    if ((flags & (FLAG_CURVE_MASTER * 15u)) != 0u) {
        for (var ch = 0u; ch < 3u; ch = ch + 1u) {
            var x = srgb_encode(rgb[ch]);
            if ((flags & FLAG_CURVE_MASTER) != 0u) {
                x = curve(0u, x);
            }
            if ((flags & (FLAG_CURVE_MASTER << (ch + 1u))) != 0u) {
                x = curve(ch + 1u, x);
            }
            rgb[ch] = srgb_decode(x);
        }
    }

    textureStore(output_image, pixel, vec4<f32>(rgb, color.a));
}
