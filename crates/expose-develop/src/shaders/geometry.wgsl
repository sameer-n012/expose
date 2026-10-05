// Geometry kernel: flip, straighten (rotate), and crop (version 1).
//
// The CPU reference is `Geometry::apply_cpu` in `geometry.rs`. Keep the
// two in agreement.
//
// - Not rotated: integer copy. Output pixel (x, y) reads frame pixel
//   (x0 + x, y0 + y), mirrored when a flip is on. The result is bit-exact.
// - Rotated: s = affine * (x + 0.5, y + 0.5, 1). When s is outside
//   [0, W] x [0, H], the output is transparent black. Otherwise the kernel
//   samples with Catmull-Rom (a = -0.5, 4x4 taps, clamp to edge), with the
//   same f32 math order as the CPU.
//
// Input 0 is the source (W x H). The output has the crop size.
// Kernel contract: see `expose-gpu` `kernel.rs`.

const FLAG_ROTATED: u32 = 1u;
const FLAG_FLIP_HORIZONTAL: u32 = 2u;
const FLAG_FLIP_VERTICAL: u32 = 4u;

struct Params {
    // (a, b, c, unused): s.x = a * ox + b * oy + c.
    row_x: vec4<f32>,
    // (d, e, f, unused): s.y = d * ox + e * oy + f.
    row_y: vec4<f32>,
    // (W, H, unused, unused) as f32.
    source_f: vec4<f32>,
    // x: W, y: H, z: x0, w: y0.
    source_origin: vec4<u32>,
    // x: flags. y, z, w: unused.
    flags: vec4<u32>,
}

@group(0) @binding(0) var input_image: texture_2d<f32>;
@group(0) @binding(1) var output_image: texture_storage_2d<OUTPUT_FORMAT, write>;
@group(0) @binding(2) var<uniform> params: Params;

// Catmull-Rom weights for the taps i-1, i, i+1, i+2 at the fraction t.
// Same as `catmull_rom_weights`.
fn weights(t: f32) -> vec4<f32> {
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4<f32>(
        -0.5 * t3 + t2 - 0.5 * t,
        1.5 * t3 - 2.5 * t2 + 1.0,
        -1.5 * t3 + 2.0 * t2 + 0.5 * t,
        0.5 * t3 - 0.5 * t2,
    );
}

fn clamp_index(i: i32, size: u32) -> i32 {
    return clamp(i, 0, i32(size) - 1);
}

fn sample(sx: f32, sy: f32) -> vec4<f32> {
    let w = params.source_origin.x;
    let h = params.source_origin.y;
    let ux = sx - 0.5;
    let uy = sy - 0.5;
    let bx = floor(ux);
    let by = floor(uy);
    let fx = i32(bx) - 1;
    let fy = i32(by) - 1;
    let wx = weights(ux - bx);
    let wy = weights(uy - by);
    let cols = vec4<i32>(
        clamp_index(fx, w),
        clamp_index(fx + 1, w),
        clamp_index(fx + 2, w),
        clamp_index(fx + 3, w),
    );
    var acc = vec4<f32>(0.0);
    for (var k = 0; k < 4; k = k + 1) {
        let row = clamp_index(fy + k, h);
        var row_acc = vec4<f32>(0.0);
        for (var j = 0; j < 4; j = j + 1) {
            let p = textureLoad(input_image, vec2<i32>(cols[j], row), 0);
            row_acc = row_acc + wx[j] * p;
        }
        acc = acc + wy[k] * row_acc;
    }
    return acc;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_image);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let out_pixel = vec2<i32>(id.xy);
    let flags = params.flags.x;
    let w = params.source_origin.x;
    let h = params.source_origin.y;

    if ((flags & FLAG_ROTATED) == 0u) {
        // Clamp so that a wrong parameter block cannot read outside.
        let fx = min(params.source_origin.z + id.x, w - 1u);
        let fy = min(params.source_origin.w + id.y, h - 1u);
        let sx = select(fx, w - 1u - fx, (flags & FLAG_FLIP_HORIZONTAL) != 0u);
        let sy = select(fy, h - 1u - fy, (flags & FLAG_FLIP_VERTICAL) != 0u);
        textureStore(output_image, out_pixel, textureLoad(input_image, vec2<i32>(i32(sx), i32(sy)), 0));
        return;
    }

    let ox = f32(id.x) + 0.5;
    let oy = f32(id.y) + 0.5;
    let sx = params.row_x.x * ox + params.row_x.y * oy + params.row_x.z;
    let sy = params.row_y.x * ox + params.row_y.y * oy + params.row_y.z;
    let inside = sx >= 0.0 && sx <= params.source_f.x && sy >= 0.0 && sy <= params.source_f.y;
    if (!inside) {
        textureStore(output_image, out_pixel, vec4<f32>(0.0));
        return;
    }
    textureStore(output_image, out_pixel, sample(sx, sy));
}
