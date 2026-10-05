//! The layered document: canvas settings and the layer tree.
//!
//! Change operations are in [`crate::ops`]. The compositor is in
//! [`crate::composite`].

use expose_core::buffer::{Rect, WorkingImage};

use crate::blend::BlendSpace;
use crate::error::{EditorError, EditorResult, check_range};
use crate::layer::{Layer, LayerId, LayerKind, LayerProps, Locks, PixelLayer, Placement};

/// The largest permitted width or height of a document, a layer, or a
/// mask, in pixels.
pub const MAX_DIMENSION: u32 = 30_000;

/// The largest permitted pixel count of a document, a layer, or a mask.
///
/// 2^28 pixels is 4 GiB for one full layer in the working format (16 bytes
/// per pixel). Layers are now dense buffers. A later sparse, tiled layer
/// storage can increase this limit.
pub const MAX_PIXELS: u64 = 1 << 28;

/// The lowest permitted resolution, in pixels per inch.
pub const MIN_RESOLUTION_PPI: f32 = 1.0;

/// The highest permitted resolution, in pixels per inch.
pub const MAX_RESOLUTION_PPI: f32 = 10_000.0;

/// Checks a buffer size against [`MAX_DIMENSION`] and [`MAX_PIXELS`] and
/// returns the pixel count.
pub(crate) fn check_size(width: u32, height: u32) -> EditorResult<usize> {
    if width == 0 || height == 0 {
        return Err(expose_core::CoreError::InvalidDimensions { width, height }.into());
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(EditorError::TooLarge {
            width,
            height,
            reason: "width or height is more than MAX_DIMENSION",
        });
    }
    let count = u64::from(width) * u64::from(height);
    if count > MAX_PIXELS {
        return Err(EditorError::TooLarge {
            width,
            height,
            reason: "pixel count is more than MAX_PIXELS",
        });
    }
    // count <= 2^28, which fits in usize on all supported targets (32 and
    // 64 bit).
    usize::try_from(count).map_err(|_| EditorError::TooLarge {
        width,
        height,
        reason: "pixel count does not fit in usize",
    })
}

/// The color mode of a document.
///
/// Only RGB exists now. The working data is always linear Rec. 2020 RGBA
/// `f32` (ADR 0002). Other modes can come later as metadata on the same
/// working data: grayscale keeps `r = g = b` (each operation then writes
/// gray values), and the conversion to a gray or other output format
/// happens at export. Modes that need other channels (for example, CMYK)
/// need a separate decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ColorMode {
    /// RGB color.
    #[default]
    Rgb,
}

/// The bit depth of a document.
///
/// This is metadata only (for example, the default bit depth at export).
/// The working data is always `f32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BitDepth {
    /// 8 bits per channel.
    Eight,
    /// 16 bits per channel.
    #[default]
    Sixteen,
    /// 32-bit float per channel.
    ThirtyTwo,
}

impl BitDepth {
    /// Returns the number of bits per channel.
    pub const fn bits(self) -> u32 {
        match self {
            Self::Eight => 8,
            Self::Sixteen => 16,
            Self::ThirtyTwo => 32,
        }
    }
}

/// The content of the first layer of a new document.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Background {
    /// An opaque white layer.
    #[default]
    White,
    /// An opaque black layer.
    Black,
    /// A transparent layer.
    Transparent,
    /// An opaque layer with this straight linear Rec. 2020 color. Values
    /// must be finite.
    Color([f32; 3]),
}

/// The settings for a new document.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DocumentSpec {
    /// Width in pixels, `1..=MAX_DIMENSION`.
    pub width: u32,
    /// Height in pixels, `1..=MAX_DIMENSION`.
    pub height: u32,
    /// Resolution in pixels per inch, `MIN_RESOLUTION_PPI..=MAX_RESOLUTION_PPI`.
    pub resolution_ppi: f32,
    /// Color mode.
    pub color_mode: ColorMode,
    /// Bit depth (metadata).
    pub bit_depth: BitDepth,
    /// The first layer.
    pub background: Background,
    /// The blending space of the compositor.
    pub blend_space: BlendSpace,
}

impl DocumentSpec {
    /// Returns a spec with this size and the defaults: 300 PPI, RGB,
    /// 16 bits, white background, perceptual blending.
    pub const fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            resolution_ppi: 300.0,
            color_mode: ColorMode::Rgb,
            bit_depth: BitDepth::Sixteen,
            background: Background::White,
            blend_space: BlendSpace::Perceptual,
        }
    }
}

/// A layered document.
///
/// The canvas covers the pixels `(0, 0)..(width, height)`. Layers can
/// extend outside the canvas. The root layer list has index 0 at the
/// bottom.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    width: u32,
    height: u32,
    resolution_ppi: f32,
    color_mode: ColorMode,
    bit_depth: BitDepth,
    pub(crate) blend_space: BlendSpace,
    pub(crate) layers: Vec<Layer>,
    next_id: u64,
}

impl Document {
    /// Makes a document with one layer named "Background".
    ///
    /// The background layer is a normal pixel layer of canvas size with the
    /// fill from `spec.background` (transparent black for
    /// [`Background::Transparent`]). It has no locks.
    pub fn new(spec: DocumentSpec) -> EditorResult<Self> {
        check_size(spec.width, spec.height)?;
        check_range(
            "resolution_ppi",
            spec.resolution_ppi,
            MIN_RESOLUTION_PPI,
            MAX_RESOLUTION_PPI,
        )?;
        let fill = match spec.background {
            Background::White => [1.0, 1.0, 1.0, 1.0],
            Background::Black => [0.0, 0.0, 0.0, 1.0],
            Background::Transparent => [0.0; 4],
            Background::Color(c) => {
                if c.iter().any(|v| !v.is_finite()) {
                    return Err(EditorError::NonFinite {
                        name: "background color",
                    });
                }
                // Alpha is 1, so the premultiplied color is the color.
                [c[0], c[1], c[2], 1.0]
            }
        };
        let image = WorkingImage::new_filled(spec.width, spec.height, fill)?;
        let mut doc = Self {
            width: spec.width,
            height: spec.height,
            resolution_ppi: spec.resolution_ppi,
            color_mode: spec.color_mode,
            bit_depth: spec.bit_depth,
            blend_space: spec.blend_space,
            layers: Vec::new(),
            next_id: 1,
        };
        let id = doc.alloc_id();
        doc.layers.push(Layer::new(
            id,
            LayerProps::named("Background"),
            LayerKind::Pixel(PixelLayer::new(image, (0, 0))?),
        ));
        Ok(doc)
    }

    /// Makes a document with no layers. The composite is transparent.
    pub fn new_empty(spec: DocumentSpec) -> EditorResult<Self> {
        let mut doc = Self::new(DocumentSpec {
            background: Background::Transparent,
            ..spec
        })?;
        doc.layers.clear();
        Ok(doc)
    }

    /// Returns the canvas width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the canvas height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the canvas rectangle `(0, 0, width, height)`.
    pub const fn bounds(&self) -> Rect {
        Rect::from_size(self.width, self.height)
    }

    /// Returns the resolution in pixels per inch.
    pub const fn resolution_ppi(&self) -> f32 {
        self.resolution_ppi
    }

    /// Returns the color mode.
    pub const fn color_mode(&self) -> ColorMode {
        self.color_mode
    }

    /// Returns the bit depth (metadata).
    pub const fn bit_depth(&self) -> BitDepth {
        self.bit_depth
    }

    /// Returns the blending space of the compositor.
    pub const fn blend_space(&self) -> BlendSpace {
        self.blend_space
    }

    /// Returns the root layers, bottom first.
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// Returns the number of layers in the tree (groups and their children
    /// included).
    pub fn layer_count(&self) -> usize {
        let mut n = 0;
        for l in &self.layers {
            l.visit(&mut |_| n += 1);
        }
        n
    }

    /// Returns the layer with this ID.
    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        let path = self.find_path(id)?;
        layer_at(&self.layers, &path)
    }

    /// Returns `true` if a layer has this ID.
    pub fn contains(&self, id: LayerId) -> bool {
        self.find_path(id).is_some()
    }

    /// Returns the position of a layer in the tree.
    pub fn placement_of(&self, id: LayerId) -> Option<Placement> {
        let path = self.find_path(id)?;
        let (&index, parent_path) = path.split_last()?;
        let parent = if parent_path.is_empty() {
            None
        } else {
            Some(layer_at(&self.layers, parent_path)?.id())
        };
        Some(Placement { parent, index })
    }

    /// Returns the IDs of the groups that contain the layer, nearest first.
    pub fn ancestors(&self, id: LayerId) -> EditorResult<Vec<LayerId>> {
        let path = self.find_path(id).ok_or(EditorError::LayerNotFound(id))?;
        let mut out = Vec::with_capacity(path.len().saturating_sub(1));
        for len in (1..path.len()).rev() {
            if let Some(l) = layer_at(&self.layers, &path[..len]) {
                out.push(l.id());
            }
        }
        Ok(out)
    }

    /// Returns the locks of the layer combined with the locks of all groups
    /// that contain it.
    pub fn effective_locks(&self, id: LayerId) -> EditorResult<Locks> {
        let path = self.find_path(id).ok_or(EditorError::LayerNotFound(id))?;
        let mut locks = Locks::NONE;
        let mut list = self.layers.as_slice();
        for &i in &path {
            let l = list.get(i).ok_or(EditorError::LayerNotFound(id))?;
            locks = locks.union(l.locks());
            list = l.children();
        }
        Ok(locks)
    }

    /// Returns all layer IDs in depth-first order, bottom first (a group
    /// comes before its children).
    pub fn layer_ids(&self) -> Vec<LayerId> {
        let mut out = Vec::new();
        for l in &self.layers {
            l.visit(&mut |x| out.push(x.id()));
        }
        out
    }

    pub(crate) fn alloc_id(&mut self) -> LayerId {
        let id = LayerId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Makes sure that new IDs are larger than `id`.
    pub(crate) fn reserve_id(&mut self, id: LayerId) {
        if id.0 >= self.next_id {
            self.next_id = id.0 + 1;
        }
    }

    /// Returns the index path from the root to the layer.
    pub(crate) fn find_path(&self, id: LayerId) -> Option<Vec<usize>> {
        let mut path = Vec::new();
        find_path_in(&self.layers, id, &mut path).then_some(path)
    }

    pub(crate) fn layer_mut(&mut self, id: LayerId) -> EditorResult<&mut Layer> {
        let path = self.find_path(id).ok_or(EditorError::LayerNotFound(id))?;
        layer_at_mut(&mut self.layers, &path).ok_or(EditorError::LayerNotFound(id))
    }

    /// Returns the child list of a group, or the root list for `None`.
    pub(crate) fn children_mut(
        &mut self,
        parent: Option<LayerId>,
    ) -> EditorResult<&mut Vec<Layer>> {
        match parent {
            None => Ok(&mut self.layers),
            Some(pid) => match self.layer_mut(pid)?.kind_mut() {
                LayerKind::Group(g) => Ok(g.children_mut()),
                _ => Err(EditorError::NotAGroup(pid)),
            },
        }
    }

    /// Returns the number of children of a group, or of the root for `None`.
    pub(crate) fn child_count(&self, parent: Option<LayerId>) -> EditorResult<usize> {
        match parent {
            None => Ok(self.layers.len()),
            Some(pid) => match self
                .layer(pid)
                .ok_or(EditorError::LayerNotFound(pid))?
                .kind()
            {
                LayerKind::Group(g) => Ok(g.children().len()),
                _ => Err(EditorError::NotAGroup(pid)),
            },
        }
    }
}

fn find_path_in(list: &[Layer], id: LayerId, path: &mut Vec<usize>) -> bool {
    for (i, l) in list.iter().enumerate() {
        path.push(i);
        if l.id() == id || find_path_in(l.children(), id, path) {
            return true;
        }
        path.pop();
    }
    false
}

fn layer_at<'a>(list: &'a [Layer], path: &[usize]) -> Option<&'a Layer> {
    let (&first, rest) = path.split_first()?;
    let l = list.get(first)?;
    if rest.is_empty() {
        Some(l)
    } else {
        layer_at(l.children(), rest)
    }
}

fn layer_at_mut<'a>(list: &'a mut [Layer], path: &[usize]) -> Option<&'a mut Layer> {
    let (&first, rest) = path.split_first()?;
    let l = list.get_mut(first)?;
    if rest.is_empty() {
        return Some(l);
    }
    match l.kind_mut() {
        LayerKind::Group(g) => layer_at_mut(g.children_mut(), rest),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // Tests compare exact expected values.
mod tests {
    use super::*;

    #[test]
    fn new_document_validates_spec() {
        assert!(Document::new(DocumentSpec::new(0, 10)).is_err());
        assert!(matches!(
            Document::new(DocumentSpec::new(MAX_DIMENSION + 1, 10)),
            Err(EditorError::TooLarge { .. })
        ));
        assert!(matches!(
            Document::new(DocumentSpec::new(MAX_DIMENSION, MAX_DIMENSION)),
            Err(EditorError::TooLarge { .. })
        ));
        let mut spec = DocumentSpec::new(4, 4);
        spec.resolution_ppi = 0.0;
        assert!(matches!(
            Document::new(spec),
            Err(EditorError::OutOfRange { .. })
        ));
        spec.resolution_ppi = f32::NAN;
        assert!(matches!(
            Document::new(spec),
            Err(EditorError::NonFinite { .. })
        ));
        let mut spec = DocumentSpec::new(4, 4);
        spec.background = Background::Color([f32::INFINITY, 0.0, 0.0]);
        assert!(Document::new(spec).is_err());
    }

    #[test]
    fn new_document_has_background_layer() {
        let mut spec = DocumentSpec::new(3, 2);
        spec.background = Background::Color([0.5, 0.25, 2.0]);
        spec.bit_depth = BitDepth::Eight;
        let doc = Document::new(spec).unwrap();
        assert_eq!((doc.width(), doc.height()), (3, 2));
        assert_eq!(doc.bit_depth().bits(), 8);
        assert_eq!(doc.color_mode(), ColorMode::Rgb);
        assert_eq!(doc.layers().len(), 1);
        let bg = &doc.layers()[0];
        assert_eq!(bg.name(), "Background");
        let px = bg.as_pixel().unwrap();
        assert_eq!(px.image().pixel(2, 1), Some([0.5, 0.25, 2.0, 1.0]));

        let mut spec = DocumentSpec::new(3, 2);
        spec.background = Background::Transparent;
        let doc = Document::new(spec).unwrap();
        let px = doc.layers()[0].as_pixel().unwrap();
        assert_eq!(px.image().pixel(0, 0), Some([0.0; 4]));

        let doc = Document::new_empty(DocumentSpec::new(3, 2)).unwrap();
        assert_eq!(doc.layer_count(), 0);
    }
}
