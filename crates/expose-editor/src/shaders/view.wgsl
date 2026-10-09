// Renders a view (zoom and pan) of the composite from one level of the
// view pyramid. The input is premultiplied. The output is straight alpha.
//
// Level coordinates: texel `i` of the level covers `i..i + 1`. The output
// pixel `o` covers `origin + o * step .. origin + (o + 1) * step`, with
// `origin = base + frac` (an integer part and a fraction, for precision).
//
// - Mode 0 (nearest, zoom 1 or more): the texel at the center of the
//   output pixel.
// - Mode 1 (area, zoom less than 1): the area mean of the texels under the
//   output pixel. The caller selects the level so that `step` is less than
//   2 texels (except on the last level, which has 1 x 1 texel). Thus at
//   most 3 x 3 texels are under one output pixel.
//
// Texels outside the level are transparent.

struct ViewParams {
    // Integer part of the origin (x, y), level size (z, w).
    base: vec4<i32>,
    // Fraction of the origin (x, y), step in texels for each output pixel
    // (z), mode (w: 0 = nearest, 1 = area).
    frac: vec4<f32>,
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var output: texture_storage_2d<OUTPUT_FORMAT, write>;
@group(0) @binding(2) var<uniform> params: ViewParams;

// The texel at offset `i` from `params.base`. Transparent outside the level.
fn texel(i: vec2<i32>) -> vec4<f32> {
    let p = params.base.xy + i;
    if (p.x < 0 || p.y < 0 || p.x >= params.base.z || p.y >= params.base.w) {
        return vec4<f32>(0.0);
    }
    return textureLoad(src, p, 0);
}

// Premultiplied to straight alpha. Transparent black if alpha <= 0.
fn straight(p: vec4<f32>) -> vec4<f32> {
    if (p.a > 0.0) {
        return vec4<f32>(p.rgb / p.a, p.a);
    }
    return vec4<f32>(0.0);
}

// The weight of texel `t` (offset from the base) in the span `u0..u1`.
fn weight(t: i32, u0: f32, u1: f32) -> f32 {
    return min(u1, f32(t + 1)) - max(u0, f32(t));
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(output);
    if (gid.x >= size.x || gid.y >= size.y) {
        return;
    }
    let out_pos = vec2<i32>(gid.xy);
    let o = vec2<f32>(gid.xy);
    let step = params.frac.z;
    if (params.frac.w == 0.0) {
        let u = params.frac.xy + (o + 0.5) * step;
        textureStore(output, out_pos, straight(texel(vec2<i32>(floor(u)))));
        return;
    }
    let u0 = params.frac.xy + o * step;
    let u1 = u0 + step;
    // Start at the first texel inside the level: the texels before it are
    // transparent. This keeps the last level (1 x 1) correct for a span of
    // more than 3 texels.
    let first = max(vec2<i32>(floor(u0)), -params.base.xy);
    var sum = vec4<f32>(0.0);
    for (var j = 0; j < 3; j = j + 1) {
        let ty = first.y + j;
        let wy = weight(ty, u0.y, u1.y);
        if (wy <= 0.0) {
            break;
        }
        for (var i = 0; i < 3; i = i + 1) {
            let tx = first.x + i;
            let wx = weight(tx, u0.x, u1.x);
            if (wx <= 0.0) {
                break;
            }
            sum = sum + texel(vec2<i32>(tx, ty)) * (wx * wy);
        }
    }
    textureStore(output, out_pos, straight(sum / (step * step)));
}
