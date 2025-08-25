use adic_node::{NodeConfig, AdicNode};
use adic_crypto::Keypair;
use adic_storage::{StorageEngine, StorageConfig};
use adic_types::*;
use chrono::Utc;

#[tokio::test]
async fn test_node_initialization() {
    let config = NodeConfig::default();
    let node = AdicNode::new(config).await.unwrap();
    
    // Verify node ID is generated
    let node_id = node.node_id();
    assert!(!node_id.is_empty());
    assert_eq!(node_id.len(), 16); // 8 bytes in hex
}

#[tokio::test]
async fn test_message_submission() {
    let config = NodeConfig::default();
    let node = AdicNode::new(config).await.unwrap();
    
    // Submit a test message
    let content = b"Hello, ADIC!".to_vec();
    let message_id = node.submit_message(content.clone()).await.unwrap();
    
    // Verify message was stored
    let retrieved = node.get_message(&message_id).await.unwrap();
    assert!(retrieved.is_some());
    
    let msg = retrieved.unwrap();
    assert_eq!(msg.id, message_id);
    assert_eq!(msg.payload, content);
}

#[tokio::test]
async fn test_node_stats() {
    let config = NodeConfig::default();
    let node = AdicNode::new(config).await.unwrap();
    
    // Get initial stats
    let stats = node.get_stats().await.unwrap();
    assert_eq!(stats.message_count, 0);
    assert_eq!(stats.tip_count, 0);
    
    // Submit a message
    node.submit_message(b"test".to_vec()).await.unwrap();
    
    // Check stats updated
    let stats = node.get_stats().await.unwrap();
    assert_eq!(stats.message_count, 1);
    assert_eq!(stats.tip_count, 1);
}

#[tokio::test]
async fn test_storage_operations() {
    let config = StorageConfig::default();
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
}

#[test]
fn test_keypair_generation() {
    let keypair1 = Keypair::generate();
    let keypair2 = Keypair::generate();
    
    // Verify different keys are generated
    assert_ne!(keypair1.public_key().as_bytes(), keypair2.public_key().as_bytes());
    
    // Test serialization
    let bytes = keypair1.to_bytes();
    assert_eq!(bytes.len(), 32); // Ed25519 private key
    
    // Test deserialization
    let restored = Keypair::from_bytes(&bytes).unwrap();
    assert_eq!(restored.public_key().as_bytes(), keypair1.public_key().as_bytes());
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