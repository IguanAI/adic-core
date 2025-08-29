use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MrwWeights {
    pub proximity: f64,
    pub trust: f64,
    pub conflict_penalty: f64,
    pub total: f64,
}

impl MrwWeights {
    pub fn new(proximity: f64, trust: f64, conflict_penalty: f64) -> Self {
        Self {
            proximity,
            trust,
            conflict_penalty,
            total: proximity * trust * (1.0 - conflict_penalty),
        }
    }
}

pub struct WeightCalculator {
    lambda: f64,
    alpha: f64, // Reputation exponent from PDF spec
    beta: f64,  // Age decay exponent (currently unused, for future)
    mu: f64,
}

impl WeightCalculator {
    pub fn new(lambda: f64, beta: f64, mu: f64) -> Self {
        // Note: beta parameter position kept for compatibility
        // alpha defaults to 1.0 per PDF spec
        Self {
            lambda,
            alpha: 1.0, // Default from PDF section 1.2
            beta,       // Will be used for age decay in future
            mu,
        }
    }

    pub fn new_with_alpha(lambda: f64, alpha: f64, beta: f64, mu: f64) -> Self {
        Self {
            lambda,
            alpha,
            beta,
            mu,
        }
    }

    pub fn compute_weight(&self, proximity: f64, reputation: f64, conflict_penalty: f64) -> f64 {
        let trust = self.compute_trust(reputation);
        // Per PDF: exp(λ * proximity * trust - μ * conflict)
        // Note: The PDF shows proximity and trust multiplied together inside exp
        let exp_arg = self.lambda * proximity * trust - self.mu * conflict_penalty;
        exp_arg.exp()
    }

    pub fn compute_trust(&self, reputation: f64) -> f64 {
        // Per PDF spec section 3.3: trust(y) := R(y)^α * (1 + age(y))^(-β)
        // Currently we only implement the reputation part: R(y)^α
        // Age factor will be added when message age tracking is available
        reputation.powf(self.alpha)
    }

    pub fn compute_trust_with_age(&self, reputation: f64, age: f64) -> f64 {
        // Full formula from PDF: R(y)^α * (1 + age(y))^(-β)
        reputation.powf(self.alpha) * (1.0 + age).powf(-self.beta)
    }

    pub fn compute_detailed_weights(
        &self,
        proximity: f64,
        reputation: f64,
        conflict_penalty: f64,
    ) -> MrwWeights {
        let trust = self.compute_trust(reputation);
        // Decompose the weight components for debugging
        // Total weight = exp(λ * proximity * trust - μ * conflict)
        let combined_weight = (self.lambda * proximity * trust).exp();
        let penalty_weight = (-self.mu * conflict_penalty).exp();

        MrwWeights {
            proximity,
            trust,
            conflict_penalty: penalty_weight,
            total: combined_weight * penalty_weight,
        }
    }

    pub fn compute_transition_probability(&self, candidate_weight: f64, total_weight: f64) -> f64 {
        if total_weight > 0.0 {
            candidate_weight / total_weight
        } else {
            0.0
        }
    }

    pub fn normalize_weights(&self, weights: &mut [f64]) {
        let sum: f64 = weights.iter().sum();
        if sum > 0.0 {
            for w in weights.iter_mut() {
                *w /= sum;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_weight_calculation() {
        let calc = WeightCalculator::new(1.0, 0.5, 1.0);

        let weight = calc.compute_weight(0.5, 2.0, 0.0);
        assert!(weight > 0.0);

        let weight_with_penalty = calc.compute_weight(0.5, 2.0, 0.5);
        assert!(weight_with_penalty < weight);
    }

    #[test]
    fn test_trust_function() {
        let calc = WeightCalculator::new(1.0, 0.5, 1.0);

        let trust_low = calc.compute_trust(0.0);
        let trust_medium = calc.compute_trust(1.0);
        let trust_high = calc.compute_trust(10.0);

        // With alpha = 1.0, trust = reputation^1.0 = reputation
        assert_eq!(trust_low, 0.0);
        assert_eq!(trust_medium, 1.0);
        assert_eq!(trust_high, 10.0);

        // Test with different alpha
        let calc2 = WeightCalculator::new_with_alpha(1.0, 2.0, 0.5, 1.0);
        assert_eq!(calc2.compute_trust(3.0), 9.0); // 3^2 = 9
    }

    #[test]
    fn test_detailed_weights() {
        let calc = WeightCalculator::new(1.0, 0.5, 1.0);

        let weights = calc.compute_detailed_weights(0.5, 2.0, 0.2);

        assert_eq!(weights.proximity, 0.5);
        assert_eq!(weights.trust, 2.0); // reputation^1.0 = 2.0
        assert!(weights.conflict_penalty > 0.0);
        assert!(weights.conflict_penalty <= 1.0);

        // Per PDF: weight = exp(λ * proximity * trust - μ * conflict)
        // The detailed weights decompose this for debugging
        let combined_weight = (1.0_f64 * 0.5 * 2.0).exp(); // exp(λ * proximity * trust)
        let penalty_weight = (-0.2_f64).exp(); // exp(-μ * conflict_penalty)
        let expected_total = combined_weight * penalty_weight;
        assert!((weights.total - expected_total).abs() < 1e-10);
    }

    #[test]
    fn test_normalize_weights() {
        let calc = WeightCalculator::new(1.0, 0.5, 1.0);

        let mut weights = vec![1.0, 2.0, 3.0, 4.0];
        calc.normalize_weights(&mut weights);

        let sum: f64 = weights.iter().sum();
        assert!((sum - 1.0).abs() < 1e-10);
    }
}
