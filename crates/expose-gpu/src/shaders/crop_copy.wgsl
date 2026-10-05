// Crop kernel: output(x, y) = input(x + offset.x, y + offset.y).
//
// Kernel contract (see `kernel.rs`):
// - binding 0: input image, read with `textureLoad`. It can be larger than
//   the output.
// - binding 1: output image. `OUTPUT_FORMAT` is replaced at pipeline build.
// - binding 2: parameters (`OffsetParams` in Rust).
// - Workgroup size 8x8. The dispatch covers the output and can be larger
//   than it, so each invocation checks the output bounds.
// - Input coordinates are clamped to the input, because a read outside the
//   texture has no defined value.

struct Params {
    offset: vec2<i32>,
    _pad: vec2<i32>,
}

@group(0) @binding(0) var input_image: texture_2d<f32>;
@group(0) @binding(1) var output_image: texture_storage_2d<OUTPUT_FORMAT, write>;
@group(0) @binding(2) var<uniform> params: Params;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_image);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let pixel = vec2<i32>(id.xy);
    let last = vec2<i32>(textureDimensions(input_image)) - vec2<i32>(1, 1);
    let from_pixel = clamp(pixel + params.offset, vec2<i32>(0, 0), last);
    textureStore(output_image, pixel, textureLoad(input_image, from_pixel, 0));
}
