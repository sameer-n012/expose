// Exposure kernel: output.rgb = input.rgb * gain, output.a = input.a.
//
// `gain` is 2^ev. The CPU calculates it (see `ExposureParams`), so that
// the GPU result and the CPU reference use the same multiplier.
// The kernel does not clamp. Values below 0.0 and above 1.0 stay
// (scene-referred data).
//
// Kernel contract: see `copy.wgsl` and `kernel.rs`.

struct Params {
    gain: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
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
    let color = textureLoad(input_image, pixel, 0);
    textureStore(output_image, pixel, vec4<f32>(color.rgb * params.gain, color.a));
}
