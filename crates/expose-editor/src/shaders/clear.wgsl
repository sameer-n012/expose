// Fills the output with transparent black.

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(output);
    if (gid.x >= size.x || gid.y >= size.y) {
        return;
    }
    textureStore(output, vec2<i32>(gid.xy), vec4<f32>(0.0));
}
