use crate::backend::{StorageBackend, StorageError, Result};
use adic_types::{MessageId, AdicMessage};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Snapshot metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    pub version: u32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub message_count: usize,
    pub tip_count: usize,
    pub finalized_count: usize,
    pub block_height: Option<u64>,
    pub hash: Vec<u8>,
}

/// Snapshot data container
#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub metadata: SnapshotMetadata,
    pub messages: Vec<AdicMessage>,
    pub tips: HashSet<MessageId>,
    pub finalized: HashMap<MessageId, Vec<u8>>,
    pub reputation: HashMap<[u8; 32], f64>,
    pub conflicts: HashMap<String, HashSet<MessageId>>,
}

impl Snapshot {
    /// Create a new snapshot from current state
    pub async fn create(backend: &dyn StorageBackend) -> Result<Self> {
        // Collect all messages
        let message_ids = backend.list_messages().await?;
        let mut messages = Vec::with_capacity(message_ids.len());
        
        for id in &message_ids {
            if let Some(msg) = backend.get_message(id).await? {
                messages.push(msg);
            }
        }

        // Collect tips
        let tip_ids = backend.get_tips().await?;
        let tips: HashSet<_> = tip_ids.into_iter().collect();

        // Collect finalized messages
        let mut finalized = HashMap::new();
        for id in &message_ids {
            if backend.is_finalized(id).await? {
                if let Some(artifact) = backend.get_finality_artifact(id).await? {
                    finalized.insert(*id, artifact);
                }
            }
        }

        // Note: Reputation and conflicts would need additional backend methods
        // to enumerate all entries. For now, we'll use empty collections.
        let reputation = HashMap::new();
        let conflicts = HashMap::new();

        // Calculate hash of snapshot data
        let hash = Self::calculate_hash(&messages);

        let metadata = SnapshotMetadata {
            version: 1,
            created_at: chrono::Utc::now(),
            message_count: messages.len(),
            tip_count: tips.len(),
            finalized_count: finalized.len(),
            block_height: None,
            hash,
        };

        Ok(Self {
            metadata,
            messages,
            tips,
            finalized,
            reputation,
            conflicts,
        })
    }

    /// Restore snapshot to backend
    pub async fn restore(&self, backend: &dyn StorageBackend) -> Result<()> {
        // Begin transaction
        backend.begin_transaction().await?;

        // Restore messages
        for message in &self.messages {
            if let Err(e) = backend.put_message(message).await {
                backend.rollback_transaction().await?;
                return Err(e);
            }
        }

        // Restore tips
        for tip_id in &self.tips {
            if let Err(e) = backend.add_tip(tip_id).await {
                backend.rollback_transaction().await?;
                return Err(e);
            }
        }

        // Restore finalized messages
        for (id, artifact) in &self.finalized {
            if let Err(e) = backend.mark_finalized(id, artifact).await {
                backend.rollback_transaction().await?;
                return Err(e);
            }
        }

        // Restore reputation scores
        for (pubkey_bytes, score) in &self.reputation {
            let pubkey = adic_types::PublicKey::from_bytes(*pubkey_bytes);
            if let Err(e) = backend.put_reputation(&pubkey, *score).await {
                backend.rollback_transaction().await?;
                return Err(e);
            }
        }

        // Restore conflicts
        for (conflict_id, message_ids) in &self.conflicts {
            for msg_id in message_ids {
                if let Err(e) = backend.add_to_conflict(conflict_id, msg_id).await {
                    backend.rollback_transaction().await?;
                    return Err(e);
                }
            }
        }

        // Commit transaction
        backend.commit_transaction().await?;

        Ok(())
    }

    /// Calculate hash of messages for integrity
    fn calculate_hash(messages: &[AdicMessage]) -> Vec<u8> {
        use sha2::{Digest, Sha256};
        
        let mut hasher = Sha256::new();
        for msg in messages {
            hasher.update(&msg.id.as_bytes());
        }
        hasher.finalize().to_vec()
    }

    /// Verify snapshot integrity
    pub fn verify(&self) -> bool {
        let calculated_hash = Self::calculate_hash(&self.messages);
        calculated_hash == self.metadata.hash
    }

    /// Save snapshot to file
    pub async fn save_to_file(&self, path: &Path) -> Result<()> {
        let data = bincode::serialize(self)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        let mut file = fs::File::create(path).await?;
        file.write_all(&data).await?;
        file.flush().await?;
        
        Ok(())
    }

    /// Load snapshot from file
    pub async fn load_from_file(path: &Path) -> Result<Self> {
        let mut file = fs::File::open(path).await?;
        let mut data = Vec::new();
        file.read_to_end(&mut data).await?;
        
        let snapshot: Self = bincode::deserialize(&data)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        if !snapshot.verify() {
            return Err(StorageError::BackendError("Snapshot verification failed".into()));
        }
        
        Ok(snapshot)
    }
}

/// Manager for creating and managing snapshots
pub struct SnapshotManager {
    snapshot_dir: PathBuf,
    max_snapshots: usize,
}

impl SnapshotManager {
    pub fn new(snapshot_dir: PathBuf, max_snapshots: usize) -> Self {
        Self {
            snapshot_dir,
            max_snapshots,
        }
    }

    /// Create a new snapshot
    pub async fn create_snapshot(&self, backend: &dyn StorageBackend) -> Result<PathBuf> {
        // Ensure snapshot directory exists
        fs::create_dir_all(&self.snapshot_dir).await?;

        // Create snapshot
        let snapshot = Snapshot::create(backend).await?;
        
        // Generate filename with timestamp
        let filename = format!(
            "snapshot_{}.bin",
            snapshot.metadata.created_at.timestamp()
        );
        let path = self.snapshot_dir.join(filename);

        // Save to file
        snapshot.save_to_file(&path).await?;

        // Clean up old snapshots
        self.cleanup_old_snapshots().await?;

        Ok(path)
    }

    /// List available snapshots
    pub async fn list_snapshots(&self) -> Result<Vec<(PathBuf, SnapshotMetadata)>> {
        let mut snapshots = Vec::new();
        
        let mut entries = fs::read_dir(&self.snapshot_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("bin") {
                if let Ok(snapshot) = Snapshot::load_from_file(&path).await {
                    snapshots.push((path, snapshot.metadata));
                }
            }
        }
        
        // Sort by creation time (newest first)
        snapshots.sort_by(|a, b| b.1.created_at.cmp(&a.1.created_at));
        
        Ok(snapshots)
    }

    /// Load the latest snapshot
    pub async fn load_latest(&self) -> Result<Option<Snapshot>> {
        let snapshots = self.list_snapshots().await?;
        if let Some((path, _)) = snapshots.first() {
            Ok(Some(Snapshot::load_from_file(path).await?))
        } else {
            Ok(None)
        }
    }

    /// Restore from a specific snapshot file
    pub async fn restore_from_file(
        &self,
        path: &Path,
        backend: &dyn StorageBackend,
    ) -> Result<()> {
        let snapshot = Snapshot::load_from_file(path).await?;
        snapshot.restore(backend).await
    }

    /// Clean up old snapshots beyond max_snapshots limit
    async fn cleanup_old_snapshots(&self) -> Result<()> {
        let snapshots = self.list_snapshots().await?;
        
        if snapshots.len() > self.max_snapshots {
            // Delete oldest snapshots
            for (path, _) in snapshots.iter().skip(self.max_snapshots) {
                fs::remove_file(path).await?;
            }
        }
        
        Ok(())
    }

    /// Delete all snapshots
    pub async fn clear_all(&self) -> Result<()> {
        let snapshots = self.list_snapshots().await?;
        for (path, _) in snapshots {
            fs::remove_file(path).await?;
        }
        Ok(())
    }

    /// Get snapshot directory path
    pub fn snapshot_dir(&self) -> &Path {
        &self.snapshot_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryBackend;
    use adic_types::{AdicFeatures, AdicMeta, AxisPhi, QpDigits, PublicKey};
    use chrono::Utc;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_snapshot_create_restore() {
        let backend = MemoryBackend::new();
        
        // Add test data
        let message = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(10, 3, 5))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![1, 2, 3],
        );
        
        backend.put_message(&message).await.unwrap();
        backend.add_tip(&message.id).await.unwrap();
        
        // Create snapshot
        let snapshot = Snapshot::create(&backend).await.unwrap();
        assert_eq!(snapshot.messages.len(), 1);
        assert!(snapshot.tips.contains(&message.id));
        assert!(snapshot.verify());
        
        // Test restore to new backend
        let new_backend = MemoryBackend::new();
        snapshot.restore(&new_backend).await.unwrap();
        
        // Verify restored data
        let restored = new_backend.get_message(&message.id).await.unwrap();
        assert!(restored.is_some());
        assert_eq!(restored.unwrap().id, message.id);
        
        let tips = new_backend.get_tips().await.unwrap();
        assert!(tips.contains(&message.id));
    }

    #[tokio::test]
    async fn test_snapshot_manager() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SnapshotManager::new(temp_dir.path().to_path_buf(), 3);
        
        let backend = MemoryBackend::new();
        
        // Create test message
        let message = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(10, 3, 5))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![],
        );
        backend.put_message(&message).await.unwrap();
        
        // Create snapshot
        let path = manager.create_snapshot(&backend).await.unwrap();
        assert!(path.exists());
        
        // List snapshots
        let snapshots = manager.list_snapshots().await.unwrap();
        assert_eq!(snapshots.len(), 1);
        
        // Load latest
        let latest = manager.load_latest().await.unwrap();
        assert!(latest.is_some());
        assert_eq!(latest.unwrap().messages.len(), 1);
        
        // Test cleanup (create more than max_snapshots)
        for _ in 0..4 {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            manager.create_snapshot(&backend).await.unwrap();
        }
        
        let snapshots = manager.list_snapshots().await.unwrap();
        assert_eq!(snapshots.len(), 3); // Should be limited to max_snapshots
    }
}