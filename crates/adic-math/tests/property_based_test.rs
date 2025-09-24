use adic_math::{
    ball_id, balls_are_distinct, count_distinct_balls, padic_distance, proximity_score, vp, vp_diff,
};
use adic_types::features::QpDigits;
use proptest::prelude::*;

/// Generate arbitrary QpDigits for property testing
fn arbitrary_qp_digits() -> impl Strategy<Value = QpDigits> {
    (1u64..1000, 2u32..10, 1usize..20)
        .prop_map(|(value, p, precision)| QpDigits::from_u64(value, p, precision))
}

/// Generate small QpDigits for more focused testing
fn small_qp_digits() -> impl Strategy<Value = QpDigits> {
    (0u64..100, 2u32..5, 1usize..10)
        .prop_map(|(value, p, precision)| QpDigits::from_u64(value, p, precision))
}

proptest! {
    /// Property: p-adic valuation doesn't panic and returns a valid result
    #[test]
    fn prop_vp_doesnt_panic(value in 1u64..1000, p in 2u32..10) {
        let valuation = vp(value, p);
        // Just ensure the function completes without panic
        // For non-zero values, valuation should be less than 64 (max bits in u64)
        prop_assert!(valuation < 64);
    }

    /// Property: vp(0) should be the maximum possible value
    #[test]
    fn prop_vp_zero_is_maximum(p in 2u32..10) {
        let valuation = vp(0, p);
        prop_assert_eq!(valuation, u32::MAX);
    }

    /// Property: vp(p^k) = k for small powers
    #[test]
    fn prop_vp_powers_of_p(p in 2u32..5, k in 1u32..5) {
        let value = p.pow(k);
        let valuation = vp(value as u64, p);
        prop_assert_eq!(valuation, k, "vp({}) = {} should equal {}", value, valuation, k);
    }

    /// Property: vp_diff is symmetric
    #[test]
    fn prop_vp_diff_symmetric(a in small_qp_digits(), b in small_qp_digits()) {
        // Only test if they have the same base p
        if a.p == b.p {
            let diff_ab = vp_diff(&a, &b);
            let diff_ba = vp_diff(&b, &a);
            prop_assert_eq!(diff_ab, diff_ba, "vp_diff should be symmetric");
        }
    }

    /// Property: vp_diff(x, x) = precision (maximum similarity)
    #[test]
    fn prop_vp_diff_self_is_maximum(digits in arbitrary_qp_digits()) {
        let diff = vp_diff(&digits, &digits);
        prop_assert_eq!(diff, digits.digits.len() as u32, "vp_diff(x, x) should equal precision");
    }

    /// Property: p-adic distance is symmetric
    #[test]
    fn prop_padic_distance_symmetric(a in small_qp_digits(), b in small_qp_digits()) {
        if a.p == b.p {
            let dist_ab = padic_distance(&a, &b);
            let dist_ba = padic_distance(&b, &a);
            prop_assert!((dist_ab - dist_ba).abs() < 1e-10, "p-adic distance should be symmetric");
        }
    }

    /// Property: p-adic distance(x, x) = 0
    #[test]
    fn prop_padic_distance_self_is_zero(digits in arbitrary_qp_digits()) {
        let distance = padic_distance(&digits, &digits);
        prop_assert!(distance < 1e-10, "p-adic distance to self should be 0");
    }

    /// Property: p-adic distance is non-negative
    #[test]
    fn prop_padic_distance_non_negative(a in small_qp_digits(), b in small_qp_digits()) {
        if a.p == b.p {
            let distance = padic_distance(&a, &b);
            prop_assert!(distance >= 0.0, "p-adic distance should be non-negative");
        }
    }

    /// Property: Proximity score is bounded between 0 and 1
    #[test]
    fn prop_proximity_score_bounded(a in small_qp_digits(), b in small_qp_digits()) {
        if a.p == b.p {
            let score = proximity_score(&a, &b, a.p);
            prop_assert!((0.0..=1.0).contains(&score), "Proximity score should be in [0, 1], got {}", score);
        }
    }

    /// Property: Ball ID should be consistent
    #[test]
    fn prop_ball_id_consistent(digits in arbitrary_qp_digits(), radius in 0usize..10) {
        let radius = std::cmp::min(radius, digits.digits.len());
        let ball1 = ball_id(&digits, radius);
        let ball2 = ball_id(&digits, radius);
        prop_assert_eq!(ball1, ball2, "Ball ID should be consistent");
    }

    /// Property: Smaller radius gives shorter or equal ball ID
    #[test]
    fn prop_ball_id_radius_monotonic(digits in arbitrary_qp_digits(), r1 in 0usize..5, r2 in 0usize..5) {
        let r1 = std::cmp::min(r1, digits.digits.len());
        let r2 = std::cmp::min(r2, digits.digits.len());

        if r1 <= r2 {
            let ball1 = ball_id(&digits, r1);
            let ball2 = ball_id(&digits, r2);
            prop_assert!(ball1.len() <= ball2.len(), "Smaller radius should give shorter or equal ball ID");
        }
    }

    /// Property: Count distinct balls should be <= number of points
    #[test]
    fn prop_count_distinct_balls_bounded(
        points in prop::collection::vec(small_qp_digits(), 1..20),
        radius in 0usize..5
    ) {
        // Filter to ensure all points have the same base p
        if let Some(first_p) = points.first().map(|p| p.p) {
            let same_p_points: Vec<_> = points.into_iter().filter(|p| p.p == first_p).collect();
            if !same_p_points.is_empty() {
                let radius = std::cmp::min(radius, same_p_points[0].digits.len());
                let distinct_count = count_distinct_balls(&same_p_points, radius);
                prop_assert!(distinct_count <= same_p_points.len(),
                    "Distinct ball count {} should be <= point count {}",
                    distinct_count, same_p_points.len());
            }
        }
    }

    /// Property: All identical points should have count = 1
    #[test]
    fn prop_identical_points_one_ball(digits in arbitrary_qp_digits(), count in 1usize..10, radius in 0usize..5) {
        let radius = std::cmp::min(radius, digits.digits.len());
        let points = vec![digits; count];
        let distinct_count = count_distinct_balls(&points, radius);
        prop_assert_eq!(distinct_count, 1, "Identical points should form exactly one ball");
    }

    /// Property: Triangle inequality for p-adic distance
    #[test]
    fn prop_padic_triangle_inequality(
        p in 2u32..5,
        precision in 2usize..10,
        val_a in 0u64..100,
        val_b in 0u64..100,
        val_c in 0u64..100
    ) {
        // Create QpDigits with same p and precision
        let a = QpDigits::from_u64(val_a, p, precision);
        let b = QpDigits::from_u64(val_b, p, precision);
        let c = QpDigits::from_u64(val_c, p, precision);

        if true {
            let dist_ac = padic_distance(&a, &c);
            let dist_ab = padic_distance(&a, &b);
            let dist_bc = padic_distance(&b, &c);

            // In p-adic metrics, we have the strong triangle inequality:
            // d(a,c) <= max(d(a,b), d(b,c))
            let max_dist = dist_ab.max(dist_bc);
            // Use a larger epsilon for floating-point comparison
            let epsilon = 1e-9 * max_dist.max(1.0);
            prop_assert!(dist_ac <= max_dist + epsilon,
                "Strong triangle inequality failed: d({},{})={} > max(d({},{})={}, d({},{})={}) + eps",
                val_a, val_c, dist_ac,
                val_a, val_b, dist_ab,
                val_b, val_c, dist_bc);
        }
    }

    /// Property: Ultrametric inequality (stronger than triangle inequality)
    #[test]
    fn prop_padic_ultrametric_inequality(
        p in 2u32..5,
        precision in 2usize..10,
        val_a in 0u64..100,
        val_b in 0u64..100,
        val_c in 0u64..100
    ) {
        // Create QpDigits with same p and precision
        let a = QpDigits::from_u64(val_a, p, precision);
        let b = QpDigits::from_u64(val_b, p, precision);
        let c = QpDigits::from_u64(val_c, p, precision);

        if true {
            let dist_ac = padic_distance(&a, &c);
            let dist_ab = padic_distance(&a, &b);
            let dist_bc = padic_distance(&b, &c);

            // Ultrametric: d(a,c) <= max(d(a,b), d(b,c))
            let max_dist = dist_ab.max(dist_bc);
            // Use a larger epsilon for floating-point comparison
            let epsilon = 1e-9 * max_dist.max(1.0);
            prop_assert!(dist_ac <= max_dist + epsilon,
                "Ultrametric inequality failed: d(a,c) = {} > max({}, {}) = {} + eps",
                dist_ac, dist_ab, dist_bc, max_dist);
        }
    }

    /// Property: vp_diff should be <= max(precision_a, precision_b)
    #[test]
    fn prop_vp_diff_bounded_by_precision(a in arbitrary_qp_digits(), b in arbitrary_qp_digits()) {
        if a.p == b.p {
            let diff = vp_diff(&a, &b);
            let max_precision = a.digits.len().max(b.digits.len()) as u32;
            prop_assert!(diff <= max_precision,
                "vp_diff {} should be <= max precision {}", diff, max_precision);
        }
    }

    /// Property: Larger vp_diff should give smaller p-adic distance
    #[test]
    fn prop_vp_diff_distance_inverse_relation(
        a in small_qp_digits(),
        b in small_qp_digits(),
        c in small_qp_digits()
    ) {
        if a.p == b.p && a.p == c.p {
            // Skip the test when any two values are identical (distance is 0)
            // as the inverse relation doesn't apply when distance is exactly 0
            if a.digits == b.digits || a.digits == c.digits || b.digits == c.digits {
                return Ok(());
            }

            let vp_ab = vp_diff(&a, &b);
            let vp_ac = vp_diff(&a, &c);
            let dist_ab = padic_distance(&a, &b);
            let dist_ac = padic_distance(&a, &c);

            // If vp_diff is larger, distance should be smaller (or equal due to precision limits)
            if vp_ab > vp_ac && vp_ac < 100 {  // Skip when vp_ac is effectively infinite
                prop_assert!(dist_ab <= dist_ac + 1e-10,
                    "Larger vp_diff should give smaller distance: vp_diff(a,b)={} > vp_diff(a,c)={}, but dist(a,b)={} > dist(a,c)={}",
                    vp_ab, vp_ac, dist_ab, dist_ac);
            }
        }
    }

    /// Property: Ball distinctness is consistent for point sets
    #[test]
    fn prop_balls_are_distinct_consistent(
        a in small_qp_digits(),
        b in small_qp_digits(),
        radius in 0usize..5
    ) {
        if a.p == b.p {
            let radius = std::cmp::min(radius, a.digits.len().min(b.digits.len()));
            let points_ab = vec![a.clone(), b.clone()];
            let points_ba = vec![b, a];
            let distinct1 = balls_are_distinct(&points_ab, radius, 2);
            let distinct2 = balls_are_distinct(&points_ba, radius, 2);
            prop_assert_eq!(distinct1, distinct2, "Ball distinctness should be consistent for same point set");
        }
    }

    /// Property: Identical points don't form distinct balls
    #[test]
    fn prop_identical_balls_not_distinct(digits in arbitrary_qp_digits(), count in 2usize..10, radius in 0usize..10) {
        let radius = std::cmp::min(radius, digits.digits.len());
        let points = vec![digits; count];
        let distinct = balls_are_distinct(&points, radius, 2);
        prop_assert!(!distinct, "Identical points should not form 2 distinct balls");
    }

    /// Property: p-adic valuation of the same value should be consistent
    #[test]
    fn prop_consistent_valuation(
        value in 1u64..100,
        p in 2u32..5
    ) {
        let vp_val = vp(value, p);

        // vp should be consistent for the same input
        prop_assert_eq!(vp(value, p), vp_val, "vp should be consistent for same input");

        // If value is divisible by p, vp should be > 0
        if value % (p as u64) == 0 {
            prop_assert!(vp_val > 0, "vp({}, {}) should be > 0 when {} is divisible by {}", value, p, value, p);
        }
    }
}

// Additional deterministic tests for specific mathematical properties

#[test]
fn test_mathematical_invariant_zero_distance() {
    // Test that distance to self is always zero
    for value in [0, 1, 9, 27, 81] {
        for p in [2, 3, 5] {
            let digits = QpDigits::from_u64(value, p, 10);
            let distance = padic_distance(&digits, &digits);
            assert!(
                distance.abs() < 1e-10,
                "Distance to self should be 0 for value {}, p {}",
                value,
                p
            );
        }
    }
}

#[test]
fn test_mathematical_invariant_powers_of_p() {
    // Test specific known values
    let p = 3;

    // 3^0 = 1 should have vp = 0
    assert_eq!(vp(1, p), 0, "vp(1) should be 0");

    // 3^1 = 3 should have vp = 1
    assert_eq!(vp(3, p), 1, "vp(3) should be 1");

    // 3^2 = 9 should have vp = 2
    assert_eq!(vp(9, p), 2, "vp(9) should be 2");

    // 3^3 = 27 should have vp = 3
    assert_eq!(vp(27, p), 3, "vp(27) should be 3");
}

#[test]
fn test_mathematical_invariant_ball_containment() {
    // Test that smaller radius balls are contained in larger radius balls
    let value = 123;
    let p = 3;
    let precision = 10;
    let digits = QpDigits::from_u64(value, p, precision);

    for r1 in 0..5 {
        for r2 in r1..5 {
            let ball1 = ball_id(&digits, r1);
            let ball2 = ball_id(&digits, r2);

            // ball1 should be a prefix of ball2 (or equal length if at precision limit)
            assert!(
                ball1.len() <= ball2.len(),
                "Smaller radius {} should give shorter or equal ball than radius {}",
                r1,
                r2
            );

            if ball1.len() <= ball2.len() {
                assert_eq!(
                    &ball2[..ball1.len()],
                    &ball1[..],
                    "Smaller radius ball should be prefix of larger radius ball"
                );
            }
        }
    }
}

#[test]
fn test_mathematical_invariant_distinct_counting() {
    // Test that count_distinct_balls gives expected results for known cases
    let p = 3;
    let precision = 5;

    // Create points that should be in different balls
    let points = vec![
        QpDigits::from_u64(1, p, precision), // [1, 0, 0, 0, 0]
        QpDigits::from_u64(2, p, precision), // [2, 0, 0, 0, 0]
        QpDigits::from_u64(3, p, precision), // [0, 1, 0, 0, 0]
        QpDigits::from_u64(4, p, precision), // [1, 1, 0, 0, 0]
    ];

    // At radius 1, first digit distinguishes 1,2 from 3,4
    let count_r1 = count_distinct_balls(&points, 1);
    assert!(
        count_r1 >= 2,
        "Should have at least 2 distinct balls at radius 1"
    );

    // At radius 0, all should be in the same "ball" (no distinguishing digits)
    let count_r0 = count_distinct_balls(&points, 0);
    assert_eq!(count_r0, 1, "Should have exactly 1 ball at radius 0");

    // Total distinct count should be <= number of points
    assert!(
        count_r1 <= points.len(),
        "Distinct count should be <= number of points"
    );
}
