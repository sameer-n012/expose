//! The document selection.
//!
//! The selection tells tools which canvas pixels they change. It is a
//! canvas-size one-channel [`MaskImage`] in the [`Document`]:
//!
//! - 1.0: the pixel is selected. 0.0: the pixel is not selected. Values
//!   between are partly selected (anti-aliased or feathered edges).
//! - `None` means "no selection": tools change all pixels. This is
//!   different from a selection with all values 0.0 ("nothing selected").
//!   An operation that removes all selected pixels (for example, a subtract
//!   or an intersect) keeps an all-zero mask. Only [`Document::deselect`]
//!   sets `None`.
//!
//! The selection is not a layer, and the `.exdoc` file does not store it.
//!
//! # Shapes
//!
//! [`SelectionShape`] coordinates are canvas pixel coordinates (`f64`). The
//! pixel `(x, y)` covers the square `x..x + 1`, `y..y + 1`. A shape can
//! extend outside the canvas; the canvas clips it.
//!
//! - Without anti-alias, a pixel is selected if its center is inside the
//!   shape. Left and top edges are inside, right and bottom edges are
//!   outside (half-open intervals), so two shapes that touch do not overlap.
//! - With anti-alias, the value is the covered area of the pixel. For a
//!   rectangle the area is exact. For an ellipse and a polygon, the
//!   function samples [`SUB_ROWS`] horizontal lines in each pixel row and
//!   adds the exact covered length on each line.
//! - A polygon is closed (the last point connects to the first point). It
//!   uses the **nonzero** fill rule: an area that a self-intersecting lasso
//!   path surrounds two times is selected (not a hole).
//!
//! All rasterization is sequential `f64`/`f32` arithmetic in a fixed order,
//! so the same input always gives the same values.
//!
//! # Modes
//!
//! [`SelectionMode`] combines the old selection `a` and the new shape `b`
//! per pixel: replace = `b`, add = `max(a, b)`, subtract = `a * (1 - b)`,
//! intersect = `a * b`. If there is no selection (`None`), the combine
//! uses `a = 0` (the user sees no selection outline, so a shape in add mode
//! makes a new selection).
//!
//! # Feather
//!
//! Feather is a Gaussian blur of the selection values. For a radius `r`
//! (pixels), the standard deviation is `sigma = r / 3`, and the kernel
//! extends `ceil(r)` pixels on each side (that is, to 3 sigma). Thus `r`
//! is the distance where the blur stops. The blur is separable (rows, then
//! columns). At the canvas edges it repeats the edge value (clamp). Radius
//! 0 does not change the values.
//!
//! # History and versions
//!
//! Each selection operation returns an [`Edit`] (one undo step). The edit
//! stores the old selection in a compact form: the value at the top-left
//! pixel, and the smallest rectangle that holds all other values. Thus a
//! rectangle or an ellipse selection costs only its bounding box, and a
//! uniform selection (select all, all zero) costs no pixel data. A full
//! canvas of feathered values costs `4 * width * height` bytes (64 MiB at
//! 4096 x 4096). [`Edit::memory_bytes`] counts this data.
//!
//! A selection change does **not** change [`Document::revision`]: the
//! composite does not use the selection, so composite caches stay valid.
//! It changes [`Document::selection_version`] instead (also at undo and
//! redo).
//!
//! # Example
//!
//! ```
//! use expose_editor::{Document, DocumentSpec, History, SelectionMode, SelectionShape};
//!
//! let mut doc = Document::new(DocumentSpec::new(64, 64))?;
//! let mut history = History::default();
//! let shape = SelectionShape::Rect { x: 8.0, y: 8.0, width: 16.0, height: 16.0 };
//! let edit = doc.select_shape(&shape, SelectionMode::Replace, true, 0.0)?;
//! history.push("Rectangle selection", vec![edit]);
//! assert_eq!(doc.selection().map(|s| s.get(10, 10)), Some(Some(1.0)));
//! history.undo(&mut doc)?;
//! assert!(doc.selection().is_none());
//! # Ok::<(), expose_editor::EditorError>(())
//! ```

use expose_core::buffer::Rect;

use crate::document::Document;
use crate::error::{EditorError, EditorResult, check_range};
use crate::layer::{LayerId, Mask, MaskFill, MaskImage};
use crate::ops::Edit;
use crate::version::next_stamp;

/// The largest number of points in a polygon shape.
pub const MAX_POLYGON_POINTS: usize = 100_000;

/// The largest absolute value of a shape coordinate or size, in pixels.
pub const MAX_SELECTION_COORD: f64 = 1.0e7;

/// The largest feather radius, in pixels.
pub const MAX_FEATHER_RADIUS: f32 = 250.0;

/// The number of sample lines in each pixel row for anti-aliased ellipse
/// and polygon shapes. A power of two, so that the sums are exact.
pub const SUB_ROWS: usize = 16;

/// A selection shape in canvas pixel coordinates (see the module
/// documentation).
#[derive(Debug, Clone, PartialEq)]
pub enum SelectionShape {
    /// An axis-aligned rectangle. `width` and `height` must not be
    /// negative.
    Rect {
        /// Left edge.
        x: f64,
        /// Top edge.
        y: f64,
        /// Width.
        width: f64,
        /// Height.
        height: f64,
    },
    /// The ellipse inside this bounding box. `width` and `height` must not
    /// be negative.
    Ellipse {
        /// Left edge of the bounding box.
        x: f64,
        /// Top edge of the bounding box.
        y: f64,
        /// Width of the bounding box.
        width: f64,
        /// Height of the bounding box.
        height: f64,
    },
    /// A closed polygon (nonzero fill rule) with
    /// `3..=`[`MAX_POLYGON_POINTS`] points.
    Polygon {
        /// The corner points `(x, y)`.
        points: Vec<(f64, f64)>,
    },
}

/// How a new shape changes the selection (see the module documentation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SelectionMode {
    /// The shape replaces the selection.
    #[default]
    Replace,
    /// The shape adds to the selection: `max(a, b)`.
    Add,
    /// The shape removes from the selection: `a * (1 - b)`.
    Subtract,
    /// Only the overlap stays selected: `a * b`.
    Intersect,
}

impl SelectionMode {
    /// Combines the old value `a` and the shape value `b`.
    fn combine(self, a: f32, b: f32) -> f32 {
        match self {
            Self::Replace => b,
            Self::Add => a.max(b),
            Self::Subtract => a * (1.0 - b),
            Self::Intersect => a * b,
        }
    }
}

/// Returns an error if `value` is not finite or is outside
/// `min..=MAX_SELECTION_COORD`.
fn check_coord(name: &'static str, value: f64, min: f64) -> EditorResult<()> {
    if !value.is_finite() {
        return Err(EditorError::NonFinite { name });
    }
    if value < min || value > MAX_SELECTION_COORD {
        return Err(EditorError::OutOfRange {
            name,
            value,
            min,
            max: MAX_SELECTION_COORD,
        });
    }
    Ok(())
}

impl SelectionShape {
    /// Checks the coordinates and the point count.
    fn validate(&self) -> EditorResult<()> {
        match self {
            Self::Rect {
                x,
                y,
                width,
                height,
            }
            | Self::Ellipse {
                x,
                y,
                width,
                height,
            } => {
                check_coord("selection x", *x, -MAX_SELECTION_COORD)?;
                check_coord("selection y", *y, -MAX_SELECTION_COORD)?;
                check_coord("selection width", *width, 0.0)?;
                check_coord("selection height", *height, 0.0)
            }
            Self::Polygon { points } => {
                let n = points.len();
                if !(3..=MAX_POLYGON_POINTS).contains(&n) {
                    // The count is at most a few million in practice, so
                    // the f64 value is exact.
                    #[allow(clippy::cast_precision_loss)]
                    return Err(EditorError::OutOfRange {
                        name: "polygon point count",
                        value: n as f64,
                        min: 3.0,
                        max: MAX_POLYGON_POINTS as f64,
                    });
                }
                for &(x, y) in points {
                    check_coord("polygon x", x, -MAX_SELECTION_COORD)?;
                    check_coord("polygon y", y, -MAX_SELECTION_COORD)?;
                }
                Ok(())
            }
        }
    }
}

// -------------------------------------------------------------------
// Rasterization.
// -------------------------------------------------------------------

/// Converts an integer-valued `f64` to an index in `0..=max`. Values out
/// of range go to the nearest end.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn to_index(value: f64, max: usize) -> usize {
    if value <= 0.0 {
        0
    } else {
        // `value` is positive here, so the cast does not lose the sign. A
        // value larger than `usize::MAX` saturates, and `min` clamps it.
        (value as usize).min(max)
    }
}

/// Converts an index to `f64`. Indices are less than
/// [`MAX_DIMENSION`](crate::MAX_DIMENSION), so the value is exact.
#[allow(clippy::cast_precision_loss)]
const fn index_f64(i: usize) -> f64 {
    i as f64
}

/// Converts an index to `u32`. Indices are at most
/// [`MAX_DIMENSION`](crate::MAX_DIMENSION), so the value fits.
#[allow(clippy::cast_possible_truncation)]
const fn index_u32(i: usize) -> u32 {
    i as u32
}

/// Rasterizes a validated shape to `width * height` values.
fn rasterize(shape: &SelectionShape, width: u32, height: u32, anti_alias: bool) -> Vec<f32> {
    match shape {
        SelectionShape::Rect {
            x,
            y,
            width: rw,
            height: rh,
        } => rasterize_rect(*x, *y, *rw, *rh, width, height, anti_alias),
        SelectionShape::Ellipse {
            x,
            y,
            width: ew,
            height: eh,
        } => {
            let mut source = SpanSource::Ellipse {
                cx: x + ew / 2.0,
                cy: y + eh / 2.0,
                rx: ew / 2.0,
                ry: eh / 2.0,
            };
            rasterize_spans(&mut source, *y, y + eh, width, height, anti_alias)
        }
        SelectionShape::Polygon { points } => {
            let (y_min, y_max) = points
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                    (lo.min(p.1), hi.max(p.1))
                });
            let mut source = SpanSource::Polygon(PolygonScan::new(points));
            rasterize_spans(&mut source, y_min, y_max, width, height, anti_alias)
        }
    }
}

/// Returns the coverage of each pixel `0..n` by the interval `a..b` on one
/// axis.
fn axis_coverage(a: f64, b: f64, n: usize, anti_alias: bool) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let p = index_f64(i);
            if anti_alias {
                // The result is in 0..=1, so f32 is sufficient.
                #[allow(clippy::cast_possible_truncation)]
                let c = (b.min(p + 1.0) - a.max(p)).clamp(0.0, 1.0) as f32;
                c
            } else {
                let c = p + 0.5;
                f32::from(u8::from(c >= a && c < b))
            }
        })
        .collect()
}

/// Rasterizes a rectangle. The coverage of a rectangle is the product of
/// the two axis coverages, so the anti-aliased area is exact.
fn rasterize_rect(
    x: f64,
    y: f64,
    rw: f64,
    rh: f64,
    width: u32,
    height: u32,
    anti_alias: bool,
) -> Vec<f32> {
    let (w, h) = (width as usize, height as usize);
    let cx = axis_coverage(x, x + rw, w, anti_alias);
    let cy = axis_coverage(y, y + rh, h, anti_alias);
    let mut data = vec![0.0; w * h];
    for (row, &vy) in data.chunks_exact_mut(w).zip(&cy) {
        if vy > 0.0 {
            for (v, &vx) in row.iter_mut().zip(&cx) {
                *v = vx * vy;
            }
        }
    }
    data
}

/// Gives the inside intervals of a shape on horizontal lines.
enum SpanSource {
    Ellipse { cx: f64, cy: f64, rx: f64, ry: f64 },
    Polygon(PolygonScan),
}

impl SpanSource {
    /// Adds the inside intervals `(x0, x1)` on the line at height `sy` to
    /// `out`. For a polygon, `sy` must increase from call to call.
    fn spans(&mut self, sy: f64, out: &mut Vec<(f64, f64)>) {
        match self {
            Self::Ellipse { cx, cy, rx, ry } => {
                if *rx <= 0.0 || *ry <= 0.0 {
                    return;
                }
                let d = (sy - *cy) / *ry;
                if d.abs() < 1.0 {
                    let half = *rx * (1.0 - d * d).sqrt();
                    out.push((*cx - half, *cx + half));
                }
            }
            Self::Polygon(scan) => scan.spans(sy, out),
        }
    }
}

/// A polygon edge that is not horizontal, with `y0 < y1`.
struct Edge {
    y0: f64,
    y1: f64,
    /// The x value at `y0`.
    x0: f64,
    /// The change of x for each unit of y.
    dxdy: f64,
    /// +1 for an edge that goes down in the point order, -1 for up.
    dir: i32,
}

/// A scanline converter for a polygon with an active edge list.
struct PolygonScan {
    /// Edges sorted by `y0` (stable sort, so the order is deterministic).
    edges: Vec<Edge>,
    /// The first edge in `edges` that is not yet active.
    next: usize,
    /// Indices of the edges that can cross the current line.
    active: Vec<usize>,
    /// The crossings `(x, dir)` on the current line.
    crossings: Vec<(f64, i32)>,
}

impl PolygonScan {
    fn new(points: &[(f64, f64)]) -> Self {
        let n = points.len();
        let mut edges = Vec::with_capacity(n);
        for i in 0..n {
            let p = points[i];
            let q = points[(i + 1) % n];
            // Horizontal edges do not cross a horizontal line.
            #[allow(clippy::float_cmp)]
            if p.1 == q.1 {
                continue;
            }
            let (top, bottom, dir) = if p.1 < q.1 { (p, q, 1) } else { (q, p, -1) };
            edges.push(Edge {
                y0: top.1,
                y1: bottom.1,
                x0: top.0,
                dxdy: (bottom.0 - top.0) / (bottom.1 - top.1),
                dir,
            });
        }
        edges.sort_by(|a, b| a.y0.total_cmp(&b.y0));
        Self {
            edges,
            next: 0,
            active: Vec::new(),
            crossings: Vec::new(),
        }
    }

    /// Adds the inside intervals on the line `sy` (nonzero rule). An edge
    /// crosses the line if `y0 <= sy < y1`.
    fn spans(&mut self, sy: f64, out: &mut Vec<(f64, f64)>) {
        while self.next < self.edges.len() && self.edges[self.next].y0 <= sy {
            self.active.push(self.next);
            self.next += 1;
        }
        let edges = &self.edges;
        self.active.retain(|&i| edges[i].y1 > sy);
        self.crossings.clear();
        for &i in &self.active {
            let e = &edges[i];
            self.crossings.push((e.x0 + (sy - e.y0) * e.dxdy, e.dir));
        }
        self.crossings
            .sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut winding = 0;
        let mut start = 0.0;
        for &(x, dir) in &self.crossings {
            let before = winding;
            winding += dir;
            if before == 0 && winding != 0 {
                start = x;
            } else if before != 0 && winding == 0 {
                out.push((start, x));
            }
        }
    }
}

/// Adds the exact covered length of the interval `a..b` to a row.
///
/// `part` holds the partly covered end pixels. `diff` is a difference
/// array (length `w + 1`) for the fully covered pixels between them.
fn add_span_area(a: f64, b: f64, weight: f32, diff: &mut [f32], part: &mut [f32]) {
    let w = part.len();
    let wf = index_f64(w);
    let (a, b) = (a.clamp(0.0, wf), b.clamp(0.0, wf));
    if b <= a {
        return;
    }
    let ia = to_index(a.floor(), w);
    let ib = to_index(b.floor(), w);
    // The lengths are in 0..=1, so f32 is sufficient.
    #[allow(clippy::cast_possible_truncation)]
    let len = |v: f64| v as f32 * weight;
    if ia == ib {
        // `a < b <= w` and `floor(a) == floor(b)`, so `ia < w`.
        part[ia] += len(b - a);
        return;
    }
    part[ia] += len(index_f64(ia) + 1.0 - a);
    diff[ia + 1] += weight;
    diff[ib] -= weight;
    if ib < w {
        part[ib] += len(b - index_f64(ib));
    }
}

/// Adds the pixels with the center in `a..b` to a row (no anti-alias).
fn add_span_centers(a: f64, b: f64, diff: &mut [f32], w: usize) {
    let xa = to_index((a - 0.5).ceil(), w);
    let xb = to_index((b - 0.5).ceil(), w);
    if xa < xb {
        diff[xa] += 1.0;
        diff[xb] -= 1.0;
    }
}

/// Rasterizes a shape from its horizontal intervals. Only the rows in
/// `y_min..y_max` can have values.
fn rasterize_spans(
    source: &mut SpanSource,
    y_min: f64,
    y_max: f64,
    width: u32,
    height: u32,
    anti_alias: bool,
) -> Vec<f32> {
    let (w, h) = (width as usize, height as usize);
    let mut data = vec![0.0; w * h];
    let row0 = to_index(y_min.floor(), h);
    let row1 = to_index(y_max.ceil(), h);
    let subs = if anti_alias { SUB_ROWS } else { 1 };
    // 1 / 16 is exact in binary floating point.
    #[allow(clippy::cast_possible_truncation)]
    let weight = (1.0 / index_f64(subs)) as f32;
    let mut diff = vec![0.0f32; w + 1];
    let mut part = vec![0.0f32; w];
    let mut spans = Vec::new();
    for y in row0..row1 {
        diff.fill(0.0);
        part.fill(0.0);
        for s in 0..subs {
            let sy = index_f64(y) + (index_f64(s) + 0.5) / index_f64(subs);
            spans.clear();
            source.spans(sy, &mut spans);
            for &(a, b) in &spans {
                if anti_alias {
                    add_span_area(a, b, weight, &mut diff, &mut part);
                } else {
                    add_span_centers(a, b, &mut diff, w);
                }
            }
        }
        let row = &mut data[y * w..(y + 1) * w];
        let mut run = 0.0f32;
        for ((v, &d), &p) in row.iter_mut().zip(&diff).zip(&part) {
            run += d;
            *v = (run + p).clamp(0.0, 1.0);
        }
    }
    data
}

// -------------------------------------------------------------------
// Feather.
// -------------------------------------------------------------------

/// Returns the normalized Gaussian kernel for a feather radius (see the
/// module documentation). The kernel has `2 * ceil(radius) + 1` taps.
fn gaussian_kernel(radius: f32) -> Vec<f32> {
    let r = to_index(f64::from(radius).ceil(), usize::MAX);
    let sigma = f64::from(radius) / 3.0;
    let weights: Vec<f64> = (0..=2 * r)
        .map(|i| {
            let d = index_f64(i) - index_f64(r);
            (-d * d / (2.0 * sigma * sigma)).exp()
        })
        .collect();
    let sum: f64 = weights.iter().sum();
    // Each weight is in 0..=1, so f32 is sufficient.
    #[allow(clippy::cast_possible_truncation)]
    weights.iter().map(|v| (v / sum) as f32).collect()
}

/// Blurs `width * height` values with a Gaussian of the feather radius.
/// Returns a new buffer. Radius 0 returns a copy.
fn feather(data: &[f32], width: u32, height: u32, radius: f32) -> Vec<f32> {
    if radius <= 0.0 {
        return data.to_vec();
    }
    let (w, h) = (width as usize, height as usize);
    let kernel = gaussian_kernel(radius);
    let r = kernel.len() / 2;

    // Rows: a padded copy of the row repeats the edge values, so that each
    // tap is one vectorizable multiply-add over the full row.
    let mut tmp = vec![0.0f32; w * h];
    let mut padded = vec![0.0f32; w + 2 * r];
    for (src, out) in data.chunks_exact(w).zip(tmp.chunks_exact_mut(w)) {
        for (j, p) in padded.iter_mut().enumerate() {
            *p = src[j.saturating_sub(r).min(w - 1)];
        }
        for (i, &k) in kernel.iter().enumerate() {
            for (o, &p) in out.iter_mut().zip(&padded[i..i + w]) {
                *o += k * p;
            }
        }
    }

    // Columns: each tap adds a full (clamped) source row.
    let mut out = vec![0.0f32; w * h];
    for (y, dst) in out.chunks_exact_mut(w).enumerate() {
        for (i, &k) in kernel.iter().enumerate() {
            let sy = (y + i).saturating_sub(r).min(h - 1);
            for (o, &s) in dst.iter_mut().zip(&tmp[sy * w..(sy + 1) * w]) {
                *o += k * s;
            }
        }
    }
    // Rounding can give values a little outside 0..=1.
    for v in &mut out {
        *v = v.clamp(0.0, 1.0);
    }
    out
}

// -------------------------------------------------------------------
// Compact storage for the history.
// -------------------------------------------------------------------

/// A selection in compact form (see "History and versions" in the module
/// documentation): all values outside `region` are `background`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredSelection {
    width: u32,
    height: u32,
    background: f32,
    /// Empty if all values are `background`.
    region: Rect,
    /// The values in `region`, row-major.
    data: Vec<f32>,
}

impl StoredSelection {
    /// Makes the compact form of a selection.
    pub(crate) fn compact(mask: &MaskImage) -> Self {
        let w = mask.width() as usize;
        let values = mask.data();
        // A mask has at least one value (`check_size` refuses size 0), so
        // the fallback value is never used.
        let background = values.first().copied().unwrap_or(0.0);
        let bits = background.to_bits();
        // Bounds of the values that are not `background` (bit compare, so
        // that the restored values are bit-for-bit the same).
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for (y, row) in values.chunks_exact(w).enumerate() {
            if let Some(first) = row.iter().position(|v| v.to_bits() != bits) {
                let last = row
                    .iter()
                    .rposition(|v| v.to_bits() != bits)
                    .unwrap_or(first);
                x0 = x0.min(first);
                x1 = x1.max(last + 1);
                y0 = y0.min(y);
                y1 = y + 1;
            }
        }
        let (region, data) = if y0 == usize::MAX {
            (Rect::new(0, 0, 0, 0), Vec::new())
        } else {
            let mut data = Vec::with_capacity((x1 - x0) * (y1 - y0));
            for row in values.chunks_exact(w).skip(y0).take(y1 - y0) {
                data.extend_from_slice(&row[x0..x1]);
            }
            let region = Rect::new(
                index_u32(x0),
                index_u32(y0),
                index_u32(x1 - x0),
                index_u32(y1 - y0),
            );
            (region, data)
        };
        Self {
            width: mask.width(),
            height: mask.height(),
            background,
            region,
            data,
        }
    }

    /// Makes the full selection again. The size must be `width x height`.
    pub(crate) fn expand(&self, width: u32, height: u32) -> EditorResult<MaskImage> {
        if (self.width, self.height) != (width, height) {
            return Err(EditorError::LengthMismatch {
                expected: width as usize * height as usize,
                actual: self.width as usize * self.height as usize,
            });
        }
        let mut mask = MaskImage::new_filled(width, height, self.background)?;
        let rw = self.region.width as usize;
        if rw > 0 {
            let w = width as usize;
            let (rx, ry) = (self.region.x as usize, self.region.y as usize);
            let dst = mask.data_mut();
            for (row, chunk) in self.data.chunks_exact(rw).enumerate() {
                let start = (ry + row) * w + rx;
                dst[start..start + rw].copy_from_slice(chunk);
            }
        }
        Ok(mask)
    }

    /// Returns the approximate heap memory in bytes.
    pub(crate) fn memory_bytes(&self) -> usize {
        size_of::<Self>() + size_of_val(self.data.as_slice())
    }
}

// -------------------------------------------------------------------
// Document operations.
// -------------------------------------------------------------------

impl Document {
    /// Returns the selection, or `None` if there is no selection (see the
    /// [module documentation](crate::selection)).
    pub fn selection(&self) -> Option<&MaskImage> {
        self.selection.as_ref()
    }

    /// Returns the selection version: a stamp that changes at each
    /// selection change (each selection operation, undo, and redo). It is
    /// unique in the process (see [`crate::version`]).
    pub const fn selection_version(&self) -> u64 {
        self.selection_version
    }

    /// Sets the selection and returns the edit that restores the old one.
    /// Gives a new selection version. Keeps the revision.
    pub(crate) fn replace_selection(&mut self, new: Option<MaskImage>) -> Edit {
        let old = std::mem::replace(&mut self.selection, new);
        self.selection_version = next_stamp();
        Edit::selection(old.as_ref().map(StoredSelection::compact))
    }

    /// Returns the selection or [`EditorError::NoSelection`].
    fn require_selection(&self) -> EditorResult<&MaskImage> {
        self.selection.as_ref().ok_or(EditorError::NoSelection)
    }

    /// Changes the selection with a shape.
    ///
    /// The function rasterizes the shape (with or without anti-alias),
    /// feathers it with `feather_radius` (pixels, `0..=`
    /// [`MAX_FEATHER_RADIUS`]; 0 = no feather), and combines it with the
    /// selection by `mode`. See the module documentation for the rules.
    ///
    /// Errors: a coordinate that is not finite or is out of range, a
    /// negative size, a polygon with fewer than 3 or more than
    /// [`MAX_POLYGON_POINTS`] points, or a feather radius out of range.
    pub fn select_shape(
        &mut self,
        shape: &SelectionShape,
        mode: SelectionMode,
        anti_alias: bool,
        feather_radius: f32,
    ) -> EditorResult<Edit> {
        shape.validate()?;
        check_range("feather radius", feather_radius, 0.0, MAX_FEATHER_RADIUS)?;
        let (w, h) = (self.width(), self.height());
        let raster = rasterize(shape, w, h, anti_alias);
        let mut values = if feather_radius > 0.0 {
            feather(&raster, w, h, feather_radius)
        } else {
            raster
        };
        if mode != SelectionMode::Replace {
            match &self.selection {
                Some(old) => {
                    for (b, &a) in values.iter_mut().zip(old.data()) {
                        *b = mode.combine(a, *b);
                    }
                }
                None => {
                    // No selection counts as `a = 0`.
                    for b in &mut values {
                        *b = mode.combine(0.0, *b);
                    }
                }
            }
        }
        let mask = MaskImage::from_data(w, h, values)?;
        Ok(self.replace_selection(Some(mask)))
    }

    /// Selects all pixels (a selection with all values 1.0).
    pub fn select_all(&mut self) -> EditorResult<Edit> {
        let mask = MaskImage::new_filled(self.width(), self.height(), 1.0)?;
        Ok(self.replace_selection(Some(mask)))
    }

    /// Removes the selection (sets `None`).
    ///
    /// Returns [`EditorError::NoSelection`] if there is no selection, so
    /// that the history does not get a step that changes nothing.
    pub fn deselect(&mut self) -> EditorResult<Edit> {
        self.require_selection()?;
        Ok(self.replace_selection(None))
    }

    /// Inverts the selection: each value `v` becomes `1 - v`.
    ///
    /// Returns [`EditorError::NoSelection`] if there is no selection. ("No
    /// selection" acts as "all selected", so its inverse would select
    /// nothing, and tools would then change nothing. An error lets the UI
    /// disable the command instead.)
    pub fn invert_selection(&mut self) -> EditorResult<Edit> {
        let old = self.require_selection()?;
        let values: Vec<f32> = old.data().iter().map(|v| 1.0 - v).collect();
        let mask = MaskImage::from_data(self.width(), self.height(), values)?;
        Ok(self.replace_selection(Some(mask)))
    }

    /// Feathers the selection with a Gaussian blur of `radius` pixels
    /// (`0..=`[`MAX_FEATHER_RADIUS`]; see the module documentation).
    ///
    /// Returns [`EditorError::NoSelection`] if there is no selection.
    pub fn feather_selection(&mut self, radius: f32) -> EditorResult<Edit> {
        check_range("feather radius", radius, 0.0, MAX_FEATHER_RADIUS)?;
        let old = self.require_selection()?;
        let values = feather(old.data(), self.width(), self.height(), radius);
        let mask = MaskImage::from_data(self.width(), self.height(), values)?;
        Ok(self.replace_selection(Some(mask)))
    }

    /// Sets the mask of a layer from the selection. The new mask replaces
    /// an existing mask. The pixels lock prevents it (as for
    /// [`Document::set_mask`]).
    ///
    /// - With a selection: a canvas-size enabled mask at offset `(0, 0)`
    ///   with the selection values. Outside the canvas the mask hides the
    ///   layer ([`MaskFill::Black`]), because no pixel there is selected.
    /// - With no selection: a canvas-size enabled mask that shows all
    ///   (values 1.0, [`MaskFill::White`]), because "no selection" acts as
    ///   "all selected".
    ///
    /// The selection does not change.
    pub fn mask_from_selection(&mut self, id: LayerId) -> EditorResult<Edit> {
        let mask = match &self.selection {
            Some(s) => Mask::new(s.clone(), (0, 0), MaskFill::Black),
            None => Mask::new(
                MaskImage::new_filled(self.width(), self.height(), 1.0)?,
                (0, 0),
                MaskFill::White,
            ),
        };
        self.set_mask(id, Some(mask))
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // Tests compare exact expected values.
mod tests {
    use super::*;
    use crate::document::DocumentSpec;
    use crate::history::History;
    use crate::layer::Locks;

    fn doc(w: u32, h: u32) -> Document {
        Document::new(DocumentSpec::new(w, h)).unwrap()
    }

    fn sum(values: &[f32]) -> f64 {
        values.iter().map(|&v| f64::from(v)).sum()
    }

    fn sel(d: &Document) -> &[f32] {
        d.selection().unwrap().data()
    }

    const fn rect(x: f64, y: f64, width: f64, height: f64) -> SelectionShape {
        SelectionShape::Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn rect_coverage_is_exact() {
        let r = rasterize(&rect(1.25, 2.5, 3.5, 2.0), 8, 8, true);
        assert!((sum(&r) - 7.0).abs() < 1e-6);
        // Left edge pixel: 0.75 wide, full height row 3.
        assert_eq!(r[3 * 8 + 1], 0.75);
        // Row 2 is half covered.
        assert_eq!(r[2 * 8 + 2], 0.5);
        // Corner pixel: 0.75 * 0.5.
        assert_eq!(r[2 * 8 + 1], 0.375);
        // Inside.
        assert_eq!(r[3 * 8 + 3], 1.0);
        // Outside.
        assert_eq!(r[0], 0.0);
        assert_eq!(r[3 * 8 + 5], 0.0);
    }

    #[test]
    fn rect_without_anti_alias_uses_centers() {
        let r = rasterize(&rect(1.25, 2.5, 3.5, 2.0), 8, 8, false);
        // Columns with center in 1.25..4.75: 1, 2, 3, 4. Rows in 2.5..4.5: 2, 3.
        assert_eq!(sum(&r), 8.0);
        assert!(r.iter().all(|&v| v == 0.0 || v == 1.0));
        assert_eq!(r[2 * 8 + 1], 1.0);
        assert_eq!(r[3 * 8 + 4], 1.0);
        assert_eq!(r[4 * 8 + 1], 0.0);
    }

    #[test]
    fn rect_outside_the_canvas_is_clipped() {
        let r = rasterize(&rect(-10.0, -10.0, 13.0, 1000.0), 8, 8, true);
        assert!((sum(&r) - 24.0).abs() < 1e-6);
        let r = rasterize(&rect(20.0, 0.0, 5.0, 5.0), 8, 8, true);
        assert_eq!(sum(&r), 0.0);
    }

    #[test]
    fn ellipse_area_and_symmetry() {
        let (w, h) = (128u32, 96u32);
        let shape = SelectionShape::Ellipse {
            x: 10.3,
            y: 7.6,
            width: 100.0,
            height: 80.0,
        };
        let r = rasterize(&shape, w, h, true);
        let expected = std::f64::consts::PI * 50.0 * 40.0;
        assert!((sum(&r) - expected).abs() / expected < 1e-3, "{}", sum(&r));
        assert!(r.iter().all(|v| (0.0..=1.0).contains(v)));
        // Center is inside, corners of the box are outside.
        assert_eq!(r[47 * w as usize + 60], 1.0);
        assert_eq!(r[8 * w as usize + 11], 0.0);
        // Symmetric about the center (centered on a pixel corner).
        let c = SelectionShape::Ellipse {
            x: 14.0,
            y: 8.0,
            width: 100.0,
            height: 80.0,
        };
        let r = rasterize(&c, w, h, true);
        let wu = w as usize;
        for y in 0..h as usize {
            for x in 0..wu {
                let mx = 127 - x; // Mirror about x = 64.
                let my = 95 - y; // Mirror about y = 48.
                assert!((r[y * wu + x] - r[y * wu + mx]).abs() < 1e-6);
                assert!((r[y * wu + x] - r[my * wu + x]).abs() < 1e-6);
            }
        }
        // Without anti-alias the values are 0 or 1, and the area is close.
        let b = rasterize(&shape, w, h, false);
        assert!(b.iter().all(|&v| v == 0.0 || v == 1.0));
        assert!((sum(&b) - expected).abs() / expected < 0.02);
    }

    #[test]
    fn degenerate_ellipse_is_empty() {
        let shape = SelectionShape::Ellipse {
            x: 1.0,
            y: 1.0,
            width: 0.0,
            height: 5.0,
        };
        assert_eq!(sum(&rasterize(&shape, 8, 8, true)), 0.0);
    }

    #[test]
    fn polygon_area_matches_shoelace() {
        let points = vec![(2.2, 1.7), (30.4, 5.1), (25.0, 28.3), (4.6, 20.9)];
        let mut area = 0.0f64;
        for i in 0..points.len() {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            area += a.0 * b.1 - b.0 * a.1;
        }
        let area = area.abs() / 2.0;
        let shape = SelectionShape::Polygon { points };
        let r = rasterize(&shape, 32, 32, true);
        assert!((sum(&r) - area).abs() / area < 2e-3, "{} {}", sum(&r), area);
        assert_eq!(r[12 * 32 + 15], 1.0);
        assert_eq!(r[31 * 32], 0.0);
        let b = rasterize(&shape, 32, 32, false);
        assert!(b.iter().all(|&v| v == 0.0 || v == 1.0));
        assert!((sum(&b) - area).abs() / area < 0.03);
    }

    #[test]
    fn polygon_rect_equals_rect_shape() {
        // An axis-aligned square on whole pixels gives exact values.
        let poly = SelectionShape::Polygon {
            points: vec![(2.0, 3.0), (6.0, 3.0), (6.0, 7.0), (2.0, 7.0)],
        };
        let a = rasterize(&poly, 10, 10, true);
        let b = rasterize(&rect(2.0, 3.0, 4.0, 4.0), 10, 10, true);
        assert_eq!(a, b);
        // Reversed point order gives the same result.
        let rev = SelectionShape::Polygon {
            points: vec![(2.0, 7.0), (6.0, 7.0), (6.0, 3.0), (2.0, 3.0)],
        };
        assert_eq!(rasterize(&rev, 10, 10, true), b);
        let c = rasterize(&poly, 10, 10, false);
        assert_eq!(c, rasterize(&rect(2.0, 3.0, 4.0, 4.0), 10, 10, false));
    }

    #[test]
    fn polygon_uses_nonzero_rule() {
        // The path goes two times around the square 2..6: nonzero fills it.
        let points = vec![
            (2.0, 2.0),
            (6.0, 2.0),
            (6.0, 6.0),
            (2.0, 6.0),
            (2.0, 2.0),
            (6.0, 2.0),
            (6.0, 6.0),
            (2.0, 6.0),
        ];
        let r = rasterize(&SelectionShape::Polygon { points }, 8, 8, true);
        assert_eq!(sum(&r), 16.0);
        assert_eq!(r[3 * 8 + 3], 1.0);
    }

    #[test]
    fn shape_validation() {
        let mut d = doc(8, 8);
        let bad = [
            rect(f64::NAN, 0.0, 1.0, 1.0),
            rect(0.0, f64::INFINITY, 1.0, 1.0),
            rect(0.0, 0.0, -1.0, 1.0),
            rect(0.0, 0.0, 1.0, 2.0e7),
            SelectionShape::Polygon {
                points: vec![(0.0, 0.0), (1.0, 1.0)],
            },
            SelectionShape::Polygon {
                points: vec![(0.0, 0.0); MAX_POLYGON_POINTS + 1],
            },
            SelectionShape::Polygon {
                points: vec![(0.0, 0.0), (1.0, f64::NAN), (2.0, 0.0)],
            },
        ];
        for shape in &bad {
            assert!(
                d.select_shape(shape, SelectionMode::Replace, true, 0.0)
                    .is_err()
            );
        }
        let ok = rect(0.0, 0.0, 2.0, 2.0);
        assert!(
            d.select_shape(&ok, SelectionMode::Replace, true, f32::NAN)
                .is_err()
        );
        assert!(
            d.select_shape(&ok, SelectionMode::Replace, true, MAX_FEATHER_RADIUS + 1.0)
                .is_err()
        );
        assert!(d.selection().is_none());
    }

    #[test]
    fn modes_combine() {
        let mut d = doc(8, 1);
        let left = rect(0.0, 0.0, 4.0, 1.0);
        let mid = rect(2.0, 0.0, 4.0, 1.0);
        d.select_shape(&left, SelectionMode::Replace, true, 0.0)
            .unwrap();
        d.select_shape(&mid, SelectionMode::Add, true, 0.0).unwrap();
        assert_eq!(sel(&d), [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0]);
        d.select_shape(&left, SelectionMode::Replace, true, 0.0)
            .unwrap();
        d.select_shape(&mid, SelectionMode::Subtract, true, 0.0)
            .unwrap();
        assert_eq!(sel(&d), [1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        d.select_shape(&left, SelectionMode::Replace, true, 0.0)
            .unwrap();
        d.select_shape(&mid, SelectionMode::Intersect, true, 0.0)
            .unwrap();
        assert_eq!(sel(&d), [0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
        // Partial values: subtract 0.5 from 1 gives 0.5.
        d.select_shape(&left, SelectionMode::Replace, true, 0.0)
            .unwrap();
        d.select_shape(
            &rect(0.5, 0.0, 1.0, 1.0),
            SelectionMode::Subtract,
            true,
            0.0,
        )
        .unwrap();
        assert_eq!(sel(&d)[..3], [0.5, 0.5, 1.0]);
        // Disjoint intersect gives an all-zero mask, not `None`.
        d.select_shape(
            &rect(6.0, 0.0, 2.0, 1.0),
            SelectionMode::Intersect,
            true,
            0.0,
        )
        .unwrap();
        assert_eq!(sum(sel(&d)), 0.0);
    }

    #[test]
    fn modes_without_selection_use_zero() {
        let shape = rect(0.0, 0.0, 4.0, 1.0);
        let mut d = doc(8, 1);
        d.select_shape(&shape, SelectionMode::Add, true, 0.0)
            .unwrap();
        assert_eq!(sum(sel(&d)), 4.0);
        for mode in [SelectionMode::Subtract, SelectionMode::Intersect] {
            let mut d = doc(8, 1);
            d.select_shape(&shape, mode, true, 0.0).unwrap();
            assert_eq!(sum(sel(&d)), 0.0);
        }
    }

    #[test]
    fn select_all_deselect_invert() {
        let mut d = doc(4, 4);
        assert_eq!(d.deselect(), Err(EditorError::NoSelection));
        assert_eq!(d.invert_selection(), Err(EditorError::NoSelection));
        assert_eq!(d.feather_selection(2.0), Err(EditorError::NoSelection));
        d.select_all().unwrap();
        assert!(sel(&d).iter().all(|&v| v == 1.0));
        d.invert_selection().unwrap();
        assert!(sel(&d).iter().all(|&v| v == 0.0));
        d.select_shape(&rect(0.0, 0.0, 1.5, 4.0), SelectionMode::Replace, true, 0.0)
            .unwrap();
        d.invert_selection().unwrap();
        assert_eq!(sel(&d)[..4], [0.0, 0.5, 1.0, 1.0]);
        d.deselect().unwrap();
        assert!(d.selection().is_none());
    }

    #[test]
    fn feather_keeps_sum_and_symmetry() {
        let (w, h) = (64u32, 64u32);
        let mut d = doc(w, h);
        d.select_shape(
            &rect(24.0, 20.0, 16.0, 24.0),
            SelectionMode::Replace,
            true,
            0.0,
        )
        .unwrap();
        let before = sum(sel(&d));
        d.feather_selection(6.0).unwrap();
        let s = sel(&d);
        assert!((sum(s) - before).abs() < 1e-2, "{} {}", sum(s), before);
        assert!(s.iter().all(|v| (0.0..=1.0).contains(v)));
        // The edges are soft: the two pixels at the old edge (x = 24) are
        // symmetric about 0.5, and the value is 0 far outside.
        let wu = w as usize;
        let (out, inside) = (s[32 * wu + 23], s[32 * wu + 24]);
        assert!(((out + inside) - 1.0).abs() < 1e-3, "{out} {inside}");
        assert!(out > 0.0 && out < 0.5 && inside > 0.5 && inside < 1.0);
        assert_eq!(s[32 * wu + 10], 0.0);
        // Symmetric about x = 32 and y = 32.
        for y in 0..64 {
            for x in 0..64 {
                let v = s[y * wu + x];
                assert!((v - s[y * wu + (63 - x)]).abs() < 1e-5);
                assert!((v - s[(63 - y) * wu + x]).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn feather_clamps_at_edges() {
        let mut d = doc(16, 16);
        d.select_all().unwrap();
        d.feather_selection(5.0).unwrap();
        assert!(sel(&d).iter().all(|&v| (v - 1.0).abs() < 1e-6));
    }

    #[test]
    fn select_shape_with_feather_equals_two_steps() {
        let shape = SelectionShape::Ellipse {
            x: 4.0,
            y: 4.0,
            width: 20.0,
            height: 12.0,
        };
        let mut a = doc(32, 32);
        a.select_shape(&shape, SelectionMode::Replace, true, 3.0)
            .unwrap();
        let mut b = doc(32, 32);
        b.select_shape(&shape, SelectionMode::Replace, true, 0.0)
            .unwrap();
        b.feather_selection(3.0).unwrap();
        assert_eq!(a.selection(), b.selection());
    }

    #[test]
    fn mask_from_selection_and_locks() {
        let mut d = doc(4, 4);
        let id = d.layers()[0].id();
        d.mask_from_selection(id).unwrap();
        let m = d.layer(id).unwrap().mask().unwrap();
        assert!(m.image.data().iter().all(|&v| v == 1.0));
        assert_eq!(m.fill, MaskFill::White);
        d.select_shape(&rect(0.0, 0.0, 2.0, 4.0), SelectionMode::Replace, true, 0.0)
            .unwrap();
        let undo = d.mask_from_selection(id).unwrap();
        let m = d.layer(id).unwrap().mask().unwrap();
        assert_eq!(m.image.data(), sel(&d));
        assert_eq!(m.offset, (0, 0));
        assert_eq!(m.fill, MaskFill::Black);
        assert!(m.enabled);
        // Undo restores the old (reveal all) mask.
        d.apply_edit(undo).unwrap();
        assert_eq!(d.layer(id).unwrap().mask().unwrap().fill, MaskFill::White);

        for locks in [
            Locks {
                pixels: true,
                ..Locks::default()
            },
            Locks {
                all: true,
                ..Locks::default()
            },
        ] {
            d.set_locks(id, locks).unwrap();
            let before = d.clone();
            let err = d.mask_from_selection(id).unwrap_err();
            assert!(matches!(err, EditorError::Locked { .. }));
            assert_eq!(d, before);
        }
        d.set_locks(
            id,
            Locks {
                position: true,
                transparency: true,
                ..Locks::default()
            },
        )
        .unwrap();
        assert!(d.mask_from_selection(id).is_ok());
    }

    type Step<'a> = Box<dyn Fn(&mut Document) -> EditorResult<Edit> + 'a>;

    #[test]
    fn undo_redo_each_operation() {
        let mut d = doc(16, 16);
        let id = d.layers()[0].id();
        let mut history = History::default();
        let mut states = vec![d.clone()];
        let ellipse = SelectionShape::Ellipse {
            x: 2.0,
            y: 3.0,
            width: 9.0,
            height: 7.0,
        };
        let small = rect(1.0, 1.0, 5.0, 5.0);
        let steps: Vec<Step<'_>> = vec![
            Box::new(Document::select_all),
            Box::new(|d| d.select_shape(&ellipse, SelectionMode::Subtract, true, 0.0)),
            Box::new(Document::invert_selection),
            Box::new(|d| d.feather_selection(2.5)),
            Box::new(|d| d.select_shape(&small, SelectionMode::Add, false, 1.0)),
            Box::new(move |d| d.mask_from_selection(id)),
            Box::new(Document::deselect),
        ];
        for step in &steps {
            let edit = step(&mut d).unwrap();
            history.push("step", vec![edit]);
            states.push(d.clone());
        }
        for i in (0..steps.len()).rev() {
            assert!(history.undo(&mut d).unwrap());
            assert_eq!(d, states[i], "undo to {i}");
        }
        for (i, state) in states.iter().enumerate().skip(1) {
            assert!(history.redo(&mut d).unwrap());
            assert_eq!(&d, state, "redo to {i}");
        }
    }

    #[test]
    fn revision_unchanged_and_selection_version_changes() {
        let mut d = doc(8, 8);
        let mut history = History::default();
        let rev = d.revision();
        let mut ver = d.selection_version();
        let mut check = |d: &Document| {
            assert_eq!(d.revision(), rev);
            assert!(d.selection_version() > ver);
            ver = d.selection_version();
        };
        let e = d.select_all().unwrap();
        check(&d);
        history.push("all", vec![e]);
        let e = d
            .select_shape(&rect(1.0, 1.0, 2.0, 2.0), SelectionMode::Add, true, 0.0)
            .unwrap();
        check(&d);
        history.push("rect", vec![e]);
        let e = d.invert_selection().unwrap();
        check(&d);
        history.push("invert", vec![e]);
        let e = d.feather_selection(1.0).unwrap();
        check(&d);
        history.push("feather", vec![e]);
        let e = d.deselect().unwrap();
        check(&d);
        history.push("deselect", vec![e]);
        history.undo(&mut d).unwrap();
        check(&d);
        history.redo(&mut d).unwrap();
        check(&d);
        // A failed operation changes nothing.
        let before = d.selection_version();
        assert!(d.invert_selection().is_err());
        assert_eq!(d.selection_version(), before);
        // Mask from selection changes the layers, so it changes the revision.
        let id = d.layers()[0].id();
        d.mask_from_selection(id).unwrap();
        assert_ne!(d.revision(), rev);
        assert_eq!(d.selection_version(), before);
    }

    #[test]
    fn compact_storage() {
        let mut d = doc(64, 64);
        d.select_all().unwrap();
        // The edit of the next step stores the uniform "all" selection.
        let e = d
            .select_shape(
                &rect(10.5, 20.0, 5.0, 3.0),
                SelectionMode::Replace,
                true,
                0.0,
            )
            .unwrap();
        assert!(e.memory_bytes() < 256, "{}", e.memory_bytes());
        // The rect selection is stored as its 6 x 3 bounding box.
        let e = d.select_all().unwrap();
        assert!(e.memory_bytes() < 256 + 18 * 4, "{}", e.memory_bytes());
        // Round trip of an inverted shape (background 1.0).
        d.select_shape(
            &rect(10.5, 20.0, 5.0, 3.0),
            SelectionMode::Subtract,
            true,
            2.0,
        )
        .unwrap();
        let mask = d.selection().unwrap().clone();
        let stored = StoredSelection::compact(&mask);
        assert!(stored.data.len() < 64 * 64);
        assert_eq!(stored.expand(64, 64).unwrap(), mask);
        assert!(stored.expand(32, 64).is_err());
    }

    #[test]
    fn deterministic() {
        let shapes = [
            SelectionShape::Ellipse {
                x: 3.3,
                y: 4.4,
                width: 40.1,
                height: 30.2,
            },
            SelectionShape::Polygon {
                points: vec![
                    (1.0, 1.0),
                    (50.0, 9.5),
                    (20.0, 60.0),
                    (40.0, 30.0),
                    (3.0, 45.0),
                ],
            },
        ];
        for shape in &shapes {
            let mut a = doc(64, 64);
            let mut b = doc(64, 64);
            a.select_shape(shape, SelectionMode::Replace, true, 4.0)
                .unwrap();
            b.select_shape(shape, SelectionMode::Replace, true, 4.0)
                .unwrap();
            let bits = |d: &Document| sel(d).iter().map(|v| v.to_bits()).collect::<Vec<_>>();
            assert_eq!(bits(&a), bits(&b));
        }
    }

    /// Run with `cargo test -p expose-editor --release -- --ignored
    /// bench_ellipse_feather --nocapture`.
    #[test]
    #[ignore = "benchmark"]
    fn bench_ellipse_feather() {
        let mut d = doc(4096, 4096);
        let shape = SelectionShape::Ellipse {
            x: 300.5,
            y: 200.25,
            width: 3500.0,
            height: 3600.0,
        };
        let t = std::time::Instant::now();
        d.select_shape(&shape, SelectionMode::Replace, true, 0.0)
            .unwrap();
        let raster = t.elapsed();
        let t = std::time::Instant::now();
        d.select_shape(&shape, SelectionMode::Replace, true, 20.0)
            .unwrap();
        let total = t.elapsed();
        println!("4096x4096 ellipse: raster {raster:?}, raster + feather 20 px {total:?}");
    }
}
