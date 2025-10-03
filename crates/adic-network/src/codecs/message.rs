use super::compression::{compress, decompress, CompressionLevel};
use adic_types::{AdicError, AdicMessage, MessageId, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodedMessage {
    pub id: MessageId,
    pub data: Vec<u8>,
    pub encoding: EncodingType,
    pub compressed: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum EncodingType {
    Json,
    Protobuf,
    Bincode,
}

pub struct MessageCodec {
    encoding: EncodingType,
    compression: CompressionLevel,
}

impl MessageCodec {
    pub fn new() -> Self {
        Self {
            encoding: EncodingType::Json,
            compression: CompressionLevel::None,
        }
    }

    pub fn with_encoding(encoding: EncodingType) -> Self {
        Self {
            encoding,
            compression: CompressionLevel::None,
        }
    }

    pub fn set_compression(&mut self, level: CompressionLevel) {
        self.compression = level;
    }

    pub fn encode(&self, message: &AdicMessage) -> Result<Vec<u8>> {
        // First encode the message
        let encoded = match self.encoding {
            EncodingType::Json => serde_json::to_vec(message)
                .map_err(|e| AdicError::Serialization(format!("JSON encoding failed: {}", e)))?,
            EncodingType::Protobuf => {
                // Would need protobuf definitions
                // For now, use JSON
                serde_json::to_vec(message).map_err(|e| {
                    AdicError::Serialization(format!("Protobuf encoding failed: {}", e))
                })?
            }
            EncodingType::Bincode => bincode::serialize(message)
                .map_err(|e| AdicError::Serialization(format!("Bincode encoding failed: {}", e)))?,
        };

        // Then optionally compress
        if self.compression != CompressionLevel::None {
            compress(&encoded, self.compression)
        } else {
            Ok(encoded)
        }
    }

    pub fn decode(&self, data: &[u8]) -> Result<AdicMessage> {
        // First decompress if needed
        let decompressed = if self.compression != CompressionLevel::None {
            decompress(data)?
        } else {
            data.to_vec()
        };

        // Then decode the message
        match self.encoding {
            EncodingType::Json => serde_json::from_slice(&decompressed)
                .map_err(|e| AdicError::Serialization(format!("JSON decoding failed: {}", e))),
            EncodingType::Protobuf => {
                // Would need protobuf definitions
                // For now, use JSON
                serde_json::from_slice(&decompressed).map_err(|e| {
                    AdicError::Serialization(format!("Protobuf decoding failed: {}", e))
                })
            }
            EncodingType::Bincode => bincode::deserialize(&decompressed)
                .map_err(|e| AdicError::Serialization(format!("Bincode decoding failed: {}", e))),
        }
    }

    pub fn encode_static(message: &AdicMessage, encoding: EncodingType) -> Result<Vec<u8>> {
        let codec = Self::with_encoding(encoding);
        codec.encode(message)
    }

    pub fn decode_static(data: &[u8], encoding: EncodingType) -> Result<AdicMessage> {
        let codec = Self::with_encoding(encoding);
        codec.decode(data)
    }
}

impl Default for MessageCodec {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::{AdicFeatures, AdicMeta, PublicKey};
    use chrono::Utc;

    #[test]
    fn test_message_codec_new() {
        let codec = MessageCodec::new();
        let message = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![1, 2, 3],
        );

        let encoded = codec.encode(&message).unwrap();
        let decoded = codec.decode(&encoded).unwrap();

        assert_eq!(decoded.id, message.id);
        assert_eq!(decoded.data, message.data);
    }

    #[test]
    fn test_message_codec_with_compression() {
        let mut codec = MessageCodec::new();
        codec.set_compression(CompressionLevel::Fast);

        let message = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![1; 1000], // Compressible data
        );

        let encoded = codec.encode(&message).unwrap();
        let decoded = codec.decode(&encoded).unwrap();

        assert_eq!(decoded.id, message.id);
        assert_eq!(decoded.data, message.data);
    }

    #[test]
    fn test_message_codec_bincode() {
        let codec = MessageCodec::with_encoding(EncodingType::Bincode);

        let message = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![1, 2, 3],
        );

        let encoded = codec.encode(&message).unwrap();
        let decoded = codec.decode(&encoded).unwrap();

        assert_eq!(decoded.id, message.id);
        assert_eq!(decoded.data, message.data);
    }
}
