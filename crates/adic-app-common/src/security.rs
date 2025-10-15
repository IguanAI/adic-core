//! Security primitives for HTTP communication
//!
//! Provides TLS certificate validation, certificate pinning, and request signing
//! for secure communication with external services.

use ed25519_dalek::{Signature as DalekSignature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tracing::{debug, warn};

/// Certificate fingerprint (SHA-256 hash of DER-encoded certificate)
pub type CertFingerprint = [u8; 32];

/// Certificate pinning configuration
///
/// Implements certificate pinning to prevent MITM attacks by validating
/// that the server certificate matches a known set of fingerprints.
#[derive(Debug, Clone)]
pub struct CertificatePinning {
    /// Set of allowed certificate fingerprints (SHA-256)
    allowed_fingerprints: HashSet<CertFingerprint>,
    /// Whether to enforce pinning (if false, only logs warnings)
    enforce: bool,
}

impl CertificatePinning {
    /// Create new certificate pinning configuration
    pub fn new(fingerprints: Vec<CertFingerprint>, enforce: bool) -> Self {
        Self {
            allowed_fingerprints: fingerprints.into_iter().collect(),
            enforce,
        }
    }

    /// Create disabled certificate pinning (allows all certificates)
    pub fn disabled() -> Self {
        Self {
            allowed_fingerprints: HashSet::new(),
            enforce: false,
        }
    }

    /// Add an allowed certificate fingerprint
    pub fn add_fingerprint(&mut self, fingerprint: CertFingerprint) {
        self.allowed_fingerprints.insert(fingerprint);
    }

    /// Check if a certificate is allowed
    pub fn is_allowed(&self, fingerprint: &CertFingerprint) -> bool {
        if !self.enforce {
            return true;
        }

        self.allowed_fingerprints.contains(fingerprint)
    }

    /// Validate certificate fingerprint
    pub fn validate(
        &self,
        fingerprint: &CertFingerprint,
        service_name: &str,
    ) -> Result<(), String> {
        if self.is_allowed(fingerprint) {
            debug!(
                service = service_name,
                fingerprint = hex::encode(fingerprint),
                "Certificate pinning validation passed"
            );
            Ok(())
        } else {
            let error = format!(
                "Certificate pinning validation failed for {}: fingerprint {} not in allowed set",
                service_name,
                hex::encode(fingerprint)
            );

            if self.enforce {
                warn!("{}", error);
                Err(error)
            } else {
                warn!("⚠️  {}", error);
                Ok(())
            }
        }
    }

    /// Get all allowed fingerprints
    pub fn fingerprints(&self) -> Vec<CertFingerprint> {
        self.allowed_fingerprints.iter().cloned().collect()
    }
}

/// Request signature for authenticated HTTP requests
///
/// Signs HTTP requests with Ed25519 to prove authenticity to the server.
///
/// # Security
///
/// Uses Ed25519 signatures for authentication:
/// - Message: method || url || body || timestamp
/// - Algorithm: Ed25519 (RFC 8032)
/// - Encoding: Hex-encoded public key (32 bytes) and signature (64 bytes)
///
/// # Example
///
/// ```rust
/// use adic_app_common::security::RequestSignature;
/// use adic_crypto::Keypair;
///
/// # fn example() -> Result<(), String> {
/// let keypair = Keypair::generate();
/// let method = "POST";
/// let url = "https://api.example.com/v1/submit";
/// let body = b"{\"data\": \"value\"}";
/// let timestamp = 1234567890;
///
/// // Sign the request
/// let signature = RequestSignature::new(&keypair, method, url, body, timestamp);
///
/// // Verify the signature
/// signature.verify(method, url, body)?;
///
/// // Add to HTTP headers
/// let headers = signature.add_to_headers();
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestSignature {
    /// Public key of the signer (node identity) - hex-encoded 32 bytes
    pub public_key: String,
    /// Timestamp of the signature (Unix timestamp in seconds)
    pub timestamp: i64,
    /// Ed25519 signature over (method || url || body || timestamp) - hex-encoded 64 bytes
    pub signature: String,
}

impl RequestSignature {
    /// Create a new request signature
    ///
    /// Signs the request using Ed25519:
    /// signature = sign(method || url || body || timestamp)
    pub fn new(
        keypair: &adic_crypto::Keypair,
        method: &str,
        url: &str,
        body: &[u8],
        timestamp: i64,
    ) -> Self {
        // Build message to sign
        let mut message = Vec::new();
        message.extend_from_slice(method.as_bytes());
        message.extend_from_slice(url.as_bytes());
        message.extend_from_slice(body);
        message.extend_from_slice(&timestamp.to_le_bytes());

        // Sign with Ed25519
        let signature = keypair.sign(&message);

        Self {
            public_key: hex::encode(keypair.public_key().as_bytes()),
            timestamp,
            signature: hex::encode(signature.as_bytes()),
        }
    }

    /// Verify a request signature
    ///
    /// Verifies the Ed25519 signature over (method || url || body || timestamp).
    /// Returns Ok(()) if the signature is valid, Err with details otherwise.
    pub fn verify(&self, method: &str, url: &str, body: &[u8]) -> Result<(), String> {
        // Reconstruct message
        let mut message = Vec::new();
        message.extend_from_slice(method.as_bytes());
        message.extend_from_slice(url.as_bytes());
        message.extend_from_slice(body);
        message.extend_from_slice(&self.timestamp.to_le_bytes());

        // Decode public key and signature
        let pubkey_bytes =
            hex::decode(&self.public_key).map_err(|e| format!("Invalid public key hex: {}", e))?;
        let sig_bytes =
            hex::decode(&self.signature).map_err(|e| format!("Invalid signature hex: {}", e))?;

        if pubkey_bytes.len() != 32 {
            return Err(format!("Invalid public key length: {}", pubkey_bytes.len()));
        }
        if sig_bytes.len() != 64 {
            return Err(format!("Invalid signature length: {}", sig_bytes.len()));
        }

        // Create Ed25519 verifying key from public key bytes
        let mut pubkey_array = [0u8; 32];
        pubkey_array.copy_from_slice(&pubkey_bytes);

        let verifying_key = VerifyingKey::from_bytes(&pubkey_array)
            .map_err(|e| format!("Invalid Ed25519 public key: {}", e))?;

        // Create Ed25519 signature from signature bytes
        let mut sig_array = [0u8; 64];
        sig_array.copy_from_slice(&sig_bytes);

        let signature = DalekSignature::from_bytes(&sig_array);

        // Verify the signature
        verifying_key
            .verify(&message, &signature)
            .map_err(|_| "Signature verification failed: invalid signature".to_string())?;

        debug!(
            public_key = &self.public_key[..16],
            timestamp = self.timestamp,
            "✅ Request signature verified"
        );

        Ok(())
    }

    /// Add signature to HTTP headers
    pub fn add_to_headers(&self) -> Vec<(String, String)> {
        vec![
            ("X-ADIC-PublicKey".to_string(), self.public_key.clone()),
            ("X-ADIC-Timestamp".to_string(), self.timestamp.to_string()),
            ("X-ADIC-Signature".to_string(), self.signature.clone()),
        ]
    }
}

/// Security audit event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityAuditEvent {
    /// Timestamp of the event
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Event type (e.g., "certificate_validation", "request_signing")
    pub event_type: String,
    /// Severity level
    pub severity: SecuritySeverity,
    /// Service name or URL
    pub service: String,
    /// Additional details
    pub details: String,
    /// Whether the security check passed
    pub passed: bool,
}

/// Security event severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecuritySeverity {
    Info,
    Warning,
    Critical,
}

impl SecurityAuditEvent {
    /// Create a new audit event
    pub fn new(
        event_type: impl Into<String>,
        severity: SecuritySeverity,
        service: impl Into<String>,
        details: impl Into<String>,
        passed: bool,
    ) -> Self {
        Self {
            timestamp: chrono::Utc::now(),
            event_type: event_type.into(),
            severity,
            service: service.into(),
            details: details.into(),
            passed,
        }
    }

    /// Log the audit event
    pub fn log(&self) {
        match self.severity {
            SecuritySeverity::Info => {
                tracing::info!(
                    event_type = %self.event_type,
                    service = %self.service,
                    passed = self.passed,
                    "🔒 Security audit: {}",
                    self.details
                );
            }
            SecuritySeverity::Warning => {
                tracing::warn!(
                    event_type = %self.event_type,
                    service = %self.service,
                    passed = self.passed,
                    "⚠️  Security audit: {}",
                    self.details
                );
            }
            SecuritySeverity::Critical => {
                tracing::error!(
                    event_type = %self.event_type,
                    service = %self.service,
                    passed = self.passed,
                    "🚨 Security audit: {}",
                    self.details
                );
            }
        }
    }
}

/// Security audit logger
pub struct SecurityAuditLogger {
    events: tokio::sync::RwLock<Vec<SecurityAuditEvent>>,
    max_events: usize,
}

impl SecurityAuditLogger {
    /// Create a new audit logger
    pub fn new(max_events: usize) -> Self {
        Self {
            events: tokio::sync::RwLock::new(Vec::new()),
            max_events,
        }
    }

    /// Log a security event
    pub async fn log(&self, event: SecurityAuditEvent) {
        event.log();

        let mut events = self.events.write().await;
        events.push(event);

        // Keep only the most recent events
        if events.len() > self.max_events {
            let drain_count = events.len() - self.max_events;
            events.drain(0..drain_count);
        }
    }

    /// Get all logged events
    pub async fn get_events(&self) -> Vec<SecurityAuditEvent> {
        let events = self.events.read().await;
        events.clone()
    }

    /// Get events by type
    pub async fn get_events_by_type(&self, event_type: &str) -> Vec<SecurityAuditEvent> {
        let events = self.events.read().await;
        events
            .iter()
            .filter(|e| e.event_type == event_type)
            .cloned()
            .collect()
    }

    /// Get failed security events
    pub async fn get_failed_events(&self) -> Vec<SecurityAuditEvent> {
        let events = self.events.read().await;
        events.iter().filter(|e| !e.passed).cloned().collect()
    }

    /// Clear all events
    pub async fn clear(&self) {
        let mut events = self.events.write().await;
        events.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_certificate_pinning() {
        let fingerprint1 = [1u8; 32];
        let fingerprint2 = [2u8; 32];
        let fingerprint3 = [3u8; 32];

        let mut pinning = CertificatePinning::new(vec![fingerprint1, fingerprint2], true);

        assert!(pinning.is_allowed(&fingerprint1));
        assert!(pinning.is_allowed(&fingerprint2));
        assert!(!pinning.is_allowed(&fingerprint3));

        pinning.add_fingerprint(fingerprint3);
        assert!(pinning.is_allowed(&fingerprint3));
    }

    #[test]
    fn test_certificate_pinning_disabled() {
        let pinning = CertificatePinning::disabled();
        let any_fingerprint = [99u8; 32];

        assert!(pinning.is_allowed(&any_fingerprint));
    }

    #[test]
    fn test_request_signature_valid() {
        let keypair = adic_crypto::Keypair::generate();
        let method = "GET";
        let url = "https://api.example.com/v1/data";
        let body = b"test body";
        let timestamp = 1234567890;

        let signature = RequestSignature::new(&keypair, method, url, body, timestamp);

        // Verify valid signature
        let result = signature.verify(method, url, body);
        assert!(result.is_ok(), "Valid signature should verify successfully");
    }

    #[test]
    fn test_request_signature_invalid_signature() {
        let keypair = adic_crypto::Keypair::generate();
        let method = "GET";
        let url = "https://api.example.com/v1/data";
        let body = b"test body";
        let timestamp = 1234567890;

        let mut signature = RequestSignature::new(&keypair, method, url, body, timestamp);

        // Tamper with the signature
        signature.signature = "0".repeat(128); // Invalid signature

        let result = signature.verify(method, url, body);
        assert!(
            result.is_err(),
            "Invalid signature should fail verification"
        );
        assert!(result.unwrap_err().contains("verification failed"));
    }

    #[test]
    fn test_request_signature_wrong_public_key() {
        let keypair1 = adic_crypto::Keypair::generate();
        let keypair2 = adic_crypto::Keypair::generate();
        let method = "POST";
        let url = "/api/submit";
        let body = b"data";
        let timestamp = 9876543210;

        let mut signature = RequestSignature::new(&keypair1, method, url, body, timestamp);

        // Replace with different public key
        signature.public_key = hex::encode(keypair2.public_key().as_bytes());

        let result = signature.verify(method, url, body);
        assert!(
            result.is_err(),
            "Signature with wrong public key should fail"
        );
    }

    #[test]
    fn test_request_signature_tampered_message() {
        let keypair = adic_crypto::Keypair::generate();
        let method = "PUT";
        let url = "/api/update";
        let body = b"original data";
        let timestamp = 1111111111;

        let signature = RequestSignature::new(&keypair, method, url, body, timestamp);

        // Verify with different message components
        let tampered_body = b"tampered data";
        let result = signature.verify(method, url, tampered_body);
        assert!(
            result.is_err(),
            "Signature verification should fail for tampered message"
        );

        let tampered_method = "POST";
        let result = signature.verify(tampered_method, url, body);
        assert!(
            result.is_err(),
            "Signature verification should fail for tampered method"
        );

        let tampered_url = "/api/different";
        let result = signature.verify(method, tampered_url, body);
        assert!(
            result.is_err(),
            "Signature verification should fail for tampered URL"
        );
    }

    #[test]
    fn test_request_signature_invalid_format() {
        let signature = RequestSignature {
            public_key: "invalid_hex".to_string(),
            timestamp: 123,
            signature: "0".repeat(128),
        };

        let result = signature.verify("GET", "/test", b"");
        assert!(result.is_err(), "Invalid hex should be rejected");

        let signature = RequestSignature {
            public_key: hex::encode([0u8; 16]), // Wrong length (16 instead of 32)
            timestamp: 123,
            signature: "0".repeat(128),
        };

        let result = signature.verify("GET", "/test", b"");
        assert!(
            result.is_err(),
            "Invalid public key length should be rejected"
        );
    }

    #[test]
    fn test_signature_headers() {
        let keypair = adic_crypto::Keypair::generate();
        let signature = RequestSignature::new(&keypair, "GET", "/test", b"", 123);

        let headers = signature.add_to_headers();
        assert_eq!(headers.len(), 3);

        let header_names: Vec<_> = headers.iter().map(|(k, _)| k.as_str()).collect();
        assert!(header_names.contains(&"X-ADIC-PublicKey"));
        assert!(header_names.contains(&"X-ADIC-Timestamp"));
        assert!(header_names.contains(&"X-ADIC-Signature"));
    }

    #[tokio::test]
    async fn test_security_audit_logger() {
        let logger = SecurityAuditLogger::new(100);

        let event = SecurityAuditEvent::new(
            "certificate_validation",
            SecuritySeverity::Info,
            "metadata.adic.network",
            "Certificate validated successfully",
            true,
        );

        logger.log(event).await;

        let events = logger.get_events().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "certificate_validation");
        assert!(events[0].passed);
    }

    #[tokio::test]
    async fn test_audit_logger_max_events() {
        let logger = SecurityAuditLogger::new(5);

        // Log 10 events
        for i in 0..10 {
            let event = SecurityAuditEvent::new(
                "test",
                SecuritySeverity::Info,
                "test_service",
                format!("Event {}", i),
                true,
            );
            logger.log(event).await;
        }

        let events = logger.get_events().await;
        assert_eq!(events.len(), 5, "Should keep only last 5 events");
    }

    #[tokio::test]
    async fn test_get_failed_events() {
        let logger = SecurityAuditLogger::new(100);

        logger
            .log(SecurityAuditEvent::new(
                "test",
                SecuritySeverity::Info,
                "svc1",
                "Pass",
                true,
            ))
            .await;

        logger
            .log(SecurityAuditEvent::new(
                "test",
                SecuritySeverity::Warning,
                "svc2",
                "Fail",
                false,
            ))
            .await;

        let failed = logger.get_failed_events().await;
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].service, "svc2");
    }
}
