//! Fast quantized transfer-function encode for [`WorkingImage::to_image`].
//!
//! [`WorkingImage::to_image`]: crate::buffer::WorkingImage::to_image
//!
//! # Method
//!
//! The exact path for one sample is `T::from_unit(tf.encode(x))` for a
//! clipped linear value `x` in `0.0..=1.0`. This module gives the same
//! integer code without a `powf` call per sample:
//!
//! 1. **Thresholds.** For each code `k` in `1..=T::MAX`, find the smallest
//!    `f32` value `x` where the exact path gives a code of `k` or more. The
//!    search starts at the analytic inverse `tf.decode((k - 0.5) / MAX)`,
//!    then uses an exponential search and a binary search on the `f32` bit
//!    patterns. All tests in this search use the exact path.
//! 2. **Buckets.** The bit pattern of a non-negative `f32` increases with
//!    its value. `bits >> shift` is a bucket index with an approximately
//!    logarithmic spacing (2^(23 - shift) buckets per octave).
//! 3. **Lookup.** There are two forms. [`LutEncoder::new`] selects one:
//!    - *Step form* (`u8`): each bucket stores the code at its lower edge.
//!      If no bucket holds more than one threshold, one compare with the
//!      next threshold gives the code. With 256 buckets per octave, this
//!      is true for all curves at 8 bits. The tables are about 33 KiB.
//!    - *Interpolation form* (`u16`): the codes are too dense for the step
//!      form. A table holds `tf.encode(edge) * MAX + 0.25` at each bucket
//!      edge. Linear interpolation in the bucket, then truncation, gives an
//!      estimated code. In one octave the bit pattern is linear in the
//!      value, so this is linear interpolation in `x`. The interpolation
//!      error is less than 0.02 code for all curves (the tests measure it).
//!      The offset is 0.25 less than the 0.5 of rounding, so the estimate
//!      is the correct code or one less. One compare with the next
//!      threshold corrects it.
//!
//!    Both forms have no data-dependent branch, so noisy images are as fast
//!    as smooth images.
//!
//! # Accuracy
//!
//! The result is the number of thresholds at or below `x`. If the exact
//! path is monotonic in `x`, this is identical to the exact path for every
//! `f32` input. The platform `powf` can be non-monotonic by one ulp at a few
//! points. There, the result can differ by one code from the exact path for
//! a very small set of inputs. The tests measure this bound.
//!
//! # Determinism
//!
//! The tables are a pure function of the transfer function and the sample
//! type. They are built again for each call, with no global state. The
//! lookup uses integer operations, `f32` comparisons, and separate `f32`
//! multiplies and adds (Rust does not fuse them into a multiply-add). The
//! estimate is corrected with exact thresholds, so even a different
//! rounding of the estimate does not change the result. For the same
//! tables, the result is thus the same on all targets. The tables use the
//! platform `powf` (as does the exact path). A platform with a different
//! `powf` can move a threshold by some ulps, and thus change a code for
//! inputs that are within some ulps of a code boundary.

use crate::buffer::io::Sample;
use crate::color::TransferFunction;

/// The bit pattern of `1.0_f32`.
const ONE_BITS: u32 = 0x3F80_0000;

/// Converts a clipped linear value (`+0.0..=1.0`, no NaN, no `-0.0`) to an
/// integer sample.
pub(crate) trait Encoder<T: Sample> {
    /// Encodes `x`. `x` must be in `+0.0..=1.0`.
    fn encode(&self, x: f32) -> T;
}

/// The identity transfer function: quantize the linear value directly.
pub(crate) struct LinearEncoder;

impl<T: Sample> Encoder<T> for LinearEncoder {
    #[inline]
    fn encode(&self, x: f32) -> T {
        T::from_unit(x)
    }
}

/// A lookup-table encoder for one transfer function and one sample type.
///
/// [`LutEncoder::new`] selects the faster form that is exact for the curve.
/// The caller matches on the form once per image, so the pixel loop has no
/// per-sample `match`.
pub(crate) enum LutEncoder<T: Sample> {
    /// Bucket codes with one correction compare. Used when no bucket holds
    /// more than one threshold (all curves for `u8`).
    Step(StepEncoder<T>),
    /// Interpolated estimates with a correction of one code down or up.
    /// Used for `u16`, where the codes are too dense for [`StepEncoder`].
    Interp(InterpEncoder<T>),
}

/// The exact code boundaries of a curve. Both encoder forms use them.
pub(crate) struct Thresholds<T: Sample> {
    /// `thresholds[k]` is the smallest `x` with a code of `k` or more, for
    /// `k` in `1..LEVELS`. `thresholds[0]` is `0.0`: no input is below it.
    /// `thresholds[LEVELS]` is `+inf`: no input is at or above it.
    thresholds: Vec<f32>,
    /// `codes[k]` is the sample with the code `k`.
    codes: Vec<T>,
}

/// The [`LutEncoder::Step`] form.
pub(crate) struct StepEncoder<T: Sample> {
    /// Right shift of the `f32` bit pattern that gives the bucket index.
    shift: u32,
    /// The code at the lower edge of each bucket.
    start: Vec<T>,
    bounds: Thresholds<T>,
}

/// The [`LutEncoder::Interp`] form.
pub(crate) struct InterpEncoder<T: Sample> {
    /// Right shift of the `f32` bit pattern that gives the bucket index.
    shift: u32,
    /// The mask of the bits below `shift`.
    frac_mask: u32,
    /// `2^-shift`: converts the masked bits to a fraction in `0.0..1.0`.
    frac_scale: f32,
    /// `tf.encode(edge) * MAX + ESTIMATE_BIAS` at each bucket edge. It has one entry
    /// more than the number of buckets, for the upper edge of the last one.
    estimate: Vec<f32>,
    bounds: Thresholds<T>,
}

/// The bucket shift of [`StepEncoder`]: 256 buckets per octave.
const STEP_SHIFT: u32 = 15;

/// The offset (in codes) that [`InterpEncoder`] adds to its estimates.
///
/// Rounding adds 0.5. This value is 0.25 less, so the truncated estimate
/// is never above the correct code. The estimate error must be less than
/// this value (the tests measure at most 0.013 code). Then the truncated
/// estimate is the correct code or one less.
const ESTIMATE_BIAS: f32 = 0.25;

impl<T: Sample> LutEncoder<T> {
    /// Builds the tables for `tf`.
    pub(crate) fn new(tf: TransferFunction) -> Self {
        match StepEncoder::try_new(Thresholds::new(tf), STEP_SHIFT) {
            Ok(step) => Self::Step(step),
            Err(bounds) => Self::Interp(InterpEncoder::new(tf, bounds)),
        }
    }
}

impl<T: Sample> Encoder<T> for LutEncoder<T> {
    /// Encodes one value. This `match` is per sample, so use it only for
    /// tests; the pixel loop matches once and uses the inner encoder.
    #[inline]
    fn encode(&self, x: f32) -> T {
        match self {
            Self::Step(e) => e.encode(x),
            Self::Interp(e) => e.encode(x),
        }
    }
}

impl<T: Sample> StepEncoder<T> {
    /// Builds the bucket codes. If a bucket holds more than one threshold,
    /// this form is not exact: it returns `bounds` back as the error.
    fn try_new(bounds: Thresholds<T>, shift: u32) -> Result<Self, Thresholds<T>> {
        let buckets = ONE_BITS >> shift;
        let mut start = Vec::with_capacity(buckets as usize + 1);
        let mut k = 0_usize;
        for b in 0..=buckets {
            let edge = f32::from_bits(b << shift);
            // `thresholds[LEVELS]` is +inf, so `k + 1 <= LEVELS` always.
            while bounds.thresholds[k + 1] <= edge {
                k += 1;
            }
            start.push(bounds.codes[k]);
        }
        // The thresholds in bucket `b` are the codes from `start[b] + 1` to
        // `start[b + 1]`. The last bucket holds only 1.0, so it has none.
        let single = start
            .windows(2)
            .all(|w| w[1].to_index() - w[0].to_index() <= 1);
        if single {
            Ok(Self {
                shift,
                start,
                bounds,
            })
        } else {
            Err(bounds)
        }
    }
}

impl<T: Sample> Encoder<T> for StepEncoder<T> {
    #[inline]
    fn encode(&self, x: f32) -> T {
        // `x` is in `+0.0..=1.0`, so `bits <= ONE_BITS` and the bucket index
        // is less than `start.len()`.
        let k = self.start[(x.to_bits() >> self.shift) as usize].to_index();
        // The bucket holds at most one threshold, so one compare finds the
        // code. `k <= LEVELS - 1`, so `k + 1` is in range; for
        // `k = LEVELS - 1` the threshold is +inf and the compare is false.
        let up = usize::from(x >= self.bounds.thresholds[k + 1]);
        self.bounds.codes[k + up]
    }
}

impl<T: Sample> Thresholds<T> {
    /// Finds the code boundaries of `tf` with the exact path.
    fn new(tf: TransferFunction) -> Self {
        let levels = T::LEVELS;
        let exact = |x: f32| T::from_unit(tf.encode(x)).to_index();

        let mut thresholds = vec![f32::INFINITY; levels + 1];
        thresholds[0] = 0.0;
        let top = exact(1.0);
        let mut lo = 0_u32;
        // Codes above `top` are never reached; their thresholds stay +inf.
        for (k, slot) in thresholds.iter_mut().enumerate().take(top + 1).skip(1) {
            #[allow(
                clippy::cast_precision_loss,
                reason = "k is at most 65535, which f32 represents exactly"
            )]
            let target = (k as f32 - 0.5) / T::MAX_F32;
            let guess = tf.decode(target);
            let bits = first_at_least(lo, guess, |b| exact(f32::from_bits(b)) >= k);
            *slot = f32::from_bits(bits);
            lo = bits;
        }

        let codes: Vec<T> = (0..levels)
            .map(|k| {
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "k is at most 65535, which f32 represents exactly"
                )]
                let unit = k as f32 / T::MAX_F32;
                // The relative error of `k / MAX * MAX` is below 2^-22, so
                // the rounded result is `k` again.
                T::from_unit(unit)
            })
            .collect();
        Self { thresholds, codes }
    }
}

impl<T: Sample> InterpEncoder<T> {
    /// Builds the estimate table for `tf`.
    fn new(tf: TransferFunction, bounds: Thresholds<T>) -> Self {
        // Buckets of 2^shift bit patterns: 2^(23 - shift) per octave. The
        // interpolation error falls by 4 for each bit less of `shift`. These
        // values keep it below 0.02 code, far below `ESTIMATE_BIAS` (the
        // tables are 64 KiB for u8 and 254 KiB for u16). A smaller table is
        // faster for u16 because more of it stays in the cache.
        let shift: u32 = if T::LEVELS <= 256 { 16 } else { 14 };
        // Edges `0..=(ONE_BITS >> shift) + 1`. The last edge is above 1.0;
        // the curve formula continues there.
        let estimate: Vec<f32> = (0..=(ONE_BITS >> shift) + 1)
            .map(|b| tf.encode(f32::from_bits(b << shift)) * T::MAX_F32 + ESTIMATE_BIAS)
            .collect();
        #[allow(
            clippy::cast_precision_loss,
            reason = "2^shift is a power of two below 2^24, so f32 holds it exactly"
        )]
        let frac_scale = 1.0 / (1_u32 << shift) as f32;
        Self {
            shift,
            frac_mask: (1 << shift) - 1,
            frac_scale,
            estimate,
            bounds,
        }
    }

    /// Returns the interpolated value of `tf.encode(x) * MAX` (with no
    /// bias), for the accuracy tests.
    #[cfg(test)]
    fn estimate_code(&self, x: f32) -> f32 {
        self.interpolate(x) - ESTIMATE_BIAS
    }

    /// Returns `tf.encode(x) * MAX + ESTIMATE_BIAS`, interpolated from the
    /// table.
    #[inline]
    fn interpolate(&self, x: f32) -> f32 {
        let bits = x.to_bits();
        // `x` is in `+0.0..=1.0`, so `bits <= ONE_BITS`. The index is at most
        // `ONE_BITS >> shift`, and `idx + 1` is the last table entry.
        let idx = (bits >> self.shift) as usize;
        #[allow(
            clippy::cast_precision_loss,
            reason = "the masked value is below 2^shift < 2^24, so f32 holds it exactly"
        )]
        let frac = (bits & self.frac_mask) as f32 * self.frac_scale;
        let lo = self.estimate[idx];
        let hi = self.estimate[idx + 1];
        lo + (hi - lo) * frac
    }
}

impl<T: Sample> Encoder<T> for InterpEncoder<T> {
    #[inline]
    fn encode(&self, x: f32) -> T {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a float to integer `as` cast saturates (negative and NaN give 0); \
                      the `min` keeps the code in range"
        )]
        let c = (self.interpolate(x) as usize).min(T::LEVELS - 1);
        // The table holds `encode * MAX + ESTIMATE_BIAS`, and the
        // interpolation error is less than `ESTIMATE_BIAS`. Thus the
        // truncated estimate `c` is the correct code `k` or `k - 1`. If
        // `x >= thresholds[c + 1]`, the code is `c + 1`. `c + 1 <= LEVELS`,
        // so the index is in range; for `c = LEVELS - 1` the threshold is
        // +inf and the compare is false.
        let up = usize::from(x >= self.bounds.thresholds[c + 1]);
        self.bounds.codes[c + up]
    }
}

/// Returns the smallest bit pattern `b` in `lo..=ONE_BITS` with `ok(b)`.
///
/// `ok` must be monotonic (false, then true) and `ok(ONE_BITS)` must be
/// true. If `ok` is not monotonic, the result is one of the points where it
/// changes from false to true. The search starts near `guess`.
fn first_at_least(lo: u32, guess: f32, ok: impl Fn(u32) -> bool) -> u32 {
    let hi = ONE_BITS;
    // A NaN or negative guess gives bits above ONE_BITS, and the clamp
    // moves them to `hi`. The search is still correct, only slower.
    let g = guess.to_bits().clamp(lo, hi);
    // Invariant of the binary search: `ok(b)` is true, and `a` is `None`
    // (all values below `lo`) or `ok(a)` is false.
    let (mut a, mut b): (Option<u32>, u32);
    if ok(g) {
        b = g;
        a = None;
        let mut step = 1_u32;
        while b > lo {
            let c = b.saturating_sub(step).max(lo);
            if ok(c) {
                b = c;
                step = step.saturating_mul(2);
            } else {
                a = Some(c);
                break;
            }
        }
    } else {
        let mut lower = g;
        let mut step = 1_u32;
        b = loop {
            let c = lower.saturating_add(step).min(hi);
            if c == hi || ok(c) {
                break c;
            }
            lower = c;
            step = step.saturating_mul(2);
        };
        a = Some(lower);
    }
    if let Some(mut a) = a {
        while b - a > 1 {
            let m = a + (b - a) / 2;
            if ok(m) {
                b = m;
            } else {
                a = m;
            }
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::ColorSpaceId;

    /// Builds the interpolating form, also for `u8` (where `new` selects
    /// the step form), so that the tests cover both forms for both types.
    fn interp<T: Sample>(tf: TransferFunction) -> InterpEncoder<T> {
        InterpEncoder::new(tf, Thresholds::new(tf))
    }

    /// Both forms of the encoder for `tf`, as far as they exist for `T`.
    fn forms<T: Sample>(tf: TransferFunction) -> Vec<LutEncoder<T>> {
        let mut out = vec![LutEncoder::Interp(interp(tf))];
        if let Ok(step) = StepEncoder::try_new(Thresholds::new(tf), STEP_SHIFT) {
            out.push(LutEncoder::Step(step));
        }
        out
    }

    /// Compares the LUT with the exact path over a dense sweep of `f32`
    /// values in `0.0..=1.0`, for both encoder forms. Returns (max code
    /// difference, number of differing inputs, number of inputs).
    fn sweep<T: Sample>(tf: TransferFunction, step_bits: u32) -> (usize, usize, usize) {
        let (mut max_diff, mut diffs, mut n) = (0, 0, 0);
        for lut in forms::<T>(tf) {
            let (d, c, m) = sweep_one(tf, step_bits, &lut);
            max_diff = max_diff.max(d);
            diffs += c;
            n += m;
        }
        (max_diff, diffs, n)
    }

    fn sweep_one<T: Sample>(
        tf: TransferFunction,
        step_bits: u32,
        lut: &LutEncoder<T>,
    ) -> (usize, usize, usize) {
        let mut max_diff = 0;
        let mut diffs = 0;
        let mut n = 0;
        let mut max_est_err = 0.0_f32;
        let mut check = |x: f32| {
            if let LutEncoder::Interp(e) = lut {
                // The correction step needs an estimate error below 0.5 code.
                let est_err = (e.estimate_code(x) - tf.encode(x) * T::MAX_F32).abs();
                max_est_err = max_est_err.max(est_err);
            }
            let exact = T::from_unit(tf.encode(x)).to_index();
            let fast = lut.encode(x).to_index();
            let d = exact.abs_diff(fast);
            max_diff = max_diff.max(d);
            if d != 0 {
                diffs += 1;
            }
            n += 1;
        };
        // Every `step_bits`-th bit pattern: dense on a log scale.
        let mut b = 0_u32;
        while b <= ONE_BITS {
            check(f32::from_bits(b));
            b += step_bits;
        }
        // A uniform sweep: dense on a linear scale (where most image
        // values are).
        for i in 0..=1_000_000_u32 {
            #[allow(clippy::cast_precision_loss)]
            check(i as f32 / 1_000_000.0);
        }
        let form = match lut {
            LutEncoder::Step(_) => "step",
            LutEncoder::Interp(_) => "interp",
        };
        println!(
            "{tf:?} {}-bit {form}: max diff {max_diff}, {diffs} of {n} differ, \
             max estimate error {max_est_err:.5} code",
            T::LEVELS.ilog2()
        );
        assert!(max_est_err < 0.05, "{tf:?}: estimate error {max_est_err}");
        (max_diff, diffs, n)
    }

    fn all_transfers() -> Vec<(ColorSpaceId, TransferFunction)> {
        ColorSpaceId::ALL
            .into_iter()
            .map(|id| (id, id.transfer()))
            .collect()
    }

    #[test]
    fn new_selects_step_form_for_u8_and_interp_form_for_u16() {
        for (id, tf) in all_transfers() {
            assert!(
                matches!(LutEncoder::<u8>::new(tf), LutEncoder::Step(_)),
                "{id:?}"
            );
            assert!(
                matches!(LutEncoder::<u16>::new(tf), LutEncoder::Interp(_)),
                "{id:?}"
            );
        }
    }

    #[test]
    fn u8_lut_matches_exact_path() {
        for (id, tf) in all_transfers() {
            let (max_diff, diffs, n) = sweep::<u8>(tf, 97);
            #[allow(clippy::cast_precision_loss)]
            let frac = diffs as f64 / n as f64;
            assert!(max_diff <= 1, "{id:?}: max diff {max_diff}");
            assert!(frac <= 0.001, "{id:?}: {diffs} of {n} differ");
        }
    }

    #[test]
    fn u16_lut_matches_exact_path() {
        for (id, tf) in all_transfers() {
            let (max_diff, diffs, n) = sweep::<u16>(tf, 389);
            #[allow(clippy::cast_precision_loss)]
            let frac = diffs as f64 / n as f64;
            assert!(max_diff <= 1, "{id:?}: max diff {max_diff}");
            assert!(frac <= 0.001, "{id:?}: {diffs} of {n} differ");
        }
    }

    /// Every threshold and the value one ulp below it: the points where the
    /// LUT changes its result.
    #[test]
    fn thresholds_are_exact_code_boundaries() {
        fn check<T: Sample>(id: ColorSpaceId, tf: TransferFunction) {
            let bounds = Thresholds::<T>::new(tf);
            let exact = |x: f32| T::from_unit(tf.encode(x)).to_index();
            for lut in forms::<T>(tf) {
                for k in 1..T::LEVELS {
                    let t = bounds.thresholds[k];
                    assert!(t.is_finite(), "{id:?}: code {k} unreachable");
                    assert!(exact(t) >= k, "{id:?}: {k}");
                    assert_eq!(lut.encode(t).to_index(), exact(t), "{id:?}: {k}");
                    if t > 0.0 {
                        let below = f32::from_bits(t.to_bits() - 1);
                        assert!(exact(below) < k, "{id:?}: {k}");
                        assert_eq!(lut.encode(below).to_index(), exact(below), "{id:?}: {k}");
                    }
                }
            }
        }
        for (id, tf) in all_transfers() {
            check::<u8>(id, tf);
            check::<u16>(id, tf);
        }
    }

    #[test]
    fn edge_values() {
        fn check<T: Sample>(id: ColorSpaceId, tf: TransferFunction) {
            for lut in forms::<T>(tf) {
                assert_eq!(lut.encode(0.0).to_index(), 0, "{id:?}");
                assert_eq!(lut.encode(1.0), T::MAX, "{id:?}");
                // Subnormals, tiny normals, and values next to 1.0.
                for x in [
                    f32::from_bits(1),
                    f32::from_bits(0x007F_FFFF),
                    f32::MIN_POSITIVE,
                    1e-30,
                    1e-10,
                    f32::from_bits(ONE_BITS - 1),
                ] {
                    let exact = T::from_unit(tf.encode(x));
                    assert_eq!(lut.encode(x), exact, "{id:?}: {x}");
                }
            }
        }
        for (id, tf) in all_transfers() {
            check::<u8>(id, tf);
            check::<u16>(id, tf);
        }
    }

    /// Every `f32` in `+0.0..=1.0` (about 1.07e9 values), selected form,
    /// u8 and u16. Run in release mode:
    /// `cargo test -p expose-core --release -- --ignored exhaustive --nocapture`.
    #[test]
    #[ignore = "slow; run manually in release mode"]
    fn exhaustive_all_f32_in_unit_range() {
        fn run<T: Sample>(id: ColorSpaceId, tf: TransferFunction) {
            let lut = LutEncoder::<T>::new(tf);
            let mut diffs = 0_u64;
            let mut max_diff = 0;
            for b in 0..=ONE_BITS {
                let x = f32::from_bits(b);
                let d = lut
                    .encode(x)
                    .to_index()
                    .abs_diff(T::from_unit(tf.encode(x)).to_index());
                max_diff = max_diff.max(d);
                diffs += u64::from(d != 0);
            }
            println!(
                "exhaustive {id:?} {}-bit: max diff {max_diff}, {diffs} differ",
                T::LEVELS.ilog2()
            );
            assert!(max_diff <= 1, "{id:?}");
        }
        for (id, tf) in all_transfers() {
            run::<u8>(id, tf);
            run::<u16>(id, tf);
        }
    }

    #[test]
    fn first_at_least_finds_boundary_from_any_guess() {
        let target = 0x3E00_1234_u32;
        for guess in [
            0.0,
            1.0,
            f32::NAN,
            -1.0,
            f32::from_bits(target),
            0.25,
            1e-20,
        ] {
            assert_eq!(first_at_least(0, guess, |b| b >= target), target, "{guess}");
        }
        // The answer is `lo` when `ok(lo)` is true.
        assert_eq!(first_at_least(100, 0.5, |_| true), 100);
        // The answer is ONE_BITS when only ONE_BITS is true.
        assert_eq!(first_at_least(0, 0.5, |b| b == ONE_BITS), ONE_BITS);
    }
}
