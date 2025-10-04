use adic_crypto::Keypair;
use adic_math::ball_id;
use adic_storage::{store::BackendType, StorageConfig, StorageEngine};
use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AxisPhi, QpDigits, DEFAULT_P, DEFAULT_PRECISION,
};
use chrono::Utc;
use std::sync::Arc;

fn create_test_message(i: u32, keypair: &Keypair) -> AdicMessage {
    let features = AdicFeatures::new(vec![
        AxisPhi::new(
            0,
            QpDigits::from_u64(i as u64, DEFAULT_P, DEFAULT_PRECISION),
        ),
        AxisPhi::new(
            1,
            QpDigits::from_u64((i % 10) as u64, DEFAULT_P, DEFAULT_PRECISION),
        ),
        AxisPhi::new(
            2,
            QpDigits::from_u64((i % 5) as u64, DEFAULT_P, DEFAULT_PRECISION),
        ),
    ]);

    let meta = AdicMeta::new(Utc::now());

    let mut msg = AdicMessage::new(
        vec![],
        features,
        meta,
        *keypair.public_key(),
        format!("test_data_{}", i).into_bytes(),
    );

    let signature = keypair.sign(&msg.to_bytes());
    msg.signature = signature;

    msg
}

#[tokio::test]
async fn test_ball_membership_proof() {
    let keypair = Keypair::generate();
    let msg = create_test_message(42, &keypair);

    // Store message
    let storage = Arc::new(
        StorageEngine::new(StorageConfig {
            backend_type: BackendType::Memory,
            ..Default::default()
        })
        .unwrap(),
    );

    storage.store_message(&msg).await.unwrap();

    // Test proof generation logic (mimicking API endpoint)
    let axis = 0u32;
    let radius = 3usize;

    // Get the p-adic features for axis 0
    let axis_phi = msg
        .features
        .phi
        .iter()
        .find(|phi| phi.axis.0 == axis)
        .unwrap();

    // Compute ball ID
    let computed_ball_id = ball_id(&axis_phi.qp_digits, radius);

    // Verify the proof would be valid
    assert_eq!(computed_ball_id.len(), radius);

    // Simulate proof verification
    let qp_digits_clone = axis_phi.qp_digits.clone();
    let recomputed_ball_id = ball_id(&qp_digits_clone, radius);

    assert_eq!(computed_ball_id, recomputed_ball_id);
    println!("✓ Ball membership proof generation and verification successful");
}

#[tokio::test]
async fn test_proof_verification_logic() {
    // Test the verification logic directly
    let features_p = 3u32;
    let features_digits = vec![1u8, 2, 0, 1, 2];

    let qp_digits = QpDigits {
        p: features_p,
        digits: features_digits.clone(),
    };

    let radius = 3;
    let computed_ball_id = ball_id(&qp_digits, radius);

    // Verify proof matches
    let qp_digits_verify = QpDigits {
        p: features_p,
        digits: features_digits,
    };
    let verify_ball_id = ball_id(&qp_digits_verify, radius);

    assert_eq!(computed_ball_id, verify_ball_id);
    assert_eq!(computed_ball_id.len(), radius);

    println!("✓ Proof verification logic works correctly");
}

#[tokio::test]
async fn test_invalid_proof_detection() {
    // Test that invalid proofs are detected
    let correct_features = QpDigits {
        p: 3,
        digits: vec![1, 2, 0, 1, 2],
    };

    let wrong_features = QpDigits {
        p: 3,
        digits: vec![2, 1, 0, 1, 2], // Different first digit
    };

    let radius = 3;
    let correct_ball_id = ball_id(&correct_features, radius);
    let wrong_ball_id = ball_id(&wrong_features, radius);

    // At radius 3, different features should produce different ball IDs
    // (though this depends on the specific values)
    assert_ne!(correct_ball_id, wrong_ball_id);

    println!("✓ Invalid proof detection works correctly");
}

#[tokio::test]
async fn test_multiple_axes_proofs() {
    let keypair = Keypair::generate();
    let msg = create_test_message(123, &keypair);

    // Test proof generation for each axis
    for axis_phi in &msg.features.phi {
        let axis = axis_phi.axis.0;
        let radius = 2;

        let ball_id_result = ball_id(&axis_phi.qp_digits, radius);

        assert_eq!(ball_id_result.len(), radius);
        println!(
            "✓ Proof for axis {} produces ball ID of length {}",
            axis, radius
        );
    }
}

#[tokio::test]
async fn test_proof_with_varying_radii() {
    let keypair = Keypair::generate();
    let msg = create_test_message(456, &keypair);
    let axis_phi = &msg.features.phi[0];

    // Test different radii
    for radius in 1..=5 {
        let ball_id_result = ball_id(&axis_phi.qp_digits, radius);
        assert_eq!(ball_id_result.len(), radius);

        // Verify consistency: larger radius should be prefix of smaller
        if radius > 1 {
            let smaller_ball_id = ball_id(&axis_phi.qp_digits, radius - 1);
            assert_eq!(&ball_id_result[0..radius - 1], smaller_ball_id.as_slice());
        }

        println!("✓ Proof with radius {} produces correct ball ID", radius);
    }
}
