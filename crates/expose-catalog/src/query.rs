//! Query parameters to list photos.

use serde::{Deserialize, Serialize};

use crate::model::{ColorLabel, Flag, FolderId, Rating};

/// The field to sort photos by.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortField {
    /// Capture time. Photos with no capture time are always last.
    #[default]
    CaptureTime,
    /// File name, case-insensitive.
    FileName,
    /// Star rating.
    Rating,
    /// Time of import into the catalog.
    ImportTime,
}

/// The sort direction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortDirection {
    /// Smallest value first.
    #[default]
    Ascending,
    /// Largest value first.
    Descending,
}

/// The sort order of a photo list.
///
/// Photos with equal sort values are ordered by file name and then by id,
/// both ascending. Thus the order is always deterministic.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PhotoSort {
    /// The field to sort by.
    pub field: SortField,
    /// The sort direction.
    pub direction: SortDirection,
}

/// Filters for a photo list. All filters that are set must match.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PhotoFilter {
    /// Keep only photos with this rating or higher.
    pub min_rating: Option<Rating>,
    /// Keep only photos with this flag.
    pub flag: Option<Flag>,
    /// Keep only photos with this color label.
    pub color_label: Option<ColorLabel>,
}

/// The parameters to list the photos of a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoQuery {
    /// The folder to list.
    pub folder_id: FolderId,
    /// If `true`, also include the photos of all subfolders.
    #[serde(default)]
    pub recursive: bool,
    /// The sort order.
    #[serde(default)]
    pub sort: PhotoSort,
    /// The filters.
    #[serde(default)]
    pub filter: PhotoFilter,
    /// The maximum number of photos to return. `None` means no limit.
    #[serde(default)]
    pub limit: Option<u32>,
    /// The number of photos to skip at the start of the sorted list.
    #[serde(default)]
    pub offset: u32,
}

impl PhotoQuery {
    /// Makes a query for the direct photos of `folder_id`, sorted by
    /// capture time, with no filters and no limit.
    pub fn new(folder_id: FolderId) -> Self {
        Self {
            folder_id,
            recursive: false,
            sort: PhotoSort::default(),
            filter: PhotoFilter::default(),
            limit: None,
            offset: 0,
        }
    }
}
