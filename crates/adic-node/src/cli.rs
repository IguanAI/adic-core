use adic_consensus::ConsensusEngine;
use adic_crypto::Keypair;
use adic_finality::{FinalityEngine, FinalityConfig};
use adic_mrw::MrwEngine;
use adic_storage::{StorageEngine, StorageConfig, TipManager};
use adic_types::{AdicMessage, AdicParams, AdicFeatures, AdicMeta, AxisPhi, QpDigits, DEFAULT_PRECISION, DEFAULT_P};
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
    let storage = StorageEngine::new(storage_config)?;
    
    // Create consensus engine
    let consensus = Arc::new(ConsensusEngine::new(params.clone()));
    
    // Create MRW engine
    let mrw = MrwEngine::new(params.clone());
    
    // Create finality engine
    let finality_config = FinalityConfig::from(&params);
    let finality = FinalityEngine::new(finality_config, consensus.clone());
    
    // Create tip manager
    let tip_manager = TipManager::new();
    
    // Generate a keypair
    let keypair = Keypair::generate();
    info!("Test keypair: {}", hex::encode(keypair.public_key().as_bytes()));
    
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
    consensus.deposits.escrow(genesis.id, *keypair.public_key()).await?;
    
    // Validate genesis
    let validation_result = consensus.validate_and_slash(&genesis).await?;
    if !validation_result.is_valid {
        return Err(anyhow::anyhow!("Genesis failed validation: {:?}", validation_result.errors));
    }
    
    storage.store_message(&genesis).await?;
    tip_manager.add_tip(genesis.id, 1.0).await;
    info!("Genesis message created: {}", hex::encode(genesis.id.as_bytes()));
    let mut messages_created = 1;
    
    // Create additional messages
    for i in 0..count.saturating_sub(1) {
        // Create features with slight variation
        let features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64((i + 1) as u64, params.p, DEFAULT_PRECISION)),
            AxisPhi::new(1, QpDigits::from_u64((i % 3) as u64, params.p, DEFAULT_PRECISION)),
            AxisPhi::new(2, QpDigits::from_u64((i % 5) as u64, params.p, DEFAULT_PRECISION)),
        ]);
        
        // Get current tips
        let tips = tip_manager.get_tips().await;
        
        // Select parents using MRW (if enough tips available)
        let parents = if tips.len() >= (params.d + 1) as usize {
            mrw.select_parents(&features, &tips, &storage, consensus.conflicts()).await?
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
        
        // Escrow deposit
        consensus.deposits.escrow(message.id, *keypair.public_key()).await?;
        
        // Validate and potentially slash
        let validation_result = consensus.validate_and_slash(&message).await?;
        let is_valid = validation_result.is_valid;
        
        if is_valid {
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
            
            finality.add_message(
                message.id,
                parents,
                1.0,
                ball_ids,
            ).await?;
            
            messages_created += 1;
            info!("Created message {}: {}", i + 1, hex::encode(message.id.as_bytes()));
        } else {
            info!("Message {} failed validation and was slashed: {:?}", i + 1, validation_result.errors);
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
    info!("Final finality check: {} messages finalized", finalized.len());
    
    // Get statistics
    let stats = storage.get_stats().await?;
    info!("Test completed:");
    info!("  Messages created: {}", messages_created);
    info!("  Total messages: {}", stats.message_count);
    info!("  Current tips: {}", stats.tip_count);
    info!("  Finalized: {}", stats.finalized_count);
    
    Ok(())
}