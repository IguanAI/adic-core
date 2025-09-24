use anyhow::{anyhow, Result};
use bech32::{Bech32, Hrp};

const ADIC_HRP: &str = "adic";

pub fn encode_address(bytes: &[u8; 32]) -> Result<String> {
    let hrp = Hrp::parse(ADIC_HRP)?;

    let address = bech32::encode::<Bech32>(hrp, bytes)
        .map_err(|e| anyhow!("Failed to encode address: {}", e))?;

    Ok(address)
}

pub fn decode_address(address: &str) -> Result<[u8; 32]> {
    let (hrp, data) =
        bech32::decode(address).map_err(|e| anyhow!("Failed to decode address: {}", e))?;

    if hrp.as_str() != ADIC_HRP {
        return Err(anyhow!(
            "Invalid address prefix: expected '{}', got '{}'",
            ADIC_HRP,
            hrp.as_str()
        ));
    }

    if data.len() != 32 {
        return Err(anyhow!(
            "Invalid address length: expected 32 bytes, got {}",
            data.len()
        ));
    }

    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&data);

    Ok(bytes)
}

pub fn validate_address(address: &str) -> bool {
    decode_address(address).is_ok()
}

pub fn is_hex_address(address: &str) -> bool {
    let hex_str = address.strip_prefix("0x").unwrap_or(address);
    hex::decode(hex_str).map(|b| b.len() == 32).unwrap_or(false)
}

pub fn from_hex_address(hex_addr: &str) -> Result<[u8; 32]> {
    let hex_str = hex_addr.strip_prefix("0x").unwrap_or(hex_addr);

    let bytes = hex::decode(hex_str).map_err(|e| anyhow!("Invalid hex address: {}", e))?;

    if bytes.len() != 32 {
        return Err(anyhow!(
            "Invalid address length: expected 32 bytes, got {}",
            bytes.len()
        ));
    }

    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode() {
        let bytes = [0x12; 32];
        let encoded = encode_address(&bytes).unwrap();

        assert!(encoded.starts_with("adic1"));

        let decoded = decode_address(&encoded).unwrap();
        assert_eq!(decoded, bytes);
    }

    #[test]
    fn test_validate_address() {
        let bytes = [0x55; 32];
        let valid_address = encode_address(&bytes).unwrap();

        assert!(validate_address(&valid_address));
        assert!(!validate_address("invalid_address"));
        assert!(!validate_address("btc1abc123")); // Wrong prefix
        assert!(!validate_address("adictest1abc123")); // We don't support testnet prefix anymore
    }

    #[test]
    fn test_hex_address_compatibility() {
        let bytes = [0xFF; 32];
        let hex_with_prefix = format!("0x{}", hex::encode(bytes));
        let hex_without_prefix = hex::encode(bytes);

        assert!(is_hex_address(&hex_with_prefix));
        assert!(is_hex_address(&hex_without_prefix));
        assert!(!is_hex_address("not_hex"));

        let decoded1 = from_hex_address(&hex_with_prefix).unwrap();
        let decoded2 = from_hex_address(&hex_without_prefix).unwrap();

        assert_eq!(decoded1, bytes);
        assert_eq!(decoded2, bytes);
    }

    #[test]
    fn test_different_addresses() {
        let addr1 = [0x01; 32];
        let addr2 = [0x02; 32];

        let encoded1 = encode_address(&addr1).unwrap();
        let encoded2 = encode_address(&addr2).unwrap();

        assert_ne!(encoded1, encoded2);

        let decoded1 = decode_address(&encoded1).unwrap();
        let decoded2 = decode_address(&encoded2).unwrap();

        assert_eq!(decoded1, addr1);
        assert_eq!(decoded2, addr2);
    }

    #[test]
    fn test_round_trip_various_addresses() {
        // Test various byte patterns
        let test_cases = vec![[0x00; 32], [0xFF; 32], [0xAA; 32], {
            let mut bytes = [0u8; 32];
            for (i, byte) in bytes.iter_mut().enumerate() {
                *byte = i as u8;
            }
            bytes
        }];

        for original in test_cases {
            let encoded = encode_address(&original).unwrap();
            let decoded = decode_address(&encoded).unwrap();
            assert_eq!(original, decoded, "Round trip failed for {:?}", original);
        }
    }
}
