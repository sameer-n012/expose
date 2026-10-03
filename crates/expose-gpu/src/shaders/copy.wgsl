// Identity kernel: output = input.
//
// Kernel contract (see `kernel.rs`):
// - binding 0: input image, read with `textureLoad`.
// - binding 1: output image. `OUTPUT_FORMAT` is replaced at pipeline build.
// - binding 2: parameters (not used by this kernel).
// - Workgroup size 8x8. The dispatch can be larger than the image, so each
//   invocation checks the image bounds.

@group(0) @binding(0) var input_image: texture_2d<f32>;
@group(0) @binding(1) var output_image: texture_storage_2d<OUTPUT_FORMAT, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_image);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let pixel = vec2<i32>(id.xy);
    textureStore(output_image, pixel, textureLoad(input_image, pixel, 0));
}
