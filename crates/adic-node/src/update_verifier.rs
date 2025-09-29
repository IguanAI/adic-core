use anyhow::{anyhow, Result};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use tracing::{debug, info};

/// Update verification keys - these would be the official ADIC signing keys
const ADIC_UPDATE_PUBLIC_KEY: &str =
    "9f7b0675feb9e4e5a5c5b8f5a5b5c5d5e5f5061727374757677787980818283";

/// Verifies binary updates using Ed25519 signatures
pub struct UpdateVerifier {
    /// The public key used for verification
    verifying_key: VerifyingKey,
}

impl UpdateVerifier {
    /// Create a new update verifier with the default ADIC public key
    pub fn new() -> Result<Self> {
        Self::with_public_key(ADIC_UPDATE_PUBLIC_KEY)
    }

    /// Create a new update verifier with a specific public key
    pub fn with_public_key(public_key_hex: &str) -> Result<Self> {
        let public_key_bytes = hex::decode(public_key_hex)
            .map_err(|e| anyhow!("Failed to decode public key: {}", e))?;

        if public_key_bytes.len() != 32 {
            return Err(anyhow!(
                "Invalid public key length: expected 32 bytes, got {}",
                public_key_bytes.len()
            ));
        }

        let verifying_key =
            VerifyingKey::from_bytes(&public_key_bytes.as_slice().try_into().unwrap())
                .map_err(|e| anyhow!("Invalid public key: {}", e))?;

        Ok(Self { verifying_key })
    }

    /// Verify a binary update signature
    pub fn verify_binary(
        &self,
        binary_path: &Path,
        signature_hex: &str,
        expected_hash: Option<&str>,
    ) -> Result<()> {
        // Read the binary
        let binary_data =
            fs::read(binary_path).map_err(|e| anyhow!("Failed to read binary: {}", e))?;

        // Calculate hash
        let mut hasher = Sha256::new();
        hasher.update(&binary_data);
        let computed_hash = format!("{:x}", hasher.finalize());

        // Verify hash if provided
        if let Some(expected) = expected_hash {
            if computed_hash != expected {
                return Err(anyhow!(
                    "Binary hash mismatch: expected {}, got {}",
                    expected,
                    computed_hash
                ));
            }
            debug!("Binary hash verified: {}", &computed_hash[..16]);
        }

        // Decode signature
        let signature_bytes =
            hex::decode(signature_hex).map_err(|e| anyhow!("Failed to decode signature: {}", e))?;

        if signature_bytes.len() != 64 {
            return Err(anyhow!(
                "Invalid signature length: expected 64 bytes, got {}",
                signature_bytes.len()
            ));
        }

        let signature = Signature::from_bytes(&signature_bytes.as_slice().try_into().unwrap());

        // Verify signature
        self.verifying_key
            .verify(&binary_data, &signature)
            .map_err(|e| anyhow!("Signature verification failed: {}", e))?;

        info!(
            "Binary signature verified successfully for {}",
            binary_path.display()
        );

        Ok(())
    }

    /// Verify a chunk signature
    pub fn verify_chunk(
        &self,
        chunk_data: &[u8],
        chunk_hash: &str,
        signature_hex: Option<&str>,
    ) -> Result<()> {
        // Calculate hash
        let mut hasher = Sha256::new();
        hasher.update(chunk_data);
        let computed_hash = format!("{:x}", hasher.finalize());

        // Verify hash
        if computed_hash != chunk_hash {
            return Err(anyhow!(
                "Chunk hash mismatch: expected {}, got {}",
                chunk_hash,
                computed_hash
            ));
        }

        // If signature provided, verify it
        if let Some(sig_hex) = signature_hex {
            let signature_bytes = hex::decode(sig_hex)
                .map_err(|e| anyhow!("Failed to decode chunk signature: {}", e))?;

            if signature_bytes.len() != 64 {
                return Err(anyhow!(
                    "Invalid chunk signature length: expected 64 bytes, got {}",
                    signature_bytes.len()
                ));
            }

            let signature = Signature::from_bytes(&signature_bytes.as_slice().try_into().unwrap());

            self.verifying_key
                .verify(chunk_data, &signature)
                .map_err(|e| anyhow!("Chunk signature verification failed: {}", e))?;

            debug!("Chunk signature verified");
        }

        Ok(())
    }

    /// Verify version info from DNS TXT record
    pub fn verify_version_record(
        &self,
        version: &str,
        binary_hash: &str,
        signature_hex: &str,
    ) -> Result<()> {
        // Create the message that was signed (version + hash)
        let message = format!("{}:{}", version, binary_hash);

        // Decode signature
        let signature_bytes = hex::decode(signature_hex)
            .map_err(|e| anyhow!("Failed to decode version signature: {}", e))?;

        if signature_bytes.len() != 64 {
            return Err(anyhow!(
                "Invalid version signature length: expected 64 bytes, got {}",
                signature_bytes.len()
            ));
        }

        let signature = Signature::from_bytes(&signature_bytes.as_slice().try_into().unwrap());

        // Verify signature
        self.verifying_key
            .verify(message.as_bytes(), &signature)
            .map_err(|e| anyhow!("Version signature verification failed: {}", e))?;

        info!("Version {} signature verified", version);

        Ok(())
    }
}

/// Generate a signature for testing purposes
#[cfg(test)]
pub fn sign_data(data: &[u8], signing_key: &ed25519_dalek::SigningKey) -> String {
    use ed25519_dalek::Signer;
    let signature = signing_key.sign(data);
    hex::encode(signature.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_binary_verification() -> Result<()> {
        // Generate test keypair
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();
        let public_key_hex = hex::encode(verifying_key.to_bytes());

        // Create verifier
        let verifier = UpdateVerifier::with_public_key(&public_key_hex)?;

        // Create test binary
        let temp_dir = TempDir::new()?;
        let binary_path = temp_dir.path().join("test_binary");
        let binary_data = b"test binary content";
        fs::write(&binary_path, binary_data)?;

        // Sign the binary
        let signature = sign_data(binary_data, &signing_key);

        // Calculate hash
        let mut hasher = Sha256::new();
        hasher.update(binary_data);
        let hash = format!("{:x}", hasher.finalize());

        // Verify should succeed
        assert!(verifier
            .verify_binary(&binary_path, &signature, Some(&hash))
            .is_ok());

        // Verify with wrong signature should fail
        assert!(verifier
            .verify_binary(&binary_path, &hex::encode([0u8; 64]), Some(&hash))
            .is_err());

        // Verify with wrong hash should fail
        assert!(verifier
            .verify_binary(&binary_path, &signature, Some("wrong_hash"))
            .is_err());

        Ok(())
    }

    #[test]
    fn test_chunk_verification() -> Result<()> {
        // Generate test keypair
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();
        let public_key_hex = hex::encode(verifying_key.to_bytes());

        // Create verifier
        let verifier = UpdateVerifier::with_public_key(&public_key_hex)?;

        // Create test chunk
        let chunk_data = b"test chunk content";

        // Calculate hash
        let mut hasher = Sha256::new();
        hasher.update(chunk_data);
        let chunk_hash = format!("{:x}", hasher.finalize());

        // Sign the chunk
        let signature = sign_data(chunk_data, &signing_key);

        // Verify should succeed
        assert!(verifier
            .verify_chunk(chunk_data, &chunk_hash, Some(&signature))
            .is_ok());

        // Verify with wrong hash should fail
        assert!(verifier
            .verify_chunk(chunk_data, "wrong_hash", Some(&signature))
            .is_err());

        // Verify without signature should succeed if hash matches
        assert!(verifier.verify_chunk(chunk_data, &chunk_hash, None).is_ok());

        Ok(())
    }

    #[test]
    fn test_version_record_verification() -> Result<()> {
        // Generate test keypair
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();
        let public_key_hex = hex::encode(verifying_key.to_bytes());

        // Create verifier
        let verifier = UpdateVerifier::with_public_key(&public_key_hex)?;

        // Create version info
        let version = "0.2.0";
        let binary_hash = "abc123def456";
        let message = format!("{}:{}", version, binary_hash);

        // Sign the version info
        let signature = sign_data(message.as_bytes(), &signing_key);

        // Verify should succeed
        assert!(verifier
            .verify_version_record(version, binary_hash, &signature)
            .is_ok());

        // Verify with wrong version should fail
        assert!(verifier
            .verify_version_record("0.1.0", binary_hash, &signature)
            .is_err());

        // Verify with wrong hash should fail
        assert!(verifier
            .verify_version_record(version, "wrong_hash", &signature)
            .is_err());

        Ok(())
    }
}
