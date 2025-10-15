use crate::{AppError, LockId, Result};
use adic_economics::types::{AccountAddress, AdicAmount};
use adic_economics::BalanceManager;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Types of escrow locks used across applications
#[derive(Debug, Clone)]
pub enum EscrowType {
    /// Storage market provider collateral
    StorageCollateral {
        deal_id: u64,
        provider: AccountAddress,
    },

    /// Storage market client payment escrow
    StoragePayment {
        deal_id: u64,
        client: AccountAddress,
    },

    /// PoUW worker deposit
    PoUWDeposit {
        hook_id: adic_types::MessageId,
        worker: AccountAddress,
    },

    /// Treasury grant escrow
    TreasuryGrant {
        proposal_id: adic_types::MessageId,
        recipient: AccountAddress,
    },

    /// Generic escrow with custom lock ID
    Custom {
        lock_id: String,
        owner: AccountAddress,
    },
}

impl EscrowType {
    pub fn to_lock_id(&self) -> LockId {
        match self {
            EscrowType::StorageCollateral { deal_id, provider } => LockId::new(format!(
                "storage_collateral_{}_{}",
                deal_id,
                hex::encode(&provider.as_bytes()[..8])
            )),
            EscrowType::StoragePayment { deal_id, client } => LockId::new(format!(
                "storage_payment_{}_{}",
                deal_id,
                hex::encode(&client.as_bytes()[..8])
            )),
            EscrowType::PoUWDeposit { hook_id, worker } => LockId::new(format!(
                "pouw_deposit_{}_{}",
                hex::encode(hook_id.as_bytes()),
                hex::encode(&worker.as_bytes()[..8])
            )),
            EscrowType::TreasuryGrant {
                proposal_id,
                recipient,
            } => LockId::new(format!(
                "treasury_grant_{}_{}",
                hex::encode(proposal_id.as_bytes()),
                hex::encode(&recipient.as_bytes()[..8])
            )),
            EscrowType::Custom { lock_id, .. } => LockId::new(lock_id.clone()),
        }
    }

    pub fn owner(&self) -> AccountAddress {
        match self {
            EscrowType::StorageCollateral { provider, .. } => *provider,
            EscrowType::StoragePayment { client, .. } => *client,
            EscrowType::PoUWDeposit { worker, .. } => *worker,
            EscrowType::TreasuryGrant { recipient, .. } => *recipient,
            EscrowType::Custom { owner, .. } => *owner,
        }
    }
}

/// Lock metadata for tracking
#[derive(Debug, Clone)]
pub struct LockMetadata {
    pub escrow_type: EscrowType,
    pub amount: AdicAmount,
    pub locked_at_epoch: u64,
    pub owner: AccountAddress,
}

/// Escrow manager wraps BalanceManager to provide application-level escrow
pub struct EscrowManager {
    balance_mgr: Arc<BalanceManager>,
    locks: Arc<RwLock<HashMap<LockId, LockMetadata>>>,
}

impl EscrowManager {
    pub fn new(balance_mgr: Arc<BalanceManager>) -> Self {
        Self {
            balance_mgr,
            locks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Lock funds for escrow
    pub async fn lock(
        &self,
        escrow_type: EscrowType,
        amount: AdicAmount,
        current_epoch: u64,
    ) -> Result<LockId> {
        let start = std::time::Instant::now();
        let lock_id = escrow_type.to_lock_id();
        let owner = escrow_type.owner();

        // Capture balance before
        let balance_before = self
            .balance_mgr
            .get_balance(owner)
            .await
            .map_err(|e| AppError::EscrowError(e.to_string()))?;

        // Lock the funds using BalanceManager
        self.balance_mgr
            .lock(owner, amount)
            .await
            .map_err(|e| AppError::EscrowError(e.to_string()))?;

        // Capture locked balance after
        let locked_after = self
            .balance_mgr
            .get_locked_balance(owner)
            .await
            .map_err(|e| AppError::EscrowError(e.to_string()))?;

        // Track the lock
        let mut locks = self.locks.write().await;
        locks.insert(
            lock_id.clone(),
            LockMetadata {
                escrow_type: escrow_type.clone(),
                amount,
                locked_at_epoch: current_epoch,
                owner,
            },
        );

        info!(
            lock_id = %lock_id,
            owner = %owner,
            amount_adic = amount.to_adic(),
            balance_before_adic = balance_before.to_adic(),
            locked_after_adic = locked_after.to_adic(),
            escrow_type = ?escrow_type,
            epoch = current_epoch,
            duration_ms = start.elapsed().as_millis() as u64,
            "💰 Escrow locked"
        );

        Ok(lock_id)
    }

    /// Release locked funds to a recipient
    pub async fn release(&self, lock_id: &LockId, to: AccountAddress) -> Result<()> {
        let start = std::time::Instant::now();
        let locks = self.locks.read().await;
        let metadata = locks
            .get(lock_id)
            .ok_or_else(|| AppError::LockNotFound(lock_id.to_string()))?;

        // Capture balances before
        let from_balance_before = self
            .balance_mgr
            .get_balance(metadata.owner)
            .await
            .map_err(|e| AppError::EscrowError(e.to_string()))?;
        let to_balance_before = self
            .balance_mgr
            .get_balance(to)
            .await
            .map_err(|e| AppError::EscrowError(e.to_string()))?;

        // Unlock funds from owner
        self.balance_mgr
            .unlock(metadata.owner, metadata.amount)
            .await
            .map_err(|e| AppError::EscrowError(e.to_string()))?;

        // Transfer to recipient
        self.balance_mgr
            .transfer(metadata.owner, to, metadata.amount)
            .await
            .map_err(|e| AppError::EscrowError(e.to_string()))?;

        // Capture balances after
        let to_balance_after = self
            .balance_mgr
            .get_balance(to)
            .await
            .map_err(|e| AppError::EscrowError(e.to_string()))?;

        info!(
            lock_id = %lock_id,
            from = %metadata.owner,
            to = %to,
            amount_adic = metadata.amount.to_adic(),
            from_balance_before_adic = from_balance_before.to_adic(),
            to_balance_before_adic = to_balance_before.to_adic(),
            to_balance_after_adic = to_balance_after.to_adic(),
            duration_ms = start.elapsed().as_millis() as u64,
            "💸 Escrow released"
        );

        Ok(())
    }

    /// Slash locked funds (send to treasury)
    pub async fn slash(&self, lock_id: &LockId, treasury: AccountAddress) -> Result<()> {
        let start = std::time::Instant::now();
        let locks = self.locks.read().await;
        let metadata = locks
            .get(lock_id)
            .ok_or_else(|| AppError::LockNotFound(lock_id.to_string()))?;

        // Capture balances before
        let treasury_balance_before = self
            .balance_mgr
            .get_balance(treasury)
            .await
            .map_err(|e| AppError::EscrowError(e.to_string()))?;

        // Unlock funds
        self.balance_mgr
            .unlock(metadata.owner, metadata.amount)
            .await
            .map_err(|e| AppError::EscrowError(e.to_string()))?;

        // Transfer to treasury
        self.balance_mgr
            .transfer(metadata.owner, treasury, metadata.amount)
            .await
            .map_err(|e| AppError::EscrowError(e.to_string()))?;

        // Capture treasury balance after
        let treasury_balance_after = self
            .balance_mgr
            .get_balance(treasury)
            .await
            .map_err(|e| AppError::EscrowError(e.to_string()))?;

        info!(
            lock_id = %lock_id,
            owner = %metadata.owner,
            treasury = %treasury,
            amount_adic = metadata.amount.to_adic(),
            treasury_balance_before_adic = treasury_balance_before.to_adic(),
            treasury_balance_after_adic = treasury_balance_after.to_adic(),
            duration_ms = start.elapsed().as_millis() as u64,
            "⚔️ Escrow slashed"
        );

        Ok(())
    }

    /// Refund locked funds back to owner
    pub async fn refund(&self, lock_id: &LockId) -> Result<()> {
        let start = std::time::Instant::now();
        let locks = self.locks.read().await;
        let metadata = locks
            .get(lock_id)
            .ok_or_else(|| AppError::LockNotFound(lock_id.to_string()))?;

        // Capture locked balance before
        let locked_before = self
            .balance_mgr
            .get_locked_balance(metadata.owner)
            .await
            .map_err(|e| AppError::EscrowError(e.to_string()))?;

        // Simply unlock the funds (returns to owner's balance)
        self.balance_mgr
            .unlock(metadata.owner, metadata.amount)
            .await
            .map_err(|e| AppError::EscrowError(e.to_string()))?;

        // Capture locked balance after
        let locked_after = self
            .balance_mgr
            .get_locked_balance(metadata.owner)
            .await
            .map_err(|e| AppError::EscrowError(e.to_string()))?;

        info!(
            lock_id = %lock_id,
            owner = %metadata.owner,
            amount_adic = metadata.amount.to_adic(),
            locked_before_adic = locked_before.to_adic(),
            locked_after_adic = locked_after.to_adic(),
            duration_ms = start.elapsed().as_millis() as u64,
            "🔄 Escrow refunded"
        );

        Ok(())
    }

    /// Get lock metadata
    pub async fn get_lock(&self, lock_id: &LockId) -> Result<LockMetadata> {
        let locks = self.locks.read().await;
        locks
            .get(lock_id)
            .cloned()
            .ok_or_else(|| AppError::LockNotFound(lock_id.to_string()))
    }

    /// Check if a lock exists
    pub async fn lock_exists(&self, lock_id: &LockId) -> bool {
        let locks = self.locks.read().await;
        locks.contains_key(lock_id)
    }

    /// Remove lock from tracking (after funds are released/slashed)
    pub async fn remove_lock(&self, lock_id: &LockId) -> Result<()> {
        let mut locks = self.locks.write().await;
        locks
            .remove(lock_id)
            .ok_or_else(|| AppError::LockNotFound(lock_id.to_string()))?;

        debug!(lock_id = %lock_id, "Lock removed from tracking");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_economics::storage::MemoryStorage;

    #[tokio::test]
    async fn test_escrow_lifecycle() {
        let storage = Arc::new(MemoryStorage::new());
        let balance_mgr = Arc::new(BalanceManager::new(storage));
        let escrow_mgr = EscrowManager::new(balance_mgr.clone());

        let owner = AccountAddress::from_bytes([1; 32]);
        let recipient = AccountAddress::from_bytes([2; 32]);

        // Credit owner with funds
        balance_mgr
            .credit(owner, AdicAmount::from_adic(100.0))
            .await
            .unwrap();

        // Lock funds
        let escrow_type = EscrowType::Custom {
            lock_id: "test_lock".to_string(),
            owner,
        };
        let lock_id = escrow_mgr
            .lock(escrow_type, AdicAmount::from_adic(50.0), 1)
            .await
            .unwrap();

        // Verify locked balance
        let locked = balance_mgr.get_locked_balance(owner).await.unwrap();
        assert_eq!(locked, AdicAmount::from_adic(50.0));

        // Release funds
        escrow_mgr.release(&lock_id, recipient).await.unwrap();

        // Verify recipient got funds
        let recipient_balance = balance_mgr.get_balance(recipient).await.unwrap();
        assert_eq!(recipient_balance, AdicAmount::from_adic(50.0));
    }
}
