use crate::balance::BalanceManager;
use crate::supply::TokenSupply;
use crate::types::{AccountAddress, AdicAmount, EmissionSchedule, TransferEvent, TransferReason};
use anyhow::{bail, Result};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, debug};

pub struct EmissionMetrics {
    pub total_emitted: AdicAmount,
    pub last_emission: i64,
    pub current_rate: f64,
    pub years_elapsed: f64,
}

pub struct EmissionController {
    supply: Arc<TokenSupply>,
    balances: Arc<BalanceManager>,
    schedule: Arc<RwLock<EmissionSchedule>>,
    metrics: Arc<RwLock<EmissionMetrics>>,
}

impl EmissionController {
    pub fn new(supply: Arc<TokenSupply>, balances: Arc<BalanceManager>) -> Self {
        Self {
            supply,
            balances,
            schedule: Arc::new(RwLock::new(EmissionSchedule::default())),
            metrics: Arc::new(RwLock::new(EmissionMetrics {
                total_emitted: AdicAmount::ZERO,
                last_emission: chrono::Utc::now().timestamp(),
                current_rate: 0.01, // 1% initial rate
                years_elapsed: 0.0,
            })),
        }
    }

    pub fn with_schedule(mut self, schedule: EmissionSchedule) -> Self {
        self.schedule = Arc::new(RwLock::new(schedule));
        self
    }

    fn calculate_emission_rate(&self, years_elapsed: f64, schedule: &EmissionSchedule) -> f64 {
        // Formula: emission_rate(t) = initial_rate * 0.5^(t/half_life)
        // Where t is years elapsed since genesis
        schedule.initial_rate * 0.5_f64.powf(years_elapsed / schedule.half_life_years)
    }

    fn calculate_emission_amount(
        &self,
        current_supply: AdicAmount,
        rate: f64,
        time_delta_seconds: f64,
    ) -> AdicAmount {
        // Calculate emission for the time period
        // annual_emission = current_supply * rate
        // emission_per_second = annual_emission / (365.25 * 24 * 3600)
        let seconds_per_year = 365.25 * 24.0 * 3600.0;
        let emission_per_second = (current_supply.to_adic() * rate) / seconds_per_year;
        let emission_amount = emission_per_second * time_delta_seconds;
        
        AdicAmount::from_adic(emission_amount)
    }

    pub async fn process_emission(&self, recipient: AccountAddress) -> Result<AdicAmount> {
        let mut schedule = self.schedule.write().await;
        let mut metrics = self.metrics.write().await;
        
        let now = chrono::Utc::now().timestamp();
        let time_since_last = now - schedule.last_emission_timestamp;
        
        if time_since_last < 60 {
            // Minimum 60 seconds between emissions to prevent spam
            bail!("Emission too frequent. Wait {} seconds", 60 - time_since_last);
        }
        
        // Calculate years elapsed since start
        let years_elapsed = (now - schedule.start_timestamp) as f64 / (365.25 * 24.0 * 3600.0);
        
        // Calculate current emission rate with decay
        let current_rate = self.calculate_emission_rate(years_elapsed, &*schedule);
        
        // Get current supply to calculate emission
        let current_supply = self.supply.get_total_supply().await;
        
        // Calculate emission amount
        let emission_amount = self.calculate_emission_amount(
            current_supply,
            current_rate,
            time_since_last as f64,
        );
        
        // Check if we can mint this amount
        if !self.supply.can_mint(emission_amount).await {
            let remaining = self.supply.remaining_mintable().await;
            if remaining == AdicAmount::ZERO {
                bail!("Max supply reached. No more emissions possible");
            }
            // Mint only what's remaining
            info!("Emission limited to remaining mintable: {}", remaining);
            return self.mint_and_distribute(remaining, recipient, now, &mut schedule, &mut metrics).await;
        }
        
        self.mint_and_distribute(emission_amount, recipient, now, &mut schedule, &mut metrics).await
    }

    async fn mint_and_distribute(
        &self,
        amount: AdicAmount,
        recipient: AccountAddress,
        timestamp: i64,
        schedule: &mut EmissionSchedule,
        metrics: &mut EmissionMetrics,
    ) -> Result<AdicAmount> {
        // Mint new tokens
        self.supply.mint_emission(amount).await?;
        
        // Credit to recipient
        self.balances.credit(recipient, amount).await?;
        
        // Record transfer event
        let event = TransferEvent {
            from: AccountAddress::from_bytes([0; 32]),
            to: recipient,
            amount,
            timestamp,
            reason: TransferReason::Emission,
        };
        self.supply.add_transfer_event(event).await;
        
        // Update schedule and metrics
        schedule.last_emission_timestamp = timestamp;
        metrics.total_emitted = metrics.total_emitted.saturating_add(amount);
        metrics.last_emission = timestamp;
        metrics.current_rate = self.calculate_emission_rate(
            metrics.years_elapsed,
            schedule,
        );
        
        info!("Emission processed: {} to {}", amount, recipient);
        debug!("Current emission rate: {:.6}%/year", metrics.current_rate * 100.0);
        
        Ok(amount)
    }

    pub async fn process_pouw_reward(
        &self,
        validator: AccountAddress,
        base_reward: AdicAmount,
        sponsor_funded: bool,
    ) -> Result<AdicAmount> {
        let reward = if sponsor_funded {
            // Sponsor-funded reward (external funding)
            base_reward
        } else {
            // Treasury-matched reward
            // Check treasury balance and match up to base_reward
            let treasury_addr = AccountAddress::treasury();
            let treasury_balance = self.balances.get_balance(treasury_addr).await?;
            
            let matched_amount = if treasury_balance >= base_reward {
                base_reward
            } else {
                treasury_balance // Match what's available
            };
            
            if matched_amount > AdicAmount::ZERO {
                // Transfer from treasury to validator
                self.balances.transfer(treasury_addr, validator, matched_amount).await?;
                
                let event = TransferEvent {
                    from: treasury_addr,
                    to: validator,
                    amount: matched_amount,
                    timestamp: chrono::Utc::now().timestamp(),
                    reason: TransferReason::PoUWReward,
                };
                self.supply.add_transfer_event(event).await;
            }
            
            matched_amount
        };
        
        // Process as emission if we need to mint new tokens
        if sponsor_funded && self.supply.can_mint(reward).await {
            self.process_emission(validator).await
        } else {
            Ok(reward)
        }
    }

    pub async fn get_current_emission_rate(&self) -> f64 {
        let schedule = self.schedule.read().await;
        let now = chrono::Utc::now().timestamp();
        let years_elapsed = (now - schedule.start_timestamp) as f64 / (365.25 * 24.0 * 3600.0);
        self.calculate_emission_rate(years_elapsed, &*schedule)
    }

    pub async fn get_metrics(&self) -> EmissionMetrics {
        let metrics = self.metrics.read().await;
        EmissionMetrics {
            total_emitted: metrics.total_emitted,
            last_emission: metrics.last_emission,
            current_rate: metrics.current_rate,
            years_elapsed: metrics.years_elapsed,
        }
    }

    pub async fn get_projected_emission(&self, years: f64) -> AdicAmount {
        let schedule = self.schedule.read().await;
        let current_supply = self.supply.get_total_supply().await;
        let mut total_emission = AdicAmount::ZERO;
        
        // Simulate emission over the given years with monthly granularity
        let months = (years * 12.0) as u32;
        let seconds_per_month = 30.44 * 24.0 * 3600.0;
        
        for month in 0..months {
            let years_at_month = (month as f64) / 12.0;
            let rate = self.calculate_emission_rate(years_at_month, &*schedule);
            let month_emission = self.calculate_emission_amount(
                current_supply.saturating_add(total_emission),
                rate,
                seconds_per_month,
            );
            
            total_emission = total_emission.saturating_add(month_emission);
            
            // Stop if we hit max supply
            if current_supply.saturating_add(total_emission) >= AdicAmount::MAX_SUPPLY {
                break;
            }
        }
        
        total_emission
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;

    #[tokio::test]
    async fn test_emission_decay() {
        let storage = Arc::new(MemoryStorage::new());
        let supply = Arc::new(TokenSupply::new());
        let balances = Arc::new(BalanceManager::new(storage));
        
        let controller = EmissionController::new(supply.clone(), balances.clone());
        
        // Test decay calculation
        let schedule = EmissionSchedule::default();
        
        // Year 0: full rate (1%)
        let rate_0 = controller.calculate_emission_rate(0.0, &schedule);
        assert_eq!(rate_0, 0.01);
        
        // Year 6: half rate (0.5%)
        let rate_6 = controller.calculate_emission_rate(6.0, &schedule);
        assert!((rate_6 - 0.005).abs() < 0.0001);
        
        // Year 12: quarter rate (0.25%)
        let rate_12 = controller.calculate_emission_rate(12.0, &schedule);
        assert!((rate_12 - 0.0025).abs() < 0.0001);
    }

    #[tokio::test]
    async fn test_emission_amount_calculation() {
        let storage = Arc::new(MemoryStorage::new());
        let supply = Arc::new(TokenSupply::new());
        let balances = Arc::new(BalanceManager::new(storage));
        
        let controller = EmissionController::new(supply, balances);
        
        // Test emission calculation
        let current_supply = AdicAmount::from_adic(1_000_000.0);
        let rate = 0.01; // 1% per year
        let time_seconds = 365.25 * 24.0 * 3600.0; // 1 year
        
        let emission = controller.calculate_emission_amount(current_supply, rate, time_seconds);
        let expected = AdicAmount::from_adic(10_000.0); // 1% of 1M
        
        // Allow small rounding difference
        assert!((emission.to_adic() - expected.to_adic()).abs() < 1.0);
    }

    #[tokio::test]
    async fn test_max_supply_limit() {
        let storage = Arc::new(MemoryStorage::new());
        let supply = Arc::new(TokenSupply::new());
        let balances = Arc::new(BalanceManager::new(storage));
        
        // Mint genesis first
        supply.mint_genesis(AdicAmount::GENESIS_SUPPLY).await.unwrap();
        
        let controller = EmissionController::new(supply.clone(), balances);
        
        // Project emissions for 100 years
        let projected = controller.get_projected_emission(100.0).await;
        
        // Total should not exceed max supply
        let total = AdicAmount::GENESIS_SUPPLY.saturating_add(projected);
        assert!(total <= AdicAmount::MAX_SUPPLY);
    }
}