pub mod consensus_crypto;
pub mod feature_crypto;
pub mod padic_crypto;
pub mod security_params;
pub mod standard_crypto;
pub mod ultrametric_security;

#[cfg(test)]
mod test_helpers;

use adic_types::{AdicError, AdicMessage, PublicKey, Result, Signature};
use ed25519_dalek::{Signature as DalekSignature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;

pub use consensus_crypto::{
    CertificateValidator, ConflictResolutionProof, ConflictResolver, ConsensusValidator,
    FinalityCertificate, HomologyProof, KCoreProof,
};
pub use feature_crypto::{
    FeatureCommitment, FeatureEncoder, FeatureProof, FeatureProver, ProofType,
};
pub use padic_crypto::{
    PadicCrypto, PadicKeyExchange, ProximityEncryption, UltrametricKeyDerivation,
};
pub use security_params::{NetworkPhase, ParameterValidator, SecurityParams};
pub use standard_crypto::{StandardCrypto, StandardKeyExchange, StandardSigner};
pub use ultrametric_security::{BallMembershipProof, UltrametricAttestation, UltrametricValidator};

/// A keypair for signing and verification
#[derive(Clone)]
pub struct Keypair {
    signing_key: SigningKey,
    public_key: PublicKey,
}

impl Keypair {
    /// Generate a new random keypair
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let public_key = PublicKey::from_bytes(verifying_key.to_bytes());

        Self {
            signing_key,
            public_key,
        }
    }

    /// Create a keypair from bytes (32 bytes for private key)
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 32 {
            return Err(AdicError::InvalidParameter(
                "Invalid key length".to_string(),
            ));
        }

        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(bytes);

        let signing_key = SigningKey::from_bytes(&key_bytes);
        let verifying_key = signing_key.verifying_key();
        let public_key = PublicKey::from_bytes(verifying_key.to_bytes());

        Ok(Self {
            signing_key,
            public_key,
        })
    }

    /// Get the public key
    pub fn public_key(&self) -> &PublicKey {
        &self.public_key
    }

    /// Sign a message
    pub fn sign(&self, message: &[u8]) -> Signature {
        let signature = self.signing_key.sign(message);
        Signature::new(signature.to_bytes().to_vec())
    }

    /// Export keypair as bytes (private key only, public can be derived)
    pub fn to_bytes(&self) -> Vec<u8> {
        self.signing_key.to_bytes().to_vec()
    }
}

pub struct CryptoEngine {
    signing_key: Option<SigningKey>,
}

impl Default for CryptoEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CryptoEngine {
    pub fn new() -> Self {
        Self { signing_key: None }
    }

    pub fn generate_keypair(&mut self) -> PublicKey {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        self.signing_key = Some(signing_key);
        PublicKey::from_bytes(verifying_key.to_bytes())
    }

    pub fn sign_message(&self, message: &AdicMessage) -> Result<Signature> {
        let signing_key = self
            .signing_key
            .as_ref()
            .ok_or(AdicError::InvalidParameter(
                "No signing key available".to_string(),
            ))?;

        let message_bytes = self.message_to_bytes(message);
        let signature = signing_key.sign(&message_bytes);

        Ok(Signature::new(signature.to_bytes().to_vec()))
    }

    pub fn verify_signature(
        &self,
        message: &AdicMessage,
        public_key: &PublicKey,
        signature: &Signature,
    ) -> Result<bool> {
        let verifying_key = VerifyingKey::from_bytes(public_key.as_bytes())
            .map_err(|_| AdicError::SignatureVerification)?;

        let sig_bytes = signature.as_bytes();
        if sig_bytes.len() != 64 {
            return Ok(false);
        }

        let mut sig_array = [0u8; 64];
        sig_array.copy_from_slice(sig_bytes);

        let dalek_sig = DalekSignature::from_bytes(&sig_array);
        let message_bytes = self.message_to_bytes(message);

        Ok(verifying_key.verify(&message_bytes, &dalek_sig).is_ok())
    }

    fn message_to_bytes(&self, message: &AdicMessage) -> Vec<u8> {
        // Use the message's own to_bytes() method for consistency
        message.to_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::{AdicFeatures, AdicMeta, AxisPhi, MessageId, QpDigits};
    use chrono::Utc;

    #[test]
    fn test_sign_and_verify() {
        let mut crypto = CryptoEngine::new();
        let public_key = crypto.generate_keypair();

        let message = AdicMessage::new(
            vec![MessageId::new(b"parent")],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(10, 3, 5))]),
            AdicMeta::new(Utc::now()),
            public_key,
            vec![1, 2, 3],
        );

        let signature = crypto.sign_message(&message).unwrap();
        assert!(!signature.is_empty());

        let valid = crypto
            .verify_signature(&message, &public_key, &signature)
            .unwrap();
        assert!(valid);

        let wrong_key = PublicKey::from_bytes([0; 32]);
        let invalid = crypto
            .verify_signature(&message, &wrong_key, &signature)
            .unwrap();
        assert!(!invalid);
    }
}
