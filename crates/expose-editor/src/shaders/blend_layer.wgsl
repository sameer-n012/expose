// Blends one source image (a pixel layer or an isolated group) onto the
// backdrop. `aux` is the source. `params.source` gives its canvas position
// and size. Outside the source, the source is transparent (no change).

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(output);
    if (gid.x >= size.x || gid.y >= size.y) {
        return;
    }
    let p = vec2<i32>(gid.xy);
    let c = canvas_pos(p);
    let dst = textureLoad(backdrop, p, 0);
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
