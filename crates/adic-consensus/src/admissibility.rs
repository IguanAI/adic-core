use adic_types::{AdicMessage, AdicParams, AdicError, Result, AxisId, QpDigits};
use adic_math::{vp_diff, count_distinct_balls};

#[derive(Debug, Clone)]
pub struct AdmissibilityResult {
    pub score: f64,
    pub score_passed: bool,  // S(x;A) >= d
    pub c2_passed: bool,
    pub c3_passed: bool,
    pub is_admissible: bool,
    pub details: String,
}

impl AdmissibilityResult {
    pub fn new(score: f64, score_passed: bool, c2: bool, c3: bool, details: String) -> Self {
        Self {
            score,
            score_passed,
            c2_passed: c2,
            c3_passed: c3,
            is_admissible: score_passed && c2 && c3,
            details,
        }
    }
    
    pub fn is_admissible(&self) -> bool {
        self.is_admissible
    }
}

pub struct AdmissibilityChecker {
    params: AdicParams,
}

impl AdmissibilityChecker {
    pub fn new(params: AdicParams) -> Self {
        Self { params }
    }

    pub fn check_message(
        &self,
        message: &AdicMessage,
        parent_features: &[Vec<QpDigits>],
        parent_reputations: &[f64],
    ) -> Result<AdmissibilityResult> {
        if message.parents.len() != (self.params.d + 1) as usize {
            return Err(AdicError::AdmissibilityFailed(format!(
                "Expected {} parents, got {}",
                self.params.d + 1,
                message.parents.len()
            )));
        }

        // Calculate score S(x;A) using the correct paper formula and check if >= d
        let score = self.compute_admissibility_score(message, parent_features);
        let score_passed = score >= self.params.d as f64;
        
        let c2_result = self.check_c2_diversity(parent_features)?;
        let c3_result = self.check_c3_reputation(parent_reputations)?;

        let details = format!(
            "Score: {:.2} (>= {}), C2: {} (diversity), C3: {} (reputation)",
            score,
            self.params.d,
            if c2_result.0 { "PASS" } else { &c2_result.1 },
            if c3_result.0 { "PASS" } else { &c3_result.1 }
        );

        Ok(AdmissibilityResult::new(
            score,
            score_passed,
            c2_result.0,
            c3_result.0,
            details,
        ))
    }
    


    fn check_c2_diversity(&self, parent_features: &[Vec<QpDigits>]) -> Result<(bool, String)> {
        for (axis_idx, radius) in self.params.rho.iter().enumerate() {
            let axis_features: Vec<QpDigits> = parent_features
                .iter()
                .filter_map(|pf| pf.get(axis_idx).cloned())
                .collect();

            if axis_features.len() != parent_features.len() {
                return Ok((false, format!("Not all parents have axis {}", axis_idx)));
            }

            let distinct_count = count_distinct_balls(&axis_features, *radius as usize);
            
            if distinct_count < self.params.q as usize {
                return Ok((false, format!(
                    "C2 failed: axis {} has {} distinct balls, need >= {}",
                    axis_idx, distinct_count, self.params.q
                )));
            }
        }

        Ok((true, String::new()))
    }

    fn check_c3_reputation(&self, parent_reputations: &[f64]) -> Result<(bool, String)> {
        let min_rep = parent_reputations
            .iter()
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .copied()
            .unwrap_or(0.0);

        if min_rep < self.params.r_min {
            return Ok((false, format!(
                "C3 failed: minimum reputation {} < required {}",
                min_rep, self.params.r_min
            )));
        }

        let sum_rep: f64 = parent_reputations.iter().sum();
        
        if sum_rep < self.params.r_sum_min {
            return Ok((false, format!(
                "C3 failed: sum reputation {} < required {}",
                sum_rep, self.params.r_sum_min
            )));
        }

        Ok((true, String::new()))
    }

    pub fn compute_admissibility_score(
        &self,
        message: &AdicMessage,
        parent_features: &[Vec<QpDigits>],
    ) -> f64 {
        let mut total_score = 0.0;

        for (axis_idx, &radius) in self.params.rho.iter().enumerate() {
            let axis_id = AxisId(axis_idx as u32);
            let msg_phi = match message.features.get_axis(axis_id) {
                Some(phi) => &phi.qp_digits,
                None => continue,
            };

            let mut min_parent_term = 1.0;

            // Find the minimum term T_j(a) for the current axis j
            for parent_axis_features in parent_features {
                if let Some(parent_phi) = parent_axis_features.get(axis_idx) {
                    let valuation = vp_diff(msg_phi, parent_phi);
                    // Term is p^{-max(0, rho - valuation)}
                    let exponent = -(radius.saturating_sub(valuation) as i32);
                    let term = (self.params.p as f64).powi(exponent);
                    
                    if term < min_parent_term {
                        min_parent_term = term;
                    }
                }
            }
            // Add the minimum term for this axis to the total score
            total_score += min_parent_term;
        }

        total_score
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::{MessageId, AdicFeatures, AxisPhi, AdicMeta, PublicKey};
    use chrono::Utc;

    fn create_test_message(d: u32) -> AdicMessage {
        let parents = (0..=d).map(|i| MessageId::new(&[i as u8])).collect();
        let features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(10, 3, 5)),
            AxisPhi::new(1, QpDigits::from_u64(20, 3, 5)),
            AxisPhi::new(2, QpDigits::from_u64(30, 3, 5)),
        ]);
        let meta = AdicMeta::new(Utc::now());
        let pk = PublicKey::from_bytes([0; 32]);
        
        AdicMessage::new(parents, features, meta, pk, vec![])
    }

    #[test]
    fn test_admissibility_checker() {
        let params = AdicParams::default();
        let checker = AdmissibilityChecker::new(params);
        
        let message = create_test_message(3);
        let parent_features = vec![
            vec![
                QpDigits::from_u64(11, 3, 5),
                QpDigits::from_u64(21, 3, 5),
                QpDigits::from_u64(31, 3, 5),
            ],
            vec![
                QpDigits::from_u64(12, 3, 5),
                QpDigits::from_u64(22, 3, 5),
                QpDigits::from_u64(32, 3, 5),
            ],
            vec![
                QpDigits::from_u64(13, 3, 5),
                QpDigits::from_u64(23, 3, 5),
                QpDigits::from_u64(33, 3, 5),
            ],
            vec![
                QpDigits::from_u64(19, 3, 5),
                QpDigits::from_u64(29, 3, 5),
                QpDigits::from_u64(39, 3, 5),
            ],
        ];
        let parent_reps = vec![2.0, 2.0, 2.0, 2.0];
        
        let result = checker.check_message(&message, &parent_features, &parent_reps).unwrap();
        assert!(result.c3_passed);
    }

    #[test]
    fn test_admissibility_score_implementation_vs_spec() {
        // This test verifies the formula for S(x;A) from the whitepaper:
        // S(x; A) = Σ_j min_k p^(-max(0, ρ_j - vp(φ_j(x) - φ_j(a_k))))
        // The current implementation calculates Σ_j p^(-max(0, ρ_j - max_k vp(φ_j(x) - φ_j(a_k))))
        // which is sum over axis of max proximity, instead of sum over axis of min proximity term.

        let p = 3;
        let d = 1;
        let rho = vec![2, 1]; // ρ_0=2, ρ_1=1

        let mut params = AdicParams::default();
        params.p = p;
        params.d = d;
        params.rho = rho;

        let checker = AdmissibilityChecker::new(params);

        // Message features φ(x)
        let msg_features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(0, p, 5)), // φ_0(x) = 0
            AxisPhi::new(1, QpDigits::from_u64(0, p, 5)), // φ_1(x) = 0
        ]);
        let message = AdicMessage::new(vec![], msg_features, AdicMeta::new(Utc::now()), PublicKey::from_bytes([0;32]), vec![]);

        // Parent features φ(a_k)
        // Parent a_0: φ_0(a_0)=9 (vp=2), φ_1(a_0)=3 (vp=1)
        // Parent a_1: φ_0(a_1)=3 (vp=1), φ_1(a_1)=9 (vp=2)
        let parent_features = vec![
            vec![QpDigits::from_u64(9, p, 5), QpDigits::from_u64(3, p, 5)], // Parent 0
            vec![QpDigits::from_u64(3, p, 5), QpDigits::from_u64(9, p, 5)], // Parent 1
        ];

        // --- Manual Calculation (Spec) ---
        // Axis j=0 (ρ_0 = 2):
        //  k=0: vp(0-9)=2. term = 3^(-max(0, 2-2)) = 3^0 = 1.0
        //  k=1: vp(0-3)=1. term = 3^(-max(0, 2-1)) = 3^-1 = 0.333
        //  min_k term for j=0 is 0.333
        // Axis j=1 (ρ_1 = 1):
        //  k=0: vp(0-3)=1. term = 3^(-max(0, 1-1)) = 3^0 = 1.0
        //  k=1: vp(0-9)=2. term = 3^(-max(0, 1-2)) = 3^0 = 1.0
        //  min_k term for j=1 is 1.0
        // S(x;A) = 0.333 + 1.0 = 1.333
        let expected_score = (1.0/3.0) + 1.0;

        let score = checker.compute_admissibility_score(&message, &parent_features);

        // This assertion will fail with the current implementation.
        assert!((score - expected_score).abs() < 1e-9, "The calculated score {} does not match the expected score {}", score, expected_score);
    }
}