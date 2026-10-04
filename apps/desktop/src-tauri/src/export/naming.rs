//! Output file names and name conflicts.
//!
//! The functions in this module are pure. They do not touch the file
//! system: the caller gives a function that tells if a name exists in the
//! destination folder.

use std::collections::HashSet;
use std::path::Path;

use super::request::{ConflictPolicy, Naming};

/// The maximum number of candidate names (`name`, `name-2`, ...,
/// `name-N`) that [`JobNames::resolve`] tries before it stops.
pub const MAX_NAME_ATTEMPTS: u32 = 10_000;

/// The result of a name resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Write the file with this name. No file with this name exists.
    Write(String),
    /// Write the file with this name. It replaces an existing file.
    Overwrite(String),
    /// Do not write. A file with this name exists, and the policy is
    /// [`ConflictPolicy::Skip`].
    Skip(String),
}

/// No free name was found in [`MAX_NAME_ATTEMPTS`] attempts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("no free file name for \"{base}.{extension}\" after {MAX_NAME_ATTEMPTS} attempts")]
pub struct NoFreeName {
    /// The base name (stem and suffix) without extension.
    pub base: String,
    /// The extension without a dot.
    pub extension: String,
}

/// Returns the base name of an output file (without extension): the stem
/// of `source` plus the suffix of `naming`.
///
/// If `source` has no stem (or the stem is empty), the base name is
/// `photo-<fallback_id>`. A stem that is not valid UTF-8 is converted with
/// replacement characters.
pub fn base_name(source: &Path, naming: &Naming, fallback_id: i64) -> String {
    let stem = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| format!("photo-{fallback_id}"));
    match naming {
        Naming::Original => stem,
        Naming::Suffix { suffix } => format!("{stem}{suffix}"),
    }
}

/// The names that one export job uses.
///
/// Two photos of one job never get the same output name, with all
/// policies. Names are compared without case, because the default file
/// systems of macOS and Windows do not make a difference between
/// `A.jpg` and `a.jpg`.
#[derive(Debug, Default)]
pub struct JobNames {
    used: HashSet<String>,
}

impl JobNames {
    /// Makes an empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Selects the output name for `base` + `.` + `extension`.
    ///
    /// The candidates are `base.ext`, `base-2.ext`, `base-3.ext`, and so
    /// on. A candidate is not available if an earlier photo of this job
    /// uses it, or if it is equal to `protected` (the original file of the
    /// photo, when the destination is the source folder). The policy then
    /// applies to the first available candidate:
    ///
    /// - [`ConflictPolicy::UniqueName`]: the first candidate that also
    ///   does not exist (`exists` gives `false`).
    /// - [`ConflictPolicy::Overwrite`]: the first available candidate. If
    ///   it exists, the result is [`Resolution::Overwrite`].
    /// - [`ConflictPolicy::Skip`]: the first available candidate. If it
    ///   exists, the result is [`Resolution::Skip`].
    ///
    /// The selected name is recorded as used, also for a skip.
    ///
    /// # Errors
    ///
    /// Returns [`NoFreeName`] after [`MAX_NAME_ATTEMPTS`] candidates.
    pub fn resolve(
        &mut self,
        base: &str,
        extension: &str,
        policy: ConflictPolicy,
        protected: Option<&str>,
        exists: impl Fn(&str) -> bool,
    ) -> Result<Resolution, NoFreeName> {
        let protected = protected.map(str::to_lowercase);
        for attempt in 1..=MAX_NAME_ATTEMPTS {
            let name = if attempt == 1 {
                format!("{base}.{extension}")
            } else {
                format!("{base}-{attempt}.{extension}")
            };
            let key = name.to_lowercase();
            if self.used.contains(&key) || protected.as_deref() == Some(key.as_str()) {
                continue;
            }
            let resolution = match (policy, exists(&name)) {
                (_, false) => Resolution::Write(name),
                (ConflictPolicy::UniqueName, true) => continue,
                (ConflictPolicy::Overwrite, true) => Resolution::Overwrite(name),
                (ConflictPolicy::Skip, true) => Resolution::Skip(name),
            };
            self.used.insert(key);
            return Ok(resolution);
        }
        Err(NoFreeName {
            base: base.to_owned(),
            extension: extension.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn on_disk(names: &[&str]) -> impl Fn(&str) -> bool + use<> {
        let names: HashSet<String> = names.iter().map(|name| name.to_lowercase()).collect();
        move |name: &str| names.contains(&name.to_lowercase())
    }

    #[test]
    fn base_name_uses_stem_and_suffix() {
        let path = PathBuf::from("/photos/IMG_0001.CR3");
        assert_eq!(base_name(&path, &Naming::Original, 1), "IMG_0001");
        let naming = Naming::Suffix {
            suffix: "_web".into(),
        };
        assert_eq!(base_name(&path, &naming, 1), "IMG_0001_web");
        assert_eq!(base_name(Path::new("/"), &Naming::Original, 7), "photo-7");
    }

    #[test]
    fn unique_name_skips_existing_and_used_names() {
        let exists = on_disk(&["a.jpg", "a-2.jpg"]);
        let mut names = JobNames::new();
        let first = names.resolve("a", "jpg", ConflictPolicy::UniqueName, None, &exists);
        assert_eq!(first, Ok(Resolution::Write("a-3.jpg".into())));
        let second = names.resolve("a", "jpg", ConflictPolicy::UniqueName, None, &exists);
        assert_eq!(second, Ok(Resolution::Write("a-4.jpg".into())));
        let other = names.resolve("b", "jpg", ConflictPolicy::UniqueName, None, &exists);
        assert_eq!(other, Ok(Resolution::Write("b.jpg".into())));
    }

    #[test]
    fn overwrite_replaces_disk_files_but_not_names_of_this_job() {
        let exists = on_disk(&["a.jpg"]);
        let mut names = JobNames::new();
        let first = names.resolve("a", "jpg", ConflictPolicy::Overwrite, None, &exists);
        assert_eq!(first, Ok(Resolution::Overwrite("a.jpg".into())));
        // Same name from a second photo, other case: no overwrite in the job.
        let second = names.resolve("A", "jpg", ConflictPolicy::Overwrite, None, &exists);
        assert_eq!(second, Ok(Resolution::Write("A-2.jpg".into())));
    }

    #[test]
    fn skip_skips_existing_files() {
        let exists = on_disk(&["a.jpg", "a-2.jpg"]);
        let mut names = JobNames::new();
        let first = names.resolve("a", "jpg", ConflictPolicy::Skip, None, &exists);
        assert_eq!(first, Ok(Resolution::Skip("a.jpg".into())));
        let second = names.resolve("a", "jpg", ConflictPolicy::Skip, None, &exists);
        assert_eq!(second, Ok(Resolution::Skip("a-2.jpg".into())));
        let third = names.resolve("a", "jpg", ConflictPolicy::Skip, None, &exists);
        assert_eq!(third, Ok(Resolution::Write("a-3.jpg".into())));
    }

    #[test]
    fn protected_original_is_never_selected() {
        let exists = on_disk(&["a.jpg"]);
        for policy in [
            ConflictPolicy::UniqueName,
            ConflictPolicy::Overwrite,
            ConflictPolicy::Skip,
        ] {
            let mut names = JobNames::new();
            let result = names.resolve("a", "jpg", policy, Some("A.JPG"), &exists);
            assert_eq!(
                result,
                Ok(Resolution::Write("a-2.jpg".into())),
                "{policy:?}"
            );
        }
    }

    #[test]
    fn stops_after_max_attempts() {
        let mut names = JobNames::new();
        let result = names.resolve("a", "jpg", ConflictPolicy::UniqueName, None, |_| true);
        assert_eq!(
            result,
            Err(NoFreeName {
                base: "a".into(),
                extension: "jpg".into()
            })
        );
    }
}
