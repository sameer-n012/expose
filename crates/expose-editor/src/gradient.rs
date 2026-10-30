//! Gradient fill on a pixel layer or a layer mask.
//!
//! # Use
//!
//! Make a [`GradientSpec`] (the line from `start` to `end`, the shape, and
//! the color stops) and give it to [`Document::fill_gradient`] with a
//! [`GradientTarget`]. The function changes the layer in place and returns
//! one undo [`Edit`].
//!
//! # Gradient position
//!
//! For each pixel, the fill computes a gradient position `t`. All values
//! are canvas coordinates. The y axis points down.
//!
//! ```text
//! p  = (x + 0.5, y + 0.5)              (pixel center)
//! q  = p - start
//! d  = end - start,  L = |d|
//! u  = dot(q, d) / L^2
//! v  = cross(d, q) / L^2 = (d.x * q.y - d.y * q.x) / L^2
//! ```
//!
//! | Shape                        | `t`                                   |
//! | ---------------------------- | ------------------------------------- |
//! | [`GradientShape::Linear`]    | `u`                                   |
//! | [`GradientShape::Radial`]    | `\|q\| / L`                           |
//! | [`GradientShape::Angle`]     | `a / (2 pi)`, see below               |
//! | [`GradientShape::Reflected`] | `\|u\|`                               |
//! | [`GradientShape::Diamond`]   | `\|u\| + \|v\|`                       |
//!
//! Angle: `a = atan2(cross(d, q), dot(d, q))`, plus `2 pi` if it is less
//! than 0. Thus `a` is in `0..2 pi` and increases **clockwise on the
//! screen** (because the y axis points down) from the direction `d`. For
//! example, with `d` to the right, a pixel directly below `start` has
//! `t = 0.25`. A pixel at `start` has `t = 0`.
//!
//! Then `t` is clamped to `0..=1`. With [`GradientSpec::reverse`],
//! `t = 1 - t`.
//!
//! # Stops
//!
//! A gradient has 2 to [`MAX_GRADIENT_STOPS`] stops with positions in
//! `0..=1` in ascending order. Two stops can have the same position: this
//! gives a hard edge (at that position the later stop applies). Before the
//! first stop and after the last stop, the end stop applies.
//!
//! Between two stops `a` and `b` (with `w = (t - pos_a) / (pos_b - pos_a)`):
//!
//! - Color: `decode(lerp(encode(color_a), encode(color_b), w))`, where
//!   `encode` and `decode` are [`BlendSpace::encode`] and
//!   [`BlendSpace::decode`] of the document blend space. Two stops with the
//!   same color give that color exactly.
//! - Opacity: `lerp(opacity_a, opacity_b, w)`.
//! - Mask value ([`GradientTarget::Mask`]): `lerp(value_a, value_b, w)`.
//!
//! # Table
//!
//! The fill does not compute the stops for each pixel. It makes a table
//! with [`GRADIENT_TABLE_STEPS`]` + 1` entries for `t = i / 4096` and uses
//! the nearest entry. Thus `t` has an error of at most `1 / 8192`. The error
//! of a color (in the blend space), an opacity, or a mask value is at most
//! `1 / 8192` times its change between two stops per unit of `t`. For a
//! black to white gradient over the full line, this is less than 0.04 of
//! one 8-bit step. A hard edge moves by at most `L / 8192`.
//!
//! # Fill
//!
//! - Pixels: the coverage is `c = selection * opacity * stop_opacity(t)`
//!   (no selection counts as 1). The fill uses the same paint rule as a
//!   brush stroke with the coverage `c` and the color at `t` (see
//!   "Accumulation" and "Blend mode" in [`crate::brush`]), also the rule for
//!   the transparency lock. Only pixel layers. Only the pixels inside the
//!   layer bounds and the canvas change: the layer does not grow.
//! - Mask: `m = lerp(orig_m, value(t), c)` with the same coverage `c`. The
//!   blend mode must be [`BlendMode::Normal`]. Only the mask values inside
//!   the canvas change.

use expose_core::buffer::{Rect, Rgba};

use crate::blend::{BlendMode, BlendSpace};
use crate::brush::Mode;
use crate::document::Document;
use crate::error::{EditorError, EditorResult, check_range};
use crate::flood::FillPixel;
use crate::layer::{LayerId, LockKind};
use crate::ops::Edit;

#[cfg(test)]
mod tests;

/// Largest number of stops of a gradient.
pub const MAX_GRADIENT_STOPS: usize = 32;
/// Largest absolute value of a gradient line coordinate (canvas pixels).
pub const MAX_GRADIENT_COORD: f64 = 1.0e5;
/// Smallest length of the gradient line in pixels.
pub const MIN_GRADIENT_LENGTH: f64 = 0.5;
/// Number of steps of the gradient table (see "Table" in the
/// [module documentation](self)). The table has one more entry.
pub const GRADIENT_TABLE_STEPS: usize = 4096;

/// The shape of a gradient (see the [module documentation](self)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GradientShape {
    /// Along the line: `t = u`.
    Linear,
    /// Circles around the start: `t = |q| / L`.
    Radial,
    /// Clockwise around the start, from the line direction.
    Angle,
    /// Along the line, mirrored at the start: `t = |u|`.
    Reflected,
    /// Squares around the start, with a corner on the line: `t = |u| + |v|`.
    Diamond,
}

/// One color stop of a gradient.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    /// Position on the gradient line, `0.0..=1.0`.
    pub position: f32,
    /// Straight (not premultiplied) linear Rec. 2020 color. Values must be
    /// finite.
    pub color: [f32; 3],
    /// `0.0..=1.0`.
    pub opacity: f32,
}

/// The settings of a gradient fill.
#[derive(Debug, Clone, PartialEq)]
pub struct GradientSpec {
    /// Start of the gradient line (canvas coordinates).
    pub start: (f64, f64),
    /// End of the gradient line (canvas coordinates).
    pub end: (f64, f64),
    /// The shape.
    pub shape: GradientShape,
    /// The stops (2 to [`MAX_GRADIENT_STOPS`], ascending positions).
    pub stops: Vec<GradientStop>,
    /// `0.0..=1.0`. Multiplies the coverage.
    pub opacity: f32,
    /// How the gradient color mixes with the pixels below it (the same
    /// rule as a brush, see [`crate::brush`]). [`BlendMode::Dissolve`] is
    /// not available. A mask target accepts only [`BlendMode::Normal`].
    pub blend_mode: BlendMode,
    /// If `true`, `t = 1 - t`.
    pub reverse: bool,
}

/// What a gradient fill changes.
#[derive(Debug, Clone, PartialEq)]
pub enum GradientTarget {
    /// The pixels of a pixel layer (the stop colors and opacities).
    Pixels,
    /// The mask of a layer.
    Mask {
        /// One mask value for each stop, `0.0..=1.0` (0 = black, hides;
        /// 1 = white, shows).
        values: Vec<f32>,
    },
}

impl GradientSpec {
    /// Returns an error if a setting is not valid:
    /// - [`EditorError::NonFinite`] / [`EditorError::OutOfRange`] for a line
    ///   coordinate that is not finite or has an absolute value more than
    ///   [`MAX_GRADIENT_COORD`], for a stop position or opacity that is not
    ///   in `0..=1`, for a stop color that is not finite, or for an opacity
    ///   that is not in `0..=1`.
    /// - [`EditorError::InvalidGradient`] for fewer than 2 or more than
    ///   [`MAX_GRADIENT_STOPS`] stops, stops that are not in ascending
    ///   order, a line shorter than [`MIN_GRADIENT_LENGTH`], or
    ///   [`BlendMode::Dissolve`].
    pub fn validate(&self) -> EditorResult<()> {
        if self.blend_mode == BlendMode::Dissolve {
            // The dissolve pattern changes alpha per pixel position. The
            // coverage rule of the paint rule has no place for it.
            return Err(EditorError::InvalidGradient(
                "the dissolve blend mode is not available for a gradient",
            ));
        }
        let coords = [
            ("gradient start x", self.start.0),
            ("gradient start y", self.start.1),
            ("gradient end x", self.end.0),
            ("gradient end y", self.end.1),
        ];
        for (name, v) in coords {
            if !v.is_finite() {
                return Err(EditorError::NonFinite { name });
            }
            if v.abs() > MAX_GRADIENT_COORD {
                return Err(EditorError::OutOfRange {
                    name,
                    value: v,
                    min: -MAX_GRADIENT_COORD,
                    max: MAX_GRADIENT_COORD,
                });
            }
        }
        let (dx, dy) = (self.end.0 - self.start.0, self.end.1 - self.start.1);
        if dx.hypot(dy) < MIN_GRADIENT_LENGTH {
            return Err(EditorError::InvalidGradient(
                "the gradient line is shorter than 0.5 px",
            ));
        }
        if self.stops.len() < 2 {
            return Err(EditorError::InvalidGradient(
                "a gradient needs at least 2 stops",
            ));
        }
        if self.stops.len() > MAX_GRADIENT_STOPS {
            return Err(EditorError::InvalidGradient(
                "a gradient has at most 32 stops",
            ));
        }
        for s in &self.stops {
            check_range("gradient stop position", s.position, 0.0, 1.0)?;
            if s.color.iter().any(|v| !v.is_finite()) {
                return Err(EditorError::NonFinite {
                    name: "gradient stop color",
                });
            }
            check_range("gradient stop opacity", s.opacity, 0.0, 1.0)?;
        }
        if self.stops.windows(2).any(|w| w[1].position < w[0].position) {
            return Err(EditorError::InvalidGradient(
                "the gradient stops are not in ascending order",
            ));
        }
        check_range("gradient opacity", self.opacity, 0.0, 1.0)
    }
}

impl GradientTarget {
    /// Returns an error if the target does not fit the spec:
    /// [`EditorError::InvalidGradient`] for a mask target with a blend mode
    /// other than normal or with a value count that is not the stop count;
    /// [`EditorError::NonFinite`] / [`EditorError::OutOfRange`] for a mask
    /// value that is not in `0..=1`.
    fn validate(&self, spec: &GradientSpec) -> EditorResult<()> {
        match self {
            Self::Pixels => Ok(()),
            Self::Mask { values } => {
                if spec.blend_mode != BlendMode::Normal {
                    return Err(EditorError::InvalidGradient(
                        "a mask gradient accepts only the normal blend mode",
                    ));
                }
                if values.len() != spec.stops.len() {
                    return Err(EditorError::InvalidGradient(
                        "a mask gradient needs one value for each stop",
                    ));
                }
                for &v in values {
                    check_range("gradient mask value", v, 0.0, 1.0)?;
                }
                Ok(())
            }
        }
    }
}

impl Document {
    /// Fills a pixel layer or a layer mask with a gradient (see the
    /// [module documentation](crate::gradient)).
    ///
    /// Returns the undo edit and the changed rectangle in canvas
    /// coordinates, or `None` if no value changed. If a value changed, the
    /// layer gets a new pixel (or mask) version and the document a new
    /// revision. The edit holds the original values of the changed tiles
    /// ([`STROKE_TILE_SIZE`](crate::STROKE_TILE_SIZE) pixels) only.
    ///
    /// Errors:
    /// - Bad settings ([`GradientSpec::validate`]), or a mask target that
    ///   does not fit ([`EditorError::InvalidGradient`] for a value count
    ///   that is not the stop count or a blend mode other than normal).
    /// - [`EditorError::LayerNotFound`].
    /// - [`EditorError::NotAPixelLayer`] for [`GradientTarget::Pixels`] on
    ///   a layer that is not a pixel layer; [`EditorError::NoMask`] for
    ///   [`GradientTarget::Mask`] on a layer without a mask.
    /// - [`EditorError::Locked`] if the pixels lock (or `all`) is set on
    ///   the layer or on a parent group (also for a mask target, as for a
    ///   brush mask stroke).
    pub fn fill_gradient(
        &mut self,
        layer: LayerId,
        spec: &GradientSpec,
        target: &GradientTarget,
    ) -> EditorResult<Option<(Edit, Rect)>> {
        spec.validate()?;
        target.validate(spec)?;
        let l = self.layer(layer).ok_or(EditorError::LayerNotFound(layer))?;
        match target {
            GradientTarget::Pixels if l.as_pixel().is_none() => {
                return Err(EditorError::NotAPixelLayer(layer));
            }
            GradientTarget::Mask { .. } if l.mask().is_none() => {
                return Err(EditorError::NoMask(layer));
            }
            _ => {}
        }
        let locks = self.effective_locks(layer)?;
        locks.check(layer, LockKind::Pixels)?;
        let geometry = Geometry::new(spec);
        match target {
            GradientTarget::Pixels if spec.blend_mode == BlendMode::Normal => {
                // The fast path: a table of colors and coverages. The paint
                // mode is made for each pixel from the known variant, so
                // the compiler can remove the mode match.
                let table = normal_table(spec, self.blend_space);
                let keep_alpha = locks.transparency;
                let space = self.blend_space;
                self.fill_pixels_with(layer, |px, old: Rgba| {
                    let e = table[geometry.index(px)];
                    let c = e[3] * px.selection;
                    if c <= 0.0 {
                        old
                    } else {
                        let color = [e[0], e[1], e[2]];
                        Mode::paint(color, BlendMode::Normal, space, keep_alpha).pixel(old, c)
                    }
                })
            }
            GradientTarget::Pixels => {
                let table = pixel_table(spec, self.blend_space, locks.transparency);
                self.fill_pixels_with(layer, |px, old: Rgba| {
                    let (mode, cov) = table[geometry.index(px)];
                    let c = cov * px.selection;
                    if c <= 0.0 { old } else { mode.pixel(old, c) }
                })
            }
            GradientTarget::Mask { values } => {
                let table = mask_table(spec, values);
                self.fill_mask_with(layer, |px, old: f32| {
                    let (mode, cov) = table[geometry.index(px)];
                    let c = cov * px.selection;
                    if c <= 0.0 { old } else { mode.mask(old, c) }
                })
            }
        }
    }
}

/// The fixed values of the position computation (see "Gradient position"
/// in the [module documentation](self)).
#[derive(Debug, Clone, Copy)]
struct Geometry {
    shape: GradientShape,
    reverse: bool,
    start: (f64, f64),
    d: (f64, f64),
    /// `1 / L^2`.
    inv_l2: f64,
    /// `1 / L`.
    inv_l: f64,
}

impl Geometry {
    /// The caller must validate the spec before (`L >= 0.5`).
    fn new(spec: &GradientSpec) -> Self {
        let d = (spec.end.0 - spec.start.0, spec.end.1 - spec.start.1);
        let l2 = d.0 * d.0 + d.1 * d.1;
        Self {
            shape: spec.shape,
            reverse: spec.reverse,
            start: spec.start,
            d,
            inv_l2: 1.0 / l2,
            inv_l: 1.0 / l2.sqrt(),
        }
    }

    /// The gradient position `t` at the center of the canvas pixel
    /// `(x, y)`, clamped to `0..=1`, after `reverse`.
    // Canvas coordinates are below `MAX_DIMENSION`, so they are exact in
    // f64. The f32 casts for `atan2` keep the relative precision (about
    // 6e-8) of the values, which is sufficient (see the comment there).
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    #[inline]
    fn t(&self, x: usize, y: usize) -> f64 {
        let q = (x as f64 + 0.5 - self.start.0, y as f64 + 0.5 - self.start.1);
        let dot = q.0 * self.d.0 + q.1 * self.d.1;
        let t = match self.shape {
            GradientShape::Linear => dot * self.inv_l2,
            // The coordinates are at most about 1e5, so the squares cannot
            // overflow: `sqrt` is safe here and faster than `hypot`.
            GradientShape::Radial => (q.0 * q.0 + q.1 * q.1).sqrt() * self.inv_l,
            GradientShape::Angle => {
                let cross = self.d.0 * q.1 - self.d.1 * q.0;
                // `atan2` in f32 is faster. Its error (about 1e-7) is much
                // less than the table step (1 / 4096).
                let a = f64::from((cross as f32).atan2(dot as f32)) / std::f64::consts::TAU;
                if a < 0.0 { a + 1.0 } else { a }
            }
            GradientShape::Reflected => (dot * self.inv_l2).abs(),
            GradientShape::Diamond => {
                let cross = self.d.0 * q.1 - self.d.1 * q.0;
                (dot.abs() + cross.abs()) * self.inv_l2
            }
        };
        let t = t.clamp(0.0, 1.0);
        if self.reverse { 1.0 - t } else { t }
    }

    /// The table index of the nearest entry for a pixel.
    // `t` is in `0..=1` (the inputs are finite, so it is not NaN), thus the
    // value is in `0..=4096.5` before the cast.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    #[inline]
    fn index(&self, px: FillPixel) -> usize {
        /// [`GRADIENT_TABLE_STEPS`] as f64.
        const SCALE: f64 = 4096.0;
        const _: () = assert!(GRADIENT_TABLE_STEPS == 4096);
        let i = (self.t(px.x, px.y) * SCALE + 0.5) as usize;
        i.min(GRADIENT_TABLE_STEPS)
    }
}

/// The two stops around a position and the weight of the second one.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Segment {
    a: usize,
    b: usize,
    /// `0..=1`.
    w: f32,
}

impl Segment {
    /// Finds the segment of `t` (see "Stops" in the
    /// [module documentation](self)). `stops` must be valid.
    fn at(stops: &[GradientStop], t: f32) -> Self {
        // The first stop with a position more than `t`. With equal
        // positions, the later stop applies at that position.
        let k = stops.partition_point(|s| s.position <= t);
        if k == 0 {
            return Self { a: 0, b: 0, w: 0.0 };
        }
        if k == stops.len() {
            let last = stops.len() - 1;
            return Self {
                a: last,
                b: last,
                w: 0.0,
            };
        }
        let (pa, pb) = (stops[k - 1].position, stops[k].position);
        // `pa <= t < pb`, so the divisor is more than 0.
        let w = ((t - pa) / (pb - pa)).clamp(0.0, 1.0);
        Self { a: k - 1, b: k, w }
    }

    fn lerp(self, va: f32, vb: f32) -> f32 {
        va + (vb - va) * self.w
    }
}

/// The positions `t = i / GRADIENT_TABLE_STEPS` of the table entries.
fn table_positions() -> impl Iterator<Item = f32> {
    // The values are at most 4096, so they are exact in f32, and the
    // divisions by a power of two are exact.
    #[allow(clippy::cast_precision_loss)]
    (0..=GRADIENT_TABLE_STEPS).map(|i| i as f32 / GRADIENT_TABLE_STEPS as f32)
}

/// The color at a segment: the stop color at the ends or if both stops
/// have the same color, else the interpolation in the blend space.
fn color_at(
    stops: &[GradientStop],
    encoded: &[[f32; 3]],
    space: BlendSpace,
    s: Segment,
) -> [f32; 3] {
    let (ca, cb) = (stops[s.a].color, stops[s.b].color);
    // An exact compare on purpose: only equal colors skip the encode and
    // decode, so that a flat gradient gives the stop color exactly.
    #[allow(clippy::float_cmp)]
    let same = ca == cb;
    if s.w <= 0.0 || same {
        return ca;
    }
    if s.w >= 1.0 {
        return cb;
    }
    let (ea, eb) = (encoded[s.a], encoded[s.b]);
    space.decode([
        s.lerp(ea[0], eb[0]),
        s.lerp(ea[1], eb[1]),
        s.lerp(ea[2], eb[2]),
    ])
}

/// The colors and coverages (before the selection) of the table entries
/// for a pixels target.
fn color_entries(spec: &GradientSpec, space: BlendSpace) -> impl Iterator<Item = ([f32; 3], f32)> {
    let stops = &spec.stops;
    let encoded: Vec<[f32; 3]> = stops.iter().map(|s| space.encode(s.color)).collect();
    table_positions().map(move |t| {
        let s = Segment::at(stops, t);
        let color = color_at(stops, &encoded, space, s);
        let opacity = s.lerp(stops[s.a].opacity, stops[s.b].opacity);
        (color, spec.opacity * opacity)
    })
}

/// The table for a pixels target with [`BlendMode::Normal`]: the color
/// and the coverage (before the selection) of each entry.
fn normal_table(spec: &GradientSpec, space: BlendSpace) -> Vec<[f32; 4]> {
    color_entries(spec, space)
        .map(|(c, cov)| [c[0], c[1], c[2], cov])
        .collect()
}

/// The table of paint modes and coverages (before the selection) for a
/// pixels target.
fn pixel_table(spec: &GradientSpec, space: BlendSpace, keep_alpha: bool) -> Vec<(Mode, f32)> {
    color_entries(spec, space)
        .map(|(color, cov)| {
            let mode = Mode::paint(color, spec.blend_mode, space, keep_alpha);
            (mode, cov)
        })
        .collect()
}

/// The table of mask modes and coverages (before the selection) for a
/// mask target. `values` has one value for each stop.
fn mask_table(spec: &GradientSpec, values: &[f32]) -> Vec<(Mode, f32)> {
    let stops = &spec.stops;
    table_positions()
        .map(|t| {
            let s = Segment::at(stops, t);
            let value = s.lerp(values[s.a], values[s.b]);
            let opacity = s.lerp(stops[s.a].opacity, stops[s.b].opacity);
            (Mode::Mask { value }, spec.opacity * opacity)
        })
        .collect()
}
