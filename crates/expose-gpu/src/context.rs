//! Headless GPU context: instance, adapter, device, and queue.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::GpuError;
use crate::image::ImageFormat;
use crate::kernel::{KernelLayout, MAX_KERNEL_INPUTS, PipelineKey};

/// The power preference for the adapter choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PowerPreference {
    /// Prefer the fastest GPU (for example, a discrete GPU).
    #[default]
    HighPerformance,
    /// Prefer the GPU that uses the least power (for example, an integrated GPU).
    LowPower,
    /// Let wgpu choose.
    None,
}

impl From<PowerPreference> for wgpu::PowerPreference {
    fn from(value: PowerPreference) -> Self {
        match value {
            PowerPreference::HighPerformance => Self::HighPerformance,
            PowerPreference::LowPower => Self::LowPower,
            PowerPreference::None => Self::None,
        }
    }
}

/// Options for [`GpuContext::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GpuContextOptions {
    /// The power preference for the adapter choice.
    pub power_preference: PowerPreference,
}

/// Information about the selected adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterSummary {
    /// The adapter name (for example, "Apple M2").
    pub name: String,
    /// The graphics backend (for example, "metal", "vulkan", "webgpu").
    pub backend: String,
    /// The device type (for example, "`IntegratedGpu`").
    pub device_type: String,
    /// The driver name. Can be empty.
    pub driver: String,
    /// The driver version information. Can be empty.
    pub driver_info: String,
}

/// The device limits that are important to image processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuLimits {
    /// The maximum width and height of a 2D texture, in pixels.
    pub max_texture_dimension_2d: u32,
    /// The maximum size of one buffer, in bytes.
    pub max_buffer_size: u64,
    /// The maximum number of workgroups in one dispatch dimension.
    pub max_compute_workgroups_per_dimension: u32,
}

/// A headless GPU context. It owns the wgpu device and queue, and a cache of
/// compiled compute pipelines.
///
/// # Requirements
///
/// The context requests **no optional wgpu features**. All texture usages
/// that this crate needs are guaranteed by the WebGPU core specification:
///
/// - `rgba32float`, `rgba16float`, and `rgba8unorm`: sampled texture with an
///   *unfilterable* float sample type, write-only storage texture, copy
///   source, and copy destination.
/// - Kernels read pixels with `textureLoad`, not with a sampler. Thus
///   `rgba32float` does not need to be filterable (the optional
///   `float32-filterable` feature is not requested).
///
/// The context also requests the WebGPU default limits, but raises
/// `max_texture_dimension_2d` and `max_buffer_size` to the adapter values,
/// so that large photos fit in one texture where the adapter allows it.
///
/// [`GpuContext::new`] checks all of this. It returns
/// [`GpuError::UnsupportedFormat`] or [`GpuError::UnsupportedLimit`] when the
/// adapter cannot supply a requirement.
///
/// # Threads
///
/// The context does not start threads. All functions that wait for the GPU
/// are `async`. On native targets, they block in `Device::poll` and resolve
/// at once. In a browser, the event loop resolves them.
pub struct GpuContext {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    adapter_summary: AdapterSummary,
    limits: GpuLimits,
    /// Bind group and pipeline layouts, one for each output format and input
    /// count. Index: `[format.index()][input_count - 1]`.
    pub(crate) layouts: [[KernelLayout; MAX_KERNEL_INPUTS]; ImageFormat::ALL.len()],
    /// Compiled pipelines. The lock is never held across an `.await`.
    pub(crate) pipelines: Mutex<HashMap<PipelineKey, wgpu::ComputePipeline>>,
}

impl std::fmt::Debug for GpuContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuContext")
            .field("adapter", &self.adapter_summary)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

/// The texture usages that every [`ImageFormat`] must support.
pub(crate) const IMAGE_USAGES: wgpu::TextureUsages = wgpu::TextureUsages::TEXTURE_BINDING
    .union(wgpu::TextureUsages::STORAGE_BINDING)
    .union(wgpu::TextureUsages::COPY_SRC)
    .union(wgpu::TextureUsages::COPY_DST);

impl GpuContext {
    /// Creates a headless context (no surface).
    ///
    /// # Errors
    ///
    /// - [`GpuError::NoAdapter`] when no adapter is found.
    /// - [`GpuError::UnsupportedLimit`] or [`GpuError::UnsupportedFormat`]
    ///   when the adapter cannot supply the requirements (see the type docs).
    /// - [`GpuError::RequestDevice`] when the device request fails.
    pub async fn new(options: GpuContextOptions) -> Result<Self, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: options.power_preference.into(),
                force_fallback_adapter: false,
                compatible_surface: None,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|error| GpuError::NoAdapter(error.to_string()))?;

        let adapter_limits = adapter.limits();
        let required_limits = required_limits(&adapter_limits)?;
        check_formats(&adapter)?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("expose-gpu device"),
                required_features: wgpu::Features::empty(),
                required_limits: required_limits.clone(),
                ..Default::default()
            })
            .await
            .map_err(|error| GpuError::RequestDevice(error.to_string()))?;

        device.set_device_lost_callback(|reason, message| {
            tracing::error!(?reason, %message, "GPU device lost");
        });

        let info = adapter.get_info();
        let adapter_summary = AdapterSummary {
            name: info.name,
            backend: info.backend.to_str().to_owned(),
            device_type: format!("{:?}", info.device_type),
            driver: info.driver,
            driver_info: info.driver_info,
        };
        let limits = GpuLimits {
            max_texture_dimension_2d: required_limits.max_texture_dimension_2d,
            max_buffer_size: required_limits.max_buffer_size,
            max_compute_workgroups_per_dimension: required_limits
                .max_compute_workgroups_per_dimension,
        };
        tracing::info!(adapter = ?adapter_summary, ?limits, "GPU context created");

        // All 3 x 4 layouts are made now. They are small, and an eager table
        // needs no lock on the kernel run path.
        let layouts = ImageFormat::ALL.map(|format| {
            std::array::from_fn(|index| KernelLayout::new(&device, format, index + 1))
        });
        Ok(Self {
            device,
            queue,
            adapter_summary,
            limits,
            layouts,
            pipelines: Mutex::new(HashMap::new()),
        })
    }

    /// Returns information about the selected adapter.
    pub fn adapter_summary(&self) -> &AdapterSummary {
        &self.adapter_summary
    }

    /// Returns the limits of the device.
    pub fn limits(&self) -> GpuLimits {
        self.limits
    }

    /// Waits until the GPU completes all submitted work.
    ///
    /// On native targets, this blocks. In a browser, it does nothing,
    /// because the browser polls the device.
    pub fn wait_idle(&self) -> Result<(), GpuError> {
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map(|_| ())
            .map_err(|error| GpuError::Poll(error.to_string()))
    }
}

/// Builds the device limits: WebGPU defaults, with the texture and buffer
/// size limits raised to the adapter values.
fn required_limits(adapter_limits: &wgpu::Limits) -> Result<wgpu::Limits, GpuError> {
    let base = wgpu::Limits::default();
    let mut failure = None;
    base.check_limits_with_fail_fn(adapter_limits, true, |name, required, supported| {
        failure = Some(GpuError::UnsupportedLimit {
            name,
            required,
            supported,
        });
    });
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(wgpu::Limits {
        max_texture_dimension_2d: adapter_limits.max_texture_dimension_2d,
        max_buffer_size: adapter_limits.max_buffer_size,
        ..base
    })
}

/// Checks that the adapter supports all image usages for all image formats.
fn check_formats(adapter: &wgpu::Adapter) -> Result<(), GpuError> {
    for format in ImageFormat::ALL {
        let wgpu_format = format.wgpu_format();
        // The device has no optional features, so only the guaranteed
        // format features are usable. Check both: the guaranteed set (what
        // validation allows) and the adapter set (what the hardware does).
        let guaranteed = wgpu_format
            .guaranteed_format_features(wgpu::Features::empty())
            .allowed_usages;
        let supported = adapter
            .get_texture_format_features(wgpu_format)
            .allowed_usages;
        let missing = IMAGE_USAGES - (guaranteed & supported);
        if !missing.is_empty() {
            return Err(GpuError::UnsupportedFormat {
                format,
                missing: format!("{missing:?}"),
            });
        }
    }
    Ok(())
}
