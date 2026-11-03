use crate::int_to_f32;

/// Represents an x-y coordinate.
#[derive(Debug, Clone)]
pub struct Point {
  pub x: i32,
  pub y: i32,
}

impl Point {
  /// Calculates the Euclidean distance between this point and another
  /// point.
  #[must_use]
  pub fn distance_between(&self, other: &Point) -> f32 {
    let dx = self.x - other.x;
    let dy = self.y - other.y;

    int_to_f32(dx * dx + dy * dy).sqrt()
  }
}
