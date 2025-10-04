use serde::{Deserialize, Serialize};

/// Which finality method was used
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FinalityMethod {
    /// F2 (topological) finality via persistent homology
    F2 {
        h_d_stable: bool,
        h_d_minus_1_bottleneck: String, // Formatted as string for serialization
        confidence: String,
        elapsed_ms: u64,
    },
    /// F1 (k-core) finality as fallback
    F1Fallback {
        reason: String,
        k_core_depth: usize,
        elapsed_ms: u64,
    },
    /// F1 only (F2 disabled)
    F1Only {
        k_core_depth: usize,
        elapsed_ms: u64,
    },
}

impl FinalityMethod {
    pub fn f2(
        h_d_stable: bool,
        h_d_minus_1_bottleneck: f64,
        confidence: f64,
        elapsed_ms: u64,
    ) -> Self {
        Self::F2 {
            h_d_stable,
            h_d_minus_1_bottleneck: format!("{:.6}", h_d_minus_1_bottleneck),
            confidence: format!("{:.4}", confidence),
            elapsed_ms,
        }
    }

    pub fn f1_fallback(reason: String, k_core_depth: usize, elapsed_ms: u64) -> Self {
        Self::F1Fallback {
            reason,
            k_core_depth,
            elapsed_ms,
        }
    }

    pub fn f1_only(k_core_depth: usize, elapsed_ms: u64) -> Self {
        Self::F1Only {
            k_core_depth,
            elapsed_ms,
        }
    }

    pub fn is_f2(&self) -> bool {
        matches!(self, FinalityMethod::F2 { .. })
    }

    pub fn is_f1_fallback(&self) -> bool {
        matches!(self, FinalityMethod::F1Fallback { .. })
    }

    pub fn elapsed_ms(&self) -> u64 {
        match self {
            FinalityMethod::F2 { elapsed_ms, .. } => *elapsed_ms,
            FinalityMethod::F1Fallback { elapsed_ms, .. } => *elapsed_ms,
            FinalityMethod::F1Only { elapsed_ms, .. } => *elapsed_ms,
        }
    }
}
