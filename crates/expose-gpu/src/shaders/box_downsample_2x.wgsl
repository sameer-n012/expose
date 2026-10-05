// 2x box downsample: output(x, y) = mean of input(2x..2x+1, 2y..2y+1).
//
// Kernel contract (see `kernel.rs`):
// - binding 0: input image, read with `textureLoad`. Its size is about two
//   times the output size.
// - binding 1: output image. `OUTPUT_FORMAT` is replaced at pipeline build.
// - binding 2: parameters (not used by this kernel).
// - Workgroup size 8x8. The dispatch covers the output and can be larger
//   than it, so each invocation checks the output bounds.
// - Input coordinates are clamped to the input. At an odd edge, the edge
//   pixel counts two times in the mean.

@group(0) @binding(0) var input_image: texture_2d<f32>;
@group(0) @binding(1) var output_image: texture_storage_2d<OUTPUT_FORMAT, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_image);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let pixel = vec2<i32>(id.xy);
    let last = vec2<i32>(textureDimensions(input_image)) - vec2<i32>(1, 1);
    let origin = pixel * 2;
    var sum = vec4<f32>(0.0);
    for (var dy = 0; dy < 2; dy++) {
        for (var dx = 0; dx < 2; dx++) {
            let from_pixel = clamp(origin + vec2<i32>(dx, dy), vec2<i32>(0, 0), last);
            sum += textureLoad(input_image, from_pixel, 0);
        }
    }
    textureStore(output_image, pixel, sum * 0.25);
}
