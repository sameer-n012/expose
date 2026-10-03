//! GPU images: 2D float textures with upload and read-back.

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use half::f16;
use half::slice::HalfFloatSliceExt;

use crate::context::{GpuContext, IMAGE_USAGES};
use crate::error::{GpuError, scoped};
use crate::oneshot;

/// Number of channels per pixel. All images are RGBA.
pub const CHANNELS: usize = 4;

/// The maximum size of one upload or read-back strip, in bytes.
///
/// Uploads and read-backs move the image in strips of full rows. This
/// limits the size of staging memory, and keeps each staging buffer below
/// the `max_buffer_size` limit of the device.
const STRIP_BYTES: u64 = 64 * 1024 * 1024;

/// The pixel format of a [`GpuImage`].
///
/// The CPU side always uses `f32` RGBA (interleaved, row-major). The format
/// sets only the storage on the GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    /// 4 x 32-bit float. Full working precision. 16 bytes per pixel.
    Rgba32Float,
    /// 4 x 16-bit float. For previews. 8 bytes per pixel.
    ///
    /// Upload converts each `f32` to the nearest `f16` (round to nearest
    /// even). Values above 65504 become infinity. Read-back converts to
    /// `f32` without loss.
    Rgba16Float,
    /// 4 x 8-bit unsigned normalized (`0..=255` maps to `0.0..=1.0`). For
    /// display-encoded frames. 4 bytes per pixel.
    ///
    /// Kernels write `f32` values; the GPU clips them to `0.0..=1.0` and
    /// converts them to 8 bits. To get an exact code `c`, write `c / 255`.
    /// [`GpuImage::read_rgba8`] reads the bytes without conversion.
    /// `f32` upload clips to `0.0..=1.0` and rounds to the nearest code;
    /// `f32` read-back gives `code / 255`.
    Rgba8Unorm,
}

impl ImageFormat {
    /// All formats, in a fixed order.
    pub const ALL: [Self; 3] = [Self::Rgba32Float, Self::Rgba16Float, Self::Rgba8Unorm];

    /// Index of this format in [`Self::ALL`].
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Rgba32Float => 0,
            Self::Rgba16Float => 1,
            Self::Rgba8Unorm => 2,
        }
    }

    /// The bytes of one pixel on the GPU.
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::Rgba32Float => 16,
            Self::Rgba16Float => 8,
            Self::Rgba8Unorm => 4,
        }
    }

    /// The WGSL name of the texel format (for storage texture types).
    pub const fn wgsl_name(self) -> &'static str {
        match self {
            Self::Rgba32Float => "rgba32float",
            Self::Rgba16Float => "rgba16float",
            Self::Rgba8Unorm => "rgba8unorm",
        }
    }

    pub(crate) const fn wgpu_format(self) -> wgpu::TextureFormat {
        match self {
            Self::Rgba32Float => wgpu::TextureFormat::Rgba32Float,
            Self::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
            Self::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
        }
    }
}

/// Converts a unit value to an 8-bit code: clip to `0.0..=1.0` (NaN gives
/// 0), scale to 255, and round to the nearest code.
fn unit_to_u8(v: f32) -> u8 {
    let clipped = if v > 0.0 { v.min(1.0) } else { 0.0 };
    // `clipped * 255.0` is in `0.0..=255.0`, so the cast cannot truncate.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let code = (clipped * 255.0).round() as u8;
    code
}

/// Gives each image a unique ID, so that kernels can detect when the same
/// image is used as input and output.
static NEXT_IMAGE_ID: AtomicU64 = AtomicU64::new(0);

/// A 2D RGBA float image on the GPU.
///
/// The texture has these usages: sampled (for `textureLoad`), write-only
/// storage, copy source, and copy destination. Thus each image can be a
/// kernel input or a kernel output.
///
/// An image belongs to the [`GpuContext`] that created it. Do not use it
/// with a different context.
#[derive(Debug)]
pub struct GpuImage {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    width: u32,
    height: u32,
    format: ImageFormat,
    pub(crate) id: u64,
}

impl GpuImage {
    /// Creates an image. The GPU fills the contents with zeros.
    ///
    /// # Errors
    ///
    /// - [`GpuError::InvalidDimensions`] when a side is zero or larger than
    ///   the device limit.
    /// - [`GpuError::OutOfMemory`] when the GPU has no memory for the texture.
    pub async fn new(
        ctx: &GpuContext,
        width: u32,
        height: u32,
        format: ImageFormat,
    ) -> Result<Self, GpuError> {
        let max = ctx.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > max || height > max {
            return Err(GpuError::InvalidDimensions { width, height, max });
        }
        let texture = scoped(&ctx.device, || {
            ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("expose-gpu image"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: format.wgpu_format(),
                usage: IMAGE_USAGES,
                view_formats: &[],
            })
        })
        .await?;
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Ok(Self {
            texture,
            view,
            width,
            height,
            format,
            id: NEXT_IMAGE_ID.fetch_add(1, Ordering::Relaxed),
        })
    }

    /// Creates an image and uploads RGBA `f32` pixels to it.
    ///
    /// `data` is interleaved RGBA, row-major, with no row padding. Its length
    /// must be `width * height * 4`.
    pub async fn from_rgba_f32(
        ctx: &GpuContext,
        width: u32,
        height: u32,
        format: ImageFormat,
        data: &[f32],
    ) -> Result<Self, GpuError> {
        // Check the length before the texture is created.
        check_length(width, height, data.len())?;
        let image = Self::new(ctx, width, height, format).await?;
        image.write_rgba_f32(ctx, data).await?;
        Ok(image)
    }

    /// The width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The GPU pixel format.
    pub fn format(&self) -> ImageFormat {
        self.format
    }

    /// The number of `f32` samples on the CPU side (`width * height * 4`).
    pub fn sample_count(&self) -> usize {
        sample_count(self.width, self.height)
    }

    /// Replaces all pixels with RGBA `f32` data.
    ///
    /// `data` is interleaved RGBA, row-major, with no row padding. For
    /// [`ImageFormat::Rgba16Float`], each sample is converted to `f16`.
    ///
    /// The upload goes to the GPU queue. Later kernels and read-backs see
    /// the new data, because the queue keeps the order of operations.
    pub async fn write_rgba_f32(&self, ctx: &GpuContext, data: &[f32]) -> Result<(), GpuError> {
        check_length(self.width, self.height, data.len())?;
        let row_samples = self.width as usize * CHANNELS;
        let bytes_per_row = self.width * self.format.bytes_per_pixel();
        let mut half_rows: Vec<f16> = Vec::new();
        let mut byte_rows: Vec<u8> = Vec::new();
        scoped(&ctx.device, || {
            for rows in self.strips(u64::from(bytes_per_row)) {
                let samples =
                    &data[rows.start as usize * row_samples..rows.end as usize * row_samples];
                let bytes: &[u8] = match self.format {
                    ImageFormat::Rgba32Float => bytemuck::cast_slice(samples),
                    ImageFormat::Rgba16Float => {
                        half_rows.resize(samples.len(), f16::ZERO);
                        half_rows.convert_from_f32_slice(samples);
                        bytemuck::cast_slice(&half_rows)
                    }
                    ImageFormat::Rgba8Unorm => {
                        byte_rows.clear();
                        byte_rows.extend(samples.iter().map(|&v| unit_to_u8(v)));
                        &byte_rows
                    }
                };
                self.write_strip(ctx, &rows, bytes, bytes_per_row);
            }
        })
        .await
    }

    /// Replaces all pixels of an [`ImageFormat::Rgba8Unorm`] image with
    /// RGBA8 bytes (interleaved, row-major, no row padding).
    ///
    /// # Errors
    ///
    /// - [`GpuError::FormatMismatch`] when the image is not `Rgba8Unorm`.
    /// - [`GpuError::DataLength`] when `data.len()` is not
    ///   `width * height * 4`.
    pub async fn write_rgba8(&self, ctx: &GpuContext, data: &[u8]) -> Result<(), GpuError> {
        self.require_format(ImageFormat::Rgba8Unorm)?;
        check_length(self.width, self.height, data.len())?;
        let bytes_per_row = self.width * self.format.bytes_per_pixel();
        let row_bytes = bytes_per_row as usize;
        scoped(&ctx.device, || {
            for rows in self.strips(u64::from(bytes_per_row)) {
                let bytes = &data[rows.start as usize * row_bytes..rows.end as usize * row_bytes];
                self.write_strip(ctx, &rows, bytes, bytes_per_row);
            }
        })
        .await
    }

    /// Queues the upload of one strip of tightly packed rows.
    fn write_strip(&self, ctx: &GpuContext, rows: &Range<u32>, bytes: &[u8], bytes_per_row: u32) {
        // `Queue::write_texture` has no 256-byte row alignment rule (only
        // buffer-to-texture copies have it). Thus rows stay tightly packed.
        ctx.queue.write_texture(
            self.copy_info(rows.start),
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(rows.end - rows.start),
            },
            self.strip_extent(rows),
        );
    }

    /// Returns [`GpuError::FormatMismatch`] when the image format is not
    /// `expected`.
    fn require_format(&self, expected: ImageFormat) -> Result<(), GpuError> {
        if self.format == expected {
            Ok(())
        } else {
            Err(GpuError::FormatMismatch {
                expected,
                actual: self.format,
            })
        }
    }

    /// Reads all pixels back as RGBA `f32` data (interleaved, row-major, no
    /// row padding).
    ///
    /// The GPU copies the texture to a staging buffer in strips of rows.
    /// Each buffer row is padded to a multiple of 256 bytes, as WebGPU
    /// requires; this function removes the padding.
    pub async fn read_rgba_f32(&self, ctx: &GpuContext) -> Result<Vec<f32>, GpuError> {
        let mut output = vec![0.0_f32; self.sample_count()];
        let mut half_rows: Vec<f16> = Vec::new();
        let row_samples = self.width as usize * CHANNELS;
        let format = self.format;
        self.read_strips(ctx, |rows, unpad| {
            let first = rows.start as usize * row_samples;
            let last = rows.end as usize * row_samples;
            match format {
                ImageFormat::Rgba32Float => {
                    unpad(bytemuck::cast_slice_mut(&mut output[first..last]));
                }
                ImageFormat::Rgba16Float => {
                    half_rows.resize(last - first, f16::ZERO);
                    unpad(bytemuck::cast_slice_mut(&mut half_rows));
                    half_rows.convert_to_f32_slice(&mut output[first..last]);
                }
                ImageFormat::Rgba8Unorm => {
                    let mut bytes = vec![0_u8; last - first];
                    unpad(&mut bytes);
                    for (dst, &code) in output[first..last].iter_mut().zip(&bytes) {
                        *dst = f32::from(code) / 255.0;
                    }
                }
            }
        })
        .await?;
        Ok(output)
    }

    /// Reads all pixels of an [`ImageFormat::Rgba8Unorm`] image as RGBA8
    /// bytes (interleaved, row-major, no row padding). The bytes are the
    /// texel codes, with no conversion.
    ///
    /// # Errors
    ///
    /// [`GpuError::FormatMismatch`] when the image is not `Rgba8Unorm`, and
    /// the read-back errors of [`Self::read_rgba_f32`].
    pub async fn read_rgba8(&self, ctx: &GpuContext) -> Result<Vec<u8>, GpuError> {
        self.require_format(ImageFormat::Rgba8Unorm)?;
        let mut output = vec![0_u8; self.sample_count()];
        let row_bytes = self.width as usize * CHANNELS;
        self.read_strips(ctx, |rows, unpad| {
            unpad(&mut output[rows.start as usize * row_bytes..rows.end as usize * row_bytes]);
        })
        .await?;
        Ok(output)
    }

    /// Copies the texture to a staging buffer in strips of rows, and calls
    /// `sink` once for each strip (top to bottom). `sink` gets the rows of
    /// the strip and a function that copies the strip, without the row
    /// padding, into a byte slice of exactly
    /// `rows * width * bytes_per_pixel` bytes.
    async fn read_strips(
        &self,
        ctx: &GpuContext,
        mut sink: impl FnMut(Range<u32>, &mut dyn FnMut(&mut [u8])),
    ) -> Result<(), GpuError> {
        // The width is at most the device limit (16384 or less on current
        // hardware), so `width * 16` rounded up to 256 fits in `u32`.
        let unpadded_u32 = self.width * self.format.bytes_per_pixel();
        let padded_u32 = padded_bytes_per_row(unpadded_u32);
        let padded_bytes_per_row = u64::from(padded_u32);
        let strips = self.strips(padded_bytes_per_row);
        let largest_strip_rows = strips
            .iter()
            .map(|rows| rows.end - rows.start)
            .max()
            .unwrap_or(0);
        let staging_size = padded_bytes_per_row * u64::from(largest_strip_rows);

        let staging = scoped(&ctx.device, || {
            ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("expose-gpu read-back staging"),
                size: staging_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        })
        .await?;

        let unpadded = unpadded_u32 as usize;
        let padded = padded_u32 as usize;

        for rows in strips {
            let strip_bytes = padded_bytes_per_row * u64::from(rows.end - rows.start);
            let submission = scoped(&ctx.device, || {
                let mut encoder =
                    ctx.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("expose-gpu read-back"),
                        });
                encoder.copy_texture_to_buffer(
                    self.copy_info(rows.start),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &staging,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(padded_u32),
                            rows_per_image: Some(rows.end - rows.start),
                        },
                    },
                    self.strip_extent(&rows),
                );
                ctx.queue.submit([encoder.finish()])
            })
            .await?;

            map_for_read(ctx, &staging, strip_bytes, submission).await?;
            {
                let mapped = staging
                    .get_mapped_range(0..strip_bytes)
                    .map_err(|error| GpuError::BufferMap(error.to_string()))?;
                sink(rows, &mut |dst| unpad_rows(&mapped, dst, padded, unpadded));
            }
            staging.unmap();
        }
        Ok(())
    }

    /// Splits the rows into strips of at most [`STRIP_BYTES`] (at least one
    /// row per strip).
    fn strips(&self, bytes_per_row: u64) -> Vec<Range<u32>> {
        let rows_per_strip = (STRIP_BYTES / bytes_per_row).clamp(1, u64::from(self.height));
        // `rows_per_strip <= height`, so the conversion cannot fail.
        let rows_per_strip = u32::try_from(rows_per_strip).unwrap_or(self.height);
        (0..self.height)
            .step_by(rows_per_strip as usize)
            .map(|start| start..(start + rows_per_strip).min(self.height))
            .collect()
    }

    fn copy_info(&self, first_row: u32) -> wgpu::TexelCopyTextureInfo<'_> {
        wgpu::TexelCopyTextureInfo {
            texture: &self.texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: first_row,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        }
    }

    fn strip_extent(&self, rows: &Range<u32>) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.width,
            height: rows.end - rows.start,
            depth_or_array_layers: 1,
        }
    }
}

/// Maps `0..size` of `buffer` for reading, and waits for the map.
async fn map_for_read(
    ctx: &GpuContext,
    buffer: &wgpu::Buffer,
    size: u64,
    submission: wgpu::SubmissionIndex,
) -> Result<(), GpuError> {
    let (sender, receiver) = oneshot::channel();
    buffer.map_async(wgpu::MapMode::Read, 0..size, move |result| {
        sender.send(result);
    });
    // Native: block until the copy is done; this also runs the map callback.
    // Browser: no effect; the event loop runs the callback.
    ctx.device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .map_err(|error| GpuError::Poll(error.to_string()))?;
    match receiver.await {
        Some(Ok(())) => Ok(()),
        Some(Err(error)) => Err(GpuError::BufferMap(error.to_string())),
        None => Err(GpuError::BufferMap(
            "wgpu dropped the map callback without a result".to_owned(),
        )),
    }
}

/// Copies rows from `src` (stride `padded`) to `dst` (stride `unpadded`).
fn unpad_rows(src: &[u8], dst: &mut [u8], padded: usize, unpadded: usize) {
    for (dst_row, src_row) in dst.chunks_exact_mut(unpadded).zip(src.chunks(padded)) {
        dst_row.copy_from_slice(&src_row[..unpadded]);
    }
}

/// Rounds a row size up to the copy alignment (256 bytes).
pub(crate) fn padded_bytes_per_row(unpadded: u32) -> u32 {
    unpadded.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
}

fn sample_count(width: u32, height: u32) -> usize {
    width as usize * height as usize * CHANNELS
}

fn check_length(width: u32, height: u32, actual: usize) -> Result<(), GpuError> {
    let expected = sample_count(width, height);
    if actual == expected {
        Ok(())
    } else {
        Err(GpuError::DataLength { expected, actual })
    }
}

#[cfg(test)]
mod tests {
    use super::{padded_bytes_per_row, unpad_rows};

    #[test]
    fn row_padding_rounds_up_to_256() {
        assert_eq!(padded_bytes_per_row(256), 256);
        assert_eq!(padded_bytes_per_row(37 * 16), 768);
        assert_eq!(padded_bytes_per_row(1), 256);
        assert_eq!(padded_bytes_per_row(6000 * 16), 96_000);
    }

    #[test]
    fn unpad_removes_row_padding() {
        let src = [1, 2, 3, 0, 4, 5, 6, 0];
        let mut dst = [0; 6];
        unpad_rows(&src, &mut dst, 4, 3);
        assert_eq!(dst, [1, 2, 3, 4, 5, 6]);
    }
}
