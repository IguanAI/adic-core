use blake3::Hasher;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId([u8; 32]);

impl MessageId {
    pub fn new(data: &[u8]) -> Self {
        let mut hasher = Hasher::new();
        hasher.update(data);
        let hash = hasher.finalize();
        Self(hash.into())
    }

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

impl fmt::Debug for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MessageId({}...)", &self.to_hex()[..8])
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_id() {
        let data = b"test message";
        let id1 = MessageId::new(data);
        let id2 = MessageId::new(data);
        assert_eq!(id1, id2);

        let hex = id1.to_hex();
        let id3 = MessageId::from_hex(&hex).unwrap();
        assert_eq!(id1, id3);
    }
}