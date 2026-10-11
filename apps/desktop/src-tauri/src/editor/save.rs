//! Save of an editor document (ADR 0015): the layered `.exdoc` file and a
//! flattened 16-bit TIFF next to the source photo, the import of the TIFF
//! into the library, and the catalog link between them.
//!
//! # Steps
//!
//! 1. The render thread composites the document on the GPU and gives a
//!    [`SaveSnapshot`]: a copy of the document, the premultiplied
//!    composite, the revision, and the [`DocumentOrigin`].
//! 2. [`plan_targets`] (catalog lock) selects the two paths and checks
//!    that the save overwrites only files of an earlier save of this
//!    document. Catalog originals are never written (ADR 0010).
//! 3. [`write_files`] (no lock) writes the two files. Each write is atomic
//!    ([`write_atomic`]).
//! 4. [`record_in_catalog`] (catalog lock) imports the TIFF and records
//!    the link at the first save, or updates the image info at a later
//!    save.
//!
//! [`save_snapshot`] runs steps 2 to 4.
//!
//! # TIFF
//!
//! 16 bits per sample, deflate, Rec. 2020 primaries with the BT.2020
//! curve and its ICC profile. The working space is linear Rec. 2020, so
//! this space keeps the full gamut of the document, and color-managed
//! applications (and the Expose decoder) read the profile. The file has an
//! alpha channel (straight) only if a pixel of the composite is not opaque.

use std::collections::HashSet;
use std::fs::{self, File};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use expose_catalog::{AddOutcome, CatalogStore, EditorOutput, ImageInfo, NewPhoto, PhotoId};
use expose_core::buffer::WorkingImage;
use expose_core::color::ColorSpaceId;
use expose_editor::{Document, FILE_EXTENSION, to_straight_alpha, write_document};
use expose_raw::{BitDepth, EncodeOptions, TiffCompression};

use super::EditorEngineError;
use crate::error::{CommandError, ErrorKind};
use crate::state::CatalogState;

/// The text that the save adds to the stem of the source file name.
pub const EDIT_SUFFIX: &str = "-edit";

/// The file extension of the flattened image.
pub const IMAGE_EXTENSION: &str = "tif";

/// The highest number that the name selection tries (`-edit-9999`).
const MAX_NAME_NUMBER: u32 = 9999;

/// The number of tries to make a temporary file with a new random name.
const TEMP_TRIES: u32 = 16;

/// The files and the library photo of a save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedFiles {
    /// The library photo of the flattened image.
    pub photo_id: PhotoId,
    /// The absolute path of the layered document file.
    pub document_path: PathBuf,
    /// The absolute path of the flattened image file.
    pub image_path: PathBuf,
}

/// Where an open document came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentOrigin {
    /// The library photo that the document was first opened from, or
    /// `None` if it is not known.
    pub source_photo_id: Option<PhotoId>,
    /// The files of the last save (or of the link that the document was
    /// opened from). The next save writes the same files.
    pub saved: Option<SavedFiles>,
}

/// The data that the render thread gives to a save.
#[derive(Debug)]
pub struct SaveSnapshot {
    /// A copy of the document at `revision`.
    pub document: Document,
    /// The composite of the full canvas, premultiplied alpha, linear
    /// Rec. 2020.
    pub composite: WorkingImage,
    /// The document revision of the copy.
    pub revision: u64,
    /// The origin of the document at the time of the snapshot.
    pub origin: DocumentOrigin,
}

/// The two paths of a save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveTargets {
    /// The layered document file.
    pub document_path: PathBuf,
    /// The flattened image file.
    pub image_path: PathBuf,
    /// The library photo of an earlier save, or `None` for a first save.
    pub photo_id: Option<PhotoId>,
}

/// Facts about the flattened image file after the write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrittenImage {
    /// The image width in pixels.
    pub width: u32,
    /// The image height in pixels.
    pub height: u32,
    /// The file size in bytes.
    pub file_size: u64,
    /// The file modification time (Unix milliseconds).
    pub modified_ms: i64,
}

/// The result of [`save_snapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveOutcome {
    /// The files and the photo of the save.
    pub files: SavedFiles,
    /// `true` for a first save (new files and a new library photo).
    pub created: bool,
}

/// Runs the file and catalog steps of a save (see the module docs).
///
/// At a first save, a failure after the first file write removes the new
/// files again, so that the next save can use the same names. A later save
/// replaces the files one at a time; if the second write fails, the first
/// file is already new.
///
/// # Errors
///
/// - `invalidInput`: the save would overwrite a file that Expose did not
///   make, or the document has no source photo.
/// - `notFound`: the source photo is not in the catalog.
/// - `io`: a file cannot be written (for example, the folder is
///   read-only).
/// - `database` and `internal` for catalog and encoder errors.
pub fn save_snapshot(
    state: &CatalogState,
    snapshot: SaveSnapshot,
) -> Result<SaveOutcome, CommandError> {
    let SaveSnapshot {
        document,
        composite,
        origin,
        ..
    } = snapshot;
    let targets = plan_targets(&*state.lock()?, &origin)?;
    let created = targets.photo_id.is_none();
    let written = write_files(&document, composite, &targets, created)?;
    // Free the document copy before the catalog step.
    drop(document);
    let recorded = state
        .lock()
        .and_then(|mut store| record_in_catalog(&mut *store, &targets, &origin, written));
    match recorded {
        Ok(photo_id) => Ok(SaveOutcome {
            files: SavedFiles {
                photo_id,
                document_path: targets.document_path,
                image_path: targets.image_path,
            },
            created,
        }),
        Err(error) => {
            if created {
                remove_quietly(&targets.image_path);
                remove_quietly(&targets.document_path);
            }
            Err(error)
        }
    }
}

/// Selects the two paths of a save and checks that the save may write
/// them.
///
/// - A document with an earlier save (or opened from a link) uses the
///   same two paths again.
/// - Else the paths are `<stem>-edit.exdoc` and `<stem>-edit.tif` in the
///   folder of the source photo. If a file with either name exists on the
///   disk or in the catalog, the function tries `-edit-2`, `-edit-3`, ...
///
/// # Errors
///
/// See [`save_snapshot`].
pub fn plan_targets<S: CatalogStore + ?Sized>(
    store: &S,
    origin: &DocumentOrigin,
) -> Result<SaveTargets, CommandError> {
    let targets = if let Some(saved) = &origin.saved {
        SaveTargets {
            document_path: saved.document_path.clone(),
            image_path: saved.image_path.clone(),
            photo_id: Some(saved.photo_id),
        }
    } else {
        let source = origin.source_photo_id.ok_or_else(|| {
            CommandError::invalid_input("the document has no source photo, so it has no folder")
        })?;
        let path = store.photo_path(source)?.ok_or_else(|| {
            CommandError::new(
                ErrorKind::NotFound,
                "the source photo is not in the library",
            )
        })?;
        let (folder, stem) = folder_and_stem(&path)?;
        let (document_path, image_path) = pick_names(store, folder, stem)?;
        SaveTargets {
            document_path,
            image_path,
            photo_id: None,
        }
    };
    check_target(store, &targets.document_path, origin.saved.as_ref())?;
    check_target(store, &targets.image_path, origin.saved.as_ref())?;
    Ok(targets)
}

/// Returns the folder and the file stem of a photo path.
fn folder_and_stem(path: &Path) -> Result<(&Path, &str), CommandError> {
    let folder = path.parent();
    let stem = path.file_stem().and_then(|s| s.to_str());
    match (folder, stem) {
        (Some(folder), Some(stem)) if !stem.is_empty() => Ok((folder, stem)),
        _ => Err(CommandError::internal(format!(
            "the photo path {} has no folder or file name",
            path.display()
        ))),
    }
}

/// Returns the first free pair of names (see [`plan_targets`]).
fn pick_names<S: CatalogStore + ?Sized>(
    store: &S,
    folder: &Path,
    stem: &str,
) -> Result<(PathBuf, PathBuf), CommandError> {
    let known: HashSet<String> = match store.find_folder(folder)? {
        Some(f) => store.known_file_names(f.id)?,
        None => HashSet::new(),
    };
    for number in 1..=MAX_NAME_NUMBER {
        let base = if number == 1 {
            format!("{stem}{EDIT_SUFFIX}")
        } else {
            format!("{stem}{EDIT_SUFFIX}-{number}")
        };
        let document_name = format!("{base}.{FILE_EXTENSION}");
        let image_name = format!("{base}.{IMAGE_EXTENSION}");
        let document_path = folder.join(&document_name);
        let image_path = folder.join(&image_name);
        let taken = known.contains(&document_name)
            || known.contains(&image_name)
            || exists_on_disk(&document_path)?
            || exists_on_disk(&image_path)?
            || store.editor_output_for_document(&document_path)?.is_some();
        if !taken {
            return Ok((document_path, image_path));
        }
    }
    Err(CommandError::invalid_input(format!(
        "no free file name for {stem}{EDIT_SUFFIX} in {}",
        folder.display()
    )))
}

/// Refuses a path that the save must not write.
///
/// A path is free if no file is on the disk and no catalog photo has it.
/// Else it must be a file of `link` (the earlier save of this document)
/// that the catalog records as an editor output.
fn check_target<S: CatalogStore + ?Sized>(
    store: &S,
    path: &Path,
    link: Option<&SavedFiles>,
) -> Result<(), CommandError> {
    let refuse = || {
        CommandError::invalid_input(format!(
            "the save would overwrite a file that Expose did not create: {}",
            path.display()
        ))
    };
    let photo = store.find_photo_by_path(path)?;
    let on_disk = exists_on_disk(path)?;
    if photo.is_none() && !on_disk {
        return Ok(());
    }
    let Some(link) = link else {
        return Err(refuse());
    };
    let ours = path == link.document_path || path == link.image_path;
    if !ours || !store.is_editor_output_path(path)? {
        return Err(refuse());
    }
    // The catalog photo at the path (the image file) must be the photo of
    // the link, and the document record must belong to it.
    if photo.is_some_and(|id| id != link.photo_id) {
        return Err(refuse());
    }
    if let Some(record) = store.editor_output_for_document(path)? {
        if record.photo_id != link.photo_id {
            return Err(refuse());
        }
    }
    Ok(())
}

/// Returns `true` if a file (or a link) exists at `path`.
fn exists_on_disk(path: &Path) -> Result<bool, CommandError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error("cannot check", path, &error)),
    }
}

/// The TIFF options of the flattened image (see the module docs).
pub fn image_options(alpha: bool) -> EncodeOptions {
    EncodeOptions::tiff(BitDepth::Sixteen, alpha, TiffCompression::Deflate)
        .with_color_space(ColorSpaceId::Rec2020)
}

/// Writes the layered document and the flattened image to `targets`.
///
/// `composite` is premultiplied. With `created` (a first save), a failure
/// of the image write removes the new document file.
///
/// # Errors
///
/// `io` for file errors, `internal` (or `invalidInput`) for document and
/// encoder errors.
pub fn write_files(
    document: &Document,
    composite: WorkingImage,
    targets: &SaveTargets,
    created: bool,
) -> Result<WrittenImage, CommandError> {
    let image = to_straight_alpha(composite);
    let alpha = image.pixels().iter().any(|p| p[3] < 1.0);
    write_atomic(&targets.document_path, |out| {
        write_document(document, out).map_err(|error| {
            let error = CommandError::from(EditorEngineError::Editor(error));
            CommandError::new(
                error.kind,
                format!(
                    "cannot write {}: {}",
                    targets.document_path.display(),
                    error.message
                ),
            )
        })
    })?;
    let result = write_image(&image, alpha, &targets.image_path);
    if result.is_err() && created {
        remove_quietly(&targets.document_path);
    }
    result
}

/// Encodes and writes the flattened image, and reads the file facts.
fn write_image(
    image: &WorkingImage,
    alpha: bool,
    path: &Path,
) -> Result<WrittenImage, CommandError> {
    let bytes = expose_raw::encode(image, &image_options(alpha)).map_err(|error| {
        CommandError::internal(format!("cannot encode {}: {error}", path.display()))
    })?;
    write_atomic(path, |out| {
        out.write_all(&bytes)
            .map_err(|error| io_error("cannot write", path, &error))
    })?;
    let meta =
        fs::metadata(path).map_err(|error| io_error("cannot read the size of", path, &error))?;
    let modified = meta
        .modified()
        .map_err(|error| io_error("cannot read the time of", path, &error))?;
    // A time before 1970 is not possible for a file that was just written;
    // it gives 0.
    let modified_ms = modified
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    Ok(WrittenImage {
        width: image.width(),
        height: image.height(),
        file_size: meta.len(),
        modified_ms,
    })
}

/// Records the save in the catalog and returns the photo of the image.
///
/// - First save (`targets.photo_id` is `None`): adds the image file to the
///   folder (the folder is added if the catalog does not have it), sets
///   its image info, and records the link.
/// - Later save: replaces the image info of the photo (the size can
///   change).
///
/// The image info has the size of the image and the capture time of the
/// source photo, so that the new photo sorts next to its source.
///
/// # Errors
///
/// `database` and other catalog errors; `internal` if the catalog already
/// had a photo at the image path.
pub fn record_in_catalog<S: CatalogStore + ?Sized>(
    store: &mut S,
    targets: &SaveTargets,
    origin: &DocumentOrigin,
    written: WrittenImage,
) -> Result<PhotoId, CommandError> {
    let source = match origin.source_photo_id {
        Some(id) => store.get_photo(id)?,
        None => None,
    };
    let info = ImageInfo {
        capture_time_ms: source.as_ref().and_then(|p| p.capture_time_ms),
        width: Some(written.width),
        height: Some(written.height),
    };
    if let Some(photo_id) = targets.photo_id {
        store.set_image_info(photo_id, info)?;
        return Ok(photo_id);
    }
    let (folder_path, file_name) = match (
        targets.image_path.parent(),
        targets.image_path.file_name().and_then(|n| n.to_str()),
    ) {
        (Some(folder), Some(name)) => (folder, name.to_owned()),
        _ => {
            return Err(CommandError::internal(format!(
                "the image path {} has no folder or file name",
                targets.image_path.display()
            )));
        }
    };
    let folder = match store.find_folder(folder_path)? {
        Some(folder) => folder,
        None => store.add_folder(folder_path)?,
    };
    let outcomes = store.add_photos(
        folder.id,
        &[NewPhoto {
            file_name,
            file_size: written.file_size,
            file_modified_ms: written.modified_ms,
        }],
    )?;
    let photo_id = match outcomes.first() {
        Some(AddOutcome::Added(id)) => *id,
        Some(AddOutcome::AlreadyKnown(_)) => {
            return Err(CommandError::internal(format!(
                "the library already has a photo at {}",
                targets.image_path.display()
            )));
        }
        None => return Err(CommandError::internal("the import gave no result")),
    };
    store.set_image_info(photo_id, info)?;
    store.set_editor_output(&EditorOutput {
        photo_id,
        document_path: targets.document_path.clone(),
        source_photo_id: source.map(|p| p.id),
    })?;
    Ok(photo_id)
}

/// Reads the layered document of an editor output link.
///
/// The editor must not show the flattened photo in place of the layers,
/// so a missing or damaged file is an error, with no fallback.
///
/// # Errors
///
/// `notFound` if the file is missing, `invalidInput` if it is damaged or
/// not supported, `io` for other read errors.
pub fn read_linked_document(path: &Path) -> Result<Document, CommandError> {
    let file = File::open(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            CommandError::new(
                ErrorKind::NotFound,
                format!(
                    "the layered file of this photo is missing: {}",
                    path.display()
                ),
            )
        } else {
            io_error("cannot open", path, &error)
        }
    })?;
    expose_editor::read_document(io::BufReader::new(file)).map_err(|error| {
        let error = CommandError::from(EditorEngineError::Editor(error));
        CommandError::new(
            error.kind,
            format!(
                "the layered file {} cannot be read: {}",
                path.display(),
                error.message
            ),
        )
    })
}

/// Writes a file atomically: `write` fills a new temporary file
/// `.<name>.tmp-<random>` in the same folder, the function flushes it to
/// the disk and renames it to `path`. An existing file at `path` is
/// replaced. If a step fails, the function removes the temporary file and
/// `path` does not change.
///
/// # Errors
///
/// `io` for file system errors, and the errors of `write`.
pub fn write_atomic<F>(path: &Path, write: F) -> Result<(), CommandError>
where
    F: FnOnce(&mut BufWriter<&File>) -> Result<(), CommandError>,
{
    let (Some(folder), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str()))
    else {
        return Err(CommandError::invalid_input(format!(
            "the path {} has no folder or file name",
            path.display()
        )));
    };
    let (temp_path, file) = create_temp(folder, name)?;
    let result = fill_and_rename(&file, &temp_path, path, write);
    drop(file);
    if result.is_err() {
        remove_quietly(&temp_path);
    } else {
        sync_folder(folder);
    }
    result
}

fn fill_and_rename<F>(
    file: &File,
    temp_path: &Path,
    path: &Path,
    write: F,
) -> Result<(), CommandError>
where
    F: FnOnce(&mut BufWriter<&File>) -> Result<(), CommandError>,
{
    let mut out = BufWriter::new(file);
    write(&mut out)?;
    out.flush()
        .map_err(|error| io_error("cannot write", temp_path, &error))?;
    drop(out);
    file.sync_all()
        .map_err(|error| io_error("cannot flush", temp_path, &error))?;
    fs::rename(temp_path, path).map_err(|error| io_error("cannot replace", path, &error))
}

/// Makes a new temporary file next to the target.
fn create_temp(folder: &Path, name: &str) -> Result<(PathBuf, File), CommandError> {
    let mut last = None;
    for _ in 0..TEMP_TRIES {
        let temp_path = folder.join(format!(".{name}.tmp-{:016x}", random_u64()));
        match File::options()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = Some(error),
            Err(error) => return Err(io_error("cannot write", &temp_path, &error)),
        }
    }
    let error = last.unwrap_or_else(|| io::Error::other("no try was made"));
    Err(io_error("cannot make a temporary file in", folder, &error))
}

/// A random number from the random keys of the standard hash map.
fn random_u64() -> u64 {
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(u64::from(std::process::id()));
    hasher.finish()
}

/// Flushes the folder entry after a rename, so that the new name is on the
/// disk. The file data is already flushed; a failure is logged and does
/// not fail the save.
fn sync_folder(folder: &Path) {
    #[cfg(unix)]
    if let Err(error) = File::open(folder).and_then(|dir| dir.sync_all()) {
        tracing::warn!(folder = %folder.display(), %error, "cannot flush the folder after a save");
    }
    #[cfg(not(unix))]
    let _ = folder;
}

/// Removes a file after a failed step. A failure is logged.
fn remove_quietly(path: &Path) {
    if let Err(error) = fs::remove_file(path) {
        if error.kind() != io::ErrorKind::NotFound {
            tracing::warn!(path = %path.display(), %error, "cannot remove a file after a failed save");
        }
    }
}

/// Makes an `io` command error (or `notFound` for a missing folder).
fn io_error(action: &str, path: &Path, error: &io::Error) -> CommandError {
    let kind = if error.kind() == io::ErrorKind::NotFound {
        ErrorKind::NotFound
    } else {
        ErrorKind::Io
    };
    CommandError::new(kind, format!("{action} {}: {error}", path.display()))
}

#[cfg(test)]
#[path = "save_tests.rs"]
mod tests;
