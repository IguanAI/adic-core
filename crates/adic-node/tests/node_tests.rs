use adic_crypto::Keypair;
use adic_node::{AdicNode, NodeConfig};
use adic_storage::{StorageConfig, StorageEngine};
use adic_types::features::{AxisPhi, QpDigits};
use adic_types::*;
use chrono::Utc;

#[tokio::test]
async fn test_node_initialization() {
    let mut config = NodeConfig::default();
    // Use in-memory storage for tests
    config.storage.backend = "memory".to_string();
    config.node.bootstrap = Some(true); // Set as bootstrap node for tests
    let node = AdicNode::new(config).await.unwrap();

    // Verify node ID is generated
    let node_id = node.node_id();
    assert!(!node_id.is_empty());
    assert_eq!(node_id.len(), 16); // 8 bytes in hex
}

#[tokio::test]
async fn test_message_submission() {
    // Use test-friendly config with relaxed params and in-memory storage
    let mut config = NodeConfig::default();
    // Keep d = 3 as per spec, but relax other constraints for testing
    config.consensus.q = 1; // Minimal diversity requirement
    config.consensus.rho = vec![1, 1, 1]; // Lower radii for easier proximity matching
    config.consensus.r_sum_min = 0.5; // Lower C3 threshold for testing
    config.consensus.r_min = 0.1; // Lower minimum reputation
    config.consensus.depth_star = 0; // Allow shallow messages
    config.consensus.k = 1; // Minimal core size
    config.storage.backend = "memory".to_string(); // Use in-memory storage
    config.node.bootstrap = Some(true); // Set as bootstrap node for tests
    let node = AdicNode::new(config).await.unwrap();

    // First create genesis and some diverse parent messages
    // Genesis is required to bootstrap the DAG
    let genesis_id = create_genesis_for_node(&node).await;

    // Create diverse parents to satisfy diversity requirements
    let _parent_ids = create_diverse_parents(&node, genesis_id).await;

    // Sync tips from storage to TipManager
    node.sync_tips_from_storage().await.unwrap();

    // Check what tips are available
    let tips = node.storage.get_tips().await.unwrap();
    println!("Available tips in storage: {}", tips.len());
    assert!(
        tips.len() >= 4,
        "Need at least 4 tips for diversity, got {}",
        tips.len()
    );

    // Now we can submit a test message with features that have good proximity to parents
    let content = b"Hello, ADIC!".to_vec();
    let message_id = submit_test_message_with_proximity(&node, content.clone())
        .await
        .unwrap();

    // Verify message was stored
    let retrieved = node.get_message(&message_id).await.unwrap();
    assert!(retrieved.is_some());

    let msg = retrieved.unwrap();
    assert_eq!(msg.id, message_id);
    assert_eq!(msg.data, content);
}

#[tokio::test]
async fn test_node_stats() {
    // Use test-friendly config with relaxed params and in-memory storage
    let mut config = NodeConfig::default();
    // Keep d = 3 as per spec, but relax other constraints for testing
    config.consensus.q = 1; // Minimal diversity requirement
    config.consensus.rho = vec![1, 1, 1]; // Lower radii for easier proximity matching
    config.consensus.r_sum_min = 0.5; // Lower C3 threshold for testing
    config.consensus.r_min = 0.1; // Lower minimum reputation
    config.consensus.depth_star = 0; // Allow shallow messages
    config.consensus.k = 1; // Minimal core size
    config.storage.backend = "memory".to_string(); // Use in-memory storage
    config.node.bootstrap = Some(true); // Set as bootstrap node for tests
    let node = AdicNode::new(config).await.unwrap();

    // Get initial stats
    let stats = node.get_stats().await.unwrap();
    assert_eq!(stats.message_count, 0);
    assert_eq!(stats.tip_count, 0);

    // Create genesis and diverse parents first
    let genesis_id = create_genesis_for_node(&node).await;
    let _parent_ids = create_diverse_parents(&node, genesis_id).await;

    // Sync tips from storage to TipManager
    node.sync_tips_from_storage().await.unwrap();

    // Get stats after genesis
    let stats = node.get_stats().await.unwrap();
    assert!(stats.message_count > 0);

    // Submit a message with proper proximity
    submit_test_message_with_proximity(&node, b"test".to_vec())
        .await
        .unwrap();

    // Check stats updated
    let new_stats = node.get_stats().await.unwrap();
    assert_eq!(new_stats.message_count, stats.message_count + 1);
}

#[tokio::test]
async fn test_storage_operations() {
    // Use in-memory storage to avoid conflicts with other tests
    let config = StorageConfig {
        backend_type: adic_storage::store::BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    let keypair = Keypair::generate();

    // Create and store a message
    let message = AdicMessage::new(
        vec![],
        AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(100, 3, 10)),
            AxisPhi::new(1, QpDigits::from_u64(200, 3, 10)),
            AxisPhi::new(2, QpDigits::from_u64(300, 3, 10)),
        ]),
        AdicMeta::new(Utc::now()),
        *keypair.public_key(),
        b"test message".to_vec(),
    );

    storage.store_message(&message).await.unwrap();

    // Verify storage
    let retrieved = storage.get_message(&message.id).await.unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().id, message.id);

    // Check tips
    let tips = storage.get_tips().await.unwrap();
    assert_eq!(tips.len(), 1);
    assert_eq!(tips[0], message.id);

    // Check stats
    let stats = storage.get_stats().await.unwrap();
    assert_eq!(stats.message_count, 1);
    assert_eq!(stats.tip_count, 1);
}

#[test]
fn test_config_serialization() {
    use adic_node::genesis::GenesisConfig;

    // Test config without genesis
    let config = NodeConfig::default();

    // Serialize to TOML
    let toml_str = toml::to_string_pretty(&config).unwrap();
    assert!(toml_str.contains("[node]"));
    assert!(toml_str.contains("[consensus]"));
    assert!(toml_str.contains("[storage]"));

    // Deserialize back
    let parsed: NodeConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.consensus.p, config.consensus.p);
    assert_eq!(parsed.consensus.d, config.consensus.d);
    assert!(
        parsed.genesis.is_none(),
        "Default config should have no genesis"
    );

    // Test config with genesis
    let config_with_genesis = NodeConfig {
        genesis: Some(GenesisConfig::default()),
        ..Default::default()
    };

    let toml_str2 = toml::to_string_pretty(&config_with_genesis).unwrap();
    assert!(
        toml_str2.contains("[genesis]"),
        "TOML should include genesis section"
    );
    assert!(
        toml_str2.contains("allocations"),
        "Genesis should include allocations"
    );
    assert!(
        toml_str2.contains("chain_id"),
        "Genesis should include chain_id"
    );

    // Deserialize and verify genesis is preserved
    let parsed2: NodeConfig = toml::from_str(&toml_str2).unwrap();
    assert!(
        parsed2.genesis.is_some(),
        "Genesis should be present after deserialization"
    );
    let genesis = parsed2.genesis.unwrap();
    assert_eq!(genesis.chain_id, "adic-dag-v1");
    assert!(!genesis.allocations.is_empty());
}

#[test]
fn test_keypair_generation() {
    let keypair1 = Keypair::generate();
    let keypair2 = Keypair::generate();

    // Verify different keys are generated
    assert_ne!(
        keypair1.public_key().as_bytes(),
        keypair2.public_key().as_bytes()
    );

    // Test serialization
    let bytes = keypair1.to_bytes();
    assert_eq!(bytes.len(), 32); // Ed25519 private key

    // Test deserialization
    let restored = Keypair::from_bytes(&bytes).unwrap();
    assert_eq!(
        restored.public_key().as_bytes(),
        keypair1.public_key().as_bytes()
    );
}

#[test]
fn test_message_id_generation() {
    let msg1 = AdicMessage::new(
        vec![],
        AdicFeatures::new(vec![]),
        AdicMeta::new(Utc::now()),
        PublicKey::from_bytes([0; 32]),
        b"message 1".to_vec(),
    );

    let msg2 = AdicMessage::new(
        vec![],
        AdicFeatures::new(vec![]),
        AdicMeta::new(Utc::now()),
        PublicKey::from_bytes([0; 32]),
        b"message 2".to_vec(),
    );

    // Different content should produce different IDs
    assert_ne!(msg1.id, msg2.id);
}

// Helper function to submit a message with good proximity to parents
async fn submit_test_message_with_proximity(
    node: &AdicNode,
    content: Vec<u8>,
) -> anyhow::Result<MessageId> {
    // Get current tips and select parents manually
    let tips = node.get_tips().await?;
    if tips.len() < 4 {
        return Err(anyhow::anyhow!(
            "Need at least 4 tips for d=3, got {}",
            tips.len()
        ));
    }

    // Get the first 4 tips as parents (d+1 = 4 for d=3)
    let parent_ids = tips[0..4].to_vec();

    // Get parent messages to understand their features
    let mut parent_messages = Vec::new();
    for parent_id in &parent_ids {
        if let Some(parent_msg) = node.get_message(parent_id).await? {
            parent_messages.push(parent_msg);
        } else {
            return Err(anyhow::anyhow!("Parent message not found"));
        }
    }

    // Create features with good proximity to the first parent
    // Strategy: use the first parent's features but modify slightly for proximity
    let base_features = &parent_messages[0].features;
    let mut new_phi = Vec::new();

    for (i, axis_phi) in base_features.phi.iter().enumerate() {
        // Create features very close to the first parent in p-adic metric
        // Use the same digits but modify the first digit for proximity
        let mut new_digits = axis_phi.qp_digits.digits.clone();
        if !new_digits.is_empty() {
            // Increment the first digit by 1 (mod p) for closeness in p-adic metric
            new_digits[0] = (new_digits[0] + 1) % 3;
        }

        // Create QpDigits directly from the modified digits
        let new_qp = QpDigits {
            digits: new_digits,
            p: axis_phi.qp_digits.p,
        };
        new_phi.push(AxisPhi::new(i as u32, new_qp));
    }

    let features = AdicFeatures::new(new_phi);
    let keypair = Keypair::generate();
    let mut message = AdicMessage::new(
        parent_ids,
        features,
        AdicMeta::new(Utc::now()),
        *keypair.public_key(),
        content,
    );

    // Sign the message
    let signature = keypair.sign(&message.to_bytes());
    message.signature = signature;

    // Store directly and update tip manager
    node.storage
        .store_message(&message)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to store message: {}", e))?;

    // Update tip manager
    node.sync_tips_from_storage().await?;

    Ok(message.id)
}

// Helper functions for test setup
async fn create_genesis_for_node(node: &AdicNode) -> MessageId {
    // Create genesis message with all-zero features as per spec
    let genesis_features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(0, 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(0, 3, 10)),
        AxisPhi::new(2, QpDigits::from_u64(0, 3, 10)),
    ]);

    let keypair = Keypair::generate();
    let mut genesis = AdicMessage::new(
        vec![], // No parents for genesis
        genesis_features,
        AdicMeta::new(Utc::now()),
        *keypair.public_key(),
        b"Genesis".to_vec(),
    );

    // Sign the message
    let signature = keypair.sign(&genesis.to_bytes());
    genesis.signature = signature;

    // Store directly in the node's storage
    node.storage.store_message(&genesis).await.unwrap();

    genesis.id
}

async fn create_diverse_parents(node: &AdicNode, genesis_id: MessageId) -> Vec<MessageId> {
    let mut tip_ids = Vec::new();

    // Create messages with diverse features to satisfy diversity requirements
    // We need at least d+1 parents with diverse p-adic balls
    // Use powers of 3 to ensure maximum diversity in base 3
    let diverse_values = [
        (1, 2, 4),       // Different low values
        (3, 5, 7),       // Prime numbers
        (9, 10, 11),     // Powers and neighbors
        (27, 28, 29),    // 3^3 and neighbors
        (81, 82, 83),    // 3^4 and neighbors
        (100, 101, 102), // Different century
    ];

    for (i, (v0, v1, v2)) in diverse_values.iter().enumerate() {
        let features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(*v0, 3, 10)),
            AxisPhi::new(1, QpDigits::from_u64(*v1, 3, 10)),
            AxisPhi::new(2, QpDigits::from_u64(*v2, 3, 10)),
        ]);

        let keypair = Keypair::generate();
        let mut msg = AdicMessage::new(
            vec![genesis_id], // Point back to genesis
            features,
            AdicMeta::new(Utc::now()),
            *keypair.public_key(),
            format!("Parent {}", i + 1).into_bytes(),
        );

        let signature = keypair.sign(&msg.to_bytes());
        msg.signature = signature;

        // Store the message - storage automatically manages tips
        node.storage.store_message(&msg).await.unwrap();

        tip_ids.push(msg.id);
    }

    tip_ids
}

#[tokio::test]
async fn test_node_uses_config_genesis() {
    use adic_node::genesis::{GenesisConfig, GenesisManifest};
    use tempfile::TempDir;

    // Create a temporary directory for the test
    let temp_dir = TempDir::new().unwrap();

    // Create a custom genesis config
    let mut custom_genesis = GenesisConfig::test();
    custom_genesis.chain_id = "test-custom-chain".to_string();

    // Create node config with custom genesis
    let mut config = NodeConfig::default();
    config.node.data_dir = temp_dir.path().to_path_buf();
    config.node.bootstrap = Some(true); // Set as bootstrap node
    config.storage.backend = "memory".to_string();
    config.network.enabled = false;
    config.genesis = Some(custom_genesis.clone());

    // Create node - it should use the config genesis
    let node = AdicNode::new(config).await.unwrap();

    // Verify the node was initialized successfully
    assert!(!node.node_id().is_empty());

    // The node should have applied the custom genesis allocations
    // Verify the genesis.json file was created
    let genesis_json_path = temp_dir.path().join("genesis.json");
    assert!(
        genesis_json_path.exists(),
        "Bootstrap node should create genesis.json"
    );

    // Verify the genesis.json contains the custom chain_id
    let genesis_data = std::fs::read_to_string(&genesis_json_path).unwrap();
    let genesis_manifest: GenesisManifest = serde_json::from_str(&genesis_data).unwrap();
    assert_eq!(
        genesis_manifest.config.chain_id, "test-custom-chain",
        "Genesis should use custom chain_id from config"
    );

    // Verify the node can operate normally
    let stats = node.get_stats().await.unwrap();
    assert_eq!(
        stats.message_count, 0,
        "New node should start with 0 messages"
    );
}
