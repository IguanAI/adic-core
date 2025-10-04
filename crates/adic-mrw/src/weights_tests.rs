#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn test_weight_calculator_creation() {
        let calc = WeightCalculator::new(1.5, 0.8, 2.0);
        // Test that it was created (we can't access private fields)
        // Test through computation instead
        let weight = calc.compute_weight(1.0, 1.0, 0.0, 0.0);
        assert!(weight > 0.0);
    }

    #[test]
    fn test_compute_weight() {
        let calc = WeightCalculator::new(1.0, 0.5, 1.0);

        let weight = calc.compute_weight(0.5, 2.0, 0.1, 0.0);
        assert!(weight > 0.0);

        // Higher proximity should give higher weight
        let weight_high_prox = calc.compute_weight(1.0, 2.0, 0.1, 0.0);
        assert!(weight_high_prox > weight);

        // Higher reputation should give higher weight
        let weight_high_rep = calc.compute_weight(0.5, 5.0, 0.1, 0.0);
        assert!(weight_high_rep > weight);

        // Higher conflict penalty should give lower weight
        let weight_high_penalty = calc.compute_weight(0.5, 2.0, 1.0, 0.0);
        assert!(weight_high_penalty < weight);
    }

    #[test]
    fn test_compute_trust() {
        let calc = WeightCalculator::new(1.0, 0.5, 1.0);

        let trust1 = calc.compute_trust(1.0);
        let trust2 = calc.compute_trust(2.0);
        let trust3 = calc.compute_trust(0.5);

        // Higher reputation should give higher trust
        assert!(trust2 > trust1);
        assert!(trust1 > trust3);

        // Trust should be reputation^alpha (alpha=1.0 by default)
        assert!((trust1 - 1.0).abs() < 0.001); // 1.0^1.0 = 1.0
        assert!((trust2 - 2.0).abs() < 0.001); // 2.0^1.0 = 2.0
        assert!((trust3 - 0.5).abs() < 0.001); // 0.5^1.0 = 0.5
    }

    #[test]
    fn test_compute_detailed_weights() {
        let calc = WeightCalculator::new(1.0, 0.5, 1.0);

        let weights = calc.compute_detailed_weights(0.5, 2.0, 0.2);

        assert_eq!(weights.proximity, 0.5);
        assert_eq!(weights.trust, 2.0); // reputation^1.0 = 2.0
        assert!(weights.conflict_penalty > 0.0);
        assert!(weights.conflict_penalty <= 1.0);
        assert!(weights.total > 0.0);

        // Per PDF: weight = exp(λ * proximity * trust - μ * conflict)
        // The detailed weights decompose this for debugging
        let combined_weight = (1.0_f64 * 0.5 * 2.0).exp(); // exp(λ * proximity * trust)
        let penalty_weight = (-0.2_f64).exp(); // exp(-μ * conflict)
        let expected_total = combined_weight * penalty_weight;
        assert!((weights.total - expected_total).abs() < 0.001);
    }

    #[test]
    fn test_compute_transition_probability() {
        let calc = WeightCalculator::new(1.0, 0.5, 1.0);

        let prob = calc.compute_transition_probability(3.0, 10.0);
        assert_eq!(prob, 0.3);

        let prob_zero = calc.compute_transition_probability(0.0, 10.0);
        assert_eq!(prob_zero, 0.0);

        let prob_full = calc.compute_transition_probability(10.0, 10.0);
        assert_eq!(prob_full, 1.0);

        // Division by zero case
        let prob_div_zero = calc.compute_transition_probability(1.0, 0.0);
        assert_eq!(prob_div_zero, 0.0);
    }

    #[test]
    fn test_normalize_weights() {
        let calc = WeightCalculator::new(1.0, 0.5, 1.0);

        let mut weights = vec![2.0, 3.0, 5.0];
        calc.normalize_weights(&mut weights);

        let sum: f64 = weights.iter().sum();
        assert!((sum - 1.0).abs() < 0.001);

        assert!((weights[0] - 0.2).abs() < 0.001);
        assert!((weights[1] - 0.3).abs() < 0.001);
        assert!((weights[2] - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_normalize_weights_empty() {
        let calc = WeightCalculator::new(1.0, 0.5, 1.0);

        let mut weights = vec![];
        calc.normalize_weights(&mut weights);
        assert!(weights.is_empty());
    }

    #[test]
    fn test_normalize_weights_single() {
        let calc = WeightCalculator::new(1.0, 0.5, 1.0);

        let mut weights = vec![5.0];
        calc.normalize_weights(&mut weights);
        assert_eq!(weights[0], 1.0);
    }

    #[test]
    fn test_edge_cases() {
        let calc = WeightCalculator::new(0.0, 0.0, 0.0);

        // With lambda=0, mu=0: weight = exp(0 * proximity * trust - 0 * conflict) = e^0 = 1.0
        let weight = calc.compute_weight(10.0, 10.0, 10.0, 0.0);
        assert_eq!(weight, 1.0);

        let detailed = calc.compute_detailed_weights(10.0, 10.0, 10.0);
        assert_eq!(detailed.proximity, 10.0); // Input proximity
        assert_eq!(detailed.trust, 10.0); // 10.0^1.0 = 10.0 (alpha=1.0 by default)
                                          // conflict_penalty = exp(-0 * 10.0) = exp(0) = 1.0
        assert_eq!(detailed.conflict_penalty, 1.0);
        // total = exp(0 * 10.0 * 10.0) * exp(-0 * 10.0) = 1.0 * 1.0 = 1.0
        assert_eq!(detailed.total, 1.0);
    }

    #[test]
    fn test_extreme_values() {
        let calc = WeightCalculator::new(10.0, 10.0, 10.0);

        // Even with extreme parameters, weights should be finite
        let weight = calc.compute_weight(1.0, 1.0, 1.0, 0.0);
        assert!(weight.is_finite());

        // Very high conflict penalty should give very low weight
        let weight_high_penalty = calc.compute_weight(0.0, 0.0, 100.0, 0.0);
        assert!(weight_high_penalty < 1e-10);

        // Very high proximity and reputation should give very high weight
        let weight_high_positive = calc.compute_weight(10.0, 10.0, 0.0, 0.0);
        assert!(weight_high_positive > 1e10);
    }
}
