// Applies an adjustment layer to the backdrop.
//
// `params.flags.w` is the adjustment kind. `params.adj` holds its data:
// - 0, brightness/contrast: adj[0] = (brightness, slope k, threshold flag, 0).
// - 1, levels: adj[0..5] = input black, input white, gamma, output black,
//   output white. Each is (red, green, blue, master).
// - 2, curves: `aux` is a table with N texels in one row. Texel i holds
//   the red, green, and blue curve values at x = i / (N - 1) (the color
//   curve, then the master curve).
// - 3, hue/saturation: adj[0..3] = hue matrix rows (xyz),
//   adj[3] = (saturation, lightness, 0, 0).

const ADJUST_BRIGHTNESS_CONTRAST: u32 = 0u;
const ADJUST_LEVELS: u32 = 1u;
const ADJUST_CURVES: u32 = 2u;

// Each formula applies on 0..=1, and the result keeps the excess outside
// that range (`extend` in `adjust.rs`): `f(clamp(v)) + (v - clamp(v))`.

fn brightness_contrast(x: f32) -> f32 {
    let b = params.adj[0].x;
    var v: f32;
    if (b >= 0.0) {
        v = x + (1.0 - x) * b;
    } else {
        v = x * (1.0 + b);
    }
    var r: f32;
    if (params.adj[0].z != 0.0) {
        if (v > 0.5) {
            r = 1.0;
        } else if (v < 0.5) {
            r = 0.0;
        } else {
            r = 0.5;
        }
    } else {
        r = (v - 0.5) * params.adj[0].y + 0.5;
    }
    return clamp(r, 0.0, 1.0);
}

// `LevelsChannel::apply` for channel `i` (0..3 = red, green, blue, master).
fn levels_channel(v: f32, i: u32) -> f32 {
    let ib = params.adj[0][i];
    let iw = params.adj[1][i];
    let g = params.adj[2][i];
    let ob = params.adj[3][i];
    let ow = params.adj[4][i];
    var t = clamp((v - ib) / (iw - ib), 0.0, 1.0);
    // pow(0, y) is not defined in all shader languages. The CPU result is 0.
    if (t > 0.0) {
        t = pow(t, 1.0 / g);
    }
    return ob + t * (ow - ob);
}

// The curve table at `x` in 0..=1 for channel `i`, with linear
// interpolation between the texels.
fn curve(x: f32, i: u32) -> f32 {
    let n = i32(textureDimensions(aux).x);
    let t = x * f32(n - 1);
    let i0 = clamp(i32(floor(t)), 0, n - 2);
    let f = t - f32(i0);
    let a = textureLoad(aux, vec2<i32>(i0, 0), 0)[i];
    let b = textureLoad(aux, vec2<i32>(i0 + 1, 0), 0)[i];
    return a + (b - a) * f;
}

fn lightness(x: f32) -> f32 {
    let lt = params.adj[3].y;
    if (lt >= 0.0) {
        return x + (1.0 - x) * lt;
    }
    return x * (1.0 + lt);
}

// `PreparedAdjustment::apply` for a straight linear color.
fn apply_adjustment(linear: vec3<f32>) -> vec3<f32> {
    let e = srgb_encode3(linear);
    let ce = clamp(e, vec3<f32>(0.0), vec3<f32>(1.0));
    var o: vec3<f32>;
    switch params.flags.w {
        case ADJUST_BRIGHTNESS_CONTRAST: {
            o = vec3<f32>(
                brightness_contrast(ce.x),
                brightness_contrast(ce.y),
                brightness_contrast(ce.z),
            ) + (e - ce);
        }
        case ADJUST_LEVELS: {
            o = vec3<f32>(
                levels_channel(levels_channel(ce.x, 0u), 3u),
                levels_channel(levels_channel(ce.y, 1u), 3u),
                levels_channel(levels_channel(ce.z, 2u), 3u),
            ) + (e - ce);
        }
        case ADJUST_CURVES: {
            o = vec3<f32>(curve(ce.x, 0u), curve(ce.y, 1u), curve(ce.z, 2u)) + (e - ce);
        }
        default: {
            // Hue/saturation.
            let r = vec3<f32>(
                params.adj[0].x * e.x + params.adj[0].y * e.y + params.adj[0].z * e.z,
                params.adj[1].x * e.x + params.adj[1].y * e.y + params.adj[1].z * e.z,
                params.adj[2].x * e.x + params.adj[2].y * e.y + params.adj[2].z * e.z,
            );
            let l = luminance(r);
            let s = l + (r - l) * (1.0 + params.adj[3].x);
            let cs = clamp(s, vec3<f32>(0.0), vec3<f32>(1.0));
            o = vec3<f32>(lightness(cs.x), lightness(cs.y), lightness(cs.z)) + (s - cs);
        }
    }
    return srgb_decode3(o);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(output);
    if (gid.x >= size.x || gid.y >= size.y) {
        return;
    }
    let p = vec2<i32>(gid.xy);
    let c = canvas_pos(p);
    let dst = textureLoad(backdrop, p, 0);
    let ab = dst.a;
    if (ab <= 0.0) {
        textureStore(output, p, dst);
        return;
    }
    let k = dissolve(min(coverage(c.x, c.y), 1.0), c.x, c.y);
    if (k <= 0.0) {
        textureStore(output, p, dst);
        return;
    }
    let cb_lin = unpremultiply(dst);
    let adjusted = apply_adjustment(cb_lin);
    let cb = to_space(cb_lin);
    let a = to_space(adjusted);
    let b = blend(params.flags.x, cb, a);
    let mixed = cb + k * (b - cb);
    textureStore(output, p, vec4<f32>(from_space(mixed) * ab, ab));
}
