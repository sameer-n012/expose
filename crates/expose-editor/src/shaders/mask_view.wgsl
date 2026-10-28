// Renders the mask of one layer as an opaque gray image of the canvas.
//
// Inputs (see `common.wgsl`): the mask at binding 3, the region (the full
// canvas) in `params.region`, the uploaded mask part in `params.mask`,
// and the fill value in `params.coef.y`. The mask flag is not used: a
// disabled mask also shows its values.
//
// The output is premultiplied with alpha 1 (thus also straight). The gray
// value is the sRGB decode of the mask value, so that a display encoder
// with the sRGB transfer curve shows the mask value `v` as display gray `v`.
//
// The output is `rgba16float` (`VIEW_FORMAT` in `gpu.rs`). A store of an
// `f32` value to that format can round toward zero on some GPUs. Thus the
// kernel first rounds the gray value to the nearest `f16` value (the value
// is in 0..=1, inside the finite `f16` range).

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(output);
    if (gid.x >= size.x || gid.y >= size.y) {
        return;
    }
    let x = params.region.x + i32(gid.x);
    let y = params.region.y + i32(gid.y);
    let g = quantizeToF16(srgb_decode(mask_value(x, y)));
    textureStore(output, vec2<i32>(gid.xy), vec4<f32>(g, g, g, 1.0));
}
