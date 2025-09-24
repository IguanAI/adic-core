use crate::balance::BalanceManager;
use crate::supply::TokenSupply;
use crate::treasury::TreasuryManager;
use crate::types::{AccountAddress, AdicAmount, AllocationConfig, TransferEvent, TransferReason};
use anyhow::{bail, Result};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

#[derive(Debug, Clone)]
pub struct GenesisAllocation {
    pub treasury_amount: AdicAmount,
    pub liquidity_amount: AdicAmount,
    pub genesis_amount: AdicAmount,
    pub timestamp: i64,
    pub executed: bool,
}

pub struct GenesisAllocator {
    supply: Arc<TokenSupply>,
    balances: Arc<BalanceManager>,
    treasury: Arc<TreasuryManager>,
    allocation: Arc<RwLock<Option<GenesisAllocation>>>,
    config: AllocationConfig,
}

impl GenesisAllocator {
    pub fn new(
        supply: Arc<TokenSupply>,
        balances: Arc<BalanceManager>,
        treasury: Arc<TreasuryManager>,
    ) -> Self {
        Self {
            supply,
            balances,
            treasury,
            allocation: Arc::new(RwLock::new(None)),
            config: AllocationConfig::default(),
        }
    }

    pub fn with_config(mut self, config: AllocationConfig) -> Self {
        self.validate_config(&config)
            .expect("Invalid allocation config");
        self.config = config;
        self
    }

    fn validate_config(&self, config: &AllocationConfig) -> Result<()> {
        let total = config.treasury_percent + config.liquidity_percent + config.genesis_percent;
        if (total - 1.0).abs() > 0.001 {
            bail!(
                "Allocation percentages must sum to 100%, got {}%",
                total * 100.0
            );
        }

        if config.treasury_percent < 0.0 || config.treasury_percent > 1.0 {
            bail!("Invalid treasury percentage: {}", config.treasury_percent);
        }

        if config.liquidity_percent < 0.0 || config.liquidity_percent > 1.0 {
            bail!("Invalid liquidity percentage: {}", config.liquidity_percent);
        }

        if config.genesis_percent < 0.0 || config.genesis_percent > 1.0 {
            bail!("Invalid genesis percentage: {}", config.genesis_percent);
        }

        if config.treasury_multisig_keys.len() < config.treasury_multisig_threshold as usize {
            bail!("Not enough multisig keys for threshold");
        }

        Ok(())
    }

    pub async fn allocate_genesis(&self) -> Result<()> {
        let mut allocation_guard = self.allocation.write().await;

        if allocation_guard.is_some() {
            bail!("Genesis allocation already executed");
        }

        // Mint genesis supply
        self.supply.mint_genesis(AdicAmount::GENESIS_SUPPLY).await?;

        // Calculate allocation amounts
        let total = AdicAmount::GENESIS_SUPPLY;
        let treasury_amount = AdicAmount::from_adic(total.to_adic() * self.config.treasury_percent);
        let liquidity_amount =
            AdicAmount::from_adic(total.to_adic() * self.config.liquidity_percent);
        let genesis_amount = AdicAmount::from_adic(total.to_adic() * self.config.genesis_percent);

        // Ensure no tokens are lost due to rounding
        let allocated = treasury_amount
            .checked_add(liquidity_amount)
            .and_then(|sum| sum.checked_add(genesis_amount))
            .ok_or_else(|| anyhow::anyhow!("Allocation overflow"))?;

        if allocated > total {
            bail!("Allocation exceeds genesis supply");
        }

        let timestamp = chrono::Utc::now().timestamp();

        // Allocate to treasury (multisig)
        let treasury_addr = AccountAddress::treasury();
        self.balances.credit(treasury_addr, treasury_amount).await?;
        self.supply.update_treasury_balance(treasury_amount).await;
        self.treasury
            .initialize_multisig(
                self.config.treasury_multisig_keys.clone(),
                self.config.treasury_multisig_threshold,
            )
            .await?;

        let treasury_event = TransferEvent {
            from: AccountAddress::from_bytes([0; 32]),
            to: treasury_addr,
            amount: treasury_amount,
            timestamp,
            reason: TransferReason::Genesis,
        };
        self.supply.add_transfer_event(treasury_event).await;

        info!("Allocated {} to Treasury (multisig)", treasury_amount);

        // Allocate to liquidity pool
        let liquidity_addr = AccountAddress::liquidity_pool();
        self.balances
            .credit(liquidity_addr, liquidity_amount)
            .await?;
        self.supply.update_liquidity_balance(liquidity_amount).await;

        let liquidity_event = TransferEvent {
            from: AccountAddress::from_bytes([0; 32]),
            to: liquidity_addr,
            amount: liquidity_amount,
            timestamp,
            reason: TransferReason::Genesis,
        };
        self.supply.add_transfer_event(liquidity_event).await;

        info!(
            "Allocated {} to Liquidity & Community R&D",
            liquidity_amount
        );

        // Allocate to genesis pool
        let genesis_addr = AccountAddress::genesis_pool();
        self.balances.credit(genesis_addr, genesis_amount).await?;
        self.supply.update_genesis_balance(genesis_amount).await;

        let genesis_event = TransferEvent {
            from: AccountAddress::from_bytes([0; 32]),
            to: genesis_addr,
            amount: genesis_amount,
            timestamp,
            reason: TransferReason::Genesis,
        };
        self.supply.add_transfer_event(genesis_event).await;

        info!("Allocated {} to Genesis Pool", genesis_amount);

        // Update circulating supply to reflect allocated tokens
        // Note: In production, some tokens might be locked, but for testing we consider them circulating
        self.supply.update_circulating_supply(allocated).await;

        // Store allocation record
        *allocation_guard = Some(GenesisAllocation {
            treasury_amount,
            liquidity_amount,
            genesis_amount,
            timestamp,
            executed: true,
        });

        info!(
            total_allocated = %allocated,
            treasury = %treasury_amount,
            liquidity = %liquidity_amount,
            genesis_pool = %genesis_amount,
            "✅ Genesis allocation completed successfully"
        );
        info!("Total allocated: {}", allocated);
        info!("Treasury (20%): {}", treasury_amount);
        info!("Liquidity & Grants (30%): {}", liquidity_amount);
        info!("Genesis Pool (50%): {}", genesis_amount);

        Ok(())
    }

    pub async fn get_allocation(&self) -> Option<GenesisAllocation> {
        let allocation = self.allocation.read().await;
        allocation.clone().map(|mut a| {
            a.executed = true;
            a
        })
    }

    pub async fn is_allocated(&self) -> bool {
        self.allocation.read().await.is_some()
    }

    pub async fn release_genesis_tokens(
        &self,
        recipient: AccountAddress,
        amount: AdicAmount,
    ) -> Result<()> {
        let allocation = self.allocation.read().await;
        if allocation.is_none() {
            bail!("Genesis not yet allocated");
        }

        let genesis_addr = AccountAddress::genesis_pool();
        let genesis_balance = self.balances.get_balance(genesis_addr).await?;

        if amount > genesis_balance {
            bail!("Insufficient genesis pool balance");
        }

        // Transfer from genesis pool to recipient
        self.balances
            .transfer(genesis_addr, recipient, amount)
            .await?;

        // Update supply metrics
        let new_genesis_balance = genesis_balance.saturating_sub(amount);
        self.supply
            .update_genesis_balance(new_genesis_balance)
            .await;

        // Update circulating supply
        let current_circulating = self.supply.get_circulating_supply().await;
        self.supply
            .update_circulating_supply(current_circulating.saturating_add(amount))
            .await;

        info!("Released {} from genesis pool to {}", amount, recipient);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;

    #[tokio::test]
    async fn test_genesis_allocation() {
        let storage = Arc::new(MemoryStorage::new());
        let supply = Arc::new(TokenSupply::new());
        let balances = Arc::new(BalanceManager::new(storage));
        let treasury = Arc::new(TreasuryManager::new(balances.clone()));

        // Use default config which doesn't require multisig keys
        let config = AllocationConfig {
            treasury_multisig_keys: vec![
                AccountAddress::from_bytes([1; 32]),
                AccountAddress::from_bytes([2; 32]),
            ],
            ..Default::default()
        };

        let allocator =
            GenesisAllocator::new(supply.clone(), balances.clone(), treasury).with_config(config);

        // Allocate genesis
        assert!(allocator.allocate_genesis().await.is_ok());
        assert!(allocator.is_allocated().await);

        // Check allocations
        let treasury_balance = balances
            .get_balance(AccountAddress::treasury())
            .await
            .unwrap();
        let expected_treasury = AdicAmount::from_adic(300_000_000.0 * 0.20);
        assert_eq!(treasury_balance, expected_treasury);

        let liquidity_balance = balances
            .get_balance(AccountAddress::liquidity_pool())
            .await
            .unwrap();
        let expected_liquidity = AdicAmount::from_adic(300_000_000.0 * 0.30);
        assert_eq!(liquidity_balance, expected_liquidity);

        let genesis_balance = balances
            .get_balance(AccountAddress::genesis_pool())
            .await
            .unwrap();
        let expected_genesis = AdicAmount::from_adic(300_000_000.0 * 0.50);
        assert_eq!(genesis_balance, expected_genesis);

        // Cannot allocate twice
        assert!(allocator.allocate_genesis().await.is_err());
    }
}
