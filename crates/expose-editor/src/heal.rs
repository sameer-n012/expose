//! The membrane solver of the healing brush ([`crate::brush`]).
//!
//! # Problem
//!
//! The input is a grid of cells. Each cell is one of:
//!
//! - [`Cell::Unknown`]: the solver finds the membrane value `M` here.
//! - [`Cell::Fixed`]: the membrane value is known (a boundary value).
//! - [`Cell::Free`]: the cell is not part of the problem. An unknown cell
//!   next to a free cell (or next to the grid edge) has a zero gradient
//!   toward it (Neumann boundary).
//!
//! The solver finds `M` with `ΔM = 0` on the unknown cells (5-point
//! Laplacian, 4-neighbourhood). Each of the four channels is a separate
//! problem with the same matrix.
//!
//! A connected set of unknown cells with no fixed neighbour has no unique
//! solution. The solver gives `M = 0` for these cells.
//!
//! # Method
//!
//! Conjugate gradients, preconditioned with one multigrid V-cycle
//! (aggregation of 2 × 2 blocks, Galerkin coarse matrices, symmetric
//! Gauss-Seidel smoothing, a dense Cholesky solve on the coarsest level).
//! All loops run in a fixed order on one thread, so the result is
//! deterministic.

/// One cell of the problem.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Cell {
    /// The solver finds the value of this cell.
    Unknown,
    /// A known boundary value.
    Fixed([f32; 4]),
    /// Not part of the problem (zero gradient toward this cell).
    Free,
}

/// No neighbour.
const NONE: u32 = u32::MAX;
/// The coarsest level has this number of cells or fewer.
const COARSE_DIRECT: usize = 256;
/// The maximum number of levels.
const MAX_LEVELS: usize = 24;
/// The maximum number of conjugate gradient iterations.
const MAX_ITERATIONS: usize = 200;
/// The solver stops when the residual norm of each channel is at or below
/// this fraction of the norm of the right side. On a 1 MP heal test, the
/// largest pixel difference to a solve with 1e-7 is about 4e-4 (an 8-bit
/// step is 3.9e-3), with 8 iterations instead of 20.
const TOLERANCE: f64 = 1.0e-4;
/// The scale of the coarse correction. A piecewise constant interpolation
/// gives a correction that is too small; a scale in `1..2` makes the
/// V-cycle stronger (about half the iterations at 1.6) and keeps it
/// symmetric positive definite (a scale below 2 keeps the coarse step a
/// contraction in the energy norm).
const OVER_CORRECTION: f32 = 1.6;
/// Symmetric Gauss-Seidel sweeps on the coarsest level when the dense solve
/// is not used (the coarsening stopped above [`COARSE_DIRECT`] cells).
const COARSE_SWEEPS: usize = 20;

type V4 = [f32; 4];

#[inline]
fn add4(a: V4, b: V4) -> V4 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

#[inline]
fn sub4(a: V4, b: V4) -> V4 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]]
}

#[inline]
fn mad4(a: V4, s: f32, b: V4) -> V4 {
    [
        a[0] + s * b[0],
        a[1] + s * b[1],
        a[2] + s * b[2],
        a[3] + s * b[3],
    ]
}

/// One level of the multigrid hierarchy. Cell `i` has the matrix row
/// `A[i][i] = diag[i]`, `A[i][nbr[i][k]] = -w[i][k]`.
///
/// The cells are in red-black order: the cells with an even `x + y` come
/// first (`0..red`). Two cells of the same color are not neighbours, so a
/// Gauss-Seidel sweep over one color has no dependency between cells.
struct Level {
    /// Cell positions on the grid of this level.
    coords: Vec<(u32, u32)>,
    /// The matrix rows (one memory stream for the loops).
    rows: Vec<Row>,
    /// The number of red cells.
    red: usize,
    /// The cell on the next coarser level (empty on the coarsest level).
    parent: Vec<u32>,
}

/// One matrix row.
#[derive(Debug, Clone, Copy)]
struct Row {
    /// Neighbours: left, right, up, down. If there is no unknown neighbour
    /// in a direction, the entry is the cell itself with the weight 0 (no
    /// branch in the loops).
    nb: [u32; 4],
    /// Coupling weights (positive) for `nb`.
    w: [f32; 4],
    diag: f32,
    inv_diag: f32,
}

impl Level {
    /// Makes a level from cells in any order. `nbr` uses [`NONE`] for no
    /// neighbour. Returns the level and the new index of each input cell.
    #[allow(clippy::cast_possible_truncation)] // Cell counts fit in u32.
    fn new(
        coords: &[(u32, u32)],
        diag: &[f32],
        nbr: &[[u32; 4]],
        w: &[[f32; 4]],
    ) -> (Self, Vec<u32>) {
        let n = coords.len();
        let even = |i: &usize| (coords[*i].0 + coords[*i].1) % 2 == 0;
        let mut order: Vec<usize> = (0..n).filter(even).collect();
        let red = order.len();
        order.extend((0..n).filter(|i| !even(i)));
        let mut new_index = vec![0u32; n];
        for (new, &old) in order.iter().enumerate() {
            new_index[old] = new as u32;
        }
        let mut level = Self {
            coords: Vec::with_capacity(n),
            rows: Vec::with_capacity(n),
            red,
            parent: Vec::new(),
        };
        for (new, &old) in order.iter().enumerate() {
            level.coords.push(coords[old]);
            let mut row = Row {
                nb: [new as u32; 4],
                w: [0.0; 4],
                diag: diag[old],
                // The diagonal is positive (the matrix is positive
                // definite).
                inv_diag: if diag[old] > 0.0 {
                    1.0 / diag[old]
                } else {
                    0.0
                },
            };
            for k in 0..4 {
                if nbr[old][k] != NONE {
                    row.nb[k] = new_index[nbr[old][k] as usize];
                    row.w[k] = w[old][k];
                }
            }
            level.rows.push(row);
        }
        (level, new_index)
    }

    fn len(&self) -> usize {
        self.rows.len()
    }

    /// Row `i` of `A x`.
    #[inline]
    fn row(&self, i: usize, x: &[V4]) -> V4 {
        let row = &self.rows[i];
        let mut acc = scale4(x[i], row.diag);
        for k in 0..4 {
            acc = mad4(acc, -row.w[k], x[row.nb[k] as usize]);
        }
        acc
    }

    /// `y = A x`. Returns the dot product of `x` and `y` per channel.
    fn apply_dot(&self, x: &[V4], y: &mut [V4]) -> [f64; 4] {
        for (i, yi) in y.iter_mut().enumerate() {
            *yi = self.row(i, x);
        }
        dot4(x, y)
    }

    /// Adds the residual `b - A u` of each cell to its parent cell in
    /// `coarse_b`, which the function first sets to 0.
    fn restrict_residual(&self, b: &[V4], u: &[V4], coarse_b: &mut [V4]) {
        coarse_b.fill([0.0; 4]);
        for (i, &p) in self.parent.iter().enumerate() {
            let r = sub4(b[i], self.row(i, u));
            coarse_b[p as usize] = add4(coarse_b[p as usize], r);
        }
    }

    /// Gauss-Seidel updates of the cells `range`.
    #[inline]
    fn relax(&self, range: std::ops::Range<usize>, b: &[V4], u: &mut [V4]) {
        let start = range.start;
        for (i, (row, bi)) in self.rows[range.clone()].iter().zip(&b[range]).enumerate() {
            let mut acc = *bi;
            for k in 0..4 {
                acc = mad4(acc, row.w[k], u[row.nb[k] as usize]);
            }
            u[start + i] = scale4(acc, row.inv_diag);
        }
    }

    /// A red-black Gauss-Seidel sweep: red, then black.
    fn forward(&self, b: &[V4], u: &mut [V4]) {
        self.relax(0..self.red, b, u);
        self.relax(self.red..self.len(), b, u);
    }

    /// The adjoint sweep of [`Self::forward`]: black, then red.
    fn backward(&self, b: &[V4], u: &mut [V4]) {
        self.relax(self.red..self.len(), b, u);
        self.relax(0..self.red, b, u);
    }

    /// Makes the next coarser level from 2 × 2 blocks and sets
    /// `self.parent`.
    fn coarsen(&mut self) -> Level {
        let (mut gw, mut gh) = (0u32, 0u32);
        for &(x, y) in &self.coords {
            gw = gw.max(x / 2 + 1);
            gh = gh.max(y / 2 + 1);
        }
        let mut lookup = vec![NONE; gw as usize * gh as usize];
        let mut coords = Vec::new();
        self.parent = Vec::with_capacity(self.len());
        for &(x, y) in &self.coords {
            let (cx, cy) = (x / 2, y / 2);
            let slot = &mut lookup[cy as usize * gw as usize + cx as usize];
            if *slot == NONE {
                // The cell count is below the grid size, which fits in u32.
                *slot = u32::try_from(coords.len()).unwrap_or(NONE);
                coords.push((cx, cy));
            }
            self.parent.push(*slot);
        }
        let n = coords.len();
        let mut diag = vec![0.0f32; n];
        let mut nbr = vec![[NONE; 4]; n];
        let mut w = vec![[0.0f32; 4]; n];
        for (i, row) in self.rows.iter().enumerate() {
            let p = self.parent[i] as usize;
            diag[p] += row.diag;
            for k in 0..4 {
                let j = row.nb[k];
                if j as usize == i {
                    // No neighbour in this direction.
                    continue;
                }
                let q = self.parent[j as usize];
                if q as usize == p {
                    // An edge inside the block: the row sum of the Galerkin
                    // matrix loses `w` from each side.
                    diag[p] -= row.w[k];
                } else {
                    // The blocks are neighbours in direction `k` (a fine
                    // edge in direction `k` crosses to the block in the
                    // same direction).
                    nbr[p][k] = q;
                    w[p][k] += row.w[k];
                }
            }
        }
        let (level, new_index) = Level::new(&coords, &diag, &nbr, &w);
        for p in &mut self.parent {
            *p = new_index[*p as usize];
        }
        level
    }
}

#[inline]
fn scale4(a: V4, s: f32) -> V4 {
    [a[0] * s, a[1] * s, a[2] * s, a[3] * s]
}

/// The dense Cholesky factor of the coarsest level.
struct Dense {
    n: usize,
    /// Lower triangle, row-major `n × n`.
    l: Vec<f64>,
}

impl Dense {
    /// Factors the matrix of `level`. Returns `None` if a pivot is not
    /// positive. The matrix is symmetric positive definite (a Galerkin
    /// product of the fine matrix, which is a nonsingular M-matrix because
    /// each component has a fixed neighbour), so this does not occur in
    /// practice; the caller then uses sweeps.
    fn factor(level: &Level) -> Option<Self> {
        let n = level.len();
        let mut a = vec![0.0f64; n * n];
        for (i, row) in level.rows.iter().enumerate() {
            a[i * n + i] += f64::from(row.diag);
            for k in 0..4 {
                // A missing neighbour points to the cell itself with the
                // weight 0, so it adds nothing.
                a[i * n + row.nb[k] as usize] -= f64::from(row.w[k]);
            }
        }
        for j in 0..n {
            let mut d = a[j * n + j];
            for k in 0..j {
                d -= a[j * n + k] * a[j * n + k];
            }
            if d <= 0.0 || !d.is_finite() {
                return None;
            }
            let d = d.sqrt();
            a[j * n + j] = d;
            for i in j + 1..n {
                let mut s = a[i * n + j];
                for k in 0..j {
                    s -= a[i * n + k] * a[j * n + k];
                }
                a[i * n + j] = s / d;
            }
        }
        Some(Self { n, l: a })
    }

    // f64 to f32 of pixel values. The index loops follow the triangular
    // solve formulas.
    #[allow(clippy::cast_possible_truncation, clippy::needless_range_loop)]
    fn solve(&self, b: &[V4], u: &mut [V4]) {
        let n = self.n;
        for c in 0..4 {
            let mut y: Vec<f64> = b.iter().map(|v| f64::from(v[c])).collect();
            for i in 0..n {
                let mut s = y[i];
                for k in 0..i {
                    s -= self.l[i * n + k] * y[k];
                }
                y[i] = s / self.l[i * n + i];
            }
            for i in (0..n).rev() {
                let mut s = y[i];
                for k in i + 1..n {
                    s -= self.l[k * n + i] * y[k];
                }
                y[i] = s / self.l[i * n + i];
            }
            for (ui, yi) in u.iter_mut().zip(&y) {
                ui[c] = *yi as f32;
            }
        }
    }
}

/// The multigrid hierarchy.
struct Hierarchy {
    levels: Vec<Level>,
    dense: Option<Dense>,
}

impl Hierarchy {
    fn new(fine: Level) -> Self {
        let mut levels = vec![fine];
        while levels.len() < MAX_LEVELS {
            let Some(last) = levels.last_mut() else { break };
            let n = last.len();
            if n <= COARSE_DIRECT {
                break;
            }
            let coarse = last.coarsen();
            // Stop if the coarsening does not make the level smaller
            // enough (for example, many single cells far apart).
            if coarse.len() * 10 > n * 9 {
                last.parent.clear();
                break;
            }
            levels.push(coarse);
        }
        let dense = levels
            .last()
            .filter(|l| l.len() <= COARSE_DIRECT)
            .and_then(Dense::factor);
        Self { levels, dense }
    }

    /// One V-cycle from `u = 0` for `A u = b` on level `l`. This is a
    /// symmetric linear operator of `b`, so it is a valid preconditioner
    /// for conjugate gradients.
    /// Makes the work vectors of the coarse levels.
    fn scratch(&self) -> Vec<Scratch> {
        self.levels[1..]
            .iter()
            .map(|l| Scratch {
                b: vec![[0.0; 4]; l.len()],
                u: vec![[0.0; 4]; l.len()],
            })
            .collect()
    }

    /// One V-cycle from `u = 0` for `A u = b` on level `l`. This is a
    /// symmetric positive definite linear operator of `b`, so it is a
    /// valid preconditioner for conjugate gradients. `scratch` holds the
    /// work vectors of the levels below `l`.
    fn vcycle(&self, l: usize, b: &[V4], u: &mut [V4], scratch: &mut [Scratch]) {
        let level = &self.levels[l];
        u.fill([0.0; 4]);
        let Some((next, rest)) = scratch.split_first_mut() else {
            // The coarsest level.
            match &self.dense {
                Some(d) => d.solve(b, u),
                None => {
                    for _ in 0..COARSE_SWEEPS {
                        level.forward(b, u);
                        level.backward(b, u);
                    }
                }
            }
            return;
        };
        level.forward(b, u);
        level.restrict_residual(b, u, &mut next.b);
        self.vcycle(l + 1, &next.b, &mut next.u, rest);
        for (ui, &p) in u.iter_mut().zip(&level.parent) {
            *ui = mad4(*ui, OVER_CORRECTION, next.u[p as usize]);
        }
        level.backward(b, u);
    }
}

/// The work vectors of one coarse level.
struct Scratch {
    b: Vec<V4>,
    u: Vec<V4>,
}

/// The dot product per channel. Sums of 256 cells are in `f32` (fast);
/// the total is in `f64`. The order is fixed, so the result is
/// deterministic.
fn dot4(a: &[V4], b: &[V4]) -> [f64; 4] {
    let mut s = [0.0f64; 4];
    for (ca, cb) in a.chunks(256).zip(b.chunks(256)) {
        let mut t = [0.0f32; 4];
        for (x, y) in ca.iter().zip(cb) {
            for c in 0..4 {
                t[c] += x[c] * y[c];
            }
        }
        for c in 0..4 {
            s[c] += f64::from(t[c]);
        }
    }
    s
}

/// Solves the membrane problem (see the [module documentation](self)).
/// `cells` is row-major with `width * height` entries. Returns the
/// membrane value of each cell: the solution on unknown cells, the value
/// on fixed cells, and 0 on free cells.
#[allow(clippy::cast_possible_truncation)] // Grid sizes fit in u32; f64 steps to f32.
pub(crate) fn solve_membrane(width: usize, height: usize, cells: &[Cell]) -> Vec<V4> {
    let mut out: Vec<V4> = cells
        .iter()
        .map(|c| match c {
            Cell::Fixed(v) => *v,
            _ => [0.0; 4],
        })
        .collect();
    // The only caller (the heal pass) makes `cells` with this size.
    debug_assert_eq!(cells.len(), width * height);
    if cells.len() != width * height {
        return out;
    }
    let anchored = anchored_unknowns(width, height, cells);
    // Compact indices of the anchored unknown cells, row-major.
    let mut index = vec![NONE; cells.len()];
    let mut coords = Vec::new();
    for (p, &a) in anchored.iter().enumerate() {
        if a {
            index[p] = coords.len() as u32;
            coords.push(((p % width) as u32, (p / width) as u32));
        }
    }
    let n = coords.len();
    if n == 0 {
        return out;
    }
    let mut diag = vec![0.0f32; n];
    let mut nbr = vec![[NONE; 4]; n];
    let w = vec![[1.0f32; 4]; n];
    let mut b = vec![[0.0f32; 4]; n];
    for (i, &(x, y)) in coords.iter().enumerate() {
        let (x, y) = (x as usize, y as usize);
        let around = [
            (x > 0).then(|| y * width + x - 1),
            (x + 1 < width).then(|| y * width + x + 1),
            (y > 0).then(|| (y - 1) * width + x),
            (y + 1 < height).then(|| (y + 1) * width + x),
        ];
        for (k, q) in around.into_iter().enumerate() {
            let Some(q) = q else { continue };
            match cells[q] {
                Cell::Unknown if index[q] != NONE => {
                    diag[i] += 1.0;
                    nbr[i][k] = index[q];
                }
                Cell::Fixed(v) => {
                    diag[i] += 1.0;
                    b[i] = add4(b[i], v);
                }
                // An unknown cell that is not anchored is in another
                // component, so it is not a neighbour here.
                _ => {}
            }
        }
    }
    let (fine, new_index) = Level::new(&coords, &diag, &nbr, &w);
    let mut rhs = vec![[0.0f32; 4]; n];
    for (i, v) in b.into_iter().enumerate() {
        rhs[new_index[i] as usize] = v;
    }
    let h = Hierarchy::new(fine);
    let x = pcg(&h, &rhs);
    for (p, &i) in index.iter().enumerate() {
        if i != NONE {
            out[p] = x[new_index[i as usize] as usize];
        }
    }
    out
}

/// Preconditioned conjugate gradients for the four channels at the same
/// time (separate step sizes per channel).
// f64 step sizes to f32. The channel index loops read four arrays.
#[allow(clippy::cast_possible_truncation, clippy::needless_range_loop)]
fn pcg(h: &Hierarchy, b: &[V4]) -> Vec<V4> {
    let fine = &h.levels[0];
    let n = fine.len();
    let mut x = vec![[0.0f32; 4]; n];
    let mut r = b.to_vec();
    let mut z = vec![[0.0f32; 4]; n];
    let mut q = vec![[0.0f32; 4]; n];
    let bn = dot4(b, b);
    let limit: [f64; 4] = bn.map(|v| (v * TOLERANCE * TOLERANCE).max(1.0e-24));
    let done = |rr: [f64; 4]| (0..4).all(|c| rr[c] <= limit[c]);
    if done(bn) {
        return x;
    }
    let mut scratch = h.scratch();
    h.vcycle(0, &r, &mut z, &mut scratch);
    let mut p = z.clone();
    let mut rz = dot4(&r, &z);
    for _ in 0..MAX_ITERATIONS {
        let pq = fine.apply_dot(&p, &mut q);
        let alpha: [f32; 4] = std::array::from_fn(|c| {
            if pq[c] > 0.0 {
                (rz[c] / pq[c]) as f32
            } else {
                0.0
            }
        });
        for i in 0..n {
            for c in 0..4 {
                x[i][c] += alpha[c] * p[i][c];
                r[i][c] -= alpha[c] * q[i][c];
            }
        }
        if done(dot4(&r, &r)) {
            break;
        }
        h.vcycle(0, &r, &mut z, &mut scratch);
        let rz_new = dot4(&r, &z);
        let beta: [f32; 4] = std::array::from_fn(|c| {
            if rz[c] > 0.0 {
                (rz_new[c] / rz[c]) as f32
            } else {
                0.0
            }
        });
        for i in 0..n {
            for c in 0..4 {
                p[i][c] = z[i][c] + beta[c] * p[i][c];
            }
        }
        rz = rz_new;
    }
    x
}

/// Marks the unknown cells that are in a connected set (4-neighbourhood)
/// with at least one fixed neighbour.
fn anchored_unknowns(width: usize, height: usize, cells: &[Cell]) -> Vec<bool> {
    let mut seen = vec![false; cells.len()];
    let mut anchored = vec![false; cells.len()];
    let mut stack = Vec::new();
    let mut members = Vec::new();
    for start in 0..cells.len() {
        if seen[start] || cells[start] != Cell::Unknown {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        members.clear();
        let mut has_fixed = false;
        while let Some(p) = stack.pop() {
            members.push(p);
            let (x, y) = (p % width, p / width);
            let around = [
                (x > 0).then(|| p - 1),
                (x + 1 < width).then(|| p + 1),
                (y > 0).then(|| p - width),
                (y + 1 < height).then(|| p + width),
            ];
            for q in around.into_iter().flatten() {
                match cells[q] {
                    Cell::Unknown if !seen[q] => {
                        seen[q] = true;
                        stack.push(q);
                    }
                    Cell::Fixed(_) => has_fixed = true,
                    _ => {}
                }
            }
        }
        if has_fixed {
            for &m in &members {
                anchored[m] = true;
            }
        }
    }
    anchored
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::cast_precision_loss)]

    use super::*;

    fn disc(w: usize, h: usize, r: f32, boundary: impl Fn(usize, usize) -> V4) -> Vec<Cell> {
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        (0..w * h)
            .map(|p| {
                let (x, y) = (p % w, p / w);
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                if d < r {
                    Cell::Unknown
                } else {
                    Cell::Fixed(boundary(x, y))
                }
            })
            .collect()
    }

    #[test]
    fn constant_boundary_gives_constant() {
        let cells = disc(64, 64, 25.0, |_, _| [0.5, -0.2, 1.0, 0.0]);
        let m = solve_membrane(64, 64, &cells);
        for v in &m {
            for (a, b) in v.iter().zip([0.5, -0.2, 1.0, 0.0]) {
                assert!((a - b).abs() < 1e-3, "{a} vs {b}");
            }
        }
    }

    #[test]
    fn linear_boundary_gives_linear() {
        // A linear function is harmonic, also for the discrete Laplacian.
        let f = |x: usize, y: usize| 0.01 * x as f32 - 0.02 * y as f32;
        let cells = disc(80, 60, 25.0, |x, y| [f(x, y); 4]);
        let m = solve_membrane(80, 60, &cells);
        for (p, v) in m.iter().enumerate() {
            let e = f(p % 80, p / 80);
            assert!((v[0] - e).abs() < 1e-3, "{} vs {e}", v[0]);
        }
    }

    #[test]
    fn neumann_side_and_free_component() {
        // Left column fixed at 1, the rest unknown with free edges: the
        // solution is 1 everywhere. A separate unknown cell with only free
        // neighbours gets 0.
        let (w, h) = (40, 30);
        let mut cells: Vec<Cell> = (0..w * h)
            .map(|p| {
                if p % w == 0 {
                    Cell::Fixed([1.0; 4])
                } else {
                    Cell::Unknown
                }
            })
            .collect();
        // Cut a free row, so that the bottom rows have no fixed cell.
        for x in 0..w {
            cells[20 * w + x] = Cell::Free;
        }
        for y in 21..h {
            cells[y * w] = Cell::Unknown;
        }
        let m = solve_membrane(w, h, &cells);
        for y in 0..20 {
            for x in 0..w {
                assert!((m[y * w + x][2] - 1.0).abs() < 1e-3);
            }
        }
        for y in 21..h {
            for x in 0..w {
                assert_eq!(m[y * w + x], [0.0; 4]);
            }
        }
    }

    #[test]
    fn is_deterministic() {
        let cells = disc(70, 50, 22.0, |x, y| [(x * y % 7) as f32 * 0.1; 4]);
        let a = solve_membrane(70, 50, &cells);
        let b = solve_membrane(70, 50, &cells);
        let bits = |v: &[V4]| -> Vec<u32> { v.iter().flatten().map(|f| f.to_bits()).collect() };
        assert_eq!(bits(&a), bits(&b));
    }
}
