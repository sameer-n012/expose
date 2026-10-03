//! GPU version of the pipeline: one fused WGSL kernel.
//!
//! The kernel ([`DEVELOP_KERNEL`]) does all stages in one pass. Its
//! parameter block holds the fused matrix, the tone LUT, and the curve
//! LUTs. The block is about 24 KiB. This is less than the WebGPU default
//! limit for one uniform buffer binding (64 KiB), so the kernel needs no
//! storage buffers and fits the `expose-gpu` kernel contract.

use bytemuck::{Pod, Zeroable};
use expose_core::color::REC2020_LUMINANCE;
use expose_gpu::Kernel;

use crate::curve::{CURVE_LUT_SIZE, CurveTables};
use crate::tone::{
    LUMINANCE_FLOOR, MID_GRAY, TONE_LUT_MIN_EV, TONE_LUT_SAMPLES_PER_EV, TONE_LUT_SIZE, ToneLut,
};

/// The fused develop kernel. Parameters: [`GpuParams`] (built by the
/// pipeline).
pub const DEVELOP_KERNEL: Kernel = Kernel {
    name: "expose-develop/develop-v1",
    source: include_str!("shaders/develop.wgsl"),
};

/// Flag: the matrix stage is active.
pub(crate) const FLAG_MATRIX: u32 = 1;
/// Flag: the tone LUT stage is active.
pub(crate) const FLAG_TONE: u32 = 2;
/// Flag: the master curve is active. `FLAG_CURVE_MASTER << c` is curve `c`.
pub(crate) const FLAG_CURVE_MASTER: u32 = 4;

/// The tone LUT packed in `vec4` values.
const TONE_LUT_VEC4: usize = TONE_LUT_SIZE / 4;

/// The parameter block of [`DEVELOP_KERNEL`]. The layout matches the WGSL
/// `Params` struct (all members are 16-byte aligned `vec4` values or arrays
/// of them, so there is no implicit padding).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct GpuParams {
    matrix: [[f32; 4]; 3],
    flags: [u32; 4],
    tone: [f32; 4],
    luma: [f32; 4],
    curve_ends: [[f32; 4]; 4],
    tone_lut: [[f32; 4]; TONE_LUT_VEC4],
    curve_lut: [[f32; 4]; CURVE_LUT_SIZE],
}

// The WGSL struct uses fixed array lengths. Fail the build if they change.
const _: () = assert!(TONE_LUT_VEC4 == 512 && TONE_LUT_SIZE % 4 == 0);
const _: () = assert!(CURVE_LUT_SIZE == 1024);
// The WebGPU default `maxUniformBufferBindingSize` is 65536 bytes.
const _: () = assert!(size_of::<GpuParams>() <= 65536);

impl GpuParams {
    /// Builds the parameter block from the compiled stages.
    pub(crate) fn build(
        matrix: Option<&[[f32; 3]; 3]>,
        tone_lut: Option<&ToneLut>,
        curves: Option<&CurveTables>,
    ) -> Box<Self> {
        let mut p = Box::new(Self::zeroed());
        let mut flags = 0;
        if let Some(m) = matrix {
            flags |= FLAG_MATRIX;
            for (row, src) in p.matrix.iter_mut().zip(m) {
                *row = [src[0], src[1], src[2], 0.0];
            }
        }
        p.tone = [
            TONE_LUT_MIN_EV,
            TONE_LUT_SAMPLES_PER_EV,
            LUMINANCE_FLOOR,
            MID_GRAY,
        ];
        p.luma = [
            REC2020_LUMINANCE[0],
            REC2020_LUMINANCE[1],
            REC2020_LUMINANCE[2],
            0.0,
        ];
        if let Some(lut) = tone_lut {
            flags |= FLAG_TONE;
            for (dst, src) in p.tone_lut.iter_mut().zip(lut.chunks_exact(4)) {
                dst.copy_from_slice(src);
            }
        }
        if let Some(c) = curves {
            for (i, active) in c.active.iter().enumerate() {
                if *active {
                    flags |= FLAG_CURVE_MASTER << i;
                }
            }
            p.curve_ends = c.ends;
            p.curve_lut = *c.lut;
        }
        p.flags = [flags, 0, 0, 0];
        p
    }

    /// The stage flags (for tests).
    #[cfg(test)]
    pub(crate) fn flags(&self) -> u32 {
        self.flags[0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_and_layout() {
        // 6 vec4 + 4 vec4 + 512 vec4 + 1024 vec4.
        assert_eq!(size_of::<GpuParams>(), (6 + 4 + 512 + 1024) * 16);
        let p = GpuParams::build(None, None, None);
        assert_eq!(p.flags(), 0);
    }

    #[test]
    fn wgsl_uses_the_same_constants() {
        let src = DEVELOP_KERNEL.source;
        assert!(src.contains(&format!("const TONE_LUT_SIZE: u32 = {TONE_LUT_SIZE}u;")));
        assert!(src.contains(&format!("const CURVE_LUT_SIZE: u32 = {CURVE_LUT_SIZE}u;")));
        assert!(src.contains(&format!("array<vec4<f32>, {TONE_LUT_VEC4}>")));
        assert!(src.contains(&format!("array<vec4<f32>, {CURVE_LUT_SIZE}>")));
        assert!(src.contains(&format!("const FLAG_MATRIX: u32 = {FLAG_MATRIX}u;")));
        assert!(src.contains(&format!("const FLAG_TONE: u32 = {FLAG_TONE}u;")));
        assert!(src.contains(&format!(
            "const FLAG_CURVE_MASTER: u32 = {FLAG_CURVE_MASTER}u;"
        )));
    }
}
