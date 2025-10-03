#[cfg(test)]
mod tests {
    use crate::codecs::{compression::*, message::*};
    use adic_types::features::{AxisPhi, QpDigits};
    use adic_types::{AdicFeatures, AdicMessage, AdicMeta, MessageId, PublicKey};
    use chrono::Utc;

    fn create_test_message() -> AdicMessage {
        AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![
                AxisPhi::new(0, QpDigits::from_u64(10, 3, 5)),
                AxisPhi::new(1, QpDigits::from_u64(20, 3, 5)),
            ]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([1; 32]),
            vec![1, 2, 3, 4, 5],
        )
    }

    #[test]
    fn test_compression_levels() {
        assert_eq!(CompressionLevel::None as u8, 0);
        assert_eq!(CompressionLevel::Fast as u8, 1);
        assert_eq!(CompressionLevel::Balanced as u8, 2);
        assert_eq!(CompressionLevel::Best as u8, 3);
    }

    #[test]
    fn test_compression_none() {
        let data = b"Hello, World!";
        let compressed = compress(data, CompressionLevel::None).unwrap();

        // With no compression, should just add a header
        assert_eq!(&compressed[1..], data);
        assert_eq!(compressed[0], CompressionLevel::None as u8);

        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(&decompressed, data);
    }

    #[test]
    fn test_compression_fast() {
        // Use repetitive data that will compress well
        let data = b"Hello, World! Hello, World! Hello, World! Hello, World! Hello, World!";
        let compressed = compress(data, CompressionLevel::Fast).unwrap();

        assert_eq!(compressed[0], CompressionLevel::Fast as u8);

        // For repetitive data, compression should reduce size
        // But we don't guarantee it for all data

        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(&decompressed, data);
    }

    #[test]
    fn test_compression_balanced() {
        let data = vec![42u8; 1000]; // Highly compressible data
        let compressed = compress(&data, CompressionLevel::Balanced).unwrap();

        assert_eq!(compressed[0], CompressionLevel::Balanced as u8);
        assert!(compressed.len() < data.len());

        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_compression_best() {
        let data = vec![1, 2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3, 4, 5];
        let compressed = compress(&data, CompressionLevel::Best).unwrap();

        assert_eq!(compressed[0], CompressionLevel::Best as u8);

        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_compression_empty_data() {
        let data = b"";
        let compressed = compress(data, CompressionLevel::Balanced).unwrap();

        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(&decompressed, data);
    }

    #[test]
    fn test_decompress_invalid_header() {
        let invalid_data = vec![255u8]; // Invalid compression level
        let result = decompress(&invalid_data);
        assert!(result.is_err());
    }

    #[test]
    fn test_message_codec_encode_decode() {
        let message = create_test_message();
        let codec = MessageCodec::new();

        let encoded = codec.encode(&message).unwrap();
        assert!(!encoded.is_empty());

        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded.id, message.id);
        assert_eq!(decoded.parents, message.parents);
        assert_eq!(decoded.data, message.data);
        assert_eq!(decoded.proposer_pk, message.proposer_pk);
    }

    #[test]
    fn test_message_codec_with_compression() {
        let message = create_test_message();
        let mut codec = MessageCodec::new();
        codec.set_compression(CompressionLevel::Balanced);

        let encoded = codec.encode(&message).unwrap();
        let decoded = codec.decode(&encoded).unwrap();

        assert_eq!(decoded.id, message.id);
        assert_eq!(decoded.data, message.data);
    }

    #[test]
    fn test_message_codec_invalid_data() {
        let codec = MessageCodec::new();
        let invalid_data = vec![0, 1, 2, 3];

        let result = codec.decode(&invalid_data);
        assert!(result.is_err());
    }

    #[test]
    fn test_message_codec_empty_message() {
        let message = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![],
        );

        let codec = MessageCodec::new();
        let encoded = codec.encode(&message).unwrap();
        let decoded = codec.decode(&encoded).unwrap();

        assert_eq!(decoded.parents.len(), 0);
        assert_eq!(decoded.data.len(), 0);
    }

    #[test]
    fn test_message_codec_with_parents() {
        let parent1 = MessageId::new(b"parent1");
        let parent2 = MessageId::new(b"parent2");

        let mut message = create_test_message();
        message.parents = vec![parent1, parent2];

        let codec = MessageCodec::new();
        let encoded = codec.encode(&message).unwrap();
        let decoded = codec.decode(&encoded).unwrap();

        assert_eq!(decoded.parents.len(), 2);
        assert_eq!(decoded.parents[0], parent1);
        assert_eq!(decoded.parents[1], parent2);
    }

    #[test]
    fn test_compression_ratio() {
        // Test with repetitive data that should compress well
        let data = "The quick brown fox jumps over the lazy dog. "
            .repeat(100)
            .into_bytes();

        let compressed_fast = compress(&data, CompressionLevel::Fast).unwrap();
        let compressed_best = compress(&data, CompressionLevel::Best).unwrap();

        // Best compression should be smaller or equal to fast
        assert!(compressed_best.len() <= compressed_fast.len());

        // Both should be significantly smaller than original
        assert!(compressed_fast.len() < data.len() / 2);
        assert!(compressed_best.len() < data.len() / 2);
    }
}
