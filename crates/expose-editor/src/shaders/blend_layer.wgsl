// Blends one source image (a pixel layer or an isolated group) onto the
// backdrop. `aux` is the source. `params.source` gives its canvas position
// and size. Outside the source, the source is transparent (no change).
//
// Transform mode (`params.flags.w == TRANSFORM_MODE`, only for the
// transform preview of `GpuCompositor::render_view_transformed`): the
// kernel samples the source and the mask through an affine map, with
// bilinear interpolation. The parameters are:
// - `adj[0] = (a, c, e, -)`, `adj[1] = (b, d, f, -)`: the source position
//   of the output pixel `p`, in source texel-index space (texel `i` has
//   its center at `i`): `q = (a * p.x + c * p.y + e, b * p.x + d * p.y + f)`.
// - `adj[2].xy`: the mask position relative to `q`, in mask pixels.
// - `params.source.zw`: the source size. `params.source.xy` is not used.
// - `params.mask.zw`: the mask size. `params.mask.xy` is not used.
// Outside the source, the source is transparent. Outside the mask, the
// mask has the fill value. The other kernels and the normal blend step set
// `flags.w` to 0, so that this code path does not change them.

const TRANSFORM_MODE: u32 = 1u;

// The source texel `i`. Transparent outside the source.
fn source_texel(i: vec2<i32>) -> vec4<f32> {
    if (i.x < 0 || i.y < 0 || i.x >= params.source.z || i.y >= params.source.w) {
        return vec4<f32>(0.0);
    }
    return textureLoad(aux, i, 0);
}

// Bilinear sample of the source at `q` (texel-index space).
fn sample_source(q: vec2<f32>) -> vec4<f32> {
    let q0 = floor(q);
    let f = q - q0;
    let i = vec2<i32>(q0);
    if (f.x == 0.0 && f.y == 0.0) {
        // On a texel center: the texel value without arithmetic, so that
        // an integer translation gives the exact values.
        return source_texel(i);
    }
    let top = source_texel(i) * (1.0 - f.x) + source_texel(i + vec2<i32>(1, 0)) * f.x;
    let bottom = source_texel(i + vec2<i32>(0, 1)) * (1.0 - f.x)
        + source_texel(i + vec2<i32>(1, 1)) * f.x;
    return top * (1.0 - f.y) + bottom * f.y;
}

// The mask value at mask pixel `i`, clamped to 0..=1. The fill value
// outside the mask.
fn mask_texel(i: vec2<i32>) -> f32 {
    var v = params.coef.y;
    if (i.x >= 0 && i.y >= 0 && i.x < params.mask.z && i.y < params.mask.w) {
        let texel = textureLoad(mask_tex, vec2<i32>(i.x / 4, i.y), 0);
        v = texel[i.x % 4];
    }
    return clamp(v, 0.0, 1.0);
}

// Bilinear sample of the clamped mask values at `q` (mask pixel-index
// space).
fn sample_mask(q: vec2<f32>) -> f32 {
    let q0 = floor(q);
    let f = q - q0;
    let i = vec2<i32>(q0);
    if (f.x == 0.0 && f.y == 0.0) {
        return mask_texel(i);
    }
    let top = mask_texel(i) * (1.0 - f.x) + mask_texel(i + vec2<i32>(1, 0)) * f.x;
    let bottom = mask_texel(i + vec2<i32>(0, 1)) * (1.0 - f.x)
        + mask_texel(i + vec2<i32>(1, 1)) * f.x;
    return top * (1.0 - f.y) + bottom * f.y;
}

// The transform mode of `main`.
fn blend_transformed(p: vec2<i32>, dst: vec4<f32>) -> vec4<f32> {
    let g = vec2<f32>(p);
    let q = vec2<f32>(
        params.adj[0].x * g.x + params.adj[0].y * g.y + params.adj[0].z,
        params.adj[1].x * g.x + params.adj[1].y * g.y + params.adj[1].z,
    );
    let src = sample_source(q);
    if (src.a <= 0.0) {
        return dst;
    }
    var k = params.coef.x;
    if (params.flags.z != 0u) {
        k = k * sample_mask(q + params.adj[2].xy);
    }
    let c = canvas_pos(p);
    return blend_pixel(dst, src, k, c.x, c.y);
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
    if (params.flags.w == TRANSFORM_MODE) {
        textureStore(output, p, blend_transformed(p, dst));
        return;
    }
    let s = c - params.source.xy;
    if (s.x < 0 || s.y < 0 || s.x >= params.source.z || s.y >= params.source.w) {
        textureStore(output, p, dst);
        return;
    }
    let src = textureLoad(aux, s, 0);
    if (src.a <= 0.0) {
        // No change. This also skips the mask read.
        textureStore(output, p, dst);
        return;
    }
    textureStore(output, p, blend_pixel(dst, src, coverage(c.x, c.y), c.x, c.y));
}
