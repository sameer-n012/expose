//! Capture time and image size of imported photos.
//!
//! After an import, the shell reads the metadata of each new file (no pixel
//! decode) and stores an [`ImageInfo`] in the catalog. This module has the
//! file work. It does not touch the catalog, so the caller can run it with
//! no catalog lock held.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use expose_catalog::{ImageInfo, PhotoId};
use expose_raw::{LocalDateTime, Orientation, PhotoMetadata};

use crate::error::CommandError;

/// The upper limit of the number of worker threads for metadata reads.
const MAX_WORKERS: usize = 8;

/// The metadata read for one file.
#[derive(Debug)]
pub struct MetadataJob {
    /// The photo in the catalog.
    pub id: PhotoId,
    /// The file to read.
    pub path: PathBuf,
}

/// The result of one [`MetadataJob`]: the image information, or the text of
/// the read error.
#[derive(Debug)]
pub struct MetadataOutcome {
    /// The photo in the catalog.
    pub id: PhotoId,
    /// The file that was read.
    pub path: PathBuf,
    /// The image information, or the reason why the file did not read.
    pub info: Result<ImageInfo, String>,
}

/// Converts a camera wall-clock time to Unix milliseconds.
///
/// **Time zone rule.** The result is the wall-clock time as if it were UTC
/// (a "floating" local time). `2024-05-01 14:30:00` gives the Unix
/// milliseconds of `2024-05-01T14:30:00Z`. The function ignores the offset
/// of the capture time. Thus a photo keeps the time that the camera showed,
/// and sorting by this value is the same in all time zones.
///
/// The fraction of a second is added as whole milliseconds (the rest is
/// dropped). A leap second (second 60) counts as one second after second 59
/// (the same value as the next minute, second 0): the Unix time scale has
/// no leap seconds, and this keeps the order of the times.
///
/// Returns `None` if a field is out of range: year 0, a month not in 1 to
/// 12, a day that the month does not have (the function knows leap years),
/// an hour above 23, a minute above 59, a second above 60, or a fraction of
/// one second or more.
#[must_use]
pub fn wall_clock_ms(time: &LocalDateTime) -> Option<i64> {
    let (year, month, day) = (i64::from(time.year), time.month, time.day);
    if year == 0 || !(1..=12).contains(&month) {
        return None;
    }
    if day == 0 || day > days_in_month(year, month) {
        return None;
    }
    if time.hour > 23 || time.minute > 59 || time.second > 60 {
        return None;
    }
    let millis = match time.nanosecond {
        Some(ns) if ns >= 1_000_000_000 => return None,
        Some(ns) => i64::from(ns / 1_000_000),
        None => 0,
    };
    let seconds = days_from_civil(year, month, day) * 86_400
        + i64::from(time.hour) * 3_600
        + i64::from(time.minute) * 60
        + i64::from(time.second);
    Some(seconds * 1_000 + millis)
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// The number of days in `month` (1 to 12).
fn days_in_month(year: i64, month: u8) -> u8 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The number of days from 1970-01-01 to the date (negative before). The
/// date must be valid. This is the "days from civil" algorithm.
fn days_from_civil(year: i64, month: u8, day: u8) -> i64 {
    let month = i64::from(month);
    // The year starts in March, so the leap day is the last day of a year.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Builds the [`ImageInfo`] of a photo from its metadata.
///
/// - Capture time: see [`wall_clock_ms`]. It is `None` if the file has no
///   capture time or the time is not valid.
/// - Size: the display size, which is the size after the EXIF orientation.
///   If the orientation turns the image by 90 or 270 degrees, width and
///   height are swapped. For a RAW file, the size is what the reader gives:
///   the crop size (the default crop of the file) if the reader finds it,
///   and otherwise the EXIF pixel size, or `None`. It is not the full
///   sensor size. A size of 0 is stored as `None`.
#[must_use]
pub fn image_info(meta: &PhotoMetadata) -> ImageInfo {
    let capture_time_ms = meta
        .capture_time
        .as_ref()
        .and_then(|time| time.local.as_ref())
        .and_then(wall_clock_ms);
    let swap = meta.orientation.is_some_and(Orientation::swaps_dimensions);
    let nonzero = |v: Option<u32>| v.filter(|&n| n > 0);
    let (width, height) = (nonzero(meta.width), nonzero(meta.height));
    let (width, height) = if swap {
        (height, width)
    } else {
        (width, height)
    };
    ImageInfo {
        capture_time_ms,
        width,
        height,
    }
}

/// Reads the metadata of each file and returns one outcome per job, in the
/// order of `jobs`.
///
/// The function reads files in parallel with a bounded number of threads
/// and does not touch the catalog. A file that fails gives an `Err` outcome;
/// it does not stop the other files.
///
/// # Errors
///
/// Returns an internal error if a worker thread panics.
pub fn read_image_infos(jobs: &[MetadataJob]) -> Result<Vec<MetadataOutcome>, CommandError> {
    let workers = thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(MAX_WORKERS)
        .min(jobs.len());
    if workers <= 1 {
        return Ok(jobs.iter().map(|j| read_path(j.id, &j.path)).collect());
    }
    let next = AtomicUsize::new(0);
    let jobs_ref = jobs;
    let mut indexed: Vec<(usize, MetadataOutcome)> = thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(job) = jobs_ref.get(index) else {
                            break;
                        };
                        done.push((index, read_path(job.id, &job.path)));
                    }
                    done
                })
            })
            .collect();
        let mut all = Vec::with_capacity(jobs_ref.len());
        for handle in handles {
            let done = handle
                .join()
                .map_err(|_| CommandError::internal("a metadata worker thread panicked"))?;
            all.extend(done);
        }
        Ok::<_, CommandError>(all)
    })?;
    indexed.sort_by_key(|(index, _)| *index);
    Ok(indexed.into_iter().map(|(_, outcome)| outcome).collect())
}

fn read_path(id: PhotoId, path: &std::path::Path) -> MetadataOutcome {
    let info = expose_raw::read_metadata_path(path)
        .map(|meta| image_info(&meta))
        .map_err(|error| format!("cannot read the metadata: {error}"));
    MetadataOutcome {
        id,
        path: path.to_path_buf(),
        info,
    }
}

/// A valid 1 x 1 PNG with no EXIF, for tests.
#[cfg(test)]
pub(crate) fn tiny_png() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]
}

#[cfg(test)]
mod tests {
    use expose_raw::{CaptureTime, CaptureTimeSource};

    use super::*;

    fn time(
        (year, month, day): (u16, u8, u8),
        (hour, minute, second): (u8, u8, u8),
    ) -> LocalDateTime {
        LocalDateTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
            nanosecond: None,
        }
    }

    #[test]
    fn epoch_is_zero() {
        assert_eq!(wall_clock_ms(&time((1970, 1, 1), (0, 0, 0))), Some(0));
    }

    #[test]
    fn known_dates() {
        // 2024-05-01T14:30:00Z
        assert_eq!(
            wall_clock_ms(&time((2024, 5, 1), (14, 30, 0))),
            Some(1_714_573_800_000)
        );
        // Leap day: 2024-02-29T00:00:00Z
        assert_eq!(
            wall_clock_ms(&time((2024, 2, 29), (0, 0, 0))),
            Some(1_709_164_800_000)
        );
        // Before the epoch: 1900-01-01T00:00:00Z
        assert_eq!(
            wall_clock_ms(&time((1900, 1, 1), (0, 0, 0))),
            Some(-2_208_988_800_000)
        );
    }

    #[test]
    fn invalid_dates_give_none() {
        assert_eq!(wall_clock_ms(&time((2023, 2, 29), (0, 0, 0))), None);
        assert_eq!(wall_clock_ms(&time((1900, 2, 29), (0, 0, 0))), None);
        assert_eq!(
            wall_clock_ms(&time((2000, 2, 29), (0, 0, 0))),
            Some(951_782_400_000)
        );
        assert_eq!(wall_clock_ms(&time((2024, 0, 1), (0, 0, 0))), None);
        assert_eq!(wall_clock_ms(&time((2024, 13, 1), (0, 0, 0))), None);
        assert_eq!(wall_clock_ms(&time((2024, 4, 31), (0, 0, 0))), None);
        assert_eq!(wall_clock_ms(&time((2024, 4, 0), (0, 0, 0))), None);
        assert_eq!(wall_clock_ms(&time((0, 1, 1), (0, 0, 0))), None);
        assert_eq!(wall_clock_ms(&time((2024, 1, 1), (24, 0, 0))), None);
        assert_eq!(wall_clock_ms(&time((2024, 1, 1), (0, 60, 0))), None);
        assert_eq!(wall_clock_ms(&time((2024, 1, 1), (0, 0, 61))), None);
    }

    #[test]
    fn leap_second_counts_as_next_second() {
        let leap = wall_clock_ms(&time((2016, 12, 31), (23, 59, 60)));
        let next = wall_clock_ms(&time((2017, 1, 1), (0, 0, 0)));
        assert_eq!(leap, next);
        assert!(leap.is_some());
    }

    #[test]
    fn fraction_adds_whole_milliseconds() {
        let mut t = time((1970, 1, 1), (0, 0, 1));
        t.nanosecond = Some(123_999_999);
        assert_eq!(wall_clock_ms(&t), Some(1_123));
        t.nanosecond = Some(1_000_000_000);
        assert_eq!(wall_clock_ms(&t), None);
    }

    fn metadata(width: u32, height: u32, orientation: Option<Orientation>) -> PhotoMetadata {
        let mut meta = expose_raw::read_metadata(&tiny_png()).unwrap();
        meta.width = Some(width);
        meta.height = Some(height);
        meta.orientation = orientation;
        meta
    }

    #[test]
    fn size_swaps_for_quarter_turns_only() {
        let swapped = [
            Orientation::Transpose,
            Orientation::Rotate90,
            Orientation::Transverse,
            Orientation::Rotate270,
        ];
        for o in swapped {
            let info = image_info(&metadata(6000, 4000, Some(o)));
            assert_eq!((info.width, info.height), (Some(4000), Some(6000)), "{o:?}");
        }
        for o in [
            None,
            Some(Orientation::Normal),
            Some(Orientation::Rotate180),
        ] {
            let info = image_info(&metadata(6000, 4000, o));
            assert_eq!((info.width, info.height), (Some(6000), Some(4000)), "{o:?}");
        }
    }

    #[test]
    fn zero_size_is_unknown() {
        let info = image_info(&metadata(0, 10, None));
        assert_eq!((info.width, info.height), (None, Some(10)));
    }

    #[test]
    fn capture_time_comes_from_local_value() {
        let mut meta = metadata(1, 1, None);
        let local = time((2024, 5, 1), (14, 30, 0));
        meta.capture_time = Some(CaptureTime {
            original: "2024:05:01 14:30:00".into(),
            local: Some(local),
            offset_minutes: Some(-300),
            offset_original: Some("-05:00".into()),
            source: CaptureTimeSource::Original,
        });
        // The offset does not change the value.
        assert_eq!(image_info(&meta).capture_time_ms, Some(1_714_573_800_000));
        meta.capture_time = Some(CaptureTime {
            original: "garbage".into(),
            local: None,
            offset_minutes: None,
            offset_original: None,
            source: CaptureTimeSource::Original,
        });
        assert_eq!(image_info(&meta).capture_time_ms, None);
    }

    #[test]
    fn parallel_read_keeps_job_order_and_reports_failures() {
        let dir = tempfile::tempdir().unwrap();
        let mut jobs = Vec::new();
        for n in 0..20_i64 {
            let path = dir.path().join(format!("{n}.png"));
            if n % 5 == 0 {
                std::fs::write(&path, b"not an image").unwrap();
            } else {
                std::fs::write(&path, tiny_png()).unwrap();
            }
            jobs.push(MetadataJob {
                id: PhotoId(n),
                path,
            });
        }
        let outcomes = read_image_infos(&jobs).unwrap();
        assert_eq!(outcomes.len(), 20);
        for (n, outcome) in (0_i64..).zip(&outcomes) {
            assert_eq!(outcome.id, PhotoId(n));
            assert_eq!(outcome.info.is_err(), n % 5 == 0, "{n}");
        }
    }

    #[test]
    fn empty_job_list_gives_no_outcomes() {
        assert!(read_image_infos(&[]).unwrap().is_empty());
    }
}
