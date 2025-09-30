use adic_node::genesis::{GenesisConfig, GenesisManifest};
use adic_node::{AdicNode, NodeConfig};
use tempfile::TempDir;

#[tokio::test]
async fn test_non_bootstrap_genesis_validation() {
    let temp_dir = TempDir::new().unwrap();

    // Create a genesis manifest with the canonical hash
    let genesis_manifest = GenesisManifest::new();
    let genesis_json = serde_json::to_string_pretty(&genesis_manifest).unwrap();

    // Write genesis.json to the temp directory
    let genesis_path = temp_dir.path().join("genesis.json");
    std::fs::write(&genesis_path, &genesis_json).unwrap();

    // Create a non-bootstrap node config
    let mut config = NodeConfig::default();
    config.node.data_dir = temp_dir.path().to_path_buf();
    config.node.bootstrap = Some(false); // Non-bootstrap node
    config.storage.backend = "memory".to_string();
    config.network.enabled = false;

    // Node should initialize successfully with correct genesis
    let result = AdicNode::new(config).await;
    assert!(
        result.is_ok(),
        "Node should initialize with valid genesis.json"
    );
}

#[tokio::test]
async fn test_genesis_hash_mismatch_rejected() {
    let temp_dir = TempDir::new().unwrap();

    // Create a genesis manifest with a different hash (corrupted)
    let mut genesis_manifest = GenesisManifest::new();
    genesis_manifest.hash = "invalid_hash_that_does_not_match".to_string();

    let genesis_json = serde_json::to_string_pretty(&genesis_manifest).unwrap();

    // Write corrupted genesis.json
    let genesis_path = temp_dir.path().join("genesis.json");
    std::fs::write(&genesis_path, &genesis_json).unwrap();

    // Create a non-bootstrap node config
    let mut config = NodeConfig::default();
    config.node.data_dir = temp_dir.path().to_path_buf();
    config.node.bootstrap = Some(false);
    config.storage.backend = "memory".to_string();
    config.network.enabled = false;

    // Node should fail to initialize with mismatched genesis hash
    let result = AdicNode::new(config).await;
    assert!(
        result.is_err(),
        "Node should reject genesis with mismatched hash"
    );

    if let Err(e) = result {
        let error_msg = e.to_string();
        assert!(
            error_msg.contains("hash mismatch") || error_msg.contains("Invalid genesis"),
            "Error should indicate hash mismatch, got: {}",
            error_msg
        );
    }
}

#[tokio::test]
async fn test_genesis_hash_validation_against_canonical() {
    let temp_dir = TempDir::new().unwrap();

    // Create a genesis with different allocations
    let mut different_genesis = GenesisConfig::default();
    different_genesis.allocations[0].1 = 999999; // Change treasury amount

    let manifest = GenesisManifest {
        config: different_genesis.clone(),
        hash: different_genesis.calculate_hash(),
        version: "1.0.0".to_string(),
    };

    // Even though the manifest is internally consistent, it doesn't match canonical
    let genesis_json = serde_json::to_string_pretty(&manifest).unwrap();
    let genesis_path = temp_dir.path().join("genesis.json");
    std::fs::write(&genesis_path, &genesis_json).unwrap();

    let mut config = NodeConfig::default();
    config.node.data_dir = temp_dir.path().to_path_buf();
    config.node.bootstrap = Some(false);
    config.storage.backend = "memory".to_string();
    config.network.enabled = false;

    // This should fail because the hash doesn't match the canonical network hash
    let result = AdicNode::new(config).await;
    assert!(
        result.is_err(),
        "Node should reject genesis that doesn't match canonical hash"
    );

    if let Err(e) = result {
        let error_msg = e.to_string();
        assert!(
            error_msg.contains("canonical") || error_msg.contains("mismatch"),
            "Error should mention canonical hash mismatch, got: {}",
            error_msg
        );
    }
}

#[tokio::test]
async fn test_bootstrap_node_creates_genesis() {
    let temp_dir = TempDir::new().unwrap();

    // Bootstrap node shouldn't need genesis.json file
    let mut config = NodeConfig::default();
    config.node.data_dir = temp_dir.path().to_path_buf();
    config.node.bootstrap = Some(true); // Bootstrap node
    config.storage.backend = "memory".to_string();
    config.network.enabled = false;

    // Bootstrap node should initialize and create genesis
    let result = AdicNode::new(config).await;
    assert!(
        result.is_ok(),
        "Bootstrap node should initialize without genesis.json"
    );

    // Verify genesis.json was created
    let genesis_path = temp_dir.path().join("genesis.json");
    assert!(
        genesis_path.exists(),
        "Bootstrap node should create genesis.json"
    );

    // Verify the created genesis has correct hash
    let genesis_data = std::fs::read_to_string(&genesis_path).unwrap();
    let manifest: GenesisManifest = serde_json::from_str(&genesis_data).unwrap();
    assert_eq!(
        manifest.hash,
        GenesisManifest::canonical_hash(),
        "Bootstrap-created genesis should have canonical hash"
    );
}

#[test]
fn test_genesis_manifest_verify() {
    // Valid manifest should pass verification
    let valid_manifest = GenesisManifest::new();
    assert!(
        valid_manifest.verify().is_ok(),
        "Valid genesis manifest should pass verification"
    );

    // Manifest with corrupted hash should fail
    let mut invalid_manifest = GenesisManifest::new();
    invalid_manifest.hash = "corrupted_hash".to_string();
    assert!(
        invalid_manifest.verify().is_err(),
        "Corrupted genesis manifest should fail verification"
    );

    // Manifest with invalid allocations should fail
    let invalid_config = GenesisConfig {
        allocations: vec![], // Empty allocations
        ..Default::default()
    };

    let manifest_with_invalid_config = GenesisManifest {
        config: invalid_config,
        hash: "some_hash".to_string(),
        version: "1.0.0".to_string(),
    };

    assert!(
        manifest_with_invalid_config.verify().is_err(),
        "Genesis with invalid config should fail verification"
    );
}
