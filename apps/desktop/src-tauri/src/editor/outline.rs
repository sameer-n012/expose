//! The outline of the selection of a document, as closed polygons.

use expose_core::CoreError;
use expose_core::buffer::trace_contours;
use expose_editor::Document;

use super::EditorEngineError;
use super::dto::SelectionOutline;

/// The maximum number of points of all outlines of a result. The UI draws
/// the points as a moving dashed line, so more points cost time and give
/// no gain.
pub const MAX_OUTLINE_POINTS: usize = 100_000;

/// The mask value that separates inside from outside.
const THRESHOLD: f32 = 0.5;

/// Traces the outline of the selection of `document`.
///
/// `step` is the sample step in pixels. If the outline has more than
/// `max_points` points, the function doubles the step and traces again.
/// The result holds the step that fits. A document without a selection
/// gives no contours.
///
/// # Errors
///
/// - [`EditorEngineError::InvalidStep`] if `step` is 0.
/// - [`EditorEngineError::Editor`] if the tracing fails for another reason.
/// - [`EditorEngineError::Internal`] if no step fits the cap (the cap is
///   smaller than the points of the coarsest outline).
pub fn selection_outline(
    document: &Document,
    step: u32,
    max_points: usize,
) -> Result<SelectionOutline, EditorEngineError> {
    if step == 0 {
        return Err(EditorEngineError::InvalidStep);
    }
    let selection_version = document.selection_version();
    let Some(selection) = document.selection() else {
        return Ok(SelectionOutline {
            selection_version,
            step,
            contours: Vec::new(),
        });
    };
    // The loop ends: each pass doubles `step`, and `checked_mul` fails
    // when `step` passes `u32::MAX`, so there are at most 32 passes. With
    // a step of at least the long edge of the canvas, the grid has one
    // cell, so the outline has few points and fits the normal cap.
    let mut step = step;
    loop {
        match trace_contours(selection.as_gray(), THRESHOLD, step, max_points) {
            Ok(contours) => {
                let contours = contours
                    .into_iter()
                    .map(|c| c.points.into_iter().flatten().collect())
                    .collect();
                return Ok(SelectionOutline {
                    selection_version,
                    step,
                    contours,
                });
            }
            Err(CoreError::ContourTooLarge { .. }) => {
                step = step.checked_mul(2).ok_or(EditorEngineError::Internal(
                    "no outline step fits the point cap",
                ))?;
            }
            Err(other) => return Err(EditorEngineError::Editor(other.into())),
        }
    }
}

#[cfg(test)]
mod tests {
    // Test code: a failed setup is a test failure.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use expose_core::buffer::WorkingImage;
    use expose_editor::{SelectionMode, SelectionShape};

    use super::super::session::build_document;
    use super::*;

    fn document() -> Document {
        let image = WorkingImage::new_filled(64, 48, [0.5, 0.5, 0.5, 1.0]).unwrap();
        build_document(image).unwrap()
    }

    fn bounds(values: impl Iterator<Item = f32>) -> (f32, f32) {
        values.fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        })
    }

    #[test]
    fn no_selection_gives_no_contours() {
        let doc = document();
        let outline = selection_outline(&doc, 1, MAX_OUTLINE_POINTS).unwrap();
        assert!(outline.contours.is_empty());
        assert_eq!(outline.step, 1);
        assert_eq!(outline.selection_version, doc.selection_version());
    }

    #[test]
    fn rect_gives_one_contour_with_matching_bounds() {
        let mut doc = document();
        doc.select_shape(
            &SelectionShape::Rect {
                x: 10.0,
                y: 8.0,
                width: 20.0,
                height: 16.0,
            },
            SelectionMode::Replace,
            false,
            0.0,
        )
        .unwrap();
        let outline = selection_outline(&doc, 1, MAX_OUTLINE_POINTS).unwrap();
        assert_eq!(outline.contours.len(), 1);
        let flat = &outline.contours[0];
        assert_eq!(flat.len() % 2, 0);
        let (x0, x1) = bounds(flat.iter().step_by(2).copied());
        let (y0, y1) = bounds(flat.iter().skip(1).step_by(2).copied());
        assert!(
            (x0 - 10.0).abs() <= 1.0 && (x1 - 30.0).abs() <= 1.0,
            "{x0} {x1}"
        );
        assert!(
            (y0 - 8.0).abs() <= 1.0 && (y1 - 24.0).abs() <= 1.0,
            "{y0} {y1}"
        );
        assert_eq!(outline.selection_version, doc.selection_version());
    }

    #[test]
    fn small_cap_gives_a_larger_step() {
        let mut doc = document();
        doc.select_shape(
            &SelectionShape::Ellipse {
                x: 2.0,
                y: 2.0,
                width: 60.0,
                height: 44.0,
            },
            SelectionMode::Replace,
            true,
            0.0,
        )
        .unwrap();
        let count = |o: &SelectionOutline| o.contours.iter().map(|c| c.len() / 2).sum::<usize>();
        let fine = selection_outline(&doc, 1, MAX_OUTLINE_POINTS).unwrap();
        assert_eq!(fine.step, 1);
        let cap = count(&fine) / 2;
        let coarse = selection_outline(&doc, 1, cap).unwrap();
        assert!(coarse.step > 1, "step {}", coarse.step);
        assert!(count(&coarse) <= cap);
    }

    #[test]
    fn step_zero_is_an_error() {
        let doc = document();
        assert!(matches!(
            selection_outline(&doc, 0, MAX_OUTLINE_POINTS),
            Err(EditorEngineError::InvalidStep)
        ));
    }
}
