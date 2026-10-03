//! The storage trait of the catalog.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::import::ImportReport;
use crate::model::{
    AddOutcome, BatchInfoOutcome, ColorLabel, DevelopSettings, Flag, Folder, FolderId, FolderNode,
    ImageInfo, LabelUpdate, NewPhoto, Photo, PhotoId, Rating,
};
use crate::platform::FileSystem;
use crate::query::PhotoQuery;

/// The operations of a catalog storage backend.
///
/// The desktop build uses [`SqliteStore`](crate::SqliteStore). A web build
/// can supply a different backend.
///
/// Each method that writes is atomic: all of its changes are applied, or
/// none of them are applied.
pub trait CatalogStore {
    /// Adds the folder at `path` and returns it. If the folder is already
    /// in the catalog, returns the stored folder and changes nothing.
    ///
    /// The path is normalized with
    /// [`normalize_folder_path`](crate::normalize_folder_path). The
    /// folder tree has no gaps: if the catalog has an ancestor folder,
    /// the method also adds all folders between that ancestor and `path`.
    /// Folders already in the catalog below `path` move under it.
    fn add_folder(&mut self, path: &Path) -> Result<Folder>;

    /// Returns the folder with this id, or `None`.
    fn get_folder(&self, id: FolderId) -> Result<Option<Folder>>;

    /// Returns the folder at this path, or `None`. The path is normalized
    /// first.
    fn find_folder(&self, path: &Path) -> Result<Option<Folder>>;

    /// Returns all folders, sorted by path.
    fn list_folders(&self) -> Result<Vec<Folder>>;

    /// Returns the folder tree, with the direct photo count of each folder.
    fn list_folder_tree(&self) -> Result<Vec<FolderNode>>;

    /// Adds files of one folder in one transaction. Files that the catalog
    /// already knows (same folder and file name) are not changed.
    ///
    /// Returns one outcome for each item of `photos`, in the same order.
    fn add_photos(&mut self, folder_id: FolderId, photos: &[NewPhoto]) -> Result<Vec<AddOutcome>>;

    /// Returns the file names of all photos directly in a folder.
    fn known_file_names(&self, folder_id: FolderId) -> Result<HashSet<String>>;

    /// Returns the photo with this id, or `None`.
    fn get_photo(&self, id: PhotoId) -> Result<Option<Photo>>;

    /// Returns the absolute path of the original file of a photo, or `None`
    /// if the photo does not exist.
    fn photo_path(&self, id: PhotoId) -> Result<Option<PathBuf>>;

    /// Returns the photos that match the query, in the sort order of the
    /// query, with its limit and offset applied.
    fn list_photos(&self, query: &PhotoQuery) -> Result<Vec<Photo>>;

    /// Returns the number of photos that match the query. The limit and
    /// offset of the query are ignored.
    fn count_photos(&self, query: &PhotoQuery) -> Result<u64>;

    /// Replaces the image information (capture time and size) of a photo.
    ///
    /// Returns [`CatalogError::ZeroDimension`](crate::CatalogError::ZeroDimension)
    /// if a width or height is 0, and
    /// [`CatalogError::PhotoNotFound`](crate::CatalogError::PhotoNotFound)
    /// if the photo does not exist.
    fn set_image_info(&mut self, id: PhotoId, info: ImageInfo) -> Result<()>;

    /// Replaces the image information of many photos in one transaction.
    ///
    /// Ids that are not in the catalog are skipped and returned in
    /// [`BatchInfoOutcome::unknown`]; they do not fail the batch. If a width
    /// or height is 0 (`ZeroDimension`) or the database fails, the method
    /// returns the error and changes nothing. If an id occurs more than
    /// once, the last item wins. Empty input does nothing.
    ///
    /// The default implementation calls [`set_image_info`](Self::set_image_info)
    /// for each item and is not atomic. Backends that can do better must
    /// override it.
    fn set_image_infos(&mut self, items: &[(PhotoId, ImageInfo)]) -> Result<BatchInfoOutcome> {
        let mut outcome = BatchInfoOutcome::default();
        for &(id, info) in items {
            match self.set_image_info(id, info) {
                Ok(()) => outcome.written += 1,
                Err(crate::CatalogError::PhotoNotFound(_)) => outcome.unknown.push(id),
                Err(e) => return Err(e),
            }
        }
        Ok(outcome)
    }

    /// Applies label changes to all photos in `ids`, in one transaction.
    ///
    /// If a photo does not exist, the method returns
    /// [`CatalogError::PhotoNotFound`](crate::CatalogError::PhotoNotFound)
    /// and changes nothing.
    fn update_labels(&mut self, ids: &[PhotoId], update: LabelUpdate) -> Result<()>;

    /// Sets the star rating of all photos in `ids`, in one transaction.
    fn set_rating(&mut self, ids: &[PhotoId], rating: Rating) -> Result<()> {
        self.update_labels(
            ids,
            LabelUpdate {
                rating: Some(rating),
                ..LabelUpdate::default()
            },
        )
    }

    /// Sets the flag of all photos in `ids`, in one transaction.
    fn set_flag(&mut self, ids: &[PhotoId], flag: Flag) -> Result<()> {
        self.update_labels(
            ids,
            LabelUpdate {
                flag: Some(flag),
                ..LabelUpdate::default()
            },
        )
    }

    /// Sets the color label of all photos in `ids`, in one transaction.
    fn set_color_label(&mut self, ids: &[PhotoId], color_label: ColorLabel) -> Result<()> {
        self.update_labels(
            ids,
            LabelUpdate {
                color_label: Some(color_label),
                ..LabelUpdate::default()
            },
        )
    }

    /// Returns the develop settings of a photo, or `None` if the photo has
    /// no stored settings.
    ///
    /// Returns [`CatalogError::PhotoNotFound`](crate::CatalogError::PhotoNotFound)
    /// if the photo does not exist.
    fn get_develop_settings(&self, id: PhotoId) -> Result<Option<DevelopSettings>>;

    /// Stores the develop settings of a photo. This replaces the settings
    /// that were stored before. Returns the stored settings.
    fn set_develop_settings(
        &mut self,
        id: PhotoId,
        schema_version: u32,
        settings: &serde_json::Value,
    ) -> Result<DevelopSettings>;

    /// Imports the supported photo files in `root` in place (no copy).
    ///
    /// See [`import_folder`](crate::import_folder).
    fn import_folder(
        &mut self,
        fs: &dyn FileSystem,
        root: &Path,
        recursive: bool,
    ) -> Result<ImportReport> {
        crate::import::import_folder(self, fs, root, recursive)
    }
}
