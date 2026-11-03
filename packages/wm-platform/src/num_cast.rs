//! Conversions between integers and the `f32` amounts used for fractional
//! sizes.
//!
//! Each conversion gives the same result as the matching `as` cast. They
//! exist so that the rounding and saturation are chosen once and written
//! down here, rather than left implicit in casts across the codebase.

use num_traits::{NumCast, PrimInt};

/// Converts an integer to the nearest `f32`.
///
/// Integers up to 2^24 in magnitude convert exactly, which covers every
/// pixel length, coordinate and count the WM handles. Larger values round
/// to the nearest representable `f32`, and values beyond its range become
/// infinite.
#[must_use]
pub fn int_to_f32<T: PrimInt>(value: T) -> f32 {
  value.to_f32().unwrap_or_else(|| {
    if value < T::zero() {
      f32::NEG_INFINITY
    } else {
      f32::INFINITY
    }
  })
}

/// Converts an `f32` to an integer, truncating toward zero.
///
/// Values outside the integer type's range saturate at its bounds, and NaN
/// becomes zero.
#[must_use]
pub fn f32_to_int<T: PrimInt>(value: f32) -> T {
  <T as NumCast>::from(value).unwrap_or_else(|| {
    if value.is_nan() {
      T::zero()
    } else if value.is_sign_negative() {
      T::min_value()
    } else {
      T::max_value()
    }
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn int_to_f32_is_exact_for_pixel_values() {
    assert!((int_to_f32(1920) - 1920.0).abs() < f32::EPSILON);
    assert!((int_to_f32(-32_000) + 32_000.0).abs() < f32::EPSILON);
    assert!((int_to_f32(16_777_216_i32) - 16_777_216.0).abs() < 1.0);
    assert!((int_to_f32(3_usize) - 3.0).abs() < f32::EPSILON);
  }

  #[test]
  fn f32_to_int_truncates_toward_zero() {
    assert_eq!(f32_to_int::<i32>(29.999), 29);
    assert_eq!(f32_to_int::<i32>(-29.999), -29);
    assert_eq!(f32_to_int::<u8>(191.25), 191);
  }

  #[test]
  fn f32_to_int_saturates_like_as() {
    assert_eq!(f32_to_int::<i32>(1e20), i32::MAX);
    assert_eq!(f32_to_int::<i32>(-1e20), i32::MIN);
    assert_eq!(f32_to_int::<i32>(f32::INFINITY), i32::MAX);
    assert_eq!(f32_to_int::<i32>(f32::NAN), 0);
    assert_eq!(f32_to_int::<u8>(300.0), u8::MAX);
    assert_eq!(f32_to_int::<u8>(-3.0), 0);
    assert_eq!(f32_to_int::<u8>(-0.5), 0);
  }
}
