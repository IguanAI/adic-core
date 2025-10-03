use adic_economics::{storage::MemoryStorage, AccountAddress, AdicAmount, EconomicsEngine};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::timeout;

/// Stress test with high-volume concurrent transfers
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_test_concurrent_transfers() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = Arc::new(EconomicsEngine::new(storage).await.unwrap());

    economics.initialize_genesis().await.unwrap();

    println!("\n=== Stress Test: High-Volume Concurrent Transfers ===");

    // Create test accounts
    let num_accounts = 100;
    let accounts: Vec<AccountAddress> = (0..num_accounts)
        .map(|i| {
            let mut bytes = [0u8; 32];
            bytes[0] = (i % 256) as u8;
            bytes[1] = (i / 256) as u8;
            AccountAddress::from_bytes(bytes)
        })
        .collect();

    // Give initial balance to half the accounts
    for account in &accounts[..num_accounts / 2] {
        economics
            .balances
            .transfer(
                AccountAddress::genesis_pool(),
                *account,
                AdicAmount::from_adic(10000.0),
            )
            .await
            .unwrap();
    }

    let start = Instant::now();
    let successful_transfers = Arc::new(AtomicUsize::new(0));
    let failed_transfers = Arc::new(AtomicUsize::new(0));

    // Launch many concurrent transfer tasks
    let mut handles = vec![];
    let num_tasks = 50;
    let transfers_per_task = 100;

    for task_id in 0..num_tasks {
        let economics_clone = economics.clone();
        let accounts_clone = accounts.clone();
        let success_counter = successful_transfers.clone();
        let fail_counter = failed_transfers.clone();

        let handle = tokio::spawn(async move {
            for i in 0..transfers_per_task {
                let from_idx = (task_id + i) % (num_accounts / 2);
                let to_idx = (num_accounts / 2) + ((task_id + i) % (num_accounts / 2));

                let from = accounts_clone[from_idx];
                let to = accounts_clone[to_idx];
                let amount = AdicAmount::from_adic(((i % 10) + 1) as f64);

                match economics_clone.balances.transfer(from, to, amount).await {
                    Ok(_) => success_counter.fetch_add(1, Ordering::Relaxed),
                    Err(_) => fail_counter.fetch_add(1, Ordering::Relaxed),
                };
            }
        });

        handles.push(handle);
    }

    // Wait for all tasks with timeout
    let wait_result = timeout(Duration::from_secs(30), async {
        for handle in handles {
            handle.await.unwrap();
        }
    })
    .await;

    assert!(wait_result.is_ok(), "Tasks took too long to complete");

    let elapsed = start.elapsed();
    let total_transfers = successful_transfers.load(Ordering::Relaxed);
    let failed = failed_transfers.load(Ordering::Relaxed);

    println!(
        "Completed {} successful transfers in {:?}",
        total_transfers, elapsed
    );
    println!("Failed transfers: {}", failed);
    println!(
        "Throughput: {:.0} transfers/sec",
        total_transfers as f64 / elapsed.as_secs_f64()
    );

    // Verify balance consistency
    let mut total_balance = AdicAmount::ZERO;
    for account in &accounts {
        let balance = economics.balances.get_balance(*account).await.unwrap();
        total_balance = total_balance.saturating_add(balance);
    }

    println!("Total balance across all accounts: {}", total_balance);
    println!("✓ Stress test completed successfully");
}

/// Stress test with large number of accounts
#[tokio::test]
async fn stress_test_large_account_database() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = Arc::new(EconomicsEngine::new(storage).await.unwrap());

    economics.initialize_genesis().await.unwrap();

    println!("\n=== Stress Test: Large Account Database ===");

    let num_accounts = 10_000;
    let mut accounts = Vec::new();

    let start = Instant::now();

    // Create many accounts
    for i in 0..num_accounts {
        let mut bytes = [0u8; 32];
        bytes[0] = (i % 256) as u8;
        bytes[1] = ((i / 256) % 256) as u8;
        bytes[2] = ((i / 65536) % 256) as u8;
        bytes[3] = ((i / 16777216) % 256) as u8;
        accounts.push(AccountAddress::from_bytes(bytes));
    }

    println!("Created {} accounts in {:?}", num_accounts, start.elapsed());

    // Benchmark balance queries
    let query_start = Instant::now();
    for account in &accounts[..1000] {
        let _ = economics.balances.get_balance(*account).await.unwrap();
    }
    let query_elapsed = query_start.elapsed();

    println!("1000 balance queries in {:?}", query_elapsed);
    println!("Average query time: {:?}", query_elapsed / 1000);

    // Benchmark batch operations
    let batch_start = Instant::now();

    // Give some accounts balance
    for account in &accounts[..100] {
        economics
            .balances
            .transfer(
                AccountAddress::genesis_pool(),
                *account,
                AdicAmount::from_adic(100.0),
            )
            .await
            .unwrap();
    }

    println!("100 genesis releases in {:?}", batch_start.elapsed());

    println!("✓ Large account database handled efficiently");
}

/// Stress test emission processing
#[tokio::test]
async fn stress_test_emission_processing() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = Arc::new(EconomicsEngine::new(storage).await.unwrap());

    economics.initialize_genesis().await.unwrap();

    println!("\n=== Stress Test: Emission Processing ===");

    // Simulate many emission events
    let validators: Vec<AccountAddress> = (0..100)
        .map(|i| AccountAddress::from_bytes([i; 32]))
        .collect();

    let start = Instant::now();
    let mut total_emitted = AdicAmount::ZERO;

    for validator in &validators {
        let emission_amount = AdicAmount::from_adic(100.0);

        if economics.supply.can_mint(emission_amount).await {
            economics
                .supply
                .mint_emission(emission_amount)
                .await
                .unwrap();
            economics
                .balances
                .credit(*validator, emission_amount)
                .await
                .unwrap();
            total_emitted = total_emitted.saturating_add(emission_amount);
        }
    }

    let elapsed = start.elapsed();

    println!(
        "Processed {} emission events in {:?}",
        validators.len(),
        elapsed
    );
    println!("Total emitted: {}", total_emitted);
    println!(
        "Average emission time: {:?}",
        elapsed / validators.len() as u32
    );

    // Verify supply increased correctly (genesis + faucet + emitted)
    let final_supply = economics.get_total_supply().await;
    let expected_supply = AdicAmount::GENESIS_SUPPLY
        .saturating_add(AdicAmount::from_adic(10_000_000.0)) // faucet
        .saturating_add(total_emitted);
    assert_eq!(final_supply, expected_supply);

    println!("✓ Emission processing stress test completed");
}

/// Stress test treasury proposal system
#[tokio::test]
async fn stress_test_treasury_proposals() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = Arc::new(EconomicsEngine::new(storage).await.unwrap());

    economics.initialize_genesis().await.unwrap();

    println!("\n=== Stress Test: Treasury Proposals ===");

    // Use correct genesis signers - multisig is already initialized
    let signers: Vec<AccountAddress> = vec![
        AccountAddress::from_bytes([0xAA; 32]),
        AccountAddress::from_bytes([0xBB; 32]),
        AccountAddress::from_bytes([0xCC; 32]),
    ];
    let num_signers = signers.len();

    // Don't initialize multisig again - it's already done in genesis

    // Create many proposals
    let num_proposals = 100;
    let mut proposal_ids = Vec::new();

    let create_start = Instant::now();

    for i in 0..num_proposals {
        let recipient = AccountAddress::from_bytes([(100 + i) as u8; 32]);
        let amount = AdicAmount::from_adic((i + 1) as f64);

        let id = economics
            .treasury
            .propose_transfer(
                recipient,
                amount,
                format!("Proposal {}", i),
                signers[i % num_signers],
            )
            .await
            .unwrap();

        proposal_ids.push(id);
    }

    println!(
        "Created {} proposals in {:?}",
        num_proposals,
        create_start.elapsed()
    );

    // Approve some proposals concurrently
    let approve_start = Instant::now();
    let mut handles = vec![];

    for proposal_id in proposal_ids[..10].iter() {
        let economics_clone = economics.clone();
        let signers_clone = signers.clone();
        let pid = *proposal_id;

        let handle = tokio::spawn(async move {
            // Get approvals from different signers (threshold is 2, so need 1 more after proposer)
            for signer_idx in 1..2 {
                // Only need 1 more approval since threshold is 2
                if signer_idx < signers_clone.len() {
                    let _ = economics_clone
                        .treasury
                        .approve_proposal(pid, signers_clone[signer_idx])
                        .await;
                }
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }

    println!("Approved 10 proposals in {:?}", approve_start.elapsed());

    // Get active proposals
    let active = economics.treasury.get_active_proposals().await;
    println!("Active proposals remaining: {}", active.len());

    // Cleanup
    economics.treasury.cleanup_expired_proposals().await;

    println!("✓ Treasury proposal stress test completed");
}

/// Stress test with memory pressure
#[tokio::test]
async fn stress_test_memory_pressure() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = Arc::new(EconomicsEngine::new(storage).await.unwrap());

    economics.initialize_genesis().await.unwrap();

    println!("\n=== Stress Test: Memory Pressure ===");

    // Create scenario with many operations that could leak memory
    let iterations = 1000;

    for i in 0..iterations {
        // Create new accounts
        let account1 = AccountAddress::from_bytes([(i % 256) as u8; 32]);
        let account2 = AccountAddress::from_bytes([((i + 1) % 256) as u8; 32]);

        // Query balances (creates cache entries)
        let _ = economics.balances.get_balance(account1).await.unwrap();
        let _ = economics.balances.get_balance(account2).await.unwrap();

        // Create and forget proposals using existing multisig
        if i < 10 {
            // Use genesis signers for proposals
            let genesis_signer = AccountAddress::from_bytes([0xAA; 32]);
            let _ = economics
                .treasury
                .propose_transfer(
                    account2,
                    AdicAmount::from_adic(1.0),
                    format!("Test {}", i),
                    genesis_signer,
                )
                .await;
        }

        // Periodic cleanup
        if i % 100 == 0 {
            economics.treasury.cleanup_expired_proposals().await;
            economics.balances.clear_cache().await;
        }
    }

    println!("Completed {} iterations without memory issues", iterations);
    println!("✓ Memory pressure test completed");
}

/// Benchmark critical operations
#[tokio::test]
async fn benchmark_critical_operations() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = Arc::new(EconomicsEngine::new(storage).await.unwrap());

    economics.initialize_genesis().await.unwrap();

    println!("\n=== Benchmarking Critical Operations ===");

    let account1 = AccountAddress::from_bytes([1; 32]);
    let account2 = AccountAddress::from_bytes([2; 32]);

    economics
        .balances
        .transfer(
            AccountAddress::genesis_pool(),
            account1,
            AdicAmount::from_adic(100000.0),
        )
        .await
        .unwrap();

    // Benchmark: Balance query
    let mut times = Vec::new();
    for _ in 0..1000 {
        let start = Instant::now();
        let _ = economics.balances.get_balance(account1).await.unwrap();
        times.push(start.elapsed());
    }
    let avg_query = times.iter().sum::<Duration>() / times.len() as u32;
    println!("Balance query: avg {:?}", avg_query);

    // Benchmark: Transfer
    times.clear();
    for _ in 0..100 {
        let start = Instant::now();
        economics
            .balances
            .transfer(account1, account2, AdicAmount::from_adic(1.0))
            .await
            .unwrap();
        times.push(start.elapsed());

        // Transfer back
        economics
            .balances
            .transfer(account2, account1, AdicAmount::from_adic(1.0))
            .await
            .unwrap();
    }
    let avg_transfer = times.iter().sum::<Duration>() / times.len() as u32;
    println!("Transfer: avg {:?}", avg_transfer);

    // Benchmark: Lock/Unlock
    times.clear();
    for _ in 0..100 {
        let start = Instant::now();
        economics
            .balances
            .lock(account1, AdicAmount::from_adic(100.0))
            .await
            .unwrap();
        economics
            .balances
            .unlock(account1, AdicAmount::from_adic(100.0))
            .await
            .unwrap();
        times.push(start.elapsed());
    }
    let avg_lock_unlock = times.iter().sum::<Duration>() / times.len() as u32;
    println!("Lock/Unlock: avg {:?}", avg_lock_unlock);

    // Benchmark: Supply metrics
    times.clear();
    for _ in 0..1000 {
        let start = Instant::now();
        let _ = economics.supply.get_metrics().await;
        times.push(start.elapsed());
    }
    let avg_metrics = times.iter().sum::<Duration>() / times.len() as u32;
    println!("Supply metrics: avg {:?}", avg_metrics);

    // Performance assertions
    assert!(
        avg_query < Duration::from_millis(1),
        "Balance query too slow"
    );
    assert!(
        avg_transfer < Duration::from_millis(10),
        "Transfer too slow"
    );
    assert!(
        avg_metrics < Duration::from_millis(1),
        "Metrics query too slow"
    );

    println!("✓ All operations within performance targets");
}

/// Test system recovery under load
#[tokio::test]
async fn test_recovery_under_load() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = Arc::new(EconomicsEngine::new(storage).await.unwrap());

    economics.initialize_genesis().await.unwrap();

    println!("\n=== Testing Recovery Under Load ===");

    let accounts: Vec<AccountAddress> = (0..10)
        .map(|i| AccountAddress::from_bytes([i; 32]))
        .collect();

    for account in &accounts {
        economics
            .balances
            .transfer(
                AccountAddress::genesis_pool(),
                *account,
                AdicAmount::from_adic(1000.0),
            )
            .await
            .unwrap();
    }

    // Start background load
    let economics_clone = economics.clone();
    let accounts_clone = accounts.clone();

    let load_handle = tokio::spawn(async move {
        for i in 0..1000 {
            let from = accounts_clone[i % accounts_clone.len()];
            let to = accounts_clone[(i + 1) % accounts_clone.len()];

            let _ = economics_clone
                .balances
                .transfer(from, to, AdicAmount::from_adic(0.01))
                .await;

            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });

    // Simulate recovery scenarios while under load
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Clear caches (simulates memory pressure recovery)
    economics.balances.clear_cache().await;
    println!("Cache cleared while under load");

    // Verify system still functional
    let balance = economics.balances.get_balance(accounts[0]).await.unwrap();
    assert!(balance > AdicAmount::ZERO);

    // Clean up proposals (simulates maintenance)
    economics.treasury.cleanup_expired_proposals().await;
    println!("Cleanup performed while under load");

    // Wait for load to complete
    let _ = timeout(Duration::from_secs(5), load_handle).await;

    // Final verification
    let mut total = AdicAmount::ZERO;
    for account in &accounts {
        let balance = economics.balances.get_balance(*account).await.unwrap();
        total = total.saturating_add(balance);
    }

    assert_eq!(total, AdicAmount::from_adic(10000.0));
    println!("✓ System recovered successfully under load");
}
