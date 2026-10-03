//! Identification of embedded ICC profiles.
//!
//! This is not a color management system. The code reads the colorant tags
//! (`rXYZ`, `gXYZ`, `bXYZ`) and the tone curve tags (`rTRC`, `kTRC`) of a
//! matrix/curve RGB or gray profile. It compares them with the built-in
//! color spaces of `expose-core`. If they agree, the decoder can use the
//! exact built-in conversion. Other profiles are reported as not
//! recognized, so that the caller can see that the color space was assumed.

use expose_core::color::{ColorSpaceId, Mat3, bradford_adaptation, white};

/// Largest profile that the parser reads (profiles are normally < 1 MB).
const MAX_PROFILE_LEN: usize = 16 * 1024 * 1024;

/// Largest difference per XYZ component between a colorant and the
/// expected value. ICC values have a resolution of about 1.5e-5; the
/// tolerance also accepts small differences of chromatic adaptation in
/// older profiles. The candidate spaces differ by much more than this.
const COLORANT_TOLERANCE: f64 = 0.01;

/// The data color space of a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileKind {
    /// RGB data.
    Rgb,
    /// Gray data.
    Gray,
    /// Another data color space (for example CMYK).
    Other,
}

/// The tone curve of a profile, as far as the parser can classify it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ToneCurve {
    /// Identity curve.
    Linear,
    /// A pure power curve with this gamma.
    Gamma(f32),
    /// The sRGB piecewise curve.
    Srgb,
    /// A curve that the parser cannot classify.
    Other,
}

/// What the parser found in a profile.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProfileInfo {
    /// The data color space.
    pub kind: ProfileKind,
    /// The profile description (`desc` tag), if present and readable.
    pub description: Option<String>,
    /// The built-in space whose primaries and white agree with the
    /// colorants. `None` if there are no colorants, or no space agrees.
    pub primaries: Option<ColorSpaceId>,
    /// The tone curve of the first channel (`rTRC` or `kTRC`).
    pub curve: Option<ToneCurve>,
}

fn be_u32(d: &[u8], pos: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        d.get(pos..pos.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn be_u16(d: &[u8], pos: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        d.get(pos..pos.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn s15f16(d: &[u8], pos: usize) -> Option<f64> {
    let v = i32::from_be_bytes(d.get(pos..pos.checked_add(4)?)?.try_into().ok()?);
    Some(f64::from(v) / 65536.0)
}

/// Finds a tag and returns its data.
fn tag(d: &[u8], sig: [u8; 4]) -> Option<&[u8]> {
    let count = usize::try_from(be_u32(d, 128)?).ok()?;
    // Each entry is 12 bytes; reject counts that cannot fit in the data.
    if count > d.len() / 12 {
        return None;
    }
    (0..count).find_map(|i| {
        let e = 132 + i * 12;
        if d.get(e..e + 4)? != sig.as_slice() {
            return None;
        }
        let off = usize::try_from(be_u32(d, e + 4)?).ok()?;
        let len = usize::try_from(be_u32(d, e + 8)?).ok()?;
        d.get(off..off.checked_add(len)?)
    })
}

/// Reads an `XYZ ` type tag (one XYZ value).
fn xyz_tag(d: &[u8], sig: [u8; 4]) -> Option<[f64; 3]> {
    let t = tag(d, sig)?;
    if t.get(0..4)? != b"XYZ " {
        return None;
    }
    Some([s15f16(t, 8)?, s15f16(t, 12)?, s15f16(t, 16)?])
}

/// Reads the description from a `desc` (ICC v2) or `mluc` (ICC v4) tag.
fn description(d: &[u8]) -> Option<String> {
    let t = tag(d, *b"desc")?;
    match t.get(0..4)? {
        b"desc" => {
            let n = usize::try_from(be_u32(t, 8)?).ok()?;
            let bytes = t.get(12..12usize.checked_add(n)?)?;
            let s = String::from_utf8_lossy(bytes);
            Some(s.trim_end_matches('\0').to_string())
        }
        b"mluc" => {
            // Use the first record. Text is UTF-16BE.
            let len = usize::try_from(be_u32(t, 20)?).ok()?;
            let off = usize::try_from(be_u32(t, 24)?).ok()?;
            let bytes = t.get(off..off.checked_add(len)?)?;
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            Some(
                String::from_utf16_lossy(&units)
                    .trim_end_matches('\0')
                    .to_string(),
            )
        }
        _ => None,
    }
}

/// The sRGB decode function in f64, for curve comparison.
fn srgb_decode(v: f64) -> f64 {
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Classifies a `curv` or `para` tone curve tag.
fn curve(d: &[u8], sig: [u8; 4]) -> Option<ToneCurve> {
    let t = tag(d, sig)?;
    match t.get(0..4)? {
        b"curv" => {
            let n = usize::try_from(be_u32(t, 8)?).ok()?;
            match n {
                0 => Some(ToneCurve::Linear),
                1 => {
                    let g = f32::from(be_u16(t, 12)?) / 256.0;
                    Some(gamma_curve(g))
                }
                _ => {
                    let table: Option<Vec<f64>> = (0..n)
                        .map(|i| be_u16(t, 12 + 2 * i).map(|v| f64::from(v) / 65535.0))
                        .collect();
                    Some(classify_table(&table?))
                }
            }
        }
        b"para" => {
            let func = be_u16(t, 8)?;
            let p: Vec<f64> = (0..7).map_while(|i| s15f16(t, 12 + 4 * i)).collect();
            let g = *p.first()?;
            match func {
                0 =>
                {
                    #[allow(
                        clippy::cast_possible_truncation,
                        reason = "a gamma value is a small number"
                    )]
                    Some(gamma_curve(g as f32))
                }
                3 if p.len() >= 5 => {
                    // Y = (aX + b)^g for X >= d, else cX. Compare with sRGB.
                    let srgb = [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.040_45];
                    let close = p.iter().zip(srgb).all(|(a, b)| (a - b).abs() < 2e-3);
                    Some(if close {
                        ToneCurve::Srgb
                    } else {
                        ToneCurve::Other
                    })
                }
                _ => Some(ToneCurve::Other),
            }
        }
        _ => None,
    }
}

fn gamma_curve(g: f32) -> ToneCurve {
    if (g - 1.0).abs() < 1e-3 {
        ToneCurve::Linear
    } else {
        ToneCurve::Gamma(g)
    }
}

/// Classifies a sampled curve: linear, sRGB, or other.
fn classify_table(table: &[f64]) -> ToneCurve {
    let n = table.len();
    if n < 2 {
        return ToneCurve::Other;
    }
    #[allow(clippy::cast_precision_loss, reason = "table sizes are small")]
    let x = |i: usize| i as f64 / (n - 1) as f64;
    let max_err = |f: &dyn Fn(f64) -> f64| {
        table
            .iter()
            .enumerate()
            .map(|(i, v)| (f(x(i)) - v).abs())
            .fold(0.0_f64, f64::max)
    };
    // 16-bit tables and 8-bit-derived tables are accurate to about 1e-3.
    if max_err(&|v| v) < 2e-3 {
        ToneCurve::Linear
    } else if max_err(&srgb_decode) < 2e-3 {
        ToneCurve::Srgb
    } else {
        ToneCurve::Other
    }
}

/// Returns the colorant columns (RGB to XYZ under D50) that an ICC profile
/// of the built-in space `id` has, with Bradford adaptation.
fn expected_colorants(id: ColorSpaceId) -> Option<Mat3> {
    let def = id.definition();
    let m = def.to_xyz().ok()?;
    Some(bradford_adaptation(def.white, white::D50).ok()? * m)
}

/// Parses `profile` and identifies it as far as possible.
///
/// Returns `None` if the data is not an ICC profile.
pub(crate) fn identify(profile: &[u8]) -> Option<ProfileInfo> {
    if profile.len() < 132 || profile.len() > MAX_PROFILE_LEN || profile.get(36..40)? != b"acsp" {
        return None;
    }
    let kind = match profile.get(16..20)? {
        b"RGB " => ProfileKind::Rgb,
        b"GRAY" => ProfileKind::Gray,
        _ => ProfileKind::Other,
    };
    let description = description(profile);
    let (primaries, curve) = match kind {
        ProfileKind::Rgb => {
            let cols = [
                xyz_tag(profile, *b"rXYZ"),
                xyz_tag(profile, *b"gXYZ"),
                xyz_tag(profile, *b"bXYZ"),
            ];
            let primaries = match cols {
                [Some(r), Some(g), Some(b)] => match_primaries(&Mat3::from_columns(r, g, b)),
                _ => None,
            };
            (primaries, curve(profile, *b"rTRC"))
        }
        ProfileKind::Gray => (None, curve(profile, *b"kTRC")),
        ProfileKind::Other => (None, None),
    };
    Some(ProfileInfo {
        kind,
        description,
        primaries,
        curve,
    })
}

/// Finds the built-in space whose colorants agree with `m`.
fn match_primaries(m: &Mat3) -> Option<ColorSpaceId> {
    [
        ColorSpaceId::Srgb,
        ColorSpaceId::DisplayP3,
        ColorSpaceId::OpRgb,
        ColorSpaceId::ProPhotoRgb,
        ColorSpaceId::Rec2020,
    ]
    .into_iter()
    .find(|&id| {
        expected_colorants(id).is_some_and(|e| {
            e.rows
                .iter()
                .flatten()
                .zip(m.rows.iter().flatten())
                .all(|(a, b)| (a - b).abs() < COLORANT_TOLERANCE)
        })
    })
}

/// Test helpers that build small ICC profiles.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    fn s15(v: f64) -> [u8; 4] {
        #[allow(clippy::cast_possible_truncation, reason = "test values are small")]
        let i = (v * 65536.0).round() as i32;
        i.to_be_bytes()
    }

    /// Builds a matrix/curve RGB profile for `id` with a gamma curve.
    pub(crate) fn rgb_profile(id: ColorSpaceId, gamma: f32, desc: &str) -> Vec<u8> {
        let m = expected_colorants(id).unwrap_or(Mat3::IDENTITY);
        let mut tags: Vec<([u8; 4], Vec<u8>)> = Vec::new();
        for (i, sig) in [b"rXYZ", b"gXYZ", b"bXYZ"].into_iter().enumerate() {
            let mut t = b"XYZ \0\0\0\0".to_vec();
            for r in 0..3 {
                t.extend_from_slice(&s15(m.rows[r][i]));
            }
            tags.push((*sig, t));
        }
        let mut curv = b"curv\0\0\0\0".to_vec();
        curv.extend_from_slice(&1_u32.to_be_bytes());
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let g = (gamma * 256.0).round() as u16;
        curv.extend_from_slice(&g.to_be_bytes());
        tags.push((*b"rTRC", curv));
        let mut d = b"desc\0\0\0\0".to_vec();
        let text = format!("{desc}\0");
        d.extend_from_slice(&u32::try_from(text.len()).unwrap_or(0).to_be_bytes());
        d.extend_from_slice(text.as_bytes());
        tags.push((*b"desc", d));
        build(*b"RGB ", &tags)
    }

    /// Builds a profile from a data color space and tags.
    pub(crate) fn build(space: [u8; 4], tags: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = vec![0_u8; 128];
        out[16..20].copy_from_slice(&space);
        out[20..24].copy_from_slice(b"XYZ ");
        out[36..40].copy_from_slice(b"acsp");
        let n = u32::try_from(tags.len()).unwrap_or(0);
        out.extend_from_slice(&n.to_be_bytes());
        let mut offset = 132 + tags.len() * 12;
        let mut data = Vec::new();
        for (sig, t) in tags {
            out.extend_from_slice(sig);
            out.extend_from_slice(&u32::try_from(offset).unwrap_or(0).to_be_bytes());
            out.extend_from_slice(&u32::try_from(t.len()).unwrap_or(0).to_be_bytes());
            data.extend_from_slice(t);
            while data.len() % 4 != 0 {
                data.push(0);
            }
            offset = 132 + tags.len() * 12 + data.len();
        }
        out.extend_from_slice(&data);
        let len = u32::try_from(out.len()).unwrap_or(0);
        out[0..4].copy_from_slice(&len.to_be_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn identifies_builtin_spaces() {
        for id in [
            ColorSpaceId::Srgb,
            ColorSpaceId::DisplayP3,
            ColorSpaceId::OpRgb,
            ColorSpaceId::ProPhotoRgb,
        ] {
            let p = rgb_profile(id, 2.2, "Test profile");
            let info = identify(&p).unwrap();
            assert_eq!(info.kind, ProfileKind::Rgb);
            assert_eq!(info.primaries, Some(id), "{id:?}");
            assert_eq!(info.description.as_deref(), Some("Test profile"));
            assert!(matches!(info.curve, Some(ToneCurve::Gamma(g)) if (g - 2.2).abs() < 0.01));
        }
        let lin = identify(&rgb_profile(ColorSpaceId::Srgb, 1.0, "lin")).unwrap();
        assert_eq!(lin.curve, Some(ToneCurve::Linear));
    }

    #[test]
    fn classifies_tables_and_parametric_curves() {
        let table: Vec<f64> = (0..1024)
            .map(|i| srgb_decode(f64::from(i) / 1023.0))
            .collect();
        assert_eq!(classify_table(&table), ToneCurve::Srgb);
        let lin: Vec<f64> = (0..256).map(|i| f64::from(i) / 255.0).collect();
        assert_eq!(classify_table(&lin), ToneCurve::Linear);
        let g: Vec<f64> = (0..256).map(|i| (f64::from(i) / 255.0).powf(1.8)).collect();
        assert_eq!(classify_table(&g), ToneCurve::Other);
    }

    #[test]
    fn rejects_bad_data() {
        assert!(identify(b"not a profile").is_none());
        let p = rgb_profile(ColorSpaceId::Srgb, 2.2, "x");
        for n in 0..p.len() {
            let _ = identify(&p[..n]);
        }
        let mut bad = p.clone();
        // Huge tag count.
        bad[128..132].copy_from_slice(&u32::MAX.to_be_bytes());
        let info = identify(&bad).unwrap();
        assert_eq!(info.primaries, None);
    }
}
