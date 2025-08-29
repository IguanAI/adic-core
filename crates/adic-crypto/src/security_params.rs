//! Security parameters management for ADIC-DAG
//!
//! Manages and validates system security parameters according to the whitepaper
//! specification (Section 1.2 and throughout).

use serde::{Deserialize, Serialize};
use std::fmt;

/// Security parameters for the ADIC-DAG system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityParams {
    /// Prime for p-adic arithmetic
    pub p: u32,
    /// Dimension (number of axes)
    pub d: u32,
    /// Axis radii for p-adic balls
    pub rho: Vec<u32>,
    /// Diversity threshold (min distinct balls per axis)
    pub q: u32,
    /// Refundable anti-spam deposit (in ADIC)
    pub deposit: f64,
    /// Reputation decay factor
    pub gamma: f64,
    /// Reputation exponents (alpha, beta)
    pub reputation_exponents: (f64, f64),
    /// k-core threshold for finality
    pub k_core: u32,
    /// Finality depth threshold
    pub depth_threshold: u32,
    /// Persistent homology window
    pub homology_window: u32,
    /// Minimum individual reputation (C3)
    pub r_min: f64,
    /// Minimum sum reputation (C3)
    pub r_sum_min: f64,
    /// MRW parameters (lambda, mu)
    pub mrw_params: (f64, f64),
    /// Maximum message size (bytes)
    pub max_message_size: usize,
    /// Precision for p-adic calculations
    pub padic_precision: usize,
}

impl SecurityParams {
    /// Create v1 default parameters as specified in the whitepaper
    pub fn v1_defaults() -> Self {
        Self {
            p: 3,
            d: 3,
            rho: vec![2, 2, 1],
            q: 3,
            deposit: 0.1,
            gamma: 0.9,
            reputation_exponents: (1.0, 1.0),
            k_core: 20,
            depth_threshold: 12,
            homology_window: 5,
            r_min: 1.0,
            r_sum_min: 10.0,
            mrw_params: (1.0, 0.5),
            max_message_size: 1_048_576, // 1MB
            padic_precision: 10,
        }
    }

    /// Create parameters for testing
    pub fn test_params() -> Self {
        let mut params = Self::v1_defaults();
        params.k_core = 5;
        params.depth_threshold = 3;
        params.r_min = 0.1;
        params.r_sum_min = 1.0;
        params
    }

    /// Validate parameter consistency and security
    pub fn validate(&self) -> Result<(), SecurityError> {
        // Check prime is actually prime
        if !is_prime(self.p) {
            return Err(SecurityError::InvalidPrime(self.p));
        }

        // Check dimension constraints
        if self.d == 0 || self.d > 10 {
            return Err(SecurityError::InvalidDimension(self.d));
        }

        // Check radii vector matches dimension
        if self.rho.len() != self.d as usize {
            return Err(SecurityError::RadiiDimensionMismatch {
                radii_len: self.rho.len(),
                dimension: self.d,
            });
        }

        // Check all radii are positive
        if self.rho.contains(&0) {
            return Err(SecurityError::ZeroRadius);
        }

        // Check diversity threshold
        if self.q < 2 || self.q > self.d + 1 {
            return Err(SecurityError::InvalidDiversity(self.q));
        }

        // Check k-core threshold
        if self.k_core < self.d + 1 {
            return Err(SecurityError::KCoreTooSmall {
                k_core: self.k_core,
                min_required: self.d + 1,
            });
        }

        // Check reputation parameters
        if self.r_min <= 0.0 || self.r_sum_min <= 0.0 {
            return Err(SecurityError::InvalidReputation);
        }

        if self.r_sum_min < self.r_min * (self.d + 1) as f64 {
            return Err(SecurityError::InconsistentReputation);
        }

        // Check gamma in valid range
        if self.gamma <= 0.0 || self.gamma >= 1.0 {
            return Err(SecurityError::InvalidGamma(self.gamma));
        }

        // Check MRW parameters
        if self.mrw_params.0 <= 0.0 || self.mrw_params.1 < 0.0 {
            return Err(SecurityError::InvalidMRWParams);
        }

        Ok(())
    }

    /// Calculate security level estimate (0-100)
    pub fn security_level(&self) -> u32 {
        let mut score = 0u32;

        // Prime size contribution (larger is better)
        score += match self.p {
            p if p >= 251 => 20,
            p if p >= 127 => 15,
            p if p >= 31 => 10,
            _ => 5,
        };

        // Dimension contribution
        score += self.d.min(5) * 5;

        // Diversity contribution
        score += self.q.min(5) * 5;

        // k-core size contribution
        score += match self.k_core {
            k if k >= 50 => 20,
            k if k >= 20 => 15,
            k if k >= 10 => 10,
            _ => 5,
        };

        // Depth contribution
        score += match self.depth_threshold {
            d if d >= 20 => 15,
            d if d >= 12 => 10,
            d if d >= 6 => 5,
            _ => 2,
        };

        // Reputation thresholds
        if self.r_min >= 1.0 && self.r_sum_min >= 10.0 {
            score += 10;
        }

        // Radii contribution (higher ultrametric enforcement)
        let avg_radius = self.rho.iter().sum::<u32>() as f64 / self.rho.len() as f64;
        score += if avg_radius >= 2.0 { 10 } else { 5 };

        score.min(100)
    }

    /// Check if parameters meet minimum security requirements
    pub fn meets_security_requirements(&self) -> bool {
        self.validate().is_ok() && self.security_level() >= 60
    }

    /// Adjust parameters for different network phases
    pub fn adjust_for_phase(&mut self, phase: NetworkPhase) {
        match phase {
            NetworkPhase::Bootstrap => {
                self.k_core = 5;
                self.depth_threshold = 3;
                self.r_min = 0.1;
                self.r_sum_min = 1.0;
            }
            NetworkPhase::Growth => {
                self.k_core = 10;
                self.depth_threshold = 6;
                self.r_min = 0.5;
                self.r_sum_min = 5.0;
            }
            NetworkPhase::Mature => {
                self.k_core = 20;
                self.depth_threshold = 12;
                self.r_min = 1.0;
                self.r_sum_min = 10.0;
            }
            NetworkPhase::HighSecurity => {
                self.k_core = 50;
                self.depth_threshold = 20;
                self.r_min = 2.0;
                self.r_sum_min = 50.0;
                self.q = (self.d + 1).min(5);
            }
        }
    }
}

/// Network phase for parameter adjustment
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkPhase {
    Bootstrap,
    Growth,
    Mature,
    HighSecurity,
}

/// Security parameter errors
#[derive(Debug, Clone)]
pub enum SecurityError {
    InvalidPrime(u32),
    InvalidDimension(u32),
    RadiiDimensionMismatch { radii_len: usize, dimension: u32 },
    ZeroRadius,
    InvalidDiversity(u32),
    KCoreTooSmall { k_core: u32, min_required: u32 },
    InvalidReputation,
    InconsistentReputation,
    InvalidGamma(f64),
    InvalidMRWParams,
}

impl fmt::Display for SecurityError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::InvalidPrime(p) => write!(f, "Invalid prime: {}", p),
            Self::InvalidDimension(d) => write!(f, "Invalid dimension: {}", d),
            Self::RadiiDimensionMismatch {
                radii_len,
                dimension,
            } => {
                write!(
                    f,
                    "Radii vector length {} doesn't match dimension {}",
                    radii_len, dimension
                )
            }
            Self::ZeroRadius => write!(f, "Radius cannot be zero"),
            Self::InvalidDiversity(q) => write!(f, "Invalid diversity threshold: {}", q),
            Self::KCoreTooSmall {
                k_core,
                min_required,
            } => {
                write!(f, "k-core {} is less than minimum {}", k_core, min_required)
            }
            Self::InvalidReputation => write!(f, "Invalid reputation parameters"),
            Self::InconsistentReputation => write!(f, "Inconsistent reputation thresholds"),
            Self::InvalidGamma(g) => write!(f, "Invalid gamma: {}", g),
            Self::InvalidMRWParams => write!(f, "Invalid MRW parameters"),
        }
    }
}

impl std::error::Error for SecurityError {}

/// Check if a number is prime
fn is_prime(n: u32) -> bool {
    if n < 2 {
        return false;
    }
    if n == 2 {
        return true;
    }
    if n % 2 == 0 {
        return false;
    }

    let sqrt_n = (n as f64).sqrt() as u32;
    for i in (3..=sqrt_n).step_by(2) {
        if n % i == 0 {
            return false;
        }
    }
    true
}

/// Security parameter validator for runtime checks
pub struct ParameterValidator {
    params: SecurityParams,
}

impl ParameterValidator {
    pub fn new(params: SecurityParams) -> Result<Self, SecurityError> {
        params.validate()?;
        Ok(Self { params })
    }

    /// Check if a value satisfies p-adic constraints
    pub fn validate_padic_value(&self, value: u64, axis_idx: usize) -> bool {
        if axis_idx >= self.params.d as usize {
            return false;
        }

        // Check value is within reasonable bounds for p-adic representation
        let max_value = (self.params.p as u64).pow(self.params.padic_precision as u32);
        value < max_value
    }

    /// Validate message size
    pub fn validate_message_size(&self, size: usize) -> bool {
        size <= self.params.max_message_size
    }

    /// Get adjusted parameters for current network conditions
    pub fn get_adjusted_params(&self, node_count: usize, avg_reputation: f64) -> SecurityParams {
        let mut adjusted = self.params.clone();

        // Dynamically adjust based on network size
        if node_count < 100 {
            adjusted.adjust_for_phase(NetworkPhase::Bootstrap);
        } else if node_count < 1000 {
            adjusted.adjust_for_phase(NetworkPhase::Growth);
        } else if avg_reputation > 10.0 {
            adjusted.adjust_for_phase(NetworkPhase::HighSecurity);
        } else {
            adjusted.adjust_for_phase(NetworkPhase::Mature);
        }

        adjusted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_v1_defaults() {
        let params = SecurityParams::v1_defaults();
        assert_eq!(params.p, 3);
        assert_eq!(params.d, 3);
        assert_eq!(params.rho, vec![2, 2, 1]);
        assert_eq!(params.q, 3);
        assert!(params.validate().is_ok());
    }

    #[test]
    fn test_validation() {
        let mut params = SecurityParams::v1_defaults();
        assert!(params.validate().is_ok());

        // Invalid prime
        params.p = 4;
        assert!(params.validate().is_err());
        params.p = 3;

        // Dimension mismatch
        params.rho = vec![2, 2];
        assert!(params.validate().is_err());
        params.rho = vec![2, 2, 1];

        // Zero radius
        params.rho = vec![2, 0, 1];
        assert!(params.validate().is_err());
        params.rho = vec![2, 2, 1];

        // Invalid diversity
        params.q = 10;
        assert!(params.validate().is_err());
        params.q = 3;

        // k-core too small
        params.k_core = 2;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_security_level() {
        let params = SecurityParams::v1_defaults();
        let level = params.security_level();
        assert!(level > 0);
        assert!(level <= 100);
        assert!(params.meets_security_requirements());
    }

    #[test]
    fn test_phase_adjustment() {
        let mut params = SecurityParams::v1_defaults();

        params.adjust_for_phase(NetworkPhase::Bootstrap);
        assert_eq!(params.k_core, 5);

        params.adjust_for_phase(NetworkPhase::HighSecurity);
        assert_eq!(params.k_core, 50);
        assert_eq!(params.r_min, 2.0);
    }

    #[test]
    fn test_is_prime() {
        assert!(is_prime(2));
        assert!(is_prime(3));
        assert!(is_prime(5));
        assert!(is_prime(251));
        assert!(!is_prime(4));
        assert!(!is_prime(9));
        assert!(!is_prime(100));
    }
}
