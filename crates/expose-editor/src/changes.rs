//! Change logs of layer content.
//!
//! Each layer has one log for its pixel values and one log for its mask
//! values. A log records the rectangle of each content change with the new
//! content version (see [`crate::version`]). A cache that holds the values
//! of an older version can thus update only the changed area (for example,
//! the layer texture cache of [`GpuCompositor`](crate::GpuCompositor)).
//!
//! - The rectangles are in layer (or mask) pixel coordinates. A move of the
//!   layer does not change them.
//! - A change without a known rectangle (for example, a new mask) clears the
//!   log. Then a cache with an older version must load all values again.
//! - The log keeps at most [`LOG_CAPACITY`] changes. When it is full, it
//!   drops the oldest change. A cache with a version before the dropped
//!   change must then load all values again.
//! - A clone of a layer has a clone of the log. This is correct, because
//!   the clone has the same versions and the same values. Versions are
//!   unique in the process, so a version that one copy made after the clone
//!   is never in the log of the other copy.
//! - The log is not part of the document file and not part of layer
//!   equality.

use std::collections::VecDeque;

use expose_core::buffer::Rect;

/// The maximum number of changes in one log.
pub(crate) const LOG_CAPACITY: usize = 64;

/// The change log of one content (pixels or mask) of one layer.
#[derive(Debug, Clone)]
pub(crate) struct ChangeLog {
    /// The oldest version that the log can give the changes for.
    base: u64,
    /// The changes after `base`, oldest first: the new version and the
    /// changed rectangle. The versions increase.
    entries: VecDeque<(u64, Rect)>,
}

impl ChangeLog {
    /// Makes an empty log for content at `version`.
    pub(crate) fn new(version: u64) -> Self {
        Self {
            base: version,
            entries: VecDeque::new(),
        }
    }

    /// Clears the log after a change without a known rectangle. `version`
    /// is the new content version.
    pub(crate) fn reset(&mut self, version: u64) {
        self.base = version;
        self.entries.clear();
    }

    /// Records a change of `rect` that made the content version `version`.
    pub(crate) fn push(&mut self, version: u64, rect: Rect) {
        if self.entries.len() >= LOG_CAPACITY {
            if let Some((dropped, _)) = self.entries.pop_front() {
                // The changes before `dropped` are no longer known.
                self.base = dropped;
            }
        }
        self.entries.push_back((version, rect));
    }

    /// Returns the area that changed after content version `version`.
    pub(crate) fn since(&self, version: u64) -> Changes {
        let start = if version == self.base {
            0
        } else {
            match self.entries.iter().position(|(v, _)| *v == version) {
                Some(i) => i + 1,
                None => return Changes::Unknown,
            }
        };
        let area = self
            .entries
            .iter()
            .skip(start)
            .fold(None, |acc, (_, r)| union(acc, *r));
        area.map_or(Changes::Nothing, Changes::Area)
    }
}

/// The changes after one content version (see [`ChangeLog::since`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Changes {
    /// The log does not know the version: the changes are not known. Load
    /// all values again.
    Unknown,
    /// No value changed (for example, the version is the current version).
    Nothing,
    /// The union of all changed rectangles after the version.
    Area(Rect),
}

impl Changes {
    /// Returns `true` if the changes are known.
    pub(crate) const fn is_known(self) -> bool {
        !matches!(self, Self::Unknown)
    }

    /// Returns the changed area, or `None` if no value changed or the
    /// changes are not known.
    pub(crate) const fn area(self) -> Option<Rect> {
        match self {
            Self::Area(rect) => Some(rect),
            Self::Unknown | Self::Nothing => None,
        }
    }
}

/// Returns the smallest rectangle that contains `acc` and `rect`. Empty
/// rectangles have no area, so they do not change the result.
pub(crate) fn union(acc: Option<Rect>, rect: Rect) -> Option<Rect> {
    if rect.is_empty() {
        return acc;
    }
    let Some(a) = acc else {
        return Some(rect);
    };
    let x0 = a.x.min(rect.x);
    let y0 = a.y.min(rect.y);
    let x1 = a.right().max(rect.right());
    let y1 = a.bottom().max(rect.bottom());
    // Both rectangles have their right and bottom edges in u32 + u32
    // range. The union of two rectangles inside one buffer is inside that
    // buffer, so the sizes fit in u32. A larger value (only for rectangles
    // outside every buffer) saturates.
    let w = u32::try_from(x1 - u64::from(x0)).unwrap_or(u32::MAX);
    let h = u32::try_from(y1 - u64::from(y0)).unwrap_or(u32::MAX);
    Some(Rect::new(x0, y0, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_gives_the_union_after_a_version() {
        let mut log = ChangeLog::new(10);
        log.push(11, Rect::new(0, 0, 2, 2));
        log.push(12, Rect::new(5, 5, 1, 1));
        log.push(13, Rect::new(8, 1, 2, 1));
        assert_eq!(log.since(10), Changes::Area(Rect::new(0, 0, 10, 6)));
        assert_eq!(log.since(11), Changes::Area(Rect::new(5, 1, 5, 5)));
        assert_eq!(log.since(13), Changes::Nothing);
        assert_eq!(log.since(9), Changes::Unknown);
        assert_eq!(log.since(14), Changes::Unknown);
    }

    #[test]
    fn reset_forgets_the_old_versions() {
        let mut log = ChangeLog::new(1);
        log.push(2, Rect::new(0, 0, 1, 1));
        log.reset(3);
        assert_eq!(log.since(1), Changes::Unknown);
        assert_eq!(log.since(2), Changes::Unknown);
        assert_eq!(log.since(3), Changes::Nothing);
    }

    #[test]
    fn overflow_drops_the_oldest_change() {
        let mut log = ChangeLog::new(0);
        let count = u64::try_from(LOG_CAPACITY).unwrap() + 1;
        for v in 1..=count {
            log.push(v, Rect::new(0, 0, 1, 1));
        }
        assert_eq!(log.since(0), Changes::Unknown);
        assert_eq!(log.since(1), Changes::Area(Rect::new(0, 0, 1, 1)));
        assert_eq!(log.since(count), Changes::Nothing);
    }

    #[test]
    fn union_ignores_empty_rects() {
        assert_eq!(union(None, Rect::new(3, 3, 0, 5)), None);
        let r = Rect::new(1, 2, 3, 4);
        assert_eq!(union(Some(r), Rect::new(0, 0, 0, 0)), Some(r));
    }
}
