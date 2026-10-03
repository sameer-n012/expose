//! Develop parameter types (non-destructive edit settings).
//!
//! This module has data types only. It does no image processing. The
//! develop pipeline reads these settings and renders the image.
//!
//! # Schema evolution
//!
//! All structs use `#[serde(default)]`, and no struct uses
//! `deny_unknown_fields`. This gives these results:
//!
//! - **Older settings** (with fewer fields) load: missing fields get their
//!   default (identity) value.
//! - **Newer settings** (with more fields) load: unknown fields are ignored.
//!   [`DevelopSettings::validate`] then rejects a `version` that is newer
//!   than [`DEVELOP_SETTINGS_VERSION`], so that the caller knows that the
//!   render can be different.
//! - An unknown **enum variant** (for example a new white balance preset)
//!   is a deserialization error. It is not replaced silently.
//!
//! The structs are not `#[non_exhaustive]`. That attribute changes only the
//! Rust API (other crates cannot use struct literals), and it has no effect
//! on serde. Other crates can construct settings with
//! `..Default::default()`.
//!
//! # Validation
//!
//! [`DevelopSettings::validate`] uses two rules:
//!
//! - **Clamp:** a finite slider value outside its range is clamped to the
//!   range (see [`ranges`]).
//! - **Error:** a NaN or infinite value, an invalid tone curve, an invalid
//!   crop rectangle, an invalid aspect ratio, or an unsupported version
//!   gives a [`CoreError`].

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// The current schema version of [`DevelopSettings`].
pub const DEVELOP_SETTINGS_VERSION: u32 = 1;

/// The maximum number of control points in one tone curve channel.
pub const MAX_CURVE_POINTS: usize = 32;

/// Permitted ranges of the develop sliders. `validate` clamps to these.
pub mod ranges {
    use std::ops::RangeInclusive;

    /// White balance temperature, in kelvin. This is the range of the
    /// Planckian locus helper, less the lowest part.
    pub const TEMPERATURE: RangeInclusive<f32> = 2000.0..=25000.0;
    /// White balance tint. Positive is magenta, negative is green.
    pub const TINT: RangeInclusive<f32> = -150.0..=150.0;
    /// Exposure, in EV (stops).
    pub const EXPOSURE: RangeInclusive<f32> = -5.0..=5.0;
    /// Contrast, highlights, shadows, whites, and blacks.
    pub const TONE: RangeInclusive<f32> = -100.0..=100.0;
    /// Crop straighten angle, in degrees.
    pub const CROP_ANGLE: RangeInclusive<f32> = -45.0..=45.0;
    /// Sharpening amount.
    pub const SHARPEN_AMOUNT: RangeInclusive<f32> = 0.0..=150.0;
    /// Sharpening radius, in pixels.
    pub const SHARPEN_RADIUS: RangeInclusive<f32> = 0.5..=3.0;
    /// Sharpening detail.
    pub const SHARPEN_DETAIL: RangeInclusive<f32> = 0.0..=100.0;
    /// Sharpening masking.
    pub const SHARPEN_MASKING: RangeInclusive<f32> = 0.0..=100.0;
}

/// Returns an error if `value` is not finite. Otherwise clamps it to `range`.
fn clamp_finite(
    name: &'static str,
    value: &mut f32,
    range: &RangeInclusive<f32>,
) -> CoreResult<()> {
    if !value.is_finite() {
        return Err(CoreError::NonFinite { name });
    }
    *value = value.clamp(*range.start(), *range.end());
    Ok(())
}

/// All develop settings of one photo. `Default` is the identity edit (no
/// change to the image).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DevelopSettings {
    /// Schema version. New settings use [`DEVELOP_SETTINGS_VERSION`].
    pub version: u32,
    /// White balance.
    pub white_balance: WhiteBalance,
    /// Exposure and basic tone sliders.
    pub tone: BasicTone,
    /// Point tone curve.
    pub tone_curve: ToneCurve,
    /// Crop, straighten, and flip.
    pub crop: Crop,
    /// Sharpening.
    pub sharpening: Sharpening,
}

impl Default for DevelopSettings {
    fn default() -> Self {
        Self {
            version: DEVELOP_SETTINGS_VERSION,
            white_balance: WhiteBalance::default(),
            tone: BasicTone::default(),
            tone_curve: ToneCurve::default(),
            crop: Crop::default(),
            sharpening: Sharpening::default(),
        }
    }
}

impl DevelopSettings {
    /// Checks all values. Clamps finite out-of-range slider values, and
    /// returns an error for values that cannot be repaired (see the module
    /// documentation).
    ///
    /// If the function returns an error, `self` is not changed.
    pub fn validate(&mut self) -> CoreResult<()> {
        if self.version == 0 {
            return Err(CoreError::OutOfRange {
                name: "version",
                value: 0.0,
                min: 1.0,
                max: f64::from(DEVELOP_SETTINGS_VERSION),
            });
        }
        if self.version > DEVELOP_SETTINGS_VERSION {
            return Err(CoreError::UnsupportedVersion {
                found: self.version,
                supported: DEVELOP_SETTINGS_VERSION,
            });
        }
        // Work on a copy, so that an error leaves `self` unchanged.
        let mut out = self.clone();
        out.white_balance.validate()?;
        out.tone.validate()?;
        out.tone_curve.validate()?;
        out.crop.validate()?;
        out.sharpening.validate()?;
        *self = out;
        Ok(())
    }
}

/// White balance presets with nominal temperature and tint values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhiteBalancePreset {
    /// Direct sunlight (5500 K).
    Daylight,
    /// Overcast sky (6500 K).
    Cloudy,
    /// Open shade (7500 K).
    Shade,
    /// Incandescent light (2850 K).
    Tungsten,
    /// Fluorescent light (3800 K, tint +10).
    Fluorescent,
    /// Camera flash (5500 K).
    Flash,
}

impl WhiteBalancePreset {
    /// Returns the nominal `(temperature in kelvin, tint)` of the preset.
    pub const fn temperature_tint(self) -> (f32, f32) {
        match self {
            Self::Daylight | Self::Flash => (5500.0, 0.0),
            Self::Cloudy => (6500.0, 0.0),
            Self::Shade => (7500.0, 0.0),
            Self::Tungsten => (2850.0, 0.0),
            Self::Fluorescent => (3800.0, 10.0),
        }
    }
}

/// The source of the white balance values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhiteBalanceMode {
    /// Use the white balance that the camera recorded. The pipeline ignores
    /// `temperature` and `tint`.
    #[default]
    AsShot,
    /// Use the values of a preset. The pipeline ignores `temperature` and
    /// `tint`.
    Preset(WhiteBalancePreset),
    /// Use `temperature` and `tint`.
    Custom,
}

/// White balance settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WhiteBalance {
    /// Source of the values. Default: [`WhiteBalanceMode::AsShot`].
    pub mode: WhiteBalanceMode,
    /// Temperature in kelvin, in [`ranges::TEMPERATURE`]. Default: 5500.
    pub temperature: f32,
    /// Tint, in [`ranges::TINT`]. Positive is magenta. Default: 0.
    pub tint: f32,
}

impl Default for WhiteBalance {
    fn default() -> Self {
        Self {
            mode: WhiteBalanceMode::AsShot,
            temperature: 5500.0,
            tint: 0.0,
        }
    }
}

impl WhiteBalance {
    fn validate(&mut self) -> CoreResult<()> {
        clamp_finite(
            "white_balance.temperature",
            &mut self.temperature,
            &ranges::TEMPERATURE,
        )?;
        clamp_finite("white_balance.tint", &mut self.tint, &ranges::TINT)
    }
}

/// Exposure and basic tone sliders. All defaults are 0 (no change).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BasicTone {
    /// Exposure in EV (stops), in [`ranges::EXPOSURE`].
    pub exposure: f32,
    /// Contrast, in [`ranges::TONE`].
    pub contrast: f32,
    /// Highlights, in [`ranges::TONE`].
    pub highlights: f32,
    /// Shadows, in [`ranges::TONE`].
    pub shadows: f32,
    /// Whites (white clipping point), in [`ranges::TONE`].
    pub whites: f32,
    /// Blacks (black clipping point), in [`ranges::TONE`].
    pub blacks: f32,
}

impl BasicTone {
    fn validate(&mut self) -> CoreResult<()> {
        clamp_finite("tone.exposure", &mut self.exposure, &ranges::EXPOSURE)?;
        clamp_finite("tone.contrast", &mut self.contrast, &ranges::TONE)?;
        clamp_finite("tone.highlights", &mut self.highlights, &ranges::TONE)?;
        clamp_finite("tone.shadows", &mut self.shadows, &ranges::TONE)?;
        clamp_finite("tone.whites", &mut self.whites, &ranges::TONE)?;
        clamp_finite("tone.blacks", &mut self.blacks, &ranges::TONE)
    }
}

/// One control point of a tone curve. Both coordinates are in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct CurvePoint {
    /// Input value.
    pub x: f32,
    /// Output value.
    pub y: f32,
}

impl CurvePoint {
    /// Makes a control point.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Returns the identity curve: `(0, 0)` and `(1, 1)`.
pub fn identity_curve() -> Vec<CurvePoint> {
    vec![CurvePoint::new(0.0, 0.0), CurvePoint::new(1.0, 1.0)]
}

/// Point tone curve with one channel for all colors and one per color.
///
/// Each channel has 2 to [`MAX_CURVE_POINTS`] points. Coordinates are in
/// `0.0..=1.0`, and `x` values are strictly increasing. The default for each
/// channel is [`identity_curve`]. The interpolation method is a pipeline
/// detail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToneCurve {
    /// Curve for all three channels.
    pub master: Vec<CurvePoint>,
    /// Curve for the red channel.
    pub red: Vec<CurvePoint>,
    /// Curve for the green channel.
    pub green: Vec<CurvePoint>,
    /// Curve for the blue channel.
    pub blue: Vec<CurvePoint>,
}

impl Default for ToneCurve {
    fn default() -> Self {
        Self {
            master: identity_curve(),
            red: identity_curve(),
            green: identity_curve(),
            blue: identity_curve(),
        }
    }
}

impl ToneCurve {
    /// Returns an error if a channel breaks the rules of [`ToneCurve`]. This
    /// function does not clamp, because clamping can make two points equal.
    pub fn validate(&self) -> CoreResult<()> {
        validate_curve("master", &self.master)?;
        validate_curve("red", &self.red)?;
        validate_curve("green", &self.green)?;
        validate_curve("blue", &self.blue)
    }
}

fn validate_curve(channel: &'static str, points: &[CurvePoint]) -> CoreResult<()> {
    let err = |reason| Err(CoreError::InvalidToneCurve { channel, reason });
    if points.len() < 2 {
        return err("fewer than 2 points");
    }
    if points.len() > MAX_CURVE_POINTS {
        return err("more than the maximum number of points");
    }
    let unit = 0.0..=1.0;
    for p in points {
        if !p.x.is_finite() || !p.y.is_finite() {
            return err("point is not finite");
        }
        if !unit.contains(&p.x) || !unit.contains(&p.y) {
            return err("point is outside 0..=1");
        }
    }
    if points.windows(2).any(|w| w[1].x <= w[0].x) {
        return err("x values are not strictly increasing");
    }
    Ok(())
}

/// A rectangle in normalized image coordinates (`0.0..=1.0` on each axis,
/// origin at the top left).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NormalizedRect {
    /// Left edge.
    pub left: f32,
    /// Top edge.
    pub top: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

impl NormalizedRect {
    /// The full image.
    pub const FULL: Self = Self {
        left: 0.0,
        top: 0.0,
        width: 1.0,
        height: 1.0,
    };
}

impl Default for NormalizedRect {
    fn default() -> Self {
        Self::FULL
    }
}

/// The aspect ratio constraint of the crop tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CropAspect {
    /// No constraint.
    Free,
    /// The aspect ratio of the original photo.
    #[default]
    Original,
    /// A fixed ratio `width : height` (for example 3:2 or 16:9). Both values
    /// must be greater than 0.
    Ratio {
        /// Width part of the ratio.
        width: u32,
        /// Height part of the ratio.
        height: u32,
    },
}

/// Crop, straighten, and flip settings.
///
/// The intended order of operations is: flip, then rotate by `angle` about
/// the image center, then crop to `rect` in the rotated frame. The pipeline
/// defines the exact geometry. `aspect` is a constraint for the crop tool;
/// `validate` does not check that `rect` agrees with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Crop {
    /// Crop rectangle. Default: [`NormalizedRect::FULL`].
    pub rect: NormalizedRect,
    /// Straighten angle in degrees, in [`ranges::CROP_ANGLE`]. Positive is
    /// clockwise. Default: 0.
    pub angle: f32,
    /// Mirror left to right. Default: `false`.
    pub flip_horizontal: bool,
    /// Mirror top to bottom. Default: `false`.
    pub flip_vertical: bool,
    /// Aspect ratio constraint. Default: [`CropAspect::Original`].
    pub aspect: CropAspect,
}

impl Default for Crop {
    fn default() -> Self {
        Self {
            rect: NormalizedRect::FULL,
            angle: 0.0,
            flip_horizontal: false,
            flip_vertical: false,
            aspect: CropAspect::Original,
        }
    }
}

impl Crop {
    /// Clamps the angle and the rectangle into the image. Returns an error
    /// for non-finite values, for a width or height that is not greater
    /// than 0 after clamping, and for a ratio with a zero part.
    fn validate(&mut self) -> CoreResult<()> {
        clamp_finite("crop.angle", &mut self.angle, &ranges::CROP_ANGLE)?;
        if let CropAspect::Ratio { width, height } = self.aspect
            && (width == 0 || height == 0)
        {
            return Err(CoreError::InvalidCrop {
                reason: "aspect ratio part is zero",
            });
        }
        let r = &mut self.rect;
        let unit = 0.0..=1.0;
        clamp_finite("crop.rect.left", &mut r.left, &unit)?;
        clamp_finite("crop.rect.top", &mut r.top, &unit)?;
        clamp_finite("crop.rect.width", &mut r.width, &(0.0..=1.0 - r.left))?;
        clamp_finite("crop.rect.height", &mut r.height, &(0.0..=1.0 - r.top))?;
        if r.width <= 0.0 || r.height <= 0.0 {
            return Err(CoreError::InvalidCrop {
                reason: "rectangle width or height is not greater than 0",
            });
        }
        Ok(())
    }
}

/// Sharpening settings. The default has amount 0 (no sharpening).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sharpening {
    /// Strength, in [`ranges::SHARPEN_AMOUNT`]. Default: 0.
    pub amount: f32,
    /// Radius in pixels, in [`ranges::SHARPEN_RADIUS`]. Default: 1.0.
    pub radius: f32,
    /// Detail (halo suppression), in [`ranges::SHARPEN_DETAIL`]. Default: 25.
    pub detail: f32,
    /// Edge masking, in [`ranges::SHARPEN_MASKING`]. Default: 0.
    pub masking: f32,
}

impl Default for Sharpening {
    fn default() -> Self {
        Self {
            amount: 0.0,
            radius: 1.0,
            detail: 25.0,
            masking: 0.0,
        }
    }
}

impl Sharpening {
    fn validate(&mut self) -> CoreResult<()> {
        clamp_finite(
            "sharpening.amount",
            &mut self.amount,
            &ranges::SHARPEN_AMOUNT,
        )?;
        clamp_finite(
            "sharpening.radius",
            &mut self.radius,
            &ranges::SHARPEN_RADIUS,
        )?;
        clamp_finite(
            "sharpening.detail",
            &mut self.detail,
            &ranges::SHARPEN_DETAIL,
        )?;
        clamp_finite(
            "sharpening.masking",
            &mut self.masking,
            &ranges::SHARPEN_MASKING,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DevelopSettings {
        DevelopSettings {
            white_balance: WhiteBalance {
                mode: WhiteBalanceMode::Preset(WhiteBalancePreset::Cloudy),
                temperature: 6500.0,
                tint: 5.0,
            },
            tone: BasicTone {
                exposure: 0.7,
                contrast: 20.0,
                highlights: -40.0,
                shadows: 30.0,
                whites: 5.0,
                blacks: -10.0,
            },
            tone_curve: ToneCurve {
                master: vec![
                    CurvePoint::new(0.0, 0.05),
                    CurvePoint::new(0.5, 0.55),
                    CurvePoint::new(1.0, 1.0),
                ],
                ..ToneCurve::default()
            },
            crop: Crop {
                rect: NormalizedRect {
                    left: 0.1,
                    top: 0.2,
                    width: 0.5,
                    height: 0.6,
                },
                angle: -2.5,
                flip_horizontal: true,
                flip_vertical: false,
                aspect: CropAspect::Ratio {
                    width: 3,
                    height: 2,
                },
            },
            sharpening: Sharpening {
                amount: 40.0,
                ..Sharpening::default()
            },
            ..DevelopSettings::default()
        }
    }

    #[test]
    fn default_is_identity_and_valid() {
        let mut d = DevelopSettings::default();
        assert_eq!(d.version, DEVELOP_SETTINGS_VERSION);
        assert_eq!(d.white_balance.mode, WhiteBalanceMode::AsShot);
        assert_eq!(d.tone, BasicTone::default());
        assert_eq!(d.tone_curve.master, identity_curve());
        assert_eq!(d.crop.rect, NormalizedRect::FULL);
        assert!(d.sharpening.amount.abs() < f32::EPSILON);
        let before = d.clone();
        d.validate().unwrap();
        assert_eq!(d, before);
    }

    #[test]
    fn json_round_trip() {
        let s = sample();
        let json = serde_json::to_string(&s).unwrap();
        let back: DevelopSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        let d = DevelopSettings::default();
        let back: DevelopSettings =
            serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn json_shape_is_readable() {
        let json = serde_json::to_value(sample()).unwrap();
        assert_eq!(json["white_balance"]["mode"]["preset"], "cloudy");
        assert_eq!(json["crop"]["aspect"]["ratio"]["width"], 3);
        let d = serde_json::to_value(DevelopSettings::default()).unwrap();
        assert_eq!(d["white_balance"]["mode"], "as_shot");
    }

    #[test]
    fn missing_fields_get_defaults() {
        let back: DevelopSettings =
            serde_json::from_str(r#"{"version":1,"tone":{"exposure":1.5}}"#).unwrap();
        assert!((back.tone.exposure - 1.5).abs() < f32::EPSILON);
        assert_eq!(back.tone.contrast.to_bits(), 0.0_f32.to_bits());
        assert_eq!(back.crop, Crop::default());
        let empty: DevelopSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, DevelopSettings::default());
    }

    #[test]
    fn unknown_fields_are_ignored_but_newer_version_is_rejected() {
        let mut d: DevelopSettings = serde_json::from_str(
            r#"{"version":2,"future_panel":{"x":1},"tone":{"exposure":1.0,"future":3}}"#,
        )
        .unwrap();
        assert!((d.tone.exposure - 1.0).abs() < f32::EPSILON);
        assert_eq!(
            d.validate(),
            Err(CoreError::UnsupportedVersion {
                found: 2,
                supported: DEVELOP_SETTINGS_VERSION
            })
        );
        d.version = 0;
        assert!(matches!(d.validate(), Err(CoreError::OutOfRange { .. })));
    }

    #[test]
    fn unknown_enum_variant_is_an_error() {
        let r = serde_json::from_str::<DevelopSettings>(
            r#"{"white_balance":{"mode":{"preset":"moonlight"}}}"#,
        );
        assert!(r.is_err());
    }

    #[test]
    fn validate_clamps_slider_values() {
        let mut d = sample();
        d.tone.exposure = 9.0;
        d.tone.blacks = -300.0;
        d.white_balance.temperature = 100.0;
        d.sharpening.radius = 0.0;
        d.crop.angle = 90.0;
        d.validate().unwrap();
        assert!((d.tone.exposure - 5.0).abs() < f32::EPSILON);
        assert!((d.tone.blacks + 100.0).abs() < f32::EPSILON);
        assert!((d.white_balance.temperature - 2000.0).abs() < f32::EPSILON);
        assert!((d.sharpening.radius - 0.5).abs() < f32::EPSILON);
        assert!((d.crop.angle - 45.0).abs() < f32::EPSILON);
    }

    #[test]
    fn validate_rejects_non_finite_and_keeps_input() {
        let mut d = sample();
        d.tone.exposure = 9.0;
        d.sharpening.amount = f32::NAN;
        let before_exposure = d.tone.exposure;
        assert_eq!(
            d.validate(),
            Err(CoreError::NonFinite {
                name: "sharpening.amount"
            })
        );
        // The error leaves the settings unchanged (exposure is not clamped).
        assert!((d.tone.exposure - before_exposure).abs() < f32::EPSILON);
    }

    #[test]
    fn tone_curve_rules() {
        let mut c = ToneCurve::default();
        assert!(c.validate().is_ok());
        c.red = vec![CurvePoint::new(0.0, 0.0)];
        assert!(matches!(
            c.validate(),
            Err(CoreError::InvalidToneCurve { channel: "red", .. })
        ));
        c.red = vec![
            CurvePoint::new(0.0, 0.0),
            CurvePoint::new(0.6, 0.5),
            CurvePoint::new(0.6, 0.7),
            CurvePoint::new(1.0, 1.0),
        ];
        assert!(c.validate().is_err(), "duplicate x");
        c.red = vec![CurvePoint::new(0.5, 0.0), CurvePoint::new(0.2, 1.0)];
        assert!(c.validate().is_err(), "unsorted");
        c.red = vec![CurvePoint::new(0.0, -0.1), CurvePoint::new(1.0, 1.0)];
        assert!(c.validate().is_err(), "outside 0..=1");
        c.red = vec![CurvePoint::new(0.0, f32::NAN), CurvePoint::new(1.0, 1.0)];
        assert!(c.validate().is_err(), "NaN");
        c.red = (0..=MAX_CURVE_POINTS)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let x = i as f32 / MAX_CURVE_POINTS as f32;
                CurvePoint::new(x, x)
            })
            .collect();
        assert!(c.validate().is_err(), "too many points");
        c.red.pop();
        assert!(c.validate().is_ok());
    }

    #[test]
    fn crop_rules() {
        let mut d = sample();
        d.crop.rect = NormalizedRect {
            left: -0.5,
            top: 0.5,
            width: 2.0,
            height: 0.8,
        };
        d.validate().unwrap();
        assert_eq!(
            d.crop.rect,
            NormalizedRect {
                left: 0.0,
                top: 0.5,
                width: 1.0,
                height: 0.5
            }
        );
        d.crop.rect.width = 0.0;
        assert!(matches!(d.validate(), Err(CoreError::InvalidCrop { .. })));
        d.crop.rect = NormalizedRect {
            left: 1.0,
            ..NormalizedRect::FULL
        };
        assert!(matches!(d.validate(), Err(CoreError::InvalidCrop { .. })));
        d.crop.rect = NormalizedRect::FULL;
        d.crop.aspect = CropAspect::Ratio {
            width: 0,
            height: 1,
        };
        assert!(matches!(d.validate(), Err(CoreError::InvalidCrop { .. })));
    }

    #[test]
    fn presets_are_in_range() {
        for p in [
            WhiteBalancePreset::Daylight,
            WhiteBalancePreset::Cloudy,
            WhiteBalancePreset::Shade,
            WhiteBalancePreset::Tungsten,
            WhiteBalancePreset::Fluorescent,
            WhiteBalancePreset::Flash,
        ] {
            let (t, tint) = p.temperature_tint();
            assert!(ranges::TEMPERATURE.contains(&t));
            assert!(ranges::TINT.contains(&tint));
        }
    }
}
