use adic_types::{AdicFeatures, AxisId, AxisPhi, QpDigits};

#[test]
fn test_qp_digits_creation() {
    let digits = QpDigits {
        p: 3,
        digits: vec![0, 1, 2, 1, 0],
    };

    assert_eq!(digits.p, 3);
    assert_eq!(digits.digits.len(), 5);
}

#[test]
fn test_qp_digits_validation() {
    // Valid digits for p=3 (should be 0, 1, or 2)
    let valid = QpDigits {
        p: 3,
        digits: vec![0, 1, 2, 0],
    };
    assert!(valid.digits.iter().all(|&d| d < 3));

    // Test with p=5
    let valid_p5 = QpDigits {
        p: 5,
        digits: vec![0, 4, 3],
    };
    assert!(valid_p5.digits.iter().all(|&d| d < 5));

    // Test with p=2 (binary)
    let binary = QpDigits {
        p: 2,
        digits: vec![1, 0, 1, 1, 0, 0, 1, 0],
    };
    assert!(binary.digits.iter().all(|&d| d < 2));
}

#[test]
fn test_qp_digits_edge_cases() {
    // Empty digits
    let empty = QpDigits {
        p: 3,
        digits: vec![],
    };
    assert_eq!(empty.digits.len(), 0);

    // Single digit
    let single = QpDigits {
        p: 7,
        digits: vec![5],
    };
    assert_eq!(single.digits.len(), 1);

    // Large precision
    let large_precision = QpDigits {
        p: 3,
        digits: vec![0; 1000],
    };
    assert_eq!(large_precision.digits.len(), 1000);

    // Maximum values
    let max_values = QpDigits {
        p: 11,
        digits: vec![10, 10, 10, 10, 10],
    };
    assert!(max_values.digits.iter().all(|&d| d < 11));
}

#[test]
fn test_qp_digits_serialization() {
    let original = QpDigits {
        p: 5,
        digits: vec![4, 2, 0, 3, 1, 4],
    };

    let serialized = serde_json::to_string(&original).unwrap();
    let deserialized: QpDigits = serde_json::from_str(&serialized).unwrap();

    assert_eq!(original.p, deserialized.p);
    assert_eq!(original.digits, deserialized.digits);
}

#[test]
fn test_qp_digits_methods() {
    use num_bigint::BigUint;

    // Test from_u64
    let qp = QpDigits::from_u64(27, 3, 5); // 27 in base 3 is 1000
    assert_eq!(qp.p, 3);
    assert_eq!(qp.digits.len(), 5);

    // Test new with BigUint
    let big_val = BigUint::from(100u32);
    let qp_big = QpDigits::new(&big_val, 3, 8);
    assert_eq!(qp_big.p, 3);
    assert_eq!(qp_big.digits.len(), 8);

    // Test ball_id
    let qp2 = QpDigits {
        p: 3,
        digits: vec![2, 1, 0, 1, 2],
    };
    let ball = qp2.ball_id(3);
    assert_eq!(ball, vec![2, 1, 0]);

    // Test is_zero
    let zero = QpDigits {
        p: 3,
        digits: vec![0, 0, 0, 0],
    };
    assert!(zero.is_zero());

    let non_zero = QpDigits {
        p: 3,
        digits: vec![0, 1, 0, 0],
    };
    assert!(!non_zero.is_zero());
}

#[test]
fn test_axis_phi() {
    let axis_phi = AxisPhi::new(
        42,
        QpDigits {
            p: 3,
            digits: vec![2, 1, 0, 2],
        },
    );

    assert_eq!(axis_phi.axis.0, 42);
    assert_eq!(axis_phi.qp_digits.p, 3);
    assert_eq!(axis_phi.qp_digits.digits, vec![2, 1, 0, 2]);

    // Test serialization
    let serialized = serde_json::to_string(&axis_phi).unwrap();
    let deserialized: AxisPhi = serde_json::from_str(&serialized).unwrap();

    assert_eq!(axis_phi.axis, deserialized.axis);
    assert_eq!(axis_phi.qp_digits.p, deserialized.qp_digits.p);
    assert_eq!(axis_phi.qp_digits.digits, deserialized.qp_digits.digits);
}

#[test]
fn test_adic_features_creation() {
    let features = AdicFeatures::new(vec![]);

    assert!(features.phi.is_empty());
    assert_eq!(features.dimension(), 0);

    // Should serialize even when empty
    let serialized = serde_json::to_string(&features).unwrap();
    assert!(serialized.contains("phi"));
}

#[test]
fn test_adic_features_with_multiple_axes() {
    let mut phi_vec = Vec::new();

    // Add multiple axis phi values
    for i in 0..5 {
        phi_vec.push(AxisPhi::new(
            i,
            QpDigits {
                p: 3,
                digits: vec![i as u8 % 3, (i as u8 + 1) % 3, (i as u8 + 2) % 3],
            },
        ));
    }

    let features = AdicFeatures::new(phi_vec);

    assert_eq!(features.phi.len(), 5);
    assert_eq!(features.dimension(), 5);

    // Check each axis
    for (idx, axis_phi) in features.phi.iter().enumerate() {
        assert_eq!(axis_phi.axis.0, idx as u32);
        assert_eq!(axis_phi.qp_digits.p, 3);
        assert_eq!(axis_phi.qp_digits.digits.len(), 3);
    }

    // Test serialization with multiple axes
    let serialized = serde_json::to_string(&features).unwrap();
    let deserialized: AdicFeatures = serde_json::from_str(&serialized).unwrap();

    assert_eq!(features.phi.len(), deserialized.phi.len());
    for i in 0..5 {
        assert_eq!(features.phi[i].axis, deserialized.phi[i].axis);
        assert_eq!(
            features.phi[i].qp_digits.digits,
            deserialized.phi[i].qp_digits.digits
        );
    }
}

#[test]
fn test_adic_features_get_axis() {
    let features = AdicFeatures::new(vec![
        AxisPhi::new(
            0,
            QpDigits {
                p: 3,
                digits: vec![1, 2],
            },
        ),
        AxisPhi::new(
            2,
            QpDigits {
                p: 3,
                digits: vec![2, 1],
            },
        ),
        AxisPhi::new(
            5,
            QpDigits {
                p: 3,
                digits: vec![0, 1],
            },
        ),
    ]);

    // Get existing axis
    let axis0 = features.get_axis(AxisId(0));
    assert!(axis0.is_some());
    assert_eq!(axis0.unwrap().axis.0, 0);

    let axis2 = features.get_axis(AxisId(2));
    assert!(axis2.is_some());
    assert_eq!(axis2.unwrap().axis.0, 2);

    // Get non-existing axis
    let axis1 = features.get_axis(AxisId(1));
    assert!(axis1.is_none());

    let axis10 = features.get_axis(AxisId(10));
    assert!(axis10.is_none());
}

#[test]
fn test_adic_features_equality() {
    let features1 = AdicFeatures::new(vec![AxisPhi::new(
        1,
        QpDigits {
            p: 3,
            digits: vec![1, 2],
        },
    )]);

    let features2 = features1.clone();
    assert_eq!(features1.phi.len(), features2.phi.len());

    let features3 = AdicFeatures::new(vec![AxisPhi::new(
        2, // Different axis
        QpDigits {
            p: 3,
            digits: vec![1, 2],
        },
    )]);

    assert_ne!(features1.phi[0].axis, features3.phi[0].axis);
}

#[test]
fn test_axis_id_limits() {
    // Test with various axis IDs
    let test_ids: Vec<u32> = vec![0, 1, 255, 65535, u32::MAX];

    for id in test_ids {
        let axis_phi = AxisPhi::new(
            id,
            QpDigits {
                p: 3,
                digits: vec![0],
            },
        );

        assert_eq!(axis_phi.axis.0, id);

        // Should serialize correctly
        let serialized = serde_json::to_string(&axis_phi).unwrap();
        let deserialized: AxisPhi = serde_json::from_str(&serialized).unwrap();
        assert_eq!(axis_phi.axis, deserialized.axis);
    }
}

#[test]
fn test_duplicate_axis_ids() {
    // Test features with duplicate axis IDs (allowed in current implementation)
    let features = AdicFeatures::new(vec![
        AxisPhi::new(
            1,
            QpDigits {
                p: 3,
                digits: vec![0, 1],
            },
        ),
        AxisPhi::new(
            1,
            QpDigits {
                p: 3,
                digits: vec![1, 2],
            },
        ),
        AxisPhi::new(
            1,
            QpDigits {
                p: 3,
                digits: vec![2, 0],
            },
        ),
    ]);

    assert_eq!(features.phi.len(), 3);
    // All should have same axis
    assert!(features.phi.iter().all(|ap| ap.axis.0 == 1));

    // But different digits
    assert_ne!(
        features.phi[0].qp_digits.digits,
        features.phi[1].qp_digits.digits
    );

    // get_axis will return the first match
    let found = features.get_axis(AxisId(1));
    assert!(found.is_some());
    assert_eq!(found.unwrap().qp_digits.digits, vec![0, 1]);
}

// Need to import for QpDigits methods
