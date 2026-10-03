//! Undo and redo with named steps.
//!
//! # Model
//!
//! The history is a list of [`HistoryEntry`] values and a cursor. The
//! entries before the cursor are applied (they can be undone). The entries
//! at and after the cursor are undone (they can be redone). Cursor 0 is the
//! oldest state that the history can restore.
//!
//! - [`History::push`] records a new step. It drops all undone entries (the
//!   redo branch). If the number of entries is then more than the maximum,
//!   the oldest entries are removed (their changes can no longer be
//!   undone).
//! - An entry holds the [`Edit`] values that the operations returned.
//!   Undo applies them in reverse order and keeps their inverses for redo.
//!   Thus each entry holds only one set of edits at any time.
//!
//! # Memory
//!
//! Pixel writes store only the old values of the written rectangle (see
//! [`crate::ops`]). The memory of the history is thus the sum of the
//! changed areas, not of whole layers. [`History::memory_bytes`] reports
//! the approximate total.
//!
//! # Consistency
//!
//! The history assumes that all changes to the document since the first
//! entry were recorded in it. If an edit fails (for example, because the
//! document was changed without a record), the history restores the state
//! before the failed step and returns the error.

use std::collections::VecDeque;

use crate::document::Document;
use crate::error::{EditorError, EditorResult};
use crate::ops::Edit;

/// The default maximum number of steps.
pub const DEFAULT_MAX_STEPS: usize = 50;

/// One named step in the history.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    name: String,
    /// Undo edits (if applied) or redo edits (if undone), in the order of
    /// the original operations.
    edits: Vec<Edit>,
}

impl HistoryEntry {
    /// Returns the step name for the history panel (for example, "Brush
    /// Stroke").
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the approximate memory of the step in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.edits.iter().map(Edit::memory_bytes).sum::<usize>() + self.name.capacity()
    }
}

/// Command-based undo and redo history.
#[derive(Debug, Clone, PartialEq)]
pub struct History {
    entries: VecDeque<HistoryEntry>,
    /// Number of applied entries.
    cursor: usize,
    max_steps: usize,
}

impl Default for History {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            cursor: 0,
            max_steps: DEFAULT_MAX_STEPS,
        }
    }
}

impl History {
    /// Makes an empty history that keeps at most `max_steps` entries.
    ///
    /// Returns [`EditorError::InvalidHistory`] if `max_steps` is 0.
    pub fn new(max_steps: usize) -> EditorResult<Self> {
        if max_steps == 0 {
            return Err(EditorError::InvalidHistory("max_steps must be at least 1"));
        }
        Ok(Self {
            max_steps,
            ..Self::default()
        })
    }

    /// Returns the maximum number of entries.
    pub const fn max_steps(&self) -> usize {
        self.max_steps
    }

    /// Records a step. `edits` are the edits that the operations of the step
    /// returned, in the order of the operations.
    ///
    /// This drops the redo branch and, if necessary, the oldest entries.
    pub fn push(&mut self, name: impl Into<String>, edits: Vec<Edit>) {
        self.entries.truncate(self.cursor);
        self.entries.push_back(HistoryEntry {
            name: name.into(),
            edits,
        });
        self.cursor = self.entries.len();
        self.evict();
    }

    fn evict(&mut self) {
        while self.entries.len() > self.max_steps {
            self.entries.pop_front();
            self.cursor = self.cursor.saturating_sub(1);
        }
    }

    /// Returns all entries, oldest first.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &HistoryEntry> {
        self.entries.iter()
    }

    /// Returns the number of applied entries. Entries `0..cursor()` are
    /// applied; the others can be redone.
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// Returns `true` if an entry can be undone.
    pub const fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    /// Returns `true` if an entry can be redone.
    pub fn can_redo(&self) -> bool {
        self.cursor < self.entries.len()
    }

    /// Returns the approximate memory of all entries in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.entries.iter().map(HistoryEntry::memory_bytes).sum()
    }

    /// Removes all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.cursor = 0;
    }

    /// Undoes the last applied entry. Returns `false` if there is nothing
    /// to undo.
    pub fn undo(&mut self, doc: &mut Document) -> EditorResult<bool> {
        if self.cursor == 0 {
            return Ok(false);
        }
        let index = self.cursor - 1;
        self.flip(doc, index)?;
        self.cursor = index;
        Ok(true)
    }

    /// Redoes the next undone entry. Returns `false` if there is nothing to
    /// redo.
    pub fn redo(&mut self, doc: &mut Document) -> EditorResult<bool> {
        if self.cursor >= self.entries.len() {
            return Ok(false);
        }
        self.flip(doc, self.cursor)?;
        self.cursor += 1;
        Ok(true)
    }

    /// Undoes or redoes entries until `cursor()` is `step`. Use it for a
    /// click in the history panel.
    ///
    /// Returns [`EditorError::IndexOutOfRange`] if `step` is more than the
    /// number of entries.
    pub fn jump_to(&mut self, doc: &mut Document, step: usize) -> EditorResult<()> {
        if step > self.entries.len() {
            return Err(EditorError::IndexOutOfRange {
                index: step,
                len: self.entries.len(),
            });
        }
        while self.cursor > step {
            self.undo(doc)?;
        }
        while self.cursor < step {
            self.redo(doc)?;
        }
        Ok(())
    }

    /// Applies the edits of entry `index` in reverse order and stores their
    /// inverses (in original order). An applied entry becomes undone, and
    /// an undone entry becomes applied.
    ///
    /// If an edit fails, the function applies the inverses that it already
    /// has, so that the document returns to its state before the call. The
    /// failed entry cannot be used again, so the function also removes the
    /// entries that depend on it: for an undo, the entry and all older
    /// entries; for a redo, the entry and all newer entries.
    fn flip(&mut self, doc: &mut Document, index: usize) -> EditorResult<()> {
        let undoing = index < self.cursor;
        let len = self.entries.len();
        let entry = self
            .entries
            .get_mut(index)
            .ok_or(EditorError::IndexOutOfRange { index, len })?;
        let mut pending = std::mem::take(&mut entry.edits);
        let mut inverses: Vec<Edit> = Vec::with_capacity(pending.len());
        while let Some(edit) = pending.pop() {
            match doc.apply_edit(edit) {
                Ok(inv) => inverses.push(inv),
                Err(e) => {
                    // Roll back: apply the collected inverses in reverse
                    // order. They restore the state before this call.
                    while let Some(inv) = inverses.pop() {
                        doc.apply_edit(inv)?;
                    }
                    if undoing {
                        self.entries.drain(..=index);
                        self.cursor = 0;
                    } else {
                        self.entries.truncate(index);
                    }
                    return Err(e);
                }
            }
        }
        // `inverses` is in reverse order of the operations.
        inverses.reverse();
        entry.edits = inverses;
        Ok(())
    }
}
