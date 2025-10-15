use crate::{GovernanceError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{info, warn};

/// Governable parameter with validation rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    pub key: String,
    pub param_type: ParameterType,
    pub governance_class: GovernanceClass,
    pub default_value: serde_json::Value,
    pub validation: ValidationRule,
    pub description: String,
}

/// Parameter data type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParameterType {
    U8,
    U32,
    U64,
    F64,
    VecU8,
    VecU32,
    VecF64,
    TupleF64F64,
}

/// Governance class determines voting threshold
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GovernanceClass {
    /// Requires ≥66.7% supermajority
    Constitutional,
    /// Requires >50% simple majority
    Operational,
}

/// Parameter validation rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidationRule {
    /// Value must be in set
    InSet(Vec<serde_json::Value>),
    /// Value must be in range [min, max]
    Range {
        min: serde_json::Value,
        max: serde_json::Value,
    },
    /// Value must be ≥ min
    GreaterThanOrEqual(serde_json::Value),
    /// Custom validation function name
    Custom(String),
    /// No validation (always valid)
    None,
}

/// Parameter registry managing all governable parameters
/// Based on PoUW III Table 1 (19 parameters)
pub struct ParameterRegistry {
    parameters: HashMap<String, Parameter>,
    current_values: HashMap<String, serde_json::Value>,
}

impl ParameterRegistry {
    /// Create new registry with default ADIC parameters
    pub fn new() -> Self {
        let mut registry = Self {
            parameters: HashMap::new(),
            current_values: HashMap::new(),
        };

        registry.register_default_parameters();
        registry
    }

    /// Register all default ADIC parameters from PoUW III Table 1
    fn register_default_parameters(&mut self) {
        // Constitutional parameters (structural)

        self.register(Parameter {
            key: "p".to_string(),
            param_type: ParameterType::U8,
            governance_class: GovernanceClass::Constitutional,
            default_value: serde_json::json!(3),
            validation: ValidationRule::InSet(vec![
                serde_json::json!(3),
                serde_json::json!(5),
                serde_json::json!(7),
            ]),
            description: "Prime base for p-adic valuation".to_string(),
        });

        self.register(Parameter {
            key: "d".to_string(),
            param_type: ParameterType::U8,
            governance_class: GovernanceClass::Constitutional,
            default_value: serde_json::json!(3),
            validation: ValidationRule::InSet(vec![
                serde_json::json!(3),
                serde_json::json!(4),
            ]),
            description: "Dimension of p-adic feature space".to_string(),
        });

        self.register(Parameter {
            key: "rho".to_string(),
            param_type: ParameterType::VecU8,
            governance_class: GovernanceClass::Constitutional,
            default_value: serde_json::json!([2, 2, 1]),
            validation: ValidationRule::Custom("validate_rho".to_string()),
            description: "Axis radii (must be length d, all ≥1)".to_string(),
        });

        self.register(Parameter {
            key: "q".to_string(),
            param_type: ParameterType::U8,
            governance_class: GovernanceClass::Constitutional,
            default_value: serde_json::json!(3),
            validation: ValidationRule::Range {
                min: serde_json::json!(2),
                max: serde_json::json!(5), // d+1 when d=4
            },
            description: "Diversity requirement (distinct balls per axis)".to_string(),
        });

        // Operational parameters (tunable)

        self.register(Parameter {
            key: "k".to_string(),
            param_type: ParameterType::U32,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(20),
            validation: ValidationRule::GreaterThanOrEqual(serde_json::json!(1)),
            description: "k-core threshold for F1 finality".to_string(),
        });

        self.register(Parameter {
            key: "D".to_string(),
            param_type: ParameterType::U32,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(12),
            validation: ValidationRule::GreaterThanOrEqual(serde_json::json!(1)),
            description: "Depth threshold for F1 finality".to_string(),
        });

        self.register(Parameter {
            key: "Delta".to_string(),
            param_type: ParameterType::U32,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(5),
            validation: ValidationRule::GreaterThanOrEqual(serde_json::json!(1)),
            description: "Window size for finality stabilization".to_string(),
        });

        self.register(Parameter {
            key: "lambda".to_string(),
            param_type: ParameterType::F64,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(1.0),
            validation: ValidationRule::GreaterThanOrEqual(serde_json::json!(0.0)),
            description: "MRW proximity weight".to_string(),
        });

        self.register(Parameter {
            key: "mu".to_string(),
            param_type: ParameterType::F64,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(1.0),
            validation: ValidationRule::GreaterThanOrEqual(serde_json::json!(0.0)),
            description: "MRW trust weight".to_string(),
        });

        self.register(Parameter {
            key: "alpha".to_string(),
            param_type: ParameterType::F64,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(1.0),
            validation: ValidationRule::GreaterThanOrEqual(serde_json::json!(0.0)),
            description: "MRW proximity exponent".to_string(),
        });

        self.register(Parameter {
            key: "beta".to_string(),
            param_type: ParameterType::F64,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(1.0),
            validation: ValidationRule::GreaterThanOrEqual(serde_json::json!(0.0)),
            description: "MRW trust exponent".to_string(),
        });

        self.register(Parameter {
            key: "Rmin".to_string(),
            param_type: ParameterType::F64,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(50.0),
            validation: ValidationRule::GreaterThanOrEqual(serde_json::json!(0.0)),
            description: "Minimum reputation for message submission".to_string(),
        });

        self.register(Parameter {
            key: "rmin".to_string(),
            param_type: ParameterType::F64,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(10.0),
            validation: ValidationRule::GreaterThanOrEqual(serde_json::json!(0.0)),
            description: "Minimum reputation weight for parents".to_string(),
        });

        self.register(Parameter {
            key: "m".to_string(),
            param_type: ParameterType::U32,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(64),
            validation: ValidationRule::GreaterThanOrEqual(serde_json::json!(1)),
            description: "Quorum committee size".to_string(),
        });

        self.register(Parameter {
            key: "t_bls".to_string(),
            param_type: ParameterType::U32,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(43), // ⌈2*64/3⌉
            validation: ValidationRule::Custom("validate_t_bls".to_string()),
            description: "BLS threshold (must be ≥ ⌈m/2⌉, ≤ m)".to_string(),
        });

        self.register(Parameter {
            key: "replication_r".to_string(),
            param_type: ParameterType::U32,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(5),
            validation: ValidationRule::GreaterThanOrEqual(serde_json::json!(1)),
            description: "Storage replication factor".to_string(),
        });

        self.register(Parameter {
            key: "replication_t".to_string(),
            param_type: ParameterType::U32,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(4),
            validation: ValidationRule::Custom("validate_replication_t".to_string()),
            description: "Storage retrieval threshold (must be ≤ r)".to_string(),
        });

        self.register(Parameter {
            key: "challenge_depth".to_string(),
            param_type: ParameterType::U32,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(5),
            validation: ValidationRule::GreaterThanOrEqual(serde_json::json!(1)),
            description: "Challenge window depth in epochs".to_string(),
        });

        self.register(Parameter {
            key: "T_enact".to_string(),
            param_type: ParameterType::F64,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(1000.0),
            validation: ValidationRule::GreaterThanOrEqual(serde_json::json!(0.0)),
            description: "Timelock duration for governance enactment (seconds)".to_string(),
        });

        self.register(Parameter {
            key: "Rmax".to_string(),
            param_type: ParameterType::F64,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(100_000.0),
            validation: ValidationRule::GreaterThanOrEqual(serde_json::json!(1.0)),
            description: "Maximum reputation cap for voting credits".to_string(),
        });

        // F2 Finality Stability Parameters
        self.register(Parameter {
            key: "f2_max_betti_d".to_string(),
            param_type: ParameterType::U32,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(2),
            validation: ValidationRule::Range {
                min: serde_json::json!(1),
                max: serde_json::json!(10),
            },
            description: "Maximum allowed Betti number for H_d stability check in F2 finality"
                .to_string(),
        });

        self.register(Parameter {
            key: "f2_min_persistence".to_string(),
            param_type: ParameterType::F64,
            governance_class: GovernanceClass::Operational,
            default_value: serde_json::json!(0.01),
            validation: ValidationRule::Range {
                min: serde_json::json!(0.001),
                max: serde_json::json!(1.0),
            },
            description:
                "Minimum persistence threshold for noise filtering in F2 finality stability check"
                    .to_string(),
        });

        info!("Registered {} governable parameters", self.parameters.len());
    }

    /// Register a parameter
    fn register(&mut self, param: Parameter) {
        let key = param.key.clone();
        let default = param.default_value.clone();
        self.parameters.insert(key.clone(), param);
        self.current_values.insert(key, default);
    }

    /// Get current value of a parameter
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.current_values.get(key)
    }

    /// Get parameter metadata
    pub fn get_metadata(&self, key: &str) -> Option<&Parameter> {
        self.parameters.get(key)
    }

    /// Validate proposed parameter change
    pub fn validate_change(&self, key: &str, new_value: &serde_json::Value) -> Result<()> {
        let param = self
            .parameters
            .get(key)
            .ok_or_else(|| GovernanceError::InvalidParameter(key.to_string()))?;

        match &param.validation {
            ValidationRule::InSet(allowed) => {
                if !allowed.contains(new_value) {
                    return Err(GovernanceError::ParameterOutOfRange {
                        param: key.to_string(),
                        value: new_value.to_string(),
                        range: format!("one of {:?}", allowed),
                    });
                }
            }
            ValidationRule::Range { min, max } => {
                if !self.value_in_range(new_value, min, max) {
                    return Err(GovernanceError::ParameterOutOfRange {
                        param: key.to_string(),
                        value: new_value.to_string(),
                        range: format!("[{}, {}]", min, max),
                    });
                }
            }
            ValidationRule::GreaterThanOrEqual(min) => {
                if !self.value_gte(new_value, min) {
                    return Err(GovernanceError::ParameterOutOfRange {
                        param: key.to_string(),
                        value: new_value.to_string(),
                        range: format!(">= {}", min),
                    });
                }
            }
            ValidationRule::Custom(func_name) => {
                self.validate_custom(key, new_value, func_name)?;
            }
            ValidationRule::None => {}
        }

        Ok(())
    }

    /// Apply validated parameter change
    pub fn apply_change(&mut self, key: &str, new_value: serde_json::Value) -> Result<()> {
        // Validate first
        self.validate_change(key, &new_value)?;

        // Apply change
        if let Some(current) = self.current_values.get_mut(key) {
            *current = new_value.clone();
            info!(
                key = %key,
                new_value = %new_value,
                "✅ Parameter updated"
            );
            Ok(())
        } else {
            Err(GovernanceError::InvalidParameter(key.to_string()))
        }
    }

    /// Get all parameters in a governance class
    pub fn get_by_class(&self, class: GovernanceClass) -> Vec<&Parameter> {
        self.parameters
            .values()
            .filter(|p| p.governance_class == class)
            .collect()
    }

    /// Get all current parameter values
    pub fn get_all_values(&self) -> &HashMap<String, serde_json::Value> {
        &self.current_values
    }

    // Helper: Check if value is in range
    fn value_in_range(
        &self,
        value: &serde_json::Value,
        min: &serde_json::Value,
        max: &serde_json::Value,
    ) -> bool {
        if let (Some(v), Some(min_v), Some(max_v)) =
            (value.as_f64(), min.as_f64(), max.as_f64())
        {
            v >= min_v && v <= max_v
        } else if let (Some(v), Some(min_v), Some(max_v)) =
            (value.as_u64(), min.as_u64(), max.as_u64())
        {
            v >= min_v && v <= max_v
        } else {
            false
        }
    }

    // Helper: Check if value >= min
    fn value_gte(&self, value: &serde_json::Value, min: &serde_json::Value) -> bool {
        if let (Some(v), Some(min_v)) = (value.as_f64(), min.as_f64()) {
            v >= min_v
        } else if let (Some(v), Some(min_v)) = (value.as_u64(), min.as_u64()) {
            v >= min_v
        } else {
            false
        }
    }

    // Custom validation functions
    fn validate_custom(&self, key: &str, value: &serde_json::Value, func: &str) -> Result<()> {
        match func {
            "validate_rho" => {
                let arr = value.as_array().ok_or_else(|| {
                    GovernanceError::ParameterValidationFailed {
                        param: key.to_string(),
                        reason: "rho must be an array".to_string(),
                    }
                })?;

                let d = self.get("d").and_then(|v| v.as_u64()).unwrap_or(3);

                if arr.len() != d as usize {
                    return Err(GovernanceError::ParameterValidationFailed {
                        param: key.to_string(),
                        reason: format!("rho length must match d={}", d),
                    });
                }

                for (i, r) in arr.iter().enumerate() {
                    if r.as_u64().map_or(true, |v| v < 1) {
                        return Err(GovernanceError::ParameterValidationFailed {
                            param: key.to_string(),
                            reason: format!("rho[{}] must be >= 1", i),
                        });
                    }
                }
            }
            "validate_t_bls" => {
                let t = value.as_u64().ok_or_else(|| {
                    GovernanceError::ParameterValidationFailed {
                        param: key.to_string(),
                        reason: "t_bls must be a number".to_string(),
                    }
                })?;

                let m = self.get("m").and_then(|v| v.as_u64()).unwrap_or(64);

                let min_threshold = (m + 1) / 2; // ⌈m/2⌉
                if t < min_threshold || t > m {
                    return Err(GovernanceError::ParameterValidationFailed {
                        param: key.to_string(),
                        reason: format!("t_bls must be in [{}, {}]", min_threshold, m),
                    });
                }
            }
            "validate_replication_t" => {
                let t = value.as_u64().ok_or_else(|| {
                    GovernanceError::ParameterValidationFailed {
                        param: key.to_string(),
                        reason: "replication_t must be a number".to_string(),
                    }
                })?;

                let r = self
                    .get("replication_r")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(5);

                if t > r {
                    return Err(GovernanceError::ParameterValidationFailed {
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

    /// Save current parameter values to JSON file
    pub fn save_to_file(&self, path: &std::path::Path) -> Result<()> {
        use std::fs::File;
        use std::io::Write;

        let json = serde_json::to_string_pretty(&self.current_values)
            .map_err(|e| GovernanceError::ParameterValidationFailed {
                param: "all".to_string(),
                reason: format!("Failed to serialize parameters: {}", e),
            })?;

        let mut file = File::create(path).map_err(|e| {
            GovernanceError::ParameterValidationFailed {
                param: "all".to_string(),
                reason: format!("Failed to create parameter file: {}", e),
            }
        })?;

        file.write_all(json.as_bytes()).map_err(|e| {
            GovernanceError::ParameterValidationFailed {
                param: "all".to_string(),
                reason: format!("Failed to write parameters: {}", e),
            }
        })?;

        info!(path = ?path, "💾 Parameters saved to disk");
        Ok(())
    }

    /// Load parameter values from JSON file
    pub fn load_from_file(&mut self, path: &std::path::Path) -> Result<()> {
        use std::fs::File;
        use std::io::Read;

        let mut file = File::open(path).map_err(|e| {
            GovernanceError::ParameterValidationFailed {
                param: "all".to_string(),
                reason: format!("Failed to open parameter file: {}", e),
            }
        })?;

        let mut contents = String::new();
        file.read_to_string(&mut contents).map_err(|e| {
            GovernanceError::ParameterValidationFailed {
                param: "all".to_string(),
                reason: format!("Failed to read parameter file: {}", e),
            }
        })?;

        let values: HashMap<String, serde_json::Value> =
            serde_json::from_str(&contents).map_err(|e| {
                GovernanceError::ParameterValidationFailed {
                    param: "all".to_string(),
                    reason: format!("Failed to parse parameters: {}", e),
                }
            })?;

        // Validate all loaded values
        for (key, value) in &values {
            self.validate_change(key, value)?;
        }

        // Apply loaded values
        self.current_values = values;

        info!(path = ?path, "📂 Parameters loaded from disk");
        Ok(())
    }

    /// Export parameters in app-common format
    ///
    /// Maps governance parameters to AppParams structure
    pub fn export_app_params(&self) -> serde_json::Value {
        serde_json::json!({
            "vrf": {
                "min_committer_reputation": self.get("rmin").and_then(|v| v.as_f64()).unwrap_or(10.0),
                "reveal_depth": self.get("challenge_depth").and_then(|v| v.as_u64()).unwrap_or(5),
                "domain_prefix": "ADIC-VRF-v1"
            },
            "quorum": {
                "min_arbitrator_reputation": self.get("rmin").and_then(|v| v.as_f64()).unwrap_or(10.0),
                "members_per_axis": self.get("m").and_then(|v| v.as_u64()).unwrap_or(64) / 4, // Approximate
                "quorum_size": self.get("m").and_then(|v| v.as_u64()).unwrap_or(64),
                "max_per_asn": 2,
                "max_per_region": 3,
                "vote_threshold": 0.66
            },
            "challenge": {
                "window_depth": self.get("challenge_depth").and_then(|v| v.as_u64()).unwrap_or(5),
                "fraud_proof_deposit_factor": 10,
                "dispute_deposit_factor": 5,
                "arbitrator_minority_penalty": -2.0
            },
            "timelock": {
                "f1_grace_period": 3,
                "f2_grace_period": 12,
                "min_timelock_epochs": self.get("T_enact").and_then(|v| v.as_f64()).unwrap_or(1000.0) as u64 / 100
            }
        })
    }
}

impl Default for ParameterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parameter_registration() {
        let registry = ParameterRegistry::new();
        assert!(registry.get("p").is_some());
        assert!(registry.get("k").is_some());
        assert!(registry.get("invalid_key").is_none());
    }

    #[test]
    fn test_parameter_validation_in_set() {
        let registry = ParameterRegistry::new();

        // Valid value
        assert!(registry
            .validate_change("p", &serde_json::json!(3))
            .is_ok());
        assert!(registry
            .validate_change("p", &serde_json::json!(5))
            .is_ok());

        // Invalid value
        assert!(registry
            .validate_change("p", &serde_json::json!(11))
            .is_err());
    }

    #[test]
    fn test_parameter_validation_range() {
        let registry = ParameterRegistry::new();

        // Valid range
        assert!(registry
            .validate_change("q", &serde_json::json!(2))
            .is_ok());
        assert!(registry
            .validate_change("q", &serde_json::json!(4))
            .is_ok());

        // Out of range
        assert!(registry
            .validate_change("q", &serde_json::json!(1))
            .is_err());
        assert!(registry
            .validate_change("q", &serde_json::json!(10))
            .is_err());
    }

    #[test]
    fn test_parameter_apply() {
        let mut registry = ParameterRegistry::new();

        assert_eq!(registry.get("k").unwrap(), &serde_json::json!(20));

        registry
            .apply_change("k", serde_json::json!(30))
            .unwrap();

        assert_eq!(registry.get("k").unwrap(), &serde_json::json!(30));
    }

    #[test]
    fn test_governance_class_filtering() {
        let registry = ParameterRegistry::new();

        let constitutional = registry.get_by_class(GovernanceClass::Constitutional);
        let operational = registry.get_by_class(GovernanceClass::Operational);

        assert!(constitutional.len() > 0);
        assert!(operational.len() > 0);

        // p should be constitutional
        assert!(constitutional.iter().any(|p| p.key == "p"));
        // k should be operational
        assert!(operational.iter().any(|p| p.key == "k"));
    }
}
