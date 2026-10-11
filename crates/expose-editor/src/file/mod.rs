//! The native layered document file (`.exdoc`): read and write.
//!
//! The functions use [`std::io::Read`] and [`std::io::Write`] only. They do
//! not open files and do not start threads, so a WebAssembly build can
//! supply its own storage. A round trip gives a document that is equal to
//! the original (see [`Document`] equality). All pixel and mask values are
//! kept bit for bit.
//!
//! # Byte layout
//!
//! All integers are little-endian. The file has three parts:
//!
//! 1. **Header** (20 bytes):
//!
//!    | Offset | Size | Content                                        |
//!    | ------ | ---- | ---------------------------------------------- |
//!    | 0      | 8    | Magic: [`MAGIC`] (`89 45 58 44 4F 43 0D 0A`)   |
//!    | 8      | 2    | Major version (`u16`), now [`VERSION_MAJOR`]   |
//!    | 10     | 2    | Minor version (`u16`), now [`VERSION_MINOR`]   |
//!    | 12     | 8    | Manifest length `M` in bytes (`u64`)           |
//!
//! 2. **Manifest** (`M` bytes): UTF-8 JSON, at most [`MAX_MANIFEST_BYTES`].
//! 3. **Blob section**: the blobs. A blob offset is the number of bytes
//!    from the start of the blob section (byte `20 + M` of the file). The
//!    writer puts the blobs in table order with no gaps. A reader accepts
//!    gaps, but blobs must not overlap.
//!
//! A reader rejects a different major version. A new minor version can add
//! JSON fields. A reader ignores the fields that it does not know.
//!
//! # Manifest
//!
//! All names are camelCase. Enum values are camelCase strings (for
//! example, the blend mode `colorBurn`).
//!
//! ```text
//! {
//!   "document": { "width", "height", "resolutionPpi",
//!                 "colorMode": "rgb", "bitDepth": 8 | 16 | 32,
//!                 "blendSpace": "linear" | "perceptual",
//!                 "nextLayerId" (optional) },
//!   "layers": [ layer, ... ],          // root layers, bottom first
//!   "blobs":  [ { "offset", "compressedLength", "rawLength", "crc32" }, ... ]
//! }
//!
//! layer = { "id", "name", "visible", "opacity", "fillOpacity",
//!           "blendMode", "locks": { "transparency", "pixels",
//!           "position", "all" },
//!           "mask" (optional): { "offset": [x, y], "width", "height",
//!                                "enabled", "fill": "white" | "black",
//!                                "blob" },
//!           "content": content }
//!
//! content = { "kind": "pixel", "offset": [x, y], "width", "height", "blob" }
//!         | { "kind": "group", "isolation": "passThrough" | "isolated",
//!             "children": [ layer, ... ] }      // bottom first
//!         | { "kind": "adjustment", "adjustment": adjustment }
//!
//! adjustment = { "type": "brightnessContrast", "brightness", "contrast" }
//!            | { "type": "levels", "master", "red", "green", "blue" }
//!                // each: { "inputBlack", "inputWhite", "gamma",
//!                //         "outputBlack", "outputWhite" }
//!            | { "type": "curves", "master", "red", "green", "blue" }
//!                // each: [ [x, y], ... ]
//!            | { "type": "hueSaturation", "hue", "saturation", "lightness" }
//! ```
//!
//! # Blobs
//!
//! A pixel blob holds `width * height * 4` values (premultiplied linear
//! Rec. 2020 RGBA, row-major, interleaved). A mask blob holds
//! `width * height` values. Each layer and each mask has its own blob. The
//! writer encodes a blob in three steps:
//!
//! 1. Write each value as a little-endian `f32` (4 bytes).
//! 2. Shuffle the bytes: first byte 0 of all values, then byte 1 of all
//!    values, then byte 2, then byte 3.
//! 3. Compress the result with raw deflate (RFC 1951, no zlib header).
//!
//! `rawLength` is the length after step 2. `crc32` is the CRC-32
//! (ISO-HDLC, the zlib CRC) of the stored bytes (after step 3).
//!
//! # Reader checks
//!
//! The reader checks the magic and the major version. It checks all sizes
//! against [`MAX_DIMENSION`](crate::MAX_DIMENSION) and
//! [`MAX_PIXELS`](crate::MAX_PIXELS), and the raw length of each blob
//! against the size of its layer or mask, before it allocates pixel
//! memory. A raw length must also be possible for the compressed length
//! (deflate compresses at most about 1032 to 1), so a small file cannot
//! cause a very large allocation. It checks the CRC and the length of each blob, the layer IDs
//! (not 0, no duplicates), the blob references (in range, each blob used
//! once), the group depth, and the adjustment parameters. The document
//! keeps the layer IDs of the file. It gets new version stamps, so caches
//! do not mix it with other documents.

mod blob;
mod manifest;

use std::collections::HashSet;
use std::io::{self, Read, Write};

use expose_core::buffer::WorkingImage;
use thiserror::Error;

use crate::adjust::Adjustment;
use crate::document::{Document, DocumentSpec, check_size};
use crate::error::{EditorError, EditorResult, check_range};
use crate::layer::{Group, Layer, LayerId, LayerKind, LayerProps, Mask, MaskImage, PixelLayer};
use blob::VALUE_BYTES;
use manifest::{
    BlobDto, ContentDto, DocumentDto, LayerDto, ManifestDto, MaskDto, bit_depth_from_dto,
    bit_depth_to_dto,
};

/// The file name extension of the native document file (without a dot).
pub const FILE_EXTENSION: &str = "exdoc";

/// The first 8 bytes of a document file.
///
/// The first byte is not ASCII and the last two bytes are CR LF, so that a
/// text-mode transfer that changes the file is found.
pub const MAGIC: [u8; 8] = [0x89, b'E', b'X', b'D', b'O', b'C', 0x0D, 0x0A];

/// The major format version that this crate reads and writes.
pub const VERSION_MAJOR: u16 = 1;

/// The minor format version that this crate writes.
pub const VERSION_MINOR: u16 = 0;

/// The largest permitted manifest, in bytes (64 MiB).
pub const MAX_MANIFEST_BYTES: u64 = 64 << 20;

/// The deepest permitted group nesting. A root layer has depth 0, and a
/// layer in a root group has depth 1.
///
/// The JSON parser has a nesting limit of 128. Each group level adds 3
/// JSON levels, so this limit keeps all valid files below the parser limit.
pub const MAX_GROUP_DEPTH: usize = 32;

/// Length of the fixed header in bytes.
const HEADER_LEN: usize = 20;

/// Number of `f32` values in one pixel.
const PIXEL_VALUES: usize = 4;

/// The largest compression ratio of deflate: a match of 258 bytes can use
/// 2 bits (a 1-bit length code and a 1-bit distance code).
const MAX_DEFLATE_RATIO: u64 = 1032;

/// Bytes that the ratio check adds to the limit, for the block headers of
/// very short streams.
const DEFLATE_RATIO_SLACK: u64 = 1024;

/// An error in a document file.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FileError {
    /// The input or output stream failed.
    #[error("I/O error ({kind:?}): {message}")]
    Io {
        /// The kind of the I/O error.
        kind: io::ErrorKind,
        /// The message of the I/O error.
        message: String,
    },

    /// The file ends before all data is read.
    #[error("the file is truncated")]
    Truncated,

    /// The file does not start with [`MAGIC`].
    #[error("the file is not an Expose document")]
    BadMagic,

    /// The major version is not [`VERSION_MAJOR`].
    #[error("format version {major}.{minor} is not supported")]
    UnsupportedVersion {
        /// Major version in the file.
        major: u16,
        /// Minor version in the file.
        minor: u16,
    },

    /// The manifest is larger than [`MAX_MANIFEST_BYTES`].
    #[error("manifest length {0} is more than the limit")]
    ManifestTooLarge(u64),

    /// The manifest is not valid JSON, or it does not have the expected
    /// structure.
    #[error("manifest is not valid: {0}")]
    Manifest(String),

    /// The manifest has a structural problem (for example, a blob index
    /// that is out of range).
    #[error("{0}")]
    Invalid(&'static str),

    /// The CRC-32 of a blob is not correct.
    #[error("blob {blob} has a CRC error")]
    Crc {
        /// Blob index.
        blob: usize,
    },

    /// The data of a blob is not valid (but its CRC is correct).
    #[error("blob {blob} is not valid: {reason}")]
    Blob {
        /// Blob index.
        blob: usize,
        /// Description of the problem.
        reason: &'static str,
    },
}

impl From<io::Error> for FileError {
    /// Converts an I/O error. An unexpected end of the input becomes
    /// [`FileError::Truncated`].
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            Self::Truncated
        } else {
            Self::Io {
                kind: error.kind(),
                message: error.to_string(),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Write
// ---------------------------------------------------------------------------

/// Writes a document to `out` in the native format.
///
/// The function compresses all blobs in memory first, because the manifest
/// (which comes first) holds their lengths and CRCs. It does not flush or
/// close `out` other than with [`Write::flush`] at the end.
///
/// # Errors
///
/// - [`EditorError::File`] if `out` fails, or if the groups are nested
///   deeper than [`MAX_GROUP_DEPTH`].
/// - The error of [`Adjustment::validate`] or of a property check, if the
///   document has a value that a reader would not accept.
pub fn write_document<W: Write>(document: &Document, mut out: W) -> EditorResult<()> {
    let mut blobs = Vec::new();
    let layers = document
        .layers()
        .iter()
        .map(|l| layer_to_dto(l, 0, &mut blobs))
        .collect::<EditorResult<Vec<_>>>()?;

    let mut offset = 0_u64;
    let mut table = Vec::with_capacity(blobs.len());
    for b in &blobs {
        let length = b.bytes.len() as u64;
        table.push(BlobDto {
            offset,
            compressed_length: length,
            raw_length: b.raw_length,
            crc32: b.crc32,
        });
        offset += length;
    }

    let manifest = ManifestDto {
        document: DocumentDto {
            width: document.width(),
            height: document.height(),
            resolution_ppi: document.resolution_ppi(),
            color_mode: document.color_mode().into(),
            bit_depth: bit_depth_to_dto(document.bit_depth()),
            blend_space: document.blend_space().into(),
            next_layer_id: document.next_id(),
        },
        layers,
        blobs: table,
    };
    let json = serde_json::to_vec(&manifest).map_err(|e| FileError::Manifest(e.to_string()))?;
    let json_len = json.len() as u64;
    if json_len > MAX_MANIFEST_BYTES {
        return Err(FileError::ManifestTooLarge(json_len).into());
    }

    let mut header = [0_u8; HEADER_LEN];
    header[0..8].copy_from_slice(&MAGIC);
    header[8..10].copy_from_slice(&VERSION_MAJOR.to_le_bytes());
    header[10..12].copy_from_slice(&VERSION_MINOR.to_le_bytes());
    header[12..20].copy_from_slice(&json_len.to_le_bytes());
    let io = |e: io::Error| EditorError::from(FileError::from(e));
    out.write_all(&header).map_err(io)?;
    out.write_all(&json).map_err(io)?;
    for b in &blobs {
        out.write_all(&b.bytes).map_err(io)?;
    }
    out.flush().map_err(io)?;
    Ok(())
}

fn offset_to_dto(offset: (i32, i32)) -> [i32; 2] {
    [offset.0, offset.1]
}

/// Checks the properties that the reader checks, so that the writer does
/// not make a file that it cannot read.
fn check_props(props: &LayerProps) -> EditorResult<()> {
    check_range("opacity", props.opacity, 0.0, 1.0)?;
    check_range("fill_opacity", props.fill_opacity, 0.0, 1.0)
}

fn layer_to_dto(
    layer: &Layer,
    depth: usize,
    blobs: &mut Vec<blob::EncodedBlob>,
) -> EditorResult<LayerDto> {
    let props = layer.props();
    check_props(props)?;
    let content = match layer.kind() {
        LayerKind::Pixel(p) => {
            let image = p.image();
            let index = blobs.len();
            blobs.push(blob::encode(image.as_slice())?);
            ContentDto::Pixel {
                offset: offset_to_dto(p.offset()),
                width: image.width(),
                height: image.height(),
                blob: index,
            }
        }
        LayerKind::Group(g) => {
            if !g.children().is_empty() && depth >= MAX_GROUP_DEPTH {
                return Err(FileError::Invalid("groups are nested too deep").into());
            }
            let children = g
                .children()
                .iter()
                .map(|c| layer_to_dto(c, depth + 1, blobs))
                .collect::<EditorResult<Vec<_>>>()?;
            ContentDto::Group {
                isolation: g.isolation().into(),
                children,
            }
        }
        LayerKind::Adjustment(a) => {
            a.validate()?;
            ContentDto::Adjustment {
                adjustment: a.into(),
            }
        }
    };
    let mask = match layer.mask() {
        None => None,
        Some(m) => {
            let index = blobs.len();
            blobs.push(blob::encode(m.image.data())?);
            Some(MaskDto {
                offset: offset_to_dto(m.offset),
                width: m.image.width(),
                height: m.image.height(),
                enabled: m.enabled,
                fill: m.fill.into(),
                blob: index,
            })
        }
    };
    Ok(LayerDto {
        id: layer.id().get(),
        name: props.name.clone(),
        visible: props.visible,
        opacity: props.opacity,
        fill_opacity: props.fill_opacity,
        blend_mode: props.blend_mode.into(),
        locks: props.locks.into(),
        mask,
        content,
    })
}

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// The decoded data of one blob.
enum Decoded {
    Pixels(Vec<[f32; 4]>),
    Mask(Vec<f32>),
}

/// What a blob holds, from the manifest.
#[derive(Clone, Copy)]
struct BlobUse {
    /// Number of `f32` values.
    values: usize,
    /// `true` for a pixel blob, `false` for a mask blob.
    pixels: bool,
}

/// Reads a document in the native format from `input`.
///
/// The function reads the header, the manifest, and then the blobs that
/// the layers use. It does not read past the last used blob. It allocates
/// one buffer for each pixel layer and each mask, and no canvas buffers.
///
/// # Errors
///
/// - [`EditorError::File`] for a stream error, a truncated file, a wrong
///   magic, an unsupported major version, a manifest that is not valid,
///   a CRC error, or blob data that is not valid.
/// - [`EditorError::TooLarge`] (or a core size error) for a size that
///   breaks the limits.
/// - [`EditorError::DuplicateId`] for a layer ID that occurs two times.
/// - The error of a property or adjustment check
///   ([`EditorError::OutOfRange`], [`EditorError::NonFinite`],
///   [`EditorError::InvalidCurve`]).
pub fn read_document<R: Read>(mut input: R) -> EditorResult<Document> {
    let manifest = read_manifest(&mut input)?;
    let doc = &manifest.document;
    let bit_depth = bit_depth_from_dto(doc.bit_depth)
        .ok_or(FileError::Invalid("bit depth is not 8, 16, or 32"))?;
    let spec = DocumentSpec {
        width: doc.width,
        height: doc.height,
        resolution_ppi: doc.resolution_ppi,
        color_mode: doc.color_mode.into(),
        bit_depth,
        background: crate::document::Background::Transparent,
        blend_space: doc.blend_space.into(),
    };
    // Check the canvas before any allocation (`from_parts` checks again).
    check_size(spec.width, spec.height)?;

    // Check the layer tree and find the use of each blob.
    let mut uses: Vec<Option<BlobUse>> = vec![None; manifest.blobs.len()];
    let mut ids = HashSet::new();
    for l in &manifest.layers {
        check_layer(l, 0, &manifest.blobs, &mut uses, &mut ids)?;
    }

    // Read the used blobs in file order.
    let mut order: Vec<usize> = (0..uses.len()).filter(|&i| uses[i].is_some()).collect();
    order.sort_by_key(|&i| manifest.blobs[i].offset);
    let mut decoded: Vec<Option<Decoded>> = (0..uses.len()).map(|_| None).collect();
    let mut pos = 0_u64;
    for index in order {
        let entry = manifest.blobs[index];
        let Some(blob_use) = uses[index] else {
            continue;
        };
        if entry.offset < pos {
            return Err(FileError::Invalid("blobs overlap").into());
        }
        skip(&mut input, entry.offset - pos)?;
        decoded[index] = Some(read_blob(&mut input, index, &entry, blob_use)?);
        pos = entry
            .offset
            .checked_add(entry.compressed_length)
            .ok_or(FileError::Invalid("blob end is out of range"))?;
    }

    let layers = manifest
        .layers
        .iter()
        .map(|l| layer_from_dto(l, &mut decoded))
        .collect::<EditorResult<Vec<_>>>()?;
    Document::from_parts(&spec, layers, doc.next_layer_id)
}

/// Reads and parses the header and the manifest.
fn read_manifest<R: Read>(input: &mut R) -> EditorResult<ManifestDto> {
    let mut header = [0_u8; HEADER_LEN];
    input.read_exact(&mut header).map_err(FileError::from)?;
    if header[0..8] != MAGIC {
        return Err(FileError::BadMagic.into());
    }
    let major = u16::from_le_bytes([header[8], header[9]]);
    let minor = u16::from_le_bytes([header[10], header[11]]);
    if major != VERSION_MAJOR {
        return Err(FileError::UnsupportedVersion { major, minor }.into());
    }
    let mut len_bytes = [0_u8; 8];
    len_bytes.copy_from_slice(&header[12..20]);
    let len = u64::from_le_bytes(len_bytes);
    if len > MAX_MANIFEST_BYTES {
        return Err(FileError::ManifestTooLarge(len).into());
    }
    let mut json = Vec::new();
    input
        .take(len)
        .read_to_end(&mut json)
        .map_err(FileError::from)?;
    if json.len() as u64 != len {
        return Err(FileError::Truncated.into());
    }
    serde_json::from_slice(&json).map_err(|e| FileError::Manifest(e.to_string()).into())
}

/// Reads and drops `count` bytes.
fn skip<R: Read>(input: &mut R, count: u64) -> EditorResult<()> {
    let copied = io::copy(&mut input.take(count), &mut io::sink()).map_err(FileError::from)?;
    if copied != count {
        return Err(FileError::Truncated.into());
    }
    Ok(())
}

/// Allocates the buffer of one blob and decodes the blob into it. The
/// caller has checked the size against the limits.
fn read_blob<R: Read>(
    input: &mut R,
    index: usize,
    entry: &BlobDto,
    blob_use: BlobUse,
) -> EditorResult<Decoded> {
    if blob_use.pixels {
        let mut pixels = vec![[0.0_f32; PIXEL_VALUES]; blob_use.values / PIXEL_VALUES];
        blob::decode(
            input,
            index,
            entry.compressed_length,
            entry.crc32,
            bytemuck::cast_slice_mut(&mut pixels),
        )?;
        Ok(Decoded::Pixels(pixels))
    } else {
        let mut values = vec![0.0_f32; blob_use.values];
        blob::decode(
            input,
            index,
            entry.compressed_length,
            entry.crc32,
            &mut values,
        )?;
        Ok(Decoded::Mask(values))
    }
}

/// Checks one buffer reference: the size limits, the blob index, the raw
/// length, and that no other buffer uses the blob.
fn check_blob_ref(
    width: u32,
    height: u32,
    channels: usize,
    index: usize,
    blobs: &[BlobDto],
    uses: &mut [Option<BlobUse>],
) -> EditorResult<()> {
    let count = check_size(width, height)?;
    let entry = blobs
        .get(index)
        .ok_or(FileError::Invalid("blob index is out of range"))?;
    let slot = uses
        .get_mut(index)
        .ok_or(FileError::Invalid("blob index is out of range"))?;
    if slot.is_some() {
        return Err(FileError::Invalid("two buffers use the same blob").into());
    }
    // count <= MAX_PIXELS (2^28), so count * 4 * 4 fits in u64.
    let values = count * channels;
    let raw = values as u64 * VALUE_BYTES as u64;
    if entry.raw_length != raw {
        return Err(FileError::Blob {
            blob: index,
            reason: "raw length does not agree with the buffer size",
        }
        .into());
    }
    if entry.offset.checked_add(entry.compressed_length).is_none() {
        return Err(FileError::Invalid("blob end is out of range").into());
    }
    // Deflate cannot compress more than about 1032 to 1. This check stops
    // a small file that claims a very large buffer before the allocation.
    let max_raw = entry
        .compressed_length
        .saturating_mul(MAX_DEFLATE_RATIO)
        .saturating_add(DEFLATE_RATIO_SLACK);
    if raw > max_raw {
        return Err(FileError::Blob {
            blob: index,
            reason: "raw length is too large for the compressed length",
        }
        .into());
    }
    *slot = Some(BlobUse {
        values,
        pixels: channels == PIXEL_VALUES,
    });
    Ok(())
}

/// Checks a layer and its descendants without allocating pixel memory.
fn check_layer(
    layer: &LayerDto,
    depth: usize,
    blobs: &[BlobDto],
    uses: &mut [Option<BlobUse>],
    ids: &mut HashSet<u64>,
) -> EditorResult<()> {
    if layer.id == 0 {
        return Err(FileError::Invalid("layer ID 0 is not permitted").into());
    }
    if !ids.insert(layer.id) {
        return Err(EditorError::DuplicateId(LayerId(layer.id)));
    }
    check_range("opacity", layer.opacity, 0.0, 1.0)?;
    check_range("fill_opacity", layer.fill_opacity, 0.0, 1.0)?;
    if let Some(m) = &layer.mask {
        check_blob_ref(m.width, m.height, 1, m.blob, blobs, uses)?;
    }
    match &layer.content {
        ContentDto::Pixel {
            width,
            height,
            blob,
            ..
        } => check_blob_ref(*width, *height, PIXEL_VALUES, *blob, blobs, uses),
        ContentDto::Group { children, .. } => {
            if !children.is_empty() && depth >= MAX_GROUP_DEPTH {
                return Err(FileError::Invalid("groups are nested too deep").into());
            }
            for c in children {
                check_layer(c, depth + 1, blobs, uses, ids)?;
            }
            Ok(())
        }
        ContentDto::Adjustment { adjustment } => Adjustment::from(adjustment).validate(),
    }
}

/// Takes the decoded buffer of a blob.
fn take_blob(decoded: &mut [Option<Decoded>], index: usize) -> EditorResult<Decoded> {
    decoded
        .get_mut(index)
        .and_then(Option::take)
        .ok_or_else(|| FileError::Invalid("blob was not decoded").into())
}

/// Builds a layer from its checked manifest entry and the decoded blobs.
fn layer_from_dto(layer: &LayerDto, decoded: &mut [Option<Decoded>]) -> EditorResult<Layer> {
    let kind = match &layer.content {
        ContentDto::Pixel {
            offset,
            width,
            height,
            blob,
        } => {
            let Decoded::Pixels(pixels) = take_blob(decoded, *blob)? else {
                return Err(FileError::Invalid("pixel layer uses a mask blob").into());
            };
            let image = WorkingImage::from_pixels(*width, *height, pixels)?;
            LayerKind::Pixel(PixelLayer::new(image, (offset[0], offset[1]))?)
        }
        ContentDto::Group {
            isolation,
            children,
        } => {
            let mut group = Group::default();
            group.set_isolation((*isolation).into());
            let list = group.children_mut();
            list.reserve(children.len());
            for c in children {
                list.push(layer_from_dto(c, decoded)?);
            }
            LayerKind::Group(group)
        }
        ContentDto::Adjustment { adjustment } => LayerKind::Adjustment(adjustment.into()),
    };
    let props = LayerProps {
        name: layer.name.clone(),
        visible: layer.visible,
        opacity: layer.opacity,
        fill_opacity: layer.fill_opacity,
        blend_mode: layer.blend_mode.into(),
        locks: layer.locks.into(),
    };
    let mut out = Layer::new(LayerId(layer.id), props, kind);
    if let Some(m) = &layer.mask {
        let Decoded::Mask(values) = take_blob(decoded, m.blob)? else {
            return Err(FileError::Invalid("mask uses a pixel blob").into());
        };
        let image = MaskImage::from_data(m.width, m.height, values)?;
        out.replace_mask(Some(Mask {
            image,
            offset: (m.offset[0], m.offset[1]),
            enabled: m.enabled,
            fill: m.fill.into(),
        }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
