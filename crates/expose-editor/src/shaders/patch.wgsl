// Copies a texture and replaces one rectangle with a patch:
//
//   output(p) = patch_image(p - rect.xy)  if p is inside `rect`
//   output(p) = base_image(p)             else
//
// `base_image` and `output` have the same size and format. `patch_image`
// has the size `rect.zw`. The compositor uses this kernel for partial updates of the
// layer texture cache and of the view composite (see "Partial updates" in
// `gpu.rs`).
//
// The kernel does no math: it only copies values. For an `rgba16float`
// output, all values come from `rgba16float` textures, so they are `f16`
// values, and a store keeps them without change. Thus the kernel does not
// need `quantizeToF16` (which also has no defined result for infinite
// values, and a layer texture can hold them).

struct PatchParams {
    // Texel position (x, y) and size (z, w) of the patch in the output.
    rect: vec4<i32>,
}

@group(0) @binding(0) var base_image: texture_2d<f32>;
@group(0) @binding(1) var output: texture_storage_2d<OUTPUT_FORMAT, write>;
@group(0) @binding(2) var<uniform> params: PatchParams;
@group(0) @binding(3) var patch_image: texture_2d<f32>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(output);
    if (gid.x >= size.x || gid.y >= size.y) {
        return;
    }
    let p = vec2<i32>(gid.xy);
    let q = p - params.rect.xy;
    var value: vec4<f32>;
    if (all(q >= vec2<i32>(0)) && all(q < params.rect.zw)) {
        value = textureLoad(patch_image, q, 0);
    } else {
        value = textureLoad(base_image, p, 0);
    }
    textureStore(output, p, value);
}
