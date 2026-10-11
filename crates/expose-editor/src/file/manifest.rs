//! The JSON manifest of a document file: serde types and the conversions
//! to and from the document model.
//!
//! The model types do not use serde. These types are the file format, so
//! a change of a model type does not change the format by accident. All
//! field names and enum names are camelCase strings.

use serde::{Deserialize, Serialize};

use crate::adjust::{Adjustment, BrightnessContrast, Curves, HueSaturation, Levels, LevelsChannel};
use crate::blend::{BlendMode, BlendSpace};
use crate::document::{BitDepth, ColorMode};
use crate::layer::{GroupIsolation, Locks, MaskFill};
use expose_core::develop::CurvePoint;

/// The root object of the manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ManifestDto {
    /// Canvas settings.
    pub document: DocumentDto,
    /// Root layers, bottom first.
    pub layers: Vec<LayerDto>,
    /// The blob table. A blob index is an index in this list.
    pub blobs: Vec<BlobDto>,
}

/// Canvas settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DocumentDto {
    pub width: u32,
    pub height: u32,
    pub resolution_ppi: f32,
    pub color_mode: ColorModeDto,
    /// 8, 16, or 32.
    pub bit_depth: u32,
    pub blend_space: BlendSpaceDto,
    /// The value of the next new layer ID. Optional: if it is not there, or
    /// if it is too small, the reader uses the largest layer ID plus 1.
    #[serde(default)]
    pub next_layer_id: u64,
}

/// One layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LayerDto {
    pub id: u64,
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub fill_opacity: f32,
    pub blend_mode: BlendModeDto,
    pub locks: LocksDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<MaskDto>,
    pub content: ContentDto,
}

/// The locks of a layer.
// Four independent flags, the same as `Locks`.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LocksDto {
    #[serde(default)]
    pub transparency: bool,
    #[serde(default)]
    pub pixels: bool,
    #[serde(default)]
    pub position: bool,
    #[serde(default)]
    pub all: bool,
}

impl From<Locks> for LocksDto {
    fn from(l: Locks) -> Self {
        Self {
            transparency: l.transparency,
            pixels: l.pixels,
            position: l.position,
            all: l.all,
        }
    }
}

impl From<LocksDto> for Locks {
    fn from(l: LocksDto) -> Self {
        Self {
            transparency: l.transparency,
            pixels: l.pixels,
            position: l.position,
            all: l.all,
        }
    }
}

/// A layer mask.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MaskDto {
    /// Canvas position `[x, y]` of the top-left mask pixel.
    pub offset: [i32; 2],
    pub width: u32,
    pub height: u32,
    pub enabled: bool,
    pub fill: MaskFillDto,
    /// Index in the blob table. The blob holds `width * height` values.
    pub blob: usize,
}

/// The content of a layer. The `kind` field selects the variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(super) enum ContentDto {
    /// A raster layer.
    Pixel {
        /// Canvas position `[x, y]` of the top-left pixel.
        offset: [i32; 2],
        width: u32,
        height: u32,
        /// Index in the blob table. The blob holds `width * height * 4`
        /// values (premultiplied RGBA).
        blob: usize,
    },
    /// A group.
    Group {
        isolation: IsolationDto,
        /// Children, bottom first.
        children: Vec<LayerDto>,
    },
    /// An adjustment layer.
    Adjustment { adjustment: AdjustmentDto },
}

/// Adjustment parameters. The `type` field selects the variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(super) enum AdjustmentDto {
    BrightnessContrast {
        brightness: f32,
        contrast: f32,
    },
    Levels {
        master: LevelsChannelDto,
        red: LevelsChannelDto,
        green: LevelsChannelDto,
        blue: LevelsChannelDto,
    },
    /// Each curve is a list of `[x, y]` points.
    Curves {
        master: Vec<[f32; 2]>,
        red: Vec<[f32; 2]>,
        green: Vec<[f32; 2]>,
        blue: Vec<[f32; 2]>,
    },
    HueSaturation {
        hue: f32,
        saturation: f32,
        lightness: f32,
    },
}

/// One levels channel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LevelsChannelDto {
    pub input_black: f32,
    pub input_white: f32,
    pub gamma: f32,
    pub output_black: f32,
    pub output_white: f32,
}

impl From<LevelsChannel> for LevelsChannelDto {
    fn from(c: LevelsChannel) -> Self {
        Self {
            input_black: c.input_black,
            input_white: c.input_white,
            gamma: c.gamma,
            output_black: c.output_black,
            output_white: c.output_white,
        }
    }
}

impl From<LevelsChannelDto> for LevelsChannel {
    fn from(c: LevelsChannelDto) -> Self {
        Self {
            input_black: c.input_black,
            input_white: c.input_white,
            gamma: c.gamma,
            output_black: c.output_black,
            output_white: c.output_white,
        }
    }
}

fn points_to_dto(points: &[CurvePoint]) -> Vec<[f32; 2]> {
    points.iter().map(|p| [p.x, p.y]).collect()
}

fn points_from_dto(points: &[[f32; 2]]) -> Vec<CurvePoint> {
    points.iter().map(|p| CurvePoint::new(p[0], p[1])).collect()
}

impl From<&Adjustment> for AdjustmentDto {
    fn from(a: &Adjustment) -> Self {
        match a {
            Adjustment::BrightnessContrast(b) => Self::BrightnessContrast {
                brightness: b.brightness,
                contrast: b.contrast,
            },
            Adjustment::Levels(l) => Self::Levels {
                master: l.master.into(),
                red: l.red.into(),
                green: l.green.into(),
                blue: l.blue.into(),
            },
            Adjustment::Curves(c) => Self::Curves {
                master: points_to_dto(&c.master),
                red: points_to_dto(&c.red),
                green: points_to_dto(&c.green),
                blue: points_to_dto(&c.blue),
            },
            Adjustment::HueSaturation(h) => Self::HueSaturation {
                hue: h.hue,
                saturation: h.saturation,
                lightness: h.lightness,
            },
        }
    }
}

impl From<&AdjustmentDto> for Adjustment {
    /// The conversion does not validate. The reader calls
    /// [`Adjustment::validate`] on the result.
    fn from(a: &AdjustmentDto) -> Self {
        match a {
            AdjustmentDto::BrightnessContrast {
                brightness,
                contrast,
            } => Self::BrightnessContrast(BrightnessContrast {
                brightness: *brightness,
                contrast: *contrast,
            }),
            AdjustmentDto::Levels {
                master,
                red,
                green,
                blue,
            } => Self::Levels(Levels {
                master: (*master).into(),
                red: (*red).into(),
                green: (*green).into(),
                blue: (*blue).into(),
            }),
            AdjustmentDto::Curves {
                master,
                red,
                green,
                blue,
            } => Self::Curves(Curves {
                master: points_from_dto(master),
                red: points_from_dto(red),
                green: points_from_dto(green),
                blue: points_from_dto(blue),
            }),
            AdjustmentDto::HueSaturation {
                hue,
                saturation,
                lightness,
            } => Self::HueSaturation(HueSaturation {
                hue: *hue,
                saturation: *saturation,
                lightness: *lightness,
            }),
        }
    }
}

/// One entry of the blob table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BlobDto {
    /// Byte offset from the start of the blob section.
    pub offset: u64,
    /// Number of stored (compressed) bytes.
    pub compressed_length: u64,
    /// Number of bytes after decompression (4 bytes per `f32` value).
    pub raw_length: u64,
    /// CRC-32 (ISO-HDLC) of the stored (compressed) bytes.
    pub crc32: u32,
}

/// Defines a serde enum with camelCase names and the conversions to and
/// from a model enum with the same variant names.
macro_rules! mirror_enum {
    ($(#[$meta:meta])* $dto:ident <=> $model:ident { $($variant:ident),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub(super) enum $dto {
            $($variant),+
        }

        impl From<$model> for $dto {
            fn from(v: $model) -> Self {
                match v {
                    $($model::$variant => Self::$variant),+
                }
            }
        }

        impl From<$dto> for $model {
            fn from(v: $dto) -> Self {
                match v {
                    $($dto::$variant => Self::$variant),+
                }
            }
        }
    };
}

mirror_enum!(
    /// Color mode (`rgb`).
    ColorModeDto <=> ColorMode { Rgb }
);

mirror_enum!(
    /// Blending space (`linear`, `perceptual`).
    BlendSpaceDto <=> BlendSpace { Linear, Perceptual }
);

mirror_enum!(
    /// Group isolation (`passThrough`, `isolated`).
    IsolationDto <=> GroupIsolation { PassThrough, Isolated }
);

mirror_enum!(
    /// Mask fill (`white`, `black`).
    MaskFillDto <=> MaskFill { White, Black }
);

mirror_enum!(
    /// Blend mode (`normal`, `colorBurn`, ...).
    BlendModeDto <=> BlendMode {
        Normal,
        Dissolve,
        Darken,
        Multiply,
        ColorBurn,
        LinearBurn,
        DarkerColor,
        Lighten,
        Screen,
        ColorDodge,
        LinearDodge,
        LighterColor,
        Overlay,
        SoftLight,
        HardLight,
        VividLight,
        LinearLight,
        PinLight,
        HardMix,
        Difference,
        Exclusion,
        Subtract,
        Divide,
        Hue,
        Saturation,
        Color,
        Luminosity,
    }
);

/// Returns the manifest value of a bit depth.
pub(super) const fn bit_depth_to_dto(depth: BitDepth) -> u32 {
    depth.bits()
}

/// Returns the bit depth for a manifest value, or `None` if the value is
/// not 8, 16, or 32.
pub(super) const fn bit_depth_from_dto(bits: u32) -> Option<BitDepth> {
    match bits {
        8 => Some(BitDepth::Eight),
        16 => Some(BitDepth::Sixteen),
        32 => Some(BitDepth::ThirtyTwo),
        _ => None,
    }
}
