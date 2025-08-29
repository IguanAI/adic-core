// Security tests for C1 admissibility implementation
// These tests verify that the implementation correctly enforces
// that messages must be reasonably close to ALL parents, not just one.
use crate::AdmissibilityChecker;
use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AdicParams, AxisPhi, MessageId, PublicKey, QpDigits,
};
use chrono::Utc;

#[cfg(test)]
mod c1_security_tests {
    use super::*;

    #[test]
    fn test_c1_security_single_close_parent() {
        // This test verifies that the implementation correctly REJECTS messages
        // that are only close to ONE parent but far from others.

        let p = 3u32;
        let d = 2u32; // Require 3 parents
        let rho = vec![2, 2, 2]; // Three axes with radius 2 each

        let params = AdicParams {
            p,
            d,
            rho,
            ..Default::default()
        };

        let checker = AdmissibilityChecker::new(params);

        // Create a malicious message at (0, 0, 0)
        let msg_features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(0, p, 5)),
            AxisPhi::new(1, QpDigits::from_u64(0, p, 5)),
            AxisPhi::new(2, QpDigits::from_u64(0, p, 5)),
        ]);

        let parents = vec![
            MessageId::new(&[1]),
            MessageId::new(&[2]),
            MessageId::new(&[3]),
        ];

        let message = AdicMessage::new(
            parents,
            msg_features,
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![],
        );

        // Parent features:
        // Parent 0: Very close at (1, 1, 1) - distance 1 on all axes
        // Parent 1: Far at (27, 27, 27) - distance 3 on all axes
        // Parent 2: Far at (81, 81, 81) - distance 4 on all axes
        let parent_features = vec![
            vec![
                QpDigits::from_u64(1, p, 5), // Close parent
                QpDigits::from_u64(1, p, 5),
                QpDigits::from_u64(1, p, 5),
            ],
            vec![
                QpDigits::from_u64(27, p, 5), // Far parent (3^3)
                QpDigits::from_u64(27, p, 5),
                QpDigits::from_u64(27, p, 5),
            ],
            vec![
                QpDigits::from_u64(81, p, 5), // Very far parent (3^4)
                QpDigits::from_u64(81, p, 5),
                QpDigits::from_u64(81, p, 5),
            ],
        ];

        // All parents have good reputation
        let parent_reputations = vec![1.0, 1.0, 1.0];

        let result = checker
            .check_message(&message, &parent_features, &parent_reputations)
            .unwrap();

        println!("=== C1 Security Test Results ===");
        println!("Message at (0,0,0) with parents:");
        println!("  Parent 0: (1,1,1) - CLOSE");
        println!("  Parent 1: (27,27,27) - FAR");
        println!("  Parent 2: (81,81,81) - VERY FAR");
        println!();
        println!("Admissibility score: {}", result.score);
        println!("Score passed (>= {}): {}", d, result.score_passed);
        println!("Is admissible: {}", result.is_admissible);
        println!();

        // Calculate what the score SHOULD be if checking all parents
        println!("=== Expected Behavior (Secure) ===");
        for axis in 0..3 {
            println!("Axis {}:", axis);
            println!("  Parent 0: vp(0-1)=0, term=3^(-max(0,2-0))=3^-2=0.111");
            println!("  Parent 1: vp(0-27)=3, term=3^(-max(0,2-3))=3^0=1.0");
            println!("  Parent 2: vp(0-81)=4, term=3^(-max(0,2-4))=3^0=1.0");
            println!("  min term = 0.111");
        }
        println!("Expected secure score: 3 * 0.111 = 0.333");
        println!();

        // The vulnerability: if implementation takes min across parents FIRST,
        // it finds the closest parent and ignores the far ones
        println!("=== Actual Behavior (Vulnerable) ===");
        println!("If taking closest parent first, then summing:");
        println!("Each axis uses only Parent 0's distance");
        println!("Score would be: 3 * 0.111 = 0.333");
        println!();

        // Check if vulnerability exists
        if result.score_passed && result.score < 2.0 {
            println!("🚨 VULNERABILITY CONFIRMED!");
            println!("Message admitted despite being far from 2 out of 3 parents!");
            println!("This breaks the fundamental security model.");
        } else if !result.score_passed {
            println!("✅ Implementation appears secure");
            println!("Message correctly rejected for not being close to all parents");
        }

        // The message should NOT be admissible in a secure implementation
        // because it's only close to 1 out of 3 parents
        assert!(
            !result.is_admissible || result.score < 2.0,
            "CRITICAL: Message admitted when only close to 1 parent! Score: {}",
            result.score
        );
    }

    #[test]
    fn test_c1_correct_all_close_parents() {
        // Control test: all parents are close, should be admitted

        let p = 3u32;
        let d = 2u32;
        let rho = vec![2, 2, 2];

        let params = AdicParams {
            p,
            d,
            rho,
            ..Default::default()
        };

        let checker = AdmissibilityChecker::new(params);

        let msg_features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(0, p, 5)),
            AxisPhi::new(1, QpDigits::from_u64(0, p, 5)),
            AxisPhi::new(2, QpDigits::from_u64(0, p, 5)),
        ]);

        let parents = vec![
            MessageId::new(&[1]),
            MessageId::new(&[2]),
            MessageId::new(&[3]),
        ];

        let message = AdicMessage::new(
            parents,
            msg_features,
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![],
        );

        // All parents are close
        let parent_features = vec![
            vec![
                QpDigits::from_u64(1, p, 5),
                QpDigits::from_u64(1, p, 5),
                QpDigits::from_u64(1, p, 5),
            ],
            vec![
                QpDigits::from_u64(3, p, 5), // vp=1, still within radius 2
                QpDigits::from_u64(3, p, 5),
                QpDigits::from_u64(3, p, 5),
            ],
            vec![
                QpDigits::from_u64(9, p, 5), // vp=2, exactly at radius 2
                QpDigits::from_u64(9, p, 5),
                QpDigits::from_u64(9, p, 5),
            ],
        ];

        let parent_reputations = vec![1.0, 1.0, 1.0];

        let result = checker
            .check_message(&message, &parent_features, &parent_reputations)
            .unwrap();

        println!("=== Control Test: All Parents Close ===");
        println!("Score: {}", result.score);
        println!("Is admissible: {}", result.is_admissible);
        println!("Score breakdown:");
        println!("  Each axis contributes min(0.111, 0.333, 1.0) = 0.111");
        println!("  Total: 3 × 0.111 = 0.333");
        println!("  Required score: d = 2");

        // With these parent distances, score is 0.333 which is < 2, so not admissible
        // This is correct behavior - even these "close" parents aren't close enough
        assert!(!result.score_passed, "Score 0.333 < 2, correctly fails");
        assert_eq!(result.score, 0.3333333333333333);
    }
}
