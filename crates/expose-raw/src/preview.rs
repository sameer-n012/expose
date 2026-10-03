//! Extraction of embedded JPEG previews.
//!
//! Most camera RAW files, and many JPEG files (EXIF IFD1), contain one or
//! more embedded JPEG images. A thumbnail made from such a preview is much
//! faster than a full decode and develop.
//!
//! This module finds the previews itself. It uses no RAW backend, so it
//! also works in a build without the `rawler` feature. It reads:
//!
//! - TIFF-based files (DNG, CR2, NEF, ARW, PEF, SRW, ORF, RW2, TIFF): each
//!   IFD and sub-IFD with a JPEG interchange format (tags 513/514) or with
//!   one JPEG-compressed strip (tags 273/279). RW2 also has a full JPEG in
//!   tag 0x002E of IFD0.
//! - JPEG files: the TIFF structure inside the EXIF APP1 segment.
//! - RAF files: the JPEG offset and length in the file header.
//! - CR3 files: the JPEG of the first track and the `PRVW` preview.
//!
//! Each candidate must start with a JPEG start-of-image marker and have a
//! baseline, extended, or progressive frame header. Other JPEG kinds (for
//! example lossless JPEG sensor data) are ignored. All reads are
//! bounds-checked: bad or truncated data gives fewer candidates, never a
//! panic.

use std::ops::Range;

use crate::error::{DecodeError, DecodeResult};
use crate::format::{ImageFormat, ORF_MAGIC, RW2_MAGIC, RawFormat, detect_format};
use crate::metadata;
use crate::orientation::Orientation;
use crate::tiff::TiffReader;

/// TIFF tags that this module reads.
mod tag {
    /// Compression scheme.
    pub const COMPRESSION: u16 = 259;
    /// Orientation.
    pub const ORIENTATION: u16 = 274;
    /// Offsets of the image strips.
    pub const STRIP_OFFSETS: u16 = 273;
    /// Byte counts of the image strips.
    pub const STRIP_BYTE_COUNTS: u16 = 279;
    /// Shared JPEG tables (abbreviated JPEG streams).
    pub const JPEG_TABLES: u16 = 347;
    /// Offset of a JPEG interchange format stream.
    pub const JPEG_IF_OFFSET: u16 = 513;
    /// Length of a JPEG interchange format stream.
    pub const JPEG_IF_LENGTH: u16 = 514;
    /// RW2: a full JPEG image stored as the tag value.
    pub const RW2_JPEG_FROM_RAW: u16 = 0x002E;
}

/// UUID of the CR3 box that holds the `PRVW` preview.
const CR3_PREVIEW_UUID: [u8; 16] = [
    0xea, 0xf4, 0x2b, 0x5e, 0x1c, 0x98, 0x4b, 0x88, 0xb9, 0xfb, 0xb7, 0xdc, 0x40, 0x6e, 0x4d, 0x16,
];

/// Where an embedded preview was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewSource {
    /// A TIFF IFD (or sub-IFD) of the file, or the EXIF IFDs of a JPEG.
    TiffIfd,
    /// The JPEG that a RW2 file stores in tag 0x002E.
    Rw2JpegFromRaw,
    /// The JPEG that the RAF header points to.
    RafHeader,
    /// A JPEG of a CR3 file (first track or `PRVW` box).
    Cr3,
}

/// An embedded JPEG preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedPreview {
    /// The JPEG stream. It can have extra bytes after the end-of-image
    /// marker; JPEG decoders ignore them.
    pub jpeg: Vec<u8>,
    /// Stored width in pixels (from the JPEG frame header), before
    /// orientation.
    pub width: u32,
    /// Stored height in pixels (from the JPEG frame header), before
    /// orientation.
    pub height: u32,
    /// The orientation of the file. Apply it to the decoded preview: the
    /// preview pixels have the same stored orientation as the main image.
    /// `None` if the file does not give an orientation (then show the
    /// pixels as stored).
    ///
    /// Decode the JPEG with `apply_orientation: false`: an EXIF block in
    /// the preview itself could otherwise apply the transform two times.
    pub orientation: Option<Orientation>,
    /// Where the preview was found.
    pub source: PreviewSource,
}

impl EmbeddedPreview {
    /// Returns the longer side of the stored size.
    pub fn long_edge(&self) -> u32 {
        self.width.max(self.height)
    }
}

/// A preview candidate: a byte range of the file and its stored size.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    range: Range<usize>,
    width: u32,
    height: u32,
    source: PreviewSource,
}

impl Candidate {
    fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// Returns the largest embedded JPEG preview of an image file.
///
/// Returns `Ok(None)` if the file has no usable preview (or a PNG / WebP
/// file, which cannot have one). Returns [`DecodeError::UnknownFormat`] if
/// the data is not a known image format.
pub fn embedded_preview(data: &[u8]) -> DecodeResult<Option<EmbeddedPreview>> {
    embedded_preview_min(data, u32::MAX)
}

/// Returns the smallest embedded JPEG preview with a long edge of at least
/// `min_long_edge` pixels. If no preview is as large, returns the largest
/// preview.
///
/// A small preview decodes faster, so use this function when the output
/// size is known (for example, a thumbnail).
///
/// Errors are the same as for [`embedded_preview`].
pub fn embedded_preview_min(
    data: &[u8],
    min_long_edge: u32,
) -> DecodeResult<Option<EmbeddedPreview>> {
    let format = detect_format(data);
    let candidates = match format {
        Some(ImageFormat::Jpeg) => exif_candidates(data),
        Some(ImageFormat::Raw(RawFormat::Raf)) => raf_candidates(data),
        Some(ImageFormat::Raw(RawFormat::Cr3)) => cr3_candidates(data),
        Some(ImageFormat::Tiff | ImageFormat::Raw(_)) | None => {
            let found = tiff_candidates(data, 0);
            if format.is_none() && TiffReader::new(data, &tiff_magics()).is_none() {
                return Err(DecodeError::UnknownFormat);
            }
            found
        }
        Some(_) => Vec::new(),
    };
    let Some(best) = select(candidates, min_long_edge) else {
        return Ok(None);
    };
    let Some(jpeg) = data.get(best.range.clone()) else {
        // The range was checked when the candidate was made.
        return Ok(None);
    };
    // TIFF structures: the orientation tag of IFD0. Other containers: the
    // metadata reader (EXIF of the RAF preview, CR3 metadata boxes).
    let tiff_base = match format {
        Some(ImageFormat::Jpeg) => exif_tiff_start(data),
        Some(ImageFormat::Raw(RawFormat::Raf | RawFormat::Cr3)) => None,
        _ => Some(0),
    };
    let orientation = match tiff_base {
        Some(base) => tiff_orientation(data, base),
        None => format
            .and_then(|f| metadata::read_metadata_for(data, f, false).ok())
            .and_then(|m| m.orientation),
    };
    Ok(Some(EmbeddedPreview {
        jpeg: jpeg.to_vec(),
        width: best.width,
        height: best.height,
        orientation,
        source: best.source,
    }))
}

/// Picks the smallest candidate with a long edge of at least
/// `min_long_edge`, or else the largest candidate.
fn select(candidates: Vec<Candidate>, min_long_edge: u32) -> Option<Candidate> {
    let large_enough = candidates
        .iter()
        .filter(|c| c.width.max(c.height) >= min_long_edge)
        .min_by_key(|c| c.pixels())
        .cloned();
    large_enough.or_else(|| candidates.into_iter().max_by_key(Candidate::pixels))
}

/// The TIFF magic numbers that this module accepts (standard, ORF, RW2).
fn tiff_magics() -> [u16; 3] {
    [ORF_MAGIC[0], ORF_MAGIC[1], RW2_MAGIC]
}

/// Makes a candidate from `data[start..start + len]` if it is a JPEG
/// stream of a supported kind.
fn candidate(data: &[u8], start: usize, len: usize, source: PreviewSource) -> Option<Candidate> {
    let range = start..start.checked_add(len)?;
    let (width, height) = jpeg_size(data.get(range.clone())?)?;
    Some(Candidate {
        range,
        width,
        height,
        source,
    })
}

/// Finds the candidates in a TIFF structure that starts at `data[base..]`.
/// TIFF offsets are relative to `base`.
fn tiff_candidates(data: &[u8], base: usize) -> Vec<Candidate> {
    let Some(tiff) = data.get(base..) else {
        return Vec::new();
    };
    let Some(reader) = TiffReader::new(tiff, &tiff_magics()) else {
        return Vec::new();
    };
    let at = |offset: u32, len: u32, source| {
        let start = base.checked_add(usize::try_from(offset).ok()?)?;
        candidate(data, start, usize::try_from(len).ok()?, source)
    };
    let mut out = Vec::new();
    for ifd in reader.all_ifds() {
        let jpeg_if = ifd
            .get(tag::JPEG_IF_OFFSET)
            .and_then(|e| reader.uint(e))
            .zip(ifd.get(tag::JPEG_IF_LENGTH).and_then(|e| reader.uint(e)));
        if let Some(c) = jpeg_if.and_then(|(o, l)| at(o, l, PreviewSource::TiffIfd)) {
            out.push(c);
        }
        // One JPEG-compressed strip (old-style 6 or new-style 7). Strips
        // that need shared tables (tag 347) are not complete streams.
        let compression = ifd.get(tag::COMPRESSION).and_then(|e| reader.uint(e));
        if matches!(compression, Some(6 | 7)) && ifd.get(tag::JPEG_TABLES).is_none() {
            let offsets = ifd.get(tag::STRIP_OFFSETS).and_then(|e| reader.uints(e, 2));
            let counts = ifd
                .get(tag::STRIP_BYTE_COUNTS)
                .and_then(|e| reader.uints(e, 2));
            if let (Some([o]), Some([l])) = (offsets.as_deref(), counts.as_deref()) {
                if let Some(c) = at(*o, *l, PreviewSource::TiffIfd) {
                    out.push(c);
                }
            }
        }
        // RW2: the tag value is the JPEG stream (type UNDEFINED).
        if let Some(e) = ifd.get(tag::RW2_JPEG_FROM_RAW).filter(|e| e.typ == 7) {
            let offset = (e.count > 4).then(|| tiff_u32(tiff, e.field_pos)).flatten();
            if let Some(c) = offset.and_then(|o| at(o, e.count, PreviewSource::Rw2JpegFromRaw)) {
                out.push(c);
            }
        }
    }
    out
}

/// Reads a 32-bit value at `pos` in the byte order of the TIFF header.
fn tiff_u32(tiff: &[u8], pos: usize) -> Option<u32> {
    let b: [u8; 4] = tiff.get(pos..pos.checked_add(4)?)?.try_into().ok()?;
    match tiff.get(0..2)? {
        b"II" => Some(u32::from_le_bytes(b)),
        b"MM" => Some(u32::from_be_bytes(b)),
        _ => None,
    }
}

/// Returns the orientation (tag 274) of IFD0 of the TIFF structure at
/// `data[base..]`.
fn tiff_orientation(data: &[u8], base: usize) -> Option<Orientation> {
    let reader = TiffReader::new(data.get(base..)?, &tiff_magics())?;
    let ifd0 = reader.all_ifds().into_iter().next()?;
    let v = reader.uint(ifd0.get(tag::ORIENTATION)?)?;
    Orientation::from_exif(u16::try_from(v).ok()?)
}

/// Finds the candidates in the EXIF segment of a JPEG file.
fn exif_candidates(data: &[u8]) -> Vec<Candidate> {
    exif_tiff_start(data).map_or_else(Vec::new, |base| tiff_candidates(data, base))
}

/// Returns the position of the TIFF header in the EXIF APP1 segment of a
/// JPEG file.
fn exif_tiff_start(data: &[u8]) -> Option<usize> {
    let mut p = 2;
    loop {
        if *data.get(p)? != 0xFF {
            return None;
        }
        while *data.get(p)? == 0xFF {
            p += 1;
        }
        let marker = *data.get(p)?;
        p += 1;
        // The metadata segments come before the start of scan.
        if matches!(marker, 0xD9 | 0xDA) {
            return None;
        }
        if marker == 0x01 || (0xD0..=0xD8).contains(&marker) {
            continue;
        }
        let len = usize::from(be16(data, p)?);
        if len < 2 {
            return None;
        }
        let body = p + 2;
        if marker == 0xE1 && data.get(body..body.checked_add(6)?) == Some(b"Exif\0\0") {
            return Some(body + 6);
        }
        p = p.checked_add(len)?;
    }
}

/// Finds the JPEG that the RAF header points to. The header stores the
/// offset and length (big-endian) at bytes 84 and 88.
fn raf_candidates(data: &[u8]) -> Vec<Candidate> {
    let offset = be32(data, 84).and_then(|v| usize::try_from(v).ok());
    let len = be32(data, 88).and_then(|v| usize::try_from(v).ok());
    offset
        .zip(len)
        .and_then(|(o, l)| candidate(data, o, l, PreviewSource::RafHeader))
        .into_iter()
        .collect()
}

/// Finds the JPEGs of a CR3 file: the first track (full size) and the
/// `PRVW` box (a medium size preview).
fn cr3_candidates(data: &[u8]) -> Vec<Candidate> {
    let mut out = Vec::new();
    if let Some(c) = cr3_track_jpeg(data) {
        out.push(c);
    }
    if let Some(c) = cr3_prvw_jpeg(data) {
        out.push(c);
    }
    out
}

/// Returns the first sample of the first track of a CR3 file.
fn cr3_track_jpeg(data: &[u8]) -> Option<Candidate> {
    let moov = find_box(data, 0..data.len(), *b"moov")?;
    let trak = find_box(data, moov, *b"trak")?;
    let mdia = find_box(data, trak, *b"mdia")?;
    let minf = find_box(data, mdia, *b"minf")?;
    let stbl = find_box(data, minf, *b"stbl")?;
    // Chunk offset: 64-bit (co64) or 32-bit (stco). Both are full boxes:
    // version + flags (4), entry count (4), then the entries.
    let offset = if let Some(co64) = find_box(data, stbl.clone(), *b"co64") {
        let b: [u8; 8] = data.get(co64.start + 8..co64.start + 16)?.try_into().ok()?;
        usize::try_from(u64::from_be_bytes(b)).ok()?
    } else {
        let stco = find_box(data, stbl.clone(), *b"stco")?;
        usize::try_from(be32(data, stco.start + 8)?).ok()?
    };
    // Sample size box: version + flags (4), sample size (4), count (4),
    // then the per-sample sizes if the common sample size is 0.
    let stsz = find_box(data, stbl, *b"stsz")?;
    let common = be32(data, stsz.start + 4)?;
    let size = if common == 0 {
        be32(data, stsz.start + 12)?
    } else {
        common
    };
    candidate(
        data,
        offset,
        usize::try_from(size).ok()?,
        PreviewSource::Cr3,
    )
}

/// Returns the JPEG of the `PRVW` box in the preview UUID box of a CR3
/// file. The JPEG starts some bytes after the box header (the size fields
/// come first), so the first start-of-image marker in the box is used.
fn cr3_prvw_jpeg(data: &[u8]) -> Option<Candidate> {
    let mut p = 0;
    while p < data.len() {
        let (typ, body, end) = box_at(data, p, data.len())?;
        if typ == *b"uuid" && data.get(body.start..body.start + 16) == Some(&CR3_PREVIEW_UUID) {
            let inner = body.start + 16..body.end;
            let window = data.get(inner.clone())?;
            let tag_pos = window.windows(4).position(|w| w == b"PRVW")?;
            // The 4-byte box size comes before the type.
            let box_start = (inner.start + tag_pos).checked_sub(4)?;
            let (_, prvw, _) = box_at(data, box_start, inner.end)?;
            let bytes = data.get(prvw.clone())?;
            let soi = bytes
                .windows(3)
                .take(64)
                .position(|w| w == [0xFF, 0xD8, 0xFF])?;
            let start = prvw.start + soi;
            return candidate(data, start, prvw.end - start, PreviewSource::Cr3);
        }
        p = end;
    }
    None
}

/// Returns the body range of the first box of type `typ` in `range`.
fn find_box(data: &[u8], range: Range<usize>, typ: [u8; 4]) -> Option<Range<usize>> {
    let mut p = range.start;
    for _ in 0..4096 {
        if p >= range.end {
            return None;
        }
        let (t, body, end) = box_at(data, p, range.end)?;
        if t == typ {
            return Some(body);
        }
        p = end;
    }
    None
}

/// Reads the box header at `p`. Returns the type, the body range, and the
/// end of the box. The box must end at or before `limit`.
fn box_at(data: &[u8], p: usize, limit: usize) -> Option<([u8; 4], Range<usize>, usize)> {
    let size32 = be32(data, p)?;
    let typ: [u8; 4] = data.get(p + 4..p + 8)?.try_into().ok()?;
    let (header, size) = match size32 {
        // Size 0: the box extends to the end of the enclosing range.
        0 => (8, limit.checked_sub(p)?),
        // Size 1: a 64-bit size follows the type.
        1 => {
            let b: [u8; 8] = data.get(p + 8..p + 16)?.try_into().ok()?;
            (16, usize::try_from(u64::from_be_bytes(b)).ok()?)
        }
        s => (8, usize::try_from(s).ok()?),
    };
    if size < header {
        return None;
    }
    let end = p.checked_add(size)?;
    if end > limit || end > data.len() {
        return None;
    }
    Some((typ, p + header..end, end))
}

/// Reads a big-endian 16-bit value.
fn be16(data: &[u8], p: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        data.get(p..p.checked_add(2)?)?.try_into().ok()?,
    ))
}

/// Reads a big-endian 32-bit value.
fn be32(data: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(p..p.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// Returns the frame size (width, height) of a JPEG stream.
///
/// Returns `None` if the data does not start with a start-of-image marker,
/// if no frame header comes before the scan, or if the frame kind is not
/// baseline (SOF0), extended (SOF1), or progressive (SOF2).
fn jpeg_size(jpeg: &[u8]) -> Option<(u32, u32)> {
    if jpeg.get(0..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut p = 2;
    loop {
        if *jpeg.get(p)? != 0xFF {
            return None;
        }
        // Skip fill bytes.
        while *jpeg.get(p)? == 0xFF {
            p += 1;
        }
        let marker = *jpeg.get(p)?;
        p += 1;
        match marker {
            0x01 | 0xD0..=0xD8 => {}
            0xC0..=0xC2 => {
                // Length (2), precision (1), height (2), width (2).
                let h = be16(jpeg, p + 3)?;
                let w = be16(jpeg, p + 5)?;
                return (w > 0 && h > 0).then(|| (u32::from(w), u32::from(h)));
            }
            // Lossless, hierarchical, arithmetic frames; end of image or
            // start of scan before a frame header.
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF | 0xD9 | 0xDA => return None,
            _ => {
                let len = usize::from(be16(jpeg, p)?);
                if len < 2 {
                    return None;
                }
                p = p.checked_add(len)?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal JPEG header: SOI, an APP0-like segment, and a frame header
    /// of type `sof` with the given size. Enough for `jpeg_size`.
    fn jpeg_stub(sof: u8, width: u16, height: u16) -> Vec<u8> {
        let mut j = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        j.extend_from_slice(&[0xFF, sof, 0x00, 0x0B, 0x08]);
        j.extend_from_slice(&height.to_be_bytes());
        j.extend_from_slice(&width.to_be_bytes());
        j.extend_from_slice(&[0x01, 0x01, 0x11, 0x00]);
        j.extend_from_slice(&[0xFF, 0xD9]);
        j
    }

    /// A little-endian TIFF entry.
    fn entry(tag: u16, typ: u16, count: u32, value: u32) -> Vec<u8> {
        let mut e = tag.to_le_bytes().to_vec();
        e.extend_from_slice(&typ.to_le_bytes());
        e.extend_from_slice(&count.to_le_bytes());
        e.extend_from_slice(&value.to_le_bytes());
        e
    }

    /// A little-endian TIFF file with IFD0 (orientation 6, and one JPEG via
    /// tags 513/514) and IFD1 (one JPEG via a compressed strip).
    fn tiff_with_previews(a: &[u8], b: &[u8]) -> Vec<u8> {
        let ifd0_len = 2 + 3 * 12 + 4;
        let ifd1_pos = 8 + ifd0_len;
        let ifd1_len = 2 + 3 * 12 + 4;
        let a_pos = ifd1_pos + ifd1_len;
        let b_pos = a_pos + a.len();
        let u = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
        let mut d = b"II*\0\x08\0\0\0".to_vec();
        d.extend_from_slice(&3_u16.to_le_bytes());
        d.extend(entry(274, 3, 1, 6));
        d.extend(entry(tag::JPEG_IF_OFFSET, 4, 1, u(a_pos)));
        d.extend(entry(tag::JPEG_IF_LENGTH, 4, 1, u(a.len())));
        d.extend_from_slice(&u(ifd1_pos).to_le_bytes());
        d.extend_from_slice(&3_u16.to_le_bytes());
        d.extend(entry(tag::COMPRESSION, 3, 1, 7));
        d.extend(entry(tag::STRIP_OFFSETS, 4, 1, u(b_pos)));
        d.extend(entry(tag::STRIP_BYTE_COUNTS, 4, 1, u(b.len())));
        d.extend_from_slice(&0_u32.to_le_bytes());
        d.extend_from_slice(a);
        d.extend_from_slice(b);
        d
    }

    #[test]
    fn jpeg_size_reads_frame_kinds() {
        assert_eq!(jpeg_size(&jpeg_stub(0xC0, 160, 120)), Some((160, 120)));
        assert_eq!(jpeg_size(&jpeg_stub(0xC2, 7, 9)), Some((7, 9)));
        // Lossless JPEG (sensor data) is not a preview.
        assert_eq!(jpeg_size(&jpeg_stub(0xC3, 160, 120)), None);
        assert_eq!(jpeg_size(&jpeg_stub(0xC0, 0, 120)), None);
        assert_eq!(jpeg_size(b"not a jpeg"), None);
    }

    #[test]
    fn tiff_previews_are_found_and_selected() {
        let small = jpeg_stub(0xC0, 640, 480);
        let large = jpeg_stub(0xC0, 1600, 1200);
        let d = tiff_with_previews(&small, &large);
        let largest = embedded_preview(&d).unwrap().unwrap();
        assert_eq!((largest.width, largest.height), (1600, 1200));
        assert_eq!(largest.jpeg, large);
        assert_eq!(largest.source, PreviewSource::TiffIfd);
        assert_eq!(largest.orientation, Some(Orientation::Rotate90));
        // The smallest preview that is large enough.
        let fit = embedded_preview_min(&d, 512).unwrap().unwrap();
        assert_eq!((fit.width, fit.height), (640, 480));
        assert_eq!(fit.jpeg, small);
        // No preview is large enough: the largest one.
        let big = embedded_preview_min(&d, 4000).unwrap().unwrap();
        assert_eq!(big.long_edge(), 1600);
    }

    #[test]
    fn jpeg_exif_thumbnail_is_found() {
        let thumb = jpeg_stub(0xC0, 160, 120);
        let tiff = tiff_with_previews(&thumb, &[]);
        let mut d = vec![0xFF, 0xD8, 0xFF, 0xE1];
        let seg_len = u16::try_from(2 + 6 + tiff.len()).unwrap_or(u16::MAX);
        d.extend_from_slice(&seg_len.to_be_bytes());
        d.extend_from_slice(b"Exif\0\0");
        d.extend_from_slice(&tiff);
        d.extend_from_slice(&jpeg_stub(0xC0, 4000, 3000)[2..]);
        let p = embedded_preview(&d).unwrap().unwrap();
        assert_eq!((p.width, p.height), (160, 120));
        assert_eq!(p.jpeg, thumb);
    }

    #[test]
    fn raf_header_preview_is_found() {
        let jpeg = jpeg_stub(0xC0, 1920, 1280);
        let mut d = b"FUJIFILMCCD-RAW 0201".to_vec();
        d.resize(100, 0);
        d[84..88].copy_from_slice(&100_u32.to_be_bytes());
        let len = u32::try_from(jpeg.len()).unwrap_or(0);
        d[88..92].copy_from_slice(&len.to_be_bytes());
        d.extend_from_slice(&jpeg);
        let p = embedded_preview(&d).unwrap().unwrap();
        assert_eq!(p.source, PreviewSource::RafHeader);
        assert_eq!(p.long_edge(), 1920);
    }

    fn bmff_box(typ: &[u8], body: &[u8]) -> Vec<u8> {
        let size = u32::try_from(8 + body.len()).unwrap_or(0);
        let mut b = size.to_be_bytes().to_vec();
        b.extend_from_slice(typ);
        b.extend_from_slice(body);
        b
    }

    #[test]
    fn cr3_previews_are_found() {
        let full = jpeg_stub(0xC0, 6000, 4000);
        let medium = jpeg_stub(0xC0, 1620, 1080);
        let ftyp = bmff_box(b"ftyp", b"crx \0\0\0\x01crx isom");
        // PRVW: 14 bytes of size fields, then the JPEG.
        let mut prvw_body = vec![0_u8; 14];
        prvw_body.extend_from_slice(&medium);
        let mut uuid_body = CR3_PREVIEW_UUID.to_vec();
        uuid_body.extend_from_slice(&[0_u8; 8]);
        uuid_body.extend(bmff_box(b"PRVW", &prvw_body));
        let uuid = bmff_box(b"uuid", &uuid_body);
        // The moov size does not depend on the offset value, so build it
        // once to know where the JPEG data starts.
        let moov_for = |offset: u64| {
            let mut co64 = vec![0, 0, 0, 0, 0, 0, 0, 1];
            co64.extend_from_slice(&offset.to_be_bytes());
            let mut stsz = vec![0_u8; 4];
            stsz.extend_from_slice(&0_u32.to_be_bytes());
            stsz.extend_from_slice(&1_u32.to_be_bytes());
            let len = u32::try_from(full.len()).unwrap_or(0);
            stsz.extend_from_slice(&len.to_be_bytes());
            let stbl = bmff_box(
                b"stbl",
                &[bmff_box(b"co64", &co64), bmff_box(b"stsz", &stsz)].concat(),
            );
            let minf = bmff_box(b"minf", &stbl);
            let mdia = bmff_box(b"mdia", &minf);
            let trak = bmff_box(b"trak", &[bmff_box(b"tkhd", &[0; 4]), mdia].concat());
            bmff_box(b"moov", &trak)
        };
        let head = ftyp.len() + uuid.len() + moov_for(0).len() + 8;
        let mdat = bmff_box(b"mdat", &full);
        let d = [ftyp, uuid, moov_for(head as u64), mdat].concat();
        assert_eq!(detect_format(&d), Some(ImageFormat::Raw(RawFormat::Cr3)));
        let largest = embedded_preview(&d).unwrap().unwrap();
        assert_eq!(largest.long_edge(), 6000);
        assert_eq!(largest.jpeg, full);
        let fit = embedded_preview_min(&d, 512).unwrap().unwrap();
        assert_eq!(fit.long_edge(), 1620);
        assert!(fit.jpeg.starts_with(&medium));
    }

    #[test]
    fn absent_preview_is_none() {
        // A TIFF without JPEG data.
        let mut d = b"II*\0\x08\0\0\0".to_vec();
        d.extend_from_slice(&1_u16.to_le_bytes());
        d.extend(entry(274, 3, 1, 1));
        d.extend_from_slice(&0_u32.to_le_bytes());
        assert_eq!(embedded_preview(&d).unwrap(), None);
        // A JPEG without EXIF.
        assert_eq!(embedded_preview(&jpeg_stub(0xC0, 10, 10)).unwrap(), None);
        // A PNG signature.
        assert_eq!(
            embedded_preview(b"\x89PNG\r\n\x1a\n\0\0\0\0").unwrap(),
            None
        );
        // Unknown data.
        assert!(matches!(
            embedded_preview(b"hello world"),
            Err(DecodeError::UnknownFormat)
        ));
    }

    #[test]
    fn truncated_data_does_not_panic() {
        let d = tiff_with_previews(&jpeg_stub(0xC0, 640, 480), &jpeg_stub(0xC0, 1600, 1200));
        for n in 0..d.len() {
            let _ = embedded_preview(&d[..n]);
        }
        // Cut inside the second JPEG: only the first one is left.
        let cut = d.len() - 3;
        let p = embedded_preview(&d[..cut]).unwrap().unwrap();
        assert_eq!(p.long_edge(), 640);
    }
}
