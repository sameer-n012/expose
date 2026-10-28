//! Outline polygons of a single-channel image (marching squares).
//!
//! The editor uses [`trace_contours`] to draw the selection outline.
//!
//! # Method
//!
//! 1. The function samples the image on a grid. With `step == 1`, one sample
//!    is one pixel, at the pixel center. With `step > 1`, one sample is the
//!    **maximum** of a `step` x `step` block, at the block center. The
//!    maximum keeps thin selections (for example a 1 px line) visible when
//!    the view is zoomed out.
//! 2. A ring of samples with the value 0 surrounds the grid ("outside the
//!    image counts as 0"). These border samples sit on the image border, and
//!    a crossing between an image sample and a border sample is put on the
//!    image border. Thus an outline never goes out of the image, and an
//!    outline along the image border has sharp corners.
//! 3. A sample is *inside* when its value is `>= threshold`. Each grid cell
//!    (four samples) gives zero, one, or two line segments. A segment point
//!    on a cell edge is the linear interpolation of the threshold crossing.
//! 4. Saddle cells (two diagonal corners inside, two outside) use the mean
//!    of the four corner values. If the mean is `>= threshold`, the two
//!    inside corners connect; if not, they stay apart. Neighbor cells share
//!    the edge points, so contours never cross.
//! 5. A hash map on edge ids joins the segments into closed polylines.
//!
//! # Output rules
//!
//! - Points are in image pixel coordinates: pixel `(x, y)` covers
//!   `[x, x + 1) x [y, y + 1)`, and pixel corners are at integer values.
//! - Each contour is closed. The first point is **not** repeated at the end.
//! - Consecutive points on the same horizontal or vertical line are merged,
//!   so a rectangle along pixel rows and columns has few points. Duplicate
//!   points are removed. Contours with fewer than 3 points (zero area) are
//!   removed.
//! - Orientation: the inside area is always on the **left** side of the
//!   walk direction, as seen on screen (y axis points down). Thus outer
//!   outlines go counter-clockwise on screen and holes go clockwise. In raw
//!   coordinates (y down), [`Contour::signed_area`] is negative for outer
//!   outlines and positive for holes.
//! - Interior corners of a hard-edged (0 or 1) mask are cut by half a sample,
//!   as in all marching-squares methods. Corners on the image border stay
//!   sharp (see step 2).
//! - A NaN value counts as 0.
//! - The output order is deterministic (scan order of the first segment of
//!   each contour).

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use super::GrayImage;
use crate::error::{CoreError, CoreResult};

/// One closed outline.
///
/// Points are in image pixel coordinates (pixel `(x, y)` covers
/// `[x, x + 1) x [y, y + 1)`; a pixel corner is at integer coordinates). The
/// last point connects to the first point; the first point is not repeated.
#[derive(Debug, Clone, PartialEq)]
pub struct Contour {
    /// Polygon points `[x, y]`.
    pub points: Vec<[f32; 2]>,
}

impl Contour {
    /// Returns the signed area of the polygon (shoelace formula) in raw
    /// image coordinates, where the y axis points down.
    ///
    /// The value is negative for an outer outline (counter-clockwise on
    /// screen) and positive for a hole (clockwise on screen).
    pub fn signed_area(&self) -> f64 {
        let n = self.points.len();
        let mut sum = 0.0f64;
        for i in 0..n {
            let [x0, y0] = self.points[i];
            let [x1, y1] = self.points[(i + 1) % n];
            sum += f64::from(x0) * f64::from(y1) - f64::from(x1) * f64::from(y0);
        }
        sum * 0.5
    }
}

/// Traces the iso-lines at `threshold` of `image`.
///
/// - `threshold`: the iso value (for example 0.5 for a selection mask). A
///   sample with a value `>= threshold` is inside.
/// - `step`: the sample distance in pixels (1 or more). Use `step > 1` when
///   the view is zoomed out, so that the outline has about one point per
///   screen pixel. Each sample is the maximum of a `step` x `step` block.
///   The output points are always in image coordinates.
/// - `max_points`: the permitted total number of points in all contours.
///
/// Values outside the image count as 0, so every contour is closed. Refer
/// to the module documentation for the full rules (orientation, merging,
/// saddle cells).
///
/// # Errors
///
/// - [`CoreError::OutOfRange`] if `step` is 0.
/// - [`CoreError::NonFinite`] if `threshold` is NaN or infinite.
/// - [`CoreError::ContourTooLarge`] if the total point count after merging
///   is more than `max_points`. The function does not return a partial
///   result; the caller can try again with a larger `step`.
pub fn trace_contours(
    image: &GrayImage,
    threshold: f32,
    step: u32,
    max_points: usize,
) -> CoreResult<Vec<Contour>> {
    if step == 0 {
        return Err(CoreError::OutOfRange {
            name: "step",
            value: 0.0,
            min: 1.0,
            max: f64::from(u32::MAX),
        });
    }
    if !threshold.is_finite() {
        return Err(CoreError::NonFinite { name: "threshold" });
    }
    let grid = Grid::new(image, step);
    let segments = collect_segments(image, &grid, threshold);
    join_segments(&segments, max_points)
}

/// Sample positions of the padded grid along one axis.
///
/// Index 0 and index `n + 1` are the border samples, at the image border.
/// Index `1..=n` are the block centers.
fn axis_positions(size: u32, step: u32) -> Vec<f32> {
    let n = size.div_ceil(step);
    let mut pos = Vec::with_capacity(n as usize + 2);
    pos.push(0.0);
    for i in 0..n {
        let lo = u64::from(i) * u64::from(step);
        let hi = (lo + u64::from(step)).min(u64::from(size));
        // Block centers are multiples of 0.5. They are exact in f32 up to
        // 2^23 px; f32 rounds larger values, which is acceptable for
        // display outlines.
        #[allow(clippy::cast_precision_loss)]
        pos.push((lo + hi) as f32 * 0.5);
    }
    #[allow(clippy::cast_precision_loss)]
    pos.push(size as f32);
    pos
}

/// The padded sample grid.
struct Grid {
    step: usize,
    /// Number of image samples per row (without border samples).
    nx: usize,
    /// Number of image sample rows (without border rows).
    ny: usize,
    /// Padded grid width (`nx + 2`).
    gx: usize,
    /// Padded grid height (`ny + 2`).
    gy: usize,
    xs: Vec<f32>,
    ys: Vec<f32>,
}

impl Grid {
    fn new(image: &GrayImage, step: u32) -> Self {
        let xs = axis_positions(image.width(), step);
        let ys = axis_positions(image.height(), step);
        let gx = xs.len();
        let gy = ys.len();
        Self {
            step: step as usize,
            nx: gx - 2,
            ny: gy - 2,
            gx,
            gy,
            xs,
            ys,
        }
    }

    /// Edge id of the horizontal edge from sample `(gi, gj)` to `(gi + 1, gj)`.
    fn h_id(&self, gi: usize, gj: usize) -> u64 {
        (gj as u64 * self.gx as u64 + gi as u64) * 2
    }

    /// Edge id of the vertical edge from sample `(gi, gj)` to `(gi, gj + 1)`.
    fn v_id(&self, gi: usize, gj: usize) -> u64 {
        (gj as u64 * self.gx as u64 + gi as u64) * 2 + 1
    }
}

/// Fills `out[1..=nx]` with the samples of image sample row `j`. `out[0]`
/// and `out[nx + 1]` are border samples (0).
fn sample_row(image: &GrayImage, grid: &Grid, j: usize, out: &mut [f32]) {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let data = image.data();
    let y0 = j * grid.step;
    let y1 = (y0 + grid.step).min(height);
    out.fill(0.0);
    let inner = &mut out[1..=grid.nx];
    if grid.step == 1 {
        // Fast path: one pixel per sample.
        let row = &data[y0 * width..(y0 + 1) * width];
        for (dst, &v) in inner.iter_mut().zip(row) {
            // A NaN value counts as 0 (module rule).
            *dst = if v.is_nan() { 0.0 } else { v };
        }
        return;
    }
    inner.fill(f32::NEG_INFINITY);
    for y in y0..y1 {
        let row = &data[y * width..(y + 1) * width];
        for (dst, block) in inner.iter_mut().zip(row.chunks(grid.step)) {
            for &v in block {
                // A NaN value counts as 0 (module rule).
                let v = if v.is_nan() { 0.0 } else { v };
                if v > *dst {
                    *dst = v;
                }
            }
        }
    }
}

/// One directed segment in one cell. The inside area is on its left side
/// (on screen).
struct Segment {
    start_edge: u64,
    end_edge: u64,
    start: [f32; 2],
    /// An extra point between `start` and the end point (an image corner).
    corner: Option<[f32; 2]>,
}

/// Cell edges in clockwise order (on screen). Edge `k` goes from corner `k`
/// to corner `(k + 1) % 4`. Corners: 0 top-left, 1 top-right,
/// 2 bottom-right, 3 bottom-left. Edge 3 is the left edge; the `match`
/// expressions below use the default arm for it.
const EDGE_T: usize = 0;
const EDGE_R: usize = 1;
const EDGE_B: usize = 2;

/// The crossing point between two samples on one axis.
///
/// A border sample (`a_border`, `b_border`) gets the crossing on its own
/// position, which is on the image border.
fn crossing(pa: f32, pb: f32, va: f32, vb: f32, a_border: bool, b_border: bool, t: f32) -> f32 {
    if a_border {
        return pa;
    }
    if b_border {
        return pb;
    }
    let f = (t - va) / (vb - va);
    // `f` is not finite only for infinite sample values; then use the edge
    // center. The clamp keeps the point on the edge.
    let f = if f.is_finite() {
        f.clamp(0.0, 1.0)
    } else {
        0.5
    };
    pa + f * (pb - pa)
}

/// Data of one cell for the segment builder.
struct Cell {
    gi: usize,
    gj: usize,
    /// Corner values in the order top-left, top-right, bottom-right,
    /// bottom-left.
    v: [f32; 4],
    inside: [bool; 4],
}

fn edge_id(grid: &Grid, cell: &Cell, edge: usize) -> u64 {
    match edge {
        EDGE_T => grid.h_id(cell.gi, cell.gj),
        EDGE_R => grid.v_id(cell.gi + 1, cell.gj),
        EDGE_B => grid.h_id(cell.gi, cell.gj + 1),
        _ => grid.v_id(cell.gi, cell.gj),
    }
}

fn edge_point(grid: &Grid, cell: &Cell, edge: usize, t: f32) -> [f32; 2] {
    let (gi, gj) = (cell.gi, cell.gj);
    let x0 = grid.xs[gi];
    let x1 = grid.xs[gi + 1];
    let y0 = grid.ys[gj];
    let y1 = grid.ys[gj + 1];
    let left_border = gi == 0;
    let right_border = gi + 1 == grid.gx - 1;
    let top_border = gj == 0;
    let bottom_border = gj + 1 == grid.gy - 1;
    let [tl, tr, br, bl] = cell.v;
    match edge {
        EDGE_T => [crossing(x0, x1, tl, tr, left_border, right_border, t), y0],
        EDGE_R => [x1, crossing(y0, y1, tr, br, top_border, bottom_border, t)],
        EDGE_B => [crossing(x0, x1, bl, br, left_border, right_border, t), y1],
        _ => [x0, crossing(y0, y1, tl, bl, top_border, bottom_border, t)],
    }
}

/// Returns the image corner if the cell is one of the four corner cells of
/// the padded grid. In such a cell the corner sample is a border sample at
/// the image corner, and the outline must go through it.
fn image_corner(grid: &Grid, cell: &Cell) -> Option<[f32; 2]> {
    let last_i = grid.gx - 2;
    let last_j = grid.gy - 2;
    let x = if cell.gi == 0 {
        grid.xs[0]
    } else if cell.gi == last_i {
        grid.xs[grid.gx - 1]
    } else {
        return None;
    };
    let y = if cell.gj == 0 {
        grid.ys[0]
    } else if cell.gj == last_j {
        grid.ys[grid.gy - 1]
    } else {
        return None;
    };
    Some([x, y])
}

/// Adds the segments of one cell that is not uniform.
fn add_cell_segments(grid: &Grid, cell: &Cell, t: f32, out: &mut Vec<Segment>) {
    let c = cell.inside;
    // Edge k "enters" when corner k is outside and corner k + 1 is inside
    // (walking clockwise around the cell); it "leaves" in the other case.
    let mut enters = [0usize; 2];
    let mut n_enter = 0;
    for k in 0..4 {
        if !c[k] && c[(k + 1) % 4] {
            enters[n_enter] = k;
            n_enter += 1;
        }
    }
    let corner = image_corner(grid, cell);
    let mut push = |enter: usize, leave: usize| {
        out.push(Segment {
            start_edge: edge_id(grid, cell, enter),
            end_edge: edge_id(grid, cell, leave),
            start: edge_point(grid, cell, enter, t),
            corner,
        });
    };
    if n_enter == 1 {
        // One inside run: it leaves at the next edge where corner k is
        // inside and corner k + 1 is outside.
        let e = enters[0];
        let mut leave = (e + 1) % 4;
        while !c[leave] || c[(leave + 1) % 4] {
            leave = (leave + 1) % 4;
        }
        push(e, leave);
    } else {
        // Saddle: two inside corners on one diagonal. The mean of the four
        // values decides if the center is inside.
        let mean = (cell.v[0] + cell.v[1] + cell.v[2] + cell.v[3]) * 0.25;
        let center_inside = mean >= t;
        for &e in &enters[..n_enter] {
            // Center outside: each inside corner is cut off alone, so the
            // segment leaves at the next edge. Center inside: the inside
            // corners connect, the outside corners are cut off, so the
            // segment leaves at the previous leave edge.
            let leave = if center_inside {
                (e + 3) % 4
            } else {
                (e + 1) % 4
            };
            push(e, leave);
        }
    }
}

/// Scans all cells and returns the directed segments in scan order.
fn collect_segments(image: &GrayImage, grid: &Grid, t: f32) -> Vec<Segment> {
    let gx = grid.gx;
    let mut top = vec![0.0f32; gx];
    let mut bottom = vec![0.0f32; gx];
    // Inside flags as bytes (0 or 1), so that 8 cells can be checked with
    // one 64-bit compare.
    let mut top_in = vec![0u8; gx];
    let mut bottom_in = vec![0u8; gx];
    let border_inside = u8::from(0.0 >= t);
    top_in.fill(border_inside);
    let mut segments = Vec::new();

    for gj in 0..grid.gy - 1 {
        // Bottom row of this cell row: padded row gj + 1.
        if gj < grid.ny {
            sample_row(image, grid, gj, &mut bottom);
        } else {
            bottom.fill(0.0);
        }
        for (flag, &v) in bottom_in.iter_mut().zip(bottom.iter()) {
            *flag = u8::from(v >= t);
        }
        let mut gi = 0;
        while gi < gx - 1 {
            // Skip 8 uniform cells at once: cells gi..gi + 8 use the flags
            // gi..gi + 9 of both rows.
            if let (Some(a), Some(a1), Some(b), Some(b1)) = (
                top_in[gi..].first_chunk::<8>(),
                top_in[gi + 1..].first_chunk::<8>(),
                bottom_in[gi..].first_chunk::<8>(),
                bottom_in[gi + 1..].first_chunk::<8>(),
            ) {
                let a = u64::from_ne_bytes(*a);
                let diff = (a ^ u64::from_ne_bytes(*a1))
                    | (a ^ u64::from_ne_bytes(*b))
                    | (a ^ u64::from_ne_bytes(*b1));
                if diff == 0 {
                    gi += 8;
                    continue;
                }
            }
            let inside = [
                top_in[gi] != 0,
                top_in[gi + 1] != 0,
                bottom_in[gi + 1] != 0,
                bottom_in[gi] != 0,
            ];
            if inside[0] == inside[1] && inside[1] == inside[2] && inside[2] == inside[3] {
                gi += 1;
                continue;
            }
            let cell = Cell {
                gi,
                gj,
                v: [top[gi], top[gi + 1], bottom[gi + 1], bottom[gi]],
                inside,
            };
            add_cell_segments(grid, &cell, t, &mut segments);
            gi += 1;
        }
        std::mem::swap(&mut top, &mut bottom);
        std::mem::swap(&mut top_in, &mut bottom_in);
    }
    segments
}

/// A fast hasher for `u64` edge ids (multiplicative hash). The ids are not
/// attacker-controlled in a way that matters: the worst case is slow
/// lookups, not wrong results.
#[derive(Default)]
struct EdgeHasher(u64);

impl Hasher for EdgeHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(8) ^ u64::from(b)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }

    fn write_u64(&mut self, n: u64) {
        let h = n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        // Mix the high bits down, because the table uses the low bits for
        // the bucket and the high bits for the control bytes.
        self.0 = h ^ (h >> 29);
    }
}

type EdgeMap = HashMap<u64, usize, BuildHasherDefault<EdgeHasher>>;

/// True if `b` is on the same horizontal or vertical line as `a` and `c`.
///
/// The compare is exact on purpose: points on one sample row or column
/// have bit-identical coordinates, and only those are merged.
#[allow(clippy::float_cmp)]
fn axis_collinear(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    (a[0] == b[0] && b[0] == c[0]) || (a[1] == b[1] && b[1] == c[1])
}

/// Adds `p` to the open polyline `out`, with duplicate and collinear merge.
/// Duplicates are bit-identical points (exact compare on purpose).
#[allow(clippy::float_cmp)]
fn push_merged(out: &mut Vec<[f32; 2]>, p: [f32; 2]) {
    if out.last() == Some(&p) {
        return;
    }
    while out.len() >= 2 && axis_collinear(out[out.len() - 2], out[out.len() - 1], p) {
        out.pop();
    }
    out.push(p);
}

/// Merges across the closing link (last point to first point).
#[allow(clippy::float_cmp)]
fn close_merged(points: &mut Vec<[f32; 2]>) {
    loop {
        let n = points.len();
        if n < 3 {
            return;
        }
        if points[n - 1] == points[0] || axis_collinear(points[n - 2], points[n - 1], points[0]) {
            points.pop();
            continue;
        }
        if axis_collinear(points[n - 1], points[0], points[1]) {
            points.remove(0);
            continue;
        }
        return;
    }
}

/// Joins the directed segments into closed contours.
fn join_segments(segments: &[Segment], max_points: usize) -> CoreResult<Vec<Contour>> {
    let mut by_start =
        EdgeMap::with_capacity_and_hasher(segments.len(), BuildHasherDefault::default());
    for (i, s) in segments.iter().enumerate() {
        by_start.insert(s.start_edge, i);
    }
    let mut used = vec![false; segments.len()];
    let mut contours = Vec::new();
    let mut total = 0usize;
    for first in 0..segments.len() {
        if used[first] {
            continue;
        }
        let mut points = Vec::new();
        let mut i = first;
        loop {
            used[i] = true;
            let s = &segments[i];
            push_merged(&mut points, s.start);
            if let Some(c) = s.corner {
                push_merged(&mut points, c);
            }
            // Each crossing edge has exactly two cells. The flags of its two
            // samples are the same in both cells, so one cell starts a
            // segment on it and the other cell ends one there. Thus the
            // lookup always finds a segment and the walk returns to `first`.
            let Some(&next) = by_start.get(&s.end_edge) else {
                debug_assert!(false, "open contour at edge {}", s.end_edge);
                break;
            };
            i = next;
            if i == first || used[i] {
                break;
            }
        }
        close_merged(&mut points);
        if points.len() < 3 {
            continue;
        }
        total += points.len();
        if total > max_points {
            return Err(CoreError::ContourTooLarge { limit: max_points });
        }
        contours.push(Contour { points });
    }
    Ok(contours)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    #![allow(clippy::cast_precision_loss, reason = "test sizes are small")]
    #![allow(clippy::cast_possible_truncation, reason = "sign of a product")]
    #![allow(clippy::float_cmp, reason = "the tests check exact points")]
    #![allow(clippy::many_single_char_names, reason = "geometry names")]

    use super::*;

    fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> f32) -> GrayImage {
        let mut data = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                data.push(f(x, y));
            }
        }
        GrayImage::from_data(w, h, data).unwrap()
    }

    fn rect_mask(w: u32, h: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> GrayImage {
        image(w, h, |x, y| {
            if x >= x0 && x < x1 && y >= y0 && y < y1 {
                1.0
            } else {
                0.0
            }
        })
    }

    fn bounds(c: &Contour) -> [f32; 4] {
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for p in &c.points {
            b[0] = b[0].min(p[0]);
            b[1] = b[1].min(p[1]);
            b[2] = b[2].max(p[0]);
            b[3] = b[3].max(p[1]);
        }
        b
    }

    /// True if the open segments `p1-p2` and `p3-p4` cross at one inner point.
    fn proper_cross(p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], p4: [f32; 2]) -> bool {
        let o = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| {
            let v = f64::from(b[0] - a[0]) * f64::from(c[1] - a[1])
                - f64::from(b[1] - a[1]) * f64::from(c[0] - a[0]);
            v.signum() as i32 * i32::from(v != 0.0)
        };
        let d1 = o(p3, p4, p1);
        let d2 = o(p3, p4, p2);
        let d3 = o(p1, p2, p3);
        let d4 = o(p1, p2, p4);
        d1 * d2 < 0 && d3 * d4 < 0
    }

    fn assert_no_crossings(contours: &[Contour]) {
        let mut edges = Vec::new();
        for c in contours {
            let n = c.points.len();
            for i in 0..n {
                edges.push((c.points[i], c.points[(i + 1) % n]));
            }
        }
        for i in 0..edges.len() {
            for j in i + 1..edges.len() {
                assert!(
                    !proper_cross(edges[i].0, edges[i].1, edges[j].0, edges[j].1),
                    "edges {:?} and {:?} cross",
                    edges[i],
                    edges[j]
                );
            }
        }
    }

    fn assert_closed_form(contours: &[Contour]) {
        for c in contours {
            assert!(c.points.len() >= 3);
            assert_ne!(c.points.first(), c.points.last(), "first point repeated");
        }
    }

    #[test]
    fn rejects_bad_arguments() {
        let img = GrayImage::new_filled(4, 4, 0.0).unwrap();
        assert!(matches!(
            trace_contours(&img, 0.5, 0, 100),
            Err(CoreError::OutOfRange { name: "step", .. })
        ));
        assert!(matches!(
            trace_contours(&img, f32::NAN, 1, 100),
            Err(CoreError::NonFinite { .. })
        ));
        assert!(matches!(
            trace_contours(&img, f32::INFINITY, 1, 100),
            Err(CoreError::NonFinite { .. })
        ));
    }

    #[test]
    fn empty_image_has_no_contours() {
        let img = GrayImage::new_filled(16, 9, 0.0).unwrap();
        assert!(trace_contours(&img, 0.5, 1, 100).unwrap().is_empty());
    }

    #[test]
    fn full_image_gives_border_rectangle() {
        let img = GrayImage::new_filled(16, 9, 1.0).unwrap();
        let cs = trace_contours(&img, 0.5, 1, 100).unwrap();
        assert_eq!(cs.len(), 1);
        let pts = &cs[0].points;
        assert_eq!(pts.len(), 4, "{pts:?}");
        for corner in [[0.0, 0.0], [16.0, 0.0], [16.0, 9.0], [0.0, 9.0]] {
            assert!(pts.contains(&corner), "missing {corner:?} in {pts:?}");
        }
        assert!(cs[0].signed_area() < 0.0);
        assert!((cs[0].signed_area() + 144.0).abs() < 1e-9);
    }

    #[test]
    fn one_by_one_image() {
        let img = GrayImage::new_filled(1, 1, 1.0).unwrap();
        let cs = trace_contours(&img, 0.5, 1, 100).unwrap();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].points.len(), 4);
        assert!((cs[0].signed_area() + 1.0).abs() < 1e-9);
    }

    #[test]
    fn single_pixel_gives_diamond() {
        let img = rect_mask(8, 8, 3, 4, 4, 5);
        let cs = trace_contours(&img, 0.5, 1, 100).unwrap();
        assert_eq!(cs.len(), 1);
        let pts = &cs[0].points;
        assert_eq!(pts.len(), 4);
        for p in [[3.5, 4.0], [4.0, 4.5], [3.5, 5.0], [3.0, 4.5]] {
            assert!(pts.contains(&p), "missing {p:?} in {pts:?}");
        }
        assert!(cs[0].signed_area() < 0.0);
        assert_closed_form(&cs);
    }

    #[test]
    fn rectangle_has_eight_points() {
        let img = rect_mask(40, 30, 10, 5, 30, 20);
        let cs = trace_contours(&img, 0.5, 1, 100).unwrap();
        assert_eq!(cs.len(), 1);
        // Four sides plus four half-pixel corner cuts.
        assert_eq!(cs[0].points.len(), 8, "{:?}", cs[0].points);
        assert_eq!(bounds(&cs[0]), [10.0, 5.0, 30.0, 20.0]);
    }

    #[test]
    fn rect_with_hole_gives_opposite_orientations() {
        let outer = rect_mask(40, 40, 5, 5, 35, 35);
        let img = image(40, 40, |x, y| {
            let hole = (15..25).contains(&x) && (15..25).contains(&y);
            if hole { 0.0 } else { outer.get(x, y).unwrap() }
        });
        let cs = trace_contours(&img, 0.5, 1, 100).unwrap();
        assert_eq!(cs.len(), 2);
        let areas: Vec<f64> = cs.iter().map(Contour::signed_area).collect();
        assert!(areas[0] * areas[1] < 0.0, "{areas:?}");
        let (outer_c, hole_c) = if areas[0] < 0.0 {
            (&cs[0], &cs[1])
        } else {
            (&cs[1], &cs[0])
        };
        assert_eq!(bounds(outer_c), [5.0, 5.0, 35.0, 35.0]);
        assert_eq!(bounds(hole_c), [15.0, 15.0, 25.0, 25.0]);
        assert_no_crossings(&cs);
        assert_closed_form(&cs);
    }

    /// Anti-aliased ellipse coverage with 16 x 16 supersampling.
    fn ellipse(w: u32, h: u32, cx: f32, cy: f32, a: f32, b: f32) -> GrayImage {
        const N: u32 = 16;
        image(w, h, |x, y| {
            let mut hits = 0u32;
            for sy in 0..N {
                for sx in 0..N {
                    let px = x as f32 + (sx as f32 + 0.5) / N as f32 - cx;
                    let py = y as f32 + (sy as f32 + 0.5) / N as f32 - cy;
                    if (px / a).powi(2) + (py / b).powi(2) <= 1.0 {
                        hits += 1;
                    }
                }
            }
            hits as f32 / (N * N) as f32
        })
    }

    #[test]
    fn ellipse_points_are_near_the_curve() {
        let (cx, cy, a, b) = (32.3f32, 25.7f32, 21.0f32, 13.5f32);
        let img = ellipse(64, 52, cx, cy, a, b);
        let cs = trace_contours(&img, 0.5, 1, 10_000).unwrap();
        assert_eq!(cs.len(), 1);
        assert!(cs[0].signed_area() < 0.0);
        assert!(cs[0].points.len() > 50);
        for p in &cs[0].points {
            let (dx, dy) = (f64::from(p[0] - cx), f64::from(p[1] - cy));
            let (a, b) = (f64::from(a), f64::from(b));
            // First-order distance to the curve: |F| / |grad F|.
            let f = (dx / a).powi(2) + (dy / b).powi(2) - 1.0;
            let g = ((2.0 * dx / (a * a)).powi(2) + (2.0 * dy / (b * b)).powi(2)).sqrt();
            let dist = f.abs() / g;
            assert!(dist < 0.5, "point {p:?} is {dist} px from the curve");
        }
        assert_no_crossings(&cs);
    }

    #[test]
    fn saddle_contours_do_not_cross() {
        // Checkerboard 2 x 2 in a larger image, both saddle rules.
        for (hi, lo) in [(1.0, 0.0), (0.6, 0.0)] {
            let img = image(6, 6, |x, y| match (x, y) {
                (2, 2) | (3, 3) => hi,
                _ => lo,
            });
            let cs = trace_contours(&img, 0.5, 1, 100).unwrap();
            assert!(!cs.is_empty());
            assert_no_crossings(&cs);
            assert_closed_form(&cs);
            for c in &cs {
                assert!(c.signed_area() < 0.0);
            }
            // Mean 0.5 >= 0.5 connects; mean 0.3 < 0.5 separates.
            let expected = if hi == 1.0 { 1 } else { 2 };
            assert_eq!(cs.len(), expected, "hi = {hi}");
        }
        // Inverse checkerboard: holes inside a full image.
        let img = image(6, 6, |x, y| match (x, y) {
            (2, 2) | (3, 3) => 0.0,
            _ => 1.0,
        });
        let cs = trace_contours(&img, 0.5, 1, 100).unwrap();
        assert_no_crossings(&cs);
        assert_closed_form(&cs);
    }

    #[test]
    fn coarse_steps_on_a_large_rect() {
        let img = rect_mask(400, 300, 101, 53, 299, 201);
        for step in [2u32, 4] {
            let cs = trace_contours(&img, 0.5, step, 1000).unwrap();
            assert_eq!(cs.len(), 1, "step {step}");
            let c = &cs[0];
            assert_eq!(c.points.len(), 8, "step {step}: {:?}", c.points);
            let b = bounds(c);
            let s = step as f32;
            let want = [101.0, 53.0, 299.0, 201.0];
            for k in 0..4 {
                assert!((b[k] - want[k]).abs() <= s, "step {step}: {b:?}");
            }
            assert!(c.signed_area() < 0.0);
        }
    }

    #[test]
    fn coarse_step_keeps_thin_lines() {
        // A 1 px vertical line must not vanish at step 4.
        let img = rect_mask(64, 64, 33, 8, 34, 56);
        let cs = trace_contours(&img, 0.5, 4, 1000).unwrap();
        assert_eq!(cs.len(), 1);
    }

    #[test]
    fn coarse_step_with_partial_blocks() {
        // 10 is not a multiple of 4: the last block is 2 px wide.
        let img = GrayImage::new_filled(10, 7, 1.0).unwrap();
        let cs = trace_contours(&img, 0.5, 4, 100).unwrap();
        assert_eq!(cs.len(), 1);
        assert_eq!(bounds(&cs[0]), [0.0, 0.0, 10.0, 7.0]);
        assert_eq!(cs[0].points.len(), 4);
    }

    #[test]
    fn max_points_limit() {
        // 10 x 10 separate pixels, not on the image border: 100 diamonds
        // of 4 points.
        let img = image(
            21,
            21,
            |x, y| if x % 2 == 1 && y % 2 == 1 { 1.0 } else { 0.0 },
        );
        assert_eq!(trace_contours(&img, 0.5, 1, 400).unwrap().len(), 100);
        assert_eq!(
            trace_contours(&img, 0.5, 1, 399),
            Err(CoreError::ContourTooLarge { limit: 399 })
        );
    }

    #[test]
    fn nan_counts_as_zero() {
        let img = image(4, 4, |x, _| if x == 0 { f32::NAN } else { 1.0 });
        let cs = trace_contours(&img, 0.5, 1, 100).unwrap();
        assert_eq!(cs.len(), 1);
        for p in &cs[0].points {
            assert!(p[0].is_finite() && p[1].is_finite());
            assert!(p[0] >= 0.5);
        }
    }

    #[test]
    fn output_is_deterministic() {
        let img = ellipse(48, 40, 20.0, 18.0, 15.0, 9.0);
        let a = trace_contours(&img, 0.5, 1, 10_000).unwrap();
        let b = trace_contours(&img, 0.5, 1, 10_000).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    #[ignore = "benchmark; run in release with --ignored --nocapture"]
    fn bench_trace_large_ellipse() {
        let n = 4096u32;
        let (c, a, b) = (2048.0f32, 1800.0f32, 1200.0f32);
        // A 1 px soft edge from the first-order distance, enough for timing.
        let img = image(n, n, |x, y| {
            let dx = x as f32 + 0.5 - c;
            let dy = y as f32 + 0.5 - c;
            let f = (dx / a).powi(2) + (dy / b).powi(2) - 1.0;
            let g = ((2.0 * dx / (a * a)).powi(2) + (2.0 * dy / (b * b)).powi(2)).sqrt();
            (0.5 - f / g.max(1e-12)).clamp(0.0, 1.0)
        });
        let mut best = f64::MAX;
        let mut points = 0;
        for _ in 0..5 {
            let t0 = std::time::Instant::now();
            let cs = trace_contours(&img, 0.5, 1, usize::MAX).unwrap();
            best = best.min(t0.elapsed().as_secs_f64() * 1e3);
            points = cs.iter().map(|c| c.points.len()).sum();
        }
        println!("trace_contours 4096^2 ellipse step 1: {best:.2} ms, {points} points");
    }
}
