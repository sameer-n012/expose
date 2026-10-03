//! ICC profile writer for the output color spaces.
//!
//! The writer makes small matrix/curve RGB display profiles (class `mntr`)
//! in ICC version 4.3. Each profile has these tags:
//!
//! | Tag | Type | Content |
//! | --- | --- | --- |
//! | `desc` | `mluc` | the description (for example "Display P3") |
//! | `cprt` | `mluc` | a copyright notice ("No copyright, use freely") |
//! | `wtpt` | `XYZ ` | the PCS illuminant D50 (as ICC v4 requires) |
//! | `chad` | `sf32` | the Bradford matrix from the space white to D50 |
//! | `rXYZ`, `gXYZ`, `bXYZ` | `XYZ ` | primaries, Bradford-adapted to D50 |
//! | `rTRC`, `gTRC`, `bTRC` | `para` | one shared parametric curve |
//!
//! # Decisions
//!
//! - Version 4 (not 2.4): the `para` curve type and `chad` are v4 types,
//!   and the parametric curves are exact (a v2 profile needs a sampled
//!   `curv` table for the piecewise curves). Current operating systems and
//!   browsers read v4 matrix/curve profiles.
//! - The profile ID (MD5) is zero. ICC v4 permits this ("not computed").
//! - The creation date is fixed (2026-01-01), so that the output is
//!   deterministic.
//! - The colorant matrix is rounded to `s15Fixed16` values, and then the
//!   green column is corrected so that the three columns add up exactly to
//!   the D50 white. Then RGB white maps exactly to the PCS white.

use expose_core::color::{Chromaticity, ColorSpaceId, Mat3, TransferFunction, bradford_adaptation};

use crate::error::{EncodeError, EncodeResult};
use crate::icc::{PROPHOTO_PARA, REC2020_PARA, SRGB_PARA};

/// The ICC PCS illuminant (D50) in XYZ, with the exact `s15Fixed16` values
/// that ICC.1 specifies (X = 0.9642, Y = 1.0, Z = 0.8249).
const PCS_D50_FIXED: [i32; 3] = [0x0000_F6D6, 0x0001_0000, 0x0000_D32D];

/// A 3x3 matrix of `s15Fixed16Number` values.
type Fixed3x3 = [[i32; 3]; 3];

/// Size of the ICC header in bytes.
const HEADER_LEN: usize = 128;

/// Returns the description that the profile of `space` has.
///
/// The names are neutral standard names.
pub const fn icc_description(space: ColorSpaceId) -> &'static str {
    match space {
        ColorSpaceId::Srgb => "sRGB IEC61966-2.1",
        ColorSpaceId::DisplayP3 => "Display P3",
        ColorSpaceId::OpRgb => "opRGB",
        ColorSpaceId::ProPhotoRgb => "ROMM RGB",
        ColorSpaceId::Rec2020 => "Rec. ITU-R BT.2020",
        ColorSpaceId::LinearRec2020 => "Rec. ITU-R BT.2020 linear",
    }
}

/// Converts a value to `s15Fixed16Number`.
fn s15f16(v: f64) -> EncodeResult<i32> {
    let scaled = (v * 65536.0).round();
    if !scaled.is_finite() || scaled < f64::from(i32::MIN) || scaled > f64::from(i32::MAX) {
        return Err(EncodeError::InvalidOption {
            name: "icc value",
            reason: format!("{v} does not fit in s15Fixed16"),
        });
    }
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the value is rounded and checked to be in the i32 range"
    )]
    Ok(scaled as i32)
}

/// The chromaticity of the ICC PCS illuminant, from its XYZ value.
fn pcs_white() -> Chromaticity {
    let xyz = PCS_D50_FIXED.map(|v| f64::from(v) / 65536.0);
    let sum = xyz[0] + xyz[1] + xyz[2];
    Chromaticity::new(xyz[0] / sum, xyz[1] / sum)
}

/// Returns the colorant matrix (columns = R, G, B in PCS XYZ) as
/// `s15Fixed16` values, with the white sum correction (see the module
/// documentation), and the `chad` matrix.
fn colorants(space: ColorSpaceId) -> EncodeResult<(Fixed3x3, Fixed3x3)> {
    let def = space.definition();
    let chad = bradford_adaptation(def.white, pcs_white())?;
    let m: Mat3 = chad * def.to_xyz()?;
    // cols[c][row]: column c (primary), XYZ component `row`.
    let mut cols: Fixed3x3 = [[0; 3]; 3];
    for (c, col) in cols.iter_mut().enumerate() {
        for (row, v) in col.iter_mut().enumerate() {
            *v = s15f16(m.rows[row][c])?;
        }
    }
    let [red, _, blue] = cols;
    for (row, white) in PCS_D50_FIXED.iter().enumerate() {
        cols[1][row] = white - red[row] - blue[row];
    }
    let mut chad_fixed: Fixed3x3 = [[0; 3]; 3];
    for (r, out) in chad_fixed.iter_mut().enumerate() {
        for (c, v) in out.iter_mut().enumerate() {
            *v = s15f16(chad.rows[r][c])?;
        }
    }
    Ok((cols, chad_fixed))
}

/// Builds the `para` tag data for a transfer function.
fn para_tag(tf: TransferFunction) -> EncodeResult<Vec<u8>> {
    let (func, params): (u16, Vec<f64>) = match tf {
        TransferFunction::Linear => (0, vec![1.0]),
        TransferFunction::Gamma(g) => (0, vec![f64::from(g)]),
        TransferFunction::Srgb => (3, SRGB_PARA.to_vec()),
        TransferFunction::Rec2020 => (3, REC2020_PARA.to_vec()),
        TransferFunction::ProPhoto => (3, PROPHOTO_PARA.to_vec()),
    };
    let mut t = Vec::with_capacity(12 + 4 * params.len());
    t.extend_from_slice(b"para\0\0\0\0");
    t.extend_from_slice(&func.to_be_bytes());
    t.extend_from_slice(&[0, 0]);
    for p in params {
        t.extend_from_slice(&s15f16(p)?.to_be_bytes());
    }
    Ok(t)
}

/// Builds an `XYZ ` tag with one value.
fn xyz_tag(v: [i32; 3]) -> Vec<u8> {
    let mut t = Vec::with_capacity(20);
    t.extend_from_slice(b"XYZ \0\0\0\0");
    for c in v {
        t.extend_from_slice(&c.to_be_bytes());
    }
    t
}

/// Builds an `sf32` tag with a 3x3 matrix in row order.
fn sf32_tag(m: [[i32; 3]; 3]) -> Vec<u8> {
    let mut t = Vec::with_capacity(44);
    t.extend_from_slice(b"sf32\0\0\0\0");
    for v in m.iter().flatten() {
        t.extend_from_slice(&v.to_be_bytes());
    }
    t
}

/// Builds an `mluc` tag with one English (US) record.
fn mluc_tag(text: &str) -> EncodeResult<Vec<u8>> {
    let utf16: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let len = u32::try_from(utf16.len()).map_err(|_| EncodeError::InvalidOption {
        name: "icc text",
        reason: "text is too long".to_string(),
    })?;
    let mut t = Vec::with_capacity(28 + utf16.len());
    t.extend_from_slice(b"mluc\0\0\0\0");
    t.extend_from_slice(&1_u32.to_be_bytes()); // record count
    t.extend_from_slice(&12_u32.to_be_bytes()); // record size
    t.extend_from_slice(b"enUS");
    t.extend_from_slice(&len.to_be_bytes());
    t.extend_from_slice(&28_u32.to_be_bytes()); // offset of the text
    t.extend_from_slice(&utf16);
    Ok(t)
}

fn to_u32(v: usize) -> EncodeResult<u32> {
    u32::try_from(v).map_err(|_| EncodeError::InvalidOption {
        name: "icc profile",
        reason: "profile is too large".to_string(),
    })
}

/// Writes the 128-byte header. The size field is set later.
fn header() -> [u8; HEADER_LEN] {
    let mut h = [0_u8; HEADER_LEN];
    // 0..4 size (set later), 4..8 preferred CMM (none).
    h[8..12].copy_from_slice(&[4, 0x30, 0, 0]); // version 4.3
    h[12..16].copy_from_slice(b"mntr");
    h[16..20].copy_from_slice(b"RGB ");
    h[20..24].copy_from_slice(b"XYZ ");
    // Creation date and time: 2026-01-01 00:00:00 (fixed, see module doc).
    for (i, v) in [2026_u16, 1, 1, 0, 0, 0].into_iter().enumerate() {
        h[24 + 2 * i..26 + 2 * i].copy_from_slice(&v.to_be_bytes());
    }
    h[36..40].copy_from_slice(b"acsp");
    // 40..64: platform, flags, manufacturer, model, attributes: all zero.
    // 64..68: rendering intent 0 (perceptual).
    for (i, v) in PCS_D50_FIXED.into_iter().enumerate() {
        h[68 + 4 * i..72 + 4 * i].copy_from_slice(&v.to_be_bytes());
    }
    // 80..84 creator, 84..100 profile ID (zero = not computed), rest reserved.
    h
}

/// Returns an ICC v4.3 matrix/curve display profile for `space`.
///
/// The profile has the primaries, white point (through `chad`), and
/// transfer function of the `expose-core` definition of `space`.
pub fn icc_profile(space: ColorSpaceId) -> EncodeResult<Vec<u8>> {
    let (cols, chad) = colorants(space)?;
    let trc = para_tag(space.definition().transfer)?;
    // Each data block, and the tag signatures that point to it.
    let blocks: Vec<(Vec<[u8; 4]>, Vec<u8>)> = vec![
        (vec![*b"desc"], mluc_tag(icc_description(space))?),
        (vec![*b"cprt"], mluc_tag("No copyright, use freely")?),
        (vec![*b"wtpt"], xyz_tag(PCS_D50_FIXED)),
        (vec![*b"chad"], sf32_tag(chad)),
        (vec![*b"rXYZ"], xyz_tag(cols[0])),
        (vec![*b"gXYZ"], xyz_tag(cols[1])),
        (vec![*b"bXYZ"], xyz_tag(cols[2])),
        (vec![*b"rTRC", *b"gTRC", *b"bTRC"], trc),
    ];
    let tag_count: usize = blocks.iter().map(|(sigs, _)| sigs.len()).sum();
    let table_end = HEADER_LEN + 4 + 12 * tag_count;

    let mut table = Vec::with_capacity(4 + 12 * tag_count);
    table.extend_from_slice(&to_u32(tag_count)?.to_be_bytes());
    let mut data = Vec::new();
    for (sigs, block) in &blocks {
        // Tag data starts on a 4-byte boundary (the table end is aligned
        // because the header is 128 bytes and each entry is 12 bytes).
        while data.len() % 4 != 0 {
            data.push(0);
        }
        let offset = to_u32(table_end + data.len())?;
        let size = to_u32(block.len())?;
        for sig in sigs {
            table.extend_from_slice(sig);
            table.extend_from_slice(&offset.to_be_bytes());
            table.extend_from_slice(&size.to_be_bytes());
        }
        data.extend_from_slice(block);
    }
    // ICC v4: the profile size is a multiple of 4.
    while data.len() % 4 != 0 {
        data.push(0);
    }

    let mut out = Vec::with_capacity(table_end + data.len());
    out.extend_from_slice(&header());
    out.extend_from_slice(&table);
    out.extend_from_slice(&data);
    let len = to_u32(out.len())?;
    out[0..4].copy_from_slice(&len.to_be_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;
    use crate::icc::{ProfileKind, ToneCurve, identify};

    fn be_u32(d: &[u8], pos: usize) -> u32 {
        u32::from_be_bytes(d[pos..pos + 4].try_into().unwrap())
    }

    fn be_i32(d: &[u8], pos: usize) -> i32 {
        i32::from_be_bytes(d[pos..pos + 4].try_into().unwrap())
    }

    const SPACES: [ColorSpaceId; 6] = ColorSpaceId::ALL;

    #[test]
    fn structure_is_valid() {
        for space in SPACES {
            let p = icc_profile(space).unwrap();
            assert_eq!(be_u32(&p, 0) as usize, p.len(), "{space:?}");
            assert_eq!(p.len() % 4, 0);
            assert_eq!(&p[8..12], &[4, 0x30, 0, 0]);
            assert_eq!(&p[12..24], b"mntrRGB XYZ ");
            assert_eq!(&p[36..40], b"acsp");
            assert!(p[84..100].iter().all(|b| *b == 0), "profile ID is zero");
            let count = be_u32(&p, 128) as usize;
            assert_eq!(count, 10);
            for i in 0..count {
                let e = 132 + 12 * i;
                let off = be_u32(&p, e + 4) as usize;
                let len = be_u32(&p, e + 8) as usize;
                assert_eq!(off % 4, 0, "tag {i} offset alignment");
                assert!(off >= 132 + 12 * count && off + len <= p.len());
            }
        }
    }

    #[test]
    fn reader_identifies_written_profiles() {
        for space in SPACES {
            let p = icc_profile(space).unwrap();
            let info = identify(&p).unwrap();
            assert_eq!(info.kind, ProfileKind::Rgb);
            let expected_primaries = match space {
                ColorSpaceId::LinearRec2020 => ColorSpaceId::Rec2020,
                other => other,
            };
            assert_eq!(info.primaries, Some(expected_primaries), "{space:?}");
            assert_eq!(info.description.as_deref(), Some(icc_description(space)));
            let curve = info.curve.unwrap();
            match space.definition().transfer {
                TransferFunction::Linear => assert_eq!(curve, ToneCurve::Linear),
                TransferFunction::Srgb => assert_eq!(curve, ToneCurve::Srgb),
                TransferFunction::Rec2020 => assert_eq!(curve, ToneCurve::Rec2020),
                TransferFunction::ProPhoto => assert_eq!(curve, ToneCurve::ProPhoto),
                TransferFunction::Gamma(g) => {
                    assert!(matches!(curve, ToneCurve::Gamma(x) if (x - g).abs() < 1e-4));
                }
            }
        }
    }

    #[test]
    fn colorants_add_up_to_d50() {
        for space in SPACES {
            let p = icc_profile(space).unwrap();
            let count = be_u32(&p, 128) as usize;
            let mut sum = [0_i32; 3];
            for i in 0..count {
                let e = 132 + 12 * i;
                if [b"rXYZ", b"gXYZ", b"bXYZ"].contains(&&p[e..e + 4].try_into().unwrap()) {
                    let off = be_u32(&p, e + 4) as usize;
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += be_i32(&p, off + 8 + 4 * c);
                    }
                }
            }
            assert_eq!(sum, PCS_D50_FIXED, "{space:?}");
        }
    }

    /// Evaluates a written `para` curve (decode direction).
    fn eval_para(t: &[u8], x: f64) -> f64 {
        let func = u16::from_be_bytes([t[8], t[9]]);
        let p: Vec<f64> = (0..5)
            .map_while(|i| t.get(12 + 4 * i..16 + 4 * i))
            .map(|b| f64::from(i32::from_be_bytes(b.try_into().unwrap())) / 65536.0)
            .collect();
        match func {
            0 => x.powf(p[0]),
            3 => {
                if x >= p[4] {
                    (p[1] * x + p[2]).powf(p[0])
                } else {
                    p[3] * x
                }
            }
            _ => panic!("unexpected function type {func}"),
        }
    }

    #[test]
    fn curves_agree_with_core_transfer_functions() {
        for space in SPACES {
            let tf = space.definition().transfer;
            let t = para_tag(tf).unwrap();
            for i in 0..=100 {
                let x = f64::from(i) / 100.0;
                #[allow(clippy::cast_possible_truncation, reason = "test value in 0..=1")]
                let core = f64::from(tf.decode(x as f32));
                let icc = eval_para(&t, x);
                assert!(
                    (core - icc).abs() < 2e-4,
                    "{space:?} x={x}: {core} vs {icc}"
                );
            }
        }
    }

    #[test]
    fn descriptions_are_neutral() {
        let p = icc_profile(ColorSpaceId::OpRgb).unwrap();
        assert_eq!(identify(&p).unwrap().description.as_deref(), Some("opRGB"));
        // Output is deterministic.
        assert_eq!(p, icc_profile(ColorSpaceId::OpRgb).unwrap());
    }
}
