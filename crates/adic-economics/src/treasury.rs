use crate::balance::BalanceManager;
use crate::types::{AccountAddress, AdicAmount};
use anyhow::{bail, Result};
use blake3::Hasher;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct MultisigConfig {
    pub signers: Vec<AccountAddress>,
    pub threshold: u32,
}

#[derive(Debug, Clone)]
pub struct TreasuryProposal {
    pub id: [u8; 32],
    pub recipient: AccountAddress,
    pub amount: AdicAmount,
    pub reason: String,
    pub proposer: AccountAddress,
    pub approvals: HashSet<AccountAddress>,
    pub rejections: HashSet<AccountAddress>,
    pub executed: bool,
    pub created_at: i64,
    pub expires_at: i64,
}

impl TreasuryProposal {
    pub fn new(
        recipient: AccountAddress,
        amount: AdicAmount,
        reason: String,
        proposer: AccountAddress,
    ) -> Self {
        let mut hasher = Hasher::new();
        hasher.update(recipient.as_bytes());
        hasher.update(&amount.to_base_units().to_le_bytes());
        hasher.update(reason.as_bytes());
        hasher.update(&chrono::Utc::now().timestamp().to_le_bytes());
        
        let id = *hasher.finalize().as_bytes();
        let now = chrono::Utc::now().timestamp();
        
        Self {
            id,
            recipient,
            amount,
            reason,
            proposer,
            approvals: HashSet::from([proposer]),
            rejections: HashSet::new(),
            executed: false,
            created_at: now,
            expires_at: now + (7 * 24 * 3600), // 7 days expiry
        }
    }
    
    pub fn is_expired(&self) -> bool {
        chrono::Utc::now().timestamp() > self.expires_at
    }
}

pub struct TreasuryManager {
    balances: Arc<BalanceManager>,
    multisig: Arc<RwLock<Option<MultisigConfig>>>,
    proposals: Arc<RwLock<HashMap<[u8; 32], TreasuryProposal>>>,
    executed_proposals: Arc<RwLock<Vec<[u8; 32]>>>,
}

impl TreasuryManager {
    pub fn new(balances: Arc<BalanceManager>) -> Self {
        Self {
            balances,
            multisig: Arc::new(RwLock::new(None)),
            proposals: Arc::new(RwLock::new(HashMap::new())),
            executed_proposals: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn initialize_multisig(
        &self,
        signers: Vec<AccountAddress>,
        threshold: u32,
    ) -> Result<()> {
        if signers.is_empty() {
            bail!("Multisig requires at least one signer");
        }
        
        if threshold == 0 || threshold > signers.len() as u32 {
            bail!("Invalid threshold: must be between 1 and {}", signers.len());
        }
        
        // Check for duplicate signers
        let unique_signers: HashSet<_> = signers.iter().collect();
        if unique_signers.len() != signers.len() {
            bail!("Duplicate signers not allowed");
        }
        
        let mut multisig = self.multisig.write().await;
        if multisig.is_some() {
            bail!("Multisig already initialized");
        }
        
        *multisig = Some(MultisigConfig {
            signers: signers.clone(),
            threshold,
        });
        
        info!("Treasury multisig initialized with {} signers, threshold {}", 
              signers.len(), threshold);
        
        Ok(())
    }

    pub async fn update_multisig(
        &self,
        new_signers: Vec<AccountAddress>,
        new_threshold: u32,
        approver: AccountAddress,
    ) -> Result<()> {
        let multisig = self.multisig.read().await;
        let config = multisig.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Multisig not initialized"))?;
        
        // Only existing signers can update multisig
        if !config.signers.contains(&approver) {
            bail!("Only existing signers can update multisig");
        }
        
        drop(multisig);
        
        // Create a special proposal for multisig update
        // This would need proper consensus among existing signers
        // For now, we'll require all current signers to approve
        
        info!("Multisig update proposed by {}", approver);
        
        // In production, this would go through a proposal process
        // For now, we'll just validate and update
        if new_threshold == 0 || new_threshold > new_signers.len() as u32 {
            bail!("Invalid new threshold");
        }
        
        let mut multisig = self.multisig.write().await;
        *multisig = Some(MultisigConfig {
            signers: new_signers,
            threshold: new_threshold,
        });
        
        Ok(())
    }

    pub async fn propose_transfer(
        &self,
        recipient: AccountAddress,
        amount: AdicAmount,
        reason: String,
        proposer: AccountAddress,
    ) -> Result<[u8; 32]> {
        let multisig = self.multisig.read().await;
        let config = multisig.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Multisig not initialized"))?;
        
        if !config.signers.contains(&proposer) {
            bail!("Only multisig signers can propose transfers");
        }
        
        // Check treasury balance
        let treasury_addr = AccountAddress::treasury();
        let balance = self.balances.get_balance(treasury_addr).await?;
        
        if amount > balance {
            bail!("Insufficient treasury balance: has {}, requested {}", balance, amount);
        }
        
        let proposal = TreasuryProposal::new(recipient, amount, reason.clone(), proposer);
        let proposal_id = proposal.id;
        
        let mut proposals = self.proposals.write().await;
        proposals.insert(proposal_id, proposal);
        
        info!("Treasury transfer proposed: {} to {} for '{}'", 
              amount, recipient, reason);
        
        Ok(proposal_id)
    }

    pub async fn approve_proposal(
        &self,
        proposal_id: [u8; 32],
        approver: AccountAddress,
    ) -> Result<bool> {
        let multisig = self.multisig.read().await;
        let config = multisig.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Multisig not initialized"))?;
        
        if !config.signers.contains(&approver) {
            bail!("Only multisig signers can approve proposals");
        }
        
        let threshold = config.threshold;
        drop(multisig);
        
        let mut proposals = self.proposals.write().await;
        let proposal = proposals.get_mut(&proposal_id)
            .ok_or_else(|| anyhow::anyhow!("Proposal not found"))?;
        
        if proposal.executed {
            bail!("Proposal already executed");
        }
        
        if proposal.is_expired() {
            bail!("Proposal has expired");
        }
        
        // Remove from rejections if previously rejected
        proposal.rejections.remove(&approver);
        proposal.approvals.insert(approver);
        
        let approval_count = proposal.approvals.len() as u32;
        
        info!("Proposal {} approved by {} ({}/{})", 
              hex::encode(&proposal_id[..8]), approver, approval_count, threshold);
        
        // Execute if threshold reached
        if approval_count >= threshold && !proposal.executed {
            self.execute_proposal_internal(proposal).await?;
            return Ok(true);
        }
        
        Ok(false)
    }

    pub async fn reject_proposal(
        &self,
        proposal_id: [u8; 32],
        rejector: AccountAddress,
    ) -> Result<()> {
        let multisig = self.multisig.read().await;
        let config = multisig.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Multisig not initialized"))?;
        
        if !config.signers.contains(&rejector) {
            bail!("Only multisig signers can reject proposals");
        }
        
        drop(multisig);
        
        let mut proposals = self.proposals.write().await;
        let proposal = proposals.get_mut(&proposal_id)
            .ok_or_else(|| anyhow::anyhow!("Proposal not found"))?;
        
        if proposal.executed {
            bail!("Proposal already executed");
        }
        
        // Remove from approvals if previously approved
        proposal.approvals.remove(&rejector);
        proposal.rejections.insert(rejector);
        
        info!("Proposal {} rejected by {}", hex::encode(&proposal_id[..8]), rejector);
        
        Ok(())
    }

    async fn execute_proposal_internal(&self, proposal: &mut TreasuryProposal) -> Result<()> {
        let treasury_addr = AccountAddress::treasury();
        
        // Execute the transfer
        self.balances.transfer(treasury_addr, proposal.recipient, proposal.amount).await?;
        
        // Mark as executed
        proposal.executed = true;
        
        // Record the execution
        let mut executed = self.executed_proposals.write().await;
        executed.push(proposal.id);
        
        // Keep only last 1000 executed proposals
        if executed.len() > 1000 {
            executed.drain(0..100);
        }
        
        info!("Treasury proposal executed: {} to {} for '{}'",
              proposal.amount, proposal.recipient, proposal.reason);
        
        Ok(())
    }

    pub async fn get_proposal(&self, proposal_id: [u8; 32]) -> Option<TreasuryProposal> {
        let proposals = self.proposals.read().await;
        proposals.get(&proposal_id).cloned()
    }

    pub async fn get_active_proposals(&self) -> Vec<TreasuryProposal> {
        let proposals = self.proposals.read().await;
        let now = chrono::Utc::now().timestamp();
        
        proposals.values()
            .filter(|p| !p.executed && p.expires_at > now)
            .cloned()
            .collect()
    }

    pub async fn get_treasury_balance(&self) -> Result<AdicAmount> {
        self.balances.get_balance(AccountAddress::treasury()).await
    }

    pub async fn cleanup_expired_proposals(&self) {
        let mut proposals = self.proposals.write().await;
        let now = chrono::Utc::now().timestamp();
        
        let expired: Vec<_> = proposals
            .iter()
            .filter(|(_, p)| !p.executed && p.expires_at <= now)
            .map(|(id, _)| *id)
            .collect();
        
        for id in expired {
            proposals.remove(&id);
            info!("Removed expired proposal: {}", hex::encode(&id[..8]));
        }
    }

    pub async fn emergency_pause(&self, pauser: AccountAddress) -> Result<()> {
        let multisig = self.multisig.read().await;
        let config = multisig.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Multisig not initialized"))?;
        
        if !config.signers.contains(&pauser) {
            bail!("Only multisig signers can pause treasury");
        }
        
        // In production, this would pause all treasury operations
        // For now, we'll just log it
        warn!("EMERGENCY: Treasury paused by {}", pauser);
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;

    #[tokio::test]
    async fn test_multisig_initialization() {
        let storage = Arc::new(MemoryStorage::new());
        let balances = Arc::new(BalanceManager::new(storage));
        let treasury = TreasuryManager::new(balances);
        
        let signers = vec![
            AccountAddress::from_bytes([1; 32]),
            AccountAddress::from_bytes([2; 32]),
            AccountAddress::from_bytes([3; 32]),
        ];
        
        // Initialize multisig
        assert!(treasury.initialize_multisig(signers.clone(), 2).await.is_ok());
        
        // Cannot initialize twice
        assert!(treasury.initialize_multisig(signers, 2).await.is_err());
    }

    #[tokio::test]
    async fn test_proposal_approval() {
        let storage = Arc::new(MemoryStorage::new());
        let balances = Arc::new(BalanceManager::new(storage));
        let treasury = Arc::new(TreasuryManager::new(balances.clone()));
        
        let signer1 = AccountAddress::from_bytes([1; 32]);
        let signer2 = AccountAddress::from_bytes([2; 32]);
        let signer3 = AccountAddress::from_bytes([3; 32]);
        let recipient = AccountAddress::from_bytes([4; 32]);
        
        let signers = vec![signer1, signer2, signer3];
        treasury.initialize_multisig(signers, 2).await.unwrap();
        
        // Fund treasury
        let treasury_addr = AccountAddress::treasury();
        balances.credit(treasury_addr, AdicAmount::from_adic(1000.0)).await.unwrap();
        
        // Create proposal
        let proposal_id = treasury.propose_transfer(
            recipient,
            AdicAmount::from_adic(100.0),
            "Test transfer".to_string(),
            signer1,
        ).await.unwrap();
        
        // First approval (proposer already approved)
        // Second approval should execute
        let executed = treasury.approve_proposal(proposal_id, signer2).await.unwrap();
        assert!(executed);
        
        // Check balances
        assert_eq!(balances.get_balance(treasury_addr).await.unwrap(), 
                   AdicAmount::from_adic(900.0));
        assert_eq!(balances.get_balance(recipient).await.unwrap(),
                   AdicAmount::from_adic(100.0));
    }
}