// Sum kernel: output = input0 + input1 for each channel.
//
// Kernel contract (see `kernel.rs`):
// - binding 0: input image 0, read with `textureLoad`.
// - binding 1: output image. `OUTPUT_FORMAT` is replaced at pipeline build.
// - binding 2: parameters (not used by this kernel).
// - binding 3: input image 1, read with `textureLoad`.
// - Both inputs must have the size of the output.
// - Workgroup size 8x8. The dispatch can be larger than the image, so each
//   invocation checks the image bounds.

@group(0) @binding(0) var input0: texture_2d<f32>;
@group(0) @binding(1) var output_image: texture_storage_2d<OUTPUT_FORMAT, write>;
@group(0) @binding(3) var input1: texture_2d<f32>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_image);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let pixel = vec2<i32>(id.xy);
    let sum = textureLoad(input0, pixel, 0) + textureLoad(input1, pixel, 0);
    textureStore(output_image, pixel, sum);
}
