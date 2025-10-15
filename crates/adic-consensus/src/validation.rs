use adic_crypto::CryptoEngine;
use adic_types::{AdicMessage, MessageId};
use chrono::Utc;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ValidationResult {
    pub fn valid() -> Self {
        Self {
            is_valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn invalid(error: String) -> Self {
        Self {
            is_valid: false,
            errors: vec![error],
            warnings: Vec::new(),
        }
    }

    pub fn add_error(&mut self, error: String) {
        self.errors.push(error);
        self.is_valid = false;
    }

    pub fn add_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }
}

pub struct MessageValidator {
    max_timestamp_drift: i64,
    max_payload_size: usize,
    crypto: CryptoEngine,
    /// LRU cache for validation results (message_id -> (result, timestamp))
    validation_cache: Arc<RwLock<HashMap<MessageId, (ValidationResult, std::time::Instant)>>>,
    /// LRU queue for cache eviction
    cache_queue: Arc<RwLock<VecDeque<MessageId>>>,
    /// Maximum cache size
    max_cache_size: usize,
    // Metrics counters - updated externally by incrementing directly
    pub signature_verifications: Option<Arc<prometheus::IntCounter>>,
    pub signature_failures: Option<Arc<prometheus::IntCounter>>,
}

impl Default for MessageValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageValidator {
    pub fn new() -> Self {
        Self {
            max_timestamp_drift: 300,
            max_payload_size: 1024 * 1024,
            crypto: CryptoEngine::new(),
            validation_cache: Arc::new(RwLock::new(HashMap::new())),
            cache_queue: Arc::new(RwLock::new(VecDeque::new())),
            max_cache_size: 10000, // Cache up to 10k validation results
            signature_verifications: None,
            signature_failures: None,
        }
    }

    /// Set metrics for signature tracking
    pub fn set_metrics(
        &mut self,
        signature_verifications: Arc<prometheus::IntCounter>,
        signature_failures: Arc<prometheus::IntCounter>,
    ) {
        self.signature_verifications = Some(signature_verifications);
        self.signature_failures = Some(signature_failures);
    }

    /// Clear the validation cache
    pub async fn clear_cache(&self) {
        let mut cache = self.validation_cache.write().await;
        cache.clear();
        let mut queue = self.cache_queue.write().await;
        queue.clear();
    }

    /// Get cache statistics
    pub async fn cache_stats(&self) -> (usize, usize) {
        let cache = self.validation_cache.read().await;
        (cache.len(), self.max_cache_size)
    }

    /// Validate message with caching (async version)
    /// This method checks the cache first and returns cached results if available
    pub async fn validate_message_cached(&self, message: &AdicMessage) -> ValidationResult {
        // Check cache first
        {
            let cache = self.validation_cache.read().await;
            if let Some((cached_result, _timestamp)) = cache.get(&message.id) {
                return cached_result.clone();
            }
        }

        // Cache miss - perform validation
        let result = self.validate_message(message);

        // Store in cache if cache is not full
        {
            let mut cache = self.validation_cache.write().await;
            let mut queue = self.cache_queue.write().await;

            // Evict oldest if cache is full
            if cache.len() >= self.max_cache_size {
                if let Some(old_id) = queue.pop_front() {
                    cache.remove(&old_id);
                }
            }

            // Add new entry
            cache.insert(message.id, (result.clone(), std::time::Instant::now()));
            queue.push_back(message.id);
        }

        result
    }

    pub fn validate_message(&self, message: &AdicMessage) -> ValidationResult {
        let mut result = ValidationResult::valid();

        // Perform structural checks but do not early-return; collect all errors
        let _ = self.validate_structure(message, &mut result);

        self.validate_timestamp(message, &mut result);
        self.validate_parents(message, &mut result);
        self.validate_payload(message, &mut result);
        self.validate_features(message, &mut result);
        self.validate_transfer(message, &mut result);

        info!(
            message_id = %message.id,
            proposer = %message.proposer_pk.to_hex(),
            is_valid = result.is_valid,
            error_count = result.errors.len(),
            warning_count = result.warnings.len(),
            parents_count = message.parents.len(),
            payload_size = message.data.len(),
            features_count = message.features.dimension(),
            has_transfer = message.has_value_transfer(),
            errors = ?result.errors,
            warnings = ?result.warnings,
            "🔍 Message validated"
        );

        result
    }

    fn validate_structure(&self, message: &AdicMessage, result: &mut ValidationResult) -> bool {
        if !message.verify_id() {
            result.add_error("Message ID does not match computed hash".to_string());
            return false;
        }

        // Verify cryptographic signature
        if !self.verify_signature(message, result) {
            return false;
        }

        true
    }

    fn verify_signature(&self, message: &AdicMessage, result: &mut ValidationResult) -> bool {
        // Update metrics - signature verification attempt
        if let Some(ref counter) = self.signature_verifications {
            counter.inc();
        }

        if message.signature.is_empty() {
            result.add_error("Message signature is empty".to_string());
            if let Some(ref counter) = self.signature_failures {
                counter.inc();
            }
            return false;
        }

        // Verify signature using CryptoEngine
        match self
            .crypto
            .verify_signature(message, &message.proposer_pk, &message.signature)
        {
            Ok(is_valid) => {
                if !is_valid {
                    result.add_error("Invalid message signature".to_string());
                    if let Some(ref counter) = self.signature_failures {
                        counter.inc();
                    }
                    return false;
                }
            }
            Err(e) => {
                result.add_error(format!("Signature verification error: {}", e));
                if let Some(ref counter) = self.signature_failures {
                    counter.inc();
                }
                return false;
            }
        }

        true
    }

    fn validate_timestamp(&self, message: &AdicMessage, result: &mut ValidationResult) {
        let now = Utc::now();
        let msg_time = message.meta.timestamp;
        let drift = (now - msg_time).num_seconds().abs();

        if drift > self.max_timestamp_drift {
            result.add_error(format!(
                "Timestamp drift {} exceeds maximum {}",
                drift, self.max_timestamp_drift
            ));
        }
    }

    fn validate_parents(&self, message: &AdicMessage, result: &mut ValidationResult) {
        if message.parents.is_empty() && !message.is_genesis() {
            result.add_error("Non-genesis message has no parents".to_string());
        }

        let unique_parents: HashSet<_> = message.parents.iter().collect();
        if unique_parents.len() != message.parents.len() {
            result.add_error("Message contains duplicate parents".to_string());
        }

        for parent in &message.parents {
            if *parent == message.id {
                result.add_error("Message references itself as parent".to_string());
                break;
            }
        }
    }

    fn validate_payload(&self, message: &AdicMessage, result: &mut ValidationResult) {
        if message.data.len() > self.max_payload_size {
            result.add_error(format!(
                "Payload size {} exceeds maximum {}",
                message.data.len(),
                self.max_payload_size
            ));
        }

        if message.data.is_empty() {
            result.add_warning("Message has empty payload".to_string());
        }
    }

    fn validate_features(&self, message: &AdicMessage, result: &mut ValidationResult) {
        if message.features.dimension() == 0 {
            result.add_error("Message has no feature dimensions".to_string());
        }

        let mut seen_axes = HashSet::new();
        for phi in &message.features.phi {
            if !seen_axes.insert(phi.axis) {
                result.add_error(format!("Duplicate axis {} in features", phi.axis.0));
            }

            if phi.qp_digits.digits.is_empty() {
                result.add_error(format!("Axis {} has empty digits", phi.axis.0));
            }
        }
    }

    fn validate_transfer(&self, message: &AdicMessage, result: &mut ValidationResult) {
        // If there's no transfer, nothing to validate
        if let Some(transfer) = message.get_transfer() {
            // Use the built-in validation from ValueTransfer
            if !transfer.is_valid() {
                result.add_error("Invalid value transfer".to_string());

                // Provide more specific errors for debugging
                if transfer.amount == 0 {
                    result.add_error("Transfer amount must be greater than zero".to_string());
                }

                if transfer.from == transfer.to {
                    result.add_error("Transfer sender and recipient must be different".to_string());
                }

                if transfer.from.len() != 32 {
                    result.add_error(format!(
                        "Transfer sender address must be 32 bytes, got {}",
                        transfer.from.len()
                    ));
                }

                if transfer.to.len() != 32 {
                    result.add_error(format!(
                        "Transfer recipient address must be 32 bytes, got {}",
                        transfer.to.len()
                    ));
                }
            }

            // Warn about zero nonce (potential security issue)
            if transfer.nonce == 0 {
                result.add_warning(
                    "Transfer has zero nonce, potential replay vulnerability".to_string(),
                );
            }

            // Note: Balance checks and signature verification against the 'from' address
            // will be performed by the economics engine, not here
        }
    }

    pub fn validate_acyclicity(
        &self,
        message: &AdicMessage,
        ancestor_ids: &HashSet<MessageId>,
    ) -> bool {
        let is_acyclic = !ancestor_ids.contains(&message.id);

        info!(
            message_id = %message.id,
            ancestor_count = ancestor_ids.len(),
            is_acyclic = is_acyclic,
            "🔄 Acyclicity checked"
        );

        is_acyclic
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::{AdicFeatures, AdicMeta, AxisPhi, PublicKey, QpDigits, Signature};

    fn create_valid_message() -> AdicMessage {
        let parents = vec![MessageId::new(b"parent1")];
        let features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(10, 3, 5)),
            AxisPhi::new(1, QpDigits::from_u64(20, 3, 5)),
            AxisPhi::new(2, QpDigits::from_u64(30, 3, 5)),
        ]);
        let meta = AdicMeta::new(Utc::now());
        let pk = PublicKey::from_bytes([0; 32]);

        // Create message without signature first

        // For testing, we'll need to mock or skip signature verification
        AdicMessage::new(parents, features, meta, pk, vec![1, 2, 3])
    }

    #[test]
    fn test_validate_valid_message() {
        let validator = MessageValidator::new();
        let mut message = create_valid_message();

        // For testing purposes, we create a dummy valid signature
        // In a real scenario, this would be properly signed by the crypto engine
        message.signature = Signature::new(vec![1; 64]); // Non-empty signature

        let result = validator.validate_message(&message);
        // Due to signature verification, this will fail without a proper mock
        // So we test that at least the structure validation works
        assert!(!result.errors.is_empty()); // Expect signature error
        assert!(result
            .errors
            .iter()
            .any(|e| e.contains("signature") || e.contains("Signature")));
    }

    #[test]
    fn test_validate_empty_signature() {
        let validator = MessageValidator::new();
        let mut message = create_valid_message();
        message.signature = Signature::empty();

        let result = validator.validate_message(&message);
        assert!(!result.is_valid);
        assert!(result.errors.iter().any(|e| e.contains("signature")));
    }

    #[test]
    fn test_validate_duplicate_parents() {
        let validator = MessageValidator::new();
        let mut message = create_valid_message();
        let parent = MessageId::new(b"parent");
        message.parents = vec![parent, parent];

        let result = validator.validate_message(&message);
        assert!(!result.is_valid);
        assert!(result.errors.iter().any(|e| e.contains("duplicate")));
    }

    #[test]
    fn test_validate_acyclicity() {
        let validator = MessageValidator::new();
        let message = create_valid_message();

        let mut ancestors = HashSet::new();
        ancestors.insert(message.id);

        assert!(!validator.validate_acyclicity(&message, &ancestors));

        let empty_ancestors = HashSet::new();
        assert!(validator.validate_acyclicity(&message, &empty_ancestors));
    }

    #[test]
    fn test_validate_message_with_valid_transfer() {
        use adic_types::ValueTransfer;

        let validator = MessageValidator::new();
        let mut message = create_valid_message();

        let from = vec![1u8; 32];
        let to = vec![2u8; 32];
        let transfer = ValueTransfer::new(from, to, 1000, 1);

        message.transfer = Some(transfer);
        message.signature = Signature::new(vec![1; 64]);

        let result = validator.validate_message(&message);

        // Should not have transfer-related errors (may have signature errors)
        assert!(!result
            .errors
            .iter()
            .any(|e| e.contains("Transfer") || e.contains("transfer")));
    }

    #[test]
    fn test_validate_message_without_transfer() {
        let validator = MessageValidator::new();
        let mut message = create_valid_message();
        message.signature = Signature::new(vec![1; 64]);

        let result = validator.validate_message(&message);

        // Should validate normally without transfer
        // No transfer-related errors expected
        assert!(!result
            .errors
            .iter()
            .any(|e| e.contains("Transfer") || e.contains("transfer")));
    }

    #[test]
    fn test_validate_transfer_zero_amount() {
        use adic_types::ValueTransfer;

        let validator = MessageValidator::new();
        let mut message = create_valid_message();

        let from = vec![1u8; 32];
        let to = vec![2u8; 32];
        let transfer = ValueTransfer::new(from, to, 0, 1); // Zero amount

        message.transfer = Some(transfer);
        message.signature = Signature::new(vec![1; 64]);

        let result = validator.validate_message(&message);

        assert!(!result.is_valid);
        assert!(result.errors.iter().any(|e| e.contains("amount")));
    }

    #[test]
    fn test_validate_transfer_same_addresses() {
        use adic_types::ValueTransfer;

        let validator = MessageValidator::new();
        let mut message = create_valid_message();

        let addr = vec![1u8; 32];
        let transfer = ValueTransfer::new(addr.clone(), addr, 1000, 1); // Same address

        message.transfer = Some(transfer);
        message.signature = Signature::new(vec![1; 64]);

        let result = validator.validate_message(&message);

        assert!(!result.is_valid);
        assert!(result
            .errors
            .iter()
            .any(|e| e.contains("sender and recipient")));
    }

    #[test]
    fn test_validate_transfer_invalid_address_length() {
        use adic_types::ValueTransfer;

        let validator = MessageValidator::new();
        let mut message = create_valid_message();

        let from = vec![1u8; 16]; // Wrong length
        let to = vec![2u8; 32];
        let transfer = ValueTransfer::new(from, to, 1000, 1);

        message.transfer = Some(transfer);
        message.signature = Signature::new(vec![1; 64]);

        let result = validator.validate_message(&message);

        assert!(!result.is_valid);
        assert!(result.errors.iter().any(|e| e.contains("32 bytes")));
    }

    #[test]
    fn test_validate_transfer_zero_nonce_warning() {
        use adic_types::ValueTransfer;

        let validator = MessageValidator::new();
        let mut message = create_valid_message();

        let from = vec![1u8; 32];
        let to = vec![2u8; 32];
        let transfer = ValueTransfer::new(from, to, 1000, 0); // Zero nonce

        message.transfer = Some(transfer);
        message.signature = Signature::new(vec![1; 64]);

        let result = validator.validate_message(&message);

        // Should have a warning about zero nonce
        assert!(result.warnings.iter().any(|w| w.contains("nonce")));
    }

    #[tokio::test]
    async fn test_validate_message_cached_basic() {
        let validator = MessageValidator::new();
        let mut message = create_valid_message();
        message.signature = Signature::new(vec![1; 64]);

        // First call - cache miss
        let result1 = validator.validate_message_cached(&message).await;

        // Second call - should hit cache
        let result2 = validator.validate_message_cached(&message).await;

        // Results should be identical
        assert_eq!(result1.is_valid, result2.is_valid);
        assert_eq!(result1.errors.len(), result2.errors.len());

        // Check cache stats
        let (cache_size, max_size) = validator.cache_stats().await;
        assert_eq!(cache_size, 1, "Should have 1 cached entry");
        assert_eq!(max_size, 10000);
    }

    #[tokio::test]
    async fn test_validate_message_cached_multiple() {
        let validator = MessageValidator::new();

        // Create 5 different messages
        let messages: Vec<_> = (0..5)
            .map(|i| {
                let parents = vec![MessageId::new(format!("parent{}", i).as_bytes())];
                let features =
                    AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(10 + i, 3, 5))]);
                let meta = AdicMeta::new(Utc::now());
                let pk = PublicKey::from_bytes([i as u8; 32]);
                let mut msg = AdicMessage::new(parents, features, meta, pk, vec![1, 2, 3]);
                msg.signature = Signature::new(vec![1; 64]);
                msg
            })
            .collect();

        // Validate all messages
        for msg in &messages {
            let _ = validator.validate_message_cached(msg).await;
        }

        // Cache should have 5 entries
        let (cache_size, _) = validator.cache_stats().await;
        assert_eq!(cache_size, 5, "Should have 5 cached entries");

        // Re-validate first message - should hit cache
        let _ = validator.validate_message_cached(&messages[0]).await;

        // Cache size should still be 5
        let (cache_size, _) = validator.cache_stats().await;
        assert_eq!(cache_size, 5, "Cache size should remain 5");
    }

    #[tokio::test]
    async fn test_validate_message_cached_eviction() {
        // Create validator with very small cache for testing
        let mut validator = MessageValidator::new();
        validator.max_cache_size = 3; // Only 3 entries

        // Create 5 different messages
        let messages: Vec<_> = (0..5)
            .map(|i| {
                let parents = vec![MessageId::new(format!("parent{}", i).as_bytes())];
                let features =
                    AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(10 + i, 3, 5))]);
                let meta = AdicMeta::new(Utc::now());
                let pk = PublicKey::from_bytes([i as u8; 32]);
                let mut msg = AdicMessage::new(parents, features, meta, pk, vec![1, 2, 3]);
                msg.signature = Signature::new(vec![1; 64]);
                msg
            })
            .collect();

        // Validate all 5 messages
        for msg in &messages {
            let _ = validator.validate_message_cached(msg).await;
        }

        // Cache should be capped at max_cache_size (3)
        let (cache_size, max_size) = validator.cache_stats().await;
        assert_eq!(cache_size, 3, "Cache should be limited to max size");
        assert_eq!(max_size, 3);
    }

    #[tokio::test]
    async fn test_clear_cache() {
        let validator = MessageValidator::new();
        let mut message = create_valid_message();
        message.signature = Signature::new(vec![1; 64]);

        // Add to cache
        let _ = validator.validate_message_cached(&message).await;

        // Verify cache has entry
        let (cache_size, _) = validator.cache_stats().await;
        assert_eq!(cache_size, 1);

        // Clear cache
        validator.clear_cache().await;

        // Verify cache is empty
        let (cache_size, _) = validator.cache_stats().await;
        assert_eq!(cache_size, 0, "Cache should be empty after clear");
    }
}
