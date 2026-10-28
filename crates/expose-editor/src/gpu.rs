//! GPU compositor.
//!
//! [`GpuCompositor`] gives the same result as the CPU reference
//! [`composite`](crate::composite::composite), within float tolerance. The
//! model (coverage, pixel layers, adjustment layers, groups, dissolve) is
//! the same. See [`crate::composite`](mod@crate::composite).
//!
//! # Design
//!
//! - The compositor uses the `expose-gpu` kernel API only. All composite
//!   buffers are `rgba32float` textures of the region size with
//!   **premultiplied** alpha. The layer texture cache is `rgba16float`
//!   (see "Texture formats").
//! - Each layer is **one kernel dispatch**. The kernel reads the current
//!   buffer (the backdrop), the layer data, and the mask, and writes a new
//!   buffer. A kernel output cannot be one of its inputs, so the
//!   compositor uses a pool of region-size buffers and takes a new one for
//!   each step. Two buffers are sufficient for a flat layer list. Each group
//!   level adds one or two buffers.
//! - The blend space encode and decode (sRGB transfer curve) occur per
//!   pixel in the kernel. Perceptual blending (the default) thus needs no
//!   extra passes.
//! - Layer pixels and masks come from the layer texture cache (see below).
//!   The kernel gets the canvas position and the size of each texture as
//!   parameters, so a texture can be larger than the region.
//! - Masks are stored as `rgba16float` with **four mask values in each
//!   texel** (texel `x / 4`, channel `x % 4`). This uses the memory of a
//!   one-channel `f16` texture.
//! - Curves use a table of [`CURVE_TABLE_SIZE`] samples for each channel
//!   (the color curve, then the master curve), with linear interpolation.
//!   The other adjustments use the exact formulas. The table (64 KiB) is
//!   in the layer texture cache with the curves that made it. It is
//!   uploaded again only when the curves change.
//! - Free region-size buffers stay in a pool in the compositor, so that the
//!   next call with the same region size does not make new textures.
//!
//! # Layer texture cache
//!
//! The compositor keeps one texture for the pixels and one for the mask of
//! each layer, keyed by [`LayerId`]. Each texture records the content
//! version ([`Layer::pixel_version`], [`Layer::mask_version`]) and the part
//! of the buffer that it holds.
//!
//! - The cached part is the part of the layer (or mask) that is inside the
//!   canvas. Thus a move inside the canvas does not upload. A move of a
//!   layer that is partly outside the canvas changes its canvas part and
//!   uploads it again. If the canvas part is larger than the device texture
//!   limit, only the part inside the region is cached.
//! - A property change (opacity, blend mode, visibility, mask enable) does
//!   not upload. A pixel write or a mask write (also a brush batch, an
//!   undo, and a redo) updates only the changed area of the texture (see
//!   "Partial updates"). A new mask, or a change that the change log does
//!   not know, uploads the texture again (the full canvas part, once).
//! - At each composite, the cache drops the entries of layers that are no
//!   longer in the document, and the textures with an old version that
//!   cannot get a partial update. A deleted layer that comes back by undo
//!   is uploaded again.
//! - Content versions are unique in the process (see [`crate::version`]),
//!   so one compositor can serve many documents and clones.
//!
//! # View rendering
//!
//! [`GpuCompositor::render_view`] keeps the composite of the full canvas
//! and a pyramid of 2 x 2 means of it on the GPU. It composites again only
//! when [`Document::revision`] changes. A pan or a zoom then runs one
//! kernel of the view size: nearest neighbor for zoom 1 or more, an area
//! mean for zoom less than 1 (from the pyramid level on which one output
//! pixel covers 1 to 2 texels). The output has straight alpha.
//!
//! Limit: the pyramid levels have the canvas size divided by `2^k`,
//! rounded up. At the right and bottom canvas edge, the last texel of a
//! level can include up to `2^k - 1` canvas pixels of transparent area.
//! Output pixels at that edge can thus be slightly more transparent than
//! the exact area mean.
//!
//! # Mask view
//!
//! [`GpuCompositor::render_mask_view`] shows the mask of one layer as an
//! opaque gray image, with the same view geometry and sampling as
//! [`GpuCompositor::render_view`]. One kernel makes the gray image of the
//! full canvas from the canvas part of the mask (the mask fill outside the
//! mask buffer). The compositor keeps this image and its pyramid for one
//! mask, with the key: layer ID, [`Layer::mask_version`], mask offset,
//! mask fill, and canvas size. A pan or a zoom thus runs only the view
//! kernel. This cache is separate from the composite view cache, so a
//! change between the two views does not composite or upload again. Both
//! views write to the same view output texture.
//!
//! # Below snapshot
//!
//! A slider step changes one property of one layer. The composite of the
//! layers below that layer does not change. Thus the view composite keeps
//! a "below" snapshot: the composite of the bottom `k` top-level layers,
//! with a key for each of these layers.
//!
//! - A layer key holds all data of the layer that changes its step: the
//!   ID, the content versions (they stand for the pixel and mask values),
//!   the properties, the mask offset, enable flag, and fill, the
//!   adjustment parameters, and for a group the isolation and the keys of
//!   all children. The canvas size and the blend space are also part of
//!   the snapshot key.
//! - The composite starts from the snapshot only if the snapshot keys are
//!   equal to the keys of the bottom `k` layers now. Any change below
//!   (also a new, deleted, or moved layer) thus discards the snapshot.
//! - The snapshot index `k` is the lowest top-level layer that changed
//!   since the last view composite. The first step on a layer composites
//!   the full stack and keeps the state below the layer. The next steps on
//!   the same layer composite only that layer and the layers above it.
//! - A change in a group changes the key of its top-level group, so the
//!   snapshot is the state below the top-level group.
//! - Only [`GpuCompositor::render_view`] uses the snapshot.
//!   [`GpuCompositor::composite_gpu`] always composites all layers.
//!
//! The result is the same as a full composite: the same kernels run on
//! the same buffers in the same order.
//!
//! # Partial updates
//!
//! Each layer has a change log for its pixel values and one for its mask
//! values (the private module `changes`). The log records the changed rectangle
//! of each content version (brush batches, pixel and mask patches and
//! tiles, and their undo and redo).
//!
//! - **Layer textures.** If a cached texture has an old version and the
//!   log knows the changes after that version, the cache uploads only the
//!   union of the changed rectangles (inside the cached part). The upload
//!   uses the same CPU conversion to `f16` as a full upload. For masks,
//!   the area extends to whole texels (four values). Otherwise (a new
//!   mask, a cancelled stroke, more than 64 changes, a different
//!   cached part), the cache uploads the full canvas part as before.
//! - **View composite.** [`GpuCompositor::render_view`] compares the layer
//!   keys with the keys of its last composite. If only the pixel or the
//!   mask values of one layer changed, the log knows the changed area, and
//!   the area is at most half of the canvas, it composites all layers only
//!   in that canvas area, puts the result into the old composite, and
//!   builds the pyramid again. All composite steps work per pixel, so the
//!   result is bit-equal to a full composite. A change outside the canvas
//!   does not composite. All other changes use the full composite (with
//!   the "below" snapshot).
//! - `expose-gpu` has no upload to a part of a texture, and a kernel cannot
//!   write to one of its inputs. Thus a partial update uploads the changed
//!   area as a small texture, and the patch kernel copies the old texture
//!   with the small texture in the changed area to a second texture of the
//!   same size. The old texture becomes a spare (the layer cache keeps at
//!   most two spares; the view composite uses the buffer pool). The copy
//!   reads and writes the full texture: measured at 4096 x 4096, about
//!   2.6 ms for a layer texture, 4.8 ms for the view composite, and 3.5 ms
//!   for the pyramid build.
//!
//! # Texture formats
//!
//! All kernels read their inputs as `f32` and do all math in `f32`. Only
//! the storage format changes from texture to texture:
//!
//! | Texture                                   | Format        | Reason |
//! | ----------------------------------------- | ------------- | ------ |
//! | Layer pixels (layer texture cache)        | `rgba16float` | The largest part of the memory. Each value is read once in each composite, so the rounding does not add up. |
//! | Masks (layer texture cache)               | `rgba16float` | As the layer pixels. Mask values are in `0..=1` (absolute error at most `2^-12`). |
//! | Composite buffers (pool)                  | `rgba32float` | The working buffers: each step reads and writes them, so `f16` rounding would add up from layer to layer. |
//! | View composite                            | `rgba32float` | It is a composite buffer that moves to the view cache without a copy. In `f16`, the free buffer would stay in the pool, and the memory would not decrease. |
//! | "Below" snapshot                          | `rgba32float` | As the view composite. Also, the result of a composite from the snapshot stays bit-equal to a full composite. |
//! | View pyramid, mask view image and pyramid | `rgba16float` | Only for the display. |
//! | Curve tables, view output, placeholder    | `rgba32float` | Small. The view output format is part of the API. |
//!
//! - The upload converts `f32` to the nearest `f16` on the CPU (the
//!   `expose-gpu` upload of [`ImageFormat::Rgba16Float`]). This sends half
//!   the bytes to the GPU. A conversion in a kernel needs the `f32` upload
//!   first, so it cannot be faster. Measured for a 4096 x 4096 layer: about
//!   46 ms for `rgba32float`, about 42 ms for `rgba16float` (conversion
//!   included).
//! - A kernel store of an `f32` value to `rgba16float` can round toward
//!   zero on some GPUs (measured on Apple GPUs). The kernels that write
//!   `rgba16float` (pyramid levels, mask view image) thus round to the
//!   nearest `f16` value with `quantizeToF16` first.
//! - Layer values outside the finite `f16` range (larger than 65504)
//!   become infinite. Values below about `6.1e-5` lose relative
//!   precision (absolute error at most about `3e-8`).
//!
//! # Memory
//!
//! For a canvas of `N` pixels, the compositor keeps about:
//!
//! - for each pixel layer: 8 bytes for each pixel of its canvas part
//!   (`8 N` for a canvas-size layer);
//! - for each mask: 2 bytes for each pixel of its canvas part;
//! - after partial updates: at most two spare layer textures (for example
//!   `8 N` for a canvas-size layer that a brush changes), and the free
//!   buffers of the last patch composite (16 bytes for each pixel of the
//!   changed area);
//! - free composite buffers: `16 N` each, normally one or two (one more
//!   for each level of nested groups);
//! - with [`GpuCompositor::render_view`]: the composite (`16 N`), the
//!   pyramid (about `8 N / 3`), the "below" snapshot (`16 N`, at most
//!   one), and the view output (16 bytes for each output pixel). While a
//!   snapshot is in use, the pool can hold one more free buffer.
//! - with [`GpuCompositor::render_mask_view`]: the gray image (`8 N`) and
//!   its pyramid (about `8 N / 3`), for one mask.
//!   [`GpuCompositor::clear_mask_view_cache`] frees only these.
//!
//! Example: 4096 x 4096 (`16 N` = 256 MiB) with 6 canvas-size layers, 2
//! smaller layers, one mask, the view composite, a snapshot, and a 2560 x
//! 1600 view output: about 1.7 GiB (2.5 GiB with all textures in
//! `rgba32float`).
//! [`GpuCompositor::cache_memory_bytes`] reports the current value, and
//! [`GpuCompositor::clear_cache`] frees all of it (for example, when the
//! editor closes a document).
//!
//! # Differences from the CPU reference
//!
//! - The layer texture cache rounds each layer and mask value to the
//!   nearest `f16` value (at most `2^-12` relative). The effect on the
//!   composite, measured with `f32` values that are not `f16` values, in
//!   `|gpu - cpu| / max(1, |cpu|)` for each sample: at most about 8.8e-4
//!   for the separable blend modes, and about 3.0e-3 for hue, saturation,
//!   color, and luminosity (the hue and saturation of a color near gray
//!   change much for a small change of the values). Adjustments with a
//!   large slope (for example levels with a small gamma) make the error
//!   larger by their slope. The test tolerances are `2e-3` and `6e-3`.
//! - The other tests use layer values that are `f16` values, so they test
//!   only the composite math. GPU float math (for example `pow`, division,
//!   fused multiply-add) can differ from the CPU in the last bits. These
//!   tests use a tolerance of `5e-5 * max(1, |value|)` for each sample
//!   (measured: at most about 1.3e-5). The divide mode uses `5e-4`,
//!   because a small divisor makes the rounding error larger (measured: at
//!   most about 1.1e-4).
//! - The view pyramid levels and the mask view image are `rgba16float`.
//!   Each level adds one rounding to the nearest `f16` value (the tests
//!   use `5e-4`; measured: at most about 2.4e-4).
//! - Curves: the table interpolation adds an error of less than about
//!   `1e-6` where the curve is smooth. At a curve end point that is not on
//!   a table sample (`x` not a multiple of `1 / 4096`), the error is at
//!   most about `slope / 16384` in the encoded values (the decode to
//!   linear multiplies it by up to about 2.3). Measured with slope 1.25
//!   near white: 1.7e-4 in linear values.
//! - Steps (hard mix, threshold contrast, dissolve, color selection, and
//!   the branches of some blend modes) can give a different result when a
//!   value is within float error of the step.

use std::collections::{HashMap, HashSet};

use bytemuck::{Pod, Zeroable};
use expose_core::buffer::{Rect, WorkingImage};
use expose_gpu::{CHANNELS, GpuContext, GpuImage, ImageFormat, Kernel};

use crate::adjust::{Adjustment, Curves, MonotoneCubic, Prepared, PreparedAdjustment};
use crate::blend::{BlendMode, BlendSpace};
use crate::changes::Changes;
use crate::document::Document;
use crate::error::{EditorError, EditorResult};
use crate::layer::{
    GroupIsolation, Layer, LayerId, LayerKind, LayerProps, Mask, MaskFill, MaskImage,
};

/// The number of samples in the curve table of a curves adjustment. The
/// sample `i` is at `x = i / 4096`, so the usual curve points (for example
/// 0, 0.25, 0.5) are on samples.
pub const CURVE_TABLE_SIZE: u32 = 4097;

/// The GPU format of the composite buffers (also the view composite and
/// the "below" snapshot), the curve tables, the view output, and the
/// placeholder. See "Texture formats" in the module documentation.
const FORMAT: ImageFormat = ImageFormat::Rgba32Float;

/// The GPU format of the layer texture cache (layer pixels and masks).
/// The kernels read the values as `f32` and do all math in `f32`.
const LAYER_FORMAT: ImageFormat = ImageFormat::Rgba16Float;

/// The GPU format of the view pyramids and of the mask view image. These
/// textures are only for the display.
const VIEW_FORMAT: ImageFormat = ImageFormat::Rgba16Float;

/// Blends a source image (pixel layer or isolated group) onto the backdrop.
const BLEND_KERNEL: Kernel = Kernel {
    name: "expose-editor/blend-layer",
    source: concat!(
        include_str!("shaders/common.wgsl"),
        include_str!("shaders/blend_layer.wgsl")
    ),
};

/// Applies an adjustment layer.
const ADJUST_KERNEL: Kernel = Kernel {
    name: "expose-editor/adjust",
    source: concat!(
        include_str!("shaders/common.wgsl"),
        include_str!("shaders/adjust.wgsl")
    ),
};

/// Mixes a pass-through group result with the state before the group.
const PASS_MIX_KERNEL: Kernel = Kernel {
    name: "expose-editor/pass-mix",
    source: concat!(
        include_str!("shaders/common.wgsl"),
        include_str!("shaders/pass_mix.wgsl")
    ),
};

/// Fills a buffer with transparent black.
const CLEAR_KERNEL: Kernel = Kernel {
    name: "expose-editor/clear",
    source: concat!(
        include_str!("shaders/common.wgsl"),
        include_str!("shaders/clear.wgsl")
    ),
};

/// Makes the next level of the view pyramid (2 x 2 mean).
const MIP_KERNEL: Kernel = Kernel {
    name: "expose-editor/mip-down",
    source: include_str!("shaders/mip_down.wgsl"),
};

/// Renders a view from one level of the view pyramid.
const VIEW_KERNEL: Kernel = Kernel {
    name: "expose-editor/view",
    source: include_str!("shaders/view.wgsl"),
};

/// Renders the mask of one layer as an opaque gray image of the canvas.
const MASK_VIEW_KERNEL: Kernel = Kernel {
    name: "expose-editor/mask-view",
    source: concat!(
        include_str!("shaders/common.wgsl"),
        include_str!("shaders/mask_view.wgsl")
    ),
};

/// Copies a texture and replaces one rectangle with a patch texture (see
/// "Partial updates" in the module documentation).
const PATCH_KERNEL: Kernel = Kernel {
    name: "expose-editor/patch",
    source: include_str!("shaders/patch.wgsl"),
};

/// The parameter block of [`PATCH_KERNEL`]. It agrees with `PatchParams`
/// in `shaders/patch.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct PatchParams {
    /// Texel position and size of the patch in the output.
    rect: [i32; 4],
}

/// Adjustment kinds in the kernel (`params.flags.w`).
const ADJUST_BRIGHTNESS_CONTRAST: u32 = 0;
const ADJUST_LEVELS: u32 = 1;
const ADJUST_CURVES: u32 = 2;
const ADJUST_HUE_SATURATION: u32 = 3;

/// The kernel parameter block. It agrees with `Params` in
/// `shaders/common.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Params {
    /// Canvas position and size of the region.
    region: [i32; 4],
    /// Canvas position and size of the auxiliary image (blend kernel).
    source: [i32; 4],
    /// Canvas position and size (in mask pixels) of the uploaded mask part.
    mask: [i32; 4],
    /// Blend mode ID, blend space (0 = linear, 1 = perceptual), mask flag,
    /// adjustment kind.
    flags: [u32; 4],
    /// Coverage base (opacity * fill), mask fill value, unused, unused.
    coef: [f32; 4],
    /// Adjustment data (see `shaders/adjust.wgsl`).
    adj: [[f32; 4]; 5],
}

/// Returns the kernel ID of a blend mode: its index in [`BlendMode::ALL`].
const fn mode_id(mode: BlendMode) -> u32 {
    match mode {
        BlendMode::Normal => 0,
        BlendMode::Dissolve => 1,
        BlendMode::Darken => 2,
        BlendMode::Multiply => 3,
        BlendMode::ColorBurn => 4,
        BlendMode::LinearBurn => 5,
        BlendMode::DarkerColor => 6,
        BlendMode::Lighten => 7,
        BlendMode::Screen => 8,
        BlendMode::ColorDodge => 9,
        BlendMode::LinearDodge => 10,
        BlendMode::LighterColor => 11,
        BlendMode::Overlay => 12,
        BlendMode::SoftLight => 13,
        BlendMode::HardLight => 14,
        BlendMode::VividLight => 15,
        BlendMode::LinearLight => 16,
        BlendMode::PinLight => 17,
        BlendMode::HardMix => 18,
        BlendMode::Difference => 19,
        BlendMode::Exclusion => 20,
        BlendMode::Subtract => 21,
        BlendMode::Divide => 22,
        BlendMode::Hue => 23,
        BlendMode::Saturation => 24,
        BlendMode::Color => 25,
        BlendMode::Luminosity => 26,
    }
}

const fn space_id(space: BlendSpace) -> u32 {
    match space {
        BlendSpace::Linear => 0,
        BlendSpace::Perceptual => 1,
    }
}

/// Converts a canvas coordinate or size to `i32` for the kernel.
fn to_i32(v: u32) -> EditorResult<i32> {
    // Canvas values are at most `MAX_DIMENSION`, so this does not fail for
    // a valid document.
    i32::try_from(v).map_err(|_| EditorError::OffsetOverflow)
}

fn rect_param(r: Rect) -> EditorResult<[i32; 4]> {
    Ok([
        to_i32(r.x)?,
        to_i32(r.y)?,
        to_i32(r.width)?,
        to_i32(r.height)?,
    ])
}

/// Returns [`EditorError::InvalidRect`] if `region` is empty or not fully
/// inside the canvas (the same check as the CPU compositor).
fn check_region(document: &Document, region: Rect) -> EditorResult<()> {
    if region.is_empty() || !document.bounds().contains_rect(&region) {
        return Err(EditorError::InvalidRect {
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
        });
    }
    Ok(())
}

/// The smallest permitted [`ViewSpec::scale`] (1/65536).
pub const MIN_VIEW_SCALE: f64 = 1.0 / 65_536.0;

/// The largest permitted [`ViewSpec::scale`] (65536).
pub const MAX_VIEW_SCALE: f64 = 65_536.0;

/// The largest permitted absolute value of [`ViewSpec::origin_x`] and
/// [`ViewSpec::origin_y`], in canvas pixels. It keeps the integer part of
/// the origin in the `i32` range of the kernel.
pub const MAX_VIEW_ORIGIN: f64 = 1.0e9;

/// A view of the canvas: the part of the composite that an editor canvas
/// shows, at a zoom level. See [`GpuCompositor::render_view`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewSpec {
    /// Canvas x coordinate at the left edge of the output.
    pub origin_x: f64,
    /// Canvas y coordinate at the top edge of the output.
    pub origin_y: f64,
    /// Output pixels for each canvas pixel. 1 is 100 %, 0.5 is 50 %.
    /// Must be finite and in [`MIN_VIEW_SCALE`]..=[`MAX_VIEW_SCALE`].
    pub scale: f64,
    /// Output width in pixels, `1..=` the device texture limit.
    pub width: u32,
    /// Output height in pixels, `1..=` the device texture limit.
    pub height: u32,
}

/// The parameter block of the view kernel. It agrees with `ViewParams` in
/// `shaders/view.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct ViewParams {
    /// Integer part of the origin in level texels (x, y), level size.
    base: [i32; 4],
    /// Fraction of the origin (x, y), texels for each output pixel, mode
    /// (0 = nearest, 1 = area mean).
    frac: [f32; 4],
}

/// Checks a view specification.
fn check_view(view: &ViewSpec, max_dim: u32) -> EditorResult<()> {
    if !view.scale.is_finite() || view.scale < MIN_VIEW_SCALE || view.scale > MAX_VIEW_SCALE {
        return Err(EditorError::InvalidView(
            "scale must be finite and in MIN_VIEW_SCALE..=MAX_VIEW_SCALE",
        ));
    }
    if !view.origin_x.is_finite()
        || !view.origin_y.is_finite()
        || view.origin_x.abs() > MAX_VIEW_ORIGIN
        || view.origin_y.abs() > MAX_VIEW_ORIGIN
    {
        return Err(EditorError::InvalidView(
            "origin must be finite and at most MAX_VIEW_ORIGIN from 0",
        ));
    }
    if view.width == 0 || view.height == 0 {
        return Err(EditorError::InvalidView("output size must not be 0"));
    }
    if view.width > max_dim || view.height > max_dim {
        return Err(EditorError::InvalidView(
            "output size is larger than the device texture limit",
        ));
    }
    Ok(())
}

/// Returns the pyramid level for a view scale: 0 for a scale of 1 or more,
/// else the largest `k` with `scale * 2^k <= 1`. At level `k`, one output
/// pixel then covers `1 / (scale * 2^k)` texels, which is in `1..2`.
fn view_level(scale: f64) -> usize {
    let mut level = 0;
    let mut covered = scale;
    // `scale >= MIN_VIEW_SCALE = 2^-16`, so the loop stops after at most
    // 16 steps.
    while covered * 2.0 <= 1.0 {
        covered *= 2.0;
        level += 1;
    }
    level
}

/// Builds the view kernel parameters for a source level of `size` texels.
/// `level_scale` is `2^level` (canvas pixels for each texel).
fn view_params(
    view: &ViewSpec,
    level_scale: f64,
    size: (u32, u32),
    nearest: bool,
) -> EditorResult<ViewParams> {
    let ox = view.origin_x / level_scale;
    let oy = view.origin_y / level_scale;
    let (bx, by) = (ox.floor(), oy.floor());
    // `check_view` limits the origin to MAX_VIEW_ORIGIN (1e9 < 2^31), and
    // `level_scale >= 1`, so the integer parts fit in i32. The fractions
    // are in 0..1 and the step is at most 2^16 (scale >= 2^-16), so the
    // f32 values are finite.
    #[allow(clippy::cast_possible_truncation)]
    let params = ViewParams {
        base: [bx as i32, by as i32, to_i32(size.0)?, to_i32(size.1)?],
        frac: [
            (ox - bx) as f32,
            (oy - by) as f32,
            (1.0 / (view.scale * level_scale)) as f32,
            if nearest { 0.0 } else { 1.0 },
        ],
    };
    Ok(params)
}

/// One cached texture: a part of the pixels of a layer, or of its mask.
#[derive(Debug)]
struct CachedPart {
    /// The content version of the values ([`Layer::pixel_version`] or
    /// [`Layer::mask_version`]).
    version: u64,
    /// The uploaded part, in layer (or mask) pixel coordinates.
    rect: Rect,
    /// The values (masks: four values in each texel).
    image: GpuImage,
}

/// The curve table of a curves adjustment layer.
#[derive(Debug)]
struct CurveTable {
    /// The curves of the table.
    curves: Curves,
    /// [`CURVE_TABLE_SIZE`] x 1 texels: red, green, blue, unused.
    image: GpuImage,
}

/// The cached textures of one layer.
#[derive(Debug, Default)]
struct LayerEntry {
    pixels: Option<CachedPart>,
    mask: Option<CachedPart>,
    curve: Option<CurveTable>,
}

/// The layer texture cache. See the module documentation.
#[derive(Debug, Default)]
struct LayerCache {
    entries: HashMap<LayerId, LayerEntry>,
    /// Free layer-format textures for partial updates (at most
    /// [`MAX_SPARES`]).
    spares: Vec<GpuImage>,
    /// The number of full pixel and mask uploads since the compositor was
    /// made.
    uploads: u64,
    /// The number of partial pixel and mask updates since the compositor
    /// was made.
    partial_uploads: u64,
    /// If `true`, the cache and the view composite do no partial updates.
    /// Only the tests and the benchmark set it, to compare with the full
    /// updates.
    no_partial: bool,
    /// The number of curve table uploads since the compositor was made.
    curve_uploads: u64,
}

/// The cached composite of the full canvas and its view pyramid.
#[derive(Debug)]
struct ViewCache {
    /// The document revision of the composite.
    revision: u64,
    /// The premultiplied composite of the full canvas.
    composite: GpuImage,
    /// The pyramid: `mips[0]` is the composite halved (sizes rounded up),
    /// `mips[k + 1]` is `mips[k]` halved. The last level is 1 x 1.
    mips: Vec<GpuImage>,
}

/// The key of the mask view cache: all data that changes the gray image
/// of a mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MaskViewKey {
    layer: LayerId,
    /// [`Layer::mask_version`] (the mask values).
    version: u64,
    /// The mask offset and fill. They do not change the mask version.
    offset: (i32, i32),
    fill: MaskFill,
    /// The canvas size.
    size: (u32, u32),
}

/// The cached gray image of one mask and its view pyramid, for
/// [`GpuCompositor::render_mask_view`].
#[derive(Debug)]
struct MaskViewCache {
    key: MaskViewKey,
    /// The gray image of the full canvas (opaque) in `composite`, and its
    /// pyramid. `revision` is not used.
    view: ViewCache,
}

/// All data of a layer (and of its children) that changes its composite
/// step, without the pixel and mask values. The content versions stand
/// for the values: they are unique stamps in the process (see
/// [`crate::version`]), so equal keys give equal steps, also for clones
/// and other documents.
///
/// [`layer_key`] destructures the property structs fully, so a new field
/// gives a compile error there until the key includes it (or a comment
/// tells why the composite does not use it).
#[derive(Debug, Clone, PartialEq)]
struct LayerKey {
    id: LayerId,
    pixel_version: u64,
    mask_version: u64,
    visible: bool,
    /// The bits of the opacity and of the fill opacity. A compare of the
    /// bits is exact (also for `-0.0` and NaN).
    opacity: u32,
    fill_opacity: u32,
    blend_mode: BlendMode,
    /// Mask offset, enable flag, and fill (the values: `mask_version`).
    mask: Option<((i32, i32), bool, MaskFill)>,
    kind: KindKey,
}

/// The content part of a [`LayerKey`].
#[derive(Debug, Clone, PartialEq)]
enum KindKey {
    /// The offset (the values: `pixel_version`).
    Pixel((i32, i32)),
    /// The parameters. A float compare can only give "not equal" for equal
    /// steps (NaN), which only makes the cache miss.
    Adjustment(Adjustment),
    Group {
        isolation: GroupIsolation,
        children: Vec<LayerKey>,
    },
}

/// Returns the key of `layer` and its children.
fn layer_key(layer: &Layer) -> LayerKey {
    let LayerProps {
        // The name and the locks do not change the composite.
        name: _,
        visible,
        opacity,
        fill_opacity,
        blend_mode,
        locks: _,
    } = layer.props();
    let mask = layer.mask().map(|m| {
        let Mask {
            image: _,
            offset,
            enabled,
            fill,
        } = m;
        (*offset, *enabled, *fill)
    });
    let kind = match layer.kind() {
        LayerKind::Pixel(p) => KindKey::Pixel(p.offset()),
        LayerKind::Adjustment(a) => KindKey::Adjustment(a.clone()),
        LayerKind::Group(g) => KindKey::Group {
            isolation: g.isolation(),
            children: g.children().iter().map(layer_key).collect(),
        },
    };
    LayerKey {
        id: layer.id(),
        pixel_version: layer.pixel_version(),
        mask_version: layer.mask_version(),
        visible: *visible,
        opacity: opacity.to_bits(),
        fill_opacity: fill_opacity.to_bits(),
        blend_mode: *blend_mode,
        mask,
        kind,
    }
}

/// One content change of one layer: its pixel values or its mask values
/// changed, and nothing else in the layer stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentChange {
    /// The pixel values of layer `id` changed after pixel version `old`.
    Pixels { id: LayerId, old: u64 },
    /// The mask values of layer `id` changed after mask version `old`.
    Mask { id: LayerId, old: u64 },
}

/// The difference between two layer key lists (see [`diff_lists`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyDiff {
    /// The keys are equal.
    Same,
    /// Only one content of one layer changed.
    One(ContentChange),
    /// Other changes, or more than one change.
    Other,
}

impl KeyDiff {
    /// Combines the differences of two disjoint parts of a layer stack.
    const fn and(self, other: Self) -> Self {
        match (self, other) {
            (d, Self::Same) | (Self::Same, d) => d,
            _ => Self::Other,
        }
    }
}

/// Compares the key lists `old` and `new` of one layer list.
fn diff_lists(old: &[LayerKey], new: &[LayerKey]) -> KeyDiff {
    if old.len() != new.len() {
        return KeyDiff::Other;
    }
    let mut diff = KeyDiff::Same;
    for (a, b) in old.iter().zip(new) {
        diff = diff.and(diff_key(a, b));
        if diff == KeyDiff::Other {
            break;
        }
    }
    diff
}

/// Compares the keys of one layer (and of its children). The function
/// destructures the key fully, so a new field gives a compile error here.
fn diff_key(old: &LayerKey, new: &LayerKey) -> KeyDiff {
    let LayerKey {
        id,
        pixel_version,
        mask_version,
        visible,
        opacity,
        fill_opacity,
        blend_mode,
        mask,
        kind,
    } = old;
    let same_props = *id == new.id
        && *visible == new.visible
        && *opacity == new.opacity
        && *fill_opacity == new.fill_opacity
        && *blend_mode == new.blend_mode
        && *mask == new.mask;
    if !same_props {
        return KeyDiff::Other;
    }
    let children = match (kind, &new.kind) {
        (
            KindKey::Group {
                isolation: a,
                children: ca,
            },
            KindKey::Group {
                isolation: b,
                children: cb,
            },
        ) if a == b => diff_lists(ca, cb),
        (a, b) if a == b => KeyDiff::Same,
        _ => KeyDiff::Other,
    };
    let own = match (
        *pixel_version != new.pixel_version,
        *mask_version != new.mask_version,
    ) {
        (false, false) => KeyDiff::Same,
        (true, false) => KeyDiff::One(ContentChange::Pixels {
            id: *id,
            old: *pixel_version,
        }),
        (false, true) => KeyDiff::One(ContentChange::Mask {
            id: *id,
            old: *mask_version,
        }),
        (true, true) => KeyDiff::Other,
    };
    own.and(children)
}

/// Returns the canvas area of a change `rect` of a buffer at `offset`,
/// clipped to `canvas`. Returns [`Changes::Unknown`] if the position
/// overflows, and [`Changes::Nothing`] if the area is outside the canvas.
fn changed_canvas_part(canvas: Rect, offset: (i32, i32), rect: Rect) -> Changes {
    let position = i32::try_from(rect.x)
        .ok()
        .and_then(|x| offset.0.checked_add(x))
        .zip(
            i32::try_from(rect.y)
                .ok()
                .and_then(|y| offset.1.checked_add(y)),
        );
    let Some((x, y)) = position else {
        return Changes::Unknown;
    };
    canvas
        .intersection_signed(x, y, rect.width, rect.height)
        .map_or(Changes::Nothing, Changes::Area)
}

/// Returns the canvas area that `change` changed in the composite.
/// [`Changes::Unknown`] means that the change log does not know the old
/// version (or the layer is not as expected).
///
/// All composite steps work per pixel, so a change of the values of one
/// layer changes the composite only in the canvas area of the changed
/// values.
fn changed_canvas_area(document: &Document, change: ContentChange) -> Changes {
    let canvas = document.bounds();
    let found = match change {
        ContentChange::Pixels { id, old } => document.layer(id).and_then(|layer| {
            let pixels = layer.as_pixel()?;
            Some((pixels.offset(), layer.pixel_changes_since(old)))
        }),
        ContentChange::Mask { id, old } => document.layer(id).and_then(|layer| {
            let mask = layer.mask()?;
            Some((mask.offset, layer.mask_changes_since(old)))
        }),
    };
    match found {
        None => Changes::Unknown,
        Some((offset, Changes::Area(rect))) => changed_canvas_part(canvas, offset, rect),
        Some((_, changes)) => changes,
    }
}

/// The composite of the bottom top-level layers of the canvas: the
/// "below" snapshot of [`GpuCompositor::render_view`].
#[derive(Debug)]
struct Snapshot {
    /// The keys of the top-level layers in the snapshot (the bottom
    /// `keys.len()` layers).
    keys: Vec<LayerKey>,
    /// Their premultiplied composite of the full canvas.
    image: GpuImage,
}

/// The state of the "below" snapshot. See the module documentation.
#[derive(Debug)]
struct BelowCache {
    /// The canvas size and the blend space of `last` and `snapshot`.
    size: (u32, u32),
    space: BlendSpace,
    /// The keys of the top-level layers at the last view composite.
    last: Vec<LayerKey>,
    snapshot: Option<Snapshot>,
}

/// The GPU compositor.
///
/// Make one compositor for each [`GpuContext`] and use it for all
/// composites. The compositor keeps GPU memory between calls: the layer
/// texture cache, free composite buffers, and the cached view composite
/// (see the module documentation). [`GpuCompositor::clear_cache`] frees
/// it. The kernel pipelines are compiled on the first use and stay in the
/// context cache.
#[derive(Debug)]
pub struct GpuCompositor {
    /// A 1 x 1 texture for kernel inputs that a step does not use.
    placeholder: GpuImage,
    /// Layer textures.
    cache: LayerCache,
    /// Free composite buffers. All have the size of the last composite
    /// region.
    pool: Vec<GpuImage>,
    /// The composite of the full canvas for [`GpuCompositor::render_view`].
    view: Option<ViewCache>,
    /// The output of the last [`GpuCompositor::render_view`] or
    /// [`GpuCompositor::render_mask_view`].
    view_output: Option<GpuImage>,
    /// The number of view composites since the compositor was made.
    view_builds: u64,
    /// The "below" snapshot for [`GpuCompositor::render_view`].
    below: Option<BelowCache>,
    /// The number of view composites that started from the snapshot.
    view_partial_builds: u64,
    /// Free buffers for the patch composites of
    /// [`GpuCompositor::render_view`] (the size of the last patch).
    patch_pool: Vec<GpuImage>,
    /// The number of view composites that composited only a changed area.
    view_patch_builds: u64,
    /// The gray image of the last mask for
    /// [`GpuCompositor::render_mask_view`].
    mask_view: Option<MaskViewCache>,
    /// The number of mask view builds since the compositor was made.
    mask_view_builds: u64,
}

impl GpuCompositor {
    /// Makes a compositor for `ctx`.
    pub async fn new(ctx: &GpuContext) -> EditorResult<Self> {
        Ok(Self {
            placeholder: GpuImage::new(ctx, 1, 1, FORMAT).await?,
            cache: LayerCache::default(),
            pool: Vec::new(),
            view: None,
            view_output: None,
            view_builds: 0,
            below: None,
            view_partial_builds: 0,
            patch_pool: Vec::new(),
            view_patch_builds: 0,
            mask_view: None,
            mask_view_builds: 0,
        })
    }

    /// Frees all cached GPU memory: layer textures, free buffers, the view
    /// composite, its pyramid, and the "below" snapshot, the mask view
    /// image and its pyramid, and the view output. The next call uploads
    /// and composites again.
    pub fn clear_cache(&mut self) {
        self.cache.entries.clear();
        self.cache.spares.clear();
        self.pool.clear();
        self.patch_pool.clear();
        self.view = None;
        self.view_output = None;
        self.below = None;
        self.mask_view = None;
    }

    /// Frees only the mask view image and its pyramid (for example, when
    /// the editor stops to show a mask). The composite view cache stays.
    pub fn clear_mask_view_cache(&mut self) {
        self.mask_view = None;
    }

    /// Returns the number of mask view builds (mask upload, gray image,
    /// and pyramid) that [`GpuCompositor::render_mask_view`] made since
    /// the compositor was made. Use it to check the cache (for example, a
    /// pan must not build again).
    pub const fn mask_view_builds(&self) -> u64 {
        self.mask_view_builds
    }

    /// Returns the number of layer and mask uploads since the compositor
    /// was made. Use it to check the cache (for example, a change of the
    /// opacity must not upload).
    pub const fn layer_uploads(&self) -> u64 {
        self.cache.uploads
    }

    /// Returns the number of partial layer and mask updates since the
    /// compositor was made. A partial update uploads only the area that
    /// changed after the cached version (see "Partial updates" in the
    /// module documentation). [`Self::layer_uploads`] does not count them.
    pub const fn layer_partial_uploads(&self) -> u64 {
        self.cache.partial_uploads
    }

    /// Returns the number of curve table uploads since the compositor was
    /// made. A curves layer uploads its table only when its curves change.
    pub const fn curve_uploads(&self) -> u64 {
        self.cache.curve_uploads
    }

    /// Returns the number of full-canvas composites that
    /// [`GpuCompositor::render_view`] made since the compositor was made.
    /// Use it to check the cache (for example, a pan must not composite).
    pub const fn view_composites(&self) -> u64 {
        self.view_builds
    }

    /// Returns the number of the view composites of
    /// [`Self::view_composites`] that started from the "below" snapshot
    /// (they did not composite the unchanged bottom layers again).
    pub const fn view_partial_composites(&self) -> u64 {
        self.view_partial_builds
    }

    /// Returns the number of the view composites of
    /// [`Self::view_composites`] that composited only the changed area of
    /// one layer (see "Partial updates" in the module documentation).
    pub const fn view_patch_composites(&self) -> u64 {
        self.view_patch_builds
    }

    /// Returns the approximate GPU memory that the compositor keeps
    /// between calls, in bytes (all textures, the placeholder included).
    pub fn cache_memory_bytes(&self) -> u64 {
        let bytes = |image: &GpuImage| {
            u64::from(image.width())
                * u64::from(image.height())
                * u64::from(image.format().bytes_per_pixel())
        };
        let parts = self.cache.entries.values().flat_map(|e| {
            e.pixels
                .iter()
                .chain(e.mask.iter())
                .map(|part| &part.image)
                .chain(e.curve.iter().map(|c| &c.image))
                .map(bytes)
        });
        let spares = self.cache.spares.iter().map(bytes);
        let view = self
            .view
            .iter()
            .chain(self.mask_view.iter().map(|m| &m.view))
            .flat_map(|v| std::iter::once(&v.composite).chain(v.mips.iter()))
            .map(bytes);
        let below = self
            .below
            .iter()
            .flat_map(|b| b.snapshot.iter())
            .map(|s| bytes(&s.image));
        bytes(&self.placeholder)
            + below.sum::<u64>()
            + parts.sum::<u64>()
            + spares.sum::<u64>()
            + self.pool.iter().map(bytes).sum::<u64>()
            + self.patch_pool.iter().map(bytes).sum::<u64>()
            + view.sum::<u64>()
            + self.view_output.iter().map(bytes).sum::<u64>()
    }

    /// Composites the visible layers of `document` in `region` (canvas
    /// coordinates) on the GPU. Returns a premultiplied `rgba32float` image
    /// of the region size.
    ///
    /// The layer texture cache supplies the layer pixels and masks: only
    /// layers with changed values are uploaded. The function does not use
    /// or change the view composite of [`GpuCompositor::render_view`].
    ///
    /// The function submits the work and returns. A read-back of the
    /// result waits for the GPU.
    ///
    /// # Errors
    ///
    /// - [`EditorError::InvalidRect`] if `region` is empty or not fully
    ///   inside the canvas.
    /// - Errors of [`Adjustment::prepare`](crate::Adjustment::prepare).
    /// - [`EditorError::Gpu`] if a GPU operation fails (for example, the
    ///   region is larger than the device texture limit).
    pub async fn composite_gpu(
        &mut self,
        ctx: &GpuContext,
        document: &Document,
        region: Rect,
    ) -> EditorResult<GpuImage> {
        check_region(document, region)?;
        let mut run = Run::new(
            ctx,
            &mut self.cache,
            &mut self.pool,
            &self.placeholder,
            document,
            region,
        )?;
        let start = run.bufs.take_clear(run.placeholder).await?;
        let result = run
            .composite_list(document.layers(), Buf::Owned(start))
            .await?;
        run.bufs.into_image(result).await
    }

    /// Composites like [`Self::composite_gpu`] and reads the result back
    /// to a premultiplied [`WorkingImage`].
    pub async fn composite_gpu_image(
        &mut self,
        ctx: &GpuContext,
        document: &Document,
        region: Rect,
    ) -> EditorResult<WorkingImage> {
        let image = self.composite_gpu(ctx, document, region).await?;
        let data = image.read_rgba_f32(ctx).await?;
        let pixels = data
            .chunks_exact(CHANNELS)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect();
        Ok(WorkingImage::from_pixels(
            region.width,
            region.height,
            pixels,
        )?)
    }

    /// Renders a view of the composite of `document` (zoom and pan) for an
    /// editor canvas.
    ///
    /// Returns an `rgba32float` image of `view.width x view.height` with
    /// **straight** alpha (linear Rec. 2020), for a display encoder that
    /// expects straight alpha. Output pixels outside the canvas are
    /// transparent (alpha 0). The image belongs to the compositor; the next
    /// call writes to it again.
    ///
    /// - The compositor keeps the composite of the full canvas on the GPU.
    ///   It composites again only if [`Document::revision`] changed since
    ///   the last call (or the document is a different one). A pan or a
    ///   zoom thus runs one kernel of the view size.
    /// - A composite starts from the "below" snapshot when the bottom
    ///   layers did not change (see the module documentation). Repeated
    ///   changes of one layer thus composite only that layer and the
    ///   layers above it.
    /// - Zoom 1 or more (`scale >= 1`): nearest neighbor, so that canvas
    ///   pixels show as sharp squares.
    /// - Zoom less than 1: the area mean of the canvas pixels under each
    ///   output pixel (no aliasing). The compositor keeps a pyramid of
    ///   2 x 2 means of the composite and reads the level on which an
    ///   output pixel covers 1 to 2 texels. The mean is in premultiplied
    ///   space, so transparent pixels do not darken the colors.
    ///
    /// # Errors
    ///
    /// - [`EditorError::InvalidView`] if the scale or the origin is not
    ///   valid, or the output size is 0 or larger than the device texture
    ///   limit.
    /// - The errors of [`Self::composite_gpu`] (for example, a canvas that
    ///   is larger than the device texture limit gives
    ///   [`EditorError::Gpu`]).
    pub async fn render_view(
        &mut self,
        ctx: &GpuContext,
        document: &Document,
        view: &ViewSpec,
    ) -> EditorResult<&GpuImage> {
        check_view(view, ctx.limits().max_texture_dimension_2d)?;
        let cache = self.take_view(ctx, document).await?;
        let result = draw_view(ctx, &cache, view, &mut self.view_output).await;
        self.view = Some(cache);
        result
    }

    /// Renders the mask of `layer` as an opaque gray image in the same view
    /// geometry as [`Self::render_view`].
    ///
    /// Each canvas pixel shows the mask value `v` of the layer at that
    /// pixel: inside the mask buffer the value clamped to `0.0..=1.0`,
    /// outside it the value of the mask fill. A disabled mask also shows
    /// its values. The output pixel is opaque gray with the RGB value
    /// `srgb_decode(v)` (linear), so that a display encoder with the sRGB
    /// transfer curve shows `v` as display gray `v`. Output pixels outside
    /// the canvas are transparent, as in [`Self::render_view`].
    ///
    /// The image belongs to the compositor. It is the same image as the
    /// output of [`Self::render_view`]: the next call of either function
    /// writes to it again.
    ///
    /// - The compositor keeps the gray image of the full canvas and its
    ///   pyramid on the GPU, for one mask. It uploads the mask and builds
    ///   them again only if the layer, the mask values
    ///   ([`Layer::mask_version`]), the mask offset or fill, or the canvas
    ///   size changed. A pan or a zoom thus runs one kernel of the view
    ///   size.
    /// - The function does not use or change the composite view cache of
    ///   [`Self::render_view`].
    /// - Zoom 1 or more: nearest neighbor. Zoom less than 1: the area mean
    ///   of the gray values (linear) under each output pixel.
    ///
    /// # Errors
    ///
    /// - [`EditorError::InvalidView`] as in [`Self::render_view`].
    /// - [`EditorError::LayerNotFound`] if no layer has the ID `layer`.
    /// - [`EditorError::NoMask`] if the layer has no mask.
    /// - [`EditorError::Gpu`] if a GPU operation fails (for example, the
    ///   canvas is larger than the device texture limit).
    pub async fn render_mask_view(
        &mut self,
        ctx: &GpuContext,
        document: &Document,
        view: &ViewSpec,
        layer: LayerId,
    ) -> EditorResult<&GpuImage> {
        check_view(view, ctx.limits().max_texture_dimension_2d)?;
        let found = document
            .layer(layer)
            .ok_or(EditorError::LayerNotFound(layer))?;
        let mask = found.mask().ok_or(EditorError::NoMask(layer))?;
        let bounds = document.bounds();
        check_region(document, bounds)?;
        let key = MaskViewKey {
            layer,
            version: found.mask_version(),
            offset: mask.offset,
            fill: mask.fill,
            size: (bounds.width, bounds.height),
        };
        let cache = self.take_mask_view(ctx, key, mask).await?;
        let result = draw_view(ctx, &cache.view, view, &mut self.view_output).await;
        self.mask_view = Some(cache);
        result
    }

    /// Takes the mask view cache out of `self.mask_view`, and builds it
    /// again if its key is not `key`.
    async fn take_mask_view(
        &mut self,
        ctx: &GpuContext,
        key: MaskViewKey,
        mask: &Mask,
    ) -> EditorResult<MaskViewCache> {
        let mut reuse = None;
        if let Some(cache) = self.mask_view.take() {
            if cache.key == key {
                return Ok(cache);
            }
            if cache.key.size == key.size {
                // The new image replaces the old one: the textures stay.
                reuse = Some((cache.view.composite, cache.view.mips));
            }
        }
        self.mask_view_builds += 1;
        let (width, height) = key.size;
        let canvas = Rect::new(0, 0, width, height);
        let mut params = Params::zeroed();
        params.region = rect_param(canvas)?;
        params.coef[1] = mask.fill.value();
        // Only the part of the mask buffer inside the canvas is uploaded.
        // If there is no such part, the fill value applies everywhere
        // (`params.mask` is then empty).
        let (mw, mh) = (mask.image.width(), mask.image.height());
        let texture = match canvas.intersection_signed(mask.offset.0, mask.offset.1, mw, mh) {
            Some(part) => {
                params.mask = rect_param(part)?;
                let local = to_local(part, mask.offset)?;
                Some(upload_mask(ctx, &mask.image, local).await?)
            }
            None => None,
        };
        let (image, mips) = match reuse {
            Some(textures) => textures,
            None => (
                GpuImage::new(ctx, width, height, VIEW_FORMAT).await?,
                Vec::new(),
            ),
        };
        let p = &self.placeholder;
        let m = texture.as_ref().unwrap_or(p);
        ctx.run_kernel_multi(&MASK_VIEW_KERNEL, &params, &[p, m, p], &image)
            .await?;
        let view = build_pyramid(ctx, key.version, image, mips).await?;
        Ok(MaskViewCache { key, view })
    }

    /// Takes the view composite out of `self.view`, and composites again
    /// if the document revision or the canvas size changed.
    async fn take_view(
        &mut self,
        ctx: &GpuContext,
        document: &Document,
    ) -> EditorResult<ViewCache> {
        let bounds = document.bounds();
        let mut mips = Vec::new();
        if let Some(cache) = self.view.take() {
            let same_size = cache.composite.width() == bounds.width
                && cache.composite.height() == bounds.height;
            if same_size && cache.revision == document.revision() {
                return Ok(cache);
            }
            if same_size {
                if let Some((keys, area)) = self.single_content_change(document) {
                    return self.patch_view(ctx, document, cache, keys, area).await;
                }
                // The new composite replaces the old one, so the old buffer
                // can take the new composite. The pyramid textures stay.
                self.pool.push(cache.composite);
                mips = cache.mips;
            }
        }
        self.view_builds += 1;
        let composite = self.composite_canvas(ctx, document).await?;
        build_pyramid(ctx, document.revision(), composite, mips).await
    }

    /// Checks if the view composite can get a patch (see "Partial updates"
    /// in the module documentation): since the last view composite, only
    /// the pixel or mask values of one layer changed, the change log knows
    /// the changed area, and the area is at most [`MAX_PATCH_FRACTION`] of
    /// the canvas.
    ///
    /// Returns the keys of the top-level layers now and the changed canvas
    /// area (`None`: no canvas pixel changed), or `None` if a full
    /// composite is necessary.
    fn single_content_change(&self, document: &Document) -> Option<(Vec<LayerKey>, Option<Rect>)> {
        if self.cache.no_partial {
            return None;
        }
        let bounds = document.bounds();
        // `below.last` holds the keys of the view composite: each view
        // composite sets it, and a failed composite drops it.
        let below = self.below.as_ref()?;
        if below.size != (bounds.width, bounds.height) || below.space != document.blend_space() {
            return None;
        }
        let keys: Vec<LayerKey> = document.layers().iter().map(layer_key).collect();
        let KeyDiff::One(change) = diff_lists(&below.last, &keys) else {
            return None;
        };
        match changed_canvas_area(document, change) {
            Changes::Nothing => Some((keys, None)),
            Changes::Area(rect) if rect.area() <= bounds.area() / MAX_PATCH_FRACTION => {
                Some((keys, Some(rect)))
            }
            // Not known, or too large.
            Changes::Unknown | Changes::Area(_) => None,
        }
    }

    /// Updates the view composite in `cache` after one content change
    /// (see [`Self::single_content_change`]): composites all layers only in
    /// `area`, puts the result into a copy of the old composite (patch
    /// kernel), and builds the pyramid again. `keys` are the keys of the
    /// top-level layers now.
    ///
    /// The result is equal to a full composite: the composite steps work
    /// per pixel, and they run with the same inputs in the same order.
    async fn patch_view(
        &mut self,
        ctx: &GpuContext,
        document: &Document,
        cache: ViewCache,
        keys: Vec<LayerKey>,
        area: Option<Rect>,
    ) -> EditorResult<ViewCache> {
        let ViewCache {
            composite, mips, ..
        } = cache;
        let Some(rect) = area else {
            // No canvas pixel changed: the composite stays.
            if let Some(below) = self.below.as_mut() {
                below.last = keys;
            }
            return Ok(ViewCache {
                revision: document.revision(),
                composite,
                mips,
            });
        };
        self.view_builds += 1;
        self.view_patch_builds += 1;
        let mut run = Run::new(
            ctx,
            &mut self.cache,
            &mut self.patch_pool,
            &self.placeholder,
            document,
            rect,
        )?;
        let start = run.bufs.take_clear(run.placeholder).await?;
        let result = run
            .composite_list(document.layers(), Buf::Owned(start))
            .await?;
        let small = run.bufs.into_image(result).await?;

        let (w, h) = (composite.width(), composite.height());
        let out = match self
            .pool
            .iter()
            .position(|b| b.width() == w && b.height() == h)
        {
            Some(i) => self.pool.swap_remove(i),
            None => GpuImage::new(ctx, w, h, FORMAT).await?,
        };
        let params = PatchParams {
            rect: rect_param(rect)?,
        };
        ctx.run_kernel_multi(&PATCH_KERNEL, &params, &[&composite, &small], &out)
            .await?;
        self.patch_pool.push(small);
        // The old composite is a free canvas-size buffer now.
        self.pool.push(composite);
        // The snapshot stays: if the changed layer is in it, its keys are
        // no longer a prefix of the keys, and the next full composite drops
        // it.
        if let Some(below) = self.below.as_mut() {
            below.last = keys;
        }
        build_pyramid(ctx, document.revision(), out, mips).await
    }

    /// Composites the full canvas like [`Self::composite_gpu`], with the
    /// "below" snapshot (see the module documentation):
    ///
    /// 1. If the snapshot keys are the keys of the bottom top-level layers
    ///    now, the composite starts from the snapshot. Else it starts from
    ///    transparent black, and the snapshot is dropped.
    /// 2. The target snapshot index is the lowest top-level layer that
    ///    changed since the last call (normally the layer that the user
    ///    edits). If the composite passes that index, the state there
    ///    becomes the new snapshot (no copy: the buffer moves).
    async fn composite_canvas(
        &mut self,
        ctx: &GpuContext,
        document: &Document,
    ) -> EditorResult<GpuImage> {
        let bounds = document.bounds();
        check_region(document, bounds)?;
        let layers = document.layers();
        let keys: Vec<LayerKey> = layers.iter().map(layer_key).collect();
        let size = (bounds.width, bounds.height);
        let space = document.blend_space();
        // If the call fails, the snapshot stays dropped (no stale state).
        let (last, old) = match self.below.take() {
            Some(b) if b.size == size && b.space == space => (b.last, b.snapshot),
            _ => (Vec::new(), None),
        };
        let mut snapshot = old.filter(|s| keys.starts_with(&s.keys));
        let start = snapshot.as_ref().map_or(0, |s| s.keys.len());
        let target = if keys == last {
            start
        } else {
            keys.iter()
                .zip(&last)
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| keys.len().min(last.len()))
        };

        let mut run = Run::new(
            ctx,
            &mut self.cache,
            &mut self.pool,
            &self.placeholder,
            document,
            bounds,
        )?;
        let mut done = start;
        let mut cur = match &snapshot {
            Some(s) => Buf::Borrowed(&s.image),
            None => Buf::Owned(run.bufs.take_clear(run.placeholder).await?),
        };
        // A snapshot of 0 layers or of all layers has no use.
        if target > start && target < layers.len() {
            let below = run.composite_list(&layers[start..target], cur).await?;
            cur = match below {
                Buf::Owned(image) => {
                    if let Some(old) = snapshot.take() {
                        run.bufs.recycle(Buf::Owned(old.image));
                    }
                    let saved = snapshot.insert(Snapshot {
                        keys: keys[..target].to_vec(),
                        image,
                    });
                    Buf::Borrowed(&saved.image)
                }
                // No step ran (the layers are hidden): the buffer is the
                // old snapshot. It stays correct with its own keys.
                borrowed @ Buf::Borrowed(_) => borrowed,
            };
            done = target;
        }
        let result = run.composite_list(&layers[done..], cur).await?;
        let image = run.bufs.into_image(result).await?;
        if start > 0 {
            self.view_partial_builds += 1;
        }
        self.below = Some(BelowCache {
            size,
            space,
            last: keys,
            snapshot,
        });
        Ok(image)
    }
}

/// Renders `view` from the image and pyramid in `cache` to the view output
/// in `output` (a new texture if the output size changed). See
/// [`GpuCompositor::render_view`] for the sampling.
async fn draw_view<'o>(
    ctx: &GpuContext,
    cache: &ViewCache,
    view: &ViewSpec,
    output: &'o mut Option<GpuImage>,
) -> EditorResult<&'o GpuImage> {
    let wanted = view_level(view.scale);
    let level = wanted.min(cache.mips.len());
    let source = match level.checked_sub(1).and_then(|i| cache.mips.get(i)) {
        Some(mip) => mip,
        None => &cache.composite,
    };
    // `level` is at most the pyramid depth (at most 15 for
    // MAX_DIMENSION), so the cast is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let level_scale = 2.0_f64.powi(level as i32);
    let params = view_params(
        view,
        level_scale,
        (source.width(), source.height()),
        view.scale >= 1.0,
    )?;

    let out = match output.take() {
        Some(image) if image.width() == view.width && image.height() == view.height => image,
        _ => GpuImage::new(ctx, view.width, view.height, FORMAT).await?,
    };
    let out = output.insert(out);
    ctx.run_kernel_multi(&VIEW_KERNEL, &params, &[source], out)
        .await?;
    Ok(out)
}

/// Fills the view pyramid of `composite`. `mips` are the textures of an
/// earlier pyramid of the same canvas size, or empty (then the function
/// makes new textures).
async fn build_pyramid(
    ctx: &GpuContext,
    revision: u64,
    composite: GpuImage,
    mut mips: Vec<GpuImage>,
) -> EditorResult<ViewCache> {
    if mips.is_empty() {
        let (mut w, mut h) = (composite.width(), composite.height());
        while w > 1 || h > 1 {
            w = w.div_ceil(2);
            h = h.div_ceil(2);
            mips.push(GpuImage::new(ctx, w, h, VIEW_FORMAT).await?);
        }
    }
    let mut source = &composite;
    for mip in &mips {
        ctx.run_kernel_multi(&MIP_KERNEL, &(), &[source], mip)
            .await?;
        source = mip;
    }
    Ok(ViewCache {
        revision,
        composite,
        mips,
    })
}

/// A compositor buffer: owned by the current step, or the backdrop of a
/// pass-through group (owned by the caller, read-only).
enum Buf<'a> {
    Owned(GpuImage),
    Borrowed(&'a GpuImage),
}

impl Buf<'_> {
    fn get(&self) -> &GpuImage {
        match self {
            Self::Owned(image) => image,
            Self::Borrowed(image) => image,
        }
    }
}

/// The region-size buffers of one composite call. Free buffers go back to
/// the pool of the compositor, so the next call can use them again.
struct Bufs<'c> {
    ctx: &'c GpuContext,
    width: u32,
    height: u32,
    /// Free region-size buffers (the pool of the compositor).
    free: &'c mut Vec<GpuImage>,
    /// The parameters of the clear kernel.
    clear_params: Params,
}

impl Bufs<'_> {
    /// Takes a region-size buffer with any content.
    async fn take(&mut self) -> EditorResult<GpuImage> {
        match self.free.pop() {
            Some(image) => Ok(image),
            None => Ok(GpuImage::new(self.ctx, self.width, self.height, FORMAT).await?),
        }
    }

    /// Takes a transparent region-size buffer.
    async fn take_clear(&mut self, placeholder: &GpuImage) -> EditorResult<GpuImage> {
        match self.free.pop() {
            Some(image) => {
                let p = placeholder;
                self.ctx
                    .run_kernel_multi(&CLEAR_KERNEL, &self.clear_params, &[p, p, p], &image)
                    .await?;
                Ok(image)
            }
            // A new image is zero-filled.
            None => Ok(GpuImage::new(self.ctx, self.width, self.height, FORMAT).await?),
        }
    }

    /// Puts an owned buffer back in the pool.
    fn recycle(&mut self, buf: Buf<'_>) {
        if let Buf::Owned(image) = buf {
            self.free.push(image);
        }
    }

    /// Returns `buf` as an owned image. A borrowed buffer is copied.
    async fn into_image(mut self, buf: Buf<'_>) -> EditorResult<GpuImage> {
        match buf {
            Buf::Owned(image) => Ok(image),
            Buf::Borrowed(image) => {
                let out = self.take().await?;
                self.ctx
                    .run_kernel(&expose_gpu::kernels::COPY, &(), image, &out)
                    .await?;
                Ok(out)
            }
        }
    }

    /// Runs one step: `kernel` reads `cur`, `mask`, and `aux` and writes a
    /// new buffer, which becomes `cur`.
    async fn step(
        &mut self,
        cur: &mut Buf<'_>,
        kernel: &Kernel,
        params: &Params,
        mask: &GpuImage,
        aux: &GpuImage,
    ) -> EditorResult<()> {
        let out = self.take().await?;
        self.ctx
            .run_kernel_multi(kernel, params, &[cur.get(), mask, aux], &out)
            .await?;
        let old = std::mem::replace(cur, Buf::Owned(out));
        self.recycle(old);
        Ok(())
    }
}

/// Converts a rectangle in canvas coordinates to the coordinates of a
/// buffer at `offset`. The caller must make sure that `rect` is inside the
/// buffer (for example, an intersection with it).
fn to_local(rect: Rect, offset: (i32, i32)) -> EditorResult<Rect> {
    let x = u32::try_from(i64::from(rect.x) - i64::from(offset.0))
        .map_err(|_| EditorError::OffsetOverflow)?;
    let y = u32::try_from(i64::from(rect.y) - i64::from(offset.1))
        .map_err(|_| EditorError::OffsetOverflow)?;
    Ok(Rect::new(x, y, rect.width, rect.height))
}

/// Returns the kernel rectangle (canvas position and size) of a part
/// `rect` of a buffer at `offset`.
fn canvas_param(rect: Rect, offset: (i32, i32)) -> EditorResult<[i32; 4]> {
    let x = offset
        .0
        .checked_add(to_i32(rect.x)?)
        .ok_or(EditorError::OffsetOverflow)?;
    let y = offset
        .1
        .checked_add(to_i32(rect.y)?)
        .ok_or(EditorError::OffsetOverflow)?;
    Ok([x, y, to_i32(rect.width)?, to_i32(rect.height)?])
}

impl LayerCache {
    /// Drops the entries of layers that are no longer in `document`, and
    /// the textures whose values changed and cannot get a partial update
    /// (see "Partial updates" in the module documentation).
    fn prune(&mut self, document: &Document) {
        let mut live: HashSet<LayerId> = HashSet::new();
        let partial = !self.no_partial;
        for layer in document.layers() {
            layer.visit(&mut |l| {
                live.insert(l.id());
                let Some(entry) = self.entries.get_mut(&l.id()) else {
                    return;
                };
                // A part stays if it has the current values, or if the
                // change log knows the changes after its version.
                let keep_pixels = entry.pixels.as_ref().is_some_and(|p| {
                    l.as_pixel().is_some()
                        && (p.version == l.pixel_version()
                            || (partial && l.pixel_changes_since(p.version).is_known()))
                });
                if !keep_pixels {
                    entry.pixels = None;
                }
                let keep_mask = entry.mask.as_ref().is_some_and(|m| {
                    l.mask().is_some()
                        && (m.version == l.mask_version()
                            || (partial && l.mask_changes_since(m.version).is_known()))
                });
                if !keep_mask {
                    entry.mask = None;
                }
                let keep_curve = match (entry.curve.as_ref(), l.kind()) {
                    (Some(table), LayerKind::Adjustment(Adjustment::Curves(c))) => {
                        table.curves == *c
                    }
                    _ => false,
                };
                if !keep_curve {
                    entry.curve = None;
                }
            });
        }
        self.entries.retain(|id, entry| {
            live.contains(id)
                && (entry.pixels.is_some() || entry.mask.is_some() || entry.curve.is_some())
        });
    }

    /// Returns the textures of one layer step, and uploads the values that
    /// are not in the cache or are out of date.
    ///
    /// - `source`: the pixels of a pixel layer, or `None` for other layers
    ///   and for a pixel layer outside the region (then the function also
    ///   does not get the mask).
    /// - `mask`: the values of an enabled mask, or `None` if the layer has
    ///   no enabled mask or the mask buffer is outside the region.
    async fn textures<'s>(
        &'s mut self,
        ctx: &GpuContext,
        layer: &Layer,
        area: Area,
    ) -> EditorResult<StepTextures<'s>> {
        let Self {
            entries,
            uploads,
            partial_uploads,
            spares,
            no_partial,
            ..
        } = self;
        let mut counts = UploadCounts {
            full: uploads,
            partial: partial_uploads,
            spares,
            allow_partial: !*no_partial,
        };
        let entry = entries.entry(layer.id()).or_default();
        let source = match layer.as_pixel() {
            Some(p) => {
                let img = p.image();
                let (ox, oy) = p.offset();
                let Some(needed) =
                    area.region
                        .intersection_signed(ox, oy, img.width(), img.height())
                else {
                    return Ok(StepTextures {
                        source: None,
                        mask: None,
                        table: None,
                    });
                };
                let wanted = Wanted {
                    version: layer.pixel_version(),
                    offset: (ox, oy),
                    needed,
                };
                let part = ensure_part(
                    ctx,
                    &mut entry.pixels,
                    &mut counts,
                    wanted,
                    PartData::Pixels(img),
                    &|v| layer.pixel_changes_since(v),
                    area,
                )
                .await?;
                Some(part)
            }
            None => None,
        };
        let mask = match layer.mask().filter(|m| m.enabled) {
            Some(m) => {
                let img = &m.image;
                let (ox, oy) = m.offset;
                match area
                    .region
                    .intersection_signed(ox, oy, img.width(), img.height())
                {
                    Some(needed) => {
                        let wanted = Wanted {
                            version: layer.mask_version(),
                            offset: (ox, oy),
                            needed,
                        };
                        let part = ensure_part(
                            ctx,
                            &mut entry.mask,
                            &mut counts,
                            wanted,
                            PartData::Mask(img),
                            &|v| layer.mask_changes_since(v),
                            area,
                        )
                        .await?;
                        Some(part)
                    }
                    None => None,
                }
            }
            None => None,
        };
        // `ensure_curve_table` stored the table of a curves layer, and
        // `prune` dropped the tables of other layers.
        let table = entry.curve.as_ref().map(|c| &c.image);
        Ok(StepTextures {
            source,
            mask,
            table,
        })
    }

    /// Makes sure that the entry of layer `id` has the curve table of
    /// `curves`. Uploads the table only if the curves changed. `master`
    /// and `channels` are the prepared splines of `curves`.
    async fn ensure_curve_table(
        &mut self,
        ctx: &GpuContext,
        id: LayerId,
        curves: &Curves,
        master: Option<&MonotoneCubic>,
        channels: &[MonotoneCubic; 3],
    ) -> EditorResult<()> {
        let entry = self.entries.entry(id).or_default();
        if entry.curve.as_ref().is_some_and(|c| c.curves == *curves) {
            return Ok(());
        }
        // The old table is dropped before the new texture is made.
        entry.curve = None;
        let last = CURVE_TABLE_SIZE - 1;
        let mut data = Vec::with_capacity(CURVE_TABLE_SIZE as usize * CHANNELS);
        for i in 0..CURVE_TABLE_SIZE {
            // `i` and `last` are at most 4096, so they are exact in f32,
            // and the quotient is in 0..=1.
            #[allow(clippy::cast_precision_loss)]
            let x = i as f32 / last as f32;
            for curve in channels {
                let y = curve.eval(x);
                data.push(master.map_or(y, |m| m.eval(y)));
            }
            data.push(0.0);
        }
        let image = GpuImage::from_rgba_f32(ctx, CURVE_TABLE_SIZE, 1, FORMAT, &data).await?;
        self.curve_uploads += 1;
        entry.curve = Some(CurveTable {
            curves: curves.clone(),
            image,
        });
        Ok(())
    }
}

/// The area of one composite call, for the layer cache.
#[derive(Debug, Clone, Copy)]
struct Area {
    /// The composite region.
    region: Rect,
    /// The canvas rectangle. The cache keeps the parts of layers and masks
    /// that are inside it.
    canvas: Rect,
    /// The device texture limit.
    max_dim: u32,
}

/// The values of a cached part.
#[derive(Clone, Copy)]
enum PartData<'a> {
    Pixels(&'a WorkingImage),
    Mask(&'a MaskImage),
}

impl PartData<'_> {
    fn size(self) -> (u32, u32) {
        match self {
            Self::Pixels(img) => (img.width(), img.height()),
            Self::Mask(img) => (img.width(), img.height()),
        }
    }

    /// The texture width of a part `width` values wide.
    const fn texel_width(self, width: u32) -> u32 {
        match self {
            Self::Pixels(_) => width,
            Self::Mask(_) => width.div_ceil(4),
        }
    }

    async fn upload(self, ctx: &GpuContext, rect: Rect) -> EditorResult<GpuImage> {
        match self {
            Self::Pixels(img) => upload_pixels(ctx, img, rect).await,
            Self::Mask(img) => upload_mask(ctx, img, rect).await,
        }
    }
}

/// What a step needs of one cached part.
#[derive(Debug, Clone, Copy)]
struct Wanted {
    /// The content version of the values.
    version: u64,
    /// The canvas position of the buffer.
    offset: (i32, i32),
    /// The part of the buffer that the step reads, in canvas coordinates.
    needed: Rect,
}

/// The textures of one layer step (see [`LayerCache::textures`]).
struct StepTextures<'a> {
    source: Option<&'a CachedPart>,
    mask: Option<&'a CachedPart>,
    /// The curve table of a curves layer.
    table: Option<&'a GpuImage>,
}

/// The upload counters of the layer cache, and its spare textures.
struct UploadCounts<'a> {
    full: &'a mut u64,
    partial: &'a mut u64,
    spares: &'a mut Vec<GpuImage>,
    /// `false` if partial updates are off (see `LayerCache::no_partial`).
    allow_partial: bool,
}

/// The change log query of one content of a layer (see
/// [`Layer::pixel_changes_since`]).
type ChangesSince<'a> = dyn Fn(u64) -> Changes + Sync + 'a;

/// Returns the cached part in `slot` if it has the wanted version and
/// covers the needed part. If it covers the needed part and the change log
/// (`changes`) knows the changes after its version, updates only the
/// changed area (see [`patch_part`]). Else uploads a new part and stores
/// it in `slot`.
///
/// A new part is the part of the buffer inside the canvas, so that a later
/// call with a different region can use it again. If that part is larger
/// than the device texture limit, the new part is only the needed part.
async fn ensure_part<'e>(
    ctx: &GpuContext,
    slot: &'e mut Option<CachedPart>,
    counts: &mut UploadCounts<'_>,
    wanted: Wanted,
    data: PartData<'_>,
    changes: &ChangesSince<'_>,
    area: Area,
) -> EditorResult<&'e CachedPart> {
    let local = to_local(wanted.needed, wanted.offset)?;
    let part = match slot.take() {
        Some(part) if part.version == wanted.version && part.rect.contains_rect(&local) => part,
        Some(part)
            if counts.allow_partial
                && part.rect.contains_rect(&local)
                && changes(part.version).is_known() =>
        {
            // The check above makes sure that the log knows the changes:
            // `None` here means that no value changed.
            let changed = changes(part.version).area();
            *counts.partial += 1;
            patch_part(ctx, part, changed, wanted.version, data, counts.spares).await?
        }
        // An old part is dropped here, before the new texture is made.
        _ => {
            let (w, h) = data.size();
            let (ox, oy) = wanted.offset;
            // `needed` is inside the region, which is inside the canvas,
            // so the canvas part exists and contains `needed`. If it did
            // not exist, `needed` is a correct (smaller) choice.
            let cover = area
                .canvas
                .intersection_signed(ox, oy, w, h)
                .unwrap_or(wanted.needed);
            let fits =
                data.texel_width(cover.width) <= area.max_dim && cover.height <= area.max_dim;
            let chosen = if fits { cover } else { wanted.needed };
            let rect = to_local(chosen, wanted.offset)?;
            let image = data.upload(ctx, rect).await?;
            *counts.full += 1;
            CachedPart {
                version: wanted.version,
                rect,
                image,
            }
        }
    };
    Ok(slot.insert(part))
}

/// A view composite gets a patch only if the changed area is at most
/// `1 / MAX_PATCH_FRACTION` of the canvas. For a larger area, a full
/// composite from the "below" snapshot can be faster, because it does not
/// composite the layers below the changed layer.
const MAX_PATCH_FRACTION: u64 = 2;

/// The maximum number of spare textures in the layer cache. One spare is
/// sufficient for repeated partial updates of one layer (the old texture of
/// one update is the output of the next). A second one covers a pixel layer
/// and a mask in the same frame.
const MAX_SPARES: usize = 2;

/// Updates the area `changed` (buffer coordinates, `None`: no area) of a
/// cached part to the values of `data`, and gives it the version
/// `version`.
///
/// `expose-gpu` has no upload to a part of a texture, and a kernel cannot
/// write its input. Thus the function uploads the changed area as a small
/// texture, and the patch kernel copies the old texture with the small
/// texture in the changed area to a spare texture of the same size. The
/// old texture becomes a spare. The values are the same as after a full
/// upload: the same CPU conversion to `f16` makes them, and the kernel only
/// copies them.
///
/// Masks have four values in each texel. The function extends the changed
/// area to whole texels of the part, so that the small texture has the
/// same texel grid (and the same zero padding in the last texel) as a full
/// upload.
async fn patch_part(
    ctx: &GpuContext,
    part: CachedPart,
    changed: Option<Rect>,
    version: u64,
    data: PartData<'_>,
    spares: &mut Vec<GpuImage>,
) -> EditorResult<CachedPart> {
    let Some(r) = changed.and_then(|c| c.intersection(&part.rect)) else {
        // No value of the part changed.
        return Ok(CachedPart { version, ..part });
    };
    // Position in the part, in values. `r` is inside `part.rect`.
    let (mut px, py) = (r.x - part.rect.x, r.y - part.rect.y);
    let mut width = r.width;
    if let PartData::Mask(_) = data {
        let end = (px + width).next_multiple_of(4).min(part.rect.width);
        px -= px % 4;
        width = end - px;
    }
    let upload_rect = Rect::new(part.rect.x + px, part.rect.y + py, width, r.height);
    let small = data.upload(ctx, upload_rect).await?;
    let texel_x = match data {
        PartData::Pixels(_) => px,
        PartData::Mask(_) => px / 4,
    };
    let params = PatchParams {
        rect: [
            to_i32(texel_x)?,
            to_i32(py)?,
            to_i32(small.width())?,
            to_i32(small.height())?,
        ],
    };
    let (w, h) = (part.image.width(), part.image.height());
    let out = match spares
        .iter()
        .position(|s| s.width() == w && s.height() == h && s.format() == LAYER_FORMAT)
    {
        Some(i) => spares.swap_remove(i),
        None => GpuImage::new(ctx, w, h, LAYER_FORMAT).await?,
    };
    ctx.run_kernel_multi(&PATCH_KERNEL, &params, &[&part.image, &small], &out)
        .await?;
    spares.push(part.image);
    if spares.len() > MAX_SPARES {
        // The oldest spare has the least chance to match the next update.
        spares.remove(0);
    }
    Ok(CachedPart {
        version,
        rect: part.rect,
        image: out,
    })
}

/// Sets the mask parameters of a layer step. Returns the mask texture, or
/// `None` if the kernel does not read it.
fn set_mask_params<'t>(
    layer: &Layer,
    part: Option<&'t CachedPart>,
    params: &mut Params,
) -> EditorResult<Option<&'t GpuImage>> {
    let Some(m) = layer.mask().filter(|m| m.enabled) else {
        // A disabled mask has the value 1 everywhere: the coverage is the
        // base value.
        params.flags[2] = 0;
        return Ok(None);
    };
    params.flags[2] = 1;
    params.coef[1] = m.fill.value();
    if let Some(part) = part {
        params.mask = canvas_param(part.rect, m.offset)?;
        Ok(Some(&part.image))
    } else {
        // The region is fully outside the mask buffer: the fill value
        // applies everywhere.
        params.mask = [0; 4];
        Ok(None)
    }
}

/// Uploads the part `rect` (layer pixel coordinates, inside the image) of
/// a layer image.
async fn upload_pixels(ctx: &GpuContext, img: &WorkingImage, rect: Rect) -> EditorResult<GpuImage> {
    if rect == img.bounds() {
        // The full layer: no CPU copy.
        let image =
            GpuImage::from_rgba_f32(ctx, rect.width, rect.height, LAYER_FORMAT, img.as_slice())
                .await?;
        return Ok(image);
    }
    let (x0, y0) = (rect.x as usize, rect.y as usize);
    let (w, h) = (rect.width as usize, rect.height as usize);
    let src_row = img.width() as usize * CHANNELS;
    let all = img.as_slice();
    let mut data = Vec::with_capacity(w * h * CHANNELS);
    for row in y0..y0 + h {
        let start = row * src_row + x0 * CHANNELS;
        data.extend_from_slice(&all[start..start + w * CHANNELS]);
    }
    Ok(GpuImage::from_rgba_f32(ctx, rect.width, rect.height, LAYER_FORMAT, &data).await?)
}

/// Uploads the part `rect` (mask pixel coordinates, inside the mask) of a
/// mask image, four values in each texel.
async fn upload_mask(ctx: &GpuContext, img: &MaskImage, rect: Rect) -> EditorResult<GpuImage> {
    let (x0, y0) = (rect.x as usize, rect.y as usize);
    let (w, h) = (rect.width as usize, rect.height as usize);
    let texel_width = rect.width.div_ceil(4);
    let row_len = texel_width as usize * CHANNELS;
    let src_row = img.width() as usize;
    let all = img.data();
    let mut data = vec![0.0_f32; row_len * h];
    for (row, dst) in data.chunks_exact_mut(row_len).enumerate() {
        let start = (y0 + row) * src_row + x0;
        dst[..w].copy_from_slice(&all[start..start + w]);
    }
    Ok(GpuImage::from_rgba_f32(ctx, texel_width, rect.height, LAYER_FORMAT, &data).await?)
}

/// The state of one composite call.
struct Run<'c> {
    bufs: Bufs<'c>,
    cache: &'c mut LayerCache,
    placeholder: &'c GpuImage,
    region: Rect,
    region_param: [i32; 4],
    /// The canvas rectangle. The cache keeps the parts of layers and masks
    /// that are inside it.
    canvas: Rect,
    /// The device texture limit.
    max_dim: u32,
    space: BlendSpace,
}

impl<'c> Run<'c> {
    /// Starts a composite of `region` (which the caller checked). Prunes
    /// the layer cache and drops the free buffers of a different size.
    fn new(
        ctx: &'c GpuContext,
        cache: &'c mut LayerCache,
        pool: &'c mut Vec<GpuImage>,
        placeholder: &'c GpuImage,
        document: &Document,
        region: Rect,
    ) -> EditorResult<Self> {
        cache.prune(document);
        // The pool keeps buffers of one size only.
        pool.retain(|b| b.width() == region.width && b.height() == region.height);
        let region_param = rect_param(region)?;
        let mut clear_params = Params::zeroed();
        clear_params.region = region_param;
        Ok(Self {
            bufs: Bufs {
                ctx,
                width: region.width,
                height: region.height,
                free: pool,
                clear_params,
            },
            cache,
            placeholder,
            region,
            region_param,
            canvas: document.bounds(),
            max_dim: ctx.limits().max_texture_dimension_2d,
            space: document.blend_space(),
        })
    }

    /// The parameters that are the same for all steps.
    fn params(&self) -> Params {
        let mut params = Params::zeroed();
        params.region = self.region_param;
        params.flags[1] = space_id(self.space);
        params
    }

    /// The parameters of a layer step without the mask.
    fn layer_params(&self, layer: &Layer, base: f32) -> Params {
        let mut params = self.params();
        params.flags[0] = mode_id(layer.blend_mode());
        params.coef[0] = base;
        params
    }

    /// The area of this call for the layer cache.
    const fn area(&self) -> Area {
        Area {
            region: self.region,
            canvas: self.canvas,
            max_dim: self.max_dim,
        }
    }

    /// Composites `layers` onto `cur` and returns the new buffer.
    async fn composite_list<'a>(
        &mut self,
        layers: &[Layer],
        mut cur: Buf<'a>,
    ) -> EditorResult<Buf<'a>> {
        let placeholder = self.placeholder;
        let ctx = self.bufs.ctx;
        let area = self.area();
        for layer in layers {
            if !layer.visible() {
                continue;
            }
            let base = layer.opacity() * layer.fill_opacity();
            if base <= 0.0 {
                continue;
            }
            let mut params = self.layer_params(layer, base);
            match layer.kind() {
                LayerKind::Pixel(p) => {
                    let tex = self.cache.textures(ctx, layer, area).await?;
                    // No source: the layer is outside the region.
                    let Some(source) = tex.source else {
                        continue;
                    };
                    params.source = canvas_param(source.rect, p.offset())?;
                    let mask = set_mask_params(layer, tex.mask, &mut params)?;
                    let mask = mask.unwrap_or(placeholder);
                    self.bufs
                        .step(&mut cur, &BLEND_KERNEL, &params, mask, &source.image)
                        .await?;
                }
                LayerKind::Adjustment(a) => {
                    let prepared = a.prepare()?;
                    Self::adjust_params(&prepared, &mut params);
                    // `prepare` gives `Prepared::Curves` for curves.
                    if let (Adjustment::Curves(c), Prepared::Curves { master, channels }) =
                        (a, &prepared.inner)
                    {
                        self.cache
                            .ensure_curve_table(ctx, layer.id(), c, master.as_ref(), channels)
                            .await?;
                    }
                    let tex = self.cache.textures(ctx, layer, area).await?;
                    let table = tex.table.unwrap_or(placeholder);
                    let mask = set_mask_params(layer, tex.mask, &mut params)?;
                    let mask = mask.unwrap_or(placeholder);
                    self.bufs
                        .step(&mut cur, &ADJUST_KERNEL, &params, mask, table)
                        .await?;
                }
                LayerKind::Group(g) => match g.isolation() {
                    GroupIsolation::PassThrough => {
                        let after =
                            Box::pin(self.composite_list(g.children(), Buf::Borrowed(cur.get())))
                                .await?;
                        // The mask textures are read after the children,
                        // because the children also use the cache.
                        let tex = self.cache.textures(ctx, layer, area).await?;
                        let mask = set_mask_params(layer, tex.mask, &mut params)?;
                        let mask = mask.unwrap_or(placeholder);
                        let out = self.bufs.take().await?;
                        ctx.run_kernel_multi(
                            &PASS_MIX_KERNEL,
                            &params,
                            &[cur.get(), mask, after.get()],
                            &out,
                        )
                        .await?;
                        self.bufs.recycle(after);
                        let old = std::mem::replace(&mut cur, Buf::Owned(out));
                        self.bufs.recycle(old);
                    }
                    GroupIsolation::Isolated => {
                        let start = self.bufs.take_clear(placeholder).await?;
                        let group =
                            Box::pin(self.composite_list(g.children(), Buf::Owned(start))).await?;
                        params.source = self.region_param;
                        let tex = self.cache.textures(ctx, layer, area).await?;
                        let mask = set_mask_params(layer, tex.mask, &mut params)?;
                        let mask = mask.unwrap_or(placeholder);
                        self.bufs
                            .step(&mut cur, &BLEND_KERNEL, &params, mask, group.get())
                            .await?;
                        self.bufs.recycle(group);
                    }
                },
            }
        }
        Ok(cur)
    }

    /// Sets the adjustment parameters. The curve table of a curves
    /// adjustment comes from the layer cache.
    fn adjust_params(prepared: &PreparedAdjustment, params: &mut Params) {
        match &prepared.inner {
            Prepared::BrightnessContrast(bc) => {
                params.flags[3] = ADJUST_BRIGHTNESS_CONTRAST;
                let (k, threshold) = bc.k.map_or((0.0, 1.0), |k| (k, 0.0));
                params.adj[0] = [bc.brightness, k, threshold, 0.0];
            }
            Prepared::Levels(l) => {
                params.flags[3] = ADJUST_LEVELS;
                let ch = [l.red, l.green, l.blue, l.master];
                params.adj[0] = ch.map(|c| c.input_black);
                params.adj[1] = ch.map(|c| c.input_white);
                params.adj[2] = ch.map(|c| c.gamma);
                params.adj[3] = ch.map(|c| c.output_black);
                params.adj[4] = ch.map(|c| c.output_white);
            }
            Prepared::Curves { .. } => {
                // The curve table comes from the layer cache.
                params.flags[3] = ADJUST_CURVES;
            }
            Prepared::HueSaturation {
                matrix,
                saturation,
                lightness,
            } => {
                params.flags[3] = ADJUST_HUE_SATURATION;
                for (dst, row) in params.adj.iter_mut().zip(matrix) {
                    *dst = [row[0], row[1], row[2], 0.0];
                }
                params.adj[3] = [*saturation, *lightness, 0.0, 0.0];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // Test code: a failed setup is a test failure. Some tests compare
    // exact values (copies of one texel, transparent black).
    #![allow(
        clippy::float_cmp,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap,
        clippy::too_many_lines
    )]

    use expose_core::color::TransferFunction;
    use expose_core::develop::CurvePoint;
    use expose_gpu::{GpuContextOptions, GpuError};

    use super::*;
    use crate::adjust::{Adjustment, BrightnessContrast, Curves, HueSaturation, Levels};
    use crate::composite::composite;
    use crate::document::DocumentSpec;
    use crate::history::History;
    use crate::layer::{LayerContent, Mask, MaskFill, PixelLayer, Placement};

    /// The tolerance for each sample: `|gpu - cpu| <= TOL * max(1, |cpu|)`.
    ///
    /// The GPU and the CPU round some operations differently (`pow`,
    /// division, fused multiply-add). After several layers in the
    /// perceptual space (sRGB encode and decode at each step), the
    /// measured error is at most about 1.3e-5 (curves included).
    const TOL: f32 = 5e-5;

    /// The tolerance for the divide mode. A divisor near
    /// [`DIVIDE_MIN_SOURCE`](crate::blend::DIVIDE_MIN_SOURCE) makes the
    /// relative rounding error of the divisor much larger. The measured
    /// error is at most about 1.1e-4.
    const DIVIDE_TOL: f32 = 5e-4;

    /// The tolerance for the textures in `rgba16float` that only the
    /// display uses (view pyramid levels, mask view image), for each
    /// sample: `|gpu - want| <= VIEW_TOL * max(1, |want|)`. Each kernel
    /// rounds its output to the nearest `f16` value (at most `2^-12`
    /// relative), and the test layers with values that are not `f16`
    /// values add one more rounding. Measured: at most about 2.4e-4.
    const VIEW_TOL: f32 = 5e-4;

    /// Tests of the partial updates (`src/gpu/tests/partial.rs`).
    mod partial;

    fn context() -> Option<GpuContext> {
        match pollster::block_on(GpuContext::new(GpuContextOptions::default())) {
            Ok(ctx) => Some(ctx),
            Err(GpuError::NoAdapter(reason)) => {
                eprintln!("SKIPPED: no GPU adapter ({reason})");
                None
            }
            Err(error) => panic!("GPU context creation failed: {error}"),
        }
    }

    /// A small deterministic random generator (xorshift64*).
    struct Rng(u64);

    impl Rng {
        fn next_u32(&mut self) -> u32 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
        }

        /// A value in `lo..hi`.
        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            let u = (self.next_u32() >> 8) as f32 / 16_777_216.0;
            lo + (hi - lo) * u
        }
    }

    /// Rounds `v` to the nearest `f16` value (the rounding of the upload
    /// to the layer texture cache).
    fn to_f16(v: f32) -> f32 {
        half::f16::from_f32(v).to_f32()
    }

    /// Random premultiplied pixels. Colors are in -0.2..2.0 (values below
    /// 0 and above 1 are included). 10 % of the pixels are transparent and
    /// 20 % are opaque.
    ///
    /// All values are `f16` values, so the upload to the layer texture
    /// cache does not change them, and a compare with the CPU reference
    /// tests only the composite math. [`f16_layer_storage_error_is_small`]
    /// tests the rounding of other values.
    fn random_image(rng: &mut Rng, width: u32, height: u32) -> WorkingImage {
        let mut image = random_image_f32(rng, width, height);
        for p in image.pixels_mut() {
            *p = p.map(to_f16);
        }
        image
    }

    /// Random premultiplied pixels as [`random_image`], without the
    /// rounding to `f16`.
    fn random_image_f32(rng: &mut Rng, width: u32, height: u32) -> WorkingImage {
        let pixels = (0..width as usize * height as usize)
            .map(|_| {
                let pick = rng.range(0.0, 1.0);
                let a = if pick < 0.1 {
                    0.0
                } else if pick < 0.3 {
                    1.0
                } else {
                    rng.range(0.0, 1.0)
                };
                let c = [
                    rng.range(-0.2, 2.0),
                    rng.range(-0.2, 2.0),
                    rng.range(-0.2, 2.0),
                ];
                [c[0] * a, c[1] * a, c[2] * a, a]
            })
            .collect();
        WorkingImage::from_pixels(width, height, pixels).unwrap()
    }

    fn random_mask(rng: &mut Rng, width: u32, height: u32) -> MaskImage {
        // Values slightly outside 0..=1 test the clamp.
        let data = (0..width as usize * height as usize)
            .map(|_| to_f16(rng.range(-0.2, 1.2)))
            .collect();
        MaskImage::from_data(width, height, data).unwrap()
    }

    fn empty_doc(width: u32, height: u32, space: BlendSpace) -> Document {
        Document::new_empty(DocumentSpec {
            blend_space: space,
            ..DocumentSpec::new(width, height)
        })
        .unwrap()
    }

    /// Adds a layer at the top of `parent` and returns its ID.
    fn add(doc: &mut Document, parent: Option<LayerId>, content: LayerContent) -> LayerId {
        let index = match parent {
            Some(id) => doc.layer(id).unwrap().children().len(),
            None => doc.layers().len(),
        };
        doc.add_layer("layer", content, Placement::new(parent, index))
            .unwrap()
            .0
    }

    fn add_pixel(
        doc: &mut Document,
        rng: &mut Rng,
        parent: Option<LayerId>,
        size: (u32, u32),
        offset: (i32, i32),
    ) -> LayerId {
        let image = random_image(rng, size.0, size.1);
        add(
            doc,
            parent,
            LayerContent::Pixel(PixelLayer::new(image, offset).unwrap()),
        )
    }

    fn set_mask(
        doc: &mut Document,
        rng: &mut Rng,
        id: LayerId,
        size: (u32, u32),
        offset: (i32, i32),
        fill: MaskFill,
    ) {
        let mask = Mask::new(random_mask(rng, size.0, size.1), offset, fill);
        doc.set_mask(id, Some(mask)).unwrap();
    }

    fn curves() -> Curves {
        Curves {
            master: vec![
                CurvePoint::new(0.0, 0.0),
                CurvePoint::new(0.25, 0.3),
                CurvePoint::new(0.75, 0.8),
                CurvePoint::new(1.0, 1.0),
            ],
            red: vec![
                CurvePoint::new(0.0, 0.1),
                CurvePoint::new(0.5, 0.6),
                CurvePoint::new(1.0, 0.9),
            ],
            green: vec![CurvePoint::new(0.0, 0.0), CurvePoint::new(1.0, 1.0)],
            blue: vec![CurvePoint::new(0.25, 0.0), CurvePoint::new(1.0, 1.0)],
        }
    }

    fn levels() -> Levels {
        let mut l = Levels::default();
        l.master.gamma = 1.4;
        l.master.output_black = 0.05;
        l.red.input_black = 0.1;
        l.red.input_white = 0.9;
        l.green.gamma = 0.6;
        l.blue.output_white = 0.2;
        l.blue.output_black = 0.8;
        l
    }

    fn all_adjustments() -> Vec<Adjustment> {
        vec![
            Adjustment::BrightnessContrast(BrightnessContrast {
                brightness: 0.2,
                contrast: 0.3,
            }),
            Adjustment::BrightnessContrast(BrightnessContrast {
                brightness: -0.3,
                contrast: 1.0,
            }),
            Adjustment::Levels(levels()),
            Adjustment::Curves(curves()),
            Adjustment::HueSaturation(HueSaturation {
                hue: 40.0,
                saturation: 0.3,
                lightness: -0.2,
            }),
            Adjustment::HueSaturation(HueSaturation {
                hue: -120.0,
                saturation: -0.5,
                lightness: 0.4,
            }),
        ]
    }

    /// Blend modes with a step in the formula, where a float error in an
    /// input can give a large change of the result:
    ///
    /// - Color burn (and vivid light) at `cb = 1`, `cs -> 0`: the result is
    ///   1 for `cb >= 1`, but about 0 for `cb = 1 - ulp`. Color dodge (and
    ///   vivid light) has the same step at `cb = 0`, `cs -> 1`.
    /// - Hard mix at `cb + cs = 1`.
    /// - Darker color and lighter color when both luminances are equal.
    ///
    /// Values exactly at 1.0 are frequent in the tests (the clamp of
    /// values above 1, and adjustment results at white).
    const STEP_MODES: [BlendMode; 6] = [
        BlendMode::ColorBurn,
        BlendMode::ColorDodge,
        BlendMode::VividLight,
        BlendMode::HardMix,
        BlendMode::DarkerColor,
        BlendMode::LighterColor,
    ];

    /// Compares the GPU and CPU composites of `region` and returns the
    /// largest scaled error of the samples inside the tolerance.
    ///
    /// If `steps` is true (the document uses a mode in [`STEP_MODES`]), at
    /// most 1 sample in 1000 (and at least 1) can be outside the tolerance.
    /// Else all samples must be inside it.
    fn compare(
        ctx: &GpuContext,
        comp: &mut GpuCompositor,
        doc: &Document,
        region: Rect,
        steps: bool,
        tol: f32,
        what: &str,
    ) -> f32 {
        let cpu = composite(doc, region).unwrap();
        let gpu = pollster::block_on(comp.composite_gpu_image(ctx, doc, region)).unwrap();
        assert_eq!((gpu.width(), gpu.height()), (region.width, region.height));
        let mut worst = 0.0_f32;
        let mut bad = Vec::new();
        for (i, (c, g)) in cpu.as_slice().iter().zip(gpu.as_slice()).enumerate() {
            let err = (g - c).abs() / c.abs().max(1.0);
            assert!(g.is_finite(), "{what}: sample {i} is not finite ({g})");
            if err > tol {
                bad.push((i / 4, i % 4, *c, *g));
            } else {
                worst = worst.max(err);
            }
        }
        let allowed = if steps {
            cpu.as_slice().len() / 1000 + 1
        } else {
            0
        };
        if !bad.is_empty() {
            eprintln!("{what}: {} samples outside the tolerance", bad.len());
        }
        assert!(
            bad.len() <= allowed,
            "{what}: {} samples outside the tolerance, first: {:?}",
            bad.len(),
            &bad[..bad.len().min(5)]
        );
        worst
    }

    #[test]
    fn mode_ids_are_the_menu_index() {
        for (i, mode) in BlendMode::ALL.iter().enumerate() {
            assert_eq!(mode_id(*mode) as usize, i, "{mode:?}");
        }
    }

    #[test]
    fn params_block_size_agrees_with_the_shader() {
        // 5 vec4 fields and an array of 5 vec4: 10 * 16 bytes.
        assert_eq!(std::mem::size_of::<Params>(), 160);
    }

    #[test]
    fn invalid_region_is_rejected() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let doc = empty_doc(8, 8, BlendSpace::Perceptual);
        for region in [Rect::new(0, 0, 0, 4), Rect::new(4, 4, 5, 2)] {
            let result = pollster::block_on(comp.composite_gpu(&ctx, &doc, region));
            assert!(
                matches!(result, Err(EditorError::InvalidRect { .. })),
                "{region:?}"
            );
        }
    }

    #[test]
    fn empty_document_is_transparent() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let doc = empty_doc(9, 7, BlendSpace::Perceptual);
        let image = pollster::block_on(comp.composite_gpu_image(&ctx, &doc, doc.bounds())).unwrap();
        assert!(image.as_slice().iter().all(|&v| v == 0.0));
    }

    /// All blend modes in both blend spaces, for pixel layers, adjustment
    /// layers, isolated groups, and pass-through groups, with masks and
    /// offsets.
    #[test]
    fn all_blend_modes_match_cpu() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(0x1234_5678_9ABC_DEF0);
        let mut worst = 0.0_f32;
        for space in [BlendSpace::Linear, BlendSpace::Perceptual] {
            for (n, mode) in BlendMode::ALL.into_iter().enumerate() {
                let mut doc = empty_doc(40, 32, space);
                add_pixel(&mut doc, &mut rng, None, (40, 32), (0, 0));

                // Pixel layer partly outside the canvas, with a black-fill mask.
                let p = add_pixel(&mut doc, &mut rng, None, (30, 20), (-6, 5));
                doc.set_blend_mode(p, mode).unwrap();
                doc.set_opacity(p, 0.85).unwrap();
                doc.set_fill_opacity(p, 0.9).unwrap();
                set_mask(&mut doc, &mut rng, p, (20, 14), (2, 8), MaskFill::Black);

                // Adjustment layer with a white-fill mask.
                let adjustments = all_adjustments();
                let adj = adjustments[n % adjustments.len()].clone();
                let a = add(&mut doc, None, LayerContent::Adjustment(adj));
                doc.set_blend_mode(a, mode).unwrap();
                doc.set_opacity(a, 0.7).unwrap();
                set_mask(&mut doc, &mut rng, a, (25, 18), (12, -3), MaskFill::White);

                // Isolated group.
                let g = add(
                    &mut doc,
                    None,
                    LayerContent::Group(GroupIsolation::Isolated),
                );
                doc.set_blend_mode(g, mode).unwrap();
                doc.set_opacity(g, 0.9).unwrap();
                add_pixel(&mut doc, &mut rng, Some(g), (25, 25), (10, -4));
                let c = add_pixel(&mut doc, &mut rng, Some(g), (16, 16), (20, 10));
                doc.set_blend_mode(c, BlendMode::Screen).unwrap();

                // Pass-through group with a mask, an adjustment, and a layer.
                let t = add(
                    &mut doc,
                    None,
                    LayerContent::Group(GroupIsolation::PassThrough),
                );
                doc.set_opacity(t, 0.6).unwrap();
                set_mask(&mut doc, &mut rng, t, (30, 30), (5, 5), MaskFill::White);
                let ta = add(
                    &mut doc,
                    Some(t),
                    LayerContent::Adjustment(Adjustment::Levels(levels())),
                );
                doc.set_blend_mode(ta, mode).unwrap();
                let tp = add_pixel(&mut doc, &mut rng, Some(t), (12, 30), (30, 4));
                doc.set_blend_mode(tp, mode).unwrap();

                for region in [doc.bounds(), Rect::new(5, 3, 30, 22)] {
                    let what = format!("{space:?} {mode:?} {region:?}");
                    let steps = STEP_MODES.contains(&mode);
                    let tol = if mode == BlendMode::Divide {
                        DIVIDE_TOL
                    } else {
                        TOL
                    };
                    worst = worst.max(compare(&ctx, &mut comp, &doc, region, steps, tol, &what));
                }
            }
        }
        eprintln!("all_blend_modes_match_cpu: largest scaled error {worst:e}");
    }

    /// The layer texture cache stores layer pixels and masks as `f16`.
    /// Values that are not `f16` values thus change by at most `2^-12`
    /// relative. This test measures the effect on the composite against
    /// the CPU reference (with the `f32` values), for each group of blend
    /// modes. The step modes and the divide mode are not in the test: near
    /// a step, the input rounding can select the other branch.
    ///
    /// Measured (scaled error, see [`compare`]): separable modes at most
    /// about 8.8e-4 (normal 5.0e-4, difference 8.8e-4); non-separable modes
    /// at most about 3.0e-3 (hue 3.0e-3, saturation 2.3e-3). The hue and
    /// saturation of a color near gray change much for a small change of
    /// the values.
    #[test]
    fn f16_layer_storage_error_is_small() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(0x0F16_0F16);
        let groups: [(&str, &[BlendMode], f32); 2] = [
            (
                "separable",
                &[
                    BlendMode::Normal,
                    BlendMode::Darken,
                    BlendMode::Multiply,
                    BlendMode::LinearBurn,
                    BlendMode::Lighten,
                    BlendMode::Screen,
                    BlendMode::LinearDodge,
                    BlendMode::Overlay,
                    BlendMode::SoftLight,
                    BlendMode::HardLight,
                    BlendMode::LinearLight,
                    BlendMode::PinLight,
                    BlendMode::Difference,
                    BlendMode::Exclusion,
                    BlendMode::Subtract,
                ],
                2e-3,
            ),
            (
                "non-separable",
                &[
                    BlendMode::Hue,
                    BlendMode::Saturation,
                    BlendMode::Color,
                    BlendMode::Luminosity,
                ],
                6e-3,
            ),
        ];
        for (name, modes, tol) in groups {
            let mut worst = 0.0_f32;
            for &mode in modes {
                for space in [BlendSpace::Linear, BlendSpace::Perceptual] {
                    let mut doc = empty_doc(48, 40, space);
                    let base = random_image_f32(&mut rng, 48, 40);
                    add(
                        &mut doc,
                        None,
                        LayerContent::Pixel(PixelLayer::new(base, (0, 0)).unwrap()),
                    );
                    let top = random_image_f32(&mut rng, 40, 30);
                    let id = add(
                        &mut doc,
                        None,
                        LayerContent::Pixel(PixelLayer::new(top, (4, 6)).unwrap()),
                    );
                    doc.set_blend_mode(id, mode).unwrap();
                    doc.set_opacity(id, 0.8).unwrap();
                    let data = (0..30 * 20).map(|_| rng.range(0.0, 1.0)).collect();
                    let mask = MaskImage::from_data(30, 20, data).unwrap();
                    doc.set_mask(id, Some(Mask::new(mask, (8, 10), MaskFill::White)))
                        .unwrap();
                    let what = format!("{name}: {space:?} {mode:?}");
                    let err = compare(&ctx, &mut comp, &doc, doc.bounds(), false, tol, &what);
                    worst = worst.max(err);
                }
            }
            eprintln!("f16_layer_storage_error_is_small: {name}: largest scaled error {worst:e}");
        }
    }

    /// All adjustment kinds, with several blend modes, in both spaces.
    #[test]
    fn adjustments_match_cpu() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(0x0BAD_CAFE_1234_5678);
        let mut worst = 0.0_f32;
        let modes = [
            BlendMode::Normal,
            BlendMode::Multiply,
            BlendMode::Dissolve,
            BlendMode::Color,
            BlendMode::Overlay,
        ];
        for space in [BlendSpace::Linear, BlendSpace::Perceptual] {
            for adj in all_adjustments() {
                for mode in modes {
                    let mut doc = empty_doc(48, 40, space);
                    add_pixel(&mut doc, &mut rng, None, (48, 40), (0, 0));
                    let a = add(&mut doc, None, LayerContent::Adjustment(adj.clone()));
                    doc.set_blend_mode(a, mode).unwrap();
                    doc.set_opacity(a, 0.8).unwrap();
                    let what = format!("{space:?} {mode:?} {adj:?}");
                    worst = worst.max(compare(
                        &ctx,
                        &mut comp,
                        &doc,
                        doc.bounds(),
                        false,
                        TOL,
                        &what,
                    ));
                }
            }
        }
        eprintln!("adjustments_match_cpu: largest scaled error {worst:e}");
    }

    /// Hidden and zero-opacity layers, disabled masks, masks and layers
    /// outside the region, nested groups, and dissolve at all levels.
    #[test]
    fn special_cases_match_cpu() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(0x5EED_5EED_5EED_5EED);
        let mut worst = 0.0_f32;
        for space in [BlendSpace::Linear, BlendSpace::Perceptual] {
            let mut doc = empty_doc(64, 48, space);
            add_pixel(&mut doc, &mut rng, None, (64, 48), (0, 0));

            // Ignored layers.
            let hidden = add_pixel(&mut doc, &mut rng, None, (64, 48), (0, 0));
            doc.set_visible(hidden, false).unwrap();
            let zero = add_pixel(&mut doc, &mut rng, None, (64, 48), (0, 0));
            doc.set_opacity(zero, 0.0).unwrap();
            let zero_fill = add_pixel(&mut doc, &mut rng, None, (64, 48), (0, 0));
            doc.set_fill_opacity(zero_fill, 0.0).unwrap();

            // Disabled mask: no effect.
            let dm = add_pixel(&mut doc, &mut rng, None, (40, 30), (10, 10));
            set_mask(&mut doc, &mut rng, dm, (10, 10), (0, 0), MaskFill::Black);
            doc.set_mask_enabled(dm, false).unwrap();

            // Mask fully outside the region (black fill): the layer is hidden
            // in the region. Mask fully outside (white fill): no mask effect.
            let mo = add_pixel(&mut doc, &mut rng, None, (64, 48), (0, 0));
            set_mask(&mut doc, &mut rng, mo, (8, 8), (-20, -20), MaskFill::Black);
            let mw = add_pixel(&mut doc, &mut rng, None, (30, 30), (-10, 20));
            doc.set_blend_mode(mw, BlendMode::SoftLight).unwrap();
            set_mask(&mut doc, &mut rng, mw, (8, 8), (100, 100), MaskFill::White);

            // Layer fully outside the canvas.
            add_pixel(&mut doc, &mut rng, None, (10, 10), (70, 0));

            // Dissolve pixel layer with a mask.
            let d = add_pixel(&mut doc, &mut rng, None, (50, 40), (7, 3));
            doc.set_blend_mode(d, BlendMode::Dissolve).unwrap();
            doc.set_opacity(d, 0.6).unwrap();
            set_mask(&mut doc, &mut rng, d, (30, 30), (12, 6), MaskFill::White);

            // Isolated group > pass-through group > isolated group, with an
            // adjustment in the pass-through group that changes the layers
            // below it in the outer isolated group only.
            let outer = add(
                &mut doc,
                None,
                LayerContent::Group(GroupIsolation::Isolated),
            );
            doc.set_blend_mode(outer, BlendMode::Dissolve).unwrap();
            doc.set_opacity(outer, 0.75).unwrap();
            add_pixel(&mut doc, &mut rng, Some(outer), (50, 40), (3, 2));
            let pass = add(
                &mut doc,
                Some(outer),
                LayerContent::Group(GroupIsolation::PassThrough),
            );
            doc.set_opacity(pass, 0.8).unwrap();
            set_mask(&mut doc, &mut rng, pass, (40, 20), (8, 20), MaskFill::Black);
            let pa = add(
                &mut doc,
                Some(pass),
                LayerContent::Adjustment(Adjustment::Curves(curves())),
            );
            doc.set_blend_mode(pa, BlendMode::Dissolve).unwrap();
            doc.set_opacity(pa, 0.5).unwrap();
            let inner = add(
                &mut doc,
                Some(pass),
                LayerContent::Group(GroupIsolation::Isolated),
            );
            doc.set_blend_mode(inner, BlendMode::Luminosity).unwrap();
            add_pixel(&mut doc, &mut rng, Some(inner), (20, 20), (30, 20));
            let ia = add(
                &mut doc,
                Some(inner),
                LayerContent::Adjustment(Adjustment::HueSaturation(HueSaturation {
                    hue: 90.0,
                    saturation: 0.5,
                    lightness: 0.1,
                })),
            );
            doc.set_blend_mode(ia, BlendMode::Hue).unwrap();
            // An empty pass-through group and an empty isolated group.
            add(
                &mut doc,
                Some(outer),
                LayerContent::Group(GroupIsolation::PassThrough),
            );
            add(
                &mut doc,
                None,
                LayerContent::Group(GroupIsolation::Isolated),
            );

            for region in [
                doc.bounds(),
                Rect::new(9, 5, 40, 33),
                Rect::new(63, 47, 1, 1),
            ] {
                let what = format!("special cases {space:?} {region:?}");
                worst = worst.max(compare(&ctx, &mut comp, &doc, region, false, TOL, &what));
            }
        }
        eprintln!("special_cases_match_cpu: largest scaled error {worst:e}");
    }

    /// The composite of a region is the crop of the full composite (also
    /// for dissolve, which uses canvas coordinates).
    #[test]
    fn region_is_crop_of_full_composite() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(42);
        let mut doc = empty_doc(32, 24, BlendSpace::Perceptual);
        add_pixel(&mut doc, &mut rng, None, (32, 24), (0, 0));
        let d = add_pixel(&mut doc, &mut rng, None, (32, 24), (0, 0));
        doc.set_blend_mode(d, BlendMode::Dissolve).unwrap();
        doc.set_opacity(d, 0.5).unwrap();
        let full = pollster::block_on(comp.composite_gpu_image(&ctx, &doc, doc.bounds())).unwrap();
        let region = Rect::new(7, 5, 13, 11);
        let part = pollster::block_on(comp.composite_gpu_image(&ctx, &doc, region)).unwrap();
        for y in 0..region.height {
            for x in 0..region.width {
                assert_eq!(
                    part.pixel(x, y),
                    full.pixel(x + region.x, y + region.y),
                    "({x}, {y})"
                );
            }
        }
    }

    /// The layer cache gives the same result as the CPU after each kind of
    /// change, and uploads only the values that changed.
    #[test]
    fn layer_cache_matches_cpu_and_skips_uploads() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(11);
        let mut doc = empty_doc(40, 30, BlendSpace::Perceptual);
        let a = add_pixel(&mut doc, &mut rng, None, (40, 30), (0, 0));
        // `b` is fully inside the canvas; `c` is partly outside it.
        let b = add_pixel(&mut doc, &mut rng, None, (20, 16), (5, 4));
        set_mask(&mut doc, &mut rng, b, (18, 12), (6, 5), MaskFill::Black);
        let c = add_pixel(&mut doc, &mut rng, None, (30, 20), (25, -6));
        doc.set_blend_mode(c, BlendMode::Multiply).unwrap();
        let g = add(
            &mut doc,
            None,
            LayerContent::Group(GroupIsolation::Isolated),
        );
        let d = add_pixel(&mut doc, &mut rng, Some(g), (16, 16), (12, 10));
        set_mask(&mut doc, &mut rng, g, (40, 30), (0, 0), MaskFill::White);
        add(
            &mut doc,
            None,
            LayerContent::Adjustment(Adjustment::Curves(curves())),
        );

        let check = |comp: &mut GpuCompositor, doc: &Document, what: &str| {
            compare(&ctx, comp, doc, doc.bounds(), false, TOL, what);
            compare(&ctx, comp, doc, Rect::new(3, 2, 17, 13), false, TOL, what);
        };

        // a, b, mask of b, c, d, mask of g. The region composite uses the
        // cached canvas parts.
        check(&mut comp, &doc, "initial");
        let mut uploads = comp.layer_uploads();
        assert_eq!(uploads, 6);

        // Property changes do not upload.
        doc.set_opacity(a, 0.6).unwrap();
        doc.set_blend_mode(b, BlendMode::Screen).unwrap();
        doc.set_fill_opacity(c, 0.7).unwrap();
        doc.set_visible(d, false).unwrap();
        doc.set_mask_enabled(b, false).unwrap();
        check(&mut comp, &doc, "props");
        doc.set_visible(d, true).unwrap();
        doc.set_mask_enabled(b, true).unwrap();
        check(&mut comp, &doc, "props back");
        assert_eq!(comp.layer_uploads(), uploads);

        // A pixel write updates the changed area of the layer once (a
        // partial update); a mask write the mask once.
        let written = [0.3, 0.2, 0.1, 0.5].map(to_f16);
        doc.write_pixels(b, Rect::new(2, 3, 4, 2), &[written; 8])
            .unwrap();
        check(&mut comp, &doc, "pixel write");
        let mut partial = 1;
        assert_eq!(comp.layer_uploads(), uploads);
        assert_eq!(comp.layer_partial_uploads(), partial);
        doc.write_mask(g, Rect::new(10, 10, 5, 5), &[0.25; 25])
            .unwrap();
        check(&mut comp, &doc, "mask write");
        partial += 1;
        assert_eq!(comp.layer_uploads(), uploads);
        assert_eq!(comp.layer_partial_uploads(), partial);

        // A move inside the canvas does not upload. A move of a layer that
        // is partly outside the canvas changes its canvas part: one upload.
        doc.translate_layer(b, 3, -2).unwrap();
        check(&mut comp, &doc, "move inside");
        assert_eq!(comp.layer_uploads(), uploads);
        doc.translate_layer(c, -3, 2).unwrap();
        check(&mut comp, &doc, "move at edge");
        uploads += 1;
        assert_eq!(comp.layer_uploads(), uploads);

        // Delete drops the entry; undo of the delete uploads again.
        let mut history = History::default();
        let e = doc.delete_layer(d).unwrap();
        history.push("Delete", vec![e]);
        check(&mut comp, &doc, "delete");
        assert_eq!(comp.layer_uploads(), uploads);
        history.undo(&mut doc).unwrap();
        check(&mut comp, &doc, "undo delete");
        uploads += 1;
        assert_eq!(comp.layer_uploads(), uploads);

        // A write and its undo give new values: one partial update.
        let e = doc
            .write_pixels(a, Rect::new(0, 0, 2, 2), &[[0.0, 0.0, 0.0, 1.0]; 4])
            .unwrap();
        history.push("Write", vec![e]);
        history.undo(&mut doc).unwrap();
        check(&mut comp, &doc, "undo write");
        partial += 1;
        assert_eq!(comp.layer_uploads(), uploads);
        assert_eq!(comp.layer_partial_uploads(), partial);

        // A new mask has no known changed area: one full upload.
        let mask = MaskImage::new_filled(40, 30, 0.5).unwrap();
        doc.set_mask(g, Some(Mask::new(mask, (0, 0), MaskFill::White)))
            .unwrap();
        check(&mut comp, &doc, "mask replaced");
        uploads += 1;
        assert_eq!(comp.layer_uploads(), uploads);
        assert_eq!(comp.layer_partial_uploads(), partial);

        // Mask removal does not upload.
        doc.set_mask(b, None).unwrap();
        check(&mut comp, &doc, "mask removed");
        assert_eq!(comp.layer_uploads(), uploads);

        // `clear_cache` frees all textures except the placeholder.
        assert!(comp.cache_memory_bytes() > 40 * 30 * 16);
        comp.clear_cache();
        assert_eq!(comp.cache_memory_bytes(), 16);
        check(&mut comp, &doc, "after clear");
        // a, b, c, d, mask of g.
        assert_eq!(comp.layer_uploads(), uploads + 5);
    }

    /// Curve end points that are not on a table sample (`x` not a multiple
    /// of `1 / 4096`). The table interpolation rounds the corner at the end
    /// point. The module documentation gives the bound: `slope / 16384`
    /// in the encoded values.
    #[test]
    fn curve_end_points_between_table_samples() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        // The sRGB decode: the adjustment reads the encoded values.
        let decode = |v: f32| {
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let (lo, hi) = (0.1003_f32, 0.9001_f32);
        let cell = 1.0 / 4096.0;
        let (w, h) = (256_u32, 64_u32);
        let n = (w * h) as f32;
        // Red: dense near `lo`; green: dense near `hi`; blue: a full ramp.
        let pixels = (0..w * h)
            .map(|i| {
                let t = i as f32 / n;
                let near = (t - 0.5) * 4.0 * cell;
                // `f16` values: the test is about the table error only.
                [decode(lo + near), decode(hi + near), decode(t), 1.0].map(to_f16)
            })
            .collect();
        let image = WorkingImage::from_pixels(w, h, pixels).unwrap();
        let mut doc = empty_doc(w, h, BlendSpace::Perceptual);
        add(
            &mut doc,
            None,
            LayerContent::Pixel(PixelLayer::new(image, (0, 0)).unwrap()),
        );
        let line = vec![CurvePoint::new(lo, 0.0), CurvePoint::new(hi, 1.0)];
        let c = Curves {
            master: vec![
                CurvePoint::new(0.000_07, 0.0),
                CurvePoint::new(0.999_93, 1.0),
            ],
            red: line.clone(),
            green: line,
            blue: vec![
                CurvePoint::new(0.0301, 0.1),
                CurvePoint::new(0.5, 0.6),
                CurvePoint::new(0.9702, 0.95),
            ],
        };
        add(
            &mut doc,
            None,
            LayerContent::Adjustment(Adjustment::Curves(c)),
        );

        // The largest slope is about 1.25 (red and green). The decode at
        // the output multiplies the error by at most about 2.3.
        let slope = 1.0 / (hi - lo) * 1.0001;
        let bound = 2.3 * slope / 16384.0 + TOL;
        let cpu = composite(&doc, doc.bounds()).unwrap();
        let gpu = pollster::block_on(comp.composite_gpu_image(&ctx, &doc, doc.bounds())).unwrap();
        let worst = cpu
            .as_slice()
            .iter()
            .zip(gpu.as_slice())
            .map(|(c, g)| (g - c).abs())
            .fold(0.0_f32, f32::max);
        eprintln!(
            "curve_end_points_between_table_samples: largest error {worst:e}, bound {bound:e}"
        );
        assert!(worst <= bound, "largest error {worst:e} above {bound:e}");
        // The samples near the end points show the interpolation error
        // (measured: about 8.7e-5 with these `f16` inputs, and 1.7e-4 with
        // denser `f32` inputs near the end points). It is larger than the
        // tolerance of the smooth parts.
        assert!(worst > TOL, "the test does not reach an end point");
    }

    #[test]
    fn curve_table_uploads_only_after_a_curve_change() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(17);
        let mut doc = empty_doc(24, 20, BlendSpace::Perceptual);
        add_pixel(&mut doc, &mut rng, None, (24, 20), (0, 0));
        let a = add(
            &mut doc,
            None,
            LayerContent::Adjustment(Adjustment::Curves(curves())),
        );
        let check = |comp: &mut GpuCompositor, doc: &Document, what: &str| {
            compare(&ctx, comp, doc, doc.bounds(), false, TOL, what);
            compare(&ctx, comp, doc, Rect::new(2, 3, 10, 9), false, TOL, what);
        };

        check(&mut comp, &doc, "initial");
        assert_eq!(comp.curve_uploads(), 1);
        // Property changes and equal curves do not upload.
        doc.set_opacity(a, 0.5).unwrap();
        doc.set_blend_mode(a, BlendMode::Overlay).unwrap();
        doc.set_adjustment(a, Adjustment::Curves(curves())).unwrap();
        check(&mut comp, &doc, "props");
        assert_eq!(comp.curve_uploads(), 1);
        // New curves upload once.
        let mut changed = curves();
        changed.green[1] = CurvePoint::new(1.0, 0.7);
        doc.set_adjustment(a, Adjustment::Curves(changed.clone()))
            .unwrap();
        check(&mut comp, &doc, "new curves");
        check(&mut comp, &doc, "new curves again");
        assert_eq!(comp.curve_uploads(), 2);
        // A different adjustment kind drops the table; curves again upload.
        let before = comp.cache_memory_bytes();
        doc.set_adjustment(a, Adjustment::Levels(levels())).unwrap();
        check(&mut comp, &doc, "levels");
        assert!(
            comp.cache_memory_bytes() < before,
            "the table was not dropped"
        );
        doc.set_adjustment(a, Adjustment::Curves(changed)).unwrap();
        check(&mut comp, &doc, "curves again");
        assert_eq!(comp.curve_uploads(), 3);
        // Two curves layers have one table each.
        let b = add(
            &mut doc,
            None,
            LayerContent::Adjustment(Adjustment::Curves(curves())),
        );
        doc.set_opacity(b, 0.6).unwrap();
        check(&mut comp, &doc, "two layers");
        check(&mut comp, &doc, "two layers again");
        assert_eq!(comp.curve_uploads(), 4);
    }

    fn view(origin_x: f64, origin_y: f64, scale: f64, width: u32, height: u32) -> ViewSpec {
        ViewSpec {
            origin_x,
            origin_y,
            scale,
            width,
            height,
        }
    }

    /// Renders a view and reads it back.
    fn read_view(
        ctx: &GpuContext,
        comp: &mut GpuCompositor,
        doc: &Document,
        spec: ViewSpec,
    ) -> Vec<[f32; 4]> {
        let image = pollster::block_on(comp.render_view(ctx, doc, &spec)).unwrap();
        assert_eq!((image.width(), image.height()), (spec.width, spec.height));
        let data = pollster::block_on(image.read_rgba_f32(ctx)).unwrap();
        data.chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect()
    }

    /// Checks a straight view pixel against a premultiplied CPU pixel.
    fn assert_view_pixel(got: [f32; 4], cpu: [f32; 4], what: &str) {
        let a = got[3];
        let premultiplied = [got[0] * a, got[1] * a, got[2] * a, a];
        for (g, c) in premultiplied.iter().zip(cpu) {
            assert!(
                (g - c).abs() <= TOL * c.abs().max(1.0),
                "{what}: {got:?} (premultiplied {premultiplied:?}) against {cpu:?}"
            );
        }
        if a > 0.1 {
            // The color is straight (not premultiplied).
            for (g, c) in got.iter().zip(cpu).take(3) {
                assert!(
                    (g - c / cpu[3]).abs() <= 1e-3 * (c / cpu[3]).abs().max(1.0),
                    "{what}"
                );
            }
        }
    }

    /// A document with one canvas-size pixel layer.
    fn flat_doc(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [f32; 4]) -> Document {
        let mut doc = empty_doc(width, height, BlendSpace::Perceptual);
        let pixels = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| pixel(x, y))
            .collect();
        let image = WorkingImage::from_pixels(width, height, pixels).unwrap();
        add(
            &mut doc,
            None,
            LayerContent::Pixel(PixelLayer::new(image, (0, 0)).unwrap()),
        );
        doc
    }

    #[test]
    fn render_view_at_scale_one_is_the_straight_crop() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(5);
        let mut doc = empty_doc(24, 20, BlendSpace::Perceptual);
        add_pixel(&mut doc, &mut rng, None, (24, 20), (0, 0));
        let top = add_pixel(&mut doc, &mut rng, None, (10, 10), (8, 4));
        doc.set_opacity(top, 0.7).unwrap();
        doc.set_blend_mode(top, BlendMode::Overlay).unwrap();
        let cpu = composite(&doc, doc.bounds()).unwrap();
        // The left 3 columns and the bottom 5 rows are outside the canvas.
        let out = read_view(&ctx, &mut comp, &doc, view(-3.0, 5.0, 1.0, 30, 20));
        for y in 0..20_i64 {
            for x in 0..30_i64 {
                let got = out[(y * 30 + x) as usize];
                let (cx, cy) = (x - 3, y + 5);
                if (0..24).contains(&cx) && (0..20).contains(&cy) {
                    let c = cpu.pixel(cx as u32, cy as u32).unwrap();
                    assert_view_pixel(got, c, &format!("({x}, {y})"));
                } else {
                    assert_eq!(got, [0.0; 4], "({x}, {y}) is outside the canvas");
                }
            }
        }
    }

    #[test]
    fn render_view_nearest_neighbor_when_zoomed_in() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(9);
        let mut doc = empty_doc(8, 8, BlendSpace::Perceptual);
        add_pixel(&mut doc, &mut rng, None, (8, 8), (0, 0));
        let cpu = composite(&doc, doc.bounds()).unwrap();
        // Origin (2, 1), zoom 4: each canvas pixel is a 4 x 4 block.
        let out = read_view(&ctx, &mut comp, &doc, view(2.0, 1.0, 4.0, 16, 16));
        for y in 0..16_u32 {
            for x in 0..16_u32 {
                let got = out[(y * 16 + x) as usize];
                let c = cpu.pixel(2 + x / 4, 1 + y / 4).unwrap();
                assert_view_pixel(got, c, &format!("({x}, {y})"));
                assert_eq!(got, out[((y - y % 4) * 16 + x - x % 4) as usize]);
            }
        }
        // A fractional origin moves the block edges: output pixel 0 and 1
        // show canvas column 2, pixel 2 shows column 3.
        let out = read_view(&ctx, &mut comp, &doc, view(2.5, 1.0, 4.0, 4, 1));
        assert_eq!(out[0], out[1]);
        assert_view_pixel(out[1], cpu.pixel(2, 1).unwrap(), "fraction 1");
        assert_view_pixel(out[2], cpu.pixel(3, 1).unwrap(), "fraction 2");
    }

    #[test]
    fn render_view_downscale_keeps_a_flat_color() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        for (color, alpha) in [([0.2, 0.5, 0.9], 1.0), ([0.8, 0.1, 1.5], 0.5)] {
            let premultiplied = [color[0] * alpha, color[1] * alpha, color[2] * alpha, alpha];
            let doc = flat_doc(64, 48, |_, _| premultiplied);
            for scale in [0.9, 0.5, 0.3, 0.25, 0.1] {
                // Keep the output pixels inside the canvas.
                let w = (64.0 * scale) as u32 - 1;
                let h = (48.0 * scale) as u32 - 1;
                let out = read_view(&ctx, &mut comp, &doc, view(0.0, 0.0, scale, w, h));
                for (i, p) in out.iter().enumerate() {
                    let want = [color[0], color[1], color[2], alpha];
                    for (g, e) in p.iter().zip(want) {
                        let err = (g - e).abs() / e.abs().max(1.0);
                        assert!(
                            err <= VIEW_TOL,
                            "scale {scale}, pixel {i}: {p:?} against {want:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn render_view_downscale_of_a_checkerboard_is_gray() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let doc = flat_doc(64, 64, |x, y| {
            if (x + y) % 2 == 0 {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [0.0, 0.0, 0.0, 1.0]
            }
        });
        let cases = [
            (0.0, 0.0, 0.5, 1e-5),
            (1.0, 0.0, 0.5, 1e-5),
            (0.5, 0.25, 0.5, 1e-4),
            (0.0, 0.0, 0.37, 1e-4),
            (3.0, 1.0, 0.25, 1e-4),
            // Level 0 with a footprint of 4/3 pixels: an area mean, not
            // exact gray, but never a black or white sample.
            (0.0, 0.0, 0.75, 0.15),
        ];
        for (ox, oy, scale, tol) in cases {
            let size = (64.0 * scale) as u32 - 2;
            let out = read_view(&ctx, &mut comp, &doc, view(ox, oy, scale, size, size));
            for (i, p) in out.iter().enumerate() {
                assert!((p[3] - 1.0).abs() <= 1e-5, "{scale}: alpha {p:?}");
                for v in &p[..3] {
                    assert!(
                        (v - 0.5).abs() <= tol,
                        "origin ({ox}, {oy}), scale {scale}, pixel {i}: {p:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn render_view_composites_only_after_a_change() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(3);
        let mut doc = empty_doc(32, 32, BlendSpace::Perceptual);
        add_pixel(&mut doc, &mut rng, None, (32, 32), (0, 0));
        let top = add_pixel(&mut doc, &mut rng, None, (16, 16), (4, 4));

        read_view(&ctx, &mut comp, &doc, view(0.0, 0.0, 1.0, 32, 32));
        assert_eq!(comp.view_composites(), 1);
        // Pan and zoom only: no composite.
        read_view(&ctx, &mut comp, &doc, view(-5.5, 3.0, 1.0, 20, 10));
        read_view(&ctx, &mut comp, &doc, view(0.0, 0.0, 0.3, 9, 9));
        read_view(&ctx, &mut comp, &doc, view(1.0, 2.0, 8.0, 64, 64));
        assert_eq!(comp.view_composites(), 1);
        // A clone has the same revision.
        let copy = doc.clone();
        read_view(&ctx, &mut comp, &copy, view(0.0, 0.0, 1.0, 32, 32));
        assert_eq!(comp.view_composites(), 1);

        // A change composites again, without an upload.
        let uploads = comp.layer_uploads();
        doc.set_opacity(top, 0.3).unwrap();
        let out = read_view(&ctx, &mut comp, &doc, view(0.0, 0.0, 1.0, 32, 32));
        assert_eq!(comp.view_composites(), 2);
        assert_eq!(comp.layer_uploads(), uploads);
        let cpu = composite(&doc, doc.bounds()).unwrap();
        for (i, got) in out.iter().enumerate() {
            let (x, y) = (i as u32 % 32, i as u32 / 32);
            assert_view_pixel(*got, cpu.pixel(x, y).unwrap(), "after change");
        }

        // A different document size makes a new pyramid.
        let small = flat_doc(5, 3, |_, _| [0.5, 0.5, 0.5, 1.0]);
        let out = read_view(&ctx, &mut comp, &small, view(0.0, 0.0, 0.5, 2, 1));
        assert_eq!(comp.view_composites(), 3);
        assert!((out[0][0] - 0.5).abs() < 1e-6);
    }

    /// Renders a mask view and reads it back.
    fn read_mask_view(
        ctx: &GpuContext,
        comp: &mut GpuCompositor,
        doc: &Document,
        spec: ViewSpec,
        layer: LayerId,
    ) -> Vec<[f32; 4]> {
        let image = pollster::block_on(comp.render_mask_view(ctx, doc, &spec, layer)).unwrap();
        assert_eq!((image.width(), image.height()), (spec.width, spec.height));
        let data = pollster::block_on(image.read_rgba_f32(ctx)).unwrap();
        data.chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect()
    }

    /// The expected mask view pixel for the mask value `v`.
    fn mask_gray(v: f32) -> [f32; 4] {
        let g = TransferFunction::Srgb.decode(v);
        [g, g, g, 1.0]
    }

    fn assert_mask_pixel(got: [f32; 4], want: [f32; 4], tol: f32, what: &str) {
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() <= tol, "{what}: {got:?} against {want:?}");
        }
    }

    #[test]
    fn render_mask_view_at_scale_one_is_the_straight_crop() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(21);
        let mut doc = empty_doc(24, 20, BlendSpace::Perceptual);
        let a = add_pixel(&mut doc, &mut rng, None, (24, 20), (0, 0));
        let b = add_pixel(&mut doc, &mut rng, None, (12, 12), (6, 2));
        // Mask of `a`: smaller than the canvas, partly outside it, black
        // fill. Mask of `b`: disabled, white fill.
        set_mask(&mut doc, &mut rng, a, (10, 6), (-3, 4), MaskFill::Black);
        set_mask(&mut doc, &mut rng, b, (8, 9), (10, 8), MaskFill::White);
        doc.set_mask_enabled(b, false).unwrap();
        for id in [a, b] {
            let mask = doc.layer(id).unwrap().mask().unwrap().clone();
            // The left 3 columns and the bottom 5 rows are outside the canvas.
            let out = read_mask_view(&ctx, &mut comp, &doc, view(-3.0, 5.0, 1.0, 30, 20), id);
            for y in 0..20_i64 {
                for x in 0..30_i64 {
                    let got = out[(y * 30 + x) as usize];
                    let (cx, cy) = (x - 3, y + 5);
                    if !((0..24).contains(&cx) && (0..20).contains(&cy)) {
                        assert_eq!(got, [0.0; 4], "({x}, {y}) is outside the canvas");
                        continue;
                    }
                    let (mx, my) = (cx - i64::from(mask.offset.0), cy - i64::from(mask.offset.1));
                    let inside = match (u32::try_from(mx), u32::try_from(my)) {
                        (Ok(mx), Ok(my)) => mask.image.get(mx, my),
                        _ => None,
                    };
                    let v = inside.unwrap_or(mask.fill.value()).clamp(0.0, 1.0);
                    assert_mask_pixel(got, mask_gray(v), VIEW_TOL, &format!("{id} ({x}, {y})"));
                }
            }
        }
    }

    #[test]
    fn render_mask_view_downscale_keeps_a_flat_value() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut doc = flat_doc(64, 48, |_, _| [0.2, 0.3, 0.4, 1.0]);
        let id = doc.layers()[0].id();
        let image = MaskImage::from_data(64, 48, vec![0.5; 64 * 48]).unwrap();
        doc.set_mask(id, Some(Mask::new(image, (0, 0), MaskFill::White)))
            .unwrap();
        let want = mask_gray(0.5);
        for scale in [0.9, 0.5, 0.3, 0.25, 0.1] {
            // Keep the output pixels inside the canvas.
            let w = (64.0 * scale) as u32 - 1;
            let h = (48.0 * scale) as u32 - 1;
            let out = read_mask_view(&ctx, &mut comp, &doc, view(0.0, 0.0, scale, w, h), id);
            for (i, p) in out.iter().enumerate() {
                assert_mask_pixel(*p, want, VIEW_TOL, &format!("scale {scale}, pixel {i}"));
            }
        }
        assert_eq!(comp.mask_view_builds(), 1);
    }

    #[test]
    fn render_mask_view_builds_only_after_a_mask_change() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(22);
        let mut doc = empty_doc(16, 16, BlendSpace::Perceptual);
        let id = add_pixel(&mut doc, &mut rng, None, (16, 16), (0, 0));
        let image = MaskImage::from_data(16, 16, vec![0.25; 256]).unwrap();
        doc.set_mask(id, Some(Mask::new(image, (0, 0), MaskFill::White)))
            .unwrap();
        let full = view(0.0, 0.0, 1.0, 16, 16);

        read_view(&ctx, &mut comp, &doc, full);
        assert_eq!(comp.view_composites(), 1);
        let out = read_mask_view(&ctx, &mut comp, &doc, full, id);
        assert_mask_pixel(out[0], mask_gray(0.25), VIEW_TOL, "first");
        assert_eq!(comp.mask_view_builds(), 1);

        // Pan and zoom, and a disable of the mask: no new build.
        read_mask_view(&ctx, &mut comp, &doc, view(-2.5, 3.0, 2.0, 20, 10), id);
        read_mask_view(&ctx, &mut comp, &doc, view(0.0, 0.0, 0.3, 4, 4), id);
        doc.set_mask_enabled(id, false).unwrap();
        let out = read_mask_view(&ctx, &mut comp, &doc, full, id);
        assert_mask_pixel(out[0], mask_gray(0.25), VIEW_TOL, "disabled");
        assert_eq!(comp.mask_view_builds(), 1);
        // The composite view cache stays: no new composite.
        read_view(&ctx, &mut comp, &doc, full);
        assert_eq!(
            comp.view_composites(),
            2,
            "the disable changed the revision"
        );
        read_mask_view(&ctx, &mut comp, &doc, full, id);
        read_view(&ctx, &mut comp, &doc, full);
        assert_eq!(comp.view_composites(), 2);
        assert_eq!(comp.mask_view_builds(), 1);

        // A mask write builds again and changes the result.
        doc.write_mask(id, Rect::new(0, 0, 1, 1), &[0.75]).unwrap();
        let out = read_mask_view(&ctx, &mut comp, &doc, full, id);
        assert_eq!(comp.mask_view_builds(), 2);
        assert_mask_pixel(out[0], mask_gray(0.75), VIEW_TOL, "after write");
        assert_mask_pixel(out[1], mask_gray(0.25), VIEW_TOL, "after write, next pixel");

        // The cache is counted, and the mask view clear frees it (at least
        // the gray image of 16 x 16 rgba16float pixels).
        let with_mask = comp.cache_memory_bytes();
        comp.clear_mask_view_cache();
        assert!(comp.cache_memory_bytes() + 8 * 256 <= with_mask);
        read_mask_view(&ctx, &mut comp, &doc, full, id);
        assert_eq!(comp.mask_view_builds(), 3);
    }

    #[test]
    fn render_mask_view_rejects_a_layer_without_mask_and_an_unknown_layer() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(23);
        let mut doc = empty_doc(8, 8, BlendSpace::Perceptual);
        let id = add_pixel(&mut doc, &mut rng, None, (8, 8), (0, 0));
        let full = view(0.0, 0.0, 1.0, 8, 8);
        let result = pollster::block_on(comp.render_mask_view(&ctx, &doc, &full, id));
        assert!(matches!(result, Err(EditorError::NoMask(e)) if e == id));
        let unknown = LayerId(u64::MAX);
        let result = pollster::block_on(comp.render_mask_view(&ctx, &doc, &full, unknown));
        assert!(matches!(result, Err(EditorError::LayerNotFound(e)) if e == unknown));
        let bad = view(0.0, 0.0, 0.0, 8, 8);
        let result = pollster::block_on(comp.render_mask_view(&ctx, &doc, &bad, id));
        assert!(matches!(result, Err(EditorError::InvalidView(_))));
        assert_eq!(comp.mask_view_builds(), 0);
    }

    /// Renders a view of `doc` and checks the cached view composite: it must
    /// be bit-equal to a full GPU composite, and the full composite must
    /// agree with the CPU reference.
    fn check_view_composite(
        ctx: &GpuContext,
        comp: &mut GpuCompositor,
        doc: &Document,
        what: &str,
    ) {
        read_view(ctx, comp, doc, view(0.0, 0.0, 1.0, 8, 8));
        let cached = {
            let cache = comp.view.as_ref().expect("a view composite");
            pollster::block_on(cache.composite.read_rgba_f32(ctx)).unwrap()
        };
        let full = pollster::block_on(comp.composite_gpu_image(ctx, doc, doc.bounds())).unwrap();
        assert!(
            cached
                .iter()
                .zip(full.as_slice())
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "{what}: the view composite is not the full composite"
        );
        compare(ctx, comp, doc, doc.bounds(), false, TOL, what);
    }

    /// The index of the "below" snapshot, or `None`.
    fn snapshot_index(comp: &GpuCompositor) -> Option<usize> {
        comp.below
            .as_ref()
            .and_then(|b| b.snapshot.as_ref())
            .map(|s| s.keys.len())
    }

    #[test]
    fn render_view_below_snapshot_matches_full_composite() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(21);
        let mut doc = empty_doc(40, 32, BlendSpace::Perceptual);
        // 0: base, 1: masked multiply layer, 2: levels, 3: isolated group,
        // 4: pass-through group, 5: hue/saturation, 6: hidden layer.
        let base = add_pixel(&mut doc, &mut rng, None, (40, 32), (0, 0));
        let p1 = add_pixel(&mut doc, &mut rng, None, (30, 20), (-4, 6));
        doc.set_blend_mode(p1, BlendMode::Multiply).unwrap();
        set_mask(&mut doc, &mut rng, p1, (20, 16), (3, 8), MaskFill::Black);
        let a2 = add(
            &mut doc,
            None,
            LayerContent::Adjustment(Adjustment::Levels(levels())),
        );
        let g3 = add(
            &mut doc,
            None,
            LayerContent::Group(GroupIsolation::Isolated),
        );
        add_pixel(&mut doc, &mut rng, Some(g3), (20, 20), (10, 4));
        let child = add_pixel(&mut doc, &mut rng, Some(g3), (16, 16), (18, 12));
        doc.set_blend_mode(child, BlendMode::Screen).unwrap();
        let g4 = add(
            &mut doc,
            None,
            LayerContent::Group(GroupIsolation::PassThrough),
        );
        doc.set_opacity(g4, 0.7).unwrap();
        add(
            &mut doc,
            Some(g4),
            LayerContent::Adjustment(Adjustment::Curves(curves())),
        );
        let hs = |hue: f32| {
            Adjustment::HueSaturation(HueSaturation {
                hue,
                saturation: 0.2,
                lightness: -0.1,
            })
        };
        let a5 = add(&mut doc, None, LayerContent::Adjustment(hs(30.0)));
        let p6 = add_pixel(&mut doc, &mut rng, None, (12, 12), (2, 2));
        doc.set_visible(p6, false).unwrap();

        let mut partial = 0;
        let mut step = |comp: &mut GpuCompositor, doc: &Document, hit: bool, at, what: &str| {
            check_view_composite(&ctx, comp, doc, what);
            partial += u64::from(hit);
            assert_eq!(comp.view_partial_composites(), partial, "{what}: hits");
            assert_eq!(snapshot_index(comp), at, "{what}: snapshot index");
        };

        step(&mut comp, &doc, false, None, "initial");
        // The first change of a layer keeps the state below it; the next
        // changes start from there.
        doc.set_opacity(a5, 0.8).unwrap();
        step(&mut comp, &doc, false, Some(5), "top 1");
        doc.set_adjustment(a5, hs(45.0)).unwrap();
        step(&mut comp, &doc, true, Some(5), "top 2");
        // A change below the snapshot discards it.
        let mut l = levels();
        l.master.gamma = 0.8;
        doc.set_adjustment(a2, Adjustment::Levels(l)).unwrap();
        step(&mut comp, &doc, false, Some(2), "levels 1");
        doc.set_blend_mode(a2, BlendMode::Overlay).unwrap();
        step(&mut comp, &doc, true, Some(2), "levels 2");
        // A change above the snapshot starts from it and moves it up.
        doc.set_opacity(a5, 0.6).unwrap();
        step(&mut comp, &doc, true, Some(5), "top 3");
        doc.set_opacity(a5, 0.5).unwrap();
        step(&mut comp, &doc, true, Some(5), "top 4");
        // A change in a group: the snapshot is below the top-level group.
        doc.set_opacity(child, 0.4).unwrap();
        step(&mut comp, &doc, false, Some(3), "child 1");
        doc.set_blend_mode(child, BlendMode::Difference).unwrap();
        step(&mut comp, &doc, true, Some(3), "child 2");
        // A small mask write and pixel write get a patch composite (see
        // "Partial updates"). The patch does not change the snapshot, but
        // its keys are no longer a prefix of the keys.
        doc.write_mask(p1, Rect::new(2, 2, 3, 3), &[0.5; 9])
            .unwrap();
        step(&mut comp, &doc, false, Some(3), "mask write");
        let mut history = History::default();
        let e = doc
            .write_pixels(base, Rect::new(1, 1, 2, 2), &[[0.1, 0.2, 0.3, 1.0]; 4])
            .unwrap();
        history.push("Write", vec![e]);
        step(&mut comp, &doc, false, Some(3), "pixel write");
        history.undo(&mut doc).unwrap();
        step(&mut comp, &doc, false, Some(3), "undo write");
        assert_eq!(comp.view_patch_composites(), 3);
        // The hidden top layer: no step runs below it, then it shows. The
        // full composite discards the old snapshot (the writes were below
        // it).
        doc.set_visible(p6, true).unwrap();
        step(&mut comp, &doc, false, Some(6), "show top");
        doc.set_opacity(p6, 0.5).unwrap();
        step(&mut comp, &doc, true, Some(6), "top opacity");
        // A new layer inside the snapshot range, and its delete.
        let e = doc.delete_layer(g3).unwrap();
        history.push("Delete", vec![e]);
        step(&mut comp, &doc, false, Some(3), "delete group");
        history.undo(&mut doc).unwrap();
        step(&mut comp, &doc, true, Some(3), "undo delete");
        // A move of a layer below and a blend space change discard it.
        doc.translate_layer(p1, 2, 1).unwrap();
        step(&mut comp, &doc, false, Some(1), "move");
        doc.set_blend_space(BlendSpace::Linear).unwrap();
        step(&mut comp, &doc, false, None, "blend space");
        doc.set_opacity(a5, 0.9).unwrap();
        step(&mut comp, &doc, false, Some(5), "linear 1");
        doc.set_opacity(a5, 0.3).unwrap();
        step(&mut comp, &doc, true, Some(5), "linear 2");
        // `clear_cache` drops the snapshot.
        comp.clear_cache();
        assert_eq!(snapshot_index(&comp), None);
        assert_eq!(comp.cache_memory_bytes(), 16);
    }

    #[test]
    fn render_view_rejects_invalid_views() {
        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let doc = empty_doc(8, 8, BlendSpace::Perceptual);
        let max = ctx.limits().max_texture_dimension_2d;
        let bad = [
            view(0.0, 0.0, 0.0, 8, 8),
            view(0.0, 0.0, -1.0, 8, 8),
            view(0.0, 0.0, f64::NAN, 8, 8),
            view(0.0, 0.0, f64::INFINITY, 8, 8),
            view(0.0, 0.0, MIN_VIEW_SCALE / 2.0, 8, 8),
            view(0.0, 0.0, MAX_VIEW_SCALE * 2.0, 8, 8),
            view(f64::NAN, 0.0, 1.0, 8, 8),
            view(0.0, -2.0e9, 1.0, 8, 8),
            view(0.0, 0.0, 1.0, 0, 8),
            view(0.0, 0.0, 1.0, 8, max + 1),
        ];
        for spec in bad {
            let result = pollster::block_on(comp.render_view(&ctx, &doc, &spec));
            assert!(
                matches!(result, Err(EditorError::InvalidView(_))),
                "{spec:?}"
            );
        }
        assert_eq!(comp.view_composites(), 0);
        // The smallest and the largest scales are valid.
        read_view(&ctx, &mut comp, &doc, view(0.0, 0.0, MIN_VIEW_SCALE, 1, 1));
        read_view(&ctx, &mut comp, &doc, view(0.0, 0.0, MAX_VIEW_SCALE, 2, 2));
    }

    #[test]
    fn view_level_selects_a_footprint_below_two_texels() {
        assert_eq!(view_level(4.0), 0);
        assert_eq!(view_level(1.0), 0);
        assert_eq!(view_level(0.75), 0);
        assert_eq!(view_level(0.5), 1);
        assert_eq!(view_level(0.3), 1);
        assert_eq!(view_level(0.25), 2);
        assert_eq!(view_level(MIN_VIEW_SCALE), 16);
    }

    /// Benchmark: a 4096 x 4096 document with 10 layers in the perceptual
    /// space, CPU against GPU. Run with
    /// `cargo test -p expose-editor --release -- --ignored --nocapture bench_`.
    #[test]
    #[ignore = "benchmark; run manually in release mode"]
    fn bench_composite_4096_perceptual() {
        use std::time::Instant;

        let Some(ctx) = context() else { return };
        let mut comp = pollster::block_on(GpuCompositor::new(&ctx)).unwrap();
        let mut rng = Rng(7);
        let size = 4096;
        let mut doc = empty_doc(size, size, BlendSpace::Perceptual);
        let full_modes = [
            BlendMode::Normal,
            BlendMode::Multiply,
            BlendMode::Screen,
            BlendMode::Overlay,
            BlendMode::SoftLight,
            BlendMode::Color,
        ];
        for mode in full_modes {
            let id = add_pixel(&mut doc, &mut rng, None, (size, size), (0, 0));
            doc.set_blend_mode(id, mode).unwrap();
            doc.set_opacity(id, 0.8).unwrap();
        }
        let half = add_pixel(&mut doc, &mut rng, None, (2048, 2048), (1000, 1500));
        set_mask(
            &mut doc,
            &mut rng,
            half,
            (2048, 2048),
            (1000, 1500),
            MaskFill::Black,
        );
        let add_layer = add_pixel(&mut doc, &mut rng, None, (2048, 2048), (-500, 2600));
        doc.set_blend_mode(add_layer, BlendMode::LinearDodge)
            .unwrap();
        add(
            &mut doc,
            None,
            LayerContent::Adjustment(Adjustment::Curves(curves())),
        );
        add(
            &mut doc,
            None,
            LayerContent::Adjustment(Adjustment::HueSaturation(HueSaturation {
                hue: 20.0,
                saturation: 0.2,
                lightness: 0.0,
            })),
        );
        let region = doc.bounds();

        let t = Instant::now();
        let cpu = composite(&doc, region).unwrap();
        let cpu_ms = t.elapsed().as_secs_f64() * 1e3;

        let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;

        // The first call compiles the pipelines and uploads all layers.
        let t = Instant::now();
        drop(pollster::block_on(comp.composite_gpu(&ctx, &doc, region)).unwrap());
        ctx.wait_idle().unwrap();
        let first_ms = ms(t);
        let uploads = comp.layer_uploads();

        // Warm: an opacity change before each call. The cache has all
        // layers, so this is the kernel time only.
        let top = doc.layers()[5].id();
        let mut warm_ms = f64::MAX;
        for i in 0..3 {
            doc.set_opacity(top, 0.5 + 0.1 * i as f32).unwrap();
            let t = Instant::now();
            drop(pollster::block_on(comp.composite_gpu(&ctx, &doc, region)).unwrap());
            ctx.wait_idle().unwrap();
            warm_ms = warm_ms.min(ms(t));
        }
        assert_eq!(comp.layer_uploads(), uploads, "a property change uploaded");

        // A cold upload of all layers, for comparison.
        comp.clear_cache();
        let t = Instant::now();
        drop(pollster::block_on(comp.composite_gpu(&ctx, &doc, region)).unwrap());
        ctx.wait_idle().unwrap();
        let cold_ms = ms(t);

        // View: the first call composites and builds the pyramid; a pan
        // runs the view kernel only.
        let spec = |x: f64, scale: f64| ViewSpec {
            origin_x: x,
            origin_y: 100.0,
            scale,
            width: 2560,
            height: 1600,
        };
        doc.set_opacity(top, 0.9).unwrap();
        let t = Instant::now();
        pollster::block_on(comp.render_view(&ctx, &doc, &spec(0.0, 0.5))).unwrap();
        ctx.wait_idle().unwrap();
        let view_first_ms = ms(t);
        let mut pan_ms = [f64::MAX; 3];
        for (slot, scale) in pan_ms.iter_mut().zip([1.0, 0.5, 0.3]) {
            for i in 0..5 {
                let t = Instant::now();
                pollster::block_on(comp.render_view(&ctx, &doc, &spec(f64::from(i) * 7.5, scale)))
                    .unwrap();
                ctx.wait_idle().unwrap();
                *slot = slot.min(ms(t));
            }
        }
        assert_eq!(comp.view_composites(), 1, "a pan composited");
        // A slider step: an opacity change, then a view (composite, pyramid,
        // and view kernel; no upload, no buffer allocation). The first step
        // on the layer composites all layers and keeps the "below"
        // snapshot; the next steps start from it.
        let mut full_change_ms = 0.0;
        let mut change_ms = f64::MAX;
        for i in 0..4 {
            doc.set_opacity(top, 0.4 + 0.1 * i as f32).unwrap();
            let t = Instant::now();
            pollster::block_on(comp.render_view(&ctx, &doc, &spec(0.0, 0.3))).unwrap();
            ctx.wait_idle().unwrap();
            if i == 0 {
                full_change_ms = ms(t);
            } else {
                change_ms = change_ms.min(ms(t));
            }
        }
        assert_eq!(
            comp.view_partial_composites(),
            3,
            "the snapshot was not used"
        );
        let memory_mib = comp.cache_memory_bytes() as f64 / (1024.0 * 1024.0);

        let t = Instant::now();
        let gpu = pollster::block_on(comp.composite_gpu_image(&ctx, &doc, region)).unwrap();
        let readback_ms = ms(t) - warm_ms;
        assert_eq!(gpu.width(), cpu.width());

        // The upload of one canvas-size layer: `f32` values as they are,
        // and with the CPU conversion to `f16` (the layer cache format).
        // A conversion in a kernel needs the `f32` upload first, so it
        // cannot be faster than the `f32` upload.
        let layer = doc.layers()[0].as_pixel().unwrap().image();
        let mut upload_ms = [f64::MAX; 2];
        for (slot, format) in upload_ms.iter_mut().zip([FORMAT, LAYER_FORMAT]) {
            for _ in 0..3 {
                let t = Instant::now();
                let image = pollster::block_on(GpuImage::from_rgba_f32(
                    &ctx,
                    size,
                    size,
                    format,
                    layer.as_slice(),
                ))
                .unwrap();
                ctx.wait_idle().unwrap();
                *slot = slot.min(ms(t));
                drop(image);
            }
        }

        eprintln!(
            "bench 4096x4096, 10 layers, perceptual: CPU {cpu_ms:.1} ms; GPU first call \
             {first_ms:.1} ms ({uploads} uploads); warm after an opacity change \
             {warm_ms:.1} ms; cold (all uploads) {cold_ms:.1} ms; render_view 2560x1600: \
             first {view_first_ms:.1} ms, pan at scale 1 / 0.5 / 0.3: {:.2} / {:.2} / \
             {:.2} ms, after an opacity change of layer 5: first {full_change_ms:.1} ms, \
             next (below snapshot) {change_ms:.1} ms; cache {memory_mib:.0} MiB; \
             read-back about {readback_ms:.1} ms; upload of one 4096x4096 layer: rgba32float \
             {:.1} ms, rgba16float {:.1} ms",
            pan_ms[0], pan_ms[1], pan_ms[2], upload_ms[0], upload_ms[1]
        );
    }
}
