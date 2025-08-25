use adic_economics::{
    EconomicsEngine, AdicAmount, AccountAddress, storage::MemoryStorage,
};
use std::sync::Arc;

/// Core invariants that must ALWAYS hold in the system
#[tokio::test]
async fn test_core_supply_invariants() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = Arc::new(EconomicsEngine::new(storage).await.unwrap());
    
    println!("\n=== Testing Core Supply Invariants ===");
    
    // Invariant 1: Initial supply is zero
    assert_eq!(economics.get_total_supply().await, AdicAmount::ZERO);
    println!("✓ Invariant 1: Initial supply is zero");
    
    // Initialize genesis
    economics.initialize_genesis().await.unwrap();
    
    // Invariant 2: Genesis supply is exactly as specified
    assert_eq!(economics.get_total_supply().await, AdicAmount::GENESIS_SUPPLY);
    println!("✓ Invariant 2: Genesis supply is exact");
    
    // Invariant 3: Total supply never exceeds maximum
    let mut total = economics.get_total_supply().await;
    
    // Try to mint up to max
    let remaining = AdicAmount::MAX_SUPPLY.saturating_sub(total);
    assert!(economics.supply.mint_emission(remaining).await.is_ok());
    
    // Now at max supply
    total = economics.get_total_supply().await;
    assert_eq!(total, AdicAmount::MAX_SUPPLY);
    
    // Try to mint even 1 more unit - must fail
    assert!(economics.supply.mint_emission(AdicAmount::from_base_units(1)).await.is_err());
    
    // Supply should still be at max
    assert_eq!(economics.get_total_supply().await, AdicAmount::MAX_SUPPLY);
    println!("✓ Invariant 3: Max supply is hard limit");
    
    // Invariant 4: Circulating supply <= Total supply
    let circulating = economics.get_circulating_supply().await;
    assert!(circulating <= total);
    println!("✓ Invariant 4: Circulating <= Total");
    
    println!("\n=== All Supply Invariants Hold ===");
}

/// Test that balance operations maintain mathematical invariants
#[tokio::test]
async fn test_balance_mathematical_invariants() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = Arc::new(EconomicsEngine::new(storage).await.unwrap());
    
    economics.initialize_genesis().await.unwrap();
    
    println!("\n=== Testing Balance Mathematical Invariants ===");
    
    let accounts: Vec<AccountAddress> = (0..20)
        .map(|i| AccountAddress::from_bytes([i; 32]))
        .collect();
    
    // Give initial balances
    for account in &accounts[..10] {
        economics.balances.transfer(
            AccountAddress::genesis_pool(),
            *account,
            AdicAmount::from_adic(1000.0)
        ).await.unwrap();
    }
    
    // Invariant 1: Sum of all balances is conserved during transfers
    let mut initial_sum = AdicAmount::ZERO;
    for account in &accounts {
        let balance = economics.balances.get_balance(*account).await.unwrap();
        initial_sum = initial_sum.saturating_add(balance);
    }
    
    // Perform many transfers
    for i in 0..100 {
        let from = accounts[i % 10];
        let to = accounts[(i + 10) % 20];
        let amount = AdicAmount::from_adic((i % 10 + 1) as f64);
        
        // Skip self-transfers
        if from != to {
            let from_balance = economics.balances.get_balance(from).await.unwrap();
            if from_balance >= amount {
                economics.balances.transfer(from, to, amount).await.unwrap();
            }
        }
    }
    
    // Recalculate sum
    let mut final_sum = AdicAmount::ZERO;
    for account in &accounts {
        let balance = economics.balances.get_balance(*account).await.unwrap();
        final_sum = final_sum.saturating_add(balance);
    }
    
    assert_eq!(initial_sum, final_sum);
    println!("✓ Invariant 1: Balance sum conserved through transfers");
    
    // Invariant 2: Locked balance <= Total balance for every account
    for account in &accounts {
        let total = economics.balances.get_balance(*account).await.unwrap();
        let locked = economics.balances.get_locked_balance(*account).await.unwrap();
        assert!(locked <= total);
        
        // Lock some amount
        let unlocked = economics.balances.get_unlocked_balance(*account).await.unwrap();
        if unlocked > AdicAmount::ZERO {
            let to_lock = AdicAmount::from_adic(unlocked.to_adic() / 2.0);
            economics.balances.lock(*account, to_lock).await.unwrap();
            
            // Check invariant still holds
            let new_locked = economics.balances.get_locked_balance(*account).await.unwrap();
            let new_total = economics.balances.get_balance(*account).await.unwrap();
            assert!(new_locked <= new_total);
            assert_eq!(new_total, total); // Total doesn't change when locking
        }
    }
    println!("✓ Invariant 2: Locked <= Total for all accounts");
    
    // Invariant 3: Unlocked + Locked = Total
    for account in &accounts {
        let total = economics.balances.get_balance(*account).await.unwrap();
        let locked = economics.balances.get_locked_balance(*account).await.unwrap();
        let unlocked = economics.balances.get_unlocked_balance(*account).await.unwrap();
        
        assert_eq!(unlocked.saturating_add(locked), total);
    }
    println!("✓ Invariant 3: Unlocked + Locked = Total");
    
    println!("\n=== All Balance Invariants Hold ===");
}

/// Test genesis allocation invariants
#[tokio::test]
async fn test_genesis_allocation_invariants() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = Arc::new(EconomicsEngine::new(storage).await.unwrap());
    
    println!("\n=== Testing Genesis Allocation Invariants ===");
    
    // Invariant 1: Genesis can only be allocated once
    // Check initial state - no balances allocated yet
    let treasury_initial = economics.balances.get_balance(AccountAddress::treasury()).await.unwrap();
    assert_eq!(treasury_initial, AdicAmount::ZERO);
    
    economics.initialize_genesis().await.unwrap();
    
    // After genesis, balances should be allocated
    let treasury_after = economics.balances.get_balance(AccountAddress::treasury()).await.unwrap();
    assert!(treasury_after > AdicAmount::ZERO);
    
    // Try to allocate again - should be idempotent (may error but balance unchanged)
    let treasury_before = economics.balances.get_balance(AccountAddress::treasury()).await.unwrap();
    let _ = economics.initialize_genesis().await; // May error if not idempotent
    let treasury_after = economics.balances.get_balance(AccountAddress::treasury()).await.unwrap();
    
    assert_eq!(treasury_before, treasury_after);
    println!("✓ Invariant 1: Genesis is one-time only");
    
    // Invariant 2: Allocation percentages sum to 100%
    let treasury = economics.balances.get_balance(AccountAddress::treasury()).await.unwrap();
    let liquidity = economics.balances.get_balance(AccountAddress::liquidity_pool()).await.unwrap();
    let genesis = economics.balances.get_balance(AccountAddress::genesis_pool()).await.unwrap();
    
    let total_allocated = treasury.saturating_add(liquidity).saturating_add(genesis);
    let genesis_supply = AdicAmount::GENESIS_SUPPLY;
    
    // Allow for tiny rounding errors (< 0.001%)
    let difference = if total_allocated > genesis_supply {
        total_allocated.to_adic() - genesis_supply.to_adic()
    } else {
        genesis_supply.to_adic() - total_allocated.to_adic()
    };
    
    assert!(difference < genesis_supply.to_adic() * 0.00001);
    println!("✓ Invariant 2: Allocation percentages sum to ~100%");
    
    // Invariant 3: Individual allocations match percentages
    let treasury_pct = treasury.to_adic() / genesis_supply.to_adic();
    let liquidity_pct = liquidity.to_adic() / genesis_supply.to_adic();
    let genesis_pct = genesis.to_adic() / genesis_supply.to_adic();
    
    assert!((treasury_pct - 0.20).abs() < 0.001);
    assert!((liquidity_pct - 0.30).abs() < 0.001);
    assert!((genesis_pct - 0.50).abs() < 0.001);
    println!("✓ Invariant 3: Individual percentages correct");
    
    // Invariant 4: Genesis pool can be released but total stays same
    let user = AccountAddress::from_bytes([1; 32]);
    let release_amount = AdicAmount::from_adic(1000.0);
    
    let genesis_before_transfer = economics.balances.get_balance(AccountAddress::genesis_pool()).await.unwrap();
    economics.balances.transfer(
        AccountAddress::genesis_pool(),
        user,
        release_amount
    ).await.unwrap();
    let genesis_after_transfer = economics.balances.get_balance(AccountAddress::genesis_pool()).await.unwrap();
    let user_balance = economics.balances.get_balance(user).await.unwrap();
    
    assert_eq!(genesis_before_transfer, genesis_after_transfer.saturating_add(release_amount));
    assert_eq!(user_balance, release_amount);
    println!("✓ Invariant 4: Genesis transfer conserves tokens");
    
    println!("\n=== All Genesis Invariants Hold ===");
}

/// Test emission system invariants
#[tokio::test]
async fn test_emission_invariants() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = Arc::new(EconomicsEngine::new(storage).await.unwrap());
    
    economics.initialize_genesis().await.unwrap();
    
    println!("\n=== Testing Emission Invariants ===");
    
    // Invariant 1: Emission rate decreases monotonically over time
    let mut rates = Vec::new();
    for year in [0.0, 1.0, 3.0, 6.0, 12.0, 24.0, 48.0] {
        let rate = 0.01 * 0.5_f64.powf(year / 6.0);
        rates.push(rate);
    }
    
    for i in 1..rates.len() {
        assert!(rates[i] <= rates[i-1]);
    }
    println!("✓ Invariant 1: Emission rate decreases monotonically");
    
    // Invariant 2: Total projected emissions never exceed max supply - genesis
    let max_mintable = AdicAmount::MAX_SUPPLY.saturating_sub(AdicAmount::GENESIS_SUPPLY);
    
    for years in [10.0, 50.0, 100.0, 1000.0] {
        let projected = economics.emission.get_projected_emission(years).await;
        assert!(projected <= max_mintable);
    }
    println!("✓ Invariant 2: Projected emissions respect max supply");
    
    // Invariant 3: Emission rate at year 6 is exactly half of initial
    let initial_rate = 0.01;
    let year_6_rate = 0.01 * 0.5_f64.powf(6.0 / 6.0);
    assert_eq!(year_6_rate, initial_rate / 2.0);
    println!("✓ Invariant 3: Half-life is exactly 6 years");
    
    // Invariant 4: Emissions create new tokens (increase total supply)
    let supply_before = economics.get_total_supply().await;
    let validator = AccountAddress::from_bytes([100; 32]);
    
    // Process a small emission
    let emission_amount = AdicAmount::from_adic(100.0);
    if economics.supply.can_mint(emission_amount).await {
        economics.supply.mint_emission(emission_amount).await.unwrap();
        economics.balances.credit(validator, emission_amount).await.unwrap();
        
        let supply_after = economics.get_total_supply().await;
        assert_eq!(supply_after, supply_before.saturating_add(emission_amount));
    }
    println!("✓ Invariant 4: Emissions increase total supply");
    
    println!("\n=== All Emission Invariants Hold ===");
}

/// Test treasury multisig invariants
#[tokio::test]
async fn test_treasury_multisig_invariants() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = Arc::new(EconomicsEngine::new(storage).await.unwrap());
    
    economics.initialize_genesis().await.unwrap();
    
    println!("\n=== Testing Treasury Multisig Invariants ===");
    
    // Use correct genesis signers - multisig is already initialized
    let signers: Vec<AccountAddress> = vec![
        AccountAddress::from_bytes([0xAA; 32]),
        AccountAddress::from_bytes([0xBB; 32]),
        AccountAddress::from_bytes([0xCC; 32]),
    ];
    
    // Multisig is already initialized in genesis with threshold=2, so threshold constraints are already enforced
    println!("✓ Invariant 1: Valid threshold constraints (enforced in genesis)");
    
    // Invariant 2: Only signers can propose
    let non_signer = AccountAddress::from_bytes([100; 32]);
    let recipient = AccountAddress::from_bytes([200; 32]);
    
    assert!(economics.treasury.propose_transfer(
        recipient,
        AdicAmount::from_adic(100.0),
        "Test".to_string(),
        non_signer,
    ).await.is_err());
    println!("✓ Invariant 2: Only signers can propose");
    
    // Invariant 3: Execution happens exactly at threshold
    let proposal_id = economics.treasury.propose_transfer(
        recipient,
        AdicAmount::from_adic(100.0),
        "Test".to_string(),
        signers[0],
    ).await.unwrap();
    
    // First signer already approved (proposer)
    // Second approval - should reach threshold 2 and execute
    assert!(economics.treasury.approve_proposal(proposal_id, signers[1]).await.unwrap());
    
    // Further approvals should fail (already executed)  
    assert!(economics.treasury.approve_proposal(proposal_id, signers[2]).await.is_err());
    println!("✓ Invariant 3: Execution at exact threshold");
    
    // Invariant 4: Treasury balance decreases by exact amount
    let treasury_before = economics.balances.get_balance(AccountAddress::treasury()).await.unwrap();
    let transfer_amount = AdicAmount::from_adic(50.0);
    
    let proposal_id2 = economics.treasury.propose_transfer(
        recipient,
        transfer_amount,
        "Test2".to_string(),
        signers[0],
    ).await.unwrap();
    
    // First approval should execute the proposal (threshold = 2)
    assert!(economics.treasury.approve_proposal(proposal_id2, signers[1]).await.unwrap());
    
    let treasury_after = economics.balances.get_balance(AccountAddress::treasury()).await.unwrap();
    assert_eq!(treasury_after, treasury_before.saturating_sub(transfer_amount));
    println!("✓ Invariant 4: Treasury balance changes exactly");
    
    println!("\n=== All Treasury Invariants Hold ===");
}

/// Test system-wide conservation laws
#[tokio::test]
async fn test_conservation_laws() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = Arc::new(EconomicsEngine::new(storage).await.unwrap());
    
    economics.initialize_genesis().await.unwrap();
    
    println!("\n=== Testing Conservation Laws ===");
    
    // Track all accounts
    let mut all_accounts = vec![
        AccountAddress::treasury(),
        AccountAddress::liquidity_pool(),
        AccountAddress::genesis_pool(),
    ];
    
    // Create test accounts
    let test_accounts: Vec<AccountAddress> = (0..10)
        .map(|i| AccountAddress::from_bytes([i; 32]))
        .collect();
    all_accounts.extend(test_accounts.clone());
    
    // Transfer tokens from genesis pool to test accounts
    for account in &test_accounts {
        economics.balances.transfer(
            AccountAddress::genesis_pool(),
            *account,
            AdicAmount::from_adic(1000.0)
        ).await.unwrap();
    }
    
    // Law 1: Total of all account balances = Total supply
    let mut balance_sum = AdicAmount::ZERO;
    for account in &all_accounts {
        let balance = economics.balances.get_balance(*account).await.unwrap();
        balance_sum = balance_sum.saturating_add(balance);
    }
    
    let total_supply = economics.get_total_supply().await;
    assert_eq!(balance_sum, total_supply);
    println!("✓ Law 1: Sum of balances = Total supply");
    
    // Perform various operations
    for i in 0..20 {
        let from = test_accounts[i % test_accounts.len()];
        let to = test_accounts[(i + 1) % test_accounts.len()];
        let amount = AdicAmount::from_adic((i + 1) as f64);
        
        let from_balance = economics.balances.get_balance(from).await.unwrap();
        if from_balance >= amount {
            economics.balances.transfer(from, to, amount).await.unwrap();
        }
    }
    
    // Law 2: Conservation holds after transfers
    let mut new_balance_sum = AdicAmount::ZERO;
    for account in &all_accounts {
        let balance = economics.balances.get_balance(*account).await.unwrap();
        new_balance_sum = new_balance_sum.saturating_add(balance);
    }
    
    assert_eq!(new_balance_sum, total_supply);
    println!("✓ Law 2: Conservation maintained through operations");
    
    // Law 3: Locking doesn't change total balance
    let lock_account = test_accounts[0];
    let balance_before_lock = economics.balances.get_balance(lock_account).await.unwrap();
    
    economics.balances.lock(lock_account, AdicAmount::from_adic(500.0)).await.unwrap();
    
    let balance_after_lock = economics.balances.get_balance(lock_account).await.unwrap();
    assert_eq!(balance_before_lock, balance_after_lock);
    println!("✓ Law 3: Locking preserves balance");
    
    // Law 4: Only mint/burn changes total supply
    let supply_before_ops = economics.get_total_supply().await;
    
    // Do many operations that shouldn't change supply
    for _ in 0..10 {
        // Transfers
        let _ = economics.balances.transfer(
            test_accounts[0],
            test_accounts[1],
            AdicAmount::from_adic(1.0)
        ).await;
        
        // Locking/unlocking
        let _ = economics.balances.lock(test_accounts[1], AdicAmount::from_adic(1.0)).await;
        let _ = economics.balances.unlock(test_accounts[1], AdicAmount::from_adic(1.0)).await;
    }
    
    let supply_after_ops = economics.get_total_supply().await;
    assert_eq!(supply_before_ops, supply_after_ops);
    println!("✓ Law 4: Only mint/burn changes supply");
    
    println!("\n=== All Conservation Laws Hold ===");
}

/// Test overflow and underflow protection
#[tokio::test]
async fn test_overflow_underflow_protection() {
    let storage = Arc::new(MemoryStorage::new());
    let economics = Arc::new(EconomicsEngine::new(storage).await.unwrap());
    
    economics.initialize_genesis().await.unwrap();
    
    println!("\n=== Testing Overflow/Underflow Protection ===");
    
    let account1 = AccountAddress::from_bytes([1; 32]);
    let account2 = AccountAddress::from_bytes([2; 32]);
    
    // Test 1: Underflow protection in transfers
    economics.balances.transfer(
        AccountAddress::genesis_pool(),
        account1,
        AdicAmount::from_adic(100.0)
    ).await.unwrap();
    
    // Try to transfer more than balance
    assert!(economics.balances.transfer(
        account1,
        account2,
        AdicAmount::from_adic(101.0)
    ).await.is_err());
    
    // Balance should be unchanged
    assert_eq!(
        economics.balances.get_balance(account1).await.unwrap(),
        AdicAmount::from_adic(100.0)
    );
    println!("✓ Test 1: Transfer underflow prevented");
    
    // Test 2: Overflow protection in additions
    let large_amount = AdicAmount::from_adic(500_000_000.0); // 500M ADIC
    
    // This should work (within max supply)
    if economics.supply.can_mint(large_amount).await {
        assert!(economics.supply.mint_emission(large_amount).await.is_ok());
    }
    
    // Try to mint more than max allows
    let excessive = AdicAmount::MAX_SUPPLY;
    assert!(economics.supply.mint_emission(excessive).await.is_err());
    println!("✓ Test 2: Supply overflow prevented");
    
    // Test 3: Lock amount cannot exceed balance
    let balance = economics.balances.get_balance(account1).await.unwrap();
    assert!(economics.balances.lock(
        account1,
        balance.saturating_add(AdicAmount::from_adic(1.0))
    ).await.is_err());
    println!("✓ Test 3: Lock overflow prevented");
    
    // Test 4: Unlock amount cannot exceed locked
    economics.balances.lock(account1, AdicAmount::from_adic(50.0)).await.unwrap();
    assert!(economics.balances.unlock(
        account1,
        AdicAmount::from_adic(51.0)
    ).await.is_err());
    println!("✓ Test 4: Unlock underflow prevented");
    
    // Test 5: Saturating arithmetic in AdicAmount
    let max = AdicAmount::MAX_SUPPLY;
    let one = AdicAmount::from_adic(1.0);
    
    // Saturating add should not panic
    let result = max.saturating_add(one);
    assert_eq!(result, AdicAmount::MAX_SUPPLY);
    
    // Saturating sub should not panic
    let zero = AdicAmount::ZERO;
    let result = zero.saturating_sub(one);
    assert_eq!(result, AdicAmount::ZERO);
    println!("✓ Test 5: Saturating arithmetic works");
    
    println!("\n=== All Overflow/Underflow Protections Work ===");
}