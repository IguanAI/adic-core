pub mod address_encoding;
pub mod balance;
pub mod emission;
pub mod genesis;
pub mod storage;
pub mod supply;
pub mod treasury;
pub mod types;

pub use balance::BalanceManager;
pub use emission::{EmissionController, EmissionMetrics};
pub use genesis::{GenesisAllocation, GenesisAllocator};
pub use storage::TransactionRecord;
pub use supply::{SupplyMetrics, TokenSupply, TransferEventCallback};
pub use treasury::{TreasuryManager, TreasuryProposal};
pub use types::{AccountAddress, AdicAmount, AllocationConfig, EmissionSchedule};

// Export TokenomicsEngine as an alias to EconomicsEngine
pub type TokenomicsEngine = EconomicsEngine;

use anyhow::Result;
use std::sync::Arc;

pub struct EconomicsEngine {
    pub supply: Arc<TokenSupply>,
    pub genesis: Arc<GenesisAllocator>,
    pub emission: Arc<EmissionController>,
    pub balances: Arc<BalanceManager>,
    pub treasury: Arc<TreasuryManager>,
    storage: Arc<dyn storage::EconomicsStorage>,
}

impl EconomicsEngine {
    // Constructor with AdicParams for compatibility
    pub fn new_with_params(_params: adic_types::AdicParams) -> Self {
        // Create a simple in-memory storage for testing
        let storage: Arc<dyn storage::EconomicsStorage> = Arc::new(storage::MemoryStorage::new());
        let supply = Arc::new(TokenSupply::new());
        let balances = Arc::new(BalanceManager::new(storage.clone()));
        let treasury = Arc::new(TreasuryManager::new(balances.clone()));
        let emission = Arc::new(EmissionController::new(supply.clone(), balances.clone()));

        let genesis_config = AllocationConfig {
            treasury_percent: 0.20,
            liquidity_percent: 0.30,
            genesis_percent: 0.50,
            treasury_multisig_threshold: 2,
            treasury_multisig_keys: vec![
                AccountAddress::from_bytes([0xAA; 32]),
                AccountAddress::from_bytes([0xBB; 32]),
                AccountAddress::from_bytes([0xCC; 32]),
            ],
        };

        let genesis = Arc::new(
            GenesisAllocator::new(supply.clone(), balances.clone(), treasury.clone())
                .with_config(genesis_config),
        );

        Self {
            supply,
            genesis,
            emission,
            balances,
            treasury,
            storage,
        }
    }
}

impl EconomicsEngine {
    pub async fn new(storage: Arc<dyn storage::EconomicsStorage>) -> Result<Self> {
        let supply = Arc::new(TokenSupply::new());
        let balances = Arc::new(BalanceManager::new(storage.clone()));
        let treasury = Arc::new(TreasuryManager::new(balances.clone()));
        let emission = Arc::new(EmissionController::new(supply.clone(), balances.clone()));
        // Set up default multisig keys for genesis
        let genesis_config = AllocationConfig {
            treasury_percent: 0.20,
            liquidity_percent: 0.30,
            genesis_percent: 0.50,
            treasury_multisig_threshold: 2,
            treasury_multisig_keys: vec![
                AccountAddress::from_bytes([0xAA; 32]),
                AccountAddress::from_bytes([0xBB; 32]),
                AccountAddress::from_bytes([0xCC; 32]),
            ],
        };

        let genesis = Arc::new(
            GenesisAllocator::new(supply.clone(), balances.clone(), treasury.clone())
                .with_config(genesis_config),
        );

        Ok(Self {
            supply,
            genesis,
            emission,
            balances,
            treasury,
            storage,
        })
    }

    pub async fn initialize_genesis(&self) -> Result<()> {
        // Check if already allocated
        if self.genesis.is_allocated().await {
            return Ok(());
        }

        // Set up default multisig keys
        let _config = AllocationConfig {
            treasury_percent: 0.20,
            liquidity_percent: 0.30,
            genesis_percent: 0.50,
            treasury_multisig_threshold: 2,
            treasury_multisig_keys: vec![
                AccountAddress::from_bytes([0xAA; 32]),
                AccountAddress::from_bytes([0xBB; 32]),
                AccountAddress::from_bytes([0xCC; 32]),
            ],
        };

        // Use the genesis allocator that was configured during initialization
        self.genesis.allocate_genesis().await
    }

    pub async fn get_total_supply(&self) -> AdicAmount {
        self.supply.get_total_supply().await
    }

    pub async fn get_circulating_supply(&self) -> AdicAmount {
        self.supply.get_circulating_supply().await
    }

    /// Get paginated transaction history for an account
    pub async fn get_transaction_history_paginated(
        &self,
        address: AccountAddress,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<(Vec<storage::TransactionRecord>, Option<String>)> {
        self.storage
            .get_transaction_history_paginated(address, limit, cursor)
            .await
    }

    /// Get full transaction history for an account
    pub async fn get_transaction_history(
        &self,
        address: AccountAddress,
    ) -> Result<Vec<storage::TransactionRecord>> {
        self.storage.get_transaction_history(address).await
    }
}
