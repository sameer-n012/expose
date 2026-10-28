//! Brush strokes on pixel layers and layer masks.
//!
//! # Use
//!
//! 1. [`Stroke::begin`] checks the settings, the layer kind, and the locks.
//! 2. [`Stroke::add_points`] places dabs along the pointer path and
//!    changes the layer in place. It returns the changed rectangle in
//!    canvas coordinates, so that the caller can update only that part of
//!    the view.
//! 3. [`Stroke::end`] returns one undo [`Edit`] for the full stroke. Give it
//!    to [`History::push`](crate::History::push). Or [`Stroke::cancel`]
//!    restores the original values.
//!
//! # Dab placement
//!
//! The first point of the stroke gets a dab. Then a dab goes at each
//! `step` along the polyline of the points, with
//! `step = max(spacing * diameter, MIN_DAB_STEP)`, where `diameter` is the
//! dab diameter at the last dab (it changes with pressure if
//! `pressure_size` is set). The pressure between two points is a linear
//! interpolation. The distance to the next dab carries over between calls
//! of [`Stroke::add_points`], so a split of the points into batches gives
//! the same pixels as one batch.
//!
//! # Dab shape
//!
//! For a dab with radius `r` (pixels) and hardness `h`, the value at a
//! pixel center with the distance `d` from the dab center is:
//!
//! ```text
//! edge(d)  = clamp(r + 0.5 - d, 0, 1)            (1 px anti-alias edge)
//! t        = clamp((d - h*r) / (r - h*r), 0, 1)  (t = 0 if h = 1)
//! soft(d)  = 1 - t*t*(3 - 2*t)                   (smoothstep falloff)
//! small(r) = min(1, 2*r)                         (fades dabs below 1 px)
//! dab(d)   = edge(d) * soft(d) * small(r)
//! ```
//!
//! # Accumulation
//!
//! The stroke keeps a coverage value `c` (start 0) for each touched pixel.
//! Each dab changes it as `c = min(c + (1 - c) * k, opacity)`, with
//! `k = dab(d) * flow * pressure_factor * selection`. The result of each
//! pixel comes from the **original** value (the value before the stroke)
//! and `c`, thus `opacity` is the maximum effect of the stroke, also where
//! dabs overlap:
//!
//! - Paint: the color with coverage `c` over the original
//!   (premultiplied). With the transparency lock, the alpha stays the same
//!   and only the color changes: `rgb = lerp(orig_rgb, color * orig_a, c)`.
//! - Erase: `result = original * (1 - c)`.
//! - Mask: `m = lerp(orig_m, value, c)`.
//!
//! # Memory
//!
//! The stroke keeps the original values and the coverage in tiles of
//! [`STROKE_TILE_SIZE`] pixels. It makes a tile when a dab touches it for
//! the first time, so the memory agrees with the touched area. The undo
//! edit holds only the tiles that changed.
//!
//! # Conflicts
//!
//! The layer must not change outside the stroke while the stroke is
//! active. The stroke compares the content version, the offset, the
//! buffer size, and the locks of the layer at each call. If one of them
//! changed, the call returns [`EditorError::StrokeConflict`].

use std::fmt;

use expose_core::buffer::{Rect, Rgba};

use crate::document::Document;
use crate::error::{EditorError, EditorResult, check_range};
use crate::layer::{Layer, LayerId, LayerKind, LockKind, Locks, MaskImage};
use crate::ops::Edit;

/// Smallest brush diameter in pixels.
pub const MIN_BRUSH_SIZE: f32 = 1.0;
/// Largest brush diameter in pixels.
pub const MAX_BRUSH_SIZE: f32 = 5000.0;
/// Smallest dab spacing (fraction of the diameter).
pub const MIN_BRUSH_SPACING: f32 = 0.01;
/// Largest dab spacing (fraction of the diameter).
pub const MAX_BRUSH_SPACING: f32 = 10.0;
/// Smallest distance between two dabs in pixels.
pub const MIN_DAB_STEP: f64 = 0.5;
/// Largest absolute value of a stroke point coordinate (canvas pixels).
pub const MAX_STROKE_COORD: f64 = 1.0e5;
/// Edge length of the stroke tiles in pixels.
pub const STROKE_TILE_SIZE: u32 = 64;

/// The settings of a brush.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrushSettings {
    /// Diameter in pixels, [`MIN_BRUSH_SIZE`]`..=`[`MAX_BRUSH_SIZE`].
    pub size: f32,
    /// `0.0..=1.0`. 1 gives a hard edge (1 px anti-alias). Lower values
    /// give a smooth falloff from `hardness * radius` to the radius.
    pub hardness: f32,
    /// `0.0..=1.0`. The maximum coverage of the stroke.
    pub opacity: f32,
    /// `0.0..=1.0`. The coverage that one dab adds.
    pub flow: f32,
    /// Distance between dabs as a fraction of the diameter,
    /// [`MIN_BRUSH_SPACING`]`..=`[`MAX_BRUSH_SPACING`].
    pub spacing: f32,
    /// If `true`, the pressure multiplies the diameter.
    pub pressure_size: bool,
    /// If `true`, the pressure multiplies the flow.
    pub pressure_opacity: bool,
}

impl Default for BrushSettings {
    /// Diameter 20 px, hardness 1, opacity 1, flow 1, spacing 0.25, no
    /// pressure control.
    fn default() -> Self {
        Self {
            size: 20.0,
            hardness: 1.0,
            opacity: 1.0,
            flow: 1.0,
            spacing: 0.25,
            pressure_size: false,
            pressure_opacity: false,
        }
    }
}

impl BrushSettings {
    /// Returns an error if a value is not finite or is out of its range.
    pub fn validate(&self) -> EditorResult<()> {
        check_range("brush size", self.size, MIN_BRUSH_SIZE, MAX_BRUSH_SIZE)?;
        check_range("brush hardness", self.hardness, 0.0, 1.0)?;
        check_range("brush opacity", self.opacity, 0.0, 1.0)?;
        check_range("brush flow", self.flow, 0.0, 1.0)?;
        check_range(
            "brush spacing",
            self.spacing,
            MIN_BRUSH_SPACING,
            MAX_BRUSH_SPACING,
        )
    }
}

/// What a stroke changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StrokeTarget {
    /// Paint a color on a pixel layer. The color is straight (not
    /// premultiplied) linear Rec. 2020. Values must be finite.
    Paint {
        /// The paint color.
        color: [f32; 3],
    },
    /// Erase pixels of a pixel layer (make them transparent).
    Erase,
    /// Paint a value on the mask of a layer.
    Mask {
        /// The mask value, `0.0..=1.0` (0 = black, hides; 1 = white,
        /// shows).
        value: f32,
    },
}

/// One pointer sample of a stroke.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokePoint {
    /// Horizontal canvas position in pixels.
    pub x: f64,
    /// Vertical canvas position in pixels.
    pub y: f64,
    /// Pen pressure, `0.0..=1.0`. Use 1.0 for a mouse.
    pub pressure: f32,
}

impl StrokePoint {
    /// Makes a stroke point.
    pub const fn new(x: f64, y: f64, pressure: f32) -> Self {
        Self { x, y, pressure }
    }

    fn validate(&self) -> EditorResult<()> {
        for (name, v) in [("stroke point x", self.x), ("stroke point y", self.y)] {
            if !v.is_finite() {
                return Err(EditorError::NonFinite { name });
            }
            if v.abs() > MAX_STROKE_COORD {
                return Err(EditorError::OutOfRange {
                    name,
                    value: v,
                    min: -MAX_STROKE_COORD,
                    max: MAX_STROKE_COORD,
                });
            }
        }
        check_range("stroke point pressure", self.pressure, 0.0, 1.0)
    }
}

/// How the stroke computes a result from the original value and the
/// coverage.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Paint { color: [f32; 3] },
    PaintKeepAlpha { color: [f32; 3] },
    Erase,
    Mask { value: f32 },
}

impl Mode {
    /// Result for a pixel layer. `c == 0` gives `o` exactly.
    fn pixel(self, o: Rgba, c: f32) -> Rgba {
        match self {
            Self::Paint { color } => {
                let k = 1.0 - c;
                [
                    color[0] * c + o[0] * k,
                    color[1] * c + o[1] * k,
                    color[2] * c + o[2] * k,
                    c + o[3] * k,
                ]
            }
            Self::PaintKeepAlpha { color } => [
                o[0] + (color[0] * o[3] - o[0]) * c,
                o[1] + (color[1] * o[3] - o[1]) * c,
                o[2] + (color[2] * o[3] - o[2]) * c,
                o[3],
            ],
            Self::Erase => {
                let k = 1.0 - c;
                [o[0] * k, o[1] * k, o[2] * k, o[3] * k]
            }
            // `begin` does not make a mask mode for a pixel layer.
            Self::Mask { .. } => o,
        }
    }

    /// Result for a mask. `c == 0` gives `o` exactly.
    fn mask(self, o: f32, c: f32) -> f32 {
        match self {
            Self::Mask { value } => o + (value - o) * c,
            // `begin` does not make a pixel mode for a mask.
            _ => o,
        }
    }
}

/// A value type of a layer buffer.
trait Texel: Copy {
    /// Bitwise equality (exact, also for -0.0 and NaN).
    fn same_bits(&self, other: &Self) -> bool;
}

impl Texel for Rgba {
    fn same_bits(&self, other: &Self) -> bool {
        self.iter()
            .zip(other)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

impl Texel for f32 {
    fn same_bits(&self, other: &Self) -> bool {
        self.to_bits() == other.to_bits()
    }
}

/// The original values and the coverage of one tile.
struct Tile<T> {
    /// The tile area in buffer coordinates (clipped to the buffer).
    rect: Rect,
    /// The values before the stroke, row-major in `rect`.
    orig: Vec<T>,
    /// The stroke coverage, row-major in `rect`.
    cov: Vec<f32>,
}

/// A sparse grid of tiles that covers a layer buffer.
struct TileGrid<T> {
    width: u32,
    height: u32,
    cols: u32,
    tiles: Vec<Option<Box<Tile<T>>>>,
}

impl<T: Texel> TileGrid<T> {
    fn new(width: u32, height: u32) -> Self {
        let cols = width.div_ceil(STROKE_TILE_SIZE);
        let rows = height.div_ceil(STROKE_TILE_SIZE);
        // The buffer size is below `MAX_PIXELS`, so the count fits in usize.
        let count = cols as usize * rows as usize;
        let mut tiles = Vec::new();
        tiles.resize_with(count, || None);
        Self {
            width,
            height,
            cols,
            tiles,
        }
    }

    fn tile_count(&self) -> usize {
        self.tiles.iter().filter(|t| t.is_some()).count()
    }

    /// Returns the tile `(tx, ty)`. If it does not exist, the function makes
    /// it with the current values of `buf` as the original values.
    fn tile_mut(&mut self, tx: u32, ty: u32, buf: &[T]) -> &mut Tile<T> {
        let (width, height) = (self.width, self.height);
        let index = ty as usize * self.cols as usize + tx as usize;
        self.tiles[index].get_or_insert_with(|| {
            let x = tx * STROKE_TILE_SIZE;
            let y = ty * STROKE_TILE_SIZE;
            let w = STROKE_TILE_SIZE.min(width - x);
            let h = STROKE_TILE_SIZE.min(height - y);
            let mut orig = Vec::with_capacity(w as usize * h as usize);
            for row in y..y + h {
                let start = row as usize * width as usize + x as usize;
                orig.extend_from_slice(&buf[start..start + w as usize]);
            }
            let cov = vec![0.0; orig.len()];
            Box::new(Tile {
                rect: Rect::new(x, y, w, h),
                orig,
                cov,
            })
        })
    }

    /// Writes the original values of all tiles back into `buf`. Returns
    /// `true` if a value changed.
    fn restore(&self, buf: &mut [T]) -> bool {
        let mut changed = false;
        for tile in self.tiles.iter().flatten() {
            let w = tile.rect.width as usize;
            for (row, orig) in tile.orig.chunks_exact(w).enumerate() {
                let start =
                    (tile.rect.y as usize + row) * self.width as usize + tile.rect.x as usize;
                let dst = &mut buf[start..start + w];
                changed |= dst.iter().zip(orig).any(|(a, b)| !a.same_bits(b));
                dst.copy_from_slice(orig);
            }
        }
        changed
    }

    /// Returns the tiles whose values in `buf` differ from the original
    /// values, each with its original values.
    fn changed_tiles(&mut self, buf: &[T]) -> Vec<(Rect, Vec<T>)> {
        let mut out = Vec::new();
        for slot in &mut self.tiles {
            let Some(tile) = slot.take() else { continue };
            let w = tile.rect.width as usize;
            let differs = tile.orig.chunks_exact(w).enumerate().any(|(row, orig)| {
                let start =
                    (tile.rect.y as usize + row) * self.width as usize + tile.rect.x as usize;
                buf[start..start + w]
                    .iter()
                    .zip(orig)
                    .any(|(a, b)| !a.same_bits(b))
            });
            if differs {
                let Tile { rect, orig, .. } = *tile;
                out.push((rect, orig));
            }
        }
        out
    }
}

/// The tile grid of the stroke target.
enum Grid {
    Pixel(TileGrid<Rgba>),
    Mask(TileGrid<f32>),
}

/// One dab.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Dab {
    /// Center in canvas coordinates.
    x: f64,
    y: f64,
    /// Radius in pixels.
    radius: f64,
    /// Flow multiplied by the pressure factor.
    strength: f32,
}

/// The dab shape constants (see the module documentation).
struct Shape {
    radius: f64,
    inner: f64,
    inv_falloff: f64,
    small: f32,
    /// `(radius + 0.5)^2`: pixels at this squared distance or more get 0.
    outer2: f64,
}

impl Shape {
    fn new(radius: f64, hardness: f32) -> Self {
        let inner = radius * f64::from(hardness);
        let falloff = radius - inner;
        let reach = radius + 0.5;
        // Values are in 0..=2500, so the cast to f32 is exact enough.
        #[allow(clippy::cast_possible_truncation)]
        let small = (2.0 * radius).min(1.0) as f32;
        Self {
            radius,
            inner,
            inv_falloff: if falloff > 0.0 { 1.0 / falloff } else { 0.0 },
            small,
            outer2: reach * reach,
        }
    }

    /// The dab value at the squared distance `d2` from the center.
    #[allow(clippy::cast_possible_truncation)] // Values are in 0..=1.
    fn value(&self, d2: f64) -> f32 {
        if d2 >= self.outer2 {
            return 0.0;
        }
        let d = d2.sqrt();
        let edge = (self.radius + 0.5 - d).clamp(0.0, 1.0);
        let soft = if d <= self.inner || self.inv_falloff == 0.0 {
            1.0
        } else {
            let t = ((d - self.inner) * self.inv_falloff).clamp(0.0, 1.0);
            1.0 - t * t * (3.0 - 2.0 * t)
        };
        (edge * soft) as f32 * self.small
    }
}

/// A changed area in buffer coordinates (`x0..x1`, `y0..y1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bounds {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

impl Bounds {
    fn add(this: &mut Option<Self>, x: i64, y: i64) {
        match this {
            None => {
                *this = Some(Self {
                    x0: x,
                    y0: y,
                    x1: x + 1,
                    y1: y + 1,
                });
            }
            Some(b) => {
                b.x0 = b.x0.min(x);
                b.y0 = b.y0.min(y);
                b.x1 = b.x1.max(x + 1);
                b.y1 = b.y1.max(y + 1);
            }
        }
    }
}

/// The fixed inputs of one dab application.
struct DabContext<'a> {
    offset: (i32, i32),
    selection: Option<&'a MaskImage>,
    hardness: f32,
    opacity: f32,
}

/// Applies one dab to a buffer. `f` computes the result from the original
/// value and the coverage. Adds the changed pixels to `changed`.
// Buffer coordinates are clamped to `0..=width` / `0..=height` (u32) before
// the casts, and canvas coordinates are checked against the selection size.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]
fn apply_dab<T: Texel>(
    grid: &mut TileGrid<T>,
    buf: &mut [T],
    ctx: &DabContext<'_>,
    dab: &Dab,
    f: impl Fn(T, f32) -> T,
    changed: &mut Option<Bounds>,
) {
    if dab.strength <= 0.0 || dab.radius <= 0.0 {
        return;
    }
    let shape = Shape::new(dab.radius, ctx.hardness);
    let (ox, oy) = (i64::from(ctx.offset.0), i64::from(ctx.offset.1));
    let (bw, bh) = (i64::from(grid.width), i64::from(grid.height));
    let reach = dab.radius + 1.0;
    // The dab box in buffer coordinates, clipped to the buffer.
    let x0 = ((dab.x - reach).floor() as i64 - ox).clamp(0, bw);
    let x1 = ((dab.x + reach).ceil() as i64 - ox).clamp(0, bw);
    let y0 = ((dab.y - reach).floor() as i64 - oy).clamp(0, bh);
    let y1 = ((dab.y + reach).ceil() as i64 - oy).clamp(0, bh);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let tile = i64::from(STROKE_TILE_SIZE);
    let width = grid.width as usize;
    for ty in y0 / tile..=(y1 - 1) / tile {
        for tx in x0 / tile..=(x1 - 1) / tile {
            let t = grid.tile_mut(tx as u32, ty as u32, buf);
            let (rx, ry) = (i64::from(t.rect.x), i64::from(t.rect.y));
            let tw = t.rect.width as usize;
            let (sx0, sx1) = (x0.max(rx), x1.min(rx + i64::from(t.rect.width)));
            let (sy0, sy1) = (y0.max(ry), y1.min(ry + i64::from(t.rect.height)));
            for y in sy0..sy1 {
                let dy = (y + oy) as f64 + 0.5 - dab.y;
                let dy2 = dy * dy;
                if dy2 >= shape.outer2 {
                    continue;
                }
                // The selection row (canvas coordinates). Outside the canvas
                // nothing is selected.
                let sel_row: Option<&[f32]> = match ctx.selection {
                    None => None,
                    Some(sel) => {
                        let cy = y + oy;
                        if cy < 0 || cy >= i64::from(sel.height()) {
                            continue;
                        }
                        let sw = sel.width() as usize;
                        let start = cy as usize * sw;
                        Some(&sel.data()[start..start + sw])
                    }
                };
                let trow = (y - ry) as usize * tw;
                let brow = y as usize * width;
                for x in sx0..sx1 {
                    let dx = (x + ox) as f64 + 0.5 - dab.x;
                    let v = shape.value(dx * dx + dy2);
                    if v <= 0.0 {
                        continue;
                    }
                    let sel = match sel_row {
                        None => 1.0,
                        Some(row) => {
                            let cx = x + ox;
                            if cx < 0 || cx >= row.len() as i64 {
                                continue;
                            }
                            row[cx as usize].clamp(0.0, 1.0)
                        }
                    };
                    let k = v * dab.strength * sel;
                    if k <= 0.0 {
                        continue;
                    }
                    let ti = trow + (x - rx) as usize;
                    let old = t.cov[ti];
                    let new = (old + (1.0 - old) * k).min(ctx.opacity);
                    if new <= old {
                        continue;
                    }
                    t.cov[ti] = new;
                    buf[brow + x as usize] = f(t.orig[ti], new);
                    Bounds::add(changed, x, y);
                }
            }
        }
    }
}

/// The layer state that the stroke expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Expected {
    version: u64,
    offset: (i32, i32),
    size: (u32, u32),
    locks: Locks,
}

/// An active brush stroke. See the [module documentation](self).
pub struct Stroke {
    layer: LayerId,
    mode: Mode,
    settings: BrushSettings,
    grid: Grid,
    expected: Expected,
    /// The last point of the path, if the stroke has one.
    last: Option<StrokePoint>,
    /// The path distance from the last point to the next dab.
    to_next: f64,
}

impl fmt::Debug for Stroke {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tiles = match &self.grid {
            Grid::Pixel(g) => g.tile_count(),
            Grid::Mask(g) => g.tile_count(),
        };
        f.debug_struct("Stroke")
            .field("layer", &self.layer)
            .field("mode", &self.mode)
            .field("settings", &self.settings)
            .field("tiles", &tiles)
            .finish_non_exhaustive()
    }
}

impl Stroke {
    /// Starts a stroke on a layer.
    ///
    /// Errors:
    /// - [`EditorError::OutOfRange`] / [`EditorError::NonFinite`] for bad
    ///   settings or a bad target value.
    /// - [`EditorError::LayerNotFound`].
    /// - [`EditorError::NotAPixelLayer`] for paint or erase on a layer that
    ///   is not a pixel layer; [`EditorError::NoMask`] for a mask stroke on
    ///   a layer without a mask.
    /// - [`EditorError::Locked`] if the pixels lock (or `all`) is set on
    ///   the layer or on a parent group.
    /// - [`EditorError::TransparencyLocked`] for erase with the
    ///   transparency lock.
    pub fn begin(
        doc: &Document,
        layer: LayerId,
        target: StrokeTarget,
        settings: BrushSettings,
    ) -> EditorResult<Self> {
        settings.validate()?;
        match target {
            StrokeTarget::Paint { color } => {
                if color.iter().any(|v| !v.is_finite()) {
                    return Err(EditorError::NonFinite {
                        name: "paint color",
                    });
                }
            }
            StrokeTarget::Mask { value } => check_range("mask value", value, 0.0, 1.0)?,
            StrokeTarget::Erase => {}
        }
        let l = doc.layer(layer).ok_or(EditorError::LayerNotFound(layer))?;
        let locks = doc.effective_locks(layer)?;
        locks.check(layer, LockKind::Pixels)?;
        let mode = match target {
            StrokeTarget::Paint { color } if locks.transparency => Mode::PaintKeepAlpha { color },
            StrokeTarget::Paint { color } => Mode::Paint { color },
            StrokeTarget::Erase if locks.transparency => {
                return Err(EditorError::TransparencyLocked(layer));
            }
            StrokeTarget::Erase => Mode::Erase,
            StrokeTarget::Mask { value } => Mode::Mask { value },
        };
        let expected = expected_state(l, mode, locks)?;
        let (w, h) = expected.size;
        let grid = match mode {
            Mode::Mask { .. } => Grid::Mask(TileGrid::new(w, h)),
            _ => Grid::Pixel(TileGrid::new(w, h)),
        };
        Ok(Self {
            layer,
            mode,
            settings,
            grid,
            expected,
            last: None,
            to_next: 0.0,
        })
    }

    /// Returns the layer of the stroke.
    pub const fn layer(&self) -> LayerId {
        self.layer
    }

    /// Returns the brush settings of the stroke.
    pub const fn settings(&self) -> &BrushSettings {
        &self.settings
    }

    /// Adds points to the stroke path, places the dabs, and changes the
    /// layer in place.
    ///
    /// Returns the changed rectangle in canvas coordinates, clipped to the
    /// canvas. `None` means that no pixel in the canvas changed.
    ///
    /// If a value changed, the layer gets a new pixel version (or mask
    /// version for a mask stroke), and the document a new revision.
    ///
    /// The function checks all points before it changes the layer: a bad
    /// point gives an error and no change.
    pub fn add_points(
        &mut self,
        doc: &mut Document,
        points: &[StrokePoint],
    ) -> EditorResult<Option<Rect>> {
        for p in points {
            p.validate()?;
        }
        self.check(doc)?;
        let dabs = self.place_dabs(points);
        if dabs.is_empty() {
            return Ok(None);
        }
        let (canvas_w, canvas_h) = (doc.width(), doc.height());
        let (layer, selection) = doc.layer_mut_and_selection(self.layer)?;
        let ctx = DabContext {
            offset: self.expected.offset,
            selection,
            hardness: self.settings.hardness,
            opacity: self.settings.opacity,
        };
        let mode = self.mode;
        let mut changed = None;
        match &mut self.grid {
            Grid::Pixel(grid) => {
                let LayerKind::Pixel(p) = layer.kind_mut() else {
                    return Err(EditorError::StrokeConflict(self.layer));
                };
                let buf = p.image_mut().pixels_mut();
                for dab in &dabs {
                    apply_dab(grid, buf, &ctx, dab, |o, c| mode.pixel(o, c), &mut changed);
                }
            }
            Grid::Mask(grid) => {
                let m = layer
                    .mask_mut()
                    .ok_or(EditorError::StrokeConflict(self.layer))?;
                let buf = m.image.data_mut();
                for dab in &dabs {
                    apply_dab(grid, buf, &ctx, dab, |o, c| mode.mask(o, c), &mut changed);
                }
            }
        }
        let Some(b) = changed else {
            return Ok(None);
        };
        self.touch(doc)?;
        let (ox, oy) = (
            i64::from(self.expected.offset.0),
            i64::from(self.expected.offset.1),
        );
        Ok(canvas_rect(b, ox, oy, canvas_w, canvas_h))
    }

    /// Ends the stroke and returns one undo edit for it. Give the edit to
    /// [`History::push`](crate::History::push). The edit holds the original
    /// values of the changed tiles only. Returns `None` if the stroke
    /// changed no value.
    ///
    /// Returns [`EditorError::StrokeConflict`] if the layer changed outside
    /// the stroke. Then the layer keeps the stroke values without an undo
    /// edit.
    pub fn end(mut self, doc: &mut Document) -> EditorResult<Option<Edit>> {
        self.check(doc)?;
        let id = self.layer;
        let conflict = EditorError::StrokeConflict(id);
        let layer = doc.layer(id).ok_or(conflict.clone())?;
        let edit = match &mut self.grid {
            Grid::Mask(grid) => {
                let m = layer.mask().ok_or(conflict)?;
                let tiles = grid.changed_tiles(m.image.data());
                (!tiles.is_empty()).then(|| Edit::mask_tiles(id, tiles))
            }
            Grid::Pixel(grid) => {
                let p = layer.as_pixel().ok_or(conflict)?;
                let tiles = grid.changed_tiles(p.image().pixels());
                (!tiles.is_empty()).then(|| Edit::pixel_tiles(id, tiles))
            }
        };
        Ok(edit)
    }

    /// Cancels the stroke and restores the original values of the layer.
    ///
    /// Returns [`EditorError::StrokeConflict`] (and changes nothing) if the
    /// layer changed outside the stroke.
    pub fn cancel(mut self, doc: &mut Document) -> EditorResult<()> {
        self.check(doc)?;
        let conflict = EditorError::StrokeConflict(self.layer);
        let layer = doc.layer_mut(self.layer)?;
        let changed = match &self.grid {
            Grid::Mask(grid) => {
                let m = layer.mask_mut().ok_or(conflict)?;
                grid.restore(m.image.data_mut())
            }
            Grid::Pixel(grid) => {
                let LayerKind::Pixel(p) = layer.kind_mut() else {
                    return Err(conflict);
                };
                grid.restore(p.image_mut().pixels_mut())
            }
        };
        if changed {
            self.touch(doc)?;
        }
        Ok(())
    }

    /// Returns [`EditorError::StrokeConflict`] if the layer is not in the
    /// state that the stroke expects.
    fn check(&self, doc: &Document) -> EditorResult<()> {
        let conflict = EditorError::StrokeConflict(self.layer);
        let l = doc.layer(self.layer).ok_or(conflict.clone())?;
        let locks = doc.effective_locks(self.layer)?;
        match expected_state(l, self.mode, locks) {
            Ok(state) if state == self.expected => Ok(()),
            _ => Err(conflict),
        }
    }

    /// Gives the layer a new content version and the document a new
    /// revision, and records the new version.
    fn touch(&mut self, doc: &mut Document) -> EditorResult<()> {
        let layer = doc.layer_mut(self.layer)?;
        let version = if matches!(self.mode, Mode::Mask { .. }) {
            layer.touch_mask();
            layer.mask_version()
        } else {
            layer.touch_pixels();
            layer.pixel_version()
        };
        self.expected.version = version;
        doc.touch();
        Ok(())
    }

    /// The dab for a point and pressure.
    fn dab_at(&self, x: f64, y: f64, pressure: f32) -> Dab {
        let s = &self.settings;
        let size_factor = if s.pressure_size { pressure } else { 1.0 };
        let flow_factor = if s.pressure_opacity { pressure } else { 1.0 };
        Dab {
            x,
            y,
            radius: f64::from(s.size) * 0.5 * f64::from(size_factor),
            strength: s.flow * flow_factor,
        }
    }

    /// The path distance to the next dab after a dab with `radius`.
    fn step(&self, radius: f64) -> f64 {
        (f64::from(self.settings.spacing) * 2.0 * radius).max(MIN_DAB_STEP)
    }

    /// Places the dabs for new points and updates the path state.
    #[allow(clippy::cast_possible_truncation)] // `t` is in 0..=1.
    fn place_dabs(&mut self, points: &[StrokePoint]) -> Vec<Dab> {
        let mut dabs = Vec::new();
        for &p in points {
            match self.last {
                None => {
                    let dab = self.dab_at(p.x, p.y, p.pressure);
                    self.to_next = self.step(dab.radius);
                    dabs.push(dab);
                }
                Some(prev) => {
                    let (dx, dy) = (p.x - prev.x, p.y - prev.y);
                    let len = dx.hypot(dy);
                    let mut s = 0.0;
                    // `to_next >= MIN_DAB_STEP > 0`, so the loop ends.
                    while s + self.to_next <= len {
                        s += self.to_next;
                        let t = s / len;
                        let pressure = prev.pressure + (p.pressure - prev.pressure) * t as f32;
                        let dab = self.dab_at(prev.x + dx * t, prev.y + dy * t, pressure);
                        self.to_next = self.step(dab.radius);
                        dabs.push(dab);
                    }
                    self.to_next -= len - s;
                }
            }
            self.last = Some(p);
        }
        dabs
    }
}

/// Returns the layer state for a stroke mode, or an error if the layer
/// does not fit the mode.
fn expected_state(l: &Layer, mode: Mode, locks: Locks) -> EditorResult<Expected> {
    let id = l.id();
    if let Mode::Mask { .. } = mode {
        let m = l.mask().ok_or(EditorError::NoMask(id))?;
        return Ok(Expected {
            version: l.mask_version(),
            offset: m.offset,
            size: (m.image.width(), m.image.height()),
            locks,
        });
    }
    let p = l.as_pixel().ok_or(EditorError::NotAPixelLayer(id))?;
    Ok(Expected {
        version: l.pixel_version(),
        offset: p.offset(),
        size: (p.image().width(), p.image().height()),
        locks,
    })
}

/// Converts changed bounds (buffer coordinates) to a canvas rectangle,
/// clipped to the canvas.
fn canvas_rect(b: Bounds, ox: i64, oy: i64, canvas_w: u32, canvas_h: u32) -> Option<Rect> {
    let x0 = (b.x0 + ox).max(0);
    let y0 = (b.y0 + oy).max(0);
    let x1 = (b.x1 + ox).min(i64::from(canvas_w));
    let y1 = (b.y1 + oy).min(i64::from(canvas_h));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    // All values are in 0..=canvas size, so they fit in u32.
    Some(Rect::new(
        u32::try_from(x0).ok()?,
        u32::try_from(y0).ok()?,
        u32::try_from(x1 - x0).ok()?,
        u32::try_from(y1 - y0).ok()?,
    ))
}

#[cfg(test)]
mod tests;
