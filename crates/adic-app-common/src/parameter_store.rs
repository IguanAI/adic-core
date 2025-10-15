//! Live Parameter Store with Governance Integration
//!
//! Provides thread-safe access to live network parameters that can be
//! updated via governance proposals. Bridges ParameterRegistry (governance)
//! and application modules.
//!
//! # Architecture
//!
//! ```text
//! Governance Proposal (Enacted)
//!         ↓
//! ParameterStore::apply_governance_change()
//!         ↓
//! Validate → Update → Notify listeners
//!         ↓
//! Application modules read updated values
//! ```
//!
//! # Usage
//!
//! ```rust,no_run
//! # use adic_app_common::ParameterStore;
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let store = ParameterStore::new();
//!
//! // Read current value
//! let k_core = store.get_u32("k").await?;
//!
//! // Listen for changes
//! let mut rx = store.subscribe();
//! tokio::spawn(async move {
//!     while let Ok(change) = rx.recv().await {
//!         println!("Parameter {} changed to {}", change.key, change.new_value);
//!     }
//! });
//!
//! // Apply governance change (called by lifecycle manager)
//! store.apply_governance_change("k", serde_json::json!(25), None).await?;
//! # Ok(())
//! # }
//! ```

use crate::{AppError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{info, warn};

/// Maximum number of parameter change subscribers
const MAX_SUBSCRIBERS: usize = 100;

/// Parameter change notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterChange {
    /// Parameter key
    pub key: String,
    /// Previous value
    pub old_value: serde_json::Value,
    /// New value
    pub new_value: serde_json::Value,
    /// Epoch when change was applied
    pub applied_at_epoch: u64,
    /// Governance proposal ID that triggered this change
    pub proposal_id: Option<[u8; 32]>,
}

/// Validation rule for parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidationRule {
    /// Value must be in set
    InSet(Vec<serde_json::Value>),
    /// Value must be in range [min, max]
    Range {
        min: serde_json::Value,
        max: serde_json::Value,
    },
    /// Value must be >= min
    GreaterThanOrEqual(serde_json::Value),
    /// Custom validation function
    Custom(String),
    /// No validation
    None,
}

/// Parameter metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterMetadata {
    pub key: String,
    pub param_type: String,
    pub governance_class: String,
    pub default_value: serde_json::Value,
    pub validation: ValidationRule,
    pub description: String,
}

/// Live parameter store with governance integration
#[derive(Clone)]
pub struct ParameterStore {
    /// Current parameter values (thread-safe)
    values: Arc<RwLock<HashMap<String, serde_json::Value>>>,

    /// Parameter metadata and validation rules
    metadata: Arc<RwLock<HashMap<String, ParameterMetadata>>>,

    /// Change notification broadcaster
    change_tx: broadcast::Sender<ParameterChange>,

    /// Current epoch (for change tracking)
    current_epoch: Arc<RwLock<u64>>,
}

impl ParameterStore {
    /// Create new parameter store with default ADIC parameters
    pub fn new() -> Self {
        let (change_tx, _) = broadcast::channel(MAX_SUBSCRIBERS);

        let mut values = HashMap::new();
        let mut metadata_map = HashMap::new();

        // Register all 19 parameters from PoUW III Table 1
        Self::register_default_parameters(&mut values, &mut metadata_map);

        Self {
            values: Arc::new(RwLock::new(values)),
            metadata: Arc::new(RwLock::new(metadata_map)),
            change_tx,
            current_epoch: Arc::new(RwLock::new(0)),
        }
    }

    /// Register default ADIC parameters from PoUW III Table 1
    fn register_default_parameters(
        values: &mut HashMap<String, serde_json::Value>,
        metadata: &mut HashMap<String, ParameterMetadata>,
    ) {
        // Constitutional parameters
        Self::register_param(
            values,
            metadata,
            "p",
            "U8",
            "Constitutional",
            serde_json::json!(3),
            ValidationRule::InSet(vec![
                serde_json::json!(3),
                serde_json::json!(5),
                serde_json::json!(7),
            ]),
            "Prime base for p-adic valuation",
        );

        Self::register_param(
            values,
            metadata,
            "d",
            "U8",
            "Constitutional",
            serde_json::json!(3),
            ValidationRule::InSet(vec![serde_json::json!(3), serde_json::json!(4)]),
            "Dimension of p-adic feature space",
        );

        Self::register_param(
            values,
            metadata,
            "rho",
            "VecU8",
            "Constitutional",
            serde_json::json!([2, 2, 1]),
            ValidationRule::Custom("validate_rho".to_string()),
            "Axis radii (must be length d, all ≥1)",
        );

        Self::register_param(
            values,
            metadata,
            "q",
            "U8",
            "Constitutional",
            serde_json::json!(3),
            ValidationRule::Range {
                min: serde_json::json!(2),
                max: serde_json::json!(5),
            },
            "Diversity requirement (distinct balls per axis)",
        );

        // Operational parameters
        Self::register_param(
            values,
            metadata,
            "k",
            "U32",
            "Operational",
            serde_json::json!(20),
            ValidationRule::GreaterThanOrEqual(serde_json::json!(1)),
            "k-core threshold for F1 finality",
        );

        Self::register_param(
            values,
            metadata,
            "D",
            "U32",
            "Operational",
            serde_json::json!(12),
            ValidationRule::GreaterThanOrEqual(serde_json::json!(1)),
            "Depth threshold for F1 finality",
        );

        Self::register_param(
            values,
            metadata,
            "Delta",
            "U32",
            "Operational",
            serde_json::json!(5),
            ValidationRule::GreaterThanOrEqual(serde_json::json!(1)),
            "Window size for finality stabilization",
        );

        Self::register_param(
            values,
            metadata,
            "lambda",
            "F64",
            "Operational",
            serde_json::json!(1.0),
            ValidationRule::GreaterThanOrEqual(serde_json::json!(0.0)),
            "MRW proximity weight",
        );

        Self::register_param(
            values,
            metadata,
            "mu",
            "F64",
            "Operational",
            serde_json::json!(1.0),
            ValidationRule::GreaterThanOrEqual(serde_json::json!(0.0)),
            "MRW trust weight",
        );

        Self::register_param(
            values,
            metadata,
            "alpha",
            "F64",
            "Operational",
            serde_json::json!(1.0),
            ValidationRule::GreaterThanOrEqual(serde_json::json!(0.0)),
            "MRW proximity exponent",
        );

        Self::register_param(
            values,
            metadata,
            "beta",
            "F64",
            "Operational",
            serde_json::json!(1.0),
            ValidationRule::GreaterThanOrEqual(serde_json::json!(0.0)),
            "MRW trust exponent",
        );

        Self::register_param(
            values,
            metadata,
            "Rmin",
            "F64",
            "Operational",
            serde_json::json!(50.0),
            ValidationRule::GreaterThanOrEqual(serde_json::json!(0.0)),
            "Minimum reputation for message submission",
        );

        Self::register_param(
            values,
            metadata,
            "rmin",
            "F64",
            "Operational",
            serde_json::json!(10.0),
            ValidationRule::GreaterThanOrEqual(serde_json::json!(0.0)),
            "Minimum reputation weight for parents",
        );

        Self::register_param(
            values,
            metadata,
            "m",
            "U32",
            "Operational",
            serde_json::json!(64),
            ValidationRule::GreaterThanOrEqual(serde_json::json!(1)),
            "Quorum committee size",
        );

        Self::register_param(
            values,
            metadata,
            "t_bls",
            "U32",
            "Operational",
            serde_json::json!(43), // ⌈2*64/3⌉
            ValidationRule::Custom("validate_t_bls".to_string()),
            "BLS threshold (must be ≥ ⌈m/2⌉, ≤ m)",
        );

        Self::register_param(
            values,
            metadata,
            "replication_r",
            "U32",
            "Operational",
            serde_json::json!(5),
            ValidationRule::GreaterThanOrEqual(serde_json::json!(1)),
            "Storage replication factor",
        );

        Self::register_param(
            values,
            metadata,
            "replication_t",
            "U32",
            "Operational",
            serde_json::json!(4),
            ValidationRule::Custom("validate_replication_t".to_string()),
            "Storage retrieval threshold (must be ≤ r)",
        );

        Self::register_param(
            values,
            metadata,
            "challenge_depth",
            "U32",
            "Operational",
            serde_json::json!(5),
            ValidationRule::GreaterThanOrEqual(serde_json::json!(1)),
            "Challenge window depth in epochs",
        );

        Self::register_param(
            values,
            metadata,
            "T_enact",
            "F64",
            "Operational",
            serde_json::json!(1000.0),
            ValidationRule::GreaterThanOrEqual(serde_json::json!(0.0)),
            "Timelock duration for governance enactment (seconds)",
        );

        Self::register_param(
            values,
            metadata,
            "Rmax",
            "F64",
            "Operational",
            serde_json::json!(100_000.0),
            ValidationRule::GreaterThanOrEqual(serde_json::json!(1.0)),
            "Maximum reputation cap for voting credits",
        );

        info!("✅ Registered {} governable parameters", metadata.len());
    }

    /// Helper to register a parameter
    fn register_param(
        values: &mut HashMap<String, serde_json::Value>,
        metadata: &mut HashMap<String, ParameterMetadata>,
        key: &str,
        param_type: &str,
        governance_class: &str,
        default_value: serde_json::Value,
        validation: ValidationRule,
        description: &str,
    ) {
        values.insert(key.to_string(), default_value.clone());
        metadata.insert(
            key.to_string(),
            ParameterMetadata {
                key: key.to_string(),
                param_type: param_type.to_string(),
                governance_class: governance_class.to_string(),
                default_value,
                validation,
                description: description.to_string(),
            },
        );
    }

    /// Update current epoch (called by node)
    pub async fn set_epoch(&self, epoch: u64) {
        let mut current = self.current_epoch.write().await;
        *current = epoch;
    }

    /// Get current epoch
    pub async fn get_epoch(&self) -> u64 {
        *self.current_epoch.read().await
    }

    /// Get parameter value as JSON
    pub async fn get(&self, key: &str) -> Option<serde_json::Value> {
        let values = self.values.read().await;
        values.get(key).cloned()
    }

    /// Get parameter as u8
    pub async fn get_u8(&self, key: &str) -> Result<u8> {
        self.get(key)
            .await
            .and_then(|v| v.as_u64().map(|n| n as u8))
            .ok_or_else(|| AppError::ParameterNotFound(key.to_string()))
    }

    /// Get parameter as u32
    pub async fn get_u32(&self, key: &str) -> Result<u32> {
        self.get(key)
            .await
            .and_then(|v| v.as_u64().map(|n| n as u32))
            .ok_or_else(|| AppError::ParameterNotFound(key.to_string()))
    }

    /// Get parameter as u64
    pub async fn get_u64(&self, key: &str) -> Result<u64> {
        self.get(key)
            .await
            .and_then(|v| v.as_u64())
            .ok_or_else(|| AppError::ParameterNotFound(key.to_string()))
    }

    /// Get parameter as f64
    pub async fn get_f64(&self, key: &str) -> Result<f64> {
        self.get(key)
            .await
            .and_then(|v| v.as_f64())
            .ok_or_else(|| AppError::ParameterNotFound(key.to_string()))
    }

    /// Get parameter as Vec<u8>
    pub async fn get_vec_u8(&self, key: &str) -> Result<Vec<u8>> {
        self.get(key)
            .await
            .and_then(|v| {
                v.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.as_u64().map(|n| n as u8))
                        .collect()
                })
            })
            .ok_or_else(|| AppError::ParameterNotFound(key.to_string()))
    }

    /// Get parameter as Vec<u32>
    pub async fn get_vec_u32(&self, key: &str) -> Result<Vec<u32>> {
        self.get(key)
            .await
            .and_then(|v| {
                v.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.as_u64().map(|n| n as u32))
                        .collect()
                })
            })
            .ok_or_else(|| AppError::ParameterNotFound(key.to_string()))
    }

    /// Get all current parameter values
    pub async fn get_all(&self) -> HashMap<String, serde_json::Value> {
        let values = self.values.read().await;
        values.clone()
    }

    /// Get parameter metadata
    pub async fn get_metadata(&self, key: &str) -> Option<ParameterMetadata> {
        let metadata = self.metadata.read().await;
        metadata.get(key).cloned()
    }

    /// Subscribe to parameter changes
    pub fn subscribe(&self) -> broadcast::Receiver<ParameterChange> {
        self.change_tx.subscribe()
    }

    /// Apply governance-enacted parameter change
    ///
    /// This is the main entry point for governance integration.
    /// Called by ProposalLifecycleManager after proposal passes timelock.
    pub async fn apply_governance_change(
        &self,
        key: &str,
        new_value: serde_json::Value,
        proposal_id: Option<[u8; 32]>,
    ) -> Result<()> {
        // Validate parameter exists
        let meta = self
            .get_metadata(key)
            .await
            .ok_or_else(|| AppError::ParameterNotFound(key.to_string()))?;

        // Validate new value
        self.validate_value(key, &new_value, &meta.validation)
            .await?;

        // Get old value
        let old_value = self
            .get(key)
            .await
            .ok_or_else(|| AppError::ParameterNotFound(key.to_string()))?;

        // Apply change
        {
            let mut values = self.values.write().await;
            values.insert(key.to_string(), new_value.clone());
        }

        // Get current epoch
        let epoch = self.get_epoch().await;

        // Notify listeners
        let change = ParameterChange {
            key: key.to_string(),
            old_value: old_value.clone(),
            new_value: new_value.clone(),
            applied_at_epoch: epoch,
            proposal_id,
        };

        // Ignore send errors (no subscribers is ok)
        let _ = self.change_tx.send(change);

        info!(
            key = %key,
            old_value = %old_value,
            new_value = %new_value,
            epoch = epoch,
            proposal_id = proposal_id.map(|id| hex::encode(&id[..8])).unwrap_or_else(|| "none".to_string()),
            "📝 Parameter updated via governance"
        );

        Ok(())
    }

    /// Validate parameter value
    async fn validate_value(
        &self,
        key: &str,
        value: &serde_json::Value,
        rule: &ValidationRule,
    ) -> Result<()> {
        match rule {
            ValidationRule::InSet(allowed) => {
                if !allowed.contains(value) {
                    return Err(AppError::ParameterValidationFailed {
                        param: key.to_string(),
                        reason: format!("Value must be one of {:?}", allowed),
                    });
                }
            }
            ValidationRule::Range { min, max } => {
                if !self.value_in_range(value, min, max) {
                    return Err(AppError::ParameterValidationFailed {
                        param: key.to_string(),
                        reason: format!("Value must be in range [{}, {}]", min, max),
                    });
                }
            }
            ValidationRule::GreaterThanOrEqual(min) => {
                if !self.value_gte(value, min) {
                    return Err(AppError::ParameterValidationFailed {
                        param: key.to_string(),
                        reason: format!("Value must be >= {}", min),
                    });
                }
            }
            ValidationRule::Custom(func_name) => {
                self.validate_custom(key, value, func_name).await?;
            }
            ValidationRule::None => {}
        }

        Ok(())
    }

    /// Check if value is in range
    fn value_in_range(
        &self,
        value: &serde_json::Value,
        min: &serde_json::Value,
        max: &serde_json::Value,
    ) -> bool {
        if let (Some(v), Some(min_v), Some(max_v)) = (value.as_f64(), min.as_f64(), max.as_f64()) {
            v >= min_v && v <= max_v
        } else if let (Some(v), Some(min_v), Some(max_v)) =
            (value.as_u64(), min.as_u64(), max.as_u64())
        {
            v >= min_v && v <= max_v
        } else {
            false
        }
    }

    /// Check if value >= min
    fn value_gte(&self, value: &serde_json::Value, min: &serde_json::Value) -> bool {
        if let (Some(v), Some(min_v)) = (value.as_f64(), min.as_f64()) {
            v >= min_v
        } else if let (Some(v), Some(min_v)) = (value.as_u64(), min.as_u64()) {
            v >= min_v
        } else {
            false
        }
    }

    /// Custom validation functions
    async fn validate_custom(
        &self,
        key: &str,
        value: &serde_json::Value,
        func: &str,
    ) -> Result<()> {
        match func {
            "validate_rho" => {
                let arr = value
                    .as_array()
                    .ok_or_else(|| AppError::ParameterValidationFailed {
                        param: key.to_string(),
                        reason: "rho must be an array".to_string(),
                    })?;

                let d = self.get_u64("d").await.unwrap_or(3);

                if arr.len() != d as usize {
                    return Err(AppError::ParameterValidationFailed {
                        param: key.to_string(),
                        reason: format!("rho length must match d={}", d),
                    });
                }

                for (i, r) in arr.iter().enumerate() {
                    if r.as_u64().map_or(true, |v| v < 1) {
                        return Err(AppError::ParameterValidationFailed {
                            param: key.to_string(),
                            reason: format!("rho[{}] must be >= 1", i),
                        });
                    }
                }
            }
            "validate_t_bls" => {
                let t = value
                    .as_u64()
                    .ok_or_else(|| AppError::ParameterValidationFailed {
                        param: key.to_string(),
                        reason: "t_bls must be a number".to_string(),
                    })?;

                let m = self.get_u64("m").await.unwrap_or(64);

                let min_threshold = (m + 1) / 2; // ⌈m/2⌉
                if t < min_threshold || t > m {
                    return Err(AppError::ParameterValidationFailed {
                        param: key.to_string(),
                        reason: format!("t_bls must be in [{}, {}]", min_threshold, m),
                    });
                }
            }
            "validate_replication_t" => {
                let t = value
                    .as_u64()
                    .ok_or_else(|| AppError::ParameterValidationFailed {
                        param: key.to_string(),
                        reason: "replication_t must be a number".to_string(),
                    })?;

                let r = self.get_u64("replication_r").await.unwrap_or(5);

                if t > r {
                    return Err(AppError::ParameterValidationFailed {
                        param: key.to_string(),
                        reason: format!("replication_t must be <= replication_r ({})", r),
                    });
                }
            }
            _ => {
                warn!(func = func, "Unknown custom validation function");
            }
        }

        Ok(())
    }

    /// Reset parameter to default value
    pub async fn reset_to_default(&self, key: &str) -> Result<()> {
        let meta = self
            .get_metadata(key)
            .await
            .ok_or_else(|| AppError::ParameterNotFound(key.to_string()))?;

        self.apply_governance_change(key, meta.default_value, None)
            .await
    }

    /// Get all parameters in a governance class
    pub async fn get_by_class(&self, governance_class: &str) -> Vec<ParameterMetadata> {
        let metadata = self.metadata.read().await;
        metadata
            .values()
            .filter(|m| m.governance_class == governance_class)
            .cloned()
            .collect()
    }

    /// Save current parameter values to JSON file
    ///
    /// Persists parameters to disk for loading after restart.
    /// Only values are saved (metadata is hardcoded in register_default_parameters).
    pub async fn save_to_file(&self, path: &std::path::Path) -> Result<()> {
        use std::fs::File;
        use std::io::Write;

        let values = self.values.read().await;

        let json = serde_json::to_string_pretty(&*values).map_err(|e| {
            AppError::InvalidConfiguration(format!("Failed to serialize parameters: {}", e))
        })?;

        let mut file = File::create(path).map_err(|e| {
            AppError::InvalidConfiguration(format!("Failed to create parameter file: {}", e))
        })?;

        file.write_all(json.as_bytes()).map_err(|e| {
            AppError::InvalidConfiguration(format!("Failed to write parameters: {}", e))
        })?;

        info!(path = ?path, "💾 Parameter store saved to disk");
        Ok(())
    }

    /// Load parameter values from JSON file
    ///
    /// Loads previously saved parameter values and validates them.
    /// Metadata is not loaded (hardcoded in register_default_parameters).
    pub async fn load_from_file(&self, path: &std::path::Path) -> Result<()> {
        use std::fs::File;
        use std::io::Read;

        let mut file = File::open(path).map_err(|e| {
            AppError::InvalidConfiguration(format!("Failed to open parameter file: {}", e))
        })?;

        let mut contents = String::new();
        file.read_to_string(&mut contents).map_err(|e| {
            AppError::InvalidConfiguration(format!("Failed to read parameter file: {}", e))
        })?;

        let values: HashMap<String, serde_json::Value> =
            serde_json::from_str(&contents).map_err(|e| {
                AppError::InvalidConfiguration(format!("Failed to parse parameters: {}", e))
            })?;

        // Validate all loaded values against metadata
        let metadata = self.metadata.read().await;
        for (key, value) in &values {
            if let Some(meta) = metadata.get(key) {
                self.validate_value(key, value, &meta.validation).await?;
            } else {
                warn!(key = %key, "Ignoring unknown parameter from file");
            }
        }
        drop(metadata);

        // Apply validated values
        let mut current_values = self.values.write().await;
        for (key, value) in values {
            if current_values.contains_key(&key) {
                current_values.insert(key, value);
            }
        }

        info!(path = ?path, "📂 Parameter store loaded from disk");
        Ok(())
    }

    /// Auto-save parameters after governance changes
    ///
    /// If auto_save_path is set, parameters are saved after each change.
    /// This ensures parameters persist across node restarts.
    pub async fn apply_governance_change_with_persistence(
        &self,
        key: &str,
        new_value: serde_json::Value,
        proposal_id: Option<[u8; 32]>,
        persist_path: Option<&std::path::Path>,
    ) -> Result<()> {
        // Apply change normally
        self.apply_governance_change(key, new_value, proposal_id)
            .await?;

        // Persist to disk if path provided
        if let Some(path) = persist_path {
            self.save_to_file(path).await?;
        }

        Ok(())
    }

    /// Build AdicParams from current parameter store values
    ///
    /// Reads all relevant parameters and constructs an AdicParams struct.
    /// This is used for hot-reload to apply governance changes to consensus engines.
    pub async fn build_adic_params(&self) -> Result<adic_types::AdicParams> {
        Ok(adic_types::AdicParams {
            p: self.get_u8("p").await? as u32,
            d: self.get_u8("d").await? as u32,
            rho: self
                .get_vec_u8("rho")
                .await?
                .into_iter()
                .map(|x| x as u32)
                .collect(),
            q: self.get_u8("q").await? as u32,
            k: self.get_u32("k").await?,
            depth_star: self.get_u32("D").await?,
            delta: self.get_u32("Delta").await?,
            deposit: self.get_f64("Rmin").await?,
            r_min: self.get_f64("Rmin").await?,
            r_sum_min: self.get_f64("rmin").await?,
            lambda: self.get_f64("lambda").await?,
            alpha: self.get_f64("alpha").await?,
            beta: self.get_f64("beta").await?,
            mu: self.get_f64("mu").await?,
            gamma: 0.9, // Default gamma value (not in governance parameters)
        })
    }
}

impl Default for ParameterStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parameter_registration() {
        let store = ParameterStore::new();
        assert!(store.get("p").await.is_some());
        assert!(store.get("k").await.is_some());
        assert!(store.get("invalid_key").await.is_none());
    }

    #[tokio::test]
    async fn test_get_typed_values() {
        let store = ParameterStore::new();

        assert_eq!(store.get_u8("p").await.unwrap(), 3);
        assert_eq!(store.get_u32("k").await.unwrap(), 20);
        assert_eq!(store.get_f64("lambda").await.unwrap(), 1.0);

        let rho = store.get_vec_u8("rho").await.unwrap();
        assert_eq!(rho, vec![2, 2, 1]);
    }

    #[tokio::test]
    async fn test_governance_change() {
        let store = ParameterStore::new();

        // Initial value
        assert_eq!(store.get_u32("k").await.unwrap(), 20);

        // Apply governance change
        store
            .apply_governance_change("k", serde_json::json!(25), None)
            .await
            .unwrap();

        // Verify change
        assert_eq!(store.get_u32("k").await.unwrap(), 25);
    }

    #[tokio::test]
    async fn test_change_notification() {
        let store = ParameterStore::new();
        let mut rx = store.subscribe();

        // Apply change in background
        let store_clone = store.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            store_clone
                .apply_governance_change("k", serde_json::json!(30), None)
                .await
                .unwrap();
        });

        // Wait for notification
        let change = tokio::time::timeout(tokio::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(change.key, "k");
        assert_eq!(change.old_value, serde_json::json!(20));
        assert_eq!(change.new_value, serde_json::json!(30));
    }

    #[tokio::test]
    async fn test_validation_in_set() {
        let store = ParameterStore::new();

        // Valid value
        assert!(store
            .apply_governance_change("p", serde_json::json!(5), None)
            .await
            .is_ok());

        // Invalid value
        assert!(store
            .apply_governance_change("p", serde_json::json!(11), None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn test_validation_range() {
        let store = ParameterStore::new();

        // Valid range
        assert!(store
            .apply_governance_change("q", serde_json::json!(3), None)
            .await
            .is_ok());

        // Out of range
        assert!(store
            .apply_governance_change("q", serde_json::json!(10), None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn test_custom_validation_t_bls() {
        let store = ParameterStore::new();

        // Valid: t = ⌈m/2⌉ = 33
        assert!(store
            .apply_governance_change("t_bls", serde_json::json!(33), None)
            .await
            .is_ok());

        // Valid: t = m = 64
        assert!(store
            .apply_governance_change("t_bls", serde_json::json!(64), None)
            .await
            .is_ok());

        // Invalid: t < ⌈m/2⌉
        assert!(store
            .apply_governance_change("t_bls", serde_json::json!(30), None)
            .await
            .is_err());

        // Invalid: t > m
        assert!(store
            .apply_governance_change("t_bls", serde_json::json!(100), None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn test_get_by_class() {
        let store = ParameterStore::new();

        let constitutional = store.get_by_class("Constitutional").await;
        let operational = store.get_by_class("Operational").await;

        assert!(!constitutional.is_empty());
        assert!(!operational.is_empty());

        // p should be constitutional
        assert!(constitutional.iter().any(|p| p.key == "p"));
        // k should be operational
        assert!(operational.iter().any(|p| p.key == "k"));
    }
}
