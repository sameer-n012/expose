//! The thumbnail cache of the library grid.
//!
//! A thumbnail is a JPEG file (sRGB, long edge [`THUMB_LONG_EDGE`]) in the
//! folder `<app cache dir>/thumbnails/`. This module has no Tauri
//! dependency, so tests can call it. The `thumb` URI scheme
//! ([`crate::thumbnail_protocol`]) serves the files to the UI.
//!
//! # Cache key
//!
//! The file name contains the photo id and a 64-bit FNV-1a hash
//! ([`cache_key`]) of:
//!
//! - the stored develop settings (JSON text and schema version),
//! - the size and the modification time of the original file,
//! - [`THUMB_FORMAT_VERSION`].
//!
//! A change of one of these values gives a new key, so an old thumbnail is
//! never served. When a new thumbnail is written, the files of older keys
//! of the same photo are removed. FNV-1a is used because its output is the
//! same for all Rust versions and all runs (the standard hasher is not
//! stable).
//!
//! # Generation
//!
//! - Photos with default develop settings: the embedded JPEG preview of
//!   the file, if it has one with a long edge of at least
//!   [`THUMB_LONG_EDGE`] (see [`expose_raw::embedded_preview_min`]).
//! - All other photos (and the fallback): a full decode, an area
//!   downsample to [`WORK_LONG_EDGE`], the full develop pipeline on the
//!   CPU (geometry, tone, sharpening), and a Lanczos-3 resize to the
//!   thumbnail size. A crop gives the thumbnail the aspect ratio of the
//!   crop. With a crop, the downsample keeps more pixels (up to
//!   [`MAX_WORK_LONG_EDGE`]), so that the cropped image still reaches the
//!   thumbnail size. The thumbnail is smaller only when the cropped
//!   original is smaller than [`THUMB_LONG_EDGE`] (no enlargement).
//!
//! # Threads
//!
//! [`ThumbnailService`] has a fixed pool of worker threads (half of the
//! CPU cores, at least one). Requests for the same key share one
//! generation. The queue is last-in, first-out: the photos that the user
//! scrolled to last are made first.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use expose_catalog::{DevelopSettings as StoredSettings, PhotoId};
use expose_core::buffer::{AlphaMode, ResizeMode, WorkingImage, fit_size, resize};
use expose_core::develop::DevelopSettings;
use expose_core::error::CoreError;
use expose_develop::{DevelopError, Pipeline, SourceInfo, downsample_area};
use expose_raw::{
    DecodeError, DecodeOptions, EmbeddedPreview, EncodeError, EncodeOptions, decode, decode_with,
    embedded_preview_min, encode_path,
};

use crate::error::CommandError;

/// The long edge of a thumbnail, in pixels.
pub const THUMB_LONG_EDGE: u32 = 512;

/// The JPEG quality of a thumbnail.
pub const THUMB_JPEG_QUALITY: u8 = 85;

/// The version of the thumbnail format. Increase it when the generation
/// changes (size, quality, color processing), so that old files are made
/// again.
///
/// Version 2: thumbnails apply geometry (crop) and sharpening.
/// Version 3: a crop enlarges the develop input, so cropped thumbnails
/// keep the full thumbnail size.
pub const THUMB_FORMAT_VERSION: u32 = 3;

/// The name of the thumbnail folder in the application cache folder.
pub const THUMB_DIR_NAME: &str = "thumbnails";

/// The long edge of the intermediate image of a full render. An area
/// downsample to this size is fast; the Lanczos-3 filter then makes the
/// final size from it.
pub const WORK_LONG_EDGE: u32 = 2 * THUMB_LONG_EDGE;

/// The largest long edge of the develop input of a cropped full render
/// (see [`work_long_edge`]). A 4096 px input uses about 180 MB (`f32`
/// RGBA) for a 3:2 photo, and lets a crop to 25 % still reach
/// [`WORK_LONG_EDGE`].
pub const MAX_WORK_LONG_EDGE: u32 = 4 * WORK_LONG_EDGE;

/// An error of a thumbnail request.
#[derive(Debug, thiserror::Error)]
pub enum ThumbnailError {
    /// The photo is not in the catalog.
    #[error("photo {0} does not exist")]
    PhotoNotFound(i64),
    /// The original file of the photo does not exist.
    #[error("the file {} does not exist", path.display())]
    FileNotFound {
        /// The path of the original file.
        path: PathBuf,
    },
    /// A file system operation failed.
    #[error("{op} {}: {source}", path.display())]
    Io {
        /// What the operation did.
        op: &'static str,
        /// The file or folder.
        path: PathBuf,
        /// The cause.
        #[source]
        source: io::Error,
    },
    /// The image file cannot be decoded.
    #[error("cannot decode the image: {0}")]
    Decode(#[from] DecodeError),
    /// The develop pipeline failed.
    #[error("cannot develop the image: {0}")]
    Develop(#[from] DevelopError),
    /// A resize failed.
    #[error("cannot resize the image: {0}")]
    Core(#[from] CoreError),
    /// The JPEG encode or the write of the cache file failed.
    #[error("cannot write the thumbnail: {0}")]
    Encode(#[from] EncodeError),
    /// The catalog read failed.
    #[error(transparent)]
    Catalog(#[from] CommandError),
    /// The generation panicked, or the service stopped before the request
    /// was done. This is a bug or an application shutdown.
    #[error("the thumbnail worker failed: {0}")]
    Worker(String),
}

impl ThumbnailError {
    fn io(op: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            op,
            path: path.to_path_buf(),
            source,
        }
    }
}

/// The result of a thumbnail request: the JPEG bytes. The error is shared
/// because one generation can answer more than one request.
pub type ThumbnailResult = Result<Arc<[u8]>, Arc<ThumbnailError>>;

/// A function that receives the result of a request.
pub type ThumbnailCallback = Box<dyn FnOnce(ThumbnailResult) + Send + 'static>;

/// All data that the generation of one thumbnail needs.
#[derive(Debug, Clone, PartialEq)]
pub struct ThumbnailInput {
    /// The photo id.
    pub photo_id: PhotoId,
    /// The original file.
    pub path: PathBuf,
    /// The parsed develop settings.
    pub settings: DevelopSettings,
    /// The cache key (see [`cache_key`]).
    pub key: u64,
}

impl ThumbnailInput {
    /// Makes the input for a photo from its catalog data. Reads the size
    /// and the modification time of the file.
    ///
    /// Errors: [`ThumbnailError::FileNotFound`] if the file does not exist,
    /// [`ThumbnailError::Io`] for other file system errors.
    pub fn from_catalog(
        photo_id: PhotoId,
        path: PathBuf,
        stored: Option<&StoredSettings>,
    ) -> Result<Self, ThumbnailError> {
        let meta = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(ThumbnailError::FileNotFound { path });
            }
            Err(e) => return Err(ThumbnailError::io("cannot read the metadata of", &path, e)),
        };
        let modified = meta
            .modified()
            .map_err(|e| ThumbnailError::io("cannot read the modification time of", &path, e))?;
        let key = cache_key(
            stored.map(|s| (s.schema_version, &s.settings)),
            meta.len(),
            modified,
        );
        Ok(Self {
            photo_id,
            settings: parse_settings(photo_id, stored),
            path,
            key,
        })
    }
}

/// Parses the stored develop settings of a photo.
///
/// No stored settings give the default settings. Settings that do not
/// parse or do not validate also give the default settings; this is
/// logged as a warning, because the thumbnail then does not show the
/// edits.
pub fn parse_settings(photo_id: PhotoId, stored: Option<&StoredSettings>) -> DevelopSettings {
    let Some(stored) = stored else {
        return DevelopSettings::default();
    };
    let parsed = serde_json::from_value::<DevelopSettings>(stored.settings.clone())
        .map_err(|e| e.to_string())
        .and_then(|mut s| s.validate().map(|()| s).map_err(|e| e.to_string()));
    match parsed {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!(
                photo = photo_id.0,
                schema_version = stored.schema_version,
                %error,
                "the stored develop settings are not valid; the thumbnail uses the default settings"
            );
            DevelopSettings::default()
        }
    }
}

/// A 64-bit FNV-1a hash. Its output does not depend on the Rust version
/// or on the run.
#[derive(Debug, Clone, Copy)]
struct Fnv1a(u64);

impl Fnv1a {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    const fn new() -> Self {
        Self(Self::OFFSET)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    /// Writes the length first, so that two fields cannot run together.
    fn write_field(&mut self, bytes: &[u8]) {
        self.write(&(bytes.len() as u64).to_le_bytes());
        self.write(bytes);
    }
}

/// Returns the cache key of a thumbnail.
///
/// `settings` is the stored settings (schema version and JSON value), or
/// `None` if the photo has no stored settings. `file_size` and `modified`
/// are from the original file.
pub fn cache_key(
    settings: Option<(u32, &serde_json::Value)>,
    file_size: u64,
    modified: SystemTime,
) -> u64 {
    let mut h = Fnv1a::new();
    h.write_field(&THUMB_FORMAT_VERSION.to_le_bytes());
    match settings {
        Some((version, value)) => {
            h.write_field(&[1]);
            h.write_field(&version.to_le_bytes());
            h.write_field(value.to_string().as_bytes());
        }
        None => h.write_field(&[0]),
    }
    h.write_field(&file_size.to_le_bytes());
    // Times before 1970 are possible on some file systems; a sign byte
    // keeps them apart from the times after 1970.
    match modified.duration_since(UNIX_EPOCH) {
        Ok(d) => {
            h.write_field(&[1]);
            h.write_field(&d.as_nanos().to_le_bytes());
        }
        Err(e) => {
            h.write_field(&[0]);
            h.write_field(&e.duration().as_nanos().to_le_bytes());
        }
    }
    h.0
}

/// Renders the thumbnail image of a file: display-oriented, long edge at
/// most [`THUMB_LONG_EDGE`], straight alpha.
///
/// See the module documentation for the two paths.
pub fn render_thumbnail(
    path: &Path,
    settings: &DevelopSettings,
) -> Result<WorkingImage, ThumbnailError> {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(ThumbnailError::FileNotFound {
                path: path.to_path_buf(),
            });
        }
        Err(e) => return Err(ThumbnailError::io("cannot read", path, e)),
    };
    if *settings == DevelopSettings::default() {
        match embedded_preview_min(&data, THUMB_LONG_EDGE) {
            Ok(Some(preview)) if preview.long_edge() >= THUMB_LONG_EDGE => {
                match decode_preview(&preview) {
                    Ok(image) => return fit_thumbnail(&image),
                    Err(error) => tracing::warn!(
                        path = %path.display(),
                        %error,
                        "the embedded preview does not decode; using a full decode"
                    ),
                }
            }
            Ok(_) => {}
            Err(error) => tracing::debug!(
                path = %path.display(),
                %error,
                "no embedded preview; using a full decode"
            ),
        }
    }
    let decoded = decode(&data)?;
    // Free the file data before the develop work.
    drop(data);
    // The full decode gives the full-resolution original in display
    // orientation (the embedded preview is used only for default settings,
    // above). Thus its size is the original size for the pixel scale.
    let original = (decoded.image.width(), decoded.image.height());
    // The decoders apply the as-shot white balance of RAW files, so the
    // decoded image is a rendered source.
    let pipeline = Pipeline::new(settings, &SourceInfo::RENDERED)?;
    let work = work_long_edge(&pipeline, original)?;
    let small = downsample_area(&decoded.image, work)?;
    drop(decoded);
    let developed = if pipeline.is_identity() {
        small
    } else {
        let scale = pixel_scale((small.width(), small.height()), original);
        // Geometry (crop) changes the output size and aspect ratio;
        // `fit_thumbnail` accepts any size.
        pipeline.develop_cpu_owned(small, scale)?
    };
    fit_thumbnail(&developed)
}

/// Returns the long edge of the develop input of a full render.
///
/// Without geometry it is [`WORK_LONG_EDGE`] (the fast path). With
/// geometry, a crop makes the develop output smaller than the input. Thus
/// the input long edge is [`WORK_LONG_EDGE`] multiplied by the crop factor
/// (original long edge / output long edge), so that the cropped image
/// still has a long edge of about [`WORK_LONG_EDGE`]. The value is limited
/// to [`MAX_WORK_LONG_EDGE`]. The downsample does not enlarge, so a value
/// above the original size gives the original size.
///
/// # Errors
///
/// [`ThumbnailError::Develop`] when the original size is zero.
fn work_long_edge(pipeline: &Pipeline, original: (u32, u32)) -> Result<u32, ThumbnailError> {
    if !pipeline.has_geometry() {
        return Ok(WORK_LONG_EDGE);
    }
    let output = pipeline.output_size(original.0, original.1)?;
    let original_long = u64::from(original.0.max(original.1));
    let output_long = u64::from(output.0.max(output.1)).max(1);
    let work = (u64::from(WORK_LONG_EDGE) * original_long)
        .div_ceil(output_long)
        .clamp(u64::from(WORK_LONG_EDGE), u64::from(MAX_WORK_LONG_EDGE));
    // The clamp limits the value to a `u32` constant; the fallback cannot
    // apply.
    Ok(u32::try_from(work).unwrap_or(MAX_WORK_LONG_EDGE))
}

/// Returns the output pixels for each original pixel: `long edge of
/// small / long edge of original`. The sharpening radius is in original
/// pixels, so the develop pipeline needs this value.
///
/// A zero-size original gives a value that is not finite; the develop
/// pipeline rejects it with an error (no silent fallback).
fn pixel_scale(small: (u32, u32), original: (u32, u32)) -> f32 {
    let ratio = f64::from(small.0.max(small.1)) / f64::from(original.0.max(original.1));
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the ratio of two u32 long edges is in the f32 range"
    )]
    let ratio = ratio as f32;
    ratio
}

/// Decodes an embedded preview and applies the orientation of the file.
fn decode_preview(preview: &EmbeddedPreview) -> Result<WorkingImage, DecodeError> {
    // The preview pixels have the stored orientation of the main image.
    // Do not apply an EXIF orientation inside the preview as well.
    let options = DecodeOptions {
        apply_orientation: false,
        ..DecodeOptions::default()
    };
    let decoded = decode_with(&preview.jpeg, &options)?;
    // `None`: the file gives no orientation, so the stored pixels are shown.
    // `apply` returns a `CoreError`; `DecodeError` wraps it.
    Ok(preview
        .orientation
        .unwrap_or_default()
        .apply(decoded.image)?)
}

/// Scales an image to the thumbnail size. Smaller images are not
/// enlarged.
fn fit_thumbnail(image: &WorkingImage) -> Result<WorkingImage, ThumbnailError> {
    let image = if image.width().max(image.height()) > WORK_LONG_EDGE {
        downsample_area(image, WORK_LONG_EDGE)?
    } else {
        image.clone()
    };
    let (w, h) = fit_size(
        image.width(),
        image.height(),
        ResizeMode::LongEdge(THUMB_LONG_EDGE),
        false,
    )?;
    Ok(resize(&image, w, h, AlphaMode::Straight)?)
}

/// The queue of the worker pool.
struct Queue {
    /// Inputs to generate. Workers take the last one first.
    jobs: VecDeque<ThumbnailInput>,
    /// The callbacks that wait for each queued or running key.
    waiters: HashMap<(PhotoId, u64), Vec<ThumbnailCallback>>,
    /// `true` after the service is dropped.
    stopped: bool,
}

/// The state that the service and its workers share.
struct Shared {
    dir: PathBuf,
    queue: Mutex<Queue>,
    ready: Condvar,
    generated: AtomicU64,
}

impl Shared {
    /// Locks the queue. The queue data stays valid if a holder panicked
    /// (each change is one step), so a poisoned lock is recovered.
    fn lock_queue(&self) -> MutexGuard<'_, Queue> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The thumbnail cache with its worker pool.
pub struct ThumbnailService {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for ThumbnailService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThumbnailService")
            .field("dir", &self.shared.dir)
            .finish_non_exhaustive()
    }
}

/// Returns the default number of worker threads: half of the CPU cores,
/// at least one.
pub fn default_workers() -> usize {
    match thread::available_parallelism() {
        Ok(n) => (n.get() / 2).max(1),
        Err(error) => {
            tracing::warn!(%error, "the number of CPU cores is not known; using 1 thumbnail worker");
            1
        }
    }
}

impl ThumbnailService {
    /// Makes the service. Makes the cache folder `dir` if it does not
    /// exist, and starts `workers` threads (at least one).
    ///
    /// Errors: the folder cannot be made, or no thread can start.
    pub fn new(dir: PathBuf, workers: usize) -> Result<Self, ThumbnailError> {
        std::fs::create_dir_all(&dir)
            .map_err(|e| ThumbnailError::io("cannot make the folder", &dir, e))?;
        let shared = Arc::new(Shared {
            dir,
            queue: Mutex::new(Queue {
                jobs: VecDeque::new(),
                waiters: HashMap::new(),
                stopped: false,
            }),
            ready: Condvar::new(),
            generated: AtomicU64::new(0),
        });
        let mut started = 0;
        for i in 0..workers.max(1) {
            let worker = Arc::clone(&shared);
            let spawned = thread::Builder::new()
                .name(format!("thumbnail-{i}"))
                .spawn(move || worker_loop(&worker));
            match spawned {
                Ok(_) => started += 1,
                Err(error) => tracing::error!(%error, "cannot start a thumbnail worker"),
            }
        }
        if started == 0 {
            return Err(ThumbnailError::Worker(
                "no thumbnail worker thread could start".into(),
            ));
        }
        Ok(Self { shared })
    }

    /// Returns the cache folder.
    pub fn dir(&self) -> &Path {
        &self.shared.dir
    }

    /// Returns the number of thumbnails that this service generated (cache
    /// misses).
    pub fn generated_count(&self) -> u64 {
        self.shared.generated.load(Ordering::Relaxed)
    }

    /// Returns the cache file path of a thumbnail. Files are in 256
    /// sub-folders (by photo id), so that the removal of old files reads
    /// small folders.
    pub fn cache_path(&self, photo_id: PhotoId, key: u64) -> PathBuf {
        cache_path(&self.shared.dir, photo_id, key)
    }

    /// Requests a thumbnail. `callback` receives the result once, on this
    /// thread (cache hit) or on a worker thread.
    ///
    /// This function reads the cache file, so do not call it on the UI
    /// thread.
    pub fn request(&self, input: ThumbnailInput, callback: ThumbnailCallback) {
        match read_cached(&self.shared.dir, &input) {
            Ok(Some(bytes)) => return callback(Ok(bytes)),
            Ok(None) => {}
            Err(error) => return callback(Err(Arc::new(error))),
        }
        let mut queue = self.shared.lock_queue();
        if queue.stopped {
            drop(queue);
            return callback(Err(Arc::new(ThumbnailError::Worker(
                "the thumbnail service stopped".into(),
            ))));
        }
        let key = (input.photo_id, input.key);
        if let Some(waiters) = queue.waiters.get_mut(&key) {
            // The same thumbnail is queued or in work: share the result.
            waiters.push(callback);
            return;
        }
        queue.waiters.insert(key, vec![callback]);
        queue.jobs.push_back(input);
        drop(queue);
        self.shared.ready.notify_one();
    }

    /// Returns the thumbnail, from the cache or made now on this thread.
    /// This is the work of one job of the pool.
    pub fn get_or_create(&self, input: &ThumbnailInput) -> Result<Arc<[u8]>, ThumbnailError> {
        get_or_create(&self.shared, input)
    }
}

impl Drop for ThumbnailService {
    fn drop(&mut self) {
        let waiters: Vec<ThumbnailCallback> = {
            let mut queue = self.shared.lock_queue();
            queue.stopped = true;
            queue.jobs.clear();
            queue.waiters.drain().flat_map(|(_, v)| v).collect()
        };
        self.shared.ready.notify_all();
        for callback in waiters {
            callback(Err(Arc::new(ThumbnailError::Worker(
                "the thumbnail service stopped".into(),
            ))));
        }
    }
}

fn cache_path(dir: &Path, photo_id: PhotoId, key: u64) -> PathBuf {
    dir.join(format!("{:02x}", photo_id.0.rem_euclid(256)))
        .join(format!("{}-{key:016x}.jpg", photo_id.0))
}

/// Reads the cache file of `input`. `Ok(None)` if it does not exist.
fn read_cached(dir: &Path, input: &ThumbnailInput) -> Result<Option<Arc<[u8]>>, ThumbnailError> {
    let path = cache_path(dir, input.photo_id, input.key);
    match std::fs::read(&path) {
        Ok(bytes) => Ok(Some(bytes.into())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ThumbnailError::io("cannot read", &path, e)),
    }
}

fn get_or_create(shared: &Shared, input: &ThumbnailInput) -> Result<Arc<[u8]>, ThumbnailError> {
    // An earlier job can have made the file after the request looked.
    if let Some(bytes) = read_cached(&shared.dir, input)? {
        return Ok(bytes);
    }
    let image = render_thumbnail(&input.path, &input.settings)?;
    let path = cache_path(&shared.dir, input.photo_id, input.key);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ThumbnailError::io("cannot make the folder", parent, e))?;
    }
    // `encode_path` writes a temporary file and renames it, so a reader
    // never sees a partial file.
    encode_path(&image, &EncodeOptions::jpeg(THUMB_JPEG_QUALITY), &path)?;
    shared.generated.fetch_add(1, Ordering::Relaxed);
    remove_stale(&path, input.photo_id);
    let bytes = std::fs::read(&path).map_err(|e| ThumbnailError::io("cannot read", &path, e))?;
    Ok(bytes.into())
}

/// Removes the cache files of other keys of the same photo. Errors are
/// logged: an old file only uses disk space.
fn remove_stale(current: &Path, photo_id: PhotoId) {
    let Some(parent) = current.parent() else {
        return;
    };
    let prefix = format!("{}-", photo_id.0);
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(dir = %parent.display(), %error, "cannot list the thumbnail folder");
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // Only names of the form `<id>-<hex key>.jpg` (this module writes
        // them in lower case).
        let is_old_key = name
            .strip_prefix(&prefix)
            .and_then(|rest| rest.strip_suffix(".jpg"))
            .is_some_and(|key| !key.is_empty() && key.chars().all(|c| c.is_ascii_hexdigit()));
        if path != current && is_old_key {
            if let Err(error) = std::fs::remove_file(&path) {
                tracing::warn!(path = %path.display(), %error, "cannot remove an old thumbnail");
            }
        }
    }
}

fn worker_loop(shared: &Shared) {
    loop {
        let input = {
            let mut queue = shared.lock_queue();
            loop {
                if queue.stopped {
                    return;
                }
                if let Some(input) = queue.jobs.pop_back() {
                    break input;
                }
                queue = shared
                    .ready
                    .wait(queue)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        // A decoder bug must not stop the worker or leave the requests
        // without an answer.
        let result = catch_unwind(AssertUnwindSafe(|| get_or_create(shared, &input)))
            .unwrap_or_else(|_| {
                Err(ThumbnailError::Worker(format!(
                    "the generation of the thumbnail of photo {} panicked",
                    input.photo_id.0
                )))
            })
            .map_err(Arc::new);
        if let Err(error) = &result {
            tracing::warn!(photo = input.photo_id.0, path = %input.path.display(), %error, "thumbnail failed");
        }
        let callbacks = shared
            .lock_queue()
            .waiters
            .remove(&(input.photo_id, input.key))
            .unwrap_or_default();
        for callback in callbacks {
            callback(result.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    fn stored(value: serde_json::Value) -> StoredSettings {
        StoredSettings {
            photo_id: PhotoId(1),
            schema_version: 1,
            settings: value,
            updated_ms: 0,
        }
    }

    /// Writes a gradient image of `w` x `h` pixels as PNG.
    fn write_png(path: &Path, w: u32, h: u32) {
        let mut img = WorkingImage::new(w, h).unwrap();
        for y in 0..h {
            for x in 0..w {
                #[allow(clippy::cast_precision_loss, reason = "small test values")]
                let v = (x + y) as f32 / (w + h) as f32;
                *img.pixel_mut(x, y).unwrap() = [v, 0.5, 1.0 - v, 1.0];
            }
        }
        let options = EncodeOptions::png(expose_raw::BitDepth::Eight, false);
        encode_path(&img, &options, path).unwrap();
    }

    fn jpeg_size(bytes: &[u8]) -> (u32, u32) {
        let d = expose_raw::decode(bytes).unwrap();
        assert_eq!(d.info.format, expose_raw::ImageFormat::Jpeg);
        (d.image.width(), d.image.height())
    }

    #[test]
    fn cache_key_is_stable_and_changes_with_inputs() {
        let time = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
        let one = serde_json::json!({ "tone": { "exposure": 1.0 } });
        let two = serde_json::json!({ "tone": { "exposure": 1.5 } });
        let key = cache_key(Some((1, &one)), 100, time);
        assert_eq!(key, cache_key(Some((1, &one)), 100, time));
        assert_ne!(key, cache_key(Some((1, &two)), 100, time));
        assert_ne!(key, cache_key(Some((2, &one)), 100, time));
        assert_ne!(key, cache_key(None, 100, time));
        assert_ne!(key, cache_key(Some((1, &one)), 101, time));
        let later = time + Duration::from_millis(1);
        assert_ne!(key, cache_key(Some((1, &one)), 100, later));
        // The reference value of FNV-1a 64 for "a": the hash must not
        // change between builds.
        let mut hash = Fnv1a::new();
        hash.write(b"a");
        assert_eq!(hash.0, 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn invalid_settings_give_defaults() {
        let bad = stored(serde_json::json!({ "version": 99 }));
        assert_eq!(
            parse_settings(PhotoId(1), Some(&bad)),
            DevelopSettings::default()
        );
        let good = stored(serde_json::json!({ "tone": { "exposure": 1.0 } }));
        assert_ne!(
            parse_settings(PhotoId(1), Some(&good)),
            DevelopSettings::default()
        );
        assert_eq!(parse_settings(PhotoId(1), None), DevelopSettings::default());
    }

    #[test]
    fn generates_and_reuses_thumbnail() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("photo.png");
        write_png(&src, 900, 300);
        let service = ThumbnailService::new(dir.path().join("cache"), 1).unwrap();

        let input = ThumbnailInput::from_catalog(PhotoId(7), src.clone(), None).unwrap();
        let bytes = service.get_or_create(&input).unwrap();
        assert_eq!(jpeg_size(&bytes), (512, 171));
        assert_eq!(service.generated_count(), 1);
        assert!(service.cache_path(PhotoId(7), input.key).is_file());

        // Cache hit: no new generation.
        let again = service.get_or_create(&input).unwrap();
        assert_eq!(again, bytes);
        assert_eq!(service.generated_count(), 1);

        // New settings: a new key; the old file is removed.
        let edited = stored(serde_json::json!({ "tone": { "exposure": 1.0 } }));
        let input2 = ThumbnailInput::from_catalog(PhotoId(7), src, Some(&edited)).unwrap();
        assert_ne!(input2.key, input.key);
        let bright = service.get_or_create(&input2).unwrap();
        assert_ne!(bright, bytes);
        assert_eq!(service.generated_count(), 2);
        assert!(!service.cache_path(PhotoId(7), input.key).exists());
        assert!(service.cache_path(PhotoId(7), input2.key).is_file());
    }

    #[test]
    fn small_images_are_not_enlarged() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("small.png");
        write_png(&src, 40, 30);
        let image = render_thumbnail(&src, &DevelopSettings::default()).unwrap();
        assert_eq!((image.width(), image.height()), (40, 30));
    }

    #[test]
    fn crop_changes_the_thumbnail_aspect_ratio() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("wide.png");
        write_png(&src, 1200, 600);
        let mut settings = DevelopSettings::default();
        // The left half: a square crop.
        settings.crop.rect.width = 0.5;
        settings.sharpening.amount = 50.0;
        let image = render_thumbnail(&src, &settings).unwrap();
        assert_eq!(
            (image.width(), image.height()),
            (THUMB_LONG_EDGE, THUMB_LONG_EDGE)
        );
        // No geometry: the aspect ratio of the source.
        let image = render_thumbnail(&src, &DevelopSettings::default()).unwrap();
        assert_eq!((image.width(), image.height()), (THUMB_LONG_EDGE, 256));
    }

    #[test]
    fn strong_crop_keeps_the_thumbnail_size() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("large.png");
        write_png(&src, 2400, 1200);
        let mut settings = DevelopSettings::default();
        // A centered crop to 25 % of each side: 600 x 300 original pixels.
        settings.crop.rect = expose_core::develop::NormalizedRect {
            left: 0.375,
            top: 0.375,
            width: 0.25,
            height: 0.25,
        };
        let image = render_thumbnail(&src, &settings).unwrap();
        assert_eq!((image.width(), image.height()), (THUMB_LONG_EDGE, 256));
    }

    #[test]
    fn work_long_edge_follows_the_crop() {
        let plain = Pipeline::new(&DevelopSettings::default(), &SourceInfo::RENDERED).unwrap();
        assert_eq!(
            work_long_edge(&plain, (6000, 4000)).unwrap(),
            WORK_LONG_EDGE
        );
        let mut settings = DevelopSettings::default();
        // A crop to the left half keeps the full height (4000 px), so the
        // factor is 6000 / 4000.
        settings.crop.rect.width = 0.5;
        let left = Pipeline::new(&settings, &SourceInfo::RENDERED).unwrap();
        assert_eq!(work_long_edge(&left, (6000, 4000)).unwrap(), 1536);
        settings.crop.rect.height = 0.5;
        let half = Pipeline::new(&settings, &SourceInfo::RENDERED).unwrap();
        assert_eq!(work_long_edge(&half, (6000, 4000)).unwrap(), 2048);
        settings.crop.rect.width = 0.1;
        settings.crop.rect.height = 0.1;
        let tiny = Pipeline::new(&settings, &SourceInfo::RENDERED).unwrap();
        assert_eq!(
            work_long_edge(&tiny, (6000, 4000)).unwrap(),
            MAX_WORK_LONG_EDGE
        );
    }

    #[test]
    fn pixel_scale_is_the_long_edge_ratio() {
        assert!((pixel_scale((1024, 683), (6000, 4000)) - 1024.0 / 6000.0).abs() < 1e-6);
        // Portrait: the long edge is the height.
        assert!((pixel_scale((683, 1024), (4000, 6000)) - 1024.0 / 6000.0).abs() < 1e-6);
        assert!((pixel_scale((40, 30), (40, 30)) - 1.0).abs() < f32::EPSILON);
        // A zero-size original gives a value that the pipeline rejects.
        assert!(!pixel_scale((40, 30), (0, 0)).is_finite());
    }

    #[test]
    fn missing_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.jpg");
        let error = ThumbnailInput::from_catalog(PhotoId(1), missing.clone(), None).unwrap_err();
        assert!(matches!(error, ThumbnailError::FileNotFound { .. }));
        let error = render_thumbnail(&missing, &DevelopSettings::default()).unwrap_err();
        assert!(matches!(error, ThumbnailError::FileNotFound { .. }));
    }

    #[test]
    fn requests_share_one_generation() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("photo.png");
        write_png(&src, 640, 480);
        let service = ThumbnailService::new(dir.path().join("cache"), 2).unwrap();
        let input = ThumbnailInput::from_catalog(PhotoId(3), src, None).unwrap();
        let (tx, rx) = mpsc::channel();
        for _ in 0..4 {
            let tx = tx.clone();
            service.request(
                input.clone(),
                Box::new(move |r| {
                    let _ = tx.send(r.map(|b| b.len()).map_err(|e| e.to_string()));
                }),
            );
        }
        for _ in 0..4 {
            let r = rx.recv_timeout(Duration::from_secs(30)).unwrap();
            assert!(r.unwrap() > 0);
        }
        assert_eq!(service.generated_count(), 1);
    }

    #[test]
    fn decode_error_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("bad.jpg");
        std::fs::write(&src, b"\xFF\xD8\xFFnot really a jpeg").unwrap();
        let service = ThumbnailService::new(dir.path().join("cache"), 1).unwrap();
        let input = ThumbnailInput::from_catalog(PhotoId(4), src, None).unwrap();
        let (tx, rx) = mpsc::channel();
        service.request(
            input,
            Box::new(move |r| {
                let _ = tx.send(r.is_err());
            }),
        );
        assert!(rx.recv_timeout(Duration::from_secs(30)).unwrap());
    }

    #[test]
    fn embedded_preview_is_used_for_default_settings() {
        // A TIFF-structured file whose only JPEG is a 600x400 preview. The
        // full decode of this file fails, so a thumbnail proves that the
        // preview path ran.
        let dir = tempfile::tempdir().unwrap();
        let preview_path = dir.path().join("preview.jpg");
        let img = WorkingImage::new_filled(600, 400, [0.2, 0.4, 0.6, 1.0]).unwrap();
        encode_path(&img, &EncodeOptions::jpeg(90), &preview_path).unwrap();
        let jpeg = std::fs::read(&preview_path).unwrap();
        let mut file = b"II*\0\x08\0\0\0".to_vec();
        let entry = |tag: u16, value: u32| {
            let mut e = tag.to_le_bytes().to_vec();
            e.extend_from_slice(&4_u16.to_le_bytes());
            e.extend_from_slice(&1_u32.to_le_bytes());
            e.extend_from_slice(&value.to_le_bytes());
            e
        };
        let jpeg_pos = 8 + 2 + 2 * 12 + 4;
        file.extend_from_slice(&2_u16.to_le_bytes());
        file.extend(entry(513, jpeg_pos));
        file.extend(entry(514, u32::try_from(jpeg.len()).unwrap()));
        file.extend_from_slice(&0_u32.to_le_bytes());
        file.extend_from_slice(&jpeg);
        let src = dir.path().join("camera.tif");
        std::fs::write(&src, &file).unwrap();

        let image = render_thumbnail(&src, &DevelopSettings::default()).unwrap();
        assert_eq!((image.width(), image.height()), (512, 341));
        // With edits, the full decode runs and fails for this file.
        let edited = DevelopSettings {
            tone: expose_core::develop::BasicTone {
                exposure: 1.0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(render_thumbnail(&src, &edited).is_err());
    }
}
