//! 3x3 matrix type for color math.

use std::ops::Mul;

use crate::error::{CoreError, CoreResult};

/// Determinants with an absolute value below this limit are singular.
const SINGULAR_LIMIT: f64 = 1e-12;

/// A 3x3 matrix in row-major order, with `f64` elements.
///
/// Color matrices use `f64` for the derivation, to keep precision. Use
/// [`Mat3::to_f32`] to get `f32` values for per-pixel work and GPU uniforms.
///
/// The matrix multiplies column vectors: `out = M * [r, g, b]`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct Mat3 {
    /// Rows of the matrix. `rows[r][c]` is row `r`, column `c`.
    pub rows: [[f64; 3]; 3],
}

impl Mat3 {
    /// The identity matrix.
    pub const IDENTITY: Self = Self {
        rows: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };

    /// Makes a matrix from rows.
    pub const fn new(rows: [[f64; 3]; 3]) -> Self {
        Self { rows }
    }

    /// Makes a diagonal matrix.
    pub const fn diagonal(d: [f64; 3]) -> Self {
        Self {
            rows: [[d[0], 0.0, 0.0], [0.0, d[1], 0.0], [0.0, 0.0, d[2]]],
        }
    }

    /// Makes a matrix whose columns are `c0`, `c1`, and `c2`.
    pub const fn from_columns(c0: [f64; 3], c1: [f64; 3], c2: [f64; 3]) -> Self {
        Self {
            rows: [
                [c0[0], c1[0], c2[0]],
                [c0[1], c1[1], c2[1]],
                [c0[2], c1[2], c2[2]],
            ],
        }
    }

    /// Multiplies this matrix by the column vector `v`.
    pub fn mul_vec(&self, v: [f64; 3]) -> [f64; 3] {
        let m = &self.rows;
        [
            m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
            m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
            m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
        ]
    }

    /// Returns the transpose.
    pub fn transpose(&self) -> Self {
        let m = &self.rows;
        Self::new([
            [m[0][0], m[1][0], m[2][0]],
            [m[0][1], m[1][1], m[2][1]],
            [m[0][2], m[1][2], m[2][2]],
        ])
    }

    /// Returns the determinant.
    pub fn determinant(&self) -> f64 {
        let m = &self.rows;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// Returns the inverse.
    ///
    /// Returns [`CoreError::SingularMatrix`] if the determinant is not
    /// finite or its absolute value is less than `1e-12`.
    pub fn inverse(&self) -> CoreResult<Self> {
        let det = self.determinant();
        if !det.is_finite() || det.abs() < SINGULAR_LIMIT {
            return Err(CoreError::SingularMatrix);
        }
        let m = &self.rows;
        // Adjugate (transpose of the cofactor matrix) divided by the determinant.
        let cof = |r0: usize, r1: usize, c0: usize, c1: usize| {
            m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
        };
        let inv_det = 1.0 / det;
        Ok(Self::new([
            [
                cof(1, 2, 1, 2) * inv_det,
                -cof(0, 2, 1, 2) * inv_det,
                cof(0, 1, 1, 2) * inv_det,
            ],
            [
                -cof(1, 2, 0, 2) * inv_det,
                cof(0, 2, 0, 2) * inv_det,
                -cof(0, 1, 0, 2) * inv_det,
            ],
            [
                cof(1, 2, 0, 1) * inv_det,
                -cof(0, 2, 0, 1) * inv_det,
                cof(0, 1, 0, 1) * inv_det,
            ],
        ]))
    }

    /// Returns the rows as `f32` values.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "f64 to f32 rounds to the nearest f32; color matrices need no more precision per pixel"
    )]
    pub fn to_f32(&self) -> [[f32; 3]; 3] {
        self.rows.map(|row| row.map(|v| v as f32))
    }
}

impl Default for Mat3 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mul for Mat3 {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self {
        let a = &self.rows;
        let b = &rhs.rows;
        let mut out = [[0.0; 3]; 3];
        for (r, out_row) in out.iter_mut().enumerate() {
            for (c, out_val) in out_row.iter_mut().enumerate() {
                *out_val = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
            }
        }
        Self::new(out)
    }
}

impl Mul<[f64; 3]> for Mat3 {
    type Output = [f64; 3];

    fn mul(self, rhs: [f64; 3]) -> [f64; 3] {
        self.mul_vec(rhs)
    }
}

/// Multiplies an `f32` row-major matrix by the column vector `v`.
///
/// Use this with the result of [`Mat3::to_f32`] in per-pixel loops.
#[inline]
pub fn mul_vec_f32(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

#[cfg(test)]
pub(crate) fn assert_mat_close(a: &Mat3, b: &Mat3, tol: f64) {
    for r in 0..3 {
        for c in 0..3 {
            let d = (a.rows[r][c] - b.rows[r][c]).abs();
            assert!(d <= tol, "element [{r}][{c}]: {a:?} vs {b:?} (diff {d})");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Mat3 = Mat3::new([[2.0, 1.0, 0.5], [0.0, 3.0, 1.0], [1.0, 0.0, 4.0]]);

    #[test]
    fn identity_is_neutral() {
        assert_mat_close(&(A * Mat3::IDENTITY), &A, 0.0);
        assert_mat_close(&(Mat3::IDENTITY * A), &A, 0.0);
    }

    #[test]
    fn inverse_times_matrix_is_identity() {
        let inv = A.inverse().unwrap();
        assert_mat_close(&(A * inv), &Mat3::IDENTITY, 1e-12);
        assert_mat_close(&(inv * A), &Mat3::IDENTITY, 1e-12);
    }

    #[test]
    fn singular_matrix_is_error() {
        let s = Mat3::new([[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.0, 1.0, 1.0]]);
        assert_eq!(s.inverse(), Err(CoreError::SingularMatrix));
        let nan = Mat3::new([[f64::NAN, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        assert_eq!(nan.inverse(), Err(CoreError::SingularMatrix));
    }

    #[test]
    fn transpose_swaps_elements() {
        let t = A.transpose();
        assert!((t.rows[0][2] - 1.0).abs() < f64::EPSILON);
        assert!((t.rows[2][0] - 0.5).abs() < f64::EPSILON);
        assert_mat_close(&t.transpose(), &A, 0.0);
    }

    #[test]
    fn mul_vec_matches_f32_path() {
        let v = [0.25, 0.5, 0.75];
        let d = A.mul_vec(v);
        #[allow(clippy::cast_possible_truncation)]
        let f = mul_vec_f32(&A.to_f32(), v.map(|x| x as f32));
        for i in 0..3 {
            assert!((d[i] - f64::from(f[i])).abs() < 1e-6);
        }
        // 2*0.25 + 1*0.5 + 0.5*0.75 = 1.375
        assert!((d[0] - 1.375).abs() < 1e-12);
    }

    #[test]
    fn determinant_of_diagonal() {
        assert!((Mat3::diagonal([2.0, 3.0, 4.0]).determinant() - 24.0).abs() < 1e-12);
    }
}
