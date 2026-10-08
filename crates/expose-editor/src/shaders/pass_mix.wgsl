// Mixes the result of a pass-through group (`aux`) with the state before
// the group (`backdrop`) by the group coverage, in the blend space.

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(output);
    if (gid.x >= size.x || gid.y >= size.y) {
        return;
    }
    let p = vec2<i32>(gid.xy);
    let c = canvas_pos(p);
    let dst = textureLoad(backdrop, p, 0);
    let after = textureLoad(aux, p, 0);
    let k = min(coverage(c.x, c.y), 1.0);
    if (k >= 1.0) {
        textureStore(output, p, after);
        return;
    }
    if (k <= 0.0) {
        textureStore(output, p, dst);
        return;
    }
    let b = encode_premultiplied(dst);
    let a = encode_premultiplied(after);
    textureStore(output, p, decode_premultiplied(b + k * (a - b)));
}
