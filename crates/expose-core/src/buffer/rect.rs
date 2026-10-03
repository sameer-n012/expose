//! Integer pixel rectangle.

/// An axis-aligned rectangle in pixel coordinates.
///
/// `x` and `y` are the top-left corner. The rectangle covers the columns
/// `x..x + width` and the rows `y..y + height`. Use it for tiles and crops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rect {
    /// Left edge (first column).
    pub x: u32,
    /// Top edge (first row).
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl Rect {
    /// Makes a rectangle.
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Makes a rectangle at the origin with the given size.
    pub const fn from_size(width: u32, height: u32) -> Self {
        Self::new(0, 0, width, height)
    }

    /// Returns the column after the right edge (`x + width`).
    ///
    /// The result is `u64`, so it cannot overflow.
    pub const fn right(&self) -> u64 {
        self.x as u64 + self.width as u64
    }

    /// Returns the row after the bottom edge (`y + height`).
    ///
    /// The result is `u64`, so it cannot overflow.
    pub const fn bottom(&self) -> u64 {
        self.y as u64 + self.height as u64
    }

    /// Returns `true` if the rectangle covers no pixels.
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Returns the number of pixels in the rectangle.
    pub const fn area(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// Returns `true` if the pixel `(px, py)` is inside the rectangle.
    pub const fn contains_point(&self, px: u32, py: u32) -> bool {
        px >= self.x && (px as u64) < self.right() && py >= self.y && (py as u64) < self.bottom()
    }

    /// Returns `true` if `other` is fully inside this rectangle.
    ///
    /// An empty `other` is inside if its corner is inside or on the edge.
    pub const fn contains_rect(&self, other: &Rect) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    /// Returns the overlap of two rectangles, or `None` if they do not
    /// overlap (an empty overlap is `None`).
    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());
        if x1 <= u64::from(x0) || y1 <= u64::from(y0) {
            return None;
        }
        // The overlap is not wider than either input, so the size fits in u32.
        let width = u32::try_from(x1 - u64::from(x0)).ok()?;
        let height = u32::try_from(y1 - u64::from(y0)).ok()?;
        Some(Rect::new(x0, y0, width, height))
    }

    /// Returns the overlap of this rectangle with a rectangle of size
    /// `width` x `height` at the signed position `(x, y)`. Use it to clamp
    /// a layer with a signed offset to a region. The result is in the same
    /// coordinates as `self`. `None` means no overlap.
    pub fn intersection_signed(&self, x: i32, y: i32, width: u32, height: u32) -> Option<Rect> {
        let x0 = i64::from(x).max(i64::from(self.x));
        let y0 = i64::from(y).max(i64::from(self.y));
        let x1 = (i64::from(x) + i64::from(width)).min(i64::from(self.x) + i64::from(self.width));
        let y1 = (i64::from(y) + i64::from(height)).min(i64::from(self.y) + i64::from(self.height));
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        // The overlap is inside `self`, so all values fit in u32.
        Some(Rect::new(
            u32::try_from(x0).ok()?,
            u32::try_from(y0).ok()?,
            u32::try_from(x1 - x0).ok()?,
            u32::try_from(y1 - y0).ok()?,
        ))
    }

    /// Moves the rectangle by a signed offset. Returns `None` if the new
    /// corner is negative or does not fit in `u32`.
    pub fn translated(&self, dx: i32, dy: i32) -> Option<Rect> {
        let x = u32::try_from(i64::from(self.x) + i64::from(dx)).ok()?;
        let y = u32::try_from(i64::from(self.y) + i64::from(dy)).ok()?;
        Some(Rect::new(x, y, self.width, self.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersection_of_overlapping_rects() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 6, 10, 10);
        assert_eq!(a.intersection(&b), Some(Rect::new(5, 6, 5, 4)));
        assert_eq!(b.intersection(&a), Some(Rect::new(5, 6, 5, 4)));
    }

    #[test]
    fn intersection_of_disjoint_or_touching_rects_is_none() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(a.intersection(&Rect::new(10, 0, 5, 5)), None);
        assert_eq!(a.intersection(&Rect::new(20, 20, 5, 5)), None);
        assert_eq!(a.intersection(&Rect::new(2, 2, 0, 5)), None);
    }

    #[test]
    fn signed_intersection_and_translation() {
        let r = Rect::new(2, 2, 6, 6);
        assert_eq!(
            r.intersection_signed(-3, 0, 6, 4),
            Some(Rect::new(2, 2, 1, 2))
        );
        assert_eq!(r.intersection_signed(i32::MIN, 0, 5, 5), None);
        assert_eq!(r.intersection_signed(8, 2, 3, 3), None);
        assert_eq!(r.translated(-2, 1), Some(Rect::new(0, 3, 6, 6)));
        assert_eq!(r.translated(-3, 0), None);
        assert_eq!(Rect::new(u32::MAX, 0, 1, 1).translated(1, 0), None);
    }

    #[test]
    fn edges_do_not_overflow() {
        let r = Rect::new(u32::MAX, u32::MAX, u32::MAX, 1);
        assert_eq!(r.right(), u64::from(u32::MAX) * 2);
        let big = Rect::from_size(u32::MAX, u32::MAX);
        assert_eq!(
            big.intersection(&r),
            None,
            "x = u32::MAX is outside 0..u32::MAX"
        );
        let inner = Rect::new(u32::MAX - 2, 0, 10, 1);
        assert_eq!(
            big.intersection(&inner),
            Some(Rect::new(u32::MAX - 2, 0, 2, 1))
        );
    }

    #[test]
    fn containment() {
        let a = Rect::new(2, 2, 4, 4);
        assert!(a.contains_point(2, 2));
        assert!(a.contains_point(5, 5));
        assert!(!a.contains_point(6, 5));
        assert!(a.contains_rect(&Rect::new(3, 3, 3, 3)));
        assert!(!a.contains_rect(&Rect::new(3, 3, 4, 3)));
        assert_eq!(a.area(), 16);
        assert!(Rect::new(1, 1, 0, 3).is_empty());
    }
}
