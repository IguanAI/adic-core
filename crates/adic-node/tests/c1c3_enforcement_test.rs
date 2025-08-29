use adic_node::{AdicNode, NodeConfig};
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn test_node_initialization() {
    // Create a temporary directory for test data
    let temp_dir = TempDir::new().unwrap();

    // Test that node can be initialized with default config
    let mut config = NodeConfig::default();
    config.node.data_dir = temp_dir.path().to_path_buf();
    // Use in-memory storage for tests to avoid persistence
    config.storage.backend = "memory".to_string();

    let node = Arc::new(AdicNode::new(config).await.unwrap());

    // Verify node stats
    let stats = node.get_stats().await.unwrap();
    assert_eq!(stats.message_count, 0, "New node should have no messages");
    assert_eq!(stats.tip_count, 0, "New node should have no tips");
}

// Note: More complex C1-C3 enforcement tests should be in the consensus module
// where we have direct access to the consensus engine and can properly test
// the admissibility rules. These tests would be:
// - crates/adic-consensus/src/admissibility.rs (unit tests)
// - crates/adic-consensus/tests/ (integration tests)
