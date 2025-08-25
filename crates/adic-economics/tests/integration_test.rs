use adic_economics::{
    EconomicsEngine, AdicAmount, AccountAddress,
    EmissionSchedule, storage::MemoryStorage,
};
use std::sync::Arc;

#[tokio::test]
async fn test_complete_tokenomics_lifecycle() {
    // Initialize economics engine
    let storage = Arc::new(MemoryStorage::new());
    let economics = EconomicsEngine::new(storage).await.unwrap();
    
    // 1. Test Genesis Allocation
    println!("\n=== Testing Genesis Allocation ===");
    
    // Check initial state
    assert_eq!(economics.get_total_supply().await, AdicAmount::ZERO);
    assert!(!economics.genesis.is_allocated().await);
    
    // Initialize genesis
    economics.initialize_genesis().await.unwrap();
    // Note: We check supply instead since the genesis allocator is created internally
    assert_eq!(economics.get_total_supply().await, AdicAmount::GENESIS_SUPPLY);
    
    // Verify total supply matches genesis
    assert_eq!(economics.get_total_supply().await, AdicAmount::GENESIS_SUPPLY);
    
    // Verify allocations (20% Treasury, 30% Liquidity, 50% Genesis)
    let treasury_balance = economics.balances.get_balance(AccountAddress::treasury()).await.unwrap();
    let expected_treasury = AdicAmount::from_adic(300_000_000.0 * 0.20);
    assert_eq!(treasury_balance, expected_treasury);
    println!("Treasury balance: {}", treasury_balance);
    
    let liquidity_balance = economics.balances.get_balance(AccountAddress::liquidity_pool()).await.unwrap();
    let expected_liquidity = AdicAmount::from_adic(300_000_000.0 * 0.30);
    assert_eq!(liquidity_balance, expected_liquidity);
    println!("Liquidity balance: {}", liquidity_balance);
    
    let genesis_balance = economics.balances.get_balance(AccountAddress::genesis_pool()).await.unwrap();
    let expected_genesis = AdicAmount::from_adic(300_000_000.0 * 0.50);
    assert_eq!(genesis_balance, expected_genesis);
    println!("Genesis pool balance: {}", genesis_balance);
    
    // 2. Test Emission
    println!("\n=== Testing Emission System ===");
    
    let _validator = AccountAddress::from_bytes([1; 32]);
    
    // Get current emission rate (should be 1% initially)
    let emission_rate = economics.emission.get_current_emission_rate().await;
    assert!((emission_rate - 0.01).abs() < 0.0001);
    println!("Initial emission rate: {:.4}%/year", emission_rate * 100.0);
    
    // Project emissions
    let emission_1yr = economics.emission.get_projected_emission(1.0).await;
    let emission_5yr = economics.emission.get_projected_emission(5.0).await;
    let emission_10yr = economics.emission.get_projected_emission(10.0).await;
    
    println!("Projected 1-year emission: {}", emission_1yr);
    println!("Projected 5-year emission: {}", emission_5yr);
    println!("Projected 10-year emission: {}", emission_10yr);
    
    // Verify emission decay (5-year should be less than 5x 1-year due to decay)
    assert!(emission_5yr.to_adic() < emission_1yr.to_adic() * 5.0);
    
    // 3. Test Treasury Operations
    println!("\n=== Testing Treasury Multisig ===");
    
    // Use the signers from genesis config (already initialized)
    let signer1 = AccountAddress::from_bytes([0xAA; 32]);
    let signer2 = AccountAddress::from_bytes([0xBB; 32]);
    let _signer3 = AccountAddress::from_bytes([0xCC; 32]);
    
    // Create a treasury proposal
    let recipient = AccountAddress::from_bytes([20; 32]);
    let proposal_amount = AdicAmount::from_adic(1000.0);
    
    let proposal_id = economics.treasury.propose_transfer(
        recipient,
        proposal_amount,
        "Development grant".to_string(),
        signer1,
    ).await.unwrap();
    
    println!("Created proposal: {}", hex::encode(&proposal_id[..8]));
    
    // First approval (proposer already approved)
    // Second approval should execute
    let executed = economics.treasury.approve_proposal(proposal_id, signer2).await.unwrap();
    assert!(executed);
    println!("Proposal executed after reaching threshold");
    
    // Verify recipient received funds
    let recipient_balance = economics.balances.get_balance(recipient).await.unwrap();
    assert_eq!(recipient_balance, proposal_amount);
    println!("Recipient received: {}", recipient_balance);
    
    // 4. Test Balance Operations
    println!("\n=== Testing Balance Operations ===");
    
    let user1 = AccountAddress::from_bytes([30; 32]);
    let user2 = AccountAddress::from_bytes([31; 32]);
    
    // Transfer from genesis pool to user1 (genesis tokens already allocated to pools)
    economics.balances.transfer(
        AccountAddress::genesis_pool(),
        user1,
        AdicAmount::from_adic(1000.0)
    ).await.unwrap();
    let user1_balance = economics.balances.get_balance(user1).await.unwrap();
    assert_eq!(user1_balance, AdicAmount::from_adic(1000.0));
    println!("User1 balance after genesis release: {}", user1_balance);
    
    // Transfer between users
    economics.balances.transfer(user1, user2, AdicAmount::from_adic(250.0)).await.unwrap();
    
    let user1_after = economics.balances.get_balance(user1).await.unwrap();
    let user2_after = economics.balances.get_balance(user2).await.unwrap();
    
    assert_eq!(user1_after, AdicAmount::from_adic(750.0));
    assert_eq!(user2_after, AdicAmount::from_adic(250.0));
    println!("After transfer - User1: {}, User2: {}", user1_after, user2_after);
    
    // 5. Test Deposit Integration
    println!("\n=== Testing Deposit Integration ===");
    
    // Lock funds for deposit
    let deposit_amount = AdicAmount::from_adic(10.0);
    economics.balances.lock(user1, deposit_amount).await.unwrap();
    
    let locked = economics.balances.get_locked_balance(user1).await.unwrap();
    let unlocked = economics.balances.get_unlocked_balance(user1).await.unwrap();
    
    assert_eq!(locked, deposit_amount);
    assert_eq!(unlocked, AdicAmount::from_adic(740.0));
    println!("User1 locked: {}, unlocked: {}", locked, unlocked);
    
    // Simulate deposit refund
    economics.balances.unlock(user1, deposit_amount).await.unwrap();
    let unlocked_after = economics.balances.get_unlocked_balance(user1).await.unwrap();
    assert_eq!(unlocked_after, AdicAmount::from_adic(750.0));
    println!("User1 unlocked balance after refund: {}", unlocked_after);
    
    // 6. Test Supply Metrics
    println!("\n=== Testing Supply Metrics ===");
    
    let metrics = economics.supply.get_metrics().await;
    println!("Total Supply: {}", metrics.total_supply);
    println!("Circulating Supply: {}", metrics.circulating_supply);
    println!("Treasury Balance: {}", metrics.treasury_balance);
    println!("Liquidity Balance: {}", metrics.liquidity_balance);
    println!("Genesis Balance: {}", metrics.genesis_balance);
    
    // Verify max supply constraint
    let remaining = economics.supply.remaining_mintable().await;
    println!("Remaining mintable: {}", remaining);
    assert!(remaining.to_adic() <= (1_000_000_000.0 - 300_000_000.0));
    
    println!("\n=== All Tests Passed ===");
}

#[tokio::test]
async fn test_emission_decay_formula() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = EconomicsEngine::new(storage).await.unwrap();
    
    // Initialize to have a base for emission calculations
    economics.initialize_genesis().await.unwrap();
    
    // Test emission rate decay over years
    let years_to_test = vec![0.0, 1.0, 3.0, 6.0, 12.0, 18.0, 24.0];
    
    println!("\n=== Emission Rate Decay ===");
    for years in years_to_test {
        // Manually calculate expected rate
        let expected_rate = 0.01 * 0.5_f64.powf(years / 6.0);
        
        // Create a custom schedule
        let mut schedule = EmissionSchedule::default();
        schedule.start_timestamp = chrono::Utc::now().timestamp() - (years * 365.25 * 24.0 * 3600.0) as i64;
        
        let _controller = economics.emission.clone();
        let controller_with_schedule = adic_economics::EmissionController::new(
            economics.supply.clone(),
            economics.balances.clone(),
        ).with_schedule(schedule);
        
        let rate = controller_with_schedule.get_current_emission_rate().await;
        
        println!("Year {}: {:.6}% (expected: {:.6}%)", years, rate * 100.0, expected_rate * 100.0);
        assert!((rate - expected_rate).abs() < 0.0001);
    }
}

#[tokio::test]
async fn test_max_supply_enforcement() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = EconomicsEngine::new(storage).await.unwrap();
    
    // Initialize genesis
    economics.initialize_genesis().await.unwrap();
    
    // Try to mint more than max supply allows
    let remaining = economics.supply.remaining_mintable().await;
    
    // This should succeed
    assert!(economics.supply.mint_emission(remaining).await.is_ok());
    
    // This should fail
    assert!(economics.supply.mint_emission(AdicAmount::from_adic(1.0)).await.is_err());
    
    // Verify total equals max
    assert_eq!(economics.get_total_supply().await, AdicAmount::MAX_SUPPLY);
}

#[tokio::test]
async fn test_treasury_multisig_threshold() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = EconomicsEngine::new(storage).await.unwrap();
    
    economics.initialize_genesis().await.unwrap();
    
    // Use the signers from genesis config (already initialized with 2-of-3)
    let signers = vec![
        AccountAddress::from_bytes([0xAA; 32]),
        AccountAddress::from_bytes([0xBB; 32]),
        AccountAddress::from_bytes([0xCC; 32]),
    ];
    
    let recipient = AccountAddress::from_bytes([100; 32]);
    let amount = AdicAmount::from_adic(100.0);
    
    // Create proposal (proposer must be a signer)
    let proposal_id = economics.treasury.propose_transfer(
        recipient,
        amount,
        "Test".to_string(),
        signers[0],  // First signer proposes
    ).await.unwrap();
    
    // First approval (proposer already approved)
    // Second approval - should execute (2-of-3 threshold)
    assert!(economics.treasury.approve_proposal(proposal_id, signers[1]).await.unwrap());
    
    // Verify execution
    let balance = economics.balances.get_balance(recipient).await.unwrap();
    assert_eq!(balance, amount);
}