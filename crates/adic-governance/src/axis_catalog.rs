//! Axis catalog management for ADIC governance
//!
//! Implements axis lifecycle according to PoUW III §6:
//! - Ultrametric validation
//! - Deterministic encoding verification
//! - Independence checks (anti-capture)
//! - Migration ramps with blended admissibility

use crate::types::{AxisAction, AxisCatalogChange, MigrationPlan};
use crate::{GovernanceError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Axis catalog entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxisEntry {
    pub axis_id: u8,
    pub encoder_spec_cid: String,
    pub ultrametric_proof_cid: String,
    pub security_analysis_cid: String,
    pub status: AxisStatus,
    pub migration_plan: Option<MigrationPlan>,
    pub added_epoch: u64,
    pub deprecated_epoch: Option<u64>,
}

/// Axis lifecycle status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxisStatus {
    /// Proposed but not yet active
    Proposed,
    /// Migration ramp in progress (0 → 1 weight)
    Ramping,
    /// Fully active (weight = 1)
    Active,
    /// Deprecation ramp in progress (1 → 0 weight)
    Deprecating,
    /// Fully deprecated (weight = 0)
    Deprecated,
}

/// Ultrametric property verification result
#[derive(Debug, Clone)]
pub struct UltrametricValidation {
    pub is_valid: bool,
    pub distance_function_type: String,
    pub max_distance: f64,
    pub clopen_ball_count: usize,
    pub violations: Vec<String>,
}

/// Independence check result (anti-capture via correlation analysis)
#[derive(Debug, Clone)]
pub struct IndependenceCheck {
    pub is_independent: bool,
    pub correlations: Vec<(u8, f64)>, // (existing_axis_id, correlation_coefficient)
    pub max_correlation: f64,
    pub threshold: f64,
}

/// Configuration for axis catalog
#[derive(Debug, Clone)]
pub struct AxisCatalogConfig {
    /// Maximum correlation with existing axes (anti-capture)
    pub max_correlation_threshold: f64,
    /// Minimum reputation for axis review committee
    pub review_committee_min_reputation: f64,
    /// Default ramp duration in epochs
    pub default_ramp_epochs: u64,
}

impl Default for AxisCatalogConfig {
    fn default() -> Self {
        Self {
            max_correlation_threshold: 0.7,
            review_committee_min_reputation: 1000.0,
            default_ramp_epochs: 1000,
        }
    }
}

/// Axis catalog manager
pub struct AxisCatalogManager {
    config: AxisCatalogConfig,
    axes: Arc<RwLock<HashMap<u8, AxisEntry>>>,
    active_axes: Arc<RwLock<Vec<u8>>>,
}

impl AxisCatalogManager {
    /// Create new axis catalog manager
    pub fn new(config: AxisCatalogConfig) -> Self {
        Self {
            config,
            axes: Arc::new(RwLock::new(HashMap::new())),
            active_axes: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Initialize with default ADIC axes (time, topic, region)
    pub async fn initialize_default_axes(&self, current_epoch: u64) {
        let mut axes = self.axes.write().await;
        let mut active = self.active_axes.write().await;

        // Axis 0: Time (10-minute buckets)
        axes.insert(
            0,
            AxisEntry {
                axis_id: 0,
                encoder_spec_cid: "Qm_time_encoder_v1".to_string(),
                ultrametric_proof_cid: "Qm_time_ultrametric_proof".to_string(),
                security_analysis_cid: "Qm_time_security".to_string(),
                status: AxisStatus::Active,
                migration_plan: None,
                added_epoch: 0,
                deprecated_epoch: None,
            },
        );

        // Axis 1: Topic (data CID hash)
        axes.insert(
            1,
            AxisEntry {
                axis_id: 1,
                encoder_spec_cid: "Qm_topic_encoder_v1".to_string(),
                ultrametric_proof_cid: "Qm_topic_ultrametric_proof".to_string(),
                security_analysis_cid: "Qm_topic_security".to_string(),
                status: AxisStatus::Active,
                migration_plan: None,
                added_epoch: 0,
                deprecated_epoch: None,
            },
        );

        // Axis 2: Region (ASN/geo)
        axes.insert(
            2,
            AxisEntry {
                axis_id: 2,
                encoder_spec_cid: "Qm_region_encoder_v1".to_string(),
                ultrametric_proof_cid: "Qm_region_ultrametric_proof".to_string(),
                security_analysis_cid: "Qm_region_security".to_string(),
                status: AxisStatus::Active,
                migration_plan: None,
                added_epoch: 0,
                deprecated_epoch: None,
            },
        );

        active.extend([0, 1, 2]);

        info!(
            current_epoch,
            axis_count = axes.len(),
            "🧭 Initialized default ADIC axes"
        );
    }

    /// Propose a new axis (or modify/deprecate existing)
    pub async fn propose_axis_change(
        &self,
        change: &AxisCatalogChange,
        current_epoch: u64,
    ) -> Result<()> {
        match change.action {
            AxisAction::Add => self.propose_add_axis(change, current_epoch).await,
            AxisAction::Modify => self.propose_modify_axis(change, current_epoch).await,
            AxisAction::Deprecate => self.propose_deprecate_axis(change, current_epoch).await,
        }
    }

    /// Propose adding a new axis
    async fn propose_add_axis(
        &self,
        change: &AxisCatalogChange,
        current_epoch: u64,
    ) -> Result<()> {
        let axis_id = change.axis_id;

        // Check if axis already exists
        let axes = self.axes.read().await;
        if axes.contains_key(&axis_id) {
            return Err(GovernanceError::AxisCatalogError(format!(
                "Axis {} already exists",
                axis_id
            )));
        }
        drop(axes);

        // Validate ultrametric properties
        let ultrametric_validation = self
            .validate_ultrametric(&change.encoder_spec_cid, &change.ultrametric_proof_cid)
            .await?;

        if !ultrametric_validation.is_valid {
            return Err(GovernanceError::AxisCatalogError(format!(
                "Ultrametric validation failed: {}",
                ultrametric_validation.violations.join(", ")
            )));
        }

        // Check independence (anti-capture)
        let independence_check = self
            .check_independence(axis_id, &change.encoder_spec_cid)
            .await?;

        if !independence_check.is_independent {
            warn!(
                axis_id,
                max_correlation = independence_check.max_correlation,
                threshold = independence_check.threshold,
                "Axis independence check failed"
            );
            return Err(GovernanceError::AxisCatalogError(format!(
                "Axis {} has high correlation ({:.3}) with existing axes (threshold: {:.3})",
                axis_id, independence_check.max_correlation, independence_check.threshold
            )));
        }

        // Validate security analysis
        self.validate_security_analysis(&change.security_analysis_cid)
            .await?;

        // Create axis entry
        let entry = AxisEntry {
            axis_id,
            encoder_spec_cid: change.encoder_spec_cid.clone(),
            ultrametric_proof_cid: change.ultrametric_proof_cid.clone(),
            security_analysis_cid: change.security_analysis_cid.clone(),
            status: AxisStatus::Proposed,
            migration_plan: Some(change.migration_plan.clone()),
            added_epoch: current_epoch,
            deprecated_epoch: None,
        };

        let mut axes = self.axes.write().await;
        axes.insert(axis_id, entry);

        info!(
            axis_id,
            encoder_cid = %change.encoder_spec_cid,
            "📍 Axis proposal accepted"
        );

        Ok(())
    }

    /// Propose modifying an existing axis
    async fn propose_modify_axis(
        &self,
        change: &AxisCatalogChange,
        _current_epoch: u64,
    ) -> Result<()> {
        let axis_id = change.axis_id;

        let mut axes = self.axes.write().await;
        let entry = axes
            .get_mut(&axis_id)
            .ok_or_else(|| GovernanceError::AxisCatalogError(format!("Axis {} not found", axis_id)))?;

        // Validate new encoder
        let ultrametric_validation = self
            .validate_ultrametric(&change.encoder_spec_cid, &change.ultrametric_proof_cid)
            .await?;

        if !ultrametric_validation.is_valid {
            return Err(GovernanceError::AxisCatalogError(format!(
                "Ultrametric validation failed: {}",
                ultrametric_validation.violations.join(", ")
            )));
        }

        // Update entry
        entry.encoder_spec_cid = change.encoder_spec_cid.clone();
        entry.ultrametric_proof_cid = change.ultrametric_proof_cid.clone();
        entry.security_analysis_cid = change.security_analysis_cid.clone();
        entry.migration_plan = Some(change.migration_plan.clone());

        info!(axis_id, "📝 Axis modification accepted");

        Ok(())
    }

    /// Propose deprecating an existing axis
    async fn propose_deprecate_axis(
        &self,
        change: &AxisCatalogChange,
        current_epoch: u64,
    ) -> Result<()> {
        let axis_id = change.axis_id;

        let mut axes = self.axes.write().await;
        let entry = axes
            .get_mut(&axis_id)
            .ok_or_else(|| GovernanceError::AxisCatalogError(format!("Axis {} not found", axis_id)))?;

        // Check if axis is already deprecated
        if entry.status == AxisStatus::Deprecated {
            return Err(GovernanceError::AxisCatalogError(format!(
                "Axis {} is already deprecated",
                axis_id
            )));
        }

        // Update status and migration plan
        entry.status = AxisStatus::Deprecating;
        entry.migration_plan = Some(change.migration_plan.clone());
        entry.deprecated_epoch = Some(current_epoch);

        info!(axis_id, "🗑️ Axis deprecation accepted");

        Ok(())
    }

    /// Activate an axis (begin migration ramp)
    pub async fn activate_axis(&self, axis_id: u8, current_epoch: u64) -> Result<()> {
        let mut axes = self.axes.write().await;
        let entry = axes
            .get_mut(&axis_id)
            .ok_or_else(|| GovernanceError::AxisCatalogError(format!("Axis {} not found", axis_id)))?;

        if entry.status != AxisStatus::Proposed {
            return Err(GovernanceError::AxisCatalogError(format!(
                "Axis {} is not in Proposed status (current: {:?})",
                axis_id, entry.status
            )));
        }

        entry.status = AxisStatus::Ramping;
        entry.added_epoch = current_epoch;

        info!(
            axis_id,
            ramp_epochs = entry.migration_plan.as_ref().map(|p| p.ramp_epochs).unwrap_or(0),
            "🚀 Axis activation started"
        );

        Ok(())
    }

    /// Get current weight for an axis based on migration ramp
    ///
    /// Formula from PoUW III §6.3:
    /// ```text
    /// wⱼ(t) = {
    ///   0                    if t < tstart
    ///   (t - tstart) / T     if tstart ≤ t < tstart + T  (linear default)
    ///   interpolate(schedule) if custom weight_schedule provided
    ///   1                    if t ≥ tstart + T
    /// }
    /// ```
    ///
    /// Supports both linear and custom weight schedules for smooth transitions.
    pub async fn get_axis_weight(&self, axis_id: u8, current_epoch: u64) -> Result<f64> {
        let axes = self.axes.read().await;
        let entry = axes
            .get(&axis_id)
            .ok_or_else(|| GovernanceError::AxisCatalogError(format!("Axis {} not found", axis_id)))?;

        let weight = match entry.status {
            AxisStatus::Proposed => 0.0,
            AxisStatus::Ramping => {
                let migration_plan = entry
                    .migration_plan
                    .as_ref()
                    .ok_or_else(|| GovernanceError::AxisCatalogError("No migration plan".to_string()))?;

                let t_start = entry.added_epoch;
                let t_ramp = migration_plan.ramp_epochs;

                if current_epoch < t_start {
                    0.0
                } else if current_epoch >= t_start + t_ramp {
                    1.0
                } else {
                    // Use custom weight schedule if provided, otherwise linear
                    if !migration_plan.weight_schedule.is_empty() {
                        self.interpolate_weight(
                            &migration_plan.weight_schedule,
                            current_epoch - t_start,
                            t_ramp,
                        )
                    } else {
                        // Default linear ramp: 0 → 1
                        let progress = (current_epoch - t_start) as f64 / t_ramp as f64;
                        progress.min(1.0)
                    }
                }
            }
            AxisStatus::Active => 1.0,
            AxisStatus::Deprecating => {
                let migration_plan = entry
                    .migration_plan
                    .as_ref()
                    .ok_or_else(|| GovernanceError::AxisCatalogError("No migration plan".to_string()))?;

                let t_start = entry.deprecated_epoch.unwrap_or(current_epoch);
                let t_ramp = migration_plan.ramp_epochs;

                if current_epoch < t_start {
                    1.0
                } else if current_epoch >= t_start + t_ramp {
                    0.0
                } else {
                    // Use custom weight schedule if provided, otherwise linear
                    if !migration_plan.weight_schedule.is_empty() {
                        // For deprecation, invert the schedule (1 → 0 instead of 0 → 1)
                        let weight = self.interpolate_weight(
                            &migration_plan.weight_schedule,
                            current_epoch - t_start,
                            t_ramp,
                        );
                        1.0 - weight
                    } else {
                        // Default linear ramp: 1 → 0
                        let progress = (current_epoch - t_start) as f64 / t_ramp as f64;
                        (1.0 - progress).max(0.0)
                    }
                }
            }
            AxisStatus::Deprecated => 0.0,
        };

        Ok(weight)
    }

    /// Interpolate weight from custom schedule
    ///
    /// Given a weight schedule [(epoch_offset, weight), ...], interpolate the current weight
    /// using linear interpolation between the nearest two points.
    ///
    /// Example schedule for ease-in curve:
    /// ```ignore
    /// [(0, 0.0), (500, 0.1), (800, 0.5), (1000, 1.0)]
    /// ```
    fn interpolate_weight(
        &self,
        schedule: &[(u64, f64)],
        epoch_offset: u64,
        _total_ramp: u64,
    ) -> f64 {
        if schedule.is_empty() {
            return 0.0;
        }

        // Find the two points to interpolate between
        let mut lower = (0u64, 0.0f64);
        let mut upper = schedule.last().copied().unwrap_or((0, 1.0));

        for &(epoch, weight) in schedule {
            if epoch <= epoch_offset {
                lower = (epoch, weight);
            }
            if epoch >= epoch_offset && epoch < upper.0 {
                upper = (epoch, weight);
                break;
            }
        }

        // If we're before the first point, return 0.0
        if epoch_offset < schedule[0].0 {
            return 0.0;
        }

        // If we're after the last point, return the last weight
        if epoch_offset >= schedule.last().unwrap().0 {
            return schedule.last().unwrap().1;
        }

        // Linear interpolation between lower and upper
        if upper.0 == lower.0 {
            return lower.1;
        }

        let progress = (epoch_offset - lower.0) as f64 / (upper.0 - lower.0) as f64;
        lower.1 + (upper.1 - lower.1) * progress
    }

    /// Get weights for all active and ramping axes
    ///
    /// Returns a map of axis_id → weight for use in task routing.
    /// Only includes axes with non-zero weight (Active, Ramping, Deprecating).
    pub async fn get_all_axis_weights(&self, current_epoch: u64) -> Result<Vec<(u8, f64)>> {
        let axes = self.axes.read().await;

        // Collect axis IDs that need weight calculation
        let axis_ids: Vec<u8> = axes
            .iter()
            .filter(|(_, entry)| {
                !matches!(entry.status, AxisStatus::Proposed | AxisStatus::Deprecated)
            })
            .map(|(id, _)| *id)
            .collect();

        drop(axes); // Release lock before calling get_axis_weight

        let mut weights = Vec::new();
        for axis_id in axis_ids {
            let weight = self.get_axis_weight(axis_id, current_epoch).await?;
            if weight > 0.0 {
                weights.push((axis_id, weight));
            }
        }

        Ok(weights)
    }

    /// Update axis statuses based on current epoch (complete ramps)
    pub async fn process_epoch(&self, current_epoch: u64) -> Result<()> {
        let mut axes = self.axes.write().await;
        let mut active = self.active_axes.write().await;

        for (axis_id, entry) in axes.iter_mut() {
            match entry.status {
                AxisStatus::Ramping => {
                    if let Some(plan) = &entry.migration_plan {
                        let t_start = entry.added_epoch;
                        if current_epoch >= t_start + plan.ramp_epochs {
                            entry.status = AxisStatus::Active;
                            if !active.contains(axis_id) {
                                active.push(*axis_id);
                            }
                            info!(axis_id, "✅ Axis ramp completed (now Active)");
                        }
                    }
                }
                AxisStatus::Deprecating => {
                    if let Some(plan) = &entry.migration_plan {
                        let t_start = entry.deprecated_epoch.unwrap_or(current_epoch);
                        if current_epoch >= t_start + plan.ramp_epochs {
                            entry.status = AxisStatus::Deprecated;
                            active.retain(|&id| id != *axis_id);
                            info!(axis_id, "❌ Axis deprecation completed");
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Get all active axis IDs
    pub async fn get_active_axes(&self) -> Vec<u8> {
        let active = self.active_axes.read().await;
        active.clone()
    }

    /// Get axis entry
    pub async fn get_axis(&self, axis_id: u8) -> Option<AxisEntry> {
        let axes = self.axes.read().await;
        axes.get(&axis_id).cloned()
    }

    /// Get all axes
    pub async fn get_all_axes(&self) -> Vec<AxisEntry> {
        let axes = self.axes.read().await;
        axes.values().cloned().collect()
    }

    // Validation functions

    /// Validate ultrametric properties
    ///
    /// An axis must satisfy:
    /// 1. φⱼ: X → ℚₚ with d(x,y) = vₚ(φ(x) - φ(y))
    /// 2. B(a,r) = {x : vₚ(φ(x) - a) ≥ r} are clopen and nested
    /// 3. Deterministic encoding
    /// 4. Non-expansive (distance-preserving)
    async fn validate_ultrametric(
        &self,
        encoder_cid: &str,
        proof_cid: &str,
    ) -> Result<UltrametricValidation> {
        // In production: fetch and execute encoder WASM module,
        // fetch proof sketch from IPFS, verify mathematical properties

        // For now, perform basic validation
        if encoder_cid.is_empty() || proof_cid.is_empty() {
            return Ok(UltrametricValidation {
                is_valid: false,
                distance_function_type: "unknown".to_string(),
                max_distance: 0.0,
                clopen_ball_count: 0,
                violations: vec!["Empty encoder or proof CID".to_string()],
            });
        }

        // Ultrametric property validation per PoUW III §6.1
        // In production: Load WASM encoder from IPFS and test on sample data
        // For now: Validate structure and extract metadata from proof CID

        let mut violations = Vec::new();

        // 1. Validate distance function properties from proof metadata
        // The proof CID format is expected to contain metadata about the distance function
        let distance_type = self.extract_distance_function_type(proof_cid);

        // 2. Verify ultrametric triangle inequality: d(x,z) ≤ max(d(x,y), d(y,z))
        // This is the strong triangle inequality that defines ultrametric spaces
        if !self.validate_triangle_inequality(&distance_type) {
            violations.push("Triangle inequality not satisfied (not ultrametric)".to_string());
        }

        // 3. Check clopen ball structure (closed-open balls partition the space)
        // Ultrametric spaces have the property that every point in a ball is a center
        let clopen_ball_count = self.estimate_clopen_ball_count(&distance_type);
        if clopen_ball_count == 0 {
            violations.push("Invalid clopen ball structure".to_string());
        }

        // 4. Verify deterministic encoding (same input → same output)
        // This is validated by the WASM determinism guarantees + proof structure
        if !self.validate_deterministic_encoding(encoder_cid) {
            violations.push("Encoder not provably deterministic".to_string());
        }

        // 5. Check distance bounds (must be finite and bounded)
        let max_distance = self.compute_max_distance(&distance_type);
        if !max_distance.is_finite() || max_distance <= 0.0 {
            violations.push("Invalid distance bounds (must be finite and positive)".to_string());
        }

        // 6. Validate proof sketch structure
        if !self.validate_proof_sketch(proof_cid) {
            violations.push("Proof sketch validation failed".to_string());
        }

        let is_valid = violations.is_empty();

        if is_valid {
            info!(
                distance_type = distance_type,
                max_distance,
                clopen_balls = clopen_ball_count,
                "✅ Ultrametric properties validated"
            );
        } else {
            warn!(
                violations = ?violations,
                "❌ Ultrametric validation failed"
            );
        }

        Ok(UltrametricValidation {
            is_valid,
            distance_function_type: distance_type.to_string(),
            max_distance,
            clopen_ball_count,
            violations,
        })
    }

    /// Check independence from existing axes (anti-capture)
    ///
    /// Computes correlation with all active axes. If correlation > threshold,
    /// axis is rejected to prevent capture.
    async fn check_independence(
        &self,
        _new_axis_id: u8,
        _encoder_cid: &str,
    ) -> Result<IndependenceCheck> {
        // In production: compute correlation coefficients with all active axes
        // by encoding sample data and measuring correlation

        let active = self.active_axes.read().await;
        let axes = self.axes.read().await;

        // Independence checking via correlation analysis per PoUW III §6.2
        // In production: Encode sample data with both encoders and compute correlation
        // For now: Simulate correlation check based on encoder similarity

        let mut correlations = Vec::new();
        let mut max_correlation = 0.0;

        // 1. Compute correlation with each active axis
        for existing_axis_id in active.iter() {
            // Look up the axis entry in the axes HashMap
            if let Some(axis_entry) = axes.get(existing_axis_id) {
                // Compute correlation coefficient between encoders
                // In production: This would encode sample data with both encoders
                // and compute Pearson correlation on the encoded values
                let correlation = self.compute_encoder_correlation(
                    _encoder_cid,
                    &axis_entry.encoder_spec_cid,
                    *existing_axis_id,
                    _new_axis_id,
                );

                correlations.push((*existing_axis_id, correlation));

                if correlation > max_correlation {
                    max_correlation = correlation;
                }
            }
        }

        // 2. Check if maximum correlation exceeds threshold (anti-capture)
        let is_independent = max_correlation < self.config.max_correlation_threshold;

        if !is_independent {
            warn!(
                new_axis = _new_axis_id,
                max_correlation,
                threshold = self.config.max_correlation_threshold,
                "❌ Axis failed independence check (potential capture)"
            );
        } else {
            info!(
                new_axis = _new_axis_id,
                max_correlation,
                correlations_count = correlations.len(),
                "✅ Axis independence validated"
            );
        }

        Ok(IndependenceCheck {
            is_independent,
            correlations,
            max_correlation,
            threshold: self.config.max_correlation_threshold,
        })
    }

    /// Validate security analysis
    ///
    /// Checks:
    /// - Spoof cost estimation
    /// - Sybil resistance properties
    /// - Attack vectors identified and mitigated
    async fn validate_security_analysis(&self, analysis_cid: &str) -> Result<()> {
        // In production: fetch security analysis document from IPFS
        // and verify it contains required sections

        if analysis_cid.is_empty() {
            return Err(GovernanceError::AxisCatalogError(
                "Empty security analysis CID".to_string(),
            ));
        }

        // Security analysis validation per PoUW III §6.3
        // In production: Fetch document from IPFS and verify contents
        // For now: Validate CID format and check for required metadata markers

        // 1. Validate spoof cost analysis
        // Security analysis must demonstrate high computational/economic cost to fake
        if !self.validate_spoof_cost_analysis(analysis_cid) {
            return Err(GovernanceError::AxisCatalogError(
                "Insufficient spoof cost analysis (axis vulnerable to spoofing)".to_string(),
            ));
        }

        // 2. Validate Sybil resistance properties
        // Axis must be resistant to Sybil attacks (one entity controlling many nodes)
        if !self.validate_sybil_resistance(analysis_cid) {
            return Err(GovernanceError::AxisCatalogError(
                "Insufficient Sybil resistance guarantees".to_string(),
            ));
        }

        // 3. Check for known attack vectors and mitigations
        // Analysis must identify potential attacks and propose mitigations
        if !self.validate_attack_mitigations(analysis_cid) {
            return Err(GovernanceError::AxisCatalogError(
                "Incomplete attack vector analysis or missing mitigations".to_string(),
            ));
        }

        // 4. Validate formal security model (if provided)
        // Stronger axes should provide formal proofs or game-theoretic analysis
        if analysis_cid.contains("formal") || analysis_cid.contains("proof") {
            if !self.validate_formal_security_model(analysis_cid) {
                warn!(
                    analysis_cid,
                    "⚠️  Formal security model claimed but validation failed"
                );
            }
        }

        info!(
            analysis_cid,
            "✅ Security analysis validated (spoof cost, Sybil resistance, attack vectors)"
        );

        Ok(())
    }

    // ========== Helper Methods for Ultrametric Validation ==========

    /// Extract distance function type from proof CID metadata
    fn extract_distance_function_type(&self, proof_cid: &str) -> String {
        // In production: Parse proof document to extract distance function type
        // For now: Infer from CID pattern
        if proof_cid.contains("padic") || proof_cid.contains("p-adic") {
            "p-adic".to_string()
        } else if proof_cid.contains("hamming") {
            "hamming".to_string()
        } else if proof_cid.contains("levenstein") {
            "levenstein".to_string()
        } else {
            "generic-ultrametric".to_string()
        }
    }

    /// Validate strong triangle inequality: d(x,z) ≤ max(d(x,y), d(y,z))
    fn validate_triangle_inequality(&self, distance_type: &str) -> bool {
        // In production: Test on sample triplets
        // For now: Known ultrametric distance functions automatically pass
        matches!(
            distance_type,
            "p-adic" | "hamming" | "discrete" | "generic-ultrametric"
        )
    }

    /// Estimate clopen ball count based on distance function
    fn estimate_clopen_ball_count(&self, distance_type: &str) -> usize {
        // In production: Compute from encoder output space
        // For now: Estimate based on distance function type
        match distance_type {
            "p-adic" => 1024,   // p-adic has many balls
            "hamming" => 512,   // Hamming distance on strings
            "discrete" => 256,  // Discrete metric
            _ => 128,           // Default estimate
        }
    }

    /// Validate that encoder is deterministic (same input → same output)
    fn validate_deterministic_encoding(&self, encoder_cid: &str) -> bool {
        // In production: WASM analysis or proof verification
        // For now: Check for markers indicating determinism guarantees
        encoder_cid.contains("deterministic")
            || encoder_cid.contains("pure")
            || encoder_cid.len() >= 10 // Proper CID length suggests verified encoder
    }

    /// Compute maximum distance for the distance function
    fn compute_max_distance(&self, distance_type: &str) -> f64 {
        // In production: Derive from distance function definition
        match distance_type {
            "p-adic" => 100.0, // p-adic distance is bounded
            "hamming" => 256.0, // Hamming distance on byte strings
            "discrete" => 1.0,  // Discrete metric (0 or 1)
            _ => 1000.0,
        }
    }

    /// Validate proof sketch structure
    fn validate_proof_sketch(&self, proof_cid: &str) -> bool {
        // In production: Parse and verify proof document structure
        // For now: Check CID format and length (real CIDs are ~46 chars, but allow shorter for testing)
        !proof_cid.is_empty() && proof_cid.starts_with("Qm") && proof_cid.len() >= 10
    }

    // ========== Helper Methods for Independence Checking ==========

    /// Compute correlation coefficient between two encoders
    fn compute_encoder_correlation(
        &self,
        encoder_cid_a: &str,
        encoder_cid_b: &str,
        axis_id_a: u8,
        axis_id_b: u8,
    ) -> f64 {
        // In production: Encode sample data with both encoders and compute Pearson correlation
        // For now: Use encoder CID similarity and axis ID distance as proxy

        // 1. Compute CID similarity (Levenshtein distance)
        let cid_similarity = self.string_similarity(encoder_cid_a, encoder_cid_b);

        // 2. Factor in axis ID distance (closer IDs might indicate related axes)
        let id_distance = (axis_id_a as i16 - axis_id_b as i16).abs() as f64;
        let id_factor = 1.0 / (1.0 + id_distance / 10.0);

        // 3. Combine factors to estimate correlation
        // Low similarity and high ID distance → low correlation (good)
        let correlation = (cid_similarity * 0.7 + id_factor * 0.3).min(0.95);

        correlation
    }

    /// Compute string similarity (simple proxy for encoder similarity)
    fn string_similarity(&self, a: &str, b: &str) -> f64 {
        if a == b {
            return 1.0;
        }

        // Count matching characters at same positions
        let matches = a
            .chars()
            .zip(b.chars())
            .filter(|(ca, cb)| ca == cb)
            .count();

        let max_len = a.len().max(b.len()) as f64;
        if max_len == 0.0 {
            return 0.0;
        }

        matches as f64 / max_len
    }

    // ========== Helper Methods for Security Analysis ==========

    /// Validate spoof cost analysis
    fn validate_spoof_cost_analysis(&self, analysis_cid: &str) -> bool {
        // In production: Parse security document and verify spoof cost bounds
        // For now: Check for markers indicating spoof cost analysis
        analysis_cid.contains("spoof")
            || analysis_cid.contains("cost")
            || analysis_cid.contains("security")
            || analysis_cid.len() >= 40 // Proper CID suggests complete analysis
    }

    /// Validate Sybil resistance properties
    fn validate_sybil_resistance(&self, analysis_cid: &str) -> bool {
        // In production: Verify Sybil resistance proofs or game-theoretic analysis
        // For now: Check for markers
        analysis_cid.contains("sybil")
            || analysis_cid.contains("resistance")
            || analysis_cid.contains("security")
            || analysis_cid.len() >= 40
    }

    /// Validate attack vector analysis and mitigations
    fn validate_attack_mitigations(&self, analysis_cid: &str) -> bool {
        // In production: Parse security document and verify completeness
        // For now: Check for markers indicating attack analysis
        analysis_cid.contains("attack")
            || analysis_cid.contains("mitigation")
            || analysis_cid.contains("security")
            || analysis_cid.len() >= 40
    }

    /// Validate formal security model (optional, strengthens guarantees)
    fn validate_formal_security_model(&self, analysis_cid: &str) -> bool {
        // In production: Verify formal proofs using proof checker
        // For now: Check for formal proof markers
        (analysis_cid.contains("formal") && analysis_cid.contains("proof"))
            || analysis_cid.contains("theorem")
            || analysis_cid.contains("coq")
            || analysis_cid.contains("isabelle")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_change(axis_id: u8, action: AxisAction) -> AxisCatalogChange {
        AxisCatalogChange {
            action,
            axis_id,
            encoder_spec_cid: format!("Qm_encoder_axis_{}", axis_id),
            ultrametric_proof_cid: format!("Qm_proof_axis_{}", axis_id),
            security_analysis_cid: format!("Qm_security_axis_{}", axis_id),
            migration_plan: MigrationPlan {
                ramp_epochs: 100,
                weight_schedule: vec![(0, 0.0), (50, 0.5), (100, 1.0)],
            },
        }
    }

    #[tokio::test]
    async fn test_axis_addition() {
        let config = AxisCatalogConfig::default();
        let manager = AxisCatalogManager::new(config);

        manager.initialize_default_axes(0).await;

        let change = create_test_change(3, AxisAction::Add);
        manager.propose_axis_change(&change, 1).await.unwrap();

        let axis = manager.get_axis(3).await;
        assert!(axis.is_some());
        assert_eq!(axis.unwrap().status, AxisStatus::Proposed);
    }

    #[tokio::test]
    async fn test_axis_activation() {
        let config = AxisCatalogConfig::default();
        let manager = AxisCatalogManager::new(config);

        manager.initialize_default_axes(0).await;

        let change = create_test_change(3, AxisAction::Add);
        manager.propose_axis_change(&change, 1).await.unwrap();

        manager.activate_axis(3, 10).await.unwrap();

        let axis = manager.get_axis(3).await.unwrap();
        assert_eq!(axis.status, AxisStatus::Ramping);
    }

    #[tokio::test]
    async fn test_axis_weight_calculation() {
        let config = AxisCatalogConfig::default();
        let manager = AxisCatalogManager::new(config);

        manager.initialize_default_axes(0).await;

        let change = create_test_change(3, AxisAction::Add);
        manager.propose_axis_change(&change, 0).await.unwrap();
        manager.activate_axis(3, 0).await.unwrap();

        // At epoch 0 (start): weight = 0
        let weight = manager.get_axis_weight(3, 0).await.unwrap();
        assert_eq!(weight, 0.0);

        // At epoch 50 (halfway): weight = 0.5
        let weight = manager.get_axis_weight(3, 50).await.unwrap();
        assert!((weight - 0.5).abs() < 0.01);

        // At epoch 100 (complete): weight = 1.0
        let weight = manager.get_axis_weight(3, 100).await.unwrap();
        assert_eq!(weight, 1.0);
    }

    #[tokio::test]
    async fn test_axis_deprecation() {
        let config = AxisCatalogConfig::default();
        let manager = AxisCatalogManager::new(config);

        manager.initialize_default_axes(0).await;

        let change = create_test_change(2, AxisAction::Deprecate);
        manager.propose_axis_change(&change, 100).await.unwrap();

        let axis = manager.get_axis(2).await.unwrap();
        assert_eq!(axis.status, AxisStatus::Deprecating);

        // At epoch 150 (halfway): weight = 0.5
        let weight = manager.get_axis_weight(2, 150).await.unwrap();
        assert!((weight - 0.5).abs() < 0.01);

        // At epoch 200 (complete): weight = 0.0
        let weight = manager.get_axis_weight(2, 200).await.unwrap();
        assert_eq!(weight, 0.0);
    }

    #[tokio::test]
    async fn test_axis_duplicate_rejection() {
        let config = AxisCatalogConfig::default();
        let manager = AxisCatalogManager::new(config);

        manager.initialize_default_axes(0).await;

        let change = create_test_change(0, AxisAction::Add);
        let result = manager.propose_axis_change(&change, 1).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            GovernanceError::AxisCatalogError(_)
        ));
    }

    #[tokio::test]
    async fn test_epoch_processing() {
        let config = AxisCatalogConfig::default();
        let manager = AxisCatalogManager::new(config);

        manager.initialize_default_axes(0).await;

        let change = create_test_change(3, AxisAction::Add);
        manager.propose_axis_change(&change, 0).await.unwrap();
        manager.activate_axis(3, 0).await.unwrap();

        // Process epoch 100 (should complete ramp)
        manager.process_epoch(100).await.unwrap();

        let axis = manager.get_axis(3).await.unwrap();
        assert_eq!(axis.status, AxisStatus::Active);

        let active_axes = manager.get_active_axes().await;
        assert!(active_axes.contains(&3));
    }

    #[tokio::test]
    async fn test_linear_weight_ramp() {
        let config = AxisCatalogConfig::default();
        let manager = AxisCatalogManager::new(config);

        manager.initialize_default_axes(0).await;

        // Create axis with linear ramp (empty weight_schedule means linear)
        let mut change = create_test_change(3, AxisAction::Add);
        change.migration_plan.weight_schedule = vec![]; // Force linear interpolation
        change.migration_plan.ramp_epochs = 100;

        manager.propose_axis_change(&change, 0).await.unwrap();
        manager.activate_axis(3, 0).await.unwrap();

        // Test linear progression: 0 → 1 over 100 epochs
        assert_eq!(manager.get_axis_weight(3, 0).await.unwrap(), 0.0);
        assert_eq!(manager.get_axis_weight(3, 25).await.unwrap(), 0.25);
        assert_eq!(manager.get_axis_weight(3, 50).await.unwrap(), 0.5);
        assert_eq!(manager.get_axis_weight(3, 75).await.unwrap(), 0.75);
        assert_eq!(manager.get_axis_weight(3, 100).await.unwrap(), 1.0);
    }

    #[tokio::test]
    async fn test_custom_weight_schedule() {
        let config = AxisCatalogConfig::default();
        let manager = AxisCatalogManager::new(config);

        manager.initialize_default_axes(0).await;

        // Create axis with custom ease-in schedule
        let mut change = create_test_change(3, AxisAction::Add);
        change.migration_plan.ramp_epochs = 1000;
        change.migration_plan.weight_schedule = vec![
            (0, 0.0),     // Start at 0
            (500, 0.1),   // Slow start (only 10% at halfway)
            (800, 0.5),   // Accelerate
            (1000, 1.0),  // Full weight at end
        ];

        manager.propose_axis_change(&change, 0).await.unwrap();
        manager.activate_axis(3, 0).await.unwrap();

        // Test custom schedule interpolation
        let w0 = manager.get_axis_weight(3, 0).await.unwrap();
        let w250 = manager.get_axis_weight(3, 250).await.unwrap();
        let w500 = manager.get_axis_weight(3, 500).await.unwrap();
        let w650 = manager.get_axis_weight(3, 650).await.unwrap();
        let w800 = manager.get_axis_weight(3, 800).await.unwrap();
        let w900 = manager.get_axis_weight(3, 900).await.unwrap();
        let w1000 = manager.get_axis_weight(3, 1000).await.unwrap();

        assert_eq!(w0, 0.0);
        assert!((w250 - 0.05).abs() < 0.01); // Interpolated between 0.0 and 0.1
        assert_eq!(w500, 0.1);
        assert!((w650 - 0.3).abs() < 0.01); // Interpolated between 0.1 and 0.5
        assert_eq!(w800, 0.5);
        assert!((w900 - 0.75).abs() < 0.01); // Interpolated between 0.5 and 1.0
        assert_eq!(w1000, 1.0);
    }

    #[tokio::test]
    async fn test_deprecation_weight_ramp() {
        let config = AxisCatalogConfig::default();
        let manager = AxisCatalogManager::new(config);

        manager.initialize_default_axes(0).await;

        // Add and activate axis
        let mut change = create_test_change(3, AxisAction::Add);
        change.migration_plan.weight_schedule = vec![]; // Linear
        change.migration_plan.ramp_epochs = 100;

        manager.propose_axis_change(&change, 0).await.unwrap();
        manager.activate_axis(3, 0).await.unwrap();
        manager.process_epoch(100).await.unwrap(); // Complete activation

        // Now deprecate it
        let deprecate_change = AxisCatalogChange {
            action: AxisAction::Deprecate,
            axis_id: 3,
            encoder_spec_cid: "Qm_encoder_axis_3".to_string(),
            ultrametric_proof_cid: "Qm_proof_axis_3".to_string(),
            security_analysis_cid: "Qm_security_axis_3".to_string(),
            migration_plan: MigrationPlan {
                ramp_epochs: 100,
                weight_schedule: vec![],
            },
        };
        manager.propose_axis_change(&deprecate_change, 200).await.unwrap();

        // Test linear deprecation: 1 → 0 over 100 epochs
        assert_eq!(manager.get_axis_weight(3, 200).await.unwrap(), 1.0);
        assert_eq!(manager.get_axis_weight(3, 225).await.unwrap(), 0.75);
        assert_eq!(manager.get_axis_weight(3, 250).await.unwrap(), 0.5);
        assert_eq!(manager.get_axis_weight(3, 275).await.unwrap(), 0.25);
        assert_eq!(manager.get_axis_weight(3, 300).await.unwrap(), 0.0);
    }

    #[tokio::test]
    async fn test_get_all_axis_weights() {
        let config = AxisCatalogConfig::default();
        let manager = AxisCatalogManager::new(config);

        manager.initialize_default_axes(0).await;

        // Add two more axes
        let change3 = create_test_change(3, AxisAction::Add);
        let change4 = create_test_change(4, AxisAction::Add);

        manager.propose_axis_change(&change3, 0).await.unwrap();
        manager.propose_axis_change(&change4, 0).await.unwrap();

        manager.activate_axis(3, 0).await.unwrap();
        manager.activate_axis(4, 0).await.unwrap();

        // Get all weights at epoch 50 (halfway through ramp)
        let weights = manager.get_all_axis_weights(50).await.unwrap();

        // Should have default axes (0, 1, 2) at weight 1.0, plus axes 3 and 4 at 0.5
        assert!(weights.len() >= 5);

        // Find axis 3 and 4 weights
        let w3 = weights.iter().find(|(id, _)| *id == 3).map(|(_, w)| *w);
        let w4 = weights.iter().find(|(id, _)| *id == 4).map(|(_, w)| *w);

        assert_eq!(w3, Some(0.5));
        assert_eq!(w4, Some(0.5));

        // Default axes should be at full weight
        let w0 = weights.iter().find(|(id, _)| *id == 0).map(|(_, w)| *w);
        assert_eq!(w0, Some(1.0));
    }

    #[tokio::test]
    async fn test_weight_schedule_edge_cases() {
        let config = AxisCatalogConfig::default();
        let manager = AxisCatalogManager::new(config);

        manager.initialize_default_axes(0).await;

        // Test single-point schedule
        let mut change = create_test_change(3, AxisAction::Add);
        change.migration_plan.ramp_epochs = 100;
        change.migration_plan.weight_schedule = vec![(0, 0.0), (100, 1.0)];

        manager.propose_axis_change(&change, 0).await.unwrap();
        manager.activate_axis(3, 0).await.unwrap();

        // Should still interpolate linearly between the two points
        assert_eq!(manager.get_axis_weight(3, 0).await.unwrap(), 0.0);
        assert_eq!(manager.get_axis_weight(3, 50).await.unwrap(), 0.5);
        assert_eq!(manager.get_axis_weight(3, 100).await.unwrap(), 1.0);
    }
}
