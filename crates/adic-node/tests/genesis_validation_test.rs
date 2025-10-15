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

/// SECURITY TEST: Custom network with custom genesis should be allowed (but warned)
#[test]
fn test_custom_network_allowed() {
    // Someone creates a custom network (not claiming to be mainnet)
    // This is allowed for private networks, testing, etc.
    let mut custom_config = GenesisConfig::default();
    custom_config.chain_id = "my-custom-network".to_string(); // Not "adic-dag-v1"
    custom_config.allocations[0].1 = 200_000_000; // Custom allocations

    // This should SUCCEED - custom networks can have custom genesis
    let result = custom_config.verify_with_canonical(true); // true = bootstrap
    assert!(
        result.is_ok(),
        "Custom network bootstrap should be allowed with custom genesis: {:?}",
        result
    );
}

/// SECURITY TEST: Using canonical genesis with wrong chain_id should fail
#[test]
fn test_canonical_genesis_requires_correct_chain_id() {
    // Attacker tries to use canonical mainnet genesis but with different chain_id
    // to confuse users into thinking it's mainnet
    let mut sneaky_config = GenesisConfig::default(); // Default = canonical mainnet
    sneaky_config.chain_id = "adic-mainnet-v1".to_string(); // Wrong chain_id!

    // This should FAIL - if you use canonical genesis, must use correct chain_id
    let result = sneaky_config.verify_with_canonical(true); // true = bootstrap
    assert!(
        result.is_err(),
        "Bootstrap with canonical genesis but wrong chain_id should fail"
    );

    if let Err(e) = result {
        assert!(
            e.contains("SECURITY") || e.contains("chain_id"),
            "Error should mention security/chain_id issue, got: {}",
            e
        );
    }
}

/// SECURITY TEST: Attempt to use mainnet chain_id with manipulated allocations
#[test]
fn test_bypass_attempt_mainnet_manipulated() {
    // Attacker modifies genesis allocations but keeps mainnet chain_id
    let mut malicious_config = GenesisConfig::default();
    malicious_config.chain_id = "adic-dag-v1".to_string(); // Correct mainnet chain_id
    malicious_config.allocations[0].1 = 999_999_999; // Manipulated allocation

    // This should FAIL because hash won't match canonical
    let result = malicious_config.verify_with_canonical(true); // true = bootstrap
    assert!(
        result.is_err(),
        "Bootstrap node should reject manipulated mainnet genesis"
    );

    // Error should mention hash mismatch
    if let Err(e) = result {
        assert!(
            e.contains("hash") || e.contains("canonical"),
            "Error should mention hash/canonical mismatch, got: {}",
            e
        );
    }
}

/// SECURITY TEST: Verify testnet allows custom genesis (not mainnet)
#[test]
fn test_testnet_allows_custom_genesis() {
    // Testnet should allow custom allocations (for testing)
    let mut testnet_config = GenesisConfig::default();
    testnet_config.chain_id = "adic-testnet".to_string();
    testnet_config.allocations[0].1 = 123_456_789; // Custom allocation

    // This should SUCCEED for testnet bootstrap
    let result = testnet_config.verify_with_canonical(true); // true = bootstrap
    assert!(
        result.is_ok(),
        "Testnet bootstrap should allow custom genesis"
    );
}

/// SECURITY TEST: Verify devnet allows custom genesis
#[test]
fn test_devnet_allows_custom_genesis() {
    // Devnet should allow custom allocations (for development)
    let mut devnet_config = GenesisConfig::default();
    devnet_config.chain_id = "adic-devnet".to_string();
    devnet_config.allocations = vec![("test_address".to_string(), 1000)];

    // This should SUCCEED for devnet bootstrap
    let result = devnet_config.verify_with_canonical(true); // true = bootstrap
    assert!(result.is_ok(), "Devnet bootstrap should allow custom genesis");
}

/// SECURITY TEST: Canonical mainnet genesis should always validate
#[test]
fn test_canonical_mainnet_validates() {
    // The default genesis config is the canonical mainnet config
    let mainnet_config = GenesisConfig::default();

    // Should validate successfully for bootstrap
    let result = mainnet_config.verify_with_canonical(true);
    assert!(
        result.is_ok(),
        "Canonical mainnet genesis should validate: {:?}",
        result
    );

    // Should validate successfully for non-bootstrap
    let result = mainnet_config.verify_with_canonical(false);
    assert!(
        result.is_ok(),
        "Canonical mainnet genesis should validate for regular nodes: {:?}",
        result
    );
}

/// SECURITY TEST: Verify NetworkType detection
#[test]
fn test_network_type_detection() {
    use adic_node::genesis::NetworkType;

    // Test chain_id detection
    assert_eq!(
        NetworkType::from_chain_id("adic-dag-v1"),
        NetworkType::Mainnet
    );
    assert_eq!(
        NetworkType::from_chain_id("adic-testnet"),
        NetworkType::Testnet
    );
    assert_eq!(
        NetworkType::from_chain_id("adic-devnet"),
        NetworkType::Devnet
    );
    assert_eq!(
        NetworkType::from_chain_id("custom-network"),
        NetworkType::Custom
    );

    // Test canonical hash requirement
    assert!(NetworkType::Mainnet.requires_canonical_genesis());
    assert!(!NetworkType::Testnet.requires_canonical_genesis());
    assert!(!NetworkType::Devnet.requires_canonical_genesis());
    assert!(!NetworkType::Custom.requires_canonical_genesis());
}
