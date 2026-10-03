//! Layer types: IDs, properties, locks, masks, and layer content.
//!
//! Layers are in a tree. The document root and each group have an ordered
//! list of children. **Index 0 is the bottom layer.** The compositor
//! processes each list from index 0 up.
//!
//! The fields of [`Layer`] are private. Change a layer only through the
//! operations on [`Document`](crate::Document), so that the document can
//! enforce locks and make undo records.

use std::fmt;

use expose_core::buffer::{Rect, WorkingImage};

use crate::adjust::Adjustment;
use crate::blend::BlendMode;
use crate::document::check_size;
use crate::error::{EditorError, EditorResult};

/// A stable layer identifier.
///
/// A document gives each new layer a new ID. IDs are not used again in the
/// same document, also after a delete. A deleted layer that comes back by
/// undo keeps its old ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerId(pub(crate) u64);

impl LayerId {
    /// Returns the numeric value of the ID.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for LayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// The kind of a lock. Errors use it to name the lock that stops an
/// operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LockKind {
    /// Pixel writes must not change alpha.
    Transparency,
    /// Pixel and mask writes are not permitted.
    Pixels,
    /// Moves (offset changes) are not permitted.
    Position,
    /// All of the above, and changes to opacity, fill opacity, blend mode,
    /// group isolation, and adjustment parameters.
    All,
}

impl fmt::Display for LockKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Transparency => "transparency",
            Self::Pixels => "pixels",
            Self::Position => "position",
            Self::All => "all",
        };
        f.write_str(name)
    }
}

/// The locks of a layer.
///
/// The locks of a group apply also to all layers in the group. These
/// operations are always permitted, also with `all`: rename, show/hide,
/// change locks, reorder in the layer stack, duplicate, and delete.
// Four independent flags, one for each lock button in the layers panel.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Locks {
    /// Pixel writes must keep the alpha of each pixel.
    pub transparency: bool,
    /// Pixel and mask writes are not permitted.
    pub pixels: bool,
    /// The layer cannot move (offset changes).
    pub position: bool,
    /// All locks, and also property changes (opacity, fill, blend mode,
    /// group isolation, adjustment parameters).
    pub all: bool,
}

impl Locks {
    /// No locks.
    pub const NONE: Self = Self {
        transparency: false,
        pixels: false,
        position: false,
        all: false,
    };

    /// Returns the union of two lock sets.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self {
            transparency: self.transparency || other.transparency,
            pixels: self.pixels || other.pixels,
            position: self.position || other.position,
            all: self.all || other.all,
        }
    }

    /// Returns `Err(Locked)` if `kind` (or `all`) is set.
    pub(crate) fn check(self, id: LayerId, kind: LockKind) -> EditorResult<()> {
        if self.all {
            return Err(EditorError::Locked {
                id,
                lock: LockKind::All,
            });
        }
        let set = match kind {
            LockKind::Transparency => self.transparency,
            LockKind::Pixels => self.pixels,
            LockKind::Position => self.position,
            LockKind::All => false,
        };
        if set {
            return Err(EditorError::Locked { id, lock: kind });
        }
        Ok(())
    }
}

/// The value of a mask outside its buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MaskFill {
    /// 1.0: the layer shows outside the mask buffer.
    #[default]
    White,
    /// 0.0: the layer is hidden outside the mask buffer.
    Black,
}

impl MaskFill {
    /// Returns the mask value (0.0 or 1.0).
    pub const fn value(self) -> f32 {
        match self {
            Self::White => 1.0,
            Self::Black => 0.0,
        }
    }
}

/// A one-channel `f32` image for masks. Row-major, no padding.
///
/// Mask values are in `0.0..=1.0` (1.0 shows the layer). The compositor
/// clamps values outside that range when it reads them.
#[derive(Debug, Clone, PartialEq)]
pub struct MaskImage {
    width: u32,
    height: u32,
    data: Vec<f32>,
}

impl MaskImage {
    /// Makes a mask image with all values set to `fill`.
    ///
    /// The size limits are the same as for layers
    /// ([`MAX_DIMENSION`](crate::MAX_DIMENSION),
    /// [`MAX_PIXELS`](crate::MAX_PIXELS)).
    pub fn new_filled(width: u32, height: u32, fill: f32) -> EditorResult<Self> {
        let count = check_size(width, height)?;
        Ok(Self {
            width,
            height,
            data: vec![fill; count],
        })
    }

    /// Makes a mask image from values in row-major order.
    pub fn from_data(width: u32, height: u32, data: Vec<f32>) -> EditorResult<Self> {
        let count = check_size(width, height)?;
        if data.len() != count {
            return Err(EditorError::LengthMismatch {
                expected: count,
                actual: data.len(),
            });
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    /// Returns the width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the rectangle `(0, 0, width, height)`.
    pub const fn bounds(&self) -> Rect {
        Rect::from_size(self.width, self.height)
    }

    /// Returns all values in row-major order.
    pub fn data(&self) -> &[f32] {
        &self.data
    }

    pub(crate) fn data_mut(&mut self) -> &mut [f32] {
        &mut self.data
    }

    /// Returns the value at `(x, y)`, or `None` if it is outside the image.
    pub fn get(&self, x: u32, y: u32) -> Option<f32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        // The constructor proved that width * height fits in usize.
        self.data
            .get(y as usize * self.width as usize + x as usize)
            .copied()
    }
}

/// A pixel mask of a layer.
///
/// The mask has its own offset in canvas coordinates. A move of the layer
/// moves the mask by the same amount (the mask is always linked).
#[derive(Debug, Clone, PartialEq)]
pub struct Mask {
    /// The mask values.
    pub image: MaskImage,
    /// Canvas position of the top-left mask pixel.
    pub offset: (i32, i32),
    /// If `false`, the compositor ignores the mask.
    pub enabled: bool,
    /// The value outside the mask buffer.
    pub fill: MaskFill,
}

impl Mask {
    /// Makes an enabled mask.
    pub const fn new(image: MaskImage, offset: (i32, i32), fill: MaskFill) -> Self {
        Self {
            image,
            offset,
            enabled: true,
            fill,
        }
    }

    /// Returns the mask value at canvas pixel `(x, y)`, clamped to
    /// `0.0..=1.0`. Returns 1.0 if the mask is disabled.
    pub fn value_at(&self, x: i64, y: i64) -> f32 {
        if !self.enabled {
            return 1.0;
        }
        let lx = x - i64::from(self.offset.0);
        let ly = y - i64::from(self.offset.1);
        let inside = match (u32::try_from(lx), u32::try_from(ly)) {
            (Ok(mx), Ok(my)) => self.image.get(mx, my),
            _ => None,
        };
        inside.unwrap_or(self.fill.value()).clamp(0.0, 1.0)
    }
}

/// A raster layer.
#[derive(Debug, Clone, PartialEq)]
pub struct PixelLayer {
    image: WorkingImage,
    offset: (i32, i32),
}

impl PixelLayer {
    /// Makes a pixel layer.
    ///
    /// - `image` must have **premultiplied** alpha (ADR 0002).
    /// - `offset` is the canvas position of the top-left pixel. The layer can
    ///   be smaller than the canvas or extend outside it.
    ///
    /// Returns [`EditorError::TooLarge`] if the image breaks the size limits.
    pub fn new(image: WorkingImage, offset: (i32, i32)) -> EditorResult<Self> {
        check_size(image.width(), image.height())?;
        Ok(Self { image, offset })
    }

    /// Returns the image (premultiplied alpha).
    pub const fn image(&self) -> &WorkingImage {
        &self.image
    }

    pub(crate) fn image_mut(&mut self) -> &mut WorkingImage {
        &mut self.image
    }

    /// Returns the canvas position of the top-left pixel.
    pub const fn offset(&self) -> (i32, i32) {
        self.offset
    }

    pub(crate) fn set_offset(&mut self, offset: (i32, i32)) {
        self.offset = offset;
    }
}

/// How a group composites its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GroupIsolation {
    /// The children composite directly onto the layers below the group, as
    /// if they were not in a group. Adjustment layers in the group thus also
    /// change the layers below the group. The group blend mode has no
    /// effect. The group opacity and mask mix the result with the state
    /// before the group.
    #[default]
    PassThrough,
    /// The children composite onto a transparent buffer. Then the
    /// compositor blends that buffer onto the layers below with the group
    /// blend mode, opacity, and mask (like one pixel layer). Isolated with
    /// the normal blend mode is the "normal" group mode.
    Isolated,
}

/// A group of layers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Group {
    children: Vec<Layer>,
    isolation: GroupIsolation,
}

impl Group {
    /// Returns the children, bottom first.
    pub fn children(&self) -> &[Layer] {
        &self.children
    }

    pub(crate) fn children_mut(&mut self) -> &mut Vec<Layer> {
        &mut self.children
    }

    /// Returns the isolation mode.
    pub const fn isolation(&self) -> GroupIsolation {
        self.isolation
    }

    pub(crate) fn set_isolation(&mut self, isolation: GroupIsolation) {
        self.isolation = isolation;
    }
}

/// The content of a layer.
#[derive(Debug, Clone, PartialEq)]
pub enum LayerKind {
    /// A raster layer.
    Pixel(PixelLayer),
    /// A group of layers.
    Group(Group),
    /// An adjustment layer. It changes the composite of the layers below
    /// it in the same group (see [`GroupIsolation`] for pass-through
    /// groups).
    Adjustment(Adjustment),
}

/// The content of a new layer (input to
/// [`Document::add_layer`](crate::Document::add_layer)).
#[derive(Debug, Clone, PartialEq)]
pub enum LayerContent {
    /// A raster layer.
    Pixel(PixelLayer),
    /// An empty group.
    Group(GroupIsolation),
    /// An adjustment layer.
    Adjustment(Adjustment),
}

/// The common properties of a layer.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerProps {
    /// Display name.
    pub name: String,
    /// If `false`, the compositor ignores the layer.
    pub visible: bool,
    /// Layer opacity, `0.0..=1.0`. It applies to the layer content and (in
    /// the future) to the layer effects.
    pub opacity: f32,
    /// Fill opacity, `0.0..=1.0`. It applies to the layer content only,
    /// not to layer effects.
    ///
    /// Layer effects (for example, drop shadow) do not exist yet. Thus the
    /// compositor now multiplies the content alpha by `opacity * fill`. When
    /// effects come, the compositor will draw the effects with `opacity`
    /// only, and the content with `opacity * fill`.
    pub fill_opacity: f32,
    /// Blend mode. Pass-through groups ignore it.
    pub blend_mode: BlendMode,
    /// Locks.
    pub locks: Locks,
}

impl LayerProps {
    /// Returns default properties with the given name: visible, opacity
    /// 1.0, fill 1.0, normal blend, no locks.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            visible: true,
            opacity: 1.0,
            fill_opacity: 1.0,
            blend_mode: BlendMode::Normal,
            locks: Locks::NONE,
        }
    }
}

/// One layer in the layer tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    id: LayerId,
    props: LayerProps,
    mask: Option<Mask>,
    kind: LayerKind,
}

impl Layer {
    pub(crate) fn new(id: LayerId, props: LayerProps, kind: LayerKind) -> Self {
        Self {
            id,
            props,
            mask: None,
            kind,
        }
    }

    /// Returns the ID.
    pub const fn id(&self) -> LayerId {
        self.id
    }

    pub(crate) fn set_id(&mut self, id: LayerId) {
        self.id = id;
    }

    /// Returns the common properties.
    pub const fn props(&self) -> &LayerProps {
        &self.props
    }

    pub(crate) fn props_mut(&mut self) -> &mut LayerProps {
        &mut self.props
    }

    /// Returns the name.
    pub fn name(&self) -> &str {
        &self.props.name
    }

    /// Returns `true` if the layer is visible.
    pub const fn visible(&self) -> bool {
        self.props.visible
    }

    /// Returns the opacity.
    pub const fn opacity(&self) -> f32 {
        self.props.opacity
    }

    /// Returns the fill opacity.
    pub const fn fill_opacity(&self) -> f32 {
        self.props.fill_opacity
    }

    /// Returns the blend mode.
    pub const fn blend_mode(&self) -> BlendMode {
        self.props.blend_mode
    }

    /// Returns the locks of this layer (without the locks of parent groups).
    pub const fn locks(&self) -> Locks {
        self.props.locks
    }

    /// Returns the mask, if the layer has one.
    pub const fn mask(&self) -> Option<&Mask> {
        self.mask.as_ref()
    }

    pub(crate) fn mask_mut(&mut self) -> Option<&mut Mask> {
        self.mask.as_mut()
    }

    pub(crate) fn replace_mask(&mut self, mask: Option<Mask>) -> Option<Mask> {
        std::mem::replace(&mut self.mask, mask)
    }

    /// Returns the content.
    pub const fn kind(&self) -> &LayerKind {
        &self.kind
    }

    pub(crate) fn kind_mut(&mut self) -> &mut LayerKind {
        &mut self.kind
    }

    /// Returns the children if the layer is a group, else an empty slice.
    pub fn children(&self) -> &[Layer] {
        match &self.kind {
            LayerKind::Group(g) => g.children(),
            _ => &[],
        }
    }

    /// Returns the pixel layer content, if this is a pixel layer.
    pub const fn as_pixel(&self) -> Option<&PixelLayer> {
        match &self.kind {
            LayerKind::Pixel(p) => Some(p),
            _ => None,
        }
    }

    /// Calls `f` for this layer and all descendants (depth first, bottom
    /// first).
    pub fn visit(&self, f: &mut impl FnMut(&Layer)) {
        f(self);
        for child in self.children() {
            child.visit(f);
        }
    }

    /// Calls `f` for this layer and all descendants, mutable.
    pub(crate) fn visit_mut(&mut self, f: &mut impl FnMut(&mut Layer)) {
        f(self);
        if let LayerKind::Group(g) = &mut self.kind {
            for child in g.children_mut() {
                child.visit_mut(f);
            }
        }
    }
}

/// A position in the layer tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Placement {
    /// The parent group, or `None` for the document root.
    pub parent: Option<LayerId>,
    /// The index in the list of children of the parent (0 is the bottom).
    /// For a move, the index is in the list after the layer is removed from
    /// its old position.
    pub index: usize,
}

impl Placement {
    /// Makes a placement.
    pub const fn new(parent: Option<LayerId>, index: usize) -> Self {
        Self { parent, index }
    }

    /// A placement at `index` in the document root.
    pub const fn root(index: usize) -> Self {
        Self::new(None, index)
    }
}
