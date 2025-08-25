use std::time::Duration;
use tokio::time::sleep;
use adic_node::config::NodeConfig;
use adic_types::{AdicMessage, MessageId, Features, QpDigits};
use adic_consensus::ConsensusEngine;
use adic_storage::memory::MemoryBackend;
use adic_crypto::Keypair;

#[tokio::test]
async fn test_single_node_bootstrap() {
    // Initialize node components
    let storage = MemoryBackend::new();
    let config = NodeConfig::default();
    let keypair = Keypair::generate();
    
    // Create first message
    let msg = create_test_message(0, vec![], &keypair).await;
    
    // Store and verify
    storage.put_message(&msg).await.unwrap();
    let retrieved = storage.get_message(&msg.id).await.unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().id, msg.id);
}

#[tokio::test]
async fn test_dag_construction() {
    let storage = MemoryBackend::new();
    let keypair = Keypair::generate();
    let mut message_ids = vec![];
    
    // Create initial messages
    for i in 0..3 {
        let msg = create_test_message(i, vec![], &keypair).await;
        message_ids.push(msg.id.clone());
        storage.put_message(&msg).await.unwrap();
    }
    
    // Create messages with parents
    for i in 3..10 {
        let parents = vec![
            message_ids[i - 3].clone(),
            message_ids[i - 2].clone(),
        ];
        let msg = create_test_message(i as u32, parents.clone(), &keypair).await;
        
        // Verify parent relationships
        assert_eq!(msg.parents.len(), 2);
        assert!(msg.parents.contains(&message_ids[i - 3]));
        
        message_ids.push(msg.id.clone());
        storage.put_message(&msg).await.unwrap();
    }
    
    // Verify DAG structure
    let tips = storage.get_tips().await.unwrap();
    assert!(!tips.is_empty());
}

#[tokio::test]
async fn test_consensus_flow() {
    let config = adic_consensus::ConsensusConfig {
        k: 3,
        c: 0.5,
        alpha: 2.0,
        tau: 0.1,
        max_parents: 8,
        p: 3,
        delta: 0.05,
    };
    
    let mut engine = ConsensusEngine::new(config);
    let keypair = Keypair::generate();
    let mut all_messages = vec![];
    
    // Bootstrap phase
    for i in 0..5 {
        let msg = create_test_message(i, vec![], &keypair).await;
        let result = engine.process_message(msg.clone());
        assert!(result.is_ok());
        all_messages.push(msg);
    }
    
    // Normal operation with parent selection
    for i in 5..20 {
        let parents = if i < 8 {
            all_messages[i - 3..i].iter()
                .map(|m| m.id.clone())
                .collect()
        } else {
            all_messages[i - 5..i - 2].iter()
                .map(|m| m.id.clone())
                .collect()
        };
        
        let msg = create_test_message(i as u32, parents, &keypair).await;
        let result = engine.process_message(msg.clone());
        assert!(result.is_ok());
        all_messages.push(msg);
    }
    
    // Check finality
    let finalized = engine.compute_finality();
    assert!(!finalized.is_empty(), "Some messages should be finalized");
}

#[tokio::test]
async fn test_concurrent_message_processing() {
    let storage = MemoryBackend::new();
    let keypair = Keypair::generate();
    
    // Spawn multiple tasks creating messages
    let mut handles = vec![];
    
    for author_id in 0..3 {
        let storage_clone = storage.clone();
        let keypair_clone = keypair.clone();
        
        let handle = tokio::spawn(async move {
            for i in 0..10 {
                let msg = create_test_message_with_author(
                    i + (author_id * 10),
                    vec![],
                    &keypair_clone,
                    author_id,
                ).await;
                
                storage_clone.put_message(&msg).await.unwrap();
                sleep(Duration::from_millis(10)).await;
            }
        });
        
        handles.push(handle);
    }
    
    // Wait for all tasks
    for handle in handles {
        handle.await.unwrap();
    }
    
    // Verify all messages were stored
    let all_messages = storage.get_messages_range(0, 100).await.unwrap();
    assert_eq!(all_messages.len(), 30);
}

#[tokio::test]
async fn test_finality_progression() {
    let config = adic_consensus::ConsensusConfig {
        k: 3,
        c: 0.5,
        alpha: 2.0,
        tau: 0.1,
        max_parents: 8,
        p: 3,
        delta: 0.05,
    };
    
    let mut engine = ConsensusEngine::new(config);
    let keypair = Keypair::generate();
    let mut finality_counts = vec![];
    
    // Add messages in batches and track finality
    for batch in 0..5 {
        for i in 0..10 {
            let msg_num = batch * 10 + i;
            let parents = if msg_num < 3 {
                vec![]
            } else {
                vec![] // Simplified for test
            };
            
            let msg = create_test_message(msg_num as u32, parents, &keypair).await;
            engine.process_message(msg).unwrap();
        }
        
        let finalized = engine.compute_finality();
        finality_counts.push(finalized.len());
    }
    
    // Verify finality increases over time
    for i in 1..finality_counts.len() {
        assert!(
            finality_counts[i] >= finality_counts[i - 1],
            "Finality count should not decrease"
        );
    }
}

#[tokio::test]
async fn test_conflict_resolution() {
    let storage = MemoryBackend::new();
    let keypair = Keypair::generate();
    
    // Create conflicting messages (same content, different authors)
    let msg1 = create_test_message_with_author(1, vec![], &keypair, 0).await;
    let msg2 = create_test_message_with_author(1, vec![], &keypair, 1).await;
    
    storage.put_message(&msg1).await.unwrap();
    storage.put_message(&msg2).await.unwrap();
    
    // Both should be stored (conflict resolution happens at consensus layer)
    assert!(storage.get_message(&msg1.id).await.unwrap().is_some());
    assert!(storage.get_message(&msg2.id).await.unwrap().is_some());
    
    // Verify they are tracked as potential conflicts
    let tips = storage.get_tips().await.unwrap();
    assert_eq!(tips.len(), 2);
}

#[tokio::test]
async fn test_storage_persistence() {
    let storage = MemoryBackend::new();
    let keypair = Keypair::generate();
    
    // Store messages
    let mut message_ids = vec![];
    for i in 0..10 {
        let msg = create_test_message(i, vec![], &keypair).await;
        message_ids.push(msg.id.clone());
        storage.put_message(&msg).await.unwrap();
    }
    
    // Create and verify snapshot
    let snapshot = storage.create_snapshot().await.unwrap();
    assert!(snapshot.message_count >= 10);
    
    // Verify retrieval
    for id in &message_ids {
        assert!(storage.get_message(id).await.unwrap().is_some());
    }
}

#[tokio::test]
async fn test_padic_distance_computation() {
    use adic_math::PadicOperations;
    
    let x = QpDigits::from_u64(3, 100);
    let y = QpDigits::from_u64(3, 200);
    
    let distance = PadicOperations::distance(&x, &y);
    assert!(distance > 0.0);
    
    // Test ultrametric property
    let z = QpDigits::from_u64(3, 150);
    let d_xy = PadicOperations::distance(&x, &y);
    let d_xz = PadicOperations::distance(&x, &z);
    let d_yz = PadicOperations::distance(&y, &z);
    
    // Strong triangle inequality
    assert!(d_xy <= d_xz.max(d_yz));
}

// Helper functions
async fn create_test_message(
    index: u32,
    parents: Vec<MessageId>,
    keypair: &Keypair,
) -> AdicMessage {
    create_test_message_with_author(index, parents, keypair, 0).await
}

async fn create_test_message_with_author(
    index: u32,
    parents: Vec<MessageId>,
    keypair: &Keypair,
    author_id: u32,
) -> AdicMessage {
    let id = MessageId::new(&format!("msg_{}_{}", author_id, index).into_bytes());
    let features = Features {
        time: QpDigits::from_u64(3, index as u64),
        topic: QpDigits::from_u64(3, (index % 10) as u64),
        region: QpDigits::from_u64(3, (index % 5) as u64),
    };
    
    let payload = format!("data_{}", index).into_bytes();
    let author = format!("author_{}", author_id).into_bytes();
    let signature = keypair.sign(&payload);
    
    AdicMessage {
        id,
        parents,
        features,
        author,
        signature,
        payload,
        timestamp: index as u64,
        deposit: 100,
    }
}