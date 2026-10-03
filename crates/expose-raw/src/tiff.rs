//! A small, bounds-checked TIFF directory (IFD) reader.
//!
//! This reader is for format detection, and for the ICC profile of small
//! TIFF files (see `standard.rs`). It reads tag values of a few simple
//! types. It never panics on bad input: each read is bounds-checked
//! and returns `None` when the data is not valid.

/// Byte order of a TIFF file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ByteOrder {
    /// Intel byte order (`II`).
    Little,
    /// Motorola byte order (`MM`).
    Big,
}

/// Maximum number of entries that one IFD can have. Real files have less
/// than a few hundred entries. This limit stops very long loops on bad data.
const MAX_ENTRIES: u16 = 1024;

/// Maximum number of IFDs that [`TiffReader::all_ifds`] visits.
const MAX_IFDS: usize = 32;

/// TIFF tag numbers that format detection uses.
pub(crate) mod tag {
    /// Image width in pixels.
    pub const IMAGE_WIDTH: u16 = 256;
    /// Image height in pixels.
    pub const IMAGE_LENGTH: u16 = 257;
    /// Photometric interpretation.
    pub const PHOTOMETRIC: u16 = 262;
    /// Camera maker name.
    pub const MAKE: u16 = 271;
    /// Offsets of sub-IFDs.
    pub const SUB_IFDS: u16 = 330;
    /// DNG version (DNG files only).
    pub const DNG_VERSION: u16 = 50706;
    /// DNG default crop size.
    pub const DEFAULT_CROP_SIZE: u16 = 50720;
    /// Embedded ICC profile.
    pub const ICC_PROFILE: u16 = 34675;
}

/// Photometric interpretation values of sensor data.
pub(crate) mod photometric {
    /// Color filter array (mosaic) data.
    pub const CFA: u32 = 32803;
    /// Linear raw data (demosaiced sensor data).
    pub const LINEAR_RAW: u32 = 34892;
}

/// One directory entry.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Entry {
    /// Tag number.
    pub tag: u16,
    /// Field type.
    pub typ: u16,
    /// Number of values.
    pub count: u32,
    /// Offset of the 4-byte value/offset field in the file.
    pub field_pos: usize,
}

/// A parsed IFD: its entries and the offset of the next IFD (0 = none).
#[derive(Debug, Clone)]
pub(crate) struct Ifd {
    pub entries: Vec<Entry>,
    pub next: u32,
}

impl Ifd {
    /// Returns the entry with `tag`, if present.
    pub fn get(&self, tag: u16) -> Option<&Entry> {
        self.entries.iter().find(|e| e.tag == tag)
    }
}

/// A TIFF reader over a byte slice.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TiffReader<'a> {
    data: &'a [u8],
    order: ByteOrder,
    first_ifd: u32,
}

impl<'a> TiffReader<'a> {
    /// Reads the 8-byte header. Accepts the magic number 42 (standard) and
    /// the variants `extra_magic` (for example, 0x4F52 and 0x0055 of some
    /// camera formats). Returns `None` if the header is not valid.
    pub fn new(data: &'a [u8], extra_magic: &[u16]) -> Option<Self> {
        let order = match data.get(0..2)? {
            b"II" => ByteOrder::Little,
            b"MM" => ByteOrder::Big,
            _ => return None,
        };
        let mut r = Self {
            data,
            order,
            first_ifd: 0,
        };
        let magic = r.u16_at(2)?;
        if magic != 42 && !extra_magic.contains(&magic) {
            return None;
        }
        r.first_ifd = r.u32_at(4)?;
        Some(r)
    }

    /// Returns the offset of the first IFD.
    #[cfg(test)]
    pub fn first_ifd(&self) -> u32 {
        self.first_ifd
    }

    fn bytes(&self, pos: usize, len: usize) -> Option<&'a [u8]> {
        self.data.get(pos..pos.checked_add(len)?)
    }

    fn u16_at(&self, pos: usize) -> Option<u16> {
        let b: [u8; 2] = self.bytes(pos, 2)?.try_into().ok()?;
        Some(match self.order {
            ByteOrder::Little => u16::from_le_bytes(b),
            ByteOrder::Big => u16::from_be_bytes(b),
        })
    }

    fn u32_at(&self, pos: usize) -> Option<u32> {
        let b: [u8; 4] = self.bytes(pos, 4)?.try_into().ok()?;
        Some(match self.order {
            ByteOrder::Little => u32::from_le_bytes(b),
            ByteOrder::Big => u32::from_be_bytes(b),
        })
    }

    /// Reads the IFD at `offset`. Returns `None` if it is not valid.
    pub fn ifd(&self, offset: u32) -> Option<Ifd> {
        let pos = usize::try_from(offset).ok()?;
        let n = self.u16_at(pos)?;
        if n == 0 || n > MAX_ENTRIES {
            return None;
        }
        let mut entries = Vec::with_capacity(usize::from(n));
        for i in 0..usize::from(n) {
            let e = pos.checked_add(2)?.checked_add(i.checked_mul(12)?)?;
            entries.push(Entry {
                tag: self.u16_at(e)?,
                typ: self.u16_at(e + 2)?,
                count: self.u32_at(e + 4)?,
                field_pos: e + 8,
            });
        }
        let next_pos = pos.checked_add(2)?.checked_add(usize::from(n) * 12)?;
        // A missing next-IFD field is common in truncated files; treat it as
        // the end of the chain.
        let next = self.u32_at(next_pos).unwrap_or(0);
        Some(Ifd { entries, next })
    }

    /// Returns the byte position of the value data of `e`, and the size of
    /// one value. Returns `None` for unknown types or out-of-range data.
    fn value_pos(&self, e: &Entry) -> Option<(usize, usize)> {
        let size: usize = match e.typ {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 | 13 => 4,
            5 | 10 | 12 => 8,
            _ => return None,
        };
        let total = size.checked_mul(usize::try_from(e.count).ok()?)?;
        let pos = if total <= 4 {
            e.field_pos
        } else {
            usize::try_from(self.u32_at(e.field_pos)?).ok()?
        };
        self.bytes(pos, total)?;
        Some((pos, size))
    }

    /// Returns the integer values of `e` (types BYTE, SHORT, LONG, IFD).
    /// At most `max` values are read.
    pub fn uints(&self, e: &Entry, max: usize) -> Option<Vec<u32>> {
        let (pos, size) = self.value_pos(e)?;
        let count = usize::try_from(e.count).ok()?.min(max);
        (0..count)
            .map(|i| {
                let p = pos + i * size;
                match e.typ {
                    1 => self.data.get(p).copied().map(u32::from),
                    3 => self.u16_at(p).map(u32::from),
                    4 | 13 => self.u32_at(p),
                    _ => None,
                }
            })
            .collect()
    }

    /// Returns the ICC profile (tag 34675, type BYTE or UNDEFINED) of the
    /// first IFD, if present and inside the data.
    pub fn icc_profile(&self) -> Option<&'a [u8]> {
        let ifd = self.ifd(self.first_ifd)?;
        let e = ifd.get(tag::ICC_PROFILE)?;
        if e.typ != 1 && e.typ != 7 {
            return None;
        }
        let (pos, _) = self.value_pos(e)?;
        self.bytes(pos, usize::try_from(e.count).ok()?)
    }

    /// Returns the first integer value of `e`.
    pub fn uint(&self, e: &Entry) -> Option<u32> {
        self.uints(e, 1)?.first().copied()
    }

    /// Returns the ASCII value of `e`, without trailing NUL bytes and
    /// spaces. Bytes that are not valid UTF-8 are replaced.
    pub fn ascii(&self, e: &Entry) -> Option<String> {
        if e.typ != 2 {
            return None;
        }
        let (pos, _) = self.value_pos(e)?;
        let bytes = self.bytes(pos, usize::try_from(e.count).ok()?)?;
        let s = String::from_utf8_lossy(bytes);
        Some(s.trim_end_matches(['\0', ' ']).to_string())
    }

    /// Returns value `i` of `e` as a number (types BYTE, SHORT, LONG,
    /// RATIONAL). Returns `None` for other types or a zero denominator.
    pub fn number(&self, e: &Entry, i: usize) -> Option<f64> {
        if usize::try_from(e.count).ok()? <= i {
            return None;
        }
        if e.typ == 5 {
            let (pos, size) = self.value_pos(e)?;
            let p = pos.checked_add(i.checked_mul(size)?)?;
            let n = self.u32_at(p)?;
            let d = self.u32_at(p + 4)?;
            return (d != 0).then(|| f64::from(n) / f64::from(d));
        }
        self.uints(e, i + 1)?.get(i).map(|&v| f64::from(v))
    }

    /// Returns the default crop size of the sensor data of a DNG file: the
    /// `DefaultCropSize` of the first IFD with CFA or linear raw data, or
    /// its full size if the tag is missing.
    pub fn dng_crop_size(&self) -> Option<(u32, u32)> {
        let ifds = self.all_ifds();
        let raw = ifds.iter().find(|ifd| {
            ifd.get(tag::PHOTOMETRIC)
                .and_then(|e| self.uint(e))
                .is_some_and(|p| p == photometric::CFA || p == photometric::LINEAR_RAW)
        })?;
        // Values are checked to be in the u32 range before the cast.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the range is checked first"
        )]
        let to_u32 = |v: f64| (v >= 1.0 && v <= f64::from(u32::MAX)).then(|| v.round() as u32);
        let crop = raw
            .get(tag::DEFAULT_CROP_SIZE)
            .and_then(|e| Some((to_u32(self.number(e, 0)?)?, to_u32(self.number(e, 1)?)?)));
        crop.or_else(|| {
            let w = self.uint(raw.get(tag::IMAGE_WIDTH)?)?;
            let h = self.uint(raw.get(tag::IMAGE_LENGTH)?)?;
            Some((w, h))
        })
    }

    /// Returns the size of the largest image (by pixel count) that the
    /// IFDs declare, or `None` if no IFD declares a size.
    #[cfg_attr(
        not(feature = "rawler"),
        allow(dead_code, reason = "used by the rawler backend only")
    )]
    pub fn largest_image(&self) -> Option<(u64, u64)> {
        self.all_ifds()
            .iter()
            .filter_map(|ifd| {
                let w = self.uint(ifd.get(tag::IMAGE_WIDTH)?)?;
                let h = self.uint(ifd.get(tag::IMAGE_LENGTH)?)?;
                Some((u64::from(w), u64::from(h)))
            })
            .max_by_key(|(w, h)| w * h)
    }

    /// Returns IFD0, the IFDs in its chain, and their sub-IFDs (one level),
    /// up to a fixed limit. Loops in the chain are detected and stopped.
    pub fn all_ifds(&self) -> Vec<Ifd> {
        let mut out = Vec::new();
        let mut visited = Vec::new();
        let mut queue = vec![self.first_ifd];
        while let Some(off) = queue.pop() {
            if off == 0 || visited.contains(&off) || out.len() >= MAX_IFDS {
                continue;
            }
            visited.push(off);
            let Some(ifd) = self.ifd(off) else { continue };
            queue.push(ifd.next);
            if let Some(subs) = ifd.get(tag::SUB_IFDS).and_then(|e| self.uints(e, 16)) {
                queue.extend(subs);
            }
            out.push(ifd);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le_tiff_with_make(make: &[u8]) -> Vec<u8> {
        let mut d = b"II*\0\x08\0\0\0".to_vec();
        d.extend_from_slice(&1_u16.to_le_bytes());
        d.extend_from_slice(&tag::MAKE.to_le_bytes());
        d.extend_from_slice(&2_u16.to_le_bytes());
        let len = u32::try_from(make.len()).unwrap_or(0);
        d.extend_from_slice(&len.to_le_bytes());
        d.extend_from_slice(&26_u32.to_le_bytes());
        d.extend_from_slice(&0_u32.to_le_bytes());
        d.extend_from_slice(make);
        d
    }

    #[test]
    fn reads_ascii_tag() {
        let d = le_tiff_with_make(b"NIKON CORPORATION\0");
        let r = TiffReader::new(&d, &[]).unwrap();
        let ifd = r.ifd(r.first_ifd()).unwrap();
        let make = r.ascii(ifd.get(tag::MAKE).unwrap()).unwrap();
        assert_eq!(make, "NIKON CORPORATION");
        assert_eq!(r.all_ifds().len(), 1);
    }

    #[test]
    fn bad_input_gives_none() {
        assert!(TiffReader::new(b"II", &[]).is_none());
        assert!(TiffReader::new(b"II+\0\0\0\0\0", &[]).is_none());
        let mut d = le_tiff_with_make(b"X\0");
        // Point the first IFD outside the data.
        d[4] = 0xFF;
        let r = TiffReader::new(&d, &[]).unwrap();
        assert!(r.ifd(r.first_ifd()).is_none());
        assert!(r.all_ifds().is_empty());
        // Truncate at each position: no panic.
        let full = le_tiff_with_make(b"SONY\0");
        for n in 0..full.len() {
            if let Some(r) = TiffReader::new(&full[..n], &[]) {
                let _ = r.all_ifds();
            }
        }
    }
}
