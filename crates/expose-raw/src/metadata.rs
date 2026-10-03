//! Photo metadata (EXIF) read.
//!
//! [`read_metadata`] reads the container header and the EXIF data. It does
//! not decode pixels, so that the catalog can call it fast at import.
//!
//! Sources:
//!
//! - JPEG, PNG, WebP, HEIF, TIFF, and TIFF-based RAW files (DNG, CR2, NEF,
//!   ARW, PEF, SRW, ORF, RW2, ...): `kamadak-exif` (BSD-2-Clause).
//! - RAF: the EXIF data of the embedded JPEG preview.
//! - CR3, CRW, X3F, MRW: the RAW backend `rawler` (feature `rawler`).

use std::io::Cursor;

use serde::{Deserialize, Serialize};

use crate::error::{DecodeError, DecodeResult};
use crate::format::{ImageFormat, RawFormat, detect_format};
use crate::orientation::Orientation;

/// The state of the EXIF data in a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "status", content = "message")]
pub enum ExifStatus {
    /// EXIF data was found and read without errors.
    Present,
    /// EXIF data was found, but some fields were not valid and were
    /// skipped.
    Partial(String),
    /// The file has no EXIF data.
    Absent,
    /// EXIF data was found but could not be read.
    Invalid(String),
    /// This build cannot read EXIF data from this format.
    Unsupported,
}

/// An exposure time as the file stores it, and in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExposureTime {
    /// Numerator of the stored rational value.
    pub numerator: u32,
    /// Denominator of the stored rational value (not 0).
    pub denominator: u32,
    /// The value in seconds.
    pub seconds: f64,
}

/// A parsed local date and time. No time zone is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDateTime {
    /// Year.
    pub year: u16,
    /// Month (1 to 12).
    pub month: u8,
    /// Day of month (1 to 31).
    pub day: u8,
    /// Hour (0 to 23).
    pub hour: u8,
    /// Minute (0 to 59).
    pub minute: u8,
    /// Second (0 to 60; 60 is a leap second).
    pub second: u8,
    /// Fraction of the second in nanoseconds, if the file records it.
    pub nanosecond: Option<u32>,
}

/// Which EXIF field the capture time comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureTimeSource {
    /// `DateTimeOriginal` (time of capture).
    Original,
    /// `DateTimeDigitized`.
    Digitized,
    /// `DateTime` (time of the last file change).
    Modified,
}

/// The capture date and time.
///
/// The original strings are kept. A time zone offset is given only if the
/// file records one; no time zone is invented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTime {
    /// The original EXIF string (`YYYY:MM:DD HH:MM:SS`, local time).
    pub original: String,
    /// The parsed local value, if `original` is valid.
    pub local: Option<LocalDateTime>,
    /// The offset from UTC in minutes, if the file records it.
    pub offset_minutes: Option<i16>,
    /// The original offset string (for example `+02:00`), if present.
    pub offset_original: Option<String>,
    /// The field that the value comes from.
    pub source: CaptureTimeSource,
}

/// A GPS position in decimal degrees (WGS 84 as EXIF defines).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpsPosition {
    /// Latitude in degrees, positive north (-90 to 90).
    pub latitude: f64,
    /// Longitude in degrees, positive east (-180 to 180).
    pub longitude: f64,
    /// Altitude in meters, negative below sea level, if present.
    pub altitude_meters: Option<f64>,
}

/// Photo metadata.
///
/// `width` and `height` are the stored pixel size, before orientation is
/// applied. For RAW files they are the size of the recommended crop (or,
/// without a RAW backend, the EXIF pixel size if present).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoMetadata {
    /// The detected file format.
    pub format: ImageFormat,
    /// State of the EXIF data.
    pub exif_status: ExifStatus,
    /// Stored width in pixels.
    pub width: Option<u32>,
    /// Stored height in pixels.
    pub height: Option<u32>,
    /// Orientation. `None` if the file does not record it.
    pub orientation: Option<Orientation>,
    /// Camera maker.
    pub make: Option<String>,
    /// Camera model.
    pub model: Option<String>,
    /// Lens maker.
    pub lens_make: Option<String>,
    /// Lens model.
    pub lens: Option<String>,
    /// Focal length in millimeters.
    pub focal_length_mm: Option<f64>,
    /// Focal length for a 35 mm film camera, in millimeters.
    pub focal_length_35mm: Option<u32>,
    /// Aperture as an f-number (for example 2.8).
    pub f_number: Option<f64>,
    /// Exposure time.
    pub exposure_time: Option<ExposureTime>,
    /// ISO sensitivity.
    pub iso: Option<u32>,
    /// Exposure compensation in EV.
    pub exposure_bias_ev: Option<f64>,
    /// Capture date and time.
    pub capture_time: Option<CaptureTime>,
    /// GPS position.
    pub gps: Option<GpsPosition>,
    /// The EXIF `ColorSpace` value (1 = sRGB, 65535 = uncalibrated).
    pub exif_color_space: Option<u16>,
    /// The EXIF interoperability index (for example `R98` or `R03`).
    pub interop_index: Option<String>,
}

impl PhotoMetadata {
    pub(crate) fn empty(format: ImageFormat, exif_status: ExifStatus) -> Self {
        Self {
            format,
            exif_status,
            width: None,
            height: None,
            orientation: None,
            make: None,
            model: None,
            lens_make: None,
            lens: None,
            focal_length_mm: None,
            focal_length_35mm: None,
            f_number: None,
            exposure_time: None,
            iso: None,
            exposure_bias_ev: None,
            capture_time: None,
            gps: None,
            exif_color_space: None,
            interop_index: None,
        }
    }
}

/// Reads the metadata of an image file without decoding its pixels.
///
/// Returns [`DecodeError::UnknownFormat`] if the format is not known. Bad
/// EXIF data does not cause an error: [`PhotoMetadata::exif_status`] tells
/// the state. A standard image (JPEG, PNG, TIFF, WebP) with a header that
/// is not valid gives an error.
pub fn read_metadata(data: &[u8]) -> DecodeResult<PhotoMetadata> {
    let format = detect_format(data).ok_or(DecodeError::UnknownFormat)?;
    read_metadata_for(data, format, true)
}

/// Reads metadata of a file with a known format. With `raw_dims`, the RAW
/// crop size is read from the RAW backend (an extra header parse).
pub(crate) fn read_metadata_for(
    data: &[u8],
    format: ImageFormat,
    raw_dims: bool,
) -> DecodeResult<PhotoMetadata> {
    let mut meta = match format {
        ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP => {
            let mut m = from_kamadak(format, read_container(data));
            let (w, h) = standard_dimensions(data, format)?;
            m.width = Some(w);
            m.height = Some(h);
            m
        }
        ImageFormat::Heif | ImageFormat::Avif => from_kamadak(format, read_container(data)),
        ImageFormat::Tiff => {
            let mut m = from_kamadak(format, read_tiff(data, None));
            let (w, h) = standard_dimensions(data, format)?;
            m.width = Some(w);
            m.height = Some(h);
            m
        }
        ImageFormat::Raw(raw) if raw.is_tiff_based() => {
            from_kamadak(format, read_tiff(data, Some(raw)))
        }
        ImageFormat::Raw(RawFormat::Raf) => from_kamadak(format, read_raf(data)),
        ImageFormat::Raw(_) => raw_backend_metadata(data, format),
    };
    if raw_dims && format.is_raw() {
        raw_dimensions(data, &mut meta);
    }
    Ok(meta)
}

/// Reads the metadata of a file (feature `native`).
#[cfg(feature = "native")]
pub fn read_metadata_path(path: impl AsRef<std::path::Path>) -> DecodeResult<PhotoMetadata> {
    let data = std::fs::read(path)?;
    read_metadata(&data)
}

/// Reads the width and height from the header of a standard format.
fn standard_dimensions(data: &[u8], format: ImageFormat) -> DecodeResult<(u32, u32)> {
    let f = match format {
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::WebP => image::ImageFormat::WebP,
        ImageFormat::Tiff => image::ImageFormat::Tiff,
        _ => {
            return Err(DecodeError::UnsupportedFormat {
                format,
                reason: "no header reader for this format",
            });
        }
    };
    Ok(image::ImageReader::with_format(Cursor::new(data), f).into_dimensions()?)
}

/// Sets the RAW crop size: for DNG from the TIFF directories (no pixel
/// decode); for other formats from a header-only decode of the RAW backend.
/// If neither is available, the EXIF pixel size (if any) stays.
fn raw_dimensions(data: &[u8], meta: &mut PhotoMetadata) {
    let size = if meta.format == ImageFormat::Raw(RawFormat::Dng) {
        crate::tiff::TiffReader::new(data, &[]).and_then(|r| r.dng_crop_size())
    } else {
        backend_crop_size(data)
    };
    if let Some((w, h)) = size {
        meta.width = Some(w);
        meta.height = Some(h);
    }
}

#[cfg(feature = "rawler")]
fn backend_crop_size(data: &[u8]) -> Option<(u32, u32)> {
    rawler_meta::crop_size(data)
}

#[cfg(not(feature = "rawler"))]
fn backend_crop_size(_data: &[u8]) -> Option<(u32, u32)> {
    None
}

type ExifResult = Result<exif::Exif, exif::Error>;

fn reader() -> exif::Reader {
    let mut r = exif::Reader::new();
    r.continue_on_error(true);
    r
}

fn read_container(data: &[u8]) -> ExifResult {
    reader().read_from_container(&mut Cursor::new(data))
}

/// Reads EXIF from a TIFF structure. ORF and RW2 files have a non-standard
/// magic number; a copy with the standard number is parsed.
fn read_tiff(data: &[u8], raw: Option<RawFormat>) -> ExifResult {
    let mut buf = data.to_vec();
    if matches!(raw, Some(RawFormat::Orf | RawFormat::Rw2)) && buf.len() >= 4 {
        let magic = if buf[0] == b'I' { [42, 0] } else { [0, 42] };
        buf[2..4].copy_from_slice(&magic);
    }
    reader().read_raw(buf)
}

/// Reads EXIF from the JPEG preview of a RAF file. The header stores the
/// JPEG offset and length (big-endian) at bytes 84 and 88.
fn read_raf(data: &[u8]) -> ExifResult {
    let be = |p: usize| {
        data.get(p..p + 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_be_bytes)
    };
    let jpeg = be(84)
        .zip(be(88))
        .and_then(|(o, l)| {
            let o = usize::try_from(o).ok()?;
            data.get(o..o.checked_add(usize::try_from(l).ok()?)?)
        })
        .ok_or(exif::Error::InvalidFormat("RAF header has no valid JPEG"))?;
    read_container(jpeg)
}

fn from_kamadak(format: ImageFormat, result: ExifResult) -> PhotoMetadata {
    let (exif, status) = match result {
        Ok(e) => (e, ExifStatus::Present),
        Err(exif::Error::PartialResult(p)) => {
            let (e, errors) = p.into_inner();
            let msg = errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ");
            (e, ExifStatus::Partial(msg))
        }
        Err(exif::Error::NotFound(_)) => return PhotoMetadata::empty(format, ExifStatus::Absent),
        Err(e) => return PhotoMetadata::empty(format, ExifStatus::Invalid(e.to_string())),
    };
    kamadak::fill(format, &exif, status)
}

/// Mapping of `kamadak-exif` fields.
mod kamadak {
    use exif::{Exif, In, Tag, Value};

    use super::{
        CaptureTime, CaptureTimeSource, ExifStatus, ExposureTime, GpsPosition, PhotoMetadata,
        parse_capture_time, valid_gps,
    };
    use crate::format::ImageFormat;
    use crate::orientation::Orientation;

    fn value(e: &Exif, t: Tag) -> Option<&Value> {
        e.get_field(t, In::PRIMARY).map(|f| &f.value)
    }

    fn ascii(e: &Exif, t: Tag) -> Option<String> {
        match value(e, t)? {
            Value::Ascii(v) => {
                let s = String::from_utf8_lossy(v.first()?);
                let s = s.trim_matches(['\0', ' ']);
                (!s.is_empty()).then(|| s.to_string())
            }
            _ => None,
        }
    }

    fn uint(e: &Exif, t: Tag) -> Option<u32> {
        value(e, t)?.get_uint(0)
    }

    fn rational(e: &Exif, t: Tag, i: usize) -> Option<(u32, u32)> {
        match value(e, t)? {
            Value::Rational(v) => v.get(i).filter(|r| r.denom != 0).map(|r| (r.num, r.denom)),
            _ => None,
        }
    }

    fn real(e: &Exif, t: Tag) -> Option<f64> {
        match value(e, t)? {
            Value::Rational(v) => v
                .first()
                .filter(|r| r.denom != 0)
                .map(exif::Rational::to_f64),
            Value::SRational(v) => v
                .first()
                .filter(|r| r.denom != 0)
                .map(exif::SRational::to_f64),
            _ => None,
        }
    }

    fn dms(e: &Exif, t: Tag) -> Option<f64> {
        let d = rational(e, t, 0)?;
        let m = rational(e, t, 1)?;
        let s = rational(e, t, 2)?;
        let f = |(n, dd): (u32, u32)| f64::from(n) / f64::from(dd);
        Some(f(d) + f(m) / 60.0 + f(s) / 3600.0)
    }

    fn gps(e: &Exif) -> Option<GpsPosition> {
        let mut lat = dms(e, Tag::GPSLatitude)?;
        let mut lon = dms(e, Tag::GPSLongitude)?;
        if ascii(e, Tag::GPSLatitudeRef).is_some_and(|r| r.eq_ignore_ascii_case("S")) {
            lat = -lat;
        }
        if ascii(e, Tag::GPSLongitudeRef).is_some_and(|r| r.eq_ignore_ascii_case("W")) {
            lon = -lon;
        }
        let altitude = real(e, Tag::GPSAltitude).map(|a| {
            if uint(e, Tag::GPSAltitudeRef) == Some(1) {
                -a
            } else {
                a
            }
        });
        valid_gps(lat, lon, altitude)
    }

    fn capture_time(e: &Exif) -> Option<CaptureTime> {
        [
            (
                Tag::DateTimeOriginal,
                Tag::OffsetTimeOriginal,
                Tag::SubSecTimeOriginal,
                CaptureTimeSource::Original,
            ),
            (
                Tag::DateTimeDigitized,
                Tag::OffsetTimeDigitized,
                Tag::SubSecTimeDigitized,
                CaptureTimeSource::Digitized,
            ),
            (
                Tag::DateTime,
                Tag::OffsetTime,
                Tag::SubSecTime,
                CaptureTimeSource::Modified,
            ),
        ]
        .into_iter()
        .find_map(|(dt, off, sub, src)| {
            let s = ascii(e, dt)?;
            Some(parse_capture_time(
                s,
                ascii(e, off),
                ascii(e, sub).as_deref(),
                src,
            ))
        })
    }

    pub(super) fn fill(format: ImageFormat, e: &Exif, status: ExifStatus) -> PhotoMetadata {
        let mut m = PhotoMetadata::empty(format, status);
        m.orientation = uint(e, Tag::Orientation)
            .and_then(|v| u16::try_from(v).ok())
            .and_then(Orientation::from_exif);
        m.make = ascii(e, Tag::Make);
        m.model = ascii(e, Tag::Model);
        m.lens_make = ascii(e, Tag::LensMake);
        m.lens = ascii(e, Tag::LensModel);
        m.focal_length_mm = real(e, Tag::FocalLength).filter(|v| *v > 0.0);
        m.focal_length_35mm = uint(e, Tag::FocalLengthIn35mmFilm).filter(|v| *v > 0);
        m.f_number = real(e, Tag::FNumber).filter(|v| *v > 0.0).or_else(|| {
            // APEX aperture value: N = 2^(Av / 2).
            real(e, Tag::ApertureValue).map(|av| (av / 2.0).exp2())
        });
        m.exposure_time = rational(e, Tag::ExposureTime, 0).map(|(n, d)| ExposureTime {
            numerator: n,
            denominator: d,
            seconds: f64::from(n) / f64::from(d),
        });
        m.iso = uint(e, Tag::PhotographicSensitivity).filter(|v| *v > 0);
        m.exposure_bias_ev = real(e, Tag::ExposureBiasValue);
        m.capture_time = capture_time(e);
        m.gps = gps(e);
        m.exif_color_space = uint(e, Tag::ColorSpace).and_then(|v| u16::try_from(v).ok());
        m.interop_index = ascii(e, Tag::InteroperabilityIndex);
        m.width = uint(e, Tag::PixelXDimension).filter(|v| *v > 0);
        m.height = uint(e, Tag::PixelYDimension).filter(|v| *v > 0);
        m
    }
}

/// Returns a GPS position if the values are in range.
fn valid_gps(lat: f64, lon: f64, altitude: Option<f64>) -> Option<GpsPosition> {
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some(GpsPosition {
        latitude: lat,
        longitude: lon,
        altitude_meters: altitude.filter(|a| a.is_finite()),
    })
}

/// Parses an EXIF date-time string with optional offset and sub-second
/// strings. Values that are not valid give `None` in the parsed fields, but
/// the original string is always kept.
fn parse_capture_time(
    original: String,
    offset: Option<String>,
    subsec: Option<&str>,
    source: CaptureTimeSource,
) -> CaptureTime {
    let parsed = exif::DateTime::from_ascii(original.as_bytes()).ok();
    let local = parsed.and_then(|mut dt| {
        if let Some(s) = subsec {
            // A bad sub-second string only removes the fraction.
            if dt.parse_subsec(s.as_bytes()).is_err() {
                dt.nanosecond = None;
            }
        }
        let ok = (1..=12).contains(&dt.month)
            && (1..=31).contains(&dt.day)
            && dt.hour <= 23
            && dt.minute <= 59
            && dt.second <= 60;
        ok.then_some(LocalDateTime {
            year: dt.year,
            month: dt.month,
            day: dt.day,
            hour: dt.hour,
            minute: dt.minute,
            second: dt.second,
            nanosecond: dt.nanosecond,
        })
    });
    let offset_minutes = offset.as_deref().and_then(|o| {
        let mut dt = exif::DateTime::from_ascii(b"2000:01:01 00:00:00").ok()?;
        dt.parse_offset(o.as_bytes()).ok()?;
        dt.offset.filter(|m| m.abs() <= 18 * 60)
    });
    CaptureTime {
        original,
        local,
        offset_minutes,
        offset_original: offset,
        source,
    }
}

/// Metadata for formats that only the RAW backend can read.
fn raw_backend_metadata(data: &[u8], format: ImageFormat) -> PhotoMetadata {
    #[cfg(feature = "rawler")]
    {
        rawler_meta::read(data, format)
    }
    #[cfg(not(feature = "rawler"))]
    {
        let _ = data;
        PhotoMetadata::empty(format, ExifStatus::Unsupported)
    }
}

/// Metadata from the `rawler` backend (LGPL-2.1, operator approved).
#[cfg(feature = "rawler")]
mod rawler_meta {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use rawler::decoders::{RawDecodeParams, RawMetadata};
    use rawler::formats::tiff::Rational;
    use rawler::rawsource::RawSource;

    use super::{
        CaptureTimeSource, ExifStatus, ExposureTime, PhotoMetadata, parse_capture_time, valid_gps,
    };
    use crate::format::ImageFormat;
    use crate::orientation::Orientation;

    fn ratio(r: Rational) -> Option<f64> {
        (r.d != 0).then(|| f64::from(r.n) / f64::from(r.d))
    }

    fn non_empty(s: &str) -> Option<String> {
        let s = s.trim_matches(['\0', ' ']);
        (!s.is_empty()).then(|| s.to_string())
    }

    /// Returns the recommended crop size from a header-only decode.
    pub(super) fn crop_size(data: &[u8]) -> Option<(u32, u32)> {
        let src = RawSource::new_from_slice(data);
        let params = RawDecodeParams::default();
        catch_unwind(AssertUnwindSafe(|| {
            let decoder = rawler::get_decoder(&src).ok()?;
            let img = decoder.raw_image(&src, &params, true).ok()?;
            let (w, h) = img
                .crop_area
                .or(img.active_area)
                .map_or((img.width, img.height), |r| (r.d.w, r.d.h));
            Some((u32::try_from(w).ok()?, u32::try_from(h).ok()?))
        }))
        .ok()
        .flatten()
    }

    fn raw_metadata(data: &[u8]) -> Result<RawMetadata, String> {
        let src = RawSource::new_from_slice(data);
        let params = RawDecodeParams::default();
        catch_unwind(AssertUnwindSafe(|| {
            let decoder = rawler::get_decoder(&src).map_err(|e| e.to_string())?;
            decoder
                .raw_metadata(&src, &params)
                .map_err(|e| e.to_string())
        }))
        .unwrap_or_else(|_| Err("the RAW decoder stopped with a panic".to_string()))
    }

    pub(super) fn read(data: &[u8], format: ImageFormat) -> PhotoMetadata {
        let md = match raw_metadata(data) {
            Ok(md) => md,
            Err(e) => return PhotoMetadata::empty(format, ExifStatus::Invalid(e)),
        };
        let x = &md.exif;
        let mut m = PhotoMetadata::empty(format, ExifStatus::Present);
        m.orientation = x.orientation.and_then(Orientation::from_exif);
        m.make = non_empty(&md.make);
        m.model = non_empty(&md.model);
        m.lens_make = x.lens_make.as_deref().and_then(non_empty);
        m.lens = x
            .lens_model
            .as_deref()
            .and_then(non_empty)
            .or_else(|| md.lens.as_ref().and_then(|l| non_empty(&l.lens_model)));
        m.focal_length_mm = x.focal_length.and_then(ratio).filter(|v| *v > 0.0);
        m.f_number = x.fnumber.and_then(ratio).filter(|v| *v > 0.0);
        m.exposure_time = x.exposure_time.filter(|r| r.d != 0).map(|r| ExposureTime {
            numerator: r.n,
            denominator: r.d,
            seconds: f64::from(r.n) / f64::from(r.d),
        });
        m.iso = x
            .iso_speed_ratings
            .map(u32::from)
            .or(x.iso_speed)
            .or(x.recommended_exposure_index)
            .filter(|v| *v > 0);
        m.exposure_bias_ev = x
            .exposure_bias
            .as_ref()
            .and_then(|r| (r.d != 0).then(|| f64::from(r.n) / f64::from(r.d)));
        m.capture_time = [
            (
                &x.date_time_original,
                &x.offset_time_original,
                &x.sub_sec_time_original,
                CaptureTimeSource::Original,
            ),
            (
                &x.create_date,
                &x.offset_time_digitized,
                &x.sub_sec_time_digitized,
                CaptureTimeSource::Digitized,
            ),
            (
                &x.modify_date,
                &x.offset_time,
                &x.sub_sec_time,
                CaptureTimeSource::Modified,
            ),
        ]
        .into_iter()
        .find_map(|(dt, off, sub, src)| {
            let s = non_empty(dt.as_deref()?)?;
            Some(parse_capture_time(
                s,
                off.as_deref().and_then(non_empty),
                sub.as_deref(),
                src,
            ))
        });
        m.gps = x.gps.as_ref().and_then(|g| {
            let dms = |v: &[Rational; 3]| -> Option<f64> {
                Some(ratio(v[0])? + ratio(v[1])? / 60.0 + ratio(v[2])? / 3600.0)
            };
            let mut lat = dms(g.gps_latitude.as_ref()?)?;
            let mut lon = dms(g.gps_longitude.as_ref()?)?;
            if g.gps_latitude_ref
                .as_deref()
                .is_some_and(|r| r.starts_with('S'))
            {
                lat = -lat;
            }
            if g.gps_longitude_ref
                .as_deref()
                .is_some_and(|r| r.starts_with('W'))
            {
                lon = -lon;
            }
            let alt = g
                .gps_altitude
                .and_then(ratio)
                .map(|a| if g.gps_altitude_ref == Some(1) { -a } else { a });
            valid_gps(lat, lon, alt)
        });
        m.exif_color_space = x.color_space;
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_time_keeps_original_and_offset() {
        let t = parse_capture_time(
            "2024:06:30 18:45:10".into(),
            Some("+02:00".into()),
            Some("25"),
            CaptureTimeSource::Original,
        );
        assert_eq!(t.original, "2024:06:30 18:45:10");
        assert_eq!(t.offset_minutes, Some(120));
        let l = t.local.unwrap();
        assert_eq!((l.year, l.month, l.day, l.hour), (2024, 6, 30, 18));
        assert_eq!(l.nanosecond, Some(250_000_000));

        let bad = parse_capture_time(
            "2024:13:01 00:00:00".into(),
            None,
            None,
            CaptureTimeSource::Modified,
        );
        assert!(bad.local.is_none());
        assert_eq!(bad.offset_minutes, None);
        let junk = parse_capture_time(
            "junk".into(),
            Some("xx".into()),
            None,
            CaptureTimeSource::Original,
        );
        assert!(junk.local.is_none() && junk.offset_minutes.is_none());
    }

    #[test]
    fn gps_range_check() {
        assert!(valid_gps(45.0, 7.0, Some(100.0)).is_some());
        assert!(valid_gps(91.0, 7.0, None).is_none());
        assert!(valid_gps(f64::NAN, 7.0, None).is_none());
    }

    #[test]
    fn unknown_format_is_error() {
        assert!(matches!(
            read_metadata(b"plain text"),
            Err(DecodeError::UnknownFormat)
        ));
    }

    #[test]
    fn serializes_camel_case() {
        let m = PhotoMetadata::empty(ImageFormat::Jpeg, ExifStatus::Absent);
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains("\"exifStatus\":{\"status\":\"absent\"}"), "{j}");
        assert!(j.contains("\"focalLengthMm\":null"));
        assert!(j.contains("\"format\":{\"kind\":\"jpeg\"}"), "{j}");
    }
}
