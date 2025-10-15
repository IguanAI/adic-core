//! RocksDB key prefix definitions using Blake3 hashing
//!
//! All keys use a consistent 10-byte prefix for RocksDB prefix extraction optimization.
//! This enables memtable prefix bloom filters and faster range scans.

/// Fixed prefix length for all RocksDB keys
pub const PREFIX_LEN: usize = 10;

// Compile-time computed prefixes using Blake3 hash
// Format: First 10 bytes of blake3(prefix_name)

lazy_static::lazy_static! {
    /// Message storage: `msg:{message_id}`
    pub static ref PREFIX_MSG: [u8; PREFIX_LEN] = hash_prefix(b"msg");

    /// Parent-child relationship: `pc:{parent_id}:{child_id}`
    pub static ref PREFIX_PC: [u8; PREFIX_LEN] = hash_prefix(b"pc");

    /// Child-parent relationship: `cp:{child_id}:{parent_id}`
    pub static ref PREFIX_CP: [u8; PREFIX_LEN] = hash_prefix(b"cp");

    /// DAG tips: `tip:{message_id}`
    pub static ref PREFIX_TIP: [u8; PREFIX_LEN] = hash_prefix(b"tip");

    /// Metadata: `meta:{message_id}:{key}`
    pub static ref PREFIX_META: [u8; PREFIX_LEN] = hash_prefix(b"meta");

    /// Reputation: `rep:{pubkey}`
    pub static ref PREFIX_REP: [u8; PREFIX_LEN] = hash_prefix(b"rep");

    /// Finality status: `fin:{message_id}`
    pub static ref PREFIX_FIN: [u8; PREFIX_LEN] = hash_prefix(b"fin");

    /// Finality artifacts: `fin_art:{message_id}`
    pub static ref PREFIX_FIN_ART: [u8; PREFIX_LEN] = hash_prefix(b"fin_art");

    /// Conflict sets: `conflict:{conflict_id}:{message_id}`
    pub static ref PREFIX_CONFLICT: [u8; PREFIX_LEN] = hash_prefix(b"conflict");

    /// P-adic ball index: `ball:{axis}:{ball_id}:{message_id}`
    pub static ref PREFIX_BALL: [u8; PREFIX_LEN] = hash_prefix(b"ball");

    /// Time index: `msg_by_time:{timestamp}:{message_id}`
    pub static ref PREFIX_MSG_BY_TIME: [u8; PREFIX_LEN] = hash_prefix(b"msg_by_time");

    /// Transaction by address index: `tx_by_addr:{address}:{timestamp}:{tx_hash}`
    pub static ref PREFIX_TX_BY_ADDR: [u8; PREFIX_LEN] = hash_prefix(b"tx_by_addr");
}

/// Hash a prefix string to a fixed 10-byte array
fn hash_prefix(prefix: &[u8]) -> [u8; PREFIX_LEN] {
    let hash = blake3::hash(prefix);
    let mut result = [0u8; PREFIX_LEN];
    result.copy_from_slice(&hash.as_bytes()[..PREFIX_LEN]);
    result
}

/// Decode a prefix hash back to its name (for debugging/tooling)
pub fn prefix_name(prefix: &[u8; PREFIX_LEN]) -> Option<&'static str> {
    if prefix == &*PREFIX_MSG {
        Some("msg")
    } else if prefix == &*PREFIX_PC {
        Some("pc")
    } else if prefix == &*PREFIX_CP {
        Some("cp")
    } else if prefix == &*PREFIX_TIP {
        Some("tip")
    } else if prefix == &*PREFIX_META {
        Some("meta")
    } else if prefix == &*PREFIX_REP {
        Some("rep")
    } else if prefix == &*PREFIX_FIN {
        Some("fin")
    } else if prefix == &*PREFIX_FIN_ART {
        Some("fin_art")
    } else if prefix == &*PREFIX_CONFLICT {
        Some("conflict")
    } else if prefix == &*PREFIX_BALL {
        Some("ball")
    } else if prefix == &*PREFIX_MSG_BY_TIME {
        Some("msg_by_time")
    } else if prefix == &*PREFIX_TX_BY_ADDR {
        Some("tx_by_addr")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefix_uniqueness() {
        // Ensure all prefixes are unique (no collisions)
        let prefixes = vec![
            *PREFIX_MSG,
            *PREFIX_PC,
            *PREFIX_CP,
            *PREFIX_TIP,
            *PREFIX_META,
            *PREFIX_REP,
            *PREFIX_FIN,
            *PREFIX_FIN_ART,
            *PREFIX_CONFLICT,
            *PREFIX_BALL,
            *PREFIX_MSG_BY_TIME,
            *PREFIX_TX_BY_ADDR,
        ];

        for i in 0..prefixes.len() {
            for j in (i + 1)..prefixes.len() {
                assert_ne!(
                    prefixes[i], prefixes[j],
                    "Prefix collision detected at indices {} and {}",
                    i, j
                );
            }
        }
    }

    #[test]
    fn test_prefix_length() {
        assert_eq!(PREFIX_MSG.len(), PREFIX_LEN);
        assert_eq!(PREFIX_PC.len(), PREFIX_LEN);
        assert_eq!(PREFIX_TIP.len(), PREFIX_LEN);
    }

    #[test]
    fn test_prefix_decoder() {
        assert_eq!(prefix_name(&*PREFIX_MSG), Some("msg"));
        assert_eq!(prefix_name(&*PREFIX_PC), Some("pc"));
        assert_eq!(prefix_name(&*PREFIX_TIP), Some("tip"));
        assert_eq!(prefix_name(&*PREFIX_META), Some("meta"));
        assert_eq!(prefix_name(&*PREFIX_FIN_ART), Some("fin_art"));

        // Unknown prefix
        let unknown = [0xFF; PREFIX_LEN];
        assert_eq!(prefix_name(&unknown), None);
    }

    #[test]
    fn test_hash_determinism() {
        // Ensure hashing is deterministic
        let hash1 = hash_prefix(b"msg");
        let hash2 = hash_prefix(b"msg");
        assert_eq!(hash1, hash2);
    }
}
