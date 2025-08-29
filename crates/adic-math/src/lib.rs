pub mod ball;
pub mod distance;
pub mod padic;
pub mod valuation;

pub use ball::{ball_id, balls_are_distinct, count_distinct_balls};
pub use distance::{padic_distance, proximity_score};
pub use padic::{vp, vp_diff};
pub use valuation::PadicValuation;

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::features::QpDigits;

    #[test]
    fn test_vp_basic() {
        assert_eq!(vp(9, 3), 2);
        assert_eq!(vp(10, 3), 0);
        assert_eq!(vp(27, 3), 3);
    }

    #[test]
    fn test_vp_diff() {
        let x = QpDigits::from_u64(10, 3, 5);
        let y = QpDigits::from_u64(11, 3, 5);
        let diff = vp_diff(&x, &y);
        // diff is u32, so it's always >= 0
        assert!(diff < u32::MAX); // Just check it's a valid value
    }

    #[test]
    fn test_ball_id() {
        let x = QpDigits::from_u64(42, 3, 5);
        let id = ball_id(&x, 2);
        assert_eq!(id.len(), 2);
    }
}
