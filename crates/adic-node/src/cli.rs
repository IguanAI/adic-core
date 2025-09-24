use crate::genesis::{account_address_from_hex, GenesisConfig};
use crate::wallet::NodeWallet;
use adic_consensus::ConsensusEngine;
use adic_economics::{AdicAmount, BalanceManager};
use adic_finality::{FinalityConfig, FinalityEngine};
use adic_mrw::MrwEngine;
use adic_storage::{StorageConfig, StorageEngine, TipManager};
use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AdicParams, AxisPhi, QpDigits, DEFAULT_PRECISION,
};
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

/// Run a local test with the specified number of messages
pub async fn run_local_test(count: usize) -> Result<()> {
    info!("Running local test with {} messages", count);

    // Create default parameters
    let params = AdicParams::default();

    // Create storage engine
    let storage_config = StorageConfig::default();
    let storage = Arc::new(StorageEngine::new(storage_config)?);

    // Create consensus engine
    let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));

    // Create MRW engine
    let mrw = MrwEngine::new(params.clone());

    // Create finality engine
    let finality_config = FinalityConfig::from(&params);
    let finality = FinalityEngine::new(finality_config, consensus.clone(), storage.clone());

    // Create tip manager
    let tip_manager = TipManager::new();

    // Create or load test wallet
    let wallet_path = std::env::temp_dir().join("adic-test-wallet");
    let test_wallet = NodeWallet::load_or_create(&wallet_path, "test-node")?;
    let keypair = test_wallet.keypair();
    let address = test_wallet.address();
    info!("Test wallet address: {}", hex::encode(address.as_bytes()));

    // Initialize balance manager and apply genesis if first run
    use adic_economics::storage::MemoryStorage;
    let economics_storage = Arc::new(MemoryStorage::new());
    let balance_manager = BalanceManager::new(economics_storage);

    // Check if genesis has been applied
    let genesis_applied = balance_manager.get_balance(address).await? > AdicAmount::ZERO;

    if !genesis_applied {
        info!("Applying genesis allocations for test...");
        let genesis_config = GenesisConfig::test();

        for (address_hex, amount_adic) in &genesis_config.allocations {
            let alloc_address = account_address_from_hex(address_hex)
                .map_err(|e| anyhow::anyhow!("Invalid address: {}", e))?;
            let amount = AdicAmount::from_adic(*amount_adic as f64);
            balance_manager.credit(alloc_address, amount).await?;
            info!("Allocated {} ADIC to {}", amount_adic, &address_hex[..16]);
        }

        // Also give test wallet some ADIC
        let test_amount = AdicAmount::from_adic(1000.0);
        balance_manager.credit(address, test_amount).await?;
        info!("Allocated 1000 ADIC to test wallet");
    }

    // Check balance
    let balance = balance_manager.get_balance(address).await?;
    info!("Test wallet balance: {} ADIC", balance.to_adic());

    let deposit_amount = AdicAmount::from_adic(GenesisConfig::default().deposit_amount);
    if balance < deposit_amount {
        return Err(anyhow::anyhow!(
            "Insufficient balance for deposits. Need at least {} ADIC",
            deposit_amount.to_adic()
        ));
    }

    // Create genesis message
    let genesis_features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(0, params.p, DEFAULT_PRECISION)),
        AxisPhi::new(1, QpDigits::from_u64(0, params.p, DEFAULT_PRECISION)),
        AxisPhi::new(2, QpDigits::from_u64(0, params.p, DEFAULT_PRECISION)),
    ]);

    let mut genesis = AdicMessage::new(
        vec![],
        genesis_features,
        AdicMeta::new(Utc::now()),
        *keypair.public_key(),
        b"Genesis".to_vec(),
    );

    // Sign genesis message
    let signature = keypair.sign(&genesis.to_bytes());
    genesis.signature = signature;

    // Escrow deposit for genesis
    balance_manager.debit(address, deposit_amount).await?;
    consensus
        .deposits
        .escrow(genesis.id, *keypair.public_key())
        .await?;

    // Validate genesis
    let validation_result = consensus.validate_and_slash(&genesis).await?;
    if !validation_result.is_valid {
        return Err(anyhow::anyhow!(
            "Genesis failed validation: {:?}",
            validation_result.errors
        ));
    }

    storage.store_message(&genesis).await?;
    tip_manager.add_tip(genesis.id, 1.0).await;
    info!(
        "Genesis message created: {}",
        hex::encode(genesis.id.as_bytes())
    );
    let mut messages_created = 1;

    // Create additional messages
    for i in 0..count.saturating_sub(1) {
        // Create features with slight variation
        let features = AdicFeatures::new(vec![
            AxisPhi::new(
                0,
                QpDigits::from_u64((i + 1) as u64, params.p, DEFAULT_PRECISION),
            ),
            AxisPhi::new(
                1,
                QpDigits::from_u64((i % 3) as u64, params.p, DEFAULT_PRECISION),
            ),
            AxisPhi::new(
                2,
                QpDigits::from_u64((i % 5) as u64, params.p, DEFAULT_PRECISION),
            ),
        ]);

        // Get current tips
        let tips = tip_manager.get_tips().await;

        // Select parents using MRW (if enough tips available)
        let parents = if tips.len() >= (params.d + 1) as usize {
            mrw.select_parents(
                &features,
                &tips,
                &storage,
                consensus.conflicts(),
                &consensus.reputation,
            )
            .await?
        } else {
            // Bootstrap phase - use all available tips
            tips
        };

        // Create message
        let mut message = AdicMessage::new(
            parents.clone(),
            features,
            AdicMeta::new(Utc::now()),
            *keypair.public_key(),
            format!("Message {}", i + 1).into_bytes(),
        );

        // Sign the message
        let signature = keypair.sign(&message.to_bytes());
        message.signature = signature;

        // Check balance for deposit
        let current_balance = balance_manager.get_balance(address).await?;
        if current_balance < deposit_amount {
            info!("Insufficient balance for message {}. Stopping test.", i + 1);
            break;
        }

        // Escrow deposit
        balance_manager.debit(address, deposit_amount).await?;
        consensus
            .deposits
            .escrow(message.id, *keypair.public_key())
            .await?;

        // Validate and potentially slash
        let validation_result = consensus.validate_and_slash(&message).await?;
        let is_valid = validation_result.is_valid;

        if is_valid {
            // Refund deposit on valid message
            balance_manager.credit(address, deposit_amount).await?;

            // Store message
            storage.store_message(&message).await?;

            // Add to tip manager
            tip_manager.add_tip(message.id, 1.0).await;

            // Remove selected parents from tips
            for parent_id in &parents {
                tip_manager.remove_tip(parent_id).await;
            }

            // Add to finality engine
            let mut ball_ids = HashMap::new();
            for axis_phi in &message.features.phi {
                ball_ids.insert(axis_phi.axis.0, axis_phi.qp_digits.ball_id(3));
            }

            finality
                .add_message(message.id, parents, 1.0, ball_ids)
                .await?;

            messages_created += 1;
            info!(
                "Created message {}: {}",
                i + 1,
                hex::encode(message.id.as_bytes())
            );
        } else {
            info!(
                "Message {} failed validation and was slashed: {:?}",
                i + 1,
                validation_result.errors
            );
        }

        // Periodically check finality
        if (i + 1) % 10 == 0 {
            let finalized = finality.check_finality().await?;
            if !finalized.is_empty() {
                info!("Finalized {} messages", finalized.len());
            }
        }
    }

    // Final finality check
    let finalized = finality.check_finality().await?;
    info!(
        "Final finality check: {} messages finalized",
        finalized.len()
    );

    // Get statistics
    let stats = storage.get_stats().await?;
    info!("Test completed:");
    info!("  Messages created: {}", messages_created);
    info!("  Total messages: {}", stats.message_count);
    info!("  Current tips: {}", stats.tip_count);
    info!("  Finalized: {}", stats.finalized_count);

    Ok(())
}
