pub mod supply;
pub mod genesis;
pub mod emission;
pub mod balance;
pub mod treasury;
pub mod storage;
pub mod types;

pub use supply::{TokenSupply, SupplyMetrics};
pub use genesis::{GenesisAllocator, GenesisAllocation};
pub use emission::{EmissionController, EmissionMetrics};
pub use balance::BalanceManager;
pub use treasury::{TreasuryManager, TreasuryProposal};
pub use types::{AdicAmount, AccountAddress, AllocationConfig, EmissionSchedule};

use anyhow::Result;
use std::sync::Arc;

pub struct EconomicsEngine {
    pub supply: Arc<TokenSupply>,
    pub genesis: Arc<GenesisAllocator>,
    pub emission: Arc<EmissionController>,
    pub balances: Arc<BalanceManager>,
    pub treasury: Arc<TreasuryManager>,
}

impl EconomicsEngine {
    pub async fn new(storage: Arc<dyn storage::EconomicsStorage>) -> Result<Self> {
        let supply = Arc::new(TokenSupply::new());
        let balances = Arc::new(BalanceManager::new(storage.clone()));
        let treasury = Arc::new(TreasuryManager::new(balances.clone()));
        let emission = Arc::new(EmissionController::new(supply.clone(), balances.clone()));
        let genesis = Arc::new(GenesisAllocator::new(
            supply.clone(),
            balances.clone(),
            treasury.clone(),
        ));

        Ok(Self {
            supply,
            genesis,
            emission,
            balances,
            treasury,
        })
    }

    pub async fn initialize_genesis(&self) -> Result<()> {
        // Check if already allocated
        if self.genesis.is_allocated().await {
            return Ok(());
        }
        
        // Set up default multisig keys
        let config = AllocationConfig {
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
        
        // Use the existing genesis allocator with config
        let allocator = GenesisAllocator::new(
            self.supply.clone(),
            self.balances.clone(),
            self.treasury.clone(),
        ).with_config(config);
        
        allocator.allocate_genesis().await
    }

    pub async fn get_total_supply(&self) -> AdicAmount {
        self.supply.get_total_supply().await
    }

    pub async fn get_circulating_supply(&self) -> AdicAmount {
        self.supply.get_circulating_supply().await
    }
}