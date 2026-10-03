//! Image format detection from magic bytes.
//!
//! Detection reads the content, not the file name extension.

use serde::{Deserialize, Serialize};

use crate::tiff::{TiffReader, photometric, tag};

/// The container format of an image file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "raw")]
pub enum ImageFormat {
    /// JPEG (JFIF or EXIF).
    Jpeg,
    /// PNG.
    Png,
    /// TIFF (not a camera RAW file).
    Tiff,
    /// WebP.
    WebP,
    /// HEIF / HEIC (detected only; decode is not supported).
    Heif,
    /// AVIF (detected only; decode is not supported).
    Avif,
    /// A camera RAW file.
    Raw(RawFormat),
}

impl ImageFormat {
    /// Returns `true` for camera RAW formats.
    pub const fn is_raw(self) -> bool {
        matches!(self, Self::Raw(_))
    }
}

/// Camera RAW container formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RawFormat {
    /// Digital Negative (`.dng`), an open TIFF-based format.
    Dng,
    /// Canon CR2 (TIFF-based).
    Cr2,
    /// Canon CR3 (ISO base media file format).
    Cr3,
    /// Canon CRW (CIFF).
    Crw,
    /// Nikon NEF / NRW (TIFF-based).
    Nef,
    /// Sony ARW / SR2 / SRF (TIFF-based).
    Arw,
    /// Fujifilm RAF.
    Raf,
    /// Olympus / OM System ORF (TIFF variant).
    Orf,
    /// Panasonic / Leica RW2 / RWL (TIFF variant).
    Rw2,
    /// Pentax / Ricoh PEF (TIFF-based).
    Pef,
    /// Samsung SRW (TIFF-based).
    Srw,
    /// Sigma X3F.
    X3f,
    /// Minolta MRW.
    Mrw,
    /// Another TIFF-based RAW file (for example, from medium-format or
    /// older cameras).
    OtherTiff,
}

impl RawFormat {
    /// Returns `true` if the file has a TIFF structure, so that a TIFF EXIF
    /// reader can read its metadata. ORF and RW2 have a TIFF structure with
    /// a non-standard magic number.
    pub const fn is_tiff_based(self) -> bool {
        matches!(
            self,
            Self::Dng
                | Self::Cr2
                | Self::Nef
                | Self::Arw
                | Self::Orf
                | Self::Rw2
                | Self::Pef
                | Self::Srw
                | Self::OtherTiff
        )
    }
}

/// TIFF magic numbers of ORF files (`IIRO`, `IIRS`, `MMOR`).
pub(crate) const ORF_MAGIC: [u16; 2] = [0x4F52, 0x5352];
/// TIFF magic number of RW2 files (`IIU\0`).
pub(crate) const RW2_MAGIC: u16 = 0x0055;

/// Detects the format of `data` from its first bytes.
///
/// Returns `None` if the format is not known. For TIFF files, the function
/// reads the first directories to tell a camera RAW file from a normal
/// TIFF: a DNG version tag, the CR2 marker, or sensor data (CFA or linear
/// raw photometric interpretation) identify a RAW file.
pub fn detect_format(data: &[u8]) -> Option<ImageFormat> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(ImageFormat::Jpeg);
    }
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(ImageFormat::Png);
    }
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Some(ImageFormat::WebP);
    }
    if data.starts_with(b"FUJIFILMCCD-RAW") {
        return Some(ImageFormat::Raw(RawFormat::Raf));
    }
    if data.starts_with(b"FOVb") {
        return Some(ImageFormat::Raw(RawFormat::X3f));
    }
    if data.starts_with(b"\0MRM") {
        return Some(ImageFormat::Raw(RawFormat::Mrw));
    }
    if data.len() >= 14 && &data[0..2] == b"II" && &data[6..14] == b"HEAPCCDR" {
        return Some(ImageFormat::Raw(RawFormat::Crw));
    }
    if let Some(f) = detect_isobmff(data) {
        return Some(f);
    }
    detect_tiff(data)
}

/// Detects formats based on the ISO base media file format (`ftyp` box).
fn detect_isobmff(data: &[u8]) -> Option<ImageFormat> {
    if data.get(4..8)? != b"ftyp" {
        return None;
    }
    let size = usize::try_from(u32::from_be_bytes(data.get(0..4)?.try_into().ok()?)).ok()?;
    // The brands are the major brand (8..12) and the compatible brands
    // (16..size). Read only the bytes that are present.
    let end = size.min(data.len()).min(256);
    let mut brands: Vec<&[u8]> = vec![data.get(8..12)?];
    let mut p = 16;
    while p + 4 <= end {
        brands.push(&data[p..p + 4]);
        p += 4;
    }
    let has = |b: &[u8]| brands.contains(&b);
    if has(b"crx ") {
        Some(ImageFormat::Raw(RawFormat::Cr3))
    } else if has(b"avif") || has(b"avis") {
        Some(ImageFormat::Avif)
    } else if [
        "heic", "heix", "heim", "heis", "hevc", "hevx", "mif1", "msf1",
    ]
    .iter()
    .any(|b| has(b.as_bytes()))
    {
        Some(ImageFormat::Heif)
    } else {
        None
    }
}

/// Detects TIFF and TIFF-based RAW formats.
fn detect_tiff(data: &[u8]) -> Option<ImageFormat> {
    let mut extra = ORF_MAGIC.to_vec();
    extra.push(RW2_MAGIC);
    let reader = TiffReader::new(data, &extra)?;
    let magic = match data.get(0..4)? {
        [b'I', b'I', a, b] => u16::from_le_bytes([*a, *b]),
        [_, _, a, b] => u16::from_be_bytes([*a, *b]),
        _ => return None,
    };
    if ORF_MAGIC.contains(&magic) {
        return Some(ImageFormat::Raw(RawFormat::Orf));
    }
    if magic == RW2_MAGIC {
        return Some(ImageFormat::Raw(RawFormat::Rw2));
    }
    if data.get(8..10) == Some(b"CR") {
        return Some(ImageFormat::Raw(RawFormat::Cr2));
    }

    let dirs = reader.all_ifds();
    let ifd0 = dirs.first();
    if ifd0.is_some_and(|i| i.get(tag::DNG_VERSION).is_some()) {
        return Some(ImageFormat::Raw(RawFormat::Dng));
    }
    let has_sensor_data = dirs.iter().any(|ifd| {
        ifd.get(tag::PHOTOMETRIC)
            .and_then(|e| reader.uint(e))
            .is_some_and(|p| p == photometric::CFA || p == photometric::LINEAR_RAW)
    });
    if !has_sensor_data {
        return Some(ImageFormat::Tiff);
    }
    let make = ifd0
        .and_then(|i| i.get(tag::MAKE))
        .and_then(|e| reader.ascii(e))
        .unwrap_or_default()
        .to_ascii_uppercase();
    let raw = if make.starts_with("NIKON") {
        RawFormat::Nef
    } else if make.starts_with("SONY") {
        RawFormat::Arw
    } else if make.starts_with("PENTAX") || make.starts_with("RICOH") {
        RawFormat::Pef
    } else if make.starts_with("SAMSUNG") {
        RawFormat::Srw
    } else {
        RawFormat::OtherTiff
    };
    Some(ImageFormat::Raw(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_simple_magics() {
        assert_eq!(
            detect_format(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(
            detect_format(b"\x89PNG\r\n\x1a\n...."),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            detect_format(b"RIFF\0\0\0\0WEBPVP8 "),
            Some(ImageFormat::WebP)
        );
        assert_eq!(
            detect_format(b"FUJIFILMCCD-RAW 0201"),
            Some(ImageFormat::Raw(RawFormat::Raf))
        );
        assert_eq!(
            detect_format(b"IIRO\x08\0\0\0"),
            Some(ImageFormat::Raw(RawFormat::Orf))
        );
        assert_eq!(
            detect_format(b"IIU\0\x08\0\0\0"),
            Some(ImageFormat::Raw(RawFormat::Rw2))
        );
        assert_eq!(
            detect_format(b"II*\0\x10\0\0\0CR\x02\0"),
            Some(ImageFormat::Raw(RawFormat::Cr2))
        );
        assert_eq!(detect_format(b"hello world"), None);
        assert_eq!(detect_format(b""), None);
    }

    #[test]
    fn detects_isobmff_brands() {
        let mut heic = 24_u32.to_be_bytes().to_vec();
        heic.extend_from_slice(b"ftypheic\0\0\0\0mif1heic");
        assert_eq!(detect_format(&heic), Some(ImageFormat::Heif));
        let mut cr3 = 24_u32.to_be_bytes().to_vec();
        cr3.extend_from_slice(b"ftypcrx \0\0\0\x01crx isom");
        assert_eq!(detect_format(&cr3), Some(ImageFormat::Raw(RawFormat::Cr3)));
        let mut avif = 20_u32.to_be_bytes().to_vec();
        avif.extend_from_slice(b"ftypavif\0\0\0\0mif1");
        assert_eq!(detect_format(&avif), Some(ImageFormat::Avif));
    }

    #[test]
    fn truncated_input_does_not_panic() {
        let samples: [&[u8]; 4] = [
            b"II*\0\x08\0\0\0\x01\0",
            b"\0\0\0\x18ftypheic",
            b"RIFF\0\0",
            b"MM\0*\0\0\0\x08",
        ];
        for s in samples {
            for n in 0..=s.len() {
                let _ = detect_format(&s[..n]);
            }
        }
    }
}
