// Shared code of the editor compositor kernels.
//
// This code is a port of `blend.rs`, `adjust.rs`, and `composite.rs`. Keep
// the order of the float operations the same as in the CPU code, so that
// the results agree within float tolerance.
//
// Bindings (the same for all compositor kernels):
// - 0: backdrop (region size, premultiplied rgba32f)
// - 1: output (region size)
// - 2: parameters
// - 3: mask, 4 mask values in each texel (r, g, b, a = x % 4 = 0, 1, 2, 3)
// - 4: auxiliary input: the layer pixels (blend), the curve table
//      (adjust), or the pass-through result (pass-through mix)

struct Params {
    // Canvas position of the region (x, y), and its size (z, w).
    region: vec4<i32>,
    // Canvas position (x, y) and size (z, w) of the auxiliary image.
    source: vec4<i32>,
    // Canvas position (x, y) and size (z, w, in mask pixels) of the mask.
    mask: vec4<i32>,
    // Blend mode, blend space, mask flag (0 = no mask), adjustment kind.
    flags: vec4<u32>,
    // Coverage base (opacity * fill), mask fill value, unused, unused.
    coef: vec4<f32>,
    // Adjustment parameters (see `gpu.rs`).
    adj: array<vec4<f32>, 5>,
}

@group(0) @binding(0) var backdrop: texture_2d<f32>;
@group(0) @binding(1) var output: texture_storage_2d<OUTPUT_FORMAT, write>;
@group(0) @binding(2) var<uniform> params: Params;
@group(0) @binding(3) var mask_tex: texture_2d<f32>;
@group(0) @binding(4) var aux: texture_2d<f32>;

// Blend mode IDs: the index in `BlendMode::ALL`.
const MODE_NORMAL: u32 = 0u;
const MODE_DISSOLVE: u32 = 1u;
const MODE_DARKEN: u32 = 2u;
const MODE_MULTIPLY: u32 = 3u;
const MODE_COLOR_BURN: u32 = 4u;
const MODE_LINEAR_BURN: u32 = 5u;
const MODE_DARKER_COLOR: u32 = 6u;
const MODE_LIGHTEN: u32 = 7u;
const MODE_SCREEN: u32 = 8u;
const MODE_COLOR_DODGE: u32 = 9u;
const MODE_LINEAR_DODGE: u32 = 10u;
const MODE_LIGHTER_COLOR: u32 = 11u;
const MODE_OVERLAY: u32 = 12u;
const MODE_SOFT_LIGHT: u32 = 13u;
const MODE_HARD_LIGHT: u32 = 14u;
const MODE_VIVID_LIGHT: u32 = 15u;
const MODE_LINEAR_LIGHT: u32 = 16u;
const MODE_PIN_LIGHT: u32 = 17u;
const MODE_HARD_MIX: u32 = 18u;
const MODE_DIFFERENCE: u32 = 19u;
const MODE_EXCLUSION: u32 = 20u;
const MODE_SUBTRACT: u32 = 21u;
const MODE_DIVIDE: u32 = 22u;
const MODE_HUE: u32 = 23u;
const MODE_SATURATION: u32 = 24u;
const MODE_COLOR: u32 = 25u;
const MODE_LUMINOSITY: u32 = 26u;

const SPACE_LINEAR: u32 = 0u;

const DIVIDE_MIN_SOURCE: f32 = 1.0e-4;

// --- Color helpers ----------------------------------------------------------

fn luminance(c: vec3<f32>) -> f32 {
    return 0.2627 * c.x + 0.678 * c.y + 0.0593 * c.z;
}

// sRGB transfer curve. It continues above 1.0 and is odd for negative values.
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

fn srgb_encode3(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(srgb_encode(c.x), srgb_encode(c.y), srgb_encode(c.z));
}

fn srgb_decode3(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(srgb_decode(c.x), srgb_decode(c.y), srgb_decode(c.z));
}

// `BlendSpace::encode`.
fn to_space(c: vec3<f32>) -> vec3<f32> {
    if (params.flags.y == SPACE_LINEAR) {
        return c;
    }
    return srgb_encode3(c);
}

// `BlendSpace::decode`.
fn from_space(c: vec3<f32>) -> vec3<f32> {
    if (params.flags.y == SPACE_LINEAR) {
        return c;
    }
    return srgb_decode3(c);
}

// The straight color of a premultiplied pixel. Black if alpha <= 0.
fn unpremultiply(p: vec4<f32>) -> vec3<f32> {
    if (p.a > 0.0) {
        let inv = 1.0 / p.a;
        return p.rgb * inv;
    }
    return vec3<f32>(0.0);
}

// `BlendSpace::encode_premultiplied`.
fn encode_premultiplied(p: vec4<f32>) -> vec4<f32> {
    if (params.flags.y == SPACE_LINEAR) {
        return p;
    }
    return vec4<f32>(srgb_encode3(unpremultiply(p)) * p.a, p.a);
}

// `BlendSpace::decode_premultiplied`.
fn decode_premultiplied(p: vec4<f32>) -> vec4<f32> {
    if (params.flags.y == SPACE_LINEAR) {
        return p;
    }
    return vec4<f32>(srgb_decode3(unpremultiply(p)) * p.a, p.a);
}

// --- Blend modes ------------------------------------------------------------

fn color_burn(cb: f32, cs: f32) -> f32 {
    if (cb >= 1.0) {
        return 1.0;
    }
    if (cs <= 0.0) {
        return 0.0;
    }
    return 1.0 - min((1.0 - cb) / cs, 1.0);
}

fn color_dodge(cb: f32, cs: f32) -> f32 {
    if (cb <= 0.0) {
        return 0.0;
    }
    if (cs >= 1.0) {
        return 1.0;
    }
    return min(cb / (1.0 - cs), 1.0);
}

fn hard_light(cb: f32, cs: f32) -> f32 {
    if (cs <= 0.5) {
        return cb * (2.0 * cs);
    }
    let s = 2.0 * cs - 1.0;
    return cb + s - cb * s;
}

fn soft_light(cb: f32, cs: f32) -> f32 {
    if (cs <= 0.5) {
        return cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb);
    }
    var d: f32;
    if (cb <= 0.25) {
        d = ((16.0 * cb - 12.0) * cb + 4.0) * cb;
    } else {
        d = sqrt(cb);
    }
    return cb + (2.0 * cs - 1.0) * (d - cb);
}

// The formulas for inputs in 0..=1 (`BlendMode::bounded`).
fn bounded(mode: u32, cb: f32, cs: f32) -> f32 {
    switch mode {
        case MODE_COLOR_BURN: { return color_burn(cb, cs); }
        case MODE_COLOR_DODGE: { return color_dodge(cb, cs); }
        case MODE_OVERLAY: { return hard_light(cs, cb); }
        case MODE_SOFT_LIGHT: { return soft_light(cb, cs); }
        case MODE_HARD_LIGHT: { return hard_light(cb, cs); }
        case MODE_VIVID_LIGHT: {
            if (cs <= 0.5) {
                return color_burn(cb, 2.0 * cs);
            }
            return color_dodge(cb, 2.0 * cs - 1.0);
        }
        case MODE_LINEAR_LIGHT: { return clamp(cb + 2.0 * cs - 1.0, 0.0, 1.0); }
        case MODE_PIN_LIGHT: {
            if (cs <= 0.5) {
                return min(cb, 2.0 * cs);
            }
            return max(cb, 2.0 * cs - 1.0);
        }
        case MODE_HARD_MIX: { return select(0.0, 1.0, cb + cs >= 1.0); }
        case MODE_EXCLUSION: { return cb + cs - 2.0 * cb * cs; }
        default: { return cs; }
    }
}

// `BlendMode::unbounded`.
fn unbounded(mode: u32, cb: f32, cs: f32) -> f32 {
    switch mode {
        case MODE_DARKEN: { return min(cb, cs); }
        case MODE_LIGHTEN: { return max(cb, cs); }
        case MODE_MULTIPLY: { return cb * cs; }
        case MODE_LINEAR_BURN: { return max(cb + cs - 1.0, 0.0); }
        case MODE_LINEAR_DODGE: { return cb + cs; }
        case MODE_DIFFERENCE: { return abs(cb - cs); }
        case MODE_SUBTRACT: { return max(cb - cs, 0.0); }
        case MODE_DIVIDE: { return cb / max(cs, DIVIDE_MIN_SOURCE); }
        default: { return cs; }
    }
}

fn is_bounded(mode: u32) -> bool {
    return mode == MODE_COLOR_BURN || mode == MODE_COLOR_DODGE || mode == MODE_OVERLAY
        || mode == MODE_SOFT_LIGHT || mode == MODE_HARD_LIGHT || mode == MODE_VIVID_LIGHT
        || mode == MODE_LINEAR_LIGHT || mode == MODE_PIN_LIGHT || mode == MODE_HARD_MIX
        || mode == MODE_EXCLUSION;
}

// `BlendMode::blend_channel` (separable modes, with the range handling).
fn blend_channel(mode: u32, cb: f32, cs: f32) -> f32 {
    if (mode == MODE_SCREEN) {
        if (cb <= 1.0 && cs <= 1.0) {
            return cb + cs - cb * cs;
        }
        return max(cb, cs);
    }
    if (is_bounded(mode)) {
        let b = clamp(cb, 0.0, 1.0);
        let s = clamp(cs, 0.0, 1.0);
        return bounded(mode, b, s) + (cb - b);
    }
    return unbounded(mode, cb, cs);
}

fn clip_color(c: vec3<f32>, upper: f32) -> vec3<f32> {
    let l = luminance(c);
    if (l < 0.0 || l > upper) {
        return vec3<f32>(l);
    }
    let n = min(min(c.x, c.y), c.z);
    let x = max(max(c.x, c.y), c.z);
    var out = c;
    if (n < 0.0) {
        let k = l / (l - n);
        out = l + (out - l) * k;
    }
    if (x > upper) {
        let k = (upper - l) / (x - l);
        out = l + (out - l) * k;
    }
    return out;
}

fn set_lum(c: vec3<f32>, l: f32, upper: f32) -> vec3<f32> {
    let d = l - luminance(c);
    return clip_color(c + d, upper);
}

fn sat(c: vec3<f32>) -> f32 {
    return max(max(c.x, c.y), c.z) - min(min(c.x, c.y), c.z);
}

// W3C `SetSat`. The lowest channel gets 0, the highest gets `s`, and the
// middle channel scales linearly. A gray input gives 0.
fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    let lo = min(min(c.x, c.y), c.z);
    let hi = max(max(c.x, c.y), c.z);
    let range = hi - lo;
    if (range > 0.0) {
        let scaled = (c - lo) * s / range;
        // The highest channel gets exactly `s`, as in the CPU code.
        return select(scaled, vec3<f32>(s), c == vec3<f32>(hi));
    }
    return vec3<f32>(0.0);
}

fn clip_limit(cb: vec3<f32>, cs: vec3<f32>) -> f32 {
    return max(1.0, max(max(max(cb.x, cb.y), max(cb.z, cs.x)), max(cs.y, cs.z)));
}

// `BlendMode::blend` for straight colors.
fn blend(mode: u32, cb: vec3<f32>, cs: vec3<f32>) -> vec3<f32> {
    switch mode {
        case MODE_NORMAL, MODE_DISSOLVE: { return cs; }
        case MODE_DARKER_COLOR: { return select(cb, cs, luminance(cs) < luminance(cb)); }
        case MODE_LIGHTER_COLOR: { return select(cb, cs, luminance(cs) > luminance(cb)); }
        case MODE_HUE: {
            return set_lum(set_sat(cs, sat(cb)), luminance(cb), clip_limit(cb, cs));
        }
        case MODE_SATURATION: {
            return set_lum(set_sat(cb, sat(cs)), luminance(cb), clip_limit(cb, cs));
        }
        case MODE_COLOR: { return set_lum(cs, luminance(cb), clip_limit(cb, cs)); }
        case MODE_LUMINOSITY: { return set_lum(cb, luminance(cs), clip_limit(cb, cs)); }
        default: {
            return vec3<f32>(
                blend_channel(mode, cb.x, cs.x),
                blend_channel(mode, cb.y, cs.y),
                blend_channel(mode, cb.z, cs.z),
            );
        }
    }
}

// --- Coverage ---------------------------------------------------------------

// `dissolve_threshold`: the "lowbias32" hash of the canvas pixel, in 0..1.
// Canvas coordinates are not negative, so `bitcast` gives the same bits as
// the `i64 as u32` cast of the CPU code.
fn dissolve_threshold(x: i32, y: i32) -> f32 {
    var h = (bitcast<u32>(x) * 0x9E3779B9u) ^ (bitcast<u32>(y) * 0x85EBCA6Bu);
    h = h ^ (h >> 16u);
    h = h * 0x7FEB352Du;
    h = h ^ (h >> 15u);
    h = h * 0x846CA68Bu;
    h = h ^ (h >> 16u);
    return f32(h >> 8u) / 16777216.0;
}

fn dissolve(a: f32, x: i32, y: i32) -> f32 {
    if (params.flags.x != MODE_DISSOLVE) {
        return a;
    }
    return select(0.0, 1.0, a > 0.0 && dissolve_threshold(x, y) < a);
}

// `Mask::value_at` for canvas pixel (x, y), clamped to 0..=1.
fn mask_value(x: i32, y: i32) -> f32 {
    let mx = x - params.mask.x;
    let my = y - params.mask.y;
    var v = params.coef.y;
    if (mx >= 0 && my >= 0 && mx < params.mask.z && my < params.mask.w) {
        let texel = textureLoad(mask_tex, vec2<i32>(mx / 4, my), 0);
        v = texel[mx % 4];
    }
    return clamp(v, 0.0, 1.0);
}

// `opacity * fill * mask(x, y)`.
fn coverage(x: i32, y: i32) -> f32 {
    let base = params.coef.x;
    if (params.flags.z != 0u) {
        return base * mask_value(x, y);
    }
    return base;
}

// --- Blend of one pixel -----------------------------------------------------

// `blend_pixel` of `composite.rs`: blends the premultiplied source `src`
// with coverage `k` onto the premultiplied backdrop `dst`.
fn blend_pixel(dst: vec4<f32>, src: vec4<f32>, k: f32, x: i32, y: i32) -> vec4<f32> {
    let mode = params.flags.x;
    let a_src = src.a;
    let a_s = dissolve(min(a_src * k, 1.0), x, y);
    if (a_s <= 0.0 || a_src <= 0.0) {
        return dst;
    }
    let ab = dst.a;
    if (params.flags.y == SPACE_LINEAR && (mode == MODE_NORMAL || mode == MODE_DISSOLVE)) {
        // Fast path: co = as * cs + (1 - as) * cb (premultiplied).
        let s = a_s / a_src;
        return vec4<f32>(src.rgb * s + dst.rgb * (1.0 - a_s), a_s + ab * (1.0 - a_s));
    }
    let cs = to_space(unpremultiply(src));
    let cb = to_space(unpremultiply(dst));
    let b = blend(mode, cb, cs);
    let ao = a_s + ab * (1.0 - a_s);
    let co = a_s * (1.0 - ab) * cs + ab * (1.0 - a_s) * cb + a_s * ab * b;
    // ao >= a_s > 0.
    let straight = co / ao;
    return vec4<f32>(from_space(straight) * ao, ao);
}

// The canvas position of the output pixel `p`.
fn canvas_pos(p: vec2<i32>) -> vec2<i32> {
    return params.region.xy + p;
}
