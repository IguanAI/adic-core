use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PublicKey([u8; 32]);

impl PublicKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        let bytes = hex::decode(s)?;
        if bytes.len() != 32 {
            return Err(hex::FromHexError::InvalidStringLength);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({}...)", &self.to_hex()[..8])
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature(Vec<u8>);

impl Signature {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn empty() -> Self {
        Self(Vec::new())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(&self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        Ok(Self(hex::decode(s)?))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            write!(f, "Signature(empty)")
        } else {
            write!(f, "Signature({}...)", &self.to_hex()[..8])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_public_key() {
        let bytes = [1u8; 32];
        let pk = PublicKey::from_bytes(bytes);
        assert_eq!(pk.as_bytes(), &bytes);

        let hex = pk.to_hex();
        let pk2 = PublicKey::from_hex(&hex).unwrap();
        assert_eq!(pk, pk2);
    }

    #[test]
    fn test_signature() {
        let sig = Signature::new(vec![1, 2, 3, 4]);
        assert!(!sig.is_empty());
        assert_eq!(sig.as_bytes(), &[1, 2, 3, 4]);

        let empty = Signature::empty();
        assert!(empty.is_empty());
    }
}