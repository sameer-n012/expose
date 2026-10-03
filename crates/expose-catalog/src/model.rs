//! Domain types of the catalog.
//!
//! All types are serializable with `serde`. The desktop shell sends them to
//! the UI as JSON. Struct fields use `camelCase` names in JSON.
//!
//! All times are signed Unix times in milliseconds (UTC).

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The id of a photo in the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PhotoId(pub i64);

impl fmt::Display for PhotoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The id of a folder in the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FolderId(pub i64);

impl fmt::Display for FolderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The error for a star rating that is not in the range 0 to 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("rating {0} is not in the range 0..=5")]
pub struct InvalidRating(pub u8);

/// A star rating from 0 (no stars) to 5.
///
/// The constructor makes sure that the value is in range. In JSON, a rating
/// is a number.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(try_from = "u8", into = "u8")]
pub struct Rating(u8);

impl Rating {
    /// The highest rating.
    pub const MAX: u8 = 5;

    /// Makes a rating. Returns an error if `stars` is more than 5.
    pub const fn new(stars: u8) -> Result<Self, InvalidRating> {
        if stars <= Self::MAX {
            Ok(Self(stars))
        } else {
            Err(InvalidRating(stars))
        }
    }

    /// Returns the number of stars.
    pub const fn stars(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for Rating {
    type Error = InvalidRating;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Rating> for u8 {
    fn from(value: Rating) -> Self {
        value.0
    }
}

/// The pick / reject flag of a photo.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Flag {
    /// No flag.
    #[default]
    None,
    /// The photo is a pick.
    Pick,
    /// The photo is rejected.
    Reject,
}

impl Flag {
    /// All flag values.
    pub const ALL: [Self; 3] = [Self::None, Self::Pick, Self::Reject];

    /// Returns the stored text of the flag. This text is also the JSON value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Pick => "pick",
            Self::Reject => "reject",
        }
    }

    /// Parses the stored text of a flag.
    pub fn from_str_opt(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.as_str() == text)
    }
}

/// The color label of a photo.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorLabel {
    /// No color label.
    #[default]
    None,
    /// Red label.
    Red,
    /// Yellow label.
    Yellow,
    /// Green label.
    Green,
    /// Blue label.
    Blue,
    /// Purple label.
    Purple,
}

impl ColorLabel {
    /// All color label values.
    pub const ALL: [Self; 6] = [
        Self::None,
        Self::Red,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::Purple,
    ];

    /// Returns the stored text of the label. This text is also the JSON value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Red => "red",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::Purple => "purple",
        }
    }

    /// Parses the stored text of a color label.
    pub fn from_str_opt(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == text)
    }
}

/// A folder in the catalog. The folder tree mirrors the file system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    /// The folder id.
    pub id: FolderId,
    /// The parent folder, or `None` for a top-level folder.
    pub parent_id: Option<FolderId>,
    /// The absolute, normalized path of the folder.
    pub path: PathBuf,
    /// The last component of the path (the display name).
    pub name: String,
}

/// A node in the folder tree, as returned by
/// [`CatalogStore::list_folder_tree`](crate::CatalogStore::list_folder_tree).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderNode {
    /// The folder.
    pub folder: Folder,
    /// The number of photos directly in this folder (not in subfolders).
    pub photo_count: u64,
    /// The subfolders, sorted by name.
    pub children: Vec<FolderNode>,
}

/// A photo in the catalog.
///
/// The original file is read-only. The catalog never writes to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Photo {
    /// The photo id.
    pub id: PhotoId,
    /// The folder that contains the file.
    pub folder_id: FolderId,
    /// The file name, with extension.
    pub file_name: String,
    /// The file size in bytes at import time.
    pub file_size: u64,
    /// The file modification time at import time (Unix milliseconds).
    pub file_modified_ms: i64,
    /// The time when the photo was added to the catalog (Unix milliseconds).
    pub imported_ms: i64,
    /// The capture time (Unix milliseconds), or `None` if it is not known.
    pub capture_time_ms: Option<i64>,
    /// The image width in pixels, or `None` if it is not known.
    pub width: Option<u32>,
    /// The image height in pixels, or `None` if it is not known.
    pub height: Option<u32>,
    /// The star rating.
    pub rating: Rating,
    /// The pick / reject flag.
    pub flag: Flag,
    /// The color label.
    pub color_label: ColorLabel,
}

/// The data of a file to add to the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewPhoto {
    /// The file name, with extension. It must not contain a path separator.
    pub file_name: String,
    /// The file size in bytes.
    pub file_size: u64,
    /// The file modification time (Unix milliseconds).
    pub file_modified_ms: i64,
}

/// The result of an attempt to add one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "id")]
pub enum AddOutcome {
    /// The catalog did not know the file. It is now added with this id.
    Added(PhotoId),
    /// The catalog already knew the file. Nothing changed.
    AlreadyKnown(PhotoId),
}

/// Image information that is read from the file contents (for example,
/// from embedded metadata). Import does not read it. A metadata reader
/// stores it later.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageInfo {
    /// The capture time (Unix milliseconds), or `None` if it is not known.
    pub capture_time_ms: Option<i64>,
    /// The image width in pixels, or `None` if it is not known.
    pub width: Option<u32>,
    /// The image height in pixels, or `None` if it is not known.
    pub height: Option<u32>,
}

/// The result of a batch write of image information.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchInfoOutcome {
    /// The number of photos that the batch changed.
    pub written: usize,
    /// The ids in the batch that do not exist in the catalog, in input
    /// order. The batch does not write these.
    pub unknown: Vec<PhotoId>,
}

/// Label changes to apply to one or more photos.
///
/// A field that is `None` is not changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LabelUpdate {
    /// The new star rating.
    pub rating: Option<Rating>,
    /// The new flag.
    pub flag: Option<Flag>,
    /// The new color label.
    pub color_label: Option<ColorLabel>,
}

impl LabelUpdate {
    /// Returns `true` if the update changes nothing.
    pub const fn is_empty(&self) -> bool {
        self.rating.is_none() && self.flag.is_none() && self.color_label.is_none()
    }
}

/// The develop settings of a photo, stored as an opaque JSON document.
///
/// The catalog does not interpret the document. The develop pipeline owns
/// its format and uses `schema_version` to migrate old documents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevelopSettings {
    /// The photo that the settings apply to.
    pub photo_id: PhotoId,
    /// The version of the settings document format.
    pub schema_version: u32,
    /// The settings document.
    pub settings: serde_json::Value,
    /// The time of the last change (Unix milliseconds).
    pub updated_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rating_accepts_0_to_5_and_rejects_more() {
        for stars in 0..=5 {
            assert_eq!(Rating::new(stars).map(Rating::stars), Ok(stars));
        }
        assert_eq!(Rating::new(6), Err(InvalidRating(6)));
        assert_eq!(Rating::try_from(255), Err(InvalidRating(255)));
    }

    #[test]
    fn rating_json_is_validated() {
        let ok: Rating = serde_json::from_str("4").unwrap();
        assert_eq!(ok.stars(), 4);
        assert!(serde_json::from_str::<Rating>("6").is_err());
        assert_eq!(serde_json::to_string(&ok).unwrap(), "4");
    }

    #[test]
    fn enums_round_trip_through_stored_text() {
        for flag in Flag::ALL {
            assert_eq!(Flag::from_str_opt(flag.as_str()), Some(flag));
            let json = serde_json::to_string(&flag).unwrap();
            assert_eq!(json, format!("\"{}\"", flag.as_str()));
        }
        for label in ColorLabel::ALL {
            assert_eq!(ColorLabel::from_str_opt(label.as_str()), Some(label));
            let json = serde_json::to_string(&label).unwrap();
            assert_eq!(json, format!("\"{}\"", label.as_str()));
        }
        assert_eq!(Flag::from_str_opt("maybe"), None);
        assert_eq!(ColorLabel::from_str_opt("orange"), None);
    }

    #[test]
    fn label_update_json_uses_defaults() {
        let update: LabelUpdate = serde_json::from_str(r#"{"colorLabel":"red"}"#).unwrap();
        assert_eq!(update.color_label, Some(ColorLabel::Red));
        assert_eq!(update.rating, None);
        assert!(!update.is_empty());
        assert!(LabelUpdate::default().is_empty());
    }
}
