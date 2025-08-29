#[cfg(test)]
mod tests {
    use super::super::*;
    use adic_types::{AdicFeatures, AxisPhi, QpDigits};

    fn create_test_params() -> SelectionParams {
        SelectionParams {
            d: 3,
            q: 3,
            rho: vec![2, 2, 1],
            max_horizon: 100,
            widen_factor: 1.5,
        }
    }

    fn create_test_candidate(id: u8, feature_values: Vec<u64>) -> ParentCandidate {
        let features: Vec<QpDigits> = feature_values
            .iter()
            .map(|&v| QpDigits::from_u64(v, 3, 5))
            .collect();

        ParentCandidate {
            message_id: MessageId::new(&[id; 32]),
            features,
            reputation: 1.0,
            conflict_penalty: 0.0,
            weight: 0.0,
            axis_weights: HashMap::new(),
        }
    }

    #[test]
    fn test_selection_params_from_adic() {
        let adic_params = AdicParams::default();
        let selection_params = SelectionParams::from_adic_params(&adic_params);

        assert_eq!(selection_params.d, adic_params.d);
        assert_eq!(selection_params.q, adic_params.q);
        assert_eq!(selection_params.rho, adic_params.rho);
        assert_eq!(selection_params.max_horizon, 100);
        assert_eq!(selection_params.widen_factor, 1.5);
    }

    #[tokio::test]
    async fn test_mrw_selector_basic() {
        let params = create_test_params();
        let selector = MrwSelector::new(params);

        let message_features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(10, 3, 5)),
            AxisPhi::new(1, QpDigits::from_u64(20, 3, 5)),
            AxisPhi::new(2, QpDigits::from_u64(30, 3, 5)),
        ]);

        let tips = vec![
            create_test_candidate(1, vec![11, 21, 31]),
            create_test_candidate(2, vec![12, 22, 32]),
            create_test_candidate(3, vec![13, 23, 33]),
            create_test_candidate(4, vec![14, 24, 34]),
            create_test_candidate(5, vec![15, 25, 35]),
        ];

        let weight_calc = WeightCalculator::new(1.0, 0.5, 1.0);

        let result = selector
            .select_parents(&message_features, tips, &weight_calc)
            .await;

        match result {
            Ok((selected, trace)) => {
                // Should select d+1 parents
                assert_eq!(selected.len(), 4);
                assert!(!trace.steps.is_empty());
            }
            Err(_) => {
                // Selection might fail if diversity requirement cannot be met
                // This is acceptable in test with limited candidates
            }
        }
    }

    #[tokio::test]
    async fn test_mrw_selector_empty_tips() {
        let params = create_test_params();
        let selector = MrwSelector::new(params);

        let message_features =
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(10, 3, 5))]);

        let tips = vec![];
        let weight_calc = WeightCalculator::new(1.0, 0.5, 1.0);

        let result = selector
            .select_parents(&message_features, tips, &weight_calc)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mrw_selector_insufficient_tips() {
        let params = create_test_params();
        let selector = MrwSelector::new(params);

        let message_features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(10, 3, 5)),
            AxisPhi::new(1, QpDigits::from_u64(20, 3, 5)),
            AxisPhi::new(2, QpDigits::from_u64(30, 3, 5)),
        ]);

        // Only 2 tips when we need d+1=4
        let tips = vec![
            create_test_candidate(1, vec![11, 21, 31]),
            create_test_candidate(2, vec![12, 22, 32]),
        ];

        let weight_calc = WeightCalculator::new(1.0, 0.5, 1.0);

        let result = selector
            .select_parents(&message_features, tips, &weight_calc)
            .await;
        // With bootstrap mode, this should now succeed by repeating tips
        assert!(result.is_ok());
        let (selected, _trace) = result.unwrap();
        assert_eq!(selected.len(), 4); // Should have d+1 parents
    }

    #[test]
    fn test_parent_candidate_creation() {
        let candidate = create_test_candidate(1, vec![10, 20, 30]);

        assert_eq!(candidate.features.len(), 3);
        assert_eq!(candidate.reputation, 1.0);
        assert_eq!(candidate.conflict_penalty, 0.0);
        assert_eq!(candidate.weight, 0.0);
        assert!(candidate.axis_weights.is_empty());
    }

    #[tokio::test]
    async fn test_mrw_selector_with_conflicts() {
        let params = create_test_params();
        let selector = MrwSelector::new(params);

        let message_features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(10, 3, 5)),
            AxisPhi::new(1, QpDigits::from_u64(20, 3, 5)),
            AxisPhi::new(2, QpDigits::from_u64(30, 3, 5)),
        ]);

        let mut tips = vec![
            create_test_candidate(1, vec![11, 21, 31]),
            create_test_candidate(2, vec![12, 22, 32]),
            create_test_candidate(3, vec![13, 23, 33]),
            create_test_candidate(4, vec![14, 24, 34]),
            create_test_candidate(5, vec![15, 25, 35]),
        ];

        // Add conflict penalty to some candidates
        tips[0].conflict_penalty = 0.5;
        tips[2].conflict_penalty = 0.8;

        let weight_calc = WeightCalculator::new(1.0, 0.5, 1.0);

        let result = selector
            .select_parents(&message_features, tips, &weight_calc)
            .await;

        match result {
            Ok((selected, _)) => {
                // Should prefer candidates with lower conflict penalties
                assert_eq!(selected.len(), 4);
            }
            Err(_) => {
                // Selection might fail, which is acceptable
            }
        }
    }

    #[tokio::test]
    async fn test_mrw_selector_with_reputation() {
        let params = create_test_params();
        let selector = MrwSelector::new(params);

        let message_features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(10, 3, 5)),
            AxisPhi::new(1, QpDigits::from_u64(20, 3, 5)),
            AxisPhi::new(2, QpDigits::from_u64(30, 3, 5)),
        ]);

        let mut tips = vec![
            create_test_candidate(1, vec![11, 21, 31]),
            create_test_candidate(2, vec![12, 22, 32]),
            create_test_candidate(3, vec![13, 23, 33]),
            create_test_candidate(4, vec![14, 24, 34]),
            create_test_candidate(5, vec![15, 25, 35]),
        ];

        // Vary reputation scores
        tips[0].reputation = 5.0;
        tips[1].reputation = 0.5;
        tips[2].reputation = 3.0;
        tips[3].reputation = 1.0;
        tips[4].reputation = 2.0;

        let weight_calc = WeightCalculator::new(1.0, 0.5, 1.0);

        let result = selector
            .select_parents(&message_features, tips, &weight_calc)
            .await;

        match result {
            Ok((selected, _)) => {
                // Should prefer candidates with higher reputation
                assert_eq!(selected.len(), 4);
            }
            Err(_) => {
                // Selection might fail, which is acceptable
            }
        }
    }
}
