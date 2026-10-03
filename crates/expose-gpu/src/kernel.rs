//! Compute kernels: one input image, one output image, one parameter block.
//!
//! # Kernel contract
//!
//! A kernel is WGSL source with one compute entry point named `main` and
//! this bind group 0:
//!
//! | Binding | WGSL type                                        | Content          |
//! | ------- | ------------------------------------------------ | ---------------- |
//! | 0       | `texture_2d<f32>`                                | input image      |
//! | 1       | `texture_storage_2d<OUTPUT_FORMAT, write>`       | output image     |
//! | 2       | `var<uniform>` struct (optional in the shader)   | parameters       |
//!
//! - Read the input with `textureLoad` (the input is not filterable).
//! - The text `OUTPUT_FORMAT` is replaced with the WGSL texel format of the
//!   output image (`rgba32float` or `rgba16float`) when the pipeline is built.
//! - The workgroup size must be [`WORKGROUP_SIZE`] x [`WORKGROUP_SIZE`] x 1.
//!   The dispatch covers the image with whole workgroups, so the shader must
//!   ignore invocations outside the image.
//!
//! Pipelines are compiled once for each (kernel name, output format) pair and
//! kept in the [`GpuContext`]. Each kernel must have a unique name.

use crate::context::GpuContext;
use crate::error::{GpuError, scoped};
use crate::image::{GpuImage, ImageFormat};

/// The side length of a kernel workgroup (8 x 8 = 64 invocations).
pub const WORKGROUP_SIZE: u32 = 8;

/// The text in the kernel source that is replaced with the output format.
pub const OUTPUT_FORMAT_TOKEN: &str = "OUTPUT_FORMAT";

/// The minimum size of the parameter buffer, in bytes. Uniform structs in
/// WGSL have a 16-byte alignment, so parameter blocks are padded to it.
const PARAMS_ALIGNMENT: usize = 16;

/// A compute kernel. See the module docs for the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kernel {
    /// A unique name. It is the pipeline cache key and the debug label.
    pub name: &'static str,
    /// The WGSL source.
    pub source: &'static str,
}

/// Pipeline cache key: kernel name and output format.
pub(crate) type PipelineKey = (&'static str, ImageFormat);

/// The bind group layout and pipeline layout for one output format.
///
/// The storage texture format is part of the layout, so each output format
/// needs its own layout.
#[derive(Debug)]
pub(crate) struct FormatLayout {
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
}

impl FormatLayout {
    pub(crate) fn new(device: &wgpu::Device, format: ImageFormat) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("expose-gpu kernel bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: format.wgpu_format(),
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("expose-gpu kernel pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        Self {
            bind_group_layout,
            pipeline_layout,
        }
    }
}

/// The number of workgroups that covers `extent` pixels (ceiling division).
pub const fn workgroup_count(extent: u32) -> u32 {
    extent.div_ceil(WORKGROUP_SIZE)
}

impl GpuContext {
    /// Runs `kernel` on `input` and writes the result to `output`.
    ///
    /// `params` goes to binding 2 as a uniform buffer. It is padded with zeros
    /// to a multiple of 16 bytes. Use `&()` when the kernel has no parameters.
    ///
    /// The pipeline is compiled on the first call for each kernel and output
    /// format, and is then taken from the cache. The function submits the
    /// work and returns; it does not wait for the GPU to complete it. A later
    /// read-back waits for it, because the queue keeps the order.
    ///
    /// # Errors
    ///
    /// - [`GpuError::SizeMismatch`] when the images have different sizes.
    /// - [`GpuError::SameImage`] when `input` and `output` are one image.
    /// - [`GpuError::InvalidKernel`] when the source has no `OUTPUT_FORMAT`.
    /// - [`GpuError::Validation`] for a WGSL error or a contract mismatch.
    pub async fn run_kernel<P: bytemuck::Pod>(
        &self,
        kernel: &Kernel,
        params: &P,
        input: &GpuImage,
        output: &GpuImage,
    ) -> Result<(), GpuError> {
        if input.width() != output.width() || input.height() != output.height() {
            return Err(GpuError::SizeMismatch {
                input_width: input.width(),
                input_height: input.height(),
                output_width: output.width(),
                output_height: output.height(),
            });
        }
        if input.id == output.id {
            return Err(GpuError::SameImage);
        }

        let pipeline = self.pipeline(kernel, output.format()).await?;
        let layout = &self.layouts[output.format().index()];
        let param_bytes = padded_params(params);

        scoped(&self.device, || {
            let params_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(kernel.name),
                // `usize` to `u64` cannot lose data on supported targets.
                size: param_bytes.len() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&params_buffer, 0, &param_bytes);

            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(kernel.name),
                layout: &layout.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&input.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&output.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: params_buffer.as_entire_binding(),
                    },
                ],
            });

            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some(kernel.name),
                });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(kernel.name),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(
                    workgroup_count(output.width()),
                    workgroup_count(output.height()),
                    1,
                );
            }
            self.queue.submit([encoder.finish()]);
        })
        .await
    }

    /// Returns the cached pipeline, or compiles and caches it.
    async fn pipeline(
        &self,
        kernel: &Kernel,
        format: ImageFormat,
    ) -> Result<wgpu::ComputePipeline, GpuError> {
        let key: PipelineKey = (kernel.name, format);
        if let Some(pipeline) = self.lock_pipelines().get(&key) {
            return Ok(pipeline.clone());
        }

        if !kernel.source.contains(OUTPUT_FORMAT_TOKEN) {
            return Err(GpuError::InvalidKernel {
                kernel: kernel.name,
                reason: format!("the source does not contain `{OUTPUT_FORMAT_TOKEN}`"),
            });
        }
        let source = kernel
            .source
            .replace(OUTPUT_FORMAT_TOKEN, format.wgsl_name());
        let layout = &self.layouts[format.index()];

        tracing::debug!(kernel = kernel.name, ?format, "compiling compute pipeline");
        let pipeline = scoped(&self.device, || {
            let module = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(kernel.name),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                });
            self.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(kernel.name),
                    layout: Some(&layout.pipeline_layout),
                    module: &module,
                    entry_point: Some("main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    cache: None,
                })
        })
        .await?;

        // Two concurrent first calls can both compile. Keep the first result,
        // so that all callers use one pipeline.
        Ok(self.lock_pipelines().entry(key).or_insert(pipeline).clone())
    }

    /// Locks the pipeline cache.
    ///
    /// A poisoned lock can only come from a panic while the lock is held.
    /// The code holds the lock only for one map lookup or insert, which
    /// leave the map consistent, so it is safe to continue.
    fn lock_pipelines(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::HashMap<PipelineKey, wgpu::ComputePipeline>>
    {
        self.pipelines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The number of compiled pipelines in the cache.
    pub fn cached_pipeline_count(&self) -> usize {
        self.lock_pipelines().len()
    }
}

/// Copies `params` into a zero-padded buffer of at least 16 bytes, with a
/// length that is a multiple of 16.
fn padded_params<P: bytemuck::Pod>(params: &P) -> Vec<u8> {
    let bytes = bytemuck::bytes_of(params);
    let len = bytes
        .len()
        .next_multiple_of(PARAMS_ALIGNMENT)
        .max(PARAMS_ALIGNMENT);
    let mut padded = vec![0_u8; len];
    padded[..bytes.len()].copy_from_slice(bytes);
    padded
}

/// Built-in kernels.
pub mod kernels {
    use super::Kernel;

    /// Identity: copies the input to the output. Parameters: `()`.
    pub const COPY: Kernel = Kernel {
        name: "copy",
        source: include_str!("shaders/copy.wgsl"),
    };

    /// Exposure: multiplies RGB by 2^ev; alpha stays. Parameters:
    /// [`ExposureParams`].
    pub const EXPOSURE: Kernel = Kernel {
        name: "exposure",
        source: include_str!("shaders/exposure.wgsl"),
    };

    /// Parameters of [`EXPOSURE`]. The layout matches the WGSL `Params`.
    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
    pub struct ExposureParams {
        /// The RGB multiplier (2^ev).
        pub gain: f32,
        _pad: [f32; 3],
    }

    impl ExposureParams {
        /// Makes the parameters for an exposure change of `ev` stops.
        pub fn from_ev(ev: f32) -> Self {
            Self {
                gain: ev.exp2(),
                _pad: [0.0; 3],
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{padded_params, workgroup_count};

    #[test]
    fn workgroup_count_rounds_up() {
        assert_eq!(workgroup_count(1), 1);
        assert_eq!(workgroup_count(8), 1);
        assert_eq!(workgroup_count(9), 2);
        assert_eq!(workgroup_count(37), 5);
        assert_eq!(workgroup_count(23), 3);
    }

    #[test]
    fn params_are_padded_to_16_bytes() {
        assert_eq!(padded_params(&()).len(), 16);
        assert_eq!(padded_params(&1.0_f32).len(), 16);
        assert_eq!(padded_params(&[0_u32; 5]).len(), 32);
        assert_eq!(&padded_params(&7_u32)[..4], &7_u32.to_ne_bytes());
    }
}
