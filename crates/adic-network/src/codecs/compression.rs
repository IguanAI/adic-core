use adic_types::{AdicError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompressionLevel {
    None = 0,
    Fast = 1,
    Balanced = 2,
    Best = 3,
}

impl CompressionLevel {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(CompressionLevel::None),
            1 => Some(CompressionLevel::Fast),
            2 => Some(CompressionLevel::Balanced),
            3 => Some(CompressionLevel::Best),
            _ => None,
        }
    }

    /// Convert to snap compression level
    #[allow(dead_code)]
    fn to_snap_level(self) -> Option<i32> {
        match self {
            CompressionLevel::None => None,
            CompressionLevel::Fast => Some(1),
            CompressionLevel::Balanced => Some(6),
            CompressionLevel::Best => Some(9),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum CompressionType {
    None,
    Zstd,
    Lz4,
    Snappy,
}

pub struct Compressor;

impl Compressor {
    pub fn compress(data: &[u8], compression: CompressionType) -> Result<Vec<u8>> {
        match compression {
            CompressionType::None => Ok(data.to_vec()),
            CompressionType::Zstd | CompressionType::Snappy => {
                let mut encoder = snap::raw::Encoder::new();
                encoder
                    .compress_vec(data)
                    .map_err(|e| AdicError::Serialization(format!("Compression failed: {}", e)))
            }
            CompressionType::Lz4 => {
                // Would use lz4 crate
                // For now, fallback to snap
                Self::compress(data, CompressionType::Snappy)
            }
        }
    }

    pub fn decompress(data: &[u8], compression: CompressionType) -> Result<Vec<u8>> {
        match compression {
            CompressionType::None => Ok(data.to_vec()),
            CompressionType::Zstd | CompressionType::Snappy => {
                let mut decoder = snap::raw::Decoder::new();
                decoder
                    .decompress_vec(data)
                    .map_err(|e| AdicError::Serialization(format!("Decompression failed: {}", e)))
            }
            CompressionType::Lz4 => {
                // Would use lz4 crate
                // For now, fallback to snap
                Self::decompress(data, CompressionType::Snappy)
            }
        }
    }
}

/// Compress data with specified compression level
pub fn compress(data: &[u8], level: CompressionLevel) -> Result<Vec<u8>> {
    if data.is_empty() {
        return Ok(vec![level as u8]);
    }

    match level {
        CompressionLevel::None => {
            let mut result = vec![level as u8];
            result.extend_from_slice(data);
            Ok(result)
        }
        _ => {
            let mut encoder = snap::raw::Encoder::new();
            let compressed = encoder
                .compress_vec(data)
                .map_err(|e| AdicError::Serialization(format!("Compression failed: {}", e)))?;

            let mut result = vec![level as u8];
            result.extend_from_slice(&compressed);
            Ok(result)
        }
    }
}

/// Decompress data that was compressed with compress()
pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    if data.is_empty() {
        return Err(AdicError::Serialization(
            "Empty data for decompression".to_string(),
        ));
    }

    let level = CompressionLevel::from_u8(data[0])
        .ok_or_else(|| AdicError::Serialization("Invalid compression level".to_string()))?;

    if data.len() == 1 {
        // Only header, no actual data
        return Ok(Vec::new());
    }

    match level {
        CompressionLevel::None => Ok(data[1..].to_vec()),
        _ => {
            let mut decoder = snap::raw::Decoder::new();
            decoder
                .decompress_vec(&data[1..])
                .map_err(|e| AdicError::Serialization(format!("Decompression failed: {}", e)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_level_conversion() {
        assert_eq!(CompressionLevel::from_u8(0), Some(CompressionLevel::None));
        assert_eq!(CompressionLevel::from_u8(1), Some(CompressionLevel::Fast));
        assert_eq!(
            CompressionLevel::from_u8(2),
            Some(CompressionLevel::Balanced)
        );
        assert_eq!(CompressionLevel::from_u8(3), Some(CompressionLevel::Best));
        assert_eq!(CompressionLevel::from_u8(255), None);
    }

    #[test]
    fn test_compress_decompress_none() {
        let data = b"Hello, World!";
        let compressed = compress(data, CompressionLevel::None).unwrap();
        assert_eq!(compressed[0], 0);
        assert_eq!(&compressed[1..], data);

        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(&decompressed, data);
    }

    #[test]
    fn test_compress_decompress_fast() {
        let data = b"Hello, World! This is a test message that should be compressed.";
        let compressed = compress(data, CompressionLevel::Fast).unwrap();
        assert_eq!(compressed[0], 1);

        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(&decompressed, data);
    }

    #[test]
    fn test_compress_decompress_empty() {
        let data = b"";
        let compressed = compress(data, CompressionLevel::Balanced).unwrap();
        assert_eq!(compressed.len(), 1); // Just the header

        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed.len(), 0);
    }

    #[test]
    fn test_decompress_invalid() {
        let invalid_data = vec![255u8]; // Invalid compression level
        let result = decompress(&invalid_data);
        assert!(result.is_err());

        let empty_data = vec![];
        let result = decompress(&empty_data);
        assert!(result.is_err());
    }
}
