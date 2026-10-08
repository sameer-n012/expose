//! CPU reference compositor.
//!
//! The GPU compositor ([`crate::gpu`]) must give the same result as this
//! module (within float tolerance). Change both together.
//!
//! # Model
//!
//! - The compositor starts with a transparent buffer and processes each
//!   layer list from the bottom (index 0) up. All buffers have
//!   **premultiplied** alpha.
//! - Each layer has a coverage `k = opacity * fill_opacity * mask(x, y)`.
//!   Hidden layers are skipped.
//! - **Pixel layer:** the layer pixel (alpha multiplied by `k`) is blended
//!   onto the buffer with the W3C formula (see [`crate::blend`]). Areas
//!   outside the layer are transparent.
//! - **Adjustment layer:** the adjusted backdrop color `A` is blended onto
//!   the backdrop with the layer blend mode, then mixed with the backdrop
//!   by `k`: `c = cb + k * (B(cb, A) - cb)`. Alpha does not change, so an
//!   adjustment has no effect on transparent areas. It changes everything
//!   below it in the same buffer: in an isolated group, only the layers
//!   below it in the group; in a pass-through group, also the layers below
//!   the group.
//! - **Isolated group:** the children composite onto a new transparent
//!   buffer. That buffer is blended onto the backdrop like a pixel layer,
//!   with the group blend mode and coverage.
//! - **Pass-through group:** the children composite onto a copy of the
//!   backdrop. The result is mixed with the backdrop by `k`. The group blend
//!   mode is ignored.
//! - **Dissolve:** the compositor replaces the source alpha (or, for
//!   adjustment layers and isolated groups, the effective coverage) `a` by
//!   1 if `dissolve_threshold(x, y) < a`, else by 0. `(x, y)` are canvas
//!   coordinates, so the pattern is the same for all tiles.
//!
//! All operations are per pixel. Thus the composite of a region is equal to
//! the crop of the composite of the full canvas.

use expose_core::buffer::{Rect, Rgba, WorkingImage};

use crate::adjust::PreparedAdjustment;
use crate::blend::{BlendMode, BlendSpace, dissolve_threshold, premultiply, unpremultiply};
use crate::document::Document;
use crate::error::{EditorError, EditorResult};
use crate::layer::{GroupIsolation, Layer, LayerKind, PixelLayer};

/// Composites the visible layers of `document` in `region` (canvas
/// coordinates) and returns a premultiplied image of the region size.
///
/// Returns [`EditorError::InvalidRect`] if `region` is empty or not fully
/// inside the canvas.
pub fn composite(document: &Document, region: Rect) -> EditorResult<WorkingImage> {
    if region.is_empty() || !document.bounds().contains_rect(&region) {
        return Err(EditorError::InvalidRect {
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
        });
    }
    let ctx = Ctx {
        region,
        space: document.blend_space(),
    };
    let mut buf = WorkingImage::new(region.width, region.height)?;
    composite_list(&ctx, document.layers(), &mut buf)?;
    Ok(buf)
}

/// Composites like [`composite`] and returns the result with **straight**
/// alpha, for output (export and file writes).
pub fn composite_straight(document: &Document, region: Rect) -> EditorResult<WorkingImage> {
    Ok(to_straight_alpha(composite(document, region)?))
}

/// Converts a premultiplied image to straight alpha. Pixels with zero alpha
/// get color 0.
pub fn to_straight_alpha(mut image: WorkingImage) -> WorkingImage {
    image.unpremultiply_alpha();
    image
}

struct Ctx {
    region: Rect,
    space: BlendSpace,
}

impl Ctx {
    /// Calls `f(canvas_x, canvas_y, pixel)` for each pixel of the buffer.
    fn for_each(&self, buf: &mut WorkingImage, mut f: impl FnMut(i64, i64, usize, &mut Rgba)) {
        let w = self.region.width as usize;
        let (rx, ry) = (i64::from(self.region.x), i64::from(self.region.y));
        for (i, px) in buf.pixels_mut().iter_mut().enumerate() {
            // The buffer has region size, so the indices fit in i64.
            #[allow(clippy::cast_possible_wrap)]
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            f(rx + x, ry + y, i, px);
        }
    }
}

/// Returns `opacity * fill * mask(x, y)`.
fn coverage(layer: &Layer, base: f32, x: i64, y: i64) -> f32 {
    layer.mask().map_or(base, |m| base * m.value_at(x, y))
}

fn dissolve(mode: BlendMode, a: f32, x: i64, y: i64) -> f32 {
    if mode == BlendMode::Dissolve {
        if a > 0.0 && dissolve_threshold(x, y) < a {
            1.0
        } else {
            0.0
        }
    } else {
        a
    }
}

fn composite_list(ctx: &Ctx, layers: &[Layer], buf: &mut WorkingImage) -> EditorResult<()> {
    for layer in layers {
        if !layer.visible() {
            continue;
        }
        let base = layer.opacity() * layer.fill_opacity();
        if base <= 0.0 {
            continue;
        }
        match layer.kind() {
            LayerKind::Pixel(p) => composite_pixel_layer(ctx, layer, p, base, buf),
            LayerKind::Adjustment(a) => {
                let prepared = a.prepare()?;
                apply_adjustment(ctx, layer, &prepared, base, buf);
            }
            LayerKind::Group(g) => match g.isolation() {
                GroupIsolation::PassThrough => {
                    let mut after = buf.clone();
                    composite_list(ctx, g.children(), &mut after)?;
                    mix_pass_through(ctx, layer, base, buf, &after);
                }
                GroupIsolation::Isolated => {
                    let mut group = WorkingImage::new(ctx.region.width, ctx.region.height)?;
                    composite_list(ctx, g.children(), &mut group)?;
                    let src = group.pixels();
                    let mode = layer.blend_mode();
                    ctx.for_each(buf, |x, y, i, dst| {
                        let k = coverage(layer, base, x, y);
                        blend_pixel(dst, src[i], k, mode, ctx.space, x, y);
                    });
                }
            },
        }
    }
    Ok(())
}

fn composite_pixel_layer(
    ctx: &Ctx,
    layer: &Layer,
    p: &PixelLayer,
    base: f32,
    buf: &mut WorkingImage,
) {
    let img = p.image();
    let (ox, oy) = (i64::from(p.offset().0), i64::from(p.offset().1));
    let r = ctx.region;
    // Overlap of the layer and the region, in canvas coordinates.
    let Some(ov) = r.intersection_signed(p.offset().0, p.offset().1, img.width(), img.height())
    else {
        return;
    };
    let (x0, y0) = (i64::from(ov.x), i64::from(ov.y));
    let (x1, y1) = (x0 + i64::from(ov.width), y0 + i64::from(ov.height));
    let mode = layer.blend_mode();
    let src_w = img.width() as usize;
    let dst_w = r.width as usize;
    let src = img.pixels();
    let dst = buf.pixels_mut();
    for y in y0..y1 {
        // All values are inside the layer and the region, so they are
        // non-negative and fit in usize.
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let (src_row, dst_row) = (
            (y - oy) as usize * src_w,
            (y - i64::from(r.y)) as usize * dst_w,
        );
        for x in x0..x1 {
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let (si, di) = (
                src_row + (x - ox) as usize,
                dst_row + (x - i64::from(r.x)) as usize,
            );
            let k = coverage(layer, base, x, y);
            blend_pixel(&mut dst[di], src[si], k, mode, ctx.space, x, y);
        }
    }
}

/// Blends the premultiplied source pixel `src` with coverage `k` onto `dst`.
fn blend_pixel(
    dst: &mut Rgba,
    src: Rgba,
    k: f32,
    mode: BlendMode,
    space: BlendSpace,
    x: i64,
    y: i64,
) {
    let a_src = src[3];
    let a_s = dissolve(mode, (a_src * k).min(1.0), x, y);
    if a_s <= 0.0 || a_src <= 0.0 {
        return;
    }
    let ab = dst[3];
    if space == BlendSpace::Linear && matches!(mode, BlendMode::Normal | BlendMode::Dissolve) {
        // Fast path. With B = cs, the general formula reduces to
        // co = as * cs + (1 - as) * cb_premultiplied.
        let s = a_s / a_src;
        for c in 0..3 {
            dst[c] = src[c] * s + dst[c] * (1.0 - a_s);
        }
        dst[3] = a_s + ab * (1.0 - a_s);
        return;
    }
    let (cs_lin, _) = unpremultiply(src);
    let (cb_lin, _) = unpremultiply(*dst);
    let cs = space.encode(cs_lin);
    let cb = space.encode(cb_lin);
    let b = mode.blend(cb, cs);
    let ao = a_s + ab * (1.0 - a_s);
    let mut co = [0.0_f32; 3];
    for c in 0..3 {
        co[c] = a_s * (1.0 - ab) * cs[c] + ab * (1.0 - a_s) * cb[c] + a_s * ab * b[c];
    }
    // ao >= a_s > 0.
    let straight = co.map(|v| v / ao);
    *dst = premultiply(space.decode(straight), ao);
}

fn apply_adjustment(
    ctx: &Ctx,
    layer: &Layer,
    adj: &PreparedAdjustment,
    base: f32,
    buf: &mut WorkingImage,
) {
    let mode = layer.blend_mode();
    ctx.for_each(buf, |x, y, _, dst| {
        let k = dissolve(mode, coverage(layer, base, x, y).min(1.0), x, y);
        let (cb_lin, ab) = unpremultiply(*dst);
        if k <= 0.0 || ab <= 0.0 {
            return;
        }
        let adjusted = adj.apply(cb_lin);
        let cb = ctx.space.encode(cb_lin);
        let a = ctx.space.encode(adjusted);
        let b = mode.blend(cb, a);
        let c = [0, 1, 2].map(|i| cb[i] + k * (b[i] - cb[i]));
        *dst = premultiply(ctx.space.decode(c), ab);
    });
}

fn mix_pass_through(
    ctx: &Ctx,
    layer: &Layer,
    base: f32,
    buf: &mut WorkingImage,
    after: &WorkingImage,
) {
    let after = after.pixels();
    ctx.for_each(buf, |x, y, i, dst| {
        let k = coverage(layer, base, x, y).min(1.0);
        if k >= 1.0 {
            *dst = after[i];
            return;
        }
        if k <= 0.0 {
            return;
        }
        let b = ctx.space.encode_premultiplied(*dst);
        let a = ctx.space.encode_premultiplied(after[i]);
        let mixed = [0, 1, 2, 3].map(|c| b[c] + k * (a[c] - b[c]));
        *dst = ctx.space.decode_premultiplied(mixed);
    });
}
