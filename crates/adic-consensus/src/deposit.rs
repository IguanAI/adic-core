use adic_types::{MessageId, PublicKey, Result, AdicError};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq)]
pub enum DepositState {
    Escrowed,
    Refunded,
    Slashed,
}

#[derive(Debug, Clone)]
pub struct Deposit {
    pub message_id: MessageId,
    pub proposer: PublicKey,
    pub amount: f64,
    pub state: DepositState,
    pub timestamp: i64,
}

pub struct DepositManager {
    deposit_amount: f64,
    deposits: Arc<RwLock<HashMap<MessageId, Deposit>>>,
}

impl DepositManager {
    pub fn new(deposit_amount: f64) -> Self {
        Self {
            deposit_amount,
            deposits: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn escrow(&self, message_id: MessageId, proposer: PublicKey) -> Result<()> {
        let deposit = Deposit {
            message_id,
            proposer,
            amount: self.deposit_amount,
            state: DepositState::Escrowed,
            timestamp: chrono::Utc::now().timestamp(),
        };

        let mut deposits = self.deposits.write().await;
        deposits.insert(message_id, deposit);
        Ok(())
    }

    pub async fn refund(&self, message_id: &MessageId) -> Result<()> {
        let mut deposits = self.deposits.write().await;
        
        match deposits.get_mut(message_id) {
            Some(deposit) => {
                if deposit.state == DepositState::Escrowed {
                    deposit.state = DepositState::Refunded;
                    Ok(())
                } else {
                    Err(AdicError::InvalidParameter(format!(
                        "Deposit for {} is not in escrowed state",
                        message_id
                    )))
                }
            }
            None => Err(AdicError::InvalidParameter(format!(
                "No deposit found for message {}",
                message_id
            ))),
        }
    }

    pub async fn slash(&self, message_id: &MessageId, reason: &str) -> Result<()> {
        let mut deposits = self.deposits.write().await;
        
        match deposits.get_mut(message_id) {
            Some(deposit) => {
                if deposit.state == DepositState::Escrowed {
                    deposit.state = DepositState::Slashed;
                    tracing::warn!("Slashed deposit for {}: {}", message_id, reason);
                    Ok(())
                } else {
                    Err(AdicError::InvalidParameter(format!(
                        "Deposit for {} is not in escrowed state",
                        message_id
                    )))
                }
            }
            None => Err(AdicError::InvalidParameter(format!(
                "No deposit found for message {}",
                message_id
            ))),
        }
    }

    pub async fn get_state(&self, message_id: &MessageId) -> Option<DepositState> {
        let deposits = self.deposits.read().await;
        deposits.get(message_id).map(|d| d.state.clone())
    }

    pub async fn get_proposer(&self, message_id: &MessageId) -> Option<PublicKey> {
        let deposits = self.deposits.read().await;
        deposits.get(message_id).map(|d| d.proposer)
    }

    pub async fn get_escrowed_count(&self) -> usize {
        let deposits = self.deposits.read().await;
        deposits
            .values()
            .filter(|d| d.state == DepositState::Escrowed)
            .count()
    }

    pub async fn get_total_escrowed(&self) -> f64 {
        let deposits = self.deposits.read().await;
        deposits
            .values()
            .filter(|d| d.state == DepositState::Escrowed)
            .map(|d| d.amount)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_deposit_lifecycle() {
        let manager = DepositManager::new(0.1);
        let msg_id = MessageId::new(b"test");
        let proposer = PublicKey::from_bytes([1; 32]);

        manager.escrow(msg_id, proposer).await.unwrap();
        assert_eq!(manager.get_state(&msg_id).await, Some(DepositState::Escrowed));

        manager.refund(&msg_id).await.unwrap();
        assert_eq!(manager.get_state(&msg_id).await, Some(DepositState::Refunded));
    }

    #[tokio::test]
    async fn test_slash_deposit() {
        let manager = DepositManager::new(0.1);
        let msg_id = MessageId::new(b"test");
        let proposer = PublicKey::from_bytes([1; 32]);

        manager.escrow(msg_id, proposer).await.unwrap();
        manager.slash(&msg_id, "Invalid signature").await.unwrap();
        assert_eq!(manager.get_state(&msg_id).await, Some(DepositState::Slashed));
    }
}