// Makes the next level of the view pyramid: each output texel is the mean
// of a 2 x 2 block of input texels (premultiplied alpha).
//
// The input size can be odd. Then the last block is partly outside the
// input. The missing texels are outside the canvas, so they count as
// transparent (zero). The mean thus stays a true area mean, in which the
// area outside the canvas is transparent.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var output: texture_storage_2d<OUTPUT_FORMAT, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(output);
    if (gid.x >= size.x || gid.y >= size.y) {
        return;
    }
    let src_size = vec2<i32>(textureDimensions(src));
    let base = vec2<i32>(gid.xy) * 2;
    var sum = vec4<f32>(0.0);
    for (var dy = 0; dy < 2; dy = dy + 1) {
        for (var dx = 0; dx < 2; dx = dx + 1) {
            let p = base + vec2<i32>(dx, dy);
            if (p.x < src_size.x && p.y < src_size.y) {
                sum = sum + textureLoad(src, p, 0);
            }
        }
    }
    textureStore(output, vec2<i32>(gid.xy), sum * 0.25);
}
