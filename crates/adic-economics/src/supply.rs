use crate::types::{AdicAmount, TransferEvent};
use anyhow::{bail, Result};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Callback for transfer events
pub type TransferEventCallback = Arc<dyn Fn(TransferEvent) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct SupplyMetrics {
    pub total_supply: AdicAmount,
    pub circulating_supply: AdicAmount,
    pub treasury_balance: AdicAmount,
    pub liquidity_balance: AdicAmount,
    pub genesis_balance: AdicAmount,
    pub burned_amount: AdicAmount,
    pub emission_issued: AdicAmount,
}

impl Default for SupplyMetrics {
    fn default() -> Self {
        Self {
            total_supply: AdicAmount::ZERO,
            circulating_supply: AdicAmount::ZERO,
            treasury_balance: AdicAmount::ZERO,
            liquidity_balance: AdicAmount::ZERO,
            genesis_balance: AdicAmount::ZERO,
            burned_amount: AdicAmount::ZERO,
            emission_issued: AdicAmount::ZERO,
        }
    }
}

pub struct TokenSupply {
    metrics: Arc<RwLock<SupplyMetrics>>,
    transfer_history: Arc<RwLock<Vec<TransferEvent>>>,
    event_callback: Arc<RwLock<Option<TransferEventCallback>>>,
}

impl Default for TokenSupply {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenSupply {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(RwLock::new(SupplyMetrics::default())),
            transfer_history: Arc::new(RwLock::new(Vec::new())),
            event_callback: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn mint_genesis(&self, amount: AdicAmount) -> Result<()> {
        let mut metrics = self.metrics.write().await;

        if metrics.total_supply != AdicAmount::ZERO {
            bail!("Genesis already minted");
        }

        if amount != AdicAmount::GENESIS_SUPPLY {
            bail!(
                "Invalid genesis amount. Expected {}, got {}",
                AdicAmount::GENESIS_SUPPLY,
                amount
            );
        }

        let old_supply = metrics.total_supply;
        metrics.total_supply = amount;

        info!(
            amount = amount.to_adic(),
            total_supply_before = old_supply.to_adic(),
            total_supply_after = metrics.total_supply.to_adic(),
            "🌱 Genesis supply minted"
        );
        Ok(())
    }

    pub async fn mint_emission(&self, amount: AdicAmount) -> Result<()> {
        let mut metrics = self.metrics.write().await;

        let old_total = metrics.total_supply;
        let old_emission = metrics.emission_issued;
        let old_circulating = metrics.circulating_supply;

        let new_supply = metrics
            .total_supply
            .checked_add(amount)
            .ok_or_else(|| anyhow::anyhow!("Supply overflow"))?;

        if new_supply > AdicAmount::MAX_SUPPLY {
            bail!(
                "Cannot mint: would exceed max supply of {}",
                AdicAmount::MAX_SUPPLY
            );
        }

        metrics.total_supply = new_supply;
        metrics.emission_issued = metrics.emission_issued.saturating_add(amount);
        metrics.circulating_supply = metrics.circulating_supply.saturating_add(amount);

        info!(
            amount = amount.to_adic(),
            total_supply_before = old_total.to_adic(),
            total_supply_after = new_supply.to_adic(),
            emission_issued_before = old_emission.to_adic(),
            emission_issued_after = metrics.emission_issued.to_adic(),
            circulating_before = old_circulating.to_adic(),
            circulating_after = metrics.circulating_supply.to_adic(),
            "💰 Emission minted"
        );
        Ok(())
    }

    pub async fn burn(&self, amount: AdicAmount) -> Result<()> {
        let mut metrics = self.metrics.write().await;

        if amount > metrics.circulating_supply {
            bail!("Cannot burn more than circulating supply");
        }

        let old_circulating = metrics.circulating_supply;
        let old_burned = metrics.burned_amount;

        metrics.circulating_supply = metrics.circulating_supply.saturating_sub(amount);
        metrics.burned_amount = metrics.burned_amount.saturating_add(amount);

        info!(
            amount = amount.to_adic(),
            circulating_before = old_circulating.to_adic(),
            circulating_after = metrics.circulating_supply.to_adic(),
            burned_before = old_burned.to_adic(),
            burned_after = metrics.burned_amount.to_adic(),
            "🔥 Tokens burned"
        );
        Ok(())
    }

    pub async fn update_treasury_balance(&self, balance: AdicAmount) {
        let mut metrics = self.metrics.write().await;
        let old_balance = metrics.treasury_balance;
        metrics.treasury_balance = balance;

        if old_balance != balance {
            info!(
                balance_before = old_balance.to_adic(),
                balance_after = balance.to_adic(),
                change = (balance.to_adic() - old_balance.to_adic()),
                "🏛️ Treasury balance updated"
            );
        }
    }

    pub async fn update_liquidity_balance(&self, balance: AdicAmount) {
        let mut metrics = self.metrics.write().await;
        let old_balance = metrics.liquidity_balance;
        metrics.liquidity_balance = balance;

        if old_balance != balance {
            info!(
                balance_before = old_balance.to_adic(),
                balance_after = balance.to_adic(),
                change = (balance.to_adic() - old_balance.to_adic()),
                "💧 Liquidity balance updated"
            );
        }
    }

    pub async fn update_genesis_balance(&self, balance: AdicAmount) {
        let mut metrics = self.metrics.write().await;
        let old_balance = metrics.genesis_balance;
        metrics.genesis_balance = balance;

        if old_balance != balance {
            info!(
                balance_before = old_balance.to_adic(),
                balance_after = balance.to_adic(),
                change = (balance.to_adic() - old_balance.to_adic()),
                "🌟 Genesis balance updated"
            );
        }
    }

    pub async fn update_circulating_supply(&self, amount: AdicAmount) {
        let mut metrics = self.metrics.write().await;
        let old_supply = metrics.circulating_supply;
        metrics.circulating_supply = amount;

        if old_supply != amount {
            info!(
                supply_before = old_supply.to_adic(),
                supply_after = amount.to_adic(),
                change = (amount.to_adic() - old_supply.to_adic()),
                "🔄 Circulating supply updated"
            );
        }
    }

    pub async fn get_total_supply(&self) -> AdicAmount {
        let metrics = self.metrics.read().await;
        metrics.total_supply
    }

    pub async fn get_circulating_supply(&self) -> AdicAmount {
        let metrics = self.metrics.read().await;
        metrics.circulating_supply
    }

    pub async fn get_metrics(&self) -> SupplyMetrics {
        let metrics = self.metrics.read().await;
        metrics.clone()
    }

    /// Set the event callback for transfer events
    pub async fn set_event_callback(&self, callback: TransferEventCallback) {
        let mut cb = self.event_callback.write().await;
        *cb = Some(callback);
    }

    /// Emit a transfer event if callback is set
    fn emit_transfer_event(&self, event: TransferEvent) {
        let callback_clone = self.event_callback.clone();
        tokio::spawn(async move {
            let callback_guard = callback_clone.read().await;
            if let Some(callback) = callback_guard.as_ref() {
                callback(event);
            }
        });
    }

    pub async fn add_transfer_event(&self, event: TransferEvent) {
        let mut history = self.transfer_history.write().await;
        let old_count = history.len();
        history.push(event.clone());

        // Keep only last 10000 events to prevent unbounded growth
        let pruned = if history.len() > 10000 {
            history.drain(0..1000);
            1000
        } else {
            0
        };

        info!(
            from = %event.from,
            to = %event.to,
            amount = event.amount.to_adic(),
            reason = ?event.reason,
            history_size_before = old_count,
            history_size_after = history.len(),
            events_pruned = pruned,
            "📜 Transfer event recorded"
        );

        // Emit event to node event bus via callback
        self.emit_transfer_event(event);
    }

    pub async fn get_transfer_history(&self, limit: usize) -> Vec<TransferEvent> {
        let history = self.transfer_history.read().await;
        let start = history.len().saturating_sub(limit);
        history[start..].to_vec()
    }

    pub async fn can_mint(&self, amount: AdicAmount) -> bool {
        let metrics = self.metrics.read().await;
        if let Some(new_supply) = metrics.total_supply.checked_add(amount) {
            new_supply <= AdicAmount::MAX_SUPPLY
        } else {
            false
        }
    }

    pub async fn remaining_mintable(&self) -> AdicAmount {
        let metrics = self.metrics.read().await;
        AdicAmount::MAX_SUPPLY.saturating_sub(metrics.total_supply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_genesis_mint() {
        let supply = TokenSupply::new();

        // Genesis mint should work
        assert!(supply
            .mint_genesis(AdicAmount::GENESIS_SUPPLY)
            .await
            .is_ok());
        assert_eq!(supply.get_total_supply().await, AdicAmount::GENESIS_SUPPLY);

        // Second genesis mint should fail
        assert!(supply
            .mint_genesis(AdicAmount::GENESIS_SUPPLY)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn test_max_supply_enforcement() {
        let supply = TokenSupply::new();
        supply
            .mint_genesis(AdicAmount::GENESIS_SUPPLY)
            .await
            .unwrap();

        // Should not be able to mint more than max supply
        let remaining = supply.remaining_mintable().await;
        assert!(supply.mint_emission(remaining).await.is_ok());

        // Any additional mint should fail
        assert!(supply
            .mint_emission(AdicAmount::from_adic(1.0))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn test_burn() {
        let supply = TokenSupply::new();
        supply
            .mint_genesis(AdicAmount::GENESIS_SUPPLY)
            .await
            .unwrap();
        supply
            .update_circulating_supply(AdicAmount::from_adic(1000.0))
            .await;

        // Burn should work
        assert!(supply.burn(AdicAmount::from_adic(100.0)).await.is_ok());
        assert_eq!(
            supply.get_circulating_supply().await,
            AdicAmount::from_adic(900.0)
        );

        // Cannot burn more than circulating
        assert!(supply.burn(AdicAmount::from_adic(1000.0)).await.is_err());
    }
}
