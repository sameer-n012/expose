//! The transport types of the editor commands and their conversion to and
//! from the `expose-editor` types.
//!
//! The UI types are in `ui/src/lib/editor/api.ts`. Keep the two in sync.
//! All names are camelCase. Enums are tagged with the field `type`.

use expose_catalog::PhotoId;
use expose_core::develop::CurvePoint;
use expose_editor::{
    Adjustment, Background, BitDepth, BlendMode, BlendSpace, BrightnessContrast, ColorMode, Curves,
    Document, DocumentSpec, GroupIsolation, HueSaturation, Layer, LayerKind, Levels, LevelsChannel,
    Locks, MAX_DIMENSION, MAX_PIXELS, SelectionMode, SelectionShape,
};
use serde::{Deserialize, Serialize};

use crate::error::CommandError;

/// The identifier of an open document. It is a process counter, so it is
/// never used two times.
pub type DocumentId = u64;

/// A layer blend mode (the 27 modes of [`BlendMode`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    missing_docs,
    reason = "the variants have the names of the BlendMode variants"
)]
pub enum BlendModeId {
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

impl From<BlendMode> for BlendModeId {
    fn from(mode: BlendMode) -> Self {
        match mode {
            BlendMode::Normal => Self::Normal,
            BlendMode::Dissolve => Self::Dissolve,
            BlendMode::Darken => Self::Darken,
            BlendMode::Multiply => Self::Multiply,
            BlendMode::ColorBurn => Self::ColorBurn,
            BlendMode::LinearBurn => Self::LinearBurn,
            BlendMode::DarkerColor => Self::DarkerColor,
            BlendMode::Lighten => Self::Lighten,
            BlendMode::Screen => Self::Screen,
            BlendMode::ColorDodge => Self::ColorDodge,
            BlendMode::LinearDodge => Self::LinearDodge,
            BlendMode::LighterColor => Self::LighterColor,
            BlendMode::Overlay => Self::Overlay,
            BlendMode::SoftLight => Self::SoftLight,
            BlendMode::HardLight => Self::HardLight,
            BlendMode::VividLight => Self::VividLight,
            BlendMode::LinearLight => Self::LinearLight,
            BlendMode::PinLight => Self::PinLight,
            BlendMode::HardMix => Self::HardMix,
            BlendMode::Difference => Self::Difference,
            BlendMode::Exclusion => Self::Exclusion,
            BlendMode::Subtract => Self::Subtract,
            BlendMode::Divide => Self::Divide,
            BlendMode::Hue => Self::Hue,
            BlendMode::Saturation => Self::Saturation,
            BlendMode::Color => Self::Color,
            BlendMode::Luminosity => Self::Luminosity,
        }
    }
}

impl From<BlendModeId> for BlendMode {
    fn from(mode: BlendModeId) -> Self {
        match mode {
            BlendModeId::Normal => Self::Normal,
            BlendModeId::Dissolve => Self::Dissolve,
            BlendModeId::Darken => Self::Darken,
            BlendModeId::Multiply => Self::Multiply,
            BlendModeId::ColorBurn => Self::ColorBurn,
            BlendModeId::LinearBurn => Self::LinearBurn,
            BlendModeId::DarkerColor => Self::DarkerColor,
            BlendModeId::Lighten => Self::Lighten,
            BlendModeId::Screen => Self::Screen,
            BlendModeId::ColorDodge => Self::ColorDodge,
            BlendModeId::LinearDodge => Self::LinearDodge,
            BlendModeId::LighterColor => Self::LighterColor,
            BlendModeId::Overlay => Self::Overlay,
            BlendModeId::SoftLight => Self::SoftLight,
            BlendModeId::HardLight => Self::HardLight,
            BlendModeId::VividLight => Self::VividLight,
            BlendModeId::LinearLight => Self::LinearLight,
            BlendModeId::PinLight => Self::PinLight,
            BlendModeId::HardMix => Self::HardMix,
            BlendModeId::Difference => Self::Difference,
            BlendModeId::Exclusion => Self::Exclusion,
            BlendModeId::Subtract => Self::Subtract,
            BlendModeId::Divide => Self::Divide,
            BlendModeId::Hue => Self::Hue,
            BlendModeId::Saturation => Self::Saturation,
            BlendModeId::Color => Self::Color,
            BlendModeId::Luminosity => Self::Luminosity,
        }
    }
}

/// One channel of a levels adjustment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelsChannelDto {
    /// Input black point.
    pub input_black: f32,
    /// Input white point.
    pub input_white: f32,
    /// Midtone gamma.
    pub gamma: f32,
    /// Output black.
    pub output_black: f32,
    /// Output white.
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

/// One control point of a curve. Both coordinates are in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurvePointDto {
    /// Input value.
    pub x: f32,
    /// Output value.
    pub y: f32,
}

fn points_to_dto(points: &[CurvePoint]) -> Vec<CurvePointDto> {
    points
        .iter()
        .map(|p| CurvePointDto { x: p.x, y: p.y })
        .collect()
}

fn points_from_dto(points: Vec<CurvePointDto>) -> Vec<CurvePoint> {
    points
        .into_iter()
        .map(|p| CurvePoint::new(p.x, p.y))
        .collect()
}

/// The setting of an adjustment layer. The editor crate validates the
/// values when the setting is applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[allow(missing_docs, reason = "the fields are the fields of the editor types")]
pub enum AdjustmentDto {
    /// Brightness and contrast.
    BrightnessContrast { brightness: f32, contrast: f32 },
    /// Levels.
    Levels {
        master: LevelsChannelDto,
        red: LevelsChannelDto,
        green: LevelsChannelDto,
        blue: LevelsChannelDto,
    },
    /// Curves.
    Curves {
        master: Vec<CurvePointDto>,
        red: Vec<CurvePointDto>,
        green: Vec<CurvePointDto>,
        blue: Vec<CurvePointDto>,
    },
    /// Hue, saturation, and lightness.
    HueSaturation {
        hue: f32,
        saturation: f32,
        lightness: f32,
    },
}

impl AdjustmentDto {
    /// The default name of a new layer with this adjustment.
    pub const fn layer_name(&self) -> &'static str {
        match self {
            Self::BrightnessContrast { .. } => "Brightness/Contrast",
            Self::Levels { .. } => "Levels",
            Self::Curves { .. } => "Curves",
            Self::HueSaturation { .. } => "Hue/Saturation",
        }
    }
}

impl From<&Adjustment> for AdjustmentDto {
    fn from(adjustment: &Adjustment) -> Self {
        match adjustment {
            Adjustment::BrightnessContrast(a) => Self::BrightnessContrast {
                brightness: a.brightness,
                contrast: a.contrast,
            },
            Adjustment::Levels(a) => Self::Levels {
                master: a.master.into(),
                red: a.red.into(),
                green: a.green.into(),
                blue: a.blue.into(),
            },
            Adjustment::Curves(a) => Self::Curves {
                master: points_to_dto(&a.master),
                red: points_to_dto(&a.red),
                green: points_to_dto(&a.green),
                blue: points_to_dto(&a.blue),
            },
            Adjustment::HueSaturation(a) => Self::HueSaturation {
                hue: a.hue,
                saturation: a.saturation,
                lightness: a.lightness,
            },
        }
    }
}

impl From<AdjustmentDto> for Adjustment {
    fn from(dto: AdjustmentDto) -> Self {
        match dto {
            AdjustmentDto::BrightnessContrast {
                brightness,
                contrast,
            } => Self::BrightnessContrast(BrightnessContrast {
                brightness,
                contrast,
            }),
            AdjustmentDto::Levels {
                master,
                red,
                green,
                blue,
            } => Self::Levels(Levels {
                master: master.into(),
                red: red.into(),
                green: green.into(),
                blue: blue.into(),
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
                hue,
                saturation,
                lightness,
            }),
        }
    }
}

/// The kind of a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayerKindId {
    /// A raster layer.
    Pixel,
    /// An adjustment layer.
    Adjustment,
    /// A group.
    Group,
}

/// The four locks of a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    clippy::struct_excessive_bools,
    reason = "one flag for each lock button"
)]
pub struct LocksDto {
    /// Pixel writes must keep the alpha of each pixel.
    pub transparency: bool,
    /// Pixel and mask writes are not permitted.
    pub pixels: bool,
    /// The layer cannot move.
    pub position: bool,
    /// All locks, and property changes.
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

/// The content of the first layer of a new document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundId {
    /// Opaque white.
    White,
    /// Opaque black.
    Black,
    /// Transparent.
    Transparent,
}

/// The longest name of a new document, in characters.
pub const MAX_NAME_CHARS: usize = 120;

/// The settings of `editor_new`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewDocumentSpec {
    /// The document name. It becomes a file stem.
    pub name: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Resolution in pixels per inch.
    pub resolution_ppi: f32,
    /// Bit depth: 8, 16, or 32.
    pub bit_depth: u32,
    /// The first layer.
    pub background: BackgroundId,
}

impl NewDocumentSpec {
    /// Checks the settings and makes the document spec and the trimmed
    /// name.
    ///
    /// # Errors
    ///
    /// `invalidInput` for an empty or too long name, a name with `/`, `\`,
    /// `:` or a control character, a size out of range, a resolution out
    /// of range, or a bit depth other than 8, 16, 32.
    pub fn to_spec(&self) -> Result<(DocumentSpec, String), CommandError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(CommandError::invalid_input("the name is empty"));
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(CommandError::invalid_input("the name is too long"));
        }
        if name
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':') || c.is_control())
        {
            return Err(CommandError::invalid_input(
                "the name has a character that a file name cannot have",
            ));
        }
        let (width, height) = (self.width, self.height);
        if width == 0
            || height == 0
            || width > MAX_DIMENSION
            || height > MAX_DIMENSION
            || u64::from(width) * u64::from(height) > MAX_PIXELS
        {
            return Err(CommandError::invalid_input(
                "the size is out of range for a document",
            ));
        }
        if !self.resolution_ppi.is_finite() {
            return Err(CommandError::invalid_input("the resolution is not valid"));
        }
        let bit_depth = match self.bit_depth {
            8 => BitDepth::Eight,
            16 => BitDepth::Sixteen,
            32 => BitDepth::ThirtyTwo,
            _ => {
                return Err(CommandError::invalid_input(
                    "the bit depth must be 8, 16, or 32",
                ));
            }
        };
        let background = match self.background {
            BackgroundId::White => Background::White,
            BackgroundId::Black => Background::Black,
            BackgroundId::Transparent => Background::Transparent,
        };
        let spec = DocumentSpec {
            width,
            height,
            resolution_ppi: self.resolution_ppi,
            color_mode: ColorMode::Rgb,
            bit_depth,
            background,
            blend_space: BlendSpace::Perceptual,
        };
        Ok((spec, name.to_owned()))
    }
}

/// One layer of the tree that the UI shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerInfo {
    /// The layer ID ([`expose_editor::LayerId::get`]).
    pub id: u64,
    /// The layer name.
    pub name: String,
    /// The layer kind.
    pub kind: LayerKindId,
    /// `true` if the layer is shown.
    pub visible: bool,
    /// The opacity, `0.0..=1.0`.
    pub opacity: f32,
    /// The fill opacity, `0.0..=1.0`.
    pub fill_opacity: f32,
    /// The blend mode.
    pub blend_mode: BlendModeId,
    /// `true` if the layer has a mask.
    pub has_mask: bool,
    /// `true` if the layer has a mask and the mask is on. `false` if there
    /// is no mask.
    pub mask_enabled: bool,
    /// The own locks of the layer (not the locks that a parent group adds).
    pub locks: LocksDto,
    /// The setting of an adjustment layer; `null` for other kinds.
    pub adjustment: Option<AdjustmentDto>,
    /// For a group: `true` if it is isolated, `false` if it passes
    /// through. `null` for other kinds.
    pub isolated: Option<bool>,
    /// For a group: the children, bottom first. `null` for other kinds.
    pub children: Option<Vec<LayerInfo>>,
}

impl From<&Layer> for LayerInfo {
    fn from(layer: &Layer) -> Self {
        let (kind, adjustment, isolated, children) = match layer.kind() {
            LayerKind::Pixel(_) => (LayerKindId::Pixel, None, None, None),
            LayerKind::Adjustment(a) => (LayerKindId::Adjustment, Some(a.into()), None, None),
            LayerKind::Group(g) => (
                LayerKindId::Group,
                None,
                Some(g.isolation() == GroupIsolation::Isolated),
                Some(layer_tree(g.children())),
            ),
        };
        Self {
            id: layer.id().get(),
            name: layer.name().to_owned(),
            kind,
            visible: layer.visible(),
            opacity: layer.opacity(),
            fill_opacity: layer.fill_opacity(),
            blend_mode: layer.blend_mode().into(),
            has_mask: layer.mask().is_some(),
            mask_enabled: layer.mask().is_some_and(|m| m.enabled),
            locks: layer.locks().into(),
            adjustment,
            isolated,
            children,
        }
    }
}

/// Converts a layer list (bottom first) to the transport form. The order
/// stays bottom first.
pub fn layer_tree(layers: &[Layer]) -> Vec<LayerInfo> {
    layers.iter().map(LayerInfo::from).collect()
}

/// The state of an open document that the UI shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the fields are the UI contract"
)]
pub struct DocumentInfo {
    /// The document ID.
    pub document_id: DocumentId,
    /// The canvas width in pixels.
    pub width: u32,
    /// The canvas height in pixels.
    pub height: u32,
    /// The root layers, bottom first.
    pub layers: Vec<LayerInfo>,
    /// `true` if an undo is possible.
    pub can_undo: bool,
    /// `true` if a redo is possible.
    pub can_redo: bool,
    /// The document revision ([`Document::revision`]). It changes at each
    /// edit, undo, and redo. The value is a stamp from a process counter
    /// that increases by one, so it stays far below 2^53, and a JSON number
    /// in the UI holds it exactly.
    pub revision: u64,
    /// `true` if the revision is not the revision of the last save (or of
    /// the open, if the document has no save in this session).
    pub dirty: bool,
    /// The library photo of the flattened image of the last save, or
    /// `None` if the document has no saved files yet.
    pub saved_photo_id: Option<PhotoId>,
    /// The library photo that the document was first opened from, or
    /// `None` (a new document).
    pub source_photo_id: Option<PhotoId>,
    /// `true` if the document has a selection.
    pub has_selection: bool,
    /// The selection version ([`Document::selection_version`]). It changes
    /// at each change of the selection, undo, and redo. The value is a
    /// stamp from a process counter, so it stays far below 2^53.
    pub selection_version: u64,
}

impl DocumentInfo {
    /// Makes the info of `document` with the ID `id`. The info is not
    /// dirty and has no saved photo; [`crate::editor::EditSession::info`]
    /// sets these two fields.
    pub fn new(id: DocumentId, document: &Document, can_undo: bool, can_redo: bool) -> Self {
        Self {
            document_id: id,
            width: document.width(),
            height: document.height(),
            layers: layer_tree(document.layers()),
            can_undo,
            can_redo,
            revision: document.revision(),
            dirty: false,
            saved_photo_id: None,
            source_photo_id: None,
            has_selection: document.selection().is_some(),
            selection_version: document.selection_version(),
        }
    }
}

/// The outline of the selection of a document (result of
/// `editor_selection_outline`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionOutline {
    /// The selection version of the outlines.
    pub selection_version: u64,
    /// The sample step that was used (can be larger than the requested
    /// step).
    pub step: u32,
    /// Closed outlines. Each is a flat list `[x0, y0, x1, y1, ...]` in
    /// canvas pixels.
    pub contours: Vec<Vec<f32>>,
}

/// A selection shape in canvas pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[allow(missing_docs, reason = "the field names explain the fields")]
pub enum SelectionShapeDto {
    /// A rectangle.
    Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },
    /// An ellipse in the given box.
    Ellipse {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },
    /// A closed polygon.
    Polygon { points: Vec<[f64; 2]> },
}

impl From<SelectionShapeDto> for SelectionShape {
    fn from(shape: SelectionShapeDto) -> Self {
        match shape {
            SelectionShapeDto::Rect {
                x,
                y,
                width,
                height,
            } => Self::Rect {
                x,
                y,
                width,
                height,
            },
            SelectionShapeDto::Ellipse {
                x,
                y,
                width,
                height,
            } => Self::Ellipse {
                x,
                y,
                width,
                height,
            },
            SelectionShapeDto::Polygon { points } => Self::Polygon {
                points: points.into_iter().map(|[x, y]| (x, y)).collect(),
            },
        }
    }
}

/// How a new selection shape combines with the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SelectionModeId {
    /// The shape replaces the selection.
    Replace,
    /// The shape adds to the selection.
    Add,
    /// The shape removes from the selection.
    Subtract,
    /// The selection keeps the part inside the shape.
    Intersect,
}

impl From<SelectionModeId> for SelectionMode {
    fn from(mode: SelectionModeId) -> Self {
        match mode {
            SelectionModeId::Replace => Self::Replace,
            SelectionModeId::Add => Self::Add,
            SelectionModeId::Subtract => Self::Subtract,
            SelectionModeId::Intersect => Self::Intersect,
        }
    }
}

/// The result of `editor_save`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult {
    /// The library photo of the flattened image.
    pub photo_id: PhotoId,
    /// The absolute path of the layered document file.
    pub document_path: String,
    /// The absolute path of the flattened image file.
    pub image_path: String,
    /// `true` if this save made the two files and the library photo;
    /// `false` if it replaced the files of an earlier save.
    pub created: bool,
    /// The document info after the save.
    pub info: DocumentInfo,
}

/// One edit from the UI. Layer IDs are the values of [`LayerInfo::id`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[allow(missing_docs, reason = "the field names explain the fields")]
pub enum EditDto {
    /// Shows or hides a layer.
    SetVisible { layer_id: u64, visible: bool },
    /// Sets the opacity.
    SetOpacity { layer_id: u64, opacity: f32 },
    /// Sets the fill opacity.
    SetFillOpacity { layer_id: u64, fill_opacity: f32 },
    /// Sets the blend mode.
    SetBlendMode { layer_id: u64, mode: BlendModeId },
    /// Renames a layer.
    Rename { layer_id: u64, name: String },
    /// Moves a layer. `parent_id` `null` is the root list. `index` is in
    /// the bottom-first child list, after the layer is removed.
    Move {
        layer_id: u64,
        parent_id: Option<u64>,
        index: usize,
    },
    /// Deletes a layer (and its children).
    Delete { layer_id: u64 },
    /// Copies a layer and puts the copy directly above it.
    Duplicate { layer_id: u64 },
    /// Adds an adjustment layer directly above `above_layer_id` (in the
    /// same parent), or at the top of the root list for `null`.
    AddAdjustment {
        adjustment: AdjustmentDto,
        above_layer_id: Option<u64>,
    },
    /// Replaces the setting of an adjustment layer.
    SetAdjustment {
        layer_id: u64,
        adjustment: AdjustmentDto,
    },
    /// Adds a transparent pixel layer of the canvas size directly above
    /// `above_layer_id` (in the same parent), or at the top of the root
    /// list for `null`.
    AddPixelLayer { above_layer_id: Option<u64> },
    /// Sets the own locks of a layer.
    SetLocks { layer_id: u64, locks: LocksDto },
    /// Adds a mask of the canvas size. `reveal`: all 1.0 (white). `hide`:
    /// all 0.0 (black). It is an error if the layer has a mask.
    AddMask { layer_id: u64, fill: MaskFillId },
    /// Removes the mask of a layer.
    DeleteMask { layer_id: u64 },
    /// Turns the mask of a layer on or off.
    SetMaskEnabled { layer_id: u64, enabled: bool },
    /// Combines a shape with the selection. `feather` is the blur radius
    /// in pixels.
    SelectShape {
        shape: SelectionShapeDto,
        mode: SelectionModeId,
        anti_alias: bool,
        feather: f32,
    },
    /// Selects the whole canvas.
    SelectAll,
    /// Removes the selection. It is an error if there is no selection.
    Deselect,
    /// Inverts the selection. It is an error if there is no selection.
    InvertSelection,
    /// Blurs the edge of the selection. It is an error if there is no
    /// selection.
    FeatherSelection { radius: f32 },
    /// Sets the mask of a layer from the selection. It is an error if
    /// there is no selection.
    MaskFromSelection { layer_id: u64 },
}

/// The start state of a new layer mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MaskFillId {
    /// The mask shows all of the layer.
    Reveal,
    /// The mask hides all of the layer.
    Hide,
}

impl EditDto {
    /// The name of the history step of this edit.
    pub const fn step_name(&self) -> &'static str {
        match self {
            Self::SetVisible { .. } => "Layer Visibility",
            Self::SetOpacity { .. } => "Layer Opacity",
            Self::SetFillOpacity { .. } => "Fill Opacity",
            Self::SetBlendMode { .. } => "Blend Mode",
            Self::Rename { .. } => "Rename Layer",
            Self::Move { .. } => "Move Layer",
            Self::Delete { .. } => "Delete Layer",
            Self::Duplicate { .. } => "Duplicate Layer",
            Self::AddAdjustment { .. } => "New Adjustment Layer",
            Self::SetAdjustment { .. } => "Adjustment Settings",
            Self::AddPixelLayer { .. } => "New Layer",
            Self::SetLocks { .. } => "Layer Locks",
            Self::AddMask { .. } => "Add Layer Mask",
            Self::DeleteMask { .. } => "Delete Layer Mask",
            Self::SetMaskEnabled { enabled: true, .. } => "Enable Layer Mask",
            Self::SetMaskEnabled { enabled: false, .. } => "Disable Layer Mask",
            Self::SelectShape { .. } => "Select",
            Self::SelectAll => "Select All",
            Self::Deselect => "Deselect",
            Self::InvertSelection => "Invert Selection",
            Self::FeatherSelection { .. } => "Feather Selection",
            Self::MaskFromSelection { .. } => "Mask from Selection",
        }
    }
}

#[cfg(test)]
mod tests {
    // Test code: a failed setup is a test failure.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use expose_editor::{DocumentSpec, LayerContent, Placement};
    use serde_json::json;

    use super::*;

    /// The blend mode names of the UI type `BlendModeId`, in the order of
    /// [`BlendMode::ALL`].
    const UI_NAMES: [&str; 27] = [
        "normal",
        "dissolve",
        "darken",
        "multiply",
        "colorBurn",
        "linearBurn",
        "darkerColor",
        "lighten",
        "screen",
        "colorDodge",
        "linearDodge",
        "lighterColor",
        "overlay",
        "softLight",
        "hardLight",
        "vividLight",
        "linearLight",
        "pinLight",
        "hardMix",
        "difference",
        "exclusion",
        "subtract",
        "divide",
        "hue",
        "saturation",
        "color",
        "luminosity",
    ];

    #[test]
    fn all_blend_modes_round_trip_with_ui_names() {
        for (mode, name) in BlendMode::ALL.into_iter().zip(UI_NAMES) {
            let id = BlendModeId::from(mode);
            assert_eq!(BlendMode::from(id), mode);
            assert_eq!(serde_json::to_value(id).unwrap(), json!(name));
            let parsed: BlendModeId = serde_json::from_value(json!(name)).unwrap();
            assert_eq!(parsed, id);
        }
        assert!(serde_json::from_value::<BlendModeId>(json!("ColorBurn")).is_err());
    }

    fn levels_channel(k: f32) -> LevelsChannelDto {
        LevelsChannelDto {
            input_black: 0.1 * k,
            input_white: 0.9,
            gamma: 1.0 + k,
            output_black: 0.05,
            output_white: 1.0 - 0.1 * k,
        }
    }

    fn sample_adjustments() -> Vec<AdjustmentDto> {
        let curve = |y: f32| {
            vec![
                CurvePointDto { x: 0.0, y: 0.0 },
                CurvePointDto { x: 0.5, y },
                CurvePointDto { x: 1.0, y: 1.0 },
            ]
        };
        vec![
            AdjustmentDto::BrightnessContrast {
                brightness: 0.25,
                contrast: -0.5,
            },
            AdjustmentDto::Levels {
                master: levels_channel(0.0),
                red: levels_channel(1.0),
                green: levels_channel(2.0),
                blue: levels_channel(3.0),
            },
            AdjustmentDto::Curves {
                master: curve(0.6),
                red: curve(0.4),
                green: curve(0.5),
                blue: curve(0.7),
            },
            AdjustmentDto::HueSaturation {
                hue: 30.0,
                saturation: -0.25,
                lightness: 0.5,
            },
        ]
    }

    #[test]
    fn adjustments_round_trip_through_editor_types() {
        for dto in sample_adjustments() {
            let adjustment = Adjustment::from(dto.clone());
            adjustment.validate().unwrap();
            assert_eq!(AdjustmentDto::from(&adjustment), dto);
            let text = serde_json::to_string(&dto).unwrap();
            assert_eq!(serde_json::from_str::<AdjustmentDto>(&text).unwrap(), dto);
        }
    }

    #[test]
    fn adjustment_json_uses_ui_names() {
        let value = serde_json::to_value(AdjustmentDto::BrightnessContrast {
            brightness: 0.5,
            contrast: 0.0,
        })
        .unwrap();
        assert_eq!(
            value,
            json!({ "type": "brightnessContrast", "brightness": 0.5, "contrast": 0.0 })
        );
        let levels = serde_json::to_value(&sample_adjustments()[1]).unwrap();
        assert_eq!(levels["type"], "levels");
        assert_eq!(levels["master"]["inputBlack"], 0.0);
        assert_eq!(levels["red"]["outputWhite"], 0.9_f32);
        let hs: AdjustmentDto = serde_json::from_value(
            json!({ "type": "hueSaturation", "hue": 10, "saturation": 0, "lightness": 0 }),
        )
        .unwrap();
        assert_eq!(
            hs,
            AdjustmentDto::HueSaturation {
                hue: 10.0,
                saturation: 0.0,
                lightness: 0.0
            }
        );
    }

    #[test]
    fn edits_parse_from_ui_json() {
        let cases = [
            (
                json!({ "type": "setFillOpacity", "layerId": 3, "fillOpacity": 0.5 }),
                EditDto::SetFillOpacity {
                    layer_id: 3,
                    fill_opacity: 0.5,
                },
            ),
            (
                json!({ "type": "setBlendMode", "layerId": 1, "mode": "linearDodge" }),
                EditDto::SetBlendMode {
                    layer_id: 1,
                    mode: BlendModeId::LinearDodge,
                },
            ),
            (
                json!({ "type": "move", "layerId": 2, "parentId": null, "index": 0 }),
                EditDto::Move {
                    layer_id: 2,
                    parent_id: None,
                    index: 0,
                },
            ),
            (
                json!({
                    "type": "addAdjustment",
                    "adjustment": { "type": "brightnessContrast", "brightness": 0, "contrast": 0 },
                    "aboveLayerId": 7
                }),
                EditDto::AddAdjustment {
                    adjustment: AdjustmentDto::BrightnessContrast {
                        brightness: 0.0,
                        contrast: 0.0,
                    },
                    above_layer_id: Some(7),
                },
            ),
        ];
        for (value, expected) in cases {
            assert_eq!(serde_json::from_value::<EditDto>(value).unwrap(), expected);
        }
        assert!(
            serde_json::from_value::<EditDto>(json!({ "type": "paint", "layerId": 1 })).is_err()
        );
    }

    #[test]
    fn tree_is_bottom_first_with_group_children() {
        let mut doc = Document::new(DocumentSpec::new(4, 4)).unwrap();
        let background = doc.layer_ids()[0];
        let (group, _) = doc
            .add_layer(
                "Group",
                LayerContent::Group(GroupIsolation::Isolated),
                Placement::root(1),
            )
            .unwrap();
        let (adjust, _) = doc
            .add_layer(
                "Hue",
                LayerContent::Adjustment(Adjustment::HueSaturation(HueSaturation::default())),
                Placement::new(Some(group), 0),
            )
            .unwrap();
        doc.set_blend_mode(adjust, BlendMode::Screen).unwrap();
        let info = DocumentInfo::new(9, &doc, true, false);
        assert_eq!(info.document_id, 9);
        assert_eq!((info.width, info.height), (4, 4));
        assert_eq!(info.revision, doc.revision());
        assert!(!info.has_selection);
        assert_eq!(info.selection_version, doc.selection_version());
        assert_eq!(info.layers.len(), 2);
        assert_eq!(info.layers[0].id, background.get());
        assert_eq!(info.layers[0].kind, LayerKindId::Pixel);
        assert_eq!(info.layers[0].name, "Background");
        assert_eq!(info.layers[0].children, None);
        assert_eq!(info.layers[1].kind, LayerKindId::Group);
        assert_eq!(info.layers[1].isolated, Some(true));
        let children = info.layers[1].children.as_ref().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id, adjust.get());
        assert_eq!(children[0].blend_mode, BlendModeId::Screen);
        assert!(matches!(
            children[0].adjustment,
            Some(AdjustmentDto::HueSaturation { .. })
        ));

        let value = serde_json::to_value(&info).unwrap();
        assert_eq!(value["documentId"], 9);
        assert_eq!(value["canUndo"], true);
        assert_eq!(value["canRedo"], false);
        assert_eq!(value["dirty"], false);
        assert!(value["savedPhotoId"].is_null());
        let bottom = &value["layers"][0];
        assert_eq!(bottom["kind"], "pixel");
        assert_eq!(bottom["fillOpacity"], 1.0);
        assert_eq!(bottom["blendMode"], "normal");
        assert_eq!(bottom["hasMask"], false);
        assert_eq!(bottom["maskEnabled"], false);
        assert!(bottom["adjustment"].is_null());
        assert!(bottom["isolated"].is_null());
        assert!(bottom["children"].is_null());
        assert!(value["sourcePhotoId"].is_null());
        assert_eq!(
            bottom["locks"],
            json!({ "transparency": false, "pixels": false, "position": false, "all": false })
        );
    }

    #[test]
    fn new_layer_and_lock_edits_parse() {
        let edit: EditDto =
            serde_json::from_value(json!({ "type": "addPixelLayer", "aboveLayerId": 3 })).unwrap();
        assert_eq!(
            edit,
            EditDto::AddPixelLayer {
                above_layer_id: Some(3)
            }
        );
        let edit: EditDto =
            serde_json::from_value(json!({ "type": "addPixelLayer", "aboveLayerId": null }))
                .unwrap();
        assert_eq!(
            edit,
            EditDto::AddPixelLayer {
                above_layer_id: None
            }
        );
        let edit: EditDto = serde_json::from_value(json!({
            "type": "setLocks",
            "layerId": 4,
            "locks": { "transparency": false, "pixels": true, "position": false, "all": false }
        }))
        .unwrap();
        let EditDto::SetLocks { layer_id, locks } = edit else {
            panic!("wrong variant");
        };
        assert_eq!(layer_id, 4);
        assert!(locks.pixels && !locks.all);
        assert!(Locks::from(locks).pixels);
    }

    #[test]
    fn selection_edits_parse_and_have_step_names() {
        let edit: EditDto = serde_json::from_value(json!({
            "type": "selectShape",
            "shape": {"type": "polygon", "points": [[0, 0], [10, 0], [5, 8]]},
            "mode": "add",
            "antiAlias": true,
            "feather": 0
        }))
        .unwrap();
        assert_eq!(
            edit,
            EditDto::SelectShape {
                shape: SelectionShapeDto::Polygon {
                    points: vec![[0.0, 0.0], [10.0, 0.0], [5.0, 8.0]]
                },
                mode: SelectionModeId::Add,
                anti_alias: true,
                feather: 0.0,
            }
        );
        assert_eq!(edit.step_name(), "Select");
        let rect: EditDto = serde_json::from_value(json!({
            "type": "selectShape",
            "shape": {"type": "rect", "x": 1, "y": 2, "width": 3, "height": 4},
            "mode": "intersect",
            "antiAlias": false,
            "feather": 2.5
        }))
        .unwrap();
        assert!(matches!(
            rect,
            EditDto::SelectShape {
                shape: SelectionShapeDto::Rect { .. },
                mode: SelectionModeId::Intersect,
                ..
            }
        ));
        let ellipse: SelectionShapeDto = serde_json::from_value(
            json!({"type": "ellipse", "x": 0, "y": 0, "width": 2, "height": 2}),
        )
        .unwrap();
        assert!(matches!(ellipse, SelectionShapeDto::Ellipse { .. }));
        // Unit variants of an internally tagged enum.
        for (text, expected, name) in [
            ("selectAll", EditDto::SelectAll, "Select All"),
            ("deselect", EditDto::Deselect, "Deselect"),
            (
                "invertSelection",
                EditDto::InvertSelection,
                "Invert Selection",
            ),
        ] {
            let parsed: EditDto = serde_json::from_value(json!({ "type": text })).unwrap();
            assert_eq!(parsed, expected);
            assert_eq!(parsed.step_name(), name);
            assert_eq!(
                serde_json::to_value(&parsed).unwrap(),
                json!({"type": text})
            );
        }
        let feather: EditDto =
            serde_json::from_value(json!({"type": "featherSelection", "radius": 4.5})).unwrap();
        assert_eq!(feather, EditDto::FeatherSelection { radius: 4.5 });
        assert_eq!(feather.step_name(), "Feather Selection");
        let mask: EditDto =
            serde_json::from_value(json!({"type": "maskFromSelection", "layerId": 3})).unwrap();
        assert_eq!(mask, EditDto::MaskFromSelection { layer_id: 3 });
        assert_eq!(mask.step_name(), "Mask from Selection");
    }

    #[test]
    fn selection_outline_serializes_in_camel_case() {
        let outline = SelectionOutline {
            selection_version: 7,
            step: 2,
            contours: vec![vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0]],
        };
        assert_eq!(
            serde_json::to_value(&outline).unwrap(),
            json!({"selectionVersion": 7, "step": 2, "contours": [[0.0, 0.0, 1.0, 0.0, 1.0, 1.0]]})
        );
    }

    #[test]
    fn mask_edits_parse_and_have_step_names() {
        let edit: EditDto =
            serde_json::from_value(json!({ "type": "addMask", "layerId": 2, "fill": "reveal" }))
                .unwrap();
        assert_eq!(
            edit,
            EditDto::AddMask {
                layer_id: 2,
                fill: MaskFillId::Reveal
            }
        );
        assert_eq!(edit.step_name(), "Add Layer Mask");
        let edit: EditDto =
            serde_json::from_value(json!({ "type": "addMask", "layerId": 2, "fill": "hide" }))
                .unwrap();
        assert!(matches!(
            edit,
            EditDto::AddMask {
                fill: MaskFillId::Hide,
                ..
            }
        ));
        let edit: EditDto =
            serde_json::from_value(json!({ "type": "deleteMask", "layerId": 2 })).unwrap();
        assert_eq!(edit, EditDto::DeleteMask { layer_id: 2 });
        assert_eq!(edit.step_name(), "Delete Layer Mask");
        let edit: EditDto = serde_json::from_value(
            json!({ "type": "setMaskEnabled", "layerId": 2, "enabled": false }),
        )
        .unwrap();
        assert_eq!(edit.step_name(), "Disable Layer Mask");
        let back = serde_json::to_value(&edit).unwrap();
        assert_eq!(
            back,
            json!({ "type": "setMaskEnabled", "layerId": 2, "enabled": false })
        );
        let on = EditDto::SetMaskEnabled {
            layer_id: 2,
            enabled: true,
        };
        assert_eq!(on.step_name(), "Enable Layer Mask");
        // A bad fill name is a parse error.
        assert!(
            serde_json::from_value::<EditDto>(
                json!({ "type": "addMask", "layerId": 2, "fill": "white" })
            )
            .is_err()
        );
    }

    fn new_spec() -> NewDocumentSpec {
        serde_json::from_value(json!({
            "name": " Poster ",
            "width": 800,
            "height": 600,
            "resolutionPpi": 300.0,
            "bitDepth": 8,
            "background": "transparent"
        }))
        .unwrap()
    }

    #[test]
    fn new_document_spec_parses_and_converts() {
        let spec = new_spec();
        assert_eq!(spec.background, BackgroundId::Transparent);
        let (document, name) = spec.to_spec().unwrap();
        assert_eq!(name, "Poster");
        assert_eq!(document.bit_depth, BitDepth::Eight);
        assert_eq!(document.background, Background::Transparent);
        assert_eq!((document.width, document.height), (800, 600));
    }

    #[test]
    fn new_document_spec_rejects_bad_values() {
        let bad: Vec<NewDocumentSpec> = vec![
            NewDocumentSpec {
                name: "  ".into(),
                ..new_spec()
            },
            NewDocumentSpec {
                name: "a/b".into(),
                ..new_spec()
            },
            NewDocumentSpec {
                name: "a\\b".into(),
                ..new_spec()
            },
            NewDocumentSpec {
                name: "a:b".into(),
                ..new_spec()
            },
            NewDocumentSpec {
                name: "a\nb".into(),
                ..new_spec()
            },
            NewDocumentSpec {
                name: "x".repeat(MAX_NAME_CHARS + 1),
                ..new_spec()
            },
            NewDocumentSpec {
                width: 0,
                ..new_spec()
            },
            NewDocumentSpec {
                height: MAX_DIMENSION + 1,
                ..new_spec()
            },
            NewDocumentSpec {
                width: MAX_DIMENSION,
                height: MAX_DIMENSION,
                ..new_spec()
            },
            NewDocumentSpec {
                bit_depth: 12,
                ..new_spec()
            },
            NewDocumentSpec {
                resolution_ppi: f32::NAN,
                ..new_spec()
            },
        ];
        for spec in bad {
            let error = spec.to_spec().unwrap_err();
            assert_eq!(
                error.kind,
                crate::error::ErrorKind::InvalidInput,
                "{spec:?}"
            );
        }
    }
}
