use adic_economics::{
    storage::MemoryStorage, AccountAddress, AdicAmount, EconomicsEngine, EmissionSchedule,
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
    // Total supply includes genesis (300M) + faucet allocation (10M)
    let expected_total =
        AdicAmount::GENESIS_SUPPLY.saturating_add(AdicAmount::from_adic(10_000_000.0));
    assert_eq!(economics.get_total_supply().await, expected_total);

    // Verify total supply matches genesis + faucet
    assert_eq!(economics.get_total_supply().await, expected_total);

    // Verify allocations (20% Treasury, 30% Liquidity, 50% Genesis)
    let treasury_balance = economics
        .balances
        .get_balance(AccountAddress::treasury())
        .await
        .unwrap();
    let expected_treasury = AdicAmount::from_adic(300_000_000.0 * 0.20);
    assert_eq!(treasury_balance, expected_treasury);
    println!("Treasury balance: {}", treasury_balance);

    let liquidity_balance = economics
        .balances
        .get_balance(AccountAddress::liquidity_pool())
        .await
        .unwrap();
    let expected_liquidity = AdicAmount::from_adic(300_000_000.0 * 0.30);
    assert_eq!(liquidity_balance, expected_liquidity);
    println!("Liquidity balance: {}", liquidity_balance);

    let genesis_balance = economics
        .balances
        .get_balance(AccountAddress::genesis_pool())
        .await
        .unwrap();
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

    let proposal_id = economics
        .treasury
        .propose_transfer(
            recipient,
            proposal_amount,
            "Development grant".to_string(),
            signer1,
        )
        .await
        .unwrap();

    println!("Created proposal: {}", hex::encode(&proposal_id[..8]));

    // First approval (proposer already approved)
    // Second approval should execute
    let executed = economics
        .treasury
        .approve_proposal(proposal_id, signer2)
        .await
        .unwrap();
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
    economics
        .balances
        .transfer(
            AccountAddress::genesis_pool(),
            user1,
            AdicAmount::from_adic(1000.0),
        )
        .await
        .unwrap();
    let user1_balance = economics.balances.get_balance(user1).await.unwrap();
    assert_eq!(user1_balance, AdicAmount::from_adic(1000.0));
    println!("User1 balance after genesis release: {}", user1_balance);

    // Transfer between users
    economics
        .balances
        .transfer(user1, user2, AdicAmount::from_adic(250.0))
        .await
        .unwrap();

    let user1_after = economics.balances.get_balance(user1).await.unwrap();
    let user2_after = economics.balances.get_balance(user2).await.unwrap();

    assert_eq!(user1_after, AdicAmount::from_adic(750.0));
    assert_eq!(user2_after, AdicAmount::from_adic(250.0));
    println!(
        "After transfer - User1: {}, User2: {}",
        user1_after, user2_after
    );

    // 5. Test Deposit Integration
    println!("\n=== Testing Deposit Integration ===");

    // Lock funds for deposit
    let deposit_amount = AdicAmount::from_adic(10.0);
    economics
        .balances
        .lock(user1, deposit_amount)
        .await
        .unwrap();

    let locked = economics.balances.get_locked_balance(user1).await.unwrap();
    let unlocked = economics
        .balances
        .get_unlocked_balance(user1)
        .await
        .unwrap();

    assert_eq!(locked, deposit_amount);
    assert_eq!(unlocked, AdicAmount::from_adic(740.0));
    println!("User1 locked: {}, unlocked: {}", locked, unlocked);

    // Simulate deposit refund
    economics
        .balances
        .unlock(user1, deposit_amount)
        .await
        .unwrap();
    let unlocked_after = economics
        .balances
        .get_unlocked_balance(user1)
        .await
        .unwrap();
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
        let schedule = EmissionSchedule {
            start_timestamp: chrono::Utc::now().timestamp()
                - (years * 365.25 * 24.0 * 3600.0) as i64,
            ..Default::default()
        };

        let _controller = economics.emission.clone();
        let controller_with_schedule = adic_economics::EmissionController::new(
            economics.supply.clone(),
            economics.balances.clone(),
        )
        .with_schedule(schedule);

        let rate = controller_with_schedule.get_current_emission_rate().await;

        println!(
            "Year {}: {:.6}% (expected: {:.6}%)",
            years,
            rate * 100.0,
            expected_rate * 100.0
        );
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
    assert!(economics
        .supply
        .mint_emission(AdicAmount::from_adic(1.0))
        .await
        .is_err());

    // Verify total equals max
    assert_eq!(economics.get_total_supply().await, AdicAmount::MAX_SUPPLY);
}

#[tokio::test]
async fn test_treasury_multisig_threshold() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = EconomicsEngine::new(storage).await.unwrap();

    economics.initialize_genesis().await.unwrap();

    // Use the signers from genesis config (already initialized with 2-of-3)
    let signers = [
        AccountAddress::from_bytes([0xAA; 32]),
        AccountAddress::from_bytes([0xBB; 32]),
        AccountAddress::from_bytes([0xCC; 32]),
    ];

    let recipient = AccountAddress::from_bytes([100; 32]);
    let amount = AdicAmount::from_adic(100.0);

    // Create proposal (proposer must be a signer)
    let proposal_id = economics
        .treasury
        .propose_transfer(
            recipient,
            amount,
            "Test".to_string(),
            signers[0], // First signer proposes
        )
        .await
        .unwrap();

    // First approval (proposer already approved)
    // Second approval - should execute (2-of-3 threshold)
    assert!(economics
        .treasury
        .approve_proposal(proposal_id, signers[1])
        .await
        .unwrap());

    // Verify execution
    let balance = economics.balances.get_balance(recipient).await.unwrap();
    assert_eq!(balance, amount);
}

#[tokio::test]
async fn test_transaction_history_paginated() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = EconomicsEngine::new(storage.clone()).await.unwrap();

    let sender = AccountAddress::from_bytes([1u8; 32]);
    let recipient = AccountAddress::from_bytes([2u8; 32]);

    // Create initial balance
    economics
        .balances
        .credit(sender, AdicAmount::from_adic(1000000.0))
        .await
        .unwrap();

    // Create 20 transactions
    for i in 0..20 {
        let amount = AdicAmount::from_adic(100.0 + i as f64);
        economics
            .balances
            .transfer(sender, recipient, amount)
            .await
            .unwrap();
        // Small delay to ensure different timestamps
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    // Test: First page with limit 5
    let (page1, cursor1) = economics
        .get_transaction_history_paginated(sender, 5, None)
        .await
        .unwrap();

    assert_eq!(page1.len(), 5);
    assert!(cursor1.is_some());

    // Verify transactions are in chronological order (newest first)
    for i in 0..page1.len() - 1 {
        assert!(page1[i].timestamp >= page1[i + 1].timestamp);
    }

    // Test: Second page using cursor
    let (page2, cursor2) = economics
        .get_transaction_history_paginated(sender, 5, cursor1)
        .await
        .unwrap();

    assert_eq!(page2.len(), 5);
    assert!(cursor2.is_some());

    // Verify no overlap between pages
    let page1_ids: std::collections::HashSet<_> =
        page1.iter().map(|tx| tx.tx_hash.clone()).collect();
    let page2_ids: std::collections::HashSet<_> =
        page2.iter().map(|tx| tx.tx_hash.clone()).collect();
    assert!(page1_ids.is_disjoint(&page2_ids));

    // Continue until we get all transactions
    let mut all_txs = vec![];
    all_txs.extend(page1);
    all_txs.extend(page2);

    let mut current_cursor = cursor2;
    loop {
        let (page, next_cursor) = economics
            .get_transaction_history_paginated(sender, 5, current_cursor)
            .await
            .unwrap();

        if page.is_empty() {
            break;
        }

        all_txs.extend(page);
        current_cursor = next_cursor;

        if current_cursor.is_none() {
            break;
        }
    }

    // Should have all 20 transactions
    assert_eq!(all_txs.len(), 20);
}

#[tokio::test]
async fn test_transaction_ordering_by_timestamp() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = EconomicsEngine::new(storage.clone()).await.unwrap();

    let account = AccountAddress::from_bytes([3u8; 32]);
    let other = AccountAddress::from_bytes([4u8; 32]);

    // Create balance
    economics
        .balances
        .credit(account, AdicAmount::from_adic(10000.0))
        .await
        .unwrap();

    // Create transactions with deliberate delays
    let mut expected_order = vec![];

    for i in 0..5 {
        let amount = AdicAmount::from_adic(100.0 + i as f64);
        economics
            .balances
            .transfer(account, other, amount)
            .await
            .unwrap();

        expected_order.push(amount);
        // Ensure distinct timestamps
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    // Get all transactions
    let (txs, _) = economics
        .get_transaction_history_paginated(account, 10, None)
        .await
        .unwrap();

    assert_eq!(txs.len(), 5);

    // Verify chronological ordering (newest first)
    for i in 0..txs.len() - 1 {
        assert!(
            txs[i].timestamp >= txs[i + 1].timestamp,
            "Transaction {} timestamp {} should be >= transaction {} timestamp {}",
            i,
            txs[i].timestamp,
            i + 1,
            txs[i + 1].timestamp
        );
    }

    // Verify amounts match expected order (reversed since newest first)
    expected_order.reverse();
    for (i, tx) in txs.iter().enumerate() {
        assert_eq!(tx.amount, expected_order[i]);
    }
}

#[tokio::test]
async fn test_pagination_cursor_stability() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = EconomicsEngine::new(storage.clone()).await.unwrap();

    let account = AccountAddress::from_bytes([5u8; 32]);
    let other = AccountAddress::from_bytes([6u8; 32]);

    // Create balance and transactions
    economics
        .balances
        .credit(account, AdicAmount::from_adic(5000.0))
        .await
        .unwrap();

    for i in 0..10 {
        let amount = AdicAmount::from_adic(100.0 + i as f64);
        economics
            .balances
            .transfer(account, other, amount)
            .await
            .unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    // Get first page and cursor
    let (page1_a, cursor_a) = economics
        .get_transaction_history_paginated(account, 3, None)
        .await
        .unwrap();

    // Use same parameters again - should get same results
    let (page1_b, cursor_b) = economics
        .get_transaction_history_paginated(account, 3, None)
        .await
        .unwrap();

    assert_eq!(page1_a.len(), page1_b.len());
    assert_eq!(cursor_a, cursor_b);

    // Verify transaction IDs match
    for (tx_a, tx_b) in page1_a.iter().zip(page1_b.iter()) {
        assert_eq!(tx_a.tx_hash, tx_b.tx_hash);
        assert_eq!(tx_a.amount, tx_b.amount);
        assert_eq!(tx_a.timestamp, tx_b.timestamp);
    }

    // Test cursor stability - using same cursor should give same results
    let (page2_a, _) = economics
        .get_transaction_history_paginated(account, 3, cursor_a.clone())
        .await
        .unwrap();

    let (page2_b, _) = economics
        .get_transaction_history_paginated(account, 3, cursor_a)
        .await
        .unwrap();

    assert_eq!(page2_a.len(), page2_b.len());
    for (tx_a, tx_b) in page2_a.iter().zip(page2_b.iter()) {
        assert_eq!(tx_a.tx_hash, tx_b.tx_hash);
    }
}

#[tokio::test]
async fn test_empty_transaction_history() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = EconomicsEngine::new(storage.clone()).await.unwrap();

    let account = AccountAddress::from_bytes([7u8; 32]);

    // Query empty history
    let (txs, cursor) = economics
        .get_transaction_history_paginated(account, 10, None)
        .await
        .unwrap();

    assert_eq!(txs.len(), 0);
    assert!(cursor.is_none());
}
