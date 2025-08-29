use crate::padic::vp_diff;
use adic_types::features::QpDigits;

pub fn padic_distance(x: &QpDigits, y: &QpDigits) -> f64 {
    assert_eq!(x.p, y.p, "Cannot compute distance between different primes");

    // Handle exact equality for robustness
    if x.digits == y.digits {
        return 0.0;
    }

    let valuation = vp_diff(x, y);

    let p = x.p as f64;
    p.powi(-(valuation as i32))
}

pub fn proximity_score(x: &QpDigits, y: &QpDigits, radius: u32) -> f64 {
    if radius == 0 {
        return 0.0; // Avoid division by zero
    }
    let valuation = vp_diff(x, y);
    let clamped = valuation.min(radius);
    clamped as f64 / radius as f64
}

pub fn proximity_weight(x: &QpDigits, y: &QpDigits, radius: u32, lambda: f64) -> f64 {
    let prox = proximity_score(x, y, radius);
    (lambda * prox).exp()
}

pub fn are_close(x: &QpDigits, y: &QpDigits, min_valuation: u32) -> bool {
    vp_diff(x, y) >= min_valuation
}

pub fn find_closest(reference: &QpDigits, candidates: &[QpDigits]) -> Option<usize> {
    if candidates.is_empty() {
        return None;
    }

    let mut best_idx = 0;
    let mut best_valuation = vp_diff(reference, &candidates[0]);

    for (i, candidate) in candidates.iter().enumerate().skip(1) {
        let val = vp_diff(reference, candidate);
        if val > best_valuation {
            best_valuation = val;
            best_idx = i;
        }
    }

    Some(best_idx)
}

pub fn find_furthest(reference: &QpDigits, candidates: &[QpDigits]) -> Option<usize> {
    if candidates.is_empty() {
        return None;
    }

    let mut worst_idx = 0;
    let mut worst_valuation = vp_diff(reference, &candidates[0]);

    for (i, candidate) in candidates.iter().enumerate().skip(1) {
        let val = vp_diff(reference, candidate);
        if val < worst_valuation {
            worst_valuation = val;
            worst_idx = i;
        }
    }

    Some(worst_idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_padic_distance() {
        let x = QpDigits::from_u64(10, 3, 5);
        let y = QpDigits::from_u64(11, 3, 5);
        let dist = padic_distance(&x, &y);
        assert_eq!(dist, 1.0);

        let x2 = QpDigits::from_u64(9, 3, 5);
        let y2 = QpDigits::from_u64(12, 3, 5);
        let dist2 = padic_distance(&x2, &y2);
        assert!(dist2 < 1.0);
    }

    #[test]
    fn test_padic_distance_same_value() {
        let x = QpDigits::from_u64(15, 3, 5);
        let y = QpDigits::from_u64(15, 3, 5);
        let dist = padic_distance(&x, &y);
        assert_eq!(dist, 0.0);
    }

    #[test]
    #[should_panic(expected = "Cannot compute distance between different primes")]
    fn test_padic_distance_different_primes() {
        let x = QpDigits::from_u64(10, 3, 5);
        let y = QpDigits::from_u64(10, 5, 5);
        padic_distance(&x, &y);
    }

    #[test]
    fn test_padic_distance_various_values() {
        let x = QpDigits::from_u64(0, 3, 5);
        let y = QpDigits::from_u64(3, 3, 5);
        let dist = padic_distance(&x, &y);
        assert!(dist > 0.0);

        // Test with larger values
        let x2 = QpDigits::from_u64(100, 3, 10);
        let y2 = QpDigits::from_u64(101, 3, 10);
        let dist2 = padic_distance(&x2, &y2);
        assert_eq!(dist2, 1.0);
    }

    #[test]
    fn test_proximity_score() {
        let x = QpDigits::from_u64(10, 3, 5);
        let y = QpDigits::from_u64(11, 3, 5);
        let score = proximity_score(&x, &y, 2);
        assert_eq!(score, 0.0);

        let x2 = QpDigits::from_u64(9, 3, 5);
        let y2 = QpDigits::from_u64(12, 3, 5);
        let score2 = proximity_score(&x2, &y2, 2);
        assert_eq!(score2, 0.5);
    }

    #[test]
    fn test_proximity_score_max_radius() {
        let x = QpDigits::from_u64(0, 3, 5);
        let y = QpDigits::from_u64(0, 3, 5);
        let score = proximity_score(&x, &y, 5);
        assert_eq!(score, 1.0); // Maximum proximity for identical values

        let x2 = QpDigits::from_u64(10, 3, 5);
        let y2 = QpDigits::from_u64(11, 3, 5);
        let score2 = proximity_score(&x2, &y2, 1);
        assert_eq!(score2, 0.0); // vp_diff is 0, clamped to 0, so 0/1 = 0.0
    }

    #[test]
    fn test_proximity_weight() {
        let x = QpDigits::from_u64(10, 3, 5);
        let y = QpDigits::from_u64(11, 3, 5);
        let weight = proximity_weight(&x, &y, 2, 1.0);
        assert_eq!(weight, 1.0);

        let x2 = QpDigits::from_u64(9, 3, 5);
        let y2 = QpDigits::from_u64(12, 3, 5);
        let weight2 = proximity_weight(&x2, &y2, 2, 1.0);
        assert!(weight2 > 1.0);
    }

    #[test]
    fn test_proximity_weight_with_lambda() {
        let x = QpDigits::from_u64(9, 3, 5);
        let y = QpDigits::from_u64(12, 3, 5);

        let weight1 = proximity_weight(&x, &y, 2, 0.5);
        let weight2 = proximity_weight(&x, &y, 2, 1.0);
        let weight3 = proximity_weight(&x, &y, 2, 2.0);

        assert!(weight1 < weight2);
        assert!(weight2 < weight3);

        // Test with zero lambda
        let weight_zero = proximity_weight(&x, &y, 2, 0.0);
        assert_eq!(weight_zero, 1.0);
    }

    #[test]
    fn test_are_close() {
        let x = QpDigits::from_u64(10, 3, 5);
        let y = QpDigits::from_u64(11, 3, 5);

        assert!(!are_close(&x, &y, 1));
        assert!(!are_close(&x, &y, 2));

        let x2 = QpDigits::from_u64(9, 3, 5);
        let y2 = QpDigits::from_u64(12, 3, 5);
        assert!(are_close(&x2, &y2, 1));
        assert!(!are_close(&x2, &y2, 2));

        // Identical values are close up to the precision limit
        assert!(are_close(&x, &x, 0));
        assert!(are_close(&x, &x, 5)); // precision is 5
        assert!(!are_close(&x, &x, 6)); // beyond precision
    }

    #[test]
    fn test_find_closest() {
        let reference = QpDigits::from_u64(10, 3, 5);
        // 10=[1,0,1,0,0]
        let candidates = vec![
            QpDigits::from_u64(11, 3, 5), // [2,0,1,0,0] - differs at pos 0, dist=1
            QpDigits::from_u64(13, 3, 5), // [1,1,1,0,0] - differs at pos 1, dist=1/3
            QpDigits::from_u64(19, 3, 5), // [1,0,2,0,0] - differs at pos 2, dist=1/9
        ];

        let closest = find_closest(&reference, &candidates);
        assert_eq!(closest, Some(2)); // 19 is closest with distance 1/9
    }

    #[test]
    fn test_find_closest_empty() {
        let reference = QpDigits::from_u64(10, 3, 5);
        let candidates: Vec<QpDigits> = vec![];

        let closest = find_closest(&reference, &candidates);
        assert_eq!(closest, None);
    }

    #[test]
    fn test_find_closest_single() {
        let reference = QpDigits::from_u64(10, 3, 5);
        let candidates = vec![QpDigits::from_u64(15, 3, 5)];

        let closest = find_closest(&reference, &candidates);
        assert_eq!(closest, Some(0));
    }

    #[test]
    fn test_find_closest_identical() {
        let reference = QpDigits::from_u64(10, 3, 5);
        let candidates = vec![
            QpDigits::from_u64(15, 3, 5),
            QpDigits::from_u64(10, 3, 5), // Identical to reference
            QpDigits::from_u64(20, 3, 5),
        ];

        let closest = find_closest(&reference, &candidates);
        assert_eq!(closest, Some(1)); // Should find the identical one
    }

    #[test]
    fn test_find_furthest() {
        let reference = QpDigits::from_u64(10, 3, 5);
        let candidates = vec![
            QpDigits::from_u64(11, 3, 5), // Distance 1
            QpDigits::from_u64(13, 3, 5), // Distance closer
            QpDigits::from_u64(19, 3, 5), // Distance closer
        ];

        let furthest = find_furthest(&reference, &candidates);
        assert_eq!(furthest, Some(0)); // First one is furthest
    }

    #[test]
    fn test_find_furthest_empty() {
        let reference = QpDigits::from_u64(10, 3, 5);
        let candidates: Vec<QpDigits> = vec![];

        let furthest = find_furthest(&reference, &candidates);
        assert_eq!(furthest, None);
    }

    #[test]
    fn test_find_furthest_single() {
        let reference = QpDigits::from_u64(10, 3, 5);
        let candidates = vec![QpDigits::from_u64(15, 3, 5)];

        let furthest = find_furthest(&reference, &candidates);
        assert_eq!(furthest, Some(0));
    }
}
