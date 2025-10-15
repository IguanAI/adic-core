use adic_consensus::{AdmissibilityChecker, ConsensusEngine};
use adic_crypto::Keypair;
use adic_finality::{FinalityConfig, FinalityEngine};
use adic_math::{proximity_score, vp_diff};
use adic_mrw::{MrwSelector, ParentCandidate, SelectionParams, WeightCalculator};
use adic_storage::{store::BackendType, StorageConfig, StorageEngine};
use adic_types::*;
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[tokio::test]
async fn test_full_consensus_flow() {
    // Initialize parameters
    let params = AdicParams::default();

    // Create storage - use in-memory to avoid conflicts with other tests
    let storage_config = StorageConfig {
        backend_type: adic_storage::store::BackendType::Memory,
        ..Default::default()
    };
    let storage = Arc::new(StorageEngine::new(storage_config).unwrap());

    // Create consensus engine
    let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));
    let _checker = AdmissibilityChecker::new(params.clone());

    // Create finality engine
    let finality_config = FinalityConfig::from(&params);
    let finality = Arc::new(
        FinalityEngine::new(finality_config, consensus.clone(), storage.clone()).await
    );

    // Generate keypair
    let keypair = Keypair::generate();

    // Create genesis message
    let genesis = create_test_message(vec![], 0, &keypair);
    storage.store_message(&genesis).await.unwrap();

    // Create chain of messages
    let mut current_tip = genesis.id;
    let mut all_messages = vec![genesis];

    for i in 1..10 {
        let message = create_test_message(vec![current_tip], i, &keypair);

        // Store message
        storage.store_message(&message).await.unwrap();

        // Add to finality engine
        let mut ball_ids = HashMap::new();
        for axis_phi in &message.features.phi {
            ball_ids.insert(axis_phi.axis.0, axis_phi.qp_digits.ball_id(3));
        }

        finality
            .add_message(message.id, vec![current_tip], 1.0, ball_ids)
            .await
            .unwrap();

        current_tip = message.id;
        all_messages.push(message);
    }

    // Check finality
    let _finalized = finality.check_finality().await.unwrap();

    // Verify storage
    let stats = storage.get_stats().await.unwrap();
    assert_eq!(stats.message_count, 10);
    assert_eq!(stats.tip_count, 1);

    // Verify we can retrieve messages
    for msg in &all_messages {
        let retrieved = storage.get_message(&msg.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, msg.id);
    }
}

#[tokio::test]
async fn test_admissibility_constraints() {
    let params = AdicParams::default();
    let checker = AdmissibilityChecker::new(params.clone());
    let keypair = Keypair::generate();

    // Create parent messages with specific features
    let parent_features = vec![
        vec![
            QpDigits::from_u64(10, 3, 10),
            QpDigits::from_u64(20, 3, 10),
            QpDigits::from_u64(30, 3, 10),
        ],
        vec![
            QpDigits::from_u64(11, 3, 10),
            QpDigits::from_u64(21, 3, 10),
            QpDigits::from_u64(31, 3, 10),
        ],
        vec![
            QpDigits::from_u64(12, 3, 10),
            QpDigits::from_u64(22, 3, 10),
            QpDigits::from_u64(32, 3, 10),
        ],
        vec![
            QpDigits::from_u64(13, 3, 10),
            QpDigits::from_u64(23, 3, 10),
            QpDigits::from_u64(33, 3, 10),
        ],
    ];

    let parent_reputations = vec![1.0, 1.5, 2.0, 0.5];

    // Create a message that should pass admissibility
    let message = AdicMessage::new(
        vec![
            MessageId::new(b"p1"),
            MessageId::new(b"p2"),
            MessageId::new(b"p3"),
            MessageId::new(b"p4"),
        ],
        AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(10, 3, 10)),
            AxisPhi::new(1, QpDigits::from_u64(20, 3, 10)),
            AxisPhi::new(2, QpDigits::from_u64(30, 3, 10)),
        ]),
        AdicMeta::new(Utc::now()),
        *keypair.public_key(),
        b"test".to_vec(),
    );

    let result = checker
        .check_message(&message, &parent_features, &parent_reputations)
        .unwrap();

    // Check individual constraints
    println!("Score passed (S(x;A) >= d): {}", result.score_passed);
    println!("C2 (Diversity): {}", result.c2_passed);
    println!("C3 (Reputation): {}", result.c3_passed);
    println!("Overall admissible: {}", result.is_admissible);
}

#[tokio::test]
async fn test_mrw_parent_selection() {
    let params = AdicParams::default();

    // Create MRW selector
    let selection_params = SelectionParams::from_adic_params(&params);
    let selector = MrwSelector::new(selection_params);
    let weight_calc = WeightCalculator::new(params.lambda, params.beta, params.mu);

    // Create candidate tips
    let mut candidates = Vec::new();
    for i in 0..10 {
        candidates.push(ParentCandidate {
            message_id: MessageId::new(format!("tip{}", i).as_bytes()),
            features: vec![
                QpDigits::from_u64(i as u64, 3, 10),
                QpDigits::from_u64((i * 2) as u64, 3, 10),
                QpDigits::from_u64((i * 3) as u64, 3, 10),
            ],
            reputation: 1.0 + (i as f64) * 0.1,
            conflict_penalty: 0.1 * (i as f64),
            weight: 1.0,
            axis_weights: HashMap::new(),
            age: 0.0,
        });
    }

    // Create message features
    let message_features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(5, 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(10, 3, 10)),
        AxisPhi::new(2, QpDigits::from_u64(15, 3, 10)),
    ]);

    // Select parents
    let (selected, trace) = selector
        .select_parents(&message_features, candidates.clone(), &weight_calc)
        .await
        .unwrap();

    // Verify selection
    assert_eq!(selected.len(), (params.d + 1) as usize);

    // Check diversity
    let mut balls_per_axis: HashMap<u32, HashSet<Vec<u8>>> = HashMap::new();
    for parent_id in &selected {
        let parent = candidates
            .iter()
            .find(|c| c.message_id == *parent_id)
            .unwrap();
        for (idx, qp_digits) in parent.features.iter().enumerate() {
            balls_per_axis
                .entry(idx as u32)
                .or_default()
                .insert(qp_digits.ball_id(3));
        }
    }

    // Each axis should have at least q distinct balls
    for (_axis, balls) in balls_per_axis {
        assert!(balls.len() >= params.q as usize);
    }

    println!(
        "Selected {} parents with {} steps",
        selected.len(),
        trace.steps.len()
    );
}

#[test]
fn test_padic_mathematics() {
    // Test vp_diff
    let x = QpDigits::from_u64(27, 3, 10); // 27 = 0*3^0 + 0*3^1 + 0*3^2 + 1*3^3
    let y = QpDigits::from_u64(9, 3, 10); // 9 = 0*3^0 + 0*3^1 + 1*3^2

    let diff = vp_diff(&x, &y);
    assert_eq!(diff, 2); // They differ at position 2

    // Test with same prefix
    let x = QpDigits::from_u64(10, 3, 10); // 10 = 1*3^0 + 0*3^1 + 1*3^2
    let y = QpDigits::from_u64(19, 3, 10); // 19 = 1*3^0 + 0*3^1 + 2*3^2

    let diff = vp_diff(&x, &y);
    assert_eq!(diff, 2); // They differ at position 2

    // Test proximity score
    let score = proximity_score(&x, &y, 3);
    assert!(score > 0.0);

    // Test ball identification
    let ball_id = x.ball_id(2);
    assert_eq!(ball_id.len(), 2);
}

#[tokio::test]
async fn test_finality_kcore() {
    let params = AdicParams::default();

    // Create storage for finality engine
    let storage_config = StorageConfig {
        backend_type: adic_storage::store::BackendType::Memory,
        ..Default::default()
    };
    let storage = Arc::new(StorageEngine::new(storage_config).unwrap());

    let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));
    let finality_config = FinalityConfig::from(&params);
    let finality = Arc::new(
        FinalityEngine::new(finality_config, consensus.clone(), storage).await
    );

    // Create a k-core structure
    // Need at least k nodes with degree >= k
    let k = params.k as usize;

    // Create hub node
    let hub = MessageId::new(b"hub");
    finality
        .add_message(hub, vec![], 2.0, HashMap::new())
        .await
        .unwrap();

    // Create k peripheral nodes all connecting to hub and each other
    let mut peripherals = Vec::new();
    for i in 0..k {
        let node = MessageId::new(format!("node{}", i).as_bytes());

        // Connect to hub and previous nodes to build k-core
        let mut parents = vec![hub];
        for node in peripherals.iter().take(i.min(k - 1)) {
            parents.push(*node);
        }

        finality
            .add_message(node, parents, 1.5, HashMap::new())
            .await
            .unwrap();

        peripherals.push(node);
    }

    // Check finality
    let _finalized = finality.check_finality().await.unwrap();

    // Get stats
    let stats = finality.get_stats().await;
    println!("Finality stats: {:?}", stats);

    assert!(stats.pending_count > 0);
}

// Helper function to create test messages
fn create_test_message(parents: Vec<MessageId>, index: u64, keypair: &Keypair) -> AdicMessage {
    AdicMessage::new(
        parents,
        AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(index, 3, 10)),
            AxisPhi::new(1, QpDigits::from_u64(index * 2, 3, 10)),
            AxisPhi::new(2, QpDigits::from_u64(index * 3, 3, 10)),
        ]),
        AdicMeta::new(Utc::now()),
        *keypair.public_key(),
        format!("Message {}", index).into_bytes(),
    )
}

#[tokio::test]
async fn test_wallet_registration_flow() {
    use adic_crypto::Keypair;
    use adic_economics::AccountAddress;
    use adic_node::wallet_registry::{WalletMetadata, WalletRegistrationRequest, WalletRegistry};

    // Create storage and registry
    let storage = Arc::new(
        StorageEngine::new(StorageConfig {
            backend_type: BackendType::Memory,
            ..Default::default()
        })
        .unwrap(),
    );
    let registry = WalletRegistry::new(storage);

    // Generate test keypair
    let keypair = Keypair::generate();
    let public_key = keypair.public_key();
    let address = AccountAddress::from_public_key(public_key);

    // Create registration message
    let registration_message = format!(
        "ADIC Wallet Registration\nAddress: {}\nPublic Key: {}\nTimestamp: {}",
        hex::encode(address.as_bytes()),
        hex::encode(public_key.as_bytes()),
        chrono::Utc::now().timestamp()
    );

    // Sign the message
    let signature = keypair.sign(registration_message.as_bytes());

    // Create registration request
    let request = WalletRegistrationRequest {
        address: hex::encode(address.as_bytes()),
        public_key: hex::encode(public_key.as_bytes()),
        signature: hex::encode(signature.as_bytes()),
        metadata: Some(WalletMetadata {
            label: Some("Test Wallet".to_string()),
            wallet_type: Some("hardware".to_string()),
            trusted: true,
        }),
    };

    // Register the wallet
    registry.register_wallet(request.clone()).await.unwrap();

    // Verify registration
    assert!(registry.is_registered(&address).await);
    assert_eq!(registry.get_public_key(&address).await, Some(*public_key));

    // Get wallet info
    let info = registry.get_wallet_info(&address).await.unwrap();
    assert_eq!(info.address, address);
    assert_eq!(info.public_key, *public_key);
    assert_eq!(info.metadata.label, Some("Test Wallet".to_string()));
    assert_eq!(info.metadata.wallet_type, Some("hardware".to_string()));
    assert!(info.metadata.trusted);

    // Mark wallet as used
    registry.mark_used(&address).await.unwrap();

    // Get updated info
    let updated_info = registry.get_wallet_info(&address).await.unwrap();
    assert!(updated_info.last_used.is_some());

    // Test re-registration (update)
    let updated_request = WalletRegistrationRequest {
        address: hex::encode(address.as_bytes()),
        public_key: hex::encode(public_key.as_bytes()),
        signature: hex::encode(signature.as_bytes()),
        metadata: Some(WalletMetadata {
            label: Some("Updated Wallet".to_string()),
            wallet_type: Some("software".to_string()),
            trusted: false,
        }),
    };

    registry.register_wallet(updated_request).await.unwrap();

    // Verify update
    let updated = registry.get_wallet_info(&address).await.unwrap();
    assert_eq!(updated.metadata.label, Some("Updated Wallet".to_string()));
    assert_eq!(updated.metadata.wallet_type, Some("software".to_string()));
    assert!(!updated.metadata.trusted);

    // Unregister wallet
    registry.unregister_wallet(&address).await.unwrap();

    // Verify unregistration
    assert!(!registry.is_registered(&address).await);
    assert!(registry.get_wallet_info(&address).await.is_none());
}

#[tokio::test]
async fn test_wallet_registry_stats() {
    use adic_crypto::Keypair;
    use adic_economics::AccountAddress;
    use adic_node::wallet_registry::{WalletMetadata, WalletRegistrationRequest, WalletRegistry};

    let storage = Arc::new(
        StorageEngine::new(StorageConfig {
            backend_type: BackendType::Memory,
            ..Default::default()
        })
        .unwrap(),
    );
    let registry = WalletRegistry::new(storage);

    // Register multiple wallets of different types
    let wallet_configs = [
        ("hardware", true),
        ("hardware", false),
        ("software", true),
        ("software", false),
        ("multisig", true),
    ];

    for (i, (wallet_type, trusted)) in wallet_configs.iter().enumerate() {
        let keypair = Keypair::generate();
        let public_key = keypair.public_key();
        let address = AccountAddress::from_public_key(public_key);

        let registration_message = format!(
            "ADIC Wallet Registration\nAddress: {}\nPublic Key: {}\nTimestamp: {}",
            hex::encode(address.as_bytes()),
            hex::encode(public_key.as_bytes()),
            chrono::Utc::now().timestamp()
        );

        let signature = keypair.sign(registration_message.as_bytes());

        let request = WalletRegistrationRequest {
            address: hex::encode(address.as_bytes()),
            public_key: hex::encode(public_key.as_bytes()),
            signature: hex::encode(signature.as_bytes()),
            metadata: Some(WalletMetadata {
                label: Some(format!("Wallet {}", i)),
                wallet_type: Some(wallet_type.to_string()),
                trusted: *trusted,
            }),
        };

        registry.register_wallet(request).await.unwrap();

        // Mark some as recently used
        if i < 2 {
            registry.mark_used(&address).await.unwrap();
        }
    }

    // Get stats
    let stats = registry.get_stats().await;

    assert_eq!(stats.total_wallets, 5);
    assert_eq!(stats.trusted_wallets, 3);
    assert!(stats.last_registration.is_some());

    // Check wallet type counts
    assert_eq!(*stats.wallet_types.get("hardware").unwrap_or(&0), 2);
    assert_eq!(*stats.wallet_types.get("software").unwrap_or(&0), 2);
    assert_eq!(*stats.wallet_types.get("multisig").unwrap_or(&0), 1);
}

#[tokio::test]
async fn test_wallet_list_operations() {
    use adic_crypto::Keypair;
    use adic_economics::AccountAddress;
    use adic_node::wallet_registry::{WalletRegistrationRequest, WalletRegistry};

    let storage = Arc::new(
        StorageEngine::new(StorageConfig {
            backend_type: BackendType::Memory,
            ..Default::default()
        })
        .unwrap(),
    );
    let registry = WalletRegistry::new(storage);

    let mut addresses = Vec::new();

    // Register 3 wallets
    for _i in 0..3 {
        let keypair = Keypair::generate();
        let public_key = keypair.public_key();
        let address = AccountAddress::from_public_key(public_key);
        addresses.push(address);

        let registration_message = format!(
            "ADIC Wallet Registration\nAddress: {}\nPublic Key: {}\nTimestamp: {}",
            hex::encode(address.as_bytes()),
            hex::encode(public_key.as_bytes()),
            chrono::Utc::now().timestamp()
        );

        let signature = keypair.sign(registration_message.as_bytes());

        let request = WalletRegistrationRequest {
            address: hex::encode(address.as_bytes()),
            public_key: hex::encode(public_key.as_bytes()),
            signature: hex::encode(signature.as_bytes()),
            metadata: None,
        };

        registry.register_wallet(request).await.unwrap();

        // Small delay to ensure different timestamps
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    // List all wallets
    let wallets = registry.list_wallets().await;
    assert_eq!(wallets.len(), 3);

    // Verify all addresses are present
    let listed_addresses: Vec<_> = wallets.iter().map(|w| w.address).collect();
    for addr in &addresses {
        assert!(listed_addresses.contains(addr));
    }

    // No ordering guaranteed for list_wallets (uses HashMap internally)
}

#[tokio::test]
async fn test_wallet_json_export_import() {
    use adic_node::wallet::NodeWallet;
    use tempfile::TempDir;

    // Create test wallet
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path();
    let node_id = "test_node";

    let wallet = NodeWallet::load_or_create(data_dir, node_id).unwrap();
    let original_address = wallet.address();
    let original_pubkey = wallet.public_key();

    // Export to JSON
    let password = "test_password_123";
    let json_str = wallet.export_to_json(password).unwrap();

    // Verify JSON structure
    let json_value: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(json_value["version"], 4);
    assert!(json_value["encrypted_private_key"].is_string());
    assert!(json_value["salt"].is_string());
    assert!(json_value["nonce"].is_string()); // v4 uses nonce for XSalsa20-Poly1305
    // node_id is intentionally not exported for security/portability
    assert!(json_value["node_id"].is_null());

    // Import from JSON
    let imported_wallet =
        NodeWallet::import_from_json(&json_str, password, "imported_node").unwrap();

    // Verify imported wallet matches original
    assert_eq!(imported_wallet.address(), original_address);
    assert_eq!(imported_wallet.public_key(), original_pubkey);

    // Test wrong password
    let wrong_password = "wrong_password";
    let result = NodeWallet::import_from_json(&json_str, wrong_password, "imported_node");
    assert!(result.is_err());
}
