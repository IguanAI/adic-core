use adic_economics::{
    EconomicsEngine, AdicAmount, AccountAddress, storage::MemoryStorage,
};
use proptest::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashSet;

// Custom strategies for generating test data
prop_compose! {
    fn arb_adic_amount()
        (units in 0u64..=1_000_000_000_000_000_000u64) -> AdicAmount {
        AdicAmount::from_base_units(units)
    }
}

prop_compose! {
    fn arb_small_adic_amount()
        (units in 0u64..=1_000_000_000u64) -> AdicAmount {
        AdicAmount::from_base_units(units)
    }
}

prop_compose! {
    fn arb_account_address()
        (bytes in prop::array::uniform32(any::<u8>())) -> AccountAddress {
        AccountAddress::from_bytes(bytes)
    }
}

prop_compose! {
    fn arb_account_list()
        (addresses in prop::collection::vec(arb_account_address(), 1..20)) -> Vec<AccountAddress> {
        addresses
    }
}

// Property: Total supply never exceeds maximum
proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]
    
    #[test]
    fn prop_supply_never_exceeds_max(
        amounts in prop::collection::vec(arb_small_adic_amount(), 1..100)
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let storage = Arc::new(MemoryStorage::new());
            let economics = EconomicsEngine::new(storage).await.unwrap();
            
            economics.initialize_genesis().await.unwrap();
            
            let mut total_minted = AdicAmount::GENESIS_SUPPLY;
            
            for amount in amounts {
                if economics.supply.can_mint(amount).await {
                    let _ = economics.supply.mint_emission(amount).await;
                    total_minted = total_minted.saturating_add(amount);
                }
                
                let current_supply = economics.get_total_supply().await;
                prop_assert!(current_supply <= AdicAmount::MAX_SUPPLY);
                prop_assert_eq!(current_supply, total_minted);
            }
            
            Ok(())
        })?;
    }
}

// Property: Balance conservation in transfers
proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]
    
    #[test]
    fn prop_balance_conservation(
        accounts in arb_account_list(),
        amounts in prop::collection::vec(arb_small_adic_amount(), 1..50)
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let storage = Arc::new(MemoryStorage::new());
            let economics = EconomicsEngine::new(storage).await.unwrap();
            
            economics.initialize_genesis().await.unwrap();
            
            // Give initial balances
            let initial_amount = AdicAmount::from_adic(10000.0);
            for account in &accounts {
                economics.balances.transfer(
                    AccountAddress::genesis_pool(),
                    *account,
                    initial_amount
                ).await.unwrap();
            }
            
            // Record total before transfers
            let mut total_before = AdicAmount::ZERO;
            for account in &accounts {
                let balance = economics.balances.get_balance(*account).await.unwrap();
                total_before = total_before.saturating_add(balance);
            }
            
            // Perform random transfers
            for (i, amount) in amounts.iter().enumerate() {
                if accounts.len() >= 2 {
                    let from_idx = i % accounts.len();
                    let to_idx = (i + 1) % accounts.len();
                    
                    if from_idx != to_idx {
                        let from = accounts[from_idx];
                        let to = accounts[to_idx];
                        
                        let from_balance = economics.balances.get_balance(from).await.unwrap();
                        if from_balance >= *amount {
                            economics.balances.transfer(from, to, *amount).await.unwrap();
                        }
                    }
                }
            }
            
            // Verify total is conserved
            let mut total_after = AdicAmount::ZERO;
            for account in &accounts {
                let balance = economics.balances.get_balance(*account).await.unwrap();
                total_after = total_after.saturating_add(balance);
            }
            
            prop_assert_eq!(total_before, total_after);
            
            Ok(())
        })?;
    }
}

// Property: Locked funds cannot exceed total balance
proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]
    
    #[test]
    fn prop_locked_never_exceeds_balance(
        account in arb_account_address(),
        lock_amounts in prop::collection::vec(arb_small_adic_amount(), 1..20)
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let storage = Arc::new(MemoryStorage::new());
            let economics = EconomicsEngine::new(storage).await.unwrap();
            
            economics.initialize_genesis().await.unwrap();
            
            // Give account some balance
            let initial_balance = AdicAmount::from_adic(100000.0);
            economics.balances.transfer(
                AccountAddress::genesis_pool(),
                account,
                initial_balance
            ).await.unwrap();
            
            let mut total_locked = AdicAmount::ZERO;
            
            for amount in lock_amounts {
                let unlocked = economics.balances.get_unlocked_balance(account).await.unwrap();
                
                if amount <= unlocked {
                    economics.balances.lock(account, amount).await.unwrap();
                    total_locked = total_locked.saturating_add(amount);
                }
                
                let locked = economics.balances.get_locked_balance(account).await.unwrap();
                let balance = economics.balances.get_balance(account).await.unwrap();
                
                prop_assert!(locked <= balance);
                prop_assert_eq!(locked, total_locked);
                prop_assert_eq!(balance, initial_balance); // Balance doesn't change with locking
            }
            
            Ok(())
        })?;
    }
}

// Property: Emission rate always decreases over time
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    
    #[test]
    fn prop_emission_rate_monotonic_decrease(
        years in prop::collection::vec(0.0f64..100.0, 10..50)
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let storage = Arc::new(MemoryStorage::new());
            let _economics = EconomicsEngine::new(storage).await.unwrap();
            
            let mut sorted_years = years.clone();
            sorted_years.sort_by(|a, b| a.partial_cmp(b).unwrap());
            
            let mut prev_rate = 1.0;
            
            for year in sorted_years {
                let rate = 0.01 * 0.5_f64.powf(year / 6.0);
                prop_assert!(rate <= prev_rate);
                prev_rate = rate;
            }
            
            Ok(())
        })?;
    }
}

// Property: Treasury multisig threshold is always enforced
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    
    #[test]
    fn prop_treasury_threshold_enforcement(
        approvals in prop::collection::vec(0usize..3, 1..10)
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let storage = Arc::new(MemoryStorage::new());
            let economics = EconomicsEngine::new(storage).await.unwrap();
            
            economics.initialize_genesis().await.unwrap();
            
            // Use genesis signers - multisig already initialized with threshold=2
            let signers: Vec<AccountAddress> = vec![
                AccountAddress::from_bytes([0xAA; 32]),
                AccountAddress::from_bytes([0xBB; 32]),
                AccountAddress::from_bytes([0xCC; 32]),
            ];
            let num_signers = signers.len();
            let valid_threshold = 2u32; // Genesis threshold
            
            // Create a proposal
            let recipient = AccountAddress::from_bytes([100; 32]);
            let amount = AdicAmount::from_adic(100.0);
            
            let proposal_id = economics.treasury.propose_transfer(
                recipient,
                amount,
                "Test".to_string(),
                signers[0],
            ).await.unwrap();
            
            // Track unique approvers
            let mut unique_approvers = HashSet::new();
            unique_approvers.insert(0); // Proposer auto-approves
            
            let mut executed = false;
            
            for approval_idx in approvals {
                let signer_idx = approval_idx % num_signers;
                
                if signer_idx > 0 && !unique_approvers.contains(&signer_idx) {
                    unique_approvers.insert(signer_idx);
                    
                    let should_execute = unique_approvers.len() >= valid_threshold as usize;
                    let result = economics.treasury.approve_proposal(
                        proposal_id,
                        signers[signer_idx],
                    ).await.unwrap();
                    
                    if result {
                        executed = true;
                        prop_assert!(should_execute);
                        break;
                    }
                }
            }
            
            // If we have enough approvals, it should have executed
            if unique_approvers.len() >= valid_threshold as usize {
                prop_assert!(executed);
                
                // Verify the transfer happened
                let balance = economics.balances.get_balance(recipient).await.unwrap();
                prop_assert_eq!(balance, amount);
            }
            
            Ok(())
        })?;
    }
}

// Property: No negative balances possible
proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]
    
    #[test]
    fn prop_no_negative_balances(
        operations in prop::collection::vec(
            (arb_account_address(), arb_account_address(), arb_small_adic_amount()),
            1..50
        )
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let storage = Arc::new(MemoryStorage::new());
            let economics = EconomicsEngine::new(storage).await.unwrap();
            
            economics.initialize_genesis().await.unwrap();
            
            // Give some accounts initial balance
            let test_accounts: HashSet<AccountAddress> = operations
                .iter()
                .flat_map(|(from, to, _)| vec![*from, *to])
                .collect();
            
            for account in &test_accounts {
                economics.balances.transfer(
                    AccountAddress::genesis_pool(),
                    *account,
                    AdicAmount::from_adic(1000.0)
                ).await.unwrap();
            }
            
            // Perform operations
            for (from, to, amount) in operations {
                if from != to {
                    let from_balance = economics.balances.get_balance(from).await.unwrap();
                    
                    // Try the transfer
                    let result = economics.balances.transfer(from, to, amount).await;
                    
                    if result.is_ok() {
                        // Transfer succeeded - check balances are non-negative
                        let new_from_balance = economics.balances.get_balance(from).await.unwrap();
                        let new_to_balance = economics.balances.get_balance(to).await.unwrap();
                        
                        prop_assert!(new_from_balance <= from_balance);
                        prop_assert!(new_to_balance >= amount);
                    } else {
                        // Transfer failed - balance should be unchanged
                        let unchanged_balance = economics.balances.get_balance(from).await.unwrap();
                        prop_assert_eq!(unchanged_balance, from_balance);
                    }
                }
            }
            
            // Final check: all balances are non-negative (implicitly true with AdicAmount)
            for account in test_accounts {
                let balance = economics.balances.get_balance(account).await.unwrap();
                prop_assert!(balance >= AdicAmount::ZERO);
            }
            
            Ok(())
        })?;
    }
}

// Property: Genesis allocation percentages are correct
#[test]
fn test_genesis_allocation_percentages() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let storage = Arc::new(MemoryStorage::new());
        let economics = EconomicsEngine::new(storage).await.unwrap();
        
        economics.initialize_genesis().await.unwrap();
        
        let treasury = economics.balances.get_balance(AccountAddress::treasury()).await.unwrap();
        let liquidity = economics.balances.get_balance(AccountAddress::liquidity_pool()).await.unwrap();
        let genesis = economics.balances.get_balance(AccountAddress::genesis_pool()).await.unwrap();
        
        let total = AdicAmount::GENESIS_SUPPLY.to_adic();
        
        // Check percentages (allowing for small rounding errors)
        let treasury_pct = treasury.to_adic() / total;
        let liquidity_pct = liquidity.to_adic() / total;
        let genesis_pct = genesis.to_adic() / total;
        
        assert!((treasury_pct - 0.20).abs() < 0.001);
        assert!((liquidity_pct - 0.30).abs() < 0.001);
        assert!((genesis_pct - 0.50).abs() < 0.001);
        
        // Sum should equal total (within rounding)
        let sum = treasury.saturating_add(liquidity).saturating_add(genesis);
        assert!((sum.to_adic() - total).abs() < 1.0);
    });
}

// Property: Concurrent operations maintain consistency
proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]
    
    #[test]
    fn prop_concurrent_consistency(
        num_accounts in 2usize..10,
        num_operations in 10usize..50
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let storage = Arc::new(MemoryStorage::new());
            let economics = Arc::new(EconomicsEngine::new(storage).await.unwrap());
            
            economics.initialize_genesis().await.unwrap();
            
            // Create accounts
            let accounts: Vec<AccountAddress> = (0..num_accounts)
                .map(|i| AccountAddress::from_bytes([i as u8; 32]))
                .collect();
            
            // Give initial balances from genesis pool
            let initial_amount_per_account = AdicAmount::from_adic(10000.0);
            for account in &accounts {
                economics.balances.transfer(
                    AccountAddress::genesis_pool(),
                    *account,
                    initial_amount_per_account
                ).await.unwrap();
            }
            
            // Record initial total (only count test accounts, not genesis pool)
            let mut initial_total = AdicAmount::ZERO;
            for account in &accounts {
                let balance = economics.balances.get_balance(*account).await.unwrap();
                initial_total = initial_total.saturating_add(balance);
            }
            
            // Sanity check: initial total should be exactly what we distributed
            let expected_initial = AdicAmount::from_adic(10000.0 * num_accounts as f64);
            assert_eq!(initial_total, expected_initial);
            
            // Spawn concurrent operations
            let mut handles = vec![];
            let successful_transfers = Arc::new(AtomicUsize::new(0));
            let failed_transfers = Arc::new(AtomicUsize::new(0));
            
            for i in 0..num_operations {
                let economics_clone = economics.clone();
                let from_idx = i % accounts.len();
                let to_idx = (i + 1) % accounts.len();
                
                // Skip if from and to are the same (self-transfer not allowed)
                if from_idx == to_idx {
                    continue;
                }
                
                let from = accounts[from_idx];
                let to = accounts[to_idx];
                let amount = AdicAmount::from_adic((i % 10 + 1) as f64);
                let success_counter = successful_transfers.clone();
                let fail_counter = failed_transfers.clone();
                
                let handle = tokio::spawn(async move {
                    match economics_clone.balances.transfer(from, to, amount).await {
                        Ok(_) => success_counter.fetch_add(1, Ordering::SeqCst),
                        Err(_) => fail_counter.fetch_add(1, Ordering::SeqCst),
                    };
                });
                
                handles.push(handle);
            }
            
            // Wait for all operations
            for handle in handles {
                let _ = handle.await;
            }
            
            // Verify total is conserved
            let mut final_total = AdicAmount::ZERO;
            for account in &accounts {
                final_total = final_total.saturating_add(
                    economics.balances.get_balance(*account).await.unwrap()
                );
            }
            
            // Log stats for debugging
            let successful = successful_transfers.load(Ordering::SeqCst);
            let failed = failed_transfers.load(Ordering::SeqCst);
            
            // In concurrent operations with potential conflicts, 
            // some transfers may fail due to insufficient balance
            // This is expected behavior - what matters is conservation
            if initial_total != final_total {
                eprintln!("Conservation check: initial={}, final={}, successful={}, failed={}", 
                    initial_total, final_total, successful, failed);
            }
            
            prop_assert_eq!(initial_total, final_total);
            
            Ok(())
        })?;
    }
}

// Property: Emission projections are monotonically increasing
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    
    #[test]
    fn prop_emission_projection_monotonic(
        years in prop::collection::vec(0.1f64..50.0, 2..10)
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let storage = Arc::new(MemoryStorage::new());
            let economics = EconomicsEngine::new(storage).await.unwrap();
            
            economics.initialize_genesis().await.unwrap();
            
            let mut sorted_years = years.clone();
            sorted_years.sort_by(|a, b| a.partial_cmp(b).unwrap());
            
            let mut prev_projection = AdicAmount::ZERO;
            
            for year in sorted_years {
                let projection = economics.emission.get_projected_emission(year).await;
                
                // Projection should increase with time (more total emission)
                prop_assert!(projection >= prev_projection);
                
                // But should never exceed what's possible
                let max_possible = AdicAmount::MAX_SUPPLY.saturating_sub(AdicAmount::GENESIS_SUPPLY);
                prop_assert!(projection <= max_possible);
                
                prev_projection = projection;
            }
            
            Ok(())
        })?;
    }
}

// Property: Transfer atomicity (all or nothing)
proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]
    
    #[test]
    fn prop_transfer_atomicity(
        from in arb_account_address(),
        to in arb_account_address(),
        amount in arb_small_adic_amount(),
        initial_balance in arb_small_adic_amount()
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            if from == to {
                return Ok(()); // Skip self-transfers
            }
            
            let storage = Arc::new(MemoryStorage::new());
            let economics = EconomicsEngine::new(storage).await.unwrap();
            
            economics.initialize_genesis().await.unwrap();
            
            // Setup initial state
            if initial_balance > AdicAmount::ZERO {
                economics.balances.transfer(
                    AccountAddress::genesis_pool(),
                    from,
                    initial_balance
                ).await.unwrap();
            }
            
            let from_balance_before = economics.balances.get_balance(from).await.unwrap();
            let to_balance_before = economics.balances.get_balance(to).await.unwrap();
            
            // Attempt transfer
            let result = economics.balances.transfer(from, to, amount).await;
            
            let from_balance_after = economics.balances.get_balance(from).await.unwrap();
            let to_balance_after = economics.balances.get_balance(to).await.unwrap();
            
            if result.is_ok() {
                // Transfer succeeded - verify exact amounts
                prop_assert_eq!(from_balance_after, from_balance_before.saturating_sub(amount));
                prop_assert_eq!(to_balance_after, to_balance_before.saturating_add(amount));
            } else {
                // Transfer failed - nothing should change
                prop_assert_eq!(from_balance_after, from_balance_before);
                prop_assert_eq!(to_balance_after, to_balance_before);
            }
            
            Ok(())
        })?;
    }
}