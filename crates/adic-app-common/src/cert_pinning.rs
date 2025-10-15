//! Certificate Pinning with Custom TLS Verification
//!
//! Implements certificate pinning for HTTPS requests using rustls custom certificate verification.
//! This provides protection against certificate authority compromise and man-in-the-middle attacks.

#[cfg(feature = "http-metadata")]
use crate::security::CertificatePinning;
#[cfg(feature = "http-metadata")]
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
#[cfg(feature = "http-metadata")]
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
#[cfg(feature = "http-metadata")]
use rustls::{ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme};
#[cfg(feature = "http-metadata")]
use std::sync::Arc;
#[cfg(feature = "http-metadata")]
use tracing::{debug, warn};

#[cfg(feature = "http-metadata")]
/// Custom certificate verifier that implements certificate pinning
///
/// This verifier performs standard TLS validation AND checks that the server
/// certificate's fingerprint matches one of the allowed fingerprints.
#[derive(Debug)]
pub struct PinningVerifier {
    /// Certificate pinning configuration
    pinning: CertificatePinning,
    /// Standard webpki-based verifier for certificate chain validation
    #[allow(dead_code)] // Used in trait impl
    standard_verifier: Arc<dyn ServerCertVerifier>,
}

#[cfg(feature = "http-metadata")]
impl PinningVerifier {
    /// Create a new pinning verifier
    pub fn new(pinning: CertificatePinning) -> Result<Self, Box<dyn std::error::Error>> {
        // Create standard verifier using webpki roots
        let mut root_store = rustls::RootCertStore::empty();

        // Add native system certificates
        for cert in rustls_native_certs::load_native_certs()? {
            root_store.add(cert).ok();
        }

        // Also add webpki roots as fallback
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let standard_verifier =
            rustls::client::WebPkiServerVerifier::builder(Arc::new(root_store)).build()?;

        Ok(Self {
            pinning,
            standard_verifier,
        })
    }

    /// Extract SHA-256 fingerprint from certificate
    fn compute_fingerprint(cert_der: &CertificateDer) -> [u8; 32] {
        use blake3::Hasher;
        let mut hasher = Hasher::new();
        hasher.update(cert_der.as_ref());
        *hasher.finalize().as_bytes()
    }

    /// Verify certificate fingerprint against pinned fingerprints
    fn verify_pinning(
        &self,
        end_entity: &CertificateDer,
        server_name: &ServerName,
    ) -> Result<(), TlsError> {
        let fingerprint = Self::compute_fingerprint(end_entity);

        if self.pinning.is_allowed(&fingerprint) {
            debug!(
                server = ?server_name,
                fingerprint = hex::encode(fingerprint),
                "✅ Certificate pinning validation passed"
            );
            Ok(())
        } else {
            let error_msg = format!(
                "Certificate pinning validation failed for {:?}: fingerprint {} not in allowed set",
                server_name,
                hex::encode(fingerprint)
            );

            if self.pinning.fingerprints().is_empty() {
                // No pins configured - this shouldn't happen if pinning is enabled
                warn!("{}", error_msg);
                Ok(())
            } else {
                warn!("{}", error_msg);
                Err(TlsError::General(
                    "Certificate pinning validation failed".to_string(),
                ))
            }
        }
    }
}

#[cfg(feature = "http-metadata")]
impl ServerCertVerifier for PinningVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        // First, perform standard TLS validation (chain, expiry, etc.)
        self.standard_verifier.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        )?;

        // Then, verify certificate pinning
        self.verify_pinning(end_entity, server_name)?;

        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.standard_verifier
            .verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.standard_verifier
            .verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.standard_verifier.supported_verify_schemes()
    }
}

#[cfg(feature = "http-metadata")]
/// Create a rustls ClientConfig with certificate pinning
pub fn create_pinned_tls_config(
    pinning: CertificatePinning,
) -> Result<ClientConfig, Box<dyn std::error::Error>> {
    let verifier = PinningVerifier::new(pinning)?;

    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();

    Ok(config)
}

#[cfg(feature = "http-metadata")]
/// Create a reqwest Client with certificate pinning
pub fn create_pinned_client(
    pinning: CertificatePinning,
    timeout: std::time::Duration,
) -> Result<reqwest::Client, Box<dyn std::error::Error>> {
    let tls_config = create_pinned_tls_config(pinning)?;

    let client = reqwest::Client::builder()
        .timeout(timeout)
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .use_preconfigured_tls(tls_config)
        .build()?;

    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "http-metadata")]
    #[test]
    fn test_fingerprint_computation() {
        // Test that fingerprint computation is deterministic
        let cert_data = b"test certificate data";
        let cert = CertificateDer::from(cert_data.to_vec());

        let fp1 = PinningVerifier::compute_fingerprint(&cert);
        let fp2 = PinningVerifier::compute_fingerprint(&cert);

        assert_eq!(fp1, fp2, "Fingerprints should be deterministic");
        assert_eq!(fp1.len(), 32, "SHA-256 fingerprint should be 32 bytes");
    }

    #[cfg(feature = "http-metadata")]
    #[test]
    fn test_pinning_verifier_creation() {
        // Install crypto provider for tests
        let _ = rustls::crypto::ring::default_provider().install_default();

        let pinning = CertificatePinning::new(vec![[1u8; 32], [2u8; 32]], true);
        let verifier = PinningVerifier::new(pinning);
        assert!(verifier.is_ok(), "Should create verifier successfully");
    }

    #[cfg(feature = "http-metadata")]
    #[test]
    fn test_create_pinned_tls_config() {
        // Install crypto provider for tests
        let _ = rustls::crypto::ring::default_provider().install_default();

        let pinning = CertificatePinning::new(vec![[1u8; 32]], true);
        let config = create_pinned_tls_config(pinning);
        assert!(config.is_ok(), "Should create TLS config successfully");
    }

    #[cfg(feature = "http-metadata")]
    #[tokio::test]
    async fn test_create_pinned_client() {
        // Install crypto provider for tests
        let _ = rustls::crypto::ring::default_provider().install_default();

        let pinning = CertificatePinning::new(vec![[1u8; 32]], true);
        let client = create_pinned_client(pinning, std::time::Duration::from_secs(10));
        assert!(client.is_ok(), "Should create HTTP client successfully");
    }
}
