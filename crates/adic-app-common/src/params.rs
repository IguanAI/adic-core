use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Shared application parameters
/// These can be updated via governance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppParams {
    /// VRF parameters
    pub vrf: VRFParams,

    /// Quorum parameters
    pub quorum: QuorumParams,

    /// Challenge parameters
    pub challenge: ChallengeParams,

    /// Timelock parameters
    pub timelock: TimelockParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VRFParams {
    /// Minimum reputation to participate in VRF commit-reveal
    pub min_committer_reputation: f64,

    /// Depth by which VRF must be revealed (in epochs)
    pub reveal_depth: u64,

    /// VRF domain separator prefix
    pub domain_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuorumParams {
    /// Minimum reputation for quorum eligibility
    pub min_arbitrator_reputation: f64,

    /// Members selected per axis
    pub members_per_axis: usize,

    /// Total quorum size
    pub quorum_size: usize,

    /// Max members from same ASN
    pub max_per_asn: usize,

    /// Max members from same region
    pub max_per_region: usize,

    /// Voting threshold (e.g., 0.66 for 2/3 majority)
    pub vote_threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeParams {
    /// Challenge window depth in epochs
    pub window_depth: u64,

    /// Fraud proof deposit factor (multiple of base deposit)
    pub fraud_proof_deposit_factor: u64,

    /// Dispute deposit factor (multiple of base deposit)
    pub dispute_deposit_factor: u64,

    /// Arbitrator minority penalty (ADIC-Rep adjustment)
    pub arbitrator_minority_penalty: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelockParams {
    /// F1 grace period in epochs
    pub f1_grace_period: u64,

    /// F2 grace period in epochs
    pub f2_grace_period: u64,

    /// Minimum timelock duration in epochs
    pub min_timelock_epochs: u64,
}

impl Default for AppParams {
    fn default() -> Self {
        Self {
            vrf: VRFParams {
                min_committer_reputation: 50.0,
                reveal_depth: 10,
                domain_prefix: "ADIC-VRF-v1".to_string(),
            },
            quorum: QuorumParams {
                min_arbitrator_reputation: 50.0,
                members_per_axis: 5,
                quorum_size: 15,
                max_per_asn: 2,
                max_per_region: 3,
                vote_threshold: 0.66,
            },
            challenge: ChallengeParams {
                window_depth: 100,
                fraud_proof_deposit_factor: 10,
                dispute_deposit_factor: 5,
                arbitrator_minority_penalty: -2.0,
            },
            timelock: TimelockParams {
                f1_grace_period: 3,
                f2_grace_period: 12,
                min_timelock_epochs: 10,
            },
        }
    }
}

impl AppParams {
    /// Load parameters from storage or use defaults
    ///
    /// Attempts to load from:
    /// 1. Governance parameter file at provided path
    /// 2. Falls back to defaults if file doesn't exist
    pub fn load_or_default() -> Self {
        Self::load_from_path_or_default(None)
    }

    /// Load parameters from specific path or use defaults
    pub fn load_from_path_or_default(path: Option<std::path::PathBuf>) -> Self {
        let param_path = path.unwrap_or_else(|| {
            std::env::var("ADIC_PARAMS_PATH")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("governance_params.json"))
        });

        if param_path.exists() {
            match Self::load_from_file(&param_path) {
                Ok(params) => {
                    info!("✅ Loaded parameters from {:?}", param_path);
                    return params;
                }
                Err(e) => {
                    warn!(
                        "Failed to load parameters from {:?}: {}. Using defaults.",
                        param_path, e
                    );
                }
            }
        }

        Self::default()
    }

    /// Load parameters from JSON file
    fn load_from_file(path: &std::path::Path) -> crate::Result<Self> {
        use std::fs::File;
        use std::io::Read;

        let mut file = File::open(path).map_err(|e| {
            crate::AppError::InvalidConfiguration(format!("Failed to open params file: {}", e))
        })?;

        let mut contents = String::new();
        file.read_to_string(&mut contents).map_err(|e| {
            crate::AppError::InvalidConfiguration(format!("Failed to read params file: {}", e))
        })?;

        let params: AppParams = serde_json::from_str(&contents).map_err(|e| {
            crate::AppError::InvalidConfiguration(format!("Failed to parse params: {}", e))
        })?;

        params.validate()?;
        Ok(params)
    }

    /// Load from governance-exported JSON value
    ///
    /// Used when governance system exports parameters via ParameterRegistry::export_app_params()
    pub fn from_governance_export(value: &serde_json::Value) -> crate::Result<Self> {
        let params: AppParams = serde_json::from_value(value.clone()).map_err(|e| {
            crate::AppError::InvalidConfiguration(format!(
                "Failed to parse governance params: {}",
                e
            ))
        })?;

        params.validate()?;
        Ok(params)
    }

    /// Save parameters to JSON file
    pub fn save_to_file(&self, path: &std::path::Path) -> crate::Result<()> {
        use std::fs::File;
        use std::io::Write;

        let json = serde_json::to_string_pretty(self).map_err(|e| {
            crate::AppError::InvalidConfiguration(format!("Failed to serialize params: {}", e))
        })?;

        let mut file = File::create(path).map_err(|e| {
            crate::AppError::InvalidConfiguration(format!("Failed to create params file: {}", e))
        })?;

        file.write_all(json.as_bytes()).map_err(|e| {
            crate::AppError::InvalidConfiguration(format!("Failed to write params: {}", e))
        })?;

        info!("💾 Parameters saved to {:?}", path);
        Ok(())
    }

    /// Validate parameter ranges
    pub fn validate(&self) -> crate::Result<()> {
        if self.vrf.min_committer_reputation < 0.0 {
            return Err(crate::AppError::InvalidConfiguration(
                "VRF min_committer_reputation must be >= 0".to_string(),
            ));
        }

        if self.quorum.vote_threshold < 0.5 || self.quorum.vote_threshold > 1.0 {
            return Err(crate::AppError::InvalidConfiguration(
                "Quorum vote_threshold must be between 0.5 and 1.0".to_string(),
            ));
        }

        if self.challenge.window_depth == 0 {
            return Err(crate::AppError::InvalidConfiguration(
                "Challenge window_depth must be > 0".to_string(),
            ));
        }

        Ok(())
    }
}
