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
    beta: f64,
    mu: f64,
}

impl WeightCalculator {
    pub fn new(lambda: f64, beta: f64, mu: f64) -> Self {
        Self { lambda, beta, mu }
    }

    pub fn compute_weight(
        &self,
        proximity: f64,
        reputation: f64,
        conflict_penalty: f64,
    ) -> f64 {
        let trust = self.compute_trust(reputation);
        let exp_arg = self.lambda * proximity + self.beta * trust - self.mu * conflict_penalty;
        exp_arg.exp()
    }

    pub fn compute_trust(&self, reputation: f64) -> f64 {
        (1.0 + reputation).ln()
    }

    pub fn compute_detailed_weights(
        &self,
        proximity: f64,
        reputation: f64,
        conflict_penalty: f64,
    ) -> MrwWeights {
        let trust = self.compute_trust(reputation);
        let prox_weight = (self.lambda * proximity).exp();
        let trust_weight = (self.beta * trust).exp();
        let penalty_weight = (-self.mu * conflict_penalty).exp();
        
        MrwWeights {
            proximity: prox_weight,
            trust: trust_weight,
            conflict_penalty: penalty_weight,
            total: prox_weight * trust_weight * penalty_weight,
        }
    }

    pub fn compute_transition_probability(
        &self,
        candidate_weight: f64,
        total_weight: f64,
    ) -> f64 {
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
        let trust_high = calc.compute_trust(10.0);
        
        assert_eq!(trust_low, 0.0);
        assert!(trust_high > trust_low);
        assert!(trust_high < 10.0);
    }

    #[test]
    fn test_detailed_weights() {
        let calc = WeightCalculator::new(1.0, 0.5, 1.0);
        
        let weights = calc.compute_detailed_weights(0.5, 2.0, 0.2);
        
        assert!(weights.proximity > 0.0);
        assert!(weights.trust > 0.0);
        assert!(weights.conflict_penalty > 0.0);
        assert!(weights.conflict_penalty <= 1.0);
        
        let expected = weights.proximity * weights.trust * weights.conflict_penalty;
        assert!((weights.total - expected).abs() < 1e-10);
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