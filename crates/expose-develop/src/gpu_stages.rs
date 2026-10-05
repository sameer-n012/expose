//! GPU kernels of the geometry and sharpening stages.
//!
//! Each kernel follows the `expose-gpu` kernel contract and uses
//! `GpuContext::run_kernel_multi` (the output size can differ from the
//! input size). The CPU references are [`Geometry::apply_cpu`] and
//! [`Sharpener::apply_cpu`]. The parameter blocks below match the WGSL
//! `Params` structs: all members are `vec4` values or arrays of them, so
//! there is no implicit padding.

use bytemuck::{Pod, Zeroable};
use expose_core::color::REC2020_LUMINANCE;
use expose_gpu::Kernel;

use crate::geometry::Geometry;
use crate::sharpen::{
    HALO_SOFT_LIMIT, LUMINANCE_EPSILON, MASK_RAMP_WIDTH, MASK_THRESHOLD_SCALE, MAX_TAPS, Sharpener,
};

/// Geometry kernel (flip, rotate, crop). One input: the source. The output
/// must have [`Geometry::output_size`]. Parameters: [`GeometryParams`].
pub const GEOMETRY_KERNEL: Kernel = Kernel {
    name: "expose-develop/geometry-v1",
    source: include_str!("shaders/geometry.wgsl"),
};

/// Sharpening pass 1: the encoded luminance `E` into the red channel. One
/// input: the developed image. Same output size. Parameters:
/// [`SharpenEncodeParams`].
pub const SHARPEN_ENCODE_KERNEL: Kernel = Kernel {
    name: "expose-develop/sharpen-encode-v1",
    source: include_str!("shaders/sharpen_encode.wgsl"),
};

/// Sharpening pass 2: Gaussian blur of the red channel along x. One input.
/// Same output size. Parameters: [`SharpenBlurParams`].
pub const SHARPEN_BLUR_H_KERNEL: Kernel = Kernel {
    name: "expose-develop/sharpen-blur-h-v1",
    source: include_str!("shaders/sharpen_blur_h.wgsl"),
};

/// Sharpening pass 3: Gaussian blur of the red channel along y. One input.
/// Same output size. Parameters: [`SharpenBlurParams`].
pub const SHARPEN_BLUR_V_KERNEL: Kernel = Kernel {
    name: "expose-develop/sharpen-blur-v-v1",
    source: include_str!("shaders/sharpen_blur_v.wgsl"),
};

/// Sharpening pass 4: combine. Three inputs: the developed image, `E`, and
/// the blurred `B`. Same output size. Parameters: [`SharpenCombineParams`].
pub const SHARPEN_COMBINE_KERNEL: Kernel = Kernel {
    name: "expose-develop/sharpen-combine-v1",
    source: include_str!("shaders/sharpen_combine.wgsl"),
};

/// Geometry flag: the angle is not zero (interpolate).
pub(crate) const GEOMETRY_FLAG_ROTATED: u32 = 1;
/// Geometry flag: horizontal flip.
pub(crate) const GEOMETRY_FLAG_FLIP_HORIZONTAL: u32 = 2;
/// Geometry flag: vertical flip.
pub(crate) const GEOMETRY_FLAG_FLIP_VERTICAL: u32 = 4;
/// Combine flag: the edge mask is on (`masking > 0`).
pub(crate) const COMBINE_FLAG_MASK: u32 = 1;

/// Number of `vec4` values that hold the blur weights.
const WEIGHT_VEC4: usize = MAX_TAPS / 4;
const _: () = assert!(MAX_TAPS % 4 == 0 && WEIGHT_VEC4 == 4);

/// The parameter block of [`GEOMETRY_KERNEL`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct GeometryParams {
    row_x: [f32; 4],
    row_y: [f32; 4],
    source_f: [f32; 4],
    source_origin: [u32; 4],
    flags: [u32; 4],
}

impl GeometryParams {
    /// Builds the parameter block for `geometry`.
    #[must_use]
    pub fn new(geometry: &Geometry) -> Self {
        let [row_x, row_y] = geometry.affine();
        let (width, height) = geometry.source_size();
        let (x0, y0) = geometry.origin();
        let (flip_h, flip_v) = geometry.flips();
        let mut flags = 0;
        if geometry.needs_resample() {
            flags |= GEOMETRY_FLAG_ROTATED;
        }
        if flip_h {
            flags |= GEOMETRY_FLAG_FLIP_HORIZONTAL;
        }
        if flip_v {
            flags |= GEOMETRY_FLAG_FLIP_VERTICAL;
        }
        #[allow(
            clippy::cast_precision_loss,
            reason = "image sizes are below 2^24 pixels per side, as on the CPU"
        )]
        let source_f = [width as f32, height as f32, 0.0, 0.0];
        Self {
            row_x: [row_x[0], row_x[1], row_x[2], 0.0],
            row_y: [row_y[0], row_y[1], row_y[2], 0.0],
            source_f,
            source_origin: [width, height, x0, y0],
            flags: [flags, 0, 0, 0],
        }
    }
}

/// The parameter block of [`SHARPEN_ENCODE_KERNEL`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct SharpenEncodeParams {
    luma: [f32; 4],
}

impl Default for SharpenEncodeParams {
    fn default() -> Self {
        Self { luma: luma_vec4() }
    }
}

/// The parameter block of [`SHARPEN_BLUR_H_KERNEL`] and
/// [`SHARPEN_BLUR_V_KERNEL`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct SharpenBlurParams {
    weights: [[f32; 4]; WEIGHT_VEC4],
    info: [u32; 4],
}

impl SharpenBlurParams {
    /// Builds the parameter block from the weights of `sharpener`.
    #[must_use]
    pub fn new(sharpener: &Sharpener) -> Self {
        let mut flat = [0.0_f32; MAX_TAPS];
        // `weights().len() <= MAX_TAPS` (see `Sharpener::weights`); `zip`
        // stops at the shorter side in any case.
        for (dst, src) in flat.iter_mut().zip(sharpener.weights()) {
            *dst = *src;
        }
        let mut weights = [[0.0_f32; 4]; WEIGHT_VEC4];
        for (dst, src) in weights.iter_mut().zip(flat.chunks_exact(4)) {
            dst.copy_from_slice(src);
        }
        // The half-width is at most MAX_TAPS - 1 = 15.
        let half_width = u32::try_from(sharpener.half_width().min(MAX_TAPS - 1)).unwrap_or(0);
        Self {
            weights,
            info: [half_width, 0, 0, 0],
        }
    }
}

/// The parameter block of [`SHARPEN_COMBINE_KERNEL`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct SharpenCombineParams {
    factors: [f32; 4],
    limits: [f32; 4],
    luma: [f32; 4],
    flags: [u32; 4],
}

impl SharpenCombineParams {
    /// Builds the parameter block from `sharpener`. The mask thresholds
    /// use the same `f32` math as the CPU.
    #[must_use]
    pub fn new(sharpener: &Sharpener) -> Self {
        let masking = sharpener.masking();
        let lo = masking * MASK_THRESHOLD_SCALE;
        let hi = lo + MASK_RAMP_WIDTH;
        let flags = if masking > 0.0 { COMBINE_FLAG_MASK } else { 0 };
        Self {
            factors: [sharpener.gain(), sharpener.detail(), lo, hi],
            limits: [HALO_SOFT_LIMIT, LUMINANCE_EPSILON, 0.0, 0.0],
            luma: luma_vec4(),
            flags: [flags, 0, 0, 0],
        }
    }
}

fn luma_vec4() -> [f32; 4] {
    [
        REC2020_LUMINANCE[0],
        REC2020_LUMINANCE[1],
        REC2020_LUMINANCE[2],
        0.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use expose_core::develop::{Crop, NormalizedRect, Sharpening};

    #[test]
    fn sizes_match_the_wgsl_structs() {
        assert_eq!(size_of::<GeometryParams>(), 5 * 16);
        assert_eq!(size_of::<SharpenEncodeParams>(), 16);
        assert_eq!(size_of::<SharpenBlurParams>(), 5 * 16);
        assert_eq!(size_of::<SharpenCombineParams>(), 4 * 16);
    }

    #[test]
    fn wgsl_uses_the_same_flags() {
        let g = GEOMETRY_KERNEL.source;
        assert!(g.contains(&format!(
            "const FLAG_ROTATED: u32 = {GEOMETRY_FLAG_ROTATED}u;"
        )));
        assert!(g.contains(&format!(
            "const FLAG_FLIP_HORIZONTAL: u32 = {GEOMETRY_FLAG_FLIP_HORIZONTAL}u;"
        )));
        assert!(g.contains(&format!(
            "const FLAG_FLIP_VERTICAL: u32 = {GEOMETRY_FLAG_FLIP_VERTICAL}u;"
        )));
        let c = SHARPEN_COMBINE_KERNEL.source;
        assert!(c.contains(&format!("const FLAG_MASK: u32 = {COMBINE_FLAG_MASK}u;")));
        for k in [SHARPEN_BLUR_H_KERNEL, SHARPEN_BLUR_V_KERNEL] {
            assert!(
                k.source
                    .contains(&format!("array<vec4<f32>, {WEIGHT_VEC4}>"))
            );
            assert!(k.source.contains(&format!("{}u)", MAX_TAPS - 1)));
        }
    }

    #[test]
    fn geometry_params_hold_flags_and_origin() {
        let crop = Crop {
            rect: NormalizedRect {
                left: 0.25,
                top: 0.5,
                width: 0.5,
                height: 0.5,
            },
            flip_horizontal: true,
            ..Crop::default()
        };
        let g = Geometry::new(&crop, 8, 4).unwrap();
        let p = GeometryParams::new(&g);
        assert_eq!(p.source_origin, [8, 4, 2, 2]);
        assert_eq!(p.flags[0], GEOMETRY_FLAG_FLIP_HORIZONTAL);
        let rotated = Geometry::new(
            &Crop {
                angle: 5.0,
                ..Crop::default()
            },
            8,
            4,
        )
        .unwrap();
        assert_eq!(
            GeometryParams::new(&rotated).flags[0],
            GEOMETRY_FLAG_ROTATED
        );
    }

    #[test]
    fn blur_params_hold_weights() {
        let settings = Sharpening {
            amount: 50.0,
            radius: 1.0,
            ..Sharpening::default()
        };
        let s = Sharpener::new(&settings, 1.0).unwrap().unwrap();
        let p = SharpenBlurParams::new(&s);
        assert_eq!(p.info[0] as usize, s.half_width());
        let flat: Vec<f32> = p.weights.iter().flatten().copied().collect();
        assert_eq!(&flat[..s.weights().len()], s.weights());
        assert!(flat[s.weights().len()..].iter().all(|w| *w == 0.0));
    }
}
